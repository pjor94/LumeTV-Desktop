# Protocollo di validazione del player

Il [riepilogo delle verifiche](STATUS.md) riporta i risultati correnti. I file
citati in `reports/` restano locali ed esclusi da Git. Per confrontare nuove
esecuzioni usa sempre lo stesso [protocollo](VALIDATION.md).

Il runtime predefinito è il companion Rust/CEF (`--runtime cef`). La diagnostica con Chrome o Edge installato richiede `--runtime browser`. Registra separatamente i risultati dei due backend e i controlli dei resolver. Per valutare la distribuzione di un pacchetto con CDM servono anche le verifiche sul pacchetto, oltre alla riproduzione locale.

## Stato delle piattaforme

Stato aggiornato all'8 novembre 2026. La tabella distingue CEF incorporato dalle prove precedenti con browser installato. Risultati e limiti sono in [EMBEDDED.md](EMBEDDED.md) e [RESULTS.md](RESULTS.md).

| Piattaforma | Ambiente disponibile | CEF incorporato | Browser installato: evidenze precedenti |
| --- | --- | --- | --- |
| macOS | Host arm64, macOS 26.5.2; sessione grafica normale | Superata la matrice Rai 1/Canale 5/La7 con CEF OBS 150.0.19, almeno 30 s per canale; Widevine verificato per Canale 5 e La7. Report CEF (`reports/macos-cef-obs150-matrix.json`) | Chrome 156.0.8078.80: matrice dei tre canali superata il 7 novembre. Report browser (`reports/macos-matrix.json`) |
| Windows | VM Windows 11 presente in Parallels; CI Rust/API CEF configurata | Riproduzione CEF non eseguita | Non eseguita: avvio VM impedito dall'attivazione Parallels scaduta; VM rimasta sospesa |
| Linux | Docker aarch64: 43 test Rust superati nello snapshot della build; controllo tipi CEF fermato prima dell'esecuzione da un guasto al filesystem del daemon. Log (`reports/linux-cef-api-build-first-attempt.log`), [procedura](LINUX.md) | Riproduzione CEF desktop non eseguita | Parziale: Rai 1 riprodotto in Chromium headless; Widevine non disponibile per Canale 5/La7 nella configurazione provata. Report browser (`reports/linux-container-isolated-matrix.json`) |

Registra build e prove live come risultati distinti. Una VM è utile per una prima verifica; i risultati riguardano i decoder, la GPU e le protezioni disponibili in quell’ambiente e possono differire da quelli di un PC fisico.

