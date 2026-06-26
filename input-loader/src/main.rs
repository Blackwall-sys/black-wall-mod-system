//! input-loader — merge de keybinds do Cyberpunk 2077 no macOS, 100% Rust nativo.
//!
//! Reimplementa o cyberpunk2077-input-loader (RED4ext, Windows) como merge
//! offline. No macOS não há RED4ext rodando pra honrar um `input_loader.ini`;
//! então mesclamos os mods DIRETO nos arquivos loose que o engine lê em
//! `r6/config/` (com backup do pristino, reversível e idempotente).
//!
//! Mods põem suas binds em `r6/input/*.xml` (cada um um `<bindings>` com filhos
//! por tipo de node). Roteamos por tag:
//!   inputUserMappings.xml ← mapping, buttonGroup, pairedAxes, preset
//!   inputContexts.xml     ← blend, context, hold, multitap, repeat, toggle,
//!                           acceptedEvents
//!
//! Comandos: `merge` · `restore` · `status`  (`--game <dir>` ou CP77_DIR).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const MAPPING_TAGS: &[&str] = &["mapping", "buttonGroup", "pairedAxes", "preset"];
const CONTEXT_TAGS: &[&str] =
    &["blend", "context", "hold", "multitap", "repeat", "toggle", "acceptedEvents"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("erro: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let mut cmd = None;
    let mut game: Option<PathBuf> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--game" => game = Some(PathBuf::from(it.next().ok_or("--game exige <dir>")?)),
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other if cmd.is_none() => cmd = Some(other.to_string()),
            other => return Err(format!("argumento inesperado: '{other}'")),
        }
    }
    let game = game
        .or_else(|| std::env::var_os("CP77_DIR").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                PathBuf::from(h)
                    .join("Library/Application Support/Steam/steamapps/common/Cyberpunk 2077")
            })
        })
        .unwrap_or_else(|| PathBuf::from("."));

    match cmd.as_deref() {
        Some("merge") => cmd_merge(&game),
        Some("restore") => cmd_restore(&game),
        Some("status") => cmd_status(&game),
        None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(o) => Err(format!("comando desconhecido '{o}'. Use --help.")),
    }
}

const USAGE: &str = "\
input-loader — merge de keybinds (.xml) do Cyberpunk 2077 no macOS

