// SPDX-License-Identifier: GPL-3.0-or-later
// CEF's version-matched bootstrap.exe loads this DLL by executable stem
// and supplies the Windows sandbox context. Do not replace it with a null
// sandbox pointer or run a normal Rust EXE as the packaged browser process.
#[cfg(target_os = "windows")]
#[allow(dead_code)]
#[path = "../../../src/cef_runtime/mod.rs"]
mod cef_runtime;

/// Entry point required by CEF's Windows GUI bootstrap.
///
/// # Safety
/// Called only by the matching CEF bootstrap with its live sandbox context.
#[cfg(target_os = "windows")]
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn RunWinMain(
    instance: cef::sys::HINSTANCE,
    _command_line: *const u8,
    _command_show: i32,
    sandbox_info: *mut u8,
) -> i32 {
    // Never unwind across the bootstrap ABI boundary.
    std::panic::catch_unwind(|| {
        match cef_runtime::run_with_main_args(Some(cef::MainArgs { instance }), sandbox_info) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("Lume TV embedded runtime: {error}");
                1
            }
        }
    })
    .unwrap_or(1)
}
