//! Tipos do motor: reflexão (classes, propriedades, funções, enums), chamar métodos, callbacks, overrides e sistemas por nome.

#![allow(dead_code)]

use crate::tempo::ORIG_UPDATE_REGISTRAR_GROUP;
use crate::tempo::UPDATE_REGISTRAR_GROUP_VM;
use crate::tempo::update_registrar_group_probe;
use crate::tempo::{EVENTOS_DO_PIPELINE, PIPELINE_ARMADOS};
use crate::ganchos::install_hook_probe;
use crate::veiculos::VEHICLE_LIGHT_ARMADO;
use crate::mods_do_usuario::tramp_require;
use crate::mods_do_usuario::tramp_version;
use crate::mods_do_usuario::PLUGUE_VERSAO;
use crate::entidades::*;
use crate::diagnostico::{global_do_gettype, CLONE_MAIOR};
use crate::aparencia::*;
use crate::corpo_e_rosto::*;
use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;

/// Expõe Reflection GETF pro redscript: `BwmsGetPlayerField(field: CName) -> Float` — lê o CName
/// arg, acha a propriedade por nome no player vivo (find_property + prop_get_f32) e retorna. Prova
/// arg CName + Reflection-pro-redscript (mods leem campos do player por nome). Player via current_player().
pub(crate) unsafe extern "C" fn tramp_getfield(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let player = crate::current_player();
    let mut val = f32::NAN;
    let mut found = false;
    if !player.is_null() {
        let prop = rtti::find_property_in_class(rtti::class_of(player), &name);
        if !prop.is_null() {
            val = rtti::prop_get_f32(prop, player);
            found = true;
        }
    }
    crate::log_dev!(
        "[getfield] BwmsGetPlayerField('{name}' hash {hash:#x}) = {val} (achou={found}) — Reflection getf VIA REDSCRIPT"
    );
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u32, val.to_bits());
    }
}

/// Expõe Reflection SETF pro redscript: `BwmsSetPlayerField(field: CName, value: Float) -> Bool` —
/// escreve um campo do player por nome. Lê (CName, Float), find_property + prop_set_f32, round-trip log.
pub(crate) unsafe extern "C" fn tramp_setfield(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let raw_f = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let value = f32::from_bits(raw_f as u32);
    let name = crate::cname::resolve_cname(hash);
    let player = crate::current_player();
    let mut ok = false;
    if !player.is_null() {
        let prop = rtti::find_property_in_class(rtti::class_of(player), &name);
        if !prop.is_null() {
            let before = rtti::prop_get_f32(prop, player);
            rtti::prop_set_f32(prop, player, value);
            let after = rtti::prop_get_f32(prop, player);
            ok = true;
            crate::log_dev!(
                "[setfield] BwmsSetPlayerField('{name}', {value}) = {before} -> {after} — Reflection setf VIA REDSCRIPT"
            );
        }
    }
    if !ok {
        crate::log_dev!("[setfield] BwmsSetPlayerField('{name}', {value}) — prop não achada");
    }
    write_bool_ret(out, ok);
}

/// `BwmsPropOffset(obj, prop) -> Int32` — o offset que o RTTI reporta, ou `-1`.
///
/// Diagnóstico de primeira classe: com ele dá pra CONFERIR qualquer offset de header contra o
/// que esta build realmente usa, antes de escrever leitura nenhuma.
pub(crate) unsafe extern "C" fn tramp_prop_offset(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let obj = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let nome = strings.get(1).cloned().flatten().unwrap_or_default();
    match prop_em(obj, &nome) {
        Some((cls, off, _)) => {
            crate::log_dev!("[prop] {obj:p} '{cls}'::{nome} @ {off:#x}");
            write_i32_ret(out, off as i32);
        }
        None => write_i32_ret(out, -1),
    }
}

/// `BwmsPropU64(obj, prop) -> Uint64` — lê 8 bytes no offset que o RTTI reporta.
///
/// É a via genérica que o `#30` precisava: `mesh` é `RaRef<CMesh>`, e `ResourceAsyncReference`
/// guarda só um `ResourcePath` (u64). Daí em diante a cadeia hash → mapa no bank `[loader+0x48]` →
/// `CResource*` já é provada (`#18`). Serve igual pra `chunkMask`, `globalNodeID`, e pros ~1.107
/// campos que mods expõem por `@addField` e que o Mac não tem como property script-visível.
pub(crate) unsafe extern "C" fn tramp_prop_u64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let obj = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let nome = strings.get(1).cloned().flatten().unwrap_or_default();
    // Largura opcional (default 8). Importa: um enum de 1 byte lido como 8 traz os campos
    // VIZINHOS junto e vira um número que parece valor e não é — e perto do fim do objeto a
    // leitura de 8 ainda pode cair fora e devolver 0 como se o campo fosse zero.
    let largura = a.get(2).map(|(v, _)| *v as u8).filter(|w| matches!(w, 1 | 2 | 4 | 8)).unwrap_or(8);
    let mut v = 0u64;
    if let Some((cls, off, alvo)) = prop_em(obj, &nome) {
        if alvo != 0 && crate::memoria::is_readable(alvo as *const c_void, largura as usize) {
            v = match largura {
                1 => (alvo as *const u8).read_unaligned() as u64,
                2 => (alvo as *const u16).read_unaligned() as u64,
                4 => rd_u32(alvo as *const c_void, 0) as u64,
                _ => rd_u64(alvo as *const c_void, 0),
            };
        }
        crate::log_dev!("[prop] {cls}::{nome} @ {off:#x} ({largura}B) = {v:#x}");
    }
    write_uint_ret(out, v, 8);
}

/// `BwmsPropSetU64(obj, prop, largura, valor) -> Bool` — o lado da ESCRITA do `#91`.
///
/// # Por que existe
///
/// `BwmsPropOffset`/`BwmsPropU64` já provaram a LEITURA de campo nativo de widget pelo offset que o
/// RTTI reporta, com round-trip validado pelo setter do próprio motor. O item pede `Get/Set`, e o
/// `Set` não existia — sem ele metade da superfície é só leitura.
///
/// # A largura NÃO é detalhe
///
/// Escrever 8 bytes num enum de 1 byte não é "um pouco impreciso": **destrói os campos vizinhos**.
/// A leitura com largura errada devolve um número esquisito e você percebe; a escrita com largura
/// errada corrompe o objeto silenciosamente e o efeito aparece longe da causa. Por isso a largura é
/// obrigatória na assinatura, só 1/2/4/8 são aceitos, e um valor que não cabe na largura pedida é
/// RECUSADO em vez de truncado — truncar seria escrever um valor que o chamador não pediu.
///
/// # O critério de prova
///
/// Espelho exato do teste de leitura: aqui a NATIVE escreve e o **motor** lê pelo getter dele
/// (`GetLetterCase()`), em vez de comparar contra o que eu mesmo acabei de gravar. Se o getter do
/// motor devolve o valor que a native escreveu, a escrita caiu no lugar certo E está conectada à
/// lógica real — que é a objeção que a ficha do item levantava contra `@addField`.
pub(crate) unsafe extern "C" fn tramp_prop_set_u64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let obj = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let nome = strings.get(1).cloned().flatten().unwrap_or_default();
    let largura = a.get(2).map(|(v, _)| *v as u8).filter(|w| matches!(w, 1 | 2 | 4 | 8)).unwrap_or(0);
    let valor = a.get(3).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if largura != 0 {
        // Cabe na largura? Truncar em silêncio escreveria um valor que ninguém pediu.
        let cabe = match largura {
            1 => valor <= 0xff,
            2 => valor <= 0xffff,
            4 => valor <= 0xffff_ffff,
            _ => true,
        };
        if cabe {
            if let Some((cls, off, alvo)) = prop_em(obj, &nome) {
                let alvo = alvo as *mut c_void;
                if !alvo.is_null() && crate::memoria::is_readable(alvo as *const c_void, largura as usize) {
                    match largura {
                        1 => core::ptr::write_unaligned(alvo as *mut u8, valor as u8),
                        2 => core::ptr::write_unaligned(alvo as *mut u16, valor as u16),
                        4 => core::ptr::write_unaligned(alvo as *mut u32, valor as u32),
                        _ => core::ptr::write_unaligned(alvo as *mut u64, valor),
                    }
                    ok = true;
                    crate::log_dev!("[propset] {cls}::{nome} @ {off:#x} ({largura}B) <- {valor:#x}");
                }
            }
        } else {
            crate::log_dev!(
                "[propset] '{nome}': valor {valor:#x} NAO CABE em {largura}B — recusado (truncar seria escrever outra coisa)"
            );
        }
    }
    write_bool_ret(out, ok);
}

/// A `CProperty` do 2º parâmetro do `IBlackboard.SetVariant` (`value: Variant`), a mesma que as natives de variante já provadas usam
/// pelo [`compose_params`]. O `Variant` montado pelo nome do tipo não registrou no boot 267.
pub(crate) unsafe fn cprop_variant(reg: &Registry) -> u64 {
    rtti::resolve_func(reg, "gameIBlackboard", "SetVariant").map_or(0, |sv| rd_u64(compose_params(&[(sv.func, 1)]).0 as *const c_void, 0))
}

/// CallbackSystem DISPATCH: `BwmsCallMethod(target: ref<IScriptable>, function: CName) -> Bool` —
/// chama um método no-arg de QUALQUER objeto (não só player) por nome. Lê o Handle (obj ptr) + CName,
/// resolve_in_class(class_of(target)) + call_func. É o núcleo do dispatch dinâmico do CallbackSystem.
pub(crate) unsafe extern "C" fn tramp_callmethod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let mut ok = false;
    let mut ret = 0i32;
    if !target.is_null() && rtti::sane(target) {
        if let Some(rf) = rtti::resolve_in_class(rtti::class_of(target), &name) {
            if let Some(r) = rtti::call_func(&rf, target, &[]) {
                ret = i32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                ok = true;
            }
        }
    }
    crate::log_dev!(
        "[callmethod] BwmsCallMethod(target={target:p}, '{name}') ok={ok} ret={ret} — DISPATCH ARBITRÁRIO (CallbackSystem core)"
    );
    write_bool_ret(out, ok);
}

/// `BwmsCallMethodWithArg(target: ref<IScriptable>, function: CName, arg: ref<IScriptable>) -> Bool` — `BwmsCallMethod` com 1
/// argumento `ref<>` (2026-09-11, `ResourceToken.RegisterCallback`, que chama `listener.function(token)`). O argumento
/// vira um `Handle` pela rotina do motor (`make_handle`, +1 na contagem forte) e NÃO é solto depois: o `call_func` grava os 16 bytes
/// direto nos locals, e se o frame chamado soltar o parâmetro a conta fecha; se não soltar, sobra 1 referência. Nunca solta a mais.
pub(crate) unsafe extern "C" fn tramp_callmethod_with_arg(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let target = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let arg = args.get(2).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let name = crate::cname::resolve_cname(hash);
    let mut ok = false;
    if rtti::sane(target) && rtti::sane(arg) {
        if let Some(rf) = rtti::resolve_in_class(rtti::class_of(target), &name) {
            let mut h = [0u64; 2];
            rtti::make_handle(h.as_mut_ptr() as *mut c_void, arg);
            ok = rtti::call_func(&rf, target, &[rtti::Arg::Handle(h[0] as *mut c_void, h[1] as *mut c_void)]).is_some();
        }
    }
    crate::log_dev!("[callmethod] BwmsCallMethodWithArg(target={target:p}, '{name}', arg={arg:p}) ok={ok}");
    write_bool_ret(out, ok);
}

/// `BwmsRegisterCallback(eventName: CName, target: ref<IScriptable>, function: CName) -> Bool` — a via global do
/// `CallbackSystem.RegisterCallback`. 3 args via compose_params_from_types. O log mora aqui e não em `registrar_pela_via_global`: o
/// `resolve_cname` lê o pool de nomes do motor, e o corpo é testado offline também com `--features devtools`.
pub(crate) unsafe extern "C" fn tramp_register_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let guardado = registrar_pela_via_global(ev, target, fn_hash);
    crate::log_dev!(
        "[cbs] BwmsRegisterCallback(event '{}' guardado como '{}', target={target:#x}, fn '{}') ok={}",
        crate::cname::resolve_cname(ev),
        crate::cname::resolve_cname(guardado.unwrap_or(0)),
        crate::cname::resolve_cname(fn_hash),
        guardado.is_some()
    );
    write_bool_ret(out, guardado.is_some());
}

/// Corpo da `BwmsRegisterCallback` com os argumentos já lidos do frame: entra pela mesma porta da classe (`registrar_ouvinte`), sem handler. Sem
/// handler, a entrada não tem `AddTarget`, `RunMode` nem `Lifetime`: não sai no `Session/End` e só sai por `CallbackSystem.UnregisterCallback(evento,
/// alvo)`. Devolve o hash do evento guardado em `CALLBACKS`.
pub(crate) unsafe fn registrar_pela_via_global(ev_bruto: u64, alvo: u64, fn_hash: u64) -> Option<u64> {
    registrar_ouvinte(ev_bruto, alvo as usize, fn_hash, 0)
}

// ===== `CallbackSystem` NATIVO real (CallbackSystem no RTTI, 2026-07-13) ============================
// Mesmo fix de timing/parent que fechou a Facade (parent-pointer explícito + fullName BARE nos
// métodos), aplicado a uma classe INSTANCIÁVEL (não abstract) — porque `GetCallbackSystem()`
// precisa devolver um objeto de verdade. A instância é construída PREGUIÇOSAMENTE (só na 1ª
// chamada de `GetCallbackSystem`, já em gameplay normal) — o forge da classe + registro dos
// métodos continua rodando cedo (mesma janela seguro-por-construção da Facade), sem tentar
// `new_object` durante a janela frágil do bind. Ver `register_callbacksystem` pro forge completo
// e o escopo exato (o que é funcional vs. stub) de cada um dos 7 métodos.
pub(crate) static CBS_INSTANCE: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// `GameInstance.GetCallbackSystem() -> ref<CallbackSystem>` — devolve o singleton, construindo-o
/// PREGUIÇOSAMENTE (`new_object`, só na 1ª chamada) — roda em contexto de gameplay normal (um mod
/// chamando isto já está bem depois do boot), não na janela frágil do bind. `new_object` usa o
/// vtable/size do MESMO donor (`gameuiInGameMenuGameController`) do forge da classe — Construct
/// escreve dentro do size certo, sem corromper heap (mesmo mecanismo já provado por `cwregalias`+
/// `newobj` em 2026-07-12).
pub(crate) unsafe extern "C" fn tramp_get_callback_system(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut ptr = CBS_INSTANCE.load(Ordering::Acquire);
    if ptr.is_null() {
        if let Some(reg) = rtti::Registry::obtain() {
            let obj = rtti::new_object(&reg, "CallbackSystem");
            if !obj.is_null() {
                CBS_INSTANCE.store(obj, Ordering::Release);
                ptr = obj;
                crate::log_dev!("[cbs] GetCallbackSystem: instância construída {obj:p} (lazy, 1a chamada)");
            } else {
                crate::log_dev!("[cbs] GetCallbackSystem: new_object('CallbackSystem') falhou");
            }
        } else {
            crate::log_dev!("[cbs] GetCallbackSystem: Registry::obtain() falhou");
        }
    }
    write_handle_ret(out, ptr as u64); // ref<T>, ver write_handle_ret
}

/// `BwmsSerializableResourceHash(obj) -> Uint64`: `+0x30` do `CResource` dentro do `BwmsSerializable` (0 fora de recurso).
pub(crate) unsafe extern "C" fn tramp_resource_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let obj = recurso_do_script(a.first().map_or(0, |(v, _)| *v));
    let e_recurso = obj != 0 && classe_deriva_de_nome(rtti::class_of(obj as *mut c_void), crate::cname::cname("CResource"));
    let hash = if e_recurso && crate::memoria::is_readable((obj + 0x30) as *const c_void, 8) { ((obj + 0x30) as *const u64).read_unaligned() } else { 0 };
    write_uint_ret(out, hash, 8);
}

/// `BwmsSerializableClassName(obj) -> CName`: nome da classe do objeto dentro do `BwmsSerializable`.
pub(crate) unsafe extern "C" fn tramp_resource_class_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let cls = rtti::class_of(recurso_do_script(a.first().map_or(0, |(v, _)| *v)) as *mut c_void);
    write_uint_ret(out, if cls.is_null() { 0 } else { rtti::type_name_hash(cls) }, 8);
}

/// `BwmsSerializableIsA(obj, className) -> Bool`: o `IsA(GetClass(className))`.
pub(crate) unsafe extern "C" fn tramp_resource_is_a(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let cls = rtti::class_of(recurso_do_script(a.first().map_or(0, |(v, _)| *v)) as *mut c_void);
    let classe = a.get(1).and_then(|(nome, _)| rtti::Registry::obtain().map(|reg| reg.script_class(*nome))).unwrap_or(std::ptr::null_mut());
    write_bool_ret(out, class_is_kind_of(cls, classe));
}

pub(crate) unsafe fn register_resource_natives(reg: &Registry, proto: *mut c_void) {
    let r = [
        register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsSerializableResourceHash", "BwmsSerializableResourceHash", tramp_resource_hash as NativeHandler),
        register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsSerializableClassName", "BwmsSerializableClassName", tramp_resource_class_name as NativeHandler),
        register_argful_by_types(reg, proto, &["handle:IScriptable", "CName"], "BwmsSerializableIsA", "BwmsSerializableIsA", tramp_resource_is_a as NativeHandler),
    ];
    crate::log_dev!("[reg] register_all: BwmsSerializable {}/3 registradas", r.iter().filter(|x| **x).count());
}

// `Scripting::GetStackTrace` (item #60, agente de pesquisa 2026-08-11): anda `frame->parent` (o `CStackFrame*` que TODO native já
// recebe automaticamente, aqui é literalmente o parâmetro `frame` desta trampolina), lê `func@
// +0x08`/`context@+0x40`/`parent@+0x48` (offsets de CAMPO confirmados — offsets de campo são portáveis 1:1
// Windows↔Mac, regra já estabelecida dezenas de vezes neste projeto). **Divergência consciente
// de escopo (mais segura que replicar 1:1):** em vez de devolver `array<StackTraceEntry>` (a
// técnica "array-retorno-de-struct-composto" nunca foi tentada neste projeto — só ponteiros/
// handles têm a técnica de trailer-de-allocator provada), exponho GETTER INDEXADO (mesmo padrão
// já usado com segurança em `GetPropertyCount`+`GetPropertyByIndex`) — `GetStackDepth()` conta
// frames válidos, `GetStackFrameFunction(i)`/`GetStackFrameClass(i)` leem 1 frame por vez. Também
// omito o fallback `func->GetParent()` (método C++, não campo — usado só quando `context` é
// null, chamada de função global/estática) por simplicidade consciente: nesse caso `GetStackFrameClass`
// devolve `0`/vazio, divergência documentada, não afeta o caso comum (chamada de método).
pub(crate) unsafe fn stack_frame_at(start: *mut c_void, index: i32) -> *mut c_void {
    if index < 0 || index >= 128 || !rtti::sane(start) {
        return std::ptr::null_mut();
    }
    let mut f = start;
    // +1: pula o frame da PRÓPRIA chamada nativa (equivale a `aWithCurrent=false`, o default
    // real do C++ — `frame = aFrame->parent` já pula 1, depois mais 1 se `!aWithCurrent`).
    for _ in 0..=index {
        if !crate::memoria::is_readable(f as *const c_void, 0x50) {
            return std::ptr::null_mut();
        }
        f = rd_u64(f as *const c_void, 0x48) as *mut c_void; // CStackFrame.parent@+0x48
        if f.is_null() {
            return std::ptr::null_mut();
        }
    }
    let func = rd_u64(f as *const c_void, 0x08) as *mut c_void; // CStackFrame.func@+0x08
    if func.is_null() {
        return std::ptr::null_mut();
    }
    f
}

