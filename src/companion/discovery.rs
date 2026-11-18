// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};
fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_be_bytes(
        data.get(offset..offset + 2)?.try_into().ok()?,
    ))
}
fn name(data: &[u8], offset: &mut usize) -> Option<String> {
    let mut at = *offset;
    let mut end = None;
    let mut parts = Vec::new();
    for _ in 0..32 {
        let n = *data.get(at)? as usize;
        at += 1;
        if n == 0 {
            *offset = end.unwrap_or(at);
            return Some(parts.join(".").to_ascii_lowercase());
        }
        if n & 0xc0 == 0xc0 {
            let next = *data.get(at)? as usize;
            end.get_or_insert(at + 1);
            at = ((n & 63) << 8) | next;
            continue;
        }
        if n > 63 {
            return None;
        }
        let part = std::str::from_utf8(data.get(at..at + n)?).ok()?;
        parts.push(part.to_owned());
        at += n;
        if parts.len() > 16 {
            return None;
        }
    }
    None
}
#[derive(Default)]
struct Records {
    services: BTreeSet<String>,
    servers: BTreeMap<String, (u16, String)>,
    addresses: BTreeMap<String, String>,
    ids: BTreeMap<String, String>,
}
impl Records {
    fn packet(&mut self, data: &[u8]) -> Option<()> {
        if data.len() < 12 || data.len() > 8192 {
            return None;
        }
        let questions = u16_at(data, 4)? as usize;
        let records = (u16_at(data, 6)? as usize)
            + (u16_at(data, 8)? as usize)
            + (u16_at(data, 10)? as usize);
        if questions > 32 || records > 256 {
            return None;
        }
        let mut at = 12;
        for _ in 0..questions {
            name(data, &mut at)?;
            at += 4;
        }
        for _ in 0..records {
            let owner = name(data, &mut at)?;
            let kind = u16_at(data, at)?;
            let class = u16_at(data, at + 2)? & 0x7fff;
            let length = u16_at(data, at + 8)? as usize;
            at += 10;
            let end = at.checked_add(length)?;
            data.get(at..end)?;
            if class == 1 {
                match kind {
                    12 if owner == "_lumetv._tcp.local" => {
                        let mut start = at;
                        self.services.insert(name(data, &mut start)?);
                    }
                    33 if length >= 7 => {
                        let port = u16_at(data, at + 4)?;
                        let mut start = at + 6;
                        let host = name(data, &mut start)?;
                        if port > 0 {
                            self.servers.insert(owner, (port, host));
                        }
                    }
                    1 if length == 4 => {
                        let p = &data[at..end];
                        let ip = format!("{}.{}.{}.{}", p[0], p[1], p[2], p[3]);
                        if local_host(&ip) {
                            self.addresses.insert(owner, ip);
                        }
                    }
                    16 => {
                        let mut pos = at;
                        while pos < end {
                            let len = data[pos] as usize;
                            pos += 1;
                            if pos + len > end {
                                return None;
                            }
                            if let Ok(text) = std::str::from_utf8(&data[pos..pos + len]) {
                                if let Some(id) = text.strip_prefix("id=") {
                                    if uuid(id) {
                                        self.ids.insert(owner.clone(), id.into());
                                    }
                                }
                            }
                            pos += len;
                        }
                    }
                    _ => {}
                }
            }
            at = end;
        }
        Some(())
    }
    fn endpoints(&self) -> Vec<Endpoint> {
        self.services
            .iter()
            .filter_map(|service| {
                let (port, host) = self.servers.get(service)?;
                Some(Endpoint {
                    host: self.addresses.get(host)?.clone(),
                    port: *port,
                    id: self.ids.get(service).cloned().unwrap_or_default(),
                    name: display(
                        service.strip_suffix("._lumetv._tcp.local").unwrap_or("TV"),
                        80,
                    ),
                })
            })
            .take(16)
            .collect()
    }
}
pub(super) fn discover() -> Result<Vec<Endpoint>> {
    let socket = UdpSocket::bind("0.0.0.0:0")
        .map_err(|_| anyhow!("Ricerca TV non disponibile. Usa l’indirizzo mostrato sulla TV."))?;
    socket.set_read_timeout(Some(Duration::from_millis(250)))?;
    socket.set_multicast_ttl_v4(1)?;
    let mut query = vec![0; 12];
    query[5] = 1;
    for part in ["_lumetv", "_tcp", "local"] {
        query.push(part.len() as u8);
        query.extend_from_slice(part.as_bytes());
    }
    query.extend_from_slice(&[0, 0, 12, 0x80, 1]);
    socket.send_to(&query, "224.0.0.251:5353")?;
    let start = Instant::now();
    let mut records = Records::default();
    let mut buffer = [0; 8192];
    while start.elapsed() < Duration::from_secs(3) {
        if let Ok((n, peer)) = socket.recv_from(&mut buffer) {
            if local_host(&peer.ip().to_string()) {
                records.packet(&buffer[..n]);
            }
        }
    }
    Ok(records.endpoints())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compression_loops_and_bad_lengths_are_bounded() {
        let mut at = 0;
        assert!(name(&[0xc0, 0], &mut at).is_none());
        let mut records = Records::default();
        assert!(records.packet(&[0; 8]).is_none());
    }
}
