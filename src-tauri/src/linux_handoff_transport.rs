//! Private, bounded messages for a future two-process Linux update handoff.
//!
//! A readiness message is only meaningful after the caller has completed the
//! corresponding real startup phase. This transport does not start a process,
//! transfer D-Bus ownership, or authorize the old process to exit.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 1;
const MAX_MESSAGE: usize = 4096;
const CHILD_FD_ENV: &str = "VERENU_HANDOFF_FD";

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
        self.channel.expect(Message::Begin)?;
        Ok(ChildStartup(self.channel))
    }
}

pub(crate) struct ChildStartup(Channel);

impl ChildStartup {
    pub(crate) fn backend_ready(mut self) -> Result<ChildFrontends, String> {
        self.0.send(&Message::BackendReady)?;
        Ok(ChildFrontends(self.0))
    }

    pub(crate) fn fail(mut self, reason: String) -> Result<(), String> {
        self.0.send(&Message::Failed { reason })
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
    Begin,
    BackendReady,
    FrontendsReady,
    Activate,
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
        self.channel.send(&Message::Begin)?;
        self.channel.expect(Message::BackendReady)?;
        self.channel.expect(Message::FrontendsReady)?;
        Ok(Probation(self.channel))
    }
}

struct Probation(Channel);

impl Probation {
    fn activate(mut self) -> Result<Activated, String> {
        self.0.send(&Message::Activate)?;
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

    fn pair() -> (Parent, Channel) {
        let (parent, child) = UnixStream::pair().unwrap();
        let timeout = Duration::from_secs(2);
        (
            Parent {
                channel: Channel::new(parent, timeout).unwrap(),
                child_pid: 42,
                schema: 39,
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
            child.expect(Message::Begin).unwrap();
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
            child.expect(Message::Begin).unwrap();
            child.send(&Message::BackendReady).unwrap();
            child.send(&Message::FrontendsReady).unwrap();
        });
        let probation = parent.prepare().unwrap();
        worker.join().unwrap();
        assert!(probation.activate().is_err());
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
            child.expect(Message::Begin).unwrap();
            child.send(&Message::BackendReady).unwrap();
            child.send(&Message::FrontendsReady).unwrap();
            child.expect(Message::Activate).unwrap();
            child.send(&Message::Activated).unwrap();
            child.expect(Message::Commit).unwrap();
        });
        parent
            .prepare()
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
            child.expect(Message::Begin).unwrap();
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
        let _frontends = child.begin(39).unwrap().backend_ready().unwrap();
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

    #[cfg(feature = "native-testing")]
    #[test]
    #[ignore = "requires a private Linux desktop and a custom-protocol app build"]
    fn real_replacement_opens_read_only_and_keeps_parent_alive() {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

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
        let (parent, child) = UnixStream::pair().unwrap();
        let fd = child.as_raw_fd();
        let mut command = Command::new(binary);
        command
            .env(CHILD_FD_ENV, fd.to_string())
            .env("VERENU_APP_DATA_DIR_OVERRIDE", &data)
            .env("XDG_STATE_HOME", &state)
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
        struct Replacement(std::process::Child);
        impl Drop for Replacement {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut replacement = Replacement(command.spawn().unwrap());
        drop(child);
        let mut parent = Channel::new(parent, Duration::from_secs(25)).unwrap();
        assert_eq!(
            parent.receive().unwrap(),
            Message::Hello {
                protocol: PROTOCOL,
                pid: replacement.0.id(),
                schema: crate::data::db::HANDOFF_SCHEMA_VERSION,
            }
        );
        parent.send(&Message::Begin).unwrap();
        parent.expect(Message::BackendReady).unwrap();
        assert!(
            matches!(parent.receive(), Ok(Message::Failed { reason }) if reason.contains("not yet verified"))
        );
        replacement.0.wait().unwrap();
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
