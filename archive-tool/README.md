# archive-tool

CLI (Rust) para o formato `.archive` (**RDAR**) do Cyberpunk 2077 no macOS
(Apple Silicon). Lê o índice, gera um `datamap.md` e **extrai os recursos para
uma pasta** — incluindo os comprimidos, via um decodificador **Kraken nativo
arm64** (o `ooz`, compilado pelo `build.rs`).

Porta os codecs de leitura do **WolvenKit** (`WolvenKit.RED4/Archive`) para Rust.
É a primeira ferramenta offline do plano de modding macOS (`../PLANO-CYBERPUNK-MACOS.md`).

## Estado

- **datamap.md** — retrato do índice (recursos, segmentos, deps, compressão,
  sha1, timestamps). Não precisa de Kraken.
- **Extração** — recursos para uma pasta, descomprimindo o payload Kraken. **Funciona.**
- **Resolução de nomes** — o RDAR só guarda o hash FNV-1a64 do path; passe
  `--hashes <lista>` (texto **ou** a `usedhashes.kark` do WolvenKit, que é
  descomprimida na hora) para extrair com os caminhos reais.

Validado em archives reais do jogo: o SHA1 de cada recurso descomprimido bate com
o do índice, e os arquivos saem como CR2W válidos.

## Build

```sh
# Completo (extração comprimida): precisa de clang++ e da fonte do ooz em ../ooz.
cargo build --release

# Só datamap + extração crua (puro std, sem clang/ooz):
cargo build --release --no-default-features
```

A feature `kraken` é **default**. O `build.rs` compila `../ooz` (powzix/ooz,
adaptado p/ clang/arm64 em `ooz/stdafx.h` + `ooz/sse2neon.h`) num `libooz.a` e o
linka. Aponte outra fonte com `OOZ_DIR=/caminho/ooz`.

## Local do jogo (embutido)

O diretório de conteúdo do Cyberpunk já vem **marcado** no binário, então dá para
referir os archives só pelo **nome**. Override por ambiente:

- `CP77_CONTENT` — aponta direto para a pasta `.../archive/Mac/content`.
- `CP77_DIR` — raiz do jogo (resolve `archive/Mac/content` ou `archive/pc/content`).

Os nomes dos recursos são resolvidos automaticamente pela `usedhashes.kark` do
projeto (`--no-hashes` desliga; `--hashes <arquivo>` usa outra lista).

## Uso

```sh
BIN=target/release/archive-tool

# Lista os archives do jogo (local embutido) com tamanhos
$BIN list

# Resumo / datamap (por NOME, sem caminho)
$BIN info basegame_2_mainmenu
$BIN datamap basegame_2_mainmenu            # -> <archive>.datamap.md
$BIN datamap basegame_2_mainmenu -o -       # -> stdout

# Extrai: cria, AO LADO do archive, uma pasta com o nome dele = <dest>/<nome>/...
$BIN extract basegame_2_mainmenu            # -> ao lado do .archive
$BIN extract basegame_2_mainmenu ~/saida    # -> ~/saida/basegame_2_mainmenu/...
$BIN extract basegame_2_mainmenu --datamap

# Extrai TODOS os archives do jogo, cada um na sua pasta
$BIN extract --all ~/cp77-extraido
```

Recursos sem nome resolvido vão para `unknown/<hash>.bin` (ou `--skip-unresolved`
para pulá-los).

## Formato (resumo)

```
0x00  Header(40): magic "RDAR"(1380009042) u32, version u32, indexPosition u64,
                  indexSize u32, debugPosition u64, debugSize u32, filesize u64
0x28  customDataLength u32
0xAC  LxrsFooter (se customDataLength != 0): paths embutidos (ArchiveXL/mods)
@indexPosition  Index(indexSize): fileTableOffset u32, fileTableSize u32, crc u64,
                  fileEntryCount u32, fileSegmentCount u32, resourceDependencyCount u32,
                  FileEntry×N (56b), FileSegment×M (16b: offset u64, zsize u32, size u32),
                  Dependency×K (8b: hash u64)
```

Um recurso = concatenação dos segmentos `[segments_start..segments_end)`; o
primeiro (principal) é descomprimido, os buffers seguintes copiados crus (a menos
de `--decompress-buffers`). Um segmento precisa de Kraken quando `zsize != size`
e começa com o magic `KARK`.

## Compressão (ooz)

O `ooz` é **decode-only** — exatamente o que a extração precisa. Os símbolos C++
casam com o `Kraken_Decompress(const u8*, size_t, u8*, size_t)` que o WolvenKit
usa. `ooz/stdafx.h` substitui os headers MSVC e mapeia os builtins; no arm64 o
`sse2neon.h` traduz os intrínsecos SSE para NEON. Não há compressor (packing de
archives fica para depois, com outra fonte).

## Módulos

`archive.rs` (leitor RDAR) · `datamap.rs` · `extract.rs` · `kraken.rs` (FFI do
ooz) · `hashes.rs` (FNV1a64 + listas, incl. `.kark`) · `time.rs` (FILETIME→ISO)
· `build.rs` (compila o ooz) · `ooz_shim.cpp` (ponte extern "C").

## Testes

```sh
cargo test                      # default (kraken)
cargo test --no-default-features # puro std
cargo clippy --all-targets      # limpo em ambas as configs
```
