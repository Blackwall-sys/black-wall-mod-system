//! rtti.rs — universal-call do RED RTTI do CP2077, 100% Rust (sem JS).
//! Resolve uma função RED por nome (CName=FNV1a64) varrendo o registry e a
//! invoca pelo executor do jogo, montando o CScriptStackFrame na mão.
//!
//! Offsets/ABI = FATOS da RTTI do jogo (v2.3.1 build 5314028). Chamada = ponteiro transmutado em
//! `extern "C"` (AAPCS64) — NÃO precisa de gancho (`ganchos` é só para desviar código).

use std::convert::TryInto;
use std::ffi::c_void;

use crate::cname::cname;
use crate::rebase;

const ADDR_CRTTI_GET: u64 = 0x1_010b_3cac; // CRTTISystem::Get (acessor singleton)
/// PoolStorageProxy<red::PoolDefault>::AllocateAligned(u64 size, u32 align) → Block{x0=ptr,x1=size}.
/// Estática, NÃO zera. Aloca do POOL DO RED → o Free do engine (PushData etc.) casa (sem corromper).
/// Confirmado por disasm (workflow RE 2026-06-21). Ver [[cp77-macos-rtti-vtable-offsets]].
const ADDR_POOL_DEFAULT_ALLOC_ALIGNED: u64 = 0x1_0002_24a4;
/// PoolDefault::Free(red::memory::Block&) — Block = {ptr, size}. Libera o que AllocateAligned deu.
const ADDR_POOL_DEFAULT_FREE: u64 = 0x1_0002_292c;
/// `Handle<T>::Handle(T* aPtr)` — construtor UNIVERSAL de `ref<T>` do próprio motor (RE
/// 2026-07-18, sessão `handle-ctor-re`, fechando o bloqueador nº2 de handler de callback
/// documentado em `register.rs::write_handle_ret`/HISTORICO cont.100). Achado por vizinhança:
/// a rotina de RELEASE (teardown de locais `ref<>` compilados, já conhecida em `0x101025dc8`)
/// tem essa função-companheira de CONSTRUÇÃO logo ANTES no mesmo objeto/arquivo — mesmo padrão
/// `ldaddal` no mesmo campo (`block+4`), decoder completo em `0x101025c8c..0x101025d68`.
/// CONFIRMADO por 4045 call-sites reais (scan de `bl` no `__TEXT` inteiro) — o padrão em CADA
/// um é idêntico ao de `0x100f60eac`: `<constrói o objeto cru>; mov x1,x0 (raw ptr); add
/// x0,sp,#N (slot de 16 bytes); bl 0x101025c8c`. ABI: `void Handle_ctor(void* aOutHandle /*x0,
/// 16 bytes*/, void* aPtr /*x1*/)`. Semântica lida da disasm: sempre escreve `out[0]=aPtr`
/// primeiro (+`out[8]=0`); se `aPtr==null` retorna aí — handle nulo válido. Senão consulta uma
/// tabela de "weak owner" por PONTEIRO (`bl 0x1010b0fd8`): se já existe um refcount-block pra
/// este objeto (outro Handle vivo apontando pra ele), REUSA o bloco e `ldaddal` +1 no contador
/// (`block+0`, compartilhado — correto p/ múltiplos Handles ao MESMO objeto, ex. `self` sendo
/// devolvido em cadeia); senão ALOCA 8 bytes do pool `red::memory::PoolStorageProxy<PoolRefCount>
/// ::Allocate` (achado nos símbolos demangled — MESMO pool nomeado "PoolRefCount"), inicializa
/// `{1,1}` (2×u32: campo+0=1, campo+4=1 — o `+4` bate EXATO com o offset que a rotina de release
/// decrementa via `ldaddal`) e REGISTRA o par `(aPtr,bloco)` na tabela de weak-owner (`bl
/// 0x1010b12a0`) pra Handles futuros ao MESMO ponteiro reusarem o mesmo bloco. É a rotina REAL
/// que o motor chama em TODO `new T()`/factory que devolve `ref<T>` — não uma reimplementação
/// nossa do refcount.
const ADDR_HANDLE_CTOR: u64 = 0x1_0102_5c8c;
/// Tabela de handlers de opcode da VM redscript (__DATA.__common, preenchida em
/// runtime). `OPCODE_TABLE[*code](ctx, frame, &out, 0)` lê UM valor do frame.
/// Achado desmontando funcOperatorAdd<int> (lê 2 params via essa tabela).
const OPCODE_TABLE: u64 = 0x1_09a5_c940;

/// Último handler provado legível para cada opcode. O handler aponta para o `__TEXT` do jogo, que não
/// é desmapeado, então o `is_readable` (syscall de ~360 ns, bench na máquina) só precisa rodar na
/// primeira vez de cada endereço — não uma vez por parâmetro de toda native chamada. A entrada da
/// tabela é comparada por VALOR: se o motor trocar o handler, o endereço novo é checado de novo.
static HANDLER_DE_OPCODE_LEGIVEL: [std::sync::atomic::AtomicUsize; 256] =
    [const { std::sync::atomic::AtomicUsize::new(0) }; 256];

