// SPDX-License-Identifier: GPL-3.0-or-later
//! Index a previously downloaded public XMLTV file without another network request.
use anyhow::{Context, Result};
use lume_player::guide::GuideService;
use std::{fs, io::BufReader, path::PathBuf, time::UNIX_EPOCH};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().context("XMLTV input path required")?);
    let profile = PathBuf::from(args.next().context("Profile directory required")?);
    let downloaded = fs::metadata(&input)?
        .modified()?
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?;
    let guide = GuideService::new(&profile)?;
    let reader = BufReader::new(fs::File::open(input)?);
    let status = match args.next().as_deref().and_then(|v| v.to_str()) {
        None => guide.import_xml(reader, downloaded)?,
        Some("epgshare") => guide.import_epgshare_xml(reader, downloaded)?,
        _ => anyhow::bail!("Optional source must be epgshare"),
    };
    println!("{}", serde_json::to_string(&status)?);
    Ok(())
}
