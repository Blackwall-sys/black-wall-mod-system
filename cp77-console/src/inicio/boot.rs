//! Self-boot. Quando a dylib é carregada DIRETO pelo jogo (LC_LOAD_DYLIB "baked"),
//! um construtor instala o hook do executor (via `ganchos`, Rust puro) e dirige o runtime
//! sozinho. Na trilha de DEV, se outro injetor externo já dirige, fica PASSIVO p/
//!
//! Ganho de desempenho: a captura de player/tx vira comparação de classe em atomics
//! — **sem I/O de `/tmp` por chamada** — e o tick pesado roda ~1x a cada
//! [`TICK_EVERY`] chamadas, **sem FFI entre linguagens na via mais quente do jogo**.

use crate::mods_do_usuario::api::EstadoDoJogo;
use crate::diagnostico::C129_CARGAS;
use crate::diagnostico::C129_CHAMADAS;
#[cfg(feature = "devtools")]
use crate::diagnostico::{c239_job_sessao_hook, C239_ESTADOS, C239_JOBS, C239_JOB_SESSAO_VM, C239_RELOGIO, C239_SLOT_FRAMEWORK, ORIG_C239_JOB_SESSAO};
use crate::register::ler_u64;
use crate::ganchos::original_publicado;
use crate::ganchos::instalar_conferindo_prologo;
use crate::marcador_individual;
use crate::jogo::scripts::EXECUTOR_PROLOGO;
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use crate::rtti;

/// Roda o tick pesado a cada N chamadas do executor (a captura roda sempre, é barata).
const TICK_EVERY: u64 = 2048;

static ACTIVE: AtomicBool = AtomicBool::new(false);

// ===== SPLASH DE BOOT: preenche a tela preta do loading com feedback/branding =====
// Arma no on_load com `pular_intro`, que também liga o lever. Desliga (`boot_splash_off`) quando a tarefa do boot termina: menu pronto
// sem load a fazer (`BwmsMenuReadySplashOff`, `inicio/natives.rs`) ou fim do load do autocontinue (`splash_fora_quando_o_executor_assenta`).
// Fora disso, só pelos resgates: `boot_hang_watchdog` (tela que pede input travada; teto depois do 1º phase 5) e
// `install_phase_skip_near4` sem getter (sem lever a engagement espera a tecla).
static BOOT_SPLASH_ON: AtomicBool = AtomicBool::new(false);
static BOOT_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
static AUTOCONTINUE_FIRED: AtomicBool = AtomicBool::new(false);
/// O menu do boot já resolveu a tarefa dele neste processo: o autocontinue disparou (`note_autocontinue_fired`) ou o menu ficou pronto
/// sem load a fazer (`BwmsMenuReadySplashOff`). Daí em diante o load automático não roda (`menu_do_boot_resolvido`): sairia sem splash,
/// que só arma no boot (`boot_splash_arm`, chamado só por `selfboot_if_needed`).
static MENU_DO_BOOT_RESOLVIDO: AtomicBool = AtomicBool::new(false);
/// Dead-man's switch do lever `BwmsFireStart` (ver inicio/natives.rs::tramp_fire_start/tramp_autocontinue):
/// true se `~/.bwms-boot-attempt` já existia ANTES deste processo escrever qualquer coisa nele —
/// ou seja, sobrou de um boot anterior que disparou o lever mas NUNCA chegou no exit() limpo
/// (crash, hang morto a kill -9, watchdog do próprio jogo). Setado 1x em `check_stale_boot_attempt`,
/// chamado do topo do `on_load` — antes de QUALQUER hook desta sessão poder disparar o lever de
/// novo, então "o arquivo existe" só pode significar "sobrou de antes", nunca "esta sessão escreveu".
static STALE_BOOT_ATTEMPT: AtomicBool = AtomicBool::new(false);
pub(crate) static PHASE_REACHED_5: AtomicBool = AtomicBool::new(false);
/// Sinal MODO-INDEPENDENTE de "gameplay/save-load alcançado", setado na transição de player-presente
/// no `cp77_tick` (roda em TODO modo, inclusive modo 0 sem skip). `PHASE_REACHED_5` só é setado
/// dentro do getter de skip — que NÃO instala no modo 0 → a rede anti-crash do redDispatcher ficava
/// INERTE exatamente no modo 0. O `in_crash_window` faz OR com isto pra a supressão valer em qualquer
/// modo. Latcha (nunca reseta) — mais supressão = mais seguro. Ver `in_crash_window` em lib.rs.
pub(crate) static POST_SAVELOAD: AtomicBool = AtomicBool::new(false);
/// Travou no SM da EngagementScreenGameController (o objeto do lever d4=2)? O getter da phase-byte é
/// GENÉRICO — vários objetos o chamam. Sem isto, a captura pega o ÚLTIMO caller (um espúrio phase=3) e
/// o lever escreve no lugar errado. Trava assim que vê o repouso da engagement (d4==1 && phase==1). BUG
/// achado 2026-07-15 (pós-reboot a captura passou a pegar espúrios → boot-até-gameplay parou).
static ENGAGEMENT_SM_LOCKED: AtomicBool = AtomicBool::new(false);
/// `GAME_SESSION_DESC` já é o SM da engagement (não um caller espúrio do getter). Lido pela ordem `OnApply` × alavanca
/// (`natives::engagement_dispensada`).
pub(crate) fn sm_da_engagement_capturado() -> bool {
    ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed)
}
/// O lever d4=2 já foi disparado DIRETO pelo dylib (sem o timer redscript, que pós-reboot não armava)?
static LEVER_FIRED_DIRECT: AtomicBool = AtomicBool::new(false);
/// Instante em que o getter viu a engagement em repouso (d4==1,phase==1) pela 1ª vez desde o lock.
/// O disparo do lever espera um DWELL de wall-clock a partir daqui (independente de framerate) —
/// ver o bloco de disparo direto no getter.
static LEVER_REST_SINCE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
pub(crate) static PHASE5_AT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// Última vez que o getter da phase devolveu 5. O getter só roda nas transições de sessão (14 chamadas até a gameplay no boot 41b):
/// a 1ª leitura de 5 é a do autocontinue, com o load só enfileirado (`PHASE5_AT`); a última é o objeto de sessão novo, no fim do load.
pub(crate) static PHASE5_ULTIMO_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
/// Leituras de phase 5 pelo getter no processo. Cada load tem 5: a 1ª com o load só enfileirado (`PHASE5_AT`; tirar o splash nela mostra
/// a tela de loading nativa, HISTORICO.md 2026-07-12), a 2ª 1–13 s depois, e a 3ª, a 4ª e a 5ª em sequência 12–30 s depois da 1ª; o
/// getter só volta a ser chamado na troca de sessão seguinte (261 de 261 sessões completas em `proofs/`, contadas em 2026-10-01). Prazo
/// fixo depois da 1ª não serve: em boot41b e boot225 a 2ª leitura vem 11–13 s depois da 1ª e o fim do load, ~25 s depois.
static LEITURAS_PHASE5: AtomicU64 = AtomicU64::new(0);
/// Instante da 3ª leitura de phase 5 (fim do load). A saída do splash do autocontinue conta daqui.
static FIM_DO_LOAD_AT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// O load do save terminou: a 3ª leitura de phase 5 ou, para load sem 3ª leitura, o teto do splash depois do 1º phase 5. Os gatilhos de mod
/// (o `OnApply` da reserva, journals), o `gameplay_real` e o desarme do dead-man's switch esperam por isto: prazo em segundos depois do 1º phase 5 cai
/// dentro do load quando ele estica (R28: OnApply com o 1º phase 5 há 32 s, fim do load em 34 s; `proofs/20261003-refino-r28-console-completo.log`).
pub(crate) fn carga_do_save_terminou() -> bool {
    FIM_DO_LOAD_AT.get().is_some() || PHASE5_AT.get().map_or(false, |t| t.elapsed().as_secs() >= SPLASH_TETO_APOS_PHASE5_SECS)
}
/// Ritmo do executor depois do fim do load, 1 amostra por segundo: segundo e `CALLS` da última amostra, maior taxa por segundo vista
/// e quantas amostras seguidas ficaram abaixo de 10% dela.
static ASSENTO_SEG: AtomicU64 = AtomicU64::new(0);
static ASSENTO_CALLS: AtomicU64 = AtomicU64::new(0);
static ASSENTO_PICO: AtomicU64 = AtomicU64::new(0);
static ASSENTO_BAIXAS: AtomicU64 = AtomicU64::new(0);
// WATCHDOG anti-hang de boot (2026-07-19). `BINK_RELEASED`: quando o watchdog (ou o branch de
// getter-recusado) desiste do skip, faz o `bink_open_replacement` parar de falhar os opens →
// libera o vídeo/engagement NATIVO em vez de manter a tela preta. `PHASE_MAX_SEEN`: a maior phase
// já observada pelo getter (boot sadio passa de 1 em segundos). Se após N segundos seguir <=1, o
// boot está travado (lever não disparou / fingerprint do save não bateu) → desiste com graça.
static BINK_RELEASED: AtomicBool = AtomicBool::new(false);
static PHASE_MAX_SEEN: AtomicI64 = AtomicI64::new(-1);
/// A última fase lida pelo getter (3 = menu principal, 5 = sessão de jogo): o getter só é lido nas transições, então fica com a do estado atual.
pub(crate) static FASE_LIDA: AtomicI64 = AtomicI64::new(-1);
/// Segundos-desde-o-boot em que `PHASE_MAX_SEEN` subiu pela última vez — o watchdog mede "empacado há
/// N s" (dwell) em vez de só tempo-desde-o-boot, pra resgatar o travamento no MENU de forma responsiva.
static PHASE_MAX_CHANGED_ELAPSED: AtomicU64 = AtomicU64::new(0);
/// Fase do SM da engagement (o objeto travado em `GAME_SESSION_DESC`) na última leitura: o dwell do watchdog conta da última
/// mudança dela. Os callers espúrios do getter leem phase=3 em t≈4–8 s, antes da engagement (`getter#0`), e sem isto o dwell do
/// menu vencia 0–2 s depois do lever, com o save já carregando (214 de 214 logs, HISTORICO 2026-10-01).
static FASE_DO_SM: AtomicI64 = AtomicI64::new(-1);
/// Teto do splash depois do 1º phase 5, só para load que não termina: 1º→último phase 5 medido em 12–29 s em 223 logs
/// (`[tweakxl] ... 1º phase 5 há N s`). Folga larga porque a regra do dono é o splash até o load terminar.
const SPLASH_TETO_APOS_PHASE5_SECS: u64 = 180;
const BOOT_HANG_WATCHDOG_SECS: u64 = 75; // travado NA engagement (phase<=1) — claramente quebrado
const BOOT_STUCK_LOADING_WATCHDOG_SECS: u64 = 120; // phase 2 (loading) — conservador (pode ser load lento)
// phase 3-4 (menu) empacado há tanto tempo SEM virar gameplay = autocontinue genuinamente não veio.
// GENEROSO (90s): o autocontinue do GOG é mais LENTO que o Steam (o metadata do save demora — provado
// 2026-07-19: GOG chega na gameplay ~35s após o menu). Um dwell curto resgataria cedo demais e
// revelaria a transição de um boot que IA funcionar. 90s = só o caso realmente travado dispara; o
// boot GOG normal (autocontinue lento-mas-ok) mantém o splash até a gameplay (splash-off normal cuida).
const BOOT_STUCK_MENU_DWELL_SECS: u64 = 90;

/// Watchdog anti-hang de boot: se o skip está ligado e a phase não chegou na GAMEPLAY (5) dentro do
/// prazo, DESISTE do skip com graça — tira o splash e libera o bink pro que estiver por trás (vídeo/
/// engagement/MENU nativo) renderizar. Cobre DOIS estados travados, com thresholds diferentes:
///  - **phase<=1** (travado na engagement — lever não disparou, getter recusou o patch, fingerprint
///    de rest-state/save não bateu): resgata a 75s (é claramente quebrado; boot sadio passa em segundos).
///  - **phase 2-4** (passou da engagement mas empacou sem gameplay): resgata a 120s, conservador pra
///    não tirar o splash de um boot lento-mas-progredindo. Caso REAL: **GOG** — o getter hooka tarde
///    (via adrp_br8, depois da engagement) e o lever perde a janela da phase 1 → o jogo para no MENU e
///    o splash (que só some no fim do load ou no menu pronto) fica cobrindo o menu pra sempre. Aqui
///    o watchdog tira o splash → o usuário VÊ o menu e clica "Continuar" (degrada pra o comportamento
///    do modo 1, em vez de tela travada). Ver `HISTORICO.md` (refino GOG 2026-07-19).
/// Com phase 5 já vista, é daqui que sai o splash do autocontinue: no fim do load (`splash_fora_quando_o_executor_assenta`) ou
/// no teto `SPLASH_TETO_APOS_PHASE5_SECS`.
/// O pior caso vira "o jogo bootou normal, você vê o menu/APERTE ESPAÇO", NUNCA splash/tela preta
/// eterna. Idempotente. Chamado do getter (só nas transições, ≤28 chamadas por boot em 334 logs de proofs/) E do cp77_tick, que é
/// quem segue chamando depois da 5ª leitura de phase 5.
pub(crate) fn boot_hang_watchdog() {
    if BINK_RELEASED.load(Ordering::Relaxed) {
        return; // já desistiu
    }
    if !skipintro_enabled() {
        return; // skip desligado → nada a vigiar
    }
    let start = match BOOT_START.get() {
        Some(t) => t,
        None => return, // âncora ainda não armada
    };
    // Por ordem do valor bruto: o getter é genérico e pode ter visto valor fora das fases, e o watchdog sempre tratou o que não passou
    // da engagement como engagement.
    let max = PHASE_MAX_SEEN.load(Ordering::Relaxed);
    let chegou_ao_jogo = max >= FaseDaSessao::Jogo as i64;
    let nao_passou_da_engagement = max <= FaseDaSessao::Engagement as i64;
    let iniciando_usuario = max == FaseDaSessao::IniciandoUsuario as i64;
    if chegou_ao_jogo {
        if boot_splash_active() {
            // Teto: load que não chega à 3ª leitura de phase 5, ou executor que não assenta, não segura o splash por cima do jogo.
            if PHASE5_AT.get().map_or(false, |t| t.elapsed().as_secs() >= SPLASH_TETO_APOS_PHASE5_SECS) {
                crate::veredito_dev!("[skipintro] WATCHDOG: load sem fim {SPLASH_TETO_APOS_PHASE5_SECS}s depois do 1º phase 5 -> tirando o splash");
                boot_splash_off();
            } else {
                splash_fora_quando_o_executor_assenta();
            }
        }
        return;
    }
    let elapsed = start.elapsed().as_secs();
    // 3 estados travados, cada um com o critério certo:
    let fire = if nao_passou_da_engagement {
        // engagement: claramente quebrado → tempo-desde-o-boot (boot sadio passa em segundos).
        elapsed >= BOOT_HANG_WATCHDOG_SECS
    } else if iniciando_usuario {
        // loading: pode ser só load lento → conservador, tempo-desde-o-boot longo.
        elapsed >= BOOT_STUCK_LOADING_WATCHDOG_SECS
    } else {
        // MENU (phase 3-4) empacado: DWELL de `BOOT_STUCK_MENU_DWELL_SECS` sem virar gameplay = o autocontinue falhou (ex.: GOG,
        // lever perdeu a phase 1) → resgata pra o usuário ver o menu. Com o splash já fora (menu pronto sem load, `BwmsMenuReadySplashOff`)
        // o menu está na tela e não há o que resgatar (sem esta guarda: disparo falso aos 185 s com o usuário no menu,
        // `proofs/20261001-bootB-splash-ate-o-menu-pronto-PROVADO.log`).
        if !boot_splash_active() {
            return;
        }
        elapsed.saturating_sub(PHASE_MAX_CHANGED_ELAPSED.load(Ordering::Relaxed))
            >= BOOT_STUCK_MENU_DWELL_SECS
    };
    if !fire {
        return; // ainda dentro do prazo pra este estado
    }
    let onde = if nao_passou_da_engagement {
        "na engagement (phase<=1)"
    } else if iniciando_usuario {
        "no loading (phase 2)"
    } else {
        "no menu (phase 3-4, autocontinue não completou)"
    };
    crate::veredito_dev!(
        "[skipintro] WATCHDOG: boot travado {onde} após {elapsed}s -> tirando o splash + liberando o bink (o usuário vê o menu/tela nativa em vez do splash preso)"
    );
    boot_splash_off();
    BINK_RELEASED.store(true, Ordering::Relaxed);
}

/// Saída do splash no fim do load do autocontinue: 2 amostras seguidas com o executor abaixo de 10% do pico por segundo desde a 3ª
/// leitura de phase 5. A rajada de 1,4–3,4 M chamadas/s vem 1–2 s depois dessa leitura; simulado nas linhas [execpace] de 238 logs
/// de `proofs/`, o critério cai 3–6 s depois dela em todos. Roda no tick (via `boot_hang_watchdog`) porque depois da 5ª leitura o
/// getter não é mais chamado. O que está na tela nesse instante é [não medido].
fn splash_fora_quando_o_executor_assenta() {
    let Some(t) = FIM_DO_LOAD_AT.get() else { return };
    let anterior = ASSENTO_SEG.load(Ordering::Relaxed);
    let seg = t.elapsed().as_secs();
    // 1 amostra por segundo: getter e tick chegam aqui por threads diferentes.
    if seg <= anterior || ASSENTO_SEG.compare_exchange(anterior, seg, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return;
    }
    let agora = CALLS.load(Ordering::Relaxed);
    let taxa = agora.saturating_sub(ASSENTO_CALLS.swap(agora, Ordering::Relaxed)) / (seg - anterior);
    let pico = ASSENTO_PICO.fetch_max(taxa, Ordering::Relaxed).max(taxa);
    if taxa * 10 >= pico {
        ASSENTO_BAIXAS.store(0, Ordering::Relaxed);
        return;
    }
    if ASSENTO_BAIXAS.fetch_add(1, Ordering::Relaxed) < 1 {
        return;
    }
    boot_splash_off();
    crate::veredito_dev!(
        "[splash] fora: executor assentou {seg}s depois do fim do load ({taxa}/s, pico {pico}/s), t={}s",
        boot_elapsed_secs()
    );
}

/// Global do PRÓPRIO watchdog do motor (CDPR, não-nosso): `red::VTable<void,CName,Vector3>::...`
/// (nome real do binário é opaco — descoberto por RE de instrução, não por símbolo). Vive numa
/// thread dedicada "WatchdogThread": acumula tempo (`this+0x50`) a cada ciclo de sleep e, se
/// `acumulado > GLOBAL[0x634]*1000`ms, dispara um assert fatal próprio (`BRK #1`, static vmaddr
/// `0x104d86e48`/`0x1052a2284` — mesma assinatura documentada em `cp77-gog-memory-placement-wall`
/// como "crash do WatchdogThread sob Low Power Mode"). `GLOBAL[0x634]` é o budget ATUAL em
/// segundos; `GLOBAL[0x638]`/`GLOBAL[0x63c]` são os limites min/max (confirmados ao vivo via
/// `peekq`: min=1, max=86400 — 24h). Sob macOS Low Power Mode o CPU/GPU throttlado faz o
/// GameThread demorar demais entre "petadas" do watchdog, o acumulado passa do budget (visto na
/// prática: default=120s) e o motor se mata sozinho — não é bug do BWMS, mas afeta qualquer boot
/// pesado (o nosso inclusive) rodando sob LPM.
///
/// Fix: sobrescrever `GLOBAL[0x634]` DIRETO (sem passar pela função setter real — bypassa o
/// clamp dela de propósito, mas o valor escolhido já cai dentro do próprio range [min,max] que o
/// motor considera válido) com um budget bem maior, TODO TICK (não só 1x no boot) — outro código
/// do motor pode chamar a setter real e resetar o valor em qualquer fase de transição; escrever
/// de novo a cada tick garante que a NOSSA escrita sempre vence. É um único `store` de 4 bytes
/// (`u32`) num endereço `__DATA` já confirmado gravável. Roda SEMPRE (não gated por dev/skip-intro)
/// — é robustez pro usuário final, não uma ferramenta de diagnóstico.
const ENGINE_WATCHDOG_BUDGET_VMADDR: u64 = 0x1_09d0_e51c;
/// 3600s = 1h de budget (bem dentro do [1, 86400] observado) — generoso o bastante pra qualquer
/// boot/tick pesado sob CPU/GPU throttled, sem desligar o watchdog por completo (`max` observado).
const ENGINE_WATCHDOG_BUDGET_SECS: u32 = 3600;

/// Estado do endereço do budget: 0 = não conferido, 1 = legível (endereço guardado em
/// [`WDOG_BUDGET_PTR`]), 2 = ilegível neste build. `is_readable` é um `mach_vm_read_overwrite` — um
/// syscall Mach POR FRAME, mais o zeramento de um buffer de 512 B, pra reconfirmar um endereço
/// `__DATA` do próprio binário, que não deixa de estar mapeado enquanto o processo vive. Confere uma
/// vez; da segunda em diante o tick só faz o `read_volatile` de 4 bytes que já fazia.
static WDOG_ESTADO: AtomicU64 = AtomicU64::new(0);
static WDOG_BUDGET_PTR: AtomicPtr<u32> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn neutralize_engine_watchdog() {
    unsafe {
        let p = match WDOG_ESTADO.load(Ordering::Relaxed) {
            1 => WDOG_BUDGET_PTR.load(Ordering::Relaxed),
            2 => return, // endereço ilegível (build/layout inesperado) -> não escreve às cegas
            _ => {
                let p = crate::rebase(ENGINE_WATCHDOG_BUDGET_VMADDR) as *mut u32;
                if !crate::memoria::is_readable(p as *const c_void, 4) {
                    WDOG_ESTADO.store(2, Ordering::Relaxed);
                    return;
                }
                WDOG_BUDGET_PTR.store(p, Ordering::Relaxed);
                WDOG_ESTADO.store(1, Ordering::Relaxed);
                p
            }
        };
        let before = p.read_volatile();
        if before != ENGINE_WATCHDOG_BUDGET_SECS {
            p.write_volatile(ENGINE_WATCHDOG_BUDGET_SECS);
            crate::log_dev!(
                "[wdog-neutralize] budget do watchdog do motor {before}s -> {ENGINE_WATCHDOG_BUDGET_SECS}s (proteção contra Low Power Mode)"
            );
        }
    }
}

/// O splash de boot deve aparecer agora? (skip-intro ligado + ainda não chegou no menu).
pub fn boot_splash_active() -> bool {
    BOOT_SPLASH_ON.load(Ordering::Relaxed)
}
/// `BwmsAcFired` chama isto — alimenta o marco "autocontinue disparou" de `boot_progress()`.
pub(crate) fn note_autocontinue_fired() {
    AUTOCONTINUE_FIRED.store(true, Ordering::Relaxed);
    MENU_DO_BOOT_RESOLVIDO.store(true, Ordering::Relaxed);
}
/// `BwmsMenuReadySplashOff` chama isto quando o menu fica pronto sem load a fazer (no menu do boot e em todo menu seguinte; repetir não
/// muda nada).
pub(crate) fn marcar_menu_do_boot_resolvido() {
    MENU_DO_BOOT_RESOLVIDO.store(true, Ordering::Relaxed);
}
/// O load automático já não cabe: o menu do boot resolveu a tarefa, ou uma sessão de jogo já começou (1º phase 5 visto pelo getter,
/// que só existe com `pular_intro`; sem ele também não há splash a proteger). Um resgate do watchdog antes do menu não conta: ele dispara também em boot sadio quando a engagement demora,
/// e o menu do boot ainda carrega.
pub(crate) fn menu_do_boot_resolvido() -> bool {
    MENU_DO_BOOT_RESOLVIDO.load(Ordering::Relaxed) || PHASE_REACHED_5.load(Ordering::Relaxed)
}

/// Chamar 1x, o mais cedo possível no `on_load` (antes de `selfboot_if_needed`/qualquer hook) —
/// ver `STALE_BOOT_ATTEMPT`. Consome (apaga) o marcador se achar, pra não confundir o PRÓXIMO boot.
pub(crate) fn check_stale_boot_attempt() {
    if let Some(m) = crate::inicio::natives::boot_attempt_marker() {
        if m.exists() {
            let _ = std::fs::remove_file(&m);
            STALE_BOOT_ATTEMPT.store(true, Ordering::Relaxed);
            crate::log_dev!("[firestart] marcador de tentativa anterior encontrado (boot passado não fechou limpo) — auto-load fica suprimido NESTE boot por segurança");
        }
    }
}

/// `tramp_autocontinue` consulta isto em vez de reler o arquivo — reler o arquivo lá reagiria ao
/// marcador que O PRÓPRIO `tramp_fire_start` desta mesma sessão acabou de escrever (falso positivo
/// em TODO boot, não só nos que travaram). O valor é decidido 1x, cedo, antes do lever poder disparar.
pub(crate) fn autocontinue_suppressed_stale_boot() -> bool {
    STALE_BOOT_ATTEMPT.load(Ordering::Relaxed)
}

/// Desarma o dead-man's switch (`~/.bwms-boot-attempt`). Chamado (a) no fim do load do save (3ª leitura de phase 5)
/// — a etapa ARRISCADA (o auto-load do save) já provou que não crashou, então o marcador cumpriu
/// seu papel e pode sair, INDEPENDENTE de como o usuário fechar o jogo depois — e (b) no exit()
/// hookado. Só o exit() não basta: fechar por "SAIR DO JOGO"/kill -9/watchdog do CDPR/força-quit
/// deixaria o marcador armado → TODO boot seguinte cairia pro menu (modo2→1) sem explicação. HOME
/// ausente = loga aviso e degrada pro modo SEGURO (o próximo boot cai pro menu, nunca crasha).
pub(crate) fn clear_boot_attempt_marker() {
    match crate::inicio::natives::boot_attempt_marker() {
        Some(m) => {
            let _ = std::fs::remove_file(m);
        }
        None => crate::log(
            "[firestart] HOME ausente — não deu pra limpar o dead-man's switch (próximo boot cai pro menu por segurança)",
        ),
    }
}
/// Progresso REAL do boot por MARCOS OBSERVADOS, não por tempo fixo (2026-07-12: a versão antiga
/// dividia elapsed/42s e travava em 0.99 — em máquina/condição mais lenta (ex. o overlay da Steam
/// atrasando a inicialização ~50-90s) isso rushava pra 99% e ficava ali parado por dezenas de
/// segundos, PARECENDO travado quando na verdade o boot seguia progredindo por trás. Cada marco só
/// avança quando o evento REAL acontece; nunca "enche" sozinho por tempo.
pub fn boot_progress() -> f32 {
    if BOOT_START.get().is_none() {
        return 0.0;
    }
    if PHASE_REACHED_5.load(Ordering::Relaxed) {
        0.9
    } else if AUTOCONTINUE_FIRED.load(Ordering::Relaxed) {
        0.7
    } else if crate::terminal::overlay::seen_engagement() {
        0.4
    } else {
        0.15
    }
}
/// Texto do estágio atual (junto com os segundos decorridos) — mostra o usuário QUE ainda está
/// progredindo mesmo quando a % fica parada num marco por muito tempo em máquina/condição mais
/// lenta, em vez de deixar só um número estático que parece travado.
pub fn boot_stage_label() -> &'static str {
    if PHASE_REACHED_5.load(Ordering::Relaxed) {
        "carregando o mundo"
    } else if AUTOCONTINUE_FIRED.load(Ordering::Relaxed) {
        "carregando o save"
    } else if crate::terminal::overlay::seen_engagement() {
        "iniciando a sessão"
    } else {
        "carregando dados do jogo"
    }
}
/// Segundos decorridos desde o início do boot (p/ o texto "12s").
pub fn boot_elapsed_secs() -> u64 {
    BOOT_START.get().map(|t| t.elapsed().as_secs()).unwrap_or(0)
}
/// Pular a intro: os logos (bink) e a engagement "APERTE ESPAÇO" (lever d4=2). Vem de `red4ext/bwms.toml` (ligado por padrão), com
/// `~/.bwms-skipintro` valendo por cima (`configuracao_do_mod`). Bink, getter, lever e splash leem só isto.
fn skipintro_enabled() -> bool {
    crate::configuracao::do_mod::config().pular_intro
}
fn boot_splash_arm() {
    // ÂNCORA DE TEMPO DO BOOT — armada SEMPRE que o skip liga, independente de o splash VISUAL ser
    // desenhado (o watchdog anti-hang `boot_hang_watchdog` e o `boot_progress` dependem dela). Antes
    // o branch de opt-out retornava ANTES disto → com `~/.bwms-nosplash` a âncora nunca era setada e
    // o watchdog nunca dispararia. Agora fica no topo.
    let _ = BOOT_START.set(std::time::Instant::now());
    // O desenho do splash só sai com `splash = false` no `bwms.toml` (ou `~/.bwms-nosplash`, que o `config()` já considera): fica a tela
    // preta de loading nativa. O desenho custa 1 composição por segundo e reusa os buffers da GPU, então não depende de RAM.
    if !crate::configuracao::do_mod::config().splash {
        crate::log_dev!("[splash] splash de boot DESLIGADO (splash = false) — só a tela de loading nativa");
        return; // BOOT_SPLASH_ON fica false → não desenha; a âncora acima segue armada
    }
    BOOT_SPLASH_ON.store(true, Ordering::Relaxed);
}

