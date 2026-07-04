//! rtti.rs — universal-call do RED RTTI do CP2077, 100% Rust (sem JS).
//! Resolve uma função RED por nome (CName=FNV1a64) varrendo o registry e a
//! invoca pelo executor do jogo, montando o CScriptStackFrame na mão.
//!
//! Offsets/ABI = FATOS da RTTI do jogo (v2.3.1 build 5314028; os mesmos que os
//! headers públicos do RED4ext descrevem). Chamada = ponteiro transmutado em
//! `extern "C"` (AAPCS64) — NÃO precisa de gum (gum é só p/ HOOKAR).

use std::convert::TryInto;
use std::ffi::c_void;

use crate::cname::cname;
use crate::rebase;

const ADDR_CRTTI_GET: u64 = 0x1_0218_8e8c; // CRTTISystem::Get (acessor singleton)
const ADDR_EXEC: u64 = 0x1_0217_3120; // executor universal (func, ctx, frame, res, retType)
/// PoolStorageProxy<red::PoolDefault>::AllocateAligned(u64 size, u32 align) → Block{x0=ptr,x1=size}.
/// Estática, NÃO zera. Aloca do POOL DO RED → o Free do engine (PushData etc.) casa (sem corromper).
/// Confirmado por disasm (workflow RE 2026-06-21). Ver [[cp77-macos-rtti-vtable-offsets]].
const ADDR_POOL_DEFAULT_ALLOC_ALIGNED: u64 = 0x1_0002_2808;
/// PoolDefault::Free(red::memory::Block&) — Block = {ptr, size}. Libera o que AllocateAligned deu.
const ADDR_POOL_DEFAULT_FREE: u64 = 0x1_0002_2cb0;
/// Tabela de handlers de opcode da VM redscript (__DATA.__common, preenchida em
/// runtime). `OPCODE_TABLE[*code](ctx, frame, &out, 0)` lê UM valor do frame.
/// Achado desmontando funcOperatorAdd<int> (lê 2 params via essa tabela).
const OPCODE_TABLE: u64 = 0x1_0908_b798;

/// Lê os parâmetros de uma chamada a partir do CScriptStackFrame, SEM destruir a
/// chamada original (salva/restaura o estado do frame). Cada param vem como u64 cru
/// (8 bytes; o caller decide se é ponteiro/handle ou escalar). É a base do Observe
/// COM ARGS. Espelha exatamente o read de param das native funcs:
///   frame+0x00 = code ptr; frame+0x40 = ctx; frame+0x62 = contador; frame+0x30 scratch.
/// CName do TIPO do i-ésimo param (via `IRTTIType::GetName`, vtable+8 do IType).
/// 0 se não der (o caller cai na heurística). Const accessor → seguro de chamar.
pub unsafe fn param_type_cname(p_entries: *const u8, i: usize) -> u64 {
    if p_entries.is_null() {
        return 0;
    }
    let cprop = rd_ptr(p_entries.add(i * 8));
    if !sane(cprop) {
        return 0;
    }
    let ty = rd_ptr(cprop as *const u8); // CProperty+0 = IType
    if !sane(ty) || !crate::gum::is_readable(ty as *const c_void, 0x20) {
        return 0;
    }
    // GetName() (IType vtable+0x10 no macOS; Windows 0x08) → CName do tipo. Vale p/ TODOS
    // os tipos. O hack antigo (ler IType+0x18 cru) só pegava o nome em tipos CLASSE; pra
    // FUNDAMENTAIS (CName/Int32/Bool/Float) dava 0 → o spawnEvent:CName saía sem tipo e o
    // valor não era lido. GetName é getter const (seguro de chamar).
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() {
        return 0;
    }
    let get_name = rd_ptr(vt.add(0x10));
    if !sane(get_name) {
        return 0;
    }
    let f: extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(get_name);
    f(ty)
}

/// Aloca+constrói uma instância TIPADA transiente (espelha CET::HandleOverridenFunction):
/// PoolDefault::AllocateAligned(GetSize@0x18, GetAlignment@0x20) → memset 0 → Construct@0x40.
/// O handler do opcode escreve o valor do param NESTA memória tipada — pra tipos complexos
/// (String/array/handle) é obrigatório (o handler escreve o objeto inteiro; buf de pilha estoura).
unsafe fn type_inst_alloc(ty: *mut c_void) -> (*mut c_void, usize) {
    if ty.is_null() {
        return (std::ptr::null_mut(), 0);
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() {
        return (std::ptr::null_mut(), 0);
    }
    let get_size = rd_ptr(vt.add(0x18));
    let construct = rd_ptr(vt.add(0x40));
    if !sane(get_size) || !sane(construct) {
        return (std::ptr::null_mut(), 0);
    }
    let gs: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_size);
    let size = gs(ty) as usize;
    if size == 0 || size > 1_000_000 {
        return (std::ptr::null_mut(), 0);
    }
    let get_align = rd_ptr(vt.add(0x20));
    let align = if sane(get_align) {
        let ga: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_align);
        (ga(ty) as usize).max(8).next_power_of_two()
    } else {
        8
    };
    let alloc: extern "C" fn(u64, u32) -> *mut c_void =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_ALLOC_ALIGNED));
    let mem = alloc(size as u64, align as u32);
    if mem.is_null() {
        return (std::ptr::null_mut(), 0);
    }
    std::ptr::write_bytes(mem as *mut u8, 0, size);
    let ctor: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(construct);
    ctor(ty, mem);
    (mem, size)
}

/// Destrói (Destruct@0x48) + libera (PoolDefault::Free) a instância de type_inst_alloc.
unsafe fn type_inst_free(ty: *mut c_void, inst: *mut c_void, size: usize) {
    if inst.is_null() {
        return;
    }
    if !ty.is_null() {
        let vt = rd_ptr(ty as *const u8) as *const u8;
        if !vt.is_null() {
            let destruct = rd_ptr(vt.add(0x48));
            if sane(destruct) {
                let d: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(destruct);
                d(ty, inst);
            }
        }
    }
    let free: extern "C" fn(*mut c_void) =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_FREE));
    let block = [inst as u64, size as u64];
    free(block.as_ptr() as *mut c_void);
}

