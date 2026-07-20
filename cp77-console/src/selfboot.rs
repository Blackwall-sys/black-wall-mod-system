//! Self-boot. Quando a dylib é carregada DIRETO pelo jogo (LC_LOAD_DYLIB "baked"),
//! um construtor instala o hook do executor (via gum, Rust puro) e dirige o runtime
//! sozinho. Na trilha de DEV, se outro injetor externo já dirige, fica PASSIVO p/
//! não duplicar o hook (gateado pela feature `dev-gadget`, fora do build público).
//!
//! Ganho de desempenho: a captura de player/tx vira comparação de classe em atomics
//! — **sem I/O de `/tmp` por chamada** — e o tick pesado roda ~1x a cada
//! [`TICK_EVERY`] chamadas, **sem FFI entre linguagens na via mais quente do jogo**.

use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use crate::rtti;

/// Executor universal (vmaddr; base de link 0x100000000). Mesmo do probe.js.
const EXEC_VM: u64 = 0x1_0217_3120;
/// Assinatura dos 8 primeiros bytes do executor (`stp x28,x27,[sp,#-0x60]!` +
/// `stp x26,x25,[sp,#0x10]`). Só hookamos se bater — nunca chuta no endereço, e
/// se um patch do jogo mover/mudar a função a gente ABORTA limpo (sem crash).
const EXEC_PROLOGUE: u64 = 0xa901_67fa_a9ba_6ffc;
/// Roda o tick pesado a cada N chamadas do executor (a captura roda sempre, é barata).
const TICK_EVERY: u64 = 2048;

static ACTIVE: AtomicBool = AtomicBool::new(false);

// ===== SPLASH DE BOOT: preenche a tela preta do loading (~35s) com feedback/branding =====
// Liga no on_load SE o skip-intro estiver ativo (só aí existe o espaço preto); desliga quando o
// auto-proceed detecta o menu (ou por timeout de segurança). O overlay lê estes sinais e desenha.
static BOOT_SPLASH_ON: AtomicBool = AtomicBool::new(false);
static BOOT_START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
static AUTOCONTINUE_FIRED: AtomicBool = AtomicBool::new(false);
/// Dead-man's switch do lever `BwmsFireStart` (ver register.rs::tramp_fire_start/tramp_autocontinue):
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
/// O lever d4=2 já foi disparado DIRETO pelo dylib (sem o timer redscript, que pós-reboot não armava)?
static LEVER_FIRED_DIRECT: AtomicBool = AtomicBool::new(false);
/// Instante em que o getter viu a engagement em repouso (d4==1,phase==1) pela 1ª vez desde o lock.
/// O disparo do lever espera um DWELL de wall-clock a partir daqui (independente de framerate) —
/// ver o bloco de disparo direto no getter. (Substituiu o antigo contador de 500 hits, que mapeava
/// num tempo real diferente por máquina — 500 hits ≈ <1s no dev, mas muito mais em FPS baixo.)
static LEVER_REST_SINCE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
pub(crate) static PHASE5_AT: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
/// phase==5 (getter nativo) dispara assim que o load é ENFILEIRADO, bem antes do mundo renderizar
/// (achado 2026-07-12) — tirar a splash direto nele revela a tela de loading nativa por trás.
/// 2 sinais mais precisos tentados e REFUTADOS in-game (ver blackwall-mods-dev/bwms-autocontinue.reds):
/// blackboard FastTravelSystem (nunca dispara pro nosso load) e PlayerPuppet.OnGameAttached (dispara
/// cedo demais, pega um player-placeholder do menu). Sem sinal melhor achado: grace period fixo.
const SPLASH_GRACE_AFTER_PHASE5_SECS: u64 = 8;
// WATCHDOG anti-hang de boot (2026-07-19). `BINK_RELEASED`: quando o watchdog (ou o branch de
// getter-recusado) desiste do skip, faz o `bink_open_replacement` parar de falhar os opens →
// libera o vídeo/engagement NATIVO em vez de manter a tela preta. `PHASE_MAX_SEEN`: a maior phase
// já observada pelo getter (boot sadio passa de 1 em segundos). Se após N segundos seguir <=1, o
// boot está travado (lever não disparou / fingerprint do save não bateu) → desiste com graça.
static BINK_RELEASED: AtomicBool = AtomicBool::new(false);
static PHASE_MAX_SEEN: AtomicI64 = AtomicI64::new(-1);
/// Segundos-desde-o-boot em que `PHASE_MAX_SEEN` subiu pela última vez — o watchdog mede "empacado há
/// N s" (dwell) em vez de só tempo-desde-o-boot, pra resgatar o travamento no MENU de forma responsiva.
static PHASE_MAX_CHANGED_ELAPSED: AtomicU64 = AtomicU64::new(0);
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
///    o splash (que só some em phase 5 ou na detecção de menu) fica cobrindo o menu pra sempre. Aqui
///    o watchdog tira o splash → o usuário VÊ o menu e clica "Continuar" (degrada pra o comportamento
///    do modo 1, em vez de tela travada). Ver `HISTORICO.md` (refino GOG 2026-07-19).
/// O pior caso vira "o jogo bootou normal, você vê o menu/APERTE ESPAÇO", NUNCA splash/tela preta
/// eterna. Idempotente. Chamado do getter (tick confiável no boot) E do cp77_tick (backstop).
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
    let max = PHASE_MAX_SEEN.load(Ordering::Relaxed);
    if max >= 5 {
        return; // chegou na GAMEPLAY → nada a resgatar (o splash-off normal cuida)
    }
    let elapsed = start.elapsed().as_secs();
    // 3 estados travados, cada um com o critério certo:
    let fire = if max <= 1 {
        // engagement: claramente quebrado → tempo-desde-o-boot (boot sadio passa em segundos).
        elapsed >= BOOT_HANG_WATCHDOG_SECS
    } else if max == 2 {
        // loading: pode ser só load lento → conservador, tempo-desde-o-boot longo.
        elapsed >= BOOT_STUCK_LOADING_WATCHDOG_SECS
    } else {
        // MENU (phase 3-4) empacado: DWELL — se ficou aqui há >30s sem virar gameplay, o
        // autocontinue falhou (ex.: GOG, lever perdeu a phase 1) → resgata rápido pra o usuário
        // ver o menu, sem esperar um tempo-desde-o-boot fixo longo.
        elapsed.saturating_sub(PHASE_MAX_CHANGED_ELAPSED.load(Ordering::Relaxed))
            >= BOOT_STUCK_MENU_DWELL_SECS
    };
    if !fire {
        return; // ainda dentro do prazo pra este estado
    }
    let onde = if max <= 1 {
        "na engagement (phase<=1)"
    } else if max == 2 {
        "no loading (phase 2)"
    } else {
        "no menu (phase 3-4, autocontinue não completou)"
    };
    crate::log(&format!(
        "[skipintro] WATCHDOG: boot travado {onde} após {elapsed}s -> tirando o splash + liberando o bink (o usuário vê o menu/tela nativa em vez do splash preso)"
    ));
    boot_splash_off();
    BINK_RELEASED.store(true, Ordering::Relaxed);
}

/// O splash de boot deve aparecer agora? (skip-intro ligado + ainda não chegou no menu).
pub fn boot_splash_active() -> bool {
    BOOT_SPLASH_ON.load(Ordering::Relaxed)
}
/// `BwmsAcFired` chama isto — alimenta o marco "autocontinue disparou" de `boot_progress()`.
pub(crate) fn note_autocontinue_fired() {
    AUTOCONTINUE_FIRED.store(true, Ordering::Relaxed);
}

/// Chamar 1x, o mais cedo possível no `on_load` (antes de `selfboot_if_needed`/qualquer hook) —
/// ver `STALE_BOOT_ATTEMPT`. Consome (apaga) o marcador se achar, pra não confundir o PRÓXIMO boot.
pub(crate) fn check_stale_boot_attempt() {
    if let Ok(h) = std::env::var("HOME") {
        let m = std::path::Path::new(&h).join(".bwms-boot-attempt");
        if m.exists() {
            let _ = std::fs::remove_file(&m);
            STALE_BOOT_ATTEMPT.store(true, Ordering::Relaxed);
            crate::log("[firestart] marcador de tentativa anterior encontrado (boot passado não fechou limpo) — auto-load fica suprimido NESTE boot por segurança");
        }
    }
}

/// `tramp_autocontinue` consulta isto em vez de reler o arquivo — reler o arquivo lá reagiria ao
/// marcador que O PRÓPRIO `tramp_fire_start` desta mesma sessão acabou de escrever (falso positivo
/// em TODO boot, não só nos que travaram). O valor é decidido 1x, cedo, antes do lever poder disparar.
pub(crate) fn autocontinue_suppressed_stale_boot() -> bool {
    STALE_BOOT_ATTEMPT.load(Ordering::Relaxed)
}

/// Desarma o dead-man's switch (`~/.bwms-boot-attempt`). Chamado (a) quando a GAMEPLAY é alcançada
/// — a etapa ARRISCADA (o auto-load do save) já provou que não crashou, então o marcador cumpriu
/// seu papel e pode sair, INDEPENDENTE de como o usuário fechar o jogo depois — e (b) no exit()
/// hookado. Antes SÓ o exit() limpava → fechar por "SAIR DO JOGO"/kill -9/watchdog do CDPR/força-quit
/// deixava o marcador armado → TODO boot seguinte caía pro menu (modo2→1) sem explicação. HOME
/// ausente = loga aviso e degrada pro modo SEGURO (o próximo boot cai pro menu, nunca crasha).
pub(crate) fn clear_boot_attempt_marker() {
    match std::env::var("HOME") {
        Ok(h) => {
            let _ = std::fs::remove_file(std::path::Path::new(&h).join(".bwms-boot-attempt"));
        }
        Err(_) => crate::log(
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
    } else if crate::overlay::seen_engagement() {
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
    } else if crate::overlay::seen_engagement() {
        "iniciando a sessão"
    } else {
        "carregando dados do jogo"
    }
}
/// Segundos decorridos desde o início do boot (p/ o texto "12s").
pub fn boot_elapsed_secs() -> u64 {
    BOOT_START.get().map(|t| t.elapsed().as_secs()).unwrap_or(0)
}
/// Skip-intro ligado? (marcador persistente `~/.bwms-skipintro` OU de sessão `/tmp/bwms-skipintro`).
fn skipintro_enabled() -> bool {
    std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false)
}
fn boot_splash_arm() {
    // ÂNCORA DE TEMPO DO BOOT — armada SEMPRE que o skip liga, independente de o splash VISUAL ser
    // desenhado (o watchdog anti-hang `boot_hang_watchdog` e o `boot_progress` dependem dela). Antes
    // o branch de opt-out retornava ANTES disto → com `~/.bwms-nosplash` a âncora nunca era setada e
    // o watchdog nunca dispararia. Agora fica no topo.
    let _ = BOOT_START.set(std::time::Instant::now());
    // OPT-OUT do SPLASH VISUAL (2026-07-09, +auto-off de RAM baixa 2026-07-19): renderizar o splash a
    // cada frame na tela preta contende no RENDERER mutex + churn de buffer Metal e pode STARVAR o
    // WindowServer em máquina apertada (risco de panic, ver [[cp77-boot-thermal-ceiling]]). Pula o
    // desenho quando: RAM física <= 8GB (auto, sem o usuário saber do dotfile) OU `~/.bwms-nosplash`.
    // Nesses casos fica só a tela preta de loading NATIVA = zero trabalho de GPU do BWMS no boot.
    let low_ram = sysctl_memsize_bytes()
        .map(|b| b <= 8u64 * 1024 * 1024 * 1024)
        .unwrap_or(false);
    let nosplash = std::path::Path::new("/tmp/bwms-nosplash").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-nosplash").exists())
            .unwrap_or(false);
    if low_ram || nosplash {
        crate::log(&format!(
            "[splash] splash de boot DESLIGADO ({}) — só a tela de loading nativa (menos GPU no boot apertado)",
            if low_ram { "RAM <= 8GB" } else { "~/.bwms-nosplash" }
        ));
        return; // BOOT_SPLASH_ON fica false → não desenha; a âncora acima segue armada
    }
    BOOT_SPLASH_ON.store(true, Ordering::Relaxed);
}

