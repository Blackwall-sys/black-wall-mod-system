//! Codeware `#153`: entidade criada por mod sobrevive a salvar e carregar.
//!
//! O `DynamicEntitySpec` já tem `persistSpawn`/`persistState` e, até aqui, os dois campos não faziam
//! nada. A referência do Windows resolve isso serializando as entidades DENTRO do save
//! (`DynamicEntitySystemPS extends PersistentState`). O BWMS não faz isso: a regra do projeto é que
//! nada de mod entra no save, pra que desinstalar o mod nunca estrague o jogo salvo.
//!
//! Forma própria: o spec de cada entidade persistente fica num arquivo NOSSO, um por save, em
//! `r6/storage/bwms/entidades/<save>.toml`; na carga o BWMS recria as entidades pelo mesmo caminho
//! do `#73b` (`CreateEntity`). Quem carrega um save sem arquivo não ganha entidade nenhuma, e quem
//! desinstala o BWMS fica com um save vanilla íntegro.
//!
//! Como o save é identificado, sem RE: o menu de load é redscript (`LoadGameMenuGameController`),
//! então um `@wrapMethod` informa o `internalName` do save escolhido pela native
//! `BwmsSaveCarregado`. Sem aviso (o "Continuar", que carrega o último checkpoint), vale o save mais
//! recente da pasta — que é o que "último checkpoint" quer dizer.

use std::path::PathBuf;
use std::sync::Mutex;

/// Um spec guardado: o que basta pro `CreateEntity` recriar a entidade igual.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct Spec {
    pub record: u64,
    pub template: u64,
    pub aparencia: u64,
    pub posicao: [f32; 4],
    pub orientacao: [f32; 4],
    pub tags: Vec<u64>,
    pub estado: bool,
}

static VIVAS: Mutex<Vec<(u64, Spec)>> = Mutex::new(Vec::new());
static SAVE_INFORMADO: Mutex<String> = Mutex::new(String::new());
/// O save da sessão corrente, resolvido na entrada dela (`sessao_tick`): o informado pelo menu ou, sem aviso, o mais recente.
static SAVE_DA_SESSAO: Mutex<String> = Mutex::new(String::new());
/// Marca de tempo do save mais novo já visto, pra detectar save novo sem gancho no motor.
static SAVE_VISTO: Mutex<Option<std::time::SystemTime>> = Mutex::new(None);

/// Pasta de saves do jogo. O BWMS só LÊ os nomes daqui — nunca escreve dentro de um save.
fn pasta_de_saves() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home)
        .join("Library/Application Support/CD Projekt Red/Cyberpunk 2077/saves");
    p.is_dir().then_some(p)
}

fn pasta_de_specs() -> Option<PathBuf> {
    let jogo = PathBuf::from(crate::dylib_dir()?).parent()?.to_path_buf();
    let p = jogo.join("r6/storage/bwms/entidades");
    std::fs::create_dir_all(&p).ok()?;
    Some(p)
}

fn arquivo_do_save(save: &str) -> Option<PathBuf> {
    // Nome de save vem do jogo, mas só o que é nome de pasta passa: sem `/` nem `..`.
    if save.is_empty() || save.contains('/') || save.contains("..") {
        return None;
    }
    Some(pasta_de_specs()?.join(format!("{save}.toml")))
}

/// O save mais recente da pasta (o que o "Continuar" carrega), com a data dele.
fn save_mais_recente() -> Option<(String, std::time::SystemTime)> {
    let mut melhor: Option<(String, std::time::SystemTime)> = None;
    for e in std::fs::read_dir(pasta_de_saves()?).ok()?.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(nome) = e.file_name().to_str().map(str::to_string) else { continue };
        let Ok(t) = e.metadata().and_then(|m| m.modified()) else { continue };
        if melhor.as_ref().is_none_or(|(_, b)| t > *b) {
            melhor = Some((nome, t));
        }
    }
    melhor
}

/// O save que vale agora: o da sessão corrente, ou o mais recente antes da 1ª entrada em sessão.
pub(crate) fn save_corrente() -> Option<String> {
    let da_sessao = SAVE_DA_SESSAO.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if !da_sessao.is_empty() {
        return Some(da_sessao);
    }
    save_mais_recente().map(|(n, _)| n)
}

