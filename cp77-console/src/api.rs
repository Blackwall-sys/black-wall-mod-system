//! API C-ABI exposta aos plugins Rust (`BwmsApi`). O plugin recebe um `*const BwmsApi`
//! no entry e chama a vtable pra HOOKAR e REFLETIR sem linkar nada do nosso crate.
//!
//! v1 (ALFA): `log` + `vtable_hook`/`vtable_unhook` (o motor de hook = RED4ext, já provado
//! in-game, COW em __DATA_CONST) + `field_ptr`/`call_method` (reflection por nome = Codeware
//! getf/setf/callf). Roadmap v2: inline hook, `register_native`, ImGui — campos novos SÓ no
//! fim da struct + `abi_version` cresce (ABI estável pra plugins antigos).

use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

/// Vtable C-ABI passada ao plugin. `#[repr(C)]` => layout estável; o plugin declara a MESMA
/// struct (idêntica) e chama os ponteiros. Campos novos entram só no FIM (compat pra frente).
#[repr(C)]
pub struct BwmsApi {
    /// Versão da ABI (= `plugins::BWMS_PLUGIN_API`). O plugin pode checar antes de usar campos novos.
    pub abi_version: u32,
    /// Escreve no log do BWMS (`/tmp/cp77-console.log`). `msg` = C-string UTF-8.
    pub log: unsafe extern "C" fn(*const c_char),
    /// Hook de VTABLE: troca o slot `slot_idx` (índice em u64) de `vtbl` por `repl`; devolve o
    /// ponteiro ORIGINAL do slot (pra encadear). null = falhou. Não patcha __TEXT (COW na vtable).
    pub vtable_hook: unsafe extern "C" fn(vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void,
    /// Desfaz um `vtable_hook` (restaura `orig` no slot).
    pub vtable_unhook: unsafe extern "C" fn(vtbl: *mut u64, slot_idx: usize, orig: *const c_void),
    /// Endereço do CAMPO `field` (por nome, via RTTI) no objeto vivo `obj`. null = não achou.
    /// O plugin lê/escreve direto nesse ponteiro — é o getf/setf cru.
    pub field_ptr: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> *mut c_void,
    /// Chama o método `method` SEM args em `obj` (por nome); escreve 16 bytes de retorno em
    /// `ret16` (pode ser null). true = chamou. (call com args tipados = futuro.)
    pub call_method: unsafe extern "C" fn(obj: *mut c_void, method: *const c_char, ret16: *mut u8) -> bool,
    // --- v2 (abi_version 2): hook INLINE (qualquer função, não só método virtual) ---
    /// Hook inline: troca `target` por `repl`; devolve o TRAMPOLIM (chame-o pra invocar a
    /// função original) ou null. Relocator arm64 + COW em __TEXT (provado in-game).
    pub inline_hook: unsafe extern "C" fn(target: *mut c_void, repl: *mut c_void) -> *mut c_void,
    /// Desfaz um `inline_hook` em `target`.
    pub inline_revert: unsafe extern "C" fn(target: *mut c_void),
    // --- v3 (abi_version 3): registrar native global no RTTI (Codeware) ---
    /// Registra uma native global no RTTI chamável do redscript: `full`=nome completo, `short`=nome
    /// curto, `handler`=`extern "C" fn(ctx, frame, ret, a4)`. Devolve true se re-resolve OK.
    /// (Sem-args por ora; argful/method = roadmap.) PROVADO internamente (register_all/BlackwallPing).
    pub register_native: unsafe extern "C" fn(
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
    ) -> bool,
    // --- v4 (abi_version 4): API simétrica ao núcleo — nativa COM ARGS + emitir evento ---
    /// Registra uma native global COM ARGS chamável do redscript. `param_types` = array de
    /// `n_params` C-strings com os NOMES DOS TIPOS RED (ex.: `"Float"`, `"CName"`, `"Int32"`).
    /// O handler lê os args via o frame (padrão read_params). Devolve true se re-resolve OK.
    /// Fecha a assimetria: antes o plugin só registrava nativa SEM-args (o núcleo já fazia argful).
    pub register_native_argful: unsafe extern "C" fn(
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
        param_types: *const *const c_char,
        n_params: usize,
    ) -> bool,
    /// Dispara um evento do CallbackSystem por nome (ex.: `"Input/Custom"`): chama todos os
    /// callbacks registrados nesse evento. Devolve quantos dispararam. Deixa um plugin EMITIR
    /// eventos que mods (redscript/plugin) escutam — a outra ponta do RegisterCallback.
    pub fire_event: unsafe extern "C" fn(name: *const c_char) -> usize,
    // --- v5 (abi_version 5): Reflection TIPADA (get/set de campo por nome, sem ponteiro cru) ---
    /// Lê um campo Float por nome no objeto vivo. `0.0` se não achar (o plugin não faz cast de
    /// ponteiro na mão como no `field_ptr`). Parity com o getf interno.
    pub prop_get_f32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> f32,
    /// Escreve um campo Float por nome. Devolve true se achou+escreveu.
    pub prop_set_f32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char, val: f32) -> bool,
    /// Lê um campo Int32/Uint32 por nome (0 se não achar).
    pub prop_get_i32: unsafe extern "C" fn(obj: *mut c_void, field: *const c_char) -> i32,
    // --- v6 (abi_version 6): call de método COM ARGS tipados (parity com o callf interno) ---
    /// Chama `method` em `obj` com `n_args` args tipados. Cada `args[i]` é uma C-string no formato
    /// do loop de cmd: `i:5` (Int32), `f:1.5` (Float), `b:true` (Bool), `n:Nome` (CName), `s:txt`
    /// (String), `e:3` (Enum). Escreve 16B de retorno em `ret16` (pode ser null). true = chamou.
    /// Fecha o gap do `call_method` sem-args: reusa `parse_cmd_arg` + `call_func` (callf, provados).
    pub call_method_args: unsafe extern "C" fn(
        obj: *mut c_void,
        method: *const c_char,
        args: *const *const c_char,
        n_args: usize,
        ret16: *mut u8,
    ) -> bool,
    // --- v7 (abi_version 7): registrar MÉTODO novo numa classe EXISTENTE (@addMethod-style) ---
    /// Registra um método novo, chamável do redscript, numa classe REAL já existente (ex.:
    /// GameObject, Entity, gameGodModeSystem — o padrão real do Codeware via `@addMethod`).
    /// `class`=nome da classe-alvo, `full`=nome completo do método, `short`=nome curto,
    /// `handler`=`extern "C" fn(ctx, frame, ret, a4)`. Devolve true se registrou OK. Fecha
    /// `red4ext-register-method-api`: até aqui só `register_native` (função GLOBAL) era exposto
    /// a plugins — isto fecha o caso "estender classe existente", já provado internamente
    /// (register_method/regmethod_selftest, `gameGodModeSystem.BwmsRegTest`, 2026-07-12).
    pub register_method: unsafe extern "C" fn(
        class: *const c_char,
        full: *const c_char,
        short: *const c_char,
        handler: crate::register::NativeHandler,
    ) -> bool,
    // --- v8 (abi_version 8): TweakDB — flat escalar por nome + clone com herança (tweakxl-mod-api) ---
    /// Lê o valor escalar (4 bytes crus em `+0x08` do FlatValue) do flat `name` (formato
    /// `Record.campo`, ex. `"Items.GrenadeIncendiarySticky.deepWaterDepth"`). Devolve os bits em
    /// `out_bits` (o plugin reinterpreta como f32/i32 conforme o tipo do campo); true = achou.
    pub tweakdb_get_flat: unsafe extern "C" fn(name: *const c_char, out_bits: *mut u32) -> bool,
    /// Escreve o valor escalar de `name` (mesmo formato). ⚠️ afeta TODOS os records que
    /// compartilham o mesmo FlatValue (records clonados via `tweakdb_clone_record` sem override
    /// prévio nesse campo). true = achou+escreveu.
    pub tweakdb_set_flat: unsafe extern "C" fn(name: *const c_char, bits: u32) -> bool,
    /// Clona `source` (record existente) como `new_name`, herdando os flats reais (stats) via
    /// `InheritFlats`, e registra o record novo no TweakDB vivo (`CreateRecord`). Assíncrono
    /// (roda numa thread própria, ~1.5s de atraso) — chamar de dentro de um handler de native/
    /// callback é seguro, o resultado aparece no log (`[clone] N flats herdados...`). Sempre
    /// devolve true se os 3 argumentos forem C-strings válidas (aceito para processamento; não
    /// é confirmação de sucesso — ver log).
    pub tweakdb_clone_record: unsafe extern "C" fn(
        class_name: *const c_char,
        source: *const c_char,
        new_name: *const c_char,
    ) -> bool,
    // --- v9 (abi_version 9): logger por nível + SemVer runtime (parte de `red4ext-sdk-plumbing`) ---
    /// Loga `msg` marcado com `level` (0=Trace 1=Debug 2=Info 3=Warning 4=Error 5=Critical —
    /// mesma escala do Logger do RED4ext.SDK real). Níveis fora de 0..5 caem em "Info". Parity
    /// com o `log` sem-nível (v1), que continua funcionando igual.
    pub log_level: unsafe extern "C" fn(level: u8, msg: *const c_char),
    /// Compara duas versões `"major.minor.patch"` (sufixos após o patch são ignorados — não é
    /// SemVer completo com pre-release/build, cobre o caso real de `Codeware.Require("1.2.0")`
    /// checando "a versão instalada é >= a exigida"). Devolve `actual >= required`; strings
    /// malformadas ou faltando componentes tratam a parte ausente como 0. Ex.:
    /// `semver_satisfies("1.2.0", "1.3.0") == true`.
    pub semver_satisfies: unsafe extern "C" fn(required: *const c_char, actual: *const c_char) -> bool,
    // --- v10 (abi_version 10): ImGui pro plugin NÃO-lua desenhar (`cet-imgui-thirdparty`) ---
    /// Registra `cb` pra ser chamado a cada frame DENTRO da janela onDraw (mesmo ponto que os
    /// mods Lua usam via `ImGui.Begin/Text/End`, gated por `overlay::in_draw()` — só roda com o
    /// overlay BWMS aberto). O plugin usa `imgui_begin/imgui_text/imgui_end` (abaixo) de dentro
    /// de `cb` pra desenhar sua PRÓPRIA janela, sem linkar imgui-rs/cimgui — a API crua do Dear
    /// ImGui já roda no NOSSO binário, o plugin só chama os wrappers finos. Multi-registro (Vec
    /// interno) — plugins não pisam uns nos outros. Devolve true (sempre aceita).
    pub register_draw_callback: unsafe extern "C" fn(cb: extern "C" fn()) -> bool,
    /// `ImGui::Begin(title)` — abre uma janela nova. NO-OP (devolve `true`, "está visível") fora
    /// do onDraw. Mesma chamada crua (`igBegin`) que o binding Lua usa.
    pub imgui_begin: unsafe extern "C" fn(title: *const c_char) -> bool,
    /// `ImGui::Text(s)` — texto sem formatação na janela aberta por `imgui_begin`.
    pub imgui_text: unsafe extern "C" fn(text: *const c_char),
    /// `ImGui::End()` — fecha a janela aberta por `imgui_begin`. SEMPRE chamar em par (mesmo se
    /// `imgui_begin` devolveu `false`), igual à API real do Dear ImGui.
    pub imgui_end: unsafe extern "C" fn(),
    // --- v11 (abi_version 11): `scripts.Add(path)` — plugin registra um `.reds` FORA de
    // r6/scripts pro compilador incluir (`red4ext-scripts-add`) ---
    /// Registra `path` (caminho ABSOLUTO de um `.reds` fora de `r6/scripts`) num manifesto
    /// persistente (`~/.bwms-scripts-add.txt`, 1 path por linha, dedup). **NÃO recompila nem
    /// afeta o boot ATUAL** — este processo já carregou o `final.redscripts` que o compile
    /// anterior gerou, ANTES do plugin sequer rodar (redscript não hot-recarrega em runtime,
    /// diferente de Lua/CET). O efeito aparece no PRÓXIMO ciclo compile+boot: o passo de
    /// compile (`bwms-fastboot.sh compile`, ou o instalador) lê o manifesto e passa cada path
    /// via `-compilePathsFile` do `scc` (achado nesta rodada — `scc` aceita uma lista de
    /// arquivos AVULSOS, além do diretório `r6/scripts` normal, no MESMO passe de compilação;
    /// verificado num scratch-copy seguro antes de mexer no deploy real). Devolve `true` se
    /// escreveu (ou já estava) no manifesto; `false` só em erro de I/O real.
    pub scripts_add: unsafe extern "C" fn(path: *const c_char) -> bool,
}

