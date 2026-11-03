//! A saída da biblioteca da 0.1.2–0.1.4. Até a 0.1.4 havia uma 3ª forma de pôr mod no jogo, além da `Mods/` e do mod.io: o staging
//! `BWMS/mods/<categoria>/<mod>/` e a biblioteca Nexus `BWMS/nexus/mods/<id>/`, que o apply copiava para o jogo (`.archive` com o prefixo
//! `basegame_zzbwms_` em `archive/Mac/content/`, tweaks em `r6/tweaks/`, scripts em `r6/scripts/`), registrando cada destino em
//! `.cp77-mods/bwms-applied.txt` e em `BWMS/nexus/deploy.json`. O `bwms mods` do GitHub também reconstruía o `r6/cache/tweakdb.bin`
//! (backup em `.cp77-mods/tweakdb-orig/`). O instalador da 0.1.5 roda `migrar` (`bwms-helper migrar <jogo>`) a cada instalação:
//!
//! 1. Leva para `Mods/<nome>` cada mod que estava no jogo: o que o `bwms-mods.json` diz ligado (o apply só gravava `active` junto com a
//!    cópia). Sem `bwms-mods.json` e com um dos registros, todos (o apply seguinte da biblioteca os ligaria); sem os dois, nenhum (a
//!    biblioteca nunca aplicou nada). A cópia é `std::fs::copy` arquivo a arquivo, que no APFS é clone (`fclonefileat`, sem espaço extra) e fora dele cópia; a
//!    origem fica. Nome que começa com `.` (o `.cp77-mods-origem` do install, `.DS_Store`), link e arquivo especial não vão (a travessia da
//!    `Mods/` também os pula), e o manifesto da biblioteca Nexus (`bwms-biblioteca.toml`, `bwms.toml` na raiz do mod) fica.
//! 2. `Mods/<nome>` que já existe não é sobrescrito: o mod fica na origem e o relatório diz para comparar as duas pastas. Mod desligado ou
//!    que nunca foi aplicado também fica na origem, listado: levá-lo poria no jogo um mod que não estava lá.
//! 3. Sem nenhum mod que falhou, tira do jogo cada arquivo dos dois registros, pela lista (nunca por glob), e só o que tem a forma do que o
//!    apply punha (`da_biblioteca`). Os registros saem depois, se nenhuma remoção falhou.
//! 4. Põe de volta o `tweakdb.bin` do jogo quando o vivo é o último que o `bwms mods` gravou (`escrito.txt`, tamanho e mtime em ns).
//!
//! O resultado vai para `REGISTRO`, que o runtime mostra no relatório da `Mods/` (`linhas_do_relatorio`) enquanto a pasta `BWMS/` existir.
//! Rodar de novo não duplica nada: o mod já levado (no registro anterior, com o destino no disco) continua levado.

use crate::instalacao::apply::PASTA_MODS;
use crate::instalacao::theme::{load_states, Fonte, ModState, StateLoad};
use crate::json::Valor;
use std::path::{Component, Path, PathBuf};

const STAGING: &str = "BWMS/mods";
const NEXUS_MODS: &str = "BWMS/nexus/mods";
/// A pasta `BWMS/` da biblioteca: com ela no disco, o relatório lembra o que a migração fez.
const PASTA_DA_BIBLIOTECA: &str = "BWMS";
const DEPLOY: &str = "BWMS/nexus/deploy.json";
const APPLIED: &str = ".cp77-mods/bwms-applied.txt";
/// A marca que fazia o boot regravar as tabelas quando o apply as tinha gravado; o boot da 0.1.5 regrava sempre.
const MARCA_DA_PASTA_MODS: &str = ".cp77-mods/bwms-tabelas-da-pasta-mods.txt";
const TWEAKDB_ORIG: &str = ".cp77-mods/tweakdb-orig";
const TWEAKDB: &str = "r6/cache/tweakdb.bin";
/// O que a migração fez, uma linha por item (`Item::linha_do_registro`).
pub const REGISTRO: &str = ".cp77-mods/bwms-migracao-da-biblioteca.txt";
/// Os destinos que o apply dava ao `.archive` (`basegame_zzbwms_<slug>[__<stem>].archive`), relativos ao jogo.
const PREFIXO_NO_CONTENT: &str = "archive/Mac/content/basegame_zzbwms_";
const MANIFESTOS: [&str; 2] = ["bwms-biblioteca.toml", "bwms.toml"];
/// A cópia em andamento: `Mods/.bwms-migrando-<nome>`. O ponto deixa a sobra de uma cópia interrompida fora da travessia da `Mods/`.
const TEMPORARIO: &str = ".bwms-migrando-";
/// Título da seção no relatório da `Mods/`.
pub const TITULO: &str = "Biblioteca antiga / Old library";

/// Uma coisa que a migração fez ou deixou de fazer. Caminhos relativos ao jogo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// O mod foi para `destino` (nesta rodada ou numa anterior).
    Levado { origem: String, destino: String },
    /// Desligado na biblioteca ou nunca aplicado: não estava no jogo e ficou na origem.
    Fora { origem: String },
    /// `destino` já existia e não foi sobrescrito.
    Repetido { origem: String, destino: String },
    /// Outra pasta da biblioteca com o mesmo nome era a que o apply usava (`reconcile`: a categoria gravada); esta ficou.
    MesmoNome { origem: String },
    /// A cópia falhou; o mod ficou só na origem.
    Falhou { origem: String, erro: String },
    /// Link ou arquivo especial dentro de um mod levado: não copiado.
    Pulado { caminho: String },
    /// A limpeza do jogo esperou: com um mod não levado, tirar os arquivos dele do jogo o tiraria do jogo.
    LimpezaAdiada,
    /// Arquivos dos registros que saíram do jogo.
    Tirados(usize),
    /// Linha de registro fora das pastas em que o apply punha arquivos: nada apagado.
    Estranho { caminho: String },
    /// Arquivo do registro que não saiu (o erro).
    FicouNoJogo { caminho: String, erro: String },
    /// Registro ou estado ilegível: nada foi levado nem tirado do jogo.
    Ilegivel { caminho: String, detalhe: String },
    /// O `tweakdb.bin` voltou ao do jogo.
    TweakdbDoJogo,
}

