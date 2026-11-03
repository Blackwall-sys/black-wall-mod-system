//! Resolução de nomes: o RDAR guarda só o hash FNV-1a 64-bit do path REDengine
//! (minúsculas, separador `\`). Para mostrar nomes no datamap/extração montamos
//! um dicionário hash→path a partir de (a) uma lista opcional do usuário e
//! (b) os paths embutidos no rodapé de paths do próprio archive.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Devolve o texto de uma lista de paths: descomprime se vier `KARK`-comprimida
/// (formato `usedhashes.kark`), senão decodifica os bytes como texto.
pub fn decode_maybe_kark(bytes: &[u8]) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind};
    if bytes.len() >= 8
        && u32::from_le_bytes(bytes[0..4].try_into().unwrap()) == crate::archive::archive::KARK_MAGIC
    {
        let size = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let decoded = crate::archive::kraken::decompress(&bytes[8..], size)
            .map_err(|e| Error::new(ErrorKind::InvalidData, format!("lista .kark: {e}")))?;
        Ok(String::from_utf8_lossy(&decoded).into_owned())
    } else {
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }
}

/// FNV-1a 64-bit — a função de hash dos paths no RDAR. Fonte única: `bwms-primitives`.
pub use bwms_primitives::fnv1a64;

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
    /// `usedhashes.kark` da comunidade): se o arquivo começa com o magic `KARK`,
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

    /// Insere um path (qualquer forma) sob o hash que o depot do motor pede (`resource_path_hash`: aspas, barras
    /// iniciais, separador repetido e truncamento), guardando a forma canônica como nome de exibição.
    pub fn insert_path(&mut self, raw: &str) {
        self.by_hash
            .entry(bwms_primitives::resource_path_hash(raw))
            .or_insert_with(|| canonical(raw));
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

    /// Barra inicial e separador repetido: o hash tem de ser o que o depot pede, não o da forma só minúscula.
    #[test]
    fn dicionario_usa_o_hash_do_depot() {
        let mut dict = PathDictionary::new();
        dict.insert_path("/base//x.json");
        let do_depot = bwms_primitives::resource_path_hash("base\\x.json");
        assert_eq!(bwms_primitives::resource_path_hash("/base//x.json"), do_depot);
        assert!(dict.resolve(do_depot).is_some());
        assert_ne!(fnv1a64(canonical("/base//x.json").as_bytes()), do_depot);
    }
}
