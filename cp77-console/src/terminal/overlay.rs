//! overlay.rs — janela in-game desenhada por cima do frame do jogo.
//!
//! Swizzla `-[<AGXFamilyCommandBuffer> presentDrawable:]` pra desenhar o terminal no `drawable.texture` (render pass
//! loadAction=Load preserva o jogo) e `-[NSApplication sendEvent:]` pro teclado e o mouse (toggle por `). O terminal é montado
//! pelos widgets do projeto (`terminal::imediato`) numa malha só, sobre o atlas do projeto (`terminal::malha_de_desenho`).
//! Botão clicado → enfileira o comando na fila em memória que o tick do jogo drena (`crate::enfileirar_comando`).
//!
//! panic=abort: o hook roda na thread de render; sem unwrap em caminho quente.

#![allow(dead_code)]

use std::ffi::{c_void, CStr, CString};
use std::mem::size_of;
use std::os::raw::c_char;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};

use crate::terminal::edicao::{tecla, AreaDeTransferencia, Evento, Mods};
use crate::terminal::imediato::{Entrada, Frame, Persistente, Tela, TECLA_BAIXO, TECLA_CIMA};
#[cfg(any(test, feature = "devtools"))]
use crate::terminal::imediato::{Alvo, ItemAchado, TipoDeItem};
use crate::terminal::malha_de_desenho::{montar_atlas, Vertice, ATLAS_A, ATLAS_L};

use crate::macos::{
    classe, class_getInstanceMethod, class_getName, method_getImplementation, method_setImplementation, msg, msg0, msg_usize, objc_getClass,
    objc_msgSend, object_getClass, sel, Class, Id, Imp, Retido, Sel,
};

extern "C" {
    fn MTLCreateSystemDefaultDevice() -> Id;
}
// CGAssociate = controle de cursor do overlay (re-acopla o mouse ao cursor; o jogo desacopla p/ a
// câmera em gameplay). Bloco PRÓPRIO com o link do CoreGraphics — mantém o framework linkado mesmo
// quando a automação de teclado (cg_press, feature "autoproceed") sai do build público.
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
}

// Decodificação da arte do splash pelo ImageIO do sistema, no lugar do crate `png` (que levava o
// `fdeflate` junto: 47 KB no dylib público pra ler UM arquivo opcional). O ImageIO já está no
// processo — Metal e AppKit o carregam — e aceita PNG, JPEG, HEIC e TIFF, então a arte pode vir no
// formato que o usuário tiver na mão.
#[link(name = "ImageIO", kind = "framework")]
extern "C" {
    fn CGImageSourceCreateWithURL(url: *const c_void, opcoes: *const c_void) -> *mut c_void;
    fn CGImageSourceCreateImageAtIndex(fonte: *mut c_void, indice: usize, opcoes: *const c_void) -> *mut c_void;
}
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGImageGetWidth(img: *mut c_void) -> usize;
    fn CGImageGetHeight(img: *mut c_void) -> usize;
    fn CGImageRelease(img: *mut c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
    fn CGColorSpaceRelease(cs: *mut c_void);
    fn CGBitmapContextCreate(
        dados: *mut c_void,
        largura: usize,
        altura: usize,
        bits_por_componente: usize,
        bytes_por_linha: usize,
        espaco: *mut c_void,
        info: u32,
    ) -> *mut c_void;
    fn CGContextDrawImage(ctx: *mut c_void, retangulo: CGRect, img: *mut c_void);
    fn CGContextRelease(ctx: *mut c_void);
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFURLCreateFromFileSystemRepresentation(
        alocador: *const c_void,
        caminho: *const u8,
        tamanho: isize,
        e_diretorio: bool,
    ) -> *mut c_void;
    fn CFRelease(cf: *mut c_void);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    largura: f64,
    altura: f64,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    origem: CGPoint,
    tamanho: CGSize,
}

/// `kCGImageAlphaPremultipliedLast` — é o único layout RGBA de 8 bits que o `CGBitmapContext`
/// aceita (as variantes não-premultiplicadas ele recusa), daí a volta em [`desfazer_premultiplicado`].
const ALPHA_PREMULTIPLICADO_NO_FIM: u32 = 1;

/// Decodifica uma imagem do disco em RGBA8 puro (não-premultiplicado, como o shader do overlay
/// espera). `None` = arquivo ausente, formato que o sistema não conhece ou dimensão absurda.
unsafe fn decodificar_imagem(caminho: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = caminho.as_os_str().as_encoded_bytes();
    let url = CFURLCreateFromFileSystemRepresentation(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize, false);
    if url.is_null() {
        return None;
    }
    let fonte = CGImageSourceCreateWithURL(url as *const c_void, std::ptr::null());
    CFRelease(url);
    if fonte.is_null() {
        return None;
    }
    let img = CGImageSourceCreateImageAtIndex(fonte, 0, std::ptr::null());
    CFRelease(fonte);
    if img.is_null() {
        return None;
    }
    let (l, a) = (CGImageGetWidth(img), CGImageGetHeight(img));
    // Teto de 8192: acima disso é arquivo errado, não arte de splash — e o buffer seria de 268 MB.
    if l == 0 || a == 0 || l > 8192 || a > 8192 {
        CGImageRelease(img);
        return None;
    }
    let mut rgba = vec![0u8; l * a * 4];
    let espaco = CGColorSpaceCreateDeviceRGB();
    let ctx = CGBitmapContextCreate(
        rgba.as_mut_ptr() as *mut c_void,
        l,
        a,
        8,
        l * 4,
        espaco,
        ALPHA_PREMULTIPLICADO_NO_FIM,
    );
    CGColorSpaceRelease(espaco);
    if ctx.is_null() {
        CGImageRelease(img);
        return None;
    }
    CGContextDrawImage(
        ctx,
        CGRect { origem: CGPoint { x: 0.0, y: 0.0 }, tamanho: CGSize { largura: l as f64, altura: a as f64 } },
        img,
    );
    CGContextRelease(ctx);
    CGImageRelease(img);
    desfazer_premultiplicado(&mut rgba);
    Some((rgba, l as u32, a as u32))
}

/// O `CGBitmapContext` só entrega RGBA premultiplicado; o shader amostra a cor crua e faz o blend
/// com `src_alpha`, então cor premultiplicada escurece as bordas translúcidas da arte.
fn desfazer_premultiplicado(rgba: &mut [u8]) {
    for p in rgba.chunks_exact_mut(4) {
        let alfa = p[3] as u32;
        if alfa == 0 || alfa == 255 {
            continue;
        }
        for c in &mut p[..3] {
            *c = ((*c as u32 * 255 + alfa / 2) / alfa).min(255) as u8;
        }
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
            f(cls as Id, sel!("stringWithUTF8String:"), c.as_ptr())
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Desabilita o App Nap do processo do jogo: `[[NSProcessInfo processInfo] beginActivityWithOptions:
/// (UserInitiated|LatencyCritical) reason:...]`. Sem isso o macOS PAUSA o Cyberpunk quando ele não
/// está na frente (janela ocluída) — no modo CPVR (olhando o capacete) o jogo vai pro fundo e congela:
/// boot de "30min", engagement travada, câmera com "coices". O token é
/// RETIDO pela sessão toda (App Nap fica OFF enquanto o jogo vive). Chamado 1x no on_load.
pub unsafe fn disable_app_nap() {
    let pi_class = class("NSProcessInfo");
    if pi_class.is_null() {
        return;
    }
    let pi: Id = msg0(pi_class as Id, sel!("processInfo"));
    if pi.is_null() {
        return;
    }
    // NSActivityUserInitiated (0x00FFFFFF) | NSActivityLatencyCritical (0xFF00000000)
    const OPTS: u64 = 0x00FF_FFFF | 0xFF_0000_0000;
    let reason = nsstring("sem App Nap: carga e render seguem a full em segundo plano");
    let f: extern "C" fn(Id, Sel, u64, Id) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let token = f(pi, sel!("beginActivityWithOptions:reason:"), OPTS, reason);
    if !token.is_null() {
        // retém o token pela sessão (senão o autorelease o libera e o App Nap volta)
        let r: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        r(token, sel!("retain"));
        crate::log_dev!("[appnap] App Nap DESABILITADO (o jogo roda a full em background)");
    } else {
        crate::log_dev!("[appnap] beginActivityWithOptions retornou null (App Nap NÃO desabilitado)");
    }
}

/// Ponte do clipboard do sistema (NSPasteboard), extraída em funções standalone (`clipboard_get`/
/// `clipboard_set`) — reusadas por 2 consumidores: (1) `AreaDoSistema` (abaixo, campo de texto do
/// terminal — Cmd+C/Cmd+X/Cmd+V no console, colar comandos longos); (2) `register.rs::tramp_ink_*`
/// (`inkSystem.GetClipboardText`/`SetClipboardText` — via a ponte NSPasteboard
/// que este projeto já tinha, provada em produção pelo console há sessões — zero código novo de
/// baixo nível, só reuso). `pub(crate)` pra ser visível de `register.rs` sem duplicar a ponte ObjC.
pub(crate) unsafe fn clipboard_get() -> Option<String> {
    let cls = class("NSPasteboard");
    if cls.is_null() {
        return None;
    }
    let pb = msg0(cls as Id, sel!("generalPasteboard"));
    if pb.is_null() {
        return None;
    }
    let ty = nsstring("public.utf8-plain-text");
    let s = msg1(pb, sel!("stringForType:"), ty);
    if s.is_null() {
        return None;
    }
    let p = msg_cstr(s, sel!("UTF8String"));
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok().map(|x| x.to_string())
}
pub(crate) unsafe fn clipboard_set(value: &str) {
    let cls = class("NSPasteboard");
    if cls.is_null() {
        return;
    }
    let pb = msg0(cls as Id, sel!("generalPasteboard"));
    if pb.is_null() {
        return;
    }
    let _ = msg_usize(pb, sel!("clearContents")); // NSInteger; ignora
    let ns = nsstring(value);
    let ty = nsstring("public.utf8-plain-text");
    if ns.is_null() || ty.is_null() {
        return;
    }
    let f: extern "C" fn(Id, Sel, Id, Id) -> bool = std::mem::transmute(objc_msgSend as *const c_void);
    f(pb, sel!("setString:forType:"), ns, ty);
}

/// Quantos tipos o clipboard do sistema tem agora (0 = vazio de verdade; texto, imagem etc. contam).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clipboard_tipos() -> Option<usize> {
    let cls = class("NSPasteboard");
    if cls.is_null() {
        return None;
    }
    let pb = msg0(cls as Id, sel!("generalPasteboard"));
    if pb.is_null() {
        return None;
    }
    let tipos = msg0(pb, sel!("types"));
    if tipos.is_null() {
        return Some(0);
    }
    Some(msg_usize(tipos, sel!("count")) as usize)
}

/// Esvazia o clipboard do sistema (devolve o estado "vazio" guardado por um teste).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn clipboard_limpar() {
    let cls = class("NSPasteboard");
    if cls.is_null() {
        return;
    }
    let pb = msg0(cls as Id, sel!("generalPasteboard"));
    if !pb.is_null() {
        let _ = msg_usize(pb, sel!("clearContents"));
    }
}

/// A área de transferência do sistema no Cmd+C/X/V do campo de texto do terminal.
struct AreaDoSistema;
impl AreaDeTransferencia for AreaDoSistema {
    fn ler(&mut self) -> Option<String> {
        unsafe { clipboard_get() }
    }
    fn gravar(&mut self, texto: &str) {
        unsafe { clipboard_set(texto) }
    }
}

/// Teclado da main thread (`sendEvent`), na ordem em que chegou: vira a `Entrada` do próximo quadro do terminal.
static INPUT_Q: std::sync::Mutex<Vec<Evento>> = std::sync::Mutex::new(Vec::new());

fn push_input(ev: Evento) {
    if let Ok(mut q) = INPUT_Q.lock() {
        if q.len() < 256 {
            q.push(ev);
        }
    }
}

/// keyCode do macOS das teclas que vão ao terminal como TECLA, não como caractere: as da edição (`terminal::edicao::tecla`) e o ↑/↓
/// do histórico. O AppKit manda setas, Delete, Home e End também como caractere da área privada, que a edição recusa.
const TECLAS_DO_TERMINAL: [u16; 12] = [
    tecla::BACKSPACE,
    tecla::DELETE,
    tecla::ENTER,
    tecla::ENTER_NUMERICO,
    tecla::ESC,
    tecla::TAB,
    tecla::ESQUERDA,
    tecla::DIREITA,
    TECLA_BAIXO,
    TECLA_CIMA,
    tecla::HOME,
    tecla::END,
];
/// Letras dos atalhos de edição com Cmd (selecionar tudo, recortar, copiar, colar), pelo keyCode do hardware: valem em qualquer layout.
const LETRAS_DE_ATALHO: [u16; 4] = [tecla::A, tecla::X, tecla::C, tecla::V];

static ORIG_PRESENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIG_SENDEVENT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// CoreGraphics: injeção de tecla em nível HID (o CP2077 lê input via IOKit HID, NÃO via NSApp
// sendEvent — por isso postEvent NSEvent não avança a engagement, mas CGEvent HID sim, igual ao
// input real do teclado). CGEventPost pode exigir Acessibilidade (TCC) pro processo do jogo.
#[cfg(feature = "autoproceed")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventCreateKeyboardEvent(source: *mut c_void, keycode: u16, keydown: bool) -> *mut c_void;
    fn CGEventPostToPid(pid: i32, event: *mut c_void);
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
    let shared = msg0(app_cls as Id, sel!("sharedApplication"));
    if shared.is_null() {
        return true;
    }
    (msg0(shared, sel!("isActive")) as usize) & 0xff != 0
}

/// Força a janela do jogo pra FRENTE (`[NSApp activateIgnoringOtherApps:YES]`). A engagement do boot
/// só aceita o SPACE quando o jogo está em foco (lê evento HID global, não o CGEventPostToPid); no
/// CPVR o usuário olha o capacete e o jogo fica atrás. Chamado só pelo comando `focusgame` (`lib.rs`).
#[cfg(feature = "devtools")]
pub unsafe fn force_game_frontmost() {
    let app_cls = class("NSApplication");
    if app_cls.is_null() {
        return;
    }
    let shared = msg0(app_cls as Id, sel!("sharedApplication"));
    if shared.is_null() {
        return;
    }
    // DIAGNÓSTICO 2026-08-20 (madrugada, investigação "jogo nunca ganha foco de janela" — ver
    // memória bwms-t82s-crash-vs-887pct-hang Atualização 8): `activateIgnoringOtherApps:` sozinho
    // parou de funcionar nesta sessão (Finder sempre fica frontmost, confirmado por 3 métodos
    // independentes). Hipótese testada aqui: a política de ativação do app pode não estar
    // Regular (0) — setando explicitamente ANTES de ativar, e também tentando a via
    // NSRunningApplication (API mais nova, pode ter semântica diferente de ativação forçada).
    let policy_sel = sel!("setActivationPolicy:");
    let fp: extern "C" fn(Id, Sel, i64) -> bool = std::mem::transmute(objc_msgSend as *const c_void);
    let ok = fp(shared, policy_sel, 0); // NSApplicationActivationPolicyRegular
    crate::log_dev!("[focusgame] setActivationPolicy(Regular) -> {ok}");

    let f: extern "C" fn(Id, Sel, bool) = std::mem::transmute(objc_msgSend as *const c_void);
    f(shared, sel!("activateIgnoringOtherApps:"), true);

    // Via alternativa: NSRunningApplication.currentApplication.activateWithOptions: com as 2
    // flags mais fortes (ActivateAllWindows=1<<0 | ActivateIgnoringOtherApps=1<<1 = 3).
    let ra_cls = class("NSRunningApplication");
    if !ra_cls.is_null() {
        let cur = msg0(ra_cls as Id, sel!("currentApplication"));
        if !cur.is_null() {
            let fa: extern "C" fn(Id, Sel, u64) -> bool = std::mem::transmute(objc_msgSend as *const c_void);
            let ok2 = fa(cur, sel!("activateWithOptions:"), 3);
            crate::log_dev!("[focusgame] NSRunningApplication.activateWithOptions(3) -> {ok2}");
        }
    }
}

/// Injeta keyDown+keyUp da tecla só no processo do jogo (`CGEventPostToPid`). O post global (`CGEventPost` no tap HID) vai pro app da
/// FRENTE, e a checagem de foco antes dele não fecha a janela entre checar e entregar: com o usuário em outro programa, a tecla caía lá
/// (relatos de 2026-07 e 2026-09-28). Tecla que o jogo só aceita pelo HID global não é dada por aqui. Thread-safe.
#[cfg(feature = "autoproceed")]
pub unsafe fn cg_press(keycode: u16) {
    cg_key_event(keycode, true);
    cg_key_event(keycode, false);
}

/// keyDown OU keyUp isolado (não os dois em sequência) — permite ao CALLER (ex. o bridge de
/// visão por IA, processo EXTERNO no Mac) controlar a duração do "hold" via 2 comandos de canal
/// separados + sleep DELE MESMO, sem nunca bloquear a thread do jogo com um sleep interno.
/// `automacao-mundo` (2026-08-17): ponte IA-LAN precisa de movimento sustentado (WASD), não só
/// tap — mesma receita/permissão de `cg_press` (CGEventPost de dentro do processo do jogo).
#[cfg(feature = "autoproceed")]
pub unsafe fn cg_key_event(keycode: u16, down: bool) {
    let pid = getpid();
    let src = CGEventSourceCreate(1); // kCGEventSourceStateHIDSystemState
    let ev = CGEventCreateKeyboardEvent(src, keycode, down);
    if !ev.is_null() {
        CGEventPostToPid(pid, ev);
        CFRelease(ev);
    }
    if !src.is_null() {
        CFRelease(src);
    }
}

/// Mod real ponta a ponta/prova visual do link (2026-07-19): navegação de menu nativo por MOUSE.
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
    crate::log_dev!(
        "[mousedelta-diag] frontmost={frontmost} probe_ok={probe_ok} loc=({},{}) newloc=({},{}) src={src:p} mv={mv:p}",
        loc.x, loc.y, newloc.x, newloc.y
    );
    if !mv.is_null() {
        CGEventSetIntegerValueField(mv, 4 /* kCGMouseEventDeltaX */, dx);
        CGEventSetIntegerValueField(mv, 5 /* kCGMouseEventDeltaY */, dy);
        let rx = CGEventGetIntegerValueField(mv, 4);
        let ry = CGEventGetIntegerValueField(mv, 5);
        crate::log_dev!("[mousedelta-diag] readback dx={rx} dy={ry} (esperado {dx}/{dy})");
        CGEventPostToPid(getpid(), mv);
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
        CFRelease(down);
    }
    if !up.is_null() {
        CGEventPostToPid(getpid(), up);
        CFRelease(up);
    }
    if !src.is_null() {
        CFRelease(src);
    }
}

static LOGGED: AtomicBool = AtomicBool::new(false);
static SHOW: AtomicBool = AtomicBool::new(false); // começa ESCONDIDO (` abre)
static PREV_SHOW: AtomicBool = AtomicBool::new(false); // p/ detectar a borda fechar→devolver o mouse
/// Eventos de ciclo de vida: bordas abrir/fechar do overlay, sinalizadas AQUI (thread do render, via
/// presentDrawable) mas consumidas+disparadas (`fire_event`) de dentro do `cp77_tick` (thread do
/// jogo) — chamar a VM/redscript fora da thread do jogo é arriscado (lição desta sessão: 3ptest/
/// fbtest/etc. só chamam call_func do cp77_tick). Ver o consumo em `lib.rs`.
pub(crate) static OVERLAY_OPEN_EDGE: AtomicBool = AtomicBool::new(false);
pub(crate) static OVERLAY_CLOSE_EDGE: AtomicBool = AtomicBool::new(false);
// Mouse (preenchido pelo sendEvent na main thread, lido pelo render). f32 em bits.
static MOUSE_X: AtomicU32 = AtomicU32::new(0);
static MOUSE_Y: AtomicU32 = AtomicU32::new(0);
static MOUSE_DOWN: AtomicBool = AtomicBool::new(false);
static FRAME_W: AtomicU32 = AtomicU32::new(0);
static FRAME_H: AtomicU32 = AtomicU32::new(0);
static SCALE: AtomicU32 = AtomicU32::new(0x4000_0000); // 2.0f32 (retina) por padrão
static ESCALA_DA_JANELA: AtomicBool = AtomicBool::new(false);
static WANT_KEYBOARD: AtomicBool = AtomicBool::new(false); // edição em curso num campo? (`Frame::quer_teclado`)

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
            if t == 10 && msg_usize(event, sel!("isARepeat")) != 0 {
                return;
            }
            let kc = msg_u16(event, sel!("keyCode"));
            let mut ch = String::new();
            let s0 = msg0(event, sel!("charactersIgnoringModifiers"));
            if !s0.is_null() {
                let p = msg_cstr(s0, sel!("UTF8String"));
                if !p.is_null() {
                    if let Ok(st) = CStr::from_ptr(p).to_str() {
                        ch = st.to_string();
                    }
                }
            }
            format!("{} kc={kc} {ch:?}", if t == 10 { "kd" } else { "ku" })
        }
        12 => format!("mod {:#x}", msg_usize(event, sel!("modifierFlags")) & 0xffff_0000),
        1 | 2 => format!("{} L", if t == 1 { "mdn" } else { "mup" }),
        3 | 4 => format!("{} R", if t == 3 { "mdn" } else { "mup" }),
        25 | 26 => format!("{} O", if t == 25 { "mdn" } else { "mup" }),
        22 => format!("scr dy={:.1}", msg_f64(event, sel!("scrollingDeltaY"))),
        5 | 6 | 7 => {
            // mouseMoved/dragged disparam MUITO → throttle ~50ms (≤20 amostras/s)
            let n = now_ms();
            if n.wrapping_sub(LAST_MOVE_MS.load(Ordering::Relaxed)) < 50 {
                return;
            }
            LAST_MOVE_MS.store(n, Ordering::Relaxed);
            let p = msg_point(event, sel!("locationInWindow"));
            format!("mov {:.0},{:.0}", p.x, p.y)
        }
        _ => return,
    };
    input_file_log(&ev);
}

/// Força o estado aberto/fechado do overlay (teste dos eventos de ciclo de vida via canal, sem HID —
/// dispara a MESMA borda que o toggle real do backtick dispara em desenhar_terminal).
pub fn set_shown(v: bool) {
    SHOW.store(v, Ordering::Relaxed);
}

/// O overlay (console) está aberto? (cp77_tick usa p/ disparar onOverlayOpen/Close.)
pub fn is_shown() -> bool {
    SHOW.load(Ordering::Relaxed)
}

/// Entrega um comando ao tick do jogo, pela fila em memória.
fn write_cmd(c: &str) {
    if !crate::enfileirar_comando(c) {
        crate::resposta!("A fila de comandos está cheia (16). Espere os anteriores rodarem e mande de novo.");
    }
}

// ---------------------------------------------------------------- input (toggle)

