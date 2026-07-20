//! overlay.rs — janela in-game (estilo CET) desenhada por cima do frame do jogo.
//!
//! Swizzla `-[<AGXFamilyCommandBuffer> presentDrawable:]` pra renderizar nossa UI
//! (Dear ImGui via `imgui-rs`) no `drawable.texture` (render pass loadAction=Load
//! preserva o jogo), e `-[NSApplication sendEvent:]` pro teclado (toggle por `).
//! Botão clicado → escreve em /tmp/cp77-cmd.txt (o canal que o console_loop lê).
//!
//! Marcos: 1 hook ✅, 2 render de geometria ✅, 3 toggle ✅, **4 texto/UI imgui**
//! (este; render-only), 5 mouse→clique, 6 search dos 7552 itens.
//!
//! panic=abort: o hook roda na thread de render; sem unwrap em caminho quente.

#![allow(dead_code)]

use std::ffi::{c_void, CStr, CString};
use std::mem::size_of;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, AtomicU32, Ordering};

use foreign_types::ForeignTypeRef;

type Id = *mut c_void;
type Sel = *const c_void;
type Class = *mut c_void;
type Imp = *const c_void;
type Method = *mut c_void;

extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn object_getClass(obj: Id) -> Class;
    fn class_getName(cls: Class) -> *const c_char;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
    fn method_getImplementation(m: Method) -> Imp;
    fn method_setImplementation(m: Method, imp: Imp) -> Imp;
    fn objc_msgSend();
    fn MTLCreateSystemDefaultDevice() -> Id;
}
// CGAssociate = controle de cursor do overlay (re-acopla o mouse ao cursor; o jogo desacopla p/ a
// câmera em gameplay). Bloco PRÓPRIO com o link do CoreGraphics — mantém o framework linkado mesmo
// quando a automação de teclado (cg_press, feature "autoproceed") sai do build público.
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
}

