# Interfaccia desktop

La UI è una piccola applicazione React e TypeScript, compilata con Vite e
incorporata nel binario Rust. CEF la visualizza nella finestra dell’app.
CSS, icone e animazioni derivano dall’interfaccia precedente.

## Dove intervenire

| File o cartella | Responsabilità |
| --- | --- |
| [AppShell.tsx](../web/app/src/AppShell.tsx) | Navigazione, layout, tema e scorciatoie |
| [pages](../web/app/src/pages) | Canali, scoperta, guida, ricerca e impostazioni |
| [components](../web/app/src/components) | Player, dialoghi e componenti condivisi |
| [CatalogHome.tsx](../web/app/src/components/CatalogHome.tsx) | Canale in evidenza e visti di recente dei cataloghi TV |
| [app.ts](../web/app/src/app.ts) | Catalogo, guida, preferenze, promemoria e stato delle schermate |
| [playback.ts](../web/app/src/playback.ts) | Shaka, sessioni, cambio canale, tracce e diagnostica |
| [recovery.ts](../web/app/src/recovery.ts) | Tentativi di riconnessione, attesa della rete e annullamento |
| [pages/Radio.tsx](../web/app/src/pages/Radio.tsx) | Catalogo radio, ricerca, preferite, recenti e paginazione |
| [personal.ts](../web/app/src/personal.ts) | Sorgenti personali, selezioni e finestre di dati con memoria limitata |
| [ContinuousList.tsx](../web/app/src/components/ContinuousList.tsx) | Liste e griglie virtuali, scorrimento, ripristino e tastiera |
| [pages/Playlists.tsx](../web/app/src/pages/Playlists.tsx) | Importazione e impostazioni per sorgente |
| [pages/PersonalGuide.tsx](../web/app/src/pages/PersonalGuide.tsx) | XMLTV personale, storico e azioni archivio |
| [api.ts](../web/app/src/api.ts) | API locale autenticata e coda dei salvataggi |
| [widevine.ts](../web/app/src/widevine.ts) | Preparazione del componente e verifica EME |
| [updates.ts](../web/app/src/updates.ts) | Stato e controlli degli aggiornamenti dell’app |
| [motion.js](../web/app/src/motion.js) | Animazioni delle superfici e del mini-player |

Il player ha uno stato separato dalle pagine. Navigazione, ricerca e aggiornamenti
della guida non devono ricreare il video o richiamare `player.load()`.
Il mini-player usa lo stesso elemento `<video>`: cambia la posizione con CSS e
una trasformazione compensata, mantenendo le proporzioni dell’immagine.

I salvataggi di preferenze e cronologia passano dalla stessa coda. Rust conserva
`preferences.json`; la UI non mantiene una seconda copia persistente.
Il frammento iniziale dell’URL contiene il token della sessione locale e viene
rimosso all’avvio. La navigazione interna non usa il frammento come router.

TV e radio condividono il player persistente. HLS/DASH passano da Shaka;
le radio audio continue usano `video.src` con il relay Rust descritto in
[RADIO.md](RADIO.md). Il cambio di modalità scarica la sessione precedente.
Ricerca, guida e scoperta TV escludono le radio; lo zapping resta nel tipo
di contenuto in riproduzione. Il catalogo Radio rende 48 stazioni per pagina.

I cataloghi personali interrogano SQLite tramite `/api/library/*`: non vengono
inseriti nello snapshot integrato o scaricati interamente nella UI. Ogni lista
continua conserva tre finestre di 200 righe (50 per guida e selettori), con
annullamento e generazioni per scartare risposte obsolete. La posizione logica
si adatta al limite di altezza del browser anche con un milione di elementi.
L’aggiornamento di ora/prossimo ricarica solo le finestre visibili.

Le righe live personali includono i programmi Lume solo quando non è dichiarata
una guida personale. `metadata_revision` invalida le finestre anche se la sorgente
non cambia; `artwork_revision` rinnova l’URL locale del logo dopo l’aggiornamento
del feed. `guide.origin` distingue la guida condivisa da quella personale e
mostra i comandi appropriati. L’indice completo resta in SQLite sul backend:
non viene incluso nello snapshot JSON del Catalogo Lume né inviato interamente
al frontend. Le credenziali dei flussi non passano alle richieste pubbliche.

