// SPDX-License-Identifier: GPL-3.0-or-later
//! URL conventions adapted from Android CatchupUrls; no provider code is run.
use super::{guide::GuideEvent, Entry, GuideSettings};
use anyhow::{anyhow, bail, Result};
use chrono::{Datelike, TimeZone, Timelike};
use reqwest::Url;
pub struct Archive {
    pub url: String,
    pub duration: f64,
    pub offset: f64,
    pub regenerate: bool,
}
struct Spec {
    mode: String,
    template: String,
    days: f64,
    correction: f64,
}
fn spec(entry: &Entry, settings: &GuideSettings) -> Result<Option<Spec>> {
    let attrs = &entry.metadata.attributes;
    let get = |s: &str| attrs.get(s).map(String::as_str).unwrap_or("");
    let template = get("catchup-source").to_owned();
    let legacy = get("timeshift")
        .parse::<f64>()
        .ok()
        .or_else(|| get("tvg-rec").parse().ok())
        .unwrap_or(0.0);
    let mode = get("catchup").trim().to_lowercase();
    let mode = if mode.is_empty() {
        if !template.is_empty() {
            "default".into()
        } else if legacy > 0.0 {
            "shift".into()
        } else {
            return Ok(None);
        }
    } else {
        mode
    };
    if ["disabled", "none", "off", "0"].contains(&mode.as_str()) {
        return Ok(None);
    }
    if ![
        "default",
        "append",
        "shift",
        "timeshift",
        "flussonic",
        "flussonic-hls",
        "flussonic-ts",
        "fs",
        "flussonic-dash",
        "xc",
        "vod",
    ]
    .contains(&mode.as_str())
    {
        bail!("Modalità archivio non supportata.");
    }
    let days = settings
        .archive_days
        .map(|v| v as f64)
        .or_else(|| get("catchup-days").parse().ok())
        .or_else(|| get("catchup-time").parse::<f64>().ok().map(|v| v / 86400.0))
        .unwrap_or(legacy);
    let correction = get("catchup-correction").parse::<f64>().unwrap_or(0.0) * 3600.0;
    if !days.is_finite()
        || !(0.0..=365.0).contains(&days)
        || !correction.is_finite()
        || correction.abs() > 86400.0
    {
        bail!("Disponibilità o correzione dell’archivio non valida.");
    }
    Ok(Some(Spec {
        mode,
        template,
        days,
        correction,
    }))
}
pub fn available(
    entry: &Entry,
    settings: &GuideSettings,
    event: &GuideEvent,
    now: u64,
    ongoing: bool,
) -> bool {
    spec(entry, settings).ok().flatten().is_some_and(|s| {
        s.days > 0.0
            && event.programme.start < now
            && event.programme.end > event.programme.start
            && event.programme.start >= now.saturating_sub((s.days * 86400000.0) as u64)
            && (event.programme.end <= now || ongoing)
            && (s.mode != "vod" || !event.catchup_id.is_empty())
    })
}
fn encoded(value: &str) -> String {
    Url::parse_with_params("https://encode.invalid/", &[("v", value)])
        .unwrap()
        .query()
        .unwrap()[2..]
        .replace('+', "%20")
}
fn date(value: f64, format: &str, zone: chrono_tz::Tz) -> Result<String> {
    if format.len() > 80 {
        bail!("Formato della data dell’archivio troppo lungo.");
    }
    let time = zone
        .timestamp_millis_opt((value * 1000.0) as i64)
        .single()
        .ok_or_else(|| anyhow!("Data dell’archivio non valida."))?;
    let mut result = String::new();
    for c in format.chars() {
        result.push_str(&match c {
            'Y' => format!("{:04}", time.year()),
            'm' => format!("{:02}", time.month()),
            'd' => format!("{:02}", time.day()),
            'H' => format!("{:02}", time.hour()),
            'M' => format!("{:02}", time.minute()),
            'S' => format!("{:02}", time.second()),
            '-' | ':' | '_' | '.' | 'T' => c.to_string(),
            ' ' => "%20".into(),
            _ => bail!("Formato della data dell’archivio non supportato."),
        });
    }
    Ok(result)
}
pub fn build(
    entry: &Entry,
    settings: &GuideSettings,
    event: &GuideEvent,
    now: u64,
    offset: f64,
) -> Result<Archive> {
    if !available(entry, settings, event, now, settings.ongoing) {
        bail!("Il programma non è disponibile nell’archivio della sorgente.");
    }
    let spec =
        spec(entry, settings)?.ok_or_else(|| anyhow!("Il canale non dichiara un archivio."))?;
    let begin = (event.programme.start as f64 / 1000.0 - settings.before_seconds as f64)
        .max(now as f64 / 1000.0 - spec.days * 86400.0);
    let finish = (event.programme.end as f64 / 1000.0 + settings.after_seconds as f64)
        .min(now as f64 / 1000.0);
    let duration = (finish - begin).max(1.0);
    let offset = offset.clamp(0.0, (duration - 1.0).max(0.0));
    let start = begin + offset + spec.correction;
    let end = finish + spec.correction;
    let zone = settings
        .archive_timezone
        .parse::<chrono_tz::Tz>()
        .map_err(|_| anyhow!("Fuso dell’archivio non valido."))?;
    let live = Url::parse(&entry.url)?;
    let root = live.origin().ascii_serialization();
    let path = live.path();
    let parent = path.rsplit_once('/').map(|(s, _)| s).unwrap_or("");
    let query = live.query().map(|q| format!("?{q}")).unwrap_or_default();
    let append = |s: &str| {
        format!(
            "{}{}{}",
            entry.url,
            if live.query().is_some() {
                if entry.url.ends_with(['?', '&']) {
                    ""
                } else {
                    "&"
                }
            } else {
                "?"
            },
            s.trim_start_matches(['?', '&'])
        )
    };
    let mut template = if !spec.template.is_empty() {
        if spec.mode == "append" || spec.template.starts_with(['?', '&']) {
            append(&spec.template)
        } else {
            spec.template.clone()
        }
    } else {
        match spec.mode.as_str() {
            "default" | "append" | "shift" | "timeshift" => append("utc={utc}&lutc={lutc}"),
            "flussonic" | "flussonic-hls" => {
                format!("{root}{parent}/timeshift_rel-{{offset:1}}.m3u8{query}")
            }
            "flussonic-ts" | "fs" => format!("{root}{parent}/timeshift_abs-{{utc}}.ts{query}"),
            "flussonic-dash" => format!("{root}{parent}/archive-{{utc}}-{{duration}}.mpd{query}"),
            "xc" => {
                let re =
                    regex::Regex::new(r"^/(?:live/)?([^/]+)/([^/]+)/([^/.]+)(?:\.(ts|m3u8))?$")
                        .unwrap();
                let c = re.captures(path).ok_or_else(|| {
                    anyhow!("L’indirizzo non consente di costruire l’archivio Xtream.")
                })?;
                let time = date(start, "Y-m-d:H-M", zone)?;
                format!(
                    "{root}/timeshift/{}/{}/{}/{time}/{}.{}{query}",
                    &c[1],
                    &c[2],
                    ((end - start) / 60.0).ceil().max(1.0) as u64,
                    &c[3],
                    c.get(4).map(|m| m.as_str()).unwrap_or("ts")
                )
            }
            "vod" => "{catchup-id}".into(),
            _ => bail!("Manca il modello dell’archivio."),
        }
    };
    let pattern = regex::Regex::new(r"\$?\{([^{}]+)\}").unwrap();
    let regenerate = spec.mode == "xc"
        || pattern.captures_iter(&template).any(|c| {
            ["utc", "start", "offset", "Y", "m", "d", "H", "M", "S"]
                .contains(&c[1].split(':').next().unwrap_or(""))
        });
    if ["{catchup-id}", "${catchup-id}"].contains(&template.as_str()) {
        template = event.catchup_id.clone();
    } else {
        let mut result = String::new();
        let mut previous = 0;
        for c in pattern.captures_iter(&template) {
            let full = c.get(0).unwrap();
            result.push_str(&template[previous..full.start()]);
            let (name, format) = c[1].split_once(':').unwrap_or((&c[1], ""));
            let value = match name {
                "utc" | "start" | "utcend" | "end" | "lutc" | "now" | "timestamp" => {
                    let time = match name {
                        "utc" | "start" => start,
                        "utcend" | "end" => end,
                        _ => now as f64 / 1000.0,
                    };
                    if format.is_empty() {
                        (time.floor() as i64).to_string()
                    } else {
                        date(time, format, zone)?
                    }
                }
                "duration" | "offset" => {
                    let divisor = if format.is_empty() {
                        1
                    } else {
                        format
                            .parse::<u64>()
                            .ok()
                            .filter(|v| (1..=86400000).contains(v))
                            .ok_or_else(|| {
                                anyhow!("Divisore temporale dell’archivio non valido.")
                            })?
                    };
                    let value = if name == "duration" {
                        end - start
                    } else {
                        (now as f64 / 1000.0 - start).max(0.0)
                    };
                    (value / divisor as f64).floor().to_string()
                }
                "catchup-id" => encoded(&event.catchup_id),
                "Y" | "m" | "d" | "H" | "M" | "S" => date(start, name, zone)?,
                "start-year" => date(start, "Y", zone)?,
                "start-mon" => date(start, "m", zone)?,
                "start-day" => date(start, "d", zone)?,
                "start-hour" => date(start, "H", zone)?,
                "start-min" => date(start, "M", zone)?,
                "start-sec" => date(start, "S", zone)?,
                _ => bail!("Il modello dell’archivio contiene un segnaposto non supportato."),
            };
            result.push_str(&value);
            previous = full.end();
        }
        result.push_str(&template[previous..]);
        template = result;
    }
    let url = live
        .join(&template)
        .map_err(|_| anyhow!("Indirizzo dell’archivio non valido."))?;
    super::network::validate_url(url.as_str(), true)?;
    Ok(Archive {
        url: url.into(),
        duration,
        offset,
        regenerate,
    })
}
