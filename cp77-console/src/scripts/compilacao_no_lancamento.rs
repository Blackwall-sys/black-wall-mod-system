//! Compilação dos `.reds` na abertura do jogo, antes de o motor ler o `final.redscripts` (o redscript do Windows compila a cada abertura).
//!
//! Roda no `on_load`, o construtor da dylib (`__DATA,__mod_init_func`), que o dyld chama ao carregar a dylib pelo `LC_LOAD_DYLIB` do
//! binário, antes do `main()` do jogo. O motor só abre o blob dentro do `baseEngineInit`: orquestrador `0x104d7a9f8` → `0x104d76544`
//! → `Load` `0x1011e7114` → `0x10111ff98` (`DATABASE.md`, "Carregador de scripts e recarga").
//!
//! As fontes são as da recarga do modder (`carregador::mods_extras`): `r6/scripts` pelo `-compile`, os `.reds` da pasta `Mods/` e os
//! do manifesto dos plugins pelo `-compilePathsFile`. Só compila se a assinatura mudou — caminho, tamanho e mtime de cada `.reds`, dos
//! binários que registram natives e do blob —, gravada em `red4ext/` depois de cada compilação que deu certo. Sem mudança, o custo é andar
//! as pastas e comparar o texto com o gravado.
//!
//! Antes de compilar, as `native func` dos mods de fora do bundle do BWMS são conferidas contra os binários (`carregador::barrados`): o mod da
//! `Mods/` ou do manifesto com native sem registro fica fora da compilação; o de `r6/scripts` impede a compilação, porque `r6/scripts` entra
//! inteiro no `scc`.
//!
//! Mod quebrado sai sozinho: o erro do `scc` traz o caminho do arquivo como ele o recebeu, e o mod da `Mods/` ou do manifesto que traz o
//! arquivo sai da compilação, que roda de novo sem ele (`TETO_DE_RODADAS`); os outros mods e o bundle do BWMS entram, e o que saiu fica
//! vermelho (`estado_dos_scripts`). A rodada seguinte pega a cascata (mod que só falha sem o que saiu, por usar algo dele) e os erros de
//! sintaxe, que o `scc` para no 1º arquivo. Erro que aponta arquivo fora desses mods (o bundle do BWMS, mod posto direto em `r6/scripts`)
//! não tira mod nenhum: a abertura fica com a última compilação que deu certo. Entre dois mods, o erro de nome repetido (`SYM_REDEFINITION`)
//! cai no que o `scc` compila depois — `r6/scripts`, depois a lista na ordem dada (a `Mods/` em ordem de caminho, depois o manifesto) —, e
//! esse sai: vence o que vem antes, como no `.archive`. A compilação que deu certo (com mods fora ou não) e a falha ficam gravadas com a
//! assinatura de todas as fontes, os mods que saíram incluídos, e a abertura seguinte com as mesmas fontes repete as linhas e o estado sem
//! rodar o `scc`.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::carregador::{self, Barrado, ModDeScripts};
use crate::mods_do_usuario::pasta_mods::{CorDoMod, EstadosDosMods};

/// A assinatura da última compilação que deu certo (ou da compilação de fora adotada), em `red4ext/`, seguida das linhas que ela deixou
/// no relatório (os mods barrados).
const ASSINATURA: &str = "bwms-scripts-assinatura.txt";
/// A assinatura da última abertura que falhou nas fontes, seguida das linhas do relatório: com a mesma assinatura, a abertura repete as
/// linhas sem rodar o `scc`.
const FALHA: &str = "bwms-scripts-falha.txt";
/// Separa, no registro, a assinatura das linhas do relatório. Toda linha de assinatura tem tabulação, e esta não tem.
const INICIO_DAS_LINHAS: &str = "relatorio";
/// Separa, no registro, as linhas do relatório do estado dos mods (`cor\traiz\tmotivo`). Toda linha do relatório tem ` / `, e esta não tem.
const INICIO_DOS_ESTADOS: &str = "estados";
/// Passadas do `scc` numa abertura, contando a 1ª: cada uma que falha com erro de mod tira o mod e roda de novo. Medido com o `scc` real,
/// o bundle do BWMS, Equipment-EX e Virtual Atelier (relatório `z030/relatorios/C.md`): 0,45–0,75 s por passada no disco interno e no disco
/// do jogo, 1,2 s na 1ª com o cache frio; com 6, o pior caso fica em ~4,5 s, ~7 s frio. Erro de tipo de vários mods sai numa passada só;
/// sintaxe e cascata pedem uma por mod.
const TETO_DE_RODADAS: usize = 6;
/// Os avisos do `scc` (`compiler/src/diagnostics.rs`, não fatais) que dizem que parte de um mod não vale por conflito com outro mod ou com
/// o jogo; os outros avisos (variável sem uso, retorno faltando) não mudam o que entra.
const AVISOS_DE_CONFLITO: [&str; 3] = [
    "only one replacement per method can be active at a time",
    "a field with this name is already defined in the class, this will have no effect",
    "this annotation adds a method that conflicts with an existing method in the class",
];
/// stdout e stderr do `scc`, lidos depois que ele sai e apagados (o `scc` grava o mesmo em `r6/logs/redscript_rCURRENT.log`).
const SAIDA_DO_SCC: &str = "bwms-scc-saida.txt";
/// Teto do `scc`. Medido fora do jogo, no disco do jogo: 0,6–1,1 s por passada com o bundle do BWMS (mais Virtual Atelier e Equipment-EX,
/// e na 1ª execução de uma cópia nova); o teto só pega compilador travado (disco que caiu), sem segurar a abertura do jogo para sempre.
const TETO_DO_SCC: Duration = Duration::from_secs(30);
/// O trio que o `scc` mantém em `r6/cache`: o blob, o vanilla de onde ele compila e o carimbo do blob que ele gravou.
const TRIO: [&str; 3] = ["final.redscripts", "final.redscripts.bk", "final.redscripts.ts"];
const SUFIXO_DA_GUARDA: &str = ".bwms-anterior";
/// Nome que o `scc` acrescenta ao pool de todo blob que compila; o vanilla não tem.
const MARCA_DO_SCC: &[u8] = b"$REDSCRIPT_SIGNATURE";
/// A cópia do `final.redscripts` que o passo 6 do `bwms-install.sh` guarda antes da 1ª compilação.
const ORIGINAL_DO_INSTALADOR: &str = "final.redscripts.bwms-bak";
/// A única linha do `scc` quando uma fonte não abre como texto (`scc/lib/src/lib.rs`, o contexto do `Files::from_dirs`): o
/// `log::error!("{err}")` dele imprime só o contexto de fora do `anyhow`, sem o arquivo.
const FONTES_NAO_CARREGADAS: &str = "Could not load script sources";

#[derive(Debug, PartialEq)]
enum Decisao {
    NadaMudou,
    /// As fontes são as da última falha gravada: o `scc` daria o mesmo erro.
    RepetirFalha,
    /// O blob foi reescrito por uma compilação de fora (o instalador) que já cobre as fontes: grava a assinatura sem compilar.
    AdotarCompilacaoDeFora,
    Compilar,
}

/// Motivo para o usuário, em PT-BR e inglês: no relatório da `Mods/` sai como `"<pt> / <en>"`.
#[derive(Debug, PartialEq)]
struct Motivo {
    pt: String,
    en: String,
}

impl Motivo {
    fn novo(pt: impl Into<String>, en: impl Into<String>) -> Self {
        Self { pt: pt.into(), en: en.into() }
    }
}

/// O bind do blob que continua em `r6/cache` quando a abertura não grava um novo.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BlobEmUso {
    /// Não declara native de mod barrado: o jogo abre com ele.
    Abre,
    /// Declara native de mod barrado, ou não foi lido: o jogo fecha ao ligar os scripts.
    Fecha,
}

impl BlobEmUso {
    /// A frase das linhas de falha, `(pt, en)`; `ate_pt`/`ate_en` dizem o que tira o jogo da queda.
    fn frase(self, ate_pt: &str, ate_en: &str) -> (String, String) {
        match self {
            Self::Abre => {
                ("O jogo abre com os da última compilação que deu certo.".into(), "The game starts with the last scripts that compiled.".into())
            }
            Self::Fecha => (
                format!("Com os scripts que estão em r6/cache, o jogo fecha ao ligar os scripts até {ate_pt}."),
                format!("With the scripts now in r6/cache, the game closes while binding scripts until {ate_en}."),
            ),
        }
    }
}

/// O que tira o jogo da queda quando o mod barrado é da `Mods/` ou do manifesto: ele já fica fora da compilação nova.
const ATE_COMPILAR: (&str, &str) = ("uma compilação dar certo", "a build succeeds");

#[derive(Debug, PartialEq)]
enum Resultado {
    /// As linhas são as que a compilação em vigor deixou (`.reds` da `Mods/` compilados, mods barrados), repetidas a cada abertura: o
    /// relatório da `Mods/` é regravado no boot.
    NadaMudou(Vec<String>),
    Adotou,
    /// Compilou sem os mods barrados; as linhas dizem quantos `.reds` da `Mods/` entraram e quais mods ficaram fora.
    Compilou(Vec<String>),
    /// Sem compilação nova, por erro com `arquivo:linha` nas fontes, por fonte que o `scc` não carrega ou por native sem registro num mod
    /// de `r6/scripts`. Fica gravado em `FALHA` com a assinatura.
    ErroNasFontes(Vec<String>),
    /// Não houve compilação; a próxima abertura tenta de novo.
    NaoRodou(Motivo, BlobEmUso),
}

/// O que a abertura sabe das fontes, numa andada só.
struct Fontes {
    /// Uma linha por `.reds` (`tamanho\tmtime_ns\tcaminho`): os de `r6/scripts` em ordem de caminho, com as `pastas_reds` entre eles,
    /// depois os extras na ordem do passe. A pasta entra porque renomeá-la conserta a falha sem mudar arquivo nenhum.
    assinatura: String,
    /// O `ctime` mais novo da árvore de `r6/scripts`, pastas incluídas: criar, apagar ou renomear muda o da pasta, e reescrever muda o do
    /// arquivo mesmo quando a cópia preserva o mtime.
    ctime_mais_novo_ns: i128,
    /// Os `.reds` de `r6/scripts`, em ordem de caminho.
    scripts: Vec<PathBuf>,
    /// As pastas de `r6/scripts` com nome terminado em `.reds`, em ordem de caminho: o `scc` toma por fonte tudo o que a andada acha com
    /// essa extensão e não lê pasta como arquivo (`compiler/src/source_map.rs`, `dir_file_iter`).
    pastas_reds: Vec<PathBuf>,
}

/// O que a abertura grava de uma compilação: a assinatura das fontes, as linhas que ela deixou no relatório e o estado dos mods que não
/// entraram inteiros.
struct Registro {
    assinatura: String,
    linhas: Vec<String>,
    estados: EstadosDosMods,
}

/// Um diagnóstico do `scc` que aponta posição: o arquivo como o `scc` o recebeu (o caminho absoluto da lista ou de `r6/scripts`) e o texto
/// `arquivo:linha: [CÓDIGO] mensagem`.
#[derive(Debug, Clone, PartialEq)]
struct Diagnostico {
    arquivo: PathBuf,
    texto: String,
}

/// Um mod que saiu da compilação por erro: os erros dele e os mods que já tinham saído quando ele falhou (vazio = falhou com todos; senão,
/// a cascata: ele usa algo de um deles).
#[derive(Debug, Clone, PartialEq)]
struct Quebrado {
    raiz: PathBuf,
    erros: Vec<String>,
    depois_de: Vec<PathBuf>,
}

/// O estado dos mods de `.reds` em vigor: o do blob que o motor vai ligar nesta abertura (`estados_em_vigor`). Escrito uma vez no `on_load`.
static ESTADO_DOS_SCRIPTS: Mutex<EstadosDosMods> = Mutex::new(EstadosDosMods { raizes: Vec::new(), cores: Vec::new(), motivos: Vec::new() });

