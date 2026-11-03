//! Parser do formato `.xl` (subconjunto YAML), zero-dep (só `std`).
//!
//! O `.xl` é um YAML que diz o que fazer ao carregar um mod. Este módulo lê o
//! arquivo num modelo TIPADO (`XlFile`). Seções tipadas (a lista única, com
//! contagem, é `XlFile::secoes`):
//!   - `factories:`            — scalar OU lista de paths .csv
//!   - `resource.patch:`       — path → scalar | seq | {props,targets}, tag `!exclude`
//!   - `resource.{link,copy}:` — caminho → scalar | seq de caminhos; o link vira pares (pedido, servido)
//!   - `resource.{scope,fix}:`
//!   - `localization.{onscreens,subtitles,lipmaps,vomaps}` + `extend`
//!   - `overrides.tags`, `quest.phases`, `journal`,
//!     `player.bodyTypes`, `customizations`, `animations`,
//!     `streaming.{blocks,sectors}`
//!
//! Chave de topo desconhecida (customNodes, ...) e subchave desconhecida de `resource` entram em
//! `XlFile::other_sections`; subchave desconhecida das outras seções é ignorada sem registro.
//! Entrada de seção conhecida em formato inválido fica de fora do modelo e o motivo entra em
//! `XlFile::issues`, com o que fazer; a que o formato aceita e o BWMS ainda não aplica entra em `XlFile::sem_efeito_no_bwms`.
//! Ficam sem issue:
//! - contêiner do tipo errado, que some do modelo: `resource`, `quest`, `overrides`, `player`, `customizations`, `localization` e
//!   `streaming` que não são mapa; `resource.{patch,link,copy,scope}` que não são mapa;
//! - campo de entrada válida no tipo errado, que sai ou fica no padrão: `props`/`targets` do `resource.patch` e item de lista que não
//!   é texto (a entrada só avisa se ficar sem alvo e sem props); `localization.extend`; `vars`, `priority`, `component` e item de
//!   `entity` que não é texto em `animations`.
//!
//! O YAML é lido pelo leitor único `crate::tweaks::yaml`, o mesmo dos tweaks declarativos. O subconjunto e a lista do que fica fora
//! dele estão no doc daquele módulo.

use crate::tweaks::yaml::{self, Kind, Node};

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

/// Um par de `resource.link`: o loader que pede `pedido` recebe `servido`. Um por item, na ordem do YAML. A forma do YAML decide qual lado é a
/// chave, e o parse já entrega o par orientado:
/// - escalar `chave: valor`: a chave é pedida e o valor servido (caminho falso → recurso real);
/// - sequência `chave: [itens]`: cada item é pedido e a chave servida (migração: cada nome legado → o caminho da chave).
///
/// Nenhum dos dois lados é o caminho nulo: o parse tira a entrada ou o item (`caminhos_da_entrada`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceLink {
    /// Caminho que o jogo pede. É o lado que o escopo expande no plano.
    pub pedido: String,
    /// Caminho que o loader serve no lugar do pedido.
    pub servido: String,
}

/// Um escopo de recurso (`resource.scope`): o recurso `resource` define o ESCOPO dos `targets`
/// Alvos = scalar OU lista. Sem caminho nulo e com ao menos um alvo (`caminhos_da_entrada`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceScope {
    pub resource: String,
    pub targets: Vec<String>,
}

/// Uma cópia de recurso (`resource.copy`): copia `source` para cada `targets`
/// Alvos = scalar OU lista. Sem caminho nulo e com ao menos um alvo (`caminhos_da_entrada`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceCopy {
    pub source: String,
    pub targets: Vec<String>,
}

/// Um conserto de recurso (`resource.fix`): no recurso `resource`, remapeia nomes (`names`:
/// nome→nome), paths (`paths`: path→path) e parâmetros de contexto (`context`: nome→valor).
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

/// `player.bodyTypes`: lista de nomes de tipo de corpo (scalar OU seq).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PuppetStateConfig {
    pub body_types: Vec<String>,
}

/// `customizations.{male,female}`: paths de opção de customização
/// por gênero (cada um scalar OU seq — a mesma leitura pros dois).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomizationConfig {
    pub male_options: Vec<String>,
    pub female_options: Vec<String>,
}

/// Uma entrada de `animations[]`. `entity`: nomes de
/// entidade-alvo (scalar OU seq). `set`: nome do anim set (obrigatório). `vars`: lista de variáveis. `priority`
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

// Modelo de `streaming` lido por `tabelas_de_runtime::emit_streaming_table`/`linhas_de_mutacao` e `emit_resource_patch_table`.

/// Vetor 3D.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Vetor 4D.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Vec4 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// Quaternion, componentes i, j, k, r.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Quat {
    pub i: f32,
    pub j: f32,
    pub k: f32,
    pub r: f32,
}

/// Mutação de um sub-node de um node de setor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSubNodeMutation {
    pub sub_node_index: i64,
    pub position: Option<Vec4>,
    pub orientation: Option<Quat>,
    pub scale: Option<Vec3>,
}

/// Mutação de um node de setor.
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

/// Deleção de um node de setor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldNodeDeletion {
    pub node_index: i64,
    pub node_type: String,
    pub expected_sub_nodes: i64,
    pub sub_node_deletions: Vec<i64>,
    /// `[ator, shape]` de `actorDeletions`: só a shape do ator perde a colisão. Com só pares, o node não é apagado inteiro.
    pub shape_deletions: Vec<(i64, i64)>,
}

/// Um setor modificado (`streaming.sectors[]`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldSectorMod {
    pub path: String,
    pub expected_nodes: i64,
    pub node_deletions: Vec<WorldNodeDeletion>,
    pub node_mutations: Vec<WorldNodeMutation>,
}

/// `streaming.{blocks,sectors}`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldStreamingSection {
    pub blocks: Vec<String>,
    pub sectors: Vec<WorldSectorMod>,
}

/// Máscara de chunks de mesh (RE 2026-07-15): decide quais índices de
/// chunk do componente aparecem. `show=true` = seleção POSITIVA (a máscara final tem bit setado
/// exatamente nos chunks listados, forma `show: [...]`, sem inversão). `show=false` = seleção por
/// EXCLUSÃO (a máscara final é o COMPLEMENTO dos chunks listados — forma `hide: [...]`, sequência
/// nua, OU escalar cru já-computado). A lista vira máscara em `crate::override_state::mascara_de_chunks`,
/// a mesma regra da tabela de tags do runtime. Confirma e generaliza o achado da RE do full-body:
/// `chunkMask=0xFFFFFFFFFFFFFF1F` em `t0_000_pwa_fpp__01_ca_pale` = `hide: [5, 6, 7]` por esta
/// fórmula exata (`~((1<<5)|(1<<6)|(1<<7)) = 0xFFFFFFFFFFFFFF1F`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkMask {
    pub show: bool,
    pub mask: u64,
}

impl ChunkMask {
    /// Lista de índices de chunk → `ChunkMask`; `None` com chunk > 63 (a regra está em `mascara_de_chunks`).
    fn from_chunks(show: bool, chunks: &[u8]) -> Option<Self> {
        crate::override_state::mascara_de_chunks(show, chunks).map(|mask| ChunkMask { show, mask })
    }
}

// Modelo de `quest.phases` lido por `tabelas_de_runtime::emit_questphase_table`.

/// Ponto de conexão no grafo de quest: caminho de ids de nó e socket opcional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestPhaseConnection {
    pub node_path: Vec<u16>,
    pub socket: Option<String>,
}

/// Uma fase de quest de mod (`quest.phases[]`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QuestPhaseMod {
    pub phase_path: String,
    pub parent: String,
    pub input: QuestPhaseConnection,
    pub output: QuestPhaseConnection,
    pub intercept: bool,
}

/// Um override de tag de garment (`overrides.tags.<tag>.<component>` → máscara de chunks;
/// a chave de TOPO real é `overrides`, não `garment`, lida direto da raiz do documento).
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
    /// Chaves de topo e subchaves de `resource` ainda não tipadas (customNodes, ...). Subchave
    /// desconhecida das outras seções não entra aqui.
    pub other_sections: Vec<String>,
    /// Entradas de seção conhecida em formato inválido, que ficaram de fora do modelo, com onde estão e o que fazer.
    pub issues: Vec<String>,
    /// Entradas que o formato aceita e o BWMS ainda não aplica (o `context` do `resource.fix`, o grupo de `localization` sem
    /// idioma), com onde estão: limite do BWMS, não defeito do `.xl`. Fora do `issues` para o relatório da `Mods/` não mandar corrigir
    /// um arquivo que funciona no Windows; a gravação das tabelas as junta em `TabelasDosXl::sem_efeito_no_bwms` (`tabelas_de_runtime::entradas_sem_efeito`).
    pub sem_efeito_no_bwms: Vec<String>,
}

/// Uma seção tipada do `.xl` como o parse a leu.
#[derive(Debug, Clone, PartialEq)]
pub struct SecaoXl {
    /// Chave no YAML (`factories`, `resource.patch`, `quest.phases`, ...).
    pub chave: &'static str,
    /// Entradas lidas; seção de valor único conta 1 quando presente. Em `resource.link`, os pares (pedido, servido), não as chaves do YAML.
    pub entradas: usize,
    /// Detalhe das entradas, para o `summary()`.
    pub linhas: Vec<String>,
}

/// Coerção "um-ou-vários" do `.xl`: escalar → `[s]`, sequência → os itens escalares, mapa → nada. A tag não muda o resultado.
fn to_str_list(node: &Node) -> Vec<String> {
    match &node.kind {
        Kind::Scalar(s) => vec![s.clone()],
        Kind::Seq(items) => items.iter().filter_map(|x| x.as_str().map(String::from)).collect(),
        Kind::Map(_) => vec![],
    }
}

/// `to_str_list` com aviso de formato (`factories`/`journal`/`player.bodyTypes`/`customizations`/`localization`): mapa, ou item
/// de lista que não é escalar, fica de fora. O resultado é o mesmo do `to_str_list`. Em `resource.link`/`copy`/`scope`
/// o aviso também é dado.
fn lista_com_aviso(node: &Node, onde: &str, issues: &mut Vec<String>) -> Vec<String> {
    if node.as_map().is_some() || node.as_seq().is_some_and(|s| s.iter().any(|i| i.as_str().is_none())) {
        issues.push(format!("{onde}: formato inválido (parte ignorada); use um valor ou uma lista de valores"));
    }
    to_str_list(node)
}