Una prima prova di download automatico Widevine su macOS arm64 ha installato il componente in due profili inizialmente vuoti, senza sorgente Chrome o seeding. La riproduzione è rimasta inconcludente: il runtime non aveva richiesto nemmeno una pagina locale sintetica e le esecuzioni sono terminate in timeout. Riepilogo e limiti (`reports/macos-cef-auto-download-summary.json`). Una successiva prova nella UI ha verificato Canale 5 da profilo browser vuoto, dopo il download automatico e senza riavvio: vedi [lo stato corrente](STATUS.md#primo-download-widevine) e [la procedura](EMBEDDED.md#widevine-automatico-al-primo-avvio).

La matrice CEF macOS dell'8 novembre ha misurato **32,825 s/822 frame** per Rai 1, **31,871 s/1.595 frame** per Canale 5 e **31,852 s/797 frame** per La7. Tutti hanno decodificato audio e registrato la conferma manuale di udibilità nella UI. Canale 5 e La7 hanno prodotto eventi `encrypted`, chiavi `usable` e due scambi licenza HTTP riusciti ciascuno. Rai 1 era non protetto durante questa prova. Il CDM Widevine 4.10.3112.0 era fornito esternamente nel profilo dedicato; `distribution_verified` resta falso. Report (`reports/macos-cef-obs150-matrix.json`).

La baseline CEF stock 156 resta distinta: la finestra CEF e Widevine VP9/Opus erano disponibili, mentre AVC/AAC non erano supportati; i tre esiti erano `codec_unavailable`. Non è stata classificata come assenza del CDM o rifiuto di licenza. Report stock (`reports/macos-cef-stock156-matrix.json`).

La matrice macOS del 7 novembre, eseguita fra le 23:38 e le 23:40 Europe/Rome, ha misurato 32,791 s/821 frame per Rai 1, 31,768 s/1.589 frame per Canale 5 e 32,839 s/821 frame per La7. Tutti hanno decodificato audio; la conferma manuale dell’audio nella UI è registrata per Canale 5 e La7. Rai 1 resta privo di conferma di udibilità. Questi risultati riguardano il browser installato; non convalidano un runtime CEF/CDM incorporato.

Sono conservati due tentativi distinti con Chromium 156.0.8078.79 su Debian 12 aarch64:

1. Con il profilo seccomp predefinito del container, Chromium non si è avviato: creazione del namespace rifiutata (`Operation not permitted`), uscita del processo con codice 134, zero eventi e richieste. **EME/Widevine non testati in questo primo tentativo**. Matrice (`reports/linux-container-matrix.json`), log di avvio (`reports/linux-browser-startup.log`).
2. Nel container isolato con `--security-opt seccomp=unconfined`, utente non root e senza mount dell’host, Chromium ha mantenuto la propria sandbox e ha eseguito il player. Rai 1 ha prodotto **32,835 s**, **823 frame** e **1.334.683 byte audio decodificati**, con verdetto `headless_playback_observed`. MSE, EME e supporto AVC/AAC erano disponibili, mentre Widevine non era disponibile per la configurazione richiesta (`eme_error_code: 9`). Canale 5 e La7 si sono fermati prima della riproduzione e senza richieste di licenza, con verdetto `widevine_unavailable`: nessun rifiuto del provider è stato osservato. Report (`reports/linux-container-isolated-matrix.json`).

Il secondo risultato non verifica desktop, GPU o udibilità audio e non dimostra incompatibilità generale di Linux con Widevine. Occorre ripetere la prova in una sessione Linux desktop con browser/CDM disponibili. I container sono stati rimossi dopo aver acquisito i report; Windows resta da eseguire dopo il ripristino dell’avvio della VM.

## Livelli di evidenza

| Livello | Che cosa dimostra | Che cosa resta da verificare |
| --- | --- | --- |
| Ambiente | OS, backend, versioni ed eseguibile disponibili | Connettività e compatibilità della sorgente |
| Resolver | L'integrazione produce una configurazione di riproduzione | Accesso al manifest, licenza, codec e video |
| Manifest | Il manifest è raggiungibile e interpretabile | Segmenti multimediali e decodifica |
| Controllo tipi Rust CEF | Il codice usa API Rust valide sulla piattaforma, con `cef/dox` e senza SDK | Collegamento nativo, avvio CEF, codec, CDM e riproduzione |
| Capacità EME | Il runtime dichiara un sistema DRM e una configurazione utilizzabili | Accettazione della licenza da parte del fornitore |
| Riproduzione | Il tempo video avanza e vengono osservati frame decodificati | Qualità visiva, audio udibile, stabilità prolungata |
| Pacchetto distribuibile | Firma di distribuzione, installazione e percorso di aggiornamento CDM verificati | Non convalidato dal bundle locale di sviluppo |

Conserva nel report le evidenze disponibili per ciascun livello. Se manca quella richiesta, classifica il test come fallito o inconcludente. Un manifest HTTP 200 conferma solo l’accesso al manifest.

## Procedura su ciascun sistema

1. Registrare OS/architettura, backend, versioni CEF/Chromium o del browser, provenienza SDK/codec/CDM, modalità diretta/relay, tipo di sessione grafica e uso di VM. Evitare configurazioni di rete diverse quando si confrontano risultati.
2. Per CEF preparare companion, SDK e profilo come in [EMBEDDED.md](EMBEDDED.md), quindi eseguire `doctor` e salvare il report. Per la diagnostica esterna scegliere esplicitamente `--runtime browser`; annotare il browser usato.
3. Eseguire `probe` per `rai1`, `canale5` e `la7`. Questo isola i problemi di risoluzione e manifest.
4. Eseguire la matrice in modalità grafica normale. Osservare il player e conservare il report.
5. In caso di errore al primo avvio, distinguere runtime mancante, codec non supportati, CDM non disponibile e licenza fallita. Ripetere un singolo canale con il profilo dedicato e il percorso CDM previsto dal backend; annotare entrambe le esecuzioni.
6. Confrontare `relay` e `direct` quando l'errore riguarda le richieste di rete. I due risultati sono distinti.
7. Togliere il mute e verificare manualmente l'audio. Controllare immagine, sincronizzazione e comportamento a tutto schermo.

Esempi CEF, dopo la preparazione del bundle e del CDM e usando il profilo già predisposto:

```sh
cargo run --locked -- doctor --runtime cef --cef-executable "$LUME_CEF_EXE" --report reports/cef-doctor.json
cargo run --locked -- probe --channel rai1 --report reports/rai1-probe.json
cargo run --locked -- probe --channel canale5 --report reports/canale5-probe.json
cargo run --locked -- probe --channel la7 --report reports/la7-probe.json
cargo run --locked -- matrix --runtime cef --cef-executable "$LUME_CEF_EXE" --profile "$PWD/profiles/cef-obs" --seconds 30 --report reports/cef-matrix.json
```

Per ripetere invece la diagnostica precedente con Chrome/Edge installato:

```sh
cargo run --locked -- doctor --runtime browser --report reports/browser-doctor.json
cargo run --locked -- matrix --runtime browser --seconds 30 --report reports/browser-matrix.json
```

Il controllo CI `cargo check --locked --features embedded-cef,cef/dox --bins` verifica le API Rust sui tre sistemi, senza scaricare o collegare l’SDK CEF. Il target Docker `cef-api-check` esegue lo stesso controllo su Linux dopo i test core. Nell’esecuzione dell’8 novembre i test core sono passati, ma il controllo API non è partito per il guasto del daemon descritto in [LINUX.md](LINUX.md). Video e DRM richiedono comunque una prova nel runtime.

I 30 secondi servono per una prima verifica. Per valutare la stabilità, aggiungi esecuzioni più lunghe, cambi ripetuti di canale e casi di rinnovo del token/licenza.

Il comando aggiunge due secondi al timer del browser per assorbire lo scarto fra campionamento e ricezione locale. Il valutatore Rust mantiene la soglia richiesta, usa i timestamp del server e conta il minore fra avanzamento del video e tempo trascorso. Scarta seek, stalli e intervalli di campionamento eccessivi e richiede un incremento dei frame. Il solo flag di successo della pagina non basta a determinare il verdetto.

## Criteri per la decisione sullo stack

- Almeno una prova video ripetibile per ciascuna integrazione e ciascun sistema operativo richiesto.
- Risultati distinti per contenuti senza DRM e contenuti che hanno richiesto effettivamente una sessione DRM. Se il programma in onda è privo di DRM, quella prova non valida Widevine per l'emittente.
- Verifica manuale di audio, finestra video e fullscreen nella sessione grafica normale.
- Errori di rete, manifest, licenza e codec distinguibili nei report.
- Profilo nuovo e profilo già inizializzato entrambi esaminati.
- Verifica separata di un pacchetto con il motore incorporato e del suo percorso CDM prima di dichiararlo distribuibile.

Il risultato finale della matrice deve riportare per ogni combinazione `superato`, `fallito`, `inconcludente` oppure `non eseguito`, con collegamento al report. Un browser che dichiara Widevine disponibile può comunque ricevere un rifiuto dal server licenze o non supportare il codec richiesto.

Per classificare come verificata una riproduzione DRM, il rapporto deve contenere anche un evento `encrypted`, una chiave `usable` e uno scambio licenza HTTP riuscito registrato da Rust. Una sorgente che in quella esecuzione non richiede DRM valida soltanto il percorso non protetto osservato.

## Limiti e diagnostica

Il player parte in mute; il programma non certifica che l'audio sia udibile. Le prove headless sono diagnostiche e vanno conservate separatamente dalle prove grafiche. Un report positivo dimostra il comportamento osservato in quel momento: disponibilità dei canali, contenuti protetti e risposte dei fornitori possono cambiare.

Per confrontare un errore, verificare il player ufficiale dell'emittente sullo stesso sistema e sulla stessa rete. Annotare il risultato come riferimento separato: un player ufficiale funzionante non dimostra che il resolver o il player locale funzionino.

I report destinati alla condivisione devono escludere URL firmati completi, cookie, token e messaggi di licenza. Il relay opera solo come adattatore HTTP del prototipo; la decodifica dei contenuti protetti resta affidata al CDM del runtime selezionato.

## Fonti tecniche

- [Google Widevine: piattaforme e requisiti dell'integrazione](https://developers.google.com/widevine/drm/overview).
- [Shaka Player: supporto di protocolli, browser e DRM](https://github.com/shaka-project/shaka-player).
- [Tauri: motori WebView per piattaforma](https://v2.tauri.app/reference/webview-versions/).
- [Electron upstream: integrazione CDM e responsabilità dell'applicazione](https://raw.githubusercontent.com/electron/electron/main/docs/tutorial/testing-widevine-cdm.md).

Le fonti descrivono i requisiti dei componenti; per verificare le tre emittenti nel player servono le prove di questo protocollo.
