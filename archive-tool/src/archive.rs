//! Leitor do formato RDAR (`.archive`) do Cyberpunk 2077.
//!
//! Port fiel do codec de leitura do WolvenKit (`WolvenKit.RED4/Archive`):
//! `Header`, `Index`, `FileEntry`, `Index.FileSegment`/`Dependency`, `LxrsFooter`.
//! Só lê o **índice**, que é sempre **não-comprimido** — logo não precisa de
//! Kraken/Oodle. A descompressão do payload mora em [`crate::kraken`].
//!
//! Layout (little-endian em todo o formato):
//!
//! ```text
//! 0x00  Header (40 bytes)
//!         magic   u32   = 0x52414452 "RDAR" (1380009042)
//!         version u32
//!         index_position u64
//!         index_size     u32
//!         debug_position u64
//!         debug_size     u32
//!         filesize       u64
//! 0x28  custom_data_length u32        (logo após o header)
//! 0xAC  LxrsFooter (se custom_data_length != 0)   — paths embutidos (ArchiveXL/mods)
//! ...   payload dos segmentos (em offsets arbitrários)
//! @index_position  Index (index_size bytes):
//!         file_table_offset u32
//!         file_table_size   u32
//!         crc               u64
//!         file_entry_count        u32
//!         file_segment_count      u32
//!         resource_dependency_count u32
//!         FileEntry  × file_entry_count   (56 bytes cada)
//!         FileSegment× file_segment_count (16 bytes cada)
//!         Dependency × dependency_count   (8 bytes cada)
//! ```

use std::fmt;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const RDAR_MAGIC: u32 = 1_380_009_042; // 0x52414452, bytes "RDAR"
pub const HEADER_SIZE: u64 = 40;
pub const EXTENDED_SIZE: u64 = 0xAC; // 172 — onde começa o LxrsFooter
pub const INDEX_HEADER_SIZE: usize = 28;
pub const FILE_ENTRY_SIZE: usize = 56;
pub const FILE_SEGMENT_SIZE: usize = 16;
pub const DEPENDENCY_SIZE: usize = 8;
pub const LXRS_MAGIC: u32 = 0x4C58_5253; // bytes "SRXL" em disco (LxrsFooter)
pub const KARK_MAGIC: u32 = 0x4B52_414B; // bytes "KARK" — header de segmento Oodle/Kraken

/// Limite defensivo para o índice (256 MiB). Um basegame.archive real tem
/// índice de poucos MB; isto só barra arquivos corrompidos/maliciosos.
const MAX_INDEX_SIZE: u64 = 256 * 1024 * 1024;

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    /// Não é um RDAR ou as tabelas não fecham com os tamanhos declarados.
    Format(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "erro de E/S: {e}"),
            Error::Format(m) => write!(f, "formato RDAR inválido: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

fn format_err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Format(msg.into()))
}

#[derive(Debug, Clone)]
pub struct Header {
    pub version: u32,
    pub index_position: u64,
    pub index_size: u32,
    pub debug_position: u64,
    pub debug_size: u32,
    pub filesize: u64,
    pub custom_data_length: u32,
}

/// Um segmento (Index 2 / OffsetTable no WolvenKit): uma fatia contígua do
/// arquivo em `offset`, ocupando `zsize` bytes em disco e expandindo para
/// `size` bytes ao descomprimir.
#[derive(Debug, Clone, Copy)]
pub struct FileSegment {
    pub offset: u64,
    pub zsize: u32,
    pub size: u32,
}

impl FileSegment {
    /// `true` quando o segmento ocupa em disco um tamanho diferente do
    /// descomprimido — sinal (necessário, não suficiente) de que há Kraken.
    /// A confirmação definitiva é o magic `KARK` nos primeiros 4 bytes; ver
    /// [`crate::kraken::segment_needs_kraken`].
    pub fn size_differs(&self) -> bool {
        self.zsize != self.size
    }
}

/// Uma entrada de recurso (Index 1 / FileTable). 56 bytes em disco.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name_hash: u64,
    /// Windows FILETIME (100ns desde 1601). Mantido cru; formatação é cosmética.
    pub timestamp: i64,
    pub num_inline_buffer_segments: u32,
    pub segments_start: u32,
    pub segments_end: u32,
    pub deps_start: u32,
    pub deps_end: u32,
    pub sha1: [u8; 20],
}

