//! Parser do formato `.xl` do ArchiveXL (subconjunto YAML), zero-dep (só `std`).
//!
//! O `.xl` é um YAML que diz ao ArchiveXL o que fazer ao carregar um mod. Este módulo lê o
//! arquivo num modelo TIPADO (`XlFile`) fiel à fonte C++ do ArchiveXL
//! (`cp2077-archive-xl/src/App/Extensions/*/Config.cpp`). Cobre as seções NÚCLEO usadas pela
//! esmagadora maioria dos mods:
//!   - `factories:`            (FactoryIndex/Config.cpp)  — scalar OU lista de paths .csv
//!   - `resource.patch:`       (ResourcePatch/Config.cpp) — path → scalar | seq | {props,targets}, tag `!exclude`
//!   - `resource.link:`        (ResourceLink/Config.cpp)  — alvo → scalar | seq de fontes
//!   - `localization.{onscreens,subtitles,lipmaps,vomaps}` + `extend` (Localization/Config.cpp)
//!
//! Seções ainda NÃO tipadas (streaming, resource.copy, customNodes, garment, animation, ...) NÃO
//! são descartadas em silêncio: seus nomes de chave de topo entram em `XlFile::other_sections`,
//! pra a ferramenta poder avisar "tem coisa aqui que ainda não processo".
//!
//! O parser YAML é um subconjunto deliberado (o que `.xl` usa): mapas/sequências por indentação,
//! sequências em bloco (`- item`), sequências inline (`[ a, b ]`), tags de nó (`!exclude`),
//! comentários (`# ...`) e aspas opcionais. NÃO é um YAML completo (sem âncoras, multi-doc,
//! flow-maps, escapes de aspas) — de propósito.

use std::collections::HashMap;

// ============================ modelo YAML genérico ============================

/// Valor YAML do subconjunto. Ordem preservada nos mapas (chaves podem repetir, como no YAML).
#[derive(Debug, Clone, PartialEq)]
pub enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
    /// nó com tag, ex.: `!exclude [ a, b ]` → Tagged("exclude", Seq([...]))
    Tagged(String, Box<Yaml>),
}

impl Yaml {
    /// scalar (atravessa tag) → &str
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Yaml::Scalar(s) => Some(s),
            Yaml::Tagged(_, b) => b.as_str(),
            _ => None,
        }
    }
    /// sequência (atravessa tag)
    pub fn as_seq(&self) -> Option<&[Yaml]> {
        match self {
            Yaml::Seq(v) => Some(v),
            Yaml::Tagged(_, b) => b.as_seq(),
            _ => None,
        }
    }
    /// mapa (atravessa tag)
    pub fn as_map(&self) -> Option<&[(String, Yaml)]> {
        match self {
            Yaml::Map(m) => Some(m),
            Yaml::Tagged(_, b) => b.as_map(),
            _ => None,
        }
    }
    /// valor de uma chave no mapa (atravessa tag)
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        self.as_map()?.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    /// tag do nó, se houver (`!exclude` → "exclude")
    pub fn tag(&self) -> Option<&str> {
        match self {
            Yaml::Tagged(t, _) => Some(t),
            _ => None,
        }
    }
    /// coerção "um-ou-vários": scalar→[s], seq→[itens scalar]. (O padrão do `.xl`.)
    pub fn to_str_list(&self) -> Vec<String> {
        match self {
            Yaml::Scalar(s) => vec![s.clone()],
            Yaml::Seq(v) => v.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
            Yaml::Tagged(_, b) => b.to_str_list(),
            Yaml::Map(_) => vec![],
        }
    }
}

// ============================ parser (linhas + recursão) ============================

struct Line {
    indent: usize,
    text: String,
}

/// Tira comentário YAML: `#` inicia comentário se for início de linha (após indent) ou precedido
/// por espaço/tab. (Paths do `.xl` não têm espaço+`#`, então é seguro pra esse subconjunto.)
fn strip_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut prev_space = true; // início de linha conta como "após espaço"
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'#' && prev_space {
            return &line[..i];
        }
        prev_space = b == b' ' || b == b'\t';
    }
    line
}

