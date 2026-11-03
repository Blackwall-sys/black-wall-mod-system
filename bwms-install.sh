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
# -f garante FILE (-x passa pra diretório). No pacote o helper é o arquivo ao lado deste script; no layout dev é o
# 2º binário do crate bwms, compilado com `cargo build --profile helper --bin bwms-helper` dentro de bwms/.
{ [ -f "$HELPER" ] && [ -x "$HELPER" ]; } || HELPER="$HERE/bwms/target/helper/bwms-helper"

[ -f "$BIN" ] || { echo "error: binary not found at $BIN"; exit 1; }

# Bundle id read ONCE from the .app's own Info.plist — authoritative for whatever store this
# install is (Steam/GOG/Epic). Reused by the GOG preflight, the defaults bake (5b) and the GOG
# warning below, instead of re-reading it three times.
BUNDLE_ID="$(defaults read "$APP/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
# The native offsets in the bwms dylib only cover the supported build. On the supported build,
# CFBundleShortVersionString is "2.5.0".
SUPPORTED_VER="2.5.0"

# GOG Galaxy running: it verifies/repairs the game on launch and puts the ORIGINAL binary back,
# silently undoing this install. Steam does not do that, which is why only GOG users saw "it worked
# and then stopped working". Warn while it can still be acted on.
if [ "$MODE" = install ] && { pgrep -x "GOG Galaxy" >/dev/null 2>&1 || pgrep -f "GalaxyClient" >/dev/null 2>&1; }; then
  echo "NOTE: GOG Galaxy is running. It repairs the game on launch and will undo this install."
  echo "  Either quit Galaxy and launch Cyberpunk2077.app directly (it runs fully offline),"
  echo "  or in Galaxy: Cyberpunk 2077 > Manage installation > Configure > Features >"
  echo "  turn OFF automatic updates. Never use Verify/Repair after installing BWMS."
  echo
fi

# ---------------------------------------------------------------- build fingerprint gate
# Why this exists: a 2026-09-04 crash report (BWMS 0.1.4-beta on the EPIC GAMES build) showed the
# installer finishing every step — launch test included — and the game then dying with SIGTRAP on the
# GameThread at first launch. The installer had no way to tell one store's binary from another's, and
# the bwms dylib hooks the engine at ABSOLUTE addresses taken from the build it was reverse
# engineered on. On a different build those addresses land in the middle of other functions.
#
# The check: read 8 bytes at each address in bwms-build-conhecida.txt (generated from the supported
# build) straight out of the user's binary and compare. Same build = every line matches. Different
# build = almost none do. This runs BEFORE anything is touched, so a refusal leaves the game intact.
# No pacote o arquivo vem ao lado deste script (engine/); no repositório, em dist/files/.
FINGERPRINT=""
for cand in "$HERE/bwms-build-conhecida.txt" "$HERE/files/bwms-build-conhecida.txt" \
            "$HERE/dist/files/bwms-build-conhecida.txt" "$HERE/../files/bwms-build-conhecida.txt"; do
  [ -f "$cand" ] && FINGERPRINT="$cand" && break
done
# Só no caminho de INSTALAÇÃO. Desinstalar tem que funcionar SEMPRE — e principalmente numa build
# que este gate recusa, que é exatamente a do usuário que precisa voltar atrás.
if [ "$MODE" = install ] && [ -f "$FINGERPRINT" ]; then
  # One fingerprint per supported store build (Steam; GOG next to it as bwms-build-conhecida-gog.txt): the build is supported when EVERY
  # checkpoint of one of them matches.
  fp_ok=0; fp_total=0; fp_loja=""
  for fp in "$FINGERPRINT" "${FINGERPRINT%.txt}-gog.txt"; do
    [ -f "$fp" ] || continue
    ok=0; total=0
    # __TEXT is mapped at 0x100000000 with file offset 0 on this game, so file offset = vmaddr - base.
    while read -r vm esperado _; do
      case "$vm" in \#*|"") continue ;; esac
      total=$((total + 1))
      off=$(( vm - 0x100000000 ))
      lido=$(dd if="$BIN" bs=1 skip="$off" count=8 2>/dev/null | xxd -p 2>/dev/null)
      [ "$lido" = "$esperado" ] && ok=$((ok + 1))
    done < "$fp"
    if [ "$total" -gt 0 ] && [ "$ok" -eq "$total" ]; then
      fp_ok=$ok; fp_total=$total; fp_loja=Steam
      [ "$fp" != "$FINGERPRINT" ] && fp_loja=GOG
      break
    fi
    [ "$ok" -ge "$fp_ok" ] && { fp_ok=$ok; fp_total=$total; }
  done
  if [ "$fp_total" -gt 0 ] && [ -z "$fp_loja" ]; then
    echo "───────────────────────────────────────────────────────────────"
    echo "STOPPED — this is not the game build BWMS was built against."
    echo "  fingerprint: $fp_ok of $fp_total checkpoints matched"
    echo "  store:       ${BUNDLE_ID:-unknown}"
    echo "  version:     $(defaults read "$APP/Contents/Info" CFBundleShortVersionString 2>/dev/null || echo unknown) (supported: $SUPPORTED_VER)"
    echo
    echo "BWMS hooks the engine at fixed addresses measured on Cyberpunk 2077 $SUPPORTED_VER for macOS"
    echo "(Steam and GOG). On another build those addresses point somewhere else, and the game dies with"
    echo "SIGTRAP the moment it starts — which is exactly what happened to an Epic Games install"
    echo "on 2026-09-04. Nothing was modified: your game is untouched."
    echo
    echo "If your game IS $SUPPORTED_VER and you still see this, the binary may already be patched by"
    echo "another tool, or the install is incomplete — verify/repair it in your launcher first."
    echo "To install anyway and accept a likely crash:  BWMS_IGNORE_BUILD_CHECK=1 $0 \"$GAME_DIR\""
    echo "───────────────────────────────────────────────────────────────"
    [ "${BWMS_IGNORE_BUILD_CHECK:-0}" = "1" ] || exit 1
    echo "(BWMS_IGNORE_BUILD_CHECK=1 — going ahead at your own risk)"
  else
    echo "[0/6] build check: $fp_ok/$fp_total checkpoints match the supported build ($fp_loja $SUPPORTED_VER)"
  fi
