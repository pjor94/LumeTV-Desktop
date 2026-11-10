# Runtime CEF incorporato

Per lo stato delle piattaforme consulta il [riepilogo delle verifiche](STATUS.md).
I file citati in `reports/` restano locali ed esclusi da Git. Le nuove prove
seguono il [protocollo di validazione](VALIDATION.md).

## App desktop e primo avvio

La build `Lume TV.app` apre il catalogo con il backend Rust nello stesso processo. Per lo sviluppo puoi avviare il controller CLI con `lume-player app`. L’interfaccia comprende catalogo, ricerca, guida XMLTV/XZ, preferiti, recenti, promemoria, temi e player con mini-player. Le risorse sono compilate nel binario: CEF riceve gli asset statici locali e accede alle API sul loopback autenticato. Il contesto CEF rimane in memoria, con cookie disabilitati e richieste del frontend senza credenziali browser. Il backend Rust salva preferenze, guida e loghi. Il Component Updater usa il proprio `root_cache_path` sotto `browser`.

Il catalogo comprende i **126 canali Android**, con Sky, Discovery, Rai TGR e dirette, e si aggiorna automaticamente dalla stessa fonte. L’[audit del catalogo completo](CATALOG.md) riporta disponibilità dei manifest, prove video e limiti osservati.

L’8 novembre 2026, con la nuova UI su macOS arm64, Rai 1, Canale 5 e La7 hanno superato 30 secondi misurati ciascuno. Canale 5 e La7 hanno mostrato eventi cifrati, chiavi `usable` e due scambi licenza riusciti. È stata rilevata decodifica audio; l’udibilità non è stata confermata manualmente in questa nuova prova. Matrice UI (`reports/desktop-native-ui-matrix.json`).

Una prova separata è partita da `profiles/desktop-first-run-bundled`, senza sottocartella browser né CDM. CEF ha scaricato Widevine **4.10.3112.0** e Canale 5 ha riprodotto oltre due minuti, senza riavvio del browser o script di copia. La valutazione Rust registra `drm_verified: true`. Erano già presenti guida e loghi da una cache del feed reale; il profilo era privo di componenti DRM precaricati. Stato iniziale (`reports/desktop-first-run-bundled-before.json`), risultato (`reports/desktop-first-run-bundled-summary.json`), metriche (`reports/desktop-first-run-bundled-canale5.json`). Le precedenti prove inconcludenti di caricamento della pagina sono documentate più avanti e conservano i propri esiti.

Durante la chiusura, CEF 150 può lasciare in sospeso una richiesta OSCrypt al Portachiavi. Una volta chiuse tutte le finestre e il server Rust, l’app termina entro cinque secondi, senza usare un Portachiavi simulato o modificare protezioni e credenziali di sistema. Per il rilascio occorre risolvere questo limite e completare le verifiche su firma, notarizzazione, licenze, codec e VMP descritte sotto. La prova riguarda macOS; Windows e Linux non sono stati eseguiti in questa sessione.

## Architettura e stato

Il CLI Rust avvia il companion `lume-cef`, scritto anch’esso in Rust, che gestisce la finestra CEF e il relativo message loop. Il bundle contiene il framework CEF e cinque app helper costruite dal binario `lume-cef-helper`. Resolver, relay localhost autenticato, Shaka e criteri di misurazione sono condivisi con la diagnostica precedente. Il player funziona senza aprire Chrome o Edge.

La selezione predefinita è `--runtime cef`; la modalità esterna richiede `--runtime browser`. Se CEF non viene rilevato, il CLI salva un esito di runtime mancante. Il supporto headless non è implementato per questo companion.

## Risultati osservati con CEF OBS 150

La matrice dell’**8 novembre 2026**, avviata alle **00:15 Europe/Rome** su macOS arm64, ha verificato tutti e tre i canali con **CEF OBS 150.0.19 / Chromium 150.0.7871.252** nella finestra propria Rust/CEF, in modalità relay e con `headless: false`. Chrome non è stato avviato: report CEF OBS completo (`reports/macos-cef-obs150-matrix.json`).

