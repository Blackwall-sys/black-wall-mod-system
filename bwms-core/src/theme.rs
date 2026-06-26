//! Eixo TEMÁTICO do mod (roupa/cabelo/carro/...) — complementa o eixo técnico (classify.rs).
//! "tool sugere, usuário confirma": `suggest()` chuta o tema por palavras-chave; o usuário decide.
//! Também o modelo de ESTADO (ativo/inativo/favorito/ordem) persistido em `.cp77-mods/bwms-mods.json`
//! (zero-dep, JSON na mão). Esse JSON é o CONTRATO que a UI in-game vai ler/escrever (via ponte Codeware).

use crate::classify::ModReport;
use std::path::Path;

/// Uma CATEGORIA (as "abas" da página de Mods). Eixo ortogonal ao FileKind técnico.
/// O conjunto CURADO abaixo (Nexus-inspired + as nossas: LUT, Cheats) dá rótulo bonito,
/// ordem das abas e palavras-chave pro auto-sugerir. Categorias CUSTOM também valem de pleno
/// direito: basta o usuário criar `BWMS/mods/<slug>/` — vira categoria nova SEM recompilar
/// (rótulo = slug em Title Case, sem keywords). `apply::reconcile` varre TODA subpasta de
/// BWMS/mods/, então uma pasta desconhecida NÃO colapsa em "outros" — é preservada como ela mesma.
pub struct Category {
    pub slug: &'static str,
    pub label: &'static str,
    pub keywords: &'static [&'static str],
}

/// slug de fallback quando nada casa / categoria ausente no JSON.
pub const FALLBACK: &str = "outros";

/// Conjunto CURADO. A ORDEM tem dupla função: ordem das abas na UI E prioridade de desempate no
/// `suggest` (mais específico primeiro; em empate de nº de hits, o de cima vence). Por isso as
/// genéricas (visual/texturas) ficam DEPOIS das específicas, p/ não roubar de veículo/arma/etc.
/// As 9 primeiras mantêm a ordem ORIGINAL provada; áudio/interface/animações/visual são novas.
pub const CATEGORIES: &[Category] = &[
    Category { slug: "npc",       label: "NPCs",            keywords: &["npc", "companion"] },
    Category { slug: "veiculos",  label: "Veículos",        keywords: &["vehicle", "quadra", "caliburn", "rayfield", "porsche", "kusanagi", "yaiba", "brennan", "apollo", "mizutani", "thorton", "motorcycle", " moto", "motorbike", " bike", "paintjob", "paint job", "reskin car", " car ", "supercar", "aerondight", "mordred", " arch ", "nazare"] },
    Category { slug: "armas",     label: "Armas",           keywords: &["weapon", "katana", "pistol", "rifle", "shotgun", "revolver", "scope", "sword", "blade", "lizzie", "malorian", "scalpel", "skippy", "yasha", "copperhead", "nekomata", "iconic weapon", "gun "] },
    Category { slug: "cabelos",   label: "Cabelos",         keywords: &["hair", "hairstyle", "haircut", "beard", "ponytail", "braid", "cabelo"] },
    Category { slug: "roupas",    label: "Roupas",          keywords: &["jacket", "coat", "trench", "outfit", "clothing", "shirt", "pants", "trousers", "dress", "apparel", "shorts", "hotpants", "vest", "jumpsuit", "bodysuit", "wardrobe", "corpocore", "nomadcore", "netrunner jacket", "roupa", "jaqueta"] },
    Category { slug: "lut",       label: "LUT/Cor",         keywords: &["lut", "reshade", "nova", "preem", "color grade", "colorgrade", "gamma", "tonemap", "cinematic", "grading", "color preset"] },
    Category { slug: "clima",     label: "Clima",           keywords: &["weather", "lighting", "timecycle", "climate", "clima", "rain", "fog", "skybox"] },
    Category { slug: "mundo",     label: "Mundo",           keywords: &["poster", "billboard", " sign", "signage", " tv ", "remaster", "environment", "props", "street", "building", "neon", "advertis", "world textures"] },
    Category { slug: "estetica",  label: "Estética (V)",    keywords: &["complexion", "skin", " face", "makeup", "freckle", "scar", "tattoo", " eye", "eyebrow", "brow", "body texture", "pele", "rosto", "smoother"] },
    Category { slug: "animacoes", label: "Animações",       keywords: &["animation", "anim ", "idle", "emote", " pose", "gesture", "locomotion", "mocap", "walk cycle", "photomode pose"] },
    Category { slug: "audio",     label: "Áudio",           keywords: &["audio", "music", "soundtrack", " ost", "song", "sfx", "sound effect", "voice", "radio station", " wem", "sound replacer"] },
    Category { slug: "interface", label: "Interface",       keywords: &["hud", "ui ", "interface", " menu", "minimap", "map marker", "inkatlas", "font", "crosshair", "widget", "quickhack ui"] },
    Category { slug: "visual",    label: "Visual/Texturas", keywords: &["texture", "retexture", " hd ", "2k", "4k", "8k", "upscale", "material", "mesh", "model replacer", "quality mod"] },
    Category { slug: "gameplay",  label: "Gameplay",        keywords: &["balance", "difficulty", "combat", "economy", " ai ", "cyberware", "stamina", "loot", "spawn rate", "overhaul", "gameplay", "rebalance", "perk"] },
    Category { slug: "cheats",    label: "Cheats",          keywords: &["cheat", "trainer", "godmode", "god mode", "unlimited", "infinite", "unlock all", "money"] },
    Category { slug: "outros",    label: "Outros",          keywords: &[] },
];

