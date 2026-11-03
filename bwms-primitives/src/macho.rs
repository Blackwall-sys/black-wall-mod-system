//! Os comandos de carga de um Mach-O de 64 bits (`<mach-o/loader.h>`), num andador só para os três leitores do projeto: o runtime acha
//! seções, segmentos e tabelas de símbolos do jogo por ele (`enderecos`, no arquivo e na imagem da memória), o instalador lê e grava o
//! `LC_LOAD_DYLIB` do executável do jogo (`bwms-helper`, em `bwms/src/bin/`) e o build alinha o string pool do dylib
//! (`fix-symtab-align`, em `cp77-console/src/bin/`). Sem `panic` e sem alocação: o andador roda no dylib público.

/// O primeiro u32 (little-endian) de um Mach-O de 64 bits thin.
pub const MH_MAGIC_64: u32 = 0xFEED_FACF;
/// `sizeof(struct mach_header_64)`: os comandos de carga começam logo depois do cabeçalho.
pub const TAMANHO_DO_CABECALHO: usize = 0x20;
pub const LC_SYMTAB: u32 = 0x2;
pub const LC_DYSYMTAB: u32 = 0xB;
pub const LC_LOAD_DYLIB: u32 = 0xC;
pub const LC_SEGMENT_64: u32 = 0x19;
/// `cmd` + `cmdsize`: o menor comando de carga que o formato admite.
const MINIMO_DE_COMANDO: usize = 8;

/// u32 little-endian em `o`; `None` fora de `b`.
pub(crate) fn u32le(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

/// Um comando de carga: onde ele começa no Mach-O, o `cmd` e o `cmdsize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComandoDeCarga {
    pub offset: usize,
    pub cmd: u32,
    pub tamanho: usize,
}

/// Os comandos de carga na ordem do cabeçalho (ver [`comandos_de_carga`]).
#[derive(Clone, Debug)]
pub struct ComandosDeCarga<'a> {
    macho: &'a [u8],
    restantes: u32,
    offset: usize,
}

/// Os comandos de carga de um Mach-O de 64 bits thin: o arquivo, ou a imagem na memória a partir do cabeçalho. `None` sem os 32 bytes do
/// cabeçalho ou sem `MH_MAGIC_64`. O andador para no `ncmds`, no comando cujo `cmd`/`cmdsize` não cabe em `macho` e no `cmdsize` menor
/// que 8: o formato não tem comando assim (o dyld recusa o binário), e com 0 o andador não sairia do lugar. Depois do primeiro `None`,
/// continua `None`.
pub fn comandos_de_carga(macho: &[u8]) -> Option<ComandosDeCarga<'_>> {
    if macho.len() < TAMANHO_DO_CABECALHO || u32le(macho, 0)? != MH_MAGIC_64 {
        return None;
    }
    Some(ComandosDeCarga { macho, restantes: u32le(macho, 0x10)?, offset: TAMANHO_DO_CABECALHO })
}

impl Iterator for ComandosDeCarga<'_> {
    type Item = ComandoDeCarga;

    fn next(&mut self) -> Option<ComandoDeCarga> {
        if self.restantes == 0 {
            return None;
        }
        let cmd = u32le(self.macho, self.offset)?;
        let tamanho = u32le(self.macho, self.offset.checked_add(4)?)? as usize;
        if tamanho < MINIMO_DE_COMANDO {
            return None;
        }
        let comando = ComandoDeCarga { offset: self.offset, cmd, tamanho };
        self.restantes -= 1;
        self.offset = self.offset.saturating_add(tamanho);
        Some(comando)
    }
}