/// RAM física total em bytes (sysctl `hw.memsize`). None se a chamada falhar. Usado pelo auto-off
/// do splash em máquina de RAM baixa (<=8GB) — onde o desenho por-frame na tela preta pode starvar
/// o WindowServer.
fn sysctl_memsize_bytes() -> Option<u64> {
    extern "C" {
        fn sysctlbyname(
            name: *const i8,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> i32;
    }
    let mut v: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let r = unsafe {
        sysctlbyname(
            b"hw.memsize\0".as_ptr() as *const i8,
            &mut v as *mut u64 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if r == 0 && v > 0 {
        Some(v)
    } else {
        None
    }
}
pub(crate) fn boot_splash_off() {
    BOOT_SPLASH_ON.store(false, Ordering::Relaxed);
}
static ORIG_EXEC: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLS_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLS_PL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CALLS: AtomicU64 = AtomicU64::new(0);

// FromTDBID capturado NATIVAMENTE (fn/ctx/ret). Antes a sonda frida escrevia isso em
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
pub static RUST_OV_CNAME: AtomicU64 = AtomicU64::new(0);
pub static RUST_OV_VAL: AtomicI64 = AtomicI64::new(0);

/// `cet-hooks-shippable` — perna OBSERVE (a que faltava: Override+Suppress via `RUST_OV_*` já
/// provados, 2026-07-13). Mesmo mecanismo do Override, mas NÃO suprime: só loga que o callback
/// disparou e deixa a execução CAIR PRA FORA (a original roda normalmente depois). Prova que dá
/// pra "escutar" um método real sem alterar seu comportamento — o 3º modo do contrato CET
/// (Observe/Override/Suppress) por caminho não-Lua.
pub static RUST_OBS_CNAME: AtomicU64 = AtomicU64::new(0);
pub static RUST_OBS_HITS: AtomicU64 = AtomicU64::new(0);

/// Escreve `val` (i64 = a fonte) em `res` na LARGURA CORRETA do tipo de retorno, dado o NOME
/// (CName) e o SIZE PRÉ-COMPUTADOS — SEM chamar vtable (`type_size`/`type_name_getname` são
/// vtable-calls = a causa-raiz do crash no stack aninhado; ver notes/goal-next-steps.md). Trunca/
/// converte i64 pro POD. true = escreveu (tipo POD conhecido), false = recusa (não escreve lixo).
/// É o `write_pod_ret` type-aware do caminho no-lua; a INTEGRAÇÃO (pré-computar nome/size na
/// registração + chamar aqui no suppress em vez do i64 cru) é o próximo passo (não-crash = gated).
pub(crate) unsafe fn write_pod_ret_nolua(name: u64, size: u32, val: i64, res: *mut c_void) -> bool {
    if res.is_null() {
        return false;
    }
    use crate::cname::cname;
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

#[cfg(test)]
mod nolua_ret_tests {
    use super::*;
    use crate::cname::cname;
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

extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_name(i: u32) -> *const i8;
}

/// Construtor: o dyld chama isto quando carrega a dylib (modo baked OU via probe).
#[link_section = "__DATA,__mod_init_func"]
#[used]
static CTOR: extern "C" fn() = ctor;
extern "C" fn ctor() {
    unsafe { selfboot_if_needed() }
}

/// Estamos DENTRO do processo do jogo? (imagem 0 = executável principal). Evita
/// auto-bootar em testes/`dlopen` de validação, onde o executor rebaseia errado.
unsafe fn in_game() -> bool {
    // Procura "Cyberpunk2077" em TODAS as imagens, não só a índice 0. BUG (achado in-game
    // 2026-06-24 via diagnóstico): `_dyld_get_image_name(0)` NÃO é garantido ser o
    // executável principal no momento do ctor → in_game dava false → o self-boot NUNCA
    // instalava o hook do executor. O `game_base()` (lib.rs) já buscava por nome; in_game
    // agora faz igual. (A era frida injetava diferente, por isso não pegou esse bug antes.)
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
        crate::log("[selfboot] BWMS não reconheceu o processo do jogo (nenhuma imagem dyld casou 'Cyberpunk2077'/bundle-id) — hooks NÃO instalados");
    }
    false
}
static NO_GAME_LOGGED: AtomicBool = AtomicBool::new(false);

/// (Trilha DEV) Já existe um injetor externo dirigindo o runtime? Só compila no
/// build com a feature `dev-gadget` — o build PÚBLICO nem inclui essa checagem
/// (nem os literais de nome do injetor), pois lá sempre carregamos nativo.
#[cfg(feature = "dev-gadget")]
unsafe fn external_loader_present() -> bool {
    let n = _dyld_image_count();
    for i in 0..n {
        let nm = _dyld_get_image_name(i);
        if !nm.is_null() {
            let s = CStr::from_ptr(nm).to_string_lossy();
            if s.contains("FridaGadget") || s.contains("frida-gadget") {
                return true;
            }
        }
    }
    false
}

pub(crate) unsafe fn selfboot_if_needed() {
    // IDEMPOTENTE: chamado pelo ctor próprio (selfboot.rs) E pelo `on_load` (lib.rs).
    // Motivo: na build `--features lua` o ctor próprio do selfboot NÃO roda confiável
    // (luajit muda ordem/layout do __mod_init_func) → o `on_load`, que roda sempre,
    // dispara também. O guard evita instalar 2×.
    if ACTIVE.load(Ordering::Relaxed) {
        return;
    }
    let ig = in_game();
    crate::log(&format!("[selfboot] selfboot_if_needed: in_game={ig}"));
    if !ig {
        return; // testes/dlopen de validação: não toca em nada
    }
    // Trilha DEV: se um injetor externo já dirige, fica passivo (não duplica o hook).
    #[cfg(feature = "dev-gadget")]
    if external_loader_present() {
        crate::log("[bwms] injetor externo presente -> modo passivo");
        return;
    }
    crate::log("[bwms] modo runtime nativo");
    let target = crate::rebase(EXEC_VM);
    // GUARD: só hooka se os bytes do alvo forem MESMO o prólogo do executor.
    // Protege contra endereço errado / patch do jogo → aborta limpo, nunca crasha.
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[selfboot] alvo do executor ilegível -> abortando (sem crash)");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != EXEC_PROLOGUE {
        crate::log(&format!(
            "[selfboot] prólogo do executor não casou ({got:#018x} != {EXEC_PROLOGUE:#018x}) -> abortando (sem crash)"
        ));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, exec_replacement as *mut c_void) {
        Some(orig) => {
            ORIG_EXEC.store(orig, Ordering::Relaxed);
            ACTIVE.store(true, Ordering::Relaxed);
            std::mem::forget(it); // mantém o hook vivo pelo resto do processo
            crate::log("[bwms] hook do runtime instalado (nativo, Rust puro)");
            // Self-test DEV-GATED do hooking + diagnóstico ArchiveXL. SÓ roda sob
            // dev_mode() (no-op em produção); chamado DEPOIS do hook do executor estar
            // instalado. Não toca em nada do jogo num boot normal.
            if crate::dev_mode() {
                crate::selftest::run_dev_selftests();
            }
        }
        None => crate::log("[selfboot] FALHA ao hookar o executor"),
    }
    // F-B: install_bind_bridge() agora roda no TOPO do on_load (mais cedo que aqui), pois o bind
    // (RedScriptsHost::Load → orchestrator @0x1021e897c) roda muito cedo. register_all tb no cp77_tick. No selfboot/ctor o
    // CRTTISystem::Get crasha (RTTI não pronto). O bind do script é ~6s (antes de tudo isso) →
    // a ponte redscript→native precisa de gancho pós-RTTI-pré-script (RE pendente, binder ~0x2192xxx).
    // SAÍDA LIMPA (incondicional): mata a janela "Cyberpunk2077 quit unexpectedly" ao FECHAR.
    // Não é gateado — o crash de shutdown do RED4ext afeta todo fechamento.
    install_clean_exit();
    // App Nap OFF (incondicional): o macOS pausa o jogo quando ele não está na frente (janela
    // ocluída) → no CPVR, olhando o capacete, o jogo congela (boot lento + engagement travada +
    // câmera com "coices"). Desabilitar mantém o jogo a full em background. Barato, sem risco.
    crate::overlay::disable_app_nap();
    // Splash de boot: só arma se o skip-intro estiver ligado (aí a tela fica preta e vale preencher).
    if skipintro_enabled() {
        boot_splash_arm();
    }
    // Skip-intro (opt-in via marcador): pula os LOGOS de boot (funciona, ~10s).
    install_bink_skip();
    // AUTO-PROCEED da engagement ("APERTE [espaço] PARA CONTINUAR"): a tecla certa é SPACE (o ícone
    // é a barra de espaço, não "E"). Com Acessibilidade dada AO JOGO, o CGEvent injetado de dentro
    // pelo próprio processo É aceito (o que falhava antes era a tecla errada). Injeta SPACE numa
    // janela de tempo (a engagement aparece ~T+80s) até avançar. Opt-in ~/.bwms-skipintro.
    // GATEADO na feature "autoproceed": o build PÚBLICO não injeta tecla (sem CGEvent = sem padrão
    // "keylogger" no binário, e sem exigir Acessibilidade do usuário). O usuário aperta SPACE nas 2
    // telas HID; o skip do VÍDEO (install_bink_skip acima) e o auto-continue redscript seguem valendo.
    // CGEvent auto-proceed DESLIGADO — o usuário exige ZERO simulação de teclado/Acessibilidade. O skip
    // das 2 telas "APERTE ESPAÇO" agora é NATIVO pela PHASE BYTE: o getter @0x103f5ec74 lê a phase byte;
    // quando a engagement está ativa e a phase é 1(título)/2(loading), o hook MENTE 3 → o dispatcher de
    // boot segue o caminho oficial pro PreGameMenu (MENU) e as 2 telas somem de uma vez, SEM input.
    #[cfg(feature = "autoproceed")]
    let _ = install_auto_proceed; // referência p/ não warning; NÃO chama (sem CGEvent)
    install_phase_skip_near4(); // ATIVADO (near4-test provou o replace seguro; gate = engagement_active)
    let _ = install_phase_skip; // dispatcher-hook: NÃO instala (crashava).
    install_save_arm(); // SAVE-ARM: destrava o save-system no pregame SEM input (workflow RE 2026-07-05).

    // DIAGNÓSTICO near4 (F-A): a distância dylib↔__TEXT do jogo decide se o `B` de 4 bytes
    // alcança o alvo DIRETO (≤±128MB) ou se o replace_near4 precisa de um veneer near. Loga 1x.
    {
        let dylib_fn = exec_replacement as *const () as u64;
        let getter = crate::rebase(0x1_03f5_ec74) as u64;
        let dist = (dylib_fn as i64 - getter as i64).unsigned_abs();
        crate::log(&format!(
            "[near4] dylib={dylib_fn:#x} alvo_getter={getter:#x} dist={}MB B-direto-viavel={}",
            dist >> 20,
            dist < (1 << 27)
        ));
    }
    // F-A: replace_near4 no getter da phase byte JÁ está em uso pelo install_phase_skip_near4 (skip
    // real). O near4_test (passthrough gated ~/.bwms-near4test) cumpriu o papel de prova — não roda
    // junto pra não hookar o mesmo getter 2x.
    let _ = install_near4_test;
}

static NEAR4_ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static NEAR4_HITS: AtomicUsize = AtomicUsize::new(0);

/// Replacement de teste: passthrough (chama o original via trampolim) + conta/loga as chamadas.
unsafe extern "C" fn near4_test_getter(this: *mut u8) -> i32 {
    let n = NEAR4_HITS.fetch_add(1, Ordering::Relaxed);
    let orig = NEAR4_ORIG.load(Ordering::Relaxed);
    let v = if orig.is_null() {
        if !this.is_null() { (this.add(0x84) as *const i8).read() as i32 } else { 0 }
    } else {
        let f: unsafe extern "C" fn(*mut u8) -> i32 = std::mem::transmute(orig);
        f(this) // trampolim = ldrsb relocado + volta pro ret → devolve a phase real
    };
    if n < 3 {
        crate::log(&format!("[near4-test] getter chamado (hit #{n}) -> phase={v} (passthrough OK)"));
    }
    v
}

/// Hooka o getter @0x103f5ec74 com replace_near4 e mede a vizinha. Gated p/ não rodar em produção.
unsafe fn install_near4_test() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-near4test").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(0x1_03f5_ec74);
    let neigh = crate::rebase(0x1_03f5_ec7c);
    if !crate::gum::is_readable(neigh, 4) {
        crate::log("[near4-test] vizinha ilegível — abortando");
        return;
    }
    let before = core::ptr::read_unaligned(neigh as *const u32);
    let it = crate::gum::Interceptor::obtain();
    match it.replace_near4(target, near4_test_getter as *mut c_void) {
        Some(orig) => {
            NEAR4_ORIG.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            let after = core::ptr::read_unaligned(neigh as *const u32);
            crate::log(&format!(
                "[near4-test] getter HOOKADO via B de 4B. vizinha @0x3f5ec7c: {before:#010x} -> {after:#010x} INTACTA={}",
                before == after
            ));
        }
        None => crate::log("[near4-test] replace_near4 RECUSOU (alvo >128MB neste slide?)"),
    }
}

// ===== SKIP das telas "APERTE ESPAÇO" via HOOK DO GETTER da phase byte (opção #1 da nota) =====
// notes/boot-flow-phase-byte.md: o dispatcher de boot @0x103f70740 lê a phase byte pelo GETTER
// @0x103f5ec74 (`ldrsb w0,[x0,#0x84]; ret`) e faz SwitchState pela jump-table (1=engagement título,
// 2=initialize-user/loading, 3=PreGameMenu=MENU). Mentindo o getter (1/2 -> 3) o dispatcher segue o
// caminho OFICIAL pro menu → as DUAS telas de espera-por-input somem de uma vez, sem tocar o vídeo
// (logos são bink-skip) nem o piso de streaming (~30-40s de I/O, sem gate).
//
// Por que o getter e não o dispatcher (install_phase_skip, pausado): o dispatcher roda 1x cedo e não
// redispara; o getter é lido por +12 callers TODO frame → a mentira pega mesmo depois. Por que agora:
// o getter é leaf de 8B — replace normal (16B) transbordava na vizinha (SIGILL). replace_near4 (B de
// 4B) cabe SE dist<128MB; o diag [near4] mede ~111MB → viável+provado (vizinha INTACTA no near4-test).
static PHASE_GET_ORIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static PHASE_SKIP_HITS: AtomicUsize = AtomicUsize::new(0);
static PHASE_DIAG_HITS: AtomicUsize = AtomicUsize::new(0);
/// Última fase logada pelo getter — pra logar SÓ as MUDANÇAS de fase ao longo de todo o boot
/// (não só as 40 primeiras chamadas). Vê a sequência 1->2->3 quando o SPACE (ou o advance) avança.
static PHASE_LAST_LOGGED: AtomicI64 = AtomicI64::new(-128);

/// Getter da phase byte hookado: devolve a phase real (via trampolim); se for 1 (título) ou 2
/// (loading/user), MENTE 3 → o dispatcher troca pro PreGameMenu. NÃO escreve o campo (mentir só o
/// retorno preserva o estado real p/ quem lê +0x84 direto).
unsafe extern "C" fn phase_skip_getter(this: *mut u8) -> i32 {
    let orig = PHASE_GET_ORIG.load(Ordering::Relaxed);
    let real = if !orig.is_null() {
        let f: unsafe extern "C" fn(*mut u8) -> i32 = std::mem::transmute(orig);
        f(this)
    } else if !this.is_null() {
        (this.add(0x84) as *const i8).read() as i32
    } else {
        return 0;
    };
    // CAPTURA do GameSessionDesc: o getter é `ldrsb w0,[x0,#0x84]` → `this` (=x0) É o GameSessionDesc
    // (o dispatcher chama `mov x0,x20; bl getter` com x20=GameSessionDesc). Guarda o ponteiro quando a
    // fase é 1/2/3/5 (fase de sessão válida) + this+0x85 legível → destrava force_session_advance e
    // force_pregame_menu (que dependiam do dispatcher-hook, hoje OFF). Filtro pela fase evita capturar
    // um caller espúrio. Sempre re-armazena (o ponteiro é estável; a última leitura vale).
    // UMA leitura de legibilidade (0x100 cobre o 0x85 do capture E o 0xd4..0xff do fingerprint). Sob RAM
    // baixa cada is_readable (mach_vm_read) pega o vm_map lock e serializa atrás do pager do drive externo;
    // 2->1 por fire corta metade desse custo no getter (lido 12+×/frame) = P2 do fix do early-stick.
    let readable = !this.is_null() && crate::gum::is_readable(this as *const c_void, 0x100);
    if readable {
        // O getter `ldrsb [x0,#0x84]` é GENÉRICO. A EngagementScreenGameController (o SM que o lever d4=2
        // precisa) tem, no repouso da tela "APERTE ESPAÇO", d4==1 && phase==1 (rest state confirmado,
        // ENGAGEMENT-STATE-MACHINE.md:71; ou d4==4 se o check de sessão passa). Os callers ESPÚRIOS têm
        // phase=3/d4≠1 e sobrescreviam a captura → o lever escrevia no objeto errado (bug pós-reboot
        // 2026-07-15). TRAVA na engagement assim que a vê; depois disso ignora os espúrios (o ponteiro
        // é estável — mesmo que o d4 dela avance 1→2→3 pelo lever, mantemos ELA capturada).
        let d4_now = (this.add(0xd4) as *const u8).read();
        if real == 1 && (d4_now == 1 || d4_now == 4) {
            GAME_SESSION_DESC.store(this, Ordering::Relaxed);
            ENGAGEMENT_SM_LOCKED.store(true, Ordering::Relaxed);
        } else if !ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed) && (1..=5).contains(&real) {
            GAME_SESSION_DESC.store(this, Ordering::Relaxed); // fallback antigo, só até travar na engagement
        }
        // DISPARO DIRETO do lever (2026-07-15): o timer redscript (OnBwmsFireStart, 8s) NÃO estava
        // armando pós-reboot — o @wrapMethod OnInitialize da EngagementScreenGameController não dispara
        // (eng=false no phasedbg), então BwmsFireStart nunca era chamado. Com o SM certo agora capturado
        // (repouso d4==1 && phase==1), o dylib dispara o lever ELE MESMO, sem depender do redscript.
        // DWELL por WALL-CLOCK (2026-07-19): espera ~1.5s de repouso estável antes de disparar,
        // independente do framerate. Antes eram 500 hits do getter assumindo ~12×/frame (<1s) — mas
        // em máquina lenta / FPS baixo / overlay Steam atrasando o init de 50-90s, 500 hits mapeia
        // num tempo real muito diferente e o disparo direto erra a janela. fire_lever_direct() re-checa
        // TODO o gate (state/phase/guard/ctx/gate572) + dead-man no instante do disparo.
        if real == 1 && d4_now == 1
            && ENGAGEMENT_SM_LOCKED.load(Ordering::Relaxed)
            && !LEVER_FIRED_DIRECT.load(Ordering::Relaxed)
        {
            let since = LEVER_REST_SINCE.get_or_init(std::time::Instant::now);
            if since.elapsed().as_millis() >= 1500 && crate::register::fire_lever_direct() {
                LEVER_FIRED_DIRECT.store(true, Ordering::Relaxed);
            }
        }
    }
    // DIAGNÓSTICO: loga as PRIMEIRAS 40 chamadas + TODA mudança de fase depois. Não mente.
    let eng = crate::overlay::engagement_active();
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
            crate::log(&format!(
                "[phasedbg] getter#{n0} phase={real} d4={d4} e0={e0} fd={fd} fe={fe} ff={ff} eng={eng} this={this:p}"
            ));
        }
    }
    // FIX (2026-07-04, provado por [phasedbg]): mente phase 1 (engagement título) -> 3 (PreGameMenu) SEM
    // gate de eng — o getter lê phase=1 UMA vez ANTES do redscript setar engagement_active (getter#5
    // phase=1 eng=false). phase 2 (loading) e 3 (menu) ficam INTACTAS (não quebra o boot/streaming).
    // PHASE SKIP DESATIVADO: pular a engagement por completo (1->3 OU 1->2) trava o boot em 99% — a
    // engagement é um PASSO DE LOADING OBRIGATÓRIO (é onde o conteúdo/save-system inicializa). Provado
    // in-game (2026-07-04). O skip da engagement tem que ser MOSTRAR+DISMISSAR (no reds, quando a barra
    // de carregando vira "aperte para continuar" = OnAdditionalContentDataReloadProgress completa).
    // Mantido o hook (near4-test provou seguro) mas sem mentir — só passa a phase real.
    let _ = eng; // (o gate por eng era tarde demais; agora mentimos phase 1 direto, acima)
    // Modo "Até a gameplay": a splash fica coberta até aqui (tramp_engagement_off suprime o dismiss
    // dela nesse modo — ver register.rs). phase=5 = gameplay real, é o ÚNICO ponto de dismiss certo
    // (senão o usuário veria o menu/tela pós-load piscando no que devia ser 100% automático).
    // WATCHDOG: registra a MAIOR phase já vista (boot sadio passa de 1 em segundos) + tick do
    // watchdog (o getter é chamado repetidamente durante o boot travado — fonte confiável no boot,
    // ≠ cp77_tick que pode não tickar na tela preta). Ver `boot_hang_watchdog`.
    {
        let prev = PHASE_MAX_SEEN.load(Ordering::Relaxed);
        if (real as i64) > prev {
            PHASE_MAX_SEEN.store(real as i64, Ordering::Relaxed);
            // registra QUANDO a maior phase subiu (o watchdog mede "empacado há N s" a partir daqui).
            if let Some(t) = BOOT_START.get() {
                PHASE_MAX_CHANGED_ELAPSED.store(t.elapsed().as_secs(), Ordering::Relaxed);
            }
        }
    }
    boot_hang_watchdog();
    if real == 5 {
        // 1ª vez que a gameplay é alcançada: desarma o dead-man's switch AQUI (a etapa arriscada
        // — o auto-load — não crashou), independente de como o usuário fechar depois. Ver
        // `clear_boot_attempt_marker`. `swap` garante que só limpa 1x.
        if !PHASE_REACHED_5.swap(true, Ordering::Relaxed) {
            clear_boot_attempt_marker();
        }
        let _ = PHASE5_AT.set(std::time::Instant::now());
    }
    // Saída da splash: N segundos após phase==5 (2026-07-12: usar phase==5 direto tirava a splash
    // cedo demais — ele dispara quando o load é só ENFILEIRADO, revelando a tela de loading nativa
    // por trás. 2 sinais mais precisos tentados e refutados in-game, ver bwms-autocontinue.reds).
    if boot_splash_active() {
        // MODO 2 (autocontinue ON): splash até phase 5 + grace (esconde o flash do menu antes do save
        // carregar). MODO 1 (autocontinue OFF): o splash sai via o menu-dwell watchdog. (Tentei um
        // debounce por engagement-inativa pra tirar o splash mais cedo no menu, mas no GOG a engagement
        // fica ATIVA até ~93s — o sinal dispara ~no mesmo tempo do watchdog, não adianta; revertido.
        // GOG mode 1 = menu via watchdog, cosmético, não bug. Ver HISTORICO 2026-07-19 GOG.)
        let grace = PHASE5_AT
            .get()
            .map(|t| t.elapsed().as_secs() >= SPLASH_GRACE_AFTER_PHASE5_SECS)
            .unwrap_or(false);
        if grace {
            boot_splash_off();
        }
    }
    real
}

