//! As natives que os scripts chamam durante o boot: continuar o último save, disparar o início, pular a intro, tela de engagement e o
//! fim do splash no menu. O registro delas continua no `register_all`, na mesma ordem.

use std::sync::Mutex;
use crate::tipos_do_motor::fire_event_args;
use crate::tipos_do_motor::register_type_instantiable_with_parent;
use crate::diagnostico::C129_DESPACHOS;
use std::ffi::c_void;

use crate::register::*;
use crate::rtti::{self, Registry};

pub(crate) unsafe extern "C" fn tramp_autocontinue(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    // `continuar_automatico` do `red4ext/bwms.toml` (desligado por padrão: qual save abrir é decisão do usuário), lido agora para o
    // seletor "Pular boot" mostrar a escolha recém-gravada. No dev o marcador do home vale por cima (`configuracao::do_mod`).
    let on = !crate::inicio::boot::autocontinue_suppressed_stale_boot()
        && crate::configuracao::do_mod::config_lida_agora().continuar_automatico;
    // DIAGNÓSTICO: loga 1x que o reds (OnSaveMetadataReady) chamou a native + o valor lido.
    static LOGGED_AC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED_AC.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::log_dev!("[autocontinue] opção lida pelo reds (BwmsAutoContinue ou BwmsAutoContinueDoBoot) = {on}");
    }
    write_bool_ret(out, on);
}

/// `BwmsAutoContinueDoBoot() -> Bool` — o que o `BwmsTryContinue` consulta: a opção `continuar_automatico`, mas só enquanto o boot
/// não resolveu a tarefa dele (`inicio::boot::menu_do_boot_resolvido`): num menu seguinte o load não é automático, porque sairia sem
/// splash. O `BwmsAutoContinue()` segue dizendo a opção, que é o que o seletor "Pular boot" mostra.
pub(crate) unsafe extern "C" fn tramp_autocontinue_do_boot(c: *mut c_void, f: *mut c_void, out: *mut c_void, rt: i64) {
    if crate::inicio::boot::menu_do_boot_resolvido() {
        write_bool_ret(out, false);
        return;
    }
    tramp_autocontinue(c, f, out, rt);
}

// ===== Seletor "Pular boot" da tela de configurações: a escolha vai para o `bwms.toml` (vale no próximo boot) =====
const COMENTARIO_CONTINUAR: &str = "Carrega o último save sozinho, sem passar pelo menu principal.";
const COMENTARIO_PULAR: &str = "Pula os logos de abertura e a tela \"APERTE ESPAÇO\" (a tela preta de loading continua, é streaming de disco).";

/// Grava a escolha do seletor; `false` se o arquivo não aceitou a escrita.
fn gravar_escolha(chave: &str, valor: bool, comentario: &str) -> bool {
    match crate::configuracao::do_mod::gravar_bool(chave, valor, comentario) {
        Ok(()) => true,
        Err(e) => {
            crate::log_dev!("[pular-boot] não gravei {chave}={valor} no bwms.toml: {e}");
            false
        }
    }
}

/// `BwmsAutoContinueOn() -> Bool` — `continuar_automatico = true` (entra no jogo sozinho no próximo boot).
pub(crate) unsafe extern "C" fn tramp_autocontinue_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, gravar_escolha("continuar_automatico", true, COMENTARIO_CONTINUAR));
}

/// `BwmsAutoContinueOff() -> Bool` — `continuar_automatico = false` (só até o menu no próximo boot).
pub(crate) unsafe extern "C" fn tramp_autocontinue_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, gravar_escolha("continuar_automatico", false, COMENTARIO_CONTINUAR));
}

/// `BwmsAutoContinueSaveIndex() -> Int32` — índice do save que o auto-continue abre: 0 (o mais recente). No dev,
/// `~/.bwms-autocontinue-saveindex` (decimal) escolhe outro, para comparar a duração do boot por região do mapa sem recompilar.
pub(crate) unsafe extern "C" fn tramp_autocontinue_saveindex(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-autocontinue-saveindex")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    crate::log_dev!("[autocontinue] BwmsAutoContinueSaveIndex() = {v}");
    write_uint_ret(out, v as u64, 4);
}

/// `BwmsFireStartOn() -> Bool` — o seletor chama nos níveis 1 e 2; nada a gravar: o lever segue `pular_intro` (`tramp_fire_start_state`),
/// que o `BwmsSkipIntroOn` grava.
pub(crate) unsafe extern "C" fn tramp_fire_start_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, true);
}

/// `BwmsFireStartOff() -> Bool` — nível 0 do seletor; nada a gravar, mesmo motivo do `BwmsFireStartOn`.
pub(crate) unsafe extern "C" fn tramp_fire_start_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, true);
}

/// `BwmsFireStartState() -> Bool` — o `bwms-skipintro.reds` arma o timer do lever (8 s) quando isto é true. Vale `pular_intro`
/// (`bwms.toml` ou `~/.bwms-skipintro`): com o skip ligado o splash cobre a engagement, e uma engagement esperando ESPAÇO por baixo
/// dele seria uma tela interativa escondida.
///
/// SEM guarda de dead-man's switch aqui de propósito: o lever (`BwmsFireStart`, d4=2) é o que
/// ATIVA o save-system nativo — pular ele deixa a state machine do jogo presa em d4=1 pra
/// sempre (sem input real de SPACE, que a splash do bwms esconde), travando a splash em
/// "iniciando a sessão" indefinidamente (achado testando esta mesma guarda: 1ª versão pulava
/// o lever inteiro e travava o boot pra sempre, pior que o crash-loop que devia evitar). A
/// guarda mora em `tramp_autocontinue` — só suprime o AUTO-LOAD (o passo mais pesado/arriscado),
/// nunca a ativação do save-system em si.
pub(crate) unsafe extern "C" fn tramp_fire_start_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, crate::configuracao::do_mod::config().pular_intro);
}

