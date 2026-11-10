# Verifiche Linux con Docker

Questa guida descrive i controlli nel container e i loro limiti. Lo stato del
player desktop è nel [riepilogo delle verifiche](STATUS.md). I file citati in
`reports/` restano locali ed esclusi da Git; per nuove prove segui il
[protocollo di validazione](VALIDATION.md).

Il daemon Docker usato per le prove esegue Linux aarch64. Permette di compilare il codice Rust per Linux, eseguire i test e verificare resolver e manifest. La riproduzione su un PC Linux x86_64 con sessione desktop e GPU richiede un collaudo separato.

`Dockerfile.linux` usa l'immagine ufficiale `rust:1.92-slim-bookworm` per compilare e Debian Bookworm per l'esecuzione. La piattaforma segue quella del daemon: sul daemon corrente la compilazione è nativa aarch64. Un'eventuale esecuzione `--platform linux/amd64` su questo host sarebbe una prova in emulazione, da registrare come tale.

## Contesto di build

Esegui i comandi dalla directory del progetto. Prima della build, controlla che `.dockerignore` escluda almeno `target`, `.cache`, `profiles`, `reports` e `.git`. I profili browser possono contenere dati privati: tienili fuori dal contesto inviato al daemon.

Le build scaricano le immagini e le dipendenze Cargo; i test unitari non contattano le emittenti. I comandi `probe` e `play` invece effettuano richieste reali ai servizi sorgente.

## Test Rust nativi Linux

```sh
docker build --target checks -f Dockerfile.linux -t lume-player:linux-checks .
```

Il target `checks` esegue `cargo test --locked --all-targets` durante la build. Un esito positivo dimostra l'esecuzione dei test Rust in Linux sulla piattaforma registrata dal daemon. Se il layer è già in cache, Docker può riutilizzare un'esecuzione precedente: conservare il log e annotarlo. Per una nuova esecuzione esplicita dei soli test:

```sh
docker run --rm --entrypoint cargo lume-player:linux-checks test --locked --all-targets
```

I test non verificano Widevine, il video o l'audio. Questo target mantiene il toolchain e i sorgenti per rendere possibile la ripetizione della suite.

## Controllo API CEF senza SDK

```sh
docker build --target cef-api-check -f Dockerfile.linux -t lume-player:linux-cef-api .
```

Il target eredita `checks`: esegue prima i test Rust, poi `cargo check --locked --features embedded-cef,cef/dox --bins`. La feature `cef/dox` verifica i tipi e le API Rust dei companion CEF su Linux senza scaricare o collegare l’SDK nativo. Il controllo si ferma prima del packaging e dell’avvio di CEF e Widevine, senza contattare le emittenti.

Lo stesso controllo è configurato nella CI per Linux, Windows e macOS. Un job riuscito riguarda il codice Rust; la riproduzione CEF su Linux e Windows richiede ancora una prova desktop dedicata. La matrice CEF macOS verificata separatamente è descritta in [EMBEDDED.md](EMBEDDED.md).

### Esecuzione locale dell'8 novembre 2026

La build Docker aarch64 ha eseguito con successo **43 test Rust** nello snapshot inviato al daemon: 35 della libreria, 6 del CLI e 2 contratti HTTP su loopback. Prima del controllo API CEF, il daemon ha fallito il montaggio del layer successivo (`overlay mount: invalid argument`): primo log (`reports/linux-cef-api-build-first-attempt.log`).

Il secondo tentativo si è fermato prima della build perché il filesystem del daemon era in sola lettura (`read-only file system`): log del tentativo (`reports/linux-cef-api-build.log`). Il controllo API CEF Linux resta da eseguire dopo il ripristino del daemon. Entrambi i guasti riguardano Docker e sono avvenuti prima della compilazione CEF.

## Resolver e manifest

```sh
docker build --target runtime -f Dockerfile.linux -t lume-player:linux .
docker run --rm lume-player:linux channels
```

Crea la directory `reports` sul computer host e montala per conservare i risultati:

```sh
docker run --rm --mount "type=bind,source=$PWD/reports,target=/reports" lume-player:linux probe --channel rai1 --report /reports/linux-rai1-probe.json
docker run --rm --mount "type=bind,source=$PWD/reports,target=/reports" lume-player:linux probe --channel canale5 --report /reports/linux-canale5-probe.json
docker run --rm --mount "type=bind,source=$PWD/reports,target=/reports" lume-player:linux probe --channel la7 --report /reports/linux-la7-probe.json
```

Questi esempi sono per shell POSIX su macOS/Linux. Il bind mount contiene solo i report, non profili o credenziali del browser. Nessuna porta del container viene pubblicata sull'host.