/// Hooka o getter da phase byte com replace_near4 (opt-in: mesmo marcador do bink-skip). No-op se
/// alvo ilegível / prólogo inesperado / dist>128MB — nunca crasha.
unsafe fn install_phase_skip_near4() {
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(0x1_03f5_ec74);
    // GUARDA REAL (2026-07-19): o prólogo do getter da phase byte é `ldrsb w0,[x0,#0x84]`
    // (0x39c21000) + `ret` (0xd65f03c0) — mesma instrução em Steam e GOG (só o endereço muda, que o
    // `rebase` resolve). Num binário onde o rebase caiu no lugar errado (versão do jogo != 2.31 /
    // build desconhecida), esses bytes NÃO batem → recusa limpa em vez de sobrescrever 8 bytes de
    // uma função não-relacionada (que crasharia quando ela rodasse). Antes só checava `is_readable`
    // (o comentário afirmava "confere o ret" mas a checagem não existia) — insuficiente.
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[skipintro] getter ilegível -> sem skip (sem crash)");
        return;
    }
    let i0 = (target as *const u32).read_unaligned();
    let i1 = (target as *const u32).add(1).read_unaligned();
    if i0 != 0x39c2_1000 || i1 != 0xd65f_03c0 {
        crate::log(&format!(
            "[skipintro] prólogo do getter não bate ({i0:#010x}/{i1:#010x}, esperado 0x39c21000/0xd65f03c0) -> sem skip (versão do jogo incompatível?)"
        ));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    if let Some(orig) = it.replace_near4(target, phase_skip_getter as *mut c_void) {
        PHASE_GET_ORIG.store(orig, Ordering::Relaxed);
        std::mem::forget(it);
        crate::log(&format!("[skipintro] getter da phase byte HOOKADO @ {target:p} via replace_near4 (1/2->3, telas de espaço -> menu)"));
        return;
    }
    // FALLBACK (2026-07-12, GOG): `replace_near4` (± B de 128MB, incl. seu próprio landing-pad
    // de ±128MB) recusou -- no GOG o slide empurra o alvo além de ±128MB E o kernel recusa
    // alocar memória nova naquela vizinhança. `replace_adrp_br8` usa ADRP+BR (±4GB de alcance,
    // 32x mais folga) com um landing-pad num `mmap` comum (sem MAP_FIXED, sempre sucede em
    // QUALQUER endereço) -- deveria caber com folga mesmo nesse layout.
    match it.replace_adrp_br8(target, phase_skip_getter as *mut c_void) {
        Some(orig) => {
            PHASE_GET_ORIG.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[skipintro] getter da phase byte HOOKADO @ {target:p} via replace_adrp_br8 (fallback GOG)"));
        }
        None => {
            // Os 2 métodos recusaram → o skip da phase não vai acontecer. MAS o splash já foi armado
            // e o bink-skip já está falhando os opens → sem isto, tela preta eterna. Desiste com graça
            // NA HORA (não espera o watchdog de 75s): tira o splash + libera o bink pro boot nativo.
            crate::log("[skipintro] getter RECUSOU nos 2 métodos (near4 e adrp_br8) -- sem skip; liberando vídeo/engagement nativo (sem tela preta)");
            boot_splash_off();
            BINK_RELEASED.store(true, Ordering::Relaxed);
        }
    }
}

/// Trampolim do `BinkShouldSkip` original (devolvido por `Interceptor::replace`).
static ORIG_BINK_SKIP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// SKIP-INTRO: hook em `BinkShouldSkip` (Bink SDK) → retorna 1 (pular) enquanto NÃO há
/// player vivo (fase de boot/menu) → os logos + a tela "APERTE ESPAÇO" pulam direto pro
/// menu. Em gameplay (player != null) chama a original (braindances/cutscenes tocam normal).
/// OPT-IN pelo marcador `/tmp/bwms-skipintro` (sem ele = zero efeito). dlsym; se não resolver,
/// no-op (sem crash).
/// AUTO-PROCEED da engagement ("APERTE [espaço] PARA CONTINUAR") — sobe uma thread que injeta SPACE
/// (keyCode 49) via CGEvent numa janela de tempo após o boot, até avançar pro menu. A tecla é SPACE
/// (o ícone é a barra de espaço, não "E" — provado 2026-07-02). O CGEvent injetado de DENTRO do
/// jogo é aceito porque a Acessibilidade foi concedida AO JOGO. Cap de segurança; SPACE no menu é
/// inofensivo (não navega). Se o redscript marcar engagement=false (avançou), para antes. Opt-in.
/// GATEADO na feature "autoproceed" (fora do build público): injeta tecla via CGEvent = padrão
/// "keylogger" pro heurístico AV + exige Acessibilidade. O splash-off migrou p/ tramp_engagement_off.
#[cfg(feature = "autoproceed")]
unsafe fn install_auto_proceed() {
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false);
    if !on {
        return;
    }
    std::thread::spawn(|| {
        // Injeta SPACE CEGO a cada 1s desde logo (não espera 48s): durante o streaming o SPACE é
        // ignorado; no INSTANTE que a tela de espaço aceita input, pula → sem espera. Para quando o
        // redscript marca engagement=false após true (menu) ou o cap. SPACE no menu é inofensivo.
        std::thread::sleep(std::time::Duration::from_secs(3));
        let mut seen_active = false;
        let mut reached_menu = false;
        // Janela LONGA (até ~30 min): o "engagement ativa=true" dispara no OnInitialize (fase de
        // LOADING, com barra de progresso), NÃO quando o "aperte espaço" fica pronto. No boot lento
        // (CPVR, máquina lenta) o loading passa de 70s → a janela curta expirava ANTES do "aperte
        // espaço" aparecer → travava. Agora injeta SPACE até o menu chegar (engagement ativa→inativa),
        // não importa quanto o loading demore. SPACE no loading/menu é inócuo. Sai no instante que passa.
        for n in 0..1800u32 {
            if crate::overlay::engagement_active() {
                seen_active = true;
            } else if seen_active {
                crate::log(&format!("[skipintro] auto-proceed: engagement encerrou (menu) em ~{}s.", n));
                boot_splash_off(); // menu chegou → esconde o splash (o menu tem arte própria)
                reached_menu = true;
                break;
            }
            crate::overlay::cg_press(49); // SPACE (o ícone é a barra de espaço, não "E")
            if n % 30 == 0 {
                crate::log(&format!("[skipintro] auto-proceed: aguardando o menu (SPACE #{n}, engagement={seen_active})"));
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
        boot_splash_off(); // segurança: some com o splash ao fim da janela (mesmo sem detectar o menu)
        let _ = reached_menu; // auto-continue agora é 100% redscript (aciona o CONTINUAR); sem injeção
        crate::log("[skipintro] auto-proceed: janela encerrada");
    });
}

// ===== SAÍDA LIMPA: elimina a janela de erro ao fechar o jogo =====
// O binário do jogo carrega o `RED4ext.dylib` (port C++, LC_LOAD do setup CPVR/memaxo). No
// `exit()`, `__cxa_finalize_ranges` roda o `RED4extShutdown`, que desfaz os detours dele e dá
// SIGSEGV em `DetourTransaction::Abort()` → o macOS abre "Cyberpunk2077 quit unexpectedly"
// (2 crash-reports idênticos confirmam: bug determinístico). O nosso runtime NÃO depende do
// RED4ext (otool -L: 0 link). Fix: interceptar o `exit` da libc e chamar `_exit(status)`, que
// termina o processo SEM rodar o `__cxa_finalize` (portanto sem o shutdown que crasha). O `exit`
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
    let n = std::panic::catch_unwind(|| unsafe { crate::register::fire_event("Session/End") }).unwrap_or(0);
    crate::log(&format!("[cleanexit] Session/End (onShutdown) -> {n} callback(s)"));
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
    crate::log(&format!(
        "[cleanexit] BwmsOnGameShutdown (onShutdown, funcao global) -> {}",
        if g.is_some() { "chamada OK" } else { "nao achada/nao chamou" }
    ));
    // Desarma o dead-man's switch do lever (ver register.rs::tramp_fire_start_state): chegar
    // aqui prova que o processo teve um exit() de verdade (não crash/hang/kill -9), então o
    // próximo boot pode disparar o lever normalmente de novo.
    if let Ok(h) = std::env::var("HOME") {
        let _ = std::fs::remove_file(std::path::Path::new(&h).join(".bwms-boot-attempt"));
    }
    crate::log(&format!(
        "[cleanexit] exit({status}) interceptado -> exit syscall (pula finalize/RED4extShutdown)"
    ));
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
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
    }
    let rtld_default = (-2isize) as *mut c_void; // macOS RTLD_DEFAULT
    let addr = dlsym(rtld_default, b"exit\0".as_ptr() as *const i8);
    if addr.is_null() {
        crate::log("[cleanexit] exit nao resolveu (dlsym) — sem fix");
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(addr, exit_replacement as *mut c_void) {
        Some(_orig) => {
            std::mem::forget(it);
            crate::log(&format!(
                "[cleanexit] hook em exit @ {addr:p} -> _exit (saida limpa, sem janela de erro)"
            ));
        }
        None => crate::log("[cleanexit] FALHA ao hookar exit"),
    }
}

unsafe fn install_bink_skip() {
    // Opt-in PERSISTENTE: `~/.bwms-skipintro` (sobrevive ao reboot) OU `/tmp/bwms-skipintro`
    // (sessão). Sem nenhum = no-op (boot normal com a intro).
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false);
    if !on {
        return;
    }
    extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const i8) -> *mut c_void;
    }
    let rtld_default = (-2isize) as *mut c_void; // macOS RTLD_DEFAULT
    let addr = dlsym(rtld_default, b"BinkOpenWithOptions\0".as_ptr() as *const i8);
    if addr.is_null() {
        crate::log("[skipintro] BinkOpenWithOptions nao resolveu (dlsym) — sem skip");
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(addr, bink_open_replacement as *mut c_void) {
        Some(orig) => {
            ORIG_BINK_SKIP.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[skipintro] hook em BinkOpenWithOptions @ {addr:p} (falha open de boot)"));
        }
        None => crate::log("[skipintro] FALHA ao hookar BinkOpenWithOptions"),
    }
}

// ===== F-B: HOOK DO GetFunction (provisão de native on-demand no bind do redscript) =====
// O redscript faz bind das `native func` no load (~6s, eager) chamando CRTTISystem::GetFunction
// (vtbl+0x30, impl @0x102195024 descoberto runtime); se devolve null p/ uma native nossa → SEGFAULT.
// register_all no tick é tarde (bind já passou); o RTTI não é acessível no ctor (Get crasha).
// SOLUÇÃO: inline-hook do GetFunction por ENDEREÇO ESTÁTICO no ctor (não precisa do RTTI, só
// patcha código mapeado) → quando o binder pede nossa native e a original dá null, a gente PROVÊ
// o POD on-demand (RTTI já está pronto em ~6s). TEST 1 = passthrough + loga o pedido (de-risco).
const GETFN_VM: u64 = 0x1_0219_5024;
static ORIG_GETFN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static GETFN_HITS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn getfn_hook(this: *mut c_void, cname: u64) -> *mut c_void {
    let orig = ORIG_GETFN.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(orig);
    let real = f(this, cname); // chama a original
    if real.is_null() {
        static OUR: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
        let our = *OUR.get_or_init(|| crate::cname::cname("BlackwallPing"));
        // DIAGNÓSTICO: loga os primeiros cnames NULL (onde BlackwallPing apareceria SE o binder
        // resolvesse por aqui). Compara com o cname logado no install.
        let n = GETFN_HITS.fetch_add(1, Ordering::Relaxed);
        if n < 60 {
            crate::log(&format!("[getfn] NULL #{n} cn={cname:#018x}{}", if cname == our { " <<< BLACKWALLPING" } else { "" }));
        }
        if cname == our {
            let pod = crate::register::provide_blackwallping(this, orig);
            crate::log(&format!("[getfn] >>> BlackwallPing pedido -> POD on-demand = {pod:p}"));
            return pod;
        }
    }
    real
}

unsafe fn install_getfn_hook() {
    let target = crate::rebase(GETFN_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[getfn] alvo ilegível -> sem hook");
        return;
    }
    let prologue = core::ptr::read_unaligned(target as *const u64);
    crate::log(&format!(
        "[getfn] GetFunction @ {target:p} prologue={prologue:#018x} | BlackwallPing cname={:#018x}",
        crate::cname::cname("BlackwallPing")
    ));
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, getfn_hook as *mut c_void) {
        Some(orig) => {
            ORIG_GETFN.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log("[getfn] hook instalado (Test 1 passthrough)");
        }
        None => crate::log("[getfn] FALHA ao hookar GetFunction (prólogo PC-relativo?)"),
    }
}

