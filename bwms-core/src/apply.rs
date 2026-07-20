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

#[allow(dead_code)] // usado pelos testes; o apply agora resolve por caminho game-relativo
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

/// Anda `root` recursivamente e empurra `(rel_prefix/<subpath>, src)` pros arquivos com as extensões
/// dadas. Preserva a estrutura sob `root` (o mod já namespaceia, ex.: `r6/tweaks/omaha/...`).
fn collect_under(root: &Path, rel_prefix: &str, exts: &[&str], out: &mut Vec<(String, PathBuf)>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(p),
                    Ok(ft) if ft.is_file() => {
                        let ok = p.extension().and_then(|s| s.to_str()).map(|x| exts.iter().any(|e| x.eq_ignore_ascii_case(e))).unwrap_or(false);
                        if ok {
                            if let Ok(rel) = p.strip_prefix(root) {
                                let rel = rel.to_string_lossy().replace('\\', "/");
                                out.push((format!("{rel_prefix}/{rel}"), p.clone()));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// **LogicalMod:** tudo que ESTE mod coloca no jogo, como `(caminho relativo ao game, origem)`.
/// Une os artefatos de uma "arma"/mod num conjunto ATÔMICO (liga/desliga junto):
///  - `.archive`            → `archive/Mac/content/` (Path A — malha/textura)
///  - `r6/tweaks/**/*.yaml` → `r6/tweaks/**`        (TweakXL — records/stats)
/// O `.archive.xl` (ArchiveXL runtime) é contado à parte (`xl_count`) — placement pendente do runtime.
fn placements_of(mod_dir: &Path, slug: &str) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let archs = archives_of(mod_dir);
    for arch in &archs {
        let stem = arch.file_stem().and_then(|s| s.to_str()).unwrap_or("a");
        let name = if archs.len() > 1 {
            format!("{PREFIX}{}__{}.archive", slug, slugify(stem))
        } else {
            format!("{PREFIX}{}.archive", slug)
        };
        out.push((format!("archive/Mac/content/{name}"), arch.clone()));
    }
    collect_under(&mod_dir.join("r6").join("tweaks"), "r6/tweaks", &["yaml", "yml", "tweak"], &mut out);
    collect_under(&mod_dir.join("r6").join("scripts"), "r6/scripts", &["reds"], &mut out);
    out
}

/// Acha os `.xl`/`.archive.xl` dentro da pasta de UM mod (recursivo raso) — mesmo padrão de
/// `archives_of`. A CONTAGEM (`xl_pending`) usa `.len()` do resultado; `write_reslink_table` faz
/// o parse de verdade.
fn xl_files_of(mod_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![mod_dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let p = e.path();
                match e.file_type() {
                    Ok(ft) if ft.is_dir() => stack.push(p),
                    Ok(ft) if ft.is_file() => {
                        let s = p.to_string_lossy().to_ascii_lowercase();
                        if s.ends_with(".archive.xl") || s.ends_with(".xl") {
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

/// **`axl-e2e-wire-modmanager` (2026-07-13):** parseia TODOS os `.xl` dos mods ativos, monta o
/// plano de apply combinado (`apply_xl::build_apply_plan`, já com direção scalar/sequência
/// correta + cadeias/ciclos resolvidos) e escreve `<red4ext>/bwms-reslink.txt` no formato que o
/// runtime espera (`selftest::reslink_file`, `<pedido>|<servido>` por linha). Antes disso, o
/// arquivo nunca era gerado automaticamente pelo `install` — o `.xl` só era CONTADO
/// (`xl_pending`), nunca processado. Devolve quantos pares foram escritos (0 = nenhum `.xl` ou
/// nenhum link/copy neles). Erros de parse de UM arquivo não derrubam os outros (best-effort,
/// mod malformado não trava o install inteiro).
fn write_reslink_table(game: &Path, xl_paths: &[PathBuf]) -> usize {
    let mut body = String::new();
    let mut pairs = 0usize;
    for p in xl_paths {
        let Ok(src) = std::fs::read_to_string(p) else { continue };
        let Ok(xl) = crate::xl::parse_xl(&src) else { continue };
        let plan = crate::apply_xl::build_apply_plan(&xl);
        if plan.redirects.is_empty() {
            continue;
        }
        // emit_reslink relê o próprio XlFile (não o plano já resolvido) pra manter o formato
        // texto de sempre; cadeias/ciclos já resolvidos em `redirects` não mudam o formato de
        // saída aqui (link/copy diretos), só a versão em memória usada por quem chama build_apply_plan.
        let emitted = crate::apply_xl::emit_reslink(&xl);
        for line in emitted.lines() {
            if !line.starts_with('#') {
                body.push_str(line);
                body.push('\n');
            }
        }
        pairs += plan.links + plan.copies;
    }
    if pairs == 0 {
        return 0;
    }
    let dest = game.join("red4ext").join("bwms-reslink.txt");
    if let Some(d) = dest.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let out = format!("# bwms-reslink (gerado automaticamente pelo install — {} par(es))\n{body}", pairs);
    let _ = std::fs::write(&dest, out);
    pairs
}

/// Junta a secção `factories:` de todos os `.xl` ativos e escreve `<red4ext>/bwms-factories.txt`
/// no formato que o runtime espera (`selftest::factory_file`, 1 path de `.csv` por linha). Espelha
/// `write_reslink_table`: best-effort (mod malformado não trava o resto), devolve quantos paths
/// foram escritos (0 = nenhum `.xl` com `factories:`). Fecha a metade offline do `axl-factories-apply`
/// — o runtime já consome; faltava o instalador GERAR o arquivo (antes só ficava em `unsupported`).
fn write_factory_table(game: &Path, xl_paths: &[PathBuf]) -> usize {
    let mut body = String::new();
    let mut count = 0usize;
    for p in xl_paths {
        let Ok(src) = std::fs::read_to_string(p) else { continue };
        let Ok(xl) = crate::xl::parse_xl(&src) else { continue };
        if xl.factories.is_empty() {
            continue;
        }
        let emitted = crate::apply_xl::emit_factory_table(&xl);
        for line in emitted.lines() {
            if !line.starts_with('#') && !line.trim().is_empty() {
                body.push_str(line);
                body.push('\n');
                count += 1;
            }
        }
    }
    if count == 0 {
        return 0;
    }
    let dest = game.join("red4ext").join("bwms-factories.txt");
    if let Some(d) = dest.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let out = format!("# bwms-factories (gerado automaticamente pelo install — {count} factory(s))\n{body}");
    let _ = std::fs::write(&dest, out);
    count
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
        .map(|s| {
            s.lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty())
                // compat: manifesto antigo tinha só o filename (relativo a content/); o novo é game-relativo.
                .map(|l| if l.contains('/') { l.to_string() } else { format!("archive/Mac/content/{l}") })
                .collect()
        })
        .unwrap_or_default()
}
fn write_applied(game: &Path, files: &[String]) -> std::io::Result<()> {
    let p = applied_path(game);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(p, files.join("\n"))
}

/// Relatório de um apply: quantos arquivos copiados/removidos e quantos `.archive.xl` ficaram
/// pendentes (ArchiveXL runtime, F2 — o instalador reporta pra ser honesto: "malha ok, factory pendente").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyReport {
    pub copied: usize,
    pub removed: usize,
    pub xl_pending: usize,
    /// Pares `resource.link`/`resource.copy` escritos em `<red4ext>/bwms-reslink.txt` (0 se
    /// nenhum `.xl` ativo tiver link/copy, ou nenhum `.xl` existir). Ver `write_reslink_table`.
    pub reslink_pairs: usize,
    /// Paths de `.csv` de factory escritos em `<red4ext>/bwms-factories.txt` (0 se nenhum `.xl`
    /// ativo tiver `factories:`). O runtime re-injeta via HookAfter LoadFactoryAsync. Ver
    /// `write_factory_table`.
    pub factory_paths: usize,
}

/// Sincroniza o jogo com os mods ATIVOS, tratando cada mod como um **LogicalMod** (os 3 artefatos
/// — .archive, r6/tweaks/.yaml, .archive.xl — ligam/desligam JUNTOS). Removal-safe: o manifesto
/// rastreia TUDO que colocamos (game-relativo); desativar/apagar remove os 3.
pub fn apply_report(game: &Path) -> std::io::Result<ApplyReport> {
    let states = reconcile(game);

    // alvo desejado: caminho-relativo-ao-game -> origem
    let mut desired: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut xl_pending = 0usize;
    let mut xl_paths: Vec<PathBuf> = Vec::new();
    for m in states.iter().filter(|m| m.active) {
        let mdir = staging_dir(game).join(&m.category).join(&m.name);
        for (rel, src) in placements_of(&mdir, &slugify(&m.name)) {
            desired.insert(rel, src);
        }
        let xls = xl_files_of(&mdir);
        xl_pending += xls.len();
        xl_paths.extend(xls);
    }

    // remove o que aplicamos antes e não é mais desejado (NUNCA toca o que não é nosso)
    let prev = read_applied(game);
    let mut removed = 0usize;
    for f in &prev {
        if !desired.contains_key(f) && std::fs::remove_file(game.join(f)).is_ok() {
            removed += 1;
        }
    }
    // copia os desejados (cria os dirs; sobrescreve = idempotente)
    let mut copied = 0usize;
    for (rel, src) in &desired {
        let dest = game.join(rel);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(src, &dest)?;
        copied += 1;
    }
    let applied_now: Vec<String> = desired.keys().cloned().collect();
    write_applied(game, &applied_now)?;
    let reslink_pairs = write_reslink_table(game, &xl_paths);
    let factory_paths = write_factory_table(game, &xl_paths);
    Ok(ApplyReport { copied, removed, xl_pending, reslink_pairs, factory_paths })
}

/// Compat: devolve `(copiados, removidos)`. Novo código deve usar `apply_report` (traz `xl_pending`).
pub fn apply(game: &Path) -> std::io::Result<(usize, usize)> {
    let r = apply_report(game)?;
    Ok((r.copied, r.removed))
}

// ============================ deploy da BIBLIOTECA nexus (modelo novo) ============================
// Camada 3 do MODELO-BIBLIOTECA-NEXUS.md: deploya os mods ENABLED da biblioteca (`nexus/mods/`) pras
// pastas de LOAD do jogo, registrando cada arquivo em `nexus/deploy.json` p/ purge REVERSÍVEL +
// detecção de conflito. Reusa o mesmo roteamento (`placements_of`) do apply por-tema. Diferença: a
// FONTE é a biblioteca (identidade Nexus + manifesto), não o staging por-tema, e o registro é
// por-arquivo com o mod de origem (não só um filename por linha).

/// Uma entrada do `deploy.json`: um arquivo copiado da biblioteca pro jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployEntry {
    pub unique_id: String, // mod de origem
    pub src: String,       // caminho relativo à pasta do mod na biblioteca
    pub dst: String,       // caminho relativo ao jogo (o que o jogo lê)
    pub sha1: String,      // hex do conteúdo (audit / detectar alteração externa)
}

/// Relatório de um deploy da biblioteca.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeployReport {
    pub deployed: usize,        // arquivos copiados pras pastas de load
    pub removed: usize,         // arquivos de um deploy anterior removidos (não mais desejados)
    pub mods: usize,            // mods ativos deployados
    pub conflicts: Vec<String>, // "dst: mod A vs mod B (B vence por ordem)"
}

fn deploy_path(game: &Path) -> PathBuf {
    crate::nexus::library_dir(game).join("deploy.json")
}

fn jesc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn hex20(b: &[u8; 20]) -> String {
    let mut s = String::with_capacity(40);
    for x in b {
        s.push_str(&format!("{x:02x}"));
    }
    s
}

/// Extrai o valor da chave `"key":"..."` de uma linha do nosso `deploy.json` (valores nunca têm
/// aspas — paths POSIX/uid/hex —, então ler até a próxima aspa é seguro).
fn jfield(line: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].replace("\\\\", "\\"))
}

fn read_deploy(game: &Path) -> Vec<DeployEntry> {
    let Ok(text) = std::fs::read_to_string(deploy_path(game)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        if !line.starts_with("{\"unique_id\"") {
            continue;
        }
        if let (Some(unique_id), Some(src), Some(dst), Some(sha1)) = (
            jfield(line, "unique_id"),
            jfield(line, "src"),
            jfield(line, "dst"),
            jfield(line, "sha1"),
        ) {
            out.push(DeployEntry { unique_id, src, dst, sha1 });
        }
    }
    out
}

fn write_deploy(game: &Path, entries: &[DeployEntry]) -> std::io::Result<()> {
    let path = deploy_path(game);
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let mut s = String::from("{\n\"schema\":1,\n\"deployed\":[\n");
    for (i, e) in entries.iter().enumerate() {
        s.push_str(&format!(
            "{{\"unique_id\":\"{}\",\"src\":\"{}\",\"dst\":\"{}\",\"sha1\":\"{}\"}}",
            jesc(&e.unique_id),
            jesc(&e.src),
            jesc(&e.dst),
            jesc(&e.sha1)
        ));
        s.push_str(if i + 1 < entries.len() { ",\n" } else { "\n" });
    }
    s.push_str("]\n}\n");
    std::fs::write(path, s)
}

/// Deploya TODOS os mods `enabled` da biblioteca nexus pras pastas de load do jogo. Registra cada
/// arquivo em `nexus/deploy.json` (purge reversível), remove os do deploy anterior que não são mais
/// desejados (removal-safe, nunca toca no que não é nosso), e detecta conflito (dois mods mirando o
/// mesmo `dst` — o de `unique_id` maior por ordem alfabética vence). Idempotente.
pub fn deploy_library(game: &Path) -> std::io::Result<DeployReport> {
    let mut rep = DeployReport::default();
    // dst (game-relativo) -> (unique_id, src_abs, src_rel-à-pasta-do-mod)
    let mut desired: BTreeMap<String, (String, PathBuf, String)> = BTreeMap::new();
    for uid in crate::nexus::list_library(game) {
        let Some(m) = crate::nexus::Manifest::read(game, &uid) else {
            continue;
        };
        if !m.enabled {
            continue;
        }
        rep.mods += 1;
        let mdir = crate::nexus::mod_dir(game, &uid);
        for (dst, src_abs) in placements_of(&mdir, &slugify(&uid)) {
            let src_rel = src_abs
                .strip_prefix(&mdir)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            if let Some((other, _, _)) = desired.get(&dst) {
                if other != &uid {
                    rep.conflicts
                        .push(format!("{dst}: {other} vs {uid} ({uid} vence por ordem)"));
                }
            }
            desired.insert(dst, (uid.clone(), src_abs, src_rel));
        }
    }

    // remove o deploy anterior que não é mais desejado
    let prev = read_deploy(game);
    for e in &prev {
        if !desired.contains_key(&e.dst) && std::fs::remove_file(game.join(&e.dst)).is_ok() {
            rep.removed += 1;
        }
    }

    // copia os desejados + monta o deploy novo
    let mut entries: Vec<DeployEntry> = Vec::new();
    for (dst, (uid, src_abs, src_rel)) in &desired {
        let dest = game.join(dst);
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::copy(src_abs, &dest)?;
        let sha1 = std::fs::read(&dest)
            .map(|d| hex20(&bwms_hashes::sha1(&d)))
            .unwrap_or_default();
        entries.push(DeployEntry {
            unique_id: uid.clone(),
            src: src_rel.clone(),
            dst: dst.clone(),
            sha1,
        });
        rep.deployed += 1;
    }
    write_deploy(game, &entries)?;
    Ok(rep)
}

/// Purga UM mod: remove do jogo exatamente os `dst` que ele deployou (lidos do `deploy.json`) e
/// reescreve o manifesto sem eles. Reversível e cirúrgico — não toca nos arquivos de outros mods
/// nem no jogo-base. Devolve quantos arquivos foram removidos. (Não apaga a pasta da biblioteca; use
/// `nexus::remove_from_library` p/ isso.)
pub fn purge_mod(game: &Path, unique_id: &str) -> std::io::Result<usize> {
    let prev = read_deploy(game);
    let mut removed = 0usize;
    let mut keep = Vec::with_capacity(prev.len());
    for e in prev {
        if e.unique_id == unique_id {
            if std::fs::remove_file(game.join(&e.dst)).is_ok() {
                removed += 1;
            }
        } else {
            keep.push(e);
        }
    }
    write_deploy(game, &keep)?;
    Ok(removed)
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

    #[test]
    fn arma_logicalmod_atomico() {
        // uma ARMA = .archive (malha) + r6/tweaks/*.yaml (stats) + .archive.xl (pendente). Liga/desliga JUNTOS.
        let g = std::env::temp_dir().join(format!("bwms-arma-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let base = g.join("BWMS/mods/armas/Omaha");
        touch(&base.join("archive/pc/mod/Omaha.archive"));
        touch(&base.join("archive/pc/mod/Omaha_Silencer.archive.xl"));
        touch(&base.join("r6/tweaks/omaha/Items.Preset_Omaha_Default.yaml"));
        touch(&base.join("r6/tweaks/omaha/Omaha_Silencer.yaml"));

        // apply: a arma inteira entra — malha no content + 2 yaml no r6/tweaks; 1 .xl pendente
        let r = apply_report(&g).unwrap();
        assert_eq!(r.copied, 3, "1 archive + 2 yaml");
        assert_eq!(r.xl_pending, 1, "o .archive.xl fica pendente do runtime");
        assert!(content_dir(&g).join("basegame_zzbwms_omaha.archive").exists());
        assert!(g.join("r6/tweaks/omaha/Items.Preset_Omaha_Default.yaml").exists());
        assert!(g.join("r6/tweaks/omaha/Omaha_Silencer.yaml").exists());

        // desativa a arma → os 3 artefatos (archive + 2 yaml) somem JUNTOS (atômico)
        let mut st = load_states(&g);
        for m in st.iter_mut() {
            m.active = false;
        }
        save_states(&g, &st).unwrap();
        let r = apply_report(&g).unwrap();
        assert_eq!(r.removed, 3, "malha + 2 yaml removidos juntos");
        assert!(!content_dir(&g).join("basegame_zzbwms_omaha.archive").exists());
        assert!(!g.join("r6/tweaks/omaha/Items.Preset_Omaha_Default.yaml").exists());

        let _ = std::fs::remove_dir_all(&g);
    }

    /// **`axl-e2e-wire-modmanager` (2026-07-13):** `install` (via `apply_report`) agora PARSEIA os
    /// `.xl` ativos de verdade e escreve `<red4ext>/bwms-reslink.txt` sozinho — antes só CONTAVA
    /// (`xl_pending`), nunca processava. Fecha "instalar pelo mod-manager" pro `resource.link`.
    #[test]
    fn xl_com_resource_link_gera_reslink_txt_automatico() {
        let g = std::env::temp_dir().join(format!("bwms-reslink-e2e-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let base = g.join("BWMS/mods/misc/LinkMod");
        touch(&base.join("archive/pc/mod/LinkMod.archive"));
        std::fs::write(
            base.join("resource.xl"),
            "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\b.mesh\n",
        )
        .unwrap();

        let r = apply_report(&g).unwrap();
        assert_eq!(r.xl_pending, 1);
        assert_eq!(r.reslink_pairs, 1, "1 resource.link no .xl -> 1 par no reslink");

        let reslink_path = g.join("red4ext").join("bwms-reslink.txt");
        assert!(reslink_path.exists(), "bwms-reslink.txt deveria ter sido gerado automaticamente");
        let content = std::fs::read_to_string(&reslink_path).unwrap();
        assert!(content.contains("base\\fake\\a.mesh"), "conteúdo: {content}");
        assert!(content.contains("base\\real\\b.mesh"), "conteúdo: {content}");

        let _ = std::fs::remove_dir_all(&g);
    }

    /// **`axl-factories-apply` metade offline (2026-07-16):** `install` (via `apply_report`) agora
    /// também PARSEIA a secção `factories:` dos `.xl` ativos e escreve `<red4ext>/bwms-factories.txt`
    /// sozinho — o formato que `selftest::factory_file` já consome. Fecha o lado do mod-manager pro
    /// uso #1 do ArchiveXL ("adicionar itens"); só o offset de LoadFactoryAsync fica pendente in-game.
    #[test]
    fn xl_com_factories_gera_factories_txt_automatico() {
        let g = std::env::temp_dir().join(format!("bwms-factory-e2e-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let base = g.join("BWMS/mods/misc/FactMod");
        touch(&base.join("archive/pc/mod/FactMod.archive"));
        std::fs::write(
            base.join("factory.xl"),
            "factories:\n  - mymod\\factories\\clothing.csv\n  - mymod\\factories\\weapons.csv\n",
        )
        .unwrap();

        let r = apply_report(&g).unwrap();
        assert_eq!(r.xl_pending, 1);
        assert_eq!(r.factory_paths, 2, "2 factories no .xl -> 2 linhas em bwms-factories.txt");

        let fac_path = g.join("red4ext").join("bwms-factories.txt");
        assert!(fac_path.exists(), "bwms-factories.txt deveria ter sido gerado automaticamente");
        let content = std::fs::read_to_string(&fac_path).unwrap();
        assert!(content.contains("mymod\\factories\\clothing.csv"), "conteúdo: {content}");
        assert!(content.contains("mymod\\factories\\weapons.csv"), "conteúdo: {content}");
        // 1 header comentado + 2 paths = 3 linhas; 2 não-comentário.
        let paths: Vec<_> = content.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect();
        assert_eq!(paths.len(), 2);

        let _ = std::fs::remove_dir_all(&g);
    }

    // ---- deploy da BIBLIOTECA nexus (modelo novo) ----

    /// Importa um mod fake pra biblioteca com um .archive + um .reds. O src fica DENTRO do game dir
    /// (único por teste) p/ não colidir entre testes que rodam em paralelo com o mesmo `name`.
    fn import_lib_mod(game: &Path, author: &str, name: &str) -> String {
        let src = game.join("_libsrc").join(name);
        let _ = std::fs::remove_dir_all(&src);
        touch(&src.join("archive/pc/mod/x.archive"));
        std::fs::create_dir_all(src.join("r6/scripts")).unwrap();
        std::fs::write(src.join("r6/scripts/mod.reds"), b"// reds").unwrap();
        let info = crate::nexus::ImportInfo {
            name: name.into(),
            author: author.into(),
            version: "1.0.0".into(),
            installed_at: "2026-07-16T00:00:00Z".into(),
            category: "misc".into(),
            ..Default::default()
        };
        let m = crate::nexus::import_from_dir(game, &src, &info).unwrap();
        let _ = std::fs::remove_dir_all(&src);
        m.unique_id
    }

    #[test]
    fn deploy_biblioteca_copia_registra_e_purga() {
        let g = std::env::temp_dir().join(format!("bwms-deploy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let uid = import_lib_mod(&g, "Autor", "MeuMod");

        let rep = deploy_library(&g).unwrap();
        assert_eq!(rep.mods, 1);
        assert_eq!(rep.deployed, 2, "1 archive + 1 reds");
        assert!(rep.conflicts.is_empty());
        // o .archive foi pro content com prefixo; o .reds foi pro r6/scripts
        assert!(g.join(format!("archive/Mac/content/{PREFIX}{}.archive", slugify(&uid))).exists());
        assert!(g.join("r6/scripts/mod.reds").exists());
        // deploy.json registrou os 2 arquivos com o mod de origem
        let entries = read_deploy(&g);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.unique_id == uid));
        assert!(entries.iter().all(|e| e.sha1.len() == 40));

        // purge: remove exatamente os dst desse mod, esvazia o deploy.json
        let removed = purge_mod(&g, &uid).unwrap();
        assert_eq!(removed, 2);
        assert!(!g.join("r6/scripts/mod.reds").exists());
        assert!(read_deploy(&g).is_empty());

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn deploy_detecta_conflito_no_mesmo_dst() {
        let g = std::env::temp_dir().join(format!("bwms-deploy-conf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        // 2 mods, AMBOS com r6/scripts/mod.reds → mesmo dst → conflito (o .archive tem nome único
        // por slug do uid, então só o .reds colide).
        import_lib_mod(&g, "Autor", "ModA");
        import_lib_mod(&g, "Autor", "ModB");
        let rep = deploy_library(&g).unwrap();
        assert_eq!(rep.mods, 2);
        assert!(!rep.conflicts.is_empty(), "esperava conflito no r6/scripts/mod.reds");
        assert!(rep.conflicts.iter().any(|c| c.contains("r6/scripts/mod.reds")));
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn deploy_respeita_enabled_e_redeploy_remove() {
        let g = std::env::temp_dir().join(format!("bwms-deploy-en-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let uid = import_lib_mod(&g, "Autor", "MeuMod");

        // desativa no manifesto → não deploya
        let mut m = crate::nexus::Manifest::read(&g, &uid).unwrap();
        m.enabled = false;
        m.write(&g).unwrap();
        let rep = deploy_library(&g).unwrap();
        assert_eq!(rep.mods, 0);
        assert_eq!(rep.deployed, 0);
        assert!(!g.join("r6/scripts/mod.reds").exists());

        // reativa → deploya
        m.enabled = true;
        m.write(&g).unwrap();
        assert_eq!(deploy_library(&g).unwrap().deployed, 2);
        assert!(g.join("r6/scripts/mod.reds").exists());

        // desativa de novo + re-deploy → REMOVE o que tinha aplicado (removal-safe)
        m.enabled = false;
        m.write(&g).unwrap();
        let rep = deploy_library(&g).unwrap();
        assert_eq!(rep.deployed, 0);
        assert_eq!(rep.removed, 2, "re-deploy remove os dst não mais desejados");
        assert!(!g.join("r6/scripts/mod.reds").exists());
        assert!(read_deploy(&g).is_empty());

        let _ = std::fs::remove_dir_all(&g);
    }
}