/// `BwmsAcFired()` — o auto-continue disparou (BwmsDoContinue vai carregar o último save). Marca de
/// prova/telemetria: uma linha por boot quando o pular-até-o-jogo acionou. Também alimenta a barra
/// de progresso REAL da splash (ver `selfboot::boot_progress`, 2026-07-12).
pub(crate) unsafe extern "C" fn tramp_ac_fired(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::inicio::boot::note_autocontinue_fired();
    crate::log_dev!("[autocontinue] disparou -> carregando o último save (LoadModdedSave/LoadLastCheckpoint)");
    write_bool_ret(out, true);
}

/// `BwmsMenuReadySplashOff() -> Bool` — o menu ficou pronto e não há load automático a fazer: autocontinue desligado (ou suprimido pelo
/// dead-man's switch), nenhum save, o save 0 inválido, ou um menu depois que o boot resolveu a tarefa (`menu_do_boot_resolvido`). Sem
/// load não existe fim de load para tirar o splash.
pub(crate) unsafe extern "C" fn tramp_menu_ready_splash_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::inicio::boot::marcar_menu_do_boot_resolvido();
    // Na volta ao menu (sessão seguinte) o splash já está desligado: sem linha, para a prova não contar a 2ª sessão.
    if crate::inicio::boot::boot_splash_active() {
        crate::inicio::boot::boot_splash_off();
        crate::veredito_dev!("[splash] fora: menu pronto, sem load a fazer, t={}s", crate::inicio::boot::boot_elapsed_secs());
    }
    write_bool_ret(out, true);
}

/// `BwmsEngagementOn() -> Bool` — o redscript (EngagementScreenGameController.OnInitialize) chama
/// isto ao ENTRAR na tela "APERTE ESPAÇO PARA CONTINUAR". É o sinal PRECISO da engagement do boot
/// (leitores em `terminal::overlay::engagement_active`).
pub(crate) unsafe extern "C" fn tramp_engagement_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::terminal::overlay::set_engagement_active(true);
    write_bool_ret(out, true);
}

/// `BwmsEngagementOff() -> Bool` — chamado no OnUninitialize da engagement → marca a engagement como encerrada. Não mexe no splash:
/// com ele armado o lever dispensou a engagement, e a tarefa do boot (menu pronto ou load) ainda não terminou.
pub(crate) unsafe extern "C" fn tramp_engagement_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::terminal::overlay::set_engagement_active(false);
    write_bool_ret(out, true);
}

/// `BwmsSkipIntroOn() -> Bool` — `pular_intro = true` (bink-skip, getter da phase, lever e splash no próximo boot).
pub(crate) unsafe extern "C" fn tramp_skipintro_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, gravar_escolha("pular_intro", true, COMENTARIO_PULAR));
}

/// `BwmsSkipIntroOff() -> Bool` — `pular_intro = false` (logos e tela "APERTE ESPAÇO" de volta no próximo boot).
pub(crate) unsafe extern "C" fn tramp_skipintro_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, gravar_escolha("pular_intro", false, COMENTARIO_PULAR));
}

/// `BwmsSkipIntroState() -> Bool` — o `pular_intro` do `bwms.toml` agora, para o seletor mostrar a escolha atual.
pub(crate) unsafe extern "C" fn tramp_skipintro_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, crate::configuracao::do_mod::config_lida_agora().pular_intro);
}

pub(crate) unsafe extern "C" fn tramp_fire_start(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    use std::sync::atomic::Ordering;
    let func = current_native_func();
    let _args = rtti::read_params_consuming(func, frame);
    let on = crate::configuracao::do_mod::config().pular_intro;
    let sm = crate::inicio::boot::GAME_SESSION_DESC.load(Ordering::Relaxed);
    let Some(l) = LeituraDaEngagement::ler(sm) else {
        crate::log_dev!("[firestart] abortado: SM={sm:p} ilegível");
        return write_bool_ret(out, false);
    };
    if !on {
        crate::log_dev!("[firestart] gate OFF (pular_intro=false). {l:?} (só leitura)");
        return write_bool_ret(out, false);
    }
    if !l.alavanca_pode_disparar() || FIRE_START_DONE.load(Ordering::Relaxed) {
        crate::log_dev!("[firestart] guardas barraram: {l:?} done=(1x)");
        return write_bool_ret(out, false);
    }
    // O `cp77_tick` dispara a alavanca adiada (`ordem_tick`) quando o `OnApply` do menu termina.
    if !alavanca_liberada_pela_ordem("reds") {
        return write_bool_ret(out, false);
    }
    if FIRE_START_DONE.swap(true, Ordering::Relaxed) {
        crate::log_dev!("[firestart] guardas barraram: {l:?} done=(1x)");
        return write_bool_ret(out, false);
    }
    let ctx = l.ctx;
    // O WRITE (v2): d4=2 → o state-2 pump 0x1015e61b4 dispara o reload (0x1031fef40) — o pedido assíncrono
    // que dirige e0 e emite 610/800. O reload dispara ANTES do uso de [SM+0xb8], então payload nulo não bloqueia.
    let b8 = (sm.add(0xb8) as *const u64).read();
    (sm.add(0xd4) as *mut u8).write(2);
    let _ = ctx;
    // Arma o dead-man's switch (ver tramp_fire_start_state): o fim do load do save (3ª phase 5) e o exit() hookado
    // limpam este marcador (inicio::boot::clear_boot_attempt_marker) — se o processo não chegar lá (crash/hang/kill -9),
    // o próximo boot detecta e pula o lever uma vez, em vez de crash-loopar pra sempre.
    if let Some(m) = boot_attempt_marker() {
        let _ = std::fs::write(&m, b"");
    }
    crate::log_dev!("[firestart] ESCREVI d4=2 (SM={sm:p}) [SM+0xb8]={b8:#x} — pump deve chamar 0x1031fef40 (reload)");
    veredito_da_alavanca("reds");
    write_bool_ret(out, true);
}