// ===== F-B: PONTE redscript→native (hook do bind orchestrator) =====
// O redscript binda `native func` no load (~6s); native não-registrada → crash depois (executor
// com regIndex lixo). O bind orchestrator @0x1021e897c monta o bind e DEPOIS entra no resolve-loop
// @0x1021e8c84. Hookar a ENTRADA + register_all antes da original → BlackwallPing no RTTI antes do
// binder procurar (resolve limpo). RTTI já vivo aqui (Get lazy/idempotente; o binder usa o mesmo
// RegisterFunction vtbl+0xA0). Prólogo limpo (sub sp/stp, zero PC-rel). Disasm verificado.
// #1 (orchestrator @0x1021e897c) NÃO disparou (entrada mid-função pelo resolve-loop). #2 @0x1021fcee0
// é a OUTRA fn que loga "Missing native global function" (resolve de global-native), entry limpo.
const BIND_ORCH_VM: u64 = 0x1_021f_cee0;
/// `sub sp,sp,#0x70` (d101c3ff) + `stp x22,x21,[sp,#0x40]` (a90457f6), LE como u64.
const BIND_ORCH_PROLOGUE: u64 = 0xa904_57f6_d101_c3ff;
static ORIG_BIND_ORCH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
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
    // DIAGNÓSTICO: loga TODA chamada (prova se o hook dispara de todo). Gated dev_mode.
    let c = ORCH_CALLS.fetch_add(1, Ordering::Relaxed);
    let diag = crate::dev_mode();
    if diag && c < 3 {
        crate::log(&format!("[fb] bind orch CHAMADO #{c} x0={x0:p}"));
    }
    // register_all UMA vez, ANTES da original (RTTI vivo). Guard anti-recursão: register_all só
    // toca Get + RegisterFunction (não chama o executor/bind), mas o swap garante zero loop.
    if !FB_REGISTER_DONE.load(Ordering::Acquire) && !FB_IN_REGISTER.swap(true, Ordering::AcqRel) {
        crate::register::register_all();
        FB_REGISTER_DONE.store(true, Ordering::Release);
        FB_IN_REGISTER.store(false, Ordering::Release);
        crate::log("[fb] bind orch entry: register_all feito (BlackwallPing no RTTI antes do bind)");
    }
    let orig = ORIG_BIND_ORCH.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, usize, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3, x4, x5, x6, x7);
    // Tentativa 12 (2026-07-13) — este é o handler de kind==5 (função global) no dispatcher de
    // validação (0x1021fbf90), confirmado por endereço idêntico ao BIND_ORCH_VM já hookado desde
    // a saga F-B. Com classes (kind=1, 2843/2843) e enums (kind=0, 5/5) 100% validados após os
    // fixes de hoje, a falha residual que ainda derruba o boot deve estar aqui (função global não
    // registrada a tempo) ou em kind=3/4 (ainda não instrumentados). Log leve só nas falhas.
    if diag && ret == 0 {
        let x1p = x1 as *const u8;
        let name_hash = if crate::gum::is_readable(x1p.add(8) as *const c_void, 8) {
            Some(core::ptr::read_unaligned(x1p.add(8) as *const u64))
        } else {
            None
        };
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::log(&format!(
            "[fb] bind orch (kind=5/global-func) FALHOU pra '{name}' (hash={:#018x}) x1={x1:#x}",
            name_hash.unwrap_or(0)
        ));
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
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[fb] bind orch ilegível -> sem ponte");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != BIND_ORCH_PROLOGUE {
        crate::log(&format!("[fb] bind orch não casou ({got:#018x}) -> sem ponte"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, bind_orch_hook as *mut c_void) {
        Some(orig) => {
            ORIG_BIND_ORCH.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[fb] ponte redscript→native instalada (bind orch @ {target:p})"));
        }
        None => crate::log("[fb] FALHA ao hookar o bind orchestrator"),
    }
}

// ===== F4 (Facade/CallbackSystem/Reflection): validador de CLASSE nativa — OBSERVE-ONLY =====
// Achado 2026-07-12 via a MESMA técnica do F-B (xref de string de erro do bind, não busca por
// símbolo): a família de strings "Missing native class '%hs'"/"Missing native function '%hs' in
// native class '%hs'" tem xrefs concentrados ao lado do BIND_ORCH_VM (0x1021fcee0, já hookado pra
// função GLOBAL). Mapeando os limites de função (prólogo/epílogo com stack-canary) achei
// 0x1021fc61c = função DIFERENTE, ainda não hookada — o validador de CLASSE (itera funcs+props
// de um "class descriptor" com offsets que não batem com o CClass real; hipótese: struct do
// bundle/compilador, não da RTTI nativa). É provavelmente quem emite o erro que crasha o boot
// quando o .reds da Facade é deployado (ver [[cp77-codeware-port]] pra RE completa).
//
// Esta sonda é PURAMENTE OBSERVACIONAL: loga os 3 primeiros registradores de entrada (candidatos
// a args, NUNCA interpretados/desreferenciados sem `gum::is_readable` primeiro) e SEMPRE chama o
// trampolim com TODOS os 8 registradores inalterados — mesmo padrão do bind_orch_hook, que não
// precisa saber a convenção de chamada exata pra ser seguro (só observa e repassa). Gated
// ~/.bwms-classvalidate-probe (OFF por padrão, separado do bind-bridge pra não interferir no
// mecanismo já provado). NÃO tenta registrar nada ainda — é só validar a hipótese de RE antes de
// arriscar uma injeção de registro (que seria o próximo passo, numa sessão futura).
const CLASS_VALIDATE_VM: u64 = 0x1_021f_c61c;
/// `sub sp,sp,#0xd0` (d10343ff) + `stp x28,x27,[sp,#0x70]` (a9076ffc), LE como u64.
const CLASS_VALIDATE_PROLOGUE: u64 = 0xa907_6ffc_d103_43ff;
static ORIG_CLASS_VALIDATE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLASSVAL_CALLS: AtomicUsize = AtomicUsize::new(0);

/// CName de "Codeware" pré-computado 1x (evita custo por chamada; a função dispara pra CADA
/// classe do bundle durante o bind — dezenas+ vezes por boot).
static CODEWARE_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static CODEWARE_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-callbacksystem-rtti` (2026-07-13) — mesma técnica da Facade, classe diferente
/// (`CallbackSystem extends IGameSystem`). Guard próprio, mesmo padrão de `CODEWARE_REGISTERED`.
static CALLBACKSYSTEM_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
/// `cw-scriptableservice`/`cw-callback-handler` (2026-07-15) — MESMA receita 2x já provada
/// (Facade + CallbackSystem), guards próprios por classe.
static SCRIPTABLESERVICE_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static SCRIPTABLESERVICE_REGISTERED: AtomicBool = AtomicBool::new(false);
static CALLBACKSYSTEMTARGET_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static CALLBACKSYSTEMTARGET_REGISTERED: AtomicBool = AtomicBool::new(false);
static SCRIPTABLESERVICECONTAINER_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static SCRIPTABLESERVICECONTAINER_REGISTERED: AtomicBool = AtomicBool::new(false);
static CALLBACKSYSTEMHANDLER_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static CALLBACKSYSTEMHANDLER_REGISTERED: AtomicBool = AtomicBool::new(false);
static CALLBACKSYSTEM_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-event-target-classes`/`cw-rawinput-realname` (2026-07-18, sessão `handle-ctor-re`,
/// retomada após 2 crashes em 2026-07-15 — ver nota grande em `callbacksystem-native.reds`).
/// MESMO padrão de guard por classe; `CallbackSystemEvent` primeiro (parent real de
/// `KeyInputEvent`, forjada como ABSTRATA via `register_type_min`, mesma receita 100% segura já
/// provada 4x hoje), `KeyInputEvent` depois (concreta, `register_type_instantiable_with_parent`
/// com parent = `CallbackSystemEvent` REAL — ao contrário das 2 tentativas de 2026-07-15, que
/// usavam um parent não-nativo ou pulavam a hierarquia real).
static CALLBACKSYSTEMEVENT_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static CALLBACKSYSTEMEVENT_REGISTERED: AtomicBool = AtomicBool::new(false);
static KEYINPUTEVENT_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static KEYINPUTEVENT_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-controller-session` (2026-07-18) — `GameSessionEvent extends CallbackSystemEvent`, MESMA
/// receita segura de `KeyInputEvent` (parent nativo real). Dispatch de "Session/Start"/"Session/
/// End" NÃO usa hook de world-attach/detach (categoria que causou a saga de crash do full-body,
/// `SystemsUpdater::Node::LinkJob` — ver memória `cp77-fullbody-fpp-posicao-casaco`) — reusa a
/// detecção de transição de presença do player JÁ SEGURA e JÁ PROVADA (`lib.rs::cp77_tick`, o
/// mesmo bloco que dispara "Player/Spawned"/"Player/Despawned").
static GAMESESSIONEVENT_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static GAMESESSIONEVENT_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-controller-entity` (2026-07-18) — `EntityLifecycleEvent extends CallbackSystemEvent`,
/// MESMA receita segura. Dispatch de "Entity/Attach" (nome REAL, `EntityAttachHook.hpp`) NÃO
/// hooka `Raw::Entity::Attach` (função nativa de ALTA FREQUÊNCIA, dispara pra TODA entidade do
/// mundo — hook novo e não-provado neste projeto) — reusa a MESMA transição de presença do
/// player já seguríssima, escopada só ao player (o caso de uso mais comum pra mods como
/// Equipment-EX/Cyberware-EX).
static ENTITYLIFECYCLEEVENT_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static ENTITYLIFECYCLEEVENT_REGISTERED: AtomicBool = AtomicBool::new(false);
/// `cw-controller-misc` (2026-07-19) — `ResourceEvent extends CallbackSystemEvent`, MESMA receita
/// segura. Dispatch de "Resource/Load" NÃO hooka `ResourceSerializer::SchedulePostLoadJobs` (RE
/// nova, offset Mac desconhecido) — reusa o hook `resource.link` JÁ INSTALADO E PROVADO
/// (`selftest.rs::reslink_lookup`, dispara em TODA construção real de `ResourcePath`) com um
/// "watch" (`watchres <path>`): quando o hash observado bate, `cp77_tick` dispara o evento
/// edge-triggered, MESMO padrão seguro de `Player/Spawned`/`Session/Start`.
static RESOURCEEVENT_CNAME: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
static RESOURCEEVENT_REGISTERED: AtomicBool = AtomicBool::new(false);
// `cw-rawinput-realname` — 2 tentativas, 2 crashes, ambas revertidas (2026-07-13). Ver
// `blackwall-mods-dev/callbacksystem-native.reds` pro histórico completo.
/// Separado de CODEWARE_REGISTERED (Tentativa 10): a CLASSE forja cedo (getorreg-probe) mas os
/// MÉTODOS Version/Require precisam do donor (gameGodModeSystem::AddGodMode) já existir — só
/// marca sucesso de verdade (não um swap-once cego), permitindo retry num hook mais tarde.
static CODEWARE_METHODS_REGISTERED: AtomicBool = AtomicBool::new(false);

/// Mesmo dump de [engine+0x150] do initscripts-probe (header@+0x14 = size:30|flag:2 packed),
/// reusado aqui pra comparar ANTES/DEPOIS do forge+register do Codeware dentro do próprio
/// class_validate_probe_hook (ver Tentativa 7).
unsafe fn dump_engine_error_container(engine: *const u8, label: &str) {
    if engine.is_null() {
        crate::log(&format!("[classval-probe] {label}: engine nulo/ilegível"));
        return;
    }
    let container = engine.add(0x150);
    if !crate::gum::is_readable(container as *const c_void, 0x30) {
        crate::log(&format!("[classval-probe] {label}: engine={engine:p} [engine+0x150] ilegível"));
        return;
    }
    let mut bytes = [0u8; 0x30];
    core::ptr::copy_nonoverlapping(container, bytes.as_mut_ptr(), 0x30);
    let header = u32::from_le_bytes([bytes[0x14], bytes[0x15], bytes[0x16], bytes[0x17]]);
    let size = header & 0x3FFF_FFFF;
    let flag = header >> 30;
    crate::log(&format!(
        "[classval-probe] {label}: engine={engine:p} [engine+0x150]: header@+0x14={header:#010x} (size={size} flag={flag}) bytes={bytes:02x?}"
    ));
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
    // tenta ler [x1+8] (CONFIRMADO 2026-07-12: CName do class-descriptor — 'IScriptable'/
    // 'Entity'/'WorldPosition' resolveram certo via cname::resolve_cname) SÓ se legível.
    let name_hash = if !x1.is_null() && crate::gum::is_readable(x1.add(8) as *const c_void, 8) {
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
        crate::log(&format!(
            "[classval-probe] CHAMADO #{c} x0={x0:p} x1={x1:p} x2={x2:p} [x1+8]={shown}"
        ));
    }
    // Tentativa 7 (2026-07-13): a tentativa 6 tinha 2 problemas achados na análise do log real:
    // (a) BUG no filtro de range de heap — usei 0x7000_0000_0000..0x8000_0000_0000 (13 dígitos
    // hex), mas TODO ponteiro heap real observado neste boot tem 10 dígitos (formato 0x7X_XXXX_XXXX,
    // ex. engine=0x7d9e405a30, classe forjada=0x7d9ec86930, x1 do validador=0x79c185fbc0) — o filtro
    // não batia em NADA, dando falso-negativo "zero candidatos" mesmo se o valor certo estivesse lá.
    // Corrigido pra 0x7000000000..0x8000000000. (b) a tentativa 3 já provou que o orquestrador
    // (0x103d99e44, dono do x20="engine") DISPARA E RETORNA antes da validação de classe começar —
    // sua stack frame não está mais na cadeia de frame-pointer daqui, então o stack-walk sozinho
    // não tem como achar o x20 dele (árvore errada por construção).
    // NOVA IDEIA (b): x2 é CONSTANTE entre as 12 chamadas de validação (mesmo endereço sempre,
    // 0x16bdadc18 no boot anterior) e bate quase exato com frame#2.fp+0x18 — é o contexto/resultado
    // do LOOP de validação (vive na stack do CHAMADOR, não do orquestrador). Em vez de só logar o
    // valor de x2, dumpar o CONTEÚDO apontado por ele pode revelar um ponteiro salvo pro mesmo
    // objeto "engine" (ou pro container de erro +0x150), sem depender da árvore de chamada do
    // orquestrador. Só na classe 'Codeware' (evita spam nos outros 12). Diagnóstico puro —
    // gated dev_mode (não afeta o forge, que roda incondicional mais abaixo).
    let cw_hash = *CODEWARE_CNAME.get_or_init(|| crate::cname::cname("Codeware"));
    if diag && name_hash == Some(cw_hash) {
        const HEAP_LO: u64 = 0x7000000000;
        const HEAP_HI: u64 = 0x8000000000;
        let mut dump = String::from("[classval-probe] Tentativa 7 — dump *x2 (contexto do loop) + stack-walk (range corrigido):\n");
        if !x2.is_null() && crate::gum::is_readable(x2 as *const c_void, 0x80) {
            let p2 = x2 as *const u64;
            for i in 0..16usize {
                let v = p2.add(i).read();
                let tag = if v > HEAP_LO && v < HEAP_HI && crate::gum::is_readable(v as *const c_void, 8) {
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
            if fp.is_null() || !crate::gum::is_readable(fp as *const c_void, 0x10) {
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
                if !crate::gum::is_readable(addr as *const c_void, 8) {
                    continue;
                }
                let v = addr.read();
                if v > HEAP_LO && v < HEAP_HI && crate::gum::is_readable(v as *const c_void, 8) {
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
    // FIX (2026-07-13, tentativa 2): a tentativa 1 (chamar só register_codeware_facade, que
    // ASSUME a classe já existe) falhou porque a classe REALMENTE não existe ainda neste ponto
    // ("classe não existe"). Desta vez: FORJAMOS a classe NÓS MESMOS via register_type_min
    // (STEP-1 do RegisterType, já provado desde 2026-07-04 — mesmas flags isAbstract|isNative
    // que a declaração real `public abstract native class Codeware`), e SÓ DEPOIS registramos
    // os métodos nativos nela — tudo ANTES de deixar o validador original rodar.
    // ESTE BLOCO É O FIX DE PRODUÇÃO — roda SEMPRE, não só com dev_mode (ver nota no topo da fn).
    if name_hash == Some(cw_hash) && !CODEWARE_REGISTERED.swap(true, Ordering::AcqRel) {
        // Tentativa 7 cont. (2026-07-13): a tentativa 6 provou que *(x2+0x60) É o mesmo ponteiro
        // "engine" que o initscripts-probe reporta de forma independente no MESMO boot (valor
        // idêntico, bit-a-bit). Logo dá pra ler [engine+0x150] (o container de erro do assert
        // baseEngineInit.cpp:1094) diretamente daqui, ANTES e DEPOIS do nosso forge+register —
        // se o bit de erro mudar exatamente nessa janela, é prova direta de que O NOSSO
        // REGISTRO É QUEM DISPARA o assert (não uma classe anterior/outro caminho). Diagnóstico
        // puro (leitura sem efeito colateral) — gated dev_mode só pra não gastar ciclos à toa.
        let engine: *const u8 = if diag && !x2.is_null() && crate::gum::is_readable(x2.add(0x60) as *const c_void, 8) {
            core::ptr::read_unaligned(x2.add(0x60) as *const *const u8)
        } else {
            std::ptr::null()
        };
        if diag {
            dump_engine_error_container(engine, "ANTES do forge+register");
        }
        crate::log("[classval-probe] classe 'Codeware' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let forged = crate::register::register_type_min(&reg, "Codeware");
            crate::log(&format!("[classval-probe] register_type_min('Codeware') -> {forged:p}"));
            let r = crate::register::register_codeware_facade(&reg);
            crate::log(&format!("[classval-probe] register_codeware_facade -> {r}"));
        } else {
            crate::log("[classval-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
        }
        if diag {
            dump_engine_error_container(engine, "DEPOIS do forge+register");
        }
    }
    // `cw-callbacksystem-rtti` (2026-07-13) — MESMO fix da Facade (parent explícito + fullName
    // bare), classe DIFERENTE (`CallbackSystem extends IGameSystem`, instanciável). Bloco
    // ESSENCIAL, roda SEMPRE (não gated por dev_mode) — mesma justificativa de produção do bloco
    // do Codeware acima: se isso rodasse só atrás de marker de dev, um mod real que declare
    // `callbacksystem-native.reds` crasharia o boot pra qualquer usuário sem o marker.
    let cbs_hash = *CALLBACKSYSTEM_CNAME.get_or_init(|| crate::cname::cname("CallbackSystem"));
    if name_hash == Some(cbs_hash) && !CALLBACKSYSTEM_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'CallbackSystem' detectada no validador — forjando+registrando AGORA");
        // DIAGNÓSTICO (não-gated, 1x só, precisamos disto AGORA): qual nome exato o compilador
        // declarou como base de `extends IGameSystem`? Mesma leitura já provada pro Codeware
        // (Tentativa 11 abaixo) — `[x1+0x20]` = descritor da base declarada, `[+0x8]` = CName.
        if !x1.is_null() && crate::gum::is_readable(x1.add(0x20) as *const c_void, 8) {
            let decl_base_desc = core::ptr::read_unaligned(x1.add(0x20) as *const *mut u8);
            if decl_base_desc.is_null() {
                crate::log("[classval-probe] CallbackSystem: sem base declarada explícita (inesperado — o .reds tem extends)");
            } else if crate::gum::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
                let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
                let decl_name = crate::cname::resolve_cname(decl_cname);
                crate::log(&format!(
                    "[classval-probe] CallbackSystem base declarada: CName={decl_cname:#018x} nome='{decl_name}' — usar este nome exato como parent"
                ));
            }
        }
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_callbacksystem(&reg);
            crate::log(&format!("[classval-probe] register_callbacksystem -> {r}"));
            // DIAGNÓSTICO (mesma técnica da Tentativa 11, parametrizada p/ CallbackSystem/
            // IGameSystem em vez de Codeware/IScriptable) — roda IMEDIATAMENTE após o forge, pra
            // prever se o IsKindOf walk + o check de base-declarada vão passar, ANTES do validador
            // ORIGINAL rodar (log logo abaixo, fora deste bloco).
            let cbs_cls = reg.class_by_name("CallbackSystem");
            let igs_cls = reg.class_by_name("IGameSystem");
            crate::log(&format!("[classval-probe] CBS diag: CallbackSystem={cbs_cls:p} IGameSystem={igs_cls:p}"));
            let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_0223_809c));
            let expected_base = getter(); // sempre IScriptable (universal, confirmado p/ Codeware)
            let mut cur = cbs_cls;
            let mut found = false;
            for depth in 0..16u32 {
                if cur.is_null() {
                    crate::log(&format!("[classval-probe] CBS IsKindOf walk: profundidade={depth} cur=NULL — parando"));
                    break;
                }
                if cur as u64 == expected_base as u64 {
                    found = true;
                    crate::log(&format!("[classval-probe] CBS IsKindOf walk: profundidade={depth} cur={cur:p} == expected_base(IScriptable) — ACHOU"));
                    break;
                }
                if !crate::gum::is_readable((cur as *const u8).add(0x10) as *const c_void, 8) {
                    crate::log(&format!("[classval-probe] CBS IsKindOf walk: profundidade={depth} cur={cur:p} [cur+0x10] ilegível — parando"));
                    break;
                }
                let next = core::ptr::read_unaligned((cur as *const u8).add(0x10) as *const *mut c_void);
                crate::log(&format!("[classval-probe] CBS IsKindOf walk: profundidade={depth} cur={cur:p} parent[+0x10]={next:p}"));
                cur = next;
            }
            crate::log(&format!(
                "[classval-probe] CBS IsKindOf PREVISTO: {}",
                if found { "SUCESSO" } else { "FALHA" }
            ));
            if !x1.is_null() && crate::gum::is_readable(x1.add(0x20) as *const c_void, 8) {
                let decl_base_desc = core::ptr::read_unaligned(x1.add(0x20) as *const *mut u8);
                if !decl_base_desc.is_null() && crate::gum::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
                    let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
                    let getter2: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_0218_85a0));
                    let singleton2 = getter2();
                    if !singleton2.is_null() && crate::gum::is_readable(singleton2 as *const c_void, 8) {
                        let vt = core::ptr::read_unaligned(singleton2 as *const *mut u8);
                        if !vt.is_null() && crate::gum::is_readable(vt.add(0x108) as *const c_void, 8) {
                            let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
                            if crate::rtti::sane(slot) {
                                let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
                                let resolved = getclass(singleton2, decl_cname);
                                crate::log(&format!(
                                    "[classval-probe] CBS base declarada resolvida={resolved:p} vs nosso parent(IGameSystem)={igs_cls:p} (bate={})",
                                    resolved as u64 == igs_cls as u64
                                ));
                            }
                        }
                    }
                }
            }
            if !cbs_cls.is_null() && crate::gum::is_readable(cbs_cls as *const c_void, 0x74) {
                let flags = core::ptr::read_unaligned((cbs_cls as *const u8).add(0x70) as *const u32);
                crate::log(&format!("[classval-probe] CallbackSystem flags@+0x70 = {flags:#010x}"));
            }
        } else {
            crate::log("[classval-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
        }
    }
    // `cw-scriptableservice`/`cw-callback-handler` (2026-07-15) — MESMA receita já provada 2x
    // (Facade sem-extends + CallbackSystem extends IGameSystem), zero RE nova. Bloco ESSENCIAL
    // (não gated dev_mode), mesma justificativa: um .reds real que declare estas classes crasharia
    // o boot pra qualquer usuário sem o forge rodar a tempo.
    let ss_hash = *SCRIPTABLESERVICE_CNAME.get_or_init(|| crate::cname::cname("ScriptableService"));
    if name_hash == Some(ss_hash) && !SCRIPTABLESERVICE_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'ScriptableService' detectada no validador — forjando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_scriptableservice(&reg);
            crate::log(&format!("[classval-probe] register_scriptableservice -> {r}"));
        }
    }
    let cst_hash = *CALLBACKSYSTEMTARGET_CNAME.get_or_init(|| crate::cname::cname("CallbackSystemTarget"));
    if name_hash == Some(cst_hash) && !CALLBACKSYSTEMTARGET_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'CallbackSystemTarget' detectada no validador — forjando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_callbacksystemtarget(&reg);
            crate::log(&format!("[classval-probe] register_callbacksystemtarget -> {r}"));
        }
    }
    let ssc_hash = *SCRIPTABLESERVICECONTAINER_CNAME.get_or_init(|| crate::cname::cname("ScriptableServiceContainer"));
    if name_hash == Some(ssc_hash) && !SCRIPTABLESERVICECONTAINER_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'ScriptableServiceContainer' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_scriptableservicecontainer(&reg);
            crate::log(&format!("[classval-probe] register_scriptableservicecontainer -> {r}"));
        }
    }
    let csh_hash = *CALLBACKSYSTEMHANDLER_CNAME.get_or_init(|| crate::cname::cname("CallbackSystemHandler"));
    if name_hash == Some(csh_hash) && !CALLBACKSYSTEMHANDLER_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'CallbackSystemHandler' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_callbacksystemhandler(&reg);
            crate::log(&format!("[classval-probe] register_callbacksystemhandler -> {r}"));
        }
    }
    // `cw-event-target-classes`/`cw-rawinput-realname` RETRY (2026-07-18, sessão `handle-ctor-re`)
    // — ver nota grande na declaração de `CALLBACKSYSTEMEVENT_CNAME` acima.
    let cse_hash = *CALLBACKSYSTEMEVENT_CNAME.get_or_init(|| crate::cname::cname("CallbackSystemEvent"));
    if name_hash == Some(cse_hash) && !CALLBACKSYSTEMEVENT_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'CallbackSystemEvent' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_callbacksystemevent(&reg);
            crate::log(&format!("[classval-probe] register_callbacksystemevent -> {r}"));
        }
    }
    let kie_hash = *KEYINPUTEVENT_CNAME.get_or_init(|| crate::cname::cname("KeyInputEvent"));
    if name_hash == Some(kie_hash) && !KEYINPUTEVENT_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'KeyInputEvent' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_keyinputevent(&reg);
            crate::log(&format!("[classval-probe] register_keyinputevent -> {r}"));
        }
    }
    let gse_hash = *GAMESESSIONEVENT_CNAME.get_or_init(|| crate::cname::cname("GameSessionEvent"));
    if name_hash == Some(gse_hash) && !GAMESESSIONEVENT_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'GameSessionEvent' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_gamesessionevent(&reg);
            crate::log(&format!("[classval-probe] register_gamesessionevent -> {r}"));
        }
    }
    let ele_hash = *ENTITYLIFECYCLEEVENT_CNAME.get_or_init(|| crate::cname::cname("EntityLifecycleEvent"));
    if name_hash == Some(ele_hash) && !ENTITYLIFECYCLEEVENT_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'EntityLifecycleEvent' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_entitylifecycleevent(&reg);
            crate::log(&format!("[classval-probe] register_entitylifecycleevent -> {r}"));
        }
    }
    let re_hash = *RESOURCEEVENT_CNAME.get_or_init(|| crate::cname::cname("ResourceEvent"));
    if name_hash == Some(re_hash) && !RESOURCEEVENT_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[classval-probe] classe 'ResourceEvent' detectada no validador — forjando+registrando AGORA");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_resourceevent(&reg);
            crate::log(&format!("[classval-probe] register_resourceevent -> {r}"));
        }
    }
    // Tentativa 11 (2026-07-13, sessão seguinte) — o parent-pointer fix (Tentativa 10) rodou ao
    // vivo mas o validador ORIGINAL continuou retornando 0 pro Codeware. Em vez de hookar
    // `0x10219ac60` (IsKindOf) direto — função pequena em tight-loop, mesmo risco de transbordo
    // de redirect já documentado pra outros leafs — reimplementamos a MESMA caminhada em Rust
    // aqui dentro (hook já provado seguro), lendo os ponteiros REAIS: o getter do base esperado
    // (`0x10223809c`), a classe forjada, e caminhando `cls+0x10` como o disasm de 0x1021fc61c
    // mostrou. Também loga os bits de `flags@+0x70` (isNative/isImportOnly já OK por construção;
    // bits 7/8/9 são checks ADICIONAIS pós-IsKindOf que o disasm revelou, ainda não instrumentados)
    // e os bytes do descritor `x1+0x88`/`+0x89` que esses checks também consultam.
    // Diagnóstico puro (não altera o forge, só re-lê estado pra logar) — gated dev_mode.
    if diag && name_hash == Some(cw_hash) {
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let cw_cls = reg.class_by_name("Codeware");
            let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_0223_809c));
            let expected_base = getter();
            let iscriptable = reg.class_by_name("IScriptable");
            crate::log(&format!(
                "[classval-probe] IsKindOf diag: Codeware={cw_cls:p} expected_base(getter)={expected_base:p} IScriptable(by_name)={iscriptable:p} (bate={})",
                expected_base as u64 == iscriptable as u64
            ));
            let mut cur = cw_cls;
            let mut found = false;
            for depth in 0..16u32 {
                if cur.is_null() {
                    crate::log(&format!("[classval-probe] IsKindOf walk: profundidade={depth} cur=NULL — parando"));
                    break;
                }
                if cur as u64 == expected_base as u64 {
                    found = true;
                    crate::log(&format!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} == expected_base — ACHOU"));
                    break;
                }
                if !crate::gum::is_readable((cur as *const u8).add(0x10) as *const c_void, 8) {
                    crate::log(&format!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} [cur+0x10] ilegível — parando"));
                    break;
                }
                let next = core::ptr::read_unaligned((cur as *const u8).add(0x10) as *const *mut c_void);
                crate::log(&format!("[classval-probe] IsKindOf walk: profundidade={depth} cur={cur:p} parent[+0x10]={next:p}"));
                cur = next;
            }
            crate::log(&format!(
                "[classval-probe] IsKindOf PREVISTO: {}",
                if found { "SUCESSO" } else { "FALHA (não achou expected_base na cadeia de parent)" }
            ));
            if !cw_cls.is_null() && crate::gum::is_readable(cw_cls as *const c_void, 0x74) {
                let flags = core::ptr::read_unaligned((cw_cls as *const u8).add(0x70) as *const u32);
                crate::log(&format!(
                    "[classval-probe] Codeware flags@+0x70 = {flags:#010x} (bit1_isNative={} bit6_isImportOnly={} bit7={} bit8={} bit9={})",
                    (flags >> 1) & 1, (flags >> 6) & 1, (flags >> 7) & 1, (flags >> 8) & 1, (flags >> 9) & 1
                ));
            }
            if !x1.is_null() && crate::gum::is_readable(x1.add(0x89) as *const c_void, 1) {
                let b88 = *x1.add(0x88);
                let b89 = *x1.add(0x89);
                crate::log(&format!("[classval-probe] descriptor bytes: [x1+0x88]={b88:#x} [x1+0x89]={b89:#x}"));
            }
            // Tentativa 11 cont. — achado no disasm de 0x1021fc9f4..0x1021fcb1c: um SEGUNDO check,
            // INDEPENDENTE do IsKindOf, lê `[x1+0x20]` (ponteiro pra descritor da "base declarada"
            // do lado do bundle/compilador — só não-nulo se o `.reds` tiver `extends` explícito),
            // resolve o CName dela (`[+0x8]` do descritor, mesmo padrão de `[x1+8]` da própria
            // classe) via GetClass (singleton 0x1021885a0 + vtbl+0x108 — MESMA via já confirmada
            // resolver nossa própria classe forjada), e COMPARA contra `[x21+0x10]` (nosso parent
            // real). Strings de erro batem: "Native class has base class that is different..."/
            // "...that is not imported". Se isso divergir, é a causa REAL da falha residual.
            if !x1.is_null() && crate::gum::is_readable(x1.add(0x20) as *const c_void, 8) {
                let decl_base_desc = core::ptr::read_unaligned(x1.add(0x20) as *const *mut u8);
                crate::log(&format!("[classval-probe] descriptor[x1+0x20] (base declarada) = {decl_base_desc:p}"));
                if decl_base_desc.is_null() {
                    crate::log("[classval-probe] sem base declarada explícita (.reds sem `extends`) — Path A do disasm");
                } else if crate::gum::is_readable(decl_base_desc.add(8) as *const c_void, 8) {
                    let decl_cname = core::ptr::read_unaligned(decl_base_desc.add(8) as *const u64);
                    let decl_name = crate::cname::resolve_cname(decl_cname);
                    crate::log(&format!("[classval-probe] base declarada: CName={decl_cname:#018x} nome='{decl_name}'"));
                    let getter2: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_0218_85a0));
                    let singleton2 = getter2();
                    if !singleton2.is_null() && crate::gum::is_readable(singleton2 as *const c_void, 8) {
                        let vt = core::ptr::read_unaligned(singleton2 as *const *mut u8);
                        if !vt.is_null() && crate::gum::is_readable(vt.add(0x108) as *const c_void, 8) {
                            let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
                            if crate::rtti::sane(slot) {
                                let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
                                let resolved = getclass(singleton2, decl_cname);
                                crate::log(&format!(
                                    "[classval-probe] base declarada resolvida={resolved:p} vs nosso parent(IScriptable)={iscriptable:p} (bate={})",
                                    resolved as u64 == iscriptable as u64
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    let orig = ORIG_CLASS_VALIDATE.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8, *mut u8, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3, x4, x5, x6, x7);
    // Achado 2026-07-13 (RE offline, find_bl_callers.py subindo de 0x1021fc61c até o loop de
    // validação 0x1021fbf90 e daí até singleton->vtbl[8]=0x10223a748, chamado pelo MESMO
    // 0x103d9622c já mapeado nas Tentativas 7/8): a validação de classe roda AninhADA dentro do
    // orquestrador de verdade — mas NUNCA logamos o RETORNO da validação original do Codeware em
    // si. Se ele voltar 0 (falha) MESMO com nosso forge (Version/Require=true), a causa não é
    // "classe não existe" — é alguma outra checagem interna do validador que register_method
    // não satisfaz. Log SÓ pra Codeware (nome_hash já resolvido acima). Diagnóstico — dev_mode.
    if diag && name_hash == Some(cw_hash) {
        crate::log(&format!("[classval-probe] validador ORIGINAL retornou {ret} pro Codeware (0=falha, !=0=sucesso)"));
    }
    // Tentativa 12 (2026-07-13) — decodifiquei o chained-fixup de vtbl+0x08 do singleton
    // 0x10223a2ac (offline, sem boot): resolve pra 0x10223a748, a MESMA função já mapeada como
    // o DISPATCHER do loop de validação inteiro (chama 0x1021fbf90 -> este validador, ×12). Ou
    // seja: o retorno agregado de TODAS as 12 validações (não só Codeware) é o que decide
    // [engine+0x54] e, por cascata, o assert. Log leve pra TODAS (não só Codeware) — diagnóstico,
    // dev_mode (o forge de Codeware já é incondicional acima; isso é só instrumentação).
    if diag && name_hash != Some(cw_hash) {
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::log(&format!("[classval-probe] validador retornou {ret} pra '{name}' (hash={:#018x})", name_hash.unwrap_or(0)));
    }
    ret
}

// ===== F4 (Facade): sonda no orquestrador real do assert baseEngineInit.cpp:1094 =====
// Achado 2026-07-13 via RE OFFLINE (sem boot — find_bl_callers.py + disasm_region.py, ver
// [[cp77-codeware-port]]): o assert "Failed to initialize scripts data!" (0x103da2a34/
// 0x103da2a60, brk DELIBERADO) só dispara se `0x103d99e44` (o orquestrador real, assinatura
// (engine_this, ctx, extra) -> bool) devolver false. Dentro dele, `add x0,x20,#0x150 /
// bl 0x10002b9d4 / tbz w0,#0,...` checa um container em [engine+0x150] (header packed
// size:30|flags:2 em +0x14, padrão clássico de red::String/DynArray com SSO). HIPÓTESE (não
// confirmada ainda): é uma lista/string de ERROS coletados durante o bind — as mensagens
// "Missing native class/function..." do validador (class_validate_probe_hook) provavelmente
// só apendam ali, sem crashar na hora.
//
// Esta sonda hooka o ORQUESTRADOR (0x103d99e44) — NÃO a função minúscula 0x10002b9d4 (4
// instruções, pequena demais pra redirect seguro — mesma lição do "hook do getter de boot-flow
// transbordou" documentada em selfboot.rs). Dump PURAMENTE OBSERVACIONAL de 0x30 bytes crus em
// [x0+0x150] (nunca desreferenciado sem `gum::is_readable` primeiro) + repassa x0/x1/x2
// inalterados pro trampolim — mesmo padrão seguro do class_validate_probe_hook. Gated
// ~/.bwms-initscripts-probe (OFF por padrão, separado dos outros 2 hooks).
const INITSCRIPTS_ORCH_VM: u64 = 0x1_03d9_9e44;
/// `stp x22,x21,[sp,#-0x30]!` (a9bd57f6) + `stp x20,x19,[sp,#0x10]` (a9014ff4), LE como u64.
const INITSCRIPTS_ORCH_PROLOGUE: u64 = 0xa901_4ff4_a9bd_57f6;
static ORIG_INITSCRIPTS_ORCH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static INITSCRIPTS_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn initscripts_orch_probe_hook(x0: *mut u8, x1: *mut u8, x2: *mut u8) -> usize {
    let c = INITSCRIPTS_CALLS.fetch_add(1, Ordering::Relaxed);
    if c < 5 && !x0.is_null() {
        let container = x0.add(0x150);
        if crate::gum::is_readable(container as *const c_void, 0x30) {
            let mut bytes = [0u8; 0x30];
            core::ptr::copy_nonoverlapping(container, bytes.as_mut_ptr(), 0x30);
            let header = u32::from_le_bytes([bytes[0x14], bytes[0x15], bytes[0x16], bytes[0x17]]);
            let size = header & 0x3FFF_FFFF;
            let flag = header >> 30;
            crate::log(&format!(
                "[initscripts-probe] CHAMADO #{c} engine={x0:p} [engine+0x150]: header@+0x14={header:#010x} (size={size} flag={flag}) bytes={bytes:02x?}"
            ));
        } else {
            crate::log(&format!("[initscripts-probe] CHAMADO #{c} engine={x0:p} [engine+0x150] ilegível"));
        }
    }
    // Tentativa 8 (2026-07-13): a Tentativa 7 provou (via retorno logado, bit0=0) que ESTA
    // chamada — a ÚNICA ao orquestrador, disparada UMA VEZ, ANTES da validação de classe
    // começar — já decide sozinha se o assert dispara (tbz w0,#0,<chama assert> no caller).
    // Ou seja: forjar o Codeware DENTRO do class_validate_probe_hook é tarde DEMAIS — aquele
    // hook só roda DEPOIS que esta decisão (bit0=0, condenado) já foi tomada. Corrigido: forja
    // + registra o Codeware AQUI, ANTES de chamar o orquestrador original, pra ele já achar a
    // classe pronta no RTTI quando fizer sua checagem interna.
    // Tentativa 10 cont. (2026-07-13): getorreg-probe já prova que a CLASSE em si dá pra forjar
    // MUITO mais cedo (2ª chamada de GetOrRegisterType, antes de gameGodModeSystem existir) —
    // mas os MÉTODOS (Version/Require) falham lá (donor AddGodMode ainda não existe). Por isso a
    // classe e os métodos agora são gates SEPARADOS: se a classe já foi forjada em outro hook
    // (CODEWARE_REGISTERED), só tenta os MÉTODOS aqui (retry seguro — só marca sucesso se o
    // texto de retorno confirmar "Version=true", senão deixa outro hook tentar de novo depois).
    let cw_hash = *CODEWARE_CNAME.get_or_init(|| crate::cname::cname("Codeware"));
    if !CODEWARE_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[initscripts-probe] forjando 'Codeware' ANTES do orquestrador (tentativa 8/10)");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let forged = crate::register::register_type_min(&reg, "Codeware");
            crate::log(&format!("[initscripts-probe] register_type_min('Codeware') -> {forged:p}"));
        } else {
            crate::log("[initscripts-probe] Registry::obtain() falhou — RTTI não pronto aqui?");
        }
    }
    if !CODEWARE_METHODS_REGISTERED.load(Ordering::Acquire) {
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let r = crate::register::register_codeware_facade(&reg);
            crate::log(&format!("[initscripts-probe] register_codeware_facade -> {r} (cw_hash={cw_hash:#018x})"));
            if r.contains("Version=true") {
                CODEWARE_METHODS_REGISTERED.store(true, Ordering::Release);
            }
        }
    }
    let orig = ORIG_INITSCRIPTS_ORCH.load(Ordering::Relaxed);
    if orig.is_null() {
        return 1; // fail-open: nunca gatear o assert por causa da NOSSA sonda
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8, *mut u8) -> usize = std::mem::transmute(orig);
    let ret = f(x0, x1, x2);
    // Achado 2026-07-13 (RE offline de 0x103d9edd0/0x103d9eaec — os 2 callers reais do
    // assert-helper): o CALLER do orquestrador faz `tbz w0,#0,<chama assert>` — ou seja, o
    // assert dispara quando bit0 do NOSSO retorno é 0, não 1 (inverso do que a doc antiga
    // supunha). Logar o retorno aqui prova diretamente qual dos dois casos aconteceu.
    if c < 3 {
        crate::log(&format!("[initscripts-probe] CHAMADO #{c} retorno={ret:#x} bit0={}", ret & 1));
    }
    ret
}

