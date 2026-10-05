//! One clipboard transaction owns every chunk. Platform callers restore their
//! snapshot after this returns, including errors; this future is never aborted.

use std::{fmt, future::Future, time::Duration};
use unicode_segmentation::UnicodeSegmentation;

const MAX_CHARS: usize = 150;
const WRITE_SETTLE: Duration = Duration::from_millis(80);
const PASTE_SETTLE: Duration = Duration::from_millis(400);

pub(super) fn split(text: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        let mut end = 0;
        let mut chars = 0;
        let mut breaks = 0;
        let mut word_end = 0;
        for (start, grapheme) in remaining.grapheme_indices(true) {
            let size = grapheme.chars().count();
            let line_break = grapheme.contains(['\r', '\n', '\u{2028}', '\u{2029}']);
            if end > 0 && (chars + size > MAX_CHARS || (line_break && breaks == 1)) {
                break;
            }
            chars += size;
            breaks += usize::from(line_break);
            end = start + grapheme.len();
            if grapheme.chars().all(char::is_whitespace) {
                word_end = end;
            }
        }
        // A single unusually large grapheme stays intact. Never drop or add
        // whitespace, including CRLF, when preferring a word boundary.
        if end < remaining.len() && word_end > 0 {
            end = word_end;
        }
        chunks.push(&remaining[..end]);
        remaining = &remaining[end..];
    }
    chunks
}

pub(super) trait PasteBackend {
    fn check(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn write(&mut self, text: &str) -> impl Future<Output = anyhow::Result<()>> + Send;
    fn paste(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send;
}

#[derive(Debug)]
pub(crate) struct ChunkedPasteError {
    pub attempted_chunks: usize,
    cause: anyhow::Error,
}
impl fmt::Display for ChunkedPasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Chunked paste stopped after {} attempted chunks: {}",
            self.attempted_chunks, self.cause
        )
    }
}
impl std::error::Error for ChunkedPasteError {}

pub(super) async fn paste(backend: &mut impl PasteBackend, text: &str) -> anyhow::Result<()> {
    let mut attempted_chunks = 0;
    let result = async {
        anyhow::ensure!(
            !text.contains('\0'),
            "Cannot paste text containing a NUL character"
        );
        for chunk in split(text) {
            backend.check().await?;
            backend.write(chunk).await?;
            tokio::time::sleep(WRITE_SETTLE).await;
            backend.check().await?;
            // Input dispatch may partly succeed before reporting failure.
            attempted_chunks += 1;
            let pasted = backend.paste().await;
            tokio::time::sleep(PASTE_SETTLE).await;
            pasted?;
        }
        Ok(())
    }
    .await;
    result.map_err(|cause| {
        ChunkedPasteError {
            attempted_chunks,
            cause,
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_chunks_preserve_unicode_whitespace_and_line_breaks() {
        for text in [
            String::new(),
            " a  b\r\n\r\nc\n\n尾 \t".into(),
            "word ".repeat(200),
            "界".repeat(410),
            "👩🏽‍💻e\u{301}🇨🇦 ".repeat(70),
            "x".repeat(401),
        ] {
            let chunks = split(&text);
            assert_eq!(chunks.concat(), text);
            let boundaries: Vec<_> = text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .chain([text.len()])
                .collect();
            let mut offset = 0;
            for chunk in chunks {
                assert!(!chunk.is_empty());
                assert!(chunk.chars().count() <= MAX_CHARS);
                assert!(
                    chunk
                        .graphemes(true)
                        .filter(|g| g.contains(['\r', '\n']))
                        .count()
                        <= 1
                );
                offset += chunk.len();
                assert!(boundaries.contains(&offset));
            }
        }
    }

    struct Fixture {
        writes: Vec<String>,
        attempts: usize,
        fail_at: Option<usize>,
        checks: usize,
        lose_focus_at: Option<usize>,
        last_paste: Option<std::time::Instant>,
    }
    impl PasteBackend for Fixture {
        async fn check(&mut self) -> anyhow::Result<()> {
            self.checks += 1;
            anyhow::ensure!(self.lose_focus_at != Some(self.checks), "Focus changed");
            Ok(())
        }
        async fn write(&mut self, text: &str) -> anyhow::Result<()> {
            if let Some(last) = self.last_paste {
                assert!(last.elapsed() >= PASTE_SETTLE);
            }
            self.writes.push(text.into());
            Ok(())
        }
        async fn paste(&mut self) -> anyhow::Result<()> {
            self.attempts += 1;
            self.last_paste = Some(std::time::Instant::now());
            anyhow::ensure!(self.fail_at != Some(self.attempts), "Dispatch failed");
            Ok(())
        }
    }
    fn fixture() -> Fixture {
        Fixture {
            writes: vec![],
            attempts: 0,
            fail_at: None,
            checks: 0,
            lose_focus_at: None,
            last_paste: None,
        }
    }
    #[tokio::test]
    async fn paste_chunks_wait_and_stop_without_replaying_partial_dispatch() {
        let mut backend = fixture();
        backend.fail_at = Some(2);
        let error = paste(&mut backend, &"x".repeat(450)).await.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<ChunkedPasteError>()
                .unwrap()
                .attempted_chunks,
            2
        );
        assert_eq!(backend.writes.len(), 2);
        assert!(backend.last_paste.unwrap().elapsed() >= PASTE_SETTLE);
    }
    #[tokio::test]
    async fn paste_chunks_recheck_after_write_and_before_next_chunk() {
        for (check, attempted) in [(2, 0), (3, 1)] {
            let mut backend = fixture();
            backend.lose_focus_at = Some(check);
            let error = paste(&mut backend, &"x".repeat(450)).await.unwrap_err();
            assert_eq!(
                error
                    .downcast_ref::<ChunkedPasteError>()
                    .unwrap()
                    .attempted_chunks,
                attempted
            );
            assert_eq!(backend.writes.len(), 1);
        }
    }
    #[tokio::test]
    async fn paste_chunks_deliver_exact_text_and_settle_last_chunk() {
        let mut backend = fixture();
        let text = "one\ntwo\nthree\nfour";
        paste(&mut backend, text).await.unwrap();
        assert_eq!(backend.writes.concat(), text);
        assert!(backend.last_paste.unwrap().elapsed() >= PASTE_SETTLE);
    }
}