/// `BwmsGetLaunchArg(nome) -> String` (RED4ext `#458`): valor do argumento de lançamento, vazio se não veio ou se veio sem valor.
pub(crate) unsafe extern "C" fn tramp_launch_arg(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let nome = strings.first().cloned().flatten().unwrap_or_default();
    let v = crate::configuracao::argumentos::valor(&crate::configuracao::argumentos::do_processo(), &nome).unwrap_or_default();
    write_string_ret(out, &v);
}

/// `BwmsHasLaunchArg(nome) -> Bool`: o argumento veio na linha de comando (com ou sem valor).
pub(crate) unsafe extern "C" fn tramp_has_launch_arg(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let nome = strings.first().cloned().flatten().unwrap_or_default();
    write_bool_ret(out, crate::configuracao::argumentos::valor(&crate::configuracao::argumentos::do_processo(), &nome).is_some());
}

/// `BwmsPollerTick(label: String, n: Int32) -> Void` — contador de ciclos EXATO por poller
/// (bisecção do crash `SystemsUpdater::Node::LinkJob_NoFence`, 2026-07-15): loga a cada 20
/// chamadas pra correlacionar o crash por CICLO em vez de só tempo de parede — a última linha
/// gravada em `/tmp/cp77-console.log` antes de um crash futuro dá o nº exato de ciclos que
/// aquele poller alcançou.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_poller_tick(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let label = strings.first().cloned().flatten().unwrap_or_default();
    let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    crate::log_dev!("[poller-tick] {label} n={n}");
}

/// Codeware `#153`: `BwmsSaveCarregado(nome: String)` — o menu de load informa qual save entra, e
/// o BWMS usa o nome pra achar o arquivo de specs (que vive fora do save).
pub(crate) unsafe extern "C" fn tramp_save_carregado(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    crate::salvamento::entidades_persistentes::informar_save(&s);
    write_bool_ret(out, !s.is_empty());
}

/// `BwmsSessionGame() -> GameInstance`: o `GameInstance` do jogador da sessão (`PlayerPuppet.GetGame`), pro script que não tem de onde tirá-lo.
pub(crate) unsafe extern "C" fn tramp_session_game(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    let player = crate::current_player();
    let mut bytes = [0u8; 16];
    if let Some(b) = Registry::obtain()
        .filter(|_| !player.is_null())
        .and_then(|reg| crate::jogo::sessao::game_instance(&reg, player))
    {
        let n = b.len().min(16);
        bytes[..n].copy_from_slice(&b[..n]);
    }
    write_raw16_ret(out, &bytes);
}


/// Path do marcador "dead-man's switch" do lever (escrito por `tramp_fire_start`/`fire_lever_direct`;
/// lido por `check_stale_boot_attempt` e apagado por `clear_boot_attempt_marker`, em inicio/boot.rs).
/// Só HOME (não precisa do fallback /tmp dos outros toggles — é interno, nunca setado manualmente
/// pelo usuário).
pub(crate) fn boot_attempt_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-boot-attempt"))
}

/// Deref da cadeia do input-context (workflow 2): a=*(SM+0x70); b=*(a+0x288); c=*(b+0x18); ctx=*(c+0x1b0).
/// O byte gate da ação 'Start' vive em ctx+0x572. Cada nível valida legibilidade; null em qualquer passo → null.
/// O que a alavanca da engagement lê do `GameSessionDesc` para decidir se pode escrever `d4=2`.
#[derive(Debug)]
pub(crate) struct LeituraDaEngagement {
    /// Estado interno da sessão (`+0xd4`).
    estado: u8,
    fase: Option<crate::inicio::boot::FaseDaSessao>,
    /// `+0xe8`: não nulo, outra coisa já está mexendo na sessão.
    guarda: u8,
    ctx: *mut u8,
    /// `ctx+0x572`, ou -1 com o contexto ilegível.
    gate572: i32,
}

/// Estado interno `+0xd4` da tela "APERTE ESPAÇO" parada esperando a tecla.
const ESTADO_EM_REPOUSO: u8 = 1;

impl LeituraDaEngagement {
    /// `None` com o SM nulo ou ilegível.
    unsafe fn ler(sm: *mut u8) -> Option<Self> {
        if sm.is_null() || !crate::memoria::is_readable(sm as *const c_void, 0x108) {
            return None;
        }
        let ctx = deref_input_ctx(sm);
        let gate572 = if !ctx.is_null() && crate::memoria::is_readable(ctx as *const c_void, 0x573) {
            (ctx.add(0x572) as *const u8).read() as i32
        } else {
            -1
        };
        Some(LeituraDaEngagement {
            estado: (sm.add(0xd4) as *const u8).read(),
            fase: crate::inicio::boot::FaseDaSessao::do_byte((sm.add(0x84) as *const i8).read() as i64),
            guarda: (sm.add(0xe8) as *const u8).read(),
            ctx,
            gate572,
        })
    }