/// Campo do registro: tab e quebra de linha viram espaço (o registro é uma linha por item, campos por tab).
fn campo(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

impl Item {
    fn linha_do_registro(&self) -> String {
        let c = campo;
        match self {
            Item::Levado { origem, destino } => format!("levado\t{}\t{}", c(origem), c(destino)),
            Item::Fora { origem } => format!("fora\t{}", c(origem)),
            Item::Repetido { origem, destino } => format!("repetido\t{}\t{}", c(origem), c(destino)),
            Item::MesmoNome { origem } => format!("mesmo-nome\t{}", c(origem)),
            Item::Falhou { origem, erro } => format!("falhou\t{}\t{}", c(origem), c(erro)),
            Item::Pulado { caminho } => format!("pulado\t{}", c(caminho)),
            Item::LimpezaAdiada => "limpeza-adiada".to_string(),
            Item::Tirados(n) => format!("tirados\t{n}"),
            Item::Estranho { caminho } => format!("estranho\t{}", c(caminho)),
            Item::FicouNoJogo { caminho, erro } => format!("ficou-no-jogo\t{}\t{}", c(caminho), c(erro)),
            Item::Ilegivel { caminho, detalhe } => format!("ilegivel\t{}\t{}", c(caminho), c(detalhe)),
            Item::TweakdbDoJogo => "tweakdb-do-jogo".to_string(),
        }
    }

    fn da_linha(l: &str) -> Option<Item> {
        let f: Vec<&str> = l.split('\t').collect();
        let s = |i: usize| f.get(i).map(|x| x.to_string());
        Some(match *f.first()? {
            "levado" => Item::Levado { origem: s(1)?, destino: s(2)? },
            "fora" => Item::Fora { origem: s(1)? },
            "repetido" => Item::Repetido { origem: s(1)?, destino: s(2)? },
            "mesmo-nome" => Item::MesmoNome { origem: s(1)? },
            "falhou" => Item::Falhou { origem: s(1)?, erro: s(2)? },
            "pulado" => Item::Pulado { caminho: s(1)? },
            "limpeza-adiada" => Item::LimpezaAdiada,
            "tirados" => Item::Tirados(s(1)?.parse().ok()?),
            "estranho" => Item::Estranho { caminho: s(1)? },
            "ficou-no-jogo" => Item::FicouNoJogo { caminho: s(1)?, erro: s(2)? },
            "ilegivel" => Item::Ilegivel { caminho: s(1)?, detalhe: s(2)? },
            "tweakdb-do-jogo" => Item::TweakdbDoJogo,
            _ => return None,
        })
    }

    /// A linha do relatório e da saída do instalador, `"<pt> / <en>"`, com a ação.
    pub fn texto(&self) -> String {
        match self {
            Item::Levado { origem, destino } => format!("levado / moved: {origem} → {destino}"),
            Item::Fora { origem } => format!(
                "ficou / left: {origem} — estava desligado na biblioteca ou nunca foi aplicado; para usar, copie a pasta para a Mods / it was disabled in \
                 the library or never applied; to use it, copy the folder to Mods"
            ),
            Item::Repetido { origem, destino } => format!(
                "ficou / left: {origem} — {destino} já existe e não foi sobrescrito; compare as duas pastas e fique com uma / {destino} already exists \
                 and was not overwritten; compare both folders and keep one"
            ),
            Item::MesmoNome { origem } => format!(
                "ficou / left: {origem} — outra pasta de mesmo nome era a que a biblioteca aplicava; se quiser esta, copie-a para a Mods com outro nome \
                 / another folder with the same name was the one the library applied; if you want this one, copy it to Mods under another name"
            ),
            Item::Falhou { origem, erro } => format!(
                "não levado / not moved: {origem} — {erro}; libere espaço no disco do jogo e rode o instalador de novo / free space on the game disk \
                 and run the installer again"
            ),
            Item::Pulado { caminho } => format!(
                "fora / left out: {caminho} — link ou arquivo especial, não copiado; ponha o arquivo real na pasta do mod na Mods / link or special \
                 file, not copied; put the real file in the mod folder in Mods"
            ),
            Item::LimpezaAdiada => "os arquivos que a biblioteca pôs no jogo ficaram: um mod não foi levado; resolva a linha acima e rode o instalador \
                                   de novo / the files the library put in the game stayed: a mod was not moved; fix the line above and run the installer \
                                   again"
                .to_string(),
            Item::Tirados(n) => format!(
                "{n} arquivo(s) que a biblioteca tinha posto no jogo (archive/Mac/content, r6/tweaks, r6/scripts) saíram; os mesmos mods agora entram \
                 pela Mods / {n} file(s) the library had put in the game (archive/Mac/content, r6/tweaks, r6/scripts) were removed; the same mods now \
                 load from Mods"
            ),
            Item::Estranho { caminho } => format!(
                "ficou no jogo / left in the game: {caminho} — linha do registro da biblioteca fora das pastas em que ela punha arquivos; nada foi \
                 apagado, confira o arquivo / library record line outside the folders it used; nothing was deleted, check the file"
            ),
            Item::FicouNoJogo { caminho, erro } => format!(
                "ficou no jogo / left in the game: {caminho} — {erro}; confira a permissão da pasta e rode o instalador de novo / check the folder \
                 permissions and run the installer again"
            ),
            Item::Ilegivel { caminho, detalhe } => format!(
                "a migração não rodou / migration did not run: {caminho} ilegível ({detalhe}); nada foi levado nem tirado do jogo. Corrija o arquivo \
                 (ou a permissão dele) e rode o instalador de novo / {caminho} unreadable; nothing was moved or removed. Fix the file (or its \
                 permissions) and run the installer again"
            ),
            Item::TweakdbDoJogo => "r6/cache/tweakdb.bin voltou ao do jogo (o bwms mods o tinha reconstruído com os tweaks); os tweaks dos mods da Mods \
                                    entram ao carregar o save / r6/cache/tweakdb.bin is back to the game's (bwms mods had rebuilt it with the tweaks); \
                                    Mods tweaks apply when a save loads"
                .to_string(),
        }
    }
}

/// Caminho que não existe, nem como link pendente.
fn ausente(p: &Path) -> bool {
    matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// As pastas de `dir` (sem seguir link), em ordem de nome, fora as que começam com `.` (`.trocas` do install, `.DS_Store`). Sem `dir`, nenhuma.
fn pastas(dir: &Path) -> std::io::Result<Vec<(String, PathBuf)>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut v = Vec::new();
    for e in rd {
        let e = e?;
        let nome = e.file_name().to_string_lossy().into_owned();
        if !nome.starts_with('.') && e.file_type()?.is_dir() {
            v.push((nome, e.path()));
        }
    }
    v.sort();
    Ok(v)
}

/// Um mod da biblioteca: de onde vem, relativo ao jogo, e o nome dele na `Mods/`.
struct ModDaBiblioteca {
    origem: String,
    dir: PathBuf,
    nome: String,
    fonte: Fonte,
    categoria: String,
}

/// Os mods do staging em ordem (categoria, nome) e depois os da biblioteca Nexus em ordem de id: a ordem em que `migrar` os leva.
fn mods_da_biblioteca(game: &Path) -> std::io::Result<Vec<ModDaBiblioteca>> {
    let mut v = Vec::new();
    for (cat, cdir) in pastas(&game.join(STAGING))? {
        for (nome, dir) in pastas(&cdir)? {
            v.push(ModDaBiblioteca { origem: format!("{STAGING}/{cat}/{nome}"), dir, nome, fonte: Fonte::Staging, categoria: cat.clone() });
        }
    }
    for (id, dir) in pastas(&game.join(NEXUS_MODS))? {
        v.push(ModDaBiblioteca { origem: format!("{NEXUS_MODS}/{id}"), dir, nome: id, fonte: Fonte::Biblioteca, categoria: String::new() });
    }
    Ok(v)
}

/// Copia a árvore `de` para `para` (que não existe): pasta e arquivo regular, por `std::fs::copy` (clone no APFS). Nome com `.` no começo
/// fica; na raiz, com `sem_manifesto`, os manifestos da biblioteca Nexus também. Link e arquivo especial vão para `pulados`.
fn copiar_arvore(de: &Path, para: &Path, raiz: bool, sem_manifesto: bool, pulados: &mut Vec<PathBuf>) -> std::io::Result<()> {
    std::fs::create_dir(para)?;
    let mut entradas: Vec<std::fs::DirEntry> = std::fs::read_dir(de)?.collect::<Result<_, _>>()?;
    entradas.sort_by_key(|e| e.file_name());
    for e in entradas {
        let nome = e.file_name();
        let texto = nome.to_string_lossy();
        if texto.starts_with('.') || (raiz && sem_manifesto && MANIFESTOS.contains(&texto.as_ref())) {
            continue;
        }
        let tipo = e.file_type()?;
        if tipo.is_dir() {
            copiar_arvore(&e.path(), &para.join(&nome), false, sem_manifesto, pulados)?;
        } else if tipo.is_file() {
            std::fs::copy(e.path(), para.join(&nome))?;
        } else {
            pulados.push(e.path());
        }
    }
    Ok(())
}

/// Leva `m` para `<mods>/<nome>`: copia para `Mods/.bwms-migrando-<nome>` e troca o nome no fim, para uma cópia interrompida nunca virar um mod
/// pela metade na `Mods/`. Sobra de uma rodada interrompida no temporário sai antes (a pasta é só da migração).
fn levar(m: &ModDaBiblioteca, mods: &Path, pulados: &mut Vec<PathBuf>) -> std::io::Result<()> {
    std::fs::create_dir_all(mods)?;
    let temp = mods.join(format!("{TEMPORARIO}{}", m.nome));
    if !ausente(&temp) {
        std::fs::remove_dir_all(&temp)?;
    }
    let destino = mods.join(&m.nome);
    let r = copiar_arvore(&m.dir, &temp, true, m.fonte == Fonte::Biblioteca, pulados).and_then(|()| {
        if ausente(&destino) {
            std::fs::rename(&temp, &destino)
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} apareceu durante a cópia", destino.display())))
        }
    });
    if r.is_err() {
        let _ = std::fs::remove_dir_all(&temp);
    }
    r
}

