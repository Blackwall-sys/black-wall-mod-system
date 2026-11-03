//! A pasta `Mods/` do jogo lida (`ler_pasta_mods`, `arquivos_da_pasta_mods`) e as tabelas de runtime de `red4ext/` que saem dos `.xl` dela
//! (`gravar_tabelas_de_runtime`). O runtime (`cp77-console`, `mods_do_usuario::pasta_mods`) regrava as tabelas a cada abertura do jogo,
//! com a `Mods/` de agora: é o único escritor delas.

use crate::tabelas::{self, Tabela};
use crate::PASTA_DO_RUNTIME;
use crate::instalacao::theme::gravar_atomico;
use crate::instalacao::tabelas_de_runtime::PlanoDeRedirecionamento;
use crate::instalacao::xl::XlFile;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// O que o aviso de um `.xl` da `Mods/` manda fazer depois de corrigi-lo.
const DE_NOVO: &str = "abra o jogo de novo";
/// O que o erro de gravação das tabelas manda fazer.
const REPETIR: &str = "abra o jogo de novo (o BWMS regrava as tabelas a cada abertura)";

/// Escreve `<red4ext>/bwms-reslink.txt` com o plano de `resource.link`/`resource.copy` de todos os `.xl` juntos
/// (`tabelas_de_runtime::build_apply_plan_combinado`: direção escalar/sequência, pedido e cópia expandidos pelo `resource.scope`, cadeia colapsada, ciclo,
/// cópia de si mesma e link cujo pedido é cópia fora, também entre mods), no formato que o runtime lê (`tabelas::Reslink`, o leitor de
/// `arquivos_do_jogo::reslink_file`). Devolve quantas linhas de par foram escritas (0 = nenhum link/copy; aí a tabela gerada sai, ver
/// `tirar_tabela_gerada`). `Err` = a tabela em disco é a da gravação anterior; o texto leva o caminho.
fn write_reslink_table(game: &Path, plano: &PlanoDeRedirecionamento) -> std::io::Result<usize> {
    let mut body = String::new();
    let mut pairs = 0usize;
    for line in crate::instalacao::tabelas_de_runtime::emit_reslink_do_plano(plano).lines().filter(|l| !l.starts_with('#')) {
        body.push_str(line);
        body.push('\n');
        pairs += 1;
    }
    let dest = game.join(PASTA_DO_RUNTIME).join(tabelas::RESLINK.arquivo);
    // Cabeçalho do gerador (`marca_do_install`), não o do emissor (`# bwms-reslink (gerado de .xl)`): esse é o do `bwms xl <f> reslink`,
    // posto à mão, que fica.
    if pairs == 0 {
        tirar_tabela_gerada(&dest, &tabelas::RESLINK.marca_do_install())?;
        return Ok(0);
    }
    let out = format!("{}{body}", tabelas::RESLINK.cabecalho_do_install(&format!("{pairs} par(es)")));
    gravar_tabela(&dest, out.as_bytes())?;
    Ok(pairs)
}

/// Erro de tabela com o caminho dela: o `Display` do `io::Error` traz só o motivo do SO, e `gravar_tabelas_dos_xl` junta as falhas das 9
/// tabelas numa mensagem só.
fn erro_de_tabela(dest: &Path, e: std::io::Error) -> std::io::Error {
    std::io::Error::new(e.kind(), format!("{}: {e}", dest.display()))
}

fn gravar_tabela(dest: &Path, conteudo: &[u8]) -> std::io::Result<()> {
    gravar_se_mudou(dest, conteudo).map_err(|e| erro_de_tabela(dest, e))
}

/// `gravar_atomico` só quando o arquivo em disco difere de `conteudo`: o boot regrava as tabelas a cada abertura, quase
/// sempre com o mesmo conteúdo, e cada gravação leva um F_FULLFSYNC. Arquivo igual já está inteiro no disco (o `rename` do `gravar_atomico`
/// só vem depois do `sync_all`).
fn gravar_se_mudou(dest: &Path, conteudo: &[u8]) -> std::io::Result<()> {
    if std::fs::read(dest).is_ok_and(|no_disco| no_disco == conteudo) {
        return Ok(());
    }
    gravar_atomico(dest, conteudo)
}

/// Tira de `red4ext/` uma tabela que ficou sem linha, mas só se o arquivo começar com o `cabecalho` do próprio
/// gerador: a gerada de um mod que saiu da `Mods/` continuaria valendo no boot (o runtime carrega a tabela que
/// existe), e arquivo posto à mão fica. Lida em bytes: arquivo posto à mão fora de UTF-8 não casa com o cabeçalho e fica, sem virar erro.
/// Tabela que não existe é `Ok`; leitura ou remoção que falha por outro motivo é `Err`, porque a tabela velha continua no disco.
fn tirar_tabela_gerada(dest: &Path, cabecalho: &str) -> std::io::Result<()> {
    let gerada = match std::fs::read(dest) {
        Ok(t) => t.starts_with(cabecalho.as_bytes()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(erro_de_tabela(dest, e)),
    };
    if !gerada {
        return Ok(());
    }
    match std::fs::remove_file(dest) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(erro_de_tabela(dest, e)),
    }
}

/// Grava uma tabela de runtime que sai de um emissor por `.xl` — `bwms-streaming.txt` (`streaming.sectors`, `#49`), `bwms-questphase.txt`
/// (`quest.phases`, `#35`) e `bwms-journal.txt` (`journal`, `#44`), entre outras — em `<red4ext>/<tabela.arquivo>`, com o cabeçalho do emissor
/// (`Tabela::cabecalho`) e o id do `.xl` (`xl_com_id`) como primeiro campo de cada linha.
/// Sem nenhuma linha, apaga a tabela só se ela tiver o cabeçalho do próprio emissor (`tirar_tabela_gerada`). Devolve quantas linhas foram escritas;
/// `Err` como em `write_reslink_table`.
fn gravar_tabela_de_runtime(game: &Path, xls: &[(String, XlFile)], tabela: &Tabela, emite: &dyn Fn(&XlFile, &str) -> String) -> std::io::Result<usize> {
    let mut body = String::new();
    let mut count = 0usize;
    for (mod_id, xl) in xls {
        for line in emite(xl, mod_id).lines() {
            if !line.starts_with('#') && !line.trim().is_empty() {
                body.push_str(line);
                body.push('\n');
                count += 1;
            }
        }
    }
    let dest = game.join(PASTA_DO_RUNTIME).join(tabela.arquivo);
    if count == 0 {
        tirar_tabela_gerada(&dest, &tabela.marca_do_emissor())?;
        return Ok(0);
    }
    gravar_tabela(&dest, format!("{}{body}", tabela.cabecalho()).as_bytes())?;
    Ok(count)
}

/// Os `.xl` (`.xl` e `.archive.xl`) de `arquivos` (debaixo de `raiz`, a `Mods/`), na ordem dada, com o id de cada um nas tabelas de
/// runtime: o caminho do `.xl` no jogo sem extensão (`Mods/<caminho dentro da raiz>`), único entre mods e dentro de um mod (a travessia
/// desce nas subpastas). O nome do `.xl` sozinho colide (`ModA/resource.xl` e `ModB/resource.xl`), e o runtime agrupa por esse id: a
/// deleção de setor recusada de um mod cancelaria a do outro, e a reserva da localização (o 1º idioma do `.xl`) é escolhida por id. O id
/// começa com `Mods/`, então nunca é lido como cabeçalho (`#`) por `gravar_tabela_de_runtime` nem pelos leitores do runtime.
pub fn xl_com_id(raiz: &Path, arquivos: &[PathBuf]) -> Vec<(String, PathBuf)> {
    arquivos
        .iter()
        .filter(|p| p.to_string_lossy().to_ascii_lowercase().ends_with(".xl"))
        .map(|p| {
            let rel = p.strip_prefix(raiz).unwrap_or(p).with_extension("");
            (format!("{PASTA_MODS}/{}", rel.to_string_lossy()), p.clone())
        })
        .collect()
}

