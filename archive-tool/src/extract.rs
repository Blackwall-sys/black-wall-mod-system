//! Extração de recursos do `.archive` para uma pasta de conteúdo.
//!
//! Algoritmo (port de `Archive.CopyFileToStream` + `Oodle.DecompressAndCopySegment`
//! do WolvenKit): um recurso = concatenação dos seus segmentos
//! `[segments_start .. segments_end)`. O **primeiro** (principal) é sempre
//! descomprimido; os buffers seguintes são copiados crus por padrão (a menos de
//! `--decompress-buffers`). Um segmento exige Kraken só quando `zsize != size`
//! e começa com o magic `KARK`; caso contrário a saída é cópia byte-a-byte.
//!
//! Sem o backend Kraken (ooz arm64) ainda compilado, recursos cujo segmento
//! principal é `KARK`-comprimido são **pulados** e listados no relatório — o que
//! não precisa de Kraken extrai normalmente.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use crate::archive::{Archive, FileEntry, FileSegment};
use crate::hashes::PathDictionary;
use crate::kraken;

#[derive(Default)]
pub struct ExtractReport {
    pub extracted: usize,
    pub skipped_need_kraken: usize,
    pub errors: usize,
    /// Amostra de recursos pulados por falta de Kraken (hash, nome).
    pub skipped_samples: Vec<(u64, String)>,
    /// Amostra de recursos com erro de descompressão (hash, motivo).
    pub error_samples: Vec<(u64, String)>,
}

pub struct ExtractOptions {
    /// Também descomprimir os segmentos de buffer (não só o principal).
    pub decompress_buffers: bool,
    /// Recursos sem nome resolvido vão para `unknown/<hash>.bin` em vez de pular.
    pub keep_unresolved: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            decompress_buffers: false,
            keep_unresolved: true,
        }
    }
}

pub fn extract_all(
    ar: &Archive,
    dict: &PathDictionary,
    out_dir: &Path,
    opts: &ExtractOptions,
) -> io::Result<ExtractReport> {
    let mut file = File::open(&ar.path)?;
    let file_len = file.metadata()?.len();
    let mut report = ExtractReport::default();

    for entry in &ar.entries {
        let rel = match output_relative_path(entry, dict, opts.keep_unresolved) {
            Some(p) => p,
            None => continue, // sem nome e keep_unresolved=false
        };

        match extract_entry(&mut file, ar, entry, opts, file_len) {
            Ok(bytes) => {
                let dest = out_dir.join(&rel);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                File::create(&dest)?.write_all(&bytes)?;
                report.extracted += 1;
            }
            Err(ExtractError::NeedsKraken) => {
                report.skipped_need_kraken += 1;
                if report.skipped_samples.len() < 20 {
                    report
                        .skipped_samples
                        .push((entry.name_hash, rel.to_string_lossy().into_owned()));
                }
            }
            Err(ExtractError::Io(e)) => return Err(e),
            Err(ExtractError::Corrupt(msg)) => {
                report.errors += 1;
                if report.error_samples.len() < 20 {
                    report.error_samples.push((entry.name_hash, msg));
                }
            }
        }
    }

    Ok(report)
}

/// Extrai UM recurso por `name_hash` (FNV-1a64 do path), descomprimindo (Kraken se preciso), SEM
/// materializar o resto do archive. Ideal p/ pegar um arquivo de um archive gigante — ex.: um factory
/// `.csv` de 259B do `basegame_4_gamedata` (3.3GB) — sem extrair tudo. Devolve os bytes do recurso.
pub fn extract_one(ar: &Archive, name_hash: u64) -> Result<Vec<u8>, String> {
    let entry = ar
        .entries
        .iter()
        .find(|e| e.name_hash == name_hash)
        .ok_or_else(|| format!("recurso {name_hash:#018x} não está neste archive"))?;
    let mut file = File::open(&ar.path).map_err(|e| e.to_string())?;
    let file_len = file.metadata().map_err(|e| e.to_string())?.len();
    let opts = ExtractOptions { decompress_buffers: true, keep_unresolved: true };
    extract_entry(&mut file, ar, entry, &opts, file_len).map_err(|e| match e {
        ExtractError::NeedsKraken => "recurso comprimido, mas o build está sem a feature `kraken`".to_string(),
        ExtractError::Io(e) => e.to_string(),
        ExtractError::Corrupt(m) => m,
    })
}

enum ExtractError {
    NeedsKraken,
    Io(io::Error),
    Corrupt(String),
}

impl From<io::Error> for ExtractError {
    fn from(e: io::Error) -> Self {
        ExtractError::Io(e)
    }
}

