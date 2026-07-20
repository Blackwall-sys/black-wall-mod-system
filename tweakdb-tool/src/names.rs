//! Resolução de `TweakDBID` → nome legível, via a `tweakdbstr.kark` do WolvenKit
//! (lista de nomes de records/flats/queries). O arquivo é KARK-comprimido; cada
//! nome vira `CRC32(nome) | (len<<32)` = o mesmo TweakDBID que aparece no
//! tweakdb.bin. Porta o `TweakDBStringHelper` do WolvenKit.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::hashes::tweak_db_id;

pub const STR_MAGIC: u32 = 0x0BB1_DB57;

pub struct NameDb {
    by_id: HashMap<u64, String>,
    pub records: usize,
    pub flats: usize,
    pub queries: usize,
}

impl NameDb {
    /// Carrega de um `.kark` (descomprime via Kraken) ou de um `.bin` já cru.
    pub fn load(path: &Path) -> Result<NameDb, String> {
        let raw = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let data = maybe_decompress(&raw)?;
        parse(&data)
    }

    pub fn resolve(&self, id: u64) -> Option<&str> {
        self.by_id.get(&id).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Nomes (e seus ids) que contêm `needle` (case-insensitive).
    pub fn search<'a>(&'a self, needle: &str) -> impl Iterator<Item = (u64, &'a str)> {
        let needle = needle.to_ascii_lowercase();
        self.by_id.iter().filter_map(move |(id, name)| {
            name.to_ascii_lowercase()
                .contains(&needle)
                .then_some((*id, name.as_str()))
        })
    }
}

fn maybe_decompress(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() >= 8
        && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == crate::kraken::KARK_MAGIC
    {
        let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        crate::kraken::decompress(&bytes[8..], size).map_err(|e| {
            format!(
                "descompressão Kraken da lista de nomes falhou ({e}). \
                 Compile com a feature `kraken` (default) ou passe um .bin já descomprimido."
            )
        })
    } else {
        Ok(bytes.to_vec())
    }
}

fn parse(data: &[u8]) -> Result<NameDb, String> {
    let mut c = Reader::new(data);
    if c.u32().ok_or("truncado")? != STR_MAGIC {
        return Err("magic da lista de nomes inválido (esperado 0x0BB1DB57)".into());
    }
    let _version = c.u32().ok_or("truncado")?;
    let records = c.u32().ok_or("truncado")? as usize;
    let flats = c.u32().ok_or("truncado")? as usize;
    let queries = c.u32().ok_or("truncado")? as usize;

    // NÃO reservar por contagem (vem do arquivo); cresce naturalmente.
    let mut by_id = HashMap::new();
    let total = records + flats + queries;
    for _ in 0..total {
        let name = c.lp_string().ok_or("string truncada na lista de nomes")?;
        by_id.entry(tweak_db_id(&name)).or_insert(name);
    }

    Ok(NameDb {
        by_id,
        records,
        flats,
        queries,
    })
}

/// Mini-leitor LE com string VLQ (mesma codificação do tweakdb).
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len())?;
        let s = &self.data[self.pos..end];
        self.pos = end;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// VLQ signed da CDPR (bit7=sinal, bit6=continuação no 1º octeto).
    fn vlq_i32(&mut self) -> Option<i32> {
        let b = self.u8()?;
        let negative = b & 0b1000_0000 != 0;
        let mut value: u32 = (b & 0b0011_1111) as u32;
        let mut more = b & 0b0100_0000 != 0;
        let mut shift = 6;
        while more {
            let nb = self.u8()?;
            value |= ((nb & 0b0111_1111) as u32) << shift;
            more = nb & 0b1000_0000 != 0;
            shift += 7;
            if shift > 35 {
                return None;
            }
        }
        let v = value as i32;
        Some(if negative { -v } else { v })
    }
    fn lp_string(&mut self) -> Option<String> {
        let prefix = self.vlq_i32()?;
        let len = prefix.unsigned_abs() as usize;
        if len == 0 {
            return Some(String::new());
        }
        if prefix > 0 {
            let bytes = self.take(len * 2)?;
            let u16s: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            Some(String::from_utf16_lossy(&u16s))
        } else {
            Some(String::from_utf8_lossy(self.take(len)?).into_owned())
        }
    }
}

#[cfg(all(test, feature = "kraken"))]
mod corpus_tests {
    use super::*;
    use crate::hashes::{tweak_db_id, tweak_db_id_derive};

    fn corpus_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../WolvenKit/WolvenKit.Common/Resources/tweakdbstr.kark")
    }

    /// PROVA REVERSA do `tweak_db_id_derive` contra o namespace INTEIRO do jogo: pra cada nome
    /// real `<base>.<sufixo>` da `tweakdbstr.kark`, derivar do pai + sufixo TEM que dar o mesmo id
    /// que hashear o nome cheio. A propriedade telescópica do CRC garante isso em teoria; este
    /// teste comprova em centenas de milhares de strings reais (multi-ponto, nomes longos perto do
    /// limite u8 do length), fechando o `tweakxl` DERIVE com o corpus, não com 2 vetores a dedo.
    #[test]
    fn derive_bate_com_direto_no_corpus_inteiro() {
        let path = corpus_path();
        let db = NameDb::load(&path).expect("carregar tweakdbstr.kark");
        assert!(db.len() > 100_000, "corpus pequeno demais: {}", db.len());

        let mut checked = 0usize;
        let mut mismatches = 0usize;
        let mut max_len = 0usize;
        let mut first_bad: Option<String> = None;
        for name in db.by_id.values() {
            max_len = max_len.max(name.len());
            // split no ÚLTIMO ponto (a última derivação de segmento); só nomes com base+sufixo.
            let Some(dot) = name.rfind('.') else { continue };
            if dot == 0 || dot + 1 >= name.len() {
                continue;
            }
            let base = &name[..dot];
            let suffix = &name[dot..]; // inclui o '.'
            let derived = tweak_db_id_derive(tweak_db_id(base), suffix);
            let direct = tweak_db_id(name);
            if derived != direct {
                mismatches += 1;
                if first_bad.is_none() {
                    first_bad = Some(name.clone());
                }
            }
            checked += 1;
        }
        println!(
            "[corpus] nomes={} record.flat-checados={} mismatches={} max_len={}",
            db.len(),
            checked,
            mismatches,
            max_len
        );
        assert!(checked > 50_000, "poucos nomes com ponto: {checked}");
        assert_eq!(mismatches, 0, "1º divergente: {first_bad:?}");
    }
}
