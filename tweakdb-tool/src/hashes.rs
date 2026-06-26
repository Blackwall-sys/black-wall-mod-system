//! Hashes do tweakdb. O `typeHash` de cada grupo de flats é o **FNV-1a 64-bit**
//! do nome RED do tipo (ex.: `"Float"`, `"array:Float"`). O CRC32 (nome→id de
//! TweakDBID) e o Murmur32 (chave de tipo de record) ficam para o merger.

/// FNV-1a 64-bit sobre os bytes da string (mesma função dos paths do RDAR).
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// FNV-1a 32-bit (offset 0x811c9dc5, prime 0x01000193). Usado pelo TweakXL no
/// `ComposeInlineName` (hash do nome sintético de um record inline).
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash = 0x811c_9dc5_u32;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// CRC-32 IEEE (poly 0xEDB88320, init 0xFFFFFFFF, xorout 0xFFFFFFFF) — o que o
/// WolvenKit (`Crc32Algorithm`) usa para os nomes do tweakdb.
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// TweakDBID de um nome: `CRC32(nome) | (len << 32)` (nome em ASCII).
pub fn tweak_db_id(name: &str) -> u64 {
    u64::from(crc32(name.as_bytes())) | ((name.len() as u64) << 32)
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
}