/// Acha a definição curada de um slug (None = categoria custom do usuário).
pub fn curated(slug: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.slug == slug)
}

/// Rótulo bonito de um slug: curado → label da tabela; custom → slug em Title Case.
pub fn category_label(slug: &str) -> String {
    match curated(slug) {
        Some(c) => c.label.to_string(),
        None => title_case(slug),
    }
}

/// "minha-categoria_nova" → "Minha Categoria Nova" (rótulo de categoria custom).
fn title_case(slug: &str) -> String {
    slug.split(|c| c == '-' || c == '_' || c == ' ')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut ch = w.chars();
            match ch.next() {
                Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Nomes de NPC = sinal FORTE: se aparecem, o mod é sobre aquele personagem (override do hit-count).
/// (Jackie fica de fora de propósito — "Jackie's Arch" é a MOTO dele, não a aparência.)
const NPC_NAMES: &[&str] = &[
    "panam", "judy", "river ward", " river ", "johnny", "takemura", "kerry", "rogue", "goro", "misty", "claire", "viktor",
];

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub category: String, // slug (curado OU custom)
    pub confidence: u8,   // 0..=95
    pub reason: String,   // quais palavras casaram
}

/// "Feno" pra busca: nome do mod + nomes de todos os arquivos, minúsculo, com bordas em espaço
/// pra os matches de " car "/" tv " pegarem palavra inteira.
fn haystack(report: &ModReport) -> String {
    let mut s = String::with_capacity(256);
    s.push(' ');
    s.push_str(&report.name.to_ascii_lowercase());
    for f in &report.files {
        s.push(' ');
        s.push_str(&f.rel.to_string_lossy().to_ascii_lowercase());
    }
    s.push(' ');
    // normaliza separadores p/ espaço (underscores/hifens viram fronteira de palavra)
    s.chars().map(|c| if c == '_' || c == '-' || c == '/' || c == '.' { ' ' } else { c }).collect()
}

/// Sugere uma categoria por palavras-chave. Conta hits por categoria; a de mais hits vence
/// (desempate = ordem em CATEGORIES, mais específica primeiro). Só sugere categorias CURADAS;
/// o usuário pode sempre corrigir pra qualquer slug (inclusive custom).
pub fn suggest(report: &ModReport) -> Suggestion {
    let hay = haystack(report);
    // nome de NPC = override forte (mod de personagem nomeado vence face/pele genérico)
    let npc: Vec<&str> = NPC_NAMES.iter().copied().filter(|w| hay.contains(*w)).collect();
    if !npc.is_empty() {
        return Suggestion { category: "npc".into(), confidence: 90, reason: format!("personagem: {}", npc.join(", ")) };
    }
    let mut best: Option<(&'static str, Vec<&str>)> = None;
    for cat in CATEGORIES {
        let hits: Vec<&str> = cat.keywords.iter().copied().filter(|w| hay.contains(*w)).collect();
        if hits.is_empty() {
            continue;
        }
        let better = match &best {
            None => true,
            Some((_, cur)) => hits.len() > cur.len(),
        };
        if better {
            best = Some((cat.slug, hits));
        }
    }
    match best {
        Some((slug, hits)) => {
            let conf = (hits.len() as u32 * 35).min(95) as u8;
            Suggestion { category: slug.to_string(), confidence: conf, reason: format!("casou: {}", hits.join(", ")) }
        }
        None => Suggestion { category: FALLBACK.into(), confidence: 0, reason: "nenhuma palavra-chave casou".into() },
    }
}

/// Estado de UM mod na página (o CONTRATO com a UI in-game).
#[derive(Debug, Clone)]
pub struct ModState {
    pub name: String,
    pub category: String, // slug da categoria (curada OU custom = nome da pasta em BWMS/mods/)
    pub active: bool,
    pub favorite: bool,
    pub order: i32,
    pub variant: String, // p/ packs com variantes (ex.: cor da pintura); "" = única
}

fn state_path(game: &Path) -> std::path::PathBuf {
    game.join(".cp77-mods").join("bwms-mods.json")
}

fn json_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            _ => o.push(c),
        }
    }
    o
}

