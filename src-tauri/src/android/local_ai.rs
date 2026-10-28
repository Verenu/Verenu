use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const LLAMA_BINARY: &str = "libverenu_llama_server.so";
const ONNX_LIBRARY: &str = "libonnxruntime.so";
static NATIVE_LIBRARY_DIR: OnceLock<PathBuf> = OnceLock::new();
static SDK_VERSION: OnceLock<i32> = OnceLock::new();

#[cfg(target_os = "android")]
pub fn initialize() -> anyhow::Result<()> {
    use jni::objects::{JObject, JString};
    let context = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast())? };
    let mut env = vm.attach_current_thread()?;
    let application = unsafe { JObject::from_raw(context.context().cast()) };
    let info = env
        .call_method(
            &application,
            "getApplicationInfo",
            "()Landroid/content/pm/ApplicationInfo;",
            &[],
        )?
        .l()?;
    let dir = JString::from(
        env.get_field(&info, "nativeLibraryDir", "Ljava/lang/String;")?
            .l()?,
    );
    let path = PathBuf::from(String::from(env.get_string(&dir)?));
    let sdk = env
        .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
        .i()?;
    let _ = SDK_VERSION.set(sdk);
    NATIVE_LIBRARY_DIR
        .set(path)
        .map_err(|_| anyhow::anyhow!("Android native runtime paths already initialized"))?;
    Ok(())
}

pub fn native_library_dir() -> anyhow::Result<&'static Path> {
    NATIVE_LIBRARY_DIR
        .get()
        .map(PathBuf::as_path)
        .ok_or_else(|| anyhow::anyhow!("Android native runtime paths are not initialized"))
}

pub fn llama_binary() -> anyhow::Result<PathBuf> {
    let path = native_library_dir()?.join(LLAMA_BINARY);
    if !path.is_file() {
        anyhow::bail!("Local cleanup runtime is missing from this Android build. Reinstall a build with local AI support.")
    }
    Ok(path)
}

pub fn runtimes_present(dir: &Path) -> bool {
    dir.join(ONNX_LIBRARY).is_file() && dir.join(LLAMA_BINARY).is_file()
}

fn onnx_library_path(dir: &Path) -> anyhow::Result<PathBuf> {
    let path = dir.join(ONNX_LIBRARY);
    if !path.is_file() {
        anyhow::bail!("ONNX runtime is missing from this Android build")
    }
    Ok(path)
}

pub fn supported() -> bool {
    SDK_VERSION.get().is_some_and(|sdk| *sdk >= 28)
        && native_library_dir().is_ok_and(runtimes_present)
}

#[cfg(target_os = "android")]
pub fn ensure_onnx_runtime() -> anyhow::Result<()> {
    static READY: OnceLock<()> = OnceLock::new();
    static LOAD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = LOAD
        .lock()
        .map_err(|_| anyhow::anyhow!("Android ONNX initialization lock poisoned"))?;
    if READY.get().is_some() {
        return Ok(());
    }
    // ort's dynamic loader recursively initializes itself when reporting a
    // missing library, so reject the common optional-runtime case first.
    let path = onnx_library_path(native_library_dir()?)?;
    ort::init_from(&path)?.with_name("Verenu").commit();
    let _ = READY.set(());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capability_requires_both_apk_runtimes() {
        let dir =
            std::env::temp_dir().join(format!("verenu-android-runtimes-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!runtimes_present(&dir));
        std::fs::write(dir.join("libonnxruntime.so"), b"fixture").unwrap();
        assert!(!runtimes_present(&dir));
        std::fs::write(dir.join(LLAMA_BINARY), b"fixture").unwrap();
        assert!(runtimes_present(&dir));
        std::fs::remove_file(dir.join("libonnxruntime.so")).unwrap();
        assert!(!runtimes_present(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_onnx_runtime_is_reported_without_loading_it() {
        let dir =
            std::env::temp_dir().join(format!("verenu-missing-onnx-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(onnx_library_path(&dir)
            .unwrap_err()
            .to_string()
            .contains("missing"));
        std::fs::write(dir.join(ONNX_LIBRARY), b"fixture").unwrap();
        assert_eq!(onnx_library_path(&dir).unwrap(), dir.join(ONNX_LIBRARY));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
