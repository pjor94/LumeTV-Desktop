# Aggiornamenti dell’app

Le nuove build controllano le release all’avvio del catalogo e circa ogni
mezz’ora mentre Lume è aperta. Un indicatore nelle Impostazioni segnala la nuova
versione. L’avviso non compare sopra una diretta e non apre finestre modali.
Il controllo si può disattivare; il pulsante **Controlla** resta disponibile.

Il download parte su richiesta, mostra byte e percentuale e si può annullare.
Una volta verificato il pacchetto, l’utente sceglie quando installarlo.
L’aggiornamento non parte durante la TV: il pulsante di installazione ferma
prima la riproduzione.

| Installazione corrente | Comportamento |
| --- | --- |
| Bundle macOS in una cartella scrivibile | Chiude Lume, sostituisce il bundle completo e riapre l’app |
| ZIP Windows o tar.gz Linux in una cartella scrivibile | Sostituisce l’intera cartella, incluse le risorse CEF, e riapre l’app |
| DEB Linux in `/opt/lume-tv` | Apre il DEB verificato con il gestore di sistema e chiude Lume; dopo l’installazione occorre riaprirla |
| Cartella non scrivibile o formato di sviluppo non riconosciuto | Apre il pacchetto nativo per completare l’installazione manualmente |

Preferiti, guida, profilo del browser e Widevine sono nella cartella dati
dell’utente, separata dall’app. L’aggiornamento del programma non li sostituisce.
La prima versione pubblicata, **0.1.0**, non contiene l’updater: la prima build
che lo include va installata manualmente.

## Verifica dei pacchetti

Il servizio legge le release pubbliche di `pjor94/LumeTV-Desktop`. Esclude draft,
versioni uguali o precedenti e release senza feed firmato. Le anteprime sono
escluse per impostazione iniziale; si possono includere attivando **Versioni di
prova** nelle Impostazioni. L’architettura segue il binario in esecuzione: arm64 e x64 su
macOS, x64 su Windows e Linux.

`update.json` descrive versione, identità dell’app e pacchetti. `update.sig`
contiene una firma Ed25519 dei suoi byte esatti. La chiave pubblica è incorporata
nell’app e nell’helper. Dimensione e SHA-256 del download devono corrispondere
al feed firmato e ai metadati GitHub. L’app accetta solo HTTPS verso GitHub e
i suoi host di distribuzione, con limiti per risposte, redirect e file.

Un errore di rete non rimuove una versione già verificata nella sessione.
I limiti di GitHub sospendono i controlli, anche quelli manuali, fino al termine
della pausa richiesta. Il controllo non impedisce di usare Lume offline.

L’helper viene copiato fuori dal pacchetto e verifica nuovamente firma e hash.
Solo dopo la sua conferma il player si chiude. L’estrazione avviene in una
cartella temporanea sullo stesso disco, con controlli sui percorsi e sul layout
del runtime. Il nuovo bundle macOS deve superare anche `codesign --verify`.
Se la sostituzione fallisce, l’helper tenta di ripristinare la copia precedente.
La cartella `.lume-backup-*` accanto all’app resta disponibile per il recupero;
dopo aver verificato il funzionamento della nuova versione si può rimuovere.
Non viene richiesta elevazione dei privilegi dall’helper.

La firma del feed autentica i pacchetti di Lume. È distinta da Developer ID,
notarizzazione Apple e Authenticode: i controlli del sistema operativo restano
applicabili.

## Pubblicazione e chiave di firma

Il job di pubblicazione verifica i pacchetti e i sorgenti corrispondenti, crea
il feed e lo firma prima di pubblicare tutti gli allegati insieme. La secret
GitHub **`LUME_UPDATE_SIGNING_KEY`** deve contenere la chiave privata Ed25519 in
formato PEM. È disponibile soltanto nel passo di pubblicazione, non nelle build
delle pull request. Una chiave assente o diversa dalla pubblica interrompe la
release.

Per passare da una beta o rc alla versione ordinaria, pubblica un nuovo tag
senza suffisso. La pipeline firmerà un nuovo feed con il canale corretto.

La chiave pubblica è in [update-public-key.hex](../resources/update-public-key.hex).
La copia privata locale è in `release-keys/updates-ed25519.pem`, esclusa da Git
e dai sorgenti di distribuzione. Conservane un backup protetto: perderla
impedisce di firmare aggiornamenti accettati dalle app già installate.
Per un nuovo progetto si genera una coppia con
`python scripts/update_feed.py init`; il comando rifiuta di sostituire chiavi
esistenti. Non rigenerarla per una release ordinaria.

La CI prova separatamente l’helper fuori dalla cartella CEF. Le prove native
complete di aggiornamento vanno eseguite sui sistemi di destinazione, oltre ai
test di firma, download, estrazione e ripristino.