unsafe fn read_params_inner(func: *mut c_void, frame: *mut c_void, consume: bool) -> Vec<(u64, u64)> {
    if func.is_null() || frame.is_null() {
        return Vec::new();
    }
    // Sanidade: frame mapeado. Lê EXATAMENTE p_count (= apFunction->params.size) args, cada um
    // numa instância TIPADA — espelho byte-a-byte do CET::HandleOverridenFunction (FunctionOverride
    // .cpp:269-334). O bug antigo era passar buf[16] de pilha; pra tipo complexo o handler
    // escreve o objeto inteiro e estoura = crash do AddMenuItem nativo.
    if !crate::gum::is_readable(frame as *const c_void, 0x68) {
        return Vec::new();
    }
    let p_count = rd_u32((func as *const u8).add(0x30)) as usize;
    if p_count == 0 || p_count > 16 {
        return Vec::new();
    }
    let p_entries = rd_ptr((func as *const u8).add(0x28)) as *const u8;
    let f = frame as *mut u8;
    // salva code/currentParam/data/dataType — a ORIGINAL re-lê depois (CET restaura pCode).
    let save_code = (f as *const *const u8).read_unaligned();
    let save_cnt = *f.add(0x62);
    let save_30 = (f.add(0x30) as *const u64).read_unaligned();
    let save_38 = (f.add(0x38) as *const u64).read_unaligned();
    let table = rebase(OPCODE_TABLE) as *const usize;
    let mut out = Vec::with_capacity(p_count);
    for i in 0..p_count {
        // tipo do param (CProperty+0 = IType*).
        let ptype = {
            let cprop = rd_ptr(p_entries.add(i * 8));
            if sane(cprop) {
                rd_ptr(cprop as *const u8)
            } else {
                std::ptr::null_mut()
            }
        };
        let tc = param_type_cname(p_entries, i);
        // instância TIPADA pro out do handler (NÃO buf de pilha).
        let (pinst, psize) = type_inst_alloc(ptype);
        if pinst.is_null() {
            break;
        }
        // setup por param (igual CET): currentParam++, data/dataType = null.
        *f.add(0x62) = (*f.add(0x62)).wrapping_add(1);
        (f.add(0x30) as *mut u64).write_unaligned(0);
        (f.add(0x38) as *mut u64).write_unaligned(0);
        let ctx = (f.add(0x40) as *const *mut c_void).read_unaligned();
        let code = (f as *const *const u8).read_unaligned();
        if code.is_null() || !crate::gum::is_readable(code as *const c_void, 1) {
            type_inst_free(ptype, pinst, psize);
            break;
        }
        let opcode = *code as usize;
        (f as *mut *const u8).write_unaligned(code.add(1)); // *code++
        let handler = *table.add(opcode);
        if handler == 0
            || !sane(handler as *mut c_void)
            || !crate::gum::is_readable(handler as *const c_void, 4)
        {
            type_inst_free(ptype, pinst, psize);
            break;
        }
        let h: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) =
            std::mem::transmute(handler);
        // ERTTIType do param (GetType@vtable+0x28): decide scriptRefOut + como ler o valor.
        // 9=Handle, 10=WeakHandle, 5=Enum, 15=ScriptReference. (Itanium não desloca DADO, só
        // método de vtable; GetType é Windows 0x20 → macOS 0x28.)
        let ertti = {
            let vt = rd_ptr(ptype as *const u8) as *const u8;
            if !vt.is_null() {
                let gt = rd_ptr(vt.add(0x28));
                if sane(gt) {
                    let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(gt);
                    f(ptype)
                } else {
                    u32::MAX
                }
            } else {
                u32::MAX
            }
        };
        // script_ref<T>=15 precisa de pinst como scriptRefOut (espelha CET: isScriptRef ?
        // pInstance : nullptr) — senão o frame não avança e o param SEGUINTE sai 0.
        let scriptref_out = if ertti == 15 {
            pinst
        } else {
            std::ptr::null_mut()
        };
        h(ctx, frame, pinst, scriptref_out);
        // lê 8 bytes de pinst+0 quando o "valor" cabe em um qword: ESCALAR (CName/Int/Float/
        // Bool/...) OU Handle(9)/WeakHandle(10) — nestes pinst+0 = PONTEIRO do objeto, que o
        // decode_arg embrulha em Handle (ex.: target:ref<ListItemController> do OnMenuItemActivated,
        // que o NativeSettings usa p/ setar fromMods). String/array/ScriptRef/struct = 0
        // (o callback lê do objeto destino). O objeto do Handle vive além do free (dono = menu).
        let escalar = tc == cname("CName")
            || tc == cname("Int32")
            || tc == cname("Uint32")
            || tc == cname("Int64")
            || tc == cname("Uint64")
            || tc == cname("Float")
            || tc == cname("Bool")
            || tc == cname("TweakDBID")
            || tc == cname("EntityID");
        let read8 = escalar || ertti == 9 || ertti == 10 || ertti == 5;
        let raw = if read8 && crate::gum::is_readable(pinst as *const c_void, 8) {
            (pinst as *const u64).read_unaligned()
        } else {
            0
        };
        out.push((raw, tc));
        type_inst_free(ptype, pinst, psize);
    }
    // CONSUME=false (hooks): restaura o frame pra a original re-ler os params intactos.
    // CONSUME=true (handler de NATIVE nosso, sem original): NÃO restaura — deixa o frame
    // avançado (args consumidos) pra a VM redscript continuar correta após a chamada.
    if !consume {
        (f as *mut *const u8).write_unaligned(save_code);
        *f.add(0x62) = save_cnt;
        (f.add(0x30) as *mut u64).write_unaligned(save_30);
        (f.add(0x38) as *mut u64).write_unaligned(save_38);
    }
    out
}

/// Lê os params do frame SEM consumir (restaura o frame). P/ hooks Observe/Override (a original re-lê).
pub unsafe fn read_params(func: *mut c_void, frame: *mut c_void) -> Vec<(u64, u64)> {
    read_params_inner(func, frame, false)
}
/// Lê os params CONSUMINDO (frame avançado). P/ handler de NATIVE nosso (não há original p/ re-ler).
pub unsafe fn read_params_consuming(func: *mut c_void, frame: *mut c_void) -> Vec<(u64, u64)> {
    read_params_inner(func, frame, true)
}

#[inline]
unsafe fn rd_ptr(p: *const u8) -> *mut c_void {
    (p as *const *mut c_void).read_unaligned()
}
#[inline]
unsafe fn rd_u32(p: *const u8) -> u32 {
    (p as *const u32).read_unaligned()
}
#[inline]
unsafe fn rd_u64(p: *const u8) -> u64 {
    (p as *const u64).read_unaligned()
}

/// Leituras SEGURAS (via gum is_readable) — None se o endereço não estiver mapeado.
/// Base dos diagnósticos/resolvedores que leem structs RTTI a partir de offsets ainda
/// não validados (evita segfault ao tocar ponteiro "são" porém não-mapeado).
#[inline]
unsafe fn rd_ptr_chk(p: *const u8) -> Option<*mut c_void> {
    crate::gum::is_readable(p as *const c_void, 8).then(|| rd_ptr(p))
}
#[inline]
unsafe fn rd_u32_chk(p: *const u8) -> Option<u32> {
    crate::gum::is_readable(p as *const c_void, 4).then(|| rd_u32(p))
}

pub struct Registry {
    reg: *mut c_void,
    vtbl: *const u8,
    get_class: extern "C" fn(*mut c_void, u64) -> *mut c_void,
    get_enum: extern "C" fn(*mut c_void, u64) -> *mut c_void,
}

impl Registry {
    /// Chama CRTTISystem::Get e captura GetClass (vtbl+0x10) e GetEnum (vtbl+0x18).
    pub unsafe fn obtain() -> Option<Registry> {
        let get: extern "C" fn() -> *mut c_void = std::mem::transmute(rebase(ADDR_CRTTI_GET));
        let reg = get();
        if reg.is_null() {
            return None;
        }
        let vtbl = rd_ptr(reg as *const u8) as *const u8;
        if vtbl.is_null() {
            return None;
        }
        let gc = rd_ptr(vtbl.add(0x10));
        let ge = rd_ptr(vtbl.add(0x18));
        if gc.is_null() || ge.is_null() {
            return None;
        }
        Some(Registry {
            reg,
            vtbl,
            get_class: std::mem::transmute(gc),
            get_enum: std::mem::transmute(ge),
        })
    }

    pub unsafe fn class_by_name(&self, name: &str) -> *mut c_void {
        (self.get_class)(self.reg, cname(name))
    }

    pub unsafe fn enum_by_name(&self, name: &str) -> *mut c_void {
        (self.get_enum)(self.reg, cname(name))
    }

    /// O CRTTISystem* cru (o `this` das chamadas de vtable).
    pub unsafe fn raw(&self) -> *mut c_void {
        self.reg
    }

    /// Ponteiro do slot `off` da vtable do CRTTISystem (ex.: +0x30 GetFunction,
    /// +0x80 RegisterType, +0xA0 RegisterFunction). Usado pelo registro nativo.
    pub unsafe fn vtbl_slot(&self, off: usize) -> *mut c_void {
        if self.vtbl.is_null() || !crate::gum::is_readable(self.vtbl as *const c_void, off + 8) {
            return std::ptr::null_mut();
        }
        rd_ptr(self.vtbl.add(off))
    }
}

/// Aloca `size` bytes alinhados no POOL DO RED (PoolDefault) — o MESMO pool de
/// `new_object`, então o `Free` da engine casa. NÃO zera (faça você). Reutilizável
/// pelo registro nativo (construir CGlobalFunction/CClassFunction à mão).
pub unsafe fn pool_alloc(size: usize, align: usize) -> *mut c_void {
    let alloc: extern "C" fn(u64, u32) -> *mut c_void =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_ALLOC_ALIGNED));
    alloc(size as u64, align.max(8) as u32)
}

