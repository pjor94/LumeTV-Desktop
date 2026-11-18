// SPDX-License-Identifier: GPL-3.0-or-later
use super::{
    catchup,
    store::{open, Library},
    Headers, Resource, Source,
};
use anyhow::{anyhow, bail, Result};
use chrono::{FixedOffset, NaiveDate, TimeZone};
use quick_xml::{
    events::{BytesStart, Event},
    Reader,
};
use reqwest::Method;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, BufRead, BufReader, Read},
    path::PathBuf,
    sync::Arc,
    thread,
};

pub struct GuideEvent {
    pub programme: crate::guide::Programme,
    pub catchup_id: String,
}
#[derive(Debug)]
struct GuideRetry(u64);
impl std::fmt::Display for GuideRetry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Il servizio della guida richiede una pausa. La copia salvata resta disponibile.",
        )
    }
}
impl std::error::Error for GuideRetry {}
fn now() -> u64 {
    crate::report::now_ms()
}
fn attr(e: &BytesStart<'_>, key: &[u8]) -> String {
    e.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|s| s.into_owned()))
        .unwrap_or_default()
}
struct Limited {
    input: Box<dyn Read + Send>,
    left: u64,
}
impl Read for Limited {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let n = self.input.read(out)?;
        if n as u64 > self.left {
            return Err(io::Error::other("Guida oltre i limiti previsti."));
        }
        self.left -= n as u64;
        Ok(n)
    }
}
fn decoded(input: Box<dyn Read + Send>) -> Result<Box<dyn Read + Send>> {
    let mut input = BufReader::new(Limited {
        input,
        left: 32 * 1024 * 1024,
    });
    let magic = input.fill_buf()?;
    let compressed: Box<dyn Read + Send> = if magic.starts_with(&[0x1f, 0x8b]) {
        Box::new(flate2::read::GzDecoder::new(input))
    } else if magic.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0]) {
        Box::new(lzma_rust2::XzReader::new_mem_limit(input, false, 64 * 1024))
    } else {
        Box::new(input)
    };
    Ok(Box::new(
        encoding_rs_io::DecodeReaderBytesBuilder::new().build(Limited {
            input: compressed,
            left: 256 * 1024 * 1024,
        }),
    ))
}
pub fn timestamp(value: &str, zone: chrono_tz::Tz) -> Option<u64> {
    let mut parts = value.split_whitespace();
    let date = parts.next()?;
    if ![4, 6, 8, 10, 12, 14].contains(&date.len()) || !date.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let number = |a, b, default| {
        date.get(a..b)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(default)
    };
    let date = NaiveDate::from_ymd_opt(number(0, 4, 0) as i32, number(4, 6, 1), number(6, 8, 1))?
        .and_hms_opt(number(8, 10, 0), number(10, 12, 0), number(12, 14, 0))?;
    let millis = if let Some(offset) = parts.next() {
        if offset.len() != 5
            || !offset.is_ascii()
            || !offset[1..].bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        let hour = offset[1..3].parse::<i32>().ok()?;
        let minute = offset[3..5].parse::<i32>().ok()?;
        if hour > 23 || minute > 59 {
            return None;
        }
        let sign = match &offset[..1] {
            "+" => 1,
            "-" => -1,
            _ => return None,
        };
        FixedOffset::east_opt(sign * (hour * 3600 + minute * 60))?
            .from_local_datetime(&date)
            .single()?
            .timestamp_millis()
    } else {
        zone.from_local_datetime(&date)
            .earliest()?
            .timestamp_millis()
    };
    u64::try_from(millis).ok()
}
impl Library {
    pub(super) fn guide_path(&self, id: &str) -> Result<PathBuf> {
        self.path(id)?;
        Ok(self.directory.join(format!("guide-{id}.sqlite")))
    }
    pub fn refresh_guide(self: &Arc<Self>, id: &str, force: bool) {
        let Ok(source) = self.source(id) else { return };
        if !source.enabled {
            return;
        }
        if !source.has_guide() {
            if let Some(shared) = self.shared_guide() {
                shared.refresh(force);
            }
            return;
        }
        if !source.settings.guide.enabled {
            return;
        }
        if source.guide.next_retry_at > now()
            || !force && now().saturating_sub(source.guide.updated_at) < 6 * 3_600_000
        {
            return;
        }
        if !self
            .guide_loading
            .lock()
            .expect("guide loading")
            .insert(id.into())
        {
            return;
        }
        {
            let mut sources = self.sources.lock().expect("sources");
            if let Some(s) = sources.get_mut(id) {
                s.guide.loading = true;
            }
        }
        let library = self.clone();
        thread::spawn(move || {
            let result = library.fetch_guide(&source);
            library
                .guide_loading
                .lock()
                .expect("guide loading")
                .remove(&source.id);
            let _operation = library.operation.lock().expect("operation");
            let mut sources = library.sources.lock().expect("sources");
            if let Some(s) = sources.get_mut(&source.id) {
                if s.revision == source.revision {
                    s.guide.loading = false;
                    match result {
                        Ok((matched, programmes)) => {
                            s.guide.updated_at = now();
                            s.guide.matched = matched;
                            s.guide.programmes = programmes;
                            s.guide.error.clear();
                            s.guide.next_retry_at = 0;
                        }
                        Err(error) => {
                            s.guide.error = error.to_string();
                            s.guide.next_retry_at = now()
                                + error
                                    .downcast_ref::<GuideRetry>()
                                    .map_or(3_600_000, |retry| retry.0);
                        }
                    }
                    let _ = library.save(&sources);
                } else {
                    s.guide.loading = false;
                }
            }
        });
    }
    fn fetch_guide(&self, source: &Source) -> Result<(usize, usize)> {
        let context = self.context(source)?;
        let urls = if source.settings.guide.urls.is_empty() {
            &source.report.guide_urls
        } else {
            &source.settings.guide.urls
        };
        if urls.is_empty() {
            bail!("Questa lista non indica una guida. Aggiungi un indirizzo XMLTV nelle impostazioni.");
        }
        let draft = self.directory.join(format!(
            ".draft-guide-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        let result = (|| -> Result<(usize, usize)> {
            let db = open(&draft)?;
            schema(&db)?;
            let signature = format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&(
                    urls,
                    &source.settings.guide,
                    source.updated_at,
                    &source.settings.content
                ))?)
            );
            db.execute("INSERT INTO metadata VALUES('signature',?)", [&signature])?;
            {
                let _operation = self.operation.lock().expect("operation");
                let catalog = open(&self.path(&source.id)?)?;
                let content = match source.settings.content.as_str() {
                    "vod" => "0",
                    "live" => "1",
                    _ => "content='live'",
                };
                let mut stmt=catalog.prepare(&format!("SELECT id,name,payload FROM records WHERE active=1 AND kind='tv' AND {content} ORDER BY position LIMIT 100000"))?;
                let mut rows = stmt.query([])?;
                let mut insert = db.prepare("INSERT INTO candidates VALUES(?,?,?,?,?)")?;
                while let Some(row) = rows.next()? {
                    let entry: super::Entry = serde_json::from_str(&row.get::<_, String>(2)?)?;
                    let shift = if source.settings.guide.shift_minutes != 0 {
                        source.settings.guide.shift_minutes as i64 * 60_000
                    } else {
                        (entry.metadata.shift_hours * 3_600_000.0) as i64
                    };
                    insert.execute(params![
                        row.get::<_, String>(0)?,
                        entry.name.to_lowercase(),
                        entry.metadata.tvg_id,
                        if entry.metadata.tvg_name.is_empty() {
                            entry.name.to_lowercase()
                        } else {
                            entry.metadata.tvg_name.to_lowercase()
                        },
                        shift
                    ])?;
                }
            }
            let mut total = 0usize;
            let old = self.guide_path(&source.id)?;
            for (feed, url) in urls.iter().enumerate() {
                context.check_for(Resource::Guide)?;
                let mut validators = Headers::new();
                if old.exists() {
                    let _operation = self.operation.lock().expect("operation");
                    let previous = open(&old)?;
                    let same = previous
                        .query_row(
                            "SELECT value FROM metadata WHERE key='signature'",
                            [],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?
                        .as_deref()
                        == Some(&signature);
                    if same {
                        for (key, header) in
                            [("etag", "If-None-Match"), ("modified", "If-Modified-Since")]
                        {
                            if let Some(value) = previous
                                .query_row(
                                    "SELECT value FROM metadata WHERE key=?",
                                    [format!("feed:{feed}:{key}")],
                                    |r| r.get::<_, String>(0),
                                )
                                .optional()?
                            {
                                if !value.is_empty() {
                                    validators.insert(header.into(), value);
                                }
                            }
                        }
                    }
                }
                let response =
                    context.request(url, url, Resource::Guide, &validators, Method::GET, None)?;
                if response.status().as_u16() == 304 {
                    if validators.is_empty() || !old.exists() {
                        bail!("La guida non ha restituito un documento utilizzabile.");
                    }
                    let _operation = self.operation.lock().expect("operation");
                    db.execute(
                        "ATTACH DATABASE ? AS unchanged",
                        [old.to_string_lossy().as_ref()],
                    )?;
                    db.execute("INSERT OR IGNORE INTO channels SELECT * FROM unchanged.channels WHERE feed=?",[feed])?;
                    db.execute(
                        "INSERT OR IGNORE INTO names SELECT * FROM unchanged.names WHERE feed=?",
                        [feed],
                    )?;
                    db.execute("INSERT OR IGNORE INTO mappings SELECT * FROM unchanged.mappings WHERE feed=?",[feed])?;
                    db.execute("INSERT OR IGNORE INTO programmes SELECT * FROM unchanged.programmes WHERE channel IN(SELECT channel FROM unchanged.mappings WHERE feed=?)",[feed])?;
                    db.execute("INSERT OR IGNORE INTO metadata SELECT * FROM unchanged.metadata WHERE key LIKE ?",[format!("feed:{feed}:%")])?;
                    db.execute_batch("DETACH DATABASE unchanged")?;
                    continue;
                }
                if response.status().as_u16() == 429 {
                    let seconds = response
                        .headers()
                        .get("Retry-After")
                        .and_then(|s| s.to_str().ok())
                        .and_then(|s| s.parse::<u64>().ok())
                        .unwrap_or(3600)
                        .clamp(60, 86400);
                    return Err(GuideRetry(seconds * 1000).into());
                }
                if !response.status().is_success() {
                    bail!("Il server della guida non risponde o non autorizza l’accesso.");
                }
                for (key, header) in [("etag", "ETag"), ("modified", "Last-Modified")] {
                    let value = response
                        .headers()
                        .get(header)
                        .and_then(|s| s.to_str().ok())
                        .unwrap_or("")
                        .chars()
                        .take(1024)
                        .collect::<String>();
                    db.execute(
                        "INSERT OR REPLACE INTO metadata VALUES(?,?)",
                        params![format!("feed:{feed}:{key}"), value],
                    )?;
                }
                parse_feed(
                    &db,
                    decoded(Box::new(response))?,
                    source,
                    feed,
                    &mut total,
                    &|| context.check_for(Resource::Guide),
                )?;
            }
            // Infer only missing stops that have a later programme on the same channel.
            db.execute_batch("UPDATE programmes SET end=(SELECT MIN(p.start) FROM programmes p WHERE p.channel=programmes.channel AND p.start>programmes.start) WHERE end=0;
                DELETE FROM programmes WHERE end IS NULL OR end<=start;")?;
            let from = now().saturating_sub(source.settings.guide.history_days as u64 * 86_400_000);
            let until = now() + source.settings.guide.future_hours as u64 * 3_600_000;
            if old.exists() {
                let _operation = self.operation.lock().expect("operation");
                db.execute(
                    "ATTACH DATABASE ? AS previous",
                    [old.to_string_lossy().as_ref()],
                )?;
                let previous = db
                    .query_row(
                        "SELECT value FROM previous.metadata WHERE key='signature'",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()?;
                if previous.as_deref() == Some(&signature) {
                    db.execute("INSERT OR IGNORE INTO programmes SELECT * FROM previous.programmes WHERE end>? AND start<? AND channel IN(SELECT channel FROM mappings)",params![from,until])?;
                }
                db.execute_batch("DETACH DATABASE previous;")?;
            }
            db.execute(
                "DELETE FROM programmes WHERE end<=? OR start>=?",
                params![from, until],
            )?;
            let matched: usize = db.query_row("SELECT COUNT(*) FROM mappings", [], |r| r.get(0))?;
            let programmes: usize =
                db.query_row("SELECT COUNT(*) FROM programmes", [], |r| r.get(0))?;
            if programmes > 1_000_000 {
                bail!("La guida supera un milione di programmi.");
            }
            let definitions: usize =
                db.query_row("SELECT COUNT(*) FROM channels", [], |r| r.get(0))?;
            if definitions == 0 && programmes == 0 {
                bail!("Il documento XMLTV è vuoto. La guida precedente resta disponibile.");
            }
            drop(db);
            context.check_for(Resource::Guide)?;
            let _operation = self.operation.lock().expect("operation");
            if self.source(&source.id)?.revision != source.revision {
                bail!("La configurazione della guida è cambiata.");
            }
            let used = fs::read_dir(&self.directory)?
                .filter_map(|f| f.ok())
                .filter(|f| {
                    f.file_name().to_string_lossy().starts_with("guide-") && f.path() != old
                })
                .filter_map(|f| f.metadata().ok())
                .map(|m| m.len())
                .sum::<u64>();
            if used + fs::metadata(&draft)?.len() > 256 * 1024 * 1024 {
                bail!("Le guide personali superano 256 MiB.");
            }
            let backup = self
                .directory
                .join(format!(".backup-guide-{}.sqlite", source.id));
            if old.exists() {
                fs::rename(&old, &backup)?;
            }
            if let Err(error) = fs::rename(&draft, &old) {
                if backup.exists() {
                    let _ = fs::rename(&backup, &old);
                }
                return Err(error.into());
            }
            let _ = fs::remove_file(backup);
            Ok((matched, programmes))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&draft);
            let _ = fs::remove_file(format!("{}-journal", draft.display()));
        }
        result
    }
    pub fn clear_guide(&self, id: &str) -> Result<Value> {
        let _operation = self.operation.lock().expect("operation");
        let mut sources = self.sources.lock().expect("sources");
        let source = sources
            .get_mut(id)
            .ok_or_else(|| anyhow!("Lista non disponibile."))?;
        source.revision += 1;
        source.guide = Default::default();
        let path = self.guide_path(id)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
        self.save(&sources)?;
        drop(sources);
        Ok(self.snapshot())
    }
    pub(super) fn guide_event(
        &self,
        source: &str,
        channel: &str,
        event: &str,
    ) -> Result<GuideEvent> {
        let start = event
            .parse::<u64>()
            .map_err(|_| anyhow!("Programma non valido."))?;
        let _operation = self.operation.lock().expect("operation");
        let path = self.guide_path(source)?;
        if !path.exists() {
            bail!("La guida non è ancora disponibile.");
        }
        open(&path)?.query_row("SELECT title,start,end,description,catchup_id FROM programmes WHERE channel=? AND start=?",params![channel,start],event_row).optional()?.ok_or_else(||anyhow!("Il programma non è più presente nella guida."))
    }
    pub fn guide_window(
        &self,
        id: &str,
        from: u64,
        until: u64,
        text: &str,
        offset: usize,
    ) -> Result<Value> {
        let (source, entry) = self.entry(id)?;
        let hints = super::metadata::Hints::from(&entry);
        if entry.kind == crate::model::MediaKind::Tv
            && entry.content == "live"
            && self.use_public_programmes(&source, id, &hints)
        {
            if let Some(shared) = self.shared_guide() {
                let (programmes, total, origin) = shared.public_window(
                    &hints.identity(&entry.name),
                    from,
                    until,
                    text,
                    offset,
                )?;
                let rows=programmes.into_iter().map(|p|json!({"title":p.title,"description":p.description,"start":p.start,"end":p.end,"event":format!("{origin}:{}",p.start),"archive":false,"ongoing":false})).collect::<Vec<_>>();
                return Ok(
                    json!({"rows":rows,"total":total,"status":shared.public_status(),"enabled":true,"origin":origin}),
                );
            }
        }
        let _operation = self.operation.lock().expect("operation");
        let path = self.guide_path(&source.id)?;
        if !source.settings.guide.enabled || !path.exists() {
            return Ok(
                json!({"rows":[],"total":0,"status":source.guide,"enabled":source.settings.guide.enabled}),
            );
        }
        let db = open(&path)?;
        let mut args = vec![id.to_owned()];
        let mut filter = "channel=?".to_owned();
        if text.is_empty() {
            filter.push_str(" AND end>? AND start<?");
            args.push(from.to_string());
            args.push(until.min(from + 8 * 86_400_000).to_string());
        } else {
            filter.push_str(" AND (title LIKE ? ESCAPE '\\' OR description LIKE ? ESCAPE '\\')");
            let pattern = format!(
                "%{}%",
                text.chars()
                    .take(300)
                    .collect::<String>()
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            );
            args.push(pattern.clone());
            args.push(pattern);
        }
        let total: usize = db.query_row(
            &format!("SELECT COUNT(*) FROM programmes WHERE {filter}"),
            rusqlite::params_from_iter(&args),
            |r| r.get(0),
        )?;
        args.push(offset.to_string());
        let mut stmt=db.prepare(&format!("SELECT title,start,end,description,catchup_id FROM programmes WHERE {filter} ORDER BY start LIMIT 50 OFFSET ?"))?;
        let events = stmt
            .query_map(rusqlite::params_from_iter(&args), event_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let rows=events.into_iter().map(|event|json!({"title":event.programme.title,"description":event.programme.description,"start":event.programme.start,"end":event.programme.end,"event":event.programme.start.to_string(),"archive":catchup::available(&entry,&source.settings.guide,&event,now(),false),"ongoing":source.settings.guide.ongoing&&catchup::available(&entry,&source.settings.guide,&event,now(),true)&&event.programme.end>now()})).collect::<Vec<_>>();
        Ok(json!({"rows":rows,"total":total,"status":source.guide,"enabled":true}))
    }
    pub fn guide_choices(&self, source: &str, query: &str, offset: usize) -> Result<Value> {
        let _operation = self.operation.lock().expect("operation");
        let path = self.guide_path(source)?;
        if !path.exists() {
            return Ok(json!({"rows":[],"total":0}));
        }
        let db = open(&path)?;
        let query = format!(
            "%{}%",
            query
                .chars()
                .take(300)
                .collect::<String>()
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let total: usize = db.query_row(
            "SELECT COUNT(*) FROM channels WHERE name LIKE ? ESCAPE '\\' OR id LIKE ? ESCAPE '\\'",
            params![query, query],
            |r| r.get(0),
        )?;
        let mut stmt=db.prepare("SELECT feed,id,name FROM channels WHERE name LIKE ? ESCAPE '\\' OR id LIKE ? ESCAPE '\\' ORDER BY name COLLATE NOCASE,feed,id LIMIT 50 OFFSET ?")?;
        let rows=stmt.query_map(params![query,query,offset],|r|Ok(json!({"key":format!("{}|{}",r.get::<_,usize>(0)?,r.get::<_,String>(1)?),"name":r.get::<_,String>(2)?,"xml_id":r.get::<_,String>(1)?,"feed":r.get::<_,usize>(0)?})))?.collect::<std::result::Result<Vec<_>,_>>()?;
        Ok(json!({"rows":rows,"total":total}))
    }
}
fn schema(db: &Connection) -> Result<()> {
    db.execute_batch("PRAGMA max_page_count=32768;
                CREATE TABLE channels(feed INTEGER,id TEXT,name TEXT,PRIMARY KEY(feed,id));
                CREATE TABLE names(feed INTEGER,id TEXT,name TEXT,PRIMARY KEY(feed,id,name));CREATE INDEX by_name ON names(feed,name);
                CREATE TABLE candidates(id TEXT PRIMARY KEY,name TEXT,tvg_id TEXT,tvg_name TEXT,shift INTEGER);CREATE INDEX tvg ON candidates(tvg_id);CREATE INDEX title ON candidates(tvg_name);
                CREATE TABLE mappings(channel TEXT PRIMARY KEY,feed INTEGER,xml TEXT,shift INTEGER);CREATE INDEX xml_mapping ON mappings(feed,xml);
                CREATE TABLE programmes(channel TEXT,start INTEGER,end INTEGER,title TEXT,description TEXT,catchup_id TEXT,PRIMARY KEY(channel,start));CREATE INDEX by_time ON programmes(start,end);
                CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT);")?;
    Ok(())
}

fn event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<GuideEvent> {
    Ok(GuideEvent {
        programme: crate::guide::Programme {
            title: row.get(0)?,
            start: row.get(1)?,
            end: row.get(2)?,
            description: row.get(3)?,
            category: String::new(),
        },
        catchup_id: row.get(4)?,
    })
}
fn map_channels(db: &Connection, source: &Source, feed: usize) -> Result<()> {
    for (channel, choice) in &source.settings.guide.mappings {
        if let Some((f, id)) = choice.split_once('|') {
            if f.parse::<usize>().ok() == Some(feed) {
                db.execute("INSERT OR REPLACE INTO mappings SELECT candidates.id,?, ?,shift FROM candidates WHERE candidates.id=? AND EXISTS(SELECT 1 FROM channels WHERE feed=? AND id=?)",params![feed,id,channel,feed,id])?;
            }
        }
    }
    db.execute("INSERT OR IGNORE INTO mappings SELECT c.id,?,x.id,c.shift FROM candidates c JOIN channels x ON x.feed=? AND x.id=c.tvg_id WHERE c.tvg_id<>''",params![feed,feed])?;
    db.execute("INSERT OR IGNORE INTO mappings SELECT c.id,?,MIN(n.id),c.shift FROM candidates c JOIN names n ON n.feed=? AND n.name=c.tvg_name GROUP BY c.id HAVING COUNT(DISTINCT n.id)=1",params![feed,feed])?;
    for (channel, choice) in &source.settings.guide.mappings {
        if choice.is_empty() {
            db.execute("DELETE FROM mappings WHERE channel=?", [channel])?;
        } else if let Some((f, id)) = choice.split_once('|') {
            if f.parse::<usize>().ok() != Some(feed) {
                db.execute(
                    "DELETE FROM mappings WHERE channel=? AND feed=?",
                    params![channel, feed],
                )?;
            } else {
                db.execute(
                    "DELETE FROM mappings WHERE channel=? AND (feed<>? OR xml<>?)",
                    params![channel, feed, id],
                )?;
            }
        }
    }
    Ok(())
}
fn parse_feed(
    db: &Connection,
    input: Box<dyn Read + Send>,
    source: &Source,
    feed: usize,
    total: &mut usize,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    let zone = source.settings.guide.timezone.parse::<chrono_tz::Tz>()?;
    let mut reader = Reader::from_reader(BufReader::new(input));
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut depth = 0;
    let mut root = false;
    let mut seen_programme = false;
    let mut definitions = 0;
    let mut channel = String::new();
    let mut channel_names = Vec::<String>::new();
    let mut field = String::new();
    let mut field_text = String::new();
    let mut rank = 0;
    let (mut title_rank, mut desc_rank) = (0, 0);
    let mut xml = String::new();
    let (mut start, mut end) = (0, 0);
    let (mut title, mut description, mut catchup_id) =
        (String::new(), String::new(), String::new());
    db.execute_batch("BEGIN IMMEDIATE")?;
    loop {
        check()?;
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| anyhow!("Il documento XMLTV è incompleto o non valido."))?;
        if event.as_ref().len() > 128 * 1024 {
            bail!("Una dichiarazione XMLTV supera il limite previsto.");
        }
        match event {
            Event::Start(e) => {
                depth += 1;
                if depth > 32 {
                    bail!("La guida XMLTV supera 32 livelli.");
                }
                if depth == 1 {
                    if e.name().as_ref() != b"tv" {
                        bail!("Il documento non è una guida XMLTV.");
                    }
                    root = true;
                }
                match e.name().as_ref() {
                    b"channel" => {
                        if seen_programme {
                            bail!("I canali XMLTV devono precedere i programmi.");
                        }
                        channel = attr(&e, b"id");
                        channel_names.clear();
                        if channel.len() > 300 {
                            bail!("Identificativo XMLTV troppo lungo.");
                        }
                    }
                    b"display-name" if !channel.is_empty() => {
                        field = "name".into();
                        field_text.clear();
                    }
                    b"programme" => {
                        if !seen_programme {
                            map_channels(db, source, feed)?;
                            seen_programme = true;
                        }
                        xml = attr(&e, b"channel");
                        start = timestamp(&attr(&e, b"start"), zone).unwrap_or(0);
                        end = timestamp(&attr(&e, b"stop"), zone).unwrap_or(0);
                        title.clear();
                        description.clear();
                        title_rank = 0;
                        desc_rank = 0;
                        catchup_id = attr(&e, b"catchup-id");
                    }
                    b"title" | b"desc" if !xml.is_empty() => {
                        field = if e.name().as_ref() == b"title" {
                            "title"
                        } else {
                            "desc"
                        }
                        .into();
                        field_text.clear();
                        let language = attr(&e, b"lang");
                        rank = if language == source.settings.guide.language {
                            3
                        } else if language.is_empty() {
                            2
                        } else {
                            1
                        };
                    }
                    _ => {}
                }
            }
            Event::Text(e) if !field.is_empty() => {
                let text = e
                    .unescape()
                    .map_err(|_| anyhow!("Testo XMLTV non valido."))?;
                let max = if field == "desc" { 1600 } else { 300 };
                field_text.extend(text.chars().take(max - field_text.chars().count().min(max)));
            }
            Event::CData(e) if !field.is_empty() => {
                let max = if field == "desc" { 1600 } else { 300 };
                field_text.extend(
                    String::from_utf8_lossy(&e)
                        .chars()
                        .take(max - field_text.chars().count().min(max)),
                );
            }
            Event::End(e) => {
                match e.name().as_ref() {
                    b"display-name" => {
                        if channel_names.len() < 5 && !field_text.trim().is_empty() {
                            channel_names.push(field_text.trim().to_owned());
                        }
                        field.clear();
                    }
                    b"channel" => {
                        definitions += 1;
                        if definitions > 100000 {
                            bail!("La guida supera 100.000 definizioni di canale.");
                        }
                        let name = channel_names
                            .first()
                            .cloned()
                            .unwrap_or_else(|| channel.clone());
                        db.execute(
                            "INSERT OR REPLACE INTO channels VALUES(?,?,?)",
                            params![feed, channel, name],
                        )?;
                        for name in &channel_names {
                            db.execute(
                                "INSERT OR IGNORE INTO names VALUES(?,?,?)",
                                params![feed, channel, name.to_lowercase()],
                            )?;
                        }
                        channel.clear();
                    }
                    b"title" => {
                        if rank > title_rank && !field_text.trim().is_empty() {
                            title = field_text.trim().into();
                            title_rank = rank;
                        }
                        field.clear();
                    }
                    b"desc" => {
                        if rank > desc_rank && !field_text.trim().is_empty() {
                            description = field_text.trim().into();
                            desc_rank = rank;
                        }
                        field.clear();
                    }
                    b"programme" => {
                        if start > 0 && !title.is_empty() {
                            let mut stmt = db.prepare(
                                "SELECT channel,shift FROM mappings WHERE feed=? AND xml=?",
                            )?;
                            let mappings = stmt
                                .query_map(params![feed, xml], |r| {
                                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                                })?
                                .collect::<std::result::Result<Vec<_>, _>>()?;
                            for (channel, shift) in mappings {
                                let start = (start as i64 + shift).max(0) as u64;
                                let end = if end == 0 {
                                    0
                                } else {
                                    (end as i64 + shift).max(0) as u64
                                };
                                if start
                                    > now() + source.settings.guide.future_hours as u64 * 3_600_000
                                    || end != 0
                                        && end
                                            < now().saturating_sub(
                                                source.settings.guide.history_days as u64
                                                    * 86_400_000,
                                            )
                                {
                                    continue;
                                }
                                *total += 1;
                                if *total > 1_000_000 {
                                    bail!("La guida supera un milione di programmi.");
                                }
                                db.execute(
                                    "INSERT OR REPLACE INTO programmes VALUES(?,?,?,?,?,?)",
                                    params![channel, start, end, title, description, catchup_id],
                                )?;
                            }
                        }
                        xml.clear();
                        field.clear();
                    }
                    _ => {}
                }
                depth -= 1;
            }
            Event::DocType(_) => {
                bail!("Le dichiarazioni DTD non sono supportate nelle guide personali.")
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    if !root || depth != 0 {
        bail!("Il documento XMLTV è incompleto.");
    }
    if !seen_programme {
        map_channels(db, source, feed)?;
    }
    db.execute_batch("COMMIT")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn compressed_xmltv_cache_etags_and_rate_limits_preserve_the_valid_guide() {
        let fixture = crate::personal::tests::Fixture::new();
        let source_id = fixture.import(
            "#EXTM3U\n#EXTINF:-1 tvg-id=\"one\",Primo\nhttps://media.example/live.m3u8\n",
            None,
        );
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let time = |offset: i64| {
            chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now() as i64 + offset)
                .unwrap()
                .format("%Y%m%d%H%M%S +0000")
                .to_string()
        };
        let xml=format!("<tv><channel id=\"one\"><display-name>Primo</display-name></channel><programme channel=\"one\" start=\"{}\"><title lang=\"en\">English</title><title lang=\"it\">Titolo italiano</title><desc><![CDATA[Descrizione <test>]]></desc></programme><programme channel=\"one\" start=\"{}\" stop=\"{}\"><title>Successivo</title></programme><programme channel=\"one\" start=\"{}\"><title>Fine sconosciuta</title></programme></tv>",time(-60000),time(600000),time(1200000),time(1300000));
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(xml.as_bytes()).unwrap();
        let bytes = gzip.finish().unwrap();
        let worker = thread::spawn(move || {
            for step in 0..3 {
                let request = server
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                if step == 0 {
                    request
                        .respond(tiny_http::Response::from_data(bytes.clone()).with_header(
                            tiny_http::Header::from_bytes("ETag", "\"fixture-v1\"").unwrap(),
                        ))
                        .unwrap();
                } else if step == 1 {
                    assert!(request
                        .headers()
                        .iter()
                        .any(|h| h.field.equiv("If-None-Match")
                            && h.value.as_str() == "\"fixture-v1\""));
                    request
                        .respond(tiny_http::Response::from_string("").with_status_code(304))
                        .unwrap();
                } else {
                    request
                        .respond(
                            tiny_http::Response::from_string("")
                                .with_status_code(429)
                                .with_header(
                                    tiny_http::Header::from_bytes("Retry-After", "7200").unwrap(),
                                ),
                        )
                        .unwrap();
                }
            }
        });
        let original = fixture.library.source(&source_id).unwrap();
        let mut settings = original.settings.clone();
        settings.guide.enabled = true;
        settings.guide.allow_http = true;
        settings.guide.urls = vec![format!("http://guide.example:{}/tv.xml.gz", address.port())];
        fixture
            .library
            .edit(
                &source_id,
                original.revision,
                &original.name,
                true,
                settings,
            )
            .unwrap();
        let source = fixture.library.source(&source_id).unwrap();
        fixture.library.fixture_context(
            &source_id,
            super::super::network::Context::fixture(
                &source.settings,
                &[("guide.example", address)],
            ),
        );
        let (matched, count) = fixture.library.fetch_guide(&source).unwrap();
        assert_eq!(matched, 1);
        assert_eq!(count, 2);
        let channel = fixture.rows(&source_id)["rows"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let display = fixture.library.channel(&channel).unwrap();
        assert_eq!(display.now.unwrap().title, "Titolo italiano");
        assert_eq!(display.next.unwrap().title, "Successivo");
        let window = fixture
            .library
            .guide_window(&channel, now() - 3600000, now() + 3600000, "", 0)
            .unwrap();
        assert_eq!(window["rows"][0]["title"], "Titolo italiano");
        assert_eq!(window["rows"][0]["description"], "Descrizione <test>");
        assert_eq!(window["rows"][0]["end"], window["rows"][1]["start"]);
        assert_eq!(fixture.library.fetch_guide(&source).unwrap(), (1, 2));
        let error = fixture.library.fetch_guide(&source).unwrap_err();
        assert_eq!(error.downcast_ref::<GuideRetry>().unwrap().0, 7200000);
        assert_eq!(
            fixture
                .library
                .guide_window(&channel, now() - 3600000, now() + 3600000, "", 0)
                .unwrap()["total"],
            2
        );
        worker.join().unwrap();
    }
    #[test]
    fn xml_entities_and_oversized_nesting_are_rejected() {
        let fixture = crate::personal::tests::Fixture::new();
        let id = fixture.import(
            "#EXTM3U\n#EXTINF:-1,Primo\nhttps://media.example/live.m3u8\n",
            None,
        );
        let source = fixture.library.source(&id).unwrap();
        for xml in [
            "<!DOCTYPE tv [<!ENTITY x 'unsafe'>]><tv></tv>".to_owned(),
            format!(
                "<tv>{}{}</tv>",
                "<nested>".repeat(33),
                "</nested>".repeat(33)
            ),
        ] {
            let db = Connection::open_in_memory().unwrap();
            schema(&db).unwrap();
            assert!(parse_feed(
                &db,
                Box::new(std::io::Cursor::new(xml.into_bytes())),
                &source,
                0,
                &mut 0,
                &|| Ok(())
            )
            .is_err());
        }
    }
}