unsafe extern "C" fn api_log(msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    crate::log(&format!("[plugin] {}", CStr::from_ptr(msg).to_string_lossy()));
}

unsafe extern "C" fn api_vtable_hook(vtbl: *mut u64, slot_idx: usize, repl: *const c_void) -> *const c_void {
    crate::gum::vtable_hook(vtbl, slot_idx, repl).unwrap_or(std::ptr::null())
}

unsafe extern "C" fn api_vtable_unhook(vtbl: *mut u64, slot_idx: usize, orig: *const c_void) {
    crate::gum::vtable_unhook(vtbl, slot_idx, orig);
}

unsafe extern "C" fn api_field_ptr(obj: *mut c_void, field: *const c_char) -> *mut c_void {
    if obj.is_null() || field.is_null() {
        return std::ptr::null_mut();
    }
    let name = match CStr::from_ptr(field).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    let p = crate::rtti::find_property_in_class(cls, name);
    if p.is_null() {
        return std::ptr::null_mut();
    }
    let vo = crate::rtti::prop_value_offset(p) as usize;
    (obj as *mut u8).add(vo) as *mut c_void
}

unsafe extern "C" fn api_call_method(obj: *mut c_void, method: *const c_char, ret16: *mut u8) -> bool {
    if obj.is_null() || method.is_null() {
        return false;
    }
    let name = match CStr::from_ptr(method).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return false;
    }
    match crate::rtti::resolve_in_class(cls, name) {
        Some(rf) => match crate::rtti::call_func(&rf, obj, &[]) {
            Some(r) => {
                if !ret16.is_null() {
                    std::ptr::copy_nonoverlapping(r.as_ptr(), ret16, 16);
                }
                true
            }
            None => false,
        },
        None => false,
    }
}