unsafe fn handler_de_opcode_legivel(opcode: u8, handler: usize) -> bool {
    use std::sync::atomic::Ordering;
    let provado = &HANDLER_DE_OPCODE_LEGIVEL[opcode as usize];
    if handler != 0 && provado.load(Ordering::Relaxed) == handler {
        return true;
    }
    let ok = handler != 0 && sane(handler as *mut c_void) && crate::memoria::is_readable(handler as *const c_void, 4);
    if ok {
        provado.store(handler, Ordering::Relaxed);
    }
    ok
}

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
    if !sane(ty) || !crate::memoria::is_readable(ty as *const c_void, 0x20) {
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

/// Aloca+constrói uma instância TIPADA transiente:
/// PoolDefault::AllocateAligned(GetSize@0x18, GetAlignment@0x20) → memset 0 → Construct@0x40.
/// O handler do opcode escreve o valor do param NESTA memória tipada — pra tipos complexos
/// (String/array/handle) é obrigatório (o handler escreve o objeto inteiro; buf de pilha estoura).
pub(crate) unsafe fn type_inst_alloc(ty: *mut c_void) -> (*mut c_void, usize) {
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
pub(crate) unsafe fn type_inst_free(ty: *mut c_void, inst: *mut c_void, size: usize) {
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

/// `Assign(lhs, rhs)` do tipo, `vt+0x58` no Mac: a ordem do SDK (`Construct`, `Destruct`, `IsEqual`, `Assign`, `Move`) com os 8 B do 2º destrutor, que já
/// bate no `Construct` `0x40` e no `Destruct` `0x48` usados acima. `false` com tipo, vtable ou slot ilegível; quem copia confere o slot uma vez antes
/// (`register::assign_confere`).
pub(crate) unsafe fn type_assign(ty: *mut c_void, lhs: *mut c_void, rhs: *const c_void) -> bool {
    if !sane(ty) || lhs.is_null() || rhs.is_null() {
        return false;
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() {
        return false;
    }
    let f = rd_ptr(vt.add(0x58));
    if !sane(f) {
        return false;
    }
    let assign: extern "C" fn(*mut c_void, *mut c_void, *const c_void) = std::mem::transmute(f);
    assign(ty, lhs, rhs);
    true
}

// Comparar e formatar valor de qualquer tipo, medidos nos handlers dos opcodes da VM (registrados por `0x1011a9554`): o `Equals` (42,
// `0x101201cbc`) chama `[vt+0x50](tipo, a, b, 0)` e depois o `Destruct` `[vt+0x48]` dos dois temporários; o `ToString` (91, `0x101202544`)
// passa pelo auxiliar `0x101202764(tipo, valor, &saída)`, que formata handle (tipo 9) com classe e endereço e manda o resto pro
// `[vt+0x70]`. O `ToVariant` (92, `0x1012037cc`) enche a variante por `0x1010dc4f4(variante, tipo, valor)`.
const SLOT_IGUAL: usize = 0x50;
const TEXTO_DO_VALOR_VM: u64 = 0x1_0120_2764;
const VARIANTE_PREENCHE_VM: u64 = 0x1_010d_c4f4;

/// Função do slot `off` da vtable do tipo, conferida; `None` com tipo, vtable ou função ilegível.
unsafe fn slot_do_tipo(ty: *mut c_void, off: usize) -> Option<*mut c_void> {
    if !sane(ty) {
        return None;
    }
    let vt = rd_ptr_chk(ty as *const u8)? as *const u8;
    let f = rd_ptr_chk(vt.add(off))?;
    sane(f).then_some(f)
}

/// `IsEqual(a, b)` do tipo: a comparação que o motor usa no `==` dos scripts (String por texto, handle pelo objeto, array elemento a elemento).
pub(crate) unsafe fn type_is_equal(ty: *mut c_void, a: *const c_void, b: *const c_void) -> Option<bool> {
    if a.is_null() || b.is_null() {
        return None;
    }
    let f: extern "C" fn(*mut c_void, *const c_void, *const c_void, u32) -> u8 = std::mem::transmute(slot_do_tipo(ty, SLOT_IGUAL)?);
    Some(f(ty, a, b, 0) & 1 == 1)
}

/// O texto que o `ToString` dos scripts dá ao valor (o auxiliar do motor: `[vt+0x70]` do tipo, ou classe e endereço pra handle), num `CString`
/// do motor montado e destruído aqui.
pub(crate) unsafe fn type_to_string(ty: *mut c_void, valor: *const c_void) -> Option<String> {
    if valor.is_null() || slot_do_tipo(ty, 0x70).is_none() {
        return None;
    }
    let f: extern "C" fn(*mut c_void, *const c_void, *mut u8) = std::mem::transmute(rebase(TEXTO_DO_VALOR_VM));
    let mut saida = [0u64; 4];
    let cs = saida.as_mut_ptr() as *mut u8;
    crate::jogo::cstring::nova(cs, "");
    f(ty, valor, cs);
    let texto = crate::jogo::cstring::texto(cs);
    crate::jogo::cstring::destroi(cs);
    Some(texto)
}

/// `(campo, tipo)` da propriedade `prop` em `obj`, com o endereço pelo `inValueHolder`: o ponto comum de quem lê, grava, compara ou formata
/// valor de campo (as natives de script e a `BwmsApi`).
pub(crate) unsafe fn campo_e_tipo(obj: *mut c_void, prop: *const c_void) -> Option<(*mut c_void, *mut c_void)> {
    if !sane(obj) || !sane(prop as *mut c_void) {
        return None;
    }
    let tipo = rd_ptr(prop as *const u8);
    let campo = prop_value_ptr(prop, obj);
    (sane(tipo) && !campo.is_null()).then_some((campo, tipo))
}

/// O valor do tipo `origem` pode ir pra um campo do tipo `destino`: o mesmo tipo, ou handle (ou `wref`) de classe filha pra handle da mãe —
/// o layout do handle é o mesmo e o `Assign` do destino copia o ponteiro e a contagem.
pub(crate) unsafe fn tipo_cabe_em(origem: *mut c_void, destino: *mut c_void) -> bool {
    if origem == destino {
        return true;
    }
    match (type_kind(origem), type_kind(destino)) {
        (Some(a), Some(b)) if a == b && matches!(a, 9 | 10) => match (get_inner_type(origem), get_inner_type(destino)) {
            (Some(filha), Some(mae)) => crate::register::class_is_kind_of(filha, mae),
            _ => false,
        },
        _ => false,
    }
}

/// Bit 0 do ponteiro de tipo do `Variant`: o valor está inline em `+0x08`.
const VARIANTE_INLINE: u64 = 1;

/// Partes de um `Variant` do motor (0x18): `(tipo, valor)`. O tipo mora em `+0x00` com o bit 0 ligado quando o valor está inline em `+0x08`
/// (até 16 B); sem o bit, `+0x08` aponta a instância. `None` com variante vazia.
pub(crate) unsafe fn variante_partes(v: *const u8) -> Option<(*mut c_void, *mut c_void)> {
    if v.is_null() {
        return None;
    }
    let bruto = (v as *const u64).read_unaligned();
    let tipo = (bruto & !VARIANTE_INLINE) as *mut c_void;
    if !sane(tipo) {
        return None;
    }
    let valor = if bruto & VARIANTE_INLINE != 0 { v.add(8) as *mut c_void } else { (v.add(8) as *const u64).read_unaligned() as *mut c_void };
    (!valor.is_null()).then_some((tipo, valor))
}

/// Monta em `out` (0x18 crus) um `Variant` do tipo com a cópia de `valor`, pelo preenchimento do próprio motor (o do `ToVariant`): inline ou
/// no alocador do tipo, `Construct` e `Assign`. Serve pra qualquer tamanho; a VM destrói a variante devolvida como destrói a dela.
pub(crate) unsafe fn variante_montar(out: *mut u8, ty: *mut c_void, valor: *const c_void) -> bool {
    std::ptr::write_bytes(out, 0, 24);
    if slot_do_tipo(ty, 0x58).is_none() || valor.is_null() {
        return false;
    }
    let f: extern "C" fn(*mut u8, *mut c_void, *const c_void) = std::mem::transmute(rebase(VARIANTE_PREENCHE_VM));
    f(out, ty, valor);
    variante_partes(out).is_some()
}

/// Parâmetros lidos do frame, `(valor cru, CName do tipo)`, guardados na pilha. O leitor recusa
/// `p_count > 16`, então cabem num array fixo: o `Vec` era uma alocação por native chamada (15–17 ns
/// contra 3–4 ns na pilha, bench `rustc -O`). Lê-se como slice.
#[derive(Clone, Copy)]
pub struct ArgsLidos {
    itens: [(u64, u64); 16],
    len: u8,
}

impl ArgsLidos {
    const fn vazio() -> Self {
        Self { itens: [(0, 0); 16], len: 0 }
    }
    fn push(&mut self, arg: (u64, u64)) {
        if let Some(livre) = self.itens.get_mut(self.len as usize) {
            *livre = arg;
            self.len += 1;
        }
    }
}

impl std::ops::Deref for ArgsLidos {
    type Target = [(u64, u64)];
    fn deref(&self) -> &[(u64, u64)] {
        &self.itens[..self.len as usize]
    }
}

/// Parâmetro `Bool` do script no qword de `ArgsLidos`: o tipo tem 1 byte (`GetSize()`), só o byte baixo é o valor.
#[inline]
pub(crate) const fn bool_do_script(v: u64) -> bool {
    v & 0xff != 0
}

/// Página do arm64 no macOS: um mapeamento é sempre de páginas inteiras, então 1 byte legível prova a
/// página toda.
const PAGINA_DO_ARM64: usize = 0x4000;

/// `[p, p+len)` legível, sem refazer o syscall (~360 ns) quando o intervalo cai inteiro na página já provada NESTA
/// chamada. Serve a leituras que andam pouco dentro de memória que o motor mantém viva durante a chamada: o bytecode
/// entre um parâmetro e o seguinte (o executor lê o mesmo buffer logo depois — se sumisse, o motor cairia antes de nós)
/// e as tabelas de função do RTTI numa busca. A prova não sobrevive à chamada (recarga de script libera bytecode, ver
/// `cp77-recarga-de-scripts-quebra-caches`).
unsafe fn legivel_na_pagina_provada(p: *const u8, len: usize, pagina_provada: &mut usize) -> bool {
    if p.is_null() || len == 0 {
        return false;
    }
    let pagina = p as usize & !(PAGINA_DO_ARM64 - 1);
    let mesma_pagina = (p as usize + len - 1) & !(PAGINA_DO_ARM64 - 1) == pagina;
    if mesma_pagina && pagina == *pagina_provada {
        return true;
    }
    let ok = crate::memoria::is_readable(p as *const c_void, len);
    if ok && mesma_pagina {
        *pagina_provada = pagina;
    }
    ok
}

unsafe fn read_params_inner(func: *mut c_void, frame: *mut c_void, consume: bool) -> ArgsLidos {
    read_params_inner_ext(func, frame, consume, None, None)
}

/// Núcleo de `read_params_inner` + captura OPCIONAL de valores `String` (2026-07-13, fecha
/// utilitários de script Hash.reds/Number.reds). `strings_out[i]` = `Some(texto)` se o param `i` é String
/// e leu OK; senão `None`. Passar `None` (via `read_params_inner`) mantém o comportamento
/// EXATO de antes — nenhum caller existente muda.
unsafe fn read_params_inner_ext(
    func: *mut c_void,
    frame: *mut c_void,
    consume: bool,
    strings_out: Option<&mut Vec<Option<String>>>,
    variants_out: Option<&mut Vec<Option<[u8; 24]>>>,
) -> ArgsLidos {
    read_params_inner_ext3(func, frame, consume, strings_out, variants_out, None, None, None, None)
}

/// Como `read_params_inner_ext`, com uma 4ª captura OPCIONAL: `scriptrefs_out[i]` = ponteiro cru
/// `T*` (`ScriptRef<T>{...ref: T*@0x18...}`) quando o param `i` é
/// `script_ref<T>` (ertti==15), pra QUALQUER `T` — generaliza o caso já provado só pra `String`
/// (`scriptref_data_ptr` local, usado por `read_cstring` abaixo). 2026-08-12, round 5: permite ESCREVER campos dentro do `T*` real (não só ler `String`),
/// reusando a MESMA leitura de `ScriptRef` já confirmada — `inkWidgetRef.Set`/
/// `inkWidgetLibraryResource.SetPath` (`@addMethod` ESTÁTICO com `self: script_ref<T>`,
/// evita o bug de `this`-por-struct já documentado) só escrevem 1 campo de `T` via ponteiro cru,
/// zero vtable/RawFunc. `None` mantém comportamento IDÊNTICO (nenhum caller existente muda).
unsafe fn read_params_inner_ext3(
    func: *mut c_void,
    frame: *mut c_void,
    consume: bool,
    mut strings_out: Option<&mut Vec<Option<String>>>,
    mut variants_out: Option<&mut Vec<Option<[u8; 24]>>>,
    mut scriptrefs_out: Option<&mut Vec<u64>>,
    mut raw16_out: Option<&mut Vec<Option<[u8; 16]>>>,
    mut cnamearray_out: Option<&mut Vec<Option<Vec<u64>>>>,
    mut visita: Option<&mut dyn FnMut(usize, *mut c_void, *mut c_void)>,
) -> ArgsLidos {
    if func.is_null() || frame.is_null() {
        return ArgsLidos::vazio();
    }
    // Sanidade: frame mapeado. Lê EXATAMENTE p_count (= apFunction->params.size) args, cada um
    // numa instância TIPADA. O bug antigo era passar buf[16] de pilha; pra tipo complexo o handler
    // escreve o objeto inteiro e estoura = crash do AddMenuItem nativo.
    if !crate::memoria::is_readable(frame as *const c_void, 0x68) {
        return ArgsLidos::vazio();
    }
    let p_count = rd_u32((func as *const u8).add(0x30)) as usize;
    if p_count == 0 || p_count > 16 {
        return ArgsLidos::vazio();
    }
    let p_entries = rd_ptr((func as *const u8).add(0x28)) as *const u8;
    let f = frame as *mut u8;
    // salva code/currentParam/data/dataType — a ORIGINAL re-lê depois (o `code` é restaurado).
    let save_code = (f as *const *const u8).read_unaligned();
    let save_cnt = *f.add(0x62);
    let save_30 = (f.add(0x30) as *const u64).read_unaligned();
    let save_38 = (f.add(0x38) as *const u64).read_unaligned();
    let table = rebase(OPCODE_TABLE) as *const usize;
    let mut out = ArgsLidos::vazio();
    let mut pagina_do_bytecode = usize::MAX;
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
        // setup por param: currentParam++, data/dataType = null.
        *f.add(0x62) = (*f.add(0x62)).wrapping_add(1);
        (f.add(0x30) as *mut u64).write_unaligned(0);
        (f.add(0x38) as *mut u64).write_unaligned(0);
        let ctx = (f.add(0x40) as *const *mut c_void).read_unaligned();
        let code = (f as *const *const u8).read_unaligned();
        if !legivel_na_pagina_provada(code, 1, &mut pagina_do_bytecode) {
            type_inst_free(ptype, pinst, psize);
            break;
        }
        let opcode = *code;
        (f as *mut *const u8).write_unaligned(code.add(1)); // *code++
        let handler = *table.add(opcode as usize);
        if !handler_de_opcode_legivel(opcode, handler) {
            type_inst_free(ptype, pinst, psize);
            break;
        }
        let h: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) =
            std::mem::transmute(handler);
        // ERTTIType do param (GetType@vtable+0x28): decide scriptRefOut + como ler o valor.
        // 9=Handle, 10=WeakHandle, 5=Enum, 15=ScriptReference. (Itanium não desloca DADO, só
        // método de vtable; GetType é Windows 0x20 → macOS 0x28.)
        // Inline, sem os dois `is_readable` do `type_kind` (~720 ns): o `ptype` veio da `CProperty` do motor.
        let categoria = {
            let vt = rd_ptr(ptype as *const u8) as *const u8;
            if !vt.is_null() {
                let gt = rd_ptr(vt.add(0x28));
                if sane(gt) {
                    let f: extern "C" fn(*mut c_void) -> u8 = std::mem::transmute(gt);
                    f(ptype)
                } else {
                    u8::MAX
                }
            } else {
                u8::MAX
            }
        };
        // script_ref<T>=15 precisa de pinst como scriptRefOut (nos outros tipos, nulo) — senão o frame não avança e o param SEGUINTE sai 0.
        let scriptref_out = if categoria == ertti::SCRIPT_REFERENCE {
            pinst
        } else {
            std::ptr::null_mut()
        };
        h(ctx, frame, pinst, scriptref_out);
        // 2026-08-10: quando ertti==15, `pinst` agora contém o wrapper
        // `ScriptRef<T> { unk00[0x10], innerType: IType*@0x10, ref: T*@0x18, hash: CName@0x20 }`
        // (`CRTTIScriptReferenceType`). `ref@0x18` é o PONTEIRO pro dado real por trás do
        // `script_ref<T>` — usado abaixo só pra extrair `String` (o caso concreto documentado
        // em `Utils/Hash.reds`/`Number.reds`: `FNV1a64(data: script_ref<String>, ...)`).
        // Generalizar pra outros T (Int32/Float/...) fica pra quando surgir caso de uso real.
        let scriptref_data_ptr = if categoria == ertti::SCRIPT_REFERENCE && crate::memoria::is_readable((pinst as *const u8).add(0x18) as *const c_void, 8) {
            rd_ptr((pinst as *const u8).add(0x18))
        } else {
            std::ptr::null_mut()
        };
        // lê 8 bytes de pinst+0 quando o "valor" cabe em um qword: ESCALAR (CName/Int/Float/
        // Bool/...) OU Handle(9)/WeakHandle(10) — nestes pinst+0 = PONTEIRO do objeto, que o
        // decode_arg embrulha em Handle (ex.: target:ref<ListItemController> do OnMenuItemActivated,
        // que a página de settings de mod usa p/ setar fromMods). String/array/ScriptRef/struct = 0
        // (o callback lê do objeto destino). O objeto do Handle vive além do free (dono = menu).
        // `ResRef` (2026-07-24): confirmado que é um struct de 8 bytes
        // achatado envolvendo só um `ResourcePath` (u64, o mesmo FNV-1a64 que `resource.link`/
        // `bwms-primitives::resource_path_hash` já usam em todo o projeto) — categoria idêntica a
        // TweakDBID/EntityID (struct achatado de 1 qword), NÃO parente de String/CString (essa é
        // 0x20 bytes, SSO+ptr+len+allocator). Ler como escalar aqui é seguro (`type_inst_alloc` já
        // aloca pelo tamanho REAL do tipo via vtable, não um buffer fixo) — só ainda não é ÚTIL pro
        // lado de ESCREVER/devolver um ResRef novo (RE separada, documentada em `register.rs`
        // `register_resourceevent`).
        // 2026-08-09 (achado ao vivo, smoke de casts): CRUID/NodeRef são wrappers
        // de 1 qword IDÊNTICOS a EntityID/TweakDBID/ResRef (mesma categoria "struct achatado"),
        // mas faltavam nesta whitelist — CRUIDToHash/NodeRefToHash como PARÂMETRO (não como
        // retorno, que já usava `write_uint_ret` sem problema) sempre lia `raw=0` (confirmado
        // por log: `arg_tc_name=CRUID arg_raw=0x0` mesmo com o valor real gravado no slot).
        // 2º achado no MESMO teste: `EntityID` já estava na whitelist mas NUNCA batia — o CNAME
        // real do tipo (o que `GetType()` devolve pro parâmetro) é `entEntityID`, não `EntityID`
        // (nome redscript-facing ≠ nome interno do motor; confirmado por log:
        // `arg_tc_name=entEntityID`). O `cname("EntityID")` antigo comparava contra um hash que
        // nunca aparecia de verdade — bug latente desde que a entrada foi adicionada (2026-07-24),
        // nunca pego porque `EntityID` só tinha sido exercitado como TIPO DE RETORNO até agora
        // (retorno usa `write_uint_ret` direto, não passa por este whitelist de leitura).
        let escalar = tc == cname("CName")
            || tc == cname("Int32")
            || tc == cname("Uint32")
            || tc == cname("Int64")
            || tc == cname("Uint64")
            || tc == cname("Float")
            || tc == cname("Bool")
            || tc == cname("TweakDBID")
            || tc == cname("entEntityID")
            || tc == cname("ResRef")
            // 2026-08-10 (diagnóstico ao vivo pendente, smoke de hash→ResRef):
            // `ResRef` sozinho leu `raw=0` pra um param real — candidato, mesma categoria do bug
            // já achado 2x nesta whitelist (nome redscript-facing != nome interno do motor, ver
            // EntityID/entEntityID acima). Nome do tipo C++ real por trás de `ResRef`.
            || tc == cname("redResourceReferenceScriptToken")
            || tc == cname("CRUID")
            || tc == cname("NodeRef")
            // 2026-08-12 (rodada de composição barata): `EngineTime`
            // é `importonly struct` VANILLA (usado em dezenas de scripts
            // reais, `orphans.script:22270`, ex. `GameInstance.GetEngineTime()`), achatado num único
            // `ticks: uint64_t` — mesma categoria "struct de 1 qword" de CRUID/NodeRef/TweakDBID/EntityID/ResRef acima.
            // NUNCA confirmado ao vivo (sessão offline, sem boot) — pelo padrão já achado 2x nesta
            // MESMA whitelist (EntityID/ResRef), o nome interno real do `GetType()` PODE divergir do
            // nome redscript-facing; aqui é um tipo genuinamente vanilla (não um alias
            // como os outros 2 casos), então a hipótese de bater é mais forte, mas
            // ainda não é certeza — confirmar contra log real (`arg_tc_name`) na próxima sessão com
            // boot antes de contar como fechado.
            || tc == cname("EngineTime");
        let read8 = escalar || categoria == ertti::HANDLE || categoria == ertti::WEAK_HANDLE || categoria == ertti::ENUM;
        // 2026-08-15 (rodada 12, fix da MÁQUINA GENÉRICA — não mais só o call-site individual):
        // o bug achado na rodada 11 (`SetAttached(false)` lendo lixo do pool nos bytes 1-7 de um
        // `Bool`) não era do trampolim, era DAQUI. `type_inst_alloc` já zera CORRETAMENTE só
        // `GetSize()` bytes (1 pra `Bool`, 4 pra `Int32`/`Float`/muitos `Enum`, 8 pra tipos
        // achatados como `CName`/`TweakDBID`/handles) — mas esta leitura sempre lia 8 bytes CRUS
        // de `pinst`, incondicional. Pra tipos de 8 bytes isso é correto (é o próprio valor); pra
        // tipos MENORES é uma leitura FORA DO ALOCADO (a alocação real só tem `psize` bytes) que
        // pega lixo determinístico de uma alocação anterior do pool. Isso é inofensivo pra
        // `Int32`/`Uint32`/`Float` (todo consumidor já faz `as i32`/`as u32`/`f32::from_bits(v as
        // u32)`, cast que trunca os bytes altos — o lixo nunca aparece no resultado), mas
        // CORROMPE qualquer comparação `*v != 0` sem máscara sobre um tipo de 1-4 bytes (Bool
        // sendo o caso mais perigoso: 1 bit de lixo em qualquer um dos 7 bytes extras vira `true`
        // por engano). Fix: ler só os `psize` (`.min(8)`) bytes REAIS de `pinst`, zero-estendido
        // pros bytes que faltam — pra qualquer tipo já testado com `psize>=8` (todos os "struct
        // achatado de 1 qword": CName/TweakDBID/entEntityID/ResRef/CRUID/NodeRef/EngineTime,
        // Handle/WeakHandle) isso lê EXATAMENTE os mesmos 8 bytes de antes, byte-a-byte idêntico —
        // zero mudança de comportamento pra qualquer caso já provado. Só os tipos <8 bytes
        // (Bool/Int32/Uint32/Float/muitos Enum) mudam: de "8 bytes crus com lixo" pra "N bytes
        // reais + zero-extend", que é o comportamento CORRETO desde sempre. Fecha a classe
        // inteira do bug de uma vez (Bits.reds `BitSet*`, `ToCharacterKey.shift`,
        // `ProcessPostLoad.disablePreInitialization`, `VehicleSystem.ToggleGarageVehicle.enable`
        // — os 4 outros call-sites achados na mesma varredura, nenhum precisou de máscara local).
        let raw = if read8 {
            let n = psize.min(8);
            if n > 0 && crate::memoria::is_readable(pinst as *const c_void, n) {
                let mut buf = [0u8; 8];
                std::ptr::copy_nonoverlapping(pinst as *const u8, buf.as_mut_ptr(), n);
                u64::from_le_bytes(buf)
            } else {
                0
            }
        } else {
            0
        };
        // Recurso que o BWMS entregou ao script dentro de um `BwmsResource` volta cru pra native (o objeto do campo privado).
        let raw = if (categoria == ertti::HANDLE || categoria == ertti::WEAK_HANDLE) && raw != 0 { crate::register::recurso_do_parametro(raw) } else { raw };
        out.push((raw, tc));
        if let Some(srefs) = scriptrefs_out.as_deref_mut() {
            srefs.push(scriptref_data_ptr as u64);
        }
        if let Some(strs) = strings_out.as_deref_mut() {
            let s = if tc == cname("String") {
                read_cstring(pinst as *const u8)
            } else if tc == cname("LocalizationString") {
                // Candidato da 6ª rodada (2026-08-11): o struct
                // `LocalizationString{int64_t unk00; CString unk08;}` (tamanho 0x28) —
                // um `int64` seguido do MESMO layout de `CString` (0x20 bytes) já lido em
                // `pinst` pra qualquer param `String`. `type_inst_alloc(ptype)` já aloca `pinst`
                // pelo TAMANHO REAL do tipo (via vtable GetSize, não um buffer fixo) — pra
                // LocalizationString isso são os 0x28 bytes inteiros, então basta ler o CString
                // embutido em `pinst+0x08` (mesmo idioma já usado abaixo pro `scriptref_data_ptr`,
                // que também lê de um offset DENTRO de outro ponteiro). Confirmado que o tipo é
                // genuinamente redscript-visível (não injetado por mod): `orphans.script:21668`
                // declara `LocalizationStringComponent.GetString(key: CName) -> LocalizationString`
                // como NATIVA VANILLA real do próprio jogo.
                read_cstring((pinst as *const u8).add(8))
            } else if !scriptref_data_ptr.is_null() {
                read_cstring(scriptref_data_ptr as *const u8)
            } else {
                None
            };
            strs.push(s);
        }
        // `StaticEntitySpec.position`+`.orientation` (2026-08-15, rodada 10):
        // captura os 16 bytes crus de um param `Vector4`/`Quaternion` — MESMO padrão já provado
        // pra `Variant`(24B, acima)/`CString`(0x20B, `read_cstring`): `pinst` já é alocado pelo
        // TAMANHO REAL do tipo (`type_inst_alloc`, via vtable `GetSize`, CONFIRMADO 16 pra Vector4
        // desde 2026-06-20, ver o doc de `type_inst_alloc`) e populado pelo MESMO dispatch
        // de opcode (`h(ctx,frame,pinst,scriptref_out)`) que QUALQUER outro param usa — é o
        // idioma genérico que a VM real usa pra decodificar `new Vector4(...)`/etc. como argumento
        // de QUALQUER native (nossa ou vanilla). Diferente de CString (SSO+ptr+allocator, precisa
        // de lógica de leitura), Vector4/Quaternion são POD puro (4 floats) — memcpy direto dos
        // 16 bytes é seguro, sem nenhum campo de posse/ownership envolvido.
        if let Some(r16) = raw16_out.as_deref_mut() {
            let is_v4q = tc == cname("Vector4") || tc == cname("Quaternion");
            let bytes = if is_v4q && crate::memoria::is_readable(pinst as *const c_void, 16) {
                let mut b = [0u8; 16];
                std::ptr::copy_nonoverlapping(pinst as *const u8, b.as_mut_ptr(), 16);
                Some(b)
            } else {
                None
            };
            r16.push(bytes);
        }
        if let Some(vars) = variants_out.as_deref_mut() {
            // Captura os 24 bytes ANTES do destructor (inline data fica no buffer; heap = aponta pra
            // dentro do objeto e sobrevive se IsInline=true — IntXX/Float/Bool/CName/TweakDBID).
            let vbytes = if tc == cname("Variant") && crate::memoria::is_readable(pinst as *const c_void, 24) {
                let mut buf = [0u8; 24];
                std::ptr::copy_nonoverlapping(pinst as *const u8, buf.as_mut_ptr(), 24);
                Some(buf)
            } else {
                None
            };
            vars.push(vbytes);
        }
        // `DynamicEntityTarget.Tags(tags: array<CName>)` (2026-08-18): 1ª leitura
        // deste projeto de `array<T>` como PARÂMETRO de entrada (toda leitura anterior de
        // `DynArray<T>` foi ou campo embutido num struct C++ real, ex. `find_garment_component_
        // by_cname`/`read_dynarray_len` acima em `register.rs`, ou RETORNO de native, nunca
        // argumento). Reusa a MESMA máquina genérica já provada pra CString(0x20B)/Vector4(16B)/
        // Variant(24B): `type_inst_alloc(ptype)` já aloca `pinst` pelo TAMANHO REAL do tipo (via
        // vtable `GetSize()` do `IType` do parâmetro, não um buffer fixo) — pra `array<CName>` isso
        // é o `DynArray<CName>` inteiro (16 bytes: `entries:*const T@0x00`/`cap:u32@0x08`/
        // `size:u32@0x0C`, MESMO layout já confirmado dezenas de vezes neste projeto, ex.
        // `read_dynarray_len`/`register.rs`), populado pelo MESMO dispatch de opcode
        // (`h(ctx,frame,pinst,scriptref_out)`) que QUALQUER outro param usa — é o idioma genérico
        // que a VM real usa pra decodificar `[n"a",n"b"]`/uma variável `array<CName>` como argumento
        // de QUALQUER native (nossa ou vanilla), não um mecanismo novo. `tc` pro tipo `array<CName>`
        // é a CName REAL computada pelo próprio motor via `GetName()` (não uma string sintetizada
        // por nós) — o formato `"array:<InnerTypeName>"` já está confirmado neste projeto
        // (`BwmsGetInnerTypeName`, "array:Float"→"Float"), então `cname("array:
        // CName")` computa o MESMO hash FNV1a64 que `GetName()` devolve pro tipo do parâmetro —
        // comparação de hash puro, sem precisar internar a string no `CNamePool` real.
        if let Some(arrs) = cnamearray_out.as_deref_mut() {
            let is_cname_arr = tc == cname("array:CName");
            let values = if is_cname_arr && crate::memoria::is_readable(pinst as *const c_void, 16) {
                let size = core::ptr::read_unaligned((pinst as *const u8).add(0x0C) as *const u32);
                if size == 0 {
                    Some(Vec::new())
                } else if size <= 4096 {
                    let entries = (pinst as *const u8 as *const u64).read_unaligned() as *const u64;
                    if !entries.is_null() && crate::memoria::is_readable(entries as *const c_void, (size as usize) * 8) {
                        let mut v = Vec::with_capacity(size as usize);
                        for i in 0..size as usize {
                            v.push(core::ptr::read_unaligned(entries.add(i)));
                        }
                        Some(v)
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            arrs.push(values);
        }
        if let Some(v) = visita.as_mut() {
            v(i, ptype, pinst);
        }
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
    } else {
        // 2026-09-12 (boot 66): o handler vanilla avança 1 byte depois dos params (`0x10310a9e4`, handler do `GetQuestMappinPosition`:
        // `ldr x8,[frame]; add x8,#1; str x8,[frame]`) — é o `ParamEnd` (opcode 0x26). Sem isso o byte sobra no bytecode: numa chamada solta vira
        // instrução vazia, mas com a native nossa como argumento de outra chamada ele toma o lugar do argumento seguinte (boot 66:
        // `Create(BwmsQuestPhaseResourceGraph(r), recursive)` recebeu `recursive` 0, e na chamada direta o CName chegou como `0x1`, o valor do Int32).
        let code = (f as *const *const u8).read_unaligned();
        if legivel_na_pagina_provada(code, 1, &mut pagina_do_bytecode) && *code == 0x26 {
            (f as *mut *const u8).write_unaligned(code.add(1));
        }
    }
    out
}

/// Lê os params CONSUMINDO (frame avançado), como o read de param das native funcs: frame+0x00 = code ptr; frame+0x40 = ctx;
/// frame+0x62 = contador; frame+0x30 scratch. Cada param vem como u64 cru (o caller decide se é ponteiro/handle ou escalar).
/// Lê os params sem avançar o frame: a original do método observado lê de novo depois do callback.
#[cfg(feature = "lua")]
pub(crate) unsafe fn read_params(func: *mut c_void, frame: *mut c_void) -> ArgsLidos {
    read_params_inner(func, frame, false)
}

pub unsafe fn read_params_consuming(func: *mut c_void, frame: *mut c_void) -> ArgsLidos {
    read_params_inner(func, frame, true)
}

/// Como `read_params_consuming`, mas TAMBÉM extrai o texto de qualquer param `String` (via
/// `read_cstring`) — o vetor devolvido tem o MESMO tamanho e índices do `Vec<(u64,u64)>`
/// principal; `strings[i]` é `Some(texto)` só quando o param `i` é String. Usado pelos
/// handlers de utilitários de script Hash.reds/Number.reds (`FNV1a64(data: script_ref<String>, ...)`).
pub unsafe fn read_params_consuming_with_strings(func: *mut c_void, frame: *mut c_void) -> (ArgsLidos, Vec<Option<String>>) {
    let mut strings = Vec::new();
    let args = read_params_inner_ext(func, frame, true, Some(&mut strings), None);
    (args, strings)
}

/// Como `read_params_consuming`, mas TAMBÉM captura o ponteiro cru `T*` de qualquer param
/// `script_ref<T>` (via `ScriptRef<T>::ref@0x18`, já provado) — generaliza pra
/// QUALQUER `T` a extração que antes só existia hard-coded pra `String`. `scriptrefs[i]` = ponteiro
/// não-nulo se o param `i` é `script_ref<T>` e leu OK; `0` caso contrário. 2026-08-12, round 5: permite compor natives que ESCREVEM dentro de um `script_ref<T>` de saída
/// (ex. `inkWidgetRef.Set`/`inkWidgetLibraryResource.SetPath`), não só ler.
pub unsafe fn read_params_consuming_with_scriptrefs(func: *mut c_void, frame: *mut c_void) -> (ArgsLidos, Vec<u64>) {
    let mut scriptrefs = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, Some(&mut scriptrefs), None, None, None);
    (args, scriptrefs)
}

/// Como `read_params_consuming`, mas TAMBÉM captura os 16 bytes crus de qualquer param
/// `Vector4`/`Quaternion` (POD, 4 floats) — `raw16[i]` = `Some(bytes)` quando o param `i` é um
/// desses 2 tipos e leu OK; `None` caso contrário. Fecha `StaticEntitySpec.
/// SetPosition`/`SetOrientation` (2026-08-15, rodada 10) — 1ª vez que struct-de-16-bytes é lido
/// como PARÂMETRO nesta codebase (antes só como RETORNO de natives vanilla, ex. `#29`). Reusa a
/// MESMA máquina genérica (`type_inst_alloc`+dispatch de opcode) já provada pra CString/Variant,
/// só lendo mais bytes de `pinst` no final — ver comentário completo em `read_params_inner_ext3`.
pub unsafe fn read_params_consuming_with_raw16(func: *mut c_void, frame: *mut c_void) -> (ArgsLidos, Vec<Option<[u8; 16]>>) {
    let mut raw16 = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, None, Some(&mut raw16), None, None);
    (args, raw16)
}

/// Como `read_params_consuming`, mas TAMBÉM captura os valores de qualquer param `array<CName>`
/// (`cnamearrays[i] = Some(Vec<u64>)` com os hashes CName na ORDEM real do array, ou `Some(vec![])`
/// se o array veio vazio; `None` se o param `i` não é `array<CName>`).
/// (`DynamicEntityTarget.Tags(tags: array<CName>)`/`StaticEntityTarget.Tags`, 2026-08-18) — 1ª
/// leitura de `array<T>` como PARÂMETRO de entrada nesta codebase (ver comentário completo em
/// `read_params_inner_ext3`, bloco `cnamearray_out`). Generaliza pra qualquer native futura que
/// precise receber uma LISTA de CNames do redscript (não só este item).
pub unsafe fn read_params_consuming_with_cnamearray(func: *mut c_void, frame: *mut c_void) -> (ArgsLidos, Vec<Option<Vec<u64>>>) {
    let mut arrs = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, None, None, Some(&mut arrs), None);
    (args, arrs)
}

/// Lê e consome os parâmetros chamando `visita(índice, tipo, instância)` em cada um ANTES do destrutor: é onde um valor com posse (String,
/// handle, array, o conteúdo de um `Variant`) ainda está vivo pra ser copiado pelo `Assign` do tipo.
pub unsafe fn read_params_consuming_visit(func: *mut c_void, frame: *mut c_void, visita: &mut dyn FnMut(usize, *mut c_void, *mut c_void)) -> ArgsLidos {
    read_params_inner_ext3(func, frame, true, None, None, None, None, None, Some(visita))
}

/// Como `read_params_consuming`, mas captura os 24 bytes brutos de qualquer param `Variant` ANTES do
/// destructor. `variants[i]` = `Some([u8;24])` se param i era Variant, `None` caso contrário.
/// Tipos inline (Int32/Float/Bool/CName/TweakDBID): bytes 8..24 contêm o valor diretamente.
pub unsafe fn read_params_consuming_with_variants(
    func: *mut c_void,
    frame: *mut c_void,
) -> (ArgsLidos, Vec<Option<[u8; 24]>>) {
    let mut variants = Vec::new();
    let args = read_params_inner_ext(func, frame, true, None, Some(&mut variants));
    (args, variants)
}

/// Extrai o CName hash do tipo armazenado no Variant (bytes 0..8 do buffer de 24).
/// Bit 0 = InlineFlag; mascara com !1 pra obter o IType*. Chama GetName() nele.
pub unsafe fn variant_type_cname(buf: &[u8; 24]) -> u64 {
    let raw_type = (buf.as_ptr() as *const usize).read_unaligned();
    let type_ptr = (raw_type & !1usize) as *mut c_void;
    if type_ptr.is_null() {
        return 0;
    }
    type_name_getname(type_ptr)
}

/// Lê o qword nos bytes 8..16 do Variant (inline value para tipos ≤8 bytes:
/// Int32, Uint32, Float, Bool, CName, TweakDBID, EntityID).
pub fn variant_inline_u64(buf: &[u8; 24]) -> u64 {
    u64::from_le_bytes(buf[8..16].try_into().unwrap_or([0u8; 8]))
}

/// Item #316 (`PENDENCIAS-UNIFICADAS.md`, marshalling de Variant): CONSTRÓI um
/// `Variant` NOVO (24 bytes) pra um tipo ≤8 bytes (Int32/Float/Bool/CName/TweakDBID/etc, os
/// mesmos que `variant_inline_u64` já lê) — o inverso exato de `variant_type_cname`+
/// `variant_inline_u64`: byte 0 = `(itype_ptr | 1)` (bit0=InlineFlag, mesmo truque já confirmado
/// do lado leitura), bytes 8..16 = valor inline, bytes 16..24 = 0 (não usado no caso inline).
/// `itype` vem de `register::get_type` (vtbl+0x00 do CRTTISystem, já provado seguro). None se o
/// tipo não resolver.
pub unsafe fn build_variant_inline(reg: &Registry, type_name: &str, value: u64) -> Option<[u8; 24]> {
    let itype = crate::register::get_type(reg, type_name);
    if !sane(itype) {
        return None;
    }
    let mut buf = [0u8; 24];
    let tagged = (itype as usize) | 1;
    buf[0..8].copy_from_slice(&(tagged as u64).to_le_bytes());
    buf[8..16].copy_from_slice(&value.to_le_bytes());
    Some(buf)
}

/// Lê o conteúdo de um `red::CString` (o `String` nativo do redscript) a partir do ponteiro pro
/// objeto (`pinst` — 0x20 bytes, layout de dados idêntico entre Windows/macOS — só vtable/dtor-count difere, já
/// documentado em [[cp77-macos-rtti-vtable-offsets]]): união SSO de 20 bytes em `+0x00`
/// (`inline_str[0x14]` OU `{ptr:8, unk:8, capacity:4}`), `length` (u32, com a flag "não-inline"
/// no bit 30 = `0x40000000`) em `+0x14`, `allocator` em `+0x18`. `IsInline()` = `length <
/// 0x40000000`; `Length()` = `length & 0x3FFFFFFF`; `c_str()` = inline_str OU o ponteiro heap.
/// Fecha a represa de leitura de String nativa (bloqueava utilitários de script Hash.reds/Number.reds) —
/// achado 2026-07-13, ainda SEM confirmação in-game (layout vem do SDK, não de dump ao vivo).
pub unsafe fn read_cstring(pinst: *const u8) -> Option<String> {
    const CSTRING_SIZE: usize = 0x20;
    const NOT_INLINE_FLAG: u32 = 0x4000_0000;
    const LENGTH_MASK: u32 = 0x3FFF_FFFF;
    if pinst.is_null() || !crate::memoria::is_readable(pinst as *const c_void, CSTRING_SIZE) {
        return None;
    }
    let length_raw = (pinst.add(0x14) as *const u32).read_unaligned();
    let length = (length_raw & LENGTH_MASK) as usize;
    if length == 0 {
        return Some(String::new());
    }
    let is_inline = length_raw < NOT_INLINE_FLAG;
    // O inline cabe em 0x14 B; comprimento maior com o bit de inline é lixo. Teto de 1 MB pro de fora: comprimento lido de um offset errado chega a
    // 1 GB, e o `is_readable` só confere os primeiros 512 B — no boot 162 isso levou a leitura pra uma página protegida.
    if (is_inline && length > 0x14) || length > (1 << 20) {
        return None;
    }
    let text_ptr = if is_inline {
        pinst // union começa em +0x00; inline_str É o próprio início do objeto
    } else {
        rd_ptr(pinst) as *const u8 // union.str.ptr, também em +0x00
    };
    if text_ptr.is_null() {
        return None;
    }
    if length <= 512 {
        if !crate::memoria::is_readable(text_ptr as *const c_void, length) {
            return None;
        }
        let bytes = std::slice::from_raw_parts(text_ptr, length);
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut bytes = vec![0u8; length];
    if !crate::memoria::read_chunk(text_ptr as usize, &mut bytes) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[inline]
pub(crate) unsafe fn rd_ptr(p: *const u8) -> *mut c_void {
    (p as *const *mut c_void).read_unaligned()
}
#[inline]
unsafe fn rd_u32(p: *const u8) -> u32 {
    (p as *const u32).read_unaligned()
}
/// `register::rd_u64` com o deslocamento já somado ao ponteiro.
#[inline]
pub(crate) unsafe fn rd_u64(p: *const u8) -> u64 {
    crate::register::rd_u64(p as *const c_void, 0)
}

/// Leituras SEGURAS (via `memoria::is_readable`) — None se o endereço não estiver mapeado.
/// Base dos diagnósticos/resolvedores que leem structs RTTI a partir de offsets ainda
/// não validados (evita segfault ao tocar ponteiro "são" porém não-mapeado).
#[inline]
unsafe fn rd_ptr_chk(p: *const u8) -> Option<*mut c_void> {
    crate::memoria::is_readable(p as *const c_void, 8).then(|| rd_ptr(p))
}
#[inline]
unsafe fn rd_u32_chk(p: *const u8) -> Option<u32> {
    crate::memoria::is_readable(p as *const c_void, 4).then(|| rd_u32(p))
}

pub struct Registry {
    pub(crate) reg: *mut c_void,
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

    /// `GetClass` pelo hash do `CName` que o script passou (sem voltar pra texto).
    pub unsafe fn class_by_hash(&self, hash: u64) -> *mut c_void {
        (self.get_class)(self.reg, hash)
    }

    pub unsafe fn enum_by_name(&self, name: &str) -> *mut c_void {
        (self.get_enum)(self.reg, cname(name))
    }

    /// `IRTTISystem::GetType(CName)` (vtbl+0x00, 2 slots antes do `GetClass`@0x10): qualquer tipo, fundamental incluso (`CName`, `Float`). Nulo sem
    /// tipo ou quando o nome do tipo devolvido não é o pedido — conferido pelo `GetName` da vtable, que vale também pra tipo fundamental.
    pub unsafe fn type_by_name(&self, name: &str) -> *mut c_void {
        let gt = rd_ptr(self.vtbl);
        if gt.is_null() {
            return std::ptr::null_mut();
        }
        let get_type: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(gt);
        let ty = get_type(self.reg, cname(name));
        if ty.is_null() || type_name_getname(ty) != cname(name) {
            return std::ptr::null_mut();
        }
        ty
    }

    /// `IRTTISystem::GetClassByScriptName(CName) -> CClass*` (vtbl+0x108, item
    /// #82) — mapeamento NOME-SCRIPT→classe, distinto de `class_by_name` (que resolve
    /// `GetClass`@0x10, NOME-NATIVO). Em conteúdo vanilla os 2 nomes quase sempre coincidem
    /// (nenhum `RegisterScriptName` remapeando é usado pelo BWMS hoje), então `class_by_name`
    /// já cobre o caso comum — este método existe pro caso raro de um mod usar
    /// `RegisterScriptName` pra apelidar uma classe nativa com nome-script diferente.
    pub unsafe fn class_by_script_name(&self, name: &str) -> *mut c_void {
        self.class_by_script_hash(cname(name))
    }

    /// `class_by_script_name` pelo hash do `CName` que o script passou.
    pub unsafe fn class_by_script_hash(&self, hash: u64) -> *mut c_void {
        let f = self.vtbl_slot(0x108);
        if f.is_null() {
            return std::ptr::null_mut();
        }
        let call: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(f);
        call(self.reg, hash)
    }

    /// Classe pelo nome que o script escreveu: o nativo primeiro, o apelido de script só quando o nativo não existe.
    /// Nulo quando nenhum dos dois resolve.
    pub unsafe fn script_class(&self, hash: u64) -> *mut c_void {
        // CName vazio: o motor não tem classe com esse nome, e não vale a chamada.
        if hash == 0 {
            return std::ptr::null_mut();
        }
        let nativa = self.class_by_hash(hash);
        if !nativa.is_null() {
            return nativa;
        }
        self.class_by_script_hash(hash)
    }

    /// `IRTTISystem::GetEnumByScriptName(CName) -> CEnum*` (vtbl+0x110, item #83). Ver nota de
    /// `class_by_script_name` — mesmo raciocínio, versão pra enum.
    pub unsafe fn enum_by_script_name(&self, name: &str) -> *mut c_void {
        let f = self.vtbl_slot(0x110);
        if f.is_null() {
            return std::ptr::null_mut();
        }
        let call: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(f);
        call(self.reg, cname(name))
    }

    /// `IRTTISystem::ConvertNativeToScriptName(CName) -> CName` (vtbl+0x118, item #84) — devolve
    /// o CName SCRIPT correspondente a um CName NATIVO (`Some(hash)` mesmo se igual ao nativo —
    /// o caso comum; resolve pra nome de exibição via `crate::cname::resolve_cname`).
    pub unsafe fn convert_native_to_script_name(&self, native_hash: u64) -> u64 {
        let f = self.vtbl_slot(0x118);
        if f.is_null() {
            return 0;
        }
        let call: extern "C" fn(*mut c_void, u64) -> u64 = std::mem::transmute(f);
        call(self.reg, native_hash)
    }

    /// `IRTTISystem::ConvertScriptToNativeName(CName) -> CName` (vtbl+0x120, item #85) — inverso
    /// de `convert_native_to_script_name`.
    pub unsafe fn convert_script_to_native_name(&self, script_hash: u64) -> u64 {
        let f = self.vtbl_slot(0x120);
        if f.is_null() {
            return 0;
        }
        let call: extern "C" fn(*mut c_void, u64) -> u64 = std::mem::transmute(f);
        call(self.reg, script_hash)
    }

    /// `IRTTISystem::RegisterScriptName(CName aNativeName, CName aScriptedName)` (vtbl+0x100,
    /// item #81) — registra um apelido SCRIPT pra uma classe/enum/etc. já registrada por nome
    /// NATIVO. `void`, sem valor de retorno pra confirmar sucesso — a única forma de verificar é
    /// chamar `class_by_script_name`/`convert_native_to_script_name` depois e comparar.
    pub unsafe fn register_script_name(&self, native_name: &str, script_name: &str) -> bool {
        let f = self.vtbl_slot(0x100);
        if f.is_null() {
            return false;
        }
        let call: extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(f);
        call(self.reg, cname(native_name), cname(script_name));
        true
    }

    /// O CRTTISystem* cru (o `this` das chamadas de vtable).
    pub unsafe fn raw(&self) -> *mut c_void {
        self.reg
    }

    /// Ponteiro do slot `off` da vtable do CRTTISystem (ex.: +0x30 GetFunction,
    /// +0x80 RegisterType, +0xA0 RegisterFunction). Usado pelo registro nativo.
    pub unsafe fn vtbl_slot(&self, off: usize) -> *mut c_void {
        if self.vtbl.is_null() || !crate::memoria::is_readable(self.vtbl as *const c_void, off + 8) {
            return std::ptr::null_mut();
        }
        rd_ptr(self.vtbl.add(off))
    }

    // ===== Cluster de introspecção RTTI em massa (`IRTTISystem`, offsets 0x40-0x78,
    // `PENDENCIAS-UNIFICADAS.md` itens #57/#58/#60/#61/#62/#63/#64). Todo método aqui devolve
    // `void Foo(DynArray<T*>& out, ...)` — o payload de saída é o MESMO DynArray de 16 bytes
    // (entries:u64@0, size:u32@8, cap:u32@12) já lido por `dispatch_scriptable_tweaks` (vtbl+0x40,
    // "27718 tipos" confirmado ao vivo) — mecanismo genérico reaproveitado, mesma
    // confiança.

    /// Núcleo comum: chama `void Foo(DynArray<T*>&)` (sem args extras) e devolve (entries_ptr, count).
    /// Item #63 (2026-08-11, disassembly ARM64 real): layout do `DynArray` de saída é
    /// `{entries:u64@0x00, capacity:u32@0x08, size:u32@0x0C}` — `count` PRECISA vir de `size`
    /// (buf[12..16]), não `capacity` (buf[8..12], usado incorretamente antes). Pra funções sem
    /// reserve antecipado (a maioria) `capacity` fica PRÓXIMO de `size` por growth amortizado
    /// (~1.5x), mascarando o bug até `GetClasses`/`GetNativeTypes` (reserve antecipado pro
    /// universo inteiro) expuserem a divergência real. Ver `probe_getclasses_capacity_vs_size`.
    unsafe fn call_dynarray_out0(&self, off: usize) -> (*const c_void, usize) {
        let f = self.vtbl_slot(off);
        if f.is_null() {
            return (std::ptr::null(), 0);
        }
        let mut buf = [0u8; 16];
        let call: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(f);
        call(self.reg, buf.as_mut_ptr() as *mut c_void);
        let entries = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0u8; 8])) as *const c_void;
        let count = u32::from_le_bytes(buf[12..16].try_into().unwrap_or([0u8; 4])) as usize;
        (entries, count)
    }

    /// Núcleo comum: chama `void Foo(DynArray<T*>&, bool)` e devolve (entries_ptr, count). Mesmo
    /// fix de `size` vs `capacity` de `call_dynarray_out0` (ver doc-comment acima).
    unsafe fn call_dynarray_out1_bool(&self, off: usize, flag: bool) -> (*const c_void, usize) {
        let f = self.vtbl_slot(off);
        if f.is_null() {
            return (std::ptr::null(), 0);
        }
        let mut buf = [0u8; 16];
        let call: extern "C" fn(*mut c_void, *mut c_void, u8) = std::mem::transmute(f);
        call(self.reg, buf.as_mut_ptr() as *mut c_void, flag as u8);
        let entries = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0u8; 8])) as *const c_void;
        let count = u32::from_le_bytes(buf[12..16].try_into().unwrap_or([0u8; 4])) as usize;
        (entries, count)
    }

    /// `GetNativeTypes(DynArray<IType*>&)` (vtbl+0x40, item #57) — formalizado como método geral
    /// (já usado embutido em `dispatch_scriptable_tweaks` sob o nome informal "GetAllTypes", nunca
    /// exposto como capacidade própria reutilizável até agora).
    pub unsafe fn get_native_types(&self) -> (*const c_void, usize) {
        self.call_dynarray_out0(0x40)
    }

    /// `GetGlobalFunctions(DynArray<CBaseFunction*>&)` (vtbl+0x48, item #58).
    pub unsafe fn get_global_functions(&self) -> (*const c_void, usize) {
        self.call_dynarray_out0(0x48)
    }

    /// `GetClassFunctions(DynArray<CBaseFunction*>&)` (vtbl+0x58, item #60).
    pub unsafe fn get_class_functions(&self) -> (*const c_void, usize) {
        self.call_dynarray_out0(0x58)
    }

    /// `GetEnums(DynArray<CEnum*>&, bool aScriptedOnly)` (vtbl+0x60, item #61).
    pub unsafe fn get_enums(&self, scripted_only: bool) -> (*const c_void, usize) {
        self.call_dynarray_out1_bool(0x60, scripted_only)
    }

    /// `GetBitfields(DynArray<CBitfield*>&, bool aScriptedOnly)` (vtbl+0x68, item #62).
    pub unsafe fn get_bitfields(&self, scripted_only: bool) -> (*const c_void, usize) {
        self.call_dynarray_out1_bool(0x68, scripted_only)
    }

    /// `GetClasses(CClass* aIsAClass, DynArray<CClass*>&, bool(*aFilter)(CClass*)=null, bool
    /// aIncludeAbstract=false)` (vtbl+0x70, item #63) — enumeração de classes por ANCESTRALIDADE,
    /// o mecanismo OFICIAL que `TweakExecutor::ExecuteTweaks()` real usa (`m_rtti->GetClasses(
    /// s_scriptableTweakType, tweakClasses)`) — mais direto que o walk manual de cadeia de pais
    /// já usado em `dispatch_scriptable_tweaks`. `aFilter` sempre null (não usamos callback C++).
    pub unsafe fn get_classes(&self, is_a_class: *mut c_void, include_abstract: bool) -> (*const c_void, usize) {
        let f = self.vtbl_slot(0x70);
        if f.is_null() || is_a_class.is_null() {
            return (std::ptr::null(), 0);
        }
        let mut buf = [0u8; 16];
        let call: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_void, u8) =
            std::mem::transmute(f);
        call(self.reg, is_a_class, buf.as_mut_ptr() as *mut c_void, std::ptr::null(), include_abstract as u8);
        let entries = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0u8; 8])) as *const c_void;
        let count = u32::from_le_bytes(buf[12..16].try_into().unwrap_or([0u8; 4])) as usize;
        (entries, count)
    }

    /// `GetDerivedClasses(CClass* aBaseClass, DynArray<CClass*>&)` (vtbl+0x78, item #64) —
    /// mesma família de `get_classes`, mas só 1 nível de herança DIRETA (não transitiva).
    pub unsafe fn get_derived_classes(&self, base_class: *mut c_void) -> (*const c_void, usize) {
        let f = self.vtbl_slot(0x78);
        if f.is_null() || base_class.is_null() {
            return (std::ptr::null(), 0);
        }
        let mut buf = [0u8; 16];
        let call: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(f);
        call(self.reg, base_class, buf.as_mut_ptr() as *mut c_void);
        let entries = u64::from_le_bytes(buf[0..8].try_into().unwrap_or([0u8; 8])) as *const c_void;
        let count = u32::from_le_bytes(buf[12..16].try_into().unwrap_or([0u8; 4])) as usize;
        (entries, count)
    }

    /// `IRTTISystem::UnregisterType(rtti::IType* aType)` (vtbl+0x98, item #68) — desregistra um
    /// tipo (classe/enum/bitfield) do RTTI. `void`, sem confirmação de sucesso própria — a única
    /// forma de verificar é resolver o tipo de novo depois e confirmar que sumiu.
    pub unsafe fn unregister_type(&self, ty: *mut c_void) -> bool {
        let f = self.vtbl_slot(0x98);
        if f.is_null() || ty.is_null() {
            return false;
        }
        let call: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(f);
        call(self.reg, ty);
        true
    }

    /// `IRTTISystem::GetBitfield(CName) -> CBitfield*` (vtbl+0x20) — irmão de `class_by_name`/
    /// `enum_by_name` (0x10/0x18), nunca exposto antes (`PENDENCIAS-UNIFICADAS.md`, item implícito
    /// do cluster `CBitfield` #117-128). Usado pra verificar `create_scripted_bitfield`.
    pub unsafe fn bitfield_by_name(&self, name: &str) -> *mut c_void {
        let f = self.vtbl_slot(0x20);
        if f.is_null() {
            return std::ptr::null_mut();
        }
        let call: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(f);
        call(self.reg, cname(name))
    }

    /// `IRTTISystem::UnregisterFunction(CGlobalFunction* aFunc)` (vtbl+0xA8, item #70) — irmão de
    /// `UnregisterType`, mesmo raciocínio: `void`, verificação só por re-resolver depois.
    pub unsafe fn unregister_function(&self, func: *mut c_void) -> bool {
        let f = self.vtbl_slot(0xA8);
        if f.is_null() || func.is_null() {
            return false;
        }
        let call: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(f);
        call(self.reg, func);
        true
    }

    /// `IRTTISystem::AddRegisterCallback(Callback<void(*)()>)` (vtbl+0xC0, item #73) /
    /// `AddPostRegisterCallback` (vtbl+0xC8, item #74) — `Callback<void(*)()>` é um
    /// type-erased "std::function" de 0x48 bytes na 2.5 (0x28 na 2.31; buffer padrão 32 → 64):
    /// `{buffer:[u8;64], handler:*CallbackHandler}`, NÃO-trivial (tem
    /// copy/move/destruct) → passado POR REFERÊNCIA INVISÍVEL no Itanium ABI (ponteiro pro objeto
    /// materializado pelo chamador, não os bytes inline). `cb_ptr` = ponteiro pro struct já
    /// montado (`build_void_callback`, abaixo).
    unsafe fn add_callback_at(&self, off: usize, cb_ptr: *mut c_void) -> bool {
        let f = self.vtbl_slot(off);
        if f.is_null() || cb_ptr.is_null() {
            return false;
        }
        let call: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(f);
        call(self.reg, cb_ptr);
        true
    }
    pub unsafe fn add_register_callback(&self, cb_ptr: *mut c_void) -> bool {
        self.add_callback_at(0xC0, cb_ptr)
    }
    pub unsafe fn add_post_register_callback(&self, cb_ptr: *mut c_void) -> bool {
        self.add_callback_at(0xC8, cb_ptr)
    }
}


