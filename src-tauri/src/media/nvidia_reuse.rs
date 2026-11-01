use serde::Serialize;
use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex, OnceLock};

const MIN_SETUP_SPACE: u64 = 20 * 1024 * 1024 * 1024;
const STORAGE_ERROR: &str = "Not enough disk space. NVIDIA RE-USE setup needs at least 20 GiB of free space.";

#[derive(Clone, Serialize)]
pub struct RuntimeStatus {
    pub state: &'static str,
    pub message: String,
    pub has_files: bool,
}

static ACTIVE_WORKERS: AtomicUsize = AtomicUsize::new(0);

fn status_cell() -> &'static Mutex<RuntimeStatus> {
    static STATUS: OnceLock<Mutex<RuntimeStatus>> = OnceLock::new();
    STATUS.get_or_init(|| Mutex::new(RuntimeStatus { state: "unknown", message: String::new(), has_files: false }))
}

fn set_status(state: &'static str, message: impl Into<String>) {
    if let Ok(mut status) = status_cell().lock() {
        *status = RuntimeStatus { state, message: message.into(), has_files: false };
    }
}

fn runtime_dir() -> std::path::PathBuf {
    crate::app_data_dir().join("runtimes").join("nvidia-reuse-v1")
}

fn has_runtime_files(dir: &std::path::Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some())
}

pub fn selected_and_ready() -> bool {
    status_cell().lock().is_ok_and(|status| status.state == "ready")
}

pub fn mark_failed() {
    if let Ok(mut status) = status_cell().lock() {
        if status.state != "removing" {
            status.state = "error";
            status.message = "NVIDIA RE-USE failed. The original recording was used; check the runtime and GPU.".into();
        }
    }
}

#[tauri::command]
pub fn nvidia_reuse_status() -> RuntimeStatus {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return RuntimeStatus { state: "unavailable", message: "Requires Linux x86_64 and a compatible NVIDIA GPU.".into(), has_files: false };
    }
    let mut status = status_cell().lock().map(|s| s.clone()).unwrap_or(RuntimeStatus { state: "error", message: "Runtime status unavailable.".into(), has_files: false });
    status.has_files = has_runtime_files(&runtime_dir());
    status
}

#[tauri::command]
pub async fn nvidia_reuse_check() -> Result<(), String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        set_status("unavailable", "Requires Linux x86_64 and a compatible NVIDIA GPU.");
        return Ok(());
    }
    {
        let mut status = status_cell().lock().map_err(|_| "Runtime status unavailable")?;
        if matches!(status.state, "installing" | "checking" | "removing") { return Ok(()); }
        *status = RuntimeStatus { state: "checking", message: "Checking NVIDIA GPU availability…".into(), has_files: false };
    }
    if !nvidia_driver_available().await {
        set_status("unavailable", "No compatible NVIDIA GPU with a supported driver was detected.");
        return Ok(());
    }
    if !runtime_dir().join("ready").is_file() {
        set_status("not_installed", "Download the experimental runtime to check this computer.");
        return Ok(());
    }
    set_status("checking", "Testing the GPU and model…");
    match probe().await {
        Ok(()) => set_status("ready", "NVIDIA RE-USE is ready."),
        Err(_) => set_status("error", "The GPU or model self-test failed. Check the NVIDIA driver or reinstall the runtime."),
    }
    Ok(())
}

#[tauri::command]
pub async fn nvidia_reuse_install() -> Result<(), String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("Requires Linux x86_64 and a compatible NVIDIA GPU.".into());
    }
    {
        let mut status = status_cell().lock().map_err(|_| "Runtime status unavailable")?;
        if matches!(status.state, "installing" | "checking" | "removing") { return Err("A setup, check, or removal is already running.".into()); }
        if ACTIVE_WORKERS.load(Ordering::Acquire) != 0 { return Err("NVIDIA RE-USE is processing audio. Try again when it finishes.".into()); }
        *status = RuntimeStatus { state: "installing", message: "Preparing runtime…".into(), has_files: false };
    }
    if !nvidia_driver_available().await {
        set_status("unavailable", "No compatible NVIDIA GPU with a supported driver was detected.");
        return Err("No compatible NVIDIA GPU with a supported driver was detected.".into());
    }
    tauri::async_runtime::spawn(async {
        if let Err(error) = install().await { set_status("error", error); }
    });
    Ok(())
}

