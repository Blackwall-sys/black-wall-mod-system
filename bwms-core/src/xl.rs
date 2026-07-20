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

/// Um link de recurso. **Direção depende da FORMA no YAML (achado 2026-07-13, RE de
/// `ResourceLink/Config.cpp` + `Extension.cpp` real do ArchiveXL — não é uma escolha nossa, é
/// assimetria genuína no C++ upstream):**
/// - forma SCALAR (`target: source`): `target` (fake) resolve pra `source` (real) — 1 fonte.
/// - forma SEQUÊNCIA (`target: [source1, source2, ...]`): **INVERTIDO** — CADA item da lista
///   resolve pro `target` (o padrão real de uso, confirmado no fixture `Migration.xl`: o
///   `target` é o path NOVO/real, e a lista contém nomes ANTIGOS/legados que devem redirecionar
///   pra ele — é o caso de uso de MIGRAÇÃO, não "várias fontes candidatas pro mesmo alvo").
/// Rastreado via `is_sequence_form` (setado no parse, consumido em `apply_xl::build_apply_plan`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceLink {
    pub target: String,
    pub sources: Vec<String>,
    pub is_sequence_form: bool,
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

/// `player.bodyTypes` (`PuppetState/Config.cpp`): lista de nomes de tipo de corpo (scalar OU seq).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PuppetStateConfig {
    pub body_types: Vec<String>,
}

/// `customizations.{male,female}` (`Customization/Config.cpp`): paths de opção de customização
/// por gênero (cada um scalar OU seq — `ReadOptions` idêntica pros dois).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomizationConfig {
    pub male_options: Vec<String>,
    pub female_options: Vec<String>,
}

/// Uma entrada de `animations[]` (`Animation/Config.cpp::AnimationEntry`). `entity`: nomes de
/// entidade-alvo (scalar OU seq — **achado de RE: o C++ upstream tem um bug de copy-paste,
/// `else if (entityNode.IsScalar())` deveria ser `IsSequence()`, então a forma-lista NUNCA
/// preenche no binário real; implementamos CORRETO aqui pro nosso parser ter valor prático,
/// documentando a divergência**). `set`: nome do anim set (obrigatório). `vars`: **outro bug
/// upstream** (`if (variablesNode.IsScalar())` checa o nó ERRADO — o pai, não o item do loop —
/// `variables` fica SEMPRE vazio no binário real; idem, implementamos correto). `priority`
/// (default 128) e `component` (default "root") opcionais.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEntry {
    pub entities: Vec<String>,
    pub set: String,
    pub variables: Vec<String>,
    pub priority: u8,
    pub component: String,
}

impl Default for AnimationEntry {
    fn default() -> Self {
        AnimationEntry { entities: vec![], set: String::new(), variables: vec![], priority: 128, component: "root".to_string() }
    }
}

/// Vetor 3D (`scale`, sempre `[x, y, z]` — exatamente 3 floats).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Vetor 4D (`position`). **Achado de RE:** o `.W` final é SEMPRE forçado a `0` no C++ real
/// (`WorldStreaming/Config.cpp`), mesmo quando a lista YAML tem 4 valores — o 4º valor (índice 3)
/// é lido em `positionValues[3]` mas NUNCA usado; só serve pra decidir se a forma de 4 é aceita
/// (nó de sector aceita 3 OU 4 valores; sub-node exige EXATAMENTE 4). Replicado aqui fielmente.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// Quaternion (`orientation`, sempre `[i, j, k, r]` — exatamente 4 floats, ordem i/j/k/r do RED4).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Quat {
    pub i: f32,
    pub j: f32,
    pub k: f32,
    pub r: f32,
}

/// Mutação de um SUB-node (`actorMutations`/`instanceMutations` dentro de um `nodeMutations[]`;
/// `WorldStreaming/Config.cpp::ParseSubMutations`). Exige `expectedActors`/`expectedInstances`
/// (a contagem esperada) presente e válido, senão a lista inteira é ignorada (fiel ao C++: sem
/// count válido, `ParseSubMutations` devolve `false` cedo e NADA é lido). `position` aqui exige
/// EXATAMENTE 4 valores (≠ do node-level, que aceita 3 ou 4).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSubNodeMutation {
    pub sub_node_index: i64,
    pub position: Option<Vec4>,
    pub orientation: Option<Quat>,
    pub scale: Option<Vec3>,
}

/// Mutação de um node de streaming sector (`nodeMutations[]`, `WorldStreaming/Config.cpp`).
/// `resource_path`/`appearance_name`/`record_id` cada um lido de VÁRIAS chaves-sinônimo (a última
/// presente vence — `resource`/`mesh`/`meshRef`/`material`/`effect`/`entityTemplate` pro path;
/// `appearance`/`appearanceName`/`meshAppearance` pro nome; `recordID`/`recordId`/`objectRecordId`
/// pro TweakDBID). `sub_node_mutations` vem de `actorMutations` OU `instanceMutations` (mutuamente
/// exclusivos na prática — o 2º chamado, se presente e válido, sobrescreve).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldNodeMutation {
    pub node_index: i64,
    pub node_type: String,
    pub position: Option<Vec4>,
    pub orientation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub resource_path: Option<String>,
    pub appearance_name: Option<String>,
    pub record_id: Option<String>,
    pub nb_nodes_under_proxy_diff: Option<i32>,
    pub expected_sub_nodes: i64,
    pub sub_node_mutations: Vec<WorldSubNodeMutation>,
}

