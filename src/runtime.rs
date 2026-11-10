// SPDX-License-Identifier: GPL-3.0-or-later
//! Runtime selection without any dependency on CEF or a fallback between engines.
use crate::browser;
use anyhow::{bail, Context, Result};
use clap::ValueEnum;
use serde_json::{json, Value};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Backend {
    Cef,
    Browser,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Self::Cef => "embedded_cef",
            Self::Browser => "installed_browser",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Cef => "cef",
            Self::Browser => "browser",
        }
    }

    pub fn default_profile(self) -> PathBuf {
        PathBuf::from(match self {
            Self::Cef => "profiles/cef-probe",
            Self::Browser => "profiles/chrome-probe",
        })
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeSelection {
    pub backend: Backend,
    pub browser: Option<PathBuf>,
    pub cef_executable: Option<PathBuf>,
    pub widevine_cdm: Option<PathBuf>,
}

impl RuntimeSelection {
    pub fn validate(&self, headless: bool) -> Result<()> {
        match self.backend {
            Backend::Cef => {
                if self.browser.is_some() {
                    bail!("--browser requires --runtime browser");
                }
                if headless {
                    bail!("The embedded CEF runtime currently supports headful mode only");
                }
            }
            Backend::Browser => {
                if self.cef_executable.is_some() || self.widevine_cdm.is_some() {
                    bail!("--cef-executable and --widevine-cdm require --runtime cef");
                }
            }
        }
        Ok(())
    }

    pub fn missing_reason(&self) -> &'static str {
        match self.backend {
            Backend::Cef => "cef_runtime_not_found",
            Backend::Browser => "browser_not_found",
        }
    }

    pub fn report_metadata(&self) -> Value {
        json!({
            "backend": self.backend.name(),
            "embedded_runtime": self.backend == Backend::Cef,
            "embedded_runtime_initialized": false,
            "embedded_runtime_verified": false,
            "distribution_verified": false,
            "runtime_detected": false,
            "cdm_source_supplied": self.widevine_cdm.is_some(),
            "widevine": "not_tested",
        })
    }

    pub fn detect(&self) -> Result<RuntimeSpec> {
        self.validate(false)?;
        let executable = match self.backend {
            Backend::Browser => browser::detect(self.browser.as_deref())?,
            Backend::Cef => {
                if let Some(explicit) = &self.cef_executable {
                    if !is_executable(explicit) {
                        bail!("The specified embedded CEF executable is unavailable");
                    }
                    explicit.clone()
                } else {
                    let current = env::current_exe().context("Cannot locate this executable")?;
                    let working = env::current_dir().context("Cannot locate working directory")?;
                    cef_candidates(&current, &working)
                        .into_iter()
                        .find(|candidate| is_executable(candidate))
                        .context("Embedded CEF runtime not found; build the bundled lume-cef runtime or specify --cef-executable")?
                }
            }
        };
        Ok(RuntimeSpec {
            selection: self.clone(),
            executable,
        })
    }
}

pub struct RuntimeSpec {
    selection: RuntimeSelection,
    executable: PathBuf,
}

pub struct LaunchOptions<'a> {
    pub profile: &'a Path,
    pub url: &'a str,
    pub headless: bool,
}

impl RuntimeSpec {
    pub fn report_metadata(&self) -> Value {
        let mut metadata = self.selection.report_metadata();
        metadata["runtime_detected"] = json!(true);
        match self.selection.backend {
            Backend::Browser => {
                metadata["browser_version"] = json!(browser::version(&self.executable));
            }
            Backend::Cef => {
                // This sidecar describes the build; it is not proof of loaded codecs/CDM.
                #[cfg(target_os = "macos")]
                let path = self
                    .executable
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join("../Resources/runtime.json");
                #[cfg(not(target_os = "macos"))]
                let path = self.executable.with_file_name("runtime.json");
                if fs::metadata(&path).is_ok_and(|m| m.len() <= 64 * 1024) {
                    if let Ok(bytes) = fs::read(path) {
                        if let Ok(sidecar) = serde_json::from_slice::<Value>(&bytes) {
                            copy_build_metadata(&sidecar, &mut metadata);
                        }
                    }
                }
            }
        }
        metadata
    }

