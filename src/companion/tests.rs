use super::*;
use base64::Engine;
#[test]
fn private_addresses_and_invitation_proofs_match_android() {
    for host in ["192.168.1.20", "10.0.0.2", "172.16.2.4", "169.254.2.4"] {
        assert!(local_host(host));
    }
    for host in [
        "127.0.0.1",
        "localhost",
        "192.168.01.2",
        "192.168.1.255",
        "172.32.0.2",
        "8.8.8.8",
        "[::1]",
        "10.1.1.0",
    ] {
        assert!(!local_host(host));
    }
    let a = proof(
        "23456789ABCDEFGH",
        "server",
        &"a".repeat(64),
        &"b".repeat(64),
        &"c".repeat(64),
        "e55b4f9d-1865-491e-9585-ae423ad8aca1",
        "",
    );
    assert_eq!(a, include_str!("../../tests/fixtures/companion/proof.txt"));
    assert!(same(&a, &a));
    assert!(!same(&a, &format!("{a}0")));
    assert_ne!(
        a,
        proof(
            "23456789ABCDEFGH",
            "client",
            &"a".repeat(64),
            &"b".repeat(64),
            &"c".repeat(64),
            "e55b4f9d-1865-491e-9585-ae423ad8aca1",
            "Lume Desktop"
        )
    );
}
#[test]
fn certificate_parser_rejects_truncated_or_unbounded_der() {
    for bytes in [
        &[][..],
        &[0x30, 0x80],
        &[0x30, 0x84, 255, 255, 255, 255],
        &[0x30, 1, 0],
    ] {
        assert!(transport::spki(bytes).is_none());
    }
}
#[test]
fn saved_tv_tokens_never_enter_public_device_snapshots() {
    let root = std::env::temp_dir().join(format!("lume-companion-{}", uuid::Uuid::new_v4()));
    let service = Companion::new(&root).unwrap();
    service.store.lock().unwrap().tvs.push(Television {
        endpoint: Endpoint {
            host: "192.168.1.20".into(),
            port: 4443,
            id: uuid::Uuid::new_v4().to_string(),
            name: "Salotto".into(),
        },
        pin: "a".repeat(64),
        token: "fixture-secret".into(),
    });
    let public = service.devices().to_string();
    assert!(!public.contains("fixture-secret"));
    assert!(!public.contains("pin"));
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn certificate_spki_matches_the_independent_openssl_vector() {
    let cert = base64::engine::general_purpose::STANDARD
        .decode(include_str!("../../tests/fixtures/companion/cert.der.base64").trim())
        .unwrap();
    assert_eq!(
        format!(
            "{:x}",
            sha2::Sha256::digest(transport::spki(&cert).unwrap())
        ),
        include_str!("../../tests/fixtures/companion/pin.txt")
    );
}

fn fixture_server() -> (u16, std::thread::JoinHandle<String>) {
    use std::io::{BufRead, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = std::thread::spawn(move || {
        let cert = rustls::pki_types::CertificateDer::from(
            base64::engine::general_purpose::STANDARD
                .decode(include_str!("../../tests/fixtures/companion/cert.der.base64").trim())
                .unwrap(),
        );
        let key =
            rustls::pki_types::PrivateKeyDer::Pkcs8(rustls::pki_types::PrivatePkcs8KeyDer::from(
                base64::engine::general_purpose::STANDARD
                    .decode(include_str!("../../tests/fixtures/companion/key.der.base64").trim())
                    .unwrap(),
            ));
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
        let (socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(4)))
            .unwrap();
        let mut stream = rustls::StreamOwned::new(
            rustls::ServerConnection::new(Arc::new(config)).unwrap(),
            socket,
        );
        let mut headers = String::new();
        let mut body = Vec::new();
        {
            let mut reader = std::io::BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => return String::new(),
                    _ => {}
                }
                if line == "\r\n" {
                    break;
                }
                headers.push_str(&line);
                if headers.len() > 8192 {
                    return String::new();
                }
            }
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|v| v.parse::<usize>().ok())
                })
                .unwrap_or(0);
            if length > 32768 {
                return String::new();
            }
            body.resize(length, 0);
            if reader.read_exact(&mut body).is_err() {
                return String::new();
            }
        }
        let payload = br#"{"version":1,"session":"fixture:1","active":false}"#;
        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",payload.len());
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.write_all(payload);
        let _ = stream.flush();
        headers + &String::from_utf8_lossy(&body)
    });
    (port, worker)
}
#[test]
fn local_tls_accepts_the_exact_pin_and_blocks_credentials_on_identity_change() {
    let (port, server) = fixture_server();
    let client = Connection::fixture(
        port,
        include_str!("../../tests/fixtures/companion/pin.txt").into(),
        "fixture-secret-token".into(),
    )
    .unwrap();
    let value = client.exchange("/v1/state", None, true).unwrap();
    assert_eq!(value["version"], 1);
    let received = server.join().unwrap().to_ascii_lowercase();
    assert!(received.contains("authorization: bearer fixture-secret-token"));
    assert!(!received.contains("origin:"));
    let (port, server) = fixture_server();
    let client = Connection::fixture(port, "0".repeat(64), "fixture-secret-token".into()).unwrap();
    assert!(client
        .exchange(
            "/v1/command",
            Some(&json!({"private":"fixture-value"})),
            true
        )
        .is_err());
    assert_eq!(server.join().unwrap(), "");
}