fi

# ---------------------------------------------------------------- third-party framework gate
# Decision of 2026-10-27: another script framework in the game, as a plugin (red4ext/plugins/<name>/) or as scripts
# (r6/scripts/<name>/, the Windows copy included), fights BWMS over the same script classes and hooks. Install refuses until it is
# removed; uninstall always works.
if [ "$MODE" = install ]; then
  terceiros=""
  for nome in TweakXL ArchiveXL Codeware; do
    for onde in "red4ext/plugins/$nome" "r6/scripts/$nome"; do
      [ -e "$GAME_DIR/$onde" ] && terceiros="$terceiros  $GAME_DIR/$onde\n"
    done
  done
  if [ -n "$terceiros" ]; then
    echo "───────────────────────────────────────────────────────────────"
    echo "STOPPED — another mod framework is installed in this game:"
    printf "%b" "$terceiros"
    echo
    echo "It and BWMS fight over the same classes and hooks in the same game: scripts stop compiling or the game crashes."
    echo "Move the folders above to the Trash and run the installer again. Nothing was modified: your game is untouched."
    echo "───────────────────────────────────────────────────────────────"
    exit 1
  fi
fi

# GOG-specific preflight (2026-07-12): a genuine Galaxy install always writes goggame-*.info/
# .hashdb next to the game folder. A copy missing these (manual copy, interrupted install, etc.)
# can be internally broken in ways unrelated to bwms — CDPR's own binary asserts/crashes on launch
# even 100% vanilla (confirmed by testing: EXC_BREAKPOINT deep in the game's own code, zero bwms
# involved). Warn here so the symptom isn't mistaken for a bwms bug — patching a broken install
# doesn't make it MORE broken, but it also won't fix the underlying issue.
if [ "$MODE" = "install" ]; then
  # Full Disk Access up-front probe (2.8): the LC_LOAD insert (step 4) and re-sign (step 5) both
  # write INSIDE the .app bundle. On a Mac without Full Disk Access — common on GOG copies and on
  # ANY game kept on an EXTERNAL disk — macOS App Management blocks that write with EPERM. Probe it
  # NOW with a throwaway file so we fail fast with instructions, instead of dying mid-bake.
  PROBE="$APP/Contents/MacOS/.bwms-fda-probe"
  if ! ( : > "$PROBE" ) 2>/dev/null; then
    echo "============================================================"
    echo "  CAN'T WRITE INSIDE THE GAME — Full Disk Access needed"
    echo "============================================================"
    echo "macOS is blocking writes inside Cyberpunk2077.app (App Management protection)."
    echo "This is common on GOG installs and on games kept on an EXTERNAL disk."
    echo "To fix:"
    echo "  1) System Settings > Privacy & Security > Full Disk Access > enable Terminal"
    echo "  2) FULLY QUIT Terminal (Cmd+Q, not just the window)"
    echo "  3) reopen Terminal and run this installer again"
    exit 1
  fi
  rm -f "$PROBE" 2>/dev/null || true

  # Partial/previous bake detection (2.8): a crashed insert can leave the helper's temp file
  # behind. That's a broken-in-progress bake, not a clean state — clear it and, if a backup
  # exists, point the user at --restore so a retry starts from a known-good binary rather than
  # the misleading "already in the binary".
  if [ -f "$BIN.bwms-tmp" ]; then
    echo "NOTE: found leftover from an interrupted install ($BIN.bwms-tmp) — cleaning it up."
    rm -f "$BIN.bwms-tmp" 2>/dev/null || true
    [ -f "$BAK" ] && echo "  If the game won't launch, restore first:  $0 --restore \"$GAME_DIR\""
  fi

  # Game-version detect/warn (2.6): the dylib's native offsets only cover the supported build.
  # Bake anyway (non-fatal) but WARN loudly if the version differs, so a boot failure on an
  # unsupported build isn't mistaken for a bwms bug.
  GAME_VER="$(defaults read "$APP/Contents/Info" CFBundleShortVersionString 2>/dev/null || true)"
  if [ -n "$GAME_VER" ] && [ "$GAME_VER" != "$SUPPORTED_VER" ]; then
    echo "!!==========================================================!!"
    echo "  WARNING: unsupported game version detected: $GAME_VER"
    echo "  bwms is built for Cyberpunk 2077 $SUPPORTED_VER only. The native"
    echo "  offsets won't match another build — the game may crash on boot."
    echo "  Installing anyway; if it won't launch, this is the likely cause."
    echo "  (Revert with:  $0 --restore \"$GAME_DIR\")"
    echo "!!==========================================================!!"
  fi

  case "$BUNDLE_ID" in
    *.gog)
      if ! ls "$GAME_DIR/.."/goggame-*.info >/dev/null 2>&1 && ! ls "$GAME_DIR"/goggame-*.info >/dev/null 2>&1; then
        echo "WARNING: this looks like an incomplete GOG install (missing goggame-*.info/.hashdb,"
        echo "the files GOG Galaxy normally writes next to the game). If the game doesn't even"
        echo "launch vanilla, use Galaxy's Verify/Repair (or reinstall) BEFORE installing bwms —"
        echo "patching an already-broken install won't fix that underlying problem."
      fi
      ;;
  esac

  # The boot level lives only in red4ext/bwms.toml (written below with "To gameplay" on a fresh install; the in-game
  # "Skip boot" selector rewrites it). Dotfile markers a previous version or test left in $HOME must not override that.
  rm -f "$HOME/.bwms-skipintro" "$HOME/.bwms-fire-start" "$HOME/.bwms-autocontinue" "$HOME/.bwms-boot-attempt"
  # Also clear the /tmp session markers (2.7) so "ship pure" is real — these are the per-boot
  # counterparts of the ~/.bwms-* markers above and must not carry across a fresh install either.
  rm -f /tmp/bwms-skipintro /tmp/bwms-fire-start /tmp/bwms-autocontinue /tmp/bwms-boot-attempt