unsafe extern "C" fn api_inline_hook(target: *mut c_void, repl: *mut c_void) -> *mut c_void {
    if target.is_null() || repl.is_null() {
        return std::ptr::null_mut();
    }
    crate::gum::Interceptor::obtain().replace(target, repl).unwrap_or(std::ptr::null_mut())
}

unsafe extern "C" fn api_inline_revert(target: *mut c_void) {
    if target.is_null() {
        return;
    }
    crate::gum::Interceptor::obtain().revert(target);
}

unsafe extern "C" fn api_register_native(
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
) -> bool {
    if full.is_null() || short.is_null() {
        return false;
    }
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // mesmo caminho do register_all: Registry::obtain + proto global clonável (Cos/Sin/...).
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = crate::register::get_function(&reg, n);
        if crate::rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !crate::rtti::sane(proto) {
        return false;
    }
    crate::register::register_global(&reg, proto, fulls, shorts, handler)
}

unsafe extern "C" fn api_register_method(
    class: *const c_char,
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
) -> bool {
    if class.is_null() || full.is_null() || short.is_null() {
        return false;
    }
    let classs = match CStr::from_ptr(class).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    // proto = método estático nativo conhecido (mesmo donor do register_codeware_facade/
    // regmethod_selftest, já provado in-game 2026-07-12).
    let proto = match crate::rtti::resolve_any(&reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return false,
    };
    crate::register::register_method(&reg, classs, proto, fulls, shorts, handler, true)
}

