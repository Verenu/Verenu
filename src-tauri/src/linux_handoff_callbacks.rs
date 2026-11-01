//! Transactional callback storage shared only during a Linux restart.
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use std::{path::{Path, PathBuf}, sync::Mutex, time::Duration};

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

    fn code(self) -> i64 {
        match self {
            Self::Release => 0,
            Self::Handsfree => 1,
            Self::CaptureSubApp => 2,
            Self::Show => 3,
        }
    }

    fn parse(code: i64) -> rusqlite::Result<Self> {
        match code {
            0 => Ok(Self::Release),
            1 => Ok(Self::Handsfree),
            2 => Ok(Self::CaptureSubApp),
            3 => Ok(Self::Show),
            _ => Err(rusqlite::Error::InvalidQuery),
        }
    }
}

pub(crate) struct CallbackQueue(Mutex<Connection>);
const MAX_PENDING: i64 = 4096;

impl CallbackQueue {
    pub(crate) fn create_private() -> anyhow::Result<(Self, PathBuf)> {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
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
        let path = directory.join("callbacks.sqlite");
        match Self::create(&path) {
            Ok(queue) => Ok((queue, path)),
            Err(error) => {
                let _ = std::fs::remove_dir_all(directory);
                Err(error.into())
            }
        }
    }

    pub(crate) fn create(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )?;
        connection.busy_timeout(Duration::from_secs(3))?;
        connection.execute_batch(
            "CREATE TABLE callbacks (id INTEGER PRIMARY KEY AUTOINCREMENT, kind INTEGER NOT NULL CHECK(kind BETWEEN 0 AND 3), source INTEGER NOT NULL DEFAULT 0 CHECK(source BETWEEN 0 AND 1));",
        )?;
        Ok(Self(Mutex::new(connection)))
    }

    pub(crate) fn open(path: &Path) -> rusqlite::Result<Self> {
        let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        connection.busy_timeout(Duration::from_secs(3))?;
        // Reject a missing or incompatible queue before accepting callbacks.
        connection.prepare("SELECT id, kind, source FROM callbacks LIMIT 0")?;
        Ok(Self(Mutex::new(connection)))
    }

    pub(crate) fn push(&self, callback: Callback) -> anyhow::Result<()> {
        self.push_from(callback, 0)
    }

    pub(crate) fn push_replacement(&self, callback: Callback) -> anyhow::Result<()> {
        self.push_from(callback, 1)
    }

    fn push_from(&self, callback: Callback, source: i64) -> anyhow::Result<()> {
        let connection = self.0.lock().map_err(|_| anyhow::anyhow!("Callback queue is unavailable"))?;
        let inserted = connection.execute(
            "INSERT INTO callbacks (kind, source) SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM callbacks) < ?3",
            [callback.code(), source, MAX_PENDING],
        )?;
        anyhow::ensure!(inserted == 1, "Restart callback queue is full");
        Ok(())
    }

    pub(crate) fn first(&self) -> rusqlite::Result<Option<(i64, Callback)>> {
        let connection = self.0.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
        // The old owner must be drained through its D-Bus barrier before
        // replay: it can finish an earlier callback after the new owner writes.
        let row: Option<(i64, i64)> = connection
            .query_row("SELECT id, kind FROM callbacks ORDER BY source, id LIMIT 1", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?;
        row.map(|(id, code)| Ok((id, Callback::parse(code)?))).transpose()
    }

    pub(crate) fn acknowledge(&self, id: i64) -> rusqlite::Result<bool> {
        let connection = self.0.lock().map_err(|_| rusqlite::Error::InvalidQuery)?;
        Ok(connection.execute(
            "DELETE FROM callbacks WHERE id = ?1 AND id = (SELECT id FROM callbacks ORDER BY source, id LIMIT 1)",
            [id],
        )? == 1)
    }
}

#[cfg(test)]
mod tests {
    use super::{Callback, CallbackQueue};

    #[test]
    fn committed_callbacks_survive_another_process_exiting() {
        const CHILD_PATH: &str = "VERENU_TEST_CALLBACK_QUEUE";
        if let Some(path) = std::env::var_os(CHILD_PATH) {
            let queue = CallbackQueue::open(std::path::Path::new(&path)).unwrap();
            queue.push(Callback::Release).unwrap();
            queue.push(Callback::Handsfree).unwrap();
            let connection = rusqlite::Connection::open(std::path::Path::new(&path)).unwrap();
            connection.execute_batch("BEGIN IMMEDIATE; INSERT INTO callbacks (kind) VALUES (2);").unwrap();
            // Simulate a replacement dying without running destructors.
            std::process::exit(17);
        }

        use std::os::unix::fs::PermissionsExt;
        let (queue, path) = CallbackQueue::create_private().unwrap();
        let directory = path.parent().unwrap().to_path_buf();
        assert_eq!(std::fs::metadata(&directory).unwrap().permissions().mode() & 0o777, 0o700);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "linux_handoff_callbacks::tests::committed_callbacks_survive_another_process_exiting"])
            .env(CHILD_PATH, &path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(17));
        let (first, callback) = queue.first().unwrap().unwrap();
        assert_eq!(callback, Callback::Release);
        assert!(!queue.acknowledge(first + 1).unwrap());
        assert!(queue.acknowledge(first).unwrap());
        let (second, callback) = queue.first().unwrap().unwrap();
        assert_eq!(callback, Callback::Handsfree);
        assert!(queue.acknowledge(second).unwrap());
        assert_eq!(queue.first().unwrap(), None);
        drop(queue);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn classification_keeps_the_existing_hotkey_precedence() {
        assert_eq!(Callback::from_args(&["--verenu-hotkey-handsfree".into(), "--verenu-hotkey-release".into()]), Callback::Release);
        assert_eq!(Callback::from_args(&["--verenu-capture-sub-app".into()]), Callback::CaptureSubApp);
        assert_eq!(Callback::from_args(&["unrelated".into()]), Callback::Show);
    }

    #[test]
    fn parent_callbacks_precede_replacement_callbacks_even_if_written_later() {
        let (parent, path) = CallbackQueue::create_private().unwrap();
        let replacement = CallbackQueue::open(&path).unwrap();
        replacement.push_replacement(Callback::Handsfree).unwrap();
        let (replacement_id, _) = replacement.first().unwrap().unwrap();
        parent.push(Callback::Release).unwrap();
        let (parent_id, callback) = parent.first().unwrap().unwrap();
        assert_eq!(callback, Callback::Release);
        assert!(!parent.acknowledge(replacement_id).unwrap());
        assert!(parent.acknowledge(parent_id).unwrap());
        let (id, callback) = parent.first().unwrap().unwrap();
        assert_eq!((id, callback), (replacement_id, Callback::Handsfree));
        assert!(parent.acknowledge(id).unwrap());
        drop(replacement);
        drop(parent);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
