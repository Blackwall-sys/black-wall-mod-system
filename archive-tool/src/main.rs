//! archive-tool — CLI para o formato `.archive` (RDAR) do Cyberpunk 2077 no
//! macOS (Apple Silicon).
//!
//! Comandos:
//!   list    [<dir>]                       Lista os .archive (padrão: diretório do jogo).
//!   info    <nome|caminho>                Resumo de uma linha.
//!   datamap <nome|caminho> [-o ...]       Gera o datamap.md do índice.
//!   extract <nome|caminho> [<dest>]       Extrai para <dest>/<nome>/ (padrão: ao lado do archive).
//!   extract --all [<dest>]                Extrai TODOS os archives do jogo, cada um na sua pasta.
//!
//! Local do jogo: vem embutido (este Mac), e pode ser trocado por `CP77_CONTENT`
//! (aponta direto p/ a pasta content) ou `CP77_DIR` (raiz do jogo). Os nomes dos
//! recursos são resolvidos automaticamente pela `usedhashes.kark` do projeto.

mod archive;
mod datamap;
mod extract;
mod hashes;
mod kraken;
mod time;

use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use archive::Archive;
use hashes::PathDictionary;

/// Locais padrão deste Mac (descobertos uma vez; overridáveis por env).
mod defaults {
    use std::path::PathBuf;

    // Sem const fixa: a raiz vem de env (CP77_CONTENT/CP77_DIR) ou da instalação
    // Steam padrão sob o HOME do usuário (portável, não embute a máquina).

    /// Lista hash→path da comunidade (`usedhashes.kark` do WolvenKit), no projeto.
    /// Resolvida em tempo de compilação relativa ao crate; usada se existir.
    pub fn hashes_path() -> PathBuf {
        // ao lado do exe se existir; senão relativo (sem env! que embute o path do projeto).
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let beside = dir.join("usedhashes.kark");
                if beside.is_file() {
                    return beside;
                }
            }
        }
        PathBuf::from("usedhashes.kark")
    }

    pub fn content_dir() -> PathBuf {
        if let Some(c) = std::env::var_os("CP77_CONTENT") {
            return PathBuf::from(c);
        }
        if let Some(root) = std::env::var_os("CP77_DIR") {
            let root = PathBuf::from(root);
            let mac = root.join("archive/Mac/content");
            if mac.is_dir() {
                return mac;
            }
            let pc = root.join("archive/pc/content");
            if pc.is_dir() {
                return pc;
            }
            return mac;
        }
        if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
            return h.join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077/archive/Mac/content");
        }
        PathBuf::from("archive/Mac/content")
    }
}

const USAGE: &str = "\
archive-tool — lê/extrai .archive (RDAR) do Cyberpunk 2077 (macOS)

USO:
    archive-tool list    [<dir>]
    archive-tool info    <nome|caminho>
    archive-tool datamap <nome|caminho> [-o <out.md|->] [--hashes <lista>] [--no-hashes]
    archive-tool extract <nome|caminho> [<dest>] [opções]
    archive-tool extract --all [<dest>] [opções]

O <nome> pode ser só o nome do archive (ex.: `basegame_2_mainmenu`): é procurado
no diretório do jogo, que já vem embutido. Override: CP77_CONTENT ou CP77_DIR.

COMANDOS:
    list      Lista os .archive do diretório do jogo (ou de <dir>).
    info      Resumo de uma linha (versão, contagens, tamanhos).
    datamap   Gera datamap.md do índice. Padrão: <archive>.datamap.md
    extract   Extrai os recursos. Por padrão cria, AO LADO do archive, uma pasta
              com o nome dele contendo o conteúdo: <dest>/<nome>/...
              Com --all, faz isso para todos os archives do jogo.

OPÇÕES de extract:
    --all                  extrai todos os archives do diretório do jogo
    --datamap              também grava <dest>/<nome>/datamap.md
    --hashes <lista>       lista de paths (texto ou .kark) para resolver nomes
    --no-hashes            não usar a lista de hashes embutida
    --decompress-buffers   descomprime também os segmentos de buffer
    --skip-unresolved      não extrai recursos sem nome (padrão: unknown/<hash>.bin)