unsafe extern "C" fn api_register_native_argful(
    full: *const c_char,
    short: *const c_char,
    handler: crate::register::NativeHandler,
    param_types: *const *const c_char,
    n_params: usize,
) -> bool {
    if full.is_null() || short.is_null() {
        return false;
    }
    let fulls = match CStr::from_ptr(full).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let shorts = match CStr::from_ptr(short).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // lê os NOMES DOS TIPOS do array de C-strings (null em qualquer um = aborta seguro).
    let mut types: Vec<&str> = Vec::with_capacity(n_params);
    if n_params > 0 {
        if param_types.is_null() {
            return false;
        }
        for i in 0..n_params {
            let p = *param_types.add(i);
            if p.is_null() {
                return false;
            }
            match CStr::from_ptr(p).to_str() {
                Ok(s) => types.push(s),
                Err(_) => return false,
            }
        }
    }
    let reg = match crate::rtti::Registry::obtain() {
        Some(r) => r,
        None => return false,
    };
    // proto = uma global argful já existente (Cos/Sin tomam Float) — mesma descoberta do register_native.
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = crate::register::get_function(&reg, n);
        if crate::rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !crate::rtti::sane(proto) {
        return false;
    }
    crate::register::register_argful_by_types(&reg, proto, &types, fulls, shorts, handler)
}

unsafe extern "C" fn api_fire_event(name: *const c_char) -> usize {
    if name.is_null() {
        return 0;
    }
    match CStr::from_ptr(name).to_str() {
        Ok(s) => crate::register::fire_event(s),
        Err(_) => 0,
    }
}

/// Resolve o PONTEIRO da propriedade `field` (por nome, via RTTI) no objeto `obj`. Base dos get/set
/// tipados (v5). null-safe (obj/field nulos → null).
unsafe fn resolve_prop(obj: *mut c_void, field: *const c_char) -> *mut c_void {
    if obj.is_null() || field.is_null() {
        return std::ptr::null_mut();
    }
    let name = match CStr::from_ptr(field).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    crate::rtti::find_property_in_class(cls, name)
}