pub(crate) fn boot_splash_off() {
    let estava = BOOT_SPLASH_ON.swap(false, Ordering::Relaxed);
    // O resumo do splash (quadros, pausas evitadas, buraco do executor) só existe com o log de dev.
    #[cfg(not(feature = "devlog"))]
    let _ = estava;
    #[cfg(feature = "devlog")]
    if estava {
        crate::veredito_dev!(
            "[splash] {} quadro(s) composto(s) em {}s de splash, {} pausa(s) do motor por janela oculta evitada(s), maior buraco do executor {}s",
            crate::inicio::splash::QUADROS_COMPOSTOS.load(Ordering::Relaxed),
            boot_elapsed_secs(),
            PAUSAS_EVITADAS.load(Ordering::Relaxed),
            EXECPACE_BURACO_MAX.load(Ordering::Relaxed)
        );
    }
}

/// Predicado do motor que pausa o tick com a janela oculta: devolve 1 e o wrapper `0x104d79e8c` faz `Sleep(33)` e pula o tick
/// (`proofs/20261003-motor-oculto-re-estatica.txt`). O byte que ele consulta é o `isWindowVisible` (`0x107d84c40`), gravado pelo handler
/// de oclusão da janela. Na engagement e no menu essa pausa congela o timer da alavanca até a janela voltar.
const MOTOR_PAUSA_OCULTO_VM: u64 = 0x1_04d7_9d80;
const MOTOR_PAUSA_OCULTO_PROLOGO: u32 = 0xd100_c3ff;
static ORIG_MOTOR_PAUSA_OCULTO: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devlog")]
static PAUSAS_EVITADAS: AtomicU64 = AtomicU64::new(0);
/// Dev (`~/.bwms-hook-tick-com-janela-oculta`, ligado pelo `bwms-smoke.sh`): o tick segue com a janela oculta também depois do splash. Sem
/// isso o motor dorme quando o dono usa outro app durante o teste, o canal de comandos para e o smoke perde comandos (boots de 2026-10-06 e
/// 2026-10-12: `usleep` do loop do jogo na pilha, `janela_visivel=0`). Lido uma vez, na instalação: o predicado roda a cada quadro.
static TICK_COM_JANELA_OCULTA: AtomicBool = AtomicBool::new(false);

/// Com o splash do boot no ar o motor não pausa por janela oculta: o boot segue até a gameplay com a janela em outro Space, como já segue
/// quando ela nunca foi vista. Toda saída do splash desliga o portão, e a pausa do jogo volta a valer.
unsafe extern "C" fn motor_pausa_oculto(motor: *mut c_void) -> u32 {
    let original: unsafe extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(original_publicado(&ORIG_MOTOR_PAUSA_OCULTO));
    let pausar = original(motor);
    // Também depois de um resgate do watchdog antes do menu: o splash saiu, mas a engagement e o menu do boot ainda precisam de tick.
    if pausar != 0
        && (boot_splash_active()
            || (BINK_RELEASED.load(Ordering::Relaxed) && !menu_do_boot_resolvido())
            || TICK_COM_JANELA_OCULTA.load(Ordering::Relaxed))
    {
        #[cfg(feature = "devlog")]
        if crate::primeiras(&PAUSAS_EVITADAS, 1) {
            crate::veredito_dev!(
                "[motor-oculto] janela oculta em t={}s: o tick segue (splash no ar {}, marcador do smoke {})",
                boot_elapsed_secs(),
                boot_splash_active(),
                TICK_COM_JANELA_OCULTA.load(Ordering::Relaxed)
            );
        }
        return 0;
    }
    pausar
}

/// Gancho da pausa por janela oculta, só com o splash armado. Prólogo diferente (ou endereço sem tradução noutro build) deixa o jogo como
/// está: o boot pausa com a janela oculta e o watchdog do splash segue de rede.
unsafe fn install_motor_sem_pausa_no_boot() {
    if crate::marcador_individual("janela-oculta-sem-gancho") {
        crate::veredito_dev!("[motor-oculto] gancho não instalado (marcador de teste)");
        return;
    }
    let tick_sempre = crate::marcador_individual("tick-com-janela-oculta");
    TICK_COM_JANELA_OCULTA.store(tick_sempre, Ordering::Relaxed);
    if !boot_splash_active() && !tick_sempre {
        return;
    }
    let posto = instalar_conferindo_prologo(MOTOR_PAUSA_OCULTO_VM, MOTOR_PAUSA_OCULTO_PROLOGO, motor_pausa_oculto as *mut c_void, &ORIG_MOTOR_PAUSA_OCULTO);
    crate::veredito_dev!("[motor-oculto] gancho no ar: {posto:?} (tick com a janela oculta depois do splash: {tick_sempre})");
}

/// Teste (dev): ao entrar na engagement, grava 0 no `isWindowVisible` como se a janela tivesse ido pra outro Space, e devolve o valor em
/// 60 s. Sem tecla e sem captura: prova a pausa (sem gancho) e o gancho (com ele) pelo `[execpace]` e pelos vereditos.
#[cfg(feature = "devtools")]
fn simular_janela_oculta() {
    if !crate::marcador_individual("janela-oculta-teste") {
        return;
    }
    let byte = crate::rebase(0x1_07d8_4c40) as *mut u8;
    if !unsafe { crate::memoria::is_readable(byte as *const c_void, 1) } {
        crate::veredito_dev!("[motor-oculto-teste] isWindowVisible ilegível");
        return;
    }
    let antes = unsafe { byte.read_volatile() };
    unsafe { byte.write_volatile(0) };
    crate::veredito_dev!("[motor-oculto-teste] isWindowVisible {antes}->0 em t={}s", boot_elapsed_secs());
    let endereco = byte as usize;
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let byte = endereco as *mut u8;
        if unsafe { byte.read_volatile() } == 0 {
            unsafe { byte.write_volatile(antes) };
        }
        crate::veredito_dev!("[motor-oculto-teste] isWindowVisible de volta a {antes} em t={}s", boot_elapsed_secs());
    });
}
static ORIG_EXEC: AtomicU64 = AtomicU64::new(0);
static CLS_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLS_PL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
// PERF (2026-08-16, achado via lldb num boot travado): `capture()` cai no caminho caro
// (`ganchos::is_readable` = syscall `mach_vm_read_overwrite`, depois `rtti::class_of`) toda vez
// que `ctx` NÃO é o player/tx já conhecido — durante streaming pesado de mundo, a esmagadora
// maioria dos `ctx` são objetos distintos legítimos (não player/tx), então isso rodava a
// syscall em quase TODA chamada de exec_replacement (hot-path extremo). Rate-limitado: só
// tenta a via cara 1 em cada `CAPTURE_MISS_STRIDE` misses da mesma thread — ainda detecta player/tx dentro de
// poucas dezenas de chamadas (rápido o bastante pra qualquer uso real), mas corta o volume de
// syscalls por essa mesma proporção durante fases de alto volume de ctx desconhecido. O contador é
// por thread (`ContadoresDoExecutor`): global, ele seria 1 RMW por chamada disputado por todas as threads de script.
const CAPTURE_MISS_STRIDE: u64 = 64;
static CALLS: AtomicU64 = AtomicU64::new(0);

// ===== [execpace] — instrumentação da rodada 31 (2026-08-17), investigando o achado de infra
// da rodada 30: a cadeia de `@wrapMethod(PlayerPuppet) OnGameAttached` (108 arquivos `.reds`
// distintos empilhados pelo compilador redscript, acumulados desde 2026-06) pode estar longa o
// suficiente pra nunca devolver controle ao `cp77_tick` normal dentro da janela de boot testada
// — hipótese levantada mas nunca medida. Em vez de tocar a cadeia em si (risco alto, usada por
// TODO smoke test do projeto), mede-se o RITMO de `exec_replacement` (via `CALLS`, já existente,
// incrementado em toda chamada de script não-suprimida/não-roteada) throttled a 1 log/segundo,
// chamado de dentro de `cp77_tick` (que já dispara a cada [`TICK_EVERY`] chamadas do executor —
// inclusive DURANTE a rajada de wraps, já que cada `Print`/chamada de método dentro de um wrap É
// uma chamada de script que passa pelo executor). Puramente observacional: 1 `OnceLock::get_or_init`
// + 1 CAS por chamada de `cp77_tick`, sem nenhuma mudança de comportamento. Isolado de
// `BOOT_START` de propósito (esse só arma com skip-intro ligado; este timer é sempre ativo).
static EXECPACE_T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
#[cfg(feature = "devlog")]
static EXECPACE_LAST_SEC: AtomicU64 = AtomicU64::new(u64::MAX);

/// Getter público pro contador de chamadas do executor (`CALLS`) — sem expor o `static` privado.
#[cfg(feature = "devlog")]
pub fn exec_calls() -> u64 {
    CALLS.load(Ordering::Relaxed)
}

/// Loga o ritmo de `exec_replacement` (`[execpace] t=Ns calls=N`), throttled a 1x/segundo real.
/// Chamar de dentro de `cp77_tick` (barato — early-return se o segundo não mudou desde a
/// última chamada). Ver comentário do bloco acima pro contexto completo da investigação.
/// Vazia no build público: é medição de ritmo do executor, e o `Instant::elapsed` + o swap rodavam
/// todo frame só pra alimentar um log que lá não existe.
#[cfg(not(feature = "devlog"))]
#[inline(always)]
pub fn log_exec_pace() {}

#[cfg(feature = "devlog")]
pub fn log_exec_pace() {
    let t0 = EXECPACE_T0.get_or_init(std::time::Instant::now);
    let secs = t0.elapsed().as_secs();
    let prev = EXECPACE_LAST_SEC.swap(secs, Ordering::Relaxed);
    if prev != secs {
        crate::log_dev!("[execpace] t={secs}s calls={}", exec_calls());
        // Buraco de 5 s ou mais com o splash no ar, depois da engagement = o motor não ticou (janela oculta sem gancho, ou travamento).
        // Antes da engagement o executor de script quase não roda na tela de loading (~t=4→82 em todo boot): não conta.
        if prev != u64::MAX && secs >= prev + 5 && boot_splash_active() && crate::terminal::overlay::seen_engagement() {
            EXECPACE_BURACO_MAX.fetch_max(secs - prev, Ordering::Relaxed);
            crate::veredito_dev!("[motor-oculto] executor parado {}s (t={prev}→{secs}) com o splash no ar", secs - prev);
        }
    }
}

/// Maior intervalo entre duas linhas `[execpace]` com o splash no ar; sai no veredito do fim do splash.
#[cfg(feature = "devlog")]
static EXECPACE_BURACO_MAX: AtomicU64 = AtomicU64::new(0);

// ===== [fntrace] — instrumentação da rodada 32 (2026-08-16/17), continuação direta da bisecção
// do achado `[execpace]` (rodada 31): sabemos que o log VIVO para 155+s logo após
// `codeware-inklayerwrapper-smoke.reds` terminar, com o processo consumindo 852-863% CPU
// sustentado — mas `[execpace]` só diz O RITMO (quantas chamadas/segundo), nunca QUAL função
// está sendo chamada. `[fntrace]` fecha essa lacuna: loga o CNAME resolvido (`func+0x10`, mesmo
// offset já usado por `RUST_OV_CNAME`) toda vez que
// ele MUDA em relação à última chamada logada — não a cada chamada (evitaria flood/custo alto
// numa rajada de 100k+ calls/s) — dando um traço COMPACTO de transição de função. Se o processo
// trava numa função REDSCRIPT específica (loop/recursão), a ÚLTIMA linha `[fntrace]` antes do
// silêncio aponta pra ela com precisão cirúrgica. Se o silêncio for CÓDIGO NATIVO puro do motor
// (sem mais chamadas de script — ex. streaming/spawn de mundo pós-`OnGameAttached`, hipótese
// alternativa que refutaria "bug na cadeia de wraps"), a última linha simplesmente para de
// mudar e mais nenhuma aparece, mesmo sinal mas agora com NOME em vez de só contagem.
// Opt-in via `~/.bwms-fntrace` (mesmo idioma de `skipintro_enabled`), checado 1x e cacheado —
// custo no hot-path desligado = 1 leitura atômica de bool, igual a `dev_mode()`.
static FNTRACE_ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static FNTRACE_LAST_MCNAME: AtomicU64 = AtomicU64::new(0);

fn fntrace_enabled() -> bool {
    *FNTRACE_ON.get_or_init(|| {
        cfg!(feature = "devtools") && (std::env::var_os("BWMS_FNTRACE").is_some() || crate::marcador_de_dev(".bwms-fntrace"))
    })
}

/// Loga `[fntrace] t=Ns calls=N -> <nome resolvido>` só quando o CNAME da função chamada MUDA
/// desde a última chamada — chamar com o `mcname` já lido em `exec_replacement` (evita reler
/// `func+0x10` 2x). Sem custo quando desligado (1 `OnceLock` já-inicializado + early-return).
fn log_fn_trace(mcname: u64, calls: u64) {
    if !fntrace_enabled() {
        return;
    }
    let prev = FNTRACE_LAST_MCNAME.swap(mcname, Ordering::Relaxed);
    if prev == mcname {
        return;
    }
    let t0 = EXECPACE_T0.get_or_init(std::time::Instant::now);
    let secs = t0.elapsed().as_secs();
    let name = crate::cname::resolve_cname(mcname);
    crate::log_dev!("[fntrace] t={secs}s calls={calls} -> {name}");
}

// FromTDBID capturado NATIVAMENTE (fn/ctx/ret). Antes isto vinha de fora, escrito em
// /tmp/cp77-fromtd.txt; como o ASLR muda por sessão, ler o arquivo de outra sessão dava
// endereço morto → crash no cheat de item. Agora o hook do executor publica aqui.
pub static FROMTD_TGT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); // addr do FromTDBID (resolvido 1x no tick)
pub static FROMTD_CTX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); // ctx capturado em runtime
pub static FROMTD_RET: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); // ret_type capturado


/// Override RUST-nativo (validação do Override-suppress SEM lua, à prova de aninhamento):
/// se `RUST_OV_CNAME` != 0 e o método em execução tem esse CName (func+0x10), o executor
/// escreve `RUST_OV_VAL` no aOut (tipado) e RETORNA (suprime a original). É o suppress puro
/// (interceptar + retorno custom + pular original) num caminho Rust-only — o que o teste via
/// lua não conseguia (o cb lua crashava no stack profundo do aninhamento).
/// Só no dev: quem arma são os comandos de prova `ovcall` e `cethooksproof`, e prova de item fechado não vai
/// ao público (régua de 2026-10-03). Assim o executor do público não paga os 2 loads por chamada.
#[cfg(feature = "devtools")]
pub static RUST_OV_CNAME: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub static RUST_OV_VAL: AtomicI64 = AtomicI64::new(0);

/// `cet-hooks-shippable` — perna OBSERVE (a que faltava: Override+Suppress via `RUST_OV_*` já
/// provados, 2026-07-13). Mesmo mecanismo do Override, mas NÃO suprime: só loga que o callback
/// disparou e deixa a execução CAIR PRA FORA (a original roda normalmente depois). Prova que dá
/// pra "escutar" um método real sem alterar seu comportamento — o 3º modo do contrato CET
/// (Observe/Override/Suppress) por caminho não-Lua. Só no dev, pelo mesmo motivo de `RUST_OV_CNAME`.
#[cfg(feature = "devtools")]
pub static RUST_OBS_CNAME: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub static RUST_OBS_HITS: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod nolua_ret_tests {
    use super::*;
    use crate::cname::cname;

    /// Só teste (regra do dono de 2026-10-03, D6): nenhum caminho do executor chama isto, então não entra no dylib.
    /// Escreve `val` (i64 = a fonte) em `res` na LARGURA CORRETA do tipo de retorno, dado o NOME
    /// (CName) e o SIZE PRÉ-COMPUTADOS — SEM chamar vtable (`type_size`/`type_name_getname` são
    /// vtable-calls = a causa-raiz do crash no stack aninhado; ver notes/goal-next-steps.md). Trunca/
    /// converte i64 pro POD. true = escreveu (tipo POD conhecido), false = recusa (não escreve lixo).
    /// É o `write_pod_ret` type-aware do caminho no-lua; a INTEGRAÇÃO (pré-computar nome/size na
    /// registração + chamar aqui no suppress em vez do i64 cru) é o próximo passo (não-crash = gated).
    unsafe fn write_pod_ret_nolua(name: u64, size: u32, val: i64, res: *mut c_void) -> bool {
        if res.is_null() {
            return false;
        }
        let p = res as *mut u8;
        if name == cname("Bool") && size == 1 {
            *p = u8::from(val != 0);
            return true;
        }
        if (name == cname("Int8") || name == cname("Uint8")) && size == 1 {
            *p = val as u8;
            return true;
        }
        if (name == cname("Int16") || name == cname("Uint16")) && size == 2 {
            (p as *mut i16).write_unaligned(val as i16);
            return true;
        }
        if (name == cname("Int32") || name == cname("Uint32")) && size == 4 {
            (p as *mut i32).write_unaligned(val as i32);
            return true;
        }
        if (name == cname("Int64") || name == cname("Uint64")) && size == 8 {
            (p as *mut i64).write_unaligned(val);
            return true;
        }
        if name == cname("Float") && size == 4 {
            (p as *mut f32).write_unaligned(val as f32);
            return true;
        }
        if name == cname("Double") && size == 8 {
            (p as *mut f64).write_unaligned(val as f64);
            return true;
        }
        false
    }

    #[test]
    fn write_pod_ret_nolua_widths() {
        unsafe {
            let mut buf = [0u8; 8];
            assert!(write_pod_ret_nolua(cname("Int32"), 4, 0x1234, buf.as_mut_ptr() as *mut c_void));
            assert_eq!(i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 0x1234);
            assert_eq!(buf[4], 0, "Int32 não pode escrever além de 4 bytes");
            buf = [0u8; 8];
            assert!(write_pod_ret_nolua(cname("Bool"), 1, 1, buf.as_mut_ptr() as *mut c_void));
            assert_eq!(buf[0], 1);
            assert_eq!(buf[1], 0);
            buf = [0u8; 8];
            assert!(write_pod_ret_nolua(cname("Float"), 4, 2, buf.as_mut_ptr() as *mut c_void));
            assert_eq!(f32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]), 2.0);
            assert!(!write_pod_ret_nolua(cname("Vector4"), 16, 0, buf.as_mut_ptr() as *mut c_void));
        }
    }
}

/// True = a dylib está dirigindo o runtime sozinha (modo nativo).
pub(crate) fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

use crate::macos::{_dyld_get_image_name, _dyld_image_count};

/// Estamos DENTRO do processo do jogo? (imagem 0 = executável principal). Evita
/// auto-bootar em testes/`dlopen` de validação, onde o executor rebaseia errado.
pub(crate) unsafe fn in_game() -> bool {
    // Procura "Cyberpunk2077" em TODAS as imagens, não só a índice 0. BUG (achado in-game
    // 2026-06-24 via diagnóstico): `_dyld_get_image_name(0)` NÃO é garantido ser o
    // executável principal no momento do ctor → in_game dava false → o self-boot NUNCA
    // instalava o hook do executor. O `game_base()` (lib.rs) já buscava por nome; in_game
    // agora faz igual. (A injeção antiga, de fora do processo, seguia outro caminho e não passava por aqui.)
    let n = _dyld_image_count();
    for i in 0..n {
        let nm = _dyld_get_image_name(i);
        if nm.is_null() {
            continue;
        }
        let s = CStr::from_ptr(nm).to_string_lossy();
        // Casa pelo nome do executável interno ("Cyberpunk2077", que NÃO muda ao renomear o .app),
        // pelo bundle-id conhecido (Steam/GOG) OU pelo nome do .app ("Cyberpunk 2077") — cobre
        // distribuições/renomeações que a substring simples do executável poderia perder.
        if s.contains("Cyberpunk2077")
            || s.contains("com.cdprojektred.cyberpunk")
            || s.contains("Cyberpunk 2077")
        {
            return true;
        }
    }
    // Nada casou → o executor NÃO será instalado (nenhuma feature BWMS). Loga 1x pra o no-op ser
    // DIAGNOSTICÁVEL (antes era um silêncio total: "instalei e não faz nada, sem log").
    if !NO_GAME_LOGGED.swap(true, Ordering::Relaxed) {
        crate::log_dev!("[selfboot] BWMS não reconheceu o processo do jogo (nenhuma imagem dyld casou 'Cyberpunk2077'/bundle-id) — hooks NÃO instalados");
    }
    false
}
static NO_GAME_LOGGED: AtomicBool = AtomicBool::new(false);


