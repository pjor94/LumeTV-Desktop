// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared, bounded XMLTV index. Personal playlist credentials never enter this database.
use crate::guide::Programme;
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

const MAX_CHANNELS: usize = 20_000;
const MAX_PROGRAMMES: usize = 1_000_000;
const MAX_CHANNEL_PROGRAMMES: usize = 192;
const VERSION: u32 = 1;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Info {
    pub version: u32,
    pub updated_at: u64,
    pub channels: usize,
    pub programmes: usize,
}
#[derive(Clone, Default)]
pub(crate) struct Identity {
    pub xml_id: String,
    pub names: Vec<String>,
}

pub(crate) struct ChannelAlias {
    pub playlist_id: &'static str,
    pub playlist_name: &'static str,
    pub feed_id: &'static str,
    pub feed_name: &'static str,
}
#[derive(Clone, Default)]
pub(crate) struct Match {
    pub logo: String,
    pub now: Option<Programme>,
    pub next: Option<Programme>,
    pub has_programmes: bool,
    pub origin: &'static str,
}
#[derive(Clone, Default)]
pub(crate) struct Probe {
    pub data: Option<Match>,
    pub known: bool,
}
pub(crate) struct PublicGuide {
    path: PathBuf,
    state: Mutex<Info>,
    channel_aliases: &'static [ChannelAlias],
}
fn read(path: &Path) -> Result<Connection> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(std::time::Duration::from_secs(5))?;
    db.execute_batch("PRAGMA cache_size=-2048; PRAGMA temp_store=MEMORY;")?;
    Ok(db)
}
fn info(path: &Path) -> Result<Info> {
    if fs::metadata(path)?.len() > 256 * 1024 * 1024 {
        bail!("public_guide_size");
    }
    let db = read(path)?;
    let value: String = db.query_row("SELECT value FROM metadata WHERE key='info'", [], |r| {
        r.get(0)
    })?;
    let value: Info = serde_json::from_str(&value)?;
    if value.version != VERSION
        || value.channels == 0
        || value.programmes == 0
        || value.channels > MAX_CHANNELS
        || value.programmes > MAX_PROGRAMMES
    {
        bail!("public_guide_version");
    }
    Ok(value)
}
impl PublicGuide {
    pub fn new(directory: &Path) -> Self {
        Self::named(directory, "public-guide.sqlite", &[])
    }
    pub fn named(
        directory: &Path,
        filename: &str,
        channel_aliases: &'static [ChannelAlias],
    ) -> Self {
        let path = directory.join(filename);
        let previous = path.with_extension("sqlite.previous");
        if !path.exists() && info(&previous).is_ok() {
            let _ = fs::rename(&previous, &path);
        }
        Self {
            state: Mutex::new(info(&path).unwrap_or_default()),
            path,
            channel_aliases,
        }
    }
    pub fn info(&self) -> Info {
        self.state.lock().expect("public guide").clone()
    }
    pub fn ready(&self) -> bool {
        self.info().version == VERSION
    }
    pub fn begin(&self, at: u64) -> Result<Writer> {
        Writer::new(
            self.path
                .parent()
                .ok_or_else(|| anyhow!("public_guide_path"))?,
            at,
        )
    }
    pub fn lookup(&self, identities: &[Identity], at: u64) -> Vec<Probe> {
        let state = self.state.lock().expect("public guide");
        if state.version != VERSION {
            return vec![Probe::default(); identities.len()];
        }
        let Ok(db) = read(&self.path) else {
            return vec![Probe::default(); identities.len()];
        };
        identities
            .iter()
            .map(|identity| {
                let resolution = resolve(&db, identity, self.channel_aliases)
                    .unwrap_or(Resolution::Ambiguous);
                let id = match resolution {
                    Resolution::Missing => return Probe::default(),
                    Resolution::Ambiguous => return Probe { known: true, data: None },
                    Resolution::Matched(id) => id,
                };
                let mut logo: String = db
                    .query_row("SELECT logo FROM channels WHERE id=?", [&id], |r| r.get(0))
                    .unwrap_or_default();
                if !crate::guide::allowed_logo(&logo) {
                    logo.clear();
                }
                let current = |condition: &str| -> Option<Programme> {
                    db.query_row(
                        &format!("SELECT title,start,end,description,category FROM programmes WHERE channel=?1 AND {condition} LIMIT 1"),
                        params![id, at],
                        programme,
                    )
                    .optional()
                    .ok()
                    .flatten()
                };
                Probe {
                    known: true,
                    data: Some(Match {
                        logo,
                        now: current("start<=?2 AND end>?2 ORDER BY start DESC"),
                        next: current("start>?2 ORDER BY start"),
                        has_programmes: db.query_row(
                            "SELECT EXISTS(SELECT 1 FROM programmes WHERE channel=? AND end>?)",
                            params![id, at.saturating_sub(6 * 3_600_000)],
                            |r| r.get(0),
                        ).unwrap_or(false),
                        origin: "lume",
                    }),
                }
            })
            .collect()
    }
    pub fn window(
        &self,
        identity: &Identity,
        from: u64,
        until: u64,
        text: &str,
        offset: usize,
    ) -> Result<(Vec<Programme>, usize)> {
        let state = self.state.lock().expect("public guide");
        if state.version != VERSION {
            return Ok((vec![], 0));
        }
        let db = read(&self.path)?;
        let Resolution::Matched(id) = resolve(&db, identity, self.channel_aliases)? else {
            return Ok((vec![], 0));
        };
        let mut filter = "channel=?".to_owned();
        let mut args = vec![id];
        if text.is_empty() {
            filter.push_str(" AND end>? AND start<?");
            args.push(from.to_string());
            args.push(until.min(from.saturating_add(8 * 86_400_000)).to_string());
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
        let total = db.query_row(
            &format!("SELECT COUNT(*) FROM programmes WHERE {filter}"),
            rusqlite::params_from_iter(&args),
            |r| r.get(0),
        )?;
        args.push(offset.to_string());
        let mut statement=db.prepare(&format!("SELECT title,start,end,description,category FROM programmes WHERE {filter} ORDER BY start LIMIT 50 OFFSET ?"))?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(&args), programme)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((rows, total))
    }
    pub fn publish(&self, mut writer: Writer) -> Result<()> {
        let new_info = writer.finish()?;
        let mut state = self.state.lock().expect("public guide");
        let backup = self.path.with_extension("sqlite.previous");
        let had_old = self.path.exists();
        if had_old {
            let _ = fs::remove_file(&backup);
            fs::rename(&self.path, &backup)?;
        }
        if let Err(error) = fs::rename(&writer.path, &self.path) {
            if had_old {
                let _ = fs::rename(&backup, &self.path);
            }
            return Err(error.into());
        }
        *state = new_info;
        let _ = fs::remove_file(backup);
        Ok(())
    }
}
fn programme(row: &rusqlite::Row<'_>) -> rusqlite::Result<Programme> {
    Ok(Programme {
        title: row.get(0)?,
        start: row.get(1)?,
        end: row.get(2)?,
        description: row.get(3)?,
        category: row.get(4)?,
    })
}
pub(crate) fn name_key(value: &str) -> String {
    let mut words = value.split_whitespace().collect::<Vec<_>>();
    while words.last().is_some_and(|w| {
        ["hd", "sd", "fhd", "uhd", "4k", "8k"].contains(&w.to_ascii_lowercase().as_str())
    }) {
        words.pop();
    }
    words
        .join(" ")
        .nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .filter(|c| c.is_alphanumeric() || *c == '+')
        .collect()
}
fn candidates(db: &Connection, kind: u8, key: &str) -> Result<BTreeSet<String>> {
    if key.is_empty() {
        return Ok(BTreeSet::new());
    }
    let mut statement =
        db.prepare_cached("SELECT channel FROM aliases WHERE kind=? AND name=? LIMIT 3")?;
    let rows = statement
        .query_map(params![kind, key], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}
enum Resolution {
    Matched(String),
    Missing,
    Ambiguous,
}
fn resolve(db: &Connection, identity: &Identity, aliases: &[ChannelAlias]) -> Result<Resolution> {
    let mut named = BTreeSet::new();
    for name in &identity.names {
        named.extend(candidates(db, 2, &name_key(name))?);
    }
    for (kind, key) in [
        (0, identity.xml_id.trim().to_lowercase()),
        (1, crate::guide::canonical(&identity.xml_id)),
    ] {
        let found = candidates(db, kind, &key)?;
        if found.len() > 1 {
            return Ok(Resolution::Ambiguous);
        }
        if let Some(id) = found.into_iter().next() {
            if !named.is_empty() && !named.contains(&id) {
                return Ok(Resolution::Ambiguous);
            }
            return compatible(db, &id, identity).map(|ok| {
                if ok {
                    Resolution::Matched(id)
                } else {
                    Resolution::Ambiguous
                }
            });
        }
    }
    if named.is_empty() {
        return resolve_alias(db, identity, aliases);
    }
    if named.len() > 1 {
        return Ok(Resolution::Ambiguous);
    }
    let id = named.into_iter().next().unwrap();
    compatible(db, &id, identity).map(|ok| {
        if ok {
            Resolution::Matched(id)
        } else {
            Resolution::Ambiguous
        }
    })
}

fn resolve_alias(
    db: &Connection,
    identity: &Identity,
    aliases: &[ChannelAlias],
) -> Result<Resolution> {
    // Use a verified pair of identifiers and names only after normal matching found nothing.
    let Some(alias) = aliases.iter().find(|alias| {
        identity
            .xml_id
            .trim()
            .eq_ignore_ascii_case(alias.playlist_id)
    }) else {
        return Ok(Resolution::Missing);
    };
    let expected_name = name_key(alias.playlist_name);
    let mut names = identity
        .names
        .iter()
        .map(|n| name_key(n))
        .filter(|n| !n.is_empty());
    if names.next().is_none_or(|name| name != expected_name)
        || names.any(|name| name != expected_name)
    {
        return Ok(Resolution::Missing);
    }
    let mut found = candidates(db, 0, alias.feed_id)?;
    if found.len() > 1 {
        return Ok(Resolution::Ambiguous);
    }
    let Some(id) = found.pop_first() else {
        return Ok(Resolution::Missing);
    };
    let names = display_names(db, &id)?;
    let expected_name = name_key(alias.feed_name);
    if names.iter().any(|n| name_key(n) == expected_name)
        && compatible_identity(identity, &id, &names)
    {
        Ok(Resolution::Matched(id))
    } else {
        Ok(Resolution::Ambiguous)
    }
}

fn qualifiers(value: &str) -> (BTreeSet<&'static str>, String) {
    let key = name_key(value);
    let regions = [
        "abruzzo",
        "basilicata",
        "calabria",
        "campania",
        "emiliaromagna",
        "friuliveneziagiulia",
        "lazio",
        "liguria",
        "lombardia",
        "marche",
        "molise",
        "piemonte",
        "puglia",
        "sardegna",
        "sicilia",
        "toscana",
        "trentino",
        "valledaosta",
        "veneto",
        "bolzano",
        "trento",
    ];
    let region = if key.contains("rai3") {
        regions.into_iter().filter(|r| key.contains(r)).collect()
    } else {
        BTreeSet::new()
    };
    let shift = key
        .split_once('+')
        .map(|(_, s)| s.chars().take_while(char::is_ascii_digit).collect())
        .unwrap_or_default();
    (region, shift)
}
fn display_names(db: &Connection, id: &str) -> Result<Vec<String>> {
    let names: String =
        db.query_row("SELECT names FROM channels WHERE id=?", [id], |r| r.get(0))?;
    Ok(serde_json::from_str(&names)?)
}
fn compatible(db: &Connection, id: &str, identity: &Identity) -> Result<bool> {
    Ok(compatible_identity(identity, id, &display_names(db, id)?))
}
pub(crate) fn compatible_identity(identity: &Identity, id: &str, names: &[String]) -> bool {
    let target = qualifiers(&format!(
        "{} {}",
        crate::guide::canonical(id),
        names.join(" ")
    ));
    for name in identity
        .names
        .iter()
        .chain(std::iter::once(&identity.xml_id))
    {
        let source = qualifiers(name);
        if !source.0.is_empty() && source.0.is_disjoint(&target.0)
            || !source.1.is_empty() && source.1 != target.1
        {
            return false;
        }
    }
    true
}

pub(crate) struct Writer {
    path: PathBuf,
    db: Option<Connection>,
    at: u64,
    channels: BTreeSet<String>,
    counts: BTreeMap<String, usize>,
    programmes: usize,
}
impl Writer {
    fn new(directory: &Path, at: u64) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let path = directory.join(format!(
            ".public-guide-{}.sqlite",
            uuid::Uuid::new_v4().simple()
        ));
        let db = Connection::open(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        }
        db.execute_batch("PRAGMA page_size=4096; PRAGMA max_page_count=65536; PRAGMA cache_size=-4096; PRAGMA journal_mode=DELETE; PRAGMA temp_store=FILE; CREATE TABLE metadata(key TEXT PRIMARY KEY,value TEXT); CREATE TABLE channels(id TEXT PRIMARY KEY,logo TEXT NOT NULL,names TEXT NOT NULL); CREATE TABLE aliases(kind INTEGER NOT NULL,name TEXT NOT NULL,channel TEXT NOT NULL,PRIMARY KEY(kind,name,channel)) WITHOUT ROWID; CREATE TABLE programmes(channel TEXT NOT NULL,start INTEGER NOT NULL,end INTEGER NOT NULL,title TEXT NOT NULL,description TEXT NOT NULL,category TEXT NOT NULL,PRIMARY KEY(channel,start,title)) WITHOUT ROWID; BEGIN IMMEDIATE;")?;
        Ok(Self {
            path,
            db: Some(db),
            at,
            channels: BTreeSet::new(),
            counts: BTreeMap::new(),
            programmes: 0,
        })
    }
    pub fn channel(&mut self, id: &str, names: &[String], logo: &str) -> Result<()> {
        if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
            bail!("guide_channel_identity");
        }
        self.channels.insert(id.into());
        if self.channels.len() > MAX_CHANNELS {
            bail!("guide_channel_limit");
        }
        let db = self.db.as_ref().unwrap();
        let names = names
            .iter()
            .take(16)
            .map(|v| v.chars().take(300).collect::<String>())
            .collect::<Vec<_>>();
        let logo = if crate::guide::allowed_logo(logo) {
            logo
        } else {
            ""
        };
        db.execute("INSERT INTO channels VALUES(?,?,?) ON CONFLICT(id) DO UPDATE SET logo=CASE WHEN excluded.logo<>'' THEN excluded.logo ELSE logo END,names=CASE WHEN excluded.names<>'[]' THEN excluded.names ELSE names END",params![id,logo,serde_json::to_string(&names)?])?;
        let mut alias = db.prepare_cached("INSERT OR IGNORE INTO aliases VALUES(?,?,?)")?;
        alias.execute(params![0, id.trim().to_lowercase(), id])?;
        alias.execute(params![1, crate::guide::canonical(id), id])?;
        for name in names
            .iter()
            .cloned()
            .chain([crate::catalog::canonical_epg(id)])
        {
            let key = name_key(&name);
            if !key.is_empty() {
                alias.execute(params![2, key, id])?;
            }
        }
        Ok(())
    }
    pub fn programme(&mut self, id: &str, programme: &Programme) -> Result<()> {
        if !self.channels.contains(id) {
            self.channel(id, &[], "")?;
        }
        let count = self.counts.entry(id.into()).or_default();
        if *count >= MAX_CHANNEL_PROGRAMMES {
            return Ok(());
        }
        let db = self.db.as_ref().unwrap();
        let inserted = db.execute(
            "INSERT OR IGNORE INTO programmes VALUES(?,?,?,?,?,?)",
            params![
                id,
                programme.start,
                programme.end,
                programme.title,
                programme.description,
                programme.category
            ],
        )?;
        *count += inserted;
        self.programmes += inserted;
        if self.programmes > MAX_PROGRAMMES {
            bail!("guide_programme_limit");
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<Info> {
        if self.channels.is_empty() || self.programmes == 0 {
            bail!("guide_empty");
        }
        let value = Info {
            version: VERSION,
            updated_at: self.at,
            channels: self.channels.len(),
            programmes: self.programmes,
        };
        let db = self.db.take().unwrap();
        db.execute(
            "INSERT INTO metadata VALUES('info',?)",
            [serde_json::to_string(&value)?],
        )?;
        db.execute_batch("COMMIT; PRAGMA optimize;")?;
        db.close().map_err(|(_, e)| e)?;
        Ok(value)
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        self.db.take();
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_file(format!("{}-journal", self.path.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        index: PublicGuide,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("lume-public-guide-{}", uuid::Uuid::new_v4()));
            fs::create_dir_all(&root).unwrap();
            let index = PublicGuide::new(&root);
            Self { root, index }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn identity(id: &str, name: &str) -> Identity {
        Identity {
            xml_id: id.into(),
            names: vec![name.into()],
        }
    }
    fn programme(title: &str) -> Programme {
        Programme {
            title: title.into(),
            start: 1000,
            end: 20_000,
            description: "Descrizione".into(),
            category: String::new(),
        }
    }
    #[test]
    fn full_feed_matching_is_conservative_and_never_confuses_regions_or_timeshifts() {
        let f = Fixture::new();
        let mut writer = f.index.begin(10_000).unwrap();
        for (id, names, title) in [
            (
                "extra.it",
                vec!["Canale Esteso HD".into(), "Tèlè Èsteso".into()],
                "Extra",
            ),
            ("other.it", vec!["Canale Esteso SD".into()], "Altro"),
            ("rai3.it", vec!["Rai 3".into()], "Nazionale"),
            ("rai3tgr", vec!["Rai 3 TGR".into()], "Veneto"),
            ("extra1.it", vec!["Canale Esteso +1".into()], "Differita"),
        ] {
            writer
                .channel(id, &names, "https://images.example.org/logo.png")
                .unwrap();
            writer.programme(id, &programme(title)).unwrap();
        }
        f.index.publish(writer).unwrap();
        let inputs = [
            identity("extra.it", "Canale Esteso"),
            identity("", "Canale Esteso FHD"),
            identity("provider-id", "Tele Esteso"),
            identity("rai3.it", "Rai 3 TGR Puglia"),
            identity("rai3tgrveneto", "Rai 3 TGR Veneto"),
            identity("rai3tgrpuglia", "Rai 3"),
            identity("extra.it", "Canale Esteso +1"),
            identity("extra1.it", "Canale Esteso +1"),
        ];
        let result = f.index.lookup(&inputs, 10_000);
        assert_eq!(
            result[0].data.as_ref().unwrap().now.as_ref().unwrap().title,
            "Extra"
        );
        assert!(result[1].known && result[1].data.is_none());
        assert!(result[2].data.is_some());
        assert!(result[3].data.is_none());
        assert_eq!(
            result[4].data.as_ref().unwrap().now.as_ref().unwrap().title,
            "Veneto"
        );
        assert!(result[5].data.is_none());
        assert!(result[6].data.is_none());
        assert_eq!(
            result[7].data.as_ref().unwrap().now.as_ref().unwrap().title,
            "Differita"
        );
        let (window, total) = f
            .index
            .window(&inputs[0], 0, 30_000, "Descrizione", 0)
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(window[0].title, "Extra");
    }
    #[test]
    fn failed_publication_preserves_the_index_and_cached_artwork_is_revalidated() {
        let f = Fixture::new();
        let mut writer = f.index.begin(10_000).unwrap();
        writer
            .channel(
                "extra",
                &["Esteso".into()],
                "https://images.example.org/extra.png",
            )
            .unwrap();
        writer.programme("extra", &programme("Originale")).unwrap();
        f.index.publish(writer).unwrap();
        let mut empty = f.index.begin(20_000).unwrap();
        empty.channel("empty", &[], "").unwrap();
        assert!(f.index.publish(empty).is_err());
        assert_eq!(f.index.info().updated_at, 10_000);
        let input = identity("extra", "Esteso");
        assert_eq!(
            f.index.lookup(std::slice::from_ref(&input), 10_000)[0]
                .data
                .as_ref()
                .unwrap()
                .now
                .as_ref()
                .unwrap()
                .title,
            "Originale"
        );
        let db = Connection::open(&f.index.path).unwrap();
        db.execute("UPDATE channels SET logo='http://127.0.0.1/private'", [])
            .unwrap();
        drop(db);
        assert!(f.index.lookup(std::slice::from_ref(&input), 10_000)[0]
            .data
            .as_ref()
            .unwrap()
            .logo
            .is_empty());
        assert_eq!(PublicGuide::new(&f.root).info().channels, 1);
        fs::rename(
            &f.index.path,
            f.index.path.with_extension("sqlite.previous"),
        )
        .unwrap();
        assert!(PublicGuide::new(&f.root).ready());
    }
}
