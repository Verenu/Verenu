//! CoreML lives in a private, separately deployed macOS 14 helper. The host
//! binary has no loader dependency on FluidAudio and still supports macOS 11.
use super::vocabulary::Vocabulary;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::atomic::AtomicBool,
};

pub struct FluidEngine {
    child: Child,
    input: ChildStdin,
    output: ChildStdout,
    root: PathBuf,
    booster_integrity: super::integrity_cache::IntegrityCache,
}
#[derive(Deserialize)]
struct Response {
    text: Option<String>,
    error: Option<String>,
}

impl FluidEngine {
    pub fn load(path: &Path, model: &str) -> anyhow::Result<Self> {
        Self::load_with_cancellation(path, model, &AtomicBool::new(false))
    }

    pub fn load_with_cancellation(
        path: &Path,
        model: &str,
        cancellation: &AtomicBool,
    ) -> anyhow::Result<Self> {
        crate::api::model_download::ensure_not_cancelled(cancellation)?;
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
        let output = child.stdout.take().unwrap();
        let mut engine = Self {
            child,
            input,
            output,
            root,
            booster_integrity: Default::default(),
        };
        engine.request(
            &serde_json::json!({"operation":"load", "path":path, "model":model}),
            cancellation,
        )?;
        Ok(engine)
    }

    fn request(
        &mut self,
        request: &serde_json::Value,
        cancellation: &AtomicBool,
    ) -> anyhow::Result<Response> {
        let started = std::time::Instant::now();
        let mut payload = serde_json::to_vec(request)?;
        payload.push(b'\n');
        self.request_payload(&payload, cancellation, started)
    }

    fn request_payload(
        &mut self,
        payload: &[u8],
        cancellation: &AtomicBool,
        started: std::time::Instant,
    ) -> anyhow::Result<Response> {
        let line = match super::fluid_io::exchange(
            &mut self.input,
            &mut self.output,
            payload,
            cancellation,
            started,
            std::time::Duration::from_secs(120),
        ) {
            Ok(line) => line,
            Err(error) => {
                self.child.kill().ok();
                self.child.wait().ok();
                return Err(error);
            }
        };
        let response: Response = serde_json::from_slice(&line)
            .map_err(|_| anyhow::anyhow!("invalid FluidAudio response"))?;
        anyhow::ensure!(
            response.error.is_none(),
            "FluidAudio could not load or transcribe the installed model"
        );
        Ok(response)
    }

    pub fn transcribe(
        &mut self,
        samples: &[f32],
        language: &str,
        vocabulary: &Vocabulary,
        cancellation: &AtomicBool,
    ) -> anyhow::Result<String> {
        let booster = super::model::manifest_by_id("fluid-english-booster").filter(|m| {
            language == "en"
                && !vocabulary.terms().is_empty()
                && m.is_downloaded(&self.root)
                && super::fluid_download::verify_cached_integrity(
                    m.id,
                    &m.final_path(&self.root),
                    &mut self.booster_integrity,
                )
                .is_ok()
        });
        let started = std::time::Instant::now();
        let booster_path = booster.map(|m| m.final_path(&self.root));
        let payload = super::fluid_protocol::transcription_payload(
            samples, language, vocabulary.terms(), booster_path.as_deref(), cancellation,
        )?;
        let response = self.request_payload(&payload, cancellation, started)?;
        Ok(response.text.unwrap_or_default())
    }
}
impl Drop for FluidEngine {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}
