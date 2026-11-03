//! Fonte única do que o runtime (`cp77-console`) e as ferramentas (`bwms`; `bwms-catalog` e `save-parser` do `bwms-dev`) calculam igual.
//! Zero-dep. Hashes do Cyberpunk 2077: TweakDBID = CRC32(nome)|(len<<32); type_key de record = murmur3(miolo, seed 0x5EEDBA5E);
//! ResourcePath (RDAR/ArchiveXL) = FNV-1a64 do path normalizado; NodeRef; chave de LocKey; FNV-1a 32/64 crus; CRC-64 e SHA-1 do `.archive`;
//! CRC-16 do id de nó de quest. A raiz também guarda regras que o runtime e o install leem igual (`formato_tweak`, `PASTA_DO_RUNTIME`,
//! props do `resource.patch`). Os módulos não são hash: `editor_texto`, `garment_ctx`, `override_state`, `regex`, `restarget` e `targets`
//! são núcleos puros (sem endereço do jogo) de itens Codeware/ArchiveXL que o runtime usa, testáveis fora do jogo; `dynarray` é o cabeçalho
//! do `DynArray` do motor; `json`, o JSON estrito dos arquivos de estado da `instalacao`. `enderecos` é a resolução de endereço do jogo por
//! assinatura de instruções ARM64 (RED4ext #447): o mesmo código gera a tabela fora do jogo (`examples/gerar_enderecos.rs`, que o
//! `build-core.sh` roda antes do runtime) e a resolve no runtime. `macho` é o andador dos comandos de carga de um Mach-O de 64 bits, para o
//! `enderecos`, o `bwms-helper` do instalador e o `fix-symtab-align` do build. `native_registrada` é a conferência de `native func` declarada
//! num `.reds` contra os binários que registram natives: a mesma para o `bwms-helper natives`, o `bwms mods checar` e a compilação dos
//! scripts na abertura do jogo. `rdar` é o cabeçalho do `.archive`: o runtime confere por ele
//! o arquivo que entrega ao motor, e o `bwms archive` abre o índice por ele. `tweakxl` é o parser do formato declarativo do TweakXL
//! (`.yaml`/`.toml`/`.tweak` → `Vec<operacoes::Op>`), com o leitor YAML (`tweakxl::yaml`) que também lê o `.xl` da `instalacao`.
//! `operacoes` é a representação canônica que os formatos produzem: o runtime aplica as ops no TweakDB vivo e o `bwms tweak`, no
//! `tweakdb.bin`. `instalacao` é o motor de mods (`.xl`, a pasta `Mods/`, tabelas `red4ext/bwms-*.txt`, a migração da biblioteca antiga). `tabelas`
//! é o formato dessas tabelas (arquivo, cabeçalho e linha), o mesmo para quem grava (`instalacao`) e para quem lê (o runtime).
pub mod dynarray;
pub mod editor_texto;
pub mod enderecos;
pub mod garment_ctx;
pub mod instalacao;
pub mod json;
pub mod macho;
pub mod native_registrada;
pub mod operacoes;
pub mod override_state;
pub mod rdar;
pub mod regex;
pub mod restarget;
pub mod tabelas;
pub mod targets;
pub mod tweakxl;

/// Offset basis do FNV-1a 64-bit: o seed de [`fnv1a64`], de [`resource_path_hash`] e de
/// [`node_ref_hash`], e o default do `opt seed` do `FNV1a64` do Codeware.
pub const FNV1A64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// Primo do FNV-1a 64-bit.
pub const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01b3;
/// Offset basis do FNV-1a 32-bit: o seed de [`fnv1a32`] e o default do `opt seed` do `FNV1a32` do Codeware.
pub const FNV1A32_OFFSET: u32 = 0x811c_9dc5;
/// Primo do FNV-1a 32-bit.
pub const FNV1A32_PRIME: u32 = 0x0100_0193;

/// FNV-1a 64-bit sobre os bytes da string (mesma função dos paths do RDAR, sem normalizar).
pub const fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_seeded(bytes, FNV1A64_OFFSET)
}

/// FNV-1a 64-bit com hash INICIAL customizável (`seed`) — a mesma assinatura do `Red::FNV1a64`
/// real do Codeware (`App/Utils/Hashing.hpp`: `Red::Optional<uint64_t, 0xCBF29CE484222325>`,
/// ou seja, o valor default do parâmetro `opt seed` É o offset basis padrão do FNV — quando o
/// .reds chamador omite o seed, o COMPILADOR já embute esse default na chamada, então o
/// handler nativo nunca precisa adivinhar "seed==0 → usa o default"). `fnv1a64` acima é só
/// este com o offset basis padrão.
///
/// `const fn` para o hash de um texto fixo virar constante na compilação. `while let` sobre a fatia e
/// `as` porque `for` e `u64::from` não são const no rust-version 1.74 (`Cargo.toml`).
pub const fn fnv1a64_seeded(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    let mut bytes = bytes;
    while let [byte, resto @ ..] = bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV1A64_PRIME);
        bytes = resto;
    }
    hash
}

/// Prefixo do texto de uma chave de localização (`Red::LocKeyPrefix`, `Red/Localization.hpp` do TweakXL).
pub const LOCKEY_PREFIXO: &str = "LocKey#";

/// `primaryKey` do `gamedataLocKeyWrapper` a partir do texto INTEIRO de um mod, pela regra do runtime (escritor
/// TweakXL e `SetFlat` por script, `cp77-console/src/tweakdb/escritor.rs` e `natives.rs`), para a ferramenta offline
/// e o jogo darem a mesma chave: `LocKey#<n>` com `n` decimal u64 é a chave primária; qualquer outro texto depois
/// de [`LOCKEY_PREFIXO`], e o texto sem o prefixo, é a chave secundária, guardada como FNV-1a64.
/// TODO(decisão do dono, F1-30): o conversor YAML do TweakXL (`YamlReader.Values.cpp:174-191`, `Str.hpp:7-24`) dá
/// a chave decimal também sem o prefixo e 0 para o miolo vazio, e o `strtoull` dele aceita espaço à esquerda, sinal
/// `-` e estouro; trocar aqui troca a ferramenta e, depois do F3-04, o runtime.
pub fn lockey_chave(texto: &str) -> u64 {
    match texto.strip_prefix(LOCKEY_PREFIXO) {
        Some(n) => n.parse::<u64>().unwrap_or_else(|_| fnv1a64(n.as_bytes())),
        None => fnv1a64(texto.as_bytes()),
    }
}

/// Tipos de texto do `localization` do `.archive.xl` que o runtime aplica (`cp77-console/src/localizacao`, `recursos_do_idioma`). O install
/// avisa de cada grupo de outro tipo (`instalacao::tabelas_de_runtime::localizacao_sem_efeito`). Mora aqui porque é a folha que o runtime e o install
/// usam. Um tipo só entra quando o runtime souber ler o recurso dele: o leitor de hoje (`entradas_do_token`) lê o formato do onscreens.
pub const TIPOS_DE_TEXTO_APLICADOS: &[&str] = &["onscreens"];

