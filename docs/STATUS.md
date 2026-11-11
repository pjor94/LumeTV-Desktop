# Stato del progetto e verifiche

Ultimo aggiornamento dei sorgenti: **11 novembre 2026**. Le nuove prove della UI
React sono descritte qui sotto; le sezioni successive conservano i risultati
dell’8 e 10 novembre. La disponibilità delle emittenti può cambiare.

## Portachiavi macOS dell’11 novembre

La build locale disabilita `UseKeychainKeyProvider` per il contesto browser in
memoria di Lume. Questa feature di Chromium 150 avvia preventivamente OSCrypt
e può provocare la richiesta “Chromium Safe Storage” anche con i cookie disabilitati.
La modifica è limitata a macOS e conserva le altre feature della riga di comando.
Preferenze e sessioni dei provider continuano a essere gestite da Rust.

Il primo avvio, da una cartella browser assente, ha completato il download
Widevine 4.10.3112.0 e aperto il catalogo senza richieste del Portachiavi.
Anche un profilo usato prima della modifica si è riaperto senza richieste.
La chiusura del primo avvio non ha attivato il limite di teardown di cinque secondi.

La matrice nativa con la nuova build macOS arm64 ha misurato:

| Canale | Avanzamento misurato | DRM |
| --- | --- | --- |
| Rai 1 | 37,939 s | Non richiesto |
| Canale 5 | 37,926 s | Verificato: evento cifrato, licenza riuscita e chiave utilizzabile |
| La7 | 36,947 s | Verificato: evento cifrato, licenza riuscita e chiave utilizzabile |

Tutti i canali hanno mostrato avanzamento dei frame e decodifica audio.
L’audio era disattivato; l’udibilità e le immagini protette sul monitor restano
da confermare separatamente. Sono passati 81 test della libreria Rust, quattro
test del binario CEF, formattazione e Clippy del runtime nativo.
Il [riepilogo pubblico](evidence/2026-11-11-keychain.json) conserva le metriche
selezionate e gli hash dei sorgenti modificati, senza dati delle licenze o URL dei flussi.
La correzione è inclusa nella versione 0.2.0.

## Radio dell’11 novembre

La sezione Radio include 1.310 stazioni italiane di WorldRadio, ricerca per nome
e città, preferite, recenti, aggiornamento con cache e ascolto nel mini-player.
TV e radio condividono il player e mantengono separati cataloghi, guida e zapping.
La riconnessione prevede tre tentativi e si annulla con Pausa, cambio o chiusura.

Sono passati 91 test Rust, 40 test unitari JavaScript, 66 Python e 27 scenari
browser, comprese le catture visive. Formattazione, Clippy, bundle React e
controlli del repository sono passati. La UI Radio è stata controllata nei due
temi a 820 e 1280 pixel; il titolo poco leggibile nel tema chiaro è stato corretto.

Riproduzioni misurate nella finestra nativa macOS arm64:

| Sorgente | Formato | Avanzamento nei campioni | Metadati ricevuti |
| --- | --- | --- | --- |
| Rai Radio 1 | MP3 continuo | 62 s | No |
| RDS FM 103.0 | MP3 continuo | 216 s | Sì |
| R101 | AAC continuo | 56 s | Sì |
| Riferimento audio Shaka | HLS/AAC | 62 s | Non richiesti dalla prova |

Il riferimento HLS è stato aggiunto soltanto al profilo di collaudo e poi rimosso.
Radio Spazio Noi, presente nel catalogo, ha restituito una playlist secondaria
vuota con HTTP 200; il player ha mostrato l’errore dopo i tentativi previsti.
Il formato HLS è quindi verificato con una sorgente di riferimento, mentre la
disponibilità di questa emittente resta un problema della sorgente alla data
della prova. Non è stato collaudato l’intero catalogo.

