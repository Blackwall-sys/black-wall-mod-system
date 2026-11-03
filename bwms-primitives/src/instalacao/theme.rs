//! Eixo TEMÁTICO do mod (roupa/cabelo/carro/...) — complementa o eixo técnico (classify.rs).
//! "tool sugere, usuário confirma": `suggest()` chuta o tema por palavras-chave; o usuário decide.
//! Também o LEITOR do estado (`.cp77-mods/bwms-mods.json`, zero-dep, JSON pelo `crate::json`) que a biblioteca da 0.1.2–0.1.4 gravava:
//! só a migração dela (`migracao`) o lê, para levar à `Mods/` os mods que estavam ligados. O `gravar_atomico` das tabelas de `red4ext/`
//! mora aqui também.

use crate::instalacao::classify::ModReport;
use crate::json::Valor;
use std::path::Path;

/// Uma CATEGORIA (o rótulo do mod no `bwms classify`/`bwms suggest`). Eixo ortogonal ao FileKind técnico.
/// O conjunto CURADO abaixo (Nexus-inspired + as nossas: LUT, Cheats) dá rótulo bonito, desempate e
/// palavras-chave pro auto-sugerir. Slug fora da tabela vale como categoria CUSTOM (rótulo = slug em Title Case, sem keywords).
pub struct Category {
    pub slug: &'static str,
    pub label: &'static str,
    /// Forma de cada palavra-chave: ver `casamentos`.
    pub keywords: &'static [&'static str],
}

/// slug de fallback quando nada casa / categoria ausente no JSON.
pub const FALLBACK: &str = "outros";

/// Conjunto CURADO. A ORDEM é a prioridade de desempate no `suggest` (mais específico primeiro; em empate de palavras casadas, o de
/// cima vence). Por isso as genéricas (visual/texturas) ficam DEPOIS das específicas, p/ não roubar de veículo/arma/etc.
/// As 9 primeiras mantêm a ordem ORIGINAL provada; áudio/interface/animações/visual são novas.
pub const CATEGORIES: &[Category] = &[
    Category { slug: "npc",       label: "NPCs",            keywords: NPC_KEYWORDS },
    Category { slug: "veiculos",  label: "Veículos",        keywords: &["vehicle", "quadra", "caliburn", "rayfield", "porsche", "kusanagi", "yaiba", "brennan", "apollo", "mizutani", "thorton", "motorcycle", "moto", "motorbike", "bike", "paintjob", "paint job", "reskin car", " car ", "supercar", "aerondight", "mordred", " arch ", "nazare"] },
    Category { slug: "armas",     label: "Armas",           keywords: &["weapon", "katana", "pistol", "rifle", "shotgun", "revolver", "scope", "sword", "blade", "lizzie", "malorian", "scalpel", "skippy", "yasha", "copperhead", "nekomata", "iconic weapon", "*gun "] },
    Category { slug: "cabelos",   label: "Cabelos",         keywords: &["hair", "hairstyle", "haircut", "beard", "ponytail", "braid", "cabelo"] },
    Category { slug: "roupas",    label: "Roupas",          keywords: &["*jacket", "*coat", "trench", "outfit", "clothing", "*shirt", "*pants", "trousers", "dress", "apparel", "shorts", "vest", "jumpsuit", "bodysuit", "wardrobe", "corpocore", "nomadcore", "roupa", "jaqueta"] },
    Category { slug: "lut",       label: "LUT/Cor",         keywords: &["lut", "reshade", "nova", "preem", "color grade", "colorgrade", "gamma", "tonemap", "cinematic", "grading", "color preset"] },
    Category { slug: "clima",     label: "Clima",           keywords: &["weather", "lighting", "timecycle", "climate", "clima", "rain", "fog", "skybox"] },
    Category { slug: "mundo",     label: "Mundo",           keywords: &["poster", "billboard", "sign", "signage", " tv ", "remaster", "environment", "props", "street", "building", "neon", "advertis", "world textures"] },
    Category { slug: "estetica",  label: "Estética (V)",    keywords: &["complexion", "skin", "face", "makeup", "freckle", "scar", "tattoo", "eye", "eyebrow", " brow ", " brows ", "body texture", "pele", "rosto", "smoother"] },
    Category { slug: "animacoes", label: "Animações",       keywords: &["animation", " anim ", "idle", "emote", "pose", "gesture", "locomotion", "mocap", "walk cycle", "photomode pose"] },
    Category { slug: "audio",     label: "Áudio",           keywords: &["audio", "music", "soundtrack", "ost", "song", "sfx", "sound effect", "voice", "radio station", "wem", "sound replacer"] },
    // `*hud`: sigla colada em outra toda maiúscula ("LHUD") não tem fronteira dentro do token
    Category { slug: "interface", label: "Interface",       keywords: &["*hud", " ui ", "interface", "menu", "minimap", "map marker", "inkatlas", "font", "crosshair", "widget", "quickhack ui"] },
    Category { slug: "visual",    label: "Visual/Texturas", keywords: &["texture", "retexture", " hd ", "2k", "4k", "8k", "upscale", "material", "mesh", "model replacer", "quality mod"] },
    Category { slug: "gameplay",  label: "Gameplay",        keywords: &["balance", "difficulty", "combat", "economy", " ai ", "cyberware", "stamina", "loot", "autoloot", "spawn rate", "overhaul", "gameplay", "rebalance", "perk"] },
    Category { slug: "cheats",    label: "Cheats",          keywords: &["cheat", "trainer", "godmode", "god mode", "unlimited", "infinite", " unlock all ", "money"] },
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

/// Nomes de NPC = sinal FORTE: se aparecem, o mod é sobre aquele personagem e a categoria é `npc`,
/// qualquer que seja a contagem das outras. Mesma forma das palavras-chave (`casamentos`).
/// (Jackie fica de fora de propósito — "Jackie's Arch" é a MOTO dele, não a aparência.)
const NPC_NAMES: &[&str] = &[
    "panam", "judy", "river ward", " river ", "johnny", "takemura", "kerry", "rogue", "goro", "misty", "claire", "viktor",
];

/// Palavras-chave da categoria `npc`. Quando casa nome de NPC, `suggest` conta estas junto dos nomes.
/// `*npc`: sigla colada em outra toda maiúscula ("AMMNPCs") não tem fronteira dentro do token.
const NPC_KEYWORDS: &[&str] = &["*npc", "companion"];

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub category: String, // slug (curado OU custom)
    /// Palavras do feno que casaram com a categoria (`palavras_casadas`). É contagem, não probabilidade:
    /// nada mede o acerto da sugestão.
    pub matched_words: u8,
    pub reason: String, // quais palavras do feno casaram
}