/// Prova ao vivo do cluster de introspecção em massa (#57/#58/#60/#61/#62/#63/#64) — só CONTA (não
/// dereferencia elementos individuais, suficiente pra provar que o mecanismo funciona e os counts
/// são plausíveis). `GetNativeTypes` é o único com um baseline JÁ conhecido (~27718, de
/// `dispatch_scriptable_tweaks`) — usado como âncora de sanidade pros outros.
pub unsafe fn probe_mass_enumeration(reg: &Registry) {
    let (_, n_types) = reg.get_native_types();
    crate::log_dev!(
        "[rttimass] GetNativeTypes: {n_types} tipos (baseline conhecido ~27718-27719, de dispatch_scriptable_tweaks)"
    );
    let (_, n_gfuncs) = reg.get_global_functions();
    crate::log_dev!("[rttimass] GetGlobalFunctions: {n_gfuncs} funções globais");
    let (_, n_cfuncs) = reg.get_class_functions();
    crate::log_dev!("[rttimass] GetClassFunctions: {n_cfuncs} funções de classe");
    let (_, n_enums) = reg.get_enums(false);
    crate::log_dev!("[rttimass] GetEnums(scriptedOnly=false): {n_enums} enums");
    let (_, n_enums_scripted) = reg.get_enums(true);
    crate::log_dev!("[rttimass] GetEnums(scriptedOnly=true): {n_enums_scripted} enums (deve ser <= {n_enums})");
    let (_, n_bitfields) = reg.get_bitfields(false);
    crate::log_dev!("[rttimass] GetBitfields(scriptedOnly=false): {n_bitfields} bitfields");

    let iscriptable = reg.class_by_name("IScriptable");
    if !iscriptable.is_null() {
        let (_, n_derived) = reg.get_derived_classes(iscriptable);
        crate::log_dev!("[rttimass] GetDerivedClasses(IScriptable): {n_derived} classes (herança DIRETA só)");
        let (_, n_classes_concrete) = reg.get_classes(iscriptable, false);
        let (_, n_classes_all) = reg.get_classes(iscriptable, true);
        crate::log_dev!(
            "[rttimass] GetClasses(IScriptable, includeAbstract=false): {n_classes_concrete} | includeAbstract=true: {n_classes_all} (transitivo, deve ser >= GetDerivedClasses e all>=concrete)"
        );
    } else {
        crate::log_dev!("[rttimass] classe 'IScriptable' não achada — GetClasses/GetDerivedClasses pulados");
    }
}

/// Item #68 (`UnregisterType`, vtbl+0x98, `PENDENCIAS-UNIFICADAS.md`) — prova ao vivo
/// REVERSÍVEL e AUTOCONTIDA: cria um enum de TESTE descartável via `create_scripted_enum` (já
/// provado seguro, 2026-08-05, "zero crash até aqui"), confirma que existe, desregistra, confirma
/// que sumiu. Não toca em NADA vanilla nem em classes forjadas usadas por outro código — o enum de
/// teste nasce e morre inteiramente dentro desta função.
pub unsafe fn probe_unregister_type(reg: &Registry) {
    let name = "BwmsUnregisterTestEnum";
    let created = crate::tipos_do_motor::create_scripted_enum(reg, name, 1, &[("A", 0), ("B", 1)]);
    let before = reg.enum_by_name(name);
    crate::log_dev!(
        "[rttiunreg] CreateScriptedEnum('{name}') call_ok={created} enum_by_name antes={before:p} (esperado: não-null)"
    );
    if before.is_null() {
        crate::log_dev!("[rttiunreg] enum de teste não resolveu — UnregisterType pulado (sem alvo seguro)");
        return;
    }
    let unreg_ok = reg.unregister_type(before);
    let after = reg.enum_by_name(name);
    crate::log_dev!(
        "[rttiunreg] UnregisterType(enum de teste) call_ok={unreg_ok} enum_by_name depois={after:p} desregistrado={}",
        after.is_null()
    );
}


/// Item #70 (`UnregisterFunction`, vtbl+0xA8) — mesmo padrão de `probe_unregister_type`,
/// mas pro lado das globais: registra uma global de teste descartável (`register_global`, já
/// provado por `register_smoke`), confirma que resolve, desregistra, confirma que sumiu.
pub unsafe fn probe_unregister_function(reg: &Registry) {
    let name = "BwmsUnregFuncTest";
    let proto_names = ["Cos", "Sin", "AbsF", "SqrtF", "LogF", "TanF", "AsinF"];
    let mut proto = std::ptr::null_mut();
    for n in proto_names {
        let p = crate::register::get_function(reg, n);
        if sane(p) {
            proto = p;
            break;
        }
    }
    if !sane(proto) {
        crate::log_dev!("[rttiunregfn] nenhum global de protótipo resolveu — probe pulado");
        return;
    }
    unsafe extern "C" fn tramp_unregfn_test(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {}
    let created = crate::register::register_global(reg, proto, name, name, tramp_unregfn_test);
    let before = crate::register::get_function(reg, name);
    crate::log_dev!(
        "[rttiunregfn] register_global('{name}') call_ok={created} get_function antes={before:p} (esperado: não-null)"
    );
    if before.is_null() {
        crate::log_dev!("[rttiunregfn] global de teste não resolveu — UnregisterFunction pulado (sem alvo seguro)");
        return;
    }
    let unreg_ok = reg.unregister_function(before);
    let after = crate::register::get_function(reg, name);
    crate::log_dev!(
        "[rttiunregfn] UnregisterFunction(global de teste) call_ok={unreg_ok} get_function depois={after:p} desregistrada={}",
        after.is_null()
    );
}

// ===== `CBitfield` (12 métodos, `PENDENCIAS-UNIFICADAS.md`: "nunca lido nem escrito — diferente
// de CEnum, que já tem equivalente") — campos diretos, mesmo padrão já usado pra `CEnum`
// (`hashList`/`valueList`): `validBits:u64@+0x28` (máscara de quais dos 64 bits possíveis estão em
// uso) + `bitNames:CName[64]@+0x30`.

/// Enumera os bits NOMEADOS de um `CBitfield*` (índice, nome). Varre `validBits` — só lê
/// `bitNames[i]` pros bits realmente marcados (evita resolver 64 CNames à toa).
#[cfg(feature = "devtools")]
pub unsafe fn dump_bitfield_names(bf: *mut c_void) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let base = bf as *const u8;
    if !crate::memoria::is_readable(base.add(0x28) as *const c_void, 8 + 64 * 8) {
        return out;
    }
    let valid_bits = rd_u64(base.add(0x28));
    for i in 0..64u32 {
        if valid_bits & (1u64 << i) != 0 {
            let hash = rd_u64(base.add(0x30 + i as usize * 8));
            // `name_of` (mirror local + pool nativo), NÃO `resolve_cname` cru — o pool nativo só
            // conhece strings que o PRÓPRIO engine internou; um bitfield forjado por nós usa
            // `crate::cname::intern` (não `cname` cru) exatamente pra isso resolver aqui (achado
            // 2026-08-09: a 1ª versão usava `resolve_cname` direto + filtrava hash==0, escondendo
            // bits reais cujo nome só existe no NOSSO mirror). Sem filtro de hash: `validBits` é o
            // único sinal confiável de "esse índice está em uso" — mostrar até hash=0 (nome "None"
            // de verdade) é mais honesto que esconder.
            let name = crate::cname::name_of(hash).unwrap_or_else(|| format!("(hash={hash:#x}, não resolvido)"));
            out.push((i, name));
        }
    }
    out
}

/// Comando `bitfielddump <nome>` — resolve o bitfield (`GetBitfield`, vtbl+0x20) e lista seus bits
/// nomeados. Testado direto contra `BwmsTestBitfield` (criado por `probe_create_scripted_bitfield`)
/// reforça essa prova: confirma não só que o tipo RESOLVE por nome, mas que os `bitNames`
/// realmente batem com o que foi passado em `CreateScriptedBitfield`.
#[cfg(feature = "devtools")]
pub unsafe fn probe_bitfield_dump(reg: &Registry, name: &str) {
    let bf = reg.bitfield_by_name(name);
    if bf.is_null() {
        crate::log_dev!("[bitfielddump] bitfield '{name}' não resolveu");
        return;
    }
    let bits = dump_bitfield_names(bf);
    crate::log_dev!("[bitfielddump] {name} ({bf:p}): {} bit(s) nomeado(s)", bits.len());
    for (i, n) in &bits {
        crate::log_dev!("[bitfielddump]   [{i}] = '{n}'");
    }
}

/// `Reflection.ReflectionEnum.IsNative()`/`ReflectionBitfield.IsNative()` — `!flags.
/// isScripted`. `CEnum.flags@+0x21`/`CBitfield.flags@+0x21`
/// (ambos 1 byte, bit0=isScripted), leitura trivial, mesma
/// categoria de `class_flags`/`property_flags` já fechados nesta sessão.
#[cfg(feature = "devtools")]
pub unsafe fn type_is_native_scripted(ty: *mut c_void) -> Option<bool> {
    if !crate::memoria::is_readable((ty as *const u8).add(0x21) as *const c_void, 1) {
        return None;
    }
    let b = *(ty as *const u8).add(0x21);
    Some(b & 1 == 0) // isScripted=bit0; IsNative = !isScripted
}

/// Comando `enumisnative <nome>` — `CEnum`/`CBitfield` compartilham offset (ambos herdam
/// `rtti::IType`, layout `name/computedName/actualSize/flags` idêntico até `+0x22`).
#[cfg(feature = "devtools")]
pub unsafe fn probe_type_is_native(reg: &Registry, name: &str) {
    let en = reg.enum_by_name(name);
    let ty = if !en.is_null() { en } else { reg.bitfield_by_name(name) };
    if ty.is_null() {
        crate::log_dev!("[isnative] '{name}' não resolveu (nem enum nem bitfield)");
        return;
    }
    match type_is_native_scripted(ty) {
        Some(native) => crate::log_dev!("[isnative] {name}: is_native={native}"),
        None => crate::log_dev!("[isnative] {name}: flags@+0x21 ilegível"),
    }
}

/// Item #79 (`CreateScriptedBitfield(CName, DynArray<uint64_t>&)`, vtbl+0xF0) — via
/// OFICIAL de criar um bitfield, irmã de `CreateScriptedEnum` (item #78, já usada por
/// `create_scripted_enum`/`probe_unregister_type`). O array de bits não é de `uint64_t`, é de
/// struct com o nome e o bit — mesma forma de
/// `EnumMemberEntry{name:CName, value:i64}` já provada, aqui com o campo renomeado pra `bit`
/// (índice do bit, não um valor arbitrário). Prova: cria bitfield de teste, confirma via
/// `bitfield_by_name` (novo, vtbl+0x20) + delta de contagem via `get_bitfields` + `dump_bitfield_names`
/// (confirma os NOMES reais dos bits, não só que o ponteiro resolve).
#[cfg(feature = "devtools")]
pub unsafe fn probe_create_scripted_bitfield(reg: &Registry) {
    let name = "BwmsTestBitfield2";
    let (_, before_count) = reg.get_bitfields(false);
    // Achado 2026-08-09 (1ª rodada desta prova, `name = "BwmsTestBitfield"`): o campo "bit" NÃO é
    // um índice zero-based — é o VALOR da bitmask (potência de 2). Passar índices (0,1,2) fez
    // `FlagA` (bit=0) sumir (tratado como "sem bit"/inválido) e `FlagB`/`FlagC` caírem 1 posição
    // pra trás (bit=1 -> validBits index 0, bit=2 -> index 1) — 2 de 3 bits, nomes certos mas
    // deslocados. Corrigido: valores de bitmask reais (1,2,4).
    let created = crate::tipos_do_motor::create_scripted_bitfield(reg, name, &[("FlagA", 1), ("FlagB", 2), ("FlagC", 4)]);
    let resolved = reg.bitfield_by_name(name);
    let (_, after_count) = reg.get_bitfields(false);
    crate::log_dev!(
        "[rttibitfield] CreateScriptedBitfield('{name}', 3 bits) call_ok={created} bitfield_by_name={resolved:p} \
         GetBitfields antes={before_count} depois={after_count} (esperado: depois==antes+1 se resolveu)"
    );
    if !resolved.is_null() {
        let bits = dump_bitfield_names(resolved);
        crate::log_dev!(
            "[rttibitfield] dump_bitfield_names: {} bit(s) — esperado 3 (FlagA/FlagB/FlagC)",
            bits.len()
        );
        for (i, n) in &bits {
            crate::log_dev!("[rttibitfield]   [{i}] = '{n}'");
        }
    }
}


/// Item #63 (`GetClasses`, vtbl+0x70) — investigação da anomalia achada em
/// 2026-08-08 (cont.134): com âncora `IScriptable`, devolveu o universo INTEIRO (27718, igual a
/// `GetNativeTypes`) em vez de filtrar por ancestralidade, contradizendo `GetDerivedClasses`
/// (mesma âncora, 1066, plausível). Testa com âncoras MENORES (classe folha sem subclasse
/// conhecida) pra ver se o comportamento muda — se `GetClasses` também devolver o universo
/// inteiro pra uma âncora restrita, confirma que o parâmetro `aIsAClass` está sendo ignorado (não
/// é specific-to-IScriptable); se filtrar corretamente aqui, a anomalia é específica de
/// IScriptable (raiz da hierarquia inteira, caso extremo).
pub unsafe fn probe_getclasses_anomaly(reg: &Registry) {
    for name in ["gameGodModeSystem", "CallbackSystem", "ScriptableSystem"] {
        let anchor = reg.class_by_name(name);
        if anchor.is_null() {
            crate::log_dev!("[rttiganom] âncora '{name}' não resolveu — pulado");
            continue;
        }
        let (_, n_derived) = reg.get_derived_classes(anchor);
        let (_, n_classes) = reg.get_classes(anchor, false);
        let (_, n_types) = reg.get_native_types();
        crate::log_dev!(
            "[rttiganom] âncora='{name}' GetDerivedClasses={n_derived} GetClasses(includeAbstract=false)={n_classes} \
             (universo GetNativeTypes={n_types}) — anomalia presente se GetClasses==universo apesar da âncora restrita"
        );
    }
}

/// Item #63, continuação (2026-08-11): a anomalia de `GetClasses`(vtbl+0x70) foi
/// reproduzida em 3 âncoras diferentes (sempre devolve o universo inteiro, igual a
/// `GetNativeTypes`) — hipótese nunca testada: será que o SLOT em si (o ENDEREÇO da função,
/// não o comportamento) é literalmente o MESMO que `GetNativeTypes`(0x40)? Este projeto já
/// documentou ICF (Identical Code Folding) do compilador colapsando métodos com corpo idêntico
/// noutra vtable (`TweakDB::FlatValue`, os 26 `GetValueOffset_*` — "todos com corpo IDENTICO...
/// plausivelmente colapsados por ICF"). Diagnóstico 100% READ-ONLY (nunca chama através do
/// ponteiro, só lê o VALOR do slot + os primeiros 8 bytes no destino, mesma disciplina segura
/// já usada em `game_instance_vtable_dump` pro item #472) — decide entre 2 hipóteses: (a) slots
/// 0x40 e 0x70 têm o MESMO endereço (ICF confirmado, explica tudo sem culpa nossa) vs. (b)
/// endereços DIFERENTES (a função é genuinamente distinta, e o bug está na nossa leitura do
/// parâmetro `aIsAClass` ou create um ABI mismatch específico deste método).
pub unsafe fn probe_getclasses_vtable_identity(reg: &Registry) {
    if reg.vtbl.is_null() {
        crate::log_dev!("[rttiganom2] vtbl da Registry é null — abortado");
        return;
    }
    let p40 = reg.vtbl_slot(0x40);
    let p70 = reg.vtbl_slot(0x70);
    let p78 = reg.vtbl_slot(0x78); // GetDerivedClasses, já confirmado correto — controle de sanidade
    let same_40_70 = !p40.is_null() && p40 == p70;
    let same_70_78 = !p70.is_null() && p70 == p78;
    let bytes_at = |p: *mut c_void| -> [u8; 8] {
        if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 8) {
            return [0; 8];
        }
        let mut b = [0u8; 8];
        std::ptr::copy_nonoverlapping(p as *const u8, b.as_mut_ptr(), 8);
        b
    };
    let b40 = bytes_at(p40);
    let b70 = bytes_at(p70);
    let b78 = bytes_at(p78);
    crate::log_dev!(
        "[rttiganom2] slot0x40(GetNativeTypes)={p40:p} bytes={b40:02x?} | slot0x70(GetClasses)={p70:p} bytes={b70:02x?} | \
         slot0x78(GetDerivedClasses,controle)={p78:p} bytes={b78:02x?} | 0x40==0x70={same_40_70} (ICF hipótese) | 0x70==0x78={same_70_78} (sanidade, deveria ser false)"
    );
}