/// `BwmsSaveCarregado(nome)`: o menu de load diz qual save vai entrar.
pub(crate) fn informar_save(nome: &str) {
    *SAVE_INFORMADO.lock().unwrap_or_else(|e| e.into_inner()) = nome.to_string();
    crate::log_dev!("[ent153] save carregado: {nome}");
}

/// Um spec igual já está na lista da sessão? Compara o record e a posição com meio metro de folga —
/// o texto do arquivo passa por `Display`, então igualdade exata de float não serve.
fn ja_esta_viva(s: &Spec) -> bool {
    let v = VIVAS.lock().unwrap_or_else(|e| e.into_inner());
    v.iter().any(|(_, e)| {
        e.record == s.record
            && (0..3).all(|k| (e.posicao[k] - s.posicao[k]).abs() < 0.5)
    })
}

/// Registra o spec de uma entidade criada com `persistSpawn`.
pub(crate) fn registrar(id: u64, spec: Spec) {
    let estava_vazia = {
        let mut v = VIVAS.lock().unwrap_or_else(|e| e.into_inner());
        let vazia = v.is_empty();
        v.retain(|(k, _)| *k != id);
        v.push((id, spec));
        vazia
    };
    // 1ª entidade (criada pelo mod, ou pelo `recriar`/fila da carga): o save mais novo de agora foi feito sem ela, então o vigia
    // só grava em save feito daqui pra frente. A pasta é lida fora da trava da lista.
    if estava_vazia {
        if let Some((_, data)) = save_mais_recente() {
            *SAVE_VISTO.lock().unwrap_or_else(|e| e.into_inner()) = Some(data);
        }
    }
}

/// Esquece o spec de uma entidade apagada pelo mod.
pub(crate) fn esquecer(id: u64) {
    VIVAS.lock().unwrap_or_else(|e| e.into_inner()).retain(|(k, _)| *k != id);
}

pub(crate) fn quantas() -> usize {
    VIVAS.lock().unwrap_or_else(|e| e.into_inner()).len()
}

fn texto(specs: &[(u64, Spec)]) -> String {
    let mut s = String::from(
        "# BWMS: entidades de mod pra recriar quando este save for carregado.\n\
         # Escrito pelo BWMS, FORA do save — desinstalar o mod deixa o save vanilla intacto.\n\
         versao = 1\n",
    );
    for (_, e) in specs {
        s.push_str("\n[[entidade]]\n");
        s.push_str(&format!("record = \"{:#018x}\"\n", e.record));
        s.push_str(&format!("template = \"{:#018x}\"\n", e.template));
        s.push_str(&format!("aparencia = \"{:#018x}\"\n", e.aparencia));
        s.push_str(&format!(
            "posicao = [{}, {}, {}, {}]\n",
            e.posicao[0], e.posicao[1], e.posicao[2], e.posicao[3]
        ));
        s.push_str(&format!(
            "orientacao = [{}, {}, {}, {}]\n",
            e.orientacao[0], e.orientacao[1], e.orientacao[2], e.orientacao[3]
        ));
        let tags: Vec<String> = e.tags.iter().map(|t| format!("\"{t:#018x}\"")).collect();
        s.push_str(&format!("tags = [{}]\n", tags.join(", ")));
        s.push_str(&format!("estado = {}\n", e.estado));
    }
    s
}

fn u64_de(v: &str) -> u64 {
    let v = v.trim().trim_matches('"');
    v.strip_prefix("0x")
        .and_then(|h| u64::from_str_radix(h, 16).ok())
        .or_else(|| v.parse().ok())
        .unwrap_or(0)
}

fn lista_de<const N: usize>(v: &str, u64s: bool) -> ([f32; N], Vec<u64>) {
    let dentro = v.trim().trim_start_matches('[').trim_end_matches(']');
    let mut f = [0.0f32; N];
    let mut u = Vec::new();
    for (i, parte) in dentro.split(',').map(str::trim).filter(|p| !p.is_empty()).enumerate() {
        if u64s {
            u.push(u64_de(parte));
        } else if i < N {
            f[i] = parte.parse().unwrap_or(0.0);
        }
    }
    (f, u)
}

