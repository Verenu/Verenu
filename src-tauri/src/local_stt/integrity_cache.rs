//! A loaded helper can reuse verification while its installed files are unchanged.
//! Replacement or modification invalidates the cache before native loading.
//! Platforms without file identity and change-time stamps always reverify.
use std::path::PathBuf;

#[derive(PartialEq, Eq)]
struct Stamp {
    path: PathBuf,
    length: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

#[derive(Default)]
pub(super) struct IntegrityCache(Option<Vec<Stamp>>);

impl IntegrityCache {
    pub(super) fn verify(
        &mut self,
        paths: &[PathBuf],
        verify: impl FnOnce() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        // Length and last-write time can collide for different files on Windows.
        // Only Unix stamps include identity and change time; otherwise fail closed
        // by checking contents again rather than trusting metadata equality.
        self.verify_with_reuse(paths, verify, cfg!(unix))
    }

    fn verify_with_reuse(
        &mut self,
        paths: &[PathBuf],
        verify: impl FnOnce() -> anyhow::Result<()>,
        can_reuse: bool,
    ) -> anyhow::Result<()> {
        let stamps = paths
            .iter()
            .map(|path| {
                let metadata = std::fs::metadata(path)?;
                #[cfg(unix)]
                use std::os::unix::fs::MetadataExt;
                Ok(Stamp {
                    path: path.clone(),
                    length: metadata.len(),
                    modified: metadata.modified()?,
                    #[cfg(unix)]
                    identity: (
                        metadata.dev(),
                        metadata.ino(),
                        metadata.ctime(),
                        metadata.ctime_nsec(),
                    ),
                })
            })
            .collect::<std::io::Result<Vec<_>>>()?;
        if can_reuse && self.0.as_ref() == Some(&stamps) {
            return Ok(());
        }
        self.0 = None;
        verify()?;
        if can_reuse {
            self.0 = Some(stamps);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn cached_integrity_avoids_repeat_reads_but_rejects_replacement_and_removal() {
        let root = std::env::temp_dir().join(format!("verenu-integrity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("weights");
        std::fs::write(&path, b"verified").unwrap();
        let mut cache = super::IntegrityCache::default();
        let reads = std::cell::Cell::new(0);
        let check = || {
            reads.set(reads.get() + 1);
            anyhow::ensure!(std::fs::read(&path)? == b"verified", "corrupt");
            Ok(())
        };
        for _ in 0..3 {
            cache.verify(std::slice::from_ref(&path), check).unwrap();
        }
        let initial_reads = if cfg!(unix) { 1 } else { 3 };
        assert_eq!(reads.get(), initial_reads);
        let replacement = root.join("replacement");
        std::fs::write(&replacement, b"corrupt!").unwrap();
        let original = std::fs::metadata(&path).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(original.modified().unwrap()))
            .unwrap();
        std::fs::rename(replacement, &path).unwrap();
        let replaced = std::fs::metadata(&path).unwrap();
        assert_eq!(original.len(), replaced.len());
        assert_eq!(original.modified().unwrap(), replaced.modified().unwrap());
        assert!(cache.verify(std::slice::from_ref(&path), check).is_err());
        assert_eq!(reads.get(), initial_reads + 1);
        std::fs::write(&path, b"verified").unwrap();
        cache.verify(std::slice::from_ref(&path), check).unwrap();
        assert_eq!(reads.get(), initial_reads + 2);
        std::fs::remove_file(&path).unwrap();
        assert!(cache.verify(std::slice::from_ref(&path), check).is_err());
        assert_eq!(reads.get(), initial_reads + 2);
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn metadata_only_platforms_reverify_identical_stamps_and_reject_corruption() {
        let root = std::env::temp_dir().join(format!("verenu-integrity-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("weights");
        std::fs::write(&path, b"verified").unwrap();
        let mut cache = super::IntegrityCache::default();
        let reads = std::cell::Cell::new(0);
        let check = || {
            reads.set(reads.get() + 1);
            anyhow::ensure!(std::fs::read(&path)? == b"verified", "corrupt");
            Ok(())
        };
        for _ in 0..3 {
            cache
                .verify_with_reuse(std::slice::from_ref(&path), check, false)
                .unwrap();
        }
        assert_eq!(reads.get(), 3);
        assert!(cache.0.is_none());
        std::fs::write(&path, b"corrupt!").unwrap();
        assert!(cache
            .verify_with_reuse(std::slice::from_ref(&path), check, false)
            .is_err());
        assert_eq!(reads.get(), 4);
        assert!(cache.0.is_none());
        std::fs::remove_file(&path).unwrap();
        assert!(cache
            .verify_with_reuse(std::slice::from_ref(&path), check, false)
            .is_err());
        assert_eq!(reads.get(), 4);
        std::fs::remove_dir(root).unwrap();
    }
}
