// SPDX-License-Identifier: GPL-3.0-or-later
//! VOD presentation and continuity, adapted from Android's VideoCatalog/VodMetadata.
//! Playback identities and resource policies remain owned by the personal library.
mod online;
#[cfg(test)]
mod tests;

use super::{store::open, Entry, Library, Source};
use anyhow::{bail, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::LazyLock;

static EPISODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(.+?)[ ._\-]+(?:[sS](\d{1,2})[ ._\-]*[eE](\d{1,3})|(\d{1,2})[xX](\d{1,3}))(?:\b|_)(.*)$",
    )
    .unwrap()
});
static YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[ (\[])(19\d{2}|20\d{2})(?:[)\] ]|$)").unwrap());
static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]{0,200}>").unwrap());

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Metadata {
    title: String,
    pub(super) kind: String,
    series: String,
    series_title: String,
    season: Option<u32>,
    episode: Option<u32>,
    year: Option<u32>,
    duration: f64,
    description: String,
    genres: Vec<String>,
    groups: Vec<String>,
    poster: String,
    backdrop: String,
    tmdb_id: Option<u32>,
    tmdb_type: String,
    imdb_id: String,
    series_year: Option<u32>,
}
fn value(entry: &Entry, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|key| {
            entry
                .metadata
                .attributes
                .get(*key)
                .map(|v| v.trim())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or("")
        .to_owned()
}
pub(super) fn classify(entry: &Entry, settings: &super::Settings) -> &'static str {
    let choices = settings
        .group_content
        .iter()
        .filter(|(name, _)| {
            entry
                .groups
                .iter()
                .any(|g| g.to_lowercase() == name.to_lowercase())
        })
        .map(|(_, kind)| kind.as_str())
        .collect::<Vec<_>>();
    if choices.contains(&"live") {
        return "live";
    }
    if choices.contains(&"vod") {
        return "vod";
    }
    for key in ["type", "content-type", "media-type"] {
        match value(entry, &[key]).to_lowercase().as_str() {
            "live" | "linear" => return "live",
            "vod" | "video" | "movie" | "movies" | "film" | "films" | "series" | "episode"
            | "episodes" | "tvshow" => return "vod",
            _ => {}
        }
    }
    match value(entry, &["media"]).to_lowercase().as_str() {
        "true" | "1" | "yes" => return "vod",
        "false" | "0" | "no" => return "live",
        _ => {}
    }
    if entry
        .metadata
        .properties
        .get("lumetv.vod_marker")
        .is_some_and(|v| v == "true")
    {
        return "vod";
    }
    if !value(entry, &["media-dir"]).is_empty()
        || value(entry, &["media-size"]).parse::<u64>().unwrap_or(0) > 0
        || entry.metadata.duration > 0.0
    {
        return "vod";
    }
    if let Ok(url) = reqwest::Url::parse(&entry.url) {
        let path = url.path().to_lowercase();
        let segments = path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>();
        static NUMERIC: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r"^\d+(?:\.[a-z0-9]{1,8})?$").unwrap());
        if segments.len() >= 4
            && ["movie", "series"].contains(&segments[segments.len() - 4])
            && NUMERIC.is_match(segments.last().unwrap())
            && !segments
                .iter()
                .any(|s| ["live", "timeshift", "catchup", "archive"].contains(s))
        {
            return "vod";
        }
        if entry
            .metadata
            .properties
            .get("lumetv.legacy_vod")
            .is_some_and(|v| v == "true")
        {
            return "vod";
        }
        if settings.vod_detection == "extended"
            && !segments
                .iter()
                .any(|s| ["live", "timeshift", "catchup", "archive"].contains(s))
            && value(entry, &["catchup", "catchup-type", "timeshift"]).is_empty()
            && segments.last().is_some_and(|s| {
                s.rsplit_once('.').is_some_and(|(_, ext)| {
                    ["mp4", "mkv", "avi", "mov", "m4v", "webm", "mpg", "mpeg"].contains(&ext)
                })
            })
            && entry.groups.iter().any(|g| {
                let words = normalized(g);
                let tokens = words.split_whitespace().collect::<Vec<_>>();
                tokens.iter().any(|w| {
                    [
                        "vod", "film", "films", "movie", "movies", "serie", "series", "episodi",
                        "episodes",
                    ]
                    .contains(w)
                }) && !tokens
                    .iter()
                    .any(|w| ["live", "diretta", "dirette", "24h"].contains(w))
            })
        {
            return "vod";
        }
    }
    "live"
}
fn normalized(value: &str) -> String {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    value
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Reclassify saved records locally when reading rules change; stream URLs are never probed.
pub(super) fn ensure_content(db: &Connection, settings: &super::Settings) -> Result<()> {
    let version = serde_json::to_string(&(1, &settings.vod_detection, &settings.group_content))?;
    if db
        .query_row(
            "SELECT value FROM metadata WHERE key='content_rules'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        == Some(&version)
    {
        return Ok(());
    }
    db.execute_batch("BEGIN IMMEDIATE")?;
    let result = (|| -> Result<()> {
        let mut cursor = 0_i64;
        loop {
            let mut select = db.prepare(
                "SELECT rowid,payload,content FROM records WHERE rowid>? ORDER BY rowid LIMIT 200",
            )?;
            let rows = select
                .query_map([cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if rows.is_empty() {
                break;
            }
            for (row, payload, old) in rows {
                cursor = row;
                let entry: Entry = serde_json::from_str(&payload)?;
                let content = classify(&entry, settings);
                if content != old {
                    db.execute(
                        "UPDATE records SET content=? WHERE rowid=?",
                        params![content, row],
                    )?;
                }
            }
        }
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES('content_rules',?)",
            [&version],
        )?;
        db.execute("DELETE FROM metadata WHERE key='video_index'", [])?;
        Ok(())
    })();
    db.execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" })?;
    result
}
impl Metadata {
    fn online_kind(&self) -> Option<&str> {
        if self.kind == "movie" {
            Some("movie")
        } else if !self.series.is_empty() {
            Some("tv")
        } else if ["movie", "tv"].contains(&self.tmdb_type.as_str()) {
            Some(&self.tmdb_type)
        } else {
            None
        }
    }
    fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!(
                "{}:{}|{}|{:?}|{:?}|{}",
                self.kind,
                self.tmdb_type,
                if self.series.is_empty() {
                    &self.title
                } else {
                    &self.series_title
                },
                if self.series.is_empty() {
                    self.year
                } else {
                    self.series_year
                },
                self.tmdb_id,
                self.imdb_id
            ))
        )
    }
    pub(super) fn from(entry: &Entry) -> Self {
        let capture = EPISODE.captures(&entry.name);
        let mut series_title = value(
            entry,
            &["series-title", "series-name", "show-title", "tvg-series"],
        );
        if series_title.is_empty() {
            series_title = capture
                .as_ref()
                .map(|c| c[1].trim().to_owned())
                .unwrap_or_default();
        }
        series_title = series_title.chars().take(300).collect();
        let number = |names: &[&str]| {
            value(entry, names)
                .parse::<u32>()
                .ok()
                .filter(|n| *n <= 9999)
        };
        let named_number = |a, b| {
            capture
                .as_ref()
                .and_then(|c| c.get(a).or_else(|| c.get(b)))
                .and_then(|n| n.as_str().parse::<u32>().ok())
        };
        let season = number(&["season", "season-number"]).or_else(|| named_number(2, 4));
        let episode = number(&["episode", "episode-number"]).or_else(|| named_number(3, 5));
        let movie = ["movie", "movies", "film", "films"];
        let episodes = ["series", "episode", "episodes", "tvshow"];
        let types = ["type", "content-type", "media-type"]
            .iter()
            .map(|k| value(entry, &[*k]).to_lowercase())
            .collect::<Vec<_>>();
        let segments = reqwest::Url::parse(&entry.url)
            .ok()
            .map(|u| u.path().to_lowercase())
            .unwrap_or_default();
        let words = entry
            .groups
            .iter()
            .flat_map(|g| g.split(|c: char| !c.is_alphanumeric()))
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        let kind = if types.iter().any(|t| movie.contains(&t.as_str())) {
            "movie"
        } else if !series_title.is_empty()
            || types.iter().any(|t| episodes.contains(&t.as_str()))
            || segments.split('/').any(|s| s == "series")
        {
            "episode"
        } else if segments.split('/').any(|s| s == "movie") {
            "movie"
        } else if words
            .iter()
            .any(|w| ["serie", "series", "tvshow"].contains(&w.as_str()))
        {
            "episode"
        } else if words.iter().any(|w| movie.contains(&w.as_str())) {
            "movie"
        } else {
            "video"
        };
        let mut series = String::new();
        if kind == "episode" && !series_title.is_empty() {
            let id = value(entry, &["series-id", "show-id"]);
            let key = if id.is_empty() {
                format!(
                    "name:{}|{}",
                    series_title
                        .to_lowercase()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" "),
                    value(entry, &["series-year"])
                )
            } else {
                format!("id:{id}")
            };
            series = format!("{:x}", Sha256::digest(key));
        }
        let image = |names: &[&str]| {
            let raw = value(entry, names);
            reqwest::Url::parse(&raw)
                .or_else(|_| reqwest::Url::parse(&entry.url)?.join(&raw))
                .ok()
                .filter(|u| ["http", "https"].contains(&u.scheme()) && !raw.is_empty())
                .map(|u| u.to_string())
                .unwrap_or_default()
        };
        let mut poster = image(&["poster", "poster-url", "tvg-poster", "cover", "cover-url"]);
        if poster.is_empty() {
            poster = entry.logo.clone();
        }
        let mut genres = vec![];
        for genre in value(entry, &["genre", "genres"])
            .split([';', ','])
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .take(20)
        {
            if !genres.iter().any(|g| g == genre) {
                genres.push(genre.chars().take(100).collect());
            }
        }
        Self {
            title: entry.name.clone(),
            kind: kind.into(),
            series,
            series_title,
            season,
            episode,
            year: number(&["year", "release-year"])
                .filter(|n| (1888..=2100).contains(n))
                .or_else(|| YEAR.captures(&entry.name).and_then(|c| c[1].parse().ok())),
            duration: if entry.metadata.duration.is_finite() {
                entry.metadata.duration.max(0.0)
            } else {
                0.0
            },
            description: TAG
                .replace_all(
                    &value(entry, &["plot", "description", "tvg-description"]),
                    "",
                )
                .chars()
                .take(4000)
                .collect(),
            genres,
            groups: entry.groups.clone(),
            poster,
            backdrop: image(&[
                "backdrop",
                "backdrop-url",
                "fanart",
                "fanart-url",
                "tvg-background",
            ]),
            tmdb_id: value(entry, &["tmdb-id", "tmdb_id"])
                .parse()
                .ok()
                .filter(|n| *n > 0),
            imdb_id: value(entry, &["imdb-id", "imdb_id"]),
            tmdb_type: value(entry, &["tmdb-type"]).to_lowercase(),
            series_year: number(&["series-year"]).filter(|y| (1888..=2100).contains(y)),
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Query {
    pub source: String,
    pub group: String,
    pub query: String,
    pub kind: String,
    pub shelf: String,
    pub order: String,
    pub series: String,
    pub season: Option<u32>,
    pub episodes: bool,
    pub offset: usize,
    pub limit: usize,
}
#[derive(Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub source: String,
    pub title: String,
    pub kind: String,
    pub series: String,
    pub series_title: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub year: Option<u32>,
    pub duration: f64,
    pub description: String,
    pub genres: Vec<String>,
    pub groups: Vec<String>,
    pub added: u64,
    pub episodes: usize,
    pub position: f64,
    pub watched_duration: f64,
    pub completed: bool,
    pub hidden: bool,
    pub saved: bool,
    pub poster: bool,
    pub backdrop: bool,
    pub cast: Vec<String>,
    pub directors: Vec<String>,
    pub overview_language: String,
    pub online: bool,
    #[serde(skip)]
    lookup: String,
    #[serde(rename = "metadata_type")]
    lookup_kind: String,
}
impl Item {
    fn from(id: String, source: String, meta: Metadata, added: u64, episodes: usize) -> Self {
        let lookup = meta.fingerprint();
        let lookup_kind = meta.online_kind().unwrap_or("").to_owned();
        Self {
            id,
            source,
            title: meta.title,
            kind: meta.kind,
            series: meta.series,
            series_title: meta.series_title,
            season: meta.season,
            episode: meta.episode,
            year: meta.year,
            duration: meta.duration,
            description: meta.description,
            genres: meta.genres,
            groups: meta.groups,
            added,
            episodes,
            position: 0.0,
            watched_duration: 0.0,
            completed: false,
            hidden: false,
            saved: false,
            poster: !meta.poster.is_empty(),
            backdrop: !meta.backdrop.is_empty(),
            cast: vec![],
            directors: vec![],
            overview_language: String::new(),
            online: false,
            lookup,
            lookup_kind,
        }
    }
    fn key(&self) -> String {
        if self.series.is_empty() {
            format!("i:{}", self.id)
        } else {
            format!("s:{}", self.series)
        }
    }
}

/// Bounded cursor migration. A failed/cancelled rebuild rolls back and leaves the old index intact.
pub(super) fn index(
    db: &Connection,
    content: &str,
    added: u64,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let version = format!("4:{content}");
    db.execute_batch("CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT)")?;
    if db
        .query_row(
            "SELECT value FROM metadata WHERE key='video_index'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        == Some(&version)
    {
        return Ok(());
    }
    check()?;
    db.execute_batch("PRAGMA max_page_count=262144; BEGIN IMMEDIATE;
        CREATE TABLE IF NOT EXISTS video_added(id TEXT PRIMARY KEY,added INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS video(id TEXT PRIMARY KEY,added INTEGER NOT NULL);
        INSERT OR IGNORE INTO video_added SELECT id,added FROM video;
        DROP TABLE IF EXISTS video; DROP TABLE IF EXISTS video_titles;
        CREATE TABLE video(id TEXT PRIMARY KEY,kind TEXT NOT NULL,series TEXT NOT NULL,series_title TEXT NOT NULL,season INTEGER,episode INTEGER,title TEXT NOT NULL,sort_title TEXT NOT NULL,position INTEGER NOT NULL,added INTEGER NOT NULL);
        CREATE TABLE video_titles(group_name TEXT NOT NULL,key TEXT NOT NULL,id TEXT NOT NULL,kind TEXT NOT NULL,title TEXT NOT NULL,position INTEGER NOT NULL,added INTEGER NOT NULL,episodes INTEGER NOT NULL,PRIMARY KEY(group_name,key));")?;
    let result = (|| -> Result<()> {
        let filter = if content == "vod" {
            ""
        } else if content == "live" {
            " AND 0"
        } else {
            " AND content='vod'"
        };
        let mut cursor = 0_i64;
        loop {
            check()?;
            // A secondary content index would sort/rescan the remaining catalog for every batch.
            // NOT INDEXED still permits the primary rowid seek and bounds each cursor window.
            let mut select = db.prepare(&format!("SELECT rowid,id,payload,position,seen FROM records NOT INDEXED WHERE rowid>? AND active=1 AND kind='tv'{filter} ORDER BY rowid LIMIT 200"))?;
            let batch = select
                .query_map([cursor], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, usize>(3)?,
                        r.get::<_, u64>(4)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if batch.is_empty() {
                break;
            }
            let mut insert = db.prepare("INSERT INTO video VALUES(?,?,?,?,?,?,?,?,?,?)")?;
            let mut date_lookup = db.prepare("SELECT added FROM video_added WHERE id=?")?;
            for (rowid, id, payload, position, seen) in batch {
                cursor = rowid;
                let mut entry: Entry = serde_json::from_str(&payload)?;
                entry.id = id.clone();
                entry.position = position;
                let meta = Metadata::from(&entry);
                let date = date_lookup
                    .query_row([&id], |r| r.get::<_, u64>(0))
                    .optional()?
                    .unwrap_or(if seen == 0 { added } else { seen });
                let title = if meta.series.is_empty() {
                    &meta.title
                } else {
                    &meta.series_title
                };
                let sort = title.to_lowercase();
                insert.execute(params![
                    id,
                    meta.kind,
                    meta.series,
                    meta.series_title,
                    meta.season,
                    meta.episode,
                    meta.title,
                    sort,
                    position,
                    date
                ])?;
            }
        }
        check()?;
        db.execute_batch("CREATE INDEX video_series ON video(series,season,episode,position,id); CREATE UNIQUE INDEX video_position ON video(position);
            WITH members AS (
                SELECT '' AS group_name,id,position,added,CASE WHEN series='' THEN 'i:'||id ELSE 's:'||series END AS key FROM video
                UNION ALL SELECT g.name,m.id,m.position,m.added,CASE WHEN m.series='' THEN 'i:'||m.id ELSE 's:'||m.series END FROM groups g JOIN video m ON m.position=g.position WHERE g.name<>''
            ), titles AS (SELECT group_name,key,MIN(position) AS position,MAX(added) AS added,COUNT(*) AS episodes FROM members GROUP BY group_name,key)
            INSERT INTO video_titles SELECT t.group_name,t.key,v.id,v.kind,v.sort_title,t.position,t.added,t.episodes FROM titles t JOIN video v ON v.position=t.position;
            CREATE INDEX video_title_position ON video_titles(group_name,kind,position); CREATE INDEX video_title_sort ON video_titles(group_name,title,position); CREATE INDEX video_title_added ON video_titles(group_name,added DESC,position);
            DROP TABLE video_added;")?;
        db.execute(
            "INSERT OR REPLACE INTO metadata VALUES('video_index',?)",
            [&version],
        )?;
        Ok(())
    })();
    if result.is_ok() {
        db.execute_batch("COMMIT")?;
    } else {
        let _ = db.execute_batch("ROLLBACK");
    }
    result
}

fn pattern(value: &str) -> String {
    format!(
        "%{}%",
        value
            .chars()
            .take(300)
            .collect::<String>()
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}
impl Library {
    fn video_connection(&self, sources: &[Source], favorites: &[String]) -> Result<Connection> {
        let history = self.watch_db()?;
        drop(history);
        let db = open(&self.directory.join("watch.sqlite"))?;
        db.execute_batch("PRAGMA temp_store=FILE; PRAGMA cache_size=-4096; CREATE TEMP TABLE favorites(id TEXT PRIMARY KEY)")?;
        for id in favorites {
            db.execute("INSERT OR IGNORE INTO favorites VALUES(?)", [id])?;
        }
        for (i, source) in sources.iter().enumerate() {
            let source_db = open(&self.path(&source.id)?)?;
            ensure_content(&source_db, &source.settings)?;
            index(
                &source_db,
                &source.settings.content,
                source.updated_at,
                &|| Ok(()),
            )?;
            drop(source_db);
            db.execute(
                &format!("ATTACH DATABASE ? AS v{i}"),
                [self.path(&source.id)?.to_string_lossy().as_ref()],
            )?;
        }
        Ok(db)
    }
    fn video_page_locked(&self, q: &Query, favorites: &[String]) -> Result<Value> {
        if !["", "movie", "episode", "video"].contains(&q.kind.as_str())
            || !["", "all", "continue", "saved", "added"].contains(&q.shelf.as_str())
            || !["", "playlist", "title", "added"].contains(&q.order.as_str())
        {
            bail!("Filtri video non validi.");
        }
        let sources = self
            .selected(&q.source)
            .into_iter()
            .filter(|s| s.settings.kind != "radio" && s.settings.content != "live")
            .collect::<Vec<_>>();
        if sources.is_empty() {
            return Ok(json!({"rows":[],"total":0}));
        }
        let db = self.video_connection(&sources, favorites)?;
        let collapse = !q.episodes && q.shelf != "continue";
        let static_page = collapse
            && q.query.is_empty()
            && q.series.is_empty()
            && q.season.is_none()
            && q.shelf != "saved";
        let mut selects = vec![];
        let mut args = vec![];
        for (i, source) in sources.iter().enumerate() {
            let schema = format!("v{i}");
            if static_page {
                let mut filter = vec!["group_name=?".to_owned()];
                args.push(q.group.clone());
                if !q.kind.is_empty() {
                    filter.push("kind=?".into());
                    args.push(q.kind.clone());
                }
                selects.push(format!("SELECT '{}' AS source,id,position AS first_position,episodes,title AS sort_title,added,0 AS updated,0 AS season,0 AS episode FROM {schema}.video_titles WHERE {}",source.id,filter.join(" AND ")));
                continue;
            }
            let from = if static_page {
                format!("{schema}.video_titles t JOIN {schema}.video m ON m.id=t.id")
            } else {
                format!("{schema}.video m")
            };
            let key = "CASE WHEN m.series='' THEN 'i:'||m.id ELSE 's:'||m.series END";
            let mut clauses = vec!["1".to_owned()];
            if static_page {
                clauses.push("t.group_name=?".into());
                args.push(q.group.clone());
            } else if !q.group.is_empty() {
                clauses.push(format!(
                    "m.position IN(SELECT position FROM {schema}.groups WHERE name=?)"
                ));
                args.push(q.group.clone());
            }
            if !q.kind.is_empty() {
                clauses.push("m.kind=?".into());
                args.push(q.kind.clone());
            }
            if !q.series.is_empty() {
                clauses.push("m.series=?".into());
                args.push(q.series.clone());
            }
            if let Some(season) = q.season {
                clauses.push("COALESCE(m.season,0)=CAST(? AS INTEGER)".into());
                args.push(season.to_string());
            }
            if !q.query.is_empty() {
                clauses.push(
                    "(m.title LIKE ? ESCAPE '\\' OR m.series_title LIKE ? ESCAPE '\\')".into(),
                );
                args.push(pattern(&q.query));
                args.push(pattern(&q.query));
            }
            let watched = format!("LEFT JOIN main.watch w ON w.source='{}' AND w.channel=m.id AND w.mode='vod' AND w.event=''",source.id);
            if q.shelf == "continue" {
                clauses.push("w.position>=5 AND w.completed=0 AND w.hidden=0".into());
            }
            if q.shelf == "saved" {
                clauses.push(format!("(m.id IN(SELECT id FROM favorites) OR EXISTS(SELECT 1 FROM main.video_saved s WHERE s.source='{}' AND s.key={key}))",source.id));
            }
            let group = if collapse && !static_page {
                format!(" GROUP BY {key}")
            } else {
                String::new()
            };
            // MIN(position) selects the representative row consistently, including arbitrary search windows.
            let pos = if collapse && !static_page {
                "MIN(m.position)"
            } else if static_page {
                "t.position"
            } else {
                "m.position"
            };
            let count = if static_page {
                "t.episodes"
            } else if collapse {
                "COUNT(*)"
            } else {
                "1"
            };
            let added = if static_page { "t.added" } else { "m.added" };
            selects.push(format!("SELECT '{}' AS source,m.id,{pos} AS first_position,{count} AS episodes,m.sort_title,{added} AS added,COALESCE(w.updated,0) AS updated,COALESCE(m.season,0) AS season,COALESCE(m.episode,0) AS episode FROM {from} {watched} WHERE {}{group}",source.id,clauses.join(" AND ")));
        }
        let union = selects.join(" UNION ALL ");
        let total: usize = db.query_row(
            &format!("SELECT COUNT(*) FROM ({union})"),
            rusqlite::params_from_iter(&args),
            |r| r.get(0),
        )?;
        let order = if q.shelf == "continue" {
            "updated DESC,source,id"
        } else if !q.series.is_empty() {
            "season,episode,first_position,id"
        } else if q.order == "title" {
            "sort_title,source,first_position,id"
        } else if q.order == "added" || q.shelf == "added" {
            "added DESC,source,first_position,id"
        } else {
            "source,first_position,id"
        };
        let limit = if q.limit == 0 {
            50
        } else {
            q.limit.clamp(1, 200)
        };
        let mut statement = db.prepare(&format!("SELECT source,id,episodes,added FROM ({union}) ORDER BY {order} LIMIT {limit} OFFSET {}",q.offset.min(super::MAX_TOTAL)))?;
        let selected = statement
            .query_map(rusqlite::params_from_iter(&args), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, usize>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut rows = vec![];
        for (source, id, episodes, added) in selected {
            let i = sources.iter().position(|s| s.id == source).unwrap();
            let raw: String = db.query_row(
                &format!("SELECT payload FROM v{i}.records WHERE id=?"),
                [&id],
                |r| r.get(0),
            )?;
            let mut item = Item::from(
                id,
                source,
                Metadata::from(&serde_json::from_str::<Entry>(&raw)?),
                added,
                episodes,
            );
            self.decorate_video(&db, &mut item, favorites)?;
            rows.push(item);
        }
        Ok(json!({"rows":rows,"total":total}))
    }
    fn decorate_video(&self, db: &Connection, item: &mut Item, favorites: &[String]) -> Result<()> {
        if let Some((position,duration,completed,hidden)) = db.query_row("SELECT position,duration,completed,hidden FROM main.watch WHERE source=? AND channel=? AND mode='vod' AND event=''",params![item.source,item.id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()? {
            item.position=position; item.watched_duration=duration; item.completed=completed; item.hidden=hidden;
        }
        item.saved = favorites.contains(&item.id)
            || db
                .query_row(
                    "SELECT 1 FROM main.video_saved WHERE source=? AND key=?",
                    params![item.source, item.key()],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
        self.online_cached(item)?;
        Ok(())
    }
    pub fn videos(&self, q: &Query, favorites: &[String]) -> Result<Value> {
        let _operation = self.operation.lock().expect("video operation");
        self.video_page_locked(q, favorites)
    }
    pub fn video_home(&self, source: &str, favorites: &[String]) -> Result<Value> {
        let _operation = self.operation.lock().expect("video operation");
        let mut shelves = vec![];
        for (id, title, kind, shelf) in [
            ("continue", "Continua a guardare", "", "continue"),
            ("saved", "La mia lista", "", "saved"),
            ("movies", "Film", "movie", ""),
            ("series", "Serie", "episode", ""),
            ("added", "Aggiunti di recente", "", "added"),
            ("all", "Dalle tue liste", "", ""),
        ] {
            let query = Query {
                source: source.into(),
                kind: kind.into(),
                shelf: shelf.into(),
                limit: 12,
                ..Default::default()
            };
            let page = self.video_page_locked(&query, favorites)?;
            if page["total"].as_u64().unwrap_or(0) > 0 {
                shelves.push(json!({"id":id,"title":title,"kind":kind,"shelf":shelf,"rows":page["rows"],"total":page["total"]}));
            }
        }
        Ok(json!({"shelves":shelves}))
    }
    pub fn video_detail(&self, id: &str, favorites: &[String]) -> Result<Value> {
        let (source, entry) = self.entry(id)?;
        if entry.display(&source.id, &source.settings).content != "vod" {
            bail!("Questo contenuto non è un video.");
        }
        let _operation = self.operation.lock().expect("video operation");
        self.source(&source.id)?;
        let db = self.video_connection(std::slice::from_ref(&source), favorites)?;
        let meta = Metadata::from(&entry);
        let added = db.query_row("SELECT added FROM v0.video WHERE id=?", [id], |r| r.get(0))?;
        let episodes = if meta.series.is_empty() {
            1
        } else {
            db.query_row(
                "SELECT COUNT(*) FROM v0.video WHERE series=?",
                [&meta.series],
                |r| r.get(0),
            )?
        };
        let mut item = Item::from(id.into(), source.id.clone(), meta, added, episodes);
        self.decorate_video(&db, &mut item, favorites)?;
        let mut seasons = vec![];
        let mut target = id.to_owned();
        if !item.series.is_empty() {
            seasons=db.prepare("SELECT COALESCE(season,0),COUNT(*) FROM v0.video WHERE series=? GROUP BY COALESCE(season,0) ORDER BY COALESCE(season,0)")?.query_map([&item.series],|r| Ok(json!({"season":r.get::<_,u32>(0)?,"episodes":r.get::<_,usize>(1)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
            target=db.query_row("SELECT m.id FROM v0.video m LEFT JOIN main.watch w ON w.source=? AND w.channel=m.id AND w.mode='vod' AND w.event='' WHERE m.series=? ORDER BY CASE WHEN w.position>=5 AND w.completed=0 AND w.hidden=0 THEN 0 WHEN COALESCE(w.completed,0)=0 THEN 1 ELSE 2 END,CASE WHEN w.position>=5 AND w.completed=0 AND w.hidden=0 THEN w.updated ELSE 0 END DESC,COALESCE(m.season,0),COALESCE(m.episode,0),m.position,m.id LIMIT 1",params![source.id,item.series],|r|r.get(0))?;
        }
        let next = self.next_video_locked(&db, &source.id, id)?;
        let raw: String = db.query_row(
            "SELECT payload FROM v0.records WHERE id=?",
            [&target],
            |r| r.get(0),
        )?;
        let mut target_item = Item::from(
            target.clone(),
            source.id,
            Metadata::from(&serde_json::from_str::<Entry>(&raw)?),
            added,
            1,
        );
        self.decorate_video(&db, &mut target_item, favorites)?;
        Ok(
            json!({"item":item,"target":target,"target_item":target_item,"next":next,"seasons":seasons}),
        )
    }
    fn next_video_locked(&self, db: &Connection, source: &str, id: &str) -> Result<Option<String>> {
        // Known episode numbers skip duplicate encodings of the same episode.
        Ok(db.query_row("SELECT n.id FROM v0.video c JOIN v0.video n ON n.series=c.series AND n.series<>'' AND n.kind='episode' WHERE c.id=? AND (COALESCE(n.season,0)>COALESCE(c.season,0) OR COALESCE(n.season,0)=COALESCE(c.season,0) AND (COALESCE(n.episode,0)>COALESCE(c.episode,0) OR (c.episode IS NULL OR n.episode IS NULL) AND (n.position>c.position OR n.position=c.position AND n.id>c.id))) ORDER BY COALESCE(n.season,0),COALESCE(n.episode,0),n.position,n.id LIMIT 1",[id],|r|r.get::<_,String>(0)).optional()?.filter(|_|self.sources.lock().expect("sources").get(source).is_some_and(|s| s.enabled)))
    }
    pub fn video_action(&self, id: &str, action: &str, enabled: bool) -> Result<Value> {
        let (source, entry) = self.entry(id)?;
        if entry.display(&source.id, &source.settings).content != "vod" {
            bail!("Questo contenuto non è un video.");
        }
        let _operation = self.operation.lock().expect("video operation");
        let current = self.source(&source.id)?;
        if current.revision != source.revision {
            bail!("La lista è cambiata. Riapri il video.");
        }
        let db = self.watch_db()?;
        match action {
            "saved" => {
                let meta = Metadata::from(&entry);
                let key = if meta.series.is_empty() {
                    format!("i:{id}")
                } else {
                    format!("s:{}", meta.series)
                };
                if enabled {
                    db.execute(
                        "INSERT OR IGNORE INTO video_saved VALUES(?,?)",
                        params![source.id, key],
                    )?;
                } else {
                    db.execute(
                        "DELETE FROM video_saved WHERE source=? AND key=?",
                        params![source.id, key],
                    )?;
                }
            }
            "completed" | "hidden" => {
                db.execute("INSERT OR IGNORE INTO watch(source,channel,mode,event,position,duration,completed,updated) VALUES(?,?,'vod','',0,?,0,?)",params![source.id,id,entry.metadata.duration.max(0.0),crate::report::now_ms()])?;
                if action == "completed" {
                    db.execute("UPDATE watch SET completed=?,position=CASE WHEN ?=0 THEN 0 ELSE position END,hidden=0,revision=revision+1,updated=? WHERE source=? AND channel=? AND mode='vod' AND event=''",params![enabled,enabled,crate::report::now_ms(),source.id,id])?;
                } else {
                    db.execute("UPDATE watch SET hidden=?,revision=revision+1 WHERE source=? AND channel=? AND mode='vod' AND event=''",params![enabled,source.id,id])?;
                }
                db.execute_batch("DELETE FROM watch WHERE rowid NOT IN(SELECT rowid FROM watch ORDER BY updated DESC LIMIT 5000)")?;
            }
            _ => bail!("Azione video non valida."),
        }
        Ok(json!({"saved":true}))
    }
}
