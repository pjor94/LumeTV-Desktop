# Contribuire a Lume TV Desktop

Lume TV Desktop è un prototipo Rust/CEF con un core condiviso fra macOS, Windows
e Linux. La CI compila i pacchetti sui runner nativi delle tre piattaforme.
Prima di modificare runtime o integrazioni leggi [stato delle prove](docs/STATUS.md)
e [attribuzioni](THIRD_PARTY_NOTICES.md).

## Ambiente

- Rust 1.92+ con `rustfmt` e `clippy`.
- Node 22+ per i test JavaScript; nessuna installazione npm necessaria.
- Python 3.12+ per gli strumenti e i loro test; non servono pacchetti pip.
- Git per controllare il contenuto destinato al repository.

Per il bundle macOS occorrono inoltre CMake, Ninja, Xcode Command Line Tools e
il toolchain Rust nativo ARM64. Segui [la guida CEF](docs/EMBEDDED.md). Non serve
scaricare un SDK o Widevine per lavorare sul core, sui test Node o sui test Python.

## Controlli prima di una pull request

```sh
python3 scripts/check_repository.py
cargo fmt --all -- --check
cargo test --locked --all-targets
cargo clippy --locked --all-targets -- -D warnings
cargo build --locked
cargo check --locked --features embedded-cef,cef/dox --bins
cargo check --locked -p lume-cef-windows --features dox
node --test web/player.test.cjs web/app/app.test.cjs web/app/ui.test.cjs
python3 -m unittest discover -s tests -p 'test_*.py'
bash -n scripts/build_embedded_macos.sh
```

`cef/dox` controlla le API Rust senza scaricare, collegare o eseguire l’SDK CEF.
Il job Python usa Linux; le fixture del bundler impiegano symlink e funzionano
senza un framework macOS reale. Il controllo del repository legge i file
indicizzati e quelli nuovi non ignorati. Se trova possibili credenziali, indica
percorso e riga senza stamparne il valore. La cronologia Git e i formati di
segreti non riconosciuti dal controllo richiedono una revisione separata.

Per le verifiche DOM, avvia un server con document root `web` e apri
`web/app/ui.fixture.html`. Premi il pulsante di avvio e controlla i nove risultati.
La directory del progetto contiene profili e cache private: tienila fuori dal
server. La fixture è esclusa dagli asset dell’app.

## Modifiche CEF e dipendenze

Mantieni `Cargo.lock` e gli avvisi dei componenti vendorizzati. La coppia
CEF/binding e la patch `vendor/download-cef` sono vincolate a versioni e hash API
precisi: un aggiornamento richiede un controllo coordinato di SDK, binding,
wrapper, packaging e riproduzione. Codec e DRM richiedono prove nel player.
Vedi [la patch](vendor/download-cef/LUME_PATCH.md).

Quando cambi i pin SDK, elimina la cache locale CEF e le cache GitHub Actions
con prefisso desktop prima di ricompilare. Gli SDK recuperati dalla cache
vengono sempre ricontrollati; una provenienza diversa dai pin ferma la build.

I programmi e gli script non devono disattivare sandbox, TLS o controlli delle
destinazioni di rete per risolvere un problema di un’emittente. I test automatici
ordinari usano fixture e server locali; le prove live vanno richieste ed eseguite
separatamente seguendo [il protocollo](docs/VALIDATION.md).

## Segnalazioni e diagnostica

Indica versione o commit, sistema, architettura, backend, passaggi per riprodurre
il problema e risultato atteso. Per i problemi video specifica quali controlli
hai eseguito: manifest, decodifica, DRM, immagine sul monitor e ascolto dell’audio.
Segnala anche quelli ancora da fare.

`profiles/`, `.cache/`, `target/` e i report grezzi restano locali. Non allegare
cookie, token, URL firmati, header di autorizzazione o challenge/risposte DRM.
Le evidenze pubbliche contengono soltanto dati selezionati e controllati; non
rendere pubblica una directory di report con `git add -f`.

Le contribuzioni al codice del progetto seguono la licenza GPL-3.0-or-later;
conserva le attribuzioni e segnala la provenienza di codice o risorse aggiunti.
