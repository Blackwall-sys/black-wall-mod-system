//! Classificação SEGURA de um pacote de mod (já extraído numa pasta).
//!
//! A regra de ouro (do design do dono): o pacote DESCREVE, a ferramenta DECIDE e EXECUTA.
//! Aqui só inspecionamos — nada é instalado nem executado. Quatro classes (conteúdo puro /
//! REDmod / script adaptável / código nativo) + 3 estados de compat (universal / adaptável-Mac
//! / precisa-port-nativo) + detecção de dependências + flags de risco (path-traversal, scripts,
//! binários, symlinks, tipos especiais, itens não lidos).

use crate::instalacao::tabelas_de_runtime::SECOES_SEM_TABELA;
use crate::instalacao::xl::SecaoXl;
use std::path::{Component, Path, PathBuf};

/// As 4 classes de mod (fronteira conteúdo declarativo ↔ código executável).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModClass {
    PureContent,  // .archive/.xl/.mesh/textura — dados que o jogo/frameworks interpretam
    RedMod,       // pacote REDmod oficial (info.json + archives/scripts)
    Script,       // .reds (redscript, roda dentro do jogo) e/ou .lua de mod de CET (mod de CET inteiro ainda não roda no BWMS)
    NativeCode,   // .dll/.dylib/Mach-O — código nativo, canal restrito
    Mixed,        // mistura (ex.: conteúdo + script)
    Unknown,      // nada reconhecido
}

/// Estado de compatibilidade macOS (3 estados, melhor que sim/não).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    Universal,           // o mesmo arquivo roda em Windows e Mac (conteúdo puro)
    MacAdapter,          // roda via nossa adaptação (redscript/TweakXL/ArchiveXL-offline)
    NativePortRequired,  // tem código Windows, mod de CET (inteiro, ainda não roda no BWMS) ou depende de hook ainda ausente
}

/// Tipo de UM arquivo dentro do pacote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Archive,    // .archive (conteúdo empacotado)
    Xl,         // .xl (manifesto do mod; o runtime aplica as seções com tabela, `tabelas_de_runtime`)
    Content,    // mesh/ent/textura/etc. (loose content)
    Redscript,  // .reds
    Tweak,      // .yaml/.tweak (TweakXL)
    Lua,        // .lua de mod de CET (mod de CET inteiro ainda não roda no BWMS)
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
    pub risks: Vec<String>,   // path-traversal, scripts, binários, symlinks, tipos especiais, não lidos — não-vazio = revisar
    pub notes: Vec<String>,   // achados informativos (ex.: o que o .xl faz e o que precisa de runtime)
    /// O que os `.xl` do pacote deixam fora do jogo, `.xl '<rel>': <motivo>; <ação>`: `.xl` ilegível, que não parseia, cada entrada de
    /// `XlFile::issues` e cada entrada que o runtime não aplica (`tabelas_de_runtime::entradas_sem_efeito`). Fica fora de `notes`: são
    /// os mesmos motivos que o relatório da `Mods/` dá quando o runtime lê o `.xl`, e o `bwms classify` os imprime à parte.
    pub avisos_do_xl: Vec<String>,
}

/// Reconhece o tipo de um arquivo pelo nome (info.json) e pela extensão. Mach-O sem extensão ou com extensão de conteúdo sai
/// daqui como Other/Content e é reclassificado pelo magic em scan_risks.
fn kind_of(rel: &Path) -> FileKind {
    let name = rel.file_name().and_then(|s| s.to_str()).unwrap_or("");
    let ext = rel.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
    if name.eq_ignore_ascii_case("info.json") {
        return FileKind::RedModInfo;
    }
    match ext.as_str() {
        "archive" => FileKind::Archive,
        "xl" => FileKind::Xl,
        "reds" => FileKind::Redscript,
        "lua" => FileKind::Lua,
        // A regra do runtime: `.toml` de config de plugin não vira tweak.
        e if crate::formato_tweak(e).is_some() => FileKind::Tweak,
        "dll" | "dylib" | "exe" | "so" => FileKind::Native,
        "sh" | "command" | "bat" | "ps1" | "zsh" => FileKind::InstallScript,
        "mesh" | "ent" | "mi" | "mlsetup" | "app" | "streamingsector" | "xbm" | "dds"
        | "morphtarget" | "anims" | "wem" | "opusinfo" | "opuspak" | "json" | "inkatlas"
        | "inkwidget" | "inkstyle" | "physmatlib" | "mt" | "csv" => FileKind::Content,
        _ => FileKind::Other,
    }
}

/// Tipo do nó no disco como a travessia o viu, sem seguir symlink.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    File,
    Symlink,
    /// FIFO, socket ou dispositivo.
    Special,
}

/// Nome que nenhuma travessia de pasta de mod segue, em qualquer nível e de qualquer tipo: `.git` (histórico do autor) e `.DS_Store`
/// (do Finder). A travessia da `Mods/` (`apply::arquivos_da_pasta_mods`) pula todo nome que começa com `.`, estes dois inclusive.
pub(crate) fn fora_da_travessia(nome: &std::ffi::OsStr) -> bool {
    nome == ".git" || nome == ".DS_Store"
}

