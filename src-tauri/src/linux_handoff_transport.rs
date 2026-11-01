//! Private, bounded messages for a future two-process Linux update handoff.
//!
//! A readiness message is only meaningful after the caller has completed the
//! corresponding real startup phase. This transport does not start a process,
//! transfer D-Bus ownership, or authorize the old process to exit.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 5;
const MAX_MESSAGE: usize = 4096;
const CHILD_FD_ENV: &str = "VERENU_HANDOFF_FD";
pub(crate) type KeyAvailability = BTreeMap<String, bool>;

fn database_code_fingerprint() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for source in [
        &include_bytes!("data/db/mod.rs")[..],
        &include_bytes!("data/db/schema.rs")[..],
        &include_bytes!("data/db/activity.rs")[..],
        &include_bytes!("data/db/agent_audit.rs")[..],
        &include_bytes!("data/db/agent_history.rs")[..],
        &include_bytes!("data/db/agent_library.rs")[..],
        &include_bytes!("data/db/cleanup_cache.rs")[..],
        &include_bytes!("data/db/contexts.rs")[..],
        &include_bytes!("data/db/dictionary.rs")[..],
        &include_bytes!("data/db/insights.rs")[..],
        &include_bytes!("data/db/library_usage.rs")[..],
        &include_bytes!("data/db/pricing.rs")[..],
        &include_bytes!("data/db/snippets.rs")[..],
        &include_bytes!("data/db/sub_apps.rs")[..],
        &include_bytes!("data/db/transcriptions.rs")[..],
        &include_bytes!("data/db/validation.rs")[..],
        &include_bytes!("linux_handoff_callbacks.rs")[..],
    ] {
        hash.update((source.len() as u64).to_le_bytes());
        hash.update(source);
    }
    format!("{:x}", hash.finalize())
}

fn descends_from(mut pid: libc::pid_t, ancestor: libc::pid_t) -> bool {
    for _ in 0..32 {
        if pid == ancestor {
            return true;
        }
        if pid <= 1 {
            return false;
        }
        let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
            return false;
        };
        let Some(parent) = status.lines().find_map(|line| {
            line.strip_prefix("PPid:")
                .and_then(|value| value.trim().parse::<libc::pid_t>().ok())
        }) else {
            return false;
        };
        if parent == pid {
            return false;
        }
        pid = parent;
    }
    false
}

pub(crate) fn is_replacement_group(group: u32) -> bool {
    if group == 0 || group > i32::MAX as u32 {
        return false;
    }
    let pid = group as libc::pid_t;
    let current = std::process::id() as libc::pid_t;
    pid != current && unsafe { libc::getpgid(pid) == pid } && descends_from(pid, current)
}

pub(crate) fn process_in_replacement_group(pid: u32, group: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 || !is_replacement_group(group) {
        return false;
    }
    let pid = pid as libc::pid_t;
    (unsafe { libc::getpgid(pid) == group as libc::pid_t })
        && descends_from(pid, std::process::id() as libc::pid_t)
}

fn child_channel_from_environment(timeout: Duration) -> Result<Option<Channel>, String> {
    let Some(value) = std::env::var_os(CHILD_FD_ENV) else {
        return Ok(None);
    };
    std::env::remove_var(CHILD_FD_ENV);
    let fd: i32 = value
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|fd| *fd >= 3)
        .ok_or("Invalid restart connection")?;
    let mut socket_type = 0i32;
    let mut socket_len = std::mem::size_of_val(&socket_type) as libc::socklen_t;
    let mut address: libc::sockaddr_storage = unsafe { std::mem::zeroed() };
    let mut address_len = std::mem::size_of_val(&address) as libc::socklen_t;
    let mut peer: libc::ucred = unsafe { std::mem::zeroed() };
    let mut peer_len = std::mem::size_of_val(&peer) as libc::socklen_t;
    let valid = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut i32).cast(),
            &mut socket_len,
        ) == 0
            && socket_type == libc::SOCK_STREAM
            && libc::getsockname(
                fd,
                (&mut address as *mut libc::sockaddr_storage).cast(),
                &mut address_len,
            ) == 0
            && address.ss_family as i32 == libc::AF_UNIX
            && libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut peer as *mut libc::ucred).cast(),
                &mut peer_len,
            ) == 0
            && peer.uid == libc::geteuid()
    };
    if !valid {
        return Err("Invalid restart connection".into());
    }
    let parent = ParentMonitor::open(peer.pid)?;
    if !descends_from(unsafe { libc::getppid() }, peer.pid) {
        return Err("Invalid restart connection".into());
    }
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err("Could not secure the restart connection".into());
    }
    // The descriptor was explicitly passed to this process and validated above.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    let mut channel = Channel::new(stream, timeout)?;
    channel.parent = Some(parent);
    Ok(Some(channel))
}

struct ParentMonitor(OwnedFd);

impl ParentMonitor {
    fn open(pid: libc::pid_t) -> Result<Self, String> {
        if pid <= 0 {
            return Err("Invalid restart parent".into());
        }
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
        if fd < 0 {
            return Err(format!(
                "Could not monitor the restart parent: {}",
                std::io::Error::last_os_error()
            ));
        }
        // pidfd_open returns a new close-on-exec descriptor owned by this process.
        Ok(Self(unsafe { OwnedFd::from_raw_fd(fd as i32) }))
    }