    /// A tela "APERTE ESPAÇO" está em repouso na fase da engagement, ninguém mais mexe na sessão e o contexto de input é legível: a alavanca
    /// pode escrever.
    fn alavanca_pode_disparar(&self) -> bool {
        self.estado == ESTADO_EM_REPOUSO
            && self.fase == Some(crate::inicio::boot::FaseDaSessao::Engagement)
            && self.guarda == 0
            && !self.ctx.is_null()
            && self.gate572 >= 0
    }
}

#[cfg(test)]
mod leitura_da_engagement_teste {
    use super::{LeituraDaEngagement, ESTADO_EM_REPOUSO};
    use crate::inicio::boot::FaseDaSessao;

    fn em_repouso() -> LeituraDaEngagement {
        LeituraDaEngagement {
            estado: ESTADO_EM_REPOUSO,
            fase: Some(FaseDaSessao::Engagement),
            guarda: 0,
            ctx: 8 as *mut u8,
            gate572: 0,
        }
    }

    #[test]
    fn alavanca_so_no_repouso_da_engagement_sem_guarda_e_com_contexto() {
        assert!(em_repouso().alavanca_pode_disparar());
        let barradas = [
            LeituraDaEngagement { estado: 2, ..em_repouso() },
            LeituraDaEngagement { fase: Some(FaseDaSessao::MenuDoPregame), ..em_repouso() },
            LeituraDaEngagement { fase: None, ..em_repouso() },
            LeituraDaEngagement { guarda: 1, ..em_repouso() },
            LeituraDaEngagement { ctx: std::ptr::null_mut(), ..em_repouso() },
            LeituraDaEngagement { gate572: -1, ..em_repouso() },
        ];
        for l in barradas {
            assert!(!l.alavanca_pode_disparar(), "{l:?}");
        }
    }
}

pub(crate) unsafe fn deref_input_ctx(sm: *const u8) -> *mut u8 {
    let rd = |p: *const u8| -> *const u8 {
        if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 8) {
            std::ptr::null()
        } else {
            (p as *const *const u8).read()
        }
    };
    let a = rd(sm.add(0x70));
    if a.is_null() { return std::ptr::null_mut(); }
    let b = rd(a.add(0x288));
    if b.is_null() { return std::ptr::null_mut(); }
    let c = rd(b.add(0x18));
    if c.is_null() { return std::ptr::null_mut(); }
    rd(c.add(0x1b0)) as *mut u8
}

/// LEVER: escreve `SM+0xd4=2` (d4 1→2) → o pump do state 2 (0x1015e61b4) dispara o reload assíncrono
/// (0x1031fef40) → gameplay, sem HID e sem forjar ActionEvent. `ctx+0x572` (gate da ação 'Start') só é
/// LIDO, como guarda. GATE `pular_intro` (ligado por padrão) + 1x + guardas (d4==1, phase==1, e8==0).
pub(crate) static FIRE_START_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Versão CHAMÁVEL do lever (não-native): o dylib dispara DIRETO quando o getter (inicio::boot::
/// phase_skip_getter) vê a engagement em repouso, sem depender do timer redscript OnBwmsFireStart
/// (que pós-reboot não armava — o @wrapMethod OnInitialize não disparava, eng=false). Mesmo gate +
/// write + dead-man's switch do tramp_fire_start. Compartilha FIRE_START_DONE (dispara 1x por qualquer
/// caminho). Retorna true SÓ se ESCREVEU d4=2.
pub(crate) unsafe fn fire_lever_direct() -> bool {
    disparar_alavanca_direta("direto")
}

/// Corpo do [`fire_lever_direct`]; `origem` vai para o veredito (`direto` = getter da phase, `tick` = alavanca adiada solta pelo `ordem_tick`).
unsafe fn disparar_alavanca_direta(origem: &'static str) -> bool {
    use std::sync::atomic::Ordering;
    let on = crate::configuracao::do_mod::config().pular_intro;
    if !on {
        return false;
    }
    let sm = crate::inicio::boot::GAME_SESSION_DESC.load(Ordering::Relaxed);
    let Some(l) = LeituraDaEngagement::ler(sm) else { return false };
    if !l.alavanca_pode_disparar() || FIRE_START_DONE.load(Ordering::Relaxed) {
        return false;
    }
    if !alavanca_liberada_pela_ordem(origem) || FIRE_START_DONE.swap(true, Ordering::Relaxed) {
        return false;
    }
    let gate_now = l.gate572;
    let b8 = (sm.add(0xb8) as *const u64).read();
    (sm.add(0xd4) as *mut u8).write(2);
    if let Some(m) = boot_attempt_marker() {
        let _ = std::fs::write(&m, b"");
    }
    crate::log_dev!(
        "[firestart-direto] ESCREVI d4=2 (SM={sm:p}) [SM+0xb8]={b8:#x} — sem timer redscript, gate572={gate_now}, origem={origem}"
    );
    veredito_da_alavanca(origem);
    true
}

// ===== Ordem no menu: `OnApply` dos `ScriptableTweak` × alavanca d4=2 =====
// A alavanca põe a sessão no estado 2 e o pump chama o `GameServicesAssignUser` (`0x1031fef40`): a inicialização do usuário corre por
// baixo. Com o `OnApply` do menu começando depois dela, a thread de script parou dentro do `CreateRecord` do motor em 2 de 3 boots
// (2026-10-25, z038/relatorios/VEREDITOS.md); com o `OnApply` antes, 0 de 5. O despacho do menu só começa com a engagement em repouso e
// sem alavanca; a alavanca espera o despacho terminar. Um sai antes do outro, nunca sobrepostos.

/// Quem veio primeiro no menu. Um byte só, trocado por CAS: o `cp77_tick` (despacho) e a alavanca (native do reds, getter da phase)
/// podem estar em threads diferentes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum OrdemNoMenu {
    /// Nem o `OnApply` do menu nem a alavanca.
    Aberta = 0,
    /// O despacho do `OnApply` começou no menu; a alavanca espera.
    OnApplyEmCurso = 1,
    /// O despacho do menu terminou; a alavanca está livre.
    OnApplyFeito = 2,
    /// A alavanca (ou o espaço do jogador) dispensou a engagement antes do despacho: o `OnApply` fica para o gameplay (a reserva de 30 s
    /// depois da última phase 5, o caminho de 2026-09-11 a 2026-10-23).
    AlavancaPrimeiro = 3,
}