GLOBAIS:
    -h, --help       mostra esta ajuda
    -V, --version    mostra a versão
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("erro: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some(first) = args.first() else {
        print!("{USAGE}");
        return Ok(());
    };

    match first.as_str() {
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("archive-tool {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "list" => cmd_list(&args[1..]),
        "datamap" => cmd_datamap(&args[1..]),
        "extract" => cmd_extract(&args[1..]),
        "info" => cmd_info(&args[1..]),
        other => Err(format!(
            "comando desconhecido '{other}'. Use `archive-tool --help`."
        )),
    }
}

// ---- resolução de archives, dicionário e helpers ----

/// Resolve um argumento de archive: caminho existente, ou nome procurado no
/// diretório do jogo (com ou sem a extensão `.archive`).
fn resolve_archive_arg(arg: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(arg);
    if direct.is_file() {
        return Ok(direct);
    }
    let content = defaults::content_dir();
    for candidate in [content.join(arg), content.join(format!("{arg}.archive"))] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "archive '{arg}' não encontrado (nem como caminho, nem em {}). \
         Ajuste CP77_CONTENT/CP77_DIR ou passe um caminho.",
        content.display()
    ))
}

/// Lista os `.archive` de um diretório, ordenados por nome.
fn list_archives(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let read = std::fs::read_dir(dir)
        .map_err(|e| format!("não consegui ler {}: {e}", dir.display()))?;
    let mut archives: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("archive"))
        })
        .collect();
    archives.sort();
    Ok(archives)
}

/// Decide qual lista de hashes usar: explícita > embutida (se existir) > nenhuma.
fn resolve_hashes(explicit: Option<PathBuf>, no_hashes: bool) -> Option<PathBuf> {
    if no_hashes {
        return None;
    }
    if explicit.is_some() {
        return explicit;
    }
    let default = defaults::hashes_path();
    default.is_file().then_some(default)
}

/// Dicionário base, carregado uma vez (a lista de hashes vale para todos os
/// archives). Os paths embutidos de cada archive entram depois, via [`add_archive_paths`].
fn load_base_dict(hashes: Option<&Path>) -> Result<PathDictionary, String> {
    let mut dict = PathDictionary::new();
    if let Some(list) = hashes {
        let n = dict
            .load_list(list)
            .map_err(|e| format!("não consegui ler a lista de hashes {}: {e}", list.display()))?;
        eprintln!("dicionário: {n} paths de {}", list.display());
    }
    Ok(dict)
}

/// Adiciona ao dicionário os paths embutidos no LxrsFooter do archive (corretos
/// globalmente: são mapeamentos hash→path, válidos para qualquer archive).
fn add_archive_paths(dict: &mut PathDictionary, ar: &Archive) {
    for p in &ar.custom_paths {
        dict.insert_path(p);
    }
    if !ar.custom_paths.is_empty() {
        eprintln!("dicionário: +{} paths embutidos do LxrsFooter", ar.custom_paths.len());
    }
    if ar.custom_paths_need_kraken {
        eprintln!("aviso: LxrsFooter comprimido; nomes embutidos exigem Kraken para ler.");
    }
}

fn write_datamap_file(
    ar: &Archive,
    dict: &PathDictionary,
    path: &Path,
) -> Result<datamap::Stats, String> {
    let file = std::fs::File::create(path)
        .map_err(|e| format!("não consegui criar {}: {e}", path.display()))?;
    let mut w = BufWriter::new(file);
    let stats = datamap::write_datamap(ar, dict, &mut w)
        .map_err(|e| format!("falha ao escrever datamap: {e}"))?;
    w.flush().map_err(|e| e.to_string())?;
    eprintln!("datamap escrito em {}", path.display());
    Ok(stats)
}

// ---- comandos ----

fn cmd_list(args: &[String]) -> Result<(), String> {
    let dir = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(PathBuf::from)
        .unwrap_or_else(defaults::content_dir);
    let archives = list_archives(&dir)?;
    if archives.is_empty() {
        return Err(format!("nenhum .archive em {}", dir.display()));
    }
    println!("{} archives em {}:", archives.len(), dir.display());
    let mut total = 0u64;
    for p in &archives {
        let size = std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
        total += size;
        println!(
            "  {:>13}  {}",
            human(size),
            p.file_name().unwrap_or_default().to_string_lossy()
        );
    }
    println!("total: {}", human(total));
    Ok(())
}

