//! example-plugin — dylib de 3o-autor MÍNIMA, fora da árvore do cp77-console, sem linkar
//! nenhum crate nosso. Só declara a MESMA struct BwmsApi (repr(C), ABI estável) e o entry
//! `bwms_plugin_main`. Prova o gap `red4ext-3rdparty-plugin-proof` (F1, trilha C): um plugin
//! de verdade consegue (1) logar via a API e (2) registrar uma native NOVA no RTTI, chamável
//! do redscript, sem nenhum acoplamento ao código interno do BWMS.
//!
//! Deploy: dropar o .dylib compilado em `<GAME>/red4ext/plugins/example-plugin.dylib` — o
//! `plugins::load_plugins` do core escaneia essa pasta, dlopen+dlsym `bwms_plugin_main`.

use std::ffi::{c_void, CString};
use std::os::raw::c_char;

type NativeHandler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i64);

// `red4ext-api-prove-hooks-extplugin` (fatia inline_hook): pra chamar `api.inline_hook` num
// alvo REAL do jogo, o plugin precisa converter o VM addr estático (do binário) pro endereço
// de runtime (desloca por ASLR) — SEM linkar nada do core, só APIs públicas do `libSystem`
// (`_dyld_get_image_header` etc.), mesma técnica que `cp77_console::rebase` usa internamente.
extern "C" {
    fn _dyld_image_count() -> u32;
    fn _dyld_get_image_name(image_index: u32) -> *const c_char;
    fn _dyld_get_image_header(image_index: u32) -> *const c_void;
}
unsafe fn game_base() -> usize {
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
    _dyld_get_image_header(0) as usize
}
const LINK_BASE: u64 = 0x1_0000_0000;
/// `AlignedFree(void*,void*)@0x100fa6f00` — MESMO alvo já provado seguro internamente
/// (`red4ext-reloc-prove-ingame`, `test_relocator_cbz`): wrapper de 4 instruções (prólogo CBZ),
/// hook+revert imediato sem nunca chamar através dele.
const ALIGNED_FREE_VM: u64 = 0x1_00fa_6f00;
unsafe fn rebase(vmaddr: u64) -> *mut c_void {
    (game_base() + (vmaddr - LINK_BASE) as usize) as *mut c_void
}
/// Replacement dummy — nunca é de fato CHAMADO (só instalado e revertido na sequência).
unsafe extern "C" fn plugin_inline_dummy() {}

/// Mirror de `cp77_console::plugins::PluginInfo`. Buffers fixos (sem alocação cruzando o ABI).
#[repr(C)]
pub struct PluginInfo {
    pub name: [u8; 64],
    pub author: [u8; 64],
    pub version: [u8; 32],
}

fn write_field(buf: &mut [u8], s: &str) {
    let bytes = s.as_bytes();
    let n = bytes.len().min(buf.len() - 1); // deixa 1 byte pro \0 final
    buf[..n].copy_from_slice(&bytes[..n]);
    buf[n] = 0;
}

/// Entry OPCIONAL (v9): o loader chama ANTES de `bwms_plugin_main` se este símbolo existir.
/// Prova `red4ext-sdk-plumbing` (PluginInfo) ponta-a-ponta — um dylib de 3o-autor genuíno se
/// descrevendo pro loader, sem precisar linkar nada do core.
#[no_mangle]
pub unsafe extern "C" fn bwms_plugin_query(info: *mut PluginInfo) -> bool {
    if info.is_null() {
        return false;
    }
    write_field(&mut (*info).name, "example-plugin");
    write_field(&mut (*info).author, "BWMS 3rd-party proof");
    write_field(&mut (*info).version, "0.1.0");
    true
}