/// Os mods de `.reds` que não entraram inteiros nesta abertura (vermelho: ficou fora da compilação; amarelo: conflito que o `scc` avisa). O
/// mod que não está aqui entrou inteiro. `mods_do_usuario::pasta_mods::estados_dos_mods` junta isto ao resto do estado de cada mod.
pub(crate) fn estado_dos_scripts() -> EstadosDosMods {
    ESTADO_DOS_SCRIPTS.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Ponto de entrada, no `on_load` depois do gate de versão. Fora do processo do jogo (teste, `dlopen` de checagem) não faz nada.
pub(crate) fn compilar_se_mudou() {
    if !unsafe { crate::inicio::boot::in_game() } {
        return;
    }
    let (Some(jogo), Some(red4)) = (crate::arquivos_do_jogo::game_dir_from_dylib(), crate::dylib_dir()) else { return };
    let t0 = Instant::now();
    let scc = carregador::scc();
    let resultado = executar(Path::new(&jogo), Path::new(&red4), scc.as_deref(), &carregador::texto_do_manifesto());
    let estados = estados_em_vigor(Path::new(&red4), &resultado);
    let vermelhos = estados.cores.iter().filter(|c| **c == CorDoMod::Vermelho).count();
    *ESTADO_DOS_SCRIPTS.lock().unwrap_or_else(PoisonError::into_inner) = estados;
    pasta_do_relatorio_para_a_falha(Path::new(&jogo), &resultado);
    relatar(&resultado, t0.elapsed().as_millis(), vermelhos);
}

/// O estado do blob que fica em `r6/cache`: o da compilação em vigor (`ASSINATURA`); com erro nas fontes, mais os mods que a falha apontou,
/// que nesta abertura também não entram com a versão nova.
fn estados_em_vigor(red4: &Path, resultado: &Resultado) -> EstadosDosMods {
    let mut e = ler_registro(&red4.join(ASSINATURA)).map(|r| r.estados).unwrap_or_default();
    if matches!(resultado, Resultado::ErroNasFontes(_)) {
        if let Some(f) = ler_registro(&red4.join(FALHA)) {
            for i in 0..f.estados.raizes.len() {
                for motivo in f.estados.motivos[i].split('\n') {
                    e.marcar(&f.estados.raizes[i], f.estados.cores[i], motivo);
                }
            }
        }
    }
    e
}

/// Falha sem a pasta `Mods/` cria a pasta: o relatório dela é onde o usuário lê a seção Scripts, e o relatório não sai sem a pasta. Sem
/// ela, o mod de `r6/scripts` que não compila sumiria do jogo e o que declara native sem registro fecharia o jogo a cada abertura, os dois
/// sem mensagem (a linha ficaria em `red4ext/` e no log de mod, que só existe se o usuário o criou). `Compilou` e `NadaMudou` não criam.
fn pasta_do_relatorio_para_a_falha(jogo: &Path, resultado: &Resultado) {
    if !matches!(resultado, Resultado::ErroNasFontes(_) | Resultado::NaoRodou(..)) {
        return;
    }
    if let Err(e) = std::fs::create_dir_all(crate::mods_do_usuario::pasta_mods::pasta(jogo)) {
        crate::log_dev!("[scripts] pasta Mods não criada para o relatório da falha: {e}");
    }
}

/// TweakXL, ArchiveXL e Codeware de terceiro no jogo: o port de Mac em `red4ext/plugins/<nome>/` (onde o RED4ext do Mac carrega plugins) ou os
/// scripts em `r6/scripts/<nome>/` (a cópia do Windows também). Decisão do dono de 2026-10-27: o BWMS faz o que eles fazem, com os nomes que os
/// mods usam, e os dois juntos brigam pelas mesmas classes e ganchos; o instalador recusa (`bwms-install.sh`) e a abertura não compila.
fn frameworks_de_terceiro(jogo: &Path) -> Vec<String> {
    ["TweakXL", "ArchiveXL", "Codeware"]
        .iter()
        .flat_map(|nome| [format!("red4ext/plugins/{nome}"), format!("r6/scripts/{nome}")])
        .filter(|onde| jogo.join(onde).exists())
        .collect()
}

fn executar(jogo: &Path, red4: &Path, scc: Option<&Path>, manifesto: &str) -> Resultado {
    let terceiros = frameworks_de_terceiro(jogo);
    // Sem registro em `FALHA`: a abertura seguinte confere de novo, e sai do caminho assim que as pastas saírem.
    if !terceiros.is_empty() {
        let lista = terceiros.join(", ");
        return Resultado::ErroNasFontes(vec![format!(
            "framework de terceiro no jogo ({lista}): o BWMS já faz o que o TweakXL, o ArchiveXL e o Codeware fazem, com os mesmos nomes, e os dois \
             juntos brigam pelas mesmas classes. Os scripts não foram compilados e o jogo abre com a última compilação que deu certo; mova essas \
             pastas para a Lixeira e abra o jogo de novo / third-party framework in the game ({lista}): BWMS already does what TweakXL, ArchiveXL \
             and Codeware do, under the same names, and both together fight over the same classes. Scripts were not compiled and the game opens \
             with the last build that succeeded; move these folders to the Trash and open the game again"
        )]);
    }
    let scripts = jogo.join("r6/scripts");
    let cache = jogo.join("r6/cache");
    let blob = cache.join(TRIO[0]);
    if recuperar_trio_largado(&cache) {
        crate::log_dev!("[scripts] abertura: guarda de uma compilação interrompida devolvida ao r6/cache");
    }
    let extras = carregador::mods_extras(jogo, manifesto);
    let fontes = fontes(&scripts, &carregador::permitidos(&extras, &[]));
    let binarios = carregador::binarios(red4);
    // A conferência das natives depende dos binários: plugin novo em `red4ext/plugins/` ou dylib nova muda a assinatura.
    let antes_do_blob = format!("{}{}", fontes.assinatura, linhas_dos_binarios(&binarios));
    let atual = format!("{antes_do_blob}{}", linha_do_blob(&blob));
    let salva = ler_registro(&red4.join(ASSINATURA));
    let falha = ler_registro(&red4.join(FALHA));
    let blob_mtime = mtime_ns(&blob);
    let de_fora_cobre = compilacao_de_fora_cobre(carimbo_do_scc(&cache), blob_mtime, fontes.ctime_mais_novo_ns, extras.is_empty());
    crate::log_dev!(
        "[scripts] abertura: {} .reds em r6/scripts, {} mod(s) extra(s); assinatura gravada {}; falha gravada {}; compilação de fora cobre: \
         {de_fora_cobre}",
        fontes.scripts.len(),
        extras.len(),
        if salva.is_some() { "sim" } else { "não" },
        if falha.is_some() { "sim" } else { "não" }
    );
    let decisao = decidir(salva.as_ref().map(|r| r.assinatura.as_str()), falha.as_ref().map(|r| r.assinatura.as_str()), &atual, de_fora_cobre);
    match decisao {
        Decisao::NadaMudou => return Resultado::NadaMudou(salva.map(|r| r.linhas).unwrap_or_default()),
        Decisao::RepetirFalha => return Resultado::ErroNasFontes(falha.map(|r| r.linhas).unwrap_or_default()),
        Decisao::AdotarCompilacaoDeFora | Decisao::Compilar => {}
    }
    let mut conferir = carregador::mods_de_r6_scripts(&scripts, &fontes.scripts);
    conferir.extend(extras.iter().cloned());
    let barrados = match carregador::barrados(&conferir, &binarios) {
        Ok(b) => b,
        Err(e) => {
            return Resultado::NaoRodou(
                Motivo::novo(
                    format!("a dylib do BWMS não foi lida para conferir as natives dos mods ({e}); rode o instalador do BWMS de novo"),
                    format!("the BWMS dylib could not be read to check the mods' natives ({e}); run the BWMS installer again"),
                ),
                BlobEmUso::Abre,
            )
        }
    };
    let (em_scripts, nos_extras): (Vec<Barrado>, Vec<Barrado>) = barrados.into_iter().partition(|b| b.raiz.starts_with(&scripts));
    if !em_scripts.is_empty() {
        let linhas = linhas_de_r6_scripts_barrado(&em_scripts, blob_em_uso(&blob, em_scripts.iter().chain(&nos_extras)));
        gravar_registro(&red4.join(FALHA), &atual, &linhas, &EstadosDosMods::default());
        return Resultado::ErroNasFontes(linhas);
    }
    if decisao == Decisao::AdotarCompilacaoDeFora {
        gravar_registro(&red4.join(ASSINATURA), &atual, &[], &EstadosDosMods::default());
        let _ = std::fs::remove_file(red4.join(FALHA));
        return Resultado::Adotou;
    }
    // Daqui em diante, sem compilação nova, o blob em uso continua; o mod barrado que ele ainda traga derruba o bind.
    let em_uso = || blob_em_uso(&blob, &nos_extras);
    let Some(scc) = scc else {
        return Resultado::NaoRodou(
            Motivo::novo(
                "o compilador do BWMS não foi achado (red4ext/scc); rode o instalador do BWMS de novo",
                "the BWMS compiler was not found (red4ext/scc); run the BWMS installer again",
            ),
            em_uso(),
        );
    };
    let barrados_na_compilacao = linhas_dos_barrados(&nos_extras);
    tirar_carimbo_alheio(&cache, blob_mtime);
    let pasta_mods = crate::mods_do_usuario::pasta_mods::pasta(jogo);
    // Os mods que um erro pode tirar da compilação: os da `Mods/` e do manifesto que a conferência das natives deixou entrar.
    let candidatos: Vec<&ModDeScripts> = extras.iter().filter(|m| !nos_extras.iter().any(|b| b.raiz == m.raiz)).collect();
    let saida = red4.join(SAIDA_DO_SCC);
    let mut quebrados: Vec<Quebrado> = Vec::new();
    // A cascata: os mods que saíram na rodada anterior, quando ela checou os tipos de todas as fontes (o `scc` acusa todo erro de tipo numa
    // passada só); depois de erro de sintaxe ou de fonte que não carrega ele para antes, e a falha seguinte não diz de quem o mod dependia.
    let mut sairam_na_anterior: Vec<PathBuf> = Vec::new();
    let mut rodada = 0;
    loop {
        rodada += 1;
        let restantes: Vec<&ModDeScripts> = candidatos.iter().copied().filter(|m| !quebrados.iter().any(|q| q.raiz == m.raiz)).collect();
        let extras_permitidos: Vec<PathBuf> = restantes.iter().flat_map(|m| m.reds.iter().cloned()).collect();
        let lista = match carregador::gravar_lista(red4, &extras_permitidos) {
            Ok(l) => l,
            Err(e) => {
                return Resultado::NaoRodou(
                    Motivo::novo(
                        format!("a lista de caminhos não foi gravada em {} ({e}); confira se a pasta do jogo aceita escrita e abra o jogo de novo", red4.display()),
                        format!("the path list was not written to {} ({e}); check that the game folder is writable and start the game again", red4.display()),
                    ),
                    em_uso(),
                )
            }
        };
        let mut cmd = carregador::comando_do_scc(scc, &scripts, lista.as_deref());
        let mut passada = passada_do_scc(&cache, &mut cmd, &saida);
        if matches!(&passada, Ok((false, texto)) if backup_recusado(texto)) && repor_backup(&cache) {
            crate::log_dev!("[scripts] abertura: o scc recusou o backup; .bk reposto de {ORIGINAL_DO_INSTALADOR}, segunda passada");
            passada = passada_do_scc(&cache, &mut cmd, &saida);
        }
        let texto = match passada {
            Err(motivo) => return Resultado::NaoRodou(motivo, em_uso()),
            Ok((true, texto)) => {
                let da_pasta_mods = extras_permitidos.iter().filter(|p| p.starts_with(&pasta_mods)).count();
                let avisos = avisos_de_conflito(&texto);
                let mut linhas = Vec::new();
                if da_pasta_mods > 0 {
                    linhas.push(linha_dos_compilados(da_pasta_mods));
                }
                linhas.extend(linhas_dos_quebrados(&quebrados, true));
                linhas.extend(barrados_na_compilacao.iter().cloned());
                linhas.extend(linhas_dos_avisos(&avisos));
                let mut estados = estados_dos_quebrados_e_barrados(&quebrados, &nos_extras);
                for a in &avisos {
                    if let Some(m) = dono(&restantes, &a.arquivo) {
                        estados.marcar(&m.raiz, CorDoMod::Amarelo, &format!("aviso / warning: {}", a.texto));
                    }
                }
                gravar_registro(&red4.join(ASSINATURA), &format!("{antes_do_blob}{}", linha_do_blob(&blob)), &linhas, &estados);
                let _ = std::fs::remove_file(red4.join(FALHA));
                crate::log_dev!("[scripts] abertura: compilou na rodada {rodada}; {} mod(s) fora por erro", quebrados.len());
                return Resultado::Compilou(linhas);
            }
            Ok((false, texto)) => texto,
        };
        let (mut com_posicao, mut sem_posicao) = erros_do_scc(&texto);
        if com_posicao.is_empty() && texto.contains(FONTES_NAO_CARREGADAS) {
            // O `scc` não diz qual fonte não abriu; a mesma leitura sobre as mesmas fontes aponta. A linha dele sai: a do arquivo a explica.
            let culpadas = fontes_que_o_scc_nao_carrega(&fontes, &extras_permitidos);
            if !culpadas.is_empty() {
                (com_posicao, sem_posicao) = (culpadas, Vec::new());
            }
        }
        if com_posicao.is_empty() {
            return Resultado::NaoRodou(motivo_sem_posicao(&texto, &sem_posicao), em_uso());
        }
        let checou_os_tipos = !texto.contains(FONTES_NAO_CARREGADAS) && !com_posicao.iter().any(|d| d.texto.contains("syntax error"));
        let anteriores = std::mem::take(&mut sairam_na_anterior);
        let mut fora_dos_mods = Vec::new();
        for d in &com_posicao {
            match dono(&restantes, &d.arquivo) {
                Some(m) => match quebrados.iter_mut().find(|q| q.raiz == m.raiz) {
                    Some(q) => q.erros.push(d.texto.clone()),
                    None => {
                        sairam_na_anterior.push(m.raiz.clone());
                        quebrados.push(Quebrado { raiz: m.raiz.clone(), erros: vec![d.texto.clone()], depois_de: anteriores.clone() })
                    }
                },
                None => fora_dos_mods.push(d.arquivo.clone()),
            }
        }
        if !checou_os_tipos {
            sairam_na_anterior.clear();
        }
        crate::log_dev!(
            "[scripts] abertura: rodada {rodada} falhou; {} mod(s) fora por erro, {} erro(s) fora dos mods",
            quebrados.len(),
            fora_dos_mods.len()
        );
        if fora_dos_mods.is_empty() && rodada < TETO_DE_RODADAS {
            continue;
        }
        // Sem compilação nova: os erros desta rodada sem dono (ou no teto) e os mods que já tinham saído vão para o relatório.
        let sem_dono: Vec<String> = com_posicao.iter().filter(|d| fora_dos_mods.contains(&d.arquivo)).map(|d| d.texto.clone()).collect();
        let mut linhas = linhas_de_erro_nas_fontes(&sem_dono, &sem_posicao, &barrados_na_compilacao, em_uso());
        let porque = if fora_dos_mods.is_empty() { linha_do_teto() } else { linha_de_erro_fora_dos_mods() };
        linhas.insert(1, porque);
        linhas.extend(linhas_dos_quebrados(&quebrados, false));
        gravar_registro(&red4.join(FALHA), &atual, &linhas, &estados_dos_quebrados_e_barrados(&quebrados, &nos_extras));
        return Resultado::ErroNasFontes(linhas);
    }
}

/// O bind do blob em uso diante dos mods barrados. Mod barrado só sai da compilação nova; o blob que fica em `r6/cache` pode trazê-lo: a
/// compilação de fora (o instalador) leva `r6/scripts` inteiro, e a nossa de quando o plugin dele estava em `red4ext/plugins/`
/// também. Native declarada sem registro derruba o bind com SIGTRAP, então `Fecha` quando o blob declara como `native` uma função com o
/// nome de uma native barrada, ou quando não abre (o motor também não o liga). Sem native barrada, o blob não é lido.
fn blob_em_uso<'a>(blob: &Path, barrados: impl IntoIterator<Item = &'a Barrado>) -> BlobEmUso {
    let nomes: std::collections::HashSet<&str> = barrados.into_iter().flat_map(|b| b.nomes.iter().map(String::as_str)).collect();
    if nomes.is_empty() {
        return BlobEmUso::Abre;
    }
    let Ok(lido) = super::blob::ler(blob) else { return BlobEmUso::Fecha };
    for (i, d) in lido.defs.iter().enumerate() {
        if d.tipo != super::blob::TIPO_FUNCAO {
            continue;
        }
        // O nome no blob traz o módulo na frente e a assinatura atrás (`Modulo.Nome;Tipos`); a regra das natives olha o nome curto.
        let nome = lido.nome(d.nome);
        let nome = nome.split_once(';').map_or(nome, |(n, _)| n);
        let nome = nome.rsplit_once('.').map_or(nome, |(_, n)| n);
        if nomes.contains(nome) && lido.funcao(i).map_or(true, |f| f.nativa()) {
            return BlobEmUso::Fecha;
        }
    }
    BlobEmUso::Abre
}

/// O erro vai para o relatório da `Mods/` (seção Scripts) e para o log de mod; o veredito de dev diz a decisão e o tempo, para o
/// integrador conferir que a linha sai antes do primeiro bind (`[fb] bind orch CHAMADO #0`).
fn relatar(resultado: &Resultado, ms: u128, vermelhos: usize) {
    let linhas = linhas_do_relatorio(resultado);
    for l in &linhas {
        crate::mods_do_usuario::log_de_mod::gravar("bwms", l);
    }
    if matches!(resultado, Resultado::Compilou(_)) {
        crate::mods_do_usuario::log_de_mod::gravar("bwms", &format!("scripts compilados na abertura em {ms} ms"));
    }
    let decisao = match resultado {
        Resultado::NadaMudou(_) => "nada-mudou",
        Resultado::Adotou => "adotou-compilacao-de-fora",
        Resultado::Compilou(_) => "compilou",
        Resultado::ErroNasFontes(_) => "erro-nas-fontes",
        Resultado::NaoRodou(..) => "nao-rodou",
    };
    crate::veredito_dev!("[scripts] VEREDITO-ABERTURA decisao={decisao} ms={ms} linhas={} vermelhos={vermelhos}", linhas.len());
    if !linhas.is_empty() {
        crate::mods_do_usuario::pasta_mods::anexar_ao_relatorio("Scripts", &linhas);
    }
}

/// As linhas da seção Scripts do relatório da `Mods/`, cada uma `"<pt> / <en>"`.
fn linhas_do_relatorio(resultado: &Resultado) -> Vec<String> {
    match resultado {
        Resultado::NadaMudou(linhas) | Resultado::Compilou(linhas) | Resultado::ErroNasFontes(linhas) => linhas.clone(),
        Resultado::Adotou => Vec::new(),
        Resultado::NaoRodou(m, em_uso) => {
            let (pt, en) = em_uso.frase(ATE_COMPILAR.0, ATE_COMPILAR.1);
            vec![format!("Os scripts mudaram e não foram compilados: {}. {pt} / Scripts changed and were not compiled: {}. {en}", m.pt, m.en)]
        }
    }
}

/// A linha da compilação que deu certo com `.reds` da pasta `Mods/`: o relatório de cada mod dessa pasta aponta a seção Scripts.
fn linha_dos_compilados(n: usize) -> String {
    format!("compilados / built: {n} .reds da pasta Mods / {n} .reds from the Mods folder")
}

/// Cada mod barrado e as natives dele, uma linha cada, como as linhas de mod do relatório da `Mods/`.
fn linhas_dos_barrados(barrados: &[Barrado]) -> Vec<String> {
    let mut linhas = Vec::new();
    for b in barrados {
        linhas.push(format!(
            "fora / left out: {} — o mod precisa de um plugin de Windows que o BWMS não tem; tire-o da pasta / the mod needs a Windows \
             plugin that BWMS does not have; remove it from the folder",
            b.raiz.display()
        ));
        linhas.extend(b.natives.iter().map(|n| format!("native sem registro / unregistered native: {n}")));
    }
    linhas
}

/// `r6/scripts` entra inteiro no `scc`: o mod barrado de lá impede a compilação até sair da pasta.
fn linhas_de_r6_scripts_barrado(barrados: &[Barrado], em_uso: BlobEmUso) -> Vec<String> {
    let (pt, en) = em_uso.frase("o mod sair de r6/scripts", "the mod leaves r6/scripts");
    std::iter::once(format!(
        "Os scripts não foram compilados: um mod de r6/scripts declara native de um plugin de Windows que o BWMS não tem, e com ele o jogo \
         fecha ao ligar os scripts. {pt} / Scripts were not compiled: a mod in r6/scripts declares a native from a Windows plugin that BWMS \
         does not have, and with it the game closes while binding scripts. {en}"
    ))
    .chain(linhas_dos_barrados(barrados))
    .collect()
}

fn linhas_de_erro_nas_fontes(com_posicao: &[String], sem_posicao: &[String], barrados: &[String], em_uso: BlobEmUso) -> Vec<String> {
    let (pt, en) = em_uso.frase(ATE_COMPILAR.0, ATE_COMPILAR.1);
    std::iter::once(format!(
        "Os scripts não compilaram. {pt} Corrija o arquivo abaixo, ou tire da pasta o mod dele, e abra o jogo de novo. / Scripts did not \
         compile. {en} Fix the file below, or remove its mod from the folder, and start the game again."
    ))
    .chain(com_posicao.iter().chain(sem_posicao).map(|e| format!("erro / error: {e}")))
    .chain(barrados.iter().cloned())
    .collect()
}

/// A 2ª linha da falha cujo erro aponta arquivo fora dos mods que podem sair.
fn linha_de_erro_fora_dos_mods() -> String {
    "O erro aponta um arquivo fora da pasta Mods (o BWMS em r6/scripts/blackwall-mods ou um mod posto direto em r6/scripts): esse o BWMS não \
     tira sozinho; se o arquivo é do BWMS, rode o instalador do BWMS de novo. / The error points to a file outside the Mods folder (BWMS in \
     r6/scripts/blackwall-mods or a mod put directly in r6/scripts): BWMS does not remove that one by itself; if the file belongs to BWMS, run \
     the BWMS installer again."
        .to_string()
}

/// A 2ª linha da falha que chegou ao teto de rodadas com mods ainda falhando.
fn linha_do_teto() -> String {
    format!(
        "Depois de {TETO_DE_RODADAS} compilações tirando os mods com erro, ainda havia erro: tire da pasta os mods abaixo. / After \
         {TETO_DE_RODADAS} builds removing the mods with errors, there were still errors: remove the mods below from the folder."
    )
}

/// Cada mod que saiu por erro, com os erros dele e, na cascata, os mods de que ele dependia. `compilou`: a compilação sem ele deu certo e
/// os outros entraram; senão, o jogo ficou com a última compilação que deu certo e a linha só diz que ele não compilou.
fn linhas_dos_quebrados(quebrados: &[Quebrado], compilou: bool) -> Vec<String> {
    let mut linhas = Vec::new();
    for q in quebrados {
        linhas.push(if compilou {
            format!(
                "vermelho / red: {} — não compilou e ficou fora; os outros mods e o BWMS entraram. Corrija o arquivo abaixo, ou tire o mod da \
                 pasta, e abra o jogo de novo / did not compile and was left out; the other mods and BWMS loaded. Fix the file below, or remove \
                 the mod from the folder, and start the game again",
                q.raiz.display()
            )
        } else {
            format!("vermelho / red: {} — não compilou / did not compile", q.raiz.display())
        });
        linhas.extend(q.erros.iter().map(|e| format!("erro / error: {e}")));
        if let Some(cascata) = linha_da_cascata(q) {
            linhas.push(cascata);
        }
    }
    linhas
}

fn linha_da_cascata(q: &Quebrado) -> Option<String> {
    if q.depois_de.is_empty() {
        return None;
    }
    let antes: Vec<String> = q.depois_de.iter().map(|p| p.display().to_string()).collect();
    let antes = antes.join(", ");
    Some(format!(
        "só falhou depois que saiu {antes}: ele usa algo de lá e volta quando aquele mod for consertado / only failed after {antes} left: it \
         uses something from there and comes back once that mod is fixed"
    ))
}

/// Os avisos de conflito (`AVISOS_DE_CONFLITO`), um por linha.
fn linhas_dos_avisos(avisos: &[Diagnostico]) -> Vec<String> {
    avisos
        .iter()
        .map(|a| {
            format!(
                "aviso / warning: {} — conflito com outro mod ou com o jogo: parte deste mod pode não valer / conflict with another mod or with \
                 the game: part of this mod may not apply",
                a.texto
            )
        })
        .collect()
}