/// Deleção de um node de streaming sector (`nodeDeletions[]`). `sub_node_deletions` vem de
/// `actorDeletions` OU `instanceDeletions` (mesmo padrão sinônimo-sobrescreve de `WorldNodeMutation`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldNodeDeletion {
    pub node_index: i64,
    pub node_type: String,
    pub expected_sub_nodes: i64,
    pub sub_node_deletions: Vec<i64>,
}

/// Um streaming sector modificado (`streaming.sectors[]`). `expected_nodes` é obrigatório e usado
/// como LIMITE de sanidade pros índices de `nodeIndex` (fora do range = descartado, fiel ao C++).
/// Uma entrada só é mantida se tiver PELO MENOS 1 deleção ou 1 mutação (senão é ruído).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSectorMod {
    pub path: String,
    pub expected_nodes: i64,
    pub node_deletions: Vec<WorldNodeDeletion>,
    pub node_mutations: Vec<WorldNodeMutation>,
}

/// `streaming.{blocks,sectors}` (`WorldStreaming/Config.cpp`) — o 7º e último item da lista
/// original de "11 seções" do parser `.xl` (as outras 4 citadas — Attachment/Mesh/Transmog/
/// InkSpawner — não têm `Config.cpp`/seção própria nenhuma, RE 2026-07-15 cont.60).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldStreamingSection {
    pub blocks: Vec<String>,
    pub sectors: Vec<WorldSectorMod>,
}

/// Máscara de chunks de mesh (`Garment/ChunkMask.hpp`, RE 2026-07-15): decide quais índices de
/// chunk do componente aparecem. `show=true` = seleção POSITIVA (a máscara final tem bit setado
/// exatamente nos chunks listados, forma `show: [...]`, sem inversão). `show=false` = seleção por
/// EXCLUSÃO (a máscara final é o COMPLEMENTO dos chunks listados — forma `hide: [...]`, sequência
/// nua, OU escalar cru já-computado — replica `ChunkMask::Set` bit a bit: OR dos `1<<chunk`, depois
/// `mask = ~mask` se `!show && mask != 0`). Confirma e generaliza o achado da RE do full-body:
/// `chunkMask=0xFFFFFFFFFFFFFF1F` em `t0_000_pwa_fpp__01_ca_pale` = `hide: [5, 6, 7]` por esta
/// fórmula exata (`~((1<<5)|(1<<6)|(1<<7)) = 0xFFFFFFFFFFFFFF1F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkMask {
    pub show: bool,
    pub mask: u64,
}

impl ChunkMask {
    /// Constrói a partir de uma lista de índices de chunk (0-63), replicando `ChunkMask::Set`.
    fn from_chunks(show: bool, chunks: &[u8]) -> Self {
        let mut mask: u64 = 0;
        for &c in chunks {
            mask |= 1u64 << (c & 63);
        }
        if !show && mask != 0 {
            mask = !mask;
        }
        ChunkMask { show, mask }
    }
}

/// Uma conexão de nó do quest graph (`node`+`socket`, ou só `node` — `QuestPhase/Config.cpp::
/// FillConnection`): forma mapa `{node:[..], socket: nome}` OU sequência nua (só node path, sem
/// socket).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestPhaseConnection {
    pub node_path: Vec<u16>,
    pub socket: Option<String>,
}

/// Uma phase de quest (`quest.phases[]`, `QuestPhase/Config.cpp`): `path`+`parent` obrigatórios
/// (senão a entrada é descartada); `connection`/`input` alimentam o MESMO campo `input` (o C++
/// chama `FillConnection` duas vezes seguidas pro mesmo destino — "connection" é sinônimo/alias
/// de "input", o 2º chamado sobrescreve se ambos existirem); `output` e `intercept` (bool) opcionais.
/// `parent` é uma STRING só (não lista): o C++ guarda num `Set` mas cada parse de UM phase-node só
/// insere UM scalar — o merge entre vários mods pro mesmo `path` acontece rio-acima, fora do escopo
/// de parsear um `.xl` isolado.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestPhaseMod {
    pub phase_path: String,
    pub parent: String,
    pub input: QuestPhaseConnection,
    pub output: QuestPhaseConnection,
    pub intercept: bool,
}