pub(crate) unsafe extern "C" fn tramp_stack_get_depth(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut n = 0i32;
    while n < 128 && !stack_frame_at(frame, n).is_null() {
        n += 1;
    }
    write_uint_ret(out, n as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_stack_get_frame_function(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let idx = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let f = stack_frame_at(frame, idx);
    let hash = if !f.is_null() {
        let func = rd_u64(f as *const c_void, 0x08) as *const u8;
        if crate::memoria::is_readable(func as *const c_void, 0x18) { rd_u64(func as *const c_void, 0x08) } else { 0 } // fullName@+0x08
    } else {
        0
    };
    write_uint_ret(out, hash, 8);
}

pub(crate) unsafe extern "C" fn tramp_stack_get_frame_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let idx = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let f = stack_frame_at(frame, idx);
    let mut hash = 0u64;
    if !f.is_null() {
        let ctx = rd_u64(f as *const c_void, 0x40) as *mut c_void; // CStackFrame.context@+0x40
        if rtti::sane(ctx) {
            let cls = rtti::class_of(ctx);
            if !cls.is_null() {
                hash = rtti::type_name_getname(cls);
            }
        }
    }
    write_uint_ret(out, hash, 8);
}

// ===== Ângulo NOVO 2026-08-14 (boot dedicado
// de descoberta, `gisysmapdump 200`): esses 6 itens estavam TODOS catalogados como bloqueados
// por "singleton nunca localizado"/"interfaces internas nunca portadas" (`gameICommunitySystem`/
// `gameIPopulationSystem`/`gameIPersistencySystem`/`gameIContainerManager`/`gameIJournalManager`/
// `gamemappinsIMappinSystem`/`questIQuestsSystem`/`IDynamicEntityIDSystem`/`IEntityStubSystem`/
// `IEntitySpawnerEventsBroadcaster`/`redTagSystem`). O dump AO VIVO do `systemMap` real (149
// entradas, GAMEPLAY confirmada) mostra que quase TODAS essas interfaces são sistemas VANILLA
// REAIS já registrados sob nomes conhecidos: `gamePopulationSystem`, `gamePersistencySystem`,
// `gameContainerManager`, `gameJournalManager`, `gamemappinsMappinSystem`, `questQuestsSystem`,
// `gameDynamicEntityIDSystem`, `gameEntityStubSystem`, `gameEntitySpawnerEventsBroadcasterImpl`,
// `gameGameTagSystem`, `questWorldStateSystem`, `gameCommunitySystem` — 4 delas CONFIRMADAS ao
// vivo nesta mesma rodada via `sysbymap` (ponteiro não-nulo, mesmo mecanismo já provado):
// `questWorldStateSystem`/`gamePopulationSystem`/`gamePersistencySystem`/
// `gameContainerManager`. Só `FactManager` (citado pelo item #141) não apareceu no dump — não é
// um `IGameSystem` do `systemMap`, precisa de outra via.
//
// Isso NÃO fecha nenhum item sozinho: `funclistdump` contra 2 dos alvos (`questWorldStateSystem`/
// `gamePopulationSystem`) devolveu 0 métodos de instância — as classes existem no RTTI mas não
// expõem API própria pro redscript (mesmo padrão já visto no MappinSystem/CommunitySystem, que
// precisam de dispatch por SLOT DE VTABLE, não de reflection). O que MUDA de verdade: a
// caracterização "singleton nunca localizado" (repetida em várias notas do catálogo pros itens
// acima) está DESATUALIZADA — o ponteiro é obtível de forma barata e seguro (mesma técnica
// read-only já provada, `jogo::sessao::sistema_do_motor`, zero execução de código do motor).
// O bloqueador real que sobra é RE de vtable-slot POR CLASSE (mesma categoria de esforço que já
// resolveu `MappinSystem`/`CommunitySystem`), não mais "não sei nem onde o objeto vive".
//
// `BwmsGetSystemByName(name: CName) -> wref<IScriptable>` generaliza o mecanismo (hoje só
// existente como comando de diagnóstico `sysbymap`/`gisysmapdump`, não-redscript-callable) numa
// capacidade PRÓPRIA do BWMS: qualquer mod pode pedir QUALQUER uma das 149 entradas reais do
// `systemMap` por nome, sem precisar de um `GameInstance.GetXSystem()` vanilla dedicado (que só
// existe pra uma fração desses sistemas). `wref<>` (não-owning) — mesma semântica já usada pra
// `BwmsInkLayerGetWindow`/`GetGameController` (objeto ENGINE-gerenciado, vive pelo resto da
// sessão, nunca precisa de retenção forte).
pub(crate) unsafe extern "C" fn tramp_get_system_by_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    // O tipo sai direto do hash do `CName` (sem voltar pra texto) e o `GameInstance` da fonte única; o `GetGame` só roda se a classe existe.
    let sys = match (hash, crate::registry()) {
        (0, _) | (_, None) => std::ptr::null_mut(),
        (_, Some(reg)) => {
            let tipo = reg.class_by_hash(hash);
            if tipo.is_null() {
                std::ptr::null_mut()
            } else {
                crate::jogo::sessao::game_instance_ptr().map_or(std::ptr::null_mut(), |game| crate::jogo::sessao::sistema_do_motor(game, tipo))
            }
        }
    };
    crate::log_dev!("[sysbyname] BwmsGetSystemByName('{}') -> {sys:p}", crate::cname::resolve_cname(hash));
    rtti::make_weak_handle(out, sys);
}

pub(crate) unsafe fn register_system_by_name_utils(reg: &Registry, proto: *mut c_void) {
    let ok = register_argful_by_types(reg, proto, &["CName"], "BwmsGetSystemByName", "BwmsGetSystemByName", tramp_get_system_by_name);
    crate::log_dev!(
        "[reg] register_all: cw-75-76-138-139-141-143 BwmsGetSystemByName={ok} (systemMap por nome, generaliza sysbymap pro redscript)"
    );
}

// ===== `IScriptable.Clone()` (`bwms-clone.reds`) =====
//
// A referência clona pelo serializador do motor (`ISerializable::Clone`, sem endereço nesta build: nem a string nem o símbolo existem). Aqui a cópia sai do
// `CreateInstance` da classe e cada propriedade de RTTI passa pelo `Assign` do tipo; depois, handle forte vira o clone do objeto apontado (um clone por
// objeto de origem, então referência compartilhada continua compartilhada) e handle fraco que aponta pra algo clonado passa a apontar pro clone. Desce em
// struct e em array dinâmico; array estático e nativo ficam com o `Assign`. Passado o teto de objetos ou de profundidade, o handle fica no original.
pub(crate) const CLONE_TETO: usize = 4096;

pub(crate) const CLONE_PROFUNDIDADE: u32 = 64;

pub(crate) const TIPO_ARRAY: u8 = 3;

pub(crate) const TIPO_HANDLE: u8 = 9;

pub(crate) const TIPO_FRACO: u8 = 10;

pub(crate) struct Clonagem {
    pub(crate) copias: std::collections::HashMap<usize, [u64; 2]>,
    pub(crate) fracos: Vec<(*mut u64, usize)>,
    pub(crate) rasos: u32,
    pub(crate) profundidade: u32,
}

/// Clone de `obj` (Handle forte guardado em `copias`), o mesmo clone se `obj` já foi clonado nesta chamada; `None` sem classe instanciável ou no teto.
pub(crate) unsafe fn clonar_objeto(c: &mut Clonagem, obj: *mut c_void) -> Option<[u64; 2]> {
    if let Some(h) = c.copias.get(&(obj as usize)) {
        return Some(*h);
    }
    if c.copias.len() >= CLONE_TETO || c.profundidade >= CLONE_PROFUNDIDADE {
        c.rasos += 1;
        return None;
    }
    let cls = rtti::class_of(obj);
    let novo = if rtti::sane(cls) { rtti::create_instance(cls) } else { std::ptr::null_mut() };
    if novo.is_null() {
        return None;
    }
    let mut h = [0u64; 2];
    rtti::make_handle(h.as_mut_ptr() as *mut c_void, novo);
    c.copias.insert(obj as usize, h);
    c.profundidade += 1;
    for (tipo, off, no_holder) in rtti::propriedades_com_holder(cls) {
        let (origem, destino) = (rtti::field_ptr(obj, off, no_holder), rtti::field_ptr(novo, off, no_holder));
        if !origem.is_null() && !destino.is_null() && rtti::type_assign(tipo, destino, origem) {
            clonar_valor(c, tipo, destino as *mut u8);
        }
    }
    c.profundidade -= 1;
    Some(h)
}

/// Desce num valor já copiado pelo `Assign`: troca handle forte pelo clone, anota handle fraco pro fim, entra em struct e em array dinâmico.
pub(crate) unsafe fn clonar_valor(c: &mut Clonagem, tipo: *mut c_void, valor: *mut u8) {
    match rtti::type_kind(tipo) {
        Some(TIPO_HANDLE) => {
            let alvo = (valor as *const u64).read_unaligned();
            if alvo != 0 {
                if let Some(h) = clonar_objeto(c, alvo as *mut c_void) {
                    crate::jogo::objetos::gravar_handle_forte(valor as *mut u64, h);
                }
            }
        }
        Some(TIPO_FRACO) => {
            let alvo = (valor as *const u64).read_unaligned();
            if alvo != 0 {
                c.fracos.push((valor as *mut u64, alvo as usize));
            }
        }
        Some(TIPO_ARRAY) => {
            let Some(interno) = rtti::get_inner_type(tipo) else { return };
            if !matches!(rtti::type_kind(interno), Some(TIPO_CLASSE | TIPO_ARRAY | TIPO_HANDLE | TIPO_FRACO)) {
                return;
            }
            let passo = rtti::type_size(interno) as usize;
            let dados = (valor as *const u64).read_unaligned() as *mut u8;
            let n = (valor.add(0x0c) as *const u32).read_unaligned() as usize;
            if !dados.is_null() && passo != 0 {
                for k in 0..n {
                    clonar_valor(c, interno, dados.add(k * passo));
                }
            }
        }
        Some(TIPO_CLASSE) if !rtti::derives_from_iscriptable(tipo) => {
            for (t, off, _) in rtti::propriedades_com_holder(tipo) {
                clonar_valor(c, t, valor.add(off as usize));
            }
        }
        _ => {}
    }
}

/// `BwmsCloneObject(obj) -> ref<IScriptable>`: nulo com objeto nulo, classe abstrata ou sem instância.
pub(crate) unsafe extern "C" fn tramp_clone_object(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let obj = a.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    let mut c = Clonagem { copias: std::collections::HashMap::new(), fracos: Vec::new(), rasos: 0, profundidade: 0 };
    let raiz = if obj.is_null() { None } else { clonar_objeto(&mut c, obj) };
    for (slot, alvo) in std::mem::take(&mut c.fracos) {
        if let Some(h) = c.copias.get(&alvo) {
            crate::jogo::objetos::gravar_handle_fraco(slot, *h);
        }
    }
    rtti::make_handle(out, raiz.map_or(std::ptr::null_mut(), |h| h[0] as *mut c_void));
    if let Ok(mut m) = CLONE_MAIOR.lock() {
        if c.copias.len() > m.0 {
            *m = (c.copias.len(), c.rasos);
        }
    }
    for h in c.copias.values_mut() {
        crate::jogo::objetos::soltar_handle(h);
    }
}

/// `BwmsScriptDefinitionCount(kind) -> Int32`: 0 arquivos, 1 globais, 2 enums, 3 bitfields, 4 classes, 5 tipos, 6 strings; -1 fora disso ou antes da carga.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_script_definition_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let tipo = a.first().map_or(-1, |(v, _)| *v as u32 as i32);
    write_i32_ret(out, crate::inicio::boot::bundle_contagem(tipo));
}

/// `BwmsScriptSourceLine(owner, name) -> Int32`: linha da função no arquivo de origem; -1 sem a função ou sem arquivo (nativa).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_script_source_line(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    write_i32_ret(out, crate::inicio::boot::bundle_fonte(g(0), g(1)).map_or(-1, |(_, linha)| linha as i32));
}

/// `BwmsScriptSourceFileMatches(owner, name, suffix) -> Bool`: o caminho do arquivo de origem da função termina com `suffix`.
pub(crate) unsafe extern "C" fn tramp_script_source_file_matches(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let g = |k: usize| a.get(k).map_or(0, |(v, _)| *v);
    let sufixo = strings.get(2).cloned().flatten().unwrap_or_default();
    let ok = !sufixo.is_empty() && crate::inicio::boot::bundle_fonte(g(0), g(1)).is_some_and(|(arquivo, _)| arquivo.ends_with(sufixo.as_str()));
    write_bool_ret(out, ok);
}

/// CRUID do 1º componente do player — o dado que faltava pra provar o caminho POSITIVO do
/// `ComponentTarget.ID`.
///
/// O smoke do `ComponentTarget` só exercitava `Name` com um nome fabricado: prova que o filtro REJEITA, não
/// que ele ACEITA. Filtro que rejeita tudo passaria nesse teste. Com um `id` REAL dá pra registrar
/// um handler que TEM que disparar, e aí as duas direções ficam cobertas.
///
/// `0` quando não há componente legível — distinguível de um CRUID válido, que nunca é zero.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_first_component_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let ent = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let comp = first_component(ent);
    let id = component_id_name(comp).map(|(id, _)| id).unwrap_or(0);
    crate::log_dev!("[cw132] first_component={comp:p} CRUID={id:#x}");
    write_uint_ret(out, id, 8);
}

// ===== API de reflexão de classe (2026-07-24) — atalho pragmático de reclassificação, achado pela
// investigação paralela do mesmo nome. Forjar `ReflectionType`/`ReflectionClass`/`ReflectionFunc`/
// `ReflectionProp`/`ReflectionEnum` como `native class` seria XL genuíno (essas
// classes não têm NENHUM parent vanilla pra herdar — diferente da localização). Em vez
// disso: TODO leaf-primitivo que `Reflection.reds` precisa já está construído e provado in-game há
// sessões (só exposto hoje via comando de console/uso interno, não pro redscript de mods) —
// `class_by_name`/`enum_by_name`/`find_property_in_class`/`resolve_in_class`/`call_func`/
// `props_array` (usados por `getf`/`setf`/`callf`/`rttidump`/`propdump`). Este bloco expõe esses
// primitivos como uma dúzia de GLOBAIS NOVAS que recebem/devolvem ponteiros OPACOS (`Uint64`) — a
// MESMA receita 100% provada de `BwmsGetPlayerField`/`BwmsCallMethod`
// (`register_global_argful`/`register_global_composed` + `compose_params_from_types`), só
// generalizada de "sempre o player" pra "qualquer classe/objeto resolvido por nome/ponteiro". O
// `.reds` (`ponte-reflection.reds`) embrulha cada `Uint64` numa CLASSE COMUM (SEM `native`, SEM
// `register_type` nenhum) — mesma categoria de segurança de `BWMSCheatHandler`: zero RTTI-forge,
// zero risco de `class_validate_probe_hook`.
//
// Divergências documentadas (pragmáticas, mesmo estilo de `ResourceEvent.GetPath` — hash em vez de
// `ResRef` real):
// - `GetProperties()` (array) vira `GetPropertyCount()`+`GetPropertyByIndex(i)` — evita marshalar
//   um `array<Uint64>` de retorno (RE nova, fora de escopo desta rodada); mesmo dado, forma
//   indexada em vez de array.
// - `ReflectionFunc.Call` só cobre chamada SEM ARGS (retorno truncado pra `Bool`). Valor de campo
//   em `Variant` existe (`BwmsReflPropGetValue`/`SetValue`, `BwmsValueToString`/`IsEqual`); chamada
//   com args ou retorno tipado arbitrário, não.
// - `ReflectionClass.GetFunction` não distingue estático/instância (`resolve_in_class` já varre os
//   dois arrays — funcs@+0x48/staticFuncs@+0x58 — e devolve o 1º match por nome); aceitável pro
//   escopo deste round.
// - `GetType`, `IsNative`/`IsAbstract`, `GetConstants` de bitfield e a família enumerate-all
//   (`GetClasses`/`GetTypes`/...) ficam de fora (a própria investigação já sinalizou como precisando
//   de RE nova pequena — não "pure wiring"). `GetInnerType` é o `BwmsGetInnerTypeName`.
pub(crate) unsafe extern "C" fn tramp_refl_get_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let cls = rtti::Registry::obtain().map(|reg| reg.class_by_name(&name)).unwrap_or(std::ptr::null_mut());
    crate::log_dev!("[refl] BwmsReflGetClass('{name}') = {cls:p}");
    write_uint_ret(out, cls as u64, 8);
}