/// Valor de um membro de enum por NOME (ex.: gamedataDevelopmentPointType::Attribute).
/// CEnum: hashList@+0x28 (fnv dos nomes), count@+0x30, valueList@+0x38 (u64 cada).
pub unsafe fn resolve_enum_value(reg: &Registry, enum_type: &str, member: &str) -> Option<u64> {
    let en = reg.enum_by_name(enum_type) as *const u8;
    if en.is_null() {
        return None;
    }
    let hp = rd_ptr(en.add(0x28)) as *const u8;
    let n = rd_u32(en.add(0x30));
    let vp = rd_ptr(en.add(0x38)) as *const u8;
    if hp.is_null() || vp.is_null() || n > 100_000 {
        return None;
    }
    let mh = cname(member);
    for i in 0..n as usize {
        if rd_u64(hp.add(i * 8)) == mh {
            return Some(rd_u64(vp.add(i * 8)));
        }
    }
    None
}

pub struct ResolvedFn {
    pub func: *mut c_void,
    pub ret_type: *mut c_void,
    pub is_static: bool,
}

/// Nº de parâmetros declarados da função (CBaseFunction+0x30). Pra debug/validação.
pub unsafe fn param_count(rf: &ResolvedFn) -> u32 {
    rd_u32((rf.func as *const u8).add(0x30))
}

/// True se a função retorna `void` — `*(func+0x18)` (descritor de retorno) é null. Usado p/
/// SUPRIMIR override com segurança: a sonda pula a original com `x0=1` sem escrever o slot de
/// retorno, então só é seguro suprimir funções VOID (o caller não dereferencia o retorno). Pra
/// função value-returning, suprimir = caller pega `1` falso e crasha (EXC_BAD_ACCESS at 0xa9).
/// Default FALSE (não-void = não suprime) se algo estiver ilegível → lado seguro.
pub unsafe fn fn_returns_void(func: *mut c_void) -> bool {
    if func.is_null() || !crate::gum::is_readable(func as *const c_void, 0x20) {
        return false;
    }
    rd_ptr((func as *const u8).add(0x18)).is_null()
}

/// Descritor de TIPO do retorno da função: `*(func+0x18)` = `IType*` (null = void).
/// Mesmo campo que `fn_returns_void` lê; aqui devolvido pra inspecionar nome+tamanho
/// (usado pelo Override-suppress p/ marshalar o retorno no aOut com largura correta).
/// gum-checked — nunca congela numa func torta.
pub unsafe fn fn_ret_type(func: *mut c_void) -> *mut c_void {
    if func.is_null() || !crate::gum::is_readable(func as *const c_void, 0x20) {
        return std::ptr::null_mut();
    }
    rd_ptr((func as *const u8).add(0x18))
}

/// Escreve `val` no buffer de retorno `res` com a LARGURA do tipo de retorno da função
/// (mesmo gate de tipo do `write_pod_ret` do lua, mas Rust-nativo: entrada i64). Usado pelo
/// override RUST-nativo (valida o suppress SEM lua → sem crash de stack no aninhamento).
/// `false` = tipo não-POD/incompatível → não suprime.
pub unsafe fn write_pod_i64(func: *mut c_void, res: *mut c_void, val: i64) -> bool {
    if res.is_null() {
        return false;
    }
    let ty = fn_ret_type(func);
    if ty.is_null() {
        return false;
    }
    use crate::cname::cname;
    let tn = type_name_getname(ty);
    let sz = type_size(ty);
    let p = res as *mut u8;
    if tn == cname("Bool") && sz == 1 {
        *p = u8::from(val != 0);
        return true;
    } else if (tn == cname("Int8") || tn == cname("Uint8")) && sz == 1 {
        *p = val as u8;
        return true;
    } else if (tn == cname("Int16") || tn == cname("Uint16")) && sz == 2 {
        (p as *mut i16).write_unaligned(val as i16);
        return true;
    } else if (tn == cname("Int32") || tn == cname("Uint32")) && sz == 4 {
        (p as *mut i32).write_unaligned(val as i32);
        return true;
    } else if (tn == cname("Int64") || tn == cname("Uint64")) && sz == 8 {
        (p as *mut i64).write_unaligned(val);
        return true;
    } else if tn == cname("Float") && sz == 4 {
        (p as *mut f32).write_unaligned(val as f32);
        return true;
    } else if tn == cname("Double") && sz == 8 {
        (p as *mut f64).write_unaligned(val as f64);
        return true;
    }
    false
}

/// CName do TIPO do i-ésimo parâmetro de uma função (via GetName, vale p/ fundamentais). Pra
/// marshaling DIRIGIDA POR TIPO: uma string Lua vira `String` (red::CString) se o param é String,
/// ou `CName` se o param é CName — senão `SetText("x")` mandava CName e a label não renderizava.
pub unsafe fn fn_param_type(func: *mut c_void, i: usize) -> u64 {
    if func.is_null() {
        return 0;
    }
    let p_entries = rd_ptr((func as *const u8).add(0x28)) as *const u8;
    param_type_cname(p_entries, i)
}

/// Resolve um método tentando vários nomes de classe (fallback, como o `resolveAny`
/// dele — os nomes de classe variam: ScriptGameInstance/GameInstance/gameScript…).
pub unsafe fn resolve_any(reg: &Registry, classes: &[&str], method: &str) -> Option<ResolvedFn> {
    for c in classes {
        if let Some(r) = resolve_func(reg, c, method) {
            return Some(r);
        }
    }
    None
}

/// Chama `rf` e lê os 8 primeiros bytes do retorno como ponteiro (getters de
/// sistema/player retornam ponteiro/handle).
pub unsafe fn call_ptr(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg]) -> *mut c_void {
    match call_func(rf, ctx, args) {
        Some(res) => u64::from_le_bytes(res[0..8].try_into().unwrap()) as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

/// Ponteiro plausível (mesma checagem `sane()` dele): fora de null/baixo e abaixo
/// do teto de espaço de usuário.
pub fn sane(p: *mut c_void) -> bool {
    let a = p as usize;
    a > 0x1_0000 && a < 0x8000_0000_0000
}

/// Resolve uma função RED por classe+método, subindo a cadeia de heranças.
pub unsafe fn resolve_func(reg: &Registry, class_name: &str, method: &str) -> Option<ResolvedFn> {
    resolve_in_class(reg.class_by_name(class_name), method)
}

/// Resolve um método a partir do PONTEIRO da classe (CClass*), subindo a cadeia de
/// pais. Varre instância (CClass+0x48) e estáticas (CClass+0x58), casando o CName
/// do método em `func+0x10`. Base do proxy genérico `handle:Method()`.
pub unsafe fn resolve_in_class(cls0: *mut c_void, method: &str) -> Option<ResolvedFn> {
    let mh = cname(method);
    let mut cls = cls0;
    // GUARDA: cadeia de parents (cls+0x10) sem limite loopa ETERNO (= CONGELA o jogo) se o
    // cls for handle stale/torto. Limita a profundidade + exige cls mapeado (gum) por nível.
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        if !crate::gum::is_readable(cls as *const c_void, 0x60) {
            break;
        }
        let clsb = cls as *const u8;
        for off in [0x48usize, 0x58usize] {
            let fp = rd_ptr(clsb.add(off)) as *const u8;
            let n = rd_u32(clsb.add(off + 8));
            if !fp.is_null() && n < 20_000 && crate::gum::is_readable(fp as *const c_void, 8) {
                for i in 0..n as usize {
                    let slot = fp.add(i * 8);
                    if !crate::gum::is_readable(slot as *const c_void, 8) {
                        break;
                    }
                    let f = rd_ptr(slot) as *const u8;
                    if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
                        continue;
                    }
                    if rd_u64(f.add(0x10)) == mh {
                        let rp = rd_ptr(f.add(0x18));
                        let ret_type = if rp.is_null() {
                            std::ptr::null_mut()
                        } else {
                            rd_ptr(rp as *const u8)
                        };
                        return Some(ResolvedFn {
                            func: f as *mut c_void,
                            ret_type,
                            is_static: off == 0x58,
                        });
                    }
                }
            }
        }
        cls = rd_ptr(clsb.add(0x10)); // parent
    }
    None
}

