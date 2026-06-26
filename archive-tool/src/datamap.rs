//! Geração do `datamap.md` — o retrato textual do índice de um `.archive`.
//!
//! Tudo aqui sai **só do índice**, que é não-comprimido: não abre o payload,
//! não precisa de Kraken. É a primeira metade da tarefa (a que "já dá pra
//! fazer"). Escreve em streaming num `Write` para aguentar archives enormes
//! sem montar uma `String` gigante na memória.

use std::io::{self, Write};

use crate::archive::Archive;
use crate::hashes::PathDictionary;
use crate::time::filetime_to_iso;

/// Números agregados, devolvidos para o CLI ecoar um resumo.
pub struct Stats {
    pub entries: usize,
    pub segments: usize,
    pub dependencies: usize,
    pub resolved: usize,
    pub compressed_segments: usize,
    pub total_disk: u64,
    pub total_uncompressed: u64,
}

/// Deriva a extensão a partir do path resolvido (sem path → `?`). O WolvenKit
/// também adivinha pelo magic do payload, mas isso exigiria descomprimir o
/// segmento principal (Kraken) — fora do escopo do datamap.
fn ext_of(name: Option<&str>) -> &str {
    match name {
        Some(p) => p.rsplit(['.', '\\', '/']).next().filter(|_| p.contains('.')).unwrap_or("?"),
        None => "?",
    }
}

