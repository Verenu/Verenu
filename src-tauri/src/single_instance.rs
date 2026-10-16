use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
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
    mutex: OwnedHandle,
    _event: OwnedHandle,
}

impl Drop for Guard {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Threading::ReleaseMutex(HANDLE(
                self.mutex.as_raw_handle(),
            ));
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
        let event = unsafe { OwnedHandle::from_raw_handle(event.0) };
        let mutex = unsafe {
            CreateMutexW(None, true, PCWSTR(mutex_name.as_ptr()))
                .expect("failed to create Verenu single-instance mutex")
        };
        let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = unsafe { OwnedHandle::from_raw_handle(mutex.0) };

        if !already_exists {
            return Guard {
                mutex,
                _event: event,
            };
        }

        unsafe {
            let _ = SetEvent(HANDLE(event.as_raw_handle()));
        }
        let result = unsafe { WaitForSingleObject(HANDLE(mutex.as_raw_handle()), 15_000) };
        if result == WAIT_OBJECT_0 || result == WAIT_ABANDONED_0 {
            return Guard {
                mutex,
                _event: event,
            };
        }

        drop(mutex);
        drop(event);
        if result != WAIT_TIMEOUT {
            panic!("failed waiting for the previous Verenu instance to exit");
        }
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
