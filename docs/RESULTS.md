# Risultati osservati del prototipo

Queste prove del 7 novembre 2026 riguardano il prototipo con browser esterno.
Il player attuale usa CEF incorporato; per lo stato corrente consulta
[STATUS.md](STATUS.md) e [EMBEDDED.md](EMBEDDED.md).

Il [riepilogo delle verifiche](STATUS.md) raccoglie risultati e limiti attuali.
I file citati in `reports/` restano locali ed esclusi da Git. Per ripetere le
prove segui il [protocollo di validazione](VALIDATION.md).

## Ambito e ambiente

Il **7 novembre 2026**, fra le **23:38 e le 23:40 Europe/Rome**, il prototipo ha riprodotto Rai 1, Canale 5 e La7 sul dispositivo macOS disponibile. La matrice registra tre esecuzioni completate, tutte con verdetto `playback_verified`: report completo macOS (`reports/macos-matrix.json`).

Ambiente: macOS 26.5.2, architettura **arm64/aarch64**, **Google Chrome 156.0.8078.80 installato**, sessione grafica normale (`headless: false`), richieste multimediali in modalità **relay** attraverso Rust. Shaka esegue la riproduzione nel browser; il browser gestisce il CDM. Il report doctor (`reports/macos-doctor.json`) registra il runtime rilevato.

La soglia richiesta era di **30 secondi** di riproduzione misurata. Il browser ha ricevuto una durata di 32 secondi, con un margine di due secondi per la misurazione; la valutazione indipendente Rust ha mantenuto la soglia di 30 secondi. I tempi riportati sotto sono `playback.progress_seconds`, calcolati dai campioni e limitati dal tempo di ricezione sul server.

## Riproduzione live macOS

| Canale | Avvio Europe/Rome | Formato osservato | Video verificato | Avanzamento misurato | Incremento frame decodificati | DRM osservato |
| --- | --- | --- | --- | ---: | ---: | --- |
| Rai 1 (`reports/macos-rai1-play.json`) | 23:38:42 | HLS senza marcatori di cifratura nel manifest | Superato | 32,791 s | 821 | Non richiesto in questa esecuzione |
| Canale 5 (`reports/macos-canale5-play.json`) | 23:39:20 | DASH protetto | Superato | 31,768 s | 1.589 | Widevine verificato |
| La7 (`reports/macos-la7-play.json`) | 23:39:56 | DASH protetto | Superato | 32,839 s | 821 | Widevine verificato |

Per tutti e tre i canali sono stati osservati avanzamento del tempo video, frame crescenti e dimensioni video non nulle. Nessuna esecuzione è terminata per timeout o interruzione. I report registrano zero richieste di rete fallite; questo dato riguarda soltanto la breve finestra osservata.

Per **Canale 5 e La7**, la verifica DRM comprende tutte le evidenze seguenti:

- supporto EME/Widevine dichiarato dal browser per una configurazione AVC/AAC concreta;
- manifest DASH con marcatori di cifratura;
- eventi `encrypted` durante la riproduzione;
- stato della chiave `usable`;
- risposta HTTP riuscita alle richieste di licenza: due scambi riusciti per ciascun canale;
- avanzamento video e incremento dei frame per almeno la durata richiesta.

La verifica richiede tutte queste evidenze. La sorgente Rai 1 osservata non richiedeva una sessione DRM; gli eventuali contenuti Rai protetti richiedono una prova separata.

## Audio

| Canale | Byte audio decodificati osservati | Conferma manuale nell’interfaccia |
| --- | ---: | --- |
| Rai 1 | 1.333.657 | Non registrata; udibilità non verificata |
| Canale 5 | 400.293 | Registrata (`audio_audibility: manually_verified`) |
| La7 | 402.029 | Registrata (`audio_audibility: manually_verified`) |

Il player parte con l’audio disattivato. Il contatore dei byte misura la decodifica; per verificare l’uscita dagli altoparlanti serve l’ascolto. Per Canale 5 e La7 il report conserva l’azione “Audio verificato” registrata nella UI come conferma manuale. Non è stata eseguita una misura acustica automatica né un test separato della sincronizzazione audio/video.

## Verifiche del codice

- **33 test Rust superati** sul target nativo macOS arm64: 29 test di libreria, due test CLI e due test HTTP locali. Evidenza: log dei test macOS (`reports/macos-tests.log`).
- **9 test Node superati** per l’harness browser: redazione degli errori e degli stati delle chiavi, richieste relay, challenge binarie, progressione/frame, URI dopo redirect e cancellazione.
- **Clippy superato** sul target nativo macOS arm64 con `-D warnings`; controllo `rustfmt` superato.
- **33 test Rust superati anche in Linux aarch64**, durante il target Docker `checks`: log della build e dei test Linux (`reports/linux-build-isolated.log`). Il risultato riguarda il codice Rust e i test locali, senza riproduzione video.

I test HTTP locali verificano anche l’accesso alla sessione tramite token e i controlli su host/origine, la mancata esposizione dei segreti dei provider e il rifiuto delle richieste relay non ammesse. I test dei report escludono successi fondati soltanto su `loaded` o `complete.success`, seek, stalli e frame fermi; distinguono inoltre le prove DRM e la conferma manuale dell’audio.

## Altre piattaforme

| Piattaforma | Stato verificato | Riproduzione live |
| --- | --- | --- |
| Windows 11 | VM presente; avvio impedito dall’attivazione Parallels scaduta | **Non eseguita**; nessun verdetto di compatibilità |
| Linux aarch64 in Docker | Immagine release compilata; 33 test Rust superati; resolver e manifest delle tre sorgenti verificati | **Parziale**: Rai 1 riprodotto in headless nel secondo tentativo; Widevine non disponibile nel Chromium provato per Canale 5/La7. **Non eseguita** la prova desktop |