fi

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

# archive/Mac/mod/BWMS-Mods/ is BWMS's own folder (the runtime mirrors Mods/ into it, and the game loads every .archive there), so it goes
# whole, by its exact path: rm -R does not follow links inside it, and a link in its place goes as a link, its target untouched. Anything
# else in archive/Mac/mod belongs to the user.
remover_pasta_nativa() {
  rm -rf "$GAME_DIR/archive/Mac/mod/BWMS-Mods"
}

if [ "$MODE" = restore ]; then
  # A binary without our load command is already the store's own file (a game update or "Verify integrity" put it back): the backup may
  # be from another build and would put an old binary over a new one, so only bwms's own files go.
  if [ -x "$HELPER" ] && "$HELPER" info "$BIN" >/dev/null 2>&1 && ! "$HELPER" has "$BIN" libcp77_console.dylib; then
    rm -rf "$GAME_DIR/r6/scripts/blackwall-mods"
    rm -f "$GAME_DIR/r6/config/redsUserHints/bwms-dicas.toml" "$GAME_DIR/r6/config/redsUserHints/bwms.toml"
    remover_pasta_nativa
    echo "the game binary is already the store's own (no bwms load command): nothing to put back; bwms scripts removed."; exit 0
  fi
  [ -f "$BAK" ] || { echo "no backup found. Use Steam: Cyberpunk 2077 > Properties > Installed Files > Verify integrity."; exit 1; }
  cp -f "$BAK" "$BIN"
  write_ents
  xattr -cr "$APP" 2>/dev/null || true
  sign_app >/dev/null 2>&1 || true   # re-seal the bundle (ad-hoc-safe) with the original binary back
  # restore the redscript cache (remove our Cheats tab) + drop our .reds
  CACHE="$GAME_DIR/r6/cache/final.redscripts"
  [ -f "$CACHE.bwms-bak" ] && cp -f "$CACHE.bwms-bak" "$CACHE"
  rm -rf "$GAME_DIR/r6/scripts/blackwall-mods"
  # our compile-error hints, under their own name and under the kit's legacy name (bwms.toml)
  rm -f "$GAME_DIR/r6/config/redsUserHints/bwms-dicas.toml" "$GAME_DIR/r6/config/redsUserHints/bwms.toml"
  remover_pasta_nativa
  echo "restored the original binary + redscript cache."; exit 0
fi

# Undo everything and leave the game exactly as it was. Used by the steps AFTER the binary has
# already been touched: until 2026-09-20 a failure there just exited, leaving the binary with the
# load command but a broken signature — the game would not open and the user had no idea why.
abortar_restaurando() {
  echo
  echo "$1"
  if [ -f "$BAK" ]; then
    cp -f "$BAK" "$BIN" 2>/dev/null && chmod +x "$BIN" 2>/dev/null
    xattr -cr "$APP" 2>/dev/null || true
    codesign -f -s - "$APP" >/dev/null 2>&1 || codesign -f -s - "$BIN" >/dev/null 2>&1 || true
    CACHE_R="$GAME_DIR/r6/cache/final.redscripts"
    [ -f "$CACHE_R.bwms-bak" ] && cp -f "$CACHE_R.bwms-bak" "$CACHE_R" 2>/dev/null
    rm -rf "$GAME_DIR/r6/scripts/blackwall-mods" 2>/dev/null
    echo "  ROLLED BACK: the original binary and the redscript cache are back in place."
    echo "  Your game is exactly as it was before this installer ran."
  else
    echo "  (no backup found to roll back to — run:  $0 --restore \"$GAME_DIR\")"
  fi
  exit 1
}

# 0) remove quarantine from the KIT (anything downloaded gets com.apple.quarantine)
xattr -dr com.apple.quarantine "$HERE" 2>/dev/null || true

# 1) back up the original binary, OUTSIDE the bundle (in red4ext/)
mkdir -p "$RED4"
mkdir -p "$RED4/plugins"   # pasta de plugins Rust (opt-in; vazia por padrao = sem efeito)
# A binary without our load command is the store's own file for the build installed now (after a game update or "Verify integrity"
# the old backup is from another build), so it replaces the backup; one that already has it keeps the backup taken before it was patched.
if [ -x "$HELPER" ] && "$HELPER" info "$BIN" >/dev/null 2>&1 && ! "$HELPER" has "$BIN" libcp77_console.dylib; then
  cp "$BIN" "$BAK"; echo "[1/6] backup -> $BAK"
