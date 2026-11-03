# bwms tweak

Subcomando da CLI `bwms` (Rust, **sem dependência do crates.io**: só o
`bwms-primitives`, da mesma árvore) para inspecionar o `tweakdb.bin` do Cyberpunk
2077 no macOS. O `tweakdb.bin` é o banco de dados de **gameplay** (dano, stats,
preços, itens, ...): records + flats (valores) + queries + group tags. Os flats
**não são comprimidos**, então é leitura direta — sem Kraken.

Porta o codec de leitura do **WolvenKit** (`WolvenKit.RED4/TweakDB`). É a base do
merger offline de dano/stats — a primeira ferramenta de modding de gameplay no Mac
**sem hook**.

## Uso

```sh
# na pasta bwms/
cargo build --release                     # puro std
cargo build --release --features kraken   # + descomprime a tweakdbstr.kark (clang++ e a fonte do ooz)
BWMS=target/release/bwms

# Sem <file>, procura o tweakdb.bin nas instalações comuns do macOS (override: CP77_DIR). `ep1` = Phantom Liberty.
$BWMS tweak info                          # resumo
$BWMS tweak map                           # header + flat types + contagens
$BWMS tweak sample --type CFloat -n 8     # exemplos: NOME -> valor
$BWMS tweak find "Items.Preset_Lexington_Default"   # acha flats pelo nome + valor
$BWMS tweak find Damage                   # qualquer flat cujo nome contém "Damage"

# Exporta TODOS os atributos (nome ⇥ tipo ⇥ valor) — a lista completa
$BWMS tweak dump -o tweakdb-flats.tsv                       # 2,88M flats (~248 MB)
$BWMS tweak dump --type CFloat --filter damage -o dano.tsv  # só os de dano numérico

# Inspeção pontual
$BWMS tweak get Items.Preset_Lexington_Default.OnAttach          # 1 flat
$BWMS tweak record Items.Preset_Lexington_Default                # todas as props do record

# Edita e grava arquivo NOVO (nunca toca no original). Qualquer tipo:
$BWMS tweak set Items.GrenadeIncendiarySticky.range 30           # numérico
$BWMS tweak set Items.X.IsIKEnabled true                          # bool
$BWMS tweak set Items.X.NPCAnimWrapperWeightOverride MeuNome      # string (CName/CString)
$BWMS tweak set Items.X.fxAppearance '$Foo.Bar'                   # ref TweakDBID por nome
$BWMS tweak set Items.X.tags '[a, b, c]'                          # array inteiro

# Lote (changeset .tweak), com append/remove em arrays (estilo TweakXL)
#   Flat = valor | Flat = [a,b,c] | Flat += valor | Flat -= valor
$BWMS tweak batch conforto-camera.tweak -o tweakdb-mod.bin

$BWMS tweak roundtrip                     # confirma que o writer reproduz o arquivo byte-a-byte
$BWMS tweak map ep1                       # tweakdb_ep1.bin
```

Para usar no jogo: substitua `r6/cache/tweakdb.bin` pelo arquivo gerado (faça
backup do original primeiro).

Os **nomes** (TweakDBID → texto) vêm da `tweakdbstr.kark` do WolvenKit, procurada
ao lado do binário e depois na pasta atual (override `TWEAKDB_NAMES`; `--no-names`
desliga; sem a lista, segue sem nomes). O build padrão é puro std: lê tweakdb.bin e
listas já descomprimidas, mas não `.kark`, que pede `--features kraken`.

## O que já faz

- Parseia header (magic `0x0BB1DB47`, blob v8 / parser v4), os 4 offsets de seção,
  a tabela de flat types, os records (id + tipo Murmur32), e conta queries/group tags.
- **Resolve os 22 flat types por nome** via FNV-1a64 do nome RED do tipo
  (validado: todos os 22 do `tweakdb.bin` real batem).
- **Decodifica os valores dos flats** — todos os 20 `ETweakType` (Float, Bool,
  Int/Uint 8–64, CName/CString com VLQ, TweakDBID/CResource/LocKey, Color,
  Vector2/3, EulerAngles, Quaternion) e arrays (`array:T`).

Validado no `tweakdb.bin` real (42 MB): 2,88M flats, 176K records, 22/22 tipos
resolvidos; valores de CFloat decodificados (ex.: `0.1`, `-0.5`, `2`).

## Formato (resumo)

```
magic u32 = 0x0BB1DB47
Header(28): blobVersion i32(8), parserVersion i32(4), recordChecksum u32,
            flatsOffset, recordsOffset, queriesOffset, groupTagsOffset (i32 cada)
@flats     numFlatTypes i32; {typeHash u64, valueCount u32, keyCount u32, offset u32}×N;
           em cada offset: numValues u32, valores; numKeys u32, {keyId u64, valueIndex i32}×K
@records   numRecords i32; {id u64 (TweakDBID), typeKey u32 (Murmur32)}×N
@queries   numQueries i32; {id u64, numResults u32, result u64×R}×N
@groupTags numGroupTags i32; {id u64, tag u8}×N
```