La risposta condivisa include `guide.supplement` per lo stato EPGShare IT1.
`Channel.guide_origin` e la risposta di `/api/library/guide/window` distinguono
`lume` da `epgshare`, con selezione dello stesso palinsesto su scheda e guida.
La revisione include entrambi gli indici pubblici. La UI mantiene i comandi
della guida condivisa e indica la fonte integrativa senza offrire associazioni
manuali o cancellazione della cache privata per un feed pubblico.

I canali personali riusano `ChannelCard`; le radio riusano `RadioCard`.
La griglia virtuale legge colonne, spazi e altezza dalle stesse regole CSS dei
cataloghi integrati e segue lo scorrimento di `.content-scroll`. I selettori
e il browser compatto del player usano ancora righe. Le frecce seguono le
colonne della griglia; ritorno dal player, navigazione e ridimensionamento
conservano la posizione senza caricare l'intero catalogo.

La home personale legge `/api/library/home`: il backend seleziona solo TV live
nella sorgente abilitata, ordina al massimo 40 ID della cronologia e restituisce
cinque recenti. Il canale in evidenza è il più recente, oppure il primo canale.
Il collegamento alla guida conserva sorgente e canale selezionati. Le richieste
obsolete vengono annullate e non aggiornano la home di un'altra lista.
Quando il player occupa la pagina, home e griglia sospendono i loro aggiornamenti.

VOD e archivio conservano posizione e completamento sul backend, separati dai
recenti live. La modalità della sessione decide fine riproduzione, riavvio e
avanzamento; la riconnessione automatica riguarda le dirette. Il sistema DRM
arriva dalla sessione: ClearKey non usa la preparazione Widevine del contenuto.
Prima di applicare la nuova sessione, `drm.clearKeys` e `drm.servers` vengono
ripristinati con `undefined`. Shaka unisce le mappe: passare `{}` conserva le
vecchie voci e può impedire la riproduzione Widevine dopo una lista ClearKey.
La regressione è verificata leggendo la configurazione del vero Shaka dopo
la decodifica del video cifrato, mantenendo lo stesso player.
Dettagli e limiti in [PERSONAL_PLAYLISTS.md](PERSONAL_PLAYLISTS.md).

## Libreria VOD

`Videos.tsx` gestisce raccolte, locandine, dettagli, stagioni e ricerca TMDB.
I dati tipizzati e la cache immagini condivisa sono in `video.ts`; la vista
continua aggiunge il layout `videos` mantenendo tre finestre di dati. L’apertura
di un episodio conserva la navigazione della serie e la stagione scelta.

Le API `/api/library/video/{home,page,detail,action,settings,metadata,choose}`
sono autenticate come le altre API della libreria. Il backend `personal/video`
costruisce l’indice sul disco a blocchi di 200 righe, aggrega le serie in SQL e
legge i metadati estesi solo per la finestra restituita. Gli indici esistenti si
migrano al primo accesso; una sostituzione conserva identità e prima aggiunta.

Le immagini passano da `/art/video/{id}/{poster,backdrop}`, senza esportare gli
indirizzi delle sorgenti. Il frontend attende l’intersezione con l’area visibile,
condivide i download concorrenti e limita la cache a 100 immagini/16 MiB.
I dati immagine sono compatibili con `img-src 'self' data:` della build nativa.
Il server di test usa la stessa CSP e verifica la decodifica della locandina.

Completamento e visibilità di Continua a guardare invalidano la revisione di
scrittura del player precedente. Le azioni VOD condividono la coda delle
preferenze, così la rimozione di un vecchio preferito resta coerente con il player.
L’episodio successivo è un comando esplicito e non ricrea il player.

Il client TMDB usa host HTTPS fissi, DNS pubblici verificati e nessun header o
cookie della lista. Ricerche e schede partono dal solo dettaglio; la home legge
la cache. Spegnere la funzione o modificare/eliminare una sorgente impedisce la
pubblicazione di risposte ormai obsolete. Cache e impostazioni sono nel profilo
privato; il token non torna al frontend. I dettagli restano disponibili offline
finché la funzione è attiva.

## Widevine

