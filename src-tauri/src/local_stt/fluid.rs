//! CoreML lives in a private, separately deployed macOS 14 helper. The host
//! binary has no loader dependency on FluidAudio and still supports macOS 11.
use super::vocabulary::Vocabulary;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::{BufRead, BufReader, Write},
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
};

pub struct FluidEngine {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    root: PathBuf,
}
#[derive(Deserialize)]
struct Response {
    text: Option<String>,
    error: Option<String>,
}

impl FluidEngine {
    pub fn load(path: &Path, model: &str) -> anyhow::Result<Self> {
        anyhow::ensure!(
            super::fluid_supported(),
            "FluidAudio requires macOS 14 or later on Apple Silicon"
        );
        super::fluid_download::verify_integrity(model, path)?;
        let root = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("missing model root"))?
            .to_path_buf();
        let binary = include_bytes!(env!("VERENU_FLUID_HELPER"));
        let hash = format!("{:x}", Sha256::digest(binary));
        let runtime = root.join("fluid-runtime").join(&hash);
        std::fs::create_dir_all(&runtime)?;
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))?;
        let resources = include_bytes!(env!("VERENU_FLUID_RESOURCES"));
        // SwiftPM resolves Bundle.module next to the helper executable. Keep
        // the exact package resource names alongside every embedded build.
        tar::Archive::new(&resources[..]).unpack(&runtime)?;
        let helper = runtime.join(format!("VerenuFluidSpeech-{hash}"));
        if !helper.exists() || std::fs::read(&helper)?.as_slice() != binary {
            let staging = runtime.join(format!("{hash}.partial"));
            std::fs::write(&staging, binary)?;
            std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700))?;
            std::fs::rename(staging, &helper)?;
        }
        let mut child = Command::new(helper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().unwrap();
        let output = BufReader::new(child.stdout.take().unwrap());
        let mut engine = Self {
            child,
            input,
            output,
            root,
        };
        engine.request(
            &serde_json::json!({"operation":"load", "path":path, "model":model}),
            &AtomicBool::new(false),
        )?;
        Ok(engine)
    }

    fn request(
        &mut self,
        request: &serde_json::Value,
        cancellation: &AtomicBool,
    ) -> anyhow::Result<Response> {
        serde_json::to_writer(&mut self.input, request)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        let started = std::time::Instant::now();
        loop {
            if cancellation.load(Ordering::Acquire) || started.elapsed().as_secs() >= 120 {
                self.child.kill().ok();
                self.child.wait().ok();
                anyhow::bail!("FluidAudio operation cancelled or timed out");
            }
            let mut poll = libc::pollfd {
                fd: self.output.get_ref().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut poll, 1, 50) };
            if ready < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if ready == 0 {
                continue;
            }
            let mut line = String::new();
            anyhow::ensure!(
                self.output.read_line(&mut line)? > 0,
                "FluidAudio helper stopped"
            );
            let response: Response = serde_json::from_str(&line)
                .map_err(|_| anyhow::anyhow!("invalid FluidAudio response"))?;
            anyhow::ensure!(
                response.error.is_none(),
                "FluidAudio could not load or transcribe the installed model"
            );
            return Ok(response);
        }
    }

    pub fn transcribe(
        &mut self,
        samples: &[f32],
        language: &str,
        vocabulary: &Vocabulary,
        cancellation: &AtomicBool,
    ) -> anyhow::Result<String> {
        let booster = super::model::manifest_by_id("fluid-english-booster").filter(|m| {
            m.is_downloaded(&self.root)
                && super::fluid_download::verify_integrity(m.id, &m.final_path(&self.root)).is_ok()
        });
        let response = self.request(&serde_json::json!({
            "operation":"transcribe", "samples":samples, "language":language,
            "vocabulary":vocabulary.terms(), "boosterPath":booster.map(|m| m.final_path(&self.root))
        }), cancellation)?;
        Ok(response.text.unwrap_or_default())
    }
}
impl Drop for FluidEngine {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
