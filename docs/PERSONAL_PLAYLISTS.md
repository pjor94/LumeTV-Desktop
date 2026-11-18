# Liste personali sul desktop

Da **Impostazioni → Le mie liste → Gestisci liste** puoi aggiungere una lista
M3U tramite indirizzo oppure file. La verifica prepara un’anteprima con conteggi,
esclusioni e un piccolo campione; **Importa lista** la rende disponibile.
Puoi annullare l’operazione, rinominare, disattivare, sostituire o eliminare una
sorgente. Gli aggiornamenti falliti conservano l’ultima copia valida.

Il selettore in **Canali**, **Radio** e **Guida TV** distingue le liste personali
dal catalogo Lume. **Video** è disponibile per le liste TV personali, anche
quando non contengono ancora VOD.
Ricerca e filtri interrogano tutto il catalogo. Posizione e filtri restano
separati fra TV, radio e video. Frecce, Pagina su/giù, Inizio e Fine spostano
il focus nella lista.

## Lettura della lista

Sono supportate liste semplici o estese, UTF-8, Windows-1252 e ISO-8859-1,
indirizzi relativi nelle sorgenti URL, liste incluse e gruppi nei profili
predefinito, Kodi e OTT. La modalità automatica distingue dirette e VOD;
la sorgente può anche forzare il tipo di contenuto e l’uso come radio.
I manifest HLS con segmenti non si importano come cataloghi M3U.

Gli identificativi vengono conservati durante la sostituzione tramite
corrispondenze univoche su tipo e `tvg-id`, indirizzo o nome e gruppi.
Le voci temporaneamente scomparse mantengono una piccola cronologia degli ID.
Preferiti e riprese non dipendono dalla posizione dell’elemento nella lista.

Le sorgenti URL si aggiornano secondo l’intervallo configurato, la direttiva
`refresh` o, in assenza di entrambe, ogni 24 ore mentre l’app è aperta.
ETag, Last-Modified e Retry-After evitano download inutili e rispettano le pause
richieste dal servizio. Le sorgenti da file si aggiornano sostituendo il file.

## Accesso ai server e riproduzione

Gli header di lista (`EXTHTTP`, `EXTVLCOPT`, `KODIPROP` e suffissi URL) possono
essere integrati o sostituiti dalle impostazioni della sorgente. Playlist,
manifest, segmenti, licenze, chiavi HLS, loghi e guida hanno configurazioni
distinte. Gli eventuali cookie restano in memoria, separati per sorgente e ruolo.
Un cambio di origine rimuove gli header di accesso, salvo autorizzazione esplicita
del server per quel ruolo. Non viene disattivata la verifica TLS.

HTTP richiede un consenso per la lista; loghi e guida hanno consensi separati.
Sono accettati server pubblici HTTP/HTTPS. Indirizzi locali, credenziali inserite
nell’URL e schemi eseguibili non sono accettati. Questi vincoli riguardano solo
le sorgenti personali; le integrazioni del catalogo Lume restano dedicate.
Gli IP espliciti delle CDN passano la stessa verifica degli indirizzi risolti
dal DNS: loopback, reti private, link-local e indirizzi riservati restano esclusi.

HLS e DASH usano Shaka. MP4, WebM e audio progressivo passano da un relay locale
con lettura a flusso e supporto Range. La decodifica dipende dai codec e dai
contenitori del runtime CEF: la compatibilità di Media3 su Android non implica
la stessa compatibilità sul desktop.

Per i percorsi live Xtream `/live/utente/password/id.ts`, il desktop verifica
se lo stesso servizio offre la rappresentazione `.m3u8`. La usa soltanto quando
riceve una playlist HLS valida; indirizzo salvato, identità del canale e regole
degli header restano invariati. Non è una conversione universale dei flussi TS.

Se Shaka riceve HTTP 401, 403, 404 o 410 durante l’avvio, una sorgente remota
non appena controllata viene aggiornata una sola volta per quel tentativo di
visione, quindi viene riaperto lo stesso canale. I file locali non sono aggiornati
in questo modo. Un cambio di contenuto annulla l’attesa e non riapre il precedente;
un aggiornamento fallito lascia disponibili i normali tentativi di riconnessione.