impl FileEntry {
    /// Quantidade de segmentos do recurso (principal + buffers).
    pub fn segment_count(&self) -> u32 {
        self.segments_end.saturating_sub(self.segments_start)
    }

    pub fn sha1_hex(&self) -> String {
        let mut s = String::with_capacity(40);
        for byte in self.sha1 {
            s.push_str(&format!("{byte:02x}"));
        }
        s
    }
}

pub struct Archive {
    pub path: PathBuf,
    pub header: Header,
    pub file_table_offset: u32,
    pub file_table_size: u32,
    pub crc: u64,
    pub entries: Vec<FileEntry>,
    pub segments: Vec<FileSegment>,
    pub dependencies: Vec<u64>,
    /// Paths embutidos no LxrsFooter (ArchiveXL/mods). Vazio se ausente ou se
    /// estava comprimido e o Kraken não estava disponível.
    pub custom_paths: Vec<String>,
    /// `true` se havia um LxrsFooter comprimido que não pôde ser lido sem Kraken.
    pub custom_paths_need_kraken: bool,
}

impl Archive {
    /// Abre o `.archive` e lê só o necessário: header, footer (best-effort) e o
    /// índice inteiro. Não carrega o payload (arquivos têm GBs).
    pub fn open(path: &Path) -> Result<Archive> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();

        if file_len < HEADER_SIZE + 4 {
            return format_err(format!(
                "arquivo menor que o cabeçalho ({file_len} bytes)"
            ));
        }

        // --- Header + custom_data_length (44 bytes) ---
        let mut head = [0u8; 44];
        file.read_exact(&mut head)?;
        if read_u32(&head, 0)? != RDAR_MAGIC {
            return format_err("magic não é RDAR (0x52414452)");
        }
        let header = Header {
            version: read_u32(&head, 4)?,
            index_position: read_u64(&head, 8)?,
            index_size: read_u32(&head, 16)?,
            debug_position: read_u64(&head, 20)?,
            debug_size: read_u32(&head, 28)?,
            filesize: read_u64(&head, 32)?,
            custom_data_length: read_u32(&head, 40)?,
        };

        // O tamanho declarado, quando presente, deve bater com o arquivo real.
        if header.filesize != 0 && header.filesize != file_len {
            return format_err(format!(
                "filesize declarado ({}) difere do real ({file_len})",
                header.filesize
            ));
        }

        // --- LxrsFooter (paths embutidos), best-effort ---
        let mut custom_paths = Vec::new();
        let mut custom_paths_need_kraken = false;
        if header.custom_data_length != 0 {
            match read_lxrs_footer(&mut file, header.custom_data_length, file_len) {
                Ok(LxrsResult::Paths(paths)) => custom_paths = paths,
                Ok(LxrsResult::NeedsKraken) => custom_paths_need_kraken = true,
                // Footer corrompido não é fatal: seguimos só com o índice.
                Err(_) => {}
            }
        }

        // --- Índice ---
        let index_size = header.index_size as u64;
        let index_end = header
            .index_position
            .checked_add(index_size)
            .ok_or_else(|| Error::Format("índice estoura u64".into()))?;
        if (index_size as usize) < INDEX_HEADER_SIZE
            || index_size > MAX_INDEX_SIZE
            || index_end > file_len
        {
            return format_err(format!(
                "índice ({} bytes @ {}) aponta para fora do arquivo",
                header.index_size, header.index_position
            ));
        }

        file.seek(SeekFrom::Start(header.index_position))?;
        let mut index = vec![0u8; index_size as usize];
        file.read_exact(&mut index)?;

        let file_table_offset = read_u32(&index, 0)?;
        let file_table_size = read_u32(&index, 4)?;
        let crc = read_u64(&index, 8)?;
        let entry_count = read_u32(&index, 16)? as usize;
        let segment_count = read_u32(&index, 20)? as usize;
        let dependency_count = read_u32(&index, 24)? as usize;