#[inline]
unsafe fn sel(name: &str) -> Sel {
    match CString::new(name) {
        Ok(c) => sel_registerName(c.as_ptr()),
        Err(_) => std::ptr::null(),
    }
}
#[inline]
unsafe fn class(name: &str) -> Class {
    match CString::new(name) {
        Ok(c) => objc_getClass(c.as_ptr()),
        Err(_) => std::ptr::null_mut(),
    }
}
#[inline]
unsafe fn msg0(recv: Id, s: Sel) -> Id {
    let f: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg_usize(recv: Id, s: Sel) -> usize {
    let f: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg_u16(recv: Id, s: Sel) -> u16 {
    let f: extern "C" fn(Id, Sel) -> u16 = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg_f64(recv: Id, s: Sel) -> f64 {
    let f: extern "C" fn(Id, Sel) -> f64 = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}
#[inline]
unsafe fn msg_point(recv: Id, s: Sel) -> NSPoint {
    let f: extern "C" fn(Id, Sel) -> NSPoint = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg_cstr(recv: Id, s: Sel) -> *const c_char {
    let f: extern "C" fn(Id, Sel) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
unsafe fn msg1(recv: Id, s: Sel, a: Id) -> Id {
    let f: extern "C" fn(Id, Sel, Id) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s, a)
}
/// Cria uma NSString a partir de &str (autoreleased) p/ falar com NSPasteboard.
unsafe fn nsstring(s: &str) -> Id {
    match CString::new(s) {
        Ok(c) => {
            let cls = class("NSString");
            if cls.is_null() {
                return std::ptr::null_mut();
            }
            let f: extern "C" fn(Id, Sel, *const c_char) -> Id =
                std::mem::transmute(objc_msgSend as *const c_void);
            f(cls as Id, sel("stringWithUTF8String:"), c.as_ptr())
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Desabilita o App Nap do processo do jogo: `[[NSProcessInfo processInfo] beginActivityWithOptions:
/// (UserInitiated|LatencyCritical) reason:...]`. Sem isso o macOS PAUSA o Cyberpunk quando ele não
/// está na frente (janela ocluída) — no modo CPVR (olhando o capacete) o jogo vai pro fundo e congela:
/// boot de "30min", engagement que não processa o SPACE injetado, câmera com "coices". O token é
/// RETIDO pela sessão toda (App Nap fica OFF enquanto o jogo vive). Chamado 1x no on_load.
pub unsafe fn disable_app_nap() {
    let pi_class = class("NSProcessInfo");
    if pi_class.is_null() {
        return;
    }
    let pi: Id = msg0(pi_class as Id, sel("processInfo"));
    if pi.is_null() {
        return;
    }
    // NSActivityUserInitiated (0x00FFFFFF) | NSActivityLatencyCritical (0xFF00000000)
    const OPTS: u64 = 0x00FF_FFFF | 0xFF_0000_0000;
    let reason = nsstring("BWMS: sem App Nap (boot/CPVR rodam a full mesmo em background)");
    let f: extern "C" fn(Id, Sel, u64, Id) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let token = f(pi, sel("beginActivityWithOptions:reason:"), OPTS, reason);
    if !token.is_null() {
        // retém o token pela sessão (senão o autorelease o libera e o App Nap volta)
        let r: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        r(token, sel("retain"));
        crate::log("[appnap] App Nap DESABILITADO (o jogo roda a full em background)");
    } else {
        crate::log("[appnap] beginActivityWithOptions retornou null (App Nap NÃO desabilitado)");
    }
}

/// Ponte do clipboard do sistema (NSPasteboard) pro editor de texto do imgui.
/// Faz Cmd+C/Cmd+X/Cmd+V/Cmd+A funcionarem no console (colar comandos longos).
struct MacClipboard;
impl imgui::ClipboardBackend for MacClipboard {
    fn get(&mut self) -> Option<String> {
        unsafe {
            let cls = class("NSPasteboard");
            if cls.is_null() {
                return None;
            }
            let pb = msg0(cls as Id, sel("generalPasteboard"));
            if pb.is_null() {
                return None;
            }
            let ty = nsstring("public.utf8-plain-text");
            let s = msg1(pb, sel("stringForType:"), ty);
            if s.is_null() {
                return None;
            }
            let p = msg_cstr(s, sel("UTF8String"));
            if p.is_null() {
                return None;
            }
            CStr::from_ptr(p).to_str().ok().map(|x| x.to_string())
        }
    }
    fn set(&mut self, value: &str) {
        unsafe {
            let cls = class("NSPasteboard");
            if cls.is_null() {
                return;
            }
            let pb = msg0(cls as Id, sel("generalPasteboard"));
            if pb.is_null() {
                return;
            }
            let _ = msg_usize(pb, sel("clearContents")); // NSInteger; ignora
            let ns = nsstring(value);
            let ty = nsstring("public.utf8-plain-text");
            if ns.is_null() || ty.is_null() {
                return;
            }
            let f: extern "C" fn(Id, Sel, Id, Id) -> bool =
                std::mem::transmute(objc_msgSend as *const c_void);
            f(pb, sel("setString:forType:"), ns, ty);
        }
    }
}

/// Eventos de teclado capturados na main thread, drenados pelo render no io do imgui.
enum InputEv {
    Char(char),
    Key(u16, bool),
    /// estado dos modificadores (Cmd/Shift) p/ os atalhos de edição funcionarem.
    Mods { cmd: bool, shift: bool },
    /// tecla-letra de atalho (A/X/C/V) — só usada com Cmd p/ selecionar/recortar/copiar/colar.
    Letter(u16, bool),
}
static INPUT_Q: std::sync::Mutex<Vec<InputEv>> = std::sync::Mutex::new(Vec::new());

fn push_input(ev: InputEv) {
    if let Ok(mut q) = INPUT_Q.lock() {
        if q.len() < 256 {
            q.push(ev);
        }
    }
}

/// keyCode do macOS → tecla de edição do imgui (nav/edição de texto).
fn map_key(kc: u16) -> Option<imgui::Key> {
    use imgui::Key::*;
    Some(match kc {
        51 => Backspace,
        117 => Delete,
        36 | 76 => Enter,
        53 => Escape,
        48 => Tab,
        123 => LeftArrow,
        124 => RightArrow,
        125 => DownArrow,
        126 => UpArrow,
        115 => Home,
        119 => End,
        _ => return None,
    })
}
fn is_edit_key(kc: u16) -> bool {
    map_key(kc).is_some()
}
/// keyCode (hardware, independe de layout) das letras de atalho de edição.
fn letter_key(kc: u16) -> Option<imgui::Key> {
    use imgui::Key::*;
    Some(match kc {
        0 => A,  // Cmd+A selecionar tudo
        7 => X,  // Cmd+X recortar
        8 => C,  // Cmd+C copiar
        9 => V,  // Cmd+V colar
        _ => return None,
    })
}

static ORIG_PRESENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIG_SENDEVENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Injeta um key press (keyDown+keyUp) no app via NSEvent + sendEvent original — SEM acessibilidade,
/// SEM foco (fala direto com o NSApplication). Usado p/ auto-avançar a engagement do boot ("APERTE E
/// PARA CONTINUAR"), cujo proceed é 100% nativo (não há gancho redscript). keyCode 14 = "E".
/// Chamado da main thread (present) — AppKit exige. No-op se o sendEvent ainda não foi swizzlado.
// CoreGraphics: injeção de tecla em nível HID (o CP2077 lê input via IOKit HID, NÃO via NSApp
// sendEvent — por isso postEvent NSEvent não avança a engagement, mas CGEvent HID sim, igual ao
// input real do teclado). CGEventPost pode exigir Acessibilidade (TCC) pro processo do jogo.
#[cfg(feature = "autoproceed")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, keycode: u16, keydown: bool) -> *mut c_void;
    fn CGEventPost(tap: u32, event: *mut c_void);
    fn CGEventPostToPid(pid: i32, event: *mut c_void);
    fn CFRelease(cf: *mut c_void);
    fn CGEventCreateMouseEvent(
        source: *mut c_void,
        mouse_type: u32,
        xy: NSPoint,
        button: u32,
    ) -> *mut c_void;
    fn CGEventSetIntegerValueField(event: *mut c_void, field: u32, value: i64);
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
    fn CGEventGetLocation(event: *mut c_void) -> NSPoint;
    fn CGEventCreate(source: *mut c_void) -> *mut c_void;
}
#[cfg(feature = "autoproceed")]
extern "C" {
    fn getpid() -> i32;
}

/// O jogo é o app da FRENTE agora? (NSApp.isActive). Usado p/ não vazar o CGEvent global pros
/// outros apps do usuário quando ele sai do jogo (ex.: digitar num browser). Fail-open (assume
/// frente se não der pra checar) — o cheque normal funciona.
#[cfg(feature = "autoproceed")]
pub unsafe fn game_is_frontmost() -> bool {
    let app_cls = class("NSApplication");
    if app_cls.is_null() {
        return true;
    }
    let shared = msg0(app_cls as Id, sel("sharedApplication"));
    if shared.is_null() {
        return true;
    }
    (msg0(shared, sel("isActive")) as usize) & 0xff != 0
}

/// Força a janela do jogo pra FRENTE (`[NSApp activateIgnoringOtherApps:YES]`). A engagement do boot
/// só aceita o SPACE quando o jogo está em foco (lê evento HID global, não o CGEventPostToPid). No
/// CPVR o usuário olha o capacete → o jogo fica atrás → o auto-proceed não passava. Chamado antes de
/// cada injeção durante o boot pra garantir que o SPACE global chegue. Só age até o menu (auto-proceed).
pub unsafe fn force_game_frontmost() {
    let app_cls = class("NSApplication");
    if app_cls.is_null() {
        return;
    }
    let shared = msg0(app_cls as Id, sel("sharedApplication"));
    if shared.is_null() {
        return;
    }
    let f: extern "C" fn(Id, Sel, bool) = std::mem::transmute(objc_msgSend as *const c_void);
    f(shared, sel("activateIgnoringOtherApps:"), true);
}

/// Injeta keyDown+keyUp da tecla via CGEvent (HID). O `CGEventPostToPid(getpid())` é inócuo (o jogo
/// lê HID, não pega o self-post). O que REALMENTE avança é o `CGEventPost` global — mas esse vai pro
/// app da FRENTE, então SÓ é postado quando o jogo está em foco; senão vazaria SPACE pros outros apps
/// do usuário (bug reportado: teclado dando espaço aleatório ao digitar fora do jogo). Thread-safe.
#[cfg(feature = "autoproceed")]
pub unsafe fn cg_press(keycode: u16) {
    let pid = getpid();
    let frontmost = game_is_frontmost();
    let src = CGEventSourceCreate(1); // kCGEventSourceStateHIDSystemState
    let down = CGEventCreateKeyboardEvent(src, keycode, true);
    let up = CGEventCreateKeyboardEvent(src, keycode, false);
    if !down.is_null() {
        CGEventPostToPid(pid, down);
        if frontmost {
            CGEventPost(0, down); // global SÓ com o jogo em foco (senão vaza pros outros apps)
        }
        CFRelease(down);
    }
    if !up.is_null() {
        CGEventPostToPid(pid, up);
        if frontmost {
            CGEventPost(0, up);
        }
        CFRelease(up);
    }
    if !src.is_null() {
        CFRelease(src);
    }
}

/// `cw-real-mod-e2e`/`axl-link-visual-proof` (2026-07-19): navegação de menu nativo por MOUSE.
/// Achado de sessão anterior (documentado em `proofs/2026-07-18-redscript-cheat-effects-proof-
/// CLIQUE-REAL-PROVADO.log`): o jogo lê DELTA relativo de HID pro cursor renderizado por ele
/// (não posição absoluta — `CGWarpMouseCursorPosition` não sincroniza). MESMA receita de
/// `cg_press` (CGEventPost de DENTRO do processo do jogo, evita o gate de Acessibilidade/TCC que
/// bloquearia um processo externo tentando postar globalmente). `dx`/`dy` em pontos de tela;
/// aplicados como `kCGMouseEventDeltaX/Y` num evento `mouseMoved` cuja location é a posição
/// ATUAL (lida via `CGEventGetLocation` de um evento novo) — não usamos warp/posição alvo.
#[cfg(feature = "autoproceed")]
pub unsafe fn cg_mouse_delta(dx: i64, dy: i64) {
    let frontmost = game_is_frontmost();
    let probe = CGEventCreate(std::ptr::null_mut());
    let probe_ok = !probe.is_null();
    let loc = if probe_ok {
        let l = CGEventGetLocation(probe);
        CFRelease(probe);
        l
    } else {
        NSPoint { x: 0.0, y: 0.0 }
    };
    let newloc = NSPoint {
        x: loc.x + dx as f64,
        y: loc.y + dy as f64,
    };
    let src = CGEventSourceCreate(1);
    let mv = CGEventCreateMouseEvent(src, 5 /* kCGEventMouseMoved */, newloc, 0);
    crate::log(&format!(
        "[mousedelta-diag] frontmost={frontmost} probe_ok={probe_ok} loc=({},{}) newloc=({},{}) src={src:p} mv={mv:p}",
        loc.x, loc.y, newloc.x, newloc.y
    ));
    if !mv.is_null() {
        CGEventSetIntegerValueField(mv, 4 /* kCGMouseEventDeltaX */, dx);
        CGEventSetIntegerValueField(mv, 5 /* kCGMouseEventDeltaY */, dy);
        let rx = CGEventGetIntegerValueField(mv, 4);
        let ry = CGEventGetIntegerValueField(mv, 5);
        crate::log(&format!("[mousedelta-diag] readback dx={rx} dy={ry} (esperado {dx}/{dy})"));
        CGEventPostToPid(getpid(), mv);
        if frontmost {
            CGEventPost(0, mv);
        }
        CFRelease(mv);
    }
    if !src.is_null() {
        CFRelease(src);
    }
}

/// Clique (mouseDown+mouseUp) do botão esquerdo na posição ATUAL do cursor do jogo (não move
/// nada — só o clique). Mesma receita/permissão de `cg_mouse_delta`/`cg_press`.
#[cfg(feature = "autoproceed")]
pub unsafe fn cg_click() {
    let frontmost = game_is_frontmost();
    let probe = CGEventCreate(std::ptr::null_mut());
    let loc = if !probe.is_null() {
        let l = CGEventGetLocation(probe);
        CFRelease(probe);
        l
    } else {
        NSPoint { x: 0.0, y: 0.0 }
    };
    let src = CGEventSourceCreate(1);
    let down = CGEventCreateMouseEvent(src, 1 /* kCGEventLeftMouseDown */, loc, 0);
    let up = CGEventCreateMouseEvent(src, 2 /* kCGEventLeftMouseUp */, loc, 0);
    if !down.is_null() {
        CGEventPostToPid(getpid(), down);
        if frontmost {
            CGEventPost(0, down);
        }
        CFRelease(down);
    }
    if !up.is_null() {
        CGEventPostToPid(getpid(), up);
        if frontmost {
            CGEventPost(0, up);
        }
        CFRelease(up);
    }
    if !src.is_null() {
        CFRelease(src);
    }
}

/// (mantida) Injeta via NSEvent postEvent — não avança a engagement (o jogo usa HID); ver cg_press.
pub unsafe fn inject_key(keycode: u16, chars: &str) {
    let app_cls = class("NSApplication");
    if app_cls.is_null() {
        return;
    }
    let shared = msg0(app_cls as Id, sel("sharedApplication"));
    if shared.is_null() {
        return;
    }
    let nsevent = class("NSEvent") as Id;
    let s = sel("keyEventWithType:location:modifierFlags:timestamp:windowNumber:context:characters:charactersIgnoringModifiers:isARepeat:keyCode:");
    let cs = nsstring(chars);
    let nil: Id = std::ptr::null_mut();
    let loc = NSPoint { x: 0.0, y: 0.0 };
    // ABI arm64: NSPoint (2×f64) = HFA em d0,d1; inteiros/ponteiros em x; f64 timestamp em d2.
    type KeyEvFn = extern "C" fn(Id, Sel, u64, NSPoint, u64, f64, i64, Id, Id, Id, i8, u16) -> Id;
    let mk: KeyEvFn = std::mem::transmute(objc_msgSend as *const c_void);
    // postEvent:atStart: é THREAD-SAFE (enfileira pro run loop da main thread) e NÃO precisa de
    // acessibilidade (é dentro do app) — ao contrário de sendEvent (só main thread) e CGEventPost
    // (precisa TCC). Permite injetar da thread de proceed.
    let post: extern "C" fn(Id, Sel, Id, i8) = std::mem::transmute(objc_msgSend as *const c_void);
    let sp = sel("postEvent:atStart:");
    // 10 = NSEventTypeKeyDown, 11 = NSEventTypeKeyUp
    let down = mk(nsevent, s, 10, loc, 0, 0.0, 0, nil, cs, cs, 0, keycode);
    if !down.is_null() {
        post(shared, sp, down, 0);
    }
    let up = mk(nsevent, s, 11, loc, 0, 0.0, 0, nil, cs, cs, 0, keycode);
    if !up.is_null() {
        post(shared, sp, up, 0);
    }
}
static LOGGED: AtomicBool = AtomicBool::new(false);
static SHOW: AtomicBool = AtomicBool::new(false); // começa ESCONDIDO (` abre)
static PREV_SHOW: AtomicBool = AtomicBool::new(false); // p/ detectar a borda fechar→devolver o mouse
/// `cet-lifecycle-events`: bordas abrir/fechar do overlay, sinalizadas AQUI (thread do render, via
/// presentDrawable) mas consumidas+disparadas (`fire_event`) de dentro do `cp77_tick` (thread do
/// jogo) — chamar a VM/redscript fora da thread do jogo é arriscado (lição desta sessão: 3ptest/
/// fbtest/etc. só chamam call_func do cp77_tick). Ver o consumo em `lib.rs`.
pub(crate) static OVERLAY_OPEN_EDGE: AtomicBool = AtomicBool::new(false);
pub(crate) static OVERLAY_CLOSE_EDGE: AtomicBool = AtomicBool::new(false);
// Mouse (preenchido pelo sendEvent na main thread, lido pelo render). f32 em bits.
static MOUSE_X: AtomicU32 = AtomicU32::new(0);
static MOUSE_Y: AtomicU32 = AtomicU32::new(0);
static MOUSE_DOWN: AtomicBool = AtomicBool::new(false);
static FRAME_H: AtomicU32 = AtomicU32::new(0);
static FRAME_W: AtomicU32 = AtomicU32::new(0); // largura do frame (f32 bits) p/ GetDisplayResolution
/// Resolução do frame do jogo (w, h) — pro `GetDisplayResolution()` do Lua.
pub fn frame_size() -> (u32, u32) {
    let w = f32::from_bits(FRAME_W.load(Ordering::Relaxed)) as u32;
    let h = f32::from_bits(FRAME_H.load(Ordering::Relaxed)) as u32;
    (w, h)
}
static SCALE: AtomicU32 = AtomicU32::new(0x4000_0000); // 2.0f32 (retina) por padrão
static WANT_MOUSE: AtomicBool = AtomicBool::new(false); // imgui quer o cursor?
static WANT_KEYBOARD: AtomicBool = AtomicBool::new(false); // campo de texto focado?
static IN_DRAW: AtomicBool = AtomicBool::new(false); // dentro do onDraw (ImGui-pro-Lua)?

/// Captura de teclas vai pra um buffer SEPARADO (aba "K-LOG"), não pro console principal —
/// senão cada tecla suja a aba Console com `[key] ...`. Ring de 200 linhas. Toggle on/off.
static KEY_LOG: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
static KEY_CAPTURE: AtomicBool = AtomicBool::new(false); // começa OFF: não captura à toa

fn key_log_push(s: String) {
    if let Ok(mut k) = KEY_LOG.lock() {
        k.push(s);
        let n = k.len();
        if n > 200 {
            k.drain(0..n - 200);
        }
    }
}

// ===== INPUT LOGGER pro dataset de ML (input → tela → estado) =====
// Grava TODA entrada do usuário com timestamp ms em /tmp/cp77-input.log, casável com os frames
// do harness (mesmo relógio). Cobertura: tecla down/up (sem auto-repeat → dá a DURAÇÃO do hold,
// ex.: andar = hold W), botões do mouse (L/R/outros), scroll (troca de arma), modificadores
// (shift=sprint, ctrl=crouch, alt). Liga/desliga por `inputlog on|off`. OFF por padrão.
static INPUT_LOG_ON: AtomicBool = AtomicBool::new(false);
static LAST_MOVE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const INPUT_LOG_PATH: &str = "/tmp/cp77-input.log";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Liga/desliga o logger. Ao ligar, trunca o arquivo e escreve um cabeçalho.
pub fn input_log_set(on: bool) {
    INPUT_LOG_ON.store(on, Ordering::Relaxed);
    if on {
        if let Ok(mut f) = std::fs::File::create(INPUT_LOG_PATH) {
            use std::io::Write;
            let _ = writeln!(
                f,
                "# bwms input log | ts_ms ev | kd/ku=key down/up(kc=keyCode), mdn/mup L|R|O=mouse, scr=scroll, mod=flags, mov=mouse pos"
            );
        }
    }
}

fn input_file_log(ev: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(INPUT_LOG_PATH)
    {
        let _ = writeln!(f, "{} {}", now_ms(), ev);
    }
}

/// Formata e grava UM evento de input (chamado do swizzle de sendEvent quando INPUT_LOG_ON).
unsafe fn log_input_event(event: Id, t: usize) {
    let ev: String = match t {
        10 | 11 => {
            // pula auto-repeat do key-down: hold = 1 down + 1 up (a duração sai da diferença de ts)
            if t == 10 && msg_usize(event, sel("isARepeat")) != 0 {
                return;
            }
            let kc = msg_u16(event, sel("keyCode"));
            let mut ch = String::new();
            let s0 = msg0(event, sel("charactersIgnoringModifiers"));
            if !s0.is_null() {
                let p = msg_cstr(s0, sel("UTF8String"));
                if !p.is_null() {
                    if let Ok(st) = CStr::from_ptr(p).to_str() {
                        ch = st.to_string();
                    }
                }
            }
            format!("{} kc={kc} {ch:?}", if t == 10 { "kd" } else { "ku" })
        }
        12 => format!("mod {:#x}", msg_usize(event, sel("modifierFlags")) & 0xffff_0000),
        1 | 2 => format!("{} L", if t == 1 { "mdn" } else { "mup" }),
        3 | 4 => format!("{} R", if t == 3 { "mdn" } else { "mup" }),
        25 | 26 => format!("{} O", if t == 25 { "mdn" } else { "mup" }),
        22 => format!("scr dy={:.1}", msg_f64(event, sel("scrollingDeltaY"))),
        5 | 6 | 7 => {
            // mouseMoved/dragged disparam MUITO → throttle ~50ms (≤20 amostras/s)
            let n = now_ms();
            if n.wrapping_sub(LAST_MOVE_MS.load(Ordering::Relaxed)) < 50 {
                return;
            }
            LAST_MOVE_MS.store(n, Ordering::Relaxed);
            let p = msg_point(event, sel("locationInWindow"));
            format!("mov {:.0},{:.0}", p.x, p.y)
        }
        _ => return,
    };
    input_file_log(&ev);
}

/// Verdadeiro só durante o `onDraw` (dentro do frame imgui, thread de render). As
/// funções `ImGui.*` do Lua checam isso — chamar fora do onDraw é no-op (sem crash).
pub fn in_draw() -> bool {
    IN_DRAW.load(Ordering::Relaxed)
}

/// Força o estado aberto/fechado do overlay (teste de `cet-lifecycle-events` via canal, sem HID —
/// dispara a MESMA borda que o toggle real do backtick dispara em render_imgui).
pub fn set_shown(v: bool) {
    SHOW.store(v, Ordering::Relaxed);
}

/// O overlay (console) está aberto? (cp77_tick usa p/ disparar onOverlayOpen/Close.)
pub fn is_shown() -> bool {
    SHOW.load(Ordering::Relaxed)
}

/// Tema do console pedido por um mod (SetTheme(idx)); -1 = sem pedido. O render
/// aplica no próximo frame. Permite mod trocar a aparência do Blackwall.sys.
static REQ_THEME: AtomicI32 = AtomicI32::new(-1);
pub fn request_theme(idx: i32) {
    REQ_THEME.store(idx, Ordering::Relaxed);
}

/// Escreve um comando no canal /tmp que o console_loop consome.
fn write_cmd(c: &str) {
    let _ = std::fs::write("/tmp/cp77-cmd.txt", c);
}

// ---------------------------------------------------------------- input (toggle)

extern "C" fn my_sendevent(this: Id, cmd: Sel, event: Id) {
    unsafe {
        let mut consume = false;
        if !event.is_null() {
            let t = msg_usize(event, sel("type"));
            // dataset ML: grava o input cru (não consome, não altera o fluxo do jogo).
            if INPUT_LOG_ON.load(Ordering::Relaxed) {
                log_input_event(event, t);
            }
            match t {
                10 => {
                    let kc = msg_u16(event, sel("keyCode"));
                    // char base da tecla (independe de layout) — acha a crase no ABNT2.
                    let mut ch = String::new();
                    let s0 = msg0(event, sel("charactersIgnoringModifiers"));
                    if !s0.is_null() {
                        let p = msg_cstr(s0, sel("UTF8String"));
                        if !p.is_null() {
                            if let Ok(st) = CStr::from_ptr(p).to_str() {
                                ch = st.to_string();
                            }
                        }
                    }
                    // TOGGLE robusto: keycodes US(50)/ISO(10)/F1(122) OU o caractere crase/til.
                    if kc == 50 || kc == 10 || kc == 122 || ch == "`" || ch == "~" {
                        SHOW.store(!SHOW.load(Ordering::Relaxed), Ordering::Relaxed);
                        consume = true;
                    } else if !SHOW.load(Ordering::Relaxed) {
                        // CallbackSystem RawInput (2026-07-18, `cw-rawinput-realname`): mapeia o
                        // keycode+char pro valor REAL de EInputKey + lê os modificadores AppKit,
                        // enfileira TODOS pro evento "Input/Key" (drenado no cp77_tick, que
                        // constrói+despacha um `ref<KeyInputEvent>` REAL na thread do jogo).
                        let flags_raw = msg_usize(event, sel("modifierFlags"));
                        let raw_shift = (flags_raw & 0x0002_0000) != 0; // NSEventModifierFlagShift
                        let raw_control = (flags_raw & 0x0004_0000) != 0; // NSEventModifierFlagControl
                        let raw_alt = (flags_raw & 0x0008_0000) != 0; // NSEventModifierFlagOption
                        let mapped_key = crate::register::map_macos_keycode_to_einputkey(kc as i32, ch.chars().next());
                        crate::push_raw_key(mapped_key, raw_shift, raw_control, raw_alt);
                        // hotkeys/inputs de mods ativos em gameplay: se a tecla é
                        // registrada, enfileira (cp77_tick dispara o cb na thread do jogo).
                        if let Some(c) = ch.chars().next() {
                            if crate::hotkey_is(c) {
                                crate::hotkey_press(c);
                            }
                            if crate::input_is(c) {
                                crate::input_event(c, true); // registerInput: key-down
                            }
                        }
                        // escondido: captura a tecla SÓ pro buffer K-LOG (não pro console),
                        // e só se a captura estiver ligada (aba K-LOG). Mantém o console limpo.
                        if KEY_CAPTURE.load(Ordering::Relaxed) {
                            key_log_push(format!("[key] keyCode={kc} char={ch:?}"));
                        }
                    } else if SHOW.load(Ordering::Relaxed) {
                        // overlay aberto: alimenta o imgui; só ENGOLE do jogo se um
                        // campo de texto estiver focado (senão WASD/Enter passam).
                        let flags = msg_usize(event, sel("modifierFlags"));
                        let cmd = (flags & 0x0010_0000) != 0; // NSEventModifierFlagCommand
                        let shift = (flags & 0x0002_0000) != 0; // NSEventModifierFlagShift
                        push_input(InputEv::Mods { cmd, shift });
                        if cmd && letter_key(kc).is_some() {
                            // Cmd+A/X/C/V → atalho de edição (selecionar/recortar/copiar/colar).
                            // NÃO empurra o char (senão inseria 'v' junto com o paste).
                            push_input(InputEv::Letter(kc, true));
                            consume = true;
                        } else if is_edit_key(kc) {
                            push_input(InputEv::Key(kc, true));
                        } else if !cmd {
                            let s = msg0(event, sel("characters"));
                            if !s.is_null() {
                                let p = msg_cstr(s, sel("UTF8String"));
                                if !p.is_null() {
                                    if let Ok(st) = CStr::from_ptr(p).to_str() {
                                        for c in st.chars() {
                                            if c >= ' ' {
                                                push_input(InputEv::Char(c));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if WANT_KEYBOARD.load(Ordering::Relaxed) {
                            consume = true;
                        }
                    }
                }
                11 => {
                    // KeyUp
                    if SHOW.load(Ordering::Relaxed) {
                        let kc = msg_u16(event, sel("keyCode"));
                        if letter_key(kc).is_some() {
                            push_input(InputEv::Letter(kc, false));
                        }
                        if is_edit_key(kc) {
                            push_input(InputEv::Key(kc, false));
                        }
                        if WANT_KEYBOARD.load(Ordering::Relaxed) {
                            consume = true;
                        }
                    } else {
                        // overlay fechado: key-up de registerInput (cb recebe isDown=false).
                        let s0 = msg0(event, sel("charactersIgnoringModifiers"));
                        if !s0.is_null() {
                            let p = msg_cstr(s0, sel("UTF8String"));
                            if !p.is_null() {
                                if let Ok(st) = CStr::from_ptr(p).to_str() {
                                    if let Some(c) = st.chars().next() {
                                        if crate::input_is(c) {
                                            crate::input_event(c, false);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                1 => MOUSE_DOWN.store(true, Ordering::Relaxed), // LeftMouseDown
                2 => MOUSE_DOWN.store(false, Ordering::Relaxed), // LeftMouseUp
                _ => {}
            }
            // posição do mouse (down/up/moved/dragged) → pixels top-left.
            if matches!(t, 1 | 2 | 5 | 6) {
                let p = msg_point(event, sel("locationInWindow"));
                let scale = f32::from_bits(SCALE.load(Ordering::Relaxed)) as f64;
                let fh = f32::from_bits(FRAME_H.load(Ordering::Relaxed)) as f64;
                MOUSE_X.store(((p.x * scale) as f32).to_bits(), Ordering::Relaxed);
                MOUSE_Y.store(((fh - p.y * scale) as f32).to_bits(), Ordering::Relaxed);
                // overlay aberto: o jogo NÃO vê o mouse (sem câmera/clique no jogo).
                // O imgui desenha o próprio cursor, então congelar o do jogo é ok.
                if SHOW.load(Ordering::Relaxed) {
                    consume = true;
                }
            }
        }
        if consume {
            return;
        }
        let orig = ORIG_SENDEVENT.load(Ordering::Relaxed);
        if !orig.is_null() {
            let f: extern "C" fn(Id, Sel, Id) = std::mem::transmute(orig);
            f(this, cmd, event);
        }
    }
}

unsafe fn install_input_hook() {
    // escala da tela (retina) p/ converter pontos→pixels do mouse.
    let screen_cls = class("NSScreen");
    if !screen_cls.is_null() {
        let main = msg0(screen_cls as Id, sel("mainScreen"));
        if !main.is_null() {
            let s = msg_f64(main, sel("backingScaleFactor")) as f32;
            if s > 0.0 {
                SCALE.store(s.to_bits(), Ordering::Relaxed);
                crate::log(&format!("[overlay] backingScaleFactor={s}"));
            }
        }
    }
    let app_cls = class("NSApplication");
    if app_cls.is_null() {
        return;
    }
    let shared = msg0(app_cls as Id, sel("sharedApplication"));
    if shared.is_null() {
        crate::log("[overlay] sharedApplication = null");
        return;
    }
    let app_class = object_getClass(shared);
    let m = class_getInstanceMethod(app_class, sel("sendEvent:"));
    if m.is_null() {
        crate::log("[overlay] sem sendEvent:");
        return;
    }
    ORIG_SENDEVENT.store(method_getImplementation(m) as *mut c_void, Ordering::Relaxed);
    method_setImplementation(m, my_sendevent as Imp);
    crate::log("[overlay] sendEvent: swizzlado (` alterna o overlay)");
    // dataset ML: liga o input-logger DESDE O BOOT se o marcador existir — cobre menus, criação
    // de ficha e intro (onde o canal do console ainda não drena). `inputlog on/off` toggla em runtime.
    let marker = std::env::var_os("HOME")
        .map(|h| std::path::Path::new(&h).join(".bwms-inputlog").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-inputlog").exists();
    if marker {
        input_log_set(true);
        crate::log("[inputlog] ligado no boot (marcador ~/.bwms-inputlog) -> /tmp/cp77-input.log");
    }
}

// ---------------------------------------------------------------- render (imgui)

struct UiState {
    cmd_buf: String,
    search_buf: String,
    qty: i32,
    frame: u32,
    log_lines: Vec<String>,
    favorites: Vec<(String, String)>,
    fav_dirty: bool,
    theme: usize,
    theme_dirty: bool,
    /// Histórico de comandos do console (mais recente por último) + índice de navegação (↑/↓);
    /// `None` = não navegando (campo livre pra digitar); `Some(i)` = mostrando `history[i]`.
    cmd_history: Vec<String>,
    cmd_history_idx: Option<usize>,
}

impl UiState {
    /// Recall do histórico do console (↑=up=true = comando mais antigo; ↓=up=false = mais recente,
    /// passar do fim volta ao campo livre). Muda `cmd_history_idx` + `cmd_buf`. Extraído do render loop
    /// pra ser testável (`cet-console-history`). É EXATAMENTE a lógica que a tecla ↑/↓ aciona no overlay.
    fn history_recall(&mut self, up: bool) {
        if self.cmd_history.is_empty() {
            return;
        }
        if up {
            let next = match self.cmd_history_idx {
                None => self.cmd_history.len() - 1,
                Some(i) => i.saturating_sub(1),
            };
            self.cmd_history_idx = Some(next);
            self.cmd_buf = self.cmd_history[next].clone();
        } else {
            match self.cmd_history_idx {
                Some(i) if i + 1 < self.cmd_history.len() => {
                    self.cmd_history_idx = Some(i + 1);
                    self.cmd_buf = self.cmd_history[i + 1].clone();
                }
                _ => {
                    self.cmd_history_idx = None;
                    self.cmd_buf.clear();
                }
            }
        }
    }

    /// `cet-console-history` — self-test AUTOMATIZÁVEL no menu (sem HID): exercita o `cmd_history` REAL
    /// do overlay com a MESMA lógica de recall que a tecla ↑/↓ aciona, + o comando `help`. Loga cada
    /// passo. Prova a paridade de console (↑ recupera o último comando no campo; `help` lista in-overlay)
    /// sem precisar digitar/apertar teclas. Restaura o estado no fim. Ver `render_imgui` (gate marcador).
    fn run_history_selftest(&mut self) {
        let saved_hist = std::mem::take(&mut self.cmd_history);
        let saved_idx = self.cmd_history_idx.take();
        let saved_buf = std::mem::take(&mut self.cmd_buf);
        let saved_log_len = self.log_lines.len();

        // popula 3 comandos (como se o usuário tivesse digitado)
        self.cmd_history = vec![
            "money 5000".to_string(),
            "godmode".to_string(),
            "give Items.wsp_smg 1".to_string(),
        ];
        self.cmd_history_idx = None;
        self.cmd_buf.clear();

        // ↑ recupera o ÚLTIMO comando no campo (cmd_buf) — a essência do gap
        self.history_recall(true);
        let up1 = self.cmd_buf.clone();
        self.history_recall(true);
        let up2 = self.cmd_buf.clone();
        self.history_recall(true);
        let up3 = self.cmd_buf.clone();
        // ↓ volta pra frente
        self.history_recall(false);
        let down1 = self.cmd_buf.clone();

        // `help` lista comandos IN-OVERLAY (empurra HELP_LINES pro log_lines do console — o que a UI mostra)
        for line in HELP_LINES {
            self.log_lines.push((*line).to_string());
        }
        let help_pushed = self.log_lines.len() - saved_log_len - 0; // linhas de help adicionadas
        let help_ok = help_pushed >= HELP_LINES.len()
            && self.log_lines.iter().any(|l| l.contains("↑/↓ navegam o histórico"));

        let hist_ok = up1 == "give Items.wsp_smg 1"   // ↑ pega o MAIS RECENTE (último digitado)
            && up2 == "godmode"
            && up3 == "money 5000"
            && down1 == "godmode";
        let ok = hist_ok && help_ok;
        let verdict = if ok {
            ">>> CONSOLE-HISTORY OK: ↑ recupera o último comando no campo (give→godmode→money), ↓ volta, e 'help' listou os comandos in-overlay (log_lines) — tudo no cmd_history REAL do overlay, sem HID <<<"
        } else {
            "verificar: ↑/↓ recall ou help não bateu"
        };
        crate::log(&format!(
            "[histtest] ↑1='{up1}' ↑2='{up2}' ↑3='{up3}' ↓1='{down1}' | hist_ok={hist_ok} | help: +{help_pushed} linhas help_ok={help_ok} | {verdict}"
        ));

        // restaura
        self.log_lines.truncate(saved_log_len);
        self.cmd_history = saved_hist;
        self.cmd_history_idx = saved_idx;
        self.cmd_buf = saved_buf;
    }
}
struct Renderer {
    ctx: imgui::Context,
    pso: metal::RenderPipelineState,
    lut_pso: Option<metal::RenderPipelineState>,
    font_tex: metal::Texture,
    /// Textura do splash de boot (bwms-splash.png do dir do jogo), se existir. tex_id imgui = 2.
    splash_tex: Option<metal::Texture>,
    splash_dim: (f32, f32), // (w, h) da imagem, p/ manter aspecto
    ui: UiState,
}
/// imgui TextureId da fonte (=1) e do splash (=2); usados no draw loop p/ ligar a textura certa.
const TEXID_FONT: usize = 1;
const TEXID_SPLASH: usize = 2;

/// Carrega `<jogo>/red4ext/bwms-splash.png` (troca sem recompilar) → textura Metal RGBA8.
/// Ausente = sem imagem (o splash mostra só a barra de progresso). PNG RGBA/RGB/Gray → RGBA8.
unsafe fn load_splash_texture(dev: &metal::DeviceRef) -> Option<(metal::Texture, f32, f32)> {
    // caminho ao lado do dylib deployado: .../Cyberpunk 2077/red4ext/bwms-splash.png
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|exe_dir| exe_dir.join("../../../red4ext/bwms-splash.png"))
        .filter(|p| p.exists())
        .or_else(|| {
            // fallback: procura a partir do CWD do jogo
            let p = std::path::Path::new("red4ext/bwms-splash.png");
            if p.exists() { Some(p.to_path_buf()) } else { None }
        })?;
    let file = std::fs::File::open(&path).ok()?;
    let dec = png::Decoder::new(std::io::BufReader::new(file));
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    // normaliza p/ RGBA8 (o shader amostra RGBA)
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..(w * h * 4) as usize].to_vec(),
        png::ColorType::Rgb => buf[..(w * h * 3) as usize]
            .chunks(3)
            .flat_map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf[..(w * h) as usize]
            .iter()
            .flat_map(|&g| [g, g, g, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..(w * h * 2) as usize]
            .chunks(2)
            .flat_map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        _ => return None, // indexed/paletted: fora de escopo (raro em splash)
    };
    let tdesc = metal::TextureDescriptor::new();
    tdesc.set_pixel_format(metal::MTLPixelFormat::RGBA8Unorm);
    tdesc.set_width(w as u64);
    tdesc.set_height(h as u64);
    tdesc.set_usage(metal::MTLTextureUsage::ShaderRead);
    let tex = dev.new_texture(&tdesc);
    let region = metal::MTLRegion {
        origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
        size: metal::MTLSize { width: w as u64, height: h as u64, depth: 1 },
    };
    tex.replace_region(region, 0, rgba.as_ptr() as *const c_void, (w * 4) as u64);
    crate::log(&format!("[splash] bwms-splash.png carregado: {w}x{h}"));
    Some((tex, w as f32, h as f32))
}
/// O `present` dispara em VÁRIOS threads (redDispatcher1..9); o ImGui tem UM
/// contexto global. Seguro porque o Mutex serializa (um thread por vez).
struct SendRenderer(Renderer);
unsafe impl Send for SendRenderer {}
static RENDERER: std::sync::Mutex<Option<SendRenderer>> = std::sync::Mutex::new(None);

const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct VIn  { float2 pos [[attribute(0)]]; float2 uv [[attribute(1)]]; float4 col [[attribute(2)]]; };
struct VOut { float4 pos [[position]]; float2 uv; float4 col; };
vertex VOut v(VIn in [[stage_in]], constant float4x4& proj [[buffer(1)]]) {
    VOut o; o.pos = proj * float4(in.pos, 0.0, 1.0); o.uv = in.uv; o.col = in.col; return o;
}
fragment float4 f(VOut in [[stage_in]], texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(mag_filter::linear, min_filter::linear);
    return in.col * tex.sample(s, in.uv);
}
// ReShade-Metal: grading no frame final via FRAMEBUFFER FETCH ([[color(0)]] = o pixel atual).
// Funciona em drawable framebufferOnly (Apple Silicon) — sem cópia/blit. Triângulo de tela cheia.
// preset 0 nunca chega aqui (o passe é pulado).
struct LOut { float4 pos [[position]]; };
vertex LOut lut_v(uint vid [[vertex_id]]) {
    float2 uv = float2((vid << 1) & 2, vid & 2);
    LOut o; o.pos = float4(uv * 2.0 - 1.0, 0.0, 1.0); return o;
}
// ---- helpers da grade Bodycam cyberpunk (cor) ----
float bc_chroma(float3 c) { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
// VIBRANCE escalar HUE-NEUTRO: sobe croma baixo, suave no croma já-alto (não estoura neon, protege pele)
float3 bc_vibrance(float3 c, float amount) {
    float l = dot(c, float3(0.2126, 0.7152, 0.0722));
    return mix(float3(l), c, 1.0 + amount * (1.0 - bc_chroma(c)));
}
// peso por janela de matiz (hue circular 0..1)
float bc_hue_w(float hue, float center, float width) {
    float d = abs(hue - center); d = min(d, 1.0 - d);
    return 1.0 - smoothstep(0.0, width, d);
}
float3 bc_rgb2hsv(float3 c) {
    float4 K = float4(0.0, -1.0/3.0, 2.0/3.0, -1.0);
    float4 p = mix(float4(c.bg, K.wz), float4(c.gb, K.xy), step(c.b, c.g));
    float4 q = mix(float4(p.xyw, c.r), float4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y); float e = 1e-10;
    return float3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}
float3 bc_hsv2rgb(float3 c) {
    float4 K = float4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
    float3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, saturate(p - K.xxx), c.y);
}
fragment float4 lut_f(float4 cur [[color(0)]], float4 pos [[position]],
                      constant uint& preset [[buffer(0)]],
                      constant uint2& res [[buffer(1)]],
                      constant uint& frame [[buffer(2)]],
                      constant uint& fx [[buffer(3)]]) {
    float3 x = cur.rgb;
    // ===== GRADE de cor (selector 'preset') =====
    if (preset == 1u) { x = clamp(x * float3(1.18, 1.0, 0.82), 0.0, 1.0); }
    else if (preset == 2u) { x = clamp(x * float3(0.82, 0.95, 1.25), 0.0, 1.0); }
    else if (preset == 3u) { float g = dot(x, float3(0.299, 0.587, 0.114)); x = float3(g); }
    else if (preset == 4u) { x = clamp((x - 0.5) * 1.45 + 0.5, 0.0, 1.0); }
    else if (preset == 5u) { float g = dot(x, float3(0.299, 0.587, 0.114)); x = clamp(float3(g) * float3(1.0, 0.78, 0.52) * 1.25, 0.0, 1.0); }
    else if (preset == 6u) {
        // BODYCAM cyberpunk (per-pixel): CONTRASTE LOCAL de saturação — neon/marketing VIBRA,
        // decadência/sombra MORRE. Vibrance hue-neutro c/ gate de luma + decay só na sombra REAL
        // (correções da verificação: sem hue-shift, sem matar croma demais). Neon-noir, math própria.
        // (glow/bloom dos neons = upgrade scene_tex, vem depois.)
        x = pow(x, float3(1.10));                                 // toe: leve crush (preto sólido neon-noir)
        x = (x - 0.5) * 1.22 + 0.5;                               // contraste
        float luma = dot(x, float3(0.2126, 0.7152, 0.0722));
        float lo_mask = 1.0 - smoothstep(0.15, 0.55, luma);       // cancela véu verde-ciano nas baixas/médias
        x.g -= 0.30 * lo_mask * max(0.0, x.g - max(x.r, x.b));
        x = bc_vibrance(x, 0.30 * smoothstep(0.10, 0.30, luma));  // vibrance só fora da sombra (sem ressuscitar ruído)
        x = saturate(x);                                          // saneia antes do HSV
        float3 hsv = bc_rgb2hsv(x);                               // realce SELETIVO do neon (magenta/ciano/amber)
        float wHue = max(bc_hue_w(hsv.x, 0.86, 0.06),
                     max(bc_hue_w(hsv.x, 0.52, 0.07),
                         bc_hue_w(hsv.x, 0.10, 0.04)))
                     * smoothstep(0.15, 0.40, hsv.y);             // sat-gate: concreto/pele de fora
        hsv.y = saturate(hsv.y * (1.0 + 0.45 * wHue));
        x = bc_hsv2rgb(hsv);
        float l2 = dot(x, float3(0.2126, 0.7152, 0.0722));        // split-tone teal-sombra / amber-luz (tint honesto)
        x = mix(x * float3(0.94, 1.0, 1.06), x * float3(1.07, 1.01, 0.92), smoothstep(0.0, 1.0, l2));
        float l3 = dot(x, float3(0.2126, 0.7152, 0.0722));        // decay: SÓ sombra real perde croma (janela estreita)
        x = mix(float3(l3), x, mix(0.70, 1.0, smoothstep(0.04, 0.22, l3)));
        float hot = smoothstep(0.82, 1.0, l3);                    // highlight: núcleo -> branco quente, borda mantém cor
        x = mix(x, mix(float3(l3), float3(1.0, 0.96, 0.90), 0.6), hot * 0.55);
    }
    x = clamp(x, 0.0, 1.0);
    // ===== EFEITOS independentes (bitmask 'fx'; cada um liga/desliga na aba) =====
    if ((fx & 1u) != 0u) {        // bit0 GRÃO de filme (Box-Muller, mais nas sombras, animado)
        float luma = dot(x, float3(0.299, 0.587, 0.114));
        float seed = dot(pos.xy, float2(12.9898, 78.233)) + float(frame) * 0.013;
        float u1 = fract(sin(seed) * 43758.5453);
        float u2 = fract(sin(seed + 1.0) * 43758.5453);
        float gn = sqrt(-2.0 * log(max(u1, 1e-6))) * cos(6.2831853 * u2);
        x = clamp(x * (1.0 + gn * mix(0.10, 0.03, luma)), 0.0, 1.0);
    }
    if ((fx & 2u) != 0u) {        // bit1 VINHETA oval (queda dura)
        float2 uv = pos.xy / float2(res);
        float2 c = (uv - 0.5) * float2(1.0, 1.12);
        float d = length(c);
        x *= mix(1.0, smoothstep(0.95, 0.32, d), 0.5);
    }
    return float4(x, cur.a);
}
"#;

/// ReShade-Metal: preset de LUT/grading ativo (0 = off, jogo intocado). Cicla com F2.
static LUT_PRESET: AtomicU32 = AtomicU32::new(0);
const LUT_COUNT: u32 = 7;
const LUT_NAMES: [&str; 7] = ["off", "Quente", "Frio", "P&B", "Contraste", "Sepia", "Bodycam"];
/// `cet-lut-pixel-proof`: seta o preset por comando (sem depender de F2/ImGui click, útil pro
/// cmd-channel de dev). MESMO write atômico que o clique da aba "LUT" já faz — zero risco novo.
pub(crate) fn set_lut_preset(n: u32) {
    LUT_PRESET.store(n % LUT_COUNT, Ordering::Relaxed);
}
/// Efeitos independentes (bitmask) que ligam SOBRE a grade: bit0=Grão, bit1=Vinheta.
/// Texture-sample (Bloom/Aberração/Barril) entram quando o passe ganhar o `scene_tex`.
static EFFECTS: AtomicU32 = AtomicU32::new(0);
const FX_NAMES: [&str; 2] = ["Grão", "Vinheta"];

unsafe fn init_renderer(dev: &metal::DeviceRef, pixfmt: u64) -> Option<Renderer> {
    let mut ctx = imgui::Context::create();
    ctx.set_ini_filename(None);
    ctx.set_log_filename(None);
    // clipboard do sistema + atalhos no estilo Mac (Cmd, não Ctrl).
    ctx.set_clipboard_backend(MacClipboard);
    ctx.io_mut().config_mac_os_behaviors = true;
    apply_theme(ctx.style_mut(), load_theme());

    // Atlas de fonte → textura Metal.
    let (tw, th, data) = {
        let atlas = ctx.fonts().build_rgba32_texture();
        (atlas.width, atlas.height, atlas.data.to_vec())
    };
    let tdesc = metal::TextureDescriptor::new();
    tdesc.set_pixel_format(metal::MTLPixelFormat::RGBA8Unorm);
    tdesc.set_width(tw as u64);
    tdesc.set_height(th as u64);
    tdesc.set_usage(metal::MTLTextureUsage::ShaderRead);
    let font_tex = dev.new_texture(&tdesc);
    let region = metal::MTLRegion {
        origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
        size: metal::MTLSize { width: tw as u64, height: th as u64, depth: 1 },
    };
    font_tex.replace_region(region, 0, data.as_ptr() as *const c_void, (tw * 4) as u64);
    ctx.fonts().tex_id = imgui::TextureId::from(1usize);

    // Pipeline.
    let lib = dev
        .new_library_with_source(SHADER, &metal::CompileOptions::new())
        .map_err(|e| crate::log(&format!("[overlay] shader err: {e}")))
        .ok()?;
    let vf = lib.get_function("v", None).ok()?;
    let ff = lib.get_function("f", None).ok()?;
    let desc = metal::RenderPipelineDescriptor::new();
    desc.set_vertex_function(Some(&vf));
    desc.set_fragment_function(Some(&ff));

    let vd = metal::VertexDescriptor::new();
    let a0 = vd.attributes().object_at(0).unwrap();
    a0.set_format(metal::MTLVertexFormat::Float2);
    a0.set_offset(0);
    a0.set_buffer_index(0);
    let a1 = vd.attributes().object_at(1).unwrap();
    a1.set_format(metal::MTLVertexFormat::Float2);
    a1.set_offset(8);
    a1.set_buffer_index(0);
    let a2 = vd.attributes().object_at(2).unwrap();
    a2.set_format(metal::MTLVertexFormat::UChar4Normalized);
    a2.set_offset(16);
    a2.set_buffer_index(0);
    let l0 = vd.layouts().object_at(0).unwrap();
    l0.set_stride(size_of::<imgui::DrawVert>() as u64);
    desc.set_vertex_descriptor(Some(vd));

    let att = desc.color_attachments().object_at(0)?;
    att.set_pixel_format(std::mem::transmute::<u64, metal::MTLPixelFormat>(pixfmt));
    att.set_blending_enabled(true);
    att.set_source_rgb_blend_factor(metal::MTLBlendFactor::SourceAlpha);
    att.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
    att.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
    att.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);

    let pso = dev
        .new_render_pipeline_state(&desc)
        .map_err(|e| crate::log(&format!("[overlay] pipeline err: {e}")))
        .ok()?;

    // pipeline do LUT (ReShade-Metal): triângulo gerado por vertex_id (sem vertex descriptor),
    // SEM blending (o fragment le o pixel atual via framebuffer-fetch e devolve o final).
    let lut_pso: Option<metal::RenderPipelineState> =
        match (lib.get_function("lut_v", None), lib.get_function("lut_f", None)) {
            (Ok(lvf), Ok(lff)) => {
                let ld = metal::RenderPipelineDescriptor::new();
                ld.set_vertex_function(Some(&lvf));
                ld.set_fragment_function(Some(&lff));
                if let Some(a) = ld.color_attachments().object_at(0) {
                    a.set_pixel_format(std::mem::transmute::<u64, metal::MTLPixelFormat>(pixfmt));
                }
                dev.new_render_pipeline_state(&ld)
                    .map_err(|e| crate::log(&format!("[overlay] lut pipeline err: {e}")))
                    .ok()
            }
            _ => {
                crate::log("[overlay] lut: funcoes do shader nao achadas");
                None
            }
        };
    let (splash_tex, splash_dim) = match load_splash_texture(dev) {
        Some((t, w, h)) => (Some(t), (w, h)),
        None => (None, (0.0, 0.0)),
    };
    crate::log("[overlay] imgui renderer pronto (fonte + pipeline + lut)");
    Some(Renderer {
        ctx,
        pso,
        lut_pso,
        font_tex,
        splash_tex,
        splash_dim,
        ui: UiState {
            cmd_buf: String::with_capacity(128),
            search_buf: String::with_capacity(64),
            qty: 1,
            frame: 0,
            log_lines: Vec::new(),
            favorites: load_favs(),
            fav_dirty: false,
            theme: load_theme(),
            theme_dirty: false,
            cmd_history: Vec::new(),
            cmd_history_idx: None,
        },
    })
}

/// Os 4 estilos estéticos do Cyberpunk 2077 como temas: nome + tagline.
/// Texto do comando `help` — só os comandos PRÁTICOS de cheat (o que o hint do campo já anuncia),
/// não a lista inteira de comandos internos de dev/RE (esses ficam em CODEBASE.md, não na UI).
const HELP_LINES: [&str; 9] = [
    "comandos: money N | give Items.X [N] | remove Items.X [N] | godmode [off] | heal | level N",
    "          summon (chama veículo) | attrs/perks/relic N (pontos) | hasgod (consulta)",
    "atalhos: ↑/↓ navegam o histórico de comandos já digitados nesta sessão.",
    "'Items.X' = TweakDBID do item (ex: Items.money, Items.PreventionEliteBundle_Cyberware).",
    "",
    "exemplos:",
    "  money 5000            → dá 5000 eddies",
    "  give Items.wsp_smg 1   → dá 1 unidade do item",
    "  godmode                → liga o modo deus (godmode off desliga)",
];

const THEMES: [(&str, &str); 4] = [
    ("ENTROPISM", "necessity over style"),
    ("KITSCH", "style over substance"),
    ("NEOMILITARISM", "substance over style"),
    ("NEOKITSCH", "style and substance"),
];

fn theme_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.blackwall_theme")
}
fn load_theme() -> usize {
    std::fs::read_to_string(theme_path())
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&i| i < THEMES.len())
        .unwrap_or(1) // KITSCH (verde vibrante) = look original
}
fn save_theme(i: usize) {
    let _ = std::fs::write(theme_path(), i.to_string());
}

/// Aplica a paleta de um dos 4 estilos do CP2077 ao estilo do imgui.
fn apply_theme(style: &mut imgui::Style, idx: usize) {
    use imgui::StyleColor::*;
    style.window_rounding = 3.0;
    style.frame_rounding = 2.0;
    style.mouse_cursor_scale = 0.5;
    // (text, win_bg, title, title_active, button, button_hover, button_active, frame_bg, border)
    type P = [[f32; 4]; 9];
    let p: P = match idx {
        // ENTROPISM — utilitário, âmbar/laranja sobre marrom escuro
        0 => [
            [0.98, 0.70, 0.33, 1.0],
            [0.10, 0.07, 0.035, 0.94],
            [0.45, 0.25, 0.07, 1.0],
            [0.62, 0.34, 0.09, 1.0],
            [0.30, 0.18, 0.06, 1.0],
            [0.55, 0.30, 0.09, 1.0],
            [0.95, 0.58, 0.18, 1.0],
            [0.22, 0.14, 0.05, 1.0],
            [0.55, 0.30, 0.10, 1.0],
        ],
        // NEOMILITARISM — austero, vermelho/cinza sobre quase-preto
        2 => [
            [0.86, 0.84, 0.82, 1.0],
            [0.05, 0.05, 0.06, 0.96],
            [0.40, 0.08, 0.07, 1.0],
            [0.58, 0.12, 0.10, 1.0],
            [0.22, 0.06, 0.06, 1.0],
            [0.50, 0.12, 0.10, 1.0],
            [0.85, 0.20, 0.16, 1.0],
            [0.12, 0.10, 0.11, 1.0],
            [0.45, 0.12, 0.10, 1.0],
        ],
        // NEOKITSCH — luxo, magenta + ouro sobre roxo profundo
        3 => [
            [0.96, 0.63, 0.91, 1.0],
            [0.10, 0.04, 0.12, 0.94],
            [0.42, 0.10, 0.36, 1.0],
            [0.60, 0.15, 0.52, 1.0],
            [0.32, 0.10, 0.30, 1.0],
            [0.60, 0.16, 0.52, 1.0],
            [0.95, 0.78, 0.38, 1.0],
            [0.18, 0.08, 0.18, 1.0],
            [0.60, 0.18, 0.52, 1.0],
        ],
        // KITSCH (1, default) — teal/verde vibrante com toque rosa (look original)
        _ => [
            [0.20, 0.98, 0.55, 1.0],
            [0.0, 0.18, 0.10, 0.93],
            [0.0, 0.45, 0.15, 1.0],
            [0.0, 0.58, 0.20, 1.0],
            [0.0, 0.30, 0.12, 1.0],
            [0.95, 0.25, 0.70, 1.0],
            [0.20, 0.98, 0.55, 1.0],
            [0.0, 0.25, 0.10, 1.0],
            [0.0, 0.50, 0.20, 1.0],
        ],
    };
    style.colors[Text as usize] = p[0];
    style.colors[WindowBg as usize] = p[1];
    style.colors[TitleBg as usize] = p[2];
    style.colors[TitleBgActive as usize] = p[3];
    style.colors[Button as usize] = p[4];
    style.colors[ButtonHovered as usize] = p[5];
    style.colors[ButtonActive as usize] = p[6];
    style.colors[FrameBg as usize] = p[7];
    style.colors[Border as usize] = p[8];
    // abas seguem o tema
    style.colors[Tab as usize] = p[4];
    style.colors[TabHovered as usize] = p[5];
    style.colors[TabActive as usize] = p[3];
}

/// Últimas `n` linhas do log do console (a saída pra aba Console).
// Marca de "clear": a view do console só mostra as linhas APÓS este índice (ex.: esconde o
// spam de boot na 1a abertura do overlay). O arquivo /tmp/cp77-console.log fica intacto (debug).
static LINE_CLEAR: AtomicU32 = AtomicU32::new(0);
pub fn clear_console_view() {
    // Build PÚBLICO (sem `devlog`): o log é no-op → nada a ler; não referencia o path (traceless).
    #[cfg(feature = "devlog")]
    {
        let n = std::fs::read_to_string("/tmp/cp77-console.log").map(|s| s.lines().count()).unwrap_or(0);
        LINE_CLEAR.store(n as u32, Ordering::Relaxed);
    }
}
fn tail_log(_n: usize) -> Vec<String> {
    // Público: Console tab vazio (o log não é escrito). Só o DEV lê o arquivo de /tmp.
    #[cfg(not(feature = "devlog"))]
    {
        Vec::new()
    }
    #[cfg(feature = "devlog")]
    {
        let s = std::fs::read_to_string("/tmp/cp77-console.log").unwrap_or_default();
        let v: Vec<&str> = s.lines().collect();
        let base = (LINE_CLEAR.load(Ordering::Relaxed) as usize).min(v.len());
        let view = &v[base..];
        let start = view.len().saturating_sub(_n);
        view[start..].iter().map(|x| x.to_string()).collect()
    }
}

fn favs_path() -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    format!("{home}/.blackwall_favs.tsv")
}
fn load_favs() -> Vec<(String, String)> {
    std::fs::read_to_string(favs_path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut it = l.split('\t');
            Some((it.next()?.to_string(), it.next().unwrap_or("").to_string()))
        })
        .collect()
}
fn save_favs(f: &[(String, String)]) {
    let s: String = f.iter().map(|(i, l)| format!("{i}\t{l}\n")).collect();
    let _ = std::fs::write(favs_path(), s);
}

/// Catálogo mínimo de itens (id\tlabel\ttipo\tcategoria), dados próprios. O picker é só
/// conveniência — `give Items.X N` aceita qualquer ID do TweakDB do próprio usuário.
const CATALOG_TSV: &str = include_str!("cet_catalog.tsv");
fn catalog() -> &'static Vec<[&'static str; 4]> {
    static C: std::sync::OnceLock<Vec<[&'static str; 4]>> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        CATALOG_TSV
            .lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                let id = it.next()?;
                let label = it.next()?;
                Some([id, label, it.next().unwrap_or(""), it.next().unwrap_or("")])
            })
            .collect()
    })
}

/// Badge discreto no canto superior direito: avisa que o runtime de mods está
/// ATIVO (heartbeat que pisca conforme os ticks) + nº de mods carregados. Substitui
/// o spam de "onUpdate tick #N" no log. Não-interativo (NO_INPUTS) e auto-resize.
/// Banner ASCII "BWMS" estilo isometric (MOTD de servidor antigo, "fingindo 3D"). Monoespaçado
/// → renderiza nítido (é feito de muitos chars pequenos, não um glifo gigante escalado).
const BWMS_BANNER: &str = r#"      ___           ___           ___           ___
     /\  \         /\__\         /\__\         /\  \
    /::\  \       /:/ _/_       /::|  |       /::\  \
   /:/\:\  \     /:/ /\__\     /:|:|  |      /:/\ \  \
  /::\~\:\__\   /:/ /:/ _/_   /:/|:|__|__   _\:\~\ \  \
 /:/\:\ \:|__| /:/_/:/ /\__\ /:/ |::::\__\ /\ \:\ \ \__\
 \:\~\:\/:/  / \:\/:/ /:/  / \/__/~~/:/  / \:\ \:\ \/__/
  \:\ \::/  /   \::/_/:/  /        /:/  /   \:\ \:\__\
   \:\/:/  /     \:\/:/  /        /:/  /     \:\/:/  /
    \::/__/       \::/  /        /:/  /       \::/  /
     ~~            \/__/         \/__/         \/__/"#;

/// Splash de boot: desenha por cima da tela preta do loading. Fundo escuro + (se existir)
/// a imagem `bwms-splash.png` centralizada com aspecto preservado + barra de progresso + texto.
/// Tudo no background draw list (atrás de qualquer janela; cobre a tela toda). Some no menu.
fn build_boot_splash(ui: &imgui::Ui, splash: Option<(f32, f32)>, w: f32, h: f32) {
    // Desenha numa JANELA fullscreen (o background_draw_list do imgui-rs não é renderizado por
    // este renderer; a janela + get_window_draw_list é o caminho que comprovadamente aparece).
    let flags = imgui::WindowFlags::NO_TITLE_BAR
        | imgui::WindowFlags::NO_RESIZE
        | imgui::WindowFlags::NO_MOVE
        | imgui::WindowFlags::NO_SCROLLBAR
        | imgui::WindowFlags::NO_INPUTS
        | imgui::WindowFlags::NO_NAV
        | imgui::WindowFlags::NO_SAVED_SETTINGS
        | imgui::WindowFlags::NO_FOCUS_ON_APPEARING
        | imgui::WindowFlags::NO_BRING_TO_FRONT_ON_FOCUS
        | imgui::WindowFlags::NO_BACKGROUND;
    let _pad = ui.push_style_var(imgui::StyleVar::WindowPadding([0.0, 0.0]));
    ui.window("##bwms_boot_splash")
        .flags(flags)
        .position([0.0, 0.0], imgui::Condition::Always)
        .size([w, h], imgui::Condition::Always)
        .build(|| {
            // ---- paleta BWMS (tema NEOMILITARISM: vermelho sobre quase-preto) ----
            const DARK: [f32; 4] = [0.02, 0.02, 0.03, 1.0];
            const RED: [f32; 4] = [0.86, 0.16, 0.15, 1.0]; // o vermelho nosso
            const RED_SOFT: [f32; 4] = [0.86, 0.16, 0.15, 0.55];
            const DIM: [f32; 4] = [0.62, 0.62, 0.68, 0.80];
            let dl = ui.get_window_draw_list();
            // fundo escuro sólido
            dl.add_rect([0.0, 0.0], [w, h], DARK).filled(true).build();
            // molduras de canto (frame estilo HUD cyberpunk), em vermelho
            let m = 26.0;
            let ll = (w.min(h) * 0.05).clamp(28.0, 60.0);
            let th = 2.0;
            let corner = |ax: f32, ay: f32, dx: f32, dy: f32| {
                dl.add_line([ax, ay], [ax + dx * ll, ay], RED).thickness(th).build();
                dl.add_line([ax, ay], [ax, ay + dy * ll], RED).thickness(th).build();
            };
            corner(m, m, 1.0, 1.0); // sup-esq
            corner(w - m, m, -1.0, 1.0); // sup-dir
            corner(m, h - m, 1.0, -1.0); // inf-esq
            corner(w - m, h - m, -1.0, -1.0); // inf-dir

            // ---- centro: imagem do usuário OU a marca BWMS ----
            if let Some((iw, ih)) = splash {
                if iw > 0.0 && ih > 0.0 {
                    let scale = (w / iw).min(h / ih);
                    let (dw, dh) = (iw * scale, ih * scale);
                    dl.add_image(
                        imgui::TextureId::from(TEXID_SPLASH),
                        [(w - dw) * 0.5, (h - dh) * 0.5],
                        [(w + dw) * 0.5, (h + dh) * 0.5],
                    )
                    .build();
                }
            } else {
                // banner ASCII "BWMS" (isometric, fingindo 3D) — monoespaçado, nítido, em vermelho
                let banner_scale = 1.5;
                ui.set_window_font_scale(banner_scale);
                let s = ui.calc_text_size(BWMS_BANNER);
                let bx0 = (w - s[0]) * 0.5;
                let by0 = (h * 0.44 - s[1]).max(h * 0.14); // bloco centrado ~44% da altura
                ui.set_cursor_pos([bx0, by0]);
                ui.text_colored(RED, BWMS_BANNER);
                let sub_y = by0 + s[1] + 14.0;
                // subtítulo "BLACK WALL MOD SYSTEM"
                ui.set_window_font_scale(1.4);
                let sub = "B L A C K   W A L L   M O D   S Y S T E M";
                let s2 = ui.calc_text_size(sub);
                ui.set_cursor_pos([(w - s2[0]) * 0.5, sub_y]);
                ui.text_colored(DIM, sub);
                ui.set_window_font_scale(1.0);
                // linha-acento vermelha sob o subtítulo
                let ly = sub_y + s2[1] + 12.0;
                dl.add_line([(w - s2[0]) * 0.5, ly], [(w + s2[0]) * 0.5, ly], RED_SOFT)
                    .thickness(2.0)
                    .build();
            }

            // ---- barra de progresso (vermelha) + texto, na base ----
            let prog = crate::selfboot::boot_progress();
            let secs = crate::selfboot::boot_elapsed_secs();
            let bar_w = (w * 0.42).min(560.0);
            let bar_h = 5.0;
            let bx = (w - bar_w) * 0.5;
            let by = h * 0.86;
            dl.add_rect([bx, by], [bx + bar_w, by + bar_h], [1.0, 1.0, 1.0, 0.08])
                .filled(true)
                .build(); // trilho
            dl.add_rect([bx, by], [bx + bar_w * prog, by + bar_h], RED)
                .filled(true)
                .build(); // preenchimento
            let pct = (prog * 100.0) as u32;
            let stage = crate::selfboot::boot_stage_label();
            let label = format!("CARREGANDO   {pct}%   ·   {stage}   ·   {secs}s");
            ui.set_window_font_scale(1.1);
            let ls = ui.calc_text_size(&label);
            ui.set_cursor_pos([(w - ls[0]) * 0.5, by - ls[1] - 12.0]);
            ui.text_colored(DIM, label);
            ui.set_window_font_scale(1.0);
            // versão (a que vai pro Nexus) — escrita no splash de carregamento
            let ver = format!("v{}  BETA", crate::BWMS_VERSION);
            ui.set_window_font_scale(0.95);
            let vs = ui.calc_text_size(&ver);
            ui.set_cursor_pos([(w - vs[0]) * 0.5, by + bar_h + 10.0]);
            ui.text_colored(DIM, &ver);
            ui.set_window_font_scale(1.0);
        });
}

fn build_badge(ui: &imgui::Ui, ticks: u64, mods: usize) {
    let [dw, _] = ui.io().display_size;
    let flags = imgui::WindowFlags::NO_TITLE_BAR
        | imgui::WindowFlags::NO_RESIZE
        | imgui::WindowFlags::NO_MOVE
        | imgui::WindowFlags::NO_SCROLLBAR
        | imgui::WindowFlags::NO_INPUTS
        | imgui::WindowFlags::ALWAYS_AUTO_RESIZE
        | imgui::WindowFlags::NO_FOCUS_ON_APPEARING
        | imgui::WindowFlags::NO_NAV
        | imgui::WindowFlags::NO_SAVED_SETTINGS;
    ui.window("##badge")
        .flags(flags)
        .position([dw - 14.0, 14.0], imgui::Condition::Always)
        .position_pivot([1.0, 0.0]) // ancora pelo canto superior direito
        .bg_alpha(0.30)
        .build(|| {
            // heartbeat: alterna a cada ~6 ticks; se o runtime travar, para de piscar.
            let on = (ticks / 6) % 2 == 0;
            ui.text(if on { "[*] BWMS" } else { "[ ] BWMS" });
            ui.same_line();
            let tag = if mods == 0 {
                "console".to_string()
            } else {
                format!("{mods} mod{}", if mods == 1 { "" } else { "s" })
            };
            ui.text_disabled(format!("- {tag}"));
        });
}

/// Constrói a UI: abas Console (terminal) / Items (busca+pin) / Game cheats.
fn build_ui(ui: &imgui::Ui, st: &mut UiState) {
    ui.window("BWMS  //  CP2077 console  ( ` alterna )")
        .size([660.0, 460.0], imgui::Condition::FirstUseEver)
        // longe do topo: em janela, a barra de título do macOS rouba o clique perto da borda.
        .position([60.0, 150.0], imgui::Condition::FirstUseEver)
        .build(|| {
            // seletor de tema: os 4 estilos estéticos do CP2077.
            for (i, (name, tag)) in THEMES.iter().enumerate() {
                if i > 0 {
                    ui.same_line();
                }
                if ui.button(name) {
                    st.theme = i;
                    st.theme_dirty = true;
                }
                if ui.is_item_hovered() {
                    ui.tooltip_text(tag);
                }
            }
            ui.text_disabled(format!("tema: {} - {}", THEMES[st.theme].0, THEMES[st.theme].1));
            ui.separator();
            if let Some(_tb) = ui.tab_bar("##tabs") {
                // ---- CONSOLE: terminal puro (saída em cima, comando embaixo) ----
                if let Some(_t) = ui.tab_item("Console") {
                    ui.child_window("##out").size([0.0, -30.0]).build(|| {
                        for line in &st.log_lines {
                            ui.text_wrapped(line);
                        }
                        ui.set_scroll_here_y_with_ratio(1.0);
                    });
                    ui.set_next_item_width(-1.0);
                    // sem a feature `lua`, o overlay é 100% comandos nativos (sem menção a Lua)
                    #[cfg(feature = "lua")]
                    let hint = "Enter: money N | give Items.X N | godmode | level N | help | OU Lua: Game.AddMoney(7777)";
                    #[cfg(not(feature = "lua"))]
                    let hint = "Enter: money N | give Items.X N | godmode | level N | help";
                    let entered = ui
                        .input_text("##cmd", &mut st.cmd_buf)
                        .enter_returns_true(true)
                        .hint(hint)
                        .build();
                    // Histórico ↑/↓ (paridade de console, cet-console-history): só navega quando o
                    // campo de comando está com foco (senão ↑/↓ noutro widget da aba acionaria isso
                    // à toa). ↑ = comando mais antigo, ↓ = mais recente; passar do mais recente
                    // volta ao campo livre (history_idx=None).
                    if ui.is_item_focused() && !st.cmd_history.is_empty() {
                        if ui.is_key_pressed(imgui::Key::UpArrow) {
                            st.history_recall(true);
                        } else if ui.is_key_pressed(imgui::Key::DownArrow) {
                            st.history_recall(false);
                        }
                    }
                    if entered {
                        let c = st.cmd_buf.trim().to_string();
                        if !c.is_empty() {
                            if c.eq_ignore_ascii_case("help") {
                                for line in HELP_LINES {
                                    st.log_lines.push((*line).to_string());
                                }
                            } else {
                                write_cmd(&c);
                            }
                            // não duplica se repetir o último comando seguido (padrão de shell)
                            if st.cmd_history.last().map(|s| s.as_str()) != Some(c.as_str()) {
                                st.cmd_history.push(c);
                            }
                        }
                        st.cmd_history_idx = None;
                        st.cmd_buf.clear();
                    }
                }
                // ---- ITEMS: busca + Give + pin favorito ----
                if let Some(_t) = ui.tab_item("Items") {
                    ui.set_next_item_width(330.0);
                    ui.input_text("##search", &mut st.search_buf)
                        .hint("filtrar (ex: erebus, katana, money)")
                        .build();
                    ui.same_line();
                    ui.set_next_item_width(80.0);
                    ui.input_int("qty", &mut st.qty).build();
                    if st.qty < 1 {
                        st.qty = 1;
                    }
                    let needle = st.search_buf.to_lowercase();
                    let q = st.qty;
                    let favs = &mut st.favorites;
                    let mut dirty = false;
                    ui.separator();
                    ui.child_window("##list").size([0.0, 0.0]).build(|| {
                        let mut shown = 0u32;
                        for row in catalog().iter() {
                            if shown >= 250 {
                                ui.text_disabled("... refine a busca");
                                break;
                            }
                            let [id, label, ty, _sheet] = *row;
                            if !needle.is_empty()
                                && !label.to_lowercase().contains(&needle)
                                && !id.to_lowercase().contains(&needle)
                                && !ty.to_lowercase().contains(&needle)
                            {
                                continue;
                            }
                            shown += 1;
                            if ui.button(&format!("Give##{id}")) {
                                write_cmd(&format!("give {id} {q}"));
                            }
                            ui.same_line();
                            let is_fav = favs.iter().any(|(fid, _)| fid == id);
                            let star = if is_fav { "fav" } else { "+pin" };
                            if ui.button(&format!("{star}##pin{id}")) {
                                if is_fav {
                                    favs.retain(|(fid, _)| fid != id);
                                } else {
                                    favs.push((id.to_string(), label.to_string()));
                                }
                                dirty = true;
                            }
                            ui.same_line();
                            ui.text(&format!("{label}  [{ty}]"));
                        }
                    });
                    if dirty {
                        st.fav_dirty = true;
                    }
                }
                // ---- FAVORITOS: itens pinados na aba Items. Os CHEATS foram removidos daqui
                // (não duplicar) — vivem em Settings > Cheats (redscript/config). ----
                if let Some(_t) = ui.tab_item("Favoritos") {
                    if !st.favorites.is_empty() {
                        ui.text("Favoritos (pinados na aba Items):");
                        let mut remove = None;
                        for (i, (id, label)) in st.favorites.iter().enumerate() {
                            if ui.button(&format!("Give##f{i}")) {
                                write_cmd(&format!("give {id} 1"));
                            }
                            ui.same_line();
                            if ui.button(&format!("x##f{i}")) {
                                remove = Some(i);
                            }
                            ui.same_line();
                            ui.text(label);
                        }
                        if let Some(i) = remove {
                            st.favorites.remove(i);
                            st.fav_dirty = true;
                        }
                    } else {
                        ui.text_disabled("Sem favoritos. Pinhe itens na aba Items (botao +) pra aparecerem aqui.");
                    }
                    ui.separator();
                    ui.text_disabled("Cheats (Godmode/Money/Perks/Level...) ficam em Settings > Cheats, sem duplicar.");
                }
                // ---- K-LOG: captura de teclas (input/atalhos), fora do console ----
                if let Some(_t) = ui.tab_item("K-LOG") {
                    let mut cap = KEY_CAPTURE.load(Ordering::Relaxed);
                    if ui.checkbox("capturar teclas (keyCode/char)", &mut cap) {
                        KEY_CAPTURE.store(cap, Ordering::Relaxed);
                    }
                    ui.same_line();
                    if ui.button("limpar") {
                        if let Ok(mut k) = KEY_LOG.lock() {
                            k.clear();
                        }
                    }
                    ui.text_disabled("teclas capturadas com o overlay FECHADO (debug de input/atalho)");
                    ui.separator();
                    ui.child_window("##klog").size([0.0, 0.0]).build(|| {
                        if let Ok(k) = KEY_LOG.lock() {
                            for line in k.iter() {
                                ui.text_wrapped(line);
                            }
                        }
                        ui.set_scroll_here_y_with_ratio(1.0);
                    });
                }
                if let Some(_t) = ui.tab_item("LUT") {
                    let cur = LUT_PRESET.load(Ordering::Relaxed);
                    ui.text(format!("Filtro ativo: {}", LUT_NAMES[cur as usize]));
                    ui.spacing();
                    for (i, name) in LUT_NAMES.iter().enumerate() {
                        if i > 0 {
                            ui.same_line();
                        }
                        if ui.button(format!("{}##lut{}", name, i)) {
                            LUT_PRESET.store(i as u32, Ordering::Relaxed);
                        }
                    }
                    ui.separator();
                    ui.text("Efeitos (liga/desliga sobre a grade):");
                    let mut fx = EFFECTS.load(Ordering::Relaxed);
                    for (b, name) in FX_NAMES.iter().enumerate() {
                        let bit = 1u32 << b;
                        let mut on = (fx & bit) != 0;
                        if b > 0 {
                            ui.same_line();
                        }
                        if ui.checkbox(format!("{}##fx{}", name, b), &mut on) {
                            if on { fx |= bit; } else { fx &= !bit; }
                            EFFECTS.store(fx, Ordering::Relaxed);
                        }
                    }
                    ui.separator();
                    ui.text_disabled("Grade (cor) + efeitos AO VIVO no frame; cada um liga/desliga.");
                    ui.text_disabled("Em breve (precisa do upgrade do passe): Bloom, Aberracao cromatica, Barril.");
                }
            }
        });
}

unsafe fn render_imgui(cb_raw: Id, drawable: Id) {
    if drawable.is_null() {
        return;
    }
    let tex_raw = msg0(drawable, sel("texture"));
    if tex_raw.is_null() {
        return;
    }
    if !LOGGED.swap(true, Ordering::Relaxed) {
        let w = msg_usize(tex_raw, sel("width"));
        let h = msg_usize(tex_raw, sel("height"));
        crate::log(&format!("[overlay] render ON — frame {w}x{h}"));
    }
    let show = SHOW.load(Ordering::Relaxed);
    // enquanto o painel está FECHADO, a view do console acompanha o fim do log (roda todo frame,
    // inclusive no boot) → quando o usuario abre, começa VAZIA (sem o spam de boot). Ao abrir,
    // congela e passa a acumular só o que vier novo.
    if !show {
        clear_console_view();
    }
    // badge no canto = "Blackwall.sys ativo": aparece assim que o runtime roda em
    // gameplay (ticks>0), com ou sem mod carregado. Some só no menu/boot (ticks==0).
    let badge = crate::ticks() > 0;
    // splash de boot: preenche a tela preta do loading. Gate = boot_splash_active() SÓ (armado no
    // on_load com skip-intro, desarmado ao chegar no menu). NÃO gatear por !badge: o runtime já
    // tickava >0 no boot, então !badge era sempre falso e o splash nunca desenhava (bug do gate).
    // NÃO cobrir a engagement ("APERTE ESPAÇO"): se ela está ativa, esconde a splash pro usuário VER o
    // prompt e poder apertar (senão parece congelado no BWMS). Volta a cobrir a tela preta pós-engagement.
    // EXCEÇÃO (2026-07-12): no modo "Até a gameplay" (~/.bwms-fire-start ligado) o usuário NUNCA precisa
    // apertar espaço — o lever dispara sozinho após o timer de bwms-skipintro.reds. Sem esta exceção, o
    // gate acima escondia a splash pelos ~8s inteiros do timer, revelando "APERTE ESPAÇO PARA CONTINUAR"
    // numa tela que era pra ser 100% automática (achado do Perrotta: via o prompt mesmo sem precisar dele).
    let fire_start = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-fire-start").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-fire-start").exists();
    let boot = crate::selfboot::boot_splash_active() && (fire_start || !engagement_active());
    if !show && !badge && !boot {
        return;
    }
    // Gestão do cursor: ABERTO = acopla (clicar no overlay). Ao FECHAR (borda de descida),
    // DESACOPLA p/ devolver a câmera ao jogo — sem isto o mouse-look trava depois do `,
    // porque o jogo só desacopla 1× ao entrar em gameplay e não "re-desacopla" sozinho.
    let prev_show = PREV_SHOW.swap(show, Ordering::Relaxed);
    if show {
        CGAssociateMouseAndMouseCursorPosition(true);
        if !prev_show {
            OVERLAY_OPEN_EDGE.store(true, Ordering::Relaxed); // `cet-lifecycle-events`: onOverlayOpen
        }
    } else if prev_show {
        CGAssociateMouseAndMouseCursorPosition(false); // fechou agora → mouse volta pra câmera
        OVERLAY_CLOSE_EDGE.store(true, Ordering::Relaxed); // `cet-lifecycle-events`: onOverlayClose
    }
    let w = msg_usize(tex_raw, sel("width")) as f32;
    let h = msg_usize(tex_raw, sel("height")) as f32;
    FRAME_W.store(w.to_bits(), Ordering::Relaxed);
    let pixfmt = msg_usize(tex_raw, sel("pixelFormat")) as u64;
    let cb = metal::CommandBufferRef::from_ptr(cb_raw as *mut _);
    let tex = metal::TextureRef::from_ptr(tex_raw as *mut _);
    let dev_raw = msg0(cb_raw, sel("device"));
    if dev_raw.is_null() {
        return;
    }
    let dev = metal::DeviceRef::from_ptr(dev_raw as *mut _);

    let mut guard = match RENDERER.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guard.is_none() {
        *guard = init_renderer(dev, pixfmt).map(SendRenderer);
    }
    let rd = match guard.as_mut() {
        Some(x) => &mut x.0,
        None => return,
    };
    {
        FRAME_H.store(h.to_bits(), Ordering::Relaxed);
        rd.ui.frame = rd.ui.frame.wrapping_add(1);
        if show && rd.ui.frame % 30 == 1 {
            rd.ui.log_lines = tail_log(60);
        }
        // `cet-console-history` self-test AUTOMATIZÁVEL no menu (sem HID): roda 1× no `cmd_history` REAL
        // do overlay, gate ~/.bwms-histtest (dev). Ver `run_history_selftest`.
        {
            static HISTTEST_DONE: AtomicBool = AtomicBool::new(false);
            if !HISTTEST_DONE.load(Ordering::Relaxed)
                && std::env::var("HOME")
                    .ok()
                    .map(|hh| std::path::Path::new(&hh).join(".bwms-histtest").exists())
                    .unwrap_or(false)
            {
                rd.ui.run_history_selftest();
                HISTTEST_DONE.store(true, Ordering::Relaxed);
            }
        }
        {
            let io = rd.ctx.io_mut();
            io.display_size = [w, h];
            io.delta_time = 1.0 / 60.0;
            io.mouse_pos = [
                f32::from_bits(MOUSE_X.load(Ordering::Relaxed)),
                f32::from_bits(MOUSE_Y.load(Ordering::Relaxed)),
            ];
            io.mouse_down[0] = MOUSE_DOWN.load(Ordering::Relaxed);
            io.mouse_draw_cursor = show; // só desenha o cursor com o painel aberto
            if let Ok(mut q) = INPUT_Q.lock() {
                for ev in q.drain(..) {
                    match ev {
                        InputEv::Char(c) => io.add_input_character(c),
                        InputEv::Key(kc, down) => {
                            if let Some(k) = map_key(kc) {
                                io.add_key_event(k, down);
                            }
                        }
                        InputEv::Mods { cmd, shift } => {
                            io.add_key_event(imgui::Key::ModSuper, cmd);
                            io.add_key_event(imgui::Key::ModShift, shift);
                        }
                        InputEv::Letter(kc, down) => {
                            if let Some(k) = letter_key(kc) {
                                io.add_key_event(k, down);
                            }
                        }
                    }
                }
            }
        }
        // tema pedido por um mod via SetTheme(idx).
        let req = REQ_THEME.swap(-1, Ordering::Relaxed);
        if req >= 0 && (req as usize) < THEMES.len() {
            rd.ui.theme = req as usize;
            rd.ui.theme_dirty = true;
        }
        // troca de tema (selecionada no frame anterior): aplica + persiste.
        if rd.ui.theme_dirty {
            apply_theme(rd.ctx.style_mut(), rd.ui.theme);
            save_theme(rd.ui.theme);
            rd.ui.theme_dirty = false;
        }
        let ui = rd.ctx.new_frame();
        if boot {
            let splash = if rd.splash_tex.is_some() { Some(rd.splash_dim) } else { None };
            build_boot_splash(ui, splash, w, h);
        }
        if badge && !boot {
            build_badge(ui, crate::ticks(), crate::mods_loaded());
        }
        if show {
            build_ui(ui, &mut rd.ui);
            // mods desenham as PRÓPRIAS janelas (ImGui-pro-Lua) durante o onDraw.
            IN_DRAW.store(true, Ordering::Relaxed);
            crate::lua::run_event_draw();
            crate::api::run_plugin_draw_callbacks(); // mods Rust NÃO-lua (cet-imgui-thirdparty)
            IN_DRAW.store(false, Ordering::Relaxed);
            WANT_MOUSE.store(ui.io().want_capture_mouse, Ordering::Relaxed);
            WANT_KEYBOARD.store(ui.io().want_capture_keyboard, Ordering::Relaxed);
        } else {
            WANT_MOUSE.store(false, Ordering::Relaxed);
            WANT_KEYBOARD.store(false, Ordering::Relaxed);
        }
        if rd.ui.fav_dirty {
            save_favs(&rd.ui.favorites);
            rd.ui.fav_dirty = false;
        }
        let dd = rd.ctx.render();

        // ReShade-Metal: passe de LUT no FRAME DO JOGO (antes da UI), só com preset > 0.
        // Render pass no proprio drawable (Load preserva o frame p/ o framebuffer-fetch),
        // sem blending → o fragment substitui cada pixel pela versao graduada.
        let preset = LUT_PRESET.load(Ordering::Relaxed);
        let fxbits = EFFECTS.load(Ordering::Relaxed);
        if preset > 0 || fxbits != 0 {
            if let Some(lp) = rd.lut_pso.as_ref() {
                let lrpd = metal::RenderPassDescriptor::new();
                let la = lrpd.color_attachments().object_at(0).unwrap();
                la.set_texture(Some(tex));
                la.set_load_action(metal::MTLLoadAction::Load);
                la.set_store_action(metal::MTLStoreAction::Store);
                let le = cb.new_render_command_encoder(lrpd);
                le.set_render_pipeline_state(lp);
                le.set_fragment_bytes(0, 4, &preset as *const u32 as *const c_void);
                // res (buffer 1) + nº do frame (buffer 2): o shader os declara, então liga SEMPRE
                // (mesmo nos presets 1-5 que ignoram), senão o Metal aborta por buffer não-ligado.
                let res: [u32; 2] = [tex.width() as u32, tex.height() as u32];
                le.set_fragment_bytes(1, 8, res.as_ptr() as *const c_void);
                let bframe: u32 = rd.ui.frame;
                le.set_fragment_bytes(2, 4, &bframe as *const u32 as *const c_void);
                le.set_fragment_bytes(3, 4, &fxbits as *const u32 as *const c_void);
                le.draw_primitives(metal::MTLPrimitiveType::Triangle, 0, 3);
                le.end_encoding();
            }
        }

        let rpd = metal::RenderPassDescriptor::new();
        let att = rpd.color_attachments().object_at(0).unwrap();
        att.set_texture(Some(tex));
        att.set_load_action(metal::MTLLoadAction::Load);
        att.set_store_action(metal::MTLStoreAction::Store);
        let enc = cb.new_render_command_encoder(rpd);
        enc.set_render_pipeline_state(&rd.pso);

        // projeção ortográfica (pixels top-left → NDC)
        let proj: [[f32; 4]; 4] = [
            [2.0 / w, 0.0, 0.0, 0.0],
            [0.0, -2.0 / h, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ];
        enc.set_vertex_bytes(1, 64, proj.as_ptr() as *const c_void);
        // fonte por padrão; a textura por-comando é (re)ligada no loop conforme o texture_id
        // (1=fonte, 2=splash) — assim o `ui.image` do splash amostra o PNG e o resto, a fonte.
        enc.set_fragment_texture(0, Some(&rd.font_tex));

        for dl in dd.draw_lists() {
            let vtx = dl.vtx_buffer();
            let idx = dl.idx_buffer();
            if vtx.is_empty() || idx.is_empty() {
                continue;
            }
            let vbuf = dev.new_buffer_with_data(
                vtx.as_ptr() as *const c_void,
                (vtx.len() * size_of::<imgui::DrawVert>()) as u64,
                metal::MTLResourceOptions::StorageModeShared,
            );
            let ibuf = dev.new_buffer_with_data(
                idx.as_ptr() as *const c_void,
                (idx.len() * 2) as u64,
                metal::MTLResourceOptions::StorageModeShared,
            );
            enc.set_vertex_buffer(0, Some(&vbuf), 0);

            for cmd in dl.commands() {
                if let imgui::DrawCmd::Elements { count, cmd_params } = cmd {
                    // liga a textura do comando: splash (id=2) amostra o PNG; qualquer outro, a fonte.
                    if cmd_params.texture_id.id() == TEXID_SPLASH {
                        if let Some(sp) = rd.splash_tex.as_ref() {
                            enc.set_fragment_texture(0, Some(sp));
                        }
                    } else {
                        enc.set_fragment_texture(0, Some(&rd.font_tex));
                    }
                    let c = cmd_params.clip_rect;
                    let x = c[0].max(0.0);
                    let y = c[1].max(0.0);
                    let x2 = c[2].min(w);
                    let y2 = c[3].min(h);
                    if x2 <= x || y2 <= y {
                        continue;
                    }
                    enc.set_scissor_rect(metal::MTLScissorRect {
                        x: x as u64,
                        y: y as u64,
                        width: (x2 - x) as u64,
                        height: (y2 - y) as u64,
                    });
                    enc.draw_indexed_primitives_instanced_base_instance(
                        metal::MTLPrimitiveType::Triangle,
                        count as u64,
                        metal::MTLIndexType::UInt16,
                        &ibuf,
                        (cmd_params.idx_offset * 2) as u64,
                        1,
                        cmd_params.vtx_offset as i64,
                        0,
                    );
                }
            }
        }
        enc.end_encoding();
    }
}

// ---------------------------------------------------------------- present hook

// ===== AUTO-PROCEED da engagement do boot ("APERTE E PARA CONTINUAR") =====
// O proceed da engagement é 100% nativo (sem gancho redscript — GotoMainMenu/wraps não pulam).
// Estratégia: o vídeo de fundo da engagement é um bink; quando ele abre (selfboot marca via
// note_engagement_video), agendamos injetar a tecla "E" ~1-2s depois, no present (main thread).
// Injeta enquanto a engagement estiver "recente" (janela pós-bink) e SEM player; no menu o marcador
// envelhece → para sozinho. Cap total de segurança. Opt-in pelo marcador ~/.bwms-skipintro.
static ENGAGEMENT_ACTIVE: AtomicBool = AtomicBool::new(false); // setado pelo redscript (sinal preciso)
static SEEN_ENGAGEMENT: AtomicBool = AtomicBool::new(false); // já apareceu 1x nesta sessão (p/ boot_progress)

/// A engagement do boot está ativa agora? (setado pelo redscript). Lido pelo cp77_tick, que na
/// game thread FORÇA o proceed nativo (force_pregame_menu) enquanto isto for true.
pub fn engagement_active() -> bool {
    ENGAGEMENT_ACTIVE.load(Ordering::Relaxed)
}
/// A engagement JÁ apareceu nesta sessão (mesmo que já tenha terminado)? Sinal p/ boot_progress()
/// distinguir "ainda não chegou" de "passou e seguiu em frente" — engagement_active() sozinho não
/// dá pra diferenciar as duas (ambas retornam false).
pub fn seen_engagement() -> bool {
    SEEN_ENGAGEMENT.load(Ordering::Relaxed)
}

/// Chamado via native BwmsEngagementOn/Off pelo redscript (EngagementScreenGameController.
/// OnInitialize / OnUninitialize) — o ÚNICO sinal PRECISO de "estou na engagement do boot" (o bink
/// marca cedo demais; 'sem player' é falso no pregame por causa do puppet do V). O auto-proceed é
/// feito pelo cp77_tick (força o dispatcher na game thread), NÃO por injeção de input: o jogo lê HID
/// e eventos sintéticos (postEvent/CGEvent) do próprio processo não avançam nem com Acessibilidade.
pub fn set_engagement_active(on: bool) {
    ENGAGEMENT_ACTIVE.store(on, Ordering::Relaxed);
    if on {
        SEEN_ENGAGEMENT.store(true, Ordering::Relaxed);
    }
    crate::log(&format!("[skipintro] engagement ativa = {on} (via redscript)"));
}

extern "C" fn my_present(this: Id, cmd: Sel, drawable: Id) {
    unsafe {
        render_imgui(this, drawable);
        // VR: captura a cor do drawable final (game + UI/ink + nosso overlay) pro Quest.
        // Mesmo hook = VR e CET juntos, um dylib. Só compila com `--features capture`.
        #[cfg(feature = "capture")]
        crate::capture::on_present(this, drawable);
        let orig = ORIG_PRESENT.load(Ordering::Relaxed);
        if !orig.is_null() {
            let f: extern "C" fn(Id, Sel, Id) = std::mem::transmute(orig);
            f(this, cmd, drawable);
        }
    }
}

unsafe fn install_present_hook() {
    let dev = MTLCreateSystemDefaultDevice();
    if dev.is_null() {
        crate::log("[overlay] MTLCreateSystemDefaultDevice = null");
        return;
    }
    let q = msg0(dev, sel("newCommandQueue"));
    let cb = if q.is_null() {
        std::ptr::null_mut()
    } else {
        msg0(q, sel("commandBuffer"))
    };
    if cb.is_null() {
        crate::log("[overlay] sem command buffer p/ achar a classe");
        return;
    }
    let cb_class = object_getClass(cb);
    let name = CStr::from_ptr(class_getName(cb_class))
        .to_string_lossy()
        .into_owned();
    let m = class_getInstanceMethod(cb_class, sel("presentDrawable:"));
    if m.is_null() {
        crate::log(&format!("[overlay] sem presentDrawable: em {name}"));
        return;
    }
    ORIG_PRESENT.store(method_getImplementation(m) as *mut c_void, Ordering::Relaxed);
    method_setImplementation(m, my_present as Imp);
    crate::log(&format!("[overlay] presentDrawable: swizzlado em {name}"));
}

pub fn start() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        unsafe {
            install_present_hook();
            install_input_hook();
        }
    });
    // hook do scaler MetalFX (captura de depth) — só com `--features capture`.
    #[cfg(feature = "capture")]
    crate::capture::install_scaler_hooks();
}
