//! Abre um pacote de mod: pasta direta OU .zip (extrai via `unzip` do sistema = zero-dep).
//! A extração vai pra /tmp; se o zip tiver uma única subpasta no topo, usa ela como raiz real.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Resolve o argumento (.zip ou pasta) numa PASTA pronta pra classificar.
pub fn open_source(path: &Path) -> Result<PathBuf, String> {
    if path.is_dir() {
        return Ok(path.to_path_buf());
    }
    if !path.exists() {
        return Err(format!("'{}' não existe.", path.display()));
    }
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext != "zip" {
        return Err(format!("'{}' não é pasta nem .zip.", path.display()));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mod");
    let tmp = std::env::temp_dir().join(format!("cp77-mods-{stem}-{stamp}"));
    std::fs::create_dir_all(&tmp).map_err(|e| format!("criando tmp: {e}"))?;
    let st = Command::new("unzip")
        .arg("-q")
        .arg("-o")
        .arg(path)
        .arg("-d")
        .arg(&tmp)
        .status()
        .map_err(|e| format!("falha ao rodar unzip ({e}) — extraia o .zip manualmente e aponte a pasta"))?;
    if !st.success() {
        return Err("unzip retornou erro (zip corrompido?).".into());
    }
    Ok(single_subdir(&tmp).unwrap_or(tmp))
}

/// Se a pasta tem EXATAMENTE uma subpasta (ignorando dotfiles), retorna ela = raiz real do mod
/// (zips do Nexus costumam ter um wrapper `NomeDoMod/`).
fn single_subdir(dir: &Path) -> Option<PathBuf> {
    let entries: Vec<_> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    if entries.len() == 1 {
        let only = &entries[0];
        if only.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            return Some(only.path());
        }
    }
    None
}