/// Fronteira dentro de um token do feno, antes do byte da mesma posição. Em todas a palavra-chave pode
/// começar ou passar por cima, porque o feno não sabe se ali começa outra palavra ou não.
#[derive(Clone, Copy, PartialEq)]
enum Fronteira {
    Nenhuma,
    /// Minúscula → maiúscula. CamelCase separa palavras (`Better|Vehicle`) e também parte palavra composta
    /// (`Re|Shade`, `Mini|Map`, `Copper|Head`): `reshade` atravessa, ` car ` acaba aqui ("CarMod") e o
    /// espaço de uma palavra-chave casa aqui (`spawn rate` casa "SpawnRate").
    Camelo,
    /// Antes da última maiúscula de uma sigla seguida de minúscula (`HD·Textures`, `NEON·Jacket`): a
    /// minúscula abre palavra (`Textures`) ou é sufixo da sigla (`HUDless` casa `hud`, `NPCs` casa `npc`).
    /// Palavra inteira não acaba aqui (`HDRless` não casa ` hd `).
    Sigla,
    /// Dígito → maiúscula (`2077:Textures`, `V2:Hair`, `2077:NPCs`): a maiúscula abre a palavra seguinte, como
    /// depois de minúscula. Ela também pode ser unidade do número (`4:K`, `3:D`), então palavra inteira não
    /// acaba aqui (` 4 ` não casa "4K") e o espaço de uma palavra-chave não casa aqui.
    Digito,
}

/// Feno da busca: nome do mod e caminho de cada arquivo, em tokens ASCII minúsculos separados por um
/// espaço, com espaço nas duas pontas. Vira espaço: caractere fora de [A-Za-z0-9] e letra → dígito
/// (`RealisticMap4K` → `realistic|map 4:k`). Dígito → letra não separa, para `4k`/`8k` serem um token só.
struct Feno {
    texto: String,
    /// `fronteiras[p]`: a fronteira antes de `texto[p]`. Mesmo tamanho de `texto`, para olhar uma posição
    /// em O(1) num mod com milhares de arquivos.
    fronteiras: Vec<Fronteira>,
}

impl Feno {
    fn de(report: &ModReport) -> Feno {
        let mut feno = Feno { texto: String::from(" "), fronteiras: vec![Fronteira::Nenhuma] };
        feno.push_tokens(&report.name);
        for f in &report.files {
            feno.push_tokens(&f.rel.to_string_lossy());
        }
        feno
    }

    fn push_tokens(&mut self, s: &str) {
        // os dois últimos caracteres do token corrente (None no começo do token)
        let (mut ant, mut antes_de_ant): (Option<char>, Option<char>) = (None, None);
        for c in s.chars() {
            if !c.is_ascii_alphanumeric() {
                self.fecha_token();
                (ant, antes_de_ant) = (None, None);
                continue;
            }
            let mut fronteira = Fronteira::Nenhuma;
            match ant {
                Some(a) if a.is_ascii_alphabetic() && c.is_ascii_digit() => {
                    self.fecha_token();
                    ant = None;
                }
                Some(a) if a.is_ascii_lowercase() && c.is_ascii_uppercase() => fronteira = Fronteira::Camelo,
                Some(a) if a.is_ascii_digit() && c.is_ascii_uppercase() => fronteira = Fronteira::Digito,
                Some(a)
                    if a.is_ascii_uppercase() && c.is_ascii_lowercase() && antes_de_ant.is_some_and(|b| b.is_ascii_uppercase()) =>
                {
                    // `ant` é o último byte escrito: a fronteira fica antes dele
                    let p = self.fronteiras.len() - 1;
                    self.fronteiras[p] = Fronteira::Sigla;
                }
                _ => {}
            }
            self.texto.push(c.to_ascii_lowercase());
            self.fronteiras.push(fronteira);
            (ant, antes_de_ant) = (Some(c), ant);
        }
        self.fecha_token();
    }

    fn fecha_token(&mut self) {
        if !self.texto.ends_with(' ') {
            self.texto.push(' ');
            self.fronteiras.push(Fronteira::Nenhuma);
        }
    }

    /// Começo de token em `p`: depois de espaço ou numa fronteira.
    fn comeca_token(&self, p: usize) -> bool {
        p == 0 || self.texto.as_bytes()[p - 1] == b' ' || self.fronteiras.get(p).is_some_and(|&f| f != Fronteira::Nenhuma)
    }

    /// Fim de palavra inteira em `p`: antes de espaço ou de CamelCase, nunca na sigla nem depois de dígito.
    fn acaba_palavra(&self, p: usize) -> bool {
        self.texto.as_bytes().get(p).map_or(true, |&b| b == b' ') || self.fronteiras.get(p) == Some(&Fronteira::Camelo)
    }

    /// Fim das palavras de uma palavra-chave depois da primeira, que acabou em `q`. Cada uma começa logo depois
    /// de espaço ou de CamelCase, então a palavra de antes dela acaba junto com o token ou na CamelCase.
    fn segue<'a>(&self, mut q: usize, palavras: impl Iterator<Item = &'a str>) -> Option<usize> {
        for palavra in palavras {
            match self.texto.as_bytes().get(q) {
                Some(b' ') => q += 1,
                Some(_) if self.fronteiras[q] == Fronteira::Camelo => {}
                _ => return None,
            }
            if !self.texto[q..].starts_with(palavra) {
                return None;
            }
            q += palavra.len();
        }
        Some(q)
    }

    /// Começo do trecho que contém `p`: a fronteira (espaço ou `Fronteira`) mais próxima em `p` ou antes.
    fn inicio_do_trecho(&self, p: usize) -> usize {
        (0..=p).rev().find(|&i| self.comeca_token(i)).unwrap_or(0)
    }

    /// Fim (exclusivo) do trecho que acaba em `p` ou depois: a fronteira (espaço ou `Fronteira`) mais próxima.
    fn fim_do_trecho(&self, p: usize) -> usize {
        (p..self.texto.len())
            .find(|&i| self.texto.as_bytes()[i] == b' ' || self.fronteiras[i] != Fronteira::Nenhuma)
            .unwrap_or(self.texto.len())
    }
}

/// Trechos `(início, fim)` do feno que a palavra-chave casa, cada um esticado até a fronteira (espaço ou
/// `Fronteira`) mais próxima de cada ponta. Forma da palavra-chave (minúscula, palavras separadas por um espaço):
/// - sem marca: começa num início de token, e a última palavra casa pelo começo do token
///   (`trench` casa "trenchcoat", `lut` não casa "resolution", `paint job` casa "paint jobs" e não "repaint jobs");
/// - espaço no fim: a última palavra acaba junto com o token (` brow ` não casa "browser"). O espaço do
///   começo só acompanha o do fim, para a palavra inteira ficar legível;
/// - `*` na frente: começa em qualquer ponto do token (`*jacket` casa "femvjacket"; `*gun ` casa "handgun"
///   e não "burgundy").
///
/// As palavras do meio de uma palavra-chave são tokens inteiros. A palavra-chave passa por cima das
/// fronteiras de `Fronteira` (`reshade` casa "ReShade"), e o espaço dela casa espaço ou CamelCase.
fn casamentos(feno: &Feno, chave: &str) -> Vec<(usize, usize)> {
    let em_qualquer_ponto = chave.starts_with('*');
    let inteira = chave.ends_with(' ');
    let mut palavras = chave.trim_start_matches('*').split_whitespace();
    let mut trechos = Vec::new();
    let Some(primeira) = palavras.next() else {
        return trechos;
    };
    let mut desde = 0;
    while let Some(i) = feno.texto[desde..].find(primeira) {
        let p = desde + i;
        // um byte adiante, não depois da ocorrência: a seguinte pode sobrepor esta ("ReyEye" casa `eye` em `Eye`).
        // O feno é ASCII, então p + 1 é fronteira de caractere.
        desde = p + 1;
        if !em_qualquer_ponto && !feno.comeca_token(p) {
            continue;
        }
        let Some(fim) = feno.segue(p + primeira.len(), palavras.clone()) else {
            continue;
        };
        if inteira && !feno.acaba_palavra(fim) {
            continue;
        }
        let inicio = if em_qualquer_ponto { feno.inicio_do_trecho(p) } else { p };
        trechos.push((inicio, feno.fim_do_trecho(fim)));
    }
    trechos
}

