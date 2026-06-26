//! bwms — CLI única do Black Wall Mod System.
//!
//! Uma porta de entrada. Os comandos de MOD rodam in-process via `bwms-core`
//! (a mesma lógica que o mod-manager usa — sem duplicar, sem shellar). As
//! ferramentas de DADOS (tweak/archive), que carregam C++/ooz e têm build.rs
//! próprio, ficam como binários standalone e a `bwms` as front-enda.
//!
//! Mapa de onde cada função vive: `BWMS-MAPA.md` (ou `bwms map`).

use bwms_core::{apply, classify, theme};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest: &[String] = if args.len() > 1 { &args[1..] } else { &[] };
    match cmd {
        // --- mods: in-process via bwms-core ---
        "classify" => cmd_classify(rest),
        "suggest" => cmd_suggest(rest),
        "list" => cmd_list(rest),
        "apply" => cmd_apply(rest),
        "xl" => cmd_xl(rest),
        // --- ferramentas de dados: front-end dos binários standalone ---
        "tweak" => exec_tool("tweakdb-tool", "tweakdb-tool", rest),
        "archive" => exec_tool("archive-tool", "archive-tool", rest),
        "mods" => exec_tool("cp77-mods", "mac-mod-manager", rest),
        // --- meta ---
        "map" => cmd_map(),
        "help" | "-h" | "--help" => {
            usage();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("bwms: comando desconhecido '{other}'\n");
            usage();
            ExitCode::FAILURE
        }
    }
}

fn cmd_classify(args: &[String]) -> ExitCode {
    let Some(dir) = args.first() else {
        eprintln!("uso: bwms classify <pasta-do-mod>");
        return ExitCode::FAILURE;
    };
    let r = classify::classify(Path::new(dir));
    println!("mod:      {}", r.name);
    println!("classe:   {:?}", r.class);
    println!("compat:   {:?}", r.compat);
    println!("arquivos: {}", r.files.len());
    let s = theme::suggest(&r);
    println!("tema:     {} ({}%) — {}", theme::category_label(&s.category), s.confidence, s.reason);
    if !r.deps.is_empty() {
        println!("deps:");
        for d in &r.deps {
            println!("  - {} ({})", d.name, d.detail);
        }
    }
    if !r.risks.is_empty() {
        println!("riscos:");
        for x in &r.risks {
            println!("  ! {x}");
        }
    }
    if !r.notes.is_empty() {
        println!("notas:");
        for n in &r.notes {
            println!("  • {n}");
        }
    }
    ExitCode::SUCCESS
}