Sono tradotte configurazioni Kodi Widevine e ClearKey, chiavi ClearKey locali
in formato KID/KEY o JWK, header e parametri delle chiavi HLS AES-128 e i modelli
di richiesta/risposta licenza riconosciuti dal parser. ClearKey richiede DASH
CENC. Configurazioni DRM non supportate vengono escluse con una diagnosi;
non si eseguono script o componenti esterni. Il progetto non incorpora un CDM.

Le chiavi ClearKey locali vengono fornite alla sola sessione autenticata del
player e impostate in `drm.clearKeys`, così la scelta esplicita della lista
prevale anche su un MPD che annuncia Widevine/PlayReady. Non compaiono nel
catalogo o nella diagnostica. Cambiare contenuto azzera questa configurazione.

Video e archivio hanno ripresa, riavvio e avanzamento separati dalla cronologia
delle dirette. Alla fine il video si ferma senza attivare la riconnessione live.
Catalogo, player principale e mini-player condividono un solo elemento video.
Arresto, cambio contenuto e disattivazione della sorgente annullano le richieste
precedenti. Modificare solo la guida permette alla riproduzione di proseguire.

## Libreria Video

La home raccoglie **Continua a guardare**, **La mia lista**, **Film**, **Serie**
e **Aggiunti di recente**. Le serie occupano una sola locandina; aprendo il titolo
trovi stagioni, episodi e il contenuto da riprendere. Il comando **Prossimo
episodio** resta disponibile nel player. Il passaggio è esplicito e non avvia
automaticamente un altro contenuto a fine visione.

Sono riconosciuti attributi come `series-title`, `series-id`, `season`, `episode`,
`year`, `plot`, `genre`, `poster` e `backdrop`, oltre ai titoli `S01E02` e `1x02`.
La classificazione rispetta `type="live"`; tipi film/serie, indicazioni media,
durata e percorsi provider VOD riconosciuti individuano i video. Se la lista non
fornisce indicazioni sufficienti, il tipo della sorgente può essere forzato
nelle sue impostazioni. Le modifiche al riconoscimento si applicano alla lettura
o all’aggiornamento della lista; l’indice di video già riconosciuti viene migrato
al primo accesso alla libreria.

Ricerca, gruppo e ordinamento interrogano tutta la sorgente. Le schede estese
leggono solo la finestra richiesta, fino a 200 titoli; il frontend conserva tre
finestre e virtualizza la griglia. Identità, data della prima aggiunta, decisioni
di visione e riprese vengono conservate quando la lista si aggiorna.

Dal dettaglio puoi salvare un film o un’intera serie, segnare un episodio come
visto/non visto e nasconderlo da Continua a guardare. Una riproduzione avviata
di nuovo ripristina la visibilità. Le scritture di un player già superato da una
decisione manuale vengono ignorate. Cronologia e salvati contengono identificativi
locali, non indirizzi o credenziali dei flussi.

**Video → Informazioni online** permette di configurare il Read Access Token
TMDB e attivare trama, cast e immagini. L’opzione è inizialmente disattivata.
In alternativa il processo può ricevere `LUME_TMDB_READ_TOKEN` dall’ambiente.
Solo il token inserito nell’app viene salvato nel profilo privato, con permessi
0600 sui sistemi Unix. Le API restituiscono lo stato della configurazione.

L’apertura del dettaglio cerca il titolo e l’anno, oppure gli identificativi
TMDB/IMDb dichiarati. Solo corrispondenze univoche vengono accettate automaticamente;
in caso di dubbio puoi scegliere il titolo, cercarne un altro oppure usare solo
le informazioni della lista. Una trama già scaricata è leggibile offline mentre
l’opzione resta attiva; disattivandola tornano i dati locali. Svuotare la cache
conserva le associazioni manuali. Il client TMDB non riceve URL, header o cookie
delle sorgenti; le locandine restano soggette a limiti di formato e dimensione.