Il monitor CEF legge un nuovo snapshot del component updater ogni secondo sul
thread UI del processo browser. Le API locali `/api/widevine` e
`/api/widevine/update` espongono soltanto fase, versione, tempo trascorso e codici
di errore previsti. Le richieste manuali vengono accorpate e limitate nel tempo.

La UI mostra controllo, download, installazione e verifica finale. La barra è
indeterminata: l’API pubblica CEF non fornisce byte scaricati o dimensione totale.
La verifica EME decide quando avviare la riproduzione. Su Linux una nuova
installazione può richiedere di chiudere completamente e riaprire Lume; un
componente già presente all’avvio non genera un ciclo di richieste di riavvio.

La preparazione viene eseguita all’avvio, prima di montare catalogo e player.
[startup.ts](../web/app/src/startup.ts) esegue il controllo EME con un elemento
video non collegato alla pagina: non apre una diretta e non richiede una licenza TV.
La [schermata iniziale](../web/app/src/components/Startup.tsx) mostra lo stato e
offre Riprova in caso di errore. Il catalogo si apre soltanto quando il componente
risulta utilizzabile. Se Widevine è già pronto, il controllo non richiede un
nuovo download. Non viene salvato un flag di completamento che potrebbe diventare
obsoleto dopo la rimozione o il guasto del componente.

## Compilazione

Servono Node 22.12+ e Python 3.12+. Dalla cartella `web/app`:

```sh
npm ci
npm run build
```

`bundle/` contiene HTML, JavaScript e CSS con nomi fissi, inclusi in Rust tramite
`include_bytes!`. Il manifest registra gli hash di sorgenti e asset. Questi file
vanno aggiornati insieme ai sorgenti; non si modificano a mano.

Per lavorare sulle schermate, avvia `npm run dev` e l’anteprima Rust
`cargo run -- app --serve-only --profile profiles/ui-test`. Il watcher ricompila
gli asset; riavvia l’anteprima dopo le modifiche perché gli asset sono incorporati
alla compilazione Rust. Il watcher usa una build di sviluppo; prima di terminare
esegui `npm run build` per produrre il bundle di distribuzione.

`python3 scripts/check_frontend.py`, dalla radice, rileva asset mancanti o non
allineati. La compilazione Rust e i pacchetti usano il bundle conservato nel
repository e non richiedono npm. Per ricompilare la UI dall’archivio sorgente
servono anche le dipendenze npm risolte nel lockfile, scaricate con `npm ci`.

## Verifiche

```sh
npm run check:format
npm test
npx playwright install chromium
npm run test:browser
```

I test browser usano un catalogo di fixture e un player simulato. Verificano
interazioni e continuità del DOM. Disponibilità DRM, codec, audio e immagini
richiedono le prove native con Rai, Mediaset e La7 su ciascun sistema.
La CI verifica anche che la ricompilazione produca gli stessi asset incorporati.

La prova della guida simula una risposta con una scadenza di riprova, poi controlla
che programmi e immagine compaiano senza ricaricare l'app e che il segnaposto si
nasconda. È distinta dal controllo del feed reale e dei loghi nell'app CEF,
documentato nello [stato delle verifiche](STATUS.md#guida-e-loghi-del-catalogo-lume-16-novembre).


## Compatibilità Android aggiuntiva

`components/Backups.tsx` gestisce upload, anteprima e selezione del ripristino.
Le risposte che modificano le preferenze passano dalla stessa coda delle azioni
del player. I contatori finali arrivano dallo stato del job, senza mostrare
indirizzi o header delle liste. Il download cifrato usa una capability locale;
il runtime CEF accetta il solo percorso di backup e presenta il dialogo Salva.

`components/Companion.tsx` usa il servizio Rust con pin TLS. Il telecomando e
Guarda sulla TV condividono i dialoghi nativi HTML, interrompono il polling
alla chiusura e mantengono controlli distinti per verifica e avvio. L’invio delle
liste richiede selezione della TV e consenso esplicito alle credenziali.

`components/Personalization.tsx` applica ordine e canali nascosti alle viste,
con ripristino dalle impostazioni. Le query personali applicano le stesse
scelte in SQLite, comprese paginazione e cambio canale. La revisione della
libreria include le personalizzazioni, per invalidare le finestre dopo una modifica.