/// O estado dos mods que ficaram fora da compilação: vermelho, com os erros (e a cascata) ou as natives sem registro.
fn estados_dos_quebrados_e_barrados(quebrados: &[Quebrado], barrados: &[Barrado]) -> EstadosDosMods {
    let mut e = EstadosDosMods::default();
    for q in quebrados {
        for erro in &q.erros {
            e.marcar(&q.raiz, CorDoMod::Vermelho, &format!("não compilou / did not compile: {erro}"));
        }
        if let Some(cascata) = linha_da_cascata(q) {
            e.marcar(&q.raiz, CorDoMod::Vermelho, &cascata);
        }
    }
    for b in barrados {
        for n in &b.natives {
            e.marcar(
                &b.raiz,
                CorDoMod::Vermelho,
                &format!("precisa de um plugin de Windows que o BWMS não tem / needs a Windows plugin BWMS does not have: {n}"),
            );
        }
    }
    e
}

/// O mod de `mods` que traz `arquivo`: o `scc` imprime o caminho como o recebeu na lista. Um caminho debaixo da raiz também serve (arquivo
/// solto na `Mods/` é a própria raiz).
fn dono<'a>(mods: &[&'a ModDeScripts], arquivo: &Path) -> Option<&'a ModDeScripts> {
    mods.iter().copied().find(|m| m.reds.iter().any(|r| r == arquivo)).or_else(|| mods.iter().copied().find(|m| arquivo.starts_with(&m.raiz)))
}

/// As fontes que o `scc` não carrega, uma por item de erro (`arquivo: <pt> / <en>`): as `pastas_reds` e os `.reds` (de `r6/scripts` e os
/// extras do passe) cujos bytes não são UTF-8, que o `read_to_string` dele recusa. Todas, não só a primeira: cada uma derruba a compilação,
/// e apontar uma por abertura pediria uma abertura com falha por arquivo. Arquivo que não abre fica de fora (a ação seria outra).
fn fontes_que_o_scc_nao_carrega(fontes: &Fontes, extras: &[PathBuf]) -> Vec<Diagnostico> {
    let pastas = fontes.pastas_reds.iter().map(|p| Diagnostico {
        arquivo: p.clone(),
        texto: format!(
            "{}: é uma pasta com nome terminado em .reds; renomeie a pasta ou tire o mod de r6/scripts / is a folder whose name ends in \
             .reds; rename the folder or remove the mod from r6/scripts",
            p.display()
        ),
    });
    let fora_do_utf8 = fontes.scripts.iter().chain(extras).filter(|p| std::fs::read(p).is_ok_and(|b| std::str::from_utf8(&b).is_err()));
    let arquivos = fora_do_utf8.map(|p| Diagnostico {
        arquivo: p.clone(),
        texto: format!(
            "{}: não está em UTF-8; salve o arquivo em UTF-8 ou tire o mod da pasta / is not UTF-8; save it as UTF-8 or remove the mod from \
             the folder",
            p.display()
        ),
    });
    pastas.chain(arquivos).collect()
}

/// O `scc` recusou o `.bk`: com a marca dele (um blob compilado tomado por vanilla) ou ausente com o `.ts` presente.
fn backup_recusado(saida: &str) -> bool {
    saida.contains("The REDScript backup has been corrupted") || saida.contains("backup files are missing")
}

/// A ação para a falha do `scc` que não aponta `arquivo:linha`, pelo que ele disse. A linha crua dele entra como dado, menos a do backup,
/// que manda verificar os arquivos do jogo sem rodar o instalador de novo, e isso desfaz a instalação do BWMS.
fn motivo_sem_posicao(saida: &str, erros: &[String]) -> Motivo {
    let (crua_pt, crua_en) = if erros.is_empty() { ("sem saída".to_string(), "no output".to_string()) } else { (erros.join("; "), erros.join("; ")) };
    if backup_recusado(saida) {
        Motivo::novo(
            "o backup dos scripts originais do jogo em r6/cache está estragado, e a cópia que o instalador do BWMS guarda não serviu para \
             repô-lo; verifique os arquivos do jogo na Steam ou na GOG e rode o instalador do BWMS de novo",
            "the game's original scripts backup in r6/cache is damaged, and the copy the BWMS installer keeps could not restore it; verify \
             the game files in Steam or GOG and run the BWMS installer again",
        )
    } else if saida.contains("Could not load the scc shared library") {
        Motivo::novo(
            format!("o compilador do BWMS está incompleto ({crua_pt}); rode o instalador do BWMS de novo"),
            format!("the BWMS compiler is incomplete ({crua_en}); run the BWMS installer again"),
        )
    } else {
        Motivo::novo(
            format!("o compilador parou sem apontar arquivo ({crua_pt}); confira se o disco do jogo está conectado e tem espaço livre, e abra o jogo de novo"),
            format!("the compiler stopped without pointing to a file ({crua_en}); check that the game disk is connected and has free space, and start the game again"),
        )
    }
}

fn ns(segundos: i64, nanos: i64) -> i128 {
    segundos as i128 * 1_000_000_000 + nanos as i128
}

fn mtime_ns(caminho: &Path) -> Option<i128> {
    std::fs::metadata(caminho).ok().map(|m| ns(m.mtime(), m.mtime_nsec()))
}

fn linha(caminho: &Path, m: &std::fs::Metadata) -> String {
    format!("{}\t{}\t{}\n", m.len(), ns(m.mtime(), m.mtime_nsec()), caminho.display())
}

/// As linhas dos binários que registram natives, depois das fontes; binário que não existe fica sem linha.
fn linhas_dos_binarios(binarios: &[PathBuf]) -> String {
    binarios.iter().filter_map(|b| std::fs::metadata(b).ok().map(|m| linha(b, &m))).collect()
}

/// A última linha da assinatura: o blob que a compilação deixou.
fn linha_do_blob(blob: &Path) -> String {
    match std::fs::metadata(blob) {
        Ok(m) => format!("blob\t{}\t{}\n", m.len(), ns(m.mtime(), m.mtime_nsec())),
        Err(_) => "blob\tausente\n".to_string(),
    }
}

/// Lê o registro que `gravar_registro` deixou; o arquivo sem a linha `INICIO_DAS_LINHAS` é só assinatura, e sem `INICIO_DOS_ESTADOS`, sem
/// estado (todo mod de `.reds` entrou inteiro).
fn ler_registro(caminho: &Path) -> Option<Registro> {
    let texto = std::fs::read_to_string(caminho).ok()?;
    let separador = format!("\n{INICIO_DAS_LINHAS}\n");
    let Some((assinatura, resto)) = texto.split_once(&separador) else {
        return Some(Registro { assinatura: texto, linhas: Vec::new(), estados: EstadosDosMods::default() });
    };
    let (linhas, estados) = resto.split_once(&format!("\n{INICIO_DOS_ESTADOS}\n")).unwrap_or((resto, ""));
    Some(Registro {
        assinatura: format!("{assinatura}\n"),
        linhas: linhas.lines().map(str::to_string).collect(),
        estados: ler_estados(estados),
    })
}

/// Uma linha por motivo: `cor\traiz\tmotivo`. Linha fora do formato fica de fora.
fn ler_estados(texto: &str) -> EstadosDosMods {
    let mut e = EstadosDosMods::default();
    for l in texto.lines() {
        let mut partes = l.splitn(3, '\t');
        let (Some(cor), Some(raiz), Some(motivo)) = (partes.next(), partes.next(), partes.next()) else { continue };
        let Some(cor) = CorDoMod::do_nome(cor) else { continue };
        e.marcar(Path::new(raiz), cor, motivo);
    }
    e
}

fn texto_dos_estados(e: &EstadosDosMods) -> String {
    let mut t = String::new();
    for i in 0..e.raizes.len() {
        for motivo in e.motivos[i].split('\n') {
            t.push_str(&format!("{}\t{}\t{}\n", e.cores[i].nome(), e.raizes[i].display(), motivo.replace('\t', " ")));
        }
    }
    t
}

fn gravar_registro(caminho: &Path, assinatura: &str, linhas: &[String], estados: &EstadosDosMods) {
    let mut texto = assinatura.to_string();
    if !linhas.is_empty() || !estados.raizes.is_empty() {
        texto.push_str(INICIO_DAS_LINHAS);
        texto.push('\n');
        texto.push_str(&linhas.join("\n"));
    }
    if !estados.raizes.is_empty() {
        texto.push('\n');
        texto.push_str(INICIO_DOS_ESTADOS);
        texto.push('\n');
        texto.push_str(&texto_dos_estados(estados));
    }
    if let Err(e) = std::fs::write(caminho, texto) {
        crate::log_dev!("[scripts] registro não gravado em {}: {e}", caminho.display());
    }
}

/// Anda `r6/scripts` como o `scc` (que segue link de pasta: `follow_links(true)`), sem repetir pasta já vista — um link que aponta para
/// cima faria a volta sem fim —, e junta os extras que existem.
fn fontes(scripts: &Path, extras: &[PathBuf]) -> Fontes {
    let mut achados = Vec::new();
    let mut ctime_mais_novo_ns = 0;
    let mut vistas = std::collections::HashSet::new();
    let mut pendentes = vec![scripts.to_path_buf()];
    while let Some(dir) = pendentes.pop() {
        let Ok(m) = std::fs::metadata(&dir) else { continue };
        if !vistas.insert((m.dev(), m.ino())) {
            continue;
        }
        ctime_mais_novo_ns = ctime_mais_novo_ns.max(ns(m.ctime(), m.ctime_nsec()));
        let Ok(entradas) = std::fs::read_dir(&dir) else { continue };
        for e in entradas.flatten() {
            let caminho = e.path();
            let Ok(m) = std::fs::metadata(&caminho) else { continue };
            let reds = caminho.extension().is_some_and(|x| x == "reds");
            if m.is_dir() {
                if reds {
                    achados.push((caminho.clone(), m));
                }
                pendentes.push(caminho);
            } else if m.is_file() && reds {
                ctime_mais_novo_ns = ctime_mais_novo_ns.max(ns(m.ctime(), m.ctime_nsec()));
                achados.push((caminho, m));
            }
        }
    }
    achados.sort_by(|a, b| a.0.as_os_str().as_encoded_bytes().cmp(b.0.as_os_str().as_encoded_bytes()));
    let mut assinatura = String::new();
    for (caminho, m) in &achados {
        assinatura.push_str(&linha(caminho, m));
    }
    for caminho in extras {
        if let Ok(m) = std::fs::metadata(caminho) {
            assinatura.push_str(&linha(caminho, &m));
        }
    }
    let (pastas, arquivos): (Vec<_>, Vec<_>) = achados.into_iter().partition(|(_, m)| m.is_dir());
    Fontes {
        assinatura,
        ctime_mais_novo_ns,
        scripts: arquivos.into_iter().map(|(caminho, _)| caminho).collect(),
        pastas_reds: pastas.into_iter().map(|(caminho, _)| caminho).collect(),
    }
}

/// O carimbo que o `scc` grava em `final.redscripts.ts` depois de salvar o blob: o mtime dele em ns, `u128` little-endian
/// (`scc/lib/src/timestamp.rs`). Arquivo vazio ou curto = sem carimbo.
fn carimbo_do_scc(cache: &Path) -> Option<i128> {
    let bytes = std::fs::read(cache.join(TRIO[2])).ok()?;
    let bytes: [u8; 16] = bytes.get(..16)?.try_into().ok()?;
    i128::try_from(u128::from_le_bytes(bytes)).ok()
}

/// A compilação de fora cobre as fontes quando o carimbo do `scc` é o mtime do blob (ele gravou este blob, terminou bem e ninguém o
/// reescreveu depois), nada na árvore de `r6/scripts` mudou depois disso e não há fonte fora de `r6/scripts`, que o
/// instalador não passa ao `scc`.
fn compilacao_de_fora_cobre(carimbo: Option<i128>, blob_mtime_ns: Option<i128>, ctime_mais_novo_ns: i128, sem_extras: bool) -> bool {
    match (carimbo, blob_mtime_ns) {
        (Some(c), Some(b)) => sem_extras && c == b && ctime_mais_novo_ns <= b,
        _ => false,
    }
}

/// `salva` é a assinatura da compilação em vigor e `falha`, a da última falha nas fontes; a última linha de cada uma é o blob. Fontes
/// iguais às da falha repetem a falha. Blob igual ao gravado e assinatura diferente = mudaram as fontes, e só compilando o blob as
/// acompanha; blob diferente = alguém o reescreveu, e a compilação de fora pode ser adotada.
fn decidir(salva: Option<&str>, falha: Option<&str>, atual: &str, de_fora_cobre: bool) -> Decisao {
    if salva == Some(atual) {
        return Decisao::NadaMudou;
    }
    if falha == Some(atual) {
        return Decisao::RepetirFalha;
    }
    let blob_e_o_gravado = salva.and_then(|s| s.lines().last()) == atual.lines().last();
    if !blob_e_o_gravado && de_fora_cobre {
        Decisao::AdotarCompilacaoDeFora
    } else {
        Decisao::Compilar
    }
}

fn contem(dados: &[u8], agulha: &[u8]) -> bool {
    dados.windows(agulha.len()).any(|j| j == agulha)
}

/// Um blob compilado sem o carimbo do `scc` — copiado para o cache por fora dele, como a aplicação a quente faz — seria tomado por
/// vanilla novo: o `scc` o copiaria por cima do `.bk` e recusaria compilar ("backup has been corrupted"). Sem o `.ts` e com o `.bk` no
/// lugar, o `scc` compila do `.bk`. Blob sem a marca é vanilla novo (atualização do jogo) e fica com o `scc`, que renova o `.bk`.
fn tirar_carimbo_alheio(cache: &Path, blob_mtime_ns: Option<i128>) {
    let carimbo = carimbo_do_scc(cache);
    if carimbo.is_none() || carimbo == blob_mtime_ns || !cache.join(TRIO[1]).is_file() {
        return;
    }
    if std::fs::read(cache.join(TRIO[0])).is_ok_and(|b| contem(&b, MARCA_DO_SCC)) {
        let _ = std::fs::remove_file(cache.join(TRIO[2]));
    }
}

/// Repõe o `.bk` que o `scc` recusou pela cópia do instalador, quando ela não tem a marca do `scc`, e tira o `.ts`: sem carimbo e com o
/// `.bk` no lugar, o `scc` compila do `.bk`. É o conserto que o `bwms-fastboot.sh` faz antes de compilar. `false` sem a cópia, com a cópia
/// compilada ou sem escrita no `r6/cache`.
fn repor_backup(cache: &Path) -> bool {
    let original = cache.join(ORIGINAL_DO_INSTALADOR);
    if !std::fs::read(&original).is_ok_and(|b| !contem(&b, MARCA_DO_SCC)) || std::fs::copy(&original, cache.join(TRIO[1])).is_err() {
        return false;
    }
    let _ = std::fs::remove_file(cache.join(TRIO[2]));
    true
}

fn guarda_de(cache: &Path, nome: &str) -> PathBuf {
    cache.join(format!("{nome}{SUFIXO_DA_GUARDA}"))
}

/// Guarda o trio antes de compilar. O `scc` só reescreve o blob depois de compilar, mas morto no meio da escrita (teto) ou com disco
/// cheio deixa o blob truncado; e devolver só o blob, sem o `.bk` e o `.ts` daquele momento, faz o próximo `scc` tomar o blob compilado
/// por vanilla. `fs::copy` no APFS é `clonefile`: sem dado copiado, com o mtime junto. Devolve quais peças existiam.
fn guardar_trio(cache: &Path) -> Result<[bool; 3], String> {
    let mut existia = [false; 3];
    for (i, nome) in TRIO.iter().enumerate() {
        let guarda = guarda_de(cache, nome);
        let _ = std::fs::remove_file(&guarda);
        if cache.join(nome).is_file() {
            std::fs::copy(cache.join(nome), &guarda).map_err(|e| format!("{nome}: {e}"))?;
            existia[i] = true;
        }
    }
    Ok(existia)
}

/// O trio volta como estava: cada guarda volta por `rename`, e a peça que não existia antes de compilar sai.
fn devolver_trio(cache: &Path, existia: [bool; 3]) {
    for (i, nome) in TRIO.iter().enumerate() {
        if existia[i] {
            let _ = std::fs::rename(guarda_de(cache, nome), cache.join(nome));
        } else {
            let _ = std::fs::remove_file(cache.join(nome));
        }
    }
}

fn soltar_trio(cache: &Path) {
    for nome in TRIO {
        let _ = std::fs::remove_file(guarda_de(cache, nome));
    }
}

/// Guarda que sobrou no `r6/cache` é de uma compilação que não terminou: processo morto durante o `scc` (fim forçado, queda de energia).
/// O `scc` grava o `.ts` com o mtime do blob depois de salvá-lo inteiro (`scc/lib/src/lib.rs`): com o carimbo casando com o blob, o
/// trio em `r6/cache` é de uma compilação que terminou — a do `scc` órfão ou uma depois dela, como a do passo 6 do instalador — e só as
/// guardas saem. Sem casar, o blob pode estar truncado, e o motor não abre blob truncado: a guarda volta por cima, e o custo é compilar
/// de novo. Peça sem guarda fica como está. Devolve se alguma guarda voltou.
fn recuperar_trio_largado(cache: &Path) -> bool {
    if !TRIO.iter().any(|n| guarda_de(cache, n).exists()) {
        return false;
    }
    if matches!((carimbo_do_scc(cache), mtime_ns(&cache.join(TRIO[0]))), (Some(c), Some(b)) if c == b) {
        crate::log_dev!("[scripts] abertura: guarda largada solta; o trio do r6/cache é de uma compilação que terminou");
        soltar_trio(cache);
        return false;
    }
    let mut devolveu = false;
    for nome in TRIO {
        devolveu |= std::fs::rename(guarda_de(cache, nome), cache.join(nome)).is_ok();
    }
    devolveu
}

/// Uma passada do `scc` com o trio guardado: compilou, o trio novo fica; falhou ou não rodou, o trio volta como estava. Devolve se
/// compilou e a saída dele.
fn passada_do_scc(cache: &Path, cmd: &mut Command, saida: &Path) -> Result<(bool, String), Motivo> {
    let existia = guardar_trio(cache).map_err(|e| {
        soltar_trio(cache);
        Motivo::novo(
            format!("não deu para guardar os scripts atuais ({e}); libere espaço no disco do jogo e abra o jogo de novo"),
            format!("the current scripts could not be backed up ({e}); free space on the game disk and start the game again"),
        )
    })?;
    let rodou = rodar_com_teto(cmd, saida, TETO_DO_SCC);
    let texto = std::fs::read_to_string(saida).unwrap_or_default();
    let _ = std::fs::remove_file(saida);
    match rodou {
        Ok(true) => {
            soltar_trio(cache);
            Ok((true, texto))
        }
        Ok(false) => {
            devolver_trio(cache, existia);
            Ok((false, texto))
        }
        Err(motivo) => {
            devolver_trio(cache, existia);
            Err(motivo)
        }
    }
}

