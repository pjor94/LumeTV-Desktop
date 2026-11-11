# Crediti, provenienza e licenze

## Lume TV e WorldLiveTV

Questo prototipo Rust prende come riferimento [Lume TV per Android](https://github.com/pjor94/LumeTV). Il codice e le risorse originali di Lume TV sono distribuiti con licenza **GPL-3.0-or-later**.

Il catalogo e la logica delle integrazioni di Lume TV derivano da **WorldLiveTV e WLTV Group**, autori dell'add-on Kodi `plugin.video.wltvhelper`:

- Progetto: [WorldLiveTV/WorldLiveTV.github.io](https://github.com/WorldLiveTV/WorldLiveTV.github.io).
- Riferimento registrato da Lume TV: add-on 1.2.14, hotfix 121417.
- Revisione upstream registrata: `af57d4ccbac56f6464b16a9dfaf4f0d7fb03d20e`.
- Licenza dichiarata: GPL-3.0-or-later.

**Avviso di modifica — 7 novembre 2026:** adattamento delle integrazioni Kotlin a un prototipo Rust desktop, con verifica dei manifest, relay HTTP locale, player Shaka nel browser e report di riproduzione. Questa versione è indipendente dall'add-on originale e dall'applicazione Android.

**Ulteriori modifiche — 8 novembre 2026:** companion e helper Rust per la finestra CEF incorporata, selezione esplicita del backend, packaging macOS, diagnostica separata codec/EME e preparazione di un CDM esterno nel solo profilo dedicato. Il progetto non include un CDM.

**UI desktop — 8 novembre 2026:** logo, simbolo e icona derivano dalle risorse originali Android `docs/brand`; il catalogo iniziale di 46 canali deriva dalla playlist locale Android. Le risorse web, la guida XMLTV/XZ con `lzma-rust2` 0.24.0 e la persistenza delle preferenze sono state aggiunte per il desktop. La fonte pubblica EPG è `api.epg-guide.com/get/wltv.xz`; loghi e palinsesti restano dei rispettivi titolari. Widevine è stato acquisito dal Component Updater nel solo profilo dell’app ed è escluso dal bundle.

**Catalogo completo — 8 novembre 2026:** estensione ai 126 canali supportati da Android nella playlist `resources/channels.m3u`, con porting delle integrazioni Sky/Discovery e della selezione Rai TGR da `StreamResolver.kt` e `RaiResolver.kt`. Le dirette conservano gli header previsti dalla playlist. La lista si aggiorna dalla fonte pubblica `tvchannels.worldtvlive.eu/tv/_dtt_kodi/list.m3u8`; `resources/catalog-legacy.json` mantiene gli identificativi dei precedenti 46 canali. Nuove cache, controlli delle destinazioni di rete e filtri desktop sono modifiche del progetto Lume TV.

Le integrazioni adattate in Rust conservano l’origine e le attribuzioni del codice da cui derivano. Chi le ridistribuisce deve mantenerne gli avvisi e rispettare la GPL-3.0-or-later indicata in `Cargo.toml` e nel file `LICENSE`.

## Shaka Player

- Componente: **Shaka Player 4.16.56**.
- Autore: Copyright 2016 Google LLC e contributori, secondo gli avvisi nei sorgenti.
- Licenza: **Apache-2.0**; testo conservato in `vendor/shaka/LICENSE`.
- Sorgente della versione: [shaka-project/shaka-player, tag v4.16.56](https://github.com/shaka-project/shaka-player/tree/v4.16.56).
- Artefatto incluso: `vendor/shaka/dist/shaka-player.compiled.js`.
- SHA-256 dell'artefatto incluso: `61e5fb69716a6971992616cffbfdbc75d3afadcd881295243de7b6f12ddb0f26`.

Il file compilato conserva i propri avvisi di copyright e licenza, compresi quelli dei componenti incorporati. Viene servito localmente dal prototipo.

## React, React DOM e Scheduler

L’interfaccia desktop incorpora React e React DOM **19.3.1** e Scheduler **0.28.0**,
distribuiti sotto licenza MIT. I testi originali sono conservati in
[web/app/licenses](web/app/licenses) e copiati nei pacchetti nativi. I sorgenti
upstream sono nel [repository React](https://github.com/facebook/react).
Le versioni risolte delle dipendenze e degli strumenti frontend sono registrate
in `web/app/package-lock.json`; il codice dell’applicazione è in `web/app/src`.

## Chromium Embedded Framework e Chromium

CEF è un progetto distinto, con codice sotto licenza **BSD a tre clausole** e componenti Chromium con i rispettivi avvisi. Il testo CEF presente nell’SDK attribuisce il copyright a Marshall A. Greenblatt e Google Inc.; i crediti Chromium e delle dipendenze sono nell’SDK `CREDITS.html`.

- Progetto: [chromiumembedded/cef](https://github.com/chromiumembedded/cef).
- Prima configurazione provata: CEF stock `156.0.15+gcf671c0+chromium-156.0.8078.80` macOS arm64.
- Configurazione verificata nella prova locale dell’8 novembre 2026: archivio **OBS CEF 150.0.19** macOS arm64, distribuito da OBS e derivato da CEF/Chromium. Il [riepilogo delle prove incorporate](docs/STATUS.md) resta distinto dalla baseline stock 156.
- URL dell’archivio OBS: [cef_binary_7871_macos_arm64.tar.xz](https://cdn-fastly.obsproject.com/downloads/cef_binary_7871_macos_arm64.tar.xz).
- SHA-256 originale verificato: `0343233fc462384c2f1f58a4222028bc6171f05948a822241f89df2faa7cee8d`.
- Versione SDK effettiva: `150.0.19+gcf60f42+chromium-150.0.7871.252`.
- Riferimento di provenienza OBS: [OBS Studio 33.0.0, CMakePresets.json](https://github.com/obsproject/obs-studio/blob/33.0.0/CMakePresets.json).

L’SDK viene acquisito nella cache locale e non è vendorizzato nei sorgenti del progetto. I suoi `LICENSE.txt`, `CREDITS.html` e `include/cef_version.h` sono conservati durante la preparazione. `scripts/bundle_cef.py` copia gli avvisi CEF disponibili sotto `Contents/Resources/licenses/`, insieme alla licenza del progetto e a questo documento; le versioni reali sono registrate in `Contents/Resources/runtime.json`. Il framework conserva inoltre le proprie risorse e attribuzioni.

L’etichetta `codec_build: custom` identifica il framework alternativo scelto. Disponibilità dei codec e risultati di riproduzione si verificano nella configurazione eseguita; le licenze per codec e contenuti restano separate da questa etichetta.

## Binding Rust cef-rs e patch download-cef

I binding derivano da [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs), con licenza **Apache-2.0 OR MIT**, secondo i manifest dei pacchetti:

| Pacchetto | Versione usata |
| --- | --- |
| `cef` | `150.2.1+150.0.14` |
| `cef-dll-sys` | `150.2.1+150.0.14` |
| `download-cef` | `2.3.2`, con eccezione locale descritta sotto |

La copia modificata è in `vendor/download-cef`; i testi integrali upstream sono conservati in [LICENSE-APACHE](vendor/download-cef/LICENSE-APACHE) e [LICENSE-MIT](vendor/download-cef/LICENSE-MIT). La copia sorgente di riferimento di `download-cef` 2.3.2 proviene dal commit `0c577ce44dbd36952ac3721b577c6e423ceff44f` del repository cef-rs. Copyright e altri avvisi originali restano nei file.

**Avviso di modifica — 8 novembre 2026:** aggiunta un’eccezione opt-in, attivata soltanto da `LUME_CEF_ALLOW_OBS_150_0_19=1`, per i binding che attendono CEF 150.0.14 e l’SDK OBS 150.0.19 macOS arm64. Il controllo verifica versione reale e hash API macOS Stable `15000` ed Experimental `999999`; non accetta genericamente SDK di versioni successive. [Descrizione e riferimenti della patch](vendor/download-cef/LUME_PATCH.md).

`scripts/prepare_obs_cef.py` verifica il checksum dell’archivio OBS e confronta con esso gli header, l’entrypoint CMake e il binario CEF. La normalizzazione aggiunge metadati con la versione reale e un alias relativo al framework; non modifica framework, implementazioni codec, CDM o firme. I binding e il wrapper restano allineati sull’API Experimental `999999` prevista dalla configurazione.

**Packaging multipiattaforma — 9 novembre 2026:** `scripts/prepare_cef_sdk.py` e
`LUME_CEF_USE_PINNED_OBS=1` estendono la selezione ai quattro SDK registrati in
[lume-sdk-pins.json](vendor/download-cef/lume-sdk-pins.json), con checksum OBS
33.0.0, versione reale, architettura e hash Stable/Experimental verificati.
Mac arm64/Intel e Linux x64 usano CEF 150.0.19; Windows x64 riporta realmente
`150.0.20-7871-devtools_patch.3553+gc4b3522+chromium-150.0.7871.252`.
Non vengono riscritti gli header per dichiarare una versione diversa.

Il punto di ingresso `RunWinMain` della DLL in `crates/lume-cef-windows` segue
il contratto del bootstrap CEF e l’esempio cef-rs `cefsimple/src/win.rs` della
stessa revisione upstream sopra indicata, con licenza Apache-2.0 OR MIT. La
gestione degli errori e il collegamento al backend Lume sono modifiche del
progetto. Il bootstrap binario proviene dall’SDK Windows verificato e resta
accoppiato alla relativa DLL CEF; il contesto sandbox viene passato alla app.

## Dipendenze Rust

Le dipendenze dirette sono dichiarate in `Cargo.toml`; versioni e checksum risolti, comprese le dipendenze transitive, sono registrati in `Cargo.lock`. I pacchetti mantengono le proprie licenze e i propri avvisi nei sorgenti distribuiti tramite Cargo.

Le pipeline raccolgono gli avvisi disponibili e l’inventario delle licenze delle
dipendenze Cargo risolte. Preparano anche un archivio dei sorgenti corrispondenti
con `cargo vendor --locked`. CEF conserva i propri crediti e i
pin registrano l’origine del framework. Le firme dei pacchetti sono ancora di
sviluppo; prima di una distribuzione di produzione resta da rivedere la
configurazione scelta e i relativi requisiti. Vedi [RELEASE.md](docs/RELEASE.md).

## Browser installati e DRM

Chrome, Edge e il CDM Widevine non sono inclusi nel repository o nel bundle CEF. Il backend diagnostico esplicito `--runtime browser` avvia il browser installato e usa i componenti che esso rende disponibili. Il backend predefinito CEF usa il proprio framework e i propri helper.

Lo script `scripts/seed_widevine.py` può copiare, su indicazione esplicita, un CDM già presente in una sorgente esterna nel profilo marcato del prototipo. Conserva la licenza del componente e copia soltanto i file della piattaforma selezionata; non modifica l’installazione sorgente, non scarica un CDM e non lo incorpora nel pacchetto dell’app. `--widevine-cdm` registra una provenienza, senza attestare caricamento o autorizzazione alla distribuzione. Condizioni e licenze dei componenti restano quelle dei rispettivi fornitori.

La prova macOS incorporata dell’8 novembre 2026 ha usato un componente esterno **Widevine 4.10.3112.0** nel solo profilo dedicato. Il successo della riproduzione non modifica la provenienza del componente; `distribution_verified` resta falso nel report.

La firma ad hoc del bundle è una firma di sviluppo locale; non rappresenta VMP Widevine o una notarizzazione del prodotto. La presenza di un CDM, una risposta EME o una prova effettuata con Chrome non costituiscono una verifica della distribuibilità di un pacchetto CEF/CDM. Il relay HTTP inoltra challenge e licenze opache, senza implementare la decifratura DRM o estrarre chiavi.

## Emittenti e servizi

Marchi, loghi, palinsesti e trasmissioni appartengono ai rispettivi titolari. Le prove contattano i servizi sorgente dal dispositivo dell'utente. Il progetto non ospita le trasmissioni e la licenza del software non trasferisce diritti sui contenuti delle emittenti.
