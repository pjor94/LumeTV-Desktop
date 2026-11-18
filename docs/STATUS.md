# Stato del progetto e verifiche

Ultimo aggiornamento dei sorgenti: **18 novembre 2026**. Le sezioni successive
conservano anche le prove precedenti. La disponibilità delle emittenti può cambiare.

## EPGShare IT1 per guida e loghi personali, 18 novembre

La precedenza ora è **lista personale → Lume → EPGShare IT1**, separatamente
per guida e logo. Una guida personale dichiarata resta prioritaria anche se
non disponibile. Quando Lume ha programmi per un canale viene mantenuto il suo
palinsesto, senza riempirne singoli vuoti con eventi EPGShare. I due feed pubblici
hanno indici, controlli condizionali e pause indipendenti.

Sulla copia della lista «prova», composta da 157 dirette, Lume conserva i propri
14 palinsesti ed EPGShare ne aggiunge 43: **57 canali con programmi**, tutti
con una trasmissione corrente al momento della verifica, e **58 loghi validi**.
Fra gli Sky riconosciuti ci sono Uno, Atlantic, Serie, Adventure, Cinema Uno,
Sport Uno, Calcio, F1 e Tennis. Sport 253–259, Cinema Illumination e Uno Plus
restano senza associazione/programmi nel campione; non vengono inventate
corrispondenze o trasmissioni.

Il client Rust ha scaricato direttamente il gzip pubblico e creato l’indice
di 195 canali, con ETag e Last-Modified salvati. Sono passati 161 test Rust,
56 scenari browser e 29 test TypeScript; tre prove Rust facoltative e due
acquisizioni browser restano separate. Formattazione, Clippy e build macOS
sono verificati. Risultati e limiti:
[evidenza dell’integrazione](evidence/2026-11-18-epgshare-integration.json).
Il bundle locale aggiornato e firmato è stato aperto sul Mac: nella lista
«prova» la guida di Sky Cinema Collection mostra programmi, descrizioni e
avviso EPGShare IT1. La cache pubblica è stata popolata senza cambiare sorgenti
personali, preferenze o cache Lume.

## Guida e loghi Lume nelle liste personali, 18 novembre

Lume integra i metadati mancanti delle dirette personali usando l’intero feed
EPG, senza limitare la ricerca ai 126 canali del catalogo. Una guida dichiarata
o configurata e un logo dichiarato restano prioritari, anche in caso di errore;
le due regole sono indipendenti. Le associazioni ambigue o incompatibili con
edizioni regionali e varianti «+1» vengono escluse.

La copia pubblica scaricata il 16 novembre contiene 514 definizioni di canale.
Indicizzata con gli intervalli correnti produce 7.555 programmi conservati.
Sulla copia della lista «prova», composta da 157 dirette, le API hanno restituito
programmi per 14 canali e immagini valide per 15. Super Tennis viene riconosciuto
dal nome; Man-Ga ha il logo ma non il palinsesto nel feed disponibile. Il feed
non contiene le guide degli altri canali Sky presenti nella lista. Questi numeri
descrivono il campione disponibile, non una garanzia di copertura futura.

Il servizio pubblico ha risposto 429 con `Retry-After` durante il controllo:
è stata riutilizzata la copia già scaricata, conservandone la data e rispettando
la pausa. Le evidenze non contengono indirizzi o credenziali personali.
Sono passati 156 test Rust, 29 test TypeScript e 56 scenari browser; le tre
prove Rust facoltative e due acquisizioni browser restano escluse. Clippy,
formattazione, build macOS e firma del bundle sono verificati. La build locale
e la cache pubblica sono aggiornate, mantenendo liste, preferenze e riproduzione
in corso; il nuovo codice entra in uso alla prossima apertura dell’app.
Dettagli e verifiche: [evidenza dei metadati condivisi](evidence/2026-11-18-public-guide-fallback.json).

## Indirizzi scaduti e flussi TS personali, 18 novembre

Una prova autorizzata ha isolato indirizzi di canale cambiati dopo l’importazione:
i vecchi manifest di Sky Adventure e Sky Serie rispondevano 200, ma inizializzazioni
e frammenti audio/video restituivano 403. La sorgente remota conteneva nuovi URL,
con gli stessi header e le stesse chiavi. I nuovi percorsi hanno risposto 200.
L’aggiornamento ha portato la lista da 151 a 157 voci, conservando le identità.

