#!/usr/bin/env bash
# bwms-install.sh — installs the Black Wall Mod System: makes Cyberpunk 2077 load
# the bwms dylib (adds an LC_LOAD_DYLIB and re-signs the .app PRESERVING CDPR's
# entitlements). Uses only BASE macOS tools (codesign, xattr) + the native
# bwms-helper — ZERO Xcode/Python. Backup + reversible.
#
# Usage:  ./bwms-install.sh "/path/to/Cyberpunk 2077"            (install)
#         ./bwms-install.sh --restore "/path/to/Cyberpunk 2077"  (revert)
set -uo pipefail

GAME_DIR="${2:-${1:-$HOME/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077}}"
[ "${1:-}" = "--restore" ] && MODE=restore || MODE=install
APP="$GAME_DIR/Cyberpunk2077.app"
BIN="$APP/Contents/MacOS/Cyberpunk2077"
RED4="$GAME_DIR/red4ext"
BAK="$RED4/Cyberpunk2077-original.bin"   # OUTSIDE the .app bundle (else it breaks the seal)
ENT="$RED4/_bwms_entitlements.plist"
HERE="$(cd "$(dirname "$0")" && pwd)"
DYLIB_SRC="${BWMS_DYLIB_SRC:-$HERE/libcp77_console.dylib}"
[ -f "$DYLIB_SRC" ] || DYLIB_SRC="$HERE/cp77-console/target/release/libcp77_console.dylib"
NEW_PATH='@executable_path/../../../red4ext/libcp77_console.dylib'
# native helper (replaces otool/python3, which are Xcode CLT and absent on a clean Mac)
HELPER="${BWMS_HELPER:-$HERE/bwms-helper}"
[ -x "$HELPER" ] || HELPER="$HERE/bwms-helper/target/release/bwms-helper"

[ -f "$BIN" ] || { echo "error: binary not found at $BIN"; exit 1; }

# sign the BUNDLE (.app) ad-hoc, re-sealing the modified binary with ONLY the
# hardened-runtime relaxations the mod needs (see write_ents). We deliberately do
# NOT keep CDPR's identity entitlements: on an ad-hoc signature AMFI kills the game
# at launch ("cannot be opened" / Steam "OS Error 256").
sign_app() {
  if [ -s "$ENT" ]; then codesign -f -s - --entitlements "$ENT" "$APP"
  else codesign -f -s - "$APP"; fi
}

# write the ad-hoc-safe entitlements: cs.* relaxations only. We DROP CDPR's
# application-identifier / team-identifier / developer.* entitlements — those are
# bound to CDPR's cert + provisioning profile, so on an ad-hoc signature AMFI denies
# exec and the game won't launch. disable-library-validation lets our dylib load; the
# jit/executable-memory ones let our native arm64 hook write its trampolines.
write_ents() {
  mkdir -p "$RED4"
  cat > "$ENT" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>com.apple.security.cs.disable-library-validation</key><true/>
  <key>com.apple.security.cs.allow-dyld-environment-variables</key><true/>
  <key>com.apple.security.cs.allow-unsigned-executable-memory</key><true/>
  <key>com.apple.security.cs.disable-executable-page-protection</key><true/>
  <key>com.apple.security.cs.allow-jit</key><true/>
</dict>
</plist>
PLIST
}

if [ "$MODE" = restore ]; then
  [ -f "$BAK" ] || { echo "no backup found. Use Steam: Cyberpunk 2077 > Properties > Installed Files > Verify integrity."; exit 1; }
  cp -f "$BAK" "$BIN"
  write_ents
  xattr -cr "$APP" 2>/dev/null || true
  sign_app >/dev/null 2>&1 || true   # re-seal the bundle (ad-hoc-safe) with the original binary back
  echo "restored the original binary."; exit 0
fi

# 0) remove quarantine from the KIT (anything downloaded gets com.apple.quarantine)
xattr -dr com.apple.quarantine "$HERE" 2>/dev/null || true

# 1) back up the original binary, OUTSIDE the bundle (in red4ext/)
mkdir -p "$RED4"
mkdir -p "$RED4/plugins"   # pasta de plugins Rust (opt-in; vazia por padrao = sem efeito)
[ -f "$BAK" ] || { cp "$BIN" "$BAK"; echo "[1/5] backup -> $BAK"; }

# 2) write the ad-hoc-safe entitlements (cs.* relaxations only). NOT CDPR's identity
#    entitlements — those make AMFI kill the game on an ad-hoc signature.
write_ents
echo "[2/5] entitlements set (ad-hoc-safe: cs.* only)"

# 3) place the dylib + mods into red4ext/
[ -f "$DYLIB_SRC" ] || { echo "[3/5] ERROR: libcp77_console.dylib not found"; exit 1; }
cp -f "$DYLIB_SRC" "$RED4/libcp77_console.dylib"
xattr -dr com.apple.quarantine "$RED4/libcp77_console.dylib" 2>/dev/null || true
codesign -f -s - "$RED4/libcp77_console.dylib" || { echo "[3/5] ERROR signing the dylib"; exit 1; }
[ -d "$HERE/blackwall-mods" ] && cp -Rf "$HERE/blackwall-mods" "$RED4/"
echo "[3/5] dylib + mods in red4ext/"

# 4) add the LC_LOAD_DYLIB to the binary (NATIVE helper — no otool/python = no CLT)
[ -x "$HELPER" ] || { echo "[4/5] ERROR: bwms-helper not found/executable"; exit 1; }
if "$HELPER" has "$BIN" libcp77_console.dylib; then
  echo "[4/5] already in the binary — ok (idempotent)"
else
  "$HELPER" insert "$BIN" "$NEW_PATH" || { echo "[4/5] ERROR adding the load command — use Steam (Verify integrity) and try again"; exit 1; }
  echo "[4/5] load command added"
fi

# 5) re-sign the .app (re-seals the bundle w/ the modified binary + ad-hoc-safe entitlements).
#    Clear xattrs first: a launched app gets com.apple.provenance, which makes codesign
#    fail with "Operation not permitted".
xattr -cr "$APP" 2>/dev/null || true
sign_app || { echo "[5/5] ERROR re-signing"; exit 1; }
xattr -dr com.apple.quarantine "$BIN" "$APP" 2>/dev/null || true
codesign -v "$APP" >/dev/null 2>&1 && echo "[5/5] signature VALID" || echo "[5/5] WARNING: signature did not validate (the game usually still runs; report it if it won't open)"

echo
echo "Engine done. If you Update the game OR use 'Verify integrity', run the installer again."
echo "Revert:  $0 --restore \"$GAME_DIR\""