/// Mirror EXATO de `cp77_console::api::BwmsApi` (abi_version 8). Um 3o-autor real colaria
/// isto de um header/crate de API público — aqui é reproduzido à mão para provar que NENHUM
/// link ao crate interno é necessário, só o layout repr(C).
#[repr(C)]
pub struct BwmsApi {
    pub abi_version: u32,
    pub log: unsafe extern "C" fn(*const c_char),
    pub vtable_hook: unsafe extern "C" fn(*mut u64, usize, *const c_void) -> *const c_void,
    pub vtable_unhook: unsafe extern "C" fn(*mut u64, usize, *const c_void),
    pub field_ptr: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    pub call_method: unsafe extern "C" fn(*mut c_void, *const c_char, *mut u8) -> bool,
    pub inline_hook: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    pub inline_revert: unsafe extern "C" fn(*mut c_void),
    pub register_native: unsafe extern "C" fn(*const c_char, *const c_char, NativeHandler) -> bool,
    pub register_native_argful:
        unsafe extern "C" fn(*const c_char, *const c_char, NativeHandler, *const *const c_char, usize) -> bool,
    pub fire_event: unsafe extern "C" fn(*const c_char) -> usize,
    pub prop_get_f32: unsafe extern "C" fn(*mut c_void, *const c_char) -> f32,
    pub prop_set_f32: unsafe extern "C" fn(*mut c_void, *const c_char, f32) -> bool,
    pub prop_get_i32: unsafe extern "C" fn(*mut c_void, *const c_char) -> i32,
    pub call_method_args:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const *const c_char, usize, *mut u8) -> bool,
    pub register_method:
        unsafe extern "C" fn(*const c_char, *const c_char, *const c_char, NativeHandler) -> bool,
    pub tweakdb_get_flat: unsafe extern "C" fn(*const c_char, *mut u32) -> bool,
    pub tweakdb_set_flat: unsafe extern "C" fn(*const c_char, u32) -> bool,
    pub tweakdb_clone_record: unsafe extern "C" fn(*const c_char, *const c_char, *const c_char) -> bool,
    pub log_level: unsafe extern "C" fn(u8, *const c_char),
    pub semver_satisfies: unsafe extern "C" fn(*const c_char, *const c_char) -> bool,
    pub register_draw_callback: unsafe extern "C" fn(extern "C" fn()) -> bool,
    pub imgui_begin: unsafe extern "C" fn(*const c_char) -> bool,
    pub imgui_text: unsafe extern "C" fn(*const c_char),
    pub imgui_end: unsafe extern "C" fn(),
    pub scripts_add: unsafe extern "C" fn(*const c_char) -> bool,
}

/// Handler da nossa native nova `ExamplePluginPing() -> Bool`: sempre retorna true, e loga
/// via a MESMA API (prova que o handler do plugin, chamado pelo motor via routing do core,
/// ainda consegue reentrar na API — ex.: logar de dentro do próprio handler nativo).
static API_PTR: std::sync::atomic::AtomicPtr<BwmsApi> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

unsafe extern "C" fn example_ping_handler(_ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let api = API_PTR.load(std::sync::atomic::Ordering::Relaxed);
    if !api.is_null() {
        if let Ok(msg) = CString::new("[example-plugin] ExamplePluginPing handler rodou (nativa 3o-autor)") {
            ((*api).log)(msg.as_ptr());
        }
        // Disparado SOB DEMANDA (via callg, em gameplay real) — ao contrário da chamada em
        // bwms_plugin_main (cedo demais, TweakDB ainda não carregada), aqui o timing é
        // garantido correto. Prova tweakdb_get_flat/tweakdb_set_flat ponta-a-ponta de um dylib
        // de 3o-autor genuíno: lê o valor original, escreve um valor de teste, lê de novo pra
        // confirmar o round-trip, e RESTAURA o valor original (não deixa o TweakDB sujo).
        let field = CString::new("Items.GrenadeIncendiarySticky.deepWaterDepth").unwrap();
        let mut orig_bits: u32 = 0;
        let got1 = ((*api).tweakdb_get_flat)(field.as_ptr(), &mut orig_bits);
        if let Ok(msg) = CString::new(format!(
            "[example-plugin] (sob demanda) get ANTES -> got={got1} bits={orig_bits:#x} f32={}",
            f32::from_bits(orig_bits)
        )) {
            ((*api).log)(msg.as_ptr());
        }
        let test_bits = 3.0f32.to_bits();
        let set_ok = ((*api).tweakdb_set_flat)(field.as_ptr(), test_bits);
        let mut readback_bits: u32 = 0;
        let got2 = ((*api).tweakdb_get_flat)(field.as_ptr(), &mut readback_bits);
        if let Ok(msg) = CString::new(format!(
            "[example-plugin] (sob demanda) set(3.0)={set_ok} readback got={got2} bits={readback_bits:#x} f32={} (esperado 3.0)",
            f32::from_bits(readback_bits)
        )) {
            ((*api).log)(msg.as_ptr());
        }
        // restaura o valor original (limpeza — não deixa o TweakDB desta sessão sujo pro resto do boot)
        let restore_ok = ((*api).tweakdb_set_flat)(field.as_ptr(), orig_bits);
        if let Ok(msg) = CString::new(format!("[example-plugin] (sob demanda) restaurou original -> {restore_ok}")) {
            ((*api).log)(msg.as_ptr());
        }
    }
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8); // Bool true
    }
}