/// Quebra em linhas significativas (sem comentário, sem brancas), com indent contado em espaços.
fn prelex(input: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in input.lines() {
        let no_comment = strip_comment(raw);
        let indent = no_comment.chars().take_while(|c| *c == ' ').count();
        let text = no_comment[indent..].trim_end();
        if text.is_empty() {
            continue;
        }
        out.push(Line { indent, text: text.to_string() });
    }
    out
}

/// tira aspas simples/duplas externas, se houver.
fn unquote(s: &str) -> String {
    let s = s.trim();
    let b = s.as_bytes();
    if s.len() >= 2 && ((b[0] == b'"' && b[s.len() - 1] == b'"') || (b[0] == b'\'' && b[s.len() - 1] == b'\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// "chave: resto" → (chave, resto). A chave termina no PRIMEIRO `:` seguido de espaço ou fim.
/// (Paths do `.xl` usam `\`/`.`, nunca `:`, então não há ambiguidade.)
fn split_key(s: &str) -> Option<(String, String)> {
    let bytes = s.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b':' && (i + 1 == bytes.len() || bytes[i + 1] == b' ') {
            let key = unquote(s[..i].trim());
            let rest = s[i + 1..].trim().to_string();
            return Some((key, rest));
        }
    }
    None
}

/// valor inline (na mesma linha): tag `!x`, flow-seq `[..]`, ou scalar.
fn parse_inline(s: &str) -> Result<Yaml, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('!') {
        let mut parts = rest.splitn(2, ' ');
        let tag = parts.next().unwrap_or("").to_string();
        let val = parts.next().unwrap_or("").trim();
        let inner = if val.is_empty() { Yaml::Scalar(String::new()) } else { parse_inline(val)? };
        return Ok(Yaml::Tagged(tag, Box::new(inner)));
    }
    if s.starts_with('[') {
        return parse_flow_seq(s);
    }
    Ok(Yaml::Scalar(unquote(s)))
}

/// `[ a, b, c ]` / `[]` → Seq de scalars (sem aninhamento — `.xl` não usa).
fn parse_flow_seq(s: &str) -> Result<Yaml, String> {
    let inner = s
        .strip_prefix('[')
        .and_then(|x| x.strip_suffix(']'))
        .ok_or_else(|| format!("sequência inline malformada: '{s}'"))?;
    let inner = inner.trim();
    if inner.is_empty() {
        return Ok(Yaml::Seq(vec![]));
    }
    Ok(Yaml::Seq(inner.split(',').map(|p| Yaml::Scalar(unquote(p.trim()))).collect()))
}

/// Estado do parser recursivo. Carrega o registro de ÂNCORAS (`&nome`) p/ resolver ALIASES
/// (`*nome`) — usados de verdade nos `.xl` de customização (EyesFix/BrowsFix/LashesFix).
struct Parser {
    lines: Vec<Line>,
    pos: usize,
    anchors: HashMap<String, Yaml>,
}

impl Parser {
    fn parse_block(&mut self, indent: usize) -> Result<Yaml, String> {
        if self.pos >= self.lines.len() {
            return Ok(Yaml::Scalar(String::new()));
        }
        if self.lines[self.pos].text.starts_with('-') {
            self.parse_seq(indent)
        } else {
            self.parse_map(indent)
        }
    }

    fn parse_map(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut entries = Vec::new();
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(format!("indentação inesperada em '{}'", line.text));
            }
            let text = line.text.clone();
            let (key, rest) = split_key(&text).ok_or_else(|| format!("esperava 'chave:' em '{text}'"))?;
            self.pos += 1;
            let val = self.parse_value(&rest, indent)?;
            entries.push((key, val));
        }
        Ok(Yaml::Map(entries))
    }

    fn parse_seq(&mut self, indent: usize) -> Result<Yaml, String> {
        let mut items = Vec::new();
        while self.pos < self.lines.len() {
            let line = &self.lines[self.pos];
            if line.indent < indent {
                break;
            }
            if line.indent > indent {
                return Err(format!("indentação inesperada em '{}'", line.text));
            }
            if !line.text.starts_with('-') {
                break; // fim da sequência (uma chave de mapa no mesmo indent)
            }
            let after = line.text[1..].trim_start().to_string();
            self.pos += 1;
            if after.is_empty() {
                items.push(self.parse_nested_or_empty(indent)?);
            } else if after.starts_with('&') || after.starts_with('*') || after.starts_with('[') || after.starts_with('!') {
                items.push(self.parse_value(&after, indent)?);
            } else if let Some((k, r)) = split_key(&after) {
                // "- chave: ..." → item-mapa; chaves seguintes mais indentadas pertencem a ele
                let mut map = Vec::new();
                let v = self.parse_value(&r, indent)?;
                map.push((k, v));
                while self.pos < self.lines.len()
                    && self.lines[self.pos].indent > indent
                    && !self.lines[self.pos].text.starts_with('-')
                {
                    let l2t = self.lines[self.pos].text.clone();
                    let li = self.lines[self.pos].indent;
                    let (k2, r2) = split_key(&l2t).ok_or_else(|| format!("esperava chave em '{l2t}'"))?;
                    self.pos += 1;
                    let v2 = self.parse_value(&r2, li)?;
                    map.push((k2, v2));
                }
                items.push(Yaml::Map(map));
            } else {
                items.push(parse_inline(&after)?);
            }
        }
        Ok(Yaml::Seq(items))
    }

    /// Resolve o valor de uma chave/item a partir do `rest` inline + bloco que segue.
    /// Trata aliases (`*nome`), âncoras (`&nome [valor]`), tags/flow/scalar inline e bloco aninhado.
    fn parse_value(&mut self, rest: &str, indent: usize) -> Result<Yaml, String> {
        let rest = rest.trim();
        if let Some(name) = rest.strip_prefix('*') {
            let name = name.trim();
            return self
                .anchors
                .get(name)
                .cloned()
                .ok_or_else(|| format!("alias *{name} sem âncora correspondente"));
        }
        if let Some(after) = rest.strip_prefix('&') {
            let mut it = after.splitn(2, ' ');
            let name = it.next().unwrap_or("").trim().to_string();
            let inline = it.next().unwrap_or("").trim();
            let val = if inline.is_empty() {
                self.parse_nested_or_empty(indent)?
            } else {
                parse_inline(inline)?
            };
            self.anchors.insert(name, val.clone());
            return Ok(val);
        }
        if !rest.is_empty() {
            return parse_inline(rest);
        }
        self.parse_nested_or_empty(indent)
    }

    /// valor de uma chave sem inline: bloco mais indentado, seq em bloco no mesmo indent, ou vazio.
    fn parse_nested_or_empty(&mut self, indent: usize) -> Result<Yaml, String> {
        if self.pos < self.lines.len() && self.lines[self.pos].indent > indent {
            let child = self.lines[self.pos].indent;
            self.parse_block(child)
        } else if self.pos < self.lines.len()
            && self.lines[self.pos].indent == indent
            && self.lines[self.pos].text.starts_with('-')
        {
            self.parse_seq(indent)
        } else {
            Ok(Yaml::Scalar(String::new()))
        }
    }
}