/// Hash de ResourcePath do RDAR (o que o depot usa p/ resolver .archive): FNV-1a 64-bit do path
/// NORMALIZADO. É o coração do resource.link/copy do ArchiveXL. PROVADO 5/5 goldens reais
/// (testes em `instalacao/tabelas_de_runtime.rs`). Fonte única do runtime E do offline.
///
/// Sanitização na ordem do comentário do `HashSanitized` do RED4ext.SDK (`ResourcePath.hpp:73-77`,
/// `cp77-symbols/notes/CATALOGO-EXAUSTIVO-RED4EXT.md` item #395):
/// (1) descarta 1 aspas de abertura E 1 de fechamento (simples ou duplas), se presentes;
/// (2) descarta TODAS as barras/contrabarras iniciais (não só 1);
/// (3) colapsa barras/contrabarras repetidas em sequência pra 1 só;
/// (4) converte toda barra `/` pra contrabarra `\` (separador canônico);
/// (5) converte pra minúsculas.
/// + trunca em 216 bytes máx (pós-normalização); string vazia ou 100% descartada → hash=0.
/// As regras (2)/(3)/(4)/(5) e o truncamento afetam a mesma posição relativa umas às outras
/// (colapso considera o resultado JÁ normalizado — uma barra seguida de contrabarra também
/// colapsa, já que ambas viram `\` antes da checagem de repetição).
///
/// TODO(#395): o CÓDIGO do `HashSanitized` não faz a regra (1) como o comentário dele descreve: para
/// na primeira aspa depois da de abertura, em qualquer posição (`ResourcePath.hpp:99`), e devolve 0
/// se nada sobrou. Aqui só sai a aspa do último byte e as internas entram no hash (`base\it's.mesh`,
/// `""x""`). Qual dos dois o motor segue está \[UNCONFIRMED\]: medir o hash que o jogo dá a um path
/// com apóstrofo interno (`depot_loaded_resource_class_by_hash`) antes de mudar a regra.
/// TODO: byte >= 0x80 entra sem sinal; o SDK (`char` com sinal) e o índice do WolvenKit
/// (`useSignedBuffer`) estendem o sinal. \[UNCONFIRMED\] contra um `.archive` do WolvenKit com path
/// acentuado (name_hash do índice x este hash).
///
/// `const fn` pelo mesmo motivo de [`fnv1a64_seeded`]. Os cortes de aspa e de barra são padrões de
/// fatia porque fatiar por intervalo (`&bytes[1..]`) não é const no rust-version 1.74.
pub const fn resource_path_hash(path: &str) -> u64 {
    let mut bytes = path.as_bytes();
    // Regra 1: 1 aspas de abertura (simples ou dupla), se o primeiro byte for uma.
    if let [b'"' | b'\'', resto @ ..] = bytes {
        bytes = resto;
    }
    // Regra 1: 1 aspas de fechamento (simples ou dupla), se o último byte for uma.
    if let [resto @ .., b'"' | b'\''] = bytes {
        bytes = resto;
    }
    // Regra 2: TODAS as barras/contrabarras iniciais.
    while let [b'/' | b'\\', resto @ ..] = bytes {
        bytes = resto;
    }

    // Regras 3+4+5, truncamento e FNV no mesmo passe: o byte normalizado vai direto pro hash, sem
    // buffer, e o corte de 216 (`MaxPathLength` do SDK) conta os bytes emitidos, pós-colapso.
    let mut hash = FNV1A64_OFFSET;
    let mut emitidos = 0usize;
    let mut prev_was_sep = false;
    while let [b, resto @ ..] = bytes {
        bytes = resto;
        let b = *b;
        let sep = b == b'/' || b == b'\\';
        if sep && prev_was_sep {
            continue; // Regra 3: colapsa repetição.
        }
        prev_was_sep = sep;
        let norm = if sep { b'\\' } else { b.to_ascii_lowercase() };
        hash ^= norm as u64;
        hash = hash.wrapping_mul(FNV1A64_PRIME);
        emitidos += 1;
        if emitidos == 216 {
            break;
        }
    }

    if emitidos == 0 {
        0
    } else {
        hash
    }
}

/// Hash de um `NodeRef` do mundo (o que `ToNodeRef`/`ToEntityID`/spawns e scene-mods do Codeware
/// usam pra resolver um nó pelo path) — porte do `NodeRef::Hash` do RED4ext.SDK (`NodeRef.hpp:83-111`),
/// byte-exato para ASCII sem NUL. O Codeware não tem algoritmo próprio: chama a nativa
/// `NodeRef::Create` do jogo (`Codeware/src/Red/NodeRef.hpp:19-23`); este é o algoritmo do SDK.
/// É FNV-1a64 (seed/prime padrão) do path, com DUAS regras de skip que o distinguem do `fnv1a64` cru:
/// - `#` (marcador de nó dinâmico) é PULADO — o char não entra no hash.
/// - `;alias` (um alias local) é PULADO até o `/` seguinte; o `/` terminador SIM entra no hash.
///   Se o `;alias` vai até o fim da string (sem `/`), o hash para ali.
/// Saída: hash final == seed → 0, a regra literal do SDK (`return hash == seed ? 0 : hash`,
/// `NodeRef.hpp:110`). Cobre a string vazia ou toda pulada e também uma entrada que volte ao seed.
/// `GlobalRoot`=`node_ref_hash("$")`, `RelativeRoot`=`node_ref_hash("~")`.
/// TODO: fora de ASCII o SDK estende o sinal do `char` e para no NUL; aqui o byte entra sem sinal e o
/// NUL é hasheado. A equivalência com a nativa `NodeRef::Create` está \[UNCONFIRMED\]: falta o endereço
/// Mac dela para comparar.
pub fn node_ref_hash(node_ref: &str) -> u64 {
    let bytes = node_ref.as_bytes();
    let mut hash = FNV1A64_OFFSET;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' {
            i += 1;
            continue;
        }
        if bytes[i] == b';' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'/' {
                i += 1;
            }
            if i >= bytes.len() {
                break;
            }
            // bytes[i] == b'/' aqui: cai fora do if e HASHEIA o '/' (fiel ao C++).
        }
        hash ^= u64::from(bytes[i]);
        hash = hash.wrapping_mul(FNV1A64_PRIME);
        i += 1;
    }
    if hash == FNV1A64_OFFSET {
        0
    } else {
        hash
    }
}

/// FNV-1a 32-bit ([`FNV1A32_OFFSET`], [`FNV1A32_PRIME`]): hash da chave dos HashMap do motor (nó e balde), FactID e pathHash de
/// nó de quest, chave secundária de texto da localização, nome de record inline do TweakXL (`ComposeInlineName`) e native `BwmsFNV1a32`.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    fnv1a32_seeded(bytes, FNV1A32_OFFSET)
}

/// FNV-1a 32-bit com hash inicial customizável — mesma lógica do `fnv1a64_seeded`, pro
/// `Red::FNV1a32` real do Codeware (default `0x811C9DC5` = o offset basis padrão).
pub fn fnv1a32_seeded(bytes: &[u8], seed: u32) -> u32 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(FNV1A32_PRIME);
    }
    hash
}

/// CRC-32 IEEE com hash INICIAL customizável (`seed`) — a assinatura exata do `CRC32(data, len,
/// seed)` do RED4ext (`Hashing/CRC.hpp`: `crc = ~seed; ...; return ~crc`). É a impl CANÔNICA;
/// [`crc32`] é só este com `seed=0`. O seed permite CONTINUAR um CRC de onde outro parou — a base
/// da derivação de TweakDBID ([`tweak_db_id_derive`]): como `~seed` no início e `~crc` no fim se
/// telescopam, `crc32_seeded(B, crc32_seeded(A, s)) == crc32_seeded(A++B, s)`.
///
/// Um byte por passo, pela [`CRC32_TABELA`]: no opt-level `s` do perfil `publico` do cp77-console, a forma bit a
/// bit (8 voltas por byte) gasta 7,6× o tempo da tabela num nome de TweakDB (`bench_crc32`, offline).
pub fn crc32_seeded(bytes: &[u8], seed: u32) -> u32 {
    let mut crc = !seed;
    for &b in bytes {
        crc = (crc >> 8) ^ CRC32_TABELA[usize::from(crc as u8 ^ b)];
    }
    !crc
}

