//! Classificação SEGURA de um pacote de mod (já extraído numa pasta).
//!
//! A regra de ouro (do design do dono): o pacote DESCREVE, a ferramenta DECIDE e EXECUTA.
//! Aqui só inspecionamos — nada é instalado nem executado. Quatro classes (conteúdo puro /
//! REDmod / script adaptável / código nativo) + 3 estados de compat (universal / adaptável-Mac
//! / precisa-port-nativo) + detecção de dependências + flags de risco (path-traversal, scripts,
//! binários, symlinks).

use std::path::{Path, PathBuf};

/// As 4 classes de mod (fronteira conteúdo declarativo ↔ código executável).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClass {
    PureContent,  // .archive/.xl/.mesh/textura — dados que o jogo/frameworks interpretam
    RedMod,       // pacote REDmod oficial (info.json + archives/scripts)
    Script,       // .reds (redscript) ou Lua de CET — roda DENTRO do jogo
    NativeCode,   // .dll/.dylib/Mach-O — código nativo, canal restrito
    Mixed,        // mistura (ex.: conteúdo + script)
    Unknown,      // nada reconhecido
}

/// Estado de compatibilidade macOS (3 estados, melhor que sim/não).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Universal,           // o mesmo arquivo roda em Windows e Mac (conteúdo puro)
    MacAdapter,          // roda via nossa adaptação (redscript/TweakXL/ArchiveXL-offline/CET-Mac)
    NativePortRequired,  // tem código Windows ou depende de hook ainda ausente
}

/// Tipo de UM arquivo dentro do pacote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Archive,    // .archive (conteúdo empacotado)
    ArchiveXl,  // .xl (manifesto ArchiveXL)
    Content,    // mesh/ent/textura/etc. (loose content)
    Redscript,  // .reds
    Tweak,      // .yaml/.tweak (TweakXL)
    CetLua,     // .lua de mod CET
    RedModInfo, // info.json (manifesto REDmod)
    Native,     // .dll/.dylib/Mach-O/exe
    InstallScript, // .sh/.command/.bat/.ps1 — NUNCA executar
    Other,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub rel: PathBuf,   // caminho relativo à raiz do pacote
    pub kind: FileKind,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub detail: String,
}

#[derive(Debug)]
pub struct ModReport {
    pub name: String,
    pub class: ModClass,
    pub compat: Compat,
    pub files: Vec<FileEntry>,
    pub deps: Vec<Dep>,
    pub risks: Vec<String>,   // path-traversal, scripts, binários, symlinks — não-vazio = revisar
    pub notes: Vec<String>,   // achados informativos (ex.: o que o .xl faz e o que precisa de runtime)
}

/// Reconhece o tipo de um arquivo por extensão + posição (path).
fn kind_of(rel: &Path) -> FileKind {
    let name = rel.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = rel.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    let path = rel.to_string_lossy().to_ascii_lowercase();
    if name.eq_ignore_ascii_case("info.json") {
        return FileKind::RedModInfo;
    }
    match ext.as_str() {
        "archive" => FileKind::Archive,
        "xl" => FileKind::ArchiveXl,
        "reds" => FileKind::Redscript,
        "lua" => FileKind::CetLua,
        "yaml" | "yml" | "tweak" => FileKind::Tweak,
        "dll" | "dylib" | "exe" | "so" => FileKind::Native,
        "sh" | "command" | "bat" | "ps1" | "zsh" => FileKind::InstallScript,
        "mesh" | "ent" | "mi" | "mlsetup" | "app" | "streamingsector" | "xbm" | "dds"
        | "morphtarget" | "anims" | "wem" | "opusinfo" | "opuspak" | "json" | "inkatlas"
        | "inkwidget" | "inkstyle" | "physmatlib" | "mt" | "csv" => FileKind::Content,
        _ => {
            // Mach-O sem extensão (executável): detecta pelo path comum
            if path.contains("/bin/") || path.contains("plugins/") {
                FileKind::Other // refina via magic-bytes em scan_risks
            } else {
                FileKind::Other
            }
        }
    }
}

