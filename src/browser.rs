use anyhow::{bail, Context, Result};
use std::env;
use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// Prefer an installed Chrome or Edge: a generic Chromium build may lack a CDM.
pub fn detect(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        if is_executable(path) {
            return Ok(path.to_path_buf());
        }
        if path.components().count() == 1 {
            if let Some(found) = find_on_path(path.as_os_str()) {
                return Ok(found);
            }
        }
        bail!("The specified browser executable could not be found or executed");
    }

    candidates()
        .into_iter()
        .find(|path| is_executable(path))
        .context("No supported browser found. Install Google Chrome or Microsoft Edge, or specify its executable with --browser")
}

/// Launch a separate browser profile without changing the user's normal profile.
pub fn launch(binary: &Path, profile: &Path, url: &str, headless: bool) -> Result<Child> {
    if profile.as_os_str().is_empty() {
        bail!("A dedicated browser profile directory is required");
    }
    let profile = if profile.is_absolute() {
        profile.to_path_buf()
    } else {
        env::current_dir()
            .context("Could not resolve the browser profile directory")?
            .join(profile)
    };
    std::fs::create_dir_all(&profile).context("Could not create the browser profile directory")?;

    Command::new(binary)
        .args(launch_args(&profile, url, headless))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Could not launch the browser")
}

/// Query version metadata only; never include a playback URL in the subprocess.
pub fn version(binary: &Path) -> Option<String> {
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut bytes = Vec::new();
                child
                    .stdout
                    .take()?
                    .take(4096)
                    .read_to_end(&mut bytes)
                    .ok()?;
                return String::from_utf8_lossy(&bytes)
                    .lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(|line| line.chars().take(256).collect());
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn launch_args(profile: &Path, url: &str, headless: bool) -> Vec<OsString> {
    let mut profile_arg = OsString::from("--user-data-dir=");
    profile_arg.push(profile);
    let mut app_arg = OsString::from("--app=");
    app_arg.push(url);
    let mut args = vec![
        profile_arg,
        OsString::from("--no-first-run"),
        OsString::from("--no-default-browser-check"),
        OsString::from("--autoplay-policy=no-user-gesture-required"),
        app_arg,
    ];
    if headless {
        // Diagnostics only: this does not establish headful DRM/GPU compatibility.
        args.push(OsString::from("--headless=new"));
    }
    args
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_on_path(name: &std::ffi::OsStr) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| is_executable(candidate))
    })
}

#[cfg(target_os = "macos")]
fn candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for name in ["Google Chrome", "Microsoft Edge"] {
        paths.push(PathBuf::from(format!(
            "/Applications/{name}.app/Contents/MacOS/{name}"
        )));
        if let Some(home_dir) = env::var_os("HOME") {
            paths.push(
                PathBuf::from(home_dir)
                    .join(format!("Applications/{name}.app/Contents/MacOS/{name}")),
            );
        }
    }
    paths
}

#[cfg(target_os = "windows")]
fn candidates() -> Vec<PathBuf> {
    let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
        .iter()
        .filter_map(env::var_os)
        .map(PathBuf::from)
        .collect();
    let mut paths = Vec::new();
    for suffix in [
        "Google/Chrome/Application/chrome.exe",
        "Microsoft/Edge/Application/msedge.exe",
    ] {
        paths.extend(roots.iter().map(|root| root.join(suffix)));
    }
    for name in ["chrome.exe", "msedge.exe"] {
        if let Some(path) = find_on_path(std::ffi::OsStr::new(name)) {
            paths.push(path);
        }
    }
    paths
}

#[cfg(target_os = "linux")]
fn candidates() -> Vec<PathBuf> {
    [
        "google-chrome",
        "google-chrome-stable",
        "microsoft-edge",
        "microsoft-edge-stable",
        // These fallbacks must be reported as unverified for Widevine.
        "chromium",
        "chromium-browser",
    ]
    .iter()
    .filter_map(|name| find_on_path(std::ffi::OsStr::new(name)))
    .collect()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn candidates() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_launch_keeps_profile_and_url_as_individual_arguments() {
        let profile = Path::new("/tmp/lume profile");
        let args = launch_args(profile, "http://127.0.0.1:12345/", false);
        assert_eq!(args[0], OsString::from("--user-data-dir=/tmp/lume profile"));
        assert_eq!(args[4], OsString::from("--app=http://127.0.0.1:12345/"));
        assert!(!args.iter().any(|arg| arg == "--headless=new"));
        assert!(!args.iter().any(|arg| arg == "--no-sandbox"));
        assert!(!args.iter().any(|arg| arg == "--disable-web-security"));
    }

    #[test]
    fn headless_is_explicitly_opted_in() {
        let args = launch_args(Path::new("/tmp/lume"), "http://127.0.0.1:12345/", true);
        assert_eq!(args.last(), Some(&OsString::from("--headless=new")));
    }
}