pub(crate) unsafe fn selfboot_if_needed() {
    // Chamado só pelo `on_load` (lib.rs), depois do gate de versão: build não suportado nunca chega aqui.
    // O guard ACTIVE deixa a chamada idempotente.
    if ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let ig = in_game();
    crate::log_dev!("[selfboot] selfboot_if_needed: in_game={ig}");
    if !ig {
        return; // testes/dlopen de validação: não toca em nada
    }
    crate::log_dev!("[bwms] modo runtime nativo");
    let target = crate::rebase(crate::jogo::scripts::EXECUTOR_VM);
    // GUARD: só hooka se os bytes do alvo forem MESMO o prólogo do executor.
    // Protege contra endereço errado / patch do jogo → aborta limpo, nunca crasha.
    if !crate::memoria::is_readable(target as *const c_void, 8) {
        crate::log_dev!("[selfboot] alvo do executor ilegível -> abortando (sem crash)");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != EXECUTOR_PROLOGO {
        crate::log_dev!(
            "[selfboot] prólogo do executor não casou ({got:#018x} != {EXECUTOR_PROLOGO:#018x}) -> abortando (sem crash)"
        );
        return;
    }
    match crate::ganchos::instalar_com_threads_paradas(target, exec_replacement as *mut c_void, Some(&ORIG_EXEC)) {
        Some(_) => {
            ACTIVE.store(true, Ordering::Relaxed);
            crate::log_dev!("[bwms] hook do runtime instalado (nativo, Rust puro)");
            // Censo de hooks observe-only (`censo.rs`). FORA do `dev_mode()`: no boot 9 de 2026-09-11 ele
            // estava dentro de `run_dev_selftests`, que sai sem `/tmp/bwms-dev`, e não instalou nada. O
            // gate dele é o próprio marcador `~/.bwms-hook-censo`.
            #[cfg(feature = "devtools")]
            crate::diagnostico::censo::install();
            crate::roupas_e_armas::terceira_pessoa::install();
            // Self-test DEV-GATED do hooking + diagnóstico ArchiveXL. SÓ roda sob
            // dev_mode() (no-op em produção); chamado DEPOIS do hook do executor estar
            // instalado. Não toca em nada do jogo num boot normal.
            #[cfg(feature = "devtools")]
            if crate::dev_mode() {
                crate::selftest::run_dev_selftests();
            }
        }
        None => crate::log_dev!("[selfboot] FALHA ao hookar o executor"),
    }
    // F-B: install_bind_bridge() agora roda no TOPO do on_load (mais cedo que aqui), pois o bind
    // (RedScriptsHost::Load → orchestrator @0x101113e44) roda muito cedo. register_all tb no cp77_tick. No selfboot/ctor o
    // CRTTISystem::Get crasha (RTTI não pronto).
    // SAÍDA LIMPA (incondicional): mata a janela "Cyberpunk2077 quit unexpectedly" ao FECHAR.
    // Não é gateado — o crash de shutdown do RED4ext afeta todo fechamento.
    install_clean_exit();
    // App Nap OFF (incondicional): o macOS pausa o jogo quando ele não está na frente (janela
    // ocluída) → no CPVR, olhando o capacete, o jogo congela (boot lento + engagement travada +
    // câmera com "coices"). Desabilitar mantém o jogo a full em background. Barato, sem risco.
    crate::terminal::overlay::disable_app_nap();
    // Registro de energia deste boot (baixo consumo, tomada, térmico): o boot seguinte o lê se este não chegar ao fim do load nem à saída
    // limpa (`inicio::energia`). O do boot anterior já foi lido no `on_load`, antes do relatório de crash.
    {
        let cfg = crate::configuracao::do_mod::config();
        let modo = match (cfg.pular_intro, cfg.continuar_automatico) {
            (true, true) => "continue",
            (true, false) => "menu",
            _ => "sem-skip",
        };
        let e = crate::inicio::energia::ler();
        crate::inicio::energia::gravar(&e, modo);
        crate::veredito_dev!("[energia] início: {}", crate::inicio::energia::linha(&e, modo));
    }
    // Splash de boot: a âncora de tempo e o watchdog dependem do skip (é ele que deixa a tela preta),
    // e o desenho em si tem opção própria no `bwms.toml`.
    if skipintro_enabled() {
        boot_splash_arm();
        install_motor_sem_pausa_no_boot();
    }
    // Skip-intro (`pular_intro`): pula os LOGOS de boot (funciona, ~10s).
    install_bink_skip();
    // Zero tecla e zero Acessibilidade: nada aqui injeta input. O hook do getter da phase byte
    // @0x1015d2dec devolve a phase real (não mente), captura o SM em `GAME_SESSION_DESC`, dispara a
    // alavanca d4=2 com `pular_intro` (`fire_lever_direct`) e alimenta o watchdog e o fim do load
    // que tira a splash (3ª leitura de phase 5) — ver `phase_skip_getter`.
    install_phase_skip_near4(); // B de 4B: vizinha INTACTA em proofs/2026-06-25-honest-up.log

    // DIAGNÓSTICO near4 (F-A): a distância dylib↔__TEXT do jogo decide se o `B` de 4 bytes
    // alcança o alvo DIRETO (≤±128MB) ou se o instalar_perto precisa de um veneer near. Loga 1x.
    {
        let dylib_fn = exec_replacement as *const () as u64;
        let getter = crate::rebase(0x1_015d_2dec) as u64;
        let dist = (dylib_fn as i64 - getter as i64).unsigned_abs();
        crate::log_dev!(
            "[near4] dylib={dylib_fn:#x} alvo_getter={getter:#x} dist={}MB B-direto-viavel={}",
            dist >> 20,
            dist < (1 << 27)
        );
    }
}

// ===== HOOK DO GETTER da phase byte (`GameSessionDesc+0x84`) =====
// notes/boot-flow-phase-byte.md: o dispatcher de boot @0x1015e489c lê a phase byte pelo GETTER
// @0x1015d2dec (`ldrsb w0,[x0,#0x84]; ret`) e faz SwitchState pela jump-table (1=engagement título,
// 2=initialize-user/loading, 3=PreGameMenu=MENU). O hook devolve a phase real: gravar 3 pelo
// dispatcher dispara um assert do jogo (EXC_BREAKPOINT) e pular a engagement trava o boot (ver o
// fim de `phase_skip_getter`). Ele serve pra capturar o SM (`GAME_SESSION_DESC`), disparar a
// alavanca d4=2 e acompanhar a phase (watchdog, `FASE_LIDA`, fim do load na 3ª leitura de phase 5).
//
// O getter é leaf de 8B — replace normal (16B) transbordava na vizinha (SIGILL). instalar_perto (B de
// 4B) cabe SE dist<128MB; o diag [near4] (build dev) deu 111–122MB nos 287 logs de proofs/ que têm a
// linha (contados em 2026-09-30), máximo em proofs/20260919-boot276-console-completo.log → viável, folga
// de 6MB (vizinha INTACTA em proofs/2026-06-25-honest-up.log).
static PHASE_GET_ORIG: AtomicU64 = AtomicU64::new(0);
static PHASE_DIAG_HITS: AtomicUsize = AtomicUsize::new(0);
/// Última fase logada pelo getter — pra logar SÓ as MUDANÇAS de fase ao longo de todo o boot
/// (não só as 40 primeiras chamadas). Vê a sequência 1->2->3 quando o SPACE avança.
static PHASE_LAST_LOGGED: AtomicI64 = AtomicI64::new(-128);

/// Getter da phase byte hookado: devolve a phase real (via trampolim; 0 se `this` ilegível) e não
/// escreve o campo. Em volta disso captura o SM, dispara a alavanca d4=2 e alimenta watchdog e splash.
unsafe extern "C" fn phase_skip_getter(this: *mut u8) -> i32 {
    // 2026-08-20 (madrugada, sessão 2, achado de segurança): `this` era dereferenciado (via
    // `f(this)`, chamando o trampolim original que faz `ldrsb w0,[x0,#0x84]`) ANTES de qualquer
    // checagem de legibilidade — só os campos de diagnóstico (0xd4..0xff) eram protegidos por
    // `is_readable`, não a chamada real. Suspeita (não confirmada, mas defesa de baixo risco de
    // qualquer forma): o 2º ponto de crash achado nesta sessão (durante a transição real de
    // autocontinue/save-load, ver memória `bwms-t82s-crash-vs-887pct-hang` Atualização 11) aconteceu
    // logo após um `[phasedbg] getter#N phase=-1` — plausível que `this` fique momentaneamente
    // inválido/sendo realocado durante o teardown de sessão que `LoadModdedSave` dispara. Movida a
    // checagem de legibilidade pra ANTES de qualquer leitura (inclusive a chamada ao trampolim) —
    // hardening puro, não muda comportamento no caso comum (this válido, que é 99%+ das chamadas).
    let orig = PHASE_GET_ORIG.load(Ordering::Relaxed) as *mut c_void;
    // UMA leitura de legibilidade (0x100 cobre o 0x85 do capture E o 0xd4..0xff do fingerprint). Sob RAM
    // baixa cada is_readable (mach_vm_read) pega o vm_map lock e serializa atrás do pager do drive externo;
    // 2->1 por fire corta metade desse custo no getter (≤28 chamadas por boot em 334 logs de proofs/) = P2 do
    // fix do early-stick.
    let readable = !this.is_null() && crate::memoria::is_readable(this as *const c_void, 0x100);
    if !readable {
        // `this` null ou não-mapeado — não dereferenciar de jeito NENHUM (nem via trampolim).
        // Devolve 0 (mesmo sentinela já usado em vários lugares do projeto pra "sem fase ainda").
        return 0;
    }
    let real = if !orig.is_null() {
        let f: unsafe extern "C" fn(*mut u8) -> i32 = std::mem::transmute(orig);
        f(this)
    } else {
        (this.add(0x84) as *const i8).read() as i32
    };
    let fase = FaseDaSessao::do_byte(real as i64);
    // CAPTURA do GameSessionDesc: o getter é `ldrsb w0,[x0,#0x84]` → `this` (=x0) É o GameSessionDesc
    // (o dispatcher chama `mov x0,x20; bl getter` com x20=GameSessionDesc). Até travar no SM da
    // engagement (abaixo), guarda o último `this` com phase 1..=5; depois de travar, ignora os outros
    // callers. Quem lê o ponteiro está no doc de `GAME_SESSION_DESC`.
    if readable {
        // O getter `ldrsb [x0,#0x84]` é GENÉRICO. A EngagementScreenGameController (o SM que o lever d4=2
        // precisa) tem, no repouso da tela "APERTE ESPAÇO", d4==1 && phase==1 (rest state confirmado,
        // ENGAGEMENT-STATE-MACHINE.md:71; ou d4==4 se o check de sessão passa). Os callers ESPÚRIOS têm
        // phase=3/d4≠1 e sobrescreviam a captura → o lever escrevia no objeto errado (bug pós-reboot
        // 2026-07-15). TRAVA na engagement assim que a vê; depois disso ignora os espúrios (o ponteiro
        // é estável — mesmo que o d4 dela avance 1→2→3 pelo lever, mantemos ELA capturada).
        let d4_now = (this.add(0xd4) as *const u8).read();
        if fase == Some(FaseDaSessao::Engagement) && (d4_now == 1 || d4_now == 4) {
            GAME_SESSION_DESC.store(this, Ordering::Relaxed);
            ENGAGEMENT_SM_LOCKED.store(true, Ordering::Relaxed);
        } else if !ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed) && fase.is_some() {
            GAME_SESSION_DESC.store(this, Ordering::Relaxed); // fallback antigo, só até travar na engagement
        }
        // DISPARO DIRETO do lever (2026-07-15): o timer redscript (OnBwmsFireStart, 8s) NÃO estava
        // armando pós-reboot — o @wrapMethod OnInitialize da EngagementScreenGameController não dispara
        // (eng=false no phasedbg), então BwmsFireStart nunca era chamado. Com o SM certo agora capturado
        // (repouso d4==1 && phase==1), o dylib dispara o lever ELE MESMO, sem depender do redscript.
        // DWELL por WALL-CLOCK (2026-07-19): espera ~1.5s de repouso estável antes de disparar. Contar
        // chamadas do getter não mede tempo: ele não roda por frame (≤28 chamadas por boot em 334 logs de
        // proofs/). fire_lever_direct() re-checa TODO o gate (state/phase/guard/ctx/gate572) + dead-man
        // no instante do disparo.
        // Sem `pular_intro` quem dispensa a engagement é o jogador, no espaço.
        if skipintro_enabled()
            && fase == Some(FaseDaSessao::Engagement)
            && d4_now == 1
            && ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed)
            && !LEVER_FIRED_DIRECT.load(Ordering::Relaxed)
        {
            let since = LEVER_REST_SINCE.get_or_init(std::time::Instant::now);
            if since.elapsed().as_millis() >= 1500 && crate::inicio::natives::fire_lever_direct() {
                LEVER_FIRED_DIRECT.store(true, Ordering::Relaxed);
            }
        }
    }
    // DIAGNÓSTICO: loga as PRIMEIRAS 40 chamadas + TODA mudança de fase depois. Não mente.
    let eng = crate::terminal::overlay::engagement_active();
    {
        let n0 = PHASE_DIAG_HITS.fetch_add(1, Ordering::Relaxed);
        // +0xd4 = estado INTERNO da sessão; +0xe0 sub-estado; fd/fe/ff flags. Fingerprint p/ detectar avanço.
        let (d4, e0, fd, fe, ff) = if readable {
            (
                (this.add(0xd4) as *const u8).read(),
                (this.add(0xe0) as *const u8).read(),
                (this.add(0xfd) as *const u8).read(),
                (this.add(0xfe) as *const u8).read(),
                (this.add(0xff) as *const u8).read(),
            )
        } else {
            (255, 255, 255, 255, 255)
        };
        // fingerprint = phase byte + estado interno + flags fd/fe/ff (o SPACE pode setar um destes que
        // arma o save-system e o d4=4 não). Loga em toda mudança de QUALQUER campo.
        let fp = (real as i64)
            | ((d4 as i64) << 8)
            | ((e0 as i64) << 16)
            | ((fd as i64) << 24)
            | ((fe as i64) << 32)
            | ((ff as i64) << 40);
        let changed = PHASE_LAST_LOGGED.swap(fp, Ordering::Relaxed) != fp;
        if n0 < 40 || changed {
            crate::log_dev!(
                "[phasedbg] getter#{n0} phase={real} d4={d4} e0={e0} fd={fd} fe={fe} ff={ff} eng={eng} this={this:p}"
            );
        }
    }
    // Não mente a phase: pular a engagement por completo (1->3 OU 1->2) trava o boot em 99% — a
    // engagement é um PASSO DE LOADING OBRIGATÓRIO (é onde o conteúdo/save-system inicializa). Provado
    // in-game (2026-07-04). O skip da engagement é MOSTRAR+DISMISSAR: a alavanca d4=2 no repouso da tela
    // (disparo direto acima, ou `BwmsFireStart` pelo reds), com `pular_intro`.
    let _ = eng; // sem `devlog` o `log_dev!` acima é no-op e `eng` ficaria sem uso
    // WATCHDOG: registra a MAIOR phase já vista (boot sadio passa de 1 em segundos) + tick do
    // watchdog. O getter só roda nas transições (≤28 chamadas por boot em 334 logs de proofs/); o tick
    // contínuo é o `cp77_tick` (lib.rs). Se o `cp77_tick` tica na tela preta de um boot travado na
    // engagement: [não medido] — log 1/s no `boot_hang_watchdog` vindo do `cp77_tick` num boot que trave
    // ali. Ver `boot_hang_watchdog`.
    {
        // `PHASE_MAX_SEEN` escolhe o ramo do watchdog; o dwell anda quando o máximo sobe OU quando a fase do SM da engagement muda.
        let prev = PHASE_MAX_SEEN.load(Ordering::Relaxed);
        let subiu = (real as i64) > prev;
        if subiu {
            PHASE_MAX_SEEN.store(real as i64, Ordering::Relaxed);
        }
        let do_sm = ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed) && this == GAME_SESSION_DESC.load(Ordering::Relaxed);
        let sm_andou = do_sm && FASE_DO_SM.swap(real as i64, Ordering::Relaxed) != real as i64;
        if subiu || sm_andou {
            if let Some(t) = BOOT_START.get() {
                let s = t.elapsed().as_secs();
                PHASE_MAX_CHANGED_ELAPSED.store(s, Ordering::Relaxed);
                if sm_andou {
                    crate::log_dev!("[skipintro] SM da engagement: fase {real} em t={s}s");
                    #[cfg(feature = "devtools")]
                    if fase == Some(FaseDaSessao::Engagement) {
                        simular_janela_oculta();
                    }
                }
            }
        }
    }
    boot_hang_watchdog();
    // A fase sai de 5 na volta ao menu (5 -> 3 no boot 311, `proofs/20260919-boot311-console-completo.log:2767-2854`; volta ao menu pedida em
    // `:3464`), e aí a cena capturada no `OnWorldAttached` é do mundo que saiu. Troca de sessão sem fase fora de 5 e o momento do
    // `OnWorldAttached` numa sessão seguinte são [não medido] (`register::cena_base`).
    // Antes de publicar a fase: quem lê `fase_lida() == Jogo` (Acquire) já acha o player do mundo que sai zerado.
    if troca_de_mundo(FASE_LIDA.load(Ordering::Relaxed), real as i64) {
        trocar_de_mundo();
    }
    let fase_antes = FaseDaSessao::do_byte(FASE_LIDA.swap(real as i64, Ordering::AcqRel));
    if fase_antes == Some(FaseDaSessao::Jogo) && fase != Some(FaseDaSessao::Jogo) {
        crate::mundo::IGS_OWA_SCENE_PTR.store(0, Ordering::Relaxed);
    }
    if fase == Some(FaseDaSessao::Jogo) {
        LEITURAS_PHASE5_DO_MUNDO.fetch_add(1, Ordering::AcqRel);
        PHASE_REACHED_5.store(true, Ordering::Relaxed);
        let _ = PHASE5_AT.set(std::time::Instant::now());
        if let Ok(mut ultimo) = PHASE5_ULTIMO_AT.lock() {
            *ultimo = Some(std::time::Instant::now());
        }
        if LEITURAS_PHASE5.fetch_add(1, Ordering::Relaxed) == 2 {
            ASSENTO_CALLS.store(CALLS.load(Ordering::Relaxed), Ordering::Relaxed);
            let _ = FIM_DO_LOAD_AT.set(std::time::Instant::now());
            // O auto-load passou do load inteiro: desarma o dead-man's switch aqui, não na 1ª leitura (load só enfileirado). Crash no meio
            // do load deixa o marcador, e o boot seguinte cai no menu (modo seguro). Load sem 3ª leitura: o marcador fica até o exit().
            clear_boot_attempt_marker();
            crate::inicio::energia::apagar();
            crate::veredito_dev!(
                "[splash] fim do load (3ª leitura de phase 5) em t={}s, {}s depois do 1º phase 5",
                boot_elapsed_secs(),
                PHASE5_AT.get().map_or(0, |t| t.elapsed().as_secs())
            );
        }
    }
    real
}

/// Hooka o getter da phase byte com instalar_perto, nos dois modos de boot: é dele que saem o fim do load e o `gameplay_real` (OnApply,
/// journals, fila do console, entrada no jogo), e sem `pular_intro` eles ficavam mortos. Só a alavanca da engagement depende de `pular_intro`
/// (dentro do getter). No-op se alvo ilegível / prólogo inesperado / dist>128MB — nunca crasha.
unsafe fn install_phase_skip_near4() {
    let target = crate::rebase(0x1_015d_2dec);
    // GUARDA REAL: o prólogo do getter da phase byte é `ldrsb w0,[x0,#0x84]` (0x39c21000) + `ret` (0xd65f03c0). Num binário onde o
    // endereço caiu no lugar errado (não é o Steam 2.5.0), esses bytes NÃO batem → recusa limpa em vez de sobrescrever 8 bytes de
    // uma função não-relacionada (que crasharia quando ela rodasse). `is_readable` sozinho não pega isso.
    if !crate::memoria::is_readable(target as *const c_void, 8) {
        crate::log_dev!("[skipintro] getter ilegível -> sem skip (sem crash)");
        // Sem getter não há lever: a engagement vai esperar a tecla, e o splash por cima dela esconderia o prompt até o watchdog.
        boot_splash_off();
        BINK_RELEASED.store(true, Ordering::Relaxed);
        return;
    }
    let i0 = (target as *const u32).read_unaligned();
    let i1 = (target as *const u32).add(1).read_unaligned();
    if i0 != 0x39c2_1000 || i1 != 0xd65f_03c0 {
        crate::log_dev!(
            "[skipintro] prólogo do getter não bate ({i0:#010x}/{i1:#010x}, esperado 0x39c21000/0xd65f03c0) -> sem skip (versão do jogo incompatível?)"
        );
        // Sem getter não há lever: a engagement vai esperar a tecla, e o splash por cima dela esconderia o prompt até o watchdog.
        boot_splash_off();
        BINK_RELEASED.store(true, Ordering::Relaxed);
        return;
    }
    if crate::ganchos::instalar_perto(target, phase_skip_getter as *mut c_void, Some(&PHASE_GET_ORIG)).is_some() {
        crate::log_dev!("[skipintro] getter da phase byte HOOKADO @ {target:p} via instalar_perto (1/2->3, telas de espaço -> menu)");
        return;
    }
    // `instalar_perto` (± B de 128MB, incl. seu próprio landing-pad de ±128MB) recusa quando o slide da imagem deixa o alvo além de
    // ±128MB do dylib E o kernel não aloca memória nova naquela vizinhança — no Steam 2.5.0 é o caso da maioria dos boots (12 de 15
    // consoles de 2026-10-18 a 10-23). `instalar_com_ponte` usa ADRP+BR (±4GB de alcance) com o landing-pad num `mmap` comum (sem
    // MAP_FIXED, sempre sucede em QUALQUER endereço).
    match crate::ganchos::instalar_com_ponte(target, phase_skip_getter as *mut c_void, Some(&PHASE_GET_ORIG)) {
        Some(_) => {
            crate::log_dev!("[skipintro] getter da phase byte HOOKADO @ {target:p} via instalar_com_ponte (instalar_perto recusou: alvo fora de ±128MB)");
        }
        None => {
            // Os 2 métodos recusaram → o skip da phase não vai acontecer. MAS o splash já foi armado
            // e o bink-skip já está falhando os opens → sem isto, tela preta eterna. Desiste com graça
            // NA HORA (não espera o watchdog de 75s): tira o splash + libera o bink pro boot nativo.
            crate::log_dev!("[skipintro] getter RECUSOU nos 2 métodos (near4 e adrp_br8) -- sem skip; liberando vídeo/engagement nativo (sem tela preta)");
            boot_splash_off();
            BINK_RELEASED.store(true, Ordering::Relaxed);
        }
    }
}

/// Trampolim do `BinkOpenWithOptions` original (publicado por `ganchos::instalar_perto` em
/// `install_bink_skip`), chamado por `bink_open_replacement`.
static ORIG_BINK_SKIP: AtomicU64 = AtomicU64::new(0);

// ===== SAÍDA LIMPA: elimina a janela de erro ao fechar o jogo =====
// O binário do jogo carrega o `RED4ext.dylib` (port C++, LC_LOAD do setup CPVR/memaxo). No
// `exit()`, `__cxa_finalize_ranges` roda o `RED4extShutdown`, que desfaz os detours dele e dá
// SIGSEGV em `DetourTransaction::Abort()` → o macOS abre "Cyberpunk2077 quit unexpectedly"
// (2 crash-reports idênticos confirmam: bug determinístico). O nosso runtime NÃO depende do
// RED4ext (otool -L: 0 link). Fix: interceptar o `exit` da libc e encerrar por syscall direto
// (SYS_exit, ver `exit_replacement`), que termina o processo SEM rodar o `__cxa_finalize`
// (portanto sem o shutdown que crasha). O `exit`
// só é chamado no encerramento, então não há efeito em runtime; settings do jogo são gravados na
// hora do Apply (não no finalize), então nada se perde. Idempotente (roda 1x).
static EXIT_HOOKED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn exit_replacement(status: i32) {
    // `cet-lifecycle-events` (onShutdown): dispara ANTES de qualquer coisa — o motor ainda está
    // 100% intacto aqui (é POR ISSO que este hook existe: interceptamos `exit()` ANTES do
    // `__cxa_finalize_ranges`/RED4extShutdown desmontarem qualquer coisa). `fire_event` é o MESMO
    // mecanismo já provado em Input/Key, Session/Ready, Overlay/Open/Close — só percorre a
    // registry e chama via `call_func` (nenhuma escrita/lock novo). `catch_unwind` por segurança
    // (um mod que panica aqui não pode impedir a saída limpa, que é o objetivo #1 deste hook).
    // Só se nenhuma fonte entregou o `End` desta sessão (o `OnWorldDetached` do jogo, `inicio::sessao_do_jogo`, ou o despawn da reserva).
    let n = if crate::inicio::sessao_do_jogo::marcar_fim() {
        let n = std::panic::catch_unwind(|| unsafe { crate::tipos_do_motor::fire_event("Session/End") }).unwrap_or(0);
        crate::veredito_dev!("[sessao-reserva] Session/End (exit) → {n} callback(s)");
        n
    } else {
        0
    };
    crate::log_dev!("[cleanexit] Session/End (onShutdown) -> {n} callback(s)");
    // red4ext-gamestates-add: Shutdown.OnEnter (type=3) — exit() hookado = entrando em Shutdown
    let _ = std::panic::catch_unwind(|| crate::mods_do_usuario::api::call_game_state_enter(EstadoDoJogo::Shutdown));
    // Via ALTERNATIVA (2026-07-18): `fire_event`/CallbackSystem exige um TARGET (ref<IScriptable>)
    // vivo — mas em quit real o mundo/player já foram desmontados antes do exit() rodar (achado
    // desta mesma sessão, ver proof onshutdown-mechanism-safe-NAOfechado). Uma FUNÇÃO GLOBAL não
    // depende de nenhuma instância — mesmo padrão já provado de `BwmsPluginOnUpdate` (lib.rs,
    // resolvida 1x via get_function, chamada sem `self`). Mod declara
    // `public static func BwmsOnGameShutdown() -> Void` e o core chama direto, sem precisar de
    // objeto sobrevivente. Resolve a Registry aqui mesmo (não temos `reg` neste escopo).
    let g = std::panic::catch_unwind(|| unsafe {
        let reg = crate::rtti::Registry::obtain()?;
        let f = crate::register::get_function(&reg, "BwmsOnGameShutdown");
        if !crate::rtti::sane(f) {
            return None;
        }
        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[]).map(|_| ())
    })
    .unwrap_or(None);
    crate::log_dev!(
        "[cleanexit] BwmsOnGameShutdown (onShutdown, funcao global) -> {}",
        if g.is_some() { "chamada OK" } else { "nao achada/nao chamou" }
    );
    // RED4ext.SDK `#45` (`EMainReason::Unload`): sinaliza plugins Rust ANTES do processo morrer
    // (mesmo timing/motivo dos 2 disparos acima — motor ainda intacto aqui). Isolado por plugin.
    let _ = std::panic::catch_unwind(crate::mods_do_usuario::plugins::unload_all_plugins);
    // Desarma o dead-man's switch do lever (ver inicio/natives.rs::tramp_fire_start_state): chegar
    // aqui prova que o processo teve um exit() de verdade (não crash/hang/kill -9), então o
    // próximo boot pode disparar o lever normalmente de novo.
    clear_boot_attempt_marker();
    crate::inicio::energia::apagar();
    crate::log_dev!(
        "[cleanexit] exit({status}) interceptado -> exit syscall (pula finalize/RED4extShutdown)"
    );
    // Encerramento DIRETO por syscall (macOS arm64: x16=1 = SYS_exit, x0=status, svc #0x80).
    // NÃO usar o `_exit` da libc por FFI: o símbolo `_exit` no Mach-O é ambíguo e reentrava no
    // próprio `exit` já hookado — o processo não terminava e travava girando (183% CPU, provado
    // por sample). O syscall termina o processo inteiro na hora, sem libc, sem reentrância.
    #[cfg(target_arch = "aarch64")]
    core::arch::asm!(
        "mov x16, #1",
        "svc #0x80",
        in("x0") status,
        options(noreturn, nostack),
    );
    #[cfg(not(target_arch = "aarch64"))]
    std::process::abort();
}

unsafe fn install_clean_exit() {
    if EXIT_HOOKED.swap(true, Ordering::Relaxed) {
        return; // já instalado
    }
    use crate::macos::{dlsym, RTLD_DEFAULT};
    let addr = dlsym(RTLD_DEFAULT, b"exit\0".as_ptr() as *const i8);
    if addr.is_null() {
        crate::log_dev!("[cleanexit] exit nao resolveu (dlsym) — sem fix");
        return;
    }
    match crate::ganchos::instalar(addr, exit_replacement as *mut c_void) {
        Some(_orig) => {
            crate::log_dev!(
                "[cleanexit] hook em exit @ {addr:p} -> exit por syscall (saida limpa, sem janela de erro)"
            );
        }
        None => crate::log_dev!("[cleanexit] FALHA ao hookar exit"),
    }
}

unsafe fn install_bink_skip() {
    // `pular_intro` desligado = no-op (boot normal com a intro).
    if !skipintro_enabled() {
        return;
    }
    use crate::macos::{dlsym, RTLD_DEFAULT};
    let addr = dlsym(RTLD_DEFAULT, b"BinkOpenWithOptions\0".as_ptr() as *const i8);
    if addr.is_null() {
        crate::log_dev!("[skipintro] BinkOpenWithOptions nao resolveu (dlsym) — sem skip");
        return;
    }
    // `instalar_perto`, NÃO `replace` (2026-09-11): `BinkOpenWithOptions` é UMA instrução
    // (`0x95d4: b 0x7de4` no libBink2MacArm64) e `BinkCopyToBuffer` começa em `0x95d8`. O `replace`
    // grava 16 bytes e destruía os 3 primeiros comandos da vizinha. Nada chama `BinkCopyToBuffer` hoje
    // (o jogo não a importa; 0 desvios internos no dylib), então era latente — mesma classe do SIGILL
    // do getter da phase-byte. O relocador trata o `B` roubado.
    match crate::ganchos::instalar_perto(addr, bink_open_replacement as *mut c_void, Some(&ORIG_BINK_SKIP)) {
        Some(_) => {
            crate::log_dev!("[skipintro] hook em BinkOpenWithOptions @ {addr:p} (falha open de boot)");
        }
        None => crate::log_dev!("[skipintro] FALHA ao hookar BinkOpenWithOptions"),
    }
}

// ===== F-B: PONTE redscript→native (hook do bind orchestrator) =====
// O redscript binda `native func` no load (~6s); native não-registrada → crash depois (executor
// com regIndex lixo). O bind orchestrator @0x101113e44 monta o bind e DEPOIS entra no resolve-loop
// @0x101114148. Hookar a ENTRADA + register_all antes da original → BlackwallPing no RTTI antes do
// binder procurar (resolve limpo). RTTI já vivo aqui (Get lazy/idempotente; o binder usa o mesmo
// RegisterFunction vtbl+0xA0). Prólogo limpo (sub sp/stp, zero PC-rel). Disasm verificado.
// #1 (orchestrator @0x101113e44) NÃO disparou (entrada mid-função pelo resolve-loop). #2 @0x1011a8e58
// é a OUTRA fn que loga "Missing native global function" (resolve de global-native), entry limpo.
const BIND_ORCH_VM: u64 = 0x1_011a_8e58;
/// `sub sp,sp,#0x70` (d101c3ff) + `stp x22,x21,[sp,#0x40]` (a90457f6), LE como u64.
const BIND_ORCH_PROLOGUE: u64 = 0xa904_57f6_d101_c3ff;
static ORIG_BIND_ORCH: AtomicU64 = AtomicU64::new(0);
static FB_REGISTER_DONE: AtomicBool = AtomicBool::new(false);
static FB_IN_REGISTER: AtomicBool = AtomicBool::new(false);
static ORCH_CALLS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn bind_orch_hook(
    x0: *mut u8,
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    // DIAGNÓSTICO: loga as 3 primeiras chamadas (prova se o hook dispara de todo). Gated dev_mode.
    let c = ORCH_CALLS.fetch_add(1, Ordering::Relaxed);
    let diag = crate::dev_mode();
    if diag && c < 3 {
        crate::log_dev!("[fb] bind orch CHAMADO #{c} x0={x0:p}");
    }
    // register_all UMA vez, ANTES da original (RTTI vivo). Guard anti-recursão: register_all só
    // toca Get + RegisterFunction (não chama o executor/bind), mas o swap garante zero loop.
    if !FB_REGISTER_DONE.load(Ordering::Acquire) && !FB_IN_REGISTER.swap(true, Ordering::AcqRel) {
        crate::register::register_all();
        crate::mods_do_usuario::pipeline::carregar_plugins();
        FB_REGISTER_DONE.store(true, Ordering::Release);
        FB_IN_REGISTER.store(false, Ordering::Release);
        crate::log_dev!("[fb] bind orch entry: register_all feito (BlackwallPing no RTTI antes do bind)");
    }
    let orig = ORIG_BIND_ORCH.load(Ordering::Relaxed) as *mut c_void;
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, usize, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3, x4, x5, x6, x7);
    // Tentativa 12 (2026-07-13) — este é o handler de kind==5 (função global) no dispatcher de
    // validação (0x1011a7e28), confirmado por endereço idêntico ao BIND_ORCH_VM já hookado desde
    // a saga F-B. Com classes (kind=1, 2843/2843) e enums (kind=0, 5/5) 100% validados após os
    // fixes de hoje, a falha residual que ainda derruba o boot deve estar aqui (função global não
    // registrada a tempo) ou em kind=3/4 (ainda não instrumentados). Log leve só nas falhas.
    if diag && ret == 0 {
        let x1p = x1 as *const u8;
        let name_hash = if crate::memoria::is_readable(x1p.add(8) as *const c_void, 8) {
            Some(core::ptr::read_unaligned(x1p.add(8) as *const u64))
        } else {
            None
        };
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::log_dev!(
            "[fb] bind orch (kind=5/global-func) FALHOU pra '{name}' (hash={:#018x}) x1={x1:#x}",
            name_hash.unwrap_or(0)
        );
    }
    ret
}