/// Lista recursiva de arquivos (rel à raiz), pulando .git e afins.
fn walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let p = e.path();
            let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if fname.starts_with('.') && (fname == ".git" || fname == ".DS_Store") {
                continue;
            }
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(p),
                Ok(ft) if ft.is_file() => {
                    if let Ok(rel) = p.strip_prefix(root) {
                        out.push(rel.to_path_buf());
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Deriva a classe agregada a partir dos tipos dos arquivos.
fn class_of(files: &[FileEntry]) -> ModClass {
    let has = |k: FileKind| files.iter().any(|f| f.kind == k);
    let has_native = has(FileKind::Native);
    let has_script = has(FileKind::Redscript) || has(FileKind::CetLua);
    let has_content = has(FileKind::Archive) || has(FileKind::Content) || has(FileKind::ArchiveXl) || has(FileKind::Tweak);
    if has(FileKind::RedModInfo) {
        return ModClass::RedMod;
    }
    if has_native {
        return ModClass::NativeCode;
    }
    match (has_script, has_content) {
        (true, true) => ModClass::Mixed,
        (true, false) => ModClass::Script,
        (false, true) => ModClass::PureContent,
        (false, false) => ModClass::Unknown,
    }
}

/// Detecta dependências de framework olhando os arquivos (não executa nada).
fn detect_deps(files: &[FileEntry]) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut add = |name: &str, detail: &str| deps.push(Dep { name: name.into(), detail: detail.into() });
    if files.iter().any(|f| f.kind == FileKind::ArchiveXl) {
        add("ArchiveXL", "tem .xl (manifesto) → precisa do ArchiveXL");
    }
    if files.iter().any(|f| f.kind == FileKind::Tweak) {
        add("TweakXL", "tem .yaml/.tweak → precisa do TweakXL");
    }
    if files.iter().any(|f| f.kind == FileKind::Redscript) {
        add("redscript", "tem .reds → precisa do compilador redscript");
    }
    if files.iter().any(|f| f.kind == FileKind::CetLua) {
        add("CET", "tem .lua de mod → precisa do Cyber Engine Tweaks");
    }
    if files.iter().any(|f| f.kind == FileKind::Native) {
        add("RED4ext/nativo", "tem .dll/.dylib → plugin nativo (canal restrito)");
    }
    deps
}

/// Estado de compat macOS a partir da classe + deps.
fn compat_of(class: ModClass, deps: &[Dep]) -> Compat {
    let dep = |n: &str| deps.iter().any(|d| d.name == n);
    if class == ModClass::NativeCode || dep("RED4ext/nativo") {
        return Compat::NativePortRequired; // código Windows / hook runtime ausente
    }
    if class == ModClass::PureContent && deps.is_empty() {
        return Compat::Universal; // conteúdo puro, sem framework → o mesmo arquivo
    }
    Compat::MacAdapter // redscript/TweakXL/ArchiveXL-offline/CET-Mac cobrem
}

/// Magic-bytes Mach-O (executável nativo sem extensão).
fn is_macho(abs: &Path) -> bool {
    let mut buf = [0u8; 4];
    if let Ok(mut f) = std::fs::File::open(abs) {
        use std::io::Read;
        if f.read_exact(&mut buf).is_ok() {
            let m = u32::from_le_bytes(buf);
            // MH_MAGIC_64 0xfeedfacf, MH_CIGAM_64 0xcffaedfe, FAT 0xcafebabe/bebafeca
            return matches!(m, 0xfeed_facf | 0xcffa_edfe | 0xfeed_face | 0xcefa_edfe)
                || matches!(u32::from_be_bytes(buf), 0xcafe_babe | 0xcafe_babf);
        }
    }
    false
}

/// Flags de risco: path-traversal, absolutos, symlinks, scripts, binários ocultos.
fn scan_risks(root: &Path, files: &mut [FileEntry]) -> Vec<String> {
    let mut risks = Vec::new();
    for f in files.iter_mut() {
        let s = f.rel.to_string_lossy();
        if s.contains("..") {
            risks.push(format!("PATH-TRAVERSAL: '{s}' (REJEITAR — tenta escapar da pasta)"));
        }
        if f.rel.is_absolute() || s.starts_with('/') || s.starts_with('~') {
            risks.push(format!("CAMINHO ABSOLUTO: '{s}' (REJEITAR)"));
        }
        if f.kind == FileKind::InstallScript {
            risks.push(format!("SCRIPT DE INSTALAÇÃO: '{s}' (NUNCA executar; a ferramenta instala)"));
        }
        // Mach-O sem extensão escondido como "Other"
        if f.kind == FileKind::Other && is_macho(&root.join(&f.rel)) {
            f.kind = FileKind::Native;
            risks.push(format!("BINÁRIO Mach-O: '{s}' (código nativo — canal restrito)"));
        }
        // symlink dentro do pacote
        if let Ok(md) = std::fs::symlink_metadata(root.join(&f.rel)) {
            if md.file_type().is_symlink() {
                risks.push(format!("SYMLINK: '{s}' (REJEITAR — pode apontar fora)"));
            }
        }
    }
    risks
}

/// Parseia os `.xl` do pacote e descreve, honestamente, o que cada um faz — e o que ainda
/// depende de runtime no Mac. A regra de ouro do Path A: o(s) `.archive` carregam pelo GLOB
/// nativo (provado in-game), mas as OPERAÇÕES do `.xl` (factories/patch/link/scope/copy/fix)
/// exigem o ArchiveXL runtime, que ainda NÃO está portado — então essas partes não aplicam.
fn analyze_xl(root: &Path, files: &[FileEntry]) -> Vec<String> {
    let mut notes = Vec::new();
    for f in files.iter().filter(|f| f.kind == FileKind::ArchiveXl) {
        let rel = f.rel.display();
        let text = match std::fs::read_to_string(root.join(&f.rel)) {
            Ok(t) => t,
            Err(e) => {
                notes.push(format!(".xl '{rel}' não pôde ser lido: {e}"));
                continue;
            }
        };
        let xl = match crate::xl::parse_xl(&text) {
            Ok(x) => x,
            Err(e) => {
                notes.push(format!(".xl '{rel}' não parseou: {e}"));
                continue;
            }
        };
        // descreve as operações de runtime presentes
        let mut parts = Vec::new();
        let mut push = |n: usize, label: &str| {
            if n > 0 {
                parts.push(format!("{n} {label}"));
            }
        };
        push(xl.factories.len(), "factory(ies)");
        push(xl.patches.len(), "patch(es)");
        push(xl.links.len(), "link(s)");
        push(xl.scopes.len(), "scope(s)");
        push(xl.copies.len(), "copy(ies)");
        push(xl.fixes.len(), "fix(es)");
        push(xl.localization.len(), "grupo(s) de localização");
        if parts.is_empty() {
            notes.push(format!(
                ".xl '{rel}': sem operações de runtime — o(s) .archive carregam via Path A (provado in-game)."
            ));
        } else {
            notes.push(format!(
                ".xl '{rel}': {} → o(s) .archive carregam via Path A, mas essas operações precisam do ArchiveXL runtime (ainda não no Mac).",
                parts.join(", ")
            ));
        }
        if !xl.other_sections.is_empty() {
            notes.push(format!(".xl '{rel}': seções não reconhecidas pelo parser: {}", xl.other_sections.join(", ")));
        }
    }
    notes
}

/// Classifica um pacote de mod já extraído em `root`. Não instala nem executa nada.
pub fn classify(root: &Path) -> ModReport {
    let name = root.file_name().and_then(|s| s.to_str()).unwrap_or("mod").to_string();
    let mut files: Vec<FileEntry> = walk(root)
        .into_iter()
        .map(|rel| {
            let kind = kind_of(&rel);
            FileEntry { rel, kind }
        })
        .collect();
    let risks = scan_risks(root, &mut files); // pode reclassificar Mach-O
    let class = class_of(&files);
    let deps = detect_deps(&files);
    let compat = compat_of(class, &deps);
    let notes = analyze_xl(root, &files);
    ModReport { name, class, compat, files, deps, risks, notes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, content: &str) {
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    #[test]
    fn xl_com_factory_avisa_que_precisa_de_runtime() {
        let g = std::env::temp_dir().join(format!("bwms-classify-xl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("MeuMod");
        touch(&m.join("x.archive"), "FAKE");
        touch(&m.join("mymod.xl"), "factories:\n  - mymod\\f.csv\n");

        let r = classify(&m);
        // detecta a dependência do ArchiveXL
        assert!(r.deps.iter().any(|d| d.name == "ArchiveXL"));
        // e a nota honesta: factory precisa de runtime (não só Path A)
        assert!(r.notes.iter().any(|n| n.contains("factory") && n.contains("runtime")), "notes: {:?}", r.notes);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn xl_so_de_carga_de_archive_diz_que_path_a_funciona() {
        let g = std::env::temp_dir().join(format!("bwms-classify-xl2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModSimples");
        touch(&m.join("y.archive"), "FAKE");
        // .xl vazio (sem operações) → carga via Path A funciona
        touch(&m.join("empty.xl"), "# só pra existir\n");

        let r = classify(&m);
        assert!(r.notes.iter().any(|n| n.contains("Path A")), "notes: {:?}", r.notes);

        let _ = std::fs::remove_dir_all(&g);
    }
}