/// Classe (CClass*) de um objeto RED via `vtable+8` = `GetType(this)`. É como a
/// sonda identifica o player/tx; aqui serve pro proxy resolver métodos no objeto.
pub unsafe fn class_of(obj: *mut c_void) -> *mut c_void {
    if obj.is_null() || !crate::gum::is_readable(obj as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = rd_ptr(obj as *const u8);
    // NÃO exigir a vtable no módulo: classes SCRIPTED (ex.: PauseMenuListItemData, controllers
    // de lista ink) têm vtable GERADA EM RUNTIME no HEAP, não no binário. Só exige vt legível.
    if !crate::gum::is_readable(vt as *const c_void, 0x10) {
        return std::ptr::null_mut();
    }
    let get_type = rd_ptr((vt as *const u8).add(8));
    // O GetType (vtable+8) é SEMPRE código NATIVO no módulo (até p/ scripted). Isso é o que
    // distingue objeto VÁLIDO de handle STALE: stale → vt lixo → get_type lixo → fora do
    // módulo → rejeita (evita o FREEZE de chamar lixo). Válido → get_type no módulo → chama.
    let get_type_static = crate::un_rebase(get_type);
    if !(0x1_0000_0000..0x1_0A00_0000).contains(&get_type_static) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(get_type);
    let cls = f(obj);
    if sane(cls) {
        cls
    } else {
        std::ptr::null_mut()
    }
}

// ===== Reflection: propriedades por nome (GetValue/SetValue) =========================
// CProperty (RED4ext.SDK, layout CONFIRMADO no macOS via reflection-test pois CProperty NÃO tem
// vtable → sem o shift +0x08 do Itanium): type@0x00, name(CName)@0x08, group@0x10, parent@0x18,
// valueOffset(u32)@0x20, flags@0x28; sizeof 0x30. O valor vive em `obj + valueOffset`
// (salvo flags.inValueHolder@bit0x15 — raro; V1 ignora e lê inline).

/// Acha a `DynArray<CProperty*>` da classe. NÃO é offset fixo garantido no macOS → tenta
/// cls+0x28 (documentado) e fallback noutros, EXCLUINDO 0x48/0x58 (= funcs/staticFuncs, que
/// também têm name@0x08 e confundiriam). Valida: 1a entrada é CProperty* com name!=0 e
/// valueOffset<0x10000. Devolve (entries, count).
unsafe fn props_array(cls: *mut c_void) -> Option<(*const u8, u32)> {
    if !crate::gum::is_readable(cls as *const c_void, 0x80) {
        return None;
    }
    let clsb = cls as *const u8;
    for &off in &[0x28usize, 0x30, 0x38, 0x40, 0x20, 0x18, 0x60, 0x68, 0x70] {
        let arr = match rd_ptr_chk(clsb.add(off)) {
            Some(p) => p as *const u8,
            None => continue,
        };
        let cnt = match rd_u32_chk(clsb.add(off + 8)) {
            Some(c) => c,
            None => continue,
        };
        if arr.is_null() || cnt == 0 || cnt > 20_000 {
            continue;
        }
        let p0 = match rd_ptr_chk(arr) {
            Some(p) => p as *const u8,
            None => continue,
        };
        if p0.is_null() || !crate::gum::is_readable(p0 as *const c_void, 0x30) {
            continue;
        }
        if rd_u64(p0.add(0x08)) != 0 && rd_u32(p0.add(0x20)) < 0x10000 {
            return Some((arr, cnt));
        }
    }
    None
}

/// Acha um CProperty por nome numa CClass (+ cadeia de parents). Devolve o CProperty* ou null.
pub unsafe fn find_property(reg: &Registry, class: &str, prop: &str) -> *mut c_void {
    find_property_in_class(reg.class_by_name(class), prop)
}

/// Idem, dado o CClass* direto (ex.: `class_of(obj)` p/ reflection num objeto vivo).
pub unsafe fn find_property_in_class(cls0: *mut c_void, prop: &str) -> *mut c_void {
    let p = find_prop_exact(cls0, prop);
    if !p.is_null() {
        return p;
    }
    // A fonte decompilada usa `m_xxx`, mas o nome no RTTI DROPA o prefixo `m_`
    // (ex.: m_inCrouch -> inCrouch). Aceita os DOIS nomes: tenta sem o prefixo.
    // (Provado in-game 2026-06-26: 211 props no PlayerPuppet, todas sem `m_`.)
    if let Some(stripped) = prop.strip_prefix("m_") {
        return find_prop_exact(cls0, stripped);
    }
    std::ptr::null_mut()
}

unsafe fn find_prop_exact(cls0: *mut c_void, prop: &str) -> *mut c_void {
    let ph = cname(prop);
    let mut cls = cls0;
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        if let Some((arr, cnt)) = props_array(cls) {
            for i in 0..cnt as usize {
                let pp = match rd_ptr_chk(arr.add(i * 8)) {
                    Some(p) => p as *const u8,
                    None => break,
                };
                if pp.is_null() || !crate::gum::is_readable(pp as *const c_void, 0x30) {
                    continue;
                }
                if rd_u64(pp.add(0x08)) == ph {
                    return pp as *mut c_void;
                }
            }
        }
        if !crate::gum::is_readable(cls as *const c_void, 0x18) {
            break;
        }
        cls = rd_ptr((cls as *const u8).add(0x10)); // parent
    }
    std::ptr::null_mut()
}

/// valueOffset (u32@0x20) de um CProperty.
pub unsafe fn prop_value_offset(prop: *const c_void) -> u32 {
    rd_u32((prop as *const u8).add(0x20))
}
/// Ponteiro pro valor da propriedade no objeto (obj + valueOffset). V1 ignora inValueHolder.
pub unsafe fn prop_value_ptr(prop: *const c_void, obj: *mut c_void) -> *mut c_void {
    (obj as *mut u8).add(prop_value_offset(prop) as usize) as *mut c_void
}
pub unsafe fn prop_get_u32(prop: *const c_void, obj: *mut c_void) -> u32 {
    rd_u32(prop_value_ptr(prop, obj) as *const u8)
}
pub unsafe fn prop_set_u32(prop: *const c_void, obj: *mut c_void, v: u32) {
    core::ptr::write_unaligned(prop_value_ptr(prop, obj) as *mut u32, v);
}
pub unsafe fn prop_get_f32(prop: *const c_void, obj: *mut c_void) -> f32 {
    f32::from_bits(prop_get_u32(prop, obj))
}
pub unsafe fn prop_set_f32(prop: *const c_void, obj: *mut c_void, v: f32) {
    prop_set_u32(prop, obj, v.to_bits());
}
pub unsafe fn prop_get_bool(prop: *const c_void, obj: *mut c_void) -> bool {
    core::ptr::read_unaligned(prop_value_ptr(prop, obj) as *const u8) != 0
}
pub unsafe fn prop_set_bool(prop: *const c_void, obj: *mut c_void, v: bool) {
    core::ptr::write_unaligned(prop_value_ptr(prop, obj) as *mut u8, v as u8);
}

