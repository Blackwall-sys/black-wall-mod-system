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
/// Escreve o retorno Bool (u8) de uma native no `out` do frame, se não-nulo. VIA CANÔNICA dos
/// trampolines — antes o `if !out.is_null() { write_unaligned(out as *mut u8, v as u8); }` estava
/// repetido ~16× (a auditoria de convergência apontou o boilerplate).
#[inline]
unsafe fn write_bool_ret(out: *mut c_void, v: bool) {
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut u8, v as u8);
    }
}
/// Escreve um Int32 no slot de retorno (pro seletor de modo CPVR ler no redscript).
#[cfg(feature = "cpvr")]
unsafe fn write_i32_ret(out: *mut c_void, v: i32) {
    if !out.is_null() {
        core::ptr::write_unaligned(out as *mut i32, v);
    }
}

/// Escreve um inteiro sem sinal no slot de retorno, na LARGURA certa (1/2/4/8 bytes) — usado
/// pelos `BitSetN`/`BitShiftLN`/`BitShiftRN` (Codeware `Utils/Bits.reds`), cujo retorno é
/// `UintN` (N variando por função). VIA CANÔNICA, mesmo espírito do `write_bool_ret`.
#[inline]
unsafe fn write_uint_ret(out: *mut c_void, v: u64, width_bytes: u8) {
    if out.is_null() {
        return;
    }
    match width_bytes {
        1 => core::ptr::write_unaligned(out as *mut u8, v as u8),
        2 => core::ptr::write_unaligned(out as *mut u16, v as u16),
        4 => core::ptr::write_unaligned(out as *mut u32, v as u32),
        _ => core::ptr::write_unaligned(out as *mut u64, v),
    }
}

/// Escreve um `ref<T>` (handle) NOVO no slot de retorno — 2026-07-18, achado ao vivo (crash-report
/// `Cyberpunk2077-2026-07-18-144828.ips`, `SIGBUS EXC_ARM_DA_ALIGN`): um `ref<T>` LOCAL em bytecode
/// compilado NÃO é um ponteiro cru de 8 bytes — é uma estrutura de **16 bytes**: `[+0x00]`=ponteiro
/// do objeto, `[+0x08]`=ponteiro pro BLOCO DE REFCOUNT (ou `0` = sem refcount/"não-dono", igual ao
/// padrão de `weak`/raw). Confirmado por disasm da rotina de RELEASE que o compilador injeta no
/// teardown de TODO frame com locais `ref<>` (`0x1021048c4`, achada ao vivo no crash): lê
/// `[slot+0x08]`, se não-nulo faz `ldaddal w9,w8,[refcount_block+4]` (decremento ATÔMICO, exige
/// alinhamento de 4 bytes — o crash: `write_uint_ret` só escrevia os 8 bytes de `[+0x00]`, deixando
/// `[+0x08]` com LIXO da stack; a rotina lia esse lixo como ponteiro-de-refcount-block e o `ldaddal`
/// numa base desalinhada faturava SIGBUS). Fix: escrever os 16 bytes — `[+0x08]=0` EXPLÍCITO faz a
/// rotina de release tomar o ramo `cbz x8,...` (sem refcount = no-op seguro), igual a um handle
/// "raw"/sem dono. Usado por TODA função nossa que devolve `ref<classe-forjada>`.
#[inline]
unsafe fn write_handle_ret(out: *mut c_void, ptr: u64) {
    if out.is_null() {
        return;
    }
    core::ptr::write_unaligned(out as *mut u64, ptr);
    core::ptr::write_unaligned((out as *mut u64).add(1), 0u64);
}

/// Escreve um `red::CString` NOVO no slot de retorno — SÓ o caminho SSO/inline (string <=19
/// bytes UTF-8). Layout = o inverso exato de `rtti::read_cstring` (mesmo union em +0x00,
/// length SEM a NOT_INLINE_FLAG em +0x14, allocator zerado em +0x18). Mesma técnica já usada
/// (e provada em todo `s:` arg desta sessão) por `rtti::red_string_write_inline` — só que
/// aquela escreve um ARGUMENTO (dentro do buffer `locals` de call_func, sempre 0x20B por slot);
/// esta escreve um RETORNO (precisa do alargamento de `call_func::res` pra 0x20B, ver nota lá).
/// Strings >19 bytes exigiriam alocar no heap do PRÓPRIO jogo (allocator certo) — fora de
/// escopo (risco real de corrupção se malfeito); retorna `false` nesse caso, o chamador decide
/// o fallback. Requer que `out` aponte pra >= 0x20 bytes graváveis (garantido pelo alargamento
/// de `call_func`/pelo executor real do jogo, que aloca o slot pelo tipo de retorno declarado).
unsafe fn write_cstring_inline_ret(out: *mut c_void, s: &str) -> bool {
    const CSTRING_SIZE: usize = 0x20;
    const INLINE_CAP: usize = 20;
    if out.is_null() {
        return false;
    }
    let bytes = s.as_bytes();
    if bytes.len() >= INLINE_CAP {
        return false;
    }
    core::ptr::write_bytes(out as *mut u8, 0, CSTRING_SIZE);
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len());
    core::ptr::write_unaligned((out as *mut u8).add(0x14) as *mut u32, bytes.len() as u32);
    true
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

/// RegisterType STEP-1 (Codeware): forja uma CClass MÍNIMA e registra via `RegisterType@vtbl+0x80`.
/// **Register-SEM-instanciar** = baixo risco (mapa do CClass em proofs/map-cclass-forge): a vtable é
/// CLONADA de uma classe nativa (getters reais GetName@0x10/GetSize@0x18/GetType@0x28), todo o resto
/// é ZERO (DynArrays size=0 → o dump não itera nada), e `isAbstract=1` blinda contra CreateInstance.
/// Blast radius = "ou GetClass devolve o ptr e o rttidump lê, ou não aparece". NÃO instancia, NÃO
/// popula prop/func/default. Layout CClass (0x2D0): name(CName)@0x18, size(u32)@0x68, flags@0x70,
/// alignment@0x74. Confirmar com `rttidump <nome>` depois. Retorna a forja ou null.
/// FIX (2026-07-18, sessão `dynarraygrowth-probe`): todo forge de CClass abaixo aloca `CCLASS_SIZE`
/// (0x2D0) bytes e faz `write_bytes(mem, 0, CCLASS_SIZE)` antes de escrever só um punhado de campos
/// conhecidos (vtable/name/size/flags/parent/...). O RESTO fica zerado — inclusive vários hashmaps
/// EMBUTIDOS (bucket array + entries + capacity + stride + um "alocador" sub-objeto cujo 1º campo
/// é um vtable pointer, formato confirmado via a rotina de crescimento `0x10096ca74`, ver
/// `dynarraygrowth_probe_hook` em selfboot.rs). Rastreado AO VIVO (crash-report + hook 2026-07-18):
/// quando o engine insere a 1ª entrada num desses hashmaps (achado disparando ao registrar
/// `GetService` numa classe forjada — happens no 1º `register_method` de uma classe), um invoke-
/// thunk genérico desreferencia o vtable NULO do alocador -> SIGSEGV. Achados 2 slots ao vivo
/// (`CClass+0x78` e `CClass+0xA8`, exatamente 0x30 bytes = tamanho de 1 slot, um atrás do outro).
///
/// Esta função varre candidatos espaçados de 0x30 a partir de `0x78` (dentro de `CCLASS_SIZE`) e,
/// pra CADA um, só copia o vtable do alocador (`slot+0x28`, 8 bytes) do DONOR real se: (a) os
/// primeiros 0x28 bytes do slot no donor forem TODOS "vazio" (cada palavra de 32 bits é `0x0` OU
/// `0xFFFFFFFF` — o MESMO esquema de sentinela -1 que a própria rotina de crescimento usa pros
/// slots de bucket vazios, achado ao vivo: `container+0x20` do slot @0xA8 é `0xFFFFFFFF`, não
/// zero puro — "vazio" não é só zero neste formato) — o MESMO estado que o nosso forge já produz
/// (zero) OU um estado "vazio" equivalente, provando que não há dado per-instância ali, só a
/// vtable estática do tipo alocador — E (b) o valor em `slot+0x28` passar `rtti::sane()` (parece
/// mesmo um ponteiro, não lixo). Auto-guardado: nunca copia um slot que não bata as 2 condições —
/// pior caso é deixar aquele slot específico com o mesmo risco de antes (nunca corrompe nada, só
/// não conserta um slot que a heurística não reconheceu). Loga 1x por slot corrigido.
unsafe fn fix_embedded_allocator_vtables(mem: *mut c_void, src: *mut c_void, cclass_size: usize) {
    const SLOT_STRIDE: usize = 0x30;
    const FIRST_SLOT: usize = 0x78;
    let mut off = FIRST_SLOT;
    let mut fixed = 0usize;
    let mut checked = 0usize;
    while off + SLOT_STRIDE <= cclass_size {
        checked += 1;
        if crate::gum::is_readable((src as *const u8).add(off) as *const c_void, SLOT_STRIDE) {
            let mut prefix_empty = true;
            for i in (0..0x28usize).step_by(4) {
                let w = ((src as *const u8).add(off + i) as *const u32).read_unaligned();
                if w != 0 && w != 0xFFFF_FFFF {
                    prefix_empty = false;
                    break;
                }
            }
            if prefix_empty {
                let alloc_vt = rd_u64(src as *const c_void, off + 0x28);
                if rtti::sane(alloc_vt as *mut c_void) {
                    wr_u64(mem, off + 0x28, alloc_vt);
                    fixed += 1;
                }
            }
        }
        off += SLOT_STRIDE;
    }
    if fixed > 0 {
        crate::log(&format!(
            "[reg] fix_embedded_allocator_vtables: {fixed}/{checked} slot(s) corrigido(s) (hashmap embutido, vtable copiado do donor)"
        ));
    }
}

pub unsafe fn register_type_min(reg: &Registry, new_name: &str) -> *mut c_void {
    const CCLASS_SIZE: usize = 0x2D0;
    // dedup DURO: sobrescrever um tipo existente corromperia o RTTI global → aborta se já existe.
    if rtti::sane(reg.class_by_name(new_name)) {
        crate::log(&format!("[reg] register_type: '{new_name}' JÁ existe — abortado (não sobrescrevo o RTTI)"));
        return std::ptr::null_mut();
    }
    crate::log(&format!("[reg] register_type: entrou p/ '{new_name}'"));
    // fonte da vtable: 1ª classe nativa concreta que resolver (só usamos os getters dela).
    let mut src = std::ptr::null_mut();
    let mut src_name = "";
    for n in ["gameuiInGameMenuGameController", "ScriptableSystem", "GameObject", "IScriptable", "PlayerPuppet", "gameObject"] {
        let c = reg.class_by_name(n);
        if rtti::sane(c) {
            src = c;
            src_name = n;
            break;
        }
    }
    if !rtti::sane(src) {
        crate::log("[reg] register_type: nenhuma classe-fonte p/ clonar a vtable");
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(src as *const c_void, 0x00);
    crate::log(&format!("[reg] register_type: fonte='{src_name}' cls={src:p} vtable={vtable:#x}"));
    if vtable == 0 {
        crate::log("[reg] register_type: vtable da fonte = 0 (abortado)");
        return std::ptr::null_mut();
    }
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        crate::log("[reg] register_type: pool_alloc(0x2D0) devolveu null (abortado)");
        return std::ptr::null_mut();
    }
    crate::log(&format!("[reg] register_type: forja alocada {mem:p}"));
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE); // memset 0 ANTES de qualquer campo
    wr_u64(mem, 0x00, vtable); // vtable CLONADA (getters reais)
    wr_u64(mem, 0x18, cname(new_name)); // name (CName) — a chave do RegisterType
    wr_u32(mem, 0x68, 0x40); // size = sizeof(IScriptable)
    wr_u32(mem, 0x70, 0x03); // flags: isAbstract(bit0)|isNative(bit1) — blinda instanciar
    wr_u32(mem, 0x74, 0x08); // alignment = 8 (NUNCA 0: AllocMemory faria AlignUp(size,0)=crash)
    // PARENT (CClass+0x10) — achado 2026-07-13 via RE offline (disasm de 0x1021fc61c, o
    // validador de classe nativa do bundle): ele chama um walker `IsKindOf(cls, base)` que segue
    // ESTE ponteiro em loop (até ~16 níveis) comparando contra um ponteiro-base fixo (cache
    // global, quase certeza IScriptable — todo `native class` sem `extends` explícito herda
    // implicitamente dele). O memset-zero do forge deixava esse campo NULL, então o walker
    // falhava na 1ª iteração (cbz) — a classe "existia" e os métodos resolviam por nome (nossa
    // reflection não olha o parent), mas o VALIDADOR NATIVO do motor rejeitava (retorno 0 pro
    // Codeware, confirmado ao vivo mesmo com Version/Require=true). Setar o parent de verdade é
    // o candidato mais forte pro fix real da Facade.
    // CONTROLE FEITO (2026-07-13, mesma sessão): desliguei isto por 1 boot pra isolar a causa de
    // 4 travamentos seguidos em t=38s/phase=-1 — o boot de CONTROLE (sem este write) travou no
    // MESMO ponto exato, provando que a causa é degradação ambiental da máquina (sessão de
    // muitas horas, uptime 1d+14h), NÃO este código. Reativado.
    let parent = reg.class_by_name("IScriptable");
    if rtti::sane(parent) {
        wr_u64(mem, 0x10, parent as u64);
        crate::log(&format!("[reg] register_type: parent (CClass+0x10) = IScriptable @ {parent:p}"));
    } else {
        crate::log("[reg] register_type: IScriptable não resolveu — parent fica NULL (achado 2026-07-13 sugere que isso quebra a validação nativa)");
    }
    // FIX (2026-07-18, sessão `dynarraygrowth-probe`): mesmo fix de `register_type_instantiable_
    // with_parent` — conserta os hashmaps embutidos ANTES do RegisterType (ver
    // `fix_embedded_allocator_vtables`). Classes forjadas por ESTA função hoje têm 0 métodos
    // (`ScriptableService`/`CallbackSystemTarget`), mas aplicar aqui tb blinda qualquer
    // `register_method` futuro nelas contra o mesmo crash.
    fix_embedded_allocator_vtables(mem, src, CCLASS_SIZE);
    // RegisterType@vtbl+0x80 = IRTTISystem::RegisterType(IType* type, u32 hash=0).
    let slot = reg.vtbl_slot(0x80);
    if !rtti::sane(slot) {
        crate::log("[reg] RegisterType (vtbl+0x80) ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    // confirma SEM instanciar: re-resolve por nome.
    let back = reg.class_by_name(new_name);
    let ok = rtti::sane(back);
    crate::log(&format!(
        "[reg] register_type '{new_name}' -> forja {mem:p} (vtable {vtable:#x} de fonte {src:p}); re-resolve {} ({back:p})",
        if ok { "OK ✓ (RTTI aceitou a classe forjada)" } else { "FALHOU" }
    ));
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
}

/// STEP-2: forja uma classe INSTANCIÁVEL — alias fiel de `src_name` (vtable + size@0x68 + align@0x74
/// REAIS da fonte; flags native SEM isAbstract). Como size/vtable batem com a fonte, `newobj <nome>`
/// constrói um objeto VÁLIDO (o Construct@vtbl+0x40 escreve dentro do size certo, sem corromper heap;
/// class_of do objeto devolve a classe forjada via nativeType@obj+0x30). Prova a instanciação de
/// classe forjada. Confirmar: `cwregalias <novo> <src>` -> `newobj <novo>` (deve dar OK sem crash).
pub unsafe fn register_type_alias(reg: &Registry, new_name: &str, src_name: &str) -> *mut c_void {
    const CCLASS_SIZE: usize = 0x2D0;
    if rtti::sane(reg.class_by_name(new_name)) {
        crate::log(&format!("[reg] register_type_alias: '{new_name}' JÁ existe — abortado"));
        return std::ptr::null_mut();
    }
    let src = reg.class_by_name(src_name);
    if !rtti::sane(src) {
        crate::log(&format!("[reg] register_type_alias: fonte '{src_name}' não achada"));
        return std::ptr::null_mut();
    }
    if !crate::gum::is_readable(src as *const c_void, 0x78) {
        crate::log("[reg] register_type_alias: fonte ilegível");
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(src as *const c_void, 0x00);
    let src_size = ((src as *const u8).add(0x68) as *const u32).read_unaligned();
    let src_align = ((src as *const u8).add(0x74) as *const u32).read_unaligned();
    crate::log(&format!(
        "[reg] register_type_alias: fonte='{src_name}' vtable={vtable:#x} size={src_size:#x} align={src_align}"
    ));
    if vtable == 0 || src_size == 0 || src_size > 0x10000 {
        crate::log("[reg] register_type_alias: fonte inválida (vtable=0 ou size fora de faixa)");
        return std::ptr::null_mut();
    }
    let align = if src_align == 0 || src_align > 64 { 8 } else { src_align };
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        crate::log("[reg] register_type_alias: pool_alloc null");
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE);
    wr_u64(mem, 0x00, vtable); // vtable da fonte (Construct/getters reais)
    wr_u64(mem, 0x18, cname(new_name)); // name
    wr_u32(mem, 0x68, src_size); // size REAL da fonte (p/ o Construct escrever certo)
    wr_u32(mem, 0x6C, src_size); // holderSize = size
    wr_u32(mem, 0x70, 0x02); // flags: isNative, SEM isAbstract → instanciável
    wr_u32(mem, 0x74, align); // alignment REAL
    let slot = reg.vtbl_slot(0x80);
    if !rtti::sane(slot) {
        crate::log("[reg] register_type_alias: RegisterType@0x80 ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    let back = reg.class_by_name(new_name);
    let ok = rtti::sane(back);
    crate::log(&format!(
        "[reg] register_type_alias '{new_name}' (alias de '{src_name}', size={src_size:#x}) -> {} ({mem:p})",
        if ok { "OK ✓ (instanciável — teste com newobj)" } else { "FALHOU" }
    ));
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
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
        crate::log(&format!("[reg] register_type_instantiable: '{new_name}' JÁ existe — abortado"));
        return std::ptr::null_mut();
    }
    let src = reg.class_by_name(vtable_donor);
    if !rtti::sane(src) || !crate::gum::is_readable(src as *const c_void, 0x78) {
        crate::log(&format!("[reg] register_type_instantiable: donor '{vtable_donor}' inválido"));
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
        crate::log("[reg] register_type_instantiable: donor inválido (vtable/size)");
        return std::ptr::null_mut();
    }
    let align = if src_align == 0 || src_align > 64 { 8 } else { src_align };
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        crate::log("[reg] register_type_instantiable: pool_alloc null");
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE);
    wr_u64(mem, 0x00, vtable); // vtable do donor (Construct/getters reais)
    wr_u64(mem, 0x18, cname(new_name)); // name
    wr_u32(mem, 0x68, src_size); // size REAL do donor (Construct escreve dentro do size certo)
    wr_u32(mem, 0x6C, src_size); // holderSize = size
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
        crate::log("[reg] register_type_instantiable: RegisterType@0x80 ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    let back = reg.class_by_name(new_name);
    let ok = rtti::sane(back);
    crate::log(&format!(
        "[reg] register_type_instantiable '{new_name}' (donor='{vtable_donor}' parent={parent:p} size={src_size:#x}) -> {} ({mem:p})",
        if ok { "OK ✓" } else { "FALHOU" }
    ));
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
}

/// STEP-3: forja uma classe com uma PROPRIEDADE Float custom (alias de `src` + 1 prop apendada).
/// Forja a CClass (size = src_size+8) + forja uma CProperty (0x30: type@0x00=Float, name@0x08,
/// parent@0x18, valueOffset@0x20 = src_size → o valor fica DEPOIS dos campos da fonte, sem overlap)
/// + PushBack em props@0x28 + RegisterType. Prova: `cwregprop <novo> <src> <prop>` -> `propdump <novo>`
/// mostra a prop. Confirma que dá pra dar props custom a uma classe forjada (base dos eventos tipados).
pub unsafe fn register_type_with_prop(reg: &Registry, new_name: &str, src_name: &str, prop_name: &str) -> *mut c_void {
    const CCLASS_SIZE: usize = 0x2D0;
    const CPROP_SIZE: usize = 0x30;
    if rtti::sane(reg.class_by_name(new_name)) {
        crate::log(&format!("[reg] register_type_with_prop: '{new_name}' JÁ existe — abortado"));
        return std::ptr::null_mut();
    }
    let src = reg.class_by_name(src_name);
    if !rtti::sane(src) || !crate::gum::is_readable(src as *const c_void, 0x78) {
        crate::log(&format!("[reg] register_type_with_prop: fonte '{src_name}' inválida"));
        return std::ptr::null_mut();
    }
    let vtable = rd_u64(src as *const c_void, 0x00);
    let src_size = ((src as *const u8).add(0x68) as *const u32).read_unaligned();
    let src_align = ((src as *const u8).add(0x74) as *const u32).read_unaligned();
    if vtable == 0 || src_size == 0 || src_size > 0x10000 {
        crate::log("[reg] register_type_with_prop: fonte inválida (vtable/size)");
        return std::ptr::null_mut();
    }
    let ftype = get_type(reg, "Float");
    if !rtti::sane(ftype) {
        crate::log("[reg] register_type_with_prop: tipo 'Float' não resolveu");
        return std::ptr::null_mut();
    }
    let val_off = src_size; // prop APENDADA após os campos da fonte (sem overlap)
    let new_size = src_size + 8; // room p/ o Float (4B + padding)
    let align = if src_align == 0 || src_align > 64 { 8 } else { src_align };
    // 1) forja a CClass
    let mem = rtti::pool_alloc(CCLASS_SIZE, 8);
    if mem.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, CCLASS_SIZE);
    wr_u64(mem, 0x00, vtable);
    wr_u64(mem, 0x18, cname(new_name));
    wr_u32(mem, 0x68, new_size);
    wr_u32(mem, 0x6C, new_size);
    wr_u32(mem, 0x70, 0x02); // native, não-abstrata
    wr_u32(mem, 0x74, align);
    // 2) forja a CProperty (Float @ val_off)
    let cprop = rtti::pool_alloc(CPROP_SIZE, 8);
    if cprop.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(cprop as *mut u8, 0, CPROP_SIZE);
    wr_u64(cprop, 0x00, ftype as u64); // type = Float
    wr_u64(cprop, 0x08, cname(prop_name)); // name
    wr_u64(cprop, 0x18, mem as u64); // parent = a classe forjada
    wr_u32(cprop, 0x20, val_off); // valueOffset (valor vive em obj+val_off)
    // flags@0x28 = 0 (valor inline, sem inValueHolder)
    // 3) PushBack a CProperty na props@0x28 (DynArray vazio → dynarray_push_ptr realoca no pool do jogo)
    let props_arr = (mem as *mut u8).add(0x28) as *mut c_void;
    let pushed = dynarray_push_ptr(props_arr, cprop as u64);
    // 4) RegisterType
    let slot = reg.vtbl_slot(0x80);
    if !rtti::sane(slot) {
        crate::log("[reg] register_type_with_prop: RegisterType@0x80 ilegível");
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void, u32) = std::mem::transmute(slot);
    f(reg.raw(), mem, 0);
    let ok = rtti::sane(reg.class_by_name(new_name));
    crate::log(&format!(
        "[reg] register_type_with_prop '{new_name}' (alias '{src_name}' size={new_size:#x}) + prop '{prop_name}':Float@{val_off:#x} pushed={pushed:?} -> {} ({mem:p})",
        if ok { "OK ✓ (propdump p/ ver a prop)" } else { "FALHOU" }
    ));
    if ok {
        mem
    } else {
        std::ptr::null_mut()
    }
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
    write_bool_ret(out, true);
}

/// `BwmsAutoContinue() -> Bool` (DEV) — retorna true se `~/.bwms-autocontinue` existe.
/// SEGUNDA native real registrada via `register_all` — prova que o registro escala
/// além do smoke (2 funções no RTTI) E que o RETORNO Bool native→redscript é CONSUMIDO
/// numa condicional (o `.reds` de auto-continue gateia o LoadLastCheckpoint nisto).
/// Toggle por marcador SEM recompilar: `touch`/`rm ~/.bwms-autocontinue`.
///
/// GUARDA dead-man's switch (2026-07-12): se `~/.bwms-boot-attempt` sobreviveu de um boot
/// anterior (o lever `BwmsFireStart` disparou da última vez, mas o processo NUNCA chegou num
/// exit() limpo — crash, hang forçado a kill -9, watchdog do próprio jogo, ex.: Low Power Mode
/// do macOS throttlando o motor o bastante pra travar um lock interno dele, nada a ver com o
/// BWMS), suprime o AUTO-LOAD por ESTE boot só (o nível persistido do usuário não muda). O
/// lever continua ativando o save-system normalmente (`tramp_fire_start_state` não tem essa
/// guarda — pular ELE trava o boot pra sempre, ver comentário lá); só o passo mais pesado
/// (carregar o save de verdade) é que fica de fora, então o pior caso vira "cai no menu com
/// CONTINUAR pronto" (igual ao nível 1) em vez de repetir o crash indefinidamente.
/// O check em si roda 1x cedo no on_load (`selfboot::check_stale_boot_attempt`), NÃO aqui —
/// reler o arquivo aqui acharia o marcador que o `tramp_fire_start` DESTA MESMA sessão acabou
/// de escrever, suprimindo em TODO boot em vez de só nos que travaram.
unsafe extern "C" fn tramp_autocontinue(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let on = !crate::selfboot::autocontinue_suppressed_stale_boot()
        && std::env::var("HOME")
            .ok()
            .map(|h| std::path::Path::new(&h).join(".bwms-autocontinue").exists())
            .unwrap_or(false);
    // DIAGNÓSTICO: loga 1x que o reds (OnSaveMetadataReady) chamou a native + o valor lido.
    static LOGGED_AC: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED_AC.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::log(&format!("[autocontinue] BwmsAutoContinue() chamado pelo reds = {on}"));
    }
    write_bool_ret(out, on);
}