/// `xl <arquivo.xl>` — parseia um `.xl` do ArchiveXL e imprime o resumo tipado (via bwms-core).
fn cmd_xl(args: &[String]) -> ExitCode {
    let Some(path) = args.first() else {
        eprintln!("uso: bwms xl <arquivo.xl>");
        return ExitCode::FAILURE;
    };
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("não consegui ler '{path}': {e}");
            return ExitCode::FAILURE;
        }
    };
    match bwms_core::xl::parse_xl(&text) {
        Ok(xl) => {
            print!("{}", xl.summary());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erro ao parsear '{path}': {e}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_suggest(args: &[String]) -> ExitCode {
    let Some(dir) = args.first() else {
        eprintln!("uso: bwms suggest <pasta-do-mod>");
        return ExitCode::FAILURE;
    };
    let r = classify::classify(Path::new(dir));
    let s = theme::suggest(&r);
    println!("{} ({}%) — {}", theme::category_label(&s.category), s.confidence, s.reason);
    ExitCode::SUCCESS
}

fn cmd_list(args: &[String]) -> ExitCode {
    let Some(game) = args.first() else {
        eprintln!("uso: bwms list <pasta-do-jogo>");
        return ExitCode::FAILURE;
    };
    let states = apply::reconcile(Path::new(game));
    if states.is_empty() {
        println!("(nenhum mod em staging — esperado em <jogo>/BWMS/mods/<tema>/<mod>/)");
        return ExitCode::SUCCESS;
    }
    for m in &states {
        let mark = if m.active { "[x]" } else { "[ ]" };
        let fav = if m.favorite { " *" } else { "" };
        println!("{mark} {:14} {}{}", theme::category_label(&m.category), m.name, fav);
    }
    ExitCode::SUCCESS
}

fn cmd_apply(args: &[String]) -> ExitCode {
    let Some(game) = args.first() else {
        eprintln!("uso: bwms apply <pasta-do-jogo>");
        return ExitCode::FAILURE;
    };
    match apply::apply(Path::new(game)) {
        Ok((added, removed)) => {
            println!("apply ok: {added} .archive ativos no content, {removed} removidos");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("apply falhou: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Acha o binário de uma ferramenta standalone: ao lado da `bwms`, em
/// `<crate>/target/release/<bin>` subindo a árvore (a partir do exe e do CWD),
/// senão deixa o PATH resolver.
fn find_tool(bin: &str, crate_dir: &str) -> PathBuf {
    let rel = format!("{crate_dir}/target/release/{bin}");
    let mut cands: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            cands.push(d.join(bin));
        }
        for a in exe.ancestors() {
            cands.push(a.join(&rel));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for a in cwd.ancestors() {
            cands.push(a.join(&rel));
        }
    }
    for c in cands {
        if c.is_file() {
            return c;
        }
    }
    PathBuf::from(bin) // fallback: PATH
}

fn exec_tool(bin: &str, crate_dir: &str, args: &[String]) -> ExitCode {
    let path = find_tool(bin, crate_dir);
    match std::process::Command::new(&path).args(args).status() {
        Ok(st) => ExitCode::from(st.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(e) => {
            eprintln!("bwms: não consegui executar '{bin}' ({e}).");
            eprintln!("      compile: (cd {crate_dir} && cargo build --release)  ou ponha no PATH.");
            ExitCode::FAILURE
        }
    }
}

fn cmd_map() -> ExitCode {
    // Acha o BWMS-MAPA.md subindo a árvore a partir do exe e do CWD.
    let mut found = None;
    let probe = |start: &Path, found: &mut Option<PathBuf>| {
        for a in start.ancestors() {
            let p = a.join("BWMS-MAPA.md");
            if p.is_file() {
                *found = Some(p);
                break;
            }
            let p2 = a.join("mods-research").join("BWMS-MAPA.md");
            if p2.is_file() {
                *found = Some(p2);
                break;
            }
        }
    };
    if let Ok(exe) = std::env::current_exe() {
        probe(&exe, &mut found);
    }
    if found.is_none() {
        if let Ok(cwd) = std::env::current_dir() {
            probe(&cwd, &mut found);
        }
    }
    match found {
        Some(p) => {
            println!("{}", p.display());
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("bwms: BWMS-MAPA.md não encontrado por perto.");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    println!("bwms — Black Wall Mod System (CLI unificada)\n");
    println!("USO: bwms <comando> [args]\n");
    println!("  Mods (in-process, via bwms-core):");
    println!("    classify <pasta>    analisa um mod (classe, compat, arquivos, tema, deps, riscos)");
    println!("    suggest  <pasta>    sugere o tema (Roupas/Veículos/LUT/...)");
    println!("    list     <jogo>     lista os mods em staging (BWMS/mods/) e o estado");
    println!("    apply    <jogo>     sincroniza os .archive ativos pro content do jogo");
    println!("    xl       <arq.xl>   le um .xl do ArchiveXL (factories/patch/link/...) e resume");
    println!();
    println!("  Ferramentas de dados (front-end):");
    println!("    tweak    [args]     -> tweakdb-tool (le/edita tweakdb.bin)");
    println!("    archive  [args]     -> archive-tool (le/extrai .archive)");
    println!("    mods     [args]     -> cp77-mods (gerenciador completo / TUI)");
    println!();
    println!("    map                 caminho do BWMS-MAPA.md (onde agir em cada funcao)");
    println!("    help                esta ajuda");
}
