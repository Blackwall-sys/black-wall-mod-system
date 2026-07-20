#!/usr/bin/env bash
# build-core.sh — builds the 0% Lua core dylib AND applies the symtab-align fix.
#
# WHY: on macOS 27 the linker emits the no-`lua` build with the LC_SYMTAB string pool at a
# non-8-aligned file offset (mod8=4), and dyld refuses to load it ("mis-aligned LINKEDIT string
# pool") → the game won't launch. fix-symtab-align.py inserts the exact padding before the
# string table to realign it. ALWAYS build the shipped core through this script, not bare cargo.
#
# Usage:  ./build-core.sh              (core, 0% Lua)
#         ./build-core.sh --features lua   (CET-mod gateway; already aligned, fix is a no-op)
set -euo pipefail
cd "$(dirname "$0")"
# TRACELESS: remapeia os prefixos de path do build (HOME + RAIZ do projeto) p/ que NENHUM caminho
# real (/Users/<user>, /Volumes/.../mods-research) vaze no binário via metadata de panic/debug —
# inclusive das DEPS locais (bwms-hashes/bwms-core, que ficam FORA do $HOME). Dinâmico (sem hardcode
# do path do dono). Sobrepõe qualquer CARGO_ENCODED_RUSTFLAGS do ambiente do chamador.
ROOT="$(cd .. && pwd)"
US=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${HOME}=.${US}--remap-path-prefix=${ROOT}=."
cargo build --release "$@"
D=target/release/libcp77_console.dylib
codesign --remove-signature "$D" 2>/dev/null || true
python3 fix-symtab-align.py "$D"
codesign -s - --force "$D"
m=$(otool -l "$D" 2>/dev/null | awk '/cmd LC_SYMTAB/{x=1} x&&/stroff/{print ($2%8); exit}')
echo "core: $D  (stroff mod8=$m — 0=loadable, $(nm "$D" 2>/dev/null | grep -ciE 'lua_|luaL_|lj_') símbolos lua)"