/// O que `gravar_tabelas_de_runtime` gravou em `red4ext/`: as contagens de cada tabela, os `.xl` que ficaram fora de todas e as partes de
/// `.xl` que o runtime não aplica.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabelasDosXl {
    pub xl_lidos: usize,
    /// `.xl` ilegível ou que não parseou, na ordem de entrada; o motivo, com o caminho do `.xl` no jogo, foi para os avisos.
    pub xl_fora: Vec<PathBuf>,
    /// Cada entrada de um `.xl` que entrou e que o runtime não aplica (`tabelas_de_runtime::entradas_sem_efeito`), com o `.xl` dela, na ordem
    /// de entrada. É limite do BWMS, não defeito do `.xl` (o mesmo arquivo funciona no Windows e o resto dele entrou), e fica fora dos avisos
    /// para o relatório da `Mods/` não mandar corrigir o arquivo.
    pub sem_efeito_no_bwms: Vec<(PathBuf, String)>,
    /// Cada seção de um `.xl` que entrou e que o runtime não aplica (`tabelas_de_runtime::secoes_sem_efeito`), com o `.xl` dela, na ordem de
    /// entrada.
    pub secoes_sem_efeito: Vec<(PathBuf, String)>,
    /// Linhas de par `resource.link`/`resource.copy` em `bwms-reslink.txt`, já sem cadeia e sem ciclo. Ver `write_reslink_table`.
    pub reslink_pairs: usize,
    /// Linhas de `bwms-streaming.txt` (`streaming.sectors`, ArchiveXL `#49`).
    pub streaming_lines: usize,
    /// Linhas de `bwms-questphase.txt` (`quest.phases`, ArchiveXL `#35`).
    pub questphase_lines: usize,
    /// Linhas de `bwms-journal.txt` (`journal`, ArchiveXL `#44`).
    pub journal_lines: usize,
    /// Linhas de `bwms-resource-patch.txt` (`resource.patch` de todos os `.xl`, ArchiveXL `#17`). Ver `write_tabela_de_todos_os_xl`.
    pub resource_patch_lines: usize,
    /// Linhas de `bwms-resfixes.txt` (`resource.fix`, ArchiveXL `#51` 51i).
    pub resfix_lines: usize,
    /// Linhas de `bwms-resscopes.txt` (`resource.scope` de todos os `.xl`, ArchiveXL `#20` 20e-2). Ver `write_tabela_de_todos_os_xl`.
    pub resscope_lines: usize,
    /// Linhas de `bwms-customizations.txt` (`customizations`, ArchiveXL `#47`).
    pub customization_lines: usize,
    /// `resource.link` e `resource.scope` que caem num ciclo, uma mensagem por caminho (`PlanoDeRedirecionamento.cyclic_links` e o 2º valor
    /// de `tabelas_de_runtime::resource_scope_listas`): o link sai da tabela, como o `Link "…" is cyclic` da referência, e o escopo fica só
    /// com os alvos fora do ciclo.
    pub ciclos: Vec<String>,
    /// O que o jogo aplica dos `.xl` da `Mods/` (`xl_no_jogo::planejar`).
    pub no_jogo: crate::instalacao::xl_no_jogo::PlanoDoJogo,
    /// Os `.json` de `no_jogo` estavam no disco com o conteúdo do plano (`xl_no_jogo::no_disco`) e as seções deles saíram das tabelas. Sem
    /// isso, o BWMS aplica tudo.
    pub no_jogo_entregue: bool,
}

/// As 9 tabelas de runtime de `red4ext/` a partir de `xl_paths` (`(id, .xl)`, na ordem de precedência: no mesmo recurso vence o `.xl` que
/// vem depois). Os `.xl` de `localizacao_no_jogo` (por id) saem da tabela de localização: o jogo aplica o `onscreens` deles.
fn gravar_tabelas_dos_xl(
    game: &Path,
    xl_paths: Vec<(String, PathBuf)>,
    localizacao_no_jogo: &BTreeSet<String>,
    avisos: &mut Vec<String>,
) -> std::io::Result<TabelasDosXl> {
    // Cada `.xl` é lido e parseado uma vez; `.xl` ilegível ou que não parseia fica de fora de todas as tabelas (mod malformado não
    // trava o boot). O motivo vai para `avisos`, como cada entrada que o parse deixou de fora (`XlFile::issues`), com o caminho do `.xl`
    // no jogo, que é o que o usuário abre para corrigir. O que o runtime não aplica não tem o que corrigir no `.xl`: vai para
    // `sem_efeito_no_bwms` e `secoes_sem_efeito`.
    let mut xls: Vec<(String, XlFile)> = Vec::with_capacity(xl_paths.len());
    let mut xl_fora: Vec<PathBuf> = Vec::new();
    let mut sem_efeito_no_bwms: Vec<(PathBuf, String)> = Vec::new();
    let mut secoes_sem_efeito: Vec<(PathBuf, String)> = Vec::new();
    for (id, p) in xl_paths {
        let arquivo = p.strip_prefix(game).unwrap_or(&p).display().to_string();
        let texto = match std::fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => {
                avisos.push(format!(
                    "{arquivo}: .xl ilegível ({e}), fora das tabelas do runtime; confira se é texto UTF-8 com permissão de leitura e {DE_NOVO}"
                ));
                xl_fora.push(p);
                continue;
            }
        };
        match crate::instalacao::xl::parse_xl(&texto) {
            Ok(xl) => {
                avisos.extend(xl.issues.iter().map(|i| format!("{arquivo}: {i}")));
                sem_efeito_no_bwms.extend(crate::instalacao::tabelas_de_runtime::entradas_sem_efeito(&xl).into_iter().map(|a| (p.clone(), a)));
                secoes_sem_efeito.extend(crate::instalacao::tabelas_de_runtime::secoes_sem_efeito(&xl).into_iter().map(|a| (p.clone(), a)));
                xls.push((id, xl));
            }
            Err(e) => {
                avisos.push(format!("{arquivo}: .xl não parseou ({e}), fora das tabelas do runtime; corrija o YAML do arquivo e {DE_NOVO}"));
                xl_fora.push(p);
            }
        }
    }
    let xl_lidos = xls.len();
    let xls_lidos: Vec<&XlFile> = xls.iter().map(|(_, xl)| xl).collect();
    // As 9 tabelas são tentadas mesmo depois de uma falha: parar na primeira deixaria as seguintes com o conteúdo da abertura anterior,
    // que o runtime carrega. As falhas viram um `Err` só, no fim.
    let mut falhas: Vec<std::io::Error> = Vec::new();
    let mut tabela = |gravada: std::io::Result<usize>| {
        gravada.unwrap_or_else(|e| {
            falhas.push(e);
            0
        })
    };
    // Os escopos do `resource.scope` valem entre mods (a referência junta os de todas as configs): saem de todos os `.xl` primeiro, porque o
    // plano de link/copy expande o pedido e a cópia por eles, e o `resource.patch` de cada `.xl` expande os alvos.
    let (scopes, escopos_em_ciclo) = crate::instalacao::tabelas_de_runtime::resource_scope_listas(&xls_lidos);
    // `resource.link`/`resource.copy` valem entre mods: um plano só, como a referência junta as configs antes de resolver cadeia e ciclo.
    let plano = crate::instalacao::tabelas_de_runtime::build_apply_plan_combinado(&xls_lidos, &scopes);
    avisos.extend(plano.avisos.iter().cloned());
    let reslink_pairs = tabela(write_reslink_table(game, &plano));
    let streaming_lines = tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::STREAMING, &crate::instalacao::tabelas_de_runtime::emit_streaming_table));
    let questphase_lines = tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::QUESTPHASE, &crate::instalacao::tabelas_de_runtime::emit_questphase_table));
    let journal_lines = tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::JOURNAL, &crate::instalacao::tabelas_de_runtime::emit_journal_table));
    // O `onscreens` que o jogo aplica pelo `.json` (`xl_no_jogo`) sai da tabela: uma seção vai por um caminho só.
    tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::LOCALIZACAO, &|xl, id| {
        if localizacao_no_jogo.contains(id) {
            String::new()
        } else {
            crate::instalacao::tabelas_de_runtime::emit_localization_table(xl, id)
        }
    }));
    let ciclos: Vec<String> = plano
        .cyclic_links
        .iter()
        .map(|c| format!("resource.link {c}: cai num ciclo e fica fora de bwms-reslink.txt"))
        .chain(escopos_em_ciclo.iter().map(|e| format!("resource.scope {e}: passa por um ciclo e fica só com os alvos fora dele")))
        .collect();
    // `resource.patch` vale entre mods: o emissor vê todos os `.xl` e já grava a tabela resolvida (alvo que é patch fora, par único, props do
    // último `.xl` do patch); o runtime só lê.
    let resource_patch_lines = tabela(write_tabela_de_todos_os_xl(
        game,
        &tabelas::RESOURCE_PATCH,
        &tabelas::RESOURCE_PATCH.marca_do_emissor(),
        &crate::instalacao::tabelas_de_runtime::emit_resource_patch_table(&xls, &scopes),
    ));
    // `resource.fix`: o runtime lê as linhas num leitor só (`pipeline::resfixes_linhas`; o mesmo antigo fica com o novo da última linha) e troca os
    // nomes de material no `PostLoad` do mesh (`names`) e o recurso de aparência no `FixCustomizationOptions` (`paths`).
    let resfix_lines = tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::RESFIXES, &|xl, _| crate::instalacao::tabelas_de_runtime::emit_resfixes(xl)));
    // `resource.scope` em runtime: o `InScope` do `FixCustomizationAppearance` (`register::es20_no_escopo`).
    let resscope_lines = tabela(write_tabela_de_todos_os_xl(
        game,
        &tabelas::RESSCOPES,
        &format!("# {}", tabelas::RESSCOPES.nome()),
        &crate::instalacao::tabelas_de_runtime::emit_resscopes(&scopes),
    ));
    // `customizations`: o runtime pede cada recurso e mescla nos recursos base no `Initialize` do criador (`register::cust_tabela`).
    let customization_lines = tabela(gravar_tabela_de_runtime(game, &xls, &tabelas::CUSTOMIZATIONS, &crate::instalacao::tabelas_de_runtime::emit_customization_table));
    if let Some(primeira) = falhas.first() {
        let lista: Vec<String> = falhas.iter().map(|e| e.to_string()).collect();
        return Err(std::io::Error::new(
            primeira.kind(),
            format!(
                "não consegui atualizar {} tabela(s) em red4ext/ ({}). Confira a permissão de escrita da pasta red4ext/ e {REPETIR}; até lá o jogo carrega as tabelas antigas.",
                falhas.len(),
                lista.join("; ")
            ),
        ));
    }
    Ok(TabelasDosXl {
        xl_lidos,
        xl_fora,
        sem_efeito_no_bwms,
        secoes_sem_efeito,
        reslink_pairs,
        streaming_lines,
        questphase_lines,
        journal_lines,
        resource_patch_lines,
        resfix_lines,
        resscope_lines,
        customization_lines,
        ciclos,
        ..TabelasDosXl::default()
    })
}

