//! A parte dos `.xl` da `Mods/` que o próprio jogo aplica. A 2.5 traz o ArchiveXL compilado no binário (`App::LocalizationExtension`,
//! `App::FactoryIndexExtension`, ...; `relatorios/N.md` §1) com as instruções em JSON: `0x101cbd304` varre `archive/mac/mod` e as pastas do
//! mod.io com glob `*.json` recursivo, sem filtro de nome, e cada arquivo é uma config (`0x101cbdddc`, rapidjson). O BWMS converte o `.xl`
//! (YAML, pelo `xl::parse_xl`) num `.json` em `archive/Mac/mod/BWMS-Mods/xl/` e tira da tabela de runtime a seção que o jogo aplica: uma
//! seção de um `.xl` vai por um caminho só.
//!
//! Só vai ao jogo a seção que ele aplica com a mesma semântica do BWMS (`SECOES_NO_JOGO`). Hoje é o `localization.onscreens`, e com duas
//! correções da diferença medida no binário (`relatorios/N.md` §2):
//! - **Reserva:** o `OnLoadTexts` do jogo (`0x101cdd2e4`) mescla o idioma de reserva da config sem trocar texto e depois o idioma em uso só
//!   se ele for outro (`0x10002d9d8`, `!=`); o BWMS (`localizacao::recursos_do_idioma`) troca texto no idioma em uso também quando ele é a
//!   reserva. O `.json` declara antes de tudo um idioma que o mod não tem (`reserva_livre`) com os arquivos da reserva: ele vira a reserva
//!   da config (`0x101cdb780`: a 1ª chave conhecida com caminho) e o idioma em uso nunca é ela, então o jogo sempre faz a passada que troca.
//! - **Ordem entre mods:** o jogo aplica as configs na ordem em que o glob as acha, e no mesmo texto vence a última. O glob
//!   (`0x101c3c410`) pega os arquivos de uma pasta e só depois desce nas subpastas, sem ordenar a listagem (`0x101c3c020`/`0x101c3c1d8`,
//!   ordem do `readdir`). Com um `.json` e uma subpasta por nível (`destino`), a ordem é a da profundidade: o `.xl` que vence na `Mods/`
//!   (o 1º em ordem de caminho, `apply::xl_da_pasta_mods`) fica no fundo e é aplicado por último.
//!
//! Fica tudo no BWMS quando um `.json` de fora da `Mods/` poderia mexer na mesma ordem (`Motivo`): config de mod do mod.io ou de outra
//! subpasta de `archive/Mac/mod` com `localization` (o jogo a aplicaria depois ou em ordem do `readdir`), e `.xl` da `Mods/` sem idioma
//! livre para a reserva. É tudo ou nada: com parte no
//! jogo e parte no BWMS, o BWMS aplica depois e a ordem entre os dois lados não seria a da `Mods/`.
//!
//! O `factories` vai sempre ao jogo, por `.xl`, sem ordem a manter (`relatorios/L10.md` §2, lote 1): o `Configure` nativo (`0x101cbee88`) junta
//! os `.csv` de todas as configs e o handler (`0x101cbf6d4`) os carrega depois do `vehicles.csv`; em chave repetida vence a 1ª linha que
//! termina de carregar. O BWMS não tem outro caminho para ele: o que o jogo não aplica fica sem efeito, e o mod fica amarelo com o motivo
//! (`factories_fora` e `csvs_fora_do_jogo`). O `Configure` pula calado o `.csv` que o depot não tem (`CheckResource`, `0x101cbf0f0`). O `.json`
//! de um `.xl` leva todas as seções dele que vão ao jogo, e um `.json` só com `factories` fica no 1º nível da corrente: sem `localization`,
//! ele não mexe na ordem dos textos.

use crate::instalacao::localizacao::{e_idioma_conhecido, CODIGOS};
use crate::instalacao::xl::XlFile;
use crate::json::{self, Valor};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A pasta dos `.json`, relativa ao jogo: dentro da `BWMS-Mods` que o `cp77-console` (`mods_do_usuario::pasta_nativa`) espelha.
pub const PASTA_DOS_JSON: &str = "archive/Mac/mod/BWMS-Mods/xl";

/// O nome de cada nível da corrente de pastas (`destino`).
const NIVEL: &str = "xl";

/// Teto de `.json` na corrente: cada um desce um nível (`xl/` tem 3 bytes), e 200 níveis ficam longe do `PATH_MAX` de 1024.
pub const TETO_DE_CONFIGS: usize = 200;

/// As seções que vão ao jogo, com a chave do relatório da `Mods/`.
pub const SECOES_NO_JOGO: [&str; 2] = ["localization.onscreens", "factories"];

/// O `EnabledMods.json` do jogo, relativo à home: a lista das pastas do mod.io que o `0x101cbd304` varre depois de `archive/mac/mod`
/// (`GameModService` `vt+0x178` = `0x101d191fc`, lido em `0x101d33310`).
pub const ENABLED_MODS: &str = "Library/Application Support/CD Projekt Red/Cyberpunk 2077/saves/EnabledMods.json";

/// Uma config que o jogo lê: o `.json` de um `.xl` da `Mods/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDoJogo {
    /// O `.xl` de origem.
    pub xl: PathBuf,
    /// Onde o `.json` fica, relativo ao jogo.
    pub destino: PathBuf,
    /// O `.json` inteiro.
    pub conteudo: String,
    /// O `onscreens` como o BWMS o aplicaria (`onscreens_do_bwms`): idioma → arquivos, na ordem do `.xl`; o 1º é a reserva do mod. Vazio = o
    /// `.json` não leva textos.
    pub onscreens: Vec<(String, Vec<String>)>,
    /// Os `.csv` do `factories` (`factories_do_xl`). Vazio = o `.json` não leva `factories`.
    pub factories: Vec<String>,
}

impl ConfigDoJogo {
    /// O `.json` leva `secao` (uma chave de `SECOES_NO_JOGO`).
    pub fn leva(&self, secao: &str) -> bool {
        match secao {
            "localization.onscreens" => !self.onscreens.is_empty(),
            "factories" => !self.factories.is_empty(),
            _ => false,
        }
    }
}

