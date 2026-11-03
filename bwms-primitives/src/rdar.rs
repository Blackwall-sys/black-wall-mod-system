//! O cabeçalho do `.archive` (RDAR), o do índice e o `name_hash` de cada recurso, num leitor só para os que abrem `.archive`: o runtime
//! confere o arquivo antes de entregá-lo ao motor (`archive_valido`, em `cp77-console/src/arquivos_do_jogo`) e acha os recursos repetidos
//! entre mods da pasta `Mods/` (`hashes_do_archive`, no mesmo módulo), e o `bwms archive` lê o índice (`bwms/src/archive/archive.rs`). Cada
//! um soma a regra própria por cima: o runtime, a do assert do motor (versão, índice alinhado, teto de recursos); o `bwms archive`, a do
//! WolvenKit (tamanho declarado, teto do índice). Sem `panic` e sem alocação: roda no dylib público.

/// O primeiro u32 (little-endian) do `.archive`: os bytes `RDAR`.
pub const RDAR_MAGIC: u32 = 0x5241_4452;
/// Bytes do cabeçalho fixo: magic, versão, posição e tamanho do índice, posição e tamanho do debug, tamanho do arquivo.
pub const TAMANHO_DO_CABECALHO: usize = 40;
/// Bytes do cabeçalho do índice (offset e tamanho da tabela de recursos, crc64, as 3 contagens): o menor índice que o formato tem.
pub const TAMANHO_DO_CABECALHO_DO_INDICE: usize = 28;

/// Os campos do cabeçalho fixo, na ordem do arquivo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cabecalho {
    pub versao: u32,
    pub posicao_do_indice: u64,
    pub tamanho_do_indice: u32,
    pub posicao_do_debug: u64,
    pub tamanho_do_debug: u32,
    /// O tamanho do arquivo que o cabeçalho declara (0 = não declarado).
    pub tamanho_declarado: u64,
}

/// O cabeçalho no começo de `bytes`. `None` com menos de [`TAMANHO_DO_CABECALHO`] bytes ou sem [`RDAR_MAGIC`].
pub fn cabecalho(bytes: &[u8]) -> Option<Cabecalho> {
    let u32_em = |o: usize| Some(u32::from_le_bytes(bytes.get(o..o + 4)?.try_into().ok()?));
    let u64_em = |o: usize| Some(u64::from_le_bytes(bytes.get(o..o + 8)?.try_into().ok()?));
    if bytes.len() < TAMANHO_DO_CABECALHO || u32_em(0)? != RDAR_MAGIC {
        return None;
    }
    Some(Cabecalho {
        versao: u32_em(4)?,
        posicao_do_indice: u64_em(8)?,
        tamanho_do_indice: u32_em(16)?,
        posicao_do_debug: u64_em(20)?,
        tamanho_do_debug: u32_em(28)?,
        tamanho_declarado: u64_em(32)?,
    })
}

impl Cabecalho {
    /// O índice tem ao menos o cabeçalho dele e termina dentro de um arquivo de `tamanho` bytes. Posição + tamanho que estoura u64 não cabe.
    pub fn indice_cabe(&self, tamanho: u64) -> bool {
        self.tamanho_do_indice as usize >= TAMANHO_DO_CABECALHO_DO_INDICE
            && self.posicao_do_indice.checked_add(u64::from(self.tamanho_do_indice)).is_some_and(|fim| fim <= tamanho)
    }
}

/// Bytes de cada recurso na tabela do índice, logo depois do cabeçalho dele: `name_hash` (u64) primeiro, depois data, faixas de segmento e
/// de dependência e o sha1.
pub const TAMANHO_DO_RECURSO: usize = 56;

/// As três contagens do cabeçalho do índice, depois do offset e do tamanho da tabela de recursos e do crc64.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CabecalhoDoIndice {
    pub recursos: u32,
    pub segmentos: u32,
    pub dependencias: u32,
}

/// O cabeçalho no começo de `indice` (os bytes em `posicao_do_indice`). `None` com menos de [`TAMANHO_DO_CABECALHO_DO_INDICE`] bytes.
pub fn cabecalho_do_indice(indice: &[u8]) -> Option<CabecalhoDoIndice> {
    let u32_em = |o: usize| Some(u32::from_le_bytes(indice.get(o..o + 4)?.try_into().ok()?));
    if indice.len() < TAMANHO_DO_CABECALHO_DO_INDICE {
        return None;
    }
    Some(CabecalhoDoIndice { recursos: u32_em(16)?, segmentos: u32_em(20)?, dependencias: u32_em(24)? })
}

