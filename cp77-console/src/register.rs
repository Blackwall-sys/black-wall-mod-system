//! Registro de FUNÇÃO/TIPO NATIVO no RTTI do jogo — o pré-requisito do Codeware.
//!
//! Descoberta-chave (workflow codeware-100-map): **registrar é o inverso de
//! resolver**. Nosso `rtti::resolve_in_class` já LÊ `funcs@CClass+0x48` /
//! `staticFuncs@+0x58`; registrar um método = dar PushBack nesses mesmos
//! `DynArray`. Registrar um global = chamar `CRTTISystem::RegisterFunction`
//! (vtbl+0xA0). O objeto-função (`CGlobalFunction`/`CClassFunction`) é um POD que
//! a gente constrói à mão, clonando a **vtable** de uma função nativa existente.
//!
//! ÚNICO desconhecido de RE (resolvido in-game pelo `probe`): o **offset do
//! handler** dentro do objeto-função — onde o executor lê o `ScriptingFunction_t`
//! (assinatura `extern "C" fn(ctx, frame, out, retType)`). O `probe` despeja o
//! layout de uma função nativa real e marca os ponteiros-de-código candidatos.
//!
//! Estado: ESCRITO + COMPILA offline. Comportamento valida-se no jogo (probe →
//! fixar HANDLER_OFFSET → smoke-test). Slots de vtbl do CRTTISystem (workflow):
//!   +0x30 GetFunction · +0x80 RegisterType · +0xA0 RegisterFunction
//!   +0xC8 AddPostRegisterCallback · +0x100 RegisterScriptName

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::cname::cname;
use crate::rtti::{self, Registry};

/// (HISTÓRICO) Tentativa antiga: gravar o handler NO objeto-função em 0xB0. O `cwprobe`
/// PROVOU in-game que 0xB0 = parent/regIndex e que o engine pega o handler de uma TABELA
/// GLOBAL por regIndex — então escrever ali nunca dispararia (e corromperia o regIndex).
/// Substituído pelo routing-hook (`route_native` + `exec_replacement`). Mantido só como
/// nota do layout decifrado.
#[allow(dead_code)]
pub const HANDLER_OFFSET: usize = 0xB0;

/// Flags de CBaseFunction (RED4ext): bit0=isNative, bit2=isStatic (palpite —
/// confirmar no probe lendo flags de uma static native conhecida).
const FLAG_NATIVE: u32 = 1 << 0;
const FLAG_STATIC: u32 = 1 << 2;

const FUNC_POD_SIZE: usize = 0xC0;

/// Assinatura que o executor RED chama para uma função nativa.
pub type NativeHandler = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i64);

/// ROTEAMENTO de nativas registradas: ponteiro do POD da func -> handler Rust.
///
/// O despacho NÃO usa mais o `handler@0xB0` do objeto (slot ERRADO: 0xB0=parent/
/// regIndex; o engine pega o handler de uma tabela GLOBAL por regIndex). Em vez de
/// achar essa tabela, o executor (`selfboot::exec_replacement`, hook que já temos)
/// consulta ESTE mapa ANTES de cair na via nativa do jogo: se `func` é nossa, chama
/// o handler direto e retorna. Escrito 1x no register e lido no executor — AMBOS na
/// thread do jogo (o register roda via cp77_tick, que é chamado de dentro do hook) →
/// sem concorrência real; `try_lock` + fast-path atômico mantêm o hot-path barato.
static NATIVE_ROUTES: Mutex<Vec<(usize, NativeHandler)>> = Mutex::new(Vec::new());
static NATIVE_ROUTE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// `func` da native EM EXECUÇÃO, publicado pelo executor (exec_replacement) ANTES de chamar o
/// handler → o handler lê seus args do frame via `read_params_consuming(func, frame)`. Game-thread
/// (sem corrida real); re-entrância (native→native) sobrescreve, mas nossos handlers são leaf.
static CURRENT_NATIVE_FUNC: std::sync::atomic::AtomicPtr<c_void> =
    std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
pub fn set_current_native_func(f: *mut c_void) {
    CURRENT_NATIVE_FUNC.store(f, Ordering::Relaxed);
}
pub fn current_native_func() -> *mut c_void {
    CURRENT_NATIVE_FUNC.load(Ordering::Relaxed)
}

/// Registra `func -> handler` (chamado quando o POD é construído).
unsafe fn add_route(func: *mut c_void, handler: NativeHandler) {
    if let Ok(mut v) = NATIVE_ROUTES.lock() {
        v.push((func as usize, handler));
        NATIVE_ROUTE_COUNT.store(v.len(), Ordering::Relaxed);
    }
}

/// O executor consulta isto a CADA chamada. Fast-path: 0 nativas registradas = 1
/// load atômico e sai (custo desprezível). Se `func` é nossa, devolve o handler.
pub unsafe fn route_native(func: *mut c_void) -> Option<NativeHandler> {
    if NATIVE_ROUTE_COUNT.load(Ordering::Relaxed) == 0 || func.is_null() {
        return None;
    }
    let v = NATIVE_ROUTES.try_lock().ok()?;
    let f = func as usize;
    v.iter().find(|(p, _)| *p == f).map(|(_, h)| *h)
}

