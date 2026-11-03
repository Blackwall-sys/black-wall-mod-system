//! A pasta `Mods/` ao lado do `Cyberpunk2077.app`: o usuário joga ali o que baixou (a pasta do mod como vem no zip, com ou sem a árvore do
//! Windows, ou arquivos soltos) e o BWMS acha as peças ao abrir o jogo: o `.archive` vai como clone APFS para a pasta que o próprio jogo
//! monta, `archive/Mac/mod/BWMS-Mods/` (`pasta_nativa`, scope 6, acima de `patch`/`ep1`/`content`), e o gancho do `InitializeArchives`
//! confere o que o jogo montou (`conferir_montagem`); o `.xl` nas tabelas de runtime de `red4ext/` (`preparar_no_boot`, pelo
//! escritor único do motor, `apply::gravar_tabelas_de_runtime`), o tweak no pipeline do TweakXL (`pipeline::apply_mod_tweaks`, pela lista de
//! `tweaks`) e o `.reds` na compilação do lançamento (`reds`). O que entrou e o que ficou de fora vai para `Mods/BWMS-RELATORIO.txt`,
//! regravado a cada boot.
//!
//! Quando dois trazem a mesma coisa: a `Mods/` vence o jogo. Entre mods da `Mods/`, no `.archive`
//! e no `.xl` vence o que vem primeiro em ordem de caminho (o motor serve o archive carregado primeiro, e o `.xl` entra nas tabelas de trás para
//! frente porque nelas vence o que vem depois); para inverter, renomeia-se a pasta ou o arquivo solto. No tweak, os arquivos são aplicados
//! em ordem de caminho, cada um na vez da chamada (`tweakdb::FilaDeVez`), e no mesmo flat vale o aplicado por
//! último: a ordem de aplicação é a que deixa um mod usar o record que outro criou antes dele.
//!
//! Com o mod.io (`mod_io`): no `.archive` a `Mods/` vence o mod.io (o jogo monta o mod.io no fim do mesmo set de `archive/Mac/mod`, e o 1º
//! que tem o hash serve), e entre dois do mod.io vence o de `modID` menor. Os conflitos dos 2 caminhos saem pelo índice dos archives
//! (`conflitos`) no relatório, na aba Mods e no selo (quem perde recurso fica amarelo).

use super::mod_io::ModsDoModIo;
use super::ModStatus;
use crate::arquivos_do_jogo::Recusa;
use bwms_primitives::instalacao::apply::{self, PastaMods, TabelasDosXl, PASTA_MODS};
use bwms_primitives::instalacao::xl_no_jogo::{Motivo, PlanoDoJogo};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};

/// O relatório para o usuário, na raiz da `Mods/`. A varredura não o conta como mod.
const RELATORIO: &str = "BWMS-RELATORIO.txt";

/// Pastas da árvore do jogo em que um `.yaml` solto não é tweak (configuração de plugin, de CET, do motor).
const ARVORES_DO_JOGO: [&str; 5] = ["archive", "bin", "engine", "r6", "red4ext"];

/// A pasta `Mods/` do jogo.
pub(crate) fn pasta(jogo: impl AsRef<Path>) -> PathBuf {
    jogo.as_ref().join(PASTA_MODS)
}

/// O que o BWMS faz com um arquivo da `Mods/`. Os outros tipos ficam onde estão, sem linha no relatório.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tipo {
    Archive,
    Xl,
    Tweak,
    Reds,
    Lua,
    Dll,
}

/// O tipo de um arquivo da `Mods/` pelo caminho dentro dela. O `.reds` compara a extensão exata, como o `scc` ao andar uma pasta: `X.REDS` não
/// compila no Windows e não entra aqui. O tweak (a regra única de extensão, `bwms_primitives::formato_tweak`) conta debaixo de `r6/tweaks/`
/// ou solto fora das árvores do jogo (`ARVORES_DO_JOGO`): um `.yaml` de `bin/` ou `red4ext/` é configuração de outro programa, e o parser do
/// TweakXL o leria como record.
fn tipo(rel: &Path) -> Option<Tipo> {
    let ext = rel.extension()?.to_str()?;
    let e = |x: &str| ext.eq_ignore_ascii_case(x);
    if e("archive") {
        Some(Tipo::Archive)
    } else if e("xl") {
        Some(Tipo::Xl)
    } else if ext == "reds" {
        Some(Tipo::Reds)
    } else if e("lua") {
        Some(Tipo::Lua)
    } else if e("dll") {
        Some(Tipo::Dll)
    } else if bwms_primitives::formato_tweak(ext).is_some() && tweak_no_lugar(rel) {
        Some(Tipo::Tweak)
    } else {
        None
    }
}

/// `rel` está debaixo de `r6/tweaks/` ou fora das árvores do jogo, sem distinguir caixa (o APFS padrão também não distingue).
fn tweak_no_lugar(rel: &Path) -> bool {
    let pastas: Vec<String> = rel
        .parent()
        .map(|p| p.components().filter_map(|c| matches!(c, Component::Normal(_)).then(|| c.as_os_str().to_string_lossy().to_ascii_lowercase())).collect())
        .unwrap_or_default();
    pastas.windows(2).any(|w| w[0] == "r6" && w[1] == "tweaks") || !pastas.iter().any(|p| ARVORES_DO_JOGO.contains(&p.as_str()))
}

/// A `Mods/` lida no `on_load` (`ler_no_boot`) antes da compilação dos `.reds`: a lista que a compilação pede (`reds`) e a varredura, as
/// tabelas e o relatório de `preparar_no_boot` saem da mesma andada. `preparar` a tira daqui; depois do boot, quem pede uma lista anda a
/// pasta de novo (o vigia dos scripts, a recarga dos tweaks).
static LEITURA_DO_BOOT: Mutex<Option<PastaMods>> = Mutex::new(None);

fn leitura_do_boot() -> std::sync::MutexGuard<'static, Option<PastaMods>> {
    LEITURA_DO_BOOT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// No `on_load`, antes da compilação dos `.reds` (`scripts::compilacao_no_lancamento`): lê a `Mods/` uma vez para ela e para
/// `preparar_no_boot`.
pub(crate) fn ler_no_boot() {
    let Some(jogo) = crate::arquivos_do_jogo::game_dir_from_dylib() else { return };
    *leitura_do_boot() = Some(apply::ler_pasta_mods(Path::new(&jogo)));
}

/// A leitura do boot, se for a de `raiz`, sai de `LEITURA_DO_BOOT`.
fn tirar_leitura_do_boot(raiz: &Path) -> Option<PastaMods> {
    let mut leitura = leitura_do_boot();
    if leitura.as_ref().is_some_and(|p| p.raiz == raiz) {
        leitura.take()
    } else {
        None
    }
}

/// Os arquivos de `raiz` do tipo `t`, em ordem de caminho (a travessia da `Mods/`, `apply::arquivos_da_pasta_mods`, ou a leitura do boot
/// quando ela é de `raiz`).
fn do_tipo(raiz: &Path, t: Tipo) -> Vec<PathBuf> {
    let filtrar = |arquivos: &[PathBuf]| arquivos.iter().filter(|p| p.strip_prefix(raiz).ok().and_then(tipo) == Some(t)).cloned().collect();
    if let Some(lida) = leitura_do_boot().as_ref().filter(|p| p.raiz == raiz) {
        return filtrar(&lida.arquivos);
    }
    filtrar(&apply::arquivos_da_pasta_mods(raiz))
}

/// Todo `.archive` debaixo de `raiz`, na ordem de carga.
pub(crate) fn archives(raiz: &Path) -> Vec<PathBuf> {
    do_tipo(raiz, Tipo::Archive)
}

/// Todo `.reds` debaixo de `raiz`, em ordem de caminho e com as exclusões da travessia da `Mods/` (`apply::arquivos_da_pasta_mods`: nome que
/// começa com ponto, link simbólico e pasta que não se lê ficam de fora): a lista que a compilação do lançamento leva ao compilador.
pub(crate) fn reds(raiz: &Path) -> Vec<PathBuf> {
    do_tipo(raiz, Tipo::Reds)
}

/// Os tweaks de `raiz` (`tipo`), em ordem de caminho: a ordem em que `pipeline::apply_mod_tweaks` os aplica.
pub(crate) fn tweaks(raiz: &Path) -> Vec<PathBuf> {
    do_tipo(raiz, Tipo::Tweak)
}

/// Os tweaks que a varredura do boot achou, sem varrer de novo: o hot reload (`pipeline::hot_reload_tick`) confere o mtime deles na thread do
/// jogo a cada ~10 s, e a varredura inteira da `Mods/` cresce com o número de arquivos de todos os mods. Tweak novo entra na recarga
/// (`pipeline::recarregar_tweaks`, que varre) ou no boot seguinte.
pub(crate) fn tweaks_do_boot() -> Vec<PathBuf> {
    estado().relatorio.as_ref().map(|r| r.mods.iter().flat_map(|m| m.tweaks.iter().cloned()).collect()).unwrap_or_default()
}

/// Um mod da `Mods/`: a pasta de 1º nível, ou o arquivo solto direto nela, com os arquivos por tipo em ordem de caminho.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ModDaPasta {
    pub(crate) nome: String,
    pub(crate) archives: Vec<PathBuf>,
    pub(crate) xls: Vec<PathBuf>,
    pub(crate) tweaks: Vec<PathBuf>,
    pub(crate) reds: Vec<PathBuf>,
    pub(crate) luas: usize,
    pub(crate) dlls: usize,
}

/// Os mods de `pasta`, na ordem de carga. Os arquivos de um mod vêm juntos na ordem de caminho (todo caminho entre dois `<mod>/…` também
/// começa com `<mod>/`), então o mod que vem antes carrega todos os archives dele antes do seguinte.
pub(crate) fn varrer(pasta: &PastaMods) -> Vec<ModDaPasta> {
    let mut mods: Vec<ModDaPasta> = Vec::new();
    for p in &pasta.arquivos {
        let Ok(rel) = p.strip_prefix(&pasta.raiz) else { continue };
        let Some(primeiro) = rel.components().next() else { continue };
        let nome = primeiro.as_os_str().to_string_lossy().into_owned();
        if nome == RELATORIO && rel.components().count() == 1 {
            continue;
        }
        if mods.last().map_or(true, |m| m.nome != nome) {
            mods.push(ModDaPasta { nome, ..ModDaPasta::default() });
        }
        let t = tipo(rel);
        let Some(m) = mods.last_mut() else { continue };
        match t {
            Some(Tipo::Archive) => m.archives.push(p.clone()),
            Some(Tipo::Xl) => m.xls.push(p.clone()),
            Some(Tipo::Tweak) => m.tweaks.push(p.clone()),
            Some(Tipo::Reds) => m.reds.push(p.clone()),
            Some(Tipo::Lua) => m.luas += 1,
            Some(Tipo::Dll) => m.dlls += 1,
            None => {}
        }
    }
    mods
}

/// O que o boot fez com as tabelas de `red4ext/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tabelas {
    /// Regravadas; os avisos do parse de cada `.xl` (`XlFile::issues` e o `.xl` que ficou fora) começam pelo caminho dele no jogo. O que o
    /// BWMS ainda não aplica vem à parte, em `TabelasDosXl::sem_efeito_no_bwms` e `secoes_sem_efeito`.
    Gravadas { tabelas: TabelasDosXl, avisos: Vec<String> },
    /// O texto do erro, com a ação.
    Falhou(String),
}

/// Regrava as tabelas de runtime com os `.xl` da `Mods/` pelo escritor único do motor (`apply::gravar_tabelas_de_runtime`).
fn gravar_tabelas(jogo: &Path, pasta: &PastaMods) -> Tabelas {
    match apply::gravar_tabelas_de_runtime(jogo, pasta) {
        Ok((tabelas, avisos)) => Tabelas::Gravadas { tabelas, avisos },
        Err(e) => Tabelas::Falhou(e.to_string()),
    }
}

/// Os tweaks de `mods` que o pipeline lê e não aplica, com o motivo: o parser da aplicação (`tweakdb::ops_do_arquivo`), sem o jogo.
fn tweaks_recusados(mods: &[ModDaPasta]) -> Vec<(PathBuf, String)> {
    mods.iter()
        .flat_map(|m| &m.tweaks)
        .filter_map(|p| crate::tweakdb::ops_do_arquivo(&p.to_string_lossy(), || false).err().map(|e| (p.clone(), e)))
        .collect()
}

/// Os 2 caminhos de um mod entrar no jogo: a `Mods/` (o BWMS faz funcionar) e o mod.io (o menu Mods do jogo; o jogo faz funcionar).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caminho {
    Mods,
    ModIo,
}

impl Caminho {
    /// O nome do caminho no relatório e na aba.
    pub(crate) fn nome(self) -> &'static str {
        match self {
            Caminho::Mods => "Mods",
            Caminho::ModIo => "mod.io",
        }
    }
}

/// Dois mods que trazem o mesmo recurso no `.archive`: dois da `Mods/`, um da `Mods/` e um do mod.io, ou dois do mod.io.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Conflito {
    /// O que carrega primeiro e serve o recurso.
    pub(crate) vence: String,
    pub(crate) perde: String,
    /// Recursos que os dois trazem.
    pub(crate) recursos: usize,
    pub(crate) vence_no: Caminho,
    pub(crate) perde_no: Caminho,
}

impl Conflito {
    /// O motivo do amarelo de quem perde: o recurso que ele traz não entra.
    pub(crate) fn motivo_de_quem_perde(&self) -> String {
        let (v, n, c) = (&self.vence, self.recursos, self.vence_no.nome());
        format!("perde {n} recurso(s) para {v} ({c}) no .archive / loses {n} resource(s) to {v} ({c}) in archives")
    }

    /// A linha do relatório: quem vence, pela regra medida, e o que o usuário faz para inverter. `solto` diz se o mod da `Mods/` é um arquivo
    /// solto na raiz dela.
    fn linha_do_relatorio(&self, solto: impl Fn(&str) -> bool) -> String {
        let (v, p, n) = (&self.vence, &self.perde, self.recursos);
        let (cv, cp) = (self.vence_no.nome(), self.perde_no.nome());
        match (self.vence_no, self.perde_no) {
            (Caminho::Mods, Caminho::Mods) => {
                let (renomeie, rename) = match (solto(v), solto(p)) {
                    (false, false) => ("renomeie a pasta", "rename the folder"),
                    (true, true) => ("renomeie o arquivo", "rename the file"),
                    _ => ("renomeie a pasta ou o arquivo", "rename the folder or the file"),
                };
                format!("{v} x {p}: {n} recurso(s) em comum, vence {v}; para inverter, {renomeie} / {n} shared resource(s), {v} wins; to swap, {rename}")
            }
            // O mod.io entra depois de toda a `archive/Mac/mod` (onde fica a `BWMS-Mods`), no mesmo set: o 1º que tem o hash serve.
            (Caminho::Mods, Caminho::ModIo) => format!(
                "{v} ({cv}) x {p} ({cp}): {n} recurso(s) em comum, vence {v}: no .archive a pasta Mods vence o mod.io; para o mod.io vencer, tire \
                 o mod da pasta Mods / {n} shared resource(s), {v} wins: in archives the Mods folder beats mod.io; for the mod.io mod to win, take \
                 the mod out of the Mods folder"
            ),
            // Entre dois do mod.io o jogo monta por `modID` crescente: a ordem não se troca, só desligando um no menu.
            _ => format!(
                "{v} ({cv}) x {p} ({cp}): {n} recurso(s) em comum, vence {v}: entre mods do mod.io vence o de id menor; para inverter, desligue \
                 {v} no menu Mods do jogo / {n} shared resource(s), {v} wins: between mod.io mods the lower id wins; to swap, disable {v} in the \
                 game's Mods menu"
            ),
        }
    }
}

/// Os pares de `mods` (caminho, nome e archives, na ordem de carga: a `Mods/` antes do mod.io) que trazem o mesmo recurso, pelo hash de
/// caminho do índice (`arquivos_do_jogo::hashes_do_archive`, sem ler os dados): quantos recursos cada par divide e quem vence, o que carrega
/// primeiro. O recurso repetido dentro de um mod não conta, e o archive que o motor não monta não disputa nada.
fn conflitos(mods: &[(Caminho, String, Vec<PathBuf>)]) -> Vec<Conflito> {
    let mut dono: HashMap<u64, usize> = HashMap::new();
    let mut pares: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (i, (_, _, archives)) in mods.iter().enumerate() {
        let deste: HashSet<u64> = archives.iter().filter_map(|a| crate::arquivos_do_jogo::hashes_do_archive(a)).flatten().collect();
        for h in deste {
            match dono.get(&h) {
                Some(&primeiro) => *pares.entry((primeiro, i)).or_default() += 1,
                None => {
                    dono.insert(h, i);
                }
            }
        }
    }
    pares
        .into_iter()
        .filter_map(|((v, p), recursos)| {
            let (vence_no, vence, _) = mods.get(v)?;
            let (perde_no, perde, _) = mods.get(p)?;
            Some(Conflito { vence: vence.clone(), perde: perde.clone(), recursos, vence_no: *vence_no, perde_no: *perde_no })
        })
        .collect()
}

