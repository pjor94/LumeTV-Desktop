#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the audited macOS ARM64 CEF prototype. No playback or CDM copy occurs.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
PROJECT_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd -P)"
NATIVE_TOOLCHAIN="stable-aarch64-apple-darwin"
CACHE_DIR="$PROJECT_ROOT/.cache/cef-obs"
ARCHIVE_PATH="$CACHE_DIR/cef_binary_7871_macos_arm64.tar.xz"
SDK_DIR="$CACHE_DIR/sdk"
BUILD_DIR="$PROJECT_ROOT/target/cef-obs-arm64"
APP_PATH="$BUILD_DIR/debug/Lume TV.app"
ARCHIVE_TEMP=""
SDK_TEMP=""

fail() {
    printf 'Errore: %s\n' "$*" >&2
    exit 1
}

cleanup() {
    # Only paths created by this invocation are removed on failure.
    if [[ -n "$ARCHIVE_TEMP" && -f "$ARCHIVE_TEMP" ]]; then
        rm -f -- "$ARCHIVE_TEMP"
    fi
    if [[ -n "$SDK_TEMP" && -d "$SDK_TEMP" ]]; then
        rm -rf -- "$SDK_TEMP"
    fi
}
trap cleanup EXIT

[[ "$(uname -s)" == "Darwin" ]] || fail "Questo script richiede macOS su Apple Silicon."
case "$(uname -m)" in
    arm64|aarch64) ;;
    *)
        # A Rosetta shell can still launch the installed native Rust toolchain.
        [[ "$(sysctl -n hw.optional.arm64 2>/dev/null || true)" == "1" ]] \
            || fail "È richiesto un Mac ARM64; gli host Intel non sono supportati da questo SDK."
        ;;
esac

for executable in python3 rustup cargo curl shasum cmake ninja codesign xcrun; do
    command -v "$executable" >/dev/null 2>&1 || fail "Comando richiesto non disponibile: $executable"
done
rustup which --toolchain "$NATIVE_TOOLCHAIN" rustc >/dev/null 2>&1 \
    || fail "Installa prima il toolchain $NATIVE_TOOLCHAIN; lo script non modifica il toolchain globale."
NATIVE_HOST="$(rustup run "$NATIVE_TOOLCHAIN" rustc --version --verbose)"
[[ "$NATIVE_HOST" == *"host: aarch64-apple-darwin"* ]] \
    || fail "Il toolchain selezionato non è nativo ARM64."
xcrun --find clang >/dev/null

cd -- "$PROJECT_ROOT"
if [[ -z "${CARGO_HOME:-}" ]]; then
    export CARGO_HOME="$PROJECT_ROOT/.cache/cargo"
fi
mkdir -p -- "$CACHE_DIR"
[[ ! -L "$ARCHIVE_PATH" && ! -L "$SDK_DIR" ]] \
    || fail "Archivio e directory SDK devono essere percorsi regolari nella cache del progetto."