#[inline]
unsafe fn wr_u64(base: *mut c_void, off: usize, v: u64) {
    core::ptr::write_unaligned((base as *mut u8).add(off) as *mut u64, v);
}
#[inline]
unsafe fn wr_u32(base: *mut c_void, off: usize, v: u32) {
    core::ptr::write_unaligned((base as *mut u8).add(off) as *mut u32, v);
}
#[inline]
unsafe fn rd_u64(base: *const c_void, off: usize) -> u64 {
    core::ptr::read_unaligned((base as *const u8).add(off) as *const u64)
}

/// `CRTTISystem::GetFunction(CName)` (vtbl+0x30) — acha um GLOBAL existente. Usado
/// p/ (a) clonar a vtable de CGlobalFunction e (b) confirmar que nosso registro
/// entrou (re-resolve por nome).
pub unsafe fn get_function(reg: &Registry, name: &str) -> *mut c_void {
    let slot = reg.vtbl_slot(0x30);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    f(reg.raw(), cname(name))
}

/// `CRTTISystem::RegisterFunction(CGlobalFunction*)` (vtbl+0xA0).
unsafe fn call_register_function(reg: &Registry, func: *mut c_void) -> bool {
    let slot = reg.vtbl_slot(0xA0);
    if !rtti::sane(slot) {
        crate::log("[reg] RegisterFunction (vtbl+0xA0) ilegível");
        return false;
    }
    let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(slot);
    f(reg.raw(), func);
    true
}

/// Constrói um objeto-função nativo (POD) à mão: clona a `vtable` de `proto`
/// (uma função nativa existente do mesmo tipo) e preenche os campos conhecidos.
/// Devolve o ponteiro ou null.
unsafe fn build_native_func(
    proto: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
    is_static: bool,
) -> *mut c_void {
    if !rtti::sane(proto) {
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(proto as *const c_void, 0x00);
    if vtable == 0 {
        return std::ptr::null_mut();
    }
    let mem = rtti::pool_alloc(FUNC_POD_SIZE, 8);
    if mem.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, FUNC_POD_SIZE);
    wr_u64(mem, 0x00, vtable); // vtable clonada
    wr_u64(mem, 0x08, cname(full)); // fullName (CName)
    wr_u64(mem, 0x10, cname(short)); // shortName (CName)
    let mut flags = FLAG_NATIVE;
    if is_static {
        flags |= FLAG_STATIC;
    }
    wr_u32(mem, 0xA8, flags); // flags
    // O handler NÃO vai no objeto: 0xB0 é parent/regIndex no layout real (cwprobe), e
    // escrever um ponteiro de 64 bits lá corromperia o regIndex (se o engine algum dia
    // lesse). Deixamos 0xB0 zerado e roteamos func->handler pelo executor (add_route).
    add_route(mem, handler);
    crate::log(&format!(
        "[reg] build {full} -> {mem:p} (vtable={vtable:#x}, rota func->handler registrada)"
    ));
    mem
}

/// Registra um GLOBAL nativo. Clona a vtable de `proto_global` (ex.: GetFunction
/// de uma global conhecida). Retorna sucesso.
pub unsafe fn register_global(
    reg: &Registry,
    proto_global: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let func = build_native_func(proto_global, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    if !call_register_function(reg, func) {
        return false;
    }
    // Confirma: re-resolve por nome.
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log(&format!("[reg] register_global {full}: re-resolve -> {} ({back:p})", if ok { "OK" } else { "FALHOU" }));
    ok
}

/// PushBack num `red::DynArray<T*>` (T* = 8 bytes): entries(ptr)@0x00, capacity@0x08(u32),
/// size@0x0C(u32). Se cabe (`size < cap`) escreve in-place. Senão **REALOCA** no MESMO pool do
/// jogo (`PoolDefault`), copia os `size` entries existentes, faz append, e republica
/// entries/capacity/size. Devolve o índice do slot novo (ou `None` se ilegível/alloc-falhou).
/// SEGURO: o buffer novo vem do allocator do jogo → o engine libera certo no teardown; o buffer
/// antigo vaza (pequeno, aceitável — `build_cname_dynarray` documenta o mesmo trade-off). Roda na
/// thread do jogo (register via cp77_tick dentro do hook) → sem corrida real.
unsafe fn dynarray_push_ptr(arr: *mut c_void, val: u64) -> Option<usize> {
    if !crate::gum::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    let entries = rd_u64(arr as *const c_void, 0x00) as *mut u8;
    let cap = core::ptr::read_unaligned((arr as *const u8).add(0x08) as *const u32);
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if !entries.is_null() && size < cap {
        core::ptr::write_unaligned(entries.add(size as usize * 8) as *mut u64, val);
        core::ptr::write_unaligned((arr as *mut u8).add(0x0C) as *mut u32, size + 1);
        return Some(size as usize);
    }
    // Cheio (ou sem buffer): realoca. Novo cap = max(cap*2, size+4, 4).
    let new_cap = cap.saturating_mul(2).max(size + 4).max(4);
    let new_buf = rtti::pool_alloc(new_cap as usize * 8, 8) as *mut u8;
    if new_buf.is_null() {
        return None;
    }
    if !entries.is_null() && size > 0 {
        core::ptr::copy_nonoverlapping(entries, new_buf, size as usize * 8);
    }
    core::ptr::write_unaligned(new_buf.add(size as usize * 8) as *mut u64, val);
    // Republica: entries -> cap -> size (size por último p/ o engine nunca ver um size > buffer).
    core::ptr::write_unaligned((arr as *mut u8).add(0x00) as *mut u64, new_buf as u64);
    core::ptr::write_unaligned((arr as *mut u8).add(0x08) as *mut u32, new_cap);
    core::ptr::write_unaligned((arr as *mut u8).add(0x0C) as *mut u32, size + 1);
    crate::log(&format!(
        "[reg] dynarray realocou: cap {cap}->{new_cap} entries {entries:p}->{new_buf:p} size {size}->{}",
        size + 1
    ));
    Some(size as usize)
}

/// Registra um MÉTODO (instância/estático) numa CClass existente: PushBack no
/// `DynArray` `funcs@cls+0x48` (instância) ou `staticFuncs@cls+0x58` (estático).
/// V2: REALOCA se o array estiver cheio (via `dynarray_push_ptr`) — não falha mais
/// por falta de capacidade. É o caminho que EquipmentEX/Cyberware-EX exigem (vários métodos).
pub unsafe fn register_method(
    reg: &Registry,
    class: &str,
    proto_method: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
    is_static: bool,
) -> bool {
    let cls = reg.class_by_name(class);
    if !rtti::sane(cls) {
        crate::log(&format!("[reg] classe '{class}' não existe"));
        return false;
    }
    let func = build_native_func(proto_method, full, short, handler, is_static);
    if func.is_null() {
        return false;
    }
    let arr = (cls as *mut u8).add(if is_static { 0x58 } else { 0x48 }) as *mut c_void;
    match dynarray_push_ptr(arr, func as u64) {
        Some(slot) => {
            crate::log(&format!("[reg] register_method {class}.{short} -> slot {slot}"));
            true
        }
        None => {
            crate::log(&format!("[reg] register_method {class}.{short} FALHOU (array ilegível ou alloc null)"));
            false
        }
    }
}

// ===== Trampolins de smoke-test =====================================================

/// `BlackwallPing() -> Bool` — escreve `true` no retorno. Prova que o registro
/// entrou no RTTI e o executor chama nosso handler.
unsafe extern "C" fn tramp_ping(_ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[reg] >>> BlackwallPing chamado (handler nativo rodou!)");
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8); // Bool = true
    }
}

