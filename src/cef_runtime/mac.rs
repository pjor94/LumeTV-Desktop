// SPDX-License-Identifier: GPL-3.0-or-later
// Cocoa lifecycle follows tauri-apps/cef-rs's cefsimple example (MIT/Apache-2.0).
use super::app::{close_browser, BrowserState};
use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    define_class, extern_methods, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool, NSObjectProtocol},
    sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSEvent, NSMenu, NSMenuItem};
use objc2_foundation::NSString;
use std::{
    cell::{Cell, RefCell},
    sync::{Arc, Mutex},
};

thread_local! {
    static BROWSER_STATE: RefCell<Option<Arc<Mutex<BrowserState>>>> = const { RefCell::new(None) };
}

#[derive(Default)]
pub struct ApplicationIvars {
    handling_send_event: Cell<Bool>,
}

define_class!(
    #[unsafe(super(NSApplication))]
    #[ivars = ApplicationIvars]
    pub struct LumeApplication;

    impl LumeApplication {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was_handling = self.ivars().handling_send_event.get();
            self.ivars().handling_send_event.set(Bool::YES);
            let _: () = msg_send![super(self), sendEvent: event];
            self.ivars().handling_send_event.set(was_handling);
        }

        #[unsafe(method(terminate:))]
        unsafe fn terminate(&self, _sender: Option<&AnyObject>) {
            super::startup_trace("quit_requested");
            BROWSER_STATE.with(|state| {
                if let Some(state) = state.borrow().as_ref() {
                    close_browser(state);
                }
            });
        }
    }

    unsafe impl NSObjectProtocol for LumeApplication {}
    unsafe impl CrAppProtocol for LumeApplication {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn is_handling_send_event(&self) -> Bool {
            self.ivars().handling_send_event.get()
        }
    }
    unsafe impl CrAppControlProtocol for LumeApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn set_handling_send_event(&self, value: Bool) {
            self.ivars().handling_send_event.set(value);
        }
    }
    unsafe impl CefAppProtocol for LumeApplication {}
);

impl LumeApplication {
    extern_methods!(
        #[unsafe(method(sharedApplication))]
        fn shared_application() -> Retained<Self>;
    );
}

pub(super) fn setup(state: Arc<Mutex<BrowserState>>) -> Retained<LumeApplication> {
    let marker = MainThreadMarker::new().expect("CEF must initialize on the main thread");
    BROWSER_STATE.with(|slot| *slot.borrow_mut() = Some(state));
    // Call our subclass first; NSApplication must not be initialized earlier.
    let app = LumeApplication::shared_application();
    assert!(NSApplication::sharedApplication(marker).isKindOfClass(LumeApplication::class()));
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app
}

pub(super) fn install_menu(app: &LumeApplication) {
    let marker = MainThreadMarker::new().expect("Menus require the main thread");
    let menu = NSMenu::initWithTitle(NSMenu::alloc(marker), &NSString::from_str("Lume TV"));
    let sections = [
        (
            "Lume TV",
            vec![
                (
                    "Informazioni su Lume TV",
                    sel!(orderFrontStandardAboutPanel:),
                    "",
                ),
                ("Esci da Lume TV", sel!(terminate:), "q"),
            ],
        ),
        (
            "Modifica",
            vec![
                ("Taglia", sel!(cut:), "x"),
                ("Copia", sel!(copy:), "c"),
                ("Incolla", sel!(paste:), "v"),
                ("Seleziona tutto", sel!(selectAll:), "a"),
            ],
        ),
        (
            "Finestra",
            vec![
                ("Riduci a icona", sel!(performMiniaturize:), "m"),
                ("Chiudi", sel!(performClose:), "w"),
            ],
        ),
    ];
    for (title, actions) in sections {
        // The heading has no action; it only owns its submenu.
        let heading = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(marker),
                &NSString::from_str(title),
                None,
                &NSString::from_str(""),
            )
        };
        let submenu = NSMenu::initWithTitle(NSMenu::alloc(marker), &NSString::from_str(title));
        for (label, action, shortcut) in actions {
            // All selectors are standard AppKit responder-chain actions.
            let item = unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(marker),
                    &NSString::from_str(label),
                    Some(action),
                    &NSString::from_str(shortcut),
                )
            };
            submenu.addItem(&item);
        }
        heading.setSubmenu(Some(&submenu));
        menu.addItem(&heading);
    }
    app.setMainMenu(Some(&menu));
}