/// Item #63, continuação (2026-08-11, sessão de disassembly ARM64 real): a hipótese
/// ICF já foi refutada (`probe_getclasses_vtable_identity`) — endereços de `GetClasses`(0x70) e
/// `GetNativeTypes`(0x40) são genuinamente diferentes. Disassembly manual (via
/// `rttislot` + disassembly da região) do CORPO INTEIRO dos 3 métodos (`GetNativeTypes`/`GetClasses`/
/// `GetDerivedClasses`) revelou o mecanismo exato:
///
/// - As 3 funções usam a MESMA rotina de push/growth pro `DynArray` de saída (`bl 0x100028674`),
///   com layout `{entries:u64@0x00, capacity:u32@0x08, size:u32@0x0C}` (confirmado
///   inequivocamente: o campo comparado contra "needed" pra decidir growth é @0x08, e o campo que
///   recebe `size+1` após cada push bem-sucedido é @0x0C).
/// - `GetNativeTypes` e `GetClasses` (mas NÃO `GetDerivedClasses`) fazem um RESERVE ANTECIPADO da
///   capacidade pro TAMANHO TOTAL da hashtable `types` (`this+0x48`) **ANTES de qualquer
///   filtro rodar** — over-allocam pro universo inteiro de cara, independente de quantos itens vão
///   de fato passar no filtro.
/// - `GetClasses`, quando `aIsAClass != null` (nosso uso sempre), CHAMA um helper de ancestralidade
///   real (`0x1010c6bf4`, disassemblado e confirmado: `IsKindOf(candidate, target)`, walk do campo
///   `parent@+0x10` até achar `target` ou chegar em null) — o filtro EXISTE e está correto.
/// - **A CAUSA RAIZ:** `rtti.rs` (`get_classes`/`get_derived_classes`/`call_dynarray_out0`/
///   `call_dynarray_out1_bool`) lê o "count" de `buf[8..12]` — que é a **CAPACIDADE**, não o
///   `size` real (`buf[12..16]`). Pra `GetDerivedClasses`/`GetEnums`/`GetBitfields` (sem reserve
///   antecipado, growth incremental ~1.5x por push) capacidade fica PRÓXIMA do size real
///   (por isso essas pareciam "certas" — coincidência de growth amortizado, nunca verificada
///   contra o campo certo). Pra `GetClasses`/`GetNativeTypes` (reserve antecipado pro universo
///   inteiro), a capacidade fica FIXA no total pré-alocado — em `GetNativeTypes` isso não importa
///   (todo tipo é incluído, size acaba == capacity de qualquer jeito), mas em `GetClasses` (que
///   FILTRA por ancestralidade) a capacidade nunca encolhe, ficando sempre = universo, enquanto o
///   `size` real (nunca lido) é o número filtrado de verdade.
///
/// Este probe é 100% ADITIVO — chama os MESMOS 3 métodos pela MESMA convenção já usada em
/// `get_native_types`/`get_classes`/`get_derived_classes` (nenhum código existente é tocado), só
/// lê os DOIS campos (capacity@8 e size@12) em vez de um, pra confirmar a hipótese ao vivo antes
/// de qualquer fix.
pub unsafe fn probe_getclasses_capacity_vs_size(reg: &Registry, anchor_name: &str) {
    let anchor = reg.class_by_name(anchor_name);
    if anchor.is_null() {
        crate::log_dev!("[rttisize] âncora '{anchor_name}' não resolveu — abortado");
        return;
    }
    let read_cap_size = |buf: &[u8; 16]| -> (u32, u32) {
        let cap = u32::from_le_bytes(buf[8..12].try_into().unwrap_or([0; 4]));
        let size = u32::from_le_bytes(buf[12..16].try_into().unwrap_or([0; 4]));
        (cap, size)
    };

    // GetNativeTypes (0x40): void(this, DynArray&) — controle (esperado cap==size, sem filtro).
    let (cap_nt, size_nt) = {
        let f = reg.vtbl_slot(0x40);
        let mut buf = [0u8; 16];
        if !f.is_null() {
            let call: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(f);
            call(reg.reg, buf.as_mut_ptr() as *mut c_void);
        }
        read_cap_size(&buf)
    };

    // GetClasses (0x70): void(this, CClass* aIsAClass, DynArray&, filter, bool) — o alvo real.
    let (cap_gc, size_gc) = {
        let f = reg.vtbl_slot(0x70);
        let mut buf = [0u8; 16];
        if !f.is_null() {
            let call: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *const c_void, u8) =
                std::mem::transmute(f);
            call(reg.reg, anchor, buf.as_mut_ptr() as *mut c_void, std::ptr::null(), 0u8);
        }
        read_cap_size(&buf)
    };

    // GetDerivedClasses (0x78): void(this, CClass* base, DynArray&) — controle (sem reserve
    // antecipado, cap deve ficar PRÓXIMO mas não necessariamente IGUAL ao size real).
    let (cap_gd, size_gd) = {
        let f = reg.vtbl_slot(0x78);
        let mut buf = [0u8; 16];
        if !f.is_null() {
            let call: extern "C" fn(*mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(f);
            call(reg.reg, anchor, buf.as_mut_ptr() as *mut c_void);
        }
        read_cap_size(&buf)
    };

    crate::log_dev!(
        "[rttisize] âncora='{anchor_name}' | GetNativeTypes cap={cap_nt} size={size_nt} (esperado cap==size) | \
         GetClasses cap={cap_gc} size={size_gc} (BUG: BWMS lê cap, deveria ler size) | \
         GetDerivedClasses cap={cap_gd} size={size_gd} (controle, sem reserve antecipado)"
    );
}

/// Item #50 (`Reflection.GetClasses`/`GetEnums`/`GetBitfields`/`GetDerivedClasses`/
/// `GetGlobalFunctions`, "enumeração em massa nunca confirmada") — `GetClasses`/`GetEnums`/`GetBitfields` são só `rtti->types.ForEach`
/// FILTRADO por `aType->GetType()==ERTTIType::X` — ou seja, `get_native_types()` (universo
/// completo, já provado) + `type_kind()` (decoder de categoria, já provado) compõem a MESMA
/// capacidade sem RE nova. O caminho direto de `GetDerivedClasses` seria `rtti->GetClasses(base,...,true)`
/// — o MESMO método com a anomalia documentada nesta sessão (item #63, devolve o
/// universo inteiro) — em vez de replicar o bug, uso `reg.get_derived_classes` (vtbl+0x78, método
/// DIFERENTE e JÁ PROVADO correto) pro mesmo efeito funcional. `GetGlobalFunctions` já é
/// `reg.get_global_functions()` (vtbl+0x48, já provado, "4435 funções globais").
///
/// Conta quantos dos tipos do universo (`get_native_types`) batem uma categoria `ERTTIType`, e
/// lista os primeiros `n` nomes (via `type_name_getname`, já usado em todo o projeto).
#[cfg(feature = "devtools")]
pub unsafe fn count_and_list_by_kind(reg: &Registry, target_kind: u8, n: usize) -> (usize, Vec<String>) {
    let (entries, count) = reg.get_native_types();
    if entries.is_null() || count == 0 {
        return (0, Vec::new());
    }
    let arr = entries as *const *mut c_void;
    let mut total = 0usize;
    let mut names = Vec::new();
    for i in 0..count {
        let Some(ty) = rd_ptr_chk(arr.add(i) as *const u8) else { break };
        if ty.is_null() {
            continue;
        }
        if type_kind(ty) == Some(target_kind) {
            total += 1;
            if names.len() < n {
                let name_hash = type_name_getname(ty);
                names.push(crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})")));
            }
        }
    }
    (total, names)
}

/// Comando `reflenum <categoria> [n]` — `classes`/`enums`/`bitfields` (categorias mais úteis pra
/// mods; `fundamental`/`array`/etc. também aceitos pelo nome cru do `ERTTIType`).
#[cfg(feature = "devtools")]
pub unsafe fn probe_count_and_list_by_kind(reg: &Registry, kind_name: &str, n: usize) {
    let target: u8 = match kind_name {
        "classes" | "Class" => 2,
        "enums" | "Enum" => 5,
        "bitfields" | "BitField" => 13,
        "fundamental" | "Fundamental" => 1,
        "array" | "Array" => 3,
        "handle" | "Handle" => 9,
        "weakhandle" | "WeakHandle" => 10,
        _ => {
            crate::log_dev!(
                "[reflenum] categoria '{kind_name}' desconhecida (use classes/enums/bitfields/fundamental/array/handle/weakhandle)"
            );
            return;
        }
    };
    let (total, names) = count_and_list_by_kind(reg, target, n);
    crate::log_dev!("[reflenum] categoria='{kind_name}' total={total}, primeiros {}: {:?}", names.len(), names);
}

/// Comando `reflfind <trecho> [n]` — varre TODO o mapa de tipos nativos e devolve os nomes que
/// contêm `trecho` (case-insensitive), com o kind de cada um.
///
/// POR QUE ISTO EXISTE. Vários itens de "forja de tipo" do catálogo estão parados na MESMA
/// pergunta, e ela nunca foi respondida diretamente: *o tipo que o framework do Windows chama de
/// `X` existe no RTTI deste build, e com que nome?* Até agora a resposta vinha de adivinhar um
/// nome e chamar `GetType` — que devolve NULL tanto pra "não existe" quanto pra "existe com outro
/// nome", dois casos MUITO diferentes que o teste não separava. O caso concreto que motivou:
/// `CurveDataFloat` (`#21`) não existe em lugar nenhum, mas o binário tem
/// `GetTypeObject<TSingleChannelCurve<Vector4>>()::rttiType` — ou seja, o tipo existe, o NOME é
/// que é outro.
///
/// Uma linha só de log de propósito: `log()` descarta 7 de 8 linhas sob rate-limit, então um
/// resultado espalhado em N linhas volta furado.
#[cfg(feature = "devtools")]
pub unsafe fn probe_find_types(reg: &Registry, needle: &str, n: usize) {
    let (entries, count) = reg.get_native_types();
    if entries.is_null() || count == 0 {
        crate::log_dev!("[reflfind] mapa de tipos vazio/indisponível");
        return;
    }
    let alvo = needle.to_ascii_lowercase();
    let arr = entries as *const *mut c_void;
    let mut achados: Vec<String> = Vec::new();
    let mut total = 0usize;
    let mut varridos = 0usize;
    for i in 0..count {
        let Some(ty) = rd_ptr_chk(arr.add(i) as *const u8) else { break };
        if ty.is_null() {
            continue;
        }
        varridos += 1;
        let h = type_name_getname(ty);
        let Some(nome) = crate::cname::name_of(h) else { continue };
        if !nome.to_ascii_lowercase().contains(&alvo) {
            continue;
        }
        total += 1;
        if achados.len() < n {
            let k = type_kind(ty).map(|k| decode_ertti_type(k).to_string()).unwrap_or_else(|| "?".into());
            achados.push(format!("{nome}[{k}]"));
        }
    }
    crate::log_dev!(
        "[reflfind] trecho='{needle}' varridos={varridos}/{count} casaram={total} -> {achados:?}"
    );
}

/// Descobre, POR MEDIÇÃO, em que offset do `CProperty` mora o ponteiro pro tipo da property.
///
/// # Por que medir em vez de ler o header
///
/// A regra do projeto desde o `#30` é perguntar o offset ao RTTI, não ao header do SDK — e aqui
/// há um motivo extra: `dump_props` já convive com a incerteza varrendo qwords e testando cada um
/// contra o dicionário de CName, exatamente porque o layout do `CProperty` nunca foi fixado neste
/// build. Chutar `+0x08` daria um número plausível (a armadilha documentada em
/// `cp77-offset-lixo-plausivel`), não uma resposta.
///
/// # O critério
///
/// O conjunto dos 27882 ponteiros de tipo nativo é conhecido. Então: pra cada property de algumas
/// classes, leia cada qword dos primeiros 0x30 bytes e conte quantas vezes o valor **é um desses
/// ponteiros**.
///
/// # O crivo que faltava (medido em 2026-09-07, denunciado pelo controle positivo)
///
/// A 1ª versão parava aí, e a medição real foi `acertos por qword: [94, 0, 0, 94, 0, 0]` —
/// **EMPATE** entre `+0x00` e `+0x18`. O código pegou o último e ficou com o campo errado:
/// `reflholders Vector4` devolveu 12 casamentos, todos da forma `Vector4.?:Vector4`, classe e tipo
/// iguais — assinatura de estar lendo o **`parent`** (o CClass dono), não o `type`. Os dois campos
/// são SEMPRE ponteiros de tipo válidos, então "está no conjunto" não os separa.
///
/// O que separa: **`parent` é igual à classe que está sendo varrida; `type` em geral não é.** Então
/// um 2º contador registra quantas vezes o valor bate com a própria classe, e o offset vencedor é o
/// que acerta muito no 1º critério e POUCO no 2º. Sem isso, `curveData casou 0` teria sido lido
/// como resposta do `#21` quando era a ferramenta medindo o campo errado.
#[cfg(feature = "devtools")]
unsafe fn calibrar_offset_do_tipo(reg: &Registry, tipos: &std::collections::HashSet<u64>) -> Option<usize> {
    // Classes de amostra: escolhidas por terem muitas properties e serem de famílias diferentes
    // (UI, entidade, componente), pra que o offset vencedor não seja um artefato de uma família.
    const AMOSTRA: [&str; 4] = ["inkTextWidget", "gameObject", "entMeshComponent", "inkWidget"];
    let mut acertos = [0usize; 6];
    let mut ehodono = [0usize; 6];
    let mut olhadas = 0usize;
    for cn in AMOSTRA {
        let cls = reg.class_by_name(cn);
        if cls.is_null() {
            continue;
        }
        for (pp, _) in props_da_classe(cls) {
            olhadas += 1;
            for j in 0..6usize {
                let v = rd_u64((pp as *const u8).add(j * 8));
                if v == 0 {
                    continue;
                }
                if tipos.contains(&v) {
                    acertos[j] += 1;
                }
                if v == cls as u64 {
                    ehodono[j] += 1;
                }
            }
        }
    }
    if olhadas == 0 {
        return None;
    }
    // Candidato = acerta na MAIORIA das properties (piso contra "o maior de seis zeros") E NÃO é o
    // ponteiro da própria classe na maioria delas (o crivo que separa `type` de `parent`).
    let escolhido = (0..6usize)
        .filter(|&j| acertos[j] * 2 > olhadas && ehodono[j] * 2 <= olhadas)
        .max_by_key(|&j| acertos[j]);
    match escolhido {
        Some(j) => {
            crate::log_dev!(
                "[reflholders] offset do tipo CALIBRADO = +{:#x} ({}/{olhadas}) | e_ponteiro_de_tipo: {acertos:?} | e_a_propria_classe: {ehodono:?}",
                j * 8,
                acertos[j]
            );
            Some(j * 8)
        }
        None => {
            crate::log_dev!(
                "[reflholders] calibração FALHOU em {olhadas} properties — nenhum offset passou nos 2 crivos | e_ponteiro_de_tipo: {acertos:?} | e_a_propria_classe: {ehodono:?}"
            );
            None
        }
    }
}

/// As properties de uma classe: `(ponteiro do CProperty, índice)`.
///
/// Reusa o mesmo crivo do `dump_props` — varre os pares (ptr,count) em 0x18..0x70 e aceita o
/// primeiro array cujos entries são legíveis. Toda leitura é checada pela `memoria`.
#[cfg(feature = "devtools")]
unsafe fn props_da_classe(cls: *mut c_void) -> Vec<(*mut c_void, usize)> {
    let clsb = cls as *const u8;
    for off in (0x18usize..=0x70).step_by(8) {
        let Some(arr) = rd_ptr_chk(clsb.add(off)) else { continue };
        let Some(cnt) = rd_u32_chk(clsb.add(off + 0x0C)) else { continue };
        if arr.is_null() || cnt == 0 || cnt > 5000 {
            continue;
        }
        let arr = arr as *const u8;
        let mut out = Vec::new();
        for i in 0..cnt as usize {
            let Some(pp) = rd_ptr_chk(arr.add(i * 8)) else { break };
            if pp.is_null() || !crate::memoria::is_readable(pp, 0x30) {
                continue;
            }
            out.push((pp, i));
        }
        // Um array só conta se a MAIORIA dos entries virou ponteiro legível — senão é lixo que
        // por acaso tinha um count pequeno.
        if out.len() * 2 > cnt as usize {
            return out;
        }
    }
    Vec::new()
}

/// Comando `reflholders <trecho> [n]` — QUE CLASSES têm uma property cujo TIPO casa `trecho`.
///
/// # O bloqueio que isto resolve
///
/// Vários itens do catálogo estão parados na mesma pergunta, e não é mais "o tipo existe?" —
/// `reflfind` já respondeu isso. É a seguinte: **como obter uma INSTÂNCIA de um tipo que nenhum
/// campo de script expõe?** O `#21` (`curveData:*`) é o caso puro: os 5 tipos estão vivos no RTTI,
/// declarar é bloqueio medido (categoria `CMesh`), e a rota de composição precisa de um objeto
/// real pra apontar. Se alguma classe nativa tem uma property daquele tipo, então qualquer
/// instância dela é o caminho — e essa é uma pergunta que o RTTI responde sozinho.
///
/// Só leituras. Nunca chama método virtual em ponteiro achado por varredura (`is_readable` protege
/// LEITURA, não CHAMADA — lição do `depot_resources_by_class`).
#[cfg(feature = "devtools")]
pub unsafe fn probe_find_prop_holders(reg: &Registry, needle: &str, n: usize) {
    let (entries, count) = reg.get_native_types();
    if entries.is_null() || count == 0 {
        crate::log_dev!("[reflholders] mapa de tipos vazio/indisponível");
        return;
    }
    let arr = entries as *const *mut c_void;

    // 1) o conjunto de TODOS os ponteiros de tipo, e o nome de cada um.
    let mut tipos: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut nome_do_tipo: std::collections::HashMap<u64, String> = std::collections::HashMap::new();
    let mut classes: Vec<(*mut c_void, String)> = Vec::new();
    for i in 0..count {
        let Some(ty) = rd_ptr_chk(arr.add(i) as *const u8) else { break };
        if ty.is_null() {
            continue;
        }
        tipos.insert(ty as u64);
        let h = type_name_getname(ty);
        if let Some(nm) = crate::cname::name_of(h) {
            if type_kind(ty) == Some(ertti::CLASS) {
                classes.push((ty, nm.clone()));
            }
            nome_do_tipo.insert(ty as u64, nm);
        }
    }

    // 2) o offset do campo `type`, medido.
    let Some(off_tipo) = calibrar_offset_do_tipo(reg, &tipos) else { return };

    // 3) a varredura de verdade.
    let alvo = needle.to_ascii_lowercase();
    let mut achados: Vec<String> = Vec::new();
    let mut total = 0usize;
    for (cls, nome_cls) in &classes {
        for (pp, _) in props_da_classe(*cls) {
            let tv = rd_u64((pp as *const u8).add(off_tipo));
            if tv == 0 {
                continue;
            }
            let Some(tn) = nome_do_tipo.get(&tv) else { continue };
            if !tn.to_ascii_lowercase().contains(&alvo) {
                continue;
            }
            total += 1;
            if achados.len() < n {
                // O nome da property: o mesmo crivo do `dump_props` — o qword que o dicionário
                // de CName reverte pra texto.
                let mut pn = String::from("?");
                for j in 0..6usize {
                    let q = rd_u64((pp as *const u8).add(j * 8));
                    if q == 0 || q == tv {
                        continue;
                    }
                    if let Some(x) = crate::cname::name_of(q) {
                        if x.len() >= 2 && x.len() <= 64 {
                            pn = x;
                            break;
                        }
                    }
                }
                achados.push(format!("{nome_cls}.{pn}:{tn}"));
            }
        }
    }
    crate::log_dev!(
        "[reflholders] trecho='{needle}' classes={} casaram={total} -> {achados:?}",
        classes.len()
    );
}

/// Comando `curvelayout <classe> <property>` — MEDE o layout interno de um `curveData:*`.
///
/// # O bloqueio que isto ataca
///
/// O `#21` pede ~50 chamadas sobre `LegacySingleChannelCurve` (`Get/SetInterpolationType`,
/// `Get/SetLinkType`, `Get/SetSize`, `GetPoint`, `GetPointValue`, `SetPoint`, `SetPointValue` × 5
/// tipos). `reflholders` já resolveu *onde achar uma instância* — 215 classes têm property desse
/// tipo. Falta o layout interno, e a regra do projeto desde o `#30` é MEDIR, nunca ler do header:
/// um offset errado não estoura, devolve um número plausível (`cp77-offset-lixo-plausivel`).
///
/// # NÃO CONSTRÓI NADA — e essa é a correção que este comando já custou
///
/// A 1ª versão instanciava a classe portadora com `new_object` pra ler o campo no objeto vivo.
/// Medido em 2026-09-07: `curvelayout ExposureAreaSettings exposureMin` **travou a thread do
/// console** — o jogo seguiu a 112% de CPU, com os ticks fluindo, mas o canal parou de consumir
/// comandos e não voltou. `new_object` chama `Construct()` na vtable, e uma classe de *settings*
/// do motor não é construível fora do contexto dela.
///
/// A lição generaliza: **um comando de canal nunca deve chamar `Construct()` numa classe
/// arbitrária vinda de argumento.** O sintoma não é crash (que aparece no `.ips` e se investiga) —
/// é congelamento MUDO do canal, que parece "o comando não existe".
///
/// # O que mede agora, só com leitura + as viruais do PRÓPRIO objeto de tipo
///
/// `GetSize`/`GetAlignment` da vtable do objeto de TIPO (offsets `+0x18`/`+0x20`, confirmados no
/// macOS em 2026-06-20). Chamar virtual num
/// objeto de tipo do RTTI é o caso seguro — é um objeto que o motor mantém vivo e polimórfico,
/// não um ponteiro colhido em varredura (a distinção de `cp77-is-readable-nao-protege-chamada`).
///
/// O tamanho já é discriminante: uma curva é logicamente `{array de pontos, tipo de interpolação,
/// tipo de link}`, e o `DynArray` deste build ocupa 16 bytes (`entries@00, capacity@08, size@0C`).
/// Então 0x18 = DynArray + 2 enums de 4 bytes, e qualquer outro valor diz que há mais campo.
#[cfg(feature = "devtools")]
pub unsafe fn probe_curve_layout(reg: &Registry, classe: &str, prop_nome: &str) {
    let cls = reg.class_by_name(classe);
    if cls.is_null() {
        crate::veredito_dev!("[curvelayout] classe '{classe}' nao resolveu no RTTI");
        return;
    }
    // O offset da property na classe portadora — leitura pura do RTTI, sem instanciar nada.
    let Some((dono, off, tipo)) = prop_com_tipo(cls, prop_nome) else {
        crate::veredito_dev!(
            "[curvelayout] '{classe}' nao tem property '{prop_nome}' (ou o RTTI nao a reporta)"
        );
        return;
    };
    let nome_tipo = crate::cname::name_of(type_name_getname(tipo)).unwrap_or_else(|| "?".into());
    let (tam, alin) = tamanho_e_alinhamento(tipo);
    // 16 = o DynArray deste build (entries@00, capacity@08, size@0C). O resto do tamanho é o que
    // sobra pros escalares (tipo de interpolação, tipo de link).
    let sobra = tam.saturating_sub(16);
    crate::veredito_dev!(
        "[curvelayout] {dono}::{prop_nome} @ {off:#x} tipo='{nome_tipo}' kind={} | \
size={tam} align={alin} | se DynArray(16)+escalares, sobram {sobra} bytes",
        type_kind(tipo).map(|k| decode_ertti_type(k).to_string()).unwrap_or_else(|| "?".into())
    );
}

/// `GetSize`/`GetAlignment` de um objeto de TIPO do RTTI, pelas virtuais dele.
///
/// Offsets `+0x18`/`+0x20` na vtable, confirmados no macOS em 2026-06-20 pelo rttidump da vtable
/// do `Vector4`. São `+0x08` vs o Windows porque
/// o Itanium ABI do macOS põe DOIS slots de destrutor no topo.
#[cfg(feature = "devtools")]
fn tamanho_e_alinhamento(ty: *mut c_void) -> (u32, u32) {
    unsafe {
        let vtbl = rd_ptr(ty as *const u8) as *const u8;
        if vtbl.is_null() {
            return (0, 0);
        }
        let gs = rd_ptr(vtbl.add(0x18));
        let ga = rd_ptr(vtbl.add(0x20));
        let tam = if sane(gs) {
            let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(gs);
            f(ty)
        } else {
            0
        };
        let alin = if sane(ga) {
            let f: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(ga);
            f(ty)
        } else {
            0
        };
        (tam, alin)
    }
}

/// `(classe dona, offset, ponteiro do tipo)` de uma property, subindo a herança. Só leituras.
#[cfg(feature = "devtools")]
unsafe fn prop_com_tipo(cls: *mut c_void, nome: &str) -> Option<(String, usize, *mut c_void)> {
    let alvo = crate::cname::cname(nome);
    let mut c = cls;
    let mut guarda = 0;
    while !c.is_null() && guarda < 32 {
        guarda += 1;
        let nome_cls = crate::cname::name_of(type_name_getname(c)).unwrap_or_else(|| "?".into());
        for (pp, _) in props_da_classe(c) {
            for j in 0..6usize {
                if rd_u64((pp as *const u8).add(j * 8)) == alvo {
                    // `type` mora em +0x00 — MEDIDO em 2026-09-07 (o +0x18 é o `parent`, e os dois
                    // empatam no critério ingênuo "é um ponteiro de tipo"; ver `probe_find_prop_holders`).
                    let tipo = rd_ptr(pp as *const u8);
                    return Some((nome_cls, prop_value_offset(pp) as usize, tipo));
                }
            }
        }
        c = rd_ptr((c as *const u8).add(0x10));
    }
    None
}