/// CRC-32 IEEE refletido (poly `0xEDB88320`) de cada byte, calculado na compilação: 1 KiB em `.rodata`.
const CRC32_TABELA: [u32; 256] = crc32_tabela();

/// `while` porque `for` não é const no rust-version 1.74 (`Cargo.toml`).
const fn crc32_tabela() -> [u32; 256] {
    let mut tabela = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut volta = 0;
        while volta < 8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
            volta += 1;
        }
        tabela[i] = crc;
        i += 1;
    }
    tabela
}

/// CRC-32 IEEE (poly 0xEDB88320, init 0xFFFFFFFF, xorout 0xFFFFFFFF) — o que o
/// WolvenKit (`Crc32Algorithm`) usa para os nomes do tweakdb. É `crc32_seeded(_, 0)`.
pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_seeded(bytes, 0)
}

/// TweakDBID de um nome: `CRC32(nome) | (len << 32)` (nome em ASCII), onde o campo `length` é UM
/// BYTE (`static_cast<uint8_t>(len)` no `TweakDBID(string_view)` do RED4ext) — nomes >255 chars
/// truncam o length em u8, batendo com o id real do jogo (os bytes 5-7 = tdbOffset, sempre 0 aqui).
pub fn tweak_db_id(name: &str) -> u64 {
    u64::from(crc32(name.as_bytes())) | (u64::from(name.len() as u8) << 32)
}

/// Chave de 40 bits do TweakDBID: crc32 (bytes 0-3) + length (byte 4), sem o tdbOffset (bytes 5-7).
/// É como o motor compara ids; um id lido da memória traz o tdbOffset, um vindo de [`tweak_db_id`] não.
pub const fn tweak_db_id_chave(id: u64) -> u64 {
    id & 0xFF_FFFF_FFFF
}

/// Deriva o TweakDBID de um record/flat filho a partir do id do PAI + um sufixo — o
/// `TweakDBID(const TweakDBID& aBase, string_view aName)` / `operator+` do RED4ext
/// (`NativeTypes-inl.hpp:15,58`), que é como o TweakXL resolve `record.flat` no RegisterName /
/// CreateExtraNames sem re-hashear o nome inteiro. Semântica exata do C++:
/// `hash = CRC32(sufixo, seed=base.hash)` e `length = base.length + sufixo.len()` (u8 wrapping);
/// o tdbOffset do pai é ignorado (o C++ faz memset em zero). Pela propriedade telescópica do CRC,
/// `tweak_db_id_derive(tweak_db_id("base"), ".suf") == tweak_db_id("base.suf")` — PROVADO nos testes.
pub fn tweak_db_id_derive(base_id: u64, suffix: &str) -> u64 {
    let base_hash = (base_id & 0xFFFF_FFFF) as u32;
    let base_len = ((base_id >> 32) & 0xFF) as u8;
    let new_hash = crc32_seeded(suffix.as_bytes(), base_hash);
    let new_len = base_len.wrapping_add(suffix.len() as u8);
    u64::from(new_hash) | (u64::from(new_len) << 32)
}

/// Seed dos type_keys de record no tweakdb (`TweakDB.RecordsSeed` do WolvenKit).
pub const RECORDS_SEED: u32 = 0x5EED_BA5E;

/// MurmurHash3 x86 32-bit (seed configurável). O `type_key` de um record do
/// tweakdb é o murmur3_32, com seed [`RECORDS_SEED`], do MIOLO do nome da classe
/// RED — o WolvenKit aplica o regex `gamedata(.*)_Record` antes (ver
/// [`record_type_key`]).
pub fn murmur3_32(data: &[u8], seed: u32) -> u32 {
    const C1: u32 = 0xcc9e_2d51;
    const C2: u32 = 0x1b87_3593;
    let mut h = seed;
    let nblocks = data.len() / 4;
    for i in 0..nblocks {
        let mut k = u32::from_le_bytes([
            data[i * 4],
            data[i * 4 + 1],
            data[i * 4 + 2],
            data[i * 4 + 3],
        ]);
        k = k.wrapping_mul(C1);
        k = k.rotate_left(15);
        k = k.wrapping_mul(C2);
        h ^= k;
        h = h.rotate_left(13);
        h = h.wrapping_mul(5).wrapping_add(0xe654_6b64);
    }
    let tail = &data[nblocks * 4..];
    let mut k1 = 0u32;
    if tail.len() >= 3 {
        k1 ^= u32::from(tail[2]) << 16;
    }
    if tail.len() >= 2 {
        k1 ^= u32::from(tail[1]) << 8;
    }
    if !tail.is_empty() {
        k1 ^= u32::from(tail[0]);
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(15);
        k1 = k1.wrapping_mul(C2);
        h ^= k1;
    }
    h ^= data.len() as u32;
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    h
}

/// type_key de uma classe de record: aplica o regex `gamedata(.*)_Record` do
/// WolvenKit (strip do prefixo `gamedata` + sufixo `_Record`) e hasheia o miolo
/// com [`murmur3_32`]/[`RECORDS_SEED`]. Aceita tanto a forma completa
/// (`gamedataWeaponItem_Record`) quanto o miolo já stripado (`WeaponItem`).
pub fn record_type_key(class_name: &str) -> u32 {
    let core = class_name
        .strip_prefix("gamedata")
        .and_then(|s| s.strip_suffix("_Record"))
        .unwrap_or(class_name);
    murmur3_32(core.as_bytes(), RECORDS_SEED)
}

/// CRC-64/XZ (refletido, init/xorout 0xFFFF…FFFF, poly refletido 0xC96C5795D7870F42) — o `Crc64`
/// do WolvenKit, usado no crc do ÍNDICE RDAR (`.archive`). Bit-a-bit.
pub fn crc64(bytes: &[u8]) -> u64 {
    const POLY: u64 = 0xC96C_5795_D787_0F42;
    let mut crc = !0u64;
    for &b in bytes {
        crc ^= u64::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ POLY } else { crc >> 1 };
        }
    }
    !crc
}

/// SHA-1 (FIPS 180-1) — hash do `FileEntry.SHA1Hash` de cada recurso do `.archive` (WolvenKit).
/// Os blocos cheios saem direto da entrada e só a cauda com o padding vai pra pilha: o `bwms archive` passa cada recurso
/// inteiro, e copiar a entrada dobraria o pico de memória.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let mut blocos = data.chunks_exact(64);
    for bloco in &mut blocos {
        sha1_bloco(&mut h, bloco);
    }
    // Cauda: resto + 0x80 + zeros + comprimento em bits (u64 BE) no fim; resto >= 56 não deixa
    // os 8 bytes do comprimento caberem num bloco só e pede dois.
    let resto = blocos.remainder();
    let mut cauda = [0u8; 128];
    cauda[..resto.len()].copy_from_slice(resto);
    cauda[resto.len()] = 0x80;
    let fim = if resto.len() < 56 { 64 } else { 128 };
    cauda[fim - 8..fim].copy_from_slice(&(data.len() as u64).wrapping_mul(8).to_be_bytes());
    for bloco in cauda[..fim].chunks_exact(64) {
        sha1_bloco(&mut h, bloco);
    }
    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

/// Compressão de um bloco de 64 bytes do [`sha1`]: os blocos da entrada e os da cauda passam por aqui.
fn sha1_bloco(h: &mut [u32; 5], chunk: &[u8]) {
    let mut w = [0u32; 80];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
    for (i, &wi) in w.iter().enumerate() {
        let (f, k) = match i {
            0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999u32),
            20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
            40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
            _ => (b ^ c ^ d, 0xCA62_C1D6),
        };
        let tmp = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = tmp;
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
}