# One source of truth for the official OBS URL and published checksum.
OBS_CONSTANTS="$(python3 - "$SCRIPT_DIR/prepare_obs_cef.py" <<'PY'
import importlib.util
from pathlib import Path
import sys
spec = importlib.util.spec_from_file_location("lume_obs_prepare", Path(sys.argv[1]))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
print(module.OBS_ARCHIVE_URL)
print(module.OBS_ARCHIVE_SHA256)
PY
)"
OBS_URL="${OBS_CONSTANTS%%$'\n'*}"
OBS_SHA256="${OBS_CONSTANTS##*$'\n'}"
[[ "$OBS_URL" == https://cdn-fastly.obsproject.com/downloads/* ]] \
    || fail "Origine dell'archivio OBS inattesa."
[[ "$OBS_SHA256" =~ ^[0-9a-f]{64}$ ]] || fail "Checksum OBS non valido."

verify_archive() {
    printf '%s  %s\n' "$OBS_SHA256" "$1" | shasum -a 256 --check --status
}

if [[ ! -f "$ARCHIVE_PATH" ]] || ! verify_archive "$ARCHIVE_PATH"; then
    ARCHIVE_TEMP="$(mktemp "$CACHE_DIR/.cef-download.XXXXXX")"
    printf 'Scaricamento SDK CEF ufficiale OBS…\n'
    curl --fail --location --retry 3 --connect-timeout 20 --max-time 1200 \
        --proto '=https' --proto-redir '=https' --output "$ARCHIVE_TEMP" "$OBS_URL"
    verify_archive "$ARCHIVE_TEMP" || fail "Il checksum dell'archivio scaricato non corrisponde."
    mv -f -- "$ARCHIVE_TEMP" "$ARCHIVE_PATH"
    ARCHIVE_TEMP=""
else
    printf 'Archivio OBS in cache: checksum verificato.\n'
fi

if [[ ! -f "$SDK_DIR/include/cef_version.h" ]]; then
    [[ ! -e "$SDK_DIR" ]] \
        || fail "La directory SDK esiste senza include/cef_version.h; viene conservata senza sovrascriverla."
    SDK_TEMP="$(mktemp -d "$CACHE_DIR/.sdk-extract.XXXXXX")"
    printf 'Estrazione dell’SDK verificato nella cache del progetto…\n'
    python3 - "$ARCHIVE_PATH" "$SDK_TEMP" "$SDK_DIR" <<'PY'
import os
from pathlib import Path
import sys
import tarfile
archive, staging, sdk = map(Path, sys.argv[1:])
with tarfile.open(archive, "r:xz") as source:
    source.extractall(staging, filter="data")
if not (staging / "include/cef_version.h").is_file():
    raise SystemExit("Layout dell'archivio OBS inatteso; SDK esistente invariato.")
os.rename(staging, sdk)
PY
    SDK_TEMP=""
fi

python3 "$SCRIPT_DIR/prepare_obs_cef.py" --archive "$ARCHIVE_PATH" --sdk "$SDK_DIR"

printf 'Compilazione Rust ARM64 con SDK CEF verificato…\n'
CEF_PATH="$SDK_DIR" LUME_CEF_ALLOW_OBS_150_0_19=1 \
    cargo +"$NATIVE_TOOLCHAIN" build --locked --features embedded-cef --bins --target-dir "$BUILD_DIR"

python3 "$SCRIPT_DIR/bundle_cef.py" --sdk "$SDK_DIR" \
    --target-dir "$BUILD_DIR/debug" --output "$APP_PATH" --codec-build custom

printf '\nApp pronta: %s\n' "$APP_PATH"
cat <<EOF

La build non ha avviato il player né copiato componenti Widevine.
Apri "Lume TV.app" con un doppio clic oppure avvia il catalogo completo:

  "$BUILD_DIR/debug/lume-player" app --cef-executable "$APP_PATH/Contents/MacOS/lume-cef"

Per provare la preparazione automatica Widevine in un profilo dedicato:

  "$BUILD_DIR/debug/lume-player" play --cef-executable "$APP_PATH/Contents/MacOS/lume-cef" --channel canale5 --profile "$PROJECT_ROOT/profiles/cef-first-run" --drm-setup-seconds 120 --seconds 30

CEF gestisce il download nel profilo: serve Internet al primo avvio.
La UI macOS arm64 ha verificato Canale 5 da un profilo browser vuoto,
dopo il download automatico e senza riavvio. Dettagli in docs/EMBEDDED.md.

In alternativa, per una prova con un CDM già disponibile e autorizzato:

  cd "$PROJECT_ROOT"
  export LUME_CDM_SOURCE="/percorso/al/componente/WidevineCdm"
  mkdir -p "$PROJECT_ROOT/profiles/cef-obs"
  touch "$PROJECT_ROOT/profiles/cef-obs/.lume-player-profile"
  python3 "$SCRIPT_DIR/seed_widevine.py" --source "\$LUME_CDM_SOURCE" --profile "$PROJECT_ROOT/profiles/cef-obs" --check
  python3 "$SCRIPT_DIR/seed_widevine.py" --source "\$LUME_CDM_SOURCE" --profile "$PROJECT_ROOT/profiles/cef-obs"

Per avviare successivamente la matrice grafica Rai 1 / Canale 5 / La7:

  CEF_PATH="$SDK_DIR" LUME_CEF_ALLOW_OBS_150_0_19=1 cargo +$NATIVE_TOOLCHAIN run --locked --features embedded-cef --target-dir "$BUILD_DIR" -- matrix --cef-executable "$APP_PATH/Contents/MacOS/lume-cef" --widevine-cdm "\$LUME_CDM_SOURCE" --profile "$PROJECT_ROOT/profiles/cef-obs" --seconds 30 --report "$PROJECT_ROOT/reports/macos-cef-obs-matrix.json"

EOF