/// `Reflection.ReflectionClass.MakeHandle()` (`PENDENCIAS-UNIFICADAS.md` item #51, resto
/// pendente) — comportamento esperado: `if (!IsInstanceOf<ISerializable>(m_class))
/// return {}; return ToHandle(m_class->CreateInstance(true));`. `CreateInstance(true)` já é
/// EXATAMENTE o que `rtti::new_object` reproduz (GetSize+GetAlignment+Construct via vtable, efeito
/// idêntico, caminho diferente — já provado em dezenas de forjas deste projeto). `IsInstanceOf
/// <ISerializable>` vira checagem de `!isAbstract` (via `class_flags`, fechado nesta sessão) — uma
/// classe abstrata nunca é instanciável, mesmo raciocínio usado pelo `CreateInstance` real
/// internamente. Devolve o ponteiro OPACO (`Uint64`), mesma convenção do resto do
/// API de reflexão de classe (nunca um `ref<T>` de verdade — evita RE nova de refcount-block pra
/// caso genérico).
pub(crate) unsafe extern "C" fn tramp_refl_make_handle(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    if cls.is_null() {
        crate::log_dev!("[refl] BwmsReflMakeHandle: classe null — abortado");
        write_uint_ret(out, 0, 8);
        return;
    }
    let abstract_bit = rtti::class_flags(cls).map(rtti::classe_abstrata).unwrap_or(true); // default seguro: trata como abstrata se ilegível
    if abstract_bit {
        crate::log_dev!("[refl] BwmsReflMakeHandle: classe é abstrata (isAbstract) — não instanciável, devolvendo null");
        write_uint_ret(out, 0, 8);
        return;
    }
    let obj = rtti::new_object_from_class(cls);
    crate::log_dev!("[refl] BwmsReflMakeHandle: instância construída {obj:p}");
    write_uint_ret(out, obj as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_get_enum(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let en = rtti::Registry::obtain().map(|reg| reg.enum_by_name(&name)).unwrap_or(std::ptr::null_mut());
    crate::log_dev!("[refl] BwmsReflGetEnum('{name}') = {en:p}");
    write_uint_ret(out, en as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_get_global_function(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let f = rtti::Registry::obtain().map(|reg| get_function(&reg, &name)).unwrap_or(std::ptr::null_mut());
    crate::log_dev!("[refl] BwmsReflGetGlobalFunction('{name}') = {f:p}");
    write_uint_ret(out, f as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    // CClass+0x18 = name (CName) — mesmo campo que TODA forja deste arquivo escreve
    // (`wr_u64(mem, 0x18, cname(new_name))`), lido aqui na direção inversa.
    let h = if rtti::sane(cls) { rd_u64(cls, 0x18) } else { 0 };
    write_uint_ret(out, h, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_parent(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let parent = if rtti::sane(cls) { rd_u64(cls, 0x10) } else { 0 };
    write_uint_ret(out, parent, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_property(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let prop = if rtti::sane(cls) { rtti::find_property_in_class(cls, &name) } else { std::ptr::null_mut() };
    write_uint_ret(out, prop as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_property_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = if rtti::sane(cls) { rtti::class_own_properties(cls).len() as u32 } else { 0 };
    write_uint_ret(out, n as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_property_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut prop: *mut c_void = std::ptr::null_mut();
    if rtti::sane(cls) && idx >= 0 {
        let props = rtti::class_own_properties(cls);
        if let Some(p) = props.get(idx as usize) {
            prop = *p;
        }
    }
    write_uint_ret(out, prop as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_class_get_function(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let cls = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let f = if rtti::sane(cls) {
        rtti::resolve_in_class(cls, &name).map(|rf| rf.func).unwrap_or(std::ptr::null_mut())
    } else {
        std::ptr::null_mut()
    };
    write_uint_ret(out, f as u64, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_get_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let prop = args.first().map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    // CProperty+0x08 = name (CName) — layout já documentado/usado em `find_prop_exact`/`props_array`.
    let h = if rtti::sane(prop as *mut c_void) { rd_u64(prop, 0x08) } else { 0 };
    write_uint_ret(out, h, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value_float(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let v = if rtti::sane(obj) && !prop.is_null() { rtti::prop_get_f32(prop, obj) } else { f32::NAN };
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u32, v.to_bits());
    }
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value_float(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let raw = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let value = f32::from_bits(raw as u32);
    let ok = rtti::sane(obj) && !prop.is_null() && rtti::prop_set_f32(prop, obj, value);
    write_bool_ret(out, ok);
}

// A ROTA DE COMPOSIÇÃO. Medido em 2026-09-07: `inkTextWidget` tem **28**
// properties de RTTI (`text`, `fontFamily`, `fontStyle`, `fontSize`, `letterCase`, `tracking`,
// `justification`, alinhamentos, `textOverflowPolicy`, …), `inkImageWidget`/`inkShapeWidget` 19 e
// `inkMaskWidget` 6 — o que REFUTA a nota do item ("a suposição 'campos NATIVOS JÁ EXISTENTES'
// estava ERRADA"). Os campos existem; o que faltava era acessor pros TIPOS deles.
//
// A API de reflexão só tinha `Bool` e `Float`. `Int32` cobre todos os ENUMS (letterCase,
// justification, os 2 alinhamentos, overflow policy, contentH/VAlign) e `CName` cobre
// `fontStyle`/`textIdKey`/`fontFamily`. Com estes dois, a superfície do item é montável em
// redscript sobre `ref<inkWidget>` — que o corpus DECLARA, então não precisa forjar nada.
pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value_int32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    // -1 como "não deu" é seguro: nenhum enum do motor usa negativo, e devolver 0 confundiria
    // "falhou" com "o valor é o 1º membro do enum".
    let v: i32 = if rtti::sane(obj) && !prop.is_null() {
        rtti::prop_get_u32(prop, obj) as i32
    } else {
        -1
    };
    write_i32_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value_int32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let value = args.get(2).map(|(v, _)| *v as u32).unwrap_or(0);
    let ok = rtti::sane(obj) && !prop.is_null() && rtti::prop_set_u32(prop, obj, value);
    write_bool_ret(out, ok);
}

/// `BwmsReflPropIsSet(obj, prop) -> Bool` — a property de PONTEIRO está preenchida?
///
/// A reflexão tinha acessor pra Bool/Float/Int32/CName, todos valores escalares. Faltava o caso
/// que mais aparece nos itens de "compor em vez de forjar": a property é um `Handle<T>`/`wref<T>`
/// e a pergunta é só *tem alguém aí?* — `inkComponent.IsAttached` do `#90` é exatamente isso.
///
/// Devolve só um `Bool` de propósito. Devolver o ponteiro daria ao redscript um handle de tipo não
/// verificado, e a lição do `depot_resources_by_class` é dura: `is_readable` protege LEITURA, não
/// CHAMADA — um handle cru de tipo errado vira chamada virtual num objeto que não é aquilo, e o
/// jogo morre. Ler os 8 bytes e responder sim/não não tem esse risco.
pub(crate) unsafe extern "C" fn tramp_refl_prop_is_set(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let mut set = false;
    if rtti::sane(obj) && !prop.is_null() {
        let vp = rtti::prop_value_ptr(prop, obj);
        // NUNCA comparar retorno de native inline: o valor vai pra um LOCAL antes do teste.
        if crate::memoria::is_readable(vp as *const c_void, 8) {
            let v: u64 = core::ptr::read_unaligned(vp as *const u64);
            set = v != 0;
        }
    }
    write_bool_ret(out, set);
}

/// `CName` é um `u64` (o hash) — ler/escrever é o mesmo par de 8 bytes.
pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value_cname(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let p = if rtti::sane(obj) { rtti::prop_value_ptr(prop, obj) } else { std::ptr::null_mut() };
    let v: u64 = if p.is_null() { 0 } else { core::ptr::read_unaligned(p as *const u64) };
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value_cname(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let value = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let p = if rtti::sane(obj) { rtti::prop_value_ptr(prop, obj) } else { std::ptr::null_mut() };
    if !p.is_null() {
        core::ptr::write_unaligned(p as *mut u64, value);
    }
    write_bool_ret(out, !p.is_null());
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value_bool(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let v = rtti::sane(obj) && !prop.is_null() && rtti::prop_get_bool(prop, obj);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value_bool(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let value = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let ok = rtti::sane(obj) && !prop.is_null() && rtti::prop_set_bool(prop, obj, value);
    write_bool_ret(out, ok);
}

/// Lê os parâmetros `(obj, prop, valor)` e copia o valor pro campo pelo `Assign` do tipo do campo, antes do destrutor do parâmetro. `valor`
/// vem cru (`Variant`) ou tipado (`String`): `desembrulha` diz onde está o valor e de que tipo.
pub(crate) unsafe fn gravar_campo_do_parametro(frame: *mut c_void, desembrulha: impl Fn(*mut c_void, *mut c_void) -> Option<(*mut c_void, *mut c_void)>) -> bool {
    let (mut obj, mut prop, mut ok) = (std::ptr::null_mut(), std::ptr::null(), false);
    rtti::read_params_consuming_visit(current_native_func(), frame, &mut |i, tipo, inst| match i {
        0 => obj = rd_u64(inst, 0) as *mut c_void,
        1 => prop = rd_u64(inst, 0) as *const c_void,
        2 => {
            ok = match (rtti::campo_e_tipo(obj, prop), desembrulha(tipo, inst)) {
                (Some((campo, tipo_campo)), Some((tipo_valor, valor))) => {
                    rtti::tipo_cabe_em(tipo_valor, tipo_campo) && rtti::type_assign(tipo_campo, campo, valor)
                }
                _ => false,
            }
        }
        _ => {}
    });
    ok
}

/// `BwmsReflPropGetValue(obj, prop) -> Variant`: cópia do campo num `Variant` do tipo dele, qualquer tipo (o script desembrulha com
/// `FromVariant<T>`); vazio com objeto ou propriedade inválidos.
pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    if out.is_null() {
        return;
    }
    match rtti::campo_e_tipo(obj, prop) {
        Some((campo, tipo)) => {
            rtti::variante_montar(out as *mut u8, tipo, campo);
        }
        None => core::ptr::write_bytes(out as *mut u8, 0, 24),
    }
}

/// `BwmsReflPropSetValue(obj, prop, valor: Variant) -> Bool`: o valor da variante vai pro campo pelo `Assign` do tipo do campo; recusa tipo
/// que não cabe (handle de classe filha cabe em handle da mãe).
pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ok = gravar_campo_do_parametro(frame, |_, inst| rtti::variante_partes(inst as *const u8));
    write_bool_ret(out, ok);
}

/// `BwmsReflPropGetValueString(obj, prop) -> String`.
pub(crate) unsafe extern "C" fn tramp_refl_prop_get_value_string(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    if out.is_null() {
        return;
    }
    match rtti::campo_e_tipo(obj, prop).filter(|(_, tipo)| rtti::type_name_getname(*tipo) == cname("String")) {
        Some((campo, _)) => crate::jogo::cstring::copia(out as *mut u8, campo as *const u8),
        None => write_string_ret(out, ""),
    }
}

/// `BwmsReflPropSetValueString(obj, prop, valor: String) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_refl_prop_set_value_string(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let ok = gravar_campo_do_parametro(frame, |tipo, inst| Some((tipo, inst)));
    write_bool_ret(out, ok);
}

/// `BwmsReflPropToString(obj, prop) -> String`: o texto que o tipo do campo dá ao valor (`ToString` do IType).
pub(crate) unsafe extern "C" fn tramp_refl_prop_to_string(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(1).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let texto = rtti::campo_e_tipo(obj, prop).and_then(|(campo, tipo)| rtti::type_to_string(tipo, campo)).unwrap_or_default();
    write_string_ret(out, &texto);
}

/// `BwmsReflPropIsEqual(a, b, prop) -> Bool`: o mesmo campo em dois objetos, comparado pelo `IsEqual` do tipo. `false` se `b` não tem a
/// propriedade (a classe dele tem que achar a mesma `CProperty` pelo nome).
pub(crate) unsafe extern "C" fn tramp_refl_prop_is_equal(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let a = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let b = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(2).map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let mesma = rtti::sane(b) && rtti::sane(prop as *mut c_void) && {
        let nome = crate::cname::resolve_cname(rd_u64(prop, 0x08));
        rtti::find_property_in_class(rtti::class_of(b), &nome) as *const c_void == prop
    };
    let igual = mesma
        && match (rtti::campo_e_tipo(a, prop), rtti::campo_e_tipo(b, prop)) {
            (Some((ca, tipo)), Some((cb, _))) => rtti::type_is_equal(tipo, ca, cb) == Some(true),
            _ => false,
        };
    write_bool_ret(out, igual);
}

/// `BwmsValueToString(valor: Variant) -> String` e `BwmsValueIsEqual(a: Variant, b: Variant) -> Bool`: as mesmas operações do tipo sobre
/// qualquer valor que o script embrulhe com `ToVariant`, lidas antes do destrutor do parâmetro.
pub(crate) unsafe extern "C" fn tramp_value_to_string(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut texto = String::new();
    rtti::read_params_consuming_visit(current_native_func(), frame, &mut |_, _, inst| {
        if let Some((tipo, valor)) = rtti::variante_partes(inst as *const u8) {
            texto = rtti::type_to_string(tipo, valor).unwrap_or_default();
        }
    });
    write_string_ret(out, &texto);
}

pub(crate) unsafe extern "C" fn tramp_value_is_equal(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // O 1º valor fica vivo numa cópia tipada do pool até o 2º chegar; tipos diferentes nunca são iguais.
    let mut primeiro: Option<(*mut c_void, *mut c_void, usize)> = None;
    let mut igual = false;
    rtti::read_params_consuming_visit(current_native_func(), frame, &mut |i, _, inst| {
        let Some((tipo, valor)) = rtti::variante_partes(inst as *const u8) else { return };
        if i == 0 {
            let (copia, tam) = rtti::type_inst_alloc(tipo);
            if !copia.is_null() {
                primeiro = Some((tipo, copia, tam));
                rtti::type_assign(tipo, copia, valor);
            }
        } else if let Some((tipo_a, copia, _)) = primeiro {
            igual = tipo_a == tipo && rtti::type_is_equal(tipo, copia, valor) == Some(true);
        }
    });
    if let Some((tipo, copia, tam)) = primeiro {
        rtti::type_inst_free(tipo, copia, tam);
    }
    write_bool_ret(out, igual);
}

pub(crate) unsafe extern "C" fn tramp_refl_func_get_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let func = args.first().map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    // CBaseFunction+0x10 = shortName (CName) — mesmo campo que `resolve_in_class` casa por nome.
    let h = if rtti::sane(func as *mut c_void) { rd_u64(func, 0x10) } else { 0 };
    write_uint_ret(out, h, 8);
}

/// `ReflectionFunc.Call(func, obj)` — chamada SEM ARGS (mesma receita de `BwmsCallMethod`, só que
/// aqui o método já veio de um `GetFunction`/`GetClass(...).GetFunction` anterior em vez de
/// resolvido por nome na hora). Retorno truncado pra `Bool` (primeiros 4 bytes != 0); chamada com
/// args ou retorno tipado arbitrário não é coberta.
pub(crate) unsafe extern "C" fn tramp_refl_func_call(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let func = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let obj = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if rtti::sane(func) && rtti::sane(obj) {
        let rf = rtti::ResolvedFn { func, ret_type: rtti::ret_type_of(func), is_static: false };
        if let Some(r) = rtti::call_func(&rf, obj, &[]) {
            rtti::soltar_retorno(&rf, &r);
            ok = true;
        }
    }
    crate::log_dev!("[refl] BwmsReflFuncCall(func={func:p}, obj={obj:p}) ok={ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_refl_enum_get_value(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let en = args.first().map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    let hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    // O caller já tem o CEnum* de um `GetEnum` anterior, não o nome do tipo.
    let val = if rtti::sane(en as *mut c_void) { rtti::enum_valor(en as *const u8, cname(&name)).unwrap_or(0) } else { 0 };
    write_uint_ret(out, val, 8);
}

/// `ReflectionType.GetInnerType()` (2026-08-10) — `BwmsGetInnerTypeName
/// (typeName: CName) -> CName`: resolve o IType por nome (mesmo `get_type` já usado por
/// `compose_params_from_types`), lê `innerType@+0x10` (campo de dado, sem risco de vtable-shift —
/// ver nota longa em `rtti::get_inner_type`), resolve o nome do tipo interno.
pub(crate) unsafe extern "C" fn tramp_refl_get_inner_type_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let mut inner_hash: u64 = 0;
    if let Some(reg) = rtti::Registry::obtain() {
        let ty = get_type(&reg, &name);
        if rtti::sane(ty) {
            if let Some(inner) = rtti::get_inner_type(ty) {
                inner_hash = rtti::type_name_getname(inner);
            }
        }
    }
    let inner_name = crate::cname::resolve_cname(inner_hash);
    crate::log_dev!("[refl] BwmsGetInnerTypeName('{name}') -> '{inner_name}'");
    write_uint_ret(out, inner_hash, 8);
}

/// As natives de reflexão numa tabela: nome, tipos dos parâmetros e handler. Cada registro escrito
/// à mão gerava a sua própria sequência de código; aqui são 24 entradas de dados e um laço só — e o
/// diagnóstico passou a dizer o que FALHOU, em vez de 19 `ok=true` num log de 6 linhas.
pub(crate) static NATIVES_DA_REFLEXAO: &[(&str, &[&str], NativeHandler)] = &[
    ("BwmsReflGetClass", &["CName"], tramp_refl_get_class),
    ("BwmsReflGetEnum", &["CName"], tramp_refl_get_enum),
    ("BwmsReflGetGlobalFunction", &["CName"], tramp_refl_get_global_function),
    ("BwmsReflClassGetName", &["Uint64"], tramp_refl_class_get_name),
    ("BwmsReflClassGetParent", &["Uint64"], tramp_refl_class_get_parent),
    ("BwmsReflClassGetProperty", &["Uint64", "CName"], tramp_refl_class_get_property),
    ("BwmsReflClassGetPropertyCount", &["Uint64"], tramp_refl_class_get_property_count),
    ("BwmsReflClassGetPropertyByIndex", &["Uint64", "Int32"], tramp_refl_class_get_property_by_index),
    ("BwmsReflClassGetFunction", &["Uint64", "CName"], tramp_refl_class_get_function),
    ("BwmsReflPropGetName", &["Uint64"], tramp_refl_prop_get_name),
    ("BwmsReflPropGetValueFloat", &["handle:IScriptable", "Uint64"], tramp_refl_prop_get_value_float),
    ("BwmsReflPropSetValueFloat", &["handle:IScriptable", "Uint64", "Float"], tramp_refl_prop_set_value_float),
    ("BwmsReflPropGetValueBool", &["handle:IScriptable", "Uint64"], tramp_refl_prop_get_value_bool),
    ("BwmsReflPropSetValueBool", &["handle:IScriptable", "Uint64", "Bool"], tramp_refl_prop_set_value_bool),
    ("BwmsReflFuncGetName", &["Uint64"], tramp_refl_func_get_name),
    ("BwmsReflFuncCall", &["Uint64", "handle:IScriptable"], tramp_refl_func_call),
    ("BwmsReflEnumGetValue", &["Uint64", "CName"], tramp_refl_enum_get_value),
    ("BwmsReflMakeHandle", &["Uint64"], tramp_refl_make_handle),
    ("BwmsGetInnerTypeName", &["CName"], tramp_refl_get_inner_type_name),
    // 2026-09-07 — Int32/enum e CName, os tipos que faltavam pra rota de
    // composição sobre `ref<inkWidget>`.
    ("BwmsReflPropGetValueInt32", &["handle:IScriptable", "Uint64"], tramp_refl_prop_get_value_int32),
    ("BwmsReflPropSetValueInt32", &["handle:IScriptable", "Uint64", "Int32"], tramp_refl_prop_set_value_int32),
    ("BwmsReflPropGetValueCName", &["handle:IScriptable", "Uint64"], tramp_refl_prop_get_value_cname),
    ("BwmsReflPropSetValueCName", &["handle:IScriptable", "Uint64", "CName"], tramp_refl_prop_set_value_cname),
    ("BwmsReflPropIsSet", &["handle:IScriptable", "Uint64"], tramp_refl_prop_is_set),
];

/// Registra as globais da API de reflexão de classe (ver nota longa acima): a tabela `NATIVES_DA_REFLEXAO` por
/// `register_argful_by_types` e as de valor de qualquer tipo (`Variant`) por `register_global_composed`.
pub(crate) unsafe fn register_reflection_natives(reg: &Registry, proto: *mut c_void) {
    let falhas: Vec<&str> = NATIVES_DA_REFLEXAO
        .iter()
        .filter(|(nome, tipos, h)| !register_argful_by_types(reg, proto, tipos, nome, nome, *h))
        .map(|(nome, _, _)| *nome)
        .collect();
    // Vai por veredito, não por `log()`: native declarada no `.reds` cujo registro falhou liga o
    // bind a nada, e o `log()` amostra 1 em 8 nesta altura do boot (a linha simplesmente não sai).
    crate::veredito_dev!(
        "[reg] reflexão: {} de {} natives registradas; falhou: {falhas:?}",
        NATIVES_DA_REFLEXAO.len() - falhas.len(),
        NATIVES_DA_REFLEXAO.len()
    );
    // RTTI #168/#169/#232/#236: valor de qualquer tipo pelo IType (`Assign`, `IsEqual`, `ToString`).
    // Uma `CProperty` nova por parâmetro; o `Variant` usa a do `SetVariant` (a das natives de variante) e,
    // sem ela, a montada pelo nome do tipo.
    let variante = cprop_variant(reg);
    let cprop = |nome: &str| match nome {
        "Variant" if variante != 0 => variante,
        _ => build_min_cprop(get_type(reg, nome)) as u64,
    };
    let (obj, prop) = ("handle:IScriptable", "Uint64");
    let valor: [(&[&str], &str, NativeHandler); 8] = [
        (&[obj, prop], "BwmsReflPropGetValue", tramp_refl_prop_get_value),
        (&[obj, prop, "Variant"], "BwmsReflPropSetValue", tramp_refl_prop_set_value),
        (&[obj, prop], "BwmsReflPropGetValueString", tramp_refl_prop_get_value_string),
        (&[obj, prop, "String"], "BwmsReflPropSetValueString", tramp_refl_prop_set_value_string),
        (&[obj, prop], "BwmsReflPropToString", tramp_refl_prop_to_string),
        (&[obj, obj, prop], "BwmsReflPropIsEqual", tramp_refl_prop_is_equal),
        (&["Variant"], "BwmsValueToString", tramp_value_to_string),
        (&["Variant", "Variant"], "BwmsValueIsEqual", tramp_value_is_equal),
    ];
    let registradas: Vec<String> = valor
        .iter()
        .filter(|(tipos, nome, h)| {
            let cprops: Vec<u64> = tipos.iter().map(|t| cprop(t)).collect();
            register_global_composed(reg, proto, compose_params_de_cprops(&cprops), nome, nome, *h)
        })
        .map(|(_, nome, _)| nome.to_string())
        .collect();
    crate::veredito_dev!("[reg] valor de qualquer tipo pelo IType: {} de {} ({registradas:?}); CProperty do Variant {variante:#x}", registradas.len(), valor.len());
}

/// 2026-08-11: 1º componente REAL do array `entity+0xA0` (mesmo layout já
/// batalha-testado de `BwmsEntityFindComponentByType`/`GetComponents`) — usado pra testar
/// `EntityComponentEvent.GetComponent()->wref<IComponent>` com um ponteiro genuíno, não fixture.
#[cfg(feature = "devtools")]
pub unsafe fn first_component(entity: *mut c_void) -> *mut c_void {
    if !rtti::sane(entity) {
        return std::ptr::null_mut();
    }
    let arr = (entity as *mut u8).add(0xA0) as *const c_void;
    if !crate::memoria::is_readable(arr, 0x10) {
        return std::ptr::null_mut();
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size >= 10_000 {
        return std::ptr::null_mut();
    }
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if rtti::sane(inst) {
            return inst;
        }
    }
    std::ptr::null_mut()
}


/// Expõe Reflection CALLF pro redscript: `BwmsCallPlayerMethod(method: CName) -> Bool` — chama um
/// método NO-ARG do player por nome (resolve_in_class + call_func). Completa get/set/CALL pro redscript.
pub(crate) unsafe extern "C" fn tramp_callplayer(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(hash);
    let player = crate::current_player();
    let mut ok = false;
    let mut ret = 0i32;
    if !player.is_null() {
        if let Some(rf) = rtti::resolve_in_class(rtti::class_of(player), &name) {
            if let Some(r) = rtti::call_func(&rf, player, &[]) {
                ret = i32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                rtti::soltar_retorno(&rf, &r);
                ok = true;
            }
        }
    }
    crate::log_dev!(
        "[callplayer] BwmsCallPlayerMethod('{name}') ok={ok} ret={ret} — Reflection callf VIA REDSCRIPT"
    );
    write_bool_ret(out, ok);
}


/// Forja uma classe NOVA **instanciável** (vtable+size+align reais de um `vtable_donor`, mesmo
/// mecanismo de `register_type_alias`) COM **parent explícito** (mesmo fix de `register_type_min`
/// que fechou a Facade — `CClass+0x10` — mas parametrizado, porque aqui a base NÃO é IScriptable
/// implícito: é o que o `.reds` declarar via `extends`, ex. `CallbackSystem extends IGameSystem`).
/// Sem essa escrita, o validador de classe nativa (`IsKindOf` walk) rejeita mesmo com os métodos
/// certos registrados — achado já documentado em `register_type_min`. Devolve a forja ou null.
pub unsafe fn register_type_instantiable_with_parent(
    reg: &Registry,
    new_name: &str,
    vtable_donor: &str,
    parent: *mut c_void,
) -> *mut c_void {
    const CCLASS_SIZE: usize = 0x2D0;
    if rtti::sane(reg.class_by_name(new_name)) {
        crate::log_dev!("[reg] register_type_instantiable: '{new_name}' JÁ existe — abortado");
        return std::ptr::null_mut();
    }
    let src = reg.class_by_name(vtable_donor);
    if !rtti::sane(src) || !crate::memoria::is_readable(src as *const c_void, 0x78) {
        crate::log_dev!("[reg] register_type_instantiable: donor '{vtable_donor}' inválido");
        return std::ptr::null_mut();
    }
    if !rtti::sane(parent) {
        crate::log(
            "[reg] register_type_instantiable: parent (ptr) não é sane — abortado (sem parent o validador rejeita, ver fix da Facade)"
        );
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(src as *const c_void, 0x00);
    let src_size = ((src as *const u8).add(0x68) as *const u32).read_unaligned();
    let src_align = ((src as *const u8).add(0x74) as *const u32).read_unaligned();
    if vtable == 0 || src_size == 0 || src_size > 0x10000 {
        crate::log_dev!("[reg] register_type_instantiable: donor inválido (vtable/size)");
        return std::ptr::null_mut();
    }
    let align = if src_align == 0 || src_align > 64 { 8 } else { src_align };
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        crate::log_dev!("[reg] register_type_instantiable: pool_alloc null");
        return std::ptr::null_mut();
    }
    // Filha nunca menor que a mãe: campo da mãe depois dos bytes do doador (a cauda de `campo_na_cauda`) tem que caber no objeto.
    let tamanho = if crate::memoria::is_readable((parent as *const u8).add(0x68) as *const c_void, 4) {
        src_size.max(rd_u32(parent, 0x68))
    } else {
        src_size
    };
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE);
    wr_u64(mem, 0x00, vtable); // vtable do donor (Construct/getters reais)
    wr_u64(mem, 0x18, cname(new_name)); // name
    wr_u32(mem, 0x68, tamanho); // o Construct do donor escreve dentro do size dele
    wr_u32(mem, 0x6C, tamanho); // holderSize = size
    wr_u32(mem, 0x70, 0x02); // flags: isNative, SEM isAbstract → instanciável
    wr_u32(mem, 0x74, align); // alignment REAL
    wr_u64(mem, 0x10, parent as u64); // parent (CClass+0x10) — o fix da Facade, generalizado
    // FIX (2026-07-18, sessão `dynarraygrowth-probe`): conserta os hashmaps embutidos (vários
    // slots, ver `fix_embedded_allocator_vtables`) ANTES do RegisterType — sem isto, registrar um
    // MÉTODO cujo param/retorno é `ref<classe-forjada>` crasha (SIGSEGV) na 1ª inserção do engine
    // num desses hashmaps. Root-cause do crash de `GetService`/`CallbackSystemHandler`.
    fix_embedded_allocator_vtables(mem, src, CCLASS_SIZE);
    let slot = reg.vtbl_slot(0x80);
    if !rtti::sane(slot) {
        crate::log_dev!("[reg] register_type_instantiable: RegisterType@0x80 ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    let back = reg.class_by_name(new_name);
    let ok = rtti::sane(back);
    crate::log_dev!(
        "[reg] register_type_instantiable '{new_name}' (donor='{vtable_donor}' parent={parent:p} size={src_size:#x}) -> {} ({mem:p})",
        if ok { "OK ✓" } else { "FALHOU" }
    );
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
}

/// `PoolRTTIProperty::AllocateAligned(tamanho, alinhamento)` — o alocador das `CProperty` do motor.
pub(crate) const PROPRIEDADE_ALOCA_VM: u64 = 0x1_010a_2e90;

pub(crate) const PROP_FLAG_SCRIPTED: u64 = 1 << 5;

pub(crate) const PROP_FLAG_NO_HOLDER: u64 = 1 << 21;

/// `isPersistent` (bit `0x1C`) e `isSavable` (bit `0x21`) da `CProperty`: propriedade criada pelo BWMS
/// não vai para o save nem para o recurso serializado.
pub(crate) const PROP_FLAGS_DE_SAVE: u64 = (1 << 0x1C) | (1 << 0x21);

/// Doador da vtable das classes nativas novas: construtor e `GetSize` reais, o mesmo das classes forjadas do `CallbackSystem`.
pub(crate) const CLASSE_NOVA_DOADOR: &str = "gameuiInGameMenuGameController";

/// A `CProperty` de nome `nome` na classe ou numa mãe.
pub(crate) unsafe fn propriedade_da_cadeia(classe: *mut c_void, nome: u64) -> Option<*mut c_void> {
    let mut c = classe;
    for _ in 0..64 {
        if !rtti::sane(c) {
            return None;
        }
        let (arr, n) = (rd_u64(c as *const c_void, 0x28) as *const u64, rd_u32(c as *const c_void, 0x34) as usize);
        if !arr.is_null() && n < 100_000 {
            for i in 0..n {
                let p = arr.add(i).read_unaligned() as *mut c_void;
                if rtti::sane(p) && rd_u64(p as *const c_void, 0x08) == nome {
                    return Some(p);
                }
            }
        }
        c = rd_u64(c as *const c_void, 0x10) as *mut c_void;
    }
    None
}

/// Flags de uma propriedade nativa da classe ou de uma mãe, sem `isScripted`, `inValueHolder` e as de save; `0` se não há nenhuma.
pub(crate) unsafe fn flags_de_irma_nativa(classe: *mut c_void) -> u64 {
    let mut c = classe;
    for _ in 0..64 {
        if !rtti::sane(c) {
            break;
        }
        let (arr, n) = (rd_u64(c as *const c_void, 0x28) as *const u64, rd_u32(c as *const c_void, 0x34) as usize);
        for i in 0..if arr.is_null() || n > 100_000 { 0 } else { n } {
            let p = arr.add(i).read_unaligned() as *mut c_void;
            let f = if rtti::sane(p) { rd_u64(p as *const c_void, 0x28) } else { PROP_FLAG_SCRIPTED };
            if f & PROP_FLAG_SCRIPTED == 0 {
                return f & !(PROP_FLAG_SCRIPTED | PROP_FLAG_NO_HOLDER | PROP_FLAGS_DE_SAVE);
            }
        }
        c = rd_u64(c as *const c_void, 0x10) as *mut c_void;
    }
    0
}

/// Propriedade nativa nova numa classe (o construtor e o `Create` da `CProperty`): 0x30 bytes do pool de propriedades
/// do RTTI com tipo, nome, dona, offset e as flags de uma propriedade nativa irmã, anexada ao `props` (`+0x28`) da classe. O `GetProperty` do
/// motor (`0x1010c4c14`), que o bind dos scripts usa pra cada `native let`, cai na busca linear em `props` quando o `propsByName` (`+0xE8`) não
/// acha, então a propriedade vale pro bind sem mexer no mapa; o bind só confere o nome do tipo contra o do script. Precisa rodar antes do
/// bind, junto do registro das natives. Idempotente: se a classe (ou uma mãe) já tem o nome com o mesmo tipo, devolve a que existe.
pub(crate) unsafe fn propriedade_nativa(reg: &Registry, classe: &str, nome: &str, tipo: &str, offset: u32) -> Result<*mut c_void, String> {
    let cls = reg.script_class(cname(classe));
    if !rtti::sane(cls) {
        return Err(format!("classe '{classe}' não existe"));
    }
    let ty = get_type(reg, tipo);
    if !rtti::sane(ty) {
        return Err(format!("tipo '{tipo}' não existe"));
    }
    if let Some(p) = propriedade_da_cadeia(cls, cname(nome)) {
        return if rd_u64(p as *const c_void, 0x00) == ty as u64 { Ok(p) } else { Err(format!("'{classe}.{nome}' já existe com outro tipo")) };
    }
    let (tamanho, classe_tam) = (rtti::type_size(ty), rd_u32(cls as *const c_void, 0x68));
    if tamanho == 0 || offset.checked_add(tamanho).is_none_or(|fim| fim > classe_tam) {
        return Err(format!("'{tipo}' ({tamanho} B) em {offset:#x} passa do tamanho da classe ({classe_tam:#x})"));
    }
    let aloca: extern "C" fn(u64, u32) -> *mut c_void = std::mem::transmute(crate::rebase(PROPRIEDADE_ALOCA_VM));
    let p = aloca(0x30, 8);
    if p.is_null() {
        return Err("o pool de propriedades não alocou".into());
    }
    std::ptr::write_bytes(p as *mut u8, 0, 0x30);
    wr_u64(p, 0x00, ty as u64);
    wr_u64(p, 0x08, cname(nome));
    wr_u64(p, 0x18, cls as u64);
    wr_u32(p, 0x20, offset);
    wr_u64(p, 0x28, flags_de_irma_nativa(cls));
    dynarray_push_ptr((cls as *mut u8).add(0x28) as *mut c_void, p as u64).ok_or("o props da classe não cresceu")?;
    crate::interface::c175_esquecer_classes_legadas();
    Ok(p)
}

/// Classe nativa nova: instanciável, com a mãe dada, e `extra` bytes depois dos da construção pros campos de quem a criou.
/// Devolve a classe e o offset onde os campos começam. A vtable (construtor e tamanho) vem do doador, como nas classes forjadas do
/// `CallbackSystem`; o tamanho lido pelo `GetSize` é o do campo `+0x68`, então os bytes a mais entram na alocação.
pub(crate) unsafe fn classe_nativa(reg: &Registry, nome: &str, mae: &str, extra: u32) -> Result<(*mut c_void, u32), String> {
    let existente = reg.script_class(cname(nome));
    if rtti::sane(existente) {
        return Err(format!("'{nome}' já existe"));
    }
    let pai = reg.script_class(cname(mae));
    if !rtti::sane(pai) {
        return Err(format!("mãe '{mae}' não existe"));
    }
    let cls = register_type_instantiable_with_parent(reg, nome, CLASSE_NOVA_DOADOR, pai);
    if cls.is_null() {
        return Err(format!("o registro de '{nome}' falhou"));
    }
    let base = (rd_u32(cls as *const c_void, 0x68) + 7) & !7;
    let total = base + ((extra + 7) & !7);
    wr_u32(cls, 0x68, total);
    wr_u32(cls, 0x6C, total);
    Ok((cls, base))
}

/// Campos do motor que o script do BWMS enxerga como `native let` sem getter próprio: (classe, nome, tipo, offset). O `parentWidget` do
/// `inkWidget` (`WeakHandle` em `+0x168`) é o que a API de UI dos mods usa pra subir a árvore.
pub(crate) const CAMPOS_NATIVOS: &[(&str, &str, &str, u32)] = &[
    ("inkWidget", "parentWidget", "whandle:inkWidget", 0x168),
    // Offsets do Mac 2.5 (prova offline de 2026-10-27 em `proofs/`): o mesh que o componente carregou
    // (o `LoadResource` grava o Handle em `+0x200`) e o gênero da fala, no buraco de 4 B de `+0xC` da struct de `0x28`.
    ("entSkinnedMeshComponent", "meshResource", "handle:CMesh", 0x200),
    ("audioDialogLineEventData", "gender", "Int32", 0xC),
];

/// A fase de tipos antes da validação das classes (chamada pelo gancho do validador na primeira classe; o `register_all` chama de novo, sem
/// efeito, pro caso de o gancho não estar posto).
pub(crate) unsafe fn fase_de_tipos() {
    if let Some(reg) = rtti::Registry::obtain() {
        registrar_tipos_de_mod(&reg);
    }
}

/// A fase de tipos no bind: os campos do produto e, com o marcador do teste, as propriedades de teste e uma classe criada pela API dos
/// plugins (a mesma rota de um plugin no carregamento). Sem condição de runtime nos campos de produto: um `native let` declarado sem a
/// propriedade derrubaria o bind.
pub(crate) unsafe fn registrar_tipos_de_mod(reg: &Registry) {
    static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if FEITO.swap(true, Ordering::Relaxed) {
        return;
    }
    for (classe, nome, tipo, offset) in CAMPOS_NATIVOS {
        let r = propriedade_nativa(reg, classe, nome, tipo, *offset);
        crate::log_dev!("[tipos] {classe}.{nome}: {}", r.map_or_else(|e| e, |p| format!("{p:p}")));
    }
    #[cfg(feature = "devtools")]
    if crate::marcador_individual("campo-teste") {
        for (nome, tipo, offset) in [("bwmsTesteOpacidade", "Float", 0x1F0u32), ("bwmsTestePai", "whandle:inkWidget", 0x168)] {
            let r = propriedade_nativa(reg, "inkWidget", nome, tipo, offset);
            crate::veredito_dev!("[campo] teste inkWidget.{nome}: {}", r.map_or_else(|e| e, |p| format!("{p:p}")));
        }
        let base = (crate::mods_do_usuario::api::BWMS_API.class_create)(c"BwmsCampoTesteClasse".as_ptr(), c"IScriptable".as_ptr(), 8);
        let prop = base != 0 && (crate::mods_do_usuario::api::BWMS_API.class_add_property)(c"BwmsCampoTesteClasse".as_ptr(), c"valor".as_ptr(), c"Int32".as_ptr(), base);
        crate::veredito_dev!("[campo] teste classe pela API: base dos campos {base:#x}, valor: Int32 {prop}");
    }
}

/// `IRTTISystem::CreateScriptedEnum(CName aName, int8_t aSize, DynArray<uint64_t>& aMembers)` —
/// vtable slot `+0xE8` do `CRTTISystem`, 2026-08-05 (achado de auditoria — nunca chamado em
/// NENHUM lugar do binário vanilla, ABI confirmada só por RE de disassembly, não por caller
/// real). Layout do elemento CONFIRMADO por leitura direta do corpo real da função (não
/// suposição): `{ name: CName(u64) @+0x00; value: i64 @+0x08 }`, stride 16 bytes — a função só
/// FAZ PushBack de `name`/`value` em `hashList`/`valueList` do `CEnum` recém-construído, nunca
/// escreve no buffer que passamos (só lê). `aMembers` é um `DynArray<T>` no mesmo layout já usado
/// no resto deste arquivo (`entries@0x00, capacity@0x08(u32), size@0x0C(u32)`).
/// `aSize` = tamanho em bytes do enum subjacente (1/2/4/8 — `EInputAction`/enums pequenos
/// usam 1, mesmo padrão de `EInputKey`).
#[repr(C)]
pub(crate) struct EnumMemberEntry {
    pub(crate) name: u64,
    pub(crate) value: i64,
}

#[repr(C)]
pub(crate) struct DynArrayHeaderRaw {
    pub(crate) entries: *mut c_void,
    pub(crate) capacity: u32,
    pub(crate) size: u32,
}

pub(crate) unsafe fn create_scripted_enum(reg: &Registry, name: &str, size_bytes: i8, members: &[(&str, i64)]) -> bool {
    let fp = reg.vtbl_slot(0xE8);
    if fp.is_null() {
        crate::log_dev!("[reg] CreateScriptedEnum: vtbl+0xE8 ilegível/null");
        return false;
    }
    let n = members.len();
    if n == 0 {
        crate::log_dev!("[reg] CreateScriptedEnum: zero membros, abortando");
        return false;
    }
    let buf = rtti::pool_alloc(n * std::mem::size_of::<EnumMemberEntry>(), 8) as *mut EnumMemberEntry;
    if buf.is_null() {
        crate::log_dev!("[reg] CreateScriptedEnum: pool_alloc falhou");
        return false;
    }
    for (i, (mname, mval)) in members.iter().enumerate() {
        // `intern` (não `cname` cru) — registra no espelho local (`cname.rs::POOL`), senão um dump
        // futuro (`name_of`) nunca reverte esses nomes: o pool NATIVO do jogo só conhece strings
        // que o PRÓPRIO engine internou (compiladas/carregadas), não hashes que só computamos aqui
        // sem avisar ninguém (achado 2026-08-09, ver `dump_bitfield_names`).
        core::ptr::write_unaligned(buf.add(i), EnumMemberEntry { name: crate::cname::intern(mname), value: *mval });
    }
    let mut header = DynArrayHeaderRaw { entries: buf as *mut c_void, capacity: n as u32, size: n as u32 };
    let func: unsafe extern "C" fn(*mut c_void, u64, i8, *mut c_void) = std::mem::transmute(fp);
    func(reg.raw(), crate::cname::intern(name), size_bytes, &mut header as *mut DynArrayHeaderRaw as *mut c_void);
    crate::log_dev!("[reg] CreateScriptedEnum('{name}', size={size_bytes}, {n} membros) chamado — zero crash até aqui");
    true
}

/// `IRTTISystem::CreateScriptedBitfield(CName aName, DynArray<uint64_t>& aBits)` (vtbl+0xF0,
/// `PENDENCIAS-UNIFICADAS.md` item #79) — irmã de `CreateScriptedEnum` (0xE8, acima):
/// o array de bits não é de `uint64_t`, é de struct com o nome e o bit; mesmo layout de entry de 16 bytes já provado
/// (`EnumMemberEntry`), só o campo semântico muda de "value" pra "bit" (índice do bit, não valor
/// arbitrário) — reuso do MESMO struct Rust (layout idêntico), sem `aSize` (bitfield não tem
/// tamanho variável como enum).
pub(crate) unsafe fn create_scripted_bitfield(reg: &Registry, name: &str, bits: &[(&str, i64)]) -> bool {
    let fp = reg.vtbl_slot(0xF0);
    if fp.is_null() {
        crate::log_dev!("[reg] CreateScriptedBitfield: vtbl+0xF0 ilegível/null");
        return false;
    }
    let n = bits.len();
    if n == 0 {
        crate::log_dev!("[reg] CreateScriptedBitfield: zero bits, abortando");
        return false;
    }
    let buf = rtti::pool_alloc(n * std::mem::size_of::<EnumMemberEntry>(), 8) as *mut EnumMemberEntry;
    if buf.is_null() {
        crate::log_dev!("[reg] CreateScriptedBitfield: pool_alloc falhou");
        return false;
    }
    for (i, (bname, bidx)) in bits.iter().enumerate() {
        // `intern` — mesmo achado de `create_scripted_enum` (ver comentário lá): registra no
        // espelho local pra `dump_bitfield_names`/`name_of` conseguirem reverter o hash depois.
        core::ptr::write_unaligned(buf.add(i), EnumMemberEntry { name: crate::cname::intern(bname), value: *bidx });
    }
    let mut header = DynArrayHeaderRaw { entries: buf as *mut c_void, capacity: n as u32, size: n as u32 };
    let func: unsafe extern "C" fn(*mut c_void, u64, *mut c_void) = std::mem::transmute(fp);
    func(reg.raw(), crate::cname::intern(name), &mut header as *mut DynArrayHeaderRaw as *mut c_void);
    crate::log_dev!("[reg] CreateScriptedBitfield('{name}', {n} bits) chamado — zero crash até aqui");
    true
}

/// `Reflection.ReflectionEnum::AddConstant(name, value)`: dedup por NOME OU VALOR já existente (`return` sem
/// adicionar), senão `PushBack` em `hashList`(CName)/`valueList`(int64_t) — 2 `DynArray` PARALELAS
/// de `CEnum` (`hashList@+0x28`/`valueList@+0x38`, mesmo layout já usado por `resolve_enum_value`).
/// Reusa `dynarray_push_ptr` (grow seguro já provado pra props array) pros 2 PUSH — cap/size ficam
/// corretos por construção, sem reimplementar grow à mão (evita a ambiguidade real que existia
/// antes de eu confirmar, via essa mesma função já provada, que o campo em `en+0x30` é CAP, não
/// SIZE — `resolve_enum_value` só "acerta por acidente" porque cap==size em todo enum vanilla
/// nunca crescido; `+0x34` é o size real). **Testado só contra enum FORJADO por nós
/// (`CreateScriptedEnum`), nunca vanilla** — mutar o array de um enum vanilla compartilhado é uma
/// categoria de risco maior (outro código do motor pode cachear ponteiros/índices antigos),
/// deliberadamente fora de escopo desta rodada.
pub(crate) unsafe fn enum_add_constant(en: *mut c_void, name: &str, value: i64) -> bool {
    let base = en as *const u8;
    if !crate::memoria::is_readable(base.add(0x28) as *const c_void, 0x18) {
        return false;
    }
    let hp = rd_u64(en, 0x28) as *const u64; // hashList.entries
    let vp = rd_u64(en, 0x38) as *const i64; // valueList.entries
    let size = core::ptr::read_unaligned(base.add(0x34) as *const u32); // hashList.size REAL
    if hp.is_null() || vp.is_null() || size > 100_000 {
        return false;
    }
    let name_hash = crate::cname::intern(name);
    for i in 0..size as usize {
        if hp.add(i).read_unaligned() == name_hash || vp.add(i).read_unaligned() == value {
            return false; // já existe (nome OU valor) — mesma regra de dedup do C++ real, no-op
        }
    }
    let h_ok = dynarray_push_ptr((en as *mut u8).add(0x28) as *mut c_void, name_hash).is_some();
    let v_ok = dynarray_push_ptr((en as *mut u8).add(0x38) as *mut c_void, value as u64).is_some();
    h_ok && v_ok
}

// ===== CallbackSystem (lite): registry event→callbacks + RegisterCallback + fire/dispatch =====
/// É a registry do CallbackSystem em Rust. O ouvinte entra pelo endereço; a vida dele é seguida em `OUVINTES`.
/// (evento, ouvinte, função, handler): o handler que o `RegisterCallback` criou pra esta entrada, 0 no registro sem handler. A mesma função do mesmo
/// ouvinte registrada 2 vezes vira 2 handlers com alvos próprios, como na referência (boot 180: o redespacho do 2º builder usava os alvos do 1º).
pub(crate) static CALLBACKS: Mutex<Vec<(u64, usize, u64, usize)>> = Mutex::new(Vec::new());

/// Todo teste que põe ou tira entrada de `CALLBACKS` ou de `STATIC_CALLBACKS` segura esta trava, leia ele contagem ou não: `eventos_ativos` e
/// `has_listener_for` olham a tabela inteira, e o `cargo test` roda os testes em threads paralelas, então uma entrada posta por um teste muda a
/// contagem que outro está conferindo.
#[cfg(test)]
pub(crate) static TESTES_DE_CALLBACKS_EM_SERIE: Mutex<()> = Mutex::new(());

/// Vida do ouvinte de cada entrada de `CALLBACKS`: (endereço, bloco de contagem com +1 fraco nosso, entradas que usam o ouvinte).
/// O fraco segura o bloco enquanto houver entrada, então ler o forte é seguro; o despacho pula ouvinte com forte 0 (destruído antes
/// de sair da lista), como o `WeakHandle::Lock` da referência. O fraco é solto na saída da última entrada.
static OUVINTES: Mutex<Vec<(usize, u64, u32)>> = Mutex::new(Vec::new());

/// Mais uma entrada pro ouvinte `alvo`. Endereço reaproveitado (o ouvinte velho morreu): troca o fraco pelo do objeto novo.
unsafe fn ouvinte_segura(alvo: usize) {
    let Ok(mut o) = OUVINTES.lock() else { return };
    if let Some(e) = o.iter_mut().find(|e| e.0 == alvo) {
        e.2 += 1;
        if e.1 != 0 && !crate::jogo::objetos::fraco_vivo(e.1) {
            let mut velho = [alvo as u64, e.1];
            crate::jogo::objetos::soltar_fraco(&mut velho);
            e.1 = crate::arquivos_do_jogo::handle_fraco(alvo as *mut c_void)[1];
        }
        return;
    }
    let h = crate::arquivos_do_jogo::handle_fraco(alvo as *mut c_void);
    o.push((alvo, h[1], 1));
}

/// Uma entrada do ouvinte `alvo` saiu de `CALLBACKS`; na última, solta o fraco.
unsafe fn ouvinte_solta(alvo: usize) {
    let Ok(mut o) = OUVINTES.lock() else { return };
    let Some(i) = o.iter().position(|e| e.0 == alvo) else { return };
    o[i].2 = o[i].2.saturating_sub(1);
    if o[i].2 == 0 {
        let (_, bloco, _) = o.swap_remove(i);
        if bloco != 0 {
            let mut h = [alvo as u64, bloco];
            crate::jogo::objetos::soltar_fraco(&mut h);
        }
    }
}

/// O ouvinte ainda existe: entrada em `OUVINTES` com bloco e forte > 0, o `Lock` do `WeakHandle` da referência. Fora da lista conta como morto: um
/// callback anterior do mesmo despacho pode ter desregistrado o ouvinte, e a saída da última entrada solta o nosso fraco antes de a cópia de `CALLBACKS`
/// chegar nele — o objeto pode já ter sido destruído, e o `sane` só confere a faixa do endereço.
unsafe fn ouvinte_vivo(alvo: usize) -> bool {
    OUVINTES.lock().is_ok_and(|o| o.iter().find(|e| e.0 == alvo).is_some_and(|e| crate::jogo::objetos::fraco_vivo(e.1)))
}

// CallbackSystem no RTTI, static callbacks: (event_hash, class_name, fn_name)
pub(crate) static STATIC_CALLBACKS: Mutex<Vec<(u64, String, String)>> = Mutex::new(Vec::new());

// CallbackSystem no RTTI, DispatchEvent: mapa type_cname_hash → event_cname_hash (de RegisterEvent)
pub(crate) static EVENT_TYPE_MAP: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

/// Estado POR-INSTÂNCIA de um `CallbackSystemHandler` real (2026-07-18, handler de callback
/// fechado — ver `register_callbacksystemhandler`). Chave = ponteiro da instância (`usize`, o
/// mesmo padrão de `CALLBACKS`). `targets` = lista de ponteiros de `CallbackSystemTarget`
/// adicionados via `AddTarget` — o "filtro de target" É essa lista (lista vazia = sem filtro,
/// aceita qualquer alvo; não-vazia = só aceita alvos presentes nela — mesmo esquema documentado
/// no `gaps-revised.json`). A entrada de `CALLBACKS` guarda o ponteiro do handler; o despacho, o `Unregister()` e o `RunMode.Once`
/// acham o estado só por ele (`estado_do_handler`). O estado só existe com o handler registrado: as 4 saídas (`Unregister()`, `UnregisterCallback`,
/// `RunMode.Once` e a varredura do `Session/End`) tiram a entrada, e `IsRegistered` é a presença dela. O ponteiro não se repete na tabela, porque o
/// handler nunca é solto (`retain_strong_ref_leaked` no registro).
pub(crate) struct HandlerState {
    pub(crate) targets: Vec<usize>,
    pub(crate) run_mode: i32,
    pub(crate) lifetime: i32,
}

pub(crate) static HANDLER_STATES: Mutex<Vec<(usize, HandlerState)>> = Mutex::new(Vec::new());

/// O estado do handler de uma entrada de `CALLBACKS`, achado só pelo ponteiro. Entrada sem handler (a global `BwmsRegisterCallback`) não tem
/// estado: sem alvos e nunca `RunMode.Once`, mesmo com outro handler registrado para a mesma função do mesmo ouvinte.
fn estado_do_handler(states: &[(usize, HandlerState)], handler: usize) -> Option<&HandlerState> {
    if handler == 0 {
        return None;
    }
    states.iter().rev().find(|(p, _)| *p == handler).map(|(_, s)| s)
}

/// `IsRegistered()`: o handler tem estado em `HANDLER_STATES`.
fn handler_registrado(handler: usize) -> bool {
    HANDLER_STATES.lock().is_ok_and(|states| states.iter().any(|(p, _)| *p == handler))
}

/// Saída de um handler, pelo `Unregister()` dele ou pelo `RunMode.Once` depois do disparo: o estado sai de `HANDLER_STATES` e de `CALLBACKS` sai só a
/// entrada dele. Outra entrada da mesma função do mesmo ouvinte (de outro handler ou da global, sem handler) continua. Handler sem estado já saiu, e
/// nada muda. Devolve quantas entradas saíram de `CALLBACKS`; quem chama tira os hooks sem ouvinte.
unsafe fn desregistrar_handler(handler: usize) -> usize {
    let estava = HANDLER_STATES.lock().is_ok_and(|mut states| {
        let n = states.len();
        states.retain(|(p, _)| *p != handler);
        states.len() < n
    });
    if !estava {
        return 0;
    }
    let mut saiu: Vec<usize> = Vec::new();
    if let Ok(mut c) = CALLBACKS.lock() {
        c.retain(|(_, t, _, h)| !(*h == handler && { saiu.push(*t); true }));
    }
    let tiradas = saiu.len();
    for t in saiu {
        ouvinte_solta(t);
    }
    tiradas
}

/// `CallbackLifetime.Session`: sai todo handler que não é `Forever` (`lifetime` 1), como o `OnAfterWorldDetach` da referência. O estado sai de
/// `HANDLER_STATES` e as entradas dele saem de `CALLBACKS`. Devolve quantos handlers saíram; quem chama tira os hooks sem ouvinte.
unsafe fn varrer_handlers_da_sessao() -> usize {
    let mut swept: Vec<usize> = Vec::new();
    if let Ok(mut states) = HANDLER_STATES.lock() {
        states.retain(|(p, st)| st.lifetime == 1 || { swept.push(*p); false });
    }
    if !swept.is_empty() {
        let mut saiu: Vec<usize> = Vec::new();
        if let Ok(mut c) = CALLBACKS.lock() {
            c.retain(|(_, t, _, h)| !(swept.contains(h) && { saiu.push(*t); true }));
        }
        for t in saiu {
            ouvinte_solta(t);
        }
    }
    swept.len()
}

/// Item do despacho: (ouvinte, função, handler, alvos do handler, `RunMode` do handler).
type ItemDoDisparo = (usize, u64, usize, Vec<usize>, i32);

/// As entradas de `CALLBACKS` do evento `eh` com os alvos e o `RunMode` de cada handler, copiados numa trava só de `HANDLER_STATES`, pega depois que a
/// de `CALLBACKS` cai. O despacho inteiro usa esta cópia, e o `call_func` roda sem trava: um `AddTarget`, `SetRunMode` ou `Unregister()` feito por um
/// callback do mesmo despacho vale a partir do evento seguinte. Entrada de handler sem estado fica de fora: outra thread tirou o handler entre as 2
/// travas, ou o registro dele ainda não guardou o estado. Entrada sem handler (a global) vai sem alvos e nunca é `Once`. `None` com a trava de
/// `CALLBACKS` envenenada.
fn itens_do_disparo(eh: u64) -> Option<Vec<ItemDoDisparo>> {
    let mut itens: Vec<ItemDoDisparo> =
        CALLBACKS.lock().ok()?.iter().filter(|(e, _, _, _)| *e == eh).map(|(_, t, f, h)| (*t, *f, *h, Vec::new(), 0)).collect();
    if itens.is_empty() {
        return Some(itens);
    }
    if let Ok(states) = HANDLER_STATES.lock() {
        itens.retain_mut(|(_, _, h, alvos, modo)| {
            *h == 0
                || estado_do_handler(&states, *h)
                    .map(|st| {
                        alvos.clone_from(&st.targets);
                        *modo = st.run_mode;
                    })
                    .is_some()
        });
    }
    Some(itens)
}

/// Emite um evento PASSANDO ARGS pro callback (= o evento carrega dados, ex. a tecla no input).
/// Despacha `target.function(args...)` via resolve_in_class + call_func. Devolve quantos despachou.
/// É o que os CONTROLLERS chamam quando a função de jogo hookada dispara.
pub unsafe fn fire_event_args(event_name: &str, args: &[rtti::Arg]) -> usize {
    fire_event_args_by_hash(cname(event_name), event_name, args)
}

/// Existe pelo menos 1 ouvinte registrado para o evento, sem RTTI nem `call_func`: trava e varre `CALLBACKS` e, sem ouvinte de instância,
/// `STATIC_CALLBACKS`. Os ganchos dos estágios do pipeline e os do ciclo de vida da entidade e da 4ª coluna de [`GRUPOS_DE_EVENTO`] não perguntam
/// aqui: leem o atômico que o registro e a remoção de ouvinte montam em `atualizar_eventos_de_entidade`.
pub fn has_listener_for(event_name: &str) -> bool {
    let eh = cname(event_name);
    // Callback estático também é ouvinte: o `RegisterStaticCallback` da referência ativa o evento como o de instância.
    CALLBACKS.lock().is_ok_and(|c| c.iter().any(|(ev, _, _, _)| *ev == eh)) || STATIC_CALLBACKS.lock().is_ok_and(|c| c.iter().any(|(ev, _, _)| *ev == eh))
}

/// Núcleo de `fire_event_args`, mas recebendo o hash JÁ CALCULADO — usado por chamadores que já
/// têm o CName cru (ex. `CallbackSystem.DispatchEventAs`, que recebe o hash como arg nativo e
/// evitaria um round-trip hash→string→hash desnecessário se re-chamasse `fire_event_args`).
pub unsafe fn fire_event_args_by_hash(eh: u64, event_name: &str, args: &[rtti::Arg]) -> usize {
    // `CallbackLifetime.Session` (2026-08-11, completa a metade que faltou de
    // `RunMode.Once`): no fim da sessão (logo após
    // `Session/End`) varre TODO o registry e remove qualquer handler com `lifetime != Forever`
    // (não só os que dispararam NESTE evento — `Session/End` é o "fim de mundo", todo handler
    // não-sticky perde validade). `SetLifetime` já provado (2026-07-18); mesma lógica de
    // unregister já provada 2x (`tramp_csh_unregister`, `RunMode.Once` abaixo). PRECISA rodar
    // ANTES do early-return por "ninguém registrado pra ESTE evento" — nada costuma se registrar
    // literalmente em "Session/End", o sweep age sobre OUTROS eventos (ex. "Input/Key").
    if event_name == "Session/End" {
        // Por handler: cada `RegisterCallback` é um handler, e 2 registros da mesma função do mesmo ouvinte não se confundem.
        let varridos = varrer_handlers_da_sessao();
        if varridos > 0 {
            crate::log_dev!("[cbs] CallbackLifetime.Session: {varridos} handler(s) não-sticky varridos em Session/End");
            desativar_hooks_sem_ouvinte();
        }
    }
    let Some(cbs) = itens_do_disparo(eh) else {
        return 0;
    };
    let static_cbs: Vec<(String, String)> = STATIC_CALLBACKS.lock()
        .map(|sc| sc.iter().filter(|(e, _, _)| *e == eh).map(|(_, c, f)| (c.clone(), f.clone())).collect())
        .unwrap_or_default();
    if cbs.is_empty() && static_cbs.is_empty() {
        return 0;
    }
    for a in args {
        if let rtti::Arg::Handle(inst, _) = a {
            gravar_nome_do_evento(*inst, eh);
        }
    }
    // cap de log: eventos periódicos (Update) não devem spammar — loga as ~12 primeiras emissões.
    static FIRE_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let do_log = crate::primeiras(&FIRE_LOG, 12);
    if do_log {
        crate::log_dev!(
            "[cbs] fire '{event_name}'({} arg) → {} instance + {} static callback(s)",
            args.len(), cbs.len(), static_cbs.len()
        );
    }
    let mut n = 0;
    // Tipo do evento pelo objeto do 1º argumento, calculado 1x e só quando algum handler tem alvo.
    let mut tipo_do_evento_cache: Option<TipoDoEvento> = None;
    // `CallbackRunMode.Once` (2026-08-11): coleta quem precisa ser desregistrado
    // DEPOIS do loop de disparo (nunca durante — mutar `HANDLER_STATES`/`CALLBACKS` no meio da
    // iteração de `cbs`, um snapshot já clonado do lock, é seguro, mas fazer o unregister real
    // só depois evita qualquer sutileza de reentrância).
    let mut to_unregister: Vec<(usize, u64, usize)> = Vec::new();
    for (target_us, fn_hash, handler, handler_targets, run_mode) in &cbs {
        let target = *target_us as *mut c_void;
        if target.is_null() || !rtti::sane(target) || !ouvinte_vivo(*target_us) {
            continue;
        }
        // Só contam os alvos cuja família suporta o tipo do evento (o
        // `AddTarget` recusa o resto); sem nenhum, o handler dispara; com algum, dispara se ALGUM casa. Substitui os filtros por família de
        // 2026-08-12, que exigiam que cada família presente permitisse e deixavam passar quando o dado do evento faltava. Os alvos são os da cópia
        // de `itens_do_disparo`.
        if !handler_targets.is_empty() {
            let tipo = *tipo_do_evento_cache.get_or_insert_with(|| tipo_do_evento(args));
            if alvos_barram_o_disparo(handler_targets, tipo, args) {
                if do_log {
                    crate::log_dev!(
                        "[cbs]   instance '{}' to {target:p} FILTRADO (nenhum dos {} alvo(s) que suportam {tipo:?} casou: {:?})",
                        crate::cname::resolve_cname(*fn_hash),
                        alvos_que_suportam(handler_targets, tipo).count(),
                        alvos_que_suportam(handler_targets, tipo).map(|s| s.1).collect::<Vec<_>>()
                    );
                }
                continue;
            }
        }
        if let Some(rf) = rtti::resolve_in_class_hash(rtti::class_of(target), *fn_hash) {
            let r = rtti::call_func(&rf, target, args);
            if do_log {
                crate::log_dev!("[cbs]   instance '{}' to {target:p} ok={}", crate::cname::resolve_cname(*fn_hash), r.is_some());
            }
            if r.is_some() {
                n += 1;
                // `RunMode.Once` (enum valor 1, `SetRunMode` já provado 2026-07-18): dispara UMA
                // vez, depois se desregistra sozinho pela mesma saída do `Unregister()`
                // (`desregistrar_handler`), aplicada aqui após o disparo bem-sucedido. O modo é o
                // da cópia de `itens_do_disparo`.
                if *run_mode == 1 {
                    to_unregister.push((*target_us, *fn_hash, *handler));
                }
            }
        }
    }
    for (listener, fh, handler) in &to_unregister {
        desregistrar_handler(*handler);
        if do_log {
            crate::log_dev!(
                "[cbs] RunMode.Once: auto-desregistrado listener={listener:#x} fn_hash={fh:#x} após disparar 1x"
            );
        }
    }
    if !to_unregister.is_empty() {
        desativar_hooks_sem_ouvinte();
    }
    // Dispatch estático: resolve por classe ou como global, chama com ctx=null
    if !static_cbs.is_empty() {
        if let Some(reg) = rtti::Registry::obtain() {
            for (cls, fname) in &static_cbs {
                let rf = rtti::resolve_func(&reg, cls, fname)
                    .or_else(|| {
                        let fp = get_function(&reg, fname);
                        if !fp.is_null() {
                            Some(rtti::ResolvedFn { func: fp, ret_type: std::ptr::null_mut(), is_static: true })
                        } else {
                            None
                        }
                    });
                match rf {
                    Some(rf) => {
                        let r = rtti::call_func(&rf, std::ptr::null_mut(), args);
                        if do_log {
                            crate::log_dev!("[cbs]   static '{cls}::{fname}' ok={}", r.is_some());
                        }
                        if r.is_some() { n += 1; }
                    }
                    None => {
                        if do_log {
                            crate::log_dev!("[cbs]   static '{cls}::{fname}' não resolveu (mod ainda não carregado?)");
                        }
                    }
                }
            }
        }
    }
    n
}

/// Emite um evento SEM args (callbacks no-arg). Atalho de `fire_event_args`.
pub unsafe fn fire_event(event_name: &str) -> usize {
    fire_event_args(event_name, &[])
}

/// Nomes antigos que a referência aceita no registro e na remoção (`GetMappings` dos controllers): `Entity/Attached` do `EntityAttachHook` e os
/// nomes de antes dos hooks de recurso.
pub(crate) const EVENTOS_RENOMEADOS: [(&str, &str); 3] = [("Entity/Attached", "Entity/AfterAttach"), ("Resource/Loaded", "Resource/Load"), ("Resource/Ready", "Resource/PostLoad")];

/// `MapEventName` da referência: o hash do nome atual quando `ev` é um nome antigo.
pub(crate) fn evento_mapeado(ev: u64) -> u64 {
    EVENTOS_RENOMEADOS.iter().find(|(antigo, _)| cname(antigo) == ev).map_or(ev, |(_, atual)| cname(atual))
}

/// Linha de [`GRUPOS_DE_EVENTO`]: eventos, instalação, remoção (`None` = o hook fica) e atômico que o hook lê (`None` = o hook não lê atômico da tabela).
type GrupoDeEvento = (&'static [&'static str], unsafe fn(), Option<unsafe fn()>, Option<&'static std::sync::atomic::AtomicBool>);

/// Cada grupo de hooks com os eventos que ele atende:
/// o que o registro instala (`ativar_hooks_do_evento`), o que a remoção tira quando nenhum evento da linha tem ouvinte (`desativar_hooks_sem_ouvinte`) e
/// o atômico que o hook lê (`atualizar_eventos_de_entidade`). Evento fora da tabela não sobe nem derruba hook ao registrar ou remover ouvinte. A ordem
/// das linhas é a da desativação.
/// - Os 6 `Entity/*` são um grupo só: o despacho do player em `lib.rs` lê o bit do hook do próprio evento (`HOOKS_DO_CICLO_NO_AR`, `evento_do_player`). O hook
///   deles lê o bit de cada evento em `ENTIDADE_EVENTOS_ARMADOS`, não um atômico da tabela.
/// - O `Resource/Load` liga o despacho por `RESOURCE_LOAD_ATIVO`, dentro do próprio `garantir_hooks_de_recurso`.
/// - O `Entity/Extract` não tem remoção: o hook do job de componentes do builder é o do patch de recursos (`install_job_do_builder_hook`), e o evento ali só
///   lê o atômico.
/// - O `InkWidget/Spawn` não tem remoção: os hooks do spawn de widget também trocam o controller do `Item:Controller`, que não tem ouvinte. O atômico diz
///   só se há ouvinte; a chave do `BwmsToggleWidgetSpawnEvent` é outro atômico, que o despacho lê junto (`spawn_de_widget::evento_ligado`).
const GRUPOS_DE_EVENTO: [GrupoDeEvento; 7] = [
    (&ENTIDADE_EVENTOS, crate::entidades::garantir_hooks_do_ciclo_de_vida, Some(desativar_hooks_do_ciclo_de_vida), None),
    (&["Entity/Assemble"], crate::entidades::garantir_hook_do_assemble, Some(desativar_hook_do_assemble), Some(&ASSEMBLE_ARMADO)),
    (&["Component/Toggle"], crate::aparencia::garantir_hooks_de_toggle, Some(desativar_hooks_de_toggle), Some(&COMPONENT_TOGGLE_ARMADO)),
    (&["Vehicle/ToggleAuxLights"], crate::veiculos::garantir_hook_de_luz, Some(desativar_hook_de_luz), Some(&VEHICLE_LIGHT_ARMADO)),
    (&["Resource/Load"], crate::arquivos_do_jogo::garantir_hooks_de_recurso, Some(desativar_hooks_de_recurso), None),
    (&["Entity/Extract"], crate::entidades::garantir_hook_do_extract, None, Some(&EXTRACT_ARMADO)),
    (&["InkWidget/Spawn"], crate::interface::spawn_de_widget::instalar, None, Some(&crate::interface::WIDGET_SPAWN_ARMADO)),
];

/// Linha de [`GRUPOS_DE_EVENTO`] que atende `ev`, o hash do nome que o despacho usa (depois de `evento_mapeado`).
fn grupo_do_evento(ev: u64) -> Option<usize> {
    GRUPOS_DE_EVENTO.iter().position(|(eventos, ..)| eventos.iter().any(|nome| cname(nome) == ev))
}

/// Um ouvinte novo, de instância ou estático, instala o grupo de hooks do evento e só depois arma os atômicos que os hooks leem, como o `ActivateEvent` da
/// referência. Evento sem grupo não instala hook, mas também arma: os ganchos dos estágios do pipeline sobem pelo tick, sem depender de ouvinte, e só
/// leem `PIPELINE_ARMADOS`.
pub(crate) fn ativar_hooks_do_evento(ev: u64) {
    let _grupo = HOOKS_DE_EVENTO.lock();
    if let Some(k) = grupo_do_evento(ev) {
        let (_, garantir, _, _) = GRUPOS_DE_EVENTO[k];
        unsafe { garantir() };
    }
    atualizar_eventos_de_entidade();
}

/// Serializa a ativação e a desativação dos grupos de hooks de evento: um registro e uma remoção ao mesmo tempo em threads diferentes não instalam e tiram
/// o mesmo hook em ordem trocada, nem empilham 2 hooks no mesmo endereço.
pub(crate) static HOOKS_DE_EVENTO: Mutex<()> = Mutex::new(());

/// Depois de uma remoção de ouvinte, cada grupo de [`GRUPOS_DE_EVENTO`] com remoção cujos eventos ficaram todos sem ouvinte cai com as
/// outras threads paradas, como o `DeactivateEvent` da referência. Os atômicos descem antes: até a remoção, o hook que ainda roda já não despacha.
pub(crate) fn desativar_hooks_sem_ouvinte() {
    atualizar_eventos_de_entidade();
    let _grupo = HOOKS_DE_EVENTO.lock();
    for (_, desativar) in grupos_sem_ouvinte() {
        unsafe { desativar() };
    }
}

/// Linha e remoção de cada grupo de [`GRUPOS_DE_EVENTO`] com remoção cujos eventos estão todos sem ouvinte, na ordem da tabela. Preguiçoso: cada linha
/// lê os ouvintes logo antes da própria remoção.
fn grupos_sem_ouvinte() -> impl Iterator<Item = (usize, unsafe fn())> {
    GRUPOS_DE_EVENTO.into_iter().enumerate().filter_map(|(k, (eventos, _, desativar, _))| {
        desativar.filter(|_| !eventos.iter().any(|nome| has_listener_for(nome))).map(|desativar| (k, desativar))
    })
}

/// Tira o hook nosso de `vm` com as outras threads paradas; `None` sem hook no endereço.
unsafe fn tirar_com_threads_paradas(vm: u64) -> Option<(bool, u32)> {
    let end = crate::rebase(vm) as *mut c_void;
    (crate::ganchos::ganchos_no_endereco(end) > 0).then(|| crate::ganchos::remover_com_threads_paradas(end))
}

/// Tira os hooks do ciclo de vida com as outras threads paradas quando nenhum evento `Entity/*` tem ouvinte.
pub(crate) unsafe fn desativar_hooks_do_ciclo_de_vida() {
    let (mut estado, mut ocupadas) = (Vec::new(), 0);
    for (k, _, _) in crate::entidades::hooks_do_ciclo() {
        let alvo = &crate::entidades::ALVOS_DO_CICLO_DE_VIDA[k];
        let end = crate::rebase(alvo.vm) as *mut c_void;
        if crate::ganchos::ganchos_no_endereco(end) == 0 {
            continue;
        }
        let (ok, tentativas) = crate::ganchos::remover_com_threads_paradas(end);
        ocupadas += tentativas;
        estado.push(format!("{} {}", alvo.nome, if ok { "fora" } else { "ficou" }));
    }
    crate::entidades::gravar_hooks_do_ciclo_no_ar();
    if !estado.is_empty() {
        crate::veredito_dev!("[c130b] hooks do ciclo de vida sem ouvinte: {} ({ocupadas} tentativa(s) com PC no trecho)", estado.join(", "));
    }
}

/// Tira o hook do `Entity::Assemble` com as outras threads paradas quando o `Entity/Assemble` fica sem ouvinte.
pub(crate) unsafe fn desativar_hook_do_assemble() {
    let end = crate::rebase(crate::entidades::ENTITY_ASSEMBLE_VM) as *mut c_void;
    if crate::ganchos::ganchos_no_endereco(end) == 0 {
        return;
    }
    let (ok, ocupadas) = crate::ganchos::remover_com_threads_paradas(end);
    crate::diagnostico::C130C_HOOK_OK.store(!ok, Ordering::Relaxed);
    crate::veredito_dev!("[c130c] hook do Entity::Assemble sem ouvinte: {} ({ocupadas} tentativa(s) com PC no trecho)", if ok { "fora" } else { "ficou" });
}

/// Tira os 2 hooks do `Toggle` quando o `Component/Toggle` fica sem ouvinte.
pub(crate) unsafe fn desativar_hooks_de_toggle() {
    let (script, cpp) =
        (tirar_com_threads_paradas(crate::aparencia::ICOMP_TOGGLE_VM), tirar_com_threads_paradas(crate::aparencia::ICOMP_TOGGLE_CPP_VM));
    if script.is_some() || cpp.is_some() {
        crate::veredito_dev!("[toggle] hooks do IComponent::Toggle sem ouvinte (tirado, tentativas com PC no trecho): handler de script {script:?}, função C++ {cpp:?}");
    }
}

/// Tira o hook do `ToggleLights` quando o `Vehicle/ToggleAuxLights` fica sem ouvinte.
pub(crate) unsafe fn desativar_hook_de_luz() {
    if let Some(r) = tirar_com_threads_paradas(crate::veiculos::VEHICLE_AUX_VM) {
        crate::veredito_dev!("[luz] hook do vehicleController::ToggleLights sem ouvinte (tirado, tentativas com PC no trecho): {r:?}");
    }
}

/// Sem ouvinte de `Resource/Load`, desliga o despacho e tira o hook do `OnDependenciesReady` que o despacho de recurso pôs; com o patch de
/// recursos usando o mesmo hook, ele fica.
pub(crate) unsafe fn desativar_hooks_de_recurso() {
    use crate::arquivos_do_jogo::GANCHO_DEPENDENCIAS_NO_AR;
    crate::arquivos_do_jogo::RESOURCE_LOAD_ATIVO.store(false, Ordering::Relaxed);
    if !GANCHO_DEPENDENCIAS_NO_AR.load(Ordering::Relaxed)
        || crate::curvas::A51G_ATIVO.load(Ordering::Relaxed)
        || crate::mundo::MESCLA_DE_DEVICES_ATIVA.load(Ordering::Relaxed)
    {
        return;
    }
    if let Some((ok, ocupadas)) = tirar_com_threads_paradas(crate::jogo::recursos::DEPENDENCIAS_PRONTAS_VM) {
        GANCHO_DEPENDENCIAS_NO_AR.store(!ok, Ordering::Relaxed);
        crate::veredito_dev!(
            "[c122] hook do OnDependenciesReady sem ouvinte de Resource/Load: {} ({ocupadas} tentativa(s) com PC no trecho)",
            if ok { "fora" } else { "ficou" }
        );
    }
}

/// Remonta [`ENTIDADE_EVENTOS_ARMADOS`], `PIPELINE_ARMADOS` e os atômicos de [`GRUPOS_DE_EVENTO`] a partir dos ouvintes registrados.
pub fn atualizar_eventos_de_entidade() {
    let mascara = ENTIDADE_EVENTOS.iter().enumerate().fold(0u32, |m, (k, nome)| if has_listener_for(nome) { m | 1 << k } else { m });
    ENTIDADE_EVENTOS_ARMADOS.store(mascara, Ordering::Relaxed);
    let pipeline = EVENTOS_DO_PIPELINE.iter().enumerate().fold(0u8, |m, (k, nome)| if has_listener_for(nome) { m | 1 << k } else { m });
    PIPELINE_ARMADOS.store(pipeline, Ordering::Relaxed);
    for (eventos, _, _, armado) in GRUPOS_DE_EVENTO {
        if let Some(armado) = armado {
            armado.store(eventos.iter().any(|nome| has_listener_for(nome)), Ordering::Relaxed);
        }
    }
}

pub fn tem_ouvinte(event_name: &str) -> bool {
    // Delega pra `has_listener_for`, que varre o `CALLBACKS` real. Manter uma tabela PARALELA de
    // contagem no caminho quente seria uma 2ª fonte de verdade capaz de divergir da 1ª — e
    // divergir aqui significa evento silenciosamente não entregue, o pior defeito possível nesta
    // camada. A mesma tabela decide instalar e tirar os hooks (`ativar_hooks_do_evento`/`desativar_hooks_sem_ouvinte`).
    has_listener_for(event_name)
}

/// Porta única de entrada em `CALLBACKS`, da classe (`CallbackSystem.RegisterCallback`) e da global (`BwmsRegisterCallback`): o alias do evento
/// vira o nome que o despacho usa (`evento_mapeado`, tabela `EVENTOS_RENOMEADOS`), o ouvinte passa a ser seguido em `OUVINTES` e o hook do evento
/// sobe (`ativar_hooks_do_evento`). `handler` é 0 na global. Devolve o hash do evento guardado, que quem chama só usa no log; `None` com evento,
/// ouvinte ou função zerados, ou com a trava de `CALLBACKS` envenenada.
pub(crate) unsafe fn registrar_ouvinte(ev_bruto: u64, alvo: usize, fn_hash: u64, handler: usize) -> Option<u64> {
    let ev = evento_mapeado(ev_bruto);
    if ev == 0 || alvo == 0 || fn_hash == 0 {
        return None;
    }
    // A trava de `CALLBACKS` cai no fim desta linha: `ativar_hooks_do_evento` volta a lê-la por `has_listener_for`.
    CALLBACKS.lock().ok()?.push((ev, alvo, fn_hash, handler));
    ouvinte_segura(alvo);
    ativar_hooks_do_evento(ev);
    Some(ev)
}

/// Corpo de `CallbackSystem.RegisterCallback` depois de a fábrica criar o handler: a entrada em `CALLBACKS` (`registrar_ouvinte`) e, com ela aceita,
/// o estado do handler em `HANDLER_STATES`. Handler 0 (a fábrica falhou) fica só com a entrada, como a global.
unsafe fn registrar_handler(ev_bruto: u64, alvo: usize, fn_hash: u64, handler: usize) -> Option<u64> {
    let ev = registrar_ouvinte(ev_bruto, alvo, fn_hash, handler)?;
    if handler != 0 {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            states.push((handler, HandlerState { targets: Vec::new(), run_mode: 0, lifetime: 0 }));
        }
    }
    Some(ev)
}

pub(crate) unsafe extern "C" fn tramp_cbs_register_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev_bruto = args.first().map(|(v, _)| *v).unwrap_or(0);
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    // O handler nasce primeiro, porque a entrada guarda o ponteiro dele: o despacho, a remoção e o `RunMode.Once` acham os alvos por ele. A
    // validade se decide pelo nome bruto, porque o mapeado só é zero quando o bruto é zero.
    let valido = ev_bruto != 0 && target != 0 && fn_hash != 0;
    let mut handler: *mut c_void = std::ptr::null_mut();
    if valido {
        if let Some(reg) = rtti::Registry::obtain() {
            handler = rtti::new_object(&reg, "CallbackSystemHandler");
        }
    }
    let ev = if valido { registrar_handler(ev_bruto, target as usize, fn_hash, handler as usize) } else { None };
    let ok = ev.is_some();
    if ok && !handler.is_null() {
        // O registro guarda o ponteiro do handler em `HANDLER_STATES`: sem referência forte própria, o objeto morre quando o script solta o handle e
        // o endereço volta pra próxima fábrica — no boot 174 o `AddTarget` de um handler novo achou a entrada velha no mesmo endereço e os alvos
        // foram pro handler errado.
        retain_strong_ref_leaked(handler);
    }
    crate::log_dev!(
        "[cbs] CallbackSystem.RegisterCallback(event '{}', target={target:#x}, fn '{}') ok={ok} -> handler={handler:p} (handle real via make_handle, ver nota 2026-07-18 acima)",
        crate::cname::resolve_cname(ev.unwrap_or(ev_bruto)), crate::cname::resolve_cname(fn_hash)
    );
    rtti::make_handle(out, handler);
    // Diagnóstico barato (mantido, 2026-07-18): confirma que `make_handle` realmente escreveu um
    // bloco de refcount NÃO-NULO em `[+0x08]` (não só o ponteiro do objeto em `[+0x00]`) — achado
    // que refutou a hipótese de "Context null-check vê isto como null" ao investigar por que a
    // forma ENCADEADA (`RegisterCallback(...).AddTarget(...)` numa expressão só) pula as chamadas
    // seguintes mesmo com um handle 100% válido (ver nota grande no topo desta função).
    if !out.is_null() {
        let raw0 = core::ptr::read_unaligned(out as *const u64);
        let raw1 = core::ptr::read_unaligned((out as *const u64).add(1));
        crate::log_dev!("[cbs][diag] out lido de volta: [+0x00]={raw0:#x} [+0x08]={raw1:#x}");
    }
}

/// `RegisterStaticCallback(eventName, target: CName, function: CName, opt sticky) -> ref<CallbackSystemHandler>`
/// — FUNCIONAL: guarda (event_hash, class_name, fn_name) em `STATIC_CALLBACKS`; o dispatch ocorre
/// em `fire_event_args_by_hash` via `rtti::resolve_func` + `call_func` com ctx=null (estático).
pub(crate) unsafe extern "C" fn tramp_cbs_register_static_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev_hash   = evento_mapeado(args.first().map(|(v, _)| *v).unwrap_or(0));
    // A declaração do jogo (2.5) passa o alvo como `ref<IScriptable>` e o contexto é a classe dele; a forma anterior, o nome (`CName`).
    let cls_hash  = match args.get(1) {
        Some(&(v, tc)) if tc != cname("CName") => {
            let cls = rtti::class_of(v as *mut c_void);
            if rtti::sane(cls) { rtti::type_name_hash(cls) } else { 0 }
        }
        Some(&(v, _)) => v,
        None => 0,
    };
    let fn_hash   = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let cls_name  = crate::cname::resolve_cname(cls_hash);
    let fn_name   = crate::cname::resolve_cname(fn_hash);
    let ev_name   = crate::cname::resolve_cname(ev_hash);
    let stored = if ev_hash != 0 && cls_hash != 0 && fn_hash != 0 {
        if let Ok(mut sc) = STATIC_CALLBACKS.lock() {
            let dup = sc.iter().any(|(e, c, f)| *e == ev_hash && c == &cls_name && f == &fn_name);
            if !dup { sc.push((ev_hash, cls_name.clone(), fn_name.clone())); true } else { false }
        } else { false }
    } else { false };
    if stored {
        ativar_hooks_do_evento(ev_hash);
    }
    crate::log_dev!(
        "[cbs] CallbackSystem.RegisterStaticCallback('{}', class='{}', fn='{}') -> stored={stored}",
        ev_name, cls_name, fn_name
    );
    write_handle_ret(out, 0); // retorna null handler (static não usa handler object)
}

/// Corpo de `UnregisterCallback`: tira de `CALLBACKS` as entradas de (`ev`, `target`) e, com `fn_hash` != 0, só as dessa função. O estado do handler
/// de cada entrada tirada sai de `HANDLER_STATES`. Devolve quantas entradas saíram; quem chama tira os hooks sem ouvinte.
unsafe fn tirar_ouvinte_do_evento(ev: u64, target: u64, fn_hash: u64) -> usize {
    let mut tirados: Vec<usize> = Vec::new();
    let removed = CALLBACKS
        .lock()
        .map(|mut c| {
            let before = c.len();
            c.retain(|(e, t, f, h)| {
                let sai = *e == ev && *t as u64 == target && (fn_hash == 0 || *f == fn_hash);
                if sai && *h != 0 {
                    tirados.push(*h);
                }
                !sai
            });
            before - c.len()
        })
        .unwrap_or(0);
    // Todas as entradas tiradas são deste ouvinte.
    for _ in 0..removed {
        ouvinte_solta(target as usize);
    }
    if !tirados.is_empty() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            states.retain(|(p, _)| !tirados.contains(p));
        }
    }
    removed
}

/// `UnregisterCallback(eventName, target, opt function)` — FUNCIONAL: remove da `CALLBACKS`
/// registry as entradas que baterem (event, target) e, se `function` foi passado (hash != 0),
/// também `function` (`tirar_ouvinte_do_evento`). Void (nada a escrever em `out`).
pub(crate) unsafe extern "C" fn tramp_cbs_unregister_callback(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = evento_mapeado(args.first().map(|(v, _)| *v).unwrap_or(0));
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let removed = tirar_ouvinte_do_evento(ev, target, fn_hash);
    if removed > 0 {
        desativar_hooks_sem_ouvinte();
    }
    crate::log_dev!(
        "[cbs] CallbackSystem.UnregisterCallback(event '{}', target={target:#x}) removidos={removed}",
        crate::cname::resolve_cname(ev)
    );
}

/// `UnregisterStaticCallback(eventName, target: CName, opt function)` — FUNCIONAL: remove entradas
/// de `STATIC_CALLBACKS` que batam (event, class, opt fn). Void.
pub(crate) unsafe extern "C" fn tramp_cbs_unregister_static_callback(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev_hash  = evento_mapeado(args.first().map(|(v, _)| *v).unwrap_or(0));
    let cls_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash  = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let cls_name = crate::cname::resolve_cname(cls_hash);
    let fn_name  = crate::cname::resolve_cname(fn_hash);
    let removed = if let Ok(mut sc) = STATIC_CALLBACKS.lock() {
        let before = sc.len();
        sc.retain(|(e, c, f)| {
            !(*e == ev_hash && c == &cls_name && (fn_hash == 0 || f == &fn_name))
        });
        before - sc.len()
    } else { 0 };
    if removed > 0 {
        desativar_hooks_sem_ouvinte();
    }
    crate::log_dev!(
        "[cbs] CallbackSystem.UnregisterStaticCallback(event='{}', class='{}') -> removidos={removed}",
        crate::cname::resolve_cname(ev_hash), cls_name
    );
}

/// `RegisterEvent(eventName, opt eventType) -> Bool` — permissivo: sempre `true`. Guarda o mapa
/// type_cname→event_cname para que `DispatchEvent` consiga derivar o nome do evento a partir do
/// tipo do objeto passado.
pub(crate) unsafe extern "C" fn tramp_cbs_register_event(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let ty_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    if ty_hash != 0 {
        if let Ok(mut m) = EVENT_TYPE_MAP.lock() {
            if !m.iter().any(|(t, _)| *t == ty_hash) {
                m.push((ty_hash, ev_hash));
            }
        }
    }
    crate::log_dev!(
        "[cbs] CallbackSystem.RegisterEvent('{}', type={:#018x}) -> true",
        crate::cname::resolve_cname(ev_hash), ty_hash
    );
    write_bool_ret(out, true);
}

/// `DispatchEvent(eventObject: ref<CallbackSystemEvent>)` — FUNCIONAL via EVENT_TYPE_MAP:
/// lê o tipo do objeto passado via RTTI, consulta o mapa type→event registrado por
/// `RegisterEvent`, e delega a `fire_event_args_by_hash`. Void. Fallback: loga o tipo
/// desconhecido (sem crash, sem dispatch silencioso).
///
/// FIX 2026-08-11 (5ª rodada de mineração de candidatos baratos, achado de
/// bookkeeping/dependência-já-implementada): a versão anterior lia `obj_vtable+0x10`
/// DIRETO no vtable do OBJETO como se fosse "GetName" — isso não bate com o padrão já
/// PROVADO e usado em TODO o resto do projeto (`rtti::class_of`/`rtti::type_name_getname`,
/// dezenas de call-sites): o nome do TIPO só é resolvido via um `IRTTIType::GetName()`
/// (vtable+0x10) chamado numa `CClass*` (o TIPO), nunca direto no vtable da INSTÂNCIA (que
/// é o vtable C++ da classe concreta, com slots totalmente diferentes — `class_of` é quem
/// já sabe o slot certo do objeto pra achar o `GetType()`, vtable+0x08, não +0x10). Composição
/// correta: 2 passos, ambos já mecanicamente provados em produção — `rtti::class_of(obj_ptr)`
/// (obj vtable+0x08 = `GetType(this) -> CClass*`) seguido de `rtti::type_name_getname(cls_ptr)`
/// (CClass vtable+0x10 = `GetName()`, a MESMA função que `class_of`+`type_name_getname` já usam
/// com sucesso em dezenas de outros lugares, ex. `class_own_functions`/`funclistdump`). Zero
/// endereço novo — só a composição estava errada (faltava o passo intermediário `class_of`).
pub(crate) unsafe extern "C" fn tramp_cbs_dispatch_event(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    use std::ffi::c_void;
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    // arg0 = ref<CallbackSystemEvent>: handle = (ptr, refcnt) — pegamos só o ptr
    let obj_ptr = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    if obj_ptr.is_null() || !crate::memoria::is_readable(obj_ptr as *const c_void, 8) {
        crate::log_dev!("[cbs] DispatchEvent: objeto null ou ilegível");
        return;
    }
    // class_of: obj_vtable+0x08 = GetType(this) -> CClass* (já validado/provado, ganchos::is_readable
    // + checagem de endereço-no-módulo embutidas). type_name_getname: CClass_vtable+0x10 =
    // GetName() -> CName hash (idem, já provado em dezenas de call-sites).
    let cls_ptr = rtti::class_of(obj_ptr);
    if cls_ptr.is_null() {
        crate::log_dev!("[cbs] DispatchEvent: class_of(objeto) falhou (vtable ilegível/fora do módulo)");
        return;
    }
    let type_hash = rtti::type_name_getname(cls_ptr);
    if type_hash == 0 {
        crate::log_dev!("[cbs] DispatchEvent: type_name_getname(classe) devolveu 0");
        return;
    }
    // Consulta o mapa type→event registrado por RegisterEvent
    let ev_hash = EVENT_TYPE_MAP.lock().ok()
        .and_then(|m| m.iter().find(|(t, _)| *t == type_hash).map(|(_, e)| *e));
    match ev_hash {
        Some(eh) if eh != 0 => {
            let ev_name = crate::cname::resolve_cname(eh);
            let n = fire_event_args_by_hash(eh, &ev_name, &[]);
            crate::log_dev!("[cbs] DispatchEvent(type={type_hash:#018x} → '{}') → {n} callback(s)", ev_name);
        }
        _ => {
            let type_name = crate::cname::resolve_cname(type_hash);
            crate::log_dev!("[cbs] DispatchEvent: tipo '{type_name}' sem mapeamento (chamar RegisterEvent antes)");
        }
    }
}

/// `DispatchEventAs(eventName, eventObject: ref<CallbackSystemEvent>) -> Void` — PARCIALMENTE
/// FUNCIONAL: dispara a `CALLBACKS` registry pelo NOME (via `fire_event_args_by_hash`, reusando o
/// hash já lido, sem round-trip). O PAYLOAD do `eventObject` (campos do evento) não é marshallado
/// pro alvo — só o disparo por nome funciona, igual ao `fire_event`/`fire_event_args` já provados.
pub(crate) unsafe extern "C" fn tramp_cbs_dispatch_event_as(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(ev);
    let n = fire_event_args_by_hash(ev, &name, &[]);
    crate::log_dev!(
        "[cbs] CallbackSystem.DispatchEventAs(event '{name}') -> disparou por NOME ({n} callback(s); payload do eventObject não marshallado)"
    );
}

pub unsafe fn register_callbacksystem(reg: &Registry) -> String {
    // `IGameSystem` é classe nativa REAL do próprio motor (não precisa forjar) — mas
    // `reg.class_by_name` não a resolve (via errada, ver `resolve_class_via_validator_getclass`).
    // Tenta a via normal primeiro (barata, funciona pra quase tudo); só usa a via alternativa (que
    // sabemos ser a que o validador usa de verdade) se a normal falhar; forja sintética como
    // ÚLTIMO recurso (nunca deveria disparar pra uma classe engine real como esta).
    let igs = reg.class_by_name("IGameSystem");
    let igs = if rtti::sane(igs) { igs } else { resolve_class_via_validator_getclass("IGameSystem") };
    let igs = if rtti::sane(igs) { igs } else { register_type_min(reg, "IGameSystem") };
    if !rtti::sane(igs) {
        return "[reg] register_callbacksystem: 'IGameSystem' não resolveu por NENHUMA via (nem forja)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "CallbackSystem", "gameuiInGameMenuGameController", igs);
    if forged.is_null() {
        return "[reg] register_callbacksystem: forja da classe FALHOU (ver log acima)".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_callbacksystem: sem protótipo de método (AddGodMode não resolveu)".into(),
    };
    let m1 = register_method(reg, "CallbackSystem", proto, "RegisterCallback", "RegisterCallback", tramp_cbs_register_callback, false);
    let m2 = register_method(reg, "CallbackSystem", proto, "RegisterStaticCallback", "RegisterStaticCallback", tramp_cbs_register_static_callback, false);
    let m3 = register_method(reg, "CallbackSystem", proto, "UnregisterCallback", "UnregisterCallback", tramp_cbs_unregister_callback, false);
    let m4 = register_method(reg, "CallbackSystem", proto, "UnregisterStaticCallback", "UnregisterStaticCallback", tramp_cbs_unregister_static_callback, false);
    let m5 = register_method(reg, "CallbackSystem", proto, "RegisterEvent", "RegisterEvent", tramp_cbs_register_event, false);
    let m6 = register_method(reg, "CallbackSystem", proto, "DispatchEvent", "DispatchEvent", tramp_cbs_dispatch_event, false);
    let m7 = register_method(reg, "CallbackSystem", proto, "DispatchEventAs", "DispatchEventAs", tramp_cbs_dispatch_event_as, false);
    // GetCallbackSystem NÃO é registrado aqui — ACHADO AO VIVO (2026-07-13): `@addMethod(GameInstance)`
    // crasha o boot, porque a classe "GameInstance" é validada MUITO CEDO (uma das primeiras ~100
    // entradas do bundle, ANTES até do 1º `register_all()` — que só roda no 1º kind=5/função-global,
    // processado DEPOIS de todas as classes). Nenhum hook desta sessão roda cedo o bastante. Fix:
    // exposto como GLOBAL `BwmsGetCallbackSystem()` (ver `register_all`), mesmo padrão de toda
    // Bwms* function — funciona, só não é a chamada LITERAL `GameInstance.GetCallbackSystem()` que
    // um mod real do Windows esperaria (ver `ponte-callbacksystem.reds`).
    format!(
        "[reg] CallbackSystem: classe=OK RegisterCallback={m1} RegisterStaticCallback={m2} UnregisterCallback={m3} UnregisterStaticCallback={m4} RegisterEvent={m5} DispatchEvent={m6} DispatchEventAs={m7}"
    )
}

/// `CallbackSystemTarget` — classe abstrata SEM métodos, só precisa existir como TIPO pra
/// `CallbackSystemHandler.AddTarget(ref<CallbackSystemTarget>)` type-checar no compilador.
pub unsafe fn register_callbacksystemtarget(reg: &Registry) -> String {
    let forged = register_type_min(reg, "CallbackSystemTarget");
    format!("[reg] CallbackSystemTarget: forjada={}", !forged.is_null())
}

/// `AddTarget(target: ref<CallbackSystemTarget>) -> ref<CallbackSystemHandler>` — FUNCIONAL
/// (2026-07-18): adiciona `target` (ponteiro) na lista de alvos do handler (`c` = `this`, ver
/// `HANDLER_STATES`) se ainda não estiver lá (dedupe), e devolve `self` (`c`) — o encadeamento
/// real (`RegisterCallback(...).AddTarget(...).AddTarget(...)`) funciona porque cada chamada
/// devolve o MESMO ponteiro de handler. Lista de alvos = O FILTRO (ver `tramp_csh_has_target`):
/// vazia = sem filtro (aceita qualquer alvo); não-vazia = só aceita alvos presentes nela. Handler
/// desregistrado não tem estado: o alvo não entra e não ganha a referência forte.
pub(crate) unsafe extern "C" fn tramp_csh_add_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut found = false;
    if !c.is_null() && target_arg != 0 {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().rev().find(|(p, _)| *p == c as usize) {
                if !st.targets.contains(&(target_arg as usize)) {
                    st.targets.push(target_arg as usize);
                    // fix 2026-08-12: retenção forte p/ a cópia de vida longa em `targets` — sem
                    // isso o único dono forte é o `ref<>` temporário do argumento (ver doc de
                    // `retain_strong_ref_leaked`, mesma causa-raiz das factories de EntityTarget).
                    retain_strong_ref_leaked(target_arg as *mut c_void);
                }
                found = true;
            }
        }
    }
    crate::log_dev!("[csh] CallbackSystemHandler.AddTarget(target={target_arg:#x}) on {c:p} handler_achado={found}");
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}

/// `RemoveTarget(target: ref<CallbackSystemTarget>) -> ref<CallbackSystemHandler>` — FUNCIONAL:
/// remove `target` da lista (se presente), devolve `self` (mesmo padrão de `AddTarget`).
pub(crate) unsafe extern "C" fn tramp_csh_remove_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    if !c.is_null() && target_arg != 0 {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().rev().find(|(p, _)| *p == c as usize) {
                st.targets.retain(|t| *t != target_arg as usize);
            }
        }
    }
    crate::log_dev!("[csh] CallbackSystemHandler.RemoveTarget(target={target_arg:#x}) on {c:p}");
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}

/// `SetRunMode(runMode: CallbackRunMode) -> ref<CallbackSystemHandler>` — FUNCIONAL: guarda o
/// valor do enum (lido como i64 cru, mesmo esquema de qualquer param — enums são POD) no estado
/// do handler, devolve `self`.
pub(crate) unsafe extern "C" fn tramp_csh_set_run_mode(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let mode = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    if !c.is_null() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().rev().find(|(p, _)| *p == c as usize) {
                st.run_mode = mode;
            }
        }
    }
    crate::log_dev!("[csh] CallbackSystemHandler.SetRunMode({mode}) on {c:p}");
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}

/// `SetLifetime(lifetime: CallbackLifetime) -> ref<CallbackSystemHandler>` — FUNCIONAL: mesmo
/// padrão de `SetRunMode`.
pub(crate) unsafe extern "C" fn tramp_csh_set_lifetime(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let lifetime = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    if !c.is_null() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().rev().find(|(p, _)| *p == c as usize) {
                st.lifetime = lifetime;
            }
        }
    }
    crate::log_dev!("[csh] CallbackSystemHandler.SetLifetime({lifetime}) on {c:p}");
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}

/// `IsRegistered() -> Bool` — FUNCIONAL: `true` do `RegisterCallback` até a saída do handler (`Unregister()`, `UnregisterCallback`,
/// `RunMode.Once` ou `Session/End`), que tira o estado dele de `HANDLER_STATES` (`handler_registrado`).
pub(crate) unsafe extern "C" fn tramp_csh_is_registered(c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let reg = !c.is_null() && handler_registrado(c as usize);
    crate::log_dev!("[csh] CallbackSystemHandler.IsRegistered() on {c:p} -> {reg}");
    write_bool_ret(out, reg);
}

/// `Unregister()` — FUNCIONAL: tira o estado do handler de `HANDLER_STATES` e de `CALLBACKS` a entrada que guarda o ponteiro dele
/// (`desregistrar_handler`) — daí em diante o evento NÃO dispara mais pra ela. Void.
pub(crate) unsafe extern "C" fn tramp_csh_unregister(c: *mut c_void, _frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let removed_cb = if c.is_null() { 0 } else { desregistrar_handler(c as usize) };
    if removed_cb > 0 {
        desativar_hooks_sem_ouvinte();
    }
    crate::log_dev!("[csh] CallbackSystemHandler.Unregister() on {c:p} — removeu {removed_cb} entrada(s) de CALLBACKS");
}

/// `HasTarget(target: ref<CallbackSystemTarget>) -> Bool` — BWMS-ONLY (adicionado 2026-07-18 pra tornar o "filtro de target" testável/observável): devolve
/// `true` se `target` está na lista adicionada via `AddTarget` (ou se a lista está vazia — "sem
/// filtro" aceita qualquer alvo, mesmo esquema documentado em `HandlerState`). Prova, por chamada
/// de método REAL (não só leitura de estado interno), que `AddTarget`/`RemoveTarget` mutam um
/// filtro genuíno e que ele DIFERENCIA alvos corretamente. Handler desregistrado não tem estado: `false`.
pub(crate) unsafe extern "C" fn tramp_csh_has_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut has = false;
    if !c.is_null() {
        if let Ok(states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter().rev().find(|(p, _)| *p == c as usize) {
                has = st.targets.is_empty() || st.targets.contains(&(target_arg as usize));
            }
        }
    }
    crate::log_dev!("[csh] CallbackSystemHandler.HasTarget(target={target_arg:#x}) on {c:p} -> {has}");
    write_bool_ret(out, has);
}

/// Forja `CallbackSystemHandler` — INSTANCIÁVEL de verdade (2026-07-18, handler de callback
/// FECHADO). Histórico: TENTADO 2026-07-15 como classe (métodos com `ref<X>` de classe forjada) —
/// CRASHOU (SIGTRAP), mesma categoria genérica do crash de `GetService`/`ScriptableServiceContainer`.
/// Revertido pra placeholder script seguro (`class CallbackSystemHandler extends IScriptable {}`)
/// até a causa raiz ser achada. **CAUSA ACHADA E CORRIGIDA 2026-07-18** (`dynarraygrowth-probe`,
/// mesma sessão que fechou GetService): hashmaps embutidos no forge de `CClass` com alocador NULO
/// — fix em `fix_embedded_allocator_vtables`, já aplicado dentro de
/// `register_type_instantiable_with_parent` (chamada abaixo). SEM `extends` na fonte real = parent
/// `IScriptable` implícito (mesma receita da Facade), mas agora **INSTANCIÁVEL** (não
/// `register_type_min`/abstract — `RegisterCallback` precisa devolver uma instância REAL pro
/// encadeamento `.AddTarget(...).SetLifetime(...)` funcionar).
pub unsafe fn register_callbacksystemhandler(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) {
        return "[reg] register_callbacksystemhandler: 'IScriptable' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "CallbackSystemHandler", "gameuiInGameMenuGameController", iscriptable);
    if forged.is_null() {
        return "[reg] register_callbacksystemhandler: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_callbacksystemhandler: classe=OK mas sem protótipo (AddGodMode não resolveu) — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "CallbackSystemHandler", proto, "AddTarget", "AddTarget", tramp_csh_add_target, false);
    let m2 = register_method(reg, "CallbackSystemHandler", proto, "RemoveTarget", "RemoveTarget", tramp_csh_remove_target, false);
    let m3 = register_method(reg, "CallbackSystemHandler", proto, "SetRunMode", "SetRunMode", tramp_csh_set_run_mode, false);
    let m4 = register_method(reg, "CallbackSystemHandler", proto, "SetLifetime", "SetLifetime", tramp_csh_set_lifetime, false);
    let m5 = register_method(reg, "CallbackSystemHandler", proto, "IsRegistered", "IsRegistered", tramp_csh_is_registered, false);
    let m6 = register_method(reg, "CallbackSystemHandler", proto, "Unregister", "Unregister", tramp_csh_unregister, false);
    let m7 = register_method(reg, "CallbackSystemHandler", proto, "HasTarget", "HasTarget", tramp_csh_has_target, false);
    format!(
        "[reg] CallbackSystemHandler: classe=OK (INSTANCIÁVEL, crash de hashmap embutido CORRIGIDO) AddTarget={m1} RemoveTarget={m2} SetRunMode={m3} SetLifetime={m4} IsRegistered={m5} Unregister={m6} HasTarget={m7}"
    )
}

/// `CallbackSystemEvent` — classe abstrata, base real de TODOS os eventos do CallbackSystem
/// (`KeyInputEvent`, etc.). Fonte real: `public abstract native class CallbackSystemEvent {
/// public native func GetEventName() -> CName }`, SEM `extends` explícito = parent `IScriptable`
/// implícito (mesma receita já provada 4x hoje — Facade/ScriptableService/CallbackSystemTarget/
/// BwmsDefaultService). `GetEventName` devolve um CName FIXO por ora (gap residual — não faz
/// parte do `proof_needed` das classes de evento-alvo, que foca em `KeyInputEvent.GetKey/
/// GetAction`).
pub(crate) unsafe extern "C" fn tramp_cse_get_event_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let nome = nome_do_evento_na_cauda(c as *const u8).unwrap_or_else(|| cname("KeyInputEvent"));
    write_uint_ret(out, nome, 8);
}

/// Classe base do evento do BWMS e offset do `eventName` quando a declaração do jogo (2.5) pede o campo; 0 sem ele. Os campos que o jogo declara
/// e o objeto do doador não tem moram depois dos bytes do doador (a "cauda"), do offset até o fim da classe.
static CAUDA_EVENTO_BASE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static CAUDA_EVENTO_INICIO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Campo nativo `nome: tipo` depois do fim de `classe` (`+0x68` cresce junto com o `holderSize` `+0x6C`): a declaração de script do jogo traz
/// `native let` que o objeto do doador não tem, e campo ausente derruba o bind (`Missing native property`, 2.5 Mac `0x1011a8b0c`). `base` marca a
/// classe base dos eventos pra cauda ser zerada na criação e receber o nome no despacho. Devolve o offset.
pub(crate) unsafe fn campo_na_cauda(reg: &Registry, classe: *mut c_void, nome: &str, tipo: &str, base: bool) -> Result<u32, String> {
    let ty = get_type(reg, tipo);
    if !rtti::sane(ty) {
        return Err(format!("tipo '{tipo}' não existe"));
    }
    let offset = (rd_u32(classe, 0x68) + 7) & !7;
    let fim = offset + ((rtti::type_size(ty) + 7) & !7);
    wr_u32(classe, 0x68, fim);
    wr_u32(classe, 0x6C, fim.max(rd_u32(classe, 0x6C)));
    let nome_da_classe = crate::cname::resolve_cname(rtti::type_name_hash(classe));
    propriedade_nativa(reg, &nome_da_classe, nome, tipo, offset)?;
    if base {
        CAUDA_EVENTO_INICIO.store(offset, Ordering::Relaxed);
        CAUDA_EVENTO_BASE.store(classe as usize, Ordering::Release);
    }
    Ok(offset)
}

/// `cls` é a base dos eventos do BWMS ou herda dela.
unsafe fn herda_da_base_de_evento(cls: *const u8) -> bool {
    let base = CAUDA_EVENTO_BASE.load(Ordering::Acquire);
    let mut c = cls;
    for _ in 0..16 {
        if base == 0 || !rtti::sane(c as *mut c_void) {
            return false;
        }
        if c as usize == base {
            return true;
        }
        // O `+0x30` de objeto que não é `IScriptable` não é classe (num `CResource` é o hash do caminho).
        if !crate::memoria::is_readable(c.add(0x10) as *const c_void, 8) {
            return false;
        }
        c = rd_u64(c as *const c_void, 0x10) as *const u8;
    }
    false
}

/// Zera a cauda de um evento recém-criado de `cls`: o construtor do doador não a escreve e o `resource` da cauda é um handle.
pub(crate) unsafe fn zerar_cauda_de_evento(cls: *mut c_void, obj: *mut c_void) {
    let inicio = CAUDA_EVENTO_INICIO.load(Ordering::Relaxed);
    if inicio == 0 || obj.is_null() || !herda_da_base_de_evento(cls as *const u8) {
        return;
    }
    let fim = rd_u32(cls, 0x68);
    if fim > inicio {
        std::ptr::write_bytes((obj as *mut u8).add(inicio as usize), 0, (fim - inicio) as usize);
    }
}

/// Classe de script do objeto, a que o cast de script lê no `+0x30` (2.5 Mac `0x101202c44`); objetos do BWMS nascem pelo `CreateInstance` da
/// classe forjada, que a grava ali.
unsafe fn classe_do_evento(obj: *const u8) -> *const u8 {
    if obj.is_null() || !crate::memoria::is_readable(obj.add(0x30) as *const c_void, 8) {
        return std::ptr::null();
    }
    rd_u64(obj as *const c_void, 0x30) as *const u8
}

/// Grava o `eventName` na cauda do evento despachado (o campo que o jogo declara em `CallbackSystemEvent`).
pub(crate) unsafe fn gravar_nome_do_evento(obj: *mut c_void, nome: u64) {
    let inicio = CAUDA_EVENTO_INICIO.load(Ordering::Relaxed);
    if inicio != 0 && herda_da_base_de_evento(classe_do_evento(obj as *const u8)) {
        ((obj as *mut u8).add(inicio as usize) as *mut u64).write_unaligned(nome);
    }
}

unsafe fn nome_do_evento_na_cauda(obj: *const u8) -> Option<u64> {
    let inicio = CAUDA_EVENTO_INICIO.load(Ordering::Relaxed);
    if inicio == 0 || !herda_da_base_de_evento(classe_do_evento(obj)) {
        return None;
    }
    let nome = (obj.add(inicio as usize) as *const u64).read_unaligned();
    (nome != 0).then_some(nome)
}

/// `GameInstance.GetCallbackSystem(opt gi)` do jogo devolvendo o singleton do BWMS: consome o parâmetro e responde como a global.
pub(crate) unsafe extern "C" fn tramp_get_callback_system_do_jogo(c: *mut c_void, frame: *mut c_void, out: *mut c_void, rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    tramp_get_callback_system(c, frame, out, rt);
}

/// Rota da native `GetCallbackSystem` do `GameInstance` do jogo pro singleton do BWMS. A função do jogo fica no RTTI (o mod continua chamando
/// `GameInstance.GetCallbackSystem()`); o executor despacha pela rota antes da via nativa.
pub(crate) unsafe fn rotear_getter_do_callbacksystem(reg: &Registry) -> String {
    let gi = reg.script_class(cname("GameInstance"));
    match rtti::resolve_in_class(gi, "GetCallbackSystem") {
        Some(f) if f.is_static => {
            crate::npcs::add_route(f.func, tramp_get_callback_system_do_jogo);
            format!("rota em {:p}", f.func)
        }
        Some(_) => "GetCallbackSystem do jogo não é estática".into(),
        None => "GameInstance sem GetCallbackSystem".into(),
    }
}

pub unsafe fn register_callbacksystemevent(reg: &Registry) -> String {
    let forged = register_type_min(reg, "CallbackSystemEvent");
    if forged.is_null() {
        return "[reg] register_callbacksystemevent: forja FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_callbacksystemevent: classe=OK mas sem protótipo — GetEventName NÃO registrado".into(),
    };
    let m1 = register_method(reg, "CallbackSystemEvent", proto, "GetEventName", "GetEventName", tramp_cse_get_event_name, false);
    format!("[reg] CallbackSystemEvent: classe=OK GetEventName={m1}")
}

pub(crate) const TIPO_CLASSE: u8 = 2;

/// Offset de uma propriedade, resolvendo a classe pelo nome no RTTI. `None` = classe ou
/// propriedade ausente nesta build (que é resposta, não erro).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn prop_offset_por_classe(classe: &str, prop: &str) -> Option<usize> {
    let reg = rtti::Registry::obtain()?;
    let cls = reg.class_by_name(classe);
    if !rtti::sane(cls) {
        return None;
    }
    prop_value_offset(cls, prop)
}