/// Por que uma seção de um `.xl` da `Mods/` não vai ao jogo: o `localization.onscreens` de todos fica no BWMS (`PlanoDoJogo::motivo`); o
/// `factories` de um fica sem efeito (`PlanoDoJogo::factories_fora`, `csvs_fora_do_jogo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Motivo {
    /// O jogo não tem `archive/Mac/`.
    SemPastaDoJogo,
    /// Mods do mod.io ligados com config de `localization` (nome da pasta de cada um).
    ModIo(Vec<String>),
    /// O `EnabledMods.json` não deu para ler (com o erro): os mods do mod.io que o jogo carrega ficam desconhecidos.
    ModIoIlegivel(String),
    /// `.json` com `localization` (ou ilegível) em outra subpasta de `archive/Mac/mod`, relativo ao jogo.
    OutraPasta(Vec<PathBuf>),
    /// `.xl` da `Mods/` que declara os 19 idiomas: não sobra idioma para a reserva.
    SemIdiomaLivre(PathBuf),
    /// Mais `.xl` com `onscreens` que `TETO_DE_CONFIGS`.
    Teto(usize),
    /// O `.json` não foi gravado (com o erro).
    NaoGravou(String),
    /// Os `.csv` do `factories` que não estão no índice de nenhum `.archive` que o jogo monta pelo próprio carregador antes de ler as configs:
    /// o `Configure` nativo os pula em silêncio.
    CsvForaDoJogo(Vec<String>),
}

impl Motivo {
    /// `(PT-BR, inglês)`.
    pub fn texto(&self) -> (String, String) {
        let lista = |v: &[String]| v.join(", ");
        let caminhos = |v: &[PathBuf]| v.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
        match self {
            Motivo::SemPastaDoJogo => (
                "o jogo não tem a pasta archive/Mac; verifique a instalação do jogo".into(),
                "the game has no archive/Mac folder; verify the game installation".into(),
            ),
            Motivo::ModIo(v) => (
                format!("mod(s) do mod.io também traz(em) textos ({}); com eles, o jogo não manteria a ordem da Mods", lista(v)),
                format!("mod.io mod(s) also have texts ({}); with them the game would not keep the Mods order", lista(v)),
            ),
            Motivo::ModIoIlegivel(e) => (
                format!("a lista de mods do mod.io do jogo não deu para ler ({e}); abra o jogo uma vez pelo menu de mods para ele a regravar"),
                format!("the game's mod.io mod list could not be read ({e}); open the game's mod menu once so it rewrites the list"),
            ),
            Motivo::OutraPasta(v) => (
                format!("há instruções de textos fora da BWMS-Mods ({}); mova esse mod para a Mods", caminhos(v)),
                format!("there are text instructions outside BWMS-Mods ({}); move that mod to the Mods folder", caminhos(v)),
            ),
            Motivo::SemIdiomaLivre(x) => (
                format!("{} declara todos os idiomas do jogo; nada a corrigir no arquivo", x.display()),
                format!("{} declares every game language; nothing to fix in the file", x.display()),
            ),
            Motivo::Teto(n) => (
                format!("{n} .xl com textos, acima do teto de {TETO_DE_CONFIGS}"),
                format!("{n} .xl files with texts, above the limit of {TETO_DE_CONFIGS}"),
            ),
            Motivo::NaoGravou(e) => (
                format!("o .json não foi gravado em archive/Mac/mod ({e}); confira o espaço e a permissão da pasta do jogo"),
                format!("the .json could not be written to archive/Mac/mod ({e}); check the free space and permissions of the game folder"),
            ),
            Motivo::CsvForaDoJogo(v) => (
                format!(
                    "{} não está em nenhum .archive da Mods, de archive/Mac/mod nem do mod.io ligado, e o jogo só registra factory de .csv que ele \
                     carrega; reinstale o mod com o .archive que traz o .csv (se o .csv é do próprio jogo, nada a corrigir)",
                    lista(v)
                ),
                format!(
                    "{} is not in any .archive from Mods, archive/Mac/mod or an enabled mod.io mod, and the game only registers factories from .csv \
                     files it loads; reinstall the mod with the .archive that has the .csv (if the .csv is the game's own, nothing to fix)",
                    lista(v)
                ),
            ),
        }
    }
}

/// O que vai ao jogo e o que fica no BWMS.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanoDoJogo {
    /// As configs na ordem em que o jogo as aplica (a mais rasa primeiro).
    pub configs: Vec<ConfigDoJogo>,
    /// Com `onscreens` na `Mods/` e nenhuma config com ele: por que ficou no BWMS.
    pub motivo: Option<Motivo>,
    /// Cada `.xl` da `Mods/` com `factories` que o plano não leva ao jogo, em ordem de caminho, com o motivo: sem efeito.
    pub factories_fora: Vec<(PathBuf, Motivo)>,
    /// Cada `.xl` da `Mods/` que parseou, em ordem de caminho, com as chaves das seções que têm entrada (o `localization` dividido por grupo,
    /// `localization.onscreens`, `localization.subtitles`, ...): o relatório diz quem aplica cada uma.
    pub secoes: Vec<(PathBuf, Vec<String>)>,
}

impl PlanoDoJogo {
    /// `xl` vai ao jogo com `secao` neste plano.
    pub fn no_jogo(&self, xl: &Path, secao: &str) -> bool {
        SECOES_NO_JOGO.contains(&secao) && self.configs.iter().any(|c| c.xl == xl && c.leva(secao))
    }

    /// As configs do plano só com o `factories`, no mesmo destino: as que ficam no disco quando os textos voltam ao BWMS depois de o plano ter
    /// sido gravado (tabelas que não gravaram). O `factories` não tem tabela, e sem o `.json` ficaria sem efeito.
    pub fn so_factories(&self) -> Vec<ConfigDoJogo> {
        self.configs
            .iter()
            .filter(|c| c.leva("factories"))
            .filter_map(|c| {
                Some(ConfigDoJogo { conteudo: json_da_config(&[], &c.factories)?, onscreens: Vec::new(), ..c.clone() })
            })
            .collect()
    }
}

/// Configs com `localization` que o jogo lê fora da `Mods/` e que mudariam a ordem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Concorrentes {
    /// Pastas do mod.io (o nome) com config de `localization`; `Err` com o `EnabledMods.json` ilegível.
    pub modio: Result<Vec<String>, String>,
    /// `.json` com `localization`, ou que não parseia, nas subpastas de `archive/Mac/mod` fora da `BWMS-Mods`, relativos ao jogo.
    pub outras_pastas: Vec<PathBuf>,
}