fn cmd_info(args: &[String]) -> Result<(), String> {
    let arg = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .ok_or("info exige <nome|caminho>")?;
    let path = resolve_archive_arg(arg)?;
    let ar = Archive::open(&path).map_err(|e| format!("{} — {e}", path.display()))?;
    let disk: u64 = ar.segments.iter().map(|s| u64::from(s.zsize)).sum();
    let raw: u64 = ar.segments.iter().map(|s| u64::from(s.size)).sum();
    let comp = ar.segments.iter().filter(|s| s.size_differs()).count();
    println!(
        "{}: RDAR v{} · {} recursos · {} segmentos ({comp} comprimidos) · {} deps · disco {} · descomprimido {}",
        path.file_name().unwrap_or_default().to_string_lossy(),
        ar.header.version,
        ar.entries.len(),
        ar.segments.len(),
        ar.dependencies.len(),
        human(disk),
        human(raw),
    );
    Ok(())
}

fn cmd_datamap(args: &[String]) -> Result<(), String> {
    let mut archive: Option<String> = None;
    let mut out: Option<String> = None;
    let mut hashes: Option<PathBuf> = None;
    let mut no_hashes = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-o" | "--output" => out = Some(it.next().ok_or("-o exige um caminho")?.clone()),
            "--hashes" => hashes = Some(PathBuf::from(it.next().ok_or("--hashes exige um caminho")?)),
            "--no-hashes" => no_hashes = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if archive.is_none() && !other.starts_with('-') => archive = Some(other.to_string()),
            other => return Err(format!("argumento inesperado em datamap: '{other}'")),
        }
    }

    let archive = resolve_archive_arg(&archive.ok_or("datamap exige <nome|caminho>")?)?;
    let mut dict = load_base_dict(resolve_hashes(hashes, no_hashes).as_deref())?;
    let ar = Archive::open(&archive).map_err(|e| format!("{} — {e}", archive.display()))?;
    add_archive_paths(&mut dict, &ar);

    // Saída: stdout se "-", senão o caminho dado, senão <archive>.datamap.md.
    let out_path = match out.as_deref() {
        Some("-") => None,
        Some(p) => Some(PathBuf::from(p)),
        None => Some(default_datamap_path(&archive)),
    };

    let stats = match &out_path {
        None => {
            let stdout = io::stdout();
            let mut w = BufWriter::new(stdout.lock());
            // Cano fechado a jusante (ex.: `| head`) é saída limpa, padrão Unix.
            let stats = match datamap::write_datamap(&ar, &dict, &mut w) {
                Ok(s) => s,
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
                Err(e) => return Err(format!("falha ao escrever datamap: {e}")),
            };
            if let Err(e) = w.flush() {
                if e.kind() == io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(e.to_string());
            }
            stats
        }
        Some(path) => write_datamap_file(&ar, &dict, path)?,
    };

    eprintln!(
        "{} recursos · {} segmentos ({} comprimidos) · {} deps · {} nomes resolvidos · {} em disco / {} descomprimidos",
        stats.entries,
        stats.segments,
        stats.compressed_segments,
        stats.dependencies,
        stats.resolved,
        human(stats.total_disk),
        human(stats.total_uncompressed),
    );
    Ok(())
}

