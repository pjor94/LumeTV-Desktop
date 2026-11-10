# Changelog

## 0.1.0 — pre-release

- Prototipo Rust con player CEF incorporato; bundle di sviluppo macOS arm64.
- Catalogo di 126 canali, aggiornamento della lista e guida XMLTV/XZ con cache locale.
- Ricerca, filtri, preferiti, recenti e promemoria mentre l’app è aperta.
- Player, mini-player, fullscreen, temi, tastiera e animazioni ridotte.
- Download Widevine tramite Component Updater nel profilo, senza CDM nel bundle.
- Diagnostica separata di manifest, video, audio e DRM.
- CI del core/API CEF per macOS, Windows e Linux, test UI e strumenti di packaging.
- Controlli sui file pubblici e documentazione per la pubblicazione su GitHub.
- Pipeline native con DMG/ZIP Mac arm64 e Intel, ZIP Windows x64 e DEB/tar.gz Linux x64.
- Pre-release da tag, checksum e sorgenti corrispondenti con dipendenze Cargo.
- Bootstrap sandbox Windows, SDK OBS verificati e raccolta degli avvisi nei pacchetti.

Windows/Linux desktop, installer di produzione e aggiornamento automatico
dell’applicazione restano da completare. Vedi [stato delle verifiche](docs/STATUS.md).
