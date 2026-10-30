//! Private, bounded messages for a future two-process Linux update handoff.
//!
//! A readiness message is only meaningful after the caller has completed the
//! corresponding real startup phase. This transport does not start a process,
//! transfer D-Bus ownership, or authorize the old process to exit.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 2;
const MAX_MESSAGE: usize = 4096;
const CHILD_FD_ENV: &str = "VERENU_HANDOFF_FD";
pub(crate) type KeyAvailability = BTreeMap<String, bool>;

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
    if !valid || !descends_from(unsafe { libc::getppid() }, peer.pid) {
        return Err("Invalid restart connection".into());
    }
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err("Could not secure the restart connection".into());
    }
    // The descriptor was explicitly passed to this process and validated above.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    Channel::new(stream, timeout).map(Some)
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
        })?;
        match self.channel.receive()? {
            Message::Begin {
                key_availability,
                parent_unique_name,
            } => Ok(ChildStartup {
                channel: self.channel,
                key_availability,
                parent_unique_name,
            }),
            _ => Err("Replacement received an invalid startup request".into()),
        }
    }
}

pub(crate) struct ChildStartup {
    channel: Channel,
    key_availability: KeyAvailability,
    parent_unique_name: Option<String>,
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
        Ok(ChildCommit)
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.0.send(&Message::Failed { reason })
    }
}

pub(crate) struct ChildCommit;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Message {
    Hello {
        protocol: u32,
        pid: u32,
        schema: i64,
    },
    Begin {
        key_availability: KeyAvailability,
        parent_unique_name: Option<String>,
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
}

impl Channel {
    fn new(stream: UnixStream, timeout: Duration) -> Result<Self, String> {
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| error.to_string())?;
        Ok(Self { stream, timeout })
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
}

struct ProcessGuard(std::process::Child, UnixStream);

fn group_has_live_members(group: u32) -> Result<bool, String> {
    let entries = std::fs::read_dir("/proc").map_err(|error| error.to_string())?;
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
            .is_none()
        {
            continue;
        }
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        let Some((_, fields)) = stat.rsplit_once(')') else {
            continue;
        };
        let mut fields = fields.split_whitespace();
        let state = fields.next();
        let _parent = fields.next();
        let process_group = fields.next().and_then(|value| value.parse::<u32>().ok());
        if process_group == Some(group) && !matches!(state, Some("Z" | "X")) {
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

impl Parent {
    fn prepare(mut self) -> Result<Probation, String> {
        match self.channel.receive()? {
            Message::Hello {
                protocol,
                pid,
                schema,
            } if protocol == PROTOCOL
                && schema == self.schema
                && (pid == self.child_pid
                    || (pid <= i32::MAX as u32
                        && self.child_pid <= i32::MAX as u32
                        && descends_from(pid as i32, self.child_pid as i32))) => {}
            _ => return Err("Replacement identity or compatibility did not match".into()),
        }
        self.channel.send(&Message::Begin {
            key_availability: self.key_availability,
            parent_unique_name: self.parent_unique_name,
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

    fn begin() -> Message {
        Message::Begin {
            key_availability: KeyAvailability::new(),
            parent_unique_name: None,
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
    fn wrong_pid_or_schema_is_rejected_before_startup() {
        for message in [
            Message::Hello {
                protocol: PROTOCOL,
                pid: 43,
                schema: 39,
            },
            Message::Hello {
                protocol: PROTOCOL,
                pid: 42,
                schema: 40,
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
            startup
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
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("settings.json"), b"{\"setup_complete\":true}").unwrap();
        let parent_db = crate::data::db::open(data.join("verenu.db")).unwrap();
        let original_db = std::fs::read(data.join("verenu.db")).unwrap();
        let original_wal = std::fs::read(data.join("verenu.db-wal")).unwrap();
        let original_settings = std::fs::read(data.join("settings.json")).unwrap();
        let mut original_files: Vec<_> = std::fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        original_files.sort();
        let mut command = Command::new(binary);
        command
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state);
        let replacement = Replacement::spawn(
            command,
            crate::data::db::HANDOFF_SCHEMA_VERSION,
            KeyAvailability::new(),
            Duration::from_secs(25),
        )
        .unwrap()
        .with_parent_unique_name(owner.unique_name().unwrap());
        let child_pid = replacement.process.0.id();
        let ready = replacement
            .prepare()
            .expect("replacement did not complete frontend readiness");
        let pause = tauri::async_runtime::block_on(gate.pause()).unwrap();
        owner
            .set_ownership_mode(crate::linux_single_instance::OwnershipMode::Transferable)
            .unwrap();
        let owned = ready
            .acquire_ownership()
            .expect("replacement did not acquire ownership");
        assert!(!owner.owns_name().unwrap());
        let connection = zbus::blocking::Connection::session().unwrap();
        let started = crate::linux_single_instance::test_callback_started();
        let (sent, received) = std::sync::mpsc::channel();
        let callback_connection = connection.clone();
        let callback = std::thread::spawn(move || {
            let result = callback_connection.call_method(
                Some("com.verenu.app.SingleInstance"),
                "/com/verenu/app/SingleInstance",
                Some("org.SingleInstance.DBus"),
                "ExecuteCallback",
                &(vec!["--verenu-hotkey-release".to_string()], String::new()),
            );
            let _ = sent.send(result.is_ok());
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while crate::linux_single_instance::test_callback_started() == started {
            assert!(Instant::now() < deadline, "forwarded callback did not reach parent");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            crate::linux_single_instance::test_callback_count(),
            callbacks_before
        );
        let outcome = owned.activate();
        assert!(matches!(outcome, Err(reason) if reason.contains("services are not yet verified")));
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
        drop(pause);
        let _ = received.recv_timeout(Duration::from_secs(5)).unwrap();
        callback.join().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while crate::linux_single_instance::test_callback_count() == callbacks_before {
            assert!(Instant::now() < deadline, "parent did not handle the deferred callback");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(crate::linux_single_instance::test_callback_count(), callbacks_before + 1);
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
        drop(parent_db);
        std::fs::remove_dir_all(root).unwrap();
    }
}