/// O `onscreens` de `xl` como o BWMS o aplica: os grupos `onscreens` na ordem do `.xl`, idioma sem as pontas, arquivos sem as pontas e sem
/// vazio, idioma sem arquivo fora (`tabelas_de_runtime::emit_localization_table`). O mesmo idioma repetido junta os arquivos na 1ª posição:
/// na tabela as linhas dos dois valem, e a reserva é a 1ª linha (`localizacao::recursos_do_idioma`).
pub fn onscreens_do_bwms(xl: &XlFile) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for g in xl.localization.iter().filter(|g| g.kind == "onscreens") {
        for (idioma, caminhos) in &g.entries {
            let idioma = idioma.trim();
            let caminhos: Vec<String> = caminhos.iter().map(|c| c.trim()).filter(|c| !c.is_empty()).map(str::to_string).collect();
            if caminhos.is_empty() || !e_idioma_conhecido(idioma) {
                continue;
            }
            match out.iter_mut().find(|(i, _)| i == idioma) {
                Some((_, v)) => v.extend(caminhos),
                None => out.push((idioma.to_string(), caminhos)),
            }
        }
    }
    out
}

/// O 1º idioma do jogo que o mod não declara: a reserva da config no jogo (ver o doc do módulo). `None` com os 19 declarados.
pub fn reserva_livre(onscreens: &[(String, Vec<String>)]) -> Option<&'static str> {
    CODIGOS.into_iter().find(|c| e_idioma_conhecido(c) && !onscreens.iter().any(|(i, _)| i == c))
}

/// Os `.csv` do `factories` de `xl`, na ordem do `.xl`, sem as pontas e sem o caminho nulo (vazio ou só barras: o `ResourcePath` 0, que o
/// `Configure` nativo descartaria).
pub fn factories_do_xl(xl: &XlFile) -> Vec<String> {
    xl.factories.iter().map(|c| c.trim()).filter(|c| crate::resource_path_hash(c) != 0).map(str::to_string).collect()
}

/// O membro `localization` de um `onscreens` (`onscreens_do_bwms`): a reserva livre com os arquivos da reserva do mod e depois os idiomas do
/// mod, na ordem. `None` sem idioma ou sem reserva livre.
fn membro_da_localizacao(onscreens: &[(String, Vec<String>)]) -> Option<String> {
    let reserva_do_mod = onscreens.first()?;
    let livre = reserva_livre(onscreens)?;
    let idioma = |i: &str, v: &[String]| {
        let arquivos: Vec<String> = v.iter().map(|c| format!("                {}", json::texto(c))).collect();
        format!("            {}: [\n{}\n            ]", json::texto(i), arquivos.join(",\n"))
    };
    let idiomas: Vec<String> =
        std::iter::once(idioma(livre, &reserva_do_mod.1)).chain(onscreens.iter().map(|(i, v)| idioma(i, v))).collect();
    Some(format!("    \"localization\": {{\n        \"onscreens\": {{\n{}\n        }}\n    }}", idiomas.join(",\n")))
}

/// O `.json` de uma config: o `localization` de `onscreens` e a lista `factories` (a forma de lista que o parse nativo `0x101cbe9b4` lê), cada
/// um só quando tem entrada. `None` sem nenhum dos dois, ou com `onscreens` sem reserva livre.
pub fn json_da_config(onscreens: &[(String, Vec<String>)], factories: &[String]) -> Option<String> {
    let mut membros = Vec::new();
    if !onscreens.is_empty() {
        membros.push(membro_da_localizacao(onscreens)?);
    }
    if !factories.is_empty() {
        let csvs: Vec<String> = factories.iter().map(|c| format!("        {}", json::texto(c))).collect();
        membros.push(format!("    \"factories\": [\n{}\n    ]", csvs.join(",\n")));
    }
    (!membros.is_empty()).then(|| format!("{{\n{}\n}}\n", membros.join(",\n")))
}

/// O destino da config aplicada na posição `k` (0 = a 1ª): `k` níveis abaixo de `PASTA_DOS_JSON`, com `NNNN-<nome sem .xl>.json`, `NNNN` a
/// posição do `.xl` na ordem de caminho da `Mods/` (1 = o que vence) com o mesmo número de dígitos para todos.
fn destino(k: usize, posicao: usize, total: usize, xl: &Path) -> PathBuf {
    let digitos = total.to_string().len().max(4);
    let nome = xl.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let sem_xl = if nome.to_ascii_lowercase().ends_with(".xl") { &nome[..nome.len() - 3] } else { nome.as_str() };
    let mut p = PathBuf::from(PASTA_DOS_JSON);
    for _ in 0..k {
        p.push(NIVEL);
    }
    p.push(format!("{posicao:0digitos$}-{sem_xl}.json"));
    p
}

/// As seções de `xl` com entrada, com o `localization` dividido por grupo: as chaves do relatório.
fn secoes_do_xl(xl: &XlFile) -> Vec<String> {
    let mut v = Vec::new();
    for s in xl.secoes().iter().filter(|s| s.entradas > 0) {
        if s.chave == "localization" {
            for g in &xl.localization {
                let k = format!("localization.{}", g.kind);
                if !v.contains(&k) {
                    v.push(k);
                }
            }
        } else {
            v.push(s.chave.to_string());
        }
    }
    v
}

