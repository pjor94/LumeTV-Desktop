// SPDX-License-Identifier: GPL-3.0-or-later
use super::{strip_private_switches, Config};
use cef::*;
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub(super) struct BrowserState {
    browser: Option<Browser>,
    window: Option<Window>,
}

#[cfg(target_os = "macos")]
pub(super) fn close_browser(state: &Arc<Mutex<BrowserState>>) {
    let window = state.lock().ok().and_then(|state| state.window.clone());
    if let Some(window) = window {
        window.close();
        return;
    }
    let browser = state.lock().ok().and_then(|state| state.browser.clone());
    if let Some(host) = browser.and_then(|browser| browser.host()) {
        host.close_browser(0);
    }
}

wrap_app! {
    pub(super) struct LumeApp {
        config: Arc<Config>,
        state: Arc<Mutex<BrowserState>>,
    }
    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            super::startup_trace("command_line");
            if let Some(command_line) = command_line {
                strip_private_switches(command_line);
                command_line.append_switch_with_value(
                    Some(&CefString::from("log-severity")),
                    Some(&CefString::from("disable")),
                );
                command_line.append_switch_with_value(
                    Some(&CefString::from("autoplay-policy")),
                    Some(&CefString::from("no-user-gesture-required")),
                );
                command_line.append_switch_with_value(
                    Some(&CefString::from("component-updater")),
                    Some(&CefString::from("fast-update")),
                );
            }
        }
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            super::startup_trace("browser_process_handler");
            Some(LumeBrowserProcess::new(self.config.clone(), self.state.clone()))
        }
    }
}

wrap_browser_process_handler! {
    struct LumeBrowserProcess {
        config: Arc<Config>,
        state: Arc<Mutex<BrowserState>>,
    }
    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            super::startup_trace("context_initialized");
            let mut client = LumeClient::new(self.config.clone(), self.state.clone());
            let mut view_delegate = LumeBrowserViewDelegate::new();
            let settings = BrowserSettings::default();
            let context_settings = RequestContextSettings {
                cookieable_schemes_exclude_defaults: 1,
                ..Default::default()
            };
            let mut context = request_context_create_context(Some(&context_settings), None);
            let Some(browser_view) = browser_view_create(
                Some(&mut client),
                Some(&CefString::from(self.config.url.as_str())),
                Some(&settings),
                None,
                context.as_mut(),
                Some(&mut view_delegate),
            ) else {
                quit_message_loop();
                return;
            };
            super::startup_trace("browser_view_created");
            let mut delegate = LumeWindowDelegate::new(RefCell::new(Some(browser_view)), self.state.clone());
            if window_create_top_level(Some(&mut delegate)).is_none() {
                quit_message_loop();
            }
        }
        fn on_before_child_process_launch(&self, command_line: Option<&mut CommandLine>) {
            if let Some(command_line) = command_line {
                strip_private_switches(command_line);
            }
        }
    }
}

wrap_browser_view_delegate! {
    struct LumeBrowserViewDelegate {}
    impl ViewDelegate {}
    impl BrowserViewDelegate {
        fn browser_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_window_delegate! {
    struct LumeWindowDelegate {
        browser_view: RefCell<Option<BrowserView>>,
        state: Arc<Mutex<BrowserState>>,
    }
    impl ViewDelegate {
        fn preferred_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 1280, height: 820 }
        }
        fn minimum_size(&self, _view: Option<&mut View>) -> Size {
            Size { width: 820, height: 600 }
        }
    }
    impl PanelDelegate {}
    impl WindowDelegate {
        fn on_window_created(&self, window: Option<&mut Window>) {
            super::startup_trace("window_created");
            let view = self.browser_view.borrow();
            if let (Some(window), Some(browser_view)) = (window, view.as_ref()) {
                if let Ok(mut state) = self.state.lock() { state.window = Some(window.clone()); }
                let mut view = View::from(browser_view);
                window.set_to_fill_layout();
                window.add_child_view(Some(&mut view));
                window.set_title(Some(&CefString::from("Lume TV")));
                window.center_window(Some(&Size { width: 1280, height: 820 }));
                window.show();
                window.activate();
                view.request_focus();
            }
        }
        fn on_window_destroyed(&self, _window: Option<&mut Window>) {
            super::startup_trace("window_destroyed");
            if let Ok(mut state) = self.state.lock() { state.window = None; }
            self.browser_view.borrow_mut().take();
        }
        fn can_close(&self, _window: Option<&mut Window>) -> i32 {
            self.browser_view.borrow().as_ref()
                .and_then(|view| view.browser())
                .and_then(|browser| browser.host())
                .map(|host| host.try_close_browser())
                .unwrap_or(1)
        }
        fn can_resize(&self, _window: Option<&mut Window>) -> i32 { 1 }
        fn with_standard_window_buttons(&self, _window: Option<&mut Window>) -> i32 { 1 }
        fn can_maximize(&self, _window: Option<&mut Window>) -> i32 { 1 }
        fn can_minimize(&self, _window: Option<&mut Window>) -> i32 { 1 }
        fn window_runtime_style(&self) -> RuntimeStyle {
            RuntimeStyle::ALLOY
        }
    }
}

