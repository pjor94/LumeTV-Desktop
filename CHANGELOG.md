# Changelog

## 0.2.0

- macOS: avvio della UI in memoria senza la richiesta “Chromium Safe Storage”,
  disabilitando l’inizializzazione preventiva del provider Portachiavi di Chromium.
- Sezione Radio con 1.310 stazioni italiane di WorldRadio, ricerca per nome e città,
  preferite, recenti e catalogo aggiornato ogni giorno con copia locale.
- Riproduzione radio MP3/AAC/HLS nel player integrato, mini-player e titolo del
  brano quando fornito dalla stazione; riconnessione con tentativi limitati.
- Aggiornamenti dell’app dalle release GitHub: controllo in background, download
  con percentuale e annullamento, firma Ed25519 e verifica SHA-256.
- Installazione del pacchetto completo con riavvio per bundle e cartelle portabili
  scrivibili; gestore pacchetti per il DEB e copia precedente conservata.
- Feed firmato nel job di pubblicazione e helper incluso nei quattro pacchetti.
- Preparazione Widevine all’avvio: schermata dedicata prima del catalogo,
  verifica senza aprire una diretta e riprova in caso di errore.
- Interfaccia React e TypeScript con stile, navigazione e animazioni di Lume.
- Player persistente e logica separata per catalogo, guida e preferenze.
- Stato del download Widevine dal component updater CEF, barra di attività,
  errori con riprova e indicazione di riavvio su Linux dopo una nuova installazione.
- Build frontend riproducibile, asset incorporati verificati e test browser in CI.
- Release GitHub ordinarie per i tag di versione; anteprime solo per tag beta/rc.
- Canale delle anteprime disattivato per impostazione iniziale.
- README e note di rilascio aggiornati, con istruzioni per chi arriva dalla 0.1.0.

## 0.1.0 — 10 novembre 2026

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

Nella release 0.1.0 le prove desktop Windows/Linux, gli installer di produzione
e l’aggiornamento dell’applicazione restavano da completare.
Vedi [stato delle verifiche](docs/STATUS.md).