impl OrdemNoMenu {
    fn do_byte(b: u8) -> Self {
        match b {
            1 => OrdemNoMenu::OnApplyEmCurso,
            2 => OrdemNoMenu::OnApplyFeito,
            3 => OrdemNoMenu::AlavancaPrimeiro,
            _ => OrdemNoMenu::Aberta,
        }
    }
}

static ORDEM_NO_MENU: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(OrdemNoMenu::Aberta as u8);

pub(crate) fn ordem_no_menu() -> OrdemNoMenu {
    OrdemNoMenu::do_byte(ORDEM_NO_MENU.load(std::sync::atomic::Ordering::Acquire))
}

fn trocar_ordem(de: OrdemNoMenu, para: OrdemNoMenu) -> bool {
    use std::sync::atomic::Ordering;
    ORDEM_NO_MENU.compare_exchange(de as u8, para as u8, Ordering::AcqRel, Ordering::Acquire).is_ok()
}

/// Teto da espera da alavanca pelo `OnApply` do menu, contado do 1º pedido dela. Nos 8 boots de 2026-10-25 o despacho ficou pronto até
/// 3 s depois do 1º pedido (alavanca t=93–95 s, `OnApply` t=95–97 s); passado o teto, a alavanca dispara e o `OnApply` vai para o gameplay,
/// para um despacho que nunca fica pronto não segurar o boot na engagement.
const ALAVANCA_ESPERA_MAX_S: u64 = 20;

/// 1º pedido da alavanca que passou pelas guardas da engagement.
static ALAVANCA_PEDIDA_EM: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Alavanca adiada esperando o `ordem_tick` soltar.
static ALAVANCA_PENDENTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A alavanca foi adiada alguma vez neste boot (sai no veredito).
static ALAVANCA_FOI_ADIADA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `boot_elapsed_secs` do começo do despacho do menu (sai no veredito).
static ONAPPLY_MENU_COMECOU_S: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DecisaoDaAlavanca {
    Dispara,
    Adia,
    /// Teto vencido sem despacho: dispara e fecha o menu para o `OnApply`.
    DisparaECancelaOnApply,
}

/// O que a alavanca faz com a ordem `o`, pedida há `pedida_ha_s`.
fn decidir_alavanca(o: OrdemNoMenu, pedida_ha_s: u64) -> DecisaoDaAlavanca {
    match o {
        OrdemNoMenu::OnApplyFeito | OrdemNoMenu::AlavancaPrimeiro => DecisaoDaAlavanca::Dispara,
        // O despacho é síncrono e curto (3 `OnApply` do Equipment-EX: ~4 s com o log de dev); disparar no meio dele é o caso que travou.
        OrdemNoMenu::OnApplyEmCurso => DecisaoDaAlavanca::Adia,
        OrdemNoMenu::Aberta if pedida_ha_s < ALAVANCA_ESPERA_MAX_S => DecisaoDaAlavanca::Adia,
        OrdemNoMenu::Aberta => DecisaoDaAlavanca::DisparaECancelaOnApply,
    }
}

/// Chamado pela alavanca depois das guardas da engagement e antes de escrever d4=2: `false` = adiada (o `ordem_tick` solta).
fn alavanca_liberada_pela_ordem(origem: &str) -> bool {
    use std::sync::atomic::Ordering;
    let pedida_ha_s = ALAVANCA_PEDIDA_EM.get_or_init(std::time::Instant::now).elapsed().as_secs();
    let liberada = match decidir_alavanca(ordem_no_menu(), pedida_ha_s) {
        DecisaoDaAlavanca::Dispara => true,
        DecisaoDaAlavanca::Adia => false,
        // O CAS perde só para o despacho que acabou de começar: então espera ele.
        DecisaoDaAlavanca::DisparaECancelaOnApply => trocar_ordem(OrdemNoMenu::Aberta, OrdemNoMenu::AlavancaPrimeiro),
    };
    if !liberada && !ALAVANCA_PENDENTE.swap(true, Ordering::AcqRel) {
        ALAVANCA_FOI_ADIADA.store(true, Ordering::Relaxed);
        crate::log_dev!(
            "[ordem] alavanca ({origem}) adiada em t={}s: OnApply do menu {:?}; solta quando ele terminar ou em {ALAVANCA_ESPERA_MAX_S}s",
            crate::inicio::boot::boot_elapsed_secs(),
            ordem_no_menu()
        );
    }
    liberada
}