extern "C" fn my_sendevent(this: Id, cmd: Sel, event: Id) {
    unsafe {
        let mut consume = false;
        if !event.is_null() {
            let t = msg_usize(event, sel!("type"));
            // dataset ML: grava o input cru (não consome, não altera o fluxo do jogo).
            if INPUT_LOG_ON.load(Ordering::Relaxed) {
                log_input_event(event, t);
            }
            match t {
                10 => {
                    let kc = msg_u16(event, sel!("keyCode"));
                    // char base da tecla (independe de layout) — acha a crase no ABNT2. Emprestado, sem cópia: o UTF8String vive no
                    // pool do AppKit até este sendEvent: voltar, e o `ch` não sai da função (o K-LOG leva a String do format!).
                    let mut ch: &str = "";
                    let s0 = msg0(event, sel!("charactersIgnoringModifiers"));
                    if !s0.is_null() {
                        let p = msg_cstr(s0, sel!("UTF8String"));
                        if !p.is_null() {
                            if let Ok(st) = CStr::from_ptr(p).to_str() {
                                ch = st;
                            }
                        }
                    }
                    // Atalho do terminal: o gravado pela aba Mods (`terminal::atalho`) ou, sem ele, crase/til (US 50, ISO 10, o caractere)
                    // ou F1 (122). Durante a gravação toda tecla vai para o atalho novo e não segue.
                    use crate::terminal::atalho::{self, Efeito};
                    let efeito = atalho::evento(10, kc, msg_usize(event, sel!("modifierFlags")) as u64);
                    let padrao = !atalho::tem_atalho() && (kc == 50 || kc == 10 || kc == 122 || ch == "`" || ch == "~");
                    if efeito == Efeito::Alterna || (efeito == Efeito::Segue && padrao) {
                        SHOW.store(!SHOW.load(Ordering::Relaxed), Ordering::Relaxed);
                        consume = true;
                    } else if efeito != Efeito::Segue {
                        consume = true;
                    } else if !SHOW.load(Ordering::Relaxed) {
                        // CallbackSystem RawInput (2026-07-18, input cru com nome real): mapeia o
                        // keycode+char pro valor REAL de EInputKey + lê os modificadores AppKit,
                        // enfileira TODOS pro evento "Input/Key" (drenado no cp77_tick, que
                        // constrói+despacha um `ref<KeyInputEvent>` REAL na thread do jogo).
                        let flags_raw = msg_usize(event, sel!("modifierFlags")) as u64;
                        let raw_shift = atalho::modificador_seguro(flags_raw, atalho::SHIFT);
                        let raw_control = atalho::modificador_seguro(flags_raw, atalho::CTRL);
                        let raw_alt = atalho::modificador_seguro(flags_raw, atalho::OPTION);
                        let mapped_key = crate::entrada::map_macos_keycode_to_einputkey(kc as i32, ch.chars().next());
                        crate::entrada::push_raw_key(mapped_key, raw_shift, raw_control, raw_alt);
                        #[cfg(feature = "lua")]
                        if let Some(c) = ch.chars().next() {
                            crate::mods_lua::tecla(c, true);
                        }
                        // escondido: captura a tecla SÓ pro buffer K-LOG (não pro console),
                        // e só se a captura estiver ligada (aba K-LOG). Mantém o console limpo.
                        if KEY_CAPTURE.load(Ordering::Relaxed) {
                            key_log_push(format!("[key] keyCode={kc} char={ch:?}"));
                        }
                    } else if SHOW.load(Ordering::Relaxed) {
                        // Terminal aberto: a tecla vai para a `Entrada` do próximo quadro, e só é ENGOLIDA do jogo com edição em
                        // curso num campo (senão WASD/Enter passam).
                        let flags = msg_usize(event, sel!("modifierFlags")) as u64;
                        let mods = Mods {
                            cmd: atalho::modificador_seguro(flags, atalho::CMD),
                            shift: atalho::modificador_seguro(flags, atalho::SHIFT),
                            option: atalho::modificador_seguro(flags, atalho::OPTION),
                        };
                        if mods.cmd && LETRAS_DE_ATALHO.contains(&kc) {
                            // Cmd+A/X/C/V → atalho de edição (selecionar/recortar/copiar/colar).
                            // NÃO empurra o char (senão inseria 'v' junto com o paste).
                            push_input(Evento::Tecla(kc, mods));
                            consume = true;
                        } else if TECLAS_DO_TERMINAL.contains(&kc) {
                            push_input(Evento::Tecla(kc, mods));
                        } else if !mods.cmd {
                            let s = msg0(event, sel!("characters"));
                            if !s.is_null() {
                                let p = msg_cstr(s, sel!("UTF8String"));
                                if !p.is_null() {
                                    if let Ok(st) = CStr::from_ptr(p).to_str() {
                                        for c in st.chars() {
                                            if c >= ' ' {
                                                push_input(Evento::Caractere(c));
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
                    // KeyUp. Durante a gravação do atalho a tecla solta pode terminar a gravação, e não segue.
                    let kc_solta = msg_u16(event, sel!("keyCode"));
                    let mods = msg_usize(event, sel!("modifierFlags")) as u64;
                    if crate::terminal::atalho::evento(11, kc_solta, mods) != crate::terminal::atalho::Efeito::Segue {
                        consume = true;
                    } else if SHOW.load(Ordering::Relaxed) {
                        // A soltura não muda a edição; com edição em curso ela também não vai ao jogo, que não viu a descida.
                        if WANT_KEYBOARD.load(Ordering::Relaxed) {
                            consume = true;
                        }
                    } else {
                        // overlay fechado: key-up de registerInput (cb recebe isDown=false).
                        // Só o 1º char: é tudo o que o mapeamento para EInputKey lê.
                        let s0 = msg0(event, sel!("charactersIgnoringModifiers"));
                        let mut ch_up: Option<char> = None;
                        if !s0.is_null() {
                            let p = msg_cstr(s0, sel!("UTF8String"));
                            if !p.is_null() {
                                if let Ok(st) = CStr::from_ptr(p).to_str() {
                                    ch_up = st.chars().next();
                                }
                            }
                        }
                        // Tecla solta (2026-08-11): metade que faltava do CallbackSystem
                        // RawInput — captura da tecla SOLTA, mesma técnica de mapeamento/
                        // enfileiramento do keydown (linha ~647 acima), só o destino é a fila
                        // IRMÃ (`push_raw_key_up`, drenada como "Input/KeyUp" no `cp77_tick`).
                        let kc_up = msg_u16(event, sel!("keyCode"));
                        use crate::terminal::atalho;
                        let flags_up = msg_usize(event, sel!("modifierFlags")) as u64;
                        let up_shift = atalho::modificador_seguro(flags_up, atalho::SHIFT);
                        let up_control = atalho::modificador_seguro(flags_up, atalho::CTRL);
                        let up_alt = atalho::modificador_seguro(flags_up, atalho::OPTION);
                        let mapped_key_up =
                            crate::entrada::map_macos_keycode_to_einputkey(kc_up as i32, ch_up);
                        crate::entrada::push_raw_key_up(mapped_key_up, up_shift, up_control, up_alt);
                        #[cfg(feature = "lua")]
                        if let Some(c) = ch_up {
                            crate::mods_lua::tecla(c, false);
                        }
                    }
                }
                12 => {
                    // Modificadores mudaram (FlagsChanged): só contam para o atalho (gravação ou atalho com Ctrl/Option/Shift/Cmd).
                    let mods = msg_usize(event, sel!("modifierFlags")) as u64;
                    if crate::terminal::atalho::evento(12, 0, mods) != crate::terminal::atalho::Efeito::Segue {
                        consume = true;
                    }
                }
                1 => MOUSE_DOWN.store(true, Ordering::Relaxed), // LeftMouseDown
                2 => MOUSE_DOWN.store(false, Ordering::Relaxed), // LeftMouseUp
                _ => {}
            }
            // posição do mouse (down/up/moved/dragged) → pixels top-left.
            if matches!(t, 1 | 2 | 5 | 6) {
                // Escala da tela (retina) p/ converter pontos→pixels do mouse: da janela do 1º evento que tiver uma (aqui é a main
                // thread). Evento sem janela deixa o 2.0 padrão e tenta de novo no próximo.
                if !ESCALA_DA_JANELA.load(Ordering::Relaxed) {
                    let janela = msg0(event, sel!("window"));
                    if !janela.is_null() {
                        let s = msg_f64(janela, sel!("backingScaleFactor")) as f32;
                        if s > 0.0 {
                            SCALE.store(s.to_bits(), Ordering::Relaxed);
                            ESCALA_DA_JANELA.store(true, Ordering::Relaxed);
                            crate::log_dev!("[overlay] backingScaleFactor={s} (janela do 1º evento de mouse)");
                        }
                    }
                }
                let p = msg_point(event, sel!("locationInWindow"));
                let scale = f32::from_bits(SCALE.load(Ordering::Relaxed)) as f64;
                let fh = f32::from_bits(FRAME_H.load(Ordering::Relaxed)) as f64;
                MOUSE_X.store(((p.x * scale) as f32).to_bits(), Ordering::Relaxed);
                MOUSE_Y.store(((fh - p.y * scale) as f32).to_bits(), Ordering::Relaxed);
                // overlay aberto: o jogo NÃO vê o mouse (sem câmera/clique no jogo).
                // O terminal desenha o próprio ponteiro, então congelar o do jogo é ok.
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

/// Só o swizzle, sem mensagem ao AppKit: esta função roda na thread de fundo do `start()`, e o `NSApp` fica não nulo logo depois
/// do `[super init]`, antes de o `-[NSApplication init]` abrir o contexto do app e o sistema de eventos (desmonte do AppKit).
/// A classe sai do objeto (`object_getClass`, runtime ObjC puro); a escala da tela, do 1º evento com janela no `my_sendevent`.
unsafe fn install_input_hook() {
    let app = nsapp_do_jogo();
    if app.is_null() {
        return;
    }
    let app_class = object_getClass(app);
    let m = class_getInstanceMethod(app_class, sel!("sendEvent:"));
    if m.is_null() {
        crate::log_dev!("[overlay] sem sendEvent:");
        return;
    }
    ORIG_SENDEVENT.store(method_getImplementation(m) as *mut c_void, Ordering::Relaxed);
    method_setImplementation(m, my_sendevent as Imp);
    crate::log_dev!("[overlay] sendEvent: swizzlado (` alterna o overlay)");
    // dataset ML: liga o input-logger DESDE O BOOT se o marcador existir — cobre menus, criação
    // de ficha e intro (onde o canal do console ainda não drena). `inputlog on/off` toggla em runtime.
    let marker = cfg!(feature = "devtools")
        && (crate::marcador_de_dev(".bwms-inputlog") || std::path::Path::new("/tmp/bwms-inputlog").exists());
    if marker {
        input_log_set(true);
        crate::log_dev!("[inputlog] ligado no boot (marcador ~/.bwms-inputlog) -> /tmp/cp77-input.log");
    }
}

// ---------------------------------------------------------------- render (terminal)

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
    /// pra ser testável (histórico do console). É EXATAMENTE a lógica que a tecla ↑/↓ aciona no overlay.
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

    /// Histórico do console — self-test AUTOMATIZÁVEL no menu (sem HID): exercita o `cmd_history` REAL
    /// do overlay com a MESMA lógica de recall que a tecla ↑/↓ aciona, + o comando `help`. Loga cada
    /// passo. Prova a paridade de console (↑ recupera o último comando no campo; `help` lista in-overlay)
    /// sem precisar digitar/apertar teclas. Restaura o estado no fim. Ver `desenhar_terminal` (gate marcador).
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
        crate::log_dev!(
            "[histtest] ↑1='{up1}' ↑2='{up2}' ↑3='{up3}' ↓1='{down1}' | hist_ok={hist_ok} | help: +{help_pushed} linhas help_ok={help_ok} | {verdict}"
        );

        // restaura
        self.log_lines.truncate(saved_log_len);
        self.cmd_history = saved_hist;
        self.cmd_history_idx = saved_idx;
        self.cmd_buf = saved_buf;
    }
}
struct Renderer {
    /// Amostragem NEAREST (`f_nearest`): a fonte de pixel do terminal sai nítida em 1× e em toda escala inteira (`terminal::escala`).
    pso: Retido,
    lut_pso: Option<Retido>,
    /// Nº do quadro que o grão do filtro de cor usa: anda a cada passe do filtro, com o terminal aberto ou fechado.
    quadro_do_filtro: u32,
    /// O atlas do projeto: texto, formas e ponteiro do terminal saem dele, num lote só.
    atlas: Retido,
    ui: UiState,
    /// Janelas (a do terminal e as dos plugins), foco, edição, arraste e rolagem: o que sobrevive ao quadro.
    persistente: Persistente,
    anterior: EntradaDoQuadroAnterior,
    #[cfg(feature = "devtools")]
    autoteste: Option<Autoteste>,
    /// Pedido de dev em atendimento (`termclique`, `termonde`), tirado da fila `PEDIDOS_DE_DEV`.
    #[cfg(feature = "devtools")]
    pedido: Option<PedidoDeDev>,
}

/// Clique duplo: o 2º aperto em até 0,30 s e 6 px do 1º, o tempo e a distância da referência do terminal; o 3º aperto conta de novo
/// como o 1º.
const CLIQUE_DUPLO_S: f32 = 0.30;
const CLIQUE_DUPLO_PX: f32 = 6.0;
/// Teto do `dt`: o 1º quadro depois de o terminal ficar fechado não dispara uma rajada de repetição do `+`/`−` nem pula o piscar.
const DT_MAXIMO: f32 = 0.1;

/// O que a entrada do quadro anterior deixou: o botão, o último aperto e o relógio.
#[derive(Default)]
struct EntradaDoQuadroAnterior {
    segurando: bool,
    /// Instante e posição do último aperto que não fechou um clique duplo.
    aperto: Option<(std::time::Instant, [f32; 2])>,
    /// Instante do quadro anterior: o `dt` do cursor piscando e da repetição do `+`/`−`.
    quadro: Option<std::time::Instant>,
}

impl EntradaDoQuadroAnterior {
    /// A `Entrada` do quadro: o mouse em pixels lógicos e o teclado na ordem em que chegou. Aperto e soltura saem da comparação com o
    /// quadro anterior, então um clique mais curto que um quadro se perde, como na referência do terminal.
    fn entrada(&mut self, mouse: [f32; 2], segurando: bool, eventos: Vec<Evento>, tela: [f32; 2], agora: std::time::Instant) -> Entrada {
        let dt = self.quadro.map_or(1.0 / 60.0, |q| agora.saturating_duration_since(q).as_secs_f32()).min(DT_MAXIMO);
        self.quadro = Some(agora);
        let apertou = segurando && !self.segurando;
        let soltou = !segurando && self.segurando;
        self.segurando = segurando;
        let clique_duplo = apertou
            && self.aperto.is_some_and(|(t, p)| {
                agora.saturating_duration_since(t).as_secs_f32() < CLIQUE_DUPLO_S && (mouse[0] - p[0]).hypot(mouse[1] - p[1]) <= CLIQUE_DUPLO_PX
            });
        if apertou {
            self.aperto = if clique_duplo { None } else { Some((agora, mouse)) };
        }
        let teclas = teclas_dos_eventos(&eventos);
        Entrada {
            mouse,
            segurando,
            apertou,
            soltou,
            // A roda não entra: com o terminal aberto ela segue para o jogo (troca de arma), a DD2 do D1.
            roda: 0.0,
            digitados: Vec::new(),
            teclas,
            eventos,
            clique_duplo,
            dt,
            tamanho_da_tela: tela,
            ponteiro: true,
            #[cfg(any(test, feature = "devtools"))]
            procura: None,
        }
    }
}

/// O mouse do quadro como o `sendEvent` o deixou, em pixels lógicos (quadro ÷ escala `n`), e o botão esquerdo.
fn mouse_do_sendevent(n: f32) -> ([f32; 2], bool) {
    let x = f32::from_bits(MOUSE_X.load(Ordering::Relaxed)) / n;
    let y = f32::from_bits(MOUSE_Y.load(Ordering::Relaxed)) / n;
    ([x, y], MOUSE_DOWN.load(Ordering::Relaxed))
}

/// As teclas apertadas no quadro, para o `Tela::tecla` (o ↑/↓ do histórico vale com o campo focado mesmo fora da edição).
fn teclas_dos_eventos(eventos: &[Evento]) -> Vec<u16> {
    eventos
        .iter()
        .filter_map(|e| match e {
            Evento::Tecla(k, _) => Some(*k),
            Evento::Caractere(_) => None,
        })
        .collect()
}

/// O terminal de um quadro, sem GPU: a tela com a entrada, o terminal, as janelas dos plugins e o fim (dica, ponteiro, foco). É o que
/// o render desenha e o que o autoteste e os testes conferem.
fn montar_quadro<'a>(ui: &mut UiState, p: &'a mut Persistente, e: Entrada, area: &'a mut dyn AreaDeTransferencia) -> Frame {
    let mut t = Tela::nova(e, p, ui.theme);
    t.com_area_de_transferencia(area);
    montar_terminal(&mut t, ui);
    crate::mods_do_usuario::api::desenhar_plugins(&mut t);
    #[cfg(feature = "lua")]
    crate::mods_lua::interface::desenhar(&mut t);
    t.terminar()
}

/// Os rótulos das abas do terminal, na ordem do `montar_terminal`: o autoteste mira por eles e os vereditos de dev dizem a aba ativa.
#[cfg(any(test, feature = "devtools"))]
const ABAS_DO_TERMINAL: [&str; 7] = ["Console", "Items", "Favoritos", "Mods", "K-LOG", "CPVR", "LUT"];

/// Centro da aba `i` do terminal na janela `[x, y, largura, altura]`, pelo layout do `montar_terminal`: a barra de abas fica abaixo da
/// barra de título (19), do respiro (8), da linha de temas (19 + 4) e do separador (0 + 4); cada aba mede texto × 7 + 9, com 4 entre
/// elas. Se o layout mudar, o autoteste falha no passo que errou o alvo.
#[cfg(any(test, feature = "devtools"))]
fn centro_da_aba_do_terminal(janela: [f32; 4], i: usize) -> [f32; 2] {
    let mut x = janela[0] + 8.0;
    for r in &ABAS_DO_TERMINAL[..i] {
        x += r.len() as f32 * 7.0 + 9.0 + 4.0;
    }
    let largura = ABAS_DO_TERMINAL[i].len() as f32 * 7.0 + 9.0;
    let y = janela[1] + 19.0 + 8.0 + 19.0 + 4.0 + 4.0;
    [x + largura * 0.5, y + 9.5]
}

/// Centro do campo de comando do Console: o log acaba 30 px acima do respiro de baixo (8), o campo (19) vem 4 abaixo dele.
#[cfg(any(test, feature = "devtools"))]
fn centro_do_campo_do_console(janela: [f32; 4]) -> [f32; 2] {
    let [x, y, l, a] = janela;
    [x + l * 0.5, y + a - 8.0 - 30.0 + 4.0 + 9.5]
}

/// Passos do autoteste da fiação do terminal, um por quadro, na ordem.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Copy, PartialEq, Debug)]
enum PassoDoAutoteste {
    ApertaItems,
    SoltaItems,
    ApertaConsole,
    SoltaConsole,
    ApertaCampo,
    SoltaCampo,
    Digita,
    Enter,
    Feito,
}

/// Autoteste da fiação (dev, marcador `~/.bwms-terminal-autoteste`): com o terminal aberto, troca o mouse e o teclado de 8 quadros do
/// render de verdade por cliques e teclas sintéticos (aba Items, aba Console, o campo, `help`, Enter) e confere o efeito de cada um.
/// Prova `Entrada` → `Tela` → `montar_terminal` → `Frame` sem tecla física: o `presskey` não chega à fila com o jogo atrás de outra janela.
#[cfg(any(test, feature = "devtools"))]
struct Autoteste {
    passo: PassoDoAutoteste,
    /// Linhas do Console antes do Enter: o `help` acrescenta as da ajuda.
    linhas_antes: usize,
    /// O que cada passo conferido deu, na ordem: vai inteiro no veredito.
    notas: Vec<String>,
    ok: bool,
}

#[cfg(any(test, feature = "devtools"))]
impl Autoteste {
    fn novo() -> Self {
        Autoteste { passo: PassoDoAutoteste::ApertaItems, linhas_antes: 0, notas: Vec::new(), ok: true }
    }

    /// Troca o mouse e o teclado da entrada pelos do passo atual; tela e relógio ficam os do quadro.
    fn preparar(&mut self, e: &mut Entrada, janela: [f32; 4], ui: &UiState) {
        use PassoDoAutoteste::*;
        let (alvo, segurando) = match self.passo {
            ApertaItems => (centro_da_aba_do_terminal(janela, 1), true),
            SoltaItems => (centro_da_aba_do_terminal(janela, 1), false),
            ApertaConsole => (centro_da_aba_do_terminal(janela, 0), true),
            SoltaConsole => (centro_da_aba_do_terminal(janela, 0), false),
            ApertaCampo => (centro_do_campo_do_console(janela), true),
            SoltaCampo | Digita | Enter => (centro_do_campo_do_console(janela), false),
            Feito => return,
        };
        e.mouse = alvo;
        e.segurando = segurando;
        e.apertou = segurando;
        e.soltou = matches!(self.passo, SoltaItems | SoltaConsole | SoltaCampo);
        e.clique_duplo = false;
        e.digitados.clear();
        e.eventos = match self.passo {
            // Cmd+A antes: o `help` troca o que já estivesse no campo.
            Digita => std::iter::once(Evento::Tecla(tecla::A, Mods { cmd: true, ..Mods::default() }))
                .chain("help".chars().map(Evento::Caractere))
                .collect(),
            Enter => vec![Evento::Tecla(tecla::ENTER, Mods::default())],
            _ => Vec::new(),
        };
        e.teclas = teclas_dos_eventos(&e.eventos);
        if self.passo == Enter {
            self.linhas_antes = ui.log_lines.len();
        }
    }

    /// Confere o efeito do passo deste quadro e avança. Devolve o veredito depois do último.
    fn conferir(&mut self, ui: &UiState, p: &Persistente, f: &Frame) -> Option<String> {
        use PassoDoAutoteste::*;
        let sim = |b: bool| if b { "sim" } else { "não" };
        let (proximo, nota) = match self.passo {
            ApertaItems => (SoltaItems, None),
            ApertaConsole => (SoltaConsole, None),
            ApertaCampo => (SoltaCampo, None),
            SoltaItems => (ApertaConsole, Some((p.aba_ativa[0] == 1, format!("items: aba {}", p.aba_ativa[0])))),
            SoltaConsole => (ApertaCampo, Some((p.aba_ativa[0] == 0, format!("console: aba {}", p.aba_ativa[0])))),
            SoltaCampo => {
                let certo = p.foco == Some(0) && p.edicao.is_some() && f.quer_teclado;
                let nota = format!("campo: foco {:?}, edição {}, quer_teclado {}", p.foco, sim(p.edicao.is_some()), sim(f.quer_teclado));
                (Digita, Some((certo, nota)))
            }
            Digita => (Enter, Some((ui.cmd_buf == "help", format!("digitou {:?}", ui.cmd_buf)))),
            Enter => {
                let novas = ui.log_lines.len().saturating_sub(self.linhas_antes);
                let certo = novas == HELP_LINES.len()
                    && ui.cmd_buf.is_empty()
                    && p.foco == Some(0)
                    && p.edicao.is_none()
                    && !f.quer_teclado
                    && ui.cmd_history.last().is_some_and(|c| c == "help");
                let nota = format!(
                    "enter: +{novas} linhas, foco {:?}, edição {}, quer_teclado {}, histórico {:?}",
                    p.foco,
                    sim(p.edicao.is_some()),
                    sim(f.quer_teclado),
                    ui.cmd_history.last()
                );
                (Feito, Some((certo, nota)))
            }
            Feito => return None,
        };
        if let Some((certo, nota)) = nota {
            self.ok &= certo;
            self.notas.push(nota);
        }
        self.passo = proximo;
        (proximo == Feito).then(|| {
            let fim = if self.ok { ">>> TERMINAL-AUTOTESTE OK <<<" } else { "TERMINAL-AUTOTESTE FALHOU" };
            format!("[terminal-autoteste] {} | {fim}", self.notas.join(" | "))
        })
    }
}

// ---------------------------------------------------------------- pedidos de dev: clique e procura

/// Passos do `termonde`. A espera é um quadro sem procura: o mouse só pega o que a janela desenhou no quadro anterior, e o terminal pode ter
/// acabado de abrir.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Copy, PartialEq, Debug)]
enum PassoDoOnde {
    Espera,
    Procura,
}

/// Passos do `termclique`, um por quadro, como o mouse de verdade: leva o ponteiro ao ponto, aperta (o item fica ativo e a aba troca) e
/// solta (a caixa e o botão disparam).
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Debug, PartialEq)]
enum PassoDoClique {
    Leva,
    Aperta,
    Solta(Aperto),
}

/// O quadro do aperto, guardado para o veredito do clique.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Debug, PartialEq)]
struct Aperto {
    quadro: u32,
    /// `Frame::quer_mouse` do aperto: o ponto estava numa janela do terminal, então o jogo não receberia este clique.
    terminal_pegou: bool,
    sob_o_ponto: Vec<ItemAchado>,
}

/// O que o canal de comandos pede ao terminal no dev, atendido pelo render, que é quem monta o quadro: `termonde <rótulo>` diz onde o
/// item está; `termclique <x> <y>` clica ali, em pixels lógicos do terminal (os que o `termonde` dá).
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Debug, PartialEq)]
enum PedidoDeDev {
    Onde { rotulo: String, passo: PassoDoOnde },
    Clique { ponto: [f32; 2], passo: PassoDoClique },
}

#[cfg(any(test, feature = "devtools"))]
impl PedidoDeDev {
    /// O mouse do quadro (ponto lógico, botão seguro) que o render põe no estado que o `sendEvent` alimenta. `None`: fica o de verdade.
    fn mouse(&self) -> Option<([f32; 2], bool)> {
        match self {
            PedidoDeDev::Clique { ponto, passo } => Some((*ponto, matches!(passo, PassoDoClique::Aperta))),
            PedidoDeDev::Onde { .. } => None,
        }
    }

    /// A procura da `Entrada` deste quadro. No aperto do clique o ponto é o `mouse` da entrada, que já passou pelo estado do `sendEvent`.
    fn procura(&self, mouse: [f32; 2]) -> Option<Alvo> {
        match self {
            PedidoDeDev::Onde { rotulo, passo: PassoDoOnde::Procura } => Some(Alvo::Rotulo(rotulo.clone())),
            PedidoDeDev::Clique { passo: PassoDoClique::Aperta, .. } => Some(Alvo::Ponto(mouse)),
            _ => None,
        }
    }

    /// Avança depois do quadro `quadro`, montado com a aba `aba` ativa. Devolve o veredito quando o pedido termina.
    fn depois_do_quadro(&mut self, quadro: u32, aba: usize, f: &Frame) -> Option<String> {
        let aba = ABAS_DO_TERMINAL.get(aba).copied().unwrap_or("?");
        match self {
            PedidoDeDev::Onde { rotulo, passo } => match passo {
                PassoDoOnde::Espera => {
                    *passo = PassoDoOnde::Procura;
                    None
                }
                PassoDoOnde::Procura => Some(veredito_do_onde(rotulo, aba, &f.achados)),
            },
            PedidoDeDev::Clique { ponto, passo } => match passo {
                PassoDoClique::Leva => {
                    *passo = PassoDoClique::Aperta;
                    None
                }
                PassoDoClique::Aperta => {
                    *passo = PassoDoClique::Solta(Aperto { quadro, terminal_pegou: f.quer_mouse, sob_o_ponto: f.achados.clone() });
                    None
                }
                PassoDoClique::Solta(aperto) => Some(veredito_do_clique(*ponto, quadro, aba, aperto)),
            },
        }
    }
}

#[cfg(any(test, feature = "devtools"))]
fn sim(b: bool) -> &'static str {
    if b {
        "sim"
    } else {
        "não"
    }
}

/// Um item achado, numa linha: tipo, rótulo, retângulo, centro, se o clique o alcança e o estado desenhado.
#[cfg(any(test, feature = "devtools"))]
fn descrever_item(i: &ItemAchado) -> String {
    let [x0, y0, x1, y1] = i.ret;
    let estado = match (i.tipo, i.estado) {
        (TipoDeItem::Caixa, Some(e)) => format!(" marcada={}", sim(e)),
        (TipoDeItem::Aba, Some(e)) => format!(" ativa={}", sim(e)),
        (TipoDeItem::Botao, Some(e)) => format!(" marcado={}", sim(e)),
        _ => String::new(),
    };
    let centro = [(x0 + x1) * 0.5, (y0 + y1) * 0.5];
    format!("{} {:?} em [{x0},{y0},{x1},{y1}] centro {},{} visível={}{estado}", i.tipo.nome(), i.rotulo, centro[0], centro[1], sim(i.visivel))
}

/// `[terminal-onde]`: OK com algum item visível; FORA DE VISTA com itens só rolados para fora ou cobertos por outra janela; NADA sem item
/// com o rótulo neste quadro.
#[cfg(any(test, feature = "devtools"))]
fn veredito_do_onde(rotulo: &str, aba: &str, achados: &[ItemAchado]) -> String {
    let algum_visivel = achados.iter().any(|i| i.visivel);
    let fim = if algum_visivel {
        ">>> TERMINAL-ONDE OK <<<"
    } else if achados.is_empty() {
        "TERMINAL-ONDE NADA"
    } else {
        "TERMINAL-ONDE FORA DE VISTA"
    };
    let itens = if achados.is_empty() {
        "nenhum item com esse rótulo".to_string()
    } else {
        format!("{} item(ns) | {}", achados.len(), achados.iter().map(descrever_item).collect::<Vec<_>>().join(" | "))
    };
    format!("[terminal-onde] {rotulo:?} na aba {aba}: {itens} | {fim}")
}

/// `[terminal-clique]`: OK quando o terminal pegou o mouse e havia sob o ponto um item que não é texto, ao alcance do clique.
#[cfg(any(test, feature = "devtools"))]
fn veredito_do_clique(ponto: [f32; 2], quadro_da_soltura: u32, aba: &str, a: &Aperto) -> String {
    let item_clicavel = a.sob_o_ponto.iter().any(|i| i.visivel && i.tipo != TipoDeItem::Texto);
    let fim = if !a.terminal_pegou {
        "TERMINAL-CLIQUE FORA DO TERMINAL"
    } else if item_clicavel {
        ">>> TERMINAL-CLIQUE OK <<<"
    } else {
        "TERMINAL-CLIQUE SEM ITEM"
    };
    let sob = if a.sob_o_ponto.is_empty() {
        "nada".to_string()
    } else {
        a.sob_o_ponto.iter().map(descrever_item).collect::<Vec<_>>().join(" ; ")
    };
    format!(
        "[terminal-clique] {},{} (aba {aba} depois da soltura): apertou no quadro {} e soltou no {quadro_da_soltura} | terminal pegou o mouse={} | sob o ponto: {sob} | {fim}",
        ponto[0],
        ponto[1],
        a.quadro,
        sim(a.terminal_pegou)
    )
}

/// `termclique <x> <y>`: dois números, separados por espaço ou vírgula (o `centro` do `termonde` vale como veio).
#[cfg(any(test, feature = "devtools"))]
fn ponto_do_comando(args: &str) -> Option<[f32; 2]> {
    let mut numeros = args.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(|s| s.parse::<f32>().ok().filter(|v| v.is_finite()));
    let x = numeros.next()??;
    let y = numeros.next()??;
    let sobrou = numeros.next().is_some();
    (!sobrou).then_some([x, y])
}

/// O clique de dev no estado que o `sendEvent` alimenta: o ponto lógico vira pixel do quadro (× `n`), como o `locationInWindow` vira.
#[cfg(any(test, feature = "devtools"))]
fn por_o_mouse(ponto: [f32; 2], segurando: bool, n: f32) {
    MOUSE_X.store((ponto[0] * n).to_bits(), Ordering::Relaxed);
    MOUSE_Y.store((ponto[1] * n).to_bits(), Ordering::Relaxed);
    MOUSE_DOWN.store(segurando, Ordering::Relaxed);
}

/// Fila dos pedidos de dev: o tick põe (`pedir_clique`, `pedir_onde`), o render tira um por vez, na ordem. Um teste de UI por vez: o mouse
/// do terminal é um só.
#[cfg(feature = "devtools")]
static PEDIDOS_DE_DEV: std::sync::Mutex<std::collections::VecDeque<PedidoDeDev>> = std::sync::Mutex::new(std::collections::VecDeque::new());
#[cfg(feature = "devtools")]
const TETO_DE_PEDIDOS_DE_DEV: usize = 8;

/// Põe o pedido na fila do render. Terminal fechado ou fila cheia: o veredito sai na hora, com a ação.
#[cfg(feature = "devtools")]
fn enfileirar_pedido_de_dev(marca: &str, pedido: PedidoDeDev) {
    let tag = marca.to_lowercase();
    if !is_shown() {
        crate::veredito_dev!("[{tag}] o terminal está fechado: mande overlaytoggle antes | {marca} FECHADO");
        return;
    }
    let Ok(mut fila) = PEDIDOS_DE_DEV.lock() else { return };
    let fila_cheia = fila.len() >= TETO_DE_PEDIDOS_DE_DEV;
    if fila_cheia {
        crate::veredito_dev!("[{tag}] fila cheia ({TETO_DE_PEDIDOS_DE_DEV}): espere os vereditos dos anteriores | {marca} FILA CHEIA");
        return;
    }
    fila.push_back(pedido);
}

/// `termonde <rótulo>`: onde está o item de rótulo `rotulo` (o visível ou o inteiro, com `##`) no próximo quadro do terminal. Veredito
/// `[terminal-onde]`.
#[cfg(feature = "devtools")]
pub(crate) fn pedir_onde(rotulo: &str) {
    let rotulo = rotulo.trim();
    if rotulo.is_empty() {
        crate::veredito_dev!("[terminal-onde] uso: termonde <rótulo do item, como aparece no terminal> | TERMINAL-ONDE USO");
        return;
    }
    enfileirar_pedido_de_dev("TERMINAL-ONDE", PedidoDeDev::Onde { rotulo: rotulo.to_string(), passo: PassoDoOnde::Espera });
}

/// `termclique <x> <y>`: um clique do botão esquerdo no ponto, em pixels lógicos do terminal, pelo estado de mouse que o `sendEvent`
/// alimenta, sem evento de sistema. Veredito `[terminal-clique]`.
#[cfg(feature = "devtools")]
pub(crate) fn pedir_clique(args: &str) {
    match ponto_do_comando(args) {
        Some(ponto) => enfileirar_pedido_de_dev("TERMINAL-CLIQUE", PedidoDeDev::Clique { ponto, passo: PassoDoClique::Leva }),
        None => crate::veredito_dev!("[terminal-clique] uso: termclique <x> <y> (pixels lógicos do terminal, o centro do termonde) | TERMINAL-CLIQUE USO"),
    }
}

/// Carrega `<jogo>/red4ext/bwms-splash.png` (troca sem recompilar) → textura Metal RGBA8.
/// Ausente = sem imagem (o splash mostra só a barra de progresso). O nome do arquivo segue `.png`
/// por compatibilidade com quem já tem arte no lugar, mas o decodificador é o do sistema: o conteúdo
/// pode ser PNG, JPEG, HEIC ou TIFF.
pub(crate) unsafe fn carregar_arte_do_splash(dev: Id) -> Option<(Retido, f32, f32)> {
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
    // ImageIO do sistema (ver `decodificar_imagem`): PNG, JPEG, HEIC ou TIFF, já em RGBA8.
    let (rgba, w, h) = decodificar_imagem(&path)?;
    let tdesc = Retido(msg0(classe!("MTLTextureDescriptor"), sel!("new")));
    msg!(tdesc.0, "setPixelFormat:", u64 = MTL_PIXEL_FORMAT_RGBA8_UNORM);
    msg!(tdesc.0, "setWidth:", u64 = w as u64);
    msg!(tdesc.0, "setHeight:", u64 = h as u64);
    msg!(tdesc.0, "setUsage:", u64 = MTL_TEXTURE_USAGE_SHADER_READ);
    let tex = Retido(msg!(dev, "newTextureWithDescriptor:", Id = tdesc.0 => Id));
    let region = MTLRegion {
        origem: MTLOrigin { x: 0, y: 0, z: 0 },
        tamanho: MTLSize { largura: w as u64, altura: h as u64, profundidade: 1 },
    };
    msg!(
        tex.0,
        "replaceRegion:mipmapLevel:withBytes:bytesPerRow:",
        MTLRegion = region,
        u64 = 0,
        *const c_void = rgba.as_ptr() as *const c_void,
        u64 = (w * 4) as u64
    );
    crate::log_dev!("[splash] bwms-splash.png carregado: {w}x{h}");
    Some((tex, w as f32, h as f32))
}
/// O `present` dispara em VÁRIOS threads (redDispatcher1..9) e o terminal tem UM estado (`Persistente`, `UiState`). Seguro porque o
/// Mutex serializa (um thread por vez); a `Tela` que os plugins alcançam só vale na thread que a montou (`api::com_tela_dos_plugins`).
struct SendRenderer(Renderer);
unsafe impl Send for SendRenderer {}
static RENDERER: std::sync::Mutex<Option<SendRenderer>> = std::sync::Mutex::new(None);

pub(crate) const SHADER: &str = r#"
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
// Splash, selo e terminal: NEAREST. A fonte e escalada por inteiro e o filtro linear borraria o traco de 1 px.
fragment float4 f_nearest(VOut in [[stage_in]], texture2d<float> tex [[texture(0)]]) {
    constexpr sampler s(mag_filter::nearest, min_filter::nearest);
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

static BADGE_ENABLED: AtomicBool = AtomicBool::new(true);

/// O texto da aba Mods: as duas formas de pôr mod no jogo. A prova em jogo (`termonde`) o procura inteiro.
pub(crate) const TEXTO_DA_ABA_MODS: &str = "Para pôr um mod no jogo, extraia a pasta dele em Mods/, ao lado do Cyberpunk2077.app, ou instale pelo menu \
                                            Mods do próprio jogo (mod.io). O BWMS lê a Mods/ a cada abertura do jogo e escreve o que entrou e o que \
                                            ficou de fora em Mods/BWMS-RELATORIO.txt.";

/// ReShade-Metal: preset de LUT/grading ativo (0 = off, jogo intocado). Cicla com F2.
static LUT_PRESET: AtomicU32 = AtomicU32::new(0);
const LUT_COUNT: u32 = 7;
const LUT_NAMES: [&str; 7] = ["off", "Quente", "Frio", "P&B", "Contraste", "Sepia", "Bodycam"];
/// Prova de pixel da LUT: seta o preset por comando (sem depender de F2 nem de clique na aba, útil pro
/// cmd-channel de dev). MESMO write atômico que o clique da aba "LUT" já faz — zero risco novo.
pub(crate) fn set_lut_preset(n: u32) {
    LUT_PRESET.store(n % LUT_COUNT, Ordering::Relaxed);
}
/// Efeitos independentes (bitmask) que ligam SOBRE a grade: bit0=Grão, bit1=Vinheta.
/// Texture-sample (Bloom/Aberração/Barril) entram quando o passe ganhar o `scene_tex`.
static EFFECTS: AtomicU32 = AtomicU32::new(0);
const FX_NAMES: [&str; 2] = ["Grão", "Vinheta"];

// Valores dos enums do Metal que o render usa (`MTLPixelFormat.h`, `MTLTexture.h`, `MTLRenderPass.h`, `MTLRenderCommandEncoder.h`,
// `MTLArgument.h`, `MTLResource.h`, `MTLVertexDescriptor.h`, `MTLRenderPipeline.h` do SDK).
const MTL_PIXEL_FORMAT_RGBA8_UNORM: u64 = 70;
const MTL_TEXTURE_USAGE_SHADER_READ: u64 = 1;
pub(crate) const MTL_LOAD_ACTION_LOAD: u64 = 1;
pub(crate) const MTL_STORE_ACTION_STORE: u64 = 1;
pub(crate) const MTL_PRIMITIVE_TYPE_TRIANGLE: u64 = 3;
pub(crate) const MTL_INDEX_TYPE_UINT16: u64 = 0;
/// `MTLResourceStorageModeShared` (`MTLStorageModeShared << MTLResourceStorageModeShift` = 0 << 4): memória que a CPU e a GPU veem.
const MTL_RESOURCE_STORAGE_MODE_SHARED: u64 = 0;
const MTL_VERTEX_FORMAT_UCHAR4_NORMALIZED: u64 = 9;
const MTL_VERTEX_FORMAT_FLOAT2: u64 = 29;
const MTL_BLEND_FACTOR_ONE: u64 = 1;
const MTL_BLEND_FACTOR_SOURCE_ALPHA: u64 = 4;
const MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA: u64 = 5;

#[repr(C)]
struct MTLOrigin {
    x: u64,
    y: u64,
    z: u64,
}
#[repr(C)]
struct MTLSize {
    largura: u64,
    altura: u64,
    profundidade: u64,
}
/// 48 bytes de inteiros: a ABI do arm64 passa por ponteiro para uma cópia, e o `msg!` declara o tipo por valor como o header.
#[repr(C)]
struct MTLRegion {
    origem: MTLOrigin,
    tamanho: MTLSize,
}

/// O `localizedDescription` de um `NSError` (nil vira um texto fixo): é o que sai no log de dev quando o Metal recusa o shader.
unsafe fn descricao_do_erro(erro: Id) -> String {
    let texto = msg_cstr(msg0(erro, sel!("localizedDescription")), sel!("UTF8String"));
    if texto.is_null() {
        return "erro sem descrição".to_string();
    }
    CStr::from_ptr(texto).to_string_lossy().into_owned()
}

/// `-[MTLDevice newLibraryWithSource:options:error:]` com as opções padrão: o [`SHADER`] compilado em tempo de execução.
pub(crate) unsafe fn nova_biblioteca(dev: Id, fonte: &str) -> Result<Retido, String> {
    let opcoes = Retido(msg0(classe!("MTLCompileOptions"), sel!("new")));
    let mut erro: Id = std::ptr::null_mut();
    let lib = Retido(msg!(dev, "newLibraryWithSource:options:error:", Id = nsstring(fonte), Id = opcoes.0, *mut Id = &mut erro => Id));
    if lib.0.is_null() {
        return Err(descricao_do_erro(erro));
    }
    Ok(lib)
}

/// `-[MTLDevice newBufferWithBytes:length:options:]` em memória compartilhada: o Metal copia `tamanho` bytes de `dados`.
pub(crate) unsafe fn novo_buffer(dev: Id, dados: *const c_void, tamanho: u64) -> Retido {
    Retido(msg!(
        dev,
        "newBufferWithBytes:length:options:",
        *const c_void = dados,
        u64 = tamanho,
        u64 = MTL_RESOURCE_STORAGE_MODE_SHARED
        => Id
    ))
}

/// `-[MTLLibrary newFunctionWithName:]`; `None` se o shader não tem a função.
unsafe fn funcao(lib: Id, nome: &str) -> Option<Retido> {
    let f = msg1(lib, sel!("newFunctionWithName:"), nsstring(nome));
    (!f.is_null()).then(|| Retido(f))
}

/// `-[MTLDevice newRenderPipelineStateWithDescriptor:error:]`; `Err` com a descrição quando o Metal recusa o descritor.
unsafe fn novo_pipeline(dev: Id, desc: Id) -> Result<Retido, String> {
    let mut erro: Id = std::ptr::null_mut();
    let pso = Retido(msg!(dev, "newRenderPipelineStateWithDescriptor:error:", Id = desc, *mut Id = &mut erro => Id));
    if !erro.is_null() || pso.0.is_null() {
        return Err(descricao_do_erro(erro));
    }
    Ok(pso)
}

/// Pipeline da malha do projeto (`malha_de_desenho::Vertice`: posição, uv e cor RGBA8) com o fragmento `fragmento` do [`SHADER`] e
/// blend por alfa sobre o quadro do jogo. O splash, o selo e o terminal desenham por ele.
pub(crate) unsafe fn pipeline_da_malha(dev: Id, lib: Id, pixfmt: u64, fragmento: &str) -> Option<Retido> {
    let vf = funcao(lib, "v")?;
    let ff = funcao(lib, fragmento)?;
    let desc = Retido(msg0(classe!("MTLRenderPipelineDescriptor"), sel!("new")));
    msg!(desc.0, "setVertexFunction:", Id = vf.0);
    msg!(desc.0, "setFragmentFunction:", Id = ff.0);
    let vd = msg0(classe!("MTLVertexDescriptor"), sel!("vertexDescriptor"));
    for (i, (formato, off)) in
        [(MTL_VERTEX_FORMAT_FLOAT2, 0u64), (MTL_VERTEX_FORMAT_FLOAT2, 8), (MTL_VERTEX_FORMAT_UCHAR4_NORMALIZED, 16)].iter().enumerate()
    {
        let a = msg!(msg0(vd, sel!("attributes")), "objectAtIndexedSubscript:", u64 = i as u64 => Id);
        if a.is_null() {
            return None;
        }
        msg!(a, "setFormat:", u64 = *formato);
        msg!(a, "setOffset:", u64 = *off);
        msg!(a, "setBufferIndex:", u64 = 0);
    }
    let layout = msg!(msg0(vd, sel!("layouts")), "objectAtIndexedSubscript:", u64 = 0 => Id);
    if layout.is_null() {
        return None;
    }
    msg!(layout, "setStride:", u64 = size_of::<Vertice>() as u64);
    msg!(desc.0, "setVertexDescriptor:", Id = vd);
    let att = msg!(msg0(desc.0, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
    if att.is_null() {
        return None;
    }
    msg!(att, "setPixelFormat:", u64 = pixfmt);
    msg!(att, "setBlendingEnabled:", bool = true);
    msg!(att, "setSourceRGBBlendFactor:", u64 = MTL_BLEND_FACTOR_SOURCE_ALPHA);
    msg!(att, "setDestinationRGBBlendFactor:", u64 = MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA);
    msg!(att, "setSourceAlphaBlendFactor:", u64 = MTL_BLEND_FACTOR_ONE);
    msg!(att, "setDestinationAlphaBlendFactor:", u64 = MTL_BLEND_FACTOR_ONE_MINUS_SOURCE_ALPHA);
    novo_pipeline(dev, desc.0).map_err(|e| crate::log_dev!("[overlay] pipeline {fragmento}: {e}")).ok()
}

/// O atlas do projeto (`malha_de_desenho::montar_atlas`) numa textura RGBA8 da GPU.
pub(crate) unsafe fn textura_do_atlas(dev: Id) -> Retido {
    let px = montar_atlas();
    let tdesc = Retido(msg0(classe!("MTLTextureDescriptor"), sel!("new")));
    msg!(tdesc.0, "setPixelFormat:", u64 = MTL_PIXEL_FORMAT_RGBA8_UNORM);
    msg!(tdesc.0, "setWidth:", u64 = ATLAS_L as u64);
    msg!(tdesc.0, "setHeight:", u64 = ATLAS_A as u64);
    msg!(tdesc.0, "setUsage:", u64 = MTL_TEXTURE_USAGE_SHADER_READ);
    let atlas = Retido(msg!(dev, "newTextureWithDescriptor:", Id = tdesc.0 => Id));
    msg!(
        atlas.0,
        "replaceRegion:mipmapLevel:withBytes:bytesPerRow:",
        MTLRegion = MTLRegion {
            origem: MTLOrigin { x: 0, y: 0, z: 0 },
            tamanho: MTLSize { largura: ATLAS_L as u64, altura: ATLAS_A as u64, profundidade: 1 },
        },
        u64 = 0,
        *const c_void = px.as_ptr() as *const c_void,
        u64 = (ATLAS_L * 4) as u64
    );
    atlas
}

unsafe fn init_renderer(dev: Id, pixfmt: u64) -> Option<Renderer> {
    let lib = nova_biblioteca(dev, SHADER).map_err(|e| crate::log_dev!("[overlay] shader err: {e}")).ok()?;
    let pso = pipeline_da_malha(dev, lib.0, pixfmt, "f_nearest")?;

    // pipeline do LUT (ReShade-Metal): triângulo gerado por vertex_id (sem vertex descriptor),
    // SEM blending (o fragment le o pixel atual via framebuffer-fetch e devolve o final).
    let lut_pso: Option<Retido> = match (funcao(lib.0, "lut_v"), funcao(lib.0, "lut_f")) {
        (Some(lvf), Some(lff)) => {
            let ld = Retido(msg0(classe!("MTLRenderPipelineDescriptor"), sel!("new")));
            msg!(ld.0, "setVertexFunction:", Id = lvf.0);
            msg!(ld.0, "setFragmentFunction:", Id = lff.0);
            let a = msg!(msg0(ld.0, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
            if !a.is_null() {
                msg!(a, "setPixelFormat:", u64 = pixfmt);
            }
            novo_pipeline(dev, ld.0).map_err(|e| crate::log_dev!("[overlay] lut pipeline err: {e}")).ok()
        }
        _ => {
            crate::log_dev!("[overlay] lut: funcoes do shader nao achadas");
            None
        }
    };
    crate::log_dev!("[overlay] terminal pronto (atlas {ATLAS_L}x{ATLAS_A} + pipeline + lut)");
    Some(Renderer {
        pso,
        lut_pso,
        quadro_do_filtro: 0,
        atlas: textura_do_atlas(dev),
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
        persistente: Persistente::default(),
        anterior: EntradaDoQuadroAnterior::default(),
        #[cfg(feature = "devtools")]
        autoteste: None,
        #[cfg(feature = "devtools")]
        pedido: None,
    })
}

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

/// Os 4 estilos estéticos do Cyberpunk 2077 como temas: nome + tagline.
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

// Marca de "clear": a view do console só mostra o log a partir deste BYTE (o tamanho do arquivo na limpeza; esconde o spam de boot
// na 1a abertura do overlay). O arquivo /tmp/cp77-console.log fica intacto (debug).
#[cfg(feature = "devlog")]
static CORTE_DO_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub fn clear_console_view() {
    crate::resposta_ao_usuario::limpar();
    // Build PÚBLICO (sem `devlog`): o log é no-op → nada a ler; não referencia o path (traceless).
    #[cfg(feature = "devlog")]
    CORTE_DO_LOG.store(std::fs::metadata("/tmp/cp77-console.log").map_or(0, |m| m.len()), Ordering::Relaxed);
}

/// Até `n` linhas do fim do log a partir do byte `corte`, lendo no máximo os últimos 64 KB: o log de um boot de dev passa de 5 MB e
/// isto roda no job de render. A linha cortada pelo início da janela sai; `corte` além do fim (log zerado com o jogo aberto) volta a 0.
#[cfg(any(feature = "devlog", test))]
fn ultimas_linhas_do_log(caminho: &str, corte: u64, n: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(caminho) else {
        return Vec::new();
    };
    let tamanho = f.metadata().map_or(0, |m| m.len());
    let corte = if corte > tamanho { 0 } else { corte };
    let inicio = corte.max(tamanho.saturating_sub(64 * 1024));
    // 1 byte antes do início: se for '\n' a 1ª linha está inteira; senão ela começa no meio e sai.
    let mut buf = Vec::new();
    if f.seek(SeekFrom::Start(inicio.saturating_sub(1))).is_err() || f.read_to_end(&mut buf).is_err() {
        return Vec::new();
    }
    let texto = if inicio == 0 { &buf[..] } else { buf.iter().position(|&b| b == b'\n').map_or(&buf[buf.len()..], |p| &buf[p + 1..]) };
    let texto = String::from_utf8_lossy(texto);
    let linhas: Vec<&str> = texto.lines().collect();
    linhas[linhas.len().saturating_sub(n)..].iter().map(|l| l.to_string()).collect()
}
/// As linhas que a aba Console mostra.
///
/// Últimas `n` linhas da saída da aba Console.
///
/// A RESPOSTA AO USUÁRIO vem do buffer em memória (`resposta_ao_usuario`), que existe nos dois builds.
/// O build público não escreve o arquivo de log: lendo só dele, a pessoa digita um comando, o comando
/// EXECUTA e a tela não mostra nada — indistinguível de mod quebrado.
///
/// No build de dev o arquivo de log continua entrando também, porque lá ele é a vista rica de
/// diagnóstico de RE que eu uso pra depurar.
fn tail_log(_n: usize) -> Vec<String> {
    let mut linhas: Vec<String> = Vec::new();
    #[cfg(feature = "devlog")]
    linhas.extend(ultimas_linhas_do_log("/tmp/cp77-console.log", CORTE_DO_LOG.load(Ordering::Relaxed), _n));
    linhas.extend(crate::resposta_ao_usuario::ultimas(_n));
    let start = linhas.len().saturating_sub(_n);
    linhas.split_off(start)
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
const CATALOG_TSV: &str = include_str!("../catalogo_de_itens.tsv");
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

/// A lista da aba Mods (`mods_do_usuario::lista_dos_mods`): no topo a contagem com as partes coloridas do selo, e numa área rolável os mods
/// agrupados por caminho, cada um com a luz (`*` na cor do selo), o nome e, quando não é verde, o motivo na mesma cor; no fim, os conflitos com
/// quem vence. Sem caixa de marcar: não há liga/desliga manual (o mod quebrado sai sozinho, e o do mod.io se liga no menu do jogo). Todo texto
/// vem pronto da lista: o quadro não formata nada de mod.
fn desenhar_lista_dos_mods(
    t: &mut crate::terminal::imediato::Tela,
    l: &mut crate::mods_do_usuario::lista_dos_mods::ListaDosMods,
    plugins: [usize; 4],
) {
    use crate::inicio::splash::CORES_DAS_LUZES;
    l.preparar_cabecalho(plugins);
    t.texto(&l.cabecalho);
    for (luz, parte) in &l.partes {
        t.mesma_linha();
        t.texto_colorido(CORES_DAS_LUZES[*luz as usize], parte);
    }
    t.abrir_rolavel(0.0, 0.0);
    for g in &l.grupos {
        t.texto(&g.titulo);
        if g.indices.is_empty() {
            t.texto_apagado(g.vazio);
        }
        for &i in &g.indices {
            let cor = CORES_DAS_LUZES[l.luzes[i] as usize];
            t.texto_colorido(cor, "*");
            t.mesma_linha();
            t.texto_quebrado(&l.nomes[i]);
            for motivo in l.motivos[i].split('\n').filter(|m| !m.is_empty()) {
                // Recuo de 2 colunas: o motivo fica debaixo do nome, não da luz.
                t.texto("  ");
                t.mesma_linha();
                t.texto_quebrado_colorido(cor, motivo);
            }
        }
        if !g.resto.is_empty() {
            t.texto_apagado(&g.resto);
        }
    }
    if !l.conflitos.is_empty() {
        t.texto("Conflitos (quem vence no .archive):");
        for c in &l.conflitos {
            t.texto_quebrado(c);
        }
    }
    t.fechar_area();
}

/// Constrói o terminal com os widgets do projeto (`terminal::imediato`): abas Console (terminal) / Items (busca+pin) / Favoritos /
/// Mods / K-LOG / CPVR / LUT. Vai no build PÚBLICO: o terminal é o que a pessoa usa pra pedir uma arma, e isso é o produto. O que sai
/// do público é o instrumental de DESCOBERTA (sondas, dumps de RTTI, censo, selftest), não função nem comando. O desenho é o aprovado
/// pelo dono, travado pela prova visual (`prova_visual::terminal_igual_ao_aprovado`).
fn montar_terminal(t: &mut crate::terminal::imediato::Tela, st: &mut UiState) {
    use crate::terminal::imediato::para_bytes;
    if !t.abrir_janela("BWMS  //  CP2077 console") {
        return;
    }
    // seletor de tema: os 4 estilos estéticos do CP2077.
    for (i, (name, tag)) in THEMES.iter().enumerate() {
        if i > 0 {
            t.mesma_linha();
        }
        if t.botao_marcado(name, i == st.theme) {
            st.theme = i;
            st.theme_dirty = true;
        }
        t.dica(tag);
    }
    t.separador();
    // A aba do VR sem headset continua no lugar, apagada e com um aviso: uma aba de controles que não fazem nada faz a pessoa concluir
    // que o mod está quebrado.
    let vr = crate::vr::painel::conectado();
    let rotulos = ["Console", "Items", "Favoritos", "Mods", "K-LOG", if vr { "CPVR" } else { "CPVR##apagada" }, "LUT"];
    match t.abas(&rotulos) {
        // ---- CONSOLE: terminal puro (saída em cima, comando embaixo) ----
        0 => {
            t.abrir_rolavel(0.0, -30.0);
            for line in &st.log_lines {
                t.texto_quebrado(line);
            }
            t.rolar_para_o_fim();
            t.fechar_area();
            let hint = "Enter: money N | give Items.X N | godmode | level N | help";
            let (entered, foco) = t.campo_de_texto(&mut st.cmd_buf, -1.0, hint);
            // Histórico ↑/↓ só com o campo de comando em foco.
            if foco && !st.cmd_history.is_empty() {
                if t.tecla(TECLA_CIMA) {
                    st.history_recall(true);
                    t.cursor_no_fim_do_campo();
                } else if t.tecla(TECLA_BAIXO) {
                    st.history_recall(false);
                    t.cursor_no_fim_do_campo();
                }
            }
            if entered {
                let c = st.cmd_buf.trim().to_string();
                if !c.is_empty() {
                    if c.eq_ignore_ascii_case("help") {
                        // A ajuda vai também ao canal de resposta, como a saída de qualquer comando: a vista do Console é refeita dele
                        // a cada 30 quadros, e a linha posta só na vista sumia em meio segundo.
                        for line in HELP_LINES {
                            crate::resposta_ao_usuario::responder(line);
                            st.log_lines.push((*line).to_string());
                        }
                    } else {
                        write_cmd(&c);
                    }
                    if st.cmd_history.last().map(|s| s.as_str()) != Some(c.as_str()) {
                        st.cmd_history.push(c);
                    }
                }
                st.cmd_history_idx = None;
                st.cmd_buf.clear();
            }
        }
        // ---- ITEMS: busca + Give + pin favorito ----
        1 => {
            t.campo_de_texto(&mut st.search_buf, 330.0, "filtrar (ex: erebus, katana, money)");
            t.mesma_linha();
            t.campo_numerico("qty", &mut st.qty, 80.0);
            if st.qty < 1 {
                st.qty = 1;
            }
            let needle = st.search_buf.to_lowercase();
            let q = st.qty;
            let mut dirty = false;
            t.separador();
            t.abrir_rolavel(0.0, 0.0);
            let mut shown = 0u32;
            for row in catalog().iter() {
                if shown >= 250 {
                    t.texto_apagado("... refine a busca");
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
                if t.botao(&format!("Give##{id}")) {
                    write_cmd(&format!("give {id} {q}"));
                }
                t.mesma_linha();
                let is_fav = st.favorites.iter().any(|(fid, _)| fid == id);
                let star = if is_fav { "fav" } else { "+pin" };
                if t.botao(&format!("{star}##pin{id}")) {
                    if is_fav {
                        st.favorites.retain(|(fid, _)| fid != id);
                    } else {
                        st.favorites.push((id.to_string(), label.to_string()));
                    }
                    dirty = true;
                }
                t.mesma_linha();
                t.texto(&format!("{label}  [{ty}]"));
            }
            t.fechar_area();
            if dirty {
                st.fav_dirty = true;
            }
        }
        // ---- FAVORITOS: itens pinados na aba Items (os cheats vivem em Settings > Cheats) ----
        2 => {
            if !st.favorites.is_empty() {
                t.texto("Favoritos (pinados na aba Items):");
                let mut remove = None;
                for (i, (id, label)) in st.favorites.iter().enumerate() {
                    if t.botao(&format!("Give##f{i}")) {
                        write_cmd(&format!("give {id} 1"));
                    }
                    t.mesma_linha();
                    if t.botao(&format!("x##f{i}")) {
                        remove = Some(i);
                    }
                    t.mesma_linha();
                    t.texto(label);
                }
                if let Some(i) = remove {
                    st.favorites.remove(i);
                    st.fav_dirty = true;
                }
            } else {
                t.texto_apagado("Sem favoritos. Pinhe itens na aba Items (botao +) pra aparecerem aqui.");
            }
            t.separador();
            t.texto_apagado("Cheats (Godmode/Money/Perks/Level...) ficam em Settings > Cheats, sem duplicar.");
        }
        // ---- MODS: badge, atalho, escala do terminal e onde pôr mods ----
        3 => {
            let mut badge_on = BADGE_ENABLED.load(Ordering::Relaxed);
            if t.caixa_de_marcar("Exibir badge (canto superior direito)", &mut badge_on) {
                BADGE_ENABLED.store(badge_on, Ordering::Relaxed);
            }
            // Atalho que abre e fecha o terminal, gravado no `bwms.toml` (`terminal::atalho`).
            if crate::terminal::atalho::gravando() {
                t.texto("Aperte as teclas do novo atalho e solte (Esc cancela).");
            } else {
                t.texto(&format!("Atalho do terminal: {}", crate::terminal::atalho::rotulo()));
                t.mesma_linha();
                if t.botao("Trocar") {
                    crate::terminal::atalho::iniciar_gravacao();
                }
            }
            if let Some(e) = crate::terminal::atalho::erro() {
                t.texto_colorido(para_bytes([1.0, 0.65, 0.0, 1.0]), &e);
            }
            // Escala pela resolução ou a escolhida aqui (`terminal::escala`, gravada no `bwms.toml`).
            t.texto("Escala do terminal:");
            let escolhida = crate::terminal::escala::escolha();
            for (i, nome) in crate::terminal::escala::ESCOLHAS.iter().enumerate() {
                t.mesma_linha();
                if t.botao_marcado(nome, i as u32 == escolhida) {
                    crate::terminal::escala::escolher(i as u32);
                }
            }
            if let Some(n) = crate::terminal::escala::nota() {
                t.mesma_linha();
                t.texto_apagado(&n);
            }
            if let Some(e) = crate::terminal::escala::erro() {
                t.texto_colorido(para_bytes([1.0, 0.65, 0.0, 1.0]), &e);
            }
            t.separador();
            // Mod entra no jogo por dois caminhos só, e nenhum passa por esta aba: ela mostra o que entrou por eles.
            t.texto_quebrado(TEXTO_DA_ABA_MODS);
            t.separador();
            let plugins = crate::mods_do_usuario::contagem_dos_plugins();
            crate::mods_do_usuario::lista_dos_mods::com_a_lista(|l| desenhar_lista_dos_mods(t, l, plugins));
        }
        // ---- K-LOG: captura de teclas (input/atalhos), fora do console ----
        4 => {
            let mut cap = KEY_CAPTURE.load(Ordering::Relaxed);
            if t.caixa_de_marcar("capturar teclas (keyCode/char)", &mut cap) {
                KEY_CAPTURE.store(cap, Ordering::Relaxed);
            }
            t.mesma_linha();
            if t.botao("limpar") {
                if let Ok(mut k) = KEY_LOG.lock() {
                    k.clear();
                }
            }
            t.texto_apagado("teclas capturadas com o overlay FECHADO (debug de input/atalho)");
            t.separador();
            t.abrir_rolavel(0.0, 0.0);
            if let Ok(k) = KEY_LOG.lock() {
                for line in k.iter() {
                    t.texto_quebrado(line);
                }
            }
            t.rolar_para_o_fim();
            t.fechar_area();
        }
        // ---- CPVR: só abre controles com headset; sem ele, o aviso ----
        5 => {
            if vr {
                for linha in crate::vr::painel::resumo() {
                    t.texto(&linha);
                }
                t.separador();
                let mut mudou = false;
                t.texto("Qualidade:");
                let atual = crate::vr::painel::QUALIDADE.load(Ordering::Relaxed);
                for (i, nome) in ["Fluido", "Equilibrado", "Nítido"].iter().enumerate() {
                    if i > 0 {
                        t.mesma_linha();
                    }
                    let marcado = atual == i as u32;
                    if t.botao(&format!("{}{}##q{}", if marcado { "> " } else { "" }, nome, i)) {
                        crate::vr::painel::QUALIDADE.store(i as u32, Ordering::Relaxed);
                        mudou = true;
                    }
                }
                t.espaco();
                let mut escala = crate::vr::painel::ESCALA_DE_RENDER.load(Ordering::Relaxed) as i32;
                if t.deslizante("Escala de render (%)", &mut escala, 50, 150, 0.0) {
                    crate::vr::painel::ESCALA_DE_RENDER.store(escala as u32, Ordering::Relaxed);
                    mudou = true;
                }
                let mut olhos = crate::vr::painel::DISTANCIA_DOS_OLHOS.load(Ordering::Relaxed) as i32;
                if t.deslizante("Distância dos olhos (mm)", &mut olhos, 55, 75, 0.0) {
                    crate::vr::painel::DISTANCIA_DOS_OLHOS.store(olhos as u32, Ordering::Relaxed);
                    mudou = true;
                }
                t.espaco();
                let mut maos = crate::vr::painel::MAOS_LIGADAS.load(Ordering::Relaxed) == 1;
                if t.caixa_de_marcar("Mãos pelos controles", &mut maos) {
                    crate::vr::painel::MAOS_LIGADAS.store(u32::from(maos), Ordering::Relaxed);
                    mudou = true;
                }
                if mudou {
                    crate::vr::painel::gravar_ajustes();
                }
            } else {
                t.texto_apagado(crate::vr::painel::SEM_HEADSET);
            }
        }
        // ---- LUT: filtro de cor e efeitos sobre o frame ----
        _ => {
            let cur = LUT_PRESET.load(Ordering::Relaxed);
            t.texto(&format!("Filtro ativo: {}", LUT_NAMES[cur as usize]));
            t.espaco();
            for (i, name) in LUT_NAMES.iter().enumerate() {
                if i > 0 {
                    t.mesma_linha();
                }
                if t.botao(&format!("{}##lut{}", name, i)) {
                    LUT_PRESET.store(i as u32, Ordering::Relaxed);
                }
            }
            t.separador();
            t.texto("Efeitos (liga/desliga sobre a grade):");
            let mut fx = EFFECTS.load(Ordering::Relaxed);
            for (b, name) in FX_NAMES.iter().enumerate() {
                let bit = 1u32 << b;
                let mut on = (fx & bit) != 0;
                if b > 0 {
                    t.mesma_linha();
                }
                if t.caixa_de_marcar(&format!("{}##fx{}", name, b), &mut on) {
                    if on {
                        fx |= bit;
                    } else {
                        fx &= !bit;
                    }
                    EFFECTS.store(fx, Ordering::Relaxed);
                }
            }
            t.separador();
            t.texto_apagado("Grade (cor) + efeitos AO VIVO no frame; cada um liga/desliga.");
            t.texto_apagado("Em breve (precisa do upgrade do passe): Bloom, Aberracao cromatica, Barril.");
        }
    }
}

/// Filtro de cor (LUT, grão e vinheta) no quadro do jogo, com o terminal aberto ou fechado (DD6). Roda antes do splash, do selo e do
/// terminal: eles saem com a cor deles. Desligado (preset 0, sem efeito), nem trava o `RENDERER`.
unsafe fn aplicar_filtro_de_cor(cb_raw: Id, drawable: Id) {
    let preset = LUT_PRESET.load(Ordering::Relaxed);
    let fxbits = EFFECTS.load(Ordering::Relaxed);
    if (preset == 0 && fxbits == 0) || drawable.is_null() {
        return;
    }
    let tex_raw = msg0(drawable, sel!("texture"));
    let dev_raw = msg0(cb_raw, sel!("device"));
    if tex_raw.is_null() || dev_raw.is_null() {
        return;
    }
    let pixfmt = msg_usize(tex_raw, sel!("pixelFormat")) as u64;
    let mut guard = match RENDERER.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    // O preset pode chegar pelo comando `lut <n>` antes de o terminal abrir: o pipeline do filtro sai do mesmo `init_renderer`.
    if guard.is_none() {
        *guard = init_renderer(dev_raw, pixfmt).map(SendRenderer);
    }
    let Some(rd) = guard.as_mut().map(|x| &mut x.0) else { return };
    let Some(lp) = rd.lut_pso.as_ref() else { return };
    rd.quadro_do_filtro = rd.quadro_do_filtro.wrapping_add(1);
    // Render pass no próprio drawable (Load preserva o quadro para o framebuffer-fetch), sem blending: o fragmento troca cada pixel
    // pela versão graduada.
    let lrpd = msg0(classe!("MTLRenderPassDescriptor"), sel!("renderPassDescriptor"));
    let la = msg!(msg0(lrpd, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
    if la.is_null() {
        return;
    }
    msg!(la, "setTexture:", Id = tex_raw);
    msg!(la, "setLoadAction:", u64 = MTL_LOAD_ACTION_LOAD);
    msg!(la, "setStoreAction:", u64 = MTL_STORE_ACTION_STORE);
    let le = msg!(cb_raw, "renderCommandEncoderWithDescriptor:", Id = lrpd => Id);
    msg!(le, "setRenderPipelineState:", Id = lp.0);
    msg!(le, "setFragmentBytes:length:atIndex:", *const c_void = &preset as *const u32 as *const c_void, u64 = 4, u64 = 0);
    // res (buffer 1) + nº do quadro (buffer 2): o shader os declara, então liga SEMPRE (mesmo nos presets 1-5 que ignoram), senão o
    // Metal aborta por buffer não ligado.
    let res: [u32; 2] = [msg_usize(tex_raw, sel!("width")) as u32, msg_usize(tex_raw, sel!("height")) as u32];
    msg!(le, "setFragmentBytes:length:atIndex:", *const c_void = res.as_ptr() as *const c_void, u64 = 8, u64 = 1);
    msg!(le, "setFragmentBytes:length:atIndex:", *const c_void = &rd.quadro_do_filtro as *const u32 as *const c_void, u64 = 4, u64 = 2);
    msg!(le, "setFragmentBytes:length:atIndex:", *const c_void = &fxbits as *const u32 as *const c_void, u64 = 4, u64 = 3);
    msg!(le, "drawPrimitives:vertexStart:vertexCount:", u64 = MTL_PRIMITIVE_TYPE_TRIANGLE, u64 = 0, u64 = 3);
    msg!(le, "endEncoding");
}

/// O terminal no quadro do jogo, com o terminal aberto, nesta ordem: entrada → `Tela` e `montar_terminal` → janelas dos plugins →
/// ponteiro e fim do quadro (`terminar`) → tema e favoritos no disco → um lote no encoder.
unsafe fn desenhar_terminal(cb_raw: Id, drawable: Id) {
    if drawable.is_null() {
        return;
    }
    let tex_raw = msg0(drawable, sel!("texture"));
    if tex_raw.is_null() {
        return;
    }
    if !LOGGED.swap(true, Ordering::Relaxed) {
        let w = msg_usize(tex_raw, sel!("width"));
        let h = msg_usize(tex_raw, sel!("height"));
        crate::log_dev!("[overlay] render ON — frame {w}x{h}");
    }
    let show = SHOW.load(Ordering::Relaxed);
    // enquanto o painel está FECHADO, a view do console é zerada de tempos em tempos, inclusive no
    // boot → quando o usuario abre, começa VAZIA (sem o spam de boot). Ao abrir, congela e passa a
    // acumular só o que vier novo.
    // 1 a cada 60 frames, não todo frame: isto roda dentro do job de render dos dispatchers, e a view só
    // precisa estar ~atual quando o painel abre. `clear_console_view` trava e esvazia o buffer de
    // `resposta!` (`resposta_ao_usuario::limpar`) nos dois builds; no dev (`devlog`) faz também um `stat`
    // do log para marcar o corte.
    static CLEAR_TICK: AtomicU32 = AtomicU32::new(0);
    if !show && crate::tempo::a_cada(u64::from(CLEAR_TICK.fetch_add(1, Ordering::Relaxed)), 60) {
        clear_console_view();
    }
    // Splash e selo têm renderizador próprio (`inicio::splash`, Metal direto), com o gate em `my_present`; aqui fica só o terminal.
    // Sem painel aberto, este render não tem o que fazer.
    // Gestão do cursor: ABERTO = acopla (clicar no overlay). Ao FECHAR (borda de descida),
    // DESACOPLA p/ devolver a câmera ao jogo — sem isto o mouse-look trava depois do `,
    // porque o jogo só desacopla 1× ao entrar em gameplay e não "re-desacopla" sozinho.
    //
    // ISTO VEM ANTES DE QUALQUER SAÍDA ANTECIPADA. Estava depois do `if !show { return }`, e por isso
    // o ramo que devolve o mouse NUNCA rodava: ao fechar o overlay a função saía antes de chegar nele,
    // e o `PREV_SHOW` ficava preso em `true`. O sintoma é o que o parágrafo acima descreve — abrir o
    // terminal uma vez e o mouse-look do jogo travar.
    let prev_show = PREV_SHOW.swap(show, Ordering::Relaxed);
    if show {
        CGAssociateMouseAndMouseCursorPosition(true);
        if !prev_show {
            OVERLAY_OPEN_EDGE.store(true, Ordering::Relaxed); // Eventos de ciclo de vida: onOverlayOpen
        }
    } else if prev_show {
        CGAssociateMouseAndMouseCursorPosition(false); // fechou agora → mouse volta pra câmera
        OVERLAY_CLOSE_EDGE.store(true, Ordering::Relaxed); // Eventos de ciclo de vida: onOverlayClose
    }
    if !show {
        return;
    }
    let w = msg_usize(tex_raw, sel!("width")) as f32;
    let h = msg_usize(tex_raw, sel!("height")) as f32;
    let pixfmt = msg_usize(tex_raw, sel!("pixelFormat")) as u64;
    let dev_raw = msg0(cb_raw, sel!("device"));
    if dev_raw.is_null() {
        return;
    }

    let mut guard = match RENDERER.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guard.is_none() {
        *guard = init_renderer(dev_raw, pixfmt).map(SendRenderer);
    }
    let rd = match guard.as_mut() {
        Some(x) => &mut x.0,
        None => return,
    };
    // O terminal é montado em pixels lógicos (quadro ÷ escala) e a projeção amplia por inteiro (`terminal::escala`).
    let n = crate::terminal::escala::deste_quadro([w, h]) as f32;
    {
        FRAME_W.store(w.to_bits(), Ordering::Relaxed);
        FRAME_H.store(h.to_bits(), Ordering::Relaxed);
        rd.ui.frame = rd.ui.frame.wrapping_add(1);
        if show && rd.ui.frame % 30 == 1 {
            rd.ui.log_lines = tail_log(60);
        }
        // Histórico do console: self-test AUTOMATIZÁVEL no menu (sem HID): roda 1× no `cmd_history` REAL
        // do overlay, gate ~/.bwms-histtest (dev). Ver `run_history_selftest`.
        {
            static HISTTEST_DONE: AtomicBool = AtomicBool::new(false);
            if !HISTTEST_DONE.load(Ordering::Relaxed)
                && crate::marcador_de_dev(".bwms-histtest")
            {
                rd.ui.run_history_selftest();
                HISTTEST_DONE.store(true, Ordering::Relaxed);
            }
        }
        // 1. Entrada: o teclado na ordem em que chegou e o mouse do `sendEvent`, em pixels lógicos.
        let eventos = match INPUT_Q.lock() {
            Ok(mut q) => std::mem::take(&mut *q),
            Err(_) => Vec::new(),
        };
        // Pedido de dev (`termclique`, `termonde`): o próximo da fila, um passo por quadro. O clique escreve no estado que o `sendEvent`
        // alimenta, e a entrada sai dele pelo caminho do mouse de verdade.
        #[cfg(feature = "devtools")]
        {
            if rd.pedido.is_none() {
                rd.pedido = PEDIDOS_DE_DEV.lock().ok().and_then(|mut fila| fila.pop_front());
            }
            if let Some((ponto, segurando)) = rd.pedido.as_ref().and_then(PedidoDeDev::mouse) {
                por_o_mouse(ponto, segurando, n);
            }
        }
        let (mouse, segurando) = mouse_do_sendevent(n);
        #[allow(unused_mut)]
        let mut e = rd.anterior.entrada(mouse, segurando, eventos, [w / n, h / n], std::time::Instant::now());
        #[cfg(feature = "devtools")]
        {
            e.procura = rd.pedido.as_ref().and_then(|p| p.procura(e.mouse));
            static AUTOTESTE_COMECOU: AtomicBool = AtomicBool::new(false);
            if !AUTOTESTE_COMECOU.load(Ordering::Relaxed) && crate::marcador_de_dev(".bwms-terminal-autoteste") {
                AUTOTESTE_COMECOU.store(true, Ordering::Relaxed);
                rd.autoteste = Some(Autoteste::novo());
                crate::log_dev!("[terminal-autoteste] começou (marcador ~/.bwms-terminal-autoteste)");
            }
            if let Some(a) = rd.autoteste.as_mut() {
                let janela = rd.persistente.janelas.first().map_or(crate::terminal::imediato::JANELA_INICIAL, |j| j.ret);
                a.preparar(&mut e, janela, &rd.ui);
            }
        }
        // 2–5. Terminal, janelas dos plugins, ponteiro e fim do quadro.
        let inicio = std::time::Instant::now();
        let mut area = AreaDoSistema;
        let f = montar_quadro(&mut rd.ui, &mut rd.persistente, e, &mut area);
        let montagem = inicio.elapsed();
        WANT_KEYBOARD.store(f.quer_teclado, Ordering::Relaxed);
        #[cfg(feature = "devtools")]
        if let Some(a) = rd.autoteste.as_mut() {
            if let Some(veredito) = a.conferir(&rd.ui, &rd.persistente, &f) {
                crate::veredito_dev!("{veredito}");
                rd.autoteste = None;
            }
        }
        #[cfg(feature = "devtools")]
        if let Some(p) = rd.pedido.as_mut() {
            if let Some(veredito) = p.depois_do_quadro(rd.ui.frame, rd.persistente.aba_ativa[0], &f) {
                crate::veredito_dev!("{veredito}");
                rd.pedido = None;
            }
        }
        // 6. Tema e favoritos trocados neste quadro vão para o disco; a paleta nova entra no `Tela::nova` do quadro seguinte.
        if rd.ui.theme_dirty {
            save_theme(rd.ui.theme);
            rd.ui.theme_dirty = false;
        }
        if rd.ui.fav_dirty {
            save_favs(&rd.ui.favorites);
            rd.ui.fav_dirty = false;
        }
        static PRIMEIRO_QUADRO: AtomicBool = AtomicBool::new(false);
        if !PRIMEIRO_QUADRO.swap(true, Ordering::Relaxed) {
            let [jx, jy, jl, ja] = rd.persistente.janelas.first().map_or([0.0; 4], |j| j.ret);
            crate::veredito_dev!(
                "[terminal] 1º quadro aberto: drawable {w}x{h}, escala {n}×, {} vértices, {} índices, janela [{jx},{jy},{jl},{ja}], quer_teclado={}, montagem {} µs",
                f.malha.vertices.len(),
                f.malha.indices.len(),
                f.quer_teclado,
                montagem.as_micros()
            );
        }

        // 7. Um lote: a malha inteira (o recorte já foi feito na CPU), o atlas e o pipeline nítido. O filtro de cor já passou
        // (`aplicar_filtro_de_cor`, no `my_present`).
        let m = &f.malha;
        if m.indices.is_empty() {
            return;
        }
        let rpd = msg0(classe!("MTLRenderPassDescriptor"), sel!("renderPassDescriptor"));
        let att = msg!(msg0(rpd, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
        if att.is_null() {
            return;
        }
        msg!(att, "setTexture:", Id = tex_raw);
        msg!(att, "setLoadAction:", u64 = MTL_LOAD_ACTION_LOAD);
        msg!(att, "setStoreAction:", u64 = MTL_STORE_ACTION_STORE);
        let enc = msg!(cb_raw, "renderCommandEncoderWithDescriptor:", Id = rpd => Id);
        msg!(enc, "setRenderPipelineState:", Id = rd.pso.0);
        // projeção ortográfica (pixels lógicos top-left → NDC): o quadro tem `n` pixels por pixel lógico.
        let proj: [[f32; 4]; 4] = [
            [2.0 * n / w, 0.0, 0.0, 0.0],
            [0.0, -2.0 * n / h, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0, 1.0],
        ];
        msg!(enc, "setVertexBytes:length:atIndex:", *const c_void = proj.as_ptr() as *const c_void, u64 = 64, u64 = 1);
        let vbuf = novo_buffer(dev_raw, m.vertices.as_ptr() as *const c_void, (m.vertices.len() * size_of::<Vertice>()) as u64);
        let ibuf = novo_buffer(dev_raw, m.indices.as_ptr() as *const c_void, (m.indices.len() * 2) as u64);
        msg!(enc, "setFragmentTexture:atIndex:", Id = rd.atlas.0, u64 = 0);
        msg!(enc, "setVertexBuffer:offset:atIndex:", Id = vbuf.0, u64 = 0, u64 = 0);
        msg!(
            enc,
            "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:",
            u64 = MTL_PRIMITIVE_TYPE_TRIANGLE,
            u64 = m.indices.len() as u64,
            u64 = MTL_INDEX_TYPE_UINT16,
            Id = ibuf.0,
            u64 = 0
        );
        msg!(enc, "endEncoding");
        #[cfg(any(feature = "devtools", test))]
        if sonda_de_pixels::quer("terminal") {
            sonda_de_pixels::anotar_malha("terminal", m, n);
        }
    }
}

/// Prova de pixel do caminho Metal dentro do jogo, sem captura de tela: só no dev, com `~/.bwms-prova-metal`. Cada desenho (splash,
/// selo, terminal) anota pontos que a malha deixa com cor conhecida: quad da célula branca com alfa 255, no centro ou a 2 px do canto,
/// sem triângulo desenhado depois por cima. No fim do `present`, um passe lê esses pixels do drawable por framebuffer fetch para um
/// buffer compartilhado, com máscara de escrita zero: o quadro não muda. O filtro de cor é conferido no P&B (`lut 3`, sem efeito): o
/// pixel do centro, lido na entrada e na saída do filtro, tem que dar o cinza da conta do shader. Quando o command buffer completa, sai uma
/// linha de veredito por desenho.
#[cfg(any(feature = "devtools", test))]
pub(crate) mod sonda_de_pixels {
    use super::*;
    use crate::terminal::malha_de_desenho::{Malha, CELULA_BRANCA, CELULA_L};

    const FONTE: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct S { float4 pos [[position]]; };
vertex S sonda_v(uint vid [[vertex_id]]) {
    float2 uv = float2((vid << 1) & 2, vid & 2);
    S o; o.pos = float4(uv * 2.0 - 1.0, 0.0, 1.0); return o;
}
fragment float4 sonda_f(float4 cur [[color(0)]], float4 pos [[position]],
                        constant uint2* pontos [[buffer(0)]], constant uint& n [[buffer(1)]],
                        device float4* lidos [[buffer(2)]]) {
    uint2 p = uint2(pos.xy);
    for (uint i = 0; i < n; i++) {
        if (p.x == pontos[i].x && p.y == pontos[i].y) { lidos[i] = cur; }
    }
    return cur;
}
"#;

    /// `MTLCommandBufferStatusCompleted` e `MTLCommandBufferStatusError` (`MTLCommandBuffer.h`).
    const STATUS_COMPLETO: usize = 4;
    const STATUS_ERRO: usize = 5;
    /// Teto de pontos de uma leitura: cabe no `setFragmentBytes` e no laço do fragmento.
    const TETO_DE_PONTOS: usize = 16;
    /// Pontos por desenho.
    const PONTOS_POR_DESENHO: usize = 3;
    const DESENHOS: [&str; 4] = ["splash", "selo", "terminal", "filtro"];
    /// Diferença aceita por canal, em 1/255: o formato sRGB do drawable arredonda a ida e a volta.
    const TOLERANCIA: f32 = 2.0;

    #[derive(Clone, Copy)]
    enum Esperado {
        Cor([u8; 4]),
        /// O cinza do P&B do pixel lido na entrada do filtro.
        CinzaDaEntrada,
    }

    struct Ponto {
        desenho: &'static str,
        xy: [u32; 2],
        esperado: Esperado,
    }

    /// Leitura encodada num quadro, esperando o command buffer completar.
    struct Leitura {
        cb: Retido,
        lidos: Retido,
        entrada: Option<Retido>,
        pontos: Vec<Ponto>,
        /// Largura, altura e formato do drawable.
        quadro: [u64; 3],
    }

    #[derive(Default)]
    struct Estado {
        /// Pipeline da leitura e o formato para o qual foi criado.
        pipeline: Option<(u64, Retido)>,
        anotados: Vec<Ponto>,
        entrada_do_filtro: Option<(Retido, [u32; 2])>,
        pendente: Option<Leitura>,
        conferidos: Vec<(&'static str, bool)>,
    }
    struct EstadoEnviavel(Estado);
    // O `present` chega em várias threads de dispatcher; o Mutex serializa, o mesmo contrato do `RENDERER`.
    unsafe impl Send for EstadoEnviavel {}
    static ESTADO: std::sync::Mutex<Option<EstadoEnviavel>> = std::sync::Mutex::new(None);

    #[cfg(test)]
    pub(crate) static FORCADA: AtomicBool = AtomicBool::new(false);

    fn ligada() -> bool {
        #[cfg(test)]
        if FORCADA.load(Ordering::Relaxed) {
            return true;
        }
        crate::marcador_de_dev(".bwms-prova-metal")
    }

    fn com_estado<R>(f: impl FnOnce(&mut Estado) -> R) -> R {
        let mut g = match ESTADO.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        f(&mut g.get_or_insert_with(|| EstadoEnviavel(Estado::default())).0)
    }

    /// O desenho ainda precisa de prova e não há leitura em curso.
    pub(crate) fn quer(desenho: &str) -> bool {
        ligada() && com_estado(|e| e.pendente.is_none() && !e.conferidos.iter().any(|(d, _)| *d == desenho))
    }

    /// Anota os pontos que `m` deixa com cor conhecida. `escala`: pixels do drawable por unidade da malha.
    pub(crate) fn anotar_malha(desenho: &'static str, m: &Malha, escala: f32) {
        let pontos = pontos_de_cor_conhecida(m, escala);
        com_estado(|e| e.anotados.extend(pontos.into_iter().map(|(xy, cor)| Ponto { desenho, xy, esperado: Esperado::Cor(cor) })));
    }

    /// Até `PONTOS_POR_DESENHO` pixels, do quad mais de cima para baixo: quad da célula branca com alfa 255 (a cor sai exata, o blend
    /// vira cópia), pixel inteiro dentro dele e nenhum triângulo desenhado depois com a caixa sobre o centro do pixel.
    pub(crate) fn pontos_de_cor_conhecida(m: &Malha, escala: f32) -> Vec<([u32; 2], [u8; 4])> {
        let u_da_celula_branca = (CELULA_BRANCA * CELULA_L) as f32 / crate::terminal::malha_de_desenho::ATLAS_L as f32;
        let triangulos: Vec<&[u16]> = m.indices.chunks_exact(3).collect();
        let caixa = |t: &[u16]| {
            let mut c = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for &i in t {
                let Some(v) = m.vertices.get(i as usize) else { continue };
                c = [c[0].min(v.pos[0]), c[1].min(v.pos[1]), c[2].max(v.pos[0]), c[3].max(v.pos[1])];
            }
            c.map(|x| x * escala)
        };
        let coberto = |depois_de: usize, p: [f32; 2]| {
            triangulos[depois_de..].iter().any(|t| {
                let c = caixa(t);
                p[0] >= c[0] && p[0] <= c[2] && p[1] >= c[1] && p[1] <= c[3]
            })
        };
        let mut achados = Vec::new();
        for t in (0..triangulos.len().saturating_sub(1)).rev() {
            if achados.len() == PONTOS_POR_DESENHO {
                break;
            }
            let (a, b) = (triangulos[t], triangulos[t + 1]);
            let base = a[0];
            let Some(b3) = base.checked_add(3) else { continue };
            if a != [base, base + 1, base + 2] || b != [base, base + 2, b3] {
                continue;
            }
            let Some(v) = m.vertices.get(base as usize..base as usize + 4) else { continue };
            let cor = v[0].cor;
            if cor[3] != 255 || !v.iter().all(|p| p.cor == cor && p.uv[0] >= u_da_celula_branca) {
                continue;
            }
            let r = [v[0].pos[0] * escala, v[0].pos[1] * escala, v[2].pos[0] * escala, v[2].pos[1] * escala];
            if r[2] - r[0] < 4.0 || r[3] - r[1] < 4.0 {
                continue;
            }
            for p in [[((r[0] + r[2]) * 0.5).floor(), ((r[1] + r[3]) * 0.5).floor()], [r[0].ceil() + 2.0, r[1].ceil() + 2.0]] {
                let centro = [p[0] + 0.5, p[1] + 0.5];
                let dentro = p[0] >= r[0].ceil() && p[0] + 1.0 <= r[2].floor() && p[1] >= r[1].ceil() && p[1] + 1.0 <= r[3].floor();
                if dentro && p[0] >= 0.0 && p[1] >= 0.0 && !coberto(t + 2, centro) {
                    achados.push(([p[0] as u32, p[1] as u32], cor));
                    break;
                }
            }
        }
        achados
    }

    /// Começo do quadro, com o filtro de cor por vir: confere a leitura que completou e, na vez do P&B, lê o centro do quadro sem o filtro.
    pub(crate) unsafe fn comecar_quadro(cb: Id, drawable: Id) -> Vec<String> {
        if !ligada() {
            return Vec::new();
        }
        let linhas = com_estado(|e| conferir(e));
        let vez_do_filtro = quer("filtro")
            && LUT_PRESET.load(Ordering::Relaxed) == 3
            && EFFECTS.load(Ordering::Relaxed) == 0
            && !crate::inicio::boot::boot_splash_active()
            && !SHOW.load(Ordering::Relaxed);
        if vez_do_filtro && !drawable.is_null() {
            let tex = msg0(drawable, sel!("texture"));
            if !tex.is_null() {
                let centro = [msg_usize(tex, sel!("width")) as u32 / 2, msg_usize(tex, sel!("height")) as u32 / 2];
                com_estado(|e| {
                    if let Some(entrada) = encodar_leitura(e, cb, tex, &[centro]) {
                        e.entrada_do_filtro = Some((entrada, centro));
                    }
                });
            }
        }
        linhas
    }

    /// Depois de todos os desenhos: encoda a leitura dos pontos anotados neste quadro.
    pub(crate) unsafe fn fim_do_quadro(cb: Id, drawable: Id) {
        if !ligada() || drawable.is_null() {
            return;
        }
        com_estado(|e| {
            let mut pontos = std::mem::take(&mut e.anotados);
            let entrada = e.entrada_do_filtro.take();
            if let Some((_, centro)) = entrada.as_ref() {
                pontos.push(Ponto { desenho: "filtro", xy: *centro, esperado: Esperado::CinzaDaEntrada });
            }
            let tex = msg0(drawable, sel!("texture"));
            if pontos.is_empty() || tex.is_null() || e.pendente.is_some() {
                return;
            }
            let quadro = [
                msg_usize(tex, sel!("width")) as u64,
                msg_usize(tex, sel!("height")) as u64,
                msg_usize(tex, sel!("pixelFormat")) as u64,
            ];
            pontos.retain(|p| u64::from(p.xy[0]) < quadro[0] && u64::from(p.xy[1]) < quadro[1]);
            pontos.truncate(TETO_DE_PONTOS);
            let xy: Vec<[u32; 2]> = pontos.iter().map(|p| p.xy).collect();
            let Some(lidos) = encodar_leitura(e, cb, tex, &xy) else { return };
            msg0(cb, sel!("retain"));
            e.pendente = Some(Leitura { cb: Retido(cb), lidos, entrada: entrada.map(|(b, _)| b), pontos, quadro });
        });
    }

    /// Passe que copia os pixels `xy` de `tex` para um buffer compartilhado de `float4`, sem escrever no quadro.
    unsafe fn encodar_leitura(e: &mut Estado, cb: Id, tex: Id, xy: &[[u32; 2]]) -> Option<Retido> {
        let dev = msg0(cb, sel!("device"));
        let pixfmt = msg_usize(tex, sel!("pixelFormat")) as u64;
        if dev.is_null() || xy.is_empty() || xy.len() > TETO_DE_PONTOS {
            return None;
        }
        if e.pipeline.as_ref().map_or(true, |(f, _)| *f != pixfmt) {
            e.pipeline = pipeline_da_leitura(dev, pixfmt).map(|p| (pixfmt, p));
        }
        let (_, pso) = e.pipeline.as_ref()?;
        let tamanho = (xy.len() * 16) as u64;
        let lidos = Retido(msg!(dev, "newBufferWithLength:options:", u64 = tamanho, u64 = MTL_RESOURCE_STORAGE_MODE_SHARED => Id));
        if lidos.0.is_null() {
            return None;
        }
        let rpd = msg0(classe!("MTLRenderPassDescriptor"), sel!("renderPassDescriptor"));
        let att = msg!(msg0(rpd, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
        if att.is_null() {
            return None;
        }
        msg!(att, "setTexture:", Id = tex);
        msg!(att, "setLoadAction:", u64 = MTL_LOAD_ACTION_LOAD);
        msg!(att, "setStoreAction:", u64 = MTL_STORE_ACTION_STORE);
        let enc = msg!(cb, "renderCommandEncoderWithDescriptor:", Id = rpd => Id);
        let n = xy.len() as u32;
        msg!(enc, "setRenderPipelineState:", Id = pso.0);
        msg!(enc, "setFragmentBytes:length:atIndex:", *const c_void = xy.as_ptr() as *const c_void, u64 = (xy.len() * 8) as u64, u64 = 0);
        msg!(enc, "setFragmentBytes:length:atIndex:", *const c_void = &n as *const u32 as *const c_void, u64 = 4, u64 = 1);
        msg!(enc, "setFragmentBuffer:offset:atIndex:", Id = lidos.0, u64 = 0, u64 = 2);
        msg!(enc, "drawPrimitives:vertexStart:vertexCount:", u64 = MTL_PRIMITIVE_TYPE_TRIANGLE, u64 = 0, u64 = 3);
        msg!(enc, "endEncoding");
        Some(lidos)
    }

    /// Triângulo de tela cheia com o formato do drawable e máscara de escrita zero (`MTLColorWriteMaskNone`).
    unsafe fn pipeline_da_leitura(dev: Id, pixfmt: u64) -> Option<Retido> {
        let lib = nova_biblioteca(dev, FONTE).ok()?;
        let (vf, ff) = (funcao(lib.0, "sonda_v")?, funcao(lib.0, "sonda_f")?);
        let desc = Retido(msg0(classe!("MTLRenderPipelineDescriptor"), sel!("new")));
        msg!(desc.0, "setVertexFunction:", Id = vf.0);
        msg!(desc.0, "setFragmentFunction:", Id = ff.0);
        let att = msg!(msg0(desc.0, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
        if att.is_null() {
            return None;
        }
        msg!(att, "setPixelFormat:", u64 = pixfmt);
        msg!(att, "setWriteMask:", u64 = 0);
        novo_pipeline(dev, desc.0).ok()
    }

    /// A leitura pendente, se o command buffer já completou: uma linha por desenho, e a final quando os quatro passaram.
    pub(crate) unsafe fn conferir_pendente() -> Vec<String> {
        com_estado(|e| conferir(e))
    }

    unsafe fn conferir(e: &mut Estado) -> Vec<String> {
        let Some(l) = e.pendente.as_ref() else { return Vec::new() };
        let status = msg_usize(l.cb.0, sel!("status"));
        if status != STATUS_COMPLETO && status != STATUS_ERRO {
            return Vec::new();
        }
        let Some(l) = e.pendente.take() else { return Vec::new() };
        let [largura, altura, pixfmt] = l.quadro;
        if status == STATUS_ERRO {
            return vec![format!("[prova-metal] o command buffer do quadro {largura}x{altura} terminou com erro: leitura descartada")];
        }
        let ler = |b: &Retido, i: usize| -> [f32; 4] {
            let p = msg0(b.0, sel!("contents")) as *const [f32; 4];
            if p.is_null() { [f32::NAN; 4] } else { *p.add(i) }
        };
        let byte = |c: f32| c * 255.0;
        let mut linhas = Vec::new();
        for desenho in DESENHOS {
            let dele: Vec<(usize, &Ponto)> = l.pontos.iter().enumerate().filter(|(_, p)| p.desenho == desenho).collect();
            if dele.is_empty() {
                continue;
            }
            let mut certo = true;
            let mut detalhes = Vec::new();
            for (i, p) in dele {
                let lido = ler(&l.lidos, i).map(byte);
                let esperado: [f32; 4] = match p.esperado {
                    Esperado::Cor(c) => c.map(f32::from),
                    Esperado::CinzaDaEntrada => {
                        let Some(entrada) = l.entrada.as_ref().map(|b| ler(b, 0)) else {
                            certo = false;
                            continue;
                        };
                        let cinza = (entrada[0] * 0.299 + entrada[1] * 0.587 + entrada[2] * 0.114).clamp(0.0, 1.0) * 255.0;
                        detalhes.push(format!("entrada {:.0},{:.0},{:.0}", entrada[0] * 255.0, entrada[1] * 255.0, entrada[2] * 255.0));
                        [cinza, cinza, cinza, entrada[3] * 255.0]
                    }
                };
                let bate = lido.iter().zip(esperado).all(|(a, b)| (a - b).abs() <= TOLERANCIA);
                certo &= bate;
                detalhes.push(format!(
                    "({},{}) esperado {:.0},{:.0},{:.0},{:.0} lido {:.0},{:.0},{:.0},{:.0} {}",
                    p.xy[0], p.xy[1], esperado[0], esperado[1], esperado[2], esperado[3], lido[0], lido[1], lido[2], lido[3],
                    if bate { "certo" } else { "ERRADO" }
                ));
            }
            e.conferidos.push((desenho, certo));
            linhas.push(format!(
                "[prova-metal] {desenho} {} | quadro {largura}x{altura} formato {pixfmt} | {}",
                if certo { "OK" } else { "FALHOU" },
                detalhes.join(" | ")
            ));
        }
        if e.conferidos.len() == DESENHOS.len() && !linhas.is_empty() {
            let todos = e.conferidos.iter().all(|(_, c)| *c);
            let fim = if todos { ">>> PROVA-METAL OK: splash, selo, terminal e filtro <<<" } else { "PROVA-METAL FALHOU" };
            linhas.push(fim.to_string());
        }
        linhas
    }

    #[cfg(test)]
    mod teste {
        use super::pontos_de_cor_conhecida;
        use crate::terminal::malha_de_desenho::Malha;

        /// Quad translúcido não entra, o centro coberto por um quad desenhado depois cede para o canto, e o quad de cima vem primeiro.
        #[test]
        fn escolhe_pixel_de_cor_conhecida_que_nada_cobre() {
            let mut m = Malha::default();
            m.retangulo(0.0, 0.0, 100.0, 100.0, [10, 20, 30, 255]);
            m.retangulo(40.0, 40.0, 60.0, 60.0, [1, 2, 3, 128]);
            m.retangulo(200.0, 0.0, 210.0, 10.0, [200, 0, 0, 255]);
            assert_eq!(pontos_de_cor_conhecida(&m, 2.0), vec![([410, 10], [200, 0, 0, 255]), ([2, 2], [10, 20, 30, 255])]);
        }
    }
}

// ---------------------------------------------------------------- present hook

// ===== ENGAGEMENT do boot ("APERTE ESPAÇO PARA CONTINUAR") =====
// Sinal vindo do redscript (`BwmsEngagementOn/Off`). Nada aqui injeta tecla: a tela sai pela alavanca
// d4=2 (`inicio/natives.rs`, com `pular_intro`) ou pelo ESPAÇO do usuário.
static ENGAGEMENT_ACTIVE: AtomicBool = AtomicBool::new(false); // setado pelo redscript (sinal preciso)
static SEEN_ENGAGEMENT: AtomicBool = AtomicBool::new(false); // já apareceu 1x nesta sessão (p/ boot_progress)

/// A engagement do boot está ativa agora? (setado pelo redscript). Lido pela thread do heartbeat
/// `[hb]` (`lib.rs`) e pelo log `[phasedbg]` do `phase_skip_getter` (`inicio/boot.rs`, só com `devlog`).
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
/// marca cedo demais; 'sem player' é falso no pregame por causa do puppet do V).
pub fn set_engagement_active(on: bool) {
    ENGAGEMENT_ACTIVE.store(on, Ordering::Relaxed);
    if on {
        SEEN_ENGAGEMENT.store(true, Ordering::Relaxed);
    }
    crate::log_dev!("[skipintro] engagement ativa = {on} (via redscript)");
}

extern "C" fn my_present(this: Id, cmd: Sel, drawable: Id) {
    unsafe {
        #[cfg(feature = "devtools")]
        for linha in sonda_de_pixels::comecar_quadro(this, drawable) {
            crate::veredito_dev!("{linha}");
        }
        aplicar_filtro_de_cor(this, drawable);
        // Splash de boot e selo: renderizador próprio, antes do terminal. O splash só arma com `pular_intro`, que também liga o lever,
        // então cobre a engagement inteira: quem a dispensa é o lever, não o usuário (`inicio/boot.rs`, `BOOT_SPLASH_ON`).
        if crate::inicio::boot::boot_splash_active() {
            crate::inicio::splash::desenhar(this, drawable);
        } else if crate::ticks() > 0 && BADGE_ENABLED.load(Ordering::Relaxed) && !SHOW.load(Ordering::Relaxed) {
            crate::inicio::splash::desenhar_selo(this, drawable);
        }
        desenhar_terminal(this, drawable);
        #[cfg(feature = "devtools")]
        sonda_de_pixels::fim_do_quadro(this, drawable);
        // VR: captura a cor do drawable final (game + UI/ink + nosso overlay) pro Quest.
        // Mesmo hook = VR e terminal juntos, um dylib. Só compila com `--features capture`.
        #[cfg(feature = "capture")]
        crate::vr::captura::on_present(this, drawable);
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
        crate::log_dev!("[overlay] MTLCreateSystemDefaultDevice = null");
        return;
    }
    let q = msg0(dev, sel!("newCommandQueue"));
    let cb = if q.is_null() {
        std::ptr::null_mut()
    } else {
        msg0(q, sel!("commandBuffer"))
    };
    if cb.is_null() {
        crate::log_dev!("[overlay] sem command buffer p/ achar a classe");
        return;
    }
    let cb_class = object_getClass(cb);
    let name = CStr::from_ptr(class_getName(cb_class))
        .to_string_lossy()
        .into_owned();
    let m = class_getInstanceMethod(cb_class, sel!("presentDrawable:"));
    if m.is_null() {
        crate::log_dev!("[overlay] sem presentDrawable: em {name}");
        return;
    }
    ORIG_PRESENT.store(method_getImplementation(m) as *mut c_void, Ordering::Relaxed);
    method_setImplementation(m, my_present as Imp);
    crate::log_dev!("[overlay] presentDrawable: swizzlado em {name}");
}

pub fn start() {
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(3));
        unsafe {
            install_present_hook();
            // O swizzle sai do objeto NSApp que a main do jogo criou, sem mensagem ao AppKit desta thread: criar o NSApp ou abrir a
            // conexão com o WindowServer fora da main prende a fila de eventos aqui e o AppKit derruba o jogo no 1º
            // `nextEventMatchingMask:` da main (SIGTRAP, `proofs/20261001-sigtrap-nsapp-fora-da-main.log`).
            #[cfg(feature = "devlog")]
            let espera = std::time::Instant::now();
            while !nsapp_do_jogo_existe() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            #[cfg(feature = "devlog")]
            crate::log_dev!("[overlay] NSApp do jogo pronto {} ms depois dos 3 s", espera.elapsed().as_millis());
            // Swizzle do `sendEvent:`: é ele que faz a tecla ` abrir o terminal do mod. Isso é
            // FUNÇÃO do produto — quem instala o BWMS digita ali pra receber uma arma —, então vai
            // no build público também. O que fica de fora é o input-logger do dataset de ML.
            install_input_hook();
        }
    });
    // hook do scaler MetalFX (captura de depth) — só com `--features capture`.
    #[cfg(feature = "capture")]
    crate::vr::captura::install_scaler_hooks();
}

/// Global `NSApp` do AppKit: nulo até a main do jogo chamar `sharedApplication`.
unsafe fn nsapp_do_jogo() -> Id {
    let p = crate::macos::dlsym(crate::macos::RTLD_DEFAULT, b"NSApp\0".as_ptr() as *const c_char) as *const Id;
    if p.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::read_volatile(p)
}

unsafe fn nsapp_do_jogo_existe() -> bool {
    !nsapp_do_jogo().is_null()
}

#[cfg(test)]
mod teste {
    /// PNG mínimo (filtro 0 por linha, deflate em bloco sem compressão). A amostra nasce no teste: a arte do splash é opcional e
    /// nenhum arquivo de fora do crate precisa existir para a fonte pública passar nos testes.
    pub(super) fn png(largura: u32, altura: u32, canais: u8, pixels: &[u8]) -> Vec<u8> {
        fn pedaco(saida: &mut Vec<u8>, tipo: &[u8; 4], dados: &[u8]) {
            saida.extend_from_slice(&(dados.len() as u32).to_be_bytes());
            let inicio = saida.len();
            saida.extend_from_slice(tipo);
            saida.extend_from_slice(dados);
            let crc = bwms_primitives::crc32(&saida[inicio..]);
            saida.extend_from_slice(&crc.to_be_bytes());
        }
        let mut cru = Vec::new();
        for linha in pixels.chunks_exact((largura * u32::from(canais)) as usize) {
            cru.push(0);
            cru.extend_from_slice(linha);
        }
        // um bloco "stored" leva até 65535 bytes
        assert!(cru.len() <= 0xFFFF);
        let mut zlib = vec![0x78, 0x01, 0x01];
        zlib.extend_from_slice(&(cru.len() as u16).to_le_bytes());
        zlib.extend_from_slice(&(!(cru.len() as u16)).to_le_bytes());
        zlib.extend_from_slice(&cru);
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &cru {
            a = (a + u32::from(x)) % 65521;
            b = (b + a) % 65521;
        }
        zlib.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&largura.to_be_bytes());
        ihdr.extend_from_slice(&altura.to_be_bytes());
        // 8 bits; tipo de cor 6 = RGBA, 2 = RGB
        ihdr.extend_from_slice(&[8, if canais == 4 { 6 } else { 2 }, 0, 0, 0]);
        let mut s = b"\x89PNG\r\n\x1a\n".to_vec();
        pedaco(&mut s, b"IHDR", &ihdr);
        pedaco(&mut s, b"IDAT", &zlib);
        pedaco(&mut s, b"IEND", &[]);
        s
    }

    /// Os dois layouts que a arte do splash pode ter (RGBA e RGB), em tamanhos diferentes, com alfa cheio para a volta do
    /// premultiplicado ser exata: o ImageIO tem de devolver RGBA8 com os mesmos pixels.
    #[test]
    fn imageio_le_as_amostras() {
        let pasta = std::env::temp_dir().join(format!("bwms-imageio-{}", std::process::id()));
        std::fs::create_dir_all(&pasta).expect("pasta temporária");
        for (l, alt, canais) in [(3u32, 2u32, 4u8), (5, 3, 3)] {
            let pixels: Vec<u8> = (0..l * alt * u32::from(canais))
                .map(|i| if canais == 4 && i % 4 == 3 { 255 } else { (i * 37 % 251) as u8 })
                .collect();
            let arquivo = pasta.join(format!("amostra-{canais}.png"));
            std::fs::write(&arquivo, png(l, alt, canais, &pixels)).expect("gravar a amostra");
            let (rgba, dl, da) = unsafe { super::decodificar_imagem(&arquivo) }.expect("ImageIO recusou o PNG");
            assert_eq!((dl, da), (l, alt), "{canais} canais: dimensão");
            let esperado: Vec<u8> = pixels
                .chunks_exact(usize::from(canais))
                .flat_map(|p| [p[0], p[1], p[2], if canais == 4 { p[3] } else { 255 }])
                .collect();
            assert_eq!(rgba, esperado, "{canais} canais: pixels");
        }
        let _ = std::fs::remove_dir_all(&pasta);
    }
}

/// Prova visual do terminal fora do jogo: a malha dos widgets (triângulos com uv e cor, antes da GPU) vai para o rasterizador de prova
/// em vez do Metal, sem depender de foto do jogo. Duas réguas: a impressão de cada caso no desenho aprovado pelo dono
/// (`terminal_igual_ao_aprovado`) e a distância até a referência em PNG (`terminal_contra_referencia`).
#[cfg(test)]
pub(crate) mod prova_visual {
    use super::*;
    use crate::terminal::rasterizador_de_prova::*;

    /// Fundo neutro e escuro no lugar do frame do jogo, o mesmo da referência.
    pub(crate) const FUNDO: [f32; 4] = [0.08, 0.08, 0.09, 1.0];

    /// Linhas de log de exemplo: respostas reais do terminal, com acento e sinais que o terminal usa.
    pub(crate) const LINHAS: [&str; 7] = [
        "BWMS 0.1.5 — terminal pronto.",
        "> money 5000",
        "[money] +5000 eddies (saldo: 12.480)",
        "> give Items.Preset_Katana_Saburo 1",
        "[give] 1x Items.Preset_Katana_Saburo -> inventário",
        "> godmode",
        "[godmode] invencível: ligado. Não esqueça de desligar (godmode off).",
    ];

    /// Título e linha da janela de plugin nos casos `plugin-*`: o que o `example-plugin` escreve a cada quadro, letra por letra
    /// (`example-plugin/src/lib.rs`, `example_draw_callback`). Texto trocado aqui muda as 5 impressões de plugin em `APROVADO`.
    const PLUGIN: (&str, &str) = ("Example Plugin (3rd-party, links nothing)", "Hello from example-plugin! (window via BwmsApi)");

    /// O que o `example-plugin` faz a cada quadro, pela C-ABI (abrir, uma linha, fechar), com a `Tela` ao alcance como o overlay faz.
    fn desenhar_plugin_de_exemplo(t: &mut crate::terminal::imediato::Tela<'_>) {
        use crate::mods_do_usuario::api::{api_janela_abrir, api_janela_fechar, api_janela_texto, com_tela_dos_plugins};
        let titulo = std::ffi::CString::new(PLUGIN.0).expect("título sem NUL");
        let linha = std::ffi::CString::new(PLUGIN.1).expect("linha sem NUL");
        com_tela_dos_plugins(t, || unsafe {
            api_janela_abrir(titulo.as_ptr());
            api_janela_texto(linha.as_ptr());
            api_janela_fechar();
        });
    }

    /// Um quadro de cenário: posição do mouse, botão esquerdo seguro e o texto digitado nele.
    pub(crate) type Passo = ([f32; 2], bool, &'static str);

    /// Log do teste: as linhas pedidas ou, sem elas, as de exemplo.
    fn linhas_do_log(linhas: &[&str]) -> Vec<String> {
        if linhas.is_empty() { LINHAS.iter() } else { linhas.iter() }.map(|s| s.to_string()).collect()
    }

    /// Linhas longas para a quebra de linha: espaço, ponto e vírgula sem espaço depois, caminho sem espaço nenhum, e espaços no começo
    /// da linha seguinte.
    pub(crate) const LINHAS_LONGAS: [&str; 4] = [
        "[give] 1x Items.Preset_Katana_Saburo -> inventário. Não esqueça de conferir o peso do inventário antes de pegar mais armas pesadas.",
        "[tweaks] base\\characters\\appearances\\player\\items\\clothing\\torso\\t2_jacket_aldecaldos_rich_rich.app,carregado,sem,erro",
        "a,b;c!d?e\"f.g,h;i!j?k\"l.m,n;o!p?q\"r.s,t;u!v?w\"x.y,z a,b;c!d?e\"f.g,h;i!j?k\"l.m,n;o!p?q\"r.s,t;u!v?w\"x.y,z fim",
        "palavra      com      muitos      espaços      entre      elas      para      ver      onde      a      linha      quebra      agora",
    ];

    /// Estado do terminal nos casos. Dois favoritos fora do catálogo de 6 itens: a aba Favoritos mostra a lista, e a Items não muda o
    /// botão `+pin` de nenhum item.
    pub(super) fn estado_de_exemplo(tema: usize, linhas: &[&str]) -> UiState {
        UiState {
            cmd_buf: String::new(),
            search_buf: String::new(),
            qty: 1,
            frame: 0,
            log_lines: linhas_do_log(linhas),
            favorites: vec![
                ("Items.Preset_Katana_Saburo".to_string(), "Katana Saburo".to_string()),
                ("Items.Preset_Overture_Kerry".to_string(), "Overture do Kerry".to_string()),
            ],
            fav_dirty: false,
            theme: tema,
            theme_dirty: false,
            cmd_history: Vec::new(),
            cmd_history_idx: None,
        }
    }

    /// O terminal desenhado pelos widgets do projeto depois de um cenário de mouse e teclado (cada passo é um quadro); o desenho é o do
    /// último passo. Com `escala` > 1 ele é montado em pixels lógicos (quadro ÷ escala) e ampliado por inteiro, como a projeção do
    /// overlay faz.
    pub(crate) fn terminal_proprio_cenario(largura: usize, altura: usize, tema: usize, aba: usize, cenario: &[Passo], linhas: &[&str], escala: usize, plugin: bool) -> Quadro {
        use crate::terminal::imediato::{Entrada, Persistente, Tela};
        use crate::terminal::malha_de_desenho::{montar_atlas, ATLAS_A, ATLAS_L};
        let mut st = estado_de_exemplo(tema, linhas);
        let mut p = Persistente::default();
        p.aba_ativa[0] = aba;
        // Três quadros parados: a rolagem do log assenta no fim a partir do segundo.
        let mut malha = None;
        for _ in 0..3 {
            let mut t = Tela::nova(Entrada::default(), &mut p, tema);
            montar_terminal(&mut t, &mut st);
            if plugin {
                desenhar_plugin_de_exemplo(&mut t);
            }
            malha = Some(t.terminar().malha);
        }
        // O cenário e mais 1 quadro no estado do último passo, sem digitar de novo. Quadro de 1/60 s; clique duplo = 2º aperto em até
        // 0,30 s e 6 px do 1º, e o 3º não conta (`CLIQUE_DUPLO_S` e `CLIQUE_DUPLO_PX`, os do render).
        let mut antes = false;
        let mut ultimo_aperto: Option<(usize, [f32; 2])> = None;
        let extra = cenario.last().map(|&(pos, apertado, _)| (pos, apertado, ""));
        for (quadro, (pos, apertado, digitado)) in cenario.iter().copied().chain(extra).enumerate() {
            let apertou = apertado && !antes;
            let clique_duplo = apertou
                && ultimo_aperto.is_some_and(|(q, p)| (quadro - q) as f32 / 60.0 < 0.30 && (pos[0] - p[0]).hypot(pos[1] - p[1]) <= 6.0);
            if apertou {
                ultimo_aperto = if clique_duplo { None } else { Some((quadro, pos)) };
            }
            let e = Entrada {
                mouse: pos,
                segurando: apertado,
                apertou,
                soltou: !apertado && antes,
                digitados: digitado.chars().collect(),
                clique_duplo,
                dt: 1.0 / 60.0,
                tamanho_da_tela: [(largura / escala) as f32, (altura / escala) as f32],
                ponteiro: true,
                ..Default::default()
            };
            antes = apertado;
            let mut t = Tela::nova(e, &mut p, tema);
            montar_terminal(&mut t, &mut st);
            if plugin {
                desenhar_plugin_de_exemplo(&mut t);
            }
            malha = Some(t.terminar().malha);
        }
        let m = malha.expect("três frames desenhados");
        let atlas = montar_atlas();
        // Fonte de bitmap escalada por inteiro: amostragem nítida (o `f_nearest` do overlay).
        let tex = Textura { largura: ATLAS_L, altura: ATLAS_A, rgba: &atlas, nitida: true };
        let e = escala as f32;
        let pontos: Vec<Ponto> = m.vertices.iter().map(|v| Ponto { pos: [v.pos[0] * e, v.pos[1] * e], uv: v.uv, cor: v.cor }).collect();
        let mut q = Quadro::novo(largura, altura, FUNDO);
        lista(&mut q, &pontos, &m.indices, &tex, [0.0, 0.0, largura as f32, altura as f32]);
        q
    }

    /// Centro da aba `i` com a janela na posição inicial (a conta é a do autoteste, `centro_da_aba_do_terminal`).
    fn centro_da_aba(i: usize) -> [f32; 2] {
        super::centro_da_aba_do_terminal(crate::terminal::imediato::JANELA_INICIAL, i)
    }

    /// K-LOG com dados (linhas capturadas) ou vazio. A lista da aba Mods vem de cada caso (`Caso::lista`).
    fn abas_com_dados(cheias: bool) {
        if let Ok(mut k) = KEY_LOG.lock() {
            k.clear();
            if cheias {
                k.extend(
                    ["[key] down kc=13 'w' mods=0", "[key] up kc=13 'w' mods=0", "[key] down kc=56 '' mods=shift", "[key] down kc=49 ' ' mods=0"]
                        .map(String::from),
                );
            }
        }
    }

    /// Um caso da prova: o quadro em pixels, o tema, a aba, o headset, o cenário de mouse e teclado, as linhas do log, a janela de
    /// plugin e a aba K-LOG com dados.
    struct Caso {
        nome: String,
        quadro: [usize; 2],
        tema: usize,
        aba: usize,
        headset: bool,
        cenario: Vec<Passo>,
        linhas: &'static [&'static str],
        plugin: bool,
        com_dados: bool,
        /// A lista da aba Mods do caso (`lista_dos_mods::exemplos`).
        lista: fn() -> crate::mods_do_usuario::lista_dos_mods::ListaDosMods,
    }

    const FORA: Passo = ([-100.0, -100.0], false, "");
    const QUADRO: [usize; 2] = [760, 660];

    /// Os casos, na ordem da tabela `APROVADO`: os cenários de mouse, teclado, escala e plugin no tema 0, e a matriz dos 4 temas × as 7
    /// abas em repouso (CPVR sem e com headset), que é também o conjunto da referência em PNG.
    fn casos() -> Vec<Caso> {
        let caso = |nome: &str, aba: usize, cenario: Vec<Passo>| Caso {
            nome: nome.to_string(),
            quadro: QUADRO,
            tema: 0,
            aba,
            headset: false,
            cenario,
            linhas: &[],
            plugin: false,
            com_dados: false,
            lista: crate::mods_do_usuario::lista_dos_mods::exemplos::vazia,
        };
        // Linha dos temas: y 177..196; ENTROPISM 68..139, KITSCH 147..197.
        let kitsch = [172.0, 186.0];
        // Primeiro Give da aba Items: x 68..104, y 255..274.
        let give = [86.0, 264.0];
        // Alça de redimensionar no canto da janela (60, 150, 660, 460).
        let alca = [716.0, 606.0];
        // Campo de comando do Console: x 68..712, y 583..602. "Items" no texto do campo-selecao começa em x = 68 + 4 + 5 × 7 = 107.
        let campo = [300.0, 592.0];
        let palavra = [120.0, 592.0];
        let focar = |texto: &'static str| vec![(campo, true, ""), (campo, false, ""), (campo, false, texto)];
        // O cursor apaga entre 1,1 e 1,5 s depois de digitar (aceso 0,30 s, depois 0,80 aceso e 0,40 apagado): 75 quadros dá 1,25 s.
        let mut apagado = focar("money 5000");
        apagado.extend(std::iter::repeat((campo, false, "")).take(75));
        let mut selecao = focar("give Items.Preset_Katana_Saburo 1");
        selecao.extend([(palavra, true, ""), (palavra, false, ""), (palavra, true, ""), (palavra, false, "")]);
        let mut v = vec![
            caso("dica-kitsch", 0, vec![(kitsch, false, "")]),
            caso("janela-sem-foco", 0, vec![([20.0, 640.0], true, ""), ([20.0, 640.0], false, ""), FORA]),
            caso("give-sobre", 1, vec![(give, false, "")]),
            caso("aba-sobre", 0, vec![(centro_da_aba(2), false, "")]),
            caso("janela-estreita", 0, vec![(alca, true, ""), ([400.0, 450.0], true, ""), ([300.0, 400.0], true, ""), ([300.0, 400.0], false, ""), FORA]),
            caso("janela-minima", 0, vec![(alca, true, ""), ([200.0, 300.0], true, ""), ([10.0, 10.0], true, ""), ([10.0, 10.0], false, ""), FORA]),
            caso("botao-apertado", 0, vec![(kitsch, true, "")]),
            caso("ponteiro-alca", 0, vec![(alca, false, "")]),
            caso("campo-com-texto", 0, focar("money 5000")),
            caso("campo-cursor-apagado", 0, apagado),
            caso("campo-selecao", 0, selecao),
            caso("campo-rolado", 0, focar("give Items.Preset_Katana_Saburo 1; give Items.Preset_Overture_Kerry 1; give Items.Preset_Lexington_Wilson 1; money 99999")),
            Caso { linhas: &LINHAS_LONGAS, ..caso("linhas-longas", 0, vec![FORA]) },
            // Escala automática: 1520×1320 dá 2× com a tela lógica do teste (760×660), na aba Mods onde ela se escolhe; 1920×1200 dá
            // 2× com tela lógica 960×600, e a janela sobe 10 px para caber inteira.
            Caso { quadro: [1520, 1320], ..caso("escala-2x-mods", 3, vec![FORA]) },
            Caso { quadro: [1920, 1200], ..caso("escala-2x-1920", 0, vec![FORA]) },
        ];
        // Janela do plugin sobre o terminal: nasce na frente com o foco; a seta dela recolhe; o clique no terminal devolve o foco a ele.
        let seta_do_plugin = [70.0, 69.0];
        for (nome, cenario) in [
            ("plugin-e-terminal", vec![FORA]),
            ("plugin-recolhido", vec![(seta_do_plugin, true, ""), (seta_do_plugin, false, ""), FORA]),
            ("plugin-terminal-no-foco", vec![([400.0, 400.0], true, ""), ([400.0, 400.0], false, ""), FORA]),
            // Barra do plugin arrastada para cima do corpo do terminal; depois o clique no terminal o põe na frente, cobrindo o plugin.
            ("plugin-sobre-o-terminal", vec![([200.0, 69.0], true, ""), ([200.0, 200.0], true, ""), ([200.0, 320.0], true, ""), ([200.0, 320.0], false, ""), FORA]),
            (
                "terminal-cobre-o-plugin",
                vec![([200.0, 69.0], true, ""), ([200.0, 320.0], true, ""), ([200.0, 320.0], false, ""), ([600.0, 500.0], true, ""), ([600.0, 500.0], false, ""), FORA],
            ),
        ] {
            v.push(Caso { plugin: true, ..caso(nome, 0, cenario) });
        }
        let abas = [
            (0, false, "console"),
            (1, false, "items"),
            (2, false, "favoritos"),
            (3, false, "mods"),
            (4, false, "klog"),
            (5, false, "cpvr"),
            (5, true, "cpvr-headset"),
            (6, false, "lut"),
        ];
        for tema in 0..4 {
            for (aba, headset, nome) in abas {
                v.push(Caso { tema, headset, com_dados: true, ..caso(&format!("tema{tema}-{nome}"), aba, vec![FORA]) });
            }
        }
        // A aba Mods com a lista dos 2 caminhos (a vazia é a `tema0-mods`): 1 mod, 1 de cada cor, os 2 caminhos com conflito, 100 e 10 mil
        // mods, nome e motivo longos.
        {
            use crate::mods_do_usuario::lista_dos_mods::exemplos;
            let listas: [(&str, fn() -> crate::mods_do_usuario::lista_dos_mods::ListaDosMods); 6] = [
                ("mods-um", exemplos::um),
                ("mods-tres-cores", exemplos::tres_cores),
                ("mods-dois-caminhos", exemplos::dois_caminhos),
                ("mods-cem", || exemplos::muitos(100)),
                ("mods-dez-mil", || exemplos::muitos(10_000)),
                ("mods-nome-longo", exemplos::nome_longo),
            ];
            for (nome, lista) in listas {
                v.push(Caso { lista, ..caso(nome, 3, vec![FORA]) });
            }
        }
        v
    }

    /// O caso desenhado pelo terminal, com os estáticos que ele lê postos para o caso (K-LOG, lista da aba Mods, contadores dos plugins,
    /// headset, escala do quadro): a ordem dos casos e dos testes não muda o desenho.
    fn desenhar(c: &Caso) -> Quadro {
        abas_com_dados(c.com_dados);
        crate::mods_do_usuario::lista_dos_mods::publicar((c.lista)());
        // A aba Mods escreve "plugins N" e as partes do selo com os contadores do processo; o teste do `register_mod` soma 7 neles. As
        // imagens aprovadas são com 0 plugins.
        crate::mods_do_usuario::trocar_contagem_dos_plugins([0; 4]);
        crate::vr::painel::headset_para_teste(c.headset);
        let n = crate::terminal::escala::deste_quadro([c.quadro[0] as f32, c.quadro[1] as f32]) as usize;
        terminal_proprio_cenario(c.quadro[0], c.quadro[1], c.tema, c.aba, &c.cenario, c.linhas, n, c.plugin)
    }

    /// Devolve os estáticos ao estado de fora da prova: K-LOG vazio, os contadores dos plugins de antes do teste, sem headset, escala do
    /// quadro de teste.
    fn devolver_estaticos(plugins_de_fora: [usize; 4]) {
        abas_com_dados(false);
        crate::mods_do_usuario::lista_dos_mods::publicar(crate::mods_do_usuario::lista_dos_mods::ListaDosMods::vazia());
        crate::mods_do_usuario::trocar_contagem_dos_plugins(plugins_de_fora);
        crate::vr::painel::headset_para_teste(false);
        crate::terminal::escala::deste_quadro([QUADRO[0] as f32, QUADRO[1] as f32]);
    }

    /// Impressão (`rasterizador_de_prova::impressao`) de cada caso, na ordem de `casos()`: o desenho aprovado pelo dono
    /// (estética mantida, 2026-10-05). Desenho novo entra com o sim dele, e a impressão nova entra aqui junto.
    const APROVADO: [(&str, u64); 58] = [
        ("dica-kitsch", 0x198160f7993055b6),
        ("janela-sem-foco", 0x43dbf851222e7bd2),
        ("give-sobre", 0xe1b30d9bad09f8d5),
        ("aba-sobre", 0xa1bedc3911b35a19),
        ("janela-estreita", 0xb82782e6bdb035de),
        ("janela-minima", 0xb82782e6bdb035de),
        ("botao-apertado", 0x46c58c5c190ed3ef),
        ("ponteiro-alca", 0x47f2ad7cb87935c2),
        ("campo-com-texto", 0x4617d341b0016c1e),
        ("campo-cursor-apagado", 0x954e3eaf7a9572f0),
        ("campo-selecao", 0xc9167d0b46cc918a),
        ("campo-rolado", 0xedb25a6657b7a79c),
        ("linhas-longas", 0xf10679245c215c82),
        ("escala-2x-mods", 0x893dd7600520958e),
        ("escala-2x-1920", 0xaacead9e56d9b584),
        // As 5 de plugin: o desenho é o aprovado; o título, a linha e a largura da janela, que segue o texto, são os de `PLUGIN`.
        ("plugin-e-terminal", 0x1fdc47dd331ffee5),
        ("plugin-recolhido", 0xbd866331fb13b976),
        ("plugin-terminal-no-foco", 0x5a06737ef790e32c),
        ("plugin-sobre-o-terminal", 0xc9a4dc9e089d1535),
        ("terminal-cobre-o-plugin", 0x9f44f81cefcfc222),
        ("tema0-console", 0xb82782e6bdb035de),
        ("tema0-items", 0xc74e32a0d5dc8f35),
        ("tema0-favoritos", 0xb66b85d93b56f1ab),
        ("tema0-mods", 0xca66ada83ccdec63),
        ("tema0-klog", 0xeeef441f2cab4679),
        ("tema0-cpvr", 0xa15577a6ec0cf639),
        ("tema0-cpvr-headset", 0x98a5949e9e6303fa),
        ("tema0-lut", 0xc3f5875145fe9932),
        ("tema1-console", 0xbc68ae09b5862197),
        ("tema1-items", 0x8ab3ea2c54e4fa3a),
        ("tema1-favoritos", 0x9d9791f3b2854d31),
        ("tema1-mods", 0x0220a9720c0fcac2),
        ("tema1-klog", 0x7f1d11dc4ed322da),
        ("tema1-cpvr", 0x7d1b1ceeac78a5a5),
        ("tema1-cpvr-headset", 0x4e88ec762dccaef8),
        ("tema1-lut", 0x8eed5a8f7595d708),
        ("tema2-console", 0x4c7f485b32243255),
        ("tema2-items", 0xaa117f0aa11c85c9),
        ("tema2-favoritos", 0x96911a80b398b268),
        ("tema2-mods", 0x046aea2d8bc8f235),
        ("tema2-klog", 0xf41e2904302a75bb),
        ("tema2-cpvr", 0x9773e30ee97c5f0c),
        ("tema2-cpvr-headset", 0x7d28048bb76124dc),
        ("tema2-lut", 0xa8894603535c4232),
        ("tema3-console", 0x3845f10425db3262),
        ("tema3-items", 0xebc02b0261ce0f64),
        ("tema3-favoritos", 0xe53fd0d30ebfc326),
        ("tema3-mods", 0x528972324ce4d5d3),
        ("tema3-klog", 0xffdb2075fd7cf9b1),
        ("tema3-cpvr", 0x652bfa4a02d56379),
        ("tema3-cpvr-headset", 0x95544ec541a86bd9),
        ("tema3-lut", 0xe97c45fc24b40f66),
        // A lista da aba Mods (etapa 3), aprovada pelo dono em 2026-10-24 (imagens em `z031/relatorios/M-prova-visual/`). Estas 6 e
        // as 5 `tema*-mods`/`escala-2x-mods` são com 0 plugins: `desenhar` zera os contadores dos plugins antes de cada caso.
        ("mods-um", 0x3e1c52a11e3c8809),
        ("mods-tres-cores", 0xcc7bfb5059e385c8),
        ("mods-dois-caminhos", 0x63046f426cf8eef0),
        ("mods-cem", 0x13d62eacfe344277),
        ("mods-dez-mil", 0x47367e078fcc044a),
        ("mods-nome-longo", 0x8f0e5d2a7220f74e),
    ];

    /// Pasta das imagens de revisão (`BWMS_PROVA_VISUAL=<pasta>`); sem ela, a prova só mede.
    fn pasta_de_revisao() -> Option<std::path::PathBuf> {
        std::env::var_os("BWMS_PROVA_VISUAL").map(std::path::PathBuf::from)
    }

    /// Qualquer pixel mudado em qualquer caso quebra: o desenho do terminal é o que o dono aprovou. Com `BWMS_PROVA_VISUAL=<pasta>`,
    /// grava a imagem de cada caso para revisão.
    #[test]
    fn terminal_igual_ao_aprovado() {
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        let plugins_de_fora = crate::mods_do_usuario::trocar_contagem_dos_plugins([0; 4]);
        let casos = casos();
        assert_eq!(casos.len(), APROVADO.len(), "um caso sem impressão aprovada, ou o contrário");
        let revisao = pasta_de_revisao();
        let mut mudados = Vec::new();
        for (c, (nome, aprovada)) in casos.iter().zip(APROVADO) {
            assert_eq!(c.nome, nome, "a tabela segue a ordem de `casos()`");
            let q = desenhar(c);
            if let Some(dir) = &revisao {
                gravar_bmp(&dir.join(format!("terminal-{nome}.bmp")), &q).expect("gravar a imagem de revisão");
            }
            let agora = impressao(&q);
            if agora != aprovada {
                mudados.push(format!("{nome}: {agora:#018x} (aprovado {aprovada:#018x})"));
            }
        }
        devolver_estaticos(plugins_de_fora);
        assert!(
            mudados.is_empty(),
            "o desenho do terminal mudou em {} caso(s). Grave as imagens com BWMS_PROVA_VISUAL=<pasta>, mostre ao dono e, com o sim dele, \
             troque a impressão em APROVADO:\n{}",
            mudados.len(),
            mudados.join("\n")
        );
    }

    /// A pasta da referência em PNG (`cp77-symbols/notes/proofs/20261003-terminal-referencia-…`), subindo a partir do crate: ele fica
    /// em `mods-research/` ou numa cópia de trabalho abaixo dele. `None` fora da árvore do projeto, onde as provas não vão.
    fn pasta_da_referencia() -> Option<std::path::PathBuf> {
        let provas = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .map(|d| d.join("cp77-symbols/notes/proofs"))
            .find(|p| p.is_dir())?;
        std::fs::read_dir(&provas)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("20261003-terminal-referencia-")))
    }

    /// O PNG da referência de um caso da matriz: o que termina em `-<caso>.png` (`…-tema0-console.png`).
    fn png_da_referencia(pasta: &std::path::Path, caso: &str) -> Option<std::path::PathBuf> {
        let fim = format!("-{caso}.png");
        std::fs::read_dir(pasta)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&fim)))
    }

    /// Teto da distância até a referência, em % da janela (660×460): a fonte é a do projeto (7×13), não a da referência, então o traço
    /// das letras sempre difere — o pior caso medido é o Console, com 1,5%. Layout fora do lugar passa disso.
    const TETO_CONTRA_A_REFERENCIA: f64 = 2.0;

    /// O terminal contra a referência em PNG, nos 28 casos da matriz fora a aba Mods (limiar 0,10 por canal): mede, imprime e falha acima
    /// do teto. A aba Mods mudou de conteúdo (mods da `Mods/` e do mod.io com luz, aprovado pelo dono em 2026-10-24) e a referência ImGui
    /// mostra a lista antiga; o estilo dela segue conferido pelos outros 7 casos de cada tema e o desenho novo, pelas impressões em
    /// `APROVADO`. Sem a pasta das provas (fora da árvore do projeto), pula. Com `BWMS_PROVA_VISUAL=<pasta>`, grava referência | terminal e
    /// o mapa.
    #[test]
    fn terminal_contra_referencia() {
        let Some(pasta) = pasta_da_referencia() else {
            println!("[referencia] sem a pasta das provas (cp77-symbols/notes/proofs): pulado");
            return;
        };
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        let plugins_de_fora = crate::mods_do_usuario::trocar_contagem_dos_plugins([0; 4]);
        let revisao = pasta_de_revisao();
        let janela = (crate::terminal::imediato::JANELA_INICIAL[2] * crate::terminal::imediato::JANELA_INICIAL[3]) as f64;
        let mut medidos = 0;
        let mut acima = Vec::new();
        for c in casos().iter().filter(|c| c.nome.starts_with("tema") && !c.nome.ends_with("-mods")) {
            let png = png_da_referencia(&pasta, &c.nome).unwrap_or_else(|| panic!("falta o PNG de {} em {}", c.nome, pasta.display()));
            let (rgba, l, a) = unsafe { decodificar_imagem(&png) }.unwrap_or_else(|| panic!("o ImageIO não leu {}", png.display()));
            let referencia = Quadro {
                largura: l as usize,
                altura: a as usize,
                px: rgba.chunks_exact(4).map(|p| [p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0]).collect(),
            };
            assert_eq!([referencia.largura, referencia.altura], c.quadro, "{}: a referência tem outro tamanho", c.nome);
            let q = desenhar(c);
            let (mapa, diferentes) = diferenca(&referencia, &q, 0.10);
            let pct = diferentes as f64 / janela * 100.0;
            println!("[referencia] {:20} {diferentes:5} px = {pct:.1}% da janela", c.nome);
            if let Some(dir) = &revisao {
                gravar_bmp(&dir.join(format!("referencia-e-terminal-{}.bmp", c.nome)), &lado_a_lado(&referencia, &q, 12, [0.55, 0.10, 0.10, 1.0])).expect("bmp");
                gravar_bmp(&dir.join(format!("referencia-diferenca-{}.bmp", c.nome)), &mapa).expect("bmp");
            }
            if pct > TETO_CONTRA_A_REFERENCIA {
                acima.push(format!("{}: {pct:.1}%", c.nome));
            }
            medidos += 1;
        }
        devolver_estaticos(plugins_de_fora);
        assert_eq!(medidos, 28, "a matriz tem 4 temas × 8 casos, sem a aba Mods");
        assert!(
            acima.is_empty(),
            "o terminal passou de {TETO_CONTRA_A_REFERENCIA}% de diferença da referência em: {}. Grave as imagens com BWMS_PROVA_VISUAL=<pasta> e \
             confira o layout desses casos.",
            acima.join(", ")
        );
    }

    /// A aba Mods com 0, 1, 100 e 10 mil mods e com nome longo: a janela não muda de lugar nem de tamanho, nada é desenhado fora da caixa que
    /// a aba vazia ocupa, e o tamanho da malha não cresce com o número de mods acima do teto por caminho. Imprime vértices e tempo do quadro.
    #[test]
    fn aba_mods_aguenta_0_1_100_10mil_e_nome_longo() {
        use crate::mods_do_usuario::lista_dos_mods::{exemplos, publicar, ListaDosMods};
        use crate::terminal::imediato::{Entrada, Persistente, Tela, JANELA_INICIAL};
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        let plugins_de_fora = crate::mods_do_usuario::trocar_contagem_dos_plugins([0; 4]);
        let quadro = |lista: ListaDosMods| {
            publicar(lista);
            let mut st = estado_de_exemplo(0, &[]);
            let mut p = Persistente::default();
            p.aba_ativa[0] = 3;
            let mut malha = None;
            let t0 = std::time::Instant::now();
            for _ in 0..3 {
                let e = Entrada { mouse: [-100.0, -100.0], dt: 1.0 / 60.0, tamanho_da_tela: [QUADRO[0] as f32, QUADRO[1] as f32], ..Default::default() };
                let mut t = Tela::nova(e, &mut p, 0);
                montar_terminal(&mut t, &mut st);
                malha = Some(t.terminar().malha);
            }
            let us = t0.elapsed().as_micros() / 3;
            let m = malha.expect("três quadros");
            let caixa = m.vertices.iter().fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |c, v| {
                [c[0].min(v.pos[0]), c[1].min(v.pos[1]), c[2].max(v.pos[0]), c[3].max(v.pos[1])]
            });
            (p.janelas[0].ret, caixa, m.vertices.len(), us)
        };
        let (janela_vazia, caixa_vazia, _, _) = quadro(exemplos::vazia());
        assert_eq!(janela_vazia, JANELA_INICIAL);
        let mut vertices = Vec::new();
        let casos: [(&str, fn() -> ListaDosMods); 5] =
            [("0", exemplos::vazia), ("1", exemplos::um), ("100", || exemplos::muitos(100)), ("10k", || exemplos::muitos(10_000)), ("nome longo", exemplos::nome_longo)];
        for (nome, lista) in casos {
            let (janela, caixa, n, us) = quadro(lista());
            println!("[aba-mods] {nome:10} {n:6} vértices, quadro {us} us (build de teste), caixa {caixa:?}");
            assert_eq!(janela, JANELA_INICIAL, "{nome}: a janela mudou");
            assert!(
                caixa[0] >= caixa_vazia[0] && caixa[1] >= caixa_vazia[1] && caixa[2] <= caixa_vazia[2] && caixa[3] <= caixa_vazia[3],
                "{nome}: desenhou fora da caixa da aba vazia: {caixa:?} contra {caixa_vazia:?}"
            );
            vertices.push(n);
        }
        devolver_estaticos(plugins_de_fora);
        assert!(vertices[3] <= vertices[2] * 4, "10 mil mods montam no máximo o teto por caminho: {vertices:?}");
    }
}

/// A fiação do terminal no render: entrada do quadro, teclado em ordem, e o autoteste inteiro pelo mesmo `montar_quadro` do produto.
#[cfg(test)]
mod fiacao_tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// Área de transferência que não toca a do sistema (o teste não pode apagar o que o dono copiou).
    struct AreaFalsa(Option<String>);
    impl AreaDeTransferencia for AreaFalsa {
        fn ler(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn gravar(&mut self, texto: &str) {
            self.0 = Some(texto.to_string());
        }
    }

    #[test]
    fn teclas_do_terminal_cobrem_a_edicao_e_o_historico() {
        // As 5 que o D1 pediu (Enter numérico, Esc, Tab, Home, End) e as que o campo e o ↑/↓ já tratavam.
        for k in [76, 53, 48, 115, 119, 36, 51, 117, 123, 124, 125, 126] {
            assert!(TECLAS_DO_TERMINAL.contains(&k), "keyCode {k} não vai ao terminal como tecla");
        }
        assert_eq!(LETRAS_DE_ATALHO, [0, 7, 8, 9], "Cmd+A, X, C, V pelo keyCode do hardware");
    }

    /// O `sendEvent` lê o char da tecla emprestado do `UTF8String` (keyDown) e só o 1º char (keyUp). O mapeamento para EInputKey,
    /// o atalho padrão (string inteira igual, nunca prefixo) e a linha do K-LOG saem iguais aos da cópia em `String`.
    #[test]
    fn char_da_tecla_sem_copia_mapeia_e_registra_igual() {
        use crate::entrada::map_macos_keycode_to_einputkey as mapear;
        let casos: [(u16, &str, i32, bool); 9] = [
            (50, "`", 0, true),
            (10, "~", 0, true),
            (0, "a", 65, false),
            (0, "A", 65, false),
            (18, "1", 49, false),
            (49, " ", 32, false),
            (49, "", 32, false),
            (122, "\u{F704}", 112, false),
            (50, "`a", 0, false),
        ];
        for (kc, texto, tecla, atalho_padrao) in casos {
            let c = CString::new(texto).expect("texto sem NUL");
            let ch: &str = unsafe { CStr::from_ptr(c.as_ptr()) }.to_str().expect("UTF-8");
            let copia = ch.to_string();
            assert_eq!(mapear(kc as i32, ch.chars().next()), tecla, "keyDown {texto:?}");
            assert_eq!(mapear(kc as i32, copia.chars().next()), tecla, "cópia {texto:?}");
            let ch_up: Option<char> = ch.chars().next();
            assert_eq!(mapear(kc as i32, ch_up), tecla, "keyUp {texto:?}");
            assert_eq!(ch == "`" || ch == "~", atalho_padrao, "atalho padrão {texto:?}");
            assert_eq!(copia == "`" || copia == "~", atalho_padrao, "atalho padrão da cópia {texto:?}");
            assert_eq!(format!("[key] keyCode={kc} char={ch:?}"), format!("[key] keyCode={kc} char={copia:?}"), "K-LOG {texto:?}");
        }
        let ch: &str = "`";
        assert_eq!(format!("[key] keyCode={} char={ch:?}", 50), "[key] keyCode=50 char=\"`\"");
    }

    #[test]
    fn aperto_soltura_e_clique_duplo_saem_do_quadro_anterior() {
        let t0 = Instant::now();
        let mut a = EntradaDoQuadroAnterior::default();
        let mut quadro = |ms: u64, segurando: bool, m: [f32; 2]| a.entrada(m, segurando, Vec::new(), [760.0, 660.0], t0 + Duration::from_millis(ms));
        let e = quadro(0, true, [10.0, 10.0]);
        assert!(e.apertou && !e.soltou && !e.clique_duplo);
        assert_eq!(e.dt, 1.0 / 60.0, "o 1º quadro conta como 1/60 s");
        assert!(e.ponteiro && e.roda == 0.0, "ponteiro desenhado; a roda segue para o jogo");
        let e = quadro(16, true, [10.0, 10.0]);
        assert!(!e.apertou && e.segurando);
        assert!((e.dt - 0.016).abs() < 1e-6);
        assert!(quadro(32, false, [10.0, 10.0]).soltou);
        let e = quadro(200, true, [12.0, 13.0]);
        assert!(e.apertou && e.clique_duplo, "2º aperto em 0,2 s e 3,6 px");
        quadro(220, false, [12.0, 13.0]);
        assert!(!quadro(240, true, [12.0, 13.0]).clique_duplo, "o 3º aperto conta de novo como o 1º");
        quadro(260, false, [12.0, 13.0]);
        let e = quadro(400, true, [30.0, 13.0]);
        assert!(e.apertou && !e.clique_duplo, "longe demais do anterior");
        let e = quadro(5000, false, [30.0, 13.0]);
        assert_eq!(e.dt, DT_MAXIMO, "terminal parado não vira rajada de repetição");
    }

    #[test]
    fn teclado_chega_ao_campo_na_ordem() {
        let mut ui = prova_visual::estado_de_exemplo(1, &[]);
        let mut p = Persistente { foco: Some(0), edicao: Some(crate::terminal::edicao::Edicao::comecar("")), ..Default::default() };
        let eventos = vec![Evento::Caractere('a'), Evento::Tecla(tecla::BACKSPACE, Mods::default()), Evento::Caractere('b')];
        let mut anterior = EntradaDoQuadroAnterior::default();
        let e = anterior.entrada([-100.0, -100.0], false, eventos, [760.0, 660.0], Instant::now());
        assert_eq!(e.teclas, vec![tecla::BACKSPACE]);
        let f = montar_quadro(&mut ui, &mut p, e, &mut AreaFalsa(None));
        assert_eq!(ui.cmd_buf, "b", "a, ⌫, b no mesmo quadro");
        assert!(f.quer_teclado, "edição em curso: as teclas não vão ao jogo");
    }

    #[test]
    fn autoteste_passa_pela_fiacao_do_quadro() {
        let _trava = crate::resposta_ao_usuario::UM_POR_VEZ.lock();
        let mut ui = prova_visual::estado_de_exemplo(1, &[]);
        // Campo com texto de antes: o `help` do autoteste o troca inteiro.
        ui.cmd_buf = "money 5000".to_string();
        let mut p = Persistente::default();
        let mut anterior = EntradaDoQuadroAnterior::default();
        let mut area = AreaFalsa(None);
        let mut a = Autoteste::novo();
        let t0 = Instant::now();
        for q in 0..12u64 {
            let mut e = anterior.entrada([-100.0, -100.0], false, Vec::new(), [760.0, 660.0], t0 + Duration::from_millis(16 * q));
            a.preparar(&mut e, p.janelas[0].ret, &ui);
            let f = montar_quadro(&mut ui, &mut p, e, &mut area);
            if let Some(v) = a.conferir(&ui, &p, &f) {
                assert!(v.ends_with(">>> TERMINAL-AUTOTESTE OK <<<"), "{v}");
                assert!(v.contains("enter: +9 linhas"), "{v}");
                return;
            }
        }
        panic!("o autoteste não terminou em 12 quadros");
    }

    #[test]
    fn ponto_do_termclique_aceita_espaco_ou_virgula() {
        assert_eq!(ponto_do_comando("122.5 340"), Some([122.5, 340.0]));
        assert_eq!(ponto_do_comando("122.5,340.5"), Some([122.5, 340.5]), "o centro do termonde, colado como veio");
        assert_eq!(ponto_do_comando(" 10 , 20 "), Some([10.0, 20.0]));
        for errado in ["", "10", "10 x", "10 20 30", "NaN 1", "inf 2"] {
            assert_eq!(ponto_do_comando(errado), None, "{errado:?}");
        }
    }

    /// Um pedido de dev atendido como o render atende: o passo põe o mouse no estado do `sendEvent` (escala 2×), a entrada sai dele, a
    /// procura vai na entrada e o veredito sai do quadro montado.
    fn atender(mut pedido: PedidoDeDev, ui: &mut UiState, p: &mut Persistente, anterior: &mut EntradaDoQuadroAnterior, quadro: &mut u32) -> String {
        let n = 2.0;
        let t0 = Instant::now();
        for _ in 0..8 {
            *quadro += 1;
            if let Some((ponto, segurando)) = pedido.mouse() {
                por_o_mouse(ponto, segurando, n);
            }
            let (mouse, segurando) = mouse_do_sendevent(n);
            let mut e = anterior.entrada(mouse, segurando, Vec::new(), [760.0, 660.0], t0 + Duration::from_millis(16 * u64::from(*quadro)));
            e.procura = pedido.procura(e.mouse);
            let f = montar_quadro(ui, p, e, &mut AreaFalsa(None));
            if let Some(v) = pedido.depois_do_quadro(*quadro, p.aba_ativa[0], &f) {
                assert!(!v.contains('\n'), "veredito numa linha só: {v}");
                return v;
            }
        }
        panic!("o pedido não terminou em 8 quadros");
    }

    /// O `centro x,y` do primeiro item do veredito do `termonde`, como o integrador o copia para o `termclique`.
    fn centro_do_veredito(v: &str) -> [f32; 2] {
        let resto = v.split_once(" centro ").unwrap_or_else(|| panic!("sem centro: {v}")).1;
        ponto_do_comando(resto.split(' ').next().unwrap_or("")).unwrap_or_else(|| panic!("centro ilegível: {v}"))
    }

    /// O roteiro da aba Mods no jogo, offline: a aba pelo rótulo e o clique nela, o texto que aponta a `Mods/`, depois a caixa do selo
    /// ligada e desligada por dois cliques, com o estado que cada `termonde` lê do quadro.
    #[test]
    fn termonde_e_termclique_levam_a_aba_mods_e_alternam_a_caixa() {
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        let selo_antes = BADGE_ENABLED.swap(true, Ordering::Relaxed);
        let mut ui = prova_visual::estado_de_exemplo(1, &[]);
        let mut p = Persistente::default();
        let mut anterior = EntradaDoQuadroAnterior::default();
        let mut q = 0u32;
        let onde = |r: &str| PedidoDeDev::Onde { rotulo: r.to_string(), passo: PassoDoOnde::Espera };
        let clique = |ponto: [f32; 2]| PedidoDeDev::Clique { ponto, passo: PassoDoClique::Leva };

        let v = atender(onde("Mods"), &mut ui, &mut p, &mut anterior, &mut q);
        assert!(v.starts_with("[terminal-onde] \"Mods\" na aba Console: 1 item(ns) | aba \"Mods\" em ["), "{v}");
        assert!(v.contains("visível=sim ativa=não") && v.ends_with(">>> TERMINAL-ONDE OK <<<"), "{v}");
        let aba = centro_do_veredito(&v);
        assert_eq!(aba, centro_da_aba_do_terminal(p.janelas[0].ret, 3), "a conta do autoteste e o desenho dão o mesmo centro");

        let v = atender(clique(aba), &mut ui, &mut p, &mut anterior, &mut q);
        assert_eq!(p.aba_ativa[0], 3);
        assert!(v.contains("(aba Mods depois da soltura)") && v.contains("terminal pegou o mouse=sim"), "{v}");
        assert!(v.contains("sob o ponto: aba \"Mods\"") && v.ends_with(">>> TERMINAL-CLIQUE OK <<<"), "{v}");

        let v = atender(onde(TEXTO_DA_ABA_MODS), &mut ui, &mut p, &mut anterior, &mut q);
        assert!(v.contains("visível=sim") && v.ends_with(">>> TERMINAL-ONDE OK <<<"), "{v}");

        let rotulo_do_selo = "Exibir badge (canto superior direito)";
        let v = atender(onde(rotulo_do_selo), &mut ui, &mut p, &mut anterior, &mut q);
        assert!(v.contains("caixa \"Exibir badge (canto superior direito)\"") && v.contains("marcada=sim"), "{v}");
        let caixa = centro_do_veredito(&v);
        for (marcada_antes, ligado_depois) in [("marcada=sim", false), ("marcada=não", true)] {
            let v = atender(clique(caixa), &mut ui, &mut p, &mut anterior, &mut q);
            assert!(v.contains(marcada_antes) && v.ends_with(">>> TERMINAL-CLIQUE OK <<<"), "{v}");
            assert_eq!(BADGE_ENABLED.load(Ordering::Relaxed), ligado_depois, "{v}");
        }
        let v = atender(onde(rotulo_do_selo), &mut ui, &mut p, &mut anterior, &mut q);
        assert!(v.contains("marcada=sim"), "o 2º clique devolveu a caixa: {v}");

        let v = atender(clique([5.0, 5.0]), &mut ui, &mut p, &mut anterior, &mut q);
        assert!(v.contains("terminal pegou o mouse=não | sob o ponto: nada | TERMINAL-CLIQUE FORA DO TERMINAL"), "{v}");
        let v = atender(onde("Reiniciar"), &mut ui, &mut p, &mut anterior, &mut q);
        assert_eq!(v, "[terminal-onde] \"Reiniciar\" na aba Mods: nenhum item com esse rótulo | TERMINAL-ONDE NADA");
        BADGE_ENABLED.store(selo_antes, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod vista_do_log_tests {
    use super::ultimas_linhas_do_log;

    fn arquivo(nome: &str, conteudo: &str) -> String {
        let p = std::env::temp_dir().join(nome);
        std::fs::write(&p, conteudo).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn mostra_so_o_que_veio_depois_do_corte() {
        let p = arquivo("bwms-vista-do-log-1.log", "a\nb\nc\nd\n");
        assert_eq!(ultimas_linhas_do_log(&p, 4, 10), vec!["c", "d"]);
        assert_eq!(ultimas_linhas_do_log(&p, 0, 2), vec!["c", "d"]);
    }

    #[test]
    fn corte_no_meio_da_linha_tira_a_linha_cortada() {
        let p = arquivo("bwms-vista-do-log-2.log", "abc\ndef\n");
        assert_eq!(ultimas_linhas_do_log(&p, 1, 10), vec!["def"]);
    }

    #[test]
    fn corte_alem_do_fim_volta_ao_inicio() {
        let p = arquivo("bwms-vista-do-log-3.log", "x\ny\n");
        assert_eq!(ultimas_linhas_do_log(&p, 999, 10), vec!["x", "y"]);
    }

    #[test]
    fn janela_de_64k_tira_a_linha_cortada_do_comeco() {
        let p = arquivo("bwms-vista-do-log-4.log", &format!("{}\nfim\n", "z".repeat(70 * 1024)));
        assert_eq!(ultimas_linhas_do_log(&p, 0, 10), vec!["fim"]);
    }
}

/// O caminho Metal do produto (splash, selo e filtro de cor) desenhado na GPU desta máquina e lido de volta, sem o jogo: o drawable é
/// um `MTLRenderPassColorAttachmentDescriptor` (responde a `texture` como o `CAMetalDrawable`), a textura nasce com um gradiente no lugar
/// do quadro do jogo e o command buffer sai de uma fila nova. A impressão de cada caso é o contrato de que a ponte ObjC manda as mesmas
/// mensagens. O terminal fica de fora: aberto, ele chama `CGAssociateMouseAndMouseCursorPosition`, que é estado do WindowServer.
/// Rodar sozinho (muda o diretório corrente para achar a arte): `BWMS_PROVA_VISUAL=<pasta> cargo test caminho_metal_desenha -- --ignored
/// --nocapture --test-threads=1`
#[cfg(test)]
mod prova_metal {
    use super::*;
    use crate::terminal::rasterizador_de_prova::{gravar_bmp, impressao, Quadro};

    const LARGURA: u64 = 1280;
    const ALTURA: u64 = 720;
    /// `MTLPixelFormatBGRA8Unorm`.
    const BGRA8: u64 = 80;

    #[repr(C)]
    struct Regiao {
        origem: [u64; 3],
        tamanho: [u64; 3],
    }

    unsafe fn mandar_u64(r: Id, s: Sel, v: u64) {
        let f: extern "C" fn(Id, Sel, u64) = std::mem::transmute(objc_msgSend as *const c_void);
        f(r, s, v)
    }

    /// Textura `LARGURA`×`ALTURA` BGRA8 compartilhada, com um gradiente no lugar do quadro do jogo.
    unsafe fn textura_com_gradiente(dev: Id) -> Id {
        let desc = msg0(class("MTLTextureDescriptor") as Id, sel!("new"));
        mandar_u64(desc, sel!("setPixelFormat:"), BGRA8);
        mandar_u64(desc, sel!("setWidth:"), LARGURA);
        mandar_u64(desc, sel!("setHeight:"), ALTURA);
        mandar_u64(desc, sel!("setStorageMode:"), 0);
        mandar_u64(desc, sel!("setUsage:"), 1 | 4);
        let tex = msg1(dev, sel!("newTextureWithDescriptor:"), desc);
        msg0(desc, sel!("release"));
        let (l, a) = (LARGURA as usize, ALTURA as usize);
        let mut px = vec![0u8; l * a * 4];
        for y in 0..a {
            for x in 0..l {
                let o = (y * l + x) * 4;
                px[o..o + 4].copy_from_slice(&[((x ^ y) & 255) as u8, (y * 255 / a) as u8, (x * 255 / l) as u8, 255]);
            }
        }
        let f: extern "C" fn(Id, Sel, Regiao, u64, *const c_void, u64) = std::mem::transmute(objc_msgSend as *const c_void);
        let regiao = Regiao { origem: [0; 3], tamanho: [LARGURA, ALTURA, 1] };
        f(tex, sel!("replaceRegion:mipmapLevel:withBytes:bytesPerRow:"), regiao, 0, px.as_ptr() as *const c_void, LARGURA * 4);
        tex
    }

    /// A textura lida de volta, em RGBA. `bgra`: a ordem dos bytes na textura.
    unsafe fn ler(tex: Id, largura: u64, altura: u64, bgra: bool) -> Quadro {
        let mut bytes = vec![0u8; (largura * altura * 4) as usize];
        let f: extern "C" fn(Id, Sel, *mut c_void, u64, Regiao, u64) = std::mem::transmute(objc_msgSend as *const c_void);
        let regiao = Regiao { origem: [0; 3], tamanho: [largura, altura, 1] };
        f(tex, sel!("getBytes:bytesPerRow:fromRegion:mipmapLevel:"), bytes.as_mut_ptr() as *mut c_void, largura * 4, regiao, 0);
        let px = bytes
            .chunks_exact(4)
            .map(|p| {
                let (r, b) = if bgra { (p[2], p[0]) } else { (p[0], p[2]) };
                [r as f32 / 255.0, p[1] as f32 / 255.0, b as f32 / 255.0, p[3] as f32 / 255.0]
            })
            .collect();
        Quadro { largura: largura as usize, altura: altura as usize, px }
    }

    /// A textura da arte do splash lida direto: a malha do splash pinta o fundo opaco por cima dela, então a imagem final não a mostra.
    unsafe fn arte_lida(dev: Id) -> Quadro {
        let (t, l, a) = carregar_arte_do_splash(dev).expect("a arte não carregou");
        ler(t.0, l as u64, a as u64, false)
    }

    /// Um caso: textura nova com o gradiente, `desenhar(cb, drawable)`, commit, espera e leitura.
    unsafe fn caso(dev: Id, fila: Id, desenhar: impl FnOnce(Id, Id)) -> Quadro {
        let tex = textura_com_gradiente(dev);
        let drawable = msg0(class("MTLRenderPassColorAttachmentDescriptor") as Id, sel!("new"));
        msg1(drawable, sel!("setTexture:"), tex);
        let cb = msg0(fila, sel!("commandBuffer"));
        desenhar(cb, drawable);
        msg0(cb, sel!("commit"));
        msg0(cb, sel!("waitUntilCompleted"));
        assert_eq!(msg_usize(cb, sel!("status")), 4, "o command buffer não completou");
        let q = ler(tex, LARGURA, ALTURA, true);
        msg0(drawable, sel!("release"));
        msg0(tex, sel!("release"));
        q
    }

    #[test]
    #[ignore]
    fn caminho_metal_desenha() {
        let Some(base) = std::env::var_os("BWMS_PROVA_VISUAL").map(std::path::PathBuf::from) else {
            println!("[prova-metal] sem BWMS_PROVA_VISUAL: pulado");
            return;
        };
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let dev = MTLCreateSystemDefaultDevice();
            assert!(!dev.is_null(), "sem GPU");
            let fila = msg0(dev, sel!("newCommandQueue"));
            // A arte do splash é lida de `red4ext/bwms-splash.png` no diretório corrente: RGBA com alfa parcial, para o caminho do
            // premultiplicado e do pipeline linear entrarem na prova.
            let (al, aa) = (160u32, 90u32);
            let arte: Vec<u8> = (0..al * aa)
                .flat_map(|i| [(i % al * 255 / al) as u8, (i / al * 255 / aa) as u8, 200, if i % 7 == 0 { 128 } else { 255 }])
                .collect();
            std::fs::create_dir_all(base.join("red4ext")).expect("pasta da arte");
            std::fs::write(base.join("red4ext/bwms-splash.png"), super::teste::png(al, aa, 4, &arte)).expect("gravar a arte");
            std::env::set_current_dir(&base).expect("diretório corrente");
            let mut casos: Vec<(String, Quadro)> = Vec::new();
            for nome in ["splash", "splash-reuso"] {
                casos.push((nome.into(), caso(dev, fila, |cb, d| crate::inicio::splash::desenhar(cb, d))));
            }
            casos.push(("arte".into(), arte_lida(dev)));
            for nome in ["selo", "selo-reuso"] {
                casos.push((nome.into(), caso(dev, fila, |cb, d| crate::inicio::splash::desenhar_selo(cb, d))));
            }
            for (preset, fx) in [(1, 0), (2, 0), (3, 0), (4, 0), (5, 0), (6, 0), (0, 1), (0, 2), (6, 3)] {
                LUT_PRESET.store(preset, Ordering::Relaxed);
                EFFECTS.store(fx, Ordering::Relaxed);
                casos.push((format!("filtro-p{preset}-fx{fx}"), caso(dev, fila, |cb, d| aplicar_filtro_de_cor(cb, d))));
            }
            LUT_PRESET.store(0, Ordering::Relaxed);
            EFFECTS.store(0, Ordering::Relaxed);
            for (nome, q) in &casos {
                gravar_bmp(&base.join(format!("metal-{nome}.bmp")), q).expect("bmp");
                println!("[prova-metal] {nome:16} {:#018x}", impressao(q));
            }
            assert!(casos.iter().all(|(_, q)| q.px.iter().any(|p| p[3] > 0.0)), "algum caso leu a textura vazia");
        }
    }

    /// A sonda de pixel do dev (`sonda_de_pixels`) no mesmo caminho: os pontos de cor conhecida do splash e do selo e o centro na entrada e
    /// depois do filtro P&B, lidos da GPU, batem com a malha e com a conta do shader.
    #[test]
    #[ignore]
    fn sonda_de_pixels_confere_o_caminho_metal() {
        if std::env::var_os("BWMS_PROVA_VISUAL").is_none() {
            println!("[prova-metal] sem BWMS_PROVA_VISUAL: pulado");
            return;
        }
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            let dev = MTLCreateSystemDefaultDevice();
            assert!(!dev.is_null(), "sem GPU");
            let fila = msg0(dev, sel!("newCommandQueue"));
            sonda_de_pixels::FORCADA.store(true, Ordering::Relaxed);
            let mut linhas = Vec::new();
            for desenho in ["splash", "selo", "filtro"] {
                LUT_PRESET.store(if desenho == "filtro" { 3 } else { 0 }, Ordering::Relaxed);
                EFFECTS.store(0, Ordering::Relaxed);
                caso(dev, fila, |cb, d| {
                    linhas.extend(sonda_de_pixels::comecar_quadro(cb, d));
                    aplicar_filtro_de_cor(cb, d);
                    match desenho {
                        "splash" => crate::inicio::splash::desenhar(cb, d),
                        "selo" => crate::inicio::splash::desenhar_selo(cb, d),
                        _ => {}
                    }
                    sonda_de_pixels::fim_do_quadro(cb, d);
                });
                linhas.extend(sonda_de_pixels::conferir_pendente());
            }
            LUT_PRESET.store(0, Ordering::Relaxed);
            sonda_de_pixels::FORCADA.store(false, Ordering::Relaxed);
            for l in &linhas {
                println!("{l}");
            }
            assert_eq!(linhas.iter().filter(|l| l.contains(" OK |")).count(), 3, "splash, selo e filtro conferidos");
            assert!(!linhas.iter().any(|l| l.contains("ERRADO") || l.contains("FALHOU")), "algum pixel não bateu");
        }
    }
}

/// Quadro do jogo em pixels, do último render (o `GetDisplayResolution` dos mods Lua); `(0, 0)` antes do 1º quadro.
#[cfg(feature = "lua")]
pub(crate) fn tamanho_do_quadro() -> (f32, f32) {
    (f32::from_bits(FRAME_W.load(Ordering::Relaxed)), f32::from_bits(FRAME_H.load(Ordering::Relaxed)))
}