/// Lê os specs de um save. Formato próprio, linha a linha: arquivo torto perde a entrada, não o jogo.
pub(crate) fn ler(save: &str) -> Vec<Spec> {
    let Some(caminho) = arquivo_do_save(save) else { return Vec::new() };
    let Ok(txt) = std::fs::read_to_string(&caminho) else { return Vec::new() };
    let mut saida = Vec::new();
    let mut atual: Option<Spec> = None;
    for linha in txt.lines() {
        let l = linha.trim();
        if l.starts_with("[[entidade]]") {
            if let Some(e) = atual.take() {
                saida.push(e);
            }
            atual = Some(Spec::default());
            continue;
        }
        let Some(e) = atual.as_mut() else { continue };
        let Some((chave, valor)) = l.split_once('=') else { continue };
        match chave.trim() {
            "record" => e.record = u64_de(valor),
            "template" => e.template = u64_de(valor),
            "aparencia" => e.aparencia = u64_de(valor),
            "posicao" => e.posicao = lista_de::<4>(valor, false).0,
            "orientacao" => e.orientacao = lista_de::<4>(valor, false).0,
            "tags" => e.tags = lista_de::<4>(valor, true).1,
            "estado" => e.estado = valor.trim() == "true",
            _ => {}
        }
    }
    if let Some(e) = atual {
        saida.push(e);
    }
    saida
}

/// Grava os specs vivos no arquivo do save dado. Sem entidade persistente, apaga o arquivo — save
/// sem entidade de mod não carrega lixo nosso.
pub(crate) fn gravar(save: &str) -> bool {
    let Some(caminho) = arquivo_do_save(save) else { return false };
    let v = VIVAS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if v.is_empty() {
        let _ = std::fs::remove_file(&caminho);
        return true;
    }
    let ok = std::fs::write(&caminho, texto(&v)).is_ok();
    crate::log_dev!("[ent153] {} spec(s) gravado(s) em {} = {ok}", v.len(), caminho.display());
    ok
}

/// No tick: save novo na pasta (o jogador salvou) → grava os specs vivos com o nome dele. Não escreve
/// NADA dentro do save; só usa o nome pra chavear o arquivo nosso.
pub(crate) fn vigia_de_saves_tick() {
    static ULTIMO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let agora = crate::ticks();
    if agora.wrapping_sub(ULTIMO.load(std::sync::atomic::Ordering::Relaxed)) < 120 {
        return;
    }
    ULTIMO.store(agora, std::sync::atomic::Ordering::Relaxed);
    // Sem entidade persistente nesta sessão não há nada pra gravar — sai ANTES de ler a pasta de
    // saves. Era uma leitura de diretório a cada 120 ticks em toda partida, inclusive nas que nenhum
    // mod usa `persistSpawn` (a maioria).
    let primeira_vez = SAVE_VISTO.lock().is_ok_and(|v| v.is_none());
    if quantas() == 0 && !primeira_vez {
        return;
    }
    let Some((nome, data)) = save_mais_recente() else { return };
    let mut visto = SAVE_VISTO.lock().unwrap_or_else(|e| e.into_inner());
    if visto.is_some_and(|v| v >= data) {
        return;
    }
    let primeira = visto.is_none();
    *visto = Some(data);
    drop(visto);
    // Na primeira passada só anota o que já existia: gravar aqui sobrescreveria o arquivo de um save
    // antigo com os specs desta sessão.
    if primeira || quantas() == 0 {
        return;
    }
    gravar(&nome);
}

