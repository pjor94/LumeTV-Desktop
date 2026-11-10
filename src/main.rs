// SPDX-License-Identifier: GPL-3.0-or-later
use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lume_player::{
    desktop::{default_data_directory, DesktopServer},
    model::{Channel, ResolvedStream},
    providers::{channels, probe_manifest, Resolver},
    report::{assess_playback, write_json, RunReport},
    runtime::{Backend, LaunchOptions, RuntimeSelection},
    server::HarnessServer,
};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::ExitCode,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    name = "lume-player",
    version,
    about = "Lume TV: resolver, Widevine e riproduzione desktop"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Open the complete Lume TV desktop application.
    App {
        #[command(flatten)]
        runtime: RuntimeOptions,
        /// Application data directory: favorites, preferences, guide and browser profile.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Development preview: serve the UI without opening a window.
        #[arg(long)]
        serve_only: bool,
    },
    /// Inspect local runtime availability. Does not prove EME or playback.
    Doctor {
        #[command(flatten)]
        runtime: RuntimeOptions,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// List the complete bundled catalogue.
    Channels,
    /// Resolve a channel and read its manifest, without a DRM license request.
    Probe {
        #[arg(long)]
        channel: String,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Launch the embedded CEF player, or explicitly select a browser diagnostic.
    Play {
        #[arg(long)]
        channel: String,
        #[command(flatten)]
        options: PlayOptions,
        #[arg(long)]
        report: Option<PathBuf>,
    },
    /// Run a sequential channel matrix on the current operating system.
    Matrix {
        #[arg(long, value_delimiter = ',', default_value = "rai1,canale5,la7")]
        channels: Vec<String>,
        #[command(flatten)]
        options: PlayOptions,
        #[arg(long)]
        report: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    Relay,
    Direct,
}
impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Relay => "relay",
            Self::Direct => "direct",
        }
    }
}

#[derive(Args)]
struct RuntimeOptions {
    /// CEF is the application runtime; browser is an explicit external diagnostic.
    #[arg(long, value_enum, default_value_t = Backend::Cef)]
    runtime: Backend,
    /// Installed browser executable; requires --runtime browser.
    #[arg(long)]
    browser: Option<PathBuf>,
    /// Embedded Rust CEF companion executable; no external browser fallback.
    #[arg(long)]
    cef_executable: Option<PathBuf>,
    /// Optional external CDM source; its presence does not prove CDM availability.
    #[arg(long)]
    widevine_cdm: Option<PathBuf>,
}

impl RuntimeOptions {
    fn selection(&self) -> RuntimeSelection {
        RuntimeSelection {
            backend: self.runtime,
            browser: self.browser.clone(),
            cef_executable: self.cef_executable.clone(),
            widevine_cdm: self.widevine_cdm.clone(),
        }
    }
}