    fn wait(&self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let millis = remaining
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut descriptor = libc::pollfd {
                fd: self.0.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
            if result > 0 {
                return if descriptor.revents & libc::POLLIN != 0 {
                    Ok(())
                } else {
                    Err("Could not confirm the restart parent exited".into())
                };
            }
            if result < 0 {
                let error = std::io::Error::last_os_error();
                if error.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(format!("Could not monitor the restart parent: {error}"));
            }
            if Instant::now() >= deadline {
                return Err("The restart parent did not exit in time".into());
            }
        }
    }
}

pub(crate) struct Child {
    channel: Channel,
}

impl Child {
    pub(crate) fn from_environment(timeout: Duration) -> Result<Option<Self>, String> {
        child_channel_from_environment(timeout)
            .map(|channel| channel.map(|channel| Self { channel }))
    }

    pub(crate) fn begin(mut self, schema: i64) -> Result<ChildStartup, String> {
        self.channel.send(&Message::Hello {
            protocol: PROTOCOL,
            pid: std::process::id(),
            schema,
            database_code: database_code_fingerprint(),
        })?;
        match self.channel.receive()? {
            Message::Begin {
                key_availability,
                parent_unique_name,
                callback_queue_path,
            } => Ok(ChildStartup {
                channel: self.channel,
                key_availability,
                parent_unique_name,
                callback_queue_path,
            }),
            _ => Err("Replacement received an invalid startup request".into()),
        }
    }
}

pub(crate) struct ChildStartup {
    channel: Channel,
    key_availability: KeyAvailability,
    parent_unique_name: Option<String>,
    callback_queue_path: Option<PathBuf>,
}

impl ChildStartup {
    pub(crate) fn backend_ready(mut self) -> Result<ChildFrontends, String> {
        self.channel.send(&Message::BackendReady)?;
        Ok(ChildFrontends(self.channel))
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.channel.send(&Message::Failed { reason })
    }

    pub(crate) fn key_availability(&self) -> &KeyAvailability {
        &self.key_availability
    }

    pub(crate) fn parent_unique_name(&self) -> Option<&str> {
        self.parent_unique_name.as_deref()
    }

    pub(crate) fn callback_queue_path(&self) -> Option<&Path> {
        self.callback_queue_path.as_deref()
    }
}

pub(crate) struct ChildFrontends(Channel);

impl ChildFrontends {
    pub(crate) fn ready(mut self) -> Result<ChildActivation, String> {
        self.0.send(&Message::FrontendsReady)?;
        self.0.expect(Message::Activate)?;
        Ok(ChildActivation(self.0))
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.0.send(&Message::Failed { reason })
    }
}

pub(crate) struct ChildActivation(Channel);

impl ChildActivation {
    pub(crate) fn ownership_acquired(mut self) -> Result<ChildServices, String> {
        self.0.send(&Message::OwnershipAcquired)?;
        self.0.expect(Message::ContinueActivation)?;
        Ok(ChildServices(self.0))
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.0.send(&Message::Failed { reason })
    }
}

pub(crate) struct ChildServices(Channel);

impl ChildServices {
    pub(crate) fn activated(mut self) -> Result<ChildCommit, String> {
        self.0.send(&Message::Activated)?;
        self.0.expect(Message::Commit)?;
        Ok(ChildCommit(self.0))
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.0.send(&Message::Failed { reason })
    }
}

#[must_use]
pub(crate) struct ChildCommit(Channel);
#[must_use]
pub(crate) struct ParentExited(());

#[cfg(test)]
pub(crate) fn parent_exited_for_test() -> ParentExited {
    let mut process = std::process::Command::new("/bin/sleep")
        .arg("5")
        .spawn()
        .unwrap();
    let monitor = ParentMonitor::open(process.id() as libc::pid_t).unwrap();
    process.kill().unwrap();
    process.wait().unwrap();
    monitor.wait(Duration::from_secs(5)).unwrap();
    ParentExited(())
}

impl ChildCommit {
    pub(crate) fn wait_for_parent_exit(self, timeout: Duration) -> Result<ParentExited, String> {
        self.0
            .parent
            .as_ref()
            .ok_or("Restart parent was not authenticated")?
            .wait(timeout)?;
        Ok(ParentExited(()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Hello {
        protocol: u32,
        pid: u32,
        schema: i64,
        database_code: String,
    },
    Begin {
        key_availability: KeyAvailability,
        parent_unique_name: Option<String>,
        callback_queue_path: Option<PathBuf>,
    },
    BackendReady,
    FrontendsReady,
    Activate,
    OwnershipAcquired,
    ContinueActivation,
    Activated,
    Commit,
    Failed {
        reason: String,
    },
}

struct Channel {
    stream: UnixStream,
    timeout: Duration,
    parent: Option<ParentMonitor>,
}

impl Channel {
    fn new(stream: UnixStream, timeout: Duration) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            stream,
            timeout,
            parent: None,
        })
    }

    fn send(&mut self, message: &Message) -> Result<(), String> {
        let bytes = serde_json::to_vec(message).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_MESSAGE {
            return Err("Restart message is too large".into());
        }
        self.stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .and_then(|_| self.stream.write_all(&bytes))
            .map_err(|error| format!("Restart connection failed: {error}"))
    }

