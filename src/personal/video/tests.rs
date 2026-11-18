use super::*;
use crate::personal::tests::Fixture;

fn playlist() -> &'static str {
    "#EXTM3U\n#EXTINF:7200 type=\"movie\" year=\"2020\" genre=\"Dramma,Avventura\" plot=\"Una trama <b>locale</b>\" poster=\"https://poster.example/a.jpg?secret=hidden\" group-title=\"Film\",Il film (2020)\nhttps://media.example/movie/user/password/1.mp4\n#EXTINF:1800 type=\"series\" group-title=\"Serie;Preferite\",La serie S02E01\nhttps://media.example/series/user/password/21.mp4\n#EXTINF:1800 type=\"series\" group-title=\"Serie\",La serie S01E02\nhttps://media.example/series/user/password/12.mp4\n#EXTINF:1800 type=\"series\" group-title=\"Serie\",La serie S01E01\nhttps://media.example/series/user/password/11.mp4\n#EXTINF:1800 type=\"series\" group-title=\"Serie\",La serie S01E01\nhttps://media.example/series/user/password/11-hd.mp4\n#EXTINF:-1 type=\"live\",Diretta\nhttps://media.example/live/user/password/3.ts\n"
}
fn query(source: &str) -> Query {
    Query {
        source: source.into(),
        ..Default::default()
    }
}
fn page(f: &Fixture, q: Query) -> Value {
    f.library.videos(&q, &[]).unwrap()
}
fn episodes(f: &Fixture, source: &str) -> Value {
    page(
        f,
        Query {
            episodes: true,
            kind: "episode".into(),
            ..query(source)
        },
    )
}

#[test]
fn films_series_seasons_and_continuity_match_android_without_exporting_resources() {
    let f = Fixture::new();
    let source = f.import(playlist(), None);
    let all = page(&f, query(&source));
    assert_eq!(all["total"], 2);
    let film = &all["rows"][0];
    assert_eq!(film["kind"], "movie");
    assert_eq!(film["year"], 2020);
    assert_eq!(film["description"], "Una trama locale");
    assert_eq!(film["genres"][0], "Dramma");
    assert_eq!(film["poster"], true);
    let series = &all["rows"][1];
    assert_eq!(series["series_title"], "La serie");
    assert_eq!(series["episodes"], 4);
    let detail = f
        .library
        .video_detail(series["id"].as_str().unwrap(), &[])
        .unwrap();
    assert_eq!(detail["seasons"][0]["season"], 1);
    assert_eq!(detail["seasons"][0]["episodes"], 3);
    assert_eq!(detail["seasons"][1]["season"], 2);
    let rows = episodes(&f, &source);
    let first = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["season"] == 1 && i["episode"] == 1)
        .unwrap();
    let id = first["id"].as_str().unwrap();
    let target = f.library.video_detail(id, &[]).unwrap();
    assert_eq!(target["target_item"]["season"], 1);
    assert_eq!(target["target_item"]["episode"], 1);
    let next = target["next"].as_str().unwrap();
    let next = f.library.video_detail(next, &[]).unwrap();
    assert_eq!(next["item"]["episode"], 2);
    assert_eq!(next["item"]["season"], 1);
    f.library
        .watch(id, "vod", "", 600.0, 1800.0, false)
        .unwrap();
    let resumed = f
        .library
        .video_detail(series["id"].as_str().unwrap(), &[])
        .unwrap();
    assert_eq!(resumed["target"], id);
    assert_eq!(resumed["target_item"]["position"], 600.0);
    assert!(!format!("{all}{detail}{resumed}").contains("secret="));
    assert!(!all.to_string().contains("password"));
    assert!(!all.to_string().contains("https://"));
}

