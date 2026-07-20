//! cp77-mods — instalador/validador SEGURO de mods Cyberpunk 2077 no macOS.
//!
//! Princípio (design do dono): o pacote DESCREVE, a ferramenta DECIDE e EXECUTA. NUNCA roda
//! comandos do autor do mod; instala só em destinos de uma lista fechada; transacional com
//! backup + rollback. Zero-dep (só std).
//!
//! v0.1: `classify <pasta>` — inspeciona e relata (classe/compat/deps/risco). Instalação
//! transacional vem nos próximos comandos (plan/install/list/remove), reusando este relatório.

mod runtime;
mod source;
mod tui;

// Núcleo (classify/theme/apply) agora vem da lib compartilhada `bwms-core`,
// não mais de módulos locais — a CLI `bwms` usa a MESMA implementação.
use bwms_core::{apply, classify, nexus, theme};

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

  Biblioteca nexus (modelo novo — 1 pasta por mod + metadata do Nexus):
    cp77-mods nexus import <pasta|.zip> [nxm://…|modId] [game]   guarda o mod na biblioteca + manifesto
    cp77-mods nexus list                              [game]     lista a biblioteca (id/versão/estado)
    cp77-mods nexus deploy                            [game]     aplica os mods ativos (deploy.json reversível)
    cp77-mods nexus remove <unique_id>                [game]     purga do jogo + remove da biblioteca

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
        Some("nexus") => cmd_nexus(&args),
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
    println!("MOD: {mod_name}");
    for n in &report.notes {
        println!("  · {n}");
    }
    if !report.risks.is_empty() {
        eprintln!("⚠ instalação BLOQUEADA por {} risco(s) — veja `classify`.", report.risks.len());
        return ExitCode::from(3);
    }
    if !do_install {
        println!("(dry-run) `install` faria: staging em BWMS/mods/instalados/{mod_name}/ → apply (LogicalMod) → ativar (redscript+TweakDB).");
        return ExitCode::SUCCESS;
    }
    // INSTALL = 1 PIPELINE (a Unificação): stage → apply (LogicalMod, Mac-correto) → activate.
    // Mesmo caminho do `apply` — instalar é só "colocar no staging + sincronizar + ligar".
    let staged = match stage_mod(&dir, &game, &mod_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ staging falhou: {e}");
            return ExitCode::from(1);
        }
    };
    println!("✓ staged: {}", staged.strip_prefix(&game).unwrap_or(&staged).display());
    match apply::apply_report(&game) {
        Ok(rep) => {
            print!("✓ aplicado: {} arquivo(s), {} removido(s)", rep.copied, rep.removed);
            if rep.xl_pending > 0 {
                print!(", {} .xl pendente(s) do runtime ArchiveXL", rep.xl_pending);
            }
            if rep.reslink_pairs > 0 {
                print!(" ({} par(es) resource.link/copy escritos em red4ext/bwms-reslink.txt)", rep.reslink_pairs);
            }
            println!(".");
            for s in &runtime::activate_all(&game) {
                println!("  {} {} — {}", if s.ok { "✓" } else { "✗" }, s.step, s.detail);
            }
            println!("  (mods de archive aplicam ao REINICIAR o jogo)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ apply falhou: {e}");
            ExitCode::from(1)
        }
    }
}

/// Copia a árvore do mod pro staging `BWMS/mods/instalados/<name>/` (modelo gerenciado/reversível).
/// Reinstalar limpa o anterior. Só o que o apply consome (archive/, r6/) importa; o resto é inócuo.
fn stage_mod(src: &Path, game: &Path, name: &str) -> std::io::Result<PathBuf> {
    let dest = game.join("BWMS/mods/instalados").join(name);
    let _ = std::fs::remove_dir_all(&dest);
    copy_tree(src, &dest)?;
    Ok(dest)
}