/// Probe de Reflection (DEV): dado o CClass* `cls0` (ex.: `class_of(player)`), anda a cadeia de
/// parents achando a 1a `DynArray<CProperty*>`, dumpa props (nome via resolve_cname + valueOffset)
/// p/ CONFIRMAR o layout do CProperty no macOS, faz GET read-only no `obj` vivo (se houver) e um
/// round-trip set/get num OBJETO FAKE nosso (buffer pool, ZERO efeito no jogo). Se não achar props,
/// despeja o scan CRU de cls+0x10..0x88 (ptr/count + 1a entrada) p/ eu ver onde elas estão.
pub unsafe fn reflection_probe_cls(cls0: *mut c_void, label: &str, obj: *mut c_void) -> String {
    if !sane(cls0) {
        return format!("[refl] '{label}': cls inválido");
    }
    let mut report = format!("[refl] '{label}': cls={cls0:p} obj={obj:p}\n");
    let mut first_prop: *const c_void = std::ptr::null();
    let mut cls = cls0;
    let mut guard = 0;
    while !cls.is_null() && guard < 8 {
        guard += 1;
        if let Some((arr, cnt)) = props_array(cls) {
            report.push_str(&format!("  props @ cls={cls:p} count={cnt}\n"));
            for i in 0..(cnt as usize).min(12) {
                let pp = match rd_ptr_chk(arr.add(i * 8)) {
                    Some(p) => p as *const u8,
                    None => break,
                };
                if pp.is_null() || !crate::gum::is_readable(pp as *const c_void, 0x30) {
                    continue;
                }
                let nm = rd_u64(pp.add(0x08));
                let vo = rd_u32(pp.add(0x20));
                let ty = rd_u64(pp.add(0x00));
                if first_prop.is_null() {
                    first_prop = pp as *const c_void;
                }
                report.push_str(&format!(
                    "    [{i:02}] '{}' vo={vo:#x} ty={ty:#x}\n",
                    crate::cname::resolve_cname(nm)
                ));
            }
            break;
        }
        if !crate::gum::is_readable(cls as *const c_void, 0x18) {
            break;
        }
        cls = rd_ptr((cls as *const u8).add(0x10)); // parent
    }
    if first_prop.is_null() {
        report.push_str("  props NÃO localizada — scan cru cls+0x10..0x88 (ptr/count/1a-entrada):\n");
        let clsb = cls0 as *const u8;
        for off in (0x10usize..=0x88).step_by(8) {
            let (p, c) = match (rd_ptr_chk(clsb.add(off)), rd_u32_chk(clsb.add(off + 8))) {
                (Some(p), Some(c)) => (p, c),
                _ => continue,
            };
            if p.is_null() || c == 0 || c > 20_000 || !crate::gum::is_readable(p, 8) {
                continue;
            }
            let e0 = rd_ptr(p as *const u8);
            let (nm, vo) = if crate::gum::is_readable(e0, 0x30) {
                (rd_u64((e0 as *const u8).add(0x08)), rd_u32((e0 as *const u8).add(0x20)))
            } else {
                (0, 0)
            };
            report.push_str(&format!(
                "    +{off:#04x}: ptr={p:p} count={c} e0.name='{}' e0.vo={vo:#x}\n",
                crate::cname::resolve_cname(nm)
            ));
        }
        return report;
    }
    let vo = prop_value_offset(first_prop);
    if !obj.is_null() && crate::gum::is_readable(prop_value_ptr(first_prop, obj) as *const c_void, 4) {
        let v = prop_get_u32(first_prop, obj);
        report.push_str(&format!("  GET(obj vivo vo={vo:#x}) = {v:#x} (read-only — prova get em objeto real)\n"));
    }
    let buf = pool_alloc(vo as usize + 0x10, 8);
    if !buf.is_null() {
        std::ptr::write_bytes(buf as *mut u8, 0, vo as usize + 0x10);
        prop_set_u32(first_prop, buf, 0xDEAD_BEEF);
        let g = prop_get_u32(first_prop, buf);
        prop_set_f32(first_prop, buf, 1.5);
        let gf = prop_get_f32(first_prop, buf);
        report.push_str(&format!(
            "  round-trip(fake vo={vo:#x}): u32 0xDEADBEEF->{g:#x} OK={} | f32 1.5->{gf} OK={}\n",
            g == 0xDEAD_BEEF,
            gf == 1.5
        ));
    }
    report
}

/// NewObject(className): REPLICA o `CClass::CreateInstance` (alloc(GetSize, GetAlignment)
/// + Construct). **Offsets CONFIRMADOS NO macOS via rttidump da vtable do Vector4
/// (2026-06-20): GetSize@0x18 (→16 p/ Vector4), GetAlignment@0x20, Construct@0x40.**
/// São +0x08 vs o RED4ext.SDK (Windows/MSVC) porque o Itanium ABI do macOS tem DOIS
/// slots de destructor no topo da vtable (D1 completo + D0 deleting), deslocando tudo.
/// (A RE antiga usava os offsets Windows → Construct@0x38 caía num getter → CRASH.)
/// ⚠️ A alocação usa o alloc do Rust (não o pool do RED) → OK p/ objetos TRANSIENTES
/// lidos na hora e VAZADOS (newobj de teste); p/ objetos que o RED toma posse e LIBERA
/// (ex.: PushData do NativeSettings), o free divergente corrompe — por isso o lua.rs
/// ainda GATEIA o NewObject Lua (o pool RED é o próximo passo).
pub unsafe fn new_object(reg: &Registry, class_name: &str) -> *mut c_void {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    let vtbl = rd_ptr(cls as *const u8) as *const u8;
    if vtbl.is_null() {
        return std::ptr::null_mut();
    }
    let get_size = rd_ptr(vtbl.add(0x18));
    let get_align = rd_ptr(vtbl.add(0x20));
    let construct = rd_ptr(vtbl.add(0x40));
    if !sane(get_size) || !sane(construct) {
        return std::ptr::null_mut();
    }
    let gs: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_size);
    let size = gs(cls) as usize;
    if size == 0 || size > 1_000_000 {
        return std::ptr::null_mut();
    }
    let align = if sane(get_align) {
        let ga: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_align);
        (ga(cls) as usize).max(8).next_power_of_two()
    } else {
        8
    };
    crate::trace(&format!("new_object: {class_name} size={size} align={align} -> alloc"));
    // Aloca do POOL DO RED (não std::alloc) → o Free do engine após PushData CASA, sem
    // corromper. AllocateAligned(size, align) → x0=ptr (lê só x0); NÃO zera → zera aqui.
    let alloc: extern "C" fn(u64, u32) -> *mut c_void =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_ALLOC_ALIGNED));
    let mem = alloc(size as u64, align as u32);
    if mem.is_null() {
        return std::ptr::null_mut();
    }
    std::ptr::write_bytes(mem as *mut u8, 0, size);
    crate::trace(&format!("new_object: {class_name} -> Construct@0x40"));
    let ctor: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(construct);
    ctor(cls, mem);
    // IScriptable.nativeType @ obj+0x30 = CClass*. É o ÚNICO passo que CClass::CreateInstance
    // (inlined, sem símbolo) faz a mais que o Construct puro: o ctor do IScriptable zera
    // nativeType=null → GetType() cai no fallback GetNativeType() e class_of devolve a BASE
    // IScriptable em vez da derivada. Escrever cls aqui faz GetType devolver a classe certa
    // → resolve_prop acha os campos. SÓ p/ quem deriva de IScriptable (struct puro não tem
    // esse campo; escrever 0x30 corromperia). Offset de DADO (0x30) NÃO sofre o shift Itanium.
    if derives_from_iscriptable(cls) {
        core::ptr::write_unaligned((mem as *mut u8).add(0x30) as *mut *mut c_void, cls);
        crate::trace(&format!(
            "new_object: {class_name} nativeType set -> class_of={:p} vs cls={cls:p}",
            class_of(mem)
        ));
        // valueHolder @ obj+0x38: se o Construct não criou um (lazy) e a classe tem campos
        // inValueHolder, aloca+zera o blob (do MESMO pool RED → Free casa). Sem isso, escrever
        // `data.label="Mods"` cai em holder nulo e é PULADO → item entra na lista mas em branco.
        let holder_slot = (mem as *mut u8).add(0x38) as *mut *mut c_void;
        if (*holder_slot).is_null() {
            let hsz = value_holder_size(cls);
            if hsz > 0 {
                let n = hsz.max(8);
                let holder = alloc(n as u64, 8);
                if !holder.is_null() {
                    std::ptr::write_bytes(holder as *mut u8, 0, n);
                    *holder_slot = holder;
                }
                crate::trace(&format!(
                    "new_object: {class_name} valueHolder sz={hsz} -> {holder:p}"
                ));
            }
        }
    }
    crate::trace(&format!("new_object: {class_name} -> OK {mem:p}"));
    mem
}