/// Grava `v` no campo `nome` do objeto na largura do tipo do campo (1, 2, 4 ou 8 B); `false` se o campo não existe ou a largura não confere.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn campo_inteiro_grava(obj: *mut u8, nome: &str, v: u64) -> bool {
    let prop = rtti::find_property_in_class(rtti::class_of(obj as *mut c_void), nome);
    if !rtti::sane(prop) {
        return false;
    }
    let p = obj.add(rd_u32(prop as *const c_void, 0x20) as usize);
    match rtti::type_size(rd_u64(prop as *const c_void, 0x00) as *mut c_void) {
        1 => p.write(v as u8),
        2 => (p as *mut u16).write_unaligned(v as u16),
        4 => (p as *mut u32).write_unaligned(v as u32),
        8 => (p as *mut u64).write_unaligned(v),
        _ => return false,
    }
    true
}

/// `obj` é da classe `nome` ou herda dela (hash do nome em `CClass+0x18`, subindo por `+0x10`).
pub(crate) unsafe fn acessor_e_da_classe(obj: *const u8, nome: &str) -> bool {
    let alvo = cname(nome);
    let mut cls = rtti::class_of(obj as *mut c_void);
    let mut n = 0;
    while rtti::sane(cls) && n < 32 {
        if rtti::type_name_hash(cls) == alvo {
            return true;
        }
        cls = rd_u64(cls as *const c_void, 0x10) as *mut c_void;
        n += 1;
    }
    false
}