Il target `runtime` contiene soltanto il programma e i certificati di sistema. Il CLI usa CEF come runtime predefinito, ma l’immagine non include il companion o il framework: è quindi previsto che `doctor` possa riportare `runtime_missing`. I probe verificano risoluzione e manifest e funzionano anche senza un player disponibile.

## Chromium opzionale per diagnostica esterna

```sh
docker build --target diagnostic-chromium -f Dockerfile.linux -t lume-player:linux-chromium .
```

Il target opzionale aggiunge Chromium Debian e il relativo helper sandbox, poi esegue il programma come utente non privilegiato. Per questa diagnostica con browser installato usa `--runtime browser`. Il CDM Widevine è assente; i componenti DRM disponibili in Chrome desktop x86_64 vanno verificati separatamente da quelli di Chromium Linux ARM.

Una prima prova diagnostica può usare:

```sh
docker run --rm --init --shm-size=256m lume-player:linux-chromium doctor --runtime browser --browser /usr/bin/chromium --report /tmp/linux-doctor.json
```

Per conservare il risultato di una prova headless, usare un container nominato e copiarne il report dopo l'uscita:

```sh
docker run --name lume-linux-chromium-probe --init --shm-size=256m lume-player:linux-chromium play --runtime browser --channel rai1 --seconds 30 --headless --browser /usr/bin/chromium --profile /tmp/lume-profile --report /tmp/linux-rai1-headless.json
docker cp lume-linux-chromium-probe:/tmp/linux-rai1-headless.json reports/linux-rai1-headless.json
```

Il comando `play` può terminare con un codice diverso da zero in caso di risultato negativo o incompleto; verificare comunque se il report è stato generato. Il container nominato rimane disponibile per l'ispezione e il nome deve essere diverso per eventuali nuove prove.

La configurazione di sandbox e namespace del daemon può impedire l’avvio. Registra l’errore mantenendo attiva la sandbox del browser. L’assenza del CDM e i problemi di sandbox o rendering osservati nel container riguardano quell’ambiente; per valutare il canale su Linux desktop serve una prova nella sessione grafica.

Nell'ambiente provato, il profilo seccomp predefinito del container ha impedito la creazione dei namespace. Una seconda prova, in un container temporaneo non privilegiato **senza mount dell'host**, ha allentato soltanto il filtro seccomp del container; il sandbox di Chromium è rimasto attivo:

```sh
docker run --name lume-linux-isolated --init --shm-size=256m --security-opt seccomp=unconfined lume-player:linux-chromium matrix --runtime browser --channels rai1,canale5,la7 --seconds 30 --headless --browser /usr/bin/chromium --profile /tmp/lume-profile --report /tmp/linux-container-isolated-matrix.json
docker cp lume-linux-isolated:/tmp/linux-container-isolated-matrix.json reports/linux-container-isolated-matrix.json
```

Il risultato storico con browser installato è descritto in [RESULTS.md](RESULTS.md): Rai 1 riprodotto in headless per oltre 30 secondi; Widevine non disponibile per la configurazione AVC/AAC richiesta nel Chromium ARM64 provato. Canale 5 e La7 si sono fermati alla verifica EME, senza inviare richieste di licenza. Questi report non riguardano CEF incorporato. I comandi qui riportati includono `--runtime browser` per ripetere quel percorso con il CLI corrente.

Per completare la matrice desktop usa un host Linux con sessione grafica, browser e CDM appropriati e segui [VALIDATION.md](VALIDATION.md). Immagine sul monitor, accelerazione GPU, ascolto e fullscreen richiedono questa prova; conserva separatamente i risultati headless.

## Stato da riportare

Registrare separatamente:

- Test Rust Linux aarch64: eseguiti/superati/falliti, con log.
- Controllo API Rust CEF Linux aarch64: esito del target `cef-api-check`, senza SDK o riproduzione.
- Probe Rai, Mediaset e La7 Linux aarch64: risultato del singolo report.
- Chromium headless Linux aarch64: eventuale diagnostica, CDM e limiti osservati.
- Riproduzione CEF grafica Linux: **non eseguita** finché manca un'esecuzione del companion con SDK/CDM in una sessione desktop documentata.

Associa a ogni risultato il log o il report dell’esecuzione. Il controllo API resta da ripetere dopo il ripristino del filesystem del daemon; i risultati precedenti di resolver e browser sono conservati nei propri report.

## Riferimenti

- [Immagine ufficiale Rust e architetture supportate](https://hub.docker.com/_/rust).
- [Pacchetto Chromium in Debian Bookworm](https://packages.debian.org/bookworm/chromium).
- [Google Widevine: piattaforme e integrazione](https://developers.google.com/widevine/drm/overview).