/// Palavras do feno que as palavras-chave casam, uma vez cada, na ordem do feno. Palavras-chave com
/// trechos sobrepostos são a mesma palavra (`eye` e `eyebrow` em "eyebrows", `river ward` e ` river `), e
/// uma palavra-chave conta uma vez, mesmo flexionada entre o nome e o arquivo ("Judy" e "JudysOutfit",
/// "LUT" e "NovaLUT4"). Cada palavra aparece pelo primeiro trecho dela, o mais longo se dois começam
/// juntos ("river ward", não "river").
fn palavras_casadas(feno: &Feno, chaves: &[&str]) -> Vec<String> {
    // (início, fim, índice da palavra-chave) de todos os trechos, em ordem de início
    let mut trechos: Vec<(usize, usize, usize)> = chaves
        .iter()
        .enumerate()
        .flat_map(|(k, chave)| casamentos(feno, chave).into_iter().map(move |(a, b)| (a, b, k)))
        .collect();
    trechos.sort_unstable();
    // `palavra[k]`: a palavra da palavra-chave k. Em ordem de início, o trecho que começa antes do fim mais
    // à direita da cadeia corrente sobrepõe o trecho dono desse fim e entra na palavra da cadeia. Uma passada
    // ordenada, para o custo não ficar quadrático no número de arquivos do mod.
    let mut palavra: Vec<usize> = (0..chaves.len()).collect();
    let mut cadeia: Option<(usize, usize)> = None; // (fim mais à direita, palavra-chave de um trecho dela)
    for &(inicio, fim, k) in &trechos {
        match cadeia {
            Some((fim_da_cadeia, dona)) if inicio < fim_da_cadeia => {
                let (fica, sai) = (palavra[dona], palavra[k]);
                if fica != sai {
                    palavra.iter_mut().filter(|p| **p == sai).for_each(|p| *p = fica);
                }
                cadeia = Some((fim_da_cadeia.max(fim), dona));
            }
            _ => cadeia = Some((fim, k)),
        }
    }
    let mut primeiro: Vec<Option<(usize, usize)>> = vec![None; chaves.len()];
    for &(inicio, fim, k) in &trechos {
        let p = &mut primeiro[palavra[k]];
        if p.map_or(true, |(a, _)| a == inicio) {
            *p = Some((inicio, fim));
        }
    }
    let mut primeiros: Vec<(usize, usize)> = primeiro.into_iter().flatten().collect();
    primeiros.sort_unstable();
    primeiros.into_iter().map(|(a, b)| feno.texto[a..b].to_string()).collect()
}

/// Sugere uma categoria por palavras-chave. Vence a categoria com mais palavras casadas (desempate =
/// ordem em CATEGORIES, mais específica primeiro); nome de NPC decide `npc` sozinho. Só sugere
/// categorias CURADAS; o usuário pode sempre corrigir pra qualquer slug (inclusive custom).
pub fn suggest(report: &ModReport) -> Suggestion {
    let feno = Feno::de(report);
    let nomes = palavras_casadas(&feno, NPC_NAMES);
    if !nomes.is_empty() {
        // numa passada só, para nome e palavra-chave no mesmo token ("judynpc") contarem uma palavra
        let chaves: Vec<&str> = NPC_NAMES.iter().chain(NPC_KEYWORDS).copied().collect();
        let palavras = palavras_casadas(&feno, &chaves);
        let outras: Vec<&str> = palavras.iter().filter(|p| !nomes.contains(p)).map(String::as_str).collect();
        let matched_words = u8::try_from(palavras.len()).unwrap_or(u8::MAX);
        let mut reason = format!("personagem: {}", nomes.join(", "));
        if !outras.is_empty() {
            reason.push_str(&format!("; casou: {}", outras.join(", ")));
        }
        return Suggestion { category: "npc".into(), matched_words, reason };
    }
    let mut best: Option<(&'static str, Vec<String>)> = None;
    for cat in CATEGORIES {
        let palavras = palavras_casadas(&feno, cat.keywords);
        if palavras.len() > best.as_ref().map_or(0, |(_, atual)| atual.len()) {
            best = Some((cat.slug, palavras));
        }
    }
    match best {
        Some((slug, palavras)) => {
            let matched_words = u8::try_from(palavras.len()).unwrap_or(u8::MAX);
            Suggestion { category: slug.to_string(), matched_words, reason: format!("casou: {}", palavras.join(", ")) }
        }
        None => Suggestion { category: FALLBACK.into(), matched_words: 0, reason: "nenhuma palavra-chave casou".into() },
    }
}

/// De onde vinha um mod no estado da biblioteca da 0.1.2–0.1.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fonte {
    /// `BWMS/mods/<categoria>/<nome>/`.
    Staging,
    /// `BWMS/nexus/mods/<unique_id>/`.
    Biblioteca,
}

/// Estado de UM mod em `bwms-mods.json`, como a biblioteca o gravou.
#[derive(Debug, Clone, PartialEq)]
pub struct ModState {
    pub name: String, // staging: nome da pasta do mod; biblioteca: `unique_id`
    pub category: String, // slug da categoria (nome da pasta em BWMS/mods/); "" na biblioteca, que não tem categoria
    pub fonte: Fonte,
    /// O mod estava no jogo: o apply da biblioteca só gravava `true` junto com a cópia.
    pub active: bool,
    pub favorite: bool,
    pub order: i32,
    pub variant: String, // p/ packs com variantes (ex.: cor da pintura); "" = única
}

pub(crate) fn state_path(game: &Path) -> std::path::PathBuf {
    game.join(".cp77-mods").join("bwms-mods.json")
}

/// Versão do formato de `bwms-mods.json` que a biblioteca gravou por último. Arquivo com versão MAIOR não veio de nenhum BWMS publicado
/// e é recusado inteiro.
const STATE_SCHEMA: u32 = 2;