/// Recria as entidades do save que está entrando. Chamado pelo `sessao_tick` a cada entrada em sessão e pela native
/// `BwmsRecriarEntidades` (`tramp_recriar_entidades`).
///
/// # Safety
/// Thread do jogo, com a sessão montada — os sistemas de spawn precisam existir.
pub(crate) unsafe fn recriar() -> usize {
    use std::sync::atomic::Ordering;
    // Uma recriação por geração (o `sessao_tick` abre uma a cada entrada em sessão): recriar duas vezes
    // duplicaria as entidades. A native de forçar passa antes por `esquecer_a_geracao`.
    let g = GERACAO.load(Ordering::Relaxed);
    if RECRIADA_NA.swap(g, Ordering::AcqRel) == g && g != 0 {
        return 0;
    }
    let Some(save) = save_corrente() else { return 0 };
    let specs = ler(&save);
    // Tira da lista o que o jogo não conhece mais (entidade que morreu com o mundo anterior). Limpar a
    // lista inteira aqui era errado: se o gatilho acorda tarde na MESMA sessão, apagaria o registro do
    // que o mod acabou de criar e a checagem de "já está viva" deixaria de valer, duplicando.
    let esquecidas = crate::npcs::dynent_esquecer_de_outra_sessao();
    VIVAS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(id, _)| crate::npcs::dynent_gerenciada(*id));
    if esquecidas > 0 {
        crate::log_dev!("[ent153] {esquecidas} entidade(s) da sessão anterior esquecida(s)");
    }
    RECRIACOES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if specs.is_empty() {
        return 0;
    }
    let mut feitas = 0usize;
    let mut sobraram = Vec::new();
    for s in specs {
        // Já existe uma entidade viva com esse record naquele ponto? Então não é recriação: é o gatilho
        // acordando tarde na MESMA sessão (medido no boot 312: a recriação saiu logo depois de o mod
        // criar a entidade, duplicando). Recriar aqui daria duas entidades no mesmo lugar.
        if ja_esta_viva(&s) {
            continue;
        }
        let id = crate::npcs::dynent_criar_do_spec(&s);
        if id != 0 {
            registrar(id, s);
            feitas += 1;
        } else {
            sobraram.push(s);
        }
    }
    RECRIADAS.store(feitas as u64, std::sync::atomic::Ordering::Relaxed);
    // O `sessao_tick` dispara no fim da carga com o jogador do mundo novo, e nesse instante os sistemas de spawn podem ainda
    // não resolver. Quem não nasceu fica na fila e o tick tenta de novo — sem isso a recriação perde a
    // única chance por uma questão de momento.
    if !sobraram.is_empty() {
        *PENDENTES.lock().unwrap_or_else(|e| e.into_inner()) = sobraram;
        PENDENTES_DESDE.store(crate::ticks(), std::sync::atomic::Ordering::Relaxed);
    }
    crate::log_dev!(
        "[ent153] {feitas} entidade(s) recriada(s) do save {save}, {} na fila",
        PENDENTES.lock().unwrap_or_else(|e| e.into_inner()).len()
    );
    feitas
}

/// Gatilho da recriação: entrar numa sessão de jogo, pela fase da sessão que o BWMS já lê. É a única
/// fonte — o `Session/Ready` do `#129` não chama mais, porque dois gatilhos na mesma sessão
/// duplicariam as entidades, e este não depende de nada vindo do script.
static GERACAO: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RECRIADA_NA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(u64::MAX);
static EM_SESSAO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Faz a próxima `recriar()` valer mesmo já tendo rodado nesta sessão (a native de forçar usa).
pub(crate) fn esquecer_a_geracao() {
    RECRIADA_NA.store(u64::MAX, std::sync::atomic::Ordering::Relaxed);
}

/// No tick: a cada entrada em sessão (fase 5, carga do mundo terminada, jogador do mundo novo), recria o que o save tem. Na 1ª leitura de
/// phase 5 a carga só foi pedida, e recriar ali leu um `GameInstance` que não era (crash SIGBUS de 2026-10-25, `z037/relatorios/CRASH.md`).
pub(crate) fn sessao_tick() {
    use std::sync::atomic::Ordering;
    let dentro = crate::inicio::boot::fase_lida() == Some(crate::inicio::boot::FaseDaSessao::Jogo)
        && crate::inicio::boot::carga_do_mundo_terminou()
        && !crate::current_player().is_null();
    if EM_SESSAO.swap(dentro, Ordering::AcqRel) == dentro || !dentro {
        return;
    }
    // O aviso do menu vale só pra esta entrada: o "Continuar" e a recarga do checkpoint depois da morte não avisam e entram no
    // save mais recente. O nome fica preso pela sessão, então o `BwmsRecriarEntidades` repete o mesmo save.
    let informado = std::mem::take(&mut *SAVE_INFORMADO.lock().unwrap_or_else(|e| e.into_inner()));
    let save = if informado.is_empty() { save_mais_recente().map(|(n, _)| n).unwrap_or_default() } else { informado };
    *SAVE_DA_SESSAO.lock().unwrap_or_else(|e| e.into_inner()) = save;
    GERACAO.fetch_add(1, Ordering::AcqRel);
    unsafe { recriar() };
}

