# Pubblicazione su GitHub

Il repository pubblico è [pjor94/LumeTV-Desktop](https://github.com/pjor94/LumeTV-Desktop),
con branch principale `main`. La prima versione Cargo è `0.1.0`, destinata alle
prove desktop. La CI esegue i controlli a ogni push o pull request e, se riescono,
produce i pacchetti scaricabili. Il flusso di tag e pre-release è in
[RELEASE.md](RELEASE.md).

## Contenuto pubblico

Sono inclusi sorgenti Rust/web, catalogo iniziale, asset del marchio, script,
test, `Cargo.lock`, componenti vendorizzati con licenze, documentazione e
[riepilogo delle prove](STATUS.md). Il runtime CEF si scarica durante la build;
il CDM non viene distribuito con i sorgenti o il bundle.

Restano locali `target/`, `.cache/`, `profiles/`, i report grezzi, i file di
credenziali e i certificati di firma. Le esclusioni sono in `.gitignore`; il
contesto Docker ammette solo gli input di build esplicitamente previsti.
I report storici citati nei documenti restano locali. La schermata pubblica del
catalogo mostra l’interfaccia senza fotogrammi televisivi.

## Prima di un commit

Dal checkout esegui:

```sh
python3 scripts/check_repository.py
git status --short
```

Il controllo esamina i file indicizzati e quelli nuovi non ignorati. Blocca
profili, CDM, build e materiale di firma, cerca alcuni formati di credenziali e
verifica i link locali Markdown e il checksum Shaka. I file ignorati restano
esclusi e gli eventuali valori trovati non vengono stampati. Storia Git e
condizioni di licenza delle dipendenze richiedono una revisione separata.

Prima del commit, dopo aver controllato l’elenco:

```sh
git add .
git diff --cached --stat
python3 scripts/check_repository.py --tracked
```

Esegui anche i controlli in [CONTRIBUTING.md](../CONTRIBUTING.md). Conserva i log
in `reports/`, senza forzarne l’aggiunta a Git. Il test con `--tracked` controlla
anche i file ignorati aggiunti a forza all’indice e richiede che i file
indicizzati corrispondano alla copia di lavoro.

Il remoto `origin` usa l’URL del repository pubblico, riportato anche nei
metadati Cargo. Per pubblicare occorrono credenziali con accesso a questo
repository.

## Impostazioni GitHub

- Controlla l’esito dei job del workflow [CI](../.github/workflows/ci.yml).
- Proteggi `main` e richiedi i job CI per le pull request.
- Abilita il canale per segnalazioni private di sicurezza, se disponibile.
- Rivedi le proposte Dependabot: le versioni CEF richiedono prove coordinate.

Queste impostazioni vanno applicate su GitHub dopo aver creato il remoto.

## Pacchetti e pre-release

Le pipeline preparano DMG/ZIP Mac arm64 e Intel, ZIP Windows x64 e
DEB/tar.gz Linux x64, insieme ai sorgenti Cargo corrispondenti, avvisi e
checksum. Il workflow di release pubblica una pre-release solo dopo il
successo di tutte le build e il controllo di versione/commit degli allegati.
Vedi [la guida di rilascio](RELEASE.md).

Gli esiti delle build native sono raccolti in [Stato del progetto](STATUS.md).
Le firme restano di sviluppo.

Per una release di produzione servono firma e notarizzazione appropriate,
revisione degli avvisi e dei componenti esterni, prove native di installazione
e riproduzione. La GPL copre il codice del progetto; runtime, codec, CDM e
contenuti hanno condizioni proprie: vedi [attribuzioni](../THIRD_PARTY_NOTICES.md).

L’aggiornamento automatico dell’app è ancora da implementare. Dopo la scelta
del canale di rilascio occorrerà definire feed delle versioni, verifica dei
pacchetti firmati, installazione, riavvio e recupero dopo un errore. Catalogo,
guida e componenti CEF hanno aggiornamenti separati dall’eseguibile di Lume TV.