/// `BwmsAutoContinue() -> Bool` (DEV) — retorna true se `~/.bwms-autocontinue` existe.
/// SEGUNDA native real registrada via `register_all` — prova que o registro escala
/// além do smoke (2 funções no RTTI) E que o RETORNO Bool native→redscript é CONSUMIDO
/// numa condicional (o `.reds` de auto-continue gateia o LoadLastCheckpoint nisto).
/// Toggle por marcador SEM recompilar: `touch`/`rm ~/.bwms-autocontinue`.
unsafe extern "C" fn tramp_autocontinue(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-autocontinue").exists())
        .unwrap_or(false);
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, on as u8);
    }
}

/// `Codeware.Version() -> String` (smoke do Facade).
unsafe extern "C" fn tramp_version(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[reg] >>> Codeware.Version chamado");
    if !out.is_null() {
        std::ptr::write_bytes(out as *mut u8, 0, 0x20);
        rtti::red_string_write_inline(out as *mut u8, "1.0.0-blackwall");
    }
}

/// `Codeware.Require(version) -> Bool` — V1: sempre true (NÓS provemos a API).
unsafe extern "C" fn tramp_require(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[reg] >>> Codeware.Require chamado -> true");
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8);
    }
}

/// `BwmsEmit() -> Bool` (IA Fase 0) — o redscript chama isto p/ ENFILEIRAR um evento
/// pro processo externo de IA. NÃO bloqueia (só escreve um arquivo e retorna) — a regra
/// arquitetural: a native roda na thread do jogo, o LLM lento mora num processo separado.
unsafe extern "C" fn tramp_bwms_emit(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::ai::emit_event();
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, 1u8); // Bool = true
    }
}

/// FOUNDATIONAL — native que LÊ o arg do redscript: `BwmsEchoF(x: Float) -> Float`. Usa
/// `read_params_consuming` (consome o frame, sem original p/ re-ler). Loga o arg + ecoa. Destrava
/// dispatch dinâmico (CallbackSystem) e expor Reflection (getf/setf/callf) pro redscript.
unsafe extern "C" fn tramp_echo(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let raw = args.first().map(|(v, _)| *v).unwrap_or(0);
    let fv = f32::from_bits(raw as u32);
    crate::log(&format!(
        "[echo] BwmsEchoF recebeu arg = {fv} (raw {raw:#x}, {} arg(s)) — redscript→native COM ARG!",
        args.len()
    ));
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u32, raw as u32); // ecoa o Float
    }
}

/// Expõe Reflection GETF pro redscript: `BwmsGetPlayerField(field: CName) -> Float` — lê o CName
/// arg, acha a propriedade por nome no player vivo (find_property + prop_get_f32) e retorna. Prova
/// arg CName + Reflection-pro-redscript (mods leem campos do player por nome). Player via current_player().
unsafe extern "C" fn tramp_getfield(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
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
    crate::log(&format!(
        "[getfield] BwmsGetPlayerField('{name}' hash {hash:#x}) = {val} (achou={found}) — Reflection getf VIA REDSCRIPT"
    ));
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u32, val.to_bits());
    }
}