/// Roda o `scc` com stdout e stderr em `saida`, esperando no máximo `teto`. Passou do teto, o processo leva `kill` (SIGKILL) e é
/// esperado: a espera volta, porque SIGKILL não é ignorável.
fn rodar_com_teto(cmd: &mut Command, saida: &Path, teto: Duration) -> Result<bool, Motivo> {
    let arquivo = std::fs::File::create(saida).map_err(|e| {
        Motivo::novo(
            format!("a saída do compilador não foi gravada em {} ({e}); confira se a pasta do jogo aceita escrita", saida.display()),
            format!("the compiler output was not written to {} ({e}); check that the game folder is writable", saida.display()),
        )
    })?;
    let erro = arquivo.try_clone().map_err(|e| {
        Motivo::novo(
            format!("a saída do compilador não foi gravada ({e}); abra o jogo de novo"),
            format!("the compiler output was not written ({e}); start the game again"),
        )
    })?;
    let mut filho = cmd.stdin(Stdio::null()).stdout(arquivo).stderr(erro).spawn().map_err(|e| {
        Motivo::novo(
            format!("o compilador não abriu ({e}); rode o instalador do BWMS de novo"),
            format!("the compiler did not start ({e}); run the BWMS installer again"),
        )
    })?;
    let inicio = Instant::now();
    loop {
        match filho.try_wait() {
            Ok(Some(estado)) => return Ok(estado.success()),
            Ok(None) if inicio.elapsed() < teto => std::thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = filho.kill();
                let _ = filho.wait();
                return Err(Motivo::novo(
                    format!("o compilador passou de {} s e foi parado; abra o jogo de novo", teto.as_secs()),
                    format!("the compiler ran past {} s and was stopped; start the game again", teto.as_secs()),
                ));
            }
            Err(e) => {
                let _ = filho.kill();
                let _ = filho.wait();
                return Err(Motivo::novo(
                    format!("o compilador não respondeu ({e}); abra o jogo de novo"),
                    format!("the compiler did not respond ({e}); start the game again"),
                ));
            }
        }
    }
}

/// Os erros da saída do `scc`: os que apontam `arquivo:linha` (`arquivo:linha: [CÓDIGO] mensagem`) e os outros. Erro sem posição (I/O,
/// backup corrompido) entra como veio, sem a data; o resumo "REDScript compilation has failed" e a lista de mods dele ficam fora, porque
/// repetem as linhas com posição. Sem nenhuma linha `[ERROR`, a última linha da saída vai para os sem posição.
fn erros_do_scc(saida: &str) -> (Vec<Diagnostico>, Vec<String>) {
    let (com_posicao, mut sem_posicao) = diagnosticos(saida, "ERROR");
    sem_posicao.retain(|t| !t.starts_with("REDScript compilation has failed"));
    if com_posicao.is_empty() && sem_posicao.is_empty() {
        if let Some(ultima) = saida.lines().rev().map(|l| l.trim()).find(|l| !l.is_empty()) {
            sem_posicao.push(ultima.to_string());
        }
    }
    (com_posicao, sem_posicao)
}

/// Os avisos do `scc` que apontam conflito (`AVISOS_DE_CONFLITO`), com a posição.
fn avisos_de_conflito(saida: &str) -> Vec<Diagnostico> {
    let mut avisos = diagnosticos(saida, "WARN").0;
    avisos.retain(|a| AVISOS_DE_CONFLITO.iter().any(|c| a.texto.contains(c)));
    avisos
}