        // As três tabelas têm de caber exatamente no índice lido.
        let entries_at = INDEX_HEADER_SIZE;
        let segments_at = entries_at
            .checked_add(entry_count.saturating_mul(FILE_ENTRY_SIZE))
            .ok_or_else(|| Error::Format("tabela de recursos estoura usize".into()))?;
        let deps_at = segments_at
            .checked_add(segment_count.saturating_mul(FILE_SEGMENT_SIZE))
            .ok_or_else(|| Error::Format("tabela de segmentos estoura usize".into()))?;
        let deps_end = deps_at
            .checked_add(dependency_count.saturating_mul(DEPENDENCY_SIZE))
            .ok_or_else(|| Error::Format("tabela de dependências estoura usize".into()))?;
        if deps_end > index.len() {
            return format_err(format!(
                "tabelas ({entry_count} recursos, {segment_count} segmentos, \
                 {dependency_count} deps) ultrapassam o índice de {} bytes",
                index.len()
            ));
        }

        let mut entries = Vec::with_capacity(entry_count);
        for i in 0..entry_count {
            let base = entries_at + i * FILE_ENTRY_SIZE;
            let mut sha1 = [0u8; 20];
            sha1.copy_from_slice(&index[base + 36..base + 56]);
            entries.push(FileEntry {
                name_hash: read_u64(&index, base)?,
                timestamp: read_i64(&index, base + 8)?,
                num_inline_buffer_segments: read_u32(&index, base + 16)?,
                segments_start: read_u32(&index, base + 20)?,
                segments_end: read_u32(&index, base + 24)?,
                deps_start: read_u32(&index, base + 28)?,
                deps_end: read_u32(&index, base + 32)?,
                sha1,
            });
        }

        let mut segments = Vec::with_capacity(segment_count);
        for i in 0..segment_count {
            let base = segments_at + i * FILE_SEGMENT_SIZE;
            segments.push(FileSegment {
                offset: read_u64(&index, base)?,
                zsize: read_u32(&index, base + 8)?,
                size: read_u32(&index, base + 12)?,
            });
        }

        let mut dependencies = Vec::with_capacity(dependency_count);
        for i in 0..dependency_count {
            let base = deps_at + i * DEPENDENCY_SIZE;
            dependencies.push(read_u64(&index, base)?);
        }

        // Sanidade: as faixas de segmento/dep de cada recurso têm de existir.
        for entry in &entries {
            if entry.segments_start > entry.segments_end
                || entry.segments_end as usize > segments.len()
            {
                return format_err(format!(
                    "recurso {:016x} aponta segmentos {}..{} fora de [0,{}]",
                    entry.name_hash,
                    entry.segments_start,
                    entry.segments_end,
                    segments.len()
                ));
            }
            if entry.deps_start > entry.deps_end
                || entry.deps_end as usize > dependencies.len()
            {
                return format_err(format!(
                    "recurso {:016x} aponta deps {}..{} fora de [0,{}]",
                    entry.name_hash,
                    entry.deps_start,
                    entry.deps_end,
                    dependencies.len()
                ));
            }
        }

        Ok(Archive {
            path: path.to_path_buf(),
            header,
            file_table_offset,
            file_table_size,
            crc,
            entries,
            segments,
            dependencies,
            custom_paths,
            custom_paths_need_kraken,
        })
    }

    /// Segmentos de um recurso, na ordem do arquivo. O primeiro é o principal;
    /// os demais são buffers.
    pub fn segments_of(&self, entry: &FileEntry) -> &[FileSegment] {
        &self.segments[entry.segments_start as usize..entry.segments_end as usize]
    }

    /// Bytes em disco do recurso (soma dos `zsize` de todos os segmentos).
    pub fn disk_size_of(&self, entry: &FileEntry) -> u64 {
        self.segments_of(entry)
            .iter()
            .map(|s| u64::from(s.zsize))
            .sum()
    }

    /// Tamanho descomprimido do segmento principal (o que vira o arquivo cooked).
    pub fn main_size_of(&self, entry: &FileEntry) -> u64 {
        self.segments_of(entry)
            .first()
            .map(|s| u64::from(s.size))
            .unwrap_or(0)
    }
}

enum LxrsResult {
    Paths(Vec<String>),
    NeedsKraken,
}

