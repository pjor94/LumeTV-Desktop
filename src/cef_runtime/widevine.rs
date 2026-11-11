// SPDX-License-Identifier: GPL-3.0-or-later
use cef::*;
use lume_player::widevine::{Phase, WidevineService};
use std::sync::Arc;

// Chromium's Widevine component signing-key ID, shared by the supported OSes.
const COMPONENT_ID: &str = "oimompecagnajdejgnnjijobebaeigek";

pub(super) fn start(service: Arc<WidevineService>) {
    service.start();
    poll(service);
}

fn poll(service: Arc<WidevineService>) {
    if !service.is_active() {
        return;
    }
    // This function is entered from on_context_initialized and rescheduled only
    // on TID_UI. Never call the updater from the HTTP server or renderer.
    if let Some(updater) = component_updater_get() {
        let id = CefString::from(COMPONENT_ID);
        // CefComponent is a snapshot: obtain a fresh one on every poll.
        if let Some(component) = updater.component_by_id(Some(&id)) {
            let state = component.state();
            let phase = match state {
                ComponentState::NEW | ComponentState::CHECKING => Phase::Checking,
                ComponentState::CAN_UPDATE | ComponentState::DOWNLOADING => Phase::Downloading,
                ComponentState::DECOMPRESSING
                | ComponentState::PATCHING
                | ComponentState::UPDATING => Phase::Installing,
                ComponentState::UPDATED | ComponentState::UP_TO_DATE | ComponentState::RUN => {
                    Phase::Installed
                }
                ComponentState::UPDATE_ERROR => Phase::Error,
                _ => Phase::Unavailable,
            };
            service.observe(
                phase,
                &CefString::from(&component.version()).to_string(),
                cfg!(target_os = "linux"),
            );
            if service.take_request() {
                let mut callback = UpdateComplete::new(service.clone());
                updater.update(
                    Some(&id),
                    ComponentUpdatePriority::FOREGROUND,
                    Some(&mut callback),
                );
            }
        }
    }
    let mut task = PollWidevine::new(service);
    post_delayed_task(ThreadId::UI, Some(&mut task), 1000);
}

wrap_task! {
    struct PollWidevine { service: Arc<WidevineService>, }
    impl Task { fn execute(&self) { poll(self.service.clone()); } }
}
wrap_component_update_callback! {
    struct UpdateComplete { service: Arc<WidevineService>, }
    impl ComponentUpdateCallback {
        fn on_complete(&self, _component_id: Option<&CefString>, error: ComponentUpdateError) {
            let error = match error {
                ComponentUpdateError::NONE | ComponentUpdateError::UPDATE_IN_PROGRESS => None,
                ComponentUpdateError::UPDATE_CANCELED => Some("canceled"),
                ComponentUpdateError::RETRY_LATER => Some("retry_later"),
                ComponentUpdateError::UPDATE_CHECK_ERROR => Some("check_error"),
                ComponentUpdateError::CRX_NOT_FOUND => Some("component_not_found"),
                _ => Some("service_error"),
            };
            self.service.complete(error);
        }
    }
}
