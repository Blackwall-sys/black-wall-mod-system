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
static ORIG_EXEC: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLS_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CLS_PL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CALLS: AtomicU64 = AtomicU64::new(0);

// FromTDBID capturado NATIVAMENTE (fn/ctx/ret). Antes a sonda antiga escrevia isso em
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
    // agora faz igual. (A era antiga injetava diferente, por isso não pegou esse bug antes.)
    let n = _dyld_image_count();
    for i in 0..n {
        let nm = _dyld_get_image_name(i);
        if !nm.is_null() && CStr::from_ptr(nm).to_string_lossy().contains("Cyberpunk2077") {
            return true;
        }
    }
    false
}

/// (Trilha DEV) Já existe um injetor externo dirigindo o runtime? Só compila no
/// build com a feature `dev-gadget` — o build PÚBLICO nem inclui essa checagem
/// (nem os literais de nome do injetor), pois lá sempre carregamos nativo.
#[cfg(feature = "dev-gadget")]
unsafe fn external_loader_present() -> bool {
    // detector de injetor externo (só no perfil dev-gadget); no build público não há injetor.
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
    // Skip-intro (opt-in via marcador): pula os LOGOS de boot (funciona, ~10s).
    install_bink_skip();
    // PAUSADO: o hook do dispatcher não pula as telas "aperte espaço" (o dispatcher roda 1x
    // cedo, antes da dylib, e não redispara) — ver notes/boot-flow-phase-byte.md. Desligado p/
    // não arriscar o gameplay (o 8-arg forward num game-start). Skip volta após a FUNDAÇÃO
    // (replace_near4 + ponte redscript→Rust) destravar observabilidade e hook de função pequena.
    let _ = install_phase_skip; // mantém o código vivo p/ retomada; não instala.

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
    // F-A: prova do replace_near4 hookando o getter leaf de 8B (gated em ~/.bwms-near4test).
    // Confirma: hook instala + vizinha @0x3f5ec7c INTACTA (a causa do SIGILL do E2) + jogo vivo.
    install_near4_test();
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

/// Trampolim do `BinkShouldSkip` original (devolvido por `Interceptor::replace`).
static ORIG_BINK_SKIP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// SKIP-INTRO: hook em `BinkShouldSkip` (Bink SDK) → retorna 1 (pular) enquanto NÃO há
/// player vivo (fase de boot/menu) → os logos + a tela "APERTE ESPAÇO" pulam direto pro
/// menu. Em gameplay (player != null) chama a original (braindances/cutscenes tocam normal).
/// OPT-IN pelo marcador `/tmp/bwms-skipintro` (sem ele = zero efeito). dlsym; se não resolver,
/// no-op (sem crash).
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
    // DIAGNÓSTICO: loga TODA chamada (prova se o hook dispara de todo).
    let c = ORCH_CALLS.fetch_add(1, Ordering::Relaxed);
    if c < 3 {
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
    f(x0, x1, x2, x3, x4, x5, x6, x7)
}

pub(crate) unsafe fn install_bind_bridge() {
    let on = std::env::var("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-bind-bridge").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
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
    // DIAGNÓSTICO: loga as primeiras chamadas pra ver a sequência de fases que o dispatcher vê.
    let dn = DISP_N.fetch_add(1, Ordering::Relaxed);
    if dn < 40 {
        crate::log(&format!("[skipintro] dispatcher #{dn}: x0={x0:p} phase={v}"));
    }
    if SKIP_PHASE.load(Ordering::Relaxed) && readable && (v == 1 || v == 2) {
        (x0.add(0x84) as *mut i8).write(3);
        crate::log(&format!("[skipintro] dispatcher: phase {v}->3 (tela aperte-espaço -> menu)"));
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
            SKIP_PHASE.store(true, Ordering::Relaxed);
            std::mem::forget(it);
            crate::log(&format!(
                "[skipintro] hook no dispatcher de boot @ {target:p} (telas aperte-espaço -> menu)"
            ));
        }
        None => crate::log("[skipintro] FALHA ao hookar o dispatcher de boot"),
    }
}

/// Quantos opens de boot já falhamos. Cap = só os vídeos de boot; depois o bg do menu abre.
static SKIP_N: AtomicUsize = AtomicUsize::new(0);
const BOOT_OPEN_CAP: usize = 6;

/// Falha o OPEN dos primeiros N vídeos enquanto NÃO há player (fase de boot) → o jogo trata
/// o open falho PULANDO o vídeo (caso normal de erro), SEM travar mid-play (o que o
/// BinkShouldSkip fazia e quebrava o boot). Gameplay (player vivo) ou após N → abre normal.
unsafe extern "C" fn bink_open_replacement(name: *const i8, opts: *mut c_void) -> *mut c_void {
    let orig = ORIG_BINK_SKIP.load(Ordering::Relaxed);
    if orig.is_null() {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(*const i8, *mut c_void) -> *mut c_void = std::mem::transmute(orig);
    let hbink = f(name, opts); // ABRE o vídeo de verdade (handle válido)
    if hbink.is_null() || !crate::current_player().is_null() {
        return hbink; // gameplay ou open falho → normal
    }
    let n = SKIP_N.fetch_add(1, Ordering::Relaxed); // conta este open (boot, sem player)
    // Frames=1 nos primeiros N (logos) → vídeo instantâneo. As telas "APERTE ESPAÇO" NÃO
    // são tratadas aqui — são controller/phase-driven, não vídeo; quem as pula é o hook da
    // phase byte (install_phase_skip), que manda o jogo direto pro menu.
    if n < BOOT_OPEN_CAP && crate::gum::is_readable(hbink, 0x20) {
        (hbink as *mut u32).add(2).write_unaligned(1); // 1 frame → instantâneo
    }
    hbink
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
    // DESCOBERTA (dev): ring de CNames p/ achar o handler do clique do botão MODS.
    if crate::dev_mode() {
        discovery_ring(func);
    }
    // captura nativa do FromTDBID (fn/ctx/ret) p/ os cheats de item — substitui a sonda
    // legado. Casa pelo endereço (FROMTD_TGT, resolvido no tick); 1 compare, barato.
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