wrap_client! {
    struct LumeClient {
        config: Arc<Config>,
        state: Arc<Mutex<BrowserState>>,
    }
    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(LumeLifeSpan::new(self.state.clone()))
        }
        fn request_handler(&self) -> Option<RequestHandler> {
            Some(LumeRequestHandler::new(self.config.origin.clone()))
        }
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(LumeDisplayHandler::new(self.state.clone()))
        }
        fn load_handler(&self) -> Option<LoadHandler> {
            Some(LumeLoadHandler::new())
        }
    }
}

wrap_life_span_handler! {
    struct LumeLifeSpan {
        state: Arc<Mutex<BrowserState>>,
    }
    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            super::startup_trace("browser_created");
            if let Some(host) = browser.as_ref().and_then(|b| b.host()) {
                host.set_accessibility_state(State::ENABLED);
            }
            if let Ok(mut state) = self.state.lock() {
                state.browser = browser.as_deref().cloned();
            }
        }
        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            super::startup_trace("browser_closed");
            if let Ok(mut state) = self.state.lock() {
                state.browser = None;
            }
            quit_message_loop();
        }
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            1
        }
    }
}

wrap_request_handler! {
    struct LumeRequestHandler {
        origin: String,
    }
    impl RequestHandler {
        fn resource_request_handler(
            &self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, _is_navigation: i32, _is_download: i32,
            _request_initiator: Option<&CefString>, _disable_default_handling: Option<&mut i32>,
        ) -> Option<ResourceRequestHandler> {
            super::startup_trace("resource_request");
            Some(LumeResourceRequests::new(self.origin.clone()))
        }
        fn on_before_browse(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            _user_gesture: i32,
            _is_redirect: i32,
        ) -> i32 {
            let Some(request) = request else { return 1; };
            let target = CefString::from(&request.url()).to_string();
            super::startup_trace("before_browse");
            if target == "about:blank" {
                return 0;
            }
            let same_origin = reqwest::Url::parse(&target)
                .map(|url| url.origin().ascii_serialization() == self.origin)
                .unwrap_or(false);
            super::startup_trace(if same_origin { "browse_allowed" } else { "browse_blocked" });
            i32::from(!same_origin)
        }
        fn on_open_urlfrom_tab(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            target_url: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
        ) -> i32 {
            let allowed = target_url
                .and_then(|target| reqwest::Url::parse(&target.to_string()).ok())
                .is_some_and(|url| url.origin().ascii_serialization() == self.origin);
            i32::from(!allowed)
        }
    }
}

wrap_resource_request_handler! {
    struct LumeResourceRequests { origin: String, }
    impl ResourceRequestHandler {
        fn resource_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            request: Option<&mut Request>) -> Option<ResourceHandler> {
            let request = request?;
            let url = reqwest::Url::parse(&CefString::from(&request.url()).to_string()).ok()?;
            if url.origin().ascii_serialization() != self.origin || url.path() == "/" ||
                CefString::from(&request.method()).to_string() != "GET" { return None; }
            let (mime, bytes) = lume_player::desktop::asset(url.path())?;
            Some(LumeBundledAsset::new(mime, bytes, Arc::new(Mutex::new(0))))
        }
        fn on_before_resource_load(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            super::startup_trace("before_resource_load");
            ReturnValue::CONTINUE
        }
        fn on_resource_response(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            _request: Option<&mut Request>, response: Option<&mut Response>) -> i32 {
            if let Some(response) = response {
                super::startup_trace(&format!("resource_response:{}", response.status()));
            }
            0
        }
        fn on_resource_load_complete(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>,
            request: Option<&mut Request>, _response: Option<&mut Response>, status: UrlrequestStatus,
            received_content_length: i64) {
            let path = request.and_then(|r| reqwest::Url::parse(&CefString::from(&r.url()).to_string()).ok())
                .map(|u| match u.path() { "/" => "document", "/app.js" => "app_js", "/app.css" => "app_css",
                    "/player.js" => "player_js", "/vendor/shaka-player.compiled.js" => "shaka_js",
                    "/api/bootstrap" => "bootstrap", _ => "other" }).unwrap_or("unknown");
            super::startup_trace(&format!("resource_complete:{path}:{}:{received_content_length}", status.get_raw()));
        }
    }
}

