# bwms archive

Subcomando da CLI `bwms` (Rust) para o formato `.archive` (**RDAR**) do Cyberpunk 2077 no macOS
(Apple Silicon). Lê o índice, gera um `datamap.md` e **extrai os recursos para
uma pasta** — incluindo os comprimidos, via um decodificador **Kraken nativo
arm64** (o `ooz`, compilado pelo `build.rs` com `--features kraken`).

Porta os codecs de leitura do **WolvenKit** (`WolvenKit.RED4/Archive`) para Rust.
É a primeira ferramenta offline do plano de modding macOS.

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
# na pasta bwms/
# Só datamap + extração crua (puro std, sem clang/ooz):
cargo build --release

# Completo (extração comprimida): precisa de clang++ e da fonte do ooz.
cargo build --release --features kraken
```

A feature `kraken` é **opcional**: o build padrão não leva código de terceiro. Com ela,
o `build.rs` compila o ooz (powzix/ooz, adaptado p/ clang/arm64 em `ooz/stdafx.h` +
`ooz/sse2neon.h`; padrão `../_referencia-de-terceiro/ooz`, relativo a `bwms/`) num
`libooz.a` e o linka. Aponte outra fonte com `OOZ_DIR=/caminho/ooz`.

## Local do jogo

Sem caminho, os archives são procurados na instalação Steam padrão
(`~/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077/archive/Mac/content`),
então dá para referir os archives só pelo **nome**. Override por ambiente:

- `CP77_CONTENT` — aponta direto para a pasta `.../archive/Mac/content`.
- `CP77_DIR` — raiz do jogo (resolve `archive/Mac/content` ou `archive/pc/content`).

Os nomes dos recursos são resolvidos automaticamente pela `usedhashes.kark` ao lado
do binário ou na pasta atual (`--no-hashes` desliga; `--hashes <arquivo>` usa outra
lista). Ler a `.kark` pede `--features kraken`.

## Uso

```sh
BWMS=target/release/bwms

# Lista os archives do jogo (local padrão) com tamanhos
$BWMS archive list

# Resumo / datamap (por NOME, sem caminho)
$BWMS archive info basegame_2_mainmenu
$BWMS archive datamap basegame_2_mainmenu            # -> <archive>.datamap.md
$BWMS archive datamap basegame_2_mainmenu -o -       # -> stdout

# Extrai: cria, AO LADO do archive, uma pasta com o nome dele = <dest>/<nome>/...
$BWMS archive extract basegame_2_mainmenu            # -> ao lado do .archive
$BWMS archive extract basegame_2_mainmenu ~/saida    # -> ~/saida/basegame_2_mainmenu/...
$BWMS archive extract basegame_2_mainmenu --datamap

# Extrai TODOS os archives do jogo, cada um na sua pasta
$BWMS archive extract --all ~/cp77-extraido
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
`sse2neon.h` traduz os intrínsecos SSE para NEON. Não há compressor: o `pack` e o
`packdir` gravam os recursos sem compressão (`writer.rs`).

## Módulos

`archive.rs` (leitor RDAR) · `datamap.rs` · `extract.rs` · `kraken.rs` (FFI do
ooz) · `hashes.rs` (FNV1a64 + listas, incl. `.kark`) · `time.rs` (FILETIME→ISO)
· `cr2w.rs` (CR2W: lê, edita, re-empacota) · `writer.rs` (grava RDAR sem compressão).
Na raiz de `bwms/`: `build.rs` (compila o ooz) · `ooz_shim.cpp` (ponte extern "C").

## Testes

```sh
# na pasta bwms/
cargo test archive::                     # puro std (padrão)
cargo test archive:: --features kraken   # com o Kraken
cargo clippy --all-targets
```