/// Como `register_global`, mas o POD HERDA a assinatura (params) de `proto_params` (clona
/// `params@0x28` + `count@0x30`) → o bind redscript de `native func X(args)` casa, e `read_params`
/// lê os args. `proto_params` deve ter a MESMA assinatura (ex.: AbsF/Cos = `(Float)->Float`).
pub unsafe fn register_global_argful(
    reg: &Registry,
    proto_vtable: *mut c_void,
    proto_params: *mut c_void,
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let func = build_native_func(proto_vtable, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    if rtti::sane(proto_params) {
        let pe = rd_u64(proto_params as *const c_void, 0x28); // params (ptr)
        let pc = core::ptr::read_unaligned((proto_params as *const u8).add(0x30) as *const u32); // count
        wr_u64(func, 0x28, pe);
        wr_u32(func, 0x30, pc);
    }
    if !call_register_function(reg, func) {
        return false;
    }
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log(&format!(
        "[reg] register_global_argful {full}: re-resolve -> {} ({back:p})",
        if ok { "OK" } else { "FALHOU" }
    ));
    ok
}

/// Compõe um array de params EMPRESTANDO `CProperty[idx]` de cada proto → (entries_ptr, count).
/// Permite assinaturas multi-arg de tipos arbitrários SEM GetType: ex. (CName,Float) =
/// `[(NameToString,0),(AbsF,0)]`. Buffer no pool do jogo (vaza pequeno; o RTTI só LÊ os params).
unsafe fn compose_params(specs: &[(*mut c_void, usize)]) -> (u64, u32) {
    let n = specs.len();
    let entries = rtti::pool_alloc(n * 8, 8) as *mut u64;
    if entries.is_null() {
        return (0, 0);
    }
    for (i, (proto, idx)) in specs.iter().enumerate() {
        let pe = rd_u64(*proto as *const c_void, 0x28) as *const u8; // params entries do proto
        let cprop = if pe.is_null() { 0 } else { rd_u64(pe.add(idx * 8) as *const c_void, 0) };
        entries.add(i).write_unaligned(cprop);
    }
    (entries as u64, n as u32)
}

/// Como `register_global_argful`, mas com params JÁ compostos (de `compose_params`).
pub unsafe fn register_global_composed(
    reg: &Registry,
    proto_vtable: *mut c_void,
    params: (u64, u32),
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let func = build_native_func(proto_vtable, full, short, handler, true);
    if func.is_null() {
        return false;
    }
    wr_u64(func, 0x28, params.0);
    wr_u32(func, 0x30, params.1);
    if !call_register_function(reg, func) {
        return false;
    }
    let back = get_function(reg, full);
    let ok = rtti::sane(back);
    crate::log(&format!(
        "[reg] register_global_composed {full}: re-resolve -> {} ({back:p})",
        if ok { "OK" } else { "FALHOU" }
    ));
    ok
}

/// Expõe Reflection SETF pro redscript: `BwmsSetPlayerField(field: CName, value: Float) -> Bool` —
/// escreve um campo do player por nome. Lê (CName, Float), find_property + prop_set_f32, round-trip log.
unsafe extern "C" fn tramp_setfield(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
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
            crate::log(&format!(
                "[setfield] BwmsSetPlayerField('{name}', {value}) = {before} -> {after} — Reflection setf VIA REDSCRIPT"
            ));
        }
    }
    if !ok {
        crate::log(&format!("[setfield] BwmsSetPlayerField('{name}', {value}) — prop não achada"));
    }
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, ok as u8); // Bool
    }
}

/// Expõe Reflection CALLF pro redscript: `BwmsCallPlayerMethod(method: CName) -> Bool` — chama um
/// método NO-ARG do player por nome (resolve_in_class + call_func). Completa get/set/CALL pro redscript.
unsafe extern "C" fn tramp_callplayer(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
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
                ok = true;
            }
        }
    }
    crate::log(&format!(
        "[callplayer] BwmsCallPlayerMethod('{name}') ok={ok} ret={ret} — Reflection callf VIA REDSCRIPT"
    ));
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, ok as u8); // Bool
    }
}

/// GetType @ CRTTISystem vtbl+0x00 — resolve QUALQUER IType por nome (CName/Float/`handle:IScriptable`
/// ...). PROVADO seguro in-game (não é dtor, jogo sobrevive). Destrava params de native de QUALQUER
/// assinatura → arg Handle p/ dispatch arbitrário do CallbackSystem.
pub unsafe fn get_type(reg: &Registry, name: &str) -> *mut c_void {
    let slot = reg.vtbl_slot(0x00);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    f(reg.raw(), cname(name))
}

/// Constrói um CProperty MÍNIMO (só `type@0x00`) p/ um IType — suficiente p/ read_params + bind do param.
unsafe fn build_min_cprop(itype: *mut c_void) -> *mut c_void {
    if itype.is_null() {
        return std::ptr::null_mut();
    }
    let cp = rtti::pool_alloc(0x30, 8);
    if cp.is_null() {
        return cp;
    }
    std::ptr::write_bytes(cp as *mut u8, 0, 0x30);
    wr_u64(cp, 0x00, itype as u64); // CProperty+0 = IType
    cp
}

/// Compõe params a partir de NOMES DE TIPO (via GetType + build_min_cprop) → (entries_ptr, count).
/// Permite QUALQUER assinatura sem precisar de proto pra clonar. Ex.: `["handle:IScriptable","CName"]`.
unsafe fn compose_params_from_types(reg: &Registry, type_names: &[&str]) -> (u64, u32) {
    let n = type_names.len();
    let entries = rtti::pool_alloc(n * 8, 8) as *mut u64;
    if entries.is_null() {
        return (0, 0);
    }
    for (i, tn) in type_names.iter().enumerate() {
        let it = get_type(reg, tn);
        let cp = build_min_cprop(it);
        entries.add(i).write_unaligned(cp as u64);
    }
    (entries as u64, n as u32)
}

