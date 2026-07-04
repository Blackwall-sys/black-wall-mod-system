#!/bin/bash
# Build da dylib + realign do LINKEDIT + re-assina. SEMPRE usar isso pra buildar
# (o cargo build cru pode sair com o string pool desalinhado → o jogo recusa
# carregar a dylib). Uso: ./build.sh
set -e
cd "$(dirname "$0")"
# Remapeia o caminho de fonte embutido em panics (deps em ~/.cargo usam file!() com path
# absoluto) → não vaza /Users/<user> nem o caminho do projeto no binário público.
# CARGO_ENCODED_RUSTFLAGS (separador \x1f) é OBRIGATÓRIO: o projeto tem espaço no caminho
# (caminho com espaço) e RUSTFLAGS (separado por espaço) quebraria o flag.
PROJ="$(cd .. && pwd)"
SEP=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=$HOME/.cargo=${SEP}--remap-path-prefix=$HOME=${SEP}--remap-path-prefix=$PROJ="
unset RUSTFLAGS
cargo build --release "$@"
DY="target/release/libcp77_console.dylib"
# install name = @rpath (não o path absoluto do dev) — artefato público limpo; o
# LC_LOAD no binário do jogo continua @executable_path/... (correto).
install_name_tool -id '@rpath/libcp77_console.dylib' "$DY"
# strip símbolos LOCAIS/DWARF: remove os caminhos /Users/<user> embutidos pelo cargo (privacidade
# — vazava o nome de usuário no artefato público) e reduz o string-bloat que a heurística de AV
# pontua. Mantém os símbolos GLOBAIS exportados (on_load/cp77_*) que o dyld/loader precisa.
# ANTES do align (mexe no symtab) e do codesign (strip invalidaria a assinatura).
strip -x "$DY"
python3 align_linkedit.py "$DY"
codesign --remove-signature "$DY" 2>/dev/null || true
codesign --force --sign - "$DY"
# valida que o dyld aceita antes de relançar
if python3 -c "import ctypes,sys; ctypes.CDLL(sys.argv[1])" "$DY" 2>/dev/null; then
  echo "build + realign + sign OK (dlopen valida)"
else
  echo "AVISO: dlopen ainda recusa a dylib!" >&2
  exit 1
fi