/// A pasta do jogo em que o usuário joga o mod como vem no zip (`cp77-console`, `mods_do_usuario::pasta_mods`). É também o começo do id dos
/// `.xl` dela nas tabelas (`xl_com_id`): o id fica o caminho do `.xl` no jogo.
pub const PASTA_MODS: &str = "Mods";

/// Todo arquivo regular debaixo de `raiz`, em qualquer profundidade, em ordem de caminho (bytes, a ordem ASCII do Windows: quando dois mods
/// trazem o mesmo recurso, o primeiro vence, `proofs/20260928-f4-ordem-*`). Nome que começa com `.` fica de fora, link simbólico não é
/// seguido, e pasta que não se lê fica de fora sem impedir as outras (a `Mods/` é do usuário, e o jogo abre do mesmo jeito). É a travessia
/// da `Mods/` para as tabelas e para o runtime (`.archive`, tweak, `.reds`, relatório).
pub fn arquivos_da_pasta_mods(raiz: &Path) -> Vec<PathBuf> {
    let mut achados = Vec::new();
    let mut pendentes = vec![raiz.to_path_buf()];
    while let Some(dir) = pendentes.pop() {
        let Ok(entradas) = std::fs::read_dir(&dir) else { continue };
        for e in entradas.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(tipo) = e.file_type() else { continue };
            if tipo.is_dir() {
                pendentes.push(e.path());
            } else if tipo.is_file() {
                achados.push(e.path());
            }
        }
    }
    achados.sort_by(|a, b| a.as_os_str().as_encoded_bytes().cmp(b.as_os_str().as_encoded_bytes()));
    achados
}

/// A `Mods/` de um jogo lida uma vez (`ler_pasta_mods`): o runtime varre os arquivos para o relatório e passa a mesma leitura às tabelas
/// (`gravar_tabelas_de_runtime`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PastaMods {
    /// `<jogo>/Mods`.
    pub raiz: PathBuf,
    /// `arquivos_da_pasta_mods(raiz)`.
    pub arquivos: Vec<PathBuf>,
}

/// A `Mods/` de `game` com os arquivos dela; sem a pasta, nenhum arquivo.
pub fn ler_pasta_mods(game: &Path) -> PastaMods {
    let raiz = game.join(PASTA_MODS);
    let arquivos = arquivos_da_pasta_mods(&raiz);
    PastaMods { raiz, arquivos }
}

/// Os `.xl` da `Mods/` com o id deles, na ordem em que entram nas tabelas: de trás para frente, porque nelas vence o `.xl` que vem depois, e
/// entre mods da `Mods/` vence o 1º em ordem de caminho, como no `.archive`.
fn xl_da_pasta_mods(pasta: &PastaMods) -> Vec<(String, PathBuf)> {
    let mut xls = xl_com_id(&pasta.raiz, &pasta.arquivos);
    xls.reverse();
    xls
}

/// Regrava as 9 tabelas de runtime de `red4ext/` com os `.xl` da `Mods/` de agora (`pasta`, de `ler_pasta_mods`): a cada abertura do jogo,
/// porque o usuário muda a `Mods/` com o jogo fechado. Sem `.xl`, a tabela gerada sai e a posta à mão fica (`tirar_tabela_gerada`); tabela com
/// o mesmo conteúdo não é regravada (`gravar_se_mudou`). A seção que o jogo aplica pelo `.json` (`plano_do_boot`) sai da tabela só com o
/// `.json` dela no disco. `Err` (tabela que não gravou) deixa as que não foram regravadas como estavam. Devolve o que foi gravado e os avisos
/// de cada `.xl`, com o caminho dele no jogo; o que o runtime ainda não aplica vem à parte, em `TabelasDosXl::sem_efeito_no_bwms`.
pub fn gravar_tabelas_de_runtime(game: &Path, pasta: &PastaMods) -> std::io::Result<(TabelasDosXl, Vec<String>)> {
    let xl_paths = xl_da_pasta_mods(pasta);
    let no_jogo = plano_do_boot(game, pasta);
    let no_jogo_entregue = crate::instalacao::xl_no_jogo::no_disco(game, &no_jogo);
    // A config que leva só `factories` não tira o `.xl` da tabela de localização.
    let localizacao_no_jogo: BTreeSet<String> = if no_jogo_entregue {
        xl_paths
            .iter()
            .filter(|(_, p)| no_jogo.configs.iter().any(|c| &c.xl == p && c.leva("localization.onscreens")))
            .map(|(id, _)| id.clone())
            .collect()
    } else {
        BTreeSet::new()
    };
    let mut avisos: Vec<String> = Vec::new();
    let mut t = gravar_tabelas_dos_xl(game, xl_paths, &localizacao_no_jogo, &mut avisos)?;
    t.no_jogo = no_jogo;
    t.no_jogo_entregue = no_jogo_entregue;
    Ok((t, avisos))
}

/// O plano do que o jogo aplica (`xl_no_jogo::planejar`) com os `.xl` da `Mods/` (`pasta`, em ordem de caminho). O `.xl` ilegível ou que
/// não parseia fica fora, como nas tabelas. Os concorrentes (`EnabledMods.json` da home, outras pastas de `archive/Mac/mod`) só são lidos
/// quando a `Mods/` tem textos e o jogo tem `archive/Mac/`: sem isso o plano não depende deles. O runtime põe os `.json` dele em
/// `archive/Mac/mod` antes de regravar as tabelas (`gravar_tabelas_de_runtime`), que recalculam o mesmo plano e conferem o disco.
pub fn plano_do_boot(game: &Path, pasta: &PastaMods) -> crate::instalacao::xl_no_jogo::PlanoDoJogo {
    use crate::instalacao::xl_no_jogo::{concorrentes, onscreens_do_bwms, planejar, Concorrentes};
    let ler = |p: &Path| std::fs::read_to_string(p).ok().and_then(|t| crate::instalacao::xl::parse_xl(&t).ok());
    let mods: Vec<(PathBuf, XlFile)> = xl_com_id(&pasta.raiz, &pasta.arquivos).into_iter().filter_map(|(_, p)| ler(&p).map(|xl| (p, xl))).collect();
    let pasta_do_jogo = game.join("archive/Mac").is_dir();
    let fora = if pasta_do_jogo && mods.iter().any(|(_, xl)| !onscreens_do_bwms(xl).is_empty()) {
        concorrentes(game, home().as_deref())
    } else {
        Concorrentes { modio: Ok(Vec::new()), outras_pastas: Vec::new() }
    };
    let mods_ref: Vec<(PathBuf, &XlFile)> = mods.iter().map(|(p, xl)| (p.clone(), xl)).collect();
    planejar(&mods_ref, &fora, pasta_do_jogo)
}

/// A home do usuário, onde mora o `EnabledMods.json` do jogo (`xl_no_jogo::ENABLED_MODS`). Nos testes, nenhuma: o plano não pode depender
/// dos mods do mod.io da máquina que roda o teste (a leitura tem teste próprio em `xl_no_jogo`).
#[cfg(not(test))]
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
fn home() -> Option<PathBuf> {
    None
}