Le misure mostrano avanzamento e byte audio decodificati, con audio disattivato;
l’ascolto va confermato separatamente. Il mini-player è stato provato anche nella
finestra nativa. Il passaggio Radio/TV ha ripreso il video di Rai 1; il runtime
è entrato e uscito dal Picture-in-Picture, tornando al video nella finestra
principale. Questa verifica è visiva, senza una nuova misura temporale TV.
Il [riepilogo delle misure](evidence/2026-11-11-radio.json) omette
indirizzi dei flussi, token e dati del profilo. Dettagli in [RADIO.md](RADIO.md).
Le prove native Windows, Linux e Mac Intel restano da eseguire. Le pipeline
mantengono i quattro target esistenti; le misure qui sopra riguardano solo macOS arm64.

## Distribuzione della 0.2.0

Versioni Rust, DLL Windows, frontend e lockfile sono allineati a `0.2.0`.
I tag ordinari pubblicano release normali marcate Latest; i tag beta/rc
pubblicano anteprime e il feed firmato usa lo stesso canale. Le anteprime sono
escluse per impostazione iniziale nell’app. La chiave di firma è configurata
come secret del repository.

Sono passati 82 test Rust, 36 JavaScript, 66 Python e 21 scenari browser.
I nuovi test controllano la classificazione della release, la corrispondenza
con il feed firmato, le versioni nei manifest e le note del changelog.
Formattazione, Clippy, bundle React e controlli dei file pubblici sono passati.

La release GitHub 0.1.0 è stata riclassificata come normale, mantenendo
i suoi 19 allegati e le note. La pubblicazione della 0.2.0 parte dal tag
e richiede il successo delle quattro build native.
Il comando `release_desktop.py prepare` salva le note e il piano locale senza
pubblicare. Il collaudo nativo dell’updater sugli altri sistemi e il passaggio
fra due release complete restano aperti.

## Aggiornamenti dell’app dell’11 novembre

Il codice include controllo delle release GitHub, preferenze del canale,
download con avanzamento e annullamento, firma Ed25519 e verifica SHA-256.
L’helper sostituisce il pacchetto completo nelle cartelle scrivibili e conserva
la copia precedente. Il DEB usa il gestore di sistema. Il feed viene firmato
nel job di pubblicazione; le vecchie release senza feed sono ignorate.

Sono passati 82 test Rust, 36 test JavaScript e 60 Python. I 21 scenari browser
comprendono download, annullamento, errori di verifica, impostazioni, installazione
dopo l’arresto della TV e assenza di avvisi sopra la diretta. La schermata è stata
controllata nei due temi a 820 e 1280 pixel.

Su macOS arm64 l’helper funziona fuori dal bundle, senza dipendenze da CEF.
La schermata Impostazioni nella finestra nativa ha completato il controllo
reale di GitHub e mostrato l’assenza di versioni più recenti ammissibili.
Una prova con bundle di fixture firmati ha verificato firma del feed, conferma
dell’helper, attesa del processo, estrazione dello ZIP con link del framework,
sostituzione da 0.1.0 a 0.1.1 e conservazione della copia precedente.
Il ripristino dopo un errore di spostamento è coperto da un test dedicato.
Questa prova usa eseguibili di fixture, non la TV completa. L’aggiornamento
tra due release reali e le prove native Windows/Linux e Mac Intel restano da
eseguire. La release pubblica 0.1.0 non dispone di updater: il primo passaggio
richiede un’installazione manuale. Dettagli in [UPDATES.md](UPDATES.md).
La secret di firma del repository è stata configurata con l’autorizzazione del
proprietario. La chiave privata resta esclusa dai sorgenti e dai pacchetti.

## Migrazione React dell’11 novembre

Interfaccia React e TypeScript con CSS, icone e animazioni della versione
precedente. Player e sessioni restano separati dalle pagine. La preparazione di
Widevine legge lo stato del component updater CEF e mostra attività, installazione,
errori e indicazione di riavvio Linux quando necessaria.