fn cmd_extract(args: &[String]) -> Result<(), String> {
    let mut positionals: Vec<String> = Vec::new();
    let mut hashes: Option<PathBuf> = None;
    let mut no_hashes = false;
    let mut all = false;
    let mut also_datamap = false;
    let mut opts = extract::ExtractOptions::default();

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--all" => all = true,
            "--hashes" => hashes = Some(PathBuf::from(it.next().ok_or("--hashes exige um caminho")?)),
            "--no-hashes" => no_hashes = true,
            "--datamap" => also_datamap = true,
            "--decompress-buffers" => opts.decompress_buffers = true,
            "--skip-unresolved" => opts.keep_unresolved = false,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if other.starts_with('-') => {
                return Err(format!("opção desconhecida em extract: '{other}'"));
            }
            other => positionals.push(other.to_string()),
        }
    }

    let mut dict = load_base_dict(resolve_hashes(hashes, no_hashes).as_deref())?;

    // Define a lista de archives e a base de saída.
    let (archives, base): (Vec<PathBuf>, PathBuf) = if all {
        let content = defaults::content_dir();
        let archives = list_archives(&content)?;
        if archives.is_empty() {
            return Err(format!("nenhum .archive em {}", content.display()));
        }
        // base de saída = positional[0] ou o próprio diretório do jogo (ao lado).
        let base = positionals.first().map(PathBuf::from).unwrap_or_else(|| content.clone());
        eprintln!("--all: {} archives de {}", archives.len(), content.display());
        (archives, base)
    } else {
        let arg = positionals.first().ok_or("extract exige <nome|caminho> ou --all")?;
        let ar_path = resolve_archive_arg(arg)?;
        // base de saída = positional[1] ou o diretório do próprio archive (ao lado).
        let base = positionals
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| ar_path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(".")));
        (vec![ar_path], base)
    };

    if !kraken::is_available() {
        eprintln!(
            "aviso: Kraken indisponível (build --no-default-features) — recursos KARK serão pulados."
        );
    }

    let multi = archives.len() > 1;
    let (mut g_ext, mut g_skip, mut g_err) = (0usize, 0usize, 0usize);

    for ar_path in &archives {
        let ar = Archive::open(ar_path).map_err(|e| format!("{} — {e}", ar_path.display()))?;
        add_archive_paths(&mut dict, &ar);

        // Pasta de saída = <base>/<nome-do-archive-sem-extensão>/
        let stem = ar_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "archive".into());
        let target = base.join(&stem);
        std::fs::create_dir_all(&target)
            .map_err(|e| format!("não consegui criar {}: {e}", target.display()))?;
        eprintln!(
            "extraindo {} -> {}/",
            ar_path.file_name().unwrap_or_default().to_string_lossy(),
            target.display()
        );

        if also_datamap {
            write_datamap_file(&ar, &dict, &target.join("datamap.md"))?;
        }

        let report = extract::extract_all(&ar, &dict, &target, &opts)
            .map_err(|e| format!("falha na extração de {}: {e}", ar_path.display()))?;
        println!(
            "  {} extraídos · {} pulados · {} erros",
            report.extracted, report.skipped_need_kraken, report.errors
        );
        g_ext += report.extracted;
        g_skip += report.skipped_need_kraken;
        g_err += report.errors;

        // Amostras só no modo single (evita poluir a saída do --all).
        if !multi {
            print_samples(&report);
        }
    }

    if multi {
        println!("TOTAL: {g_ext} extraídos · {g_skip} pulados · {g_err} erros em {} archives", archives.len());
    }
    Ok(())
}

fn print_samples(report: &extract::ExtractReport) {
    if !report.skipped_samples.is_empty() {
        eprintln!("amostra de pulados (precisam de Kraken):");
        for (hash, name) in &report.skipped_samples {
            eprintln!("  {hash:016x}  {name}");
        }
        if report.skipped_need_kraken > report.skipped_samples.len() {
            eprintln!(
                "  … e mais {} recursos",
                report.skipped_need_kraken - report.skipped_samples.len()
            );
        }
    }
    if !report.error_samples.is_empty() {
        eprintln!("amostra de erros de descompressão:");
        for (hash, motivo) in &report.error_samples {
            eprintln!("  {hash:016x}  {motivo}");
        }
    }
}

fn default_datamap_path(archive: &Path) -> PathBuf {
    let mut name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "archive".to_string());
    name.push_str(".datamap.md");
    archive.with_file_name(name)
}

/// Tamanho legível (B/KiB/MiB/GiB) com o número cru entre ().
fn human(n: u64) -> String {
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
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caminho_padrao_do_datamap() {
        let p = default_datamap_path(Path::new("/x/basegame.archive"));
        assert_eq!(p, PathBuf::from("/x/basegame.archive.datamap.md"));
    }

    #[test]
    fn human_formata() {
        assert_eq!(human(512), "512 B");
        assert!(human(2048).starts_with("2.0 KiB"));
    }
}