/// Parseia um documento YAML (subconjunto `.xl`) na árvore genérica `Yaml`.
pub fn parse_yaml(input: &str) -> Result<Yaml, String> {
    let input = input.replace('\t', "  ");
    let lines = prelex(&input);
    if lines.is_empty() {
        return Ok(Yaml::Map(vec![]));
    }
    let base = lines[0].indent;
    let mut p = Parser { lines, pos: 0, anchors: HashMap::new() };
    let v = p.parse_block(base)?;
    if p.pos != p.lines.len() {
        return Err(format!("conteúdo não consumido a partir de '{}'", p.lines[p.pos].text));
    }
    Ok(v)
}

// ============================ modelo tipado do .xl ============================

/// Um patch de recurso: estampa `props` do recurso `patch` nos recursos-alvo `includes`
/// (ou os remove via `excludes`, tag `!exclude`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourcePatch {
    pub patch: String,
    pub props: Vec<String>,
    pub includes: Vec<String>,
    pub excludes: Vec<String>,
}

/// Um link de recurso: o `target` (path virtual) resolve para as `sources` (paths reais).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceLink {
    pub target: String,
    pub sources: Vec<String>,
}

/// Um escopo de recurso (`resource.scope`): o recurso `resource` define o ESCOPO dos `targets`
/// (ResourceMeta/Config.cpp `LoadScopes`). Alvos = scalar OU lista.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceScope {
    pub resource: String,
    pub targets: Vec<String>,
}