elif [ ! -f "$BAK" ]; then
  cp "$BIN" "$BAK"; echo "[1/6] backup -> $BAK"
fi

# 2) write the ad-hoc-safe entitlements (cs.* relaxations only). NOT CDPR's identity
#    entitlements — those make AMFI kill the game on an ad-hoc signature.
write_ents
echo "[2/6] entitlements set (ad-hoc-safe: cs.* only)"

# 3) place the dylib + mods into red4ext/
[ -f "$DYLIB_SRC" ] || { echo "[3/6] ERROR: libcp77_console.dylib not found"; exit 1; }
cp -f "$DYLIB_SRC" "$RED4/libcp77_console.dylib"
xattr -dr com.apple.quarantine "$RED4/libcp77_console.dylib" 2>/dev/null || true
codesign -f -s - "$RED4/libcp77_console.dylib" || { echo "[3/6] ERROR signing the dylib"; exit 1; }
[ -d "$HERE/blackwall-mods" ] && cp -Rf "$HERE/blackwall-mods" "$RED4/"
# Which game record each record inherits from (the compiled TweakDB does not keep it): without it a mod's change to a game record does
# not reach the records derived from it.
[ -f "$HERE/bwms-heranca.yaml" ] && cp -f "$HERE/bwms-heranca.yaml" "$RED4/bwms-heranca.yaml"
# What BWMS does at boot. Written once, with the defaults and a comment per option — without this
# file nobody finds out the options exist: until 2026-09-20 the intro skip, the splash and the
# auto-continue all hung on HIDDEN dotfiles in the home folder that this installer never created,
# so a fresh install looked exactly like no mod at all.
if [ ! -f "$RED4/bwms.toml" ]; then
  cat > "$RED4/bwms.toml" <<'TOML'
# BWMS — what the mod does when the game starts.
# Delete a line to go back to its default; delete the file to go back to all of them.

# Skip the opening logos and the "PRESS SPACE" screen (the black loading screen stays — that part is disk streaming).
pular_intro = true

# Draw the BWMS screen over the black loading screen, with the progress bar.
splash = true

# Load your last save by itself, without going through the main menu (no save yet = the boot stops at the menu).
# In the game: Settings > Mods > Blackwall Cheats > "Skip boot" (Off / To the menu / To gameplay).
continuar_automatico = true
TOML
  echo "[3/6] bwms.toml written (boot options)"
fi
echo "[3/6] dylib + mods in red4ext/"

# 4) add the LC_LOAD_DYLIB to the binary (NATIVE helper — no otool/python = no CLT)
[ -x "$HELPER" ] || { echo "[4/6] ERROR: bwms-helper not found/executable"; exit 1; }
if "$HELPER" has "$BIN" libcp77_console.dylib; then
  echo "[4/6] already in the binary — ok (idempotent)"
else
  if ! "$HELPER" insert "$BIN" "$NEW_PATH"; then
    echo
    echo "[4/6] ERROR adding the load command."
    echo "  If it said 'Operation not permitted': macOS blocks writing INSIDE the .app"
    echo "  bundle (App Management protection). Grant your Terminal Full Disk Access, then"
    echo "  FULLY QUIT it (Cmd+Q, not just the window) and run this again:"
    echo "    System Settings > Privacy & Security > Full Disk Access > enable Terminal"
    echo "  (Common on GOG/re-launched copies — Steam's first bake happens before the lock.)"
    echo "  If it said 'no room in the header': use Steam 'Verify integrity' and retry."
    exit 1
  fi
  echo "[4/6] load command added"
fi

# 4b) VERIFY the load command actually landed BEFORE compiling any .reds (step 6). The .reds
#     declare `native func` resolved by the dylib's RTTI at boot; if the load command is missing
#     but the reds compile, the engine asserts on an unresolved native (EXC_BREAKPOINT) at boot
#     with a BLACK SCREEN and no log — the documented root cause of GOG "installed, won't open,
#     no log" reports. Abort here so that state can never be reached (binary stays vanilla-launchable).
if "$HELPER" has "$BIN" libcp77_console.dylib; then
  echo "[4b/6] load command verified present"
else
  echo "[4b/6] ERROR: the dylib load command is NOT in the binary after the insert step."
  echo "  Aborting BEFORE compiling the redscript cheats — compiling them without the dylib"
  echo "  loaded makes the game assert on an unresolved native and crash on boot (black screen,"
  echo "  no log). The binary is unchanged/vanilla-launchable. Fix the insert step (usually Full"
  echo "  Disk Access — see the message above) and run this installer again."
  exit 1
fi