| Canale | Avanzamento misurato | Incremento frame | Audio decodificato | DRM e audio manuale |
| --- | ---: | ---: | ---: | --- |
| Rai 1 | 32,825 s | 822 | 1.344.921 byte | Sorgente non protetta; conferma audio registrata |
| Canale 5 | 31,871 s | 1.595 | 399.794 byte | Widevine verificato; conferma audio registrata |
| La7 | 31,852 s | 797 | 396.739 byte | Widevine verificato; conferma audio registrata |

La soglia richiesta era di 30 secondi; il timer del browser aveva due secondi di margine, mentre Rust ha mantenuto la soglia richiesta e limitato i delta al tempo di ricezione dei campioni. Tutti i report registrano `playback_verified`, `embedded_runtime_initialized: true` ed `embedded_runtime_verified: true`, senza errori player, timeout o richieste di rete fallite nella finestra osservata.

Per Canale 5 e La7 sono presenti eventi `encrypted`, chiavi `usable` e **due scambi licenza HTTP riusciti per canale**, oltre a video effettivamente decodificato. AVC/AAC MSE e le prove EME Widevine sia AVC/AAC sia VP9/Opus risultano disponibili. La conferma audio è l’azione esplicita registrata nella UI, distinta dai contatori automatici; non è una misura acustica.

Il test ha utilizzato **Widevine 4.10.3112.0 fornito esternamente e predisposto nel solo profilo dell’app**. Il componente non è incluso nel bundle o nel repository. La firma dell’app è ad hoc per lo sviluppo locale e `distribution_verified` rimane falso. Il risultato dimostra questa esecuzione macOS: prove prolungate, distribuzione, Windows e Linux CEF richiedono ancora verifiche dedicate.

### Baseline CEF stock 156

La prima matrice **macOS arm64 con CEF stock 156** resta documentata separatamente. Ha inizializzato la finestra incorporata su Rai 1, Canale 5 e La7, con AVC/AAC non supportati da MSE e prova EME Widevine VP9/Opus disponibile. I tre verdetti sono `codec_unavailable`; il tentativo effettivo di caricamento Rai 1 ha prodotto errore Shaka **4032**. Questo risultato distingue il limite dei codec dalla disponibilità del CDM: report CEF stock 156 (`reports/macos-cef-stock156-matrix.json`).

## Configurazione riproducibile macOS arm64

Eseguire dalla root del progetto, con toolchain Rust **nativo arm64** `stable-aarch64-apple-darwin` già installato, Python 3.12+, CMake, Ninja e Xcode Command Line Tools. La dipendenza `cef` è opzionale e attivata soltanto dalla feature `embedded-cef`.

Il percorso automatico è lo [script macOS](../scripts/build_embedded_macos.sh), eseguibile senza argomenti:

```sh
./scripts/build_embedded_macos.sh
```

Verifica l’archivio, normalizza l’SDK, compila e crea `target/cef-obs-arm64/debug/lume-cef.app`, con il CLI nella stessa directory `debug`. Non modifica il toolchain globale, non avvia le prove video e non copia il CDM. Le sezioni seguenti espongono i passaggi manuali equivalenti usando una directory di output separata, `target/cef-arm64`.

La procedura è riuscita su macOS arm64: log della build (`reports/build-embedded-macos-script.log`). Il successivo `doctor` ha rilevato CEF `150.0.19+gcf60f42+chromium-150.0.7871.252` nel bundle: report doctor (`reports/macos-cef-obs150-doctor.json`). La prova video è documentata nella matrice sopra.

| Componente | Versione/configurazione |
| --- | --- |
| Binding Rust `cef` e `cef-dll-sys` | `150.2.1+150.0.14`, bloccati in Cargo.lock |
| SDK OBS effettivo | `150.0.19+gcf60f42+chromium-150.0.7871.252` |
| SDK atteso nominalmente dai binding | `150.0.14+g7c1aa68+chromium-150.0.7871.129` |
| Compatibilità controllata | Hash macOS Stable API `15000` e Experimental API `999999` identici per questi due SDK |
| API del client/wrapper | Experimental `999999`, coerente con i binding generati |
| Eccezione locale | `download-cef` 2.3.2, opt-in esatto `LUME_CEF_ALLOW_OBS_150_0_19=1` |

L’eccezione è limitata a queste versioni e a macOS arm64; non disattiva genericamente il controllo della versione. Il normalizzatore conserva la versione reale 150.0.19 nei metadati. [Dettagli della patch](../vendor/download-cef/LUME_PATCH.md).