`typeHash` = FNV-1a64 do nome RED (`"Float"`) ou `"array:"+nome`. `TweakDBID` é um
u64 = `CRC32(nome) | (len<<32)`.

## Resolução de nomes (feito)

O flat/record é endereçado por TweakDBID = `CRC32(nome) | (len<<32)`. A
`tweakdbstr.kark` (196K records, 3,3M flats, 490 queries) é descomprimida e
indexada hash→nome. Validado: `sample`/`find` mostram nomes reais (ex.:
`Items.GrenadeIncendiarySticky.deepWaterDepth = -0.5`).

## Writer / merger (feito)

Reescreve o `tweakdb.bin` no layout REAL do jogo. **Achado-chave:** o
`TweakDBWriter` do WolvenKit grava um formato **delta** (tabela de flat types de
12 bytes, sem offsets) que NÃO é o do jogo — então o writer aqui reproduz o
layout do *reader* (offsets recalculados), não porta o writer. Detalhes que
tornam o round-trip **byte-exato**:

- valores guardados como **bytes crus** (strings VLQ são ambíguas ao decodificar);
- os `valueCount`/`keyCount` da TABELA são preservados do original — o reader os
  ignora (usa os do bloco) e no arquivo do jogo eles **diferem** dos reais;
- `recordChecksum` é sobre a seção de records → editar flats (dano/stats) não o muda.

Validado: `roundtrip` reproduz o `tweakdb.bin` de 42 MB **byte-a-byte**; edits
mudam só os bytes do flat editado (e o bloco do tipo quando muda de tamanho), e o
arquivo continua válido. `set`/`batch` cobrem **todos os 20 tipos escalares** +
**arrays** (set/append/remove). Validado no tweakdb real: string `"None"→"ZZTEST"`,
array `OnAttach += $Items.X` (re-encode correto), tudo round-trip-válido.

## Cobertura vs Windows (TweakXL)

A meta é cobrir o que do TweakXL é **viável offline no macOS** (o que é runtime-only
fica de fora por definição — exige o jogo vivo + hook, não um editor de arquivo).

| Função (TweakXL) | Viável offline? | bwms tweak |
|---|---|---|
| Ler/inspecionar tweakdb | sim | ✅ `info`/`map`/`sample`/`find`/`dump`/`get`/`record` |
| Resolver nomes (TweakDBID→texto) | sim | ✅ `tweakdbstr.kark` + CRC32 |
| Editar flat numérico/bool | sim | ✅ |
| Editar string (CName/CString) | sim | ✅ (VLQ) |
| Editar ref (TweakDBID/CResource) | sim | ✅ `$Nome`/hex/decimal |
| Editar LocKey | sim | ✅ `LocKey#N` = chave; `LocKey#texto` e texto sem prefixo (decimal e vazio inclusos) = FNV-1a64 (`bwms_primitives::lockey_chave`, a regra do runtime) |
| Editar Color/Vector2-3/Euler/Quaternion | sim | ✅ |
| Array: set inteiro / append / remove | sim | ✅ `= [..]` / `+=` / `-=` |
| Changeset em lote (`.tweak`) | sim | ✅ `batch` |
| Regravar `tweakdb.bin` válido | sim | ✅ writer byte-exato |
| Criar record NOVO clonando outro (copia+ajusta) | sim | ✅ `clone` + `set` |
| Criar record de tipo do ZERO (flats inéditos) | parcial — precisa do schema RTTI (`InheritanceMap.dat`/`ExtraFlats.dat`) | ⚠️ parcial: `create --class` e `$type` copiam o schema de uma amostra da classe no próprio bin; classe sem amostra, não |
| Herança `$base` entre records | parcial — idem | ✅ `$base` (`apply-yaml`/`apply-toml`) e `Novo : Base` (`apply-tweak`): clone + overrides |
| Tweaks runtime/scriptáveis | **não** (runtime-only) | — fora do escopo offline |

**Cobertura ≳90% do viável offline.** Está completo: ler tudo, resolver nomes,
editar qualquer flat EXISTENTE (escalar ou array, todos os tipos), lote, clonar
records, herança `$base` e regravar válido — o uso real do TweakXL (tunar
dano/stats/itens/listas e clonar+ajustar). Fora: criar record de uma classe sem
amostra no bin (flats inéditos), que depende do schema RTTI (o pedaço menos viável
offline); e tweaks runtime-only, que por definição não cabem num editor de arquivo.

## Testes

```sh
# na pasta bwms/
cargo test tweak::   # os do `bwms tweak`: VLQ, strings, hashes de tipo, changeset
cargo clippy
```