# 5) re-sign the .app (re-seals the bundle w/ the modified binary + ad-hoc-safe entitlements).
#    Clear xattrs first: a launched app gets com.apple.provenance, which makes codesign
#    fail with "Operation not permitted".
xattr -cr "$APP" 2>/dev/null || true
sign_app || abortar_restaurando "[5/6] ERROR re-signing the app."
xattr -dr com.apple.quarantine "$BIN" "$APP" 2>/dev/null || true
# Belt-and-suspenders (2.3): the insert wrote the binary via a temp file + rename. If anything
# ever left it without the exec bit (a 0644 temp), launchd refuses to spawn it ("spawn failed",
# errno 111). Force +x and confirm it, unconditionally.
chmod +x "$BIN" 2>/dev/null || true
[ -x "$BIN" ] || abortar_restaurando "[5/6] ERROR: the game binary is not executable (chmod +x failed)."
# NOTE (2.5): `codesign -v` checks the signature STRUCTURE only, not the launch policy — it can
# pass even when AMFI would SIGKILL the process at exec. Report "structure OK" here; the real
# launch check is the smoke-test in 5c below, not a misleading flat "VALID".
codesign -v "$APP" >/dev/null 2>&1 && echo "[5/6] signature structure OK (launch policy verified in 5c)" || echo "[5/6] WARNING: signature structure did not validate (the game usually still runs; report it if it won't open)"

# 5b) bake the macOS per-app defaults that keep the .app double-click path from HANGING at boot
#     (2.4). Two failure modes this prevents:
#       - the "reopen windows?" NSAlert (ApplePersistenceIgnoreState / NSQuitAlwaysKeepsWindows):
#         a MODAL alert on the main run-loop blocks the whole boot forever (the mis-diagnosed
#         "t=38s freeze"); nobody clicks it on an autonomous boot.
#       - App Nap (NSAppSleepDisabled): macOS throttles/freezes the game in the background,
#         stalling boot.
#     The .app double-click path can't inject env vars, so we set these on the app's preference
#     DOMAIN instead. Applied to this install's real bundle id (from Info.plist) AND both store
#     ids, so a machine with more than one copy is covered either way. $BUNDLE_ID is authoritative
#     — the store constants are extra coverage, correctness never depends on the guessed GOG id.
for BID in "$BUNDLE_ID" com.cdprojektred.cyberpunk.steam com.cdprojektred.cyberpunk.gog; do
  [ -n "$BID" ] || continue
  defaults write "$BID" ApplePersistenceIgnoreState -bool YES 2>/dev/null || true
  defaults write "$BID" NSQuitAlwaysKeepsWindows   -bool NO  2>/dev/null || true
  defaults write "$BID" NSAppSleepDisabled         -bool YES 2>/dev/null || true
done
echo "[5b/6] boot-hang defaults baked (NSAlert reopen-windows + App Nap disabled)"

# 5c) launch smoke-test (2.5): prove the re-signed binary actually PASSES the launch policy, not
#     just codesign's structure check. AMFI can SIGKILL an ad-hoc binary at exec even when
#     `codesign -v` is happy. Spawn it, give it a couple seconds to clear the exec gate, then
#     confirm it's still alive (or exited non-crash) and terminate it. A crash SIGNAL at exec
#     (SIGKILL=137) means it was blocked; a clean self-exit (e.g. store/DRM not running during
#     the test) is NOT a block. Set BWMS_NO_SMOKE=1 to skip (headless/CI).
if [ "${BWMS_NO_SMOKE:-}" != "1" ]; then
  # O teste abre o jogo sem compilador: a dylib compila os .reds na abertura (scripts/compilacao_no_lancamento.rs) com o
  # red4ext/scc de uma instalação anterior, e o SIGTERM daqui deixaria o scc órfão escrevendo o r6/cache junto com o do passo 6.
  for f in scc bwms-scc.txt; do [ -e "$RED4/$f" ] && mv -f "$RED4/$f" "$RED4/$f.teste-de-abertura"; done
  [ -d "$GAME_DIR/Mods" ] && MODS_ANTES_DO_TESTE=1 || MODS_ANTES_DO_TESTE=0
  (
    cd "$GAME_DIR" || exit 0
    # SteamNoOverlayUIDrawing keeps Steam's overlay from hooking this throwaway process.
    SteamNoOverlayUIDrawing=1 "./Cyberpunk2077.app/Contents/MacOS/Cyberpunk2077" >/dev/null 2>&1 &
    SMOKE_PID=$!
    # ~2.5s is well past the exec/AMFI gate (an AMFI SIGKILL is immediate) but far short of any
    # save-load, so terminating here is safe.
    for _ in 1 2 3 4 5; do kill -0 "$SMOKE_PID" 2>/dev/null || break; sleep 0.5; done
    if kill -0 "$SMOKE_PID" 2>/dev/null; then
      # still running after the exec gate → launch policy OK. Stop it (SIGTERM first; a SIGKILL
      # fallback is safe here because it never reached gameplay, so no save is in flight).
      kill -TERM "$SMOKE_PID" 2>/dev/null || true
      for _ in 1 2 3 4 5 6; do kill -0 "$SMOKE_PID" 2>/dev/null || break; sleep 0.5; done
      kill -0 "$SMOKE_PID" 2>/dev/null && kill -KILL "$SMOKE_PID" 2>/dev/null
      echo "[5c/6] launch test PASSED (survived exec — AMFI/signature allow it to run)"
    else
      wait "$SMOKE_PID" 2>/dev/null; RC=$?
      if [ "$RC" -ge 128 ]; then
        echo "[5c/6] WARNING: the game was BLOCKED at launch (killed by signal $((RC-128)))."
        echo "  A signature/AMFI policy killed it at exec, so codesign's 'structure OK' is not"
        echo "  enough here. Try: re-run this installer with Terminal granted Full Disk Access"
        echo "  (Cmd+Q Terminal first); if it persists, restore + reinstall:"
        echo "    $0 --restore \"$GAME_DIR\""
        echo "  (Non-fatal: the engine bake is done; this only flags the game may not open.)"
      else
        echo "[5c/6] launch test OK (passed exec; exited on its own, code $RC — e.g. store/DRM not running)"
      fi
    fi
  ) || true
  for f in scc bwms-scc.txt; do [ -e "$RED4/$f.teste-de-abertura" ] && mv -f "$RED4/$f.teste-de-abertura" "$RED4/$f"; done
  # Sem compilador, a abertura do teste deixa no relatório da Mods/ "o compilador do BWMS não foi achado; rode o instalador do BWMS de
  # novo", que o passo 6 desmente, e a falha cria a pasta Mods/ quando ela não existe. O relatório é regravado a cada abertura do jogo; a
  # pasta criada pelo teste sai só se ficou vazia.
  rm -f "$GAME_DIR/Mods/BWMS-RELATORIO.txt"
  [ "$MODS_ANTES_DO_TESTE" = 1 ] || rmdir "$GAME_DIR/Mods" 2>/dev/null || true