unsafe extern "C" fn api_prop_get_f32(obj: *mut c_void, field: *const c_char) -> f32 {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return 0.0;
    }
    crate::rtti::prop_get_f32(p, obj)
}
unsafe extern "C" fn api_prop_set_f32(obj: *mut c_void, field: *const c_char, val: f32) -> bool {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return false;
    }
    crate::rtti::prop_set_f32(p, obj, val);
    true
}
unsafe extern "C" fn api_prop_get_i32(obj: *mut c_void, field: *const c_char) -> i32 {
    let p = resolve_prop(obj, field);
    if p.is_null() {
        return 0;
    }
    crate::rtti::prop_get_u32(p, obj) as i32
}

unsafe extern "C" fn api_call_method_args(
    obj: *mut c_void,
    method: *const c_char,
    args: *const *const c_char,
    n_args: usize,
    ret16: *mut u8,
) -> bool {
    if obj.is_null() || method.is_null() {
        return false;
    }
    let name = match CStr::from_ptr(method).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    // parseia os N args (formato i:/f:/b:/n:/s:/e:, o mesmo do callf). null em qualquer um = aborta seguro.
    let mut parsed: Vec<crate::rtti::Arg> = Vec::with_capacity(n_args);
    if n_args > 0 {
        if args.is_null() {
            return false;
        }
        for i in 0..n_args {
            let p = *args.add(i);
            if p.is_null() {
                return false;
            }
            match CStr::from_ptr(p).to_str() {
                Ok(s) => parsed.push(crate::parse_cmd_arg(s)),
                Err(_) => return false,
            }
        }
    }
    let cls = crate::rtti::class_of(obj);
    if cls.is_null() {
        return false;
    }
    match crate::rtti::resolve_in_class(cls, name) {
        Some(rf) => match crate::rtti::call_func(&rf, obj, &parsed) {
            Some(r) => {
                if !ret16.is_null() {
                    std::ptr::copy_nonoverlapping(r.as_ptr(), ret16, 16);
                }
                true
            }
            None => false,
        },
        None => false,
    }
}

unsafe extern "C" fn api_tweakdb_get_flat(name: *const c_char, out_bits: *mut u32) -> bool {
    if name.is_null() || out_bits.is_null() {
        return false;
    }
    let names = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    match crate::tweakdb_rt::api_get_flat_scalar(names) {
        Some(bits) => {
            *out_bits = bits;
            true
        }
        None => false,
    }
}

unsafe extern "C" fn api_tweakdb_set_flat(name: *const c_char, bits: u32) -> bool {
    if name.is_null() {
        return false;
    }
    let names = match CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    crate::tweakdb_rt::api_set_flat_scalar(names, bits)
}

unsafe extern "C" fn api_tweakdb_clone_record(
    class_name: *const c_char,
    source: *const c_char,
    new_name: *const c_char,
) -> bool {
    if class_name.is_null() || source.is_null() || new_name.is_null() {
        return false;
    }
    let (c, s, n) = match (
        CStr::from_ptr(class_name).to_str(),
        CStr::from_ptr(source).to_str(),
        CStr::from_ptr(new_name).to_str(),
    ) {
        (Ok(c), Ok(s), Ok(n)) => (c, s, n),
        _ => return false,
    };
    crate::tweakdb_rt::clone_record_api(c, s, n);
    true
}

unsafe extern "C" fn api_log_level(level: u8, msg: *const c_char) {
    if msg.is_null() {
        return;
    }
    let tag = match level {
        0 => "TRACE",
        1 => "DEBUG",
        2 => "INFO",
        3 => "WARN",
        4 => "ERROR",
        5 => "CRIT",
        _ => "INFO",
    };
    crate::log(&format!("[plugin][{tag}] {}", CStr::from_ptr(msg).to_string_lossy()));
}