pub unsafe fn registrar_fachada_de_versao(reg: &Registry) -> String {
    register_facade_methods(reg, "Codeware", &[("Version", tramp_version::<PLUGUE_VERSAO>), ("Require", tramp_require::<PLUGUE_VERSAO>)])
}


/// PRODUTIZAÇÃO 2026-08-17 (já ✅ FECHADO no catálogo desde 2026-08-11 —
/// isto NÃO reabre RE nova, só promove a capacidade PROVADA de "atrás de marcador de teste
/// manual" pra "sempre ativa no runtime shipado", mesmo tratamento que `install_pathb_capture()`
/// já recebeu quando virou base da API Facade). Sem isto, `CallbackSystem.RegisterCallback(n
/// "Pipeline/FrameBegin",...)` nunca dispara pra nenhum mod real (o probe que captura o alvo do
/// hook vivia só dentro de `install_postload_hooks()`, atrás de `dev_mode()`). Passthrough
/// incondicional (nunca muda comportamento do jogo) — prova ao vivo decisiva já existe (cont.239-
/// 241 de `HISTORICO.md`, 8+ chamadas vanilla capturadas, zero crash atribuível em toda a
/// investigação). Chamado 1x no `on_load`, idempotente (a macro não reinstala com o original já publicado).
pub(crate) unsafe fn install_pipeline_framebegin_probe() {
    install_hook_probe!(
        UPDATE_REGISTRAR_GROUP_VM, update_registrar_group_probe, ORIG_UPDATE_REGISTRAR_GROUP,
        &format!("[registro-de-update] UpdateRegistrar::RegisterUpdate(group) probe @ {UPDATE_REGISTRAR_GROUP_VM:#012x} (sempre-ativo)"),
        "[registro-de-update] FALHA UpdateRegistrar probe"
    );
}