#[test]
fn continue_saved_and_manual_decisions_survive_refresh_and_restart() {
    let f = Fixture::new();
    let source = f.import(playlist(), None);
    let rows = page(&f, query(&source));
    let movie = rows["rows"][0]["id"].as_str().unwrap();
    let series = rows["rows"][1]["id"].as_str().unwrap();
    f.library
        .watch(movie, "vod", "", 55.0, 7200.0, false)
        .unwrap();
    f.library.video_action(series, "saved", true).unwrap();
    assert_eq!(
        page(
            &f,
            Query {
                shelf: "continue".into(),
                ..query(&source)
            }
        )["total"],
        1
    );
    assert_eq!(
        page(
            &f,
            Query {
                shelf: "saved".into(),
                ..query(&source)
            }
        )["rows"][0]["episodes"],
        4
    );
    let date = rows["rows"][0]["added"].clone();
    f.import(playlist(), Some(&source));
    assert_eq!(page(&f, query(&source))["rows"][0]["added"], date);
    let restored = Library::new(&f.root).unwrap();
    assert_eq!(
        restored
            .videos(
                &Query {
                    shelf: "saved".into(),
                    ..query(&source)
                },
                &[]
            )
            .unwrap()["total"],
        1
    );
    f.library.video_action(movie, "hidden", true).unwrap();
    assert_eq!(
        page(
            &f,
            Query {
                shelf: "continue".into(),
                ..query(&source)
            }
        )["total"],
        0
    );
    assert_eq!(
        f.library.playback(movie, "vod", "", None).unwrap().position,
        55.0
    );
    assert_eq!(
        page(
            &f,
            Query {
                shelf: "continue".into(),
                ..query(&source)
            }
        )["total"],
        1
    );
    let playing = f.library.playback(movie, "vod", "", None).unwrap();
    f.library.video_action(movie, "completed", true).unwrap();
    f.library
        .watch_checked(
            movie,
            "vod",
            "",
            99.0,
            7200.0,
            false,
            Some(playing.watch_revision),
        )
        .unwrap();
    assert_eq!(
        f.library.video_detail(movie, &[]).unwrap()["item"]["completed"],
        true
    );
    let replay = f.library.playback(movie, "vod", "", None).unwrap();
    assert_eq!(replay.position, 0.0);
    assert!(replay.watch_revision > playing.watch_revision);
    f.library
        .watch_checked(
            movie,
            "vod",
            "",
            100.0,
            7200.0,
            false,
            Some(replay.watch_revision),
        )
        .unwrap();
    assert_eq!(
        f.library.video_detail(movie, &[]).unwrap()["item"]["position"],
        100.0
    );
}

