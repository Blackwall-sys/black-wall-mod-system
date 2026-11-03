#!/usr/bin/env bash
# build-core.sh — builds the core dylib (0% Lua) AND applies the symtab-align fix.
#
# WHY: on macOS 27 the linker emits the LC_SYMTAB string pool at a non-8-aligned file offset (mod8=4), and
# dyld refuses to load it ("mis-aligned LINKEDIT string pool") → the game won't launch. The Rust tool
# fix-symtab-align (src/bin) inserts the exact padding before the string table to realign it. ALWAYS build
# the shipped core through this script, not bare cargo.
#
# Usage:  ./build-core.sh                              (public)
#         ./build-core.sh --features devtools,capture   (dev)
set -euo pipefail
cd "$(dirname "$0")"
# TRACELESS: remapeia os prefixos de path do build (HOME + RAIZ do projeto) p/ que NENHUM caminho
# real (/Users/<user>, <raiz-do-repo>) vaze no binário via metadata de panic/debug —
# inclusive da DEP local (bwms-primitives, que fica FORA do $HOME). Dinâmico (sem hardcode
# do path do dono). Sobrepõe qualquer CARGO_ENCODED_RUSTFLAGS do ambiente do chamador.
ROOT="$(cd .. && pwd)"
US=$'\x1f'
export CARGO_ENCODED_RUSTFLAGS="--remap-path-prefix=${HOME}=.${US}--remap-path-prefix=${ROOT}=."
# Tabelas de endereços por assinatura (RED4ext #447): refeitas do binário Steam 2.5.0 e dos endereços citados em src/, pra que todo
# endereço novo no código também resolva num build de outro layout. Sem o binário no disco, ficam as tabelas que já estão em dados/.
# Saem as duas a cada build: a inteira (todo src/, o dylib de dev) e a pública (só os arquivos que o build sem feature compila, pelo
# `#[cfg]` de cada `mod`; o `enderecos.rs` escolhe pela feature `devtools`).
# O binário é a cópia vanilla guardada em cp77-symbols/binarios, não o do jogo instalado: o instalado leva o LC_LOAD do BWMS e muda a
# cada update da Steam (o gerar_enderecos recusa o que não for o 2.5.0, mas a tabela tem de sair do build que o código cita).
STEAM_BIN="${CP77_STEAM_BIN:-$ROOT/cp77-symbols/binarios/Cyberpunk2077-2.5.0-steam.bin}"
if [ -f "$STEAM_BIN" ]; then
  (cd ../bwms-primitives && cargo run --release -q --example gerar_enderecos -- "$STEAM_BIN" ../cp77-console/src ../cp77-console/dados/enderecos-steam-2.5.0.bin ../cp77-console/dados/enderecos-steam-2.5.0-publico.bin)
fi
# GOG 2.5.0 (a mesma versão compilada à parte, funções em outra ordem): tradução fixa Steam -> GOG de cada tabela, resolvida contra a cópia
# vanilla do binário GOG, mais os pares manuais (corpo de template repetido) que aquela tabela não resolveu. Sem o binário, ficam as de dados/.
GOG_BIN="${CP77_GOG_BIN:-$ROOT/cp77-symbols/binarios/Cyberpunk2077-2.5.0-gog.bin}"
if [ -f "$GOG_BIN" ]; then
  T=$(mktemp -d)
  for v in "" "-publico"; do
    (cd ../bwms-primitives && cargo run --release -q --example resolver_em_outro_build -- "../cp77-console/dados/enderecos-steam-2.5.0$v.bin" "$GOG_BIN" "$T/sem" "$T/pares") >/dev/null
    grep -v '^#' dados/enderecos-gog-2.5.0-manuais.txt | while read -r st gog; do if grep -qx "$st" "$T/sem"; then echo "$st $gog"; fi; done > "$T/manuais"
    { echo "# Steam 2.5.0 -> GOG 2.5.0, gerado pelo build-core.sh: resolver_em_outro_build contra dados/enderecos-steam-2.5.0$v.bin e o binário GOG"
      echo "# vanilla, mais dados/enderecos-gog-2.5.0-manuais.txt. Não editar à mão."
      { cat "$T/pares"; echo; cat "$T/manuais"; } | grep -v '^$' | sort -u; } > "dados/enderecos-gog-2.5.0$v.txt"
    echo "gog$v: $(grep -vc '^#' "dados/enderecos-gog-2.5.0$v.txt") pares; sem par: $(grep -vxF -f <(cut -d' ' -f1 "$T/manuais") "$T/sem" | tr '\n' ' ')"
  done
  rm -rf "$T"