/// CallbackSystem DISPATCH: `BwmsCallMethod(target: ref<IScriptable>, function: CName) -> Bool` —
/// chama um método no-arg de QUALQUER objeto (não só player) por nome. Lê o Handle (obj ptr) + CName,
/// resolve_in_class(class_of(target)) + call_func. É o núcleo do dispatch dinâmico do CallbackSystem.
unsafe extern "C" fn tramp_callmethod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
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
    crate::log(&format!(
        "[callmethod] BwmsCallMethod(target={target:p}, '{name}') ok={ok} ret={ret} — DISPATCH ARBITRÁRIO (CallbackSystem core)"
    ));
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, ok as u8);
    }
}

// ===== CallbackSystem (lite): registry event→callbacks + RegisterCallback + fire/dispatch =====
/// (event_hash, target_ptr_as_usize, function_hash). V1: ptr direto (válido enquanto o alvo vive;
/// pra robustez = wref, futuro). É a registry do Codeware CallbackSystem em Rust.
static CALLBACKS: Mutex<Vec<(u64, usize, u64)>> = Mutex::new(Vec::new());

/// `BwmsRegisterCallback(eventName: CName, target: ref<IScriptable>, function: CName) -> Bool` —
/// registra um callback (= Codeware `CallbackSystem.RegisterCallback`). 3 args via compose_params_from_types.
unsafe extern "C" fn tramp_register_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let ok = if ev != 0 && target != 0 && fn_hash != 0 {
        CALLBACKS
            .lock()
            .map(|mut c| {
                c.push((ev, target as usize, fn_hash));
                true
            })
            .unwrap_or(false)
    } else {
        false
    };
    crate::log(&format!(
        "[cbs] RegisterCallback(event '{}', target={target:#x}, fn '{}') ok={ok}",
        crate::cname::resolve_cname(ev),
        crate::cname::resolve_cname(fn_hash)
    ));
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, ok as u8);
    }
}

/// Emite um evento PASSANDO ARGS pro callback (= o evento carrega dados, ex. a tecla no input).
/// Despacha `target.function(args...)` via resolve_in_class + call_func. Devolve quantos despachou.
/// É o que os CONTROLLERS chamam quando a função de jogo hookada dispara.
pub unsafe fn fire_event_args(event_name: &str, args: &[rtti::Arg]) -> usize {
    let eh = cname(event_name);
    let cbs: Vec<(usize, u64)> = match CALLBACKS.lock() {
        Ok(c) => c.iter().filter(|(e, _, _)| *e == eh).map(|(_, t, f)| (*t, *f)).collect(),
        Err(_) => return 0,
    };
    if cbs.is_empty() {
        return 0;
    }
    // cap de log: eventos periódicos (Update) não devem spammar — loga as ~12 primeiras emissões.
    static FIRE_LOG: AtomicUsize = AtomicUsize::new(0);
    let do_log = FIRE_LOG.fetch_add(1, Ordering::Relaxed) < 12;
    if do_log {
        crate::log(&format!("[cbs] fire '{event_name}'({} arg) → {} callback(s)", args.len(), cbs.len()));
    }
    let mut n = 0;
    for (target_us, fn_hash) in cbs {
        let target = target_us as *mut c_void;
        if target.is_null() || !rtti::sane(target) {
            continue;
        }
        let name = crate::cname::resolve_cname(fn_hash);
        if let Some(rf) = rtti::resolve_in_class(rtti::class_of(target), &name) {
            let r = rtti::call_func(&rf, target, args);
            if do_log {
                crate::log(&format!("[cbs]   dispatched '{name}' to {target:p} ok={}", r.is_some()));
            }
            if r.is_some() {
                n += 1;
            }
        }
    }
    n
}

/// Emite um evento SEM args (callbacks no-arg). Atalho de `fire_event_args`.
pub unsafe fn fire_event(event_name: &str) -> usize {
    fire_event_args(event_name, &[])
}