/// Um override de tag de garment (`overrides.tags.<tag>.<component>` → máscara de chunks;
/// `Garment/Config.cpp::GarmentOverrideConfig::LoadYAML` — a chave de TOPO real é `overrides`,
/// não `garment` — `GarmentOverrideConfig::LoadYAML` lê `aNode["overrides"]["tags"]` direto da
/// raiz do documento, confirmado lendo `ExtensionLoader.cpp::AddConfig`, que passa o documento
/// INTEIRO pra cada extensão decidir sua própria subchave).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GarmentOverrideTag {
    pub tag: String,
    pub components: Vec<(String, ChunkMask)>,
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
    pub garment_overrides: Vec<GarmentOverrideTag>,
    pub quest_phases: Vec<QuestPhaseMod>,
    pub journals: Vec<String>,
    pub puppet_state: Option<PuppetStateConfig>,
    pub customization: Option<CustomizationConfig>,
    pub animations: Vec<AnimationEntry>,
    pub streaming: Option<WorldStreamingSection>,
    /// chaves de topo presentes mas ainda não tipadas (customNodes, ...). Nada some.
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
            "overrides" => parse_garment(val, &mut xl),
            "quest" => parse_quest_phase(val, &mut xl),
            // `Journal/Config.cpp`: scalar OU seq de paths. Achado de RE: no C++ real, a forma
            // escalar lê `aNode.Scalar()` (a RAIZ do documento, não o nó "journal") — sempre
            // vazio na prática; implementamos correto aqui (lê o valor do próprio nó).
            "journal" => xl.journals = val.to_str_list(),
            "player" => parse_puppet_state(val, &mut xl),
            "customizations" => parse_customization(val, &mut xl),
            "animations" => parse_animations(val, &mut xl),
            "streaming" => parse_world_streaming(val, &mut xl),
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
            xl.links.push(ResourceLink {
                target: target.clone(),
                sources: sources.to_str_list(),
                is_sequence_form: sources.as_seq().is_some(),
            });
        }
    }
}

/// sequência de escalares → índices de chunk (u8); itens não-numéricos são ignorados.
fn to_u8_list(seq: &[Yaml]) -> Vec<u8> {
    seq.iter().filter_map(|n| n.as_str()).filter_map(|s| s.trim().parse::<u8>().ok()).collect()
}

/// `ParseInt` do C++ (`Num.hpp`): decimal, ou hex com prefixo `0x`/`0X`.
fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

/// `overrides.tags.<tag>.<component>` → `ChunkMask` (`Garment/Config.cpp::LoadYAML`, 3 formas):
/// mapa `{hide:[..]}` ou `{show:[..]}` (hide checado primeiro, só um dos dois é lido — replica o
/// `for (op : {{"hide",false},{"show",true}}) ... break` do C++); sequência nua = hide implícito;
/// escalar = máscara já-computada, crua (sem inversão — `ChunkMask(uint64_t)` guarda `show=false`
/// e o valor tal como está).
fn parse_garment(node: &Yaml, xl: &mut XlFile) {
    let Some(tags) = node.get("tags").and_then(|t| t.as_map()) else { return };
    for (tag, components_node) in tags {
        let Some(components) = components_node.as_map() else { continue };
        let mut out = Vec::new();
        for (comp, chunks) in components {
            let mask = match chunks {
                Yaml::Map(_) => {
                    if let Some(seq) = chunks.get("hide").and_then(|n| n.as_seq()) {
                        ChunkMask::from_chunks(false, &to_u8_list(seq))
                    } else if let Some(seq) = chunks.get("show").and_then(|n| n.as_seq()) {
                        ChunkMask::from_chunks(true, &to_u8_list(seq))
                    } else {
                        continue;
                    }
                }
                Yaml::Seq(seq) => ChunkMask::from_chunks(false, &to_u8_list(seq)),
                Yaml::Scalar(s) => match parse_u64(s) {
                    Some(m) => ChunkMask { show: false, mask: m },
                    None => continue,
                },
                _ => continue,
            };
            out.push((comp.clone(), mask));
        }
        if !out.is_empty() {
            xl.garment_overrides.push(GarmentOverrideTag { tag: tag.clone(), components: out });
        }
    }
}

/// sequência de escalares → índices u16 (node path do quest graph).
fn to_u16_list(seq: &[Yaml]) -> Vec<u16> {
    seq.iter().filter_map(|n| n.as_str()).filter_map(|s| s.trim().parse::<u16>().ok()).collect()
}

/// `FillConnection` (`QuestPhase/Config.cpp`): mapa `{node:[..], socket: nome}` OU sequência nua.
fn fill_connection(node: Option<&Yaml>) -> QuestPhaseConnection {
    let Some(node) = node else { return QuestPhaseConnection::default() };
    if let Some(map) = node.as_map() {
        QuestPhaseConnection {
            node_path: node.get("node").and_then(|n| n.as_seq()).map(to_u16_list).unwrap_or_default(),
            socket: map.iter().find(|(k, _)| k == "socket").and_then(|(_, v)| v.as_str()).map(String::from),
        }
    } else if let Some(seq) = node.as_seq() {
        QuestPhaseConnection { node_path: to_u16_list(seq), socket: None }
    } else {
        QuestPhaseConnection::default()
    }
}

