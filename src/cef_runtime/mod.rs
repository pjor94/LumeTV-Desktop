// SPDX-License-Identifier: GPL-3.0-or-later
mod api;
mod app;
#[cfg(target_os = "macos")]
mod mac;

use anyhow::{bail, Context, Result};
use cef::{args::Args, *};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const PRIVATE_SWITCHES: &[&str] = &[
    "lume-url",
    "lume-profile",
    "lume-widevine-cdm",
    "lume-runtime-info",
    "lume-version",
];

struct Config {
    url: String,
    origin: String,
    profile: PathBuf,
    external_cdm_argument: bool,
    runtime_info: Option<PathBuf>,
}

pub(super) fn startup_trace(stage: &str) {
    use std::io::Write;
    if let Some(path) = std::env::var_os("LUME_STARTUP_TRACE") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{stage}");
        }
    }
}

impl Config {
    fn from_args() -> Result<Self> {
        let mut url = None;
        let mut profile = None;
        let mut runtime_info = None;
        let mut external_cdm_argument = false;
        for arg in std::env::args().skip(1) {
            if let Some(value) = arg.strip_prefix("--lume-url=") {
                url = Some(value.to_owned());
            } else if let Some(value) = arg.strip_prefix("--lume-profile=") {
                profile = Some(PathBuf::from(value));
            } else if let Some(value) = arg.strip_prefix("--lume-runtime-info=") {
                runtime_info = Some(PathBuf::from(value));
            } else if let Some(value) = arg.strip_prefix("--lume-widevine-cdm=") {
                if !Path::new(value).is_dir() {
                    bail!("The optional CDM source directory does not exist");
                }
                // Installation is handled explicitly by profile preparation. This
                // argument is provenance only, not a claim that CEF loaded a CDM.
                external_cdm_argument = true;
            }
        }
        let url = url.context("A --lume-url loopback player URL is required")?;
        let origin = loopback_origin(&url)?;
        let profile = profile.context("A dedicated --lume-profile directory is required")?;
        if !profile.is_absolute() {
            bail!("The embedded profile directory must be absolute");
        }
        // The parent harness marks its own profiles before launching this process.
        if !profile.join(".lume-player-profile").is_file() {
            bail!("The embedded profile must be marked as a Lume TV test profile");
        }
        let profile = profile
            .canonicalize()
            .context("Could not resolve the embedded profile directory")?;
        Ok(Self {
            url,
            origin,
            profile,
            external_cdm_argument,
            runtime_info,
        })
    }

    fn write_runtime_info(&self) -> Result<()> {
        if let Some(path) = &self.runtime_info {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .context("Could not create the runtime metadata directory")?;
            }
            let mut info = compiled_info();
            info["external_cdm_argument"] = self.external_cdm_argument.into();
            info["cdm_state"] = "not_verified".into();
            info["cdm_mechanism"] = "profile_component_updater".into();
            std::fs::write(path, serde_json::to_vec_pretty(&info)?)
                .context("Could not write embedded runtime metadata")?;
        }
        Ok(())
    }
}

fn compiled_info() -> serde_json::Value {
    let cef_version = std::ffi::CStr::from_bytes_with_nul(cef::sys::CEF_VERSION)
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unavailable".to_owned());
    let chromium_version = format!(
        "{}.{}.{}.{}",
        cef::sys::CHROME_VERSION_MAJOR,
        cef::sys::CHROME_VERSION_MINOR,
        cef::sys::CHROME_VERSION_BUILD,
        cef::sys::CHROME_VERSION_PATCH,
    );
    serde_json::json!({
        "backend": "embedded_cef",
        "cef_expected": cef_version,
        "cef_api_version": api::selected_version(),
        "chromium": chromium_version,
        "runtime_loaded": false,
        "sandbox": true,
        "compiled_os": std::env::consts::OS,
        "compiled_arch": std::env::consts::ARCH,
    })
}

fn loopback_origin(value: &str) -> Result<String> {
    let url =
        reqwest::Url::parse(value).map_err(|_| anyhow::anyhow!("Invalid local player URL"))?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "http"
        || !matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        bail!("The embedded player must use an HTTP loopback URL with an explicit port");
    }
    Ok(url.origin().ascii_serialization())
}

fn strip_private_switches(command_line: &CommandLine) {
    for name in PRIVATE_SWITCHES {
        command_line.remove_switch(Some(&CefString::from(*name)));
    }
}

pub fn run() -> Result<i32> {
    run_with_main_args(None, std::ptr::null_mut())
}