/// Guard de registro único (idempotente). Resetado p/ retry se o RTTI ainda não está pronto.
static REGISTERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Registra TODAS as nativas do BWMS no RTTI, 1x. Chamado CEDO (selfboot + cp77_tick) p/ a
/// nativa existir ANTES do bind do script (o redscript resolve `native func` por nome no load).
/// Hoje: BlackwallPing (smoke da PONTE redscript→native = fundação F-B do Codeware). Se o RTTI
/// ainda não estiver pronto, reseta o guard e tenta de novo na próxima chamada.
pub unsafe fn register_all() {
    use std::sync::atomic::Ordering;
    // Fast-path BARATO p/ o hot-path do executor: já registrado → 1 load e sai.
    if REGISTERED.load(Ordering::Relaxed) {
        return;
    }
    if REGISTERED.swap(true, Ordering::Relaxed) {
        return;
    }
    let reg = match rtti::Registry::obtain() {
        Some(r) => r,
        None => {
            REGISTERED.store(false, Ordering::Relaxed);
            return;
        }
    };
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = get_function(&reg, n);
        if rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !rtti::sane(proto) {
        crate::log("[reg] register_all: sem protótipo global (Cos/Sin/...) — adiado");
        REGISTERED.store(false, Ordering::Relaxed);
        return;
    }
    let ok = register_global(&reg, proto, "BlackwallPing", "BlackwallPing", tramp_ping);
    crate::log(&format!(
        "[reg] register_all: BlackwallPing registrado={ok} (ponte redscript→native, F-B)"
    ));
    // SEGUNDA native real — prova que register_all escala (N funções no RTTI) e que o
    // retorno Bool native→redscript é consumido numa condicional do .reds.
    let ok2 = register_global(&reg, proto, "BwmsAutoContinue", "BwmsAutoContinue", tramp_autocontinue);
    crate::log(&format!(
        "[reg] register_all: BwmsAutoContinue registrado={ok2} (2a native real, retorno consumido)"
    ));
    // IA Fase 0: ponte game->processo-externo (enfileira evento, non-blocking).
    let ok3 = register_global(&reg, proto, "BwmsEmit", "BwmsEmit", tramp_bwms_emit);
    crate::log(&format!("[reg] register_all: BwmsEmit registrado={ok3} (IA Fase 0)"));
    // FOUNDATIONAL: native que LÊ arg do redscript. proto (Cos/AbsF/...) é (Float)->Float → herda
    // a assinatura (1 Float). Destrava arg-natives (CallbackSystem dispatch, Reflection p/ redscript).
    let ok4 = register_global_argful(&reg, proto, proto, "BwmsEchoF", "BwmsEchoF", tramp_echo);
    crate::log(&format!("[reg] register_all: BwmsEchoF registrado={ok4} (arg-reading foundational)"));
    // Reflection GETF pro redscript: BwmsGetPlayerField(CName)->Float. proto de params = um global
    // com 1 param CName (NameToString/IsNameValid), disponível cedo.
    let proto_cn = ["NameToString", "IsNameValid", "StringToName"]
        .iter()
        .find_map(|n| {
            let p = get_function(&reg, n);
            if rtti::sane(p) { Some(p) } else { None }
        });
    if let Some(pc) = proto_cn {
        let ok5 = register_global_argful(&reg, proto, pc, "BwmsGetPlayerField", "BwmsGetPlayerField", tramp_getfield);
        crate::log(&format!("[reg] register_all: BwmsGetPlayerField registrado={ok5} (Reflection getf pro redscript)"));
        // setf pro redscript: BwmsSetPlayerField(CName, Float) -> Bool. params compostos (CName + Float).
        let params2 = compose_params(&[(pc, 0), (proto, 0)]);
        if params2.1 == 2 {
            let ok6 = register_global_composed(&reg, proto, params2, "BwmsSetPlayerField", "BwmsSetPlayerField", tramp_setfield);
            crate::log(&format!("[reg] register_all: BwmsSetPlayerField registrado={ok6} (Reflection setf pro redscript, params compostos)"));
        }
        // callf pro redscript: BwmsCallPlayerMethod(CName)->Bool. Completa get/set/CALL pro redscript.
        let ok7 = register_global_argful(&reg, proto, pc, "BwmsCallPlayerMethod", "BwmsCallPlayerMethod", tramp_callplayer);
        crate::log(&format!("[reg] register_all: BwmsCallPlayerMethod registrado={ok7} (Reflection callf pro redscript)"));
    } else {
        crate::log("[reg] register_all: sem proto CName p/ BwmsGetPlayerField");
    }
    // CallbackSystem DISPATCH (núcleo): BwmsCallMethod(ref<IScriptable>, CName)->Bool. Params via
    // GetType (provado) → assinatura (Handle, CName) sem precisar clonar proto. Despacha p/ qualquer alvo.
    let params_hc = compose_params_from_types(&reg, &["handle:IScriptable", "CName"]);
    if params_hc.1 == 2 {
        let ok8 = register_global_composed(&reg, proto, params_hc, "BwmsCallMethod", "BwmsCallMethod", tramp_callmethod);
        crate::log(&format!("[reg] register_all: BwmsCallMethod registrado={ok8} (dispatch arbitrário via GetType — CallbackSystem core)"));
    }
    // CallbackSystem RegisterCallback(CName, ref<IScriptable>, CName)->Bool (3-arg via GetType).
    let params_ccc = compose_params_from_types(&reg, &["CName", "handle:IScriptable", "CName"]);
    if params_ccc.1 == 3 {
        let ok9 = register_global_composed(&reg, proto, params_ccc, "BwmsRegisterCallback", "BwmsRegisterCallback", tramp_register_callback);
        crate::log(&format!("[reg] register_all: BwmsRegisterCallback registrado={ok9} (CallbackSystem RegisterCallback)"));
    }
    // Breadth Codeware: prova do register_method com REALLOC (gated ~/.bwms-regmethod-test).
    regmethod_selftest(&reg);
}

/// Breadth Reflection: probe num objeto VIVO (player), 1x, gated `~/.bwms-reflection-test`. Roda do
/// `cp77_tick` (gameplay) — em `register_all` as classes de script ainda não têm props populadas
/// (provado: PlayerPuppet não resolvia ali). Pega `class_of(player)`, dumpa props (confirma o layout
/// do CProperty no macOS) + GET no objeto vivo + round-trip set/get em objeto fake. Ver
/// rtti::reflection_probe_cls.
static REFL_LIVE_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub unsafe fn reflection_live_once(player: *mut c_void) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-reflection-test").exists())
        .unwrap_or(false);
    if !on || player.is_null() {
        return;
    }
    if REFL_LIVE_DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let cls = rtti::class_of(player);
    crate::log(&rtti::reflection_probe_cls(cls, "player(class_of)", player));
}

