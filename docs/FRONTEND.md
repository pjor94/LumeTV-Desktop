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
| [app.ts](../web/app/src/app.ts) | Catalogo, guida, preferenze, promemoria e stato delle schermate |
| [playback.ts](../web/app/src/playback.ts) | Shaka, sessioni, cambio canale, tracce e diagnostica |
| [recovery.ts](../web/app/src/recovery.ts) | Tentativi di riconnessione, attesa della rete e annullamento |
| [pages/Radio.tsx](../web/app/src/pages/Radio.tsx) | Catalogo radio, ricerca, preferite, recenti e paginazione |
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
