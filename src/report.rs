// SPDX-License-Identifier: GPL-3.0-or-later
use crate::model::Channel;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunReport {
    pub schema_version: u32,
    pub run_id: String,
    pub started_unix_ms: u64,
    pub os: String,
    pub arch: String,
    pub operation: String,
    pub verdict: String,
    pub channel: Option<Value>,
    pub runtime: Value,
    pub resolver: Value,
    pub manifest: Value,
    pub playback: Value,
    pub events: Vec<Value>,
    pub limitations: Vec<String>,
}

impl RunReport {
    pub fn new(operation: &str, channel: Option<&Channel>) -> Self {
        Self {
            schema_version: 1,
            run_id: uuid::Uuid::new_v4().to_string(),
            started_unix_ms: now_ms(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            operation: operation.into(),
            verdict: "not_tested".into(),
            channel: channel.map(|c| serde_json::to_value(c).unwrap_or(Value::Null)),
            runtime: json!({"backend":"not_applicable", "embedded_runtime_verified":false, "distribution_verified":false}),
            resolver: json!({"status":"not_tested"}),
            manifest: json!({"status":"not_tested"}),
            playback: json!({"status":"not_tested", "audio_audibility":"not_verified"}),
            events: vec![],
            limitations: vec![
                "Runtime/CDM availability and distribution require independent verification."
                    .into(),
                "Results apply only to this OS, browser, network, channel and time of observation."
                    .into(),
                "Muted automatic playback does not verify audible output.".into(),
            ],
        }
    }

    pub fn set_runtime(&mut self, metadata: Value) {
        self.runtime = metadata;
        let limitation = match self.runtime["backend"].as_str() {
            Some("embedded_cef") => "Embedded CEF execution; codec/CDM availability is measured separately and does not establish distribution rights or packaging readiness.",
            Some("installed_browser") => "Installed browser/CDM test; does not validate a distributable embedded CEF/CDM runtime.",
            _ => "Runtime/CDM availability and distribution require independent verification.",
        };
        self.limitations[0] = limitation.into();
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_json(path, self)
    }
}

pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).context("Cannot create report directory")?;
    }
    // Reports only contain allowlisted diagnostics, never stream URLs or credentials.
    fs::write(path, serde_json::to_vec_pretty(value)?).context("Cannot write report")
}