/// True se `cls` (CClass*) deriva de IScriptable (tem o campo nativeType@0x30). Sobe a
/// cadeia de parents (cls+0x10) comparando o nome do tipo. Evita corromper STRUCT puro.
unsafe fn derives_from_iscriptable(cls: *mut c_void) -> bool {
    let want = cname("IScriptable");
    let mut c = cls;
    let mut guard = 0;
    while !c.is_null() && guard < 64 {
        guard += 1;
        if !crate::gum::is_readable(c as *const c_void, 0x20) {
            break;
        }
        if type_name_hash(c) == want {
            return true;
        }
        c = rd_ptr((c as *const u8).add(0x10)); // parent
    }
    false
}

/// Resolve um CAMPO por nome subindo a cadeia de parents → (valueOffset, IRTTIType*).
/// Array de props @ CClass+0x28 (entries), count u32 @ CClass+0x34 (NÃO 0x30=capacity);
/// CProperty: type@0x00, name(CName)@0x08, valueOffset(u32)@0x20 (confirmado por propdump
/// runtime: eventName=0x20/action=0x28). Toda leitura é gum-checked (não crasha).
pub unsafe fn resolve_prop_in_class(
    cls0: *mut c_void,
    field: &str,
) -> Option<(u32, *mut c_void, bool)> {
    let want = cname(field);
    let mut cls = cls0;
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        let clsb = cls as *const u8;
        if let (Some(arr), Some(n)) = (rd_ptr_chk(clsb.add(0x28)), rd_u32_chk(clsb.add(0x34))) {
            let arr = arr as *const u8;
            if !arr.is_null() && n < 100_000 {
                for i in 0..n as usize {
                    let p = match rd_ptr_chk(arr.add(i * 8)) {
                        Some(x) => x as *const u8,
                        None => break,
                    };
                    if p.is_null() || !crate::gum::is_readable(p as *const c_void, 0x30) {
                        continue;
                    }
                    if rd_u64(p.add(0x08)) == want {
                        let voff = rd_u32(p.add(0x20));
                        let ty = rd_ptr(p.add(0x00));
                        // CProperty.flags @ +0x28; bit 0x15 (máscara 0x200000) = inValueHolder:
                        // o valor mora DENTRO do IScriptable.valueHolder (obj+0x38), não direto
                        // no obj. Props de classes SCRIPTED (controllers ink) quase sempre têm
                        // isso → era o "menuListController dangling" (líamos o ptr do holder).
                        let flags = rd_u64(p.add(0x28));
                        let in_holder = (flags & 0x0020_0000) != 0;
                        return Some((voff, ty, in_holder));
                    }
                }
            }
        }
        cls = match rd_ptr_chk(clsb.add(0x10)) {
            Some(x) => x,
            None => break,
        };
    }
    None
}

/// Ponteiro FINAL de um campo respeitando inValueHolder (espelha CProperty::GetValuePtr).
/// Base = obj, ou IScriptable.valueHolder (obj+0x38) se `in_holder`. Null se o holder ainda
/// não foi inicializado (lazy) — o caller trata como Nil (seguro, sem ler o lugar errado).
pub unsafe fn field_ptr(obj: *mut c_void, voff: u32, in_holder: bool) -> *mut c_void {
    let mut holder = obj as *mut u8;
    if in_holder {
        let vh = rd_ptr((obj as *const u8).add(0x38)); // IScriptable.valueHolder @ +0x38
        if vh.is_null() {
            return std::ptr::null_mut();
        }
        holder = vh as *mut u8;
    }
    if !crate::gum::is_readable(holder as *const c_void, voff as usize + 8) {
        return std::ptr::null_mut();
    }
    holder.add(voff as usize) as *mut c_void
}

/// Tamanho do `valueHolder` (obj+0x38) de uma classe SCRIPTED: maior (valueOffset + type_size)
/// entre TODAS as props com bit inValueHolder (0x200000), subindo a cadeia de parents. É o blob
/// onde moram os valores dos campos scripted. O `CClass::CreateInstance` do jogo aloca isso; o
/// `Construct` puro NÃO → campos scripted ficam sem onde escrever (era o botão "Mods" em branco).
unsafe fn value_holder_size(cls0: *mut c_void) -> usize {
    let mut max_end = 0usize;
    let mut cls = cls0;
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        let clsb = cls as *const u8;
        if let (Some(arr), Some(n)) = (rd_ptr_chk(clsb.add(0x28)), rd_u32_chk(clsb.add(0x34))) {
            let arr = arr as *const u8;
            if !arr.is_null() && n < 100_000 {
                for i in 0..n as usize {
                    let p = match rd_ptr_chk(arr.add(i * 8)) {
                        Some(x) => x as *const u8,
                        None => break,
                    };
                    if p.is_null() || !crate::gum::is_readable(p as *const c_void, 0x30) {
                        continue;
                    }
                    if (rd_u64(p.add(0x28)) & 0x0020_0000) != 0 {
                        let voff = rd_u32(p.add(0x20)) as usize;
                        let ty = rd_ptr(p.add(0x00));
                        let sz = (type_size(ty) as usize).max(8); // 0 torto → assume ptr
                        max_end = max_end.max(voff + sz);
                    }
                }
            }
        }
        cls = match rd_ptr_chk(clsb.add(0x10)) {
            Some(x) => x,
            None => break,
        };
    }
    max_end
}

/// CClass* INTERNO de um tipo Handle/WeakHandle (CRTTIHandleType/WeakHandleType.innerType@0x10).
/// Só chamar quando o tipo é handle:/whandle: (senão o read de +0x10 é lixo).
pub unsafe fn inner_type(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() || !crate::gum::is_readable(ty as *const c_void, 0x18) {
        return std::ptr::null_mut();
    }
    rd_ptr((ty as *const u8).add(0x10))
}

/// CName do NOME do tipo lido CRU de IType+0x18. SÓ vale p/ tipos CLASSE/handle (onde +0x18 é
/// o CName do nome cacheado). Pra tipos FUNDAMENTAIS (String/CName/Int32/Bool/Float) +0x18 NÃO
/// é o nome (dá 0/lixo) → use `type_name_getname`. Mantido p/ walks de hierarquia de classe
/// (derives_from_iscriptable etc.), onde sempre é classe.
pub unsafe fn type_name_hash(ty: *mut c_void) -> u64 {
    if ty.is_null() || !crate::gum::is_readable(ty as *const c_void, 0x20) {
        return 0;
    }
    rd_u64((ty as *const u8).add(0x18))
}

