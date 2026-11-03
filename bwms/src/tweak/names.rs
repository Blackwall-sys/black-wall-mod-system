//! Resolução de `TweakDBID` → nome legível, a partir de uma lista de nomes de records/flats do usuário.
//! Formato: texto UTF-8, um nome por linha; `#` comenta até o fim da linha; linha vazia é ignorada.
//! O `tweakdb.bin` só guarda o hash, então cada nome vira `tweak_db_id(nome)` aqui, o mesmo TweakDBID do arquivo.

use std::collections::HashMap;
use std::path::Path;

use bwms_primitives::tweak_db_id;

pub struct NameDb {
    by_id: HashMap<u64, String>,
}

impl NameDb {
    /// Carrega a lista de nomes do arquivo em `path`.
    pub fn load(path: &Path) -> Result<NameDb, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let texto = String::from_utf8(bytes)
            .map_err(|e| format!("{}: texto inválido em UTF-8 no byte {}", path.display(), e.utf8_error().valid_up_to()))?;
        Ok(NameDb::ler(&texto))
    }

    /// Lê a lista do texto. Nome repetido (mesmo TweakDBID): fica o primeiro.
    pub fn ler(texto: &str) -> NameDb {
        let mut by_id = HashMap::new();
        for linha in texto.lines() {
            let nome = linha.find('#').map_or(linha, |i| &linha[..i]).trim();
            if nome.is_empty() {
                continue;
            }
            by_id.entry(tweak_db_id(nome)).or_insert_with(|| nome.to_string());
        }
        NameDb { by_id }
    }

    /// Lista sem nomes: os ids aparecem pelo hash e `$base`/`$type` avisam que faltam nomes.
    pub fn vazia() -> NameDb {
        NameDb { by_id: HashMap::new() }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bwms_primitives::tweak_db_id_derive;

    const LISTA: &str = "\
# nomes do teste
Items.A
  Items.A.dano   # comentário depois do nome
\r
Items.Ação\r
Items.A
Items.B.tags
#Items.Comentado
";

    #[test]
    fn le_nomes_ignora_comentario_vazio_e_repetido() {
        let db = NameDb::ler(LISTA);
        assert_eq!(db.len(), 4);
        assert_eq!(db.resolve(tweak_db_id("Items.A")), Some("Items.A"));
        assert_eq!(db.resolve(tweak_db_id("Items.A.dano")), Some("Items.A.dano"));
        assert_eq!(db.resolve(tweak_db_id("Items.Ação")), Some("Items.Ação"));
        assert_eq!(db.resolve(tweak_db_id("Items.Comentado")), None);
        let mut achados: Vec<&str> = db.search("ITEMS.A").map(|(_, n)| n).collect();
        achados.sort_unstable();
        assert_eq!(achados, ["Items.A", "Items.A.dano", "Items.Ação"]);
    }

    /// O id de cada nome lido tem de ser o que o motor deriva do record pai + sufixo: é assim que um flat do
    /// `tweakdb.bin` (só o hash) casa com o nome da lista.
    #[test]
    fn id_lido_bate_com_a_derivacao_pelo_pai() {
        let db = NameDb::ler(LISTA);
        let mut conferidos = 0;
        for (id, nome) in db.search("") {
            let Some(ponto) = nome.rfind('.') else { continue };
            assert_eq!(tweak_db_id_derive(tweak_db_id(&nome[..ponto]), &nome[ponto..]), id, "{nome}");
            conferidos += 1;
        }
        assert_eq!(conferidos, 4);
    }

    #[test]
    fn arquivo_ausente_ou_binario_vira_erro() {
        let ausente = Path::new("/nao/existe/bwms-nomes.txt");
        assert!(NameDb::load(ausente).err().unwrap_or_default().starts_with("/nao/existe/bwms-nomes.txt: "));

        let p = std::env::temp_dir().join(format!("bwms-nomes-binario-{}.txt", std::process::id()));
        std::fs::write(&p, [b'I', 0xFF, b'\n']).unwrap();
        assert!(NameDb::load(&p).err().unwrap_or_default().contains("UTF-8"));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn carrega_do_arquivo() {
        let p = std::env::temp_dir().join(format!("bwms-nomes-{}.txt", std::process::id()));
        std::fs::write(&p, LISTA).unwrap();
        let db = NameDb::load(&p).unwrap();
        std::fs::remove_file(&p).ok();
        assert_eq!(db.len(), 4);
        assert_eq!(db.resolve(tweak_db_id("Items.B.tags")), Some("Items.B.tags"));
    }
}