/// Solta a alavanca adiada: com o `OnApply` do menu terminado, ou com o teto vencido. Barato quando não há alavanca adiada (2 leituras
/// atômicas); chamado pelo `cp77_tick` depois do bloco do `OnApply`.
pub(crate) unsafe fn ordem_tick() {
    use std::sync::atomic::Ordering;
    if !ALAVANCA_PENDENTE.load(Ordering::Acquire) || FIRE_START_DONE.load(Ordering::Relaxed) {
        return;
    }
    let pedida_ha_s = ALAVANCA_PEDIDA_EM.get().map_or(0, |t| t.elapsed().as_secs());
    if decidir_alavanca(ordem_no_menu(), pedida_ha_s) == DecisaoDaAlavanca::Adia {
        return;
    }
    // Uma tentativa: com a engagement fora do repouso as guardas barram, e ler o SM a cada tick seria uma syscall por tick.
    ALAVANCA_PENDENTE.store(false, Ordering::Release);
    if !disparar_alavanca_direta("tick") {
        crate::veredito_dev!(
            "[ordem] VEREDITO-ORDEM-ALAVANCA alavanca adiada não disparou em t={}s: guardas da engagement barraram (OnApply do menu {:?}) = 0",
            crate::inicio::boot::boot_elapsed_secs(),
            ordem_no_menu()
        );
    }
}

/// A engagement já saiu do repouso: alavanca escrita, ou o jogador apertou espaço (`pular_intro` desligado). Antes de o getter travar no
/// SM da engagement, `GAME_SESSION_DESC` pode ser um caller espúrio (phase 3): aí ainda não há engagement a dispensar.
unsafe fn engagement_dispensada() -> bool {
    use std::sync::atomic::Ordering;
    if FIRE_START_DONE.load(Ordering::Relaxed) {
        return true;
    }
    if !crate::inicio::boot::sm_da_engagement_capturado() {
        return false;
    }
    match LeituraDaEngagement::ler(crate::inicio::boot::GAME_SESSION_DESC.load(Ordering::Relaxed)) {
        // SM ilegível depois de capturado = sessão desmontando: já não é o repouso.
        None => true,
        Some(l) => fora_do_repouso(l.estado, l.fase),
    }
}

/// O repouso da tela "APERTE ESPAÇO" é a fase da engagement com `+0xd4` em 1, ou 4 quando o check de sessão passa (o mesmo critério da
/// captura do SM no getter).
fn fora_do_repouso(estado: u8, fase: Option<crate::inicio::boot::FaseDaSessao>) -> bool {
    fase != Some(crate::inicio::boot::FaseDaSessao::Engagement) || !(estado == ESTADO_EM_REPOUSO || estado == 4)
}

/// O `cp77_tick` pede para começar o despacho do `OnApply` no menu. `false` = não começa no menu (a alavanca veio antes, ou a engagement
/// já foi dispensada): ele fica para o gameplay.
pub(crate) unsafe fn onapply_no_menu_comecar() -> bool {
    if ordem_no_menu() != OrdemNoMenu::Aberta {
        return false;
    }
    // Teste da ordem (dev, chave `onapply_menu_atraso_s` do `~/.bwms-modconfig.txt`): o despacho do menu espera a alavanca ter sido
    // pedida há N s, para todo boot passar pelo caminho da alavanca adiada. N abaixo do `ALAVANCA_ESPERA_MAX_S`.
    #[cfg(feature = "devtools")]
    {
        static ATRASO_S: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let atraso = *ATRASO_S.get_or_init(|| crate::configuracao::modconfig_get("onapply_menu_atraso_s").trim().parse().unwrap_or(0));
        if atraso > 0 && ALAVANCA_PEDIDA_EM.get().map_or(true, |t| t.elapsed().as_secs() < atraso) {
            return false;
        }
    }
    if engagement_dispensada() {
        if trocar_ordem(OrdemNoMenu::Aberta, OrdemNoMenu::AlavancaPrimeiro) {
            crate::veredito_dev!(
                "[ordem] VEREDITO-ORDEM-ONAPPLY OnApply do menu cancelado em t={}s: engagement já dispensada (alavanca={}); vai para o gameplay = 1",
                crate::inicio::boot::boot_elapsed_secs(),
                FIRE_START_DONE.load(std::sync::atomic::Ordering::Relaxed)
            );
        }
        return false;
    }
    if !trocar_ordem(OrdemNoMenu::Aberta, OrdemNoMenu::OnApplyEmCurso) {
        return false;
    }
    ONAPPLY_MENU_COMECOU_S.store(crate::inicio::boot::boot_elapsed_secs(), std::sync::atomic::Ordering::Relaxed);
    true
}

/// O despacho do menu terminou: a alavanca adiada sai no próximo `ordem_tick`.
pub(crate) fn onapply_no_menu_terminou() {
    use std::sync::atomic::Ordering;
    let _ = trocar_ordem(OrdemNoMenu::OnApplyEmCurso, OrdemNoMenu::OnApplyFeito);
    crate::veredito_dev!(
        "[ordem] VEREDITO-ORDEM-ONAPPLY OnApply do menu t={}→{}s, antes da alavanca (alavanca escrita={}, adiada esperando={}) = {}",
        ONAPPLY_MENU_COMECOU_S.load(Ordering::Relaxed),
        crate::inicio::boot::boot_elapsed_secs(),
        FIRE_START_DONE.load(Ordering::Relaxed),
        ALAVANCA_PENDENTE.load(Ordering::Relaxed),
        !FIRE_START_DONE.load(Ordering::Relaxed) as u8
    );
}

