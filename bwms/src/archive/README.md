# bwms archive

Subcomando da CLI `bwms` (Rust) para o formato `.archive` (**RDAR**) do Cyberpunk 2077 no macOS
(Apple Silicon). Lê o índice, gera um `datamap.md`, **extrai os recursos para uma pasta**, edita e
re-empacota CR2W e grava `.archive` sem compressão.

## Estado

- **datamap.md** — retrato do índice (recursos, segmentos, deps, compressão, sha1, timestamps).
- **Extração** — recursos para uma pasta. Este build não descomprime Kraken: extrai o que está
  gravado sem compressão e conta o resto no relatório.
- **Resolução de nomes** — o RDAR só guarda o hash FNV-1a64 do path; passe `--hashes <lista>` (texto)
  para extrair com os caminhos reais.

## Build

```sh
# na pasta bwms/
cargo build --release
```

Puro `std`, sem dependência.

## Local do jogo

Sem caminho, os archives são procurados na instalação Steam padrão
(`~/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077/archive/Mac/content`),
então dá para referir os archives só pelo **nome**. Override por ambiente:

- `CP77_CONTENT` — aponta direto para a pasta `.../archive/Mac/content`.
- `CP77_DIR` — raiz do jogo (resolve `archive/Mac/content` ou `archive/pc/content`).

Os nomes dos recursos são resolvidos por uma lista de texto ao lado do binário ou na pasta atual
(`--no-hashes` desliga; `--hashes <arquivo>` usa outra lista).

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
0xAC  rodapé de paths (se customDataLength != 0): paths embutidos (mods)
@indexPosition  Index(indexSize): fileTableOffset u32, fileTableSize u32, crc u64,
                  fileEntryCount u32, fileSegmentCount u32, resourceDependencyCount u32,
                  FileEntry×N (56b), FileSegment×M (16b: offset u64, zsize u32, size u32),
                  Dependency×K (8b: hash u64)
```

Um recurso = concatenação dos segmentos `[segments_start..segments_end)`. Um segmento está
comprimido com Kraken quando `zsize != size` e começa com o magic `KARK`; o `pack` e o `packdir`
gravam sem compressão (`writer.rs`).

## Módulos

`archive.rs` (leitor RDAR) · `datamap.rs` · `extract.rs` · `kraken.rs` (detecção de segmento
comprimido) · `hashes.rs` (FNV1a64 + listas) · `time.rs` (FILETIME→ISO) · `cr2w.rs` (CR2W: lê, edita,
re-empacota) · `writer.rs` (grava RDAR sem compressão).

## Testes

```sh
# na pasta bwms/
cargo test archive::
cargo clippy --all-targets
```
