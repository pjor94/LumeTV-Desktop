// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded streaming bridge for continuous radio, with ICY metadata removed.
use anyhow::{anyhow, bail, Result};
use reqwest::{Client, Url};
use serde::Serialize;
use std::{
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Serialize)]
pub struct AudioStatus {
    pub mime_type: String,
    pub now_playing: String,
    pub state: String,
    pub bytes: u64,
}
struct Shared {
    cancelled: AtomicBool,
    title: Mutex<String>,
    state: Mutex<String>,
    bytes: AtomicU64,
}
pub struct AudioSession {
    pub capability: String,
    pub mime: String,
    shared: Arc<Shared>,
    receiver: Mutex<Option<Receiver<Vec<u8>>>>,
}
pub enum Connection {
    Audio(Arc<AudioSession>),
    Manifest(&'static str),
}
enum Prepared {
    Audio(String),
    Manifest(&'static str),
}
impl AudioSession {
    pub fn connect(url: &str) -> Result<Connection> {
        let url = Url::parse(url)?;
        if !crate::radio::allowed_url(&url) {
            bail!("radio_address");
        }
        let shared = Arc::new(Shared {
            cancelled: AtomicBool::new(false),
            title: Mutex::new(String::new()),
            state: Mutex::new("connecting".into()),
            bytes: AtomicU64::new(0),
        });
        let (sender, receiver) = mpsc::sync_channel(4);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = shared.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(produce(url, worker.clone(), sender, &ready_tx))
            })();
            if result.is_err() {
                let _ = ready_tx.try_send(Err(()));
                *worker.state.lock().expect("audio state") = "interrupted".into();
            }
        });
        match ready_rx.recv_timeout(Duration::from_secs(15)) {
            Ok(Ok(Prepared::Audio(mime))) => Ok(Connection::Audio(Arc::new(Self {
                capability: uuid::Uuid::new_v4().to_string(),
                mime,
                shared,
                receiver: Mutex::new(Some(receiver)),
            }))),
            Ok(Ok(Prepared::Manifest(mime))) => {
                shared.cancelled.store(true, Ordering::Release);
                Ok(Connection::Manifest(mime))
            }
            _ => {
                shared.cancelled.store(true, Ordering::Release);
                Err(anyhow!("radio_unavailable"))
            }
        }
    }
    pub fn snapshot(&self) -> AudioStatus {
        AudioStatus {
            mime_type: self.mime.clone(),
            now_playing: self.shared.title.lock().expect("audio title").clone(),
            state: self.shared.state.lock().expect("audio state").clone(),
            bytes: self.shared.bytes.load(Ordering::Relaxed),
        }
    }
    pub fn stop(&self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
    pub fn reader(&self) -> Option<AudioReader> {
        Some(AudioReader {
            shared: self.shared.clone(),
            receiver: self.receiver.lock().ok()?.take()?,
            pending: Vec::new(),
            offset: 0,
        })
    }
}
impl Drop for AudioSession {
    fn drop(&mut self) {
        self.stop();
    }
}
async fn produce(
    url: Url,
    shared: Arc<Shared>,
    sender: SyncSender<Vec<u8>>,
    ready: &SyncSender<std::result::Result<Prepared, ()>>,
) -> Result<()> {
    let client = Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(crate::network::PublicDns))
        .connect_timeout(Duration::from_secs(8))
        .referer(false)
        .user_agent(crate::providers::USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() < 8 && crate::radio::allowed_url(a.url()) {
                a.follow()
            } else {
                a.error("radio_redirect")
            }
        }))
        .build()?;
    let mut response = tokio::time::timeout(
        Duration::from_secs(12),
        client.get(url).header("Icy-MetaData", "1").send(),
    )
    .await??
    .error_for_status()?;
    let content = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if let Some(mime) = manifest_mime(&content, response.url().path()) {
        let _ = ready.send(Ok(Prepared::Manifest(mime)));
        return Ok(());
    }
    let mime = match content.as_str() {
        "audio/mpeg" | "audio/mp3" => "audio/mpeg",
        "audio/aac" | "audio/aacp" | "audio/x-aac" => "audio/aac",
        "audio/ogg" | "application/ogg" => "audio/ogg",
        "audio/flac" | "audio/x-flac" => "audio/flac",
        "audio/mp4" => "audio/mp4",
        "application/octet-stream" | "" => {
            if response.url().path().to_lowercase().ends_with(".aac") {
                "audio/aac"
            } else {
                "audio/mpeg"
            }
        }
        _ => bail!("radio_format"),
    };
    let interval = response
        .headers()
        .get("icy-metaint")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok());
    if interval.is_some_and(|v| v == 0 || v > 16 * 1024 * 1024) {
        bail!("radio_metadata");
    }
    let mut decoder = IcyDecoder::new(interval);
    let mut id3 = Id3Probe::default();
    ready
        .send(Ok(Prepared::Audio(mime.into())))
        .map_err(|_| anyhow!("radio_cancelled"))?;
    let mut received = Instant::now();
    while !shared.cancelled.load(Ordering::Acquire) {
        let chunk = match tokio::time::timeout(Duration::from_millis(250), response.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                received = Instant::now();
                bytes
            }
            Err(_) if received.elapsed() < Duration::from_secs(15) => continue,
            _ => bail!("radio_interrupted"),
        };
        let (audio, title) = decoder.feed(&chunk);
        if let Some(title) = title.or_else(|| id3.feed(&audio)) {
            *shared.title.lock().expect("audio title") = title;
        }
        for part in audio.chunks(32 * 1024) {
            let mut pending = part.to_vec();
            loop {
                if shared.cancelled.load(Ordering::Acquire) {
                    return Ok(());
                }
                match sender.try_send(pending) {
                    Ok(()) => break,
                    Err(TrySendError::Full(value)) => {
                        pending = value;
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                    Err(TrySendError::Disconnected(_)) => return Ok(()),
                }
            }
            shared.bytes.fetch_add(part.len() as u64, Ordering::Relaxed);
            *shared.state.lock().expect("audio state") = "streaming".into();
        }
    }
    Ok(())
}
fn manifest_mime(content: &str, path: &str) -> Option<&'static str> {
    if content == "application/dash+xml" || path.to_lowercase().ends_with(".mpd") {
        Some("application/dash+xml")
    } else if [
        "application/x-mpegurl",
        "application/vnd.apple.mpegurl",
        "audio/mpegurl",
        "audio/x-mpegurl",
    ]
    .contains(&content)
        || path.to_lowercase().ends_with(".m3u8")
    {
        Some("application/x-mpegURL")
    } else {
        None
    }
}
pub struct AudioReader {
    shared: Arc<Shared>,
    receiver: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
}
impl Read for AudioReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        loop {
            if self.shared.cancelled.load(Ordering::Acquire) {
                return Ok(0);
            }
            if self.offset < self.pending.len() {
                let count = output.len().min(self.pending.len() - self.offset);
                output[..count].copy_from_slice(&self.pending[self.offset..self.offset + count]);
                self.offset += count;
                return Ok(count);
            }
            match self.receiver.recv_timeout(Duration::from_millis(250)) {
                Ok(bytes) => {
                    self.pending = bytes;
                    self.offset = 0;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Ok(0),
            }
        }
    }
}
impl Drop for AudioReader {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

pub fn clean_title(value: &str) -> String {
    let clean = value
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect::<String>()
        .trim()
        .to_owned();
    // Some RDS sources send Song*title*artist**catalogue-id as StreamTitle.
    if let Some(song) = clean.strip_prefix("Song*") {
        let parts: Vec<_> = song.trim_end_matches('*').split('*').collect();
        if parts.len() == 4
            && !parts[0].is_empty()
            && !parts[1].is_empty()
            && parts[2].is_empty()
            && !parts[3].is_empty()
            && parts[3].bytes().all(|c| c.is_ascii_digit())
        {
            return format!("{} – {}", parts[1], parts[0]);
        }
    }
    clean
}
fn icy_title(bytes: &[u8]) -> Option<String> {
    let raw = String::from_utf8(bytes.to_vec())
        .unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
    let title = raw.split("StreamTitle='").nth(1)?.split("';").next()?;
    let title = clean_title(title);
    (!title.is_empty()).then_some(title)
}
struct IcyDecoder {
    interval: Option<usize>,
    audio_left: usize,
    meta_left: Option<usize>,
    metadata: Vec<u8>,
}
impl IcyDecoder {
    fn new(interval: Option<usize>) -> Self {
        Self {
            interval,
            audio_left: interval.unwrap_or(0),
            meta_left: None,
            metadata: Vec::new(),
        }
    }
    fn feed(&mut self, mut bytes: &[u8]) -> (Vec<u8>, Option<String>) {
        let Some(interval) = self.interval else {
            return (bytes.to_vec(), None);
        };
        let mut audio = Vec::new();
        let mut title = None;
        while !bytes.is_empty() {
            if self.audio_left > 0 {
                let count = self.audio_left.min(bytes.len());
                audio.extend_from_slice(&bytes[..count]);
                self.audio_left -= count;
                bytes = &bytes[count..];
            } else if let Some(left) = self.meta_left {
                let count = left.min(bytes.len());
                self.metadata.extend_from_slice(&bytes[..count]);
                bytes = &bytes[count..];
                if left == count {
                    if let Some(value) = icy_title(&self.metadata) {
                        title = Some(value);
                    }
                    self.metadata.clear();
                    self.meta_left = None;
                    self.audio_left = interval;
                } else {
                    self.meta_left = Some(left - count);
                }
            } else {
                let length = bytes[0] as usize * 16;
                bytes = &bytes[1..];
                if length == 0 {
                    self.audio_left = interval;
                } else {
                    self.meta_left = Some(length);
                }
            }
        }
        (audio, title)
    }
}
#[derive(Default)]
struct Id3Probe {
    bytes: Vec<u8>,
    done: bool,
}
impl Id3Probe {
    fn feed(&mut self, bytes: &[u8]) -> Option<String> {
        if self.done {
            return None;
        }
        let count = bytes.len().min(128 * 1024 - self.bytes.len());
        self.bytes.extend_from_slice(&bytes[..count]);
        if self.bytes.len() < 10 {
            return None;
        }
        if &self.bytes[..3] != b"ID3"
            || !matches!(self.bytes[3], 3 | 4)
            || self.bytes[5] & 0xc0 != 0
        {
            self.done = true;
            return None;
        }
        let sync = |b: &[u8]| {
            b.iter()
                .fold(0usize, |v, &b| (v << 7) | (b & 0x7f) as usize)
        };
        let size = sync(&self.bytes[6..10]) + 10;
        if size > 128 * 1024 {
            self.done = true;
            return None;
        }
        if self.bytes.len() < size {
            return None;
        }
        self.done = true;
        let mut at = 10;
        while at + 10 <= size {
            let frame = &self.bytes[at..at + 10];
            let length = if self.bytes[3] == 4 {
                sync(&frame[4..8])
            } else {
                u32::from_be_bytes(frame[4..8].try_into().ok()?) as usize
            };
            at += 10;
            if length == 0 || at + length > size {
                break;
            }
            if &frame[..4] == b"TIT2" {
                let value = &self.bytes[at..at + length];
                let text = match value.first()? {
                    0 => value[1..].iter().map(|&b| b as char).collect(),
                    1 | 2 => {
                        let mut bytes = &value[1..];
                        let little = bytes.starts_with(&[0xff, 0xfe]);
                        if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
                            bytes = &bytes[2..];
                        }
                        let words: Vec<u16> = bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| {
                                if little {
                                    u16::from_le_bytes([b[0], b[1]])
                                } else {
                                    u16::from_be_bytes([b[0], b[1]])
                                }
                            })
                            .collect();
                        String::from_utf16_lossy(&words)
                    }
                    3 => String::from_utf8_lossy(&value[1..]).into_owned(),
                    _ => return None,
                };
                let title = clean_title(&text);
                return (!title.is_empty()).then_some(title);
            }
            at += length;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn continuous_reader_has_one_consumer_and_cancels_when_stopped_or_dropped() {
        let shared = Arc::new(Shared {
            cancelled: AtomicBool::new(false),
            title: Mutex::new(String::new()),
            state: Mutex::new("streaming".into()),
            bytes: AtomicU64::new(0),
        });
        let (sender, receiver) = mpsc::sync_channel(4);
        let session = AudioSession {
            capability: "test".into(),
            mime: "audio/mpeg".into(),
            shared: shared.clone(),
            receiver: Mutex::new(Some(receiver)),
        };
        let mut reader = session.reader().unwrap();
        assert!(session.reader().is_none());
        sender.send(b"audio".to_vec()).unwrap();
        let mut output = [0; 3];
        assert_eq!(reader.read(&mut output).unwrap(), 3);
        assert_eq!(&output, b"aud");
        session.stop();
        assert_eq!(reader.read(&mut output).unwrap(), 0);
        shared.cancelled.store(false, Ordering::Release);
        drop(reader);
        assert!(shared.cancelled.load(Ordering::Acquire));
    }
    #[test]
    fn icy_metadata_is_removed_across_arbitrary_network_boundaries() {
        let metadata = b"StreamTitle='Artist - Song';";
        let mut frame = b"abc".to_vec();
        frame.push(2);
        frame.extend_from_slice(metadata);
        frame.resize(3 + 1 + 32, 0);
        frame.extend_from_slice(b"def\0ghi");
        let mut decoder = IcyDecoder::new(Some(3));
        let mut audio = Vec::new();
        let mut titles = Vec::new();
        for byte in frame {
            let (a, t) = decoder.feed(&[byte]);
            audio.extend(a);
            if let Some(t) = t {
                titles.push(t);
            }
        }
        assert_eq!(audio, b"abcdefghi");
        assert_eq!(titles, ["Artist - Song"]);
    }
    #[test]
    fn unmarked_audio_is_unchanged_and_titles_are_bounded() {
        let (data, title) = IcyDecoder::new(None).feed(b"audio bytes");
        assert_eq!(data, b"audio bytes");
        assert!(title.is_none());
        assert_eq!(clean_title(" a\0\n b "), "a b");
        assert_eq!(clean_title(&"a".repeat(500)).len(), 300);
        assert_eq!(
            clean_title("Song*Waves*Mr. Probz**1106388"),
            "Mr. Probz – Waves"
        );
        assert_eq!(
            clean_title("Song*An incomplete title"),
            "Song*An incomplete title"
        );
    }
    #[test]
    fn id3_title_is_found_without_buffering_the_audio_stream() {
        let mut data =
            b"ID3\x03\x00\x00\x00\x00\x00\x0fTIT2\x00\x00\x00\x05\x00\x00\x03Song".to_vec();
        data.extend_from_slice(b"frames");
        let mut probe = Id3Probe::default();
        assert!(probe.feed(&data[..8]).is_none());
        assert_eq!(probe.feed(&data[8..]).as_deref(), Some("Song"));
        assert!(probe.feed(&data).is_none());
        let utf16 =
            b"ID3\x03\x00\x00\x00\x00\x00\x11TIT2\x00\x00\x00\x07\x00\x00\x01\xff\xfeS\x00\xec\x00";
        assert_eq!(Id3Probe::default().feed(utf16).as_deref(), Some("Sì"));
    }
}