// ===== Toggle do auto-continue (pular ATÉ O JOGO: carrega o último save ao chegar no menu). A UI
// redscript liga/desliga; persiste no marcador ~/.bwms-autocontinue, que o BwmsAutoContinue() lê. =====
fn autocontinue_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-autocontinue"))
}
/// `BwmsAutoContinueOn() -> Bool` — cria o marcador (entra no jogo sozinho no próximo boot).
unsafe extern "C" fn tramp_autocontinue_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = autocontinue_marker() {
        let _ = std::fs::write(&p, b"");
    }
    crate::log("[autocontinue] toggle -> LIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsAutoContinueOff() -> Bool` — remove o marcador (só até o menu no próximo boot).
unsafe extern "C" fn tramp_autocontinue_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = autocontinue_marker() {
        let _ = std::fs::remove_file(&p);
    }
    crate::log("[autocontinue] toggle -> DESLIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsFireStartOn() -> Bool` — cria `~/.bwms-fire-start` (zero-input "Até gameplay": a native BwmsFireStart
/// dispara o lever d4=2 no timer da engagement). Ligado pelo nível 2 do seletor "Pular boot".
unsafe extern "C" fn tramp_fire_start_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Ok(h) = std::env::var("HOME") {
        let _ = std::fs::write(std::path::Path::new(&h).join(".bwms-fire-start"), b"");
    }
    crate::log("[firestart] toggle -> LIGADO (via UI, nível Até gameplay)");
    write_bool_ret(out, true);
}
/// `BwmsFireStartOff() -> Bool` — remove `~/.bwms-fire-start` (níveis 0/1 do seletor não disparam o lever).
unsafe extern "C" fn tramp_fire_start_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Ok(h) = std::env::var("HOME") {
        let _ = std::fs::remove_file(std::path::Path::new(&h).join(".bwms-fire-start"));
    }
    crate::log("[firestart] toggle -> DESLIGADO (via UI)");
    write_bool_ret(out, true);
}
/// Path do marcador "dead-man's switch" do lever (ver `boot_attempt_mark`/`boot_attempt_clear`
/// em selfboot.rs). Só HOME (não precisa do fallback /tmp dos outros toggles — é interno,
/// nunca setado manualmente pelo usuário).
fn boot_attempt_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-boot-attempt"))
}
/// `BwmsFireStartState() -> Bool` — true se `~/.bwms-fire-start` está ligado (níveis 1 e 2 do
/// seletor "Pular boot", 2026-07-12: antes só o nível 2/autocontinue ligava o lever — nível 1
/// ("Até o menu") usava um dismiss por evento que NUNCA ativava o save-system de verdade,
/// deixando o menu sem CONTINUAR/lista de saves. bwms-skipintro.reds agora usa ESTA state pra
/// decidir entre o timer do lever (8s) e o fallback de dismiss simples (6s, sem saves).
///
/// SEM guarda de dead-man's switch aqui de propósito: o lever (`BwmsFireStart`, d4=2) é o que
/// ATIVA o save-system nativo — pular ele deixa a state machine do jogo presa em d4=1 pra
/// sempre (sem input real de SPACE, que a splash do bwms esconde), travando a splash em
/// "iniciando a sessão" indefinidamente (achado testando esta mesma guarda: 1ª versão pulava
/// o lever inteiro e travava o boot pra sempre, pior que o crash-loop que devia evitar). A
/// guarda mora em `tramp_autocontinue` — só suprime o AUTO-LOAD (o passo mais pesado/arriscado),
/// nunca a ativação do save-system em si.
unsafe extern "C" fn tramp_fire_start_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let on = std::path::Path::new("/tmp/bwms-fire-start").exists()
        || std::env::var("HOME")
            .ok()
            .map(|h| std::path::Path::new(&h).join(".bwms-fire-start").exists())
            .unwrap_or(false);
    write_bool_ret(out, on);
}
/// `BwmsAcFired()` — o auto-continue disparou (BwmsDoContinue vai carregar o último save). Marca de
/// prova/telemetria: uma linha por boot quando o pular-até-o-jogo acionou. Também alimenta a barra
/// de progresso REAL da splash (ver `selfboot::boot_progress`, 2026-07-12).
unsafe extern "C" fn tramp_ac_fired(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::selfboot::note_autocontinue_fired();
    crate::log("[autocontinue] disparou -> carregando o último save (LoadModdedSave/LoadLastCheckpoint)");
    write_bool_ret(out, true);
}
/// `BwmsTppState() -> Int32` — Skill 1 (VER-O-V, câmera 3ª pessoa autônoma). Lê `~/.bwms-tppcam`:
/// "1" = ligar câmera 3ª pessoa, senão 0. O `bwms-tppcam.reds` (poller DelayCallback) consulta isto
/// a cada 0.25s em gameplay e, na transição, faz `player.QueueEvent(new ActivateTPPRepresentationEvent())`
/// (o `new` do redscript cuida do Handle/refcount — o lado Rust não conseguiria com segurança).
/// O assistente controla o estado ESCREVENDO o marcador pelo canal/bash (sem HID/Acessibilidade).
unsafe extern "C" fn tramp_tpp_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-tppcam"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| if s.trim() == "1" { 1u64 } else { 0 })
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}
/// `BwmsCamBack() -> Int32` — mod "ver corpo" / 3ª pessoa DE VERDADE (sem o crouch do
/// ActivateTPPRepresentation). Lê `~/.bwms-camback` = distância pra trás em CENTÍMETROS (0 =
/// vanilla/1ª pessoa). O poller redscript aplica isso como offset LOCAL do `FPPCameraComponent`
/// (o corpo do V já é renderizado sempre; só a câmera vai pra trás → V aparece em pé, andando
/// normal). O assistente seta o valor e o Perrotta pede "mais/menos" até a distância perfeita —
/// dá pra tunar SÓ trocando o número no marcador, sem recompilar.
unsafe extern "C" fn tramp_cam_back(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-camback"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}
/// `BwmsCamX() -> Int32` — eixo X (lateral) do offset local da câmera, em CM (assinado). Companheiro
/// de `BwmsCamBack` (eixo Y) — juntos deixam o offset 100% ajustável ao vivo, sem recompilar, pra eu
/// achar empiricamente qual eixo é "trás" e qual é "altura" e travar a vista de corpo em pé.
unsafe extern "C" fn tramp_cam_x(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-camx"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}
/// `BwmsCamZ() -> Int32` — eixo Z (altura) do offset local da câmera, em CM (assinado). Ver `BwmsCamX`.
unsafe extern "C" fn tramp_cam_z(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-camz"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}
/// `BwmsForceLook() -> Int32` — TESTE (2026-07-15): sem HID pra olhar pra baixo de verdade, o
/// assistente força a rotação da câmera FPP pra provar/refutar visualmente se o torso (anexado via
/// ActivateTPPRepresentation) aparece ao olhar pra baixo. Lê `~/.bwms-forcelook` ("1"=forçar
/// pitch pra baixo + aplicar offset máximo; senão 0 = solta a câmera, comportamento normal do
/// jogo). Gated/opt-in, só ativo quando o marcador existe — não interfere no controle normal.
unsafe extern "C" fn tramp_force_look(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-forcelook"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| if s.trim() == "1" { 1u64 } else { 0 })
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}
/// `BwmsEquipState() -> Int32` — Skill 2 (EQUIPAR item por código, "fazer aparecer no V" sem HID).
/// Lê `~/.bwms-equip`: um inteiro que seleciona qual roupa de teste equipar (0=nenhuma, 1/2/3 =
/// itens do jogo base pré-mapeados no `bwms-tppcam.reds`; mais tarde vira o ID de um mod). O poller
/// redscript, na transição, monta um `EquipRequest`+`ItemID.FromTDBID` e faz
/// `EquipmentSystem.GetInstance(player).QueueRequest(req)` com `addToInventory=true` (dá E equipa de
/// uma vez). O `new` do redscript cuida do Handle/refcount — o lado Rust só passa o número.
unsafe extern "C" fn tramp_equip_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-equip"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}
/// `BwmsEquipLog(step: Int32)` — traça onde o poller de equipar chega (diagnóstico da Skill 2):
/// 1=disparou c/ sel>0 · 2=player ok · 3=EquipmentSystem ok · 4=QueueRequest chamado · 10+=readback.
unsafe extern "C" fn tramp_equip_log(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let step = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    crate::log(&format!("[skill2] equip step {step}"));
    write_bool_ret(out, true);
}
/// `BwmsInvState() -> Int32` — Skill 1b (VER o V no preview 3D do inventário). Lê `~/.bwms-invpreview`:
/// 1 = abrir o inventário (mostra o modelo de corpo inteiro do V com a roupa), 0 = fechar. Rota
/// no-HID via `GameInstance.GetUISystem(game).QueueEvent(inkMenuInstance_SpawnEvent)`.
unsafe extern "C" fn tramp_inv_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-invpreview"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}
/// `BwmsEquipCheck() -> Int32` — gatilho de READBACK do equip (prova definitiva por código): lê
/// `~/.bwms-equipcheck`. Quando vira 1, o poller lê o item ativo de cada slot de roupa e loga.
unsafe extern "C" fn tramp_equip_check(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-equipcheck"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}
/// `BwmsEquipReadback(area: Int32, id: TweakDBID) -> Bool` — o poller passa o TweakDBID do item
/// equipado em cada slot; aqui a gente loga o hash + o nome se conhecido. Prova que a roupa
/// equipou de verdade (sem depender de câmera). Compara com os hashes dos itens de teste.
unsafe extern "C" fn tramp_equip_readback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let area = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    // hashes dos itens de teste (bwms_hashes::tweak_db_id) pra reconhecer no log.
    let known = [
        ("Items.GOG_DLC_Jacket_Legendary", bwms_hashes::tweak_db_id("Items.GOG_DLC_Jacket_Legendary")),
        ("Items.Fixer_01_Set_TShirt", bwms_hashes::tweak_db_id("Items.Fixer_01_Set_TShirt")),
        ("Items.Coat_04_rich_02_Crafting", bwms_hashes::tweak_db_id("Items.Coat_04_rich_02_Crafting")),
    ];
    let name = known.iter().find(|(_, h)| *h == id).map(|(n, _)| *n).unwrap_or("(outro/vazio)");
    // area = código NOSSO passado pelo reds (1=OuterChest,2=InnerChest,3=Legs,4=Feet,5=Head,6=Face)
    let aname = match area {
        1 => "OuterChest",
        2 => "InnerChest",
        3 => "Legs",
        4 => "Feet",
        5 => "Head",
        6 => "Face",
        _ => "?",
    };
    crate::log(&format!("[skill2] slot {aname} = item {id:#018x} {name}"));
    write_bool_ret(out, true);
}
/// `BwmsMenuReadySplashOff() -> Bool` — nível 1 ("Até o menu com saves"): os saves ficaram
/// prontos (lever disparou, gate savesReady+metaReady bateu) mas BwmsAutoContinue()==false, então
/// o jogo NÃO vai carregar sozinho — vai FICAR no menu. Sem auto-load nunca se chega em phase==5,
/// que é o gatilho normal do grace-period da splash (selfboot.rs) — sem isto ela ficaria acesa
/// pra sempre. Desliga direto, na hora que o menu fica genuinamente pronto (CONTINUAR funcionando).
unsafe extern "C" fn tramp_menu_ready_splash_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::selfboot::boot_splash_off();
    crate::log("[firestart] saves prontos, autocontinue OFF -> parou no menu com saves (splash desligada)");
    write_bool_ret(out, true);
}
// ===== Diagnóstico do auto-continue (só LOGA — zero mudança de comportamento) p/ pinpointar por que
// o pular-até-o-jogo não dispara sozinho, sem chutar (o chute anterior deu hang). =====
unsafe extern "C" fn tramp_dbg_meta0(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[ac-dbg] OnSaveMetadataReady saveIndex==0 && isValid → m_bwmsMetaReady=true");
    write_bool_ret(out, true);
}
unsafe extern "C" fn tramp_dbg_hast(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[ac-dbg] BwmsTryContinue: HasLastCheckpoint=TRUE (todas as condições p/ carregar OK)");
    write_bool_ret(out, true);
}
unsafe extern "C" fn tramp_dbg_hasf(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[ac-dbg] BwmsTryContinue: HasLastCheckpoint=FALSE (bloqueia o load)");
    write_bool_ret(out, true);
}
/// `BwmsDbgTry(flags: Int32) -> Bool` — diagnóstico DECISIVO: o reds chama isto no TOPO de
/// BwmsTryContinue TODA vez (antes de qualquer gate), com um bitmask do estado que a função vê.
/// bit0=savesReady bit1=metaReady bit2=savesCount>0 bit3=isModded bit4=continued. Se aparecer só 1x
/// = a 2ª chamada (pós-OnSaveMetadataReady) não roda; se 2x com metaReady=false na 2ª = o campo não
/// é lido. Espelha o gate REAL do jogo (m_savesCount>0), não HasLastCheckpoint. Zero mudança de comportamento.
unsafe extern "C" fn tramp_dbg_try(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let f = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    crate::log(&format!(
        "[ac-dbg] TryContinue ENTROU flags={f} (savesReady={} metaReady={} savesCount>0={} isModded={} continued={})",
        f & 1 != 0, f & 2 != 0, f & 4 != 0, f & 8 != 0, f & 16 != 0
    ));
    write_bool_ret(out, true);
}
/// `BwmsDbgSkip(stage: Int32) -> Bool` — diagnóstico do skip da engagement. O reds chama em pontos-chave
/// pra pinpointar ONDE a flakiness trava (o menu nem inicializa). Convenção de stage:
///   10 = OnHandleEngagementScreen ENTROU, evt.show original=TRUE   11 = ...show original=FALSE
///   20 = forcei evt.show=false (skip on)                          21 = após wrappedMethod (else-branch rodou)
///   30 = OnEnterScenario(SingleplayerMenu) ENTROU, prevScenario==None   31 = ...prevScenario!=None
///   40 = OnAdditionalContentDataReloadProgress (conteúdo ainda carregando) — arg = progress*100
unsafe extern "C" fn tramp_dbg_skip(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let s = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    crate::log(&format!("[skip-dbg] estágio={s}"));
    write_bool_ret(out, true);
}

/// `BwmsEngagementOn() -> Bool` — o redscript (EngagementScreenGameController.OnInitialize) chama
/// isto ao ENTRAR na tela "APERTE E PARA CONTINUAR" → o present passa a auto-injetar o proceed "E"
/// (o proceed é 100% nativo, sem gancho redscript). É o sinal PRECISO da engagement do boot.
unsafe extern "C" fn tramp_engagement_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::overlay::set_engagement_active(true);
    write_bool_ret(out, true);
}

/// `BwmsEngagementOff() -> Bool` — chamado no OnUninitialize da engagement (saiu pro menu) → PARA
/// a injeção antes do menu (senão o "E" ativaria um item do menu).
unsafe extern "C" fn tramp_engagement_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::overlay::set_engagement_active(false);
    // Menu chegou → esconde o splash de boot (o menu tem arte própria). Feito AQUI (handler redscript,
    // dispara no OnUninitialize da engagement) p/ o splash-off NÃO depender da auto-proceed, que sai do
    // build público (feature "autoproceed"). O bink-skip + o splash seguem valendo sem CGEvent.
    // EXCEÇÃO (2026-07-12): no modo "Até a gameplay" (~/.bwms-fire-start ligado) o menu é só uma
    // parada de passagem — a splash tem que cobrir até a RUA (phase=5), senão revela o menu/tela
    // "aperte espaço" pós-load no meio do que devia ser 100% automático. phase_skip_getter cuida
    // do dismiss nesse modo (phase==5).
    let fire_start = std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-fire-start").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-fire-start").exists();
    if !fire_start {
        crate::selfboot::boot_splash_off();
    }
    write_bool_ret(out, true);
}