/// Self-test do `register_method` com realloc (gated `~/.bwms-regmethod-test`). DEV.
/// T1 = lógica de realloc ISOLADA (DynArray nosso, ZERO classe do jogo): array cheio (cap==size)
/// + 1 push → deve realocar, PRESERVAR os entries antigos e ANEXAR o novo. T2 = `register_method`
/// numa CClass real (gameGodModeSystem) — caminho integrado, 1 método (poluição mínima, some no
/// reboot). Prova o gap "V1 não realoca" fechado, com integridade verificada por igualdade de ponteiro.
unsafe fn regmethod_selftest(reg: &Registry) {
    let on = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-regmethod-test").exists())
        .unwrap_or(false);
    if !on {
        return;
    }
    // --- T1: realloc isolado (memória nossa) ---
    let buf0 = rtti::pool_alloc(2 * 8, 8) as *mut u64;
    if buf0.is_null() {
        crate::log("[regmethod-test] T1: pool_alloc null — abortando");
        return;
    }
    let (s0, s1, s2) = (0xA1A1u64, 0xB2B2u64, 0xC3C3u64);
    buf0.write_unaligned(s0);
    buf0.add(1).write_unaligned(s1);
    let mut hdr = [0u8; 16];
    (hdr.as_mut_ptr() as *mut u64).write_unaligned(buf0 as u64); // entries
    (hdr.as_mut_ptr().add(0x08) as *mut u32).write_unaligned(2); // capacity
    (hdr.as_mut_ptr().add(0x0C) as *mut u32).write_unaligned(2); // size == cap → força realloc
    let arr1 = hdr.as_mut_ptr() as *mut c_void;
    let slot = dynarray_push_ptr(arr1, s2);
    let ne = rd_u64(arr1 as *const c_void, 0x00) as *const u64;
    let ncap = core::ptr::read_unaligned((arr1 as *const u8).add(0x08) as *const u32);
    let nsize = core::ptr::read_unaligned((arr1 as *const u8).add(0x0C) as *const u32);
    let ok1 = slot == Some(2)
        && (ne as u64) != (buf0 as u64)
        && ncap >= 3
        && nsize == 3
        && ne.read_unaligned() == s0
        && ne.add(1).read_unaligned() == s1
        && ne.add(2).read_unaligned() == s2;
    crate::log(&format!(
        "[regmethod-test] T1 realloc-isolado: cap 2->{ncap} size {nsize} slot={slot:?} preservou=[{:#x},{:#x}] append={:#x} OK={ok1}",
        ne.read_unaligned(),
        ne.add(1).read_unaligned(),
        ne.add(2).read_unaligned()
    ));
    // --- T2: register_method numa classe REAL (caminho integrado) ---
    let class = "gameGodModeSystem";
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => {
            crate::log("[regmethod-test] T2: sem protótipo (AddGodMode não resolveu) — só T1");
            return;
        }
    };
    let cls = reg.class_by_name(class);
    if !rtti::sane(cls) {
        crate::log("[regmethod-test] T2: classe não resolveu — só T1");
        return;
    }
    let arr = (cls as *mut u8).add(0x48) as *mut c_void;
    let size_before = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    let ok2 = register_method(
        reg, class, proto, "gameGodModeSystem::BwmsRegTest", "BwmsRegTest", tramp_ping, false,
    );
    let size_after = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    crate::log(&format!(
        "[regmethod-test] T2 register_method real: {class}.BwmsRegTest ok={ok2} size {size_before}->{size_after} (esperado +1)"
    ));
}

/// POD de BlackwallPing, construído 1x on-demand e cacheado.
static OUR_POD: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// F-B: PROVÊ o POD de BlackwallPing ON-DEMAND — chamado do hook do GetFunction quando o binder
/// do redscript pede a native (no load ~6s) e a original dá null. Constrói 1x (cache) clonando a
/// vtable de um proto (Cos), resolvido pela GetFunction ORIGINAL (`orig_getfn`, evita recursão no
/// nosso hook). Reusa build_native_func + add_route (handler=tramp_ping). SEM RegisterFunction
/// (não precisa: a gente entrega o ponteiro direto pro binder). Assinatura vem do `.reds` (import).
pub unsafe fn provide_blackwallping(this: *mut c_void, orig_getfn: *mut c_void) -> *mut c_void {
    use std::sync::atomic::Ordering;
    let cached = OUR_POD.load(Ordering::Relaxed);
    if !cached.is_null() {
        return cached;
    }
    if orig_getfn.is_null() {
        return std::ptr::null_mut();
    }
    let f: unsafe extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(orig_getfn);
    let mut proto = std::ptr::null_mut();
    for n in ["Cos", "Sin", "AbsF", "SqrtF", "LogF"] {
        let p = f(this, cname(n));
        if rtti::sane(p) {
            proto = p;
            break;
        }
    }
    if !rtti::sane(proto) {
        return std::ptr::null_mut();
    }
    let pod = build_native_func(proto, "BlackwallPing", "BlackwallPing", tramp_ping, true);
    if !pod.is_null() {
        OUR_POD.store(pod, Ordering::Relaxed);
    }
    pod
}

// ===== Probe + smoke-test (comandos de console) =====================================