/// Specs que não nasceram na 1ª tentativa (sistemas de spawn ainda não resolvidos na carga).
static PENDENTES: Mutex<Vec<Spec>> = Mutex::new(Vec::new());
static PENDENTES_DESDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// No tick: tenta criar quem sobrou da recriação, por até ~60 s depois da carga.
pub(crate) fn fila_tick() {
    use std::sync::atomic::Ordering;
    if PENDENTES.lock().is_ok_and(|p| p.is_empty()) {
        return;
    }
    let desde = PENDENTES_DESDE.load(Ordering::Relaxed);
    let agora = crate::ticks();
    if agora.wrapping_sub(desde) > 3600 {
        let n = PENDENTES.lock().map(|mut p| std::mem::take(&mut *p).len()).unwrap_or(0);
        crate::log_dev!("[ent153] desisti de {n} entidade(s) da fila: os sistemas de spawn não resolveram");
        return;
    }
    if !crate::tempo::a_cada(agora, 60) {
        return;
    }
    let _ = unsafe { crate::npcs::dynent_esquecer_de_outra_sessao() };
    let fila = std::mem::take(&mut *PENDENTES.lock().unwrap_or_else(|e| e.into_inner()));
    let mut sobraram = Vec::new();
    let mut feitas = 0u64;
    for s in fila {
        let id = unsafe { crate::npcs::dynent_criar_do_spec(&s) };
        if id != 0 {
            registrar(id, s);
            feitas += 1;
        } else {
            sobraram.push(s);
        }
    }
    if feitas > 0 {
        RECRIADAS.fetch_add(feitas, Ordering::Relaxed);
        crate::log_dev!("[ent153] {feitas} entidade(s) da fila recriada(s)");
    }
    *PENDENTES.lock().unwrap_or_else(|e| e.into_inner()) = sobraram;
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn texto_ida_e_volta() {
        let s = Spec {
            record: 0x1234_5678_9abc_def0,
            template: 0xdead_beef,
            aparencia: 0x42,
            posicao: [1.5, -2.25, 3.0, 1.0],
            orientacao: [0.0, 0.0, 0.7071, 0.7071],
            tags: vec![0xaa, 0xbb],
            estado: true,
        };
        let t = texto(&[(7, s.clone())]);
        // Escreve e lê pelo mesmo caminho do jogo, num diretório temporário.
        let dir = std::env::temp_dir().join("bwms-teste-153");
        std::fs::create_dir_all(&dir).unwrap();
        let arq = dir.join("SaveDeTeste.toml");
        std::fs::write(&arq, &t).unwrap();
        let txt = std::fs::read_to_string(&arq).unwrap();
        let mut lidos = Vec::new();
        let mut atual: Option<Spec> = None;
        for linha in txt.lines() {
            let l = linha.trim();
            if l.starts_with("[[entidade]]") {
                if let Some(e) = atual.take() {
                    lidos.push(e);
                }
                atual = Some(Spec::default());
                continue;
            }
            let Some(e) = atual.as_mut() else { continue };
            let Some((chave, valor)) = l.split_once('=') else { continue };
            match chave.trim() {
                "record" => e.record = u64_de(valor),
                "template" => e.template = u64_de(valor),
                "aparencia" => e.aparencia = u64_de(valor),
                "posicao" => e.posicao = lista_de::<4>(valor, false).0,
                "orientacao" => e.orientacao = lista_de::<4>(valor, false).0,
                "tags" => e.tags = lista_de::<4>(valor, true).1,
                "estado" => e.estado = valor.trim() == "true",
                _ => {}
            }
        }
        if let Some(e) = atual {
            lidos.push(e);
        }
        let _ = std::fs::remove_file(&arq);
        assert_eq!(lidos.len(), 1);
        assert_eq!(lidos[0].record, s.record);
        assert_eq!(lidos[0].template, s.template);
        assert_eq!(lidos[0].aparencia, s.aparencia);
        assert_eq!(lidos[0].posicao, s.posicao);
        assert_eq!(lidos[0].orientacao, s.orientacao);
        assert_eq!(lidos[0].tags, s.tags);
        assert!(lidos[0].estado);
    }

    #[test]
    fn nome_de_save_com_travessia_recusado() {
        assert!(arquivo_do_save("../fora").is_none());
        assert!(arquivo_do_save("a/b").is_none());
        assert!(arquivo_do_save("").is_none());
    }
}

// ===== Teste do ciclo (marcador `~/.bwms-hook-ent153-teste`) =====
// Em gameplay: cria a entidade persistente, grava o arquivo do save corrente, recarrega o último
// checkpoint pelo próprio jogo e confere que a entidade voltou pelo `Session/Ready`. Nenhum save é
// criado, apagado ou escrito — só o arquivo do BWMS, que o teste remove no fim.

