# Radio sul desktop

La sezione Radio usa la stessa fonte del lavoro radio presente nel checkout
Android: WorldRadio di WorldLiveTV / WLTV Group. La copia inclusa, scaricata
l’11 novembre 2026, contiene 1.312 voci; dopo la deduplicazione risultano
**1.310 stazioni**. L’origine e il checksum sono in
[`radio-source.json`](../resources/radio-source.json).

## Catalogo e preferenze

`src/radio.rs` legge il feed Base64/Deflate, normalizza le città e usa
`SHA-256("radio|codice")`, troncato a 12 byte, per mantenere gli identificativi
Android. Cambiare l’indirizzo della sorgente non cambia i preferiti. Le voci
disabilitate, incomplete o con indirizzi non ammessi vengono scartate.

Il catalogo viene controllato all’avvio e ogni 24 ore. **Aggiorna radio** richiede
un controllo manuale. Un errore introduce una pausa di un’ora; lo stato viene
conservato dopo il riavvio. Una lista vuota, corrotta o ridotta a meno della metà
delle stazioni precedenti non sostituisce la copia valida. Il file locale
`radio-catalog.json` viene scritto con sostituzione atomica.

`kind: radio` separa le stazioni dai canali TV. Le radio hanno ricerca per nome
e città, preferite e recenti. Non entrano nella guida TV, nei suggerimenti dei
programmi o nello zapping televisivo. La UI presenta 48 stazioni per pagina e
mantiene ricerca e filtro quando si torna dalla schermata di ascolto.

## Riproduzione

HLS e DASH usano Shaka e il relay già presente. Per MP3 e AAC continui,
`src/radio_audio.rs` mantiene una connessione HTTP e passa l’audio allo stesso
elemento multimediale tramite un indirizzo locale temporaneo. Il buffer della
coda è limitato a quattro blocchi da 32 KiB; la risposta non viene accumulata
per intero. Chiusura, cambio stazione e arresto del server annullano il flusso.

Il decoder rimuove i blocchi ICY dall’audio e legge `StreamTitle`. Legge anche
il titolo ID3 iniziale quando supportato. I metadati temporizzati HLS vengono
inoltrati dal player. Il titolo compare solo quando fornito dalla stazione;
non viene ricavato da servizi esterni.

Il player tenta tre riconnessioni, dopo 2, 5 e 10 secondi. Un blocco di buffering
di 45 secondi attiva il recupero. In assenza di rete attende il ritorno della
connessione. Pausa, cambio stazione e chiusura annullano i tentativi. Il contatore
si azzera dopo 30 secondi di riproduzione stabile, per evitare cicli infiniti.

Il mini-player continua l’ascolto durante la navigazione nell’app. Non sono
previsti servizi in background, controlli di sistema/cuffie, modalità TV solo
audio o risparmio dati in questa fase.

## Rete e verifica

La sorgente del catalogo è fissa e HTTPS. Le destinazioni audio e i redirect
devono essere HTTP/HTTPS pubblici, senza credenziali nell’URL; il resolver DNS
esclude gli indirizzi locali e privati. La route audio locale ammette solo la
sessione attiva, con identificativo casuale, e rifiuta origini estranee.
Le API di stato e aggiornamento richiedono il token della sessione desktop.

I test coprono importazione, limiti di decompressione, identità delle stazioni,
cache dopo un errore, separazione ICY/audio, lettura ID3, annullamento del flusso,
autenticazione delle API, filtri e continuità del player. Le prove browser usano
fixture; non dimostrano la disponibilità delle singole emittenti. Le misure
native e i sistemi ancora da verificare sono riportati in [STATUS.md](STATUS.md).

Per distinguere il supporto HLS dalla disponibilità delle emittenti, il collaudo
ha usato anche “Audio only HLS with raw AAC”, presente nel
[catalogo di test ufficiale Shaka](https://github.com/shaka-project/shaka-player/blob/main/demo/common/assets.js).
Il riferimento era nel solo profilo di prova e non viene aggiunto alle radio distribuite.