// ===== Toggle do skip-intro (a UI redscript liga/desliga; persiste no marcador ~/.bwms-skipintro,
// que o bink-skip + auto-proceed leem no boot) =====
fn skipintro_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-skipintro"))
}
/// `BwmsSkipIntroOn() -> Bool` — cria o marcador (auto-skip LIGADO no próximo boot).
unsafe extern "C" fn tramp_skipintro_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = skipintro_marker() {
        let _ = std::fs::write(&p, b"");
    }
    crate::log("[skipintro] toggle -> LIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsSkipIntroOff() -> Bool` — remove o marcador (auto-skip DESLIGADO no próximo boot).
unsafe extern "C" fn tramp_skipintro_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = skipintro_marker() {
        let _ = std::fs::remove_file(&p);
    }
    crate::log("[skipintro] toggle -> DESLIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsSkipIntroState() -> Bool` — true se o auto-skip está ligado (marcador existe). Pro toggle
/// da UI mostrar o estado atual.
unsafe extern "C" fn tramp_skipintro_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    // ROBUSTO: checa /tmp/bwms-skipintro OU ~/.bwms-skipintro (mesma lógica do selfboot::skipintro_enabled,
    // que o bink-skip/auto-proceed usam). A versão antiga só olhava ~/ e às vezes lia false cedo no boot.
    let on = std::path::Path::new("/tmp/bwms-skipintro").exists()
        || skipintro_marker().map(|p| p.exists()).unwrap_or(false);
    write_bool_ret(out, on);
}

/// `BwmsSessionAdvance() -> Bool` — AVANÇA o estado da sessão pregame um passo (fase v -> v+1), o efeito
/// colateral do SPACE nativo que ARMA o save-system, SEM input/CGEvent/Acessibilidade. O redscript chama
/// isto no SINAL DE FIM-DE-LOAD (OnAdditionalContentDataReloadProgress = 1.0), não por timer. Gate por
/// `~/.bwms-session-advance` (OFF por padrão) DENTRO de force_session_advance. Devolve true se avançou.
unsafe extern "C" fn tramp_session_advance(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let fired = crate::selfboot::force_session_advance();
    write_bool_ret(out, fired);
}

// ===== Toggle do CPVR (VR) — DEV-ONLY (feature `cpvr`). A UI redscript liga/desliga; persiste no
// marcador `~/.bwms-cpvr`, que os `CpvrXxxEnabled()` dos reds do CPVR leem p/ ativar/no-opar o modo
// VR. Native INERTE (só marcador de arquivo) — zero Frida, zero offset. O modo CPVR em si (reds do
// CPVR + gadget Frida + stream) NUNCA entra no build público; estas natives ficam atrás de `cpvr`. =====
#[cfg(feature = "cpvr")]
fn cpvr_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-cpvr"))
}
/// Estado do MODO CPVR (0 Desligado / 1 v1 leve / 2 v1.1 câmera / 3 v2.0 Alyx). Fonte = arquivo
/// `~/.bwms-cpvr-mode` (conteúdo = dígito). O marcador booleano `~/.bwms-cpvr` é DERIVADO (presente
/// iff modo>0) — o gate `.cpvr-ingame` e `CpvrStereoEnabled` dependem dele.
#[cfg(feature = "cpvr")]
fn cpvr_mode_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-cpvr-mode"))
}
#[cfg(feature = "cpvr")]
fn cpvr_read_mode() -> i32 {
    cpvr_mode_marker()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(0)
        .clamp(0, 3)
}
#[cfg(feature = "cpvr")]
fn cpvr_write_mode(mode: i32) {
    let m = mode.clamp(0, 3);
    if let Some(p) = cpvr_mode_marker() {
        let _ = std::fs::write(&p, m.to_string().as_bytes());
    }
    // sincroniza o booleano ~/.bwms-cpvr (presente iff modo>0)
    if let Some(p) = cpvr_marker() {
        if m > 0 {
            let _ = std::fs::write(&p, b"");
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}
/// `BwmsCpvrOn() -> Bool` — cria o marcador (modo VR LIGADO).
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = cpvr_marker() { let _ = std::fs::write(&p, b""); }
    crate::log("[cpvr] toggle -> LIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsCpvrOff() -> Bool` — remove o marcador (modo VR DESLIGADO).
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = cpvr_marker() { let _ = std::fs::remove_file(&p); }
    crate::log("[cpvr] toggle -> DESLIGADO (via UI)");
    write_bool_ret(out, true);
}
/// `BwmsCpvrState() -> Bool` — true se o modo VR está ligado (marcador existe). Os reds do CPVR
/// chamam isto em `CpvrStereoEnabled()/CpvrHudEnabled()/CpvrNoAnimsEnabled()` p/ ligar/no-opar.
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let on = cpvr_marker().map(|p| p.exists()).unwrap_or(false);
    write_bool_ret(out, on);
}
/// `BwmsCpvrMode() -> Int32` — 0 Desligado / 1 v1 / 2 v1.1 / 3 v2.0. O seletor do menu pinta por isto.
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_mode(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_i32_ret(out, cpvr_read_mode());
}
/// `BwmsCpvrModeNext() -> Bool` — avança o modo (clamp 0..3). Seta direita do seletor.
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_mode_next(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let m = (cpvr_read_mode() + 1).clamp(0, 3);
    cpvr_write_mode(m);
    crate::log(&format!("[cpvr] modo -> {m} (via UI)"));
    write_bool_ret(out, true);
}
/// `BwmsCpvrModePrev() -> Bool` — recua o modo (clamp 0..3). Seta esquerda do seletor.
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_mode_prev(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let m = (cpvr_read_mode() - 1).clamp(0, 3);
    cpvr_write_mode(m);
    crate::log(&format!("[cpvr] modo -> {m} (via UI)"));
    write_bool_ret(out, true);
}
/// `BwmsCpvrStereoPing() -> Bool` — DIAGNÓSTICO do v2.0: o CpvrStereoOffsetCamera (redscript) chama
/// isto por-frame; logamos throttled p/ PROVAR que o hook OnUpdate dispara em gameplay (se o offset
/// não aparecer visualmente MAS este log sair = a câmera reverte no tick nativo = a "parede" real).
/// Chamado no TOPO do OnUpdate (antes do gate/player/cam). Se ISTO logar em gameplay → o hook
/// LocomotionEventsTransition.OnUpdate DISPARA e a falha do v2.0 é downstream (gate/player/cam ou
/// SetLocalTransform revertido). Se NEM isto logar → a classe é a errada p/ o estado (próximo passo).
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_stereo_ping(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n == 0 || n % 180 == 0 {
        crate::log(&format!("[cpvr-stereo] OnUpdate TOPO rodou (chamada #{n}) — o HOOK DISPARA"));
    }
    write_bool_ret(out, true);
}
/// `BwmsCpvrIsV2() -> Bool` — TRUE se o modo é v2.0 (3). Gate do CpvrStereoEnabled via BOOL (proven),
/// contornando a dúvida do retorno Int32 do BwmsCpvrMode (proto Float pode reinterpretar os bytes).
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_is_v2(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, cpvr_read_mode() == 3);
}
/// Chamado LOGO ANTES do SetLocalTransform (após gate+player+cam). Se ISTO logar mas a vista não
/// deslocar → SetLocalTransform RODA mas é REVERTIDO pelo tick nativo = a parede real do make-or-break.
/// Se NÃO logar (mas o topo sim) → o gate/player/cam bloqueia antes do apply.
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_cpvr_apply_ping(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n == 0 || n % 180 == 0 {
        crate::log(&format!("[cpvr-stereo] vai chamar SetLocalTransform (chamada #{n}) — passou gate+player+cam"));
    }
    write_bool_ret(out, true);
}

/// `BwmsCamScan(x: Float, y: Float, z: Float) -> Bool` — MECANISMO 7 via NOSSO tool nativo (não Frida).
/// Scanner de memória crua in-process (mach_vm): acha TODAS as cópias da inverse-view da câmera com
/// col3 ~ (x,y,z) e patcha col3 += right*3m (COW). O redscript passa a pos VIVA da câmera por frame.
/// Se o render deslocar → existe cópia CPU-acessível que o shader lê = câmera ALCANÇÁVEL. Se não →
/// confirma GPU-Private de vez (scan completo, patchou tudo, zero shift). Gate por mode==3 (v2).
#[cfg(feature = "cpvr")]
unsafe extern "C" fn tramp_camscan(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let x = f32::from_bits(args.first().map(|(v, _)| *v).unwrap_or(0) as u32);
    let y = f32::from_bits(args.get(1).map(|(v, _)| *v).unwrap_or(0) as u32);
    let z = f32::from_bits(args.get(2).map(|(v, _)| *v).unwrap_or(0) as u32);
    let np = if cpvr_read_mode() == 3 { crate::camscan::cam_scan(x, y, z, 3.0) } else { 0 };
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n == 0 || n % 180 == 0 {
        crate::log(&format!(
            "[camscan] BwmsCamScan(#{n}) alvo=[{x:.0},{y:.0},{z:.0}] patchou {np} copias CPU-acessiveis"
        ));
    }
    write_bool_ret(out, np > 0);
}

/// Versão que NÓS reportamos como "Codeware compatível" — fonte única compartilhada entre
/// `Version()` (devolve isto) e `Require(version)` (compara contra isto). Subir isto quando
/// mais superfície da API real do Codeware for implementada.
const CODEWARE_VERSION: &str = "1.0.0";

/// `Codeware.Version() -> String` (smoke do Facade).
unsafe extern "C" fn tramp_version(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log("[reg] >>> Codeware.Version chamado");
    if !out.is_null() {
        std::ptr::write_bytes(out as *mut u8, 0, 0x20);
        rtti::red_string_write_inline(out as *mut u8, CODEWARE_VERSION);
    }
}

/// `Codeware.Require(version) -> Bool` — **cw-version-semver (2026-07-13): comparação SEMVER
/// real**, não mais sempre-true. Lê o arg `version: String` (mesma via já provada em
/// `read_params_consuming_with_strings`, usada por `Utils/Hash.reds`/`Number.reds`), parseia
/// major.minor.patch (`api::parse_semver_triplet`, já usado pelo `BwmsApi.semver_satisfies` do
/// plugin — reuso, não duplicação) e compara contra `CODEWARE_VERSION` (o que REALMENTE
/// implementamos). `actual >= required` → true, senão false — mods que pedem uma versão além do
/// que cobrimos agora recebem `false` de verdade (antes sempre recebiam `true`, mascarando
/// incompatibilidade real).
unsafe extern "C" fn tramp_require(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let requested = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::api::parse_semver_triplet(CODEWARE_VERSION) >= crate::api::parse_semver_triplet(&requested);
    crate::log(&format!("[reg] >>> Codeware.Require('{requested}') vs nosso '{CODEWARE_VERSION}' -> {ok}"));
    write_bool_ret(out, ok);
}

/// `BwmsEmit() -> Bool` (IA Fase 0) — o redscript chama isto p/ ENFILEIRAR um evento
/// pro processo externo de IA. NÃO bloqueia (só escreve um arquivo e retorna) — a regra
/// arquitetural: a native roda na thread do jogo, o LLM lento mora num processo separado.
unsafe extern "C" fn tramp_bwms_emit(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::ai::emit_event();
    write_bool_ret(out, true);
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

/// Registra uma native global COM ARGS a partir de NOMES DE TIPO (ex.: `["Float","CName"]`) —
/// compõe os params (`compose_params_from_types`) e registra (`register_global_composed`). É a via
/// que a API C-ABI expõe pros plugins (register_native_argful): o plugin não precisa de proto/params
/// crus, só dos nomes dos tipos. `proto_vtable` = vtable de uma global existente (Cos/Sin/...).
pub unsafe fn register_argful_by_types(
    reg: &Registry,
    proto_vtable: *mut c_void,
    type_names: &[&str],
    full: &str,
    short: &str,
    handler: NativeHandler,
) -> bool {
    let params = compose_params_from_types(reg, type_names);
    if params.0 == 0 && !type_names.is_empty() {
        return false; // composição falhou (tipo desconhecido / pool)
    }
    register_global_composed(reg, proto_vtable, params, full, short, handler)
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
    write_bool_ret(out, ok);
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
    write_bool_ret(out, ok);
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
    write_bool_ret(out, ok);
}

/// Deref da cadeia do input-context (workflow 2): a=*(SM+0x70); b=*(a+0x288); c=*(b+0x18); ctx=*(c+0x1b0).
/// O byte gate da ação 'Start' vive em ctx+0x572. Cada nível valida legibilidade; null em qualquer passo → null.
unsafe fn deref_input_ctx(sm: *const u8) -> *mut u8 {
    let rd = |p: *const u8| -> *const u8 {
        if p.is_null() || !crate::gum::is_readable(p as *const c_void, 8) {
            std::ptr::null()
        } else {
            (p as *const *const u8).read()
        }
    };
    let a = rd(sm.add(0x70));
    if a.is_null() { return std::ptr::null_mut(); }
    let b = rd(a.add(0x288));
    if b.is_null() { return std::ptr::null_mut(); }
    let c = rd(b.add(0x18));
    if c.is_null() { return std::ptr::null_mut(); }
    rd(c.add(0x1b0)) as *mut u8
}

/// READ-ONLY: dumpa os bytes da state-machine da engagement p/ VALIDAR o objeto + o state ANTES de
/// implementar a LEVER C (0x103f71e4c). `target` = a EngagementScreenGameController (= o SM candidato,
/// o `this` do tick 0x103f709f4). Loga [SM+0xd4]=state / +0xe0=sub-load / +0xdc=timer f32 / +0x88=pending
/// / +0xe8=guard / +0xfe=sinal-Start / +0xfd=gate-user / +0x84=phase. ZERO ESCRITA (sem risco de crash).
/// Se d4∈{1,4,5,6,7} => o `this` do redscript É o SM. Ver notes/ENGAGEMENT-STATE-MACHINE-2026-07-06.md.
unsafe extern "C" fn tramp_engdump(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let _args = rtti::read_params_consuming(func, frame); // consome o handle (o `this` do redscript = controller de UI, objeto ERRADO — ignorado)
    // O SM real = GAME_SESSION_DESC (o `this` do getter 0x103f5ec74, provado d4=1/fd=1 no engagement). Dump READ-ONLY.
    let sm = crate::selfboot::GAME_SESSION_DESC.load(std::sync::atomic::Ordering::Relaxed);
    let mut ok = false;
    if !sm.is_null() && crate::gum::is_readable(sm as *const c_void, 0x108) {
        let b = |off: usize| (sm.add(off) as *const u8).read();
        let timer = f32::from_bits((sm.add(0xdc) as *const u32).read());
        let pending = (sm.add(0x88) as *const u32).read();
        crate::log(&format!(
            "[engdump] SM={sm:p} d4(state)={} e0(subload)={} dc(timer)={:.2} 88(pending)={} e8(guard)={} fe(Start)={} fd(gate)={} 84(phase)={}",
            b(0xd4), b(0xe0), timer, pending, b(0xe8), b(0xfe), b(0xfd), b(0x84) as i8
        ));
        // VALIDAÇÃO da LEVER (workflow 2, read-only): slot vt+0x188 (o método que o 'Start' invoca —
        // deve cair em static 0x103f70000..0x103f72000) + o gate ctx+0x572 (deve ser 0 no repouso).
        let vt = (sm as *const *const u8).read();
        let slot = if !vt.is_null() && crate::gum::is_readable(vt as *const c_void, 0x190) {
            (vt.add(0x188) as *const *const u8).read()
        } else {
            std::ptr::null()
        };
        let ctx = deref_input_ctx(sm);
        let gate: i32 = if !ctx.is_null() && crate::gum::is_readable(ctx as *const c_void, 0x573) {
            (ctx.add(0x572) as *const u8).read() as i32
        } else {
            -1
        };
        crate::log(&format!(
            "[engdump2] vt={vt:p} slot[+0x188]=static {:#x} ctx={ctx:p} gate[ctx+0x572]={gate}",
            crate::un_rebase(slot as *const c_void)
        ));
        ok = true;
    } else {
        crate::log(&format!("[engdump] GAME_SESSION_DESC={sm:p} não capturado/ilegível ainda"));
    }
    write_bool_ret(out, ok);
}

/// LEVER (verifier FIX-2): chama o PROCEED inteiro `0x103f70e10(SM)` — a fn que o tick roda p/ o
/// proceed (contém guards próprios: [g+0x2a8/2a9], [SM+0xe8]==0; faz o TRIO 0x103f5ea40 → broadcast do
/// content-reload). SM = GAME_SESSION_DESC (CONFIRMADO = o objeto certo; ≠ [desc+0x60] do 1º rascunho
/// refutado). Escrever o byte de state deadlocka (não dispara o reload REAL); a fn de proceed deveria.
/// GATE `~/.bwms-proceed-native` (OFF por padrão = read-only seguro) + 1x/sessão + guardas duras.
/// Ver notes/ENGAGEMENT-STATE-MACHINE-2026-07-06.md.
static PROCEED_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const PROCEED_FN_VM: u64 = 0x1_03f7_0e10;
unsafe extern "C" fn tramp_proceed(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    use std::sync::atomic::Ordering;
    let func = current_native_func();
    let _args = rtti::read_params_consuming(func, frame);
    let on = std::env::var("HOME").ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-proceed-native").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-proceed-native").exists();
    let sm = crate::selfboot::GAME_SESSION_DESC.load(Ordering::Relaxed);
    if sm.is_null() || !crate::gum::is_readable(sm as *const c_void, 0x108) {
        crate::log(&format!("[proceed] abortado: SM={sm:p} ilegível"));
        return write_bool_ret(out, false);
    }
    let state = (sm.add(0xd4) as *const u8).read();
    let phase = (sm.add(0x84) as *const i8).read();
    let guard = (sm.add(0xe8) as *const u8).read();
    if !on {
        crate::log(&format!("[proceed] gate OFF (~/.bwms-proceed-native). SM={sm:p} state={state} phase={phase} guard={guard} (só leitura)"));
        return write_bool_ret(out, false);
    }
    // GUARDAS DURAS: só na engagement (phase==1), state de repouso são (1 ou 4), guard livre (e8==0), 1x.
    if phase != 1 || !(state == 1 || state == 4) || guard != 0
        || PROCEED_DONE.swap(true, Ordering::Relaxed)
    {
        crate::log(&format!("[proceed] guardas barraram: state={state} phase={phase} guard={guard} done=(1x). sem chamada"));
        return write_bool_ret(out, false);
    }
    let f: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(PROCEED_FN_VM));
    crate::log(&format!("[proceed] CHAMANDO 0x103f70e10(SM={sm:p}) state={state} phase={phase} — proceed nativo, sem HID"));
    f(sm);
    let s2 = (sm.add(0xd4) as *const u8).read();
    let p2 = (sm.add(0x84) as *const i8).read();
    crate::log(&format!("[proceed] RETORNOU: state {state}->{s2} phase {phase}->{p2} (se avançou = reload disparando)"));
    write_bool_ret(out, true);
}

/// LEVER (workflow 2, confirmada por 2 céticos): setar o byte gate da ação 'Start' em `ctx+0x572=1` —
/// no próximo frame a body 0x103f85164 lê !=0 e chama o slot [SM_vtable+0x188] (o consumidor do 'Start')
/// → d4 1→2 → pedido de reload assíncrono → gameplay. É injetar a ação no ponto EXATO do input, sem HID,
/// sem forjar ActionEvent. GATE `~/.bwms-fire-start` (OFF por padrão) + 1x + guardas (d4==1, phase==1, e8==0).
static FIRE_START_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
unsafe extern "C" fn tramp_fire_start(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    use std::sync::atomic::Ordering;
    let func = current_native_func();
    let _args = rtti::read_params_consuming(func, frame);
    let on = std::env::var("HOME").ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-fire-start").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-fire-start").exists();
    let sm = crate::selfboot::GAME_SESSION_DESC.load(Ordering::Relaxed);
    if sm.is_null() || !crate::gum::is_readable(sm as *const c_void, 0x108) {
        crate::log(&format!("[firestart] abortado: SM={sm:p} ilegível"));
        return write_bool_ret(out, false);
    }
    let state = (sm.add(0xd4) as *const u8).read();
    let phase = (sm.add(0x84) as *const i8).read();
    let guard = (sm.add(0xe8) as *const u8).read();
    let ctx = deref_input_ctx(sm);
    let gate_now: i32 = if !ctx.is_null() && crate::gum::is_readable(ctx as *const c_void, 0x573) {
        (ctx.add(0x572) as *const u8).read() as i32
    } else {
        -1
    };
    if !on {
        crate::log(&format!("[firestart] gate OFF (~/.bwms-fire-start). state={state} phase={phase} ctx={ctx:p} gate572={gate_now} (só leitura)"));
        return write_bool_ret(out, false);
    }
    if state != 1 || phase != 1 || guard != 0 || ctx.is_null() || gate_now < 0
        || FIRE_START_DONE.swap(true, Ordering::Relaxed)
    {
        crate::log(&format!("[firestart] guardas barraram: state={state} phase={phase} guard={guard} ctx={ctx:p} gate572={gate_now} done=(1x)"));
        return write_bool_ret(out, false);
    }
    // O WRITE (v2): d4=2 → o state-2 pump 0x103f72090 dispara o reload (0x101edea5c) — o pedido assíncrono
    // que dirige e0 e emite 610/800. O reload dispara ANTES do uso de [SM+0xb8], então payload nulo não bloqueia.
    let b8 = (sm.add(0xb8) as *const u64).read();
    (sm.add(0xd4) as *mut u8).write(2);
    let _ = ctx;
    // Arma o dead-man's switch (ver tramp_fire_start_state): só o exit() hookado (selfboot::
    // exit_replacement) limpa este marcador — se o processo não chegar lá (crash/hang/kill -9),
    // o próximo boot detecta e pula o lever uma vez, em vez de crash-loopar pra sempre.
    if let Some(m) = boot_attempt_marker() {
        let _ = std::fs::write(&m, b"");
    }
    crate::log(&format!("[firestart] ESCREVI d4=2 (SM={sm:p}) [SM+0xb8]={b8:#x} — pump deve chamar 0x101edea5c (reload)"));
    write_bool_ret(out, true);
}

/// Versão CHAMÁVEL do lever (não-native): o dylib dispara DIRETO quando o getter (selfboot::
/// phase_skip_getter) vê a engagement em repouso, sem depender do timer redscript OnBwmsFireStart
/// (que pós-reboot não armava — o @wrapMethod OnInitialize não disparava, eng=false). Mesmo gate +
/// write + dead-man's switch do tramp_fire_start. Compartilha FIRE_START_DONE (dispara 1x por qualquer
/// caminho). Retorna true SÓ se ESCREVEU d4=2.
pub(crate) unsafe fn fire_lever_direct() -> bool {
    use std::sync::atomic::Ordering;
    let on = std::env::var("HOME").ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-fire-start").exists())
        .unwrap_or(false)
        || std::path::Path::new("/tmp/bwms-fire-start").exists();
    if !on {
        return false;
    }
    let sm = crate::selfboot::GAME_SESSION_DESC.load(Ordering::Relaxed);
    if sm.is_null() || !crate::gum::is_readable(sm as *const c_void, 0x108) {
        return false;
    }
    let state = (sm.add(0xd4) as *const u8).read();
    let phase = (sm.add(0x84) as *const i8).read();
    let guard = (sm.add(0xe8) as *const u8).read();
    let ctx = deref_input_ctx(sm);
    let gate_now: i32 = if !ctx.is_null() && crate::gum::is_readable(ctx as *const c_void, 0x573) {
        (ctx.add(0x572) as *const u8).read() as i32
    } else {
        -1
    };
    if state != 1 || phase != 1 || guard != 0 || ctx.is_null() || gate_now < 0
        || FIRE_START_DONE.swap(true, Ordering::Relaxed)
    {
        return false;
    }
    let b8 = (sm.add(0xb8) as *const u64).read();
    (sm.add(0xd4) as *mut u8).write(2);
    if let Some(m) = boot_attempt_marker() {
        let _ = std::fs::write(&m, b"");
    }
    crate::log(&format!(
        "[firestart-direto] ESCREVI d4=2 (SM={sm:p}) [SM+0xb8]={b8:#x} — sem timer redscript, gate572={gate_now}"
    ));
    true
}

// ===== CallbackSystem (lite): registry event→callbacks + RegisterCallback + fire/dispatch =====
/// (event_hash, target_ptr_as_usize, function_hash). V1: ptr direto (válido enquanto o alvo vive;
/// pra robustez = wref, futuro). É a registry do Codeware CallbackSystem em Rust.
static CALLBACKS: Mutex<Vec<(u64, usize, u64)>> = Mutex::new(Vec::new());

/// Estado POR-INSTÂNCIA de um `CallbackSystemHandler` real (2026-07-18, `cw-callback-handler`
/// fechado — ver `register_callbacksystemhandler`). Chave = ponteiro da instância (`usize`, o
/// mesmo padrão de `CALLBACKS`). `targets` = lista de ponteiros de `CallbackSystemTarget`
/// adicionados via `AddTarget` — o "filtro de target" É essa lista (lista vazia = sem filtro,
/// aceita qualquer alvo; não-vazia = só aceita alvos presentes nela — mesmo esquema documentado
/// no `gaps-revised.json`). `event_hash`/`listener`/`fn_hash` espelham a entrada correspondente
/// em `CALLBACKS`, pra `Unregister()` conseguir remover a entrada certa de lá.
struct HandlerState {
    targets: Vec<usize>,
    run_mode: i32,
    lifetime: i32,
    registered: bool,
    event_hash: u64,
    listener: usize,
    fn_hash: u64,
}
static HANDLER_STATES: Mutex<Vec<(usize, HandlerState)>> = Mutex::new(Vec::new());

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
    write_bool_ret(out, ok);
}