La procedura Linux è in [LINUX.md](LINUX.md). Build e test nel container verificano il codice in quell’ambiente; una sessione desktop Linux con GPU, browser e CDM richiede un collaudo proprio. Lo stesso vale per Windows: il risultato macOS riguarda soltanto quella piattaforma.

### Probe Linux aarch64

Il 7 novembre 2026 alle **23:44 Europe/Rome**, il binario Rust eseguito nel container Linux ha risolto tutte e tre le sorgenti e scaricato i relativi manifest con **HTTP 200**:

| Canale | Manifest | Marcatori di cifratura | Verdetto del probe |
| --- | --- | --- | --- |
| Rai 1 (`reports/linux-rai1-probe.json`) | HLS | Assenti | `manifest_only` |
| Canale 5 (`reports/linux-canale5-probe.json`) | DASH | Presenti | `manifest_only` |
| La7 (`reports/linux-la7-probe.json`) | DASH | Presenti | `manifest_only` |

Queste esecuzioni non hanno richiesto licenze né riprodotto video o audio. Dimostrano il funzionamento dei resolver e l’accesso ai manifest in quel container Linux, senza convalidare decodifica, Widevine o il desktop Linux.

### Primo tentativo browser Linux: blocco con il profilo seccomp predefinito

È stata tentata anche la matrice con **Chromium 156.0.8078.79 su Debian 12 aarch64**, in modalità **headless/relay**, alle 23:44 Europe/Rome: report della matrice nel container (`reports/linux-container-matrix.json`).

Tutti e tre i tentativi registrano `playback_not_verified`, zero eventi dalla pagina e zero richieste multimediali o licenze nel relay. La prova separata di avvio del browser su una pagina vuota documenta il blocco prima dell’esecuzione del player: creazione del namespace rifiutata con `Operation not permitted`, uscita prematura del processo zygote e processo browser terminato con codice 134. Evidenza: log di avvio Chromium (`reports/linux-browser-startup.log`).

Il primo tentativo è inconcludente: il browser si è fermato prima dei controlli EME e Widevine, quindi il supporto DRM e i canali restano da valutare. La sandbox del browser è rimasta attiva e il container usava il profilo seccomp predefinito. I report di questo tentativo sono separati da quelli della ripetizione seguente.

### Secondo tentativo browser Linux: Rai 1 riprodotto, Widevine non disponibile

Fra le **23:46 e le 23:47 Europe/Rome**, la matrice è stata ripetuta con lo stesso Chromium headless su Debian 12 aarch64, in un container eseguito da **utente non root, senza mount dell’host**, con `--security-opt seccomp=unconfined`. La sandbox di Chromium è rimasta attiva. Questa configurazione ha permesso l’avvio del browser: report della seconda matrice (`reports/linux-container-isolated-matrix.json`).

| Canale | Esito osservato | Avanzamento misurato | Incremento frame | Audio decodificato |
| --- | --- | ---: | ---: | ---: |
| Rai 1 | `headless_playback_observed` | 32,835 s | 823 | 1.334.683 byte |
| Canale 5 | `widevine_unavailable`; riproduzione non avviata | 0 s | 0 | 0 byte |
| La7 | `widevine_unavailable`; riproduzione non avviata | 0 s | 0 | 0 byte |

Tutte e tre le pagine hanno segnalato MSE, API EME e supporto AVC/AAC disponibili. La richiesta EME per `com.widevine.alpha`, con i codec AVC/AAC previsti dall’harness, ha invece restituito `widevine_available: false` e `eme_error_code: 9`.

Rai 1, privo di DRM nella sorgente osservata, ha superato la soglia di 30 secondi con frame crescenti e audio decodificato. Per Canale 5 e La7 il test si è fermato al controllo Widevine, prima di inviare richieste multimediali o di licenza dal relay. Il componente non era disponibile in quella configurazione di Chromium; non è stato osservato un rifiuto del provider.

La riproduzione headless di Rai 1 non verifica finestra desktop, GPU, qualità percepita o audio udibile; `audio_audibility` resta `not_verified`. Il risultato DRM non si estende ad altri browser, architetture o ambienti Linux. Il container è stato rimosso dopo aver acquisito il report. Serve ancora una prova su Linux desktop con un browser e un CDM Widevine disponibili.

## Esito e verifiche successive

Il percorso resolver Rust → relay locale → Shaka → Chrome installato ha funzionato su macOS arm64, compresa la riproduzione Widevine di Canale 5 e La7.

Su Linux aarch64 sono riusciti build, test Rust e accesso ai manifest delle tre sorgenti. Rai 1 è stato decodificato in Chromium headless nel container isolato. La riproduzione DRM resta da provare in un ambiente Linux con Widevine disponibile.

Restano da verificare Windows, dopo il ripristino dell’avvio della VM, e Linux desktop con browser/CDM disponibili. Servono inoltre prove prolungate, rinnovi di token/licenza, cambi ripetuti di canale, fullscreen e sincronizzazione audio/video. La modalità `direct` non è stata collaudata in queste matrici. Questa versione del prototipo usava il browser installato, senza incorporare CEF o distribuire un CDM; il pacchetto desktop autonomo richiedeva ancora una verifica separata.

I risultati valgono per dispositivo, browser, rete, canali e programmi della finestra indicata. La parità completa con Lume TV Android e la compatibilità con tutte le emittenti restano da verificare. Per consultare i file sotto `reports/` occorrono gli output locali dell’esecuzione, oppure una copia allegata al documento; sono esclusi da Git.