    fn receive(&mut self) -> Result<Message, String> {
        let deadline = Instant::now() + self.timeout;
        let mut length = [0; 4];
        self.read_until(&mut length, deadline)?;
        let length = u32::from_be_bytes(length) as usize;
        if length == 0 || length > MAX_MESSAGE {
            return Err("Replacement sent an invalid restart message".into());
        }
        let mut bytes = vec![0; length];
        self.read_until(&mut bytes, deadline)?;
        serde_json::from_slice(&bytes)
            .map_err(|_| "Replacement sent an invalid restart message".into())
    }

    fn read_until(&mut self, mut bytes: &mut [u8], deadline: Instant) -> Result<(), String> {
        while !bytes.is_empty() {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .filter(|duration| !duration.is_zero())
                .ok_or("Replacement stopped responding: timed out")?;
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|error| error.to_string())?;
            match self.stream.read(bytes) {
                Ok(0) => return Err("Replacement stopped responding: connection closed".into()),
                Ok(count) => bytes = &mut bytes[count..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("Replacement stopped responding: {error}")),
            }
        }
        Ok(())
    }

    fn expect(&mut self, expected: Message) -> Result<(), String> {
        match self.receive()? {
            Message::Failed { reason } => Err(format!("Replacement failed to start: {reason}")),
            message if message == expected => Ok(()),
            _ => Err("Replacement sent a restart message out of order".into()),
        }
    }
}

struct Parent {
    channel: Channel,
    child_pid: u32,
    schema: i64,
    key_availability: KeyAvailability,
    parent_unique_name: Option<String>,
    callback_queue_path: Option<PathBuf>,
}

struct ProcessGuard(std::process::Child, UnixStream);