/// Emite um evento PASSANDO ARGS pro callback (= o evento carrega dados, ex. a tecla no input).
/// Despacha `target.function(args...)` via resolve_in_class + call_func. Devolve quantos despachou.
/// É o que os CONTROLLERS chamam quando a função de jogo hookada dispara.
pub unsafe fn fire_event_args(event_name: &str, args: &[rtti::Arg]) -> usize {
    fire_event_args_by_hash(cname(event_name), event_name, args)
}

/// Núcleo de `fire_event_args`, mas recebendo o hash JÁ CALCULADO — usado por chamadores que já
/// têm o CName cru (ex. `CallbackSystem.DispatchEventAs`, que recebe o hash como arg nativo e
/// evitaria um round-trip hash→string→hash desnecessário se re-chamasse `fire_event_args`).
pub unsafe fn fire_event_args_by_hash(eh: u64, event_name: &str, args: &[rtti::Arg]) -> usize {
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

// ===== `CallbackSystem` NATIVO real (cw-callbacksystem-rtti, 2026-07-13) ============================
// Mesmo fix de timing/parent que fechou a Facade (parent-pointer explícito + fullName BARE nos
// métodos), aplicado a uma classe INSTANCIÁVEL (não abstract) — porque `GetCallbackSystem()`
// precisa devolver um objeto de verdade. A instância é construída PREGUIÇOSAMENTE (só na 1ª
// chamada de `GetCallbackSystem`, já em gameplay normal) — o forge da classe + registro dos
// métodos continua rodando cedo (mesma janela seguro-por-construção da Facade), sem tentar
// `new_object` durante a janela frágil do bind. Ver `register_callbacksystem` pro forge completo
// e o escopo exato (o que é funcional vs. stub) de cada um dos 7 métodos.
static CBS_INSTANCE: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// `CallbackSystem.RegisterCallback(eventName, target, function, opt sticky) -> ref<CallbackSystemHandler>`
/// — FUNCIONAL (registry) + devolve **null** (ver ACHADO 2026-07-18 abaixo — NÃO é mais o mesmo
/// "deferido" de antes, é um bloqueador NOVO e mais fundo, isolado nesta mesma sessão).
///
/// **TENTATIVA 2026-07-18 (`cw-callback-handler`, mesma sessão que fechou `GetService`):**
/// construir uma instância REAL via `rtti::new_object("CallbackSystemHandler")` e devolvê-la
/// (permitindo `RegisterCallback(...).AddTarget(...).SetLifetime(...)` encadeado de verdade) —
/// TESTADO AO VIVO, 2 tentativas, **AMBAS CRASHARAM** num ponto NOVO, diferente do crash de
/// 2026-07-15 (que já está fechado — forjar a classe e registrar os 6 métodos NÃO crasha mais,
/// confirmado nos 2 boots desta tentativa). O crash agora é DEPOIS: quando o REDSCRIPT COMPILADO
/// tenta LIBERAR (refcount release) o `ref<CallbackSystemHandler>` devolvido, ao sair de escopo.
/// RE ao vivo (crash-report `.ips`, 2 boots): a rotina de release que o compilador injeta no
/// teardown de todo frame com locais `ref<>` (achada em `0x1021048c4`, link vmaddr) lê
/// `[slot+0x08]` como um PONTEIRO PRO BLOCO DE REFCOUNT do handle e, se não-nulo, faz um
/// DECREMENTO ATÔMICO nele (`ldaddal`) — ou seja, **um valor `ref<T>` em bytecode compilado é uma
/// ESTRUTURA DE 16 BYTES** (`[+0x00]`=ponteiro do objeto, `[+0x08]`=ponteiro pro refcount block),
/// não um ponteiro cru de 8 bytes. 1ª tentativa (`write_uint_ret`, só 8 bytes) crashou com SIGBUS
/// (endereço desalinhado, lixo da stack em `[+0x08]`). 2ª tentativa (`write_handle_ret`, novo
/// helper que escreve `[+0x08]=0` EXPLICITAMENTE, esperando que o `cbz` inicial da rotina tratasse
/// como "sem refcount = no-op seguro") **AINDA CRASHOU**, mesma instrução, endereço de fault
/// ligeiramente diferente — ou seja, zerar `[+0x08]` NÃO foi suficiente; o real problema é mais
/// fundo: `rtti::new_object` provavelmente só constrói o OBJETO cru (`Construct()`/`CreateInstance`
/// via vtable), sem alocar+ligar um BLOCO DE REFCOUNT de verdade (o que uma API real tipo
/// `MakeHandle<T>()` faria) — então mesmo escrevendo 0 em `[+0x08]`, ALGO no caminho de
/// atribuição/cópia do valor de retorno (entre o `aOut` da native e o LOCAL redscript de fato)
/// deve estar lendo/copiando de outro lugar que ainda contém lixo, não investigado a fundo.
///
/// **Por que `CallbackSystem`/`ScriptableServiceContainer` (`BwmsGetCallbackSystem`/
/// `BwmsGetScriptableServiceContainer`) NUNCA bateram nisto:** são `extends IGameSystem` —
/// singletons do motor, hipótese forte (não confirmada) de que esse tipo de referência é
/// "weak"/não-contada (o motor é dono, script só empresta), então o compilador nem EMITE a
/// rotina de release pra locais desse tipo — diferente de `CallbackSystemHandler`/
/// `ScriptableService` (`IScriptable` puro, refcount real). Ambas continuam via `write_handle_ret`
/// (seguro, já provado) — não foram tocadas por este fix.
///
/// **CRASH FECHADO 2026-07-18 (sessão `handle-ctor-re`, continuação direta):** achado o
/// construtor REAL de `Handle<T>` do motor — `rtti::make_handle`/`ADDR_HANDLE_CTOR`
/// (`0x102104788`, nota completa em `rtti.rs`), achado por vizinhança da rotina de release já
/// conhecida (`0x1021048c4`, a 0x13c bytes DEPOIS no mesmo objeto) e confirmado por 4045
/// call-sites reais no binário inteiro com o padrão exato esperado (`<constrói objeto> -> mov
/// x1,raw_ptr -> bl 0x102104788`). `RegisterCallback` agora constrói a instância de verdade
/// (`rtti::new_object`), registra o `HandlerState` (pra `IsRegistered`/`AddTarget`/etc.
/// funcionarem) e escreve um HANDLE REAL (refcount-block genuíno, não zerado) via
/// `rtti::make_handle` — o release emitido pelo compilador no teardown do `ref<>` agora acha um
/// bloco válido pra decrementar, em vez de lixo/null. PROVADO ao vivo, múltiplos boots até
/// GAMEPLAY REAL (t=84s), zero crash novo (mesma degradação/timing do controle sem o mod de
/// teste).
///
/// **Achado #2, SEPARADO — encadeamento numa ÚNICA expressão (`a().b().c()`) tem um limite do
/// COMPILADOR/VM, não da nossa fix:** testado ao vivo (mesmo dia): a forma `RegisterCallback(...)
/// .AddTarget(...).SetLifetime(...)` numa expressão só faz o VM PULAR `AddTarget`/`SetLifetime`
/// inteiramente (nenhum trampolim `tramp_csh_*` é sequer chamado — confirmado por log
/// incondicional ausente), como se o `Context` opcode (null-safe navigation — ver
/// `redscript/compiler/src/assembler.rs::assemble`, `Expr::MethodCall`) julgasse o retorno de
/// `RegisterCallback` como null. **Refutado que seja realmente null:** diagnóstico logo após
/// `make_handle` (abaixo) leu de volta os 16 bytes de `out` e confirmou AMBOS os campos
/// não-nulos e plausíveis (`[+0x00]`=ponteiro do objeto, `[+0x08]`=bloco de refcount real). A
/// forma SEPARADA (cada retorno guardado num `let` próprio, com `IsDefined` checado entre cada
/// passo — `AddTarget`/`SetLifetime`/`IsRegistered`/`HasTarget`/`Unregister` como STATEMENTS
/// distintos, não uma expressão encadeada) FUNCIONA 100%: todos os 6 métodos disparam de
/// verdade, com efeito colateral real e observável (`IsRegistered()->true`, `HasTarget()->true`,
/// `Unregister()` remove a entrada certa de `CALLBACKS`). Ou seja: o problema é ESPECÍFICO de
/// usar o retorno de uma função NATIVA diretamente como receiver de outra chamada NA MESMA
/// expressão (sem passar por um `assign` pra um local primeiro) — hipótese não confirmada:
/// o slot de stack que o `Context` lê pro receiver nesse caso pode não ser o MESMO `out` que a
/// nativa recebeu (overlap/timing entre a call-convention nativa e a starting-stack do próximo
/// `Context`). Não é mais RE necessária pro objetivo desta sessão (o crash — o motivo original de
/// `cw-callback-handler` estar bloqueado — está genuinamente fechado); é um gap NOVO, mais
/// estreito, documentado honestamente em vez de escondido. Ver
/// `cp77-symbols/notes/proofs/2026-07-18-handle-ctor-re-*.log` pros logs completos dos 4 boots
/// desta sessão (1 anomalia não-reproduzida + 3 limpos, incl. o boot da forma separada).
unsafe extern "C" fn tramp_cbs_register_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let ok = if ev != 0 && target != 0 && fn_hash != 0 {
        CALLBACKS.lock().map(|mut c| { c.push((ev, target as usize, fn_hash)); true }).unwrap_or(false)
    } else {
        false
    };
    let mut handler: *mut c_void = std::ptr::null_mut();
    if ok {
        if let Some(reg) = rtti::Registry::obtain() {
            handler = rtti::new_object(&reg, "CallbackSystemHandler");
        }
    }
    if !handler.is_null() {
        let st = HandlerState {
            targets: Vec::new(),
            run_mode: 0,
            lifetime: 0,
            registered: true,
            event_hash: ev,
            listener: target as usize,
            fn_hash,
        };
        if let Ok(mut states) = HANDLER_STATES.lock() {
            states.push((handler as usize, st));
        }
    }
    crate::log(&format!(
        "[cbs] CallbackSystem.RegisterCallback(event '{}', target={target:#x}, fn '{}') ok={ok} -> handler={handler:p} (handle real via make_handle, ver nota 2026-07-18 acima)",
        crate::cname::resolve_cname(ev), crate::cname::resolve_cname(fn_hash)
    ));
    rtti::make_handle(out, handler);
    // Diagnóstico barato (mantido, 2026-07-18): confirma que `make_handle` realmente escreveu um
    // bloco de refcount NÃO-NULO em `[+0x08]` (não só o ponteiro do objeto em `[+0x00]`) — achado
    // que refutou a hipótese de "Context null-check vê isto como null" ao investigar por que a
    // forma ENCADEADA (`RegisterCallback(...).AddTarget(...)` numa expressão só) pula as chamadas
    // seguintes mesmo com um handle 100% válido (ver nota grande no topo desta função).
    if !out.is_null() {
        let raw0 = core::ptr::read_unaligned(out as *const u64);
        let raw1 = core::ptr::read_unaligned((out as *const u64).add(1));
        crate::log(&format!("[cbs][diag] out lido de volta: [+0x00]={raw0:#x} [+0x08]={raw1:#x}"));
    }
}

/// `RegisterStaticCallback(eventName, target: CName, function, opt sticky) -> ref<CallbackSystemHandler>`
/// — STUB: dispatch estático (por NOME de classe, sem instância) não tem registry própria ainda.
/// Registrado no RTTI (o validador de classe exige a presença, não a implementação completa) mas
/// não faz nada além de logar — escopo deferido, documentado (não é um bug silencioso).
unsafe extern "C" fn tramp_cbs_register_static_callback(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    crate::log(&format!(
        "[cbs] CallbackSystem.RegisterStaticCallback(event '{}') -> STUB (dispatch estático deferido, sem registry própria) -> null",
        crate::cname::resolve_cname(ev)
    ));
    write_handle_ret(out, 0); // ref<T> null = 16 bytes zerados (ver write_handle_ret)
}

/// `UnregisterCallback(eventName, target, opt function)` — FUNCIONAL: remove da `CALLBACKS`
/// registry as entradas que baterem (event, target) e, se `function` foi passado (hash != 0),
/// também `function`. Void (nada a escrever em `out`).
unsafe extern "C" fn tramp_cbs_unregister_callback(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let fn_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let removed = CALLBACKS
        .lock()
        .map(|mut c| {
            let before = c.len();
            c.retain(|(e, t, f)| !(*e == ev && *t as u64 == target && (fn_hash == 0 || *f == fn_hash)));
            before - c.len()
        })
        .unwrap_or(0);
    crate::log(&format!(
        "[cbs] CallbackSystem.UnregisterCallback(event '{}', target={target:#x}) removidos={removed}",
        crate::cname::resolve_cname(ev)
    ));
}

/// `UnregisterStaticCallback(eventName, target: CName, opt function)` — STUB (sem registry
/// estática, ver `RegisterStaticCallback`). Void.
unsafe extern "C" fn tramp_cbs_unregister_static_callback(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    crate::log(&format!(
        "[cbs] CallbackSystem.UnregisterStaticCallback(event '{}') -> STUB (no-op)",
        crate::cname::resolve_cname(ev)
    ));
}

/// `RegisterEvent(eventName, opt eventType) -> Bool` — permissivo: sempre `true` (não mantemos
/// uma tabela nome→tipo; qualquer nome de evento passado a `DispatchEventAs`/`RegisterCallback`
/// já funciona sem pré-registro na nossa `CALLBACKS`, então recusar aqui só atrapalharia mods
/// que checam o retorno antes de prosseguir).
unsafe extern "C" fn tramp_cbs_register_event(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    crate::log(&format!(
        "[cbs] CallbackSystem.RegisterEvent(event '{}') -> true (permissivo)",
        crate::cname::resolve_cname(ev)
    ));
    write_bool_ret(out, true);
}

/// `DispatchEvent(eventObject: ref<CallbackSystemEvent>)` — STUB: sem `CallbackSystemEvent` real
/// (gap `cw-event-target-classes`) não dá pra recuperar o NOME do evento a partir do objeto (a via
/// real do Codeware usa RegisterEvent p/ mapear nome→tipo; nossa RegisterEvent é permissiva e não
/// registra o mapeamento). Void, no-op documentado — usar `DispatchEventAs` enquanto isso.
unsafe extern "C" fn tramp_cbs_dispatch_event(_c: *mut c_void, _frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log("[cbs] CallbackSystem.DispatchEvent chamado -> STUB (sem CallbackSystemEvent real, ver DispatchEventAs)");
}

/// `DispatchEventAs(eventName, eventObject: ref<CallbackSystemEvent>) -> Void` — PARCIALMENTE
/// FUNCIONAL: dispara a `CALLBACKS` registry pelo NOME (via `fire_event_args_by_hash`, reusando o
/// hash já lido, sem round-trip). O PAYLOAD do `eventObject` (campos do evento) não é marshallado
/// pro alvo — só o disparo por nome funciona, igual ao `fire_event`/`fire_event_args` já provados.
unsafe extern "C" fn tramp_cbs_dispatch_event_as(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ev = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name = crate::cname::resolve_cname(ev);
    let n = fire_event_args_by_hash(ev, &name, &[]);
    crate::log(&format!(
        "[cbs] CallbackSystem.DispatchEventAs(event '{name}') -> disparou por NOME ({n} callback(s); payload do eventObject não marshallado)"
    ));
}

/// `GameInstance.GetCallbackSystem() -> ref<CallbackSystem>` — devolve o singleton, construindo-o
/// PREGUIÇOSAMENTE (`new_object`, só na 1ª chamada) — roda em contexto de gameplay normal (um mod
/// chamando isto já está bem depois do boot), não na janela frágil do bind. `new_object` usa o
/// vtable/size do MESMO donor (`gameuiInGameMenuGameController`) do forge da classe — Construct
/// escreve dentro do size certo, sem corromper heap (mesmo mecanismo já provado por `cwregalias`+
/// `newobj` em 2026-07-12).
unsafe extern "C" fn tramp_get_callback_system(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut ptr = CBS_INSTANCE.load(Ordering::Acquire);
    if ptr.is_null() {
        if let Some(reg) = rtti::Registry::obtain() {
            let obj = rtti::new_object(&reg, "CallbackSystem");
            if !obj.is_null() {
                CBS_INSTANCE.store(obj, Ordering::Release);
                ptr = obj;
                crate::log(&format!("[cbs] GetCallbackSystem: instância construída {obj:p} (lazy, 1a chamada)"));
            } else {
                crate::log("[cbs] GetCallbackSystem: new_object('CallbackSystem') falhou");
            }
        } else {
            crate::log("[cbs] GetCallbackSystem: Registry::obtain() falhou");
        }
    }
    write_handle_ret(out, ptr as u64); // ref<T>, ver write_handle_ret
}

/// Forja a classe `CallbackSystem` (extends IGameSystem) + registra os 7 métodos nativos + o
/// getter estático `GameInstance.GetCallbackSystem()`. Mesmo fix de parent+fullName-bare que
/// fechou a Facade (`register_codeware_facade`), generalizado: aqui o parent é IGameSystem (não
/// IScriptable) porque o `.reds` real declara `extends IGameSystem` explícito, e a classe é
/// INSTANCIÁVEL (`register_type_instantiable_with_parent`, não `register_type_min`) porque
/// `GetCallbackSystem()` precisa devolver um objeto de verdade. Ver módulo `callbacksystem-native.reds`
/// pro escopo exato (funcional vs. stub) de cada método.
/// Resolve uma classe pelo MESMO caminho que o VALIDADOR de classe nativa usa internamente pra
/// checar "base declarada" (singleton em `0x1021885a0` + `vtbl+0x108`) — **diferente** de
/// `Registry::class_by_name` (que usa `vtbl+0x10` do singleton obtido por `Registry::obtain()`).
/// ACHADO AO VIVO (2026-07-13, `cw-callbacksystem-rtti`): `IGameSystem` É uma classe nativa REAL,
/// já registrada pelo próprio motor (endereço estático, ex. `0x10c7c5260`) — mas
/// `class_by_name("IGameSystem")` retornava null (vtbl+0x10 não a via, por razão ainda não
/// mapeada — objeto/slot diferente, não falta de registro). Esta função usa o MESMO caminho que
/// o validador usa de verdade, evitando forjar (arriscado e desnecessário) uma classe que já existe.
pub(crate) unsafe fn resolve_class_via_validator_getclass(name: &str) -> *mut c_void {
    let getter: extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(0x1_0218_85a0));
    let singleton = getter();
    if singleton.is_null() || !crate::gum::is_readable(singleton as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = core::ptr::read_unaligned(singleton as *const *mut u8);
    if vt.is_null() || !crate::gum::is_readable(vt.add(0x108) as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let slot = core::ptr::read_unaligned(vt.add(0x108) as *const *mut c_void);
    if !rtti::sane(slot) {
        return std::ptr::null_mut();
    }
    let getclass: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(slot);
    getclass(singleton, cname(name))
}

// `cw-event-target-classes` — TESTE ISOLADO enum-return, 2026-07-18, 4 rodadas, TESTADO E
// REVERTIDO. Conclusão final (ver nota completa em `callbacksystem-native.reds`): 3 de 4 boots
// com a mesma configuração essencial NÃO crasharam (incluindo uma repetição quase exata do
// boot que crashou) — a hipótese "retorno enum quebra o bind-pass" NÃO é confirmada de forma
// confiável; o crash isolado do Round 1 é provavelmente um flake ambiental, não determinístico.

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
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
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
    // um mod real do Windows esperaria (ver `callbacksystem-native.reds`).
    format!(
        "[reg] CallbackSystem: classe=OK RegisterCallback={m1} RegisterStaticCallback={m2} UnregisterCallback={m3} UnregisterStaticCallback={m4} RegisterEvent={m5} DispatchEvent={m6} DispatchEventAs={m7}"
    )
}

// ===== `cw-scriptableservice` + `cw-callback-handler` (2026-07-15) — MESMA receita 2x já provada,
// zero RE nova: `ScriptableService`/`CallbackSystemTarget` (sem `extends` = parent IScriptable
// implícito, receita da Facade) + `ScriptableServiceContainer` (extends IGameSystem, receita do
// CallbackSystem) + `CallbackSystemHandler` upgradado de placeholder script pra native (mesmo
// parent IScriptable seguro). Métodos que devolveriam `ref<Self>`/`ref<ScriptableService>` pra
// encadeamento retornam NULL (mesmo padrão já provado e documentado em `tramp_cbs_register_callback`
// — `write_uint_ret(out,0,8)`: zero risco de refcount, só quebra encadeamento de mods que dependam
// de uma instância real, que ainda não existe). Fonte real conferida em
// `enablers/Codeware/scripts/{Scripting/ScriptableService*.reds,Callback/CallbackSystemHandler.reds}`.

/// `ScriptableService` — classe abstrata SEM métodos nativos (os callbacks OnLoad/OnReload/
/// OnInitialize/OnUninitialize da fonte real são comentados/virtuais de script, nada a registrar
/// aqui). Só precisa EXISTIR como parent válido pra subclasses de mod (`extends ScriptableService`).
pub unsafe fn register_scriptableservice(reg: &Registry) -> String {
    let forged = register_type_min(reg, "ScriptableService");
    // EXPERIMENTO TENTADO e REFUTADO (2026-07-17): priming via `GetType("handle:X")` logo após
    // forjar a classe, na esperança de popular o cache que o bind pass do compilador consulta pra
    // métodos com retorno/param `ref<classe-forjada>`. RESULTADO: `GetType("handle:ScriptableService")`
    // devolveu NULL mesmo com a classe já forjada+registrada — refuta a hipótese de que GetType
    // faz lazy-construct-and-cache pra classes forjadas em runtime (funciona só pra "handle:IScriptable"
    // porque IScriptable é classe REAL pré-existente, com handle-type já construído pelo motor no
    // próprio boot). O cache real que o bind pass usa (`[type_ref+0x18]` no descritor da FUNÇÃO
    // COMPILADA, não no CRTTISystem::GetType geral) é outro sistema, cujo WRITER ainda não foi
    // achado. Ver HISTORICO.md + memória cp77-native-addr-re-arm64 pro relato completo. Crash
    // reconfirmado (GetService em ScriptableServiceContainer) — NÃO reativar sem achar o writer.
    format!("[reg] ScriptableService: forjada={}", !forged.is_null())
}