/// `GeneratePhaseNodeID` do ArchiveXL real (`App::QuestPhaseExtension::GeneratePhaseNodeID`,
/// `App/Extensions/QuestPhase/Extension.cpp`) — o ID determinístico (16-bit) que o sistema de
/// EDIÇÃO DE GRAFO de quest (`PatchPhase`/`CreatePhaseNode`) atribui a cada nó novo inserido
/// num `questGraphDefinition`, derivado de `"<phasePath>:<parentNodeId>"` (chave textual, evita
/// colisão entre patches de mods diferentes no MESMO grafo). Peça `#35`/`#56` do catálogo
/// ArchiveXL — a única sub-parte 100% algoritmo puro (sem `Core::RawFunc`/endereço nativo) das 7
/// funções do sistema de patch de grafo; as outras 6 (`PatchPhase`/`FindConnectionPoint`/
/// `ResolveSocket`/`AddConnection`/`RemoveConnection`/`CreatePhaseNode`) operam sobre um
/// `Handle<questGraphDefinition>` vivo e estão em `cp77-console/src/mundo/mod.rs`
/// (`questphase_patch_aplicar` chama esta fn) e `npcs/mod.rs`.
///
/// **Confirmado ser CRC-16/CCITT-FALSE clássico** (poly `0x1021`, init `0xFFFF`, sem reflexão,
/// sem xorout) — a forma byte a byte, sem tabela, do C++ real (`x = crc>>8 ^ *data; x ^= x>>4;
/// crc = (crc<<8) ^ (x<<12) ^ (x<<5) ^ x`) é uma implementação alternativa do MESMO algoritmo:
/// bate exato com o valor-de-checagem padrão da indústria pra `"123456789"` (`0x29B1`), não só
/// autoconsistência com o C++ transcrito — 2ª fonte independente confirmando a leitura do
/// código-fonte estar correta.
pub fn quest_phase_node_id(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        let mut x = ((crc >> 8) as u8) ^ byte;
        x ^= x >> 4;
        crc = (crc << 8) ^ ((x as u16) << 12) ^ ((x as u16) << 5) ^ (x as u16);
    }
    crc
}

/// Formato de um arquivo de tweak de mod em `r6/tweaks`, pela extensão. É a regra de quem classifica (`instalacao::classify`) e do runtime
/// no jogo (`cp77-console`: `mods_do_usuario::pasta_mods`, `mods_do_usuario::pipeline` e `tweakdb::aplicar_arquivo`), para os dois verem
/// o mesmo conjunto de arquivos.
/// Mora aqui porque é a folha que o runtime e as ferramentas usam.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatoTweak {
    /// `.yaml` e `.yml`: o YAML do TweakXL.
    Yaml,
    /// `.tweak`: a linguagem própria do TweakXL (`tweakxl::tweak_source`).
    Tweak,
}

/// Formato de tweak da extensão `ext` (sem o ponto), sem diferenciar maiúscula. `None` = não é tweak. `toml` fica de fora: é formato
/// de autoria do `bwms tweak`, e o runtime não aplica TOML (o leitor, `tweakxl::toml`, só serve ao `apply-toml`/`registro` do
/// `bwms tweak`).
pub fn formato_tweak(ext: &str) -> Option<FormatoTweak> {
    if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") {
        Some(FormatoTweak::Yaml)
    } else if ext.eq_ignore_ascii_case("tweak") {
        Some(FormatoTweak::Tweak)
    } else {
        None
    }
}

/// Nome da pasta do runtime dentro da pasta do jogo: `<jogo>/red4ext/` guarda a dylib do BWMS, os plugins, as tabelas que o install grava
/// e os mods do usuário. O nome não muda: o `bwms-install.sh` grava `@executable_path/../../../red4ext/libcp77_console.dylib` no
/// LC_LOAD_DYLIB do executável do jogo. Mora aqui porque é a folha que o runtime e as ferramentas usam.
pub const PASTA_DO_RUNTIME: &str = "red4ext";

/// O `scc` grava esta marca no `final.redscripts` que compila; blob sem ela é o vanilla do jogo. A compilação na abertura (`cp77-console`) e
/// o `bwms mods` escolhem o vanilla por ela.
pub const MARCA_DO_SCC: &[u8] = b"$REDSCRIPT_SIGNATURE";

/// `true` quando o blob saiu do `scc` (tem a [`MARCA_DO_SCC`]).
pub fn blob_compilado_pelo_scc(blob: &[u8]) -> bool {
    blob.windows(MARCA_DO_SCC.len()).any(|w| w == MARCA_DO_SCC)
}

/// Props do `resource.patch` (ArchiveXL `#17`) em bits, com os nomes das props da referência. O install (`instalacao::tabelas_de_runtime`,
/// `emit_resource_patch_table`) grava a máscara na `bwms-resource-patch.txt` e o runtime (`cp77-console`, `arquivos_do_jogo`) testa os bits:
/// mora aqui porque é a folha que os dois usam.
pub const A17_PROP_COMPONENTS: u32 = 1;

pub const A17_PROP_ENTITY: u32 = 2;

pub const A17_PROP_APPEARANCES: u32 = 4;

pub const A17_PROP_DEPENDENCIES: u32 = 8;

pub const A17_PROP_VISUALTAGS: u32 = 16;

pub const A17_PROP_PARTSVALUES: u32 = 32;

pub const A17_PROP_PARTSOVERRIDES: u32 = 64;

pub const A17_PROP_CENSORSHIP: u32 = 128;

/// Props do `MorphTargetMesh` (51d).
pub const A17_PROP_BASEMESHAPP: u32 = 256;

pub const A17_PROP_BASETEXTUREPARAM: u32 = 512;

pub const A17_PROP_BOUNDINGBOX: u32 = 1024;

/// `streaming.blocks` do `.xl` (51e): o block é patch do streaming world, como o `RegisterPatch` da referência.
pub const A17_PROP_BLOCKS: u32 = 2048;

/// Props do `MorphTargetMesh` e do `CMesh` que o runtime aplica (a referência aceita qualquer CName; estas são as que ela lê).
pub const A17_PROP_BASEMESH: u32 = 4096;

pub const A17_PROP_BASETEXTURE: u32 = 8192;

pub const A17_PROP_BLOB: u32 = 16384;

pub const A17_PROP_TARGETS: u32 = 32768;

pub const A17_PROP_RENDERBLOB: u32 = 65536;

/// Nome da prop no `.xl` → bit.
pub const A17_PROPS: [(&str, u32); 17] = [
    ("baseMesh", A17_PROP_BASEMESH),
    ("baseTexture", A17_PROP_BASETEXTURE),
    ("blob", A17_PROP_BLOB),
    ("targets", A17_PROP_TARGETS),
    ("renderResourceBlob", A17_PROP_RENDERBLOB),
    ("blocks", A17_PROP_BLOCKS),
    ("baseMeshAppearance", A17_PROP_BASEMESHAPP),
    ("baseTextureParamName", A17_PROP_BASETEXTUREPARAM),
    ("boundingBox", A17_PROP_BOUNDINGBOX),
    ("components", A17_PROP_COMPONENTS),
    ("entity", A17_PROP_ENTITY),
    ("appearances", A17_PROP_APPEARANCES),
    ("resolvedDependencies", A17_PROP_DEPENDENCIES),
    ("visualTags", A17_PROP_VISUALTAGS),
    ("partsValues", A17_PROP_PARTSVALUES),
    ("partsOverrides", A17_PROP_PARTSOVERRIDES),
    ("censorshipMapping", A17_PROP_CENSORSHIP),
];