/// `red4ext-api-prove-hooks-extplugin` (fatia field_ptr, 2026-07-13): registrado em
/// `PlayerPuppet` — `ctx` aqui é o objeto REAL do player vivo. Lê o campo `inCrouch` (Bool,
/// campo de script real, nome sem `m_`) via `field_ptr` (a MESMA API que um plugin de 3o usaria
/// pra ler qualquer campo por nome, sem precisar saber o offset) e loga o byte lido.
unsafe extern "C" fn example_field_handler(ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let api = API_PTR.load(std::sync::atomic::Ordering::Relaxed);
    if !api.is_null() && !ctx.is_null() {
        let field = CString::new("inCrouch").unwrap();
        let p = ((*api).field_ptr)(ctx, field.as_ptr());
        if !p.is_null() {
            let val = *(p as *const u8);
            if let Ok(msg) = CString::new(format!("[example-plugin] field_ptr(ctx, 'inCrouch') -> {p:p} valor(byte)={val}")) {
                ((*api).log)(msg.as_ptr());
            }
        } else if let Ok(msg) = CString::new("[example-plugin] field_ptr(ctx, 'inCrouch') -> null (campo não achado)") {
            ((*api).log)(msg.as_ptr());
        }
        // `red4ext-api-prove-hooks-extplugin` (fatia final, vtable_hook): `ctx` é o PlayerPuppet
        // vivo. Slot 30 é o mesmo slot que o self-test INTERNO (`gum::vtable_selftest`, gated
        // `~/.bwms-vtable-test`) já confirmou ser o 1º slot in-module (`__DATA_CONST`) na faixa
        // [30,64) desta classe, nesta versão do jogo (2.3.1) — descoberto ao vivo antes de
        // codar isto (log: `vtbl static 0x1071ff868 ... slot=30 ... OK=true`). Aqui a MESMA
        // operação roda via a API PÚBLICA (`api.vtable_hook`/`vtable_unhook`), de um dylib de
        // 3o-autor genuíno, fechando o último campo (dos 4: vtable_hook+inline_hook+field_ptr+
        // call_method) que faltava provar via plugin.
        let vtbl = *(ctx as *const *mut u64);
        if !vtbl.is_null() {
            let before = *vtbl.add(30);
            let dummy = example_field_handler as *const c_void;
            let orig = ((*api).vtable_hook)(vtbl, 30, dummy);
            let after_hook = *vtbl.add(30);
            ((*api).vtable_unhook)(vtbl, 30, orig);
            let after_unhook = *vtbl.add(30);
            if let Ok(msg) = CString::new(format!(
                "[example-plugin] vtable_hook(ctx.vtbl, slot=30) antes={before:#x} depois-do-hook={after_hook:#x} (==dummy {}) apos-unhook={after_unhook:#x} (==original {})",
                after_hook == dummy as u64,
                after_unhook == before
            )) {
                ((*api).log)(msg.as_ptr());
            }
        }
        // `red4ext-api-prove-hooks-extplugin` (fatia inline_hook): alvo REAL `AlignedFree`
        // (mesmo endereço já provado seguro por `red4ext-reloc-prove-ingame`), instalado+
        // revertido IMEDIATAMENTE (nunca chamado através do hook) — mesma disciplina do
        // self-test interno (`test_relocator`/`test_relocator_cbz`), mas via a API PÚBLICA de
        // um dylib de 3o-autor genuíno (slide de ASLR calculado com `_dyld_get_image_header`,
        // sem linkar nada do core).
        let target = rebase(ALIGNED_FREE_VM);
        if !target.is_null() {
            let before4 = *(target as *const u32);
            let repl = ((*api).inline_hook)(target, plugin_inline_dummy as *mut c_void);
            let after_hook4 = *(target as *const u32);
            ((*api).inline_revert)(target);
            let after_revert4 = *(target as *const u32);
            if let Ok(msg) = CString::new(format!(
                "[example-plugin] inline_hook(AlignedFree@{target:p}) trampolim={} 1a-instr antes={before4:#010x} depois-do-hook={after_hook4:#010x} (mudou={}) apos-revert={after_revert4:#010x} (byte-exato={})",
                !repl.is_null(),
                before4 != after_hook4,
                after_revert4 == before4
            )) {
                ((*api).log)(msg.as_ptr());
            }
        }
        // `red4ext-api-prove-hooks-extplugin` (fatia call_method, SEM args — complementa
        // `call_method_args` já provado no handler de gameGodModeSystem): `GetEntityID()` é
        // zero-arg, read-only, presente em qualquer IScriptable/Entity real.
        let method0 = CString::new("GetEntityID").unwrap();
        let mut ret16b = [0u8; 16];
        let ok0 = ((*api).call_method)(ctx, method0.as_ptr(), ret16b.as_mut_ptr());
        if let Ok(msg) = CString::new(format!(
            "[example-plugin] call_method(ctx, GetEntityID) -> ok={ok0} bytes={:02x?}",
            &ret16b[..8]
        )) {
            ((*api).log)(msg.as_ptr());
        }
    }
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8);
    }
}