/// Serializa a lista de estados em JSON (array de objetos) — pretty o suficiente p/ humano ler.
pub fn to_json(states: &[ModState]) -> String {
    let mut s = String::from("[\n");
    for (i, m) in states.iter().enumerate() {
        s.push_str(&format!(
            "  {{\"name\":\"{}\",\"category\":\"{}\",\"active\":{},\"favorite\":{},\"order\":{},\"variant\":\"{}\"}}",
            json_escape(&m.name), json_escape(&m.category), m.active, m.favorite, m.order, json_escape(&m.variant)
        ));
        s.push_str(if i + 1 < states.len() { ",\n" } else { "\n" });
    }
    s.push(']');
    s
}

pub fn save_states(game: &Path, states: &[ModState]) -> std::io::Result<()> {
    let p = state_path(game);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&p, to_json(states))
}

// ---- leitura tolerante (parse na mão dos campos do nosso próprio formato) ----

/// extrai o valor string de `"key":"..."` a partir de `obj` (o trecho entre { }).
fn field_str(obj: &str, key: &str) -> Option<String> {
    let pat = format!("\"{key}\":\"");
    let start = obj.find(&pat)? + pat.len();
    let rest = &obj[start..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(n) = chars.next() {
                    out.push(match n {
                        'n' => '\n',
                        other => other,
                    });
                }
            }
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

fn field_bool(obj: &str, key: &str) -> Option<bool> {
    let pat = format!("\"{key}\":");
    let start = obj.find(&pat)? + pat.len();
    let rest = obj[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn field_int(obj: &str, key: &str) -> Option<i32> {
    let pat = format!("\"{key}\":");
    let start = obj.find(&pat)? + pat.len();
    let rest = obj[start..].trim_start();
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '-')).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// Parser tolerante do nosso JSON (array de objetos planos). Quebra por '}' — seguro porque
/// não há '}' aninhado nos nossos objetos.
pub fn from_json(s: &str) -> Vec<ModState> {
    let mut out = Vec::new();
    for chunk in s.split('}') {
        if let Some(open) = chunk.find('{') {
            let obj = &chunk[open + 1..];
            if let Some(name) = field_str(obj, "name") {
                out.push(ModState {
                    name,
                    // aceita a chave nova "category" e a antiga "theme" (back-compat de estados salvos)
                    category: field_str(obj, "category")
                        .or_else(|| field_str(obj, "theme"))
                        .unwrap_or_else(|| FALLBACK.to_string()),
                    active: field_bool(obj, "active").unwrap_or(false),
                    favorite: field_bool(obj, "favorite").unwrap_or(false),
                    order: field_int(obj, "order").unwrap_or(0),
                    variant: field_str(obj, "variant").unwrap_or_default(),
                });
            }
        }
    }
    out
}

pub fn load_states(game: &Path) -> Vec<ModState> {
    match std::fs::read_to_string(state_path(game)) {
        Ok(s) => from_json(&s),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{FileEntry, FileKind, ModClass, Compat, ModReport};
    use std::path::PathBuf;

    fn rep(name: &str, files: &[&str]) -> ModReport {
        ModReport {
            name: name.into(),
            class: ModClass::PureContent,
            compat: Compat::Universal,
            files: files.iter().map(|f| FileEntry { rel: PathBuf::from(f), kind: FileKind::Archive }).collect(),
            deps: vec![],
            risks: vec![],
            notes: vec![],
        }
    }

    #[test]
    fn sugere_veiculo() {
        assert_eq!(suggest(&rep("Quadra Turbo R V-Tec E3 Recolor", &[])).category, "veiculos");
        assert_eq!(suggest(&rep("Free Black Caliburn Reskin", &[])).category, "veiculos");
        assert_eq!(suggest(&rep("Jackie's Arch Recolor", &["arch_nazare.archive"])).category, "veiculos");
    }
    #[test]
    fn sugere_npc_e_estetica() {
        assert_eq!(suggest(&rep("Panam scar and freckles", &[])).category, "npc");
        assert_eq!(suggest(&rep("4K Detailed Complexion makeup", &[])).category, "estetica");
    }
    #[test]
    fn sugere_roupa_arma_mundo() {
        assert_eq!(suggest(&rep("Black Leather Trenchcoat", &[])).category, "roupas");
        assert_eq!(suggest(&rep("Arasaka Thermal Katana Errata Replacer", &[])).category, "armas");
        assert_eq!(suggest(&rep("Posters Remastered", &[])).category, "mundo");
    }
    #[test]
    fn sugere_categorias_novas() {
        // áudio / interface / animações / visual — os gaps vs Nexus que antes caíam em "outros"
        assert_eq!(suggest(&rep("Samurai Radio Station soundtrack replacer", &["music.wem"])).category, "audio");
        assert_eq!(suggest(&rep("Cleaner Minimap HUD widget", &[])).category, "interface");
        assert_eq!(suggest(&rep("Smooth Idle Animation overhaul", &[])).category, "animacoes");
        assert_eq!(suggest(&rep("8K Street Retexture Pack", &["road_texture.xbm"])).category, "visual");
    }
    #[test]
    fn outros_quando_sem_pista() {
        assert_eq!(suggest(&rep("zzz mystery blob", &[])).category, "outros");
    }
    #[test]
    fn rotulo_curado_e_custom() {
        assert_eq!(category_label("audio"), "Áudio");
        assert_eq!(category_label("lut"), "LUT/Cor");
        // categoria CUSTOM (não está na tabela): rótulo = slug em Title Case, preservado
        assert_eq!(category_label("minha-categoria_nova"), "Minha Categoria Nova");
        assert!(curated("minha-categoria_nova").is_none());
    }
    #[test]
    fn json_roundtrip() {
        let states = vec![
            ModState { name: "Caliburn \"Red\"".into(), category: "veiculos".into(), active: true, favorite: true, order: 0, variant: "vermelho".into() },
            // categoria CUSTOM sobrevive ao round-trip (não colapsa em "outros")
            ModState { name: "Mod Estranho".into(), category: "minha-cat".into(), active: false, favorite: false, order: 3, variant: "".into() },
        ];
        let js = to_json(&states);
        let back = from_json(&js);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].name, "Caliburn \"Red\"");
        assert_eq!(back[0].category, "veiculos");
        assert!(back[0].active && back[0].favorite);
        assert_eq!(back[0].variant, "vermelho");
        assert_eq!(back[1].category, "minha-cat");
        assert_eq!(back[1].order, 3);
        assert!(!back[1].active);
    }
    #[test]
    fn back_compat_le_chave_theme_antiga() {
        // estado salvo na versão antiga usava "theme": deve continuar carregando
        let old = r#"[{"name":"X","theme":"veiculos","active":true,"favorite":false,"order":0,"variant":""}]"#;
        let back = from_json(old);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].category, "veiculos");
    }
}