pub(crate) unsafe fn install_initscripts_orch_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-initscripts-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(INITSCRIPTS_ORCH_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[initscripts-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != INITSCRIPTS_ORCH_PROLOGUE {
        crate::log(&format!("[initscripts-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, initscripts_orch_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_INITSCRIPTS_ORCH.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[initscripts-probe] sonda instalada (orquestrador @ {target:p})"));
        }
        None => crate::log("[initscripts-probe] FALHA ao hookar o orquestrador"),
    }
}

// Tentativa 9 (2026-07-13, mesma sessão pós-causa-raiz): já provado que registrar o Codeware
// ANTES ou DURANTE o orquestrador do assert não muda nada — a decisão real é tomada numa fase
// de import-resolution AINDA MAIS CEDO. Candidato novo, achado por disasm de `CRTTISystem::Get`
// (`0x1021_88e8c`, o getter que `Registry::obtain()` já usa): é um `call_once` clássico — flag
// `[global+0x270]` (`ldaprb`/`tbz`) guarda um construtor PRIMÁRIO chamado EXATAMENTE UMA VEZ EM
// TODO O PROCESSO: `0x102188634`. Isso é, por construção, o instante em que o RTTI PASSA A
// EXISTIR — mais cedo que qualquer coisa que já tentamos (o ctor do dylib em si é RTTI-inseguro,
// achado antigo da saga F-B; o orquestrador do assert já roda bem depois). Hook aqui: deixa o
// construtor original rodar (não mexe nele por dentro — arriscado, pode não estar 100% pronto
// no meio da própria construção), e SÓ DEPOIS que ele retorna (RTTI já existe, ainda que a
// "segunda fase" lazy do Get — flag `+0x268`, função `0x102188f20` — só rode na 2ª chamada)
// forja o Codeware. Gated `~/.bwms-rttictor-probe`, testado ISOLADO da Facade primeiro
// (observacional puro) antes de tentar o forge de verdade.
const RTTI_CTOR_VM: u64 = 0x1_0218_8634;
/// `sub sp,sp,#0x50` (d10143ff) + `stp d9,d8,[sp,#0x20]` (6d0223e9), LE como u64.
const RTTI_CTOR_PROLOGUE: u64 = 0x6d02_23e9_d101_43ff;
static ORIG_RTTI_CTOR: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C" fn rtti_ctor_hook(x0: *mut u8) -> usize {
    let orig = ORIG_RTTI_CTOR.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0; // fail-open: nunca bloquear a construção real do RTTI
    }
    let f: unsafe extern "C" fn(*mut u8) -> usize = std::mem::transmute(orig);
    let ret = f(x0);
    crate::log("[rttictor-probe] construtor primário do RTTI RETORNOU — RTTI existe agora");
    let cw_hash = *CODEWARE_CNAME.get_or_init(|| crate::cname::cname("Codeware"));
    let _ = cw_hash; // só força a inicialização do OnceLock aqui; uso real abaixo
    if !CODEWARE_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[rttictor-probe] forjando+registrando 'Codeware' logo após o RTTI existir (tentativa 9)");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let forged = crate::register::register_type_min(&reg, "Codeware");
            crate::log(&format!("[rttictor-probe] register_type_min('Codeware') -> {forged:p}"));
            let r = crate::register::register_codeware_facade(&reg);
            crate::log(&format!("[rttictor-probe] register_codeware_facade -> {r}"));
        } else {
            crate::log("[rttictor-probe] Registry::obtain() falhou — RTTI ainda não pronto o bastante aqui?");
        }
    }
    ret
}

