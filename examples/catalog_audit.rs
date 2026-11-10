// SPDX-License-Identifier: GPL-3.0-or-later
//! Read each bundled manifest with shared provider sessions; never request DRM licenses.
use lume_player::{
    catalog::catalog,
    providers::{probe_manifest, Resolver},
    report::{now_ms, write_json},
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};

fn main() -> anyhow::Result<()> {
    let output = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("reports/catalog-audit.json"));
    let resolver = Arc::new(Resolver::new()?);
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::<Value>::new());
    let started = now_ms();
    std::thread::scope(|scope| {
        for _ in 0..3 {
            let resolver = resolver.clone();
            let next = &next;
            let results = &results;
            scope.spawn(move || loop {
                let index = next.fetch_add(1, Ordering::Relaxed);
                let Some(channel) = catalog().get(index) else {
                    break;
                };
                let timer = Instant::now();
                let mut row = json!({"channel": channel, "license_requested": false});
                match resolver.resolve(&channel.stream_channel()) {
                    Ok(stream) => {
                        row["resolver"] = json!({
                            "status": "resolved",
                            "declared_drm": stream.drm.is_some(),
                            "duration_ms": timer.elapsed().as_millis(),
                        });
                        row["manifest"] = probe_manifest(&stream);
                    }
                    Err(error) => {
                        row["resolver"] = json!({
                            "status": "failed",
                            "reason": error.to_string(),
                            "duration_ms": timer.elapsed().as_millis(),
                        });
                    }
                }
                results.lock().unwrap().push(row);
            });
        }
    });
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|r| r["channel"]["number"].as_u64().unwrap_or(9999));
    let resolved = results
        .iter()
        .filter(|r| r["resolver"]["status"] == "resolved")
        .count();
    let fetched = results
        .iter()
        .filter(|r| r["manifest"]["status"] == "fetched")
        .count();
    write_json(
        &output,
        &json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"started_unix_ms":started,"completed_unix_ms":now_ms(),
        "catalog_channels":catalog().len(),"resolved":resolved,"manifests_fetched":fetched,"license_requested":false,"playback_verified":false,"runs":results}),
    )?;
    println!(
        "Catalogo: {} · Resolver riusciti: {resolved} · Manifest validi: {fetched} · Report: {}",
        catalog().len(),
        output.display()
    );
    Ok(())
}
