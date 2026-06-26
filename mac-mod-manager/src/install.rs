//! Instalação TRANSACIONAL e segura. A ferramenta decide o destino (lista fechada),
//! faz backup do que sobrescreve, grava um manifesto (pra remoção limpa) e dá rollback
//! se qualquer passo falhar. Nunca executa nada do pacote; arquivos nativos/scripts são
//! PULADOS (não instalados) e relatados.
//!
//! Destinos macOS (relativos ao game root). Onde o caminho Mac é incerto vs Windows, fica
//! marcado — a resolução fina (loose vs archive/pc/mod) é um adaptador separado.

use bwms_core::classify::{FileKind, ModReport};
use std::path::{Path, PathBuf};

/// Pasta de manifestos (estado do gerenciador) sob o game root.
const STATE_DIR: &str = ".cp77-mods";

/// Uma cópia planejada: de `src` (abs) pra `dest` (abs), sobrescrevendo ou não.
#[derive(Debug, Clone)]
pub struct Action {
    pub src: PathBuf,
    pub dest: PathBuf,
    pub overwrite: bool,
}

#[derive(Debug, Default)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub skipped: Vec<String>,   // nativo/script/desconhecido — não instala
    pub conflicts: Vec<String>, // dest já existe (será feito backup)
}

/// Subdiretório de destino p/ um tipo de arquivo. None = NÃO instalável (pular).
/// CET Lua → mods do Blackwall.sys; redscript → r6/scripts; tweak → r6/tweaks;
/// archive/.xl/content → archive/pc/mod (REDmod-style; no Mac pode virar loose — adaptador).
fn dest_subdir(kind: FileKind, mod_name: &str) -> Option<PathBuf> {
    let p = match kind {
        FileKind::CetLua => format!("red4ext/blackwall-mods/{mod_name}"),
        FileKind::Redscript => format!("r6/scripts/{mod_name}"),
        FileKind::Tweak => format!("r6/tweaks/{mod_name}"),
        FileKind::Archive | FileKind::ArchiveXl | FileKind::Content => "archive/pc/mod".into(),
        FileKind::RedModInfo => format!("mods/{mod_name}"), // REDmod
        FileKind::Native | FileKind::InstallScript | FileKind::Other => return None,
    };
    Some(PathBuf::from(p))
}

/// Monta o plano: para cada arquivo, resolve o destino (ou pula). Detecta conflitos.
/// NÃO toca em disco. `pkg_root` = pasta extraída do mod; `game` = raiz do jogo.
pub fn build_plan(report: &ModReport, pkg_root: &Path, game: &Path, mod_name: &str) -> Plan {
    let mut plan = Plan::default();
    if !report.risks.is_empty() {
        // segurança: com risco (path-traversal/script/nativo), não monta plano automático
        plan.skipped.push(format!(
            "BLOQUEADO: {} risco(s) — resolva/revise antes (veja `classify`)",
            report.risks.len()
        ));
        return plan;
    }
    for f in &report.files {
        let sub = match dest_subdir(f.kind, mod_name) {
            Some(s) => s,
            None => {
                plan.skipped.push(format!("{} (tipo não-instalável: {:?})", f.rel.display(), f.kind));
                continue;
            }
        };
        // preserva o nome do arquivo (e, p/ scripts/lua, a sub-hierarquia do mod)
        let leaf = leaf_path(&f.rel, f.kind);
        let dest = game.join(&sub).join(&leaf);
        let overwrite = dest.exists();
        if overwrite {
            plan.conflicts.push(dest.strip_prefix(game).unwrap_or(&dest).display().to_string());
        }
        plan.actions.push(Action { src: pkg_root.join(&f.rel), dest, overwrite });
    }
    plan
}

/// Para CET/redscript, preserva a hierarquia interna do mod (subpastas). Pra archive/content,
/// usa só o nome (vão todos pra archive/pc/mod). Evita colisão e mantém a estrutura do mod.
fn leaf_path(rel: &Path, kind: FileKind) -> PathBuf {
    match kind {
        FileKind::CetLua | FileKind::Redscript => {
            // tira o 1º componente (nome da pasta do mod no pacote) se houver, preserva o resto
            let comps: Vec<_> = rel.components().collect();
            if comps.len() > 1 {
                comps[1..].iter().collect()
            } else {
                rel.to_path_buf()
            }
        }
        _ => PathBuf::from(rel.file_name().unwrap_or(rel.as_os_str())),
    }
}

#[derive(Debug, Default)]
pub struct Manifest {
    pub mod_name: String,
    pub installed: Vec<String>,        // dests (rel ao game) criados
    pub backups: Vec<(String, String)>, // (dest rel, backup rel) dos sobrescritos
}

impl Manifest {
    /// Serializa em JSON simples (zero-dep).
    fn to_json(&self) -> String {
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let arr = |v: &[String]| {
            let items: Vec<String> = v.iter().map(|s| format!("\"{}\"", esc(s))).collect();
            format!("[{}]", items.join(","))
        };
        let backs: Vec<String> = self
            .backups
            .iter()
            .map(|(a, b)| format!("[\"{}\",\"{}\"]", esc(a), esc(b)))
            .collect();
        format!(
            "{{\"mod\":\"{}\",\"installed\":{},\"backups\":[{}]}}",
            esc(&self.mod_name),
            arr(&self.installed),
            backs.join(",")
        )
    }
}

