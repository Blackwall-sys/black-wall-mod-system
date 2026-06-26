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
    // re-acopla o cursor ao mouse (o jogo desacopla p/ a câmera em gameplay).
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
static LOGGED: AtomicBool = AtomicBool::new(false);
static SHOW: AtomicBool = AtomicBool::new(false); // começa ESCONDIDO (` abre)
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

/// Verdadeiro só durante o `onDraw` (dentro do frame imgui, thread de render). As
/// funções `ImGui.*` do Lua checam isso — chamar fora do onDraw é no-op (sem crash).
pub fn in_draw() -> bool {
    IN_DRAW.load(Ordering::Relaxed)
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
                        // CallbackSystem RawInput: enfileira TODA tecla (keycode) p/ o evento
                        // "Input/Key" (drenado no cp77_tick → fire_event_args na thread do jogo).
                        crate::push_raw_key(kc as i32);
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
}
struct Renderer {
    ctx: imgui::Context,
    pso: metal::RenderPipelineState,
    lut_pso: Option<metal::RenderPipelineState>,
    font_tex: metal::Texture,
    ui: UiState,
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
fragment float4 lut_f(float4 cur [[color(0)]], constant uint& preset [[buffer(0)]]) {
    float3 x = cur.rgb;
    if (preset == 1u) { x = clamp(x * float3(1.18, 1.0, 0.82), 0.0, 1.0); }
    else if (preset == 2u) { x = clamp(x * float3(0.82, 0.95, 1.25), 0.0, 1.0); }
    else if (preset == 3u) { float g = dot(x, float3(0.299, 0.587, 0.114)); x = float3(g); }
    else if (preset == 4u) { x = clamp((x - 0.5) * 1.45 + 0.5, 0.0, 1.0); }
    else if (preset == 5u) { float g = dot(x, float3(0.299, 0.587, 0.114)); x = clamp(float3(g) * float3(1.0, 0.78, 0.52) * 1.25, 0.0, 1.0); }
    return float4(x, cur.a);
}
"#;

/// ReShade-Metal: preset de LUT/grading ativo (0 = off, jogo intocado). Cicla com F2.
static LUT_PRESET: AtomicU32 = AtomicU32::new(0);
const LUT_COUNT: u32 = 6;
const LUT_NAMES: [&str; 6] = ["off", "Quente", "Frio", "P&B", "Contraste", "Sepia"];

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
    crate::log("[overlay] imgui renderer pronto (fonte + pipeline + lut)");
    Some(Renderer {
        ctx,
        pso,
        lut_pso,
        font_tex,
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
        },
    })
}

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
    let n = std::fs::read_to_string("/tmp/cp77-console.log").map(|s| s.lines().count()).unwrap_or(0);
    LINE_CLEAR.store(n as u32, Ordering::Relaxed);
}
fn tail_log(n: usize) -> Vec<String> {
    let s = std::fs::read_to_string("/tmp/cp77-console.log").unwrap_or_default();
    let v: Vec<&str> = s.lines().collect();
    let base = (LINE_CLEAR.load(Ordering::Relaxed) as usize).min(v.len());
    let view = &v[base..];
    let start = view.len().saturating_sub(n);
    view[start..].iter().map(|x| x.to_string()).collect()
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
                    let hint = "Enter: money N | give Items.X N | godmode | level N | OU Lua: Game.AddMoney(7777)";
                    #[cfg(not(feature = "lua"))]
                    let hint = "Enter: money N | give Items.X N | godmode | level N";
                    let entered = ui
                        .input_text("##cmd", &mut st.cmd_buf)
                        .enter_returns_true(true)
                        .hint(hint)
                        .build();
                    if entered {
                        let c = st.cmd_buf.trim().to_string();
                        if !c.is_empty() {
                            write_cmd(&c);
                        }
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
                // ---- GAME CHEATS: favoritos pinados + botões de 1-clique ----
                if let Some(_t) = ui.tab_item("Game cheats") {
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
                        ui.separator();
                    }
                    ui.text("Cheats:");
                    if ui.button("Money +50k") {
                        write_cmd("money 50000");
                    }
                    ui.same_line();
                    if ui.button("Heal") {
                        write_cmd("heal");
                    }
                    ui.same_line();
                    if ui.button("Godmode") {
                        write_cmd("godmode");
                    }
                    ui.same_line();
                    if ui.button("Godmode off") {
                        write_cmd("godmode off");
                    }
                    if ui.button("Perks +10") {
                        write_cmd("perks 10");
                    }
                    ui.same_line();
                    if ui.button("Attrs +10") {
                        write_cmd("attrs 10");
                    }
                    ui.same_line();
                    if ui.button("Relic +10") {
                        write_cmd("relic 10");
                    }
                    ui.same_line();
                    if ui.button("Summon car") {
                        write_cmd("summon");
                    }
                    ui.same_line();
                    if ui.button("Level 50") {
                        write_cmd("level 50");
                    }
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
                    ui.text_disabled("Grading Metal AO VIVO no frame do jogo. Clica e a tela muda na hora.");
                    ui.text_disabled("MVP: presets fixos. Proximo: LUTs .cube reais (Nova/Preem).");
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
    if !show && !badge {
        return;
    }
    // só re-acopla o cursor quando o painel está aberto (em gameplay o jogo prende
    // o cursor na câmera; o badge é não-interativo e não precisa do cursor).
    if show {
        CGAssociateMouseAndMouseCursorPosition(true);
    }
    let w = msg_usize(tex_raw, sel("width")) as f32;
    let h = msg_usize(tex_raw, sel("height")) as f32;
    let w = msg_usize(tex_raw, sel("width")) as f32;
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
        if badge {
            build_badge(ui, crate::ticks(), crate::mods_loaded());
        }
        if show {
            build_ui(ui, &mut rd.ui);
            // mods desenham as PRÓPRIAS janelas (ImGui-pro-Lua) durante o onDraw.
            IN_DRAW.store(true, Ordering::Relaxed);
            crate::lua::run_event_draw();
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
        if preset > 0 {
            if let Some(lp) = rd.lut_pso.as_ref() {
                let lrpd = metal::RenderPassDescriptor::new();
                let la = lrpd.color_attachments().object_at(0).unwrap();
                la.set_texture(Some(tex));
                la.set_load_action(metal::MTLLoadAction::Load);
                la.set_store_action(metal::MTLStoreAction::Store);
                let le = cb.new_render_command_encoder(lrpd);
                le.set_render_pipeline_state(lp);
                le.set_fragment_bytes(0, 4, &preset as *const u32 as *const c_void);
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
