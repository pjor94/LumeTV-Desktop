# Analisi UI, animazioni e profondità

Le modifiche descritte qui sono state implementate: vedi [comportamento e verifiche](../docs/UI_MOTION.md). Questa analisi conserva i rilievi precedenti all’intervento.

L’8 novembre 2026 sono stati esaminati il codice e la finestra macOS, nelle viste catalogo, Cosa guardare e dettagli programma. L’analisi è stata svolta senza modificare il codice. Le prestazioni degli effetti proposti su Windows e Linux restavano da misurare.

## Direzione consigliata

Conservare il marchio Android, la palette, la tipografia di sistema e l’impostazione del catalogo. Il momento distintivo dovrebbe essere il passaggio continuo fra player grande e mini-player. Le altre animazioni accompagnano conferme e pannelli; ricerca, zapping e navigazione da tastiera restano immediati.

Per il simbolo Lume basta un accenno di profondità, da valutare dopo aver sistemato le interazioni principali. Un motore 3D completo aggiungerebbe complessità senza una funzione utile in queste schermate.

## Stato attuale

- Backend Rust, finestra CEF 150 e frontend HTML/CSS/JavaScript senza framework di animazione. Il video Shaka è un elemento stabile fuori dal contenuto delle pagine (`web/app/index.html:69`).
- Pressione dei pulsanti già presente: scala 0.96 in 160 ms. Schede con sollevamento di 2 px in 180 ms; controlli player con dissolvenza di 200 ms. Queste intensità sono già contenute.
- Un solo ciclo CSS continuo: lo spinner durante il caricamento. Hero e simbolo sono statici; non esiste un motore 3D.
- Ricerca, zapping, filtri e scorciatoie si usano spesso e richiedono una risposta immediata. Dettagli programma, promemoria e conferme si prestano a brevi transizioni. L’introduzione del marchio, visibile raramente, deve lasciare subito utilizzabili i canali.

## Rilievi verificati

| # | Gravità | Categoria | Posizione | Rilievo | Intervento proposto |
| --- | --- | --- | --- | --- | --- |
| 1 | Alta | Continuità e prestazioni | `web/app/app.js:284`, `:292`, `:575`, `:762` | Il rendering sostituisce intere sezioni con `innerHTML`, anche durante gli aggiornamenti automatici. Animazioni di ingresso applicate indiscriminatamente potrebbero ripartire mentre si legge o si cerca. | Distinguere navigazione, azione dell’utente e aggiornamento dati. Aggiornare i nodi modificati, conservando identità, focus e scroll. Gli aggiornamenti periodici non devono rilanciare gli ingressi. Sforzo medio. |
| 2 | Media | Leggibilità e gerarchia | `web/app/app.css:12`, `web/app/app.js:163`, `:195` | Titoli dei programmi nelle schede e nelle liste sono spesso troncati; diversi metadati hanno dimensioni ridotte. Hero e recenti occupano spazio prima del catalogo. | Valutare due righe per i titoli delle schede, con altezza riservata; rendere più leggibili i metadati e compattare le sezioni introduttive nelle finestre basse. Verificare nomi lunghi e layout da 820 px. Sforzo basso/medio. |
| 3 | Media | Entrata, uscita e origine | `web/app/app.js:345`, `:670`, `:686`; `web/app/app.css:12`, `:16` | Dettagli e opzioni passano direttamente fra stati visibili e nascosti. | Dettagli: opacità 0→1 e scala 0.97→1 in 220 ms; uscita 140 ms. Opzioni player: opacità e spostamento verticale di 4 px in 160 ms, origine in basso a destra; uscita 120 ms. Curva `cubic-bezier(0.23, 1, 0.32, 1)`. Conservare la semantica del dialogo e il ritorno del focus. Sforzo basso/medio. |
| 4 | Bassa | Coerenza e accessibilità | `web/app/app.css:6`, `:8`, `:24` | Le durate sono locali; la modalità di movimento ridotto azzera globalmente quasi tutte le transizioni. Il supporto alla preferenza esiste già. | Introdurre pochi token condivisi. Con movimento ridotto rimuovere scala, inclinazioni e spostamenti; conservare, dove utile, dissolvenze di 100 ms e conferme di stato. Sforzo basso. |