/// Uma cópia de recurso (`resource.copy`): copia `source` para cada `targets`
/// (ResourceLink/Config.cpp). Alvos = scalar OU lista.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceCopy {
    pub source: String,
    pub targets: Vec<String>,
}

/// Um conserto de recurso (`resource.fix`): no recurso `resource`, remapeia nomes (`names`:
/// nome→nome), paths (`paths`: path→path) e parâmetros de contexto (`context`: nome→valor).
/// (ResourceMeta/Config.cpp `LoadFixes`.)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceFix {
    pub resource: String,
    pub names: Vec<(String, String)>,
    pub paths: Vec<(String, String)>,
    pub context: Vec<(String, String)>,
}

/// Um grupo de localização (onscreens/subtitles/lipmaps/vomaps): idioma → paths.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalizationGroup {
    pub kind: String,
    pub entries: Vec<(String, Vec<String>)>,
}

/// O `.xl` inteiro, tipado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XlFile {
    pub factories: Vec<String>,
    pub patches: Vec<ResourcePatch>,
    pub links: Vec<ResourceLink>,
    pub scopes: Vec<ResourceScope>,
    pub copies: Vec<ResourceCopy>,
    pub fixes: Vec<ResourceFix>,
    pub localization: Vec<LocalizationGroup>,
    pub localization_extend: Option<String>,
    /// chaves de topo presentes mas ainda não tipadas (streaming, customNodes, ...). Nada some.
    pub other_sections: Vec<String>,
}

/// Lê um `.xl` (texto) no modelo tipado.
pub fn parse_xl(input: &str) -> Result<XlFile, String> {
    let doc = parse_yaml(input)?;
    let map = doc.as_map().ok_or("documento .xl não é um mapa no topo")?;
    let mut xl = XlFile::default();
    for (key, val) in map {
        match key.as_str() {
            "factories" => xl.factories = val.to_str_list(),
            "resource" => parse_resource(val, &mut xl),
            "localization" => parse_localization(val, &mut xl),
            other => xl.other_sections.push(other.to_string()),
        }
    }
    Ok(xl)
}