/// Lê o LxrsFooter (lista de paths embutida). Em disco: magic/version/size/
/// zsize/count + blob de `zsize` bytes. O blob é um stream Kraken **cru** (sem
/// header KARK) quando `size != zsize`; aí precisamos do Kraken. Quando
/// `size == zsize` os bytes já são os strings ISO-8859-1 terminados em NUL.
fn read_lxrs_footer(file: &mut File, custom_len: u32, file_len: u64) -> Result<LxrsResult> {
    let custom_len = u64::from(custom_len);
    if EXTENDED_SIZE + custom_len > file_len || custom_len < 20 {
        return format_err("LxrsFooter fora dos limites do arquivo");
    }
    file.seek(SeekFrom::Start(EXTENDED_SIZE))?;
    let mut head = [0u8; 20];
    file.read_exact(&mut head)?;
    if read_u32(&head, 0)? != LXRS_MAGIC {
        return format_err("magic do LxrsFooter inválido");
    }
    let size = read_u32(&head, 8)? as usize;
    let zsize = read_u32(&head, 12)? as usize;
    let count = read_u32(&head, 16)? as usize;
    if 20 + zsize as u64 > custom_len {
        return format_err("blob do LxrsFooter maior que custom_data_length");
    }

    let mut blob = vec![0u8; zsize];
    file.read_exact(&mut blob)?;

    // Três ramos, fiéis ao `LxrsFooter.Read` do WolvenKit:
    //   size > zsize  -> blob comprimido (stream Kraken cru) -> descomprime
    //   size < zsize  -> footer malformado: o WolvenKit ignora (lista vazia)
    //   size == zsize -> sem compressão
    let plain = match size.cmp(&zsize) {
        std::cmp::Ordering::Greater => match crate::kraken::decompress(&blob, size) {
            Ok(out) => out,
            Err(_) => return Ok(LxrsResult::NeedsKraken),
        },
        std::cmp::Ordering::Less => return Ok(LxrsResult::Paths(Vec::new())),
        std::cmp::Ordering::Equal => blob,
    };

    Ok(LxrsResult::Paths(decode_null_terminated_latin1(&plain, count)))
}

/// Decodifica até `count` strings ISO-8859-1 (Latin-1) terminadas em NUL.
fn decode_null_terminated_latin1(bytes: &[u8], count: usize) -> Vec<String> {
    // NÃO dimensionar por `count`: ele vem do arquivo sem bound, e um valor
    // gigante faria `Vec::with_capacity` abortar o processo (não é panic
    // capturável). O loop já para em `count` strings; o teto natural é o
    // número de NULs no blob, limitado por `bytes.len()`.
    let mut out = Vec::new();
    let mut current = String::new();
    for &b in bytes {
        if b == 0 {
            out.push(std::mem::take(&mut current));
            if out.len() >= count {
                break;
            }
        } else {
            // Latin-1: cada byte mapeia direto para o code point U+00xx.
            current.push(b as char);
        }
    }
    out
}

#[inline]
pub fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| Error::Format(format!("u32 truncado em {offset}")))?;
    Ok(u32::from_le_bytes(slice.try_into().unwrap()))
}

#[inline]
pub fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let slice = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| Error::Format(format!("u64 truncado em {offset}")))?;
    Ok(u64::from_le_bytes(slice.try_into().unwrap()))
}

