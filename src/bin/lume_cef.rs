// SPDX-License-Identifier: GPL-3.0-or-later
#[path = "../cef_runtime/mod.rs"]
mod cef_runtime;

fn main() -> std::process::ExitCode {
    match cef_runtime::run() {
        Ok(code) => std::process::ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("Lume TV embedded runtime: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}