/// Comando `reflderived <classe base>` — `Reflection.GetDerivedClasses` real, via
/// `get_derived_classes` (JÁ PROVADO, evita a anomalia de `GetClasses`).
#[cfg(feature = "devtools")]
pub unsafe fn probe_reflect_derived(reg: &Registry, base: &str) {
    let cls = reg.class_by_name(base);
    if cls.is_null() {
        crate::log_dev!("[reflderived] classe base '{base}' não resolveu");
        return;
    }
    let (entries, count) = reg.get_derived_classes(cls);
    let mut names = Vec::new();
    if !entries.is_null() {
        let arr = entries as *const *mut c_void;
        for i in 0..count.min(20) {
            if let Some(d) = rd_ptr_chk(arr.add(i) as *const u8) {
                if !d.is_null() {
                    let h = type_name_getname(d);
                    names.push(crate::cname::name_of(h).unwrap_or_else(|| format!("(hash={h:#x})")));
                }
            }
        }
    }
    crate::log_dev!("[reflderived] {base}: {count} classe(s) derivada(s) direta(s), primeiras {}: {:?}", names.len(), names);
}

/// Aloca `size` bytes alinhados no POOL DO RED (PoolDefault), então o `Free` da engine
/// casa. NÃO zera (faça você). Reutilizável
/// pelo registro nativo (construir CGlobalFunction/CClassFunction à mão).
pub unsafe fn pool_alloc(size: usize, align: usize) -> *mut c_void {
    let alloc: extern "C" fn(u64, u32) -> *mut c_void =
        std::mem::transmute(crate::rebase(ADDR_POOL_DEFAULT_ALLOC_ALIGNED));
    alloc(size as u64, align.max(8) as u32)
}

/// Valor de um membro de enum por NOME (ex.: gamedataDevelopmentPointType::Attribute).
pub unsafe fn resolve_enum_value(reg: &Registry, enum_type: &str, member: &str) -> Option<u64> {
    enum_valor(reg.enum_by_name(enum_type) as *const u8, cname(member))
}

