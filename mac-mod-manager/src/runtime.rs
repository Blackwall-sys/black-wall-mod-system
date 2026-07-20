//! runtime.rs — localiza as peças do runtime (scc, tweakdb-tool, input-loader) e ATIVA um mod
//! depois de instalar. É o que torna o `cp77-mods` um runtime UNIFICADO, não só um copiador:
//! "instalar um mod" realmente o LIGA (compila os .reds, sinaliza o tweak/input). UX Mac:
//! o runtime já está embutido → o usuário não cata milho juntando frameworks.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Acha o binário de uma peça do runtime: bundle ao lado do cp77-mods, dirs de release do
/// projeto (mods-research), ou PATH. Retorna None se não achar.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let near = dir.join(name); // bundle ao lado
            if near.is_file() {
                return Some(near);
            }
        }
        // mods-research = .../mac-mod-manager/target/release/<exe> → 4 pais acima
        if let Some(root) = exe.ancestors().nth(4) {
            for c in [
                root.join(format!("{name}/target/release/{name}")),
                root.join(format!("dist/cp77-mac-tools/bin/{name}")),
                root.join(format!("redscript/target/release/{name}")),
                root.join(format!("dist/cp77-mac-tools/redscript-macos/engine/tools/{name}")),
            ] {
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    if Command::new(name).arg("--help").output().is_ok() {
        return Some(PathBuf::from(name));
    }
    None
}

pub struct StepResult {
    pub step: String,
    pub ok: bool,
    pub detail: String,
}


fn run(step: &str, tool: &str, args: &[&str], ok_detail: &str) -> StepResult {
    let bin = match find_tool(tool) {
        Some(b) => b,
        None => {
            return StepResult {
                step: step.into(),
                ok: false,
                detail: format!("{tool} não encontrado — bundle/instale o runtime"),
            }
        }
    };
    match Command::new(&bin).args(args).output() {
        Ok(o) if o.status.success() => StepResult { step: step.into(), ok: true, detail: ok_detail.into() },
        Ok(o) => StepResult {
            step: step.into(),
            ok: false,
            detail: format!(
                "{tool} falhou: {}",
                String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("(sem detalhe)")
            ),
        },
        Err(e) => StepResult { step: step.into(), ok: false, detail: format!("erro ao rodar {tool}: {e}") },
    }
}

// ============================================================================================
// ATIVAÇÃO UNIFICADA (game-wide) — o "ligar" do BWMS como 1 mod. Em vez de ativar por-mod (que
// não sabe do estado global), reconstrói o estado do jogo a partir dos ARTEFATOS ATIVOS que o
// `apply` (LogicalMod) já colocou: compila TODO r6/scripts + reconstrói o TweakDB de TODO
// r6/tweaks sobre o VANILLA. Idempotente e removal-safe (tirar um mod + reativar → some limpo).
// ============================================================================================

/// Existe algum arquivo com a extensão `ext` (recursivo) sob `root`?
fn has_ext(root: &Path, ext: &str) -> bool {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(p),
                    Ok(ft) if ft.is_file() => {
                        if p.extension().and_then(|s| s.to_str()).map(|x| x.eq_ignore_ascii_case(ext)).unwrap_or(false) {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    false
}

/// Compila os `.reds` de `r6/scripts` → `r6/cache/final.redscripts` — só se houver `.reds`
/// instalado (senão o boot usa o vanilla; não mexe à toa). None = nada a fazer.
pub fn compile_redscript(game: &Path) -> Option<StepResult> {
    let scripts = game.join("r6/scripts");
    if !has_ext(&scripts, "reds") {
        return None;
    }
    Some(run("redscript", "scc", &["-compile", &scripts.to_string_lossy()], "compilou r6/scripts → r6/cache/final.redscripts"))
}

/// Reconstrói o `tweakdb.bin` a partir do VANILLA + aplica TODOS os `r6/tweaks/**/*.{yaml,yml,tweak,toml}`
/// encadeados (o modelo real do TweakXL). Parte sempre do vanilla → idempotente e removal-safe
/// (sem tweaks ativos = restaura o vanilla). Recursivo (respeita o namespace do autor, ex.: `omaha/`).
pub fn apply_all_tweaks(game: &Path) -> StepResult {
    let step = "TweakDB";
    let mk = |ok: bool, detail: String| StepResult { step: step.into(), ok, detail };

    // coleta TODOS os tweaks (recursivo)
    let mut files: Vec<(PathBuf, bool)> = Vec::new();
    let mut stack = vec![game.join("r6/tweaks")];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(p),
                    Ok(ft) if ft.is_file() => match p.extension().and_then(|s| s.to_str()) {
                        Some("yaml") | Some("yml") | Some("tweak") => files.push((p, false)),
                        Some("toml") => files.push((p, true)),
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
    }
    files.sort();

    let live = game.join("r6/cache/tweakdb.bin");
    if !live.is_file() {
        return mk(false, "r6/cache/tweakdb.bin não existe — verifique a integridade do jogo (Steam › Verificar arquivos)".into());
    }
    let bdir = game.join(".cp77-mods/tweakdb-orig");
    let backup = bdir.join("tweakdb.bin");
    if !backup.exists() {
        let _ = std::fs::create_dir_all(&bdir);
        if let Err(e) = std::fs::copy(&live, &backup) {
            return mk(false, format!("backup do tweakdb.bin falhou: {e}"));
        }
    }
    // sem tweaks ativos → restaura o vanilla (removal-safe) e sai
    if files.is_empty() {
        return match std::fs::copy(&backup, &live) {
            Ok(_) => mk(true, "sem tweaks ativos → tweakdb.bin restaurado ao vanilla".into()),
            Err(e) => mk(false, format!("restaurar vanilla falhou: {e}")),
        };
    }
    let tool = match find_tool("tweakdb-tool") {
        Some(t) => t,
        None => return mk(false, "tweakdb-tool não encontrado — bundle/instale o runtime".into()),
    };
    // reconstrói a partir do VANILLA (nunca do vivo — senão empilha) + encadeia todos
    let tmp_a = bdir.join(".patched_a.bin");
    let tmp_b = bdir.join(".patched_b.bin");
    let mut cur = backup.clone();
    for (i, (file, is_toml)) in files.iter().enumerate() {
        let out_path = if i % 2 == 0 { &tmp_a } else { &tmp_b };
        let sub = if *is_toml { "apply-toml" } else { "apply-yaml" };
        let name = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
        match Command::new(&tool)
            .env("CP77_DIR", game)
            .args([sub, &*file.to_string_lossy(), &*cur.to_string_lossy(), "-o", &*out_path.to_string_lossy()])
            .output()
        {
            Ok(o) if o.status.success() => cur = out_path.clone(),
            Ok(o) => {
                let _ = std::fs::remove_file(&tmp_a);
                let _ = std::fs::remove_file(&tmp_b);
                return mk(false, format!("{sub} falhou em {name}: {}", String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("(sem detalhe)")));
            }
            Err(e) => return mk(false, format!("erro ao rodar {sub}: {e}")),
        }
    }
    // instala atômico (temp no mesmo FS + rename sobre o tweakdb.bin)
    let staged = bdir.join(".install.bin");
    if let Err(e) = std::fs::copy(&cur, &staged) {
        return mk(false, format!("preparando install do tweakdb.bin falhou: {e}"));
    }
    if let Err(e) = std::fs::rename(&staged, &live) {
        let _ = std::fs::remove_file(&staged);
        return mk(false, format!("instalar tweakdb.bin falhou: {e}"));
    }
    let _ = std::fs::remove_file(&tmp_a);
    let _ = std::fs::remove_file(&tmp_b);
    mk(true, format!("{} tweak(s) reconstruídos sobre o vanilla → tweakdb.bin", files.len()))
}

/// Ativação UNIFICADA do estado atual do jogo (após `apply`): redscript + TweakDB game-wide.
/// É o que faz `install`/`apply` serem 1 pipeline: colocar (apply) e LIGAR (aqui).
pub fn activate_all(game: &Path) -> Vec<StepResult> {
    let mut out = Vec::new();
    if let Some(s) = compile_redscript(game) {
        out.push(s);
    }
    if game.join("r6/tweaks").is_dir() {
        out.push(apply_all_tweaks(game));
    }
    out
}