/// `CallbackSystemTarget` — classe abstrata SEM métodos, só precisa existir como TIPO pra
/// `CallbackSystemHandler.AddTarget(ref<CallbackSystemTarget>)` type-checar no compilador.
pub unsafe fn register_callbacksystemtarget(reg: &Registry) -> String {
    let forged = register_type_min(reg, "CallbackSystemTarget");
    format!("[reg] CallbackSystemTarget: forjada={}", !forged.is_null())
}

static SSC_INSTANCE: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

// `tramp_ssc_get_service` (`GetService(name) -> ref<ScriptableService>`) — histórico da saga de
// crash (2026-07-15 a 2026-07-18), FECHADA:
//   TENTATIVA 1 (2026-07-15, bisect): registrar `GetService` crashava (SIGSEGV, alguns segundos
//     depois do forge reportar sucesso). `ScriptableService`/`ScriptableServiceContainer` sozinhas
//     (0 métodos) = limpo; só adicionar `GetService` = crash. Revertido.
//   TENTATIVA 2 (2026-07-17, `bindsig-probe`): hook no validador `BindFunctionSignature`
//     (0x1021ea1b8) mostrou `[type_ref+0x18]` do retorno/param JÁ POPULADO quando GetService foi
//     validado — REFUTA a hipótese de "cache de tipo null". Revertido de novo.
//   TENTATIVA 3 (2026-07-18, `dynarraygrowth-probe`) — CAUSA RAIZ ACHADA E CORRIGIDA: o
//     crash-report apontou pra uma rotina de CRESCIMENTO DE CONTAINER (`0x10096ca74`), não pro
//     validador. `CClass` (o forge usado por `register_type_min`/`register_type_instantiable_
//     with_parent`) embute hashmaps internos em offsets fixos (`+0x78`/`+0xA8` confirmados) cujo
//     "alocador" interno (vtable pointer em `slot+0x28`) fica NULO porque o forge zera a struct
//     inteira e só escreve um punhado de campos conhecidos. Na 1ª inserção nesse hashmap (dispara
//     quando a classe forjada ganha seu 1º método) o engine desreferencia esse vtable nulo ->
//     SIGSEGV. Fix: `fix_embedded_allocator_vtables` (abaixo) copia o vtable do alocador de uma
//     classe DONOR real (que já tem o campo populado mesmo "vazia") pro forjado, ANTES do
//     RegisterType. PROVADO ao vivo: boot completo até GAMEPLAY, `GetService` REGISTRADO+CHAMADO
//     via `callon` com sucesso, jogo estável (`cp77-symbols/notes/proofs/
//     2026-07-18-cw-scriptableservice-getservice-FIX-PROVADO.log`). Mesma causa provavelmente
//     explica o crash histórico de `CallbackSystemHandler` (mesma categoria — 6 métodos com
//     `ref<>` de classe forjada) já que o fix está no forge COMPARTILHADO, usado por toda classe
//     nativa forjada do projeto — não re-testado nesta sessão (candidato forte pro próximo passo).
// Nota: isto NUNCA afetou a GLOBAL `BwmsGetScriptableServiceContainer`/
// `tramp_get_scriptableservicecontainer` (abaixo) — funções GLOBAIS retornando `ref<classe-
// forjada>` sempre foram seguras (mesmo padrão de `BwmsGetCallbackSystem`); o crash era
// ESPECÍFICO de MÉTODOS registrados numa classe (o gatilho da 1ª inserção no hashmap embutido).
//   TENTATIVA 4 (2026-07-18, MESMO DIA, sessão `handle-ctor-re`) — FECHADA: o candidato forte da
//     Tentativa 3 (`fix_embedded_allocator_vtables` também resolveria o retorno de instância real)
//     era sobre OUTRA causa raiz (forge/registro), não sobre o bloqueador que na prática impedia
//     `GetService` de devolver algo != null — esse era o MESMO 2º bloqueador achado e fechado em
//     `cw-callback-handler` hoje mais cedo (Handle<T> sem refcount-block real, `ref<T>` compilado =
//     16 bytes). Com `rtti::make_handle`/`ADDR_HANDLE_CTOR` (0x102104788) disponível, `GetService`
//     agora constrói+devolve um `ref<ScriptableService>` REAL (ver `SERVICE_INSTANCES` abaixo) sem
//     crashar no release. PROVADO ao vivo — ver `cp77-symbols/notes/proofs/
//     2026-07-18-cw-scriptableservice-*-PROVADO.log`.

/// `GameInstance.GetScriptableServiceContainer() -> ref<ScriptableServiceContainer>` — exposto como
/// GLOBAL (`BwmsGetScriptableServiceContainer`), MESMO fix pragmático do `BwmsGetCallbackSystem`:
/// `@addMethod(GameInstance)` crasha (classe valida cedo demais). Singleton lazy via `new_object`.
unsafe extern "C" fn tramp_get_scriptableservicecontainer(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut ptr = SSC_INSTANCE.load(Ordering::Acquire);
    if ptr.is_null() {
        if let Some(reg) = rtti::Registry::obtain() {
            let obj = rtti::new_object(&reg, "ScriptableServiceContainer");
            if !obj.is_null() {
                SSC_INSTANCE.store(obj, Ordering::Release);
                ptr = obj;
                crate::log(&format!("[svc] GetScriptableServiceContainer: instância construída {obj:p} (lazy, 1a chamada)"));
            } else {
                crate::log("[svc] GetScriptableServiceContainer: new_object('ScriptableServiceContainer') falhou");
            }
        } else {
            crate::log("[svc] GetScriptableServiceContainer: Registry::obtain() falhou");
        }
    }
    write_handle_ret(out, ptr as u64); // ref<T>, ver write_handle_ret
}

/// Registry de instâncias de `ScriptableService`, keyed pelo NOME pedido em `GetService(name)` —
/// FECHADO 2026-07-18 (sessão `handle-ctor-re`, continuação direta do fix de `cw-callback-handler`
/// no mesmo dia). Semântica REAL do Codeware (`ScriptableServiceContainer.cpp::OnInitializeScripts`
/// — fonte vendorizada): escaneia a RTTI por TODAS as subclasses CONCRETAS de `ScriptableService` e
/// auto-instancia UMA por classe, keyed pelo NOME DA CLASSE; `GetService(name)` só faz o lookup
/// (`m_services.find(name)`), nunca constrói nada. Reimplementação simplificada aqui (sem RTTI-
/// class-scanning novo, fora de escopo desta sessão): forjamos UMA classe concreta interna
/// (`BwmsDefaultService`, parent = `ScriptableService` REAL, via `register_type_instantiable_
/// with_parent` — o forge ROBUSTO, não o `register_type_min` abstrato) e devolvemos uma instância
/// dela por NOME PEDIDO, lazy (1ª chamada com aquele nome forja+constrói+cacheia; chamadas
/// seguintes com o MESMO nome devolvem a MESMA instância — é o que "sobrevive a reload" do
/// `proof_needed` quer dizer aqui: identidade persistente por nome, dentro da sessão, não que
/// atravessa um save/load). Handle REAL via `rtti::make_handle`/`ADDR_HANDLE_CTOR` (mesma rotina
/// do motor que fechou `cw-callback-handler` hoje mais cedo).
static SERVICE_INSTANCES: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

/// `GetService(name) -> ref<ScriptableService>` — FUNCIONAL (2026-07-18): devolve uma instância
/// REAL (não mais `null`), construída sob demanda e cacheada por `name`. Ver nota grande acima
/// (`SERVICE_INSTANCES`) pro mecanismo completo e a ressalva honesta vs. a semântica real do
/// Codeware (RTTI-scan de subclasses reais de mods, não implementado aqui).
unsafe extern "C" fn tramp_ssc_get_service(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let name = args.first().map(|(v, _)| *v).unwrap_or(0);
    let name_str = crate::cname::resolve_cname(name);
    let mut inst: *mut c_void = std::ptr::null_mut();
    let mut cached = false;
    if name != 0 {
        if let Ok(states) = SERVICE_INSTANCES.lock() {
            if let Some((_, p)) = states.iter().find(|(n, _)| *n == name) {
                inst = *p as *mut c_void;
                cached = true;
            }
        }
        if inst.is_null() {
            if let Some(reg) = rtti::Registry::obtain() {
                let cls = reg.class_by_name("BwmsDefaultService");
                let cls = if rtti::sane(cls) {
                    cls
                } else {
                    let parent = reg.class_by_name("ScriptableService");
                    if rtti::sane(parent) {
                        register_type_instantiable_with_parent(&reg, "BwmsDefaultService", "gameuiInGameMenuGameController", parent)
                    } else {
                        crate::log("[svc] GetService: parent 'ScriptableService' não resolveu — não dá pra forjar BwmsDefaultService");
                        std::ptr::null_mut()
                    }
                };
                if rtti::sane(cls) {
                    let obj = rtti::new_object(&reg, "BwmsDefaultService");
                    if !obj.is_null() {
                        inst = obj;
                        if let Ok(mut states) = SERVICE_INSTANCES.lock() {
                            states.push((name, obj as usize));
                        }
                    } else {
                        crate::log("[svc] GetService: new_object('BwmsDefaultService') falhou");
                    }
                }
            } else {
                crate::log("[svc] GetService: Registry::obtain() falhou");
            }
        }
    }
    crate::log(&format!(
        "[svc] ScriptableServiceContainer.GetService('{name_str}') -> {inst:p} cached={cached} (handle real via make_handle, ver nota 2026-07-18 acima)"
    ));
    rtti::make_handle(out, inst);
}

/// Forja `ScriptableServiceContainer extends IGameSystem` (mesma receita/parent do
/// `register_callbacksystem`) + registra `GetService`. FECHADO 2026-07-18 (ver histórico completo
/// acima de `tramp_ssc_get_service`): o fix em `fix_embedded_allocator_vtables` (chamado de dentro
/// de `register_type_instantiable_with_parent`) resolve o crash que bloqueava isto desde
/// 2026-07-15 — PROVADO ao vivo (boot→gameplay, GetService chamado via `callon`, zero crash).
pub unsafe fn register_scriptableservicecontainer(reg: &Registry) -> String {
    let igs = reg.class_by_name("IGameSystem");
    let igs = if rtti::sane(igs) { igs } else { resolve_class_via_validator_getclass("IGameSystem") };
    if !rtti::sane(igs) {
        return "[reg] register_scriptableservicecontainer: 'IGameSystem' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "ScriptableServiceContainer", "gameuiInGameMenuGameController", igs);
    if forged.is_null() {
        return "[reg] register_scriptableservicecontainer: forja da classe FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_scriptableservicecontainer: classe=OK mas sem protótipo (AddGodMode não resolveu) — GetService NÃO registrado".into(),
    };
    let m1 = register_method(reg, "ScriptableServiceContainer", proto, "GetService", "GetService", tramp_ssc_get_service, false);
    format!("[reg] ScriptableServiceContainer: classe=OK GetService={m1} (crash de hashmap embutido CORRIGIDO 2026-07-18, ver fix_embedded_allocator_vtables)")
}

/// `AddTarget(target: ref<CallbackSystemTarget>) -> ref<CallbackSystemHandler>` — FUNCIONAL
/// (2026-07-18): adiciona `target` (ponteiro) na lista de alvos do handler (`c` = `this`, ver
/// `HANDLER_STATES`) se ainda não estiver lá (dedupe), e devolve `self` (`c`) — o encadeamento
/// real (`RegisterCallback(...).AddTarget(...).AddTarget(...)`) funciona porque cada chamada
/// devolve o MESMO ponteiro de handler. Lista de alvos = O FILTRO (ver `tramp_csh_has_target`):
/// vazia = sem filtro (aceita qualquer alvo); não-vazia = só aceita alvos presentes nela.
unsafe extern "C" fn tramp_csh_add_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut found = false;
    if !c.is_null() && target_arg != 0 {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().find(|(p, _)| *p == c as usize) {
                if !st.targets.contains(&(target_arg as usize)) {
                    st.targets.push(target_arg as usize);
                }
                found = true;
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.AddTarget(target={target_arg:#x}) on {c:p} handler_achado={found}"));
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}
/// `RemoveTarget(target: ref<CallbackSystemTarget>) -> ref<CallbackSystemHandler>` — FUNCIONAL:
/// remove `target` da lista (se presente), devolve `self` (mesmo padrão de `AddTarget`).
unsafe extern "C" fn tramp_csh_remove_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    if !c.is_null() && target_arg != 0 {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().find(|(p, _)| *p == c as usize) {
                st.targets.retain(|t| *t != target_arg as usize);
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.RemoveTarget(target={target_arg:#x}) on {c:p}"));
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}
/// `SetRunMode(runMode: CallbackRunMode) -> ref<CallbackSystemHandler>` — FUNCIONAL: guarda o
/// valor do enum (lido como i64 cru, mesmo esquema de qualquer param — enums são POD) no estado
/// do handler, devolve `self`.
unsafe extern "C" fn tramp_csh_set_run_mode(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let mode = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    if !c.is_null() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().find(|(p, _)| *p == c as usize) {
                st.run_mode = mode;
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.SetRunMode({mode}) on {c:p}"));
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}
/// `SetLifetime(lifetime: CallbackLifetime) -> ref<CallbackSystemHandler>` — FUNCIONAL: mesmo
/// padrão de `SetRunMode`.
unsafe extern "C" fn tramp_csh_set_lifetime(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let lifetime = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    if !c.is_null() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().find(|(p, _)| *p == c as usize) {
                st.lifetime = lifetime;
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.SetLifetime({lifetime}) on {c:p}"));
    rtti::make_handle(out, c); // encadeamento: devolve self via handle REAL (2026-07-18, ver ADDR_HANDLE_CTOR em rtti.rs — reusa/incrementa o bloco JÁ existente do próprio `c`)
}
/// `IsRegistered() -> Bool` — FUNCIONAL: reflete `HandlerState.registered` de verdade (era sempre
/// `false` até 2026-07-18; agora `true` logo após `RegisterCallback`, `false` após `Unregister`).
unsafe extern "C" fn tramp_csh_is_registered(c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut reg = false;
    if !c.is_null() {
        if let Ok(states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter().find(|(p, _)| *p == c as usize) {
                reg = st.registered;
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.IsRegistered() on {c:p} -> {reg}"));
    write_bool_ret(out, reg);
}
/// `Unregister()` — FUNCIONAL: marca `registered=false` E remove a entrada correspondente da
/// `CALLBACKS` registry (usando `event_hash`/`listener`/`fn_hash` guardados no `RegisterCallback`
/// original) — daí em diante o evento NÃO dispara mais pra este listener/função. Void.
unsafe extern "C" fn tramp_csh_unregister(c: *mut c_void, _frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let mut removed_cb = 0usize;
    if !c.is_null() {
        if let Ok(mut states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter_mut().find(|(p, _)| *p == c as usize) {
                st.registered = false;
                let (ev, listener, fh) = (st.event_hash, st.listener, st.fn_hash);
                if let Ok(mut cbs) = CALLBACKS.lock() {
                    let before = cbs.len();
                    cbs.retain(|(e, t, f)| !(*e == ev && *t == listener && *f == fh));
                    removed_cb = before - cbs.len();
                }
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.Unregister() on {c:p} — removeu {removed_cb} entrada(s) de CALLBACKS"));
}
/// `HasTarget(target: ref<CallbackSystemTarget>) -> Bool` — BWMS-ONLY (não existe na fonte real do
/// Codeware; adicionado 2026-07-18 pra tornar o "filtro de target" testável/observável): devolve
/// `true` se `target` está na lista adicionada via `AddTarget` (ou se a lista está vazia — "sem
/// filtro" aceita qualquer alvo, mesmo esquema documentado em `HandlerState`). Prova, por chamada
/// de método REAL (não só leitura de estado interno), que `AddTarget`/`RemoveTarget` mutam um
/// filtro genuíno e que ele DIFERENCIA alvos corretamente.
unsafe extern "C" fn tramp_csh_has_target(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let target_arg = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut has = false;
    if !c.is_null() {
        if let Ok(states) = HANDLER_STATES.lock() {
            if let Some((_, st)) = states.iter().find(|(p, _)| *p == c as usize) {
                has = st.targets.is_empty() || st.targets.contains(&(target_arg as usize));
            }
        }
    }
    crate::log(&format!("[csh] CallbackSystemHandler.HasTarget(target={target_arg:#x}) on {c:p} -> {has}"));
    write_bool_ret(out, has);
}

/// Forja `CallbackSystemHandler` — INSTANCIÁVEL de verdade (2026-07-18, `cw-callback-handler`
/// FECHADO). Histórico: TENTADO 2026-07-15 como classe (métodos com `ref<X>` de classe forjada) —
/// CRASHOU (SIGTRAP), mesma categoria genérica do crash de `GetService`/`ScriptableServiceContainer`
/// (ver a saga completa em `register_scriptableservicecontainer`/`tramp_ssc_get_service` acima).
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
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
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
/// parte do `proof_needed` de `cw-event-target-classes`, que foca em `KeyInputEvent.GetKey/
/// GetAction`).
unsafe extern "C" fn tramp_cse_get_event_name(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, cname("KeyInputEvent"), 8);
}

pub unsafe fn register_callbacksystemevent(reg: &Registry) -> String {
    let forged = register_type_min(reg, "CallbackSystemEvent");
    if forged.is_null() {
        return "[reg] register_callbacksystemevent: forja FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_callbacksystemevent: classe=OK mas sem protótipo — GetEventName NÃO registrado".into(),
    };
    let m1 = register_method(reg, "CallbackSystemEvent", proto, "GetEventName", "GetEventName", tramp_cse_get_event_name, false);
    format!("[reg] CallbackSystemEvent: classe=OK GetEventName={m1}")
}

/// `KeyInputEvent extends CallbackSystemEvent` — `cw-event-target-classes`/`cw-rawinput-realname`,
/// RETRY 2026-07-18 (sessão `handle-ctor-re`), 3ª tentativa (2 anteriores em 2026-07-13/15
/// crasharam — ver histórico em `callbacksystem-native.reds`). Diferença desta vez: `parent` é a
/// classe `CallbackSystemEvent` REAL, forjada nativa (`register_type_min`, chamada acima) — não
/// mais um parent não-nativo (causa da Tentativa 1) nem `IScriptable` direto pulando a hierarquia
/// real (Tentativa 2). Forjada via `register_type_instantiable_with_parent` (o forge ROBUSTO, com
/// `fix_embedded_allocator_vtables` — o fix que fechou `cw-callback-handler`/`cw-scriptableservice`
/// hoje mais cedo). Campos reais (`action: EInputAction`, `key: EInputKey`, `state: KeyboardState`)
/// NÃO são lidos da memória crua do objeto C++ (RE de layout fora de escopo) — mantidos num
/// registry Rust por-instância (`KEYINPUT_STATES`, mesmo padrão de `HANDLER_STATES`/
/// `SERVICE_INSTANCES`), populado por `BwmsMakeTestKeyInputEvent` (native de TESTE) e lido pelos
/// getters. Prova a MECÂNICA (forja+registro+construção+despacho de método com retorno enum) sem
/// depender de wiring de teclado real (gap separado, maior).
static KEYINPUT_STATES: Mutex<Vec<(usize, i32, i32, bool, bool, bool)>> = Mutex::new(Vec::new());

unsafe extern "C" fn tramp_kie_get_action(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0i32;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, a, ..)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *a;
        }
    }
    crate::log(&format!("[kie] KeyInputEvent.GetAction() on {c:p} -> {v}"));
    write_uint_ret(out, v as u32 as u64, 8);
}
unsafe extern "C" fn tramp_kie_get_key(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0i32;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, k, ..)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *k;
        }
    }
    crate::log(&format!("[kie] KeyInputEvent.GetKey() on {c:p} -> {v}"));
    write_uint_ret(out, v as u32 as u64, 8);
}
unsafe extern "C" fn tramp_kie_is_shift_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, s, ..)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *s;
        }
    }
    write_bool_ret(out, v);
}
unsafe extern "C" fn tramp_kie_is_control_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, _, ctl, _)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *ctl;
        }
    }
    write_bool_ret(out, v);
}
unsafe extern "C" fn tramp_kie_is_alt_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, _, _, alt)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *alt;
        }
    }
    write_bool_ret(out, v);
}

pub unsafe fn register_keyinputevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_keyinputevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "KeyInputEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_keyinputevent: forja da classe FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_keyinputevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    // GetAction()->EInputAction FORA (ver nota grande em `callbacksystem-native.reds`): tipo
    // `[UNRESOLVED_TYPE]` no compilador — não existe registro pra forjar ainda (gap separado).
    let m2 = register_method(reg, "KeyInputEvent", proto, "GetKey", "GetKey", tramp_kie_get_key, false);
    let m3 = register_method(reg, "KeyInputEvent", proto, "IsShiftDown", "IsShiftDown", tramp_kie_is_shift_down, false);
    let m4 = register_method(reg, "KeyInputEvent", proto, "IsControlDown", "IsControlDown", tramp_kie_is_control_down, false);
    let m5 = register_method(reg, "KeyInputEvent", proto, "IsAltDown", "IsAltDown", tramp_kie_is_alt_down, false);
    format!(
        "[reg] KeyInputEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetKey={m2} IsShiftDown={m3} IsControlDown={m4} IsAltDown={m5}"
    )
}

/// `BwmsMakeTestKeyInputEvent() -> ref<KeyInputEvent>` — GLOBAL DE TESTE (2026-07-18): constrói
/// uma instância real de `KeyInputEvent` com valores FIXOS de fixture (action=2, key=99,
/// shift=true, control=false, alt=true — marcadores arbitrários, não semânticos, só pra provar
/// round-trip), guarda em `KEYINPUT_STATES` e devolve um handle REAL via `rtti::make_handle`.
/// Simula o que um `CallbackSystem.DispatchEvent` real faria ao entregar um evento de teclado a
/// um listener — sem depender do wiring de teclado real (RawInput controller, gap separado).
unsafe extern "C" fn tramp_make_test_keyinputevent(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "KeyInputEvent");
        if !obj.is_null() {
            if let Ok(mut states) = KEYINPUT_STATES.lock() {
                states.push((obj as usize, 2, 99, true, false, true));
            }
        } else {
            crate::log("[kie] BwmsMakeTestKeyInputEvent: new_object('KeyInputEvent') falhou");
        }
    }
    crate::log(&format!("[kie] BwmsMakeTestKeyInputEvent() -> {obj:p} (fixture action=2 key=99 shift=T control=F alt=T)"));
    rtti::make_handle(out, obj);
}