/// Parseia "major.minor.patch" (componentes ausentes/não-numéricos viram 0). Descarta o sufixo de
/// pre-release/build do SemVer (tudo a partir do 1º `-` ou `+`) ANTES de separar por `.` — senão
/// `"1.2.3-rc1"` dava `(1,2,0)` (o `"3-rc1"` não parseia como número e o patch caía pra 0). Aceita
/// também um prefixo `v`/`V` (`"v1.2.3"`). Cobre o caso real de `Codeware.Require` com versão de mod.
pub(crate) fn parse_semver_triplet(s: &str) -> (u32, u32, u32) {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next().unwrap_or(""); // fora pre-release/build
    let mut parts = core.splitn(3, '.');
    let major = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let minor = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    let patch = parts.next().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

unsafe extern "C" fn api_semver_satisfies(required: *const c_char, actual: *const c_char) -> bool {
    if required.is_null() || actual.is_null() {
        return false;
    }
    let (req, act) = match (CStr::from_ptr(required).to_str(), CStr::from_ptr(actual).to_str()) {
        (Ok(r), Ok(a)) => (r, a),
        _ => return false,
    };
    parse_semver_triplet(act) >= parse_semver_triplet(req)
}

/// Callbacks de draw registrados por plugins (`cet-imgui-thirdparty`). Chamados 1x por frame,
/// DENTRO do onDraw (mesmo ponto/gate que os mods Lua — `overlay::in_draw()`), por
/// `overlay.rs::render_imgui`. `Mutex<Vec<>>` — plugins carregam 1x no boot (thread única),
/// mas o registro pode, em teoria, vir de qualquer thread; a CHAMADA em si é sempre na render.
static PLUGIN_DRAW_CALLBACKS: std::sync::Mutex<Vec<extern "C" fn()>> = std::sync::Mutex::new(Vec::new());

unsafe extern "C" fn api_register_draw_callback(cb: extern "C" fn()) -> bool {
    if let Ok(mut v) = PLUGIN_DRAW_CALLBACKS.lock() {
        v.push(cb);
    }
    true
}

/// Chamado pelo `overlay.rs::render_imgui`, DENTRO do onDraw, 1x por frame — dispara todos os
/// draw callbacks registrados por plugins. Isolado (`catch_unwind`) — um plugin que panica no
/// draw não derruba o frame nem os outros plugins.
pub(crate) unsafe fn run_plugin_draw_callbacks() {
    let cbs: Vec<extern "C" fn()> = match PLUGIN_DRAW_CALLBACKS.lock() {
        Ok(v) => v.clone(),
        Err(_) => return,
    };
    for cb in cbs {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| cb()));
    }
}

unsafe extern "C" fn api_imgui_begin(title: *const c_char) -> bool {
    if !crate::overlay::in_draw() || title.is_null() {
        return true; // fora do onDraw: no-op "visível" (mesmo padrão do binding Lua)
    }
    let s = match CStr::from_ptr(title).to_str() {
        Ok(s) => s,
        Err(_) => return true,
    };
    let c = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return true,
    };
    imgui::sys::igBegin(c.as_ptr(), std::ptr::null_mut(), 0)
}

unsafe extern "C" fn api_imgui_text(text: *const c_char) {
    if !crate::overlay::in_draw() || text.is_null() {
        return;
    }
    if let Ok(s) = CStr::from_ptr(text).to_str() {
        if let Ok(c) = std::ffi::CString::new(s) {
            imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
        }
    }
}

unsafe extern "C" fn api_imgui_end() {
    if crate::overlay::in_draw() {
        imgui::sys::igEnd();
    }
}

/// Caminho do manifesto persistente (fora do save, mesmo padrão de `~/.bwms-modconfig.txt`).
fn scripts_add_manifest_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-scripts-add.txt"))
}

/// Lê+escreve o manifesto com dedup (1 path por linha). `pub(crate)` pra `bwms-fastboot.sh`/testes
/// não precisarem — o SHELL lê o arquivo direto; isto só existe pro handler da API escrever.
unsafe extern "C" fn api_scripts_add(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }
    let p = match CStr::from_ptr(path).to_str() {
        Ok(s) => s.trim().to_string(),
        Err(_) => return false,
    };
    if p.is_empty() {
        return false;
    }
    let manifest = match scripts_add_manifest_path() {
        Some(m) => m,
        None => return false,
    };
    let existing = std::fs::read_to_string(&manifest).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == p) {
        crate::log(&format!("[api] scripts_add: '{p}' já estava no manifesto (sem duplicar)"));
        return true;
    }
    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&p);
    content.push('\n');
    match std::fs::write(&manifest, content) {
        Ok(_) => {
            crate::log(&format!(
                "[api] scripts_add: '{p}' registrado em {manifest:?} (efetivo no PRÓXIMO compile+boot, redscript não hot-recarrega)"
            ));
            true
        }
        Err(e) => {
            crate::log(&format!("[api] scripts_add: falha ao escrever manifesto '{manifest:?}': {e}"));
            false
        }
    }
}