Sky Uno è poi risultato verificato nel player CEF già in uso: 33,224 s di
avanzamento, 1.061 frame osservati, 102 richieste media riuscite e zero errori
di rete. ClearKey ha prodotto evento cifrato e chiave utilizzabile. L’udibilità
non è stata verificata e il risultato riguarda questo campione.

Il player ora tenta un solo aggiornamento della sorgente remota quando Shaka
riceve 401/403/404/410 all’avvio, poi riapre il canale; il cambio contenuto annulla
l’attesa. Per i flussi live Xtream TS verifica la rappresentazione HLS dello
stesso servizio. La prova DAZN 1 ha individuato anche un reindirizzamento verso
un IP pubblico esplicito: il trasporto personale ora applica a questi indirizzi
la medesima verifica usata dal DNS, continuando a escludere reti non pubbliche.

Gli altri nove canali TS provati hanno risposto 403 sia al percorso originale
sia alla variante HLS. Anche Caccia ha risposto 403, mentre la catena HLS di
Rai News 24 è arrivata a un frammento video con risposta 200. Questi rifiuti
non vengono classificati come riproduzioni riuscite e non attestano una
compatibilità universale con ogni servizio della lista.

Risultati e limiti: [evidenza della verifica](evidence/2026-11-18-personal-refresh.json).

## Ultimi sviluppi Android, 18 novembre

Sono disponibili backup Android v1 con anteprima e ripristino selettivo,
preferenze persistenti audio/sottotitoli e personalizzazione dell’ordine e dei
canali nascosti. Il ripristino conserva le liste e gli avanzamenti già presenti,
recupera pubblicazioni interrotte e non esporta il token TMDB né le TV associate.
I collegamenti Discovery `localplay` delle liste riusano il resolver del catalogo;
formato e DRM del provider prevalgono sui suggerimenti statici precedenti.

La home Video aggiunge anteprima del titolo e categorie caricate a richiesta.
Il riconoscimento prudente/esteso e le regole per gruppo si applicano anche ai
cataloghi già salvati. TMDB migliora il confronto Unicode e conserva i metadati
dichiarati dalla lista, con regia/creatori e indicazione delle trame inglesi.

Il companion TV comprende ricerca in LAN, associazione con codice o invito QR,
telecomando, verifica/avvio remoto e trasferimento esplicito di una lista.
Il collegamento usa TLS con pin e prove HMAC compatibili con Android. I test
confrontano vettori indipendenti e impediscono l’invio delle credenziali quando
la chiave della TV cambia. Non è stata associata una TV fisica in questa sessione.

Sono passati 146 test Rust ordinari, 52 scenari browser e 29 test TypeScript.
Le tre prove Rust facoltative restano separate: la capienza VOD è stata ripetuta
esplicitamente su 500.000 titoli, con 169,41 s per importazione/indice, 6,202 s
per ultima pagina, home e ricerca e 853.987.328 byte per snapshot.

