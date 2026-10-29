#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    if verenu::run_internal_output_guardian() { return; }
    #[cfg(target_os = "linux")]
    verenu::recover_output_volume();
    verenu::run();
}
