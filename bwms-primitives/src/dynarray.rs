//! `DynArray<T>` do RED: o cabeçalho de 16 bytes de toda lista do motor. Decodificação e crivo de
//! coerência moram aqui, uma vez só, para todo leitor concordar sobre o que é um `DynArray`: o
//! `garment_ctx` decodifica bytes já lidos, e o runtime lê o trio da memória viva
//! (`cp77-console`, `register::dynarray_em`) e julga com `register::dynarray_coerente`, que delega
//! para cá. Este módulo NÃO desreferencia nada.
//!
//! **Ordem dos campos: `entries · capacity · size`.**
//! ```text
//! T* entries;        // 0x00
//! uint32_t capacity; // 0x08
//! uint32_t size;     // 0x0C
//! ```
//! Evidência: o disassembly do validador de
//! script do motor (`0x1021fcd58`), que lê a contagem de `ScriptClass::functions` (array em `0x48`)
//! em `[x20+0x54]` = `0x48 + 0x0C`.
//!
//! **O erro de ordem é mudo.** Com os rótulos trocados, `size <= capacity` vira `capacity <= size`,
//! falso para todo array com folga: o crivo recusa o array lido certo e quem chama vê "vazio"
//! (`params=0`). Onde `capacity == size` (dado desserializado de disco) as duas
//! leituras dão o mesmo número, e é por isso que o erro passou despercebido.

/// Bytes do cabeçalho na memória.
pub const TAMANHO: usize = 0x10;

/// Um `DynArray<T>` do RED: ponteiro + capacidade + tamanho, campos crus.
///
/// Sem `Default`: zero é um array vazio legítimo, e uma leitura que falhou não pode virar esse
/// vazio por um `unwrap_or_default()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DynArray {
    pub entries: u64,
    /// `+0x08` — vem ANTES de `size` na memória.
    pub capacity: u32,
    /// `+0x0C`
    pub size: u32,
}

impl DynArray {
    /// Decodifica os 16 bytes que começam em `off`. `None` se faltar byte — nunca completa com zero,
    /// que transformaria buffer truncado em array vazio.
    pub fn decodificar(b: &[u8], off: usize) -> Option<DynArray> {
        let campo = |de: usize, ate: usize| b.get(off.checked_add(de)?..off.checked_add(ate)?);
        Some(DynArray {
            entries: u64::from_le_bytes(campo(0, 8)?.try_into().ok()?),
            capacity: u32::from_le_bytes(campo(8, 0x0C)?.try_into().ok()?),
            size: u32::from_le_bytes(campo(0x0C, TAMANHO)?.try_into().ok()?),
        })
    }

    /// Invariantes do próprio `DynArray`: `size <= capacity`, e ponteiro nulo só com `size == 0`.
    /// Violar qualquer uma prova que os 16 bytes NÃO são um `DynArray` — é offset errado lido como
    /// dado. Cumprir não prova o contrário: zeros cumprem, e qualquer outro `DynArray` também.
    pub fn estrutura_valida(&self) -> bool {
        self.size <= self.capacity && (self.entries != 0 || self.size == 0)
    }

    /// Heurística, não invariante: capacidade acima de 1M elementos é mais provável offset errado
    /// que array gigante de verdade. Vale também com ponteiro nulo — o ponteiro zerado não torna
    /// o número lido mais crível.
    pub fn capacidade_plausivel(&self) -> bool {
        self.capacity <= 1 << 20
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cabecalho(entries: u64, capacity: u32, size: u32) -> [u8; TAMANHO] {
        let mut b = [0u8; TAMANHO];
        b[0..8].copy_from_slice(&entries.to_le_bytes());
        b[8..12].copy_from_slice(&capacity.to_le_bytes());
        b[12..16].copy_from_slice(&size.to_le_bytes());
        b
    }

    /// Regressão do bug de 2026-09-06: `capacity` vem ANTES de `size`. Valores DIFERENTES de
    /// propósito — com `capacity == size` as duas leituras coincidem e o bug fica invisível.
    #[test]
    fn capacidade_antes_do_tamanho() {
        let a = DynArray::decodificar(&cabecalho(0xcafe, 64, 3), 0).unwrap();
        assert_eq!(a.entries, 0xcafe);
        assert_eq!(a.capacity, 64, "capacity mora em +0x08");
        assert_eq!(a.size, 3, "size mora em +0x0C");
        assert!(a.estrutura_valida(), "array com folga (size < capacity) tem estrutura válida");
    }

    #[test]
    fn decodifica_a_partir_do_offset() {
        let mut b = vec![0xEEu8; 0x30];
        b[0x20..0x30].copy_from_slice(&cabecalho(0x1111_0000, 8, 5));
        let a = DynArray::decodificar(&b, 0x20).unwrap();
        assert_eq!(a, DynArray { entries: 0x1111_0000, capacity: 8, size: 5 });
    }

    #[test]
    fn buffer_curto_e_none_nao_array_vazio() {
        assert!(DynArray::decodificar(&[0u8; TAMANHO - 1], 0).is_none());
        assert!(DynArray::decodificar(&[], 0).is_none());
        assert!(DynArray::decodificar(&[0u8; 0x20], 0x11).is_none(), "o cabeçalho termina fora do buffer");
        assert!(DynArray::decodificar(&[0u8; 0x20], usize::MAX).is_none(), "offset que transborda é None, não pânico");
    }

    #[test]
    fn capacidade_absurda_com_ponteiro_nulo_e_lixo() {
        let a = DynArray { entries: 0, capacity: 0x7FFF_FFFF, size: 0 };
        assert!(a.estrutura_valida());
        assert!(!a.capacidade_plausivel(), "ponteiro nulo não isenta a capacidade da heurística");
        // Nulo, vazio e com capacidade modesta continua aceito: recusar dependeria do realloc do
        // jogo no Mac, que não está medido.
        let modesto = DynArray { entries: 0, capacity: 8, size: 0 };
        assert!(modesto.estrutura_valida() && modesto.capacidade_plausivel());
    }

    /// Os dois crivos separados, nos trios que o runtime julga com `register::dynarray_coerente`
    /// (a mesma tabela está lá, contra a função do runtime).
    #[test]
    fn crivos_por_trio() {
        // (entries, size, capacity, estrutura_valida, capacidade_plausivel)
        let trios: [(u64, u32, u32, bool, bool); 8] = [
            (0x1000, 5, 4, false, true),
            (0x1000, 3, 0, false, true),
            (0, 1, 4, false, true),
            (0x1000, 1, (1 << 20) + 1, true, false),
            (0x1000, 1 << 20, 1 << 20, true, true),
            (0, 0, 0, true, true),
            (0x1000, 3, 64, true, true),
            (0x1000, 7, 7, true, true),
        ];
        for (entries, size, capacity, estrutura, capacidade) in trios {
            let a = DynArray { entries, capacity, size };
            assert_eq!(a.estrutura_valida(), estrutura, "estrutura_valida({entries:#x}, {size}, {capacity})");
            assert_eq!(a.capacidade_plausivel(), capacidade, "capacidade_plausivel({entries:#x}, {size}, {capacity})");
        }
    }
}