fi

# 5d) A 0.1.5 só põe mod no jogo pela pasta Mods/ e pelo mod.io: a biblioteca da 0.1.2–0.1.4 (BWMS/mods, BWMS/nexus) saiu. O helper leva
#     para Mods/<nome> cada mod que estava ligado nela (clone APFS, a origem fica; nome que já existe em Mods/ não é sobrescrito) e tira do
#     jogo o que ela pôs lá (.archive com o prefixo do BWMS em archive/Mac/content, r6/tweaks, r6/scripts) pela lista que ela gravou, nunca
#     por glob. Antes do passo 6: o scc compila r6/scripts sem as cópias da biblioteca, que a Mods/ agora traz. O resultado fica em
#     .cp77-mods/bwms-migracao-da-biblioteca.txt e entra no Mods/BWMS-RELATORIO.txt de cada abertura enquanto a pasta BWMS/ existir.
if [ -x "$HELPER" ]; then
  MIGRACAO="$("$HELPER" migrar "$GAME_DIR" 2>&1)"; RC_MIGRACAO=$?
  if [ -n "$MIGRACAO" ]; then
    echo "[5d/6] old BWMS library (BWMS/mods, BWMS/nexus) -> Mods folder:"
    printf '%s\n' "$MIGRACAO"
    [ "$RC_MIGRACAO" = 0 ] || echo "  Some items above need action; run this installer again after fixing them (the game works meanwhile)."
  fi
fi

# 6) redscript: deploy + compile the native Cheats tab (.reds). 0% Lua — the cheats are
#    redscript, not Lua. Non-fatal: a broken third-party .reds must not block the engine.
# ORDER MATTERS (2026-07-12): this runs LAST, only after the dylib is confirmed loadable.
# The .reds declare `native func` bound by our dylib's RTTI registration — if they compile
# while the dylib ISN'T in the binary (step 4 failed, e.g. missing Full Disk Access on GOG),
# the engine hits an unresolved-native assert and crashes on boot, before the main menu, with
# no visible error (confirmed by deliberately reproducing it: EXC_BREAKPOINT in the engine's
# native-function dispatcher, ~/Library/Logs/DiagnosticReports, nothing on screen). Doing this
# last means a step-4 failure leaves the game 100% vanilla and launchable — no orphaned natives.
# Portátil: acha o kit redscript (scc + .reds) ao lado do instalador (pacote) OU dentro
# do projeto (dev) — assim o install do DEV também compila os cheats (não fica tab morta).
REDS_SRC=""
for cand in "$HERE/redscript" "$HERE/dist/bwms-0.1.3/engine/redscript" "$HERE/dist/bwms-0.1.1/engine/redscript"; do
  [ -d "$cand/blackwall-mods" ] && [ -x "$cand/tools/scc" ] && REDS_SRC="$cand" && break