USO:
    input-loader merge   [--game <dir>]   mescla r6/input/*.xml nos config base
    input-loader restore [--game <dir>]   volta os config ao pristino
    input-loader status  [--game <dir>]   mostra estado + mods detectados

Mods vão em <game>/r6/input/*.xml. O merge é idempotente (sempre parte do
pristino .orig) e reversível. Sem <dir>, usa CP77_DIR ou o caminho padrão.
";

struct Target {
    base: PathBuf,
    orig: PathBuf,
    tags: &'static [&'static str],
    label: &'static str,
}

fn targets(game: &Path) -> [Target; 2] {
    let cfg = game.join("r6/config");
    [
        Target {
            base: cfg.join("inputUserMappings.xml"),
            orig: cfg.join("inputUserMappings.xml.input-loader-orig"),
            tags: MAPPING_TAGS,
            label: "inputUserMappings",
        },
        Target {
            base: cfg.join("inputContexts.xml"),
            orig: cfg.join("inputContexts.xml.input-loader-orig"),
            tags: CONTEXT_TAGS,
            label: "inputContexts",
        },
    ]
}

fn mod_files(game: &Path) -> Result<Vec<PathBuf>, String> {
    let dir = game.join("r6/input");
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("lendo {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("xml")).unwrap_or(false))
        .collect();
    files.sort();
    Ok(files)
}

fn cmd_merge(game: &Path) -> Result<(), String> {
    let mods = mod_files(game)?;
    if mods.is_empty() {
        return Err(format!(
            "nenhum mod em {}/r6/input/*.xml",
            game.display()
        ));
    }

    // Coleta os elementos de todos os mods, classificados por tag.
    let mut by_tag: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut unknown: Vec<String> = Vec::new();
    for mf in &mods {
        let text = std::fs::read_to_string(mf).map_err(|e| format!("lendo {}: {e}", mf.display()))?;
        let inner = bindings_inner(&text).unwrap_or(&text);
        for (tag, elem) in extract_elements(inner) {
            if MAPPING_TAGS.contains(&tag.as_str()) || CONTEXT_TAGS.contains(&tag.as_str()) {
                by_tag.entry(tag).or_default().push(elem);
            } else if !unknown.contains(&tag) {
                unknown.push(tag);
            }
        }
    }
    if !unknown.is_empty() {
        eprintln!("aviso: tags ignoradas (não roteáveis): {}", unknown.join(", "));
    }

    for t in targets(game) {
        if !t.base.is_file() {
            return Err(format!("config base não existe: {}", t.base.display()));
        }
        // Snapshot do pristino na 1ª vez; merge SEMPRE parte dele (idempotente).
        if !t.orig.exists() {
            std::fs::copy(&t.base, &t.orig)
                .map_err(|e| format!("salvando pristino {}: {e}", t.orig.display()))?;
        }
        let base = std::fs::read_to_string(&t.orig)
            .map_err(|e| format!("lendo pristino {}: {e}", t.orig.display()))?;

        let mut additions = String::new();
        let mut n = 0usize;
        for tag in t.tags {
            if let Some(elems) = by_tag.get(*tag) {
                for e in elems {
                    additions.push_str("\n\t");
                    additions.push_str(e.trim());
                    n += 1;
                }
            }
        }
        let merged = merge_into(&base, &format!(
            "\n\t<!-- input-loader: {n} node(s) de mods -->{additions}\n"
        ))?;
        std::fs::write(&t.base, merged.as_bytes())
            .map_err(|e| format!("gravando {}: {e}", t.base.display()))?;
        println!("{}: +{n} node(s) → {}", t.label, t.base.display());
    }
    println!("merge de {} mod(s) ok. (restore volta ao pristino)", mods.len());
    Ok(())
}

fn cmd_restore(game: &Path) -> Result<(), String> {
    let mut restored = 0;
    for t in targets(game) {
        if t.orig.is_file() {
            std::fs::copy(&t.orig, &t.base)
                .map_err(|e| format!("restaurando {}: {e}", t.base.display()))?;
            println!("restaurado: {}", t.label);
            restored += 1;
        }
    }
    if restored == 0 {
        return Err("sem pristino (.input-loader-orig) — nada a restaurar".into());
    }
    Ok(())
}

fn cmd_status(game: &Path) -> Result<(), String> {
    let mods = mod_files(game)?;
    println!("# input-loader — {}", game.display());
    println!("mods em r6/input: {}", mods.len());
    for m in &mods {
        println!("  {}", m.file_name().unwrap_or_default().to_string_lossy());
    }
    for t in targets(game) {
        let merged = t.base.is_file()
            && std::fs::read_to_string(&t.base)
                .map(|s| s.contains("<!-- input-loader:"))
                .unwrap_or(false);
        println!(
            "{}: base {} · pristino {} · {}",
            t.label,
            if t.base.is_file() { "ok" } else { "AUSENTE" },
            if t.orig.is_file() { "ok" } else { "—" },
            if merged { "MESCLADO" } else { "vanilla" }
        );
    }
    Ok(())
}

/// Conteúdo entre `<bindings>` e `</bindings>` (o primeiro/último).
fn bindings_inner(xml: &str) -> Option<&str> {
    let open = xml.find("<bindings")?;
    let after_open = open + xml[open..].find('>')? + 1;
    let close = xml.rfind("</bindings>")?;
    if close <= after_open {
        return None;
    }
    Some(&xml[after_open..close])
}

/// Insere `additions` imediatamente antes do `</bindings>` final.
fn merge_into(base: &str, additions: &str) -> Result<String, String> {
    let pos = base.rfind("</bindings>").ok_or("base sem </bindings>")?;
    Ok(format!("{}{}{}", &base[..pos], additions, &base[pos..]))
}

/// Extrai os elementos de topo de um conteúdo XML: `(tag, texto_completo)`.
/// Pula comentários e declarações; trata self-closing e aninhamento.
fn extract_elements(inner: &str) -> Vec<(String, String)> {
    let b = inner.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if inner[i..].starts_with("<!--") {
            match inner[i..].find("-->") {
                Some(e) => {
                    i += e + 3;
                    continue;
                }
                None => break,
            }
        }
        if inner[i..].starts_with("<?") || inner[i..].starts_with("<!") {
            match inner[i..].find('>') {
                Some(e) => {
                    i += e + 1;
                    continue;
                }
                None => break,
            }
        }
        if i + 1 < b.len() && b[i + 1] == b'/' {
            i += 1; // close-tag solto
            continue;
        }
        let start = i;
        let name_start = i + 1;
        let mut j = name_start;
        while j < b.len()
            && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] == b':' || b[j] == b'-')
        {
            j += 1;
        }
        if j == name_start {
            i += 1;
            continue;
        }
        let tag = inner[name_start..j].to_string();
        let open_end = match inner[start..].find('>') {
            Some(p) => start + p,
            None => break,
        };
        if open_end > 0 && b[open_end - 1] == b'/' {
            // self-closing
            out.push((tag, inner[start..=open_end].to_string()));
            i = open_end + 1;
            continue;
        }
        // procura o </tag> correspondente (depth por mesmo nome)
        let open_pat = format!("<{tag}");
        let close_pat = format!("</{tag}>");
        let mut depth = 1i32;
        let mut k = open_end + 1;
        let mut done = false;
        while k < inner.len() {
            let no = inner[k..].find(&open_pat).map(|p| k + p);
            let nc = inner[k..].find(&close_pat).map(|p| k + p);
            match (no, nc) {
                (Some(o), Some(c)) if o < c => {
                    // ignora self-closing do mesmo nome
                    let oe = inner[o..].find('>').map(|p| o + p);
                    let self_close = oe.map(|e| b[e - 1] == b'/').unwrap_or(false);
                    if !self_close {
                        depth += 1;
                    }
                    k = oe.map(|e| e + 1).unwrap_or(o + open_pat.len());
                }
                (_, Some(c)) => {
                    depth -= 1;
                    if depth == 0 {
                        let end = c + close_pat.len();
                        out.push((tag.clone(), inner[start..end].to_string()));
                        i = end;
                        done = true;
                        break;
                    }
                    k = c + close_pat.len();
                }
                _ => break,
            }
        }
        if !done {
            break; // desbalanceado
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extrai_self_closing_e_aninhado() {
        let xml = r#"
            <mapping name="A" type="Axis">
                <button id="IK_W"/>
            </mapping>
            <!-- c -->
            <preset name="P"/>
            <context name="C"><action/></context>
        "#;
        let els = extract_elements(xml);
        let tags: Vec<&str> = els.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(tags, vec!["mapping", "preset", "context"]);
        assert!(els[0].1.contains("IK_W"));
        assert!(els[1].1.ends_with("/>"));
    }

    #[test]
    fn merge_insere_antes_do_fecho() {
        let base = "<bindings>\n\t<mapping name=\"X\"/>\n</bindings>\n";
        let out = merge_into(base, "\n\t<mapping name=\"Y\"/>\n").unwrap();
        assert!(out.find("name=\"Y\"").unwrap() < out.find("</bindings>").unwrap());
        assert!(out.find("name=\"X\"").unwrap() < out.find("name=\"Y\"").unwrap());
    }

    #[test]
    fn bindings_inner_pega_miolo() {
        let xml = "<?xml?>\n<bindings>\nMEIO\n</bindings>\n";
        assert_eq!(bindings_inner(xml).unwrap().trim(), "MEIO");
    }
}
