//! Writer RDAR (`.archive`) — porte fiel do `ArchiveWriter` do WolvenKit p/ empacotar recursos CR2W
//! **SEM COMPRESSÃO** (zsize==size), sem precisar de Kraken *encode*. Alvo #1: mod de TRADUÇÃO —
//! dropar o `onscreens`/`subtitles` editado (por `cr2w::repack_localization_edit`) num `.archive`
//! que o jogo carrega pelo Path A (glob nativo, já provado). Fecha o pipeline ler→editar→empacotar.
//!
//! Layout (idêntico ao ArchiveWriter): header 40B + 132B pad (=0xAC) + payload dos recursos (cada
//! CR2W cru como 1 segmento) + pad-de-página + índice (crc64) + pad-de-página. Header/customDataLength
//! reescritos por último. Recursos COM buffers (`buffersEnd>objectsEnd`) ficam fora do escopo —
//! localização não tem buffer; buffers exigiriam split em segmentos + Kraken encode.

use crate::archive::{crc64, sha1, RDAR_MAGIC};

const HEADER_REGION: usize = 0xAC; // 40 (header) + 132 (pad) — payload começa aqui (WolvenKit)
const PAGE: usize = 4096;
const RDAR_VERSION: u32 = 12; // v12 = CP2077 2.x (confirmado nos archives reais)

fn pad_to_page(v: &mut Vec<u8>) {
    let rem = v.len() % PAGE;
    if rem != 0 {
        v.resize(v.len() + (PAGE - rem), 0);
    }
}

/// Empacota recursos (`name_hash` = FNV-1a64 do path REDengine, bytes CR2W já finais) num `.archive`
/// RDAR v12, cada um como UM segmento não-comprimido. Recusa CR2W com buffers. Ordena por hash (como
/// o WolvenKit). Devolve os bytes do `.archive` completo, prontos p/ gravar em disco.
pub fn pack_uncompressed(resources: &[(u64, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if resources.is_empty() {
        return Err("nada a empacotar".into());
    }
    // WolvenKit ordena o fileDict por Key (hash) ascendente.
    let mut res: Vec<&(u64, Vec<u8>)> = resources.iter().collect();
    res.sort_by_key(|(h, _)| *h);
    // hash duplicado = erro (o WolvenKit aborta).
    for w in res.windows(2) {
        if w[0].0 == w[1].0 {
            return Err(format!("hash duplicado: {:#018x}", w[0].0));
        }
    }

    let mut out = vec![0u8; HEADER_REGION]; // header + pad (preenchido no fim)
    let mut entries: Vec<u8> = Vec::with_capacity(res.len() * 56);
    let mut segments: Vec<u8> = Vec::with_capacity(res.len() * 16);

    for (i, (hash, cr2w)) in res.iter().enumerate() {
        // valida: recurso sem buffers (objectsEnd == buffersEnd == tamanho). Só assim 1 segmento basta.
        let idx = crate::cr2w::parse_cr2w_index(cr2w)?;
        if idx.header.objects_end as usize != cr2w.len()
            || idx.header.buffers_end != idx.header.objects_end
        {
            return Err(format!(
                "recurso {i} ({hash:#018x}): tem buffers (objectsEnd={} buffersEnd={} len={}) — fora do escopo do writer não-comprimido",
                idx.header.objects_end, idx.header.buffers_end, cr2w.len()
            ));
        }
        let offset = out.len() as u64;
        out.extend_from_slice(cr2w); // segmento cru (não-comprimido)
        let size = cr2w.len() as u32;

        // FileSegment (16B): offset(u64) zsize(u32) size(u32) — zsize==size (sem compressão)
        segments.extend_from_slice(&offset.to_le_bytes());
        segments.extend_from_slice(&size.to_le_bytes());
        segments.extend_from_slice(&size.to_le_bytes());

        // FileEntry (56B): nameHash(u64) timestamp(i64) numInlineBuf(u32) segStart(u32) segEnd(u32)
        //                  depStart(u32) depEnd(u32) sha1(20)
        entries.extend_from_slice(&hash.to_le_bytes());
        entries.extend_from_slice(&0i64.to_le_bytes()); // timestamp 0 = determinístico (o jogo não valida)
        entries.extend_from_slice(&0u32.to_le_bytes()); // numInlineBufferSegments = 0 (sem buffer)
        entries.extend_from_slice(&(i as u32).to_le_bytes()); // segments_start
        entries.extend_from_slice(&(i as u32 + 1).to_le_bytes()); // segments_end
        entries.extend_from_slice(&0u32.to_le_bytes()); // deps_start
        entries.extend_from_slice(&0u32.to_le_bytes()); // deps_end
        entries.extend_from_slice(&sha1(cr2w)); // sha1 do recurso inteiro
    }

    // pad até a página, depois o índice.
    pad_to_page(&mut out);
    let table_offset = out.len();

    // corpo do índice (o "ms" do WolvenKit): counts + entries + segments (+ deps, aqui 0).
    let mut ms = Vec::with_capacity(12 + entries.len() + segments.len());
    ms.extend_from_slice(&(res.len() as u32).to_le_bytes()); // FileEntryCount
    ms.extend_from_slice(&(res.len() as u32).to_le_bytes()); // FileSegmentCount
    ms.extend_from_slice(&0u32.to_le_bytes()); // ResourceDependencyCount
    ms.extend_from_slice(&entries);
    ms.extend_from_slice(&segments);

    // índice em disco: file_table_offset(=8) + file_table_size(=ms+8) + crc64(ms) + ms
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&(ms.len() as u32 + 8).to_le_bytes());
    out.extend_from_slice(&crc64(&ms).to_le_bytes());
    out.extend_from_slice(&ms);
    let index_size = (out.len() - table_offset) as u32;

    // pad até a página → filesize.
    pad_to_page(&mut out);
    let filesize = out.len() as u64;

    // header (40B) em [0..40): magic version indexPos indexSize debugPos debugSize filesize.
    let mut h = Vec::with_capacity(40);
    h.extend_from_slice(&RDAR_MAGIC.to_le_bytes());
    h.extend_from_slice(&RDAR_VERSION.to_le_bytes());
    h.extend_from_slice(&(table_offset as u64).to_le_bytes());
    h.extend_from_slice(&index_size.to_le_bytes());
    h.extend_from_slice(&0u64.to_le_bytes()); // debugPosition
    h.extend_from_slice(&0u32.to_le_bytes()); // debugSize
    h.extend_from_slice(&filesize.to_le_bytes());
    out[..40].copy_from_slice(&h);
    // customDataLength (u32) @ 0x28 = [40..44): 0 (sem LxrsFooter).
    out[40..44].copy_from_slice(&0u32.to_le_bytes());

    Ok(out)
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