fn begin_removal(status: &mut RuntimeStatus, active_workers: usize) -> Result<(), String> {
    if matches!(status.state, "installing" | "checking" | "removing") {
        return Err("Wait for the current NVIDIA RE-USE operation to finish.".into());
    }
    if active_workers != 0 {
        return Err("NVIDIA RE-USE is processing audio. Try again when it finishes.".into());
    }
    status.state = "removing";
    status.message = "Deleting runtime files…".into();
    Ok(())
}

#[tauri::command]
pub async fn nvidia_reuse_remove() -> Result<(), String> {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return Err("Requires Linux x86_64 and a compatible NVIDIA GPU.".into());
    }
    {
        let mut status = status_cell().lock().map_err(|_| "Runtime status unavailable")?;
        begin_removal(&mut status, ACTIVE_WORKERS.load(Ordering::Acquire))?;
    }
    let result = tokio::fs::remove_dir_all(runtime_dir()).await;
    match result {
        Ok(()) => {},
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
        Err(error) => {
            let message = format!("Could not delete NVIDIA RE-USE files: {error}");
            set_status("error", &message);
            return Err(message);
        }
    }
    if nvidia_driver_available().await {
        set_status("not_installed", "Turn on NVIDIA RE-USE to download and set up the runtime.");
    } else {
        set_status("unavailable", "No compatible NVIDIA GPU with a supported driver was detected.");
    }
    Ok(())
}

struct RuntimeUse;

impl RuntimeUse {
    fn acquire() -> anyhow::Result<Self> {
        let status = status_cell().lock().map_err(|_| anyhow::anyhow!("Runtime status unavailable"))?;
        anyhow::ensure!(status.state == "ready", "NVIDIA RE-USE is not ready");
        ACTIVE_WORKERS.fetch_add(1, Ordering::AcqRel);
        Ok(Self)
    }
}

impl Drop for RuntimeUse {
    fn drop(&mut self) { ACTIVE_WORKERS.fetch_sub(1, Ordering::AcqRel); }
}

async fn nvidia_driver_available() -> bool {
    use std::process::Stdio;
    match tokio::time::timeout(std::time::Duration::from_secs(5), tokio::process::Command::new("nvidia-smi")
        .args(["--query-gpu=compute_cap,driver_version", "--format=csv,noheader,nounits"])
        .stderr(Stdio::null()).kill_on_drop(true).output()).await {
        Ok(Ok(output)) if output.status.success() => compatible_gpu(&String::from_utf8_lossy(&output.stdout)),
        _ => false,
    }
}

fn compatible_gpu(output: &str) -> bool {
    // The worker selects CUDA device 0. Match its supported PyTorch CUDA architectures.
    let Some(first) = output.lines().next() else { return false };
    let mut fields = first.split(',').map(str::trim);
    let Some((major, minor)) = fields.next().and_then(|s| s.split_once('.')) else { return false };
    let (Ok(major), Ok(minor)) = (major.parse::<u32>(), minor.parse::<u32>()) else { return false };
    let Some(driver) = fields.next().and_then(|s| s.split('.').next()).and_then(|s| s.parse::<u32>().ok()) else { return false };
    driver >= 525 && match major { 7 => minor >= 5, 8..=10 | 12 => true, _ => false }
}

fn ensure_setup_space(available: u64) -> Result<(), String> {
    if available < MIN_SETUP_SPACE { Err(STORAGE_ERROR.into()) } else { Ok(()) }
}

