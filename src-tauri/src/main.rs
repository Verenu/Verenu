#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    if verenu::run_internal_output_guardian() { return; }
    verenu::run();
}