fi
# O build PÚBLICO (sem `devtools`) usa o perfil `publico`: o mesmo opt-level do `release` do dev, com
# pasta de saída própria (a medida que decidiu o opt-level está no comentário do perfil no Cargo.toml).
PERFIL=publico
DIR=publico
DEV=0
case " $* " in *" devtools "*|*"devtools,"*|*",devtools"*) PERFIL=release; DIR=release; DEV=1 ;; esac

# Qualquer `panic!` — e basta uma indexação de slice — arrasta o symbolizer do std (gimli, addr2line,
# rustc_demangle): 198 KB no dylib PÚBLICO, medido, e um cdylib sem nenhum panic não leva nada disso.
# Isso só sai recompilando o `core`/`std` com `panic=immediate-abort`, que é nightly. Então: se o
# nightly com `rust-src` estiver instalado, o build público sai menor; se não estiver, sai pelo estável
# e a única diferença é o tamanho. Medido em 2026-10-16 na mesma árvore: `__text` de 1.634.348 B pelo
# nightly e 1.922.916 B pelo estável; o dylib que este roteiro entregou pelo nightly, com esse mesmo
# `__text`, tem 1.881.808 B. As flags ficam na LINHA DE COMANDO de propósito — `cargo-features` no
# Cargo.toml faria o cargo ESTÁVEL recusar o projeto inteiro.
NIGHTLY=""
if [ "$DEV" = "0" ] && rustup run nightly rustc --version >/dev/null 2>&1 \
   && [ -d "$(rustc +nightly --print sysroot 2>/dev/null)/lib/rustlib/src/rust/library" ]; then
  NIGHTLY="+nightly"
  export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS}${US}-Zunstable-options${US}-Cpanic=immediate-abort"
fi

if [ -n "$NIGHTLY" ]; then
  ALVO="$(rustc -vV | awk '/^host:/{print $2}')"
  cargo "$NIGHTLY" build --profile "$PERFIL" -Z build-std=std,panic_abort --target "$ALVO" "$@"
  D=target/$ALVO/$DIR/libcp77_console.dylib
else
  cargo build --profile "$PERFIL" "$@"
  D=target/$DIR/libcp77_console.dylib
fi
codesign --remove-signature "$D" 2>/dev/null || true
# O alinhador é RUST, não Python (portado em 2026-08-22, equivalência provada byte a byte contra
# o script antigo). "Zero Python" vale para a ferramenta de build também: um `.py` no repositório
# aparece na barra de linguagens do GitHub e contradiz o que o projeto diz de si mesmo.
# O alinhador é uma ferramenta do build, não o dylib: compila SEM as flags do nightly (elas são do
# caminho público e quebram o cargo estável com `-Zunstable-options`).
( unset CARGO_ENCODED_RUSTFLAGS; cargo build --release --bin fix-symtab-align >/dev/null 2>&1 )
./target/release/fix-symtab-align "$D"
# TRACELESS: o --remap-path-prefix acima só cobre debug-info/panic-paths, NÃO o LC_ID_DYLIB (o
# linker grava o path absoluto de build ali por conta própria) — achado 2026-07-24, vazava o path
# inteiro da máquina de dev em todo zip publicado sem o gate `strings`-based do pack-bwms.sh pegar
# (strings -a não decodifica esse campo específico do Mach-O). Fix: forçar um ID limpo sempre.
install_name_tool -id "libcp77_console.dylib" "$D"
codesign -s - --force "$D"
# O build público sai num caminho que depende do perfil e do alvo (`target/<host>/publico/` quando o
# nightly entra). Quem empacota não deve ter que adivinhar: fica sempre uma cópia com nome fixo.
if [ "$DEV" = "0" ]; then
  cp "$D" target/libcp77_console-publico.dylib
  codesign -s - --force target/libcp77_console-publico.dylib 2>/dev/null || true
  echo "publico: target/libcp77_console-publico.dylib ($(wc -c < "$D" | tr -d ' ') B, $([ -n "$NIGHTLY" ] && echo 'nightly+build-std' || echo 'estável'))"
fi
m=$(otool -l "$D" 2>/dev/null | awk '/cmd LC_SYMTAB/{x=1} x&&/stroff/{print ($2%8); exit}')
echo "core: $D  (stroff mod8=$m — 0=loadable)"
