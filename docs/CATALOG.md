# Catalogo Android nel desktop Rust

Alla verifica dell’8 novembre 2026 su macOS arm64, il catalogo incluso e la lista online contenevano **126 canali supportati**, con la stessa selezione dell’app Android. Il [riepilogo delle verifiche](STATUS.md) riporta lo stato del player sulle diverse piattaforme.

L’11 novembre è stata aggiunta una sezione separata con **1.310 radio italiane**.
Il conteggio TV e le associazioni EPG restano riferiti ai 126 canali.
Fonte, importazione e cache delle radio sono descritti in [RADIO.md](RADIO.md).

La copia della guida in [`resources/guide.json`](../resources/guide.json) è stata
importata l’11 novembre 2026 dal feed XMLTV/XZ. Contiene
4.075 programmi e 83 canali con loghi; date, origine e checksum sono in
[`guide-source.json`](../resources/guide-source.json). È disponibile per la futura
inclusione come guida iniziale: il caricamento automatico dal bundle resta da
integrare.

| Integrazione | Canali | Manifest validi nell’audit |
| --- | ---: | ---: |
| Rai | 21 | 14 |
| Mediaset | 23 | 23 |
| La7 | 2 | 2 |
| Sky: TV8, Cielo, Sky TG24 | 3 | 3 |
| Discovery | 8 | 8 |
| Rai TGR | 22 | 22 |
| Dirette | 47 | 40 |
| Totale | **126** | **112** |

L’audit completo (`reports/catalog-all-manifests.json`) ha risolto 119 canali e letto 112 manifest validi, elaborando al massimo tre canali alla volta con sessioni provider condivise. Controlla risoluzione e manifest senza richiedere licenze DRM; la riproduzione video si verifica separatamente. Puoi ripeterlo con l’esempio `catalog_audit`.

Nella verifica risultano indisponibili Rai 4K, Rai Play 1/2/3 e Rai Play Sport 1/2/3; questi ultimi sei non compaiono nelle dirette Rai disponibili. BOM Channel, Business24, Explorer e Otto FM restituiscono HTTP 404. CS24.Live, Radio Tv Serie A e Sport Mediaset non completano la richiesta del manifest. Non sono stati disattivati i controlli TLS o modificati gli indirizzi per aggirare questi errori. La disponibilità può cambiare con il palinsesto e la rete.

## Aggiornamento e compatibilità dei dati

- `resources/channels.m3u` è la copia iniziale della playlist Android. Il parser applica la stessa selezione di integrazioni supportate e le esclusioni RSI.
- L’app controlla la fonte pubblica Android ogni 24 ore e offre **Impostazioni → Catalogo canali → Aggiorna**. Le richieste durante uno stesso aggiornamento vengono accorpate; gli errori introducono una pausa e HTTP 429 rispetta `Retry-After`.
- Il file privato `catalog.json` viene sostituito atomicamente solo dopo la validazione della lista. Una lista troncata, troppo piccola, corrotta o non scaricabile conserva i dati precedenti; senza cache resta utilizzabile la copia inclusa.
- `resources/catalog-legacy.json` mantiene tutti i 46 identificativi precedenti. I nuovi canali usano l’identità SHA-256 Android: integrazione/chiave per i resolver e identificativo EPG/nome/gruppo per le dirette. Preferiti, recenti e promemoria usano questi identificativi. La momentanea assenza di un canale non cancella le sue preferenze salvate.
- La guida associa gli identificativi XMLTV ai nuovi identificativi del catalogo. Cambiamenti alla composizione del catalogo invalidano la guida senza annullare il backoff del servizio. La verifica offline con il download XMLTV reale delle 08:56 ha trovato dati e loghi per **83 canali**; gli altri restano senza palinsesto. `examples/guide_cache.rs` conserva la data del download durante l’importazione di una fixture.

L’aggiornamento online è riuscito e la cache è stata riletta al riavvio. La UI include i filtri Sky, Discovery e Altri; Rai comprende i 22 canali regionali. Il filtro non passa da Canali a Preferiti, evitando di nascondere un preferito di un’altra emittente. Alla larghezza minima di 820 pixel non è stato osservato overflow orizzontale dei nuovi filtri.

## Riproduzione e limiti

La matrice nativa iniziale (`reports/catalog-macos-native-matrix.json`) misura video, frame e audio decodificato nella finestra Rust/CEF. I tentativi ripetuti sono nel controllo successivo (`reports/catalog-macos-native-recheck.json`); il report iniziale resta invariato.

**Rai 1, Canale 5, La7, TV8, Nove, TGR Lombardia e RTL 102.5** hanno tutti superato 30 secondi misurati, con decodifica audio. Widevine è verificato per Canale 5 e La7. Nove ha superato anche **85 secondi nella UI completa**, continuando a riprodurre durante l’aggiornamento manuale del catalogo. Risultati complessivi (`reports/catalog-validation.json`), prova nella UI (`reports/catalog-native-ui-summary.json`).

TGR Lombardia ha ricevuto segmenti HTTP 404 dopo circa 12 secondi nel primo tentativo e ha superato i 30 secondi nel secondo. La selezione regionale usa il servizio Rai della regione richiesta, senza sostituirlo con Rai 3 nazionale.

Nel profilo vuoto Widevine non era ancora pronto al termine dei 120 secondi del primo tentativo Canale 5. Durante il successivo tentativo La7 il Component Updater ha completato il download automaticamente; La7 ha poi riprodotto il video protetto. App e CLI concedono ora fino a **300 secondi**, con annullamento al cambio canale e senza includere il tempo di preparazione nella misura video. Non è stato copiato un CDM da Chrome e il bundle non contiene Widevine.

Le prove automatiche partono con audio disattivato e misurano la decodifica; l’ascolto va verificato a parte. Le prove video di 30 secondi coprono solo una parte dei 126 canali. Windows e Linux condividono l’architettura Rust/relay/Shaka, ma il nuovo catalogo resta da verificare in esecuzione su quei sistemi.

I file in `reports/` sono locali ed esclusi da Git. Per nuove prove segui il
[protocollo di validazione](VALIDATION.md).

## Dati delle sorgenti

Chiavi, URL delle sorgenti, header privati e indirizzi delle licenze restano nel backend Rust. Gli oggetti del catalogo esposti alla UI e i report omettono questi campi. Il relay e il controllo manifest verificano le destinazioni anche dopo i redirect e risolvono solo indirizzi pubblici; una sorgente HTTP legacy può restare soltanto sul suo host e sulla sua porta. Il servizio licenze continua a richiedere HTTPS.

Verifiche del codice: **66 test Rust con CEF**, **20 test Node**, **18 test Python**, Clippy `-D warnings` e formattazione. I log `reports/catalog-*` restano distinti dai risultati precedenti del prototipo a 46 canali.