/// Valor do membro `nome` (hash do `CName`) de um `CEnum`: `hashList` em `+0x28` (tamanho em `+0x34`) e `valueList` em `+0x38`, paralelas. O
/// `+0x30` é a capacidade — igual ao tamanho só enquanto o enum nunca cresceu; depois de uma constante nova lê além do fim.
pub unsafe fn enum_valor(en: *const u8, nome: u64) -> Option<u64> {
    if en.is_null() || !crate::memoria::is_readable(en.add(0x28) as *const c_void, 0x18) {
        return None;
    }
    let hp = rd_ptr(en.add(0x28)) as *const u8;
    let n = rd_u32(en.add(0x34));
    let vp = rd_ptr(en.add(0x38)) as *const u8;
    if hp.is_null() || vp.is_null() || n > 100_000 {
        return None;
    }
    (0..n as usize).find(|i| rd_u64(hp.add(i * 8)) == nome).map(|i| rd_u64(vp.add(i * 8)))
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

/// Descritor do retorno da função: `*(func+0x18)` = `CProperty*` (null = void); o `IType*` é `**(func+0x18)` (`ret_type_of`).
/// Devolvido pra inspecionar nome+tamanho
/// (usado pelo Override-suppress p/ marshalar o retorno no aOut com largura correta).
/// checada pela `memoria` — nunca congela numa func torta.
pub unsafe fn fn_ret_type(func: *mut c_void) -> *mut c_void {
    if func.is_null() || !crate::memoria::is_readable(func as *const c_void, 0x20) {
        return std::ptr::null_mut();
    }
    rd_ptr((func as *const u8).add(0x18))
}

/// `ret_type` PRONTO pra `ResolvedFn`/`call_func` (o `IType*` real que `crate::jogo::scripts::EXECUTOR_VM` espera no 5º
/// arg) — MESMA lógica de duplo-dereference que `resolve_func` já usa internamente (`func+0x18` é
/// um ponteiro pro descritor `CProperty*`; o `IType*` de verdade é `**(func+0x18)`, o 1º campo do descritor —
/// `fn_ret_type` acima faz só 1 nível, correto pro USO DELES, mas ERRADO se
/// usado direto como `ret_type` de um `ResolvedFn` construído à mão). Achado 2026-08-03 (/goal):
/// `ResolvedFn{ret_type: null}` construído à mão pra funções NÃO-void (`ref<T>` de retorno) crasha
/// — `crate::jogo::scripts::EXECUTOR_VM` recebe um `ret_type` mentiroso (diz "void" pra uma função que não é) e corrompe
/// o processamento da chamada, não só a marshalling final do valor de retorno.
pub unsafe fn ret_type_of(func: *mut c_void) -> *mut c_void {
    if func.is_null() || !crate::memoria::is_readable(func as *const c_void, 0x20) {
        return std::ptr::null_mut();
    }
    let rp = rd_ptr((func as *const u8).add(0x18));
    if rp.is_null() || !crate::memoria::is_readable(rp as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    rd_ptr(rp as *const u8)
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

/// Chama `rf` e devolve o ponteiro do objeto que ela retorna, sem ficar com referência. Só para objeto que outro dono mantém vivo
/// (player, sistema, container, estado de sistema): o `ref<T>` que o `call_func` devolve traz +1 nosso
/// (`proofs/20260926-refsis-mais-um-por-chamada-PROVADO.log`: 2, 3, 4 → 1), e sem soltar cada chamada prendia o objeto pra sempre.
/// `wref<T>` e retorno que não é handle voltam como vieram.
pub unsafe fn call_ptr(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg]) -> *mut c_void {
    let Some(res) = call_func(rf, ctx, args) else {
        return std::ptr::null_mut();
    };
    soltar_retorno(rf, &res);
    u64::from_le_bytes(res[0..8].try_into().unwrap_or([0; 8])) as *mut c_void
}

/// Solta o +1 do `ref<T>` que o `call_func` de `rf` devolveu em `res`, se o retorno é handle forte: pra quem já leu o valor e não fica
/// com o objeto. Com outro dono o objeto segue vivo; se a chamada o criou, morre aqui, porque ninguém mais o referencia. `ret_type`
/// tem de ser o `IType*` (o que `resolve_in_class` e `ret_type_of` dão); nulo = sem soltura.
pub unsafe fn soltar_retorno(rf: &ResolvedFn, res: &[u8; 0x20]) {
    if type_kind(rf.ret_type) == Some(ertti::HANDLE) {
        let mut h = [u64::from_le_bytes(res[0..8].try_into().unwrap_or([0; 8])), u64::from_le_bytes(res[8..16].try_into().unwrap_or([0; 8]))];
        crate::jogo::objetos::soltar_handle(&mut h);
    }
}

/// Ponteiro plausível (mesma checagem `sane()` dele): fora de null/baixo e abaixo
/// do teto de espaço de usuário.
pub fn sane(p: *mut c_void) -> bool {
    let a = p as usize;
    a > 0x1_0000 && a < 0x8000_0000_0000
}

/// `ISerializable.ProcessPostLoad` (2026-08-12, agente dedicado round 9) —
/// dispatch VIRTUAL GENÉRICO via vtable slot `+0x30`. No Windows, `virtual void PostLoad(const PostLoadParams&)` fica no slot
/// `+0x28` (`GetNativeType@0x00, GetType@0x08, GetAllocator@0x10, ~ISerializable@0x18,
/// sub_20@0x20, PostLoad@0x28`) — aplicando a MESMA convenção dual-dtor Itanium já estabelecida
/// neste projeto (Mac tem 2 dtor slots vs 1 no MSVC, tudo DEPOIS do dtor desloca +0x08), Mac
/// PostLoad = 0x30. **Este offset já está CONFIRMADO, não é palpite novo**: é o EXATO mesmo slot
/// que o `postload-probe` (sessão 2026-07-25) usou pra achar `CMesh::PostLoad`/
/// `MorphTargetMesh::PostLoad` por leitura direta de vtable (`selftest.rs`, comentário: "lê
/// vtable do inst criado por new_object, slot +0x30 = PostLoad") — 2 classes reais já confirmam
/// o slot number independentemente.
///
/// Diferença chave vs. `CMESH_POSTLOAD_VM`/`MORPHTGT_POSTLOAD_VM` (endereços HARDCODED por
/// classe, achados via hook install): aqui o dispatch é GENÉRICO — lê o vtable pointer do
/// PRÓPRIO objeto em runtime (`*obj`) e o slot 0x30 DESSE vtable, então funciona pra QUALQUER
/// classe ISerializable-derivada (incl. qualquer `IScriptable` real, já que `IScriptable`
/// herda de `ISerializable` na hierarquia real e preserva a posição dos slots base), sem
/// precisar de endereço por-classe. Zero RE nova.
///
/// `PostLoadParams{disablePreInitialization: bool, pad[7]}` (8 bytes) é passado
/// POR PONTEIRO (x1), igual ao C++ real (`const PostLoadParams& aParams`) — construído numa
/// array de bytes local, nunca alocado.
///
/// **Efeito real, não leitura pura**: `PostLoad` reprocessa o objeto como se tivesse acabado de
/// carregar do disco (hook-up de referências/recursos) — mutação genuína, não um getter. Por
/// isso o smoke test deste item é deliberadamente OBSERVE-ONLY (testa só os guards de
/// segurança contra ponteiro inválido, nunca chama de verdade num objeto vivo), mesma
/// disciplina já usada pra `SetChunkMask`/`RefreshAppearance`/`ToggleGarageVehicle`.
pub unsafe fn iserializable_process_post_load(obj: *mut c_void, disable_pre_init: bool) -> bool {
    const POSTLOAD_SLOT: usize = 0x30;
    if !sane(obj) {
        return false;
    }
    if !crate::memoria::is_readable(obj as *const c_void, 8) {
        return false;
    }
    let vt = (obj as *const u64).read_unaligned() as *const u8;
    if vt.is_null() || !sane(vt as *mut c_void) || !crate::memoria::is_readable(vt as *const c_void, POSTLOAD_SLOT + 8) {
        return false;
    }
    let slot = (vt.add(POSTLOAD_SLOT) as *const u64).read_unaligned();
    if slot == 0 || !crate::memoria::is_readable(slot as *const c_void, 4) {
        return false;
    }
    // PostLoadParams{bool disablePreInitialization; uint8_t pad[7];} — 8 bytes, passado por x1.
    let params: [u8; 8] = [u8::from(disable_pre_init), 0, 0, 0, 0, 0, 0, 0];
    let f: extern "C" fn(*mut c_void, *const u8) = std::mem::transmute(slot);
    f(obj, params.as_ptr());
    true
}

/// Resolve uma função RED por classe+método, subindo a cadeia de heranças.
pub unsafe fn resolve_func(reg: &Registry, class_name: &str, method: &str) -> Option<ResolvedFn> {
    resolve_in_class(reg.class_by_name(class_name), method)
}

/// Resolve um método a partir do PONTEIRO da classe (CClass*), subindo a cadeia de
/// pais. Varre instância (CClass+0x48) e estáticas (CClass+0x58), casando o CName
/// do método em `func+0x10`. Base do proxy genérico `handle:Method()`.
/// Funções já achadas por `resolve_in_class`, por (classe, CName do método), com o número de natives registradas na hora.
/// A busca sobe a cadeia de classes e confere cada método com 2 `is_readable` (syscall de ~360 ns): centenas numa classe como
/// `PlayerPuppet`, e ~40 lugares refazem a mesma busca a cada chamada (`PlayerPuppet.GetGame` em 8 deles). A classe não muda de
/// endereço enquanto o processo vive; a função de script muda quando o motor recarrega os scripts dos mods do mod.io
/// ([`scripts_recarregados`]), e native nova registrada também invalida tudo (a classe pode ter ganhado um método que
/// sombreia o do pai). Só guarda o que achou —
/// no boot a falha pode ser classe ainda incompleta. Função global (`resolve_global_function_robust`, ~4435 na varredura) entra com a
/// classe 0.
struct FuncoesResolvidas {
    rotas: usize,
    mapa: std::collections::HashMap<(usize, u64), (usize, usize, bool)>,
}
static FUNCOES_RESOLVIDAS: std::sync::Mutex<Option<FuncoesResolvidas>> = std::sync::Mutex::new(None);

pub(crate) fn esquecer_funcoes_resolvidas() {
    *FUNCOES_RESOLVIDAS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Quantas vezes o motor refez o bind das funções de script com o processo vivo: quem guarda função de script fora deste cache confere.
static GERACAO_DE_SCRIPTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn geracao_de_scripts() -> u64 {
    GERACAO_DE_SCRIPTS.load(std::sync::atomic::Ordering::Acquire)
}

/// O motor recarregou os scripts dos mods do mod.io (`inicio::boot::install_recarga_de_scripts_dos_mods`): as funções de script têm endereço
/// novo e as resolvidas antes apontam para as mortas.
pub(crate) fn scripts_recarregados() {
    esquecer_funcoes_resolvidas();
    GERACAO_DE_SCRIPTS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
}

/// Função guardada para `(classe, CName)`; função global usa a classe 0. Native nova registrada limpa tudo antes de responder.
fn funcao_guardada(chave: (usize, u64)) -> Option<(usize, usize, bool)> {
    let rotas = crate::register::rotas_registradas();
    let mut guardadas = FUNCOES_RESOLVIDAS.lock().unwrap_or_else(|e| e.into_inner());
    if guardadas.as_ref().is_some_and(|g| g.rotas != rotas) {
        *guardadas = None;
    }
    guardadas.as_ref().and_then(|g| g.mapa.get(&chave).copied())
}

fn guardar_funcao(chave: (usize, u64), funcao: (usize, usize, bool)) {
    let rotas = crate::register::rotas_registradas();
    let mut guardadas = FUNCOES_RESOLVIDAS.lock().unwrap_or_else(|e| e.into_inner());
    guardadas
        .get_or_insert_with(|| FuncoesResolvidas { rotas, mapa: std::collections::HashMap::new() })
        .mapa
        .insert(chave, funcao);
}

pub unsafe fn resolve_in_class(cls0: *mut c_void, method: &str) -> Option<ResolvedFn> {
    resolve_in_class_hash(cls0, cname(method))
}

/// `resolve_in_class` pelo hash do nome, pra quem já tem o CName na mão (o despacho de callback não volta ao texto).
pub unsafe fn resolve_in_class_hash(cls0: *mut c_void, mh: u64) -> Option<ResolvedFn> {
    let chave = (cls0 as usize, mh);
    if let Some((func, ret_type, is_static)) = funcao_guardada(chave) {
        return Some(ResolvedFn { func: func as *mut c_void, ret_type: ret_type as *mut c_void, is_static });
    }
    let achada = resolve_in_class_pela_cadeia(cls0, mh);
    if let Some(f) = &achada {
        guardar_funcao(chave, (f.func as usize, f.ret_type as usize, f.is_static));
    }
    achada
}

unsafe fn resolve_in_class_pela_cadeia(cls0: *mut c_void, mh: u64) -> Option<ResolvedFn> {
    let mut cls = cls0;
    // Tabelas e objetos de função do RTTI vivem o processo inteiro: página provada vale a busca toda.
    let mut pagina_dos_slots = usize::MAX;
    let mut pagina_das_funcoes = usize::MAX;
    // GUARDA: cadeia de parents (cls+0x10) sem limite loopa ETERNO (= CONGELA o jogo) se o
    // cls for handle stale/torto. Limita a profundidade + exige cls mapeado (`memoria`) por nível.
    let mut guard = 0;
    while !cls.is_null() && guard < 64 {
        guard += 1;
        if !crate::memoria::is_readable(cls as *const c_void, 0x60) {
            break;
        }
        let clsb = cls as *const u8;
        for off in [0x48usize, 0x58usize] {
            let fp = rd_ptr(clsb.add(off)) as *const u8;
            let n = rd_u32(clsb.add(off + 0x0C));
            if !fp.is_null() && n < 20_000 && crate::memoria::is_readable(fp as *const c_void, 8) {
                for i in 0..n as usize {
                    let slot = fp.add(i * 8);
                    if !legivel_na_pagina_provada(slot, 8, &mut pagina_dos_slots) {
                        break;
                    }
                    let f = rd_ptr(slot) as *const u8;
                    if !legivel_na_pagina_provada(f, 0x20, &mut pagina_das_funcoes) {
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
    if obj.is_null() || !crate::memoria::is_readable(obj as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = rd_ptr(obj as *const u8);
    // NÃO exigir a vtable no módulo: classes SCRIPTED (ex.: PauseMenuListItemData, controllers
    // de lista ink) têm vtable GERADA EM RUNTIME no HEAP, não no binário. Só exige vt legível.
    if !crate::memoria::is_readable(vt as *const c_void, 0x10) {
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
// CProperty (layout CONFIRMADO no macOS via reflection-test pois CProperty NÃO tem
// vtable → sem o shift +0x08 do Itanium): type@0x00, name(CName)@0x08, group@0x10, parent@0x18,
// valueOffset(u32)@0x20, flags@0x28; sizeof 0x30. O valor vive em `obj + valueOffset`
// (salvo flags.inValueHolder@bit0x15 — raro; V1 ignora e lê inline).

/// Acha a `DynArray<CProperty*>` da classe. NÃO é offset fixo garantido no macOS → tenta
/// cls+0x28 (documentado) e fallback noutros, EXCLUINDO 0x48/0x58 (= funcs/staticFuncs, que
/// também têm name@0x08 e confundiriam). Valida: 1a entrada é CProperty* com name!=0 e
/// valueOffset<0x10000. Devolve (entries, count).
unsafe fn props_array(cls: *mut c_void) -> Option<(*const u8, u32)> {
    if !crate::memoria::is_readable(cls as *const c_void, 0x80) {
        return None;
    }
    let clsb = cls as *const u8;
    for &off in &[0x28usize, 0x30, 0x38, 0x40, 0x20, 0x18, 0x60, 0x68, 0x70] {
        let arr = match rd_ptr_chk(clsb.add(off)) {
            Some(p) => p as *const u8,
            None => continue,
        };
        let cnt = match rd_u32_chk(clsb.add(off + 0x0C)) {
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
        if p0.is_null() || !crate::memoria::is_readable(p0 as *const c_void, 0x30) {
            continue;
        }
        if rd_u64(p0.add(0x08)) != 0 && rd_u32(p0.add(0x20)) < 0x10000 {
            return Some((arr, cnt));
        }
    }
    None
}

/// Acha um CProperty por nome numa CClass (+ cadeia de parents). Devolve o CProperty* ou null.
#[cfg(feature = "devtools")]
pub unsafe fn find_property(reg: &Registry, class: &str, prop: &str) -> *mut c_void {
    find_property_in_class(reg.class_by_name(class), prop)
}

/// API de reflexão de classe (2026-07-24): lista os `CProperty*` declarados DIRETAMENTE em `cls`
/// (mesmo nível que `propdump`/`dump_props` já usam — `props_array`, PROVADO in-game há sessões).
/// NÃO sobe pra parents (cada classe da cadeia tem sua própria lista; um mod pode andar
/// `GetParent()` pra ver as herdadas). Vetor vazio se a classe
/// não tiver props próprias resolvíveis. Base de `BwmsReflClassGetPropertyCount`/`ByIndex`
/// (divergência documentada: substitui `GetProperties()->array<...>` por 2 getters indexados,
/// evitando marshalar um array<Uint64> de retorno — RE nova fora de escopo desta rodada).
pub unsafe fn class_own_properties(cls: *mut c_void) -> Vec<*mut c_void> {
    let mut out = Vec::new();
    if let Some((arr, cnt)) = props_array(cls) {
        for i in 0..cnt as usize {
            match rd_ptr_chk(arr.add(i * 8)) {
                Some(p) if !p.is_null() => out.push(p as *mut c_void),
                _ => break,
            }
        }
    }
    out
}

/// `ReflectionClass.GetFunctions()`/`GetStaticFunctions()` (`PENDENCIAS-UNIFICADAS.md`
/// item #51, "listar TODOS os membros nunca confirmado") — espelha `class_own_properties`, mas
/// pros arrays `funcs@+0x48`(instância)/`staticFuncs@+0x58` já andados por `resolve_in_class`
/// (que só busca POR NOME); aqui devolve TODOS os ponteiros de uma vez. `static_only` escolhe qual
/// dos 2 arrays.
#[cfg(feature = "devtools")]
pub unsafe fn class_own_functions(cls: *mut c_void, static_only: bool) -> Vec<*mut c_void> {
    let mut out = Vec::new();
    let off = if static_only { 0x58usize } else { 0x48usize };
    if !crate::memoria::is_readable(cls as *const c_void, 0x60) {
        return out;
    }
    let clsb = cls as *const u8;
    let fp = rd_ptr(clsb.add(off)) as *const u8;
    let n = rd_u32(clsb.add(off + 0x0C));
    if fp.is_null() || n > 20_000 || !crate::memoria::is_readable(fp as *const c_void, 8) {
        return out;
    }
    for i in 0..n as usize {
        let slot = fp.add(i * 8);
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            break;
        }
        let f = rd_ptr(slot) as *mut c_void;
        // CRASH REAL achado 2026-08-09 (`Cyberpunk2077-2026-08-09-130720.ips`, EXC_BAD_ACCESS em
        // `probe_class_own_functions`): a 1ª versão só validava que o SLOT do array era legível
        // (`rd_ptr_chk` no array), nunca que o PONTEIRO GUARDADO ali (o `f` em si) apontava pra
        // memória válida — omissão vs. `resolve_in_class` (já provado), que SEMPRE valida
        // `is_readable(f, 0x20)` antes de tocar o objeto. Fix: mesma validação aqui, `continue`
        // (pula a entrada suja) em vez de assumir válido.
        if f.is_null() || !crate::memoria::is_readable(f, 0x20) {
            continue;
        }
        out.push(f);
    }
    out
}

/// Comando `funclistdump <classe> [n]` — lista as primeiras `n` (default 20) funções PRÓPRIAS
/// (instância+estática) da classe, com nome+flags decodificados (`function_flags`).
#[cfg(feature = "devtools")]
pub unsafe fn probe_class_own_functions(reg: &Registry, class: &str, n: usize) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log_dev!("[funclistdump] classe '{class}' não resolveu");
        return;
    }
    let inst = class_own_functions(cls, false);
    let stat = class_own_functions(cls, true);
    crate::log_dev!(
        "[funclistdump] {class}: {} método(s) de instância, {} estático(s), mostrando até {n} de cada",
        inst.len(),
        stat.len()
    );
    for (label, list) in [("inst", &inst), ("static", &stat)] {
        for f in list.iter().take(n) {
            if !crate::memoria::is_readable((*f as *const u8) as *const c_void, 0x20) {
                crate::log_dev!("[funclistdump]   [{label}] <ponteiro ilegível, pulado>");
                continue;
            }
            let name_hash = rd_u64((*f as *const u8).add(0x10)); // CBaseFunction.shortName@+0x10
            let name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
            let flags = function_flags(*f).map(decode_function_flags).unwrap_or_else(|| "?".into());
            crate::log_dev!("[funclistdump]   [{label}] {name}: {flags}");
        }
    }
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
                if pp.is_null() || !crate::memoria::is_readable(pp as *const c_void, 0x30) {
                    continue;
                }
                if rd_u64(pp.add(0x08)) == ph {
                    return pp as *mut c_void;
                }
            }
        }
        if !crate::memoria::is_readable(cls as *const c_void, 0x18) {
            break;
        }
        cls = rd_ptr((cls as *const u8).add(0x10)); // parent
    }
    std::ptr::null_mut()
}

/// `CProperty*` da classe e dos pais, a classe primeiro.
pub(crate) unsafe fn props_crus(cls0: *mut c_void) -> Vec<*const u8> {
    let mut saida = Vec::new();
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
                if pp.is_null() || !crate::memoria::is_readable(pp as *const c_void, 0x30) {
                    continue;
                }
                saida.push(pp);
            }
        }
        if !crate::memoria::is_readable(cls as *const c_void, 0x18) {
            break;
        }
        cls = rd_ptr((cls as *const u8).add(0x10));
    }
    saida
}

/// Properties da classe e dos pais, a classe primeiro: `(nome, tipo, offset do valor)`. Tipo em `CProperty+0x00` (calibrado pelo `reflholders`), nome em
/// `+0x08`, offset em `+0x20`.
pub unsafe fn propriedades(cls0: *mut c_void) -> Vec<(u64, *mut c_void, u32)> {
    props_crus(cls0).into_iter().map(|pp| (rd_u64(pp.add(0x08)), rd_ptr(pp), rd_u32(pp.add(0x20)))).collect()
}

/// Como [`propriedades`], com o bit `inValueHolder` (`CProperty.flags` `+0x28`, `0x200000`) no lugar do nome: `(tipo, offset, no valueHolder)`, o que a
/// cópia de campo pelo [`field_ptr`] precisa.
pub unsafe fn propriedades_com_holder(cls0: *mut c_void) -> Vec<(*mut c_void, u32, bool)> {
    props_crus(cls0).into_iter().map(|pp| (rd_ptr(pp), rd_u32(pp.add(0x20)), prop_in_holder(pp as *const c_void))).collect()
}

/// valueOffset (u32@0x20) de um CProperty.
pub unsafe fn prop_value_offset(prop: *const c_void) -> u32 {
    rd_u32((prop as *const u8).add(0x20))
}
/// `inValueHolder` (`CProperty.flags` `+0x28`, `0x200000`): o valor mora no `valueHolder` do objeto (`obj+0x38`), não no objeto — o caso
/// dos campos de classe scripted.
pub unsafe fn prop_in_holder(prop: *const c_void) -> bool {
    rd_u64((prop as *const u8).add(0x28)) & crate::tipos_do_motor::PROP_FLAG_NO_HOLDER != 0
}
/// Ponteiro pro valor da propriedade: `obj + valueOffset`, ou `valueHolder + valueOffset` com `inValueHolder` ([`field_ptr`]). Nulo sem
/// holder ou com endereço ilegível: quem lê devolve zero, quem escreve devolve `false`. Até 2026-09-19 ignorava o bit, e a reflexão lia e
/// escrevia no objeto, fora do campo, em toda classe scripted.
pub unsafe fn prop_value_ptr(prop: *const c_void, obj: *mut c_void) -> *mut c_void {
    if prop.is_null() || obj.is_null() {
        return std::ptr::null_mut();
    }
    field_ptr(obj, prop_value_offset(prop), prop_in_holder(prop))
}
pub unsafe fn prop_get_u32(prop: *const c_void, obj: *mut c_void) -> u32 {
    let p = prop_value_ptr(prop, obj);
    if p.is_null() { 0 } else { rd_u32(p as *const u8) }
}
pub unsafe fn prop_set_u32(prop: *const c_void, obj: *mut c_void, v: u32) -> bool {
    let p = prop_value_ptr(prop, obj);
    if !p.is_null() {
        core::ptr::write_unaligned(p as *mut u32, v);
    }
    !p.is_null()
}
pub unsafe fn prop_get_f32(prop: *const c_void, obj: *mut c_void) -> f32 {
    f32::from_bits(prop_get_u32(prop, obj))
}
pub unsafe fn prop_set_f32(prop: *const c_void, obj: *mut c_void, v: f32) -> bool {
    prop_set_u32(prop, obj, v.to_bits())
}
pub unsafe fn prop_get_bool(prop: *const c_void, obj: *mut c_void) -> bool {
    let p = prop_value_ptr(prop, obj);
    !p.is_null() && core::ptr::read_unaligned(p as *const u8) != 0
}
pub unsafe fn prop_set_bool(prop: *const c_void, obj: *mut c_void, v: bool) -> bool {
    let p = prop_value_ptr(prop, obj);
    if !p.is_null() {
        core::ptr::write_unaligned(p as *mut u8, v as u8);
    }
    !p.is_null()
}

/// Probe de Reflection (DEV): dado o CClass* `cls0` (ex.: `class_of(player)`), anda a cadeia de
/// parents achando a 1a `DynArray<CProperty*>`, dumpa props (nome via resolve_cname + valueOffset)
/// p/ CONFIRMAR o layout do CProperty no macOS, faz GET read-only no `obj` vivo (se houver) e um
/// round-trip set/get num OBJETO FAKE nosso (buffer pool, ZERO efeito no jogo). Se não achar props,
/// despeja o scan CRU de cls+0x10..0x88 (ptr/count + 1a entrada) p/ eu ver onde elas estão.
#[cfg(feature = "devtools")]
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
                if pp.is_null() || !crate::memoria::is_readable(pp as *const c_void, 0x30) {
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
        if !crate::memoria::is_readable(cls as *const c_void, 0x18) {
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
            if p.is_null() || c == 0 || c > 20_000 || !crate::memoria::is_readable(p, 8) {
                continue;
            }
            let e0 = rd_ptr(p as *const u8);
            let (nm, vo) = if crate::memoria::is_readable(e0, 0x30) {
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
    if !obj.is_null() && crate::memoria::is_readable(prop_value_ptr(first_prop, obj) as *const c_void, 4) {
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

/// NewObject(className): resolve a classe pelo nome nativo ou, sem ele, pelo nome de script, e cria
/// a instância pelo `CClass::CreateInstance` do motor ([`create_instance`]): o objeto nasce pelo
/// allocator da classe e o motor pode soltá-lo pelo mesmo allocator. Nulo se a classe não resolve.
pub unsafe fn new_object(reg: &Registry, class_name: &str) -> *mut c_void {
    // Nome de script (`inkText`, `inkCanvas`) cai no `GetClassByScriptName`: a classe nativa desses é `inkTextWidget`/`inkCanvasWidget`, e o
    // teste do boot 151 recebeu nulo pedindo `inkCanvas`.
    let mut cls = reg.class_by_name(class_name);
    if cls.is_null() {
        cls = reg.class_by_script_name(class_name);
        // A/B do boot 153 (engagement não chegou no boot 152): cada nome que passa por aqui vai uma vez pro log.
        // Só com o log: no público o `log_dev!` é vazio, e a trava + a busca rodavam por nada.
        #[cfg(feature = "devlog")]
        {
            static LOGADOS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            if let Ok(mut v) = LOGADOS.lock() {
                if v.len() < 64 && !v.iter().any(|n| n == class_name) {
                    v.push(class_name.to_string());
                    crate::log_dev!("[new_object] '{class_name}' pelo nome de script -> {cls:p}");
                }
            }
        }
    }
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    new_object_from_class_named(cls, class_name)
}

/// `Reflection.ReflectionClass.MakeHandle()` (`PENDENCIAS-UNIFICADAS.md` item #51) —
/// mesmo núcleo de `new_object`, mas a partir de um `CClass*` JÁ RESOLVIDO (o caso real de
/// `MakeHandle`: um mod já tem o ponteiro opaco de uma classe, via `BwmsReflGetClass` ou
/// `GetDerivedClasses`, não o nome de novo). Sem nome pra log/trace (usa o endereço cru).
pub unsafe fn new_object_from_class(cls: *mut c_void) -> *mut c_void {
    new_object_from_class_named(cls, "<sem nome>")
}


/// `CClass::CreateInstance` real (AL-2c): o objeto nasce pelo allocator da classe, e o motor pode soltá-lo pelo mesmo allocator
/// (fila de input, pin de mod, fases de missão de mod). Na game thread, em thread com script em execução, ou no job que desserializa um pacote: o
/// `Instantiate` do motor (`0x10431dc00`) cria os objetos do item por esta mesma função (`0x10121ac20` → `0x1012086dc` → `0x10120acec`).
pub(crate) unsafe fn create_instance(cls: *mut c_void) -> *mut c_void {
    if !sane(cls) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, u32, u32) -> *mut c_void = std::mem::transmute(rebase(crate::jogo::objetos::OBJETO_CRIAR_VM));
    f(cls, 0, 1)
}

#[cfg_attr(not(feature = "devlog"), allow(unused_variables))]
unsafe fn new_object_from_class_named(cls: *mut c_void, class_name: &str) -> *mut c_void {
    let obj = create_instance(cls);
    crate::tipos_do_motor::zerar_cauda_de_evento(cls, obj);
    // Breadcrumb só de dev: no público o format! por objeto criado não tem para onde ir.
    #[cfg(feature = "devlog")]
    crate::trace(&format!("new_object: {class_name} -> CreateInstance {obj:p}"));
    obj
}

/// Constrói um `ref<T>` REAL (16 bytes: `{instance, refCountBlock}`) a partir de um ponteiro de
/// objeto cru, chamando a rotina do PRÓPRIO MOTOR (`ADDR_HANDLE_CTOR`, ver nota lá) — NÃO uma
/// reimplementação nossa do refcount. `obj==null` produz um handle nulo válido (a própria rotina
/// trata isso). Escreve diretamente em `out` (16 bytes; caller garante espaço, mesmo contrato de
/// `write_handle_ret` que substitui) — usar em TODA função que devolve uma instância NOVA/self de
/// classe forjada (ex.: `RegisterCallback` devolvendo o handler recém-criado; `AddTarget`/
/// `SetLifetime`/etc. devolvendo `self` pra encadeamento — nesse caso a rotina acha o bloco JÁ
/// existente do objeto via a tabela de weak-owner e só incrementa, não duplica).
pub unsafe fn make_handle(out: *mut c_void, obj: *mut c_void) {
    if out.is_null() {
        return;
    }
    let ctor: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(ADDR_HANDLE_CTOR));
    ctor(out, obj);
}

/// Constrói um `wref<T>` REAL (item #387) — mesmos 16 bytes `{instance,
/// refCountBlock}` de `make_handle`, mas incrementa o contador FRACO (`weakRefs`, u32 @
/// `refCountBlock+0x04`) em vez do forte. Layout confirmado:
/// `RefCnt{strongRefs@0x00(u32), weakRefs@0x04(u32)}` — MESMO bloco que `make_handle`/`Handle_ctor`
/// já usa e já é real (RE confirmada, handler de callback, 2026-08-10). `IncWeakRef()` real é
/// um incremento atômico puro (sem endereço nativo, `InterlockedIncrement`), então replicável
/// direto em Rust sem nenhuma RE de endereço nova — só a leitura do bloco (via `make_handle`,
/// que a própria rotina do motor já sabe achar/reusar o bloco existente de um objeto, em vez de
/// duplicar — mesmo comportamento já documentado pra `make_handle` chamado 2x no mesmo objeto).
/// `DecWeakRef` (limpeza final) SIM precisaria de endereço nativo — não usado aqui (mesma
/// filosofia já aceita no projeto pra handles forjados/emprestados: nunca desaloca).
///
/// O construtor do motor soma 1 no FORTE; esse 1 volta aqui, senão cada wref devolvido prende o objeto (entidade, widget) pra
/// sempre. Desce só se o forte for > 1 (CAS): com 1 o bloco não tem outro dono e soltar destruiria nada — o 1 fica. Devolve
/// `true` quando o forte voltou.
pub unsafe fn make_weak_handle(out: *mut c_void, obj: *mut c_void) -> bool {
    use std::sync::atomic::{AtomicU32, Ordering};
    if out.is_null() {
        return false;
    }
    if obj.is_null() {
        core::ptr::write_unaligned(out as *mut u64, 0u64);
        core::ptr::write_unaligned((out as *mut u64).add(1), 0u64);
        return false;
    }
    let mut tmp = [0u8; 16];
    make_handle(tmp.as_mut_ptr() as *mut c_void, obj);
    let instance = (tmp.as_ptr() as *const u64).read_unaligned();
    let refcount_block = (tmp.as_ptr().add(8) as *const u64).read_unaligned();
    let mut forte_voltou = false;
    if refcount_block != 0 {
        let forte = refcount_block as *const AtomicU32;
        (*forte.add(1)).fetch_add(1, Ordering::SeqCst);
        let mut atual = (*forte).load(Ordering::SeqCst);
        while atual > 1 {
            match (*forte).compare_exchange(atual, atual - 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => {
                    forte_voltou = true;
                    break;
                }
                Err(visto) => atual = visto,
            }
        }
    }
    core::ptr::write_unaligned(out as *mut u64, instance);
    core::ptr::write_unaligned((out as *mut u64).add(1), refcount_block);
    forte_voltou
}

/// True se `cls` (CClass*) deriva de IScriptable (tem o campo nativeType@0x30). Sobe a
/// cadeia de parents (cls+0x10) comparando o nome do tipo. Evita corromper STRUCT puro.
pub(crate) unsafe fn derives_from_iscriptable(cls: *mut c_void) -> bool {
    crate::register::classe_deriva_de_nome(cls, cname("IScriptable"))
}

/// Resolve um CAMPO por nome subindo a cadeia de parents → (valueOffset, IRTTIType*).
/// Array de props @ CClass+0x28 (entries), count u32 @ CClass+0x34 (NÃO 0x30=capacity);
/// CProperty: type@0x00, name(CName)@0x08, valueOffset(u32)@0x20 (confirmado por propdump
/// runtime: eventName=0x20/action=0x28). Toda leitura é checada pela `memoria` (não crasha).
/// O nome da fonte `m_xxx` também acha o campo: o carregador de scripts registra a propriedade
/// sem o prefixo (`m_inCrouch` do `PlayerPuppet` é `inCrouch` no RTTI, `proofs/2026-06-26-
/// reflection-100-verde.log`), a mesma regra do [`find_property_in_class`]. O nome exato vem
/// primeiro.
pub unsafe fn resolve_prop_in_class(cls0: *mut c_void, field: &str) -> Option<(u32, *mut c_void, bool)> {
    resolve_prop_exato(cls0, field).or_else(|| resolve_prop_exato(cls0, field.strip_prefix("m_")?))
}

unsafe fn resolve_prop_exato(cls0: *mut c_void, field: &str) -> Option<(u32, *mut c_void, bool)> {
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
                    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 0x30) {
                        continue;
                    }
                    if rd_u64(p.add(0x08)) == want {
                        let voff = rd_u32(p.add(0x20));
                        let ty = rd_ptr(p.add(0x00));
                        // CProperty.flags @ +0x28; bit 0x15 (máscara 0x200000) = inValueHolder:
                        // o valor mora DENTRO do IScriptable.valueHolder (obj+0x38), não direto
                        // no obj. Props de classes SCRIPTED (controllers ink) quase sempre têm
                        // isso → era o "menuListController dangling" (líamos o ptr do holder).
                        let in_holder = prop_in_holder(p as *const c_void);
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
    if !crate::memoria::is_readable(holder as *const c_void, voff as usize + 8) {
        return std::ptr::null_mut();
    }
    holder.add(voff as usize) as *mut c_void
}

/// CClass* INTERNO de um tipo Handle/WeakHandle (CRTTIHandleType/WeakHandleType.innerType@0x10).
/// Só chamar quando o tipo é handle:/whandle: (senão o read de +0x10 é lixo).
pub unsafe fn inner_type(ty: *mut c_void) -> *mut c_void {
    if ty.is_null() || !crate::memoria::is_readable(ty as *const c_void, 0x18) {
        return std::ptr::null_mut();
    }
    rd_ptr((ty as *const u8).add(0x10))
}

/// CName do NOME do tipo lido CRU de IType+0x18. SÓ vale p/ tipos CLASSE/handle (onde +0x18 é
/// o CName do nome cacheado). Pra tipos FUNDAMENTAIS (String/CName/Int32/Bool/Float) +0x18 NÃO
/// é o nome (dá 0/lixo) → use `type_name_getname`. Mantido p/ walks de hierarquia de classe
/// (derives_from_iscriptable etc.), onde sempre é classe.
pub unsafe fn type_name_hash(ty: *mut c_void) -> u64 {
    if ty.is_null() || !crate::memoria::is_readable(ty as *const c_void, 0x20) {
        return 0;
    }
    rd_u64((ty as *const u8).add(0x18))
}

/// CName do nome do tipo via `IRTTIType::GetName()` (vtable+0x10 no macOS). Vale p/ TODOS os
/// tipos — inclusive FUNDAMENTAIS (String/CName/Int32). Idêntico a `type_name_hash` em tipos
/// classe/handle (+0x18 == GetName lá), mas correto onde +0x18 falha. Use SEMPRE que o tipo
/// puder ser fundamental (tipo de CAMPO/valor de marshalling). Getter const → seguro de chamar.
pub unsafe fn type_name_getname(ty: *mut c_void) -> u64 {
    if ty.is_null() || !crate::memoria::is_readable(ty as *const c_void, 0x18) {
        return 0;
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() || !crate::memoria::is_readable(vt as *const c_void, 0x18) {
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

/// Lê uma red::String (0x20 bytes) pelo `read_cstring`, com os crivos dele (inline até 0x14, teto de 1 MB, `read_chunk` acima
/// de 512 B); vazia quando a leitura é recusada.
pub unsafe fn red_string_read(slot: *const u8) -> String {
    read_cstring(slot).unwrap_or_default()
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

/// Lista métodos RTTI (instância+estáticos) de uma classe, subindo a cadeia de parents.
/// Cada CClassFunction* em CClass+0x48/0x58; nome = CName hash em func+0x10 resolvido pelo pool.
#[cfg(feature = "devtools")]
pub unsafe fn dump_class_funcs(reg: &Registry, class_name: &str) -> String {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        return format!("[rttifuncs] classe '{class_name}' NAO encontrada no registry");
    }
    let mut s = format!("[rttifuncs] '{class_name}': cls={cls:p}\n");
    let mut cur = cls;
    let mut depth = 0usize;
    while !cur.is_null() && depth < 32 {
        depth += 1;
        if !crate::memoria::is_readable(cur as *const c_void, 0x60) { break; }
        let clsb = cur as *const u8;
        for (off, kind) in [(0x48usize, "inst"), (0x58usize, "stat")] {
            let fp = rd_ptr(clsb.add(off)) as *const u8;
            let cap = rd_u32(clsb.add(off + 8));  // DynArray: {ptr@+0, cap@+8, sz@+12}
            let n   = rd_u32(clsb.add(off + 12)); // sz = count real
            s.push_str(&format!("  [depth={depth} {kind}] fp={fp:p} cap={cap} sz={n}\n"));
            if fp.is_null() || n == 0 || n > 2000 || !crate::memoria::is_readable(fp as *const c_void, 8) { continue; }
            s.push_str(&format!("  [depth={depth} {kind}] {n} funcs:\n"));
            for i in 0..n.min(200) as usize {
                let slot = fp.add(i * 8);
                if !crate::memoria::is_readable(slot as *const c_void, 8) { break; }
                let f = rd_ptr(slot) as *const u8;
                if f.is_null() || !crate::memoria::is_readable(f as *const c_void, 0x20) { continue; }
                let name_h = rd_u64(f.add(0x10));
                let name = crate::cname::resolve_cname(name_h);
                s.push_str(&format!("    [{i:03}] {name} ({name_h:#018x})\n"));
            }
        }
        cur = rd_ptr(clsb.add(0x10)); // parent CClass*
    }
    s
}

/// Item #55 (`PENDENCIAS-UNIFICADAS.md`): `GetFunction(CName)` (vtbl+0x30, já usado
/// por `register::get_function`) só acha o que foi registrado via `RegisterFunction` — funções
/// GLOBAIS puramente ESCRIPTADAS (declaradas num `.reds`, nunca passaram pelo `RegisterFunction`
/// do lado Rust) não resolvem por ele. Fallback: varre `GetGlobalFunctions` (vtbl+0x48, já
/// PROVADO — "4435 funções globais") e casa por nome, comparando tanto `name`@+0x10 (short,
/// mesmo campo que `dump_class_funcs` já lê) quanto `fullName`@+0x08 (o outro candidato citado
/// no item) — qualquer um batendo conta como achado, sem exigir saber de antemão qual campo o
/// `name` pedido representa. Read-only, nunca falha alto (retorna null se não achar).
pub unsafe fn resolve_global_function_robust(reg: &Registry, name: &str) -> *mut c_void {
    let chave = (0, crate::cname::cname(name));
    if let Some((f, _, _)) = funcao_guardada(chave) {
        return f as *mut c_void;
    }
    let f = resolve_global_function_pela_varredura(reg, name);
    if !f.is_null() {
        guardar_funcao(chave, (f as usize, 0, true));
    }
    f
}

/// A varredura das ~4435 globais; sem a prova por página eram 2 `is_readable` por função, milissegundos por busca.
unsafe fn resolve_global_function_pela_varredura(reg: &Registry, name: &str) -> *mut c_void {
    let target = crate::cname::cname(name);
    let mut pagina_dos_slots = usize::MAX;
    let mut pagina_das_funcoes = usize::MAX;
    let (entries, n) = reg.get_global_functions();
    if entries.is_null() || n == 0 || n > 200_000 || !crate::memoria::is_readable(entries, 8) {
        return std::ptr::null_mut();
    }
    let base = entries as *const u8;
    for i in 0..n {
        let slot = base.add(i * 8);
        if !legivel_na_pagina_provada(slot, 8, &mut pagina_dos_slots) {
            break;
        }
        let f = rd_ptr(slot) as *const u8;
        if !legivel_na_pagina_provada(f, 0x20, &mut pagina_das_funcoes) {
            continue;
        }
        let name_h = rd_u64(f.add(0x10));
        let fullname_h = rd_u64(f.add(0x08));
        if name_h == target || fullname_h == target {
            return f as *mut c_void;
        }
    }
    // 2ª passada — MODULE-SCOPING (item #55, 2026-08-21). A 1ª passada compara HASH, então
    // uma global declarada dentro de `module X.Y` nunca é achada pelo nome BARE: nem `name`(+0x10)
    // nem `fullName`(+0x08) hasheiam pro bare, os dois carregam a forma qualificada. Isso foi
    // confirmado ao vivo em 2026-08-17 ("nome bare falha, nome qualificado resolve e devolve o valor
    // exato esperado") e é a razão de o item ter voltado de INCERTO pra REAL_GAP.
    //
    // Aqui a comparação é por STRING (reverte o hash com `resolve_cname`, que este projeto já usa
    // pra isso) e casa pelo ÚLTIMO segmento: pedir `MinhaFunc` acha `MeuModulo.MinhaFunc`. Roda SÓ
    // se a passada por hash falhou — custo zero no caminho comum, e nunca muda resultado já
    // resolvido. Ambíguo por construção (2 módulos podem exportar o mesmo nome curto): devolve a
    // 1ª ocorrência, mesma disciplina de `resolve_in_class`, e por isso vem DEPOIS do exato.
    for i in 0..n {
        let slot = base.add(i * 8);
        if !legivel_na_pagina_provada(slot, 8, &mut pagina_dos_slots) {
            break;
        }
        let f = rd_ptr(slot) as *const u8;
        if !legivel_na_pagina_provada(f, 0x20, &mut pagina_das_funcoes) {
            continue;
        }
        for h in [rd_u64(f.add(0x08)), rd_u64(f.add(0x10))] {
            if h == 0 {
                continue;
            }
            if global_fn_name_matches(name, &crate::cname::resolve_cname(h)) {
                return f as *mut c_void;
            }
        }
    }
    std::ptr::null_mut()
}

/// Núcleo PURO da decisão de nome do `resolve_global_function_robust` (item #55) —
/// extraído pra ser testável offline, sem RTTI/jogo.
///
/// Casa `requested` contra o nome REAL de uma global. Regras, em ordem:
/// 1. igualdade exata (cobre o caso sem módulo — comportamento de sempre);
/// 2. último segmento separado por `.` (é o module-scoping: `MeuModulo.MinhaFunc` responde por
///    `MinhaFunc`);
/// 3. o nome real pode vir DECORADO com a assinatura (`Func;Int32Float`, idioma do RTTI do jogo pra
///    distinguir overload) — corta no `;` antes de comparar.
///
/// Deliberadamente NÃO casa o contrário (pedir qualificado e achar bare): quem pede
/// `Modulo.Func` está sendo específico, e responder com uma `Func` de outro escopo seria pior que
/// não achar.
pub(crate) fn global_fn_name_matches(requested: &str, actual: &str) -> bool {
    if requested.is_empty() || actual.is_empty() {
        return false;
    }
    let strip = |s: &str| -> String {
        let s = s.split(';').next().unwrap_or(s);
        s.to_string()
    };
    let a = strip(actual);
    let r = strip(requested);
    if a == r {
        return true;
    }
    match a.rsplit_once('.') {
        Some((_, short)) => short == r,
        None => false,
    }
}

/// DIAGNÓSTICO (save-safe se rodado no MENU PRINCIPAL): despeja a vtable do ClassType
/// como VM addr ESTÁTICO (un_rebase → casa com `nm`/c++filt) e chama só os getters de
/// BAIXO RISCO (no Mac GetSize@0x18, GetAlignment@0x20; o 0x10 é GetName). NÃO constrói/aloca nada. Serve p/
/// conferir os slots da vtable do macOS numa classe qualquer.
#[cfg(feature = "devtools")]
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
    let (tamanho, alinhamento) = tamanho_e_alinhamento(cls);
    s.push_str(&format!("  -> GetSize@0x18(cls) = {tamanho}, GetAlignment@0x20(cls) = {alinhamento}\n"));
    s
}

/// DIAGNÓSTICO de PROPRIEDADES (save-safe, só leitura): varre os offsets candidatos a
/// DynArray<CProperty*> na CClass e despeja cada CProperty cru (qwords [0..0x30]) pra eu
/// mapear o layout (qual qword é o CName do nome, qual u32 é o valueOffset, qual ptr é o
/// tipo). Já mostra o hash de nome contra alguns nomes conhecidos pra casar na hora.
#[cfg(feature = "devtools")]
pub unsafe fn dump_props(reg: &Registry, class_name: &str) -> String {
    let cls = reg.class_by_name(class_name);
    if cls.is_null() {
        return format!("[propdump] classe '{class_name}' NAO encontrada");
    }
    let clsb = cls as *const u8;
    let mut s = format!("[propdump] '{class_name}': cls static {:#x}\n", crate::un_rebase(cls));
    // CRITÉRIO DE VALIDADE — GENERALIZADO 2026-09-06.
    //
    // A versão anterior só reportava um array se algum entry casasse um nome de uma lista FIXA
    // (os campos da página de settings de mod, item pra qual a função nasceu). Efeito: `propdump` de
    // qualquer outra classe imprimia só o ponteiro da classe e NADA MAIS — indistinguível de
    // "essa classe não tem property". Medido em `entIComponent`/`entMeshComponent`/
    // `questWorldStateSystem`: 4 chamadas, 4 silêncios, zero informação.
    //
    // O crivo novo não depende de conhecer o nome antes: um `CProperty*` de verdade tem, num dos
    // primeiros qwords, um hash de CName que o dicionário REVERTE pra texto. Lixo quase nunca
    // reverte. Assim o comando serve pra qualquer classe — que é o que vários itens precisam
    // (`#168` quer o offset de `appearancePath`, e a regra do projeto desde o `#30` é perguntar
    // o offset ao RTTI em vez de confiar no header do SDK).
    let known = [
        "label", "eventName", "action", "value", "menuListController", "data",
        "isEmpty", "id", "name", "optionType", "icon", "description", "selectable",
    ];
    // varre 0x18..0x70: cada par (ptr@off, count@off+8). TODA leitura é checada pela `memoria` →
    // não crasha em offset falso. Só REPORTA o array se algum entry casar um nome
    // conhecido (= é o DynArray<CProperty*> de verdade, não lixo).
    for off in (0x18usize..=0x70).step_by(8) {
        let arr = match rd_ptr_chk(clsb.add(off)) {
            Some(p) => p as *const u8,
            None => continue,
        };
        let cnt = match rd_u32_chk(clsb.add(off + 0x0C)) {
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
            if pp.is_null() || !crate::memoria::is_readable(pp as *const c_void, 0x30) {
                continue;
            }
            let q: [u64; 6] = std::array::from_fn(|j| rd_u64(pp.add(j * 8)));
            let mut hit = String::new();
            // 1) nome REVERTIDO pelo dicionário de CName — critério geral.
            let mut nome = String::new();
            for (j, &qv) in q.iter().enumerate() {
                if qv == 0 {
                    continue;
                }
                if let Some(n) = crate::cname::name_of(qv) {
                    if n.len() >= 2 && n.len() <= 64 {
                        nome = format!("{n}@q{j}");
                        matched = true;
                        break;
                    }
                }
            }
            // 2) a lista fixa segue, como reforço legível pro caso da página de settings de mod.
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
                "  [{i:02}] {nome:<26} q0={:#x} q1={:#x} q2={:#x} q3={:#x} q4={:#x} q5={:#x}{hit}\n",
                q[0], q[1], q[2], q[3], q[4], q[5]
            ));
        }
        if matched {
            s.push_str(&format!(
                "--- cls+0x{off:02x}: DynArray<CProperty*> count={cnt} (nome revertido) ---\n"
            ));
            s.push_str(&body);
        }
    }
    // persiste num arquivo DEDICADO (sobrevive ao `clear` do log).
    let _ = std::fs::write("/tmp/cp77-propdump.txt", &s);
    s
}

// CScriptStackFrame da 2.5: 0xA8 B. 0x00..0x63 seguem o layout da 2.31 (código +0x00, locals +0x10/+0x18,
// ctx +0x40). O executor monta o frame filho com o nosso como pai, e o filho copia `[pai+0x98]` (mapa do frame
// raiz, lido pela VM na chamada de função com redirecionamento). Por isso o nosso frame tem de ser uma raiz
// completa: isRoot em +0x64, mapa vazio em +0x68 (+0x20 do mapa = -1, alocador em +0x28 do mapa) e +0x98
// apontando para o próprio mapa. Prova no relatório L.md (construtores do frame 2.5).
pub(crate) const FRAME_TAM: usize = 0xA8;
const FRAME_RAIZ: usize = 0x64;
const FRAME_MAPA: usize = 0x68;
const FRAME_MAPA_VAZIO: usize = 0x88;
const FRAME_MAPA_ALOCADOR: usize = 0x90;
const FRAME_MAPA_DA_RAIZ: usize = 0x98;

// Alocador do mapa do frame: o motor grava o mesmo objeto estático em todo frame que constrói. Copiado do 1º
// frame do jogo que passa pelo executor, para não fixar endereço de dado no código.
static FRAME_ALOCADOR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Chamado pelo gancho do executor com o frame do jogo; custa 1 load quando já capturado.
#[inline]
pub(crate) unsafe fn capturar_alocador_do_frame(frame: *const c_void) {
    use std::sync::atomic::Ordering;
    if FRAME_ALOCADOR.load(Ordering::Relaxed) != 0 || frame.is_null() {
        return;
    }
    let a = ((frame as *const u8).add(FRAME_MAPA_ALOCADOR) as *const usize).read_unaligned();
    if a != 0 {
        FRAME_ALOCADOR.store(a, Ordering::Relaxed);
    }
}

/// Frame raiz para o executor: código em +0x00, locals em +0x10/+0x18, ctx em +0x40. O `Box` não move, então
/// o ponteiro de +0x98 para o próprio mapa continua válido até o frame ser solto.
pub(crate) unsafe fn frame_raiz(codigo: *mut u8, locals: *mut u8, ctx: *mut c_void) -> Box<[u8; FRAME_TAM]> {
    let mut fr = Box::new([0u8; FRAME_TAM]);
    let p = fr.as_mut_ptr();
    (p as *mut *mut u8).write_unaligned(codigo);
    (p.add(0x10) as *mut *mut u8).write_unaligned(locals);
    (p.add(0x18) as *mut *mut u8).write_unaligned(locals);
    (p.add(0x40) as *mut *mut c_void).write_unaligned(ctx);
    *p.add(FRAME_RAIZ) = 1;
    (p.add(FRAME_MAPA_VAZIO) as *mut u32).write_unaligned(u32::MAX);
    let alocador = FRAME_ALOCADOR.load(std::sync::atomic::Ordering::Relaxed);
    (p.add(FRAME_MAPA_ALOCADOR) as *mut usize).write_unaligned(alocador);
    (p.add(FRAME_MAPA_DA_RAIZ) as *mut *mut u8).write_unaligned(p.add(FRAME_MAPA));
    fr
}

/// Capacidade do mapa do frame raiz depois da chamada: diferente de 0 = a VM alocou nele, e só o destrutor do
/// motor solta.
pub(crate) fn frame_mapa_alocado(fr: &[u8; FRAME_TAM]) -> u32 {
    u32::from_le_bytes(fr[FRAME_MAPA + 0xc..FRAME_MAPA + 0x10].try_into().unwrap_or([0; 4]))
}

/// Argumento já tipado pro slot de 0x20 bytes do frame.
pub enum Arg {
    Handle(*mut c_void, *mut c_void), // (instância, refcount)
    Item16([u8; 16]),                 // gameItemID já montado (fromTDBID)
    Raw([u8; 16]),                    // valor por-valor cru (ex.: GameInstance)
    Str(String),                      // red::String montado pelo construtor do motor (qualquer tamanho), solto depois da chamada
    I32(u32),
    I64(u64),
    F32(f32),
    Bool(bool),
    CName(u64),
    Enum(u64), // valor de enum já resolvido (escreve u64; engine lê a largura real)
    Tdb([u8; 8]),
    #[cfg(feature = "lua")]
    Bytes(Vec<u8>), // valor no tamanho do tipo do parâmetro (struct por valor dos mods Lua), em slot do tamanho dele
}

/// Como [`call_func`], com o retorno no tamanho do tipo de retorno da função (struct por valor como `Matrix`, 0x40 B, transbordaria os 0x20
/// do `call_func`).
#[cfg(feature = "lua")]
pub unsafe fn call_func_valor(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg]) -> Option<Vec<u8>> {
    let tam = (type_size(rf.ret_type) as usize + 0xf & !0xf).max(0x20);
    let mut res = vec![0u8; tam];
    chamar_pelo_executor(rf, ctx, args, &mut res).then_some(res)
}

/// Tipo (`IType*`) do i-ésimo parâmetro de `func` (`CProperty+0` da lista em `func+0x28`); nulo fora da lista.
#[cfg(feature = "lua")]
pub unsafe fn fn_param_itype(func: *mut c_void, i: usize) -> *mut c_void {
    if func.is_null() || i >= rd_u32((func as *const u8).add(0x30)) as usize {
        return std::ptr::null_mut();
    }
    let p_entries = rd_ptr((func as *const u8).add(0x28)) as *const u8;
    if p_entries.is_null() {
        return std::ptr::null_mut();
    }
    let cprop = rd_ptr(p_entries.add(i * 8));
    if sane(cprop) { rd_ptr(cprop as *const u8) } else { std::ptr::null_mut() }
}

/// Nome (hash do `CName`) do membro de valor `valor` de um `CEnum`: as listas paralelas do [`enum_valor`].
#[cfg(feature = "lua")]
pub unsafe fn enum_nome(en: *const u8, valor: u64) -> Option<u64> {
    if en.is_null() || !crate::memoria::is_readable(en.add(0x28) as *const c_void, 0x18) {
        return None;
    }
    let hp = rd_ptr(en.add(0x28)) as *const u8;
    let n = rd_u32(en.add(0x34));
    let vp = rd_ptr(en.add(0x38)) as *const u8;
    if hp.is_null() || vp.is_null() || n > 100_000 {
        return None;
    }
    (0..n as usize).find(|i| rd_u64(vp.add(i * 8)) == valor).map(|i| rd_u64(hp.add(i * 8)))
}

/// Invoca `rf` com `ctx` e `args`. Monta locals + CProperty sintética por arg +
/// bytecode (LocalVar 0x18 … ParamEnd 0x26) + o CScriptStackFrame, e chama o
/// executor. Devolve os bytes de retorno cru. `None` se algo estiver torto
/// (evita crashar o jogo — diferente do throw do JS dele).
///
/// Tentativa 12 (2026-07-13) — `res` era `[u8;16]` (dimensionado pro maior retorno já
/// exercitado, Vector4=16B). Uma `String` (CString) de retorno tem 0x20 bytes — escrever isso
/// no `res` de 16B TRANSBORDARIA o stack do Rust (achado ANTES de implementar qualquer
/// trampolim de retorno-String, evitando o bug em vez de descobri-lo ao vivo). Alargado pra
/// 0x20; todos os chamadores existentes só leem os primeiros 4-16 bytes, então continuam
/// funcionando sem mudança (é puramente aditivo).
pub unsafe fn call_func(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg]) -> Option<[u8; 0x20]> {
    let mut res = [0u8; 0x20];
    chamar_pelo_executor(rf, ctx, args, &mut res).then_some(res)
}

/// Corpo do [`call_func`]: o executor grava o retorno em `res`, que tem de caber o tipo de retorno. `false` se algo estiver torto.
unsafe fn chamar_pelo_executor(rf: &ResolvedFn, ctx: *mut c_void, args: &[Arg], res: &mut [u8]) -> bool {
    let fb = rf.func as *const u8;
    let p_entries = rd_ptr(fb.add(0x28)) as *const u8;
    let p_count = rd_u32(fb.add(0x30));
    if args.len() > p_count as usize {
        return false; // arg count > params → o executor leria ParamEnd como valor e crasharia
    }
    if p_entries.is_null() && !args.is_empty() {
        return false;
    }

    let n = args.len();
    // Slot de 0x20 por argumento; valor maior (struct por valor dos mods Lua, `Matrix` de 0x40) ocupa o tamanho dele, alinhado a 0x10.
    let slots: Vec<usize> = args.iter().map(|a| match a {
        #[cfg(feature = "lua")]
        Arg::Bytes(b) => ((b.len() + 0xf) & !0xf).max(0x20),
        _ => 0x20,
    }).collect();
    let mut locals = vec![0u8; 0x40 + slots.iter().sum::<usize>()];
    let mut props: Vec<Vec<u8>> = Vec::with_capacity(n);
    // Offsets dos `String` montados aqui: o executor copia o local para o parâmetro da função, o nosso continua nosso.
    let mut strings: Vec<usize> = Vec::new();

    let mut off = 0x20;
    for (i, a) in args.iter().enumerate() {
        let dst = locals.as_mut_ptr().add(off);
        match a {
            Arg::Handle(inst, refcnt) => {
                (dst as *mut *mut c_void).write_unaligned(*inst);
                (dst.add(8) as *mut *mut c_void).write_unaligned(*refcnt);
            }
            Arg::Item16(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 16),
            Arg::Raw(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 16),
            Arg::Str(s) => {
                // Construtor do motor: o inline (`red_string_write_inline`) recusa mais de 19 bytes e deixava o slot vazio.
                crate::jogo::cstring::nova(dst, s);
                strings.push(off);
            }
            Arg::I32(v) => (dst as *mut u32).write_unaligned(*v),
            Arg::I64(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::F32(v) => (dst as *mut f32).write_unaligned(*v),
            Arg::Bool(v) => dst.write(if *v { 1 } else { 0 }),
            Arg::CName(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::Enum(v) => (dst as *mut u64).write_unaligned(*v),
            Arg::Tdb(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, 8),
            #[cfg(feature = "lua")]
            Arg::Bytes(b) => std::ptr::copy_nonoverlapping(b.as_ptr(), dst, b.len()),
        }
        // CProperty sintética: +0 = ptype (de pEntries[i]), +0x20 = offset no locals
        let ptype = rd_ptr(rd_ptr(p_entries.add(i * 8)) as *const u8);
        let mut cp = vec![0u8; 0x30];
        (cp.as_mut_ptr() as *mut *mut c_void).write_unaligned(ptype);
        (cp.as_mut_ptr().add(0x20) as *mut u32).write_unaligned(off as u32);
        props.push(cp);
        off += slots[i];
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

    let mut fr = frame_raiz(bc.as_mut_ptr(), locals.as_mut_ptr(), ctx);

    let exec: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(rebase(crate::jogo::scripts::EXECUTOR_VM));
    exec(
        rf.func,
        ctx,
        fr.as_mut_ptr() as *mut c_void,
        res.as_mut_ptr() as *mut c_void,
        rf.ret_type,
    );
    for off in strings {
        crate::jogo::cstring::destroi(locals.as_mut_ptr().add(off));
    }
    let mapa = frame_mapa_alocado(&fr);
    if mapa != 0 {
        crate::log_dev!("[call_func] mapa do frame raiz alocado pela VM (capacidade {mapa}): fica sem soltar");
    }
    true
}

/// Retorno `Bool` no buffer de `call_func`: o tipo tem 1 byte (`GetSize()`) e o executor entrega `res` como `aOut`
/// da função, que grava o valor no byte 0; os outros bytes do buffer não são do valor.
#[inline]
pub(crate) const fn bool_do_retorno(res: &[u8; 0x20]) -> bool {
    res[0] != 0
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
    let tb = bwms_primitives::tweak_db_id(name).to_le_bytes();
    let mut bc = vec![0u8; 16];
    bc[0] = 0x11; // push TweakDBID literal
    bc[1..9].copy_from_slice(&tb);
    bc[9] = 0x26; // ParamEnd
    // O ctx do FromTDBID real vai em fr+0x40.
    let mut fr = frame_raiz(bc.as_mut_ptr(), std::ptr::null_mut(), ctx);
    // DIAGNÓSTICO (2026-08-07): sentinela em vez de zero — distingue "a nativa escreveu
    // zero de propósito" de "a_out nunca foi tocado" (early-return dentro do exec_replacement
    // reentrante: RUST_OV_CNAME/route_native/watched_before podem suprimir antes do ORIG_EXEC real).
    let mut out = [0xEEu8; 16];
    let exec: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(rebase(crate::jogo::scripts::EXECUTOR_VM));
    exec(
        func,
        ctx,
        fr.as_mut_ptr() as *mut c_void,
        out.as_mut_ptr() as *mut c_void,
        ret,
    );
    let untouched = out.iter().all(|&b| b == 0xEE);
    crate::log_dev!(
        "[from_tdbid] '{name}' ctx={ctx:p} out={out:02x?} untouched={untouched}"
    );
    Some(out)
}

/// Lê o FromTDBID (fn/ctx/ret) que o hook do executor publicou nos atomics `FROMTD_*`.
unsafe fn read_fromtd() -> Option<(*mut c_void, *mut c_void, *mut c_void)> {
    // Captura NATIVA, dentro do processo: o hook do executor publica fn/ctx/ret do FromTDBID em
    // atomics quando o jogo o chama. Endereço lido de fora do processo seria de OUTRA sessão
    // (ASLR) = morto = crash.
    use std::sync::atomic::Ordering;
    let c = crate::inicio::boot::FROMTD_CTX.load(Ordering::Relaxed);
    if c.is_null() {
        return None; // ainda não capturado (ex.: no menu) → fallback ctx=null (serve p/ money)
    }
    let f = crate::inicio::boot::FROMTD_TGT.load(Ordering::Relaxed);
    let r = crate::inicio::boot::FROMTD_RET.load(Ordering::Relaxed);
    if f.is_null() {
        return None;
    }
    Some((f, c, r))
}

// ===== Introspecção fina de flags (`PENDENCIAS-UNIFICADAS.md`, prosa "Restante":
// "CClass::Flags 11 bits, CProperty.flags 13 bits, CBaseFunction.flags 13
// bits — maioria nunca lida/setada"). `CProperty.flags@0x28`
// já era lido (só o bit `inValueHolder`, ver `find_prop_exact`); `CBaseFunction.flags@0xA8` já
// era ESCRITO (`register.rs::build_native_func`, `FLAG_NATIVE`/`FLAG_STATIC`). O gap real é expor
// a decodificação COMPLETA dos bits nomeados, nunca feita antes pra nenhum dos 3.

/// `CClass::Flags` (11 bits nomeados + 1 campo reservado de 21 bits) @ CClass+0x70.
pub unsafe fn class_flags(cls: *mut c_void) -> Option<u32> {
    rd_u32_chk((cls as *const u8).add(0x70))
}
/// `CClass::Flags.isAbstract` (bit 0).
pub(crate) const CLASSE_FLAG_ABSTRATA: u32 = 1 << 0;
/// Flags (`class_flags`) de classe abstrata: o motor não a instancia.
pub(crate) fn classe_abstrata(flags: u32) -> bool {
    flags & CLASSE_FLAG_ABSTRATA != 0
}
#[cfg(feature = "devtools")]
pub fn decode_class_flags(f: u32) -> String {
    let bits: &[(u32, &str)] = &[
        (0, "isAbstract"),
        (1, "isNative"),
        (2, "isScriptedClass"),
        (3, "isScriptedStruct"),
        (4, "hasNoDefaultObjectSerialization"),
        (5, "isAlwaysTransient"),
        (6, "isImportOnly"),
        (7, "isPrivate"),
        (8, "isProtected"),
        (9, "isTestOnly"),
        (10, "isSavable"),
    ];
    let names: Vec<&str> = bits.iter().filter(|(b, _)| f & (1 << b) != 0).map(|(_, n)| *n).collect();
    if names.is_empty() {
        format!("(nenhuma; raw={f:#010x})")
    } else {
        format!("{} (raw={f:#010x})", names.join("|"))
    }
}

/// `CProperty::Flags` (64 bits, ~13 nomeados) @ CProperty+0x28.
#[cfg(feature = "devtools")]
pub unsafe fn property_flags(prop: *mut c_void) -> Option<u64> {
    if !crate::memoria::is_readable((prop as *const u8).add(0x28) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64((prop as *const u8).add(0x28)))
}
#[cfg(feature = "devtools")]
pub fn decode_property_flags(f: u64) -> String {
    let bits: &[(u64, &str)] = &[
        (0x05, "isScripted"),
        (0x06, "isReturn"),
        (0x08, "isLocalVar"),
        (0x09, "isOut"),
        (0x0A, "isOptional"),
        (0x0E, "isOverriding"),
        (0x10, "isPrivate"),
        (0x11, "isProtected"),
        (0x12, "isPublic"),
        (0x15, "inValueHolder"),
        (0x1B, "isHandle"),
        (0x1C, "isPersistent"),
        (0x21, "isSavable"),
    ];
    let names: Vec<&str> = bits.iter().filter(|(b, _)| f & (1u64 << b) != 0).map(|(_, n)| *n).collect();
    if names.is_empty() {
        format!("(nenhuma nomeada; raw={f:#018x})")
    } else {
        format!("{} (raw={f:#018x})", names.join("|"))
    }
}

/// `CBaseFunction::Flags` (32 bits, ~13 nomeados) @ CBaseFunction+0xA8 — MESMO offset que
/// `register.rs::build_native_func` já ESCREVE (`FLAG_NATIVE`=bit0, `FLAG_STATIC`=bit1) pras
/// nativas forjadas; aqui é o lado LEITURA, genérico pra qualquer função (nativa ou scripted).
#[cfg(feature = "devtools")]
pub unsafe fn function_flags(func: *mut c_void) -> Option<u32> {
    rd_u32_chk((func as *const u8).add(0xA8))
}
/// Item #186 (lado escrita, 2026-08-11): escreve `CBaseFunction::flags@+0xA8` — mesmo
/// campo/offset já lido/escrito com sucesso (isNative/isStatic, confirmado ao vivo 2026-08-10).
/// Campo é metadado puro (Reflection/introspecção) — despacho de chamada nunca lê nenhum destes
/// bits (nossas natives são roteadas via `NATIVE_ROUTES`, hook próprio, nunca a via nativa do
/// engine que teoricamente poderia inspecionar isEvent/isExec/isQuest/isThreadsafe).
pub unsafe fn function_flags_write(func: *mut c_void, f: u32) -> bool {
    let p = (func as *mut u8).add(0xA8);
    if !crate::memoria::is_readable(p as *const c_void, 4) {
        return false;
    }
    (p as *mut u32).write_unaligned(f);
    true
}
#[cfg(feature = "devtools")]
pub fn decode_function_flags(f: u32) -> String {
    let bits: &[(u32, &str)] = &[
        (0, "isNative"),
        (1, "isStatic"),
        (2, "isFinal"),
        (3, "isEvent"),
        (4, "isExec"),
        (7, "isPrivate"),
        (8, "isProtected"),
        (9, "isPublic"),
        (13, "isConst"),
        (14, "isQuest"),
        (15, "isThreadsafe"),
    ];
    let names: Vec<&str> = bits.iter().filter(|(b, _)| f & (1 << b) != 0).map(|(_, n)| *n).collect();
    if names.is_empty() {
        format!("(nenhuma nomeada; raw={f:#010x})")
    } else {
        format!("{} (raw={f:#010x})", names.join("|"))
    }
}

/// Comando `classflags <nome>` — resolve a classe e decodifica `CClass::Flags`.
#[cfg(feature = "devtools")]
pub unsafe fn probe_class_flags(reg: &Registry, name: &str) {
    let cls = reg.class_by_name(name);
    if cls.is_null() {
        crate::log_dev!("[classflags] classe '{name}' não resolveu");
        return;
    }
    match class_flags(cls) {
        Some(f) => crate::log_dev!("[classflags] {name}: {}", decode_class_flags(f)),
        None => crate::log_dev!("[classflags] {name}: flags@0x70 ilegível"),
    }
}

/// Comando `propflags <classe> <prop>` — resolve a propriedade e decodifica `CProperty::Flags`.
#[cfg(feature = "devtools")]
pub unsafe fn probe_property_flags(reg: &Registry, class: &str, prop: &str) {
    let p = find_property(reg, class, prop);
    if p.is_null() {
        crate::log_dev!("[propflags] {class}.{prop} não resolveu");
        return;
    }
    match property_flags(p) {
        Some(f) => crate::log_dev!("[propflags] {class}.{prop}: {}", decode_property_flags(f)),
        None => crate::log_dev!("[propflags] {class}.{prop}: flags@0x28 ilegível"),
    }
}

// ===== `CClass.defaults` (`PENDENCIAS-UNIFICADAS.md`, prosa "Restante": "valor-
// default de propriedade (CClass.defaults)") — `Map<CName, Variant*> defaults` @ CClass+0x158
// (layout de `Map<K,V>`:
// `keys:DynArray<K>@+0x00, values:DynArray<T>@+0x10, flags:i32@+0x20`, cada `DynArray` no layout
// já provado em todo o resto do projeto — `entries@+0x00, cap(u32)@+0x08, size(u32)@+0x0C`).
// Nenhuma introspecção de valor-default de propriedade existia antes — o campo nunca foi lido.

/// Varre `CClass.defaults` (varredura LINEAR — o array pode estar `NotSorted` no momento da
/// leitura externa, `Map::Sort()` só roda lazy sob demanda do próprio motor; linear é sempre
/// correto, custo irrelevante pro tamanho típico de `defaults`) procurando `prop_name`. Devolve os
/// 24 bytes brutos do `Variant` apontado (mesmo layout já usado por
/// `read_params_consuming_with_variants`/`variant_type_cname`/`variant_inline_u64`).
#[cfg(feature = "devtools")]
pub unsafe fn class_default_value(cls: *mut c_void, prop_name: &str) -> Option<[u8; 24]> {
    const DEFAULTS_OFF: usize = 0x158;
    let base = cls as *const u8;
    if !crate::memoria::is_readable(base.add(DEFAULTS_OFF) as *const c_void, 0x28) {
        return None;
    }
    let keys_entries = rd_ptr(base.add(DEFAULTS_OFF)) as *const u64; // DynArray<CName>
    let keys_size = rd_u32(base.add(DEFAULTS_OFF + 0x0C));
    let values_entries = rd_ptr(base.add(DEFAULTS_OFF + 0x10)) as *const *mut u8; // DynArray<Variant*>
    if keys_entries.is_null() || values_entries.is_null() || keys_size == 0 || keys_size > 100_000 {
        return None;
    }
    let target = cname(prop_name);
    if !crate::memoria::is_readable(keys_entries as *const c_void, keys_size as usize * 8) {
        return None;
    }
    for i in 0..keys_size as usize {
        if keys_entries.add(i).read_unaligned() == target {
            let vp = values_entries.add(i).read_unaligned();
            if vp.is_null() || !crate::memoria::is_readable(vp as *const c_void, 24) {
                return None;
            }
            let mut buf = [0u8; 24];
            std::ptr::copy_nonoverlapping(vp, buf.as_mut_ptr(), 24);
            return Some(buf);
        }
    }
    None
}

/// Comando `classdefault <classe> <prop>` — resolve a classe, busca o default em `CClass.defaults`
/// e decodifica tipo+valor-inline (mesmo decoder já usado pra `Variant` de argumento de native).
#[cfg(feature = "devtools")]
pub unsafe fn probe_class_default(reg: &Registry, class: &str, prop: &str) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log_dev!("[classdefault] classe '{class}' não resolveu");
        return;
    }
    match class_default_value(cls, prop) {
        Some(buf) => {
            let type_hash = variant_type_cname(&buf);
            let type_name = crate::cname::resolve_cname(type_hash);
            let inline_val = variant_inline_u64(&buf);
            crate::log_dev!(
                "[classdefault] {class}.{prop}: tipo='{type_name}' (hash={type_hash:#x}) valor_inline_u64={inline_val:#x} ({inline_val})"
            );
        }
        None => crate::log_dev!(
            "[classdefault] {class}.{prop}: sem default registrado (ou 'defaults' vazio/ilegível)"
        ),
    }
}

// ===== `rtti::IType::GetType() -> ERTTIType` — introspecção GENÉRICA de categoria de tipo
// (`PENDENCIAS-UNIFICADAS.md`: "família de tipos-wrapper... nenhum tem wrapper Rust dedicado...
// não cobre introspecção genérica de 'que tipo de wrapper é esse campo' (array vs handle vs
// weak-handle vs resource-ref)"). `GetType` já era chamado internamente (ver comentário em
// `props_array`/parsing de argumento de native, achado histórico do projeto: **a vtable de
// `rtti::IType` tem um shift de +0x08 no macOS vs o offset documentado no header Windows** — 2
// slots de dtor a mais no ABI Itanium (mesma família de achado de `cp77-macos-rtti-vtable-offsets`,
// "2 dtors Itanium"). `GetName`: Windows 0x08 → macOS 0x10. `GetType`: Windows 0x20 → macOS 0x28.
// Isso NÃO afeta a vtable de `IRTTISystem`/`CRTTISystem` (cross-validada extensivamente nesta
// sessão nos offsets EXATOS do header, sem shift) — o shift é específico da hierarquia `IType`.

/// `ERTTIType`: o byte que `IType::GetType()` devolve (vtable+0x28 no macOS).
pub mod ertti {
    #[cfg(feature = "devtools")]
    pub const CLASS: u8 = 2;
    pub const ARRAY: u8 = 3;
    pub const ENUM: u8 = 5;
    pub const STATIC_ARRAY: u8 = 6;
    pub const NATIVE_ARRAY: u8 = 7;
    pub const HANDLE: u8 = 9;
    pub const WEAK_HANDLE: u8 = 10;
    pub const RESOURCE_REFERENCE: u8 = 11;
    pub const RESOURCE_ASYNC_REFERENCE: u8 = 12;
    pub const SCRIPT_REFERENCE: u8 = 15;

    /// Nome de cada categoria, indexado pelo próprio valor.
    #[cfg(feature = "devtools")]
    pub const NOMES: [&str; 17] = [
        "Name", "Fundamental", "Class", "Array", "Simple", "Enum", "StaticArray", "NativeArray", "Pointer", "Handle",
        "WeakHandle", "ResourceReference", "ResourceAsyncReference", "BitField", "LegacySingleChannelCurve",
        "ScriptReference", "FixedArray",
    ];
}

/// `rtti::IType::GetType() -> ERTTIType` (macOS: vtbl+0x28) — devolve o byte cru do enum (valores em [`ertti`]).
pub unsafe fn type_kind(ty: *mut c_void) -> Option<u8> {
    if !sane(ty) || !crate::memoria::is_readable(ty as *const c_void, 8) {
        return None;
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() || !crate::memoria::is_readable(vt.add(0x28) as *const c_void, 8) {
        return None;
    }
    let f = rd_ptr(vt.add(0x28));
    if !sane(f) {
        return None;
    }
    let call: extern "C" fn(*mut c_void) -> u8 = std::mem::transmute(f);
    Some(call(ty))
}

#[cfg(feature = "devtools")]
pub fn decode_ertti_type(k: u8) -> &'static str {
    ertti::NOMES.get(k as usize).copied().unwrap_or("?")
}

/// `ReflectionType.GetInnerType()` (2026-08-10) — "o elemento interno de um tipo
/// composto" (array/handle/weak-handle: `T` de `array<T>`/`ref<T>`/`wref<T>`). Achado que evita
/// vtable por completo: `CRTTIHandleType`/`CRTTIWeakHandleType`/`CRTTIBaseArrayType`
/// TODOS guardam `innerType: rtti::IType*` como CAMPO DE DADOS no MESMO offset fixo (`+0x10`, logo
/// após o ponteiro de vtable) — não precisa chamar `GetInnerType()` (vtable+0xC0, exigiria a
/// correção de shift Itanium já documentada pra `IType`). Campo de dado NÃO sofre esse shift (só
/// slots de vtable sofrem, por causa dos 2 dtors extra do ABI Itanium) — leitura direta é EXATA
/// em Windows e Mac igualmente, zero risco de chamar a função errada. Só válido pra `type_kind()`
/// em {3=Array,6=StaticArray,7=NativeArray,9=Handle,10=WeakHandle} — outras categorias têm outro
/// dado em `+0x10` (ex. `Simple`/`Fundamental` não têm innerType nenhum).
pub unsafe fn get_inner_type(ty: *mut c_void) -> Option<*mut c_void> {
    let k = type_kind(ty)?;
    if !matches!(k, 3 | 6 | 7 | 9 | 10) {
        return None;
    }
    if !crate::memoria::is_readable((ty as *const u8).add(0x10) as *const c_void, 8) {
        return None;
    }
    let inner = rd_ptr((ty as *const u8).add(0x10));
    if sane(inner) { Some(inner) } else { None }
}

/// Comando `innertype <typeName>` — resolve um IType por nome (via `get_type`, ex.
/// `"array:Float"`/`"handle:IScriptable"`) e mostra a categoria + o nome do tipo interno.
#[cfg(feature = "devtools")]
pub unsafe fn probe_inner_type(reg: &Registry, type_name: &str) {
    let ty = crate::register::get_type(reg, type_name);
    if !sane(ty) {
        crate::log_dev!("[innertype] GetType('{type_name}') não resolveu");
        return;
    }
    let kind = type_kind(ty).map(decode_ertti_type).unwrap_or("?");
    match get_inner_type(ty) {
        Some(inner) => {
            let name_hash = type_name_getname(inner);
            let name = crate::cname::resolve_cname(name_hash);
            crate::log_dev!("[innertype] '{type_name}' categoria={kind} innerType='{name}'");
        }
        None => crate::log_dev!("[innertype] '{type_name}' categoria={kind} sem innerType (tipo não-composto ou leitura falhou)"),
    }
}

/// Comando `proptypekind <classe> <prop>` — resolve a propriedade, lê `CProperty.type@+0x00`
/// (`rtti::IType*`) e decodifica sua categoria via `GetType()`. Responde exatamente a pergunta
/// "que tipo de wrapper é esse campo" (array/handle/weak-handle/resource-ref/etc.) sem precisar
/// de um wrapper dedicado por família — um único mecanismo genérico cobre as ~9 categorias.
#[cfg(feature = "devtools")]
pub unsafe fn probe_property_type_kind(reg: &Registry, class: &str, prop: &str) {
    let p = find_property(reg, class, prop);
    if p.is_null() {
        crate::log_dev!("[proptypekind] {class}.{prop} não resolveu");
        return;
    }
    let ty = rd_ptr(p as *const u8); // CProperty.type@+0x00
    match type_kind(ty) {
        Some(k) => crate::log_dev!(
            "[proptypekind] {class}.{prop}: categoria={} (raw={k})",
            decode_ertti_type(k)
        ),
        None => crate::log_dev!("[proptypekind] {class}.{prop}: type@+0x00 ilegível ou GetType falhou"),
    }
}

/// Comando `proptypekindall <classe> [n]` — lista as primeiras `n` (default 20) props PRÓPRIAS da
/// classe (via `class_own_properties`, já provado) com nome+categoria — valida `type_kind` contra
/// VÁRIAS categorias reais de uma vez (não só 1 propriedade escolhida à mão).
#[cfg(feature = "devtools")]
pub unsafe fn probe_property_type_kind_all(reg: &Registry, class: &str, n: usize) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log_dev!("[proptypekindall] classe '{class}' não resolveu");
        return;
    }
    let props = class_own_properties(cls);
    crate::log_dev!("[proptypekindall] {class}: {} prop(s) próprias, mostrando até {n}", props.len());
    for p in props.iter().take(n) {
        let name_hash = rd_u64((*p as *const u8).add(0x08));
        let name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
        let ty = rd_ptr(*p as *const u8);
        let kind = type_kind(ty).map(decode_ertti_type).unwrap_or("?");
        crate::log_dev!("[proptypekindall]   {name}: {kind}");
    }
}

/// `ResourceHelper.IsReferenceLoaded`/`GetReferenceResource`: `proptypekindall`
/// só varre 1 classe por vez — nunca achou um `type_kind()==11` (`ResourceReference`/`Ref<T>`,
/// distinto do `type_kind()==12`/`RaRef<T>` já testado em `mesh`/`rig`) porque ninguém tentou
/// achar um em escala. Comando `scan11 <âncora> [max_classes]` — BFS pela hierarquia via
/// `get_derived_classes` (já provado, só 1 nível por chamada — por isso o BFS, não recursão
/// nativa) checando CADA propriedade própria de CADA classe visitada. Bounded por `max_classes`
/// (leituras de memória puras + 1 chamada de vtable por prop via `type_kind`, já provado seguro
/// em `proptypekindall`; nunca executa lógica de gameplay) — pra num boot só encontrar um alvo
/// real em vez de adivinhar nome de classe.
pub unsafe fn scan_type_kind_across_hierarchy(reg: &Registry, anchor: &str, max_classes: usize) {
    let root = reg.class_by_name(anchor);
    if root.is_null() {
        crate::log_dev!("[scan11] âncora '{anchor}' não resolveu");
        return;
    }
    const MAX_HITS: usize = 40;
    let mut queue: std::collections::VecDeque<*mut c_void> = std::collections::VecDeque::new();
    queue.push_back(root);
    let mut visited: Vec<*mut c_void> = Vec::new();
    let mut hits: Vec<String> = Vec::new();
    let mut classes_scanned = 0usize;

    while let Some(cls) = queue.pop_front() {
        if visited.contains(&cls) {
            continue;
        }
        if classes_scanned >= max_classes {
            break;
        }
        visited.push(cls);
        classes_scanned += 1;

        let cls_hash = type_name_getname(cls);
        let cls_name = crate::cname::name_of(cls_hash).unwrap_or_else(|| format!("(hash={cls_hash:#x})"));

        for p in class_own_properties(cls).iter() {
            let ty = rd_ptr(*p as *const u8);
            if ty.is_null() {
                continue;
            }
            if let Some(ertti::RESOURCE_REFERENCE) = type_kind(ty) {
                let name_hash = rd_u64((*p as *const u8).add(0x08));
                let prop_name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
                hits.push(format!("{cls_name}.{prop_name}"));
            }
        }
        if hits.len() >= MAX_HITS {
            break;
        }

        let (entries, count) = reg.get_derived_classes(cls);
        if !entries.is_null() {
            let arr = entries as *const *mut c_void;
            for i in 0..count {
                if let Some(d) = rd_ptr_chk(arr.add(i) as *const u8) {
                    if !d.is_null() && !visited.contains(&d) {
                        queue.push_back(d);
                    }
                }
            }
        }
    }

    crate::log_dev!(
        "[scan11] âncora='{anchor}' classes escaneadas={classes_scanned} hits type_kind==11(ResourceReference)={} : {hits:?}",
        hits.len()
    );
}

/// `Reflection.ReflectionFunc` — "introspecção de assinatura" (`PENDENCIAS-UNIFICADAS.md`,
/// prosa "Reflection... introspecção de assinatura"). `GetReturnType`/`GetParameters`/`IsNative`/`IsStatic` são só
/// leituras de campos JÁ documentados neste projeto: `returnType->type` (= `ResolvedFn.ret_type`,
/// já existe), `params: DynArray<CProperty*>@+0x28` (nunca listado, só usado indiretamente),
/// `flags.isNative/isStatic` (= `function_flags`/`decode_function_flags`, fechado nesta mesma
/// sessão). Comando `funcsig <classe> <método>` — junta tudo numa única dump.
#[cfg(feature = "devtools")]
pub unsafe fn probe_function_signature(reg: &Registry, class: &str, method: &str) {
    let rf = match resolve_func(reg, class, method) {
        Some(rf) => rf,
        None => {
            crate::log_dev!("[funcsig] {class}.{method} não resolveu (nem na cadeia de parents)");
            return;
        }
    };
    let flags_s = function_flags(rf.func).map(decode_function_flags).unwrap_or_else(|| "?".into());
    let ret_kind = if rf.ret_type.is_null() {
        "Void".to_string()
    } else {
        type_kind(rf.ret_type).map(decode_ertti_type).unwrap_or("?").to_string()
    };
    crate::log_dev!(
        "[funcsig] {class}.{method}: flags={flags_s} is_static(walk)={} retorno={ret_kind}",
        rf.is_static
    );
    // params: DynArray<CProperty*> @ func+0x28 (mesmo layout de props_array — entries@0,cap@8,size@0xC).
    let base = rf.func as *const u8;
    if !crate::memoria::is_readable(base.add(0x28) as *const c_void, 0x10) {
        crate::log_dev!("[funcsig]   params@+0x28 ilegível");
        return;
    }
    let entries = rd_ptr(base.add(0x28)) as *const *mut c_void;
    let size = rd_u32(base.add(0x34));
    if entries.is_null() || size > 256 {
        crate::log_dev!("[funcsig]   {size} parâmetro(s) — array inválido ou vazio");
        return;
    }
    crate::log_dev!("[funcsig]   {size} parâmetro(s):");
    for i in 0..size as usize {
        let Some(prop) = rd_ptr_chk(entries.add(i) as *const u8) else { break };
        if prop.is_null() {
            continue;
        }
        let name_hash = rd_u64((prop as *const u8).add(0x08));
        let pname = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
        let pty = rd_ptr(prop as *const u8); // CProperty.type@+0x00
        let pkind = type_kind(pty).map(decode_ertti_type).unwrap_or("?");
        crate::log_dev!("[funcsig]     [{i}] {pname}: {pkind}");
    }
}

/// Comando `funcflags <classe> <método>` — resolve o método (sobe a cadeia de parents) e
/// decodifica `CBaseFunction::Flags`.
#[cfg(feature = "devtools")]
pub unsafe fn probe_function_flags(reg: &Registry, class: &str, method: &str) {
    match resolve_func(reg, class, method) {
        Some(rf) => match function_flags(rf.func) {
            Some(f) => crate::log_dev!(
                "[funcflags] {class}.{method}: {} (is_static resolvido pelo walk={})",
                decode_function_flags(f),
                rf.is_static
            ),
            None => crate::log_dev!("[funcflags] {class}.{method}: flags@0xA8 ilegível"),
        },
        None => crate::log_dev!("[funcflags] {class}.{method} não resolveu (nem na cadeia de parents)"),
    }
}

/// Itens #284/#280 (2026-08-11): `DynArray<T>::Erase`/`RemoveAt`/`Assign` — genérico,
/// zero endereço nativo (`Erase` é lógica auto-contida, `ShiftEntries` = memmove puro, sem
/// chamada nativa nenhuma). Opera no layout
/// `{entries(ptr)@0x00, capacity(u32)@0x08, size(u32)@0x0C}` — o MESMO já batalha-testado por
/// `mkarr`/`get_records_of_class`/`class_own_functions`/`Entity.GetComponents` neste projeto.
/// `header_ptr` aponta pro início desse struct de 16 bytes; `stride` = bytes por elemento;
/// `index` = posição a remover (0-based). Devolve `false` se `index>=size` (mesmo contrato de
/// `RemoveAt`, que valida o índice antes de chamar `Erase`).
pub unsafe fn dynarray_remove_at(header_ptr: *mut u8, stride: usize, index: usize) -> bool {
    if header_ptr.is_null() || !crate::memoria::is_readable(header_ptr as *const c_void, 0x10) {
        return false;
    }
    let entries = (header_ptr as *const u64).read_unaligned() as *mut u8;
    let cap = (header_ptr.add(8) as *const u32).read_unaligned();
    let size = (header_ptr.add(12) as *const u32).read_unaligned();
    if entries.is_null() || !crate::register::dynarray_coerente(entries as usize, size, cap) || index as u32 >= size {
        return false;
    }
    let tail_count = size as usize - index - 1;
    if tail_count > 0 {
        // ShiftEntries: move os `tail_count` elementos de [index+1..size) pra [index..size-1).
        let src = entries.add((index + 1) * stride);
        let dst = entries.add(index * stride);
        core::ptr::copy(src, dst, tail_count * stride); // ranges podem se sobrepor -> copy (memmove)
    }
    core::ptr::write_unaligned(header_ptr.add(12) as *mut u32, size - 1);
    true
}


#[cfg(test)]
mod dynarray_remove_at_tests {
    use super::dynarray_remove_at;

    fn cabecalho(entries: *mut u8, capacity: u32, size: u32) -> [u8; 16] {
        let mut h = [0u8; 16];
        h[0..8].copy_from_slice(&(entries as u64).to_le_bytes());
        h[8..12].copy_from_slice(&capacity.to_le_bytes());
        h[12..16].copy_from_slice(&size.to_le_bytes());
        h
    }

    /// O crivo é o de `register::dynarray_coerente`: cabeçalho incoerente não é tocado, nem o `size` nem os elementos.
    /// A capacidade acima de `1<<20` com `size` pequeno é o caso que só o crivo único recusa.
    #[test]
    fn recusa_cabecalho_incoerente_sem_escrever() {
        let mut buf = [10i32, 20, 30, 40];
        let e = buf.as_mut_ptr() as *mut u8;
        for (entries, cap, size) in [(e, 3, 4), (std::ptr::null_mut(), 4, 2), (e, (1 << 20) + 1, 4)] {
            let mut h = cabecalho(entries, cap, size);
            let antes = h;
            assert!(!unsafe { dynarray_remove_at(h.as_mut_ptr(), 4, 0) }, "cap={cap} size={size}");
            assert_eq!(h, antes, "cap={cap} size={size}");
        }
        assert_eq!(buf, [10, 20, 30, 40]);
    }

    #[test]
    fn remove_no_meio_e_encolhe() {
        let mut buf = [10i32, 20, 30, 40, 50];
        let mut h = cabecalho(buf.as_mut_ptr() as *mut u8, 8, 5);
        assert!(unsafe { dynarray_remove_at(h.as_mut_ptr(), 4, 2) });
        assert_eq!(u32::from_le_bytes([h[12], h[13], h[14], h[15]]), 4);
        assert!(!unsafe { dynarray_remove_at(h.as_mut_ptr(), 4, 4) }, "index >= size");
        assert_eq!(buf[..4], [10, 20, 40, 50]);
    }
}

#[cfg(test)]
mod cstring_tests {
    use super::{read_cstring, red_string_read};

    #[test]
    fn red_string_read_recusa_inline_maior_que_o_objeto() {
        let mut buf = make_inline_cstring("oi");
        buf[0x14..0x18].copy_from_slice(&0x30u32.to_le_bytes());
        unsafe {
            assert_eq!(red_string_read(buf.as_ptr()), "");
            assert_eq!(red_string_read(make_inline_cstring("oi").as_ptr()), "oi");
        }
    }

    /// Monta um buffer de 0x20 bytes no layout do `CString` (SSO inline): os primeiros
    /// `s.len()` bytes de `text.inline_str`, resto zerado, `length` (sem a flag 0x40000000) em
    /// +0x14, `allocator` (irrelevante pra leitura) em +0x18.
    fn make_inline_cstring(s: &str) -> [u8; 0x20] {
        let mut buf = [0u8; 0x20];
        let bytes = s.as_bytes();
        assert!(bytes.len() <= 0x14, "teste só cobre o caso inline (<=20 bytes)");
        buf[..bytes.len()].copy_from_slice(bytes);
        buf[0x14..0x18].copy_from_slice(&(bytes.len() as u32).to_le_bytes()); // length, bit30=0
        buf
    }

    /// Monta um buffer heap-alocado: `text.str.ptr` aponta pro `heap_buf` externo, `length` com
    /// a flag 0x40000000 (não-inline) OR o tamanho real.
    fn make_heap_cstring(heap_ptr: *const u8, len: usize) -> [u8; 0x20] {
        let mut buf = [0u8; 0x20];
        buf[0x00..0x08].copy_from_slice(&(heap_ptr as u64).to_le_bytes()); // text.str.ptr
        let length_raw = 0x4000_0000u32 | (len as u32);
        buf[0x14..0x18].copy_from_slice(&length_raw.to_le_bytes());
        buf
    }

    #[test]
    fn le_string_inline_curta() {
        let buf = make_inline_cstring("oi");
        unsafe {
            assert_eq!(read_cstring(buf.as_ptr()), Some("oi".to_string()));
        }
    }

    #[test]
    fn le_string_inline_no_limite_20_bytes() {
        let s = "12345678901234567890"[..20].to_string(); // exatamente 0x14 bytes
        let buf = make_inline_cstring(&s);
        unsafe {
            assert_eq!(read_cstring(buf.as_ptr()), Some(s));
        }
    }

    #[test]
    fn le_string_heap_alocada_maior_que_sso() {
        let heap_data = b"esta string e maior que 20 bytes com certeza".to_vec();
        let buf = make_heap_cstring(heap_data.as_ptr(), heap_data.len());
        unsafe {
            assert_eq!(
                read_cstring(buf.as_ptr()),
                Some(String::from_utf8(heap_data).unwrap())
            );
        }
    }

    #[test]
    fn string_vazia() {
        let buf = make_inline_cstring("");
        unsafe {
            assert_eq!(read_cstring(buf.as_ptr()), Some(String::new()));
        }
    }

    #[test]
    fn ponteiro_nulo_e_seguro() {
        unsafe {
            assert_eq!(read_cstring(std::ptr::null()), None);
        }
    }
}

/// Item #55 — núcleo de decisão de nome do `resolve_global_function_robust`, testado
/// offline (sem RTTI/jogo). O gap real: uma global dentro de `module X.Y` não é achada pelo nome
/// BARE, porque a busca por hash compara `name`/`fullName` já qualificados.
#[cfg(test)]
mod global_fn_name_tests {
    use super::global_fn_name_matches;

    #[test]
    fn exato_sem_modulo_continua_casando() {
        assert!(global_fn_name_matches("BwmsFNV1a64", "BwmsFNV1a64"));
    }

    #[test]
    fn nome_bare_acha_global_com_module_scope() {
        // O CASO DO GAP: antes disto, pedir o nome curto não achava nada.
        assert!(global_fn_name_matches("MinhaFunc", "MeuModulo.MinhaFunc"));
        assert!(global_fn_name_matches("GetText", "MeuModulo.Localization.GetText"));
    }

    #[test]
    fn nome_curto_diferente_nao_casa() {
        assert!(!global_fn_name_matches("OutraFunc", "MeuModulo.MinhaFunc"));
    }

    #[test]
    fn nao_casa_prefixo_parcial_do_segmento() {
        // "Func" não pode casar "MeuModulo.MinhaFunc" — é comparação de segmento INTEIRO.
        assert!(!global_fn_name_matches("Func", "MeuModulo.MinhaFunc"));
        assert!(!global_fn_name_matches("Minha", "MeuModulo.MinhaFunc"));
    }

    #[test]
    fn assinatura_decorada_e_ignorada() {
        // O RTTI do jogo decora overload com `;<tipos>` — cortar antes de comparar.
        assert!(global_fn_name_matches("MinhaFunc", "MeuModulo.MinhaFunc;Int32Float"));
        assert!(global_fn_name_matches("MinhaFunc", "MinhaFunc;String"));
        assert!(global_fn_name_matches("MinhaFunc;String", "MinhaFunc;String"));
    }

    #[test]
    fn pedir_qualificado_nao_aceita_bare_de_outro_escopo() {
        // Direção deliberadamente NÃO suportada: quem pede qualificado está sendo específico.
        assert!(!global_fn_name_matches("MeuModulo.MinhaFunc", "MinhaFunc"));
        assert!(!global_fn_name_matches("MeuModulo.MinhaFunc", "OutroModulo.MinhaFunc"));
    }

    #[test]
    fn modulo_aninhado_usa_o_ultimo_segmento() {
        assert!(global_fn_name_matches("Fn", "A.B.C.Fn"));
        assert!(!global_fn_name_matches("C.Fn", "A.B.C.Fn"));
    }

    #[test]
    fn vazio_nunca_casa() {
        assert!(!global_fn_name_matches("", "MinhaFunc"));
        assert!(!global_fn_name_matches("MinhaFunc", ""));
        assert!(!global_fn_name_matches("", ""));
    }
}

#[cfg(test)]
mod leitor_de_params_tests {
    use super::{bool_do_retorno, bool_do_script, legivel_na_pagina_provada, handler_de_opcode_legivel, ArgsLidos, HANDLER_DE_OPCODE_LEGIVEL, PAGINA_DO_ARM64};
    use std::sync::atomic::Ordering;

    #[test]
    fn bool_do_script_le_so_o_byte_baixo() {
        assert!(bool_do_script(1));
        assert!(!bool_do_script(0));
        assert!(!bool_do_script(0xffff_ffff_ffff_ff00), "lixo nos bytes 1-7 não liga o Bool");
        assert!(bool_do_script(0xffff_ffff_ffff_ff01));
    }

    #[test]
    fn bool_do_retorno_le_so_o_byte_0() {
        let mut res = [0u8; 0x20];
        assert!(!bool_do_retorno(&res));
        res[1..].fill(0xff);
        assert!(!bool_do_retorno(&res), "bytes 1-0x1f não ligam o Bool");
        res[0] = 1;
        assert!(bool_do_retorno(&res));
    }

    #[test]
    fn args_lidos_para_em_16_sem_estourar() {
        let mut a = ArgsLidos::vazio();
        for i in 0..20u64 {
            a.push((i, i));
        }
        assert_eq!(a.len(), 16);
        assert_eq!(a.first(), Some(&(0, 0)));
        assert_eq!(a.get(15), Some(&(15, 15)));
        assert_eq!(a.get(16), None);
    }

    #[test]
    fn bytecode_na_pagina_ja_provada_nao_repete_o_syscall() {
        let buf = vec![0u8; 2 * PAGINA_DO_ARM64];
        let p = buf.as_ptr();
        let mut provada = usize::MAX;
        assert!(unsafe { legivel_na_pagina_provada(p, 1, &mut provada) });
        assert_eq!(provada, p as usize & !(PAGINA_DO_ARM64 - 1));
        // Página marcada como provada responde sem consultar a memória: é o atalho que a chamada usa.
        assert!(unsafe { legivel_na_pagina_provada(p.wrapping_add(1), 1, &mut provada) });
        assert!(!unsafe { legivel_na_pagina_provada(std::ptr::null(), 1, &mut provada) });
    }

    #[test]
    fn handler_de_opcode_guardado_por_valor() {
        let codigo = handler_de_opcode_guardado_por_valor as fn() as usize;
        // Opcode alto, que o teste não divide com outro: a tabela é global.
        assert!(unsafe { handler_de_opcode_legivel(0xFE, codigo) });
        assert_eq!(HANDLER_DE_OPCODE_LEGIVEL[0xFE].load(Ordering::Relaxed), codigo);
        assert!(!unsafe { handler_de_opcode_legivel(0xFE, 0) });
        assert_eq!(HANDLER_DE_OPCODE_LEGIVEL[0xFE].load(Ordering::Relaxed), codigo, "handler nulo não apaga a prova");
    }
}

#[cfg(test)]
mod funcoes_resolvidas_tests {
    use super::{cname, esquecer_funcoes_resolvidas, resolve_in_class};

    /// Classe falsa com o layout que a busca lê: array de funções em `+0x48` (capacidade `+0x50`, tamanho `+0x54`), pai em `+0x10`;
    /// cada função com o CName em `+0x10` e o ponteiro do retorno em `+0x18`.
    struct ClasseFalsa {
        classe: Box<[u64; 16]>,
        funcoes: Box<[u64; 1]>,
        funcao: Box<[u64; 8]>,
    }

    fn classe_com_metodo(nome: &str) -> ClasseFalsa {
        let mut c = ClasseFalsa { classe: Box::new([0; 16]), funcoes: Box::new([0; 1]), funcao: Box::new([0; 8]) };
        c.funcao[2] = cname(nome);
        c.funcoes[0] = c.funcao.as_ptr() as u64;
        c.classe[0x48 / 8] = c.funcoes.as_ptr() as u64;
        c.classe[0x50 / 8] = (1u64 << 32) | 1;
        c
    }

    #[test]
    fn segunda_busca_vem_do_cache_e_esquecer_refaz() {
        let mut c = classe_com_metodo("BwmsTesteDoCache");
        let cls = c.classe.as_mut_ptr() as *mut std::ffi::c_void;
        let achada = unsafe { resolve_in_class(cls, "BwmsTesteDoCache") }.expect("acha na cadeia");
        assert_eq!(achada.func as u64, c.funcao.as_ptr() as u64);
        // Com a classe "esvaziada", só o cache ainda responde.
        c.classe[0x50 / 8] = 0;
        assert!(unsafe { resolve_in_class(cls, "BwmsTesteDoCache") }.is_some());
        esquecer_funcoes_resolvidas();
        assert!(unsafe { resolve_in_class(cls, "BwmsTesteDoCache") }.is_none());
    }

    #[test]
    fn falha_nao_fica_guardada() {
        let mut c = classe_com_metodo("BwmsOutroNome");
        let cls = c.classe.as_mut_ptr() as *mut std::ffi::c_void;
        assert!(unsafe { resolve_in_class(cls, "BwmsTesteDaFalha") }.is_none());
        c.funcao[2] = cname("BwmsTesteDaFalha");
        assert!(unsafe { resolve_in_class(cls, "BwmsTesteDaFalha") }.is_some());
    }
}

/// Refcount fake alto (não libera o handle) — sentinela 0x00100000_00100000 dele.
pub(crate) unsafe fn refcnt() -> *mut c_void {
    static mut REFCNT: u64 = 0x0010_0000_0010_0000;
    std::ptr::addr_of_mut!(REFCNT) as *mut c_void
}