pub fn write_datamap<W: Write>(
    ar: &Archive,
    dict: &PathDictionary,
    w: &mut W,
) -> io::Result<Stats> {
    let total_disk: u64 = ar.segments.iter().map(|s| u64::from(s.zsize)).sum();
    let total_uncompressed: u64 = ar.segments.iter().map(|s| u64::from(s.size)).sum();
    let compressed_segments = ar.segments.iter().filter(|s| s.size_differs()).count();

    let archive_name = ar
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| ar.path.display().to_string());

    // --- Cabeçalho / resumo ---
    writeln!(w, "# Datamap — `{archive_name}`")?;
    writeln!(w)?;
    writeln!(
        w,
        "Índice RDAR (REDengine archive) do Cyberpunk 2077. Gerado a partir do \
         índice não-comprimido; o payload (Kraken/Oodle) **não** foi descomprimido."
    )?;
    writeln!(w)?;
    writeln!(w, "| Campo | Valor |")?;
    writeln!(w, "|---|---|")?;
    writeln!(w, "| Versão RDAR | {} |", ar.header.version)?;
    writeln!(w, "| Recursos | {} |", ar.entries.len())?;
    writeln!(w, "| Segmentos | {} |", ar.segments.len())?;
    writeln!(w, "| Dependências | {} |", ar.dependencies.len())?;
    writeln!(
        w,
        "| Segmentos comprimidos | {compressed_segments} / {} |",
        ar.segments.len()
    )?;
    writeln!(w, "| Bytes em disco (Σ zsize) | {} |", fmt_bytes(total_disk))?;
    writeln!(
        w,
        "| Bytes descomprimidos (Σ size) | {} |",
        fmt_bytes(total_uncompressed)
    )?;
    writeln!(
        w,
        "| Índice | {} bytes @ offset {} |",
        ar.header.index_size, ar.header.index_position
    )?;
    writeln!(
        w,
        "| Tabela de arquivos | offset {} · {} bytes |",
        ar.file_table_offset, ar.file_table_size
    )?;
    writeln!(w, "| CRC do índice | {:#018x} |", ar.crc)?;
    if ar.header.debug_size != 0 || ar.header.debug_position != 0 {
        writeln!(
            w,
            "| Bloco de debug | {} bytes @ offset {} |",
            ar.header.debug_size, ar.header.debug_position
        )?;
    }

    // Resolução de nomes
    let mut resolved = 0usize;
    for e in &ar.entries {
        if dict.resolve(e.name_hash).is_some() {
            resolved += 1;
        }
    }
    writeln!(
        w,
        "| Nomes resolvidos | {resolved} / {} |",
        ar.entries.len()
    )?;
    if ar.custom_paths_need_kraken {
        writeln!(
            w,
            "| Paths embutidos (LxrsFooter) | comprimido, requer Kraken para ler |"
        )?;
    } else if !ar.custom_paths.is_empty() {
        writeln!(
            w,
            "| Paths embutidos (LxrsFooter) | {} |",
            ar.custom_paths.len()
        )?;
    }
    writeln!(w)?;

    // --- Recursos ---
    writeln!(
        w,
        "## Recursos\n\n\
         Uma linha por recurso. `comp` = o segmento principal tem zsize≠size \
         (precisa de Kraken para descomprimir). `segs`/`deps` = quantidade de \
         segmentos e de dependências. `inl` = buffers inline (campo \
         NumInlineBufferSegments). `disco` = Σ zsize; `descomp` = size do \
         segmento principal. `sha1` = hash de conteúdo gravado no índice. O nome \
         só aparece quando resolvido (via --hashes ou LxrsFooter).\n"
    )?;
    writeln!(
        w,
        "```\n# idx hash             ext     segs inl   disco    descomp comp deps sha1                                     data                  nome"
    )?;
    for (i, e) in ar.entries.iter().enumerate() {
        let name = dict.resolve(e.name_hash);
        let main_seg_comp = ar
            .segments_of(e)
            .first()
            .map(|s| s.size_differs())
            .unwrap_or(false);
        let date = filetime_to_iso(e.timestamp).unwrap_or_else(|| "-".into());
        writeln!(
            w,
            "[{i}] {hash:016x} {ext:<7} {segs:>4} {inl:>3} {disk:>9} {main:>9} {comp:>4} {deps:>4} {sha1} {date:<21} {name}",
            hash = e.name_hash,
            ext = ext_of(name),
            segs = e.segment_count(),
            inl = e.num_inline_buffer_segments,
            disk = ar.disk_size_of(e),
            main = ar.main_size_of(e),
            comp = if main_seg_comp { "sim" } else { "não" },
            deps = e.deps_end.saturating_sub(e.deps_start),
            sha1 = e.sha1_hex(),
            name = name.unwrap_or("<sem nome>"),
        )?;
    }
    writeln!(w, "```")?;
    writeln!(w)?;

    // --- Segmentos (resumo + tabela completa se couber) ---
    writeln!(
        w,
        "## Segmentos\n\n\
         `offset` = posição no arquivo; `zsize` = bytes em disco; `size` = bytes \
         descomprimidos. zsize≠size ⇒ comprimido (Kraken, se começar com `KARK`).\n"
    )?;
    writeln!(w, "```\n# idx  offset            zsize        size         comp")?;
    for (i, s) in ar.segments.iter().enumerate() {
        writeln!(
            w,
            "[{i}] {off:>14}  {zsize:>10}  {size:>10}  {comp}",
            off = s.offset,
            zsize = s.zsize,
            size = s.size,
            comp = if s.size_differs() { "sim" } else { "não" },
        )?;
    }
    writeln!(w, "```")?;
    writeln!(w)?;

    // --- Dependências (tabela global de recursos referenciados) ---
    if !ar.dependencies.is_empty() {
        writeln!(
            w,
            "## Dependências\n\n\
             Tabela global de hashes de recursos que este archive referencia \
             (cada recurso aponta uma faixa nela). {} entradas.\n",
            ar.dependencies.len()
        )?;
        writeln!(w, "```")?;
        for (i, hash) in ar.dependencies.iter().enumerate() {
            match dict.resolve(*hash) {
                Some(name) => writeln!(w, "[{i}] {hash:016x} {name}")?,
                None => writeln!(w, "[{i}] {hash:016x}")?,
            }
        }
        writeln!(w, "```")?;
        writeln!(w)?;
    }

    // --- Paths embutidos do LxrsFooter (mods/ArchiveXL) ---
    if !ar.custom_paths.is_empty() {
        writeln!(
            w,
            "## Paths embutidos (LxrsFooter)\n\n\
             Lista de paths que o próprio archive carrega (mecanismo de registro \
             do ArchiveXL). {} entradas.\n",
            ar.custom_paths.len()
        )?;
        writeln!(w, "```")?;
        for p in &ar.custom_paths {
            writeln!(w, "{p}")?;
        }
        writeln!(w, "```")?;
        writeln!(w)?;
    }

    Ok(Stats {
        entries: ar.entries.len(),
        segments: ar.segments.len(),
        dependencies: ar.dependencies.len(),
        resolved,
        compressed_segments,
        total_disk,
        total_uncompressed,
    })
}

/// Formata bytes em algo legível (KiB/MiB/GiB), mantendo o número cru entre ().
fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.2} {} ({n} B)", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensao_a_partir_do_path() {
        assert_eq!(ext_of(Some("base\\char\\v.mesh")), "mesh");
        assert_eq!(ext_of(Some("sem_extensao")), "?");
        assert_eq!(ext_of(None), "?");
    }

    #[test]
    fn formata_bytes() {
        assert_eq!(fmt_bytes(512), "512 B");
        assert!(fmt_bytes(2048).starts_with("2.00 KiB"));
        assert!(fmt_bytes(5 * 1024 * 1024).starts_with("5.00 MiB"));
    }
}
