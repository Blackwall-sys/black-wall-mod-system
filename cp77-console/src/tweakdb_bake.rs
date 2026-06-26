//! Leitura de flats do TweakDB via arquivo compacto pré-gerado (`tweakdb-tool bake`),
//! em vez de chamar o getter in-game (que TRAVA o jogo).
//!
//! Formato do arquivo (`$HOME/.blackwall_tweakdb.bin`):
//!   "BWTDB01\0" (8) + count u64 LE (8) + N × { id u64 LE, val u64 LE, tag u8 },
//!   ordenado por `id` (TweakDBID = crc32(path) | (len<<32)) p/ busca binária.
//!   tag: 1=Float (val = f32 bits), 2=Int (val = i64), 3=Bool, 4=CName (val = FNV1a64
//!   da string), 5=TweakDBID (val = u64). Gerado offline; valores reais, sem chamar o jogo.

use std::sync::OnceLock;

const MAGIC: &[u8; 8] = b"BWTDB01\0";
const HDR: usize = 16; // magic(8) + count(8)
const REC: usize = 17; // id(8) + val(8) + tag(1)

static DB: OnceLock<Option<Vec<u8>>> = OnceLock::new();

fn path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.blackwall_tweakdb.bin")
}

/// Carrega (uma vez) os bytes do arquivo, validando magic + tamanho.
fn data() -> Option<&'static [u8]> {
    DB.get_or_init(|| {
        let bytes = std::fs::read(path()).ok()?;
        if bytes.len() >= HDR && &bytes[0..8] == MAGIC {
            let count = u64::from_le_bytes(bytes[8..16].try_into().ok()?) as usize;
            if bytes.len() >= HDR + count * REC {
                return Some(bytes);
            }
        }
        None
    })
    .as_deref()
}

/// True se o arquivo de flats foi carregado com sucesso.
pub fn is_loaded() -> bool {
    data().is_some()
}

/// Busca um flat por TweakDBID. Retorna `(tag, val)` ou None.
/// Panic-safe: toda leitura usa `.get()` (arquivo malformado → None, nunca crash).
pub fn lookup(id: u64) -> Option<(u8, u64)> {
    let d = data()?;
    let count = u64::from_le_bytes(d.get(8..16)?.try_into().ok()?) as usize;
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = (lo + hi) / 2;
        let off = HDR + mid * REC;
        let rid = u64::from_le_bytes(d.get(off..off + 8)?.try_into().ok()?);
        if rid < id {
            lo = mid + 1;
        } else if rid > id {
            hi = mid;
        } else {
            let val = u64::from_le_bytes(d.get(off + 8..off + 16)?.try_into().ok()?);
            let tag = *d.get(off + 16)?;
            return Some((tag, val));
        }
    }
    None
}
