// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    install::{self, InstallMode, Layout},
    network::Network,
    policy::{self, Artifact, SignedManifest},
    Error, Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub automatic: bool,
    pub preview: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            automatic: true,
            preview: false,
        }
    }
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub current_version: &'static str,
    pub phase: String,
    pub available_version: Option<String>,
    pub notes: String,
    pub downloaded: u64,
    pub total: u64,
    pub error: Option<Error>,
    pub settings: Settings,
    pub last_checked: Option<u64>,
    pub next_check_at: u64,
    pub install_mode: InstallMode,
    pub last_install_failed: bool,
}
#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    settings: Settings,
    #[serde(default)]
    last_checked: Option<u64>,
    #[serde(default)]
    next_check_at: u64,
    #[serde(default)]
    retry_after: u64,
}
#[derive(Clone)]
struct Candidate {
    signed: SignedManifest,
    artifact: Artifact,
    version: String,
}
struct State {
    status: Snapshot,
    candidate: Option<Candidate>,
    downloaded: Option<PathBuf>,
}
pub struct UpdateService {
    directory: PathBuf,
    layout: Option<Layout>,
    network: Option<Network>,
    state: Mutex<State>,
    cancelled: AtomicBool,
    exit: AtomicBool,
    last_attempt: AtomicU64,
    retry_after: AtomicU64,
}
impl UpdateService {
    pub fn new(data: &Path) -> Arc<Self> {
        let directory = data.join("updates");
        let _ = fs::create_dir_all(&directory);
        let directory = directory.canonicalize().unwrap_or(directory);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&directory, fs::Permissions::from_mode(0o700));
        }
        let saved = fs::read(directory.join("settings.json"))
            .ok()
            .filter(|b| b.len() < 4096)
            .and_then(|b| serde_json::from_slice::<Saved>(&b).ok())
            .unwrap_or_default();
        let last_install_failed = fs::read(directory.join("install-result.json"))
            .ok()
            .filter(|b| b.len() < 4096)
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .is_some_and(|v| v["success"] == false);
        let layout = Layout::detect();
        Arc::new(Self {
            directory,
            network: Network::new().ok(),
            state: Mutex::new(State {
                status: Snapshot {
                    current_version: env!("CARGO_PKG_VERSION"),
                    phase: "idle".into(),
                    available_version: None,
                    notes: String::new(),
                    downloaded: 0,
                    total: 0,
                    error: (saved.retry_after > now()).then_some(Error::RateLimited),
                    settings: saved.settings,
                    last_checked: saved.last_checked,
                    next_check_at: saved.retry_after.min(now() + 86400),
                    install_mode: layout.as_ref().map(|l| l.mode).unwrap_or(InstallMode::Open),
                    last_install_failed,
                },
                candidate: None,
                downloaded: None,
            }),
            layout,
            cancelled: AtomicBool::new(false),
            exit: AtomicBool::new(false),
            last_attempt: AtomicU64::new(0),
            retry_after: AtomicU64::new(saved.retry_after.min(now() + 86400)),
        })
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("updates lock").status.clone()
    }
    fn save(&self, status: &Snapshot) {
        let value = Saved {
            settings: status.settings.clone(),
            last_checked: status.last_checked,
            next_check_at: status.next_check_at,
            retry_after: self.retry_after.load(Ordering::Relaxed),
        };
        if let Ok(bytes) = serde_json::to_vec(&value) {
            let temp = self.directory.join("settings.tmp");
            if fs::write(&temp, bytes).is_ok() {
                let _ = fs::rename(temp, self.directory.join("settings.json"));
            }
        }
    }
    pub fn settings(&self, automatic: Option<bool>, preview: Option<bool>) -> Result<Snapshot> {
        let mut state = self.state.lock().expect("updates lock");
        if busy(&state.status.phase) {
            return Err(Error::Busy);
        }
        if let Some(value) = automatic {
            state.status.settings.automatic = value;
        }
        if let Some(value) = preview {
            state.status.settings.preview = value;
            state.candidate = None;
            state.downloaded = None;
            state.status.available_version = None;
            state.status.notes.clear();
            state.status.phase = "idle".into();
            state.status.next_check_at = self.retry_after.load(Ordering::Relaxed);
            self.last_attempt.store(0, Ordering::Relaxed);
        }
        self.save(&state.status);
        Ok(state.status.clone())
    }
    pub fn check(self: &Arc<Self>, manual: bool) -> Snapshot {
        let mut state = self.state.lock().expect("updates lock");
        let network_backoff = self.network.as_ref().map(Network::retry_at).unwrap_or(0);
        let backoff = network_backoff.max(self.retry_after.load(Ordering::Relaxed));
        if !busy(&state.status.phase) && backoff > now() {
            state.status.next_check_at = backoff;
            state.status.error = Some(Error::RateLimited);
            return state.status.clone();
        }
        if busy(&state.status.phase)
            || self.last_attempt.load(Ordering::Relaxed).saturating_add(30) > now()
            || (!manual && (!state.status.settings.automatic || state.status.next_check_at > now()))
        {
            return state.status.clone();
        }
        state.status.phase = "checking".into();
        self.last_attempt.store(now(), Ordering::Relaxed);
        state.status.error = None;
        // Coalesce repeated manual clicks; the server can request a longer pause.
        state.status.next_check_at = now() + 30;
        let preview = state.status.settings.preview;
        let shared = self.clone();
        thread::spawn(move || {
            let result = shared.find_release(preview);
            shared.retry_after.store(
                shared.network.as_ref().map(Network::retry_at).unwrap_or(0),
                Ordering::Relaxed,
            );
            let mut state = shared.state.lock().expect("updates lock");
            match result {
                Ok(Some((candidate, notes))) => {
                    let same_download = state
                        .candidate
                        .as_ref()
                        .is_some_and(|c| c.artifact == candidate.artifact)
                        && state.downloaded.as_ref().is_some_and(|p| p.is_file());
                    if !same_download {
                        state.downloaded = None;
                        state.status.downloaded = 0;
                    }
                    state.status.phase = if same_download { "ready" } else { "available" }.into();
                    state.status.available_version = Some(candidate.version.clone());
                    state.status.notes = notes;
                    state.status.total = candidate.artifact.size;
                    state.candidate = Some(candidate);
                    state.status.last_checked = Some(now());
                    state.status.next_check_at = now() + 1800;
                }
                Ok(None) => {
                    state.candidate = None;
                    state.downloaded = None;
                    state.status.available_version = None;
                    state.status.notes.clear();
                    state.status.total = 0;
                    state.status.downloaded = 0;
                    state.status.phase = "up_to_date".into();
                    state.status.last_checked = Some(now());
                    state.status.next_check_at = now() + 1800;
                }
                Err(error) => {
                    state.status.phase = if state.downloaded.is_some() {
                        "ready"
                    } else if state.candidate.is_some() {
                        "available"
                    } else {
                        "error"
                    }
                    .into();
                    state.status.error = Some(error);
                    state.status.next_check_at = (now() + 3600)
                        .max(shared.network.as_ref().map(Network::retry_at).unwrap_or(0));
                }
            }
            shared.save(&state.status);
        });
        state.status.clone()
    }
    fn find_release(&self, preview: bool) -> Result<Option<(Candidate, String)>> {
        let network = self.network.as_ref().ok_or(Error::Network)?;
        for release in policy::releases(
            &network.text(policy::FEED, 2 * 1024 * 1024)?,
            env!("CARGO_PKG_VERSION"),
            preview,
        )? {
            let metadata = release.asset("update.json")?;
            let signature = release.asset("update.sig")?;
            if metadata.size > policy::MAX_MANIFEST as u64 || signature.size > 1024 {
                return Err(Error::Metadata);
            }
            let json = network.text(&metadata.browser_download_url, policy::MAX_MANIFEST)?;
            let sig = network.text(&signature.browser_download_url, 1024)?;
            if json.len() as u64 != metadata.size || sig.len() as u64 != signature.size {
                return Err(Error::Metadata);
            }
            let signed = SignedManifest {
                json: String::from_utf8(json).map_err(|_| Error::Metadata)?,
                signature: String::from_utf8(sig).map_err(|_| Error::Metadata)?,
            };
            let manifest = signed.verify()?;
            if !manifest.eligible(env!("CARGO_PKG_VERSION"), preview)? {
                continue;
            }
            let target = manifest
                .targets
                .get(policy::platform().ok_or(Error::Unsupported)?)
                .ok_or(Error::Metadata)?;
            let artifact = if self.snapshot().install_mode == InstallMode::System
                || (self.snapshot().install_mode == InstallMode::Open && cfg!(target_os = "macos"))
            {
                target.installer.clone().ok_or(Error::Metadata)?
            } else {
                target.archive.clone()
            };
            release.cross_check(&manifest, &artifact)?;
            return Ok(Some((
                Candidate {
                    signed,
                    artifact,
                    version: manifest.version,
                },
                manifest.notes,
            )));
        }
        Ok(None)
    }
    pub fn download(self: &Arc<Self>) -> Result<Snapshot> {
        let mut state = self.state.lock().expect("updates lock");
        if busy(&state.status.phase) {
            return Err(Error::Busy);
        }
        let candidate = state.candidate.clone().ok_or(Error::NotReady)?;
        self.cancelled.store(false, Ordering::Relaxed);
        state.status.phase = "downloading".into();
        state.status.downloaded = 0;
        state.status.error = None;
        state.downloaded = None;
        let shared = self.clone();
        thread::spawn(move || {
            let result = (|| {
                let manifest = candidate.signed.verify()?;
                if !manifest.eligible(env!("CARGO_PKG_VERSION"), true)? {
                    return Err(Error::Metadata);
                }
                shared.network.as_ref().ok_or(Error::Network)?.download(
                    &policy::asset_url(&candidate.version, &candidate.artifact.name)?,
                    &candidate.artifact,
                    &shared.directory,
                    &shared.cancelled,
                    |bytes| {
                        shared.state.lock().expect("updates lock").status.downloaded = bytes;
                    },
                )
            })();
            let mut state = shared.state.lock().expect("updates lock");
            match result {
                Ok(path) => {
                    state.downloaded = Some(path);
                    state.status.phase = "ready".into();
                }
                Err(Error::Cancelled) => {
                    state.status.phase = "available".into();
                    state.status.downloaded = 0;
                }
                Err(error) => {
                    state.status.phase = "available".into();
                    state.status.error = Some(error);
                    state.status.downloaded = 0;
                    if error == Error::RateLimited {
                        let backoff = shared.network.as_ref().map(Network::retry_at).unwrap_or(0);
                        shared.retry_after.store(backoff, Ordering::Relaxed);
                        state.status.next_check_at = backoff;
                        shared.save(&state.status);
                    }
                }
            }
        });
        Ok(state.status.clone())
    }
    pub fn cancel(&self) -> Snapshot {
        self.cancelled.store(true, Ordering::Relaxed);
        self.snapshot()
    }
    pub fn stop(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub fn install(self: &Arc<Self>) -> Result<Snapshot> {
        let mut state = self.state.lock().expect("updates lock");
        if !matches!(state.status.phase.as_str(), "ready" | "opened") {
            return Err(Error::NotReady);
        }
        let candidate = state.candidate.clone().ok_or(Error::NotReady)?;
        let path = state.downloaded.clone().ok_or(Error::NotReady)?;
        state.status.phase = "installing".into();
        state.status.error = None;
        let shared = self.clone();
        thread::spawn(move || {
            let result = (|| {
                candidate.signed.verify()?;
                super::network::verify_file(&path, &candidate.artifact)?;
                match shared.layout.as_ref() {
                    Some(layout) if layout.mode == InstallMode::Replace => {
                        install::spawn_helper(layout, &candidate.signed, &path, &shared.directory)?;
                        shared.exit.store(true, Ordering::Release);
                    }
                    _ => {
                        install::open_package(&path)?;
                        if shared.snapshot().install_mode == InstallMode::System {
                            shared.exit.store(true, Ordering::Release);
                        }
                    }
                }
                Ok(())
            })();
            let mut state = shared.state.lock().expect("updates lock");
            if let Err(error) = result {
                state.status.phase = "ready".into();
                state.status.error = Some(error);
            } else if state.status.install_mode != InstallMode::Replace {
                state.status.phase = "opened".into();
            }
        });
        Ok(state.status.clone())
    }
    pub fn take_exit_request(&self) -> bool {
        self.exit.swap(false, Ordering::AcqRel)
    }
}
fn busy(phase: &str) -> bool {
    matches!(phase, "checking" | "downloading" | "installing")
}
