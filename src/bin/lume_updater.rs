// SPDX-License-Identifier: GPL-3.0-or-later
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        println!("lume-updater {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.len() != 2 || args[0] != "--apply" {
        std::process::exit(2);
    }
    if lume_player::updater::run_helper(std::path::Path::new(&args[1])).is_err() {
        std::process::exit(1);
    }
}
