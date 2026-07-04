//! cp77-console — runtime de mods do Black Wall Mod System, 100% Rust (sem JS/Lua),
//! carregado pelo Cyberpunk 2077 do macOS via LC_LOAD_DYLIB.
//!
//! Os hooks usam uma biblioteca de instrumentação chamada de Rust (gum). Cobre o
//! console/RTTI, cheats, TweakDB, NativeSettings e o self-boot nativo.
#![allow(dead_code)] // esqueleto: vários itens só passam a ser usados com os hooks

mod ai;
mod cname;
// `capture`: módulo de captura de frame/depth (experimental, uso interno). OFF por
// padrão = não entra na dylib pública. Liga com `--features capture`.
#[cfg(feature = "capture")]
mod capture;
mod console;
mod gum;
// hooks (roteador de method-hook CET) e lua = só com a feature `lua`. Sem ela, stubs no-op
// mantêm os call sites do executor/overlay intactos e o core fica 0% Lua (sem luajit).
#[cfg(feature = "lua")]
mod hooks;
#[cfg(not(feature = "lua"))]
#[path = "hooks_stub.rs"]
mod hooks;
#[cfg(feature = "lua")]
mod lua;
#[cfg(not(feature = "lua"))]
#[path = "lua_stub.rs"]
mod lua;
mod overlay;
mod plugins;
mod register;
mod rtti;

mod selfboot;
mod selftest;
mod targets;
mod tweakdb_bake;
mod tweakdb_rt;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

// Constructor do macOS: roda quando a .dylib é carregada no processo do jogo.
// Sem a crate `ctor` — usa a seção __mod_init_func direto (zero-dep).
#[link_section = "__DATA,__mod_init_func"]
#[used]
static CTOR: extern "C" fn() = on_load;

extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_header(image_index: u32) -> *const c_void;
    fn _dyld_get_image_name(image_index: u32) -> *const std::os::raw::c_char;
}

/// Base de link do executável do jogo (__TEXT em 0x1_0000_0000).
const LINK_BASE: u64 = 0x1_0000_0000;

/// Endereço de carga do binário PRINCIPAL do jogo, achado por NOME (robusto —
/// `_dyld_get_image_vmaddr_slide(0)` devolveu o slide da NOSSA dylib, não o do
/// jogo, quando carregada via Module.load/dlopen).
fn game_base() -> usize {
    unsafe {
        let n = _dyld_image_count();
        for i in 0..n {
            let name = _dyld_get_image_name(i);
            if name.is_null() {
                continue;
            }
            if let Ok(s) = std::ffi::CStr::from_ptr(name).to_str() {
                if s.ends_with("/Cyberpunk2077") || s.ends_with("Contents/MacOS/Cyberpunk2077") {
                    return _dyld_get_image_header(i) as usize;
                }
            }
        }
        _dyld_get_image_header(0) as usize // fallback: imagem principal
    }
}

/// VM addr do binário (base 0x1_0000_0000) → endereço real em runtime.
pub(crate) fn rebase(vmaddr: u64) -> *mut c_void {
    (game_base() + (vmaddr - LINK_BASE) as usize) as *mut c_void
}

/// Inverso de `rebase`: ponteiro runtime → VM addr estático do binário (p/ casar com
/// `nm`/símbolos no diagnóstico de vtable). 0 se o ponteiro estiver fora do módulo.
pub(crate) fn un_rebase(ptr: *const c_void) -> u64 {
    let p = ptr as usize;
    let b = game_base();
    if p < b {
        return 0;
    }
    (p - b) as u64 + LINK_BASE
}

