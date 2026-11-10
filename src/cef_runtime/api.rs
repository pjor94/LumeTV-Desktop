// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{bail, Result};
use std::ffi::CStr;

/// cef-rs 150.2.1 generated both its Rust bindings and C++ wrapper against the
/// experimental ABI. The 156 wrapper explicitly selects LAST instead.
pub const fn selected_version() -> i32 {
    if cef::sys::CEF_VERSION_MAJOR == 150 {
        cef::sys::CEF_API_VERSION_EXPERIMENTAL
    } else {
        cef::sys::CEF_API_VERSION_LAST
    }
}

/// Select the same ABI in every CEF process, before any versioned API is used.
pub fn initialize_api() -> Result<()> {
    let hash = cef::api_hash(selected_version(), 0);
    if hash.is_null() {
        bail!("The embedded CEF runtime does not provide the required ABI");
    }

    // SAFETY: cef_api_hash returns a NUL-terminated static string when non-null.
    let actual = unsafe { CStr::from_ptr(hash) };
    #[cfg(target_os = "macos")]
    if cef::sys::CEF_VERSION_MAJOR == 150
        && actual.to_bytes() != b"0934859000c911cdd75907f63f2ef8e485706e5b"
    {
        // Independently verified identical for the original 150.0.14 headers
        // and OBS's 150.0.19 SDK. Stable 15000 is a different layout.
        bail!("The embedded CEF 150 experimental ABI does not match the audited macOS SDK");
    }
    #[cfg(not(target_os = "macos"))]
    let _ = actual;
    Ok(())
}
