# Pacchetti desktop e release GitHub

Le pipeline compilano l’app completa con CEF incorporato su quattro runner
nativi. I pacchetti servono per le prove desktop e non includono Widevine:
l’app può scaricarlo al primo utilizzo di una diretta protetta.

| Build | Runner | Download |
| --- | --- | --- |
| macOS Apple Silicon | `macos-15`, arm64 | `.dmg` e `.zip` con `Lume TV.app` |
| macOS Intel | `macos-15-intel`, x64 | `.dmg` e `.zip` con `Lume TV.app` |
| Windows x64 | `windows-2025` | `.zip` portabile con `LumeTV.exe`, DLL e risorse CEF |
| Linux x64 | `ubuntu-24.04` | `.deb` per Ubuntu 24.04 e `.tar.gz` portabile |

Il runner determina l’architettura della build. La pipeline controlla anche
il target Rust e l’architettura dei binari SDK. I runner sono descritti
nella [documentazione GitHub](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).

## Scaricare una build di prova

Da [GitHub Actions](https://github.com/pjor94/LumeTV-Desktop/actions):

1. Apri **Actions → Rust cross-platform checks**.
2. Scegli l’esecuzione del commit desiderato e attendi i job dei pacchetti.
3. In **Artifacts** scarica `desktop-macos-arm64`, `desktop-macos-x64`,
   `desktop-windows-x64` o `desktop-linux-x64`.
4. Estrai l’archivio dell’artifact per trovare il pacchetto, i metadati della
   build e il file `.sha256`.

`desktop-source` contiene i sorgenti dello stesso commit e delle dipendenze
Cargo risolte. Gli artifact vengono conservati per **14 giorni**. Il workflow
può essere avviato anche da **Run workflow**.

## Cosa esegue la CI

[ci.yml](../.github/workflows/ci.yml) verifica il contenuto pubblico, i link,
i test Python, Rust e Node, la formattazione, Clippy e le API CEF. Include il
controllo del punto di ingresso della DLL sandbox Windows.

Solo dopo il successo di questi controlli richiama
[packages.yml](../.github/workflows/packages.yml). Ogni job nativo:

1. Acquisisce l’SDK OBS previsto dai [pin condivisi](../vendor/download-cef/lume-sdk-pins.json).
2. Verifica SHA-256, versione effettiva, architettura e hash API Stable/Experimental.
3. Compila in release il player e i componenti richiesti dalla piattaforma,
   controllando anche Clippy sul runtime nativo con `-D warnings`.
4. Raccoglie avvisi CEF/Shaka/cef-rs e un inventario delle licenze Cargo.
5. Prepara il bundle o la cartella portabile con tutte le risorse necessarie.
6. Esegue `--lume-version` dal pacchetto per verificare caricamento, piattaforma
   e presenza del runtime incorporato con sandbox abilitata.
7. Produce archivi, metadati e checksum SHA-256 scaricabili.

Questo controllo si ferma al caricamento del programma. Renderer, immagine,
audio, richieste di licenza DRM e comportamento dei provider richiedono la
[validazione live](VALIDATION.md).

## Avviare i pacchetti

**macOS 13 o successivo:** scegli l’architettura del tuo Mac, apri il DMG e copia
`Lume TV.app` in Applicazioni, oppure estrai lo ZIP. La firma è ad hoc per lo
sviluppo locale. Firma Developer ID, notarizzazione e approvazione Gatekeeper
per la distribuzione di produzione restano da completare.

**Windows x64:** estrai l’intera cartella ZIP e apri `LumeTV.exe`. Tieni DLL,
cartella `locales` e risorse accanto all’eseguibile.
Il pacchetto non ha ancora una firma Authenticode.

**Ubuntu 24.04 x64:** installa il DEB con il gestore pacchetti, per esempio:

```sh
sudo apt install ./LumeTV-VERSIONE-linux-x64.deb
```

Sostituisci `VERSIONE` con la versione scaricata. Il DEB dichiara le dipendenze
di sistema, installa l’icona e il collegamento al menu e registra l’helper
`chrome-sandbox` con proprietario root e modalità `4755`, come previsto da CEF.
La pipeline controlla questi attributi nell’archivio finale.

Il tar.gz offre `AppRun` per l’avvio dalla cartella estratta. Richiede librerie
compatibili con Ubuntu 24.04, glibc 2.39+ e user namespace non privilegiati
disponibili. Non disabilita la sandbox e non cambia impostazioni di sistema.

Widevine viene preparato dal Component Updater nel profilo, quando disponibile
per il runtime e l’emittente; non è incluso negli allegati. Il primo utilizzo
di una diretta protetta richiede una connessione.

## Pubblicare una versione

[release.yml](../.github/workflows/release.yml) riprende il flusso della versione
Android: ogni pre-release sperimentale contiene pacchetti, sorgenti e checksum.

1. Aggiorna alla stessa versione `Cargo.toml` e
   `crates/lume-cef-windows/Cargo.toml`, il lockfile e il changelog.
2. Crea e invia il tag `vVERSIONE` sul commit da distribuire, per esempio
   `v0.1.0` se entrambi i manifest dichiarano `0.1.0`.
3. Attendi i test, le quattro build native e l’archivio dei sorgenti.
4. La pipeline verifica che ogni allegato abbia versione e commit del tag,
   hash SDK previsti, sandbox attiva, esito del caricamento e checksum corretti.
5. Il job finale pubblica insieme tutti i pacchetti e `SHA256SUMS.txt` nella
   pre-release. Il token con `contents: write` è disponibile solo in questo job.

È disponibile anche l’avvio manuale indicando un **tag già esistente**.
Un tag non coerente con Cargo o un solo target fallito impediscono la
pubblicazione della release completa. Una release esistente non viene
sovrascritta automaticamente.

I metadati `.build.json` contengono versione, commit, versione CEF reale,
hash SDK, dimensioni e hash degli archivi. Servono a identificare la build;
feed degli aggiornamenti in-app e firme di distribuzione restano da definire.

## Sorgenti e dipendenze

L’archivio `LumeTV-VERSIONE-source.tar.gz` contiene i file indicizzati del
checkout verificato, i sorgenti Cargo raccolti con `cargo vendor --locked` e
una configurazione relativa in `.cargo/config.toml`. Il core può essere
compilato senza riscaricare le dipendenze:

```sh
cargo build --locked --offline
```

Per la build CEF serve anche l’SDK nativo previsto dai pin: il download
ufficiale e il suo checksum sono registrati nel progetto. Il framework CEF e i
suoi crediti sono nei pacchetti binari, non nell’archivio dei sorgenti Cargo.
Il CDM e i profili utente non sono inclusi in nessuno degli allegati.

I pacchetti includono copie degli avvisi disponibili per le dipendenze risolte
nel lockfile. La distinzione fra licenza del progetto e componenti esterni è
descritta in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Riprodurre una build locale

Con i requisiti descritti in [CONTRIBUTING.md](../CONTRIBUTING.md), sul sistema
nativo corrispondente:

```sh
python3 scripts/build_desktop.py --platform macos-arm64 --toolchain stable-aarch64-apple-darwin
```

Gli altri valori sono `macos-x64`, `windows-x64` e `linux-x64`. Su Windows serve
l’ambiente MSVC x64, configurato dal workflow con Visual Studio DevShell; su
Linux servono le dipendenze elencate in `scripts/build_desktop.py` e `dpkg-deb`.
Gli output sono in `dist/`, SDK e intermedi in `.cache/` e `target/`.
Lo script rifiuta un host di OS/architettura diversi dal target richiesto.

## Stato della verifica

Il 9 novembre 2026 sono stati scaricati i quattro SDK ufficiali e ne sono stati
verificati checksum, header API e architettura. Il packaging macOS arm64 ha
prodotto localmente DMG e ZIP. I layout Windows/Linux, i pin e i controlli
di raccolta degli allegati hanno test con fixture.

Le build complete Mac Intel, Windows e Linux attendono la prima esecuzione sui
runner nativi GitHub. Il tentativo di compilazione incrociata da macOS si è
fermato nelle dipendenze C: mancavano gli header Windows e il compilatore GNU
Linux. Quei target restano quindi da compilare. Docker locale non era
disponibile per la verifica Linux.

L’esito delle build remote è visibile in
[GitHub Actions](https://github.com/pjor94/LumeTV-Desktop/actions).
L’autoaggiornamento dell’app resta da implementare; lo stato delle prove video
e DRM è riportato in [STATUS.md](STATUS.md).