/// `CClass` dona do vptr `vt` (runtime), pelo `GetType` decodificado; só leitura de memória.
pub(crate) unsafe fn classe_do_vptr(vt: u64) -> Option<u64> {
    if vt < 0x1_0000_0000 || vt & 7 != 0 || !crate::memoria::is_readable(vt as *const c_void, 0x10) {
        return None;
    }
    let f = ((vt + 8) as *const u64).read_unaligned();
    if f < 0x1_0000_0000 || f & 3 != 0 || !crate::memoria::is_readable(f as *const c_void, 12) {
        return None;
    }
    let ler = |k: u64| ((f + k) as *const u32).read_unaligned();
    let global = global_do_gettype(f, ler(0), ler(4), ler(8))?;
    if !crate::memoria::is_readable(global as *const c_void, 8) {
        return None;
    }
    let cls = (global as *const u64).read_unaligned();
    if cls < 0x1_0000_0000 || cls & 7 != 0 || !crate::memoria::is_readable(cls as *const c_void, 0x20) {
        return None;
    }
    Some(cls)
}

#[cfg(test)]
mod testes {
    use super::{
        atualizar_eventos_de_entidade, desativar_hooks_sem_ouvinte, desregistrar_handler, evento_mapeado, fire_event_args_by_hash, grupo_do_evento,
        handler_registrado, has_listener_for, itens_do_disparo, ouvinte_vivo, registrar_handler, registrar_pela_via_global, tirar_ouvinte_do_evento,
        varrer_handlers_da_sessao, HandlerState, CALLBACKS, EVENTOS_DO_PIPELINE, GRUPOS_DE_EVENTO, HANDLER_STATES, OUVINTES, PIPELINE_ARMADOS,
        STATIC_CALLBACKS, TESTES_DE_CALLBACKS_EM_SERIE,
    };
    use crate::cname::cname;
    use crate::diagnostico::eventos_ativos;
    use crate::entidades::{ENTIDADE_EVENTOS, ENTIDADE_EVENTOS_ARMADOS};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::PoisonError;

