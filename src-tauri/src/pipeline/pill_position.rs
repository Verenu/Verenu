use crate::core::window_geometry::DesktopPoint;
use tauri::{Runtime, WebviewWindow};

const PILL_BOTTOM_GAP_POINTS: f64 = 16.0;

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(target_os = "linux", allow(dead_code))]
struct MonitorSnapshot {
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PillPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<&tauri::Monitor> for MonitorSnapshot {
    fn from(monitor: &tauri::Monitor) -> Self {
        let work_area = monitor.work_area();
        Self {
            work_x: work_area.position.x,
            work_y: work_area.position.y,
            work_width: work_area.size.width,
            work_height: work_area.size.height,
            scale_factor: monitor.scale_factor(),
        }
    }
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn round_to_physical(points: f64, scale_factor: f64) -> i32 {
    (points * scale_factor).round() as i32
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn placement_for_monitor(
    monitor: MonitorSnapshot,
    width_points: f64,
    height_points: f64,
) -> PillPlacement {
    let width = round_to_physical(width_points, monitor.scale_factor);
    let height = round_to_physical(height_points, monitor.scale_factor);
    let gap = round_to_physical(PILL_BOTTOM_GAP_POINTS, monitor.scale_factor);
    let work_width = monitor.work_width as f64;
    let target_x = monitor.work_x + ((work_width - width as f64) / 2.0).round() as i32;
    let target_y = monitor.work_y + monitor.work_height as i32 - height - gap;

    PillPlacement {
        x: target_x,
        y: target_y,
        width,
        height,
    }
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
fn choose_monitor(
    target_monitor: Option<MonitorSnapshot>,
    primary_monitor: Option<MonitorSnapshot>,
) -> Option<MonitorSnapshot> {
    target_monitor.or(primary_monitor)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn resolve_pill_placement<R: Runtime>(
    pill: &WebviewWindow<R>,
    target_point: Option<DesktopPoint>,
    width_points: f64,
    height_points: f64,
) -> Option<PillPlacement> {
    let target_monitor = target_point.and_then(|point| {
        pill.monitor_from_point(point.x, point.y)
            .ok()
            .flatten()
            .map(|monitor| MonitorSnapshot::from(&monitor))
    });
    let primary_monitor = pill
        .primary_monitor()
        .ok()
        .flatten()
        .map(|monitor| MonitorSnapshot::from(&monitor));
    let monitor = choose_monitor(target_monitor, primary_monitor)?;
    let placement = placement_for_monitor(monitor, width_points, height_points);

    if crate::system::logger::is_verbose() {
        log::debug!(
            "pill resolve_pill_placement: target_point={target_point:?} used_target_monitor={} monitor={monitor:?} -> placement={placement:?}",
            target_monitor.is_some()
        );
    }

    Some(placement)
}

#[cfg(target_os = "linux")]
pub(super) fn resolve_pill_placement<R: Runtime>(
    _pill: &WebviewWindow<R>,
    target_point: Option<DesktopPoint>,
    width_points: f64,
    height_points: f64,
) -> Option<PillPlacement> {
    let point = target_point.unwrap_or(DesktopPoint { x: f64::NAN, y: f64::NAN });
    let monitor = crate::core::hyprland::logical_monitor_for_point(point.x, point.y)?;
    Some(placement_for_linux_monitor(
        monitor,
        width_points,
        height_points,
    ))
}

#[cfg(target_os = "linux")]
fn placement_for_linux_monitor(
    monitor: crate::core::hyprland::LogicalMonitor,
    width_points: f64,
    height_points: f64,
) -> PillPlacement {
    let width = width_points.max(1.0).round() as i32;
    let height = height_points.max(1.0).round() as i32;
    PillPlacement {
        x: (monitor.work_x + (monitor.work_width - f64::from(width)) / 2.0).round() as i32,
        y: (monitor.work_y + monitor.work_height - f64::from(height) - PILL_BOTTOM_GAP_POINTS)
            .round() as i32,
        width,
        height,
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn placement_for_current_monitor<R: Runtime>(
    pill: &WebviewWindow<R>,
    width_points: f64,
    height_points: f64,
) -> Option<PillPlacement> {
    let monitor = pill
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| pill.primary_monitor().ok().flatten())?;
    Some(placement_for_monitor(
        MonitorSnapshot::from(&monitor),
        width_points,
        height_points,
    ))
}

#[cfg(target_os = "linux")]
pub(crate) fn placement_for_current_monitor<R: Runtime>(
    pill: &WebviewWindow<R>,
    width_points: f64,
    height_points: f64,
) -> Option<PillPlacement> {
    let point = crate::core::hyprland::pill_window().map(|window| DesktopPoint {
        x: f64::from(window.at[0]) + f64::from(window.size[0]) / 2.0,
        y: f64::from(window.at[1]) + f64::from(window.size[1]) / 2.0,
    });
    resolve_pill_placement(pill, point, width_points, height_points)
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn current_placement<R: Runtime>(pill: &WebviewWindow<R>) -> Option<PillPlacement> {
    let size = pill.inner_size().ok()?;
    let pos = pill.outer_position().ok()?;
    if size.width == 0 || size.height == 0 {
        return None;
    }
    Some(PillPlacement {
        x: pos.x,
        y: pos.y,
        width: size.width as i32,
        height: size.height as i32,
    })
}

pub(super) fn dimension_changed(current: f64, desired: f64) -> bool {
    (current - desired).abs() > 1.0
}

pub(super) fn position_changed(current: i32, desired: i32) -> bool {
    current.abs_diff(desired) > 1
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(super) fn should_animate_cross_monitor_move(
    already_placed: bool,
    current: PillPlacement,
    target: PillPlacement,
) -> bool {
    if !already_placed {
        return false;
    }
    let dx = current.x.abs_diff(target.x);
    let dy = current.y.abs_diff(target.y);
    let recenter_budget = current
        .width
        .abs_diff(target.width)
        .saturating_add(current.height.abs_diff(target.height))
        .saturating_add(8);
    dx > recenter_budget || dy > recenter_budget
}

#[cfg(target_os = "linux")]
pub(crate) fn snap_linux_placement_to_mapped_size(
    requested: PillPlacement,
    mapped_size: [i32; 2],
) -> PillPlacement {
    let width = mapped_size[0].max(requested.width).max(1);
    let height = mapped_size[1].max(requested.height).max(1);
    if width == requested.width && height == requested.height {
        return requested;
    }
    PillPlacement {
        x: requested.x - (width - requested.width) / 2,
        y: requested.y - (height - requested.height),
        width,
        height,
    }
}

pub(crate) fn apply_pill_placement<R: Runtime>(
    pill: &WebviewWindow<R>,
    placement: PillPlacement,
) -> bool {
    super::pill_animation::cancel_pending_pill_tween();

    #[cfg(target_os = "linux")]
    let desired_size = {
        let scale = pill.scale_factor().unwrap_or(1.0).max(0.1);
        (
            placement.width.max(1) as f64 * scale,
            placement.height.max(1) as f64 * scale,
        )
    };
    #[cfg(not(target_os = "linux"))]
    let desired_size = (
        placement.width.max(1) as f64,
        placement.height.max(1) as f64,
    );

    let needs_resize = pill
        .inner_size()
        .map(|cur| {
            dimension_changed(cur.width as f64, desired_size.0)
                || dimension_changed(cur.height as f64, desired_size.1)
        })
        .unwrap_or(true);

    #[cfg(not(target_os = "linux"))]
    let needs_reposition = pill
        .outer_position()
        .map(|cur| position_changed(cur.x, placement.x) || position_changed(cur.y, placement.y))
        .unwrap_or(true);

    #[cfg(target_os = "linux")]
    let needs_reposition;

    #[cfg(target_os = "windows")]
    {
        use windows::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
        };

        if needs_resize || needs_reposition {
            if let Ok(hwnd) = pill.hwnd() {
                let mut flags = SWP_NOZORDER | SWP_NOACTIVATE;
                if !needs_resize {
                    flags |= SWP_NOSIZE;
                }
                if !needs_reposition {
                    flags |= SWP_NOMOVE;
                }

                unsafe {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        placement.x,
                        placement.y,
                        placement.width,
                        placement.height,
                        flags,
                    );
                }

                if crate::system::logger::is_verbose() {
                    let actual = current_placement(pill);
                    log::debug!(
                        "pill apply_pill_placement (sync): needs_resize={needs_resize} needs_reposition={needs_reposition} intended={placement:?} actual_after_setwindowpos={actual:?}"
                    );
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(window) = crate::core::hyprland::pill_window() {
            if window.size != [placement.width, placement.height] {
                if let Err(error) = crate::core::hyprland::resize_window(
                    &window.address,
                    placement.width,
                    placement.height,
                ) {
                    log::warn!("Linux pill resize failed: {error}");
                }
            }
            needs_reposition = position_changed(window.at[0], placement.x)
                || position_changed(window.at[1], placement.y);
            if needs_reposition {
                if let Err(error) = crate::core::hyprland::move_window(&window.address, placement.x, placement.y) {
                    log::warn!("Linux pill positioning failed: {error}");
                }
            }
        } else {
            if pill.is_visible().unwrap_or(false) && crate::core::hyprland::session_available() {
                log::warn!("Linux pill placement unavailable: the compositor has not exposed this process's pill window");
            }
            needs_reposition = false;
            if needs_resize {
                pill.set_size(tauri::LogicalSize::new(
                    placement.width as f64,
                    placement.height as f64,
                ))
                .ok();
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let scale = pill.scale_factor().unwrap_or(1.0).max(0.1);
        if needs_resize {
            pill.set_size(tauri::LogicalSize::new(
                placement.width as f64 / scale,
                placement.height as f64 / scale,
            ))
            .ok();
        }
        if needs_reposition {
            pill.set_position(tauri::PhysicalPosition::new(placement.x, placement.y))
                .ok();
        }
    }

    needs_resize || needs_reposition
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_scaled_offset_monitor_keeps_pill_inside_logical_work_area() {
        let placement = placement_for_linux_monitor(
            crate::core::hyprland::LogicalMonitor {
                work_x: 0.0,
                work_y: 1106.0,
                work_width: 2048.0,
                work_height: 1126.0,
            },
            200.0,
            200.0,
        );

        assert_eq!(placement.x, 924);
        assert_eq!(placement.y, 2016);
        assert_eq!(placement.width, 200);
        assert_eq!(placement.height, 200);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_mapped_min_size_keeps_content_bottom_in_the_work_area() {
        let requested = PillPlacement {
            x: 964,
            y: 2144,
            width: 120,
            height: 72,
        };

        assert_eq!(
            snap_linux_placement_to_mapped_size(requested, [200, 200]),
            PillPlacement {
                x: 924,
                y: 2016,
                width: 200,
                height: 200,
            }
        );
        assert_eq!(
            snap_linux_placement_to_mapped_size(requested, [120, 72]),
            requested
        );
    }

    #[test]
    fn dimension_changed_respects_one_pixel_tolerance() {
        assert!(!dimension_changed(380.0, 380.9));
        assert!(dimension_changed(380.0, 475.0));
    }

    #[test]
    fn position_changed_respects_one_pixel_tolerance() {
        assert!(!position_changed(100, 101));
        assert!(position_changed(100, 250));
    }

    fn monitor(
        work_x: i32,
        work_y: i32,
        work_width: u32,
        work_height: u32,
        scale_factor: f64,
    ) -> MonitorSnapshot {
        MonitorSnapshot {
            work_x,
            work_y,
            work_width,
            work_height,
            scale_factor,
        }
    }

    #[test]
    fn prefers_target_monitor_then_primary_monitor() {
        let primary = monitor(0, 0, 1920, 1080, 1.0);
        let secondary = monitor(1920, 0, 2560, 1440, 1.5);

        assert_eq!(
            choose_monitor(Some(secondary), Some(primary)),
            Some(secondary)
        );
        assert_eq!(choose_monitor(None, Some(primary)), Some(primary));
        assert_eq!(choose_monitor(None, None), None);
    }

    #[test]
    fn centers_on_secondary_monitor_with_positive_coordinates() {
        let placement = placement_for_monitor(monitor(1920, 0, 2560, 1400, 1.0), 380.0, 44.0);

        assert_eq!(placement.width, 380);
        assert_eq!(placement.height, 44);
        assert_eq!(placement.x, 3010);
        assert_eq!(placement.y, 1340);
    }

    #[test]
    fn centers_on_monitor_with_negative_coordinates() {
        let placement = placement_for_monitor(monitor(-2560, 0, 2560, 1400, 1.0), 380.0, 44.0);

        assert_eq!(placement.x, -1470);
        assert_eq!(placement.y, 1340);
    }

    #[test]
    fn respects_retina_scaling() {
        let placement = placement_for_monitor(monitor(0, 0, 2880, 1800, 2.0), 380.0, 44.0);

        assert_eq!(placement.width, 760);
        assert_eq!(placement.height, 88);
        assert_eq!(placement.x, 1060);
        assert_eq!(placement.y, 1680);
    }

    #[test]
    fn uses_work_area_bottom_instead_of_full_monitor_height() {
        let placement = placement_for_monitor(monitor(0, 40, 1920, 1040, 1.0), 380.0, 44.0);

        assert_eq!(placement.y, 1020);
    }

    #[test]
    fn content_width_places_narrower_pill_centered() {
        let placement = placement_for_monitor(monitor(0, 0, 1920, 1040, 1.0), 92.0, 44.0);

        assert_eq!(placement.width, 92);
        assert_eq!(placement.x, 914);
        assert_eq!(placement.y, 980);
    }

    #[test]
    fn content_width_scales_into_physical_pixels() {
        let placement = placement_for_monitor(monitor(0, 0, 2880, 1800, 1.5), 92.0, 44.0);

        assert_eq!(placement.width, 138);
        assert_eq!(placement.x, 1371);
    }

    #[test]
    fn taller_pill_window_rises_off_the_work_area_bottom() {
        let tall = placement_for_monitor(monitor(0, 0, 1920, 1040, 1.0), 92.0, 64.0);
        let normal = placement_for_monitor(monitor(0, 0, 1920, 1040, 1.0), 92.0, 44.0);

        assert_eq!(tall.height, 64);
        assert_eq!(tall.y, normal.y - 20);
        assert_eq!(tall.x, normal.x);
    }

    fn placement(x: i32, y: i32, width: i32, height: i32) -> PillPlacement {
        PillPlacement {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn does_not_animate_before_first_placement_even_if_geometry_differs() {
        let current = placement(0, 1000, 380, 44);
        let target = placement(1920, 1340, 570, 66);

        assert!(!should_animate_cross_monitor_move(false, current, target));
    }

    #[test]
    fn animates_once_placed_when_position_differs() {
        let current = placement(0, 1000, 380, 44);
        let target = placement(1920, 1340, 570, 66);

        assert!(should_animate_cross_monitor_move(true, current, target));
    }

    #[test]
    fn does_not_animate_once_placed_when_geometry_already_matches() {
        let current = placement(0, 1000, 380, 44);
        let target = placement(0, 1000, 380, 44);

        assert!(!should_animate_cross_monitor_move(true, current, target));
    }

    #[test]
    fn does_not_animate_same_monitor_size_only_changes() {
        let current = placement(770, 1000, 380, 44);
        let target = placement(770, 1000, 480, 54);

        assert!(!should_animate_cross_monitor_move(true, current, target));
    }

    #[test]
    fn does_not_animate_same_monitor_recenter_when_width_changes() {
        let current = placement(770, 1000, 380, 44);
        let target = placement(720, 1000, 480, 44);

        assert!(!should_animate_cross_monitor_move(true, current, target));
    }
}
