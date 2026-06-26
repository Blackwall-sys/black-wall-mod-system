//! CName do CP2077 = FNV1a64 do nome flat (ASCII). Idêntico ao RED4ext/CET/
//! redscript — e ao `fnv1a64` do nosso tweakdb-tool. É como se resolve um tipo
//! ou função RED por nome em runtime (IRTTISystem::GetClass(CName), etc.).

pub const FNV1A64_OFFSET: u64 = 0xCBF2_9CE4_8422_2325;
pub const FNV1A64_PRIME: u64 = 0x0000_0100_0000_01B3;

/// CName(nome) = FNV1a64(nome). `"None"`/vazio → 0 no engine (tratado pelo caller).
pub fn cname(name: &str) -> u64 {
    let mut h = FNV1A64_OFFSET;
    for &b in name.as_bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV1A64_PRIME);
    }
    h
}

/// Pool de nomes (hash→string) pra `NameToString` reverter — espelha o CNamePool do
/// engine no lado Lua. Populado por `CName.add`/`ToCName`/`StringToName`. (Não escreve
/// no pool nativo do jogo; é o nosso espelho pra labels/lookups dos mods CET.)
static POOL: std::sync::Mutex<Option<std::collections::HashMap<u64, String>>> =
    std::sync::Mutex::new(None);

/// Interna o nome (string→hash) no nosso espelho e devolve o hash. Idempotente.
pub fn intern(name: &str) -> u64 {
    let h = cname(name);
    if let Ok(mut g) = POOL.lock() {
        g.get_or_insert_with(std::collections::HashMap::new)
            .entry(h)
            .or_insert_with(|| name.to_string());
    }
    h
}

/// CNamePool::Get(hash) → char* (C-string null-terminated) no engine. Reverte qualquer
/// CName, não só os internados por nós. Endereço sem símbolo (inlined) → VERSIONADO,
/// validar com um hash conhecido no boot. ⚠️ muda por patch do jogo.
const VM_CNAMEPOOL_GET: u64 = 0x1_0345_28e8;

/// Resolve um CName (hash FNV1a64) → nome via pool NATIVO do engine. "" se desconhecido,
/// "None" se hash 0. Variante C-string (lê só x0 e varre até o NUL) p/ robustez de ABI.
/// Só seguro DEPOIS da RTTI viva (o hook do executor já garante isso quando isto roda).
pub fn resolve_cname(hash: u64) -> String {
    if hash == 0 {
        return "None".into();
    }
    unsafe {
        type Get = unsafe extern "C" fn(u64) -> *const i8;
        let get: Get = core::mem::transmute(crate::rebase(VM_CNAMEPOOL_GET));
        let p = get(hash);
        if p.is_null() || !crate::gum::is_readable(p as *const std::ffi::c_void, 1) {
            return String::new();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// Reverte um hash pro nome: primeiro o espelho interno (intern), senão o pool NATIVO
/// (resolve_cname). None se nenhum resolver.
pub fn name_of(hash: u64) -> Option<String> {
    if let Ok(g) = POOL.lock() {
        if let Some(m) = g.as_ref() {
            if let Some(s) = m.get(&hash) {
                return Some(s.clone());
            }
        }
    }
    let s = resolve_cname(hash);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// CRC-32 IEEE (mesmo do tweakdb-tool). Usado no TweakDBID.
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

/// TweakDBID de um nome: `CRC32(nome) | (len << 32)` (8 bytes LE).
pub fn tweak_db_id(name: &str) -> u64 {
    u64::from(crc32(name.as_bytes())) | ((name.len() as u64) << 32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vetor_padrao_fnv() {
        // Vetores canônicos do FNV-1a 64.
        assert_eq!(cname(""), FNV1A64_OFFSET);
        assert_eq!(cname("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn cnames_dos_sistemas() {
        // Conferidos contra o cálculo Python no recon (mesmo algoritmo).
        assert_eq!(cname("gameStatsSystem"), 0x7617_74a5_71cc_8913);
        assert_eq!(cname("gameCameraSystem"), 0x73b1_5f77_7a57_3929);
        assert_eq!(cname("gameDamageSystem"), 0x3619_c142_d3ec_2e47);
        assert_eq!(cname("GetStatValue"), 0x2342_7ae3_52f8_9652);
    }
}