/// Uma linha por boot, na escrita de d4=2: a ordem que o roteiro conta em N boots. `= 1` com o `OnApply` do menu fora da janela da
/// alavanca (terminado antes, ou mandado para o gameplay).
fn veredito_da_alavanca(origem: &str) {
    use std::sync::atomic::Ordering;
    let o = ordem_no_menu();
    let onapply = match o {
        OrdemNoMenu::OnApplyFeito => "menu",
        OrdemNoMenu::AlavancaPrimeiro => "gameplay",
        OrdemNoMenu::OnApplyEmCurso | OrdemNoMenu::Aberta => "sobreposto",
    };
    let adiada_ms = if ALAVANCA_FOI_ADIADA.load(Ordering::Relaxed) {
        ALAVANCA_PEDIDA_EM.get().map_or(0, |t| t.elapsed().as_millis())
    } else {
        0
    };
    crate::veredito_dev!(
        "[ordem] VEREDITO-ORDEM-ALAVANCA d4=2 por {origem} em t={}s; onapply={onapply}; adiada={adiada_ms}ms; ordem {o:?} = {}",
        crate::inicio::boot::boot_elapsed_secs(),
        matches!(o, OrdemNoMenu::OnApplyFeito | OrdemNoMenu::AlavancaPrimeiro) as u8
    );
}

#[cfg(test)]
mod ordem_no_menu_teste {
    use super::{decidir_alavanca, fora_do_repouso, DecisaoDaAlavanca, OrdemNoMenu, ALAVANCA_ESPERA_MAX_S};
    use crate::inicio::boot::FaseDaSessao;

    #[test]
    fn alavanca_espera_o_onapply_do_menu_e_nunca_dispara_no_meio_dele() {
        assert_eq!(decidir_alavanca(OrdemNoMenu::OnApplyEmCurso, 0), DecisaoDaAlavanca::Adia);
        assert_eq!(decidir_alavanca(OrdemNoMenu::OnApplyEmCurso, 10 * ALAVANCA_ESPERA_MAX_S), DecisaoDaAlavanca::Adia);
        assert_eq!(decidir_alavanca(OrdemNoMenu::OnApplyFeito, 0), DecisaoDaAlavanca::Dispara);
        assert_eq!(decidir_alavanca(OrdemNoMenu::AlavancaPrimeiro, 0), DecisaoDaAlavanca::Dispara);
    }

    #[test]
    fn sem_despacho_a_alavanca_espera_ate_o_teto_e_depois_manda_o_onapply_para_o_gameplay() {
        assert_eq!(decidir_alavanca(OrdemNoMenu::Aberta, 0), DecisaoDaAlavanca::Adia);
        assert_eq!(decidir_alavanca(OrdemNoMenu::Aberta, ALAVANCA_ESPERA_MAX_S - 1), DecisaoDaAlavanca::Adia);
        assert_eq!(decidir_alavanca(OrdemNoMenu::Aberta, ALAVANCA_ESPERA_MAX_S), DecisaoDaAlavanca::DisparaECancelaOnApply);
    }

    #[test]
    fn ordem_volta_do_byte_e_byte_desconhecido_e_aberta() {
        for o in [OrdemNoMenu::Aberta, OrdemNoMenu::OnApplyEmCurso, OrdemNoMenu::OnApplyFeito, OrdemNoMenu::AlavancaPrimeiro] {
            assert_eq!(OrdemNoMenu::do_byte(o as u8), o);
        }
        assert_eq!(OrdemNoMenu::do_byte(0xff), OrdemNoMenu::Aberta);
    }

    #[test]
    fn repouso_da_engagement_e_d4_1_ou_4_na_fase_1() {
        assert!(!fora_do_repouso(1, Some(FaseDaSessao::Engagement)));
        assert!(!fora_do_repouso(4, Some(FaseDaSessao::Engagement)));
        assert!(fora_do_repouso(2, Some(FaseDaSessao::Engagement)));
        assert!(fora_do_repouso(5, Some(FaseDaSessao::IniciandoUsuario)));
        assert!(fora_do_repouso(1, Some(FaseDaSessao::MenuDoPregame)));
        assert!(fora_do_repouso(1, None));
    }
}