/// Mapeia o keycode do AppKit (`NSEvent.keyCode`, hardware scancode do macOS) + o caractere
/// capturado (`charactersIgnoringModifiers`) pro valor NUMÉRICO REAL do `EInputKey` do motor
/// (`cp77-symbols/redscript-src/orphans.script:3223`). **Achado-chave:** `EInputKey` bate EXATO
/// com os códigos Win32 `VK_*` — `IK_A=65`=ASCII `'A'`, `IK_0=48`=ASCII `'0'`, `IK_Space=32`=ASCII
/// `' '` — ou seja, pra letras/dígitos/espaço o valor de `EInputKey` É o código ASCII do
/// caractere capturado, SEM precisar de tabela. Pra teclas SEM caractere (setas/função/etc.),
/// tabela pequena pelo keyCode macOS (RE manual, valores confirmados contra o enum real). NÃO
/// exaustiva (cobre letras+dígitos+espaço+setas+alguns extras) — suficiente pro `proof_needed`
/// de `cw-rawinput-realname` ("mod recebe `ref<KeyInputEvent>` e `GetKey()` bate com a tecla
/// real"); `0` (`IK_None`) se a tecla não mapear — não bloqueia, só não identifica.
pub fn map_macos_keycode_to_einputkey(kc: i32, ch: Option<char>) -> i32 {
    if let Some(c) = ch {
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return c as i32;
        }
        if c.is_ascii_lowercase() {
            return c.to_ascii_uppercase() as i32;
        }
        if c == ' ' {
            return 32; // IK_Space
        }
    }
    match kc {
        123 => 37,  // Left     -> IK_Left
        126 => 38,  // Up       -> IK_Up
        124 => 39,  // Right    -> IK_Right
        125 => 40,  // Down     -> IK_Down
        36 => 13,   // Return   -> IK_Enter
        48 => 9,    // Tab      -> IK_Tab
        51 => 8,    // Delete   -> IK_Backspace
        53 => 27,   // Escape   -> IK_Escape
        49 => 32,   // Space (fallback se `ch` vier vazio) -> IK_Space
        122 => 112, // F1       -> IK_F1
        120 => 113, // F2       -> IK_F2
        99 => 114,  // F3       -> IK_F3
        118 => 115, // F4       -> IK_F4
        _ => 0,     // IK_None — não mapeado
    }
}

/// `cw-rawinput-realname` (2026-07-18, sessão `handle-ctor-re`, continuação direta de
/// `cw-event-target-classes`): constrói um `KeyInputEvent` REAL (mesma técnica provada —
/// `rtti::new_object`+`make_handle`) a partir de um valor JÁ MAPEADO de `EInputKey` +
/// modificadores capturados do AppKit, e devolve um `rtti::Arg::Handle` pronto pra
/// `fire_event_args`/`call_func` despachar POR REFERÊNCIA (não mais um escalar Int32 cru).
/// `None` se a construção falhar (RTTI não pronto etc.) — o caller descarta a tecla, não crasha.
pub unsafe fn make_keyinputevent_arg(key: i32, shift: bool, control: bool, alt: bool) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "KeyInputEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = KEYINPUT_STATES.lock() {
        // action=2: placeholder (IACT_Press-like, ver App/Callback/Controllers/RawInputHook.hpp
        // vendorizado — real `EInputAction` segue não-resolvível no compilador, ver nota em
        // `register_keyinputevent`, então `GetAction()` não é exposto e este valor não é lido
        // por nenhum script — só preenche o slot do registry por completude).
        states.push((obj as usize, 2, key, shift, control, alt));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// `GameSessionEvent extends CallbackSystemEvent` — `cw-controller-session` (2026-07-18, sessão
/// `handle-ctor-re`, 6ª rodada). Fonte real: `public native class GameSessionEvent extends
/// CallbackSystemEvent { public native func IsRestored() -> Bool; public native func IsPreGame()
/// -> Bool }` (`GameSessionEvent.reds`) — dados reais (`GameSessionEvent.hpp` vendorizado):
/// `bool preGame; bool restored;`. MESMA receita 100% segura já provada em `KeyInputEvent`
/// (forge robusto + parent `CallbackSystemEvent` real). Estado por-instância no MESMO padrão de
/// `KEYINPUT_STATES` (`GAMESESSION_STATES: Vec<(ptr,pregame,restored)>`).
static GAMESESSION_STATES: Mutex<Vec<(usize, bool, bool)>> = Mutex::new(Vec::new());

unsafe extern "C" fn tramp_gse_is_restored(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = GAMESESSION_STATES.lock() {
        if let Some((_, _, r)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *r;
        }
    }
    write_bool_ret(out, v);
}
unsafe extern "C" fn tramp_gse_is_pregame(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = GAMESESSION_STATES.lock() {
        if let Some((_, pg, _)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *pg;
        }
    }
    write_bool_ret(out, v);
}

pub unsafe fn register_gamesessionevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_gamesessionevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "GameSessionEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_gamesessionevent: forja da classe FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_gamesessionevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "GameSessionEvent", proto, "IsRestored", "IsRestored", tramp_gse_is_restored, false);
    let m2 = register_method(reg, "GameSessionEvent", proto, "IsPreGame", "IsPreGame", tramp_gse_is_pregame, false);
    format!("[reg] GameSessionEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) IsRestored={m1} IsPreGame={m2}")
}

/// Constrói um `GameSessionEvent` REAL (mesma técnica de `make_keyinputevent_arg`) e devolve um
/// `Arg::Handle` pronto pra `fire_event_args`. `restored`/`pregame` refletem o contexto REAL do
/// disparo — ver `lib.rs::cp77_tick`, bloco de transição de presença do player (`restored=true`
/// pq o auto-continue SEMPRE carrega um save; `pregame=false` pq o player só fica presente DEPOIS
/// da criação de personagem). `None` se a construção falhar — o caller descarta, não crasha.
pub unsafe fn make_gamesessionevent_arg(pregame: bool, restored: bool) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "GameSessionEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = GAMESESSION_STATES.lock() {
        states.push((obj as usize, pregame, restored));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// `EntityLifecycleEvent extends CallbackSystemEvent` — `cw-controller-entity` (2026-07-18,
/// sessão `handle-ctor-re`, 7ª rodada). Fonte real: `public native class EntityLifecycleEvent
/// extends CallbackSystemEvent { public native func GetEntity() -> wref<Entity> }`. É o evento
/// REAL usado por `EntityAttachHook`/`EntityAssembleHook` (`enablers/Codeware/src/App/Callback/
/// Controllers/EntityAttachHook.hpp`: `EventName = Red::CName("Entity/Attach")`, dispatch via
/// `DispatchNativeEvent<EntityLifecycleEvent>`) — RE confirmou que "Entity/Attach" carrega
/// `EntityLifecycleEvent`, NÃO `EntityBuilderEvent` (esse último é específico de "Entity/Extract",
/// que embrulha um `Red::EntityBuilder` inteiro via `EntityBuilderWrapper` — uma peça MAIOR, fora
/// de escopo desta rodada).
///
/// **`wref<Entity>` TESTADO AO VIVO E REVERTIDO (2026-07-18):** 1ª tentativa declarou `GetEntity()
/// -> wref<Entity>` no `.reds` com `write_handle_ret {ptr,0}` (mesmo padrão 100% provado pra
/// `ref<T>` — ver `tramp_get_callback_system`). Resultado: boot limpo (zero crash, GAMEPLAY t=84s
/// OK), MAS o campo leu `IsDefined()==false` (`EntityAttachEntityNullFAIL`) mesmo com o ponteiro
/// do player, válido, escrito corretamente. Causa provável (RE do `redscript/compiler` vendorizado):
/// `build_native_func` (acima) NÃO popula NENHUM descritor de tipo na `CBaseFunction` forjada —
/// só vtable clonada+CName+flags — então o tipo `ref` vs `wref` é resolvido 100% em COMPILE-TIME
/// pelo `scc` a partir do `.reds`, não por reflexão do runtime nativo. `wref` provavelmente exige
/// um WeakRefCount block de verdade (layout NÃO RE-ado neste projeto) pra `IsDefined` resolver
/// `true` com refcount-ptr null — diferente do `ref<T>` FORTE, onde refcount=0 é convenção já
/// estabelecida (e provada, `tramp_get_callback_system`) pra "handle raw sem dono". DECISÃO
/// PRAGMÁTICA: `.reds` agora declara `GetEntity() -> ref<Entity>` (divergência documentada do
/// Codeware real) — mesmo padrão 100% provado, sem RE nova de WeakRefCount. Trade-off honesto:
/// o handle devolvido é "raw"/sem dono (não incrementa nem decrementa o refcount real do
/// entity, não detecta destruição) — seguro pra uso SÍNCRONO no mesmo dispatch (caso de uso
/// normal), mas um mod que guardasse o handle entre frames podia ficar com ponteiro pendurado se
/// o entity fosse destruído nesse meio-tempo (não testado neste round, fora de escopo).
static ENTITYLIFECYCLE_STATES: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn tramp_ele_get_entity(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut entity: usize = 0;
    if let Ok(states) = ENTITYLIFECYCLE_STATES.lock() {
        if let Some((_, e)) = states.iter().find(|(p, _)| *p == c as usize) {
            entity = *e;
        }
    }
    write_handle_ret(out, entity as u64); // ref<Entity> "raw": {ptr,0} — mesmo padrão provado de GetCallbackSystem
}

pub unsafe fn register_entitylifecycleevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_entitylifecycleevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "EntityLifecycleEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_entitylifecycleevent: forja da classe FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_entitylifecycleevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "EntityLifecycleEvent", proto, "GetEntity", "GetEntity", tramp_ele_get_entity, false);
    format!("[reg] EntityLifecycleEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetEntity={m1}")
}

/// Constrói um `EntityLifecycleEvent` REAL (mesma técnica de `make_keyinputevent_arg`) — o
/// OBJETO EVENTO em si é um `ref<>` FORTE (`make_handle`); o campo `entity` que `GetEntity()`
/// devolve é um `ref<Entity>` "raw" (ver nota de `wref` revertido acima em `tramp_ele_get_entity`).
/// `entity_ptr` = o ponteiro do player JÁ CAPTURADO com segurança pelo `cp77_tick` (mesmo ponteiro
/// usado pra `Player/Spawned` — não precisa de RE nova pra achá-lo). `None` se a construção
/// falhar — o caller descarta, não crasha.
pub unsafe fn make_entitylifecycleevent_arg(entity_ptr: *mut c_void) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "EntityLifecycleEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = ENTITYLIFECYCLE_STATES.lock() {
        states.push((obj as usize, entity_ptr as usize));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// `ResourceEvent extends CallbackSystemEvent` — `cw-controller-misc` (2026-07-19). Fonte real:
/// `public native class ResourceEvent extends CallbackSystemEvent { GetResource()->ref<CResource>;
/// GetPath()->ResRef; GetJobGroup()->JobGroup }` (Codeware `ResourceEvent.reds`/`.hpp`). MESMA
/// receita 100% segura de `GameSessionEvent`/`KeyInputEvent` (registry Rust por-instância +
/// `register_type_instantiable_with_parent`, parent `CallbackSystemEvent` real).
///
/// **Divergência documentada:** `GetPath()` devolve `Uint64` — o hash FNV-1a64 do `ResourcePath`,
/// a MESMA representação canônica que `resource.link`/`reslinkdump`/`bwms-hashes::resource_path_
/// hash` usam em TODO o resto do projeto (nenhum resultado novo é "menos real": é como este
/// engine trata paths internamente, RED4ext/CET fazem igual) — em vez do tipo `ResRef` real
/// (marshalling de retorno String/ResRef é RE nova, fora de escopo desta rodada, mesma categoria
/// do gap residual documentado em `array:String` do TweakDB). `GetResource`/`GetJobGroup` fora
/// (não fazem parte do `proof_needed` literal, que só pede "path correto").
static RESOURCEEVENT_STATES: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());

unsafe extern "C" fn tramp_re_get_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u64;
    if let Ok(states) = RESOURCEEVENT_STATES.lock() {
        if let Some((_, p)) = states.iter().find(|(p, ..)| *p == c as usize) {
            v = *p;
        }
    }
    crate::log(&format!("[re] ResourceEvent.GetPath() on {c:p} -> {v:#018x}"));
    write_uint_ret(out, v, 8);
}

pub unsafe fn register_resourceevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_resourceevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "ResourceEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_resourceevent: forja da classe FALHOU".into();
    }
    let proto = match rtti::resolve_any(reg, &["gameGodModeSystem"], "AddGodMode") {
        Some(rf) => rf.func,
        None => return "[reg] register_resourceevent: classe=OK mas sem protótipo — GetPath NÃO registrado".into(),
    };
    let m1 = register_method(reg, "ResourceEvent", proto, "GetPath", "GetPath", tramp_re_get_path, false);
    format!("[reg] ResourceEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetPath={m1}")
}

/// Constrói um `ResourceEvent` REAL com o hash do path observado pelo hook `resource.link`
/// (`selftest::WATCH_RES_HASH`, só dispara quando o jogo de fato constrói aquele `ResourcePath`) e
/// devolve um `Arg::Handle` pronto pra `fire_event_args("Resource/Load", ...)`. `None` se a
/// construção falhar — o caller descarta, não crasha.
pub unsafe fn make_resourceevent_arg(path_hash: u64) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "ResourceEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = RESOURCEEVENT_STATES.lock() {
        states.push((obj as usize, path_hash));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
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
    let oka1 = register_global(&reg, proto, "BwmsAutoContinueOn", "BwmsAutoContinueOn", tramp_autocontinue_on);
    let oka2 = register_global(&reg, proto, "BwmsAutoContinueOff", "BwmsAutoContinueOff", tramp_autocontinue_off);
    let _ = register_global(&reg, proto, "BwmsFireStartOn", "BwmsFireStartOn", tramp_fire_start_on);
    let _ = register_global(&reg, proto, "BwmsFireStartOff", "BwmsFireStartOff", tramp_fire_start_off);
    let okfss = register_global(&reg, proto, "BwmsFireStartState", "BwmsFireStartState", tramp_fire_start_state);
    crate::log(&format!("[reg] register_all: BwmsFireStartState={okfss} (níveis 1+2 do seletor usam o lever)"));
    let _ = register_global(&reg, proto, "BwmsAcFired", "BwmsAcFired", tramp_ac_fired);
    let _ = register_global(&reg, proto, "BwmsMenuReadySplashOff", "BwmsMenuReadySplashOff", tramp_menu_ready_splash_off);
    // Skill 1 (VER-O-V, câmera 3ª pessoa autônoma) — lê ~/.bwms-tppcam. Ver bwms-tppcam.reds.
    let oktpp = register_global(&reg, proto, "BwmsTppState", "BwmsTppState", tramp_tpp_state);
    // Mod FULL BODY (ver o próprio corpo em 1ª pessoa, offset da câmera tunável) — eixos X/Y/Z em cm.
    let _okcb = register_global(&reg, proto, "BwmsCamBack", "BwmsCamBack", tramp_cam_back);
    let _okcx = register_global(&reg, proto, "BwmsCamX", "BwmsCamX", tramp_cam_x);
    let _okcz = register_global(&reg, proto, "BwmsCamZ", "BwmsCamZ", tramp_cam_z);
    let _okfl = register_global(&reg, proto, "BwmsForceLook", "BwmsForceLook", tramp_force_look);
    // Skill 2 (EQUIPAR roupa por código, fazer aparecer no V) — lê ~/.bwms-equip. Ver bwms-tppcam.reds.
    let okeq = register_global(&reg, proto, "BwmsEquipState", "BwmsEquipState", tramp_equip_state);
    let _okel = register_argful_by_types(&reg, proto, &["Int32"], "BwmsEquipLog", "BwmsEquipLog", tramp_equip_log);
    let okinv = register_global(&reg, proto, "BwmsInvState", "BwmsInvState", tramp_inv_state);
    let _okec = register_global(&reg, proto, "BwmsEquipCheck", "BwmsEquipCheck", tramp_equip_check);
    let _okrb = register_argful_by_types(&reg, proto, &["Int32", "TweakDBID"], "BwmsEquipReadback", "BwmsEquipReadback", tramp_equip_readback);
    crate::log(&format!("[reg] register_all: BwmsTppState={oktpp} BwmsEquipState={okeq} BwmsInvState={okinv} (Skills 1+2)"));
    let _ = register_global(&reg, proto, "BwmsDbgMeta0", "BwmsDbgMeta0", tramp_dbg_meta0);
    let _ = register_global(&reg, proto, "BwmsDbgHasT", "BwmsDbgHasT", tramp_dbg_hast);
    let _ = register_global(&reg, proto, "BwmsDbgHasF", "BwmsDbgHasF", tramp_dbg_hasf);
    let okdt = register_argful_by_types(&reg, proto, &["Int32"], "BwmsDbgTry", "BwmsDbgTry", tramp_dbg_try);
    let _okds = register_argful_by_types(&reg, proto, &["Int32"], "BwmsDbgSkip", "BwmsDbgSkip", tramp_dbg_skip);
    crate::log(&format!(
        "[reg] register_all: BwmsAutoContinue read={ok2} On={oka1} Off={oka2} DbgTry={okdt} (toggle pular-até-o-jogo)"
    ));
    // IA Fase 0: ponte game->processo-externo (enfileira evento, non-blocking).
    let ok3 = register_global(&reg, proto, "BwmsEmit", "BwmsEmit", tramp_bwms_emit);
    crate::log(&format!("[reg] register_all: BwmsEmit registrado={ok3} (IA Fase 0)"));
    // Skip-intro: sinal preciso da engagement do boot (redscript → present injeta o proceed "E").
    let oke1 = register_global(&reg, proto, "BwmsEngagementOn", "BwmsEngagementOn", tramp_engagement_on);
    let oke2 = register_global(&reg, proto, "BwmsEngagementOff", "BwmsEngagementOff", tramp_engagement_off);
    // LEVER do proceed nativo (gate ~/.bwms-proceed-native, OFF por padrão = read-only). 0-arg -> Bool.
    let okpr = register_global(&reg, proto, "BwmsProceed", "BwmsProceed", tramp_proceed);
    crate::log(&format!("[reg] register_all: BwmsProceed registrado={okpr} (proceed nativo 0x103f70e10, gated)"));
    // LEVER da ação 'Start' (gate ~/.bwms-fire-start, OFF por padrão). Seta ctx+0x572=1. 0-arg -> Bool.
    let okfs = register_global(&reg, proto, "BwmsFireStart", "BwmsFireStart", tramp_fire_start);
    crate::log(&format!("[reg] register_all: BwmsFireStart registrado={okfs} (injeta a acao Start via ctx+0x572, gated)"));
    crate::log(&format!("[reg] register_all: BwmsEngagementOn={oke1} Off={oke2} (skip-intro)"));
    // Toggle do skip-intro pela UI (persiste no marcador ~/.bwms-skipintro).
    let oks1 = register_global(&reg, proto, "BwmsSkipIntroOn", "BwmsSkipIntroOn", tramp_skipintro_on);
    let oks2 = register_global(&reg, proto, "BwmsSkipIntroOff", "BwmsSkipIntroOff", tramp_skipintro_off);
    let oks3 = register_global(&reg, proto, "BwmsSkipIntroState", "BwmsSkipIntroState", tramp_skipintro_state);
    crate::log(&format!("[reg] register_all: BwmsSkipIntro On={oks1} Off={oks2} State={oks3} (toggle UI)"));
    // Avanço de sessão pregame (arma o save-system sem input) — gate próprio ~/.bwms-session-advance (OFF).
    let oksa = register_global(&reg, proto, "BwmsSessionAdvance", "BwmsSessionAdvance", tramp_session_advance);
    crate::log(&format!("[reg] register_all: BwmsSessionAdvance={oksa} (avança fase 1->2 no fim-de-load)"));
    // Toggle do CPVR (VR) — DEV-ONLY (feature `cpvr`); marcador ~/.bwms-cpvr lido pelos reds do CPVR.
    #[cfg(feature = "cpvr")]
    {
        let okv1 = register_global(&reg, proto, "BwmsCpvrOn", "BwmsCpvrOn", tramp_cpvr_on);
        let okv2 = register_global(&reg, proto, "BwmsCpvrOff", "BwmsCpvrOff", tramp_cpvr_off);
        let okv3 = register_global(&reg, proto, "BwmsCpvrState", "BwmsCpvrState", tramp_cpvr_state);
        crate::log(&format!("[reg] register_all: BwmsCpvr On={okv1} Off={okv2} State={okv3} (toggle VR, dev)"));
        let okm1 = register_global(&reg, proto, "BwmsCpvrMode", "BwmsCpvrMode", tramp_cpvr_mode);
        let okm2 = register_global(&reg, proto, "BwmsCpvrModeNext", "BwmsCpvrModeNext", tramp_cpvr_mode_next);
        let okm3 = register_global(&reg, proto, "BwmsCpvrModePrev", "BwmsCpvrModePrev", tramp_cpvr_mode_prev);
        crate::log(&format!("[reg] register_all: BwmsCpvrMode={okm1} Next={okm2} Prev={okm3} (seletor VR, dev)"));
        let okmp = register_global(&reg, proto, "BwmsCpvrStereoPing", "BwmsCpvrStereoPing", tramp_cpvr_stereo_ping);
        let okap = register_global(&reg, proto, "BwmsCpvrApplyPing", "BwmsCpvrApplyPing", tramp_cpvr_apply_ping);
        let okv2 = register_global(&reg, proto, "BwmsCpvrIsV2", "BwmsCpvrIsV2", tramp_cpvr_is_v2);
        crate::log(&format!("[reg] register_all: BwmsCpvrStereoPing={okmp} ApplyPing={okap} IsV2={okv2} (diag v2.0)"));
        // MECANISMO 7: BwmsCamScan(x,y,z: Float) -> Bool. Scanner nativo de memoria (nosso tool != Frida).
        let okcs = register_argful_by_types(&reg, proto, &["Float", "Float", "Float"], "BwmsCamScan", "BwmsCamScan", tramp_camscan);
        crate::log(&format!("[reg] register_all: BwmsCamScan registrado={okcs} (scanner nativo da camera, mec 7)"));
    }
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
    // READ-ONLY: dump da state-machine da engagement (validação da LEVER C, sem escrita). 1-arg handle.
    let params_h = compose_params_from_types(&reg, &["handle:IScriptable"]);
    if params_h.1 == 1 {
        let oke = register_global_composed(&reg, proto, params_h, "BwmsEngDump", "BwmsEngDump", tramp_engdump);
        crate::log(&format!("[reg] register_all: BwmsEngDump registrado={oke} (dump read-only do SM da engagement)"));
    }
    // CallbackSystem RegisterCallback(CName, ref<IScriptable>, CName)->Bool (3-arg via GetType).
    let params_ccc = compose_params_from_types(&reg, &["CName", "handle:IScriptable", "CName"]);
    if params_ccc.1 == 3 {
        let ok9 = register_global_composed(&reg, proto, params_ccc, "BwmsRegisterCallback", "BwmsRegisterCallback", tramp_register_callback);
        crate::log(&format!("[reg] register_all: BwmsRegisterCallback registrado={ok9} (CallbackSystem RegisterCallback)"));
    }
    // Breadth Codeware: prova do register_method com REALLOC (gated ~/.bwms-regmethod-test).
    regmethod_selftest(&reg);
    // cw-utils: Utils/Bits.reds (16 funções, nomes reais do Codeware) — só args numéricos, não
    // esbarra na represa de String (Hash.reds/Number.reds ficam de fora até resolver script_ref<String>).
    register_bits_utils(&reg, proto);
    // cw-utils: Utils/Hash.reds (FNV1a64/32 + Murmur3) — usa a leitura de String nativa nova
    // (read_cstring), CODADO sem confirmação in-game ainda (ver nota em register_hash_utils).
    register_hash_utils(&reg, proto);
    // cw-utils: Utils/Number.reds (ParseInt8..64 + ParseUint8..64) — mesma leitura de String nova,
    // CODADO sem confirmação in-game ainda (ver nota em register_number_utils).
    register_number_utils(&reg, proto);
    // cw-utils: Utils/String.reds (só UTF8StrLen — as outras 5 retornam String, represa de
    // escrita ainda aberta, ver nota em register_string_utils).
    register_string_utils(&reg, proto);
    // cw-utils: CName/CRUID/NodeRef(parcial)/Logging.reds — identidades u64 triviais + print no
    // dev-log (ver nota em register_hash_wrapper_utils).
    register_hash_wrapper_utils(&reg, proto);
    // cw-callbacksystem-rtti: getter do singleton `CallbackSystem`, exposto como GLOBAL (não
    // `@addMethod(GameInstance)` — essa via CRASHA o boot, ver nota longa em
    // `register_callbacksystem`: "GameInstance" é validado cedo demais, antes de QUALQUER hook
    // nosso rodar). `register_all()` roda uma vez só (guard acima) — seguro registrar direto aqui,
    // mesmo padrão de BlackwallPing/BwmsAutoContinue etc.
    let okcbs = register_global(&reg, proto, "BwmsGetCallbackSystem", "BwmsGetCallbackSystem", tramp_get_callback_system);
    crate::log(&format!("[reg] register_all: BwmsGetCallbackSystem registrado={okcbs}"));
    // `cw-scriptableservice` — mesmo padrão (getter global em vez de @addMethod(GameInstance), que
    // crasharia pelo mesmo motivo do GetCallbackSystem).
    let okssc = register_global(&reg, proto, "BwmsGetScriptableServiceContainer", "BwmsGetScriptableServiceContainer", tramp_get_scriptableservicecontainer);
    crate::log(&format!("[reg] register_all: BwmsGetScriptableServiceContainer registrado={okssc}"));
    // `cw-event-target-classes` — global de TESTE (2026-07-18), ver `tramp_make_test_keyinputevent`.
    let okkie = register_global(&reg, proto, "BwmsMakeTestKeyInputEvent", "BwmsMakeTestKeyInputEvent", tramp_make_test_keyinputevent);
    crate::log(&format!("[reg] register_all: BwmsMakeTestKeyInputEvent registrado={okkie}"));
    // Codeware Facade (Codeware.Require/Version): NÃO chamar aqui — register_all() roda EM TODO
    // TICK (idempotente pras outras natives via checagem de re-resolve, mas register_method NÃO
    // dedupa: chamar de novo aqui empilharia Version/Require DUPLICADOS a cada tick). A represa de
    // timing (classe precisa existir ANTES do bind eager, ~6s, bem antes do 1º tick) é resolvida
    // em `selfboot.rs::class_validate_probe_hook` (2026-07-13): forja a classe (register_type_min)
    // + registra os métodos NA HORA CERTA, 1x só (guard CODEWARE_REGISTERED), gated
    // ~/.bwms-classvalidate-probe. Ver [[cp77-codeware-port]].
    // `redscript-mod-persistence` (2026-07-13): config EXTERNO ao save (regra-mãe do projeto —
    // "nada escreve dado de mod dentro do save"), pra estado de cheat/mod sobreviver entre
    // sessões/saves diferentes sem tocar o save-file. Só 2 globais (mesmo padrão de
    // register_argful_by_types já provado 20+ vezes esta sessão, zero risco novo).
    let okcg = register_argful_by_types(&reg, proto, &["String"], "BwmsConfigGet", "BwmsConfigGet", tramp_config_get);
    let okcs = register_argful_by_types(&reg, proto, &["String", "String"], "BwmsConfigSet", "BwmsConfigSet", tramp_config_set);
    crate::log(&format!("[reg] register_all: BwmsConfigGet={okcg} BwmsConfigSet={okcs} (persistência externa ao save)"));
    // Bisecção do crash full-body (2026-07-15): contador de ciclos EXATO por poller.
    let okpt = register_argful_by_types(&reg, proto, &["String", "Int32"], "BwmsPollerTick", "BwmsPollerTick", tramp_poller_tick);
    crate::log(&format!("[reg] register_all: BwmsPollerTick={okpt} (contador de ciclos, bisecção do crash full-body)"));
}