/// Escreve `<red4ext>/<tabela.arquivo>` com o `conteudo` de um emissor que vê todos os `.xl` de uma vez: `bwms-resscopes.txt`
/// (`tabelas_de_runtime::emit_resscopes`) e `bwms-resource-patch.txt` (`tabelas_de_runtime::emit_resource_patch_table`). Devolve as linhas que não são cabeçalho; sem
/// nenhuma, tira a tabela só se ela começar com `marca` (`tirar_tabela_gerada`). `Err` como em `write_reslink_table`.
fn write_tabela_de_todos_os_xl(game: &Path, tabela: &Tabela, marca: &str, conteudo: &str) -> std::io::Result<usize> {
    let linhas = conteudo.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).count();
    let dest = game.join(PASTA_DO_RUNTIME).join(tabela.arquivo);
    if linhas == 0 {
        tirar_tabela_gerada(&dest, marca)?;
        return Ok(0);
    }
    gravar_tabela(&dest, conteudo.as_bytes())?;
    Ok(linhas)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jogo de teste vazio em `temp_dir`; o teste o apaga por caminho no fim.
    fn jogo(nome: &str) -> PathBuf {
        let g = std::env::temp_dir().join(format!("bwms-tabelas-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        g
    }

    fn escrever_texto(p: &Path, texto: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, texto).unwrap();
    }

    /// Um `.xl` em `<jogo>/Mods/<rel>`.
    fn xl(g: &Path, rel: &str, texto: &str) {
        escrever_texto(&g.join(PASTA_MODS).join(rel), texto);
    }

    /// A gravação de uma abertura do jogo.
    fn boot(g: &Path) -> std::io::Result<(TabelasDosXl, Vec<String>)> {
        gravar_tabelas_de_runtime(g, &ler_pasta_mods(g))
    }

    /// Caminho que não existe, nem como link pendente.
    fn ausente(p: &Path) -> bool {
        matches!(std::fs::symlink_metadata(p), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
    }

    /// Conteúdo de `red4ext/` em ordem de nome, para comparar byte a byte.
    fn tabelas(g: &Path) -> String {
        let mut nomes: Vec<_> = std::fs::read_dir(g.join("red4ext")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        nomes.sort();
        nomes.iter().map(|n| format!("== {n} ==\n{}", std::fs::read_to_string(g.join("red4ext").join(n)).unwrap())).collect()
    }

    /// Conteúdo de `red4ext/bwms-reslink.txt` sem o cabeçalho.
    fn linhas_do_reslink(g: &Path) -> Vec<String> {
        let t = std::fs::read_to_string(g.join("red4ext/bwms-reslink.txt")).unwrap_or_default();
        t.lines().filter(|l| !l.starts_with('#')).map(String::from).collect()
    }

    /// Linhas da `bwms-localizacao.txt` sem o cabeçalho.
    fn linhas_da_localizacao(g: &Path) -> Vec<String> {
        let t = std::fs::read_to_string(g.join("red4ext/bwms-localizacao.txt")).unwrap_or_default();
        t.lines().filter(|l| !l.starts_with('#')).map(String::from).collect()
    }

    /// Permissão de pasta: o dono do processo não pode criar nem apagar dentro de 0o555 (o teste não roda como root).
    fn pasta_so_leitura(dir: &Path, so_leitura: bool) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(if so_leitura { 0o555 } else { 0o755 })).unwrap();
    }

    #[test]
    fn xl_com_resource_link_gera_reslink_txt() {
        let g = jogo("reslink");
        xl(&g, "LinkMod/resource.xl", "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\b.mesh\n");
        let (t, _) = boot(&g).unwrap();
        assert_eq!((t.xl_lidos, t.reslink_pairs), (1, 1), "1 resource.link no .xl -> 1 par no reslink");
        let content = std::fs::read_to_string(g.join("red4ext/bwms-reslink.txt")).unwrap();
        assert!(content.contains("base\\fake\\a.mesh") && content.contains("base\\real\\b.mesh"), "conteúdo: {content}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// ArchiveXL `#44`/`#35`/`#49`/`#17`: o boot grava as tabelas que saem de emissor por `.xl` e, com o mod fora da `Mods/`, apaga as que ele
    /// gerou; tabela posta à mão fica.
    #[test]
    fn xl_com_journal_fase_e_setor_gera_tabelas_de_runtime() {
        let g = jogo("emissores");
        xl(
            &g,
            "QuestMod/quest.xl",
            "resource:\n  patch:\n    mod\\p.ent: base\\alvo.ent\njournal:\n  - mod\\quest\\a.journal\nquest:\n  phases:\n    - path: mod\\fase.questphase\n      parent: base\\quest\\pai.questphase\nstreaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - index: 1\n          type: worldStaticMeshNode\n",
        );
        let (t, _) = boot(&g).unwrap();
        assert_eq!((t.journal_lines, t.questphase_lines, t.streaming_lines, t.resource_patch_lines), (1, 1, 1, 1));
        let red4 = g.join("red4ext");
        let patch = std::fs::read_to_string(red4.join("bwms-resource-patch.txt")).unwrap();
        // Linha inteira: um `contains` casaria `quest | …` dentro de `Mods/QuestMod/quest | …` e não conferiria o id.
        assert!(patch.lines().any(|l| l == "Mods/QuestMod/quest | mod\\p.ent | base\\alvo.ent | -"), "{patch}");
        let journal = std::fs::read_to_string(red4.join("bwms-journal.txt")).unwrap();
        assert!(journal.starts_with("# bwms-journal (gerado de .xl)"), "{journal}");
        assert!(journal.lines().any(|l| l == "Mods/QuestMod/quest | mod\\quest\\a.journal"), "{journal}");
        let fase = std::fs::read_to_string(red4.join("bwms-questphase.txt")).unwrap();
        assert!(fase.lines().any(|l| l == "Mods/QuestMod/quest | base\\quest\\pai.questphase | mod\\fase.questphase |  | - |  | - | 0"), "{fase}");
        let setor = std::fs::read_to_string(red4.join("bwms-streaming.txt")).unwrap();
        assert!(setor.lines().any(|l| l == "Mods/QuestMod/quest | s.streamingsector | 5 | 1 | worldStaticMeshNode"), "{setor}");

        // Tabela posta à mão, sem o cabeçalho do emissor, não sai quando nenhum `.xl` gera linha.
        std::fs::write(red4.join("bwms-questphase.txt"), "teste | posto à mão\n").unwrap();
        std::fs::remove_dir_all(g.join("Mods/QuestMod")).unwrap();
        let (t, _) = boot(&g).unwrap();
        assert_eq!((t.journal_lines, t.questphase_lines, t.streaming_lines, t.resource_patch_lines), (0, 0, 0, 0));
        assert!(!red4.join("bwms-resource-patch.txt").exists(), "tabela gerada de mod que saiu da Mods tem de sair");
        assert!(!red4.join("bwms-journal.txt").exists(), "tabela gerada de mod que saiu da Mods tem de sair");
        assert!(!red4.join("bwms-streaming.txt").exists(), "tabela gerada de mod que saiu da Mods tem de sair");
        assert!(red4.join("bwms-questphase.txt").exists(), "tabela posta à mão fica");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// `resource.patch` entre mods sai resolvido: o alvo do ModA que é patch do ModB sai, e o par que os dois pedem fica uma vez, com o mod do
    /// 1º `.xl` nas tabelas (o ModB: entre mods da `Mods/` vence o 1º em ordem de caminho, que entra por último) e as props do último (ModA).
    #[test]
    fn resource_patch_de_dois_mods_sai_resolvido() {
        let g = jogo("resource-patch");
        xl(&g, "ModA/a.xl", "resource:\n  patch:\n    mod\\p.ent:\n      props: [ components ]\n      targets: [ base\\x.ent, modb\\q.ent ]\n");
        xl(&g, "ModB/b.xl", "resource:\n  patch:\n    modb\\q.ent: base\\y.ent\n    mod\\p.ent:\n      props: [ appearances ]\n      targets: [ base\\x.ent ]\n");
        let (t, _) = boot(&g).unwrap();
        assert_eq!(t.resource_patch_lines, 2);
        let patch = std::fs::read_to_string(g.join("red4ext/bwms-resource-patch.txt")).unwrap();
        let linhas: Vec<&str> = patch.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(linhas, vec!["Mods/ModB/b | modb\\q.ent | base\\y.ent | -", "Mods/ModB/b | mod\\p.ent | base\\x.ent | 0x1"], "{patch}");
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn tabelas_de_varios_xl_saem_iguais_byte_a_byte() {
        // golden das 9 tabelas com 2 `.xl` válidos (escopo de um mod usado no patch do outro) e 1 ilegível, que fica de fora, não
        // conta e vai para os avisos. Cada tabela recebe linhas de um mod só, então o golden não depende da ordem entre os mods.
        let g = jogo("golden");
        xl(
            &g,
            "ModA/alfa.xl",
            "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\a.mesh\n  copy:\n    base\\orig.mesh: mod\\copia.mesh\n  scope:\n    mod\\grupo.ent:\n      - base\\x.ent\n      - base\\y.ent\n  fix:\n    my.mesh:\n      names:\n        old_mat: new_mat\nfactories:\n  - moda\\f.csv\njournal:\n  - moda\\q.journal\nlocalization:\n  onscreens:\n    en-us: moda\\\\loc\\\\en-us.json\ncustomizations:\n  female: moda\\f.inkcharcustomization\n",
        );
        xl(
            &g,
            "ModB/beta.archive.xl",
            "resource:\n  patch:\n    modb\\p.ent:\n      props: [ components ]\n      targets: [ mod\\grupo.ent, base\\z.ent ]\nquest:\n  phases:\n    - path: modb\\fase.questphase\n      parent: base\\quest\\pai.questphase\nstreaming:\n  sectors:\n    - path: s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - index: 1\n          type: worldStaticMeshNode\n",
        );
        let c = g.join("Mods/ModC/quebrado.xl");
        std::fs::create_dir_all(c.parent().unwrap()).unwrap();
        std::fs::write(&c, b"\xff\xfe resource").unwrap();

        let (r, avisos) = boot(&g).unwrap();
        assert_eq!(r.xl_lidos, 2);
        assert_eq!(avisos.len(), 1, "{avisos:?}");
        assert!(avisos[0].starts_with("Mods/ModC/quebrado.xl: .xl ilegível ("), "{}", avisos[0]);
        assert!(avisos[0].ends_with("; confira se é texto UTF-8 com permissão de leitura e abra o jogo de novo"), "{}", avisos[0]);
        let obtido = tabelas(&g);
        let esperado = "== bwms-customizations.txt ==\n# bwms-customizations (gerado de .xl). Formato: <male|female> | <caminho do recurso>\nfemale | moda\\f.inkcharcustomization\n\
== bwms-journal.txt ==\n# bwms-journal (gerado de .xl). Formato: <mod> | <caminho do .journal>\nMods/ModA/alfa | moda\\q.journal\n\
== bwms-localizacao.txt ==\n# bwms-localizacao (gerado de .xl). Formato: <mod> | <tipo> | <idioma> | <caminho do recurso>\nMods/ModA/alfa | onscreens | en-us | moda\\\\loc\\\\en-us.json\n\
== bwms-questphase.txt ==\n# bwms-questphase (gerado de .xl). Formato: <mod> | <fase pai> | <fase do mod> | <nó de entrada> | <socket de entrada> | <nó de saída> | <socket de saída> | <intercept>\nMods/ModB/beta.archive | base\\quest\\pai.questphase | modb\\fase.questphase |  | - |  | - | 0\n\
== bwms-resfixes.txt ==\n# bwms-resfixes (gerado de .xl). Formato: <resource> | names|paths|context | <old ou parâmetro> | <new ou valor>\nmy.mesh | names | old_mat | new_mat\n\
== bwms-reslink.txt ==\n# bwms-reslink (gerado automaticamente pelo install — 2 par(es))\nbase\\fake\\a.mesh | base\\real\\a.mesh\nmod\\copia.mesh | base\\orig.mesh | copy\n\
== bwms-resource-patch.txt ==\n# bwms-resource-patch (gerado de .xl). Formato: <mod> | <caminho do patch> | <caminho do alvo> | <props em máscara hex; - = todas>\nMods/ModB/beta.archive | modb\\p.ent | base\\x.ent | 0x1\nMods/ModB/beta.archive | modb\\p.ent | base\\y.ent | 0x1\nMods/ModB/beta.archive | modb\\p.ent | base\\z.ent | 0x1\n\
== bwms-resscopes.txt ==\n# bwms-resscopes (gerado de .xl). Formato: <hash do caminho do escopo> | <caminho do alvo>\n0x11000209e2595c59 | base\\x.ent\n0x11000209e2595c59 | base\\y.ent\n\
== bwms-streaming.txt ==\n# bwms-streaming (gerado de .xl). Formato: <mod> | <setor> | <expectedNodes> | <nodeIndex> | <tipo>\nMods/ModB/beta.archive | s.streamingsector | 5 | 1 | worldStaticMeshNode\n";
        assert_eq!(obtido, esperado, "\n{obtido}");

        // Toda linha gravada se lê pelo formato da tabela (`tabelas`), sem recusa, e a contagem devolvida é a das linhas lidas.
        fn lidas<'a, T: 'a>(texto: &'a str, ler: fn(&[&'a str]) -> Option<T>) -> (usize, Vec<&'a str>) {
            let (mut n, mut recusadas) = (0, Vec::new());
            for l in tabelas::ler(texto, ler) {
                match l {
                    Ok(_) => n += 1,
                    Err(l) => recusadas.push(l),
                }
            }
            (n, recusadas)
        }
        let t = |tabela: Tabela| std::fs::read_to_string(g.join(PASTA_DO_RUNTIME).join(tabela.arquivo)).unwrap();
        assert_eq!(lidas(&t(tabelas::RESLINK), tabelas::Reslink::ler), (r.reslink_pairs, vec![]));
        assert_eq!(lidas(&t(tabelas::STREAMING), tabelas::Streaming::ler), (r.streaming_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::QUESTPHASE), tabelas::Questphase::ler), (r.questphase_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::JOURNAL), tabelas::Journal::ler), (r.journal_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::LOCALIZACAO), tabelas::Localizacao::ler), (1, vec![]));
        assert_eq!(lidas(&t(tabelas::RESOURCE_PATCH), tabelas::ResourcePatch::ler), (r.resource_patch_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::RESFIXES), tabelas::Resfix::ler), (r.resfix_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::RESSCOPES), tabelas::Resscope::ler), (r.resscope_lines, vec![]));
        assert_eq!(lidas(&t(tabelas::CUSTOMIZATIONS), tabelas::Customizacao::ler), (r.customization_lines, vec![]));
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn xl_que_nao_parseia_e_entrada_invalida_vao_para_avisos_com_o_caminho_do_arquivo() {
        let g = jogo("avisos-do-xl");
        xl(&g, "XlMod/quebrado.xl", "---\nfactories: [ a.csv ]\n");
        xl(&g, "XlMod/sub/escopo.archive.xl", "resource:\n  scope:\n    a.app:\n      x: y\n    b.app: c.app\n");
        let (r, avisos) = boot(&g).unwrap();
        assert_eq!(r.xl_lidos, 1, "só o .xl que parseou conta");
        assert_eq!(r.resscope_lines, 1, "o escopo válido do .xl que parseou continua");
        assert_eq!(avisos.len(), 2, "{avisos:?}");
        // de trás para frente: `sub/escopo` entra antes de `quebrado`
        assert_eq!(
            avisos[0],
            "Mods/XlMod/sub/escopo.archive.xl: resource.scope.a.app: formato inválido (escopo ignorado); use um caminho ou uma lista de caminhos"
        );
        assert!(avisos[1].starts_with("Mods/XlMod/quebrado.xl: .xl não parseou ("), "{}", avisos[1]);
        assert!(avisos[1].ends_with("), fora das tabelas do runtime; corrija o YAML do arquivo e abra o jogo de novo"), "{}", avisos[1]);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O grupo de `localization` que o runtime não aplica vai à parte (`sem_efeito_no_bwms`); sem o grupo no `.xl`, a parte some e a linha
    /// que o runtime aplica é a mesma.
    #[test]
    fn localizacao_que_o_runtime_nao_aplica_sai_a_parte_e_some_sem_o_grupo() {
        let g = jogo("localizacao-sem-efeito");
        let so_onscreens = "localization:\n  onscreens:\n    en-us: leg\\on.json\n";
        xl(&g, "Legendas/leg.xl", &format!("{so_onscreens}  subtitles:\n    pt-br: leg\\sub.json\n"));
        let (t, avisos) = boot(&g).unwrap();
        let limite = "localization.subtitles: não aplicado pelo runtime do BWMS, que só aplica onscreens, e o grupo fica sem efeito no jogo; tire o \
                      grupo subtitles do localization deste .xl para o aviso sumir (o jogo fica igual)";
        assert!(avisos.is_empty(), "{avisos:?}");
        assert_eq!(t.sem_efeito_no_bwms, vec![(g.join("Mods/Legendas/leg.xl"), limite.to_string())]);
        let com = linhas_da_localizacao(&g);
        assert!(com.contains(&"Mods/Legendas/leg | subtitles | pt-br | leg\\sub.json".to_string()), "{com:?}");

        xl(&g, "Legendas/leg.xl", so_onscreens);
        let (t, _) = boot(&g).unwrap();
        assert!(t.sem_efeito_no_bwms.is_empty(), "{:?}", t.sem_efeito_no_bwms);
        let onscreens = |v: &[String]| v.iter().filter(|l| l.contains(" | onscreens | ")).cloned().collect::<Vec<_>>();
        assert_eq!(onscreens(&linhas_da_localizacao(&g)), onscreens(&com));
        assert_eq!(onscreens(&com), vec!["Mods/Legendas/leg | onscreens | en-us | leg\\on.json"]);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// `resource.xl` em 2 mods e em 2 pastas do mesmo mod: o runtime agrupa por id, então cada `.xl` precisa do seu. Pasta de mod que começa
    /// com `#` (ou espaço e `#`) não vira cabeçalho: o id começa com `Mods/`.
    #[test]
    fn xl_de_mesmo_nome_em_mods_ou_subpastas_diferentes_sai_com_ids_diferentes() {
        let g = jogo("ids");
        for (rel, journal) in
            [("ModA/resource.xl", "a\\q.journal"), ("ModB/resource.xl", "b\\q.journal"), ("ModB/sub/resource.xl", "b\\sub.journal"), ("#Hash/x.xl", "h\\q.journal"), (" #Esp/x.xl", "e\\q.journal")]
        {
            xl(&g, rel, &format!("journal:\n  - {journal}\n"));
        }
        let (t, _) = boot(&g).unwrap();
        assert_eq!((t.xl_lidos, t.journal_lines), (5, 5));
        let journal = std::fs::read_to_string(g.join("red4ext/bwms-journal.txt")).unwrap();
        let linhas: Vec<&str> = journal.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(
            linhas,
            [
                "Mods/ModB/sub/resource | b\\sub.journal",
                "Mods/ModB/resource | b\\q.journal",
                "Mods/ModA/resource | a\\q.journal",
                "Mods/#Hash/x | h\\q.journal",
                "Mods/ #Esp/x | e\\q.journal",
            ],
            "\n{journal}"
        );
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Hard link aponta para o inode antigo: escrita no lugar o mudaria, o rename deixa o alias com o conteúdo de antes.
    #[test]
    fn tabela_e_trocada_por_rename_sem_sobrar_temporario() {
        let g = jogo("atomico");
        xl(&g, "LinkMod/resource.xl", "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\a.mesh\n");
        boot(&g).unwrap();
        let reslink = g.join("red4ext/bwms-reslink.txt");
        let antes = std::fs::read(&reslink).unwrap();
        std::fs::hard_link(&reslink, g.join("alias-reslink")).unwrap();
        xl(&g, "LinkMod/resource.xl", "resource:\n  link:\n    base\\fake\\b.mesh: base\\real\\b.mesh\n");
        boot(&g).unwrap();
        assert_ne!(std::fs::read(&reslink).unwrap(), antes);
        assert_eq!(std::fs::read(g.join("alias-reslink")).unwrap(), antes, "tabela foi truncada no lugar");
        let sobra: Vec<_> = std::fs::read_dir(g.join("red4ext")).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().ends_with(".tmp")).collect();
        assert!(sobra.is_empty(), "{sobra:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Sem `.xl` que gere linha, a tabela gerada sai; a posta à mão (sem cabeçalho) e a saída do `bwms xl <f> reslink` (cabeçalho do emissor)
    /// ficam, também sem `.xl` nenhum na `Mods/`.
    #[test]
    fn tabela_gerada_sai_sem_linha_e_a_posta_a_mao_fica() {
        let g = jogo("tabela-zero");
        xl(&g, "LinkMod/resource.xl", "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\b.mesh\n");
        let red4 = g.join("red4ext");
        let (t, _) = boot(&g).unwrap();
        assert_eq!(t.reslink_pairs, 1);
        assert!(red4.join("bwms-reslink.txt").exists());

        std::fs::remove_dir_all(g.join("Mods/LinkMod")).unwrap();
        let (t, _) = boot(&g).unwrap();
        assert_eq!(t.reslink_pairs, 0);
        assert!(ausente(&red4.join("bwms-reslink.txt")), "par de mod que saiu continuaria valendo no boot");

        for mao in ["x | y\n", "# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido> [| copy]\nx | y\n"] {
            std::fs::write(red4.join("bwms-reslink.txt"), mao).unwrap();
            boot(&g).unwrap();
            assert_eq!(std::fs::read_to_string(red4.join("bwms-reslink.txt")).unwrap(), mao);
        }
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Entre dois mods da `Mods/` vence o 1º em ordem de caminho, como no `.archive`; o `.xl` que não parseia fica fora, com o aviso que manda
    /// abrir o jogo de novo.
    #[test]
    fn na_pasta_mods_vence_o_primeiro_e_o_que_nao_parseia_fica_fora() {
        let g = jogo("ordem");
        xl(&g, "A/archive/pc/mod/a.archive.xl", "resource:\n  link:\n    base\\a.ent: moda\\a.ent\n");
        xl(&g, "B/b.xl", "resource:\n  link:\n    base\\a.ent: modb\\a.ent\n    base\\b.ent: modb\\b.ent\n");
        xl(&g, "Y/y.xl", "resource: [\n");
        escrever_texto(&g.join("Mods/X/leia.txt"), "x");
        let pasta = ler_pasta_mods(&g);
        let ids: Vec<String> = xl_da_pasta_mods(&pasta).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["Mods/Y/y", "Mods/B/b", "Mods/A/archive/pc/mod/a.archive"], "de trás para frente: nas tabelas vence o que vem depois");

        let (t, avisos) = gravar_tabelas_de_runtime(&g, &pasta).unwrap();
        assert_eq!(linhas_do_reslink(&g), vec!["base\\a.ent | moda\\a.ent", "base\\b.ent | modb\\b.ent"]);
        assert_eq!((t.xl_lidos, t.xl_fora.clone()), (2, vec![g.join("Mods/Y/y.xl")]));
        assert!(
            avisos.iter().any(|a| a.starts_with("Mods/Y/y.xl: .xl não parseou (") && a.ends_with("corrija o YAML do arquivo e abra o jogo de novo")),
            "{avisos:?}"
        );
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O que o runtime não aplica sai à parte dos avisos do parse (`XlFile::issues`), com o `.xl` dele: o relatório da `Mods/` não manda
    /// corrigir o arquivo por um limite do BWMS. As entradas (o grupo de `localization`) vão a
    /// `sem_efeito_no_bwms`; a seção sem tabela (`animations`) vai a `secoes_sem_efeito`.
    #[test]
    fn o_que_o_runtime_nao_aplica_sai_a_parte_dos_avisos_do_parse() {
        let g = jogo("sem-efeito");
        let caminho = g.join("Mods/Legendas/leg.xl");
        xl(
            &g,
            "Legendas/leg.xl",
            "localization:\n  onscreens:\n    en-us: leg\\on.json\n  subtitles:\n    pt-br: leg\\sub.json\nstreaming:\n  sectors:\n    - path: s.streamingsector\n\
             resource:\n  fix:\n    a.mesh:\n      names:\n        m1: m2\n      context:\n        p: v\n\
             animations:\n  - entity: player_ma_tpp\n    set: leg\\anims.anims\n",
        );
        let limite = "localization.subtitles: não aplicado pelo runtime do BWMS, que só aplica onscreens, e o grupo fica sem efeito no jogo; tire o \
                      grupo subtitles do localization deste .xl para o aviso sumir (o jogo fica igual)";
        let secao = "animations ×1: o BWMS ainda não aplica esta seção (sem efeito no jogo); nada a corrigir no arquivo";
        let (t, avisos) = boot(&g).unwrap();
        // O `context` do `resource.fix` vai ao estado de mesh nativo (S12): não entra mais aqui.
        assert_eq!(t.sem_efeito_no_bwms, vec![(caminho.clone(), limite.to_string())]);
        assert_eq!(t.secoes_sem_efeito, vec![(caminho, secao.to_string())]);
        assert_eq!(avisos.len(), 1, "{avisos:?}");
        assert!(avisos[0].starts_with("Mods/Legendas/leg.xl: streaming.sectors[0] (s.streamingsector): sem expectedNodes"), "{avisos:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Tabela com o mesmo conteúdo não é regravada (o boot regrava a cada abertura, e cada gravação leva um F_FULLFSYNC); a tabela que muda
    /// é, pelo `rename` (inode novo).
    #[test]
    fn tabela_igual_nao_e_regravada_e_a_que_muda_e() {
        use std::os::unix::fs::MetadataExt;
        let g = jogo("iguais-nao-regravam");
        xl(&g, "X/x.xl", "resource:\n  link:\n    base\\x.ent: modx\\x.ent\n");
        let reslink = g.join("red4ext/bwms-reslink.txt");
        let inode = || std::fs::metadata(&reslink).unwrap().ino();
        boot(&g).unwrap();
        let antes = inode();
        boot(&g).unwrap();
        assert_eq!(inode(), antes, "conteúdo igual foi regravado");

        xl(&g, "X/x.xl", "resource:\n  link:\n    base\\x.ent: outro\\x.ent\n");
        boot(&g).unwrap();
        assert_ne!(inode(), antes);
        assert_eq!(linhas_do_reslink(&g), vec!["base\\x.ent | outro\\x.ent"]);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// A travessia da `Mods/`: ordem de bytes do caminho (`"Mod A b/"` antes de `"Mod A/"`, ' ' < '/'), sem nome com ponto, sem seguir link.
    #[test]
    fn travessia_da_pasta_mods_em_ordem_de_bytes_sem_nome_com_ponto_e_sem_link() {
        let raiz = std::env::temp_dir().join(format!("bwms-travessia-da-pasta-mods-{}", std::process::id()));
        let fora = std::env::temp_dir().join(format!("bwms-travessia-da-pasta-mods-fora-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&raiz);
        let _ = std::fs::remove_dir_all(&fora);
        for rel in ["Mod A/a.xl", "Mod A b/b.xl", "solto.xl", ".oculto/o.xl", "Mod A/.pulado.xl"] {
            escrever_texto(&raiz.join(rel), "x");
        }
        escrever_texto(&fora.join("de-fora.xl"), "x");
        std::os::unix::fs::symlink(&fora, raiz.join("Link")).unwrap();
        std::os::unix::fs::symlink(fora.join("de-fora.xl"), raiz.join("Mod A/link.xl")).unwrap();
        let rel: Vec<String> =
            arquivos_da_pasta_mods(&raiz).iter().map(|p| p.strip_prefix(&raiz).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(rel, ["Mod A b/b.xl", "Mod A/a.xl", "solto.xl"]);
        assert!(arquivos_da_pasta_mods(&raiz.join("nao-existe")).is_empty());
        let _ = std::fs::remove_dir_all(&raiz);
        let _ = std::fs::remove_dir_all(&fora);
    }

    /// A referência junta os links de todas as configs antes de resolver cadeia e ciclo: `a: b` de um mod e `b: c` de outro dão `a | c`.
    #[test]
    fn link_em_cadeia_e_em_ciclo_entre_xl_de_mods_diferentes_resolve_junto() {
        let g = jogo("reslink-entre-xl");
        xl(&g, "ModA/alfa.xl", "resource:\n  link:\n    a: b\n    x: y\n");
        xl(&g, "ModB/beta.xl", "resource:\n  link:\n    b: c\n    y: x\n");
        let (t, _) = boot(&g).unwrap();
        let linhas = linhas_do_reslink(&g);
        assert_eq!(linhas, vec!["a | c", "b | c"]);
        assert_eq!(t.reslink_pairs, linhas.len());
        assert_eq!(
            t.ciclos,
            vec!["resource.link x: cai num ciclo e fica fora de bwms-reslink.txt", "resource.link y: cai num ciclo e fica fora de bwms-reslink.txt"]
        );
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn escopo_em_ciclo_vai_para_os_ciclos_e_a_tabela_sai_sem_ele() {
        let g = jogo("escopo-ciclo");
        xl(&g, "EscopoMod/escopo.xl", "resource:\n  scope:\n    a: [b, x]\n    b: c\n    c: b\n");
        let (t, _) = boot(&g).unwrap();
        assert_eq!(t.resscope_lines, 1);
        let tabela = std::fs::read_to_string(g.join("red4ext/bwms-resscopes.txt")).unwrap();
        assert!(tabela.ends_with(&format!("{:#018x} | x\n", crate::instalacao::tabelas_de_runtime::resource_path_hash("a"))), "{tabela}");
        assert_eq!(t.ciclos.len(), 3, "{:?}", t.ciclos);
        assert!(t.ciclos.iter().all(|c| c.starts_with("resource.scope ")), "{:?}", t.ciclos);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Link em ciclo sai da tabela; duas cópias no mesmo caminho são uma entrada, a última, como no runtime (`retain` + `push`).
    #[test]
    fn reslink_pairs_e_o_numero_de_linhas_da_tabela() {
        let g = jogo("reslink-contagem");
        xl(&g, "LinkMod/resource.xl", "resource:\n  link:\n    a: b\n    x: y\n    y: x\n  copy:\n    o1: [c]\n    o2: [c]\n");
        let (t, _) = boot(&g).unwrap();
        let linhas = linhas_do_reslink(&g);
        assert_eq!(linhas, vec!["a | b", "c | o2 | copy"]);
        assert_eq!(t.reslink_pairs, linhas.len());
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O plano recebe os escopos de todos os `.xl`; com um `.xl` só, a tabela é a do `bwms xl <f> reslink` (`emit_reslink`).
    #[test]
    fn escopo_expande_o_link_e_o_copy_como_o_bwms_xl_reslink() {
        let g = jogo("reslink-escopo");
        let texto = "resource:\n  scope:\n    grupo: [base\\x.mesh, base\\y.mesh]\n    copias: [mod\\c1.mesh, mod\\c2.mesh]\n  link:\n    mod\\real.mesh: [grupo]\n  copy:\n    base\\orig.mesh: copias\n";
        xl(&g, "EscopoLink/escopo.xl", texto);
        let (t, _) = boot(&g).unwrap();
        let linhas = linhas_do_reslink(&g);
        assert_eq!(
            linhas,
            vec!["base\\x.mesh | mod\\real.mesh", "base\\y.mesh | mod\\real.mesh", "mod\\c1.mesh | base\\orig.mesh | copy", "mod\\c2.mesh | base\\orig.mesh | copy"]
        );
        assert_eq!(t.reslink_pairs, 4);
        let um = crate::instalacao::xl::parse_xl(texto).unwrap();
        let cli: Vec<String> = crate::instalacao::tabelas_de_runtime::emit_reslink(&um).lines().filter(|l| !l.starts_with('#')).map(String::from).collect();
        assert_eq!(linhas, cli);
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn escopo_de_um_mod_expande_o_link_de_outro() {
        let g = jogo("reslink-escopo-entre-mods");
        xl(&g, "ModEscopo/escopo.xl", "resource:\n  scope:\n    mod\\grupo.mesh: [base\\x.mesh, base\\y.mesh]\n");
        xl(&g, "ModLink/link.xl", "resource:\n  link:\n    mod\\grupo.mesh: mod\\real.mesh\n");
        let (t, _) = boot(&g).unwrap();
        assert_eq!(linhas_do_reslink(&g), vec!["base\\x.mesh | mod\\real.mesh", "base\\y.mesh | mod\\real.mesh"]);
        assert_eq!(t.reslink_pairs, 2);
        let _ = std::fs::remove_dir_all(&g);
    }

    /// A cópia de um mod derruba o link do outro no mesmo pedido: o motivo vai para os avisos, nunca sob o rótulo de ciclo.
    #[test]
    fn link_sobre_copia_de_outro_mod_sai_da_tabela_e_vai_para_avisos() {
        let g = jogo("reslink-link-sobre-copia");
        xl(&g, "ModLink/link.xl", "resource:\n  link:\n    mod\\k.mesh: base\\z.mesh\n");
        xl(&g, "ModCopia/copia.xl", "resource:\n  copy:\n    base\\o.mesh: mod\\k.mesh\n");
        let (t, avisos) = boot(&g).unwrap();
        assert_eq!(linhas_do_reslink(&g), vec!["mod\\k.mesh | base\\o.mesh | copy"]);
        assert_eq!(t.reslink_pairs, 1);
        assert!(t.ciclos.is_empty(), "{:?}", t.ciclos);
        assert_eq!(
            avisos,
            vec!["resource.link mod\\k.mesh: o pedido é um resource.copy e fica fora de bwms-reslink.txt; tire esse caminho do link ou do copy no .xl"]
        );
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Um `.xl` que gera linha em `bwms-reslink.txt`, `bwms-journal.txt`, `bwms-localizacao.txt` e
    /// `bwms-customizations.txt` (1ª, 4ª, 5ª e 9ª na ordem de gravação); o `factories` dele não tem tabela.
    fn mod_com_quatro_tabelas(g: &Path) {
        xl(
            g,
            "LinkMod/tabelas.xl",
            "resource:\n  link:\n    base\\fake\\a.mesh: base\\real\\b.mesh\nfactories:\n  - m\\f.csv\njournal:\n  - m\\q.journal\nlocalization:\n  onscreens:\n    en-us: m\\\\loc\\\\en-us.json\ncustomizations:\n  female: m\\f.inkcharcustomization\n",
        );
    }

    const FIM_DO_ERRO_DE_TABELA: &str =
        "Confira a permissão de escrita da pasta red4ext/ e abra o jogo de novo (o BWMS regrava as tabelas a cada abertura); até lá o jogo carrega as tabelas antigas.";

    #[test]
    fn tabela_que_nao_grava_nem_sai_de_red4ext_vira_erro_com_o_caminho() {
        let g = jogo("so-leitura");
        mod_com_quatro_tabelas(&g);
        let red4 = g.join("red4ext");
        std::fs::create_dir_all(&red4).unwrap();
        let quatro = ["bwms-reslink.txt", "bwms-journal.txt", "bwms-localizacao.txt", "bwms-customizations.txt"];

        // gravar: as 4 com linha falham; as 5 sem linha não existem, e tabela ausente não é erro
        pasta_so_leitura(&red4, true);
        let r = boot(&g);
        pasta_so_leitura(&red4, false);
        let e = r.unwrap_err();
        let texto = e.to_string();
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{texto}");
        assert!(texto.starts_with("não consegui atualizar 4 tabela(s) em red4ext/ ("), "{texto}");
        for t in quatro {
            assert!(texto.contains(&format!("{}: ", red4.join(t).display())), "{t} fora do erro: {texto}");
        }
        assert!(texto.ends_with(FIM_DO_ERRO_DE_TABELA), "{texto}");
        assert!(std::fs::read_dir(&red4).unwrap().next().is_none(), "nem tabela nem temporário em red4ext/");

        // tirar: com o mod fora da `Mods/`, tabela gerada que não sai é erro e continua no disco até uma abertura que consiga tirá-la
        boot(&g).unwrap();
        std::fs::remove_dir_all(g.join("Mods/LinkMod")).unwrap();
        pasta_so_leitura(&red4, true);
        let r = boot(&g);
        pasta_so_leitura(&red4, false);
        let texto = r.unwrap_err().to_string();
        assert!(texto.starts_with("não consegui atualizar 4 tabela(s) em red4ext/ ("), "{texto}");
        assert!(texto.ends_with(FIM_DO_ERRO_DE_TABELA), "{texto}");
        assert!(quatro.iter().all(|t| red4.join(t).exists()));
        let (t, _) = boot(&g).unwrap();
        assert_eq!((t.reslink_pairs, t.journal_lines, t.customization_lines), (0, 0, 0));
        assert!(std::fs::read_dir(&red4).unwrap().next().is_none(), "tabela gerada de mod que saiu ficou em red4ext/");
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn tabela_que_falha_nao_impede_as_seguintes() {
        // pasta no lugar da 1ª e da 6ª tabela: o `rename` do temporário falha só nelas
        let g = jogo("falha-no-meio");
        mod_com_quatro_tabelas(&g);
        let red4 = g.join("red4ext");
        std::fs::create_dir_all(red4.join("bwms-reslink.txt/ocupado")).unwrap();
        std::fs::create_dir_all(red4.join("bwms-localizacao.txt/ocupado")).unwrap();

        let texto = boot(&g).unwrap_err().to_string();
        assert!(texto.starts_with("não consegui atualizar 2 tabela(s) em red4ext/ ("), "{texto}");
        assert!(texto.contains(&format!("{}: ", red4.join("bwms-reslink.txt").display())), "{texto}");
        assert!(texto.contains(&format!("{}: ", red4.join("bwms-localizacao.txt").display())), "{texto}");
        assert!(texto.ends_with(FIM_DO_ERRO_DE_TABELA), "{texto}");
        for (t, linha) in [("bwms-journal.txt", "m\\q.journal"), ("bwms-customizations.txt", "female | m\\f.inkcharcustomization")] {
            let conteudo = std::fs::read_to_string(red4.join(t)).unwrap();
            assert!(conteudo.lines().any(|l| l.ends_with(linha)), "{t}: {conteudo}");
        }
        let sobra: Vec<_> = std::fs::read_dir(&red4).unwrap().flatten().map(|e| e.file_name()).filter(|n| n.to_string_lossy().ends_with(".tmp")).collect();
        assert!(sobra.is_empty(), "{sobra:?}");
        let _ = std::fs::remove_dir_all(&g);
    }

    /// Uma seção por um caminho só: o `onscreens` do `.xl` da `Mods/` fica na tabela do BWMS enquanto o `.json` do plano não está no disco, e
    /// sai dela quando está, com todos os `.json` iguais ao plano; o resto do `.xl` (aqui `journal`) continua na tabela. Sem `archive/Mac/` o
    /// plano não leva nada ao jogo.
    #[test]
    fn secao_entregue_ao_jogo_sai_da_tabela() {
        let g = jogo("xl-no-jogo");
        xl(&g, "E/e.archive.xl", "localization:\n  onscreens:\n    en-us: e\\en.json\n    pt-br: e\\pt.json\njournal:\n  - e\\j.journal\n");
        xl(&g, "V/v.archive.xl", "localization:\n  onscreens:\n    en-us: v\\en.json\n");
        let (t, _) = boot(&g).unwrap();
        assert!(t.no_jogo.configs.is_empty());
        assert_eq!(t.no_jogo.motivo, Some(crate::instalacao::xl_no_jogo::Motivo::SemPastaDoJogo));
        assert_eq!(linhas_da_localizacao(&g).len(), 3);
        std::fs::create_dir_all(g.join("archive/Mac/content")).unwrap();
        let (t, _) = boot(&g).unwrap();
        assert_eq!(t.no_jogo.configs.len(), 2);
        assert!(!t.no_jogo_entregue);
        assert_eq!(linhas_da_localizacao(&g).len(), 3, "sem o .json no disco o BWMS continua aplicando");
        // Só um dos dois no disco: o BWMS continua com os dois.
        let c = &t.no_jogo.configs[0];
        escrever_texto(&g.join(&c.destino), &c.conteudo);
        let (t, _) = boot(&g).unwrap();
        assert!(!t.no_jogo_entregue);
        assert_eq!(linhas_da_localizacao(&g).len(), 3);
        let c = &t.no_jogo.configs[1];
        escrever_texto(&g.join(&c.destino), &c.conteudo);
        let (t, _) = boot(&g).unwrap();
        assert!(t.no_jogo_entregue);
        assert!(linhas_da_localizacao(&g).is_empty(), "{:?}", linhas_da_localizacao(&g));
        assert!(std::fs::read_to_string(g.join("red4ext/bwms-journal.txt")).unwrap().contains("e\\j.journal"));
        let _ = std::fs::remove_dir_all(&g);
    }

    /// O `factories` não tem tabela: vai ao jogo pelo `.json` de cada `.xl`. A config só com `factories` (aqui todas: o `s.xl` declara os 19
    /// idiomas e os textos de todos ficam no BWMS) não tira o `.xl` da tabela de localização, nem com tudo no disco.
    #[test]
    fn factories_vai_ao_jogo_sem_tabela() {
        use crate::instalacao::localizacao::{e_idioma_conhecido, CODIGOS};
        let g = jogo("factories-no-jogo");
        std::fs::create_dir_all(g.join("archive/Mac/content")).unwrap();
        let todos: String = CODIGOS.into_iter().filter(|c| e_idioma_conhecido(c)).map(|c| format!("    {c}: s\\{c}.json\n")).collect();
        xl(&g, "F/f.archive.xl", "factories:\n  - f\\f.csv\n");
        xl(&g, "S/s.xl", &format!("localization:\n  onscreens:\n{todos}factories: s\\f.csv\n"));
        xl(&g, "T/t.xl", "localization:\n  onscreens:\n    en-us: t\\en.json\nfactories: t\\f.csv\n");
        let (t, _) = boot(&g).unwrap();
        let leva: Vec<(String, bool, bool)> = t
            .no_jogo
            .configs
            .iter()
            .map(|c| (c.xl.strip_prefix(&g).unwrap().display().to_string(), c.leva("localization.onscreens"), c.leva("factories")))
            .collect();
        assert_eq!(leva, [("Mods/F/f.archive.xl".into(), false, true), ("Mods/S/s.xl".into(), false, true), ("Mods/T/t.xl".into(), false, true)]);
        assert!(matches!(t.no_jogo.motivo, Some(crate::instalacao::xl_no_jogo::Motivo::SemIdiomaLivre(_))));
        assert!(t.no_jogo.factories_fora.is_empty());
        assert!(!t.no_jogo_entregue);
        let linhas = linhas_da_localizacao(&g).len();
        assert_eq!(linhas, CODIGOS.into_iter().filter(|c| e_idioma_conhecido(c)).count() + 1);
        for c in &t.no_jogo.configs {
            escrever_texto(&g.join(&c.destino), &c.conteudo);
        }
        let (t, _) = boot(&g).unwrap();
        assert!(t.no_jogo_entregue);
        assert_eq!(linhas_da_localizacao(&g).len(), linhas, "a config só com factories não tira os textos do BWMS");
        assert!(!g.join("red4ext/bwms-factories.txt").exists());
        let _ = std::fs::remove_dir_all(&g);
    }
}