/// map `chave → valor` (string→string), p/ names/paths/context do `resource.fix`.
fn str_pairs(node: Option<&Yaml>) -> Vec<(String, String)> {
    node.and_then(|n| n.as_map())
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

fn parse_resource(node: &Yaml, xl: &mut XlFile) {
    const TYPED: [&str; 5] = ["patch", "link", "scope", "copy", "fix"];
    // sub-chaves de `resource` ainda não tipadas não somem em silêncio
    if let Some(map) = node.as_map() {
        for (k, _) in map {
            if !TYPED.contains(&k.as_str()) {
                xl.other_sections.push(format!("resource.{k}"));
            }
        }
    }
    // scope: recurso → alvos (scalar|seq)
    if let Some(scope) = node.get("scope").and_then(|s| s.as_map()) {
        for (path, targets) in scope {
            xl.scopes.push(ResourceScope { resource: path.clone(), targets: targets.to_str_list() });
        }
    }
    // copy: source → alvos (scalar|seq)
    if let Some(copy) = node.get("copy").and_then(|c| c.as_map()) {
        for (src, targets) in copy {
            xl.copies.push(ResourceCopy { source: src.clone(), targets: targets.to_str_list() });
        }
    }
    // fix: recurso → { names, paths, context } (cada um map string→string)
    if let Some(fix) = node.get("fix").and_then(|f| f.as_map()) {
        for (res, def) in fix {
            xl.fixes.push(ResourceFix {
                resource: res.clone(),
                names: str_pairs(def.get("names")),
                paths: str_pairs(def.get("paths")),
                context: str_pairs(def.get("context")),
            });
        }
    }
    if let Some(patch) = node.get("patch").and_then(|p| p.as_map()) {
        for (path, def) in patch {
            let mut p = ResourcePatch { patch: path.clone(), ..Default::default() };
            if def.as_map().is_some() {
                // forma mapa: { props: [..], targets: [..] }
                if let Some(props) = def.get("props") {
                    p.props = props.to_str_list();
                }
                if let Some(targets) = def.get("targets") {
                    let list = targets.to_str_list();
                    if targets.tag() == Some("exclude") {
                        p.excludes = list;
                    } else {
                        p.includes = list;
                    }
                }
            } else if def.as_seq().is_some() {
                // forma sequência: lista de alvos (include, ou exclude via tag)
                let list = def.to_str_list();
                if def.tag() == Some("exclude") {
                    p.excludes = list;
                } else {
                    p.includes = list;
                }
            } else if let Some(s) = def.as_str() {
                // forma scalar: um alvo
                p.includes.push(s.to_string());
            }
            if !p.includes.is_empty() || !p.excludes.is_empty() || !p.props.is_empty() {
                xl.patches.push(p);
            }
        }
    }
    if let Some(link) = node.get("link").and_then(|l| l.as_map()) {
        for (target, sources) in link {
            xl.links.push(ResourceLink { target: target.clone(), sources: sources.to_str_list() });
        }
    }
}

fn parse_localization(node: &Yaml, xl: &mut XlFile) {
    for kind in ["onscreens", "subtitles", "lipmaps", "vomaps"] {
        if let Some(grp) = node.get(kind).and_then(|g| g.as_map()) {
            let entries: Vec<(String, Vec<String>)> =
                grp.iter().map(|(lang, paths)| (lang.clone(), paths.to_str_list())).collect();
            if !entries.is_empty() {
                xl.localization.push(LocalizationGroup { kind: kind.to_string(), entries });
            }
        }
    }
    if let Some(ext) = node.get("extend").and_then(|e| e.as_str()) {
        xl.localization_extend = Some(ext.to_string());
    }
}

impl XlFile {
    /// Resumo legível pra CLI.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("factories: {}\n", self.factories.len()));
        for f in &self.factories {
            s.push_str(&format!("  - {f}\n"));
        }
        s.push_str(&format!("resource.patch: {}\n", self.patches.len()));
        for p in &self.patches {
            s.push_str(&format!(
                "  {} → +{} alvo(s), -{} excluído(s), props: [{}]\n",
                p.patch, p.includes.len(), p.excludes.len(), p.props.join(", ")
            ));
        }
        s.push_str(&format!("resource.link: {}\n", self.links.len()));
        for l in &self.links {
            s.push_str(&format!("  {} → {} fonte(s)\n", l.target, l.sources.len()));
        }
        s.push_str(&format!("resource.scope: {}\n", self.scopes.len()));
        for sc in &self.scopes {
            s.push_str(&format!("  {} → {} alvo(s)\n", sc.resource, sc.targets.len()));
        }
        s.push_str(&format!("resource.copy: {}\n", self.copies.len()));
        for c in &self.copies {
            s.push_str(&format!("  {} → {} cópia(s)\n", c.source, c.targets.len()));
        }
        s.push_str(&format!("resource.fix: {}\n", self.fixes.len()));
        for f in &self.fixes {
            s.push_str(&format!(
                "  {} → {} nome(s), {} path(s), {} contexto(s)\n",
                f.resource, f.names.len(), f.paths.len(), f.context.len()
            ));
        }
        s.push_str(&format!("localization: {} grupo(s)\n", self.localization.len()));
        for g in &self.localization {
            s.push_str(&format!("  {}: {} idioma(s)\n", g.kind, g.entries.len()));
        }
        if let Some(ext) = &self.localization_extend {
            s.push_str(&format!("localization.extend: {ext}\n"));
        }
        if !self.other_sections.is_empty() {
            s.push_str(&format!("⚠ seções ainda não processadas: {}\n", self.other_sections.join(", ")));
        }
        s
    }
}

