//! On-device FoundationModels cleanup. This module never makes network calls.
use serde::Serialize;

pub const MODEL: &str = "system";
pub const TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Debug, Serialize)]
pub struct Availability {
    pub state: &'static str,
    pub available: bool,
    pub message: &'static str,
}

fn status(code: i32) -> Availability {
    let (state, message) = match code {
        0 => ("available", "On-device cleanup. No API key or cloud connection needed."),
        1 => ("unsupported-os", "Apple Intelligence cleanup requires macOS 26 or newer."),
        2 => ("device-not-eligible", "This Mac does not support Apple Intelligence."),
        3 => ("intelligence-disabled", "Enable Apple Intelligence in macOS System Settings."),
        4 => ("model-not-ready", "Apple Intelligence is preparing its model. Try again after the system download finishes."),
        6 => ("unsupported-platform", "Apple Intelligence cleanup is available only on supported Macs."),
        _ => ("unavailable", "Apple Intelligence is unavailable. Check macOS System Settings and try again."),
    };
    Availability {
        state,
        available: code == 0,
        message,
    }
}

pub fn availability() -> Availability {
    #[cfg(target_os = "macos")]
    {
        status(unsafe { native::verenu_fm_availability() })
    }
    #[cfg(not(target_os = "macos"))]
    {
        status(6)
    }
}

#[tauri::command]
pub fn get_apple_intelligence_availability() -> Availability {
    availability()
}

#[cfg(any(target_os = "macos", test))]
fn accepted_response(value: &str) -> anyhow::Result<String> {
    if let Some(text) = value.strip_prefix("O:") {
        let text = text.trim();
        anyhow::ensure!(
            !text.is_empty(),
            "Apple Intelligence returned empty cleanup"
        );
        anyhow::ensure!(
            text.len() <= 256 * 1024,
            "Apple Intelligence response exceeded the size limit"
        );
        return Ok(text.to_owned());
    }
    let message = match value {
        "E:context-overflow" => {
            "Apple Intelligence context limit exceeded; the dictation was not truncated"
        }
        "E:refusal" => "Apple Intelligence declined this cleanup",
        "E:unsupported-language" => "Apple Intelligence does not support this language",
        "E:model-not-ready" | "E:unavailable" => "Apple Intelligence is no longer ready",
        "E:cancelled" => "Apple Intelligence cleanup cancelled",
        _ => "Apple Intelligence cleanup failed",
    };
    anyhow::bail!("{message}")
}

pub async fn cleanup(
    model: &str,
    prompt: &str,
    input: &str,
    max_tokens: u32,
) -> anyhow::Result<String> {
    anyhow::ensure!(model == MODEL, "Unsupported Apple Intelligence model");
    // This is a memory bound, not an estimated token window. FoundationModels
    // enforces its actual context limit and returns a recoverable overflow.
    anyhow::ensure!(
        prompt.len().saturating_add(input.len()) <= 64 * 1024,
        "Apple Intelligence input exceeds the size limit; the dictation was not truncated"
    );
    let readiness = availability();
    anyhow::ensure!(readiness.available, "{}", readiness.message);
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{CStr, CString};
        struct Request(u64);
        impl Drop for Request {
            fn drop(&mut self) {
                unsafe { native::verenu_fm_cancel(self.0) }
            }
        }
        let prompt =
            CString::new(prompt).map_err(|_| anyhow::anyhow!("Invalid cleanup instructions"))?;
        let input = CString::new(input).map_err(|_| anyhow::anyhow!("Invalid cleanup input"))?;
        let request = Request(unsafe {
            native::verenu_fm_start(prompt.as_ptr(), input.as_ptr(), max_tokens.min(2048) as i32)
        });
        let operation = async {
            loop {
                let result = unsafe { native::verenu_fm_poll(request.0) };
                if !result.is_null() {
                    let value = unsafe { CStr::from_ptr(result) }
                        .to_string_lossy()
                        .into_owned();
                    unsafe { native::verenu_fm_free(result) };
                    return accepted_response(&value);
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        };
        tokio::time::timeout(std::time::Duration::from_secs(TIMEOUT_SECS), operation)
            .await
            .map_err(|_| anyhow::anyhow!("Apple Intelligence cleanup timed out"))?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = max_tokens;
        anyhow::bail!("Apple Intelligence requires macOS")
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::ffi::c_char;
    unsafe extern "C" {
        pub fn verenu_fm_availability() -> i32;
        pub fn verenu_fm_start(prompt: *const c_char, input: *const c_char, tokens: i32) -> u64;
        pub fn verenu_fm_poll(id: u64) -> *mut c_char;
        pub fn verenu_fm_cancel(id: u64);
        pub fn verenu_fm_free(value: *mut c_char);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn availability_reasons_are_distinct_and_keyless() {
        for (code, state) in [
            (0, "available"),
            (1, "unsupported-os"),
            (2, "device-not-eligible"),
            (3, "intelligence-disabled"),
            (4, "model-not-ready"),
            (5, "unavailable"),
            (6, "unsupported-platform"),
        ] {
            assert_eq!(status(code).state, state);
            assert_eq!(status(code).available, code == 0);
        }
    }
    #[test]
    fn responses_fail_closed_without_framework_diagnostics() {
        assert_eq!(accepted_response("O: Hello.").unwrap(), "Hello.");
        for value in [
            "O: ",
            "E:refusal",
            "E:context-overflow",
            "E:cancelled",
            "E:unsupported-language",
            "E:unavailable",
            "private diagnostics",
        ] {
            assert!(accepted_response(value).is_err());
        }
    }
}
