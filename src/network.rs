// SPDX-License-Identifier: GPL-3.0-or-later
//! Resolve only public addresses for media supplied by the channel catalogue.
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    Url,
};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};

pub fn public_host(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default().trim_end_matches('.');
    ["http", "https"].contains(&url.scheme())
        && url.username().is_empty()
        && url.password().is_none()
        && host.contains('.')
        && host.parse::<IpAddr>().is_err()
        && ![".localhost", ".local", ".internal", ".lan"]
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_broadcast()
                && !ip.is_documentation()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 198 && (18..=19).contains(&b))
                && !(a == 192 && b == 0 && c == 0)
        }
        IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && [0, 0x0db8].contains(&s[1]))
                && s[0] != 0x2002
        }
    }
}

fn check_addresses(addresses: Vec<SocketAddr>) -> std::io::Result<Addrs> {
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "non_public_media_address",
        ));
    }
    Ok(Box::new(addresses.into_iter()))
}

pub struct PublicDns;
impl Resolve for PublicDns {
    fn resolve(&self, name: Name) -> Resolving {
        let name = name.as_str().to_owned();
        Box::pin(async move {
            let addresses = tokio::task::spawn_blocking(move || {
                (name.as_str(), 0).to_socket_addrs().map(|v| v.collect())
            })
            .await
            .map_err(|_| std::io::Error::other("media_dns_worker"))??;
            Ok(check_addresses(addresses)?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dns_rebinding_and_private_or_mapped_addresses_are_rejected_before_connecting() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "172.16.5.2",
            "192.168.1.2",
            "169.254.169.254",
            "100.64.0.1",
            "198.18.0.1",
            "0.1.2.3",
            "224.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        assert!(public_ip("1.1.1.1".parse().unwrap()));
        assert!(public_ip("2606:4700:4700::1111".parse().unwrap()));
        assert!(check_addresses(vec![
            "1.1.1.1:443".parse().unwrap(),
            "127.0.0.1:443".parse().unwrap()
        ])
        .is_err());
    }
}