/// Executa o plano transacionalmente: backup → cópia → manifesto. Em erro, rollback total.
/// Retorna o caminho do manifesto gravado, ou um erro (já tendo revertido).
pub fn install(plan: &Plan, game: &Path, mod_name: &str) -> Result<PathBuf, String> {
    if plan.actions.is_empty() {
        return Err("nada a instalar (plano vazio — veja skipped/risco).".into());
    }
    let state = game.join(STATE_DIR);
    let backup_dir = state.join("backups").join(mod_name);
    std::fs::create_dir_all(&backup_dir).map_err(|e| format!("criando backups: {e}"))?;
    let mut manifest = Manifest { mod_name: mod_name.to_string(), ..Default::default() };
    // lista de desfazimento (na ordem inversa) p/ rollback
    let mut done: Vec<Action> = Vec::new();

    for a in &plan.actions {
        // backup do existente
        if a.overwrite {
            let rel = a.dest.strip_prefix(game).unwrap_or(&a.dest);
            let bkp = backup_dir.join(rel);
            if let Some(parent) = bkp.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::copy(&a.dest, &bkp) {
                rollback(&done, game, mod_name);
                return Err(format!("backup de {} falhou: {e}", a.dest.display()));
            }
            manifest.backups.push((
                rel.display().to_string(),
                bkp.strip_prefix(game).unwrap_or(&bkp).display().to_string(),
            ));
        }
        // cópia
        if let Some(parent) = a.dest.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                rollback(&done, game, mod_name);
                return Err(format!("criando {}: {e}", parent.display()));
            }
        }
        if let Err(e) = std::fs::copy(&a.src, &a.dest) {
            rollback(&done, game, mod_name);
            return Err(format!("copiando p/ {} falhou: {e}", a.dest.display()));
        }
        manifest.installed.push(a.dest.strip_prefix(game).unwrap_or(&a.dest).display().to_string());
        done.push(a.clone());
    }

    // grava manifesto (commit)
    let man_dir = state.join("manifests");
    std::fs::create_dir_all(&man_dir).map_err(|e| format!("criando manifests: {e}"))?;
    let man_path = man_dir.join(format!("{mod_name}.json"));
    std::fs::write(&man_path, manifest.to_json()).map_err(|e| format!("gravando manifesto: {e}"))?;
    Ok(man_path)
}

/// Um mod instalado (lido do manifesto) — pro `list` e a TUI.
pub struct ModInfo {
    pub name: String,
    pub files: usize,
}

/// Lista os mods instalados por esta ferramenta (lê os manifestos JSON).
pub fn list_installed(game: &Path) -> Vec<ModInfo> {
    let man_dir = game.join(STATE_DIR).join("manifests");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&man_dir) {
        for e in rd.flatten() {
            if e.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(j) = std::fs::read_to_string(e.path()) {
                let name = e.path().file_stem().and_then(|s| s.to_str()).unwrap_or("?").to_string();
                out.push(ModInfo { name, files: json_str_array(&j, "installed").len() });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Remove um mod: apaga os instalados, restaura os backups, apaga o manifesto.
pub fn remove_mod(game: &Path, name: &str) -> Result<(), String> {
    let man_path = game.join(STATE_DIR).join("manifests").join(format!("{name}.json"));
    let json = std::fs::read_to_string(&man_path).map_err(|_| format!("mod '{name}' não consta instalado"))?;
    for rel in json_str_array(&json, "installed") {
        let _ = std::fs::remove_file(game.join(&rel));
    }
    for (dest, bkp) in json_str_array_pairs(&json, "backups") {
        let _ = std::fs::copy(game.join(&bkp), game.join(&dest));
    }
    let _ = std::fs::remove_file(&man_path);
    Ok(())
}

/// Extrai um array de strings JSON `"chave":[ "a","b" ]` (parser mínimo do nosso formato).
pub fn json_str_array(json: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let pat = format!("\"{key}\":[");
    let Some(start) = json.find(&pat) else { return out };
    let body = &json[start + pat.len()..];
    let end = body.find(']').unwrap_or(body.len());
    let mut s = &body[..end];
    while let Some(q1) = s.find('"') {
        let after = &s[q1 + 1..];
        if let Some(q2) = after.find('"') {
            out.push(after[..q2].replace("\\\"", "\"").replace("\\\\", "\\"));
            s = &after[q2 + 1..];
        } else {
            break;
        }
    }
    out
}

/// Extrai pares `"backups":[["a","b"],...]` como (a,b).
pub fn json_str_array_pairs(json: &str, key: &str) -> Vec<(String, String)> {
    let mut flat = Vec::new();
    let pat = format!("\"{key}\":[");
    if let Some(start) = json.find(&pat) {
        let body = &json[start + pat.len()..];
        let end = body.find("]]").map(|i| i + 1).unwrap_or(body.len());
        let mut s = &body[..end.min(body.len())];
        while let Some(q1) = s.find('"') {
            let after = &s[q1 + 1..];
            if let Some(q2) = after.find('"') {
                flat.push(after[..q2].replace("\\\"", "\"").replace("\\\\", "\\"));
                s = &after[q2 + 1..];
            } else {
                break;
            }
        }
    }
    let mut out = Vec::new();
    let mut it = flat.into_iter();
    while let (Some(a), Some(b)) = (it.next(), it.next()) {
        out.push((a, b));
    }
    out
}

/// Desfaz uma instalação parcial: apaga os novos, restaura os backups.
fn rollback(done: &[Action], game: &Path, mod_name: &str) {
    let backup_dir = game.join(STATE_DIR).join("backups").join(mod_name);
    for a in done.iter().rev() {
        let _ = std::fs::remove_file(&a.dest);
        if a.overwrite {
            let rel = a.dest.strip_prefix(game).unwrap_or(&a.dest);
            let bkp = backup_dir.join(rel);
            if bkp.exists() {
                let _ = std::fs::copy(&bkp, &a.dest);
            }
        }
    }
}