La prova nativa CEF su profilo temporaneo ha mostrato home VOD, preferenze
Android ripristinate, salvataggio di un backup da 3.227 byte e riapertura dello
stesso con anteprima e ripristino senza duplicare le liste. Il trailer pubblico
Sintel ha avanzato di 45,986 s, con 1.154 frame osservati e 771.437 byte audio
decodificati. L’udibilità non è stata confermata manualmente. I dati sanitizzati
sono nell’[evidenza di compatibilità](evidence/2026-11-18-android-parity.json).
La [guida alle liste](PERSONAL_PLAYLISTS.md#compatibilità-android-backup-riconoscimento-e-tv)
descrive utilizzo e limiti. Queste verifiche non dimostrano identica compatibilità
fra CEF e Media3 per ogni provider, codec o contenitore.

## Libreria VOD da Android, 17 novembre

Il desktop dispone di libreria Film/Serie, stagioni ed episodi, Continua a
guardare, La mia lista e aggiunti di recente. Schede, filtri, ordinamento e
ricerca lavorano sull’intera sorgente; la griglia conserva tre finestre di dati.
Sono disponibili episodio successivo esplicito, visto/non visto e rimozione da
Continua a guardare. Le scritture di una sessione superata non annullano le
decisioni manuali. La [guida alle liste](PERSONAL_PLAYLISTS.md#libreria-video)
descrive l’uso e i dati riconosciuti.

TMDB è facoltativo, configurabile nel profilo e disattivato inizialmente.
L’integrazione usa un trasporto dedicato, associazioni manuali e cache offline.
La prova autorizzata con un titolo pubblico ha verificato ricerca, trama,
cast, locandina reale da 140.167 byte e riuso dell’immagine dalla cache.

Sono passati 127 test Rust ordinari, 47 scenari browser e 29 test TypeScript,
oltre a Clippy. I browser test usano la stessa Content Security Policy dell’app:
le locandine sono rese come dati immagine consentiti, con caricamento visibile,
richieste condivise e cache limitata. Sono coperti entrambi i temi a 820, 1280 e
1680 pixel, oltre a ripresa, stagioni, completamento e scelta manuale TMDB.

La prova su disco con 500.000 VOD è passata: importazione e indice in 181,37 s,
snapshot di 853.889.024 byte (circa 814 MiB). Ultima pagina, raccolte home e
ricerca dell’ultimo titolo hanno richiesto complessivamente 5,557 s. La prima
versione dell’indice superava il limite di un GiB: la versione consegnata legge
i dettagli dalle righe originali e costruisce le raccolte con aggregazione SQL,
evitando la duplicazione dei metadati e gli indici ridondanti. La prova da un
milione di voci TV resta quella precedente; non viene estesa ai VOD per inferenza.

La prova macOS/CEF ha mostrato locandina e sfondo TMDB, trama, cast, stagioni e
filtro della seconda stagione. Il trailer pubblico W3C di Sintel ha prodotto
49,985 s di avanzamento, 1.253 frame video osservati e 822.671 byte audio
decodificati; l’udibilità non è stata confermata manualmente. Il servizio di prova
rifiuta lo User-Agent predefinito e accetta l’override `LumeTV`, configurato solo
nella sorgente temporanea. Il completamento è tornato nel dettaglio come Riguarda.
Questa prova non estende la compatibilità a qualsiasi provider o contenitore.
Dati sanitizzati: [evidenza VOD](evidence/2026-11-17-vod.json).

## Home delle liste personali, 16 novembre

La home personale riprende la struttura del Catalogo Lume: canale in evidenza,
visti di recente, titolo della raccolta e griglia. Riquadro principale e recenti
sono componenti condivisi. Il collegamento alla guida conserva lista e canale;
i nomi sostitutivi dei loghi si adattano allo spazio e usano iniziali nelle
versioni compatte. La struttura resta utilizzabile anche senza programmi EPG.

Il backend seleziona solo TV live delle sorgenti abilitate. La cronologia viene
limitata a 40 ID e ordinata per restituire cinque recenti; il primo viene proposto
in evidenza, con il primo canale come alternativa quando manca la cronologia.
Le risposte obsolete non aggiornano una lista diversa. Home e griglia sospendono
gli aggiornamenti quando il player occupa la pagina e conservano la posizione
anche tornando rapidamente alla home.

Sono passati 120 test Rust ordinari, 39 scenari browser e 29 test TypeScript,
oltre a Clippy senza avvisi e ai controlli degli asset. Il cambio rapido tra
player e griglia è stato ripetuto tre volte su 500.000 canali simulati. Le prove
visive coprono entrambi i temi a 820, 1280 e 1680 pixel, con programmi e recenti.
Il pacchetto CEF macOS è stato aggiornato e poi riavviato su richiesta dell'utente.
La nuova home è stata verificata anche nella finestra nativa sulla lista personale
già importata: canale in evidenza, cinque recenti, accessi alla guida e griglia.
L'app è rimasta aperta su questa schermata.

## Vista delle liste personali, 16 novembre

Le liste personali TV e Video ora usano le stesse schede del Catalogo Lume;
le radio personali riusano quelle del catalogo Radio. Nome, logo, gruppo,
programma in onda, avanzamento e preferiti condividono il componente.
La griglia legge le dimensioni dalle regole CSS esistenti e scorre con la
pagina, conservando il caricamento di tre finestre di dati. Restano compatti
i selettori e il browser dei canali durante la visione.

Sono passati **37 scenari browser e 29 test TypeScript**. La prova con 500.000
canali simulati raggiunge l'ultimo elemento, mantiene meno di 80 schede nel DOM
e ripristina la posizione tornando dal player e dalle Impostazioni. Sono coperti
ricerca, gruppi, preferiti, frecce per colonna, Home/Fine e ridimensionamento.
Il confronto visivo è stato eseguito a 820, 1280 e 1680 pixel, in entrambi i temi.
La prova di capienza SQLite non è stata ripetuta: questo intervento riguarda la UI.
La build CEF macOS è stata aggiornata e controllata sulla lista personale già
importata nel profilo esistente. L'app è rimasta aperta sulla nuova griglia.

## Guida e loghi del Catalogo Lume, 16 novembre

Il profilo segnalato conservava una guida vuota dopo una risposta di limitazione
del servizio. Le richieste desktop usavano uno User-Agent diverso da Android;
ora guida e immagini usano le rispettive identità Android. Questo allineamento
non dimostra da solo la causa della limitazione ricevuta dal servizio.
Il frontend controlla la guida alla scadenza indicata dal server, senza aggiungere
fino a cinque minuti di attesa; il backend conserva la pausa anche dopo un riavvio.

Il desktop inoltre ignorava i loghi della lista ufficiale fuori dal dominio EPG,
accettava solo PNG e non comunicava al frontend il logo Rai 3 condiviso dalle
edizioni regionali. Ora prova il logo della lista, quello EPG e l'eventuale logo
regionale condiviso. Sono supportati PNG, JPEG, GIF e WebP; il testo sostitutivo
si nasconde quando l'immagine è caricata. Le destinazioni pubbliche, i redirect,
gli indirizzi DNS e il limite di un MiB sono controllati prima di usare i dati.
L'alias osservato `Rai3TGR.it` è associato al solo Veneto, come su Android;
i programmi delle altre edizioni restano distinti.

Un download diagnostico del feed pubblico è riuscito. La lettura XZ/XMLTV reale
con il parser aggiornato produce **4.075 programmi per 83 canali e 84 loghi EPG**.
La guida recuperata da quel download è stata installata nel profilo segnalato,
verificando gli ID del catalogo e conservando backup, data del download e scadenza
della pausa precedente. Questa operazione non costituisce una nuova prova
dell'aggiornamento automatico dal servizio dopo la pausa.

Nell'app CEF macOS aggiornata, l'API espone gli stessi programmi e **110 candidati
logo** includendo lista e riuso regionale. Rai 1, Canale 5, Italia 1 e Rete 4
restituiscono immagini PNG reali con HTTP 200 e dimensioni valide; la disponibilità
dei 110 candidati non equivale al download verificato di ogni immagine.
Il catalogo mostra titoli in onda e loghi; la griglia TV mostra programmi,
orari e immagini anche per i tre canali Mediaset principali. I dati della prova sono in
[questo riepilogo](evidence/2026-11-16-builtin-guide.json), senza dati del profilo.

Sono passati 119 test Rust ordinari, Clippy senza avvisi e 34 scenari browser.
I test comprendono la richiesta XZ con l'identità Android, il recupero dopo una
pausa, i loghi indipendenti dalla guida, il ripiego dopo un'immagine non disponibile
e i vincoli sui redirect. La prova di capienza da un milione di voci resta separata.

## Correzione ClearKey del 16 novembre

Il successivo controllo dei canali Mediaset ha rilevato un secondo errore nel
cambio di sessione: Shaka unisce `drm.clearKeys` e `drm.servers`, quindi `{}`
conservava le chiavi e il sistema precedenti. Il player ora ripristina entrambe
le mappe con `undefined` prima di applicare la configurazione del nuovo canale.
Il test reale legge anche `getConfiguration()` dopo il passaggio ClearKey →
Widevine: prima falliva mostrando chiavi e server rimasti, ora passa. I soli
controlli delle chiamate al player simulato non rilevavano questa differenza.

Il collaudo nella finestra CEF ha mantenuto il player aperto, passando dalla
lista ClearKey a Rai 4 e poi ai tre canali Mediaset principali:

| Canale | Avanzamento misurato | Frame decodificati nell’intervallo | Scambi licenza riusciti |
| --- | --- | --- | --- |
| Canale 5 | 37,978 s | 389 | 2 |
| Italia 1 | 53,961 s | 426 | 2 |
| Rete 4 | 57,969 s | 389 | 2 |

Tutti hanno prodotto evento cifrato, chiave utilizzabile, audio decodificato e
zero errori di rete. La prova attesta avanzamento e decodifica; l’udibilità e
la visione dell’immagine protetta sul monitor non sono attestate dalle metriche.
Il [riepilogo delle prove](evidence/2026-11-16-drm-switch.json) conserva soltanto
metriche e hash dei sorgenti, senza indirizzi, dati del profilo o credenziali.

Una lista importata ha evidenziato un errore non coperto dai precedenti test con
player simulato: il canale dichiarava chiavi ClearKey nella lista, mentre il
manifest esponeva solo altri sistemi DRM. Configurare il solo server licenze
ClearKey non forzava Shaka a usarlo. Le chiavi esplicite ora sono applicate alla
sessione tramite `drm.clearKeys`, come previsto dalla
[configurazione Shaka](https://shaka-project.github.io/shaka-player/docs/api/tutorial-drm-config.html).
Il cambio contenuto azzera le chiavi precedenti; catalogo e report non le espongono.

Il nuovo test `clearkey.spec.ts` usa Shaka reale e un piccolo video CENC generato
con chiavi sintetiche: falliva prima della correzione e ora osserva avanzamento
e frame decodificati. Le altre prove di interazione restano simulate. È stata
corretta anche la duplicazione dello User-Agent, verificata con un server locale;
nel manifest segnalato entrambe le varianti HTTP rispondevano 200.

Nella build CEF macOS arm64 aggiornata, Sky Serie della lista personale ha
prodotto 57,983 secondi di avanzamento, 1.396 frame decodificati nell’intervallo
e 968.233 byte audio decodificati. Sono presenti eventi encrypted e chiave usable,
con 55 richieste media riuscite e zero errori di rete. Udibilità e immagine sul
monitor non sono attestate dalle metriche o dalla cattura del video protetto.
La lista completa non è stata collaudata. Gli indirizzi e i dati di accesso
sono esclusi da questo riepilogo.

Le chiavi ClearKey locali non richiedono una licenza HTTP. La diagnostica ora
distingue questa evidenza, derivata dalla configurazione server della sessione,
dallo scambio HTTP usato dai sistemi con server licenze. Entrambi richiedono
tempo, frame, evento cifrato e chiave utilizzabile; i controlli Widevine restano
invariati. Il report della prima misura sopra precedeva questa distinzione e
riportava una classificazione incompleta per l’assenza dello scambio HTTP.

Sono passati 113 test Rust ordinari, 33 scenari browser (inclusa la decodifica
ClearKey reale), 29 unitari TypeScript e 66 Python. La prova di capienza resta
separata e non è stata ripetuta per questa modifica. Le due catture facoltative
del precedente catalogo non fanno parte dei 33 scenari. Il bundle CEF macOS è
stato aggiornato con la correzione.

## Liste personali del 13 novembre

Il porting introduce liste M3U da URL/file, lettura progressiva su SQLite,
configurazioni Kodi/OTT, header e cookie per risorsa, Widevine/ClearKey,
guida XMLTV personale e archivio. Video e archivio hanno ripresa e avanzamento
separati dalla cronologia live. L’interfaccia usa un solo player persistente.
Sono escluse le funzioni Android solo audio, risparmio dati e controlli di sistema.
Flussi e limiti sono documentati in [PERSONAL_PLAYLISTS.md](PERSONAL_PLAYLISTS.md).

La prova esplicita su due liste da 500.000 voci ha verificato un milione di
elementi su disco, l’ultima finestra a offset 999.800 e il rifiuto del superamento
del limite senza cambiare le sorgenti valide. Si è conclusa in circa 236 secondi
con build Rust di sviluppo. Le prove browser verificano 500.000 righe con DOM
limitato, navigazione da tastiera e ripristino della posizione.

Sono passati 111 test Rust ordinari (compreso il percorso HTTP reale di
importazione, anteprima, ripresa VOD ed eliminazione), 29 unitari TypeScript,
15 del relay JavaScript e 66 Python. Il test di capienza è escluso dalla suite
ordinaria ed è stato eseguito separatamente. Sono passati 32 scenari browser;
le due catture facoltative delle schermate precedenti sono escluse, mentre le
catture delle nuove schermate sono incluse. La UI è stata controllata nei due
temi a 820 e 1280 pixel. Formattazione, Clippy, controllo degli asset incorporati
e verifica del repository sono passati, insieme ai controlli API CEF/dox del
binario e della DLL Windows. La build locale macOS arm64 con SDK CEF verificato
è stata prodotta; questi test usano sorgenti sintetiche e player browser simulato.

La compatibilità effettiva di codec, DRM e archivi dei singoli servizi richiede
prove di riproduzione nel runtime nativo. Per le nuove sorgenti personali non
sono state effettuate prove live, né collaudi nativi Windows, Linux o Mac Intel.
I risultati dei provider riportati sotto appartengono alle precedenti build.

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