/// Lista recursiva (rel à raiz), pulando o que `fora_da_travessia` pula. Todo nó que não é pasta entra com o tipo
/// visto aqui. O que não pôde ser lido vai para `unread`: pasta ilegível sumindo da lista faria o
/// relatório sair limpo sem ter visto o que tem dentro.
fn walk(root: &Path, unread: &mut Vec<String>) -> Vec<(PathBuf, NodeKind)> {
    let shown = |rel: &Path| {
        if rel.as_os_str().is_empty() { root.display().to_string() } else { rel.display().to_string() }
    };
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(dir) = stack.pop() {
        let rd = match std::fs::read_dir(root.join(&dir)) {
            Ok(r) => r,
            Err(e) => {
                unread.push(format!("NÃO LIDO: '{}': {e} (REJEITAR — o scan não viu o conteúdo)", shown(&dir)));
                continue;
            }
        };
        for entry in rd {
            let e = match entry {
                Ok(e) => e,
                Err(err) => {
                    unread.push(format!("NÃO LIDO: entrada de '{}': {err} (REJEITAR — o scan não viu o conteúdo)", shown(&dir)));
                    continue;
                }
            };
            let fname = e.file_name();
            if fora_da_travessia(&fname) {
                continue;
            }
            let rel = dir.join(&fname);
            match e.file_type() {
                Ok(ft) if ft.is_dir() => stack.push(rel),
                Ok(ft) if ft.is_file() => out.push((rel, NodeKind::File)),
                Ok(ft) if ft.is_symlink() => out.push((rel, NodeKind::Symlink)),
                Ok(_) => out.push((rel, NodeKind::Special)),
                Err(err) => {
                    unread.push(format!("NÃO LIDO: '{}': {err} (REJEITAR — o scan não viu o conteúdo)", rel.display()));
                }
            }
        }
    }
    out
}

/// Deriva a classe agregada a partir dos tipos dos arquivos.
fn class_of(files: &[FileEntry]) -> ModClass {
    let has = |k: FileKind| files.iter().any(|f| f.kind == k);
    let has_native = has(FileKind::Native);
    let has_script = has(FileKind::Redscript) || has(FileKind::Lua);
    let has_content = has(FileKind::Archive) || has(FileKind::Content) || has(FileKind::Xl) || has(FileKind::Tweak);
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

/// Nome das dependências que o `compat_of` lê: `detect_deps` grava e `compat_of` casa pelo mesmo nome.
const DEP_LUA: &str = "CET";
const DEP_NATIVO: &str = "nativo";

/// Detecta dependências de framework olhando os arquivos (não executa nada). O nome é o do framework que o mod pede na
/// página dele; o detalhe diz o que o BWMS faz com o arquivo.
fn detect_deps(files: &[FileEntry]) -> Vec<Dep> {
    let mut deps = Vec::new();
    let mut add = |name: &str, detail: &str| deps.push(Dep { name: name.into(), detail: detail.into() });
    if files.iter().any(|f| f.kind == FileKind::Xl) {
        add("ArchiveXL", "tem .xl → o BWMS aplica as seções pelas tabelas em red4ext/ ou pelo .json que o jogo lê; as notas dizem quem aplica cada uma e o que fica de fora");
    }
    if files.iter().any(|f| f.kind == FileKind::Tweak) {
        add("TweakXL", "tem .yaml/.tweak → o BWMS aplica o que estiver em r6/tweaks/");
    }
    if files.iter().any(|f| f.kind == FileKind::Redscript) {
        add("redscript", "tem .reds → precisa do compilador redscript");
    }
    if files.iter().any(|f| f.kind == FileKind::Lua) {
        add(DEP_LUA, "mod de CET: o BWMS roda os .lua pela pasta Mods; o que depende de .dll não");
    }
    if files.iter().any(|f| f.kind == FileKind::Native) {
        add(DEP_NATIVO, "tem .dll/.dylib → plugin nativo (canal restrito)");
    }
    deps
}

/// Estado de compat macOS a partir da classe + deps.
fn compat_of(class: ModClass, deps: &[Dep]) -> Compat {
    let dep = |n: &str| deps.iter().any(|d| d.name == n);
    if class == ModClass::NativeCode || dep(DEP_NATIVO) {
        return Compat::NativePortRequired; // código Windows / hook runtime ausente
    }
    if class == ModClass::PureContent && deps.is_empty() {
        return Compat::Universal; // conteúdo puro, sem framework → o mesmo arquivo
    }
    Compat::MacAdapter // redscript/TweakXL/ArchiveXL-offline cobrem
}

/// Magic-bytes Mach-O. Arquivo com menos de 4 bytes não é Mach-O; erro de open/read sobe, porque
/// arquivo ilegível tratado como "não é binário" passaria pelo gate.
fn is_macho(abs: &Path) -> std::io::Result<bool> {
    use std::io::Read;
    let mut buf = [0u8; 4];
    match std::fs::File::open(abs)?.read_exact(&mut buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
        Err(e) => return Err(e),
    }
    let m = u32::from_le_bytes(buf);
    // MH_MAGIC_64/MH_MAGIC e as formas trocadas (CIGAM); FAT_MAGIC/FAT_MAGIC_64 ficam em big-endian no disco.
    Ok(matches!(m, 0xfeed_facf | 0xcffa_edfe | 0xfeed_face | 0xcefa_edfe)
        || matches!(u32::from_be_bytes(buf), 0xcafe_babe | 0xcafe_babf))
}

/// Caminho que sai da raiz do pacote. Componente que não é Normal (`..`, `.`, raiz, prefixo) sai
/// pela estrutura. Componente Normal é dividido em '/' e '\' e sai se algum pedaço for exatamente
/// `..`: o arquivo literal `..\..\x.yaml` é um único Component::Normal no APFS e vira
/// `../../x.yaml` para quem troca '\' por '/' (o caminho do Windows). Pedaço igual, não substring, para que nome
/// legítimo como `Jaqueta..v2.archive` passe.
fn escapes_root(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        Component::Normal(s) => s.to_string_lossy().split(['/', '\\']).any(|p| p == ".."),
        _ => true,
    })
}