### 1. Acquisire e verificare l’SDK

Archivio ufficiale OBS usato nella prova:

- [cef_binary_7871_macos_arm64.tar.xz](https://cdn-fastly.obsproject.com/downloads/cef_binary_7871_macos_arm64.tar.xz).
- SHA-256: `0343233fc462384c2f1f58a4222028bc6171f05948a822241f89df2faa7cee8d`.

Usare la directory di cache dedicata del progetto:

```sh
mkdir -p .cache/cef-obs/sdk
curl --fail --location --output .cache/cef-obs/cef_binary_7871_macos_arm64.tar.xz https://cdn-fastly.obsproject.com/downloads/cef_binary_7871_macos_arm64.tar.xz
printf '%s  %s\n' 0343233fc462384c2f1f58a4222028bc6171f05948a822241f89df2faa7cee8d .cache/cef-obs/cef_binary_7871_macos_arm64.tar.xz | shasum -a 256 --check
```

Dopo l’esito positivo del checksum:

```sh
tar -xJf .cache/cef-obs/cef_binary_7871_macos_arm64.tar.xz -C .cache/cef-obs/sdk
python3 scripts/prepare_obs_cef.py --archive "$PWD/.cache/cef-obs/cef_binary_7871_macos_arm64.tar.xz" --sdk "$PWD/.cache/cef-obs/sdk"
```

Lo script verifica nuovamente l’archivio, le versioni e gli hash API, poi confronta con l’archivio originale header, CMake e binario del framework. Aggiunge un alias relativo al framework e `archive.json`; non modifica framework o header e non duplica il framework. `--check` esegue la stessa verifica senza scrivere i metadati.

### 2. Compilare CLI e companion Rust

```sh
CEF_PATH="$PWD/.cache/cef-obs/sdk" LUME_CEF_ALLOW_OBS_150_0_19=1 cargo +stable-aarch64-apple-darwin build --locked --features embedded-cef --bins --target-dir target/cef-arm64
```

`CEF_PATH` è esplicito: identifica l’SDK normalizzato. La patch locale di `download-cef` deve essere quella selezionata dalla configurazione Cargo del progetto. Non sostituire API `999999` con `15000`: il solo hash Stable uguale non allineerebbe questi binding Experimental con il wrapper.

La feature produce `lume-player`, `lume-cef` e `lume-cef-helper`. Il CLI core senza feature resta compilabile senza CEF.

### 3. Creare il bundle di sviluppo

```sh
python3 scripts/bundle_cef.py --sdk "$PWD/.cache/cef-obs/sdk" --target-dir target/cef-arm64/debug --output target/cef-arm64/debug/lume-cef.app --codec-build custom
```

Il bundler assembla framework, risorse e cinque helper, conserva licenze e crediti CEF e scrive `Contents/Resources/runtime.json`. La firma ad hoc serve allo sviluppo locale; notarizzazione e firma VMP Widevine richiedono procedure separate. `--skip-sign` serve per fixture e ispezione ed è escluso dalla procedura di avvio descritta qui.

`--codec-build custom` registra la provenienza del framework alternativo. I codec disponibili dipendono dal framework e vengono verificati dal player; il flag non li abilita. Il bundler sostituisce soltanto bundle riconosciuti come propri output ed esclude i file CDM dalla copia del framework.

### 4. Aprire il player incorporato

```sh
LUME_CEF_EXE="$PWD/target/cef-arm64/debug/lume-cef.app/Contents/MacOS/lume-cef"
./target/cef-arm64/debug/lume-player doctor --cef-executable "$LUME_CEF_EXE"
./target/cef-arm64/debug/lume-player play --cef-executable "$LUME_CEF_EXE" --channel rai1 --seconds 30
./target/cef-arm64/debug/lume-player matrix --cef-executable "$LUME_CEF_EXE" --seconds 30 --report reports/macos-cef-obs-matrix.json
```

Il report identifica `backend: embedded_cef`, versioni CEF/Chromium e provenienza dei codec. `embedded_runtime_initialized` richiede l’arrivo delle capacità dalla pagina; `embedded_runtime_verified` richiede la riproduzione misurata. `distribution_verified` resta falso. Senza `--report`, il CLI crea un percorso nuovo distinto per backend ed esecuzione; i report precedenti del browser restano separati.

## Widevine automatico al primo avvio

Il companion abilita il Component Updater di CEF con `--component-updater=fast-update`. CEF può scaricare e installare Widevine in `WidevineCdm/<versione>`, nel profilo dedicato dell’app, senza percorsi Chrome o script di copia. Il framework resta nel bundle; il componente scaricato si trova nel profilo scrivibile dell’utente. Per il primo download serve Internet.

L’8 novembre 2026 il componente **4.10.3112.0 per macOS arm64** è comparso in due profili creati vuoti per questa prova, con manifest e libreria di 20.416.896 byte. Nessuna sorgente CDM è stata passata e nessun seeding è stato eseguito. Il riepilogo (`reports/macos-cef-auto-download-summary.json`) conserva dimensioni e SHA-256; i record iniziali sono Rai 1 (`reports/cef-auto-download-before.json`) e Canale 5 (`reports/cef-auto-download-canale5-before.json`).

Queste due esecuzioni hanno verificato l’installazione automatica, ma sono terminate in timeout prima della riproduzione, senza eventi della pagina. Il runtime non ha richiesto nemmeno una pagina localhost sintetica, priva di player e integrazioni TV: diagnostica di avvio (`reports/macos-cef-startup-diagnostic.json`). La causa del blocco grafico/renderer resta da isolare. La matrice precedente riguarda invece un profilo con CDM predisposto esternamente e conserva quel risultato.

Il player mostra l’attesa di preparazione dei canali protetti e ripete la prova EME prima di iniziare. App e CLI concedono **300 secondi** di preparazione, separati dalla finestra di riproduzione; `--drm-setup-seconds` accetta da 0 a 300 secondi. Il limite iniziale di 120 secondi è stato aumentato durante la verifica del catalogo completo: in un profilo vuoto il download automatico è terminato dopo quel limite. Le capacità finali sostituiscono quelle iniziali nei metadati e nel verdetto. La diagnostica distingue `drm_setup` con stati `waiting`, `ready`, `timeout`, `unavailable` e il verdetto `widevine_setup_timeout`. Non tenta una copia da installazioni personali.

Esempio per ripetere il primo avvio, scegliendo una directory che non sia già stata usata:

```sh
./target/cef-obs-arm64/debug/lume-player play --channel canale5 --profile profiles/cef-first-run-new --seconds 30 --drm-setup-seconds 120 --report reports/cef-first-run-new.json
```

L’[upstream CEF](https://github.com/chromiumembedded/cef/issues/3149) documenta il caricamento del componente dopo il download su macOS/Windows e il riavvio necessario su Linux. Questa prova riguarda macOS arm64; Windows/Linux e la gestione del riavvio Linux non sono ancora verificati. Le fonti sulla licenza sono approfondite di seguito: il download ufficiale e la redistribuzione del CDM richiedono valutazioni distinte. `distribution_verified` rimane falso.

### Licenza: ricerca dell’8 novembre 2026

Le fonti pubbliche descrivono il download ufficiale al primo avvio come un percorso ammesso e lo distinguono dalla redistribuzione del CDM. La nota generale Google, da sola, non chiarisce se un’app CEF che scarica il componente debba ottenere un proprio accordo. Per il bundle personalizzato di Lume TV resta da ottenere una conferma aggiornata dei requisiti contrattuali.

| Fonte primaria | Evidenza | Portata e limite |
| --- | --- | --- |
| [CEF #1631, Marshall Greenblatt, 13 maggio 2015](https://github.com/chromiumembedded/cef/issues/1631) | L’autore distingue esplicitamente download automatico del binario da Google, dichiarato consentito, e inclusione del CDM in applicazioni di terzi, per cui richiede una licenza. | È una dichiarazione specifica del manutentore CEF, ma storica; le istruzioni Pepper e i vecchi switch del ticket non descrivono il runtime attuale. |
| [CEF #2009, 2016](https://github.com/chromiumembedded/cef/issues/2009) e [CEF #3149, 2021](https://github.com/chromiumembedded/cef/issues/3149) | Il vecchio percorso di download diretto da CDN richiedeva il permesso Google. Il Component Updater è poi stato ripristinato dopo confronti con Google, documentando il download nel profilo. | Il ticket 2021 conferma il percorso tecnico corrente; non pubblica un nuovo contratto o un’esenzione contrattuale generale. Non confondere link CDN manuali del 2016 con il Component Updater attuale. |
| [Castlabs FAQ, pagina aggiornata il 23 ottobre 2026](https://github.com/castlabs/electron-releases/wiki/FAQ#can-i-bundle-the-widevine-cdm-to-avoid-downloading-it-on-first-start) | Castlabs raccomanda il download dai server Google. Spiega che le licenze MPEG-LA dei decoder contenuti nel CDM sono coperte da Google quando è Google a distribuirli all’utente. | Il passaggio riguarda i decoder nel CDM. Non copre automaticamente i codec inclusi separatamente nel nostro framework CEF/FFmpeg e non è un’autorizzazione Google individuale a Lume TV. |
| [Castlabs ECS / VMP](https://github.com/castlabs/electron-releases/wiki/VMP) e [EVS](https://github.com/castlabs/electron-releases/wiki/EVS) | Per le release ufficiali ECS, Castlabs documenta un servizio gratuito, con registrazione, per la firma VMP di produzione senza dover ottenere un proprio certificato attraverso un accordo diretto con Google. Per adattamenti Chromium/Electron personalizzati documenta invece audit e accesso commerciale 3PL. | La facilitazione riguarda quel percorso di firma e quei binari. Non possiamo applicare automaticamente il servizio gratuito al nostro companion Rust/CEF. VMP e distribuzione del CDM sono aspetti separati. |
| [Google Widevine overview](https://developers.google.com/widevine/drm/overview) e [licenza del componente](https://raw.githubusercontent.com/chromium/chromium/main/third_party/widevine/LICENSE) | Google mantiene una richiesta generale di accordo per l’uso dei prodotti/servizi Widevine; il file LICENSE comprende anche l’uso. L’overview dichiara assenza di tariffe per l’uso. | Questi testi non precisano il trattamento di un’app CEF di terzi che riceve il CDM soltanto tramite Component Updater. La gratuità dichiarata da Google non determina i costi di eventuali servizi 3PL. |

Castlabs compare tra i [partner Widevine 3PL sul sito Google](https://www.widevine.com/solutions/widevine-3pl). Le sue indicazioni riguardano l’integrazione Widevine; la copertura del caso Lume TV deve essere confermata da Google o da un partner.

Lume TV prevede di distribuire Rust/CEF e affidare il CDM al Component Updater ufficiale, con gli aggiornamenti nel profilo dell’utente. Le fonti esaminate sostengono questa scelta: non indicano un divieto del download automatico né lo assoggettano necessariamente allo stesso accordo della redistribuzione del binario. Occorre verificare separatamente gli eventuali requisiti VMP dei servizi di destinazione.

Per chiedere un chiarimento scritto si può usare il testo seguente. La richiesta non è stata inviata:

> We are developing a Rust desktop player with a custom CEF runtime on macOS, Windows and Linux. Our installer contains no Widevine CDM. The application obtains and updates it directly from Google through CEF's built-in Component Updater after installation. Does this integration require our own Widevine Master License Agreement or any other agreement? Which terms or notices cover the end user's download and use? If production VMP signing is required by a content provider, which agreement or 3PL route applies to this custom CEF build?

La ricerca si è limitata alle fonti pubbliche, senza apertura di account, accettazione di accordi o contatti con soggetti esterni. Le prove di riproduzione sono state eseguite separatamente.

## CDM esterno nel profilo dedicato

Questo percorso opzionale serve a ripetere prove controllate o la matrice storica. L’installazione automatica funziona senza questo passaggio.

`--widevine-cdm PATH` registra e inoltra il percorso di una sorgente esterna. Il flag non copia il componente né usa il vecchio switch `--widevine-cdm-path`; il caricamento da parte di CEF va verificato nei report. Per predisporre i file prima dell’avvio si usa lo script separato, che segue la struttura del Component Updater nel profilo dell’app.

Esempio con un componente già presente nell’installazione locale di Chrome su macOS arm64:

```sh
mkdir -p profiles/cef-obs
touch profiles/cef-obs/.lume-player-profile
LUME_CDM_SOURCE="/Applications/Google Chrome.app/Contents/Frameworks/Google Chrome Framework.framework/Versions/Current/Libraries/WidevineCdm"
python3 scripts/seed_widevine.py --source "$LUME_CDM_SOURCE" --profile "$PWD/profiles/cef-obs" --check
python3 scripts/seed_widevine.py --source "$LUME_CDM_SOURCE" --profile "$PWD/profiles/cef-obs"
./target/cef-arm64/debug/lume-player matrix --cef-executable "$LUME_CEF_EXE" --widevine-cdm "$LUME_CDM_SOURCE" --profile "$PWD/profiles/cef-obs" --seconds 30 --report reports/macos-cef-obs-cdm-matrix.json
```

Il profilo deve essere una directory propria del prototipo, con marker regolare, e deve coincidere con `--lume-profile`/`root_cache_path` del companion. CEF usa una cache interna `Default`. Lo script controlla manifest, piattaforma, architettura e hash, copiando soltanto manifest, licenza e libreria/firma della piattaforma corrente sotto `WidevineCdm/<versione>`. Su Linux prepara anche l’hint di percorso richiesto dal componente; questa possibilità dello script non costituisce una prova del runtime CEF Linux.

L’installazione sorgente resta invariata. Il CDM non viene inserito nel bundle dell’app né nel repository. Il seeding non genera firme host, non sostituisce VMP e non garantisce che il servizio accetti una licenza. I report distinguono prova EME AVC/AAC, prova EME VP9/Opus, stato `usable`, richieste di licenza e decodifica effettiva.

## Windows e Linux

I resolver, il relay e la selezione runtime mantengono codice multipiattaforma; il companion è predisposto per eseguibili propri `lume-cef.exe`/`lume-cef`. La procedura di bundle documentata sopra è macOS arm64. Bootstrap, librerie e firma degli helper Windows, dipendenze native/sandbox Linux, distribuzione del CDM e riproduzione CEF sui due sistemi richiedono prove dedicate.

I risultati Linux in container e macOS con Chrome installato in [RESULTS.md](RESULTS.md) riguardano il backend precedente. Per CEF incorporato servono prove dedicate su ciascuna piattaforma.

Il tentativo Linux più recente ha completato **43 test core** (35 libreria, sei CLI e due HTTP; il test specifico macOS è escluso), prima che Docker fallisse con un errore di mount overlay. La ripetizione è stata bloccata dal filesystem Docker in sola lettura, prima dell’esecuzione del controllo API CEF. Primo tentativo (`reports/linux-cef-api-build-first-attempt.log`), ripetizione (`reports/linux-cef-api-build.log`). Il controllo API multipiattaforma è configurato in CI, ma gli esiti Windows/Linux CEF non sono stati acquisiti. [Stato Linux dettagliato](LINUX.md).

## Test degli strumenti

Verifiche osservate l’8 novembre 2026 su macOS arm64:

| Verifica | Esito | Evidenza |
| --- | --- | --- |
| Rust, feature `embedded-cef` | 45 test superati: 36 libreria + un binario CEF + sei CLI + due HTTP | Log (`reports/macos-embedded-tests.log`) |
| Clippy, feature CEF e tutti i target | Superato con `-D warnings` | Log (`reports/macos-embedded-clippy.log`) |
| Formattazione Rust | Controllo superato | Verifica locale |
| Harness Node | 11 test superati | Log (`reports/browser-harness-tests.log`) |
| Strumenti Python | 17 test superati | Log (`reports/python-tests.log`) |
| Patch `download-cef` | Tre test Rust separati superati | [Sorgenti e descrizione](../vendor/download-cef/LUME_PATCH.md) |

La suite CEF è stata eseguita con l’SDK esplicito e la configurazione di compatibilità documentata. I comandi seguenti ripetono invece le suite core e degli strumenti senza richiedere un SDK CEF:

```sh
python3 -m unittest discover -s tests -p 'test_*.py'
node --test web/player.test.cjs
cargo test --locked --no-default-features --lib --bin lume-player --test harness
```

Le fixture Python verificano il bundle, la normalizzazione SDK e il seeding senza usare una trasmissione reale. I test Node distinguono codec e configurazioni EME; quelli Rust valutano i campioni e i limiti del relay. La riproduzione CEF va documentata separatamente nel report della sua esecuzione.