Le sostituzioni complete del DOM possono interferire con le animazioni. L’analisi non ha misurato frame persi o prestazioni GPU.

## Opportunità di animazione

1. **Player ↔ mini-player** (priorità alta, sforzo medio). Raccordare posizione e dimensioni in 280 ms con `cubic-bezier(0.32, 0.72, 0, 1)`. Usare lo stesso elemento video e una trasformazione basata sui rettangoli iniziale/finale, annullabile e ricalcolabile durante resize o navigazione rapida. Ridurre al minimo la deformazione delle proporzioni e mantenere leggibili i comandi. Con movimento ridotto, cambiare geometria immediatamente. Questo passaggio rende più chiara la continuità della diretta.

2. **Preferiti e conferme** (priorità media, sforzo basso). Conferma locale della stella in 140 ms, scala 0.9→1 sull’icona; il resto della scheda rimane fermo. Messaggi: ingresso con opacità e `translateY(6px)` in 180 ms, uscita sullo stesso percorso in 140 ms, curva `cubic-bezier(0.23, 1, 0.32, 1)`. Gestire la coda e le interruzioni; non animare nuovamente l’intera griglia dopo il salvataggio.

3. **Profondità del marchio** (opzionale, sforzo basso/medio). Una breve posa prospettica del simbolo nell’introduzione: prospettiva 900 px, inclinazione iniziale di 3°, ritorno a riposo in 280 ms con `cubic-bezier(0.23, 1, 0.32, 1)`. Testi, logo del canale e pulsanti rimangono stabili. Eseguire il movimento una sola volta, lasciando subito accessibili i canali; con movimento ridotto, mantenere il simbolo fermo. Valutare il risultato nei due temi prima di includerlo.

## Effetti esclusi dalla proposta

- Cascata di ingresso su tutte le 126 schede: rallenta la scansione e rischia di ripetersi con filtri e refresh.
- Rotazione delle schede al passaggio del mouse: muove informazioni da leggere e aggiunge lavoro durante lo scorrimento.
- Animazioni della ricerca, dello zapping o dell’apertura tramite scorciatoia: sono azioni frequenti, per cui conta la risposta immediata.
- Particelle, sfondi WebGL e logo continuamente fluttuante: non aggiungono una funzione al catalogo o alla visione.

## Fattibilità e verifiche

La proposta usa CSS e Web Animations API nel frontend esistente. Non richiede una nuova dipendenza di animazione. Per gli effetti principali privilegiare `transform` e `opacity`, separare le letture di geometria dalle scritture e applicare `will-change` solo quando serve. Sono le indicazioni della [guida Google alle animazioni efficienti](https://web.dev/articles/animations-guide); la fluidità effettiva va misurata nel runtime Lume.

Per il player occorre una prova dedicata: la [documentazione Chrome](https://developer.chrome.com/docs/web-platform/view-transitions/same-document#run_transitions_without_freezing_other_animations) mostra che le View Transitions predefinite possono presentare un fotogramma congelato durante il raccordo. La proposta è animare il contenitore del video in riproduzione, senza sostituirlo con una cattura. La resa con Widevine va verificata nella build Lume.

La [prospettiva CSS](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/perspective) è sufficiente per l’effetto di profondità proposto; un motore 3D resterebbe una scelta da giustificare con una funzione o una scena concreta.

Ordine consigliato: rendering e leggibilità → pannelli e feedback → raccordo del player → eventuale profondità del marchio. Collaudo con Rai, Canale 5 e La7 durante la visione, controllando frame persi rispetto alla baseline, continuità audio, resize, inversione rapida del movimento, tastiera e movimento ridotto. Ripetere il collaudo su macOS, Windows e Linux; l’obiettivo del frontend è 60 fps sui dispositivi di riferimento, non ancora misurati per questi effetti.