/// Only structured diagnostics cross the report boundary. Browser exception messages,
/// URL fields and arbitrary nested objects are intentionally excluded.
pub fn sanitize_event(input: &Value) -> Option<Value> {
    let object = input.as_object()?;
    let kind = object.get("kind")?.as_str()?;
    if ![
        "capabilities",
        "drm_setup",
        "loaded",
        "playing",
        "sample",
        "error",
        "complete",
        "encrypted",
        "key_status",
        "key_statuses",
        "audio_verified",
        "audio_muted_changed",
        "buffering",
        "started",
        "autoplay_blocked",
    ]
    .contains(&kind)
    {
        return None;
    }
    let mut result = Map::new();
    result.insert("kind".into(), json!(kind));
    // Never accept a browser-provided clock as the independent observation clock.
    result.insert("received_unix_ms".into(), json!(now_ms()));
    for (key, value) in object {
        let finite_nonnegative = value.as_f64().is_some_and(|n| n.is_finite() && n >= 0.0);
        if ["error", "autoplay_blocked"].contains(&kind) {
            if ["code", "category", "severity"].contains(&key.as_str()) && finite_nonnegative {
                result.insert(key.clone(), value.clone());
            }
            continue;
        }
        let numeric = [
            "current_time",
            "video_frames",
            "decoded_video_frames",
            "dropped_frames",
            "ready_state",
            "width",
            "height",
            "audio_decoded_bytes",
            "decoded_audio_bytes",
            "code",
            "category",
            "severity",
            "eme_error_code",
            "webm_error_code",
            "elapsed_seconds",
            "max_wait_seconds",
            "playback_seconds",
            "observed_seconds",
            "encrypted_events",
            "license_requests",
            "frame_callbacks",
            "audio_frames",
            "timestamp_ms",
            "video_frames_delta",
            "media_time_delta",
            "buffering_seconds",
            "moving_seconds",
            "required_seconds",
            "samples",
        ];
        let boolean = [
            "widevine",
            "widevine_available",
            "widevine_webm_available",
            "widevine_access",
            "eme",
            "eme_available",
            "mse",
            "mse_available",
            "secure_context",
            "shaka_supported",
            "avc",
            "aac",
            "avc_mse_supported",
            "aac_mse_supported",
            "webm_mse_supported",
            "native_avc_supported",
            "native_aac_supported",
            "muted",
            "paused",
            "seeking",
            "encrypted",
            "usable_key",
            "key_status_usable",
            "success",
            "video_verified",
            "audio_verified",
            "audio_audibly_verified",
            "audio_decoded",
            "audio_decode_metric_available",
            "drm_configured",
            "headless",
            "playing",
            "buffering",
        ];
        if (numeric.contains(&key.as_str()) && finite_nonnegative)
            || (boolean.contains(&key.as_str()) && value.is_boolean())
        {
            result.insert(key.clone(), value.clone());
        } else if ["status", "reason", "name", "stage"].contains(&key.as_str()) {
            if let Some(s) = value.as_str().filter(|s| {
                [
                    "available",
                    "waiting",
                    "ready",
                    "unavailable",
                    "unsupported",
                    "usable",
                    "expired",
                    "released",
                    "output-restricted",
                    "output-downscaled",
                    "status-pending",
                    "internal-error",
                    "NotSupportedError",
                    "NotAllowedError",
                    "SecurityError",
                    "UnknownError",
                    "AbortError",
                    "Error",
                    "autoplay",
                    "media",
                    "drm",
                    "network",
                    "timeout",
                    "complete",
                    "passed",
                    "failed",
                    "stalled",
                    "load",
                    "play",
                    "capabilities",
                    "session",
                    "shaka_load",
                    "eme_access",
                    "insufficient_playback",
                    "unsupported_browser",
                    "playback_observed",
                    "player_error",
                    "browser_unsupported",
                    "widevine_unavailable",
                    "widevine_setup_timeout",
                    "codec_unavailable",
                ]
                .contains(s)
            }) {
                result.insert(key.clone(), json!(s));
            }
        } else if key == "video_frame_source" {
            if let Some(s) = value.as_str().filter(|s| {
                [
                    "playback_quality",
                    "webkit_decoded",
                    "video_frame_callback",
                    "unavailable",
                ]
                .contains(s)
            }) {
                result.insert(key.clone(), json!(s));
            }
        } else if [
            "requested_video_codec",
            "requested_audio_codec",
            "requested_webm_video_codec",
            "requested_webm_audio_codec",
        ]
        .contains(&key.as_str())
        {
            let expected = match key.as_str() {
                "requested_video_codec" => "video/mp4; codecs=\"avc1.640028\"",
                "requested_audio_codec" => "audio/mp4; codecs=\"mp4a.40.2\"",
                "requested_webm_video_codec" => "video/webm; codecs=\"vp9\"",
                _ => "audio/webm; codecs=\"opus\"",
            };
            if value.as_str() == Some(expected) {
                result.insert(key.clone(), json!(expected));
            }
        } else if key == "key_statuses" || key == "statuses" {
            let known = [
                "usable",
                "expired",
                "released",
                "output-restricted",
                "output-downscaled",
                "status-pending",
                "internal-error",
            ];
            let mut statuses: Vec<&str> = match value {
                Value::Array(a) => a
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|s| known.contains(s))
                    .take(16)
                    .collect(),
                Value::Object(o) => o
                    .values()
                    .filter_map(Value::as_str)
                    .filter(|s| known.contains(s))
                    .take(16)
                    .collect(),
                _ => vec![],
            };
            statuses.sort_unstable();
            statuses.dedup();
            result.insert("key_statuses".into(), json!(statuses));
        }
    }
    Some(Value::Object(result))
}

