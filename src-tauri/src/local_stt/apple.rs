use std::ffi::{CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};

extern "C" {
    fn verenu_speech_transcribe(
        samples: *const f32,
        count: usize,
        language: *const libc::c_char,
        vocabulary: *const libc::c_char,
        cancelled: extern "C" fn(*const libc::c_void) -> bool,
        cancel_state: *const libc::c_void,
        error: *mut libc::c_int,
    ) -> *mut libc::c_char;
}

extern "C" fn cancelled(state: *const libc::c_void) -> bool {
    // The worker retains the Arc for the entire native operation.
    unsafe { (&*state.cast::<AtomicBool>()).load(Ordering::Acquire) }
}

pub fn transcribe(
    samples: &[f32],
    language: &str,
    vocabulary: &super::vocabulary::Vocabulary,
    cancellation: &AtomicBool,
) -> anyhow::Result<String> {
    let language = CString::new(language)?;
    let terms = CString::new(serde_json::to_string(vocabulary.terms())?)?;
    let mut error = 0;
    let output = unsafe {
        verenu_speech_transcribe(
            samples.as_ptr(),
            samples.len(),
            language.as_ptr(),
            terms.as_ptr(),
            cancelled,
            (cancellation as *const AtomicBool).cast(),
            &mut error,
        )
    };
    if !output.is_null() {
        let text = unsafe { CStr::from_ptr(output) }
            .to_string_lossy()
            .into_owned();
        unsafe {
            libc::free(output.cast());
        }
        return Ok(text);
    }
    anyhow::bail!(match error {
        1 => "Apple Speech requires nonempty 16 kHz audio",
        2 => "Apple Speech cancelled",
        3 => "Apple Speech timed out",
        4 => "Allow Verenu Speech Recognition in System Settings > Privacy & Security",
        5 => "Apple on-device speech is unavailable for this language. Install its system language assets or choose another engine",
        _ => "Apple on-device recognition failed. Check system language assets or choose another engine",
    })
}