/// CName do nome do tipo via `IRTTIType::GetName()` (vtable+0x10 no macOS). Vale p/ TODOS os
/// tipos — inclusive FUNDAMENTAIS (String/CName/Int32). Idêntico a `type_name_hash` em tipos
/// classe/handle (+0x18 == GetName lá), mas correto onde +0x18 falha. Use SEMPRE que o tipo
/// puder ser fundamental (tipo de CAMPO/valor de marshalling). Getter const → seguro de chamar.
pub unsafe fn type_name_getname(ty: *mut c_void) -> u64 {
    if ty.is_null() || !crate::gum::is_readable(ty as *const c_void, 0x18) {
        return 0;
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() || !crate::gum::is_readable(vt as *const c_void, 0x18) {
        return 0;
    }
    let get_name = rd_ptr(vt.add(0x10));
    if !sane(get_name) {
        return 0;
    }
    let f: extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(get_name);
    f(ty)
}

/// Tamanho do tipo via GetSize@0x18 da vtable do IType (macOS). 0 se torto.
pub unsafe fn type_size(ty: *mut c_void) -> u32 {
    if ty.is_null() {
        return 0;
    }
    let vt = match rd_ptr_chk(ty as *const u8) {
        Some(p) => p as *const u8,
        None => return 0,
    };
    let gs = match rd_ptr_chk(vt.add(0x18)) {
        Some(p) => p,
        None => return 0,
    };
    if !sane(gs) {
        return 0;
    }
    let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(gs);
    f(ty)
}

/// Lê uma red::String (0x20 bytes): inline se length@0x14 < 0x4000_0000 (chars @ slot+0),
/// senão heap (char* @ slot+0). size real = length & 0x3FFF_FFFF. gum-checked.
pub unsafe fn red_string_read(slot: *const u8) -> String {
    if !crate::gum::is_readable(slot as *const c_void, 0x20) {
        return String::new();
    }
    let length = (slot.add(0x14) as *const u32).read_unaligned();
    let real = (length & 0x3FFF_FFFF) as usize;
    if real > 100_000 {
        return String::new();
    }
    let data: *const u8 = if length < 0x4000_0000 {
        slot
    } else {
        (slot as *const *const u8).read_unaligned()
    };
    if data.is_null() || !crate::gum::is_readable(data as *const c_void, real) {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(data, real)).into_owned()
}

/// Escreve string CURTA (≤19 UTF-8) num slot red::String, INLINE (não toca allocator → o
/// dtor inline do engine é no-op, sem corromper). Pra "Mods"/labels curtas. Slot deve estar
/// zerado/vazio (campo recém-Construct) — sobrescrever String HEAP existente vaza.
pub unsafe fn red_string_write_inline(slot: *mut u8, s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() > 19 {
        return false;
    }
    std::ptr::write_bytes(slot, 0, 0x20); // NUL + length=0 (inline) + allocator=0
    std::ptr::copy_nonoverlapping(b.as_ptr(), slot, b.len());
    (slot.add(0x14) as *mut u32).write_unaligned(b.len() as u32);
    true
}

/// Monta um `red::DynArray<CName>` de 16 bytes a partir de N CNames — usado p/ marshalar
/// `inkWidgetPath` (struct de 1 campo: DynArray<CName>@0x00, sizeof 0x10) no `GetWidgetByPath`.
/// Layout DynArray (macOS, RED4ext.SDK): entries(ptr)@0x00, **capacity@0x08, size@0x0C**
/// (capacity ANTES de size). Aloca o buffer no MESMO pool do new_object (PoolDefault), copia os
/// CName u64, e devolve os 16 bytes do header. Buffer é read-only durante a chamada (o engine só
/// itera entries[0..size] e copia internamente) → vazá-lo é aceitável (transiente, sem allocator
/// trailer — só preciso se o engine fosse dar realloc/free NESTE array, o que não ocorre num arg).
pub unsafe fn build_cname_dynarray(cns: &[u64]) -> Option<[u8; 16]> {
    let mut out = [0u8; 16]; // vazio: entries=null, cap=0, size=0 (GetAllocator usa &entries)
    let n = cns.len() as u32;
    if n == 0 {
        return Some(out);
    }
    let alloc: extern "C" fn(u64, u32) -> *mut c_void =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_ALLOC_ALIGNED));
    let buf = alloc((n as u64) * 8, 8);
    if buf.is_null() {
        return None;
    }
    for (i, &c) in cns.iter().enumerate() {
        ((buf as *mut u8).add(i * 8) as *mut u64).write_unaligned(c);
    }
    (out.as_mut_ptr() as *mut *mut c_void).write_unaligned(buf); // entries@0x00
    (out.as_mut_ptr().add(0x08) as *mut u32).write_unaligned(n); // capacity@0x08
    (out.as_mut_ptr().add(0x0C) as *mut u32).write_unaligned(n); // size@0x0C
    Some(out)
}

/// DIAGNÓSTICO (save-safe se rodado no MENU PRINCIPAL): despeja a vtable do ClassType
/// como VM addr ESTÁTICO (un_rebase → casa com `nm`/c++filt) e chama só os getters de
/// BAIXO RISCO (GetSize@0x10, GetAlignment@0x18). NÃO constrói/aloca nada. Serve p/
/// confirmar os offsets corretos do macOS antes de mexer no new_object.
pub unsafe fn dump_class(reg: &Registry, class_name: &str) -> String {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        return format!("[rttidump] classe '{class_name}' NAO encontrada no registry");
    }
    let vtbl = rd_ptr(cls as *const u8) as *const u8;
    if vtbl.is_null() {
        return format!("[rttidump] '{class_name}': cls={:p} mas vtbl=NULL", cls);
    }
    let mut s = format!(
        "[rttidump] '{class_name}': cls={:p} (static {:#x}) vtbl static {:#x}\n",
        cls,
        crate::un_rebase(cls),
        crate::un_rebase(vtbl as *const c_void),
    );
    for i in 0..24usize {
        let p = rd_ptr(vtbl.add(i * 8));
        s.push_str(&format!(
            "  vtbl+0x{:02x} = static {:#x}\n",
            i * 8,
            crate::un_rebase(p)
        ));
    }
    // getters const (lêem campo, não constroem/liberam → risco baixo).
    let get_size = rd_ptr(vtbl.add(0x10));
    if sane(get_size) {
        let gs: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_size);
        s.push_str(&format!("  -> GetSize@0x10(cls) = {}\n", gs(cls)));
    }
    let get_align = rd_ptr(vtbl.add(0x18));
    if sane(get_align) {
        let ga: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(get_align);
        s.push_str(&format!("  -> GetAlignment@0x18(cls) = {}\n", ga(cls)));
    }
    s
}

/// DIAGNÓSTICO de PROPRIEDADES (save-safe, só leitura): varre os offsets candidatos a
/// DynArray<CProperty*> na CClass e despeja cada CProperty cru (qwords [0..0x30]) pra eu
/// mapear o layout (qual qword é o CName do nome, qual u32 é o valueOffset, qual ptr é o
/// tipo). Já mostra o hash de nome contra alguns nomes conhecidos pra casar na hora.
pub unsafe fn dump_props(reg: &Registry, class_name: &str) -> String {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        return format!("[propdump] classe '{class_name}' NAO encontrada");
    }
    let clsb = cls as *const u8;
    let mut s = format!("[propdump] '{class_name}': cls static {:#x}\n", crate::un_rebase(cls));
    // nomes prováveis dos campos do NativeSettings — casa o hash on-the-fly.
    let known = [
        "label", "eventName", "action", "value", "menuListController", "data",
        "isEmpty", "id", "name", "optionType", "icon", "description", "selectable",
    ];
    // varre 0x18..0x70: cada par (ptr@off, count@off+8). TODA leitura é gum-checked →
    // não crasha em offset falso. Só REPORTA o array se algum entry casar um nome
    // conhecido (= é o DynArray<CProperty*> de verdade, não lixo).
    for off in (0x18usize..=0x70).step_by(8) {
        let arr = match rd_ptr_chk(clsb.add(off)) {
            Some(p) => p as *const u8,
            None => continue,
        };
        let cnt = match rd_u32_chk(clsb.add(off + 8)) {
            Some(c) => c,
            None => continue,
        };
        if arr.is_null() || cnt == 0 || cnt > 5000 {
            continue;
        }
        let show = cnt.min(48) as usize;
        let mut body = String::new();
        let mut matched = false;
        for i in 0..show {
            let pp = match rd_ptr_chk(arr.add(i * 8)) {
                Some(p) => p as *const u8,
                None => break,
            };
            // só lê os qwords do CProperty se [pp, pp+0x30) está mapeado.
            if pp.is_null() || !crate::gum::is_readable(pp as *const c_void, 0x30) {
                continue;
            }
            let q: [u64; 6] = std::array::from_fn(|j| rd_u64(pp.add(j * 8)));
            let mut hit = String::new();
            for k in known {
                let h = cname(k);
                for (j, &qv) in q.iter().enumerate() {
                    if qv == h {
                        hit = format!(" <== q[{j}]==cname(\"{k}\")");
                        matched = true;
                    }
                }
            }
            body.push_str(&format!(
                "  [{i:02}] q0={:#x} q1={:#x} q2={:#x} q3={:#x} q4={:#x} q5={:#x}{hit}\n",
                q[0], q[1], q[2], q[3], q[4], q[5]
            ));
        }
        if matched {
            s.push_str(&format!(
                "--- cls+0x{off:02x}: DynArray<CProperty*> count={cnt} (CASOU nome) ---\n"
            ));
            s.push_str(&body);
        }
    }
    // persiste num arquivo DEDICADO (sobrevive ao `clear` do log).
    let _ = std::fs::write("/tmp/cp77-propdump.txt", &s);
    s
}

