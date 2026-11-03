//! Escritor RDAR (`.archive`) sem compressão: empacota recursos CR2W já prontos, cada um como um segmento cru (zsize == size).
//! Alvo: mod de tradução (`bwms archive pack`/`packdir` e a saída do `locedit`). Recurso com buffers fica fora do escopo.

use crate::archive::archive::{crc64, sha1, EXTENDED_SIZE, RDAR_MAGIC};
use crate::archive::cr2w::parse_cr2w_index;

const VERSAO_RDAR: u32 = 12;
/// Os `.archive` do jogo alinham o índice e o fim do arquivo em 4096.
const ALINHAMENTO: usize = 4096;
/// Deslocamento da tabela de recursos dentro do índice (o que vem antes do `file_table_size`).
const DESLOCAMENTO_DA_TABELA: u32 = 8;

/// Empacota recursos (`name_hash` = FNV-1a64 do path REDengine, bytes CR2W já finais) num `.archive`
/// RDAR v12, cada um como UM segmento não-comprimido. Recusa CR2W com buffers. Ordena por hash.
/// Devolve os bytes do `.archive` completo, prontos p/ gravar em disco.
pub fn pack_uncompressed(resources: &[(u64, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if resources.is_empty() {
        return Err("nada a empacotar".into());
    }
    let mut ordem: Vec<&(u64, Vec<u8>)> = resources.iter().collect();
    ordem.sort_by_key(|(hash, _)| *hash);
    for par in ordem.windows(2) {
        if par[0].0 == par[1].0 {
            return Err(format!("hash duplicado: {:#018x}", par[0].0));
        }
    }
    for (i, (hash, bytes)) in ordem.iter().enumerate() {
        let h = parse_cr2w_index(bytes)?.header;
        // Segmento cru único: o recurso inteiro tem de ser objetos; buffer pediria segmento próprio.
        if h.objects_end != h.buffers_end || h.objects_end as usize != bytes.len() {
            return Err(format!(
                "recurso {i} ({hash:#018x}): tem buffers (objectsEnd={} buffersEnd={} len={}) — o empacotador sem compressão não grava buffer",
                h.objects_end,
                h.buffers_end,
                bytes.len()
            ));
        }
    }

    let mut saida = vec![0u8; EXTENDED_SIZE as usize];
    let mut segmentos: Vec<(u64, u32)> = Vec::new();
    for (_, bytes) in &ordem {
        // O CR2W validado acima tem tamanho = objectsEnd (u32), então cabe no u32 do segmento.
        segmentos.push((saida.len() as u64, bytes.len() as u32));
        saida.extend_from_slice(bytes);
    }
    alinhar(&mut saida);

    let n = ordem.len() as u32;
    let mut corpo = Vec::new();
    corpo.extend_from_slice(&n.to_le_bytes());
    corpo.extend_from_slice(&n.to_le_bytes());
    corpo.extend_from_slice(&0u32.to_le_bytes());
    for (i, (hash, bytes)) in ordem.iter().enumerate() {
        let i = i as u32;
        corpo.extend_from_slice(&hash.to_le_bytes());
        corpo.extend_from_slice(&0i64.to_le_bytes());
        corpo.extend_from_slice(&0u32.to_le_bytes());
        corpo.extend_from_slice(&i.to_le_bytes());
        corpo.extend_from_slice(&(i + 1).to_le_bytes());
        corpo.extend_from_slice(&0u32.to_le_bytes());
        corpo.extend_from_slice(&0u32.to_le_bytes());
        corpo.extend_from_slice(&sha1(bytes));
    }
    for (offset, tamanho) in &segmentos {
        corpo.extend_from_slice(&offset.to_le_bytes());
        corpo.extend_from_slice(&tamanho.to_le_bytes());
        corpo.extend_from_slice(&tamanho.to_le_bytes());
    }

    let posicao_do_indice = saida.len() as u64;
    // Índice = deslocamento (u32) + tamanho da tabela (u32) + crc (u64) + corpo.
    let tamanho_do_indice = u32::try_from(corpo.len() + 16).map_err(|_| "índice maior que 4 GiB".to_string())?;
    let tamanho_da_tabela = tamanho_do_indice - DESLOCAMENTO_DA_TABELA;
    saida.extend_from_slice(&DESLOCAMENTO_DA_TABELA.to_le_bytes());
    saida.extend_from_slice(&tamanho_da_tabela.to_le_bytes());
    saida.extend_from_slice(&crc64(&corpo).to_le_bytes());
    saida.extend_from_slice(&corpo);
    alinhar(&mut saida);

    let tamanho_total = saida.len() as u64;
    let mut cabecalho = Vec::with_capacity(44);
    cabecalho.extend_from_slice(&RDAR_MAGIC.to_le_bytes());
    cabecalho.extend_from_slice(&VERSAO_RDAR.to_le_bytes());
    cabecalho.extend_from_slice(&posicao_do_indice.to_le_bytes());
    cabecalho.extend_from_slice(&tamanho_do_indice.to_le_bytes());
    cabecalho.extend_from_slice(&0u64.to_le_bytes());
    cabecalho.extend_from_slice(&0u32.to_le_bytes());
    cabecalho.extend_from_slice(&tamanho_total.to_le_bytes());
    cabecalho.extend_from_slice(&0u32.to_le_bytes());
    saida[..cabecalho.len()].copy_from_slice(&cabecalho);
    Ok(saida)
}

/// Zeros até o próximo múltiplo de [`ALINHAMENTO`] (nada se já estiver alinhado).
fn alinhar(buf: &mut Vec<u8>) {
    let resto = buf.len() % ALINHAMENTO;
    if resto != 0 {
        buf.resize(buf.len() + ALINHAMENTO - resto, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_kat() {
        // FIPS 180-1: sha1("abc") = a9993e364706816aba3e25717850c26c9cd0d89d
        let d = sha1(b"abc");
        let hex: String = d.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        // sha1("") = da39a3ee5e6b4b0d3255bfef95601890afd80709
        let e: String = sha1(b"").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(e, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn crc64_xz_kat() {
        // CRC-64/XZ check: crc64("123456789") = 0x995dc9bbdf1939fa
        assert_eq!(crc64(b"123456789"), 0x995d_c9bb_df19_39fa);
    }
}
