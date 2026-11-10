// SPDX-License-Identifier: GPL-3.0-or-later
use cef::{args::Args, *};
#[path = "../cef_runtime/api.rs"]
mod cef_api;

fn main() -> std::process::ExitCode {
    let args = Args::new();

    // Match cef-rs's helper initialization order: the sandbox must precede CEF.
    #[cfg(target_os = "macos")]
    let _sandbox = {
        let mut sandbox = cef::sandbox::Sandbox::new();
        sandbox.initialize(args.as_main_args());
        sandbox
    };
    #[cfg(target_os = "macos")]
    let _loader = {
        let loader = library_loader::LibraryLoader::new(&std::env::current_exe().unwrap(), true);
        assert!(loader.load(), "Could not load the embedded CEF framework");
        loader
    };

    if let Err(error) = cef_api::initialize_api() {
        eprintln!("CEF helper startup failed: {error}");
        return std::process::ExitCode::FAILURE;
    }
    let code = execute_process(
        Some(args.as_main_args()),
        None::<&mut App>,
        std::ptr::null_mut(),
    );
    if code < 0 {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::from(code.min(255) as u8)
    }
}