/// Lê um `.xl` (texto) no modelo tipado.
pub fn parse_xl(input: &str) -> Result<XlFile, String> {
    let doc = yaml::parse(input)?;
    let map = doc.as_map().ok_or("documento .xl não é um mapa no topo")?;
    let mut xl = XlFile::default();
    for (key, val) in map {
        match key.as_str() {
            "factories" => xl.factories = lista_com_aviso(val, "factories", &mut xl.issues),
            "resource" => parse_resource(val, &mut xl),
            "localization" => parse_localization(val, &mut xl),
            "overrides" => parse_garment(val, &mut xl),
            "quest" => parse_quest_phase(val, &mut xl),
            // scalar OU seq de paths; a forma escalar lê o valor do próprio nó.
            "journal" => xl.journals = lista_com_aviso(val, "journal", &mut xl.issues),
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
fn str_pairs(node: Option<&Node>) -> Vec<(String, String)> {
    node.and_then(|n| n.as_map())
        .map(|m| m.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default()
}

/// Caminho que vira o `ResourcePath` nulo: hash 0 (vazio, só barras, só aspas). O `trim` acompanha as tabelas de texto, cujo leitor no runtime tira o
/// espaço de cada coluna (`reslink_file`) e chega ao mesmo 0.
fn caminho_nulo(caminho: &str) -> bool {
    crate::resource_path_hash(caminho.trim()) == 0
}

/// Caminhos de uma entrada de `resource.link`/`copy`/`scope` (`chave: caminho` ou `chave: [caminhos]`), ou `None` quando a entrada sai. O caminho
/// nulo do valor sai, e valor mapa não insere nada. A chave nula também tira a entrada: o par com o caminho 0 faz o shim do runtime trocar todo `ResourcePath` 0, e o
/// `resolver` colapsaria `a → 0 → c` em `a → c`. Entrada sem caminho no fim não muda tabela nenhuma e sai. Os avisos são diagnóstico do BWMS.
fn caminhos_da_entrada(secao: &str, efeito: &str, chave: &str, valor: &Node, issues: &mut Vec<String>) -> Option<Vec<String>> {
    if caminho_nulo(chave) {
        issues.push(format!("resource.{secao}: chave '{chave}' é o caminho vazio ({efeito}); informe o caminho do recurso na chave"));
        return None;
    }
    let onde = format!("resource.{secao}.{chave}");
    if valor.as_map().is_some() {
        issues.push(format!("{onde}: formato inválido ({efeito}); use um caminho ou uma lista de caminhos"));
        return None;
    }
    let mut caminhos = lista_com_aviso(valor, &onde, issues);
    caminhos.retain(|c| {
        let nulo = caminho_nulo(c);
        if nulo {
            issues.push(format!("{onde}: caminho vazio (ignorado); informe o caminho do recurso"));
        }
        !nulo
    });
    (!caminhos.is_empty()).then_some(caminhos)
}

fn parse_resource(node: &Node, xl: &mut XlFile) {
    const TYPED: [&str; 5] = ["patch", "link", "scope", "copy", "fix"];
    // sub-chaves de `resource` ainda não tipadas não somem em silêncio
    if let Some(map) = node.as_map() {
        for (k, _) in map {
            if !TYPED.contains(&k.as_str()) {
                xl.other_sections.push(format!("resource.{k}"));
            }
        }
    }
    // scope: recurso → alvos (scalar|seq). Valor mapa não dá alvo nenhum: o escopo sai.
    if let Some(scope) = node.get("scope").and_then(|s| s.as_map()) {
        for (path, targets) in scope {
            if let Some(targets) = caminhos_da_entrada("scope", "escopo ignorado", path, targets, &mut xl.issues) {
                xl.scopes.push(ResourceScope { resource: path.clone(), targets });
            }
        }
    }
    // copy: source → alvos (scalar|seq)
    if let Some(copy) = node.get("copy").and_then(|c| c.as_map()) {
        for (src, targets) in copy {
            if let Some(targets) = caminhos_da_entrada("copy", "cópia ignorada", src, targets, &mut xl.issues) {
                xl.copies.push(ResourceCopy { source: src.clone(), targets });
            }
        }
    }
    if let Some(fix) = node.get("fix") {
        parse_resource_fix(fix, xl);
    }
    if let Some(patch) = node.get("patch").and_then(|p| p.as_map()) {
        for (path, def) in patch {
            let mut p = ResourcePatch { patch: path.clone(), ..Default::default() };
            if def.as_map().is_some() {
                // forma mapa: { props: [..], targets: [..] }
                if let Some(props) = def.get("props") {
                    p.props = to_str_list(props);
                }
                if let Some(targets) = def.get("targets") {
                    let list = to_str_list(targets);
                    if targets.tag.as_deref() == Some("!exclude") {
                        p.excludes = list;
                    } else {
                        p.includes = list;
                    }
                }
            } else if def.as_seq().is_some() {
                // forma sequência: lista de alvos (include, ou exclude via tag)
                let list = to_str_list(def);
                if def.tag.as_deref() == Some("!exclude") {
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
            } else {
                xl.issues.push(format!("resource.patch.{path}: sem alvo nem props (ignorado); use targets, props ou uma lista de alvos"));
            }
        }
    }
    // link: a sequência inverte o par em relação ao escalar, e a direção se resolve aqui, uma vez.
    if let Some(link) = node.get("link").and_then(|l| l.as_map()) {
        for (chave, valor) in link {
            let Some(caminhos) = caminhos_da_entrada("link", "link ignorado", chave, valor, &mut xl.issues) else {
                continue;
            };
            if valor.as_seq().is_some() {
                xl.links.extend(caminhos.into_iter().map(|item| ResourceLink { pedido: item, servido: chave.clone() }));
            } else {
                xl.links.extend(caminhos.into_iter().map(|servido| ResourceLink { pedido: chave.clone(), servido }));
            }
        }
    }
}

/// `resource.fix` (o valor da chave `fix` de `resource`) → `xl.fixes` e `xl.issues`. Uma sublista presente que não é mapa tira a entrada
/// inteira: aplicar só as outras trocaria parte do recurso e deixaria o resto como o mod não pediu.
fn parse_resource_fix(fix: &Node, xl: &mut XlFile) {
    let Some(entradas) = fix.as_map() else { return };
    for (recurso, def) in entradas {
        if def.as_map().is_none() {
            xl.issues.push(format!("resource.fix.{recurso}: formato inválido (entrada ignorada); use um mapa com names, paths ou context"));
            continue;
        }
        let invalida = ["names", "paths", "context"].into_iter().find(|c| def.get(c).is_some_and(|v| v.as_map().is_none()));
        if let Some(chave) = invalida {
            xl.issues.push(format!("resource.fix.{recurso}.{chave}: formato inválido (entrada ignorada); use um mapa de valor antigo → valor novo"));
            continue;
        }
        xl.fixes.push(ResourceFix {
            resource: recurso.clone(),
            names: str_pairs(def.get("names")),
            paths: str_pairs(def.get("paths")),
            context: str_pairs(def.get("context")),
        });
    }
}

/// sequência de escalares → índices de chunk; `None` se algum item não for número de 0 a 255. A faixa 0..=63 é do `ChunkMask::from_chunks`.
fn to_u8_list(seq: &[Node]) -> Option<Vec<u8>> {
    seq.iter().map(|n| n.as_str()?.trim().parse::<u8>().ok()).collect()
}

/// Inteiro do `.xl`: decimal, ou hex com prefixo `0x`/`0X`.
fn parse_u64(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<u64>().ok()
    }
}

const CHUNK_INVALIDO: &str = "chunk que não é número de 0 a 63 (componente ignorado); corrija a lista de chunks";
const MASCARA_INVALIDA: &str = "máscara inválida (componente ignorado); use hide/show com lista de chunks, lista de chunks ou número";

/// Lista de chunks do `.xl` → `ChunkMask`, ou o motivo da recusa.
fn mascara_da_lista(show: bool, itens: &[Node]) -> Result<ChunkMask, &'static str> {
    to_u8_list(itens).and_then(|chunks| ChunkMask::from_chunks(show, &chunks)).ok_or(CHUNK_INVALIDO)
}

/// Valor de um componente de `overrides.tags.<tag>`: lista nua = esconder; mapa = `hide` (se presente, mesmo inválido) senão `show`, com lista
/// de chunks ou número (os bits dos chunks); texto numérico = a máscara já pronta, gravada crua.
fn mascara_do_componente(valor: &Node) -> Result<ChunkMask, &'static str> {
    match &valor.kind {
        Kind::Seq(itens) => mascara_da_lista(false, itens),
        Kind::Scalar(s) => parse_u64(s).map(|mask| ChunkMask { show: false, mask }).ok_or(MASCARA_INVALIDA),
        Kind::Map(_) => {
            let (show, op) = match (valor.get("hide"), valor.get("show")) {
                (Some(op), _) => (false, op),
                (None, Some(op)) => (true, op),
                (None, None) => return Err(MASCARA_INVALIDA),
            };
            match &op.kind {
                Kind::Seq(itens) => mascara_da_lista(show, itens),
                Kind::Scalar(s) => {
                    parse_u64(s).map(|bits| ChunkMask { show, mask: crate::override_state::mascara_de_bits(show, bits) }).ok_or(MASCARA_INVALIDA)
                }
                Kind::Map(_) => Err(MASCARA_INVALIDA),
            }
        }
    }
}

/// `overrides` (o valor da chave de topo) → `xl.garment_overrides` e `xl.issues`. Tag sem nenhum componente válido não entra.
fn parse_garment(node: &Node, xl: &mut XlFile) {
    let Some(tags) = node.get("tags") else { return };
    let Some(tags) = tags.as_map() else {
        xl.issues.push("overrides.tags: formato inválido (ignorado); use um mapa tag → componentes".to_string());
        return;
    };
    for (tag, componentes) in tags {
        let Some(componentes) = componentes.as_map() else {
            xl.issues.push(format!("overrides.tags.{tag}: formato inválido (tag ignorada); use um mapa componente → chunks"));
            continue;
        };
        let mut validos = Vec::new();
        for (comp, valor) in componentes {
            match mascara_do_componente(valor) {
                Ok(m) => validos.push((comp.clone(), m)),
                Err(motivo) => xl.issues.push(format!("overrides.tags.{tag}.{comp}: {motivo}")),
            }
        }
        if !validos.is_empty() {
            xl.garment_overrides.push(GarmentOverrideTag { tag: tag.clone(), components: validos });
        }
    }
}

/// Uma conexão de `quest.phases[]` como o `.xl` a escreveu.
enum LeituraDeConexao {
    /// Mapa com `node`: troca caminho e socket.
    Mapa(QuestPhaseConnection),
    /// Lista de ids: troca só o caminho, o socket já posto fica.
    Lista(Vec<u16>),
    /// Valor de outro tipo: a chave sai, a fase fica.
    ChaveIgnorada,
    /// Lista com item que não é id, ou mapa sem lista em `node`: a fase sai.
    FaseIgnorada(&'static str),
}

/// Lista de ids de nó de quest (`u16` decimal, com `trim`); `None` se algum item não for id.
fn ids_de_no(itens: &[Node]) -> Option<Vec<u16>> {
    itens.iter().map(|n| n.as_str()?.trim().parse::<u16>().ok()).collect()
}

fn ler_conexao(valor: &Node) -> LeituraDeConexao {
    match &valor.kind {
        Kind::Seq(itens) => ids_de_no(itens).map_or(LeituraDeConexao::FaseIgnorada("item que não é id de 0 a 65535"), LeituraDeConexao::Lista),
        Kind::Map(_) => {
            let Some(itens) = valor.get("node").and_then(Node::as_seq) else {
                return LeituraDeConexao::FaseIgnorada("mapa sem lista de ids em node");
            };
            let Some(node_path) = ids_de_no(itens) else {
                return LeituraDeConexao::FaseIgnorada("node com item que não é id de 0 a 65535");
            };
            let socket = valor.get("socket").and_then(Node::as_str).map(String::from);
            LeituraDeConexao::Mapa(QuestPhaseConnection { node_path, socket })
        }
        Kind::Scalar(_) => LeituraDeConexao::ChaveIgnorada,
    }
}

/// `quest` (o valor da chave de topo) → `xl.quest_phases` e `xl.issues`.
fn parse_quest_phase(node: &Node, xl: &mut XlFile) {
    let Some(fases) = node.get("phases").and_then(Node::as_seq) else { return };
    'fases: for (i, f) in fases.iter().enumerate() {
        let (Some(phase_path), Some(parent)) = (f.get("path").and_then(Node::as_str), f.get("parent").and_then(Node::as_str)) else {
            xl.issues.push(format!("quest.phases[{i}]: sem path ou parent (fase ignorada); informe os dois"));
            continue;
        };
        let mut fase = QuestPhaseMod { phase_path: phase_path.to_string(), parent: parent.to_string(), ..Default::default() };
        // Ordem fixa, não a do YAML: `connection` e `input` escrevem a mesma entrada e `input` tem de vencer.
        for chave in ["connection", "input", "output"] {
            let Some(valor) = f.get(chave) else { continue };
            let conexao = if chave == "output" { &mut fase.output } else { &mut fase.input };
            match ler_conexao(valor) {
                LeituraDeConexao::Mapa(c) => *conexao = c,
                LeituraDeConexao::Lista(ids) => conexao.node_path = ids,
                LeituraDeConexao::ChaveIgnorada => xl.issues.push(format!(
                    "quest.phases[{i}].{chave}: formato inválido (chave ignorada); use uma lista de ids ou um mapa com node e socket"
                )),
                LeituraDeConexao::FaseIgnorada(motivo) => {
                    xl.issues.push(format!("quest.phases[{i}].{chave}: {motivo} (fase ignorada); use uma lista de ids ou um mapa com node e socket"));
                    continue 'fases;
                }
            }
        }
        fase.intercept = f.get("intercept").and_then(Node::as_str) == Some("true");
        xl.quest_phases.push(fase);
    }
}

/// `player.bodyTypes`.
fn parse_puppet_state(node: &Node, xl: &mut XlFile) {
    let Some(body_types) = node.get("bodyTypes") else { return };
    let list = lista_com_aviso(body_types, "player.bodyTypes", &mut xl.issues);
    if !list.is_empty() {
        xl.puppet_state = Some(PuppetStateConfig { body_types: list });
    }
}

/// `customizations.{male,female}`.
fn parse_customization(node: &Node, xl: &mut XlFile) {
    let male = node.get("male").map(|n| lista_com_aviso(n, "customizations.male", &mut xl.issues)).unwrap_or_default();
    let female = node.get("female").map(|n| lista_com_aviso(n, "customizations.female", &mut xl.issues)).unwrap_or_default();
    if !male.is_empty() || !female.is_empty() {
        xl.customization = Some(CustomizationConfig { male_options: male, female_options: female });
    }
}

/// `animations[]` (ver doc de `AnimationEntry`: `entity` e `vars` aceitam escalar ou sequência).
fn parse_animations(node: &Node, xl: &mut XlFile) {
    let Some(entries) = node.as_seq() else {
        xl.issues.push("animations: formato inválido (ignorado); use uma lista de entradas".to_string());
        return;
    };
    for (i, entry) in entries.iter().enumerate() {
        let entities = entry.get("entity").map(to_str_list).unwrap_or_default();
        let (false, Some(set)) = (entities.is_empty(), entry.get("set").and_then(|s| s.as_str())) else {
            xl.issues.push(format!("animations[{i}]: sem entity ou set (entrada ignorada); informe os dois"));
            continue;
        };
        let variables = entry.get("vars").map(to_str_list).unwrap_or_default();
        let priority = entry
            .get("priority")
            .and_then(|p| p.as_str())
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(128);
        let component = entry.get("component").and_then(|c| c.as_str()).unwrap_or("root").to_string();
        xl.animations.push(AnimationEntry { entities, set: set.to_string(), variables, priority, component });
    }
}

// ===== streaming =====

/// Inteiro do `.xl`: decimal, ou hex com prefixo `0x`/`0X`.
fn parse_i64(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        i64::from_str_radix(hex, 16).ok()
    } else {
        s.parse::<i64>().ok()
    }
}

/// Item `[ator, shape]` de `actorDeletions`: só a shape `shape` do ator perde a colisão, o resto do ator fica. Ator em
/// `[0, count)`, shape ≥ 0; o par vale como os índices da mesma lista, então item inválido apaga os dois.
fn par_de_shape(item: &Node, list_key: &str, count: i64) -> Option<(i64, i64)> {
    if list_key != "actorDeletions" {
        return None;
    }
    let [ator, shape] = item.as_seq()? else { return None };
    let ator = ator.as_str().and_then(parse_i64).filter(|a| (0..count).contains(a))?;
    let shape = shape.as_str().and_then(parse_i64).filter(|s| *s >= 0)?;
    Some((ator, shape))
}

/// Inteiro do `.xl` no valor escalar da chave `chave` do mapa `no`.
fn inteiro_da_chave(no: &Node, chave: &str) -> Option<i64> {
    no.get(chave)?.as_str().and_then(parse_i64)
}

/// Lista de números reais (`f32`, com `trim`); `None` se o valor não for lista ou algum item não for número.
fn numeros(valor: Option<&Node>) -> Option<Vec<f32>> {
    valor?.as_seq()?.iter().map(|n| n.as_str()?.trim().parse::<f32>().ok()).collect()
}

/// Posição: o runtime grava `w = 0` (ponto, não direção), então o 4º número, quando aceito, não é lido.
fn posicao(valor: Option<&Node>, tamanhos: &[usize]) -> Option<Vec4> {
    numeros(valor).filter(|v| tamanhos.contains(&v.len())).map(|v| Vec4 { x: v[0], y: v[1], z: v[2], w: 0.0 })
}

fn orientacao(valor: Option<&Node>) -> Option<Quat> {
    numeros(valor).filter(|v| v.len() == 4).map(|v| Quat { i: v[0], j: v[1], k: v[2], r: v[3] })
}

fn escala(valor: Option<&Node>) -> Option<Vec3> {
    numeros(valor).filter(|v| v.len() == 3).map(|v| Vec3 { x: v[0], y: v[1], z: v[2] })
}

/// Texto da última chave presente de `chaves`, na ordem dada (não na do YAML); chave com valor que não é texto conta como ausente.
fn ultimo_sinonimo(no: &Node, chaves: &[&str]) -> Option<String> {
    chaves.iter().rev().find_map(|c| no.get(c)?.as_str()).map(String::from)
}

/// Listas de sub-node e a contagem que cada uma exige, na ordem em que acumulam.
const SUB_DELECOES: [(&str, &str); 2] = [("actorDeletions", "expectedActors"), ("instanceDeletions", "expectedInstances")];
const SUB_MUTACOES: [(&str, &str); 2] = [("actorMutations", "expectedActors"), ("instanceMutations", "expectedInstances")];

/// Sub-listas de uma deleção de node. Um item inválido tira tudo o que o node leu até ali: apagar só parte dos sub-nodes pedidos deixaria o
/// setor num estado que o mod não descreve. As issues saem depois das duas listas, com o efeito que sobrou.
fn ler_sub_delecoes(item: &Node, d: &mut WorldNodeDeletion, onde: &str, issues: &mut Vec<String>) {
    let mut falhas: Vec<(&str, String)> = Vec::new();
    for (lista, contagem) in SUB_DELECOES {
        let Some(valor) = item.get(lista) else { continue };
        let Some(itens) = valor.as_seq() else {
            falhas.push((lista, "não é uma lista de índices".to_string()));
            continue;
        };
        let Some(n) = inteiro_da_chave(item, contagem).filter(|n| *n > 0) else {
            falhas.push((lista, format!("sem {contagem} maior que 0")));
            continue;
        };
        let mut indices = Vec::new();
        let mut pares = Vec::new();
        let mut valida = true;
        for it in itens {
            if let Some(k) = it.as_str().and_then(parse_i64).filter(|k| (0..n).contains(k)) {
                indices.push(k);
            } else if let Some(par) = par_de_shape(it, lista, n) {
                pares.push(par);
            } else {
                valida = false;
                break;
            }
        }
        if valida {
            d.sub_node_deletions.extend(indices);
            d.shape_deletions.extend(pares);
            d.expected_sub_nodes = n;
        } else {
            d.sub_node_deletions.clear();
            d.shape_deletions.clear();
            falhas.push((lista, format!("item que não é índice de 0 a {}", n - 1)));
        }
    }
    let efeito = if d.sub_node_deletions.is_empty() && d.shape_deletions.is_empty() {
        "lista ignorada: o node inteiro será apagado"
    } else {
        "lista ignorada"
    };
    for (lista, motivo) in falhas {
        issues.push(format!("{onde}.{lista}: {motivo} ({efeito}); corrija a lista ou tire a chave"));
    }
}

/// Sub-listas de uma mutação de node. Item ruim só pula ele; a contagem da última lista válida fica, mesmo sem item aceito.
fn ler_sub_mutacoes(item: &Node, m: &mut WorldNodeMutation, onde: &str, issues: &mut Vec<String>) {
    for (lista, contagem) in SUB_MUTACOES {
        let Some(valor) = item.get(lista) else { continue };
        let onde_lista = format!("{onde}.{lista}");
        let Some(itens) = valor.as_seq() else {
            issues.push(format!("{onde_lista}: não é uma lista (lista ignorada); use uma lista de mutações com index"));
            continue;
        };
        let Some(n) = inteiro_da_chave(item, contagem).filter(|n| *n > 0) else {
            issues.push(format!("{onde_lista}: sem {contagem} maior que 0 (lista ignorada); informe {contagem}, a contagem de sub-nodes do node"));
            continue;
        };
        m.expected_sub_nodes = n;
        for (k, sub) in itens.iter().enumerate() {
            if sub.as_map().is_none() {
                issues.push(format!("{onde_lista}[{k}]: não é um mapa (item ignorado); use um mapa com index e position, orientation ou scale"));
                continue;
            }
            let Some(sub_node_index) = inteiro_da_chave(sub, "index").filter(|x| (0..n).contains(x)) else {
                issues.push(format!("{onde_lista}[{k}]: sem index de 0 a {} (item ignorado); informe o index do sub-node", n - 1));
                continue;
            };
            let mutacao = WorldSubNodeMutation {
                sub_node_index,
                position: posicao(sub.get("position"), &[4]),
                orientation: orientacao(sub.get("orientation")),
                scale: escala(sub.get("scale")),
            };
            if mutacao.position.is_none() && mutacao.orientation.is_none() && mutacao.scale.is_none() {
                issues.push(format!(
                    "{onde_lista}[{k}]: sem position, orientation nem scale válidos (item ignorado); informe position [x, y, z, w], orientation [i, j, k, r] ou scale [x, y, z]"
                ));
                continue;
            }
            m.sub_node_mutations.push(mutacao);
        }
    }
}

/// `type` e `index` de um item de `nodeDeletions`/`nodeMutations`, com o índice em `0..expected_nodes`; `None` com a issue já dada.
fn alvo_do_node(item: &Node, expected_nodes: i64, onde: &str, issues: &mut Vec<String>) -> Option<(String, i64)> {
    let (Some(tipo), Some(indice)) = (item.get("type").and_then(Node::as_str), inteiro_da_chave(item, "index")) else {
        issues.push(format!("{onde}: sem type ou index (ignorado); informe o type e o index do node"));
        return None;
    };
    if !(0..expected_nodes).contains(&indice) {
        issues.push(format!("{onde}: index {indice} fora de 0..{expected_nodes} (ignorado); use um index de 0 a {}", expected_nodes - 1));
        return None;
    }
    Some((tipo.to_string(), indice))
}

/// Um item de `streaming.sectors`; `None` com a issue já dada quando o setor sai.
fn ler_setor(i: usize, no: &Node, issues: &mut Vec<String>) -> Option<WorldSectorMod> {
    let raiz = format!("streaming.sectors[{i}]");
    if no.as_map().is_none() {
        issues.push(format!("{raiz}: setor inválido (ignorado); use um mapa com path e expectedNodes"));
        return None;
    }
    let Some(path) = no.get("path") else {
        issues.push(format!("{raiz}: sem path (setor ignorado); informe o caminho do .streamingsector em path"));
        return None;
    };
    let Some(path) = path.as_str().filter(|p| !p.is_empty()) else {
        issues.push(format!("{raiz}: path inválido (setor ignorado); use o caminho do .streamingsector como texto"));
        return None;
    };
    let onde = format!("{raiz} ({path})");
    let Some(esperado) = no.get("expectedNodes") else {
        issues.push(format!("{onde}: sem expectedNodes (setor ignorado); informe expectedNodes, um inteiro maior que 0"));
        return None;
    };
    let Some(expected_nodes) = esperado.as_str().and_then(parse_i64).filter(|n| *n > 0) else {
        issues.push(format!("{onde}: expectedNodes inválido (setor ignorado); use um inteiro maior que 0"));
        return None;
    };
    let mut setor = WorldSectorMod { path: path.to_string(), expected_nodes, ..Default::default() };
    for (j, item) in no.get("nodeDeletions").and_then(Node::as_seq).unwrap_or_default().iter().enumerate() {
        let onde_item = format!("{onde}.nodeDeletions[{j}]");
        let Some((node_type, node_index)) = alvo_do_node(item, expected_nodes, &onde_item, issues) else { continue };
        let mut d = WorldNodeDeletion { node_index, node_type, ..Default::default() };
        ler_sub_delecoes(item, &mut d, &onde_item, issues);
        setor.node_deletions.push(d);
    }
    for (j, item) in no.get("nodeMutations").and_then(Node::as_seq).unwrap_or_default().iter().enumerate() {
        let onde_item = format!("{onde}.nodeMutations[{j}]");
        let Some((node_type, node_index)) = alvo_do_node(item, expected_nodes, &onde_item, issues) else { continue };
        let mut m = WorldNodeMutation {
            node_index,
            node_type,
            position: posicao(item.get("position"), &[3, 4]),
            orientation: orientacao(item.get("orientation")),
            scale: escala(item.get("scale")),
            resource_path: ultimo_sinonimo(item, &["resource", "mesh", "meshRef", "material", "effect", "entityTemplate"]),
            appearance_name: ultimo_sinonimo(item, &["appearance", "appearanceName", "meshAppearance"]),
            record_id: ultimo_sinonimo(item, &["recordID", "recordId", "objectRecordId"]),
            nb_nodes_under_proxy_diff: inteiro_da_chave(item, "nbNodesUnderProxyDiff").and_then(|n| i32::try_from(n).ok()),
            ..Default::default()
        };
        ler_sub_mutacoes(item, &mut m, &onde_item, issues);
        setor.node_mutations.push(m);
    }
    if setor.node_deletions.is_empty() && setor.node_mutations.is_empty() {
        issues.push(format!(
            "{onde}: sem nodeDeletions nem nodeMutations válidos (setor ignorado); acrescente uma deleção ou mutação de node, ou tire o setor"
        ));
        return None;
    }
    Some(setor)
}

/// `streaming` (o valor da chave de topo) → `xl.streaming` e `xl.issues`. `xl.streaming` só existe com algum bloco ou setor.
fn parse_world_streaming(node: &Node, xl: &mut XlFile) {
    if node.as_map().is_none() {
        return;
    }
    let blocks = node.get("blocks").map(to_str_list).unwrap_or_default();
    let mut sectors = Vec::new();
    for (i, setor) in node.get("sectors").and_then(Node::as_seq).unwrap_or_default().iter().enumerate() {
        sectors.extend(ler_setor(i, setor, &mut xl.issues));
    }
    if !blocks.is_empty() || !sectors.is_empty() {
        xl.streaming = Some(WorldStreamingSection { blocks, sectors });
    }
}

fn parse_localization(node: &Node, xl: &mut XlFile) {
    for kind in ["onscreens", "subtitles", "lipmaps", "vomaps"] {
        let Some(grp_node) = node.get(kind) else { continue };
        let Some(grp) = grp_node.as_map() else {
            // O formato aceita lista ou escalar sem idioma (vira en-us); este parser ainda não lê essa forma.
            // Só o `onscreens` tem aplicação no runtime (`TIPOS_DE_TEXTO_APLICADOS`): para ele a forma de mapa é o conserto; os outros ficam sem
            // efeito em qualquer forma.
            let aviso = if kind == "onscreens" {
                "localization.onscreens: lista ou valor sem idioma ainda não é lido pelo BWMS (ignorado); para valer no Mac, use a forma idioma → arquivos (no Windows as duas valem)".to_string()
            } else {
                format!("localization.{kind}: o BWMS ainda só aplica onscreens (ignorado); nada a corrigir no arquivo")
            };
            xl.sem_efeito_no_bwms.push(aviso);
            continue;
        };
        let mut entries: Vec<(String, Vec<String>)> = Vec::new();
        for (lang, paths) in grp {
            // Código fora dos idiomas de jogador (caixa errada e o db-db inclusive) sai aqui: na
            // tabela do runtime ele viraria a reserva do mod quando é o primeiro declarado.
            if !crate::instalacao::localizacao::e_idioma_conhecido(lang.trim()) {
                let codigos: Vec<&str> = crate::instalacao::localizacao::CODIGOS.into_iter().filter(|c| crate::instalacao::localizacao::e_idioma_conhecido(c)).collect();
                xl.issues.push(format!(
                    "localization.{kind}: código de idioma desconhecido \"{lang}\" (idioma ignorado); use um dos códigos do jogo, em minúsculas: {}",
                    codigos.join(", ")
                ));
                continue;
            }
            entries.push((lang.clone(), lista_com_aviso(paths, &format!("localization.{kind}.{lang}"), &mut xl.issues)));
        }
        if !entries.is_empty() {
            xl.localization.push(LocalizationGroup { kind: kind.to_string(), entries });
        }
    }
    if let Some(ext) = node.get("extend").and_then(|e| e.as_str()) {
        xl.localization_extend = Some(ext.to_string());
    }
}

impl XlFile {
    /// As seções tipadas, na ordem do `summary()`: a lista única de features do `.xl`, lida pelo
    /// `summary()` e pelo `classify`. A desestruturação sem `..` faz campo novo no `XlFile` quebrar
    /// a compilação aqui, e não sumir de um dos dois.
    pub fn secoes(&self) -> [SecaoXl; 15] {
        let XlFile {
            factories,
            patches,
            links,
            scopes,
            copies,
            fixes,
            localization,
            localization_extend,
            garment_overrides,
            quest_phases,
            journals,
            puppet_state,
            customization,
            animations,
            streaming,
            other_sections: _,
            issues: _,
            sem_efeito_no_bwms: _,
        } = self;
        let secao = |chave: &'static str, entradas: usize, linhas: Vec<String>| SecaoXl { chave, entradas, linhas };
        let tags = garment_overrides
            .iter()
            .flat_map(|t| {
                std::iter::once(format!("{} → {} componente(s)", t.tag, t.components.len()))
                    .chain(t.components.iter().map(|(comp, m)| format!("  {comp}: show={} mask={:#018x}", m.show, m.mask)))
            })
            .collect();
        let setores = streaming.iter().flat_map(|st| {
            st.blocks.iter().map(|b| format!("bloco {b}")).chain(st.sectors.iter().map(|sec| {
                format!(
                    "{} (expectedNodes={}) → {} deleção(ões), {} mutação(ões)",
                    sec.path,
                    sec.expected_nodes,
                    sec.node_deletions.len(),
                    sec.node_mutations.len()
                )
            }))
        });
        [
            secao("factories", factories.len(), factories.iter().map(|f| format!("- {f}")).collect()),
            secao(
                "resource.patch",
                patches.len(),
                patches
                    .iter()
                    .map(|p| {
                        format!(
                            "{} → +{} alvo(s), -{} excluído(s), props: [{}]",
                            p.patch,
                            p.includes.len(),
                            p.excludes.len(),
                            p.props.join(", ")
                        )
                    })
                    .collect(),
            ),
            secao("resource.link", links.len(), links.iter().map(|l| format!("{} → {}", l.pedido, l.servido)).collect()),
            secao("resource.scope", scopes.len(), scopes.iter().map(|sc| format!("{} → {} alvo(s)", sc.resource, sc.targets.len())).collect()),
            secao("resource.copy", copies.len(), copies.iter().map(|c| format!("{} → {} cópia(s)", c.source, c.targets.len())).collect()),
            secao(
                "resource.fix",
                fixes.len(),
                fixes
                    .iter()
                    .map(|f| format!("{} → {} nome(s), {} path(s), {} contexto(s)", f.resource, f.names.len(), f.paths.len(), f.context.len()))
                    .collect(),
            ),
            secao("localization", localization.len(), localization.iter().map(|g| format!("{}: {} idioma(s)", g.kind, g.entries.len())).collect()),
            secao("localization.extend", usize::from(localization_extend.is_some()), localization_extend.iter().cloned().collect()),
            secao("streaming", streaming.as_ref().map_or(0, |st| st.blocks.len() + st.sectors.len()), setores.collect()),
            secao("journal", journals.len(), Vec::new()),
            secao("player.bodyTypes", puppet_state.as_ref().map_or(0, |ps| ps.body_types.len()), puppet_state.iter().map(|ps| ps.body_types.join(", ")).collect()),
            secao(
                "customizations",
                customization.as_ref().map_or(0, |c| c.male_options.len() + c.female_options.len()),
                customization.iter().map(|c| format!("{} male, {} female", c.male_options.len(), c.female_options.len())).collect(),
            ),
            secao(
                "animations",
                animations.len(),
                animations.iter().map(|a| format!("{} → set={} priority={}", a.entities.join(","), a.set, a.priority)).collect(),
            ),
            secao(
                "quest.phases",
                quest_phases.len(),
                quest_phases.iter().map(|p| format!("{} (parent={}, intercept={})", p.phase_path, p.parent, p.intercept)).collect(),
            ),
            secao("overrides.tags", garment_overrides.len(), tags),
        ]
    }

    /// Resumo legível pra CLI: cada seção com a contagem e o detalhe, depois o que o parser não leu.
    pub fn summary(&self) -> String {
        let mut s = String::new();
        for secao in self.secoes() {
            s.push_str(&format!("{}: {}\n", secao.chave, secao.entradas));
            for linha in &secao.linhas {
                s.push_str(&format!("  {linha}\n"));
            }
        }
        if !self.other_sections.is_empty() {
            s.push_str(&format!("⚠ seções ainda não processadas: {}\n", self.other_sections.join(", ")));
        }
        if !self.issues.is_empty() {
            s.push_str("⚠ entradas ignoradas por formato inválido:\n");
            for i in &self.issues {
                s.push_str(&format!("  - {i}\n"));
            }
        }
        // Formato aceito pelo `.xl`: sob "formato inválido" a linha se contradiria.
        if !self.sem_efeito_no_bwms.is_empty() {
            s.push_str("⚠ entradas que o BWMS ainda não aplica:\n");
            for i in &self.sem_efeito_no_bwms {
                s.push_str(&format!("  - {i}\n"));
            }
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
        let y = yaml::parse("a: 1\nb:\n  - x\n  - y\nc: [ p, q ]\n").unwrap();
        assert_eq!(y.get("a").unwrap().as_str(), Some("1"));
        assert_eq!(to_str_list(y.get("b").unwrap()), vec!["x", "y"]);
        assert_eq!(to_str_list(y.get("c").unwrap()), vec!["p", "q"]);
    }

    #[test]
    fn comentarios_e_brancas() {
        let y = yaml::parse("# topo\nfactories:\n  - a.csv   # inline\n\n  - b.csv\n").unwrap();
        assert_eq!(to_str_list(y.get("factories").unwrap()), vec!["a.csv", "b.csv"]);
    }

    // ---- exemplos das formas principais ----

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
    fn patch_forma_mapa_props_e_targets() {
        // forma mapa: props + targets
        let src = "resource:\n  patch:\n    mod\\characters\\common\\hair\\h1_base_color_patch.mesh:\n      props: [ appearances ]\n      targets: [ player_ma_hair.mesh, player_wa_hair.mesh ]\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.patches.len(), 1);
        let p = &xl.patches[0];
        assert_eq!(p.patch, "mod\\characters\\common\\hair\\h1_base_color_patch.mesh");
        assert_eq!(p.props, vec!["appearances"]);
        assert_eq!(p.includes, vec!["player_ma_hair.mesh", "player_wa_hair.mesh"]);
        assert!(p.excludes.is_empty());
    }

    #[test]
    fn link_de_migracao_em_sequencia() {
        // migração de nome legado: na sequência cada item é pedido e a chave é servida
        let src = "resource:\n  link:\n    mod\\a\\h1.mesh:\n      - mod\\a\\base.mesh\n    mod\\b\\head.app:\n      - mod\\b\\lashes.app\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(
            xl.links,
            vec![
                ResourceLink { pedido: "mod\\a\\base.mesh".into(), servido: "mod\\a\\h1.mesh".into() },
                ResourceLink { pedido: "mod\\b\\lashes.app".into(), servido: "mod\\b\\head.app".into() },
            ]
        );
    }

    /// `resource.link` conta e descreve pares (pedido, servido), e não chaves do YAML: `novo: [p1, p2]` são 2 links que servem `novo`. O
    /// `classify` imprime essa contagem (`resource.link ×N`, classify.rs:310) e o `bwms xl`, o `summary`.
    #[test]
    fn secao_de_link_lista_os_pares() {
        let xl = parse_xl("resource:\n  link:\n    fake: real\n    novo: [p1, p2]\n").unwrap();
        let link = xl.secoes().into_iter().find(|s| s.chave == "resource.link").expect("seção");
        assert_eq!(link.entradas, 3);
        assert_eq!(link.linhas, vec!["fake → real", "p1 → novo", "p2 → novo"]);
        assert!(xl.summary().contains("resource.link: 3\n  fake → real\n  p1 → novo\n  p2 → novo\n"), "{}", xl.summary());
    }

    // ---- formas alternativas ----

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
        // "streaming" é tipado — usa 2 chaves genuinamente desconhecidas.
        let xl = parse_xl("attachment:\n  x: y\ncustomNode: x\n").unwrap();
        assert!(xl.other_sections.contains(&"attachment".to_string()));
        assert!(xl.other_sections.contains(&"customNode".to_string()));
    }

    #[test]
    fn entrada_conhecida_malformada_vira_aviso_sem_mudar_o_modelo() {
        let xl = parse_xl("streaming:\n  sectors:\n    - path: s.streamingsector\n").unwrap();
        assert!(xl.streaming.is_none());
        assert_eq!(
            xl.issues,
            vec!["streaming.sectors[0] (s.streamingsector): sem expectedNodes (setor ignorado); informe expectedNodes, um inteiro maior que 0"]
        );

        let xl = parse_xl("overrides:\n  tags:\n    t: x\n").unwrap();
        assert!(xl.garment_overrides.is_empty());
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);

        let xl = parse_xl("factories:\n  a: b\n").unwrap();
        assert!(xl.factories.is_empty());
        assert!(!xl.issues.is_empty());

        // Item de lista que não é escalar sai; o resto fica.
        let xl = parse_xl("journal:\n  - a.journal\n  - x: y\n").unwrap();
        assert_eq!(xl.journals, vec!["a.journal"]);
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);

        let xl = parse_xl("animations:\n  x: y\nquest:\n  phases:\n    - path: a.questphase\n").unwrap();
        assert!(xl.animations.is_empty() && xl.quest_phases.is_empty());
        assert_eq!(xl.issues.len(), 2, "{:?}", xl.issues);

        let xl = parse_xl("streaming:\n  sectors:\n    - x\n    - expectedNodes: 2\n    - path: s.streamingsector\n      expectedNodes: 0\n").unwrap();
        assert!(xl.streaming.is_none());
        assert_eq!(xl.issues.len(), 3, "{:?}", xl.issues);

        // Os pontos que eram calados (XLP-02 a-f): 1 issue cada, e nada do inválido chega ao modelo nem à tabela.
        let xl = parse_xl("resource:\n  scope:\n    a.app:\n      x: y\n    b.app: c.app\n").unwrap();
        assert_eq!(xl.scopes, vec![ResourceScope { resource: "b.app".into(), targets: vec!["c.app".into()] }]);
        assert_eq!(xl.issues, vec!["resource.scope.a.app: formato inválido (escopo ignorado); use um caminho ou uma lista de caminhos"]);

        // `names` em lista tira a entrada inteira, `paths` válido incluso; definição escalar também sai
        let xl = parse_xl("resource:\n  fix:\n    t.mesh:\n      names: [ a, b ]\n      paths:\n        o.app: n.app\n    u.mesh: abc\n").unwrap();
        assert!(xl.fixes.is_empty(), "{:?}", xl.fixes);
        assert_eq!(xl.issues.len(), 2, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("resource.fix.t.mesh.names: formato inválido (entrada ignorada)"), "{}", xl.issues[0]);
        assert!(xl.issues[1].starts_with("resource.fix.u.mesh: formato inválido (entrada ignorada)"), "{}", xl.issues[1]);
        assert!(!crate::instalacao::tabelas_de_runtime::emit_resfixes(&xl).lines().any(|l| !l.starts_with('#')));

        // chunk que não é número, fora de 0..=63, e `hide` inválido ao lado de `show` válido: o componente sai, e o válido fica; número no op
        // = os bits dos chunks, `hide` invertido
        let xl = parse_xl("overrides:\n  tags:\n    t:\n      c0: [ x ]\n      c1:\n        hide: [ 70 ]\n      c2:\n        hide: x\n        show: [ 1 ]\n      c3: [ 1, 63 ]\n      c4:\n        hide: 5\n      c5:\n        show: 0x6\n").unwrap();
        assert_eq!(xl.garment_overrides.len(), 1);
        assert_eq!(
            xl.garment_overrides[0].components,
            vec![
                ("c3".to_string(), ChunkMask { show: false, mask: !((1u64 << 1) | (1u64 << 63)) }),
                ("c4".to_string(), ChunkMask { show: false, mask: !5u64 }),
                ("c5".to_string(), ChunkMask { show: true, mask: 6 }),
            ]
        );
        assert_eq!(xl.issues.len(), 3, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("overrides.tags.t.c0: chunk que não é número de 0 a 63 (componente ignorado)"), "{}", xl.issues[0]);
        assert!(xl.issues[1].starts_with("overrides.tags.t.c1: chunk que não é número de 0 a 63 (componente ignorado)"), "{}", xl.issues[1]);
        assert!(xl.issues[2].starts_with("overrides.tags.t.c2: máscara inválida (componente ignorado)"), "{}", xl.issues[2]);

        // node path escalar ou com item inválido tira a fase (vazio seria o nó inicial; [1, 3] outro nó); só a fase válida vai para a tabela
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      connection:\n        node: 5\n    - path: b.questphase\n      parent: p.questphase\n      input: [ 1, x, 3 ]\n    - path: c.questphase\n      parent: p.questphase\n      output: [ 2 ]\n").unwrap();
        assert_eq!(xl.quest_phases.iter().map(|f| f.phase_path.as_str()).collect::<Vec<_>>(), vec!["c.questphase"]);
        assert_eq!(xl.issues.len(), 2, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("quest.phases[0].connection: mapa sem lista de ids em node (fase ignorada)"), "{}", xl.issues[0]);
        assert!(xl.issues[1].starts_with("quest.phases[1].input: item que não é id de 0 a 65535 (fase ignorada)"), "{}", xl.issues[1]);
        let tabela = crate::instalacao::tabelas_de_runtime::emit_questphase_table(&xl, "m");
        assert_eq!(tabela.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>(), vec!["m | p.questphase | c.questphase |  | - | 2 | - | 0"]);

        // chave escalar não lida fica de fora com issue, e a fase segue com o `connection` que leu
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      connection: [ 1, 2 ]\n      input: 5\n      output: x\n").unwrap();
        assert_eq!(xl.quest_phases.len(), 1);
        assert_eq!(xl.quest_phases[0].input, QuestPhaseConnection { node_path: vec![1, 2], socket: None });
        assert_eq!(xl.quest_phases[0].output, QuestPhaseConnection::default());
        assert_eq!(xl.issues.len(), 2, "{:?}", xl.issues);
        assert!(xl.issues.iter().all(|i| i.contains("formato inválido (chave ignorada)")), "{:?}", xl.issues);

        // sub-mutação sem index sai com issue; a mutação do node fica (o modelo não muda)
        let xl = parse_xl("streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          scale: [ 1, 1, 1 ]\n          expectedActors: 3\n          actorMutations:\n            - scale: [ 2, 2, 2 ]\n").unwrap();
        let m = &xl.streaming.as_ref().unwrap().sectors[0].node_mutations[0];
        assert!(m.sub_node_mutations.is_empty() && m.scale.is_some());
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("streaming.sectors[0] (s.streamingsector).nodeMutations[0].actorMutations[0]: sem index de 0 a 2 (item ignorado)"), "{}", xl.issues[0]);

        // setor sem operação válida sai com issue
        let xl = parse_xl("streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n").unwrap();
        assert!(xl.streaming.is_none());
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("streaming.sectors[0] (s.streamingsector): sem nodeDeletions nem nodeMutations válidos (setor ignorado)"));
    }

    #[test]
    fn virgula_final_em_lista_inline_nao_abre_item() {
        // `[ 1, 2, ]` tem 2 itens em YAML; o `""` do fim não pode invalidar a lista de ids nem a de chunks
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      input: [ 1, 2, ]\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.quest_phases[0].input, QuestPhaseConnection { node_path: vec![1, 2], socket: None });
        let tabela = crate::instalacao::tabelas_de_runtime::emit_questphase_table(&xl, "m");
        assert!(tabela.lines().any(|l| l == "m | p.questphase | a.questphase | 1 2 | - |  | - | 0"), "{tabela}");

        let xl = parse_xl("overrides:\n  tags:\n    t:\n      c0: [ 1, 2, ]\n      c1:\n        hide: [ 3, ]\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(
            xl.garment_overrides[0].components,
            vec![
                ("c0".to_string(), ChunkMask { show: false, mask: 0xffff_ffff_ffff_fff9 }),
                ("c1".to_string(), ChunkMask { show: false, mask: 0xffff_ffff_ffff_fff7 }),
            ]
        );

        let xl = parse_xl("factories: [ a.csv, ]\n").unwrap();
        assert_eq!(xl.factories, vec!["a.csv"]);

        // `[ , ]` não vira `[]` (hide-all): a lista continua inválida e o componente sai com aviso
        let xl = parse_xl("overrides:\n  tags:\n    t:\n      c0: [ , ]\n").unwrap();
        assert!(xl.garment_overrides.is_empty());
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);
        assert!(xl.issues[0].starts_with("overrides.tags.t.c0: chunk que não é número de 0 a 63 (componente ignorado)"), "{}", xl.issues[0]);
    }

    #[test]
    fn input_em_lista_mantem_o_socket_do_connection_em_mapa() {
        // A conexão em sequência grava só o nodePath: o socket que o `connection` em mapa
        // gravou continua, e o runtime liga em "Out2", e não no "Out" do `-`.
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      connection:\n        node: [ 3 ]\n        socket: Out2\n      input: [ 5 ]\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.quest_phases[0].input, QuestPhaseConnection { node_path: vec![5], socket: Some("Out2".into()) });
        let tabela = crate::instalacao::tabelas_de_runtime::emit_questphase_table(&xl, "m");
        assert!(tabela.lines().any(|l| l == "m | p.questphase | a.questphase | 5 | Out2 |  | - | 0"), "{tabela}");

        // `input` em mapa troca node path e socket: sem socket, volta ao padrão
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      connection:\n        node: [ 3 ]\n        socket: Out2\n      input:\n        node: [ 5 ]\n").unwrap();
        assert_eq!(xl.quest_phases[0].input, QuestPhaseConnection { node_path: vec![5], socket: None });

        // sequência sozinha: socket nenhum; `connection` sozinho em mapa: o socket dele
        let xl = parse_xl("quest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\n      input: [ 4, 2 ]\n    - path: b.questphase\n      parent: p.questphase\n      connection:\n        node: [ 3 ]\n        socket: Out2\n").unwrap();
        assert_eq!(xl.quest_phases[0].input, QuestPhaseConnection { node_path: vec![4, 2], socket: None });
        assert_eq!(xl.quest_phases[1].input, QuestPhaseConnection { node_path: vec![3], socket: Some("Out2".into()) });
    }

    #[test]
    fn subdelecao_invalida_avisa_o_efeito_que_sobrou() {
        let fonte = |extra: &str| {
            format!("streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: t\n          index: 1\n{extra}")
        };
        // lista inválida e nada mais: sem sub-deleção o node inteiro é apagado
        let xl = parse_xl(&fonte("          expectedActors: 2\n          actorDeletions: [ 0, x ]\n")).unwrap();
        let d = &xl.streaming.as_ref().unwrap().sectors[0].node_deletions[0];
        assert!(d.sub_node_deletions.is_empty());
        assert_eq!(
            xl.issues,
            vec!["streaming.sectors[0] (s.streamingsector).nodeDeletions[0].actorDeletions: item que não é índice de 0 a 1 (lista ignorada: o node inteiro será apagado); corrija a lista ou tire a chave"]
        );
        let tabela = crate::instalacao::tabelas_de_runtime::emit_streaming_table(&xl, "m");
        assert!(tabela.lines().any(|l| l == "m | s.streamingsector | 5 | 1 | t"), "{tabela}");

        // sem contagem, o actor sai e o instance válido fica: só a lista é ignorada
        let xl = parse_xl(&fonte("          actorDeletions: [ 0 ]\n          expectedInstances: 3\n          instanceDeletions: [ 2 ]\n")).unwrap();
        assert_eq!(xl.streaming.as_ref().unwrap().sectors[0].node_deletions[0].sub_node_deletions, vec![2]);
        assert_eq!(
            xl.issues,
            vec!["streaming.sectors[0] (s.streamingsector).nodeDeletions[0].actorDeletions: sem expectedActors maior que 0 (lista ignorada); corrija a lista ou tire a chave"]
        );

        // item inválido no 2º sinônimo apaga também o que o 1º leu: o aviso diz o efeito final
        let xl = parse_xl(&fonte("          expectedActors: 2\n          actorDeletions: [ 0 ]\n          expectedInstances: 2\n          instanceDeletions: [ 7 ]\n")).unwrap();
        assert!(xl.streaming.as_ref().unwrap().sectors[0].node_deletions[0].sub_node_deletions.is_empty());
        assert_eq!(xl.issues.len(), 1, "{:?}", xl.issues);
        assert!(xl.issues[0].contains(".instanceDeletions: item que não é índice de 0 a 1 (lista ignorada: o node inteiro será apagado)"), "{}", xl.issues[0]);
    }

    #[test]
    fn todo_aviso_do_parse_diz_o_que_fazer() {
        // Os `.xl` passam pelos 32 pontos do parse que empurram issue; a issue vai para os avisos do relatório da `Mods/`, que prometem a
        // ação em cada linha: depois de "(efeito); " vem o que fazer. A contagem exata pega o ponto que deixar de avisar. A lista sem
        // idioma do `onscreens` vai para `sem_efeito_no_bwms`, que também chega aos avisos e também diz o que fazer.
        let fontes = [
            "factories:\n  a: b\n",
            "journal:\n  - x: y\n",
            "player:\n  bodyTypes:\n    a: b\n",
            "customizations:\n  male:\n    a: b\n",
            "localization:\n  onscreens: [ a.json ]\n  subtitles:\n    en-us:\n      a: b\n",
            "localization:\n  onscreens:\n    PT-BR: a.json\n",
            "resource:\n  patch:\n    p.mesh:\n      x: y\n  scope:\n    a.app:\n      x: y\n    b.app:\n      - c.app\n      - x: y\n  fix:\n    t.mesh: abc\n    u.mesh:\n      context: [ a ]\n",
            "resource:\n  link:\n    '': a.mesh\n    b.mesh: ''\n  copy:\n    c.mesh:\n      x: y\n",
            "overrides:\n  tags: x\n",
            "overrides:\n  tags:\n    t: x\n    u:\n      c0: [ x ]\n      c1:\n        show: x\n",
            "quest:\n  phases:\n    - path: a\n    - path: b\n      parent: p\n      input: x\n    - path: c\n      parent: p\n      output:\n        socket: s\n",
            "animations:\n  a: b\n",
            "animations:\n  - set: s\n",
            "streaming:\n  sectors:\n    - x\n    - expectedNodes: 2\n    - path: [ a ]\n    - path: s\n    - path: s\n      expectedNodes: 0\n    - path: s\n      expectedNodes: 2\n",
            "streaming:\n  sectors:\n    - path: s\n      expectedNodes: 2\n      nodeDeletions:\n        - type: t\n        - type: t\n          index: 9\n        - type: t\n          index: 0\n          actorDeletions: 3\n      nodeMutations:\n        - index: 0\n        - type: t\n          index: 9\n        - type: t\n          index: 1\n          actorMutations: 3\n          instanceMutations:\n            - 1\n          expectedInstances: 2\n        - type: t\n          index: 1\n          expectedActors: 0\n          actorMutations:\n            - index: 0\n          expectedInstances: 2\n          instanceMutations:\n            - index: 0\n            - scale: [ 1, 1, 1 ]\n",
        ];
        let lidos: Vec<XlFile> = fontes.iter().map(|f| parse_xl(f).unwrap()).collect();
        let issues: Vec<&String> = lidos.iter().flat_map(|x| &x.issues).collect();
        let sem_efeito: Vec<&String> = lidos.iter().flat_map(|x| &x.sem_efeito_no_bwms).collect();
        for i in issues.iter().chain(&sem_efeito) {
            let acao = i.split_once("); ").map(|(_, a)| a.trim()).unwrap_or("");
            assert!(!acao.is_empty(), "aviso sem ação: {i}");
        }
        assert_eq!(issues.len(), 39, "{issues:#?}");
        assert_eq!(sem_efeito.len(), 1, "{sem_efeito:#?}");
    }

    #[test]
    fn caminho_nulo_em_link_copy_e_scope_sai_com_aviso() {
        // A sonda do NOVO-01: sem a regra, a tabela teria ` | base\c.mesh`, `base\a.mesh | base\c.mesh` (a → 0 → c colapsado) e ` | base\orig.mesh | copy`.
        let sonda = "resource:\n  link:\n    base\\a.mesh:\n    base\\c.mesh: ['']\n  copy:\n    base\\orig.mesh:\n";
        let xl = parse_xl(sonda).unwrap();
        assert!(xl.links.is_empty() && xl.copies.is_empty(), "{xl:?}");
        assert_eq!(
            xl.issues,
            vec![
                "resource.copy.base\\orig.mesh: caminho vazio (ignorado); informe o caminho do recurso",
                "resource.link.base\\a.mesh: caminho vazio (ignorado); informe o caminho do recurso",
                "resource.link.base\\c.mesh: caminho vazio (ignorado); informe o caminho do recurso",
            ]
        );
        assert!(crate::instalacao::tabelas_de_runtime::emit_reslink(&xl).lines().all(|l| l.starts_with('#')));

        // Cadeia pelo 0: `a → 0` e `0 → c` não viram `a → c`.
        let cadeia = parse_xl("resource:\n  link:\n    base\\a.mesh: ''\n    '': base\\c.mesh\n").unwrap();
        assert!(cadeia.links.is_empty(), "{cadeia:?}");
        assert_eq!(cadeia.issues[1], "resource.link: chave '' é o caminho vazio (link ignorado); informe o caminho do recurso na chave");
        assert!(crate::instalacao::tabelas_de_runtime::emit_reslink(&cadeia).lines().all(|l| l.starts_with('#')));

        // O caminho válido ao lado do nulo fica. `' '` é nulo: o leitor das tabelas tira o espaço da coluna.
        let src = "resource:\n  scope:\n    s.app: [ '', ' ', x.app ]\n    ' ': y.app\n  copy:\n    /: [ k.mesh ]\n    o.mesh:\n      x: y\n    n.mesh: [ '', c.mesh ]\n  link:\n    m.mesh: [ '', old.mesh ]\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.scopes, vec![ResourceScope { resource: "s.app".into(), targets: vec!["x.app".into()] }]);
        assert_eq!(xl.copies, vec![ResourceCopy { source: "n.mesh".into(), targets: vec!["c.mesh".into()] }]);
        assert_eq!(xl.links, vec![ResourceLink { pedido: "old.mesh".into(), servido: "m.mesh".into() }]);
        assert_eq!(
            xl.issues,
            vec![
                "resource.scope.s.app: caminho vazio (ignorado); informe o caminho do recurso",
                "resource.scope.s.app: caminho vazio (ignorado); informe o caminho do recurso",
                "resource.scope: chave ' ' é o caminho vazio (escopo ignorado); informe o caminho do recurso na chave",
                "resource.copy: chave '/' é o caminho vazio (cópia ignorada); informe o caminho do recurso na chave",
                "resource.copy.o.mesh: formato inválido (cópia ignorada); use um caminho ou uma lista de caminhos",
                "resource.copy.n.mesh: caminho vazio (ignorado); informe o caminho do recurso",
                "resource.link.m.mesh: caminho vazio (ignorado); informe o caminho do recurso",
            ]
        );
        let tabela = crate::instalacao::tabelas_de_runtime::emit_reslink(&xl);
        let linhas: Vec<&str> = tabela.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["old.mesh | m.mesh", "c.mesh | n.mesh | copy"]);
        // Como o runtime lê a linha (`tabelas::Reslink`, o de `reslink_file`): os 2 lados re-hasheados, nenhum no caminho nulo.
        let h = crate::resource_path_hash;
        assert!(crate::tabelas::ler(&tabela, crate::tabelas::Reslink::ler).all(|l| l.is_ok_and(|l| h(l.pedido) != 0 && h(l.servido) != 0)), "{linhas:?}");
        let (listas, _) = crate::instalacao::tabelas_de_runtime::resource_scope_listas(&[&xl]);
        assert_eq!(listas.len(), 1, "{listas:?}");
        assert_eq!(listas[&crate::resource_path_hash("s.app")], vec!["x.app".to_string()]);
    }

    #[test]
    fn entrada_valida_nao_gera_aviso() {
        let src = "factories: a.csv\njournal:\n  - b.journal\nplayer:\n  bodyTypes: [ x ]\ncustomizations:\n  male: m.inkcharcustomization\n  female:\n    - f.inkcharcustomization\nlocalization:\n  onscreens:\n    en-us: [ e.json ]\n  extend: x\noverrides:\n  tags:\n    t:\n      c0:\n        hide: [ 1 ]\n      c1: [ 2 ]\n      c2: 0xff\nquest:\n  phases:\n    - path: a.questphase\n      parent: p.questphase\nanimations:\n  - entity: player_ma_tpp\n    set: s.anims\nstreaming:\n  blocks: [ b.streamingblock ]\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 3\n      nodeDeletions:\n        - type: t\n          index: 1\nresource:\n  patch:\n    p.mesh: [ q.mesh ]\n";
        let xl = parse_xl(src).unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        let vazias: Vec<&str> = xl.secoes().iter().filter(|s| s.entradas == 0).map(|s| s.chave).collect();
        assert_eq!(vazias, vec!["resource.link", "resource.scope", "resource.copy", "resource.fix"]);
    }

    #[test]
    fn ancora_e_alias() {
        // padrão real do EyesFix/BrowsFix: define com &nome, reusa com *nome
        let src = "resource:\n  fix:\n    a.ink: &Fix\n      paths:\n        x.app: y.app\n    b.ink: *Fix\n";
        // o YAML genérico resolve o alias pro MESMO nó da âncora
        let doc = yaml::parse(src).unwrap();
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

    /// O `.xl` segue a regra do leitor único (`crate::tweaks::yaml`): tag com a lista em bloco embaixo e flow-map viram o que o
    /// YAML define; tab na indentação e merge `<<` derrubam o documento com a linha, em vez de um modelo calado errado.
    #[test]
    fn xl_segue_a_regra_do_leitor_unico() {
        let xl = parse_xl("resource:\n  patch:\n    a.mesh: !exclude\n      - x.mesh\n    b.mesh: { props: [ appearances ], targets: [ y.mesh ] }\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.patches[0].excludes, vec!["x.mesh"]);
        assert_eq!((xl.patches[1].props.as_slice(), xl.patches[1].includes.as_slice()), (["appearances".to_string()].as_slice(), ["y.mesh".to_string()].as_slice()));
        // `''` dentro de aspa simples não fecha o trecho: a vírgula depois dele fica no item.
        let xl = parse_xl("resource:\n  patch:\n    b.mesh: { props: [ 'a''b, c' ], targets: [ y.mesh ] }\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.patches[0].props, vec!["a'b, c"]);
        let e = parse_xl("factories:\n\t- a.csv\n").unwrap_err();
        assert!(e.starts_with("linha 2: tab na indentação"), "{e}");
        let e = parse_xl("resource:\n  fix:\n    a.ink: &F\n      paths:\n        x: y\n    b.ink:\n      <<: *F\n").unwrap_err();
        assert!(e.starts_with("linha 7: merge `<<`"), "{e}");
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

    /// `resource.fix.context` vai à tabela (`<recurso> | context | <parâmetro> | <valor>`), que o runtime entrega ao estado de mesh nativo: sem aviso,
    /// e o fix fica com names, paths e context.
    #[test]
    fn fix_com_context_vai_a_tabela_sem_aviso() {
        let com = parse_xl(
            "resource:\n  fix:\n    a.mesh:\n      names:\n        m1: m2\n      context:\n        p: v\n        q: w\n    b.mesh:\n      paths:\n        x.app: y.app\n",
        )
        .unwrap();
        assert_eq!(com.fixes.len(), 2);
        assert!(com.issues.is_empty(), "{:?}", com.issues);
        assert!(com.sem_efeito_no_bwms.is_empty(), "{:?}", com.sem_efeito_no_bwms);
        assert_eq!(com.fixes[0].context, vec![("p".to_string(), "v".to_string()), ("q".to_string(), "w".to_string())]);
        assert!(!com.summary().contains("formato inválido"), "{}", com.summary());
        let sem = parse_xl("resource:\n  fix:\n    a.mesh:\n      names:\n        m1: m2\n    b.mesh:\n      paths:\n        x.app: y.app\n").unwrap();
        assert!(sem.issues.is_empty(), "{:?}", sem.issues);
        assert!(sem.sem_efeito_no_bwms.is_empty(), "{:?}", sem.sem_efeito_no_bwms);
        let tabela = crate::instalacao::tabelas_de_runtime::emit_resfixes(&com);
        assert!(tabela.contains("a.mesh | context | p | v\na.mesh | context | q | w\n"), "{tabela}");
        assert!(tabela.starts_with(&crate::instalacao::tabelas_de_runtime::emit_resfixes(&sem)[..crate::instalacao::tabelas_de_runtime::emit_resfixes(&sem).find("a.mesh").unwrap_or(0)]));
    }

    #[test]
    fn localization_multi_grupo_e_extend() {
        let src = "localization:\n  onscreens:\n    en-us: a.json\n  subtitles:\n    en-us: b.json\n  extend: base\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.localization.len(), 2);
        assert_eq!(xl.localization_extend, Some("base".to_string()));
    }

    /// Lista ou valor sem idioma é forma que o formato aceita (vira en-us) e que o parser ainda não
    /// lê: vai para `sem_efeito_no_bwms`, não para `issues`.
    #[test]
    fn localization_sem_idioma_e_limite_do_bwms_e_nao_formato_invalido() {
        let xl = parse_xl("localization:\n  subtitles:\n    - m\\sub.json\n  onscreens: m\\on.json\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.sem_efeito_no_bwms.len(), 2, "{:?}", xl.sem_efeito_no_bwms);
        assert!(xl.sem_efeito_no_bwms[0].starts_with("localization.onscreens: lista ou valor sem idioma"), "{}", xl.sem_efeito_no_bwms[0]);
        assert!(xl.sem_efeito_no_bwms[1].starts_with("localization.subtitles: o BWMS ainda só aplica onscreens"), "{}", xl.sem_efeito_no_bwms[1]);
        assert!(xl.localization.is_empty());
    }

    #[test]
    fn localization_codigo_desconhecido_vira_aviso_e_nao_entra_na_tabela() {
        // `PT-BR` (caixa errada) e `db-db` (debug) não são idiomas de jogador; o primeiro válido na ordem do .xl é a reserva do runtime
        let src = "localization:\n  onscreens:\n    PT-BR: m\\pt.json\n    db-db: m\\db.json\n    de-de: m\\de.json\n    pt-br: m\\pt-br.json\n  subtitles:\n    xx-xx: m\\sub.json\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.issues.len(), 3, "{:?}", xl.issues);
        for (i, codigo) in ["PT-BR", "db-db", "xx-xx"].iter().enumerate() {
            assert!(xl.issues[i].contains(&format!("código de idioma desconhecido \"{codigo}\" (idioma ignorado); use um dos códigos")), "{}", xl.issues[i]);
        }
        assert_eq!(xl.localization.len(), 1, "grupo só com código desconhecido não entra");
        let idiomas: Vec<&str> = xl.localization[0].entries.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(idiomas, vec!["de-de", "pt-br"]);
        let tabela = crate::instalacao::tabelas_de_runtime::emit_localization_table(&xl, "m");
        let linhas: Vec<&str> = tabela.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["m | onscreens | de-de | m\\de.json", "m | onscreens | pt-br | m\\pt-br.json"]);
    }

    // ===== overrides.tags (Garment ChunkMask, RE 2026-07-15) =====

    #[test]
    fn garment_overrides_forma_sequencia_e_hide_implicito() {
        // sequência nua = hide implícito, sem `set` explícito.
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
    fn garment_overrides_chunk_64_em_diante_tira_o_componente() {
        // 64 cabe em u8 e é recusado pela primitiva; 256 nem vira u8. Nenhum dos dois vira hide-all nem outro chunk.
        let src = "overrides:\n  tags:\n    t:\n      c0: [ 63 ]\n      c1: [ 64 ]\n      c2:\n        show: [ 0, 64 ]\n      c3: [ 255 ]\n      c4: [ 256 ]\n";
        let xl = parse_xl(src).unwrap();
        assert_eq!(xl.garment_overrides[0].components, vec![("c0".to_string(), ChunkMask { show: false, mask: !(1u64 << 63) })]);
        assert_eq!(xl.issues.len(), 4, "{:?}", xl.issues);
        for (issue, comp) in xl.issues.iter().zip(["c1", "c2", "c3", "c4"]) {
            assert!(issue.starts_with(&format!("overrides.tags.t.{comp}: chunk que não é número de 0 a 63 (componente ignorado)")), "{issue}");
        }
    }

    #[test]
    fn garment_overrides_forma_escalar() {
        // escalar = máscara já-computada, crua (sem inversão: show=false).
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

    // ===== quest.phases =====

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

    // ===== streaming.sectors =====

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

        // forma com 4 valores: o 4º é aceito mas IGNORADO (w continua 0)
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
    fn streaming_sinonimos_de_sub_node_acumulam() {
        // actor + instance somam as listas; a contagem é a do último sinônimo lido com sucesso. Nas deleções, um item fora do range no 2º apaga tudo.
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeMutations:\n        - type: t\n          index: 0\n          expectedActors: 3\n          actorMutations:\n            - index: 1\n              scale: [2.0, 2.0, 2.0]\n          expectedInstances: 4\n          instanceMutations:\n            - index: 2\n              scale: [1.0, 1.0, 1.0]\n      nodeDeletions:\n        - type: t\n          index: 1\n          expectedActors: 2\n          actorDeletions: [0, 1]\n          expectedInstances: 2\n          instanceDeletions: [0]\n        - type: t\n          index: 2\n          expectedActors: 2\n          actorDeletions: [0, 1]\n          expectedInstances: 2\n          instanceDeletions: [7]\n";
        let st = parse_xl(src).unwrap().streaming.unwrap();
        let m = &st.sectors[0].node_mutations[0];
        assert_eq!(m.sub_node_mutations.iter().map(|s| s.sub_node_index).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(m.expected_sub_nodes, 4);
        let d = &st.sectors[0].node_deletions;
        assert_eq!((d[0].sub_node_deletions.clone(), d[0].expected_sub_nodes), (vec![0, 1, 0], 2));
        assert_eq!((d[1].sub_node_deletions.clone(), d[1].expected_sub_nodes), (vec![], 2));
    }

    #[test]
    fn streaming_subdelecoes_sem_expected_count_e_ignorada() {
        // actorDeletions presente mas SEM expectedActors: nenhuma sub-deleção entra
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

    /// Patch de `.inkanim` entra no modelo como os outros e sem aviso: o runtime aplica (`interface::mesclar_inkanim`).
    #[test]
    fn patch_de_inkanim_entra_sem_aviso() {
        let xl = parse_xl("resource:\n  patch:\n    mod\\x.inkanim:\n      - base\\gameplay\\gui\\a.inkanim\n    mod\\y.mesh:\n      - base\\b.mesh\n").unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert_eq!(xl.patches.len(), 2);
        assert!(xl.sem_efeito_no_bwms.is_empty(), "{:?}", xl.sem_efeito_no_bwms);
    }

    /// `[ator, shape]` em `actorDeletions`: o par entra no `shape_deletions`, o índice simples no `sub_node_deletions`, e a
    /// tabela leva uma linha `subshape` (o node só com pares não vira linha de node inteiro).
    #[test]
    fn streaming_par_ator_shape_vai_a_tabela_sem_apagar_o_node() {
        let src = "streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: worldCollisionNode\n          index: 1\n          expectedActors: 5\n          actorDeletions: [2, [4, 1]]\n        - type: worldCollisionNode\n          index: 2\n          expectedActors: 5\n          actorDeletions: [[3, 0]]\n";
        let xl = parse_xl(src).unwrap();
        assert!(xl.issues.is_empty(), "{:?}", xl.issues);
        assert!(xl.sem_efeito_no_bwms.is_empty(), "{:?}", xl.sem_efeito_no_bwms);
        let d = &xl.streaming.as_ref().unwrap().sectors[0].node_deletions;
        assert_eq!(d.len(), 2);
        assert_eq!((d[0].node_index, d[0].sub_node_deletions.clone(), d[0].shape_deletions.clone(), d[0].expected_sub_nodes), (1, vec![2], vec![(4, 1)], 5));
        assert_eq!((d[1].sub_node_deletions.clone(), d[1].shape_deletions.clone()), (vec![], vec![(3, 0)]));
        let tabela = crate::instalacao::tabelas_de_runtime::emit_streaming_table(&xl, "m");
        let linhas: Vec<&str> = tabela.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas.len(), 3, "{tabela}");
        assert!(linhas[0].ends_with("| 1 | worldCollisionNode | subdel | 5 2"), "{}", linhas[0]);
        assert!(linhas[1].ends_with("| 1 | worldCollisionNode | subshape | 5 4 1"), "{}", linhas[1]);
        assert!(linhas[2].ends_with("| 2 | worldCollisionNode | subshape | 5 3 0"), "{}", linhas[2]);
        // Em instanceDeletions o par não é forma aceita: a lista cai inteira.
        let inst = parse_xl("streaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - type: worldInstancedMeshNode\n          index: 2\n          expectedInstances: 5\n          instanceDeletions: [[3, 0]]\n").unwrap();
        assert_eq!(inst.issues.len(), 1, "{:?}", inst.issues);
    }
}