/// Instância única passada a TODOS os plugins (vive o processo inteiro — ponteiro sempre válido).
pub static BWMS_API: BwmsApi = BwmsApi {
    abi_version: crate::plugins::BWMS_PLUGIN_API,
    log: api_log,
    vtable_hook: api_vtable_hook,
    vtable_unhook: api_vtable_unhook,
    field_ptr: api_field_ptr,
    call_method: api_call_method,
    inline_hook: api_inline_hook,
    inline_revert: api_inline_revert,
    register_native: api_register_native,
    register_native_argful: api_register_native_argful,
    fire_event: api_fire_event,
    prop_get_f32: api_prop_get_f32,
    prop_set_f32: api_prop_set_f32,
    prop_get_i32: api_prop_get_i32,
    call_method_args: api_call_method_args,
    register_method: api_register_method,
    tweakdb_get_flat: api_tweakdb_get_flat,
    tweakdb_set_flat: api_tweakdb_set_flat,
    tweakdb_clone_record: api_tweakdb_clone_record,
    log_level: api_log_level,
    semver_satisfies: api_semver_satisfies,
    register_draw_callback: api_register_draw_callback,
    imgui_begin: api_imgui_begin,
    imgui_text: api_imgui_text,
    imgui_end: api_imgui_end,
    scripts_add: api_scripts_add,
};

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn api_version_matches() {
        assert_eq!(BWMS_API.abi_version, crate::plugins::BWMS_PLUGIN_API);
    }

    // Os ponteiros de função são ITENS => não-nulos por construção. Aqui provo os caminhos
    // null-safe (o plugin pode passar lixo sem derrubar o jogo). Tudo bate no guard antes de
    // tocar o RTTI, então roda sem o runtime/jogo (autônomo).
    #[test]
    fn null_safe_paths() {
        unsafe {
            assert!((BWMS_API.vtable_hook)(std::ptr::null_mut(), 0, std::ptr::null()).is_null());
            let f = CString::new("Health").unwrap();
            assert!((BWMS_API.field_ptr)(std::ptr::null_mut(), f.as_ptr()).is_null());
            assert!(!(BWMS_API.call_method)(std::ptr::null_mut(), f.as_ptr(), std::ptr::null_mut()));
            let mut dummy = 0u64;
            let obj = &mut dummy as *mut u64 as *mut c_void;
            assert!((BWMS_API.field_ptr)(obj, std::ptr::null()).is_null());
            assert!(!(BWMS_API.call_method)(obj, std::ptr::null(), std::ptr::null_mut()));
            assert!((BWMS_API.inline_hook)(std::ptr::null_mut(), std::ptr::null_mut()).is_null());
        }
    }

    // handler no-op só p/ satisfazer a assinatura nos testes (nunca é chamado: os guards abortam antes).
    unsafe extern "C" fn test_handler(_c: *mut c_void, _f: *mut c_void, _r: *mut c_void, _rt: i64) {}

    // v4: as duas fns novas são null-safe e não tocam o RTTI antes dos guards (autônomo, sem jogo).
    #[test]
    fn v4_null_safe() {
        unsafe {
            let dummy: crate::register::NativeHandler = test_handler;
            let s = CString::new("X").unwrap();
            // full/short null → false; param_types null com n>0 → false.
            assert!(!(BWMS_API.register_native_argful)(std::ptr::null(), s.as_ptr(), dummy, std::ptr::null(), 0));
            assert!(!(BWMS_API.register_native_argful)(s.as_ptr(), std::ptr::null(), dummy, std::ptr::null(), 0));
            assert!(!(BWMS_API.register_native_argful)(s.as_ptr(), s.as_ptr(), dummy, std::ptr::null(), 2));
            // fire_event(null) → 0; nome válido sem jogo/callbacks → 0 (sem crash).
            assert_eq!((BWMS_API.fire_event)(std::ptr::null()), 0);
            assert_eq!((BWMS_API.fire_event)(s.as_ptr()), 0);
            // v5: get/set tipado null-safe (obj/field nulos → default, sem tocar RTTI).
            assert_eq!((BWMS_API.prop_get_f32)(std::ptr::null_mut(), s.as_ptr()), 0.0);
            assert!(!(BWMS_API.prop_set_f32)(std::ptr::null_mut(), s.as_ptr(), 1.0));
            assert_eq!((BWMS_API.prop_get_i32)(std::ptr::null_mut(), s.as_ptr()), 0);
            let mut d = 0u64;
            let obj = &mut d as *mut u64 as *mut c_void;
            assert_eq!((BWMS_API.prop_get_f32)(obj, std::ptr::null()), 0.0);
            // v6: call_method_args null-safe (obj/method nulos e args null com n>0 → false, sem tocar RTTI).
            assert!(!(BWMS_API.call_method_args)(std::ptr::null_mut(), s.as_ptr(), std::ptr::null(), 0, std::ptr::null_mut()));
            assert!(!(BWMS_API.call_method_args)(obj, std::ptr::null(), std::ptr::null(), 0, std::ptr::null_mut()));
            assert!(!(BWMS_API.call_method_args)(obj, s.as_ptr(), std::ptr::null(), 2, std::ptr::null_mut()));
            // v8: TweakDB null-safe (name nulo, out_bits nulo, ou singleton indisponível fora do
            // jogo real → false/sem crash, nunca toca o TweakDB sem checar antes).
            let mut bits: u32 = 0;
            assert!(!(BWMS_API.tweakdb_get_flat)(std::ptr::null(), &mut bits));
            assert!(!(BWMS_API.tweakdb_get_flat)(s.as_ptr(), std::ptr::null_mut()));
            assert!(!(BWMS_API.tweakdb_get_flat)(s.as_ptr(), &mut bits)); // sem jogo: singleton() None
            assert!(!(BWMS_API.tweakdb_set_flat)(std::ptr::null(), 0));
            assert!(!(BWMS_API.tweakdb_set_flat)(s.as_ptr(), 0)); // sem jogo: singleton() None
            assert!(!(BWMS_API.tweakdb_clone_record)(std::ptr::null(), s.as_ptr(), s.as_ptr()));
            assert!(!(BWMS_API.tweakdb_clone_record)(s.as_ptr(), std::ptr::null(), s.as_ptr()));
            assert!(!(BWMS_API.tweakdb_clone_record)(s.as_ptr(), s.as_ptr(), std::ptr::null()));
            // v9: semver_satisfies null-safe + lógica de comparação (autônomo, sem RTTI/jogo).
            assert!(!(BWMS_API.semver_satisfies)(std::ptr::null(), s.as_ptr()));
            assert!(!(BWMS_API.semver_satisfies)(s.as_ptr(), std::ptr::null()));
        }
    }

    #[test]
    fn semver_satisfies_compara_versoes() {
        unsafe {
            let req = CString::new("1.2.0").unwrap();
            let higher = CString::new("1.3.0").unwrap();
            let equal = CString::new("1.2.0").unwrap();
            let lower = CString::new("1.1.9").unwrap();
            let major_higher = CString::new("2.0.0").unwrap();
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), higher.as_ptr()));
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), equal.as_ptr()));
            assert!(!(BWMS_API.semver_satisfies)(req.as_ptr(), lower.as_ptr()));
            assert!((BWMS_API.semver_satisfies)(req.as_ptr(), major_higher.as_ptr()));
            // componente ausente/malformado vira 0 — não crasha, só compara.
            let partial_req = CString::new("1.2").unwrap();
            let partial_act = CString::new("1").unwrap();
            assert!(!(BWMS_API.semver_satisfies)(partial_req.as_ptr(), partial_act.as_ptr()));
        }
    }

    #[test]
    fn parse_semver_triplet_pre_release_e_v_prefixo() {
        // O bug corrigido: pre-release não pode zerar o patch.
        assert_eq!(parse_semver_triplet("1.2.3-rc1"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("1.2.3+build.5"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("1.2.3-rc.1+build"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("v1.2.3"), (1, 2, 3));
        assert_eq!(parse_semver_triplet("V2.0.0-beta"), (2, 0, 0));
        assert_eq!(parse_semver_triplet(" 1.2.3 "), (1, 2, 3));
        // sem o fix, "1.0.0" >= "1.0.0-rc1" seria falso-negativo (rc1 virava 1.0.0 tb, ok aqui;
        // o ponto é o patch não sumir): 1.2.3-rc1 satisfaz require 1.2.3.
        assert!(parse_semver_triplet("1.2.3-rc1") >= parse_semver_triplet("1.2.3"));
    }

    // log_level não tem valor de retorno pra checar, mas prova que não crasha (inclui msg null).
    #[test]
    fn log_level_nao_crasha() {
        unsafe {
            (BWMS_API.log_level)(2, std::ptr::null());
            let msg = CString::new("teste de log com nivel").unwrap();
            for lvl in 0..=6u8 {
                (BWMS_API.log_level)(lvl, msg.as_ptr());
            }
        }
    }
}