/// O relatório da `Mods/` de um boot.
#[derive(Debug)]
struct Relatorio {
    raiz: PathBuf,
    mods: Vec<ModDaPasta>,
    tabelas: Tabelas,
    /// Os tweaks que não viram operação (`tweaks_recusados`), com o erro.
    tweaks_recusados: Vec<(PathBuf, String)>,
    /// `None` até a carga dos `.archive` (`conferir_montagem`); depois, os que a sincronização recusou.
    recusados: Option<Vec<(PathBuf, Recusa)>>,
    /// A sincronização com a pasta nativa do jogo (`pasta_nativa::sincronizar`), do `on_load`.
    nativa: Option<super::pasta_nativa::Sincronia>,
    /// Depois da carga: os `.archive` aceitos que o jogo não montou, com o motivo.
    fora_do_jogo: Vec<(PathBuf, super::pasta_nativa::MotivoDeFora)>,
    /// Cada `.xl` cujo `factories` o jogo não aplica, ou não todo, com o motivo (`factories_sem_efeito`): o mod fica amarelo.
    factories_sem_efeito: Vec<(PathBuf, Motivo)>,
}

impl Relatorio {
    /// Cada mod com os archives que o motor montou, na ordem de carga.
    fn aceitos(&self) -> Vec<(Caminho, String, Vec<PathBuf>)> {
        let recusados = self.recusados.as_deref().unwrap_or_default();
        let entrou = |a: &PathBuf| !recusados.iter().any(|(r, _)| r == a) && !self.fora_do_jogo.iter().any(|(f, _)| f == a);
        self.mods.iter().map(|m| (Caminho::Mods, m.nome.clone(), m.archives.iter().filter(|a| entrou(a)).cloned().collect())).collect()
    }
}

/// O relatório do boot, as seções que outras partes do runtime acrescentaram (`anexar_ao_relatorio`), os mods do mod.io e os conflitos dos
/// 2 caminhos. Os do mod.io e os conflitos existem também sem a pasta `Mods/` (o selo e a aba os mostram; o relatório não cria a pasta).
struct Estado {
    relatorio: Option<Relatorio>,
    secoes: Vec<(String, Vec<String>)>,
    modio: ModsDoModIo,
    /// `None` até a carga dos `.archive` e a leitura dos índices dos aceitos.
    conflitos: Option<Vec<Conflito>>,
}

/// Escrito pelo `on_load` (`preparar_no_boot`), pelo gancho do `InitializeArchives` (`conferir_montagem`), pela thread dos conflitos e por
/// `anexar_ao_relatorio`; cada escritor publica tudo o que já se sabe (`publicar`).
static ESTADO: Mutex<Estado> = Mutex::new(Estado { relatorio: None, secoes: Vec::new(), modio: ModsDoModIo::vazio(), conflitos: None });

fn estado() -> std::sync::MutexGuard<'static, Estado> {
    ESTADO.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Acrescenta ao relatório da `Mods/` a seção `secao` com `linhas` (cada uma em PT-BR e inglês, `"<pt> / <en>"`), no lugar da seção de mesmo
/// título que já estiver lá, e regrava o arquivo; antes da 1ª gravação do boot (`preparar_no_boot`), a seção espera e entra nela. É a porta
/// das seções `Scripts` (a compilação dos `.reds` da `Mods/`, que sai de `reds`) e `Recursos` (o patch de recurso que o depot não tem,
/// `arquivos_do_jogo::carregar_tabela_de_patches`).
pub(crate) fn anexar_ao_relatorio(secao: &str, linhas: &[String]) {
    let mut e = estado();
    anexar(&mut e.secoes, secao, linhas);
    publicar(&e);
}

/// Seção sem linhas sai: a carga que não acha mais o problema tira a seção da carga anterior.
fn anexar(secoes: &mut Vec<(String, Vec<String>)>, secao: &str, linhas: &[String]) {
    if linhas.is_empty() {
        secoes.retain(|(t, _)| t != secao);
        return;
    }
    match secoes.iter_mut().find(|(t, _)| t == secao) {
        Some(s) => s.1 = linhas.to_vec(),
        None => secoes.push((secao.to_string(), linhas.to_vec())),
    }
}

/// O que se sabe além do relatório do boot: a luz de cada mod da `Mods/`, os conflitos dos 2 caminhos e os mods do mod.io (com as perdas de
/// conflito já marcadas).
struct Sabido<'a> {
    luzes: &'a EstadosDosMods,
    conflitos: Option<&'a [Conflito]>,
    modio: &'a ModsDoModIo,
}

/// A luz de cada mod da `Mods/` (`estados_de`) com o amarelo de quem perde recurso num conflito.
fn luzes_de(e: &Estado) -> EstadosDosMods {
    let scripts = crate::scripts::compilacao_no_lancamento::estado_dos_scripts();
    let patches = crate::arquivos_do_jogo::patches_fora_do_depot();
    luzes_com(e.relatorio.as_ref(), &scripts, &patches, e.conflitos.as_deref())
}

fn luzes_com(r: Option<&Relatorio>, scripts: &EstadosDosMods, patches_fora: &[(String, String)], conflitos: Option<&[Conflito]>) -> EstadosDosMods {
    let mut luzes = estados_de(r, scripts, patches_fora);
    if let (Some(r), Some(cs)) = (r, conflitos) {
        for c in cs.iter().filter(|c| c.perde_no == Caminho::Mods) {
            luzes.marcar(&r.raiz.join(&c.perde), CorDoMod::Amarelo, &c.motivo_de_quem_perde());
        }
    }
    luzes
}

/// Regrava `Mods/BWMS-RELATORIO.txt` e troca a lista do selo e da aba Mods (`lista_dos_mods`) com o que `e` já sabe. Sem a pasta o relatório
/// não é gravado (ele não cria a `Mods/`), e a lista sai do mesmo jeito: os mods do mod.io não dependem dela.
fn publicar(e: &Estado) {
    let luzes = luzes_de(e);
    let modio = e.modio.com_perdas(e.conflitos.as_deref().unwrap_or_default());
    let sabido = Sabido { luzes: &luzes, conflitos: e.conflitos.as_deref(), modio: &modio };
    if let Some(r) = &e.relatorio {
        if let Err(err) = std::fs::write(r.raiz.join(RELATORIO), texto_do_relatorio(r, &sabido, &e.secoes)) {
            crate::log_dev!("[mods] relatório da pasta Mods não gravado: {err}");
        }
    }
    let raiz = e.relatorio.as_ref().map(|r| r.raiz.as_path());
    super::lista_dos_mods::publicar(super::lista_dos_mods::montar(raiz, &luzes, &modio, e.conflitos.as_deref().unwrap_or_default()));
}

/// `texto` com os caminhos da máquina tirados: o que começa na pasta do jogo fica relativo a ela (`Mods/X/…`), e o que começa na home vira
/// `~/…` (a pasta do mod.io). O relatório é do usuário e vai em pedido de ajuda: o caminho de lá não diz nada a quem lê e expõe a máquina.
pub(crate) fn sem_caminho_da_maquina(texto: &str, jogo: Option<&Path>, home: Option<&Path>) -> String {
    let mut t = texto.to_string();
    if let Some(j) = jogo.filter(|j| j.as_os_str().len() > 1) {
        t = t.replace(&format!("{}/", j.display()), "");
    }
    if let Some(h) = home.filter(|h| h.as_os_str().len() > 1) {
        t = t.replace(&format!("{}/", h.display()), "~/");
    }
    t
}

/// O nome da luz no relatório, o mesmo das cores do selo e da aba.
fn nome_da_luz(c: CorDoMod) -> &'static str {
    ModStatus::from(c).nome_da_luz()
}

/// A seção do mod.io: cada mod com a luz e o motivo, e a regra de quem vence. Sem mod do mod.io, nada.
fn texto_do_modio(m: &ModsDoModIo) -> String {
    if m.len() == 0 {
        return String::new();
    }
    let mut t = String::from(
        "\n[mod.io]\n\
         O jogo carrega estes mods sozinho pelo menu Mods dele; o BWMS só lê a pasta deles. No .archive a pasta Mods vence o mod.io; nos .json \
         de config o jogo lê o mod.io por último. / The game loads these mods by itself from its Mods menu; BWMS only reads their folder. In \
         archives the Mods folder beats mod.io; in config .json files the game reads mod.io last.\n",
    );
    for i in 0..m.len() {
        t.push_str(&format!("{}: luz / light: {}\n", m.rotulos[i], m.luzes[i].nome_da_luz()));
        for l in m.motivos[i].split('\n').filter(|l| !l.is_empty()) {
            t.push_str(&format!("  motivo / reason: {l}\n"));
        }
    }
    t
}

/// O texto de `Mods/BWMS-RELATORIO.txt`: curto, cada linha em PT-BR e inglês, sem caminho da máquina (`sem_caminho_da_maquina`).
fn texto_do_relatorio(r: &Relatorio, sabido: &Sabido, secoes: &[(String, Vec<String>)]) -> String {
    let jogo = r.raiz.parent();
    sem_caminho_da_maquina(&texto_cru_do_relatorio(r, sabido, secoes), jogo, super::mod_io::home().as_deref())
}

fn texto_cru_do_relatorio(r: &Relatorio, sabido: &Sabido, secoes: &[(String, Vec<String>)]) -> String {
    let mut t = String::from(
        "BWMS — pasta Mods / Mods folder\n\
         Regravado a cada abertura do jogo. / Rewritten every time the game starts.\n\
         Ordem: a Mods vence o jogo; entre duas pastas da Mods, no archive e no .xl vence a que vem antes em ordem de \
         caminho (letra a letra, maiúscula antes de minúscula), e no tweak, a que vem depois. / Order: Mods beats the game; between two \
         Mods folders, in archives and .xl the one that comes first in path order (character by character, uppercase \
         first) wins, and in tweaks, the one that comes later.\n",
    );
    if r.mods.is_empty() {
        t.push_str("\nNenhum mod na pasta: ponha aqui a pasta do mod como ela vem no zip. / No mods in the folder: put the mod folder here as it comes in the zip.\n");
    }
    type Partes<'a> = &'a [(PathBuf, String)];
    let (xl_fora, avisos, sem_efeito, secoes_sem_efeito): (&[PathBuf], &[String], Partes, Partes) = match &r.tabelas {
        Tabelas::Gravadas { tabelas, avisos } => (&tabelas.xl_fora, avisos, &tabelas.sem_efeito_no_bwms, &tabelas.secoes_sem_efeito),
        _ => (&[], &[], &[], &[]),
    };
    let xl_entrou = matches!(r.tabelas, Tabelas::Gravadas { .. });
    let no_jogo = match &r.tabelas {
        Tabelas::Gravadas { tabelas, .. } => Some((&tabelas.no_jogo, tabelas.no_jogo_entregue)),
        _ => None,
    };
    // O mod solto na raiz da `Mods/` é o próprio arquivo: a ação do conflito manda renomear o arquivo, não uma pasta.
    let solto = |nome: &str| r.mods.iter().any(|m| m.nome == nome && m.archives.contains(&r.raiz.join(nome)));
    // Os motivos que as linhas do próprio bloco já dizem (archive, tweak, .xl, Lua/.dll): a luz do bloco só repete os que vêm de fora dele
    // (a compilação dos scripts, o conflito, o patch de recurso).
    let do_bloco = estados_de(Some(r), &EstadosDosMods::default(), &[]);
    for m in &r.mods {
        t.push_str(&format!("\n[{}]\n", m.nome));
        let raiz_do_mod = r.raiz.join(&m.nome);
        if let Some(i) = sabido.luzes.raizes.iter().position(|p| *p == raiz_do_mod) {
            t.push_str(&format!("luz / light: {}\n", nome_da_luz(sabido.luzes.cores[i])));
            let ja_no_bloco = do_bloco.raizes.iter().position(|p| *p == raiz_do_mod).map_or("", |j| do_bloco.motivos[j].as_str());
            for l in sabido.luzes.motivos[i].split('\n').filter(|l| !l.is_empty() && !ja_no_bloco.split('\n').any(|b| b == *l)) {
                t.push_str(&format!("  motivo / reason: {l}\n"));
            }
        }
        let recusados: Vec<_> = r.recusados.iter().flatten().filter(|(p, _)| m.archives.contains(p)).collect();
        let tweaks_recusados: Vec<_> = r.tweaks_recusados.iter().filter(|(p, _)| m.tweaks.contains(p)).collect();
        // O que ficou fora do jogo sai na seção da pasta do jogo, com o motivo.
        let fora = r.fora_do_jogo.iter().filter(|(p, _)| m.archives.contains(p)).count();
        // Antes da carga dos `.archive` (o gancho do `InitializeArchives`) nenhum entrou ainda.
        let (archives, aguardando) = match r.recusados {
            Some(_) => (m.archives.len().saturating_sub(recusados.len() + fora), 0),
            None => (0, m.archives.len()),
        };
        let xls = if xl_entrou { m.xls.iter().filter(|x| !xl_fora.contains(x)).count() } else { 0 };
        // O tweak vai ao TweakDB no load do save (`pipeline::session_phase`), depois deste relatório, e a aplicação não volta para ele: aqui
        // o tweak só foi lido.
        let tweaks_lidos = m.tweaks.len().saturating_sub(tweaks_recusados.len());
        let entrou: Vec<String> =
            [(archives, ".archive"), (xls, ".xl")].into_iter().filter(|(n, _)| *n > 0).map(|(n, tipo)| format!("{n} {tipo}")).collect();
        if !entrou.is_empty() {
            t.push_str(&format!("entrou / loaded: {}\n", entrou.join(", ")));
        } else if aguardando == 0 && tweaks_lidos == 0 && m.reds.is_empty() {
            t.push_str("entrou / loaded: nada / nothing\n");
        }
        if aguardando > 0 {
            t.push_str(&format!(
                "aguardando / pending: {aguardando} .archive (aguardando a carga do jogo / waiting for the game to load them)\n"
            ));
        }
        if tweaks_lidos > 0 {
            t.push_str(&format!(
                "tweaks: {tweaks_lidos} lido(s) — aplicado(s) ao carregar o save / {tweaks_lidos} read — applied when a save loads\n"
            ));
        }
        if !m.reds.is_empty() {
            t.push_str(&format!(
                "scripts: {} .reds — a compilação sai na seção Scripts / build result in the Scripts section\n",
                m.reds.len()
            ));
        }
        // O caminho dentro do mod; no arquivo solto na raiz da `Mods/` o mod é o próprio arquivo, e o nome dele é o caminho.
        let dentro = r.raiz.join(&m.nome);
        let no_mod = |p: &Path| match p.strip_prefix(&dentro) {
            Ok(rel) if rel.as_os_str().is_empty() => p.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            Ok(rel) => rel.display().to_string(),
            Err(_) => p.display().to_string(),
        };
        for (p, motivo) in recusados {
            let (pt, en) = motivo.texto();
            t.push_str(&format!("recusado / refused: {} — {pt} / {en}\n", no_mod(p)));
        }
        // O motivo que vem do parser (tweak e `.xl`) só existe em PT-BR: a linha de texto fixo leva as duas línguas e a ação, e o motivo vai
        // embaixo como detalhe.
        for (p, erro) in tweaks_recusados {
            t.push_str(&format!(
                "recusado / refused: {} — o tweak não foi lido; corrija o arquivo e abra o jogo de novo / the tweak could not be read; fix the file \
                 and start the game again\n  detalhe / detail: {erro}\n",
                no_mod(p)
            ));
        }
        for x in &m.xls {
            // Os avisos do motor começam pelo caminho do `.xl` no jogo (`apply::gravar_tabelas_de_runtime`).
            let arquivo = format!("{PASTA_MODS}/{}: ", x.strip_prefix(&r.raiz).unwrap_or(x).display());
            let detalhes: Vec<&str> = avisos.iter().filter_map(|a| a.strip_prefix(&arquivo)).collect();
            if xl_fora.contains(x) {
                t.push_str(&format!(
                    "recusado / refused: {} — o .xl não foi lido; corrija o arquivo e abra o jogo de novo / the .xl could not be read; fix the file \
                     and start the game again\n",
                    no_mod(x)
                ));
            } else if !detalhes.is_empty() {
                t.push_str(&format!(
                    "aviso / warning: {} — parte do .xl ficou sem efeito; corrija o arquivo e abra o jogo de novo / part of the .xl has no effect; \
                     fix the file and start the game again\n",
                    no_mod(x)
                ));
            }
            for d in detalhes {
                t.push_str(&format!("  detalhe / detail: {d}\n"));
            }
            // O que o BWMS ainda não aplica não é defeito do `.xl` (o mesmo arquivo funciona no Windows): a linha não manda corrigir.
            let limites: Vec<&str> = sem_efeito.iter().chain(secoes_sem_efeito).filter(|(p, _)| p == x).map(|(_, d)| d.as_str()).collect();
            if !limites.is_empty() {
                t.push_str(&format!(
                    "aviso / warning: {} — o BWMS ainda não aplica parte deste .xl; o resto entrou / BWMS does not apply part of this .xl yet; the \
                     rest loaded\n",
                    no_mod(x)
                ));
            }
            for d in limites {
                t.push_str(&format!("  detalhe / detail: {d}\n"));
            }
            for (_, m) in r.factories_sem_efeito.iter().filter(|(p, _)| p == x) {
                let (pt, en) = m.texto();
                t.push_str(&format!(
                    "aviso / warning: {} — itens (factories) sem efeito no jogo / items (factories) have no effect in the game\n  detalhe / detail: \
                     {pt} / {en}\n",
                    no_mod(x)
                ));
            }
            // Quem aplica cada seção: o jogo, pelo `.json` em `BWMS-Mods/xl/` (`xl_no_jogo`), ou o BWMS. O `factories` só tem o jogo.
            if let Some((_, chaves)) = no_jogo.and_then(|(p, _)| p.secoes.iter().find(|(s, _)| s == x)) {
                let entregue = no_jogo.is_some_and(|(_, e)| e);
                let partes: Vec<String> = chaves
                    .iter()
                    .map(|k| {
                        let jogo = entregue && no_jogo.is_some_and(|(p, _)| p.no_jogo(x, k));
                        let fora = if k == "factories" { "sem efeito / no effect" } else { "BWMS" };
                        format!("{k} → {}", if jogo { "jogo / game" } else { fora })
                    })
                    .collect();
                if !partes.is_empty() {
                    t.push_str(&format!("seções / sections: {} — {}\n", no_mod(x), partes.join("; ")));
                }
            }
        }
        if m.luas > 0 {
            t.push_str(&format!(
                "Lua: {} .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll não / CET mod: BWMS runs it from the Mods folder; \
                 whatever needs a .dll does not\n",
                m.luas
            ));
        }
        if m.dlls > 0 {
            t.push_str(&format!(
                "fora / left out: {} .dll — plugin de Windows, não carrega no Mac / Windows plugin, does not load on a Mac\n",
                m.dlls
            ));
        }
    }
    if let Tabelas::Falhou(erro) = &r.tabelas {
        t.push_str(&format!(
            "\n[Tabelas / Tables]\nos .xl da Mods ficaram fora das tabelas do jogo; corrija o erro abaixo e abra o jogo de novo / the Mods .xl files \
             were left out of the game tables; fix the error below and start the game again\n  detalhe / detail: {erro}\n"
        ));
    }
    if let Some(conflitos) = sabido.conflitos {
        t.push_str("\n[Conflitos / Conflicts]\n");
        if conflitos.is_empty() {
            t.push_str("nenhum / none\n");
        }
        for c in conflitos {
            t.push_str(&c.linha_do_relatorio(&solto));
            t.push('\n');
        }
    }
    t.push_str(&texto_do_modio(sabido.modio));
    if let Some(s) = &r.nativa {
        t.push_str(&texto_da_pasta_nativa(r, s));
    }
    for (titulo, linhas) in secoes {
        t.push_str(&format!("\n[{titulo}]\n"));
        for l in linhas {
            t.push_str(l);
            t.push('\n');
        }
    }
    t
}