/// PRODUÇÃO 2026-07-13: era gated `~/.bwms-bind-bridge` (marker de DEV). Um boot de verdade
/// com zero markers (simulando o usuário final) confirmou que, mesmo com o fix da Facade
/// (classvalidate incondicional) já ativo, o boot AINDA crashava — porque `register_all()`
/// (que registra `BwmsAutoContinue`/`BwmsSkipIntroOn`/etc., natives GLOBAIS declaradas nos
/// nossos próprios `.reds` JÁ SHIPADOS) só rodava tarde demais (tick, ~30s) sem esta ponte
/// cedo. Agora incondicional; o log verboso continua atrás de `dev_mode()` (ver a fn).
pub(crate) unsafe fn install_bind_bridge() {
    let target = crate::rebase(BIND_ORCH_VM);
    if !crate::memoria::is_readable(target as *const c_void, 8) {
        crate::log_dev!("[fb] bind orch ilegível -> sem ponte");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != BIND_ORCH_PROLOGUE {
        crate::log_dev!("[fb] bind orch não casou ({got:#018x}) -> sem ponte");
        return;
    }
    match crate::ganchos::instalar_com_threads_paradas(target, bind_orch_hook as *mut c_void, Some(&ORIG_BIND_ORCH)) {
        Some(_) => {
            crate::log_dev!("[fb] ponte redscript→native instalada (bind orch @ {target:p})");
        }
        None => crate::log_dev!("[fb] FALHA ao hookar o bind orchestrator"),
    }
}

// ===== F4 (Facade/CallbackSystem/Reflection): validador de CLASSE nativa — forja das classes do BWMS =====
// Achado 2026-07-12 via a MESMA técnica do F-B (xref de string de erro do bind, não busca por
// símbolo): a família de strings "Missing native class '%hs'"/"Missing native function '%hs' in
// native class '%hs'" tem xrefs concentrados ao lado do BIND_ORCH_VM (0x1011a8e58, já hookado pra
// função GLOBAL). Mapeando os limites de função (prólogo/epílogo com stack-canary) achei
// 0x1011a84b4 = função DIFERENTE, ainda não hookada — o validador de CLASSE (itera funcs+props
// de um "class descriptor" com offsets que não batem com o CClass real; hipótese: struct do
// bundle/compilador, não da RTTI nativa). É provavelmente quem emite o erro que crasha o boot
// quando o .reds da Facade é deployado (ver [[cp77-codeware-port]] pra RE completa).
//
// O hook (`class_validate_probe_hook`) é produção, instalado sem condição no `on_load` (lib.rs): na
// 1ª chamada roda `fase_de_tipos` + `carregar_plugins`; em cada chamada forja/registra a classe do
// BWMS que o validador está visitando (Facade `Codeware`, `CallbackSystem`, alvos, TweakXL...) antes
// da original — sem a forja da `Codeware` o validador reprova e o boot cai no assert
// `baseEngineInit.cpp:1094`. Sempre
// chama o trampolim com os 8 registradores inalterados. Só o log/diagnóstico fica atrás de `dev_mode()`.
const CLASS_VALIDATE_VM: u64 = 0x1_011a_84b4;
/// `sub sp,sp,#0xd0` (d10343ff) + `stp x28,x27,[sp,#0x70]` (a9076ffc), LE como u64.
const CLASS_VALIDATE_PROLOGUE: u64 = 0xa907_6ffc_d103_43ff;
static ORIG_CLASS_VALIDATE: AtomicU64 = AtomicU64::new(0);
static CLASSVAL_CALLS: AtomicUsize = AtomicUsize::new(0);

/// CName de "Codeware" pré-computado 1x (evita custo por chamada; a função dispara pra CADA
/// classe do bundle durante o bind — dezenas+ vezes por boot).
static CODEWARE_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static CODEWARE_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-callbacksystem-rtti` (2026-07-13) — mesma técnica da Facade, classe diferente
/// (`CallbackSystem extends IGameSystem`). Guard próprio, mesmo padrão de `CODEWARE_REGISTERED`.
static CALLBACKSYSTEM_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
/// `inkWidget` é VANILLA (não forjada por nós), mas passa pelo `class_validate_probe_hook` (o hook intercepta a validação de QUALQUER classe do
/// bind): a visita dele dispara o registro do `#96` em `inkUserData`.
static INKWIDGET_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static INKWIDGET_REGISTERED: AtomicBool = AtomicBool::new(false);
static CALLBACKSYSTEM_REGISTERED: AtomicBool = AtomicBool::new(false);
// `cw-rawinput-realname` — 2 tentativas, 2 crashes, ambas revertidas (2026-07-13). Ver
// `blackwall-mods-dev/ponte-callbacksystem.reds` pro histórico completo.

type Registrar = unsafe fn(&crate::rtti::Registry) -> String;
/// Uma linha de `CLASSES_FORJADAS`: (nome no bind, registrador, `register_type_min` antes do registrador, tag do
/// `diag_class_forge_prediction`, rótulo do log do resultado, texto do log de detecção, loga `Registry::obtain` nulo). A flag de 1x
/// da linha mora em `CLASSES_FORJADAS_REGISTRADAS`, no mesmo índice.
type LinhaDeForja = (&'static str, Registrar, bool, Option<&'static str>, &'static str, &'static str, bool);
/// Textos do log de detecção: `[classval-probe] classe '<nome>' detectada no validador — <texto>`.
const AVISO_FORJA_E_REGISTRO: &str = "forjando+registrando AGORA";
const AVISO_SO_FORJA: &str = "forjando AGORA";
const AVISO_FABRICA_ESTATICA: &str = "forjando+registrando AGORA (is_static factory, PROVADO 2026-08-05)";
const AVISO_SO_REGISTRO: &str = "registrando AGORA (classe vanilla, sem forja)";
const N_CLASSES_FORJADAS: usize = 38;
/// Classes que o `class_validate_probe_hook` forja pela mesma regra: na visita do validador à classe (`name_hash` igual ao hash do
/// nome), `swap` da flag da linha em `CLASSES_FORJADAS_REGISTRADAS` ANTES do `Registry::obtain`, depois `register_type_min` quando a
/// 3ª coluna pede, o registrador e o `diag_class_forge_prediction` quando há tag. Codeware e CallbackSystem (diagnóstico próprio) e
/// inkWidget (registro extra do `#96`) têm bloco próprio no hook. Cada chamada do validador traz uma classe só, então no máximo uma
/// linha casa por chamada.
static CLASSES_FORJADAS: [LinhaDeForja; N_CLASSES_FORJADAS] = [
    // 2026-08-05 (achado de auditoria): mesmo padrão da Facade do Codeware, agora pro TweakXL
    // (`TweakXL.Require`/`Version`/`Reload`, nunca portado apesar do "11/11 IMPL" do catálogo —
    // esses métodos são a API de CONTROLE que mods chamam, não o pipeline de aplicação de flats).
    // Declarada em `ponte-fachada-de-tweaks.reds`.
    ("TweakXL", crate::tweakdb::natives::registrar_fachada_de_tweaks, true, None, "registrar_fachada_de_tweaks", AVISO_FORJA_E_REGISTRO, true),
    // `TweakDBManager` (2026-08-08, item TweakXL #44 do `PENDENCIAS-UNIFICADAS.md`) — mesma técnica
    // de forja da Facade (`register_type_min`+`register_method` no validador), classe diferente. É a
    // API real que mods do Nexus importam pra CRUD de TweakDB (`SetFlat`/`CreateRecord`/
    // `UpdateRecord`/`RegisterName`), nunca exposta antes — só o backend nativo existia. Declarada em `ponte-tweakdbmanager.reds`.
    ("TweakDBManager", crate::tweakdb::natives::register_tweakdbmanager, true, None, "register_tweakdbmanager", AVISO_FORJA_E_REGISTRO, true),
    // `TweakDBBatch` (item TweakXL #45) — devolvida por `TweakDBManager.StartBatch()`. Precisa
    // registrar ANTES/independente de `TweakDBManager` porque o validador pode visitar as 2 classes
    // em qualquer ordem (cada `.reds` é seu próprio `native class`). Instanciável, sem `extends` (pai `IScriptable`).
    ("TweakDBBatch", crate::tweakdb::natives::register_tweakdbbatch, false, None, "register_tweakdbbatch", AVISO_FORJA_E_REGISTRO, true),
    // Declarada em `ponte-fachada-de-recursos.reds`.
    ("ArchiveXL", crate::aparencia::registrar_fachada_de_recursos, true, None, "registrar_fachada_de_recursos", AVISO_FORJA_E_REGISTRO, true),
    // Declarada em `bwms-aparencia.reds`.
    ("BwmsAparencia", crate::aparencia::register_bwmsaparencia, true, None, "register_bwmsaparencia", AVISO_FORJA_E_REGISTRO, true),
    // Codeware `#23` (`JobQueue`, 2026-08-14) — mesma receita Facade 100%-estática de `TweakXL`/
    // `TweakDBManager` acima (`register_type_min`+`register_facade_methods`). Ver
    // `tarefas::register_jobqueue_facade` pro escopo/divergência exatos. Declarada em `ponte-job-queue.reds`.
    ("JobQueue", crate::tarefas::register_jobqueue_facade, true, None, "register_jobqueue_facade", AVISO_FORJA_E_REGISTRO, true),
    // `ScriptableTweak` precisa estar no RTTI antes do compilador validar as subclasses dos mods (ex.: Equipment-EX:
    // `PatchCustomItems`/`RegisterOutfitSlots`).
    ("ScriptableTweak", crate::tweakdb::natives::register_scriptabletweak, false, None, "register_scriptabletweak", AVISO_SO_FORJA, false),
    ("CallbackSystemTarget", crate::tipos_do_motor::register_callbacksystemtarget, false, None, "register_callbacksystemtarget", AVISO_SO_FORJA, false),
    // `ComponentTarget extends CallbackSystemTarget` (2026-07-24) — fábricas STATIC (`ID`/`Name`).
    // is_static=true + new_object/make_handle numa classe FORJADA por nós: PROVADO em 2026-08-05 com as 13
    // fábricas da família Target (nota em `aparencia/mod.rs` acima de `COMPONENTTARGET_STATES`; `ID` fica sem
    // prova porque script não constrói CRUID). Guard próprio, mesmo padrão dos demais; registrar SÓ depois de
    // `CallbackSystemTarget` (parent) já ter passado pelo validador.
    ("ComponentTarget", crate::aparencia::register_componenttarget, false, None, "register_componenttarget", AVISO_FABRICA_ESTATICA, false),
    // 6 subclasses NOVAS `extends CallbackSystemTarget` (2026-07-24, rodada 2 — mesma sessão de
    // `ComponentTarget` acima): `ResourceTarget`/`inkWidgetTarget`/`DynamicEntityTarget`/
    // `StaticEntityTarget`/`EntityTarget`/`InputTarget`. As fábricas STATIC delas entram na mesma prova
    // de 2026-08-05 (nota em `aparencia/mod.rs` acima de `COMPONENTTARGET_STATES`). Todas
    // registram SÓ depois de `CallbackSystemTarget` (parent comum, mesmo confirmado lendo cada .reds
    // vendorizado em `enablers/Codeware/scripts/Callback/Targets/`) já ter passado pelo validador.
    // Cada chamada do validador traz uma classe só (`name_hash`), então a ordem é a do motor, não a
    // de `CLASSES_FORJADAS`; hoje ele valida `CallbackSystemTarget` antes das 7 filhas
    // (`proofs/20260919-boot312-console-completo.log`). Filha antes do pai: o `register_*` devolve
    // erro e o `swap` já consumiu a flag da linha em `CLASSES_FORJADAS_REGISTRADAS`, então ela não é
    // forjada (defeito latente, sem correção aqui).
    ("ResourceTarget", crate::arquivos_do_jogo::register_resourcetarget, false, None, "register_resourcetarget", AVISO_FABRICA_ESTATICA, false),
    ("inkWidgetTarget", crate::interface::register_inkwidgettarget, false, None, "register_inkwidgettarget", AVISO_FABRICA_ESTATICA, false),
    ("DynamicEntityTarget", crate::npcs::register_dynamicentitytarget, false, None, "register_dynamicentitytarget", AVISO_FABRICA_ESTATICA, false),
    ("StaticEntityTarget", crate::entidades::register_staticentitytarget, false, None, "register_staticentitytarget", AVISO_FABRICA_ESTATICA, false),
    ("EntityTarget", crate::entidades::register_entitytarget, false, None, "register_entitytarget", AVISO_FABRICA_ESTATICA, false),
    ("InputTarget", crate::entrada::register_inputtarget, false, None, "register_inputtarget", AVISO_FABRICA_ESTATICA, false),
    // `DynamicEntitySystem extends IGameSystem`, instanciável (mesma hierarquia de `CallbackSystem`). O `.reds` dela não pode
    // declarar `module`: `module` + `native func` no mesmo escopo derruba o bind (memória `cp77-redscript-module-plus-nativefunc-crash`).
    ("DynamicEntitySystem", crate::npcs::register_dynamicentitysystem, false, Some("DES"), "register_dynamicentitysystem", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#75` (`StaticEntitySystem`) — implementação NOVA 2026-08-15, módulo IRMÃO do
    // `DynamicEntitySystem` acima, mesma hierarquia (`extends IGameSystem`), mesma receita de forja.
    ("StaticEntitySystem", crate::entidades::register_staticentitysystem, false, Some("SES"), "register_staticentitysystem", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#76` (`StaticEntitySpec`) — implementação NOVA 2026-08-15 (rodada 9), companion struct
    // do `#75` acima. `extends IScriptable` DIRETO (mesma receita de `CallbackSystemHandler`).
    ("StaticEntitySpec", crate::entidades::register_staticentityspec, false, Some("SESPEC"), "register_staticentityspec", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#74` (`DynamicEntitySpec`) — implementação NOVA 2026-08-19, companion struct do `#73`/
    // `DynamicEntitySystem` (acima). Porte direto de `#76`/`StaticEntitySpec` (12 campos em vez de 6,
    // `extends IScriptable` direto, mesma receita).
    ("DynamicEntitySpec", crate::npcs::register_dynamicentityspec, false, Some("DESPEC"), "register_dynamicentityspec", AVISO_FORJA_E_REGISTRO, false),
    ("CallbackSystemHandler", crate::tipos_do_motor::register_callbacksystemhandler, false, None, "register_callbacksystemhandler", AVISO_FORJA_E_REGISTRO, false),
    // `cw-event-target-classes`/`cw-rawinput-realname` (2026-07-18, sessão `handle-ctor-re`,
    // retomada após 2 crashes em 2026-07-15 — ver nota grande em `ponte-callbacksystem.reds`).
    // MESMO padrão de guard por classe; `CallbackSystemEvent` primeiro (parent real de
    // `KeyInputEvent`, forjada como ABSTRATA via `register_type_min`, mesma receita 100% segura já
    // provada 4x hoje), `KeyInputEvent` depois (concreta, `register_type_instantiable_with_parent`
    // com parent = `CallbackSystemEvent` REAL — ao contrário das 2 tentativas de 2026-07-15, que
    // usavam um parent não-nativo ou pulavam a hierarquia real).
    ("CallbackSystemEvent", crate::tipos_do_motor::register_callbacksystemevent, false, None, "register_callbacksystemevent", AVISO_FORJA_E_REGISTRO, false),
    ("KeyInputEvent", crate::entrada::register_keyinputevent, false, None, "register_keyinputevent", AVISO_FORJA_E_REGISTRO, false),
    // `cw-controller-session` (2026-07-18) — `GameSessionEvent extends CallbackSystemEvent`, MESMA
    // receita segura de `KeyInputEvent` (parent nativo real). Dispatch de "Session/Start"/"Session/
    // End" NÃO usa hook de world-attach/detach (categoria que causou a saga de crash do full-body,
    // `SystemsUpdater::Node::LinkJob` — ver memória `cp77-fullbody-fpp-posicao-casaco`) — reusa a
    // detecção de transição de presença do player JÁ SEGURA e JÁ PROVADA (`lib.rs::cp77_tick`, o
    // mesmo bloco que dispara "Player/Spawned"/"Player/Despawned").
    ("GameSessionEvent", crate::inicio::natives::register_gamesessionevent, false, None, "register_gamesessionevent", AVISO_FORJA_E_REGISTRO, false),
    // `cw-controller-entity` (2026-07-18) — `EntityLifecycleEvent extends CallbackSystemEvent`,
    // MESMA receita segura. Dispatch de "Entity/Attach" (nome REAL, `EntityAttachHook.hpp`) NÃO
    // hooka `Raw::Entity::Attach` (função nativa de ALTA FREQUÊNCIA, dispara pra TODA entidade do
    // mundo — hook novo e não-provado neste projeto) — reusa a MESMA transição de presença do
    // player já seguríssima, escopada só ao player (o caso de uso mais comum pra mods como
    // Equipment-EX/Cyberware-EX).
    ("EntityLifecycleEvent", crate::entidades::register_entitylifecycleevent, false, None, "register_entitylifecycleevent", AVISO_FORJA_E_REGISTRO, false),
    // `cw-controller-misc` (2026-07-19) — `ResourceEvent extends CallbackSystemEvent`, MESMA receita
    // segura. Dispatch de "Resource/Load" NÃO hooka `ResourceSerializer::SchedulePostLoadJobs` (RE
    // nova, offset Mac desconhecido) — reusa o hook `resource.link` JÁ INSTALADO E PROVADO
    // (`selftest.rs::reslink_lookup`, dispara em TODA construção real de `ResourcePath`) com um
    // "watch" (`watchres <path>`): quando o hash observado bate, `cp77_tick` dispara o evento
    // edge-triggered, MESMO padrão seguro de `Player/Spawned`/`Session/Start`.
    ("ResourceEvent", crate::arquivos_do_jogo::register_resourceevent, false, None, "register_resourceevent", AVISO_FORJA_E_REGISTRO, false),
    // `cw-event-target-classes` (2026-07-24) — `AxisInputEvent extends KeyInputEvent`,
    // `VehicleLightControlEvent`/`EntityComponentEvent extends EntityLifecycleEvent`. MESMA receita
    // já provada (parent nativo real + `register_type_instantiable_with_parent`).
    ("AxisInputEvent", crate::entrada::register_axisinputevent, false, None, "register_axisinputevent", AVISO_FORJA_E_REGISTRO, false),
    ("VehicleLightControlEvent", crate::veiculos::register_vehiclelightcontrolevent, false, None, "register_vehiclelightcontrolevent", AVISO_FORJA_E_REGISTRO, false),
    ("EntityComponentEvent", crate::aparencia::register_entitycomponentevent, false, None, "register_entitycomponentevent", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#11` (`inkWidgetSpawnEvent`, candidato 6ª rodada 2026-08-11) — mesma receita robusta
    // (parent = `CallbackSystemEvent` real, já forjado).
    ("inkWidgetSpawnEvent", crate::interface::register_inkwidgetspawnevent, false, None, "register_inkwidgetspawnevent", AVISO_FORJA_E_REGISTRO, false),
    // Codeware 1.21.1 `GarageComponent.LoadSummonSetup` (`ponte-vehiclesystem.reds`): classe vanilla, só a native entra antes do bind.
    ("GarageComponent", crate::veiculos::registrar_garagem, false, None, "registrar_garagem", AVISO_SO_REGISTRO, false),
    // ArchiveXL `#47` (`CustomizationExtension`, 2026-08-14, agente offline dedicado) — mesma receita
    // robusta (parent = `CallbackSystemEvent` real, já forjado). Ver `corpo_e_rosto::register_customizationevent`.
    ("CustomizationEvent", crate::corpo_e_rosto::register_customizationevent, false, None, "register_customizationevent", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#25` (`EntityBuilderWrapper`, 2026-08-18 madrugada cont.20) — mesma receita robusta
    // dos eventos acima, mas parent = `IScriptable` (não `CallbackSystemEvent`), mesmo padrão já
    // usado por `CallbackSystemHandler`/`StaticEntitySpec`/`TweakDBBatch`. Ver
    // `entidades::register_entitybuilderwrapper`.
    ("EntityBuilderWrapper", crate::entidades::register_entitybuilderwrapper, false, None, "register_entitybuilderwrapper", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#8`/`#25`/`#130`: `EntityBuilderEvent` (pai `CallbackSystemEvent`) e os wrappers de template e de aparência (pai
    // `IScriptable`); Codeware `#77`: os wrappers de comunidade (pai `IScriptable`).
    ("EntityBuilderEvent", crate::entidades::register_entitybuilderevent, false, None, "EntityBuilderEvent", AVISO_FORJA_E_REGISTRO, false),
    ("EntityBuilderTemplateWrapper", crate::entidades::register_ebw_template, false, None, "EntityBuilderTemplateWrapper", AVISO_FORJA_E_REGISTRO, false),
    ("EntityBuilderAppearanceWrapper", crate::aparencia::register_ebw_appearance, false, None, "EntityBuilderAppearanceWrapper", AVISO_FORJA_E_REGISTRO, false),
    ("CommunityWrapper", crate::npcs::register_communitywrapper, false, None, "CommunityWrapper", AVISO_FORJA_E_REGISTRO, false),
    ("CommunityEntryWrapper", crate::npcs::register_communityentrywrapper, false, None, "CommunityEntryWrapper", AVISO_FORJA_E_REGISTRO, false),
    // Codeware `#176` 176b (`BwmsInkKeyEvent extends inkInputEvent`, 2026-09-13) — mesmo gate por classe, pai nativo `inkInputEvent` e doador
    // `inkCharacterEvent`. Ver `interface::register_bwmsinkkeyevent`.
    ("BwmsInkKeyEvent", crate::interface::register_bwmsinkkeyevent, false, None, "register_bwmsinkkeyevent", AVISO_FORJA_E_REGISTRO, false),
];
/// Hash (CName) de cada linha de `CLASSES_FORJADAS`, calculado na 1ª validação: o validador roda para cada classe do bundle.
static CLASSES_FORJADAS_HASH: std::sync::OnceLock<[u64; N_CLASSES_FORJADAS]> = std::sync::OnceLock::new();
/// Flag de 1x de cada linha de `CLASSES_FORJADAS`, no mesmo índice. Vira `true` na 1ª visita do validador à classe, antes do
/// `Registry::obtain`: a linha forja e registra no máximo uma vez por processo, e um `obtain` nulo nessa visita não é repetido.
static CLASSES_FORJADAS_REGISTRADAS: [AtomicBool; N_CLASSES_FORJADAS] = [const { AtomicBool::new(false) }; N_CLASSES_FORJADAS];

/// O `CallbackSystem` do Codeware que o jogo registra sob os mesmos nomes de script (2.5: `gameCallbackSystem` como `CallbackSystem` e os
/// irmãos), pai antes de filho. O nome de script passa a apontar pra classe do BWMS: o handler do jogo (`AddTarget`, 2.5 Mac `0x10504d748`)
/// chama a virtual `+0x120` do alvo e a comparação de alvos (`0x10504e0d4`) a `+0x118`, que os alvos forjados do BWMS (vtable do doador) não
/// têm; o `RegisterCallback` do jogo (`0x105047ad4`) só ativa evento com controlador C++ no mapa `sys+0x80`. Os eventos seguem a família: os
/// getters do jogo leem campos C++ fixos (`GameSessionEvent.IsPreGame` `+0x48`, `IsRestored` `+0x49`, `0x10504eea4`/`0x10504ee88`) que o
/// objeto do BWMS não tem.
const FAMILIA_DO_CALLBACKSYSTEM: [&str; 9] = [
    "CallbackSystemTarget",
    "CallbackSystemEvent",
    "CallbackSystemHandler",
    "CallbackSystem",
    "ResourceTarget",
    "InputTarget",
    "GameSessionEvent",
    "ResourceEvent",
    "KeyInputEvent",
];
/// Nome nativo que o jogo registrou pro nome de script `nome`, quando é outro (o mapa script→nativo do `CRTTISystem`, `+0x150`, lido pelo
/// `ConvertScriptToNativeName` do slot `+0x120`) e a classe existe.
unsafe fn classe_do_jogo_pelo_nome(reg: &crate::rtti::Registry, nome: &str) -> Option<*mut c_void> {
    let script = crate::cname::cname(nome);
    let nativo = reg.convert_script_to_native_name(script);
    if nativo == 0 || nativo == script {
        return None;
    }
    let cls = reg.class_by_hash(nativo);
    rtti::sane(cls).then_some(cls)
}

/// Decide no RTTI vivo, na 1ª classe validada: sem `CallbackSystem` do jogo (2.31) nada muda e a forja segue pelo validador. Com ele, forja
/// agora a família do BWMS (o validador resolve a classe nativa pelo nome de script, slot `+0x108`, 2.5 Mac `0x1011a84f8`), religa cada nome
/// de script pelo `RegisterScriptName` (slot `+0x100`, `0x1010bfb78`, que sobrescreve os dois mapas) e liga o getter do jogo ao singleton.
pub(crate) unsafe fn religar_familia_do_callbacksystem(reg: &crate::rtti::Registry) {
    static FEITO: AtomicBool = AtomicBool::new(false);
    if FEITO.swap(true, Ordering::AcqRel) || classe_do_jogo_pelo_nome(reg, "CallbackSystem").is_none() {
        return;
    }
    let linha = |nome: &str| CLASSES_FORJADAS.iter().position(|l| l.0 == nome);
    for nome in FAMILIA_DO_CALLBACKSYSTEM {
        let Some(do_jogo) = classe_do_jogo_pelo_nome(reg, nome) else {
            continue;
        };
        let r = if nome == "CallbackSystem" {
            CALLBACKSYSTEM_REGISTERED.store(true, Ordering::Release);
            crate::tipos_do_motor::register_callbacksystem(reg)
        } else if let Some(i) = linha(nome) {
            CLASSES_FORJADAS_REGISTRADAS[i].store(true, Ordering::Release);
            (CLASSES_FORJADAS[i].1)(reg)
        } else {
            continue;
        };
        let nossa = reg.class_by_name(nome);
        if !rtti::sane(nossa) {
            crate::log_dev!("[2.5] {nome}: a forja do BWMS falhou, o nome fica com o jogo ({r})");
            continue;
        }
        // O validador recusa classe nativa abstrata cuja declaração de script não é (2.5 Mac `0x1011a883c`); a declaração é a do jogo.
        let flags = (nossa as *mut u8).add(0x70) as *mut u32;
        let abstrata_no_jogo = core::ptr::read_unaligned((do_jogo as *const u8).add(0x70) as *const u32) & 1;
        core::ptr::write_unaligned(flags, (core::ptr::read_unaligned(flags) & !1) | abstrata_no_jogo);
        reg.register_script_name(nome, nome);
        let religada = reg.class_by_script_name(nome) == nossa;
        let campos = match nome {
            "CallbackSystemEvent" => crate::tipos_do_motor::campo_na_cauda(reg, nossa, "eventName", "CName", true),
            "ResourceEvent" => crate::tipos_do_motor::campo_na_cauda(reg, nossa, "resource", "handle:CResource", false),
            _ => Ok(0),
        };
        crate::log_dev!("[2.5] {nome}: religado ao BWMS={religada} ({do_jogo:p} -> {nossa:p}) campos={campos:?} {r}");
    }
    let rotas = crate::tipos_do_motor::rotear_getter_do_callbacksystem(reg);
    crate::log_dev!("[2.5] GameInstance.GetCallbackSystem -> singleton do BWMS: {rotas}");
}

/// Dump de [engine+0x150] (header@+0x14 = size:30|flag:2 packed) pra comparar ANTES/DEPOIS do
/// forge+register do Codeware dentro do próprio class_validate_probe_hook (ver Tentativa 7).
unsafe fn dump_engine_error_container(engine: *const u8, label: &str) {
    if engine.is_null() {
        crate::log_dev!("[classval-probe] {label}: engine nulo/ilegível");
        return;
    }
    let container = engine.add(0x150);
    if !crate::memoria::is_readable(container as *const c_void, 0x30) {
        crate::log_dev!("[classval-probe] {label}: engine={engine:p} [engine+0x150] ilegível");
        return;
    }
    let mut bytes = [0u8; 0x30];
    core::ptr::copy_nonoverlapping(container, bytes.as_mut_ptr(), 0x30);
    let header = u32::from_le_bytes([bytes[0x14], bytes[0x15], bytes[0x16], bytes[0x17]]);
    let size = header & 0x3FFF_FFFF;
    let flag = header >> 30;
    crate::log_dev!(
        "[classval-probe] {label}: engine={engine:p} [engine+0x150]: header@+0x14={header:#010x} (size={size} flag={flag}) bytes={bytes:02x?}"
    );
}

/// Diagnóstico "vai passar no validador original?" — IsKindOf walk (sobe a cadeia de parent de
/// `cls` até bater em IScriptable) + checa se a base DECLARADA no `.reds` (lida de `x1+0x20`)
/// resolve pro MESMO ponteiro que nosso parent forjado + lê `flags@+0x70` da classe. Roda
/// IMEDIATAMENTE após forjar+registrar uma classe, ANTES do validador original (chamado mais
/// abaixo em `class_validate_probe_hook`) — se o boot crashar depois, este log já foi escrito e
/// sobrevive no arquivo. Extraído em 2026-08-06 (cont.26): CallbackSystem e DynamicEntitySystem
/// tinham cada uma sua PRÓPRIA cópia de ~55 linhas idênticas deste bloco (a 2ª surgiu em cont.23
/// copiando a 1ª pra investigar o crash do `module`) — exatamente o tipo de redundância que
/// sobra quando cada gap é tratado como problema isolado em vez de reusar o que já existe.
/// `tag` = prefixo curto nas linhas de log (`"CBS"`/`"DES"`); `class_name` = nome RTTI real (só
/// usado na linha final de `flags@+0x70`, pra manter o texto de log idêntico ao de antes).
unsafe fn diag_class_forge_prediction(reg: &rtti::Registry, tag: &str, class_name: &str, x1: *mut u8) {
    // Só produz log de dev: no público não chama o motor por endereço cru (o getter 0x1011e46d8, sem tradução, seria chamada a nulo no bind).
    if !cfg!(feature = "devlog") {
        return;
    }
    let cls = reg.class_by_name(class_name);
    let igs = reg.class_by_name("IGameSystem");
    crate::log_dev!("[classval-probe] {tag} diag: {class_name}={cls:p} IGameSystem={igs:p}");
    let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_011e_46d8));
    let expected_base = getter(); // sempre IScriptable (universal, confirmado p/ Codeware)
    let mut cur = cls;
    let mut found = false;
    for depth in 0..16u32 {
        if cur.is_null() {
            crate::log_dev!("[classval-probe] {tag} IsKindOf walk: profundidade={depth} cur=NULL — parando");
            break;
        }
        if cur as u64 == expected_base as u64 {
            found = true;
            crate::log_dev!("[classval-probe] {tag} IsKindOf walk: profundidade={depth} cur={cur:p} == expected_base(IScriptable) — ACHOU");
            break;
        }
        if !crate::memoria::is_readable((cur as *const u8).add(0x10) as *const c_void, 8) {
            crate::log_dev!("[classval-probe] {tag} IsKindOf walk: profundidade={depth} cur={cur:p} [cur+0x10] ilegível — parando");
            break;
        }
        let next = core::ptr::read_unaligned((cur as *const u8).add(0x10) as *const *mut c_void);
        crate::log_dev!("[classval-probe] {tag} IsKindOf walk: profundidade={depth} cur={cur:p} parent[+0x10]={next:p}");
        cur = next;
    }
    crate::log_dev!(
        "[classval-probe] {tag} IsKindOf PREVISTO: {}",
        if found { "SUCESSO" } else { "FALHA" }
    );
    if !x1.is_null() && crate::memoria::is_readable(x1.add(SC_PAI) as *const c_void, 8) {
        let decl_base_desc = core::ptr::read_unaligned(x1.add(SC_PAI) as *const *mut u8);
        if decl_base_desc.is_null() {
            crate::log_dev!("[classval-probe] {tag}: sem base declarada explícita (inesperado — o .reds tem extends)");
        } else if crate::memoria::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
            let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
            let getter2: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_010b_3288));
            let singleton = getter2();
            if !singleton.is_null() && crate::memoria::is_readable(singleton as *const c_void, 8) {
                let vt = core::ptr::read_unaligned(singleton as *const *mut u8);
                if !vt.is_null() && crate::memoria::is_readable(vt.add(0x108) as *const c_void, 8) {
                    let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
                    if crate::rtti::sane(slot) {
                        let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
                        let resolved = getclass(singleton, decl_cname);
                        crate::log_dev!(
                            "[classval-probe] {tag} base declarada resolvida={resolved:p} vs nosso parent(IGameSystem)={igs:p} (bate={})",
                            resolved as u64 == igs as u64
                        );
                    }
                }
            }
        }
    }
    if !cls.is_null() && crate::memoria::is_readable(cls as *const c_void, 0x74) {
        let flags = core::ptr::read_unaligned((cls as *const u8).add(0x70) as *const u32);
        crate::log_dev!("[classval-probe] {class_name} flags@+0x70 = {flags:#010x}");
    }
}

/// Erros do bind dos scripts no canal de vereditos, com o texto que o motor monta ("Missing native property 'x' in native class 'Y'",
/// "Imported property ... does not match ..."): o relator chega em `x2` do validador de classe e cada erro sai pelo slot `+0x10` da vtable
/// dele, `(relator, texto, tamanho)`. Sem isto uma falha de bind só aparecia como o SIGTRAP do "Failed to initialize scripts data!"
/// (`0x104d8317c`), sem dizer qual `.reds` nem qual declaração.
static ORIG_RELATOR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn relator_de_erro(relator: *mut u8, texto: *const u8, tamanho: usize, x3: usize, x4: usize, x5: usize, x6: usize, x7: usize) -> usize {
    let n = tamanho & 0xFFFF_FFFF;
    if !texto.is_null() && n > 0 && n < 4096 && crate::memoria::is_readable(texto as *const c_void, n) {
        let s = String::from_utf8_lossy(std::slice::from_raw_parts(texto, n));
        crate::veredito_dev!("[bind] erro do motor: {s}");
    }
    let f: unsafe extern "C" fn(*mut u8, *const u8, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(ORIG_RELATOR.load(Ordering::Relaxed) as usize);
    f(relator, texto, tamanho, x3, x4, x5, x6, x7)
}

/// Põe o gancho no método de erro do relator na primeira validação. Recusa função curta demais pro salto (4 instruções com `ret`/desvio).
unsafe fn ligar_relator_de_erros(relator: *mut u8) {
    static FEITO: AtomicBool = AtomicBool::new(false);
    if relator.is_null() || !crate::memoria::is_readable(relator as *const c_void, 8) || FEITO.swap(true, Ordering::Relaxed) {
        return;
    }
    let vt = core::ptr::read_unaligned(relator as *const *const u8);
    if vt.is_null() || !crate::memoria::is_readable(vt.add(0x10) as *const c_void, 8) {
        return;
    }
    let mut alvo = core::ptr::read_unaligned(vt.add(0x10) as *const *mut c_void);
    if !crate::memoria::is_readable(alvo as *const c_void, 16) {
        return;
    }
    // O slot é um desvio curto (`add x0, x0, #8; b <função>`): o gancho vai na função de verdade.
    for i in 0..2usize {
        let w = core::ptr::read_unaligned((alvo as *const u32).add(i));
        if w & 0xfc00_0000 == 0x1400_0000 {
            let imm = (((w & 0x03ff_ffff) << 6) as i32 >> 6) as isize * 4;
            alvo = (alvo as *mut u8).offset(i as isize * 4 + imm) as *mut c_void;
            break;
        }
    }
    if !crate::memoria::is_readable(alvo as *const c_void, 16) {
        return;
    }
    let curta = (0..4).any(|i| {
        let w = core::ptr::read_unaligned((alvo as *const u32).add(i));
        w == 0xd65f_03c0 || w & 0xfc00_0000 == 0x1400_0000 || w & 0xffff_fc1f == 0xd61f_0000
    });
    if curta {
        return crate::log_dev!("[bind] método de erro do relator curto demais pro gancho ({alvo:p})");
    }
    let ok = crate::ganchos::instalar_com_threads_paradas(alvo, relator_de_erro as *mut c_void, Some(&ORIG_RELATOR)).is_some();
    crate::log_dev!("[bind] erros do motor no log: gancho no relator {alvo:p} ({:#x}) {ok}", crate::un_rebase(alvo));
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn class_validate_probe_hook(
    x0: *mut u8,
    x1: *mut u8,
    x2: *mut u8,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    let c = CLASSVAL_CALLS.fetch_add(1, Ordering::Relaxed);
    ligar_relator_de_erros(x2);
    // A validação das classes roda antes do bind das funções globais (onde o `register_all` entra): propriedade ou classe criada depois
    // dela reprova o bind. A fase de tipos e os plugins (que também criam classes e propriedades) vêm antes da primeira.
    if c == 0 {
        crate::tipos_do_motor::fase_de_tipos();
        crate::mods_do_usuario::pipeline::carregar_plugins();
        if let Some(reg) = crate::rtti::Registry::obtain() {
            religar_familia_do_callbacksystem(&reg);
        }
    }
    marcar_classe_do_teste_validada(x1);
    // tenta ler [x1+8] (CONFIRMADO 2026-07-12: CName do class-descriptor — 'IScriptable'/
    // 'Entity'/'WorldPosition' resolveram certo via cname::resolve_cname) SÓ se legível.
    let name_hash = if !x1.is_null() && crate::memoria::is_readable(x1.add(8) as *const c_void, 8) {
        Some(core::ptr::read_unaligned(x1.add(8) as *const u64))
    } else {
        None
    };
    // PRODUÇÃO 2026-07-13 (achado crítico pós-sessão): este hook rodava INTEIRO atrás de
    // `~/.bwms-classvalidate-probe` (marker de DEV) — incluindo o FORGE que fixa a Facade. Um
    // boot de verdade (zero markers de dev, simulando o usuário final) confirmou: sem o marker,
    // `Codeware` nunca é forjada, o validador reprova, e o boot CRASHA no assert
    // `baseEngineInit.cpp:1094` TODA VEZ — ou seja, o "6/6 PASS"/"Facade fechada" de hoje só
    // valia em configuração de DEV, não no produto real. Fix: o FORGE (mais abaixo) agora roda
    // SEMPRE; só o LOG/DIAGNÓSTICO verboso (stack-walk, dump de registradores, IsKindOf re-walk)
    // continua atrás de `dev_mode()` — não muda o que já foi provado, só destrava pro usuário
    // final sem exigir nenhum marker.
    let diag = crate::dev_mode();
    if diag && c < 12 {
        let shown = name_hash.map(|h| format!("{h:#018x}")).unwrap_or_else(|| "ilegível".into());
        crate::log_dev!(
            "[classval-probe] CHAMADO #{c} x0={x0:p} x1={x1:p} x2={x2:p} [x1+8]={shown}"
        );
    }
    // Tentativa 7 (2026-07-13): a tentativa 6 tinha 2 problemas achados na análise do log real:
    // (a) BUG no filtro de range de heap — usei 0x7000_0000_0000..0x8000_0000_0000 (13 dígitos
    // hex), mas TODO ponteiro heap real observado neste boot tem 10 dígitos (formato 0x7X_XXXX_XXXX,
    // ex. engine=0x7d9e405a30, classe forjada=0x7d9ec86930, x1 do validador=0x79c185fbc0) — o filtro
    // não batia em NADA, dando falso-negativo "zero candidatos" mesmo se o valor certo estivesse lá.
    // Corrigido pra 0x7000000000..0x8000000000. (b) a tentativa 3 já provou que o orquestrador
    // (0x104d7a9f8, dono do x20="engine") DISPARA E RETORNA antes da validação de classe começar —
    // sua stack frame não está mais na cadeia de frame-pointer daqui, então o stack-walk sozinho
    // não tem como achar o x20 dele (árvore errada por construção).
    // NOVA IDEIA (b): x2 é CONSTANTE entre as 12 chamadas de validação (mesmo endereço sempre,
    // 0x16bdadc18 no boot anterior) e bate quase exato com frame#2.fp+0x18 — é o contexto/resultado
    // do LOOP de validação (vive na stack do CHAMADOR, não do orquestrador). Em vez de só logar o
    // valor de x2, dumpar o CONTEÚDO apontado por ele pode revelar um ponteiro salvo pro mesmo
    // objeto "engine" (ou pro container de erro +0x150), sem depender da árvore de chamada do
    // orquestrador. Só na classe 'Codeware' (evita spam nos outros 12). Diagnóstico puro —
    // gated dev_mode (não afeta o forge, que roda incondicional mais abaixo).
    let fachada_de_versao_hash = *CODEWARE_CNAME.get_or_init(|| crate::cname::cname("Codeware"));
    if diag && name_hash == Some(fachada_de_versao_hash) {
        const HEAP_LO: u64 = 0x7000000000;
        const HEAP_HI: u64 = 0x8000000000;
        let mut dump = String::from("[classval-probe] Tentativa 7 — dump *x2 (contexto do loop) + stack-walk (range corrigido):\n");
        if !x2.is_null() && crate::memoria::is_readable(x2 as *const c_void, 0x80) {
            let p2 = x2 as *const u64;
            for i in 0..16usize {
                let v = p2.add(i).read();
                let tag = if v > HEAP_LO && v < HEAP_HI && crate::memoria::is_readable(v as *const c_void, 8) {
                    " <- candidato heap"
                } else {
                    ""
                };
                dump.push_str(&format!("  *x2[{i}] (+{:#x}) = {v:#018x}{tag}\n", i * 8));
            }
        } else {
            dump.push_str("  x2 ilegível ou nulo\n");
        }
        let mut fp: *const u64;
        core::arch::asm!("mov {}, x29", out(reg) fp);
        for depth in 0..6 {
            if fp.is_null() || !crate::memoria::is_readable(fp as *const c_void, 0x10) {
                dump.push_str(&format!("  frame#{depth}: fp ilegível, parando\n"));
                break;
            }
            let saved_fp = fp.read();
            let saved_lr = fp.add(1).read();
            dump.push_str(&format!("  frame#{depth}: fp={fp:p} saved_fp={saved_fp:#018x} saved_lr={saved_lr:#018x}\n"));
            // varre os slots ENTRE este frame e o anterior (callee-saved locals plausíveis),
            // olhando só valores no estilo "ponteiro heap grande" e legíveis como objeto.
            let p = fp as *const u64;
            for slot in 0..12usize {
                let addr = p.wrapping_sub(slot + 1);
                if !crate::memoria::is_readable(addr as *const c_void, 8) {
                    continue;
                }
                let v = addr.read();
                if v > HEAP_LO && v < HEAP_HI && crate::memoria::is_readable(v as *const c_void, 8) {
                    dump.push_str(&format!("    candidato slot-{slot}: {v:#018x}\n"));
                }
            }
            if saved_fp == 0 {
                break;
            }
            fp = saved_fp as *const u64;
        }
        crate::log(&dump);
    }
    // FIX (2026-07-13, tentativa 2): a tentativa 1 (chamar só registrar_fachada_de_versao, que
    // ASSUME a classe já existe) falhou porque a classe REALMENTE não existe ainda neste ponto
    // ("classe não existe"). Desta vez: FORJAMOS a classe NÓS MESMOS via register_type_min
    // (STEP-1 do RegisterType, já provado desde 2026-07-04 — mesmas flags isAbstract|isNative
    // que a declaração real `public abstract native class Codeware`), e SÓ DEPOIS registramos
    // os métodos nativos nela — tudo ANTES de deixar o validador original rodar.
    // ESTE BLOCO É O FIX DE PRODUÇÃO — roda SEMPRE, não só com dev_mode (ver nota no topo da fn).
    if name_hash == Some(fachada_de_versao_hash) && !CODEWARE_REGISTERED.swap(true, Ordering::AcqRel) {
        // Tentativa 7 cont. (2026-07-13): a tentativa 6 provou que *(x2+0x60) É o mesmo ponteiro
        // "engine" que a sonda do orquestrador (0x104d7a9f8) reportou no MESMO boot (valor
        // idêntico, bit-a-bit). Logo dá pra ler [engine+0x150] (o container de erro do assert
        // baseEngineInit.cpp:1094) diretamente daqui, ANTES e DEPOIS do nosso forge+register —
        // se o bit de erro mudar exatamente nessa janela, é prova direta de que O NOSSO
        // REGISTRO É QUEM DISPARA o assert (não uma classe anterior/outro caminho). Diagnóstico
        // puro (leitura sem efeito colateral) — gated dev_mode só pra não gastar ciclos à toa.
        let engine: *const u8 = if diag && !x2.is_null() && crate::memoria::is_readable(x2.add(0x60) as *const c_void, 8) {
            core::ptr::read_unaligned(x2.add(0x60) as *const *const u8)
        } else {
            std::ptr::null()
        };
        if diag {
            dump_engine_error_container(engine, "ANTES do forge+register");
        }
        crate::log_dev!("[classval-probe] classe 'Codeware' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let forged = crate::register::register_type_min(&reg, "Codeware");
            crate::log_dev!("[classval-probe] register_type_min('Codeware') -> {forged:p}");
            let r = crate::tipos_do_motor::registrar_fachada_de_versao(&reg);
            crate::log_dev!("[classval-probe] registrar_fachada_de_versao -> {r}");
        } else {
            crate::log_dev!("[classval-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
        }
        if diag {
            dump_engine_error_container(engine, "DEPOIS do forge+register");
        }
    }
    // Forja das classes de `CLASSES_FORJADAS`. Roda SEMPRE, como o bloco do Codeware acima: classe declarada num `.reds` e não
    // forjada reprova no validador. Uma chamada traz um `name_hash` só, então no máximo uma linha casa e a posição deste laço
    // em relação aos blocos do CallbackSystem e do inkWidget, abaixo, não muda log nem forja.
    let hashes = CLASSES_FORJADAS_HASH.get_or_init(|| std::array::from_fn(|i| crate::cname::cname(CLASSES_FORJADAS[i].0)));
    for ((&(nome, registrar, forjar_min, diag_tag, rotulo, aviso, loga_obtain_nulo), &hash), registrado) in
        CLASSES_FORJADAS.iter().zip(hashes).zip(&CLASSES_FORJADAS_REGISTRADAS)
    {
        if name_hash == Some(hash) && !registrado.swap(true, Ordering::AcqRel) {
            crate::log_dev!("[classval-probe] classe '{nome}' detectada no validador — {aviso}");
            if let Some(reg) = crate::rtti::Registry::obtain() {
                if forjar_min {
                    let forged = crate::register::register_type_min(&reg, nome);
                    crate::log_dev!("[classval-probe] register_type_min('{nome}') -> {forged:p}");
                }
                let r = registrar(&reg);
                crate::log_dev!("[classval-probe] {rotulo} -> {r}");
                if let Some(tag) = diag_tag {
                    diag_class_forge_prediction(&reg, tag, nome, x1);
                }
            } else if loga_obtain_nulo {
                crate::log_dev!("[classval-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
            }
        }
    }
    // `cw-callbacksystem-rtti` (2026-07-13) — MESMO fix da Facade (parent explícito + fullName
    // bare), classe DIFERENTE (`CallbackSystem extends IGameSystem`, instanciável). Bloco
    // ESSENCIAL, roda SEMPRE (não gated por dev_mode) — mesma justificativa de produção do bloco
    // do Codeware acima: se isso rodasse só atrás de marker de dev, um mod real que declare
    // `ponte-callbacksystem.reds` crasharia o boot pra qualquer usuário sem o marker.
    let cbs_hash = *CALLBACKSYSTEM_CNAME.get_or_init(|| crate::cname::cname("CallbackSystem"));
    if name_hash == Some(cbs_hash) && !CALLBACKSYSTEM_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log_dev!("[classval-probe] classe 'CallbackSystem' detectada no validador — forjando+registrando AGORA");
        // Diagnóstico de dev, 1x: qual nome exato o compilador declarou como base de `extends IGameSystem`? Mesma leitura já provada pro Codeware
        // (Tentativa 11 abaixo) — `[x1+0x20]` = descritor da base declarada, `[+0x8]` = CName.
        if cfg!(feature = "devlog") && !x1.is_null() && crate::memoria::is_readable(x1.add(0x20) as *const c_void, 8) {
            let decl_base_desc = core::ptr::read_unaligned(x1.add(0x20) as *const *mut u8);
            if decl_base_desc.is_null() {
                crate::log_dev!("[classval-probe] CallbackSystem: sem base declarada explícita (inesperado — o .reds tem extends)");
            } else if crate::memoria::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
                let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
                let decl_name = crate::cname::resolve_cname(decl_cname);
                crate::log_dev!(
                    "[classval-probe] CallbackSystem base declarada: CName={decl_cname:#018x} nome='{decl_name}' — usar este nome exato como parent"
                );
            }
        }
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::tipos_do_motor::register_callbacksystem(&reg);
            crate::log_dev!("[classval-probe] register_callbacksystem -> {r}");
            // DIAGNÓSTICO (mesma técnica da Tentativa 11, parametrizada p/ CallbackSystem/
            // IGameSystem em vez de Codeware/IScriptable) — roda IMEDIATAMENTE após o forge, pra
            // prever se o IsKindOf walk + o check de base-declarada vão passar, ANTES do validador
            // ORIGINAL rodar (log logo abaixo, fora deste bloco). Ver `diag_class_forge_prediction`
            // (2026-08-06, cont.26 — consolida esta lógica com a cópia idêntica do bloco DES).
            diag_class_forge_prediction(&reg, "CBS", "CallbackSystem", x1);
        } else {
            crate::log_dev!("[classval-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
        }
    }
    // Codeware `#96`: `inkUserData` é classe VANILLA e o bind dela acontece junto do resto da árvore de UI; a visita do validador ao
    // `inkWidget` (vanilla também passa por este hook) é o gatilho, antes de qualquer mod rodar.
    let iw_hash = *INKWIDGET_CNAME.get_or_init(|| crate::cname::cname("inkWidget"));
    if name_hash == Some(iw_hash) && !INKWIDGET_REGISTERED.swap(true, Ordering::AcqRel) {
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r96 = crate::interface::register_worldwidget_helper(&reg);
            crate::log_dev!("[classval-probe] {r96}");
        }
    }
    // Tentativa 11 (2026-07-13, sessão seguinte) — o parent-pointer fix (Tentativa 10) rodou ao
    // vivo mas o validador ORIGINAL continuou retornando 0 pro Codeware. Em vez de hookar
    // `0x1010c6bf4` (IsKindOf) direto — função pequena em tight-loop, mesmo risco de transbordo
    // de redirect já documentado pra outros leafs — reimplementamos a MESMA caminhada em Rust
    // aqui dentro (hook já provado seguro), lendo os ponteiros REAIS: o getter do base esperado
    // (`0x1011e46d8`), a classe forjada, e caminhando `cls+0x10` como o disasm de 0x1011a84b4
    // mostrou. Também loga os bits de `flags@+0x70` (isNative/isImportOnly já OK por construção;
    // bits 7/8/9 são checks ADICIONAIS pós-IsKindOf que o disasm revelou, ainda não instrumentados)
    // e os bytes do descritor `x1+SC_VISIBILIDADE`/`+SC_FLAGS` que esses checks também consultam.
    // Diagnóstico puro (não altera o forge, só re-lê estado pra logar) — gated dev_mode.
    if diag && name_hash == Some(fachada_de_versao_hash) {
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let fachada_de_versao_cls = reg.class_by_name("Codeware");
            let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_011e_46d8));
            let expected_base = getter();
            let iscriptable = reg.class_by_name("IScriptable");
            crate::log_dev!(
                "[classval-probe] IsKindOf diag: Codeware={fachada_de_versao_cls:p} expected_base(getter)={expected_base:p} IScriptable(by_name)={iscriptable:p} (bate={})",
                expected_base as u64 == iscriptable as u64
            );
            let mut cur = fachada_de_versao_cls;
            let mut found = false;
            for depth in 0..16u32 {
                if cur.is_null() {
                    crate::log_dev!("[classval-probe] IsKindOf walk: profundidade={depth} cur=NULL — parando");
                    break;
                }
                if cur as u64 == expected_base as u64 {
                    found = true;
                    crate::log_dev!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} == expected_base — ACHOU");
                    break;
                }
                if !crate::memoria::is_readable((cur as *const u8).add(0x10) as *const c_void, 8) {
                    crate::log_dev!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} [cur+0x10] ilegível — parando");
                    break;
                }
                let next = core::ptr::read_unaligned((cur as *const u8).add(0x10) as *const *mut c_void);
                crate::log_dev!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} parent[+0x10]={next:p}");
                cur = next;
            }
            crate::log_dev!(
                "[classval-probe] IsKindOf PREVISTO: {}",
                if found { "SUCESSO" } else { "FALHA (não achou expected_base na cadeia de parent)" }
            );
            if !fachada_de_versao_cls.is_null() && crate::memoria::is_readable(fachada_de_versao_cls as *const c_void, 0x74) {
                let flags = core::ptr::read_unaligned((fachada_de_versao_cls as *const u8).add(0x70) as *const u32);
                crate::log_dev!(
                    "[classval-probe] Codeware flags@+0x70 = {flags:#010x} (bit1_isNative={} bit6_isImportOnly={} bit7={} bit8={} bit9={})",
                    (flags >> 1) & 1, (flags >> 6) & 1, (flags >> 7) & 1, (flags >> 8) & 1, (flags >> 9) & 1
                );
            }
            if !x1.is_null() && crate::memoria::is_readable(x1.add(SC_FLAGS) as *const c_void, 1) {
                let b88 = *x1.add(SC_VISIBILIDADE);
                let b89 = *x1.add(SC_FLAGS);
                crate::log_dev!("[classval-probe] descriptor bytes: visibilidade={b88:#x} flags={b89:#x}");
            }
            // Tentativa 11 cont. — achado no disasm de 0x1011a88a8..0x1011a89d0: um SEGUNDO check,
            // INDEPENDENTE do IsKindOf, lê `[x1+0x20]` (ponteiro pra descritor da "base declarada"
            // do lado do bundle/compilador — só não-nulo se o `.reds` tiver `extends` explícito),
            // resolve o CName dela (`[+0x8]` do descritor, mesmo padrão de `[x1+8]` da própria
            // classe) via GetClass (singleton 0x1010b3288 + vtbl+0x108 — MESMA via já confirmada
            // resolver nossa própria classe forjada), e COMPARA contra `[x21+0x10]` (nosso parent
            // real). Strings de erro batem: "Native class has base class that is different..."/
            // "...that is not imported". Se isso divergir, é a causa REAL da falha residual.
            if !x1.is_null() && crate::memoria::is_readable(x1.add(SC_PAI) as *const c_void, 8) {
                let decl_base_desc = core::ptr::read_unaligned(x1.add(SC_PAI) as *const *mut u8);
                crate::log_dev!("[classval-probe] descriptor[x1+SC_PAI] (base declarada) = {decl_base_desc:p}");
                if decl_base_desc.is_null() {
                    crate::log_dev!("[classval-probe] sem base declarada explícita (.reds sem `extends`) — Path A do disasm");
                } else if crate::memoria::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
                    let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
                    let decl_name = crate::cname::resolve_cname(decl_cname);
                    crate::log_dev!("[classval-probe] base declarada: CName={decl_cname:#018x} nome='{decl_name}'");
                    let getter2: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_010b_3288));
                    let singleton2 = getter2();
                    if !singleton2.is_null() && crate::memoria::is_readable(singleton2 as *const c_void, 8) {
                        let vt = core::ptr::read_unaligned(singleton2 as *const *mut u8);
                        if !vt.is_null() && crate::memoria::is_readable(vt.add(0x108) as *const c_void, 8) {
                            let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
                            if crate::rtti::sane(slot) {
                                let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
                                let resolved = getclass(singleton2, decl_cname);
                                crate::log_dev!(
                                    "[classval-probe] base declarada resolvida={resolved:p} vs nosso parent(IScriptable)={iscriptable:p} (bate={})",
                                    resolved as u64 == iscriptable as u64
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    let orig = ORIG_CLASS_VALIDATE.load(Ordering::Relaxed) as *mut c_void;
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8, *mut u8, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3, x4, x5, x6, x7);
    // Achado 2026-07-13 (RE offline, chamadores por `bl` subindo de 0x1011a84b4 até o loop de
    // validação 0x1011a7e28 e daí até singleton->vtbl[8]=0x1011e7114, chamado pelo MESMO
    // 0x104d76544 já mapeado nas Tentativas 7/8): a validação de classe roda AninhADA dentro do
    // orquestrador de verdade — mas NUNCA logamos o RETORNO da validação original do Codeware em
    // si. Se ele voltar 0 (falha) MESMO com nosso forge (Version/Require=true), a causa não é
    // "classe não existe" — é alguma outra checagem interna do validador que register_method
    // não satisfaz. Log SÓ pra Codeware (nome_hash já resolvido acima). Diagnóstico — dev_mode.
    if diag && name_hash == Some(fachada_de_versao_hash) {
        crate::log_dev!("[classval-probe] validador ORIGINAL retornou {ret} pro Codeware (0=falha, !=0=sucesso)");
    }
    // Tentativa 12 (2026-07-13) — decodifiquei o chained-fixup de vtbl+0x08 do singleton
    // 0x1011e6c1c (offline, sem boot): resolve pra 0x1011e7114, a MESMA função já mapeada como
    // o DISPATCHER do loop de validação inteiro (chama 0x1011a7e28 -> este validador, ×12). Ou
    // seja: o retorno agregado de TODAS as 12 validações (não só Codeware) é o que decide
    // [engine+0x54] e, por cascata, o assert. Log leve pra TODAS (não só Codeware) — diagnóstico,
    // dev_mode (o forge de Codeware já é incondicional acima; isso é só instrumentação).
    if ret & 0xff == 0 {
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::veredito_dev!("[bind] classe '{name}' reprovada na validação do bind");
    }
    if diag && name_hash != Some(fachada_de_versao_hash) {
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::log_dev!("[classval-probe] validador retornou {ret} pra '{name}' (hash={:#018x})", name_hash.unwrap_or(0));
    }
    ret
}

// ===== Codeware `#251` — espelho do `ScriptBundle` no `ScriptValidator::Validate` =====
// `0x1011a7e28(validator, bundle, report) -> bool` é o único chamador do validador de classe (`0x1011a7f38`) e é chamado só de `0x1011e71d8`. A coleta de
// definições que ele faz, `0x10111ebcc(bundle, &lista, 0)`, soma os tamanhos em `+0xac`/`+0xbc`/`+0xcc`/`+0xdc`/`+0xec` e lê as entradas de `+0xe0`: os
// arrays `globals`/`enums`/`bitfields`/`classes`/`types` do `Red/ScriptBundle.hpp`, nos mesmos offsets. O hook lê o bundle antes da original, só memória
// (nem RTTI nem pool de nomes), guarda as contagens e o índice (dono, nome curto) → (arquivo, linha) das funções que têm arquivo e, com
// `~/.bwms-hook-c251-teste`, confere os membros de `bwms-c251-bundle-teste.reds`.
const SCRIPT_VALIDATE_VM: u64 = 0x1_011a_7e28;
/// `sub sp,sp,#0x90` (d10243ff) + `stp x26,x25,[sp,#0x40]` (a90467fa), LE como u64.
const SCRIPT_VALIDATE_PROLOGUE: u64 = 0xa904_67fa_d102_43ff;
static ORIG_SCRIPT_VALIDATE: AtomicU64 = AtomicU64::new(0);

/// Arrays do `ScriptBundle` na ordem das contagens de `BwmsScriptDefinitionCount`: arquivos, globais, enums, bitfields, classes, tipos, strings.
const SB_ARRAYS: [usize; 7] = [0x98, 0xa8, 0xb8, 0xc8, 0xd8, 0xe8, 0xf8];
const SB_ARQUIVOS: usize = 0x98;
const SB_GLOBAIS: usize = 0xa8;
const SB_ENUMS: usize = 0xb8;
const SB_CLASSES: usize = 0xd8;
/// `ScriptDefinition::name`. Na 2.5 toda definição tem 0x28 B a mais em +0x20..+0x47 (byte de flags, ponteiro e `DynArray`), e o bundle
/// tem 8 B a mais antes dos arrays.
const SD_NOME: usize = 0x08;
/// `ScriptClass`: pai, propriedades, funções e flags (`isNative` bit 0, `isImportOnly` 1, `isAbstract` 2, `isFinal` 3, `isStruct` 4).
const SC_PAI: usize = 0x48;
const SC_PROPS: usize = 0x50;
const SC_FUNCS: usize = 0x90;
const SC_FLAGS: usize = 0xd1;
/// `ScriptClass::visibility` (`0xd0`); a conferência lê o offset do `GetVisibility` da vtable da classe.
const SC_VISIBILIDADE: usize = 0xd0;
/// `ScriptFunction`: nome curto, parâmetros, flags (`isStatic` bit 0, `isFinal` 3, `isNative` 4, `isEvent` 7), visibilidade, arquivo e linha.
// Nome curto: nenhuma função do motor o lê (não é serializado); fica entre o pai (+0x48) e os parâmetros (+0x58), como na 2.31.
const SF_NOME_CURTO: usize = 0x50;
const SF_PARAMS: usize = 0x58;
const SF_FLAGS: usize = 0xf8;
const SF_VISIBILIDADE: usize = 0x129;
const SF_ARQUIVO: usize = 0x140;
const SF_LINHA: usize = 0x148;
/// `ScriptProperty`: flags u16 (`isPersistent` bit 6) e visibilidade. `ScriptParameter`: flags (`isOptional` bit 0, `isOut` 1).
const SP_FLAGS: usize = 0x50;
const SP_VISIBILIDADE: usize = 0x80;
const SPAR_FLAGS: usize = 0x58;
/// `ScriptEnum::constants`, `ScriptEnum::isNative` e `ScriptConstant::value`.
const SE_CONSTANTES: usize = 0x48;
const SE_NATIVO: usize = 0x69;
const SCONST_VALOR: usize = 0x50;
/// Offset do caminho (`CString`) no `ScriptFileInfo`, medido na carga por [`sb_medir_caminho`]; 0 = não achado. O `+0x28` do header deu texto vazio
/// nas 2454 entradas do boot 161.
static SFI_CAMINHO_MEDIDO: AtomicUsize = AtomicUsize::new(0);

struct BundleIndice {
    contagens: [i32; 7],
    arquivos: Vec<String>,
    funcoes: std::collections::HashMap<(u64, u64), (u32, u32)>,
    /// Votos de cada candidato a offset do caminho (`+0x10`..`+0x68`), pro diagnóstico do teste 251.
    votos: [u32; 12],
}

static BUNDLE_INDICE: std::sync::Mutex<Option<BundleIndice>> = std::sync::Mutex::new(None);
/// Classe nativa do teste 251: o `Validate` só manda classe nativa pro validador de classe (`tbnz` em `0x1011a7f28` pula as de script).
static CLASSE_NATIVA_DO_TESTE_DE_BUNDLE: AtomicUsize = AtomicUsize::new(0);
static CLASSE_NATIVA_DO_TESTE_VALIDADA: AtomicBool = AtomicBool::new(false);
static CONFERENCIA_DE_CARGA_DO_BUNDLE: std::sync::Mutex<Option<(bool, String, u32, u32)>> = std::sync::Mutex::new(None);

/// Offset do caminho nas primeiras entradas de `files`: o MENOR candidato (`+0x10`..`+0x68`, passo 8) entre os que mais derem texto terminado em `.reds` ou
/// `.script`, e os votos de cada candidato. Os objetos são contíguos: no boot 163 o caminho do próprio objeto empatou com o do seguinte (`+0x60`), e o
/// `max_by_key` devolvia o último empatado — o vizinho. Um offset positivo só alcança memória adiante, então o menor empatado é o do próprio objeto.
unsafe fn sb_medir_caminho(arquivos: &[*const u8]) -> (Option<usize>, [u32; 12]) {
    let mut votos = [0u32; 12];
    for a in arquivos.iter().take(32) {
        for (k, votos_k) in votos.iter_mut().enumerate() {
            if let Some(s) = sb_texto_curto(a.add(0x10 + k * 8)) {
                if s.ends_with(".reds") || s.ends_with(".script") {
                    *votos_k += 1;
                }
            }
        }
    }
    let maior = votos.iter().copied().max().unwrap_or(0);
    let k = votos.iter().position(|v| maior > 0 && *v == maior);
    (k.map(|k| 0x10 + k * 8), votos)
}

/// Texto de um `CString` em `p` só com comprimento de caminho (1 a 512 B). A medição passa por offsets errados, e ali o comprimento é lixo: no boot 162
/// um deles mandou o `read_cstring` ler além da região mapeada.
unsafe fn sb_texto_curto(p: *const u8) -> Option<String> {
    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 0x20) {
        return None;
    }
    let n = ((p.add(0x14) as *const u32).read_unaligned() & 0x3FFF_FFFF) as usize;
    if n == 0 || n > 512 {
        return None;
    }
    crate::rtti::read_cstring(p)
}

/// Caminho de um `ScriptFileInfo` pelo offset medido; vazio sem medida ou ilegível.
unsafe fn sb_caminho(arquivo: *const u8) -> String {
    let off = SFI_CAMINHO_MEDIDO.load(Ordering::Relaxed);
    if off == 0 || arquivo.is_null() {
        return String::new();
    }
    sb_texto_curto(arquivo.add(off)).unwrap_or_default()
}

/// `n` bytes em hexa, pro diagnóstico de layout do teste 251.
unsafe fn sb_hexa(p: *const u8, n: usize) -> String {
    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, n) {
        return "ilegível".into();
    }
    (0..n).map(|k| format!("{:02x}", p.add(k).read())).collect::<Vec<_>>().join("")
}