/// `GameSessionEvent extends CallbackSystemEvent` — `cw-controller-session` (2026-07-18, sessão
/// `handle-ctor-re`, 6ª rodada). Fonte real: `public native class GameSessionEvent extends
/// CallbackSystemEvent { public native func IsRestored() -> Bool; public native func IsPreGame()
/// -> Bool }` (`GameSessionEvent.reds`) — dados reais (`GameSessionEvent.hpp` vendorizado):
/// `bool preGame; bool restored;`. MESMA receita 100% segura já provada em `KeyInputEvent`
/// (forge robusto + parent `CallbackSystemEvent` real). Estado por-instância no MESMO padrão de
/// `KEYINPUT_STATES` (`GAMESESSION_STATES: Vec<(ptr,pregame,restored)>`).
pub(crate) static GAMESESSION_STATES: Mutex<Vec<(usize, bool, bool)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_gse_is_restored(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = GAMESESSION_STATES.lock() {
        if let Some((_, _, r)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *r;
        }
    }
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_gse_is_pregame(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = GAMESESSION_STATES.lock() {
        if let Some((_, pg, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *pg;
        }
    }
    write_bool_ret(out, v);
}

pub unsafe fn register_gamesessionevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_gamesessionevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "GameSessionEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_gamesessionevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_gamesessionevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "GameSessionEvent", proto, "IsRestored", "IsRestored", tramp_gse_is_restored, false);
    let m2 = register_method(reg, "GameSessionEvent", proto, "IsPreGame", "IsPreGame", tramp_gse_is_pregame, false);
    format!("[reg] GameSessionEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) IsRestored={m1} IsPreGame={m2}")
}

/// Constrói um `GameSessionEvent` REAL (mesma técnica de `make_keyinputevent_arg`) e devolve um
/// `Arg::Handle` pronto pra `fire_event_args`. `restored`/`pregame` refletem o contexto REAL do
/// disparo — ver `lib.rs::cp77_tick`, bloco de transição de presença do player (`restored=true`
/// pq o auto-continue SEMPRE carrega um save; `pregame=false` pq o player só fica presente DEPOIS
/// da criação de personagem). `None` se a construção falhar — o caller descarta, não crasha.
pub unsafe fn make_gamesessionevent_arg(pregame: bool, restored: bool) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "GameSessionEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = GAMESESSION_STATES.lock() {
        states.push((obj as usize, pregame, restored));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// Dispara um evento de sessão com um `GameSessionEvent` novo; devolve quantos ouvintes receberam. Depois do disparo o estado sai do registro e o
/// handle do evento é solto (o `call_func` só copia o par; o +1 é nosso), como em `entidades::disparar_evento_de_entidade`.
pub(crate) unsafe fn disparar_evento_de_sessao(evento: &str, pregame: bool, restored: bool) -> usize {
    let Some(arg) = make_gamesessionevent_arg(pregame, restored) else {
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args(evento, &[arg]);
    if let Ok(mut states) = GAMESESSION_STATES.lock() {
        states.retain(|(p, _, _)| *p as u64 != inst);
    }
    let mut h = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut h);
    enviados
}

/// `IsPreGame` (bit 0) e `IsRestored` (bit 1) da sessão do último spawn: o `Session/End` dela, quando sai no spawn seguinte, leva os dela.
static SESSAO_DO_ULTIMO_SPAWN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

pub(crate) unsafe fn session_ready_tick() {
    let pendentes: Vec<(bool, bool)> = match crate::inicio::boot::SPAWNS_DO_PLAYER_PENDENTES.lock() {
        Ok(mut p) if !p.is_empty() => std::mem::take(&mut *p),
        _ => return,
    };
    let Some(reg) = Registry::obtain() else { return };
    let f = get_function(&reg, "BwmsSessaoEstado");
    let estado = if rtti::sane(f) {
        let rf = rtti::ResolvedFn { func: f, ret_type: rtti::ret_type_of(f), is_static: true };
        rtti::call_func(&rf, std::ptr::null_mut(), &[]).map_or(-1, |b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    } else {
        -1
    };
    if estado < 0 {
        crate::veredito_dev!("[c129] {} spawn(s) do jogador sem BwmsSessaoEstado no bundle — Session/Ready não despachado", pendentes.len());
        return;
    }
    let pregame = estado & 1 != 0;
    use crate::inicio::sessao_do_jogo as sessao;
    for (restored, fim_pendente) in pendentes {
        // O mundo anterior saiu sem `Session/End` (sem `OnWorldDetached` do jogo nem despawn visto no tick): o dele vem antes do `Ready` deste,
        // como o `OnWorldDetached` da referência entre o mundo do menu e o da carga de save.
        if fim_pendente {
            let anterior = SESSAO_DO_ULTIMO_SPAWN.load(std::sync::atomic::Ordering::Relaxed);
            let (pg, rs) = (anterior & 1 != 0, anterior & 2 != 0);
            let n = sessao::repassar("Session/End", pg, rs);
            crate::veredito_dev!("[sessao-reserva] Session/End (spawn do jogador em outro mundo) pregame={pg} restored={rs} → {n} callback(s)");
        }
        SESSAO_DO_ULTIMO_SPAWN.store(u8::from(pregame) | u8::from(restored) << 1, std::sync::atomic::Ordering::Relaxed);
        // Uma fonte por sessão: o `Session/Ready` que o jogo (`OnGameRestored`) ou a reserva já entregou nesta sessão não sai de novo.
        let unica = !restored && sessao::reivindicar(sessao::SESSAO_PRONTA);
        if !restored && !unica {
            crate::veredito_dev!("[c129] spawn do jogador: pregame={pregame} restored=false → Session/Ready já entregue nesta sessão por outra fonte");
        }
        let k = if unica { disparar_evento_de_sessao("Session/Ready", pregame, restored) } else { 0 };
        if let Ok(mut d) = C129_DESPACHOS.lock() {
            d.push((pregame, restored, k));
        }
        crate::veredito_dev!("[c129] spawn do jogador: pregame={pregame} restored={restored} → Session/Ready com {k} callback(s)");
        // Carga de save: o C#129 não despacha (a fonte é o `OnGameRestored`, que com o despachante mudo não chega). A reserva entrega aqui,
        // fora do `C129_DESPACHOS`: o `cp77_tick` só tem a dele antes da 1ª presença do jogador (`in_crash_window`), que é no mundo do menu.
        if restored && sessao::pela_reserva(sessao::SESSAO_PRONTA, "Session/Ready") {
            let n = disparar_evento_de_sessao("Session/Ready", pregame, true);
            crate::veredito_dev!("[sessao-reserva] Session/Ready (carga de save, spawn do jogador) pregame={pregame} restored=true → {n} callback(s)");
        }

    }
}