/// Caminho de registro que pode ir para `game.join`: só componentes normais. `..` sai da pasta do jogo e caminho absoluto troca a base.
fn dentro_do_jogo(rel: &str) -> bool {
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// O destino tem a forma do que o apply punha no jogo: o `.archive` com o prefixo do BWMS direto em `archive/Mac/content/`, ou um arquivo
/// debaixo de `r6/tweaks/` ou `r6/scripts/`.
fn da_biblioteca(rel: &str) -> bool {
    let archive = rel.strip_prefix(PREFIXO_NO_CONTENT).is_some_and(|resto| !resto.contains('/') && resto.ends_with(".archive"));
    archive || ["r6/tweaks/", "r6/scripts/"].iter().any(|p| rel.strip_prefix(p).is_some_and(|resto| !resto.is_empty()))
}

/// As linhas de `bwms-applied.txt` (sem o arquivo, nenhuma). A linha só com o nome do arquivo é do formato antigo, relativo a
/// `archive/Mac/content/`.
fn ler_applied(game: &Path) -> Result<Vec<String>, String> {
    match std::fs::read_to_string(game.join(APPLIED)) {
        Ok(t) => Ok(t
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| if l.contains('/') { l.to_string() } else { format!("archive/Mac/content/{l}") })
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// Os `dst` de `deploy.json` (schema 1; sem o arquivo, nenhum).
fn ler_deploy(game: &Path) -> Result<Vec<String>, String> {
    let texto = match std::fs::read(game.join(DEPLOY)) {
        Ok(b) => String::from_utf8(b).map_err(|_| "o arquivo não é texto UTF-8".to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    let doc = crate::json::ler(&texto)?;
    match doc.campo("schema") {
        Some(Valor::Numero(n)) if n == "1" => {}
        _ => return Err("falta \"schema\":1".into()),
    }
    let Some(Valor::Lista(itens)) = doc.campo("deployed") else { return Err("falta a lista \"deployed\"".into()) };
    itens
        .iter()
        .enumerate()
        .map(|(i, item)| match item.campo("dst") {
            Some(Valor::Texto(s)) => Ok(s.clone()),
            _ => Err(format!("o item {} de \"deployed\" não tem \"dst\" em texto", i + 1)),
        })
        .collect()
}

/// Depois de tirar `rel`, as pastas que ficaram vazias até `r6/tweaks` ou `r6/scripts` (exclusive): o apply as criou para o arquivo.
/// `remove_dir` só tira pasta vazia.
fn tirar_pastas_vazias(game: &Path, rel: &str) {
    let mut p = Path::new(rel).parent();
    while let Some(d) = p.filter(|d| d.components().count() > 2) {
        if std::fs::remove_dir(game.join(d)).is_err() {
            return;
        }
        p = d.parent();
    }
}

/// Tira do jogo os arquivos dos dois registros e, sem erro de remoção, os registros.
fn limpar(game: &Path, itens: &mut Vec<Item>) {
    let applied = ler_applied(game);
    let deploy = ler_deploy(game);
    let (applied, deploy) = match (applied, deploy) {
        (Ok(a), Ok(d)) => (a, d),
        (Err(detalhe), _) => return itens.push(Item::Ilegivel { caminho: APPLIED.into(), detalhe }),
        (_, Err(detalhe)) => return itens.push(Item::Ilegivel { caminho: DEPLOY.into(), detalhe }),
    };
    let destinos: std::collections::BTreeSet<String> = applied.into_iter().chain(deploy).collect();
    let mut tirados = 0usize;
    let mut falhou = false;
    for rel in &destinos {
        if !dentro_do_jogo(rel) || !da_biblioteca(rel) {
            itens.push(Item::Estranho { caminho: rel.clone() });
            continue;
        }
        match std::fs::remove_file(game.join(rel)) {
            Ok(()) => {
                tirados += 1;
                tirar_pastas_vazias(game, rel);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                falhou = true;
                itens.push(Item::FicouNoJogo { caminho: rel.clone(), erro: e.to_string() });
            }
        }
    }
    if tirados > 0 {
        itens.push(Item::Tirados(tirados));
    }
    if !falhou {
        for registro in [APPLIED, DEPLOY, MARCA_DA_PASTA_MODS] {
            if let Err(e) = std::fs::remove_file(game.join(registro)) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    itens.push(Item::FicouNoJogo { caminho: registro.into(), erro: e.to_string() });
                }
            }
        }
    }
}

/// Tamanho e mtime em ns: a marca que o `bwms mods` gravava em `tweakdb-orig/escrito.txt` depois de reconstruir o `tweakdb.bin`.
fn marca_do_arquivo(p: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(p).ok().map(|m| format!("{} {}", m.len(), m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128))
}

/// O `tweakdb.bin` vivo que é o último que o `bwms mods` gravou volta ao backup (o vanilla da build instalada: o `bwms mods` o renovava
/// sempre que o vivo não era dele). Vivo de outra origem (o jogo o regravou numa atualização) fica. A pasta do backup sai nos dois casos.
fn tweakdb_do_jogo(game: &Path, itens: &mut Vec<Item>) {
    let dir = game.join(TWEAKDB_ORIG);
    if ausente(&dir) {
        return;
    }
    let vivo = game.join(TWEAKDB);
    let backup = dir.join("tweakdb.bin");
    let nosso = std::fs::read_to_string(dir.join("escrito.txt")).ok().is_some_and(|r| marca_do_arquivo(&vivo).is_some_and(|m| m == r));
    if nosso && backup.is_file() {
        let temp = vivo.with_file_name(format!(".tweakdb.bin.bwms-migracao.{}.tmp", std::process::id()));
        let r = std::fs::copy(&backup, &temp).and_then(|_| std::fs::rename(&temp, &vivo));
        if let Err(e) = r {
            let _ = std::fs::remove_file(&temp);
            return itens.push(Item::FicouNoJogo { caminho: TWEAKDB.into(), erro: e.to_string() });
        }
        itens.push(Item::TweakdbDoJogo);
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        itens.push(Item::FicouNoJogo { caminho: TWEAKDB_ORIG.into(), erro: e.to_string() });
    }
}

/// O registro de uma rodada anterior (sem o arquivo ou ilegível, nenhum item).
pub fn ler_registro(game: &Path) -> Vec<Item> {
    std::fs::read_to_string(game.join(REGISTRO)).map(|t| t.lines().filter_map(Item::da_linha).collect()).unwrap_or_default()
}

/// A migração (ver o doc do módulo). Devolve o que fez, na ordem; sem biblioteca, sem registro e sem resto dela no jogo, nada (e não grava o
/// registro). `Err` = o registro não gravou; o que a migração fez está no disco e nos itens.
pub fn migrar(game: &Path) -> std::io::Result<Vec<Item>> {
    let anterior = ler_registro(game);
    let mods = match mods_da_biblioteca(game) {
        Ok(m) => m,
        Err(e) => return gravar(game, vec![Item::Ilegivel { caminho: STAGING.into(), detalhe: e.to_string() }]),
    };
    let restos = [APPLIED, DEPLOY, MARCA_DA_PASTA_MODS, TWEAKDB_ORIG].iter().any(|r| !ausente(&game.join(r)));
    if mods.is_empty() && !restos {
        return Ok(Vec::new());
    }
    let estados: Vec<ModState> = match load_states(game) {
        StateLoad::Loaded(s) => s,
        // Sem estado e com registro, o apply seguinte da biblioteca ligaria todo mod (o `reconcile` liga o mod que não tem estado), e o que
        // ele pôs no jogo vem de mods que ninguém desligou: todos vão.
        StateLoad::Missing if [APPLIED, DEPLOY].iter().any(|r| !ausente(&game.join(r))) => {
            let mut todos: Vec<ModState> = Vec::new();
            for m in &mods {
                if !todos.iter().any(|s| s.fonte == m.fonte && s.name == m.nome) {
                    todos.push(ModState {
                        name: m.nome.clone(),
                        category: m.categoria.clone(),
                        fonte: m.fonte,
                        active: true,
                        favorite: false,
                        order: 0,
                        variant: String::new(),
                    });
                }
            }
            todos
        }
        StateLoad::Missing => Vec::new(),
        StateLoad::Invalid(detalhe) => return gravar(game, vec![Item::Ilegivel { caminho: ".cp77-mods/bwms-mods.json".into(), detalhe }]),
        StateLoad::IoError(e) => return gravar(game, vec![Item::Ilegivel { caminho: ".cp77-mods/bwms-mods.json".into(), detalhe: e.to_string() }]),
    };
    let pasta_mods = game.join(PASTA_MODS);
    let mut itens = Vec::new();
    let mut pulados = Vec::new();
    let mut falhou = false;
    for m in &mods {
        let Some(estado) = estados.iter().find(|s| s.fonte == m.fonte && s.name == m.nome).filter(|s| s.active) else {
            itens.push(Item::Fora { origem: m.origem.clone() });
            continue;
        };
        // Mesmo nome em duas categorias era um mod só no apply: o da categoria gravada, ou o da 1ª categoria em ordem com esse nome.
        if m.fonte == Fonte::Staging {
            let mesmo_nome = || mods.iter().filter(|o| o.fonte == Fonte::Staging && o.nome == m.nome);
            let aplicado = mesmo_nome().find(|o| o.categoria == estado.category).or_else(|| mesmo_nome().next());
            if aplicado.is_some_and(|o| o.origem != m.origem) {
                itens.push(Item::MesmoNome { origem: m.origem.clone() });
                continue;
            }
        }
        let destino = format!("{PASTA_MODS}/{}", m.nome);
        if !ausente(&pasta_mods.join(&m.nome)) {
            let ja_levado = anterior.iter().any(|i| matches!(i, Item::Levado { origem, destino: d } if origem == &m.origem && d == &destino));
            itens.push(if ja_levado { Item::Levado { origem: m.origem.clone(), destino } } else { Item::Repetido { origem: m.origem.clone(), destino } });
            continue;
        }
        match levar(m, &pasta_mods, &mut pulados) {
            Ok(()) => itens.push(Item::Levado { origem: m.origem.clone(), destino }),
            Err(e) => {
                falhou = true;
                itens.push(Item::Falhou { origem: m.origem.clone(), erro: e.to_string() });
            }
        }
    }
    itens.extend(pulados.iter().map(|p| Item::Pulado { caminho: p.strip_prefix(game).unwrap_or(p).display().to_string() }));
    if falhou {
        itens.push(Item::LimpezaAdiada);
    } else {
        limpar(game, &mut itens);
        tweakdb_do_jogo(game, &mut itens);
        // O que uma rodada anterior tirou do jogo continua dito no relatório.
        let antes: usize = anterior.iter().map(|i| if let Item::Tirados(n) = i { *n } else { 0 }).sum();
        if antes > 0 {
            match itens.iter_mut().find_map(|i| if let Item::Tirados(n) = i { Some(n) } else { None }) {
                Some(n) => *n += antes,
                None => itens.push(Item::Tirados(antes)),
            }
        }
        if anterior.contains(&Item::TweakdbDoJogo) && !itens.contains(&Item::TweakdbDoJogo) {
            itens.push(Item::TweakdbDoJogo);
        }
    }
    gravar(game, itens)
}

/// Grava `itens` em `REGISTRO` (registro igual não é regravado: a reinstalação não muda nada no jogo) e os devolve.
fn gravar(game: &Path, itens: Vec<Item>) -> std::io::Result<Vec<Item>> {
    let texto: String = itens.iter().map(|i| i.linha_do_registro() + "\n").collect();
    if std::fs::read(game.join(REGISTRO)).is_ok_and(|no_disco| no_disco == texto.as_bytes()) {
        return Ok(itens);
    }
    crate::instalacao::theme::gravar_atomico(&game.join(REGISTRO), texto.as_bytes())
        .map_err(|e| std::io::Error::new(e.kind(), format!("não foi possível gravar {}: {e}", game.join(REGISTRO).display())))?;
    Ok(itens)
}

/// As linhas da seção `TITULO` do relatório da `Mods/`: o que a migração fez, enquanto a pasta `BWMS/` existir ou algo dela tiver ficado
/// no jogo. Sem registro, `None`.
pub fn linhas_do_relatorio(game: &Path) -> Option<Vec<String>> {
    let itens = ler_registro(game);
    let pendente = itens.iter().any(|i| matches!(i, Item::FicouNoJogo { .. } | Item::Ilegivel { .. } | Item::Falhou { .. } | Item::LimpezaAdiada));
    if itens.is_empty() || (ausente(&game.join(PASTA_DA_BIBLIOTECA)) && !pendente) {
        return None;
    }
    let mut linhas = Vec::new();
    if !itens.iter().any(|i| matches!(i, Item::Ilegivel { .. })) {
        linhas.push(
            "A 0.1.5 não lê mais a biblioteca (BWMS/mods e BWMS/nexus): os mods que estavam ligados nela vieram para a Mods (clonados; em disco APFS, \
             sem espaço extra), e a pasta BWMS ficou como estava. Confira os mods aqui e, quando quiser, mova a pasta BWMS para a Lixeira. / 0.1.5 no \
             longer reads the library (BWMS/mods and BWMS/nexus): the mods enabled there were moved to Mods (cloned; on an APFS disk, no extra space), \
             and the BWMS folder was left as it was. Check the mods here and, when you want, move the BWMS folder to the Trash."
                .to_string(),
        );
    }
    linhas.extend(itens.iter().map(Item::texto));
    Some(linhas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jogo(nome: &str) -> PathBuf {
        let g = std::env::temp_dir().join(format!("bwms-migracao-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        std::fs::create_dir_all(g.join("archive/Mac/content")).unwrap();
        g
    }

    fn escrever(g: &Path, rel: &str, conteudo: &str) {
        let p = g.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, conteudo).unwrap();
    }

    fn ler(g: &Path, rel: &str) -> String {
        std::fs::read_to_string(g.join(rel)).unwrap()
    }

    /// O estado como a biblioteca o gravava: `(nome, categoria, fonte, ligado)`.
    fn estado(g: &Path, mods: &[(&str, &str, &str, bool)]) {
        let itens: Vec<String> = mods
            .iter()
            .map(|(n, c, f, a)| format!("{{\"name\":\"{n}\",\"category\":\"{c}\",\"fonte\":\"{f}\",\"active\":{a},\"favorite\":false,\"order\":0,\"variant\":\"\"}}"))
            .collect();
        escrever(g, ".cp77-mods/bwms-mods.json", &format!("{{\n  \"schema\":2,\n  \"mods\":[\n    {}\n  ]\n}}\n", itens.join(",\n    ")));
    }

    fn deploy(g: &Path, dsts: &[&str]) {
        let itens: Vec<String> =
            dsts.iter().map(|d| format!("{{\"unique_id\":\"autor.mod\",\"src\":\"m.archive\",\"dst\":\"{d}\",\"sha1\":\"\"}}")).collect();
        escrever(g, DEPLOY, &format!("{{\n\"schema\":1,\n\"deployed\":[\n{}\n]\n}}\n", itens.join(",\n")));
    }

    /// Uma instalação da 0.1.4 com a biblioteca em uso: X ligado do staging (archive, tweak, script e o marcador de origem do install), Y
    /// desligado, Z nunca aplicado, `autor.mod` ligado da biblioteca Nexus (com o manifesto), e o que o apply pôs no jogo nos dois registros.
    /// Do usuário, fora dos registros: um `.archive` em content e um tweak em `r6/tweaks`.
    fn instalacao_014(g: &Path) {
        escrever(g, "BWMS/mods/roupas/X/x.archive", "X");
        escrever(g, "BWMS/mods/roupas/X/r6/tweaks/x/x.yaml", "x: 1");
        escrever(g, "BWMS/mods/roupas/X/r6/scripts/x/x.reds", "// x");
        escrever(g, "BWMS/mods/roupas/X/.cp77-mods-origem", "origem");
        escrever(g, "BWMS/mods/roupas/Y/y.archive", "Y");
        escrever(g, "BWMS/mods/armas/Z/z.archive", "Z");
        escrever(g, "BWMS/nexus/mods/autor.mod/m.archive", "M");
        escrever(g, "BWMS/nexus/mods/autor.mod/bwms-biblioteca.toml", "schema = 2\n");
        estado(g, &[("X", "roupas", "staging", true), ("Y", "roupas", "staging", false), ("autor.mod", "", "biblioteca", true)]);
        escrever(g, "archive/Mac/content/basegame_zzbwms_x.archive", "X");
        escrever(g, "r6/tweaks/x/x.yaml", "x: 1");
        escrever(g, "r6/scripts/x/x.reds", "// x");
        // linha no formato antigo, só com o nome do arquivo
        escrever(g, APPLIED, "basegame_zzbwms_x.archive\nr6/tweaks/x/x.yaml\nr6/scripts/x/x.reds");
        escrever(g, "archive/Mac/content/basegame_zzbwms_autor_mod.archive", "M");
        deploy(g, &["archive/Mac/content/basegame_zzbwms_autor_mod.archive"]);
        escrever(g, MARCA_DA_PASTA_MODS, "Mods/A/a");
        escrever(g, "archive/Mac/content/basegame_meu.archive", "USUARIO");
        escrever(g, "r6/tweaks/meu.yaml", "meu: 1");
    }

    /// 0.1.4 → 0.1.5: o ligado vai para a `Mods/` inteiro (sem o que começa com ponto e sem o manifesto), a origem fica, o desligado e o nunca
    /// aplicado ficam listados, o que a biblioteca pôs no jogo sai pela lista e o do usuário fica. Rodar de novo dá o mesmo registro.
    #[test]
    fn leva_o_ligado_para_a_mods_e_tira_do_jogo_so_o_da_biblioteca() {
        let g = jogo("0-1-4");
        instalacao_014(&g);
        let itens = migrar(&g).unwrap();
        assert_eq!(
            itens,
            vec![
                Item::Fora { origem: "BWMS/mods/armas/Z".into() },
                Item::Levado { origem: "BWMS/mods/roupas/X".into(), destino: "Mods/X".into() },
                Item::Fora { origem: "BWMS/mods/roupas/Y".into() },
                Item::Levado { origem: "BWMS/nexus/mods/autor.mod".into(), destino: "Mods/autor.mod".into() },
                Item::Tirados(4),
            ]
        );
        assert_eq!(ler(&g, "Mods/X/x.archive"), "X");
        assert_eq!(ler(&g, "Mods/X/r6/tweaks/x/x.yaml"), "x: 1");
        assert_eq!(ler(&g, "Mods/X/r6/scripts/x/x.reds"), "// x");
        assert!(ausente(&g.join("Mods/X/.cp77-mods-origem")));
        assert_eq!(ler(&g, "Mods/autor.mod/m.archive"), "M");
        assert!(ausente(&g.join("Mods/autor.mod/bwms-biblioteca.toml")), "o manifesto da biblioteca não vai para a Mods");
        assert!(ausente(&g.join("Mods/Y")) && ausente(&g.join("Mods/Z")));
        for origem in ["BWMS/mods/roupas/X/x.archive", "BWMS/mods/roupas/X/.cp77-mods-origem", "BWMS/nexus/mods/autor.mod/bwms-biblioteca.toml"] {
            assert!(g.join(origem).is_file(), "a origem {origem} sumiu");
        }
        for tirado in ["archive/Mac/content/basegame_zzbwms_x.archive", "archive/Mac/content/basegame_zzbwms_autor_mod.archive", "r6/tweaks/x", "r6/scripts/x", APPLIED, DEPLOY, MARCA_DA_PASTA_MODS] {
            assert!(ausente(&g.join(tirado)), "{tirado} ficou");
        }
        assert_eq!(ler(&g, "archive/Mac/content/basegame_meu.archive"), "USUARIO");
        assert_eq!(ler(&g, "r6/tweaks/meu.yaml"), "meu: 1");
        let sobra: Vec<_> = std::fs::read_dir(g.join("Mods")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().starts_with('.')).collect();
        assert!(sobra.is_empty(), "temporário na Mods: {sobra:?}");

        let registro = ler(&g, REGISTRO);
        assert_eq!(migrar(&g).unwrap(), itens, "a 2ª rodada não leva de novo nem esquece o que saiu");
        assert_eq!(ler(&g, REGISTRO), registro);
        let linhas = linhas_do_relatorio(&g).unwrap();
        assert!(linhas[0].starts_with("A 0.1.5 não lê mais a biblioteca"), "{linhas:?}");
        assert!(linhas.contains(&"levado / moved: BWMS/mods/roupas/X → Mods/X".to_string()), "{linhas:?}");
        assert!(linhas.iter().any(|l| l.starts_with("4 arquivo(s) que a biblioteca tinha posto no jogo")), "{linhas:?}");
        // Sem a pasta BWMS (o usuário a mandou para a Lixeira), a seção some.
        std::fs::remove_dir_all(g.join("BWMS")).unwrap();
        assert_eq!(linhas_do_relatorio(&g), None);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// `Mods/<nome>` que já existe não é sobrescrito, e o mod fica listado; a limpeza roda (o mod está na `Mods/` do usuário).
    #[test]
    fn nome_repetido_na_mods_nao_sobrescreve() {
        let g = jogo("repetido");
        instalacao_014(&g);
        escrever(&g, "Mods/X/meu.archive", "DO USUARIO");
        let itens = migrar(&g).unwrap();
        assert!(itens.contains(&Item::Repetido { origem: "BWMS/mods/roupas/X".into(), destino: "Mods/X".into() }), "{itens:?}");
        assert_eq!(std::fs::read_dir(g.join("Mods/X")).unwrap().count(), 1);
        assert_eq!(ler(&g, "Mods/X/meu.archive"), "DO USUARIO");
        assert!(itens.contains(&Item::Tirados(4)), "{itens:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Estado ilegível: nada vai para a `Mods/` e nada sai do jogo; o registro diz o arquivo e a ação.
    #[test]
    fn estado_ilegivel_nao_mexe_em_nada() {
        let g = jogo("estado-ruim");
        instalacao_014(&g);
        escrever(&g, ".cp77-mods/bwms-mods.json", "{ isto não é o estado");
        let itens = migrar(&g).unwrap();
        assert!(matches!(itens.as_slice(), [Item::Ilegivel { caminho, .. }] if caminho == ".cp77-mods/bwms-mods.json"), "{itens:?}");
        assert!(ausente(&g.join("Mods")));
        assert!(g.join("archive/Mac/content/basegame_zzbwms_x.archive").exists() && g.join(APPLIED).exists());
        let linhas = linhas_do_relatorio(&g).unwrap();
        assert_eq!(linhas.len(), 1, "{linhas:?}");
        assert!(linhas[0].starts_with("a migração não rodou / migration did not run: .cp77-mods/bwms-mods.json ilegível ("), "{linhas:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Linha de registro que sai do jogo, ou fora das pastas em que o apply punha arquivos, não apaga nada.
    #[test]
    fn linha_de_registro_fora_das_pastas_da_biblioteca_nao_apaga_nada() {
        let g = jogo("registro-estranho");
        let fora = g.parent().unwrap().join(format!("bwms-migracao-fora-{}.txt", std::process::id()));
        std::fs::write(&fora, "USUARIO").unwrap();
        let nome_fora = fora.file_name().unwrap().to_string_lossy().into_owned();
        escrever(&g, "archive/Mac/content/basegame_meu.archive", "USUARIO");
        escrever(&g, "red4ext/bwms.toml", "USUARIO");
        escrever(
            &g,
            APPLIED,
            &format!("../{nome_fora}\n{}\nr6/../../{nome_fora}\narchive/Mac/content/basegame_meu.archive\nred4ext/bwms.toml\nr6/tweaks/\n", fora.display()),
        );
        let itens = migrar(&g).unwrap();
        assert_eq!(itens.iter().filter(|i| matches!(i, Item::Estranho { .. })).count(), 6, "{itens:?}");
        assert_eq!(std::fs::read_to_string(&fora).unwrap(), "USUARIO");
        assert_eq!(ler(&g, "archive/Mac/content/basegame_meu.archive"), "USUARIO");
        assert_eq!(ler(&g, "red4ext/bwms.toml"), "USUARIO");
        let _ = std::fs::remove_file(&fora);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Cópia que falha (a `Mods/` sem permissão de escrita): o mod fica só na origem, nada sai do jogo, e a rodada seguinte, com a permissão
    /// de volta, termina.
    #[test]
    fn copia_que_falha_adia_a_limpeza() {
        use std::os::unix::fs::PermissionsExt;
        let g = jogo("falha");
        instalacao_014(&g);
        std::fs::create_dir_all(g.join("Mods")).unwrap();
        std::fs::set_permissions(g.join("Mods"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let itens = migrar(&g);
        std::fs::set_permissions(g.join("Mods"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let itens = itens.unwrap();
        assert!(itens.iter().any(|i| matches!(i, Item::Falhou { origem, .. } if origem == "BWMS/mods/roupas/X")), "{itens:?}");
        assert_eq!(itens.last(), Some(&Item::LimpezaAdiada));
        assert!(g.join("archive/Mac/content/basegame_zzbwms_x.archive").exists() && g.join(APPLIED).exists() && g.join(DEPLOY).exists());
        assert!(linhas_do_relatorio(&g).unwrap().iter().any(|l| l.starts_with("não levado / not moved: BWMS/mods/roupas/X — ")));

        let itens = migrar(&g).unwrap();
        assert!(itens.contains(&Item::Levado { origem: "BWMS/mods/roupas/X".into(), destino: "Mods/X".into() }), "{itens:?}");
        assert!(itens.contains(&Item::Tirados(4)), "{itens:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Link dentro do mod não é seguido nem copiado, e fica listado.
    #[test]
    fn link_dentro_do_mod_fica_fora_e_listado() {
        let g = jogo("link");
        escrever(&g, "BWMS/mods/misc/L/l.archive", "L");
        escrever(&g, "fora/segredo.txt", "S");
        std::os::unix::fs::symlink(g.join("fora/segredo.txt"), g.join("BWMS/mods/misc/L/atalho.txt")).unwrap();
        estado(&g, &[("L", "misc", "staging", true)]);
        let itens = migrar(&g).unwrap();
        assert!(itens.contains(&Item::Pulado { caminho: "BWMS/mods/misc/L/atalho.txt".into() }), "{itens:?}");
        assert!(ausente(&g.join("Mods/L/atalho.txt")));
        assert_eq!(ler(&g, "Mods/L/l.archive"), "L");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Mesmo nome em duas categorias: vai o da categoria gravada; o outro fica listado.
    #[test]
    fn mesmo_nome_em_duas_categorias_leva_o_da_categoria_gravada() {
        let g = jogo("mesmo-nome");
        escrever(&g, "BWMS/mods/armas/A/a.archive", "ARMAS");
        escrever(&g, "BWMS/mods/roupas/A/a.archive", "ROUPAS");
        estado(&g, &[("A", "roupas", "staging", true)]);
        let itens = migrar(&g).unwrap();
        assert_eq!(
            itens,
            vec![Item::MesmoNome { origem: "BWMS/mods/armas/A".into() }, Item::Levado { origem: "BWMS/mods/roupas/A".into(), destino: "Mods/A".into() }]
        );
        assert_eq!(ler(&g, "Mods/A/a.archive"), "ROUPAS");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O `tweakdb.bin` que o `bwms mods` gravou por último volta ao backup; o que o jogo regravou fica. A pasta do backup sai nos dois casos.
    #[test]
    fn tweakdb_reconstruido_volta_ao_do_jogo() {
        let g = jogo("tweakdb");
        escrever(&g, "r6/cache/tweakdb.bin", "COM-TWEAKS");
        escrever(&g, ".cp77-mods/tweakdb-orig/tweakdb.bin", "VANILLA");
        escrever(&g, ".cp77-mods/tweakdb-orig/escrito.txt", &marca_do_arquivo(&g.join("r6/cache/tweakdb.bin")).unwrap());
        assert_eq!(migrar(&g).unwrap(), vec![Item::TweakdbDoJogo]);
        assert_eq!(ler(&g, "r6/cache/tweakdb.bin"), "VANILLA");
        assert!(ausente(&g.join(TWEAKDB_ORIG)));

        escrever(&g, "r6/cache/tweakdb.bin", "DO-JOGO-NOVO");
        escrever(&g, ".cp77-mods/tweakdb-orig/tweakdb.bin", "VANILLA-VELHO");
        escrever(&g, ".cp77-mods/tweakdb-orig/escrito.txt", "1 2");
        let itens = migrar(&g).unwrap();
        assert_eq!(itens, vec![Item::TweakdbDoJogo], "o da rodada anterior continua dito: {itens:?}");
        assert_eq!(ler(&g, "r6/cache/tweakdb.bin"), "DO-JOGO-NOVO");
        assert!(ausente(&g.join(TWEAKDB_ORIG)));
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Sem `bwms-mods.json` e com registro, todo mod vai (o apply seguinte os ligaria); sem registro também, nenhum (nunca aplicado).
    #[test]
    fn sem_estado_leva_todos_so_com_registro() {
        let g = jogo("sem-estado");
        escrever(&g, "BWMS/mods/roupas/X/x.archive", "X");
        escrever(&g, "BWMS/nexus/mods/autor.mod/m.archive", "M");
        assert_eq!(
            migrar(&g).unwrap(),
            vec![Item::Fora { origem: "BWMS/mods/roupas/X".into() }, Item::Fora { origem: "BWMS/nexus/mods/autor.mod".into() }]
        );
        escrever(&g, "archive/Mac/content/basegame_zzbwms_x.archive", "X");
        escrever(&g, APPLIED, "archive/Mac/content/basegame_zzbwms_x.archive\n");
        let itens = migrar(&g).unwrap();
        assert!(itens.contains(&Item::Levado { origem: "BWMS/mods/roupas/X".into(), destino: "Mods/X".into() }), "{itens:?}");
        assert!(itens.contains(&Item::Levado { origem: "BWMS/nexus/mods/autor.mod".into(), destino: "Mods/autor.mod".into() }), "{itens:?}");
        assert!(itens.contains(&Item::Tirados(1)), "{itens:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Jogo sem biblioteca nem resto dela: nada a fazer, e nenhum registro.
    #[test]
    fn sem_biblioteca_nao_faz_nada() {
        let g = jogo("vazio");
        assert_eq!(migrar(&g).unwrap(), Vec::new());
        assert!(ausente(&g.join(REGISTRO)) && ausente(&g.join("Mods")));
        assert_eq!(linhas_do_relatorio(&g), None);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O registro volta igual: cada item, com tab e quebra de linha do texto trocadas por espaço.
    #[test]
    fn registro_volta_igual() {
        let itens = vec![
            Item::Levado { origem: "BWMS/mods/a b/X".into(), destino: "Mods/X".into() },
            Item::Fora { origem: "o".into() },
            Item::Repetido { origem: "o".into(), destino: "d".into() },
            Item::MesmoNome { origem: "o".into() },
            Item::Falhou { origem: "o".into(), erro: "e".into() },
            Item::Pulado { caminho: "c".into() },
            Item::LimpezaAdiada,
            Item::Tirados(7),
            Item::Estranho { caminho: "c".into() },
            Item::FicouNoJogo { caminho: "c".into(), erro: "e".into() },
            Item::Ilegivel { caminho: "c".into(), detalhe: "d".into() },
            Item::TweakdbDoJogo,
        ];
        for i in &itens {
            assert_eq!(Item::da_linha(&i.linha_do_registro()).as_ref(), Some(i));
        }
        let com_tab = Item::Falhou { origem: "a\tb".into(), erro: "x\ny".into() };
        assert_eq!(Item::da_linha(&com_tab.linha_do_registro()), Some(Item::Falhou { origem: "a b".into(), erro: "x y".into() }));
        assert_eq!(Item::da_linha("outra coisa"), None);
    }
}