/// Argumento já tipado pro slot de 0x20 bytes do frame.
pub enum Arg {
    Handle(*mut c_void, *mut c_void), // (instância, refcount)
    Item16([u8; 16]),                 // gameItemID já montado (fromTDBID)
    Raw([u8; 16]),                    // valor por-valor cru (ex.: GameInstance)
    Array([u8; 16]),                  // DynArray<T> de 16B já montado (ex.: inkWidgetPath)
    Str(String),                      // red::String INLINE no slot (≤19 chars) — p/ SetText
    I32(u32),
    I64(u64),
    F32(f32),
    Bool(bool),
    CName(u64),
    Enum(u64), // valor de enum já resolvido (escreve u64; engine lê a largura real)
    Tdb([u8; 8]),
}

/// Invoca `rf` com `ctx` e `args`. Monta locals + CProperty sintética por arg +
/// bytecode (LocalVar 0x18 … ParamEnd 0x26) + o CScriptStackFrame, e chama o
/// executor. Devolve os 16 bytes de retorno cru. `None` se algo estiver torto
/// (evita crashar o jogo — diferente do throw do JS dele).
pub unsafe fn call_func(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg]) -> Option<[u8; 16]> {
    let fb = rf.func as *const u8;
    let p_entries = rd_ptr(fb.add(0x28)) as *const u8;
    let p_count = rd_u32(fb.add(0x30));
    if args.len() > p_count as usize {
        return None; // arg count > params → o executor leria ParamEnd como valor e crasharia
    }
    if p_entries.is_null() && !args.is_empty() {
        return None;
    }

    let n = args.len();
    let mut locals = vec![0u8; 0x40 + n * 0x20];
    let mut props: Vec<Vec<u8>> = Vec::with_capacity(n);

    for (i, a) in args.iter().enumerate() {
        let off = 0x20 + i * 0x20;
        let dst = locals.as_mut_ptr().add(off);
        match a {
            Arg::Handle(inst, refcnt) => {
                (dst as *mut *mut c_void).write_unaligned(*inst);
                (dst.add(8) as *mut *mut c_void).write_unaligned(*refcnt);
            }
            Arg::Item16(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 16),
            Arg::Raw(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 16),
            Arg::Array(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 16),
            Arg::Str(s) => {
                std::ptr::write_bytes(dst, 0, 0x20);
                red_string_write_inline(dst, s);
            }
            Arg::I32(v) => (dst as *mut u32).write_unaligned(*v),
            Arg::I64(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::F32(v) => (dst as *mut f32).write_unaligned(*v),
            Arg::Bool(v) => dst.write(if *v { 1 } else { 0 }),
            Arg::CName(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::Enum(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::Tdb(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 8),
        }
        // CProperty sintética: +0 = ptype (de pEntries[i]), +0x20 = offset no locals
        let ptype = rd_ptr(rd_ptr(p_entries.add(i * 8)) as *const u8);
        let mut cp = vec![0u8; 0x30];
        (cp.as_mut_ptr() as *mut *mut c_void).write_unaligned(ptype);
        (cp.as_mut_ptr().add(0x20) as *mut u32).write_unaligned(off as u32);
        props.push(cp);
    }

    let mut bc = vec![0u8; 16 + n * 9];
    let mut o = 0usize;
    for cp in props.iter_mut() {
        bc[o] = 0x18; // LocalVar
        o += 1;
        (bc.as_mut_ptr().add(o) as *mut *mut c_void)
            .write_unaligned(cp.as_mut_ptr() as *mut c_void);
        o += 8;
    }
    bc[o] = 0x26; // ParamEnd

    let mut fr = vec![0u8; 0x90];
    (fr.as_mut_ptr() as *mut *mut c_void).write_unaligned(bc.as_mut_ptr() as *mut c_void);
    (fr.as_mut_ptr().add(0x10) as *mut *mut c_void).write_unaligned(locals.as_mut_ptr() as *mut c_void);
    (fr.as_mut_ptr().add(0x18) as *mut *mut c_void).write_unaligned(locals.as_mut_ptr() as *mut c_void);
    (fr.as_mut_ptr().add(0x40) as *mut *mut c_void).write_unaligned(ctx);

    let mut res = [0u8; 16];
    let exec: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(rebase(ADDR_EXEC));
    exec(
        rf.func,
        ctx,
        fr.as_mut_ptr() as *mut c_void,
        res.as_mut_ptr() as *mut c_void,
        rf.ret_type,
    );
    Some(res)
}

/// Monta um `gameItemID` (16B) a partir do nome do item via `ItemID.FromTDBID`.
/// Usa o bytecode de literal TweakDBID (opcode `0x11` + 8 bytes + `0x26`).
pub unsafe fn from_tdbid(reg: &Registry, name: &str) -> Option<[u8; 16]> {
    // Prefere o FromTDBID REAL capturado pela sonda (fn/ctx/ret) — com o ctx certo
    // o ItemID sai com SEED válido (itens não-stackable não crasham). Fallback:
    // resolve nós mesmos + ctx null (só serve p/ money/stackable).
    let (func, ctx, ret) = match read_fromtd() {
        Some(t) => t,
        None => {
            let rf = resolve_func(reg, "gameItemID", "FromTDBID")
                .or_else(|| resolve_func(reg, "ItemID", "FromTDBID"))?;
            (rf.func, std::ptr::null_mut(), rf.ret_type)
        }
    };
    let tb = crate::cname::tweak_db_id(name).to_le_bytes();
    let mut bc = vec![0u8; 16];
    bc[0] = 0x11; // push TweakDBID literal
    bc[1..9].copy_from_slice(&tb);
    bc[9] = 0x26; // ParamEnd
    let mut fr = vec![0u8; 0x90];
    (fr.as_mut_ptr() as *mut *mut c_void).write_unaligned(bc.as_mut_ptr() as *mut c_void);
    // *** o ctx do FromTDBID real vai em fr+0x40 (o que faltava) ***
    (fr.as_mut_ptr().add(0x40) as *mut *mut c_void).write_unaligned(ctx);
    let mut out = [0u8; 16];
    let exec: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(rebase(ADDR_EXEC));
    exec(
        func,
        ctx,
        fr.as_mut_ptr() as *mut c_void,
        out.as_mut_ptr() as *mut c_void,
        ret,
    );
    crate::log(&format!("[from_tdbid] '{name}' ctx={ctx:p} out={out:02x?}"));
    Some(out)
}

/// Lê o FromTDBID capturado pela sonda em /tmp/cp77-fromtd.txt (fn/ctx/ret).
unsafe fn read_fromtd() -> Option<(*mut c_void, *mut c_void, *mut c_void)> {
    // Captura NATIVA (nativo): o hook do executor publica fn/ctx/ret do FromTDBID em
    // atomics quando o jogo o chama. Antes vinha de /tmp/cp77-fromtd.txt escrito pela
    // sonda antiga — endereço de OUTRA sessão (ASLR) = morto = crash.
    use std::sync::atomic::Ordering;
    let c = crate::selfboot::FROMTD_CTX.load(Ordering::Relaxed);
    if c.is_null() {
        return None; // ainda não capturado (ex.: no menu) → fallback ctx=null (serve p/ money)
    }
    let f = crate::selfboot::FROMTD_TGT.load(Ordering::Relaxed);
    let r = crate::selfboot::FROMTD_RET.load(Ordering::Relaxed);
    if f.is_null() {
        return None;
    }
    Some((f, c, r))
}
