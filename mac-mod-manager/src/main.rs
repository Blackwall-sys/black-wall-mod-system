//! cp77-mods — instalador/validador SEGURO de mods Cyberpunk 2077 no macOS.
//!
//! Princípio (design do dono): o pacote DESCREVE, a ferramenta DECIDE e EXECUTA. NUNCA roda
//! comandos do autor do mod; instala só em destinos de uma lista fechada; transacional com
//! backup + rollback. Zero-dep (só std).
//!
//! v0.1: `classify <pasta>` — inspeciona e relata (classe/compat/deps/risco). Instalação
//! transacional vem nos próximos comandos (plan/install/list/remove), reusando este relatório.

mod install;
mod runtime;
mod source;
mod tui;

// Núcleo (classify/theme/apply) agora vem da lib compartilhada `bwms-core`,
// não mais de módulos locais — a CLI `bwms` usa a MESMA implementação.
use bwms_core::{apply, classify, theme};

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
cp77-mods — gerenciador SEGURO de mods CP2077 (macOS)

USO:
    cp77-mods classify <pasta>            inspeciona e relata (não instala nada)
    cp77-mods suggest  <pasta>            sugere o TEMA (roupa/carro/...) p/ confirmar
    cp77-mods apply            [game]     sincroniza os mods ATIVOS (staging→content, Path A)
    cp77-mods plan     <pasta> [game]     mostra o plano de instalação (dry-run)
    cp77-mods install  <pasta> [game]     instala transacional (backup+rollback)
    cp77-mods list             [game]     lista mods instalados
    cp77-mods remove   <nome>  [game]     remove um mod (restaura backups)

[game] = raiz do jogo; default = $CP77_DIR ou o caminho Steam conhecido.
Regra: NUNCA executa scripts do pacote; destinos = lista fechada; anti-path-traversal.
";