// ===== `redscript-mod-persistence`: `BwmsConfigGet(key)->String` / `BwmsConfigSet(key,value)->Bool`,
// arquivo flat `~/.bwms-modconfig.txt` (1 "key=value" por linha, key sem "="; NUNCA o save-file). =====
fn modconfig_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-modconfig.txt"))
}

/// Chave saneada: sem `=` nem quebra de linha (quebrariam o formato "key=value"/1-linha).
fn config_sanitize_key(key: &str) -> String {
    key.chars().filter(|&c| c != '=' && c != '\n' && c != '\r').collect()
}
/// Valor saneado: sem quebra de linha (injetaria uma linha/chave falsa no arquivo).
fn config_sanitize_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}
/// Lê o valor de `key` do conteúdo do config (1º match; formato "key=value" por linha). PURA.
fn config_get_from(content: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    content
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or("")
        .to_string()
}
/// Upsert PURO: devolve o conteúdo novo com `key=value`, COLAPSANDO duplicatas da chave numa linha
/// só (o writer antigo reescrevia CADA linha da chave, deixando duplicatas), preservando as outras
/// na ordem e dropando linhas vazias. Base testável do `modconfig_set`.
fn config_upsert(existing: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut out = String::new();
    let mut written = false;
    for line in existing.lines() {
        if line.starts_with(&prefix) {
            if !written {
                out.push_str(&prefix);
                out.push_str(value);
                out.push('\n');
                written = true;
            }
            // duplicatas seguintes da MESMA chave: dropadas (colapso)
        } else if !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !written {
        out.push_str(&prefix);
        out.push_str(value);
        out.push('\n');
    }
    out
}
fn modconfig_get(key: &str) -> String {
    let key = config_sanitize_key(key);
    let Some(p) = modconfig_path() else { return String::new() };
    let Ok(content) = std::fs::read_to_string(&p) else { return String::new() };
    config_get_from(&content, &key)
}
fn modconfig_set(key: &str, value: &str) -> bool {
    let key = config_sanitize_key(key);
    if key.is_empty() {
        return false;
    }
    let value = config_sanitize_value(value);
    let Some(p) = modconfig_path() else { return false };
    let existing = std::fs::read_to_string(&p).unwrap_or_default();
    let out = config_upsert(&existing, &key, &value);
    // Escrita ATÔMICA: grava num temp e renomeia (rename é atômico no mesmo volume). Se o processo
    // morrer no meio, o config ANTIGO fica intacto — antes um `write` in-place truncado o corrompia.
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, &out).is_err() {
        return false;
    }
    std::fs::rename(&tmp, &p).is_ok()
}
unsafe extern "C" fn tramp_config_get(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let key = strings.first().cloned().flatten().unwrap_or_default();
    let v = modconfig_get(&key);
    let ok = write_cstring_inline_ret(out, &v);
    crate::log(&format!("[modconfig] BwmsConfigGet('{key}') = '{v}' (escrito={ok})"));
}
unsafe extern "C" fn tramp_config_set(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let key = strings.first().cloned().flatten().unwrap_or_default();
    let value = strings.get(1).cloned().flatten().unwrap_or_default();
    let ok = modconfig_set(&key, &value);
    crate::log(&format!("[modconfig] BwmsConfigSet('{key}','{value}') -> {ok}"));
    write_bool_ret(out, ok);
}

/// `BwmsPollerTick(label: String, n: Int32) -> Void` — contador de ciclos EXATO por poller
/// (bisecção do crash `SystemsUpdater::Node::LinkJob_NoFence`, 2026-07-15): loga a cada 20
/// chamadas pra correlacionar o crash por CICLO em vez de só tempo de parede — a última linha
/// gravada em `/tmp/cp77-console.log` antes de um crash futuro dá o nº exato de ciclos que
/// aquele poller alcançou.
unsafe extern "C" fn tramp_poller_tick(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let label = strings.first().cloned().flatten().unwrap_or_default();
    let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    crate::log(&format!("[poller-tick] {label} n={n}"));
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

// ===== cw-utils: Codeware `Utils/Bits.reds` (2026-07-13) — SÓ argumentos numéricos, então NÃO
// esbarra na represa de String (script_ref<String> ainda sem RE — Hash.reds/Number.reds ficam
// de fora por isso). Fecha uma fatia real de `cw-utils` com compatibilidade de nome EXATA da
// API real do Codeware (mods de Windows que chamam BitTest32/etc. por nome funcionam igual).
#[inline]
fn bit_test(value: u64, n: i32) -> bool {
    if !(0..64).contains(&n) {
        return false;
    }
    (value >> n) & 1 != 0
}
#[inline]
fn bit_set(value: u64, n: i32, state: bool) -> u64 {
    if !(0..64).contains(&n) {
        return value;
    }
    if state { value | (1u64 << n) } else { value & !(1u64 << n) }
}
#[inline]
fn bit_shl(value: u64, n: i32, width_bits: u32) -> u64 {
    if n < 0 {
        return value;
    }
    let shifted = value.wrapping_shl(n as u32);
    let mask = if width_bits >= 64 { u64::MAX } else { (1u64 << width_bits) - 1 };
    shifted & mask
}
#[inline]
fn bit_shr(value: u64, n: i32) -> u64 {
    if n < 0 { value } else { value.wrapping_shr(n as u32) }
}

/// Gera os 4 trampolines (`Test`/`Set`/`ShiftL`/`ShiftR`) de UMA largura de bits. `read_width` =
/// bytes do arg `value` (1/2/4/8); `width_bits` = bits totais (pro mask do shift-left).
macro_rules! bits_width_trampolines {
    ($test_fn:ident, $set_fn:ident, $shl_fn:ident, $shr_fn:ident, $read_width:literal, $width_bits:literal) => {
        unsafe extern "C" fn $test_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_bool_ret(out, bit_test(value, n));
        }
        unsafe extern "C" fn $set_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            let state = args.get(2).map(|(v, _)| *v != 0).unwrap_or(false);
            write_uint_ret(out, bit_set(value, n, state), $read_width);
        }
        unsafe extern "C" fn $shl_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_uint_ret(out, bit_shl(value, n, $width_bits), $read_width);
        }
        unsafe extern "C" fn $shr_fn(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let args = rtti::read_params_consuming(current_native_func(), frame);
            let value = mask_arg(args.first().map(|(v, _)| *v).unwrap_or(0), $read_width);
            let n = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
            write_uint_ret(out, bit_shr(value, n), $read_width);
        }
    };
}
#[inline]
fn mask_arg(raw: u64, width_bytes: u8) -> u64 {
    match width_bytes {
        1 => raw as u8 as u64,
        2 => raw as u16 as u64,
        4 => raw as u32 as u64,
        _ => raw,
    }
}
bits_width_trampolines!(tramp_bittest8, tramp_bitset8, tramp_bitshl8, tramp_bitshr8, 1, 8);
bits_width_trampolines!(tramp_bittest16, tramp_bitset16, tramp_bitshl16, tramp_bitshr16, 2, 16);
bits_width_trampolines!(tramp_bittest32, tramp_bitset32, tramp_bitshl32, tramp_bitshr32, 4, 32);
bits_width_trampolines!(tramp_bittest64, tramp_bitset64, tramp_bitshl64, tramp_bitshr64, 8, 64);

/// Registra as 16 funções de `Utils/Bits.reds` (nomes EXATOS da API real do Codeware — mods de
/// Windows que chamam `BitTest32(x, n)` etc. funcionam sem tradução). `proto` = doador de vtable
/// já resolvido em `register_all` (mesmo usado por BwmsDbgTry/etc.).
unsafe fn register_bits_utils(reg: &Registry, proto: *mut c_void) {
    let uint_t = |bits: u32| if bits == 8 { "Uint8" } else if bits == 16 { "Uint16" } else if bits == 32 { "Uint32" } else { "Uint64" };
    let mut n_ok = 0usize;
    macro_rules! reg1 {
        ($bits:literal, $test_fn:ident, $set_fn:ident, $shl_fn:ident, $shr_fn:ident) => {
            let u = uint_t($bits);
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BitTest", $bits), concat!("BitTest", $bits), $test_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32", "Bool"], concat!("BitSet", $bits), concat!("BitSet", $bits), $set_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BitShiftL", $bits), concat!("BitShiftL", $bits), $shl_fn) { n_ok += 1; }
            if register_argful_by_types(reg, proto, &[u, "Int32"], concat!("BitShiftR", $bits), concat!("BitShiftR", $bits), $shr_fn) { n_ok += 1; }
        };
    }
    reg1!(8, tramp_bittest8, tramp_bitset8, tramp_bitshl8, tramp_bitshr8);
    reg1!(16, tramp_bittest16, tramp_bitset16, tramp_bitshl16, tramp_bitshr16);
    reg1!(32, tramp_bittest32, tramp_bitset32, tramp_bitshl32, tramp_bitshr32);
    reg1!(64, tramp_bittest64, tramp_bitset64, tramp_bitshl64, tramp_bitshr64);
    crate::log(&format!("[reg] register_all: cw-utils Bits.reds {n_ok}/16 registradas (nomes reais do Codeware)"));
}

// ===== cw-utils: Codeware `Utils/Hash.reds` (2026-07-13) — fecha a represa de leitura de
// `String` nativa via `read_cstring` (layout confirmado pelo RED4ext.SDK vendorizado, ver
// [[cp77-codeware-port]]). Assinatura real (`enablers/Codeware/src/App/Utils/Hashing.hpp`):
// `FNV1a64(data: script_ref<String>, opt seed: Uint64=0xCBF29CE484222325) -> Uint64` (e
// variantes 32-bit/Murmur3). Registrado com `String` puro (não `script_ref<String>` — o nome
// EXATO do tipo wrapper no RTTI é incerto sem teste ao vivo; `String` reduz risco de bind
// falhar, ao custo de possível incompatibilidade estrita com mods reais que dependam do
// wrapper — ajustar se a 1ª prova ao vivo mostrar que não bindou). CODADO, SEM confirmação
// in-game ainda (ambiente bloqueado no momento da implementação).
unsafe extern "C" fn tramp_fnv1a64(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v).unwrap_or(0xcbf2_9ce4_8422_2325);
    let hash = bwms_hashes::fnv1a64_seeded(data.as_bytes(), seed);
    crate::log(&format!("[hashutils] FNV1a64('{data}', seed={seed:#x}) = {hash:#x}"));
    write_uint_ret(out, hash, 8);
}
unsafe extern "C" fn tramp_fnv1a32(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0x811c_9dc5);
    let hash = bwms_hashes::fnv1a32_seeded(data.as_bytes(), seed);
    crate::log(&format!("[hashutils] FNV1a32('{data}', seed={seed:#x}) = {hash:#x}"));
    write_uint_ret(out, hash as u64, 4);
}
unsafe extern "C" fn tramp_murmur3(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let data = strings.first().cloned().flatten().unwrap_or_default();
    let seed = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0x5eed_ba5e);
    let hash = bwms_hashes::murmur3_32(data.as_bytes(), seed);
    crate::log(&format!("[hashutils] Murmur3('{data}', seed={seed:#x}) = {hash:#x}"));
    write_uint_ret(out, hash as u64, 4);
}

/// Registra as 3 funções de `Utils/Hash.reds` (nomes reais). Ver nota acima sobre o tipo do
/// 1º param (`String` em vez de `script_ref<String>`).
unsafe fn register_hash_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String", "Uint64"], "FNV1a64", "FNV1a64", tramp_fnv1a64);
    let ok2 = register_argful_by_types(reg, proto, &["String", "Uint32"], "FNV1a32", "FNV1a32", tramp_fnv1a32);
    let ok3 = register_argful_by_types(reg, proto, &["String", "Uint32"], "Murmur3", "Murmur3", tramp_murmur3);
    crate::log(&format!("[reg] register_all: cw-utils Hash.reds FNV1a64={ok1} FNV1a32={ok2} Murmur3={ok3}"));
}

// ===== cw-utils: Codeware `Utils/Number.reds` (2026-07-13) — ParseInt8/16/32/64 + ParseUint8/16/
// 32/64, mesma represa de String que Hash.reds acabou de fechar (`read_cstring`). Fonte real
// (`enablers/Codeware/src/App/Utils/Number.hpp`): `T ParseInt<T>(str, opt base=10)` — chama
// `strtoll`/`strtoull` (Temp=int64_t p/ signed, uint64_t p/ unsigned), e SE `end != str+len` (sobrou
// lixo não consumido) retorna 0; senão trunca (`static_cast<T>`) pro tipo alvo. Portado fielmente:
// espaço em branco à esquerda ignorado, sinal opcional (`strtoull` aceita `-` também — nega mod
// 2^64, igual ao glibc), base 0 = auto-detecta `0x`/`0X`->16 ou `0`->8 senão 10; overflow CLAMPA em
// i64::MIN/MAX ou u64::MAX (mimetiza o comportamento real do strtoll/strtoull sem checar errno) em
// vez de dar panic/wrap arbitrário. Exige consumir a string INTEIRA (equivalente ao `end` check) —
// qualquer caractere sobrando (mesmo espaço à direita) já falha e retorna 0, igual ao C++.
fn parse_number_core(s: &str, base_in: i32, accept_leading_minus_unsigned: bool) -> Option<(usize, bool, u128)> {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
        i += 1;
    }
    let mut neg = false;
    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
        if bytes[i] == b'-' && !accept_leading_minus_unsigned {
            neg = true;
        } else if bytes[i] == b'-' {
            neg = true; // strtoull aceita '-' também (nega mod 2^64 no fim)
        }
        i += 1;
    }
    let mut base = base_in;
    if base == 0 {
        if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] | 0x20) == b'x' {
            base = 16;
            i += 2;
        } else if i < bytes.len() && bytes[i] == b'0' {
            base = 8;
            i += 1;
        } else {
            base = 10;
        }
    } else if base == 16 && i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] | 0x20) == b'x' {
        i += 2;
    }
    if !(2..=36).contains(&base) {
        return None;
    }
    let digit_start = i;
    let mut acc: u128 = 0;
    while i < bytes.len() {
        let c = bytes[i];
        let d = match c {
            b'0'..=b'9' => (c - b'0') as u32,
            b'a'..=b'z' => (c - b'a' + 10) as u32,
            b'A'..=b'Z' => (c - b'A' + 10) as u32,
            _ => break,
        };
        if d as i32 >= base {
            break;
        }
        acc = acc.saturating_mul(base as u128).saturating_add(d as u128);
        acc = acc.min(u64::MAX as u128);
        i += 1;
    }
    if i == digit_start {
        return None; // nenhum dígito consumido (strtoll deixa end==nptr, sempre falha o whole-string check)
    }
    Some((i, neg, acc))
}
/// `strtoll`-like: retorna `None` se não consumir a string inteira ou faltar dígito; clampa em
/// i64::MIN/MAX no overflow (mimetiza strtoll sem checar errno).
fn strtoll_like(s: &str, base: i32) -> Option<i64> {
    let (consumed, neg, acc) = parse_number_core(s, base, false)?;
    if consumed != s.len() {
        return None;
    }
    Some(if neg {
        if acc > (i64::MAX as u128) + 1 { i64::MIN } else { (acc as i64).wrapping_neg() }
    } else if acc > i64::MAX as u128 {
        i64::MAX
    } else {
        acc as i64
    })
}
/// `strtoull`-like: aceita `-` líder (nega mod 2^64, igual ao glibc); clampa em u64::MAX no overflow.
fn strtoull_like(s: &str, base: i32) -> Option<u64> {
    let (consumed, neg, acc) = parse_number_core(s, base, true)?;
    if consumed != s.len() {
        return None;
    }
    let v = acc.min(u64::MAX as u128) as u64;
    Some(if neg { v.wrapping_neg() } else { v })
}