/// Mach-O de teste: cabeçalho de 64 bits (`ncmds` = quantos comandos foram passados) e os comandos `(cmd, corpo)` em sequência, cada um com
/// `cmdsize` = 8 + o corpo. O `enderecos` usa o mesmo nos testes dele.
#[cfg(test)]
pub(crate) fn macho_de_teste(comandos: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let tamanhos: usize = comandos.iter().map(|(_, corpo)| 8 + corpo.len()).sum();
    let mut m = Vec::new();
    for v in [MH_MAGIC_64, 0x0100_000C, 0, 6, comandos.len() as u32, tamanhos as u32, 0, 0] {
        m.extend_from_slice(&v.to_le_bytes());
    }
    for (cmd, corpo) in comandos {
        m.extend_from_slice(&cmd.to_le_bytes());
        m.extend_from_slice(&(8 + corpo.len() as u32).to_le_bytes());
        m.extend_from_slice(corpo);
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lista(m: &[u8]) -> Vec<(usize, u32, usize)> {
        comandos_de_carga(m).expect("Mach-O").map(|c| (c.offset, c.cmd, c.tamanho)).collect()
    }

    fn com_cmdsize(mut m: Vec<u8>, offset: usize, cmdsize: u32) -> Vec<u8> {
        m[offset + 4..offset + 8].copy_from_slice(&cmdsize.to_le_bytes());
        m
    }

    #[test]
    fn anda_pelos_comandos_na_ordem() {
        let m = macho_de_teste(&[(LC_SEGMENT_64, vec![0; 64]), (LC_SYMTAB, vec![0; 16]), (LC_LOAD_DYLIB, vec![0; 24])]);
        assert_eq!(lista(&m), vec![(0x20, LC_SEGMENT_64, 72), (0x68, LC_SYMTAB, 24), (0x80, LC_LOAD_DYLIB, 32)]);
    }

    #[test]
    fn para_no_ncmds() {
        let mut m = macho_de_teste(&[(LC_SYMTAB, vec![0; 16]), (LC_DYSYMTAB, vec![0; 72]), (LC_LOAD_DYLIB, vec![0; 24])]);
        m[0x10..0x14].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(lista(&m), vec![(0x20, LC_SYMTAB, 24), (0x38, LC_DYSYMTAB, 80)]);
        m[0x10..0x14].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(lista(&m), vec![]);
    }

    #[test]
    fn para_no_comando_que_nao_cabe() {
        let m = macho_de_teste(&[(LC_SYMTAB, vec![0; 16]), (LC_LOAD_DYLIB, vec![0; 24])]);
        // O 2º comando começa em 0x38: com 7 bytes dele o cmdsize não cabe; com 8 cabe, mesmo sem o corpo.
        assert_eq!(lista(&m[..0x38 + 7]), vec![(0x20, LC_SYMTAB, 24)]);
        assert_eq!(lista(&m[..0x38 + 8]), vec![(0x20, LC_SYMTAB, 24), (0x38, LC_LOAD_DYLIB, 32)]);
        assert_eq!(lista(&m[..0x20]), vec![]);
    }

    #[test]
    fn para_no_cmdsize_menor_que_8_e_continua_parado() {
        let m = macho_de_teste(&[(LC_SYMTAB, vec![0; 16]), (LC_LOAD_DYLIB, vec![0; 24]), (LC_DYSYMTAB, vec![0; 72])]);
        for cmdsize in [0, 4, 7] {
            let torto = com_cmdsize(m.clone(), 0x38, cmdsize);
            let mut andador = comandos_de_carga(&torto).expect("Mach-O");
            assert_eq!(andador.next().map(|c| c.offset), Some(0x20), "cmdsize {cmdsize}");
            assert_eq!(andador.next(), None, "cmdsize {cmdsize}");
            assert_eq!(andador.next(), None, "cmdsize {cmdsize}");
        }
        // 8 é o mínimo e passa; o próximo cai em 0x40, no corpo zerado do 2º (cmdsize 0), e o andador para ali.
        assert_eq!(lista(&com_cmdsize(m, 0x38, 8)), vec![(0x20, LC_SYMTAB, 24), (0x38, LC_LOAD_DYLIB, 8)]);
    }

    #[test]
    fn so_mach_o_de_64_bits_thin() {
        let m = macho_de_teste(&[]);
        assert_eq!(lista(&m), vec![]);
        assert!(comandos_de_carga(&m[..0x1F]).is_none());
        assert!(comandos_de_carga(&[]).is_none());
        // 32 bits, big-endian e universal
        for magic in [0xFEED_FACEu32, 0xCFFA_EDFE, 0xBEBA_FECA] {
            let mut outro = m.clone();
            outro[..4].copy_from_slice(&magic.to_le_bytes());
            assert!(comandos_de_carga(&outro).is_none(), "{magic:#x}");
        }
    }

    /// Um binário que o linker gerou: o executável deste teste. O andador passa por todos os `ncmds`, eles somam o `sizeofcmds` e o
    /// `__TEXT`, o `__LINKEDIT` e o `LC_SYMTAB` estão entre eles.
    #[cfg(target_os = "macos")]
    #[test]
    fn anda_pelo_executavel_do_proprio_teste() {
        let exe = std::fs::read(std::env::current_exe().expect("caminho do teste")).expect("leitura do teste");
        let (ncmds, sizeofcmds) = (u32le(&exe, 0x10).expect("ncmds") as usize, u32le(&exe, 0x14).expect("sizeofcmds") as usize);
        let comandos: Vec<ComandoDeCarga> = comandos_de_carga(&exe).expect("Mach-O de 64 bits").collect();
        assert_eq!(comandos.len(), ncmds);
        assert_eq!(comandos.iter().map(|c| c.tamanho).sum::<usize>(), sizeofcmds);
        let segmento = |nome: &[u8]| comandos.iter().any(|c| c.cmd == LC_SEGMENT_64 && exe[c.offset + 8..c.offset + 24].starts_with(nome));
        assert!(segmento(b"__TEXT\0") && segmento(b"__LINKEDIT\0"));
        assert_eq!(comandos.iter().filter(|c| c.cmd == LC_SYMTAB).count(), 1);
    }
}