/// Flags de risco: path-traversal, absolutos, symlinks, tipos especiais, scripts e código nativo.
/// Régua única para binário: todo arquivo regular passa pelo magic, porque o nome não decide. Mach-O
/// sem extensão ou com extensão de binário Mac (dylib/so) é BINÁRIO; com qualquer outra extensão,
/// inclusive .dll/.exe, é MASCARADO. Nativo pela extensão que não é Mach-O (PE, ELF) também bloqueia:
/// o Mac não o carrega, e um .reds do pacote que declara `native func` dele derruba o bind dos
/// scripts com SIGTRAP. `nodes[i]` é o tipo de `files[i]` no disco.
fn scan_risks(root: &Path, files: &mut [FileEntry], nodes: &[NodeKind]) -> Vec<String> {
    let mut risks = Vec::new();
    for (f, &node) in files.iter_mut().zip(nodes) {
        let s = f.rel.to_string_lossy();
        if escapes_root(&f.rel) {
            risks.push(format!("PATH-TRAVERSAL: '{s}' (REJEITAR — tenta escapar da pasta)"));
        }
        // `~` no início é nome comum: nenhum consumidor passa o caminho por shell.
        if f.rel.is_absolute() || s.starts_with('/') {
            risks.push(format!("CAMINHO ABSOLUTO: '{s}' (REJEITAR)"));
        }
        if f.kind == FileKind::InstallScript {
            risks.push(format!("SCRIPT DE INSTALAÇÃO: '{s}' (NUNCA executar; a ferramenta instala)"));
        }
        // Symlink e tipo especial nunca são abertos: o alvo do link pode estar fora do pacote, e
        // open() num FIFO sem escritor trava o scan.
        match node {
            NodeKind::Symlink => risks.push(format!("SYMLINK: '{s}' (REJEITAR — pode apontar fora)")),
            NodeKind::Special => risks.push(format!("TIPO ESPECIAL: '{s}' (FIFO/socket/dispositivo — REJEITAR)")),
            NodeKind::File => match is_macho(&root.join(&f.rel)) {
                Ok(true) => {
                    f.kind = FileKind::Native;
                    let ext = f.rel.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
                    if matches!(ext.as_deref(), None | Some("dylib" | "so")) {
                        risks.push(format!("BINÁRIO Mach-O: '{s}' (código nativo — canal restrito)"));
                    } else {
                        risks.push(format!("BINÁRIO Mach-O MASCARADO: '{s}' (a extensão esconde código nativo — canal restrito)"));
                    }
                }
                Ok(false) if f.kind == FileKind::Native => risks.push(format!(
                    "CÓDIGO NATIVO NÃO-MAC: '{s}' (o Mac não carrega; scripts do pacote podem depender dele — REJEITAR ou remover do pacote)"
                )),
                Ok(false) => {}
                Err(e) => risks.push(format!("NÃO LIDO: '{s}': {e} (REJEITAR — o scan não viu o conteúdo)")),
            },
        }
    }
    risks
}

/// Parseia os `.xl` do pacote e descreve o que cada um faz, pela lista única de seções
/// (`XlFile::secoes`), mais as seções que o parser não reconhece. Path A: o(s) `.archive` carregam
/// pelo GLOB nativo (provado in-game); o runtime do BWMS aplica as seções que têm tabela (`tabelas_de_runtime`), o jogo aplica o `factories` (`xl_no_jogo`), e as de
/// `tabelas_de_runtime::SECOES_SEM_TABELA` ganham nota própria, porque ficam sem efeito no jogo.
/// Devolve `(notas, avisos)`: os avisos são o que fica de fora (`ModReport::avisos_do_xl`).
fn analyze_xl(root: &Path, files: &[FileEntry]) -> (Vec<String>, Vec<String>) {
    let mut notes = Vec::new();
    let mut avisos = Vec::new();
    for f in files.iter().filter(|f| f.kind == FileKind::Xl) {
        let rel = f.rel.display();
        let text = match std::fs::read_to_string(root.join(&f.rel)) {
            Ok(t) => t,
            Err(e) => {
                avisos.push(format!(".xl '{rel}' não pôde ser lido ({e}); confira se é texto UTF-8 com permissão de leitura"));
                continue;
            }
        };
        let xl = match crate::instalacao::xl::parse_xl(&text) {
            Ok(x) => x,
            Err(e) => {
                avisos.push(format!(".xl '{rel}' não parseou ({e}); corrija o YAML do arquivo"));
                continue;
            }
        };
        let secoes = xl.secoes();
        let (sem_tabela, com_tabela): (Vec<&SecaoXl>, Vec<&SecaoXl>) =
            secoes.iter().filter(|s| s.entradas > 0).partition(|s| SECOES_SEM_TABELA.contains(&s.chave));
        // O `factories` não tem tabela: o próprio jogo o aplica pelo `.json` que o BWMS grava (`xl_no_jogo`).
        let (no_jogo, com_tabela): (Vec<&SecaoXl>, Vec<&SecaoXl>) = com_tabela.into_iter().partition(|s| s.chave == "factories");
        let lista = |ss: &[&SecaoXl]| ss.iter().map(|s| format!("{} ×{}", s.chave, s.entradas)).collect::<Vec<_>>().join(", ");
        if !no_jogo.is_empty() {
            notes.push(format!(
                ".xl '{rel}': {} → o próprio jogo aplica, pelo .json que o BWMS grava em archive/Mac/mod/BWMS-Mods/xl.",
                lista(&no_jogo)
            ));
        }
        if com_tabela.is_empty() && sem_tabela.is_empty() && no_jogo.is_empty() {
            notes.push(format!(
                ".xl '{rel}': sem operações de runtime — o(s) .archive carregam via Path A (provado in-game)."
            ));
        }
        if !com_tabela.is_empty() {
            notes.push(format!(
                ".xl '{rel}': {} → o(s) .archive carregam via Path A; o runtime do BWMS aplica essas seções pelas tabelas em red4ext/.",
                lista(&com_tabela)
            ));
        }
        if !sem_tabela.is_empty() {
            notes.push(format!(
                ".xl '{rel}': {} → o BWMS ainda não aplica essas seções, que ficam sem efeito no jogo; o resto do mod carrega.",
                lista(&sem_tabela)
            ));
        }
        if !xl.other_sections.is_empty() {
            notes.push(format!(".xl '{rel}': seções não reconhecidas pelo parser: {}", xl.other_sections.join(", ")));
        }
        avisos.extend(xl.issues.iter().map(|aviso| format!(".xl '{rel}': {aviso}")));
        avisos.extend(crate::instalacao::tabelas_de_runtime::entradas_sem_efeito(&xl).iter().map(|aviso| format!(".xl '{rel}': {aviso}")));
    }
    (notes, avisos)
}