/// `quest.phases[]` (`QuestPhase/Config.cpp::LoadYAML`): cada item precisa de `path`+`parent`
/// escalares (senão é descartado); `connection`/`input` alimentam o MESMO campo (input chamado
/// por último, sobrescreve); `intercept: true` opcional.
fn parse_quest_phase(node: &Yaml, xl: &mut XlFile) {
    let Some(phases) = node.get("phases").and_then(|p| p.as_seq()) else { return };
    for phase in phases {
        let Some(path) = phase.get("path").and_then(|p| p.as_str()) else { continue };
        let Some(parent) = phase.get("parent").and_then(|p| p.as_str()) else { continue };
        let mut input = fill_connection(phase.get("connection"));
        if phase.get("input").is_some() {
            input = fill_connection(phase.get("input"));
        }
        let output = fill_connection(phase.get("output"));
        let intercept = phase.get("intercept").and_then(|i| i.as_str()).map(|s| s == "true").unwrap_or(false);
        xl.quest_phases.push(QuestPhaseMod {
            phase_path: path.to_string(),
            parent: parent.to_string(),
            input,
            output,
            intercept,
        });
    }
}

/// `player.bodyTypes` (`PuppetState/Config.cpp`).
fn parse_puppet_state(node: &Yaml, xl: &mut XlFile) {
    let Some(body_types) = node.get("bodyTypes") else { return };
    let list = body_types.to_str_list();
    if !list.is_empty() {
        xl.puppet_state = Some(PuppetStateConfig { body_types: list });
    }
}

/// `customizations.{male,female}` (`Customization/Config.cpp`).
fn parse_customization(node: &Yaml, xl: &mut XlFile) {
    let male = node.get("male").map(|n| n.to_str_list()).unwrap_or_default();
    let female = node.get("female").map(|n| n.to_str_list()).unwrap_or_default();
    if !male.is_empty() || !female.is_empty() {
        xl.customization = Some(CustomizationConfig { male_options: male, female_options: female });
    }
}

/// `animations[]` (`Animation/Config.cpp::LoadYAML` — ver doc de `AnimationEntry` pros 2 bugs
/// upstream que corrigimos aqui: `entity` como sequência e `vars` nunca funcionam no C++ real).
fn parse_animations(node: &Yaml, xl: &mut XlFile) {
    let Some(entries) = node.as_seq() else { return };
    for entry in entries {
        let Some(entity_node) = entry.get("entity") else { continue };
        let entities = entity_node.to_str_list();
        if entities.is_empty() {
            continue;
        }
        let Some(set) = entry.get("set").and_then(|s| s.as_str()) else { continue };
        let variables = entry.get("vars").map(|v| v.to_str_list()).unwrap_or_default();
        let priority = entry
            .get("priority")
            .and_then(|p| p.as_str())
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(128);
        let component = entry.get("component").and_then(|c| c.as_str()).unwrap_or("root").to_string();
        xl.animations.push(AnimationEntry { entities, set: set.to_string(), variables, priority, component });
    }
}

// ===== streaming.sectors (WorldStreaming/Config.cpp) =====

/// sequência de escalares → f32; `None` se QUALQUER item não for um número (fiel ao C++: um único
/// item inválido invalida a lista inteira — `as<std::vector<float>>()` do yaml-cpp lançaria).
fn to_f32_list(seq: &[Yaml]) -> Option<Vec<f32>> {
    let mut out = Vec::with_capacity(seq.len());
    for item in seq {
        out.push(item.as_str()?.trim().parse::<f32>().ok()?);
    }
    Some(out)
}

fn parse_vec3(node: Option<&Yaml>) -> Option<Vec3> {
    let v = to_f32_list(node?.as_seq()?)?;
    (v.len() == 3).then(|| Vec3 { x: v[0], y: v[1], z: v[2] })
}

/// `position` no nível de NODE: aceita 3 OU 4 valores; o `.w` final é SEMPRE 0 (RE, ver doc de `Vec4`).
fn parse_position_node(node: Option<&Yaml>) -> Option<Vec4> {
    let v = to_f32_list(node?.as_seq()?)?;
    (v.len() == 3 || v.len() == 4).then(|| Vec4 { x: v[0], y: v[1], z: v[2], w: 0.0 })
}

/// `position` no nível de SUB-node: exige EXATAMENTE 4 valores; `.w` também sempre 0.
fn parse_position_subnode(node: Option<&Yaml>) -> Option<Vec4> {
    let v = to_f32_list(node?.as_seq()?)?;
    (v.len() == 4).then(|| Vec4 { x: v[0], y: v[1], z: v[2], w: 0.0 })
}

fn parse_quat(node: Option<&Yaml>) -> Option<Quat> {
    let v = to_f32_list(node?.as_seq()?)?;
    (v.len() == 4).then(|| Quat { i: v[0], j: v[1], k: v[2], r: v[3] })
}

fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