// ============================ testes (a PROVA, offline) ============================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_basico_mapa_seq_flow() {
        let y = parse_yaml("a: 1\nb:\n  - x\n  - y\nc: [ p, q ]\n").unwrap();
        assert_eq!(y.get("a").unwrap().as_str(), Some("1"));
        assert_eq!(y.get("b").unwrap().to_str_list(), vec!["x", "y"]);
        assert_eq!(y.get("c").unwrap().to_str_list(), vec!["p", "q"]);
    }

    #[test]
    fn comentarios_e_brancas() {
        let y = parse_yaml("# topo\nfactories:\n  - a.csv   # inline\n\n  - b.csv\n").unwrap();
        assert_eq!(y.get("factories").unwrap().to_str_list(), vec!["a.csv", "b.csv"]);
    }

    // ---- exemplos REAIS do ArchiveXL (cp2077-archive-xl/.../resources) ----

    #[test]
    fn template_factories_e_localization() {
        let src = "factories:\n  - mymod\\factories\\clothing.csv\n  - mymod\\factories\\weapons.csv\nlocalization:\n  onscreens:\n    en-us: mymod\\localization\\en-us.json\n    de-de: mymod\\localization\\de-de.json\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.factories, vec!["mymod\\factories\\clothing.csv", "mymod\\factories\\weapons.csv"]);
        assert_eq!(xl.localization.len(), 1);
        let g = &xl.localization[0];
        assert_eq!(g.kind, "onscreens");
        assert_eq!(g.entries.len(), 2);
        assert_eq!(g.entries[0], ("en-us".to_string(), vec!["mymod\\localization\\en-us.json".to_string()]));
        assert!(xl.other_sections.is_empty());
    }

    #[test]
    fn patch_real_hairpatch() {
        // PlayerCustomizationHairPatch.xl (forma mapa: props + targets)
        let src = "resource:\n  patch:\n    archive_xl\\characters\\common\\hair\\h1_base_color_patch.mesh:\n      props: [ appearances ]\n      targets: [ player_ma_hair.mesh, player_wa_hair.mesh ]\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.patches.len(), 1);
        let p = &xl.patches[0];
        assert_eq!(p.patch, "archive_xl\\characters\\common\\hair\\h1_base_color_patch.mesh");
        assert_eq!(p.props, vec!["appearances"]);
        assert_eq!(p.includes, vec!["player_ma_hair.mesh", "player_wa_hair.mesh"]);
        assert!(p.excludes.is_empty());
    }

    #[test]
    fn link_real_migration() {
        // Migration.xl (resource.link: alvo → [fontes])
        let src = "resource:\n  link:\n    archive_xl\\a\\h1.mesh:\n      - archive_xl\\a\\base.mesh\n    archive_xl\\b\\head.app:\n      - archive_xl\\b\\lashes.app\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.links.len(), 2);
        assert_eq!(xl.links[0].target, "archive_xl\\a\\h1.mesh");
        assert_eq!(xl.links[0].sources, vec!["archive_xl\\a\\base.mesh"]);
        assert_eq!(xl.links[1].sources, vec!["archive_xl\\b\\lashes.app"]);
    }

    // ---- formas alternativas da fonte C++ ----

    #[test]
    fn patch_scalar_e_seq_e_exclude() {
        // scalar: 1 alvo
        let xl = parse_xl("resource:\n  patch:\n    a.mesh: b.mesh\n").unwrap();
        assert_eq!(xl.patches[0].includes, vec!["b.mesh"]);

        // sequência com tag !exclude → vai pra excludes
        let xl2 = parse_xl("resource:\n  patch:\n    a.mesh: !exclude [ x.mesh, y.mesh ]\n").unwrap();
        assert_eq!(xl2.patches[0].excludes, vec!["x.mesh", "y.mesh"]);
        assert!(xl2.patches[0].includes.is_empty());
    }

    #[test]
    fn factories_scalar_unico() {
        let xl = parse_xl("factories: mymod\\one.csv\n").unwrap();
        assert_eq!(xl.factories, vec!["mymod\\one.csv"]);
    }

    #[test]
    fn secao_desconhecida_vai_pra_other_sections() {
        let xl = parse_xl("streaming:\n  sectors:\n    - s.streamingsector\ncustomNode: x\n").unwrap();
        assert!(xl.other_sections.contains(&"streaming".to_string()));
        assert!(xl.other_sections.contains(&"customNode".to_string()));
    }

    #[test]
    fn ancora_e_alias() {
        // padrão real do EyesFix/BrowsFix: define com &nome, reusa com *nome
        let src = "resource:\n  fix:\n    a.ink: &Fix\n      paths:\n        x.app: y.app\n    b.ink: *Fix\n";
        // o YAML genérico resolve o alias pro MESMO nó da âncora
        let doc = parse_yaml(src).unwrap();
        let fix = doc.get("resource").unwrap().get("fix").unwrap();
        let a = fix.get("a.ink").unwrap();
        let b = fix.get("b.ink").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.get("paths").unwrap().get("x.app").unwrap().as_str(), Some("y.app"));
        // e o modelo tipado captura os dois fixes (alias expandido), via resource.fix.paths
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.fixes.len(), 2);
        assert_eq!(xl.fixes[0].paths, vec![("x.app".to_string(), "y.app".to_string())]);
        assert_eq!(xl.fixes[1].paths, vec![("x.app".to_string(), "y.app".to_string())]);
        assert!(!xl.other_sections.contains(&"resource.fix".to_string()));
    }

    #[test]
    fn scope_copy_fix_tipados() {
        // scope: recurso → alvos (scalar e seq)
        let xl = parse_xl("resource:\n  scope:\n    a.app: b.app\n    c.app:\n      - d.app\n      - e.app\n").unwrap();
        assert_eq!(xl.scopes.len(), 2);
        assert_eq!(xl.scopes[0].targets, vec!["b.app"]);
        assert_eq!(xl.scopes[1].targets, vec!["d.app", "e.app"]);

        // copy: source → alvos
        let xl2 = parse_xl("resource:\n  copy:\n    src.mesh:\n      - t1.mesh\n      - t2.mesh\n").unwrap();
        assert_eq!(xl2.copies.len(), 1);
        assert_eq!(xl2.copies[0].source, "src.mesh");
        assert_eq!(xl2.copies[0].targets, vec!["t1.mesh", "t2.mesh"]);

        // fix: names + paths + context
        let src = "resource:\n  fix:\n    target.mesh:\n      names:\n        old_mat: new_mat\n      paths:\n        old.app: new.app\n      context:\n        param: value\n";
        let xl3 = parse_xl(src).unwrap();
        assert_eq!(xl3.fixes.len(), 1);
        let f = &xl3.fixes[0];
        assert_eq!(f.resource, "target.mesh");
        assert_eq!(f.names, vec![("old_mat".to_string(), "new_mat".to_string())]);
        assert_eq!(f.paths, vec![("old.app".to_string(), "new.app".to_string())]);
        assert_eq!(f.context, vec![("param".to_string(), "value".to_string())]);
        assert!(xl3.other_sections.is_empty());
    }

    #[test]
    fn localization_multi_grupo_e_extend() {
        let src = "localization:\n  onscreens:\n    en-us: a.json\n  subtitles:\n    en-us: b.json\n  extend: base\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.localization.len(), 2);
        assert_eq!(xl.localization_extend, Some("base".to_string()));
    }
}