/// Classifica um pacote de mod já extraído em `root`. Não instala nem executa nada.
pub fn classify(root: &Path) -> ModReport {
    let name = root.file_name().and_then(|s| s.to_str()).unwrap_or("mod").to_string();
    let mut risks = Vec::new();
    let (rels, nodes): (Vec<PathBuf>, Vec<NodeKind>) = walk(root, &mut risks).into_iter().unzip();
    let mut files: Vec<FileEntry> = rels
        .into_iter()
        .map(|rel| {
            let kind = kind_of(&rel);
            FileEntry { rel, kind }
        })
        .collect();
    risks.extend(scan_risks(root, &mut files, &nodes)); // pode reclassificar Mach-O
    let class = class_of(&files);
    let deps = detect_deps(&files);
    let compat = compat_of(class, &deps);
    let (mut notes, avisos_do_xl) = analyze_xl(root, &files);
    // Sem esta nota o .lua some calado de quem lê só as notas.
    // A nota fala só do .lua. O destino do resto não é dela: o gate de risco pode barrar o pacote
    // inteiro, e o resto do mod (ex.: o settings.json do mod de CET) tem as regras da pasta Mods.
    let n_lua = files.iter().filter(|f| f.kind == FileKind::Lua).count();
    if n_lua > 0 {
        notes.push(format!("{n_lua} .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll não."));
    }
    // `.tweak` aplica desde 2026-09-19 (runtime e gerenciador, pelo mesmo aplicador do YAML); o aviso de "não faz efeito" saiu.
    ModReport { name, class, compat, files, deps, risks, notes, avisos_do_xl }
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

    /// `apply-N6`: o tipo do tweak sai de `crate::formato_tweak`, a mesma regra do runtime.
    #[test]
    fn tweak_pela_regra_unica_de_extensao() {
        for rel in ["r6/tweaks/a/X.YAML", "r6/tweaks/a/x.yml", "r6/tweaks/a/Y.Tweak"] {
            assert_eq!(kind_of(Path::new(rel)), FileKind::Tweak, "{rel}");
        }
        for rel in ["r6/tweaks/a/z.toml", "red4ext/plugins/p/config.toml"] {
            assert_eq!(kind_of(Path::new(rel)), FileKind::Other, "{rel}");
        }
    }

    #[test]
    fn xl_com_factory_diz_que_o_jogo_aplica() {
        let g = std::env::temp_dir().join(format!("bwms-classify-xl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("MeuMod");
        touch(&m.join("x.archive"), "FAKE");
        touch(&m.join("mymod.xl"), "factories:\n  - mymod\\f.csv\n");

        let r = classify(&m);
        // detecta a dependência do ArchiveXL, e o detalhe diz quem aplica
        assert!(r.deps.iter().any(|d| d.name == "ArchiveXL" && d.detail.contains("o BWMS aplica")), "deps: {:?}", r.deps);
        // e a nota: o factories vai ao jogo pelo .json, sem tabela
        assert!(r.notes.iter().any(|n| n.contains("factories") && n.contains("o próprio jogo aplica")), "notes: {:?}", r.notes);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn xl_com_secao_fora_das_7_antigas_nao_diz_sem_operacoes() {
        // Uma seção por `.xl`, ao lado de um `.archive`: a nota cita a seção em vez de "sem operações de runtime".
        let casos = [
            ("quest.phases", "quest:\n  phases:\n    - path: mod\\fase.questphase\n      parent: base\\quest\\pai.questphase\n"),
            ("journal", "journal:\n  - mod\\quest\\a.journal\n"),
            ("streaming", "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 3\n      nodeDeletions:\n        - type: t\n          index: 1\n"),
            ("overrides.tags", "overrides:\n  tags:\n    minha_tag:\n      t0_000_pma_base__full: [ 1 ]\n"),
            ("customizations", "customizations:\n  male: mod\\opcoes.inkcharcustomization\n"),
            ("animations", "animations:\n  - entity: player_ma_tpp\n    set: mod\\anims.anims\n"),
            ("player.bodyTypes", "player:\n  bodyTypes: [ Corpo ]\n"),
            ("localization.extend", "localization:\n  extend: mod\\loc.json\n"),
        ];
        for (i, (secao, src)) in casos.iter().enumerate() {
            let xl = crate::instalacao::xl::parse_xl(src).unwrap();
            assert!(xl.secoes().iter().any(|s| s.chave == *secao && s.entradas > 0), "{secao}: o .xl de teste não populou a seção");
            let g = std::env::temp_dir().join(format!("bwms-classify-xl-secao-{}-{i}", std::process::id()));
            let _ = std::fs::remove_dir_all(&g);
            touch(&g.join("x.archive"), "FAKE");
            touch(&g.join("m.xl"), src);
            let r = classify(&g);
            assert!(r.notes.iter().any(|n| n.contains(secao)), "{secao}: {:?}", r.notes);
            assert!(!r.notes.iter().any(|n| n.contains("sem operações")), "{secao}: {:?}", r.notes);
            // a nota diz quem aplica pela lista do install: seção sem tabela não pode sair como aplicada
            let esperado = if SECOES_SEM_TABELA.contains(secao) { "ainda não aplica" } else { "runtime do BWMS aplica" };
            assert!(r.notes.iter().any(|n| n.contains(secao) && n.contains(esperado)), "{secao}: {:?}", r.notes);
            let _ = std::fs::remove_dir_all(&g);
        }
    }

    #[test]
    fn xl_misto_separa_secao_aplicada_da_sem_tabela() {
        let g = std::env::temp_dir().join(format!("bwms-classify-xl-misto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        touch(&g.join("x.archive"), "FAKE");
        touch(&g.join("m.xl"), "factories:\n  - m\\f.csv\nanimations:\n  - entity: player_ma_tpp\n    set: m\\a.anims\n");
        let r = classify(&g);
        assert_eq!(
            r.notes,
            [
                ".xl 'm.xl': factories ×1 → o próprio jogo aplica, pelo .json que o BWMS grava em archive/Mac/mod/BWMS-Mods/xl.",
                ".xl 'm.xl': animations ×1 → o BWMS ainda não aplica essas seções, que ficam sem efeito no jogo; o resto do mod carrega.",
            ]
        );
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn xl_malformado_vira_aviso_do_xl() {
        // fora de `notes`: o relatório da `Mods/` traz o mesmo motivo quando o runtime lê o `.xl`
        let g = std::env::temp_dir().join(format!("bwms-classify-xl-aviso-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        touch(&g.join("x.archive"), "FAKE");
        touch(&g.join("m.xl"), "streaming:\n  sectors:\n    - path: s.streamingsector\n");
        touch(&g.join("q.xl"), "---\nfactories: [ a.csv ]\n");
        let r = classify(&g);
        // a ordem dos `.xl` é a do `read_dir`
        assert_eq!(r.avisos_do_xl.len(), 2, "{:?}", r.avisos_do_xl);
        assert!(r.avisos_do_xl.iter().any(|a| a.starts_with(".xl 'm.xl': streaming.sectors[0] (s.streamingsector): sem expectedNodes")), "{:?}", r.avisos_do_xl);
        assert!(r.avisos_do_xl.iter().any(|a| a.starts_with(".xl 'q.xl' não parseou (") && a.ends_with("); corrija o YAML do arquivo")), "{:?}", r.avisos_do_xl);
        assert!(!r.notes.iter().any(|n| n.contains("sem expectedNodes") || n.contains("não parseou")), "notes: {:?}", r.notes);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn localizacao_que_o_runtime_nao_aplica_vira_aviso_do_xl_por_tipo() {
        // o mesmo texto que o relatório da `Mods/` dá, fora de `notes` pelo motivo do teste acima
        let g = std::env::temp_dir().join(format!("bwms-classify-xl-localizacao-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        touch(&g.join("x.archive"), "FAKE");
        touch(&g.join("m.xl"), "localization:\n  onscreens:\n    en-us: m\\on.json\n  vomaps:\n    en-us: m\\vo.json\n");
        let r = classify(&g);
        assert_eq!(
            r.avisos_do_xl,
            vec![
                ".xl 'm.xl': localization.vomaps: não aplicado pelo runtime do BWMS, que só aplica onscreens, e o grupo fica sem efeito no jogo; tire o \
                 grupo vomaps do localization deste .xl para o aviso sumir (o jogo fica igual)"
                    .to_string()
            ]
        );
        assert!(r.notes.iter().any(|n| n.contains("localization ×2")), "notes: {:?}", r.notes);
        assert!(!r.notes.iter().any(|n| n.contains("não aplicado")), "notes: {:?}", r.notes);
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

    fn risk_of<'a>(r: &'a ModReport, prefix: &str, rel: &str) -> Option<&'a String> {
        let alvo = format!("{prefix}: '{rel}'");
        r.risks.iter().find(|x| x.starts_with(&alvo))
    }

    #[cfg(unix)]
    #[test]
    fn symlink_entra_no_relatorio_bloqueia_e_nao_e_seguido() {
        let g = std::env::temp_dir().join(format!("bwms-classify-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModLink");
        touch(&m.join("a.archive"), "RDAR");
        // alvo fora do pacote com magic Mach-O: se o scan seguisse o link, viraria "BINÁRIO Mach-O"
        std::fs::create_dir_all(&g).unwrap();
        std::fs::write(g.join("fora.bin"), [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
        std::os::unix::fs::symlink(g.join("fora.bin"), m.join("b.archive")).unwrap();
        std::os::unix::fs::symlink(&g, m.join("dir")).unwrap();

        let r = classify(&m);
        let mut rels: Vec<String> = r.files.iter().map(|f| f.rel.display().to_string()).collect();
        rels.sort();
        assert_eq!(rels, ["a.archive", "b.archive", "dir"]);
        assert!(risk_of(&r, "SYMLINK", "b.archive").is_some(), "risks: {:?}", r.risks);
        assert!(risk_of(&r, "SYMLINK", "dir").is_some(), "risks: {:?}", r.risks);
        assert!(!r.risks.iter().any(|x| x.contains("Mach-O")), "seguiu o link: {:?}", r.risks);
        assert_eq!(r.risks.len(), 2, "risks: {:?}", r.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[cfg(unix)]
    #[test]
    fn fifo_vira_tipo_especial_sem_travar_o_scan() {
        let g = std::env::temp_dir().join(format!("bwms-classify-fifo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModFifo");
        touch(&m.join("a.archive"), "RDAR");
        let feito = std::process::Command::new("mkfifo").arg(m.join("pipe")).status();
        if !matches!(feito, Ok(s) if s.success()) {
            let _ = std::fs::remove_dir_all(&g);
            return; // sem mkfifo no sistema não há FIFO para testar
        }

        // open() num FIFO sem escritor bloqueia: o scan roda numa thread com prazo.
        let (tx, rx) = std::sync::mpsc::channel();
        let m2 = m.clone();
        std::thread::spawn(move || {
            let _ = tx.send(classify(&m2));
        });
        let r = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("classify travou abrindo o FIFO");
        assert!(r.files.iter().any(|f| f.rel == Path::new("pipe")), "files: {:?}", r.files);
        assert!(risk_of(&r, "TIPO ESPECIAL", "pipe").is_some(), "risks: {:?}", r.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn macho_com_extensao_de_conteudo_vira_nativo() {
        let g = std::env::temp_dir().join(format!("bwms-classify-macho-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModMascarado");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("fake.archive"), [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1]).unwrap();
        // FAT fica em big-endian no disco
        std::fs::write(m.join("fat.json"), [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]).unwrap();
        // menos de 4 bytes: não é Mach-O e não é erro de leitura
        touch(&m.join("curto.reds"), "x");

        let r = classify(&m);
        let kind = |n: &str| r.files.iter().find(|f| f.rel == Path::new(n)).map(|f| f.kind);
        assert_eq!(kind("fake.archive"), Some(FileKind::Native));
        assert_eq!(kind("fat.json"), Some(FileKind::Native));
        assert_eq!(kind("curto.reds"), Some(FileKind::Redscript));
        assert_eq!(r.class, ModClass::NativeCode);
        assert_eq!(r.compat, Compat::NativePortRequired);
        assert!(risk_of(&r, "BINÁRIO Mach-O MASCARADO", "fake.archive").is_some(), "risks: {:?}", r.risks);
        assert!(risk_of(&r, "BINÁRIO Mach-O MASCARADO", "fat.json").is_some(), "risks: {:?}", r.risks);
        assert_eq!(r.risks.len(), 2, "risks: {:?}", r.risks);

        // controle: .archive de verdade começa com RDAR e passa limpo
        let m2 = g.join("ModReal");
        touch(&m2.join("real.archive"), "RDAR\0\0\0\0");
        let r2 = classify(&m2);
        assert!(r2.risks.is_empty(), "risks: {:?}", r2.risks);
        assert_eq!(r2.class, ModClass::PureContent);
        assert_eq!(r2.compat, Compat::Universal);

        let _ = std::fs::remove_dir_all(&g);
    }

    const MACHO_64: [u8; 8] = [0xcf, 0xfa, 0xed, 0xfe, 0x0c, 0, 0, 1];

    #[test]
    fn macho_com_extensao_de_binario_mac_ou_sem_extensao_bloqueia() {
        let g = std::env::temp_dir().join(format!("bwms-classify-dylib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModPlugin");
        touch(&m.join("a.archive"), "RDAR");
        std::fs::write(m.join("plugin.dylib"), MACHO_64).unwrap();

        let r = classify(&m);
        assert!(risk_of(&r, "BINÁRIO Mach-O", "plugin.dylib").is_some(), "risks: {:?}", r.risks);
        assert_eq!(r.risks.len(), 1, "risks: {:?}", r.risks);
        assert_eq!(r.class, ModClass::NativeCode);

        // .so e sem extensão seguem a mesma régua; FAT fica em big-endian no disco. Quem decide é o magic, não a pasta:
        // `solto` fica fora de bin/ e de plugins/.
        let m2 = g.join("ModBinarios");
        std::fs::create_dir_all(m2.join("bin")).unwrap();
        std::fs::write(m2.join("libx.SO"), [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]).unwrap();
        std::fs::write(m2.join("bin/ferramenta"), MACHO_64).unwrap();
        std::fs::write(m2.join("solto"), MACHO_64).unwrap();
        let r2 = classify(&m2);
        assert!(risk_of(&r2, "BINÁRIO Mach-O", "libx.SO").is_some(), "risks: {:?}", r2.risks);
        assert!(risk_of(&r2, "BINÁRIO Mach-O", "bin/ferramenta").is_some(), "risks: {:?}", r2.risks);
        assert!(risk_of(&r2, "BINÁRIO Mach-O", "solto").is_some(), "risks: {:?}", r2.risks);
        assert_eq!(r2.risks.len(), 3, "risks: {:?}", r2.risks);
        assert!(r2.files.iter().all(|f| f.kind == FileKind::Native), "files: {:?}", r2.files);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn macho_com_extensao_windows_vira_mascarado() {
        let g = std::env::temp_dir().join(format!("bwms-classify-dll-macho-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModDllMacho");
        std::fs::create_dir_all(&m).unwrap();
        std::fs::write(m.join("x.dll"), MACHO_64).unwrap();
        std::fs::write(m.join("y.exe"), MACHO_64).unwrap();

        let r = classify(&m);
        assert!(risk_of(&r, "BINÁRIO Mach-O MASCARADO", "x.dll").is_some(), "risks: {:?}", r.risks);
        assert!(risk_of(&r, "BINÁRIO Mach-O MASCARADO", "y.exe").is_some(), "risks: {:?}", r.risks);
        assert_eq!(r.risks.len(), 2, "risks: {:?}", r.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn nativo_nao_mac_bloqueia_com_ou_sem_script() {
        let g = std::env::temp_dir().join(format!("bwms-classify-nao-mac-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        // o .reds declara uma native que só o .dll registra: instalar sem ele derruba o bind
        let m = g.join("ModDllReds");
        touch(&m.join("red4ext/plugins/Foo/Foo.dll"), "MZ\u{90}\0");
        touch(&m.join("red4ext/plugins/Foo/libfoo.so"), "\u{7f}ELF");
        touch(&m.join("r6/scripts/foo.reds"), "native func FooPing() -> Int32;\n");

        let r = classify(&m);
        assert!(risk_of(&r, "CÓDIGO NATIVO NÃO-MAC", "red4ext/plugins/Foo/Foo.dll").is_some(), "risks: {:?}", r.risks);
        assert!(risk_of(&r, "CÓDIGO NATIVO NÃO-MAC", "red4ext/plugins/Foo/libfoo.so").is_some(), "risks: {:?}", r.risks);
        assert_eq!(r.risks.len(), 2, "risks: {:?}", r.risks);

        // .dll esquecido num pacote só de conteúdo também bloqueia até o dono decidir a exceção
        let m2 = g.join("ModDllEsquecido");
        touch(&m2.join("a.archive"), "RDAR");
        touch(&m2.join("x.dll"), "MZ");
        let r2 = classify(&m2);
        assert!(risk_of(&r2, "CÓDIGO NATIVO NÃO-MAC", "x.dll").is_some(), "risks: {:?}", r2.risks);
        assert_eq!(r2.risks.len(), 1, "risks: {:?}", r2.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn redmod_com_dll_precisa_port_nativo_pela_dependencia() {
        // info.json vence o .dll no `class_of` (RedMod): só a dependência `nativo` leva o compat a NativePortRequired, então o nome
        // gravado no `detect_deps` e o casado no `compat_of` têm de ser o mesmo
        let g = std::env::temp_dir().join(format!("bwms-classify-redmod-dll-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        touch(&g.join("info.json"), "{}\n");
        touch(&g.join("x.dll"), "MZ");
        let r = classify(&g);
        assert_eq!(r.class, ModClass::RedMod);
        assert!(r.deps.iter().any(|d| d.name == "nativo"), "deps: {:?}", r.deps);
        assert_eq!(r.compat, Compat::NativePortRequired);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn lua_de_cet_sozinho_roda_pela_pasta_mods() {
        let g = std::env::temp_dir().join(format!("bwms-classify-cet-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModCet");
        touch(&m.join("bin/x64/plugins/cyber_engine_tweaks/mods/x/init.lua"), "registerForEvent('onInit', function() end)\n");

        let r = classify(&m);
        assert_eq!(r.class, ModClass::Script);
        assert_eq!(r.compat, Compat::MacAdapter);
        assert!(r.deps.iter().any(|d| d.name == "CET" && d.detail.contains("o que depende de .dll não")), "deps: {:?}", r.deps);
        assert!(r.risks.is_empty(), "risks: {:?}", r.risks);
        // `plan`/`install` não mostram compat: a nota é o único aviso de que `.dll` fica de fora
        assert_eq!(r.notes, [NOTA_1_LUA], "notes: {:?}", r.notes);

        let _ = std::fs::remove_dir_all(&g);
    }

    const NOTA_1_LUA: &str = "1 .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll não.";

    #[test]
    fn nota_de_lua_nao_promete_o_destino_do_resto() {
        let g = std::env::temp_dir().join(format!("bwms-classify-cet-misto-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let cet = "bin/x64/plugins/cyber_engine_tweaks/mods/addon";
        let reds = "public func Nucleo() -> Int32 { return 1; }\n";
        let m = g.join("ModRedsLua");
        touch(&m.join("r6/scripts/nucleo.reds"), reds);
        touch(&m.join(format!("{cet}/init.lua")), "return {}\n");
        touch(&m.join(format!("{cet}/ui.lua")), "return {}\n");

        let r = classify(&m);
        assert_eq!(r.class, ModClass::Script);
        assert_eq!(r.compat, Compat::MacAdapter);
        assert_eq!(r.notes, ["2 .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll não."]);
        assert!(r.risks.is_empty(), "risks: {:?}", r.risks);

        let m2 = g.join("ModArchiveLua");
        touch(&m2.join("a.archive"), "RDAR");
        touch(&m2.join(format!("{cet}/init.lua")), "return {}\n");
        let r2 = classify(&m2);
        assert_eq!(r2.class, ModClass::Mixed);
        assert_eq!(r2.compat, Compat::MacAdapter);
        assert_eq!(r2.notes, [NOTA_1_LUA], "notes: {:?}", r2.notes);

        // PE junto: o gate barra o pacote inteiro, e a nota não pode dizer que algo segue para o jogo
        let m3 = g.join("ModLuaComDll");
        touch(&m3.join("r6/scripts/a.reds"), reds);
        touch(&m3.join(format!("{cet}/init.lua")), "return {}\n");
        touch(&m3.join("red4ext/plugins/m/m.dll"), "MZ");
        let r3 = classify(&m3);
        assert_eq!(r3.class, ModClass::NativeCode);
        assert!(risk_of(&r3, "CÓDIGO NATIVO NÃO-MAC", "red4ext/plugins/m/m.dll").is_some(), "risks: {:?}", r3.risks);
        assert_eq!(r3.notes, [NOTA_1_LUA], "notes: {:?}", r3.notes);

        // .json companheiro do mod de CET também não vai ao jogo: a nota não conta o que se perde além do .lua
        let m4 = g.join("ModLuaComJson");
        touch(&m4.join("r6/scripts/a.reds"), reds);
        touch(&m4.join("mods/m/init.lua"), "return {}\n");
        touch(&m4.join("mods/m/settings.json"), "{}\n");
        let r4 = classify(&m4);
        assert_eq!(r4.class, ModClass::Mixed);
        assert!(r4.risks.is_empty(), "risks: {:?}", r4.risks);
        assert_eq!(r4.notes, [NOTA_1_LUA], "notes: {:?}", r4.notes);

        // controle: .reds sem .lua continua Mac Adapter e sem a nota
        let m5 = g.join("ModSoReds");
        touch(&m5.join("r6/scripts/nucleo.reds"), reds);
        let r5 = classify(&m5);
        assert_eq!(r5.compat, Compat::MacAdapter);
        assert!(r5.notes.is_empty(), "notes: {:?}", r5.notes);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[cfg(unix)]
    #[test]
    fn pasta_ou_arquivo_ilegivel_vira_nao_lido() {
        use std::os::unix::fs::PermissionsExt;
        let g = std::env::temp_dir().join(format!("bwms-classify-ilegivel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModFechado");
        touch(&m.join("ok.archive"), "RDAR");
        touch(&m.join("fechada/x.dll"), "MZ");
        touch(&m.join("trancado.archive"), "RDAR");
        let modo = |p: &Path, bits: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(bits)).unwrap();
        modo(&m.join("fechada"), 0o000);
        modo(&m.join("trancado.archive"), 0o000);
        // root lê mesmo com 000: aí não há o que testar
        let root_le = std::fs::read_dir(m.join("fechada")).is_ok() || std::fs::File::open(m.join("trancado.archive")).is_ok();

        let r = classify(&m);
        modo(&m.join("fechada"), 0o755);
        modo(&m.join("trancado.archive"), 0o644);
        if !root_le {
            assert!(risk_of(&r, "NÃO LIDO", "fechada").is_some(), "risks: {:?}", r.risks);
            assert!(risk_of(&r, "NÃO LIDO", "trancado.archive").is_some(), "risks: {:?}", r.risks);
            assert_eq!(r.risks.len(), 2, "risks: {:?}", r.risks);
        }

        // a própria raiz ilegível aparece pelo caminho dela, não por um rel vazio
        let sumida = g.join("nao-existe");
        let r2 = classify(&sumida);
        assert!(risk_of(&r2, "NÃO LIDO", &sumida.display().to_string()).is_some(), "risks: {:?}", r2.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn nome_com_barra_invertida_e_pontos_continua_barrado() {
        // `..\..\..\evil.yaml` é UM componente Normal no APFS; quem troca '\' por '/' (o caminho do
        // Windows) sai da raiz do jogo. Só a divisão do componente em '\' pega este caso.
        let g = std::env::temp_dir().join(format!("bwms-classify-trav-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModTrav");
        touch(&m.join("r6/tweaks/..\\..\\..\\evil.yaml"), "x: 1\n");

        let r = classify(&m);
        assert!(r.risks.iter().any(|x| x.starts_with("PATH-TRAVERSAL")), "risks: {:?}", r.risks);

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn escapes_root_pedaco_dotdot_e_componentes() {
        // pedaço `..` dentro de um Component::Normal: a estrutura sozinha não pega este nome
        let barra_invertida = Path::new("r6/tweaks/..\\..\\..\\evil.yaml");
        assert!(barra_invertida.components().all(|c| matches!(c, Component::Normal(_))));
        assert!(escapes_root(barra_invertida));
        assert!(escapes_root(Path::new("a\\..\\b.yaml")));
        // componente que não é Normal: `.` inicial não tem pedaço `..` e não é absoluto
        assert!(escapes_root(Path::new("./x.archive")));
        assert!(escapes_root(Path::new("../x.archive")));
        assert!(escapes_root(Path::new("a/../b.archive")));
        assert!(escapes_root(Path::new("/x.archive")));
        assert!(!escapes_root(Path::new("archive/pc/mod/x.archive")));
        assert!(!escapes_root(Path::new("r6/tweaks/meu.mod/x.yaml")));
        // `..` como parte do nome não é pedaço `..`
        assert!(!escapes_root(Path::new("archive/pc/mod/Jaqueta..v2.archive")));
        assert!(!escapes_root(Path::new("r6/tweaks/..meu\\x..\\y.yaml")));
    }

    #[test]
    fn pontos_no_nome_e_til_na_raiz_nao_viram_risco() {
        let g = std::env::temp_dir().join(format!("bwms-classify-nomes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = g.join("ModNomes");
        touch(&m.join("archive/pc/mod/Jaqueta..v2.archive"), "x");
        touch(&m.join("~readme.txt"), "leia\n");

        let r = classify(&m);
        assert!(r.risks.is_empty(), "risks: {:?}", r.risks);
        assert_eq!(r.files.len(), 2, "files: {:?}", r.files);

        let _ = std::fs::remove_dir_all(&g);
    }
}
