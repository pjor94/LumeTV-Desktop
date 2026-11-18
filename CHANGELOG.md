# Changelog

## 0.3.0

- EPGShare IT1 integra guida e loghi delle liste personali solo dopo i dati dichiarati dalla lista e quelli Lume; cache, aggiornamenti condizionali e pause del servizio restano separati dalla guida principale.
- La guida riconosce anche Sky Uno Plus, Eurosport 1 e 2, le rispettive voci DAZN e Solo Calcio quando la lista usa gli identificativi italiani verificati.
- Liste personali: guida e loghi Lume quando assenti nella lista, con ricerca nell’intero feed XMLTV, anche per canali esterni al Catalogo Lume; i dati dichiarati dalla sorgente mantengono la precedenza.
- Riproduzione personale: aggiornamento singolo della sorgente su HTTP 401/403/404/410, con riapertura del canale e annullamento quando si cambia contenuto.
- Flussi live Xtream TS: selezione della rappresentazione HLS offerta dallo stesso servizio; supporto alle CDN con IP pubblico esplicito, mantenendo il blocco degli indirizzi non pubblici e lo scoping degli header.
- Diagnostica: le risposte HTTP di errore dei servizi personali incrementano anche il contatore delle richieste fallite.
- Backup `.lumebackup` Android v1: cifratura a frame autenticati, anteprima, ripristino selettivo e recupero dopo interruzioni.
- Companion per TV Android: ricerca in LAN, associazione con codice o invito, pin TLS, telecomando, verifica/avvio dei contenuti e trasferimento esplicito delle liste.
- Riconoscimento VOD prudente/esteso e regole per gruppo applicabili anche ai cataloghi già salvati.
- Home Video con anteprima del titolo e categorie caricate quando diventano visibili.
- TMDB: normalizzazione Unicode, suggerimenti ordinati per somiglianza, `tmdb-type` per video generici, regia/creatori e precedenza dei metadati della lista.
- Preferenze audio/sottotitoli persistenti, ordine dei canali e gestione dei canali nascosti, inclusi nei backup.
- Collegamenti Discovery `localplay` nelle liste e nei backup Android, con formato e DRM forniti dal resolver del canale.

- Libreria VOD da Android: raccolte Film/Serie, stagioni ed episodi, Continua a
  guardare, La mia lista e aggiunti di recente; schede con locandine e dettagli.
- Informazioni TMDB facoltative con ricerca e associazione manuale, cache locale,
  token configurabile e trasporto separato dalle credenziali delle liste.
- Episodio successivo, visto/non visto e rimozione da Continua a guardare;
  le decisioni manuali non vengono sovrascritte da un player precedente.
- Indice VOD a finestre con dati estesi letti solo per i titoli visibili e
  locandine caricate quando entrano nell’area di lettura, con cache limitata.
- Etichette del player distinte per video, archivio e diretta; gli errori di
  riproduzione non vengono presentati come una richiesta di avvio con audio.
- Home delle liste personali con canale in evidenza, visti di recente e accesso
  alla guida della lista; gli stessi componenti sono condivisi dal Catalogo Lume.
- Loghi sostitutivi adattati allo spazio disponibile e ripristino della griglia
  dopo il player anche quando cambia il contenuto della home.
- Le liste personali usano le stesse schede del Catalogo Lume, con programma
  in onda, avanzamento e preferiti; griglia adattiva e scorrimento della pagina
  mantengono caricamento limitato e posizione anche su liste molto grandi.
- Catalogo Lume: richieste della guida e dei loghi allineate ad Android, ripresa
  alla scadenza del servizio, loghi della lista ufficiale e delle edizioni TGR,
  immagini PNG/JPEG/GIF/WebP e associazione del feed TGR Veneto.
- Il logo caricato nasconde il segnaposto, evitando scritte sovrapposte.
- Corretto il passaggio da ClearKey a Widevine (Mediaset): azzeramento esplicito
  delle mappe DRM del player persistente prima di configurare la nuova sessione.
- Corretto ClearKey con chiavi esplicite quando il manifest DASH annuncia altri
  sistemi DRM; prova di regressione con video CENC e decodifica Shaka reale.
- Gli header User-Agent personalizzati sostituiscono il valore predefinito
  senza produrre header duplicati.
- Liste personali M3U da URL/file, anteprima annullabile, gestione delle sorgenti,
  gruppi, formati e codifiche, aggiornamenti con ultima copia valida conservata.
- Cataloghi SQLite e liste continue con ricerca sull’intero archivio: fino a
  500.000 voci per sorgente e un milione complessivo, mantenendo memoria e DOM limitati.
- Header e cookie separati per ruolo, loghi privati, Widevine/ClearKey Kodi e chiavi HLS.
- Guida XMLTV/gzip/XZ personale con storico, lingua, fuso e associazioni manuali.
- Archivio e inizio del programma in corso, quando disponibili dalla sorgente,
  con avanzamento e ritorno esplicito alla diretta.
- Sezione Video con ripresa e riavvio; cronologia distinta dalle dirette.
- Porting desktop delle funzioni Android, esclusi solo audio, risparmio dati e controlli di sistema.

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