done
if [ -n "$REDS_SRC" ]; then
  DEST="$GAME_DIR/r6/scripts/blackwall-mods"
  mkdir -p "$DEST"
  # Files an earlier BWMS installed and this version no longer ships. The install copies file by file,
  # so without this list an old file stays behind on update: one of the files from
  # 0.1.3 declares the same `module Codeware.UI` classes as ponte-modulo-ui.reds, and no script
  # compiles while both are there. The list is explicit on purpose: other redscript mods may keep
  # their own files in this folder, and those are not ours to remove.
  for legado in \
      bwms-vector2-utils.reds \
      bwms-vector3-utils.reds \
      bwms-delaysystem-utils.reds \
      codeware-ui-customcontroller.reds \
      codeware-inkwidgethelper.reds \
      codeware-ui-popupbutton.reds \
      codeware-ui-inmenupopup.reds \
      codeware-ui-inmenupopupcontent.reds \
      codeware-ui-inmenupopupfooter.reds \
      codeware-ui-ingamepopup.reds \
      codeware-ui-ingamepopupheader.reds \
      codeware-ui-ingamepopupfooter.reds \
      codeware-ui-ingamepopupcontent.reds \
      codeware-ui-custompopup.reds \
      codeware-ui-buttons.reds \
      codeware-ui-buttonhintsex.reds \
      codeware-ui-buttonhintsmanager.reds \
      codeware-ui-textinput-caret.reds \
      codeware-ui-textinput-selection.reds \
      codeware-ui-textinput-textflow.reds \
      codeware-ui-textinput-textmeasurer.reds \
      codeware-ui-themecolors.reds \
      codeware-ui-virtualresolution.reds \
      codeware-inkwidgetref.reds \
      codeware-screenhelper.reds \
      codeware-player.reds \
      codeware-localization.reds \
      codeware-localization-package.reds \
      codeware-device-securitysystem.reds \
      bwms-module-scoped-probe.reds \
      bwms-puppetstate-mirror.reds \
      red4ext-scriptref-proof.reds \
      bwms-tppcam.reds \
      _bwms-variantlog-decl.reds \
      bwms-customization-apply.reds \
      bwms-transmog-apply.reds \
      codeware-igamesystem-vtdump.reds \
      codeware-igamesystem-hook-ongamerestored.reds \
      codeware-igamesystem-hook-onworldattached.reds \
      archivexl-35-fase.reds \
      archivexl-44-journal-hash.reds \
      archivexl-44-journal-arvore.reds \
      archivexl-44-mappin-cooked-count.reds \
      archivexl-47-customization-events.reds \
      archivexl-66-charactercustomizationhelper-haircolor.reds \
      archivexl-77-journalmanager-tracking.reds \
      archivexl-facade.reds \
      archivexl-puppetstate-armsdetect.reds \
      archivexl-puppetstatesystem.reds \
      callbacksystem-native.reds \
      codeware-100-inksystem-camadas.reds \
      codeware-100-inksystem-clipboard-longo.reds \
      codeware-100-inksystem-mundo.reds \
      codeware-115-textedit.reds \
      codeware-160-acessor.reds \
      codeware-17-resourceref.reds \
      codeware-17-resref-api.reds \
      codeware-20-security-reset.reds \
      codeware-39-onscreenentries.reds \
      codeware-44-traceresult-hit.reds \
      codeware-80-populationspawner.reds \
      codeware-80-spawner.reds \
      codeware-85-worldnode.reds \
      codeware-90-inkcomponent.reds \
      codeware-91-widget-fields.reds \
      codeware-96-worldwidget.reds \
      codeware-casts.reds \
      codeware-communitysystem.reds \
      codeware-communitywrapper.reds \
      codeware-componentwrapper.reds \
      codeware-depot-resref-ops.reds \
      codeware-districtresolver.reds \
      codeware-dynamicentityspec.reds \
      codeware-dynamicentitysystem.reds \
      codeware-enginetime.reds \
      codeware-entity-builder.reds \
      codeware-entitybuilderwrapper.reds \
      codeware-facade.reds \
      codeware-gameobject-addtag.reds \
      codeware-gameobject-tags.reds \
      codeware-inkcharacterevent.reds \
      codeware-inkcharactereventtype.reds \
      codeware-inklayerwrapper.reds \
      codeware-inksystem-clipboard.reds \
      codeware-inksystem-layers.reds \
      codeware-inkwidgetlibrary.reds \
      codeware-iplacedcomponent-worldtransform.reds \
      codeware-iserializable-postload.reds \
      codeware-jobqueue.reds \
      codeware-journalmanager.reds \
      codeware-localizationstring.reds \
      codeware-mappinsystem.reds \
      codeware-openworldtracker.reds \
      codeware-persistency-population.reds \
      codeware-playerspawnedhook.reds \
      codeware-questgraph.reds \
      codeware-quests-executenode.reds \
      codeware-reflection.reds \
      codeware-renderproxy.reds \
      codeware-resourcedepot.reds \
      codeware-resourcehelper.reds \
      codeware-resourcetoken.reds \
      codeware-scriptableservice-state.reds \
      codeware-scripting-stacktrace.reds \
      codeware-staticentityspec.reds \
      codeware-staticentitysystem.reds \
      codeware-systembyname.reds \
      codeware-targets-native.reds \
      codeware-ui-inkinputdisplaycontroller-fields.reds \
      codeware-ui-inkwidget-ext.reds \
      codeware-utils.reds \
      codeware-vehicleobject.reds \
      codeware-vehiclesystem.reds \
      codeware-visualcontroller.reds \
      codeware-visualcontrollercomponent.reds \
      codeware-wardrobesystem-forgetitem.reds \
      codeware-weathersystem.reds \
      codeware-widgetinputservice.reds \
      codeware-worldstatesystem-community.reds \
      codeware-worldstatesystem.reds \
      scriptableservice-native.reds \
      scriptabletweak-native.reds \
      tweakdbbatch.reds \
      tweakdbmanager.reds \
      tweakxl-facade.reds \
      tweakxl-scriptinterface.reds \
      ponte-codeware-ui.reds \
      bwms-mod-settings.reds \
      bwms-security-reset.reds \
      bwms-district.reds ; do
    [ -f "$DEST/$legado" ] && rm -f "$DEST/$legado" && echo "  · removed (from a previous version): $legado"
  done
  cp -f "$REDS_SRC/blackwall-mods/"*.reds "$DEST/" 2>/dev/null || true
  # Compile-error hints for mods written for Windows (the script compiler reads r6/config/redsUserHints/*.toml).
  # Their name is bwms-dicas.toml; bwms.toml is the runtime config in red4ext/. A kit that still ships them as
  # bwms.toml installs under the new name too, and the legacy copy in the game goes away so the compiler never
  # reads the same hints twice.
  rm -f "$GAME_DIR/r6/config/redsUserHints/bwms.toml"
  for dicas in "$REDS_SRC/redsUserHints/bwms-dicas.toml" "$REDS_SRC/redsUserHints/bwms.toml" "$HERE/dist/redsUserHints/bwms-dicas.toml"; do
    [ -f "$dicas" ] && mkdir -p "$GAME_DIR/r6/config/redsUserHints" && cp -f "$dicas" "$GAME_DIR/r6/config/redsUserHints/bwms-dicas.toml" && break
  done
  CACHE="$GAME_DIR/r6/cache/final.redscripts"
  # A cache without the scc signature is the game's own vanilla for the build installed now (after a game update the old backup is from
  # another build), so it replaces the backup; a compiled one is never the vanilla and keeps the backup taken before the first compile.
  if [ -f "$CACHE" ] && ! LC_ALL=C grep -qF '$REDSCRIPT_SIGNATURE' "$CACHE"; then
    cp "$CACHE" "$CACHE.bwms-bak"
  fi
  xattr -dr com.apple.quarantine "$REDS_SRC/tools/scc" "$REDS_SRC/tools/libscc_lib.dylib" 2>/dev/null || true
  # A `native func` declared in a .reds with no matching registration in the dylib makes the script
  # binder hit SIGTRAP at startup — the game closes with no message at all. That happens whenever the
  # two halves come from different versions (new .reds next to an old dylib, or the other way round).
  # Check it here, where it can still be fixed, instead of letting the user meet it as a silent crash.
  # The check is `bwms-helper natives`, the same one the release gate in dist/pack-bwms.sh runs: missing
  # names on stdout, exit 1 when any is missing, 2 when it could not check at all.
  faltando="$(BWMS_JOGO="$BIN" "$HELPER" natives "$RED4/libcp77_console.dylib" "$DEST")"
  case $? in
    0) ;;
    1)
      echo "[6/6] STOPPED: these natives are declared in the .reds but missing from the dylib:"
      printf '%s\n' "$faltando" | head -12 | sed 's/^/     /'
      echo "  The two halves are from different versions. Install the kit as a whole, or restore with:"
      echo "     $0 --restore \"$GAME_DIR\""
      echo "  (Installing like this would make the game close at startup with no message.)"
      exit 1 ;;
    *)
      echo "[6/6] STOPPED: the .reds could not be checked against the dylib (bwms-helper error above)."
      echo "  Use the bwms-helper that came with this installer: install the kit as a whole, or restore with:"
      echo "     $0 --restore \"$GAME_DIR\""
      exit 1 ;;
  esac
  # A dylib compila os scripts na abertura do jogo com o red4ext/scc (cp77-console, scripts/carregador.rs, scc). O scc e a
  # libscc_lib.dylib vão juntos para red4ext/ (o scc carrega a lib da própria pasta): a pasta do jogo fica, e o kit pode ser
  # apagado depois da instalação. O bwms-scc.txt guarda o caminho absoluto: o jogo não roda na pasta de onde o instalador foi chamado.
  if cp -f "$REDS_SRC/tools/scc" "$RED4/scc" && cp -f "$REDS_SRC/tools/libscc_lib.dylib" "$RED4/libscc_lib.dylib"; then
    xattr -d com.apple.quarantine "$RED4/scc" "$RED4/libscc_lib.dylib" 2>/dev/null || true
    printf '%s' "$(cd "$RED4" && pwd)/scc" > "$RED4/bwms-scc.txt"
  else
    echo "[6/6] WARNING: the script compiler was not copied to red4ext/ — .reds in the Mods folder will not compile when the game"
    echo "  starts. Free space on the game disk and run this installer again."
  fi
  if "$REDS_SRC/tools/scc" -compile "$GAME_DIR/r6/scripts" >/dev/null 2>&1; then
    echo "[6/6] redscript cheats compiled (Settings > Cheats)"
  else
    echo "[6/6] WARNING: redscript compile failed (another mod's .reds may be broken; the Cheats tab will be inactive until fixed)"
  fi
