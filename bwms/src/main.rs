//! bwms — CLI única do Black Wall Mod System.
//!
//! Uma porta de entrada. Os comandos de MOD (`classify`, `suggest`, `xl`) rodam in-process via `bwms_primitives::instalacao`; o
//! `bwms mods checar` (módulo `mods`) checa os scripts do jogo. Mod entra no jogo pela pasta `Mods/` ou pelo mod.io, não por aqui. O
//! `bwms tweak` (tweakdb.bin) é o módulo `tweak`, o `bwms archive` (.archive/CR2W) é o módulo `archive` e o `bwms input`
//! (keybinds de mod) é o módulo `input`, todos in-process. O `bwms comentarios` (corte de comentário do pacote público) é o módulo
//! `comentarios`.

mod archive;
mod comentarios;
mod input;
mod mods;
mod tweak;

use bwms_primitives::instalacao::{classify, theme};
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest: &[String] = if args.len() > 1 { &args[1..] } else { &[] };
    match cmd {
        // --- mods: in-process via bwms_primitives::instalacao ---
        "classify" => cmd_classify(rest),
        "suggest" => cmd_suggest(rest),
        "xl" => cmd_xl(rest),
        // o `bwms mods` lê o subcomando em args[1], como o argv de um binário: recebe `args` com o "mods" na frente
        "mods" => mods::run(&args),
        // --- ferramentas de dados, in-process ---
        "tweak" => tweak::run(rest),
        "archive" => archive::run(rest),
        "input" => input::run(rest),
        // --- empacotamento ---
        "comentarios" => comentarios::run(rest),
        // --- meta ---
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
    println!("tema:     {} (palavras casadas: {}) — {}", theme::category_label(&s.category), s.matched_words, s.reason);
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
    if !r.notes.is_empty() || !r.avisos_do_xl.is_empty() {
        println!("notas:");
        for n in r.notes.iter().chain(&r.avisos_do_xl) {
            println!("  • {n}");
        }
    }
    ExitCode::SUCCESS
}

/// `xl <arquivo.xl>` — parseia um `.xl` e imprime o resumo tipado (via `bwms_primitives::instalacao::xl`).
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
    match bwms_primitives::instalacao::xl::parse_xl(&text) {
        Ok(xl) => {
            // `bwms xl <file> reslink [out]` — emite os pares link/copy que o runtime lê em
            // <jogo>/red4ext/bwms-reslink.txt. Sem 'reslink' = só o summary.
            if args.get(1).map(String::as_str) == Some("reslink") {
                let plan = bwms_primitives::instalacao::tabelas_de_runtime::build_apply_plan(&xl);
                let out = bwms_primitives::instalacao::tabelas_de_runtime::emit_reslink_do_plano(&plan);
                eprintln!("[xl] {} link + {} copy", plan.mapa_links.len(), plan.mapa_copies.len());
                if !plan.cyclic_links.is_empty() {
                    eprintln!("[xl] link em ciclo, fora da tabela (corrija o .xl): {}", plan.cyclic_links.join(", "));
                }
                for a in &plan.avisos {
                    eprintln!("[xl] {a}");
                }
                return match args.get(2) {
                    Some(outp) => match std::fs::write(outp, &out) {
                        Ok(_) => {
                            eprintln!("[xl] escrito em {outp} (drope em <jogo>/red4ext/bwms-reslink.txt)");
                            ExitCode::SUCCESS
                        }
                        Err(e) => {
                            eprintln!("nao escreveu '{outp}': {e}");
                            ExitCode::FAILURE
                        }
                    },
                    None => {
                        print!("{out}");
                        ExitCode::SUCCESS
                    }
                };
            }
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
    println!("{} (palavras casadas: {}) — {}", theme::category_label(&s.category), s.matched_words, s.reason);
    ExitCode::SUCCESS
}

fn usage() {
    println!("bwms — Black Wall Mod System (CLI unificada)\n");
    println!("USO: bwms <comando> [args]\n");
    println!("  Mods (in-process, via bwms-primitives):");
    println!("    classify <pasta>    analisa um mod (classe, compat, arquivos, tema, deps, riscos)");
    println!("    suggest  <pasta>    sugere o tema (Roupas/Veículos/LUT/...)");
    println!("    xl       <arq.xl>   le um .xl (factories/patch/link/...) e resume");
    println!("    mods checar [jogo]  compila os scripts do jogo sem tocar no cache e aponta native func sem registro");
    println!();
    println!("  Ferramentas de dados:");
    println!("    tweak    [args]     le/edita tweakdb.bin (bwms tweak help)");
    println!("    archive  [args]     le/extrai .archive (bwms archive help)");
    println!("    input    [args]     mescla keybinds de mods (r6/input/*.xml) (bwms input --help)");
    println!();
    println!("  Empacotamento:");
    println!("    comentarios <entrada> <saida>  tira os comentários dos .reds e dos roteiros (.sh/.command) do pacote público");
    println!();
    println!("    help                esta ajuda");
}
