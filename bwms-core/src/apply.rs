//! `apply`: sincroniza os mods ATIVOS (staging por tema) → `archive/Mac/content` (Path A).
//!
//! Princípios: (1) NUNCA toca nos archives BASE — só mexe nos arquivos que ESTE módulo colocou
//! (rastreados num manifesto). (2) Staging = `<game>/BWMS/mods/<tema>/<mod>/` — pasta do USUÁRIO,
//! NUNCA vai no zip de release. (3) Liga/desliga = incluir/excluir do content no boot. Removal-safe:
//! desativar ou apagar a pasta + apply remove o archive do content; o jogo volta ao normal.

use crate::theme::{load_states, save_states, ModState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const STAGING: &str = "BWMS/mods"; // <game>/BWMS/mods/<tema-slug>/<mod>/   (USER-LOCAL)
const PREFIX: &str = "basegame_zzbwms_"; // casa o glob basegame_*.archive + ordena por último (override)
const APPLIED: &str = ".cp77-mods/bwms-applied.txt"; // 1 filename por linha = o que pusemos em content

fn content_dir(game: &Path) -> PathBuf {
    game.join("archive").join("Mac").join("content")
}
fn staging_dir(game: &Path) -> PathBuf {
    game.join(STAGING)
}

/// slug seguro p/ nome de arquivo: alfanumérico vira igual, resto vira '_', minúsculo, sem repetir '_'.
fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_us = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_us = false;
        } else if !last_us {
            out.push('_');
            last_us = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Acha os .archive dentro da pasta de UM mod (recursivo raso).
fn archives_of(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(p),
                    Ok(ft) if ft.is_file() => {
                        if p.extension().and_then(|s| s.to_str()).map(|e| e.eq_ignore_ascii_case("archive")).unwrap_or(false) {
                            out.push(p);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out.sort();
    out
}

/// Reconcilia o estado com o disco: varre o staging (a PASTA define a categoria), preserva os flags
/// (ativo/favorito/ordem) dos mods já conhecidos, adiciona os novos (ativo=true por padrão — quem
/// largou quer usar) e remove do estado os que sumiram da pasta. Salva e devolve a lista.
///
/// Varre TODA subpasta de `BWMS/mods/` — não só as categorias curadas. Uma pasta com nome
/// desconhecido (`BWMS/mods/minha-categoria/`) é uma categoria CUSTOM de pleno direito, preservada
/// como ela mesma (não colapsa em "outros"). Pastas iniciadas por '.' (ex.: .DS_Store que viesse
/// como dir) são ignoradas.
pub fn reconcile(game: &Path) -> Vec<ModState> {
    let mut states = load_states(game);
    let sroot = staging_dir(game);
    let mut on_disk: Vec<(String, String)> = Vec::new(); // (nome_do_mod, slug_da_categoria)
    if let Ok(cats) = std::fs::read_dir(&sroot) {
        for ce in cats.flatten() {
            if !ce.file_type().map(|f| f.is_dir()).unwrap_or(false) {
                continue;
            }
            let cat = ce.file_name().to_string_lossy().to_string();
            if cat.starts_with('.') {
                continue;
            }
            if let Ok(rd) = std::fs::read_dir(ce.path()) {
                for e in rd.flatten() {
                    if e.file_type().map(|f| f.is_dir()).unwrap_or(false) {
                        let name = e.file_name().to_string_lossy().to_string();
                        on_disk.push((name, cat.clone()));
                    }
                }
            }
        }
    }
    // remove do estado quem não está mais no disco
    states.retain(|m| on_disk.iter().any(|(n, _)| n == &m.name));
    // atualiza categoria dos conhecidos + adiciona novos
    for (name, cat) in &on_disk {
        if let Some(m) = states.iter_mut().find(|m| &m.name == name) {
            m.category = cat.clone();
        } else {
            let order = states.len() as i32;
            states.push(ModState { name: name.clone(), category: cat.clone(), active: true, favorite: false, order, variant: String::new() });
        }
    }
    let _ = save_states(game, &states);
    states
}

fn applied_path(game: &Path) -> PathBuf {
    game.join(APPLIED)
}
fn read_applied(game: &Path) -> Vec<String> {
    std::fs::read_to_string(applied_path(game))
        .map(|s| s.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}
fn write_applied(game: &Path, files: &[String]) -> std::io::Result<()> {
    let p = applied_path(game);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, files.join("\n"))
}

/// Sincroniza `archive/Mac/content` com os mods ATIVOS. Devolve (copiados, removidos).
pub fn apply(game: &Path) -> std::io::Result<(usize, usize)> {
    let states = reconcile(game);
    let content = content_dir(game);
    std::fs::create_dir_all(&content)?;

    // alvo desejado: filename-em-content -> caminho de origem
    let mut desired: BTreeMap<String, PathBuf> = BTreeMap::new();
    for m in states.iter().filter(|m| m.active) {
        let mdir = staging_dir(game).join(&m.category).join(&m.name);
        let archs = archives_of(&mdir);
        for arch in &archs {
            let stem = arch.file_stem().and_then(|s| s.to_str()).unwrap_or("a");
            // nome único e estável: prefixo + slug-do-mod + slug-do-archive (cobre packs c/ vários)
            let target = if archs.len() > 1 {
                format!("{PREFIX}{}__{}.archive", slugify(&m.name), slugify(stem))
            } else {
                format!("{PREFIX}{}.archive", slugify(&m.name))
            };
            desired.insert(target, arch.clone());
        }
    }

    // remove o que aplicamos antes e não é mais desejado (NUNCA toca o que não é nosso)
    let prev = read_applied(game);
    let mut removed = 0usize;
    for f in &prev {
        if !desired.contains_key(f) {
            if std::fs::remove_file(content.join(f)).is_ok() {
                removed += 1;
            }
        }
    }
    // copia os desejados (sobrescreve = idempotente)
    let mut copied = 0usize;
    for (target, src) in &desired {
        std::fs::copy(src, content.join(target))?;
        copied += 1;
    }
    let applied_now: Vec<String> = desired.keys().cloned().collect();
    write_applied(game, &applied_now)?;
    Ok((copied, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, b"FAKEARCHIVE").unwrap();
    }

    #[test]
    fn liga_desliga_sincroniza_content() {
        let g = std::env::temp_dir().join(format!("bwms-apply-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        // user larga 2 mods em pastas-tema
        touch(&g.join("BWMS/mods/veiculos/Caliburn Red/x.archive"));
        touch(&g.join("BWMS/mods/roupas/Trenchcoat/y.archive"));

        // 1) apply inicial: reconcile cria estado (ativo=true) e copia os 2
        let (c, r) = apply(&g).unwrap();
        assert_eq!((c, r), (2, 0));
        let content = content_dir(&g);
        assert!(content.join("basegame_zzbwms_caliburn_red.archive").exists());
        assert!(content.join("basegame_zzbwms_trenchcoat.archive").exists());

        // 2) desativa o Caliburn no estado e re-aplica → ele some do content, trench fica
        let mut st = load_states(&g);
        for m in st.iter_mut() {
            if m.name == "Caliburn Red" {
                m.active = false;
            }
        }
        save_states(&g, &st).unwrap();
        let (c, r) = apply(&g).unwrap();
        assert_eq!((c, r), (1, 1));
        assert!(!content.join("basegame_zzbwms_caliburn_red.archive").exists());
        assert!(content.join("basegame_zzbwms_trenchcoat.archive").exists());

        // 3) apaga a pasta do trench → reconcile tira do estado, apply remove do content
        std::fs::remove_dir_all(g.join("BWMS/mods/roupas/Trenchcoat")).unwrap();
        let (c, r) = apply(&g).unwrap();
        assert_eq!((c, r), (0, 1));
        assert!(!content.join("basegame_zzbwms_trenchcoat.archive").exists());

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn categoria_custom_e_reconhecida() {
        let g = std::env::temp_dir().join(format!("bwms-customcat-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        // pasta-categoria com nome FORA da tabela curada
        touch(&g.join("BWMS/mods/minha-categoria/Mod Doido/z.archive"));

        let states = reconcile(&g);
        assert_eq!(states.len(), 1);
        // a categoria é preservada como a pasta (NÃO colapsa em "outros")
        assert_eq!(states[0].category, "minha-categoria");
        assert_eq!(states[0].name, "Mod Doido");

        // e o apply copia normalmente pro content
        let (c, _r) = apply(&g).unwrap();
        assert_eq!(c, 1);
        assert!(content_dir(&g).join("basegame_zzbwms_mod_doido.archive").exists());

        let _ = std::fs::remove_dir_all(&g);
    }
}