/// "última chave PRESENTE vence" — replica a sequência de `ParseResource`/`ParseName`/
/// `ParseRecordID` chamadas nas várias chaves-sinônimo (cada chamada só sobrescreve se o
/// SEU próprio nó estiver definido; ausência não apaga o que já foi lido por um sinônimo anterior).
fn last_present_str(node: &Yaml, keys: &[&str]) -> Option<String> {
    keys.iter().filter_map(|k| node.get(k).and_then(|n| n.as_str())).last().map(String::from)
}

/// `ParseSubDeletions`: exige a lista de índices E a contagem esperada, ambas presentes e válidas
/// (senão não faz nada). Índice fora de `[0, count)` OU item não-numérico invalida a lista
/// INTEIRA (`aDeletions.clear()` no C++ — inclusive apagando entradas já lidas por um sinônimo
/// anterior nesta mesma chamada de node). `expected` é sobrescrito pelo count desta chamada.
fn parse_sub_deletions(node: &Yaml, list_key: &str, count_key: &str, out: &mut Vec<i64>, expected: &mut i64) {
    let Some(seq) = node.get(list_key).and_then(|n| n.as_seq()) else { return };
    let Some(count) = node.get(count_key).and_then(|n| n.as_str()).and_then(parse_i64) else { return };
    if count <= 0 {
        return;
    }
    let mut collected = Vec::with_capacity(seq.len());
    for item in seq {
        match item.as_str().and_then(parse_i64) {
            Some(idx) if (0..count).contains(&idx) => collected.push(idx),
            _ => {
                out.clear();
                return;
            }
        }
    }
    out.extend(collected);
    *expected = count;
}

/// `ParseSubMutations`: exige a lista E a contagem esperada válidas; itens malformados ou sem
/// NENHUM de position/orientation/scale são simplesmente PULADOS (≠ `parse_sub_deletions`, que
/// invalida a lista inteira — assimetria real do C++, replicada fielmente).
fn parse_sub_mutations(node: &Yaml, list_key: &str, count_key: &str, out: &mut Vec<WorldSubNodeMutation>, expected: &mut i64) {
    let Some(seq) = node.get(list_key).and_then(|n| n.as_seq()) else { return };
    let Some(count) = node.get(count_key).and_then(|n| n.as_str()).and_then(parse_i64) else { return };
    if count <= 0 {
        return;
    }
    for item in seq {
        if item.as_map().is_none() {
            continue;
        }
        let Some(idx) = item.get("index").and_then(|n| n.as_str()).and_then(parse_i64) else { continue };
        if !(0..count).contains(&idx) {
            continue;
        }
        let position = parse_position_subnode(item.get("position"));
        let orientation = parse_quat(item.get("orientation"));
        let scale = parse_vec3(item.get("scale"));
        if position.is_none() && orientation.is_none() && scale.is_none() {
            continue;
        }
        out.push(WorldSubNodeMutation { sub_node_index: idx, position, orientation, scale });
    }
    *expected = count;
}

const RESOURCE_KEYS: [&str; 6] = ["resource", "mesh", "meshRef", "material", "effect", "entityTemplate"];
const APPEARANCE_KEYS: [&str; 3] = ["appearance", "appearanceName", "meshAppearance"];
const RECORD_ID_KEYS: [&str; 3] = ["recordID", "recordId", "objectRecordId"];