/// Onde o `GetVisibility` de uma definição (slot `+0x20` da vtable no Mac, depois dos 2 destrutores) busca o valor: `Ok(offset)` pra
/// `ldrb w0, [x0, #off]; ret`, `Err(instruções)` pra outra forma. Só leitura de memória. No boot 163: `0x58` na propriedade (`0x1011a4b50`) e `0xa9` na
/// função (`0x1011a4b64`), os offsets do header.
unsafe fn sb_offset_da_visibilidade(def: *const u8) -> Result<usize, (u32, u32)> {
    if def.is_null() || !crate::memoria::is_readable(def as *const c_void, 8) {
        return Err((0, 0));
    }
    let vt = sb_u64(def, 0);
    if vt < 0x1_0000_0000 || vt & 7 != 0 || !crate::memoria::is_readable((vt + 0x20) as *const c_void, 8) {
        return Err((0, 0));
    }
    let f = ((vt + 0x20) as *const u64).read_unaligned();
    if f < 0x1_0000_0000 || f & 3 != 0 || !crate::memoria::is_readable(f as *const c_void, 8) {
        return Err((0, 0));
    }
    let (i0, i1) = ((f as *const u32).read_unaligned(), ((f + 4) as *const u32).read_unaligned());
    if i0 & 0xFFC0_03FF == 0x3940_0000 && i1 == 0xD65F_03C0 {
        Ok(((i0 >> 10) & 0xFFF) as usize)
    } else {
        Err((i0, i1))
    }
}