/// Os diagnósticos de nível `nivel` (`ERROR`, `WARN`) da saída do `scc`: cada um com posição sai em quatro linhas — `[NÍVEL - data] [CÓDIGO]
/// At arquivo:linha:coluna:`, a linha do código, o sublinhado e a mensagem (`compiler/src/diagnostics.rs`); erro de sintaxe e aviso vêm sem
/// `[CÓDIGO]`. O de uma linha só vai para os sem posição, sem a data.
fn diagnosticos(saida: &str, nivel: &str) -> (Vec<Diagnostico>, Vec<String>) {
    let prefixo = format!("[{nivel} - ");
    let linhas: Vec<&str> = saida.lines().collect();
    let (mut com_posicao, mut sem_posicao) = (Vec::new(), Vec::new());
    let mut i = 0;
    while i < linhas.len() {
        let Some((_, texto)) = linhas[i].strip_prefix(prefixo.as_str()).and_then(|r| r.split_once("] ")) else {
            i += 1;
            continue;
        };
        let (codigo, resto) = match texto.strip_prefix('[').and_then(|t| t.split_once("] ")) {
            Some((c, r)) => (format!("[{c}] "), r),
            None => (String::new(), texto),
        };
        match resto.strip_prefix("At ").and_then(|t| t.strip_suffix(':')) {
            Some(local) => {
                let arquivo_e_linha = local.rsplit_once(':').map_or(local, |(a, _)| a);
                let arquivo = arquivo_e_linha.rsplit_once(':').map_or(arquivo_e_linha, |(a, _)| a);
                let mensagem = linhas.get(i + 3).map_or("", |m| m.trim());
                com_posicao.push(Diagnostico { arquivo: PathBuf::from(arquivo), texto: format!("{arquivo_e_linha}: {codigo}{mensagem}") });
                i += 4;
            }
            None => {
                sem_posicao.push(texto.to_string());
                i += 1;
            }
        }
    }
    (com_posicao, sem_posicao)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn pasta_de_teste(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-abertura-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn escrever(raiz: &Path, caminho: &str, conteudo: &str) -> PathBuf {
        let p = raiz.join(caminho);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, conteudo).unwrap();
        p
    }

    /// Framework de terceiro (port de Mac em `red4ext/plugins/` ou scripts em `r6/scripts/`): a abertura não roda o `scc` e diz o que mover.
    #[test]
    fn framework_de_terceiro_para_a_abertura_sem_compilar() {
        let jogo = pasta_de_teste("terceiro");
        let red4 = jogo.join("red4ext");
        escrever(&jogo, "r6/scripts/blackwall-mods/a.reds", "func A() {}\n");
        assert!(frameworks_de_terceiro(&jogo).is_empty());
        escrever(&jogo, "r6/scripts/Codeware/Codeware.reds", "module Codeware\n");
        escrever(&jogo, "red4ext/plugins/TweakXL/TweakXL.dylib", "");
        assert_eq!(frameworks_de_terceiro(&jogo), ["red4ext/plugins/TweakXL", "r6/scripts/Codeware"]);
        let scc = scc_falso(&jogo, "scc", "exit 0");
        match executar(&jogo, &red4, Some(&scc), "") {
            Resultado::ErroNasFontes(linhas) => {
                assert_eq!(linhas.len(), 1, "{linhas:?}");
                assert!(linhas[0].contains("(red4ext/plugins/TweakXL, r6/scripts/Codeware)"), "{}", linhas[0]);
                assert!(linhas[0].contains("Lixeira") && linhas[0].contains("Trash"), "{}", linhas[0]);
            }
            outro => panic!("abertura com framework de terceiro: {outro:?}"),
        }
        assert!(!jogo.join("execucoes").exists(), "o scc não pode rodar");
        assert!(!red4.join(FALHA).exists(), "sem registro de falha: a abertura seguinte confere de novo");
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Um `scc` de mentira em shell: `corpo` roda com `r6` = pai de `r6/scripts` (o `$2` do `-compile`) e conta as execuções em
    /// `<pasta>/execucoes`.
    fn scc_falso(pasta: &Path, nome: &str, corpo: &str) -> PathBuf {
        let p = pasta.join(nome);
        let texto = format!("#!/bin/sh\nr6=\"$(dirname \"$2\")\"\necho x >> \"{}/execucoes\"\n{corpo}\n", pasta.display());
        std::fs::write(&p, texto).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn execucoes(pasta: &Path) -> usize {
        std::fs::read_to_string(pasta.join("execucoes")).map_or(0, |t| t.lines().count())
    }

    fn pausa() {
        std::thread::sleep(Duration::from_millis(5));
    }

    /// Jogo com um `.reds` em `r6/scripts`, blob vanilla e `red4ext/`.
    fn jogo_de_teste(raiz: &Path) -> (PathBuf, PathBuf) {
        let jogo = raiz.join("jogo");
        escrever(&jogo, "r6/scripts/mod/a.reds", "func A() {}");
        escrever(&jogo, "r6/cache/final.redscripts", "vanilla");
        std::fs::create_dir_all(jogo.join("red4ext")).unwrap();
        (jogo.clone(), jogo.join("red4ext"))
    }

    const SCC_BOM: &str = "printf compilado > \"$r6/cache/final.redscripts\"\nexit 0";

    #[test]
    fn decide_pela_assinatura_pela_falha_e_pelo_blob() {
        let atual = "10\t1\t/a.reds\nblob\t5\t9\n";
        let mesmas_fontes_outro_blob = "10\t1\t/a.reds\nblob\t5\t8\n";
        let outras_fontes_mesmo_blob = "11\t2\t/a.reds\nblob\t5\t9\n";
        assert_eq!(decidir(Some(atual), None, atual, false), Decisao::NadaMudou);
        assert_eq!(decidir(Some(atual), None, atual, true), Decisao::NadaMudou);
        assert_eq!(decidir(None, None, atual, false), Decisao::Compilar);
        assert_eq!(decidir(None, None, atual, true), Decisao::AdotarCompilacaoDeFora);
        assert_eq!(decidir(Some(mesmas_fontes_outro_blob), None, atual, true), Decisao::AdotarCompilacaoDeFora);
        assert_eq!(decidir(Some(mesmas_fontes_outro_blob), None, atual, false), Decisao::Compilar);
        // Blob ainda é o da nossa compilação: a mudança está nas fontes (um mod da `Mods/` saiu, por exemplo), e o blob de fora
        // não existe para ser adotado.
        assert_eq!(decidir(Some(outras_fontes_mesmo_blob), None, atual, true), Decisao::Compilar);
        assert_eq!(decidir(Some(outras_fontes_mesmo_blob), Some(atual), atual, true), Decisao::RepetirFalha);
        assert_eq!(decidir(Some(outras_fontes_mesmo_blob), Some(outras_fontes_mesmo_blob), atual, false), Decisao::Compilar);
    }

    #[test]
    fn registro_guarda_a_assinatura_e_as_linhas() {
        let pasta = pasta_de_teste("registro");
        let caminho = pasta.join("r.txt");
        assert!(ler_registro(&caminho).is_none());
        // Caminho de mod chamado `relatorio` não é o separador: a linha de assinatura tem tabulação.
        let assinatura = "1\t2\t/j/Mods/relatorio\nblob\t5\t9\n";
        let sem_estado = EstadosDosMods::default();
        gravar_registro(&caminho, assinatura, &[], &sem_estado);
        assert_eq!(std::fs::read_to_string(&caminho).unwrap(), assinatura);
        let r = ler_registro(&caminho).unwrap();
        assert_eq!((r.assinatura.as_str(), r.linhas.len()), (assinatura, 0));
        let linhas = vec!["um / one".to_string(), "dois / two".to_string()];
        gravar_registro(&caminho, assinatura, &linhas, &sem_estado);
        let r = ler_registro(&caminho).unwrap();
        assert_eq!((r.assinatura.as_str(), r.linhas, r.estados), (assinatura, linhas.clone(), sem_estado));
        // O estado volta com a cor e cada motivo; mod chamado `estados` não é o separador (a linha do relatório tem ` / `).
        let mut estados = EstadosDosMods::default();
        estados.marcar(Path::new("/j/Mods/estados"), CorDoMod::Vermelho, "não compilou / did not compile: /j/Mods/estados/a.reds:1: x");
        estados.marcar(Path::new("/j/Mods/estados"), CorDoMod::Vermelho, "só falhou depois que saiu /j/Mods/A / only failed after /j/Mods/A left");
        estados.marcar(Path::new("/j/Mods/Z"), CorDoMod::Amarelo, "aviso / warning:\tcom tabulação");
        gravar_registro(&caminho, assinatura, &linhas, &estados);
        let r = ler_registro(&caminho).unwrap();
        assert_eq!((r.assinatura.as_str(), &r.linhas), (assinatura, &linhas));
        assert_eq!(r.estados.raizes, estados.raizes);
        assert_eq!(r.estados.cores, [CorDoMod::Vermelho, CorDoMod::Amarelo]);
        assert_eq!(r.estados.motivos[0], estados.motivos[0]);
        assert_eq!(r.estados.motivos[1], "aviso / warning: com tabulação");
        // Só estado, sem linha: também volta.
        gravar_registro(&caminho, assinatura, &[], &estados);
        let r = ler_registro(&caminho).unwrap();
        assert_eq!((r.linhas.len(), r.estados.raizes.len()), (0, 2));
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn compilacao_de_fora_so_cobre_com_carimbo_do_blob_arvore_mais_velha_e_sem_extras() {
        assert!(compilacao_de_fora_cobre(Some(100), Some(100), 90, true));
        assert!(compilacao_de_fora_cobre(Some(100), Some(100), 100, true));
        assert!(!compilacao_de_fora_cobre(Some(100), Some(100), 101, true), "árvore mudou depois do blob");
        assert!(!compilacao_de_fora_cobre(Some(99), Some(100), 90, true), "blob reescrito por fora do scc");
        assert!(!compilacao_de_fora_cobre(Some(100), Some(100), 90, false), "Mods/ e manifesto não entram na compilação de fora");
        assert!(!compilacao_de_fora_cobre(None, Some(100), 90, true));
        assert!(!compilacao_de_fora_cobre(Some(100), None, 90, true));
    }

    #[test]
    fn carimbo_le_u128_little_endian_e_recusa_vazio_e_curto() {
        let cache = pasta_de_teste("carimbo");
        assert_eq!(carimbo_do_scc(&cache), None);
        // O `.ts` do jogo medido ao lado do mtime do blob dele: 1792214318.059072556 s.
        std::fs::write(cache.join(TRIO[2]), [0x2c, 0xac, 0x47, 0x3d, 0x6d, 0x39, 0xdf, 0x18, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
        assert_eq!(carimbo_do_scc(&cache), Some(ns(1_792_214_318, 59_072_556)));
        std::fs::write(cache.join(TRIO[2]), []).unwrap();
        assert_eq!(carimbo_do_scc(&cache), None);
        std::fs::write(cache.join(TRIO[2]), [1, 2, 3]).unwrap();
        assert_eq!(carimbo_do_scc(&cache), None);
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn fontes_seguem_link_de_pasta_como_o_scc_sem_dar_volta() {
        let raiz = pasta_de_teste("fontes");
        let scripts = raiz.join("r6/scripts");
        escrever(&scripts, "b/b.reds", "bb");
        escrever(&scripts, "a.reds", "a");
        escrever(&scripts, "a.reds.bak", "x");
        escrever(&raiz, "fora/f.reds", "fff");
        std::os::unix::fs::symlink(raiz.join("fora"), scripts.join("ligado")).unwrap();
        std::os::unix::fs::symlink(&scripts, scripts.join("b/volta")).unwrap();
        let extra = escrever(&raiz, "Mods/m.reds", "mm");
        let f = fontes(&scripts, &[extra.clone(), raiz.join("sumiu.reds")]);
        let caminhos: Vec<&str> = f.assinatura.lines().map(|l| l.splitn(3, '\t').nth(2).unwrap()).collect();
        let esperado = [scripts.join("a.reds"), scripts.join("b/b.reds"), scripts.join("ligado/f.reds"), extra];
        assert_eq!(caminhos, esperado.iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>());
        assert_eq!(f.scripts, esperado[..3]);
        assert!(f.assinatura.starts_with("1\t"), "tamanho primeiro: {}", f.assinatura);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn ctime_da_arvore_sobe_quando_um_mod_entra_movido_com_datas_antigas() {
        let raiz = pasta_de_teste("ctime");
        let scripts = raiz.join("r6/scripts");
        // O mod baixado (mtime do zip) fica pronto antes de tudo: o ctime dele é mais velho que o de `r6/scripts`.
        let velho = escrever(&raiz, "baixado/mod/velho.reds", "v");
        std::fs::File::options().write(true).open(&velho).unwrap().set_modified(std::time::UNIX_EPOCH + Duration::from_secs(1_600_000_000)).unwrap();
        pausa();
        escrever(&scripts, "a.reds", "a");
        let antes = fontes(&scripts, &[]).ctime_mais_novo_ns;
        pausa();
        // Mover a pasta não muda o ctime dos arquivos de dentro; muda o de `r6/scripts`.
        std::fs::rename(raiz.join("baixado/mod"), scripts.join("mod")).unwrap();
        let depois = fontes(&scripts, &[]);
        assert!(depois.ctime_mais_novo_ns > antes);
        assert!(depois.assinatura.contains("\t1600000000000000000\t"));
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn erros_do_scc_separam_os_que_apontam_arquivo_e_linha() {
        let saida = "\
[INFO - Sat, 17 Oct 2026 23:29:41 -0300] Redscript cache file is not ours, copying it to /j/r6/cache/final.redscripts.bk
[INFO - Sat, 17 Oct 2026 23:29:42 -0300] Compiling files in /j/r6/scripts:
blackwall-mods/a.reds
[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] [UNRESOLVED_FN] At /j/Mods/q/q.reds:4:10:
  return NaoExiste();
         ^^^^^^^^^^^
function 'NaoExiste' not found

[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] At /j/r6/scripts/s.reds:2:1:
func {
^
syntax error, expected one of: identifier
[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] REDScript compilation has failed.
This error has been caused by mods listed below:
- q.reds
";
        let (com, sem) = erros_do_scc(saida);
        assert_eq!(
            com.iter().map(|d| d.texto.as_str()).collect::<Vec<_>>(),
            ["/j/Mods/q/q.reds:4: [UNRESOLVED_FN] function 'NaoExiste' not found", "/j/r6/scripts/s.reds:2: syntax error, expected one of: identifier"]
        );
        assert_eq!(com.iter().map(|d| d.arquivo.as_path()).collect::<Vec<_>>(), [Path::new("/j/Mods/q/q.reds"), Path::new("/j/r6/scripts/s.reds")]);
        assert!(sem.is_empty());
        // Caminho com `:` e espaço: o `scc` imprime o caminho como o recebeu, e a posição sai pelos dois últimos `:`.
        let estranho = "[ERROR - x] [SYM_REDEFINITION] At /j/Mods/Mod: B v2/b.reds:1:1:\npublic class C {}\n^^^\nthe name of this type conflicts with a type defined at '/j/Mods/A/a.reds:1:1'\n";
        assert_eq!(erros_do_scc(estranho).0[0].arquivo, Path::new("/j/Mods/Mod: B v2/b.reds"));
        // Aviso de conflito com posição; o aviso que não é de conflito e o erro ficam fora.
        let avisos = "\
[WARN - x] At /j/Mods/B/b.reds:1:1:
@replaceMethod(PlayerPuppet)
^^^
this method replacement overwrites a previous annotation targeting the same method, only one replacement per method can be active at a time
[WARN - x] At /j/Mods/B/b.reds:7:5:
  let x = 1;
  ^
this variable is never used
";
        let a = avisos_de_conflito(avisos);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].arquivo, Path::new("/j/Mods/B/b.reds"));
        assert!(a[0].texto.starts_with("/j/Mods/B/b.reds:1: this method replacement overwrites"));
        assert!(erros_do_scc(avisos).0.is_empty());
        let corrompido = "[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] The REDScript backup has been corrupted, try removing everything\n";
        assert_eq!(erros_do_scc(corrompido), (vec![], vec!["The REDScript backup has been corrupted, try removing everything".to_string()]));
        assert_eq!(erros_do_scc("Could not load the scc shared library\n\n"), (vec![], vec!["Could not load the scc shared library".to_string()]));
        assert_eq!(erros_do_scc(""), (vec![], vec![]));
    }

    #[test]
    fn falha_sem_posicao_leva_a_acao_que_resolve() {
        let backup = "[ERROR - x] The REDScript backup has been corrupted, try removing everything in the 'r6/cache' directory and verify game files with Steam/GOG";
        let m = motivo_sem_posicao(backup, &erros_do_scc(backup).1);
        assert!(m.pt.contains("rode o instalador do BWMS de novo") && m.en.contains("run the BWMS installer again"));
        assert!(!m.pt.contains("try removing") && !m.en.contains("try removing"), "a linha crua do scc fica fora: {}", m.en);
        let ts = "[ERROR - x] A REDScript timestamp was found, but backup files are missing, your installation might be corrupted";
        assert_eq!(motivo_sem_posicao(ts, &[]), m);
        let lib = "Could not load the scc shared library";
        let m = motivo_sem_posicao(lib, &erros_do_scc(lib).1);
        assert!(m.pt.contains("incompleto") && m.pt.contains(lib) && m.pt.contains("rode o instalador"));
        let io = "[ERROR - x] There's been an I/O error: No space left on device";
        let m = motivo_sem_posicao(io, &erros_do_scc(io).1);
        assert!(m.pt.contains("espaço livre") && m.pt.contains("No space left") && m.en.contains("free space"));
        assert!(motivo_sem_posicao("", &[]).pt.contains("sem saída"));
    }

    #[test]
    fn linhas_do_relatorio_tem_pt_e_en() {
        let barrado = Barrado {
            raiz: PathBuf::from("/j/Mods/ModWindows"),
            natives: vec!["/j/Mods/ModWindows/a.reds:3 NaoRegistrada".into()],
            nomes: vec!["NaoRegistrada".into()],
        };
        let um = std::slice::from_ref(&barrado);
        let quebrado = Quebrado {
            raiz: PathBuf::from("/j/Mods/B"),
            erros: vec!["/j/Mods/B/b.reds:3: [UNRESOLVED_TYPE] x".into()],
            depois_de: vec![PathBuf::from("/j/Mods/A")],
        };
        let resultados = [
            Resultado::Compilou(std::iter::once(linha_dos_compilados(2)).chain(linhas_dos_barrados(um)).collect()),
            Resultado::ErroNasFontes(linhas_de_r6_scripts_barrado(um, BlobEmUso::Abre)),
            Resultado::ErroNasFontes(linhas_de_r6_scripts_barrado(um, BlobEmUso::Fecha)),
            Resultado::ErroNasFontes(linhas_de_erro_nas_fontes(&["/j/a.reds:1: x".into()], &["sem posição".into()], &[], BlobEmUso::Abre)),
            Resultado::ErroNasFontes(linhas_de_erro_nas_fontes(&["/j/a.reds:1: x".into()], &[], &[], BlobEmUso::Fecha)),
            Resultado::NaoRodou(Motivo::novo("motivo", "reason"), BlobEmUso::Abre),
            Resultado::NaoRodou(Motivo::novo("motivo", "reason"), BlobEmUso::Fecha),
            Resultado::Compilou(linhas_dos_quebrados(std::slice::from_ref(&quebrado), true)),
            Resultado::ErroNasFontes(linhas_dos_quebrados(std::slice::from_ref(&quebrado), false)),
            Resultado::Compilou(linhas_dos_avisos(&[Diagnostico { arquivo: "/j/Mods/B/b.reds".into(), texto: "/j/Mods/B/b.reds:1: x".into() }])),
            Resultado::ErroNasFontes(vec![linha_do_teto(), linha_de_erro_fora_dos_mods()]),
        ];
        for r in &resultados {
            let linhas = linhas_do_relatorio(r);
            assert!(!linhas.is_empty());
            for l in &linhas {
                assert!(l.contains(" / "), "linha sem as duas línguas: {l}");
            }
        }
        let linhas = |i: usize| linhas_do_relatorio(&resultados[i]);
        assert_eq!(linhas(0)[0], "compilados / built: 2 .reds da pasta Mods / 2 .reds from the Mods folder");
        assert_eq!(linhas(0)[2], "native sem registro / unregistered native: /j/Mods/ModWindows/a.reds:3 NaoRegistrada");
        // O jogo só "abre com os da última compilação" quando o blob em uso não traz o mod barrado.
        assert!(linhas(1)[0].contains("O jogo abre com os da última compilação") && !linhas(1)[0].contains("até o mod sair"));
        assert!(linhas(2)[0].contains("o jogo fecha ao ligar os scripts até o mod sair de r6/scripts"), "{}", linhas(2)[0]);
        assert!(linhas(2)[0].contains("until the mod leaves r6/scripts") && !linhas(2)[0].contains("O jogo abre"));
        assert!(linhas(3)[0].contains("O jogo abre") && linhas(4)[0].contains("até uma compilação dar certo"));
        assert!(linhas(5)[0].contains(": motivo.") && linhas(5)[0].contains(": reason.") && linhas(5)[0].contains("O jogo abre"));
        assert!(linhas(6)[0].contains("o jogo fecha") && !linhas(6)[0].contains("O jogo abre"));
        assert!(linhas_do_relatorio(&Resultado::Adotou).is_empty());
    }

    /// Um `final.redscripts` mínimo no formato que o `blob.rs` lê: o pool de nomes e uma definição de função por nome, `native` ou não.
    fn blob_minimo(funcoes: &[(&str, bool)]) -> Vec<u8> {
        let (mut dados, mut deslocamentos) = (Vec::new(), Vec::new());
        for (nome, _) in funcoes {
            deslocamentos.push(dados.len() as u32);
            dados.extend_from_slice(nome.as_bytes());
            dados.push(0);
        }
        let n = funcoes.len() as u32;
        let base = 104u32;
        let t_nomes = base + dados.len() as u32;
        let t_defs = t_nomes + 4 * n;
        let registros = t_defs + 20 * n;
        let mut b = vec![0u8; 104];
        let pos = |b: &mut Vec<u8>, p: usize, v: u32| b[p..p + 4].copy_from_slice(&v.to_le_bytes());
        pos(&mut b, 0, crate::scripts::blob::MAGIC);
        pos(&mut b, 4, crate::scripts::blob::VERSAO);
        for (tabela, offset, contagem) in [(32, base, dados.len() as u32), (44, t_nomes, n), (80, t_defs, n)] {
            pos(&mut b, tabela, offset);
            pos(&mut b, tabela + 4, contagem);
        }
        b.extend_from_slice(&dados);
        for d in deslocamentos {
            b.extend_from_slice(&d.to_le_bytes());
        }
        for i in 0..n {
            for v in [i, 0, registros + 5 * i, 5] {
                b.extend_from_slice(&v.to_le_bytes());
            }
            b.extend_from_slice(&[crate::scripts::blob::TIPO_FUNCAO, 0, 0, 0]);
        }
        for (_, nativa) in funcoes {
            b.push(0);
            b.extend_from_slice(&(if *nativa { 1u32 << 4 } else { 0 }).to_le_bytes());
        }
        b
    }

    #[test]
    fn blob_em_uso_olha_as_natives_que_o_blob_declara() {
        let pasta = pasta_de_teste("blob-em-uso");
        let blob = pasta.join(TRIO[0]);
        let barrado = Barrado { raiz: pasta.join("ModW"), natives: vec!["w.reds:1 NaoRegistrada".into()], nomes: vec!["NaoRegistrada".into()] };
        let com = |funcoes: &[(&str, bool)]| {
            std::fs::write(&blob, blob_minimo(funcoes)).unwrap();
            blob_em_uso(&blob, [&barrado])
        };
        assert_eq!(com(&[("Outra;", true), ("NaoRegistrada;", true)]), BlobEmUso::Fecha);
        assert_eq!(com(&[("ModW.NaoRegistrada", true)]), BlobEmUso::Fecha, "native dentro de module tem o nome qualificado no blob");
        assert_eq!(com(&[("NaoRegistrada;Int32", false)]), BlobEmUso::Abre, "função de script com o mesmo nome não vai ao registro");
        assert_eq!(com(&[("NaoRegistradaDeOutroJeito;", true), ("Outra;", true)]), BlobEmUso::Abre);
        std::fs::write(&blob, "truncado").unwrap();
        assert_eq!(blob_em_uso(&blob, [&barrado]), BlobEmUso::Fecha, "blob que não abre também não liga");
        // Sem native barrada o blob nem é lido.
        assert_eq!(blob_em_uso(&pasta.join("nao-existe"), &[] as &[Barrado]), BlobEmUso::Abre);
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn trio_volta_com_conteudo_e_mtime_e_a_peca_nova_sai() {
        let cache = pasta_de_teste("trio");
        std::fs::write(cache.join(TRIO[0]), "compilado-antigo").unwrap();
        std::fs::write(cache.join(TRIO[2]), "carimbo").unwrap();
        let mtime = mtime_ns(&cache.join(TRIO[0]));
        let existia = guardar_trio(&cache).unwrap();
        assert_eq!(existia, [true, false, true]);
        pausa();
        // O que um `scc` morto no meio deixa: blob truncado e um `.bk` que não existia.
        std::fs::write(cache.join(TRIO[0]), "trunc").unwrap();
        std::fs::write(cache.join(TRIO[1]), "bk-novo").unwrap();
        devolver_trio(&cache, existia);
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[0])).unwrap(), "compilado-antigo");
        assert_eq!(mtime_ns(&cache.join(TRIO[0])), mtime, "o mtime volta junto, e o `.ts` continua casando com o blob");
        assert!(!cache.join(TRIO[1]).exists());
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[2])).unwrap(), "carimbo");
        assert!(TRIO.iter().all(|n| !guarda_de(&cache, n).exists()));
        guardar_trio(&cache).unwrap();
        soltar_trio(&cache);
        assert!(TRIO.iter().all(|n| !guarda_de(&cache, n).exists()));
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[0])).unwrap(), "compilado-antigo");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn carimbo_alheio_sai_so_de_blob_compilado_com_bk_no_lugar() {
        let cache = pasta_de_teste("alheio");
        let carimbo = |c: u128| std::fs::write(cache.join(TRIO[2]), c.to_le_bytes()).unwrap();
        std::fs::write(cache.join(TRIO[0]), b"...pool...$REDSCRIPT_SIGNATURE...").unwrap();
        std::fs::write(cache.join(TRIO[1]), "vanilla").unwrap();
        let mtime = mtime_ns(&cache.join(TRIO[0]));
        carimbo(mtime.unwrap() as u128);
        tirar_carimbo_alheio(&cache, mtime);
        assert!(cache.join(TRIO[2]).exists(), "carimbo do próprio blob fica");
        carimbo(1);
        tirar_carimbo_alheio(&cache, mtime);
        assert!(!cache.join(TRIO[2]).exists(), "blob compilado copiado por fora: o scc compila do .bk");
        carimbo(1);
        std::fs::write(cache.join(TRIO[0]), "vanilla-novo").unwrap();
        tirar_carimbo_alheio(&cache, mtime_ns(&cache.join(TRIO[0])));
        assert!(cache.join(TRIO[2]).exists(), "vanilla novo fica com o scc, que renova o .bk");
        std::fs::write(cache.join(TRIO[0]), "$REDSCRIPT_SIGNATURE").unwrap();
        std::fs::remove_file(cache.join(TRIO[1])).unwrap();
        tirar_carimbo_alheio(&cache, mtime_ns(&cache.join(TRIO[0])));
        assert!(cache.join(TRIO[2]).exists(), "sem .bk não há de onde compilar");
        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn teto_para_o_processo_e_a_espera_volta() {
        let pasta = pasta_de_teste("teto");
        let saida = pasta.join("saida.txt");
        let t0 = Instant::now();
        let r = rodar_com_teto(Command::new("/bin/sleep").arg("30"), &saida, Duration::from_millis(100));
        assert!(r.unwrap_err().pt.contains("foi parado"));
        assert!(t0.elapsed() < Duration::from_secs(5));
        assert_eq!(rodar_com_teto(&mut Command::new("/usr/bin/true"), &saida, Duration::from_secs(5)), Ok(true));
        assert_eq!(rodar_com_teto(&mut Command::new("/usr/bin/false"), &saida, Duration::from_secs(5)), Ok(false));
        assert!(rodar_com_teto(&mut Command::new(pasta.join("nao-existe")), &saida, Duration::from_secs(5)).unwrap_err().pt.contains("não abriu"));
        assert_eq!(
            rodar_com_teto(Command::new("/bin/sh").args(["-c", "echo fora; echo erro >&2"]), &saida, Duration::from_secs(5)),
            Ok(true)
        );
        assert_eq!(std::fs::read_to_string(&saida).unwrap(), "fora\nerro\n");
        let _ = std::fs::remove_dir_all(&pasta);
    }

    #[test]
    fn compila_quando_muda_e_nao_paga_nada_quando_nao_muda() {
        let raiz = pasta_de_teste("ciclo");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let mods = escrever(&jogo, "Mods/Loja/r6/scripts/loja.reds", "func L() {}");
        let scc = scc_falso(
            &raiz,
            "scc-bom",
            "printf compilado > \"$r6/cache/final.redscripts\"\n[ \"$3\" = -compilePathsFile ] && cp \"$4\" \"$r6/../lista-recebida\"\nexit 0",
        );
        // A linha de sucesso existe porque o relatório de cada mod da `Mods/` com `.reds` aponta a seção Scripts.
        let compilados = vec![linha_dos_compilados(1)];
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(compilados.clone()));
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "compilado");
        assert_eq!(std::fs::read_to_string(jogo.join("lista-recebida")).unwrap(), mods.to_str().unwrap());
        assert!(!red4.join(SAIDA_DO_SCC).exists());
        assert!(TRIO.iter().all(|n| !guarda_de(&jogo.join("r6/cache"), n).exists()));
        let gravada = std::fs::read_to_string(red4.join(ASSINATURA)).unwrap();
        assert!(gravada.contains(&linha_do_blob(&jogo.join("r6/cache/final.redscripts"))));
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::NadaMudou(compilados.clone()));
        assert_eq!(execucoes(&raiz), 1);
        pausa();
        escrever(&jogo, "r6/scripts/mod/a.reds", "func A() { let x = 1; }");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(compilados));
        assert_eq!(execucoes(&raiz), 2);
        // O mod sai da `Mods/`: o blob é o da nossa compilação, então compila de novo (sem ele), e a lista do passe anterior sai.
        std::fs::remove_dir_all(jogo.join("Mods")).unwrap();
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![]));
        assert_eq!(execucoes(&raiz), 3);
        assert!(!red4.join("bwms-scripts-caminhos.txt").exists());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn erro_nas_fontes_devolve_o_blob_anterior_e_nao_roda_o_scc_de_novo_sem_mudanca() {
        let raiz = pasta_de_teste("erro");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let bom = scc_falso(&raiz, "scc-bom", SCC_BOM);
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Compilou(vec![]));
        let gravada = std::fs::read_to_string(red4.join(ASSINATURA)).unwrap();
        let blob = jogo.join("r6/cache/final.redscripts");
        let mtime = mtime_ns(&blob);
        pausa();
        escrever(&jogo, "r6/scripts/mod/a.reds", "func A( {");
        // Morre no meio: blob truncado, `.bk` novo e o erro no formato do `scc`.
        let quebrado = scc_falso(
            &raiz,
            "scc-quebrado",
            "printf tr > \"$r6/cache/final.redscripts\"\nprintf bk > \"$r6/cache/final.redscripts.bk\"\n\
             echo '[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] At /j/r6/scripts/mod/a.reds:1:9:'\necho 'func A( {'\necho '        ^'\n\
             echo 'syntax error, expected one of: identifier'\nexit 1",
        );
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&quebrado), "") else { panic!("esperava erro nas fontes") };
        // O erro é de `r6/scripts`: nenhum mod sai, e o relatório diz por quê.
        assert_eq!(linhas[1], linha_de_erro_fora_dos_mods());
        assert_eq!(linhas[2], "erro / error: /j/r6/scripts/mod/a.reds:1: syntax error, expected one of: identifier");
        assert_eq!(linhas.len(), 3);
        assert_eq!(std::fs::read_to_string(&blob).unwrap(), "compilado");
        assert_eq!(mtime_ns(&blob), mtime);
        assert!(!jogo.join("r6/cache/final.redscripts.bk").exists());
        assert_eq!(std::fs::read_to_string(red4.join(ASSINATURA)).unwrap(), gravada, "falha não grava assinatura");
        assert_eq!(execucoes(&raiz), 2);
        // Sem mudança, a abertura repete as linhas gravadas sem rodar o `scc`, mesmo sem ele.
        assert_eq!(executar(&jogo, &red4, Some(&quebrado), ""), Resultado::ErroNasFontes(linhas.clone()));
        assert_eq!(executar(&jogo, &red4, None, ""), Resultado::ErroNasFontes(linhas));
        assert_eq!(execucoes(&raiz), 2);
        pausa();
        escrever(&jogo, "r6/scripts/mod/a.reds", "func A() {}");
        assert!(matches!(executar(&jogo, &red4, None, ""), Resultado::NaoRodou(m, _) if m.pt.contains("rode o instalador")));
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Compilou(vec![]));
        assert!(!red4.join(FALHA).exists(), "a compilação que deu certo apaga a falha");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn erro_sem_posicao_nao_vira_erro_nas_fontes_e_tenta_de_novo() {
        let raiz = pasta_de_teste("sem-posicao");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let sem_lib = scc_falso(&raiz, "scc-sem-lib", "echo 'Could not load the scc shared library'\nexit 1");
        let r = executar(&jogo, &red4, Some(&sem_lib), "");
        assert!(matches!(&r, Resultado::NaoRodou(m, _) if m.pt.contains("incompleto") && m.pt.contains("rode o instalador")), "{r:?}");
        assert!(matches!(executar(&jogo, &red4, Some(&sem_lib), ""), Resultado::NaoRodou(..)));
        assert_eq!(execucoes(&raiz), 2);
        assert!(!red4.join(FALHA).exists());
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "vanilla");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn fonte_que_o_scc_nao_carrega_aponta_o_arquivo_e_nao_roda_o_scc_de_novo() {
        let raiz = pasta_de_teste("nao-carrega");
        let (jogo, red4) = jogo_de_teste(&raiz);
        // `café` em Latin-1: o 0xE9 sozinho não é UTF-8.
        let latin1 = jogo.join("r6/scripts/mod/cafe.reds");
        std::fs::write(&latin1, b"func Caf\xe9() {}").unwrap();
        // A saída medida do `scc` do pacote com esse arquivo: rc=1 e uma linha só.
        let nao_carrega =
            scc_falso(&raiz, "scc-nao-carrega", "echo '[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] Could not load script sources'\nexit 1");
        let fora_do_utf8 = |p: &Path| {
            format!(
                "erro / error: {}: não está em UTF-8; salve o arquivo em UTF-8 ou tire o mod da pasta / is not UTF-8; save it as UTF-8 or remove \
                 the mod from the folder",
                p.display()
            )
        };
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&nao_carrega), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].starts_with("Os scripts não compilaram"), "{}", linhas[0]);
        assert_eq!(linhas[1], linha_de_erro_fora_dos_mods(), "o arquivo é de r6/scripts: nenhum mod sai");
        assert_eq!(linhas[2..], [fora_do_utf8(&latin1)]);
        assert!(red4.join(FALHA).exists());
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "vanilla");
        // Sem mudança, a falha volta ao relatório sem rodar o `scc`.
        assert_eq!(executar(&jogo, &red4, Some(&nao_carrega), ""), Resultado::ErroNasFontes(linhas));
        assert_eq!(execucoes(&raiz), 1);
        // O arquivo salvo em UTF-8, e outro em Latin-1 na `Mods/`: o extra entra na busca, o mod dele sai e o outro mod entra.
        pausa();
        std::fs::write(&latin1, "func Café() {}").unwrap();
        let da_mods = escrever(&jogo, "Mods/M/m.reds", "");
        std::fs::write(&da_mods, b"func M\xe9() {}").unwrap();
        escrever(&jogo, "Mods/N/n.reds", "func N() {}");
        let le_utf8 = scc_falso(&raiz, "scc-le-utf8", SCC_QUE_LE_UTF8);
        let Resultado::Compilou(linhas) = executar(&jogo, &red4, Some(&le_utf8), "") else { panic!("esperava compilação sem o mod") };
        assert_eq!(linhas[0], linha_dos_compilados(1));
        assert!(linhas[1].starts_with(&format!("vermelho / red: {} — não compilou e ficou fora", jogo.join("Mods/M").display())), "{}", linhas[1]);
        assert_eq!(linhas[2], fora_do_utf8(&da_mods));
        assert_eq!(linhas.len(), 3);
        assert_eq!(execucoes(&raiz), 3, "uma passada com o mod e outra sem");
        assert!(!red4.join(FALHA).exists());
        assert_eq!(estados_em_vigor(&red4, &Resultado::Compilou(vec![])).cor(&jogo.join("Mods/M")), Some(CorDoMod::Vermelho));
        // Pasta com nome `.reds` em `r6/scripts`, vazia: só a pasta muda a assinatura.
        std::fs::remove_dir_all(jogo.join("Mods")).unwrap();
        let pasta = jogo.join("r6/scripts/x.reds");
        std::fs::create_dir_all(&pasta).unwrap();
        let f = fontes(&jogo.join("r6/scripts"), &[]);
        assert_eq!((f.pastas_reds.as_slice(), f.scripts.len()), (std::slice::from_ref(&pasta), 2));
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&nao_carrega), "") else { panic!("esperava erro nas fontes") };
        assert_eq!(
            linhas[2],
            format!(
                "erro / error: {}: é uma pasta com nome terminado em .reds; renomeie a pasta ou tire o mod de r6/scripts / is a folder whose \
                 name ends in .reds; rename the folder or remove the mod from r6/scripts",
                pasta.display()
            )
        );
        assert_eq!(linhas.len(), 3);
        assert_eq!(executar(&jogo, &red4, Some(&nao_carrega), ""), Resultado::ErroNasFontes(linhas));
        assert_eq!(execucoes(&raiz), 4);
        // Renomear a pasta conserta, e a abertura seguinte compila em vez de repetir a falha.
        std::fs::rename(&pasta, jogo.join("r6/scripts/x")).unwrap();
        let bom = scc_falso(&raiz, "scc-bom", SCC_BOM);
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Compilou(vec![]));
        assert!(!red4.join(FALHA).exists());
        // Sem culpado nas fontes, a linha do `scc` vai como motivo, nada fica gravado e a abertura seguinte tenta de novo.
        pausa();
        escrever(&jogo, "r6/scripts/mod/a.reds", "func A() { let x = 1; }");
        let r = executar(&jogo, &red4, Some(&nao_carrega), "");
        assert!(matches!(&r, Resultado::NaoRodou(m, _) if m.pt.contains(FONTES_NAO_CARREGADAS)), "{r:?}");
        assert!(!red4.join(FALHA).exists());
        assert!(matches!(executar(&jogo, &red4, Some(&nao_carrega), ""), Resultado::NaoRodou(..)));
        assert_eq!(execucoes(&raiz), 7);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// O `scc` de mentira que recusa, como o de verdade, a fonte fora do UTF-8 de `r6/scripts` ou da lista (`scc/lib/src/lib.rs`, "Could not
    /// load script sources").
    const SCC_QUE_LE_UTF8: &str = "{ find \"$2\" -name '*.reds' -type f; [ \"$3\" = -compilePathsFile ] && cat \"$4\" && echo; } | \
         while IFS= read -r f; do [ -z \"$f\" ] && continue; iconv -f UTF-8 -t UTF-8 \"$f\" >/dev/null 2>&1 || \
         { echo '[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] Could not load script sources'; exit 1; }; done || exit 1\n\
         printf compilado > \"$r6/cache/final.redscripts\"\nexit 0";

    /// O `scc` de mentira que recusa o `.bk` com a marca, como o de verdade (`scc/lib/src/lib.rs`, "backup has been corrupted").
    const SCC_QUE_CONFERE_O_BK: &str = "if grep -q 'REDSCRIPT_SIGNATURE' \"$r6/cache/final.redscripts.bk\" 2>/dev/null; then\n\
         echo \"[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] The REDScript backup has been corrupted, try removing everything in the 'r6/cache' directory and verify game files with Steam/GOG\"\n\
         exit 1\nfi\nprintf 'compilado $REDSCRIPT_SIGNATURE' > \"$r6/cache/final.redscripts\"\nexit 0";

    #[test]
    fn backup_estragado_volta_da_copia_do_instalador_e_compila_na_segunda_passada() {
        let raiz = pasta_de_teste("backup");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let cache = jogo.join("r6/cache");
        std::fs::write(cache.join(TRIO[0]), "compilado $REDSCRIPT_SIGNATURE").unwrap();
        std::fs::write(cache.join(TRIO[1]), "compilado tomado por vanilla $REDSCRIPT_SIGNATURE").unwrap();
        std::fs::write(cache.join(TRIO[2]), (mtime_ns(&cache.join(TRIO[0])).unwrap() as u128).to_le_bytes()).unwrap();
        let scc = scc_falso(&raiz, "scc", SCC_QUE_CONFERE_O_BK);
        // Um `.reds` na `Mods/`: o blob de fora não o cobre, e a abertura compila.
        escrever(&jogo, "Mods/m/m.reds", "func M() {}");
        // Sem a cópia do instalador não há de onde repor: a ação é verificar os arquivos e rodar o instalador.
        let r = executar(&jogo, &red4, Some(&scc), "");
        assert!(matches!(&r, Resultado::NaoRodou(m, _) if m.pt.contains("verifique os arquivos do jogo") && m.pt.contains("rode o instalador")), "{r:?}");
        assert_eq!(execucoes(&raiz), 1);
        // A cópia compilada não serve.
        std::fs::write(cache.join(ORIGINAL_DO_INSTALADOR), "outro compilado $REDSCRIPT_SIGNATURE").unwrap();
        assert!(matches!(executar(&jogo, &red4, Some(&scc), ""), Resultado::NaoRodou(..)));
        assert_eq!(execucoes(&raiz), 2);
        std::fs::write(cache.join(ORIGINAL_DO_INSTALADOR), "vanilla-original").unwrap();
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![linha_dos_compilados(1)]));
        assert_eq!(execucoes(&raiz), 4, "uma passada recusada e uma depois do conserto");
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[1])).unwrap(), "vanilla-original");
        assert!(TRIO.iter().all(|n| !guarda_de(&cache, n).exists()));
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn mod_da_mods_com_native_sem_registro_fica_fora_ate_o_plugin_chegar() {
        let raiz = pasta_de_teste("native-mods");
        let (jogo, red4) = jogo_de_teste(&raiz);
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        let windows = escrever(&jogo, "Mods/ModWindows/r6/scripts/w.reds", "native func NaoRegistrada()\n");
        escrever(&jogo, "Mods/ModWindows/r6/scripts/ui.reds", "func Ui() {}");
        let bom = escrever(&jogo, "Mods/Bom/b.reds", "native func BwmsRegistrada()\n");
        let scc = scc_falso(
            &raiz,
            "scc-bom",
            "printf compilado > \"$r6/cache/final.redscripts\"\n[ \"$3\" = -compilePathsFile ] && cp \"$4\" \"$r6/../lista-recebida\"\nexit 0",
        );
        let linhas = vec![
            linha_dos_compilados(1),
            format!(
                "fora / left out: {} — o mod precisa de um plugin de Windows que o BWMS não tem; tire-o da pasta / the mod needs a Windows \
                 plugin that BWMS does not have; remove it from the folder",
                jogo.join("Mods/ModWindows").display()
            ),
            format!("native sem registro / unregistered native: {}:1 NaoRegistrada", windows.display()),
        ];
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(linhas.clone()));
        assert_eq!(std::fs::read_to_string(jogo.join("lista-recebida")).unwrap(), bom.to_str().unwrap());
        // Sem mudança, as linhas voltam ao relatório, que é regravado a cada abertura.
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::NadaMudou(linhas));
        assert_eq!(execucoes(&raiz), 1);
        // O plugin que registra a native entra em `red4ext/plugins/`: a assinatura muda e o mod entra.
        escrever(&red4, "plugins/plugin.dylib", "\0NaoRegistrada\0");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![linha_dos_compilados(3)]));
        assert_eq!(std::fs::read_to_string(jogo.join("lista-recebida")).unwrap().lines().count(), 3);
        assert_eq!(execucoes(&raiz), 2);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn mod_de_r6_scripts_com_native_sem_registro_impede_a_compilacao() {
        let raiz = pasta_de_teste("native-r6");
        let (jogo, red4) = jogo_de_teste(&raiz);
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        // A pasta do BWMS fica com a conferência do instalador.
        escrever(&jogo, "r6/scripts/blackwall-mods/b.reds", "native func BwmsSoDoDev()\n");
        let windows = escrever(&jogo, "r6/scripts/ModWindows/w.reds", "module ModWindows\nnative func NaoRegistrada()\n");
        let scc = scc_falso(&raiz, "scc-bom", SCC_BOM);
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].starts_with("Os scripts não foram compilados: um mod de r6/scripts"));
        // O blob deste jogo de teste não é um `final.redscripts`: o motor não o ligaria, e a linha não diz que o jogo abre.
        assert!(linhas[0].contains("até o mod sair de r6/scripts"), "{}", linhas[0]);
        assert!(linhas[1].contains(&jogo.join("r6/scripts/ModWindows").display().to_string()));
        assert_eq!(linhas[2], format!("native sem registro / unregistered native: {}:2 NaoRegistrada", windows.display()));
        assert_eq!(linhas.len(), 3);
        assert_eq!(execucoes(&raiz), 0);
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "vanilla");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::ErroNasFontes(linhas));
        std::fs::remove_dir_all(jogo.join("r6/scripts/ModWindows")).unwrap();
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![]));
        assert_eq!(execucoes(&raiz), 1);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// Sem a pasta `Mods/` (mod instalado direto em `r6/scripts`), a falha cria a pasta, e o relatório dela sai com a seção Scripts; a
    /// compilação que dá certo não cria nada.
    #[test]
    fn falha_sem_a_pasta_mods_cria_a_pasta_e_o_relatorio_traz_a_secao_scripts() {
        let raiz = pasta_de_teste("falha-sem-mods");
        let (jogo, red4) = jogo_de_teste(&raiz);
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        escrever(&jogo, "r6/scripts/ModWindows/w.reds", "module ModWindows\nnative func NaoRegistrada()\n");
        let scc = scc_falso(&raiz, "scc-bom", SCC_BOM);
        let mods = crate::mods_do_usuario::pasta_mods::pasta(&jogo);
        let falha = executar(&jogo, &red4, Some(&scc), "");
        assert!(matches!(falha, Resultado::ErroNasFontes(_)), "{falha:?}");
        assert!(!mods.exists());
        pasta_do_relatorio_para_a_falha(&jogo, &falha);
        assert!(mods.is_dir());
        let texto = crate::mods_do_usuario::pasta_mods::texto_do_boot(&jogo, &[("Scripts".into(), linhas_do_relatorio(&falha))])
            .expect("com a pasta, o relatório sai");
        assert!(texto.contains("\n[Scripts]\nOs scripts não foram compilados: um mod de r6/scripts"), "{texto}");

        std::fs::remove_dir_all(&mods).unwrap();
        std::fs::remove_dir_all(jogo.join("r6/scripts/ModWindows")).unwrap();
        let compilou = executar(&jogo, &red4, Some(&scc), "");
        assert_eq!(compilou, Resultado::Compilou(vec![]));
        pasta_do_relatorio_para_a_falha(&jogo, &compilou);
        assert!(!mods.exists());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn guarda_largada_por_compilacao_interrompida_volta_antes_de_decidir() {
        let raiz = pasta_de_teste("largada");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let cache = jogo.join("r6/cache");
        let bom = scc_falso(&raiz, "scc-bom", SCC_BOM);
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Compilou(vec![]));
        carimbar(&cache);
        // Processo morto durante o `scc`: as guardas ficaram, o blob ficou truncado e o `.ts` ainda é o do blob anterior.
        guardar_trio(&cache).unwrap();
        pausa();
        std::fs::write(cache.join(TRIO[0]), "tr").unwrap();
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::NadaMudou(vec![]));
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[0])).unwrap(), "compilado");
        assert!(TRIO.iter().all(|n| !guarda_de(&cache, n).exists()));
        assert!(!recuperar_trio_largado(&cache));
        assert_eq!(execucoes(&raiz), 1);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    /// O `.ts` que o `scc` grava depois de salvar o blob inteiro.
    fn carimbar(cache: &Path) {
        std::fs::write(cache.join(TRIO[2]), (mtime_ns(&cache.join(TRIO[0])).unwrap() as u128).to_le_bytes()).unwrap();
    }

    #[test]
    fn guarda_largada_com_o_trio_inteiro_so_sai_e_o_blob_novo_fica() {
        let raiz = pasta_de_teste("largada-inteira");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let cache = jogo.join("r6/cache");
        let bom = scc_falso(&raiz, "scc-bom", SCC_BOM);
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Compilou(vec![]));
        carimbar(&cache);
        // A reinstalação: o teste de abertura do instalador morre com o `scc` no meio e larga as guardas; depois o passo 6 compila o
        // blob novo, inteiro e carimbado.
        guardar_trio(&cache).unwrap();
        pausa();
        std::fs::write(cache.join(TRIO[0]), "compilado-pelo-passo-6").unwrap();
        carimbar(&cache);
        // A 1ª abertura fica com o blob novo e o adota, sem compilar.
        assert_eq!(executar(&jogo, &red4, Some(&bom), ""), Resultado::Adotou);
        assert_eq!(std::fs::read_to_string(cache.join(TRIO[0])).unwrap(), "compilado-pelo-passo-6");
        assert!(TRIO.iter().all(|n| !guarda_de(&cache, n).exists()));
        assert_eq!(execucoes(&raiz), 1);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn adocao_com_mod_de_r6_scripts_barrado_diz_se_o_jogo_fecha() {
        let raiz = pasta_de_teste("adota-barrado");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let cache = jogo.join("r6/cache");
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        escrever(&jogo, "r6/scripts/ModWindows/w.reds", "native func NaoRegistrada()\n");
        pausa();
        // O passo 6 compilou `r6/scripts` inteiro: o blob dele declara a native do mod.
        std::fs::write(cache.join(TRIO[0]), blob_minimo(&[("A;", false), ("NaoRegistrada;", true)])).unwrap();
        carimbar(&cache);
        let scc = scc_falso(&raiz, "scc-ok", SCC_BOM);
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].contains("o jogo fecha ao ligar os scripts até o mod sair de r6/scripts"), "{}", linhas[0]);
        assert!(!linhas[0].contains("O jogo abre"));
        assert_eq!(execucoes(&raiz), 0);
        // Um blob compilado antes de o mod entrar não o declara: o jogo abre com ele.
        std::fs::write(cache.join(TRIO[0]), blob_minimo(&[("A;", false)])).unwrap();
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].contains("O jogo abre com os da última compilação que deu certo"), "{}", linhas[0]);
        assert_eq!(execucoes(&raiz), 0);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn plugin_que_sai_deixa_no_blob_o_mod_de_r6_scripts_e_a_linha_diz_que_o_jogo_fecha() {
        let raiz = pasta_de_teste("plugin-saiu");
        let (jogo, red4) = jogo_de_teste(&raiz);
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        escrever(&red4, "plugins/plugin.dylib", "\0NaoRegistrada\0");
        escrever(&jogo, "r6/scripts/ModWindows/w.reds", "native func NaoRegistrada()\n");
        let pronto = raiz.join("blob-com-o-mod");
        std::fs::write(&pronto, blob_minimo(&[("A;", false), ("NaoRegistrada;", true)])).unwrap();
        let scc = scc_falso(&raiz, "scc-copia", &format!("cp \"{}\" \"$r6/cache/final.redscripts\"\nexit 0", pronto.display()));
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![]));
        // O blob em vigor é o da nossa compilação, que levou o mod quando o plugin estava lá.
        std::fs::remove_file(red4.join("plugins/plugin.dylib")).unwrap();
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].contains("o jogo fecha ao ligar os scripts até o mod sair de r6/scripts"), "{}", linhas[0]);
        assert_eq!(execucoes(&raiz), 1);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn falha_com_mod_da_mods_barrado_no_blob_em_uso_diz_que_o_jogo_fecha() {
        let raiz = pasta_de_teste("mods-no-blob");
        let (jogo, red4) = jogo_de_teste(&raiz);
        escrever(&red4, "libcp77_console.dylib", "...BwmsRegistrada...");
        escrever(&red4, "plugins/plugin.dylib", "\0NaoRegistrada\0");
        escrever(&jogo, "Mods/ModWindows/w.reds", "native func NaoRegistrada()\n");
        let pronto = raiz.join("blob-com-o-mod");
        std::fs::write(&pronto, blob_minimo(&[("NaoRegistrada;", true)])).unwrap();
        let copia = scc_falso(&raiz, "scc-copia", &format!("cp \"{}\" \"$r6/cache/final.redscripts\"\nexit 0", pronto.display()));
        assert_eq!(executar(&jogo, &red4, Some(&copia), ""), Resultado::Compilou(vec![linha_dos_compilados(1)]));
        // O plugin sai e, na mesma abertura, outro mod quebra a compilação: o blob em uso ainda traz o mod que ficou fora.
        std::fs::remove_file(red4.join("plugins/plugin.dylib")).unwrap();
        escrever(&jogo, "Mods/Quebrado/q.reds", "func Q( {");
        let quebrado = scc_falso(
            &raiz,
            "scc-quebrado",
            "echo '[ERROR - Sat, 17 Oct 2026 23:29:42 -0300] At /j/Mods/Quebrado/q.reds:1:8:'\necho 'func Q( {'\necho '       ^'\n\
             echo 'syntax error, expected one of: identifier'\nexit 1",
        );
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&quebrado), "") else { panic!("esperava erro nas fontes") };
        assert!(linhas[0].contains("o jogo fecha ao ligar os scripts até uma compilação dar certo"), "{}", linhas[0]);
        // Sem o `scc`, o motivo diz o mesmo do blob em uso.
        escrever(&jogo, "Mods/Quebrado/q.reds", "func Q() {}");
        let r = executar(&jogo, &red4, None, "");
        assert!(matches!(&r, Resultado::NaoRodou(_, BlobEmUso::Fecha)), "{r:?}");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn primeira_abertura_depois_do_instalador_adota_o_blob_dele_sem_compilar() {
        let raiz = pasta_de_teste("adota");
        let (jogo, red4) = jogo_de_teste(&raiz);
        pausa();
        // O que o passo 6 do instalador deixa: blob compilado depois dos `.reds` e o carimbo do `scc` com o mtime dele.
        let blob = escrever(&jogo, "r6/cache/final.redscripts", "compilado-pelo-instalador");
        std::fs::write(jogo.join("r6/cache/final.redscripts.ts"), (mtime_ns(&blob).unwrap() as u128).to_le_bytes()).unwrap();
        let scc = scc_falso(&raiz, "scc-ok", "exit 0");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Adotou);
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::NadaMudou(vec![]));
        assert_eq!(execucoes(&raiz), 0);
        // Com um `.reds` na `Mods/`, o blob do instalador não o tem.
        escrever(&jogo, "Mods/m/m.reds", "func M() {}");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![linha_dos_compilados(1)]));
        assert_eq!(execucoes(&raiz), 1);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn mod_que_entra_em_r6_scripts_depois_do_instalador_compila() {
        let raiz = pasta_de_teste("depois");
        let (jogo, red4) = jogo_de_teste(&raiz);
        pausa();
        let blob = escrever(&jogo, "r6/cache/final.redscripts", "compilado-pelo-instalador");
        std::fs::write(jogo.join("r6/cache/final.redscripts.ts"), (mtime_ns(&blob).unwrap() as u128).to_le_bytes()).unwrap();
        pausa();
        escrever(&jogo, "r6/scripts/novo/n.reds", "func N() {}");
        let scc = scc_falso(&raiz, "scc-ok", "exit 0");
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::Compilou(vec![]));
        let _ = std::fs::remove_dir_all(&raiz);
    }


    /// O `scc` de mentira que checa como o de verdade (medido em `z030/relatorios/C.md`): erro de sintaxe para no 1º arquivo, em ordem de
    /// compilação (`r6/scripts`, depois a lista); sem sintaxe, acusa todo erro de tipo numa passada só. Marcas no texto do `.reds`:
    /// `SINTAXE`, `TIPO`, `DEFINE:<nome>`, `USA:<nome>` (erro se nenhuma fonte define o nome) e `AVISO` (aviso de conflito, não fatal).
    const SCC_QUE_CHECA_TIPOS: &str = r#"s="$2"; lista=""; [ "$3" = -compilePathsFile ] && lista="$4"