unsafe extern "C" fn example_method_handler(ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let api = API_PTR.load(std::sync::atomic::Ordering::Relaxed);
    if !api.is_null() {
        if let Ok(msg) = CString::new("[example-plugin] ExamplePluginMethod handler rodou (metodo em classe existente)") {
            ((*api).log)(msg.as_ptr());
        }
        // `red4ext-callmethod-args` (2026-07-13): `ctx` AQUI é o objeto REAL (gameGodModeSystem
        // vivo, o mesmo `this` que o motor passou pra native) — prova que um plugin de 3o-autor
        // consegue usar `call_method_args` (não só `call_method` sem args) pra chamar OUTRO
        // método no MESMO objeto, com um arg TIPADO real (CName), tudo via a API pública, sem
        // linkar nada do core. `IsExactlyA` é um getter de reflection seguro (read-only).
        if !ctx.is_null() {
            let method = CString::new("IsExactlyA").unwrap();
            let arg = CString::new("n:gameGodModeSystem").unwrap();
            let args: [*const c_char; 1] = [arg.as_ptr()];
            let mut ret16 = [0u8; 16];
            let ok = ((*api).call_method_args)(ctx, method.as_ptr(), args.as_ptr(), 1, ret16.as_mut_ptr());
            if let Ok(msg) = CString::new(format!(
                "[example-plugin] call_method_args(ctx, IsExactlyA, n:gameGodModeSystem) -> ok={ok} ret[0]={}",
                ret16[0]
            )) {
                ((*api).log)(msg.as_ptr());
            }
        }
    }
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8); // Bool true
    }
}

/// `cet-thirdparty-mod-api`: lifecycle onUpdate de VERDADE — o CORE chama esta native TODO TICK
/// (não só 1x em `bwms_plugin_main`). Prova que um plugin de 3o-autor recebe um evento contínuo
/// pós-boot via `BwmsApi`, sem CallbackSystem/IScriptable, só `register_native` + o dispatcher do
/// core resolvendo o nome pela MESMA via de `callg` (`register::get_function`+`rtti::call_func`).
static ONUPDATE_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn example_onupdate_handler(_ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let n = ONUPDATE_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    // throttle: só loga na 1a e a cada 60 (evita floodar /tmp/cp77-console.log a cada tick).
    if n == 1 || n % 60 == 0 {
        let api = API_PTR.load(std::sync::atomic::Ordering::Relaxed);
        if !api.is_null() {
            if let Ok(msg) =
                CString::new(format!("[example-plugin] onUpdate tick #{n} (lifecycle pós-boot via BwmsApi)"))
            {
                ((*api).log)(msg.as_ptr());
            }
        }
    }
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8); // Bool true (retorno arbitrário, ninguém lê)
    }
}