fn parse_world_streaming(node: &Yaml, xl: &mut XlFile) {
    let blocks = node.get("blocks").map(|b| b.to_str_list()).unwrap_or_default();
    let mut sectors = Vec::new();
    if let Some(sector_seq) = node.get("sectors").and_then(|s| s.as_seq()) {
        for sector in sector_seq {
            let Some(path) = sector.get("path").and_then(|p| p.as_str()).filter(|s| !s.is_empty()) else { continue };
            let Some(expected_nodes) = sector.get("expectedNodes").and_then(|n| n.as_str()).and_then(parse_i64) else { continue };
            if expected_nodes <= 0 {
                continue;
            }
            let mut node_deletions = Vec::new();
            if let Some(seq) = sector.get("nodeDeletions").and_then(|n| n.as_seq()) {
                for del in seq {
                    let Some(node_type) = del.get("type").and_then(|t| t.as_str()) else { continue };
                    let Some(idx) = del.get("index").and_then(|i| i.as_str()).and_then(parse_i64) else { continue };
                    if !(0..expected_nodes).contains(&idx) {
                        continue;
                    }
                    let mut d = WorldNodeDeletion { node_index: idx, node_type: node_type.to_string(), ..Default::default() };
                    parse_sub_deletions(del, "actorDeletions", "expectedActors", &mut d.sub_node_deletions, &mut d.expected_sub_nodes);
                    parse_sub_deletions(del, "instanceDeletions", "expectedInstances", &mut d.sub_node_deletions, &mut d.expected_sub_nodes);
                    node_deletions.push(d);
                }
            }
            let mut node_mutations = Vec::new();
            if let Some(seq) = sector.get("nodeMutations").and_then(|n| n.as_seq()) {
                for mu in seq {
                    let Some(node_type) = mu.get("type").and_then(|t| t.as_str()) else { continue };
                    let Some(idx) = mu.get("index").and_then(|i| i.as_str()).and_then(parse_i64) else { continue };
                    if !(0..expected_nodes).contains(&idx) {
                        continue;
                    }
                    let nb_diff = mu
                        .get("nbNodesUnderProxyDiff")
                        .and_then(|n| n.as_str())
                        .and_then(|s| parse_i64(s))
                        .and_then(|v| i32::try_from(v).ok());
                    let mut m = WorldNodeMutation {
                        node_index: idx,
                        node_type: node_type.to_string(),
                        position: parse_position_node(mu.get("position")),
                        orientation: parse_quat(mu.get("orientation")),
                        scale: parse_vec3(mu.get("scale")),
                        resource_path: last_present_str(mu, &RESOURCE_KEYS),
                        appearance_name: last_present_str(mu, &APPEARANCE_KEYS),
                        record_id: last_present_str(mu, &RECORD_ID_KEYS),
                        nb_nodes_under_proxy_diff: nb_diff,
                        ..Default::default()
                    };
                    parse_sub_mutations(mu, "actorMutations", "expectedActors", &mut m.sub_node_mutations, &mut m.expected_sub_nodes);
                    parse_sub_mutations(mu, "instanceMutations", "expectedInstances", &mut m.sub_node_mutations, &mut m.expected_sub_nodes);
                    node_mutations.push(m);
                }
            }
            if node_deletions.is_empty() && node_mutations.is_empty() {
                continue;
            }
            sectors.push(WorldSectorMod { path: path.to_string(), expected_nodes, node_deletions, node_mutations });
        }
    }
    if !blocks.is_empty() || !sectors.is_empty() {
        xl.streaming = Some(WorldStreamingSection { blocks, sectors });
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
        if let Some(st) = &self.streaming {
            s.push_str(&format!("streaming.blocks: {}\n", st.blocks.len()));
            s.push_str(&format!("streaming.sectors: {}\n", st.sectors.len()));
            for sec in &st.sectors {
                s.push_str(&format!(
                    "  {} (expectedNodes={}) → {} deleção(ões), {} mutação(ões)\n",
                    sec.path, sec.expected_nodes, sec.node_deletions.len(), sec.node_mutations.len()
                ));
            }
        }
        s.push_str(&format!("journal: {}\n", self.journals.len()));
        if let Some(ps) = &self.puppet_state {
            s.push_str(&format!("player.bodyTypes: {}\n", ps.body_types.join(", ")));
        }
        if let Some(c) = &self.customization {
            s.push_str(&format!("customizations: {} male, {} female\n", c.male_options.len(), c.female_options.len()));
        }
        s.push_str(&format!("animations: {}\n", self.animations.len()));
        for a in &self.animations {
            s.push_str(&format!("  {} → set={} priority={}\n", a.entities.join(","), a.set, a.priority));
        }
        s.push_str(&format!("quest.phases: {}\n", self.quest_phases.len()));
        for p in &self.quest_phases {
            s.push_str(&format!("  {} (parent={}, intercept={})\n", p.phase_path, p.parent, p.intercept));
        }
        s.push_str(&format!("overrides.tags: {}\n", self.garment_overrides.len()));
        for t in &self.garment_overrides {
            s.push_str(&format!("  {} → {} componente(s)\n", t.tag, t.components.len()));
            for (comp, m) in &t.components {
                s.push_str(&format!("    {comp}: show={} mask={:#018x}\n", m.show, m.mask));
            }
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
        // "streaming" agora É tipado (RE 2026-07-15 cont.60) — usa 2 chaves genuinamente
        // desconhecidas (Attachment/Mesh/Transmog/InkSpawner não têm seção .xl própria nenhuma).
        let xl = parse_xl("attachment:\n  x: y\ncustomNode: x\n").unwrap();
        assert!(xl.other_sections.contains(&"attachment".to_string()));
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

    // ===== overrides.tags (Garment ChunkMask, RE 2026-07-15) =====

    #[test]
    fn garment_overrides_forma_sequencia_e_hide_implicito() {
        // sequência nua = hide implícito (ChunkMask(vector<uint8_t>), sem `set` explícito).
        let src = "overrides:\n  tags:\n    my_tag:\n      my_component:\n        - 0\n        - 1\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.garment_overrides.len(), 1);
        let t = &xl.garment_overrides[0];
        assert_eq!(t.tag, "my_tag");
        assert_eq!(t.components.len(), 1);
        let (comp, mask) = &t.components[0];
        assert_eq!(comp, "my_component");
        assert!(!mask.show);
        assert_eq!(mask.mask, !0b11u64); // tudo, exceto os chunks 0 e 1
        assert!(xl.other_sections.is_empty());
    }

    #[test]
    fn garment_overrides_forma_mapa_hide_e_show() {
        let src = "overrides:\n  tags:\n    t1:\n      c_hide:\n        hide:\n          - 2\n      c_show:\n        show:\n          - 2\n";
        let xl = parse_xl(src).unwrap();
        let t = &xl.garment_overrides[0];
        assert_eq!(t.components.len(), 2);
        let hide = &t.components.iter().find(|(n, _)| n == "c_hide").unwrap().1;
        let show = &t.components.iter().find(|(n, _)| n == "c_show").unwrap().1;
        assert!(!hide.show);
        assert_eq!(hide.mask, !0b100u64); // hide: complemento do bit 2
        assert!(show.show);
        assert_eq!(show.mask, 0b100u64); // show: seleção positiva, SEM inversão
    }

    #[test]
    fn garment_overrides_forma_escalar() {
        // escalar = máscara já-computada, crua (sem inversão — ChunkMask(uint64_t): show=false).
        let src = "overrides:\n  tags:\n    t1:\n      c1: 0xff\n      c2: 15\n";
        let xl = parse_xl(src).unwrap();
        let t = &xl.garment_overrides[0];
        let c1 = &t.components.iter().find(|(n, _)| n == "c1").unwrap().1;
        let c2 = &t.components.iter().find(|(n, _)| n == "c2").unwrap().1;
        assert!(!c1.show);
        assert_eq!(c1.mask, 0xff);
        assert_eq!(c2.mask, 15);
    }

    #[test]
    fn garment_overrides_bate_com_chunkmask_real_do_full_body() {
        // Fecha o loop com o achado da RE do full-body (2026-07-15, cont.56): o chunkMask
        // 0xFFFFFFFFFFFFFF1F achado em `t0_000_pwa_fpp__01_ca_pale` (componente
        // t0_000_pwa_fpp__torso) é EXATAMENTE `hide: [5, 6, 7]` por esta fórmula.
        let src = "overrides:\n  tags:\n    t:\n      t0_000_pwa_fpp__torso:\n        hide:\n          - 5\n          - 6\n          - 7\n";
        let xl = parse_xl(src).unwrap();
        let mask = xl.garment_overrides[0].components[0].1.mask;
        assert_eq!(mask, 0xFFFFFFFFFFFFFF1Fu64);
    }

    // ===== quest.phases (QuestPhase/Config.cpp) =====

    #[test]
    fn quest_phases_path_parent_e_conexoes() {
        let src = "quest:\n  phases:\n    - path: my_phase\n      parent: root_graph\n      input:\n        node: [1, 2]\n        socket: In\n      output:\n        - 3\n        - 4\n      intercept: true\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.quest_phases.len(), 1);
        let p = &xl.quest_phases[0];
        assert_eq!(p.phase_path, "my_phase");
        assert_eq!(p.parent, "root_graph");
        assert_eq!(p.input.node_path, vec![1, 2]);
        assert_eq!(p.input.socket, Some("In".to_string()));
        assert_eq!(p.output.node_path, vec![3, 4]);
        assert_eq!(p.output.socket, None);
        assert!(p.intercept);
        assert!(xl.other_sections.is_empty());
    }

    #[test]
    fn quest_phases_sem_path_ou_parent_e_descartada() {
        let src = "quest:\n  phases:\n    - path: only_path\n    - parent: only_parent\n    - path: ok\n      parent: ok_parent\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.quest_phases.len(), 1);
        assert_eq!(xl.quest_phases[0].phase_path, "ok");
    }

    #[test]
    fn quest_phases_input_sobrescreve_connection() {
        // "connection" e "input" alimentam o MESMO campo; input (chamado por último) vence.
        let src = "quest:\n  phases:\n    - path: p\n      parent: r\n      connection: [9]\n      input: [1]\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.quest_phases[0].input.node_path, vec![1]);
    }

    // ===== journal / player.bodyTypes / customizations / animations =====

    #[test]
    fn journal_scalar_e_seq() {
        let xl = parse_xl("journal: a.journal\n").unwrap();
        assert_eq!(xl.journals, vec!["a.journal"]);
        let xl2 = parse_xl("journal:\n  - a.journal\n  - b.journal\n").unwrap();
        assert_eq!(xl2.journals, vec!["a.journal", "b.journal"]);
        assert!(xl2.other_sections.is_empty());
    }

    #[test]
    fn puppet_state_body_types() {
        let xl = parse_xl("player:\n  bodyTypes:\n    - Male\n    - Female\n").unwrap();
        assert_eq!(xl.puppet_state.unwrap().body_types, vec!["Male", "Female"]);
    }

    #[test]
    fn customizations_male_female() {
        let src = "customizations:\n  male:\n    - a.app\n  female: b.app\n";
        let xl = parse_xl(src).unwrap();
        let c = xl.customization.unwrap();
        assert_eq!(c.male_options, vec!["a.app"]);
        assert_eq!(c.female_options, vec!["b.app"]);
    }

    #[test]
    fn animations_entrada_completa() {
        let src = "animations:\n  - entity:\n      - npc_a\n      - npc_b\n    set: my_set.animset\n    vars:\n      - v1\n      - v2\n    priority: 200\n    component: torso\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.animations.len(), 1);
        let a = &xl.animations[0];
        assert_eq!(a.entities, vec!["npc_a", "npc_b"]);
        assert_eq!(a.set, "my_set.animset");
        assert_eq!(a.variables, vec!["v1", "v2"]);
        assert_eq!(a.priority, 200);
        assert_eq!(a.component, "torso");
    }

    #[test]
    fn animations_defaults_e_entity_scalar() {
        let src = "animations:\n  - entity: solo_npc\n    set: s.animset\n";
        let xl = parse_xl(src).unwrap();
        let a = &xl.animations[0];
        assert_eq!(a.entities, vec!["solo_npc"]);
        assert_eq!(a.priority, 128);
        assert_eq!(a.component, "root");
        assert!(a.variables.is_empty());
    }

    #[test]
    fn animations_sem_set_e_descartada() {
        let xl = parse_xl("animations:\n  - entity: x\n").unwrap();
        assert!(xl.animations.is_empty());
    }

    // ===== streaming.sectors (WorldStreaming/Config.cpp) =====

    #[test]
    fn streaming_blocks_e_sector_com_delecao() {
        let src = "streaming:\n  blocks:\n    - a.streamingblock\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 10\n      nodeDeletions:\n        - type: worldStaticMeshNode\n          index: 3\n";
        let xl = parse_xl(src).unwrap();
        let st = xl.streaming.unwrap();
        assert_eq!(st.blocks, vec!["a.streamingblock"]);
        assert_eq!(st.sectors.len(), 1);
        let sec = &st.sectors[0];
        assert_eq!(sec.path, "s.streamingsector");
        assert_eq!(sec.expected_nodes, 10);
        assert_eq!(sec.node_deletions.len(), 1);
        assert_eq!(sec.node_deletions[0].node_index, 3);
        assert_eq!(sec.node_deletions[0].node_type, "worldStaticMeshNode");
        assert!(xl.other_sections.is_empty());
    }

    #[test]
    fn streaming_indice_fora_do_range_e_descartado() {
        // expectedNodes=5, index=9 (fora) → deleção inteira descartada → sector some (0 del + 0 mut)
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: t\n          index: 9\n";
        let xl = parse_xl(src).unwrap();
        assert!(xl.streaming.is_none());
    }

    #[test]
    fn streaming_mutacao_position_3_e_4_valores_w_sempre_zero() {
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          position: [1.0, 2.0, 3.0]\n";
        let xl = parse_xl(src).unwrap();
        let m = &xl.streaming.unwrap().sectors[0].node_mutations[0];
        let pos = m.position.unwrap();
        assert_eq!((pos.x, pos.y, pos.z, pos.w), (1.0, 2.0, 3.0, 0.0));

        // forma com 4 valores: o 4º é aceito mas IGNORADO (w continua 0, fiel ao C++)
        let src2 = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          position: [1.0, 2.0, 3.0, 9.0]\n";
        let xl2 = parse_xl(src2).unwrap();
        let m2 = &xl2.streaming.unwrap().sectors[0].node_mutations[0];
        assert_eq!(m2.position.unwrap().w, 0.0);
    }

    #[test]
    fn streaming_mutacao_recursos_e_sinonimos_ultimo_presente_vence() {
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          resource: a.mesh\n          meshRef: b.mesh\n          appearance: default\n          recordID: Items.X\n";
        let xl = parse_xl(src).unwrap();
        let m = &xl.streaming.unwrap().sectors[0].node_mutations[0];
        // "meshRef" vem depois de "resource" na ordem de chaves-sinônimo → vence
        assert_eq!(m.resource_path.as_deref(), Some("b.mesh"));
        assert_eq!(m.appearance_name.as_deref(), Some("default"));
        assert_eq!(m.record_id.as_deref(), Some("Items.X"));
    }

    #[test]
    fn streaming_submutacoes_actor_com_expected_count() {
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          expectedActors: 3\n          actorMutations:\n            - index: 1\n              scale: [2.0, 2.0, 2.0]\n";
        let xl = parse_xl(src).unwrap();
        let m = &xl.streaming.unwrap().sectors[0].node_mutations[0];
        assert_eq!(m.expected_sub_nodes, 3);
        assert_eq!(m.sub_node_mutations.len(), 1);
        assert_eq!(m.sub_node_mutations[0].sub_node_index, 1);
        assert_eq!(m.sub_node_mutations[0].scale.unwrap().x, 2.0);
    }

    #[test]
    fn streaming_subdelecoes_sem_expected_count_e_ignorada() {
        // actorDeletions presente mas SEM expectedActors → parse_sub_deletions não faz nada
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: t\n          index: 0\n          actorDeletions:\n            - 1\n";
        let xl = parse_xl(src).unwrap();
        let d = &xl.streaming.unwrap().sectors[0].node_deletions[0];
        assert!(d.sub_node_deletions.is_empty());
        assert_eq!(d.expected_sub_nodes, 0);
    }

    #[test]
    fn streaming_sector_sem_delecao_ou_mutacao_e_omitido() {
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n";
        let xl = parse_xl(src).unwrap();
        assert!(xl.streaming.is_none());
    }
}
