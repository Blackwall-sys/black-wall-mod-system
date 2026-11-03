//! Registro dos records que os mods criam: de qual record cada clone saiu (genealogia: original, pai original, descendentes) e o nome legível
//! de cada id que um mod escreveu — o TweakDB do jogo guarda só o hash. Anotado por quem cria: `$base`/`CloneRecord` anotam o pai, `$type`/
//! `CreateRecord` só marcam o record como de mod. As chaves são os 40 bits de baixo do `TweakDBID` (hash + tamanho do nome), sem o offset.
//!
//! A herança fica por flat: cada flat de um clone anota de qual flat da base herdou, e o escritor único leva a mudança de um flat aos
//! herdeiros que ainda usam o valor antigo (editar a base e a mudança chegar nos derivados que não a sobrescreveram).

use bwms_primitives::tweak_db_id_chave;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

#[derive(Default)]
struct Registro {
    criados: HashSet<u64>,
    pai: HashMap<u64, u64>,
    filhos: HashMap<u64, BTreeSet<u64>>,
    nomes: HashMap<u64, String>,
    /// flat da base → flats de clones que herdaram dele.
    herdeiros: HashMap<u64, BTreeSet<u64>>,
    /// record clone → (flat do clone, flat da base) de cada herança anotada.
    herancas: HashMap<u64, Vec<(u64, u64)>>,
}

static REGISTRO: Mutex<Option<Registro>> = Mutex::new(None);

/// O motor recarregou o TweakDB: os records de mod e as heranças anotadas eram do TweakDB velho.
pub(crate) fn esquecer_tudo() {
    *REGISTRO.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn com<R>(f: impl FnOnce(&mut Registro) -> R) -> R {
    let mut g = REGISTRO.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Registro::default))
}

impl Registro {
    fn criado(&mut self, id: u64) {
        self.criados.insert(tweak_db_id_chave(id));
        self.soltar_herancas(id);
    }

    fn herdou(&mut self, record: u64, flat: u64, flat_base: u64) {
        let (record, flat, flat_base) = (tweak_db_id_chave(record), tweak_db_id_chave(flat), tweak_db_id_chave(flat_base));
        if flat == flat_base {
            return;
        }
        self.herdeiros.entry(flat_base).or_default().insert(flat);
        let lista = self.herancas.entry(record).or_default();
        if !lista.contains(&(flat, flat_base)) {
            lista.push((flat, flat_base));
        }
    }

    fn soltar_herancas(&mut self, record: u64) {
        for (flat, base) in self.herancas.remove(&tweak_db_id_chave(record)).unwrap_or_default() {
            if let Some(h) = self.herdeiros.get_mut(&base) {
                h.remove(&flat);
            }
        }
    }

    fn clonado(&mut self, id: u64, base: u64) {
        let (id, base) = (tweak_db_id_chave(id), tweak_db_id_chave(base));
        if id == base {
            return;
        }
        self.criados.insert(id);
        if let Some(antigo) = self.pai.insert(id, base) {
            if let Some(f) = self.filhos.get_mut(&antigo) {
                f.remove(&id);
            }
        }
        self.filhos.entry(base).or_default().insert(id);
    }

    fn original(&self, id: u64) -> bool {
        !self.criados.contains(&tweak_db_id_chave(id))
    }

    /// O primeiro ancestral que não foi criado por mod; `None` pra record sem pai registrado (original, ou de `$type`).
    fn pai_original(&self, id: u64) -> Option<u64> {
        let mut atual = *self.pai.get(&tweak_db_id_chave(id))?;
        for _ in 0..256 {
            if !self.criados.contains(&atual) {
                return Some(atual);
            }
            match self.pai.get(&atual) {
                Some(p) => atual = *p,
                None => return None,
            }
        }
        None
    }

    /// Os records de mod que descendem de `base`, em qualquer geração, em ordem de chave.
    fn descendentes(&self, base: u64) -> Vec<u64> {
        let mut vistos = BTreeSet::new();
        let mut fila = vec![tweak_db_id_chave(base)];
        while let Some(atual) = fila.pop() {
            for f in self.filhos.get(&atual).into_iter().flatten() {
                if vistos.insert(*f) {
                    fila.push(*f);
                }
            }
        }
        vistos.into_iter().collect()
    }

    fn esquecer(&mut self, id: u64) {
        let id = tweak_db_id_chave(id);
        self.criados.remove(&id);
        if let Some(p) = self.pai.remove(&id) {
            if let Some(f) = self.filhos.get_mut(&p) {
                f.remove(&id);
            }
        }
        self.nomes.remove(&id);
        self.soltar_herancas(id);
    }
}

