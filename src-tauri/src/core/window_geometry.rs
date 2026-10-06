//! Native target-window geometry used to choose the dictation pill's display.
//!
//! Coordinates intentionally stay in each platform's desktop coordinate space:
//! physical pixels on Windows and Core Graphics logical coordinates on macOS.
//! Those are the coordinate spaces expected by Tauri's `monitor_from_point`.

use crate::core::window_context;
use tauri::{Runtime, WebviewWindow};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DesktopPoint {
    pub x: f64,
    pub y: f64,
}

/// The native focus target used for injection plus the display anchor captured
/// at dictation start. The id is an HWND on Windows and an application PID on
/// macOS.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WindowTarget {
    pub id: usize,
    pub display_point: Option<DesktopPoint>,
    /// Hyprland's stable client address and metadata captured *before* audio
    /// processing. Other platforms leave this empty and keep their HWND/PID
    /// identity in `id`.
    #[cfg(target_os = "linux")]
    pub linux: Option<LinuxWindowTarget>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq)]
pub struct LinuxWindowTarget {
    pub address: String,
    pub pid: u32,
    pub class_name: String,
    pub title: String,
    pub workspace_id: i32,
    pub monitor: i32,
    pub tags: Vec<String>,
}

impl WindowTarget {
    /// Context lookup must use the same window snapshot as insertion. A PID
    /// alone cannot distinguish two browser windows in the same process.
    pub fn process_name(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        if let Some(window) = &self.linux {
            return Some(window.class_name.to_ascii_lowercase()).filter(|name| !name.is_empty());
        }
        window_context::get_process_name_for_hwnd(self.id)
    }

    pub fn window_title(&self) -> Option<String> {
        #[cfg(target_os = "linux")]
        if let Some(window) = &self.linux {
            return Some(window.title.clone()).filter(|title| !title.is_empty());
        }
        window_context::get_window_title(self.id)
    }

    pub fn capture_foreground() -> Self {
        #[cfg(target_os = "linux")]
        if let Some(window) = crate::core::hyprland::active_window() {
            return Self::from_hyprland(window);
        }
        Self::from_id(window_context::get_foreground_hwnd())
    }

    #[cfg(target_os = "linux")]
    fn from_hyprland(window: crate::core::hyprland::ActiveWindow) -> Self {
        Self {
            // Kept for existing target-id contracts; Linux-native operations
            // consume the typed identity below.
            id: window.pid as usize,
            display_point: (window.size[0] > 0 && window.size[1] > 0).then_some(DesktopPoint {
                x: f64::from(window.at[0]) + f64::from(window.size[0]) / 2.0,
                y: f64::from(window.at[1]) + f64::from(window.size[1]) / 2.0,
            }),
            linux: Some(LinuxWindowTarget {
                address: window.address,
                pid: window.pid,
                class_name: window.class_name,
                title: window.title,
                workspace_id: window.workspace.id,
                monitor: window.monitor,
                tags: window.tags,
            }),
        }
    }

    pub fn capture_display_only() -> Self {
        let target = Self::capture_foreground();
        Self::from_parts(0, target.display_point)
    }

    pub fn from_parts(id: usize, display_point: Option<DesktopPoint>) -> Self {
        Self {
            id,
            display_point,
            #[cfg(target_os = "linux")]
            linux: None,
        }
    }

    pub fn from_id(id: usize) -> Self {
        Self {
            id,
            display_point: window_center(id),
            #[cfg(target_os = "linux")]
            linux: None,
        }
    }

    /// Re-read geometry for retries in case the target window moved. Retain the
    /// original point if the native window is no longer queryable.
    pub fn refreshed(self) -> Self {
        #[cfg(target_os = "linux")]
        if let Some(linux) = &self.linux {
            if let Some(window) = crate::core::hyprland::window_by_address(&linux.address) {
                return Self::from_hyprland(window);
            }
        }
        let refreshed = Self::from_id(self.id);
        Self {
            id: self.id,
            display_point: refreshed.display_point.or(self.display_point),
            #[cfg(target_os = "linux")]
            linux: self.linux,
        }
    }
}

/// Returns the center of a Tauri webview window in the platform coordinate
/// space used by `WindowTarget::display_point`.
pub fn capture_webview_center<R: Runtime>(window: &WebviewWindow<R>) -> Option<DesktopPoint> {
    let position = window.outer_position().ok()?;
    let size = window.outer_size().ok()?;
    if size.width == 0 || size.height == 0 {
        return None;
    }

    #[cfg(target_os = "macos")]
    let scale_factor = window
        .scale_factor()
        .ok()
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0);

    #[cfg(not(target_os = "macos"))]
    let scale_factor = 1.0;

    Some(DesktopPoint {
        x: (position.x as f64 + size.width as f64 / 2.0) / scale_factor,
        y: (position.y as f64 + size.height as f64 / 2.0) / scale_factor,
    })
}