Sono passati 69 test Rust, 36 test unitari JavaScript e 56 Python. I test browser
coprono 16 scenari della UI di distribuzione. Preparazione iniziale e continuità
del player sono state verificate anche con React in sviluppo e StrictMode attivo.
Il confronto di 22 catture, nei due
temi e alle larghezze di 820 e 1280 pixel, ha mantenuto la composizione precedente.

Nella UI nativa macOS arm64 sono state misurate queste riproduzioni:

| Canale | Avanzamento misurato | Video decodificato | DRM |
| --- | --- | --- | --- |
| Rai 1 | 77,887 s | 1920 × 1080 | Non richiesto |
| Canale 5 | 119,885 s | 1280 × 720 | Verificato: evento cifrato, licenza riuscita e chiave utilizzabile |
| La7 | 61,861 s | 1280 × 720 | Verificato: evento cifrato, licenza riuscita e chiave utilizzabile |

Su Canale 5 sono stati provati anche fullscreen e mini-player. Il profilo di
prova contiene Widevine 4.10.3112.0, preparato da CEF senza copia da Chrome.
Il componente era già disponibile all’avvio della riproduzione: gli stati della
barra, gli errori e il riavvio Linux sono stati verificati con fixture.
L’audio era disattivato; l’ascolto e la conferma diretta sul monitor delle immagini
protette restano da completare. Le catture macOS dei due canali protetti sono nere.

Il [riepilogo delle misure](evidence/2026-11-11-react.json) conserva soltanto dati
selezionati. La [guida al frontend](FRONTEND.md) descrive struttura e comandi.
La build macOS è pronta; build e riproduzione della nuova versione su Windows e
Linux restano da verificare sui rispettivi sistemi. I tentativi locali di cross
check si sono fermati sulle toolchain C di destinazione mancanti nel Mac.

La preparazione obbligatoria all’avvio è stata provata anche nella finestra nativa
macOS con un nuovo profilo. Widevine 4.10.3112.0 è stato installato e il catalogo
si è aperto dopo la verifica, senza selezionare un canale: nessuna sessione o evento
di riproduzione nel riepilogo finale. Errori, riprova e riapertura Linux sono stati
controllati con fixture; la prova nativa Linux resta da eseguire.

## Risultati precedenti

Il [riepilogo JSON pubblico](evidence/2026-11-08.json) raccoglie le metriche
selezionate per la condivisione e i checksum dei file locali da cui provengono.
Non contiene URL delle sorgenti, token, header, profili o dati delle licenze.
I report completi e i log restano locali in `reports/`, esclusi da Git.

## App e piattaforme

| Ambito | Osservato | Da completare |
| --- | --- | --- |
| macOS Apple Silicon | Build, bundle ad hoc, finestra Rust/CEF e catalogo completo; prove Rai/Mediaset/La7. | Conferma visiva diretta dei protetti nell’ultima build, prove prolungate, pacchetto di produzione. |
| macOS Intel | Build nativa, DMG/ZIP e controllo dei metadati dal pacchetto superati in CI. | Finestra, riproduzione, Widevine e collaudo prolungato. |
| Windows | ZIP x64 con bootstrap sandbox e controllo dei metadati dal pacchetto superati in CI. | Finestra, Widevine e riproduzione desktop. |
| Linux | DEB/tar.gz x64 e controllo dei metadati dal pacchetto superati in CI. Nelle prove storiche: core Docker arm64 e Rai 1 headless. | Finestra CEF desktop, GPU, CDM e riproduzione. |
| Autoaggiornamento app | Codice, UI, feed firmato, test di verifica e sostituzione nativa macOS con fixture. | Pubblicazione della prima build con updater, passaggio fra release reali e prove native sugli altri sistemi. |

I controlli del core Rust e delle API CEF della release 0.1.0 sono passati sui tre sistemi.
Il controllo con `cef/dox` verifica le API Rust senza collegare o avviare CEF.

## Pacchetti desktop del 10 novembre