#[derive(Args)]
struct PlayOptions {
    #[arg(long, default_value_t=30, value_parser=clap::value_parser!(u64).range(5..=600))]
    seconds: u64,
    /// Wait for embedded CEF's first-run Widevine setup before protected playback.
    #[arg(long, default_value_t=300, value_parser=clap::value_parser!(u64).range(0..=300))]
    drm_setup_seconds: u64,
    #[command(flatten)]
    runtime: RuntimeOptions,
    /// Dedicated profile; defaults differ for CEF and the external browser.
    #[arg(long)]
    profile: Option<PathBuf>,
    /// External browser diagnostic only; embedded CEF currently requires a window.
    #[arg(long)]
    headless: bool,
    #[arg(long,value_enum,default_value_t=Mode::Relay)]
    mode: Mode,
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(2),
        Err(error) => {
            eprintln!("Errore: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<bool> {
    let cli = Cli::parse();
    let running = Arc::new(AtomicBool::new(true));
    let signal = running.clone();
    ctrlc::set_handler(move || signal.store(false, Ordering::Relaxed))
        .context("Cannot set interrupt handler")?;
    match cli.command {
        Commands::App {
            runtime,
            profile,
            serve_only,
        } => {
            let directory = profile.unwrap_or_else(default_data_directory);
            let server = DesktopServer::start(&directory)?;
            let browser_profile = directory.join("browser");
            prepare_profile(&browser_profile)?;
            let mut child = if serve_only {
                println!("Anteprima locale Lume TV: {}", server.launch_url);
                None
            } else {
                let selection = runtime.selection();
                let spec = selection.detect()?;
                Some(spec.launch(LaunchOptions {
                    profile: &browser_profile,
                    url: &server.launch_url,
                    headless: false,
                })?)
            };
            while running.load(Ordering::Relaxed) {
                if let Some(process) = child.as_mut() {
                    if process.try_wait()?.is_some() {
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(150));
            }
            if let Some(process) = child.as_mut() {
                process.shutdown();
            }
            Ok(true)
        }
        Commands::Channels => {
            for channel in channels() {
                println!(
                    "{:<12} {:<10} {}",
                    channel.id, channel.provider, channel.name
                );
            }
            Ok(true)
        }
        Commands::Doctor {
            runtime,
            report: path,
        } => {
            let selection = runtime.selection();
            selection.validate(false)?;
            let mut report = RunReport::new("doctor", None);
            report.set_runtime(selection.report_metadata());
            let path = path.unwrap_or_else(|| {
                default_report(Some(selection.backend), &report.run_id, "doctor")
            });
            match selection.detect() {
                Ok(spec) => {
                    report.verdict = "runtime_found".into();
                    report.set_runtime(spec.report_metadata());
                }
                Err(_) => {
                    report.verdict = "runtime_missing".into();
                    report.runtime["error"] = json!(selection.missing_reason());
                }
            }
            report.save(&path)?;
            println!(
                "{} {}: {} ({}). Widevine e playback: non verificati. Report: {}",
                report.os,
                report.arch,
                report.verdict,
                selection.backend.name(),
                path.display()
            );
            Ok(report.verdict == "runtime_found")
        }
        Commands::Probe {
            channel,
            report: path,
        } => {
            let channel = find_channel(&channel)?;
            let mut report = RunReport::new("probe", Some(&channel));
            let path = path.unwrap_or_else(|| {
                default_report(None, &report.run_id, &format!("{}-probe", channel.id))
            });
            let resolver = Resolver::new()?;
            if let Some(stream) = resolve(&resolver, &channel, &mut report) {
                report.manifest = probe_manifest(&stream);
                report.verdict = if report.manifest["status"] == "fetched" {
                    "manifest_only"
                } else {
                    "manifest_failed"
                }
                .into();
            }
            report.save(&path)?;
            println!(
                "{}: {}. Playback: non verificato. Report: {}",
                channel.name,
                report.verdict,
                path.display()
            );
            Ok(report.verdict == "manifest_only")
        }
        Commands::Play {
            channel,
            options,
            report: path,
        } => {
            let channel = find_channel(&channel)?;
            options.runtime.selection().validate(options.headless)?;
            let resolver = Resolver::new()?;
            let report = play(&resolver, &channel, &options, &running)?;
            let path = path.unwrap_or_else(|| {
                default_report(
                    Some(options.runtime.runtime),
                    &report.run_id,
                    &format!("{}-play", channel.id),
                )
            });
            report.save(&path)?;
            println!(
                "{}: {}. Report: {}",
                channel.name,
                report.verdict,
                path.display()
            );
            Ok(report.verdict == "playback_verified")
        }
        Commands::Matrix {
            channels: ids,
            options,
            report: path,
        } => {
            options.runtime.selection().validate(options.headless)?;
            let selected: Vec<Channel> = ids
                .iter()
                .map(|id| find_channel(id))
                .collect::<Result<_>>()?;
            let path = path.unwrap_or_else(|| {
                default_report(
                    Some(options.runtime.runtime),
                    &uuid::Uuid::new_v4().to_string(),
                    "matrix",
                )
            });
            let resolver = Resolver::new()?;
            let mut reports = vec![];
            for channel in selected {
                if !running.load(Ordering::Relaxed) {
                    break;
                }
                println!("Prova {} su {}…", channel.name, std::env::consts::OS);
                let report = play(&resolver, &channel, &options, &running)?;
                println!("{}: {}", channel.name, report.verdict);
                let individual = matrix_channel_report(&path, &channel.id);
                report.save(&individual)?;
                reports.push(report);
                write_matrix(&path, &reports, ids.len(), options.runtime.runtime)?;
            }
            write_matrix(&path, &reports, ids.len(), options.runtime.runtime)?;
            println!("Matrice salvata: {}", path.display());
            Ok(reports.len() == ids.len()
                && reports.iter().all(|r| r.verdict == "playback_verified"))
        }
    }
}

fn default_report(backend: Option<Backend>, run_id: &str, name: &str) -> PathBuf {
    let backend = backend.map(Backend::slug).unwrap_or("resolver");
    PathBuf::from("reports")
        .join(format!("{}-{backend}-{run_id}", std::env::consts::OS))
        .join(format!("{name}.json"))
}

fn matrix_channel_report(matrix: &Path, channel_id: &str) -> PathBuf {
    let mut name = matrix
        .file_stem()
        .unwrap_or_else(|| std::ffi::OsStr::new("matrix"))
        .to_os_string();
    name.push(format!("-{channel_id}-play.json"));
    matrix.with_file_name(name)
}

fn find_channel(id: &str) -> Result<Channel> {
    let id = if id == "la7-cinema" { "la7cinema" } else { id };
    lume_player::catalog::catalog()
        .iter()
        .find(|c| {
            c.id == id || c.epg_id.eq_ignore_ascii_case(id) || c.name.eq_ignore_ascii_case(id)
        })
        .map(|c| c.stream_channel())
        .ok_or_else(|| anyhow!("Unknown channel; use the channels command"))
}
fn write_matrix(
    path: &Path,
    reports: &[RunReport],
    requested: usize,
    backend: Backend,
) -> Result<()> {
    write_json(
        path,
        &json!({"schema_version":1,"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"backend":backend.name(),"requested_channels":requested,"completed_channels":reports.len(),"runs":reports,"other_operating_systems":"not_tested_by_this_run"}),
    )
}

fn resolve(
    resolver: &Resolver,
    channel: &Channel,
    report: &mut RunReport,
) -> Option<ResolvedStream> {
    let started = Instant::now();
    match resolver.resolve(channel) {
        Ok(stream) => {
            report.resolver = json!({"status":"resolved","duration_ms":started.elapsed().as_millis(),"declared_drm":stream.drm.is_some(),"mime_type":stream.mime_type});
            Some(stream)
        }
        Err(error) => {
            report.resolver = json!({"status":"failed","reason":error.to_string(),"duration_ms":started.elapsed().as_millis()});
            report.verdict = "resolver_failed".into();
            None
        }
    }
}

fn prepare_profile(path: &Path) -> Result<()> {
    let marker = path.join(".lume-player-profile");
    if path.exists() && !marker.exists() && fs::read_dir(path)?.next().is_some() {
        bail!("Profile directory is not an empty or marked Lume prototype profile; choose a dedicated directory");
    }
    fs::create_dir_all(path).context("Cannot create prototype profile")?;
    fs::write(marker, b"Lume player isolated diagnostic profile\n")?;
    Ok(())
}

fn play(
    resolver: &Resolver,
    channel: &Channel,
    options: &PlayOptions,
    running: &AtomicBool,
) -> Result<RunReport> {
    let selection = options.runtime.selection();
    selection.validate(options.headless)?;
    let mut report = RunReport::new("play", Some(channel));
    report.set_runtime(selection.report_metadata());
    let spec = match selection.detect() {
        Ok(spec) => spec,
        Err(_) => {
            report.verdict = "runtime_missing".into();
            report.runtime["error"] = json!(selection.missing_reason());
            return Ok(report);
        }
    };
    report.set_runtime(spec.report_metadata());
    report.runtime["mode"] = json!(options.mode.name());
    report.runtime["headless"] = json!(options.headless);
    report.runtime["measurement_margin_seconds"] = json!(2);
    let Some(stream) = resolve(resolver, channel, &mut report) else {
        return Ok(report);
    };
    let drm = stream.drm.is_some();
    let setup_seconds = if drm && selection.backend == Backend::Cef {
        options.drm_setup_seconds
    } else {
        0
    };
    report.runtime["drm_setup_timeout_seconds"] = json!(setup_seconds);
    report.manifest = probe_manifest(&stream);
    if report.manifest["status"] != "fetched" {
        report.verdict = "manifest_failed".into();
        return Ok(report);
    }
    let profile = options
        .profile
        .clone()
        .unwrap_or_else(|| selection.backend.default_profile());
    prepare_profile(&profile)?;
    let server = HarnessServer::start(
        channel.clone(),
        stream,
        options.seconds + 2,
        options.mode.name(),
    )?;
    server.set_drm_setup_timeout(setup_seconds);
    let mut child = match spec.launch(LaunchOptions {
        profile: &profile,
        url: &server.launch_url,
        headless: options.headless,
    }) {
        Ok(child) => child,
        Err(_) => {
            report.verdict = "runtime_launch_failed".into();
            report.runtime["error"] = json!("runtime_launch_failed");
            return Ok(report);
        }
    };
    println!(
        "Player aperto: {}. Durata richiesta: {} s. Audio inizialmente disattivato.",
        channel.name, options.seconds
    );
    let deadline =
        Instant::now() + Duration::from_secs(setup_seconds + 100.max(options.seconds * 2 + 45));
    let mut timed_out = false;
    loop {
        if server.state.complete.load(Ordering::Relaxed) || !running.load(Ordering::Relaxed) {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    child.shutdown();
    report.events = server
        .state
        .events
        .lock()
        .map(|e| e.clone())
        .unwrap_or_default();
    report.playback = assess_playback(
        &report.events,
        options.seconds,
        drm,
        server.state.license_success.load(Ordering::Relaxed) > 0,
    );
    report.playback["network"] = server.network_summary();
    report.playback["timed_out"] = json!(timed_out);
    report.playback["interrupted"] = json!(!running.load(Ordering::Relaxed));
    let initialized = report
        .events
        .iter()
        .any(|event| event["kind"] == "capabilities");
    if selection.backend == Backend::Cef {
        report.runtime["embedded_runtime_initialized"] = json!(initialized);
        report.runtime["embedded_runtime_verified"] =
            json!(report.playback["status"] == "verified");
    }
    if let Some(capabilities) = latest_capabilities(&report.events) {
        if let Some(available) = capabilities["widevine_available"].as_bool() {
            report.runtime["widevine_available"] = json!(available);
            report.runtime["widevine_probe_scope"] = json!("codec_specific_eme_configurations");
            report.runtime["widevine"] = json!(match (
                available,
                capabilities["widevine_webm_available"].as_bool()
            ) {
                (true, _) => "available_for_avc_aac",
                (false, Some(true)) => "available_for_webm_only",
                (false, Some(false)) => "unavailable_for_tested_configurations",
                (false, None) => "unavailable_for_avc_aac",
            });
        }
        for field in [
            "widevine_webm_available",
            "avc_mse_supported",
            "aac_mse_supported",
            "webm_mse_supported",
        ] {
            if let Some(available) = capabilities[field].as_bool() {
                report.runtime[field] = json!(available);
            }
        }
        if capabilities["avc_mse_supported"] == false || capabilities["aac_mse_supported"] == false
        {
            report.limitations.push("The tested AVC/AAC codec configuration is unavailable in this runtime build; this does not establish an absent CDM or a provider license refusal.".into());
        }
    }
    report.verdict = playback_verdict(
        &report.playback,
        &report.events,
        drm,
        options.headless,
        timed_out,
    )
    .into();
    Ok(report)
}

fn playback_verdict(
    playback: &Value,
    events: &[Value],
    drm: bool,
    headless: bool,
    timed_out: bool,
) -> &'static str {
    if playback["status"] == "verified" {
        if headless {
            "headless_playback_observed"
        } else {
            "playback_verified"
        }
    } else if latest_capabilities(events).is_some_and(|event| {
        event["avc_mse_supported"] == false || event["aac_mse_supported"] == false
    }) {
        // Current providers require AVC/AAC. Preserve the actual Shaka error separately.
        "codec_unavailable"
    } else if drm
        && events
            .iter()
            .any(|event| event["kind"] == "drm_setup" && event["status"] == "timeout")
    {
        "widevine_setup_timeout"
    } else if latest_capabilities(events).is_some_and(|event| event["widevine_available"] == false)
        && drm
    {
        "widevine_unavailable"
    } else if timed_out {
        "playback_timeout"
    } else {
        "playback_not_verified"
    }
}

fn latest_capabilities(events: &[Value]) -> Option<&Value> {
    events.iter().rfind(|event| event["kind"] == "capabilities")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_drm_capabilities_replace_the_initial_first_run_result() {
        let events = vec![
            json!({"kind":"capabilities", "widevine_available":false}),
            json!({"kind":"drm_setup", "status":"waiting"}),
            json!({"kind":"capabilities", "widevine_available":true}),
            json!({"kind":"drm_setup", "status":"ready"}),
        ];
        assert_eq!(
            latest_capabilities(&events).unwrap()["widevine_available"],
            true
        );
        // A later player error must not be attributed to the earlier absent CDM.
        assert_eq!(
            playback_verdict(
                &json!({"status":"not_verified"}),
                &events,
                true,
                false,
                false
            ),
            "playback_not_verified"
        );
        let timeout = vec![
            json!({"kind":"capabilities", "widevine_available":false}),
            json!({"kind":"drm_setup", "status":"timeout"}),
        ];
        assert_eq!(
            playback_verdict(
                &json!({"status":"not_verified"}),
                &timeout,
                true,
                false,
                false
            ),
            "widevine_setup_timeout"
        );
    }

    #[test]
    fn missing_avc_aac_is_classified_before_drm_availability_without_a_provider_claim() {
        let capabilities = vec![
            json!({"kind":"capabilities", "avc_mse_supported":false, "aac_mse_supported":false, "widevine_available":false, "widevine_webm_available":true}),
        ];
        let failed = json!({"status":"not_verified"});
        assert_eq!(
            playback_verdict(&failed, &capabilities, true, false, false),
            "codec_unavailable"
        );
        assert_eq!(
            playback_verdict(&failed, &capabilities, false, false, false),
            "codec_unavailable"
        );
        assert_eq!(
            playback_verdict(
                &json!({"status":"verified"}),
                &capabilities,
                false,
                false,
                false
            ),
            "playback_verified"
        );
        let codecs_present = vec![
            json!({"kind":"capabilities", "avc_mse_supported":true, "aac_mse_supported":true, "widevine_available":false, "widevine_webm_available":false}),
        ];
        assert_eq!(
            playback_verdict(&failed, &codecs_present, true, false, false),
            "widevine_unavailable"
        );
    }

    #[test]
    fn cli_defaults_to_cef_and_external_browser_requires_explicit_selection() {
        let parsed = Cli::try_parse_from(["lume-player", "play", "--channel", "rai1"]).unwrap();
        let Commands::Play { options, .. } = parsed.command else {
            panic!("expected play");
        };
        assert_eq!(options.runtime.runtime, Backend::Cef);
        assert_eq!(options.drm_setup_seconds, 300);
        assert!(options.runtime.selection().validate(false).is_ok());
        let parsed = Cli::try_parse_from([
            "lume-player",
            "play",
            "--channel",
            "rai1",
            "--browser",
            "chrome",
        ])
        .unwrap();
        let Commands::Play { options, .. } = parsed.command else {
            panic!("expected play");
        };
        assert!(options.runtime.selection().validate(false).is_err());
        let parsed = Cli::try_parse_from([
            "lume-player",
            "play",
            "--channel",
            "rai1",
            "--runtime",
            "browser",
            "--browser",
            "chrome",
            "--headless",
        ])
        .unwrap();
        let Commands::Play { options, .. } = parsed.command else {
            panic!("expected play");
        };
        assert_eq!(options.runtime.runtime, Backend::Browser);
        assert!(options
            .runtime
            .selection()
            .validate(options.headless)
            .is_ok());
    }

    #[test]
    fn default_report_paths_are_unique_and_separate_runtime_evidence() {
        let cef = default_report(Some(Backend::Cef), "run-a", "matrix");
        let browser = default_report(Some(Backend::Browser), "run-a", "matrix");
        let next = default_report(Some(Backend::Cef), "run-b", "matrix");
        assert_ne!(cef, browser);
        assert_ne!(cef, next);
        assert_ne!(
            cef,
            PathBuf::from(format!("reports/{}-matrix.json", std::env::consts::OS))
        );
        assert_eq!(cef.file_name().unwrap(), "matrix.json");
    }

    #[test]
    fn custom_matrix_stem_names_individual_reports_without_overwriting_other_runs() {
        assert_eq!(
            matrix_channel_report(Path::new("reports/macos-cef-stock-matrix.json"), "rai1"),
            PathBuf::from("reports/macos-cef-stock-matrix-rai1-play.json")
        );
        assert_ne!(
            matrix_channel_report(Path::new("reports/macos-cef-stock-matrix.json"), "rai1"),
            matrix_channel_report(Path::new("reports/macos-matrix.json"), "rai1")
        );
    }

    #[test]
    fn rejects_existing_personal_profile() {
        let path = std::env::temp_dir().join(format!("lume-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("Preferences"), "personal").unwrap();
        assert!(prepare_profile(&path).is_err());
        fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn own_profile_is_reusable() {
        let path = std::env::temp_dir().join(format!("lume-test-{}", uuid::Uuid::new_v4()));
        prepare_profile(&path).unwrap();
        prepare_profile(&path).unwrap();
        fs::remove_dir_all(path).unwrap();
    }
}