La funzione usa l’[autenticazione applicazione TMDB](https://developer.themoviedb.org/docs/authentication-application).
This product uses the TMDB API but is not endorsed or certified by TMDB.

## Guida e archivio

Se la lista non dichiara una guida e non è configurato un indirizzo manuale,
Lume integra automaticamente la propria EPG. La ricerca usa il **feed XMLTV
completo**, comprese le emittenti che non compaiono nel Catalogo Lume.
L’associazione cerca l’ID esatto, l’ID normalizzato e infine un nome univoco,
ignorando accenti, simboli e suffissi di qualità. Corrispondenze ambigue,
regioni incompatibili e varianti «+1» diverse vengono escluse.

Una guida personale dichiarata resta prioritaria anche se disabilitata,
irraggiungibile o senza programmi per un canale. Non vengono riempiti i suoi
vuoti con un’altra EPG. L’esclusione manuale di un canale viene rispettata.
Il logo segue una regola indipendente: se la lista lo dichiara viene mantenuto,
anche quando non si carica; solo quando manca vengono usati il logo del
Catalogo Lume o quello del feed completo. URL, ID, gruppi e accesso al flusso
personale rimangono quelli originali.

**EPGShare IT1** è la terza fonte automatica: viene consultata solo dopo i dati
personali e quelli Lume. Per ciascun canale viene mantenuto il palinsesto Lume
se contiene programmi nell’intervallo conservato; altrimenti viene selezionato
quello EPGShare. Le ricerche e i giorni senza risultati non mescolano eventi
delle due guide. Un logo Lume disponibile rimane prioritario anche se il suo
download fallisce; EPGShare viene usata quando manca il riferimento al logo.
Guida e logo vengono scelti indipendentemente.

Il file pubblico è
[`epg_ripper_IT1.xml.gz`](https://epgshare01.online/epgshare01/epg_ripper_IT1.xml.gz).
L’indice `epgshare-guide.sqlite` e lo stato `epgshare-guide.json` sono separati
dal feed Lume. Il controllo avviene ogni quattro ore, con ETag/Last-Modified,
rispetto di Retry-After e conservazione della copia valida in caso di errore.
Un documento invariato viene comunque riletto dopo dodici ore per spostare
la finestra dei programmi. Una pausa del servizio Lume non blocca EPGShare.
La provenienza dei programmi è esposta come `guide_origin: epgshare`; stato
della sorgente e pagina Guida TV indicano la presenza dell’integrazione.

La guida integrata alimenta home, schede, player e pagina Guida TV. La pagina
indica la provenienza Lume; l’aggiornamento è condiviso con il catalogo e
rispetta la scadenza richiesta dal servizio. I loghi pubblici usano un trasporto
separato, senza header o cookie delle liste. I programmi pubblici non abilitano
da soli archivio o «Guarda dall’inizio».

La sezione **Guida e archivio** configura URL XMLTV (anche gzip o XZ), lingua,
fuso, correzione oraria, storico da uno a sette giorni e futuro fino a 48 ore.
L’associazione cerca prima `tvg-id`, poi un nome univoco. Il selettore manuale
permette di scegliere una definizione XMLTV o escludere un canale.
La guida si aggiorna ogni sei ore e mantiene la copia valida in caso di errore.

La guida personale offre scelta canale e giorno, ricerca nello storico,
descrizioni, promemoria e azioni di riproduzione. Ora e programma successivo
compaiono anche nel catalogo e nel player, quando disponibili.

L’archivio richiede indicazioni catch-up nella lista e programmi XMLTV validi.
Sono riconosciuti modelli default/append/shift, Flussonic, Xtream e catchup-id.
Disponibilità, giorni conservati, fuso e margini determinano le azioni proposte.
Il contenuto in corso può offrire **Guarda dall’inizio** se abilitato.
Il player mostra un comando esplicito **Torna alla diretta**. Lume non registra
le trasmissioni: l’archivio è fornito dalla sorgente.

## Memoria, dati locali e limiti

Il backend conserva un database SQLite per lista e uno per guida nel profilo
privato `personal/`. Cataloghi e XML vengono letti progressivamente; la UI
mantiene al massimo tre finestre di 200 elementi (50 per guida e selettori)
e rende soltanto gli elementi vicini alla posizione visibile. Canali e video
usano le stesse schede del Catalogo Lume, con logo, gruppo, programma in onda
e preferiti; le radio riusano le schede del catalogo Radio. La griglia segue
lo scorrimento della pagina e si adatta alla larghezza della finestra.
La posizione viene conservata tornando dal player o da un'altra sezione.
Le frecce seguono le colonne; Home/Fine raggiungono gli estremi della lista.

La home TV mostra un canale in evidenza e fino a cinque visti di recente della
lista selezionata. Il più recente viene proposto nel riquadro principale; senza
cronologia viene proposto il primo canale. Programma, descrizione e orario
compaiono quando disponibili nella guida. **Apri la guida** porta alla stessa
lista e preseleleziona il canale in evidenza.

| Risorsa | Limite |
| --- | --- |
| Sorgenti | 10 |
| Lista in ingresso | 256 MiB e 500.000 voci esaminate per sorgente |
| Catalogo complessivo | 1.000.000 di voci attive, 2 GiB di database |
| Liste incluse | 16 documenti, profondità 3 |
| Guide | 100.000 definizioni e 1.000.000 di programmi per sorgente |
| XMLTV | 32 MiB compressi, 256 MiB decodificati |
| Cache guida | 128 MiB per sorgente, 256 MiB complessivi |
| EPG Lume condivisa | 20.000 canali, 1.000.000 programmi, 192 per canale; indice SQLite fino a 256 MiB |
| EPGShare IT1 | Stessi limiti dell’indice pubblico, in un database separato; gzip fino a 32 MiB, XML fino a 350 MiB |
| Feed EPG Lume | 32 MiB compressi, 350 MiB decodificati; programmi da 6 ore prima a 48 ore dopo |
| Loghi Lume condivisi | Cache fino a 64 MiB, immagini fino a 1 MiB |
| Loghi | PNG/JPEG/WebP, 1 MiB per immagine, 3 download contemporanei |
| Riprese | 5.000 contenuti |

URL, header, cookie e chiavi non entrano nelle righe pubbliche del catalogo o
nei report diagnostici. Le impostazioni locali contengono i dati necessari per
riaprire le sorgenti: non condividere il profilo o i suoi database.

## Verifica

I test Rust coprono importazione, sostituzione atomica, recupero dopo interruzione,
identità, privacy dei metadati, richieste e annullamento, DRM, XMLTV e archivio.
Il test HTTP desktop percorre verifica, anteprima, importazione, preferiti,
ripresa VOD ed eliminazione senza richieste ai provider.

I test browser usano fixture e verificano un catalogo di 500.000 righe,
tastiera, filtri, moduli, focus, player persistente e archivio. La prova capienza
su SQLite usa due liste reali da 500.000 voci e verifica il limite complessivo:

```sh
cargo test --locked personal::tests::million_entry_library_keeps_queries_bounded -- --ignored --nocapture
```

`clearkey.spec.ts` usa il vero Shaka e un video CENC sintetico: verifica frame
decodificati e avanzamento, senza un player simulato e senza contattare provider.
Le altre prove di interazione continuano a usare il player simulato.

Per compatibilità effettiva di codec, licenze e archivi di singoli servizi
servono prove nel runtime nativo. Le funzioni Android solo audio, risparmio dati
e controlli di sistema sono escluse da questo porting.


## Compatibilità Android: backup, riconoscimento e TV

In **Lettura e contenuti** puoi scegliere il riconoscimento VOD prudente oppure
esteso e fino a 100 regole per gruppo. Le regole precedono le dichiarazioni delle
singole voci; se due gruppi hanno regole contrastanti prevalgono le dirette.
La modalità estesa riconosce file video nei gruppi Film/Serie/VOD, escludendo
percorsi live e voci con catch-up. Salvando le opzioni si riclassificano anche
le voci già presenti, senza interrogare gli indirizzi degli stream.

I collegamenti Discovery `localplay` riconosciuti da Android sono importati e
conservati anche nei backup. Durante la riproduzione usano il resolver del
Catalogo Lume: formato e protezione aggiornati prevalgono sui vecchi suggerimenti
statici della lista, mentre restano applicabili gli header configurati.

La home Video mostra il titolo a fuoco e le raccolte per categoria. Le categorie
vengono lette in pagine; i titoli sono richiesti quando la raccolta entra nella
finestra. TMDB conserva trama, generi e durata dichiarati dalla lista, riconosce
`tmdb-type=movie|tv` sui video generici e mostra regia/creatori. L’anno di una
serie si riferisce alla prima messa in onda, mentre l’eventuale trama inglese
è indicata come tale. L’opzione online resta inizialmente disattivata.

**Impostazioni → Backup e ripristino** crea e apre file `.lumebackup` compatibili
con Android. La password deve contenere da 8 a 128 caratteri. Il file contiene
anche indirizzi e credenziali delle liste: il contenitore v1 usa PBKDF2-HMAC-SHA256
con 600.000 iterazioni, sale casuale di 16 byte e frame AES-256-GCM di 256 KiB.
Header, indice e dimensione sono autenticati; un frame finale vuoto autenticato
rende rilevabile il troncamento. Le credenziali TMDB e le associazioni alle TV
restano nel profilo del dispositivo e non vengono esportate.

La verifica prepara tutte le liste prima di consentire il ripristino. Si possono
scegliere liste, personalizzazioni, avanzamenti e preferenze. Le liste già presenti
con lo stesso identificativo restano nella versione locale; gli avanzamenti
esistenti e le associazioni manuali non vengono sovrascritti. Il riepilogo indica
i riferimenti ignorati, anche quando il relativo contenuto non è presente.
Ordine e canali nascosti vengono applicati alle viste; ogni scheda canale offre
Personalizza, e i canali nascosti sono ripristinabili dalle impostazioni.

La decifratura e l’espansione sono limitate a 2 GiB, il manifest a 16 MiB e ogni
riga JSONL a 2 MiB. Percorsi estranei, duplicati e collegamenti negli archivi sono
rifiutati. Le copie temporanee hanno permessi privati e vengono eliminate dopo
annullamento, errore o chiusura. Un registro di ripristino con marcatore SQLite
permette di recuperare la pubblicazione interrotta all’avvio successivo.

**Impostazioni → La tua TV** collega Lume a una TV con il companion Android
abilitato in **Collega telefono**. Si può usare la ricerca locale oppure
l’indirizzo IPv4, la porta e il codice di 16 caratteri mostrati sulla TV; è
supportato anche il testo di un invito QR `LUME1`. Il codice manuale richiede
l’approvazione sulla TV. Le richieste usano TLS con pin della chiave pubblica
e prove HMAC compatibili con Android. Il trasporto LAN è separato dalle politiche
di rete delle sorgenti e accetta solo indirizzi IPv4 privati numerici.

**Guarda sulla TV**, nel player e nelle schede VOD, verifica il contenuto prima
di avviarlo. I riferimenti contengono hash e identificativi del catalogo,
non URL o credenziali degli stream. Per le liste personali la TV deve avere
la stessa sorgente e gli stessi dati di accesso. Il trasferimento di una lista
è un’azione esplicita con scelta della TV e consenso all’invio delle credenziali;
una seconda conferma importa la lista preparata sul dispositivo ricevente.
Il trasferimento usa JSONL/GZIP, chunk da 16 KiB, hash SHA-256 e i limiti Android
di 256 MiB compressi e 1 GiB espanso.

Il telecomando espone navigazione, play/pausa/stop, cambio canale, seek,
ritorno alla diretta, episodio successivo e selezione delle tracce/qualità.
I comandi sono legati alla sessione e alla revisione delle opzioni della TV.
Il video locale viene fermato soltanto dopo la conferma di riproduzione remota.
Il protocollo è coperto da vettori e prove TLS locali; il collaudo su una TV
fisica associata resta distinto dalle prove automatiche.
