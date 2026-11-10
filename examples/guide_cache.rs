// SPDX-License-Identifier: GPL-3.0-or-later
//! Import an already downloaded XMLTV file for offline UI verification.
use anyhow::{Context, Result};
use lume_player::{guide, report::now_ms};
use std::{fs, io::BufReader, path::PathBuf, time::UNIX_EPOCH};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let input = PathBuf::from(args.next().context("XMLTV input path required")?);
    let output = PathBuf::from(args.next().context("Guide cache output path required")?);
    let downloaded = fs::metadata(&input)?
        .modified()?
        .duration_since(UNIX_EPOCH)?;
    let mut snapshot = guide::parse(BufReader::new(fs::File::open(input)?), now_ms())?;
    // Retain the real download time so an old fixture never claims to be fresh.
    snapshot.updated_at = downloaded.as_millis().try_into()?;
    fs::write(output, serde_json::to_vec(&snapshot)?)?;
    println!(
        "XMLTV imported: {} channels, {} programmes, {} logos; download timestamp {}",
        snapshot.programmes.len(),
        snapshot.programmes.values().map(Vec::len).sum::<usize>(),
        snapshot.logos.len(),
        snapshot.updated_at,
    );
    Ok(())
}
