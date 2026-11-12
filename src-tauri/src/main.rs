#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

#[cfg(all(feature = "perf-harness", target_os = "macos"))]
#[path = "../../scripts/perf/app.rs"]
mod perf;

#[cfg(all(feature = "perf-harness", target_os = "macos"))]
fn main() -> ExitCode {
    perf::run()
}

#[cfg(not(all(feature = "perf-harness", target_os = "macos")))]
fn main() -> ExitCode {
    app_lib::run()
}
