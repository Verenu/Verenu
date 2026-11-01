//! Callback handoff between the sole running owner and its replacement.
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{ffi::OsStrExt, fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt}},
    },
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_PENDING: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Callback {
    Release,
    Handsfree,
    CaptureSubApp,
    Show,
}

impl Callback {
    pub(crate) fn from_args(args: &[String]) -> Self {
        if args.iter().any(|arg| arg == "--verenu-hotkey-release") {
            Self::Release
        } else if args.iter().any(|arg| arg == "--verenu-hotkey-handsfree") {
            Self::Handsfree
        } else if args.iter().any(|arg| arg == "--verenu-capture-sub-app") {
            Self::CaptureSubApp
        } else {
            Self::Show
        }
    }

    fn code(self) -> u8 {
        match self {
            Self::Release => 0,
            Self::Handsfree => 1,
            Self::CaptureSubApp => 2,
            Self::Show => 3,
        }
    }

    fn parse(code: u8) -> io::Result<Self> {
        match code {
            0 => Ok(Self::Release),
            1 => Ok(Self::Handsfree),
            2 => Ok(Self::CaptureSubApp),
            3 => Ok(Self::Show),
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid restart callback")),
        }
    }
}

struct State {
    pipe: File,
    pending: VecDeque<(i64, Callback)>,
    next: i64,
}

pub(crate) struct CallbackQueue(Mutex<State>);
pub(crate) struct CallbackReader(File);

impl CallbackQueue {
    pub(crate) fn create_private() -> anyhow::Result<(Self, PathBuf)> {
        let safe_root = |path: &Path| {
            std::fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.is_dir() && !metadata.file_type().is_symlink()
                    && ((metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0)
                        || (metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
            })
        };
        let root = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute() && safe_root(path))
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        anyhow::ensure!(safe_root(&root), "No secure runtime directory is available for restart");
        let directory = root.join(format!("verenu-handoff-{}", uuid::Uuid::new_v4()));
        std::fs::DirBuilder::new().mode(0o700).create(&directory)?;
        let path = directory.join("callbacks.fifo");
        let result = (|| -> anyhow::Result<Self> {
            let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
            if unsafe { libc::mkfifo(name.as_ptr(), 0o600) } != 0 {
                return Err(io::Error::last_os_error().into());
            }
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            let pipe = OpenOptions::new().read(true).write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW).open(&path)?;
            let capacity = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETPIPE_SZ) };
            anyhow::ensure!(capacity >= MAX_PENDING as i32, "Restart callback pipe is too small");
            Ok(Self(Mutex::new(State {
                pipe,
                pending: VecDeque::new(),
                next: 0,
            })))
        })();
        match result {
            Ok(queue) => Ok((queue, path)),
            Err(error) => {
                let _ = std::fs::remove_dir_all(directory);
                Err(error)
            }
        }
    }

    pub(crate) fn push(&self, callback: Callback) -> anyhow::Result<()> {
        let mut state = self.0.lock().map_err(|_| anyhow::anyhow!("Callback queue is unavailable"))?;
        anyhow::ensure!(state.pending.len() < MAX_PENDING, "Restart callback queue is full");
        state.pipe.write_all(&[callback.code()])?;
        let id = state.next;
        state.next += 1;
        state.pending.push_back((id, callback));
        Ok(())
    }

    pub(crate) fn first(&self) -> io::Result<Option<(i64, Callback)>> {
        let state = self.0.lock().map_err(|_| io::Error::other("Callback queue is unavailable"))?;
        Ok(state.pending.front().copied())
    }

    pub(crate) fn acknowledge(&self, id: i64) -> io::Result<bool> {
        let mut state = self.0.lock().map_err(|_| io::Error::other("Callback queue is unavailable"))?;
        let Some(&(front_id, callback)) = state.pending.front() else { return Ok(false) };
        if front_id != id {
            return Ok(false);
        }
        let mut byte = [0];
        state.pipe.read_exact(&mut byte)?;
        if byte[0] != callback.code() {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "Restart callback pipe is out of order"));
        }
        state.pending.pop_front();
        Ok(true)
    }
}

impl CallbackReader {
    pub(crate) fn open(path: &Path) -> anyhow::Result<Self> {
        let file = OpenOptions::new().read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC | libc::O_NOFOLLOW).open(path)?;
        let metadata = file.metadata()?;
        anyhow::ensure!(
            metadata.file_type().is_fifo()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Restart callback pipe is unsafe"
        );
        Ok(Self(file))
    }

    pub(crate) fn drain(&mut self, mut deliver: impl FnMut(Callback)) -> io::Result<()> {
        let mut bytes = [0u8; 256];
        loop {
            match self.0.read(&mut bytes) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    for &byte in &bytes[..count] {
                        deliver(Callback::parse(byte)?);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Callback, CallbackQueue, CallbackReader};

    #[test]
    fn pipe_keeps_callbacks_in_order_after_original_closes() {
        let (queue, path) = CallbackQueue::create_private().unwrap();
        queue.push(Callback::Release).unwrap();
        let mut reader = CallbackReader::open(&path).unwrap();
        queue.push(Callback::Handsfree).unwrap();
        assert_eq!(queue.first().unwrap().unwrap().1, Callback::Release);
        drop(queue);
        let mut received = Vec::new();
        reader.drain(|callback| received.push(callback)).unwrap();
        assert_eq!(received, [Callback::Release, Callback::Handsfree]);
        drop(reader);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn original_can_replay_callbacks_if_replacement_stops() {
        let (queue, path) = CallbackQueue::create_private().unwrap();
        drop(CallbackReader::open(&path).unwrap());
        queue.push(Callback::Release).unwrap();
        queue.push(Callback::Handsfree).unwrap();
        let (first, callback) = queue.first().unwrap().unwrap();
        assert_eq!(callback, Callback::Release);
        assert!(!queue.acknowledge(first + 1).unwrap());
        assert!(queue.acknowledge(first).unwrap());
        assert_eq!(queue.first().unwrap().unwrap().1, Callback::Handsfree);
        let mut reader = CallbackReader::open(&path).unwrap();
        drop(queue);
        let mut received = Vec::new();
        reader.drain(|callback| received.push(callback)).unwrap();
        assert_eq!(received, [Callback::Handsfree]);
        drop(reader);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn full_pipe_rejects_new_callbacks_without_blocking() {
        let (queue, path) = CallbackQueue::create_private().unwrap();
        let mut reader = CallbackReader::open(&path).unwrap();
        for _ in 0..super::MAX_PENDING { queue.push(Callback::Release).unwrap(); }
        assert!(queue.push(Callback::Show).is_err());
        drop(queue);
        let mut count = 0;
        reader.drain(|callback| { assert_eq!(callback, Callback::Release); count += 1; }).unwrap();
        assert_eq!(count, super::MAX_PENDING);
        drop(reader);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