    // Os alvos são só chaves de `OUVINTES`, nunca lidos; cada teste usa os seus.

    #[test]
    fn ouvinte_fora_da_lista_ou_sem_bloco_esta_morto() {
        let (ausente, sem_bloco) = (0x7e57_0001_0000usize, 0x7e57_0002_0000usize);
        OUVINTES.lock().expect("ouvintes").push((sem_bloco, 0, 1));
        let vivos = unsafe { (ouvinte_vivo(ausente), ouvinte_vivo(sem_bloco)) };
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != sem_bloco);
        assert_eq!(vivos, (false, false));
    }

    #[test]
    fn ouvinte_com_bloco_segue_o_forte() {
        let alvo = 0x7e57_0003_0000usize;
        let bloco = [AtomicU32::new(1), AtomicU32::new(1)];
        OUVINTES.lock().expect("ouvintes").push((alvo, bloco.as_ptr() as u64, 1));
        let com_forte = unsafe { ouvinte_vivo(alvo) };
        bloco[0].store(0, Ordering::SeqCst);
        let sem_forte = unsafe { ouvinte_vivo(alvo) };
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != alvo);
        assert_eq!((com_forte, sem_forte), (true, false));
    }

    // `ativ:N1`: a global entra pela mesma porta da classe. `Bwms/TesteAtivos` e `Resource/PostLoad` não têm hook no BWMS, então o
    // `ativar_hooks_do_evento` volta sem tocar o binário do jogo. O ouvinte já está em `OUVINTES` com bloco 0: o `ouvinte_segura` só soma, sem o
    // construtor de handle do motor, que offline não existe.
    #[test]
    fn registro_pela_via_global_mapeia_o_nome_e_ativa_o_evento() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let (alvo, f, ev) = (0x7e57_0004_0000usize, cname("OnBwmsTesteGlobal"), cname("Bwms/TesteAtivos"));
        OUVINTES.lock().expect("ouvintes").push((alvo, 0, 1));
        let base = eventos_ativos();
        let ativos = unsafe { registrar_pela_via_global(ev, alvo as u64, f) };
        let eventos = eventos_ativos();
        let alias = unsafe { registrar_pela_via_global(cname("Resource/Ready"), alvo as u64, f) };
        let pelo_nome = (has_listener_for("Resource/PostLoad"), has_listener_for("Resource/Ready"));
        let recusas = unsafe {
            [registrar_pela_via_global(0, alvo as u64, f), registrar_pela_via_global(ev, 0, f), registrar_pela_via_global(ev, alvo as u64, 0)]
        };
        let handlers: Vec<usize> = CALLBACKS.lock().expect("callbacks").iter().filter(|e| e.1 == alvo).map(|e| e.3).collect();
        let entradas_do_ouvinte = OUVINTES.lock().expect("ouvintes").iter().find(|e| e.0 == alvo).map(|e| e.2);
        CALLBACKS.lock().expect("callbacks").retain(|e| e.1 != alvo);
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != alvo);
        assert_eq!(ativos, Some(ev));
        assert_eq!(eventos, base + 1, "o evento registrado pela global conta como ativo");
        assert_eq!(alias, Some(cname("Resource/PostLoad")), "o alias é guardado com o nome que o despacho usa");
        assert_eq!(pelo_nome, (true, false), "has_listener_for acha o nome do despacho, não o alias");
        assert_eq!(recusas, [None, None, None], "evento, ouvinte ou função zerados não entram");
        assert_eq!(handlers, [0, 0], "a global não cria handler");
        assert_eq!(entradas_do_ouvinte, Some(3), "cada registro aceito soma 1 no ouvinte");
    }

    // `ativ:N6`: uma entrada sem handler (a global) e um handler `RunMode.Once` com alvo, os dois com a mesma (evento, ouvinte, função). O despacho
    // lê o estado por `itens_do_disparo` e sai do `Once` por `desregistrar_handler`: a entrada sem handler não pega o alvo nem o `Once` do outro, e
    // fica.
    #[test]
    fn entrada_sem_handler_nao_herda_o_estado_de_outro_handler() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let (alvo, handler, alvo_do_handler) = (0x7e57_0005_0000usize, 0x7e57_0006_0000usize, 0x7e57_0007_0000usize);
        let (ev, f) = (cname("Bwms/TesteOnce"), cname("OnBwmsTesteOnce"));
        OUVINTES.lock().expect("ouvintes").push((alvo, 0, 2));
        CALLBACKS.lock().expect("callbacks").extend([(ev, alvo, f, 0), (ev, alvo, f, handler)]);
        let st = HandlerState { targets: vec![alvo_do_handler], run_mode: 1, lifetime: 0 };
        HANDLER_STATES.lock().expect("handlers").push((handler, st));
        let lidos = itens_do_disparo(ev);
        let tiradas = unsafe { desregistrar_handler(handler) };
        let ficaram: Vec<usize> = CALLBACKS.lock().expect("callbacks").iter().filter(|e| e.1 == alvo).map(|e| e.3).collect();
        let entradas_do_ouvinte = OUVINTES.lock().expect("ouvintes").iter().find(|e| e.0 == alvo).map(|e| e.2);
        let registrado = handler_registrado(handler);
        CALLBACKS.lock().expect("callbacks").retain(|e| e.1 != alvo);
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != alvo);
        HANDLER_STATES.lock().expect("handlers").retain(|e| e.0 != handler);
        assert_eq!(
            lidos,
            Some(vec![(alvo, f, 0, vec![], 0), (alvo, f, handler, vec![alvo_do_handler], 1)]),
            "a entrada sem handler vai sem alvo e sem Once; a do handler leva o Once e o alvo"
        );
        assert_eq!(tiradas, 1);
        assert_eq!(ficaram, [0], "a saída do Once leva só a entrada do handler");
        assert_eq!(entradas_do_ouvinte, Some(1), "a entrada que ficou segura o ouvinte");
        assert!(!registrado, "o handler do Once deixa de estar registrado");
    }

    // `ativ:N6` (F5-08): as 4 saídas do handler tiram o estado dele de `HANDLER_STATES`, e a tabela volta ao tamanho da base, sem os registros. Os
    // ouvintes já estão em `OUVINTES` com bloco 0, como em `registro_pela_via_global…`. `Bwms/TesteHandlers` não tem grupo de hooks, então o
    // registro não toca o binário do jogo; os hooks sem ouvinte ficam com os trampolins, que este teste não chama.
    #[test]
    fn as_quatro_saidas_tiram_o_handler_da_tabela() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let (alvo, alvo2) = (0x7e57_0008_0000usize, 0x7e57_0009_0000usize);
        let ev = cname("Bwms/TesteHandlers");
        let h: Vec<usize> = (0..8).map(|i| 0x7e57_000a_0000usize + i * 0x10).collect();
        let f = |i: usize| cname("OnBwmsTesteHandlers").wrapping_add(i as u64);
        OUVINTES.lock().expect("ouvintes").extend([(alvo, 0, 1), (alvo2, 0, 1)]);
        let base = HANDLER_STATES.lock().expect("handlers").len();
        let alvo_de = |i: usize| if i == 2 || i == 3 { alvo2 } else { alvo };
        let aceitos: Vec<Option<u64>> = h.iter().enumerate().map(|(i, &hi)| unsafe { registrar_handler(ev, alvo_de(i), f(i), hi) }).collect();
        let registrados_no_inicio: Vec<bool> = h.iter().map(|&hi| handler_registrado(hi)).collect();
        // `SetLifetime(Forever)` em h6 e h7: o corpo do trampolim só grava o campo.
        for (p, st) in HANDLER_STATES.lock().expect("handlers").iter_mut() {
            if *p == h[6] || *p == h[7] {
                st.lifetime = 1;
            }
        }
        // `Unregister()` (e `RunMode.Once`, a mesma saída).
        let unregister = unsafe { desregistrar_handler(h[0]) };
        let unregister_de_novo = unsafe { desregistrar_handler(h[0]) };
        let unregister_h1 = unsafe { desregistrar_handler(h[1]) };
        // `UnregisterCallback(evento, ouvinte)` sem função: h2 e h3, do outro ouvinte.
        let por_ouvinte = unsafe { tirar_ouvinte_do_evento(ev, alvo2 as u64, 0) };
        // `Session/End`: h4 e h5 saem, h6 e h7 (`Forever`) ficam.
        let varridos = unsafe { varrer_handlers_da_sessao() };
        let depois_da_sessao: Vec<bool> = h.iter().map(|&hi| handler_registrado(hi)).collect();
        // `UnregisterCallback(evento, ouvinte, função)`: só h7.
        let por_funcao = unsafe { tirar_ouvinte_do_evento(ev, alvo as u64, f(7)) };
        let h6_ficou = handler_registrado(h[6]);
        let unregister_h6 = unsafe { desregistrar_handler(h[6]) };
        let tamanho = HANDLER_STATES.lock().expect("handlers").len();
        let nossos = HANDLER_STATES.lock().expect("handlers").iter().filter(|e| h.contains(&e.0)).count();
        let entradas = CALLBACKS.lock().expect("callbacks").iter().filter(|e| e.0 == ev).count();
        let ouvintes: Vec<Option<u32>> =
            [alvo, alvo2].iter().map(|a| OUVINTES.lock().expect("ouvintes").iter().find(|e| e.0 == *a).map(|e| e.2)).collect();
        CALLBACKS.lock().expect("callbacks").retain(|e| e.0 != ev);
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != alvo && e.0 != alvo2);
        HANDLER_STATES.lock().expect("handlers").retain(|e| !h.contains(&e.0));
        assert_eq!(aceitos, vec![Some(ev); 8]);
        assert_eq!(registrados_no_inicio, vec![true; 8]);
        assert_eq!((unregister, unregister_de_novo, unregister_h1), (1, 0, 1), "o 2º Unregister do mesmo handler não acha nada");
        assert_eq!(por_ouvinte, 2);
        assert_eq!(varridos, 2, "o Session/End leva só os handlers que não são Forever");
        assert_eq!(depois_da_sessao, [false, false, false, false, false, false, true, true]);
        assert_eq!((por_funcao, h6_ficou, unregister_h6), (1, true, 1), "a remoção por função leva só o handler dela");
        assert_eq!((tamanho, nossos), (base, 0), "a tabela volta ao tamanho da base, sem os registros");
        assert_eq!(entradas, 0);
        assert_eq!(ouvintes, [Some(1), Some(1)], "cada entrada que saiu soltou o seu ouvinte");
    }

    // `ativ:N6` (F5-08): a cópia do despacho pega os alvos e o `RunMode` de cada handler; entrada de handler sem estado (outra thread o tirou entre a
    // trava de `CALLBACKS` e a de `HANDLER_STATES`) fica de fora, e a cópia não muda com o estado depois dela.
    #[test]
    fn copia_do_despacho_leva_alvos_e_modo_e_deixa_handler_sem_estado() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let (alvo, com_estado, sem_estado, de_outro_evento, alvo_do_handler) =
            (0x7e57_000b_0000usize, 0x7e57_000c_0000usize, 0x7e57_000d_0000usize, 0x7e57_000e_0000usize, 0x7e57_000f_0000usize);
        let (ev, outro, f) = (cname("Bwms/TesteCopia"), cname("Bwms/TesteCopiaOutro"), cname("OnBwmsTesteCopia"));
        CALLBACKS.lock().expect("callbacks").extend([
            (ev, alvo, f, 0),
            (ev, alvo, f, com_estado),
            (ev, alvo, f.wrapping_add(1), sem_estado),
            (outro, alvo, f, de_outro_evento),
        ]);
        HANDLER_STATES.lock().expect("handlers").extend([
            (com_estado, HandlerState { targets: vec![alvo_do_handler], run_mode: 1, lifetime: 0 }),
            (de_outro_evento, HandlerState { targets: vec![], run_mode: 0, lifetime: 0 }),
        ]);
        let copia = itens_do_disparo(ev);
        for (p, st) in HANDLER_STATES.lock().expect("handlers").iter_mut() {
            if *p == com_estado {
                st.targets.clear();
                st.run_mode = 0;
            }
        }
        let sem_ouvinte = itens_do_disparo(cname("Bwms/TesteCopiaVazio"));
        CALLBACKS.lock().expect("callbacks").retain(|e| e.1 != alvo);
        HANDLER_STATES.lock().expect("handlers").retain(|e| e.0 != com_estado && e.0 != de_outro_evento);
        assert_eq!(copia, Some(vec![(alvo, f, 0, vec![], 0), (alvo, f, com_estado, vec![alvo_do_handler], 1)]));
        assert_eq!(sem_ouvinte, Some(vec![]));
    }

    // `ativ:A3a`: o registro acha o grupo pelo nome (`grupo_do_evento`) e a remoção confere os ouvintes pelos nomes da mesma linha, então cada evento da
    // tabela é tirado pelo grupo que o instalou. Um nome em 2 linhas só subiria a 1ª no registro; um alias nunca acharia linha, porque o registro guarda
    // o nome mapeado.
    #[test]
    fn cada_evento_da_tabela_e_tirado_pelo_grupo_que_o_instalou() {
        for (k, (eventos, _, desativar, _)) in GRUPOS_DE_EVENTO.iter().enumerate() {
            assert!(!eventos.is_empty(), "grupo {k} sem evento");
            for nome in eventos.iter() {
                assert_eq!(grupo_do_evento(cname(nome)), Some(k), "{nome}");
                assert_eq!(evento_mapeado(cname(nome)), cname(nome), "{nome} é alias");
            }
            assert_eq!(
                desativar.is_none(),
                *eventos == ["Entity/Extract"] || *eventos == ["InkWidget/Spawn"],
                "só o hook do job do builder (patch de recursos) e os do spawn de widget (Item:Controller) ficam sem remoção"
            );
        }
        assert_eq!(grupo_do_evento(cname("Bwms/TesteAtivos")), None, "evento sem hook não acha grupo");
    }

    // Instalar e tirar pedem o binário do jogo, então a ordem dos atômicos em volta da trava sai do texto: o registro arma depois de instalar, e a
    // remoção desarma antes de pegar a trava e tirar.
    #[test]
    fn registro_arma_depois_de_instalar_e_remocao_desarma_antes_de_tirar() {
        let fonte = include_str!("mod.rs");
        let corpo = |assinatura: &str| -> &'static str {
            let resto = &fonte[fonte.find(assinatura).unwrap_or_else(|| panic!("`{assinatura}` sumiu"))..];
            &resto[..resto.find("\n}\n").expect("fim da função")]
        };
        let posicao = |texto: &str, trecho: &str| texto.find(trecho).unwrap_or_else(|| panic!("`{trecho}` sumiu"));
        let ativar = corpo("pub(crate) fn ativar_hooks_do_evento(");
        assert!(posicao(ativar, "unsafe { garantir() };") < posicao(ativar, "atualizar_eventos_de_entidade();"), "o registro arma antes de instalar");
        let desativar = corpo("pub(crate) fn desativar_hooks_sem_ouvinte(");
        let trava = posicao(desativar, "let _grupo = HOOKS_DE_EVENTO.lock();");
        assert!(posicao(desativar, "atualizar_eventos_de_entidade();") < trava, "a remoção desarma depois de pegar a trava");
    }

    // As 2 portas da tabela também saem do texto, pelo mesmo motivo: o registro instala a linha que `grupo_do_evento` achou, e a remoção tira só o que
    // `grupos_sem_ouvinte` devolve. Os testes das funções puras não pegam a porta que instala a linha 0 nem a que tira todo grupo com remoção.
    #[test]
    fn registro_instala_a_linha_achada_e_remocao_tira_so_grupo_sem_ouvinte() {
        let fonte = include_str!("mod.rs");
        let corpo = |assinatura: &str| -> &'static str {
            let resto = &fonte[fonte.find(assinatura).unwrap_or_else(|| panic!("`{assinatura}` sumiu"))..];
            &resto[..resto.find("\n}\n").expect("fim da função")]
        };
        let ativar = corpo("pub(crate) fn ativar_hooks_do_evento(");
        assert!(ativar.contains("if let Some(k) = grupo_do_evento(ev) {"), "o registro não procura a linha do evento");
        assert!(ativar.contains("let (_, garantir, _, _) = GRUPOS_DE_EVENTO[k];"), "o registro instala uma linha que não é a achada");
        assert_eq!(ativar.matches("GRUPOS_DE_EVENTO").count(), 1, "o registro lê outra linha da tabela");
        let desativar = corpo("pub(crate) fn desativar_hooks_sem_ouvinte(");
        let laco = "    for (_, desativar) in grupos_sem_ouvinte() {\n        unsafe { desativar() };\n    }";
        assert!(desativar.contains(laco), "a remoção tira grupo que ainda tem ouvinte");
        assert!(!desativar.contains("GRUPOS_DE_EVENTO"), "a remoção lê a tabela sem passar por grupos_sem_ouvinte");
    }

    // Com um ouvinte só, a remoção (`grupos_sem_ouvinte`) tira todo grupo com remoção menos o do evento, e o atômico armado é o da linha do evento e
    // nenhum outro; os `Entity/*` armam o bit do evento em `ENTIDADE_EVENTOS_ARMADOS`. Sem o ouvinte, todo grupo com remoção cai e nada fica armado.
    #[test]
    fn ouvinte_segura_e_arma_so_o_proprio_grupo() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let alvo = 0x7e57_0008_0000usize;
        let lidos = || {
            let atomicos = GRUPOS_DE_EVENTO.map(|(_, _, _, armado)| armado.map(|a| a.load(Ordering::Relaxed)));
            (super::grupos_sem_ouvinte().map(|(k, _)| k).collect::<Vec<_>>(), atomicos, ENTIDADE_EVENTOS_ARMADOS.load(Ordering::Relaxed))
        };
        let com_remocao: Vec<usize> = (0..GRUPOS_DE_EVENTO.len()).filter(|&k| GRUPOS_DE_EVENTO[k].2.is_some()).collect();
        let desarmados = GRUPOS_DE_EVENTO.map(|(_, _, _, armado)| armado.map(|_| false));
        for (k, (eventos, _, _, armado)) in GRUPOS_DE_EVENTO.iter().enumerate() {
            for nome in eventos.iter() {
                CALLBACKS.lock().expect("callbacks").push((cname(nome), alvo, 1, 0));
                atualizar_eventos_de_entidade();
                let com_ouvinte = lidos();
                CALLBACKS.lock().expect("callbacks").retain(|e| e.1 != alvo);
                atualizar_eventos_de_entidade();
                let tirados: Vec<usize> = com_remocao.iter().copied().filter(|&j| j != k).collect();
                let mut atomicos = desarmados;
                atomicos[k] = armado.map(|_| true);
                let bit = ENTIDADE_EVENTOS.iter().position(|e| e == nome).map_or(0, |b| 1u32 << b);
                assert_eq!(com_ouvinte, (tirados, atomicos, bit), "{nome}");
                assert_eq!(lidos(), (com_remocao.clone(), desarmados, 0), "{nome} sem ouvinte");
            }
        }
    }

    // `perfil:PT-08`: os ganchos dos estágios leem só `PIPELINE_ARMADOS`, então o bit segue a tabela por toda porta: registro pela global, de handler
    // e estático, `UnregisterCallback`, `UnregisterStaticCallback` e a varredura do `Session/End`. As remoções passam pelo `desativar_hooks_sem_ouvinte`
    // de produção; um ouvinte global no 1º evento de cada grupo com remoção segura os grupos, e nenhum hook sai (offline, o `rebase` da remoção
    // varreria o binário de teste atrás das assinaturas).
    #[test]
    fn bit_do_pipeline_segue_registro_remocao_e_session_end() {
        let _serie = TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(PoisonError::into_inner);
        let (seguro, alvo, f) = (0x7e57_0010_0000usize, 0x7e57_0011_0000usize, cname("OnBwmsTestePipeline"));
        let grupos: Vec<u64> = GRUPOS_DE_EVENTO.iter().filter(|g| g.2.is_some()).map(|g| cname(g.0[0])).collect();
        CALLBACKS.lock().expect("callbacks").extend(grupos.iter().map(|&ev| (ev, seguro, 1, 0)));
        OUVINTES.lock().expect("ouvintes").push((alvo, 0, 1));
        let bits = || PIPELINE_ARMADOS.load(Ordering::Relaxed);
        let mut lidos = Vec::new();
        for (k, nome) in EVENTOS_DO_PIPELINE.iter().enumerate() {
            let ev = cname(nome);
            let global = unsafe { registrar_pela_via_global(ev, alvo as u64, f) }.map(|_| bits());
            let tiradas = unsafe { tirar_ouvinte_do_evento(ev, alvo as u64, 0) };
            desativar_hooks_sem_ouvinte();
            let sem_global = bits();
            let estatico = crate::diagnostico::registrar_callback_estatico(nome, "TestePipeline", "OnEvento").then(bits);
            STATIC_CALLBACKS.lock().expect("estáticos").retain(|(e, _, _)| *e != ev);
            desativar_hooks_sem_ouvinte();
            let sem_estatico = bits();
            // `lifetime` 0 (`Session`): sai na varredura.
            let handler = 0x7e57_0012_0000usize + k * 0x10;
            let de_handler = unsafe { registrar_handler(ev, alvo, f, handler) }.map(|_| bits());
            let entregues = unsafe { fire_event_args_by_hash(cname("Session/End"), "Session/End", &[]) };
            let depois_do_session_end = (bits(), handler_registrado(handler));
            lidos.push((global, tiradas, sem_global, estatico, sem_estatico, de_handler, entregues, depois_do_session_end));
        }
        CALLBACKS.lock().expect("callbacks").retain(|e| e.1 != seguro && e.1 != alvo);
        OUVINTES.lock().expect("ouvintes").retain(|e| e.0 != alvo);
        atualizar_eventos_de_entidade();
        let limpo = bits();
        for (k, lido) in lidos.into_iter().enumerate() {
            let bit = 1u8 << k;
            assert_eq!(lido, (Some(bit), 1, 0, Some(bit), 0, Some(bit), 0, (0, false)), "{}", EVENTOS_DO_PIPELINE[k]);
        }
        assert_eq!(limpo, 0);
    }
}