    pub fn launch(&self, options: LaunchOptions<'_>) -> Result<RuntimeProcess> {
        self.selection.validate(options.headless)?;
        let child = match self.selection.backend {
            Backend::Browser => browser::launch(
                &self.executable,
                options.profile,
                options.url,
                options.headless,
            )?,
            Backend::Cef => {
                let profile = absolute_path(options.profile)?;
                let cdm = self
                    .selection
                    .widevine_cdm
                    .as_deref()
                    .map(absolute_path)
                    .transpose()?;
                Command::new(&self.executable)
                    .args(cef_launch_args(&profile, options.url, cdm.as_deref()))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .context("Cannot launch the embedded CEF runtime")?
            }
        };
        Ok(RuntimeProcess { child: Some(child) })
    }
}

/// Owns the entire runtime host process, including cleanup on early error paths.
pub struct RuntimeProcess {
    child: Option<Child>,
}

impl RuntimeProcess {
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.child {
            Some(child) => child.try_wait(),
            None => Ok(None),
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

impl Drop for RuntimeProcess {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    if path.as_os_str().is_empty() {
        bail!("An empty runtime path is not permitted");
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(env::current_dir()
            .context("Cannot resolve a runtime path")?
            .join(path))
    }
}

fn cef_launch_args(profile: &Path, url: &str, cdm: Option<&Path>) -> Vec<OsString> {
    let mut url_arg = OsString::from("--lume-url=");
    url_arg.push(url);
    let mut profile_arg = OsString::from("--lume-profile=");
    profile_arg.push(profile);
    let mut args = vec![url_arg, profile_arg];
    if let Some(path) = cdm {
        let mut cdm_arg = OsString::from("--lume-widevine-cdm=");
        cdm_arg.push(path);
        args.push(cdm_arg);
    }
    args
}

fn copy_build_metadata(sidecar: &Value, metadata: &mut Value) {
    for name in ["cef_version", "chromium_version"] {
        if let Some(version) = sidecar[name].as_str().filter(|version| {
            !version.is_empty()
                && version.len() <= 128
                && version.as_bytes()[0].is_ascii_digit()
                && version.contains('.')
                && version
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".+-_".contains(c))
        }) {
            metadata[name] = json!(version);
        }
    }
    metadata["codec_build"] = json!(sidecar["codec_build"]
        .as_str()
        .filter(|value| ["stock", "custom"].contains(value))
        .unwrap_or("unknown"));
}

fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn cef_candidates(current_executable: &Path, working_directory: &Path) -> Vec<PathBuf> {
    let mut candidates = vec![];
    if let Some(parent) = current_executable.parent() {
        #[cfg(target_os = "macos")]
        {
            candidates.push(parent.join("Lume TV.app/Contents/MacOS/lume-cef"));
            candidates.push(parent.join("lume-cef.app/Contents/MacOS/lume-cef"));
            candidates.push(parent.join("../Helpers/lume-cef.app/Contents/MacOS/lume-cef"));
            if parent.file_name() == Some(std::ffi::OsStr::new("MacOS")) {
                candidates.push(parent.join("lume-cef"));
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            if cfg!(windows) {
                candidates.push(parent.join("LumeTV.exe"));
            }
            candidates.push(parent.join(if cfg!(windows) {
                "lume-cef.exe"
            } else {
                "lume-cef"
            }));
        }
    }
    for build in [
        "target/cef-obs-arm64/debug",
        "target/native-arm64/debug",
        "target/debug",
        "target/native-arm64/release",
        "target/release",
    ] {
        #[cfg(target_os = "macos")]
        {
            candidates.push(
                working_directory
                    .join(build)
                    .join("Lume TV.app/Contents/MacOS/lume-cef"),
            );
            candidates.push(
                working_directory
                    .join(build)
                    .join("lume-cef.app/Contents/MacOS/lume-cef"),
            );
        }
        #[cfg(not(target_os = "macos"))]
        candidates.push(working_directory.join(build).join(if cfg!(windows) {
            "lume-cef.exe"
        } else {
            "lume-cef"
        }));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection(backend: Backend) -> RuntimeSelection {
        RuntimeSelection {
            backend,
            browser: None,
            cef_executable: None,
            widevine_cdm: None,
        }
    }

    #[test]
    fn runtime_selection_never_silently_changes_backend() {
        let mut cef = selection(Backend::Cef);
        cef.cef_executable =
            Some(env::temp_dir().join(format!("missing-cef-{}", uuid::Uuid::new_v4())));
        assert!(cef.detect().is_err());
        assert_eq!(cef.report_metadata()["backend"], "embedded_cef");
        assert_eq!(cef.missing_reason(), "cef_runtime_not_found");
        cef.browser = Some(PathBuf::from("installed-chrome"));
        assert!(cef.validate(false).is_err());
        assert!(selection(Backend::Cef).validate(true).is_err());
        assert!(selection(Backend::Browser).validate(true).is_ok());
    }

    #[test]
    fn cdm_source_is_cef_only_and_does_not_claim_availability() {
        let mut request = selection(Backend::Browser);
        request.widevine_cdm = Some(PathBuf::from("external WidevineCdm"));
        assert!(request.validate(false).is_err());
        request.backend = Backend::Cef;
        assert!(request.validate(false).is_ok());
        let metadata = request.report_metadata();
        assert_eq!(metadata["cdm_source_supplied"], true);
        assert_eq!(metadata["widevine"], "not_tested");
        assert_ne!(
            Backend::Cef.default_profile(),
            Backend::Browser.default_profile()
        );
    }

    #[test]
    fn cef_arguments_preserve_paths_without_chrome_switches_or_shell_interpolation() {
        let args = cef_launch_args(
            Path::new("/profile with spaces"),
            "http://127.0.0.1:1234/#private",
            Some(Path::new("/external WidevineCdm")),
        );
        assert_eq!(
            args,
            vec![
                OsString::from("--lume-url=http://127.0.0.1:1234/#private"),
                OsString::from("--lume-profile=/profile with spaces"),
                OsString::from("--lume-widevine-cdm=/external WidevineCdm")
            ]
        );
        assert!(args
            .iter()
            .all(|arg| !arg.to_string_lossy().contains("--widevine-cdm-path")));
    }

    #[test]
    fn sidecar_only_describes_build_and_cannot_override_backend_or_verification() {
        let mut metadata = selection(Backend::Cef).report_metadata();
        copy_build_metadata(
            &json!({"cef_version":"156.0.4+g1234", "chromium_version":"156.0.8078.80", "codec_build":"stock", "backend":"installed_browser", "embedded_runtime_verified":true, "token":"SECRET"}),
            &mut metadata,
        );
        assert_eq!(metadata["backend"], "embedded_cef");
        assert_eq!(metadata["cef_version"], "156.0.4+g1234");
        assert_eq!(metadata["codec_build"], "stock");
        assert_eq!(metadata["embedded_runtime_verified"], false);
        assert!(!metadata.to_string().contains("SECRET"));
        copy_build_metadata(
            &json!({"cef_version":"https://SECRET", "codec_build":"SECRET"}),
            &mut metadata,
        );
        assert!(!metadata.to_string().contains("SECRET"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_detection_prefers_bundle_over_unbundled_cargo_executable() {
        let candidates = cef_candidates(
            Path::new("/build/debug/lume-player"),
            Path::new("/workspace"),
        );
        assert_eq!(
            candidates[0],
            PathBuf::from("/build/debug/Lume TV.app/Contents/MacOS/lume-cef")
        );
        assert!(!candidates.contains(&PathBuf::from("/build/debug/lume-cef")));
        assert!(candidates.contains(&PathBuf::from(
            "/workspace/target/native-arm64/debug/lume-cef.app/Contents/MacOS/lume-cef"
        )));
    }
}