/// Censo do byte de visibilidade (0, 1, 2, outro) no bundle: funções (globais e de classe), propriedades e classes, pelos offsets do header que o
/// `GetVisibility` confirma.
unsafe fn sb_censo_visibilidade(b: *const u8) -> [[usize; 4]; 3] {
    let mut censo = [[0usize; 4]; 3];
    let contar = |h: &mut [usize; 4], d: *const u8, off: usize| {
        if crate::memoria::is_readable(d as *const c_void, off + 1) {
            h[(d.add(off).read() as usize).min(3)] += 1;
        }
    };
    for f in sb_ponteiros(b, SB_GLOBAIS) {
        contar(&mut censo[0], f, SF_VISIBILIDADE);
    }
    for c in sb_ponteiros(b, SB_CLASSES) {
        if crate::memoria::is_readable(c as *const c_void, 0xd8) {
            contar(&mut censo[2], c, SC_VISIBILIDADE);
            for f in sb_ponteiros(c, SC_FUNCS) {
                contar(&mut censo[0], f, SF_VISIBILIDADE);
            }
            for p in sb_ponteiros(c, SC_PROPS) {
                contar(&mut censo[1], p, SP_VISIBILIDADE);
            }
        }
    }
    censo
}

unsafe fn sb_u64(p: *const u8, off: usize) -> u64 {
    (p.add(off) as *const u64).read_unaligned()
}

/// Ponteiros de um `DynArray<T*>` em `base+off` (`entries` `+0`, `size` `+0xc`); vazio se não for legível ou não parecer array.
unsafe fn sb_ponteiros(base: *const u8, off: usize) -> Vec<*const u8> {
    if base.is_null() || !crate::memoria::is_readable(base.add(off) as *const c_void, 0x10) {
        return Vec::new();
    }
    let dados = sb_u64(base, off) as *const *const u8;
    let n = (base.add(off + 0x0c) as *const u32).read_unaligned() as usize;
    if dados.is_null() || n == 0 || n > 1 << 20 || !crate::memoria::is_readable(dados as *const c_void, n * 8) {
        return Vec::new();
    }
    (0..n).map(|k| dados.add(k).read_unaligned()).filter(|p| !p.is_null()).collect()
}

/// Definição de nome `nome` (lido em `campo`) entre os ponteiros, legível por `tam` B.
unsafe fn sb_achar(defs: &[*const u8], nome: &str, tam: usize, campo: usize) -> Option<*const u8> {
    let h = crate::cname::cname(nome);
    defs.iter().copied().find(|d| crate::memoria::is_readable(*d as *const c_void, tam) && sb_u64(*d, campo) == h)
}

/// Índice do bundle: contagens, caminhos dos arquivos e, das funções globais e de classe com arquivo, (dono, nome curto) → (arquivo, linha).
unsafe fn bundle_indexar(b: *const u8) -> Option<BundleIndice> {
    if b.is_null() || !crate::memoria::is_readable(b as *const c_void, 0x108) {
        return None;
    }
    let contagens = SB_ARRAYS.map(|off| (b.add(off + 0x0c) as *const u32).read_unaligned() as i32);
    let arquivos_ptr = sb_ponteiros(b, SB_ARQUIVOS);
    let (caminho_off, votos) = sb_medir_caminho(&arquivos_ptr);
    SFI_CAMINHO_MEDIDO.store(caminho_off.unwrap_or(0), Ordering::Relaxed);
    let arquivo_de: std::collections::HashMap<usize, u32> = arquivos_ptr.iter().enumerate().map(|(k, a)| (*a as usize, k as u32)).collect();
    let arquivos = arquivos_ptr.iter().map(|a| sb_caminho(*a)).collect();
    let mut funcoes = std::collections::HashMap::new();
    let mut registrar = |dono: u64, f: *const u8| {
        if !crate::memoria::is_readable(f as *const c_void, 0x150) {
            return;
        }
        if let Some(&k) = arquivo_de.get(&(sb_u64(f, SF_ARQUIVO) as usize)) {
            funcoes.insert((dono, sb_u64(f, SF_NOME_CURTO)), (k, (f.add(SF_LINHA) as *const u32).read_unaligned()));
        }
    };
    for f in sb_ponteiros(b, SB_GLOBAIS) {
        registrar(0, f);
    }
    for c in sb_ponteiros(b, SB_CLASSES) {
        if crate::memoria::is_readable(c as *const c_void, 0xd8) {
            for f in sb_ponteiros(c, SC_FUNCS) {
                registrar(sb_u64(c, SD_NOME), f);
            }
        }
    }
    Some(BundleIndice { contagens, arquivos, funcoes, votos })
}

/// Contagem `tipo` do último bundle validado (ordem de [`SB_ARRAYS`]); -1 fora da faixa ou antes da carga.
#[cfg(feature = "devtools")]
pub(crate) fn bundle_contagem(tipo: i32) -> i32 {
    let Ok(g) = BUNDLE_INDICE.lock() else { return -1 };
    g.as_ref().and_then(|ind| ind.contagens.get(usize::try_from(tipo).ok()?).copied()).unwrap_or(-1)
}

/// Arquivo e linha da função `nome` (nome curto) de `dono` (nome da classe; 0 = global), se ela veio de um arquivo.
pub(crate) fn bundle_fonte(dono: u64, nome: u64) -> Option<(String, u32)> {
    let g = BUNDLE_INDICE.lock().ok()?;
    let ind = g.as_ref()?;
    let (arquivo, linha) = *ind.funcoes.get(&(dono, nome))?;
    Some((ind.arquivos.get(arquivo as usize)?.clone(), linha))
}

/// Resultado da conferência de carga do teste 251: (ok, texto, linha de `Somar`, linha de `BwmsC251Global`).
#[cfg(feature = "devtools")]
pub(crate) fn conferencia_de_carga_do_bundle() -> Option<(bool, String, u32, u32)> {
    CONFERENCIA_DE_CARGA_DO_BUNDLE.lock().ok().and_then(|g| g.clone())
}

/// Confere uma função do alvo 251 e devolve a linha lida, ou o que não bateu. `base` fixa na 1ª função se a linha vem contada de 0 ou de 1.
#[allow(clippy::too_many_arguments)]
unsafe fn conferir_funcao_do_bundle(
    f: *const u8,
    nome: &str,
    ligados: u32,
    desligados: u32,
    parametros: usize,
    flags_2o: Option<u8>,
    linha: u32,
    base: &mut Option<u32>,
) -> Result<u32, String> {
    let flags = (f.add(SF_FLAGS) as *const u32).read_unaligned();
    let params = sb_ponteiros(f, SF_PARAMS);
    let flags_params: Vec<u8> = params.iter().map(|p| if crate::memoria::is_readable(*p as *const c_void, 0x60) { p.add(SPAR_FLAGS).read() } else { 0xff }).collect();
    let lida = (f.add(SF_LINHA) as *const u32).read_unaligned();
    let caminho = sb_caminho(sb_u64(f, SF_ARQUIVO) as *const u8);
    let deslocamento = *base.get_or_insert(linha.wrapping_sub(lida));
    let ok = flags & ligados == ligados
        && flags & desligados == 0
        && params.len() == parametros
        && flags_params.first().is_none_or(|p| *p == 0)
        && flags_2o.is_none_or(|esperado| flags_params.get(1) == Some(&esperado))
        && deslocamento <= 1
        && lida.wrapping_add(deslocamento) == linha
        && caminho.ends_with("bwms-c251-bundle-teste.reds");
    if ok {
        Ok(lida)
    } else {
        Err(format!("{nome}: flags {flags:#x}, parâmetros {flags_params:?}, linha {lida} (esperado {linha}), arquivo '{caminho}'"))
    }
}

/// Conferência de carga do teste 251 sobre os membros de `bwms-c251-bundle-teste.reds`, feita antes da original.
struct ConferenciaDoBundle {
    ok: bool,
    texto: String,
    /// Linhas de `Somar` e de `BwmsC251Global`.
    somar: u32,
    global: u32,
    /// `IScriptable` nativa, pra conferência cruzada com o validador de classe.
    nativa: usize,
    /// (definição, offset da visibilidade, valor declarado) dos membros de visibilidade conhecida: 3 propriedades, 4 funções e a classe.
    membros: Vec<(usize, usize, u8)>,
}

unsafe fn conferir_bundle_de_teste(b: *const u8, ind: &BundleIndice) -> ConferenciaDoBundle {
    let classes = sb_ponteiros(b, SB_CLASSES);
    let Some(alvo) = sb_achar(&classes, "BwmsC251Alvo", 0xd8, SD_NOME) else {
        let texto = format!("classe BwmsC251Alvo fora das {} classes", classes.len());
        return ConferenciaDoBundle { ok: false, texto, somar: 0, global: 0, nativa: 0, membros: Vec::new() };
    };
    let nativa = sb_achar(&classes, "IScriptable", 0xd8, SD_NOME).filter(|c| c.add(SC_FLAGS).read() & 1 != 0).map_or(0, |c| c as usize);
    let mut falhas: Vec<String> = Vec::new();
    let caminho_off = SFI_CAMINHO_MEDIDO.load(Ordering::Relaxed);
    if caminho_off == 0 {
        falhas.push("caminho do arquivo não medido".into());
    }
    if nativa == 0 {
        falhas.push("IScriptable nativa fora das classes".into());
    }
    let flags_classe = alvo.add(SC_FLAGS).read();
    let pai = sb_u64(alvo, SC_PAI) as *const u8;
    let nome_pai = if crate::memoria::is_readable(pai as *const c_void, 0x10) { sb_u64(pai, SD_NOME) } else { 0 };
    if flags_classe & 0x1f != 0 || nome_pai != crate::cname::cname("IScriptable") {
        falhas.push(format!("classe: flags {flags_classe:#x}, pai {nome_pai:#x}"));
    }
    let props = sb_ponteiros(alvo, SC_PROPS);
    let mut vis_props: Vec<(*const u8, u8)> = Vec::new();
    // Visibilidade como está GRAVADA no bundle: o `scc` do BWMS grava todo membro de mod como público (decompilação do bundle do boot 164), então os 3 valem
    // 0 aqui; o controle com público, protegido e privado é o `Ref_2_1_BaseClass` vanilla, logo abaixo.
    for (nome, persistente, visibilidade) in [("numero", false, 0u8), ("contagem", true, 0u8), ("meio", false, 0u8)] {
        match sb_achar(&props, nome, 0xc8, SD_NOME) {
            Some(p) => {
                let flags = (p.add(SP_FLAGS) as *const u16).read_unaligned();
                if (flags & 0x40 != 0) != persistente {
                    falhas.push(format!("propriedade {nome}: flags {flags:#x}"));
                }
                vis_props.push((p, visibilidade));
            }
            None => falhas.push(format!("propriedade {nome} ausente entre {}", props.len())),
        }
    }
    let funcs = sb_ponteiros(alvo, SC_FUNCS);
    let mut base = None;
    let mut linhas = [0u32; 5];
    let mut vis_funcs: Vec<(*const u8, u8)> = Vec::new();
    // (nome, flags ligados, flags desligados, parâmetros, flags do 2º parâmetro, linha em base 1, visibilidade)
    let casos: [(&str, u32, u32, usize, Option<u8>, u32, u8); 4] = [
        ("Somar", 1 << 3, 1 | 1 << 4 | 1 << 7, 2, Some(1), 14, 0),
        ("Criar", 1, 1 << 3 | 1 << 4 | 1 << 7, 0, None, 18, 0),
        ("OnBwmsC251", 1 << 7, 1 | 1 << 4, 1, None, 22, 0),
        ("Oculta", 0, 1 | 1 << 4 | 1 << 7, 0, None, 26, 0),
    ];
    for (k, (nome, ligados, desligados, n, flags_2o, linha, vis)) in casos.into_iter().enumerate() {
        let Some(f) = sb_achar(&funcs, nome, 0x150, SF_NOME_CURTO) else {
            falhas.push(format!("função {nome} ausente entre {}", funcs.len()));
            continue;
        };
        vis_funcs.push((f, vis));
        match conferir_funcao_do_bundle(f, nome, ligados, desligados, n, flags_2o, linha, &mut base) {
            Ok(lida) => linhas[k] = lida,
            Err(e) => falhas.push(e),
        }
    }
    let globais = sb_ponteiros(b, SB_GLOBAIS);
    match sb_achar(&globais, "BwmsC251Global", 0x150, SF_NOME_CURTO).map(|f| conferir_funcao_do_bundle(f, "BwmsC251Global", 0, 1 << 4 | 1 << 7, 2, Some(2), 31, &mut base)) {
        Some(Ok(lida)) => linhas[4] = lida,
        Some(Err(e)) => falhas.push(e),
        None => falhas.push(format!("BwmsC251Global fora das {} globais", globais.len())),
    }
    // Visibilidade: o offset sai do `GetVisibility` da própria vtable. No boot 163 ele era `ldrb w0, [x0, #0x58]` na propriedade e `#0xa9` na função —
    // os offsets do header —, e o byte valia 0 nos membros privados. O censo do bundle e a leitura dos membros ficam pro hook, antes e depois da original.
    let off_prop = vis_props.first().map(|(p, _)| sb_offset_da_visibilidade(*p));
    let off_func = vis_funcs.first().map(|(f, _)| sb_offset_da_visibilidade(*f));
    let off_classe = sb_offset_da_visibilidade(alvo);
    if off_prop != Some(Ok(SP_VISIBILIDADE)) || off_func != Some(Ok(SF_VISIBILIDADE)) || off_classe != Ok(SC_VISIBILIDADE) {
        falhas.push(format!("GetVisibility: propriedade {off_prop:x?}, função {off_func:x?}, classe {off_classe:x?}"));
    }
    let mut membros: Vec<(usize, usize, u8)> = vis_props.iter().map(|(p, v)| (*p as usize, SP_VISIBILIDADE, *v)).collect();
    membros.extend(vis_funcs.iter().map(|(f, v)| (*f as usize, SF_VISIBILIDADE, *v)));
    membros.push((alvo as usize, SC_VISIBILIDADE, 0));
    // Controle vanilla com as 3 visibilidades gravadas: `Ref_2_1_BaseClass` (`core/reference.script`) — `prop1`/`f1` públicos, `prop2`/`f2` protegidos,
    // `prop3`/`f3` privados.
    match sb_achar(&classes, "Ref_2_1_BaseClass", 0xd8, SD_NOME) {
        Some(controle) => {
            let (props_c, funcs_c) = (sb_ponteiros(controle, SC_PROPS), sb_ponteiros(controle, SC_FUNCS));
            for (k, visibilidade) in [0u8, 1, 2].into_iter().enumerate() {
                match sb_achar(&props_c, &format!("prop{}", k + 1), 0xc8, SD_NOME) {
                    Some(p) => membros.push((p as usize, SP_VISIBILIDADE, visibilidade)),
                    None => falhas.push(format!("Ref_2_1_BaseClass sem prop{} entre {}", k + 1, props_c.len())),
                }
                match sb_achar(&funcs_c, &format!("f{}", k + 1), 0x150, SF_NOME_CURTO) {
                    Some(f) => membros.push((f as usize, SF_VISIBILIDADE, visibilidade)),
                    None => falhas.push(format!("Ref_2_1_BaseClass sem f{} entre {}", k + 1, funcs_c.len())),
                }
            }
        }
        None => falhas.push("classe de controle Ref_2_1_BaseClass fora do bundle".into()),
    }
    let enums = sb_ponteiros(b, SB_ENUMS);
    match sb_achar(&enums, "BwmsC251Enum", 0x70, SD_NOME) {
        Some(e) => {
            let mut valores: Vec<(u64, u64)> = sb_ponteiros(e, SE_CONSTANTES)
                .iter()
                .filter(|c| crate::memoria::is_readable(**c as *const c_void, 0x58))
                .map(|c| (sb_u64(*c, SD_NOME), sb_u64(*c, SCONST_VALOR)))
                .collect();
            valores.sort_unstable();
            let mut esperado = vec![(crate::cname::cname("Primeiro"), 3), (crate::cname::cname("Segundo"), 7)];
            esperado.sort_unstable();
            let nativo = e.add(SE_NATIVO).read();
            if valores != esperado || nativo != 0 {
                falhas.push(format!("enum: valores {:?}, nativo {nativo}", valores.iter().map(|(_, v)| *v).collect::<Vec<_>>()));
            }
        }
        None => falhas.push(format!("BwmsC251Enum fora dos {} enums", enums.len())),
    }
    let indice_somar = ind.funcoes.get(&(crate::cname::cname("BwmsC251Alvo"), crate::cname::cname("Somar"))).map(|(_, l)| *l);
    let indice_global = ind.funcoes.get(&(0, crate::cname::cname("BwmsC251Global"))).map(|(_, l)| *l);
    if indice_somar != Some(linhas[0]) || indice_global != Some(linhas[4]) {
        falhas.push(format!("índice: Somar {indice_somar:?}, global {indice_global:?}"));
    }
    if ind.contagens[4] <= 1000 || ind.contagens[0] <= 0 || ind.arquivos.len() != ind.contagens[0] as usize {
        falhas.push(format!("contagens {:?} com {} caminhos", ind.contagens, ind.arquivos.len()));
    }
    let primeiro = sb_ponteiros(b, SB_ARQUIVOS).first().copied().unwrap_or(std::ptr::null());
    let texto = format!(
        "bundle com {:?} (arquivos, globais, enums, bitfields, classes, tipos, strings), {} funções com arquivo no índice; caminho do arquivo em +{caminho_off:#x} (votos de +0x10 a +0x68 {:?}; 1º ScriptFileInfo {}); GetVisibility em {off_prop:x?} (propriedade), {off_func:x?} (função), {off_classe:x?} (classe); linhas Somar {}, Criar {}, OnBwmsC251 {}, Oculta {}, BwmsC251Global {} (deslocamento {base:?}); falhas: {}",
        ind.contagens,
        ind.funcoes.len(),
        ind.votos,
        sb_hexa(primeiro, 0x60),
        linhas[0],
        linhas[1],
        linhas[2],
        linhas[3],
        linhas[4],
        if falhas.is_empty() { "nenhuma".to_string() } else { falhas.join("; ") }
    );
    ConferenciaDoBundle { ok: falhas.is_empty(), texto, somar: linhas[0], global: linhas[4], nativa, membros }
}

/// Chamado pelo validador de classe: marca que a `IScriptable` nativa do teste 251 passou por ele.
fn marcar_classe_do_teste_validada(classe: *mut u8) {
    let nativa = CLASSE_NATIVA_DO_TESTE_DE_BUNDLE.load(Ordering::Relaxed);
    if nativa != 0 && classe as usize == nativa {
        CLASSE_NATIVA_DO_TESTE_VALIDADA.store(true, Ordering::Relaxed);
    }
}