/// Raiz do jogo: arg explícito → $CP77_DIR → caminho Steam conhecido.
fn game_root(arg: Option<&String>) -> Option<PathBuf> {
    if let Some(a) = arg {
        let p = PathBuf::from(a);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(d) = std::env::var("CP77_DIR") {
        let p = PathBuf::from(d);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Some(h) = std::env::var_os("HOME").map(PathBuf::from) {
        let p = h.join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("classify") => match args.get(2) {
            Some(dir) => cmd_classify(Path::new(dir)),
            None => {
                eprintln!("uso: cp77-mods classify <pasta>");
                ExitCode::from(2)
            }
        },
        Some("suggest") => match args.get(2) {
            Some(dir) => cmd_suggest(Path::new(dir)),
            None => {
                eprintln!("uso: cp77-mods suggest <pasta>");
                ExitCode::from(2)
            }
        },
        Some("apply") => cmd_apply(game_root(args.get(2))),
        Some("plan") => cmd_plan_or_install(args.get(2), args.get(3), false),
        Some("install") => cmd_plan_or_install(args.get(2), args.get(3), true),
        Some("list") => cmd_list(game_root(args.get(2))),
        Some("remove") => cmd_remove(args.get(2), game_root(args.get(3))),
        Some("tui") | None => cmd_tui(game_root(args.get(2))),
        Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("comando desconhecido '{other}'. Use --help.");
            ExitCode::from(2)
        }
    }
}

fn cmd_plan_or_install(dir: Option<&String>, game_arg: Option<&String>, do_install: bool) -> ExitCode {
    let input = match dir {
        Some(d) => PathBuf::from(d),
        None => {
            eprintln!("uso: cp77-mods {} <pasta|.zip> [game]", if do_install { "install" } else { "plan" });
            return ExitCode::from(2);
        }
    };
    let dir = match source::open_source(&input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("erro: {e}");
            return ExitCode::from(1);
        }
    };
    let game = match game_root(game_arg) {
        Some(g) => g,
        None => {
            eprintln!("erro: raiz do jogo não encontrada (passe [game] ou defina CP77_DIR).");
            return ExitCode::from(1);
        }
    };
    let report = classify::classify(&dir);
    let mod_name = report.name.clone();
    let plan = install::build_plan(&report, &dir, &game, &mod_name);
    print_plan(&plan, &game);
    let xmod = cross_mod_conflicts(&plan, &game, &mod_name);
    if !xmod.is_empty() {
        println!("⚠ CONFLITO COM OUTRO MOD ({}):", xmod.len());
        for (dest, owner) in &xmod {
            println!("  ! {dest} já é do mod '{owner}' (será sobrescrito + backup)");
        }
    }
    if !report.risks.is_empty() {
        eprintln!("⚠ instalação BLOQUEADA por {} risco(s) — veja `classify`.", report.risks.len());
        return ExitCode::from(3);
    }
    if !do_install {
        println!("(dry-run — use `install` p/ aplicar)");
        return ExitCode::SUCCESS;
    }
    match install::install(&plan, &game, &mod_name) {
        Ok(man) => {
            println!("✓ instalado ({} arquivo(s)). Manifesto: {}", plan.actions.len(), man.display());
            let steps = runtime::activate(&report, &game);
            if !steps.is_empty() {
                println!("ATIVAÇÃO (runtime):");
                for s in &steps {
                    println!("  {} {} — {}", if s.ok { "✓" } else { "✗" }, s.step, s.detail);
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ FALHOU (revertido): {e}");
            ExitCode::from(1)
        }
    }
}

/// Acha dests do plano que JÁ pertencem a OUTRO mod (pelos manifestos) — conflito entre mods.
fn cross_mod_conflicts(plan: &install::Plan, game: &Path, this_mod: &str) -> Vec<(String, String)> {
    let man_dir = game.join(".cp77-mods/manifests");
    let mut owner: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(rd) = std::fs::read_dir(&man_dir) {
        for e in rd.flatten() {
            let stem = e.path().file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if stem == this_mod || stem.is_empty() {
                continue;
            }
            if let Ok(j) = std::fs::read_to_string(e.path()) {
                for f in install::json_str_array(&j, "installed") {
                    owner.insert(f, stem.clone());
                }
            }
        }
    }
    plan.actions
        .iter()
        .filter_map(|a| {
            let rel = a.dest.strip_prefix(game).unwrap_or(&a.dest).display().to_string();
            owner.get(&rel).map(|m| (rel, m.clone()))
        })
        .collect()
}

fn print_plan(plan: &install::Plan, game: &Path) {
    let rel = |p: &Path| p.strip_prefix(game).unwrap_or(p).display().to_string();
    println!("PLANO: {} cópia(s), {} conflito(s), {} pulado(s)", plan.actions.len(), plan.conflicts.len(), plan.skipped.len());
    for a in &plan.actions {
        println!("  + {}{}", rel(&a.dest), if a.overwrite { "  (sobrescreve → backup)" } else { "" });
    }
    for s in &plan.skipped {
        println!("  · pulado: {s}");
    }
}

fn cmd_tui(game: Option<PathBuf>) -> ExitCode {
    match game {
        Some(g) => {
            let _ = tui::run(g);
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("erro: raiz do jogo não encontrada. Defina CP77_DIR ou: cp77-mods tui <game>");
            print!("{USAGE}");
            ExitCode::from(1)
        }
    }
}

fn cmd_list(game: Option<PathBuf>) -> ExitCode {
    let game = match game {
        Some(g) => g,
        None => {
            eprintln!("erro: raiz do jogo não encontrada.");
            return ExitCode::from(1);
        }
    };
    let mods = install::list_installed(&game);
    if mods.is_empty() {
        println!("(nenhum mod instalado por esta ferramenta)");
        return ExitCode::SUCCESS;
    }
    println!("MODS INSTALADOS:");
    for m in &mods {
        println!("  • {} — {} arquivo(s)", m.name, m.files);
    }
    ExitCode::SUCCESS
}

fn cmd_remove(name: Option<&String>, game: Option<PathBuf>) -> ExitCode {
    let name = match name {
        Some(n) => n,
        None => {
            eprintln!("uso: cp77-mods remove <nome> [game]");
            return ExitCode::from(2);
        }
    };
    let game = match game {
        Some(g) => g,
        None => {
            eprintln!("erro: raiz do jogo não encontrada.");
            return ExitCode::from(1);
        }
    };
    match install::remove_mod(&game, name) {
        Ok(()) => {
            println!("✓ '{name}' removido (instalados apagados, backups restaurados).");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_apply(game: Option<PathBuf>) -> ExitCode {
    let game = match game {
        Some(g) => g,
        None => {
            eprintln!("erro: raiz do jogo não encontrada (defina CP77_DIR ou passe [game]).");
            return ExitCode::from(1);
        }
    };
    let states = apply::reconcile(&game);
    println!("MODS no staging (BWMS/mods/<tema>/):");
    if states.is_empty() {
        println!("  (vazio — largue mods em BWMS/mods/<tema>/<NomeDoMod>/)");
    }
    for m in &states {
        println!(
            "  [{}]{} {} — {}",
            if m.active { "x" } else { " " },
            if m.favorite { "★" } else { " " },
            m.name,
            theme::category_label(&m.category)
        );
    }
    match apply::apply(&game) {
        Ok((c, r)) => {
            println!("✓ sincronizado: {c} archive(s) ativo(s) em archive/Mac/content, {r} removido(s).");
            println!("  (mods de archive aplicam ao REINICIAR o jogo; cheats/runtime são ao vivo)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ erro ao sincronizar: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_suggest(input: &Path) -> ExitCode {
    let dir = match source::open_source(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("erro: {e}");
            return ExitCode::from(1);
        }
    };
    let r = classify::classify(&dir);
    let s = theme::suggest(&r);
    println!("MOD: {}", r.name);
    println!("Categoria sugerida: {}  (confiança {}%)", theme::category_label(&s.category), s.confidence);
    println!("Motivo: {}", s.reason);
    println!("Confirme ou corrija — categorias curadas (ou crie a sua: BWMS/mods/<slug>/):");
    for c in theme::CATEGORIES {
        let mark = if c.slug == s.category { "→" } else { " " };
        println!("  {mark} {} ({})", c.label, c.slug);
    }
    ExitCode::SUCCESS
}

fn cmd_classify(input: &Path) -> ExitCode {
    let dir = match source::open_source(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("erro: {e}");
            return ExitCode::from(1);
        }
    };
    let r = classify::classify(&dir);
    print_report(&r);
    // exit≠0 se houver risco que exige revisão humana
    if r.risks.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(3) }
}

fn class_label(c: classify::ModClass) -> &'static str {
    use classify::ModClass::*;
    match c {
        PureContent => "Conteúdo puro (instalável direto)",
        RedMod => "REDmod (entregar ao REDmod)",
        Script => "Script (redscript/CET — roda no jogo)",
        NativeCode => "Código nativo (.dll/.dylib — canal restrito)",
        Mixed => "Misto (conteúdo + script)",
        Unknown => "Desconhecido (nada reconhecido)",
    }
}

fn compat_label(c: classify::Compat) -> &'static str {
    use classify::Compat::*;
    match c {
        Universal => "✓ Universal (mesmo arquivo Win/Mac)",
        MacAdapter => "◈ Mac Adapter (roda via nossa adaptação)",
        NativePortRequired => "✗ Precisa port nativo (código Win / hook ausente)",
    }
}

fn print_report(r: &classify::ModReport) {
    use classify::FileKind;
    println!("MOD: {}", r.name);
    println!("Classe : {}", class_label(r.class));
    println!("Compat : {}", compat_label(r.compat));
    // contagem por tipo
    let count = |k: FileKind| r.files.iter().filter(|f| f.kind == k).count();
    println!(
        "Arquivos: {} total  (archive {}, content {}, .xl {}, .reds {}, .lua {}, tweak {}, nativo {})",
        r.files.len(),
        count(FileKind::Archive),
        count(FileKind::Content),
        count(FileKind::ArchiveXl),
        count(FileKind::Redscript),
        count(FileKind::CetLua),
        count(FileKind::Tweak),
        count(FileKind::Native),
    );
    if r.deps.is_empty() {
        println!("Dependências: nenhuma");
    } else {
        println!("Dependências:");
        for d in &r.deps {
            println!("  - {} — {}", d.name, d.detail);
        }
    }
    if r.risks.is_empty() {
        println!("Risco: nenhum (seguro p/ instalar pela ferramenta)");
    } else {
        println!("⚠ RISCO ({}) — revisar antes de instalar:", r.risks.len());
        for risk in &r.risks {
            println!("  ! {risk}");
        }
    }
    if !r.notes.is_empty() {
        println!("Notas:");
        for n in &r.notes {
            println!("  • {n}");
        }
    }
}