/// Máscara de props escritas por nome, separados por espaço: `-` liga todas (props vazio, o `Modifies` da referência); cada nome de
/// [`A17_PROPS`] liga o bit dele, e nome desconhecido não liga nenhum. Os espaços das pontas não contam. O install converte assim as props de
/// cada patch do `.xl` (juntas por espaço) e o runtime, o campo de props por nome de uma tabela posta à mão.
pub fn a17_props_mascara(campo: &str) -> u32 {
    let campo = campo.trim();
    if campo == "-" {
        return u32::MAX;
    }
    campo.split_whitespace().filter_map(|x| A17_PROPS.iter().find(|(n, _)| *n == x).map(|(_, b)| *b)).fold(0, |a, b| a | b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marca_do_scc_separa_blob_compilado_do_vanilla() {
        assert!(blob_compilado_pelo_scc(b"RDSC...$REDSCRIPT_SIGNATURE..."));
        assert!(!blob_compilado_pelo_scc(b"RDSC... vanilla ..."));
        assert!(!blob_compilado_pelo_scc(b""));
    }

    #[test]
    fn props_do_patch_por_nome_viram_os_bits_e_traco_liga_todas() {
        for (nome, bit) in A17_PROPS {
            assert_eq!(a17_props_mascara(nome), bit, "{nome}");
        }
        let bits: Vec<u32> = A17_PROPS.iter().map(|(_, b)| *b).collect();
        assert!(bits.iter().all(|b| b.is_power_of_two()), "um bit por prop");
        assert_eq!(bits.iter().fold(0, |a, b| a | b).count_ones(), 17, "bits distintos");
        assert_eq!(a17_props_mascara(" entity  components "), A17_PROP_COMPONENTS | A17_PROP_ENTITY);
        assert_eq!(a17_props_mascara("visualTags nada"), A17_PROP_VISUALTAGS);
        assert_eq!(a17_props_mascara("Components"), 0, "nome é CName: maiúscula conta");
        assert_eq!(a17_props_mascara(""), 0);
        assert_eq!(a17_props_mascara(" - "), u32::MAX);
        assert_eq!(a17_props_mascara("- -"), 0);
    }

    #[test]
    fn fnv1a32_vetores_padrao() {
        // `fnv1a32` é a fonte única do runtime (hash de chave do HashMap do motor, `BwmsFNV1a32`).
        assert_eq!(fnv1a32(b""), 0x811c_9dc5);
        assert_eq!(fnv1a32(b"a"), 0xe40c_292c);
        assert_eq!(fnv1a32(b"foobar"), 0xbf9c_f968);
        assert_eq!(fnv1a32(&[0u8; 8]), 0x9be1_7165);
    }

    #[test]
    fn quest_phase_node_id_crc16_ccitt_false_check_value() {
        // Valor-de-checagem PADRÃO da indústria pro CRC-16/CCITT-FALSE — confirma que a leitura
        // do algoritmo do C++ real (App::QuestPhaseExtension::GeneratePhaseNodeID) está certa
        // por uma 2ª fonte independente, não só transcrição.
        assert_eq!(quest_phase_node_id(b"123456789"), 0x29B1);
    }

    #[test]
    fn quest_phase_node_id_vazio_e_determinismo() {
        // Vazio = o init puro (nunca entra no loop).
        assert_eq!(quest_phase_node_id(b""), 0xFFFF);
        // Determinístico: mesma entrada -> mesmo ID sempre (propriedade que o sistema de patch
        // de grafo depende pra "mesmo mod, mesmo parent" nunca colidir consigo mesmo entre boots).
        let key = b"mymod\\phase.questphase:12";
        assert_eq!(quest_phase_node_id(key), quest_phase_node_id(key));
    }

    #[test]
    fn quest_phase_node_id_sensivel_a_1_byte() {
        // Muda 1 char da chave (ex. o parentID no sufixo ":N") -> ID diferente (evita reusar
        // acidentalmente o mesmo node-id pra parents diferentes na mesma fase).
        let a = quest_phase_node_id(b"mymod\\phase.questphase:0");
        let b = quest_phase_node_id(b"mymod\\phase.questphase:1");
        assert_ne!(a, b);
    }

    #[test]
    fn fnv1a64_vetores_padrao() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn fnv1a64_seeded_encadeia_por_pedacos() {
        let palavras = [0u64, 1, 0x1111, u64::MAX, 0x0123_4567_89ab_cdef];
        let pixels: [[u8; 4]; 3] = [[0, 0, 0, 255], [255, 26, 26, 255], [89, 89, 89, 255]];
        let mut tudo = Vec::new();
        palavras.iter().for_each(|v| tudo.extend_from_slice(&v.to_le_bytes()));
        pixels.iter().for_each(|p| tudo.extend_from_slice(p));
        let mut encadeado = FNV1A64_OFFSET;
        for v in palavras {
            encadeado = fnv1a64_seeded(&v.to_le_bytes(), encadeado);
        }
        for p in pixels {
            encadeado = fnv1a64_seeded(&p, encadeado);
        }
        assert_eq!(encadeado, fnv1a64(&tudo));
        assert_eq!(fnv1a64_seeded(&[], 0x1234), 0x1234, "pedaço vazio não mexe no seed");
    }

    #[test]
    fn lockey_chave_vetores() {
        assert_eq!(lockey_chave("LocKey#45371"), 45371);
        assert_eq!(lockey_chave("LocKey#abc"), fnv1a64(b"abc"));
        assert_eq!(lockey_chave("abc"), fnv1a64(b"abc"));
        // A chave que o runtime grava (`escritor.rs` e `natives.rs` do cp77-console), fixada para o F3-04 trocar o runtime
        // por esta função sem mudar chave. O conversor YAML do TweakXL dá 45371, 45371, 0 e 0 nestes quatro (TODO na função).
        assert_eq!(lockey_chave("45371"), fnv1a64(b"45371"));
        assert_eq!(lockey_chave("LocKey# 45371"), fnv1a64(b" 45371"));
        assert_eq!(lockey_chave(""), fnv1a64(b""));
        assert_eq!(lockey_chave("LocKey#"), fnv1a64(b""));
    }

    // ===== `resource_path_hash` — item #395 (`cp77-symbols/notes/CATALOGO-EXAUSTIVO-RED4EXT.md`): as 5 regras de
    // sanitização, testadas por AUTOCONSISTÊNCIA (path "sujo" deve hashear IGUAL ao equivalente
    // já limpo à mão — não temos golden real do motor pra um path sujo, mas a regra em si é
    // mecânica/determinística) +
    // regressão dos 5 goldens reais (replicados aqui, fonte: `instalacao/tabelas_de_runtime.rs`).
    #[test]
    fn resource_path_hash_goldens_reais_preservados() {
        // 5/5 goldens reais do índice de `basegame_zzbwms_quadra_e3.archive` — o fix NÃO pode
        // mudar o hash de paths já limpos (sem barra inicial/dupla/aspas, <216 chars).
        let g = [
            ("base\\surfaces\\materials\\default\\white.xbm", 0x3959_590b_0d88_8df1u64),
            ("base\\vehicles\\common\\wheels\\wheel_quadra_turbo_clean_01.mlsetup", 0xc007_6ea3_a1d9_a9edu64),
            ("base\\surfaces\\microblends\\default.xbm", 0xf78f_7031_24ea_fb22u64),
            ("base\\surfaces\\materials\\metal\\metal_generic\\metal_generic_bare_01_300_d.xbm", 0x79c9_5673_c0d4_c73au64),
            ("base\\resource.cooked_mlsetup", 0x3a12_b4fd_1938_d5cau64),
        ];
        for (p, h) in g {
            assert_eq!(resource_path_hash(p), h, "hash de {p} mudou — fix quebrou regressão");
        }
    }

    #[test]
    fn resource_path_hash_strip_barra_inicial() {
        let clean = resource_path_hash("base\\characters\\foo.mesh");
        assert_eq!(resource_path_hash("/base\\characters\\foo.mesh"), clean);
        assert_eq!(resource_path_hash("\\base\\characters\\foo.mesh"), clean);
        // TODAS as iniciais, não só 1.
        assert_eq!(resource_path_hash("///base\\characters\\foo.mesh"), clean);
        assert_eq!(resource_path_hash("\\\\\\base\\characters\\foo.mesh"), clean);
    }

    #[test]
    fn resource_path_hash_colapsa_barras_repetidas() {
        let clean = resource_path_hash("base\\characters\\foo.mesh");
        assert_eq!(resource_path_hash("base//characters\\foo.mesh"), clean);
        assert_eq!(resource_path_hash("base\\\\characters\\foo.mesh"), clean);
        // mistura / e \ repetidos também colapsa (ambos normalizam pra \ antes da checagem).
        assert_eq!(resource_path_hash("base/\\characters\\foo.mesh"), clean);
    }

    #[test]
    fn resource_path_hash_strip_aspas() {
        let clean = resource_path_hash("base\\characters\\foo.mesh");
        assert_eq!(resource_path_hash("\"base\\characters\\foo.mesh\""), clean);
        assert_eq!(resource_path_hash("'base\\characters\\foo.mesh'"), clean);
        // só 1 de cada lado é removida (a regra é "descarta 1 aspas de abertura/fechamento").
        assert_ne!(resource_path_hash("\"\"base\\characters\\foo.mesh\"\""), clean);
    }

    #[test]
    fn resource_path_hash_trunca_216_bytes() {
        let long_clean = "a".repeat(216);
        let over = format!("{}extra_que_devia_sumir", "a".repeat(216));
        assert_eq!(resource_path_hash(&over), resource_path_hash(&long_clean));
    }

    #[test]
    fn resource_path_hash_string_vazia_ou_toda_descartada() {
        assert_eq!(resource_path_hash(""), 0);
        assert_eq!(resource_path_hash("/"), 0);
        assert_eq!(resource_path_hash("////"), 0);
        assert_eq!(resource_path_hash("\""), 0);
        // Aspa de abertura e de fechamento somem e não sobra nada.
        assert_eq!(resource_path_hash("''"), 0);
    }

    #[test]
    fn resource_path_hash_corte_e_normalizacao_contra_fnv_cru() {
        // O corte de 216 cai num separador e a barra repetida que vem depois some no colapso.
        let a215 = "a".repeat(215);
        assert_eq!(resource_path_hash(&format!("{a215}//b")), fnv1a64(format!("{a215}\\").as_bytes()));
        // `/` vira `\` e a caixa desce.
        assert_eq!(resource_path_hash("A/B"), fnv1a64(b"a\\b"));
        // A aspa de fechamento sai antes do corte.
        assert_eq!(resource_path_hash(&format!("{}'", "a".repeat(300))), fnv1a64("a".repeat(216).as_bytes()));
    }

    // Caminho de 302 bytes com aspas, barra inicial, barra repetida e maiúsculas: passa por todas as regras e pelo corte de 216.
    const CAMINHO_LONGO: &str = concat!(
        "'/Base//",
        "Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/",
        "Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/",
        "Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/",
        "Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Pasta_De_Teste/Fim.mesh'",
    );

    #[test]
    fn hash_avaliado_em_const_e_igual_ao_de_runtime() {
        use std::hint::black_box;
        // O avaliador de const do compilador e o código gerado têm de dar o mesmo hash: as constantes de escopo
        // calculadas na compilação (apply-12, XLP-10) dependem disso. `black_box` força o lado de runtime.
        const ENTRADAS: [&str; 6] = [
            "player_customization.app",
            "#non_existing_part.ent",
            "'/Base//Characters\\Foo.mesh'",
            "",
            "''",
            CAMINHO_LONGO,
        ];
        const RESOURCE: [u64; 6] = [
            resource_path_hash(ENTRADAS[0]),
            resource_path_hash(ENTRADAS[1]),
            resource_path_hash(ENTRADAS[2]),
            resource_path_hash(ENTRADAS[3]),
            resource_path_hash(ENTRADAS[4]),
            resource_path_hash(ENTRADAS[5]),
        ];
        const FNV: [u64; 6] = [
            fnv1a64(ENTRADAS[0].as_bytes()),
            fnv1a64(ENTRADAS[1].as_bytes()),
            fnv1a64(ENTRADAS[2].as_bytes()),
            fnv1a64(ENTRADAS[3].as_bytes()),
            fnv1a64(ENTRADAS[4].as_bytes()),
            fnv1a64(ENTRADAS[5].as_bytes()),
        ];
        const SEMEADO: u64 = fnv1a64_seeded(b"abc", 0x1234_5678_9abc_def0);
        assert_eq!(CAMINHO_LONGO.len(), 302);
        for i in 0..ENTRADAS.len() {
            let e = black_box(ENTRADAS[i]);
            assert_eq!(RESOURCE[i], resource_path_hash(e), "resource_path_hash({e:?})");
            assert_eq!(FNV[i], fnv1a64(e.as_bytes()), "fnv1a64({e:?})");
        }
        assert_eq!(SEMEADO, fnv1a64_seeded(black_box(b"abc"), black_box(0x1234_5678_9abc_def0)));
        // Os dois caminhos fixos do runtime (escopo de customização e parte fictícia, apply-12): minúsculos e sem
        // separador, o hash do caminho é o FNV cru.
        assert_eq!(RESOURCE[0], 0x4553_8d31_fbc3_c9bb);
        assert_eq!(RESOURCE[1], 0xf357_c78b_9052_272e);
        assert_eq!(RESOURCE[0], FNV[0]);
        assert_eq!(RESOURCE[1], FNV[1]);
        assert_eq!(RESOURCE[2], fnv1a64(b"base\\characters\\foo.mesh"));
        assert_eq!(RESOURCE[3], 0);
        assert_eq!(RESOURCE[4], 0);
    }

    #[test]
    fn crc32_vetor_padrao() {
        // Vetor de referência clássico do CRC-32 IEEE.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn tweak_db_id_empacota_len() {
        let id = tweak_db_id("Items.Test");
        assert_eq!(id & 0xFFFF_FFFF, u64::from(crc32(b"Items.Test")));
        assert_eq!(id >> 32, 10); // "Items.Test".len()
    }

    #[test]
    fn tweak_db_id_length_trunca_u8() {
        // O campo length é u8 (byte 4); nome de 256 chars → length wrap p/ 0, e nada vaza pros
        // bytes 5-7 (tdbOffset). Bate com o `static_cast<uint8_t>` do C++.
        let n256 = "a".repeat(256);
        let id = tweak_db_id(&n256);
        assert_eq!(id >> 32, 0); // 256 & 0xFF == 0
        assert_eq!(id & 0xFFFF_FFFF, u64::from(crc32(n256.as_bytes())));
        let n300 = "b".repeat(300);
        assert_eq!(tweak_db_id(&n300) >> 32, 300 & 0xFF); // == 44
    }

    #[test]
    fn tweak_db_id_chave_tira_so_o_tdboffset() {
        let longo = "a".repeat(256);
        for nome in ["Items.Test", "AttachmentSlots.Head", "", longo.as_str()] {
            let id = tweak_db_id(nome);
            assert_eq!(tweak_db_id_chave(id), id);
            for off in [0x00_0001u64, 0xAB_CDEF, 0xFF_FFFF] {
                assert_eq!(tweak_db_id_chave(id | (off << 40)), id);
            }
        }
        assert_eq!(tweak_db_id_chave(u64::MAX), 0xFF_FFFF_FFFF);
        const CHAVE_EM_CONST: u64 = tweak_db_id_chave(0x12_34AB_CDEF_0123);
        assert_eq!(CHAVE_EM_CONST, 0xAB_CDEF_0123);
    }

    #[test]
    fn murmur3_vetores_wolvenkit() {
        // Vetores reais do WolvenKit (Tests/.../Murmur3Tests.cs), seed RECORDS_SEED.
        assert_eq!(murmur3_32(b"", 0), 0);
        assert_eq!(murmur3_32(b"Records", RECORDS_SEED), 0x92C1_A109);
        assert_eq!(murmur3_32(b"TweakDB", RECORDS_SEED), 0xF185_1BEB);
        assert_eq!(murmur3_32(b"WolvenKit", RECORDS_SEED), 0x131C_522F);
        assert_eq!(murmur3_32(b"Hello!", RECORDS_SEED), 0xFA23_C62F);
        assert_eq!(
            murmur3_32(b"This is a longer string than usual.", RECORDS_SEED),
            0x85D8_23B6
        );
    }

    #[test]
    fn crc64_xz_e_sha1_kat() {
        // CRC-64/XZ check value + SHA-1 FIPS 180-1.
        assert_eq!(crc64(b"123456789"), 0x995d_c9bb_df19_39fa);
        assert_eq!(crc64(b""), 0);
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        let e: String = sha1(b"").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(e, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn sha1_kat_nas_fronteiras_do_padding() {
        // Resto < 56 cabe num bloco de cauda; resto >= 56 pede dois. Valores do `shasum -a 1` do sistema.
        let hex = |d: &[u8]| sha1(d).iter().map(|b| format!("{b:02x}")).collect::<String>();
        let kats = [
            (55, "c1c8bbdc22796e28c0e15163d20899b65621d65a"),
            (56, "c2db330f6083854c99d4b5bfb6e8f29f201be699"),
            (63, "03f09f5b158a7a8cdad920bddc29b81c18a551f5"),
            (64, "0098ba824b5c16427bd7a1122a5a442a25ec644d"),
            (65, "11655326c708d70319be2610e8a57d9a5b959d3b"),
            (119, "ee971065aaa017e0632a8ca6c77bb3bf8b1dfc56"),
            (120, "f34c1488385346a55709ba056ddd08280dd4c6d6"),
            (128, "ad5b3fdbcb526778c2839d2f151ea753995e26a0"),
        ];
        for (n, esperado) in kats {
            assert_eq!(hex(&vec![b'a'; n]), esperado, "sha1 de {n}×'a'");
        }
        assert_eq!(hex(b""), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        // FIPS 180-1, exemplo de dois blocos (56 bytes).
        assert_eq!(hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"), "84983e441c3bd26ebaae4aa1f95129e5e54670f1");
        // FIPS 180-1, um milhão de 'a'.
        assert_eq!(hex(&vec![b'a'; 1_000_000]), "34aa973cd4c4daa4f61eeb2bdbad27316534016f");
    }

    #[test]
    fn record_type_key_strip() {
        // O regex `gamedata(.*)_Record` do WolvenKit: hasheia só o miolo.
        assert_eq!(
            record_type_key("gamedataWeaponItem_Record"),
            murmur3_32(b"WeaponItem", RECORDS_SEED)
        );
        // Já-stripado passa direto.
        assert_eq!(
            record_type_key("WeaponItem"),
            murmur3_32(b"WeaponItem", RECORDS_SEED)
        );
    }

    #[test]
    fn record_type_key_valor_ancorado() {
        // GOLDEN ABSOLUTO (não só self-consistente): trava regressão de seed/murmur. Se o
        // RECORDS_SEED ou o murmur3 quebrarem, CreateRecord/clone quebram em silêncio — este
        // valor (confirmado in-game pro gamedataWeaponItem_Record) pega antes do boot.
        assert_eq!(record_type_key("gamedataWeaponItem_Record"), 0x7fde_f930);
    }

    #[test]
    fn crc32_seeded_equivale_crc32() {
        // seed=0 é o CRC-32 IEEE clássico.
        assert_eq!(crc32_seeded(b"123456789", 0), 0xCBF4_3926);
        assert_eq!(crc32_seeded(b"", 0), 0);
        assert_eq!(crc32_seeded(b"Items.Test", 0), crc32(b"Items.Test"));
    }

    #[test]
    fn crc32_seeded_telescopa() {
        // A propriedade que faz a derivação de TweakDBID funcionar:
        // crc_seeded(B, crc_seeded(A, 0)) == crc_seeded(A++B, 0).
        let a = b"Items.Preset_Lexington";
        let b = b".Cool";
        let mut concat = a.to_vec();
        concat.extend_from_slice(b);
        assert_eq!(
            crc32_seeded(b, crc32_seeded(a, 0)),
            crc32_seeded(&concat, 0)
        );
    }

    #[test]
    fn tweak_db_id_derive_bate_com_direto() {
        // O invariante do RegisterName/CreateExtraNames do TweakXL: derivar do pai + sufixo dá
        // o MESMO id que hashear o nome completo. Prova reversa da derivação (não só forward).
        assert_eq!(
            tweak_db_id_derive(tweak_db_id("Items.Preset_Lexington"), ".Cool"),
            tweak_db_id("Items.Preset_Lexington.Cool")
        );
        assert_eq!(
            tweak_db_id_derive(tweak_db_id("Base"), ".child"),
            tweak_db_id("Base.child")
        );
        // Golden absoluto do exemplo (hash|len<<32).
        assert_eq!(
            tweak_db_id_derive(tweak_db_id("Items.Preset_Lexington"), ".Cool"),
            0x1b_32b5_1cd8
        );
    }

    /// Oráculo do [`crc32_seeded`]: a definição do CRC-32 IEEE refletido, um bit por volta, sem tabela.
    fn crc32_seeded_bit_a_bit(bytes: &[u8], seed: u32) -> u32 {
        let mut crc = !seed;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    #[test]
    fn crc32_tabela_bate_com_bit_a_bit() {
        // Valores conhecidos da tabela do CRC-32 IEEE (zlib `crc_table`): pega poly ou reflexão trocados.
        assert_eq!(CRC32_TABELA[0], 0);
        assert_eq!(CRC32_TABELA[1], 0x7707_3096);
        assert_eq!(CRC32_TABELA[128], 0xEDB8_8320);
        assert_eq!(CRC32_TABELA[255], 0x2D02_EF8D);
        // Todo byte sozinho e todo comprimento de 0 a 300 B, com seeds de canto e o seed de um prefixo
        // (a derivação de TweakDBID continua de um CRC já feito).
        let dados: Vec<u8> = (0..300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let seeds = [0, 1, 0xFFFF_FFFF, 0xDEAD_BEEF, crc32(b"Items.Preset_Lexington")];
        for seed in seeds {
            for b in 0..=255u8 {
                assert_eq!(crc32_seeded(&[b], seed), crc32_seeded_bit_a_bit(&[b], seed), "byte {b:#04x} seed {seed:#x}");
            }
            for n in 0..=dados.len() {
                assert_eq!(crc32_seeded(&dados[..n], seed), crc32_seeded_bit_a_bit(&dados[..n], seed), "{n} B seed {seed:#x}");
            }
        }
    }

    #[test]
    fn node_ref_hash_roots_e_vazios() {
        // GlobalRoot / RelativeRoot: sem char especial, é FNV-1a64 cru do char.
        assert_eq!(node_ref_hash("$"), fnv1a64(b"$"));
        assert_eq!(node_ref_hash("~"), fnv1a64(b"~"));
        // Goldens absolutos (do header RED4ext).
        assert_eq!(node_ref_hash("$"), 0xaf63_994c_8601_7ab3);
        assert_eq!(node_ref_hash("~"), 0xaf63_f34c_8602_13a1);
        // Nada hasheado → 0 (não o seed).
        assert_eq!(node_ref_hash(""), 0);
        assert_eq!(node_ref_hash("#"), 0);
    }

    #[test]
    fn node_ref_hash_skip_rules() {
        // '#' (nó dinâmico) é pulado inteiro: "#foo" == fnv1a64("foo").
        assert_eq!(node_ref_hash("#foo"), fnv1a64(b"foo"));
        // ';alias' pulado até o '/', mas o '/' É hasheado: "a;xyz/b" == fnv1a64("a/b").
        assert_eq!(node_ref_hash("a;xyz/b"), fnv1a64(b"a/b"));
        // Combinado, contra o header.
        assert_eq!(node_ref_hash("#foo"), 0xdcb2_7518_fed9_d577);
        assert_eq!(node_ref_hash("a;xyz/b"), 0xe620_c319_0468_cf61);
    }

    /// `apply-N6`: uma regra para o apply, o jogo e a reconstrução do `tweakdb.bin`: extensão em maiúscula entra, `.toml` fica fora.
    #[test]
    fn formato_tweak_sem_diferenciar_maiuscula_e_sem_toml() {
        for ext in ["yaml", "YAML", "yml", "Yml"] {
            assert_eq!(formato_tweak(ext), Some(FormatoTweak::Yaml), "{ext}");
        }
        for ext in ["tweak", "TWEAK", "Tweak"] {
            assert_eq!(formato_tweak(ext), Some(FormatoTweak::Tweak), "{ext}");
        }
        for ext in ["toml", "TOML", "reds", "archive", "xl", "", ".yaml", "yaml ", "yamlx"] {
            assert_eq!(formato_tweak(ext), None, "{ext:?}");
        }
    }

    // Medição: `cargo test --release -- --ignored --nocapture bench_resource_path_hash`.
    #[test]
    #[ignore]
    fn bench_resource_path_hash() {
        use std::hint::black_box;
        use std::time::Instant;
        // Paths de 30 a 120 bytes nas formas que chegam ao hash: limpos, com `/`, maiúsculas e barra inicial.
        let tipicos = [
            "base\\gameplay\\factories\\vehicles\\vehicles.csv",
            "Base/Surfaces/Materials/Default/White.xbm",
            "\\base\\vehicles\\common\\wheels\\wheel_quadra_turbo_clean_01.mlsetup",
            "base\\localization\\pt-br\\onscreens\\onscreens.json",
            "base\\characters\\common\\player_base_bodies\\player_female_average\\t0_000_pwa_base__full.mesh",
            "mymod/items/jackets/custom_jacket_with_a_long_name_for_testing/appearance_variant_black_leather.app",
        ];
        let longo = format!("base\\{}fim.mesh", "Pasta_De_Teste/".repeat(19));
        assert!(tipicos.iter().all(|p| (30..=120).contains(&p.len())));
        assert_eq!(longo.len(), 298);

        const N: usize = 1_000_000;
        let t = Instant::now();
        for _ in 0..N {
            for p in &tipicos {
                black_box(resource_path_hash(black_box(p)));
            }
        }
        let dt = t.elapsed();
        let chamadas = N * tipicos.len();
        println!("bench_resource_path_hash tipicos: {chamadas} chamadas em {dt:?} = {:.2} ns/chamada", dt.as_nanos() as f64 / chamadas as f64);

        let t = Instant::now();
        for _ in 0..N {
            black_box(resource_path_hash(black_box(&longo)));
        }
        let dt = t.elapsed();
        println!("bench_resource_path_hash 298 B: {N} chamadas em {dt:?} = {:.2} ns/chamada", dt.as_nanos() as f64 / N as f64);
    }

    // Medição: `cargo test --release -- --ignored --nocapture bench_sha1`.
    #[test]
    #[ignore]
    fn bench_sha1() {
        use std::hint::black_box;
        use std::time::Instant;
        for (rotulo, tamanho, n) in [("1 KB", 1usize << 10, 200_000usize), ("1 MB", 1 << 20, 200)] {
            let dados: Vec<u8> = (0..tamanho).map(|i| (i * 31 + 7) as u8).collect();
            let t = Instant::now();
            for _ in 0..n {
                black_box(sha1(black_box(&dados)));
            }
            let dt = t.elapsed();
            let mb_s = (tamanho * n) as f64 / dt.as_secs_f64() / 1e6;
            println!("bench_sha1 {rotulo}: {n} chamadas em {dt:?} = {:.2} us/chamada, {mb_s:.1} MB/s", dt.as_secs_f64() * 1e6 / n as f64);
        }
    }

    // Medição: `cargo test --release -- --ignored --nocapture bench_crc32`, com
    // `CARGO_PROFILE_RELEASE_OPT_LEVEL=s` para o perfil `publico` do cp77-console e `=2` para o `release`.
    #[test]
    #[ignore]
    fn bench_crc32() {
        use std::hint::black_box;
        use std::time::Instant;
        // Nomes e sufixos que o runtime passa a `tweak_db_id` e `tweak_db_id_derive` (cp77-console/src).
        let nomes: &[&[u8]] = &[
            b"Items.money",
            b"Quality.Legendary",
            b"AttachmentSlots.Torso",
            b"Items.Coat_04_rich_02_Crafting",
            b"itemsFactoryAppearanceSuffix.Gender",
            b"Items.Preset_Yukimura_Default.quality",
            b"Items.GrenadeIncendiarySticky.deepWaterDepth",
        ];
        let sufixos: &[&[u8]] = &[b".enumName", b".parentSlot", b".appearanceName", b".entityTemplatePath", b".crowdAppearanceNames"];
        // Pedaços de CR2W no `.archive`: o header que `header_crc32` monta tem 160 B; as tabelas variam com o recurso.
        let cr2w: Vec<Vec<u8>> = [160usize, 4 << 10, 1 << 20]
            .iter()
            .map(|&n| (0..n).map(|i| (i * 31 + 7) as u8).collect())
            .collect();
        let rodar = |f: fn(&[u8], u32) -> u32, grupo: &[&[u8]], voltas: usize| -> (f64, usize) {
            let f = black_box(f);
            let t = Instant::now();
            for _ in 0..voltas {
                for e in grupo {
                    black_box(f(black_box(e), black_box(0x1234_5678)));
                }
            }
            (t.elapsed().as_nanos() as f64 / (voltas * grupo.len()) as f64, grupo.iter().map(|e| e.len()).sum::<usize>() / grupo.len())
        };
        let cr2w_160: Vec<&[u8]> = vec![&cr2w[0]];
        let cr2w_4k: Vec<&[u8]> = vec![&cr2w[1]];
        let cr2w_1m: Vec<&[u8]> = vec![&cr2w[2]];
        for (rotulo, grupo, voltas) in [
            ("nomes TweakDB 11-44 B", nomes, 1_000_000usize),
            ("sufixos 9-21 B", sufixos, 1_000_000),
            ("CR2W header 160 B", cr2w_160.as_slice(), 1_000_000),
            ("CR2W tabela 4 KiB", cr2w_4k.as_slice(), 50_000),
            ("CR2W tabela 1 MiB", cr2w_1m.as_slice(), 200),
        ] {
            let (bit, media) = rodar(crc32_seeded_bit_a_bit, grupo, voltas);
            let (tab, _) = rodar(crc32_seeded, grupo, voltas);
            println!(
                "bench_crc32 {rotulo} (média {media} B): bit a bit {bit:.2} ns, tabela {tab:.2} ns por chamada; {:.2} ns/B a menos, ganho {:.1}%",
                (bit - tab) / media as f64,
                (bit - tab) / bit * 100.0
            );
        }
    }
}
