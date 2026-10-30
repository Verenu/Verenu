//! Private, bounded messages for a future two-process Linux update handoff.
//!
//! A readiness message is only meaningful after the caller has completed the
//! corresponding real startup phase. This transport does not start a process,
//! transfer D-Bus ownership, or authorize the old process to exit.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const PROTOCOL: u32 = 1;
const MAX_MESSAGE: usize = 4096;

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
        self.channel.expect(Message::Hello {
            protocol: PROTOCOL,
            pid: self.child_pid,
            schema: self.schema,
        })?;
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
        use std::os::fd::FromRawFd;
        let Ok(fd) = std::env::var("VERENU_HANDOFF_TEST_FD") else {
            return;
        };
        let stream = unsafe { UnixStream::from_raw_fd(fd.parse().unwrap()) };
        let mut child = Channel::new(stream, Duration::from_secs(5)).unwrap();
        child
            .send(&Message::Hello {
                protocol: PROTOCOL,
                pid: std::process::id(),
                schema: 39,
            })
            .unwrap();
        child.expect(Message::Begin).unwrap();
        child.send(&Message::BackendReady).unwrap();
        // Exit before frontend readiness to model a failed replacement.
    }

    #[test]
    fn replacement_process_exit_after_backend_leaves_parent_alive() {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};

        let (parent, child) = UnixStream::pair().unwrap();
        let fd = child.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "linux_handoff_transport::tests::replacement_process_fixture",
                "--nocapture",
            ])
            .env("VERENU_HANDOFF_TEST_FD", fd.to_string())
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
}