/// A seção da pasta nativa do jogo: quantos `.archive` estão lá e como chegaram, o que saiu, o aviso do save e o que ficou fora do jogo, com o
/// motivo e a ação (antes da carga, o que a sincronização não pôs; depois, também o clone que o jogo não montou).
fn texto_da_pasta_nativa(r: &Relatorio, s: &super::pasta_nativa::Sincronia) -> String {
    use super::pasta_nativa::Via;
    let fora = if r.recusados.is_some() { &r.fora_do_jogo } else { &s.fora };
    // Os textos da `Mods/` que ficaram no BWMS, com o motivo: o do plano, ou o `.json` que não gravou. O que os `.json` no disco levam: os
    // textos só com o plano inteiro entregue; com os textos de volta ao BWMS, ficam os `.json` só com `factories` (`PlanoDoJogo::so_factories`).
    let (textos_no_bwms, (com_textos, com_itens)) = match &r.tabelas {
        Tabelas::Gravadas { tabelas, .. } => {
            let p = &tabelas.no_jogo;
            let na_mods = p.secoes.iter().any(|(_, k)| k.iter().any(|k| k == "localization.onscreens"));
            let motivo = match (&p.motivo, tabelas.no_jogo_entregue) {
                _ if !na_mods => None,
                (Some(m), _) => Some(m.clone()),
                (None, false) => s.json_falhou.clone().map(Motivo::NaoGravou),
                (None, true) => None,
            };
            let leva = |secao: &str| p.configs.iter().any(|c| c.leva(secao));
            (motivo, (tabelas.no_jogo_entregue && leva("localization.onscreens"), leva("factories")))
        }
        _ => (None, (false, true)),
    };
    if s.nativos.is_empty() && s.removidos == 0 && fora.is_empty() && s.jsons.is_empty() && textos_no_bwms.is_none() {
        return String::new();
    }
    let mut t = String::from("\n[Pasta do jogo / Game folder]\n");
    if !s.nativos.is_empty() {
        let conta = |v: Via| s.nativos.iter().filter(|(_, _, x)| *x == v).count();
        let (n, clones, copias, em_dia) = (s.nativos.len(), conta(Via::Clone), conta(Via::Copia), conta(Via::Mantido));
        t.push_str(&format!(
            "{n} .archive em archive/Mac/mod/BWMS-Mods, a pasta que o próprio jogo carrega ({clones} clone(s) APFS, sem espaço extra em disco; \
             {copias} cópia(s); {em_dia} já em dia) / {n} .archive in archive/Mac/mod/BWMS-Mods, the folder the game itself loads ({clones} APFS \
             clone(s), no extra disk space; {copias} copy(ies); {em_dia} already up to date)\n\
             Saves: com archive nessa pasta o jogo marca o save como modded (ícone de mod na lista de saves do Mac) / Saves: with archives in that \
             folder the game marks saves as modded (mod icon in the Mac save list)\n"
        ));
    }
    if s.removidos > 0 {
        t.push_str(&format!(
            "{0} arquivo(s) saiu(saíram) de BWMS-Mods porque não corresponde(m) a um .archive atual da Mods / {0} file(s) removed from \
             BWMS-Mods because they do not match a current .archive in the Mods folder\n",
            s.removidos
        ));
    }
    for (p, m) in fora {
        let (pt, en) = m.texto();
        let rel = p.strip_prefix(&r.raiz).unwrap_or(p).display().to_string();
        t.push_str(&format!("não entrou / not loaded: {rel} — {pt} / {en}\n"));
    }
    if !s.jsons.is_empty() {
        let (pt, en) = match (com_textos, com_itens) {
            (true, true) => ("os textos (localization.onscreens) e os itens (factories)", "the texts (localization.onscreens) and the items (factories)"),
            (false, true) => ("os itens (factories)", "the items (factories)"),
            _ => ("os textos (localization.onscreens)", "the texts (localization.onscreens)"),
        };
        t.push_str(&format!(
            "{0} .json em archive/Mac/mod/BWMS-Mods/xl: o próprio jogo aplica {pt} desses .xl, e o BWMS não os aplica / {0} .json in \
             archive/Mac/mod/BWMS-Mods/xl: the game itself applies {en} of those .xl files, and BWMS does not\n",
            s.jsons.len()
        ));
    }
    if let Some(m) = textos_no_bwms {
        let (pt, en) = m.texto();
        t.push_str(&format!(
            "textos (localization.onscreens) dos .xl da Mods: pelo BWMS — {pt} / texts (localization.onscreens) of the Mods .xl files: through BWMS — {en}\n"
        ));
    }
    t
}

/// A varredura, as tabelas e os tweaks recusados de `jogo`, pela leitura do boot (`ler_no_boot`) quando ela existe; `None` sem a pasta
/// `Mods/` (as tabelas ainda saem limpas pela marca do motor).
fn preparar(jogo: &Path) -> Option<Relatorio> {
    preparar_com(jogo, tirar_leitura_do_boot(&pasta(jogo)).unwrap_or_else(|| apply::ler_pasta_mods(jogo)))
}

fn preparar_com(jogo: &Path, pasta: PastaMods) -> Option<Relatorio> {
    let mods = varrer(&pasta);
    let tabelas = gravar_tabelas(jogo, &pasta);
    let tweaks_recusados = tweaks_recusados(&mods);
    crate::log_dev!("[mods] pasta Mods: {} mod(s); tabelas: {tabelas:?}; tweaks recusados: {tweaks_recusados:?}", mods.len());
    pasta.raiz.is_dir().then_some(Relatorio {
        raiz: pasta.raiz,
        mods,
        tabelas,
        tweaks_recusados,
        recusados: None,
        nativa: None,
        fora_do_jogo: Vec::new(),
        factories_sem_efeito: Vec::new(),
    })
}

/// O texto que o boot gravaria para `jogo` com `secoes` anexadas, sem o `ESTADO` do processo: o teste de quem anexa uma seção.
#[cfg(test)]
pub(crate) fn texto_do_boot(jogo: &Path, secoes: &[(String, Vec<String>)]) -> Option<String> {
    preparar(jogo).map(|r| texto_do_relatorio(&r, &nada(), secoes))
}

#[cfg(test)]
static SEM_LUZES: EstadosDosMods = EstadosDosMods { raizes: Vec::new(), cores: Vec::new(), motivos: Vec::new() };
#[cfg(test)]
static SEM_MODIO: ModsDoModIo = ModsDoModIo::vazio();

/// Nada além do relatório do boot: sem luz, sem conflito, sem mod do mod.io.
#[cfg(test)]
fn nada() -> Sabido<'static> {
    Sabido { luzes: &SEM_LUZES, conflitos: None, modio: &SEM_MODIO }
}

/// No `on_load`, antes de quem lê as tabelas de `red4ext/` (o gancho do `PostLoad` de setor olha a de streaming ainda no `on_load`, e a carga
/// do 1º tick, `pipeline::boot_phase`, lê as outras) e antes do `InitializeArchives` do jogo (o construtor da dylib roda antes do `main()`; o
/// depot e o `InitializeArchives` vêm depois, na subida do motor): varre a `Mods/`, põe os `.archive` e os `.json` da parte dos `.xl` que o jogo
/// aplica na pasta nativa (`pasta_nativa::sincronizar`, também sem `Mods/`: o que o BWMS pôs antes sai), regrava as tabelas com os `.xl` dela
/// (que tiram a seção cujo `.json` está no disco, `xl_no_jogo::no_disco`) e grava o relatório. Os `.json` vão antes das tabelas e antes da
/// leitura das configs do jogo (`0x101cbd304`, na subida do motor).
pub(crate) fn preparar_no_boot() {
    let Some(jogo) = crate::arquivos_do_jogo::game_dir_from_dylib() else { return };
    let jogo = Path::new(&jogo);
    let leitura = tirar_leitura_do_boot(&pasta(jogo)).unwrap_or_else(|| apply::ler_pasta_mods(jogo));
    // O mod.io é lido aqui, uma vez por abertura: os conflitos dele saem na thread da carga dos archives (`conferir_montagem`), e os archives
    // dele contam para o `factories` da `Mods/` (`factories_sem_efeito`).
    let t0 = std::time::Instant::now();
    let modio = super::mod_io::home().map(|h| super::mod_io::ler(&h)).unwrap_or(ModsDoModIo::vazio());
    let leitura_do_modio = t0.elapsed();
    let (mut r, s) = preparar_com_a_pasta_nativa(jogo, leitura, &modio);
    crate::log_dev!(
        "[mods] pasta nativa: {} na pasta, {} fora da pasta, {} recusado(s), {} removido(s)",
        s.nativos.len(),
        s.fora.len(),
        s.recusados.len(),
        s.removidos
    );
    if let Some(r) = r.as_mut() {
        r.nativa = Some(s);
    }
    crate::veredito_dev!(
        "[mods] mod.io: {} mod(s), {} ligado(s), {} archive(s) que disputam recurso; leitura {} us",
        modio.len(),
        modio.ligados.iter().filter(|l| **l == Some(true)).count(),
        modio.archives.iter().flatten().count(),
        leitura_do_modio.as_micros()
    );
    let mut e = estado();
    e.relatorio = r;
    e.modio = modio;
    e.conflitos = None;
    secao_da_biblioteca_antiga(jogo, &mut e.secoes);
    publicar(&e);
}

/// A seção do que o instalador fez com a biblioteca da 0.1.2–0.1.4 (`instalacao::migracao`, pelo registro que ele gravou), enquanto a pasta
/// `BWMS/` existir ou algo dela tiver ficado no jogo.
fn secao_da_biblioteca_antiga(jogo: &Path, secoes: &mut Vec<(String, Vec<String>)>) {
    use bwms_primitives::instalacao::migracao;
    match migracao::linhas_do_relatorio(jogo) {
        Some(linhas) => anexar(secoes, migracao::TITULO, &linhas),
        None => secoes.retain(|(t, _)| t != migracao::TITULO),
    }
}

/// O miolo de `preparar_no_boot` sem o `ESTADO`: o plano do que o jogo aplica, a pasta nativa (`.archive` e `.json`), as tabelas, o
/// `.json` que sai se as tabelas não saíram sem a seção dele, o `factories` que fica sem efeito e o veredito.
fn preparar_com_a_pasta_nativa(jogo: &Path, leitura: PastaMods, modio: &ModsDoModIo) -> (Option<Relatorio>, super::pasta_nativa::Sincronia) {
    let plano = apply::plano_do_boot(jogo, &leitura);
    let archives: Vec<PathBuf> = varrer(&leitura).iter().flat_map(|m| m.archives.iter().cloned()).collect();
    let mut s = super::pasta_nativa::sincronizar(jogo, &archives, &plano.configs);
    let mut r = preparar_com(jogo, leitura);
    // Os textos só ficam no `.json` com as tabelas gravadas sem eles (`no_jogo_entregue`). Com as tabelas sem gravar ficam as de antes, que
    // podem levar os mesmos textos: eles saem do `.json`, e o `factories`, que não tem tabela, fica.
    let entregue = matches!(r.as_ref().map(|r| &r.tabelas), Some(Tabelas::Gravadas { tabelas, .. }) if tabelas.no_jogo_entregue);
    if !s.jsons.is_empty() && !entregue {
        s = super::pasta_nativa::sincronizar(jogo, &archives, &plano.so_factories());
    }
    let sem_efeito = factories_sem_efeito(jogo, &plano, &s, modio);
    // O veredito e os textos que a sonda confere são do boot de teste: o público não relê a tabela nem monta a linha.
    #[cfg(feature = "devlog")]
    veredito_do_jogo(jogo, &s, r.as_ref().map(|r| &r.tabelas), &sem_efeito);
    if let Some(r) = r.as_mut() {
        r.factories_sem_efeito = sem_efeito;
    }
    (r, s)
}

/// Cada `.xl` da `Mods/` cujo `factories` o jogo não aplica, ou não todo: o do plano (sem `archive/Mac`), o do `.json` que não gravou, e o
/// `.csv` que não está em nenhum `.archive` que o jogo monta antes de ler as configs (`recursos_montados_pelo_jogo`). Os índices só são lidos
/// com `factories` num `.json` no disco.
fn factories_sem_efeito(jogo: &Path, plano: &PlanoDoJogo, s: &super::pasta_nativa::Sincronia, modio: &ModsDoModIo) -> Vec<(PathBuf, Motivo)> {
    let mut v = plano.factories_fora.clone();
    let com_factories: Vec<&PathBuf> = plano.configs.iter().filter(|c| c.leva("factories")).map(|c| &c.xl).collect();
    match &s.json_falhou {
        Some(e) => v.extend(com_factories.into_iter().map(|x| (x.clone(), Motivo::NaoGravou(e.clone())))),
        None if !s.jsons.is_empty() && !com_factories.is_empty() => {
            v.extend(bwms_primitives::instalacao::xl_no_jogo::csvs_fora_do_jogo(plano, &recursos_montados_pelo_jogo(jogo, s, modio)));
        }
        None => {}
    }
    v
}

/// O hash de caminho de cada recurso que o jogo monta pelo próprio carregador antes de ler as configs (`InitializeArchives` antes do
/// `Configure`, `relatorios/L10.md` §2): os `.archive` da `Mods/` que estão na `BWMS-Mods` (o índice é o da origem, igual ao do clone), os de
/// `archive/Mac/mod` fora dela e os dos mods ligados do mod.io. Os de `content`/`ep1` ficam de fora: são ~95 MB de índice, e um `.csv` só lá
/// é o do próprio jogo, que ele já carrega.
fn recursos_montados_pelo_jogo(jogo: &Path, s: &super::pasta_nativa::Sincronia, modio: &ModsDoModIo) -> HashSet<u64> {
    let raiz = jogo.join(super::pasta_nativa::PASTA_NATIVA);
    let bwms = raiz.join(super::pasta_nativa::PASTA_DO_BWMS);
    let mut de_fora = Vec::new();
    let mut pendentes = vec![raiz];
    while let Some(dir) = pendentes.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let (p, Ok(tipo)) = (e.path(), e.file_type()) else { continue };
            if tipo.is_dir() && p != bwms {
                pendentes.push(p);
            } else if tipo.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("archive")) {
                de_fora.push(p);
            }
        }
    }
    let ligados = (0..modio.len()).filter(|&i| modio.ligados[i] == Some(true)).flat_map(|i| modio.archives[i].iter().cloned());
    s.nativos
        .iter()
        .map(|(origem, _, _)| origem.clone())
        .chain(de_fora)
        .chain(ligados)
        .filter_map(|a| crate::arquivos_do_jogo::hashes_do_archive(&a))
        .flatten()
        .collect()
}

