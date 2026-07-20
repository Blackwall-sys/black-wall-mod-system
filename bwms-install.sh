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
# -f garante FILE (no layout dev, $HERE/bwms-helper é o DIRETÓRIO do crate e -x passa
# pra dir → cairia errado). Se não for arquivo executável, usa o binário compilado.
{ [ -f "$HELPER" ] && [ -x "$HELPER" ]; } || HELPER="$HERE/bwms-helper/target/release/bwms-helper"

[ -f "$BIN" ] || { echo "error: binary not found at $BIN"; exit 1; }

# Bundle id read ONCE from the .app's own Info.plist — authoritative for whatever store this
# install is (Steam/GOG/Epic). Reused by the GOG preflight, the defaults bake (5b) and the GOG
# warning below, instead of re-reading it three times.
BUNDLE_ID="$(defaults read "$APP/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
# The native offsets in the bwms dylib only cover the supported build. On the supported build,
# CFBundleShortVersionString is "2.3.1" (the project's "2.31").
SUPPORTED_VER="2.3.1"

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

  # Ship PURE (2026-07-12): every install/reinstall starts at boot-skip level 0 (off),
  # no matter what a previous version/test/reinstall left behind in $HOME. The user turns
  # it on themselves from Settings > Mods > Cheats > "Skip boot" — never inherited silently.
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

if [ "$MODE" = restore ]; then
  [ -f "$BAK" ] || { echo "no backup found. Use Steam: Cyberpunk 2077 > Properties > Installed Files > Verify integrity."; exit 1; }
  cp -f "$BAK" "$BIN"
  write_ents
  xattr -cr "$APP" 2>/dev/null || true
  sign_app >/dev/null 2>&1 || true   # re-seal the bundle (ad-hoc-safe) with the original binary back
  # restore the redscript cache (remove our Cheats tab) + drop our .reds
  CACHE="$GAME_DIR/r6/cache/final.redscripts"
  [ -f "$CACHE.bwms-bak" ] && cp -f "$CACHE.bwms-bak" "$CACHE"
  rm -rf "$GAME_DIR/r6/scripts/blackwall-mods"
  echo "restored the original binary + redscript cache."; exit 0
fi

# 0) remove quarantine from the KIT (anything downloaded gets com.apple.quarantine)
xattr -dr com.apple.quarantine "$HERE" 2>/dev/null || true

# 1) back up the original binary, OUTSIDE the bundle (in red4ext/)
mkdir -p "$RED4"
mkdir -p "$RED4/plugins"   # pasta de plugins Rust (opt-in; vazia por padrao = sem efeito)
[ -f "$BAK" ] || { cp "$BIN" "$BAK"; echo "[1/6] backup -> $BAK"; }

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
sign_app || { echo "[5/6] ERROR re-signing"; exit 1; }
xattr -dr com.apple.quarantine "$BIN" "$APP" 2>/dev/null || true
# Belt-and-suspenders (2.3): the insert wrote the binary via a temp file + rename. If anything
# ever left it without the exec bit (a 0644 temp), launchd refuses to spawn it ("spawn failed",
# errno 111). Force +x and confirm it, unconditionally.
chmod +x "$BIN" 2>/dev/null || true
[ -x "$BIN" ] || { echo "[5/6] ERROR: the game binary is not executable (chmod +x failed) — the game can't launch."; exit 1; }
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
  cp -f "$REDS_SRC/blackwall-mods/"*.reds "$DEST/" 2>/dev/null || true
  CACHE="$GAME_DIR/r6/cache/final.redscripts"
  [ -f "$CACHE" ] && [ ! -f "$CACHE.bwms-bak" ] && cp "$CACHE" "$CACHE.bwms-bak"
  xattr -dr com.apple.quarantine "$REDS_SRC/tools/scc" "$REDS_SRC/tools/libscc_lib.dylib" 2>/dev/null || true
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