/// Materializa os bytes de um recurso na memória (cooked file). Para o caso de
/// uso atual — arquivos individuais de mod — isso é barato; um stream seria
/// melhor para recursos gigantes, mas a simplicidade vence aqui.
fn extract_entry(
    file: &mut File,
    ar: &Archive,
    entry: &FileEntry,
    opts: &ExtractOptions,
    file_len: u64,
) -> Result<Vec<u8>, ExtractError> {
    let segments = ar.segments_of(entry);
    let mut out = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let is_main = i == 0;
        let decompress = is_main || opts.decompress_buffers;
        copy_segment(file, seg, decompress, file_len, &mut out)?;
    }
    Ok(out)
}

/// Port de `Oodle.DecompressAndCopySegment`.
fn copy_segment(
    file: &mut File,
    seg: &FileSegment,
    decompress: bool,
    file_len: u64,
    out: &mut Vec<u8>,
) -> Result<(), ExtractError> {
    // A faixa do segmento tem de caber no arquivo. Sem isto, um `zsize`
    // malicioso (até ~4 GiB) guiaria a alocação antes de qualquer leitura
    // falhar. O WolvenKit usa uma view mapeada, limitada pelo arquivo; aqui
    // validamos explicitamente.
    if seg
        .offset
        .checked_add(u64::from(seg.zsize))
        .map_or(true, |e| e > file_len)
    {
        return Err(ExtractError::Corrupt(format!(
            "segmento @ {} +{} ultrapassa o arquivo ({file_len} B)",
            seg.offset, seg.zsize
        )));
    }

    // Cópia crua em streaming (sem materializar um buffer de `zsize` zerado):
    // a faixa já foi validada, então `take(zsize)` lê exatamente `zsize` bytes.
    let raw_copy = |file: &mut File, out: &mut Vec<u8>| -> io::Result<()> {
        file.seek(SeekFrom::Start(seg.offset))?;
        let read = io::copy(&mut (&mut *file).take(u64::from(seg.zsize)), out)?;
        debug_assert_eq!(read, u64::from(seg.zsize));
        Ok(())
    };

    // Cópia crua quando não devemos descomprimir, quando disco==descomprimido,
    // ou quando é curto demais para um header KARK.
    if !decompress || seg.zsize == seg.size || (seg.zsize as usize) < 8 {
        raw_copy(file, out)?;
        return Ok(());
    }

    // Lê o header de 8 bytes e decide pela mesma regra do datamap/WolvenKit.
    file.seek(SeekFrom::Start(seg.offset))?;
    let mut header = [0u8; 8];
    file.read_exact(&mut header)?;
    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    if !kraken::segment_needs_kraken(seg.zsize, seg.size, Some(magic)) {
        // Não é Kraken (zsize≠size mas sem KARK): copia os zsize bytes crus.
        raw_copy(file, out)?;
        return Ok(());
    }

    // KARK: o tamanho descomprimido vem SEMPRE do header (sobrepõe o da tabela,
    // incondicionalmente, como `if (headerSize != size) size = headerSize;` no
    // WolvenKit — quando são iguais, header == tabela de qualquer forma).
    let out_size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let comp_len = seg.zsize as usize - 8;
    let mut comp = vec![0u8; comp_len];
    file.read_exact(&mut comp)?;

    match kraken::decompress(&comp, out_size) {
        Ok(decoded) => {
            out.extend_from_slice(&decoded);
            Ok(())
        }
        Err(kraken::KrakenError::Unavailable) => Err(ExtractError::NeedsKraken),
        Err(e) => Err(ExtractError::Corrupt(e.to_string())),
    }
}

/// Caminho de saída relativo, seguro (sem `..`, sem raiz absoluta). Resolve pelo
/// dicionário; senão cai em `unknown/<hash>.bin` (se `keep_unresolved`).
fn output_relative_path(
    entry: &FileEntry,
    dict: &PathDictionary,
    keep_unresolved: bool,
) -> Option<PathBuf> {
    match dict.resolve(entry.name_hash) {
        Some(name) => Some(sanitize_relative(name)),
        None if keep_unresolved => {
            Some(PathBuf::from("unknown").join(format!("{:016x}.bin", entry.name_hash)))
        }
        None => None,
    }
}