wrap_resource_handler! {
    struct LumeBundledAsset {
        mime: &'static str,
        bytes: &'static [u8],
        cursor: Arc<Mutex<usize>>,
    }
    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut i32>,
            _callback: Option<&mut Callback>) -> i32 {
            if let Some(handle) = handle_request { *handle = 1; }
            1
        }
        fn response_headers(&self, response: Option<&mut Response>, length: Option<&mut i64>,
            _redirect_url: Option<&mut CefString>) {
            if let Some(response) = response {
                response.set_status(200);
                response.set_mime_type(Some(&CefString::from(self.mime.split(';').next().unwrap_or(self.mime))));
                response.set_charset(Some(&CefString::from("utf-8")));
                response.set_header_by_name(Some(&CefString::from("Cache-Control")), Some(&CefString::from("no-store")), 1);
                response.set_header_by_name(Some(&CefString::from("X-Content-Type-Options")), Some(&CefString::from("nosniff")), 1);
            }
            if let Some(length) = length { *length = self.bytes.len() as i64; }
        }
        fn read(&self, data_out: *mut u8, bytes_to_read: i32,
            bytes_read: Option<&mut i32>, _callback: Option<&mut ResourceReadCallback>) -> i32 {
            let Some(bytes_read) = bytes_read else { return 0; };
            *bytes_read = 0;
            if data_out.is_null() || bytes_to_read <= 0 { return 0; }
            let mut cursor = self.cursor.lock().expect("asset read lock");
            let count = (bytes_to_read as usize).min(self.bytes.len().saturating_sub(*cursor));
            if count == 0 { return 0; }
            // CEF provides a writable buffer of bytes_to_read bytes for this call.
            unsafe { std::ptr::copy_nonoverlapping(self.bytes.as_ptr().add(*cursor), data_out.cast::<u8>(), count); }
            *cursor += count;
            *bytes_read = count as i32;
            1
        }
    }
}

wrap_load_handler! {
    struct LumeLoadHandler;
    impl LoadHandler {
        fn on_load_end(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, http_status_code: i32) {
            super::startup_trace(&format!("load_end:{http_status_code}"));
        }
        fn on_load_error(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, error_code: Errorcode,
            _error_text: Option<&CefString>, _failed_url: Option<&CefString>) {
            super::startup_trace(&format!("load_error:{}", error_code.get_raw()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_asset_reads_are_bounded_and_end_without_touching_other_bytes() {
        let handler = LumeBundledAsset::new("text/javascript", b"abcd", Arc::new(Mutex::new(0)));
        let mut output = [0xaa; 6];
        let mut count = 0;
        assert_eq!(
            handler.read(output[1..].as_mut_ptr(), 2, Some(&mut count), None),
            1
        );
        assert_eq!(count, 2);
        assert_eq!(output, [0xaa, b'a', b'b', 0xaa, 0xaa, 0xaa]);
        assert_eq!(
            handler.read(output[1..].as_mut_ptr(), 5, Some(&mut count), None),
            1
        );
        assert_eq!(count, 2);
        assert_eq!(output, [0xaa, b'c', b'd', 0xaa, 0xaa, 0xaa]);
        assert_eq!(
            handler.read(output[1..].as_mut_ptr(), 5, Some(&mut count), None),
            0
        );
        assert_eq!(count, 0);
    }
}

wrap_display_handler! {
    struct LumeDisplayHandler { state: Arc<Mutex<BrowserState>>, }
    impl DisplayHandler {
        fn on_fullscreen_mode_change(&self, _browser: Option<&mut Browser>, fullscreen: i32) {
            let window = self.state.lock().ok().and_then(|state| state.window.clone());
            if let Some(window) = window { window.set_fullscreen(fullscreen); }
        }
        fn on_console_message(
            &self,
            _browser: Option<&mut Browser>,
            level: LogSeverity,
            _message: Option<&CefString>,
            _source: Option<&CefString>,
            line: i32,
        ) -> i32 {
            super::startup_trace(&format!("console:{}:{line}", level.get_raw()));
            // Provider URLs/tokens in Chromium console messages stay private.
            1
        }
    }
}
