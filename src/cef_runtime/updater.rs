// SPDX-License-Identifier: GPL-3.0-or-later
use super::app::{close_browser, BrowserState};
use cef::*;
use lume_player::updater::UpdateService;
use std::sync::{Arc, Mutex};

pub(super) fn start(service: Arc<UpdateService>, state: Arc<Mutex<BrowserState>>) {
    let mut task = PollUpdates::new(service, state);
    post_delayed_task(ThreadId::UI, Some(&mut task), 500);
}
wrap_task! {
    struct PollUpdates { service: Arc<UpdateService>, state: Arc<Mutex<BrowserState>>, }
    impl Task {
        fn execute(&self) {
            if self.service.take_exit_request() { close_browser(&self.state); }
            else { start(self.service.clone(), self.state.clone()); }
        }
    }
}