unsafe extern "C" fn script_validate_hook(validador: *mut c_void, bundle: *mut u8, relatorio: *mut c_void) -> u64 {
    let indice = bundle_indexar(bundle);
    let conferencia = match &indice {
        Some(ind) if crate::marcador_individual("c251-teste") => Some(conferir_bundle_de_teste(bundle, ind)),
        _ => None,
    };
    let censo_antes = conferencia.as_ref().map(|_| sb_censo_visibilidade(bundle));
    if let Some(c) = &conferencia {
        CLASSE_NATIVA_DO_TESTE_DE_BUNDLE.store(c.nativa, Ordering::Relaxed);
    }
    let orig: unsafe extern "C" fn(*mut c_void, *mut u8, *mut c_void) -> u64 = std::mem::transmute(ORIG_SCRIPT_VALIDATE.load(Ordering::Relaxed));
    let ret = orig(validador, bundle, relatorio);
    if let Some(c) = conferencia {
        let validada = CLASSE_NATIVA_DO_TESTE_VALIDADA.load(Ordering::Relaxed);
        let (antes, depois) = (censo_antes.unwrap_or_default(), sb_censo_visibilidade(bundle));
        // Se alguma definição do bundle tem 1 ou 2, o motor guarda visibilidade e os membros do teste têm que valer o declarado; se todas valem 0, o
        // `GetVisibility` do motor devolve 0 pra todas, e os membros também têm que valer 0.
        let guarda = [antes, depois].iter().flatten().any(|h| h[1] + h[2] > 0);
        let fora = [antes, depois].iter().flatten().any(|h| h[3] > 0);
        let lidos: Vec<(u8, u8)> = c
            .membros
            .iter()
            .map(|(d, off, declarado)| (if crate::memoria::is_readable(*d as *const c_void, off + 1) { (*d as *const u8).add(*off).read() } else { 0xff }, *declarado))
            .collect();
        // 8 membros do teste (públicos no bundle) e os 6 do `Ref_2_1_BaseClass`.
        let membros_ok = c.membros.len() == 14 && lidos.iter().all(|(lido, declarado)| if guarda { lido == declarado } else { *lido == 0 });
        if let Ok(mut g) = CONFERENCIA_DE_CARGA_DO_BUNDLE.lock() {
            *g = Some((
                c.ok && validada && !fora && membros_ok,
                format!(
                    "{}; visibilidade no bundle (funções, propriedades, classes; 0/1/2/outro): antes {antes:?}, depois {depois:?}, o motor guarda visibilidade: {guarda}; membros (lido, declarado) {lidos:?}; IScriptable nativa passou pelo validador de classe: {validada}; Validate devolveu {}",
                    c.texto,
                    ret & 0xff
                ),
                c.somar,
                c.global,
            ));
        }
    }
    if let Some(ind) = indice {
        crate::log_dev!(
            "[bundle] Validate: contagens {:?}, {} funções com arquivo no índice, caminho do arquivo em +{:#x}",
            ind.contagens,
            ind.funcoes.len(),
            SFI_CAMINHO_MEDIDO.load(Ordering::Relaxed)
        );
        if let Ok(mut g) = BUNDLE_INDICE.lock() {
            *g = Some(ind);
        }
    }
    ret
}

// A recarga dos scripts dos mods do mod.io: slot 2 (`0x1011e914c`) da vtable `0x107837c80` do serviço de scripts de mods (sem RTTI), que chama o
// carregador dos `*.redscripts` dos mods (`0x1011ea220`). Roda na thread principal depois de instalar mod de script com o jogo aberto, e o bind
// dá endereço novo às funções de script (F2d, 2026-10-26: o `OnApply` da reaplicação do TweakDB chamava a função morta — 0 ms, nenhuma escrita).
const SCRIPTS_DOS_MODS_VTABLE: u64 = 0x1_0783_7c80;
const SCRIPTS_DOS_MODS_RECARGA_VM: u64 = 0x1_011e_914c;
static ORIG_RECARGA_DE_SCRIPTS: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn recarga_de_scripts_dos_mods(a0: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64, a7: u64) -> u64 {
    let f: extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(ORIG_RECARGA_DE_SCRIPTS.load(Ordering::Acquire));
    let r = f(a0, a1, a2, a3, a4, a5, a6, a7);
    crate::rtti::scripts_recarregados();
    crate::veredito_dev!("[scripts] o motor recarregou os scripts dos mods: funções resolvidas esquecidas (geração {}) = 1", crate::rtti::geracao_de_scripts());
    r
}

/// Troca o slot 2 do serviço de scripts dos mods, no carregamento; só com o slot apontando para a função esperada.
pub(crate) unsafe fn install_recarga_de_scripts_dos_mods() {
    if ORIG_RECARGA_DE_SCRIPTS.load(Ordering::Relaxed) != 0 {
        return;
    }
    let vtbl = crate::rebase(SCRIPTS_DOS_MODS_VTABLE) as *mut u64;
    if !crate::memoria::is_readable(vtbl.add(2) as *const c_void, 8) || vtbl.add(2).read_unaligned() != crate::rebase(SCRIPTS_DOS_MODS_RECARGA_VM) as u64 {
        return crate::log_dev!("[scripts] slot 2 do serviço de scripts dos mods não é o esperado — sem esquecer funções na recarga");
    }
    match crate::ganchos::trocar_slot_da_vtable(vtbl, 2, recarga_de_scripts_dos_mods as *const c_void) {
        Some(orig) => ORIG_RECARGA_DE_SCRIPTS.store(orig as u64, Ordering::Release),
        None => crate::log_dev!("[scripts] FALHA ao trocar o slot 2 do serviço de scripts dos mods"),
    }
}

/// Codeware `#251`: hook do `ScriptValidator::Validate`, incondicional como o do validador de classe (só lê o bundle antes da original).
pub(crate) unsafe fn install_script_validate_hook() {
    let target = crate::rebase(SCRIPT_VALIDATE_VM);
    if !crate::memoria::is_readable(target as *const c_void, 8) {
        crate::log_dev!("[bundle] Validate ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != SCRIPT_VALIDATE_PROLOGUE {
        crate::log_dev!("[bundle] prólogo do Validate não casou ({got:#018x}) -> sem hook");
        return;
    }
    match crate::ganchos::instalar_com_threads_paradas(target, script_validate_hook as *mut c_void, Some(&ORIG_SCRIPT_VALIDATE)) {
        Some(_) => {
            crate::log_dev!("[bundle] hook do Validate instalado @ {target:p}");
        }
        None => crate::log_dev!("[bundle] FALHA ao hookar o Validate"),
    }
}

/// PRODUÇÃO 2026-07-13: este hook agora é INCONDICIONAL (era gated `~/.bwms-classvalidate-probe`
/// até um teste com zero markers de dev revelar que, sem ele, o boot CRASHA sempre que
/// `ponte-fachada-de-versao.reds` está no bundle — ou seja, pra QUALQUER usuário final instalando o
/// BWMS hoje. O forge da classe `Codeware` (dentro do hook) é o que realmente conserta o boot;
/// o marker antigo só devia ter gateado o LOG verboso (agora atrás de `dev_mode()`, ver a fn).
pub(crate) unsafe fn install_class_validate_probe() {
    let target = crate::rebase(CLASS_VALIDATE_VM);
    if !crate::memoria::is_readable(target as *const c_void, 8) {
        crate::log_dev!("[classval-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != CLASS_VALIDATE_PROLOGUE {
        crate::log_dev!("[classval-probe] prólogo não casou ({got:#018x}) -> sem hook");
        return;
    }
    match crate::ganchos::instalar_com_threads_paradas(target, class_validate_probe_hook as *mut c_void, Some(&ORIG_CLASS_VALIDATE)) {
        Some(_) => {
            crate::log_dev!("[classval-probe] sonda instalada (validador de classe @ {target:p})");
        }
        None => crate::log_dev!("[classval-probe] FALHA ao hookar o validador de classe"),
    }
}

/// SM da engagement (`GameSessionDesc`): o `this` do getter da phase byte 0x1015d2dec, capturado em
/// `phase_skip_getter`. Lido pela alavanca d4=2 (`BwmsFireStart` e `fire_lever_direct`, em
/// `inicio/natives.rs`), pelo heartbeat `[hb]` e pelo gate de gameplay do full-body no `cp77_tick`
/// (`lib.rs`) e pelo `engdump` (`diagnostico/mod.rs`).
pub(crate) static GAME_SESSION_DESC: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());

/// O byte de fase da sessão (`GameSessionDesc+0x84`), pelo dispatcher do pregame (0x1015e489c), que troca o estado da ink state machine
/// por ele (memória cp77-pregame-statemachine-map). O getter é genérico e lê o mesmo offset de outros objetos: valor fora de 1–5 não é fase.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum FaseDaSessao {
    /// `inkEngagementScreenState`: a tela "APERTE ESPAÇO".
    Engagement = 1,
    /// `inkInitializeUserScreenState`: o perfil do usuário; avança sozinha por timer.
    IniciandoUsuario = 2,
    /// `inkPreGameMenuState`: o menu do pregame (CONTINUAR, saves).
    MenuDoPregame = 3,
    /// Caso do dispatcher sem troca de estado.
    SemTroca = 4,
    /// `inkGameState`: a sessão de jogo (o load do save passa por aqui; gameplay de verdade é `gameplay_real`).
    Jogo = 5,
}

impl FaseDaSessao {
    pub(crate) fn do_byte(v: i64) -> Option<Self> {
        match v {
            1 => Some(FaseDaSessao::Engagement),
            2 => Some(FaseDaSessao::IniciandoUsuario),
            3 => Some(FaseDaSessao::MenuDoPregame),
            4 => Some(FaseDaSessao::SemTroca),
            5 => Some(FaseDaSessao::Jogo),
            _ => None,
        }
    }
}

/// A última fase lida pelo getter (`FASE_LIDA`), se for fase. Acquire: casa com o `AcqRel` do getter, então quem vê `Jogo` aqui vê também o
/// `trocar_de_mundo` feito antes dela.
pub(crate) fn fase_lida() -> Option<FaseDaSessao> {
    FaseDaSessao::do_byte(FASE_LIDA.load(Ordering::Acquire))
}

/// Player e tx do mundo que saiu na última troca de mundo. O `capture` não os pega de volta até a carga do mundo novo terminar: se o puppet
/// do menu rodar script no desmonte [não medido], voltaria ao `CURRENT_PLAYER` já solto. O teto da recusa é o fim da carga, não a sessão
/// inteira, porque o puppet novo pode nascer no mesmo endereço.
static PLAYER_DO_MUNDO_QUE_SAIU: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static TX_DO_MUNDO_QUE_SAIU: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Leituras de phase 5 desde a última troca de mundo (a da troca é a 1ª) e o segundo do boot em que ela aconteceu.
static LEITURAS_PHASE5_DO_MUNDO: AtomicU64 = AtomicU64::new(0);
static TROCA_DE_MUNDO_S: AtomicU64 = AtomicU64::new(0);

/// A leitura `agora` da phase entra num mundo de jogo novo: 5 vindo de um objeto de sessão novo (-1) ou do pregame (1–4). Nos logs de `proofs/`, o
/// que antecede uma leitura de 5 vinda de outra fase: -1 em 310, 2 em 2, 3 em 1, 7 em 0. O 7 sai com o jogo aberto (`proofs/20261023-provas-25-tweakxl-archives-nativos-abamods-abacheats-telas-anexos/console-do-boot.log:13169`,
/// `5 5 5 5 5 7 7 7 7` em 10 logs) e não é mundo novo: um 7 → 5 zeraria e recusaria o player vivo até o fim da carga.
fn troca_de_mundo(antes: i64, agora: i64) -> bool {
    FaseDaSessao::do_byte(agora) == Some(FaseDaSessao::Jogo)
        && (antes == -1 || FaseDaSessao::do_byte(antes).is_some_and(|f| f != FaseDaSessao::Jogo))
}

/// Zera player e tx: são do mundo que sai (crash SIGBUS de 2026-10-25, `z037/relatorios/CRASH.md`: o `GetGame` do puppet do menu na 1ª
/// leitura de phase 5 levou o `sistema_do_motor_por_nome` a um nó fora do mapa). O `capture` pega os do mundo novo no 1º script deles.
fn trocar_de_mundo() {
    // A contagem volta antes da recusa: com a contagem da sessão anterior (>= 3) a recusa nasceria vencida.
    LEITURAS_PHASE5_DO_MUNDO.store(0, Ordering::Release);
    TROCA_DE_MUNDO_S.store(boot_elapsed_secs(), Ordering::Relaxed);
    let (player, tx) = (crate::current_player(), crate::current_tx());
    // Troca seguida sem captura no meio: fica recusado o par da troca anterior.
    if !player.is_null() {
        PLAYER_DO_MUNDO_QUE_SAIU.store(player, Ordering::Relaxed);
    }
    if !tx.is_null() {
        TX_DO_MUNDO_QUE_SAIU.store(tx, Ordering::Relaxed);
    }
    crate::set_current_player(std::ptr::null_mut());
    crate::set_current_tx(std::ptr::null_mut());
    crate::log_dev!("[mundo] troca de mundo em t={}s: player {player:p} e tx {tx:p} zerados e recusados até o fim da carga", boot_elapsed_secs());
}

/// A carga do mundo da última troca terminou: a 3ª leitura de phase 5 desde ela (o mesmo marco do `FIM_DO_LOAD_AT`, que vale só para a 1ª
/// carga do processo) ou o teto do splash depois dela.
pub(crate) fn carga_do_mundo_terminou() -> bool {
    LEITURAS_PHASE5_DO_MUNDO.load(Ordering::Acquire) >= 3
        || boot_elapsed_secs() >= TROCA_DE_MUNDO_S.load(Ordering::Relaxed) + SPLASH_TETO_APOS_PHASE5_SECS
}

#[cfg(test)]
mod classes_forjadas_teste {
    use super::{Registrar, CLASSES_FORJADAS};

    /// Fachada estática não existe no RTTI do jogo: sem a linha dela, com `register_type_min`, o `.reds` que a declara reprova no
    /// validador e os métodos não registram. Duas linhas com o mesmo nome deixam a 2ª sem efeito (o hash casa a 1ª).
    #[test]
    fn fachadas_da_aparencia_forjadas_com_tipo_minimo() {
        for (nome, registrar) in [
            ("ArchiveXL", crate::aparencia::registrar_fachada_de_recursos as Registrar),
            ("BwmsAparencia", crate::aparencia::register_bwmsaparencia as Registrar),
        ] {
            let linha = CLASSES_FORJADAS.iter().find(|l| l.0 == nome).unwrap_or_else(|| panic!("`{nome}` fora de CLASSES_FORJADAS"));
            assert!(linha.2, "`{nome}` sem `register_type_min`");
            assert_eq!(linha.1 as usize, registrar as usize, "`{nome}` com outro registrador");
        }
        let mut nomes: Vec<&str> = CLASSES_FORJADAS.iter().map(|l| l.0).collect();
        nomes.sort_unstable();
        nomes.dedup();
        assert_eq!(nomes.len(), CLASSES_FORJADAS.len(), "classe repetida em CLASSES_FORJADAS");
    }

    /// O rótulo do log do resultado é o nome da função registradora: a prova de boot acha `[classval-probe] <rótulo> -> ` pelo
    /// nome que está no código.
    #[test]
    fn rotulo_das_fachadas_e_o_nome_do_registrador() {
        for (nome, registrar, rotulo) in [
            ("TweakXL", crate::tweakdb::natives::registrar_fachada_de_tweaks as Registrar, "registrar_fachada_de_tweaks"),
            ("ArchiveXL", crate::aparencia::registrar_fachada_de_recursos as Registrar, "registrar_fachada_de_recursos"),
        ] {
            let linha = CLASSES_FORJADAS.iter().find(|l| l.0 == nome).unwrap_or_else(|| panic!("`{nome}` fora de CLASSES_FORJADAS"));
            assert_eq!(linha.1 as usize, registrar as usize, "`{nome}` com outro registrador");
            assert_eq!(linha.4, rotulo, "`{nome}` com rótulo de log diferente do registrador");
        }
    }
}

#[cfg(test)]
mod fase_da_sessao_teste {
    use super::FaseDaSessao;

    #[test]
    fn so_os_cinco_valores_do_dispatcher_sao_fase() {
        assert_eq!(FaseDaSessao::do_byte(1), Some(FaseDaSessao::Engagement));
        assert_eq!(FaseDaSessao::do_byte(5), Some(FaseDaSessao::Jogo));
        for lixo in [-128, -1, 0, 6, 20, 91] {
            assert_eq!(FaseDaSessao::do_byte(lixo), None, "{lixo}");
        }
        assert!(FaseDaSessao::Engagement < FaseDaSessao::Jogo, "a ordem segue o byte: o watchdog compara a maior fase vista");
    }

    /// Sequências de leitura do getter em `proofs/`: o boot com auto-continue troca de mundo uma vez, na 1ª leitura de 5; as leituras
    /// seguintes do mesmo load e o 7 do jogo aberto não trocam.
    #[test]
    fn troca_de_mundo_so_na_entrada_em_5() {
        let trocas = |seq: &[i64]| {
            let mut antes = -1;
            seq.iter()
                .filter(|&&agora| {
                    let t = super::troca_de_mundo(antes, agora);
                    antes = agora;
                    t
                })
                .count()
        };
        assert_eq!(trocas(&[3, 3, 3, 3, 3, 1, 2, 3, -1, 5, 5, 5, 5, 5]), 1);
        assert_eq!(trocas(&[3, 3, 3, 3, 3, 1, 2, 3, -1, 5, 5, 5, 5, 5, 7, 7, 7, 7]), 1);
        assert_eq!(trocas(&[3, 3, 3, 3, 3, 1, 2, 3, 5, 5]), 1, "5 direto do menu");
        assert_eq!(trocas(&[5, 5, 5, 7, 7, -1, 5, 5]), 2, "objeto de sessão novo (-1) depois do 7");
        assert_eq!(trocas(&[5, 5, 7, 5]), 1, "7 -> 5 não é mundo novo");
        assert_eq!(trocas(&[3, 3, 1, 2, 3]), 0);
    }
}

/// Quantos opens de boot já falhamos. Cap = só os vídeos de boot; depois o bg do menu abre.
static SKIP_N: AtomicUsize = AtomicUsize::new(0);
/// Opens reais com player vivo: só as 20 primeiras vão ao log de dev (o jogo abre vídeo de UI mais de mil vezes por boot).
#[cfg(feature = "devlog")]
static ABERTOS_DE_VERDADE: AtomicUsize = AtomicUsize::new(0);
/// Cap de opens FALHADOS por boot. Boot sadio falha ~8 (logos+attract). ACIMA disso = o movie-player do
/// jogo está num RETRY-LOOP num open que nunca sucede = a main-thread gira infinito (o early-stick:
/// CPU~107%, RSS~200MB, travado antes da engagement). Passar o cap = deixa abrir DE VERDADE pra quebrar o
/// loop. 64 fica muito acima do sadio → boot normal não muda (nenhum vídeo aparece).
const BOOT_OPEN_CAP: usize = 64;

/// Falha o OPEN dos primeiros N vídeos antes do 1º player capturado (fase de boot) → o jogo trata
/// o open falho PULANDO o vídeo (caso normal de erro), SEM travar mid-play (o que o
/// BinkShouldSkip fazia e quebrava o boot). Depois do 1º player (`player_ja_visto`) ou após N → abre normal.
/// As `flags` (3º argumento, `x2`) vão ao original como o jogo as passou (`0x01300440`/`0x00300440` em `0x1012b41b4`): sem elas o `x2` levava
/// o endereço do trampolim, e com o bit 28 dele ligado o `_BinkOpenWithOptions` lia o vídeo inteiro dentro do open, pedindo IO ao próprio
/// `redIOWorker0` que roda o open — a trava antes da engagement (`z034/relatorios/TRAVA-2.md`).
unsafe extern "C" fn bink_open_replacement(name: *const i8, opts: *mut c_void, flags: u32) -> *mut c_void {
    let orig = ORIG_BINK_SKIP.load(Ordering::Relaxed) as *mut c_void;
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(*const i8, *mut c_void, u32) -> *mut c_void = std::mem::transmute(orig);
    // WATCHDOG / getter-recusado pediram pra desistir do skip (boot travado ou patch recusado) →
    // deixa o vídeo/engagement nativo ABRIR de verdade em vez de manter a tela preta.
    if BINK_RELEASED.load(Ordering::Relaxed) {
        return f(name, opts, flags);
    }
    // Depois do 1º player (menu, carga e gameplay): abre normal (braindances/cutscenes tocam). Pelo player já visto, não pelo atual: o
    // atual volta a nulo na troca de mundo, e a carga não pode virar "boot" e falhar vídeo. As 20 primeiras aberturas reais vão ao log de
    // dev com o ponteiro do 1º argumento (não é texto: o jogo passa o próprio objeto de arquivo, lido pelos callbacks de IO): a trava antes
    // da engagement tem o redIOWorker0 preso num open real (`z032/relatorios/TRAVA.md`).
    if crate::player_ja_visto() {
        #[cfg(feature = "devlog")]
        {
            let k = ABERTOS_DE_VERDADE.fetch_add(1, Ordering::Relaxed);
            if k < 20 {
                crate::log_dev!("[skipintro] bink abre de verdade #{k} (player, falhados antes={}): flags={flags:#010x}", SKIP_N.load(Ordering::Relaxed));
            }
        }
        return f(name, opts, flags);
    }
    // BOOT (sem player): FALHA o open (retorna null SEM abrir) → o jogo trata como erro e PULA o
    // vídeo/tela inteiro (≠ frames=1, que só congela no 1º frame = a cena AINDA aparece). Cobre os
    // logos E o VÍDEO ATTRACT (a cena de cidade que fica de fundo do "APERTE espaço") — o dono NÃO
    // quer ver a cena. Assim o boot é streaming (I/O, ~30s) → menu, sem cena nenhuma.
    let n = SKIP_N.fetch_add(1, Ordering::Relaxed);
    // VÁLVULA DE ESCAPE (fix do early-stick, 2026-07-06): se já falhamos além do cap, o jogo está
    // re-tentando o mesmo open num loop (spin da main-thread) → deixa abrir DE VERDADE pra ele destravar.
    // Boot sadio (~8 opens) nunca chega aqui; só o loop patológico (milhares de opens/s) atinge o cap.
    if n >= BOOT_OPEN_CAP {
        if n == BOOT_OPEN_CAP {
            crate::log_dev!("[skipintro] bink open passou do cap -> deixando abrir de verdade (quebra o retry-loop/early-stick): flags={flags:#010x}");
        }
        return f(name, opts, flags);
    }
    if n < 30 {
        crate::log_dev!("[skipintro] bink #{n}: OPEN FALHADO (vídeo/cena de boot NÃO abre): flags={flags:#010x}");
    }
    std::ptr::null_mut()
}

type ExecFn = unsafe extern "C" fn(
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
    *mut c_void,
) -> *mut c_void;

/// DESCOBERTA (dev): ring dos últimos N CNames de método que passaram pelo executor.
/// Quando o marcador `PopulateSettingsData` dispara (= a tela de settings vai popular),
/// despeja o ring no trace — isso pega o handler do CLIQUE (que NÃO vigiamos) que rodou
/// logo antes. Caso-se hash→nome offline com o dicionário dos 72k nomes do final.redscripts.
/// SÓ em dev_mode (custo: 1 leitura mach_vm por chamada — ok numa sessão de descoberta).
/// Se o ring NÃO contiver o handler do clique, prova que o clique é NATIVO (fora do executor).
const DISC_N: usize = 512;
static DISC_RING: [AtomicU64; DISC_N] = [const { AtomicU64::new(0) }; DISC_N];
static DISC_IDX: AtomicUsize = AtomicUsize::new(0);

unsafe fn discovery_ring(func: *mut c_void) {
    if func.is_null() || !crate::memoria::is_readable(func as *const c_void, 0x18) {
        return;
    }
    let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
    let i = DISC_IDX.fetch_add(1, Ordering::Relaxed) % DISC_N;
    DISC_RING[i].store(mcname, Ordering::Relaxed);
    static MARKER: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let marker = *MARKER.get_or_init(|| crate::cname::cname("PopulateSettingsData"));
    if mcname == marker {
        let start = DISC_IDX.load(Ordering::Relaxed);
        let mut s = String::from("[disc] === métodos antes de PopulateSettingsData (cronológico; hash + nome resolvido) ===\n");
        for k in 0..DISC_N {
            let h = DISC_RING[(start + k) % DISC_N].load(Ordering::Relaxed);
            if h != 0 {
                // resolve_cname usa o CNamePool nativo → nome real. Se o endereço versionado
                // estiver errado p/ este patch, volta "" e eu caso o hash offline (fallback).
                s.push_str(&format!("{h:#018x}  {}\n", crate::cname::resolve_cname(h)));
            }
        }
        crate::trace(&s);
    }
}

// `equiponce-crash-2026-07-31`: profundidade de reentrância do `exec_replacement`, POR THREAD
// (`redDispatcher*` são threads de verdade, não a mesma pilha — thread_local é o isolamento certo).
// Achado do agente de RE: `cp77_tick()` dispara de DENTRO do próprio hook do executor a cada
// `TICK_EVERY` chamadas, em QUALQUER thread, em QUALQUER profundidade de aninhamento dentro da
// execução de script já em voo do motor — o crash do `equiponce` bateu assim (call_func síncrono
// disparado de dentro de uma chamada do motor já em andamento em `redDispatcher7`, não do topo).
// Profundidade==1 (valor LIDO ANTES de decrementar no drop) = esta invocação NÃO está aninhada
// dentro de outra chamada do executor ainda aberta nesta mesma thread — ponto seguro conhecido
// (é onde centenas de `call_func` já dispararam sem problema). >1 = estamos DENTRO de uma chamada
// do motor ainda em voo — mesmo padrão do crash. Só gateia a fila de comandos de canal (a via que
// de fato crashou); eventos já provados estáveis (Session/Update etc.) não são tocados.
// O contador de amostragem do `capture` mora no mesmo `thread_local!`: no macOS cada `thread_local!` custa uma chamada
// ao thunk TLV por acesso, e o `ExecDepthGuard` pega o endereço uma vez por chamada do executor.
struct ContadoresDoExecutor {
    profundidade: std::cell::Cell<u32>,
    misses_do_capture: std::cell::Cell<u32>,
}
thread_local! {
    static EXEC_DEPTH: ContadoresDoExecutor = const {
        ContadoresDoExecutor { profundidade: std::cell::Cell::new(0), misses_do_capture: std::cell::Cell::new(0) }
    };
}
struct ExecDepthGuard(*const ContadoresDoExecutor);
impl ExecDepthGuard {
    fn enter() -> Self {
        EXEC_DEPTH.with(|c| {
            c.profundidade.set(c.profundidade.get() + 1);
            ExecDepthGuard(c)
        })
    }
    fn contadores(&self) -> &ContadoresDoExecutor {
        // `thread_local!` const sem Drop não tem destrutor: o endereço vale até a thread acabar, e o ponteiro cru tira o
        // `Send` do guard, que morre na thread que o criou.
        unsafe { &*self.0 }
    }
}
impl Drop for ExecDepthGuard {
    fn drop(&mut self) {
        let p = &self.contadores().profundidade;
        p.set(p.get().saturating_sub(1));
    }
}
/// `true` se a invocação ATUAL do executor, nesta thread, está aninhada dentro de uma chamada do
/// motor ainda não retornada (ou seja: não estamos no topo da pilha do executor aqui). Usado pra
/// adiar (nunca descartar) a fila de comandos de canal por 1 tick quando inseguro.
pub(crate) fn exec_nested() -> bool {
    EXEC_DEPTH.with(|c| c.profundidade.get() > 1)
}
/// Profundidade ATUAL do executor nesta thread (1 = topo, sem aninhamento).
pub(crate) fn exec_depth() -> u32 {
    EXEC_DEPTH.with(|c| c.profundidade.get())
}

/// Roda `f` com o executor aberto nesta thread, como dentro do `exec_replacement`: teste de quem consulta [`exec_depth`].
#[cfg(test)]
pub(crate) fn com_o_executor_aberto<T>(f: impl FnOnce() -> T) -> T {
    let _execucao = ExecDepthGuard::enter();
    f()
}

#[cfg(test)]
mod contadores_do_executor_teste {
    use super::{capture, exec_depth, exec_nested, ExecDepthGuard};
    use std::ffi::c_void;

    #[test]
    fn profundidade_e_misses_do_capture_sao_da_thread() {
        // Thread própria: os contadores começam em 0. O `ctx` falso nunca é lido: offline o `capture` para no registry ausente.
        std::thread::spawn(|| {
            let ctx = 0x1000usize as *mut c_void;
            assert_eq!(exec_depth(), 0);
            let fora = ExecDepthGuard::enter();
            assert_eq!((exec_depth(), exec_nested()), (1, false));
            {
                let dentro = ExecDepthGuard::enter();
                assert_eq!((exec_depth(), exec_nested()), (2, true));
                unsafe { capture(std::ptr::null_mut(), dentro.contadores()) };
                assert_eq!(dentro.contadores().misses_do_capture.get(), 0, "ctx nulo não conta");
                for _ in 0..3 {
                    unsafe { capture(ctx, dentro.contadores()) };
                }
            }
            assert_eq!(exec_depth(), 1, "a saída do guard de dentro desce 1");
            assert_eq!(fora.contadores().misses_do_capture.get(), 3, "os guards da thread contam no mesmo lugar");
            std::thread::spawn(|| {
                let outra = ExecDepthGuard::enter();
                assert_eq!(exec_depth(), 1);
                assert_eq!(outra.contadores().misses_do_capture.get(), 0, "outra thread tem contador próprio");
            })
            .join()
            .expect("thread vizinha");
            drop(fora);
            assert_eq!(exec_depth(), 0);
        })
        .join()
        .expect("thread do teste");
    }
}