/// O plano de `mods` (os `.xl` da `Mods/` que parsearam, em ordem de caminho: o 1º vence), com os `concorrentes` de fora da `Mods/` e
/// `pasta_do_jogo` (o jogo tem `archive/Mac/`).
pub fn planejar(mods: &[(PathBuf, &XlFile)], concorrentes: &Concorrentes, pasta_do_jogo: bool) -> PlanoDoJogo {
    let secoes = mods.iter().map(|(p, xl)| (p.clone(), secoes_do_xl(xl))).collect();
    let com_textos: Vec<(PathBuf, Vec<(String, Vec<String>)>)> =
        mods.iter().map(|(p, xl)| (p.clone(), onscreens_do_bwms(xl))).filter(|(_, o)| !o.is_empty()).collect();
    let motivo = if com_textos.is_empty() {
        None
    } else if !pasta_do_jogo {
        Some(Motivo::SemPastaDoJogo)
    } else {
        match &concorrentes.modio {
            Err(e) => Some(Motivo::ModIoIlegivel(e.clone())),
            Ok(v) if !v.is_empty() => Some(Motivo::ModIo(v.clone())),
            Ok(_) if !concorrentes.outras_pastas.is_empty() => Some(Motivo::OutraPasta(concorrentes.outras_pastas.clone())),
            Ok(_) if com_textos.len() > TETO_DE_CONFIGS => Some(Motivo::Teto(com_textos.len())),
            Ok(_) => com_textos.iter().find(|(_, o)| reserva_livre(o).is_none()).map(|(p, _)| Motivo::SemIdiomaLivre(p.clone())),
        }
    };
    // A corrente dos textos: vazia quando o `onscreens` fica no BWMS.
    let corrente: Vec<(PathBuf, Vec<(String, Vec<String>)>)> = if motivo.is_none() { com_textos } else { Vec::new() };
    let com_factories: Vec<(PathBuf, Vec<String>)> =
        mods.iter().map(|(p, xl)| (p.clone(), factories_do_xl(xl))).filter(|(_, f)| !f.is_empty()).collect();
    let (factories_no_jogo, factories_fora) = if pasta_do_jogo {
        (com_factories, Vec::new())
    } else {
        (Vec::new(), com_factories.into_iter().map(|(p, _)| (p, Motivo::SemPastaDoJogo)).collect())
    };
    let total = mods.len();
    let posicao = |xl: &PathBuf| mods.iter().position(|(p, _)| p == xl).map(|i| i + 1);
    let factories_de = |xl: &PathBuf| factories_no_jogo.iter().find(|(p, _)| p == xl).map(|(_, f)| f.clone()).unwrap_or_default();
    // Só com `factories`: no 1º nível, antes da corrente; sem `localization`, a ordem do `readdir` entre eles e o 1º da corrente não pesa.
    let so_factories = factories_no_jogo.iter().filter(|(xl, _)| !corrente.iter().any(|(c, _)| c == xl)).filter_map(|(xl, f)| {
        Some(ConfigDoJogo { xl: xl.clone(), destino: destino(0, posicao(xl)?, total, xl), conteudo: json_da_config(&[], f)?, onscreens: Vec::new(), factories: f.clone() })
    });
    let da_corrente = corrente.iter().rev().enumerate().filter_map(|(k, (xl, onscreens))| {
        let f = factories_de(xl);
        Some(ConfigDoJogo {
            xl: xl.clone(),
            destino: destino(k, posicao(xl)?, total, xl),
            conteudo: json_da_config(onscreens, &f)?,
            onscreens: onscreens.clone(),
            factories: f,
        })
    });
    let configs = so_factories.chain(da_corrente).collect();
    PlanoDoJogo { configs, motivo, factories_fora, secoes }
}

/// Cada `.xl` de `plano` cujo `factories` tem `.csv` fora de `montados` (o hash de caminho de cada recurso dos `.archive` que o jogo monta pelo
/// próprio carregador antes de ler as configs), com a lista: o `Configure` nativo os pula, e eles ficam sem efeito.
pub fn csvs_fora_do_jogo(plano: &PlanoDoJogo, montados: &HashSet<u64>) -> Vec<(PathBuf, Motivo)> {
    plano
        .configs
        .iter()
        .filter_map(|c| {
            let fora: Vec<String> = c.factories.iter().filter(|f| !montados.contains(&crate::resource_path_hash(f))).cloned().collect();
            (!fora.is_empty()).then(|| (c.xl.clone(), Motivo::CsvForaDoJogo(fora)))
        })
        .collect()
}

/// Todas as configs de `plano` estão no disco de `game` com o conteúdo do plano. O escritor das tabelas só tira do BWMS a seção que o
/// jogo vai ler: sem o `.json` (a gravação que falhou), o BWMS continua aplicando.
pub fn no_disco(game: &Path, plano: &PlanoDoJogo) -> bool {
    !plano.configs.is_empty() && plano.configs.iter().all(|c| std::fs::read(game.join(&c.destino)).is_ok_and(|d| d == c.conteudo.as_bytes()))
}

/// O texto de um `.json` sem o BOM UTF-8, que o jogo pula (`0x101cbe760`).
fn sem_bom(t: &str) -> &str {
    t.strip_prefix('\u{feff}').unwrap_or(t)
}

/// O `.json` em `p` tem `localization` no topo, ou não parseia: o que o leitor estrito daqui recusa o rapidjson do jogo pode aceitar, e conta
/// como podendo ter.
fn tem_localizacao(p: &Path) -> bool {
    match std::fs::read_to_string(p).map_err(|e| e.to_string()).and_then(|t| json::ler(sem_bom(&t))) {
        Ok(v) => v.campo("localization").is_some(),
        Err(_) => true,
    }
}

/// Todo arquivo `*.json` debaixo de `dir` (a extensão sem distinguir caixa), sem seguir link, fora de `pular`.
fn jsons_em(dir: &Path, pular: Option<&Path>, out: &mut Vec<PathBuf>) {
    let Ok(entradas) = std::fs::read_dir(dir) else { return };
    for e in entradas.flatten() {
        let p = e.path();
        let Ok(tipo) = e.file_type() else { continue };
        if tipo.is_dir() {
            if pular != Some(p.as_path()) {
                jsons_em(&p, pular, out);
            }
        } else if tipo.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) {
            out.push(p);
        }
    }
}

/// Um mod do mod.io que o jogo carrega, como o `EnabledMods.json` o lista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModLigadoDoModio {
    pub pasta: PathBuf,
    /// `modID`; o jogo ordena as pastas por ele com comparação assinada (`0x101d195a0`). `None` sem o campo ou fora de `i64`.
    pub id: Option<i64>,
    /// `modName`, o nome que o menu Mods do jogo mostra.
    pub nome: Option<String>,
}

/// Os mods do mod.io do `EnabledMods.json` em `arquivo`: ausente é lista vazia (sem mod do mod.io); ilegível ou fora do formato que o
/// jogo exige (`enabled_mods` lista de objetos com `modPath` texto, `0x101d36c4c`) é `Err`.
pub fn mods_ligados_do_modio(arquivo: &Path) -> Result<Vec<ModLigadoDoModio>, String> {
    let texto = match std::fs::read_to_string(arquivo) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let doc = json::ler(sem_bom(&texto))?;
    let Some(Valor::Lista(mods)) = doc.campo("enabled_mods") else { return Err("sem a lista enabled_mods".into()) };
    mods.iter()
        .map(|m| match m.campo("modPath") {
            Some(Valor::Texto(p)) => Ok(ModLigadoDoModio {
                pasta: PathBuf::from(p),
                id: match m.campo("modID") {
                    Some(Valor::Numero(n)) => n.parse().ok(),
                    _ => None,
                },
                nome: match m.campo("modName") {
                    Some(Valor::Texto(n)) if !n.trim().is_empty() => Some(n.clone()),
                    _ => None,
                },
            }),
            _ => Err("entrada de enabled_mods sem modPath".to_string()),
        })
        .collect()
}

/// As pastas de `mods_ligados_do_modio`.
pub fn pastas_do_modio(arquivo: &Path) -> Result<Vec<PathBuf>, String> {
    mods_ligados_do_modio(arquivo).map(|v| v.into_iter().map(|m| m.pasta).collect())
}