/// `cet-imgui-thirdparty`: desenha a PRÓPRIA janela ImGui, de um dylib de 3o-autor genuíno, SEM
/// linkar imgui-rs/cimgui — só os 3 wrappers finos (`imgui_begin`/`imgui_text`/`imgui_end`) da
/// API pública. Chamado 1x/frame pelo core, dentro do onDraw (overlay BWMS aberto).
extern "C" fn example_draw_callback() {
    let api = API_PTR.load(std::sync::atomic::Ordering::Relaxed);
    if api.is_null() {
        return;
    }
    unsafe {
        let title = CString::new("Example Plugin (3rd-party, no imgui-rs link)").unwrap();
        ((*api).imgui_begin)(title.as_ptr());
        let text = CString::new("Hello from example-plugin! (cet-imgui-thirdparty)").unwrap();
        ((*api).imgui_text)(text.as_ptr());
        ((*api).imgui_end)();
    }
}

#[no_mangle]
pub unsafe extern "C" fn bwms_plugin_main(api: *const BwmsApi) -> i32 {
    if api.is_null() {
        return 1;
    }
    API_PTR.store(api as *mut BwmsApi, std::sync::atomic::Ordering::Relaxed);
    // Capacidade 1: log via a API.
    if let Ok(msg) = CString::new("[example-plugin] bwms_plugin_main entrou (dylib 3o-autor, fora do tree)") {
        ((*api).log)(msg.as_ptr());
    }
    // Capacidade 2: registrar uma native NOVA no RTTI, com o handler DESTE dylib (não do core).
    let full = CString::new("ExamplePluginPing").unwrap();
    let short = CString::new("ExamplePluginPing").unwrap();
    let ok = ((*api).register_native)(full.as_ptr(), short.as_ptr(), example_ping_handler);
    if let Ok(msg) = CString::new(format!("[example-plugin] register_native ExamplePluginPing -> {ok}")) {
        ((*api).log)(msg.as_ptr());
    }
    // Capacidade 3 (v7): registrar um MÉTODO novo numa classe EXISTENTE (gameGodModeSystem,
    // igual ao teste interno já provado 2026-07-12) — de um dylib GENUINAMENTE de terceiros.
    let class = CString::new("gameGodModeSystem").unwrap();
    let mfull = CString::new("gameGodModeSystem::ExamplePluginMethod").unwrap();
    let mshort = CString::new("ExamplePluginMethod").unwrap();
    let mok = ((*api).register_method)(class.as_ptr(), mfull.as_ptr(), mshort.as_ptr(), example_method_handler);
    if let Ok(msg) = CString::new(format!("[example-plugin] register_method ExamplePluginMethod -> {mok}")) {
        ((*api).log)(msg.as_ptr());
    }
    // Capacidade 6 (2026-07-13): registra em PlayerPuppet — `ctx` vira o player vivo quando
    // chamado — pra exercitar `field_ptr` (fatia final de `red4ext-api-prove-hooks-extplugin`).
    let pclass = CString::new("PlayerPuppet").unwrap();
    let pfull = CString::new("PlayerPuppet::ExamplePluginFieldTest").unwrap();
    let pshort = CString::new("ExamplePluginFieldTest").unwrap();
    let pok = ((*api).register_method)(pclass.as_ptr(), pfull.as_ptr(), pshort.as_ptr(), example_field_handler);
    if let Ok(msg) = CString::new(format!("[example-plugin] register_method ExamplePluginFieldTest -> {pok}")) {
        ((*api).log)(msg.as_ptr());
    }
    // `cet-thirdparty-mod-api`: registra a native global que o CORE vai chamar a CADA TICK (ver
    // cp77_tick em lib.rs) — o lifecycle onUpdate contínuo, não só esta chamada única de boot.
    let ofull = CString::new("BwmsPluginOnUpdate").unwrap();
    let oshort = CString::new("BwmsPluginOnUpdate").unwrap();
    let ook = ((*api).register_native)(ofull.as_ptr(), oshort.as_ptr(), example_onupdate_handler);
    if let Ok(msg) = CString::new(format!("[example-plugin] register_native BwmsPluginOnUpdate -> {ook}")) {
        ((*api).log)(msg.as_ptr());
    }
    // Capacidade 4 (v8): ler um flat TweakDB conhecido (deepWaterDepth, já provado = -0.5 em
    // sessões anteriores) de um dylib GENUINAMENTE de terceiros, sem tocar nada do TweakDB
    // internamente — prova `tweakxl-mod-api` ponta-a-ponta.
    let field = CString::new("Items.GrenadeIncendiarySticky.deepWaterDepth").unwrap();
    let mut bits: u32 = 0;
    let got = ((*api).tweakdb_get_flat)(field.as_ptr(), &mut bits);
    let as_f32 = f32::from_bits(bits);
    if let Ok(msg) =
        CString::new(format!("[example-plugin] tweakdb_get_flat deepWaterDepth -> got={got} bits={bits:#x} f32={as_f32}"))
    {
        ((*api).log)(msg.as_ptr());
    }
    // Capacidade 5 (v9): logger por-nível + SemVer runtime — parte de `red4ext-sdk-plumbing`.
    if let Ok(msg) = CString::new("[example-plugin] teste de log_level (nivel WARN)") {
        ((*api).log_level)(3, msg.as_ptr());
    }
    let req = CString::new("1.2.0").unwrap();
    let actual_ok = CString::new("1.3.0").unwrap();
    let actual_low = CString::new("1.1.0").unwrap();
    let ok1 = ((*api).semver_satisfies)(req.as_ptr(), actual_ok.as_ptr());
    let ok2 = ((*api).semver_satisfies)(req.as_ptr(), actual_low.as_ptr());
    if let Ok(msg) = CString::new(format!(
        "[example-plugin] semver_satisfies(req=1.2.0, actual=1.3.0)={ok1} (esperado true) / (actual=1.1.0)={ok2} (esperado false)"
    )) {
        ((*api).log)(msg.as_ptr());
    }
    // v10: registra o draw callback (cet-imgui-thirdparty) — a janela some se rodar ANTES do
    // overlay estar pronto (in_draw() sempre false até lá); registrar 1x aqui e o core chama
    // TODO frame daí em diante já é suficiente.
    let dok = ((*api).register_draw_callback)(example_draw_callback);
    if let Ok(msg) = CString::new(format!("[example-plugin] register_draw_callback -> {dok}")) {
        ((*api).log)(msg.as_ptr());
    }
    // `red4ext-scripts-add` (v11, 2026-07-18): registra um `.reds` que vive FORA de r6/scripts
    // — ao lado do PRÓPRIO dylib deste plugin (`red4ext/plugins/example-plugin-scripts/`, o
    // padrão real de um mod de 3os que shipa dylib+reds juntos), achado via `_dyld_get_image_
    // name` da PRÓPRIA imagem (mesma técnica já usada em `game_base()` acima pra achar o
    // binário do jogo — zero path absoluto do host embutido no binário, traceless). NÃO
    // recompila nada AGORA (redscript não hot-recarrega) — só registra no manifesto
    // (`~/.bwms-scripts-add.txt`); o efeito aparece no PRÓXIMO ciclo compile+boot.
    if let Some(reds_path) = find_own_external_reds_path() {
        if let Ok(cpath) = CString::new(reds_path.clone()) {
            let sok = ((*api).scripts_add)(cpath.as_ptr());
            if let Ok(msg) =
                CString::new(format!("[example-plugin] scripts_add('{reds_path}') -> {sok}"))
            {
                ((*api).log)(msg.as_ptr());
            }
        }
    } else if let Ok(msg) =
        CString::new("[example-plugin] scripts_add: não achei o .reds externo (ver example-plugin-scripts/ ao lado do dylib)")
    {
        ((*api).log)(msg.as_ptr());
    }
    0
}

/// Acha `<pasta-do-dylib-deste-plugin>/example-plugin-scripts/BwmsExternalPluginClass.reds` —
/// usa `_dyld_get_image_name` pra achar a PRÓPRIA imagem carregada (procura por
/// "example-plugin.dylib" no nome), deriva a pasta-irmã. Sem hardcode de path do host.
unsafe fn find_own_external_reds_path() -> Option<String> {
    let n = _dyld_image_count();
    for i in 0..n {
        let name = _dyld_get_image_name(i);
        if name.is_null() {
            continue;
        }
        if let Ok(s) = std::ffi::CStr::from_ptr(name).to_str() {
            if s.ends_with("example-plugin.dylib") {
                let dir = std::path::Path::new(s).parent()?;
                let reds = dir.join("example-plugin-scripts").join("BwmsExternalPluginClass.reds");
                return reds.to_str().map(|s| s.to_string());
            }
        }
    }
    None
}