macro_rules! parse_int_trampoline {
    ($name:ident, $width:literal) => {
        unsafe extern "C" fn $name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let func = current_native_func();
            let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
            let s = strings.first().cloned().flatten().unwrap_or_default();
            let base = args.get(1).map(|(v, _)| *v as i32).unwrap_or(10);
            let val = strtoll_like(&s, base).unwrap_or(0);
            crate::log(&format!("[numutils] ParseInt{}('{s}', base={base}) = {val}", $width));
            write_uint_ret(out, val as u64, $width / 8);
        }
    };
}
macro_rules! parse_uint_trampoline {
    ($name:ident, $width:literal) => {
        unsafe extern "C" fn $name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
            let func = current_native_func();
            let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
            let s = strings.first().cloned().flatten().unwrap_or_default();
            let base = args.get(1).map(|(v, _)| *v as i32).unwrap_or(10);
            let val = strtoull_like(&s, base).unwrap_or(0);
            crate::log(&format!("[numutils] ParseUint{}('{s}', base={base}) = {val}", $width));
            write_uint_ret(out, val, $width / 8);
        }
    };
}
parse_int_trampoline!(tramp_parseint8, 8);
parse_int_trampoline!(tramp_parseint16, 16);
parse_int_trampoline!(tramp_parseint32, 32);
parse_int_trampoline!(tramp_parseint64, 64);
parse_uint_trampoline!(tramp_parseuint8, 8);
parse_uint_trampoline!(tramp_parseuint16, 16);
parse_uint_trampoline!(tramp_parseuint32, 32);
parse_uint_trampoline!(tramp_parseuint64, 64);

/// Registra as 8 funções de `Utils/Number.reds` (nomes reais). Mesma incerteza do Hash.reds sobre
/// `String` vs `script_ref<String>` — ver comentário lá.
unsafe fn register_number_utils(reg: &Registry, proto: *mut c_void) {
    let mut n_ok = 0usize;
    macro_rules! reg1 {
        ($ret_t:literal, $name:literal, $fn:ident) => {
            if register_argful_by_types(reg, proto, &["String", "Int32"], $name, $name, $fn) {
                n_ok += 1;
            }
            let _ = $ret_t;
        };
    }
    reg1!("Int8", "ParseInt8", tramp_parseint8);
    reg1!("Int16", "ParseInt16", tramp_parseint16);
    reg1!("Int32", "ParseInt32", tramp_parseint32);
    reg1!("Int64", "ParseInt64", tramp_parseint64);
    reg1!("Uint8", "ParseUint8", tramp_parseuint8);
    reg1!("Uint16", "ParseUint16", tramp_parseuint16);
    reg1!("Uint32", "ParseUint32", tramp_parseuint32);
    reg1!("Uint64", "ParseUint64", tramp_parseuint64);
    crate::log(&format!("[reg] register_all: cw-utils Number.reds {n_ok}/8 registradas (nomes reais do Codeware)"));
}

// ===== cw-utils: Codeware `Utils/String.reds` (2026-07-13) — as 6 funções reais. `UTF8StrLen`
// é retorno escalar (Int32), mesmo padrão de Hash/Number.reds. As outras 5 retornam `String` —
// precisavam de um mecanismo de ESCREVER uma CString NOVA de volta pro `out` (represa distinta
// da de LER, que já tinha caído com `read_cstring`). Fechada na MESMA sessão: `call_func::res`
// era `[u8;16]` (dimensionado pro maior retorno já testado, Vector4) — alargar pra 0x20 (ver
// nota lá) tornou seguro escrever uma CString inteira sem transbordar. `write_cstring_inline_ret`
// só cobre o caminho SSO (<=19 bytes) — strings mais longas exigiriam heap-alloc no allocator do
// PRÓPRIO jogo (fora de escopo, risco real de corrupção se malfeito).
// Semântica fiel a `enablers/Codeware/src/App/Utils/String.hpp`: Left/Right/Mid operam em
// CODEPOINTS UTF-8 (não bytes, mesmo padrão de UTF8StrLen); Lower/Upper via
// `char::to_lowercase/to_uppercase` do Rust — mais completo que o `towlower/towupper` de
// 1-char-pra-1-char do C++ original (cobre expansões tipo alemão ß→"ss").
#[inline]
fn utf8_str_len(s: &str) -> i32 {
    s.chars().count() as i32
}
#[inline]
fn utf8_str_left(s: &str, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    s.chars().take(length as usize).collect()
}
#[inline]
fn utf8_str_right(s: &str, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    let total = s.chars().count() as i64;
    let skip = (total - length as i64).max(0) as usize;
    s.chars().skip(skip).collect()
}
#[inline]
fn utf8_str_mid(s: &str, offset: i32, length: i32) -> String {
    if length <= 0 {
        return String::new();
    }
    s.chars().skip(offset.max(0) as usize).take(length as usize).collect()
}
#[inline]
fn utf8_str_lower(s: &str) -> String {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}
#[inline]
fn utf8_str_upper(s: &str) -> String {
    s.chars().flat_map(|c| c.to_uppercase()).collect()
}

unsafe extern "C" fn tramp_utf8strlen(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let len = utf8_str_len(&s);
    crate::log(&format!("[strutils] UTF8StrLen('{s}') = {len}"));
    write_uint_ret(out, len as u32 as u64, 4);
}
unsafe extern "C" fn tramp_utf8strleft(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let length = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_left(&s, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log(&format!("[strutils] UTF8StrLeft('{s}',{length}) = '{r}' (escrito={ok})"));
}
unsafe extern "C" fn tramp_utf8strright(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let length = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_right(&s, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log(&format!("[strutils] UTF8StrRight('{s}',{length}) = '{r}' (escrito={ok})"));
}
unsafe extern "C" fn tramp_utf8strmid(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let offset = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let length = args.get(2).map(|(v, _)| *v as i32).unwrap_or(0);
    let r = utf8_str_mid(&s, offset, length);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log(&format!("[strutils] UTF8StrMid('{s}',{offset},{length}) = '{r}' (escrito={ok})"));
}
unsafe extern "C" fn tramp_utf8strlower(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let r = utf8_str_lower(&s);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log(&format!("[strutils] UTF8StrLower('{s}') = '{r}' (escrito={ok})"));
}
unsafe extern "C" fn tramp_utf8strupper(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let s = strings.first().cloned().flatten().unwrap_or_default();
    let r = utf8_str_upper(&s);
    let ok = write_cstring_inline_ret(out, &r);
    crate::log(&format!("[strutils] UTF8StrUpper('{s}') = '{r}' (escrito={ok})"));
}

/// Registra as 6 funções de `Utils/String.reds` (nomes reais).
unsafe fn register_string_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String"], "UTF8StrLen", "UTF8StrLen", tramp_utf8strlen);
    let ok2 = register_argful_by_types(reg, proto, &["String", "Int32"], "UTF8StrLeft", "UTF8StrLeft", tramp_utf8strleft);
    let ok3 = register_argful_by_types(reg, proto, &["String", "Int32"], "UTF8StrRight", "UTF8StrRight", tramp_utf8strright);
    let ok4 = register_argful_by_types(reg, proto, &["String", "Int32", "Int32"], "UTF8StrMid", "UTF8StrMid", tramp_utf8strmid);
    let ok5 = register_argful_by_types(reg, proto, &["String"], "UTF8StrLower", "UTF8StrLower", tramp_utf8strlower);
    let ok6 = register_argful_by_types(reg, proto, &["String"], "UTF8StrUpper", "UTF8StrUpper", tramp_utf8strupper);
    crate::log(&format!(
        "[reg] register_all: cw-utils String.reds Len={ok1} Left={ok2} Right={ok3} Mid={ok4} Lower={ok5} Upper={ok6}"
    ));
}

// ===== cw-utils: `Utils/CName.reds` + `Utils/CRUID.reds` + `Utils/NodeRef.reds` (parcial) +
// `Utils/Logging.reds` (2026-07-13, mesma sessão) — fontes reais (`enablers/Codeware/src/App/
// Utils/{CName,CRUID,NodeRef,Logging}.hpp`) confirmam: `HashToName`/`NameToHash` são IDENTIDADE
// pura (`Red::CName` É um `uint64_t` por baixo, `return aValue;` no C++ real); mesma coisa pra
// `HashToCRUID`/`CRUIDToHash` (`CRUID.unk00` na offset 0, reinterpret_cast direto) e
// `HashToNodeRef`/`NodeRefToHash` (`Red::NodeRef` também é só um `uint64_t`). Zero risco novo —
// só reinterpretar os mesmos 8 bytes já lidos como escalar/CName/handle pelo `read_params`
// existente. `CreateNodeRef` FICA DE FORA: chama `Raw::NodeRef::Create` (hash real do path do
// nó na cena, algoritmo específico do motor que não temos mapeado) — diferente de
// HashToNodeRef/NodeRefToHash, que só reinterpretam um hash JÁ CALCULADO. `Print`/`ModLog`
// chamam `Red::Log::Channel` (canal de log real do CET/engine, endereço não mapeado) — como
// pragmática de baixo risco, logam no NOSSO próprio canal de dev (`crate::log`) em vez de
// tentar achar o canal real do motor; satisfaz a assinatura (mods que chamam `Print("x")`
// funcionam, a mensagem só sai no log de dev em vez da UI oficial do CET).
unsafe extern "C" fn tramp_hashtoname(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_nametohash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_hashtocruid(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_cruidtohash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_hashtonoderef(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_noderef_to_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let v = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_uint_ret(out, v, 8);
}
unsafe extern "C" fn tramp_print(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let text = strings.first().cloned().flatten().unwrap_or_default();
    crate::log(&format!("[DEBUG] {text}"));
}
unsafe extern "C" fn tramp_modlog(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let mod_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mod_name = crate::cname::resolve_cname(mod_hash);
    let text = strings.get(1).cloned().flatten().unwrap_or_default();
    crate::log(&format!("[{mod_name}] {text}"));
}

/// Registra `Utils/CName.reds` (2/2), `Utils/CRUID.reds` (2/2), `Utils/NodeRef.reds` (2/3 —
/// `CreateNodeRef` fica de fora, ver nota acima) e `Utils/Logging.reds` (2/2, loga no dev-log).
unsafe fn register_hash_wrapper_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["Uint64"], "HashToName", "HashToName", tramp_hashtoname);
    let ok2 = register_argful_by_types(reg, proto, &["CName"], "NameToHash", "NameToHash", tramp_nametohash);
    let ok3 = register_argful_by_types(reg, proto, &["Uint64"], "HashToCRUID", "HashToCRUID", tramp_hashtocruid);
    let ok4 = register_argful_by_types(reg, proto, &["CRUID"], "CRUIDToHash", "CRUIDToHash", tramp_cruidtohash);
    let ok5 = register_argful_by_types(reg, proto, &["Uint64"], "HashToNodeRef", "HashToNodeRef", tramp_hashtonoderef);
    let ok6 = register_argful_by_types(reg, proto, &["NodeRef"], "NodeRefToHash", "NodeRefToHash", tramp_noderef_to_hash);
    let ok7 = register_argful_by_types(reg, proto, &["String"], "Print", "Print", tramp_print);
    let ok8 = register_argful_by_types(reg, proto, &["CName", "String"], "ModLog", "ModLog", tramp_modlog);
    crate::log(&format!(
        "[reg] register_all: cw-utils CName={ok1}/{ok2} CRUID={ok3}/{ok4} NodeRef={ok5}/{ok6} Logging={ok7}/{ok8}"
    ));
}

#[cfg(test)]
mod modconfig_tests {
    use super::{config_get_from, config_sanitize_key, config_sanitize_value, config_upsert};

    #[test]
    fn upsert_atualiza_e_preserva() {
        let existing = "godmode=1\nlegs=0\n";
        let out = config_upsert(existing, "legs", "1");
        assert_eq!(config_get_from(&out, "legs"), "1");
        assert_eq!(config_get_from(&out, "godmode"), "1"); // outras chaves preservadas
    }

    #[test]
    fn upsert_insere_chave_nova() {
        let out = config_upsert("a=1\n", "b", "2");
        assert_eq!(config_get_from(&out, "a"), "1");
        assert_eq!(config_get_from(&out, "b"), "2");
    }

    #[test]
    fn upsert_colapsa_duplicatas() {
        // arquivo corrompido com a chave repetida → set colapsa numa linha só (o writer antigo
        // reescrevia CADA ocorrência, mantendo as duplicatas).
        let existing = "k=1\nk=2\nother=9\nk=3\n";
        let out = config_upsert(existing, "k", "5");
        assert_eq!(out.lines().filter(|l| l.starts_with("k=")).count(), 1);
        assert_eq!(config_get_from(&out, "k"), "5");
        assert_eq!(config_get_from(&out, "other"), "9");
    }

    #[test]
    fn sanitiza_quebra_de_linha_e_igual() {
        // valor com '\n' não pode injetar uma linha/chave falsa.
        let v = config_sanitize_value("mau\nfake=999");
        assert!(!v.contains('\n'));
        let out = config_upsert("", "k", &v);
        assert_eq!(out.lines().count(), 1, "1 linha só, sem chave 'fake' injetada");
        assert_eq!(config_get_from(&out, "fake"), ""); // não vazou
        // chave com '=' é saneada (não quebra o parsing).
        assert_eq!(config_sanitize_key("a=b\nc"), "abc");
    }

    #[test]
    fn get_primeiro_match_e_vazio_ausente() {
        assert_eq!(config_get_from("k=1\nk=2\n", "k"), "1");
        assert_eq!(config_get_from("k=1\n", "ausente"), "");
    }
}

#[cfg(test)]
mod string_utils_tests {
    use super::{
        utf8_str_left, utf8_str_len, utf8_str_lower, utf8_str_mid, utf8_str_right, utf8_str_upper,
        write_cstring_inline_ret,
    };
    use std::os::raw::c_void;

    #[test]
    fn conta_ascii_simples() {
        assert_eq!(utf8_str_len("hello"), 5);
    }

    #[test]
    fn conta_codepoints_multibyte_nao_bytes() {
        // "café" = 4 codepoints, mas 5 bytes em UTF-8 (é 2 bytes) — prova que conta CODEPOINT.
        assert_eq!(utf8_str_len("café"), 4);
        assert_eq!("café".len(), 5); // bytes, pra deixar a diferença explícita
    }

    #[test]
    fn string_vazia() {
        assert_eq!(utf8_str_len(""), 0);
    }

    #[test]
    fn emoji_multibyte() {
        // emoji comuns são 4 bytes UTF-8, 1 codepoint cada.
        assert_eq!(utf8_str_len("👍👍"), 2);
    }

    #[test]
    fn left_right_mid_em_codepoints() {
        assert_eq!(utf8_str_left("hello world", 5), "hello");
        assert_eq!(utf8_str_right("hello world", 5), "world");
        assert_eq!(utf8_str_mid("hello world", 6, 5), "world");
        // multibyte: "café com leite" — Left(4) tem que pegar "café" (4 codepoints, 5 bytes),
        // não truncar no meio do 'é' como um corte por-byte faria.
        assert_eq!(utf8_str_left("café com leite", 4), "café");
    }

    #[test]
    fn left_right_mid_clampam_fora_do_alcance() {
        assert_eq!(utf8_str_left("hi", 100), "hi"); // pede mais que existe -> tudo
        assert_eq!(utf8_str_left("hi", 0), "");
        assert_eq!(utf8_str_left("hi", -5), "");
        assert_eq!(utf8_str_right("hi", 100), "hi");
        assert_eq!(utf8_str_mid("hi", 50, 5), ""); // offset além do fim -> vazio
    }

    #[test]
    fn lower_upper_ascii_e_acentuado() {
        assert_eq!(utf8_str_lower("HELLO"), "hello");
        assert_eq!(utf8_str_upper("hello"), "HELLO");
        assert_eq!(utf8_str_lower("CAFÉ"), "café");
        assert_eq!(utf8_str_upper("café"), "CAFÉ");
    }

    /// Constrói um buffer mock de 0x20 bytes (como `out` real teria) e faz ROUND-TRIP:
    /// escreve com `write_cstring_inline_ret`, lê de volta com `rtti::read_cstring` — prova que
    /// o layout escrito é EXATAMENTE o que a leitura (já provada em produção) espera, sem
    /// precisar do jogo rodando.
    #[test]
    fn roundtrip_write_read_cstring_inline() {
        let mut buf = [0u8; 0x20];
        let ptr = buf.as_mut_ptr() as *mut c_void;
        unsafe {
            assert!(write_cstring_inline_ret(ptr, "hello"));
            assert_eq!(crate::rtti::read_cstring(buf.as_ptr()), Some("hello".to_string()));
        }
    }

    #[test]
    fn roundtrip_string_vazia() {
        let mut buf = [0xFFu8; 0x20]; // lixo não-zero, prova que o writer LIMPA o buffer
        let ptr = buf.as_mut_ptr() as *mut c_void;
        unsafe {
            assert!(write_cstring_inline_ret(ptr, ""));
            assert_eq!(crate::rtti::read_cstring(buf.as_ptr()), Some(String::new()));
        }
    }

    #[test]
    fn roundtrip_multibyte() {
        let mut buf = [0u8; 0x20];
        let ptr = buf.as_mut_ptr() as *mut c_void;
        unsafe {
            assert!(write_cstring_inline_ret(ptr, "café!"));
            assert_eq!(crate::rtti::read_cstring(buf.as_ptr()), Some("café!".to_string()));
        }
    }

    #[test]
    fn recusa_string_longa_demais_pro_sso() {
        let mut buf = [0u8; 0x20];
        let ptr = buf.as_mut_ptr() as *mut c_void;
        let s19 = "a".repeat(19); // cabe (< 20)
        let s20 = "a".repeat(20); // não cabe (>= 20 -> represa de heap, fora de escopo)
        unsafe {
            assert!(write_cstring_inline_ret(ptr, &s19));
            assert!(!write_cstring_inline_ret(ptr, &s20));
        }
    }
}

#[cfg(test)]
mod number_utils_tests {
    use super::*;

    #[test]
    fn parseint_decimal_simples() {
        assert_eq!(strtoll_like("42", 10), Some(42));
        assert_eq!(strtoll_like("-42", 10), Some(-42));
        assert_eq!(strtoll_like("  7", 10), Some(7));
    }

    #[test]
    fn parseint_hex_com_prefixo() {
        assert_eq!(strtoll_like("0x2A", 16), Some(42));
        assert_eq!(strtoll_like("2A", 16), Some(42));
        assert_eq!(strtoll_like("0x2A", 0), Some(42)); // base 0 auto-detecta 0x
    }

    #[test]
    fn parseint_octal_base0() {
        assert_eq!(strtoll_like("010", 0), Some(8));
    }

    #[test]
    fn parseint_lixo_sobrando_falha() {
        assert_eq!(strtoll_like("42abc", 10), None);
        assert_eq!(strtoll_like("42 ", 10), None); // espaço à direita também conta como lixo
        assert_eq!(strtoll_like("", 10), None);
        assert_eq!(strtoll_like("abc", 10), None);
    }

    #[test]
    fn parseint_overflow_clampa() {
        assert_eq!(strtoll_like("99999999999999999999", 10), Some(i64::MAX));
        assert_eq!(strtoll_like("-99999999999999999999", 10), Some(i64::MIN));
    }

    #[test]
    fn parseuint_aceita_minus_e_nega_mod2_64() {
        // strtoull("-1", ...) == UINT64_MAX, comportamento real do glibc/libc++
        assert_eq!(strtoull_like("-1", 10), Some(u64::MAX));
    }

    #[test]
    fn parseuint_overflow_clampa() {
        assert_eq!(strtoull_like("99999999999999999999", 10), Some(u64::MAX));
    }

    #[test]
    fn trampoline_trunca_pela_largura_do_tipo_alvo() {
        // -1 em Int8 deve ter o MESMO bit-pattern que ParseInt8("-1") == 0xFF (truncado)
        let v = strtoll_like("-1", 10).unwrap();
        assert_eq!((v as u64) & 0xFF, 0xFF);
    }
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
    // Tentativa 11 (2026-07-13): disasm de 0x102198e18 (o lookup do 2º validador de classe, chamado
    // por função declarada) mostrou que ele compara [func+0x08] (fullName) contra o CName lido do
    // DESCRITOR do compilador — que é o nome BARE ("Version"/"Require"), sem prefixo de classe. Como
    // `build_native_func` grava `cname(full)` em +0x08, usar "Codeware::Version" ali produz um hash
    // DIFERENTE do que o compilador declarou — a busca nunca acha o método, mesmo com o forge+registro
    // "funcionando" (slot ocupado, re-resolve OK). Fix: `full`=`short` (nome bare nos dois campos) só
    // pras natives da Facade, onde o nome TEM que bater com o que o `.reds` declarou.
    let v = register_method(reg, "Codeware", proto, "Version", "Version", tramp_version, true);
    let r = register_method(reg, "Codeware", proto, "Require", "Require", tramp_require, true);
    format!("[reg] Codeware.Version={v} Codeware.Require={r} (precisa do .reds do Codeware carregado p/ a classe existir)")
}

#[cfg(test)]
mod bits_utils_tests {
    use super::{bit_set, bit_shl, bit_shr, bit_test, mask_arg};

    #[test]
    fn bit_test_le_e_fora_do_range() {
        assert!(bit_test(0b1010, 1));
        assert!(!bit_test(0b1010, 0));
        assert!(!bit_test(0xFF, -1));
        assert!(!bit_test(0xFF, 64));
    }

    #[test]
    fn bit_set_liga_e_desliga() {
        assert_eq!(bit_set(0b0000, 2, true), 0b0100);
        assert_eq!(bit_set(0b1111, 2, false), 0b1011);
        // fora do range: devolve o valor original, sem tocar
        assert_eq!(bit_set(0xAB, -1, true), 0xAB);
        assert_eq!(bit_set(0xAB, 64, true), 0xAB);
    }

    #[test]
    fn bit_shl_mascara_pela_largura() {
        // 8 bits: 0xFF << 4 estoura o byte, deve truncar em 8 bits (mask 0xFF)
        assert_eq!(bit_shl(0xFF, 4, 8), 0xF0);
        // 32 bits: 1 << 31 cabe exatamente
        assert_eq!(bit_shl(1, 31, 32), 1u64 << 31);
        // 64 bits: sem mask (largura total)
        assert_eq!(bit_shl(1, 63, 64), 1u64 << 63);
    }

    #[test]
    fn bit_shr_simples() {
        assert_eq!(bit_shr(0b1000, 3), 0b1);
        assert_eq!(bit_shr(0xFF, -1), 0xFF); // n negativo: devolve como veio
    }

    #[test]
    fn mask_arg_trunca_pela_largura_certa() {
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 1), 0xF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 2), 0xDEF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 4), 0x9ABC_DEF0);
        assert_eq!(mask_arg(0x1234_5678_9ABC_DEF0, 8), 0x1234_5678_9ABC_DEF0);
    }
}