todos() { find "$s" -name '*.reds' -type f | sort; [ -n "$lista" ] && cat "$lista" && echo; }
f=$(todos | while IFS= read -r f; do [ -n "$f" ] && grep -l SINTAXE "$f"; done | head -1)
if [ -n "$f" ]; then printf '[ERROR - x] At %s:1:6:\nfunc ( {\n     ^\nsyntax error, expected one of: identifier\n' "$f"; exit 1; fi
saida=$(todos | while IFS= read -r f; do
  [ -z "$f" ] && continue
  grep -q TIPO "$f" && printf '[ERROR - x] [UNRESOLVED_FN] At %s:2:3:\nNaoExiste();\n^^^\nfunction NaoExiste not found\n' "$f"
  u=$(grep -o 'USA:[A-Za-z]*' "$f" | head -1)
  if [ -n "$u" ]; then
    n=${u#USA:}
    achou=$(todos | while IFS= read -r g; do [ -n "$g" ] && grep -l "DEFINE:$n" "$g"; done | head -1)
    [ -z "$achou" ] && printf '[ERROR - x] [UNRESOLVED_TYPE] At %s:3:5:\nlet x: %s;\n^\ntype %s not found\n' "$f" "$n" "$n"
  fi
  grep -q AVISO "$f" && printf '[WARN - x] At %s:1:1:\n@replaceMethod(PlayerPuppet)\n^^^\nthis method replacement overwrites a previous annotation targeting the same method, only one replacement per method can be active at a time\n' "$f"
done)
echo "$saida"
echo "$saida" | grep -q '^\[ERROR' && exit 1
printf compilado > "$r6/cache/final.redscripts"
[ -n "$lista" ] && cp "$lista" "$r6/../lista-recebida"
exit 0"#;

    #[test]
    fn mod_quebrado_sai_sozinho_os_outros_entram_e_sem_mudanca_nada_roda() {
        let raiz = pasta_de_teste("quebrado-sai");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let scc = scc_falso(&raiz, "scc-tipos", SCC_QUE_CHECA_TIPOS);
        let m = |n: &str| jogo.join("Mods").join(n);
        let a = escrever(&jogo, "Mods/A/a.reds", "DEFINE:Base TIPO");
        let b = escrever(&jogo, "Mods/B/r6/scripts/b.reds", "USA:Base");
        let c = escrever(&jogo, "Mods/C/c.reds", "SINTAXE");
        escrever(&jogo, "Mods/D/d.reds", "SINTAXE");
        let e = escrever(&jogo, "Mods/E/e.reds", "AVISO");
        let f = escrever(&jogo, "Mods/F/f.reds", "ok");
        let Resultado::Compilou(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava compilação sem os quebrados") };
        assert_eq!(execucoes(&raiz), 5, "C e D (sintaxe, um por passada), A (tipo), B (cascata sem A) e a que deu certo");
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "compilado");
        assert_eq!(std::fs::read_to_string(jogo.join("lista-recebida")).unwrap(), format!("{}\n{}", e.display(), f.display()));
        assert_eq!(linhas[0], linha_dos_compilados(2));
        let vermelhos: Vec<&str> = linhas.iter().filter_map(|l| l.strip_prefix("vermelho / red: ")).collect();
        assert_eq!(vermelhos.len(), 4);
        for (l, nome) in vermelhos.iter().zip(["C", "D", "A", "B"]) {
            assert!(l.starts_with(&format!("{} — não compilou e ficou fora; os outros mods e o BWMS entraram", m(nome).display())), "{l}");
        }
        assert!(linhas.contains(&format!("erro / error: {}:1: syntax error, expected one of: identifier", c.display())));
        assert!(linhas.contains(&format!("erro / error: {}:2: [UNRESOLVED_FN] function NaoExiste not found", a.display())));
        assert!(linhas.contains(&format!("erro / error: {}:3: [UNRESOLVED_TYPE] type Base not found", b.display())));
        // A cascata aponta só o mod que saiu na passada anterior, a que checou os tipos: C e D saíram por sintaxe, antes da checagem.
        let cascata: Vec<&String> = linhas.iter().filter(|l| l.starts_with("só falhou depois que saiu")).collect();
        assert_eq!(cascata.len(), 1);
        assert!(cascata[0].starts_with(&format!("só falhou depois que saiu {}: ele usa algo de lá", m("A").display())), "{}", cascata[0]);
        assert!(linhas.last().unwrap().starts_with(&format!("aviso / warning: {}:1: this method replacement", e.display())));
        let estados = estados_em_vigor(&red4, &Resultado::Compilou(vec![]));
        for nome in ["C", "D", "A", "B"] {
            assert_eq!(estados.cor(&m(nome)), Some(CorDoMod::Vermelho), "{nome}");
        }
        assert_eq!(estados.cor(&m("E")), Some(CorDoMod::Amarelo));
        assert_eq!(estados.cor(&m("F")), None, "entrou inteiro");
        let i_b = estados.raizes.iter().position(|r| *r == m("B")).unwrap();
        assert!(estados.motivos[i_b].contains("[UNRESOLVED_TYPE]") && estados.motivos[i_b].contains("só falhou depois que saiu"));
        // Sem mudança: as mesmas linhas e o mesmo estado, sem rodar o `scc` (os mods que saíram estão na assinatura).
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::NadaMudou(linhas.clone()));
        assert_eq!(estados_em_vigor(&red4, &Resultado::NadaMudou(vec![])), estados);
        assert_eq!(execucoes(&raiz), 5);
        // D consertado: ele entra; C, A e B saem de novo (C por sintaxe, A por tipo, B pela cascata).
        pausa();
        escrever(&jogo, "Mods/D/d.reds", "ok");
        let Resultado::Compilou(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava compilação") };
        assert_eq!(execucoes(&raiz), 9);
        assert_eq!(linhas[0], linha_dos_compilados(3));
        assert_eq!(estados_em_vigor(&red4, &Resultado::Compilou(vec![])).cor(&m("D")), None);
        // A consertado: B volta junto.
        pausa();
        escrever(&jogo, "Mods/A/a.reds", "DEFINE:Base");
        let Resultado::Compilou(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava compilação") };
        assert_eq!(execucoes(&raiz), 11);
        assert_eq!(linhas[0], linha_dos_compilados(5));
        let estados = estados_em_vigor(&red4, &Resultado::Compilou(vec![]));
        assert_eq!((estados.cor(&m("A")), estados.cor(&m("B")), estados.cor(&m("C"))), (None, None, Some(CorDoMod::Vermelho)));
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn no_teto_de_rodadas_fica_a_ultima_compilacao_e_a_falha_repete_sem_rodar() {
        let raiz = pasta_de_teste("teto-rodadas");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let scc = scc_falso(&raiz, "scc-tipos", SCC_QUE_CHECA_TIPOS);
        for i in 0..TETO_DE_RODADAS {
            escrever(&jogo, &format!("Mods/S{i}/s.reds"), "SINTAXE");
        }
        escrever(&jogo, "Mods/T/t.reds", "ok");
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert_eq!(execucoes(&raiz), TETO_DE_RODADAS);
        assert!(linhas[0].starts_with("Os scripts não compilaram. O jogo abre com os da última compilação"), "{}", linhas[0]);
        assert_eq!(linhas[1], linha_do_teto());
        assert_eq!(linhas.iter().filter(|l| l.starts_with("vermelho / red: ")).count(), TETO_DE_RODADAS);
        assert!(linhas.iter().all(|l| !l.contains("os outros mods e o BWMS entraram")), "nada entrou");
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "vanilla");
        let estados = estados_em_vigor(&red4, &Resultado::ErroNasFontes(vec![]));
        assert_eq!(estados.cores, vec![CorDoMod::Vermelho; TETO_DE_RODADAS]);
        assert_eq!(executar(&jogo, &red4, Some(&scc), ""), Resultado::ErroNasFontes(linhas));
        assert_eq!(execucoes(&raiz), TETO_DE_RODADAS, "a falha gravada repete sem rodar o scc");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn erro_fora_da_mods_nao_tira_mod_e_aponta_o_mod_da_mods_que_tambem_quebrou() {
        let raiz = pasta_de_teste("fora-da-mods");
        let (jogo, red4) = jogo_de_teste(&raiz);
        let scc = scc_falso(&raiz, "scc-tipos", SCC_QUE_CHECA_TIPOS);
        // Mod posto direto em `r6/scripts` (estilo Windows) com erro de tipo, e um da `Mods/` também.
        let em_r6 = escrever(&jogo, "r6/scripts/ModWin/w.reds", "TIPO");
        let q = escrever(&jogo, "Mods/Q/q.reds", "TIPO");
        let Resultado::ErroNasFontes(linhas) = executar(&jogo, &red4, Some(&scc), "") else { panic!("esperava erro nas fontes") };
        assert_eq!(execucoes(&raiz), 1, "erro sem dono não pede outra passada");
        assert_eq!(linhas[1], linha_de_erro_fora_dos_mods());
        assert_eq!(linhas[2], format!("erro / error: {}:2: [UNRESOLVED_FN] function NaoExiste not found", em_r6.display()));
        assert!(linhas.contains(&format!("vermelho / red: {} — não compilou / did not compile", jogo.join("Mods/Q").display())));
        assert!(linhas.contains(&format!("erro / error: {}:2: [UNRESOLVED_FN] function NaoExiste not found", q.display())));
        assert_eq!(std::fs::read_to_string(jogo.join("r6/cache/final.redscripts")).unwrap(), "vanilla");
        assert_eq!(estados_em_vigor(&red4, &Resultado::ErroNasFontes(vec![])).cor(&jogo.join("Mods/Q")), Some(CorDoMod::Vermelho));
        let _ = std::fs::remove_dir_all(&raiz);
    }


    /// Mod quebrado sai sozinho, com o `scc` real e `.reds` com erro de verdade, ao lado dos mods que estiverem na `Mods/` da base (a medida
    /// do relatório usa Equipment-EX e Virtual Atelier). Mesma base de `medida_com_o_scc_real` (`BWMS_MEDIDA_SCRIPTS`); imprime o tempo de cada
    /// abertura. `cargo test --lib mods_quebrados_saem_com_o_scc_real -- --ignored --nocapture --test-threads=1`.
    #[test]
    #[ignore]
    fn mods_quebrados_saem_com_o_scc_real() {
        let Some(base) = std::env::var_os("BWMS_MEDIDA_SCRIPTS").map(PathBuf::from) else {
            println!("[quebrados] sem BWMS_MEDIDA_SCRIPTS: pulado");
            return;
        };
        let (jogo, scc) = (base.join("jogo"), base.join("scc/scc"));
        let (red4, cache) = (jogo.join("red4ext"), jogo.join("r6/cache"));
        for nome in TRIO {
            let _ = std::fs::remove_file(cache.join(nome));
        }
        let _ = std::fs::remove_file(red4.join(ASSINATURA));
        let _ = std::fs::remove_file(red4.join(FALHA));
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::copy(base.join("vanilla.redscripts"), cache.join(TRIO[0])).unwrap();
        let passo = |nome: &str| {
            let t0 = Instant::now();
            let r = executar(&jogo, &red4, Some(&scc), "");
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let estados = estados_em_vigor(&red4, &r);
            let cor = |c| estados.cores.iter().filter(|&&x| x == c).count();
            let resumo = match &r {
                Resultado::NadaMudou(_) => "NadaMudou".to_string(),
                Resultado::Compilou(_) => "Compilou".to_string(),
                Resultado::ErroNasFontes(_) => "ErroNasFontes".to_string(),
                Resultado::Adotou => "Adotou".to_string(),
                Resultado::NaoRodou(m, _) => format!("NaoRodou: {}", m.pt),
            };
            println!(
                "[quebrados] {nome}: {resumo} em {ms:.0} ms; vermelhos {}, amarelos {}",
                cor(CorDoMod::Vermelho),
                cor(CorDoMod::Amarelo)
            );
            (r, estados)
        };
        let mods = jogo.join("Mods");
        let (r, _) = passo("0 quebrados, 1ª abertura");
        assert!(matches!(r, Resultado::Compilou(_)), "{r:?}");
        let (r, _) = passo("0 quebrados, sem mudança");
        assert!(matches!(r, Resultado::NadaMudou(_)));
        // 1 quebrado (tipo): sai, os outros entram.
        escrever(&mods, "zz-q1/q1.reds", "module Q1\n\npublic func Q1() -> Int32 {\n  return NaoExisteQ1();\n}\n");
        escrever(&mods, "zz-q1/bom.reds", "module Q1\n\npublic func BwmsMedidaBomDoQuebrado() -> Int32 {\n  return 1;\n}\n");
        let (r, e) = passo("1 quebrado (tipo)");
        assert!(matches!(&r, Resultado::Compilou(l) if l.iter().any(|l| l.contains("q1.reds:4: [UNRESOLVED_FN]"))), "{r:?}");
        assert_eq!(e.cor(&mods.join("zz-q1")), Some(CorDoMod::Vermelho));
        // O mod sai inteiro (o arquivo dele que compila também), e os outros mods da `Mods/` estão no blob.
        let blob = std::fs::read(cache.join(TRIO[0])).unwrap();
        assert!(!contem(&blob, b"BwmsMedidaBomDoQuebrado"));
        if mods.join("Virtual Atelier").is_dir() {
            assert!(contem(&blob, b"VirtualAtelierConfig"));
        }
        assert!(matches!(passo("1 quebrado, sem mudança").0, Resultado::NadaMudou(_)));
        // 2 quebrados (tipo): os dois na mesma passada.
        escrever(&mods, "zz-q2/q2.reds", "module Q2\n\npublic func Q2() -> Bool {\n  return NaoExisteQ2(1);\n}\n");
        let (r, e) = passo("2 quebrados (tipo)");
        assert!(matches!(r, Resultado::Compilou(_)), "{r:?}");
        assert_eq!((e.cor(&mods.join("zz-q1")), e.cor(&mods.join("zz-q2"))), (Some(CorDoMod::Vermelho), Some(CorDoMod::Vermelho)));
        assert!(matches!(passo("2 quebrados, sem mudança").0, Resultado::NadaMudou(_)));
        std::fs::remove_dir_all(mods.join("zz-q1")).unwrap();
        std::fs::remove_dir_all(mods.join("zz-q2")).unwrap();
        // 2 quebrados (sintaxe): o `scc` para no 1º; uma passada por mod.
        escrever(&mods, "zz-s1/s1.reds", "module S1\n\npublic func S1( -> Int32 {\n  return 1;\n}\n");
        escrever(&mods, "zz-s2/s2.reds", "func S2( {\n");
        let (r, e) = passo("2 quebrados (sintaxe)");
        assert!(matches!(r, Resultado::Compilou(_)), "{r:?}");
        assert_eq!((e.cor(&mods.join("zz-s1")), e.cor(&mods.join("zz-s2"))), (Some(CorDoMod::Vermelho), Some(CorDoMod::Vermelho)));
        std::fs::remove_dir_all(mods.join("zz-s1")).unwrap();
        std::fs::remove_dir_all(mods.join("zz-s2")).unwrap();
        // Cascata: A tem erro de tipo e define a classe que B usa; B só falha sem A.
        escrever(&mods, "zz-ca/a.reds", "module CascataA\n\npublic class CascataBase {\n  public func Valor() -> Int32 {\n    return NaoExisteCa();\n  }\n}\n");
        escrever(&mods, "zz-cb/b.reds", "module CascataB\nimport CascataA.*\n\npublic func UsaBase() -> Int32 {\n  let c = new CascataBase();\n  return c.Valor();\n}\n");
        let (r, e) = passo("cascata (A tipo, B usa A)");
        assert!(matches!(&r, Resultado::Compilou(l) if l.iter().any(|l| l.starts_with("só falhou depois que saiu") && l.contains("zz-ca"))), "{r:?}");
        assert_eq!((e.cor(&mods.join("zz-ca")), e.cor(&mods.join("zz-cb"))), (Some(CorDoMod::Vermelho), Some(CorDoMod::Vermelho)));
        std::fs::remove_dir_all(mods.join("zz-ca")).unwrap();
        std::fs::remove_dir_all(mods.join("zz-cb")).unwrap();
        // Classe repetida entre dois mods: o erro cai no que vem depois na ordem de caminho, e só ele sai.
        escrever(&mods, "zz-d1/d.reds", "public class ClasseRepetidaNaMedida {}\n");
        escrever(&mods, "zz-d2/d.reds", "public class ClasseRepetidaNaMedida {}\n");
        let (r, e) = passo("classe repetida em dois mods");
        assert!(matches!(&r, Resultado::Compilou(l) if l.iter().any(|l| l.contains("[SYM_REDEFINITION]"))), "{r:?}");
        assert_eq!((e.cor(&mods.join("zz-d1")), e.cor(&mods.join("zz-d2"))), (None, Some(CorDoMod::Vermelho)));
        std::fs::remove_dir_all(mods.join("zz-d1")).unwrap();
        std::fs::remove_dir_all(mods.join("zz-d2")).unwrap();
        // Dois `@replaceMethod` no mesmo método: compila, e o que o `scc` avisa fica amarelo.
        let replace = "@replaceMethod(PlayerPuppet)\nprotected cb func OnDetach() -> Bool { return true; }\n";
        escrever(&mods, "zz-r1/r.reds", replace);
        escrever(&mods, "zz-r2/r.reds", replace);
        let (r, e) = passo("@replaceMethod repetido");
        assert!(matches!(&r, Resultado::Compilou(l) if l.iter().any(|l| l.starts_with("aviso / warning:"))), "{r:?}");
        assert_eq!((e.cor(&mods.join("zz-r1")), e.cor(&mods.join("zz-r2"))), (None, Some(CorDoMod::Amarelo)));
        std::fs::remove_dir_all(mods.join("zz-r1")).unwrap();
        std::fs::remove_dir_all(mods.join("zz-r2")).unwrap();
        // Erro no bundle do BWMS: nenhum mod sai, o blob anterior fica.
        assert!(matches!(passo("tudo limpo").0, Resultado::Compilou(_)));
        let antes = std::fs::read(cache.join(TRIO[0])).unwrap();
        let no_bundle = escrever(&jogo, "r6/scripts/blackwall-mods/zz-medida-quebrado.reds", "func BwmsMedidaQuebrada() -> Int32 {\n  return NaoExisteNoBundle();\n}\n");
        let (r, _) = passo("erro no bundle do BWMS");
        assert!(matches!(&r, Resultado::ErroNasFontes(l) if l[1] == linha_de_erro_fora_dos_mods()), "{r:?}");
        assert_eq!(std::fs::read(cache.join(TRIO[0])).unwrap(), antes, "blob anterior no lugar");
        std::fs::remove_file(no_bundle).unwrap();
        // As fontes voltam a ser as da última compilação, e o blob é o dela: nada roda.
        assert!(matches!(passo("bundle consertado").0, Resultado::NadaMudou(_)));
    }

    /// Medida com o `scc` real, fora do jogo. `BWMS_MEDIDA_SCRIPTS=<pasta>` com `scc/scc` (e a `libscc_lib.dylib` ao lado),
    /// `vanilla.redscripts` e um jogo montado em `jogo/` (`r6/scripts`, `Mods/`, `red4ext/` com a `libcp77_console.dylib` pública); o teste
    /// refaz o `r6/cache` do vanilla e apaga os registros de `red4ext/`.
    /// `cargo test --lib medida_com_o_scc_real -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn medida_com_o_scc_real() {
        let Some(base) = std::env::var_os("BWMS_MEDIDA_SCRIPTS").map(PathBuf::from) else {
            println!("[medida] sem BWMS_MEDIDA_SCRIPTS: pulado");
            return;
        };
        let (jogo, scc) = (base.join("jogo"), base.join("scc/scc"));
        let (red4, cache) = (jogo.join("red4ext"), jogo.join("r6/cache"));
        for nome in TRIO {
            let _ = std::fs::remove_file(cache.join(nome));
        }
        let _ = std::fs::remove_file(red4.join(ASSINATURA));
        let _ = std::fs::remove_file(red4.join(FALHA));
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::copy(base.join("vanilla.redscripts"), cache.join(TRIO[0])).unwrap();
        let passo = |nome: &str| {
            let t0 = Instant::now();
            let r = executar(&jogo, &red4, Some(&scc), "");
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let resumo = match &r {
                Resultado::NadaMudou(l) => format!("NadaMudou, {} linha(s)", l.len()),
                Resultado::Compilou(l) => format!("Compilou, {} linha(s)", l.len()),
                Resultado::ErroNasFontes(l) => format!("ErroNasFontes, {} linha(s)", l.len()),
                Resultado::Adotou => "Adotou".to_string(),
                Resultado::NaoRodou(m, _) => format!("NaoRodou: {}", m.pt),
            };
            println!("[medida] {nome}: {resumo} em {ms:.1} ms; blob {} B", std::fs::metadata(cache.join(TRIO[0])).unwrap().len());
            r
        };
        let uma_linha = |r: &Resultado| matches!(r, Resultado::Compilou(l) | Resultado::NadaMudou(l) if l.len() == 1 && l[0].starts_with("compilados"));
        let r = passo("1ª abertura, com a Mods/");
        assert!(matches!(r, Resultado::Compilou(_)) && uma_linha(&r), "{r:?}");
        let compilados = r;
        for i in 1..=3 {
            let r = passo(&format!("sem mudança {i}"));
            assert!(matches!(r, Resultado::NadaMudou(_)) && uma_linha(&r), "{r:?}");
        }
        let quebrado = escrever(&jogo, "Mods/zz-quebrado/q.reds", "module Quebrado\n\npublic func Quebra() -> Int32 {\n  return NaoExiste();\n}\n");
        // O mod quebrado sai sozinho: a compilação sem ele dá certo.
        let r = passo("mod quebrado na Mods/");
        assert!(matches!(&r, Resultado::Compilou(l) if l.iter().any(|l| l.contains("q.reds:4: [UNRESOLVED_FN]"))), "{r:?}");
        assert_eq!(passo("mod quebrado, abertura seguinte sem mudança"), Resultado::NadaMudou(linhas_do_relatorio(&r)));
        std::fs::remove_dir_all(quebrado.parent().unwrap()).unwrap();
        assert!(uma_linha(&passo("mod quebrado saiu")));
        // O `scc` sai só com "Could not load script sources" para `.reds` fora do UTF-8 e para pasta com nome `.reds`; a busca aponta.
        let latin1 = escrever(&jogo, "Mods/zz-latin1/l.reds", "");
        std::fs::write(&latin1, b"func Caf\xe9NaMedida() -> Int32 { return 1; }\n").unwrap();
        let r = passo("`.reds` fora do UTF-8 na Mods/");
        assert!(matches!(&r, Resultado::Compilou(l) if l.len() == 3 && l[2].contains("l.reds: não está em UTF-8")), "{r:?}");
        assert_eq!(passo("`.reds` fora do UTF-8, abertura seguinte sem mudança"), Resultado::NadaMudou(linhas_do_relatorio(&r)));
        let (f, extras) = (fontes(&jogo.join("r6/scripts"), &[]), carregador::permitidos(&carregador::mods_extras(&jogo, ""), &[]));
        let t0 = Instant::now();
        let culpadas = fontes_que_o_scc_nao_carrega(&f, &extras);
        println!(
            "[medida] só a busca da fonte que o scc não carrega: {} .reds lidos, {} culpada(s), {:.1} ms",
            f.scripts.len() + extras.len(),
            culpadas.len(),
            t0.elapsed().as_secs_f64() * 1000.0
        );
        std::fs::remove_dir_all(latin1.parent().unwrap()).unwrap();
        assert!(uma_linha(&passo("`.reds` fora do UTF-8 saiu")));
        let pasta = jogo.join("r6/scripts/zz-pasta.reds");
        std::fs::create_dir_all(&pasta).unwrap();
        let r = passo("pasta com nome .reds em r6/scripts");
        assert!(matches!(&r, Resultado::ErroNasFontes(l) if l.len() == 3 && l[2].contains("zz-pasta.reds: é uma pasta")), "{r:?}");
        std::fs::remove_dir(&pasta).unwrap();
        assert!(uma_linha(&passo("pasta com nome .reds saiu")));
        let windows = escrever(&jogo, "Mods/zz-windows/w.reds", "module PluginWindows\n\npublic static native func NaoRegistradaNoMac() -> Bool\n");
        let r = passo("mod com native sem registro na Mods/");
        assert!(matches!(&r, Resultado::Compilou(l) if l.len() == 3), "{r:?}");
        assert!(matches!(passo("mod com native sem registro, sem mudança"), Resultado::NadaMudou(l) if l.len() == 3));
        std::fs::remove_dir_all(windows.parent().unwrap()).unwrap();
        assert!(uma_linha(&passo("mod com native saiu")));
        // Mod de `r6/scripts` com native sem registro: o blob em uso é o nosso, sem ele, e o jogo abre.
        let em_r6 = escrever(&jogo, "r6/scripts/zz-windows/w.reds", "module PluginWindows\n\nnative func NaoRegistradaNoMac() -> Bool\n");
        let r = passo("mod de r6/scripts com native sem registro, blob nosso");
        assert!(matches!(&r, Resultado::ErroNasFontes(l) if l[0].contains("O jogo abre")), "{r:?}");
        // O instalador compila `r6/scripts` inteiro: o blob passa a declarar a native.
        assert!(std::process::Command::new(&scc).arg("-compile").arg(jogo.join("r6/scripts")).output().unwrap().status.success());
        let lido = crate::scripts::blob::ler(&cache.join(TRIO[0])).unwrap();
        for (i, d) in lido.defs.iter().enumerate().filter(|(_, d)| d.tipo == crate::scripts::blob::TIPO_FUNCAO) {
            if lido.nome(d.nome).contains("NaoRegistradaNoMac") {
                println!("[medida] no blob: {:?} nativa={}", lido.nome(d.nome), lido.funcao(i).map(|f| f.nativa()).unwrap_or(false));
            }
        }
        let barrado = Barrado { raiz: em_r6.parent().unwrap().to_path_buf(), natives: vec![], nomes: vec!["NaoRegistradaNoMac".into()] };
        let t0 = Instant::now();
        let em_uso = blob_em_uso(&cache.join(TRIO[0]), [&barrado]);
        println!("[medida] blob_em_uso no blob de {} B: {em_uso:?} em {:.1} ms", lido.bytes.len(), t0.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(em_uso, BlobEmUso::Fecha);
        let r = passo("mod de r6/scripts com native sem registro, blob de fora com ele");
        assert!(matches!(&r, Resultado::ErroNasFontes(l) if l[0].contains("o jogo fecha ao ligar os scripts até o mod sair de r6/scripts")), "{r:?}");
        std::fs::remove_dir_all(em_r6.parent().unwrap()).unwrap();
        assert!(uma_linha(&passo("mod de r6/scripts saiu")));
        // O pior caso da conferência: os `.reds` do bundle do BWMS, todos com native, contra a dylib.
        let bundle = carregador::ModDeScripts { raiz: jogo.join("r6/scripts/blackwall-mods"), reds: fontes(&jogo.join("r6/scripts"), &[]).scripts };
        let t0 = Instant::now();
        let barrados = carregador::barrados(std::slice::from_ref(&bundle), &carregador::binarios(&red4)).unwrap();
        println!(
            "[medida] conferência de {} .reds do bundle contra {} binário(s): {:.1} ms; natives sem registro: {}",
            bundle.reds.len(),
            carregador::binarios(&red4).len(),
            t0.elapsed().as_secs_f64() * 1000.0,
            barrados.iter().map(|b| b.natives.len()).sum::<usize>()
        );
        let instalador = std::process::Command::new(&scc).arg("-compile").arg(jogo.join("r6/scripts")).output().unwrap();
        assert!(instalador.status.success());
        assert!(uma_linha(&passo("depois de um scc de fora só com r6/scripts")));
        // O que a aplicação a quente faz no fim: copia por cima do cache um blob compilado com outro mtime.
        let quente = base.join("quente.redscripts");
        std::fs::copy(cache.join(TRIO[0]), &quente).unwrap();
        std::fs::File::options().write(true).open(&quente).unwrap().set_modified(std::time::SystemTime::now()).unwrap();
        std::fs::copy(&quente, cache.join(TRIO[0])).unwrap();
        let _ = std::fs::remove_file(&quente);
        assert!(uma_linha(&passo("blob compilado copiado por fora do scc")));
        assert!(!contem(&std::fs::read(cache.join(TRIO[1])).unwrap(), MARCA_DO_SCC), "o .bk continua vanilla");
        // O `.bk` estragado (um compilado no lugar do vanilla) e a cópia do instalador ao lado: conserto e segunda passada.
        std::fs::copy(base.join("vanilla.redscripts"), cache.join(ORIGINAL_DO_INSTALADOR)).unwrap();
        std::fs::copy(cache.join(TRIO[0]), cache.join(TRIO[1])).unwrap();
        escrever(&jogo, "Mods/zz-novo/n.reds", "func NovoNaMedida() -> Int32 { return 1; }\n");
        assert!(uma_linha(&passo("backup estragado, reposto da cópia do instalador")));
        assert!(!contem(&std::fs::read(cache.join(TRIO[1])).unwrap(), MARCA_DO_SCC));
        std::fs::remove_dir_all(jogo.join("Mods/zz-novo")).unwrap();
        let _ = std::fs::remove_file(cache.join(ORIGINAL_DO_INSTALADOR));
        // `Mods/` grande: 100 mods, cada um com 1 `.reds` e 39 arquivos de outro tipo em 3 pastas (4.000 arquivos a mais).
        let grande = jogo.join("Mods/zz-grande");
        for m in 0..100 {
            escrever(&grande, &format!("mod-{m:03}/r6/scripts/m.reds"), &format!("func GrandeNaMedida{m:03}() -> Int32 {{ return {m}; }}\n"));
            for a in 0..39 {
                let pasta = ["archive/pc/mod", "r6/tweaks", "textures"][a % 3];
                escrever(&grande, &format!("mod-{m:03}/{pasta}/arquivo-{a:02}.bin"), "x");
            }
        }
        let r = passo("Mods/ grande, 1ª abertura");
        assert!(matches!(&r, Resultado::Compilou(l) if l[0].contains(": 148 .reds")), "{r:?}");
        for i in 1..=3 {
            assert!(matches!(passo(&format!("Mods/ grande, sem mudança {i}")), Resultado::NadaMudou(_)));
        }
        for i in 1..=3 {
            let t0 = Instant::now();
            let extras = carregador::mods_extras(&jogo, "");
            println!(
                "[medida] Mods/ grande, só a varredura da Mods/ (mods_extras) {i}: {} mods, {:.1} ms",
                extras.len(),
                t0.elapsed().as_secs_f64() * 1000.0
            );
        }
        std::fs::remove_dir_all(&grande).unwrap();
        assert_eq!(passo("Mods/ grande saiu"), compilados);
    }
}