/// Check received samples independently of the browser's own success flag.
pub fn assess_playback(
    events: &[Value],
    required_seconds: u64,
    drm: bool,
    license_ok: bool,
) -> Value {
    struct Observation {
        current: f64,
        at: u64,
        frames: u64,
        seeking: bool,
    }
    let mut previous: Option<Observation> = None;
    let mut progress = 0.0_f64;
    let mut valid_elapsed = 0.0_f64;
    let mut max_frames = 0_u64;
    let mut progressing_frames = 0_u64;
    let mut video_dimensions = false;
    let mut usable_key = false;
    let mut encrypted = false;
    let mut audio_bytes = 0_u64;
    let mut audible = false;
    for event in events {
        let kind = event["kind"].as_str().unwrap_or_default();
        if kind == "audio_verified" {
            audible |= event["audio_audibly_verified"].as_bool() == Some(true);
        }
        // A configured DRM object or a copied summary flag is not an encrypted event.
        encrypted |= kind == "encrypted";
        usable_key |= ["key_status", "key_statuses", "sample"].contains(&kind)
            && event["key_statuses"]
                .as_array()
                .is_some_and(|s| s.iter().any(|v| v == "usable"));
        audio_bytes = audio_bytes.max(
            event["audio_decoded_bytes"]
                .as_u64()
                .or(event["decoded_audio_bytes"].as_u64())
                .unwrap_or(0),
        );
        if kind != "sample" {
            continue;
        }
        let frames = event["video_frames"]
            .as_u64()
            .or(event["decoded_video_frames"].as_u64())
            .unwrap_or(0);
        max_frames = max_frames.max(frames);
        let dimensions =
            event["width"].as_u64().unwrap_or(0) > 0 && event["height"].as_u64().unwrap_or(0) > 0;
        video_dimensions |= dimensions;
        if let (Some(current), Some(at)) = (
            event["current_time"]
                .as_f64()
                .filter(|n| n.is_finite() && *n >= 0.0),
            event["received_unix_ms"].as_u64(),
        ) {
            let playing = event["paused"].as_bool() == Some(false)
                && event["seeking"].as_bool() != Some(true)
                && event["ready_state"].as_u64().unwrap_or(0) >= 2;
            if let Some(old) = previous {
                let wall = at.saturating_sub(old.at) as f64 / 1000.0;
                let delta = current - old.current;
                if wall > 0.0
                    && wall <= 5.0
                    && delta > 0.04
                    && delta <= wall * 2.0 + 0.5
                    && playing
                    && !old.seeking
                    && dimensions
                {
                    // Each credited second is bounded by the independent server clock.
                    progress += delta.min(wall);
                    valid_elapsed += wall;
                    progressing_frames =
                        progressing_frames.saturating_add(frames.saturating_sub(old.frames));
                }
            }
            previous = Some(Observation {
                current,
                at,
                frames,
                seeking: event["seeking"].as_bool() == Some(true),
            });
        } else {
            previous = None;
        }
    }
    let video_ok = required_seconds > 0
        && progress >= required_seconds as f64
        && valid_elapsed >= required_seconds as f64
        && progressing_frames > 0
        && video_dimensions;
    let drm_ok = !drm || (license_ok && encrypted && usable_key);
    json!({
        "status":if video_ok && drm_ok {"verified"} else {"not_verified"},
        "required_seconds":required_seconds, "progress_seconds":(progress*1000.0).round()/1000.0,
        "elapsed_server_seconds":(valid_elapsed*1000.0).round()/1000.0,
        "decoded_video_frames_delta":progressing_frames, "video_frames_observed":max_frames,
        "video_dimensions_observed":video_dimensions,
        "drm_required":drm, "license_http_success":license_ok, "encrypted_event_observed":encrypted,
        "usable_key_observed":usable_key, "drm_verified":drm && drm_ok && video_ok,
        "audio_decoded_bytes":audio_bytes,
        "audio_audibility":if audible {"manually_verified"} else {"not_verified"},
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_setup_events_keep_only_allowlisted_status_and_timing() {
        let event = sanitize_event(&json!({"kind":"drm_setup", "status":"waiting", "max_wait_seconds":120, "elapsed_seconds":2.5, "url":"https://SECRET", "token":"SECRET", "message":"SECRET"})).unwrap();
        assert_eq!(event["status"], "waiting");
        assert_eq!(event["max_wait_seconds"], 120);
        assert_eq!(event["elapsed_seconds"], 2.5);
        assert!(!event.to_string().contains("SECRET"));
        let invalid =
            sanitize_event(&json!({"kind":"drm_setup", "status":"SECRET", "max_wait_seconds":-1}))
                .unwrap();
        assert!(invalid.get("status").is_none());
        assert!(invalid.get("max_wait_seconds").is_none());
    }

    #[test]
    fn reports_identify_selected_runtime_without_claiming_embedded_distribution() {
        let mut report = RunReport::new("probe", None);
        assert_eq!(report.runtime["backend"], "not_applicable");
        report.set_runtime(json!({"backend":"embedded_cef", "embedded_runtime_verified":false, "distribution_verified":false}));
        assert_eq!(report.runtime["backend"], "embedded_cef");
        assert_eq!(report.runtime["embedded_runtime_verified"], false);
        assert_eq!(report.runtime["distribution_verified"], false);
        assert!(report.limitations[0].starts_with("Embedded CEF execution"));
        report
            .set_runtime(json!({"backend":"installed_browser", "embedded_runtime_verified":false}));
        assert!(report.limitations[0].starts_with("Installed browser/CDM test"));
    }

    #[test]
    fn diagnostics_cannot_include_url_tokens_or_arbitrary_error_data() {
        let v = sanitize_event(&json!({"kind":"error","code":6007,"category":6,"severity":2,"message":"SECRET","url":"https://x/?token=SECRET","data":["SECRET"],"reason":"SECRET","headers":{"authorization":"SECRET"},"key_statuses":{"SECRET":"usable"}})).unwrap();
        assert!(!v.to_string().contains("SECRET"));
        assert_eq!(v["code"], 6007);
        assert_eq!(v.as_object().unwrap().len(), 5); // kind, server clock, 3 numeric codes
        assert!(sanitize_event(&json!({"kind":"SECRET"})).is_none());
    }

    #[test]
    fn capabilities_match_harness_and_only_known_codec_strings_survive() {
        let v = sanitize_event(&json!({"kind":"capabilities","secure_context":true,"eme_available":true,"widevine_available":true,"mse_available":true,"avc_mse_supported":true,"aac_mse_supported":true,"native_avc_supported":true,"native_aac_supported":true,"shaka_supported":true,"requested_video_codec":"video/mp4; codecs=\"avc1.640028\"","requested_audio_codec":"audio/mp4; codecs=\"mp4a.40.2\"","eme_error_code":9,"token":"SECRET","url":"https://SECRET","received_unix_ms":1})).unwrap();
        for key in [
            "secure_context",
            "eme_available",
            "widevine_available",
            "mse_available",
            "avc_mse_supported",
            "aac_mse_supported",
            "native_avc_supported",
            "native_aac_supported",
            "shaka_supported",
        ] {
            assert_eq!(v[key], true, "missing capability {key}");
        }
        assert_eq!(
            v["requested_video_codec"],
            "video/mp4; codecs=\"avc1.640028\""
        );
        assert_eq!(
            v["requested_audio_codec"],
            "audio/mp4; codecs=\"mp4a.40.2\""
        );
        assert_eq!(v["eme_error_code"], 9);
        assert_ne!(v["received_unix_ms"], 1);
        assert!(!v.to_string().contains("SECRET"));
        let bad = sanitize_event(&json!({"kind":"capabilities", "requested_video_codec":"https://SECRET", "requested_audio_codec":"token=SECRET"})).unwrap();
        assert!(bad.get("requested_video_codec").is_none());
        assert!(bad.get("requested_audio_codec").is_none());
    }

    #[test]
    fn webm_eme_diagnostics_preserve_codec_distinction_without_accepting_arbitrary_strings() {
        let event = sanitize_event(&json!({"kind":"capabilities", "avc_mse_supported":false, "aac_mse_supported":false, "widevine_available":false, "widevine_webm_available":true, "webm_mse_supported":true, "webm_error_code":9, "requested_webm_video_codec":"video/webm; codecs=\"vp9\"", "requested_webm_audio_codec":"audio/webm; codecs=\"opus\"", "token":"SECRET"})).unwrap();
        assert_eq!(event["widevine_available"], false);
        assert_eq!(event["widevine_webm_available"], true);
        assert_eq!(event["webm_mse_supported"], true);
        assert_eq!(event["webm_error_code"], 9);
        assert_eq!(
            event["requested_webm_video_codec"],
            "video/webm; codecs=\"vp9\""
        );
        assert_eq!(
            event["requested_webm_audio_codec"],
            "audio/webm; codecs=\"opus\""
        );
        assert!(!event.to_string().contains("SECRET"));
        let bad = sanitize_event(&json!({"kind":"capabilities", "requested_webm_video_codec":"https://SECRET", "requested_webm_audio_codec":"token=SECRET"})).unwrap();
        assert!(bad.get("requested_webm_video_codec").is_none());
        assert!(bad.get("requested_webm_audio_codec").is_none());
    }

    #[test]
    fn key_status_event_retains_values_without_key_ids_or_unknown_strings() {
        let v = sanitize_event(&json!({"kind":"key_statuses","key_statuses":{"SECRET-KEY-ID":"usable","second":"usable","third":"expired","fourth":"https://SECRET"}})).unwrap();
        assert_eq!(v["key_statuses"], json!(["expired", "usable"]));
        assert!(!v.to_string().contains("SECRET"));
    }

    fn samples() -> Vec<Value> {
        (0..=3).map(|i| json!({"kind":"sample","received_unix_ms":i*1000,"current_time":i as f64,"video_frames":i*25,"width":1280,"height":720,"ready_state":4,"paused":false,"seeking":false})).collect()
    }

    #[test]
    fn manifest_or_browser_success_flag_never_proves_playback() {
        let r = assess_playback(
            &[
                json!({"kind":"loaded"}),
                json!({"kind":"complete","success":true,"moving_seconds":600,"video_frames":1000}),
            ],
            2,
            false,
            false,
        );
        assert_eq!(r["status"], "not_verified");
    }

    #[test]
    fn valid_progress_is_measured_against_server_time_with_frame_deltas() {
        let r = assess_playback(&samples(), 3, false, false);
        assert_eq!(r["status"], "verified");
        assert_eq!(r["progress_seconds"], 3.0);
        assert_eq!(r["elapsed_server_seconds"], 3.0);
        assert_eq!(r["decoded_video_frames_delta"], 75);
    }

    #[test]
    fn accelerated_media_time_does_not_create_unobserved_elapsed_seconds() {
        let mut e = samples();
        for event in &mut e {
            event["current_time"] = json!(event["current_time"].as_f64().unwrap() * 2.0);
        }
        assert_eq!(
            assess_playback(&e, 4, false, false)["status"],
            "not_verified"
        );
        assert_eq!(
            assess_playback(&e, 4, false, false)["progress_seconds"],
            3.0
        );
    }

    #[test]
    fn first_actual_media_progress_after_loading_is_credited_only_for_its_delta() {
        let mut e = samples();
        e[0]["paused"] = json!(true);
        e[0]["ready_state"] = json!(0);
        e[0]["width"] = json!(0);
        e[0]["height"] = json!(0);
        for sample in &mut e[1..] {
            sample["current_time"] = json!(sample["current_time"].as_f64().unwrap() - 0.5);
        }
        let r = assess_playback(&e, 2, false, false);
        assert_eq!(r["status"], "verified");
        assert_eq!(r["progress_seconds"], 2.5);
    }

    #[test]
    fn protected_video_requires_explicit_encrypted_event_usable_key_and_license_http_success() {
        let mut e = samples();
        e.push(json!({"kind":"complete", "encrypted":true,"encrypted_events":1,"key_statuses":["usable"]}));
        assert_eq!(assess_playback(&e, 2, true, true)["status"], "not_verified");
        e.push(json!({"kind":"encrypted", "encrypted":true}));
        assert_eq!(assess_playback(&e, 2, true, true)["status"], "not_verified");
        e.push(sanitize_event(&json!({"kind":"key_statuses","key_statuses":["usable"]})).unwrap());
        assert_eq!(assess_playback(&e, 2, true, true)["status"], "verified");
        assert_eq!(
            assess_playback(&e, 2, true, false)["status"],
            "not_verified"
        );
    }

    #[test]
    fn seek_pause_stall_and_long_sampling_gaps_do_not_count_as_watching() {
        let mut jump = samples();
        jump[1]["current_time"] = json!(50000);
        assert_eq!(
            assess_playback(&jump, 3, false, false)["status"],
            "not_verified"
        );
        for flag in ["seeking", "paused"] {
            let mut e = samples();
            e[1][flag] = json!(true);
            assert_eq!(
                assess_playback(&e, 3, false, false)["status"],
                "not_verified",
                "flag {flag}"
            );
        }
        let mut stalled = samples();
        for e in &mut stalled {
            e["current_time"] = json!(0);
        }
        assert_eq!(
            assess_playback(&stalled, 2, false, false)["status"],
            "not_verified"
        );
        let mut gap = samples();
        for e in &mut gap {
            e["received_unix_ms"] = json!(e["received_unix_ms"].as_u64().unwrap() * 10);
        }
        assert_eq!(
            assess_playback(&gap, 2, false, false)["status"],
            "not_verified"
        );
    }

    #[test]
    fn a_single_decoded_frame_with_audio_time_progress_does_not_verify_video() {
        let mut e = samples();
        for sample in &mut e {
            sample["video_frames"] = json!(25);
        }
        let r = assess_playback(&e, 2, false, false);
        assert_eq!(r["status"], "not_verified");
        assert_eq!(r["decoded_video_frames_delta"], 0);
    }

    #[test]
    fn decoded_audio_and_summary_flags_never_prove_audible_output() {
        let mut e = samples();
        e.push(sanitize_event(&json!({"kind":"sample","audio_decoded_bytes":1234,"audio_decode_metric_available":true,"audio_audibly_verified":true})).unwrap());
        e.push(sanitize_event(&json!({"kind":"complete","audio_audibly_verified":true})).unwrap());
        e.push(sanitize_event(&json!({"kind":"audio_verified","audio_verified":true})).unwrap());
        let r = assess_playback(&e, 2, false, false);
        assert_eq!(r["audio_decoded_bytes"], 1234);
        assert_eq!(r["audio_audibility"], "not_verified");
        let verified = sanitize_event(
            &json!({"kind":"audio_verified","audio_audibly_verified":true,"token":"SECRET"}),
        )
        .unwrap();
        assert_eq!(verified["audio_audibly_verified"], true);
        assert!(!verified.to_string().contains("SECRET"));
        e.push(verified);
        assert_eq!(
            assess_playback(&e, 2, false, false)["audio_audibility"],
            "manually_verified"
        );
    }
}