fn copy_tree(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let from = e.path();
        let to = dest.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

// ---- Núcleo do PIPELINE UNIFICADO (compartilhado por CLI e TUI = 1 caminho só, 0 duplicação) ----

/// Instala pelo pipeline unificado: stage → apply (LogicalMod) → activate (redscript+TweakDB).
pub(crate) fn install_staged(dir: &Path, game: &Path, name: &str) -> Result<String, String> {
    stage_mod(dir, game, name).map_err(|e| format!("staging: {e}"))?;
    let rep = apply::apply_report(game).map_err(|e| format!("apply: {e}"))?;
    let ok = runtime::activate_all(game).iter().filter(|s| s.ok).count();
    let xl = if rep.xl_pending > 0 { format!(", {} .xl pendente(s)", rep.xl_pending) } else { String::new() };
    Ok(format!("{name}: {} arquivo(s){xl}, {ok} ativação(ões)", rep.copied))
}

/// Remove um mod do staging (por nome) + sincroniza (removal-safe). Devolve o resumo.
pub(crate) fn remove_staged(game: &Path, name: &str) -> String {
    let cat = match apply::reconcile(game).iter().find(|m| m.name.eq_ignore_ascii_case(name)) {
        Some(m) => m.category.clone(),
        None => return format!("'{name}' não está no staging"),
    };
    let dir = game.join("BWMS/mods").join(&cat).join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        return format!("erro ao apagar {}: {e}", dir.display());
    }
    match apply::apply_report(game) {
        Ok(rep) => {
            let _ = runtime::activate_all(game);
            format!("removido: {name} ({} arquivo(s) tirados)", rep.removed)
        }
        Err(e) => format!("erro ao sincronizar: {e}"),
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
    let states = apply::reconcile(&game);
    if states.is_empty() {
        println!("(nenhum mod no staging — use `install <pasta>` ou largue em BWMS/mods/<tema>/<Nome>/)");
        return ExitCode::SUCCESS;
    }
    println!("MODS ({} no staging):", states.len());
    for m in &states {
        println!(
            "  [{}]{} {} — {}",
            if m.active { "x" } else { " " },
            if m.favorite { "★" } else { " " },
            m.name,
            theme::category_label(&m.category)
        );
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
    // acha o mod no staging por nome, apaga a pasta dele, e apply+activate sincronizam a remoção
    let states = apply::reconcile(&game);
    let cat = match states.iter().find(|m| m.name.eq_ignore_ascii_case(name)) {
        Some(m) => m.category.clone(),
        None => {
            eprintln!("erro: '{name}' não está no staging (veja `list`).");
            return ExitCode::from(1);
        }
    };
    let dir = game.join("BWMS/mods").join(&cat).join(name);
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        eprintln!("erro ao apagar {}: {e}", dir.display());
        return ExitCode::from(1);
    }
    match apply::apply_report(&game) {
        Ok(rep) => {
            println!("✓ '{name}' removido ({} arquivo(s) tirados do jogo, removal-safe).", rep.removed);
            for s in &runtime::activate_all(&game) {
                println!("  {} {} — {}", if s.ok { "✓" } else { "✗" }, s.step, s.detail);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("erro ao sincronizar remoção: {e}");
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
    match apply::apply_report(&game) {
        Ok(rep) => {
            println!("✓ sincronizado: {} arquivo(s) ativo(s) (archive/Mac/content + r6/tweaks + r6/scripts), {} removido(s).", rep.copied, rep.removed);
            if rep.xl_pending > 0 {
                println!("  ⚠ {} .archive.xl pendente(s) do runtime ArchiveXL (factory/localization ainda não aplicam)", rep.xl_pending);
            }
            // LIGAR (ativação game-wide): compila redscript + reconstrói o TweakDB de todo r6/tweaks
            for s in &runtime::activate_all(&game) {
                println!("  {} {} — {}", if s.ok { "✓" } else { "✗" }, s.step, s.detail);
            }
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

// ============================ comandos da biblioteca nexus ============================

/// Data ISO (YYYY-MM-DD) de hoje, UTC — algoritmo civil-from-days (Howard Hinnant), sem deps.
fn today_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let z = (secs / 86400) as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Interpreta o 2º arg do import: um link `nxm://…` (mod_id+file_id) OU um modId cru → (mod_id, file_id).
fn parse_id_arg(arg: Option<&String>) -> (Option<u64>, Option<u64>) {
    match arg.map(String::as_str) {
        Some(s) if s.starts_with("nxm://") => match nexus::parse_nxm(s) {
            Some((_, mid, fid)) => (Some(mid), Some(fid)),
            None => (None, None),
        },
        Some(s) => (s.parse::<u64>().ok(), None),
        None => (None, None),
    }
}

fn cmd_nexus(args: &[String]) -> ExitCode {
    match args.get(2).map(String::as_str) {
        Some("import") => cmd_nexus_import(args.get(3), args.get(4), args.get(5)),
        Some("list") => cmd_nexus_list(game_root(args.get(3))),
        Some("deploy") => cmd_nexus_deploy(game_root(args.get(3))),
        Some("remove") => cmd_nexus_remove(args.get(3), game_root(args.get(4))),
        _ => {
            eprintln!("uso: cp77-mods nexus <import|list|deploy|remove> …  (veja --help)");
            ExitCode::from(2)
        }
    }
}

fn cmd_nexus_import(
    src_arg: Option<&String>,
    id_arg: Option<&String>,
    game_arg: Option<&String>,
) -> ExitCode {
    let Some(src_arg) = src_arg else {
        eprintln!("uso: cp77-mods nexus import <pasta|.zip> [nxm://…|modId] [game]");
        return ExitCode::from(2);
    };
    let input = PathBuf::from(src_arg);
    let dir = match source::open_source(&input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("erro: {e}");
            return ExitCode::from(2);
        }
    };
    let Some(game) = game_root(game_arg) else {
        eprintln!("erro: raiz do jogo não encontrada (passe [game] ou defina CP77_DIR).");
        return ExitCode::from(2);
    };

    // gate de segurança: mesma régua do install — risco duro barra a importação.
    let report = classify::classify(&dir);
    if !report.risks.is_empty() {
        eprintln!("⚠ importação BLOQUEADA por {} risco(s):", report.risks.len());
        for r in &report.risks {
            eprintln!("  ! {r}");
        }
        return ExitCode::from(3);
    }

    let install_file = input
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "mod".into());
    let (name, version) = nexus::infer_name_version(&install_file);
    let (mod_id, file_id) = parse_id_arg(id_arg);

    let info = nexus::ImportInfo {
        name,
        author: "Unknown".into(), // metadata do autor vem depois (API do Nexus com modId)
        version,
        mod_id,
        file_id,
        installation_file: install_file,
        installed_at: today_iso(),
        category: theme::suggest(&report).category,
    };
    match nexus::import_from_dir(&game, &dir, &info) {
        Ok(m) => {
            println!("✓ importado pra biblioteca: {}", m.unique_id);
            println!(
                "  versão: {}  ·  repo: {}",
                if m.version.is_empty() { "?" } else { &m.version },
                m.repository
            );
            if let Some((mid, fid)) = m.nexus_key() {
                println!("  nexus: mods/{mid}/files/{fid}");
            }
            println!("  pasta: BWMS/nexus/mods/{}/", m.unique_id);
            println!("  próximo: `cp77-mods nexus deploy` p/ aplicar no jogo.");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ import falhou: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_nexus_list(game: Option<PathBuf>) -> ExitCode {
    let Some(game) = game else {
        eprintln!("erro: raiz do jogo não encontrada.");
        return ExitCode::from(2);
    };
    let ids = nexus::list_library(&game);
    if ids.is_empty() {
        println!("(biblioteca vazia — use `cp77-mods nexus import <pasta|.zip>`)");
        return ExitCode::SUCCESS;
    }
    println!("Biblioteca ({} mod(s)):", ids.len());
    for id in &ids {
        let m = nexus::Manifest::read(&game, id).unwrap_or_default();
        let estado = if m.enabled { "ativo" } else { "inativo" };
        let key = m
            .nexus_key()
            .map(|(mid, fid)| format!("mods/{mid}/files/{fid}"))
            .unwrap_or_else(|| "manual".into());
        println!(
            "  {} {}  v{}  [{}]  ({})",
            if m.enabled { "●" } else { "○" },
            id,
            if m.version.is_empty() { "?" } else { &m.version },
            estado,
            key
        );
    }
    ExitCode::SUCCESS
}

fn cmd_nexus_deploy(game: Option<PathBuf>) -> ExitCode {
    let Some(game) = game else {
        eprintln!("erro: raiz do jogo não encontrada.");
        return ExitCode::from(2);
    };
    match apply::deploy_library(&game) {
        Ok(rep) => {
            println!(
                "✓ deploy: {} arquivo(s) de {} mod(s) ativo(s); {} removido(s) de deploy anterior.",
                rep.deployed, rep.mods, rep.removed
            );
            if !rep.conflicts.is_empty() {
                println!("⚠ {} conflito(s) (dois mods no mesmo arquivo):", rep.conflicts.len());
                for c in &rep.conflicts {
                    println!("  ! {c}");
                }
            }
            println!("  (mods de .archive só valem ao REINICIAR o jogo)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ deploy falhou: {e}");
            ExitCode::from(1)
        }
    }
}

fn cmd_nexus_remove(id_arg: Option<&String>, game: Option<PathBuf>) -> ExitCode {
    let Some(id) = id_arg else {
        eprintln!("uso: cp77-mods nexus remove <unique_id> [game]");
        return ExitCode::from(2);
    };
    let Some(game) = game else {
        eprintln!("erro: raiz do jogo não encontrada.");
        return ExitCode::from(2);
    };
    let purged = apply::purge_mod(&game, id).unwrap_or(0);
    let gone = nexus::remove_from_library(&game, id);
    if !gone && purged == 0 {
        eprintln!("mod '{id}' não estava na biblioteca.");
        return ExitCode::from(1);
    }
    println!("✓ removido '{id}': {purged} arquivo(s) purgado(s) do jogo + pasta da biblioteca apagada.");
    ExitCode::SUCCESS
}