pub(crate) fn group_has_live_members(group: u32) -> Result<bool, String> {
    let entries = std::fs::read_dir("/proc").map_err(|error| error.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let stat = match std::fs::read_to_string(entry.path().join("stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        let (_, fields) = stat.rsplit_once(')').ok_or("Invalid process status")?;
        let mut fields = fields.split_whitespace();
        let state = fields.next();
        let _parent = fields.next();
        let process_group = fields.next().and_then(|value| value.parse::<u32>().ok())
            .ok_or("Invalid process group")?;
        if process_group == group && !matches!(state, Some("Z" | "X")) {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        // The child starts a new process group so an AppImage launcher and its
        // application are both stopped before the parent resumes its services.
        let group = self.0.id();
        if group <= i32::MAX as u32 {
            unsafe { libc::kill(-(group as i32), libc::SIGKILL) };
        }
        let _ = self.0.kill();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match group_has_live_members(group) {
                Ok(false) => break,
                Ok(true) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                result => {
                    log::error!("Could not confirm replacement process group stopped: {result:?}");
                    break;
                }
            }
        }
        let _ = self.0.wait();
    }
}

struct Replacement {
    process: ProcessGuard,
    parent: Parent,
}

impl Replacement {
    fn spawn(
        mut command: std::process::Command,
        schema: i64,
        key_availability: KeyAvailability,
        timeout: Duration,
    ) -> Result<Self, String> {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;

        let (parent, child) = UnixStream::pair().map_err(|error| error.to_string())?;
        let keepalive = parent.try_clone().map_err(|error| error.to_string())?;
        for endpoint in [&parent, &keepalive] {
            let flags = unsafe { libc::fcntl(endpoint.as_raw_fd(), libc::F_GETFD) };
            if flags < 0
                || unsafe {
                    libc::fcntl(
                        endpoint.as_raw_fd(),
                        libc::F_SETFD,
                        flags | libc::FD_CLOEXEC,
                    )
                } < 0
            {
                return Err("Could not secure the parent restart connection".into());
            }
        }
        let fd = child.as_raw_fd();
        command
            .env(CHILD_FD_ENV, fd.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::setpgid(0, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let process = ProcessGuard(
            command
                .spawn()
                .map_err(|error| format!("Could not start replacement: {error}"))?,
            keepalive,
        );
        drop(child);
        let parent = Parent {
            channel: Channel::new(parent, timeout)?,
            child_pid: process.0.id(),
            schema,
            key_availability,
            parent_unique_name: None,
            callback_queue_path: None,
        };
        Ok(Self { process, parent })
    }

    fn prepare(self) -> Result<ReadyReplacement, String> {
        let probation = self.parent.prepare()?;
        Ok(ReadyReplacement {
            process: self.process,
            probation,
        })
    }

    fn with_parent_unique_name(mut self, name: String) -> Self {
        self.parent.parent_unique_name = Some(name);
        self
    }

    fn with_callback_queue(mut self, path: PathBuf) -> Self {
        self.parent.callback_queue_path = Some(path);
        self
    }
}

struct ReadyReplacement {
    process: ProcessGuard,
    probation: Probation,
}

impl ReadyReplacement {
    fn acquire_ownership(self) -> Result<OwnedReplacement, String> {
        let ownership = self.probation.acquire_ownership()?;
        Ok(OwnedReplacement {
            process: self.process,
            ownership,
        })
    }
}

struct OwnedReplacement {
    process: ProcessGuard,
    ownership: Ownership,
}

impl OwnedReplacement {
    fn activate(self) -> Result<ActivatedReplacement, String> {
        let activated = self.ownership.activate()?;
        Ok(ActivatedReplacement {
            process: self.process,
            activated,
        })
    }
}

struct ActivatedReplacement {
    process: ProcessGuard,
    activated: Activated,
}

#[must_use]
pub(crate) struct StagedRestart {
    // Drop the replacement before the preparation lease resumes the parent.
    replacement: ActivatedReplacement,
    preparation: crate::linux_handoff::PreparedHandoff,
}

pub(crate) async fn stage_replacement(
    app: &tauri::AppHandle,
    command: std::process::Command,
    schema: i64,
    key_availability: KeyAvailability,
    timeout: Duration,
) -> Result<StagedRestart, String> {
    use tauri::Manager;

    let preparation = crate::linux_handoff::prepare(app).await?;
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut preparation = preparation;
        let owner = app.try_state::<crate::linux_single_instance::InstanceOwner>()
            .ok_or("Single-instance ownership is unavailable")?;
        let replacement = Replacement::spawn(command, schema, key_availability, timeout)?
            .with_parent_unique_name(owner.unique_name()?)
            .with_callback_queue(preparation.callback_queue_path().to_path_buf());
        let ready = replacement.prepare()?;
        let group = ready.process.0.id();
        preparation.begin_instance_transfer(group)?;
        let owned = ready.acquire_ownership()?;
        owner.verify_replacement_owner(group)?;
        owner.callback_barrier()?;
        owner.replacement_callback_barrier(group)?;
        let replacement = owned.activate()?;
        owner.verify_replacement_owner(group)?;
        owner.callback_barrier()?;
        owner.replacement_callback_barrier(group)?;
        Ok(StagedRestart { replacement, preparation })
    }).await.map_err(|error| format!("Restart preparation stopped: {error}"))?
}

impl Parent {
    fn prepare(mut self) -> Result<Probation, String> {
        match self.channel.receive()? {
            Message::Hello {
                protocol,
                pid,
                schema,
                database_code,
            } if protocol == PROTOCOL
                && schema == self.schema
                && database_code == database_code_fingerprint()
                && (pid == self.child_pid
                    || (pid <= i32::MAX as u32
                        && self.child_pid <= i32::MAX as u32
                        && descends_from(pid as i32, self.child_pid as i32))) => {}
            _ => return Err("Replacement identity or compatibility did not match".into()),
        }
        self.channel.send(&Message::Begin {
            key_availability: self.key_availability,
            parent_unique_name: self.parent_unique_name,
            callback_queue_path: self.callback_queue_path,
        })?;
        self.channel.expect(Message::BackendReady)?;
        self.channel.expect(Message::FrontendsReady)?;
        Ok(Probation(self.channel))
    }
}

struct Probation(Channel);

impl Probation {
    fn acquire_ownership(mut self) -> Result<Ownership, String> {
        self.0.send(&Message::Activate)?;
        self.0.expect(Message::OwnershipAcquired)?;
        Ok(Ownership(self.0))
    }
}

struct Ownership(Channel);

impl Ownership {
    fn activate(mut self) -> Result<Activated, String> {
        self.0.send(&Message::ContinueActivation)?;
        self.0.expect(Message::Activated)?;
        Ok(Activated(self.0))
    }
}

struct Activated(Channel);

impl Activated {
    fn commit(mut self) -> Result<(), String> {
        self.0.send(&Message::Commit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_group_must_be_a_child_process_group() {
        use std::os::unix::process::CommandExt;
        assert!(!is_replacement_group(0));
        assert!(!is_replacement_group(std::process::id()));
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(is_replacement_group(child.id()));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn parent_monitor_waits_for_the_exact_process_to_exit() {
        let mut process = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let monitor = ParentMonitor::open(process.id() as libc::pid_t).unwrap();
        let flags = unsafe { libc::fcntl(monitor.0.as_raw_fd(), libc::F_GETFD) };
        assert!(flags >= 0 && flags & libc::FD_CLOEXEC != 0);
        assert!(monitor.wait(Duration::from_millis(50)).is_err());
        process.kill().unwrap();
        process.wait().unwrap();
        monitor.wait(Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn commit_without_an_authenticated_parent_cannot_authorize_startup() {
        let (_parent, child) = pair();
        assert!(ChildCommit(child)
            .wait_for_parent_exit(Duration::from_millis(1))
            .is_err());
    }

    fn begin() -> Message {
        Message::Begin {
            key_availability: KeyAvailability::new(),
            parent_unique_name: None,
            callback_queue_path: None,
        }
    }

    fn pair() -> (Parent, Channel) {
        let (parent, child) = UnixStream::pair().unwrap();
        let timeout = Duration::from_secs(2);
        (
            Parent {
                channel: Channel::new(parent, timeout).unwrap(),
                child_pid: 42,
                schema: 39,
                key_availability: KeyAvailability::new(),
                parent_unique_name: None,
                callback_queue_path: None,
            },
            Channel::new(child, timeout).unwrap(),
        )
    }

    #[test]
    fn failure_after_backend_readiness_does_not_reach_activation() {
        let (parent, mut child) = pair();
        let worker = std::thread::spawn(move || {
            child
                .send(&Message::Hello {
                    protocol: PROTOCOL,
                    pid: 42,
                    schema: 39,
                    database_code: database_code_fingerprint(),
                })
                .unwrap();
            child.expect(begin()).unwrap();
            child.send(&Message::BackendReady).unwrap();
            child
                .send(&Message::Failed {
                    reason: "frontend failed".into(),
                })
                .unwrap();
        });
        assert!(matches!(parent.prepare(), Err(error) if error.contains("frontend failed")));
        worker.join().unwrap();
    }

    #[test]
    fn disconnect_after_frontends_ready_does_not_commit() {
        let (parent, mut child) = pair();
        let worker = std::thread::spawn(move || {
            child
                .send(&Message::Hello {
                    protocol: PROTOCOL,
                    pid: 42,
                    schema: 39,
                    database_code: database_code_fingerprint(),
                })
                .unwrap();
            child.expect(begin()).unwrap();
            child.send(&Message::BackendReady).unwrap();
            child.send(&Message::FrontendsReady).unwrap();
        });
        let probation = parent.prepare().unwrap();
        worker.join().unwrap();
        assert!(probation.acquire_ownership().is_err());
    }

    #[test]
    fn wrong_pid_schema_or_database_code_is_rejected_before_startup() {
        for message in [
            Message::Hello {
                protocol: PROTOCOL,
                pid: 43,
                schema: 39,
                database_code: database_code_fingerprint(),
            },
            Message::Hello {
                protocol: PROTOCOL,
                pid: 42,
                schema: 40,
                database_code: database_code_fingerprint(),
            },
            Message::Hello {
                protocol: PROTOCOL,
                pid: 42,
                schema: 39,
                database_code: "different".into(),
            },
        ] {
            let (parent, mut child) = pair();
            child.send(&message).unwrap();
            assert!(parent.prepare().is_err());
        }
    }

    #[test]
    fn successful_handoff_requires_each_phase_in_order() {
        let (parent, mut child) = pair();
        let worker = std::thread::spawn(move || {
            child
                .send(&Message::Hello {
                    protocol: PROTOCOL,
                    pid: 42,
                    schema: 39,
                    database_code: database_code_fingerprint(),
                })
                .unwrap();
            child.expect(begin()).unwrap();
            child.send(&Message::BackendReady).unwrap();
            child.send(&Message::FrontendsReady).unwrap();
            child.expect(Message::Activate).unwrap();
            child.send(&Message::OwnershipAcquired).unwrap();
            child.expect(Message::ContinueActivation).unwrap();
            child.send(&Message::Activated).unwrap();
            child.expect(Message::Commit).unwrap();
        });
        parent
            .prepare()
            .unwrap()
            .acquire_ownership()
            .unwrap()
            .activate()
            .unwrap()
            .commit()
            .unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn child_receives_key_availability_without_credentials() {
        let (mut parent, child) = pair();
        parent.child_pid = std::process::id();
        parent.key_availability.insert("openai".into(), true);
        let worker = std::thread::spawn(move || {
            let startup = Child { channel: child }.begin(39).unwrap();
            assert_eq!(startup.key_availability().get("openai"), Some(&true));
            let _commit = startup
                .backend_ready()
                .unwrap()
                .ready()
                .unwrap()
                .ownership_acquired()
                .unwrap()
                .activated()
                .unwrap();
        });
        parent
            .prepare()
            .unwrap()
            .acquire_ownership()
            .unwrap()
            .activate()
            .unwrap()
            .commit()
            .unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn oversized_frames_are_rejected_without_allocation() {
        let (parent, mut child) = pair();
        child
            .stream
            .write_all(&((MAX_MESSAGE + 1) as u32).to_be_bytes())
            .unwrap();
        assert!(matches!(parent.prepare(), Err(error) if error.contains("invalid")));
    }

    #[test]
    fn frontend_readiness_cannot_skip_backend_startup() {
        let (parent, mut child) = pair();
        let worker = std::thread::spawn(move || {
            child
                .send(&Message::Hello {
                    protocol: PROTOCOL,
                    pid: 42,
                    schema: 39,
                    database_code: database_code_fingerprint(),
                })
                .unwrap();
            child.expect(begin()).unwrap();
            child.send(&Message::FrontendsReady).unwrap();
        });
        assert!(matches!(parent.prepare(), Err(error) if error.contains("out of order")));
        worker.join().unwrap();
    }

    #[test]
    fn partial_frame_cannot_extend_the_deadline() {
        let (parent, mut child) = UnixStream::pair().unwrap();
        let mut parent = Channel::new(parent, Duration::from_millis(100)).unwrap();
        let worker = std::thread::spawn(move || {
            child.write_all(&4u32.to_be_bytes()).unwrap();
            for _ in 0..4 {
                std::thread::sleep(Duration::from_millis(60));
                if child.write_all(b"x").is_err() {
                    break;
                }
            }
        });
        assert!(parent.receive().is_err());
        worker.join().unwrap();
    }

    #[test]
    fn replacement_process_fixture() {
        use std::os::fd::AsRawFd;
        let Some(mode) = std::env::var_os("VERENU_HANDOFF_TEST_CHILD") else {
            return;
        };
        if mode == "invalid" {
            assert!(child_channel_from_environment(Duration::from_secs(1)).is_err());
            assert!(std::env::var_os(CHILD_FD_ENV).is_none());
            return;
        }
        let child = Child::from_environment(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert!(std::env::var_os(CHILD_FD_ENV).is_none());
        assert_ne!(
            unsafe { libc::fcntl(child.channel.stream.as_raw_fd(), libc::F_GETFD) }
                & libc::FD_CLOEXEC,
            0
        );
        let frontends = child.begin(39).unwrap().backend_ready().unwrap();
        if mode == "hang" {
            let marker = std::env::var_os("VERENU_HANDOFF_TEST_MARKER").unwrap();
            std::fs::write(marker, std::process::id().to_string()).unwrap();
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        if mode == "activate" {
            let _ = frontends
                .ready()
                .and_then(|activation| activation.ownership_acquired())
                .and_then(|services| services.activated());
        }
        // Exit before frontend readiness to model a failed replacement.
    }

    #[test]
    fn replacement_process_exit_after_backend_leaves_parent_alive() {
        run_failed_replacement(false);
    }

    #[test]
    fn launcher_descendant_can_complete_the_initial_handshake() {
        run_failed_replacement(true);
    }

    fn run_failed_replacement(via_launcher: bool) {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let (parent, child) = UnixStream::pair().unwrap();
        let fd = child.as_raw_fd();
        let executable = std::env::current_exe().unwrap();
        let mut command = if via_launcher {
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "\"$@\" & wait $!", "verenu-test-launcher"]);
            command.arg(executable);
            command
        } else {
            Command::new(executable)
        };
        command
            .args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
                "--nocapture",
            ])
            .env("VERENU_HANDOFF_TEST_CHILD", "1")
            .env(CHILD_FD_ENV, fd.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags < 0 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut process = command.spawn().unwrap();
        drop(child);
        let parent = Parent {
            channel: Channel::new(parent, Duration::from_secs(5)).unwrap(),
            child_pid: process.id(),
            schema: 39,
            key_availability: KeyAvailability::new(),
            parent_unique_name: None,
            callback_queue_path: None,
        };
        assert!(parent.prepare().is_err());
        assert!(process.wait().unwrap().success());
    }

    #[test]
    fn child_rejects_an_invalid_inherited_descriptor() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
            ])
            .env("VERENU_HANDOFF_TEST_CHILD", "invalid")
            .env(CHILD_FD_ENV, "0")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }

    #[test]
    fn managed_replacement_is_reaped_after_failed_startup() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
            ])
            .env("VERENU_HANDOFF_TEST_CHILD", "1");
        let replacement =
            Replacement::spawn(command, 39, KeyAvailability::new(), Duration::from_secs(5))
                .unwrap();
        let pid = replacement.process.0.id();
        assert!(replacement.prepare().is_err());
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    #[test]
    fn abandoning_an_activated_replacement_kills_and_reaps_it() {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
            ])
            .env("VERENU_HANDOFF_TEST_CHILD", "activate");
        let replacement =
            Replacement::spawn(command, 39, KeyAvailability::new(), Duration::from_secs(5))
                .unwrap();
        let pid = replacement.process.0.id();
        let activated = replacement
            .prepare()
            .unwrap()
            .acquire_ownership()
            .unwrap()
            .activate()
            .unwrap();
        drop(activated);
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }

    #[cfg(feature = "native-testing")]
    struct HandoffTrayWatcher(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    #[cfg(feature = "native-testing")]
    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl HandoffTrayWatcher {
        #[zbus(property)]
        fn is_status_notifier_host_registered(&self) -> bool {
            true
        }

        fn register_status_notifier_item(&self, service: &str) {
            self.0.lock().unwrap().push(service.to_owned());
        }
    }

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires a private Linux desktop and a custom-protocol app build"]
    fn real_replacement_opens_read_only_and_keeps_parent_alive() {
        use std::process::Command;
        use tauri::Manager;

        let registrations = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let watcher = zbus::blocking::connection::Builder::session()
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at("/StatusNotifierWatcher", HandoffTrayWatcher(registrations.clone()))
            .unwrap()
            .build()
            .unwrap();
        let gate = std::sync::Arc::new(crate::linux_activity::ActivityGate::default());
        let app = tauri::Builder::default()
            .any_thread()
            .manage(gate.clone())
            .plugin(crate::linux_single_instance::plugin())
            .build(tauri::generate_context!())
            .unwrap();
        let owner = app.state::<crate::linux_single_instance::InstanceOwner>();
        assert!(owner.owns_name().unwrap());
        let callbacks_before = crate::linux_single_instance::test_callback_count();

        let binary = std::env::var_os("VERENU_HANDOFF_TEST_BINARY").expect("replacement binary");
        let root =
            std::env::temp_dir().join(format!("verenu-replacement-{}", uuid::Uuid::new_v4()));
        let data = root.join("data");
        let state = root.join("state");
        let config = root.join("config");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(data.join("settings.json"), b"{\"setup_complete\":true}").unwrap();
        let parent_db = crate::data::db::open(data.join("verenu.db")).unwrap();
        let mut sync_identity = crate::sync::identity::generate_for_tests();
        sync_identity.name = "Test device".into();
        let sync_key = sync_identity.tls_key();
        crate::sync::secrets::store_identity_key(&data, sync_key.secret_pkcs8_der()).unwrap();
        std::fs::write(data.join("sync-identity.der"), sync_identity.cert_der().as_ref()).unwrap();
        {
            let connection = parent_db.lock().unwrap();
            crate::sync::store::ensure_self_identity(&connection, &sync_identity.uuid, &sync_identity.name).unwrap();
        }
        let original_db = std::fs::read(data.join("verenu.db")).unwrap();
        let original_wal = std::fs::read(data.join("verenu.db-wal")).unwrap();
        let original_settings = std::fs::read(data.join("settings.json")).unwrap();
        let mut original_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        original_files.sort();
        let (callback_queue, callback_path) = crate::linux_handoff_callbacks::CallbackQueue::create_private().unwrap();
        let callback_queue = std::sync::Arc::new(callback_queue);
        let mut missing_queue = Command::new(&binary);
        missing_queue
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            missing_queue,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        ).unwrap().with_parent_unique_name(owner.unique_name().unwrap());
        assert!(matches!(replacement.prepare(), Err(error) if error.contains("callback queue is missing")));
        assert!(owner.owns_name().unwrap());
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        )
        .unwrap()
        .with_parent_unique_name(owner.unique_name().unwrap())
        .with_callback_queue(callback_path.clone());
        let child_pid = replacement.process.0.id();
        let ready = replacement
            .prepare()
            .expect("replacement did not complete frontend readiness");
        let pause = gate.begin_pause_with(|| owner.install_callback_queue(callback_queue.clone())).unwrap();
        tauri::async_runtime::block_on(pause.wait_drained());
        let connection = zbus::blocking::Connection::session().unwrap();
        connection.call_method(
            Some("com.verenu.app.SingleInstance"),
            "/com/verenu/app/SingleInstance",
            Some("org.SingleInstance.DBus"),
            "ExecuteCallback",
            &(vec!["--verenu-hotkey-release".to_string()], String::new()),
        ).unwrap();
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable)
            .unwrap();
        let owned = ready
            .acquire_ownership()
            .expect("replacement did not acquire ownership");
        assert!(!owner.owns_name().unwrap());
        let (sent, received) = std::sync::mpsc::channel();
        let callback_connection = connection.clone();
        let callback = std::thread::spawn(move || {
            let result = callback_connection.call_method(
                Some("com.verenu.app.SingleInstance"),
                "/com/verenu/app/SingleInstance",
                Some("org.SingleInstance.DBus"),
                "ExecuteCallback",
                &(vec!["--verenu-hotkey-handsfree".to_string()], String::new()),
            );
            let _ = sent.send(result.is_ok());
        });
        assert!(received.recv_timeout(Duration::from_secs(5)).unwrap());
        assert_eq!(callback_queue.first().unwrap().unwrap().1, crate::linux_handoff_callbacks::Callback::Release);
        owner.callback_barrier().unwrap();
        owner.replacement_callback_barrier(child_pid).unwrap();
        assert_eq!(
            crate::linux_single_instance::test_callback_count(),
            callbacks_before
        );
        let outcome = owned.activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("Replacement shortcuts could not start")));
        let tray_names = registrations.lock().unwrap().clone();
        assert_eq!(tray_names.len(), 1, "replacement tray did not register");
        let tray_alive: bool = watcher
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "NameHasOwner",
                &(tray_names[0].as_str(),),
            )
            .unwrap()
            .body()
            .deserialize()
            .unwrap();
        assert!(!tray_alive, "failed replacement left its tray running");
        assert_eq!(unsafe { libc::kill(child_pid as i32, 0) }, -1);
        assert!(!group_has_live_members(child_pid).unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(
                Instant::now() < deadline,
                "parent did not regain single-instance ownership"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive)
            .unwrap();
        owner.drain_callback_queue(app.handle()).unwrap();
        drop(pause);
        callback.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while crate::linux_single_instance::test_callback_count() == callbacks_before {
            assert!(Instant::now() < deadline, "parent did not handle the deferred callback");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks_before + 2);
        connection
            .call_method(
                Some("com.verenu.app.SingleInstance"),
                "/com/verenu/app/SingleInstance",
                Some("org.SingleInstance.DBus"),
                "ExecuteCallback",
                &(vec!["--verenu-hotkey-release".to_string()], String::new()),
            )
            .unwrap();
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), original_db);
        assert_eq!(
            std::fs::read(data.join("verenu.db-wal")).unwrap(),
            original_wal
        );
        assert_eq!(
            std::fs::read(data.join("settings.json")).unwrap(),
            original_settings
        );
        let mut final_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        final_files.sort();
        assert_eq!(final_files, original_files);
        assert!(
            !state.exists(),
            "replacement started output-volume recovery"
        );

        // A competing writer must abort activation before the replacement
        // registers its tray, leaving the original instance in control.
        let writer = parent_db.lock().unwrap();
        writer.execute_batch("BEGIN IMMEDIATE;").unwrap();
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        )
        .unwrap()
        .with_parent_unique_name(owner.unique_name().unwrap())
        .with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable)
            .unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("cannot prepare the database")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(
                Instant::now() < deadline,
                "parent did not regain ownership after database failure"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive)
            .unwrap();
        assert_eq!(registrations.lock().unwrap().len(), 1);
        writer.execute_batch("ROLLBACK;").unwrap();
        drop(writer);
        drop(pause);
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), original_db);
        assert_eq!(
            std::fs::read(data.join("verenu.db-wal")).unwrap(),
            original_wal
        );
        assert_eq!(
            std::fs::read(data.join("settings.json")).unwrap(),
            original_settings
        );

        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        )
        .unwrap()
        .with_parent_unique_name(owner.unique_name().unwrap())
        .with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let updated_settings = b"{\"setup_complete\":true,\"test_changed\":true}";
        std::fs::write(data.join("settings.json"), updated_settings).unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable)
            .unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("Settings changed")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(
                Instant::now() < deadline,
                "parent did not regain ownership after settings changed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive)
            .unwrap();
        drop(pause);
        assert_eq!(
            std::fs::read(data.join("settings.json")).unwrap(),
            updated_settings
        );
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), original_db);
        assert_eq!(
            std::fs::read(data.join("verenu.db-wal")).unwrap(),
            original_wal
        );
        assert_eq!(registrations.lock().unwrap().len(), 1);

        std::fs::write(data.join("settings.json"), &original_settings).unwrap();
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        )
        .unwrap()
        .with_parent_unique_name(owner.unique_name().unwrap())
        .with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        parent_db
            .lock()
            .unwrap()
            .execute_batch("CREATE TABLE handoff_fixture (value INTEGER); INSERT INTO handoff_fixture VALUES (1);")
            .unwrap();
        let updated_db = std::fs::read(data.join("verenu.db")).unwrap();
        let updated_wal = std::fs::read(data.join("verenu.db-wal")).unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable)
            .unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("Database changed")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(
                Instant::now() < deadline,
                "parent did not regain ownership after database changed"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive)
            .unwrap();
        drop(pause);
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), updated_db);
        assert_eq!(std::fs::read(data.join("verenu.db-wal")).unwrap(), updated_wal);
        assert_eq!(registrations.lock().unwrap().len(), 1);

        // A port already in use must fail preparation while the parent stays alive.
        let reserved = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let occupied_port = reserved.local_addr().unwrap().port();
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env("VERENU_AGENT_TOKEN", "a".repeat(64))
            .env("VERENU_AGENT_PORT", occupied_port.to_string())
            .env_remove("VERENU_HANDOFF_TEST_SHORTCUTS");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        ).unwrap().with_parent_unique_name(owner.unique_name().unwrap()).with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable).unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("agent service could not prepare")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(Instant::now() < deadline, "parent did not regain ownership after agent failure");
            std::thread::sleep(Duration::from_millis(20));
        }
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive).unwrap();
        drop(pause);
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), updated_db);
        assert_eq!(std::fs::read(data.join("verenu.db-wal")).unwrap(), updated_wal);
        assert_eq!(registrations.lock().unwrap().len(), 2);
        assert!(!data.join("agent-session.json").exists());
        drop(reserved);

        // A controlled shortcut driver exercises later startup on a desktop
        // without claiming that a real Hyprland portal has been verified.
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env("VERENU_HANDOFF_TEST_SHORTCUTS", "1");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        ).unwrap().with_parent_unique_name(owner.unique_name().unwrap()).with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable).unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("services are not yet verified")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(Instant::now() < deadline, "parent did not regain ownership after shortcut probe");
            std::thread::sleep(Duration::from_millis(20));
        }
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive).unwrap();
        drop(pause);
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), updated_db);
        assert_eq!(std::fs::read(data.join("verenu.db-wal")).unwrap(), updated_wal);
        assert_eq!(registrations.lock().unwrap().len(), 3);
        assert!(!data.join("agent-session.json").exists());
        assert!(crate::sync::secrets::load_identity_key(&data).unwrap().as_slice() == sync_key.secret_pkcs8_der());

        std::fs::write(data.join("settings.json"), b"{\"setup_complete\":true,\"cleanup_cache_enabled\":false}").unwrap();
        let mut command = Command::new(&binary);
        command.env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env("VERENU_HANDOFF_TEST_SHORTCUTS", "1");
        let replacement = Replacement::spawn(
            command, crate::data::db::HANDOFF_SCHEMA_VERSION, KeyAvailability::new(), Duration::from_secs(25),
        ).unwrap().with_parent_unique_name(owner.unique_name().unwrap()).with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable).unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("Cleanup cache settings require a manual restart")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(Instant::now() < deadline, "parent did not regain ownership after cache mismatch");
            std::thread::sleep(Duration::from_millis(20));
        }
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive).unwrap();
        drop(pause);
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), updated_db);
        assert_eq!(std::fs::read(data.join("verenu.db-wal")).unwrap(), updated_wal);
        std::fs::write(data.join("settings.json"), &original_settings).unwrap();

        std::fs::remove_file(data.join("sync-identity.key")).unwrap();
        let mut command = Command::new(&binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
            .env("XDG_CONFIG_HOME", &config)
            .env_remove("HYPRLAND_INSTANCE_SIGNATURE")
            .env_remove("VERENU_AGENT_TOKEN")
            .env_remove("VERENU_AGENT_PORT")
            .env("VERENU_HANDOFF_TEST_SHORTCUTS", "1");
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        ).unwrap().with_parent_unique_name(owner.unique_name().unwrap()).with_callback_queue(callback_path.clone());
        let ready = replacement.prepare().unwrap();
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable).unwrap();
        let outcome = ready.acquire_ownership().unwrap().activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("Replacement sync could not prepare")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !owner.owns_name().unwrap_or(false) {
            assert!(Instant::now() < deadline, "parent did not regain ownership after sync failure");
            std::thread::sleep(Duration::from_millis(20));
        }
        owner.set_ownership_mode(crate::linux_single_instance::OwnershipMode::Exclusive).unwrap();
        drop(pause);
        assert!(!data.join("sync-identity.key").exists());
        assert_eq!(std::fs::read(data.join("verenu.db")).unwrap(), updated_db);
        assert_eq!(std::fs::read(data.join("verenu.db-wal")).unwrap(), updated_wal);
        assert_eq!(registrations.lock().unwrap().len(), 5);
        drop(callback_queue);
        std::fs::remove_dir_all(callback_path.parent().unwrap()).unwrap();
        drop(parent_db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
