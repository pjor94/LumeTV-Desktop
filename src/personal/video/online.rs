// SPDX-License-Identifier: GPL-3.0-or-later
//! Optional TMDB enrichment. This transport never receives playlist credentials.
use super::*;
use anyhow::anyhow;
use reqwest::{blocking::Client, redirect::Policy};
use std::{
    borrow::Cow,
    fs,
    io::Read,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

const DAY: u64 = 86_400_000;
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    enabled: bool,
    token: String,
}
impl Settings {
    fn read_token(&self) -> Cow<'_, str> {
        if self.token.is_empty() {
            Cow::Owned(std::env::var("LUME_TMDB_READ_TOKEN").unwrap_or_default())
        } else {
            Cow::Borrowed(&self.token)
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Candidate {
    id: u32,
    kind: String,
    title: String,
    original_title: String,
    year: Option<u32>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Details {
    id: u32,
    kind: String,
    title: String,
    year: Option<u32>,
    description: String,
    genres: Vec<String>,
    cast: Vec<String>,
    directors: Vec<String>,
    overview_language: String,
    duration: f64,
    poster: String,
    backdrop: String,
}
fn private_file(path: &std::path::Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
fn client() -> Result<Client> {
    Ok(Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .dns_resolver(Arc::new(crate::network::PublicDns))
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(10))
        .user_agent("LumeTV metadata")
        .build()?)
}
fn clean(value: &str) -> String {
    static TECH: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)[(\[]\s*(?:4k|uhd|fhd|hd|sd|720p|1080p|2160p|hdr|hdr10|dv|h[. ]?26[45]|x26[45]|hevc|ita|it|multi|dual)(?:[ ,+/_-]+(?:4k|uhd|fhd|hd|sd|720p|1080p|2160p|hdr|hdr10|dv|h[. ]?26[45]|x26[45]|hevc|ita|it|multi|dual))*\s*[)\]]").unwrap()
    });
    static PREFIX: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)^(?:(?:it|ita|italian|multi|4k|uhd|fhd|hd|sd)\s*\|\s*)+").unwrap()
    });
    static BRACKET: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[(\[](?:19|20)\d{2}[)\]]").unwrap());
    BRACKET
        .replace_all(
            &TECH.replace_all(
                &PREFIX.replace_all(&value.chars().take(300).collect::<String>(), ""),
                " ",
            ),
            " ",
        )
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches([' ', '.', '_', '-', '|'])
        .to_owned()
}
fn fold(value: &str) -> String {
    super::normalized(value)
}
fn similarity(a: &str, b: &str) -> u32 {
    let a = fold(a);
    let b = fold(b);
    let a = a
        .split_whitespace()
        .collect::<std::collections::BTreeSet<_>>();
    let b = b
        .split_whitespace()
        .collect::<std::collections::BTreeSet<_>>();
    let union = a.union(&b).count();
    if union == 0 {
        0
    } else {
        (a.intersection(&b).count() * 30 / union) as u32
    }
}
fn text(value: &Value, key: &str, limit: usize) -> String {
    TAG.replace_all(value[key].as_str().unwrap_or(""), "")
        .trim()
        .chars()
        .take(limit)
        .collect()
}
fn year(value: &Value, key: &str) -> Option<u32> {
    value[key]
        .as_str()
        .and_then(|s| s.get(..4))
        .and_then(|s| s.parse().ok())
        .filter(|y| (1888..=2100).contains(y))
}
fn names(value: &Value, limit: usize) -> Vec<String> {
    value
        .as_array()
        .map(|rows| {
            rows.iter()
                .take(2000)
                .map(|v| text(v, "name", 120))
                .filter(|v| !v.is_empty())
                .take(limit)
                .collect()
        })
        .unwrap_or_default()
}
fn image_path(path: &str, backdrop: bool) -> Option<String> {
    static IMAGE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^/[A-Za-z0-9_-]+\.(?:jpg|png|webp)$").unwrap());
    IMAGE.is_match(path).then(|| {
        format!(
            "https://image.tmdb.org/t/p/{}{path}",
            if backdrop { "w1280" } else { "w500" }
        )
    })
}
fn mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[255, 216, 255]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}
impl Library {
    fn metadata_settings(&self) -> Result<Settings> {
        let path = self.directory.join("video-settings.json");
        if path.exists() {
            let bytes = fs::read(path)?;
            if bytes.len() > 8192 {
                bail!("Configurazione video non valida.");
            }
            Ok(serde_json::from_slice(&bytes)?)
        } else {
            Ok(Settings::default())
        }
    }
    pub(in crate::personal) fn metadata_db(&self) -> Result<Connection> {
        let db = open(&self.directory.join("video-metadata.sqlite"))?;
        db.execute_batch("PRAGMA max_page_count=16384; CREATE TABLE IF NOT EXISTS matches(source TEXT NOT NULL,key TEXT NOT NULL,fingerprint TEXT NOT NULL,selected INTEGER,excluded INTEGER NOT NULL,candidates TEXT NOT NULL,manual INTEGER NOT NULL,expires INTEGER NOT NULL,PRIMARY KEY(source,key)); CREATE TABLE IF NOT EXISTS details(kind TEXT NOT NULL,id INTEGER NOT NULL,value TEXT NOT NULL,expires INTEGER NOT NULL,PRIMARY KEY(kind,id));")?;
        Ok(db)
    }
    pub fn video_online_settings(&self, patch: &Value) -> Result<Value> {
        let _operation = self.operation.lock().expect("metadata settings");
        let mut settings = self.metadata_settings()?;
        let changing = patch.get("enabled").is_some()
            || patch.get("token").is_some()
            || patch["clear"] == true;
        if changing {
            self.video_epoch.fetch_add(1, Ordering::AcqRel);
            self.video_retry.store(0, Ordering::Release);
        }
        if let Some(token) = patch["token"].as_str() {
            if token.len() > 4096
                || !token
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                bail!("Token TMDB non valido.");
            }
            settings.token = token.into();
        }
        if let Some(enabled) = patch["enabled"].as_bool() {
            settings.enabled = enabled;
        }
        if changing {
            let path = self.directory.join("video-settings.json");
            let tmp = self.directory.join(".video-settings.tmp");
            fs::write(&tmp, serde_json::to_vec(&settings)?)?;
            private_file(&tmp)?;
            fs::rename(tmp, path)?;
        }
        if patch["clear"] == true {
            self.metadata_db()?.execute_batch("DELETE FROM details; DELETE FROM matches WHERE manual=0; UPDATE matches SET candidates='[]',expires=0 WHERE excluded=0; VACUUM;")?;
            let _ = fs::remove_dir_all(self.directory.join("video-images"));
        }
        Ok(json!({"enabled":settings.enabled,"configured":!settings.read_token().is_empty()}))
    }
    fn online_input(&self, id: &str) -> Result<(Source, Metadata, String, String)> {
        let (source, entry) = self.entry(id)?;
        if entry.display(&source.id, &source.settings).content != "vod" {
            bail!("Questo contenuto non è un video.");
        }
        let meta = Metadata::from(&entry);
        let key = if meta.series.is_empty() {
            format!("i:{id}")
        } else {
            format!("s:{}", meta.series)
        };
        let fingerprint = meta.fingerprint();
        Ok((source, meta, key, fingerprint))
    }
    fn online_valid(&self, source: &Source, epoch: u64) -> bool {
        self.video_epoch.load(Ordering::Acquire) == epoch
            && self
                .sources
                .lock()
                .expect("sources")
                .get(&source.id)
                .is_some_and(|s| {
                    s.enabled && s.revision == source.revision && s.snapshot == source.snapshot
                })
    }
    fn cached_details(
        &self,
        source: &str,
        key: &str,
        fingerprint: &str,
        kind: &str,
    ) -> Result<Option<Details>> {
        let db = self.metadata_db()?;
        let raw=db.query_row("SELECT d.value FROM matches m JOIN details d ON d.kind=? AND d.id=m.selected WHERE m.source=? AND m.key=? AND m.excluded=0 AND (m.fingerprint=? OR m.manual=1) LIMIT 1",params![kind,source,key,fingerprint],|r|r.get::<_,String>(0)).optional()?;
        raw.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    pub(super) fn online_cached(&self, item: &mut Item) -> Result<()> {
        if !self.metadata_settings().map(|s| s.enabled).unwrap_or(false) {
            return Ok(());
        }
        if let Ok(Some(details)) =
            self.cached_details(&item.source, &item.key(), &item.lookup, &item.lookup_kind)
        {
            item.online = true;
            if item.description.is_empty() {
                item.description = details.description;
                item.overview_language = details.overview_language;
            }
            if item.duration <= 0.0 {
                item.duration = details.duration;
            }
            if item.lookup_kind == "tv" {
                item.year = details.year.or(item.year);
            } else {
                item.year = item.year.or(details.year);
            }
            if item.genres.is_empty() {
                item.genres = details.genres;
            }
            item.directors = details.directors;
            item.cast = details.cast;
            item.poster |= image_path(&details.poster, false).is_some();
            item.backdrop |= image_path(&details.backdrop, true).is_some();
        }
        Ok(())
    }
    fn tmdb_get(
        &self,
        settings: &Settings,
        route: &str,
        query: &[(&str, String)],
    ) -> Result<Value> {
        self.tmdb_get_language(settings, route, query, "it-IT")
    }
    fn tmdb_get_language(
        &self,
        settings: &Settings,
        route: &str,
        query: &[(&str, String)],
        language: &str,
    ) -> Result<Value> {
        let now = crate::report::now_ms();
        if self.video_retry.load(Ordering::Acquire) > now {
            bail!("Il servizio richiede una pausa. Riprova più tardi.");
        }
        let url = format!("https://api.themoviedb.org/3/{route}");
        let response = client()?
            .get(url)
            .bearer_auth(settings.read_token())
            .header("Accept", "application/json")
            .query(&[("language", language)])
            .query(query)
            .send()
            .map_err(|_| {
                anyhow!("Informazioni online non disponibili. Puoi continuare a guardare il video.")
            })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            let delay = if status == 401 || status == 403 {
                86_400
            } else {
                response
                    .headers()
                    .get("Retry-After")
                    .and_then(|h| h.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(60)
                    .clamp(1, 900)
            };
            self.video_retry
                .store(now + delay * 1000, Ordering::Release);
            bail!(if status == 401 || status == 403 {
                "Il token TMDB non è valido. Controlla la configurazione."
            } else if status == 429 {
                "Il servizio richiede una pausa. Riprova più tardi."
            } else {
                "Informazioni online temporaneamente non disponibili."
            });
        }
        let mut bytes = vec![];
        response
            .take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Risposta TMDB non disponibile."))?;
        if bytes.len() > 4 * 1024 * 1024 {
            bail!("Risposta TMDB troppo grande.");
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Risposta TMDB non valida."))
    }
    pub fn video_metadata(
        &self,
        id: &str,
        search: Option<&str>,
        choose: Option<Option<u32>>,
    ) -> Result<Value> {
        let (source, meta, key, fingerprint) = self.online_input(id)?;
        let settings = self.metadata_settings()?;
        if !settings.enabled {
            return Ok(json!({"status":"disabled","candidates":[]}));
        }
        if settings.read_token().is_empty() {
            return Ok(json!({"status":"unavailable","candidates":[]}));
        }
        let epoch = self.video_epoch.load(Ordering::Acquire);
        let _flight = self.video_online.lock().expect("metadata flight");
        if !self.online_valid(&source, epoch) {
            bail!("La lista è cambiata. Riapri il video.");
        }
        let Some(kind) = meta.online_kind() else {
            return Ok(json!({"status":"unsupported","candidates":[]}));
        };
        let title = clean(search.filter(|s| !s.trim().is_empty()).unwrap_or(
            if meta.series.is_empty() {
                &meta.title
            } else {
                &meta.series_title
            },
        ));
        if title.is_empty() {
            return Ok(json!({"status":"not_found","candidates":[]}));
        }
        let db = self.metadata_db()?;
        let old=db.query_row("SELECT selected,excluded,candidates,expires,manual FROM matches WHERE source=? AND key=? AND (fingerprint=? OR manual=1)",params![source.id,key,fingerprint],|r|Ok((r.get::<_,Option<u32>>(0)?,r.get::<_,bool>(1)?,r.get::<_,String>(2)?,r.get::<_,u64>(3)?,r.get::<_,bool>(4)?))).optional()?;
        if choose == Some(None) {
            let _operation = self.operation.lock().expect("metadata choose");
            if !self.online_valid(&source, epoch) {
                bail!("La lista è cambiata.");
            }
            db.execute(
                "INSERT OR REPLACE INTO matches VALUES(?,?,?,NULL,1,'[]',1,?)",
                params![source.id, key, fingerprint, u64::MAX / 2],
            )?;
            return Ok(json!({"status":"excluded","candidates":[]}));
        }
        if search.is_none() && choose.is_none() {
            if let Some((selected, excluded, candidates, expires, _)) = &old {
                if *excluded {
                    return Ok(json!({"status":"excluded","candidates":[]}));
                }
                let fresh = selected
                    .and_then(|n| {
                        db.query_row(
                            "SELECT expires FROM details WHERE kind=? AND id=?",
                            params![kind, n],
                            |r| r.get::<_, u64>(0),
                        )
                        .optional()
                        .ok()
                        .flatten()
                    })
                    .is_some_and(|e| e > crate::report::now_ms());
                if fresh || selected.is_none() && *expires > crate::report::now_ms() {
                    return Ok(
                        json!({"status":if fresh{"ready"}else if candidates=="[]"{"not_found"}else{"ambiguous"},"candidates":serde_json::from_str::<Value>(candidates)?}),
                    );
                }
            }
        }
        let known = choose.flatten().filter(|id| *id > 0).or_else(|| {
            if search.is_none() {
                old.as_ref().and_then(|o| o.0).or(meta.tmdb_id)
            } else {
                None
            }
        });
        let mut candidates = vec![];
        let release = if kind == "movie" {
            meta.year
        } else {
            let name = if meta.series.is_empty() {
                &meta.title
            } else {
                &meta.series_title
            };
            static BRACKET_YEAR: LazyLock<Regex> =
                LazyLock::new(|| Regex::new(r"[\(\[]((?:19|20)\d{2})[\)\]]").unwrap());
            meta.series_year
                .or_else(|| BRACKET_YEAR.captures(name).and_then(|c| c[1].parse().ok()))
        };
        let imdb = Regex::new(r"^tt\d{7,10}$").unwrap().is_match(&meta.imdb_id) && search.is_none();
        if known.is_none() {
            let mut query = vec![("query", title.clone()), ("include_adult", "false".into())];
            if let Some(year) = release.filter(|_| search.is_none()) {
                query.push((
                    if kind == "movie" {
                        "year"
                    } else {
                        "first_air_date_year"
                    },
                    year.to_string(),
                ));
            }
            let result = if imdb {
                self.tmdb_get(
                    &settings,
                    &format!("find/{}", meta.imdb_id),
                    &[("external_source", "imdb_id".into())],
                )?
            } else {
                self.tmdb_get(&settings, &format!("search/{kind}"), &query)?
            };
            let rows = &result[if imdb {
                if kind == "movie" {
                    "movie_results"
                } else {
                    "tv_results"
                }
            } else {
                "results"
            }];
            for v in rows.as_array().into_iter().flatten().take(20) {
                if let Some(id) = v["id"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| *n > 0)
                {
                    let title = text(v, if kind == "movie" { "title" } else { "name" }, 300);
                    if !title.is_empty() {
                        candidates.push(Candidate {
                            id,
                            kind: kind.into(),
                            title,
                            original_title: text(
                                v,
                                if kind == "movie" {
                                    "original_title"
                                } else {
                                    "original_name"
                                },
                                300,
                            ),
                            year: year(
                                v,
                                if kind == "movie" {
                                    "release_date"
                                } else {
                                    "first_air_date"
                                },
                            ),
                        });
                    }
                }
            }
            candidates.sort_by_key(|c| {
                std::cmp::Reverse(
                    (if fold(&title) == fold(&c.title) || fold(&title) == fold(&c.original_title) {
                        100
                    } else {
                        0
                    }) + similarity(&title, &c.title).max(similarity(&title, &c.original_title))
                        + if release == c.year && release.is_some() {
                            20
                        } else {
                            0
                        },
                )
            });
        }
        if !self.online_valid(&source, epoch) {
            bail!("La richiesta è stata annullata.");
        }
        if search.is_some() {
            return Ok(json!({"status":"suggestions","candidates":candidates}));
        }
        let exact = candidates
            .iter()
            .filter(|c| fold(&title) == fold(&c.title) || fold(&title) == fold(&c.original_title))
            .collect::<Vec<_>>();
        let selected = known.or_else(|| {
            if imdb && candidates.len() == 1 {
                return Some(candidates[0].id);
            }
            if let Some(year) = release {
                let by_year = exact
                    .iter()
                    .filter(|c| c.year == Some(year))
                    .collect::<Vec<_>>();
                if by_year.len() == 1 {
                    return Some(by_year[0].id);
                }
                let nearby = exact
                    .iter()
                    .filter(|c| c.year.is_some_and(|y| y.abs_diff(year) <= 1))
                    .collect::<Vec<_>>();
                return (nearby.len() == 1).then(|| nearby[0].id);
            }
            (exact.len() == 1 && title.chars().count() >= 4).then(|| exact[0].id)
        });
        let mut details = None;
        let cached = selected
            .and_then(|id| {
                db.query_row(
                    "SELECT value FROM details WHERE kind=? AND id=? AND expires>?",
                    params![kind, id, crate::report::now_ms()],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .ok()
                .flatten()
            })
            .and_then(|s| serde_json::from_str::<Details>(&s).ok());
        if let Some(cached) = cached {
            details = Some(cached);
        } else if let Some(id) = selected {
            let credits = if kind == "movie" {
                "credits"
            } else {
                "aggregate_credits"
            };
            let v = self.tmdb_get(
                &settings,
                &format!("{kind}/{id}"),
                &[("append_to_response", credits.into())],
            )?;
            if v["id"].as_u64() != Some(id as u64) {
                bail!("Risposta TMDB non valida.");
            }
            details = Some(Details {
                id,
                kind: kind.into(),
                title: text(&v, if kind == "movie" { "title" } else { "name" }, 300),
                year: year(
                    &v,
                    if kind == "movie" {
                        "release_date"
                    } else {
                        "first_air_date"
                    },
                ),
                description: text(&v, "overview", 4000),
                overview_language: "it".into(),
                genres: names(&v["genres"], 20),
                cast: names(&v[credits]["cast"], 12),
                directors: if kind == "tv" {
                    names(&v["created_by"], 6)
                } else {
                    v["credits"]["crew"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|c| c["job"] == "Director")
                        .map(|c| text(c, "name", 120))
                        .take(6)
                        .collect()
                },
                duration: if kind == "movie" {
                    v["runtime"].as_f64().unwrap_or(0.0).clamp(0.0, 1440.0) * 60.0
                } else {
                    0.0
                },
                poster: text(&v, "poster_path", 300),
                backdrop: text(&v, "backdrop_path", 300),
            });
            if let Some(details) = details.as_mut().filter(|d| d.description.is_empty()) {
                if let Ok(english) =
                    self.tmdb_get_language(&settings, &format!("{kind}/{id}"), &[], "en-US")
                {
                    details.description = text(&english, "overview", 4000);
                    details.overview_language = "en".into();
                }
            }
        }
        let _operation = self.operation.lock().expect("metadata publication");
        if !self.online_valid(&source, epoch) || !self.metadata_settings()?.enabled {
            bail!("La richiesta è stata annullata.");
        }
        let manual = choose.is_some() || old.as_ref().is_some_and(|o| o.4);
        db.execute(
            "INSERT OR REPLACE INTO matches VALUES(?,?,?,?,0,?,?,?)",
            params![
                source.id,
                key,
                fingerprint,
                selected,
                serde_json::to_string(&candidates)?,
                manual,
                crate::report::now_ms() + DAY
            ],
        )?;
        if let Some(details) = details {
            db.execute(
                "INSERT OR REPLACE INTO details VALUES(?,?,?,?)",
                params![
                    kind,
                    details.id,
                    serde_json::to_string(&details)?,
                    crate::report::now_ms() + 7 * DAY
                ],
            )?;
        }
        db.execute_batch("DELETE FROM details WHERE rowid NOT IN(SELECT rowid FROM details ORDER BY expires DESC LIMIT 5000); DELETE FROM matches WHERE manual=0 AND rowid NOT IN(SELECT rowid FROM matches ORDER BY expires DESC LIMIT 10000)")?;
        Ok(
            json!({"status":if selected.is_some(){"ready"}else if candidates.is_empty(){"not_found"}else{"ambiguous"},"candidates":candidates}),
        )
    }
    pub fn video_artwork(&self, id: &str, backdrop: bool) -> Result<(String, Vec<u8>)> {
        let (source, meta, key, fingerprint) = self.online_input(id)?;
        let local = if backdrop {
            &meta.backdrop
        } else {
            &meta.poster
        };
        let (_, entry) = self.entry(id)?;
        if !local.is_empty() {
            if let Ok(image) = self.logo_url_headers(&source, local, &entry.headers) {
                return Ok(image);
            }
        }
        let epoch = self.video_epoch.load(Ordering::Acquire);
        if self.metadata_settings().map(|s| s.enabled).unwrap_or(false) {
            if let Ok(Some(details)) = self.cached_details(
                &source.id,
                &key,
                &fingerprint,
                meta.online_kind().unwrap_or(""),
            ) {
                if let Some(url) = image_path(
                    if backdrop {
                        &details.backdrop
                    } else {
                        &details.poster
                    },
                    backdrop,
                ) {
                    let directory = self.directory.join("video-images");
                    fs::create_dir_all(&directory)?;
                    let path = directory.join(format!("{:x}", Sha256::digest(&url)));
                    if let Ok(bytes) = fs::read(&path) {
                        if bytes.len() <= 2 * 1024 * 1024 {
                            if let Some(mime) = mime(&bytes) {
                                return Ok((mime.into(), bytes));
                            }
                        }
                    }
                    let _slot = self.video_online.lock().expect("video artwork");
                    if !self.online_valid(&source, epoch) || !self.metadata_settings()?.enabled {
                        bail!("Immagine non disponibile.");
                    }
                    let response = client()?
                        .get(url)
                        .send()
                        .map_err(|_| anyhow!("Immagine non disponibile."))?;
                    if response.status().is_success() {
                        let mut bytes = vec![];
                        response
                            .take(2 * 1024 * 1024 + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|_| anyhow!("Immagine non disponibile."))?;
                        if bytes.len() <= 2 * 1024 * 1024 {
                            if let Some(mime) = mime(&bytes) {
                                let _operation =
                                    self.operation.lock().expect("artwork publication");
                                if self.online_valid(&source, epoch)
                                    && self.metadata_settings()?.enabled
                                {
                                    fs::write(&path, &bytes)?;
                                    private_file(&path)?;
                                    let mut files = fs::read_dir(&directory)?
                                        .filter_map(|f| f.ok())
                                        .filter_map(|f| {
                                            f.metadata()
                                                .ok()
                                                .map(|m| (f.path(), m.len(), m.modified().ok()))
                                        })
                                        .collect::<Vec<_>>();
                                    let mut total = files.iter().map(|f| f.1).sum::<u64>();
                                    files.sort_by_key(|f| f.2);
                                    for (p, n, _) in files {
                                        if total <= 48 * 1024 * 1024 {
                                            break;
                                        }
                                        if fs::remove_file(p).is_ok() {
                                            total = total.saturating_sub(n);
                                        }
                                    }
                                    return Ok((mime.into(), bytes));
                                }
                            }
                        }
                    }
                }
            }
        }
        self.logo_url_headers(&source, local, &entry.headers)
    }
}

impl Library {
    pub(in crate::personal) fn video_backup_choices(
        &self,
        sources: &std::collections::BTreeMap<String, Source>,
        owners: &mut serde_json::Map<String, Value>,
    ) -> Result<Vec<Value>> {
        let db = self.metadata_db()?;
        let mut statement = db.prepare(
            "SELECT source,key,selected,excluded FROM matches WHERE manual=1 LIMIT 5000",
        )?;
        let mut result = Vec::new();
        for row in statement.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<u32>>(2)?,
                r.get::<_, bool>(3)?,
            ))
        })? {
            let (source, key, id, excluded) = row?;
            let Some(s) = sources.get(&source) else {
                continue;
            };
            let source_db = open(&self.path(&source)?)?;
            let raw = if let Some(series) = key.strip_prefix("s:") {
                source_db.query_row("SELECT r.payload FROM video v JOIN records r ON r.id=v.id WHERE v.series=? ORDER BY v.position LIMIT 1",[series],|r|r.get::<_,String>(0)).optional()?
            } else {
                source_db
                    .query_row(
                        "SELECT payload FROM records WHERE id=?",
                        [key.strip_prefix("i:").unwrap_or("")],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?
            };
            let Some(raw) = raw else {
                continue;
            };
            let entry: Entry = serde_json::from_str(&raw)?;
            let meta = Metadata::from(&entry);
            let Some(kind) = meta.online_kind() else {
                continue;
            };
            let name = if meta.series.is_empty() {
                &meta.title
            } else {
                &meta.series_title
            };
            let title = clean(name);
            let release = if kind == "movie" {
                meta.year
            } else {
                meta.series_year
                    .or_else(|| YEAR.captures(name).and_then(|c| c[1].parse().ok()))
            };
            let fingerprint = super::super::wire::hash(&format!(
                "v1|{}|{}|{}|{}|{}",
                kind.to_uppercase(),
                fold(&title),
                release
                    .map(|y| y.to_string())
                    .unwrap_or_else(|| "null".into()),
                meta.tmdb_id
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "null".into()),
                meta.imdb_id
            ));
            let key = if key.starts_with("i:") {
                let id = super::super::wire::channel_id(key.trim_start_matches("i:"));
                owners.insert(id.clone(), json!(s.id));
                format!("i:{id}")
            } else {
                key
            };
            result.push(json!({"source":source,"key":key,"fingerprint":fingerprint,"type":kind.to_uppercase(),"id":id,"excluded":excluded}));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_paths_cannot_change_hosts_or_carry_tokens() {
        for path in [
            "//evil.org/a.jpg",
            "/../a.jpg",
            "/a.jpg?token=private",
            "https://evil.org/a.jpg",
            "",
        ] {
            assert!(image_path(path, false).is_none());
        }
        assert_eq!(
            image_path("/abc.jpg", false).unwrap(),
            "https://image.tmdb.org/t/p/w500/abc.jpg"
        );
        assert_eq!(clean("IT | Film (2020) [1080p ITA]"), "Film");
    }
    #[test]
    fn unicode_titles_generic_video_types_and_playlist_metadata_follow_android() {
        assert_eq!(fold("Amélie — L’Été"), "amelie l ete");
        assert!(
            similarity("La vita è bella", "La vita e bella")
                > similarity("La vita è bella", "Bella storia")
        );
        let f = crate::personal::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:600 type=\"video\" tmdb-type=\"tv\" year=\"2022\" series-year=\"2018\" plot=\"Trama della lista\" genre=\"Locale\",Titolo generico\nhttps://media.example/files/42.mp4\n",None);
        let rows = f.rows(&source);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        let (_, meta, key, fingerprint) = f.library.online_input(id).unwrap();
        assert_eq!(meta.kind, "video");
        assert_eq!(meta.online_kind(), Some("tv"));
        f.library
            .video_online_settings(&json!({"token":"fixture-private","enabled":true}))
            .unwrap();
        let db = f.library.metadata_db().unwrap();
        db.execute(
            "INSERT INTO matches VALUES(?,?,?,42,0,'[]',1,?)",
            params![source, key, fingerprint, u64::MAX / 2],
        )
        .unwrap();
        let details = Details {
            kind: "tv".into(),
            id: 42,
            year: Some(2018),
            description: "Online English plot".into(),
            overview_language: "en".into(),
            duration: 900.0,
            genres: vec!["Online".into()],
            directors: vec!["Un creatore".into()],
            ..Default::default()
        };
        db.execute(
            "INSERT INTO details VALUES('tv',42,?,?)",
            params![serde_json::to_string(&details).unwrap(), u64::MAX / 2],
        )
        .unwrap();
        let detail = f.library.video_detail(id, &[]).unwrap();
        let item = &detail["item"];
        assert_eq!(item["description"], "Trama della lista");
        assert_eq!(item["duration"], 600.0);
        assert_eq!(item["genres"], json!(["Locale"]));
        assert_eq!(item["year"], 2018);
        assert_eq!(item["directors"], json!(["Un creatore"]));
        assert_eq!(item["overview_language"], "");
    }
    #[test]
    fn cached_metadata_works_offline_and_cache_clear_preserves_manual_decisions() {
        let f = crate::personal::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:600 type=\"movie\",Un titolo\nhttps://media.example/film.mp4?private=hidden\n",None);
        let rows = f.rows(&source);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        let (_, _, key, fingerprint) = f.library.online_input(id).unwrap();
        let config = f
            .library
            .video_online_settings(&json!({"token":"fixture-private-token","enabled":true}))
            .unwrap();
        assert_eq!(config["configured"], true);
        assert!(!config.to_string().contains("fixture-private-token"));
        let db = f.library.metadata_db().unwrap();
        db.execute(
            "INSERT INTO matches VALUES(?,?,?,42,0,'[]',1,?)",
            params![source, key, fingerprint, crate::report::now_ms() + DAY],
        )
        .unwrap();
        let details = Details {
            id: 42,
            kind: "movie".into(),
            title: "Titolo online".into(),
            description: "Trama già scaricata".into(),
            poster: "/poster.jpg".into(),
            ..Default::default()
        };
        db.execute(
            "INSERT INTO details VALUES('movie',42,?,?)",
            params![
                serde_json::to_string(&details).unwrap(),
                crate::report::now_ms() + DAY
            ],
        )
        .unwrap();
        assert_eq!(
            f.library.video_metadata(id, None, None).unwrap()["status"],
            "ready"
        );
        assert_eq!(
            f.library.video_detail(id, &[]).unwrap()["item"]["description"],
            "Trama già scaricata"
        );
        f.library
            .video_online_settings(&json!({"enabled":false}))
            .unwrap();
        assert_eq!(
            f.library.video_metadata(id, None, None).unwrap()["status"],
            "disabled"
        );
        assert_eq!(
            f.library.video_detail(id, &[]).unwrap()["item"]["online"],
            false
        );
        f.library
            .video_online_settings(&json!({"enabled":true,"clear":true}))
            .unwrap();
        assert_eq!(
            db.query_row(
                "SELECT selected FROM matches WHERE source=?",
                [&source],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            42
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM details", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            f.library.video_metadata(id, None, Some(None)).unwrap()["status"],
            "excluded"
        );
        assert_eq!(
            f.library.video_metadata(id, None, None).unwrap()["status"],
            "excluded"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(f.library.directory.join("video-settings.json"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[test]
    #[ignore = "Requires LUME_TMDB_READ_TOKEN and explicit public TMDB network access"]
    fn configured_tmdb_resolves_a_public_movie_and_caches_real_artwork() {
        assert!(
            std::env::var("LUME_TMDB_READ_TOKEN").is_ok_and(|s| !s.is_empty()),
            "Configure the local TMDB read token"
        );
        let f = crate::personal::tests::Fixture::new();
        let source=f.import("#EXTM3U\n#EXTINF:7200 type=\"movie\" year=\"1977\",Star Wars (1977)\nhttps://media.example/movie/fixture/private/1.mp4\n",None);
        let rows = f.rows(&source);
        let id = rows["rows"][0]["id"].as_str().unwrap();
        f.library
            .video_online_settings(&json!({"enabled":true}))
            .unwrap();
        assert!(f.library.metadata_settings().unwrap().token.is_empty());
        assert_eq!(
            f.library.video_metadata(id, None, None).unwrap()["status"],
            "ready"
        );
        let details = f.library.video_detail(id, &[]).unwrap();
        assert_eq!(details["item"]["online"], true);
        assert!(!details["item"]["description"].as_str().unwrap().is_empty());
        assert!(!details["item"]["cast"].as_array().unwrap().is_empty());
        let (mime, bytes) = f.library.video_artwork(id, false).unwrap();
        assert!(mime.starts_with("image/"));
        assert!(bytes.len() > 1000);
        assert_eq!(f.library.video_artwork(id, false).unwrap().1, bytes);
        eprintln!(
            "TMDB public fixture: details=ready, artwork_bytes={}, cached_artwork=verified",
            bytes.len()
        );
    }
}