#[inline]
pub fn read_i64(bytes: &[u8], offset: usize) -> Result<i64> {
    Ok(read_u64(bytes, offset)? as i64)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::hashes::fnv1a64;
    use std::io::Write;

    /// Constrói um `.archive` mínimo e válido com `segments`/`entries` dados.
    /// Cada segmento é `(bytes_do_payload, size_descomprimido)`. O `zsize` vira
    /// o tamanho real dos bytes.
    pub(crate) fn build_archive(
        segments: &[(Vec<u8>, u32)],
        entries: &[(u64, u32, u32)], // (name_hash, seg_start, seg_end)
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        let payload_base = 44u64; // logo após header + custom_data_length
        let mut seg_records = Vec::new();
        for (data, size) in segments {
            let offset = payload_base + payload.len() as u64;
            seg_records.push((offset, data.len() as u32, *size));
            payload.extend_from_slice(data);
        }

        // Índice
        let mut index = Vec::new();
        index.extend_from_slice(&8u32.to_le_bytes()); // file_table_offset
        let file_table_size_pos = index.len();
        index.extend_from_slice(&0u32.to_le_bytes()); // file_table_size (preenchido depois)
        index.extend_from_slice(&0u64.to_le_bytes()); // crc
        index.extend_from_slice(&(entries.len() as u32).to_le_bytes());
        index.extend_from_slice(&(seg_records.len() as u32).to_le_bytes());
        index.extend_from_slice(&1u32.to_le_bytes()); // 1 dependência
        for (name_hash, seg_start, seg_end) in entries {
            index.extend_from_slice(&name_hash.to_le_bytes());
            index.extend_from_slice(&0i64.to_le_bytes()); // timestamp
            index.extend_from_slice(&0u32.to_le_bytes()); // num_inline
            index.extend_from_slice(&seg_start.to_le_bytes());
            index.extend_from_slice(&seg_end.to_le_bytes());
            index.extend_from_slice(&0u32.to_le_bytes()); // dep_start
            index.extend_from_slice(&1u32.to_le_bytes()); // dep_end
            index.extend_from_slice(&[0u8; 20]); // sha1
        }
        for (offset, zsize, size) in &seg_records {
            index.extend_from_slice(&offset.to_le_bytes());
            index.extend_from_slice(&zsize.to_le_bytes());
            index.extend_from_slice(&size.to_le_bytes());
        }
        index.extend_from_slice(&0xDEAD_BEEF_u64.to_le_bytes()); // dependência

        let file_table_size = (index.len() - 8) as u32;
        index[file_table_size_pos..file_table_size_pos + 4]
            .copy_from_slice(&file_table_size.to_le_bytes());

        let index_position = payload_base + payload.len() as u64;
        let index_size = index.len() as u32;
        let filesize = index_position + index_size as u64;

        let mut out = Vec::new();
        out.extend_from_slice(&RDAR_MAGIC.to_le_bytes());
        out.extend_from_slice(&12u32.to_le_bytes()); // version
        out.extend_from_slice(&index_position.to_le_bytes());
        out.extend_from_slice(&index_size.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes()); // debug_position
        out.extend_from_slice(&0u32.to_le_bytes()); // debug_size
        out.extend_from_slice(&filesize.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // custom_data_length
        out.extend_from_slice(&payload);
        out.extend_from_slice(&index);
        out
    }

    fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("archive-tool-test-{name}.archive"));
        let mut f = File::create(&path).unwrap();
        f.write_all(bytes).unwrap();
        path
    }

    #[test]
    fn le_um_archive_nao_comprimido() {
        let hash = fnv1a64(b"base\\test.txt");
        let bytes = build_archive(&[(b"HELLO".to_vec(), 5)], &[(hash, 0, 1)]);
        let path = write_temp("read", &bytes);

        let ar = Archive::open(&path).unwrap();
        assert_eq!(ar.header.version, 12);
        assert_eq!(ar.entries.len(), 1);
        assert_eq!(ar.segments.len(), 1);
        assert_eq!(ar.dependencies, vec![0xDEAD_BEEF]);
        assert_eq!(ar.entries[0].name_hash, hash);
        assert_eq!(ar.entries[0].segment_count(), 1);
        assert_eq!(ar.disk_size_of(&ar.entries[0]), 5);
        assert!(!ar.segments[0].size_differs());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn rejeita_magic_invalido() {
        let mut bytes = build_archive(&[(b"X".to_vec(), 1)], &[(1, 0, 1)]);
        bytes[0] ^= 0xFF;
        let path = write_temp("badmagic", &bytes);
        assert!(matches!(Archive::open(&path), Err(Error::Format(_))));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn rejeita_indice_fora_do_arquivo() {
        let mut bytes = build_archive(&[(b"X".to_vec(), 1)], &[(1, 0, 1)]);
        // Aponta index_position para muito além do fim.
        bytes[8..16].copy_from_slice(&9_000_000u64.to_le_bytes());
        let path = write_temp("badindex", &bytes);
        assert!(matches!(Archive::open(&path), Err(Error::Format(_))));
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn decodifica_strings_latin1() {
        let blob = b"base\\a.mesh\0base\\b.xbm\0";
        let paths = decode_null_terminated_latin1(blob, 2);
        assert_eq!(paths, vec!["base\\a.mesh", "base\\b.xbm"]);
    }

    #[test]
    fn decode_nao_aloca_por_count_gigante() {
        // Um `count` gigante NÃO pode dimensionar a alocação (Vec::with_capacity
        // abortaria o processo). O loop deve parar pelos NULs reais do blob.
        let paths = decode_null_terminated_latin1(b"a\0bb\0", usize::MAX);
        assert_eq!(paths, vec!["a", "bb"]);
    }
}