/// Despeja o layout de um objeto-função nativo REAL p/ confirmar offsets
/// (fullName/shortName/flags/handler) e a vtable a clonar. Resolve uma função
/// nativa conhecida e anota cada qword: PTR (ponteiro mapeável) e match de CName.
pub unsafe fn probe(reg: &Registry) -> String {
    let cands: &[(&[&str], &str)] = &[
        (&["gameGodModeSystem"], "AddGodMode"),
        (&["gameStatPoolsSystem"], "RequestSettingStatPoolValue"),
        (&["PlayerDevelopmentData"], "SetLevel"),
    ];
    let mut func = std::ptr::null_mut();
    let mut label = String::new();
    for (classes, m) in cands {
        if let Some(rf) = rtti::resolve_any(reg, classes, m) {
            func = rf.func;
            label = format!("{}::{m}", classes[0]);
            break;
        }
    }
    if !rtti::sane(func) {
        return "[probe] nenhuma função nativa de amostra resolveu".into();
    }
    let want_full = cname(label.split("::").nth(1).unwrap_or(""));
    let mut out = format!("[probe] amostra={label} func={func:p}\n");
    if !crate::gum::is_readable(func as *const c_void, FUNC_POD_SIZE) {
        return out + "  (POD ilegível)";
    }
    for off in (0..FUNC_POD_SIZE).step_by(8) {
        let v = rd_u64(func as *const c_void, off);
        let ptr = crate::gum::is_readable(v as *const c_void, 8);
        let mut tag = String::new();
        if off == 0x00 {
            tag.push_str(" <- vtable (clonar esta)");
        }
        if v == want_full {
            tag.push_str(" <- shortName(CName) casou");
        }
        if ptr && off != 0x00 {
            tag.push_str(" PTR(handler?)");
        }
        out.push_str(&format!("  +{off:#04x}: {v:#018x}{tag}\n"));
    }
    out.push_str(&format!(
        "  → ajuste HANDLER_OFFSET p/ o +offset do PTR de código (provável 0xA8-0xC0). Hoje={HANDLER_OFFSET:#x}\n"
    ));
    out
}

/// Smoke-test: registra um GLOBAL `BlackwallPing()->Bool` e confirma que volta a
/// resolver por nome. Se resolver, o registro nativo funciona; chamar do redscript
/// valida o handler (depende do HANDLER_OFFSET certo).
pub unsafe fn register_smoke(reg: &Registry) -> String {
    // Precisa de um GLOBAL existente p/ clonar a vtable de CGlobalFunction.
    let proto_names = ["Cos", "Sin", "AbsF", "SqrtF", "LogF", "TanF", "AsinF"];
    let mut proto = std::ptr::null_mut();
    let mut proto_name = "";
    for n in proto_names {
        let p = get_function(reg, n);
        if rtti::sane(p) {
            proto = p;
            proto_name = n;
            break;
        }
    }
    if !rtti::sane(proto) {
        return "[reg] nenhum global nativo de protótipo (Cos/Sin/...) resolveu — não dá p/ clonar a vtable de CGlobalFunction".into();
    }
    crate::log(&format!("[reg] protótipo de global = {proto_name} ({proto:p})"));
    let ok = register_global(reg, proto, "BlackwallPing", "BlackwallPing", tramp_ping);
    if !ok {
        return "[reg] smoke BlackwallPing: registro FALHOU ✗".into();
    }
    // CHAMA BlackwallPing() de verdade: re-resolve o POD por nome e invoca via call_func.
    // call_func chama o executor (que está HOOKADO por nós) → route_native casa nosso func
    // POD → tramp_ping roda e escreve Bool=1 no retorno. Prova a cadeia INTEIRA num comando:
    // registro no RTTI + roteamento no executor + handler Rust executado.
    let back = get_function(reg, "BlackwallPing");
    if !rtti::sane(back) {
        return "[reg] smoke: ENTROU no RTTI ✓ mas re-resolve falhou — não dá p/ chamar".into();
    }
    let rf = rtti::ResolvedFn { func: back, ret_type: std::ptr::null_mut(), is_static: true };
    match rtti::call_func(&rf, std::ptr::null_mut(), &[]) {
        Some(ret) => format!(
            "[reg] smoke BlackwallPing: RTTI ✓ + HANDLER RODOU ✓ (Bool retornado = {}). Routing-hook OK — ver '>>> BlackwallPing chamado' no log.",
            ret[0]
        ),
        None => "[reg] smoke: RTTI ✓ mas call_func não completou — ver log (handler pode não ter rodado)".into(),
    }
}

/// Registra a fatia mínima do Facade Codeware (Version/Require) como métodos
/// estáticos da classe `Codeware` (que vem do redscript do Codeware). Só funciona
/// se o .reds do Codeware estiver carregado (a CClass `Codeware` precisa existir).
pub unsafe fn register_codeware_facade(reg: &Registry) -> String {
    // protótipo de método: clona a vtable de CClassFunction de um método estático
    // nativo conhecido.
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] sem protótipo de método (AddGodMode não resolveu)".into(),
    };
    let v = register_method(reg, "Codeware", proto, "Codeware::Version", "Version", tramp_version, true);
    let r = register_method(reg, "Codeware", proto, "Codeware::Require", "Require", tramp_require, true);
    format!("[reg] Codeware.Version={v} Codeware.Require={r} (precisa do .reds do Codeware carregado p/ a classe existir)")
}