/// Grava `conteudo` em `dest` de forma atômica: escreve `<nome>.<pid>.tmp` no MESMO diretório, leva os dados
/// ao disco (`sync_all`) e troca pelo `rename` (atômico no mesmo volume). Processo morto, disco cheio, queda
/// de energia ou disco USB que cai no meio deixam o arquivo anterior inteiro, nunca vazio ou truncado, e quem
/// abre pelo caminho (o runtime relendo uma tabela de `red4ext/`) vê o arquivo velho ou o novo. O pid no nome
/// evita que dois processos escrevam no mesmo temporário. Cria a pasta de `dest`.
pub(crate) fn gravar_atomico(dest: &Path, conteudo: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let nome = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dest.with_file_name(format!("{nome}.{}.tmp", std::process::id()));
    // Sem o `sync_all` entre a escrita e o `rename`, o disco pode registrar o rename sem os dados. No macOS
    // ele é F_FULLFSYNC: ~5 ms por gravação no SSD USB (APFS), medido offline no lote F5-14.
    let gravado = std::fs::File::create(&tmp)
        .and_then(|mut f| {
            std::io::Write::write_all(&mut f, conteudo)?;
            f.sync_all()
        })
        .and_then(|()| std::fs::rename(&tmp, dest));
    if gravado.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    gravado
}

// ---- leitura estrita: arquivo fora do formato é recusado inteiro, nunca lido pela metade ----

/// Lê o estado de `bwms-mods.json`. Aceita o envelope `{"schema":N,"mods":[...]}` (N até
/// `STATE_SCHEMA`) e a lista nua sem versão, formato dos arquivos gravados sem o envelope.
/// Chave ausente vale o padrão (`category` → `theme` → `FALLBACK`, `fonte` → staging, `false`, `0`, `""`). Chave com tipo
/// errado, item que não é objeto ou sem `name` é erro: lido pela metade, um mod ligado sairia da migração como desligado.
pub fn from_json(s: &str) -> Result<Vec<ModState>, String> {
    let doc = crate::json::ler(s)?;
    let itens = match &doc {
        Valor::Lista(itens) => itens,
        Valor::Objeto(_) => {
            let schema = match doc.campo("schema") {
                Some(Valor::Numero(n)) => n.parse::<u32>().ok(),
                _ => None,
            };
            match schema {
                Some(v) if v > STATE_SCHEMA => {
                    return Err(format!(
                        "schema {v} é de um BWMS mais novo (esta versão lê até {STATE_SCHEMA}); atualize o BWMS"
                    ))
                }
                Some(v) if v >= 1 => {}
                _ => return Err("falta \"schema\" com inteiro positivo".into()),
            }
            match doc.campo("mods") {
                Some(Valor::Lista(itens)) => itens,
                _ => return Err("falta a lista \"mods\"".into()),
            }
        }
        _ => return Err("o topo não é lista nem objeto".into()),
    };
    itens.iter().enumerate().map(|(i, item)| mod_state(item, i)).collect()
}

fn mod_state(item: &Valor, i: usize) -> Result<ModState, String> {
    let Some(Valor::Texto(name)) = item.campo("name") else {
        return Err(format!("o item {} da lista não é objeto com \"name\" em texto", i + 1));
    };
    let erro_de_tipo = |chave: &str, tipo: &str| format!("\"{chave}\" do mod '{name}' não é {tipo}");
    let texto = |chave: &str| match item.campo(chave) {
        None => Ok(None),
        Some(Valor::Texto(s)) => Ok(Some(s.clone())),
        Some(_) => Err(erro_de_tipo(chave, "texto")),
    };
    let booleano = |chave: &str| match item.campo(chave) {
        None => Ok(false),
        Some(Valor::Bool(b)) => Ok(*b),
        Some(_) => Err(erro_de_tipo(chave, "true/false")),
    };
    let category = match texto("category")? {
        Some(c) => c,
        // estados salvos com a chave "theme" continuam carregando
        None => texto("theme")?.unwrap_or_else(|| FALLBACK.to_string()),
    };
    let order = match item.campo("order") {
        None => 0,
        Some(Valor::Numero(n)) => n.parse::<i32>().map_err(|_| erro_de_tipo("order", "inteiro de 32 bits"))?,
        Some(_) => return Err(erro_de_tipo("order", "número")),
    };
    // arquivo da versão 1 não tem a chave, e nele todo mod é do staging
    let fonte = match texto("fonte")?.as_deref() {
        None | Some("staging") => Fonte::Staging,
        Some("biblioteca") => Fonte::Biblioteca,
        Some(_) => return Err(erro_de_tipo("fonte", "\"staging\" ou \"biblioteca\"")),
    };
    Ok(ModState {
        name: name.clone(),
        category,
        fonte,
        active: booleano("active")?,
        favorite: booleano("favorite")?,
        order,
        variant: texto("variant")?.unwrap_or_default(),
    })
}

/// O que `load_states` achou. Só `Missing` quer dizer "nenhum mod ligado": tratar `Invalid`/`IoError` como lista vazia deixaria
/// fora da migração os mods que estavam no jogo.
#[derive(Debug)]
pub enum StateLoad {
    /// O arquivo não existe (a biblioteca nunca aplicou nada).
    Missing,
    Loaded(Vec<ModState>),
    /// O arquivo existe, mas não é UTF-8, não é JSON ou não segue o formato; o texto diz onde.
    Invalid(String),
    /// Erro de leitura que não é "não existe" (permissão, disco que caiu).
    IoError(std::io::Error),
}