pub(crate) unsafe fn install_rtti_ctor_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-rttictor-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(RTTI_CTOR_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[rttictor-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != RTTI_CTOR_PROLOGUE {
        crate::log(&format!("[rttictor-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, rtti_ctor_hook as *mut c_void) {
        Some(orig) => {
            ORIG_RTTI_CTOR.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[rttictor-probe] sonda instalada (construtor RTTI @ {target:p})"));
        }
        None => crate::log("[rttictor-probe] FALHA ao hookar o construtor do RTTI"),
    }
}

// Tentativa 10 (2026-07-13, mesma sessão) — versão SEM risco de reentrância da Tentativa 9.
// RE offline (find_bl_callers.py contra CRTTISystem::Get, 0x1021_88e8c): só 3 callers diretos
// existem, todos dentro de 2 helpers minúsculos e genéricos bem ao lado do próprio Get():
// `0x1021885a4` = "GetOrRegisterType(name, descriptor)" — chama Get(), resolve por nome
// (vtbl+0x38); se null, chama Get() DE NOVO e registra via vtbl+0x80 (o MESMO slot que o nosso
// register_type_min usa). Isso é quase certamente o que TODAS as classes nativas do PRÓPRIO
// MOTOR (IScriptable, Entity, etc.) chamam pra se auto-registrar no RTTI durante o static-init
// do binário — centenas de chamadas, uma por classe nativa do C++ do jogo.
// A CHAVE da segurança aqui: a construção do RTTI (0x102188634) + registro dos built-ins
// (0x102188f20) rodam TODOS dentro da PRIMEIRA chamada a Get() (comprovado por disasm), ou
// seja, dentro da PRIMEIRA chamada a este helper (seja qual for a 1ª classe nativa que o motor
// registra). A PARTIR da 2ª chamada em diante, Get() já retornou de vez da sua própria
// inicialização — chamar Registry::obtain() de dentro do hook NESSE PONTO não é mais aninhado
// dentro da construção, é uma invocação nova e independente. Só pulamos a chamada #0 (a única
// potencialmente perigosa) e forjamos a partir da #1. Gated `~/.bwms-getorreg-probe`.
const GETORREG_TYPE_VM: u64 = 0x1_0218_85a4;
/// `stp x20,x19,[sp,#-0x20]!` (a9be4ff4) + `stp x29,x30,[sp,#0x10]` (a9017bfd), LE como u64.
const GETORREG_TYPE_PROLOGUE: u64 = 0xa901_7bfd_a9be_4ff4;
static ORIG_GETORREG_TYPE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static GETORREG_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn getorreg_type_hook(x0: *mut u8, x1: *mut u8) -> usize {
    let c = GETORREG_CALLS.fetch_add(1, Ordering::Relaxed);
    if c < 5 {
        crate::log(&format!("[getorreg-probe] CHAMADO #{c} x0={x0:p} x1={x1:p}"));
    }
    // pula a #0 (aninhada dentro da própria construção do RTTI, reentrância perigosa —
    // ver Tentativa 9); a partir da #1, Get() já terminou de vez de se auto-inicializar.
    // Só a CLASSE aqui (não os métodos) — testado ao vivo: register_type_min funciona limpo
    // nesta janela, mas register_codeware_facade falha ("sem protótipo") porque o donor
    // gameGodModeSystem::AddGodMode ainda não existe tão cedo. Os métodos ficam pro
    // initscripts-probe (gate separado CODEWARE_METHODS_REGISTERED, com retry).
    if c == 1 && !CODEWARE_REGISTERED.swap(true, Ordering::AcqRel) {
        crate::log("[getorreg-probe] forjando 'Codeware' (só classe) na 2ª chamada (tentativa 10, pós-bootstrap-RTTI)");
        if let Some(reg) = crate::rtti::Registry::obtain() {
            let forged = crate::register::register_type_min(&reg, "Codeware");
            crate::log(&format!("[getorreg-probe] register_type_min('Codeware') -> {forged:p}"));
        } else {
            crate::log("[getorreg-probe] Registry::obtain() falhou (inesperado nesta janela)");
        }
    }
    let orig = ORIG_GETORREG_TYPE.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0; // fail-open: nunca bloquear o registro real de tipos do motor
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8) -> usize = std::mem::transmute(orig);
    f(x0, x1)
}

pub(crate) unsafe fn install_getorreg_type_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-getorreg-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(GETORREG_TYPE_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[getorreg-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != GETORREG_TYPE_PROLOGUE {
        crate::log(&format!("[getorreg-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, getorreg_type_hook as *mut c_void) {
        Some(orig) => {
            ORIG_GETORREG_TYPE.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[getorreg-probe] sonda instalada (GetOrRegisterType @ {target:p})"));
        }
        None => crate::log("[getorreg-probe] FALHA ao hookar GetOrRegisterType"),
    }
}

// RE offline 2026-07-13 (disasm_region.py em 0x103d99eac, o caminho "container vazio" do
// orquestrador): o byte final que decide o retorno (logo o bit0 checado pelo caller) vem de
// `strb w0,[engine+0x54]`, onde w0 = 0x103d9622c(container=engine+0x150, flag, engine+0x90,
// count=10). O "10" fixo + o fato de termos observado 12 classes validadas (CHAMADO #0..#11)
// é candidato forte a mismatch de contagem. Sonda PURAMENTE OBSERVACIONAL: loga os 4 args +
// o retorno, sempre repassa pro trampolim. Gated ~/.bwms-countcheck-probe.
const COUNT_CHECK_VM: u64 = 0x103d9_622c;
/// `sub sp,sp,#0x80` (d10203ff) + `stp x24,x23,[sp,#0x40]` (a9045ff8), LE como u64.
const COUNT_CHECK_PROLOGUE: u64 = 0xa904_5ff8_d102_03ff;
static ORIG_COUNT_CHECK: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static COUNT_CHECK_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn count_check_probe_hook(x0: *mut u8, x1: usize, x2: *mut u8, x3: usize) -> usize {
    let c = COUNT_CHECK_CALLS.fetch_add(1, Ordering::Relaxed);
    crate::log(&format!(
        "[countcheck-probe] CHAMADO #{c} container(engine+0x150)={x0:p} flag={x1:#x} extra_ctx(engine+0x90)={x2:p} count_arg={x3:#x}"
    ));
    let orig = ORIG_COUNT_CHECK.load(Ordering::Relaxed);
    if orig.is_null() {
        return 1; // fail-open: nunca gatear o assert por causa da NOSSA sonda
    }
    let f: unsafe extern "C" fn(*mut u8, usize, *mut u8, usize) -> usize = std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3);
    crate::log(&format!("[countcheck-probe] CHAMADO #{c} retorno={ret:#x} (vira [engine+0x54])"));
    ret
}

// Tentativa 12 (2026-07-13) — o dispatcher `0x1021fbf90` (loop que chama o validador de classe)
// na verdade despacha por "kind" (0/1/3/4/5) pra handlers DIFERENTES: kind==1 (classe) é o
// `0x1021fc61c` já mapeado — e com os 2 fixes de hoje (parent-pointer + fullName bare), as 2843
// classes do bundle TODAS retornam sucesso agora (confirmado ao vivo, zero falhas). Mas o boot
// AINDA crasha — ou seja, o `w23` (contador de falhas acumulado no dispatcher) não é zero, e a
// falha deve estar em kind==0 (`0x1021fc1a4`, NUNCA examinado), 3 (`0x1021fc290`), 4
// (`0x1021fc47c`) ou 5 (`0x1021fcee0`, o bind de FUNÇÃO GLOBAL já conhecido da saga F-B — mas
// esse mecanismo já funciona pras nossas natives desde 2026-06-25). Sonda PURAMENTE
// OBSERVACIONAL no handler de kind==0 (candidato mais provável por ser o único nunca
// examinado): loga `[x1+8]` (mesmo padrão CName de sempre) + retorno, sempre repassa. Gated
// `~/.bwms-kind0-probe`.
const KIND0_VM: u64 = 0x1_021f_c1a4;
/// `sub sp,sp,#0x60` (d10183ff) + `stp x20,x19,[sp,#0x40]` (a9044ff4), LE como u64.
const KIND0_PROLOGUE: u64 = 0xa904_4ff4_d101_83ff;
static ORIG_KIND0: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static KIND0_CALLS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn kind0_probe_hook(
    x0: *mut u8,
    x1: *mut u8,
    x2: *mut u8,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    let c = KIND0_CALLS.fetch_add(1, Ordering::Relaxed);
    let name_hash = if !x1.is_null() && crate::gum::is_readable(x1.add(8) as *const c_void, 8) {
        Some(core::ptr::read_unaligned(x1.add(8) as *const u64))
    } else {
        None
    };
    let orig = ORIG_KIND0.load(Ordering::Relaxed);
    if orig.is_null() {
        return 1;
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8, *mut u8, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    let ret = f(x0, x1, x2, x3, x4, x5, x6, x7);
    if c < 5 || ret == 0 {
        let name = crate::cname::resolve_cname(name_hash.unwrap_or(0));
        crate::log(&format!(
            "[kind0-probe] CHAMADO #{c} ret={ret} pra '{name}' (hash={:#018x}) x1={x1:p}",
            name_hash.unwrap_or(0)
        ));
    }
    ret
}

pub(crate) unsafe fn install_kind0_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-kind0-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(KIND0_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[kind0-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != KIND0_PROLOGUE {
        crate::log(&format!("[kind0-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, kind0_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_KIND0.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[kind0-probe] sonda instalada (kind==0 handler @ {target:p})"));
        }
        None => crate::log("[kind0-probe] FALHA ao hookar o handler de kind==0"),
    }
}

pub(crate) unsafe fn install_count_check_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-countcheck-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(COUNT_CHECK_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[countcheck-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != COUNT_CHECK_PROLOGUE {
        crate::log(&format!("[countcheck-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, count_check_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_COUNT_CHECK.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[countcheck-probe] sonda instalada (check @ {target:p})"));
        }
        None => crate::log("[countcheck-probe] FALHA ao hookar o check"),
    }
}

// ===== `bindsig-probe` (2026-07-17) — sonda OBSERVE-ONLY no validador "BindFunctionSignature"
// ==== achado via RE hoje: `0x1021ea1b8` (rebase, link-base 0x100000000). Contexto: um método
// cujo param/retorno é `ref<X>` de uma classe FORJADA POR NÓS (não pré-existente do motor/
// bundle) crasha o boot alguns segundos depois de "registrar com sucesso" — ver a saga
// `ScriptableServiceContainer.GetService` em register.rs (REFUTADO/TENTADO 2026-07-17: priming
// via `GetType("handle:X")` não populou nada). Hipótese do research agent anterior: esta função
// walka os descritores de tipo (param/retorno/local) do "function descriptor" e checa um CACHE
// SLOT em `[type_ref+0x18]`; se null, loga "Unresolved parameter/return/local type '%hs' for
// function '%hs'" (strings confirmadas por xref no disasm) — que agrega num crash mais tarde.
//
// RE CONFIRMADA por leitura direta de `/tmp/full_disasm_check.txt` (linha 8898668+) hoje:
//   - **CORREÇÃO ao relato anterior:** o function-descriptor vem em **x1** (2º arg), não x0 —
//     confirmado pelo `mov x19,x1` no prólogo + TODOS os acessos de campo subsequentes usarem
//     x19. x0 (salvo em x22) é outro contexto (Registry/RTTI system, só usado no braço
//     alternativo de "criar o compiled-func se ausente", `[x19+0x18]`==null).
//   - func_desc(x19) +0x08 = fullName (CName) — MESMO campo que `build_native_func` (register.rs)
//     escreve nas nossas próprias funções forjadas; dá pra resolver via `cname::resolve_cname`.
//   - func_desc +0x30 = ponteiro pro ARRAY DE PONTEIROS de param-descriptors; +0x3c = count(u32).
//   - func_desc +0x40 = array de locals (mesmo esquema); +0x4c = count(u32) (não logado aqui,
//     fora do escopo pedido — só confirma a estrutura pro registro).
//   - func_desc +0x80 = ponteiro DIRETO pro type_ref do RETORNO (0 = void, sem checagem).
//   - Pra CADA param: a entrada do array é um PONTEIRO pro param-descriptor; o type_ref mora em
//     `[param_descriptor+0x28]` (1 hop a mais que o retorno, que já É o type_ref direto).
//   - Em TODOS os 3 casos (param/local/retorno) o cache checado é `[type_ref+0x18]` — confirmado
//     pelo padrão idêntico `ldr x0,[...+0x28 ou +0x80]; ldr x1,[x0,#0x18]; cbz x1,<erro>` nos 3
//     branches (0x1021ea310-320 retorno, 0x1021ea358-368 params, 0x1021ea3d8-3e4 locals).
//
// Puramente OBSERVACIONAL: NUNCA escreve memória (só leituras via `gum::is_readable`+
// `read_unaligned`), sempre chama o trampolim inalterado (HookBefore clássico do projeto). Log
// CONCISO: só as primeiras `BINDSIG_LOG_N` chamadas OU qualquer chamada com ≥1 cache slot NULL
// — a função roda ~1x por native/global declarada no bundle inteiro (centenas de chamadas por
// boot), logar tudo afogaria o log. Gated `~/.bwms-bindsig-probe`, OFF por padrão.
const BINDSIG_VM: u64 = 0x1_021e_a1b8;
/// `sub sp,sp,#0xd0` (d10343ff) + `stp x26,x25,[sp,#0x80]` (a90867fa), LE como u64 — confirmado
/// lendo os BYTES REAIS do binário (file offset == vmaddr; `__TEXT` mapeia 1:1 desde vmaddr
/// 0x100000000/fileoff 0, `otool`/parse do Mach-O header conferido antes de escrever isto).
const BINDSIG_PROLOGUE: u64 = 0xa908_67fa_d103_43ff;
static ORIG_BINDSIG: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static BINDSIG_CALLS: AtomicUsize = AtomicUsize::new(0);
/// Quantas primeiras chamadas logar incondicionalmente (depois só loga se achar cache NULL).
const BINDSIG_LOG_N: usize = 20;
/// Cap de segurança pro loop de params — nunca confia cegamente num count vindo de memória do
/// jogo (mesmo espírito da guarda de profundidade em `rtti::resolve_in_class`).
const BINDSIG_MAX_FIELDS: u32 = 32;

/// Lê `[type_ref+0x18]` (o cache slot) SE `type_ref` for legível. `None` = type_ref nulo/
/// ilegível (não dá pra saber o estado do cache); `Some(ptr)` = valor do cache (`0 as *mut _` =
/// NÃO-resolvido — o caso que o validador original rejeita).
unsafe fn bindsig_cache_slot(type_ref: *mut u8) -> Option<*mut c_void> {
    if type_ref.is_null() || !crate::gum::is_readable(type_ref.add(0x18) as *const c_void, 8) {
        return None;
    }
    Some(core::ptr::read_unaligned(type_ref.add(0x18) as *const *mut c_void))
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn bindsig_probe_hook(
    x0: usize,
    x1: *mut u8,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    let c = BINDSIG_CALLS.fetch_add(1, Ordering::Relaxed);
    let func_desc = x1; // CONFIRMADO por disasm: função-descriptor = x1 (arg2), não x0.
    let mut unresolved = false;
    let mut parts: Vec<String> = Vec::new();

    if !func_desc.is_null() && crate::gum::is_readable(func_desc as *const c_void, 0x88) {
        // Nome da função (fullName CName @ +0x08) — mesmo campo que `build_native_func` escreve
        // nas NOSSAS funções forjadas. Resolve pela mesma tabela `cname::` usada no resto do
        // projeto (pode devolver um hash cru se nunca visto por esta sessão).
        let name_hash = core::ptr::read_unaligned(func_desc.add(0x08) as *const u64);
        let name = crate::cname::resolve_cname(name_hash);

        // Retorno: ponteiro pro type_ref mora DIRETO em +0x80 (0 = void, sem type_ref a checar).
        let ret_type_ref = core::ptr::read_unaligned(func_desc.add(0x80) as *const *mut u8);
        let ret_state = if ret_type_ref.is_null() {
            "void".to_string()
        } else {
            match bindsig_cache_slot(ret_type_ref) {
                Some(v) if v.is_null() => {
                    unresolved = true;
                    format!("type_ref={ret_type_ref:p} cache=NULL <<<")
                }
                Some(v) => format!("type_ref={ret_type_ref:p} cache={v:p}"),
                None => format!("type_ref={ret_type_ref:p} cache=<ilegível>"),
            }
        };
        parts.push(format!("ret[{ret_state}]"));

        // Params: array de PONTEIROS @ +0x30, count(u32) @ +0x3c. Cada entrada -> o type_ref
        // mora em [entrada+0x28] (1 hop a mais que o retorno, que já É o type_ref direto).
        let param_count = core::ptr::read_unaligned(func_desc.add(0x3c) as *const u32);
        let param_arr = core::ptr::read_unaligned(func_desc.add(0x30) as *const *mut u8);
        if param_count > 0 && param_count <= BINDSIG_MAX_FIELDS && !param_arr.is_null() {
            for i in 0..param_count {
                let slot_addr = param_arr.add(i as usize * 8) as *const c_void;
                if !crate::gum::is_readable(slot_addr, 8) {
                    parts.push(format!("param{i}[ilegível]"));
                    continue;
                }
                let param_desc = core::ptr::read_unaligned(slot_addr as *const *mut u8);
                if param_desc.is_null() || !crate::gum::is_readable(param_desc.add(0x28) as *const c_void, 8) {
                    parts.push(format!("param{i}[desc-ilegível]"));
                    continue;
                }
                let type_ref = core::ptr::read_unaligned(param_desc.add(0x28) as *const *mut u8);
                match bindsig_cache_slot(type_ref) {
                    Some(v) if v.is_null() => {
                        unresolved = true;
                        parts.push(format!("param{i}[type_ref={type_ref:p} cache=NULL <<<]"));
                    }
                    Some(v) => parts.push(format!("param{i}[type_ref={type_ref:p} cache={v:p}]")),
                    None => parts.push(format!("param{i}[type_ref={type_ref:p} cache=<ilegível>]")),
                }
            }
        }

        // FORÇA log (ignora o gate de concisão) pra qualquer função cujo nome bata com o caso
        // FALHANTE que estamos investigando hoje (2026-07-17: GetService/ScriptableService*) —
        // sem isto, se o cache JÁ estiver populado (não-null) na hora que LEMOS, a chamada #N (N
        // grande) nunca apareceria no log (só logamos as 1as N OU as com cache null). Aqui
        // queremos ver ESTA função específica INDEPENDENTE do que o cache mostrar.
        let is_target = name.contains("GetService") || name.contains("Scriptable");
        // Marco periódico (a cada 200 chamadas, SEM detalhe) — dá pra saber quão longe o walk
        // chegou antes de um crash, mesmo se nada mais disparar o log.
        let milestone = c > 0 && c % 200 == 0;
        if c < BINDSIG_LOG_N || unresolved || is_target || milestone {
            let tag = if is_target { " <<<<< ALVO DA INVESTIGAÇÃO (GetService/Scriptable*)" } else { "" };
            crate::log(&format!(
                "[bindsig-probe] #{c} func='{name}' (hash={name_hash:#018x}) func_desc={func_desc:p} {}{tag}",
                parts.join(" ")
            ));
        }
    } else if c < BINDSIG_LOG_N {
        crate::log(&format!("[bindsig-probe] #{c} func_desc={func_desc:p} ilegível"));
    }

    let orig = ORIG_BINDSIG.load(Ordering::Relaxed);
    if orig.is_null() {
        return 1; // fail-open: nunca gatear o validador por causa da NOSSA sonda
    }
    let f: unsafe extern "C" fn(usize, *mut u8, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    f(x0, x1, x2, x3, x4, x5, x6, x7)
}

/// Instala a sonda `bindsig-probe`, gated `~/.bwms-bindsig-probe` (zero efeito se ausente —
/// checagem ANTES de tocar em qualquer memória do jogo, mesmo padrão de `install_kind0_probe`/
/// `install_count_check_probe` acima).
pub(crate) unsafe fn install_bindsig_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-bindsig-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(BINDSIG_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[bindsig-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != BINDSIG_PROLOGUE {
        crate::log(&format!("[bindsig-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, bindsig_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_BINDSIG.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[bindsig-probe] sonda instalada (BindFunctionSignature @ {target:p})"));
        }
        None => crate::log("[bindsig-probe] FALHA ao hookar BindFunctionSignature"),
    }
}

// ===== `dynarraygrowth-probe` (2026-07-18) — sonda OBSERVE-ONLY na rotina de CRESCIMENTO de
// container (hashmap/dynarray com índice de bucket) achada como o SITE REAL do crash de
// `GetService` na sessão `bindsig-probe` (2026-07-17): o crash-report (`.ips`) apontava um
// SIGSEGV `KERN_INVALID_ADDRESS at 0x8` dentro de um "invoke thunk" genérico de closure/
// allocator tipo `red::FixedSizeFunction` (`ldr x8,[x0]; ldr x3,[x8,#0x8]; br x3` — x8=vtable de
// x0, NULO). Rastreei o caller real (`0x10096ca80` no relato de ontem era o MEIO do prólogo, não
// a entrada — corrigido hoje: a entrada verdadeira é `0x10096ca74`, achada lendo os bytes crus do
// Mach-O diretamente, já que `/tmp/full_disasm_check.txt` não sobreviveu à troca de sessão/dia e
// NÃO foi regenerado — 746MB, caro demais; usei Capstone via Python pra desassemblar só a região
// necessária). Essa função cresce um container com ÍNDICE DE HASH (bucket array + entries + campo
// de stride @+0x1c, sentinela -1 nos buckets vazios — formato de hashmap, não DynArray simples) e,
// no meio do crescimento, invoca um ALOCADOR EMBUTIDO no próprio container em `container+0x28`
// através do thunk genérico (`add x21,x19,#0x28; mov x0,x21; mov w2,#8; bl <thunk>`), que lê
// `[x0]` (o PRIMEIRO campo do sub-objeto embutido = seu vtable pointer) e chama o slot 1. Se esse
// vtable pointer for NULO (sub-objeto nunca inicializado), o thunk crasha exatamente como no
// crash-report. Achei 43 chamadores diretos (`bl`) escaneando o `__TEXT` inteiro por bytes (ver
// scratchpad desta sessão) — vários na vizinhança de `CLASS_VALIDATE_VM`/`BINDSIG_VM`, incl.
// `0x10219e1d8` (a poucos bytes do frame `0x10219e1dc` do crash-report de ontem — MESMO caller).
//
// Esta sonda: loga `x0` (ponteiro do container) e o valor do QWORD em `[x0+0x28]` (o vtable
// pointer do alocador embutido — NULO = a condição exata que crasha) pra CADA chamada, sempre
// repassando ao trampolim inalterado (nunca escreve memória). Gated `~/.bwms-dynarraygrowth-probe`.
const DYNARRAYGROWTH_VM: u64 = 0x1_0096_ca74;
/// `sub sp,sp,#0x80` (d10203ff) + `stp x26,x25,[sp,#0x30]` (a90367fa), LE como u64 — lido direto
/// dos bytes do Mach-O (file offset == vmaddr; `__TEXT` mapeia 1:1 desde vmaddr 0x100000000/
/// fileoff 0, mesmo esquema já usado nos outros PROLOGUE desta sessão).
const DYNARRAYGROWTH_PROLOGUE: u64 = 0xa903_67fa_d102_03ff;
static ORIG_DYNARRAYGROWTH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static DYNARRAYGROWTH_CALLS: AtomicUsize = AtomicUsize::new(0);
/// Primeiras chamadas a logar incondicionalmente (estabelece o que "normal" parece antes de
/// qualquer null aparecer). Calls com o vtable NULO são SEMPRE logadas, sem limite — é a condição
/// exata que estamos caçando, rara o bastante pra não afogar o log.
const DYNARRAYGROWTH_LOG_N: usize = 30;

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn dynarraygrowth_probe_hook(
    x0: *mut u8,
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    let c = DYNARRAYGROWTH_CALLS.fetch_add(1, Ordering::Relaxed);
    let alloc_addr = if !x0.is_null() { x0.add(0x28) } else { std::ptr::null_mut() };
    let (alloc_vtable, readable) = if !alloc_addr.is_null() && crate::gum::is_readable(alloc_addr as *const c_void, 8) {
        (core::ptr::read_unaligned(alloc_addr as *const u64), true)
    } else {
        (0, false)
    };
    let is_null_vtable = readable && alloc_vtable == 0;
    if c < DYNARRAYGROWTH_LOG_N || is_null_vtable {
        let state = if !readable {
            "ilegível".to_string()
        } else if is_null_vtable {
            "NULL <<<<< CONDIÇÃO DE CRASH (mesma do crash-report 2026-07-17)".to_string()
        } else {
            format!("{alloc_vtable:#018x}")
        };
        crate::log(&format!(
            "[dynarraygrowth-probe] #{c} container(x0)={x0:p} new_count/cap(x1)={x1:#x} [container+0x28]={state}"
        ));
    }
    let orig = ORIG_DYNARRAYGROWTH.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0; // fail-open: nunca gatear o crescimento por causa da NOSSA sonda
    }
    let f: unsafe extern "C" fn(*mut u8, usize, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    f(x0, x1, x2, x3, x4, x5, x6, x7)
}

/// Instala a sonda `dynarraygrowth-probe`, gated `~/.bwms-dynarraygrowth-probe` (zero efeito se
/// ausente — mesmo padrão de `install_bindsig_probe` acima).
pub(crate) unsafe fn install_dynarraygrowth_probe() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-dynarraygrowth-probe").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(DYNARRAYGROWTH_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[dynarraygrowth-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != DYNARRAYGROWTH_PROLOGUE {
        crate::log(&format!("[dynarraygrowth-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, dynarraygrowth_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_DYNARRAYGROWTH.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[dynarraygrowth-probe] sonda instalada (growth routine @ {target:p})"));
        }
        None => crate::log("[dynarraygrowth-probe] FALHA ao hookar a growth routine"),
    }
}

/// PRODUÇÃO 2026-07-13: este hook agora é INCONDICIONAL (era gated `~/.bwms-classvalidate-probe`
/// até um teste com zero markers de dev revelar que, sem ele, o boot CRASHA sempre que
/// `codeware-facade.reds` está no bundle — ou seja, pra QUALQUER usuário final instalando o
/// BWMS hoje. O forge da classe `Codeware` (dentro do hook) é o que realmente conserta o boot;
/// o marker antigo só devia ter gateado o LOG verboso (agora atrás de `dev_mode()`, ver a fn).
pub(crate) unsafe fn install_class_validate_probe() {
    let target = crate::rebase(CLASS_VALIDATE_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[classval-probe] alvo ilegível -> sem hook");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != CLASS_VALIDATE_PROLOGUE {
        crate::log(&format!("[classval-probe] prólogo não casou ({got:#018x}) -> sem hook"));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, class_validate_probe_hook as *mut c_void) {
        Some(orig) => {
            ORIG_CLASS_VALIDATE.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!("[classval-probe] sonda instalada (validador de classe @ {target:p})"));
        }
        None => crate::log("[classval-probe] FALHA ao hookar o validador de classe"),
    }
}

// ===== SKIP DAS TELAS "APERTE ESPAÇO" (dispatcher de boot-state) =====
// O boot logos→título→loading→menu é dirigido por uma byte em `GameSessionDesc+0x84`:
// o dispatcher @0x103f70740 a lê e faz SwitchState pra fase 1=título glitch,
// 2=initialize-user/loading, 3=MAIN MENU. As telas "APERTE ESPAÇO" esperam input nativo
// que avança a byte. CRACK verificado por disasm — ver notes/boot-flow-phase-byte.md.
//
// 1ª tentativa (hookar o GETTER @0x103f5ec74, leaf de 8 bytes) CRASHOU: o redirect do gum
// (alvo >128MB → 16 bytes) TRANSBORDOU na função vizinha (SIGILL @0x3f5ec7c). Função
// minúscula não dá pra hookar inline.
//
// FIX: hookar o DISPATCHER @0x103f70740 (função grande, sem transbordo). Na ENTRADA dele,
// x0 = o GameSessionDesc (confirmado: `mov x20,x0` no prólogo, depois `mov x0,x20; bl getter`).
// Escrevo 3 na phase byte se for 1/2 → o próprio dispatcher lê 3 e faz SwitchState(PreGameMenu),
// o caminho OFICIAL do jogo. Replacement com 8 args (x0-x7) repassados → robusto a qualquer
// assinatura de ≤8 args inteiros/ponteiro.

/// Trampolim do dispatcher original, devolvido pelo replace.
static ORIG_PHASE_DISP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Contador de chamadas do dispatcher (só p/ o diagnóstico das primeiras N).
static DISP_N: AtomicUsize = AtomicUsize::new(0);
/// Liga/desliga o skip de tela em runtime (futuro toggle na aba mod). Setado no install.
static SKIP_PHASE: AtomicBool = AtomicBool::new(false);
/// GameSessionDesc capturado no x0 do dispatcher (na fase de setup do boot) — usado por
/// `force_pregame_menu` p/ escrever a phase byte + re-chamar o dispatcher (replica o "proceed").
pub(crate) static GAME_SESSION_DESC: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
/// vmaddr do dispatcher de boot-state (base de link 0x100000000). Conferido por disasm.
const PHASE_DISP_VM: u64 = 0x1_03f7_0740;
/// Bytes do prólogo: `stp x20,x19,[sp,#0x20]` (a9024ff4) + `stp x29,x30,[sp,#0x30]` (a9037bfd).
const PHASE_DISP_PROLOGUE: u64 = 0xa903_7bfd_a902_4ff4;

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn phase_dispatcher_replacement(
    x0: *mut u8,
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    // x0 = GameSessionDesc. Fase real 1 (título) ou 2 (initialize/loading) → escrevo 3 →
    // o dispatcher abaixo lê 3 e troca pro inkPreGameMenuState (menu), zero clique.
    let readable = !x0.is_null() && crate::gum::is_readable(x0 as *const c_void, 0x85);
    let v = if readable { (x0.add(0x84) as *const i8).read() as i32 } else { -99 };
    // CAPTURA o GameSessionDesc (x0) — a força do proceed (force_pregame_menu) precisa dele.
    if readable {
        GAME_SESSION_DESC.store(x0, Ordering::Relaxed);
    }
    // DIAGNÓSTICO: loga as primeiras chamadas pra ver a sequência de fases que o dispatcher vê.
    let dn = DISP_N.fetch_add(1, Ordering::Relaxed);
    if dn < 40 {
        crate::log(&format!("[skipintro] dispatcher #{dn}: x0={x0:p} phase={v}"));
    }
    // SKIP da engagement: o dispatcher só começa a rodar DEPOIS do boot (após imgui pronto) — provado
    // in-game 2026-07-02 (dispatcher #0 vem após "imgui pronto"), com phase=1 (engagement/título)
    // CONTINUAMENTE (redispara a cada frame; a nota antiga "não redispara" estava errada). Então
    // escrever 3 quando phase é 1/2 é SEGURO (não pega a fase de streaming) → o dispatcher original
    // lê 3 e faz SwitchState(inkPreGameMenuState) = MENU, sem input nem TCC. Gate opt-in.
    if SKIP_PHASE.load(Ordering::Relaxed) && readable && (v == 1 || v == 2) {
        (x0.add(0x84) as *mut i8).write(3);
        let n = FORCE_N.fetch_add(1, Ordering::Relaxed);
        if n < 6 {
            crate::log(&format!("[skipintro] dispatcher: phase {v}->3 (engagement -> menu, sem input)"));
        }
    }
    let orig = ORIG_PHASE_DISP.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, usize, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    f(x0, x1, x2, x3, x4, x5, x6, x7)
}

/// Hooka o dispatcher de boot-state (opt-in pelo mesmo marcador do skip). Guard de bytes igual
/// ao do executor: se o alvo não for MESMO o dispatcher esperado, aborta limpo (nunca crasha).
unsafe fn install_phase_skip() {
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false);
    if !on {
        return;
    }
    let target = crate::rebase(PHASE_DISP_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[skipintro] dispatcher ilegível -> sem skip de tela (sem crash)");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != PHASE_DISP_PROLOGUE {
        crate::log(&format!(
            "[skipintro] dispatcher não casou ({got:#018x} != {PHASE_DISP_PROLOGUE:#018x}) -> sem skip de tela"
        ));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, phase_dispatcher_replacement as *mut c_void) {
        Some(orig) => {
            ORIG_PHASE_DISP.store(orig, Ordering::Relaxed);
            // SKIP_PHASE fica FALSE: escrever phase=3 muda a state machine de boot MAS a tela
            // "APERTE E" (attract screen, EngagementScreenGameController) é um sistema PARALELO que
            // continua por cima — e a state machine inconsistente dispara um assert do jogo
            // (EXC_BREAKPOINT, provado in-game 2026-07-02). Então o hook fica só CAPTURA (o
            // GameSessionDesc), sem escrever. O proceed do attract é 100% HID nativo (ver nota).
            SKIP_PHASE.store(false, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!(
                "[skipintro] hook no dispatcher de boot @ {target:p} (captura-only; skip do attract precisa de HID)"
            ));
        }
        None => crate::log("[skipintro] FALHA ao hookar o dispatcher de boot"),
    }
}

/// Quantas vezes já forçamos o menu (cap de segurança).
static FORCE_N: AtomicUsize = AtomicUsize::new(0);

/// FORÇA a transição pro PreGameMenu replicando o "proceed" da engagement SEM input: escreve
/// phase=3 no GameSessionDesc+0x84 e RE-CHAMA o dispatcher (que lê 3 e faz SwitchState(PreGameMenu),
/// o caminho oficial). Roda na GAME thread (cp77_tick). Gate: precisa do GameSessionDesc capturado
/// (o dispatcher rodou no setup) + skipintro on. Devolve true se disparou. Cap p/ não repetir demais.
pub(crate) unsafe fn force_pregame_menu() -> bool {
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-skipintro").exists())
            .unwrap_or(false);
    if !on || FORCE_N.load(Ordering::Relaxed) >= 8 {
        return false;
    }
    let desc = GAME_SESSION_DESC.load(Ordering::Relaxed);
    if desc.is_null() || !crate::gum::is_readable(desc as *const c_void, 0x85) {
        return false;
    }
    let orig = ORIG_PHASE_DISP.load(Ordering::Relaxed);
    if orig.is_null() {
        return false;
    }
    let v = (desc.add(0x84) as *const i8).read() as i32;
    if v == 3 || v >= 5 {
        return false; // já no menu (3) ou em gameplay (5) — nada a forçar
    }
    (desc.add(0x84) as *mut i8).write(3); // phase 3 = inkPreGameMenuState (menu)
    let n = FORCE_N.fetch_add(1, Ordering::Relaxed) + 1;
    crate::log(&format!("[skipintro] force_pregame_menu #{n}: phase {v}->3, chamando dispatcher (proceed nativo)"));
    let f: unsafe extern "C" fn(*mut u8, usize, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    f(desc, 0, 0, 0, 0, 0, 0, 0);
    true
}

/// Quantas vezes já avançamos a sessão (cap de segurança).
static SESSION_ADVANCE_N: AtomicUsize = AtomicUsize::new(0);

/// AVANÇA a sessão pregame ARMANDO o gatilho do UPDATER nativo (@0x103f709f4) — o efeito colateral que o
/// SPACE causa e que ARMA o save-system, SEM input. Disasm 2026-07-05: o updater lê um ESTADO INTERNO em
/// `desc+0xd4` a cada frame; no estado 4 ele faz o proceed COMPLETO (cleanup dos input-listeners via
/// 0x103f51860 ×3 + reset de timers + escreve a phase byte=2 + chama 0x103f7073c). Escrever só a phase
/// byte (+0x84) NÃO basta — pula o cleanup/estado que o updater faz. Então setamos o ESTADO INTERNO
/// (+0xd4) = alvo (o updater roda o resto sozinho no próximo frame). O alvo default é 4 (o estado que a
/// disasm mostra disparando o proceed p/ fase 2); ajustável pelo CONTEÚDO do marcador (1 dígito) SEM
/// recompilar, depois do boot de referência revelar o valor exato que o SPACE seta. Gate por marcador
/// `~/.bwms-session-advance` (OFF por padrão). Roda na game thread (native a partir de callback redscript).
pub(crate) unsafe fn force_session_advance() -> bool {
    let marker_home = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-session-advance"));
    let marker_tmp = std::path::Path::new("/tmp/bwms-session-advance");
    let on = marker_tmp.exists() || marker_home.as_ref().map(|p| p.exists()).unwrap_or(false);
    if !on || SESSION_ADVANCE_N.load(Ordering::Relaxed) >= 8 {
        return false;
    }
    let desc = GAME_SESSION_DESC.load(Ordering::Relaxed);
    if desc.is_null() || !crate::gum::is_readable(desc as *const c_void, 0xe4) {
        crate::log("[advance] GameSessionDesc não capturado ainda (getter não rodou) -> sem avanço");
        return false;
    }
    let phase = (desc.add(0x84) as *const i8).read() as i32;
    // Só arma na engagement (fase 1); em outra fase já passou o ponto (evita mexer no meio do menu/jogo).
    if phase != 1 {
        return false;
    }
    // Alvo do estado interno: lê 1 dígito do conteúdo do marcador (ajustável pós-referência), default 4.
    let target: u8 = marker_home
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .or_else(|| std::fs::read_to_string(marker_tmp).ok())
        .and_then(|s| s.trim().parse::<u8>().ok())
        .filter(|&v| v <= 9)
        .unwrap_or(4);
    let before = (desc.add(0xd4) as *const u8).read();
    (desc.add(0xd4) as *mut u8).write(target);
    let n = SESSION_ADVANCE_N.fetch_add(1, Ordering::Relaxed) + 1;
    crate::log(&format!(
        "[advance] #{n}: estado interno +0xd4 {before}->{target} (o updater nativo roda o proceed: cleanup+phase=2). phase byte atual={phase}"
    ));
    true
}

// ===== SAVE-ARM: destrava o save-system no pregame SEM input (workflow RE 2026-07-05) =====
// GATE PROVADO por disasm: o tick `ProcessRequests` @0x103f30e70 do gsm::BaseRequestsHandler lê um
// ready-byte em `[GalaxySaveService+0x503]` (getter 0x101edde20: `ldrb w0,[x0,#0x503]; ret`); enquanto
// esse byte é 0, `RequestSavesForLoad` só ENFILEIRA (impl 0x103f33c24) e o drain nunca rende saves →
// `RequestSavesCountSync`=0 → menu SEM "CONTINUAR". Quem seta o byte=1 E monta o scan de saves é
// `0x100d68c88(service, mode=2)` — a rotina que o INPUT dispara via user-init. Confirmado seguro por
// disasm: com mode==2 checa saves-no-disco (0x1020c8440(service+0x740)) e perfil (0x101ed6a28); se
// qualquer um retorna 0 faz EARLY-RETURN (no-op), senão `strb #1,[service+0x503]` + monta o scan.
// ProcessRequests(x0=handler, x1=SERVICE): `mov x0,x1; bl getter` → x1 É o GalaxySaveService.
// Chamamos `0x100d68c88(x1, 2)` de DENTRO do hook do tick = mesma thread do caminho legítimo do jogo,
// só aloca+enfileira (não segura lock do GameThread) → NÃO deadlocka como o d4=4. Opt-in ~/.bwms-savearm.
const PROC_REQUESTS_VM: u64 = 0x1_03f3_0e70;
const PROC_REQUESTS_PROLOGUE: u64 = 0xa905_57f6_d102_03ff; // sub sp,#0x80 ; stp x22,x21,[sp,#0x50]
const SAVE_ARM_FN_VM: u64 = 0x1_00d6_8c88;
static ORIG_PROC_REQUESTS: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
pub(crate) static GALAXY_SAVE_SERVICE: AtomicPtr<u8> = AtomicPtr::new(std::ptr::null_mut());
static SAVE_ARM_FIRES: AtomicUsize = AtomicUsize::new(0);
static SAVE_ARM_IN: AtomicBool = AtomicBool::new(false);
/// Engagement já foi mostrada (sinal do redscript via overlay) — usado pra só armar os saves DEPOIS
/// dela (= no menu), nunca no asset-load inicial (disparar cedo crashou: EXC_BAD_ACCESS, service não pronto).
static ENGAGEMENT_WAS_SHOWN: AtomicBool = AtomicBool::new(false);
static PROC_TICKS: AtomicUsize = AtomicUsize::new(0);
static MENU_TICKS: AtomicUsize = AtomicUsize::new(0);
static SAVE_BYTE_LAST: AtomicI64 = AtomicI64::new(-2);

fn save_arm_enabled() -> bool {
    std::path::Path::new("/tmp/bwms-savearm").exists()
        || std::env::var("HOME")
            .map(|h| std::path::Path::new(&h).join(".bwms-savearm").exists())
            .unwrap_or(false)
}

/// Hook do tick `ProcessRequests(x0=handler, x1=service, ...)`: captura o GalaxySaveService (x1),
/// loga o ready-byte, e — gated — chama a rotina real que arma os saves SEM input. Passthrough.
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn proc_requests_replacement(
    x0: *mut u8,
    x1: *mut u8,
    x2: usize,
    x3: usize,
    x4: usize,
    x5: usize,
    x6: usize,
    x7: usize,
) -> usize {
    // x1 = GalaxySaveService. Guard de leitura (cobre +0x503 e +0x740 que o setter acessa).
    let svc = x1;
    let valid = !svc.is_null() && crate::gum::is_readable(svc as *const c_void, 0x748);
    if valid {
        GALAXY_SAVE_SERVICE.store(svc, Ordering::Relaxed);
    }
    let byte: i64 = if valid { (svc.add(0x503) as *const u8).read() as i64 } else { -1 };
    let n = PROC_TICKS.fetch_add(1, Ordering::Relaxed);
    // TIMING (crítico, provado): disparar cedo (asset-load) CRASHA (EXC_BAD_ACCESS — service não pronto).
    // Só arma DEPOIS que a engagement foi mostrada e já dismissou = estamos no MENU (o pregame session
    // inicializou o ecossistema do save-service). Sinal = engagement_active() foi true e agora é false.
    let eng = crate::overlay::engagement_active();
    if eng {
        ENGAGEMENT_WAS_SHOWN.store(true, Ordering::Relaxed);
    }
    let at_menu = ENGAGEMENT_WAS_SHOWN.load(Ordering::Relaxed) && !eng;
    let menu_n = if at_menu { MENU_TICKS.fetch_add(1, Ordering::Relaxed) } else { usize::MAX };
    if n < 6 || menu_n < 12 || SAVE_BYTE_LAST.swap(byte, Ordering::Relaxed) != byte {
        crate::log(&format!(
            "[savearm] ProcessRequests#{n} service={svc:p} ready[+0x503]={byte} eng={eng} at_menu={at_menu}"
        ));
    }
    // AÇÃO (gated ~/.bwms-savearm): chama a rotina REAL 0x100d68c88(service,2) que MONTA o scan de saves
    // (a 2ª parte dela). O ready-byte já vem 1 sem input MAS o scan NÃO está montado (menu sem CONTINUAR)
    // → precisa chamar mesmo com byte=1. Dispara algumas vezes NO MENU (o retry-loop RetrySaveDataRequestDelay
    // 1s reconcilia). Idempotente + early-return interno se preconds (disco/perfil) falham. SAVE_ARM_IN=reentrância.
    let _ = byte;
    if save_arm_enabled()
        && at_menu
        && !SAVE_ARM_IN.load(Ordering::Relaxed)
        && valid
        && SAVE_ARM_FIRES.load(Ordering::Relaxed) < 6
    {
        SAVE_ARM_IN.store(true, Ordering::Relaxed);
        let k = SAVE_ARM_FIRES.fetch_add(1, Ordering::Relaxed);
        let before = (svc.add(0x503) as *const u8).read();
        let arm: unsafe extern "C" fn(*mut u8, u32) =
            std::mem::transmute(crate::rebase(SAVE_ARM_FN_VM));
        arm(svc, 2);
        let after = (svc.add(0x503) as *const u8).read();
        crate::log(&format!(
            "[savearm] #{k} chamei 0x100d68c88(service,2) NO MENU tick={n} byte {before}->{after} (monta o scan)"
        ));
        SAVE_ARM_IN.store(false, Ordering::Relaxed);
    }
    let orig = ORIG_PROC_REQUESTS.load(Ordering::Relaxed);
    if orig.is_null() {
        return 0;
    }
    let f: unsafe extern "C" fn(*mut u8, *mut u8, usize, usize, usize, usize, usize, usize) -> usize =
        std::mem::transmute(orig);
    f(x0, x1, x2, x3, x4, x5, x6, x7)
}

/// Instala o hook do `ProcessRequests` (opt-in ~/.bwms-savearm). Guard de prólogo: aborta limpo se
/// o alvo não bater (nunca crasha). ProcessRequests é função grande → replace (16B) relocável.
unsafe fn install_save_arm() {
    if !save_arm_enabled() {
        return;
    }
    let target = crate::rebase(PROC_REQUESTS_VM);
    if !crate::gum::is_readable(target as *const c_void, 8) {
        crate::log("[savearm] ProcessRequests ilegível -> sem hook (sem crash)");
        return;
    }
    let got = core::ptr::read_unaligned(target as *const u64);
    if got != PROC_REQUESTS_PROLOGUE {
        crate::log(&format!(
            "[savearm] ProcessRequests não casou o prólogo ({got:#018x} != {PROC_REQUESTS_PROLOGUE:#018x}) -> sem hook"
        ));
        return;
    }
    let it = crate::gum::Interceptor::obtain();
    match it.replace(target, proc_requests_replacement as *mut c_void) {
        Some(orig) => {
            ORIG_PROC_REQUESTS.store(orig, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!(
                "[savearm] hook ProcessRequests @ {target:p} (captura service x1 + arma save-system sem input)"
            ));
        }
        None => crate::log("[savearm] FALHA ao hookar ProcessRequests"),
    }
}

/// Quantos opens de boot já falhamos. Cap = só os vídeos de boot; depois o bg do menu abre.
static SKIP_N: AtomicUsize = AtomicUsize::new(0);
/// Cap de opens FALHADOS por boot. Boot sadio falha ~8 (logos+attract). ACIMA disso = o movie-player do
/// jogo está num RETRY-LOOP num open que nunca sucede = a main-thread gira infinito (o early-stick:
/// CPU~107%, RSS~200MB, travado antes da engagement). Passar o cap = deixa abrir DE VERDADE pra quebrar o
/// loop. 64 fica muito acima do sadio → boot normal não muda (nenhum vídeo aparece).
const BOOT_OPEN_CAP: usize = 64;

/// Falha o OPEN dos primeiros N vídeos enquanto NÃO há player (fase de boot) → o jogo trata
/// o open falho PULANDO o vídeo (caso normal de erro), SEM travar mid-play (o que o
/// BinkShouldSkip fazia e quebrava o boot). Gameplay (player vivo) ou após N → abre normal.
unsafe extern "C" fn bink_open_replacement(name: *const i8, opts: *mut c_void) -> *mut c_void {
    let orig = ORIG_BINK_SKIP.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(*const i8, *mut c_void) -> *mut c_void = std::mem::transmute(orig);
    // WATCHDOG / getter-recusado pediram pra desistir do skip (boot travado ou patch recusado) →
    // deixa o vídeo/engagement nativo ABRIR de verdade em vez de manter a tela preta.
    if BINK_RELEASED.load(Ordering::Relaxed) {
        return f(name, opts);
    }
    // GAMEPLAY (player vivo): abre normal (braindances/cutscenes tocam).
    if !crate::current_player().is_null() {
        return f(name, opts);
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
            crate::log("[skipintro] bink open passou do cap -> deixando abrir de verdade (quebra o retry-loop/early-stick)");
        }
        return f(name, opts);
    }
    if n < 30 {
        crate::log(&format!("[skipintro] bink #{n}: OPEN FALHADO (vídeo/cena de boot NÃO abre)"));
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
    if func.is_null() || !crate::gum::is_readable(func as *const c_void, 0x18) {
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

/// Substituição do executor (ABI: `func@x0, ctx@x1, frame@x2, aOut@x3, a4@x4 -> x0`).
/// Espelha o callback do probe.js, mas em Rust: captura + tick periódico + chama a
/// original. (Observe/Override entram numa fase seguinte.)
unsafe extern "C" fn exec_replacement(
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
    a_out: *mut c_void,
    a4: *mut c_void,
) -> *mut c_void {
    capture(ctx);
    // F-B PARQUEADO: register_all no executor é TARDE — o executor dispara só em CHAMADAS de
    // script, DEPOIS do bind (~6s), que crasha antes (o bind é RESOLUÇÃO, não passa pelo executor).
    // Caminho real (lead do agente): AddPostRegisterCallback (CRTTISystem vtbl+0xC8) registrado
    // via hook do RegisterFunction (vtbl+0xA0, p/ pegar o singleton durante o build do RTTI).
    // Override RUST-nativo (validação do suppress, sem lua): se `func` é o método alvo,
    // escreve o aOut tipado + SUPRIME a original (retorna). Caminho Rust-only → à prova do
    // aninhamento que crashava o cb lua. Fast-path: 1 load; se 0 (nada armado), custo nulo.
    {
        let rov = RUST_OV_CNAME.load(Ordering::Relaxed);
        if rov != 0 && !func.is_null() && crate::gum::is_readable(func as *const c_void, 0x18) {
            let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
            if mcname == rov {
                let val = RUST_OV_VAL.load(Ordering::Relaxed);
                // DIAGNÓSTICO: escrita i64 DIRETA, SEM chamadas de vtable (GetName/GetSize) —
                // isola se eram as vtable-calls no stack aninhado que crashavam. O aOut do
                // call_func é 16B (seguro escrever 8). (Só vale p/ o teste via call_func.)
                if !a_out.is_null() {
                    (a_out as *mut i64).write_unaligned(val);
                    crate::log(&format!("[ovrust] suppress: escrevi {val} no aOut, pulando original"));
                    return 1usize as *mut c_void;
                }
            }
        }
    }
    // Observe RUST-nativo (`cet-hooks-shippable`, perna que faltava): se `func` é o alvo, LOGA que
    // o callback disparou e CAI FORA sem suprimir — a original roda normalmente logo depois.
    {
        let robs = RUST_OBS_CNAME.load(Ordering::Relaxed);
        if robs != 0 && !func.is_null() && crate::gum::is_readable(func as *const c_void, 0x18) {
            let mcname = core::ptr::read_unaligned((func as *const u8).add(0x10) as *const u64);
            if mcname == robs {
                RUST_OBS_HITS.fetch_add(1, Ordering::Relaxed);
                crate::log("[ovrust] observe: callback disparou, NAO suprimindo (original roda)");
            }
        }
    }
    // DESCOBERTA (dev): ring de CNames p/ achar o handler do clique do botão MODS.
    if crate::dev_mode() {
        discovery_ring(func);
    }
    // captura nativa do FromTDBID (fn/ctx/ret) p/ os cheats de item — substitui a sonda
    // frida. Casa pelo endereço (FROMTD_TGT, resolvido no tick); 1 compare, barato.
    let tgt = FROMTD_TGT.load(Ordering::Relaxed);
    if !tgt.is_null() && func == tgt && !ctx.is_null() && FROMTD_CTX.load(Ordering::Relaxed).is_null() {
        FROMTD_RET.store(a4, Ordering::Relaxed);
        FROMTD_CTX.store(ctx, Ordering::Relaxed); // por último: leitor vê RET pronto qdo CTX != null
    }
    // ROTEAMENTO de nativas registradas (Codeware): se `func` é uma nativa que NÓS
    // registramos no RTTI, despacha pro handler Rust e retorna — sem cair na via nativa
    // do jogo (cujo regIndex/tabela global não conhece nossa função). Fast-path dentro
    // de route_native: 0 registradas = 1 load atômico, hot-path intacto p/ todo mundo.
    if let Some(h) = crate::register::route_native(func) {
        crate::register::set_current_native_func(func); // pro handler ler seus args do frame
        h(ctx, frame, a_out, a4 as i64);
        return 1usize as *mut c_void;
    }
    let n = CALLS.fetch_add(1, Ordering::Relaxed);
    if n % TICK_EVERY == 0 {
        crate::cp77_tick();
    }
    // Observe/Override (mods que hookam funções do jogo): se vigiado, roda o `before`;
    // se pediu suppress (VOID, ou override-total de retorno POD já gravado no aOut), pula
    // a original e devolve bool=1. `a_out` vai junto p/ o override-total marshalar o retorno.
    let (suppress, mcname) = crate::hooks::watched_before(func, ctx, frame, a_out);
    if suppress {
        return 1usize as *mut c_void;
    }
    let orig = ORIG_EXEC.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: ExecFn = std::mem::transmute(orig);
    let r = f(func, ctx, frame, a_out, a4);
    if mcname != 0 {
        crate::hooks::watched_after(mcname, ctx, a_out);
    }
    r
}

/// Identifica player/tx pela CLASSE do ctx (sem `/tmp`, sem nome). Resolve as
/// classes-alvo uma vez (quando o RTTI está pronto) e cacheia.
unsafe fn capture(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    // PERF: se o ctx já é o player/tx que conhecemos, pula o `class_of` (caro) —
    // cobre o caso comum de várias chamadas seguidas no mesmo objeto.
    if ctx == crate::current_player() || ctx == crate::current_tx() {
        return;
    }
    if !crate::gum::is_readable(ctx as *const c_void, 0x40) {
        return;
    }
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
    if c == tx_cls {
        crate::set_current_tx(ctx);
    } else if !pl_cls.is_null() && c == pl_cls {
        crate::set_current_player(ctx);
    }
}