pub fn run_with_main_args(main_args: Option<MainArgs>, sandbox_info: *mut u8) -> Result<i32> {
    startup_trace("runtime_entry");
    #[cfg(target_os = "windows")]
    if sandbox_info.is_null() {
        bail!("Launch the Windows app through its bundled CEF bootstrap executable");
    }
    if std::env::args().any(|arg| arg == "--lume-version") {
        println!("{}", compiled_info());
        return Ok(0);
    }

    #[cfg(target_os = "macos")]
    let _library = {
        let executable =
            std::env::current_exe().context("Could not locate the embedded executable")?;
        let framework = executable
            .parent()
            .context("Invalid embedded executable location")?
            .join(
                "../Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework",
            );
        if !framework.is_file() {
            bail!("Run the embedded executable from its prepared .app bundle");
        }
        let loader = library_loader::LibraryLoader::new(&executable, false);
        if !loader.load() {
            bail!("Could not load the embedded CEF framework");
        }
        loader
    };

    api::initialize_api()?;
    startup_trace("api_initialized");
    // Args::new reads the process command line; Args::default is empty on
    // Unix, so the two constructors are not interchangeable here.
    let args = match main_args {
        Some(args) => Args::from(args),
        None => Args::new(),
    };
    let subprocess_code =
        execute_process(Some(args.as_main_args()), None::<&mut App>, sandbox_info);
    if subprocess_code >= 0 {
        return Ok(subprocess_code);
    }

    // A double click launches the application itself. The diagnostic CLI can
    // still provide an explicit isolated player URL with --lume-url.
    let desktop = if std::env::args().any(|arg| arg.starts_with("--lume-url=")) {
        None
    } else {
        let directory = std::env::var_os("LUME_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(lume_player::desktop::default_data_directory);
        Some((
            lume_player::desktop::DesktopServer::start(&directory)?,
            directory,
        ))
    };
    let config = Arc::new(if let Some((server, directory)) = &desktop {
        let profile = directory.join("browser");
        std::fs::create_dir_all(&profile).context("Could not create Lume TV profile")?;
        std::fs::write(
            profile.join(".lume-player-profile"),
            b"Lume TV application profile\n",
        )?;
        Config {
            url: server.launch_url.clone(),
            origin: server.origin.clone(),
            profile: profile.canonicalize()?,
            external_cdm_argument: false,
            runtime_info: None,
        }
    } else {
        Config::from_args()?
    });
    config.write_runtime_info()?;
    std::fs::create_dir_all(config.profile.join("Default"))
        .context("Could not create the embedded browser cache")?;
    let state = Arc::new(Mutex::new(app::BrowserState::default()));

    #[cfg(target_os = "macos")]
    let _application = mac::setup(state.clone());

    let mut application = app::LumeApp::new(config.clone(), state);
    let settings = Settings {
        no_sandbox: 0,
        root_cache_path: CefString::from(config.profile.to_string_lossy().as_ref()),
        // The browser UI uses an in-memory context. Favorites, guide and
        // settings persist in Rust; component downloads use root_cache_path.
        cache_path: CefString::default(),
        // The embedded document is our authenticated local UI. Provider HTTP
        // sessions live in Rust and never require browser cookies. Keeping the
        // cookie store disabled also avoids consulting another Chromium app's
        // Safe Storage keychain item in this CEF 150 build on macOS.
        cookieable_schemes_exclude_defaults: 1,
        log_severity: LogSeverity::DISABLE,
        ..Default::default()
    };
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut application),
        sandbox_info,
    ) != 1
    {
        bail!("CEF initialization failed");
    }
    startup_trace("cef_initialized");
    #[cfg(target_os = "macos")]
    mac::install_menu(&_application);
    run_message_loop();
    // Explicit development diagnostics only. The snapshot is sanitized in Rust.
    if let (Some((server, _)), Some(path)) = (&desktop, std::env::var_os("LUME_PLAYBACK_REPORT")) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&server.playback_snapshot()) {
            if std::fs::write(path, bytes).is_err() {
                startup_trace("report_write_failed");
            }
        }
    }
    // Close the Rust HTTP server before shutting down Chromium. Preferences
    // were already written synchronously; the CEF UI context is in memory.
    drop(desktop);
    #[cfg(target_os = "macos")]
    let shutdown_guard = {
        let (sender, receiver) = std::sync::mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            // CEF 150 can leave OSCrypt waiting for an unavailable macOS
            // keychain even after every browser/window has closed. Bound this
            // final teardown without changing the system keychain or using a
            // mock key. This guard only runs after the UI loop has ended.
            if receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .is_err()
            {
                startup_trace("shutdown_timeout");
                std::process::exit(0);
            }
        });
        (sender, handle)
    };
    shutdown();
    #[cfg(target_os = "macos")]
    {
        let _ = shutdown_guard.0.send(());
        let _ = shutdown_guard.1.join();
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_navigation_requires_an_explicit_loopback_origin() {
        assert_eq!(
            loopback_origin("http://127.0.0.1:3000/private-player").unwrap(),
            "http://127.0.0.1:3000"
        );
        assert!(loopback_origin("https://127.0.0.1:3000/").is_err());
        assert!(loopback_origin("http://example.com:3000/").is_err());
        assert!(loopback_origin("http://user:secret@127.0.0.1:3000/").is_err());
        assert!(loopback_origin("http://127.0.0.1/").is_err());
    }
}