#[cfg(windows)]
fn window_center(id: usize) -> Option<DesktopPoint> {
    use windows::Win32::Foundation::{HWND, RECT};
    use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

    if id == 0 {
        return None;
    }

    let mut rect = RECT::default();
    unsafe { GetWindowRect(HWND(id as *mut core::ffi::c_void), &mut rect) }.ok()?;
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return None;
    }

    Some(DesktopPoint {
        x: (f64::from(rect.left) + f64::from(rect.right)) / 2.0,
        y: (f64::from(rect.top) + f64::from(rect.bottom)) / 2.0,
    })
}

#[cfg(target_os = "macos")]
fn window_center(id: usize) -> Option<DesktopPoint> {
    use accessibility_sys::{
        kAXErrorSuccess, kAXFocusedWindowAttribute, kAXPositionAttribute, kAXSizeAttribute,
        kAXValueTypeCGPoint, kAXValueTypeCGSize, AXUIElementCopyAttributeValue,
        AXUIElementCreateApplication, AXUIElementRef, AXUIElementSetMessagingTimeout,
        AXValueGetValue, AXValueRef,
    };
    use core_foundation::base::{CFRelease, CFTypeRef, TCFType};
    use core_foundation::string::CFString;
    use std::ffi::c_void;
    use std::ptr;

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: f64,
        y: f64,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Size {
        width: f64,
        height: f64,
    }

    unsafe fn copy_attribute(element: AXUIElementRef, name: &str) -> Option<CFTypeRef> {
        let attribute = CFString::new(name);
        let mut value: CFTypeRef = ptr::null();
        if AXUIElementCopyAttributeValue(element, attribute.as_concrete_TypeRef(), &mut value)
            != kAXErrorSuccess
            || value.is_null()
        {
            return None;
        }
        Some(value)
    }

    if id == 0 || id > i32::MAX as usize {
        return None;
    }

    unsafe {
        let application = AXUIElementCreateApplication(id as i32);
        if application.is_null() {
            return None;
        }
        let _ = AXUIElementSetMessagingTimeout(application, 0.015);

        let result = (|| {
            let window_value = copy_attribute(application, kAXFocusedWindowAttribute)?;
            let window = window_value as AXUIElementRef;
            let _ = AXUIElementSetMessagingTimeout(window, 0.015);

            let position_value = copy_attribute(window, kAXPositionAttribute);
            let size_value = copy_attribute(window, kAXSizeAttribute);

            let center = match (position_value, size_value) {
                (Some(position_value), Some(size_value)) => {
                    let mut position = Point::default();
                    let mut size = Size::default();
                    let position_ok = AXValueGetValue(
                        position_value as AXValueRef,
                        kAXValueTypeCGPoint,
                        &mut position as *mut Point as *mut c_void,
                    );
                    let size_ok = AXValueGetValue(
                        size_value as AXValueRef,
                        kAXValueTypeCGSize,
                        &mut size as *mut Size as *mut c_void,
                    );
                    if position_ok && size_ok && size.width > 0.0 && size.height > 0.0 {
                        Some(DesktopPoint {
                            x: position.x + size.width / 2.0,
                            y: position.y + size.height / 2.0,
                        })
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(value) = position_value {
                CFRelease(value);
            }
            if let Some(value) = size_value {
                CFRelease(value);
            }
            CFRelease(window_value);
            center
        })();

        CFRelease(application as CFTypeRef);
        result
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn window_center(_id: usize) -> Option<DesktopPoint> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod context_snapshot_tests {
    use super::*;

    #[test]
    fn hyprland_snapshot_preserves_metadata_and_rejects_empty_geometry() {
        for size in [[400, 200], [0, 200], [400, -1]] {
            let window = crate::core::hyprland::ActiveWindow {
                address: "0x123".into(),
                pid: 123,
                class_name: "foot".into(),
                title: "Public fixture".into(),
                at: [-100, 20],
                size,
                workspace: crate::core::hyprland::Workspace { id: 2, name: "2".into() },
                monitor: 3,
                tags: vec!["terminal".into()],
            };
            let target = WindowTarget::from_hyprland(window);
            assert_eq!(target.id, 123);
            assert_eq!(
                target.display_point,
                (size == [400, 200]).then_some(DesktopPoint { x: 100.0, y: 120.0 })
            );
            assert_eq!(target.linux.unwrap(), LinuxWindowTarget {
                address: "0x123".into(), pid: 123, class_name: "foot".into(),
                title: "Public fixture".into(), workspace_id: 2, monitor: 3,
                tags: vec!["terminal".into()],
            });
        }
    }

    #[test]
    fn context_metadata_remains_bound_to_captured_window_without_live_pid() {
        let target = WindowTarget {
            id: 0,
            display_point: None,
            linux: Some(LinuxWindowTarget {
                address: "0x1".into(), pid: 0, class_name: "com.t3tools.T3Code".into(),
                title: "Public project - T3 Code".into(), workspace_id: 1, monitor: 0, tags: vec![],
            }),
        };
        assert_eq!(target.process_name().as_deref(), Some("com.t3tools.t3code"));
        assert_eq!(target.window_title().as_deref(), Some("Public project - T3 Code"));
        assert!(WindowTarget::default().process_name().is_none());
    }
}