/// O `name_hash` de cada recurso de `indice` (o FNV-1a64 do caminho, `crate::resource_path_hash`), na ordem da tabela, sem ler os dados do
/// archive. `None` sem o cabeçalho do índice ou com uma tabela de recursos que não cabe em `indice`.
pub fn hashes_dos_recursos(indice: &[u8]) -> Option<impl Iterator<Item = u64> + '_> {
    let c = cabecalho_do_indice(indice)?;
    let fim = (c.recursos as usize).checked_mul(TAMANHO_DO_RECURSO)?.checked_add(TAMANHO_DO_CABECALHO_DO_INDICE)?;
    let tabela = indice.get(TAMANHO_DO_CABECALHO_DO_INDICE..fim)?;
    Some(tabela.chunks_exact(TAMANHO_DO_RECURSO).filter_map(|r| Some(u64::from_le_bytes(r.get(..8)?.try_into().ok()?))))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cabeçalho de 40 bytes com valores distintos em cada campo, para pegar offset trocado.
    fn bytes_de(magic: &[u8; 4], c: Cabecalho) -> Vec<u8> {
        let mut b = magic.to_vec();
        b.extend_from_slice(&c.versao.to_le_bytes());
        b.extend_from_slice(&c.posicao_do_indice.to_le_bytes());
        b.extend_from_slice(&c.tamanho_do_indice.to_le_bytes());
        b.extend_from_slice(&c.posicao_do_debug.to_le_bytes());
        b.extend_from_slice(&c.tamanho_do_debug.to_le_bytes());
        b.extend_from_slice(&c.tamanho_declarado.to_le_bytes());
        b
    }

    const EXEMPLO: Cabecalho = Cabecalho {
        versao: 12,
        posicao_do_indice: 0x1_0000_2000,
        tamanho_do_indice: 0x3344,
        posicao_do_debug: 0x5_6677_8899,
        tamanho_do_debug: 0xAABB,
        tamanho_declarado: 0x7_0000_4000,
    };

    #[test]
    fn le_cada_campo_no_offset_dele() {
        let b = bytes_de(b"RDAR", EXEMPLO);
        assert_eq!(b.len(), TAMANHO_DO_CABECALHO);
        assert_eq!(u32::from_le_bytes(b[..4].try_into().unwrap()), RDAR_MAGIC);
        assert_eq!(cabecalho(&b), Some(EXEMPLO));
        // O que vem depois do cabeçalho fixo (custom_data_length, payload) não muda a leitura.
        let mut mais = b.clone();
        mais.extend_from_slice(&[0xFF; 8]);
        assert_eq!(cabecalho(&mais), Some(EXEMPLO));
    }

    #[test]
    fn so_com_magic_e_os_40_bytes() {
        let b = bytes_de(b"RDAR", EXEMPLO);
        assert_eq!(cabecalho(&b[..TAMANHO_DO_CABECALHO - 1]), None);
        assert_eq!(cabecalho(&[]), None);
        assert_eq!(cabecalho(&bytes_de(b"RADR", EXEMPLO)), None);
        assert_eq!(cabecalho(&bytes_de(b"KARK", EXEMPLO)), None);
    }

    #[test]
    fn indice_cabe_no_arquivo() {
        let c = |posicao_do_indice: u64, tamanho_do_indice: u32| Cabecalho { posicao_do_indice, tamanho_do_indice, ..EXEMPLO };
        assert!(c(4096, 28).indice_cabe(4096 + 28));
        assert!(c(4096, 28).indice_cabe(8192));
        assert!(!c(4096, 28).indice_cabe(4096 + 27));
        assert!(!c(4096, 27).indice_cabe(8192));
        assert!(!c(4096, 0).indice_cabe(8192));
        assert!(!c(u64::MAX - 27, 28).indice_cabe(u64::MAX));
        assert!(c(u64::MAX - 28, 28).indice_cabe(u64::MAX));
    }

    /// Índice com as contagens dadas e um recurso por hash, cada um com bytes de enchimento depois do `name_hash` para pegar passo errado.
    fn indice_de(hashes: &[u64], segmentos: u32, dependencias: u32) -> Vec<u8> {
        let mut b = vec![0xEEu8; 16];
        b.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
        b.extend_from_slice(&segmentos.to_le_bytes());
        b.extend_from_slice(&dependencias.to_le_bytes());
        for h in hashes {
            b.extend_from_slice(&h.to_le_bytes());
            b.extend_from_slice(&[0xAB; TAMANHO_DO_RECURSO - 8]);
        }
        b
    }

    #[test]
    fn contagens_do_indice_nos_offsets_dele() {
        let b = indice_de(&[7, 8], 3, 4);
        assert_eq!(cabecalho_do_indice(&b), Some(CabecalhoDoIndice { recursos: 2, segmentos: 3, dependencias: 4 }));
        assert_eq!(cabecalho_do_indice(&b[..TAMANHO_DO_CABECALHO_DO_INDICE - 1]), None);
    }

    #[test]
    fn hashes_na_ordem_da_tabela_e_so_com_a_tabela_inteira() {
        let hashes = [0x0123_4567_89AB_CDEF, 1, u64::MAX];
        let b = indice_de(&hashes, 0, 0);
        assert_eq!(hashes_dos_recursos(&b).map(|h| h.collect::<Vec<_>>()), Some(hashes.to_vec()));
        // O que vem depois da tabela (segmentos, dependências) não entra.
        let mut com_segmentos = b.clone();
        com_segmentos.extend_from_slice(&[0x11; 16]);
        assert_eq!(hashes_dos_recursos(&com_segmentos).map(|h| h.count()), Some(3));
        assert!(hashes_dos_recursos(&b[..b.len() - 1]).is_none(), "tabela cortada");
        assert_eq!(hashes_dos_recursos(&indice_de(&[], 0, 0)).map(|h| h.count()), Some(0));
        // Contagem que estouraria a conta do fim da tabela: recusada sem somar com estouro.
        let mut gigante = indice_de(&[], 0, 0);
        gigante[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(hashes_dos_recursos(&gigante).is_none());
    }
}