/// Converte um path REDengine (`base\char\v.mesh`) num path relativo seguro:
/// separadores normais, sem componentes `..` ou raízes, para nunca escapar do
/// diretório de saída.
fn sanitize_relative(name: &str) -> PathBuf {
    let unified = name.replace('\\', "/");
    let mut safe = PathBuf::new();
    for comp in Path::new(&unified).components() {
        // Só componentes normais; ignora raízes, prefixos (C:\), `.` e `..`
        // — anti path traversal.
        if let Component::Normal(part) = comp {
            safe.push(part);
        }
    }
    if safe.as_os_str().is_empty() {
        safe.push("unnamed");
    }
    safe
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::tests::build_archive;
    #[cfg(not(feature = "kraken"))]
    use crate::archive::KARK_MAGIC;
    use crate::hashes::fnv1a64;

    fn temp_archive(name: &str, bytes: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(format!("archive-tool-extract-{name}.archive"));
        File::create(&path).unwrap().write_all(bytes).unwrap();
        path
    }

    #[test]
    fn extrai_segmento_nao_comprimido() {
        let resource = "base\\readme.txt";
        let hash = fnv1a64(b"base\\readme.txt");
        let bytes = build_archive(&[(b"conteudo cru".to_vec(), 12)], &[(hash, 0, 1)]);
        let ar_path = temp_archive("raw", &bytes);
        let ar = Archive::open(&ar_path).unwrap();
        let mut dict = PathDictionary::new();
        dict.insert_path(resource);

        let out_dir = std::env::temp_dir().join(format!("archive-tool-out-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out_dir);
        let report = extract_all(&ar, &dict, &out_dir, &ExtractOptions::default()).unwrap();

        assert_eq!(report.extracted, 1);
        assert_eq!(report.skipped_need_kraken, 0);
        let extracted = fs::read(out_dir.join("base/readme.txt")).unwrap();
        assert_eq!(extracted, b"conteudo cru");

        fs::remove_dir_all(&out_dir).ok();
        fs::remove_file(ar_path).ok();
    }

    #[cfg(not(feature = "kraken"))]
    #[test]
    fn pula_segmento_kark_sem_kraken() {
        // Segmento principal com header KARK e zsize != size -> precisa de Kraken.
        let mut seg = Vec::new();
        seg.extend_from_slice(&KARK_MAGIC.to_le_bytes());
        seg.extend_from_slice(&5u32.to_le_bytes()); // size no header
        seg.extend_from_slice(&[0xAA; 12]); // payload comprimido fake
        let zsize = seg.len() as u32; // 20
        let hash = fnv1a64(b"base\\comp.bin");
        let bytes = build_archive(&[(seg, 5)], &[(hash, 0, 1)]);
        assert_ne!(zsize, 5);

        let ar_path = temp_archive("kark", &bytes);
        let ar = Archive::open(&ar_path).unwrap();
        let mut dict = PathDictionary::new();
        dict.insert_path("base\\comp.bin");

        let out_dir = std::env::temp_dir().join(format!("archive-tool-out-kark-{}", std::process::id()));
        let _ = fs::remove_dir_all(&out_dir);
        let report = extract_all(&ar, &dict, &out_dir, &ExtractOptions::default()).unwrap();

        assert_eq!(report.extracted, 0);
        assert_eq!(report.skipped_need_kraken, 1);
        assert_eq!(report.skipped_samples.len(), 1);
        fs::remove_dir_all(&out_dir).ok();
        fs::remove_file(ar_path).ok();
    }

    #[test]
    fn segmento_fora_do_arquivo_vira_erro_nao_panica() {
        // Arquivo de 10 bytes; o segmento declara um zsize que ultrapassa o fim.
        // Deve virar Corrupt (e não alocar/ler nada), nunca dar panic/abort.
        let path = std::env::temp_dir()
            .join(format!("archive-tool-oob-{}.bin", std::process::id()));
        File::create(&path).unwrap().write_all(&[0u8; 10]).unwrap();
        let mut f = File::open(&path).unwrap();
        let seg = FileSegment {
            offset: 5,
            zsize: 1_000_000,
            size: 1_000_000,
        };
        let mut out = Vec::new();
        let r = copy_segment(&mut f, &seg, false, 10, &mut out);
        assert!(matches!(r, Err(ExtractError::Corrupt(_))));
        assert!(out.is_empty());
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn sanitize_bloqueia_path_traversal() {
        assert_eq!(sanitize_relative("base\\a.mesh"), PathBuf::from("base/a.mesh"));
        assert_eq!(
            sanitize_relative("..\\..\\etc\\passwd"),
            PathBuf::from("etc/passwd")
        );
        assert_eq!(sanitize_relative("/abs/root"), PathBuf::from("abs/root"));
    }

    #[test]
    fn recurso_sem_nome_vai_para_unknown() {
        let p = output_relative_path(
            &FileEntry {
                name_hash: 0xABCD,
                timestamp: 0,
                num_inline_buffer_segments: 0,
                segments_start: 0,
                segments_end: 1,
                deps_start: 0,
                deps_end: 0,
                sha1: [0; 20],
            },
            &PathDictionary::new(),
            true,
        );
        assert_eq!(p, Some(PathBuf::from("unknown/000000000000abcd.bin")));
    }
}
