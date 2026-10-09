#[cfg(target_os = "macos")]
mod apple;
pub mod download;
pub mod engine;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod fluid;
mod fluid_download;
#[cfg(any(test, all(target_os = "macos", target_arch = "aarch64")))]
mod integrity_cache;
pub mod manager;
pub mod model;
pub mod transcribe;
pub mod vocabulary;

pub fn fluid_supported() -> bool {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        extern "C" {
            fn verenu_speech_os_major() -> libc::c_long;
        }
        unsafe { verenu_speech_os_major() >= 14 }
    }
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    {
        false
    }
}

pub use manager::{LocalTranscriptionManager, LocalTranscriptionState};
pub use model::LocalSttModelInfo;