/// Os `onscreens` que o jogo aplica neste boot, por id do `.xl` na tabela (`apply::xl_com_id`): o teste ao vivo (`localizacao::teste`) os
/// confere no `GetText`.
#[cfg(feature = "devlog")]
static TEXTOS_NO_JOGO: Mutex<Vec<(String, Vec<(String, Vec<String>)>)>> = Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) fn textos_no_jogo() -> Vec<(String, Vec<(String, Vec<String>)>)> {
    TEXTOS_NO_JOGO.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// O id de `xl` nas tabelas, como o apply o monta para a `Mods/`.
#[cfg(feature = "devlog")]
fn id_na_tabela(jogo: &Path, xl: &Path) -> String {
    apply::xl_com_id(&pasta(jogo), &[xl.to_path_buf()]).pop().map(|(id, _)| id).unwrap_or_default()
}

/// O veredito do boot sobre a parte dos `.xl` que o jogo aplica: os `.json` no disco, se as tabelas saíram sem a seção deles e quantas
/// linhas desses `.xl` a `bwms-localizacao.txt` ainda tem (0 = o BWMS não aplica a mesma seção), ou por que ficou tudo no BWMS.
#[cfg(feature = "devlog")]
fn veredito_do_jogo(jogo: &Path, s: &super::pasta_nativa::Sincronia, tabelas: Option<&Tabelas>, factories_sem_efeito: &[(PathBuf, Motivo)]) {
    let Some(Tabelas::Gravadas { tabelas: t, .. }) = tabelas else { return };
    let entregues: Vec<&bwms_primitives::instalacao::xl_no_jogo::ConfigDoJogo> =
        if t.no_jogo_entregue && !s.jsons.is_empty() { t.no_jogo.configs.iter().collect() } else { Vec::new() };
    let ids: Vec<String> = entregues.iter().map(|c| id_na_tabela(jogo, &c.xl)).collect();
    let com_textos = |c: &&bwms_primitives::instalacao::xl_no_jogo::ConfigDoJogo| c.leva("localization.onscreens");
    *TEXTOS_NO_JOGO.lock().unwrap_or_else(PoisonError::into_inner) =
        ids.iter().zip(&entregues).filter(|(_, c)| com_textos(c)).map(|(id, c)| (id.clone(), c.onscreens.clone())).collect();
    let tabela = std::fs::read_to_string(jogo.join(bwms_primitives::PASTA_DO_RUNTIME).join(bwms_primitives::tabelas::LOCALIZACAO.arquivo))
        .unwrap_or_default();
    let linhas = bwms_primitives::tabelas::ler(&tabela, bwms_primitives::tabelas::Localizacao::ler)
        .flatten()
        .filter(|l| ids.iter().zip(&entregues).any(|(id, c)| id == l.mod_id && com_textos(c)))
        .count();
    let motivo = t.no_jogo.motivo.as_ref().map(|m| m.texto().0).or_else(|| s.json_falhou.clone()).unwrap_or_else(|| "-".into());
    let secoes: Vec<&str> =
        bwms_primitives::instalacao::xl_no_jogo::SECOES_NO_JOGO.into_iter().filter(|k| entregues.iter().any(|c| c.leva(k))).collect();
    let sem_efeito: Vec<String> = factories_sem_efeito.iter().map(|(x, m)| format!("{} ({})", id_na_tabela(jogo, x), m.texto().0)).collect();
    let lista = |v: Vec<String>| if v.is_empty() { "-".to_string() } else { v.join("; ") };
    crate::veredito_dev!(
        "[mods] xl no jogo: {} .json em BWMS-Mods/xl ({}); entregue={}; linhas desses .xl na bwms-localizacao.txt: {linhas}; fica no BWMS: {motivo}; \
         seções no jogo: {}; factories sem efeito: {}",
        entregues.len(),
        ids.join("; "),
        !entregues.is_empty(),
        if secoes.is_empty() { "-".to_string() } else { secoes.join(", ") },
        lista(sem_efeito)
    );
}

/// No hook do `InitializeArchives`, depois da original: confere no depot quais clones da pasta nativa o jogo montou. O que ficou fora (o que a
/// sincronização não pôs e o clone que o jogo não montou) vai para o relatório com o motivo e a ação, na ordem de carga da `Mods/`; os
/// conflitos entre os montados e os dos mods ligados do mod.io saem numa thread: ler o índice de cada archive não atrasa a thread do jogo.
pub(crate) unsafe fn conferir_montagem(depot: *mut u8) {
    let da_pasta = {
        let e = estado();
        e.relatorio.as_ref().and_then(|r| {
            let ordem: Vec<PathBuf> = r.mods.iter().flat_map(|m| m.archives.iter().cloned()).collect();
            let s = r.nativa.as_ref()?;
            let mut scopes: Vec<u32> = Vec::new();
            let fora = super::pasta_nativa::fora_do_jogo(&ordem, s, |destino| {
                let scope = crate::arquivos_do_jogo::scope_do_archive_no_depot(depot, &super::pasta_nativa::sufixo_no_depot(destino));
                scopes.extend(scope);
                scope.is_some()
            });
            Some((s.recusados.clone(), fora, s.nativos.len(), scopes))
        })
    };
    // Os archives dos mods ligados do mod.io no depot, pelo mesmo sufixo (`<id>/archive/<arquivo>`): prova de dev de que o jogo os montou e em
    // que scope. A luz não depende disto: a forma do caminho do mod.io no depot é [UNCONFIRMED] (z029 `MODIO.md` §3). Só no dev: o build
    // público não lê o depot para isso.
    #[cfg(feature = "devlog")]
    {
        let modio_no_depot: Vec<String> = estado()
            .modio
            .archives
            .iter()
            .flatten()
            .map(|a| {
                let scope = crate::arquivos_do_jogo::scope_do_archive_no_depot(depot, &super::pasta_nativa::sufixo_no_depot(a));
                format!("{}={}", a.file_name().unwrap_or_default().to_string_lossy(), scope.map_or("fora".to_string(), |s| format!("scope {s}")))
            })
            .collect();
        crate::veredito_dev!("[mods] mod.io no depot: {} archive(s): {}", modio_no_depot.len(), modio_no_depot.join("; "));
    }
    if let Some((recusados, fora, nativos, scopes)) = da_pasta {
        crate::veredito_dev!(
            "[mods] pasta nativa: {}/{nativos} montado(s) pelo jogo (scopes {scopes:?}); fora do jogo: {} ({}); recusados: {}",
            scopes.len(),
            fora.len(),
            fora.iter().map(|(p, m)| format!("{}: {}", p.display(), m.texto().0)).collect::<Vec<_>>().join("; "),
            recusados.len()
        );
        let mut e = estado();
        if let Some(r) = e.relatorio.as_mut() {
            r.recusados = Some(recusados);
            r.fora_do_jogo = fora;
        }
        publicar(&e);
    }
    std::thread::spawn(|| {
        let t0 = std::time::Instant::now();
        // A ordem do jogo: a `Mods/` (na `BWMS-Mods`, dentro de `archive/Mac/mod`) antes do mod.io, que entra no fim do mesmo set.
        let mut ordem = {
            let e = estado();
            e.relatorio.as_ref().map(Relatorio::aceitos).unwrap_or_default()
        };
        ordem.extend(estado().modio.na_ordem_de_carga());
        let achados = conflitos(&ordem);
        crate::veredito_dev!(
            "[mods] conflitos: {} par(es) entre {} mod(s) dos 2 caminhos ({}); índices lidos em {} us",
            achados.len(),
            ordem.len(),
            achados.iter().map(|c| format!("{} vence {} em {}", c.vence, c.perde, c.recursos)).collect::<Vec<_>>().join("; "),
            t0.elapsed().as_micros()
        );
        let mut e = estado();
        e.conflitos = Some(achados);
        publicar(&e);
    });
}

/// A cor de um mod para o selo do canto e a aba Mods: `Verde` entrou inteiro, `Amarelo` entrou em parte, `Vermelho` não funciona. A ordem é a
/// de gravidade: `EstadosDosMods::marcar` nunca melhora uma cor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub(crate) enum CorDoMod {
    Verde = 0,
    Amarelo = 1,
    Vermelho = 2,
}

impl CorDoMod {
    /// O nome gravado no registro da compilação (`red4ext/bwms-scripts-assinatura.txt`).
    pub(crate) fn nome(self) -> &'static str {
        match self {
            Self::Verde => "verde",
            Self::Amarelo => "amarelo",
            Self::Vermelho => "vermelho",
        }
    }

    pub(crate) fn do_nome(nome: &str) -> Option<Self> {
        [Self::Verde, Self::Amarelo, Self::Vermelho].into_iter().find(|c| c.nome() == nome)
    }
}

/// O estado de cada mod, coluna por coluna (índice `i` = um mod, na ordem de carga): `raizes[i]` é a pasta de 1º nível da `Mods/` (ou o
/// arquivo solto nela, ou o item do manifesto dos plugins: a `raiz` de `scripts::carregador::ModDeScripts`), `cores[i]` a cor e
/// `motivos[i]` o porquê, uma linha `"<pt> / <en>"` por motivo separadas por `\n` (vazio no verde). É o que a etapa 3 (selo e aba Mods) lê.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct EstadosDosMods {
    pub(crate) raizes: Vec<PathBuf>,
    pub(crate) cores: Vec<CorDoMod>,
    pub(crate) motivos: Vec<String>,
}

impl EstadosDosMods {
    /// O índice de `raiz`; o mod que não está entra verde, no fim.
    fn indice(&mut self, raiz: &Path) -> usize {
        if let Some(i) = self.raizes.iter().position(|r| r == raiz) {
            return i;
        }
        self.raizes.push(raiz.to_path_buf());
        self.cores.push(CorDoMod::Verde);
        self.motivos.push(String::new());
        self.raizes.len() - 1
    }

    /// Põe `raiz` com pelo menos a cor `cor` e acrescenta `motivo` (vazio ou já presente não acrescenta).
    pub(crate) fn marcar(&mut self, raiz: &Path, cor: CorDoMod, motivo: &str) {
        let i = self.indice(raiz);
        self.cores[i] = self.cores[i].max(cor);
        if !motivo.is_empty() && !self.motivos[i].split('\n').any(|l| l == motivo) {
            if !self.motivos[i].is_empty() {
                self.motivos[i].push('\n');
            }
            self.motivos[i].push_str(motivo);
        }
    }

    pub(crate) fn cor(&self, raiz: &Path) -> Option<CorDoMod> {
        self.raizes.iter().position(|r| r == raiz).map(|i| self.cores[i])
    }
}

/// O estado de cada mod da `Mods/` (e de cada item do manifesto dos plugins com `.reds`) neste boot. Vermelho: os `.reds` dele ficaram fora
/// da compilação (erro do `scc` ou native sem registro, `scripts::compilacao_no_lancamento`), ou o mod só traz o que o Mac não carrega (`.dll`,
/// `.lua`). Amarelo: o que o relatório já põe como recusado, fora ou aviso naquele mod (archive, tweak, `.xl`, patch de recurso que o depot não
/// tem, conflito que o compilador avisa, recurso de `.archive` que perde para outro mod). O resto, verde. Lido sob demanda: antes da carga
/// dos `.archive` os recusados e os conflitos deles ainda não entram.
pub(crate) fn estados_dos_mods() -> EstadosDosMods {
    luzes_de(&estado())
}