/// Os concorrentes de `game` com a home `home`: as configs com `localization` dos mods do mod.io ligados e das subpastas de
/// `archive/Mac/mod` fora da `BWMS-Mods`. Os `.json` soltos na raiz de `archive/Mac/mod` não contam: o glob os pega antes das subpastas, e
/// os da `BWMS-Mods` vêm depois e vencem, como os da `Mods/` vencem no BWMS.
pub fn concorrentes(game: &Path, home: Option<&Path>) -> Concorrentes {
    let modio = match home {
        None => Ok(Vec::new()),
        Some(h) => pastas_do_modio(&h.join(ENABLED_MODS)).map(|pastas| {
            pastas
                .iter()
                .filter(|d| {
                    let mut v = Vec::new();
                    jsons_em(d, None, &mut v);
                    v.iter().any(|p| tem_localizacao(p))
                })
                .map(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| d.display().to_string()))
                .collect()
        }),
    };
    let raiz = game.join("archive/Mac/mod");
    let bwms = raiz.join("BWMS-Mods");
    let mut outras = Vec::new();
    for e in std::fs::read_dir(&raiz).into_iter().flatten().flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) && e.path() != bwms {
            jsons_em(&e.path(), Some(&bwms), &mut outras);
        }
    }
    let outras_pastas = outras.into_iter().filter(|p| tem_localizacao(p)).map(|p| p.strip_prefix(game).map(Path::to_path_buf).unwrap_or(p)).collect();
    Concorrentes { modio, outras_pastas }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instalacao::xl::parse_xl;

    const EEX: &str = "localization:\n  onscreens:\n    en-us: equipment_ex\\localization\\en-us.json\n    ar-ar: equipment_ex\\localization\\ar-ar.json\n    cz-cz: equipment_ex\\localization\\cz-cz.json\n    de-de: equipment_ex\\localization\\de-de.json\n    es-es: equipment_ex\\localization\\es-es.json\n    fr-fr: equipment_ex\\localization\\fr-fr.json\n    it-it: equipment_ex\\localization\\it-it.json\n    kr-kr: equipment_ex\\localization\\kr-kr.json\n    pl-pl: equipment_ex\\localization\\pl-pl.json\n    pt-br: equipment_ex\\localization\\pt-br.json\n    ru-ru: equipment_ex\\localization\\ru-ru.json\n    tr-tr: equipment_ex\\localization\\tr-tr.json\n    zh-cn: equipment_ex\\localization\\zh-cn.json\n";
    const VA: &str = "localization:\n  onscreens:\n    en-us: base\\localization\\en-us\\onscreens\\virtual-atelier.json\n    cz-cz: base\\localization\\cz-cz\\onscreens\\virtual-atelier.json\n    de-de: base\\localization\\de-de\\onscreens\\virtual-atelier.json\n    it-it: base\\localization\\it-it\\onscreens\\virtual-atelier.json\n    ru-ru: base\\localization\\ru-ru\\onscreens\\virtual-atelier.json\n    tr-tr: base\\localization\\tr-tr\\onscreens\\virtual-atelier.json\n    zh-cn: base\\localization\\zh-cn\\onscreens\\virtual-atelier.json\n    fr-fr: base\\localization\\fr-fr\\onscreens\\virtual-atelier.json\n    zh-tw: base\\localization\\zh-tw\\onscreens\\virtual-atelier.json\n    pt-br: base\\localization\\pt-br\\onscreens\\virtual-atelier.json\n";

    fn sem_concorrentes() -> Concorrentes {
        Concorrentes { modio: Ok(Vec::new()), outras_pastas: Vec::new() }
    }

    /// O `onscreens` do `.json` lido de volta: (idioma, arquivos) na ordem do objeto, como o `0x101cdb780` o percorre.
    fn idiomas_do_json(t: &str) -> Vec<(String, Vec<String>)> {
        let v = json::ler(t).expect("json estrito");
        let Some(Valor::Objeto(pares)) = v.campo("localization").and_then(|l| l.campo("onscreens")) else { panic!("sem onscreens: {t}") };
        pares
            .iter()
            .map(|(k, v)| match v {
                Valor::Lista(l) => (k.clone(), l.iter().map(|x| if let Valor::Texto(s) = x { s.clone() } else { panic!("item não é texto") }).collect()),
                _ => panic!("valor não é lista"),
            })
            .collect()
    }

    /// A reserva da config no jogo: a 1ª chave de idioma conhecido com arquivo (`0x101cdb780` → `0x10002b8e8`/`0x10002aa74`).
    fn reserva_no_jogo(o: &[(String, Vec<String>)]) -> &str {
        o.iter().find(|(i, v)| e_idioma_conhecido(i) && !v.is_empty()).map(|(i, _)| i.as_str()).unwrap()
    }

    /// As passadas do `OnLoadTexts` do jogo (`0x101cdd2e4`) numa config: a reserva sem troca e depois o idioma em uso, se for outro, com
    /// troca. `(arquivo, troca)`.
    fn passadas_do_jogo(o: &[(String, Vec<String>)], idioma: &str) -> Vec<(String, bool)> {
        let busca = |i: &str| o.iter().find(|(k, _)| k == i).map(|(_, v)| v.clone());
        let reserva = reserva_no_jogo(o).to_string();
        let mut out: Vec<(String, bool)> = busca(&reserva).unwrap_or_default().into_iter().map(|c| (c, false)).collect();
        if idioma != reserva {
            out.extend(busca(idioma).unwrap_or_default().into_iter().map(|c| (c, true)));
        }
        out
    }

    /// As passadas do BWMS (`localizacao::recursos_do_idioma`): o idioma em uso com troca, depois a reserva do mod sem troca se for outra.
    fn passadas_do_bwms(o: &[(String, Vec<String>)], idioma: &str) -> Vec<(String, bool)> {
        let reserva = &o[0].0;
        let mut out: Vec<(String, bool)> = o.iter().filter(|(i, _)| i == idioma).flat_map(|(_, v)| v.iter().map(|c| (c.clone(), true))).collect();
        if reserva != idioma {
            out.extend(o.iter().filter(|(i, _)| i == reserva).flat_map(|(_, v)| v.iter().map(|c| (c.clone(), false))));
        }
        out
    }

    /// O texto final de cada chave depois das passadas, com a regra de troca do `MergeTextEntry` (`0x101cdc9f4`) e do `gravar_entrada` do
    /// BWMS: com troca, o texto novo entra; sem troca, só entra chave que ainda não existe. `arquivos` dá as chaves de cada arquivo.
    fn aplicar(passadas: &[(String, bool)], arquivos: &[(&str, &[(u64, &str)])], vanilla: &[(u64, &str)]) -> Vec<(u64, String)> {
        let mut m: std::collections::BTreeMap<u64, String> = vanilla.iter().map(|(k, t)| (*k, t.to_string())).collect();
        for (c, troca) in passadas {
            for (k, t) in arquivos.iter().find(|(a, _)| a == c).map(|(_, e)| *e).unwrap_or(&[]) {
                if *troca || !m.contains_key(k) {
                    m.insert(*k, t.to_string());
                }
            }
        }
        m.into_iter().collect()
    }

    /// O `.json` do EEX e do VA: JSON estrito, a reserva livre na frente com os arquivos do en-us, e os idiomas do `.xl` na ordem.
    #[test]
    fn json_do_eex_e_do_va() {
        for (fonte, livre) in [(EEX, "es-mx"), (VA, "ar-ar")] {
            let xl = parse_xl(fonte).unwrap();
            let o = onscreens_do_bwms(&xl);
            let t = json_da_config(&o, &[]).unwrap();
            let lido = idiomas_do_json(&t);
            assert_eq!(lido[0], (livre.to_string(), o[0].1.clone()), "{t}");
            assert_eq!(&lido[1..], o.as_slice());
            assert_eq!(o[0].0, "en-us");
            assert!(!o.iter().any(|(i, _)| i == livre));
        }
        let t = json_da_config(&onscreens_do_bwms(&parse_xl(VA).unwrap()), &[]).unwrap();
        assert!(t.starts_with("{\n    \"localization\": {\n        \"onscreens\": {\n            \"ar-ar\": [\n                \"base\\\\localization\\\\en-us\\\\onscreens\\\\virtual-atelier.json\"\n            ],\n"), "{t}");
    }

    /// O formato da amostra real do mod.io (`ImmersiveRippers.archive.json`): objeto no topo, seção → valor, caminho com `\\`; o leitor estrito
    /// a lê, e o mesmo leitor lê o `.json` gerado.
    #[test]
    fn amostra_do_modio_e_o_gerado_no_mesmo_formato() {
        let amostra = "{\n    \"streaming\": {\n        \"blocks\": [\n            \"base\\\\worlds\\\\immersive_rippers.streamingblock\"\n        ]\n    },\n    \"journal\": [\n        \"base\\\\journal\\\\ripper_journal.journal\"\n    ]\n}";
        let v = json::ler(amostra).unwrap();
        assert_eq!(v.campo("journal"), Some(&Valor::Lista(vec![Valor::Texto("base\\journal\\ripper_journal.journal".into())])));
        let xl = parse_xl("streaming:\n  blocks:\n    - base\\worlds\\immersive_rippers.streamingblock\njournal:\n  - base\\journal\\ripper_journal.journal\n").unwrap();
        assert_eq!(xl.journals, ["base\\journal\\ripper_journal.journal"]);
        assert!(onscreens_do_bwms(&xl).is_empty());
    }

    /// Para todo idioma do jogo, as passadas do jogo sobre o `.json` dão o mesmo texto final que as do BWMS sobre o `.xl`: chave nova do mod,
    /// chave do jogo que o mod troca e chave só da reserva. Sem a reserva livre (o `.json` com o `.xl` como está), o idioma da reserva diverge.
    #[test]
    fn mesma_semantica_em_todo_idioma() {
        let xl = parse_xl("localization:\n  onscreens:\n    pt-br: m\\pt.json\n    de-de: m\\de.json\n").unwrap();
        let o = onscreens_do_bwms(&xl);
        let no_jogo = idiomas_do_json(&json_da_config(&o, &[]).unwrap());
        let arquivos: [(&str, &[(u64, &str)]); 2] = [("m\\pt.json", &[(1, "pt-nova"), (7, "pt-troca"), (9, "pt-so")]), ("m\\de.json", &[(1, "de-nova"), (7, "de-troca")])];
        let vanilla = [(7u64, "jogo")];
        for idioma in CODIGOS.into_iter().filter(|c| e_idioma_conhecido(c)) {
            let jogo = aplicar(&passadas_do_jogo(&no_jogo, idioma), &arquivos, &vanilla);
            let bwms = aplicar(&passadas_do_bwms(&o, idioma), &arquivos, &vanilla);
            assert_eq!(jogo, bwms, "{idioma}");
        }
        // O `.json` sem a reserva livre: no pt-br o jogo não troca a chave 7.
        let cru = aplicar(&passadas_do_jogo(&o, "pt-br"), &arquivos, &vanilla);
        assert_ne!(cru, aplicar(&passadas_do_bwms(&o, "pt-br"), &arquivos, &vanilla));
    }

    /// Idioma repetido junta os arquivos na 1ª posição; código desconhecido, idioma sem arquivo e caminho vazio saem.
    #[test]
    fn onscreens_como_a_tabela() {
        let xl = parse_xl("localization:\n  onscreens:\n    de-de: [ a.json, ' ' ]\n    xx-xx: b.json\n    pt-br: []\n    de-de: c.json\n    en-us: ' d.json '\n").unwrap();
        assert_eq!(onscreens_do_bwms(&xl), [("de-de".to_string(), vec!["a.json".to_string(), "c.json".to_string()]), ("en-us".to_string(), vec!["d.json".to_string()])]);
    }

    /// Os 19 idiomas declarados: sem reserva livre, o plano deixa tudo no BWMS.
    #[test]
    fn todos_os_idiomas_ficam_no_bwms() {
        let todos: String = CODIGOS.into_iter().filter(|c| e_idioma_conhecido(c)).map(|c| format!("    {c}: m\\{c}.json\n")).collect();
        let xl = parse_xl(&format!("localization:\n  onscreens:\n{todos}")).unwrap();
        assert_eq!(json_da_config(&onscreens_do_bwms(&xl), &[]), None);
        let p = planejar(&[(PathBuf::from("/j/Mods/m/m.xl"), &xl)], &sem_concorrentes(), true);
        assert!(p.configs.is_empty());
        assert_eq!(p.motivo, Some(Motivo::SemIdiomaLivre(PathBuf::from("/j/Mods/m/m.xl"))));
    }

    /// A ordem do jogo é a da profundidade, e o `.xl` que vence na `Mods/` (o 1º em ordem de caminho) fica no fundo; o `.xl` sem textos não
    /// gera `.json` e conta na numeração.
    #[test]
    fn corrente_na_ordem_inversa_da_mods() {
        let (eex, va, outro) = (parse_xl(EEX).unwrap(), parse_xl(VA).unwrap(), parse_xl("journal:\n  - j.journal\n").unwrap());
        let mods = [
            (PathBuf::from("/j/Mods/Equipment-EX/archive/pc/mod/EquipmentEx.archive.xl"), &eex),
            (PathBuf::from("/j/Mods/Outro/o.xl"), &outro),
            (PathBuf::from("/j/Mods/Virtual Atelier/archive/pc/mod/VirtualAtelier.archive.xl"), &va),
        ];
        let p = planejar(&mods, &sem_concorrentes(), true);
        assert_eq!(p.motivo, None);
        let destinos: Vec<String> = p.configs.iter().map(|c| c.destino.display().to_string()).collect();
        assert_eq!(destinos, ["archive/Mac/mod/BWMS-Mods/xl/0003-VirtualAtelier.archive.json", "archive/Mac/mod/BWMS-Mods/xl/xl/0001-EquipmentEx.archive.json"]);
        assert!(p.no_jogo(&mods[0].0, "localization.onscreens"));
        assert!(!p.no_jogo(&mods[0].0, "journal"));
        assert!(!p.no_jogo(&mods[1].0, "localization.onscreens"));
        assert_eq!(p.secoes[1], (mods[1].0.clone(), vec!["journal".to_string()]));
        assert_eq!(p.secoes[0].1, ["localization.onscreens"]);
    }

    /// Cada concorrente deixa tudo no BWMS, com o motivo.
    #[test]
    fn concorrentes_deixam_no_bwms() {
        let eex = parse_xl(EEX).unwrap();
        let mods = [(PathBuf::from("/j/Mods/E/e.xl"), &eex)];
        assert_eq!(planejar(&mods, &sem_concorrentes(), false).motivo, Some(Motivo::SemPastaDoJogo));
        assert_eq!(planejar(&mods, &sem_concorrentes(), true).motivo, None);
        let modio = Concorrentes { modio: Ok(vec!["123".into()]), outras_pastas: Vec::new() };
        assert_eq!(planejar(&mods, &modio, true).motivo, Some(Motivo::ModIo(vec!["123".into()])));
        let ilegivel = Concorrentes { modio: Err("x".into()), outras_pastas: Vec::new() };
        assert_eq!(planejar(&mods, &ilegivel, true).motivo, Some(Motivo::ModIoIlegivel("x".into())));
        let outra = Concorrentes { modio: Ok(Vec::new()), outras_pastas: vec![PathBuf::from("archive/Mac/mod/x/a.json")] };
        assert!(matches!(planejar(&mods, &outra, true).motivo, Some(Motivo::OutraPasta(_))));
        // Sem textos na `Mods/`, nada a decidir nos textos; o `factories` sem a pasta do jogo fica sem efeito.
        let sem = parse_xl("factories: f.csv\n").unwrap();
        assert_eq!(planejar(&[(PathBuf::from("/j/Mods/S/s.xl"), &sem)], &sem_concorrentes(), false), PlanoDoJogo { factories_fora: vec![(PathBuf::from("/j/Mods/S/s.xl"), Motivo::SemPastaDoJogo)], secoes: vec![(PathBuf::from("/j/Mods/S/s.xl"), vec!["factories".into()])], ..PlanoDoJogo::default() });
    }

    /// O scan de concorrentes: o `EnabledMods.json` ausente é vazio; com um mod do mod.io que tem config de `localization` (com BOM), ele
    /// aparece; o que não tem fica de fora; `.json` com `localization` numa subpasta de `archive/Mac/mod` conta, solto na raiz ou na `BWMS-Mods`
    /// não.
    #[test]
    fn concorrentes_no_disco() {
        let base = std::env::temp_dir().join(format!("bwms-xl-no-jogo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (game, home) = (base.join("jogo"), base.join("home"));
        let escrever = |p: &Path, t: &str| {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        };
        assert_eq!(concorrentes(&game, Some(&home)), sem_concorrentes());
        let (com, sem) = (base.join("modio/111"), base.join("modio/222"));
        escrever(&com.join("archive/x.archive.json"), "\u{feff}{\"localization\": {\"onscreens\": {\"en-us\": \"a.json\"}}}");
        escrever(&sem.join("archive/y.archive.json"), "{\"journal\": [\"j.journal\"]}");
        escrever(&sem.join("about.json"), "{\"ModInfo\": {\"ModName\": \"y\"}}");
        let lista = format!(
            "{{\"version\": 6, \"enabled_mods\": [{{\"modPath\": {}, \"modID\": 111}}, {{\"modPath\": {}, \"modID\": 222}}]}}",
            json::texto(&com.display().to_string()),
            json::texto(&sem.display().to_string())
        );
        escrever(&home.join(ENABLED_MODS), &lista);
        escrever(&game.join("archive/Mac/mod/solto.json"), "{\"localization\": {}}");
        escrever(&game.join("archive/Mac/mod/BWMS-Mods/xl/0001-a.json"), "{\"localization\": {}}");
        escrever(&game.join("archive/Mac/mod/outro/sub/b.JSON"), "{\"localization\": {}}");
        escrever(&game.join("archive/Mac/mod/outro/c.json"), "{\"factories\": []}");
        let c = concorrentes(&game, Some(&home));
        assert_eq!(c.modio, Ok(vec!["111".to_string()]));
        assert_eq!(c.outras_pastas, [PathBuf::from("archive/Mac/mod/outro/sub/b.JSON")]);
        let ligados = mods_ligados_do_modio(&home.join(ENABLED_MODS)).unwrap();
        assert_eq!((ligados[0].id, ligados[1].id, ligados[0].nome.clone()), (Some(111), Some(222), None));
        escrever(&home.join(ENABLED_MODS), "{\"version\": 6}");
        assert!(concorrentes(&game, Some(&home)).modio.is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// `no_disco` só com todas as configs no disco e com o conteúdo do plano.
    #[test]
    fn no_disco_exige_todas_iguais() {
        let game = std::env::temp_dir().join(format!("bwms-xl-no-disco-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&game);
        let (eex, va) = (parse_xl(EEX).unwrap(), parse_xl(VA).unwrap());
        let p = planejar(&[(PathBuf::from("/j/Mods/E/e.xl"), &eex), (PathBuf::from("/j/Mods/V/v.xl"), &va)], &sem_concorrentes(), true);
        assert!(!no_disco(&game, &p));
        for c in &p.configs[..1] {
            std::fs::create_dir_all(game.join(&c.destino).parent().unwrap()).unwrap();
            std::fs::write(game.join(&c.destino), &c.conteudo).unwrap();
        }
        assert!(!no_disco(&game, &p));
        let c = &p.configs[1];
        std::fs::create_dir_all(game.join(&c.destino).parent().unwrap()).unwrap();
        std::fs::write(game.join(&c.destino), "{}").unwrap();
        assert!(!no_disco(&game, &p));
        std::fs::write(game.join(&c.destino), &c.conteudo).unwrap();
        assert!(no_disco(&game, &p));
        assert!(!no_disco(&game, &PlanoDoJogo::default()));
        let _ = std::fs::remove_dir_all(&game);
    }

    /// Os `.csv` de um `.json` lidos de volta como o parse nativo (`0x101cbe9b4`) os lê: lista de textos sob `factories`.
    fn factories_do_json(t: &str) -> Vec<String> {
        match json::ler(t).expect("json estrito").campo("factories") {
            Some(Valor::Lista(l)) => l.iter().map(|x| if let Valor::Texto(s) = x { s.clone() } else { panic!("item não é texto") }).collect(),
            _ => Vec::new(),
        }
    }

    /// Pontas tiradas, caminho nulo fora, ordem do `.xl`, escalar ou lista.
    #[test]
    fn factories_como_o_jogo_le() {
        let xl = parse_xl("factories:\n  - ' m\\a.csv '\n  - '\\'\n  - m\\b.csv\n").unwrap();
        assert_eq!(factories_do_xl(&xl), ["m\\a.csv", "m\\b.csv"]);
        assert_eq!(factories_do_xl(&parse_xl("factories: m\\c.csv\n").unwrap()), ["m\\c.csv"]);
        assert!(factories_do_xl(&parse_xl("journal:\n  - j.journal\n").unwrap()).is_empty());
    }

    /// Textos e `factories` do mesmo `.xl` num `.json` só, com o `localization` como antes e a lista que o jogo lê; o `no_jogo` responde por
    /// seção.
    #[test]
    fn textos_e_factories_no_mesmo_json() {
        let xl = parse_xl(&format!("{VA}factories:\n  - va\\f.csv\n")).unwrap();
        let p = PathBuf::from("/j/Mods/V/v.xl");
        let plano = planejar(&[(p.clone(), &xl)], &sem_concorrentes(), true);
        assert_eq!(plano.configs.len(), 1);
        let c = &plano.configs[0];
        assert_eq!(factories_do_json(&c.conteudo), ["va\\f.csv"]);
        assert_eq!(idiomas_do_json(&c.conteudo)[1..], onscreens_do_bwms(&xl)[..]);
        assert!(c.conteudo.starts_with("{\n    \"localization\": {\n        \"onscreens\": {\n"), "{}", c.conteudo);
        assert!(c.conteudo.ends_with("        }\n    },\n    \"factories\": [\n        \"va\\\\f.csv\"\n    ]\n}\n"), "{}", c.conteudo);
        assert!(plano.no_jogo(&p, "factories") && plano.no_jogo(&p, "localization.onscreens"));
        assert!(plano.factories_fora.is_empty());
        // Os textos de volta ao BWMS: o mesmo destino, só com o `factories`.
        let so = plano.so_factories();
        assert_eq!((so.len(), &so[0].destino), (1, &c.destino));
        assert_eq!(so[0].conteudo, "{\n    \"factories\": [\n        \"va\\\\f.csv\"\n    ]\n}\n");
        assert!(!so[0].leva("localization.onscreens") && so[0].leva("factories"));
    }

    /// Cada `.xl` com `factories` vai ao jogo, no 1º nível quando não tem textos; a corrente dos textos não muda. `csvs_fora_do_jogo` aponta,
    /// por `.xl`, os `.csv` fora dos archives que o jogo monta, sem tirar o `.csv` do `.json`.
    #[test]
    fn factories_vao_ao_jogo_por_mod() {
        let (a, b, va) = (parse_xl("factories:\n  - a\\f.csv\n").unwrap(), parse_xl("factories: [ b\\f.csv, b\\g.csv ]\n").unwrap(), parse_xl(VA).unwrap());
        let mods = [(PathBuf::from("/j/Mods/A/a.xl"), &a), (PathBuf::from("/j/Mods/B/b.xl"), &b), (PathBuf::from("/j/Mods/V/v.xl"), &va)];
        let plano = planejar(&mods, &sem_concorrentes(), true);
        let destinos: Vec<String> = plano.configs.iter().map(|c| c.destino.display().to_string()).collect();
        assert_eq!(
            destinos,
            ["archive/Mac/mod/BWMS-Mods/xl/0001-a.json", "archive/Mac/mod/BWMS-Mods/xl/0002-b.json", "archive/Mac/mod/BWMS-Mods/xl/0003-v.json"]
        );
        assert_eq!(factories_do_json(&plano.configs[1].conteudo), ["b\\f.csv", "b\\g.csv"]);
        assert!(json::ler(&plano.configs[0].conteudo).unwrap().campo("localization").is_none());
        assert!(factories_do_json(&plano.configs[2].conteudo).is_empty());
        assert!(plano.no_jogo(&mods[0].0, "factories") && plano.no_jogo(&mods[1].0, "factories") && !plano.no_jogo(&mods[2].0, "factories"));
        assert_eq!(plano.motivo, None);
        let montados: HashSet<u64> = ["a\\f.csv", "b\\f.csv"].iter().map(|c| crate::resource_path_hash(c)).collect();
        assert_eq!(csvs_fora_do_jogo(&plano, &montados), [(mods[1].0.clone(), Motivo::CsvForaDoJogo(vec!["b\\g.csv".into()]))]);
        let (pt, en) = Motivo::CsvForaDoJogo(vec!["b\\g.csv".into()]).texto();
        assert!(pt.starts_with("b\\g.csv não está em nenhum .archive") && en.starts_with("b\\g.csv is not in any .archive"), "{pt} / {en}");
    }

    /// Com os textos no BWMS (mod do mod.io com textos), o `factories` vai ao jogo mesmo assim, num `.json` sem `localization`.
    #[test]
    fn factories_vao_com_os_textos_no_bwms() {
        let xl = parse_xl(&format!("{VA}factories: v\\f.csv\n")).unwrap();
        let p = PathBuf::from("/j/Mods/V/v.xl");
        let modio = Concorrentes { modio: Ok(vec!["123".into()]), outras_pastas: Vec::new() };
        let plano = planejar(&[(p.clone(), &xl)], &modio, true);
        assert_eq!(plano.motivo, Some(Motivo::ModIo(vec!["123".into()])));
        assert_eq!(plano.configs.len(), 1);
        assert_eq!(plano.configs[0].conteudo, "{\n    \"factories\": [\n        \"v\\\\f.csv\"\n    ]\n}\n");
        assert!(plano.no_jogo(&p, "factories") && !plano.no_jogo(&p, "localization.onscreens"));
    }
}