pub fn load_states(game: &Path) -> StateLoad {
    let bytes = match std::fs::read(state_path(game)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StateLoad::Missing,
        Err(e) => return StateLoad::IoError(e),
    };
    let Ok(texto) = String::from_utf8(bytes) else {
        return StateLoad::Invalid("o arquivo não é texto UTF-8".into());
    };
    match from_json(&texto) {
        Ok(states) => StateLoad::Loaded(states),
        Err(detalhe) => StateLoad::Invalid(detalhe),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instalacao::classify::{FileEntry, FileKind, ModClass, Compat, ModReport};
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
            avisos_do_xl: vec![],
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

    fn feno(name: &str, files: &[&str]) -> Feno {
        Feno::de(&rep(name, files))
    }

    /// (categoria, palavras casadas, motivo) da sugestão.
    fn sugestao(name: &str, files: &[&str]) -> (String, u8, String) {
        let s = suggest(&rep(name, files));
        (s.category, s.matched_words, s.reason)
    }

    fn casou(name: &str, chave: &str) -> Vec<String> {
        let f = feno(name, &[]);
        casamentos(&f, chave).into_iter().map(|(a, b)| f.texto[a..b].to_string()).collect()
    }

    /// Texto do feno com as fronteiras à vista: `|` antes de `Camelo`, `·` antes de `Sigla`, `:` antes de `Digito`.
    fn marcado(name: &str, files: &[&str]) -> String {
        let f = feno(name, files);
        assert_eq!(f.fronteiras.len(), f.texto.len(), "{name}");
        f.texto
            .chars()
            .zip(&f.fronteiras)
            .map(|(c, fronteira)| match fronteira {
                Fronteira::Nenhuma => c.to_string(),
                Fronteira::Camelo => format!("|{c}"),
                Fronteira::Sigla => format!("·{c}"),
                Fronteira::Digito => format!(":{c}"),
            })
            .collect()
    }

    #[test]
    fn feno_separa_simbolo_e_letra_seguida_de_digito_e_marca_as_fronteiras() {
        assert_eq!(marcado("Retro Look (Jacket)", &[]), " retro look jacket ");
        assert_eq!(marcado("Jackie's Arch, Estética", &[]), " jackie s arch est tica ");
        assert_eq!(marcado("BetterVehicleHandling", &[]), " better|vehicle|handling ");
        // letra → dígito separa; dígito → letra não, para `4k`/`8k` continuarem um token
        assert_eq!(marcado("RealisticMap4K Sky8K 8k-4k", &[]), " realistic|map 4:k sky 8:k 8k 4k ");
        assert_eq!(marcado("X", &["archive/pc/mod/###-NovaLUT4.archive"]), " x archive pc mod nova|lut 4 archive ");
        // sigla seguida de minúscula: fronteira antes da última maiúscula
        assert_eq!(marcado("HDTextures NPCs NovaLUT4 ReShade", &[]), " hd·textures np·cs nova|lut 4 re|shade ");
        assert_eq!(marcado("aBCd", &[]), " a|b·cd ");
        // maiúscula depois de dígito, com ou sem sigla
        assert_eq!(marcado("CP2077Outfits V2Hair 2077NPCs 4KTextures", &[]), " cp 2077:outfits v 2:hair 2077:np·cs 4:k·textures ");
    }

    /// Grafias CamelCase de uma palavra-chave: as palavras dela coladas, cada uma com a inicial maiúscula
    /// ("Reshade", "SpawnRate"), e cada palavra partida em duas em todo ponto ("ReShade", "REshade", "SpawnRAte").
    fn grafias_camelcase(corpo: &str) -> Vec<String> {
        fn inicial_maiuscula(s: &str) -> String {
            let mut c = s.chars();
            c.next().map(|f| f.to_ascii_uppercase().to_string() + c.as_str()).unwrap_or_default()
        }
        let palavras: Vec<&str> = corpo.split(' ').collect();
        let colada = |partida: Option<(usize, usize)>| -> String {
            palavras
                .iter()
                .enumerate()
                .map(|(j, p)| match partida {
                    Some((pj, k)) if pj == j => inicial_maiuscula(&p[..k]) + &inicial_maiuscula(&p[k..]),
                    _ => inicial_maiuscula(p),
                })
                .collect()
        };
        let mut grafias = vec![colada(None)];
        for (j, p) in palavras.iter().enumerate() {
            // uma maiúscula sozinha antes da palavra seguinte vira sigla ("PainTJob"), e palavra não acaba em sigla
            let partidas = (1..p.len()).filter(|&k| j + 1 == palavras.len() || p.len() - k >= 2);
            grafias.extend(partidas.map(|k| colada(Some((j, k)))));
        }
        grafias
    }

    #[test]
    fn grafia_camelcase_alcanca_a_palavra_chave() {
        let todas = CATEGORIES.iter().flat_map(|c| c.keywords.iter()).chain(NPC_NAMES.iter());
        let mut grafias_testadas = 0;
        for chave in todas {
            for grafia in grafias_camelcase(chave.trim_start_matches('*').trim()) {
                // colada depois de palavra e depois de número ("CP2077…", "V2…"). A que começa com dígito (`4k`)
                // só depois de palavra: colada em número, ela seria o mesmo número
                let prefixos: &[&str] =
                    if grafia.starts_with(|c: char| c.is_ascii_digit()) { &["Better"] } else { &["Better", "CP2077", "2077", "V2"] };
                let mut nomes = vec![grafia.clone()];
                for prefixo in prefixos {
                    nomes.push(format!("{prefixo}{grafia} Pack"));
                    // colada antes de outra palavra CamelCase: só quando acaba em minúscula, senão vira sigla
                    if grafia.ends_with(|c: char| c.is_ascii_lowercase()) {
                        nomes.push(format!("{prefixo}{grafia}Pack"));
                    }
                }
                for nome in nomes {
                    assert!(!casamentos(&feno(&nome, &[]), chave).is_empty(), "{chave:?} não casa {nome:?}");
                    grafias_testadas += 1;
                }
            }
        }
        assert!(grafias_testadas > 1000, "{grafias_testadas}");
        let reshade = grafias_camelcase("reshade");
        assert!(reshade.contains(&"ReShade".to_string()) && reshade.contains(&"Reshade".to_string()));
        assert!(grafias_camelcase("spawn rate").contains(&"SpawnRate".to_string()));
    }

    #[test]
    fn camelcase_de_palavra_composta_casa_a_palavra_chave() {
        for (nome, categoria, motivo) in [
            ("ReShade Preset", "lut", "casou: reshade"),
            ("MiniMap Tweaks", "interface", "casou: minimap"),
            ("SkyBox Replacer", "clima", "casou: skybox"),
            ("TimeCycle Tweaks", "clima", "casou: timecycle"),
            ("ToneMap Fix", "lut", "casou: tonemap"),
            ("inkAtlas Fix", "interface", "casou: inkatlas"),
            ("MakeUp Pack", "estetica", "casou: makeup"),
            ("SoundTrack Replacer", "audio", "casou: soundtrack"),
            ("GamePlay Tweaks", "gameplay", "casou: gameplay"),
            ("CyberWare Pack", "gameplay", "casou: cyberware"),
            ("BodySuit Collection", "roupas", "casou: bodysuit"),
            ("JumpSuit", "roupas", "casou: jumpsuit"),
            ("UpScaled Pack", "visual", "casou: upscaled"),
            ("CopperHead", "armas", "casou: copperhead"),
            // o espaço da palavra-chave casa a CamelCase, e as duas grafias da mesma palavra contam uma vez
            ("SpawnRate Tweaks", "gameplay", "casou: spawnrate"),
            ("GodMode", "cheats", "casou: godmode"),
            ("UnlockAll Cheat", "cheats", "casou: unlockall, cheat"),
            ("ColorGrade", "lut", "casou: colorgrade"),
            // palavra inteira começa e acaba na CamelCase
            ("CarMod", "veiculos", "casou: car"),
            ("SimpleUI", "interface", "casou: ui"),
        ] {
            let (cat, _, motivo_agora) = sugestao(nome, &[]);
            assert_eq!((cat.as_str(), motivo_agora.as_str()), (categoria, motivo), "{nome}");
        }
        assert_eq!(sugestao("Cinematic ReShade", &[]), ("lut".into(), 2, "casou: cinematic, reshade".into()));
        assert_eq!(sugestao("Retro", &["archive/pc/mod/RetroReShadePreset.archive"]).0, "lut");
        assert!(casou("Cartoon", " car ").is_empty());
    }

    #[test]
    fn sigla_colada_abre_palavra_sem_quebrar_sigla_com_sufixo() {
        assert_eq!(sugestao("HDTextures", &[]), ("visual".into(), 1, "casou: textures".into()));
        assert_eq!(sugestao("4KTextures", &[]), ("visual".into(), 2, "casou: 4k, textures".into()));
        assert_eq!(sugestao("NEONJacket", &[]).0, "roupas");
        assert_eq!(sugestao("AMMProps", &[]).0, "mundo");
        assert_eq!(sugestao("HUDTweaks", &[]), ("interface".into(), 1, "casou: hud".into()));
        // sufixo minúsculo da sigla: a palavra-chave pode começar ou atravessar a fronteira de sigla
        assert_eq!(sugestao("HUDless Photomode", &[]), ("interface".into(), 1, "casou: hudless".into()));
        assert_eq!(sugestao("NPCless streets", &[]).0, "npc");
        assert_eq!(sugestao("Better NPCs", &[]).0, "npc");
        // palavra inteira não acaba na fronteira de sigla: "HDR" + sufixo não vira ` hd `
        assert!(casou("HDRless Photomode", " hd ").is_empty());
        assert_eq!(sugestao("HDRless Photomode", &[]).0, FALLBACK);
    }

    #[test]
    fn maiuscula_depois_de_digito_abre_palavra() {
        for (nome, categoria, motivo) in [
            ("CP2077Outfits", "roupas", "casou: outfits"),
            ("V2Hair", "cabelos", "casou: hair"),
            ("2077Hairstyles", "cabelos", "casou: hairstyles"),
            ("Cyberpunk2077Weapons", "armas", "casou: weapons"),
            ("Mod2077Skin", "estetica", "casou: skin"),
            ("2077Textures", "visual", "casou: textures"),
            ("V2Outfit Pack", "roupas", "casou: outfit"),
            // sigla depois de número: a mesma palavra que "HDTextures", "Better NPCs" e "HUD Tweaks" dão
            ("2077HDTextures", "visual", "casou: textures"),
            ("2077NPCs", "npc", "casou: npcs"),
            ("CP2077HUD Tweaks", "interface", "casou: hud"),
        ] {
            let (cat, _, motivo_agora) = sugestao(nome, &[]);
            assert_eq!((cat.as_str(), motivo_agora.as_str()), (categoria, motivo), "{nome}");
        }
        // `4k`/`8k` continuam um token, com a maiúscula de unidade ou sem ela
        assert_eq!(sugestao("Realistic Map", &["archive/pc/mod/RealisticMap4K.archive"]), ("visual".into(), 1, "casou: 4k".into()));
        assert_eq!(sugestao("Sky8K", &[]), ("visual".into(), 1, "casou: 8k".into()));
        // a maiúscula pode ser unidade do número: palavra inteira não acaba antes dela, e o espaço de uma
        // palavra-chave não casa ali
        assert!(casou("Sky4K", " 4 ").is_empty());
        assert!(casou("Sky4K", "4 k").is_empty());
    }

    #[test]
    fn sigla_colada_em_sigla_toda_maiuscula_casa_hud_e_npc() {
        for (nome, categoria, motivo) in [
            ("LHUD", "interface", "casou: lhud"),
            ("XHUD", "interface", "casou: xhud"),
            ("LHUD Addon", "interface", "casou: lhud"),
            ("AMMNPCs", "npc", "casou: ammnpcs"),
        ] {
            let (cat, _, motivo_agora) = sugestao(nome, &[]);
            assert_eq!((cat.as_str(), motivo_agora.as_str()), (categoria, motivo), "{nome}");
        }
        // linha do censo: a pasta LHUD é a única pista de interface
        let arquivos = [
            "r6/scripts/Z_BetterEnemyHealthBar.reds",
            "r6/scripts/Z_ShowPlayerHPScanner.reds",
            "r6/scripts/LHUD/modules/worldMarkersEnemy.reds",
        ];
        assert_eq!(sugestao("Disappearing Enemy Health Bar Fix", &arquivos), ("interface".into(), 1, "casou: lhud".into()));
    }

    #[test]
    fn casa_pelo_comeco_do_token_e_nao_por_trecho() {
        assert!(casou("High Resolution Pack", "lut").is_empty());
        assert!(casou("Simple Trainer", "rain").is_empty());
        assert_eq!(casou("Simple Trainer", "trainer"), ["trainer"]);
        assert_eq!(casou("Black Leather Trenchcoat", "trench"), ["trenchcoat"]);
        // ocorrência que sobrepõe a anterior: o `eye` de "rey|eye" que não começa token não esconde o que começa
        assert_eq!(casou("ReyEye", "eye"), ["eye"]);
        assert!(casou("Browser Extension", " brow ").is_empty());
        assert!(casou("SimpleGUI Save Editor", " ui ").is_empty());
        assert!(casou("repaint jobs", "paint job").is_empty());
        assert_eq!(casou("Caliburn Paint Jobs", "paint job"), ["paint jobs"]);
        assert!(casou("Unlock Allies", " unlock all ").is_empty());
        assert_eq!(casou("Unlock All Cyberware", " unlock all "), ["unlock all"]);
        for (nome, categoria) in [
            ("High Resolution Pack", FALLBACK),
            ("Solution", FALLBACK),
            ("Train Station Remaster", "mundo"),
            ("Brain Dance", FALLBACK),
            ("Address Book", FALLBACK),
            ("Harvest Festival", FALLBACK),
            ("Browser Extension", FALLBACK),
            ("SimpleGUI Save Editor", FALLBACK),
            ("Simple Trainer", "cheats"),
            ("Unlock Allies", FALLBACK),
            ("BetterVehicleHandling", "veiculos"),
            ("Retro Look (Jacket)", "roupas"),
            ("Autoloot", "gameplay"),
            ("Beautiful Brows", "estetica"),
        ] {
            assert_eq!(suggest(&rep(nome, &[])).category, categoria, "{nome}");
        }
        // o caminho do arquivo segue a mesma regra
        assert_eq!(sugestao("Address Book", &["archive/pc/mod/address_book.archive"]).0, FALLBACK);
        assert_eq!(sugestao("Vest Pack", &["archive/pc/mod/harvest_vest.archive"]), ("roupas".into(), 1, "casou: vest".into()));
    }

    #[test]
    fn estrela_casa_no_meio_do_token() {
        let f = feno("Mod", &["archive/pc/mod/femvjacket.archive"]);
        assert!(casamentos(&f, "jacket").is_empty());
        assert_eq!(casamentos(&f, "*jacket").len(), 1);
        for nome in ["Longcoat", "Labcoat", "Raincoat", "Sweatpants", "Cargopants", "Tshirt Pack"] {
            assert_eq!(suggest(&rep(nome, &[])).category, "roupas", "{nome}");
        }
        assert_eq!(sugestao("Mod", &["archive/pc/mod/femvjacket.archive"]), ("roupas".into(), 1, "casou: femvjacket".into()));
        assert_eq!(sugestao("Mod", &["archive/pc/mod/RetroJacket.archive"]).0, "roupas");
        // `*gun `: o token acaba em "gun"
        assert_eq!(sugestao("Handgun Pack", &[]), ("armas".into(), 1, "casou: handgun".into()));
        assert_eq!(sugestao("Railgun Mod", &[]).0, "armas");
        assert!(casou("Burgundy Hair", "*gun ").is_empty());
        assert_eq!(sugestao("Burgundy Hair", &[]).0, "cabelos");
    }

    #[test]
    fn conta_palavras_casadas_sem_porcentagem() {
        assert_eq!(sugestao("Posters Remastered", &[]), ("mundo".into(), 2, "casou: posters, remastered".into()));
        assert_eq!(sugestao("8k-4k skin texture and material for female and male", &[]).1, 4);
        assert_eq!(sugestao("zzz mystery blob", &[]), (FALLBACK.into(), 0, "nenhuma palavra-chave casou".into()));
    }

    #[test]
    fn uma_palavra_conta_uma_vez() {
        // palavras-chave sobrepostas no mesmo token
        assert_eq!(sugestao("Beautiful EYEBROWS - CCXL - BOTH V", &[]), ("estetica".into(), 1, "casou: eyebrows".into()));
        assert_eq!(sugestao("Hairstyles Pack", &[]).1, 1);
        assert_eq!(sugestao("Motorcycle Pack", &[]).1, 1);
        assert_eq!(sugestao("Signage", &[]).1, 1);
        assert_eq!(sugestao("River Ward Romance", &[]), ("npc".into(), 1, "personagem: river ward".into()));
        // a mesma palavra no nome e no arquivo, igual ou flexionada
        assert_eq!(sugestao("Black Jacket", &["archive/pc/mod/black_jacket.archive"]).1, 1);
        let judy = ("npc".to_string(), 1, "personagem: judy".to_string());
        assert_eq!(sugestao("Judy Outfit", &["archive/pc/mod/JudyOutfit.archive"]), judy);
        assert_eq!(sugestao("Judy Outfit", &["archive/pc/mod/JudysOutfit.archive"]), judy);
        assert_eq!(sugestao("Hair", &["archive/pc/mod/hairstyle.archive"]).1, 1);
        assert_eq!(
            sugestao("Nova LUT 4.0 (AgX - New HDR)", &["archive/pc/mod/###-NovaLUT4.archive"]),
            ("lut".into(), 2, "casou: nova, lut".into())
        );
        // palavras diferentes contam separado
        assert_eq!(sugestao("Panam Judy Romance", &[]), ("npc".into(), 2, "personagem: panam, judy".into()));
        // `gama` sobrepõe o trecho longo, não o `beta` contido nele: continua a mesma palavra
        assert_eq!(palavras_casadas(&feno("Alfa Beta Gama", &[]), &["alfa beta gama", "beta", "gama"]), ["alfa beta gama"]);
    }

    #[test]
    fn nome_de_npc_conta_junto_das_palavras_chave_de_npc() {
        assert_eq!(sugestao("Judy NPC Outfit", &[]), ("npc".into(), 2, "personagem: judy; casou: npc".into()));
        assert_eq!(sugestao("Panam Companion Mod", &[]), ("npc".into(), 2, "personagem: panam; casou: companion".into()));
        // sem nome de NPC, as mesmas palavras-chave dão a mesma contagem
        assert_eq!(sugestao("NPC Companion Overhaul", &[]), ("npc".into(), 2, "casou: npc, companion".into()));
        // nome e palavra-chave no mesmo token são uma palavra
        assert_eq!(sugestao("judynpc", &[]), ("npc".into(), 1, "personagem: judynpc".into()));
    }

    #[test]
    fn palavras_chave_estao_na_forma_do_feno() {
        let todas = CATEGORIES.iter().flat_map(|c| c.keywords.iter()).chain(NPC_NAMES.iter());
        for chave in todas {
            let corpo = chave.trim_start_matches('*').trim();
            // a palavra-chave é o próprio feno dela: minúscula, um espaço entre palavras, sem letra colada em
            // dígito; a grafia CamelCase dela é `grafia_camelcase_alcanca_a_palavra_chave`
            assert_eq!(marcado(corpo, &[]), format!(" {corpo} "), "{chave:?}");
            // espaço no começo só junto do espaço no fim (palavra inteira)
            assert!(!chave.starts_with(' ') || chave.ends_with(' '), "{chave:?}");
        }
    }

    /// A coluna de categoria do fixture é a saída de `suggest` para nome + arquivos. Quando uma palavra-chave
    /// ou a regra muda, a falha lista cada linha que mudou com a categoria de agora, para conferir e copiar.
    #[test]
    fn censo_de_112_mods_da_a_categoria_do_fixture() {
        // O censo fica fora do crate publicado (`bwms-dev/dados/`): sem ele, o teste é pulado.
        let caminho = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../bwms-dev/dados/censo-de-mods.tsv");
        let Ok(censo) = std::fs::read_to_string(&caminho) else {
            eprintln!("sem {}: censo de mods pulado", caminho.display());
            return;
        };
        let mut linhas = 0;
        let mut divergentes = Vec::new();
        for linha in censo.lines().filter(|l| !l.starts_with('#')) {
            let mut campos = linha.split('\t');
            let (nome, no_fixture) = (campos.next().unwrap(), campos.next().unwrap());
            let arquivos: Vec<&str> = campos.collect();
            linhas += 1;
            let agora = suggest(&rep(nome, &arquivos)).category;
            if agora != no_fixture {
                divergentes.push(format!("{nome}: fixture {no_fixture}, agora {agora}"));
            }
        }
        assert_eq!(linhas, 112);
        assert!(divergentes.is_empty(), "{divergentes:#?}");
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
    fn back_compat_le_chave_theme_antiga() {
        // estado salvo na versão antiga usava "theme": deve continuar carregando
        let old = r#"[{"name":"X","theme":"veiculos","active":true,"favorite":false,"order":0,"variant":""}]"#;
        let back = from_json(old).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].category, "veiculos");
    }

    fn estado(name: &str, category: &str, active: bool, favorite: bool) -> ModState {
        ModState { name: name.into(), category: category.into(), fonte: Fonte::Staging, active, favorite, order: 0, variant: String::new() }
    }

    fn jogo_temporario(nome: &str) -> PathBuf {
        let g = std::env::temp_dir().join(format!("bwms-theme-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        g
    }

    /// O formato que a biblioteca gravou por último (envelope, `fonte`, nome com aspas e `}` vindos do nome da pasta), lido inteiro.
    #[test]
    fn le_o_formato_gravado_pela_biblioteca() {
        let gravado = "{\n  \"schema\":2,\n  \"mods\":[\n    {\"name\":\"Caliburn \\\"Red\\\" {v2}\",\"category\":\"cat}x\",\"fonte\":\"staging\",\"active\":true,\"favorite\":false,\"order\":3,\"variant\":\"azul\"},\n    {\"name\":\"Autor.Mod\",\"category\":\"\",\"fonte\":\"biblioteca\",\"active\":false,\"favorite\":false,\"order\":4,\"variant\":\"\"}\n  ]\n}\n";
        assert_eq!(
            from_json(gravado).unwrap(),
            vec![
                ModState { order: 3, variant: "azul".into(), ..estado("Caliburn \"Red\" {v2}", "cat}x", true, false) },
                ModState { fonte: Fonte::Biblioteca, order: 4, ..estado("Autor.Mod", "", false, false) },
            ]
        );
        assert!(from_json("{\n  \"schema\":2,\n  \"mods\":[\n  ]\n}\n").unwrap().is_empty());
        let v1 = r#"{"schema":1,"mods":[{"name":"A","category":"roupas","active":true,"favorite":false,"order":0,"variant":""}]}"#;
        assert_eq!(from_json(v1).unwrap(), vec![estado("A", "roupas", true, false)], "arquivo da versão 1: todo mod é do staging");
        for ruim in [r#"[{"name":"A","fonte":"nexus"}]"#, r#"[{"name":"A","fonte":true}]"#] {
            let e = from_json(ruim).unwrap_err();
            assert!(e.contains("\"fonte\""), "{e}");
        }
    }

    #[test]
    fn aceita_espaco_em_volta_de_dois_pontos_e_virgula() {
        let s = r#"[ { "name" : "A" , "category" : "roupas" , "active" : false , "favorite" : true , "order" : 2 , "variant" : "" } ]"#;
        assert_eq!(from_json(s).unwrap(), vec![ModState { order: 2, ..estado("A", "roupas", false, true) }]);
    }

    #[test]
    fn decodifica_escapes_json() {
        let s = r#"[{"name":"Tab\tX \u00e9","category":"roupas","active":false,"favorite":false,"order":0,"variant":""}]"#;
        assert_eq!(from_json(s).unwrap()[0].name, "Tab\tX \u{e9}");
    }

    #[test]
    fn le_lista_sem_versao() {
        // golden: texto exato dos arquivos sem envelope (lista nua, 2 espaços, sem '\n' no fim)
        let sem_versao = "[\n  {\"name\":\"Caliburn \\\"Red\\\"\",\"category\":\"veiculos\",\"active\":true,\"favorite\":true,\"order\":0,\"variant\":\"vermelho\"},\n  {\"name\":\"Mod Estranho\",\"category\":\"minha-cat\",\"active\":false,\"favorite\":false,\"order\":3,\"variant\":\"\"}\n]";
        assert_eq!(
            from_json(sem_versao).unwrap(),
            vec![
                ModState { variant: "vermelho".into(), ..estado("Caliburn \"Red\"", "veiculos", true, true) },
                ModState { order: 3, ..estado("Mod Estranho", "minha-cat", false, false) },
            ]
        );
        assert!(from_json("[]").unwrap().is_empty());
    }

    #[test]
    fn schema_mais_novo_ou_envelope_quebrado_e_recusado() {
        let e = from_json(r#"{"schema":3,"mods":[]}"#).unwrap_err();
        assert!(e.contains("schema 3") && e.contains("atualize o BWMS"), "{e}");
        for ruim in [
            r#"{"mods":[]}"#,
            r#"{"schema":0,"mods":[]}"#,
            r#"{"schema":"1","mods":[]}"#,
            r#"{"schema":1.5,"mods":[]}"#,
            r#"{"schema":1}"#,
            r#"{"schema":1,"mods":{}}"#,
            "1",
            "\"x\"",
        ] {
            assert!(from_json(ruim).is_err(), "devia recusar {ruim}");
        }
    }

    #[test]
    fn truncado_ou_virgula_final_e_erro_e_nao_estado_parcial() {
        assert!(from_json("").is_err());
        assert!(from_json("[\n  {\"name\":\"A\",\"categ").is_err());
        assert!(from_json("[\n  {\"name\":\"A\",\"category\":\"roupas\",\"act").is_err());
        assert!(from_json(r#"[{"name":"A","category":"roupas","active":true,"favorite":false,"order":0,"variant":""},]"#).is_err());
    }

    #[test]
    fn tipo_errado_e_erro_e_chave_ausente_e_padrao() {
        for ruim in [
            r#"[{"name":"A","active":"true"}]"#,
            r#"[{"name":"A","favorite":1}]"#,
            r#"[{"name":"A","order":1.5}]"#,
            r#"[{"name":"A","order":3000000000}]"#,
            r#"[{"name":"A","order":"1"}]"#,
            r#"[{"name":"A","category":null}]"#,
            r#"[{"name":"A","theme":7}]"#,
            r#"[{"name":"A","variant":false}]"#,
            r#"[{"category":"roupas"}]"#,
            r#"["A"]"#,
        ] {
            assert!(from_json(ruim).is_err(), "devia recusar {ruim}");
        }
        assert_eq!(from_json(r#"[{"name":"A"}]"#).unwrap(), vec![estado("A", FALLBACK, false, false)]);
        assert_eq!(from_json(r#"[{"name":"A","order":-4,"extra":[1]}]"#).unwrap()[0].order, -4);
    }

    #[test]
    fn load_states_distingue_os_quatro_casos() {
        let g = jogo_temporario("carga");
        assert!(matches!(load_states(&g), StateLoad::Missing));

        std::fs::create_dir_all(g.join(".cp77-mods")).unwrap();
        std::fs::write(state_path(&g), r#"[{"name":"A","category":"roupas","active":false,"favorite":true}]"#).unwrap();
        assert!(matches!(load_states(&g), StateLoad::Loaded(s) if s == vec![estado("A", "roupas", false, true)]));

        std::fs::write(state_path(&g), [0xff, 0xfe]).unwrap();
        assert!(matches!(load_states(&g), StateLoad::Invalid(d) if d.contains("UTF-8")));
        std::fs::write(state_path(&g), "[{\"name\":\"A\",\"cat").unwrap();
        assert!(matches!(load_states(&g), StateLoad::Invalid(_)));

        // diretório no lugar do arquivo: erro de E/S que não é NotFound
        std::fs::remove_file(state_path(&g)).unwrap();
        std::fs::create_dir_all(state_path(&g)).unwrap();
        assert!(matches!(load_states(&g), StateLoad::IoError(_)));

        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn gravar_atomico_nao_deixa_temporario_e_a_falha_nao_toca_no_arquivo_existente() {
        let g = jogo_temporario("atomico");
        let dest = g.join("red4ext/bwms-reslink.txt");
        gravar_atomico(&dest, b"um").unwrap();
        gravar_atomico(&dest, b"dois").unwrap();
        let sobra: Vec<_> = std::fs::read_dir(g.join("red4ext")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n != "bwms-reslink.txt").collect();
        assert!(sobra.is_empty(), "{sobra:?}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"dois");
        // diretório ocupando o nome do temporário: a escrita falha antes do rename
        std::fs::create_dir_all(dest.with_file_name(format!("bwms-reslink.txt.{}.tmp", std::process::id()))).unwrap();
        assert!(gravar_atomico(&dest, b"tres").is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), b"dois");
        let _ = std::fs::remove_dir_all(&g);
    }
}