#[test]
fn lazy_index_migration_group_filters_and_ten_sources_are_supported() {
    let f = Fixture::new();
    let source = f.import(playlist(), None);
    let db = open(&f.library.path(&source).unwrap()).unwrap();
    db.execute_batch(
        "DROP TABLE video;DROP TABLE video_titles;DELETE FROM metadata WHERE key='video_index';",
    )
    .unwrap();
    drop(db);
    assert_eq!(
        page(
            &f,
            Query {
                group: "Film".into(),
                ..query(&source)
            }
        )["total"],
        1
    );
    assert_eq!(
        page(
            &f,
            Query {
                query: "%".into(),
                ..query(&source)
            }
        )["total"],
        0
    );
    assert_eq!(
        page(
            &f,
            Query {
                season: Some(1),
                episodes: true,
                kind: "episode".into(),
                ..query(&source)
            }
        )["total"],
        3
    );
    for _ in 0..9 {
        f.import(playlist(), None);
    }
    assert_eq!(page(&f, query(""))["total"], 20);
    assert_eq!(
        page(
            &f,
            Query {
                offset: 19,
                limit: 1,
                ..query("")
            }
        )["rows"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let old = f.library.source(&source).unwrap();
    f.library
        .edit(&source, old.revision, &old.name, false, old.settings)
        .unwrap();
    assert_eq!(page(&f, query(""))["total"], 18);
}

#[test]
fn classification_respects_explicit_live_and_provider_path_boundaries() {
    let f = Fixture::new();
    let source=f.import("#EXTM3U\n#EXTINF:123 type=\"live\" group-title=\"Film\",Diretta\nhttps://media.example/movie/user/pass/1.mp4\n#EXTINF:-1,Film provider\nhttps://media.example/movie/user/pass/2.mkv\n#EXTINF:-1,Nome S01E01\nhttps://media.example/live/user/pass/3.ts\n#EXTINF:-1 type=\"episode\",Serie 1x02\nhttps://media.example/special.mp4\n",None);
    let result = f.rows(&source);
    assert_eq!(result["rows"][0]["content"], "live");
    assert_eq!(result["rows"][1]["content"], "vod");
    assert_eq!(result["rows"][2]["content"], "live");
    assert_eq!(result["rows"][3]["content"], "vod");
    assert_eq!(page(&f, query(&source))["total"], 2);
}

#[test]
fn relative_posters_and_backdrops_follow_the_playlist_base_instead_of_the_stream_host() {
    let settings = crate::personal::Settings::default();
    let context = crate::personal::Context::new(&settings).unwrap();
    let mut rows = Vec::new();
    {
        let mut sink = |entry: &Entry| {
            rows.push(entry.clone());
            Ok(true)
        };
        let check = || Ok(());
        let progress = |_, _| {};
        let mut scanner =
            crate::personal::parser::Scanner::new(&settings, context, &mut sink, &check, &progress);
        scanner.scan(Box::new(std::io::Cursor::new(b"#EXTM3U\n#EXTINF:120 type=\"movie\" poster=\"../covers/poster.jpg\" backdrop=\"/art/backdrop.webp\",Film\nhttps://media.example/video.mp4\n")),Some("https://lists.example/catalog/list.m3u")).unwrap();
    }
    let meta = Metadata::from(&rows[0]);
    assert_eq!(meta.poster, "https://lists.example/covers/poster.jpg");
    assert_eq!(meta.backdrop, "https://lists.example/art/backdrop.webp");
}

#[test]
#[ignore = "Full 500,000-title on-disk VOD capacity check; run explicitly after index changes"]
fn half_million_vod_titles_keep_storage_and_query_windows_bounded() {
    use std::fmt::Write;
    let f = Fixture::new();
    let mut input = String::with_capacity(70 * 1024 * 1024);
    input.push_str("#EXTM3U\n");
    for i in 0..500_000 {
        writeln!(input,"#EXTINF:5400 type=\"movie\" group-title=\"Cinema\",Film {i:06}\nhttps://media.example/movie/user/private/{i}.mp4").unwrap();
    }
    let start = std::time::Instant::now();
    let source = f.import(&input, None);
    let seconds = start.elapsed().as_secs_f64();
    drop(input);
    let start = std::time::Instant::now();
    let result = page(
        &f,
        Query {
            offset: 499_999,
            limit: 200,
            ..query(&source)
        },
    );
    assert_eq!(result["total"], 500_000);
    assert_eq!(result["rows"].as_array().unwrap().len(), 1);
    assert_eq!(result["rows"][0]["title"], "Film 499999");
    let home = f.library.video_home(&source, &[]).unwrap();
    assert_eq!(home["shelves"][0]["rows"].as_array().unwrap().len(), 12);
    let found = page(
        &f,
        Query {
            query: "Film 499999".into(),
            order: "title".into(),
            ..query(&source)
        },
    );
    assert_eq!(found["total"], 1);
    let size = std::fs::metadata(f.library.path(&source).unwrap())
        .unwrap()
        .len();
    assert!(size <= crate::personal::MAX_DATABASE);
    eprintln!("VOD capacity: titles=500000, import_seconds={seconds:.2}, query_seconds={:.3}, snapshot_bytes={size}",start.elapsed().as_secs_f64());
}

#[test]
fn saved_catalog_reclassification_matches_android_precedence_without_network_reads() {
    let f = Fixture::new();
    let source=f.import("#EXTM3U\n#EXTINF:-1 group-title=\"Fílm\",Un film\nhttps://media.example/files/film.mp4\n#EXTINF:-1 group-title=\"Fílm\",Diretta\nhttps://media.example/live/programme.mp4\n#EXTINF:-1 group-title=\"Fílm\" catchup=\"append\",Archivio\nhttps://media.example/files/archive.mp4\n#EXTINF:600 type=\"movie\" group-title=\"Forzato;Altro\",Esplicito\nhttps://media.example/file.mp4\n",None);
    let before = f.rows(&source);
    let id = before["rows"][0]["id"].as_str().unwrap();
    assert_eq!(before["rows"][0]["content"], "live");
    let original = f.library.source(&source).unwrap();
    let mut settings = original.settings.clone();
    settings.vod_detection = "extended".into();
    settings
        .group_content
        .insert("Forzato".into(), "vod".into());
    settings.group_content.insert("altro".into(), "live".into());
    f.library
        .edit(
            &source,
            original.revision,
            &original.name,
            true,
            settings.clone(),
        )
        .unwrap();
    let rows = f.rows(&source);
    assert_eq!(rows["rows"][0]["id"], id);
    assert_eq!(rows["rows"][0]["content"], "vod");
    for index in 1..4 {
        assert_eq!(rows["rows"][index]["content"], "live");
    }
    assert_eq!(page(&f, query(&source))["total"], 1);
    settings.group_content.insert("fílm".into(), "live".into());
    let current = f.library.source(&source).unwrap();
    f.library
        .edit(
            &source,
            current.revision,
            &current.name,
            true,
            settings.clone(),
        )
        .unwrap();
    assert_eq!(page(&f, query(&source))["total"], 0);
    assert_eq!(f.library.channel(id).unwrap().content, "live");
    settings.group_content.insert("FÍLM".into(), "vod".into());
    assert!(settings.validate().is_err());
}

#[test]
fn hidden_and_ordered_live_channels_apply_to_pages_and_zapping_but_keep_saved_records() {
    let f = Fixture::new();
    let source=f.import("#EXTM3U\n#EXTINF:-1,Primo\nhttps://media.example/one.m3u8\n#EXTINF:-1,Secondo\nhttps://media.example/two.m3u8\n#EXTINF:-1,Terzo\nhttps://media.example/three.m3u8\n",None);
    let rows = f.rows(&source);
    let ids = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    f.library
        .set_layout(&json!({"order":[ids[2],ids[0]],"hidden":[{"id":ids[1]}]}));
    let rows = f.rows(&source);
    assert_eq!(rows["total"], 2);
    assert_eq!(rows["rows"][0]["id"], ids[2]);
    assert_eq!(rows["rows"][1]["id"], ids[0]);
    assert!(f.library.channel(&ids[1]).is_none());
    assert_eq!(
        f.library
            .next(&ids[2], 1, &[], false)
            .unwrap()
            .unwrap()
            .channel
            .id,
        ids[0]
    );
    assert_eq!(
        f.library
            .next(&ids[0], 1, &[], false)
            .unwrap()
            .unwrap()
            .channel
            .id,
        ids[2]
    );
    assert!(f.library.entry(&ids[1]).is_ok());
}
