//! Resolução de nomes: o RDAR guarda só o hash FNV-1a 64-bit do path REDengine
//! (minúsculas, separador `\`). Para mostrar nomes no datamap/extração montamos
//! um dicionário hash→path a partir de (a) uma lista opcional do usuário e
//! (b) os paths embutidos no LxrsFooter do próprio archive.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Devolve o texto de uma lista de paths: descomprime se vier `KARK`-comprimida
/// (formato `usedhashes.kark`), senão decodifica os bytes como texto.
fn decode_maybe_kark(bytes: &[u8]) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind};
    if bytes.len() >= 8
        && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == crate::archive::KARK_MAGIC
    {
        let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let decoded = crate::kraken::decompress(&bytes[8..], size)
            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("lista .kark: {e}")))?;
        Ok(String::from_utf8_lossy(&decoded).into_owned())
    } else {
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// FNV-1a 64-bit — a função de hash dos paths no RDAR.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Normaliza um path para a forma canônica do depósito REDengine — minúsculas,
/// separador barra-invertida — que é a forma que gera o hash do archive.
pub fn canonical(raw: &str) -> String {
    raw.to_ascii_lowercase().replace('/', "\\")
}

#[derive(Default)]
pub struct PathDictionary {
    by_hash: HashMap<u64, String>,
}

impl PathDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Carrega uma lista hash→path de um arquivo texto: um path por linha
    /// (`#` = comentário). Assume-se que cada linha já é um path de recurso;
    /// é normalizado antes de ser hasheado, então aceita `/` ou `\`.
    ///
    /// Aceita tanto texto puro quanto uma lista `KARK`-comprimida (como a
    /// `usedhashes.kark` do WolvenKit): se o arquivo começa com o magic `KARK`,
    /// é descomprimido via Kraken antes de parsear (requer a feature `kraken`).
    pub fn load_list(&mut self, path: &Path) -> std::io::Result<usize> {
        let bytes = fs::read(path)?;
        let text = decode_maybe_kark(&bytes)?;
        let mut added = 0;
        for line in text.lines() {
            let entry = line.trim();
            if entry.is_empty() || entry.starts_with('#') {
                continue;
            }
            self.insert_path(entry);
            added += 1;
        }
        Ok(added)
    }

    /// Insere um path (qualquer forma) sob seu hash canônico, guardando a forma
    /// canônica como nome de exibição.
    pub fn insert_path(&mut self, raw: &str) {
        let canon = canonical(raw);
        self.by_hash
            .entry(fnv1a64(canon.as_bytes()))
            .or_insert(canon);
    }

    pub fn resolve(&self, hash: u64) -> Option<&str> {
        self.by_hash.get(&hash).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a64_segue_o_vetor_padrao() {
        // Vetores de referência do FNV-1a 64 bits.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn dicionario_resolve_independente_do_separador() {
        let mut dict = PathDictionary::new();
        dict.insert_path("Base/Characters/V.mesh");
        // O hash é calculado sobre a forma canônica.
        let h = fnv1a64(b"base\\characters\\v.mesh");
        assert_eq!(dict.resolve(h), Some("base\\characters\\v.mesh"));
        assert_eq!(dict.resolve(123), None);
    }
}
