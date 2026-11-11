// SPDX-License-Identifier: GPL-3.0-or-later
//! Sanitized component state shared by CEF's UI thread and the local API.
use serde::Serialize;
use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Unavailable,
    Checking,
    Downloading,
    Installing,
    Installed,
    Error,
}

#[derive(Clone, Default, Serialize)]
pub struct Status {
    pub phase: Phase,
    pub version: String,
    pub installed_at_startup: bool,
    pub restart_required: bool,
    pub error: Option<&'static str>,
    pub elapsed_seconds: u64,
}

#[derive(Default)]
struct State {
    status: Status,
    startup_versions: Option<Vec<String>>,
    started: Option<Instant>,
    last_request: Option<Instant>,
    requested: bool,
    in_flight: bool,
}

#[derive(Default)]
pub struct WidevineService {
    state: Mutex<State>,
    active: AtomicBool,
}

impl WidevineService {
    /// Capture installed manifests before CEF starts. Component registration is
    /// asynchronous, so its first snapshot alone may still show an empty version.
    pub fn capture_startup(&self, profile: &Path) {
        let versions = fs::read_dir(profile.join("WidevineCdm"))
            .into_iter()
            .flatten()
            .take(64)
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path().join("manifest.json");
                if fs::metadata(&path).ok()?.len() > 65_536 {
                    return None;
                }
                let manifest: serde_json::Value =
                    serde_json::from_slice(&fs::read(path).ok()?).ok()?;
                let version = manifest["version"].as_str()?;
                valid_version(version).then(|| version.to_owned())
            })
            .collect::<Vec<_>>();
        let mut state = self.state.lock().expect("component state lock");
        state.status.installed_at_startup = !versions.is_empty();
        state.startup_versions = Some(versions);
    }

    pub fn snapshot(&self) -> Status {
        let state = self.state.lock().expect("component state lock");
        let mut status = state.status.clone();
        status.elapsed_seconds = state.started.map_or(0, |start| start.elapsed().as_secs());
        status
    }

    /// HTTP threads only queue an intent. All CEF calls happen on its UI thread.
    pub fn request_update(&self) -> Status {
        let mut state = self.state.lock().expect("component state lock");
        if self.active.load(Ordering::Relaxed) && !state.in_flight && !state.requested {
            state.requested = true;
            state.started = Some(Instant::now());
            state.status.phase = Phase::Checking;
            state.status.error = None;
        }
        drop(state);
        self.snapshot()
    }

    pub fn start(&self) {
        self.active.store(true, Ordering::Relaxed);
    }
    pub fn stop(&self) {
        self.active.store(false, Ordering::Relaxed);
    }
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn take_request(&self) -> bool {
        let mut state = self.state.lock().expect("component state lock");
        if !state.requested
            || state.in_flight
            || state
                .last_request
                .is_some_and(|time| time.elapsed() < Duration::from_secs(10))
        {
            return false;
        }
        state.requested = false;
        state.in_flight = true;
        state.last_request = Some(Instant::now());
        true
    }

    pub fn observe(&self, phase: Phase, version: &str, linux: bool) {
        let version = if valid_version(version) { version } else { "" };
        let mut state = self.state.lock().expect("component state lock");
        if state.startup_versions.is_none() {
            state.startup_versions = Some(vec![version.to_owned()]);
            state.status.installed_at_startup = !version.is_empty();
        }
        // A callback error can precede a new snapshot. Keep it visible until a
        // retry or real download/install progress, instead of flashing for 1 s.
        let phase = if (state.requested || state.in_flight)
            && !matches!(
                phase,
                Phase::Downloading | Phase::Installing | Phase::Installed
            ) {
            Phase::Checking
        } else if state.status.error.is_some()
            && !state.in_flight
            && !matches!(
                phase,
                Phase::Downloading | Phase::Installing | Phase::Installed
            )
        {
            Phase::Error
        } else {
            phase
        };
        state.status.phase = phase;
        state.status.version = version.to_owned();
        state.status.restart_required = linux
            && phase == Phase::Installed
            && !version.is_empty()
            && !state
                .startup_versions
                .as_ref()
                .is_some_and(|versions| versions.iter().any(|v| v == version));
        if phase != Phase::Error {
            state.status.error = None;
        }
    }

    pub fn complete(&self, error: Option<&'static str>) {
        let mut state = self.state.lock().expect("component state lock");
        state.in_flight = false;
        if let Some(error) = error {
            state.status.phase = Phase::Error;
            state.status.error = Some(error);
        }
    }
}

fn valid_version(version: &str) -> bool {
    !version.is_empty()
        && version != "0.0.0.0"
        && version.len() <= 64
        && version.split('.').count() == 4
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.parse::<u32>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deferred_registration_does_not_misclassify_a_cached_component_as_new() {
        let directory =
            std::env::temp_dir().join(format!("lume-cdm-startup-{}", uuid::Uuid::new_v4()));
        let component = directory.join("WidevineCdm/4.10.3112.0");
        fs::create_dir_all(&component).unwrap();
        fs::write(
            component.join("manifest.json"),
            br#"{"version":"4.10.3112.0"}"#,
        )
        .unwrap();
        let service = WidevineService::default();
        service.capture_startup(&directory);
        service.observe(Phase::Checking, "", true);
        service.observe(Phase::Installed, "4.10.3112.0", true);
        assert!(service.snapshot().installed_at_startup);
        assert!(!service.snapshot().restart_required);
        fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn linux_first_install_needs_restart_but_next_launch_does_not() {
        let service = WidevineService::default();
        service.observe(Phase::Checking, "0.0.0.0", true);
        service.observe(Phase::Downloading, "", true);
        service.observe(Phase::Installed, "4.10.3112.0", true);
        assert!(service.snapshot().restart_required);
        assert!(!service.snapshot().installed_at_startup);
        let next = WidevineService::default();
        next.observe(Phase::Installed, "4.10.3112.0", true);
        assert!(!next.snapshot().restart_required);
        assert!(next.snapshot().installed_at_startup);
    }
    #[test]
    fn other_platforms_can_load_after_download() {
        let service = WidevineService::default();
        service.observe(Phase::Checking, "", false);
        service.observe(Phase::Installed, "4.10.3112.0", false);
        assert!(!service.snapshot().restart_required);
    }
    #[test]
    fn request_is_coalesced_and_preview_never_claims_a_download() {
        let service = WidevineService::default();
        assert_eq!(service.request_update().phase, Phase::Unavailable);
        assert!(!service.take_request());
        service.start();
        service.request_update();
        service.request_update();
        assert!(service.take_request());
        assert!(!service.take_request());
        service.complete(Some("service_error"));
        service.observe(Phase::Checking, "", false);
        assert_eq!(service.snapshot().error, Some("service_error"));
        service.request_update();
        assert!(!service.take_request());
        service.observe(Phase::Error, "", false);
        assert_eq!(service.snapshot().phase, Phase::Checking);
        assert_eq!(service.snapshot().error, None);
        service.state.lock().unwrap().last_request = Some(Instant::now() - Duration::from_secs(11));
        assert!(service.take_request());
        assert!(!service.take_request());
    }
    #[test]
    fn snapshots_only_serialize_component_metadata() {
        let service = WidevineService::default();
        service.observe(Phase::Installed, "https://private.invalid/token", true);
        assert!(service.snapshot().version.is_empty());
        let value = serde_json::to_value(service.snapshot()).unwrap();
        assert!(value.get("progress").is_none());
        assert!(value.get("downloaded_bytes").is_none());
    }
}