pub(crate) fn criado(id: u64) {
    com(|r| r.criado(id));
}
pub(crate) fn clonado(id: u64, base: u64) {
    com(|r| r.clonado(id, base));
}
pub(crate) fn original(id: u64) -> bool {
    com(|r| r.original(id))
}
/// Os records criados por mod (`$type`, `$base`, `CreateRecord`, `CloneRecord`).
pub(crate) fn criados() -> Vec<u64> {
    com(|r| r.criados.iter().copied().collect())
}
pub(crate) fn pai_original(id: u64) -> Option<u64> {
    com(|r| r.pai_original(id))
}
pub(crate) fn descendentes(base: u64) -> Vec<u64> {
    com(|r| r.descendentes(base))
}
/// O flat `flat` do clone `record` herdou de `flat_base`.
pub(crate) fn herdou(record: u64, flat: u64, flat_base: u64) {
    com(|r| r.herdou(record, flat, flat_base));
}
/// Flats de clones que herdaram de `flat`.
pub(crate) fn herdeiros(flat: u64) -> Vec<u64> {
    com(|r| r.herdeiros.get(&tweak_db_id_chave(flat)).map(|h| h.iter().copied().collect()).unwrap_or_default())
}
pub(crate) fn esquecer(id: u64) {
    com(|r| r.esquecer(id));
}

/// Guarda o nome de `id` (o hash do próprio nome, conferido: um nome que não gera o id não entra).
pub(crate) fn nomear(nome: &str) -> u64 {
    let id = bwms_primitives::tweak_db_id(nome);
    com(|r| r.nomes.insert(id, nome.to_string()));
    id
}
pub(crate) fn nome(id: u64) -> Option<String> {
    com(|r| r.nomes.get(&tweak_db_id_chave(id)).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn genealogia_de_clones() {
        let mut r = Registro::default();
        let (vanilla, a, b, c, tipo) = (0x10_0000_0001, 0x10_0000_0002, 0x10_0000_0003, 0x10_0000_0004, 0x10_0000_0005);
        r.clonado(a, vanilla);
        r.clonado(b, a);
        r.clonado(c, vanilla | (0x12_3456 << 40));
        r.criado(tipo);
        assert!(r.original(vanilla));
        assert!(!r.original(a) && !r.original(tipo));
        assert_eq!(r.pai_original(b), Some(vanilla));
        assert_eq!(r.pai_original(a), Some(vanilla));
        assert_eq!(r.pai_original(tipo), None);
        assert_eq!(r.descendentes(vanilla), vec![a, b, c]);
        assert_eq!(r.descendentes(a), vec![b]);
        r.clonado(b, c);
        assert_eq!(r.descendentes(a), Vec::<u64>::new());
        assert_eq!(r.descendentes(c), vec![b]);
        r.esquecer(b);
        assert!(r.original(b));
        assert_eq!(r.descendentes(vanilla), vec![a, c]);
    }

    #[test]
    fn heranca_por_flat() {
        let mut r = Registro::default();
        let (base, clone, tipo) = (0x20_0000_0001, 0x20_0000_0002, 0x20_0000_0003);
        let (fb, fc, ft) = (0x21_0000_0001, 0x21_0000_0002, 0x21_0000_0003);
        r.clonado(clone, base);
        r.herdou(clone, fc, fb);
        r.herdou(tipo, ft, fb);
        assert_eq!(r.herdeiros.get(&fb).map(|h| h.len()), Some(2));
        // `$type` marca o record como de mod e tira a herança do modelo.
        r.criado(tipo);
        assert_eq!(r.herdeiros.get(&fb).map(|h| h.iter().copied().collect::<Vec<_>>()), Some(vec![fc]));
        r.esquecer(clone);
        assert_eq!(r.herdeiros.get(&fb).map(|h| h.len()), Some(0));
    }

    /// `clonado(id, base)` pela entrada do crate (o registro do processo), com sentinelas que só existem aqui: o clone é `id`, o pai é `base`.
    #[test]
    fn clonado_anota_id_como_filho_de_base() {
        let (id, base) = (0x30_0000_0001u64, 0x30_0000_0002u64);
        clonado(id, base);
        assert!(!original(id), "o clone é o id");
        assert!(original(base), "a base continua original");
        assert_eq!(pai_original(id), Some(base));
        assert_eq!(pai_original(base), None);
        assert_eq!(descendentes(base), vec![id]);
        assert_eq!(descendentes(id), Vec::<u64>::new());
        esquecer(id);
    }

    #[test]
    fn nome_pelo_hash() {
        let id = nomear("Items.BwmsTesteRegistro");
        assert_eq!(id, bwms_primitives::tweak_db_id("Items.BwmsTesteRegistro"));
        assert_eq!(nome(id | (0xAB_CDEF << 40)).as_deref(), Some("Items.BwmsTesteRegistro"));
        assert_eq!(nome(0x42), None);
    }
}
