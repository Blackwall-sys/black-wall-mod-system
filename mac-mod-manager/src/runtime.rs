//! runtime.rs — localiza as peças do runtime (scc, tweakdb-tool, input-loader) e ATIVA um mod
//! depois de instalar. É o que torna o `cp77-mods` um runtime UNIFICADO, não só um copiador:
//! "instalar um mod" realmente o LIGA (compila os .reds, sinaliza o tweak/input). UX Mac:
//! o runtime já está embutido → o usuário não cata milho juntando frameworks.

use bwms_core::classify::{FileKind, ModReport};
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

/// Ativa o mod após a cópia: roda o passo certo pra cada tipo presente. Steps que MODIFICAM
/// arquivos do jogo de forma sensível (tweakdb.bin) são REPORTADOS (não auto-rodados às cegas).
pub fn activate(report: &ModReport, game: &Path) -> Vec<StepResult> {
    let mut out = Vec::new();
    let has = |k: FileKind| report.files.iter().any(|f| f.kind == k);

    if has(FileKind::Redscript) {
        let scripts = game.join("r6/scripts");
        out.push(run("redscript", "scc", &["-compile", &scripts.to_string_lossy()],
            "compilou r6/scripts → r6/cache/final.redscripts"));
    }
    if has(FileKind::CetLua) {
        out.push(StepResult {
            step: "CET Lua".into(),
            ok: true,
            detail: "o Blackwall.sys carrega no boot (loadmods) — sem passo extra".into(),
        });
    }
    if has(FileKind::Tweak) {
        // "arrasta e funciona": aplica os tweaks JÁ instalados sobre o tweakdb.bin e instala.
        out.push(apply_tweaks(report, game));
    }
    if has(FileKind::Archive) || has(FileKind::Content) || has(FileKind::ArchiveXl) {
        out.push(StepResult {
            step: "conteúdo/archive".into(),
            ok: true,
            detail: "no Mac o jogo lê loose-files — já no lugar, sem reempacotar".into(),
        });
    }
    out
}

/// Ciclo TweakXL "arrasta e funciona": pega os tweaks (.yaml/.tweak/.toml) que o install JÁ
/// copiou para `r6/tweaks/<mod>/`, aplica-os ENCADEADOS sobre o `r6/cache/tweakdb.bin` do jogo
/// (cada arquivo sobre o resultado do anterior → vários mods empilham) e instala o resultado.
/// Segurança: backup VANILLA do tweakdb.bin 1x (em `.cp77-mods/tweakdb-orig/`, restaurável) e
/// escrita ATÔMICA (grava num temp no mesmo FS e renomeia — nunca deixa um bin meio-escrito).
/// Reporta falha REAL (base ausente / apply falhou) em vez de só imprimir a dica.
fn apply_tweaks(report: &ModReport, game: &Path) -> StepResult {
    let step = "TweakDB";
    let mk = |ok: bool, detail: String| StepResult { step: step.into(), ok, detail };

    let tool = match find_tool("tweakdb-tool") {
        Some(t) => t,
        None => return mk(false, "tweakdb-tool não encontrado — bundle/instale o runtime".into()),
    };

    // arquivos instalados deste mod (flat em r6/tweaks/<mod>/ — install usa só o file_name p/ tweak)
    let tweaks_dir = game.join("r6/tweaks").join(&report.name);
    let mut files: Vec<(PathBuf, bool)> = Vec::new(); // (arquivo, is_toml)
    if let Ok(rd) = std::fs::read_dir(&tweaks_dir) {
        for e in rd.flatten() {
            let p = e.path();
            match p.extension().and_then(|s| s.to_str()) {
                Some("yaml") | Some("yml") | Some("tweak") => files.push((p, false)),
                Some("toml") => files.push((p, true)),
                _ => {}
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return mk(false, format!("nenhum .yaml/.toml em {}", tweaks_dir.display()));
    }

    // base = o tweakdb.bin que o jogo lê. Ausente = falha real (não "instalado" silencioso).
    let live = game.join("r6/cache/tweakdb.bin");
    if !live.is_file() {
        return mk(false, "r6/cache/tweakdb.bin não existe — verifique a integridade do jogo (Steam › Verificar arquivos)".into());
    }

    // backup vanilla 1x (a 1ª vez que QUALQUER tweak é aplicado → guarda o original p/ restaurar)
    let bdir = game.join(".cp77-mods/tweakdb-orig");
    let backup = bdir.join("tweakdb.bin");
    if !backup.exists() {
        let _ = std::fs::create_dir_all(&bdir);
        if let Err(e) = std::fs::copy(&live, &backup) {
            return mk(false, format!("backup do tweakdb.bin falhou: {e}"));
        }
    }

    // encadeia: arquivo[0] sobre o bin vivo → tmp_a; arquivo[1] sobre tmp_a → tmp_b; ... (alterna
    // p/ nunca ler=escrever o mesmo path). A base original só é tocada no rename atômico final.
    let tmp_a = bdir.join(".patched_a.bin");
    let tmp_b = bdir.join(".patched_b.bin");
    let mut cur = live.clone();
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
                return mk(false, format!(
                    "{sub} falhou em {name}: {}",
                    String::from_utf8_lossy(&o.stderr).lines().next().unwrap_or("(sem detalhe)")
                ));
            }
            Err(e) => return mk(false, format!("erro ao rodar {sub}: {e}")),
        }
    }

    // instala atômico: copia o resultado p/ um temp NO MESMO FS e renomeia sobre o tweakdb.bin
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

    mk(true, format!(
        "{} tweak(s) aplicado(s) e instalado(s) em r6/cache/tweakdb.bin (backup vanilla em .cp77-mods/tweakdb-orig/)",
        files.len()
    ))
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