else
  echo "[6/6] (no redscript in the kit — skipping the Cheats tab)"
fi

echo
echo "Engine done. If you Update the game OR use 'Verify integrity', run the installer again."
echo "Revert:  $0 --restore \"$GAME_DIR\""

# GOG-specific warning, right at the moment it matters most: Galaxy's own "verify/repair" or
# auto-update silently reverts this exact patch (same class of problem as Steam's Verify integrity,
# but Galaxy does it more eagerly/automatically) — surfaced here so a GOG user sees it immediately
# after a successful install, not just buried in the README. Boxed (2.9) so it's impossible to
# miss: Galaxy REMOVES the very dylib we inserted, so this cannot be prevented in code. Reuses the
# $BUNDLE_ID read once at the top.
case "$BUNDLE_ID" in
  *.gog)
    echo
    echo "  ##############################################################"
    echo "  ##  GOG GALAXY WILL UNDO THIS INSTALL — PLEASE READ  ##"
    echo "  ##############################################################"
    echo "  GOG Galaxy re-verifies goggame-*.hashdb every time you press Play"
    echo "  and after any update, and REPAIRS the baked game binary — which"
    echo "  DELETES the exact dylib this installer inserted. There is no way"
    echo "  to prevent that from our side. To keep bwms working, do ALL of:"
    echo "    - Launch Cyberpunk2077.app DIRECTLY (double-click it), NOT via Galaxy."
    echo "    - Galaxy > Cyberpunk 2077 > Manage installation > Configure >"
    echo "      turn OFF 'Automatic updates'."
    echo "    - NEVER use Galaxy's 'Verify / Repair' on this game."
    echo "  If Galaxy ever repairs the game, just run this installer again."
    echo "  ##############################################################"
    ;;
esac