async fn install() -> Result<(), String> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    use std::process::Stdio;
    let dir = runtime_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create runtime directory: {e}"))?;
    let free = fs2::available_space(&dir).map_err(|e| format!("Could not check free disk space: {e}"))?;
    ensure_setup_space(free)?;
    let _ = std::fs::remove_file(dir.join("ready"));
    for (name, content) in [
        ("install.sh", include_str!("../../resources/nvidia-reuse/install.sh")),
        ("worker.py", include_str!("../../resources/nvidia-reuse/worker.py")),
        ("requirements.lock", include_str!("../../resources/nvidia-reuse/requirements.lock")),
    ] { std::fs::write(dir.join(name), content).map_err(|e| e.to_string())?; }
    let mut child = tokio::process::Command::new("sh").arg(dir.join("install.sh")).arg(&dir)
        .stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true).spawn().map_err(|e| format!("Could not start setup: {e}"))?;
    let stdout = child.stdout.take().ok_or("Could not read setup progress")?;
    let result = tokio::time::timeout(std::time::Duration::from_secs(3600), async {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if !line.is_empty() { set_status("installing", line); }
        }
        child.wait().await
    }).await.map_err(|_| "Runtime setup timed out after one hour.".to_string())?.map_err(|e| e.to_string())?;
    if !result.success() {
        if fs2::available_space(&dir).is_ok_and(|free| free < 1024 * 1024 * 1024) {
            return Err(STORAGE_ERROR.into());
        }
        let stage = nvidia_reuse_status().message;
        return Err(format!("Setup failed at: {stage}. Check the network connection, disk space, and NVIDIA driver, then retry."));
    }
    set_status("ready", "NVIDIA RE-USE is ready.");
    Ok(())
}

async fn probe() -> anyhow::Result<()> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(120), tokio::process::Command::new(runtime_dir().join(".venv/bin/python"))
        .arg(runtime_dir().join("worker.py")).arg("--probe").env("HF_HUB_OFFLINE", "1")
        .env("HF_HOME", runtime_dir().join("hf-cache"))
        .env("TORCH_HOME", runtime_dir().join("torch-cache"))
        .env("TRITON_CACHE_DIR", runtime_dir().join("triton-cache"))
        .stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).status()).await??;
    anyhow::ensure!(result.success(), "GPU self-test failed");
    Ok(())
}

pub async fn enhance(wav: bytes::Bytes, expected_samples: usize) -> anyhow::Result<Vec<f32>> {
    let _runtime_use = RuntimeUse::acquire()?;
    let mut command = tokio::process::Command::new(runtime_dir().join(".venv/bin/python"));
    command.arg(runtime_dir().join("worker.py")).env("HF_HUB_OFFLINE", "1")
        .env("HF_HOME", runtime_dir().join("hf-cache"))
        .env("TORCH_HOME", runtime_dir().join("torch-cache"))
        .env("TRITON_CACHE_DIR", runtime_dir().join("triton-cache"));
    let output = run_worker(&mut command, &wav, std::time::Duration::from_secs(120)).await?;
    if !output.status.success() || output.stdout.len() != expected_samples * 4 {
        set_status("error", "NVIDIA RE-USE failed. The original recording was used; check the runtime and GPU.");
        anyhow::bail!("Invalid NVIDIA RE-USE output");
    }
    decode_samples(&output.stdout, expected_samples)
}

async fn run_worker(command: &mut tokio::process::Command, wav: &[u8], limit: std::time::Duration) -> anyhow::Result<std::process::Output> {
    use tokio::io::AsyncWriteExt;
    use std::process::Stdio;
    let mut child = command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null())
        .kill_on_drop(true).spawn()?;
    let mut stdin = child.stdin.take().ok_or_else(|| anyhow::anyhow!("No worker input"))?;
    Ok(tokio::time::timeout(limit, async {
        stdin.write_all(wav).await?;
        drop(stdin);
        child.wait_with_output().await
    }).await??)
}

fn decode_samples(data: &[u8], expected_samples: usize) -> anyhow::Result<Vec<f32>> {
    anyhow::ensure!(data.len() == expected_samples * 4, "Invalid NVIDIA RE-USE output length");
    let samples: Vec<f32> = data.as_chunks::<4>().0.iter().copied().map(f32::from_le_bytes).collect();
    anyhow::ensure!(samples.iter().all(|x| x.is_finite()), "Invalid NVIDIA RE-USE samples");
    Ok(samples)
}