fn estados_de(r: Option<&Relatorio>, scripts: &EstadosDosMods, patches_fora: &[(String, String)]) -> EstadosDosMods {
    let mut e = EstadosDosMods::default();
    if let Some(r) = r {
        let (xl_fora, avisos, sem_efeito, secoes_sem_efeito): (&[PathBuf], &[String], &[(PathBuf, String)], &[(PathBuf, String)]) = match &r.tabelas {
            Tabelas::Gravadas { tabelas, avisos } => (&tabelas.xl_fora, avisos, &tabelas.sem_efeito_no_bwms, &tabelas.secoes_sem_efeito),
            _ => (&[], &[], &[], &[]),
        };
        let amarelo = CorDoMod::Amarelo;
        for m in &r.mods {
            let raiz = r.raiz.join(&m.nome);
            let rel = |p: &Path| p.strip_prefix(&r.raiz).unwrap_or(p).display().to_string();
            e.marcar(&raiz, CorDoMod::Verde, "");
            for (p, motivo) in r.recusados.iter().flatten().filter(|(p, _)| m.archives.contains(p)) {
                let (pt, en) = motivo.texto();
                e.marcar(&raiz, amarelo, &format!("recusado / refused: {} — {pt} / {en}", rel(p)));
            }
            for (p, motivo) in r.fora_do_jogo.iter().filter(|(p, _)| m.archives.contains(p)) {
                let (pt, en) = motivo.texto();
                e.marcar(&raiz, amarelo, &format!("não entrou / not loaded: {} — {pt} / {en}", rel(p)));
            }
            for (p, _) in r.tweaks_recusados.iter().filter(|(p, _)| m.tweaks.contains(p)) {
                e.marcar(&raiz, amarelo, &format!("recusado / refused: {} — o tweak não foi lido / the tweak could not be read", rel(p)));
            }
            for x in &m.xls {
                let arquivo = format!("{PASTA_MODS}/{}: ", rel(x));
                if xl_fora.contains(x) {
                    e.marcar(&raiz, amarelo, &format!("recusado / refused: {} — o .xl não foi lido / the .xl could not be read", rel(x)));
                } else if avisos.iter().any(|a| a.starts_with(&arquivo)) {
                    e.marcar(&raiz, amarelo, &format!("aviso / warning: {} — parte do .xl ficou sem efeito / part of the .xl has no effect", rel(x)));
                }
                if sem_efeito.iter().chain(secoes_sem_efeito).any(|(p, _)| p == x) {
                    e.marcar(
                        &raiz,
                        amarelo,
                        &format!("aviso / warning: {} — o BWMS ainda não aplica parte deste .xl / BWMS does not apply part of this .xl yet", rel(x)),
                    );
                }
                // O `factories` só tem o caminho do jogo: o que ele não aplica fica sem efeito, e o mod entra em parte.
                for (_, motivo) in r.factories_sem_efeito.iter().filter(|(p, _)| p == x) {
                    let (pt, en) = motivo.texto();
                    e.marcar(&raiz, amarelo, &format!("aviso / warning: {} — itens (factories) sem efeito: {pt} / items (factories) have no effect: {en}", rel(x)));
                }
            }
            // `.dll` não carrega no Mac: o mod que só traz isso não funciona; o que traz mais entra em parte.
            if m.dlls > 0 {
                let so_isso = m.archives.is_empty() && m.xls.is_empty() && m.tweaks.is_empty() && m.reds.is_empty() && m.luas == 0;
                e.marcar(
                    &raiz,
                    if so_isso { CorDoMod::Vermelho } else { amarelo },
                    "fora / left out: .dll (plugin de Windows) não carrega no Mac / .dll (Windows plugin) does not load on a Mac",
                );
            }
        }
        // O id do `.xl` da `Mods/` na tabela é `Mods/<mod>/<caminho sem .xl>` (`id_na_tabela`).
        for (id, patch) in patches_fora {
            let Some(nome) = id.strip_prefix(&format!("{PASTA_MODS}/")).and_then(|resto| resto.split('/').next()) else { continue };
            if r.mods.iter().any(|m| m.nome == nome) {
                e.marcar(
                    &r.raiz.join(nome),
                    amarelo,
                    &format!("aviso / warning: o .xl aponta um recurso que o jogo não tem: {patch} / the .xl points to a resource the game does not have: {patch}"),
                );
            }
        }
    }
    for i in 0..scripts.raizes.len() {
        for motivo in scripts.motivos[i].split('\n') {
            e.marcar(&scripts.raizes[i], scripts.cores[i], motivo);
        }
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pasta vazia em `temp_dir` para o teste; ele a apaga por caminho no fim.
    fn pasta_de_teste(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-pasta-mods-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn escrever_em(raiz: &Path, arquivos: &[(&str, &[u8])]) {
        for (caminho, conteudo) in arquivos {
            let p = raiz.join(caminho);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, conteudo).unwrap();
        }
    }

    fn relativos(raiz: &Path, v: &[PathBuf]) -> Vec<String> {
        v.iter().map(|p| p.strip_prefix(raiz).unwrap().to_string_lossy().into_owned()).collect()
    }

    /// `raiz` lida como a `Mods/` de um jogo.
    fn ler(raiz: &Path) -> PastaMods {
        PastaMods { raiz: raiz.to_path_buf(), arquivos: apply::arquivos_da_pasta_mods(raiz) }
    }

    #[test]
    fn archives_acha_em_qualquer_profundidade_e_ordena_como_o_windows() {
        let raiz = pasta_de_teste("archives");
        escrever_em(
            &raiz,
            &[
                ("Mod B/b.archive", b"x"),
                ("Mod A/archive/pc/mod/a.archive", b"x"),
                ("Mod A/archive/pc/mod/a.archive.xl", b"x"),
                ("Mod A/r6/scripts/a.reds", b"x"),
                ("Mod C/CAIXA.ARCHIVE", b"x"),
                (".oculto/o.archive", b"x"),
                ("Mod D/.escondido.archive", b"x"),
            ],
        );
        assert_eq!(relativos(&raiz, &archives(&raiz)), ["Mod A/archive/pc/mod/a.archive", "Mod B/b.archive", "Mod C/CAIXA.ARCHIVE"]);
        assert!(archives(&raiz.join("nao-existe")).is_empty());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// A lista da compilação do lançamento: todo `.reds`, em ordem de caminho, sem nome com ponto e sem seguir link. `zz.REDS` fica fora: o
    /// `scc` compara a extensão exata.
    #[test]
    fn reds_em_ordem_de_caminho_com_as_exclusoes_dos_archives() {
        let raiz = pasta_de_teste("reds");
        escrever_em(
            &raiz,
            &[
                ("Mod B/r6/scripts/b.reds", b"x"),
                ("Mod A/r6/scripts/z/zz.REDS", b"x"),
                ("Mod A/r6/scripts/z/yy.reds", b"x"),
                ("Mod A/r6/scripts/a.reds", b"x"),
                ("solto.reds", b"x"),
                ("Mod A/.git/x.reds", b"x"),
                ("Mod A/.pulado.reds", b"x"),
                ("Mod A/leia.txt", b"x"),
            ],
        );
        let fora = pasta_de_teste("reds-fora");
        escrever_em(&fora, &[("de-fora.reds", b"x")]);
        std::os::unix::fs::symlink(&fora, raiz.join("Mod C")).unwrap();
        std::os::unix::fs::symlink(fora.join("de-fora.reds"), raiz.join("Mod B/link.reds")).unwrap();
        assert_eq!(relativos(&raiz, &reds(&raiz)), ["Mod A/r6/scripts/a.reds", "Mod A/r6/scripts/z/yy.reds", "Mod B/r6/scripts/b.reds", "solto.reds"]);
        let _ = std::fs::remove_dir_all(&raiz);
        let _ = std::fs::remove_dir_all(&fora);
    }

    #[test]
    fn tipo_do_arquivo_pelo_caminho_dentro_da_pasta() {
        let t = |s: &str| tipo(Path::new(s));
        assert_eq!(t("M/archive/pc/mod/x.archive"), Some(Tipo::Archive));
        assert_eq!(t("M/archive/pc/mod/x.archive.xl"), Some(Tipo::Xl));
        assert_eq!(t("M/solto.XL"), Some(Tipo::Xl));
        assert_eq!(t("M/r6/scripts/x.reds"), Some(Tipo::Reds));
        assert_eq!(t("M/r6/scripts/X.REDS"), None, "o scc compara a extensão exata");
        assert_eq!(t("M/r6/scripts/x.Reds"), None);
        assert_eq!(t("M/bin/x64/plugins/cyber_engine_tweaks/mods/m/init.lua"), Some(Tipo::Lua));
        assert_eq!(t("M/red4ext/plugins/p/p.DLL"), Some(Tipo::Dll));
        // Tweak: debaixo de r6/tweaks (em qualquer caixa) ou solto fora das árvores do jogo.
        assert_eq!(t("M/r6/tweaks/autor/x.yaml"), Some(Tipo::Tweak));
        assert_eq!(t("M/R6/Tweaks/x.tweak"), Some(Tipo::Tweak));
        assert_eq!(t("r6/tweaks/x.yml"), Some(Tipo::Tweak));
        assert_eq!(t("M/x.yaml"), Some(Tipo::Tweak));
        assert_eq!(t("M/extras/x.yaml"), Some(Tipo::Tweak));
        assert_eq!(t("x.yaml"), Some(Tipo::Tweak));
        assert_eq!(t("M/bin/x64/plugins/cyber_engine_tweaks/mods/m/config.yaml"), None);
        assert_eq!(t("M/red4ext/plugins/p/config.yaml"), None);
        assert_eq!(t("M/r6/config/x.yaml"), None);
        assert_eq!(t("M/archive/pc/mod/x.yaml"), None);
        // Fora da lista: não vira linha.
        assert_eq!(t("M/r6/config/redsUserHints/x.toml"), None);
        assert_eq!(t("M/leia.md"), None);
        assert_eq!(t("M/sem-extensao"), None);
    }

    /// Cada pasta de 1º nível (ou arquivo solto) é um mod, na ordem de carga; o relatório na raiz não conta.
    #[test]
    fn varredura_agrupa_por_pasta_de_primeiro_nivel_na_ordem_de_carga() {
        let raiz = pasta_de_teste("varredura");
        escrever_em(
            &raiz,
            &[
                ("Mod A/archive/pc/mod/a.archive", b"x"),
                ("Mod A/archive/pc/mod/a.archive.xl", b"x"),
                ("Mod A/r6/tweaks/a.yaml", b"x"),
                ("Mod A b/b.archive", b"x"),
                ("Mod A/r6/scripts/a.reds", b"x"),
                ("CET/bin/x64/plugins/cyber_engine_tweaks/mods/c/init.lua", b"x"),
                ("CET/bin/x64/plugins/cyber_engine_tweaks/mods/c/ui.lua", b"x"),
                ("Plug/red4ext/plugins/p/p.dll", b"x"),
                ("Leia/leia.txt", b"x"),
                ("solto.archive", b"x"),
                (RELATORIO, b"velho"),
            ],
        );
        let mods = varrer(&ler(&raiz));
        let nomes: Vec<&str> = mods.iter().map(|m| m.nome.as_str()).collect();
        // "Mod A b/" vem antes de "Mod A/" em bytes (' ' < '/'): é também a ordem de carga dos archives.
        assert_eq!(nomes, ["CET", "Leia", "Mod A b", "Mod A", "Plug", "solto.archive"]);
        let a = &mods[3];
        assert_eq!((a.archives.len(), a.xls.len(), a.tweaks.len(), a.reds.len(), a.luas, a.dlls), (1, 1, 1, 1, 0, 0));
        assert_eq!((mods[0].luas, mods[4].dlls, mods[5].archives.len()), (2, 1, 1));
        assert_eq!(mods[1], ModDaPasta { nome: "Leia".into(), ..ModDaPasta::default() });
        let em_ordem: Vec<PathBuf> = mods.iter().flat_map(|m| m.archives.clone()).collect();
        assert_eq!(em_ordem, archives(&raiz), "a ordem dos mods é a de carga dos archives");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// A leitura do boot serve a lista da compilação (`reds`) e o `preparar` sem andar a pasta de novo; o `preparar` a tira, e depois quem
    /// pede a lista anda a pasta. A leitura de uma `Mods/` não serve a outra pasta.
    #[test]
    fn leitura_do_boot_serve_a_compilacao_e_o_preparar_uma_vez() {
        let jogo = pasta_de_teste("leitura-do-boot");
        let raiz = pasta(&jogo);
        escrever_em(&raiz, &[("A/r6/scripts/a.reds", b"x"), ("A/a.archive", b"x")]);
        *leitura_do_boot() = Some(apply::ler_pasta_mods(&jogo));
        // O que entra depois da leitura fica fora da lista do boot.
        escrever_em(&raiz, &[("B/b.reds", b"x")]);
        assert_eq!(relativos(&raiz, &reds(&raiz)), ["A/r6/scripts/a.reds"]);
        let outra = pasta_de_teste("leitura-do-boot-outra");
        escrever_em(&outra, &[("c.reds", b"x")]);
        assert_eq!(relativos(&outra, &reds(&outra)), ["c.reds"]);
        let r = preparar(&jogo).expect("relatório");
        assert_eq!(r.mods.iter().map(|m| m.nome.as_str()).collect::<Vec<_>>(), ["A"]);
        assert!(leitura_do_boot().is_none(), "o preparar tira a leitura do boot");
        assert_eq!(relativos(&raiz, &reds(&raiz)), ["A/r6/scripts/a.reds", "B/b.reds"]);
        let _ = std::fs::remove_dir_all(&jogo);
        let _ = std::fs::remove_dir_all(&outra);
    }

    /// `RDAR` v12 com o índice em 4096 e um recurso por hash.
    fn archive_com(hashes: &[u64]) -> Vec<u8> {
        let mut indice = vec![0u8; 16];
        indice.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
        indice.extend_from_slice(&[0u8; 8]);
        for h in hashes {
            indice.extend_from_slice(&h.to_le_bytes());
            indice.extend_from_slice(&[0u8; bwms_primitives::rdar::TAMANHO_DO_RECURSO - 8]);
        }
        let mut b = vec![0u8; 4096];
        b[..4].copy_from_slice(b"RDAR");
        b[4..8].copy_from_slice(&12u32.to_le_bytes());
        b[8..16].copy_from_slice(&4096u64.to_le_bytes());
        b[16..20].copy_from_slice(&(indice.len() as u32).to_le_bytes());
        b.extend_from_slice(&indice);
        b
    }

    /// Os 2 caminhos pelo índice dos archives: a `Mods/` vence o mod.io (a cópia do FairerCourierTimers do roteiro), e entre dois do mod.io
    /// vence o de id menor. O relatório diz quem vence e o que fazer; o mod.io que perde fica amarelo com o nome de quem vence; o bloco de cada
    /// mod da `Mods/` traz a luz e o motivo que vem de fora dele (a compilação), sem caminho da máquina; a seção do mod.io lista cada um.
    #[test]
    fn conflitos_e_luzes_dos_dois_caminhos_no_relatorio() {
        let jogo = pasta_de_teste("dois-caminhos");
        let raiz = pasta(&jogo);
        let fct = 0xffa23560ec50cdc4u64;
        escrever_em(
            &raiz,
            &[
                ("BWMS Teste ModIO/archive/pc/mod/FairerCourierTimers.archive", &archive_com(&[fct])),
                ("Quebrado/r6/scripts/q.reds", b"// q"),
            ],
        );
        let (lcdt, outro) = (jogo.join("modio/mods/6428265"), jogo.join("modio/mods/6500000"));
        escrever_em(
            &jogo,
            &[("modio/mods/6428265/archive/FairerCourierTimers.archive", &archive_com(&[fct, 7])), ("modio/mods/6500000/archive/b.archive", &archive_com(&[7]))],
        );
        let enabled = jogo.join("EnabledMods.json");
        let item = |p: &Path, id: u32, nome: &str| {
            format!(
                "{{\"modPath\": {}, \"modID\": {id}, \"modName\": {}}}",
                bwms_primitives::json::texto(&p.display().to_string()),
                bwms_primitives::json::texto(nome)
            )
        };
        std::fs::write(&enabled, format!("{{\"enabled_mods\": [{}, {}]}}", item(&outro, 6500000, "Outro"), item(&lcdt, 6428265, "Longer Car Delivery Timers"))).unwrap();
        let modio = super::super::mod_io::ler_em(&jogo.join("modio/mods"), &enabled);
        let mods = varrer(&ler(&raiz));
        let mut ordem: Vec<(Caminho, String, Vec<PathBuf>)> = mods.iter().map(|m| (Caminho::Mods, m.nome.clone(), m.archives.clone())).collect();
        ordem.extend(modio.na_ordem_de_carga());
        let cs = conflitos(&ordem);
        let (lc, ou) = ("Longer Car Delivery Timers (mod.io 6428265)", "Outro (mod.io 6500000)");
        let pares: Vec<(&str, &str, usize, Caminho, Caminho)> = cs.iter().map(|c| (c.vence.as_str(), c.perde.as_str(), c.recursos, c.vence_no, c.perde_no)).collect();
        assert_eq!(pares, [("BWMS Teste ModIO", lc, 1, Caminho::Mods, Caminho::ModIo), (lc, ou, 1, Caminho::ModIo, Caminho::ModIo)]);

        let mut r = relatorio_de(&raiz, mods);
        r.recusados = Some(Vec::new());
        let mut scripts = EstadosDosMods::default();
        let erro = format!("erro / error: {}:5: [UNRESOLVED_FN] f", raiz.join("Quebrado/r6/scripts/q.reds").display());
        scripts.marcar(&raiz.join("Quebrado"), CorDoMod::Vermelho, &erro);
        let luzes = luzes_com(Some(&r), &scripts, &[], Some(&cs));
        let modio = modio.com_perdas(&cs);
        let texto = texto_do_relatorio(&r, &Sabido { luzes: &luzes, conflitos: Some(&cs), modio: &modio }, &[]);
        for linha in [
            "[BWMS Teste ModIO]\nluz / light: verde / green\nentrou / loaded: 1 .archive\n".to_string(),
            "[Quebrado]\nluz / light: vermelho / red (não funciona / does not work)\n  motivo / reason: erro / error: \
             Mods/Quebrado/r6/scripts/q.reds:5: [UNRESOLVED_FN] f\n"
                .to_string(),
            format!(
                "BWMS Teste ModIO (Mods) x {lc} (mod.io): 1 recurso(s) em comum, vence BWMS Teste ModIO: no .archive a pasta Mods vence o mod.io; \
                 para o mod.io vencer, tire o mod da pasta Mods / 1 shared resource(s), BWMS Teste ModIO wins: in archives the Mods folder beats \
                 mod.io; for the mod.io mod to win, take the mod out of the Mods folder\n"
            ),
            format!("{lc} (mod.io) x {ou} (mod.io): 1 recurso(s) em comum, vence {lc}: entre mods do mod.io vence o de id menor"),
            format!(
                "{lc}: luz / light: amarelo / yellow (entrou em parte / partly loaded)\n  motivo / reason: perde 1 recurso(s) para BWMS Teste \
                 ModIO (Mods) no .archive / loses 1 resource(s) to BWMS Teste ModIO (Mods) in archives\n"
            ),
            format!("{ou}: luz / light: amarelo / yellow (entrou em parte / partly loaded)\n  motivo / reason: perde 1 recurso(s) para {lc} (mod.io)"),
        ] {
            assert!(texto.contains(&linha), "falta {linha:?} em:\n{texto}");
        }
        assert!(!texto.contains(&jogo.display().to_string()), "caminho da máquina no relatório:\n{texto}");
        assert!(texto.contains("[mod.io]\nO jogo carrega estes mods sozinho"), "{texto}");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// A luz de cada mod e os conflitos que a lane M do roteiro deve ver, medidos fora do jogo sobre uma cópia da `Mods/` de teste e o mod.io
    /// real da máquina (só leitura): `BWMS_MEDIDA_JOGO=<jogo falso com Mods/> cargo test --lib luzes_da_mods_de_teste -- --ignored --nocapture`.
    /// Sem a compilação dos scripts (o estado dela é o do boot) e sem a carga dos archives (todo archive válido conta como montado).
    #[test]
    #[ignore]
    fn luzes_da_mods_de_teste() {
        let Some(jogo) = std::env::var_os("BWMS_MEDIDA_JOGO").map(PathBuf::from) else { return };
        let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
        let r = preparar(&jogo).expect("o jogo falso tem Mods/");
        let modio = super::super::mod_io::ler(&home);
        let mut ordem = r.aceitos();
        ordem.extend(modio.na_ordem_de_carga());
        let cs = conflitos(&ordem);
        let luzes = luzes_com(Some(&r), &EstadosDosMods::default(), &[], Some(&cs));
        let modio = modio.com_perdas(&cs);
        super::super::lista_dos_mods::publicar(super::super::lista_dos_mods::montar(Some(&r.raiz), &luzes, &modio, &cs));
        println!("{}", super::super::lista_dos_mods::veredito());
        for i in 0..luzes.raizes.len() {
            println!("  {} = {:?}: {}", luzes.raizes[i].display(), luzes.cores[i], luzes.motivos[i].replace('\n', " | "));
        }
    }

    /// O 1º na ordem de carga vence; recurso repetido dentro do mesmo mod não conta; archive inválido não disputa nada.
    #[test]
    fn conflitos_contam_por_par_e_o_primeiro_vence() {
        let raiz = pasta_de_teste("conflitos");
        escrever_em(
            &raiz,
            &[
                ("A/a1.archive", &archive_com(&[1, 2, 3])),
                ("A/a2.archive", &archive_com(&[3, 4])),
                ("B/b.archive", &archive_com(&[2, 3, 4, 9])),
                ("C/c.archive", &archive_com(&[4, 5])),
                ("D/d.archive", b"isto nao e um archive"),
            ],
        );
        let mods: Vec<(Caminho, String, Vec<PathBuf>)> = varrer(&ler(&raiz)).into_iter().map(|m| (Caminho::Mods, m.nome, m.archives)).collect();
        let c = conflitos(&mods);
        let pares: Vec<(&str, &str, usize)> = c.iter().map(|c| (c.vence.as_str(), c.perde.as_str(), c.recursos)).collect();
        assert_eq!(pares, [("A", "B", 3), ("A", "C", 1)]);
        assert_eq!(crate::arquivos_do_jogo::hashes_do_archive(&raiz.join("A/a2.archive")), Some(vec![3, 4]));
        assert_eq!(crate::arquivos_do_jogo::hashes_do_archive(&raiz.join("D/d.archive")), None);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// O boot regrava pelo escritor do motor a cada abertura: sem `.xl` as tabelas saem vazias, com `.xl` são gravadas, e a tabela que não
    /// grava vira o texto do erro para o relatório.
    #[test]
    fn tabelas_do_boot_pelo_escritor_do_motor() {
        let jogo = pasta_de_teste("tabelas");
        escrever_em(&pasta(&jogo), &[("Sem Xl/x.archive", b"x")]);
        let Tabelas::Gravadas { tabelas, .. } = gravar_tabelas(&jogo, &apply::ler_pasta_mods(&jogo)) else { panic!("tabelas não gravadas") };
        assert_eq!((tabelas.xl_lidos, tabelas.reslink_pairs), (0, 0));

        escrever_em(&pasta(&jogo), &[("Com Xl/archive/pc/mod/c.archive.xl", b"resource:\n  link:\n    base\\a.ent: modc\\a.ent\n")]);
        let Tabelas::Gravadas { tabelas, .. } = gravar_tabelas(&jogo, &apply::ler_pasta_mods(&jogo)) else { panic!("tabelas não gravadas") };
        assert_eq!((tabelas.xl_lidos, tabelas.reslink_pairs), (1, 1));

        std::fs::remove_file(jogo.join("red4ext/bwms-reslink.txt")).unwrap();
        std::fs::create_dir_all(jogo.join("red4ext/bwms-reslink.txt/ocupado")).unwrap();
        let Tabelas::Falhou(erro) = gravar_tabelas(&jogo, &apply::ler_pasta_mods(&jogo)) else { panic!("tabela que não grava devia falhar") };
        assert!(erro.contains("bwms-reslink.txt") && erro.contains("abra o jogo de novo"), "{erro}");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Do `.xl` ao relatório, pelo parse e pelo escritor do motor: o `context` do `resource.fix`, a seção `animations` e a seção que o
    /// parser não reconhece saem como limite do BWMS, sem mandar corrigir; a entrada em formato inválido do mesmo `.xl` manda.
    #[test]
    fn limite_do_bwms_no_xl_vai_ao_relatorio_sem_mandar_corrigir() {
        let jogo = pasta_de_teste("limites-do-xl");
        escrever_em(
            &pasta(&jogo),
            &[(
                "M/m.xl",
                b"resource:\n  fix:\n    a.mesh:\n      names:\n        m1: m2\n      context:\n        p: v\n\
                  animations:\n  - entity: player_ma_tpp\n    set: m\\anims.anims\ncustomNodes: x\nfactories:\n  a: b\n",
            )],
        );
        let texto = texto_do_boot(&jogo, &[]).expect("relatório");
        let corrigir = "aviso / warning: m.xl — parte do .xl ficou sem efeito; corrija o arquivo e abra o jogo de novo / part of the .xl has \
                        no effect; fix the file and start the game again\n  detalhe / detail: factories: formato inválido (parte ignorada); \
                        use um valor ou uma lista de valores\naviso / warning: m.xl — o BWMS ainda não aplica parte deste .xl";
        assert!(texto.contains("[M]\nentrou / loaded: 1 .xl\n"), "{texto}");
        assert!(texto.contains(corrigir), "{texto}");
        let limites = texto.split_once("o BWMS ainda não aplica parte deste .xl").map(|(_, d)| d).unwrap_or_default();
        let detalhes: Vec<&str> = limites.lines().filter_map(|l| l.strip_prefix("  detalhe / detail: ")).collect();
        // O `context` do `resource.fix` vai ao estado de mesh nativo (S12): não é mais limite.
        assert_eq!(detalhes.len(), 2, "{texto}");
        assert_eq!(detalhes[0], "animations ×1: o BWMS ainda não aplica esta seção (sem efeito no jogo); nada a corrigir no arquivo");
        assert_eq!(detalhes[1], "customNodes: seção que o BWMS não reconhece (sem efeito no jogo); confira o nome da seção, e se o mod roda no Windows não há o que corrigir");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// O tweak que o parser da aplicação recusa sai com o erro; o bom e o `.reds` não.
    #[test]
    fn tweak_que_nao_parseia_e_recusado_na_varredura() {
        let raiz = pasta_de_teste("tweaks-recusados");
        escrever_em(
            &raiz,
            &[
                ("A/r6/tweaks/bom.yaml", b"Items.X:\n  quality: Quality.Rare\n"),
                ("A/r6/tweaks/tab.yaml", b"Items.X:\n\tquality: Quality.Rare\n"),
                ("solto.tweak", b"isto nao e tweak (\n"),
            ],
        );
        let recusados = tweaks_recusados(&varrer(&ler(&raiz)));
        let nomes: Vec<String> = recusados.iter().map(|(p, _)| p.strip_prefix(&raiz).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(nomes, ["A/r6/tweaks/tab.yaml", "solto.tweak"]);
        assert!(recusados[0].1.starts_with("o YAML não parseou: linha 2"), "{recusados:?}");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// O mod de teste que o integrador copia para a `Mods/` (`teste-mods/BWMS Teste TweakXL`): os dois arquivos são tweaks do mod, na ordem
    /// de caminho (o `.tweak` antes do `.yaml`, que clona depois do que o `.tweak` gravou), viram as operações que o roteiro confere pelo parser
    /// da aplicação, e o relatório diz que foram lidos, sem recusa. Lido do disco na execução: o arquivo copiado é o arquivo testado. O mod
    /// é de dev e fica fora do export público (`dist/export-github.sh`); sem a pasta, pula. Com a pasta, arquivo faltando é falha.
    #[test]
    fn mod_de_teste_do_tweakxl_entra_pela_pasta_mods() {
        let tweaks_do_mod = Path::new(env!("CARGO_MANIFEST_DIR")).join("../teste-mods/BWMS Teste TweakXL/r6/tweaks");
        if !tweaks_do_mod.is_dir() {
            eprintln!("mod de teste ausente em {}: teste pulado", tweaks_do_mod.display());
            return;
        }
        let do_mod = |nome: &str| std::fs::read(tweaks_do_mod.join(nome)).expect("arquivo do mod de teste");
        let (fonte_yaml, fonte_tweak) = (do_mod("bwms-teste-pasta-mods.yaml"), do_mod("bwms-teste-pasta-mods.tweak"));
        let jogo = pasta_de_teste("mod-de-teste-tweakxl");
        let raiz = pasta(&jogo);
        let (yaml, tweak) =
            ("BWMS Teste TweakXL/r6/tweaks/bwms-teste-pasta-mods.yaml", "BWMS Teste TweakXL/r6/tweaks/bwms-teste-pasta-mods.tweak");
        escrever_em(&raiz, &[(yaml, fonte_yaml.as_slice()), (tweak, fonte_tweak.as_slice())]);

        assert_eq!(relativos(&raiz, &tweaks(&raiz)), [tweak, yaml]);
        let mods = varrer(&ler(&raiz));
        assert_eq!(mods.len(), 1);
        assert_eq!((mods[0].nome.as_str(), mods[0].tweaks.len()), ("BWMS Teste TweakXL", 2));
        assert!(tweaks_recusados(&mods).is_empty());

        use bwms_primitives::operacoes::{EditOp, Op};
        let ops = |p: &Path| -> Vec<String> {
            crate::tweakdb::ops_do_arquivo(&p.to_string_lossy(), || false)
                .expect("o parser da aplicação aceita o arquivo")
                .iter()
                .map(|o| match o {
                    Op::Clone { record, base } => format!("clone {record} <- {base}"),
                    Op::Create { record, class } => format!("create {record} {class}"),
                    Op::Edit { flat, op: EditOp::Assign(v) } => format!("{flat} = {v}"),
                    Op::Criar { flat, tipo, op: EditOp::Assign(v) } => format!("{flat}: {tipo} = {v}"),
                    Op::Edit { flat, op: EditOp::Append(v) } => format!("{flat} += {v}"),
                    Op::Edit { flat, .. } | Op::Criar { flat, .. } => format!("{flat}: fora deste mod"),
                })
                .collect()
        };
        assert_eq!(
            ops(&mods[0].tweaks[0]),
            [
                "Items.GrenadeIncendiarySticky.hardDifficultySelfDamagePerTick = 4.75",
                "Items.GrenadeIncendiarySticky.bwmsTesteNivel: Int32 = 3",
                "Items.GrenadeIncendiarySticky.visualTags += BwmsTesteAppend",
                "clone Items.BwmsTestePastaGranada2 <- Items.GrenadeIncendiarySticky",
                "Items.BwmsTestePastaGranada2.storyDifficultySelfDamagePerTick = 0.75",
            ]
        );
        assert_eq!(
            ops(&mods[0].tweaks[1]),
            [
                "Items.GrenadeIncendiarySticky.shallowWaterDepth = -0.25",
                "Items.GrenadeIncendiarySticky.rotationAxesSpeeds = [180, 90.5]",
                "Items.GrenadeIncendiarySticky.appearanceSuffixesOwnerOverride += true",
                "Items.GrenadeIncendiarySticky.visualTags = [Default, BwmsTesteTroca]",
                "clone Items.BwmsTestePastaGranada <- Items.GrenadeIncendiarySticky",
                "Items.BwmsTestePastaGranada.attackDuration: Float = 3.25",
                "Items.BwmsTestePastaGranada.bwmsTestePeso: Float = 0.5",
                "Items.BwmsTestePastaGranada.bwmsTesteNome: String = GranadaBWMS",
                "create Modifiers.BwmsTestePastaModificador ConstantStatModifier",
                "Modifiers.BwmsTestePastaModificador.value = 12.5",
                "Bwms.TestePastaMods.inteiro: Int32 = 7",
            ]
        );

        let texto = texto_do_boot(&jogo, &[]).expect("relatório");
        assert!(
            texto.contains("\n[BWMS Teste TweakXL]\ntweaks: 2 lido(s) — aplicado(s) ao carregar o save / 2 read — applied when a save loads\n"),
            "{texto}"
        );
        assert!(!texto.contains("recusado"), "{texto}");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    fn relatorio_de(raiz: &Path, mods: Vec<ModDaPasta>) -> Relatorio {
        Relatorio {
            raiz: raiz.to_path_buf(),
            mods,
            tabelas: Tabelas::Gravadas { tabelas: TabelasDosXl::default(), avisos: Vec::new() },
            tweaks_recusados: Vec::new(),
            recusados: None,
            nativa: None,
            fora_do_jogo: Vec::new(),
            factories_sem_efeito: Vec::new(),
        }
    }

    #[test]
    fn relatorio_diz_o_que_entrou_o_que_ficou_fora_e_os_conflitos() {
        let raiz = PathBuf::from("/jogo/Mods");
        let m = |nome: &str| ModDaPasta { nome: nome.into(), ..ModDaPasta::default() };
        let ruim = raiz.join("A/archive/pc/mod/ruim.archive");
        let xl_bom = raiz.join("A/archive/pc/mod/a.archive.xl");
        let xl_legendas = raiz.join("A/legendas.xl");
        let xl_ruim = raiz.join("A/q.xl");
        let tweak_ruim = raiz.join("A/r6/tweaks/ruim.yaml");
        let solto = raiz.join("solto.archive");
        let a = ModDaPasta {
            archives: vec![raiz.join("A/archive/pc/mod/a.archive"), ruim.clone()],
            xls: vec![xl_bom, xl_legendas.clone(), xl_ruim.clone()],
            tweaks: vec![raiz.join("A/r6/tweaks/t.yaml"), tweak_ruim.clone()],
            reds: vec![raiz.join("A/r6/scripts/a.reds"), raiz.join("A/r6/scripts/b.reds")],
            ..m("A")
        };
        let mods = vec![
            a,
            ModDaPasta { luas: 13, ..m("JB") },
            ModDaPasta { dlls: 1, ..m("input_loader") },
            ModDaPasta { archives: vec![solto.clone()], ..m("solto.archive") },
            ModDaPasta { reds: vec![raiz.join("So Reds/r6/scripts/s.reds")], ..m("So Reds") },
            ModDaPasta { tweaks: vec![raiz.join("So Tweak/t.yaml")], ..m("So Tweak") },
            m("Vazio"),
        ];
        let mut r = relatorio_de(&raiz, mods);
        // Um `.xl` com entrada que o parse deixou de fora (o usuário corrige) e um com o que o BWMS ainda não aplica (não há o que corrigir):
        // grupo de `localization`, `context` do `resource.fix` e a seção `animations`.
        let limite = "localization.subtitles: não aplicado pelo runtime do BWMS, que só aplica onscreens, e o grupo fica sem efeito no jogo; tire o \
                      grupo subtitles do localization deste .xl para o aviso sumir (o jogo fica igual)";
        let contexto = "resource.fix.a.mesh.context: não aplicado pelo runtime do BWMS, e o contexto fica sem efeito no jogo (na referência ele só \
                        pesa em mesh dinâmico); names e paths do mesmo fix seguem valendo; tire o context deste fix para o aviso sumir (o jogo fica \
                        igual)";
        let secao = "animations ×1: o BWMS ainda não aplica esta seção (sem efeito no jogo); nada a corrigir no arquivo";
        r.tabelas = Tabelas::Gravadas {
            tabelas: TabelasDosXl {
                xl_lidos: 2,
                xl_fora: vec![xl_ruim],
                sem_efeito_no_bwms: vec![(xl_legendas.clone(), contexto.into()), (xl_legendas.clone(), limite.into())],
                secoes_sem_efeito: vec![(xl_legendas, secao.into())],
                ..TabelasDosXl::default()
            },
            avisos: vec![
                "Mods/A/q.xl: .xl não parseou (x), fora das tabelas do runtime; corrija o YAML do arquivo e abra o jogo de novo".into(),
                "Mods/A/archive/pc/mod/a.archive.xl: streaming.sectors[0] (s.streamingsector): sem expectedNodes (setor ignorado); informe \
                 expectedNodes, um inteiro maior que 0"
                    .into(),
                "BWMS/mods/misc/Outro/o.xl: .xl não parseou (z), fora das tabelas do runtime; corrija o YAML do arquivo e abra o jogo de novo".into(),
            ],
        };
        r.tweaks_recusados = vec![(tweak_ruim, "o YAML não parseou: linha 2: tab na indentação; use espaços".into())];

        // Antes da carga dos archives: nenhum entrou ainda, e o mod só de archive não diz "nada". O tweak foi lido; a aplicação é no load do
        // save.
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "[A]\nentrou / loaded: 2 .xl\naguardando / pending: 2 .archive (aguardando a carga do jogo / waiting for the game to load them)\n\
             tweaks: 1 lido(s) — aplicado(s) ao carregar o save / 1 read — applied when a save loads\n\
             scripts: 2 .reds — a compilação sai na seção Scripts / build result in the Scripts section\n",
            "[solto.archive]\naguardando / pending: 1 .archive (aguardando a carga do jogo / waiting for the game to load them)\n",
            "recusado / refused: r6/tweaks/ruim.yaml — o tweak não foi lido; corrija o arquivo e abra o jogo de novo / the tweak could not be \
             read; fix the file and start the game again\n  detalhe / detail: o YAML não parseou: linha 2: tab na indentação; use espaços\n",
            "aviso / warning: archive/pc/mod/a.archive.xl — parte do .xl ficou sem efeito; corrija o arquivo e abra o jogo de novo / part of \
             the .xl has no effect; fix the file and start the game again\n  detalhe / detail: streaming.sectors[0] (s.streamingsector): sem \
             expectedNodes (setor ignorado); informe expectedNodes, um inteiro maior que 0\n",
            "recusado / refused: q.xl — o .xl não foi lido; corrija o arquivo e abra o jogo de novo / the .xl could not be read; fix the file and \
             start the game again\n  detalhe / detail: .xl não parseou (x), fora das tabelas do runtime; corrija o YAML do arquivo e abra o jogo \
             de novo\n",
            "[JB]\nentrou / loaded: nada / nothing\nLua: 13 .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll não / CET mod: \
             BWMS runs it from the Mods folder; whatever needs a .dll does not\n",
            "[input_loader]\nentrou / loaded: nada / nothing\nfora / left out: 1 .dll — plugin de Windows, não carrega no Mac / Windows plugin, \
             does not load on a Mac\n",
            "[So Reds]\nscripts: 1 .reds",
            "[So Tweak]\ntweaks: 1 lido(s)",
            "[Vazio]\nentrou / loaded: nada / nothing",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        let linha_do_limite = format!(
            "aviso / warning: legendas.xl — o BWMS ainda não aplica parte deste .xl; o resto entrou / BWMS does not apply part of this .xl yet; \
             the rest loaded\n  detalhe / detail: {contexto}\n  detalhe / detail: {limite}\n  detalhe / detail: {secao}\n"
        );
        assert!(texto.contains(&linha_do_limite), "falta {linha_do_limite:?} em:\n{texto}");
        assert!(
            texto.lines().filter(|l| l.contains("legendas.xl")).all(|l| !l.contains("corrija") && !l.contains("fix the file")),
            "limite do BWMS não manda corrigir o arquivo:\n{texto}"
        );
        assert!(!texto.contains("Outro/o.xl"), "aviso de .xl de fora da Mods não entra no relatório da Mods:\n{texto}");
        assert!(!texto.contains("Conflitos"), "antes da carga dos archives não há seção de conflitos:\n{texto}");

        // Depois da carga: os aceitos entram, e o archive solto recusado sai com o nome dele.
        r.recusados = Some(vec![(ruim, Recusa::Invalido), (solto, Recusa::NomeRepetido)]);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "[A]\nentrou / loaded: 1 .archive, 2 .xl\ntweaks: 1 lido(s)",
            "recusado / refused: archive/pc/mod/ruim.archive — não é um .archive válido; baixe o mod de novo / not a valid .archive; download the mod again",
            "[solto.archive]\nentrou / loaded: nada / nothing\nrecusado / refused: solto.archive — já há um archive carregado com este nome; renomeie o arquivo",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        assert!(!texto.contains("aguardando"), "{texto}");

        // A ação do conflito segue o tipo do mod: pasta, arquivo solto, ou um de cada.
        let m = Caminho::Mods;
        let cs = vec![
            Conflito { vence: "A".into(), perde: "JB".into(), recursos: 2, vence_no: m, perde_no: m },
            Conflito { vence: "A".into(), perde: "solto.archive".into(), recursos: 1, vence_no: m, perde_no: m },
        ];
        r.tabelas = Tabelas::Falhou("disco cheio".into());
        let texto = texto_do_relatorio(&r, &Sabido { conflitos: Some(&cs), ..nada() }, &[("Scripts".into(), vec!["A: 2 .reds compilados / 2 .reds built".into()])]);
        assert!(
            texto.contains(
                "[Conflitos / Conflicts]\nA x JB: 2 recurso(s) em comum, vence A; para inverter, renomeie a pasta / 2 shared resource(s), A wins; \
                 to swap, rename the folder\nA x solto.archive: 1 recurso(s) em comum, vence A; para inverter, renomeie a pasta ou o arquivo / 1 \
                 shared resource(s), A wins; to swap, rename the folder or the file\n"
            ),
            "{texto}"
        );
        assert!(
            texto.contains(
                "[Tabelas / Tables]\nos .xl da Mods ficaram fora das tabelas do jogo; corrija o erro abaixo e abra o jogo de novo / the Mods .xl \
                 files were left out of the game tables; fix the error below and start the game again\n  detalhe / detail: disco cheio\n"
            ),
            "{texto}"
        );
        assert!(texto.contains("[A]\nentrou / loaded: 1 .archive\n"), "sem tabelas, nenhum .xl entrou:\n{texto}");
        assert!(!texto.contains("q.xl") && !texto.contains("legendas.xl"), "sem tabelas não há .xl recusado nem com aviso:\n{texto}");
        assert!(texto.ends_with("\n[Scripts]\nA: 2 .reds compilados / 2 .reds built\n"), "{texto}");

        assert!(texto_do_relatorio(&r, &Sabido { conflitos: Some(&[]), ..nada() }, &[]).contains("[Conflitos / Conflicts]\nnenhum / none\n"));
        assert!(texto_do_relatorio(&relatorio_de(&raiz, Vec::new()), &nada(), &[]).contains("Nenhum mod na pasta"));
    }

    /// Único teste que mexe no `ESTADO` do processo: a seção anexada antes da 1ª gravação espera e entra nela, a anexada depois regrava o
    /// arquivo, e o hot reload lê os tweaks da varredura sem varrer.
    #[test]
    fn estado_do_boot_guarda_secao_antecipada_e_tweaks_da_varredura() {
        let raiz = pasta_de_teste("estado");
        anexar_ao_relatorio("Scripts", &["antes / before".to_string()]);
        assert!(!raiz.join(RELATORIO).exists(), "sem o relatório do boot, nada é gravado");
        assert!(tweaks_do_boot().is_empty());
        let t = raiz.join("A/r6/tweaks/t.yaml");
        {
            let mut e = estado();
            let mods = vec![ModDaPasta { nome: "A".into(), tweaks: vec![t.clone()], ..ModDaPasta::default() }];
            e.relatorio = Some(relatorio_de(&raiz, mods));
            publicar(&e);
        }
        assert!(std::fs::read_to_string(raiz.join(RELATORIO)).unwrap().ends_with("\n[Scripts]\nantes / before\n"));
        anexar_ao_relatorio("Scripts", &["depois / after".to_string()]);
        assert!(std::fs::read_to_string(raiz.join(RELATORIO)).unwrap().ends_with("\n[Scripts]\ndepois / after\n"));
        assert_eq!(tweaks_do_boot(), [t]);
        *estado() = Estado { relatorio: None, secoes: Vec::new(), modio: ModsDoModIo::vazio(), conflitos: None };
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// A seção da pasta nativa: contagem por via, aviso do save, o que saiu, e o que ficou fora do jogo com o motivo e a ação (antes da carga,
    /// o que a sincronização não pôs; depois, também o clone que o jogo não montou); o que ficou fora não conta como entrou nem disputa
    /// conflito; sem nada na pasta e nada fora, a seção não aparece.
    #[test]
    fn relatorio_da_pasta_nativa() {
        use super::super::pasta_nativa::{fora_do_jogo, MotivoDeFora, Sincronia, Via};
        let raiz = PathBuf::from("/jogo/Mods");
        let (a, b) = (raiz.join("A/a.archive"), raiz.join("A/b.archive"));
        let mut r = relatorio_de(&raiz, vec![ModDaPasta { nome: "A".into(), archives: vec![a.clone(), b.clone()], ..ModDaPasta::default() }]);
        let destino = PathBuf::from("/jogo/archive/Mac/mod/BWMS-Mods/0001/a.archive");
        let s = Sincronia {
            nativos: vec![(a.clone(), destino, Via::Clone)],
            fora: vec![(b.clone(), MotivoDeFora::NaoCopiou("sem espaço".into()))],
            removidos: 2,
            ..Sincronia::default()
        };
        // O caso forçado: o depot não mostra o clone de `a`.
        let fora = fora_do_jogo(&[a.clone(), b.clone()], &s, |_| false);
        r.nativa = Some(s);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "\n[Pasta do jogo / Game folder]\n1 .archive em archive/Mac/mod/BWMS-Mods, a pasta que o próprio jogo carrega (1 clone(s) APFS, sem espaço extra em \
             disco; 0 cópia(s); 0 já em dia)",
            "Saves: com archive nessa pasta o jogo marca o save como modded (ícone de mod na lista de saves do Mac)",
            "2 arquivo(s) saiu(saíram) de BWMS-Mods porque não corresponde(m) a um .archive atual da Mods",
            "não entrou / not loaded: A/b.archive — não deu para pôr o arquivo em archive/Mac/mod (sem espaço); confira o espaço livre e a permissão \
             da pasta do jogo e abra o jogo de novo / the file could not be placed in archive/Mac/mod (sem espaço)",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        assert!(!texto.contains("A/a.archive —"), "antes da carga o clone posto não está fora:\n{texto}");
        // Depois da carga vale o que o depot mostrou: o clone que o jogo não montou também fica fora.
        r.recusados = Some(Vec::new());
        r.fora_do_jogo = fora;
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "não entrou / not loaded: A/a.archive — o jogo não montou o arquivo da pasta archive/Mac/mod; baixe o mod de novo e abra o jogo de novo \
             / the game did not mount the file from archive/Mac/mod; download the mod again and start the game again\n",
            "não entrou / not loaded: A/b.archive — não deu para pôr o arquivo",
            "[A]\nentrou / loaded: nada / nothing\n",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        assert!(!texto.contains("through BWMS"), "{texto}");
        assert_eq!(r.aceitos(), [(Caminho::Mods, "A".to_string(), Vec::<PathBuf>::new())]);
        r.nativa = Some(Sincronia::default());
        r.fora_do_jogo.clear();
        assert!(!texto_do_relatorio(&r, &nada(), &[]).contains("[Pasta do jogo"));
    }

    #[test]
    fn secao_anexada_troca_a_de_mesmo_titulo() {
        let mut secoes = Vec::new();
        anexar(&mut secoes, "Scripts", &["1".to_string()]);
        anexar(&mut secoes, "Outra", &["x".to_string()]);
        anexar(&mut secoes, "Scripts", &["2".to_string(), "3".to_string()]);
        assert_eq!(secoes, [("Scripts".to_string(), vec!["2".to_string(), "3".to_string()]), ("Outra".to_string(), vec!["x".to_string()])]);
        // A carga que não acha mais o problema manda a seção sem linhas, e ela sai.
        anexar(&mut secoes, "Outra", &[]);
        assert_eq!(secoes.len(), 1);
    }

    /// Cada mod da `Mods/` tem uma cor: o que o relatório põe como recusado, fora ou aviso deixa amarelo; scripts fora da compilação, ou só
    /// `.dll`/`.lua`, deixam vermelho; a marca nunca melhora a cor, e o mod do manifesto entra pelo estado dos scripts.
    #[test]
    fn estado_por_mod_junta_o_relatorio_os_scripts_e_os_patches() {
        let raiz = PathBuf::from("/jogo/Mods");
        let arq = |s: &str| raiz.join(s);
        let mods = vec![
            ModDaPasta { nome: "Archive".into(), archives: vec![arq("Archive/a.archive"), arq("Archive/b.archive")], ..ModDaPasta::default() },
            ModDaPasta { nome: "Bom".into(), archives: vec![arq("Bom/b.archive")], reds: vec![arq("Bom/b.reds")], ..ModDaPasta::default() },
            ModDaPasta { nome: "Cet".into(), luas: 2, ..ModDaPasta::default() },
            ModDaPasta { nome: "Misto".into(), archives: vec![arq("Misto/m.archive")], dlls: 1, ..ModDaPasta::default() },
            ModDaPasta { nome: "Quebrado".into(), reds: vec![arq("Quebrado/q.reds")], tweaks: vec![arq("Quebrado/t.yaml")], ..ModDaPasta::default() },
            ModDaPasta { nome: "Xl".into(), xls: vec![arq("Xl/x.xl")], ..ModDaPasta::default() },
        ];
        let mut r = relatorio_de(&raiz, mods);
        r.recusados = Some(vec![(arq("Archive/b.archive"), Recusa::Invalido)]);
        r.tweaks_recusados = vec![(arq("Quebrado/t.yaml"), "o YAML não parseou".into())];
        let mut scripts = EstadosDosMods::default();
        scripts.marcar(&arq("Quebrado"), CorDoMod::Vermelho, "não compilou / did not compile: q.reds:1: x");
        scripts.marcar(Path::new("/plugin/p.reds"), CorDoMod::Amarelo, "aviso / warning: p.reds:1: y");
        let patches = vec![("Mods/Xl/x".to_string(), "mod\\x.ent".to_string()), ("BWMS/outro/y".to_string(), "z.ent".to_string())];
        let e = estados_de(Some(&r), &scripts, &patches);
        let cor = |p: &Path| e.cor(p);
        assert_eq!(cor(&arq("Archive")), Some(CorDoMod::Amarelo));
        assert_eq!(cor(&arq("Bom")), Some(CorDoMod::Verde));
        assert_eq!(cor(&arq("Cet")), Some(CorDoMod::Verde), "só .lua: o BWMS roda");
        assert_eq!(cor(&arq("Misto")), Some(CorDoMod::Amarelo));
        assert_eq!(cor(&arq("Quebrado")), Some(CorDoMod::Vermelho), "o tweak recusado (amarelo) não melhora o vermelho dos scripts");
        assert_eq!(cor(&arq("Xl")), Some(CorDoMod::Amarelo), "patch que o depot não tem");
        assert_eq!(cor(Path::new("/plugin/p.reds")), Some(CorDoMod::Amarelo));
        assert_eq!(e.raizes.len(), 7, "o patch de fora da Mods não cria mod");
        let i = e.raizes.iter().position(|p| *p == arq("Quebrado")).unwrap();
        assert_eq!(e.motivos[i].lines().count(), 2);
        assert!(e.motivos[i].contains("t.yaml") && e.motivos[i].contains("q.reds:1"));
        assert!(e.motivos[e.raizes.iter().position(|p| *p == arq("Bom")).unwrap()].is_empty());
        for m in e.motivos.iter().flat_map(|m| m.lines()) {
            assert!(m.contains(" / "), "motivo sem as duas línguas: {m}");
        }
        // Sem o relatório (sem a pasta `Mods/`), só os scripts.
        assert_eq!(estados_de(None, &scripts, &patches), scripts);
    }

    /// `RDAR` v12 com o índice em `indice` (tamanho `tam_indice`) e `recursos` em `indice + 16`, num arquivo de `total` bytes.
    fn rdar(magic: &[u8; 4], indice: u64, tam_indice: u32, recursos: u32, total: usize) -> Vec<u8> {
        let mut b = vec![0u8; total];
        b[..4].copy_from_slice(magic);
        b[4..8].copy_from_slice(&12u32.to_le_bytes());
        b[8..16].copy_from_slice(&indice.to_le_bytes());
        b[16..20].copy_from_slice(&tam_indice.to_le_bytes());
        if (indice as usize + 20) <= total {
            b[indice as usize + 16..indice as usize + 20].copy_from_slice(&recursos.to_le_bytes());
        }
        b
    }

    #[test]
    fn archive_valido_confere_o_cabecalho_antes_do_motor() {
        let dir = pasta_de_teste("archive-valido");
        let caso = |nome: &str, bytes: Vec<u8>| {
            let p = dir.join(nome);
            std::fs::write(&p, bytes).unwrap();
            crate::arquivos_do_jogo::archive_valido(p.to_str().unwrap())
        };
        assert_eq!(caso("bom.archive", rdar(b"RDAR", 4096, 28, 3, 8192)), Some(3));
        assert_eq!(caso("magia.archive", rdar(b"XXXX", 4096, 28, 3, 8192)), None);
        assert_eq!(caso("desalinhado.archive", rdar(b"RDAR", 4000, 28, 3, 8192)), None);
        assert_eq!(caso("alem-do-fim.archive", rdar(b"RDAR", 8192, 28, 3, 8192)), None);
        assert_eq!(caso("indice-curto.archive", rdar(b"RDAR", 4096, 8, 3, 8192)), None);
        // Posição alinhada em que posição + tamanho estoura u64: recusado sem somar com estouro.
        assert_eq!(caso("estouro.archive", rdar(b"RDAR", 0xFFFF_FFFF_FFFF_F000, 0x1000, 3, 8192)), None);
        assert_eq!(caso("vazio.archive", Vec::new()), None);
        assert_eq!(crate::arquivos_do_jogo::archive_valido(dir.join("nao-existe.archive").to_str().unwrap()), None);
        // O índice declara 3 recursos e tem só o cabeçalho: válido para o motor, mas sem tabela de hashes.
        assert_eq!(crate::arquivos_do_jogo::hashes_do_archive(&dir.join("bom.archive")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Os mods reais extraídos dos zips do Nexus, numa `Mods/` de um jogo falso. São código de terceiro: moram em
    /// `_referencia-de-terceiro/mods-reais/`, só para leitura de formato, fora da árvore do produto. Sem a pasta, pula.
    fn fixtures() -> Option<PathBuf> {
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("../_referencia-de-terceiro/mods-reais");
        if f.is_dir() {
            Some(f)
        } else {
            eprintln!("mods reais ausentes em {}: teste pulado", f.display());
            None
        }
    }

    fn copiar_arvore(de: &Path, para: &Path) {
        for p in apply::arquivos_da_pasta_mods(de) {
            let destino = para.join(p.strip_prefix(de).unwrap());
            std::fs::create_dir_all(destino.parent().unwrap()).unwrap();
            std::fs::copy(&p, destino).unwrap();
        }
    }

    #[test]
    fn mods_reais_viram_archives_tabelas_tweaks_reds_e_relatorio() {
        let Some(fixtures) = fixtures() else { return };
        let jogo = pasta_de_teste("mods-reais");
        let raiz = pasta(&jogo);
        copiar_arvore(&fixtures, &raiz);
        // Nenhum dos 5 traz tweak: um mod de tweak solto, um em r6/tweaks e um quebrado entram ao lado deles.
        escrever_em(
            &raiz,
            &[
                ("Tweak Solto/preco.yaml", b"Items.Preset_Yukimura_Default:\n  quality: Quality.Legendary\n"),
                ("Virtual Atelier/r6/tweaks/teste/va.yaml", b"Items.Preset_Yukimura_Default:\n  quality: Quality.Rare\n"),
                ("Tweak Quebrado/r6/tweaks/q.yaml", b"Items.Preset_Yukimura_Default:\n\tquality: Quality.Rare\n"),
            ],
        );

        let r = preparar(&jogo).unwrap();
        let nomes: Vec<&str> = r.mods.iter().map(|m| m.nome.as_str()).collect();
        assert_eq!(nomes, ["Equipment-EX", "JB Third Person Mod", "Quadra Turbo R", "Tweak Quebrado", "Tweak Solto", "Virtual Atelier", "input_loader"]);
        assert_eq!(
            relativos(&raiz, &archives(&raiz)),
            [
                "Equipment-EX/archive/pc/mod/EquipmentEx.archive",
                "JB Third Person Mod/archive/pc/mod/jb_tpp_mod_1.archive",
                "Quadra Turbo R/basegame_Quadra Turbo R V-Tec.archive",
                "Virtual Atelier/archive/pc/mod/VirtualAtelier.archive",
            ]
        );
        for a in archives(&raiz) {
            assert!(crate::arquivos_do_jogo::hashes_do_archive(&a).is_some_and(|h| !h.is_empty()), "índice ilegível: {}", a.display());
        }
        assert_eq!(
            relativos(&raiz, &tweaks(&raiz)),
            ["Tweak Quebrado/r6/tweaks/q.yaml", "Tweak Solto/preco.yaml", "Virtual Atelier/r6/tweaks/teste/va.yaml"]
        );
        assert_eq!(relativos(&raiz, &r.tweaks_recusados.iter().map(|(p, _)| p.clone()).collect::<Vec<_>>()), ["Tweak Quebrado/r6/tweaks/q.yaml"]);
        let reds_dos_zips = apply::arquivos_da_pasta_mods(&fixtures).iter().filter(|p| p.extension().is_some_and(|e| e == "reds")).count();
        assert_eq!(reds(&raiz).len(), reds_dos_zips);
        assert!(reds(&raiz).iter().all(|p| p.starts_with(raiz.join("Equipment-EX")) || p.starts_with(raiz.join("Virtual Atelier"))));
        let por_nome = |n: &str| r.mods.iter().find(|m| m.nome == n).unwrap();
        assert_eq!((por_nome("JB Third Person Mod").luas, por_nome("input_loader").dlls), (12, 1));

        // Os dois `.xl` (só `localization.onscreens`) viram linhas da tabela de localização, com o id do caminho no jogo.
        let Tabelas::Gravadas { tabelas, avisos } = &r.tabelas else { panic!("tabelas: {:?}", r.tabelas) };
        assert_eq!((tabelas.xl_lidos, tabelas.xl_fora.len()), (2, 0), "{avisos:?}");
        let localizacao = jogo.join("red4ext").join(bwms_primitives::tabelas::LOCALIZACAO.arquivo);
        let texto = std::fs::read_to_string(&localizacao).unwrap();
        let mut por_mod: BTreeMap<String, usize> = BTreeMap::new();
        for l in bwms_primitives::tabelas::ler(&texto, bwms_primitives::tabelas::Localizacao::ler) {
            *por_mod.entry(l.unwrap().mod_id.to_string()).or_default() += 1;
        }
        let esperado: BTreeMap<String, usize> = [
            ("Mods/Equipment-EX/archive/pc/mod/EquipmentEx.archive".to_string(), 13),
            ("Mods/Virtual Atelier/archive/pc/mod/VirtualAtelier.archive".to_string(), 10),
        ]
        .into();
        assert_eq!(por_mod, esperado);

        // O relatório antes da carga dos archives (o `on_load`): eles aguardam.
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        assert!(
            texto.contains(
                "[JB Third Person Mod]\naguardando / pending: 1 .archive (aguardando a carga do jogo / waiting for the game to load them)\nLua: 12 \
                 .lua — mod de CET: o BWMS roda pela pasta Mods"
            ),
            "{texto}"
        );

        // Conflitos reais entre os 4 archives, pelo índice.
        let achados = conflitos(&r.aceitos());
        eprintln!("conflitos reais: {achados:?}");
        let pares: Vec<(&str, &str, usize)> = achados.iter().map(|c| (c.vence.as_str(), c.perde.as_str(), c.recursos)).collect();
        assert_eq!(pares, Vec::<(&str, &str, usize)>::new());

        // Depois da carga, sem recusa (o que o gancho do `InitializeArchives` grava).
        let mut r = r;
        r.recusados = Some(Vec::new());
        let texto = texto_do_relatorio(&r, &Sabido { conflitos: Some(&achados), ..nada() }, &[]);
        eprintln!("{texto}");
        for linha in [
            "[JB Third Person Mod]\nentrou / loaded: 1 .archive\nLua: 12 .lua — mod de CET: o BWMS roda pela pasta Mods; o que depende de .dll \
             não / CET mod: BWMS runs it from the Mods folder; whatever needs a .dll does not\n",
            "[input_loader]\nentrou / loaded: nada / nothing\nfora / left out: 1 .dll — plugin de Windows, não carrega no Mac / Windows plugin, \
             does not load on a Mac\n",
            "[Equipment-EX]\nentrou / loaded: 1 .archive, 1 .xl\nscripts: 2 .reds",
            "[Virtual Atelier]\nentrou / loaded: 1 .archive, 1 .xl\ntweaks: 1 lido(s) — aplicado(s) ao carregar o save / 1 read — applied when a \
             save loads\nscripts:",
            "[Tweak Solto]\ntweaks: 1 lido(s) — aplicado(s) ao carregar o save / 1 read — applied when a save loads\n",
            "[Tweak Quebrado]\nentrou / loaded: nada / nothing\nrecusado / refused: r6/tweaks/q.yaml — o tweak não foi lido; corrija o arquivo e \
             abra o jogo de novo / the tweak could not be read; fix the file and start the game again\n  detalhe / detail: o YAML não parseou: \
             linha 2",
            "[Conflitos / Conflicts]\nnenhum / none\n",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }

        // O arquivo do relatório na raiz da `Mods/`, que a varredura seguinte não conta como mod.
        let nomes_antes: Vec<String> = r.mods.iter().map(|m| m.nome.clone()).collect();
        let e = Estado { relatorio: Some(r), secoes: vec![("Scripts".into(), vec!["linha da lane de scripts".into()])], modio: ModsDoModIo::vazio(), conflitos: None };
        publicar(&e);
        let gravado = std::fs::read_to_string(raiz.join(RELATORIO)).unwrap();
        assert!(gravado.starts_with("BWMS — pasta Mods / Mods folder\n") && gravado.ends_with("\n[Scripts]\nlinha da lane de scripts\n"), "{gravado}");
        assert_eq!(varrer(&ler(&raiz)).iter().map(|m| m.nome.clone()).collect::<Vec<_>>(), nomes_antes);
        let r = e.relatorio.unwrap();

        // Conflito real pelo índice: o archive do Quadra numa 2ª pasta, que vem antes em ordem de caminho, divide todos os recursos com ele.
        let quadra = raiz.join("Quadra Turbo R/basegame_Quadra Turbo R V-Tec.archive");
        let n = crate::arquivos_do_jogo::hashes_do_archive(&quadra).unwrap().len();
        escrever_em(&raiz, &[("Quadra Copia/quadra-copia.archive", &std::fs::read(&quadra).unwrap())]);
        let mods: Vec<(Caminho, String, Vec<PathBuf>)> = varrer(&ler(&raiz)).into_iter().map(|m| (Caminho::Mods, m.nome, m.archives)).collect();
        assert_eq!(conflitos(&mods), [Conflito { vence: "Quadra Copia".into(), perde: "Quadra Turbo R".into(), recursos: n, vence_no: Caminho::Mods, perde_no: Caminho::Mods }]);
        std::fs::remove_dir_all(raiz.join("Quadra Copia")).unwrap();

        // Sem os `.xl`, o boot seguinte tira as linhas deles.
        for m in &r.mods {
            for x in &m.xls {
                std::fs::remove_file(x).unwrap();
            }
        }
        assert!(matches!(preparar(&jogo).map(|r| r.tabelas), Some(Tabelas::Gravadas { .. })));
        assert!(!localizacao.exists());
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// `n` mods sintéticos na `Mods/`, cada `.xl` com linhas em todas as 9 tabelas (os dois `.xl` do golden do motor, com o
    /// caminho de cada mod): o pior caso do boot com `.xl` na `Mods/`, que lê o `.xl` de cada mod e regrava as tabelas.
    fn mods_sinteticos(jogo: &Path, n: usize) {
        for i in 0..n {
            let alfa = format!(
                "resource:\n  link:\n    base\\fake\\a{i}.mesh: base\\real\\a{i}.mesh\n  copy:\n    base\\orig{i}.mesh: mod{i}\\copia.mesh\n  scope:\n    mod{i}\\grupo.ent:\n      - base\\x{i}.ent\n  fix:\n    my{i}.mesh:\n      names:\n        old_mat: new_mat\nfactories:\n  - mod{i}\\f.csv\njournal:\n  - mod{i}\\q.journal\nlocalization:\n  onscreens:\n    en-us: mod{i}\\\\loc\\\\en-us.json\ncustomizations:\n  female: mod{i}\\f.inkcharcustomization\n"
            );
            let beta = format!(
                "resource:\n  patch:\n    mod{i}\\p.ent:\n      props: [ components ]\n      targets: [ base\\z{i}.ent ]\nquest:\n  phases:\n    - path: mod{i}\\fase.questphase\n      parent: base\\quest\\pai{i}.questphase\nstreaming:\n  sectors:\n    - path: s{i}.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - index: 1\n          type: worldStaticMeshNode\n"
            );
            let dir = pasta(jogo).join(format!("Sintetico{i:03}"));
            escrever_em(&dir, &[("m.archive", b"x"), ("alfa.xl", alfa.as_bytes()), ("beta.archive.xl", beta.as_bytes())]);
        }
    }

    /// Medido nos 5 mods reais: varredura, tabelas, índices e tweaks; e de novo com 20 e com 100 mods sintéticos a mais na `Mods/`, cada um
    /// com as 9 tabelas. Imprime o tempo de cada parte. `cargo test ... -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn medir_varredura_tabelas_e_indices_com_os_mods_reais() {
        let Some(fixtures) = fixtures() else { return };
        const VOLTAS: u32 = 50;
        let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0 / f64::from(VOLTAS);
        for n in [0usize, 20, 100] {
            let jogo = pasta_de_teste(&format!("medida-{n}"));
            let raiz = pasta(&jogo);
            copiar_arvore(&fixtures, &raiz);
            mods_sinteticos(&jogo, n);
            let mut t = [std::time::Duration::ZERO; 5];
            let mut linhas = 0usize;
            for _ in 0..VOLTAS {
                let inicio = std::time::Instant::now();
                let lida = apply::ler_pasta_mods(&jogo);
                let mods = varrer(&lida);
                t[0] += inicio.elapsed();
                let inicio = std::time::Instant::now();
                let tabelas = gravar_tabelas(&jogo, &lida);
                t[1] += inicio.elapsed();
                let Tabelas::Gravadas { tabelas, .. } = tabelas else { panic!("{tabelas:?}") };
                linhas = tabelas.reslink_pairs + tabelas.streaming_lines + tabelas.questphase_lines
                    + tabelas.journal_lines + tabelas.resource_patch_lines + tabelas.resfix_lines + tabelas.resscope_lines
                    + tabelas.customization_lines;
                let inicio = std::time::Instant::now();
                let aceitos: Vec<(Caminho, String, Vec<PathBuf>)> = mods.iter().map(|m| (Caminho::Mods, m.nome.clone(), m.archives.clone())).collect();
                std::hint::black_box(conflitos(&aceitos));
                t[2] += inicio.elapsed();
                let inicio = std::time::Instant::now();
                std::hint::black_box(tweaks_recusados(&mods));
                t[3] += inicio.elapsed();
                let inicio = std::time::Instant::now();
                std::hint::black_box(tweaks(&raiz));
                t[4] += inicio.elapsed();
            }
            let gravadas = std::fs::read_dir(jogo.join("red4ext")).map(|d| d.count()).unwrap_or(0);
            eprintln!(
                "[medida] {} ({VOLTAS} voltas, cache quente, {n} mods sintéticos, {gravadas} tabela(s), {linhas} linha(s) fora a localização): varredura {:.3} ms, tabelas {:.3} ms, índices+conflitos {:.3} ms, tweaks recusados {:.3} ms, varredura dos tweaks da aplicação {:.3} ms",
                jogo.display(),
                ms(t[0]),
                ms(t[1]),
                ms(t[2]),
                ms(t[3]),
                ms(t[4])
            );
            let _ = std::fs::remove_dir_all(&jogo);
        }
    }
    /// O boot com os `.xl` do Equipment-EX e do Virtual Atelier na `Mods/`: os dois `.json` na corrente da `BWMS-Mods/xl` (o EEX, que vence
    /// em ordem de caminho, no fundo; o `.xl` sem textos conta na numeração), a `bwms-localizacao.txt` sem linha deles, e o relatório dizendo quem aplica cada seção. Um mod ligado
    /// `.json` de textos em outra pasta de `archive/Mac/mod` devolve tudo ao BWMS: os `.json` saem e a tabela os leva de novo.
    #[test]
    fn boot_entrega_os_textos_ao_jogo_e_tira_da_tabela() {
        let jogo = pasta_de_teste("xl-no-jogo");
        std::fs::create_dir_all(jogo.join("archive/Mac/content")).unwrap();
        let eex = "Equipment-EX/archive/pc/mod/EquipmentEx.archive.xl";
        let va = "Virtual Atelier/archive/pc/mod/VirtualAtelier.archive.xl";
        let real = |rel: &str| std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../_referencia-de-terceiro/mods-reais").join(rel));
        // Sem a pasta (a árvore pública não a leva), pula, como o teste dos mods reais acima.
        let (Ok(texto_eex), Ok(texto_va)) = (real(eex), real(va)) else {
            eprintln!("os .xl reais de _referencia-de-terceiro/mods-reais não estão aqui: teste pulado");
            let _ = std::fs::remove_dir_all(&jogo);
            return;
        };
        escrever_em(&jogo.join("Mods"), &[(eex, &texto_eex), (va, &texto_va), ("Outro/o.xl", b"journal:\n  - o\\j.journal\n")]);
        let (r, s) = preparar_com_a_pasta_nativa(&jogo, apply::ler_pasta_mods(&jogo), &ModsDoModIo::vazio());
        let mut r = r.unwrap();
        assert_eq!(s.json_falhou, None);
        let rel: Vec<String> = s.jsons.iter().map(|p| p.strip_prefix(&jogo).unwrap().display().to_string()).collect();
        assert_eq!(rel, ["archive/Mac/mod/BWMS-Mods/xl/0003-VirtualAtelier.archive.json", "archive/Mac/mod/BWMS-Mods/xl/xl/0001-EquipmentEx.archive.json"]);
        let json_eex = std::fs::read_to_string(&s.jsons[1]).unwrap();
        assert!(json_eex.contains("\"es-mx\": [\n                \"equipment_ex\\\\localization\\\\en-us.json\"\n            ]"), "{json_eex}");
        assert!(bwms_primitives::json::ler(&json_eex).is_ok());
        let loc = std::fs::read_to_string(jogo.join("red4ext/bwms-localizacao.txt")).unwrap_or_default();
        assert!(loc.lines().all(|l| l.starts_with('#')), "{loc}");
        assert!(std::fs::read_to_string(jogo.join("red4ext/bwms-journal.txt")).unwrap().contains("o\\j.journal"));
        r.nativa = Some(s);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "seções / sections: archive/pc/mod/EquipmentEx.archive.xl — localization.onscreens → jogo / game\n",
            "seções / sections: o.xl — journal → BWMS\n",
            "2 .json em archive/Mac/mod/BWMS-Mods/xl: o próprio jogo aplica os textos (localization.onscreens) desses .xl, e o BWMS não os aplica",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        // Instruções de textos em outra pasta de `archive/Mac/mod`: o jogo as aplicaria fora da ordem da `Mods/`.
        escrever_em(&jogo, &[("archive/Mac/mod/outro/o.json", b"{\"localization\": {}}")]);
        let (r, s) = preparar_com_a_pasta_nativa(&jogo, apply::ler_pasta_mods(&jogo), &ModsDoModIo::vazio());
        let mut r = r.unwrap();
        assert!(s.jsons.is_empty());
        assert!(!jogo.join("archive/Mac/mod/BWMS-Mods/xl").exists());
        let loc = std::fs::read_to_string(jogo.join("red4ext/bwms-localizacao.txt")).unwrap();
        assert!(loc.contains("Mods/Equipment-EX/archive/pc/mod/EquipmentEx.archive | onscreens | en-us"), "{loc}");
        r.nativa = Some(s);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        assert!(texto.contains("seções / sections: archive/pc/mod/EquipmentEx.archive.xl — localization.onscreens → BWMS\n"), "{texto}");
        assert!(texto.contains("textos (localization.onscreens) dos .xl da Mods: pelo BWMS — há instruções de textos fora da BWMS-Mods (archive/Mac/mod/outro/o.json); mova esse mod para a Mods"), "{texto}");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// O boot com `factories` na `Mods/`: cada `.xl` vai ao jogo num `.json` do 1º nível, sem tabela. O `.csv` que não está num archive da
    /// `Mods/` nem de um mod ligado do mod.io deixa o mod amarelo, com o motivo no bloco dele; sem `archive/Mac`, todo `factories` fica sem
    /// efeito.
    #[test]
    fn boot_entrega_os_factories_ao_jogo_e_marca_o_que_fica_sem_efeito() {
        use bwms_primitives::resource_path_hash;
        let jogo = pasta_de_teste("factories-no-jogo");
        std::fs::create_dir_all(jogo.join("archive/Mac/content")).unwrap();
        let f = archive_com(&[resource_path_hash("f\\f.csv")]);
        escrever_em(
            &jogo.join("Mods"),
            &[
                ("F/f.archive", &f),
                ("F/f.archive.xl", b"factories:\n  - f\\f.csv\n"),
                ("G/g.archive.xl", b"factories: [ g\\g.csv, f\\f.csv ]\n"),
                ("M/m.xl", b"factories: m\\m.csv\n"),
            ],
        );
        let pasta_do_modio = jogo.join("modio/1");
        escrever_em(&pasta_do_modio, &[("archive/m.archive", &archive_com(&[resource_path_hash("m\\m.csv")]))]);
        let modio = ModsDoModIo {
            pastas: vec![pasta_do_modio.clone()],
            ids: vec![Some(1)],
            rotulos: vec!["M (mod.io 1)".into()],
            ligados: vec![Some(true)],
            archives: vec![vec![pasta_do_modio.join("archive/m.archive")]],
            luzes: vec![ModStatus::Ok],
            motivos: vec![String::new()],
        };
        let (r, s) = preparar_com_a_pasta_nativa(&jogo, apply::ler_pasta_mods(&jogo), &modio);
        let mut r = r.unwrap();
        assert_eq!(s.json_falhou, None);
        let rel: Vec<String> = s.jsons.iter().map(|p| p.strip_prefix(&jogo).unwrap().display().to_string()).collect();
        assert_eq!(
            rel,
            ["archive/Mac/mod/BWMS-Mods/xl/0001-f.archive.json", "archive/Mac/mod/BWMS-Mods/xl/0002-g.archive.json", "archive/Mac/mod/BWMS-Mods/xl/0003-m.json"]
        );
        assert_eq!(std::fs::read_to_string(&s.jsons[1]).unwrap(), "{\n    \"factories\": [\n        \"g\\\\g.csv\",\n        \"f\\\\f.csv\"\n    ]\n}\n");
        assert!(!jogo.join("red4ext/bwms-factories.txt").exists());
        let g = r.raiz.join("G/g.archive.xl");
        assert_eq!(r.factories_sem_efeito, [(g.clone(), Motivo::CsvForaDoJogo(vec!["g\\g.csv".into()]))]);
        let luzes = estados_de(Some(&r), &EstadosDosMods::default(), &[]);
        let cor = |m: &str| luzes.cor(&r.raiz.join(m));
        assert_eq!((cor("F"), cor("G"), cor("M")), (Some(CorDoMod::Verde), Some(CorDoMod::Amarelo), Some(CorDoMod::Verde)));
        r.nativa = Some(s);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        for linha in [
            "seções / sections: f.archive.xl — factories → jogo / game\n",
            "3 .json em archive/Mac/mod/BWMS-Mods/xl: o próprio jogo aplica os itens (factories) desses .xl",
            "aviso / warning: g.archive.xl — itens (factories) sem efeito no jogo / items (factories) have no effect in the game\n  detalhe / detail: g\\g.csv não está em nenhum .archive",
        ] {
            assert!(texto.contains(linha), "falta {linha:?} em:\n{texto}");
        }
        assert!(!texto.contains("textos (localization.onscreens) dos .xl da Mods"), "{texto}");

        // Sem `archive/Mac`: nenhum `.json`, e os 3 `factories` sem efeito.
        std::fs::remove_dir_all(jogo.join("archive")).unwrap();
        let (r, _) = preparar_com_a_pasta_nativa(&jogo, apply::ler_pasta_mods(&jogo), &modio);
        let r = r.unwrap();
        let motivos: Vec<&Motivo> = r.factories_sem_efeito.iter().map(|(_, m)| m).collect();
        assert_eq!(motivos, [&Motivo::SemPastaDoJogo; 3]);
        let texto = texto_do_relatorio(&r, &nada(), &[]);
        assert!(texto.contains("seções / sections: m.xl — factories → sem efeito / no effect\n"), "{texto}");
        let _ = std::fs::remove_dir_all(&jogo);
    }
}
