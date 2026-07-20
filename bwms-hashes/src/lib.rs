//! Funções de hash do Cyberpunk 2077 — FONTE ÚNICA do runtime (cp77-console) E das ferramentas
//! offline (tweakdb-tool, bwms-core). TweakDBID = CRC32(nome)|(len<<32); type_key de record =
//! murmur3(miolo, seed 0x5EEDBA5E); ResourcePath (RDAR/ArchiveXL) = FNV-1a64 do path normalizado;
//! FNV-1a 32/64 crus. Zero-dep, só core. Antes cada uma era copiada em 2-3 lugares.

/// FNV-1a 64-bit sobre os bytes da string (mesma função dos paths do RDAR, sem normalizar).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    fnv1a64_seeded(bytes, 0xcbf2_9ce4_8422_2325)
}

/// FNV-1a 64-bit com hash INICIAL customizável (`seed`) — a mesma assinatura do `Red::FNV1a64`
/// real do Codeware (`App/Utils/Hashing.hpp`: `Red::Optional<uint64_t, 0xCBF29CE484222325>`,
/// ou seja, o valor default do parâmetro `opt seed` É o offset basis padrão do FNV — quando o
/// .reds chamador omite o seed, o COMPILADOR já embute esse default na chamada, então o
/// handler nativo nunca precisa adivinhar "seed==0 → usa o default"). `fnv1a64` acima é só
/// este com o offset basis padrão.
pub fn fnv1a64_seeded(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Hash de ResourcePath do RDAR (o que o depot usa p/ resolver .archive): FNV-1a 64-bit do path
/// NORMALIZADO (`/`→`\`, minúsculas). É o coração do resource.link/copy do ArchiveXL. PROVADO
/// 5/5 goldens reais (testes em `bwms-core/apply_xl.rs`). Fonte única do runtime E do offline.
pub fn resource_path_hash(path: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for b in path
        .bytes()
        .map(|c| if c == b'/' { b'\\' } else { c.to_ascii_lowercase() })
    {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Hash de um `NodeRef` do mundo (o que `ToNodeRef`/`ToEntityID`/spawns e scene-mods do Codeware
/// usam pra resolver um nó pelo path) — porte byte-exato do `NodeRef::Hash` do RED4ext
/// (`NodeRef.hpp`, também em `Codeware/src/Red/NodeRef.hpp`). É FNV-1a64 (seed/prime padrão) do
/// path, com DUAS regras de skip que o distinguem do `fnv1a64` cru:
/// - `#` (marcador de nó dinâmico) é PULADO — o char não entra no hash.
/// - `;alias` (um alias local) é PULADO até o `/` seguinte; o `/` terminador SIM entra no hash.
///   Se o `;alias` vai até o fim da string (sem `/`), o hash para ali.
/// E o caso-borda do engine: se NADA foi hasheado (string vazia ou toda pulada), retorna 0 em vez
/// do seed. `GlobalRoot`=`node_ref_hash("$")`, `RelativeRoot`=`node_ref_hash("~")`.
pub fn node_ref_hash(node_ref: &str) -> u64 {
    const SEED: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let bytes = node_ref.as_bytes();
    let mut hash = SEED;
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
        hash = hash.wrapping_mul(PRIME);
        i += 1;
    }
    if hash == SEED {
        0
    } else {
        hash
    }
}

/// FNV-1a 32-bit (offset 0x811c9dc5, prime 0x01000193). Usado pelo TweakXL no
/// `ComposeInlineName` (hash do nome sintético de um record inline).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    fnv1a32_seeded(bytes, 0x811c_9dc5)
}

/// FNV-1a 32-bit com hash inicial customizável — mesma lógica do `fnv1a64_seeded`, pro
/// `Red::FNV1a32` real do Codeware (default `0x811C9DC5` = o offset basis padrão).
pub fn fnv1a32_seeded(bytes: &[u8], seed: u32) -> u32 {
    let mut hash = seed;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// CRC-32 IEEE com hash INICIAL customizável (`seed`) — a assinatura exata do `CRC32(data, len,
/// seed)` do RED4ext (`Hashing/CRC.hpp`: `crc = ~seed; ...; return ~crc`). É a impl CANÔNICA;
/// [`crc32`] é só este com `seed=0`. O seed permite CONTINUAR um CRC de onde outro parou — a base
/// da derivação de TweakDBID ([`tweak_db_id_derive`]): como `~seed` no início e `~crc` no fim se
/// telescopam, `crc32_seeded(B, crc32_seeded(A, s)) == crc32_seeded(A++B, s)`.
pub fn crc32_seeded(bytes: &[u8], seed: u32) -> u32 {
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
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x6745_2301, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476, 0xC3D2_E1F0];
    let ml = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());
    for chunk in msg.chunks(64) {
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
    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_vetores_padrao() {
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
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
}
