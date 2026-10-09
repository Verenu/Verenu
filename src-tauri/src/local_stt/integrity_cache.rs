//! A loaded helper can reuse verification while its installed files are unchanged.
//! Replacement or modification invalidates the cache before native loading.
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
        if self.0.as_ref() == Some(&stamps) {
            return Ok(());
        }
        self.0 = None;
        verify()?;
        self.0 = Some(stamps);
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
        assert_eq!(reads.get(), 1);
        let replacement = root.join("replacement");
        std::fs::write(&replacement, b"corrupt!").unwrap();
        std::fs::rename(replacement, &path).unwrap();
        assert!(cache.verify(std::slice::from_ref(&path), check).is_err());
        assert_eq!(reads.get(), 2);
        std::fs::write(&path, b"verified").unwrap();
        cache.verify(std::slice::from_ref(&path), check).unwrap();
        assert_eq!(reads.get(), 3);
        std::fs::remove_file(&path).unwrap();
        assert!(cache.verify(std::slice::from_ref(&path), check).is_err());
        std::fs::remove_dir(root).unwrap();
    }
}