#[cfg(test)]
mod tests {
    use super::decode_samples;

    #[test]
    fn removal_refuses_busy_runtime_and_transitions_to_removing() {
        let mut status = super::RuntimeStatus { state: "ready", message: String::new(), has_files: true };
        assert!(super::begin_removal(&mut status, 1).is_err());
        assert_eq!(status.state, "ready");
        super::begin_removal(&mut status, 0).unwrap();
        assert_eq!(status.state, "removing");
        assert!(super::begin_removal(&mut status, 0).is_err());
        status.state = "installing";
        assert!(super::begin_removal(&mut status, 0).is_err());
    }

    #[test]
    fn empty_runtime_directory_does_not_claim_downloaded_files() {
        let dir = std::env::temp_dir().join(format!("verenu-reuse-files-{}", uuid::Uuid::new_v4()));
        assert!(!super::has_runtime_files(&dir));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!super::has_runtime_files(&dir));
        std::fs::write(dir.join("partial"), b"data").unwrap();
        assert!(super::has_runtime_files(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn gpu_check_matches_the_worker_device_and_supported_architectures() {
        assert!(super::compatible_gpu("12.0, 615.78.08\n"));
        assert!(super::compatible_gpu("8.9, 550.54\n"));
        assert!(!super::compatible_gpu("7.0, 550.54\n12.0, 615.78.08\n"));
        assert!(!super::compatible_gpu("12.0, 520.00\n"));
        assert!(!super::compatible_gpu("not supported, 615.78.08\n"));
        assert!(!super::compatible_gpu(""));
    }

    #[test]
    fn setup_requires_free_space_before_downloading() {
        assert_eq!(super::ensure_setup_space(super::MIN_SETUP_SPACE - 1).unwrap_err(), super::STORAGE_ERROR);
        assert!(super::ensure_setup_space(super::MIN_SETUP_SPACE).is_ok());
    }

    #[test]
    fn rejects_truncated_and_non_finite_worker_output() {
        assert!(decode_samples(&[0, 0, 0], 1).is_err());
        assert!(decode_samples(&f32::NAN.to_le_bytes(), 1).is_err());
        assert_eq!(decode_samples(&0.25f32.to_le_bytes(), 1).unwrap(), vec![0.25]);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn worker_timeout_terminates_the_process() {
        let path = std::env::temp_dir().join(format!("verenu-reuse-pid-{}", uuid::Uuid::new_v4()));
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg("echo $$ > \"$1\"; exec sleep 30").arg("worker").arg(&path);
        assert!(super::run_worker(&mut command, &[], std::time::Duration::from_secs(1)).await.is_err());
        let pid = std::fs::read_to_string(&path).unwrap();
        let proc_path = std::path::Path::new("/proc").join(pid.trim());
        for _ in 0..50 {
            if !proc_path.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let _ = std::fs::remove_file(path);
        assert!(!proc_path.exists(), "timed out worker must exit");
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_worker_terminates_the_process() {
        let path = std::env::temp_dir().join(format!("verenu-reuse-pid-{}", uuid::Uuid::new_v4()));
        let worker_path = path.clone();
        let task = tokio::spawn(async move {
            let mut command = tokio::process::Command::new("sh");
            command.arg("-c").arg("echo $$ > \"$1\"; exec sleep 30").arg("worker").arg(worker_path);
            super::run_worker(&mut command, &[], std::time::Duration::from_secs(30)).await
        });
        for _ in 0..100 {
            if path.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let pid = std::fs::read_to_string(&path).unwrap();
        let proc_path = std::path::Path::new("/proc").join(pid.trim());
        task.abort();
        let _ = task.await;
        for _ in 0..50 {
            if !proc_path.exists() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let _ = std::fs::remove_file(path);
        assert!(!proc_path.exists(), "cancelled worker must exit");
    }
}
