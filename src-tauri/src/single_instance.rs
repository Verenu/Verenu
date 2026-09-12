//! Cross-build single-instance handoff for Windows.
//!
//! Tauri's single-instance plugin intentionally keeps the first process and
//! exits the second one. That is the wrong direction for an updater relaunch,
//! and its identifier-based namespace also lets dev and packaged builds run
//! side by side. This small mutex/event pair gives the newest process the
//! ownership instead.

use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS};
use windows::Win32::Foundation::{HANDLE, WAIT_ABANDONED_0, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject,
};

const MUTEX_NAME: &str = "Local\\Verenu.SingleInstance.v1";
const EVENT_NAME: &str = "Local\\Verenu.SingleInstance.Takeover.v1";

pub(crate) struct Guard {
    mutex: HANDLE,
    event: HANDLE,
}

// Win32 HANDLE values are process-owned opaque references. The guard is only
// moved into Tauri's synchronized state and each handle is closed exactly once
// by Drop; no handle is accessed concurrently through the Rust fields.
unsafe impl Send for Guard {}
unsafe impl Sync for Guard {}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
            let _ = CloseHandle(self.event);
        }
    }
}

pub(crate) fn acquire() -> Guard {
    let mutex_name = wide(MUTEX_NAME);
    let event_name = wide(EVENT_NAME);

    loop {
        let event = unsafe {
            CreateEventW(None, false, false, PCWSTR(event_name.as_ptr()))
                .expect("failed to create Verenu takeover event")
        };
        let mutex = unsafe {
            CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()))
                .expect("failed to create Verenu single-instance mutex")
        };

        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return Guard { mutex, event };
        }

        // Ask the current owner to perform its normal Tauri shutdown. The
        // mutex handle is also a wait handle, so this process can acquire it
        // as soon as the old process has completely torn down.
        unsafe {
            let _ = SetEvent(event);
        }
        let result = unsafe { WaitForSingleObject(mutex, 15_000) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED_0 {
            return Guard { mutex, event };
        }

        unsafe {
            let _ = CloseHandle(mutex);
            let _ = CloseHandle(event);
        }
        if result != WAIT_TIMEOUT {
            panic!("failed waiting for the previous Verenu instance to exit");
        }
        // Retry after a bounded wait. This also handles a brief startup race
        // where the previous owner has not installed its event listener yet.
        std::thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn listen_for_takeover(app: &tauri::AppHandle) {
    let event_name = wide(EVENT_NAME);
    let app = app.clone();
    std::thread::spawn(move || {
        let event = unsafe {
            match CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) {
                Ok(event) => event,
                Err(_) => return,
            }
        };
        let result = unsafe { WaitForSingleObject(event, u32::MAX) };
        unsafe {
            let _ = CloseHandle(event);
        }
        if result == WAIT_OBJECT_0 {
            app.exit(0);
        }
    });
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