pub(crate) fn log(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cp77-console.log")
    {
        let _ = writeln!(f, "{msg}");
    }
    // Em dev, ESPELHA no trace.log — sink confiável p/ diagnóstico de 1 ciclo: o console.log é
    // zerado ao abrir o console in-game (perde prints), o trace.log só zera no boot. Junta num
    // lugar só: loads de mod, prints de Lua (cheats/Cron), erros — pra ler tudo de uma vez.
    if dev_mode() {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/cp77-trace.log")
        {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// Modo-dev: liga diagnósticos verbosos (trace.log volumoso, logs de registro de hook, o
/// export `cp77-watch.txt` da era antiga). Default OFF → jogo LIMPO, registro "debaixo dos
/// panos". Liga com env `BWMS_DEV` ou tocando `/tmp/bwms-dev` (sem mexer no launch da Steam).
/// Checado 1x e cacheado.
pub(crate) fn dev_mode() -> bool {
    use std::sync::OnceLock;
    static DEV: OnceLock<bool> = OnceLock::new();
    *DEV.get_or_init(|| {
        std::env::var_os("BWMS_DEV").is_some() || std::path::Path::new("/tmp/bwms-dev").exists()
    })
}

/// Breadcrumb p/ crash nativo: SEMPRE sobrescreve /tmp/cp77-lasthook.txt com o ÚLTIMO ponto
/// (1 linha, barato — sobrevive a segfault, é o que diagnostica crash nativo). O trace.log
/// VOLUMOSO (sequência inteira, ~100KB/sessão) só é gravado em [`dev_mode`]: em jogo normal
/// não há a escrita nem o churn.
pub(crate) fn trace(msg: &str) {
    use std::io::Write;
    let _ = std::fs::write("/tmp/cp77-lasthook.txt", msg);
    if !dev_mode() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cp77-trace.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

extern "C" fn on_load() {
    if dev_mode() {
        let _ = std::fs::write("/tmp/cp77-trace.log", ""); // trace fresh por sessão (só dev)
    }
    log(&format!(
        "[cp77-console] carregada (Rust); game_base = {:#x}. Subindo thread do console.",
        game_base()
    ));
    // F-B: instala a ponte do bind orchestrator JÁ AQUI (topo do on_load), o mais cedo possível —
    // o bind do script (RedScriptsHost::Load → orchestrator @0x1021e897c) roda muito cedo, antes
    // do overlay/selfboot. É só patch de código (sem RTTI). Gated em ~/.bwms-bind-bridge.
    unsafe { selfboot::install_bind_bridge() };
    // Overlay (janela in-game) — swizzle do present do Metal numa thread própria.
    overlay::start();
    // Self-boot do runtime (hook do executor). O selfboot tem ctor próprio, mas na build
    // `--features lua` ele não roda confiável (luajit muda a ordem do __mod_init_func) →
    // disparamos AQUI também, do `on_load` que SEMPRE roda. É idempotente (guard ACTIVE).
    unsafe { selfboot::selfboot_if_needed() };
    // Execução de comandos: NÃO numa thread nossa (instanciar item da thread
    // errada crasha) — a sonda chama `cp77_tick` de dentro do hook do executor,
    // que é a THREAD DO JOGO. (Esse mesmo mecanismo serve pro Observe/Override.)
}

/// Registry global, obtida 1x na thread do jogo. Só-leitura após init → OnceLock
/// (sem Mutex, p/ o Lua poder ler sem deadlock dentro do cp77_tick).
struct SendReg(rtti::Registry);
unsafe impl Send for SendReg {}
unsafe impl Sync for SendReg {}
static REG: std::sync::OnceLock<SendReg> = std::sync::OnceLock::new();
/// player/tx atuais (a sonda captura, o cp77_tick publica; o Lua lê via Game.*).
static CURRENT_PLAYER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CURRENT_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn registry() -> Option<&'static rtti::Registry> {
    REG.get().map(|s| &s.0)
}
pub(crate) fn current_player() -> *mut c_void {
    CURRENT_PLAYER.load(Ordering::Relaxed)
}
pub(crate) fn set_current_player(p: *mut c_void) {
    CURRENT_PLAYER.store(p, Ordering::Relaxed);
}
pub(crate) fn set_current_tx(t: *mut c_void) {
    CURRENT_TX.store(t, Ordering::Relaxed);
}
pub(crate) fn current_tx() -> *mut c_void {
    CURRENT_TX.load(Ordering::Relaxed)
}

/// Heartbeat do runtime: o cp77_tick incrementa todo tick (na thread do jogo) só
/// quando há player/tx vivos. O badge do overlay lê isso → mostra "ativo" sem spam.
static TICKS: AtomicU64 = AtomicU64::new(0);
/// Quantos mods já foram carregados (loadmod) — exibido no badge.
static MODS_LOADED: AtomicUsize = AtomicUsize::new(0);
/// Auto-load dos mods (BWMS = Black Wall Mod System): dispara UMA vez quando o RTTI
/// fica pronto, pra a aba Mods/cheats vir ativa sem o usuário rodar `loadmods`.
static AUTO_LOADED: AtomicBool = AtomicBool::new(false);
static PLUGINS_LOADED: AtomicBool = AtomicBool::new(false);
/// Pasta padrão de mods. Sobreponível em runtime via `/tmp/cp77-mods-dir.txt`
/// (a sonda pode escrever o caminho certo no boot, p/ portabilidade).
fn mods_dir() -> String {
    // 1) override explícito (a sonda/dev pode fixar).
    if let Ok(s) = std::fs::read_to_string("/tmp/cp77-mods-dir.txt") {
        let s = s.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    // 2) PORTÁVEL: <pasta da nossa dylib>/blackwall-mods (a dylib mora em
    //    <jogo>/red4ext/ ao lado de blackwall-mods/). Funciona em qualquer máquina.
    if let Some(dir) = dylib_dir() {
        let p = format!("{dir}/blackwall-mods");
        if std::path::Path::new(&p).is_dir() {
            return p;
        }
    }
    // 3) fallback genérico: instalação Steam padrão sob o HOME do usuário (portável).
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/Library/Application Support/Steam/steamapps/common/Cyberpunk 2077/red4ext/blackwall-mods")
}

/// Pasta onde a NOSSA dylib está carregada (via dladdr no próprio código). Base
/// pra resolver caminhos relativos (mods) de forma portável.
fn dylib_dir() -> Option<String> {
    #[repr(C)]
    struct DlInfo {
        dli_fname: *const i8,
        dli_fbase: *mut c_void,
        dli_sname: *const i8,
        dli_saddr: *mut c_void,
    }
    extern "C" {
        fn dladdr(addr: *const c_void, info: *mut DlInfo) -> i32;
    }
    unsafe {
        let mut info: DlInfo = std::mem::zeroed();
        if dladdr(mods_dir as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
        path.rfind('/').map(|i| path[..i].to_string())
    }
}
pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}
pub(crate) fn mods_loaded() -> usize {
    MODS_LOADED.load(Ordering::Relaxed)
}

/// Hotkeys (registerHotkey): char registrado → set p/ a sonda/overlay checar barato;
/// fila de chars pressionados (overlay empurra na main thread, cp77_tick drena na
/// thread do jogo e dispara o callback Lua).
static HOTKEY_CHARS: std::sync::Mutex<Option<std::collections::HashSet<char>>> =
    std::sync::Mutex::new(None);
static HOTKEY_PRESSED: std::sync::Mutex<Vec<char>> = std::sync::Mutex::new(Vec::new());
pub(crate) fn hotkey_register_char(c: char) {
    let mut g = HOTKEY_CHARS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(c);
}
pub(crate) fn hotkey_is(c: char) -> bool {
    HOTKEY_CHARS
        .lock()
        .map(|g| g.as_ref().map_or(false, |s| s.contains(&c)))
        .unwrap_or(false)
}
pub(crate) fn hotkey_press(c: char) {
    if let Ok(mut g) = HOTKEY_PRESSED.lock() {
        if g.len() < 32 {
            g.push(c);
        }
    }
}
fn hotkey_drain() -> Vec<char> {
    HOTKEY_PRESSED
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// registerInput: chars com bind de input (down/up) + fila de eventos (char, isDown).
static INPUT_CHARS: std::sync::Mutex<Option<std::collections::HashSet<char>>> =
    std::sync::Mutex::new(None);
static INPUT_EVENTS: std::sync::Mutex<Vec<(char, bool)>> = std::sync::Mutex::new(Vec::new());
/// RawInput do CallbackSystem: TODAS as teclas (keycode) capturadas no sendEvent (gameplay), pra
/// emitir o evento "Input/Key" (≠ INPUT_EVENTS que é só das teclas registradas no registerInput).
static RAW_KEYS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());
pub(crate) fn push_raw_key(kc: i32) {
    if let Ok(mut q) = RAW_KEYS.lock() {
        if q.len() < 64 {
            q.push(kc);
        }
    }
}
fn drain_raw_keys() -> Vec<i32> {
    RAW_KEYS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}
pub(crate) fn input_register_char(c: char) {
    let mut g = INPUT_CHARS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(Default::default).insert(c);
}
pub(crate) fn input_is(c: char) -> bool {
    INPUT_CHARS
        .lock()
        .map(|g| g.as_ref().map_or(false, |s| s.contains(&c)))
        .unwrap_or(false)
}
pub(crate) fn input_event(c: char, down: bool) {
    if let Ok(mut g) = INPUT_EVENTS.lock() {
        if g.len() < 64 {
            g.push((c, down));
        }
    }
}
fn input_drain() -> Vec<(char, bool)> {
    INPUT_EVENTS
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// Chamado pela sonda DE DENTRO do hook do executor (= thread do jogo), throttled.
/// Publica player/tx, executa o comando pendente E roda o lifecycle dos mods Lua
/// (onUpdate) — tudo na thread certa.
/// Guard de re-entrância do tick (ver cp77_tick). RAII reseta no fim de qualquer caminho.
static TICK_BUSY: AtomicBool = AtomicBool::new(false);
struct TickGuard;
impl Drop for TickGuard {
    fn drop(&mut self) {
        TICK_BUSY.store(false, Ordering::Relaxed);
    }
}

#[no_mangle]
pub extern "C" fn cp77_tick() {
    // RE-ENTRÂNCIA: o tick roda DENTRO do hook do executor. Se um callback (Override/Cron/
    // console via call_func) re-entrar o executor e o módulo de tick bater de novo, NÃO
    // re-executa — senão a recursão exec→tick→…→exec→tick estoura a pilha e crasha (foi o
    // crash do `ovcall`). RAII (`_tg`) reseta o flag ao sair, em qualquer return.
    if TICK_BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    let _tg = TickGuard;
    if REG.get().is_none() {
        if let Some(r) = unsafe { rtti::Registry::obtain() } {
            let _ = REG.set(SendReg(r));
        }
    }
    let reg = match registry() {
        Some(r) => r,
        None => return,
    };
    // F-B: re-tenta registrar as nativas do BWMS se o selfboot pegou o RTTI cedo demais
    // (idempotente via REGISTERED). Garante BlackwallPing no RTTI p/ o bind do redscript.
    unsafe { crate::register::register_all() };
    // TweakDB runtime: dump observe-only do singleton (gated ~/.bwms-tdbdump) p/ confirmar
    // o records-map in-vivo antes de registrar record novo (PASSO 0 do clone-runtime).
    unsafe { crate::tweakdb_rt::dump_once_if_marked() };
    // TweakXL clone runtime-reg (PASSO final): registra Items.BwmsCloneTest no TweakDB vivo
    // via a nativa CreateRecord (gated ~/.bwms-tdbcreate). RecordExists antes/depois = prova.
    unsafe { crate::tweakdb_rt::create_once_if_marked() };
    // IA Fase 0: poll non-blocking da resposta do processo externo (throttled). Loga/exibe.
    crate::ai::poll_response();
    // F-B: GetFunction (vtbl+0x30) = vmaddr estático 0x102195024 (descoberto). PARQUEADO — não é
    // o resolvedor do binder do redscript. Resumir = achar/hookar o resolvedor real (~0x2192xxx).
    // Resolve o FromTDBID UMA vez e publica o endereço-alvo: o hook do executor então
    // captura fn/ctx/ret nativamente quando o jogo o chama (substitui a sonda antiga).
    if selfboot::active() && selfboot::FROMTD_TGT.load(Ordering::Relaxed).is_null() {
        let rf = unsafe { rtti::resolve_func(reg, "gameItemID", "FromTDBID") }
            .or_else(|| unsafe { rtti::resolve_func(reg, "ItemID", "FromTDBID") });
        if let Some(rf) = rf {
            selfboot::FROMTD_TGT.store(rf.func, Ordering::Relaxed);
        }
    }
    // Self-boot (modo nativo): o hook Rust já setou CURRENT_PLAYER/TX por captura
    // direta (sem /tmp). Trilha dev (injetor externo): lê os ponteiros do arquivo.
    let (player, tx) = if selfboot::active() {
        (current_player(), current_tx())
    } else {
        let (p, t) = read_inst();
        CURRENT_PLAYER.store(p, Ordering::Relaxed);
        CURRENT_TX.store(t, Ordering::Relaxed);
        (p, t)
    };
    // Auto-load dos mods (BWMS) na 1a vez com RTTI pronto — sem `loadmods` manual.
    // O loadmods não usa player/tx (só carrega os Lua + dispara onInit), então roda
    // mesmo no menu (onde player ainda é nulo) — aí o NativeSettings/cheats já
    // registram os hooks e a aba Mods vem ativa.
    if !AUTO_LOADED.swap(true, Ordering::Relaxed) {
        let dir = mods_dir();
        if std::path::Path::new(&dir).is_dir() {
            log(&format!("[bwms] auto-load de mods: {dir}"));
            load_mods_dir(&dir, true); // prod: pula testes/dev (carregáveis no manual)
        } else {
            log(&format!("[bwms] pasta de mods não achada p/ auto-load: {dir}"));
        }
    }
    // Carregador de plugin Rust (nativo, OPT-IN): só age se houver .dylib em
    // red4ext/plugins/. Sem plugin = zero impacto (Lua/jogo do usuário intactos).
    if !PLUGINS_LOADED.swap(true, Ordering::Relaxed) {
        if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
            plugins::load_plugins(&red4.join("plugins"));
        }
    }
    if player.is_null() || tx.is_null() {
        return;
    }
    // runtime vivo (player/tx ok): pulsa o heartbeat que o badge do overlay lê.
    TICKS.fetch_add(1, Ordering::Relaxed);
    // breadth Reflection: probe de CProperty num objeto VIVO (gated ~/.bwms-reflection-test), 1x.
    unsafe { register::reflection_live_once(player) };
    // breadth RED4ext/CET: valida vtable_hook/unhook na vtable real do player (gated ~/.bwms-vtable-test), 1x.
    unsafe { gum::vtable_selftest_once(player) };
    // CallbackSystem (lite): emite "Session/Ready" a cada ~120 ticks até despachar (espera o
    // OnGameAttached registrar). Aqui o "controller" = o tick de gameplay pronto; outros eventos
    // (input/entity) = hooks de função de jogo chamando fire_event. Ver register::fire_event.
    {
        static CBS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let t = TICKS.load(Ordering::Relaxed);
        if !CBS_DONE.load(Ordering::Relaxed) && t % 120 == 0 {
            let n = unsafe { register::fire_event("Session/Ready") };
            if n > 0 {
                CBS_DONE.store(true, Ordering::Relaxed);
            }
        }
        // "Session/Update" = evento PERIÓDICO (onUpdate, callback contínuo de mod). 2º tipo de evento.
        if t % 180 == 0 {
            unsafe { register::fire_event("Session/Update") };
        }
        // "Session/Tick" = evento periódico que PASSA DADO (o nº do tick) pro callback — prova que o
        // evento carrega args (= o que input/entity events precisam). callback = OnBwmsTick(n: Int32).
        if t % 240 == 0 {
            unsafe { register::fire_event_args("Session/Tick", &[rtti::Arg::I32(t as u32)]) };
        }
    }
    // ROTA A: registra os hooks Observe/Override pendentes (resolve a função na
    // thread do jogo + publica a lista de ptrs vigiados pra sonda).
    if hooks::has_pending() {
        unsafe { hooks::drain_pending(reg) };
    }
    // comando pendente (do overlay ou de fora, via /tmp)
    let cmd = std::fs::read_to_string("/tmp/cp77-cmd.txt").unwrap_or_default();
    let cmd = cmd.trim().to_string();
    if !cmd.is_empty() {
        let _ = std::fs::remove_file("/tmp/cp77-cmd.txt");
        run_cmd(reg, player, tx, &cmd);
    }
    // hotkeys pressionados (registerHotkey) → dispara callbacks na thread do jogo.
    for c in hotkey_drain() {
        unsafe { lua::fire_hotkey(c) };
    }
    // registerInput: eventos down/up → dispara callbacks com isDown na thread do jogo.
    for (c, down) in input_drain() {
        unsafe { lua::fire_input(c, down) };
    }
    // CallbackSystem RawInput controller: emite "Input/Key" (com o keycode) pra CADA tecla capturada
    // no sendEvent durante gameplay — o 1º controller REAL (hook de evento de jogo → fire_event_args).
    for kc in drain_raw_keys() {
        unsafe { register::fire_event_args("Input/Key", &[rtti::Arg::I32(kc as u32)]) };
    }
    // lifecycle: onOverlayOpen/onOverlayClose quando o console abre/fecha.
    {
        use std::sync::atomic::AtomicBool;
        static LAST_SHOW: AtomicBool = AtomicBool::new(false);
        let now = overlay::is_shown();
        if now != LAST_SHOW.swap(now, Ordering::Relaxed) {
            unsafe { lua::run_event(if now { "onOverlayOpen" } else { "onOverlayClose" }) };
        }
    }
    // lifecycle dos mods: onUpdate a cada tick (já throttled pela sonda).
    unsafe { lua::run_event("onUpdate") };
}

/// Chamado pela sonda DE DENTRO do hook do executor, SÓ para métodos vigiados
/// (Observe/Override), ANTES da original. `mcname` = CName do método (func+0x10,
/// lido pela sonda), `ctx` = o `this`. Retorna 1 se um Override pediu p/ SUPRIMIR a
/// original (futuro; hoje 0 = Observe puro).
#[no_mangle]
pub extern "C" fn cp77_obs_before(
    mcname: u64,
    func: *mut c_void,
    ctx: *mut c_void,
    frame: *mut c_void,
) -> u8 {
    // dispatch_before lê os args do frame e roda Observe/Override(wrapped); devolve suppress.
    // Export FFI legado (sonda antiga, morta): sem aOut aqui → res=null. Com res null o
    // override-total de retorno POD não grava nada (write_pod_ret=false) → cai no suppress
    // só-void de antes (seguro). O caminho vivo é o `exec_replacement` (passa o aOut real).
    u8::from(unsafe { hooks::dispatch_before(mcname, func, ctx, frame, std::ptr::null_mut()) })
}

/// Idem, mas DEPOIS da original rodar (ObserveAfter) + Override (reescreve `res`).
#[no_mangle]
pub extern "C" fn cp77_obs_after(mcname: u64, ctx: *mut c_void, res: *mut c_void) {
    unsafe {
        hooks::dispatch_after(mcname, ctx);
        hooks::dispatch_override(mcname, ctx, res);
    }
}

/// Carrega os mods de `<dir>/<nome>/init.lua` (estado limpo + onInit de todos).
/// `prod_only`=true (auto-load do primeiro uso) PULA mods de teste/experimentais
/// (nome com "Test" ou começando com "dev") — eles continuam vivos via `loadmods`
/// manual, nada é removido. `prod_only`=false carrega tudo.
fn load_mods_dir(dir: &str, prod_only: bool) -> usize {
    unsafe { lua::reset() };
    let mut count = 0usize;
    // Leitura ROBUSTA do diretório: o `entries.flatten()` antigo ENGOLIA em silêncio uma
    // entrada com erro de leitura transiente (volume externo) → o nativeSettings
    // sumia sem aviso e o cheats achava "NativeSettings nao encontrado", quebrando o botão
    // Mods. Agora: coleta TODAS as entradas, LOGA qualquer erro, e tenta de novo (até 3x) se
    // alguma entrada falhar — assim um hiccup transiente não derruba mais um mod inteiro.
    let mut paths: Vec<std::path::PathBuf> = Vec::new();
    for attempt in 0..3 {
        paths.clear();
        let mut any_err = false;
        match std::fs::read_dir(dir) {
            Ok(rd) => {
                for e in rd {
                    match e {
                        Ok(de) => paths.push(de.path()),
                        Err(er) => {
                            any_err = true;
                            log(&format!("[mods] entrada ilegível (tentativa {attempt}): {er}"));
                        }
                    }
                }
            }
            Err(er) => {
                any_err = true;
                log(&format!("[mods] dir ilegível (tentativa {attempt}): {er}"));
            }
        }
        if !any_err && !paths.is_empty() {
            break; // leitura limpa
        }
    }
    paths.sort(); // ordem determinística (não depende da ordem do FS)
    for path in &paths {
        // nome do mod = nome da pasta (pro GetMod), como no CET.
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if prod_only && (name.contains("Test") || name.starts_with("dev")) {
            continue; // fora do default; carregável no `loadmods` manual
        }
        let init = path.join("init.lua");
        if init.exists() {
            match std::fs::read_to_string(&init) {
                Ok(src) => {
                    unsafe { lua::run_mod(&name, &src, path) };
                    count += 1;
                    log(&format!("[mods] carregado: {}", init.display()));
                }
                Err(er) => log(&format!("[mods] erro lendo {}: {er}", init.display())),
            }
        }
    }
    unsafe { lua::run_event("onInit") };
    MODS_LOADED.fetch_add(count, Ordering::Relaxed);
    log(&format!("[mods] {count} mod(s) carregado(s) de {dir} (prod_only={prod_only})"));
    count
}

/// Parseia um arg de console p/ Reflection-Call: `i:5` `f:1.5` `b:true` `n:Name` `s:txt` `e:3`.
/// Sem prefixo conhecido → tenta i32. Marshaling real fica no rtti::Arg/call_func (provado).
fn parse_cmd_arg(s: &str) -> rtti::Arg {
    if let Some((ty, val)) = s.split_once(':') {
        match ty {
            "i" => return rtti::Arg::I32(val.parse::<i32>().map(|v| v as u32).unwrap_or(0)),
            "f" => return rtti::Arg::F32(val.parse::<f32>().unwrap_or(0.0)),
            "b" => return rtti::Arg::Bool(val == "true" || val == "1"),
            "n" => return rtti::Arg::CName(crate::cname::cname(val)),
            "e" => return rtti::Arg::Enum(val.parse::<u64>().unwrap_or(0)),
            "s" => return rtti::Arg::Str(val.to_string()),
            _ => {}
        }
    }
    rtti::Arg::I32(s.parse::<i32>().map(|v| v as u32).unwrap_or(0))
}

fn run_cmd(reg: &rtti::Registry, player: *mut c_void, tx: *mut c_void, cmd: &str) {
    // ROTA A: `lua <código>` roda Lua no runtime persistente (Game.* bindado).
    if let Some(code) = cmd.strip_prefix("lua ") {
        unsafe { lua::run_code(code) };
        return;
    }
    // `unloadmods` (ou `reset`): descarrega TODOS os mods (limpa o estado Lua).
    if cmd == "unloadmods" || cmd == "reset" {
        unsafe { lua::reset() };
        log("[mods] todos os mods descarregados");
        return;
    }
    // `loadmods <dir>` = mod-manager: estado limpo + varre <dir>/<nome>/init.lua e
    // carrega TODOS (coexistindo num estado só), depois dispara onInit de todos.
    // Estrutura de pasta de mod do CET = mods/<NomeDoMod>/init.lua.
    if let Some(dir) = cmd.strip_prefix("loadmods ") {
        // Manual = carrega TODOS (inclui dev/testes).
        load_mods_dir(dir.trim(), false);
        return;
    }
    // `loadmod <arquivo.lua>` carrega um mod (registerForEvent) + dispara onInit.
    if let Some(path) = cmd.strip_prefix("loadmod ") {
        match std::fs::read_to_string(path.trim()) {
            Ok(src) => {
                // nome do mod = stem do arquivo (ex: "meumod.lua" -> "meumod") pro GetMod
                let name = std::path::Path::new(path.trim())
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "mod".into());
                unsafe {
                    lua::reset(); // recarrega LIMPO (sem duplicar onDraw/onUpdate)
                    let d = std::path::Path::new(path.trim())
                        .parent()
                        .unwrap_or_else(|| std::path::Path::new("."));
                    lua::run_mod(&name, &src, d);
                    lua::run_event("onInit");
                }
                MODS_LOADED.fetch_add(1, Ordering::Relaxed);
                log(&format!("[console] mod carregado: {}", path.trim()));
            }
            Err(e) => log(&format!("[console] loadmod erro: {e}")),
        }
        return;
    }
    // `clear` (ou `cls`): limpa o scrollback do console (trunca o log) — estilo terminal.
    if cmd == "clear" || cmd == "cls" {
        overlay::clear_console_view(); // view-only: limpa a tela, mantém o arquivo p/ debug
        return;
    }
    // `ovcall` — validação do Override-suppress: chama BwmsProbe a partir do RUST (aqui,
    // run_cmd, SEM segurar o lock do Lua) → o override registrado via Lua consegue rodar
    // seu callback (o teste que chamava de dentro de um callback Lua/Cron falhava por
    // re-entrância: o lock do Lua estava seguro → `call_hook_override` dava try_lock Err →
    // override pulado). Na vida real o JOGO (nativo/redscript) chama o método, igual a isto.
    if cmd == "ovcall" {
        unsafe {
            let rp = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbe");
            let rc = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbeCalls");
            match (rp, rc) {
                (Some(rp), Some(rc)) => {
                    let i32_of = |o: Option<[u8; 16]>| {
                        o.map(|r| i32::from_le_bytes([r[0], r[1], r[2], r[3]])).unwrap_or(-1)
                    };
                    let before = i32_of(rtti::call_func(&rc, player, &[]));
                    // arma o override RUST-nativo de BwmsProbe→42 (sem lua) só p/ esta chamada
                    selfboot::RUST_OV_CNAME
                        .store(crate::cname::cname("BwmsProbe"), Ordering::Relaxed);
                    selfboot::RUST_OV_VAL.store(42, Ordering::Relaxed);
                    let ret = i32_of(rtti::call_func(&rp, player, &[])); // override RUST dispara AQUI
                    selfboot::RUST_OV_CNAME.store(0, Ordering::Relaxed); // desarma
                    let after = i32_of(rtti::call_func(&rc, player, &[]));
                    let verdict = if ret == 42 && after == before {
                        ">>> OVERRIDE-SUPPRESS OK (retorno reescrito + original suprimida) <<<"
                    } else if ret == 42 {
                        "retorno 42 mas a original rodou (rewrite, sem suppress)"
                    } else if after > before {
                        "original rodou (override nao pegou — CName/registro)"
                    } else {
                        "indefinido"
                    };
                    log(&format!(
                        "[ovcall] retorno={ret} (esperado 42) | contador {before}->{after} | {verdict}"
                    ));
                }
                _ => log("[ovcall] BwmsProbe/BwmsProbeCalls nao resolveram (sonda compilada?)"),
            }
        }
        return;
    }
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    // Pontos de desenvolvimento (PlayerDevelopmentData) — não retornam buffer.
    let pts = |member: &str, n: &str| {
        let q = n.parse().unwrap_or(1);
        let ok = unsafe { console::add_points(reg, player, q, member) };
        log(&format!("[console] '{cmd}' -> {}", if ok { "enviado" } else { "FALHOU" }));
    };
    let act = |label: &str, ok: bool| {
        log(&format!("[console] '{cmd}' -> {}", if ok { label } else { "FALHOU" }));
    };
    match parts.as_slice() {
        ["attrs", n] => return pts("Attribute", n),
        ["perks", n] => return pts("Primary", n),
        ["relic", n] => return pts("Espionage", n),
        ["level", n] => {
            return act("enviado", unsafe { console::level(reg, player, n.parse().unwrap_or(1)) })
        }
        ["godmode"] | ["god"] => return act("ON", unsafe { console::godmode(reg, player, true) }),
        ["godmode", "off"] | ["god", "off"] => {
            return act("OFF", unsafe { console::godmode(reg, player, false) })
        }
        ["heal"] => return act("curado", unsafe { console::heal(reg, player) }),
        ["summon"] | ["car"] => return act("enviado", unsafe { console::summon(reg, player) }),
        // Codeware/registro nativo (rodar no jogo p/ destravar a fundação):
        // cwprobe = despeja o layout de uma função nativa real → acha o offset do
        // handler; cwreg = smoke-test (registra BlackwallPing global); cwfacade =
        // registra Codeware.Version/Require (precisa do .reds do Codeware).
        ["cwprobe"] => return log(&unsafe { register::probe(reg) }),
        ["cwreg"] => return log(&unsafe { register::register_smoke(reg) }),
        ["cwfacade"] => return log(&unsafe { register::register_codeware_facade(reg) }),
        // DIAGNÓSTICO de construção RED (rodar no MENU PRINCIPAL = save-safe):
        // rttidump = só LÊ a vtable + getters (seguro); newobj = tiro único de
        // CONSTRUÇÃO (arriscado — pode crashar; por isso no menu, sem save aberto).
        ["rttidump", class] => {
            log(&unsafe { crate::rtti::dump_class(reg, class) });
            return;
        }
        ["propdump", class] => {
            log(&unsafe { crate::rtti::dump_props(reg, class) });
            return;
        }
        // TweakXL SetFlat runtime: getflat <nome> (read-only, dumpa o FlatValue vivo);
        // setflat <nome> <0xhex|int|float> (gated ~/.bwms-flatwrite; escreve 4B em +0x08).
        ["getflat", name] => {
            unsafe { crate::tweakdb_rt::probe_flat(name) };
            return;
        }
        ["setflat", name, val] => {
            let v: u32 = val
                .strip_prefix("0x")
                .and_then(|x| u32::from_str_radix(x, 16).ok())
                .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                .unwrap_or(0);
            unsafe { crate::tweakdb_rt::write_flat(name, v, 0x08) };
            return;
        }
        // Reflection USÁVEL (GetValue/SetValue por nome no PLAYER vivo): getf <prop> lê;
        // setf <prop> <0xhex|int|float> escreve. class_of(player) + find_property + prop_get/set.
        ["getf", prop] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    log(&format!("[getf] prop '{prop}' não achada na classe do player"));
                } else {
                    let vo = crate::rtti::prop_value_offset(p);
                    let u = crate::rtti::prop_get_u32(p, player);
                    let f = crate::rtti::prop_get_f32(p, player);
                    let b = crate::rtti::prop_get_bool(p, player);
                    log(&format!(
                        "[getf] {prop} (vo={vo:#x}) = u32 {u:#x} / i32 {} / f32 {f} / bool {b}",
                        u as i32
                    ));
                }
            }
            return;
        }
        ["setf", prop, val] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    log(&format!("[setf] prop '{prop}' não achada"));
                } else {
                    let v: u32 = val
                        .strip_prefix("0x")
                        .and_then(|x| u32::from_str_radix(x, 16).ok())
                        .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                        .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                        .unwrap_or(0);
                    let before = crate::rtti::prop_get_u32(p, player);
                    crate::rtti::prop_set_u32(p, player, v);
                    let after = crate::rtti::prop_get_u32(p, player);
                    log(&format!("[setf] {prop}: {before:#x} -> {after:#x} (pediu {v:#x})"));
                }
            }
            return;
        }
        // Reflection CALL com ARGS tipados: chama um método do player por nome, args =
        // `i:5 f:1.5 b:true n:Name s:txt e:3` (I32/F32/Bool/CName/Str/Enum). Sem args = getter.
        // Marshaling = o mesmo call_func dos cheats (provado). Completa get/set/call.
        ["callf", method, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                match crate::rtti::resolve_in_class(crate::rtti::class_of(player), method) {
                    Some(rf) => match crate::rtti::call_func(&rf, player, &args) {
                        // call_func devolve [u8;16] (sret). Interpreta como escalar (4B) E como
                        // Vector4/struct-de-16B (4×f32) — assim retornos como GetWorldPosition()→Vector4,
                        // GetWorldForward(), GetWorldOrientation()→Quaternion saem legíveis (Fase 1 do
                        // roadmap IA: posição/orientação do V por nome, sem RE nova).
                        Some(r) => {
                            let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                            log(&format!(
                                "[callf] {method}({}) -> i32 {} / u32 {:#x} / f32 {} / vec4 [{:.3}, {:.3}, {:.3}, {:.3}] / bytes {:02x?}",
                                raw_args.join(" "),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                f(0),
                                f(0), f(4), f(8), f(12),
                                &r[..]
                            ));
                        }
                        None => log(&format!("[callf] {method}({}) não completou (void ou falha)", raw_args.join(" "))),
                    },
                    None => log(&format!("[callf] método '{method}' não achado na classe do player")),
                }
            }
            return;
        }
        // Reflection CALL de função GLOBAL por nome (com args tipados). Complementa callf (instância).
        // Ex.: `callg Cos f:0.0` -> 1.0 ; `callg SqrtF f:4.0` -> 2.0. get_function + call_func (provados).
        ["callg", name, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                let f = register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    log(&format!("[callg] global '{name}' não resolveu"));
                } else {
                    let rf = crate::rtti::ResolvedFn {
                        func: f,
                        ret_type: std::ptr::null_mut(),
                        is_static: true,
                    };
                    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &args) {
                        Some(r) => log(&format!(
                            "[callg] {name}({}) -> f32 {} / i32 {} / u32 {:#x} / bytes {:02x?}",
                            raw_args.join(" "),
                            f32::from_bits(u32::from_le_bytes([r[0], r[1], r[2], r[3]])),
                            i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                            u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                            &r[..8]
                        )),
                        None => log(&format!("[callg] {name}({}) não completou", raw_args.join(" "))),
                    }
                }
            }
            return;
        }
        // smoke-test do Pilar 2 (CNamePool::Get): resolve um hash CName -> nome via pool
        // NATIVO. `cname 0x23427ae352f89652` deve dar "GetStatValue"; `cname 0` -> "None".
        ["cname", h] => {
            let hash = h
                .strip_prefix("0x")
                .and_then(|x| u64::from_str_radix(x, 16).ok())
                .or_else(|| h.parse::<u64>().ok())
                .unwrap_or(0);
            let name = crate::cname::resolve_cname(hash);
            log(&format!("[cname] {hash:#018x} -> '{name}'"));
            return;
        }
        ["newobj", class] => {
            log(&format!("[newobj] tentando construir '{class}' ..."));
            let p = unsafe { crate::rtti::new_object(reg, class) };
            log(&format!(
                "[newobj] '{class}' -> {:p} (static {:#x}) {}",
                p,
                un_rebase(p),
                if p.is_null() { "NULL (resolve/size falhou, sem crash)" } else { "OK (nao crashou)" }
            ));
            return;
        }
        _ => {}
    }
    let r = unsafe {
        match parts.as_slice() {
            ["money", n] => console::give(reg, player, tx, "Items.money", n.parse().unwrap_or(1)),
            ["give", name] => console::give(reg, player, tx, name, 1),
            ["give", name, n] => console::give(reg, player, tx, name, n.parse().unwrap_or(1)),
            ["remove", name] => console::remove(reg, player, tx, name, 1),
            ["remove", name, n] => console::remove(reg, player, tx, name, n.parse().unwrap_or(1)),
            _ => {
                // CET-style: o console É um REPL Lua. Comando não-reconhecido roda
                // como Lua — digitar `Game.AddMoney(7777)` direto funciona, igual CET.
                log(&format!("[console] (lua) {cmd}"));
                lua::run_code(cmd);
                return;
            }
        }
    };
    match r {
        Some(res) if res[0] != 0 => log(&format!("[console] '{cmd}' -> OK (GiveItem ret={})", res[0])),
        Some(res) => log(&format!(
            "[console] '{cmd}' -> NO-OP (GiveItem retornou {} — owner/tx errado?)",
            res[0]
        )),
        None => log(&format!("[console] '{cmd}' -> FALHOU (resolve/from_tdbid/call)")),
    }
}

/// Lê os ponteiros capturados pela sonda de /tmp/cp77-inst.txt ("player=0x…", "tx=0x…").
fn read_inst() -> (*mut c_void, *mut c_void) {
    let s = std::fs::read_to_string("/tmp/cp77-inst.txt").unwrap_or_default();
    let mut p: *mut c_void = std::ptr::null_mut();
    let mut t: *mut c_void = std::ptr::null_mut();
    for line in s.lines() {
        if let Some(v) = line.trim().strip_prefix("player=0x") {
            p = usize::from_str_radix(v.trim(), 16).unwrap_or(0) as *mut c_void;
        }
        if let Some(v) = line.trim().strip_prefix("tx=0x") {
            t = usize::from_str_radix(v.trim(), 16).unwrap_or(0) as *mut c_void;
        }
    }
    (p, t)
}
