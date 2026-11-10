# Stato del progetto e verifiche

Ultimo aggiornamento dei sorgenti: **10 novembre 2026**. Le prove live riportate
qui sono dell’**8 novembre 2026**; la disponibilità delle emittenti può essere
cambiata da allora.

Il [riepilogo JSON pubblico](evidence/2026-11-08.json) raccoglie le metriche
selezionate per la condivisione e i checksum dei file locali da cui provengono.
Non contiene URL delle sorgenti, token, header, profili o dati delle licenze.
I report completi e i log restano locali in `reports/`, esclusi da Git.

## App e piattaforme

| Ambito | Osservato | Da completare |
| --- | --- | --- |
| macOS Apple Silicon | Build, bundle ad hoc, finestra Rust/CEF e catalogo completo; prove Rai/Mediaset/La7. | Conferma visiva diretta dei protetti nell’ultima build, prove prolungate, pacchetto di produzione. |
| macOS Intel | Build nativa, DMG/ZIP e controllo dei metadati dal pacchetto superati in CI. | Finestra, riproduzione, Widevine e collaudo prolungato. |
| Windows | Build nativa e controllo dei metadati superati; confezionamento ZIP in verifica. | Pacchetto, finestra, Widevine e riproduzione desktop. |
| Linux | DEB/tar.gz x64 e controllo dei metadati dal pacchetto superati in CI. Nelle prove storiche: core Docker arm64 e Rai 1 headless. | Finestra CEF desktop, GPU, CDM e riproduzione. |
| Autoaggiornamento app | Non implementato. | Canale di rilascio, download verificato, installazione e recupero dopo errori. |

I controlli del core Rust e delle API CEF sono passati sui tre sistemi.
Il controllo con `cef/dox` verifica le API Rust senza collegare o avviare CEF.

## Pacchetti desktop del 10 novembre

L’[esecuzione nativa del 10 novembre](https://github.com/pjor94/LumeTV-Desktop/actions/runs/42799428013)
ha prodotto DMG e ZIP per Mac Apple Silicon e Intel, DEB e tar.gz per Linux x64
e l’archivio dei sorgenti con le dipendenze Cargo. Windows ha superato compilazione
e controllo dei metadati; il confezionamento ZIP richiede una correzione alle date dei file SDK.
Ogni pacchetto contiene gli avvisi dei componenti, metadati e checksum.
Il controllo automatico esegue il programma per leggere piattaforma, backend,
sandbox e versione API; finestra e riproduzione richiedono prove separate.

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
