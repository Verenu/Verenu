use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct StagedBackup(PathBuf);

impl Drop for StagedBackup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn write_backup(path: &Path, bytes: &[u8]) -> io::Result<()> {
    stage_backup(path, |file| file.write_all(bytes))
}

fn stage_backup(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    // Exclusive creation prevents concurrent exports or stale staging files
    // from sharing a writer. Staging beside the destination keeps rename on
    // the same filesystem. Never open the destination for writing.
    let mut created = None;
    for _ in 0..32 {
        let candidate = parent.join(format!(
            ".verenu-backup-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => {
                created = Some((StagedBackup(candidate), file));
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    let (staged, mut file) = created.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Could not create backup staging file",
        )
    })?;
    write(&mut file)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&staged.0, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "verenu-export-test-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn destination(&self) -> PathBuf {
            self.0.join("backup.json")
        }
        fn assert_no_staging_files(&self) {
            assert_eq!(fs::read_dir(&self.0).unwrap().count(), 1);
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn interrupted_export_preserves_existing_destination() {
        let fixture = Fixture::new();
        let destination = fixture.destination();
        fs::write(&destination, b"previous good backup").unwrap();
        let result = stage_backup(&destination, |file| {
            file.write_all(b"partial new backup")?;
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "synthetic interruption",
            ))
        });
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(destination).unwrap(), b"previous good backup");
        fixture.assert_no_staging_files();
    }

    #[test]
    fn abrupt_export_exit_preserves_existing_destination() {
        const CHILD_DESTINATION: &str = "VERENU_TEST_EXPORT_DESTINATION";
        if let Some(destination) = std::env::var_os(CHILD_DESTINATION) {
            let _ = stage_backup(Path::new(&destination), |file| {
                file.write_all(b"partial new backup")?;
                std::process::exit(73);
            });
            panic!("child did not reach the interrupted write");
        }
        let fixture = Fixture::new();
        let destination = fixture.destination();
        fs::write(&destination, b"previous good backup").unwrap();
        let test_name = format!(
            "{}::abrupt_export_exit_preserves_existing_destination",
            module_path!().split_once("::").unwrap().1
        );
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name])
            .env(CHILD_DESTINATION, &destination)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        assert_eq!(fs::read(destination).unwrap(), b"previous good backup");
        // Abrupt process death can leave staging behind, but cannot publish it.
        assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
    }

    #[test]
    fn export_replaces_destination_only_after_complete_write() {
        let fixture = Fixture::new();
        let destination = fixture.destination();
        write_backup(&destination, b"previous good backup").unwrap();
        stage_backup(&destination, |file| {
            file.write_all(b"complete new backup")?;
            assert_eq!(fs::read(&destination)?, b"previous good backup");
            Ok(())
        })
        .unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"complete new backup");
        fixture.assert_no_staging_files();
    }

    #[test]
    fn failed_export_rename_preserves_destination_and_cleans_staging() {
        let fixture = Fixture::new();
        let destination = fixture.destination();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("good.json"), b"previous good backup").unwrap();
        assert!(write_backup(&destination, b"new backup").is_err());
        assert_eq!(
            fs::read(destination.join("good.json")).unwrap(),
            b"previous good backup"
        );
        fixture.assert_no_staging_files();
    }
}
