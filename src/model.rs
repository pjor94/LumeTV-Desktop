// SPDX-License-Identifier: GPL-3.0-or-later
// Models adapted from Lume TV's Kotlin core (2026).

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Serialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(skip_serializing)]
    pub key: String,
    #[serde(skip_serializing)]
    pub headers: BTreeMap<String, String>,
}

// Deliberately omit Debug and Serialize: licenses and headers can carry session tokens.
#[derive(Clone)]
pub struct DrmSpec {
    pub license_url: String,
    pub headers: BTreeMap<String, String>,
}

// This value must remain server-side; use an explicitly redacted view in diagnostics.
#[derive(Clone)]
pub struct ResolvedStream {
    pub url: String,
    pub mime_type: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub drm: Option<DrmSpec>,
}