#[cfg(test)]
mod custo_do_executor_teste {
    use super::{exec_replacement, ORIG_EXEC};
    use crate::register::{NATIVE_ROUTES, NATIVE_ROUTE_COUNT};
    use std::ffi::c_void;
    use std::hint::black_box;
    use std::sync::atomic::Ordering;

    unsafe extern "C" fn original_falsa(_: *mut c_void, _: *mut c_void, _: *mut c_void, _: *mut c_void, _: *mut c_void) -> *mut c_void {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn rota_falsa(_: *mut c_void, _: *mut c_void, _: *mut c_void, _: i64) {}

    /// F6-03: ns por chamada de `exec_replacement` com `func` fora da tabela de rotas (quase toda chamada do jogo), no perfil em que
    /// o teste compila. `cargo test --profile publico --lib -- --ignored --nocapture --test-threads=1 executor_custo` e o mesmo com
    /// `--release`. Original falsa que devolve null, `TICK_BUSY` ligado (o tick a cada 2048 chamadas volta na entrada), `ctx` falso
    /// que nunca é lido (o `capture` para no registry ausente). 409 e 589 rotas falsas (589 = a medida de 6,1 ns do HISTORICO).
    /// Mediana de 5 rodadas de 10M chamadas. Microbenchmark offline, sem build-std nem immediate-abort: limite inferior do jogo.
    /// Os bits ligados do `ROTA_BITS` ficam (o campo é privado do `register`); o teste roda sozinho pelo filtro.
    #[test]
    #[ignore]
    fn executor_custo_por_chamada() {
        const BASE: usize = 0x7000_0000_0000;
        const CHAMADAS: usize = 10_000_000;
        // Rota i em BASE + 0x40*i liga o bit (BASE>>4) + 4i; a `func` medida cai no bit (BASE>>4) + 1, desligado.
        let fora = (BASE + 0x10) as *mut c_void;
        let ctx = 0x1000usize as *mut c_void;
        let original_antes = ORIG_EXEC.swap(original_falsa as usize as u64, Ordering::Relaxed);
        let tick_antes = crate::TICK_BUSY.swap(true, Ordering::Relaxed);
        let total_antes = NATIVE_ROUTE_COUNT.load(Ordering::Relaxed);
        let mut incluidas = 0usize;
        for alvo in [409usize, 589] {
            while incluidas < alvo {
                unsafe { crate::npcs::add_route((BASE + 0x40 * incluidas) as *mut c_void, rota_falsa) };
                incluidas += 1;
            }
            assert!(unsafe { crate::register::route_native(fora) }.is_none());
            let mut rodadas = [0f64; 5];
            for r in rodadas.iter_mut() {
                let inicio = std::time::Instant::now();
                for _ in 0..CHAMADAS {
                    black_box(unsafe {
                        exec_replacement(black_box(fora), ctx, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut())
                    });
                }
                *r = inicio.elapsed().as_nanos() as f64 / CHAMADAS as f64;
            }
            rodadas.sort_by(f64::total_cmp);
            println!("[F6-03] {alvo} rotas: {:.2} ns/chamada (mediana de 5; min {:.2}, max {:.2})", rodadas[2], rodadas[0], rodadas[4]);
        }
        if let Ok(mut m) = NATIVE_ROUTES.write() {
            for i in 0..incluidas {
                m.remove(&(BASE + 0x40 * i));
            }
        }
        NATIVE_ROUTE_COUNT.store(total_antes, Ordering::Relaxed);
        crate::TICK_BUSY.store(tick_antes, Ordering::Relaxed);
        ORIG_EXEC.store(original_antes, Ordering::Relaxed);
    }
}
// DIAGNÓSTICO (2026-08-17, achado do dia): identificar QUAL CName recursa fundo em
// `exec_replacement` (padrão de 4 níveis visto via `lldb bt` em múltiplas threads simultâneas,
// mas `lldb` não consegue ler x0/`func` em frames não-internos do unwind — instrumentação
// própria contorna isso). Loga só quando a profundidade cruza um limiar (evita spam nos
// primeiros 1-2 níveis, que são normais) — throttled por thread via contador simples pra não
// crashar log() sob volume (mesmo que agora seja barato, ainda é I/O).
thread_local! {
    static DEEPTRACE_LOGGED: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

unsafe fn deeptrace_maybe_log(depth: u32, func: *mut c_void) {
    // 2026-08-21: gate PRÓPRIO (`~/.bwms-deeptrace`), não mais só `dev_mode()`. Isto é
    // diagnóstico de UMA investigação específica (o deadlock pós-autocontinue de 2026-08-17) que
    // já foi ENCERRADA — a causa era a cadeia de 87 `@wrapMethod`, resolvida pela consolidação em
    // 1 dispatcher. Ligado por padrão em todo boot de dev, seguia custando ~26k escritas de log
    // por boot (1 em 50 amostras, sem teto, sobre 1.3M+ chamadas) mais um `resolve_cname` por
    // amostra — e volume de I/O de `log()` já foi MEDIDO como contribuinte real de fragilidade de
    // boot (2026-08-20: o rate-limit global levou o crash de ~50% pra 0/3). Segue a regra da casa
    // (ESSENCIAL sempre roda, DIAGNÓSTICO atrás de gate): rearmar com
    // `touch ~/.bwms-deeptrace` quando alguém for investigar profundidade de execução de novo.
    if depth < 4 || !crate::dev_mode() || !crate::marcador_do_processo!(".bwms-deeptrace") {
        return;
    }
    let n = DEEPTRACE_LOGGED.with(|c| {
        let v = c.get();
        c.set(v.wrapping_add(1));
        v
    });
    // 2026-08-17 (achado do dia, deadlock pós-autocontinue): teto FIXO de 30 amostras/thread
    // esgotava cedo no boot (volume real observado: 1.3M+ chamadas na janela até o travamento),
    // nunca cobrindo o instante exato do lock órfão. Trocado pra AMOSTRAGEM PERIÓDICA sem teto
    // total — garante cobertura até o fim do boot (incl. o momento do travamento), custo
    // controlado pela taxa (1 em cada 50), `log()` já provado barato (fix de hoje, file-handle
    // cacheado). Log direto (não buffer-e-flush): se o processo travar/deadlockar, o canal de
    // comando (drenado no mesmo GameThread que trava) não consegue mais pedir um dump — só o que
    // já foi ESCRITO no disco antes do travamento sobrevive.
    if !crate::tempo::a_cada(u64::from(n), 50) {
        return;
    }
    let mut mcname = 0u64;
    if !func.is_null() && crate::memoria::is_readable(func as *const c_void, 0x18) {
        mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
    }
    let name = if mcname != 0 {
        crate::cname::resolve_cname(mcname)
    } else {
        "?".to_string()
    };
    crate::log_dev!(
        "[deeptrace] depth={depth} func={func:p} cname='{name}' (amostra #{n})"
    );
}

/// Substituição do executor (ABI: `func@x0, ctx@x1, frame@x2, aOut@x3, a4@x4 -> x0`).
/// Captura + tick periódico + chama a original.
unsafe extern "C" fn exec_replacement(
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    a_out: *mut c_void,
    a4: *mut c_void,
) -> *mut c_void {
    let execucao = ExecDepthGuard::enter();
    crate::rtti::capturar_alocador_do_frame(frame);
    deeptrace_maybe_log(exec_depth(), func);
    capture(ctx, execucao.contadores());
    // Override RUST-nativo (validação do suppress, sem lua): se `func` é o método alvo,
    // escreve o aOut tipado + SUPRIME a original (retorna). Caminho Rust-only → à prova do
    // aninhamento que crashava o cb lua. Fast-path: 1 load; se 0 (nada armado), custo nulo.
    #[cfg(feature = "devtools")]
    {
        let rov = RUST_OV_CNAME.load(Ordering::Relaxed);
        if rov != 0 && !func.is_null() && crate::memoria::is_readable(func as *const c_void, 0x18) {
            let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
            if mcname == rov {
                let val = RUST_OV_VAL.load(Ordering::Relaxed);
                // DIAGNÓSTICO: escrita i64 DIRETA, SEM chamadas de vtable (GetName/GetSize) —
                // isola se eram as vtable-calls no stack aninhado que crashavam. O aOut do
                // call_func é 16B (seguro escrever 8). (Só vale p/ o teste via call_func.)
                if !a_out.is_null() {
                    (a_out as *mut i64).write_unaligned(val);
                    crate::log_dev!("[ovrust] suppress: escrevi {val} no aOut, pulando original");
                    return 1usize as *mut c_void;
                }
            }
        }
    }
    // Observe RUST-nativo (`cet-hooks-shippable`, perna que faltava): se `func` é o alvo, LOGA que
    // o callback disparou e CAI FORA sem suprimir — a original roda normalmente logo depois.
    #[cfg(feature = "devtools")]
    {
        let robs = RUST_OBS_CNAME.load(Ordering::Relaxed);
        if robs != 0 && !func.is_null() && crate::memoria::is_readable(func as *const c_void, 0x18) {
            let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
            if mcname == robs {
                RUST_OBS_HITS.fetch_add(1, Ordering::Relaxed);
                crate::log_dev!("[ovrust] observe: callback disparou, NAO suprimindo (original roda)");
            }
        }
    }
    // DESCOBERTA (dev): ring de CNames p/ achar o handler do clique do botão MODS.
    if crate::dev_mode() {
        discovery_ring(func);
    }
    // captura nativa do FromTDBID (fn/ctx/ret) p/ os cheats de item — substitui a sonda
    // de fora do processo. Casa pelo endereço (FROMTD_TGT, resolvido no tick); 1 compare, barato.
    // FIX (2026-08-07, cont.41): gate `PHASE_REACHED_5` — RE offline (agente dedicado) provou
    // que o `ctx` capturado é passado CRU pro handler nativo (sem filtro/validação nenhuma no
    // dispatcher), então capturar o 1º caller que bate ANTES da gameplay real (ex.: algum
    // warm-up interno do motor, ctx "degenerado") trava o resto do boot inteiro com resultado
    // zero. Mesmo padrão de bug/fix já usado em `tweakxl_rt::create_once_if_marked` (GOG,
    // `CreateRecord`) — esperar `PHASE_REACHED_5` garante que só um caller de GAMEPLAY real
    // (mais provável de ter estado interno inicializado) seja capturado.
    let tgt = FROMTD_TGT.load(Ordering::Relaxed);
    if !tgt.is_null()
        && func == tgt
        && !ctx.is_null()
        && FROMTD_CTX.load(Ordering::Relaxed).is_null()
        && PHASE_REACHED_5.load(Ordering::Relaxed)
    {
        FROMTD_RET.store(a4, Ordering::Relaxed);
        FROMTD_CTX.store(ctx, Ordering::Relaxed); // por último: leitor vê RET pronto qdo CTX != null
        crate::log_dev!(
            "[fromtd-diag] capturado no CALLS={} ctx={ctx:p} ret_type={a4:p}",
            CALLS.load(Ordering::Relaxed)
        );
    }
    // Tabela por ponteiro do `scripts::sobrepor_funcao` (Before/After/Replace por `CBaseFunction*`, vanilla ou de outro mod). Lida UMA
    // vez por chamada: o mesmo valor decide o Before e o After, então os dois ficam pareados em toda saída (Replace, native nossa,
    // original) e o caminho comum paga 1 load atômico. Fora do dev nada registra alvo (só os comandos de prova `fnoverride*`,
    // e a API de plugin não expõe o módulo), então o `cfg!` deixa o compilador tirar o load e os 3 dispatch do público. Quando a
    // API expuser o módulo, o `cfg!` sai junto com a troca do `try_lock` do `TARGETS`, que descarta o gancho sob concorrência.
    let alvos = cfg!(feature = "devtools") && crate::scripts::sobrepor_funcao::has_targets();
    if alvos
        && crate::scripts::sobrepor_funcao::dispatch_before(func, ctx, frame, a_out, a4)
    {
        crate::scripts::sobrepor_funcao::dispatch_after(func, ctx, frame, a_out, a4);
        return 1usize as *mut c_void;
    }
    // ROTEAMENTO de nativas registradas (Codeware): se `func` é uma nativa que NÓS
    // registramos no RTTI, despacha pro handler Rust e retorna — sem cair na via nativa
    // do jogo (cujo regIndex/tabela global não conhece nossa função). Func do jogo quase
    // sempre volta no filtro de bits do route_native, com 1 load e sem trava.
    if let Some(h) = crate::register::route_native(func) {
        crate::register::set_current_native_func(func); // pro handler ler seus args do frame
        h(ctx, frame, a_out, a4 as i64);
        if alvos {
            crate::scripts::sobrepor_funcao::dispatch_after(func, ctx, frame, a_out, a4);
        }
        return 1usize as *mut c_void;
    }
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    if crate::tempo::a_cada(n, TICK_EVERY) {
        crate::cp77_tick();
    }
    // [fntrace] (rodada 32): opt-in, ver comentário do bloco acima. Só lê func+0x10 (mesmo
    // padrão já usado por `RUST_OV_CNAME` mais acima) quando ligado.
    if fntrace_enabled() && !func.is_null() && crate::memoria::is_readable(func as *const c_void, 0x18)
    {
        let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
        log_fn_trace(mcname, n);
    }
    // Observe/Override dos mods CET (`cet::observar`): método vigiado roda o `before`; com suppress (void, ou retorno POD já gravado no
    // `a_out`) a original não roda.
    #[cfg(feature = "lua")]
    let (suprimir, vigiado) = crate::cet::observar::watched_before(func, ctx, frame, a_out);
    #[cfg(feature = "lua")]
    if suprimir {
        return 1usize as *mut c_void;
    }
    let orig = ORIG_EXEC.load(Ordering::Relaxed) as *mut c_void;
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: ExecFn = std::mem::transmute(orig);
    let r = f(func, ctx, frame, a_out, a4);
    #[cfg(feature = "lua")]
    if vigiado != 0 {
        crate::cet::observar::watched_after(vigiado, ctx, a_out);
    }
    if alvos {
        crate::scripts::sobrepor_funcao::dispatch_after(func, ctx, frame, a_out, a4);
    }
    r
}

/// Identifica player/tx pela CLASSE do ctx (sem `/tmp`, sem nome). Resolve as
/// classes-alvo uma vez (quando o RTTI está pronto) e cacheia.
unsafe fn capture(ctx: *mut c_void, contadores: &ContadoresDoExecutor) {
    if ctx.is_null() {
        return;
    }
    // PERF: se o ctx já é o player/tx que conhecemos, pula o `class_of` (caro) —
    // cobre o caso comum de várias chamadas seguidas no mesmo objeto.
    if ctx == crate::current_player() || ctx == crate::current_tx() {
        return;
    }
    // Rate-limit da via cara (syscall): só 1 em cada CAPTURE_MISS_STRIDE misses desta thread tenta de
    // verdade. As outras voltam sem custo — ainda detectamos player/tx em poucas dezenas de
    // chamadas (o próprio hot-path garante isso), sem pagar syscall em TODO ctx desconhecido.
    // O u32 dá a volta em 2^32, múltiplo de CAPTURE_MISS_STRIDE: a cadência segue igual depois da volta.
    let n = contadores.misses_do_capture.get();
    contadores.misses_do_capture.set(n.wrapping_add(1));
    if !crate::tempo::a_cada(u64::from(n), CAPTURE_MISS_STRIDE) {
        return;
    }
    // O par do mundo que saiu fica fora até a carga do mundo novo terminar (`trocar_de_mundo`). Comparação antes do relógio: a recusa só
    // custa algo no ctx recusado.
    if (ctx == PLAYER_DO_MUNDO_QUE_SAIU.load(Ordering::Relaxed) || ctx == TX_DO_MUNDO_QUE_SAIU.load(Ordering::Relaxed)) && !carga_do_mundo_terminou() {
        return;
    }
    // Sem `is_readable` próprio: até o `class_of` o `ctx` só é comparado, nunca lido, e o `class_of`
    // prova o objeto e a vtable antes de ler. Um syscall (~360 ns) a menos por amostra.
    let mut tx_cls = CLS_TX.load(Ordering::Relaxed);
    let mut pl_cls = CLS_PL.load(Ordering::Relaxed);
    if tx_cls.is_null() {
        if let Some(reg) = crate::registry() {
            tx_cls = reg.class_by_name("gameTransactionSystem");
            pl_cls = reg.class_by_name("PlayerPuppet");
            if !tx_cls.is_null() {
                CLS_TX.store(tx_cls, Ordering::Relaxed);
            }
            if !pl_cls.is_null() {
                CLS_PL.store(pl_cls, Ordering::Relaxed);
            }
        }
        if tx_cls.is_null() {
            return; // RTTI ainda não pronto; tenta de novo na próxima chamada
        }
    }
    let c = rtti::class_of(ctx);
    if c.is_null() {
        return;
    }
    let qual = if c == tx_cls {
        crate::set_current_tx(ctx);
        "tx"
    } else if !pl_cls.is_null() && c == pl_cls {
        crate::set_current_player(ctx);
        "player"
    } else {
        return;
    };
    // A volta do player e do tx depois de `trocar_de_mundo` é a prova de boot da troca; 16 linhas cobrem menu e carga sem inundar se dois
    // puppets se alternarem.
    #[cfg(feature = "devlog")]
    {
        static CAPTURAS_LOGADAS: AtomicUsize = AtomicUsize::new(0);
        if CAPTURAS_LOGADAS.fetch_add(1, Ordering::Relaxed) < 16 {
            crate::log_dev!("[mundo] capturado {qual} {ctx:p} em t={}s (fase lida {})", boot_elapsed_secs(), FASE_LIDA.load(Ordering::Relaxed));
        }
    }
    let _ = qual; // sem `devlog` o bloco acima não existe
}

/// Gameplay real: o load do save terminou (`carga_do_save_terminou`), phase 5 há pelo menos 20 s e player vivo. O heartbeat usa o mesmo
/// critério para a linha `GAMEPLAY REAL`.
pub(crate) fn gameplay_real() -> bool {
    carga_do_save_terminou()
        && crate::inicio::boot::PHASE5_AT.get().map_or(false, |t| t.elapsed().as_secs() >= 20)
        && !crate::current_player().is_null()
}

pub(crate) const JOB_DO_FRAMEWORK_VM: u64 = 0x1_0157_8050;

pub(crate) const JOB_DE_ESTADO_PROLOGO: u32 = 0xd100_c3ff;

pub(crate) static ORIG_JOB_DO_FRAMEWORK: AtomicU64 = AtomicU64::new(0);

/// Depois da original: anota a execução de um job de estado para o `selftest` do C#239.
#[cfg(feature = "devtools")]
pub(crate) fn job_de_estado_visto(job: u8, estado: u8) {
    let ms = C239_RELOGIO.get_or_init(std::time::Instant::now).elapsed().as_millis();
    if let Ok(mut v) = C239_ESTADOS.lock() {
        if v.len() < 32 {
            v.push((job, estado, ms));
        }
    }
}

pub(crate) unsafe extern "C" fn job_do_framework_hook(params: *mut c_void, contexto: *mut c_void) {
    // A original solta os params: estado, framework e slot são lidos antes.
    #[cfg(feature = "devtools")]
    let estado = if crate::rtti::sane(params) { (params as *const u8).add(8).read() } else { 0xff };
    let framework = if crate::rtti::sane(params) { (params as *const u64).read() as *mut c_void } else { std::ptr::null_mut() };
    if crate::rtti::sane(framework) {
        CGAMEFRAMEWORK_ATUAL.store(framework as u64, Ordering::Relaxed);
    }
    #[cfg(feature = "devtools")]
    if C239_SLOT_FRAMEWORK.load(Ordering::Relaxed) == 0 && crate::rtti::sane(framework) {
        let obj = (framework as *const u64).add(2).read() as *mut c_void;
        let vtable = if crate::rtti::sane(obj) { (obj as *const u64).read() as *mut c_void } else { std::ptr::null_mut() };
        if crate::rtti::sane(vtable) {
            C239_SLOT_FRAMEWORK.store((vtable as *const u64).add(5).read(), Ordering::Relaxed);
        }
    }
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(original_publicado(&ORIG_JOB_DO_FRAMEWORK));
    f(params, contexto);
    #[cfg(feature = "devtools")]
    {
        job_de_estado_visto(2, estado);
        C239_JOBS.fetch_add(1, Ordering::Relaxed);
    }
}

/// No `on_load`: hook do job de estado do framework, com o `bwms-workspot.reds` (a `RuntimeScene` do C#82 sai do framework) no bundle, ou
/// `~/.bwms-hook-c239-teste`/`c82-teste`. O do job da sessão só anota para o `selftest`: só no dev.
pub(crate) unsafe fn install_jobs_de_estado() {
    let no_bundle = |nome: &str| crate::dylib_dir().is_some_and(|d| std::path::Path::new(&d).join("../r6/scripts/blackwall-mods").join(nome).exists());
    if !no_bundle("bwms-workspot.reds") && !marcador_individual("c239-teste") && !marcador_individual("c82-teste") {
        return;
    }
    #[cfg(feature = "devtools")]
    {
        let sessao = instalar_conferindo_prologo(C239_JOB_SESSAO_VM, JOB_DE_ESTADO_PROLOGO, c239_job_sessao_hook as *mut c_void, &ORIG_C239_JOB_SESSAO);
        crate::log_dev!("[c239] hook do job de estado da sessão no on_load: {sessao:?}");
    }
    let framework = instalar_conferindo_prologo(JOB_DO_FRAMEWORK_VM, JOB_DE_ESTADO_PROLOGO, job_do_framework_hook as *mut c_void, &ORIG_JOB_DO_FRAMEWORK);
    crate::log_dev!("[c239] hook do job de estado do framework no on_load: {framework:?}");
}

// ===== Codeware `#129`: `PlayerSpawnedHook` — hook no `cp::PlayerSystem::OnPlayerMainObjectSpawned` (2026-09-18) =====
// `0x100fffcb8` (RE offline 2026-09-18: 4 argumentos — `PlayerSystem`, entrada de spawn, parâmetros, `&Handle`; único chamador é o `operator()` do
// `std::function` do spawn do jogador, `0x101003c80`; grava o objeto principal em `this+0x604c0`). Como o HookAfter da referência, cada chamada vira um
// `Session/Ready` fora de sessão carregada de save; o hook só anota e o despacho sai no tick (`register::session_ready_tick`). A sessão carregada de save
// é marcada como no `OnGameLoad` da referência: hook de passagem no `OnGameLoad` do `questWorldStateSystem` (`0x102a2dfc8`, `(this, JobGroup&, bool&,
// stream)`, chamado na carga de todo save), consumido no spawn seguinte — o jogo novo e o mundo do menu não passam por ele. O `WasRestored` de
// `ScriptableSystem` não serve: o `SaveLocksManager` não tem dado persistente e volta falso também no save carregado (boot 244).
pub(crate) const PLAYER_MAIN_OBJECT_SPAWNED: (u64, u32) = (0x1_00ff_fcb8, 0xd101_c3ff);

pub(crate) const PLAYERSYSTEM_VTBL_VM: u64 = 0x1_0782_7490;

pub(crate) static ORIG_PLAYER_MAIN_OBJECT_SPAWNED: AtomicU64 = AtomicU64::new(0);

/// Um `(restored, End da sessão anterior pendente)` por spawn do objeto principal do jogador, pro despacho no tick.
pub(crate) static SPAWNS_DO_PLAYER_PENDENTES: std::sync::Mutex<Vec<(bool, bool)>> = std::sync::Mutex::new(Vec::new());

pub(crate) const QUESTWORLDSTATE_ONGAMELOAD: (u64, u32) = (0x1_02a2_dfc8, 0xd103_03ff);

pub(crate) static ORIG_QUESTWORLDSTATE_ONGAMELOAD: AtomicU64 = AtomicU64::new(0);

pub(crate) static SESSAO_CARREGADA_DE_SAVE: AtomicBool = AtomicBool::new(false);

pub(crate) unsafe extern "C" fn questworldstate_ongameload_hook(this: u64, grupo: u64, sucesso: u64, stream: u64) -> u64 {
    let f: unsafe extern "C" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_QUESTWORLDSTATE_ONGAMELOAD));
    let r = f(this, grupo, sucesso, stream);
    SESSAO_CARREGADA_DE_SAVE.store(true, Ordering::Relaxed);
    C129_CARGAS.fetch_add(1, Ordering::Relaxed);
    r
}

pub(crate) unsafe extern "C" fn player_main_object_spawned_hook(this: u64, entrada: u64, params: u64, handle: u64) -> u64 {
    let f: unsafe extern "C" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_PLAYER_MAIN_OBJECT_SPAWNED));
    let r = f(this, entrada, params, handle);
    let do_sistema = this != 0 && crate::memoria::is_readable(this as *const c_void, 8) && ler_u64(this) == crate::rebase(PLAYERSYSTEM_VTBL_VM) as u64;
    C129_CHAMADAS[usize::from(!do_sistema)].fetch_add(1, Ordering::Relaxed);
    if do_sistema {
        // Antes de enfileirar: o `session_ready_tick` reivindica o `Session/Ready` já na sessão aberta por este spawn; o `Session/End` da
        // anterior, se ficou pendente, sai antes dele, no tick (aqui é a thread do spawn, sem despacho).
        let fim_pendente = crate::inicio::sessao_do_jogo::abrir_sessao_no_spawn();
        let restaurada = SESSAO_CARREGADA_DE_SAVE.swap(false, Ordering::Relaxed);
        if let Ok(mut p) = SPAWNS_DO_PLAYER_PENDENTES.lock() {
            p.push((restaurada, fim_pendente));
        }
    }
    r
}

/// No `on_load`: o hook do produto; com `~/.bwms-hook-c129-teste`, o callback estático do teste pro `Session/Ready`, já antes do mundo do menu.
pub(crate) unsafe fn install_player_spawned_hook() {
    let h = instalar_conferindo_prologo(PLAYER_MAIN_OBJECT_SPAWNED.0, PLAYER_MAIN_OBJECT_SPAWNED.1, player_main_object_spawned_hook as *mut c_void, &ORIG_PLAYER_MAIN_OBJECT_SPAWNED);
    let carga = instalar_conferindo_prologo(QUESTWORLDSTATE_ONGAMELOAD.0, QUESTWORLDSTATE_ONGAMELOAD.1, questworldstate_ongameload_hook as *mut c_void, &ORIG_QUESTWORLDSTATE_ONGAMELOAD);
    let teste = marcador_individual("c129-teste") && crate::diagnostico::registrar_callback_estatico("Session/Ready", "BwmsC129Teste", "OnReady");
    crate::veredito_dev!("[c129] hook do OnPlayerMainObjectSpawned {h:?}, do OnGameLoad {carga:?}; callback estático de teste {teste}");
}


pub(crate) static GS01_FIRED: AtomicBool = AtomicBool::new(false);

/// RED4ext.SDK `#32`/`#36`/`#37`/`#38` (2026-08-11, fix desta sessão — GameStates cluster):
/// latch de `Initialization.OnExit` (type=1), ESCOPO DE ARQUIVO (antes era uma `static` LOCAL
/// dentro da closure da thread de heartbeat, disparada por leitura do byte de fase — achado de
/// cont.192 (`proofs/2026-08-11-red4ext-37-gamestates-onupdate-PARCIAL.log`): essa leitura é
/// FLAKY, às vezes o byte nunca chega a `5` de forma confiável naquela thread, mesmo quando a
/// transição real pra Running já aconteceu). Movido pra disparar no MESMO ponto/mecanismo já
/// PROVADO confiável pra `Running.OnEnter`/`OnExit` — a transição de presença do player dentro
/// de `cp77_tick` (ver bloco `[cbs]` abaixo). Latch único: `Initialization` só transiciona pra
/// `Running` 1x por processo (mesmo racional já usado pra `GS01_FIRED`).
pub(crate) static GS1_EXITED: AtomicBool = AtomicBool::new(false);


/// O `CGameFramework` da última execução do job: a `RuntimeScene` fica em `+0x18` (`register::cena_base`, C#82).
pub(crate) static CGAMEFRAMEWORK_ATUAL: AtomicU64 = AtomicU64::new(0);