L’[esecuzione nativa del 10 novembre](https://github.com/pjor94/LumeTV-Desktop/actions/runs/42802395102)
ha superato tutti i job: DMG e ZIP per Mac Apple Silicon e Intel, ZIP Windows x64,
DEB e tar.gz Linux x64 e archivio dei sorgenti con le dipendenze Cargo.
Ogni pacchetto contiene gli avvisi dei componenti, metadati e checksum.
Il controllo automatico esegue il programma per leggere piattaforma, backend,
sandbox e versione API; finestra e riproduzione richiedono prove separate.

Sono passati anche i 53 test Python degli strumenti, compresi i controlli sui
percorsi MSVC e sulle date dei file SDK negli ZIP. Le cache SDK recuperate da
GitHub vengono sempre ricontrollate prima della compilazione.

La [guida alle release](RELEASE.md) descrive formati, download e limiti.
I pacchetti sono di sviluppo. Restano il collaudo desktop indicato nella tabella
e la preparazione della distribuzione di produzione.

## Catalogo e prime prove video

L’audit ha registrato **126 canali**, **119 risoluzioni riuscite** e **112 manifest
validi**, senza richiedere licenze DRM. Sono state raccolte metriche native
di almeno 30 secondi per sette canali rappresentativi: Rai 1, Canale 5, La7,
TV8, Nove, TGR Lombardia e RTL 102.5, inclusi i tentativi ripetuti documentati.
La riproduzione degli altri canali resta da verificare. Le indisponibilità e le
regole di aggiornamento sono in [CATALOG.md](CATALOG.md).

## Ultima verifica UI macOS

| Canale/esecuzione | Avanzamento misurato | DRM verificato | Audio decodificato |
| --- | ---: | --- | ---: |
| Rai 1 | 223,882 s | Non richiesto dalla sorgente osservata | 9.050.091 byte |
| Canale 5 | 77,986 s | Sì | 967.238 byte |
| La7 | 149,933 s | Sì | 1.830.677 byte |
| Canale 5, build finale | 463,810 s | Sì | 5.598.210 byte |

Sono stati provati mini-player, ritorno al player e fullscreen/uscita; le prove
di temi, layout a 820 px e focus hanno usato anche l’anteprima HTTP con gli stessi
asset. L’ultima esecuzione Canale 5 riguarda la build finale delle rifiniture UI.

Le catture del video protetto risultano nere: la conferma visiva sul monitor
resta da completare. L’audio era disattivato: i contatori misurano la decodifica,
mentre l’ascolto resta da verificare. I dati includono i frame persi. Manca una
misura della fluidità della UI rispetto all’obiettivo di 60 fps. I dettagli delle
animazioni sono in [UI_MOTION.md](UI_MOTION.md).

## Primo download Widevine

Una prova Canale 5 con profilo browser inizialmente vuoto ha acquisito Widevine
**4.10.3112.0** tramite Component Updater, senza copia da Chrome, browser esterno
o riavvio prima della riproduzione. La valutazione ha registrato **143,874 s**,
eventi cifrati, chiave utilizzabile e richiesta di licenza HTTP riuscita.
Le precedenti prove inconcludenti restano distinte nella
[documentazione CEF](EMBEDDED.md).

Il bundle verificato contiene **zero file Widevine**. CEF OBS 150.0.19 e il
binding 150.2.1 usano la configurazione descritta nella guida incorporata.
Le condizioni per la distribuzione dei binari richiedono una verifica separata:
vedi [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Test del codice nell’esecuzione UI dell’8 novembre

**66 test Rust/CEF, 25 Node, 18 Python e 9 verifiche DOM in browser** superati;
Clippy con `-D warnings`, formattazione e firma ad hoc verificati. Questi conteggi
si riferiscono a quella esecuzione. Gli strumenti aggiunti in seguito hanno i
propri test; la CI esegue la suite presente nel checkout.

Per riprodurre i controlli usa [CONTRIBUTING.md](../CONTRIBUTING.md). Per nuove
prove live segui [VALIDATION.md](VALIDATION.md), mantenendo distinti resolver,
manifest, decodifica, DRM, immagine visibile e audio udibile.