/// Partes anotadas pelo `.reds`: 0 id criado, 1 `IsManaged`, 2/3 tag e entidade antes da carga,
/// 4/5 depois da carga, 9 pedido de carga aceito.
#[cfg(feature = "devtools")]
static PARTES: [std::sync::atomic::AtomicU64; 12] =
    [const { std::sync::atomic::AtomicU64::new(u64::MAX) }; 12];
/// Quantas vezes a recriação rodou nesta sessão e quantas entidades saíram dela.
pub(crate) static RECRIACOES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static RECRIADAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Segundos de gameplay antes de começar: com o teste da troca de corpo no mesmo boot, espera o
/// veredito dele (ou 260 s, o teto dele) pra não recarregar o mundo no meio.
#[cfg(feature = "devtools")]
fn espera_de_gameplay() -> u64 {
    if (crate::marcador_individual("troca-de-corpo") && !crate::scripts::tradutor_de_bytecode::teste_terminou())
        || (crate::marcador_individual("vigia-quente") && !crate::scripts::tradutor_de_bytecode::teste_do_vigia_terminou())
    {
        260
    } else {
        12
    }
}

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    use std::sync::atomic::Ordering;
    use std::time::Instant;
    static FASE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static VOLTAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static MARCO: Mutex<Option<Instant>> = Mutex::new(None);
    static NOTAS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static SAVE: Mutex<String> = Mutex::new(String::new());
    static TENTATIVAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    static ULTIMA_TENTATIVA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let fase = FASE.load(Ordering::Relaxed);
    if fase >= 9
        || crate::inicio::boot::exec_nested()
        || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30)
        || !crate::marcador_individual("ent153-teste")
    {
        return;
    }
    let parte = |k: usize| PARTES[k].load(Ordering::Relaxed) as i64;
    let nota = |s: String| NOTAS.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    let segundos = || MARCO.lock().unwrap_or_else(|e| e.into_inner()).map_or(0, |t: Instant| t.elapsed().as_secs());
    let marcar = || *MARCO.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    let fim = |ok: bool, motivo: &str| {
        FASE.store(9, Ordering::Relaxed);
        if let Some(s) = save_corrente().as_deref().and_then(arquivo_do_save) {
            let _ = std::fs::remove_file(s);
        }
        let notas = NOTAS.lock().unwrap_or_else(|e| e.into_inner()).join("; ");
        crate::veredito_dev!("[ent153] VEREDITO-153 {notas}{motivo}; persistencia-ok={ok}");
    };
    let player = crate::current_player();
    let chamar = |nome: &str, arg: Option<i32>| -> bool {
        unsafe {
            crate::rtti::resolve_in_class(crate::rtti::class_of(player), nome).is_some_and(|rf| {
                let args: Vec<crate::rtti::Arg> = arg.map(|v| vec![crate::rtti::Arg::I32(v as u32)]).unwrap_or_default();
                crate::rtti::call_func(&rf, player, &args).is_some()
            })
        }
    };

    match fase {
        0 if crate::inicio::boot::gameplay_real() && segundos() == 0 => {
            marcar();
            FASE.store(1, Ordering::Relaxed);
        }
        // Este teste RECARREGA o mundo, então ele espera o teste da troca de corpo (`#97`) terminar:
        // no mesmo boot, recarregar no meio da troca mataria a sessão dela. Sem aquele marcador, 12 s
        // de gameplay bastam pro mundo estar montado.
        1 if segundos() >= espera_de_gameplay() => {
            if !chamar("BwmsEnt153Criar", None) {
                return fim(false, " (BwmsEnt153Criar não está no bundle)");
            }
            nota(format!("criada: id={:#x} managed={}", parte(0), parte(1)));
            if parte(0) <= 0 {
                return fim(false, " (CreateEntity não deu id)");
            }
            if quantas() != 1 {
                return fim(false, &format!(" (spec não foi guardado: {} na lista)", quantas()));
            }
            marcar();
            FASE.store(2, Ordering::Relaxed);
        }
        // Espera a entidade aparecer no mundo (spawn leva alguns segundos) e grava o arquivo do save
        // que está carregado.
        2 if segundos() >= 6 => {
            chamar("BwmsEnt153Conferir", Some(2));
            if parte(2) < 1 && segundos() < 60 {
                return;
            }
            let Some(save) = save_corrente() else {
                return fim(false, " (não achei o save corrente)");
            };
            // Fixa o save da janela do teste: um autosave do jogo mudaria o "mais recente" e o
            // arquivo iria pro nome errado.
            informar_save(&save);
            *SAVE.lock().unwrap_or_else(|e| e.into_inner()) = save.clone();
            let gravou = gravar(&save);
            let existe = arquivo_do_save(&save).is_some_and(|p| p.is_file());
            nota(format!(
                "antes da carga: tag={} entidade={}; arquivo de {save} gravado={gravou} existe={existe}",
                parte(2),
                parte(3)
            ));
            if !gravou || !existe || parte(2) < 1 {
                return fim(false, " (spec não foi pro arquivo ou entidade não entrou no mundo)");
            }
            RECRIACOES.store(0, Ordering::Relaxed);
            if !chamar("BwmsEnt153Carregar", None) || parte(9) != 1 {
                return fim(false, " (o jogo não aceitou voltar ao menu)");
            }
            nota("volta ao menu pedida (a carga sai de lá pelo LoadLastCheckpoint)".into());
            marcar();
            FASE.store(3, Ordering::Relaxed);
        }
        // A carga leva ~40 s; o `Session/Ready` da sessão nova é quem recria.
        3 if RECRIACOES.load(Ordering::Relaxed) > 0 && !crate::current_player().is_null() => {
            nota(format!(
                "depois da carga: {} recriação(ões), {} entidade(s)",
                RECRIACOES.load(Ordering::Relaxed),
                RECRIADAS.load(Ordering::Relaxed)
            ));
            marcar();
            FASE.store(4, Ordering::Relaxed);
        }
        // Diagnóstico: no boot 309 o teste passou da gravação e nunca deu veredito. Uma linha a cada
        // ~30 s diz em que ponto a espera está, pra distinguir "carga não começou" de "tick parou".
        3 => {
            let s = segundos();
            // Já no menu (fase da sessão ≠ 5): é lá que o `LoadLastCheckpoint` vale. Tenta a cada 20 s,
            // porque o menu precisa de alguns segundos pra ter a lista de saves.
            let no_jogo = crate::inicio::boot::fase_lida() == Some(crate::inicio::boot::FaseDaSessao::Jogo);
            // Por tempo desde a última, não por módulo: o teste só avalia a cada ~5 s e um `s % 20`
            // pode nunca coincidir.
            let ultima = ULTIMA_TENTATIVA.load(Ordering::Relaxed);
            if !no_jogo && s >= 12 && crate::tempo::ja_passou(ultima, s, 20) && TENTATIVAS.load(Ordering::Relaxed) < 6 {
                TENTATIVAS.fetch_add(1, Ordering::Relaxed);
                ULTIMA_TENTATIVA.store(s, Ordering::Relaxed);
                let chamou = chamar("BwmsEnt153CarregarDoMenu", None);
                crate::log_dev!(
                    "[ent153] pedindo a carga do menu ({}ª): chamou={chamou} tem-checkpoint={}",
                    TENTATIVAS.load(Ordering::Relaxed),
                    parte(10)
                );
            }
            if s % 30 < 2 {
                crate::log_dev!(
                    "[ent153] esperando a sessão nova: {s}s, recriações={}, player={}, fase={}",
                    RECRIACOES.load(Ordering::Relaxed),
                    !crate::current_player().is_null(),
                    crate::inicio::boot::FASE_LIDA.load(Ordering::Relaxed)
                );
            }
            if s > 300 {
                fim(false, " (sessão nova não chegou em 300 s)");
            }
        }
        // Dá tempo pro spawn da entidade recriada e confere no mundo.
        4 if segundos() >= 15 => {
            chamar("BwmsEnt153Conferir", Some(4));
            let save = SAVE.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let mesmo_save = save_corrente().as_deref() == Some(save.as_str());
            nota(format!(
                "no mundo novo: tag={} entidade={} (save {save}, mesmo={mesmo_save})",
                parte(4),
                parte(5)
            ));
            // Depois da limpeza da sessão anterior, a tag tem que contar exatamente a entidade recriada.
            // `parte(4)` é quantos ids a tag tem, `parte(5)` quantos deles têm entidade VIVA. O que
            // importa é ter exatamente 1 viva — id de entidade que morreu com o mundo anterior é resto
            // de registro, e a limpeza de fantasma cuida dele.
            let ok = RECRIADAS.load(Ordering::Relaxed) == 1 && parte(5) == 1;
            fim(ok, "");
        }
        _ => {}
    }
}
