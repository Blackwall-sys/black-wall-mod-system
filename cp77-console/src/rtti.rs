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
/// `Handle<T>::Handle(T* aPtr)` — construtor UNIVERSAL de `ref<T>` do próprio motor (RE
/// 2026-07-18, sessão `handle-ctor-re`, fechando o bloqueador nº2 de `cw-callback-handler`
/// documentado em `register.rs::write_handle_ret`/HISTORICO cont.100). Achado por vizinhança:
/// a rotina de RELEASE (teardown de locais `ref<>` compilados, já conhecida em `0x1021048c4`)
/// tem essa função-companheira de CONSTRUÇÃO logo ANTES no mesmo objeto/arquivo — mesmo padrão
/// `ldaddal` no mesmo campo (`block+4`), decoder completo em `0x102104788..0x102104864`.
/// CONFIRMADO por 4045 call-sites reais (scan de `bl` no `__TEXT` inteiro) — o padrão em CADA
/// um é idêntico ao de `0x10093f8c4`: `<constrói o objeto cru>; mov x1,x0 (raw ptr); add
/// x0,sp,#N (slot de 16 bytes); bl 0x102104788`. ABI: `void Handle_ctor(void* aOutHandle /*x0,
/// 16 bytes*/, void* aPtr /*x1*/)`. Semântica lida da disasm: sempre escreve `out[0]=aPtr`
/// primeiro (+`out[8]=0`); se `aPtr==null` retorna aí — handle nulo válido. Senão consulta uma
/// tabela de "weak owner" por PONTEIRO (`bl 0x102186638`): se já existe um refcount-block pra
/// este objeto (outro Handle vivo apontando pra ele), REUSA o bloco e `ldaddal` +1 no contador
/// (`block+0`, compartilhado — correto p/ múltiplos Handles ao MESMO objeto, ex. `self` sendo
/// devolvido em cadeia); senão ALOCA 8 bytes do pool `red::memory::PoolStorageProxy<PoolRefCount>
/// ::Allocate` (achado nos símbolos demangled — MESMO pool nomeado "PoolRefCount"), inicializa
/// `{1,1}` (2×u32: campo+0=1, campo+4=1 — o `+4` bate EXATO com o offset que a rotina de release
/// decrementa via `ldaddal`) e REGISTRA o par `(aPtr,bloco)` na tabela de weak-owner (`bl
/// 0x1021868f8`) pra Handles futuros ao MESMO ponteiro reusarem o mesmo bloco. É a rotina REAL
/// que o motor chama em TODO `new T()`/factory que devolve `ref<T>` — não uma reimplementação
/// nossa do refcount.
const ADDR_HANDLE_CTOR: u64 = 0x1_0210_4788;
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
    read_params_inner_ext(func, frame, consume, None, None)
}

/// Núcleo de `read_params_inner` + captura OPCIONAL de valores `String` (2026-07-13, fecha
/// `cw-utils` Hash.reds/Number.reds). `strings_out[i]` = `Some(texto)` se o param `i` é String
/// e leu OK; senão `None`. Passar `None` (via `read_params_inner`) mantém o comportamento
/// EXATO de antes — nenhum caller existente muda.
unsafe fn read_params_inner_ext(
    func: *mut c_void,
    frame: *mut c_void,
    consume: bool,
    strings_out: Option<&mut Vec<Option<String>>>,
    variants_out: Option<&mut Vec<Option<[u8; 24]>>>,
) -> Vec<(u64, u64)> {
    read_params_inner_ext3(func, frame, consume, strings_out, variants_out, None, None, None)
}

/// Como `read_params_inner_ext`, com uma 4ª captura OPCIONAL: `scriptrefs_out[i]` = ponteiro cru
/// `T*` (RED4ext.SDK `#323`, `ScriptRef<T>{...ref: T*@0x18...}`) quando o param `i` é
/// `script_ref<T>` (ertti==15), pra QUALQUER `T` — generaliza o caso já provado só pra `String`
/// (`scriptref_data_ptr` local, usado por `read_cstring` abaixo). Item Codeware `#94`/`#95`
/// (2026-08-12, round 5): permite ESCREVER campos dentro do `T*` real (não só ler `String`),
/// reusando a MESMA leitura de `ScriptRef` já confirmada — `inkWidgetRef.Set`/
/// `inkWidgetLibraryResource.SetPath` (Codeware, `@addMethod` ESTÁTICO com `self: script_ref<T>`,
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
) -> Vec<(u64, u64)> {
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
        // RED4ext #323 (2026-08-10): quando ertti==15, `pinst` agora contém o wrapper
        // `ScriptRef<T> { unk00[0x10], innerType: IType*@0x10, ref: T*@0x18, hash: CName@0x20 }`
        // (RED4ext.SDK NativeTypes.hpp:404 + RTTITypes.hpp::CRTTIScriptReferenceType, offsets
        // confirmados no header). `ref@0x18` é o PONTEIRO pro dado real por trás do
        // `script_ref<T>` — usado abaixo só pra extrair `String` (o caso concreto documentado
        // em `Utils/Hash.reds`/`Number.reds`: `FNV1a64(data: script_ref<String>, ...)`).
        // Generalizar pra outros T (Int32/Float/...) fica pra quando surgir caso de uso real.
        let scriptref_data_ptr = if ertti == 15 && crate::gum::is_readable((pinst as *const u8).add(0x18) as *const c_void, 8) {
            rd_ptr((pinst as *const u8).add(0x18))
        } else {
            std::ptr::null_mut()
        };
        // lê 8 bytes de pinst+0 quando o "valor" cabe em um qword: ESCALAR (CName/Int/Float/
        // Bool/...) OU Handle(9)/WeakHandle(10) — nestes pinst+0 = PONTEIRO do objeto, que o
        // decode_arg embrulha em Handle (ex.: target:ref<ListItemController> do OnMenuItemActivated,
        // que o NativeSettings usa p/ setar fromMods). String/array/ScriptRef/struct = 0
        // (o callback lê do objeto destino). O objeto do Handle vive além do free (dono = menu).
        // `ResRef` (2026-07-24): confirmado via RED4ext.SDK vendorizado que é um struct de 8 bytes
        // achatado envolvendo só um `ResourcePath` (u64, o mesmo FNV-1a64 que `resource.link`/
        // `bwms-hashes::resource_path_hash` já usam em todo o projeto) — categoria idêntica a
        // TweakDBID/EntityID (struct achatado de 1 qword), NÃO parente de String/CString (essa é
        // 0x20 bytes, SSO+ptr+len+allocator). Ler como escalar aqui é seguro (`type_inst_alloc` já
        // aloca pelo tamanho REAL do tipo via vtable, não um buffer fixo) — só ainda não é ÚTIL pro
        // lado de ESCREVER/devolver um ResRef novo (RE separada, documentada em `register.rs`
        // `register_resourceevent`).
        // 2026-08-09 (achado ao vivo, `codeware-casts-smoke.reds`): CRUID/NodeRef são wrappers
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
            // 2026-08-10 (diagnóstico ao vivo pendente, `codeware-hashtoresref-smoke.reds`):
            // `ResRef` sozinho leu `raw=0` pra um param real — candidato, mesma categoria do bug
            // já achado 2x nesta whitelist (nome redscript-facing != nome interno do motor, ver
            // EntityID/entEntityID acima). Nome do SDK vendorizado (`ResourceReferenceScriptToken.
            // hpp`) pro tipo C++ real por trás de `ResRef`.
            || tc == cname("redResourceReferenceScriptToken")
            || tc == cname("CRUID")
            || tc == cname("NodeRef")
            // 2026-08-12 (Codeware `#22`/`EngineTime.hpp`, rodada de composição barata): `EngineTime`
            // é `importonly struct` VANILLA (não invenção do Codeware — usado em dezenas de scripts
            // reais, `orphans.script:22270`, ex. `GameInstance.GetEngineTime()`), achatado num único
            // `ticks: uint64_t` (`App/Engine/EngineTimeEx.hpp::ToTicks` = `return aTime.ticks;`) —
            // mesma categoria "struct de 1 qword" de CRUID/NodeRef/TweakDBID/EntityID/ResRef acima.
            // NUNCA confirmado ao vivo (sessão offline, sem boot) — pelo padrão já achado 2x nesta
            // MESMA whitelist (EntityID/ResRef), o nome interno real do `GetType()` PODE divergir do
            // nome redscript-facing; aqui é um tipo genuinamente vanilla (não um alias inventado pelo
            // SDK do Codeware como os outros 2 casos), então a hipótese de bater é mais forte, mas
            // ainda não é certeza — confirmar contra log real (`arg_tc_name`) na próxima sessão com
            // boot antes de contar como fechado.
            || tc == cname("EngineTime");
        let read8 = escalar || ertti == 9 || ertti == 10 || ertti == 5;
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
            if n > 0 && crate::gum::is_readable(pinst as *const c_void, n) {
                let mut buf = [0u8; 8];
                std::ptr::copy_nonoverlapping(pinst as *const u8, buf.as_mut_ptr(), n);
                u64::from_le_bytes(buf)
            } else {
                0
            }
        } else {
            0
        };
        out.push((raw, tc));
        if let Some(srefs) = scriptrefs_out.as_deref_mut() {
            srefs.push(scriptref_data_ptr as u64);
        }
        if let Some(strs) = strings_out.as_deref_mut() {
            let s = if tc == cname("String") {
                read_cstring(pinst as *const u8)
            } else if tc == cname("LocalizationString") {
                // Codeware `#66` (LocalizationString.reds, candidato 6ª rodada 2026-08-11): a
                // fonte real (`RED4ext.SDK/NativeTypes.hpp:329-334`) confirma o struct
                // `LocalizationString{int64_t unk00; CString unk08;}` (RED4EXT_ASSERT_SIZE=0x28) —
                // um `int64` seguido do MESMO layout de `CString` (0x20 bytes) já lido em
                // `pinst` pra qualquer param `String`. `type_inst_alloc(ptype)` já aloca `pinst`
                // pelo TAMANHO REAL do tipo (via vtable GetSize, não um buffer fixo) — pra
                // LocalizationString isso são os 0x28 bytes inteiros, então basta ler o CString
                // embutido em `pinst+0x08` (mesmo idioma já usado abaixo pro `scriptref_data_ptr`,
                // que também lê de um offset DENTRO de outro ponteiro). Confirmado que o tipo é
                // genuinamente redscript-visível (não Codeware-injetado): `orphans.script:21668`
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
        // Codeware `#76`/`StaticEntitySpec.position`+`.orientation` (2026-08-15, rodada 10):
        // captura os 16 bytes crus de um param `Vector4`/`Quaternion` — MESMO padrão já provado
        // pra `Variant`(24B, acima)/`CString`(0x20B, `read_cstring`): `pinst` já é alocado pelo
        // TAMANHO REAL do tipo (`type_inst_alloc`, via vtable `GetSize`, CONFIRMADO 16 pra Vector4
        // desde 2026-06-20, ver comentário de `new_object` acima) e populado pelo MESMO dispatch
        // de opcode (`h(ctx,frame,pinst,scriptref_out)`) que QUALQUER outro param usa — é o
        // idioma genérico que a VM real usa pra decodificar `new Vector4(...)`/etc. como argumento
        // de QUALQUER native (nossa ou vanilla). Diferente de CString (SSO+ptr+allocator, precisa
        // de lógica de leitura), Vector4/Quaternion são POD puro (4 floats) — memcpy direto dos
        // 16 bytes é seguro, sem nenhum campo de posse/ownership envolvido.
        if let Some(r16) = raw16_out.as_deref_mut() {
            let is_v4q = tc == cname("Vector4") || tc == cname("Quaternion");
            let bytes = if is_v4q && crate::gum::is_readable(pinst as *const c_void, 16) {
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
            let vbytes = if tc == cname("Variant") && crate::gum::is_readable(pinst as *const c_void, 24) {
                let mut buf = [0u8; 24];
                std::ptr::copy_nonoverlapping(pinst as *const u8, buf.as_mut_ptr(), 24);
                Some(buf)
            } else {
                None
            };
            vars.push(vbytes);
        }
        // Codeware `#133` (`DynamicEntityTarget.Tags(tags: array<CName>)`, 2026-08-18): 1ª leitura
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
        // por nós) — o formato `"array:<InnerTypeName>"` já está confirmado neste projeto (item
        // Codeware `#197`, `BwmsGetInnerTypeName`, "array:Float"→"Float"), então `cname("array:
        // CName")` computa o MESMO hash FNV1a64 que `GetName()` devolve pro tipo do parâmetro —
        // comparação de hash puro, sem precisar internar a string no `CNamePool` real.
        if let Some(arrs) = cnamearray_out.as_deref_mut() {
            let is_cname_arr = tc == cname("array:CName");
            let values = if is_cname_arr && crate::gum::is_readable(pinst as *const c_void, 16) {
                let size = core::ptr::read_unaligned((pinst as *const u8).add(0x0C) as *const u32);
                if size == 0 {
                    Some(Vec::new())
                } else if size <= 4096 {
                    let entries = (pinst as *const u8 as *const u64).read_unaligned() as *const u64;
                    if !entries.is_null() && crate::gum::is_readable(entries as *const c_void, (size as usize) * 8) {
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

/// Como `read_params_consuming`, mas TAMBÉM extrai o texto de qualquer param `String` (via
/// `read_cstring`) — o vetor devolvido tem o MESMO tamanho e índices do `Vec<(u64,u64)>`
/// principal; `strings[i]` é `Some(texto)` só quando o param `i` é String. Usado pelos
/// handlers de `cw-utils` Hash.reds/Number.reds (`FNV1a64(data: script_ref<String>, ...)`).
pub unsafe fn read_params_consuming_with_strings(func: *mut c_void, frame: *mut c_void) -> (Vec<(u64, u64)>, Vec<Option<String>>) {
    let mut strings = Vec::new();
    let args = read_params_inner_ext(func, frame, true, Some(&mut strings), None);
    (args, strings)
}

/// Como `read_params_consuming`, mas TAMBÉM captura o ponteiro cru `T*` de qualquer param
/// `script_ref<T>` (via `ScriptRef<T>::ref@0x18`, RED4ext.SDK `#323` já provado) — generaliza pra
/// QUALQUER `T` a extração que antes só existia hard-coded pra `String`. `scriptrefs[i]` = ponteiro
/// não-nulo se o param `i` é `script_ref<T>` e leu OK; `0` caso contrário. Item Codeware `#94`/`#95`
/// (2026-08-12, round 5): permite compor natives que ESCREVEM dentro de um `script_ref<T>` de saída
/// (ex. `inkWidgetRef.Set`/`inkWidgetLibraryResource.SetPath`), não só ler.
pub unsafe fn read_params_consuming_with_scriptrefs(func: *mut c_void, frame: *mut c_void) -> (Vec<(u64, u64)>, Vec<u64>) {
    let mut scriptrefs = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, Some(&mut scriptrefs), None, None);
    (args, scriptrefs)
}

/// Como `read_params_consuming`, mas TAMBÉM captura os 16 bytes crus de qualquer param
/// `Vector4`/`Quaternion` (POD, 4 floats) — `raw16[i]` = `Some(bytes)` quando o param `i` é um
/// desses 2 tipos e leu OK; `None` caso contrário. Fecha Codeware `#76`/`StaticEntitySpec.
/// SetPosition`/`SetOrientation` (2026-08-15, rodada 10) — 1ª vez que struct-de-16-bytes é lido
/// como PARÂMETRO nesta codebase (antes só como RETORNO de natives vanilla, ex. `#29`). Reusa a
/// MESMA máquina genérica (`type_inst_alloc`+dispatch de opcode) já provada pra CString/Variant,
/// só lendo mais bytes de `pinst` no final — ver comentário completo em `read_params_inner_ext3`.
pub unsafe fn read_params_consuming_with_raw16(func: *mut c_void, frame: *mut c_void) -> (Vec<(u64, u64)>, Vec<Option<[u8; 16]>>) {
    let mut raw16 = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, None, Some(&mut raw16), None);
    (args, raw16)
}

/// Como `read_params_consuming`, mas TAMBÉM captura os valores de qualquer param `array<CName>`
/// (`cnamearrays[i] = Some(Vec<u64>)` com os hashes CName na ORDEM real do array, ou `Some(vec![])`
/// se o array veio vazio; `None` se o param `i` não é `array<CName>`). Item Codeware `#133`
/// (`DynamicEntityTarget.Tags(tags: array<CName>)`/`StaticEntityTarget.Tags`, 2026-08-18) — 1ª
/// leitura de `array<T>` como PARÂMETRO de entrada nesta codebase (ver comentário completo em
/// `read_params_inner_ext3`, bloco `cnamearray_out`). Generaliza pra qualquer native futura que
/// precise receber uma LISTA de CNames do redscript (não só este item).
pub unsafe fn read_params_consuming_with_cnamearray(func: *mut c_void, frame: *mut c_void) -> (Vec<(u64, u64)>, Vec<Option<Vec<u64>>>) {
    let mut arrs = Vec::new();
    let args = read_params_inner_ext3(func, frame, true, None, None, None, None, Some(&mut arrs));
    (args, arrs)
}

/// Como `read_params_consuming`, mas captura os 24 bytes brutos de qualquer param `Variant` ANTES do
/// destructor. `variants[i]` = `Some([u8;24])` se param i era Variant, `None` caso contrário.
/// Tipos inline (Int32/Float/Bool/CName/TweakDBID): bytes 8..24 contêm o valor diretamente.
pub unsafe fn read_params_consuming_with_variants(
    func: *mut c_void,
    frame: *mut c_void,
) -> (Vec<(u64, u64)>, Vec<Option<[u8; 24]>>) {
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

/// RED4ext.SDK item #316 (`PENDENCIAS-UNIFICADAS.md`, `cw-variant-marshalling`): CONSTRÓI um
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
/// objeto (`pinst` — 0x20 bytes, layout confirmado pelo RED4ext.SDK vendorizado neste projeto,
/// `RED4ext.SDK/include/RED4ext/CString.hpp`+`CString-inl.hpp`, ambos C++ open-source da mesma
/// engine, layout de dados idêntico entre Windows/macOS — só vtable/dtor-count difere, já
/// documentado em [[cp77-macos-rtti-vtable-offsets]]): união SSO de 20 bytes em `+0x00`
/// (`inline_str[0x14]` OU `{ptr:8, unk:8, capacity:4}`), `length` (u32, com a flag "não-inline"
/// no bit 30 = `0x40000000`) em `+0x14`, `allocator` em `+0x18`. `IsInline()` = `length <
/// 0x40000000`; `Length()` = `length & 0x3FFFFFFF`; `c_str()` = inline_str OU o ponteiro heap.
/// Fecha a represa de leitura de String nativa (bloqueava `cw-utils` Hash.reds/Number.reds) —
/// achado 2026-07-13, ainda SEM confirmação in-game (layout vem do SDK, não de dump ao vivo).
pub unsafe fn read_cstring(pinst: *const u8) -> Option<String> {
    const CSTRING_SIZE: usize = 0x20;
    const NOT_INLINE_FLAG: u32 = 0x4000_0000;
    const LENGTH_MASK: u32 = 0x3FFF_FFFF;
    if pinst.is_null() || !crate::gum::is_readable(pinst as *const c_void, CSTRING_SIZE) {
        return None;
    }
    let length_raw = (pinst.add(0x14) as *const u32).read_unaligned();
    let length = (length_raw & LENGTH_MASK) as usize;
    if length == 0 {
        return Some(String::new());
    }
    let is_inline = length_raw < NOT_INLINE_FLAG;
    let text_ptr = if is_inline {
        pinst // union começa em +0x00; inline_str É o próprio início do objeto
    } else {
        rd_ptr(pinst) as *const u8 // union.str.ptr, também em +0x00
    };
    if text_ptr.is_null() || !crate::gum::is_readable(text_ptr as *const c_void, length) {
        return None;
    }
    let bytes = std::slice::from_raw_parts(text_ptr, length);
    Some(String::from_utf8_lossy(bytes).into_owned())
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

    pub unsafe fn enum_by_name(&self, name: &str) -> *mut c_void {
        (self.get_enum)(self.reg, cname(name))
    }

    /// `IRTTISystem::GetClassByScriptName(CName) -> CClass*` (vtbl+0x108, RED4ext.SDK item
    /// #82) — mapeamento NOME-SCRIPT→classe, distinto de `class_by_name` (que resolve
    /// `GetClass`@0x10, NOME-NATIVO). Em conteúdo vanilla os 2 nomes quase sempre coincidem
    /// (nenhum `RegisterScriptName` remapeando é usado pelo BWMS hoje), então `class_by_name`
    /// já cobre o caso comum — este método existe pro caso raro de um mod usar
    /// `RegisterScriptName` pra apelidar uma classe nativa com nome-script diferente.
    pub unsafe fn class_by_script_name(&self, name: &str) -> *mut c_void {
        let f = self.vtbl_slot(0x108);
        if f.is_null() {
            return std::ptr::null_mut();
        }
        let call: extern "C" fn(*mut c_void, u64) -> *mut c_void = std::mem::transmute(f);
        call(self.reg, cname(name))
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
        if self.vtbl.is_null() || !crate::gum::is_readable(self.vtbl as *const c_void, off + 8) {
            return std::ptr::null_mut();
        }
        rd_ptr(self.vtbl.add(off))
    }

    // ===== RED4ext.SDK — cluster de introspecção RTTI em massa (`IRTTISystem`, offsets 0x40-0x78,
    // `PENDENCIAS-UNIFICADAS.md` itens #57/#58/#60/#61/#62/#63/#64). Todo método aqui devolve
    // `void Foo(DynArray<T*>& out, ...)` — o payload de saída é o MESMO DynArray de 16 bytes
    // (entries:u64@0, size:u32@8, cap:u32@12) já lido por `dispatch_scriptable_tweaks` (vtbl+0x40,
    // TweakXL #42, "27718 tipos" confirmado ao vivo) — mecanismo genérico reaproveitado, mesma
    // confiança. Offsets vêm direto do header vendorizado (`RTTISystem.hpp`).

    /// Núcleo comum: chama `void Foo(DynArray<T*>&)` (sem args extras) e devolve (entries_ptr, count).
    /// RED4ext.SDK #63 (2026-08-11, disassembly ARM64 real): layout do `DynArray` de saída é
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
    /// `AddPostRegisterCallback` (vtbl+0xC8, item #74) — `Callback<void(*)(), 32>` é um
    /// type-erased "std::function" de 40 bytes (`RED4EXT_ASSERT_SIZE(Callback<void(*)()>, 0x28)`,
    /// ver `Callback.hpp`): `{buffer:[u8;32], handler:*CallbackHandler}`, NÃO-trivial (tem
    /// copy/move/destruct) → passado POR REFERÊNCIA INVISÍVEL no Itanium ABI (ponteiro pro objeto
    /// materializado pelo chamador, não os 40 bytes inline). `cb_ptr` = ponteiro pro struct já
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

// ===== Callback<void(*)()> — construção manual do type-erased "std::function" que
// AddRegisterCallback/AddPostRegisterCallback exigem por valor (ver `Detail/Callback.hpp`
// vendorizado: `UnboundFunctionTarget<void>{func}` + `CallbackHandlerImpl` gera
// invoke/copy/move/destruct estáticos). Replicamos os 4 fins à mão em Rust — não precisa
// compilar C++, só bater o LAYOUT (ABI Itanium, mesma convenção usada em todo o resto do
// projeto pra objetos C++ do motor).

#[repr(C)]
struct CallbackHandlerRaw {
    invoke: extern "C" fn(*const u8),
    copy: extern "C" fn(*mut u8, *mut u8),
    mv: extern "C" fn(*mut u8, *mut u8),
    destruct: extern "C" fn(*mut u8),
}

/// `UnboundFunctionTarget<void>::Invoke` — `std::invoke(aTarget->func)`, sem args, sem retorno.
extern "C" fn cb_target_invoke(target: *const u8) {
    if target.is_null() {
        return;
    }
    let raw = unsafe { (target as *const u64).read_unaligned() };
    if raw == 0 {
        return;
    }
    let f: extern "C" fn() = unsafe { std::mem::transmute(raw) };
    f();
}
extern "C" fn cb_target_copy(dst: *mut u8, src: *mut u8) {
    unsafe {
        let v = (src as *const u64).read_unaligned();
        (dst as *mut u64).write_unaligned(v);
    }
}
extern "C" fn cb_target_move(dst: *mut u8, src: *mut u8) {
    cb_target_copy(dst, src);
    unsafe {
        (src as *mut u64).write_unaligned(0);
    }
}
extern "C" fn cb_target_destruct(target: *mut u8) {
    unsafe {
        (target as *mut u64).write_unaligned(0);
    }
}

static CB_HANDLER: CallbackHandlerRaw = CallbackHandlerRaw {
    invoke: cb_target_invoke,
    copy: cb_target_copy,
    mv: cb_target_move,
    destruct: cb_target_destruct,
};

#[repr(C)]
pub struct CallbackRaw {
    buffer: [u8; 32],
    handler: *const CallbackHandlerRaw,
}

/// Monta um `Callback<void(*)()>` (40 bytes: buffer[32]+handler@0x20) envolvendo `target` —
/// nossa própria `extern "C" fn()`, guardada nos 8 primeiros bytes do buffer (mesmo layout de
/// `UnboundFunctionTarget<void>{func}`, o resto do buffer fica zerado/não-usado).
pub fn build_void_callback(target: extern "C" fn()) -> CallbackRaw {
    let mut buffer = [0u8; 32];
    let ptr = target as usize as u64;
    buffer[0..8].copy_from_slice(&ptr.to_le_bytes());
    CallbackRaw { buffer, handler: &CB_HANDLER }
}

/// Prova ao vivo do cluster de introspecção em massa (#57/#58/#60/#61/#62/#63/#64) — só CONTA (não
/// dereferencia elementos individuais, suficiente pra provar que o mecanismo funciona e os counts
/// são plausíveis). `GetNativeTypes` é o único com um baseline JÁ conhecido (~27718, de
/// `dispatch_scriptable_tweaks`) — usado como âncora de sanidade pros outros.
pub unsafe fn probe_mass_enumeration(reg: &Registry) {
    let (_, n_types) = reg.get_native_types();
    crate::log(&format!(
        "[rttimass] GetNativeTypes: {n_types} tipos (baseline conhecido ~27718-27719, de dispatch_scriptable_tweaks)"
    ));
    let (_, n_gfuncs) = reg.get_global_functions();
    crate::log(&format!("[rttimass] GetGlobalFunctions: {n_gfuncs} funções globais"));
    let (_, n_cfuncs) = reg.get_class_functions();
    crate::log(&format!("[rttimass] GetClassFunctions: {n_cfuncs} funções de classe"));
    let (_, n_enums) = reg.get_enums(false);
    crate::log(&format!("[rttimass] GetEnums(scriptedOnly=false): {n_enums} enums"));
    let (_, n_enums_scripted) = reg.get_enums(true);
    crate::log(&format!("[rttimass] GetEnums(scriptedOnly=true): {n_enums_scripted} enums (deve ser <= {n_enums})"));
    let (_, n_bitfields) = reg.get_bitfields(false);
    crate::log(&format!("[rttimass] GetBitfields(scriptedOnly=false): {n_bitfields} bitfields"));

    let iscriptable = reg.class_by_name("IScriptable");
    if !iscriptable.is_null() {
        let (_, n_derived) = reg.get_derived_classes(iscriptable);
        crate::log(&format!("[rttimass] GetDerivedClasses(IScriptable): {n_derived} classes (herança DIRETA só)"));
        let (_, n_classes_concrete) = reg.get_classes(iscriptable, false);
        let (_, n_classes_all) = reg.get_classes(iscriptable, true);
        crate::log(&format!(
            "[rttimass] GetClasses(IScriptable, includeAbstract=false): {n_classes_concrete} | includeAbstract=true: {n_classes_all} (transitivo, deve ser >= GetDerivedClasses e all>=concrete)"
        ));
    } else {
        crate::log("[rttimass] classe 'IScriptable' não achada — GetClasses/GetDerivedClasses pulados");
    }
}

/// RED4ext.SDK item #68 (`UnregisterType`, vtbl+0x98, `PENDENCIAS-UNIFICADAS.md`) — prova ao vivo
/// REVERSÍVEL e AUTOCONTIDA: cria um enum de TESTE descartável via `create_scripted_enum` (já
/// provado seguro, 2026-08-05, "zero crash até aqui"), confirma que existe, desregistra, confirma
/// que sumiu. Não toca em NADA vanilla nem em classes forjadas usadas por outro código — o enum de
/// teste nasce e morre inteiramente dentro desta função.
pub unsafe fn probe_unregister_type(reg: &Registry) {
    let name = "BwmsUnregisterTestEnum";
    let created = crate::register::create_scripted_enum(reg, name, 1, &[("A", 0), ("B", 1)]);
    let before = reg.enum_by_name(name);
    crate::log(&format!(
        "[rttiunreg] CreateScriptedEnum('{name}') call_ok={created} enum_by_name antes={before:p} (esperado: não-null)"
    ));
    if before.is_null() {
        crate::log("[rttiunreg] enum de teste não resolveu — UnregisterType pulado (sem alvo seguro)");
        return;
    }
    let unreg_ok = reg.unregister_type(before);
    let after = reg.enum_by_name(name);
    crate::log(&format!(
        "[rttiunreg] UnregisterType(enum de teste) call_ok={unreg_ok} enum_by_name depois={after:p} desregistrado={}",
        after.is_null()
    ));
}

/// RED4ext.SDK item #68 — CAUSA RAIZ achada por disassembly ARM64 real (2026-08-11, sessão de
/// comparação `UnregisterType` vs o irmão CONFIRMADO `UnregisterFunction`/#70): disassemblei o
/// corpo inteiro de `UnregisterType`@vtbl+0x98, da rotina COMPARTILHADA de lookup por nome que
/// `GetClass`@vtbl+0x10/`GetEnum`@vtbl+0x18 chamam (`0x102192d08` nesta build), e — pra
/// comparação — `UnregisterFunction`@vtbl+0xA8/`GetFunction`@vtbl+0x30. Cruzado contra o layout
/// OFICIAL de `CRTTISystem` do `RED4ext.SDK/include/RED4ext/RTTISystem.hpp` (vendorizado neste
/// projeto): `types:HashMap<CName,IType*>@+0x10` (a PRIMÁRIA, checada PRIMEIRO por
/// GetClass/GetEnum — se bate, retorna NA HORA sem tocar mais nada), `typesByAsyncId:
/// HashMap<uint64_t,IType*>@+0x40`, `typeAsyncIds:HashMap<CName,uint32_t>@+0x70` (índice
/// CName→asyncId, só usado como FALLBACK de 2 saltos quando `types`@+0x10 não tem entrada:
/// CName→asyncId via +0x70, depois asyncId→IType* via +0x40).
///
/// **Achado**: o corpo nativo de `UnregisterType` só remove a entrada de `typesByAsyncId`@+0x40
/// (lê `typeAsyncIds`@+0x70 pra achar o asyncId, mas NUNCA desengancha nada de lá nem de
/// `types`@+0x10 — só LÊ +0x70, nunca ESCREVE). Já `UnregisterFunction` faz a dupla completa:
/// remove de `funcs`@+0xA0 (via `bl 0x100d20210` inline) E de `funcsByHash`@+0xD0 — e é
/// EXATAMENTE `funcsByHash`@+0xD0 que `GetFunction`@vtbl+0x30 consulta (confirmado por disasm
/// separado), por isso `UnregisterFunction` funciona de ponta a ponta. `UnregisterType` faz só
/// METADE do trabalho equivalente: nunca toca a estrutura (`types`@+0x10) que `GetClass`/
/// `GetEnum` checam PRIMEIRO — por isso o tipo "desregistrado" continua resolvendo pra sempre
/// (a entrada em `types`@+0x10, escrita no registro/primeira-resolução, nunca é invalidada).
///
/// **Fix** (não é bug nosso de leitura de retorno como o #63/GetClasses — é uma lacuna genuína
/// do próprio `UnregisterType` nativo): chama o `UnregisterType` real primeiro (preserva os
/// efeitos legítimos dele: remoção de `typesByAsyncId`+callback de notificação via
/// `vtbl+0x88` de um singleton à parte) e DEPOIS completa manualmente a remoção que falta em
/// `types`@+0x10 — mesmo idioma de `HashMap<CName,X>::unlink` já usado e testado em
/// `selftest::wardrobe_forget_item` (separate-chaining, node=`{next:u32@0,hashedKey:u32@4,
/// key:CName@8,value:T@0x10}`), sob o MESMO lock CAS que o `UnregisterType` nativo usa
/// (`this+0x220`, byte 0=livre/0x80=exclusivo — confirmado disassemblando o helper de lock que a
/// própria função nativa chama, `CASALB` 0↔0x80, diferente do protocolo 0↔0xFF do `mutex00` do
/// TweakDB) — nunca corre risco de colidir com uma mutação concorrente da RTTI. Deixar
/// `typeAsyncIds`@+0x70 com uma entrada "órfã" (apontando pro asyncId já removido de
/// `typesByAsyncId`) é INÓCUO: o fallback de 2 saltos que ela alimenta já falha corretamente no
/// 2º salto (native já limpou +0x40), então não precisa de mais nenhuma escrita.
pub unsafe fn unregister_type_complete(reg: &Registry, ty: *mut c_void) -> bool {
    let native_ok = reg.unregister_type(ty);
    if !native_ok || ty.is_null() {
        return native_ok;
    }
    // Precisa do CName do tipo pra achar o node em `types`@+0x10 — mesma via de `IType::GetName()`
    // (vtbl+0x10 do PRÓPRIO tipo, shift +0x08 macOS já documentado noutros pontos deste arquivo)
    // que o corpo nativo de UnregisterType também usa como 1º passo.
    if !crate::gum::is_readable(ty as *const c_void, 8) {
        return native_ok;
    }
    let type_vtbl = rd_ptr(ty as *const u8);
    if type_vtbl.is_null() || !crate::gum::is_readable(type_vtbl as *const c_void, 0x18) {
        return native_ok;
    }
    let get_name_fn = rd_ptr((type_vtbl as *const u8).add(0x10));
    if get_name_fn.is_null() {
        return native_ok;
    }
    let get_name: extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(get_name_fn);
    let name_hash = get_name(ty);
    let removed_from_fastpath = unregister_from_types_fastpath(reg.raw(), name_hash);
    crate::log(&format!(
        "[unregtype-fix] types@+0x10: removido_agora={removed_from_fastpath} (name={name_hash:#018x})"
    ));
    native_ok
}

/// Trava `this+0x220` (mesmo CAS 0↔0x80 do lock nativo de UnregisterType/GetClass/GetEnum,
/// confirmado por disasm de `0x1000020c0`) e desengancha `target_cname` de
/// `types:HashMap<CName,IType*>@this+0x10`. Mesmo idioma read-verify-write de
/// `selftest::wardrobe_forget_item_locked`: aborta sem mudar nada se qualquer leitura cair fora
/// de faixa plausível ou apontar pra memória ilegível.
unsafe fn unregister_from_types_fastpath(this_ptr: *mut c_void, target_cname: u64) -> bool {
    use std::sync::atomic::{AtomicU8, Ordering};
    if !crate::gum::is_readable(this_ptr as *const c_void, 0x40) {
        return false;
    }
    let this = this_ptr as *mut u8;
    let lock = &*(this.add(0x220) as *const AtomicU8);
    let mut locked = false;
    for i in 0..4_000_000u32 {
        if lock.compare_exchange(0, 0x80, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            locked = true;
            break;
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    if !locked {
        crate::log("[unregtype-fix] lock @+0x220 ocupado (spin esgotado) — types@+0x10 NÃO tocado");
        return false;
    }
    let result = types_fastpath_unlink_locked(this.add(0x10), target_cname);
    lock.fetch_and(!0x80u8, Ordering::Release);
    result
}

unsafe fn types_fastpath_unlink_locked(hm: *mut u8, target_cname: u64) -> bool {
    const INVALID: u32 = 0xFFFF_FFFF;
    if !crate::gum::is_readable(hm as *const c_void, 0x20) {
        return false;
    }
    let index_table = (hm as *const u64).read_unaligned() as *mut u32;
    let size_cap = (hm.add(0x08) as *const u64).read_unaligned();
    let capacity = (size_cap >> 32) as u32;
    let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *mut u8;
    let stride = (hm.add(0x1c) as *const u32).read_unaligned();
    if capacity == 0
        || capacity > 1_000_000
        || index_table.is_null()
        || nodes.is_null()
        || stride == 0
        || stride > 256
    {
        crate::log("[unregtype-fix] types@+0x10 fora de faixa plausível, abortado sem mudar nada");
        return false;
    }
    let hashed_key = ((target_cname & 0xFFFF_FFFF) as u32) ^ ((target_cname >> 32) as u32);
    let bucket = (hashed_key % capacity) as usize;
    let mut prev_next_field: *mut u32 = index_table.add(bucket);
    let mut idx = prev_next_field.read_unaligned();
    let mut guard = 0u32;
    while idx != INVALID && guard < capacity + 1 {
        guard += 1;
        let node = nodes.add(idx as usize * stride as usize);
        if !crate::gum::is_readable(node as *const c_void, stride as usize) {
            crate::log("[unregtype-fix] node ilegível no meio da cadeia, abortado sem mudar nada");
            return false;
        }
        let next = (node as *const u32).read_unaligned();
        let node_hashed = (node.add(4) as *const u32).read_unaligned();
        let node_key = (node.add(8) as *const u64).read_unaligned();
        if node_hashed == hashed_key && node_key == target_cname {
            prev_next_field.write_unaligned(next);
            // freelist push (nodeList.nextIdx @ hm+0x20, mesmo layout do HashMap.hpp vendorizado
            // já usado em wardrobe_forget_item_locked).
            let nextidx_ptr = hm.add(0x20) as *mut u32;
            let old_head = nextidx_ptr.read_unaligned();
            (node as *mut u32).write_unaligned(old_head);
            nextidx_ptr.write_unaligned(idx);
            let size_ptr = hm.add(0x08) as *mut u32;
            let cur = size_ptr.read_unaligned();
            size_ptr.write_unaligned(cur.saturating_sub(1));
            crate::log(&format!(
                "[unregtype-fix] types@+0x10: bucket={bucket} idx={idx} size {cur}->{}",
                cur.saturating_sub(1)
            ));
            return true;
        }
        prev_next_field = node as *mut u32;
        idx = next;
    }
    crate::log("[unregtype-fix] CName não achado em types@+0x10 (talvez já não estava lá)");
    false
}

/// Prova ao vivo do fix de `unregister_type_complete` — MESMO padrão autocontido/reversível de
/// `probe_unregister_type` (enum de teste descartável, nasce/morre só nesta função), mas chama a
/// via COMPLETA (nativa + fastpath manual) em vez da nativa crua, pra confirmar que o efeito
/// esperado (some do lookup por nome) agora se confirma de verdade.
pub unsafe fn probe_unregister_type_complete(reg: &Registry) {
    let name = "BwmsUnregisterTestEnum2";
    let created = crate::register::create_scripted_enum(reg, name, 1, &[("A", 0), ("B", 1)]);
    let before = reg.enum_by_name(name);
    crate::log(&format!(
        "[rttiunregfix] CreateScriptedEnum('{name}') call_ok={created} enum_by_name antes={before:p} (esperado: não-null)"
    ));
    if before.is_null() {
        crate::log("[rttiunregfix] enum de teste não resolveu — probe pulado (sem alvo seguro)");
        return;
    }
    let unreg_ok = unregister_type_complete(reg, before);
    let after = reg.enum_by_name(name);
    crate::log(&format!(
        "[rttiunregfix] unregister_type_complete(enum de teste) call_ok={unreg_ok} enum_by_name depois={after:p} desregistrado={}",
        after.is_null()
    ));
}

/// RED4ext.SDK item #70 (`UnregisterFunction`, vtbl+0xA8) — mesmo padrão de `probe_unregister_type`,
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
        crate::log("[rttiunregfn] nenhum global de protótipo resolveu — probe pulado");
        return;
    }
    unsafe extern "C" fn tramp_unregfn_test(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {}
    let created = crate::register::register_global(reg, proto, name, name, tramp_unregfn_test);
    let before = crate::register::get_function(reg, name);
    crate::log(&format!(
        "[rttiunregfn] register_global('{name}') call_ok={created} get_function antes={before:p} (esperado: não-null)"
    ));
    if before.is_null() {
        crate::log("[rttiunregfn] global de teste não resolveu — UnregisterFunction pulado (sem alvo seguro)");
        return;
    }
    let unreg_ok = reg.unregister_function(before);
    let after = crate::register::get_function(reg, name);
    crate::log(&format!(
        "[rttiunregfn] UnregisterFunction(global de teste) call_ok={unreg_ok} get_function depois={after:p} desregistrada={}",
        after.is_null()
    ));
}

// ===== `CBitfield` (12 métodos, `PENDENCIAS-UNIFICADAS.md`: "nunca lido nem escrito — diferente
// de CEnum, que já tem equivalente") — campos diretos, mesmo padrão já usado pra `CEnum`
// (`hashList`/`valueList`): `validBits:u64@+0x28` (máscara de quais dos 64 bits possíveis estão em
// uso) + `bitNames:CName[64]@+0x30` (header vendorizado `RTTITypes.hpp`).

/// Enumera os bits NOMEADOS de um `CBitfield*` (índice, nome). Varre `validBits` — só lê
/// `bitNames[i]` pros bits realmente marcados (evita resolver 64 CNames à toa).
pub unsafe fn dump_bitfield_names(bf: *mut c_void) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let base = bf as *const u8;
    if !crate::gum::is_readable(base.add(0x28) as *const c_void, 8 + 64 * 8) {
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
pub unsafe fn probe_bitfield_dump(reg: &Registry, name: &str) {
    let bf = reg.bitfield_by_name(name);
    if bf.is_null() {
        crate::log(&format!("[bitfielddump] bitfield '{name}' não resolveu"));
        return;
    }
    let bits = dump_bitfield_names(bf);
    crate::log(&format!("[bitfielddump] {name} ({bf:p}): {} bit(s) nomeado(s)", bits.len()));
    for (i, n) in &bits {
        crate::log(&format!("[bitfielddump]   [{i}] = '{n}'"));
    }
}

/// Codeware `Reflection.ReflectionEnum.IsNative()`/`ReflectionBitfield.IsNative()` — `!flags.
/// isScripted` (fonte C++ real, `ReflectionEnum.hpp`). `CEnum.flags@+0x21`/`CBitfield.flags@+0x21`
/// (ambos 1 byte, bit0=isScripted — header vendorizado `RTTITypes.hpp`), leitura trivial, mesma
/// categoria de `class_flags`/`property_flags` já fechados nesta sessão.
pub unsafe fn type_is_native_scripted(ty: *mut c_void) -> Option<bool> {
    if !crate::gum::is_readable((ty as *const u8).add(0x21) as *const c_void, 1) {
        return None;
    }
    let b = *(ty as *const u8).add(0x21);
    Some(b & 1 == 0) // isScripted=bit0; IsNative = !isScripted
}

/// Comando `enumisnative <nome>` — `CEnum`/`CBitfield` compartilham offset (ambos herdam
/// `rtti::IType`, layout `name/computedName/actualSize/flags` idêntico até `+0x22`).
pub unsafe fn probe_type_is_native(reg: &Registry, name: &str) {
    let en = reg.enum_by_name(name);
    let ty = if !en.is_null() { en } else { reg.bitfield_by_name(name) };
    if ty.is_null() {
        crate::log(&format!("[isnative] '{name}' não resolveu (nem enum nem bitfield)"));
        return;
    }
    match type_is_native_scripted(ty) {
        Some(native) => crate::log(&format!("[isnative] {name}: is_native={native}")),
        None => crate::log(&format!("[isnative] {name}: flags@+0x21 ilegível")),
    }
}

/// RED4ext.SDK item #79 (`CreateScriptedBitfield(CName, DynArray<uint64_t>&)`, vtbl+0xF0) — via
/// OFICIAL de criar um bitfield, irmã de `CreateScriptedEnum` (item #78, já usada por
/// `create_scripted_enum`/`probe_unregister_type`). Header vendorizado: "the bits array is not of
/// type uint64_t, it is a struct containing the name and the bit" — mesma forma de
/// `EnumMemberEntry{name:CName, value:i64}` já provada, aqui com o campo renomeado pra `bit`
/// (índice do bit, não um valor arbitrário). Prova: cria bitfield de teste, confirma via
/// `bitfield_by_name` (novo, vtbl+0x20) + delta de contagem via `get_bitfields` + `dump_bitfield_names`
/// (confirma os NOMES reais dos bits, não só que o ponteiro resolve).
pub unsafe fn probe_create_scripted_bitfield(reg: &Registry) {
    let name = "BwmsTestBitfield2";
    let (_, before_count) = reg.get_bitfields(false);
    // Achado 2026-08-09 (1ª rodada desta prova, `name = "BwmsTestBitfield"`): o campo "bit" NÃO é
    // um índice zero-based — é o VALOR da bitmask (potência de 2). Passar índices (0,1,2) fez
    // `FlagA` (bit=0) sumir (tratado como "sem bit"/inválido) e `FlagB`/`FlagC` caírem 1 posição
    // pra trás (bit=1 -> validBits index 0, bit=2 -> index 1) — 2 de 3 bits, nomes certos mas
    // deslocados. Corrigido: valores de bitmask reais (1,2,4).
    let created = crate::register::create_scripted_bitfield(reg, name, &[("FlagA", 1), ("FlagB", 2), ("FlagC", 4)]);
    let resolved = reg.bitfield_by_name(name);
    let (_, after_count) = reg.get_bitfields(false);
    crate::log(&format!(
        "[rttibitfield] CreateScriptedBitfield('{name}', 3 bits) call_ok={created} bitfield_by_name={resolved:p} \
         GetBitfields antes={before_count} depois={after_count} (esperado: depois==antes+1 se resolveu)"
    ));
    if !resolved.is_null() {
        let bits = dump_bitfield_names(resolved);
        crate::log(&format!(
            "[rttibitfield] dump_bitfield_names: {} bit(s) — esperado 3 (FlagA/FlagB/FlagC)",
            bits.len()
        ));
        for (i, n) in &bits {
            crate::log(&format!("[rttibitfield]   [{i}] = '{n}'"));
        }
    }
}

/// RED4ext.SDK itens #73/#74 (`AddRegisterCallback`/`AddPostRegisterCallback`, vtbl+0xC0/0xC8) —
/// registra um callback `void()` nosso via o `Callback<void(*)()>` construído à mão
/// (`build_void_callback`). A chamada em si (materializar+passar o struct) é o que estamos
/// provando primeiro — SE o callback dispara é uma 2ª pergunta em aberto (a nota original do
/// catálogo suspeita que não, porque a fase de registro do RTTI já passou faz tempo quando um
/// plugin carrega tão tarde quanto o BWMS) — por isso os 2 flags ficam em estáticos, consultáveis
/// depois por `probe_register_callback_status` sem precisar reinstalar nada.
static REGISTER_CB_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static POST_REGISTER_CB_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn on_register_cb_fired() {
    REGISTER_CB_FIRED.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::log("[rttiregcb] >>> AddRegisterCallback DISPAROU <<<");
}
extern "C" fn on_post_register_cb_fired() {
    POST_REGISTER_CB_FIRED.store(true, std::sync::atomic::Ordering::SeqCst);
    crate::log("[rttiregcb] >>> AddPostRegisterCallback DISPAROU <<<");
}

pub unsafe fn probe_register_callbacks(reg: &Registry) {
    let mut cb1 = build_void_callback(on_register_cb_fired);
    let ok1 = reg.add_register_callback(&mut cb1 as *mut CallbackRaw as *mut c_void);
    let mut cb2 = build_void_callback(on_post_register_cb_fired);
    let ok2 = reg.add_post_register_callback(&mut cb2 as *mut CallbackRaw as *mut c_void);
    crate::log(&format!(
        "[rttiregcb] AddRegisterCallback call_ok={ok1} AddPostRegisterCallback call_ok={ok2} — zero crash até aqui \
         (fires imediatos={}/{}; rode 'regcallbackstatus' depois pra checar se disparou mais tarde, ex. em hot-reload)",
        REGISTER_CB_FIRED.load(std::sync::atomic::Ordering::SeqCst),
        POST_REGISTER_CB_FIRED.load(std::sync::atomic::Ordering::SeqCst)
    ));
}

pub fn probe_register_callback_status() {
    crate::log(&format!(
        "[rttiregcb] status atual: register_fired={} post_register_fired={}",
        REGISTER_CB_FIRED.load(std::sync::atomic::Ordering::SeqCst),
        POST_REGISTER_CB_FIRED.load(std::sync::atomic::Ordering::SeqCst)
    ));
}

/// RED4ext.SDK item #63 (`GetClasses`, vtbl+0x70) — investigação da anomalia achada em
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
            crate::log(&format!("[rttiganom] âncora '{name}' não resolveu — pulado"));
            continue;
        }
        let (_, n_derived) = reg.get_derived_classes(anchor);
        let (_, n_classes) = reg.get_classes(anchor, false);
        let (_, n_types) = reg.get_native_types();
        crate::log(&format!(
            "[rttiganom] âncora='{name}' GetDerivedClasses={n_derived} GetClasses(includeAbstract=false)={n_classes} \
             (universo GetNativeTypes={n_types}) — anomalia presente se GetClasses==universo apesar da âncora restrita"
        ));
    }
}

/// RED4ext.SDK item #63, continuação (2026-08-11): a anomalia de `GetClasses`(vtbl+0x70) foi
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
        crate::log("[rttiganom2] vtbl da Registry é null — abortado");
        return;
    }
    let p40 = reg.vtbl_slot(0x40);
    let p70 = reg.vtbl_slot(0x70);
    let p78 = reg.vtbl_slot(0x78); // GetDerivedClasses, já confirmado correto — controle de sanidade
    let same_40_70 = !p40.is_null() && p40 == p70;
    let same_70_78 = !p70.is_null() && p70 == p78;
    let bytes_at = |p: *mut c_void| -> [u8; 8] {
        if p.is_null() || !crate::gum::is_readable(p as *const c_void, 8) {
            return [0; 8];
        }
        let mut b = [0u8; 8];
        std::ptr::copy_nonoverlapping(p as *const u8, b.as_mut_ptr(), 8);
        b
    };
    let b40 = bytes_at(p40);
    let b70 = bytes_at(p70);
    let b78 = bytes_at(p78);
    crate::log(&format!(
        "[rttiganom2] slot0x40(GetNativeTypes)={p40:p} bytes={b40:02x?} | slot0x70(GetClasses)={p70:p} bytes={b70:02x?} | \
         slot0x78(GetDerivedClasses,controle)={p78:p} bytes={b78:02x?} | 0x40==0x70={same_40_70} (ICF hipótese) | 0x70==0x78={same_70_78} (sanidade, deveria ser false)"
    ));
}

/// RED4ext.SDK item #63, continuação (2026-08-11, sessão de disassembly ARM64 real): a hipótese
/// ICF já foi refutada (`probe_getclasses_vtable_identity`) — endereços de `GetClasses`(0x70) e
/// `GetNativeTypes`(0x40) são genuinamente diferentes. Disassembly manual (Capstone, via
/// `rttislot`+`disasm_region.py`) do CORPO INTEIRO dos 3 métodos (`GetNativeTypes`/`GetClasses`/
/// `GetDerivedClasses`) revelou o mecanismo exato:
///
/// - As 3 funções usam a MESMA rotina de push/growth pro `DynArray` de saída (`bl 0x1000286e8`),
///   com layout `{entries:u64@0x00, capacity:u32@0x08, size:u32@0x0C}` (confirmado
///   inequivocamente: o campo comparado contra "needed" pra decidir growth é @0x08, e o campo que
///   recebe `size+1` após cada push bem-sucedido é @0x0C).
/// - `GetNativeTypes` e `GetClasses` (mas NÃO `GetDerivedClasses`) fazem um RESERVE ANTECIPADO da
///   capacidade pro TAMANHO TOTAL da hashtable `types` (`this+0x48`) **ANTES de qualquer
///   filtro rodar** — over-allocam pro universo inteiro de cara, independente de quantos itens vão
///   de fato passar no filtro.
/// - `GetClasses`, quando `aIsAClass != null` (nosso uso sempre), CHAMA um helper de ancestralidade
///   real (`0x10219ac60`, disassemblado e confirmado: `IsKindOf(candidate, target)`, walk do campo
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
        crate::log(&format!("[rttisize] âncora '{anchor_name}' não resolveu — abortado"));
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

    crate::log(&format!(
        "[rttisize] âncora='{anchor_name}' | GetNativeTypes cap={cap_nt} size={size_nt} (esperado cap==size) | \
         GetClasses cap={cap_gc} size={size_gc} (BUG: BWMS lê cap, deveria ler size) | \
         GetDerivedClasses cap={cap_gd} size={size_gd} (controle, sem reserve antecipado)"
    ));
}

/// Codeware item #50 (`Reflection.GetClasses`/`GetEnums`/`GetBitfields`/`GetDerivedClasses`/
/// `GetGlobalFunctions`, "enumeração em massa nunca confirmada") — a fonte C++ real
/// (`Reflection.hpp`) mostra que `GetClasses`/`GetEnums`/`GetBitfields` são só `rtti->types.ForEach`
/// FILTRADO por `aType->GetType()==ERTTIType::X` — ou seja, `get_native_types()` (universo
/// completo, já provado) + `type_kind()` (decoder de categoria, já provado) compõem a MESMA
/// capacidade sem RE nova. `GetDerivedClasses` na fonte real usa `rtti->GetClasses(base,...,true)`
/// — o MESMO método com a anomalia documentada nesta sessão (item #63 RED4ext.SDK, devolve o
/// universo inteiro) — em vez de replicar o bug, uso `reg.get_derived_classes` (vtbl+0x78, método
/// DIFERENTE e JÁ PROVADO correto) pro mesmo efeito funcional. `GetGlobalFunctions` já é
/// `reg.get_global_functions()` (vtbl+0x48, já provado, "4435 funções globais").
///
/// Conta quantos dos tipos do universo (`get_native_types`) batem uma categoria `ERTTIType`, e
/// lista os primeiros `n` nomes (via `type_name_getname`, já usado em todo o projeto).
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
            crate::log(&format!(
                "[reflenum] categoria '{kind_name}' desconhecida (use classes/enums/bitfields/fundamental/array/handle/weakhandle)"
            ));
            return;
        }
    };
    let (total, names) = count_and_list_by_kind(reg, target, n);
    crate::log(&format!("[reflenum] categoria='{kind_name}' total={total}, primeiros {}: {:?}", names.len(), names));
}

/// Comando `reflderived <classe base>` — `Reflection.GetDerivedClasses` real, via
/// `get_derived_classes` (JÁ PROVADO, evita a anomalia de `GetClasses`).
pub unsafe fn probe_reflect_derived(reg: &Registry, base: &str) {
    let cls = reg.class_by_name(base);
    if cls.is_null() {
        crate::log(&format!("[reflderived] classe base '{base}' não resolveu"));
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
    crate::log(&format!("[reflderived] {base}: {count} classe(s) derivada(s) direta(s), primeiras {}: {:?}", names.len(), names));
}

/// RED4ext.SDK itens #81-85 (`PENDENCIAS-UNIFICADAS.md`) — cluster de mapeamento nome-nativo↔
/// nome-script do `IRTTISystem`. Todos os 5 offsets vêm DIRETO do header vendorizado
/// (`RED4ext.SDK/include/RED4ext/RTTISystem.hpp`, linhas 0x100-0x120) — sem adivinhação de
/// slot: o offset 0x40 (`GetNativeTypes`, item #57) já está INDEPENDENTEMENTE confirmado correto
/// por um boot real anterior (`dispatch_scriptable_tweaks`, "27718 tipos" — TweakXL #42), e como
/// é a MESMA vtable sequencial, isso dá alta confiança nos offsets vizinhos sem precisar re-provar
/// cada um isoladamente. Read-only, sem mutar nada (exceto `RegisterScriptName`, testado por
/// último, sobre uma classe FORJADA NOSSA — nunca uma classe vanilla).
pub unsafe fn probe_script_name_mapping(reg: &Registry) {
    // 1. class_by_script_name vs class_by_name pra uma classe vanilla conhecida — devem bater
    // (nenhum RegisterScriptName de remapeamento está em jogo pra conteúdo vanilla).
    let native_cls = reg.class_by_name("gamedataWeaponItem_Record");
    let script_cls = reg.class_by_script_name("gamedataWeaponItem_Record");
    crate::log(&format!(
        "[rttiname] class_by_name={native_cls:p} class_by_script_name={script_cls:p} match={}",
        native_cls == script_cls && !native_cls.is_null()
    ));

    // 2. round-trip native->script->native de um CName conhecido.
    let native_hash = cname("gamedataWeaponItem_Record");
    let script_hash = reg.convert_native_to_script_name(native_hash);
    let back_hash = reg.convert_script_to_native_name(script_hash);
    crate::log(&format!(
        "[rttiname] ConvertNativeToScriptName({native_hash:#018x}) -> {script_hash:#018x} -> ConvertScriptToNativeName -> {back_hash:#018x} round_trip_ok={}",
        back_hash == native_hash
    ));

    // 3. GetEnumByScriptName pra um enum vanilla conhecido (mesmo raciocínio do passo 1) — usa
    // "gamedataDevelopmentPointType", já confirmado real neste projeto (console.rs, resolve_enum_value).
    let native_enum = reg.enum_by_name("gamedataDevelopmentPointType");
    let script_enum = reg.enum_by_script_name("gamedataDevelopmentPointType");
    crate::log(&format!(
        "[rttiname] enum_by_name={native_enum:p} enum_by_script_name={script_enum:p} match={}",
        native_enum == script_enum && !native_enum.is_null()
    ));

    // 4. RegisterScriptName — só sobre uma classe FORJADA nossa (TweakXL, se já registrada),
    // nunca vanilla. Registra "BwmsTweakXLAlias" -> confirma via class_by_script_name.
    let forged = reg.class_by_name("TweakXL");
    if !forged.is_null() {
        let ok = reg.register_script_name("TweakXL", "BwmsTweakXLAlias");
        let via_alias = reg.class_by_script_name("BwmsTweakXLAlias");
        crate::log(&format!(
            "[rttiname] RegisterScriptName(TweakXL, BwmsTweakXLAlias) call_ok={ok} class_by_script_name(alias)={via_alias:p} match_forged={}",
            via_alias == forged
        ));
    } else {
        crate::log("[rttiname] classe forjada 'TweakXL' não achada — passo 4 (RegisterScriptName) pulado");
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

/// Lista os membros de um enum com os VALORES. Par de `funcs`/`findfunc` pro lado dos enums:
/// `AddGodMode(Invulnerable=0)` no log é uma afirmação que ninguém conferiu, e um valor errado
/// aqui manda "None" pro jogo e o cheat não faz nada — sem erro nenhum.
pub unsafe fn list_enum(reg: &Registry, enum_type: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    let en = reg.enum_by_name(enum_type) as *const u8;
    if en.is_null() {
        return out;
    }
    let hp = rd_ptr(en.add(0x28)) as *const u8;
    let n = rd_u32(en.add(0x30));
    let vp = rd_ptr(en.add(0x38)) as *const u8;
    if hp.is_null() || vp.is_null() || n > 100_000 {
        return out;
    }
    for i in 0..n as usize {
        out.push((crate::cname::resolve_cname(rd_u64(hp.add(i * 8))), rd_u64(vp.add(i * 8))));
    }
    out
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

/// `ret_type` PRONTO pra `ResolvedFn`/`call_func` (o `IType*` real que `ADDR_EXEC` espera no 5º
/// arg) — MESMA lógica de duplo-dereference que `resolve_func` já usa internamente (`func+0x18` é
/// um ponteiro pro descritor; o `IType*` de verdade é `*(func+0x18)`, não `func+0x18` em si —
/// `fn_ret_type`/`fn_returns_void` acima fazem só 1 nível, correto pro USO DELES, mas ERRADO se
/// usado direto como `ret_type` de um `ResolvedFn` construído à mão). Achado 2026-08-03 (/goal):
/// `ResolvedFn{ret_type: null}` construído à mão pra funções NÃO-void (`ref<T>` de retorno) crasha
/// — `ADDR_EXEC` recebe um `ret_type` mentiroso (diz "void" pra uma função que não é) e corrompe
/// o processamento da chamada, não só a marshalling final do valor de retorno.
pub unsafe fn ret_type_of(func: *mut c_void) -> *mut c_void {
    if func.is_null() || !crate::gum::is_readable(func as *const c_void, 0x20) {
        return std::ptr::null_mut();
    }
    let rp = rd_ptr((func as *const u8).add(0x18));
    if rp.is_null() || !crate::gum::is_readable(rp as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    rd_ptr(rp as *const u8)
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

/// RED4ext.SDK `#472` (2026-08-11): `GameInstance::GetSystem(const rtti::IType*)` — método
/// VIRTUAL real. Header vendorizado (Windows) diz vtable slot +0x08 (slot 0=dtor, slot
/// 1=GetSystem) — mas a 1ª tentativa de chamar +0x08 DIRETO CROU (`EXC_BAD_ACCESS`,
/// `KERN_INVALID_ADDRESS at 0x0`, dentro de `run_cmd`/`game_instance_get_system`). Hipótese
/// forte (não confirmada ainda): mesma convenção Itanium dual-dtor já estabelecida neste
/// projeto pra OUTRAS classes (`cp77-macos-rtti-vtable-offsets`: Mac tem 2 dtor slots vs 1 no
/// MSVC — desloca tudo +0x08). `slot_off` agora é PARÂMETRO explícito (não mais hardcoded),
/// pra permitir diagnóstico seguro (dump antes de chamar) e retry com o offset certo.
pub unsafe fn game_instance_get_system(game: *mut c_void, itype: *mut c_void, slot_off: usize) -> *mut c_void {
    if !sane(game) || !sane(itype) {
        return std::ptr::null_mut();
    }
    if !crate::gum::is_readable(game as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let vt = (game as *const u64).read_unaligned() as *const u8;
    if !crate::gum::is_readable(vt as *const c_void, slot_off + 8) {
        return std::ptr::null_mut();
    }
    let slot = (vt.add(slot_off) as *const u64).read_unaligned();
    if slot == 0 || !crate::gum::is_readable(slot as *const c_void, 4) {
        return std::ptr::null_mut();
    }
    let f: extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void = std::mem::transmute(slot);
    f(game, itype)
}

/// Codeware `#58` (`ISerializable.ProcessPostLoad`, 2026-08-12, agente dedicado round 9) —
/// dispatch VIRTUAL GENÉRICO via vtable slot `+0x30`. `RED4ext.SDK/include/RED4ext/
/// ISerializable.hpp` documenta `virtual void PostLoad(const PostLoadParams&)` no slot Windows
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
/// `PostLoadParams{disablePreInitialization: bool, pad[7]}` (RED4ext.SDK, 8 bytes) é passado
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
    if !crate::gum::is_readable(obj as *const c_void, 8) {
        return false;
    }
    let vt = (obj as *const u64).read_unaligned() as *const u8;
    if vt.is_null() || !sane(vt as *mut c_void) || !crate::gum::is_readable(vt as *const c_void, POSTLOAD_SLOT + 8) {
        return false;
    }
    let slot = (vt.add(POSTLOAD_SLOT) as *const u64).read_unaligned();
    if slot == 0 || !crate::gum::is_readable(slot as *const c_void, 4) {
        return false;
    }
    // PostLoadParams{bool disablePreInitialization; uint8_t pad[7];} — 8 bytes, passado por x1.
    let params: [u8; 8] = [u8::from(disable_pre_init), 0, 0, 0, 0, 0, 0, 0];
    let f: extern "C" fn(*mut c_void, *const u8) = std::mem::transmute(slot);
    f(obj, params.as_ptr());
    true
}

/// RED4ext.SDK `#472` (2026-08-11, sessão nova) — via ALTERNATIVA mais segura que o vtable-call
/// acima: lê `systemMap: HashMap<IType*, Handle<IScriptable>>` @ `GameInstance+0x08` DIRETO (é
/// campo de OBJETO, não slot de vtable — não sofre o shift Itanium +0x08 que já causou o crash da
/// tentativa anterior por vtable). Zero execução de código do motor — só leitura de memória, mesmo
/// idioma já provado em `unregister_from_types_fastpath`/`selftest::wardrobe_forget_item` (HashMap
/// separate-chaining: `indexTable@+0x00, size@+0x08(u32), capacity@+0x0C(u32), nodeList.nodes@
/// +0x10, stride@+0x1C(u32)`). Diferença de chave: aqui a chave é um PONTEIRO cru (`IType*`), hash
/// = `FNV1a32(&itype_ptr as *const u8, 8)` (não o XOR-fold de `CName` usado pro types-map), node =
/// `{next:u32@0, hashedKey:u32@4, key:IType*@8, value:Handle<IScriptable>@0x10}` (stride 0x20).
/// Devolve `value.instance` (1º ponteiro de 8B do `Handle`, offset+0x10 do node) — null se não achar.
pub unsafe fn game_instance_get_system_by_map(game: *mut c_void, itype: *mut c_void) -> *mut c_void {
    const INVALID: u32 = 0xFFFF_FFFF;
    if !sane(game) || !sane(itype) {
        return std::ptr::null_mut();
    }
    let hm = (game as *const u8).add(0x08);
    if !crate::gum::is_readable(hm as *const c_void, 0x24) {
        return std::ptr::null_mut();
    }
    let index_table = (hm as *const u64).read_unaligned() as *const u32;
    let capacity = (hm.add(0x0C) as *const u32).read_unaligned();
    let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *const u8;
    let stride = (hm.add(0x1C) as *const u32).read_unaligned();
    if capacity == 0 || capacity > 1_000_000 || index_table.is_null() || nodes.is_null() || stride == 0 || stride > 256 {
        crate::log(&format!(
            "[gisysmap] systemMap@{hm:p} fora de faixa plausível (cap={capacity} stride={stride}) — abortado"
        ));
        return std::ptr::null_mut();
    }
    let key_bytes = (itype as u64).to_le_bytes();
    let hashed_key = bwms_hashes::fnv1a32(&key_bytes);
    let bucket = (hashed_key % capacity) as usize;
    if !crate::gum::is_readable(index_table.add(bucket) as *const c_void, 4) {
        return std::ptr::null_mut();
    }
    let mut idx = (index_table.add(bucket) as *const u32).read_unaligned();
    let mut guard = 0u32;
    while idx != INVALID && guard < capacity + 1 {
        guard += 1;
        let node = nodes.add(idx as usize * stride as usize);
        if !crate::gum::is_readable(node as *const c_void, stride as usize) {
            crate::log("[gisysmap] node ilegível no meio da cadeia — abortado");
            return std::ptr::null_mut();
        }
        let next = (node as *const u32).read_unaligned();
        let node_hashed = (node.add(4) as *const u32).read_unaligned();
        let node_key = (node.add(8) as *const u64).read_unaligned() as *mut c_void;
        if node_hashed == hashed_key && node_key == itype {
            let instance = (node.add(0x10) as *const u64).read_unaligned() as *mut c_void;
            crate::log(&format!("[gisysmap] achou: bucket={bucket} idx={idx} instance={instance:p}"));
            return instance;
        }
        idx = next;
    }
    crate::log(&format!("[gisysmap] itype={itype:p} não achado em systemMap (cap={capacity})"));
    std::ptr::null_mut()
}

/// Diagnóstico READ-ONLY pro `#472` — dump de TODAS as chaves reais do `systemMap` (walk
/// completo de todos os buckets, não busca por 1 chave). Resolve cada `IType*` chave de volta
/// pra nome via `type_name_getname` (chamada de vtable const, já provada segura em toda a
/// sessão) — deixa comparar visualmente contra o que `class_by_name("gamePlayerSystem")`
/// resolve, pra descobrir se são o MESMO ponteiro ou se o motor registra sob um tipo diferente
/// (ex.: implementação scriptada vs. classe base nativa). `n` limita quantas entradas logar
/// (proteção contra spam, cap real costuma ser pequeno — ~159 no boot já observado).
pub unsafe fn game_instance_dump_system_map(game: *mut c_void, n: usize) {
    if !sane(game) {
        crate::log("[gisysmapdump] game inválido");
        return;
    }
    let hm = (game as *const u8).add(0x08);
    if !crate::gum::is_readable(hm as *const c_void, 0x24) {
        crate::log("[gisysmapdump] systemMap ilegível");
        return;
    }
    let index_table = (hm as *const u64).read_unaligned() as *const u32;
    let capacity = (hm.add(0x0C) as *const u32).read_unaligned();
    let size = (hm.add(0x08) as *const u32).read_unaligned();
    let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *const u8;
    let stride = (hm.add(0x1C) as *const u32).read_unaligned();
    crate::log(&format!(
        "[gisysmapdump] hm@{hm:p} size={size} capacity={capacity} stride={stride} nodes@{nodes:p} — mostrando até {n}"
    ));
    if capacity == 0 || capacity > 1_000_000 || index_table.is_null() || nodes.is_null() || stride == 0 || stride > 256 {
        crate::log("[gisysmapdump] header fora de faixa plausível — abortado");
        return;
    }
    let mut shown = 0usize;
    for bucket in 0..capacity {
        if shown >= n {
            crate::log(&format!("[gisysmapdump] ...limite de {n} atingido, parando"));
            break;
        }
        if !crate::gum::is_readable(index_table.add(bucket as usize) as *const c_void, 4) {
            continue;
        }
        let mut idx = (index_table.add(bucket as usize) as *const u32).read_unaligned();
        let mut guard = 0u32;
        while idx != 0xFFFF_FFFF && guard < capacity + 1 {
            guard += 1;
            let node = nodes.add(idx as usize * stride as usize);
            if !crate::gum::is_readable(node as *const c_void, stride as usize) {
                crate::log("[gisysmapdump] node ilegível, abortando cadeia");
                break;
            }
            let next = (node as *const u32).read_unaligned();
            let node_hashed = (node.add(4) as *const u32).read_unaligned();
            let node_key = (node.add(8) as *const u64).read_unaligned() as *mut c_void;
            let instance = (node.add(0x10) as *const u64).read_unaligned() as *mut c_void;
            let name_hash = if sane(node_key) { type_name_getname(node_key) } else { 0 };
            let name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
            crate::log(&format!(
                "[gisysmapdump]   bucket={bucket} idx={idx} hashed={node_hashed:#x} key={node_key:p} name='{name}' instance={instance:p}"
            ));
            shown += 1;
            idx = next;
            if shown >= n {
                break;
            }
        }
    }
    crate::log(&format!("[gisysmapdump] fim — {shown} entrada(s) mostrada(s) de size={size}"));
}

/// Diagnóstico READ-ONLY (zero chamada) pro `#472` — dado o ponteiro `game` (mesmo C++
/// `GameInstance*` real), lê o vtable e devolve os N primeiros slots crus + os 16 bytes de
/// prólogo de cada um, pra julgar plausibilidade ANTES de tentar chamar (mesmo padrão do
/// diagnóstico `#55`/ForceStartNode).
pub unsafe fn game_instance_vtable_dump(game: *mut c_void, n: usize) -> Vec<(usize, u64, [u8; 16])> {
    let mut out = Vec::new();
    if !sane(game) || !crate::gum::is_readable(game as *const c_void, 8) {
        return out;
    }
    let vt = (game as *const u64).read_unaligned() as *const u8;
    for i in 0..n {
        let off = i * 8;
        if !crate::gum::is_readable(vt.add(off) as *const c_void, 8) {
            break;
        }
        let slot = (vt.add(off) as *const u64).read_unaligned();
        let mut bytes = [0u8; 16];
        if slot != 0 && crate::gum::is_readable(slot as *const c_void, 16) {
            core::ptr::copy_nonoverlapping(slot as *const u8, bytes.as_mut_ptr(), 16);
        }
        out.push((off, slot, bytes));
    }
    out
}

/// Resolve uma função RED por classe+método, subindo a cadeia de heranças.
/// `true` só na PRIMEIRA vez que este par (tag, chave) aparece. Uma resolução que dá certo é
/// notícia uma vez; repetida a cada tick vira ruído que esconde o resto do log — e o caminho do
/// tick (munição infinita) chama isto de segundo em segundo.
fn first_time(tag: &str, key: &str) -> bool {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(|| std::sync::Mutex::new(std::collections::HashSet::new()));
    seen.lock().unwrap().insert(format!("{tag}|{key}"))
}

/// Resolve tentando VÁRIOS nomes em várias classes. Existe porque as funções definidas em
/// redscript não entram na RTTI com o nome curto: entram com a ASSINATURA colada
/// (`CreateStatModifier;gamedataStatTypegameStatModifierTypeFloat`), enquanto as nativas do
/// engine entram com o nome cru. Procurar só o nome curto acha as nativas e erra as scriptadas —
/// foi exatamente o que fez `CreateStatModifier` e `GetActiveWeapon` "não resolverem".
/// Loga qual par (classe, nome) pegou, pra o acerto ficar registrado em vez de ser refeito.
pub unsafe fn resolve_any_name(
    reg: &Registry,
    classes: &[&str],
    names: &[&str],
    tag: &str,
) -> Option<ResolvedFn> {
    for c in classes {
        let cls = reg.class_by_name(c);
        if cls.is_null() {
            continue;
        }
        for n in names {
            if let Some(f) = resolve_in_class(cls, n) {
                if first_time(tag, &format!("{c}.{n}")) {
                    crate::log(&format!("[{tag}] resolvido: {c}.{n} (static={})", f.is_static));
                }
                return Some(f);
            }
        }
    }
    // Última tentativa: função GLOBAL. Statics de redscript nem sempre entram na tabela da
    // classe — `funcs RPGManager Modifier` mostrou que `CreateStatModifier` não está lá — e o
    // registro global é onde as declaradas em `.reds` costumam parar.
    for n in names {
        let g = resolve_global_function_robust(reg, n);
        if !g.is_null() {
            let gb = g as *const u8;
            let rp = rd_ptr(gb.add(0x18));
            let ret_type = if rp.is_null() { std::ptr::null_mut() } else { rd_ptr(rp as *const u8) };
            if first_time(tag, n) {
                crate::log(&format!("[{tag}] resolvido como função GLOBAL: {n}"));
            }
            return Some(ResolvedFn { func: g, ret_type, is_static: true });
        }
    }
    crate::log(&format!(
        "[{tag}] não resolveu: classes={classes:?} nomes={names:?} (nem como global)"
    ));
    None
}

/// Lista funções GLOBAIS cujo nome contém `filter`. Par de `list_functions` pro registro global —
/// juntos respondem "como este método se chama de verdade" sem recompilar.
pub unsafe fn list_global_functions(reg: &Registry, filter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (entries, n) = reg.get_global_functions();
    if entries.is_null() || n == 0 || n > 200_000 || !crate::gum::is_readable(entries, 8) {
        return out;
    }
    let base = entries as *const u8;
    let f_lower = filter.to_lowercase();
    for i in 0..n {
        let slot = base.add(i * 8);
        if !crate::gum::is_readable(slot as *const c_void, 8) {
            break;
        }
        let f = rd_ptr(slot) as *const u8;
        if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
            continue;
        }
        for h in [rd_u64(f.add(0x10)), rd_u64(f.add(0x08))] {
            if h == 0 {
                continue;
            }
            let nm = crate::cname::resolve_cname(h);
            if !nm.is_empty() && (f_lower.is_empty() || nm.to_lowercase().contains(&f_lower)) {
                out.push(nm);
            }
        }
    }
    out
}

/// Lista os nomes das funções de uma classe (instância + estáticas, subindo os pais). Ferramenta
/// de diagnóstico: "não resolveu" passa a ser uma pergunta respondível no console, em vez de uma
/// recompilação pra descobrir como o método se chama de verdade.
pub unsafe fn list_functions(cls0: *mut c_void, filter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cls = cls0;
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
            if fp.is_null() || n >= 20_000 || !crate::gum::is_readable(fp as *const c_void, 8) {
                continue;
            }
            for i in 0..n as usize {
                let slot = fp.add(i * 8);
                if !crate::gum::is_readable(slot as *const c_void, 8) {
                    break;
                }
                let f = rd_ptr(slot) as *const u8;
                if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
                    continue;
                }
                let nm = crate::cname::resolve_cname(rd_u64(f.add(0x10)));
                if filter.is_empty() || nm.to_lowercase().contains(&filter.to_lowercase()) {
                    out.push(format!("{}{nm}", if off == 0x58 { "static " } else { "" }));
                }
            }
        }
        cls = rd_ptr(clsb.add(0x10));
    }
    out
}

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

/// `cw-reflection-class-api` (2026-07-24): lista os `CProperty*` declarados DIRETAMENTE em `cls`
/// (mesmo nível que `propdump`/`dump_props` já usam — `props_array`, PROVADO in-game há sessões).
/// NÃO sobe pra parents (cada classe da cadeia tem sua própria lista; um mod pode andar
/// `GetParent()` pra ver as herdadas — mesmo modelo do Codeware real). Vetor vazio se a classe
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

/// Codeware `ReflectionClass.GetFunctions()`/`GetStaticFunctions()` (`PENDENCIAS-UNIFICADAS.md`
/// item #51, "listar TODOS os membros nunca confirmado") — espelha `class_own_properties`, mas
/// pros arrays `funcs@+0x48`(instância)/`staticFuncs@+0x58` já andados por `resolve_in_class`
/// (que só busca POR NOME); aqui devolve TODOS os ponteiros de uma vez. `static_only` escolhe qual
/// dos 2 arrays.
pub unsafe fn class_own_functions(cls: *mut c_void, static_only: bool) -> Vec<*mut c_void> {
    let mut out = Vec::new();
    let off = if static_only { 0x58usize } else { 0x48usize };
    if !crate::gum::is_readable(cls as *const c_void, 0x60) {
        return out;
    }
    let clsb = cls as *const u8;
    let fp = rd_ptr(clsb.add(off)) as *const u8;
    let n = rd_u32(clsb.add(off + 8));
    if fp.is_null() || n > 20_000 || !crate::gum::is_readable(fp as *const c_void, 8) {
        return out;
    }
    for i in 0..n as usize {
        let slot = fp.add(i * 8);
        if !crate::gum::is_readable(slot as *const c_void, 8) {
            break;
        }
        let f = rd_ptr(slot) as *mut c_void;
        // CRASH REAL achado 2026-08-09 (`Cyberpunk2077-2026-08-09-130720.ips`, EXC_BAD_ACCESS em
        // `probe_class_own_functions`): a 1ª versão só validava que o SLOT do array era legível
        // (`rd_ptr_chk` no array), nunca que o PONTEIRO GUARDADO ali (o `f` em si) apontava pra
        // memória válida — omissão vs. `resolve_in_class` (já provado), que SEMPRE valida
        // `is_readable(f, 0x20)` antes de tocar o objeto. Fix: mesma validação aqui, `continue`
        // (pula a entrada suja) em vez de assumir válido.
        if f.is_null() || !crate::gum::is_readable(f, 0x20) {
            continue;
        }
        out.push(f);
    }
    out
}

/// Comando `funclistdump <classe> [n]` — lista as primeiras `n` (default 20) funções PRÓPRIAS
/// (instância+estática) da classe, com nome+flags decodificados (`function_flags`).
pub unsafe fn probe_class_own_functions(reg: &Registry, class: &str, n: usize) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log(&format!("[funclistdump] classe '{class}' não resolveu"));
        return;
    }
    let inst = class_own_functions(cls, false);
    let stat = class_own_functions(cls, true);
    crate::log(&format!(
        "[funclistdump] {class}: {} método(s) de instância, {} estático(s), mostrando até {n} de cada",
        inst.len(),
        stat.len()
    ));
    for (label, list) in [("inst", &inst), ("static", &stat)] {
        for f in list.iter().take(n) {
            if !crate::gum::is_readable((*f as *const u8) as *const c_void, 0x20) {
                crate::log(&format!("[funclistdump]   [{label}] <ponteiro ilegível, pulado>"));
                continue;
            }
            let name_hash = rd_u64((*f as *const u8).add(0x10)); // CBaseFunction.shortName@+0x10
            let name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
            let flags = function_flags(*f).map(decode_function_flags).unwrap_or_else(|| "?".into());
            crate::log(&format!("[funclistdump]   [{label}] {name}: {flags}"));
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
    new_object_from_class_named(cls, class_name)
}

/// `Codeware.Reflection.ReflectionClass.MakeHandle()` (`PENDENCIAS-UNIFICADAS.md` item #51) —
/// mesmo núcleo de `new_object`, mas a partir de um `CClass*` JÁ RESOLVIDO (o caso real de
/// `MakeHandle`: um mod já tem o ponteiro opaco de uma classe, via `BwmsReflGetClass` ou
/// `GetDerivedClasses`, não o nome de novo). Sem nome pra log/trace (usa o endereço cru).
pub unsafe fn new_object_from_class(cls: *mut c_void) -> *mut c_void {
    new_object_from_class_named(cls, "<sem nome>")
}

unsafe fn new_object_from_class_named(cls: *mut c_void, class_name: &str) -> *mut c_void {
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
    // GUARD DE DIAGNÓSTICO (2026-08-21) — a assinatura MECÂNICA dos crashes de boot desta sessão,
    // provada por disassembly, é DESPACHO VIRTUAL SOBRE VTABLE NULA: o frame 0 repetido
    // (`0x10001afd4`) é um thunk de 3 instruções `ldr x8,[x0]` / `ldr x2,[x8,#0x20]` / `br x2`,
    // e o endereço de falta registrado é o offset EXATO do slot (confere para 3 dos 4 endereços
    // distintos). Como o primeiro `ldr` passa, `this` é legível e quem está nulo é o ponteiro de
    // vtable DENTRO do objeto.
    //
    // Um objeto que deriva de `IScriptable` é polimórfico: TEM que sair do `Construct` com vtable
    // não-nula. Se sair nula, ele é uma mina — e quem crasha depois é o MOTOR, ao despachar, sem
    // nenhum símbolo nosso na pilha. É exatamente por isso que esses crashes vinham sendo
    // triados como "ambiental": quem CRIA e quem DESPACHA são momentos diferentes.
    //
    // Aqui só DIAGNOSTICA (zero mudança de comportamento — o objeto segue devolvido igual). Uma
    // recusa mudaria caminhos hoje provados sem evidência de que algum deles produz isto, e a
    // pergunta em aberto é justamente "produz ou não?". Se esta linha nunca aparecer, a hipótese
    // "o BWMS forja objeto com vtable nula" fica REFUTADA para os caminhos de forja; se aparecer,
    // ela vem com o nome da classe culpada.
    let is_scriptable = derives_from_iscriptable(cls);
    if crate::null_vtable_is_anomalous(is_scriptable, rd_ptr(mem as *const u8) as usize) {
        crate::log_anomaly(&format!(
            "[anomalia] VTABLE NULA apos Construct: class={class_name} cls={cls:p} obj={mem:p} size={size}"
        ));
    }
    // IScriptable.nativeType @ obj+0x30 = CClass*. É o ÚNICO passo que CClass::CreateInstance
    // (inlined, sem símbolo) faz a mais que o Construct puro: o ctor do IScriptable zera
    // nativeType=null → GetType() cai no fallback GetNativeType() e class_of devolve a BASE
    // IScriptable em vez da derivada. Escrever cls aqui faz GetType devolver a classe certa
    // → resolve_prop acha os campos. SÓ p/ quem deriva de IScriptable (struct puro não tem
    // esse campo; escrever 0x30 corromperia). Offset de DADO (0x30) NÃO sofre o shift Itanium.
    if is_scriptable {
        core::ptr::write_unaligned((mem as *mut u8).add(0x30) as *mut *mut c_void, cls);
        crate::trace(&format!(
            "new_object: {class_name} nativeType set @ {mem:p} cls={cls:p}"
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

/// Constrói um `wref<T>` REAL (RED4ext.SDK item #387) — mesmos 16 bytes `{instance,
/// refCountBlock}` de `make_handle`, mas incrementa o contador FRACO (`weakRefs`, u32 @
/// `refCountBlock+0x04`) em vez do forte. Layout confirmado via `RED4ext.SDK/Memory/SharedPtr.hpp`:
/// `RefCnt{strongRefs@0x00(u32), weakRefs@0x04(u32)}` — MESMO bloco que `make_handle`/`Handle_ctor`
/// já usa e já é real (RE confirmada, `cw-callback-handler`, 2026-08-10). `IncWeakRef()` real é
/// um incremento atômico puro (sem endereço nativo, `InterlockedIncrement`), então replicável
/// direto em Rust sem nenhuma RE de endereço nova — só a leitura do bloco (via `make_handle`,
/// que a própria rotina do motor já sabe achar/reusar o bloco existente de um objeto, em vez de
/// duplicar — mesmo comportamento já documentado pra `make_handle` chamado 2x no mesmo objeto).
/// `DecWeakRef` (limpeza final) SIM precisaria de endereço nativo — não usado aqui (mesma
/// filosofia já aceita no projeto pra handles forjados/emprestados: nunca desaloca).
pub unsafe fn make_weak_handle(out: *mut c_void, obj: *mut c_void) {
    if out.is_null() {
        return;
    }
    if obj.is_null() {
        core::ptr::write_unaligned(out as *mut u64, 0u64);
        core::ptr::write_unaligned((out as *mut u64).add(1), 0u64);
        return;
    }
    let mut tmp = [0u8; 16];
    make_handle(tmp.as_mut_ptr() as *mut c_void, obj);
    let instance = (tmp.as_ptr() as *const u64).read_unaligned();
    let refcount_block = (tmp.as_ptr().add(8) as *const u64).read_unaligned();
    if refcount_block != 0 {
        let weak_ptr = (refcount_block as *mut u8).add(4) as *mut std::sync::atomic::AtomicU32;
        (*weak_ptr).fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    core::ptr::write_unaligned(out as *mut u64, instance);
    core::ptr::write_unaligned((out as *mut u64).add(1), refcount_block);
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

/// Lista métodos RTTI (instância+estáticos) de uma classe, subindo a cadeia de parents.
/// Cada CClassFunction* em CClass+0x48/0x58; nome = CName hash em func+0x10 resolvido pelo pool.
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
        if !crate::gum::is_readable(cur as *const c_void, 0x60) { break; }
        let clsb = cur as *const u8;
        for (off, kind) in [(0x48usize, "inst"), (0x58usize, "stat")] {
            let fp = rd_ptr(clsb.add(off)) as *const u8;
            let cap = rd_u32(clsb.add(off + 8));  // DynArray: {ptr@+0, cap@+8, sz@+12}
            let n   = rd_u32(clsb.add(off + 12)); // sz = count real
            s.push_str(&format!("  [depth={depth} {kind}] fp={fp:p} cap={cap} sz={n}\n"));
            if fp.is_null() || n == 0 || n > 2000 || !crate::gum::is_readable(fp as *const c_void, 8) { continue; }
            s.push_str(&format!("  [depth={depth} {kind}] {n} funcs:\n"));
            for i in 0..n.min(200) as usize {
                let slot = fp.add(i * 8);
                if !crate::gum::is_readable(slot as *const c_void, 8) { break; }
                let f = rd_ptr(slot) as *const u8;
                if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) { continue; }
                let name_h = rd_u64(f.add(0x10));
                let name = crate::cname::resolve_cname(name_h);
                s.push_str(&format!("    [{i:03}] {name} ({name_h:#018x})\n"));
            }
        }
        cur = rd_ptr(clsb.add(0x10)); // parent CClass*
    }
    s
}

/// RED4ext.SDK item #55 (`PENDENCIAS-UNIFICADAS.md`): `GetFunction(CName)` (vtbl+0x30, já usado
/// por `register::get_function`) só acha o que foi registrado via `RegisterFunction` — funções
/// GLOBAIS puramente ESCRIPTADAS (declaradas num `.reds`, nunca passaram pelo `RegisterFunction`
/// do lado Rust) não resolvem por ele. Fallback: varre `GetGlobalFunctions` (vtbl+0x48, já
/// PROVADO — "4435 funções globais") e casa por nome, comparando tanto `name`@+0x10 (short,
/// mesmo campo que `dump_class_funcs` já lê) quanto `fullName`@+0x08 (o outro candidato citado
/// no item) — qualquer um batendo conta como achado, sem exigir saber de antemão qual campo o
/// `name` pedido representa. Read-only, nunca falha alto (retorna null se não achar).
pub unsafe fn resolve_global_function_robust(reg: &Registry, name: &str) -> *mut c_void {
    let target = crate::cname::cname(name);
    let (entries, n) = reg.get_global_functions();
    if entries.is_null() || n == 0 || n > 200_000 || !crate::gum::is_readable(entries, 8) {
        return std::ptr::null_mut();
    }
    let base = entries as *const u8;
    for i in 0..n {
        let slot = base.add(i * 8);
        if !crate::gum::is_readable(slot as *const c_void, 8) {
            break;
        }
        let f = rd_ptr(slot) as *const u8;
        if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
            continue;
        }
        let name_h = rd_u64(f.add(0x10));
        let fullname_h = rd_u64(f.add(0x08));
        if name_h == target || fullname_h == target {
            return f as *mut c_void;
        }
    }
    // 2ª passada — MODULE-SCOPING (RED4ext.SDK `#55`, 2026-08-21). A 1ª passada compara HASH, então
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
        if !crate::gum::is_readable(slot as *const c_void, 8) {
            break;
        }
        let f = rd_ptr(slot) as *const u8;
        if f.is_null() || !crate::gum::is_readable(f as *const c_void, 0x20) {
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

/// Núcleo PURO da decisão de nome do `resolve_global_function_robust` (RED4ext.SDK `#55`) —
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

    let mut res = [0u8; 0x20];
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
    // DIAGNÓSTICO (2026-08-07): sentinela em vez de zero — distingue "a nativa escreveu
    // zero de propósito" de "a_out nunca foi tocado" (early-return dentro do exec_replacement
    // reentrante: RUST_OV_CNAME/route_native/watched_before podem suprimir antes do ORIG_EXEC real).
    let mut out = [0xEEu8; 16];
    let exec: extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        std::mem::transmute(rebase(ADDR_EXEC));
    exec(
        func,
        ctx,
        fr.as_mut_ptr() as *mut c_void,
        out.as_mut_ptr() as *mut c_void,
        ret,
    );
    let untouched = out.iter().all(|&b| b == 0xEE);
    crate::log(&format!(
        "[from_tdbid] '{name}' ctx={ctx:p} out={out:02x?} untouched={untouched}"
    ));
    Some(out)
}

/// Lê o FromTDBID capturado pela sonda em /tmp/cp77-fromtd.txt (fn/ctx/ret).
unsafe fn read_fromtd() -> Option<(*mut c_void, *mut c_void, *mut c_void)> {
    // Captura NATIVA (sem frida): o hook do executor publica fn/ctx/ret do FromTDBID em
    // atomics quando o jogo o chama. Antes vinha de /tmp/cp77-fromtd.txt escrito pela
    // sonda frida — endereço de OUTRA sessão (ASLR) = morto = crash.
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

// ===== RED4ext.SDK — introspecção fina de flags (`PENDENCIAS-UNIFICADAS.md`, prosa "Restante" da
// seção RED4ext.SDK: "CClass::Flags 11 bits, CProperty.flags 13 bits, CBaseFunction.flags 13
// bits — maioria nunca lida/setada"). Os 3 offsets vêm dos headers vendorizados
// (`RTTITypes.hpp`/`Scripting/CProperty.hpp`/`Scripting/Functions.hpp`) — `CProperty.flags@0x28`
// já era lido (só o bit `inValueHolder`, ver `find_prop_exact`); `CBaseFunction.flags@0xA8` já
// era ESCRITO (`register.rs::build_native_func`, `FLAG_NATIVE`/`FLAG_STATIC`). O gap real é expor
// a decodificação COMPLETA dos bits nomeados, nunca feita antes pra nenhum dos 3.

/// `CClass::Flags` (11 bits nomeados + 1 campo reservado de 21 bits) @ CClass+0x70.
pub unsafe fn class_flags(cls: *mut c_void) -> Option<u32> {
    rd_u32_chk((cls as *const u8).add(0x70))
}
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
pub unsafe fn property_flags(prop: *mut c_void) -> Option<u64> {
    if !crate::gum::is_readable((prop as *const u8).add(0x28) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64((prop as *const u8).add(0x28)))
}
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
pub unsafe fn function_flags(func: *mut c_void) -> Option<u32> {
    rd_u32_chk((func as *const u8).add(0xA8))
}
/// RED4ext.SDK #186 (lado escrita, 2026-08-11): escreve `CBaseFunction::flags@+0xA8` — mesmo
/// campo/offset já lido/escrito com sucesso (isNative/isStatic, confirmado ao vivo 2026-08-10).
/// Campo é metadado puro (Reflection/introspecção) — despacho de chamada nunca lê nenhum destes
/// bits (nossas natives são roteadas via `NATIVE_ROUTES`, hook próprio, nunca a via nativa do
/// engine que teoricamente poderia inspecionar isEvent/isExec/isQuest/isThreadsafe).
pub unsafe fn function_flags_write(func: *mut c_void, f: u32) -> bool {
    let p = (func as *mut u8).add(0xA8);
    if !crate::gum::is_readable(p as *const c_void, 4) {
        return false;
    }
    (p as *mut u32).write_unaligned(f);
    true
}
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
pub unsafe fn probe_class_flags(reg: &Registry, name: &str) {
    let cls = reg.class_by_name(name);
    if cls.is_null() {
        crate::log(&format!("[classflags] classe '{name}' não resolveu"));
        return;
    }
    match class_flags(cls) {
        Some(f) => crate::log(&format!("[classflags] {name}: {}", decode_class_flags(f))),
        None => crate::log(&format!("[classflags] {name}: flags@0x70 ilegível")),
    }
}

/// Comando `propflags <classe> <prop>` — resolve a propriedade e decodifica `CProperty::Flags`.
pub unsafe fn probe_property_flags(reg: &Registry, class: &str, prop: &str) {
    let p = find_property(reg, class, prop);
    if p.is_null() {
        crate::log(&format!("[propflags] {class}.{prop} não resolveu"));
        return;
    }
    match property_flags(p) {
        Some(f) => crate::log(&format!("[propflags] {class}.{prop}: {}", decode_property_flags(f))),
        None => crate::log(&format!("[propflags] {class}.{prop}: flags@0x28 ilegível")),
    }
}

// ===== `CClass.defaults` (`PENDENCIAS-UNIFICADAS.md`, prosa "Restante" do RED4ext.SDK: "valor-
// default de propriedade (CClass.defaults)") — `Map<CName, Variant*> defaults` @ CClass+0x158
// (header vendorizado `RTTITypes.hpp`; layout de `Map<K,V>` confirmado por `Map.hpp`:
// `keys:DynArray<K>@+0x00, values:DynArray<T>@+0x10, flags:i32@+0x20`, cada `DynArray` no layout
// já provado em todo o resto do projeto — `entries@+0x00, cap(u32)@+0x08, size(u32)@+0x0C`).
// Nenhuma introspecção de valor-default de propriedade existia antes — o campo nunca foi lido.

/// Varre `CClass.defaults` (varredura LINEAR — o array pode estar `NotSorted` no momento da
/// leitura externa, `Map::Sort()` só roda lazy sob demanda do próprio motor; linear é sempre
/// correto, custo irrelevante pro tamanho típico de `defaults`) procurando `prop_name`. Devolve os
/// 24 bytes brutos do `Variant` apontado (mesmo layout já usado por
/// `read_params_consuming_with_variants`/`variant_type_cname`/`variant_inline_u64`).
pub unsafe fn class_default_value(cls: *mut c_void, prop_name: &str) -> Option<[u8; 24]> {
    const DEFAULTS_OFF: usize = 0x158;
    let base = cls as *const u8;
    if !crate::gum::is_readable(base.add(DEFAULTS_OFF) as *const c_void, 0x28) {
        return None;
    }
    let keys_entries = rd_ptr(base.add(DEFAULTS_OFF)) as *const u64; // DynArray<CName>
    let keys_size = rd_u32(base.add(DEFAULTS_OFF + 0x0C));
    let values_entries = rd_ptr(base.add(DEFAULTS_OFF + 0x10)) as *const *mut u8; // DynArray<Variant*>
    if keys_entries.is_null() || values_entries.is_null() || keys_size == 0 || keys_size > 100_000 {
        return None;
    }
    let target = cname(prop_name);
    if !crate::gum::is_readable(keys_entries as *const c_void, keys_size as usize * 8) {
        return None;
    }
    for i in 0..keys_size as usize {
        if keys_entries.add(i).read_unaligned() == target {
            let vp = values_entries.add(i).read_unaligned();
            if vp.is_null() || !crate::gum::is_readable(vp as *const c_void, 24) {
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
pub unsafe fn probe_class_default(reg: &Registry, class: &str, prop: &str) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log(&format!("[classdefault] classe '{class}' não resolveu"));
        return;
    }
    match class_default_value(cls, prop) {
        Some(buf) => {
            let type_hash = variant_type_cname(&buf);
            let type_name = crate::cname::resolve_cname(type_hash);
            let inline_val = variant_inline_u64(&buf);
            crate::log(&format!(
                "[classdefault] {class}.{prop}: tipo='{type_name}' (hash={type_hash:#x}) valor_inline_u64={inline_val:#x} ({inline_val})"
            ));
        }
        None => crate::log(&format!(
            "[classdefault] {class}.{prop}: sem default registrado (ou 'defaults' vazio/ilegível)"
        )),
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

/// `rtti::IType::GetType() -> ERTTIType` (macOS: vtbl+0x28) — devolve o byte cru do enum
/// (`RED4ext.SDK/include/RED4ext/RTTITypes/ERTTIType.hpp`: 0=Name,1=Fundamental,2=Class,3=Array,
/// 4=Simple,5=Enum,6=StaticArray,7=NativeArray,8=Pointer,9=Handle,10=WeakHandle,
/// 11=ResourceReference,12=ResourceAsyncReference,13=BitField,14=LegacySingleChannelCurve,
/// 15=ScriptReference,16=FixedArray).
pub unsafe fn type_kind(ty: *mut c_void) -> Option<u8> {
    if !sane(ty) || !crate::gum::is_readable(ty as *const c_void, 8) {
        return None;
    }
    let vt = rd_ptr(ty as *const u8) as *const u8;
    if vt.is_null() || !crate::gum::is_readable(vt.add(0x28) as *const c_void, 8) {
        return None;
    }
    let f = rd_ptr(vt.add(0x28));
    if !sane(f) {
        return None;
    }
    let call: extern "C" fn(*mut c_void) -> u8 = std::mem::transmute(f);
    Some(call(ty))
}

pub fn decode_ertti_type(k: u8) -> &'static str {
    match k {
        0 => "Name",
        1 => "Fundamental",
        2 => "Class",
        3 => "Array",
        4 => "Simple",
        5 => "Enum",
        6 => "StaticArray",
        7 => "NativeArray",
        8 => "Pointer",
        9 => "Handle",
        10 => "WeakHandle",
        11 => "ResourceReference",
        12 => "ResourceAsyncReference",
        13 => "BitField",
        14 => "LegacySingleChannelCurve",
        15 => "ScriptReference",
        16 => "FixedArray",
        _ => "?",
    }
}

/// Codeware `#197` (`ReflectionType.GetInnerType()`, 2026-08-10) — "o elemento interno de um tipo
/// composto" (array/handle/weak-handle: `T` de `array<T>`/`ref<T>`/`wref<T>`). Achado que evita
/// vtable por completo: `CRTTIHandleType`/`CRTTIWeakHandleType`/`CRTTIBaseArrayType` (RTTITypes.hpp)
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
    if !crate::gum::is_readable((ty as *const u8).add(0x10) as *const c_void, 8) {
        return None;
    }
    let inner = rd_ptr((ty as *const u8).add(0x10));
    if sane(inner) { Some(inner) } else { None }
}

/// Comando `innertype <typeName>` — resolve um IType por nome (via `get_type`, ex.
/// `"array:Float"`/`"handle:IScriptable"`) e mostra a categoria + o nome do tipo interno.
pub unsafe fn probe_inner_type(reg: &Registry, type_name: &str) {
    let ty = crate::register::get_type(reg, type_name);
    if !sane(ty) {
        crate::log(&format!("[innertype] GetType('{type_name}') não resolveu"));
        return;
    }
    let kind = type_kind(ty).map(decode_ertti_type).unwrap_or("?");
    match get_inner_type(ty) {
        Some(inner) => {
            let name_hash = type_name_getname(inner);
            let name = crate::cname::resolve_cname(name_hash);
            crate::log(&format!("[innertype] '{type_name}' categoria={kind} innerType='{name}'"));
        }
        None => crate::log(&format!("[innertype] '{type_name}' categoria={kind} sem innerType (tipo não-composto ou leitura falhou)")),
    }
}

/// Comando `proptypekind <classe> <prop>` — resolve a propriedade, lê `CProperty.type@+0x00`
/// (`rtti::IType*`) e decodifica sua categoria via `GetType()`. Responde exatamente a pergunta
/// "que tipo de wrapper é esse campo" (array/handle/weak-handle/resource-ref/etc.) sem precisar
/// de um wrapper dedicado por família — um único mecanismo genérico cobre as ~9 categorias.
pub unsafe fn probe_property_type_kind(reg: &Registry, class: &str, prop: &str) {
    let p = find_property(reg, class, prop);
    if p.is_null() {
        crate::log(&format!("[proptypekind] {class}.{prop} não resolveu"));
        return;
    }
    let ty = rd_ptr(p as *const u8); // CProperty.type@+0x00
    match type_kind(ty) {
        Some(k) => crate::log(&format!(
            "[proptypekind] {class}.{prop}: categoria={} (raw={k})",
            decode_ertti_type(k)
        )),
        None => crate::log(&format!("[proptypekind] {class}.{prop}: type@+0x00 ilegível ou GetType falhou")),
    }
}

/// Comando `proptypekindall <classe> [n]` — lista as primeiras `n` (default 20) props PRÓPRIAS da
/// classe (via `class_own_properties`, já provado) com nome+categoria — valida `type_kind` contra
/// VÁRIAS categorias reais de uma vez (não só 1 propriedade escolhida à mão).
pub unsafe fn probe_property_type_kind_all(reg: &Registry, class: &str, n: usize) {
    let cls = reg.class_by_name(class);
    if cls.is_null() {
        crate::log(&format!("[proptypekindall] classe '{class}' não resolveu"));
        return;
    }
    let props = class_own_properties(cls);
    crate::log(&format!("[proptypekindall] {class}: {} prop(s) próprias, mostrando até {n}", props.len()));
    for p in props.iter().take(n) {
        let name_hash = rd_u64((*p as *const u8).add(0x08));
        let name = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
        let ty = rd_ptr(*p as *const u8);
        let kind = type_kind(ty).map(decode_ertti_type).unwrap_or("?");
        crate::log(&format!("[proptypekindall]   {name}: {kind}"));
    }
}

/// Codeware `#199` (`ResourceHelper.IsReferenceLoaded`/`GetReferenceResource`): `proptypekindall`
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
        crate::log(&format!("[scan11] âncora '{anchor}' não resolveu"));
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
            if let Some(11) = type_kind(ty) {
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

    crate::log(&format!(
        "[scan11] âncora='{anchor}' classes escaneadas={classes_scanned} hits type_kind==11(ResourceReference)={} : {hits:?}",
        hits.len()
    ));
}

/// Codeware `Reflection.ReflectionFunc` — "introspecção de assinatura" (`PENDENCIAS-UNIFICADAS.md`,
/// prosa Codeware "Reflection... introspecção de assinatura"). A fonte C++ real
/// (`ReflectionFunc.hpp`) mostra que `GetReturnType`/`GetParameters`/`IsNative`/`IsStatic` são só
/// leituras de campos JÁ documentados neste projeto: `returnType->type` (= `ResolvedFn.ret_type`,
/// já existe), `params: DynArray<CProperty*>@+0x28` (nunca listado, só usado indiretamente),
/// `flags.isNative/isStatic` (= `function_flags`/`decode_function_flags`, fechado nesta mesma
/// sessão). Comando `funcsig <classe> <método>` — junta tudo numa única dump.
pub unsafe fn probe_function_signature(reg: &Registry, class: &str, method: &str) {
    let rf = match resolve_func(reg, class, method) {
        Some(rf) => rf,
        None => {
            crate::log(&format!("[funcsig] {class}.{method} não resolveu (nem na cadeia de parents)"));
            return;
        }
    };
    let flags_s = function_flags(rf.func).map(decode_function_flags).unwrap_or_else(|| "?".into());
    let ret_kind = if rf.ret_type.is_null() {
        "Void".to_string()
    } else {
        type_kind(rf.ret_type).map(decode_ertti_type).unwrap_or("?").to_string()
    };
    crate::log(&format!(
        "[funcsig] {class}.{method}: flags={flags_s} is_static(walk)={} retorno={ret_kind}",
        rf.is_static
    ));
    // params: DynArray<CProperty*> @ func+0x28 (mesmo layout de props_array — entries@0,cap@8,size@0xC).
    let base = rf.func as *const u8;
    if !crate::gum::is_readable(base.add(0x28) as *const c_void, 0x10) {
        crate::log("[funcsig]   params@+0x28 ilegível");
        return;
    }
    let entries = rd_ptr(base.add(0x28)) as *const *mut c_void;
    let size = rd_u32(base.add(0x34));
    if entries.is_null() || size > 256 {
        crate::log(&format!("[funcsig]   {size} parâmetro(s) — array inválido ou vazio"));
        return;
    }
    crate::log(&format!("[funcsig]   {size} parâmetro(s):"));
    for i in 0..size as usize {
        let Some(prop) = rd_ptr_chk(entries.add(i) as *const u8) else { break };
        if prop.is_null() {
            continue;
        }
        let name_hash = rd_u64((prop as *const u8).add(0x08));
        let pname = crate::cname::name_of(name_hash).unwrap_or_else(|| format!("(hash={name_hash:#x})"));
        let pty = rd_ptr(prop as *const u8); // CProperty.type@+0x00
        let pkind = type_kind(pty).map(decode_ertti_type).unwrap_or("?");
        crate::log(&format!("[funcsig]     [{i}] {pname}: {pkind}"));
    }
}

/// Comando `funcflags <classe> <método>` — resolve o método (sobe a cadeia de parents) e
/// decodifica `CBaseFunction::Flags`.
pub unsafe fn probe_function_flags(reg: &Registry, class: &str, method: &str) {
    match resolve_func(reg, class, method) {
        Some(rf) => match function_flags(rf.func) {
            Some(f) => crate::log(&format!(
                "[funcflags] {class}.{method}: {} (is_static resolvido pelo walk={})",
                decode_function_flags(f),
                rf.is_static
            )),
            None => crate::log(&format!("[funcflags] {class}.{method}: flags@0xA8 ilegível")),
        },
        None => crate::log(&format!("[funcflags] {class}.{method} não resolveu (nem na cadeia de parents)")),
    }
}

/// RED4ext.SDK #284/#280 (2026-08-11): `DynArray<T>::Erase`/`RemoveAt`/`Assign` — genérico,
/// zero endereço nativo (confirmado via `Containers/DynArray.hpp`: `Erase` é lógica C++
/// auto-contida, `ShiftEntries` = memmove puro, sem chamada nativa nenhuma). Opera no layout
/// `{entries(ptr)@0x00, capacity(u32)@0x08, size(u32)@0x0C}` — o MESMO já batalha-testado por
/// `mkarr`/`get_records_of_class`/`class_own_functions`/`Entity.GetComponents` neste projeto.
/// `header_ptr` aponta pro início desse struct de 16 bytes; `stride` = bytes por elemento;
/// `index` = posição a remover (0-based). Devolve `false` se `index>=size` (mesmo contrato de
/// `RemoveAt`, que valida o índice antes de chamar `Erase`).
pub unsafe fn dynarray_remove_at(header_ptr: *mut u8, stride: usize, index: usize) -> bool {
    if header_ptr.is_null() || !crate::gum::is_readable(header_ptr as *const c_void, 0x10) {
        return false;
    }
    let entries = (header_ptr as *const u64).read_unaligned() as *mut u8;
    let cap = (header_ptr.add(8) as *const u32).read_unaligned();
    let size = (header_ptr.add(12) as *const u32).read_unaligned();
    if index as u32 >= size || size > cap || entries.is_null() {
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

/// RED4ext.SDK #280 — `Assign(aSize, aValue)`: substitui TODO o conteúdo (mesmo layout, sem
/// shift — só sobrescreve `[0..count)` com o valor dado e ajusta `size`). Exige `count<=capacity`
/// (mesmo contrato de `Assign`, que faz `Reserve` internamente se precisar crescer — aqui, por
/// simplicidade/segurança, apenas recusa se não couber, já que crescer BUFFER NOSSO seria só
/// realocar um `Vec` — não testado nesta rodada, escopo = provar o mecanismo de substituição).
pub unsafe fn dynarray_assign_fill(header_ptr: *mut u8, stride: usize, count: usize, value: &[u8]) -> bool {
    if header_ptr.is_null() || value.len() != stride || !crate::gum::is_readable(header_ptr as *const c_void, 0x10) {
        return false;
    }
    let entries = (header_ptr as *const u64).read_unaligned() as *mut u8;
    let cap = (header_ptr.add(8) as *const u32).read_unaligned();
    if count as u32 > cap || entries.is_null() {
        return false;
    }
    for i in 0..count {
        core::ptr::copy_nonoverlapping(value.as_ptr(), entries.add(i * stride), stride);
    }
    core::ptr::write_unaligned(header_ptr.add(12) as *mut u32, count as u32);
    true
}

/// Diagnóstico read/write-only (buffer 100% NOSSO, nunca exposto ao motor/engine — zero risco de
/// crash de teardown, categoria diferente do `GetRecordsArray`/#43 que crashou por precisar de
/// trailer de allocator pra um array LOCAL REDSCRIPT). Aloca `[Int32; 5]`, remove o índice 2,
/// confirma shift correto + `size` decrementado; depois testa `Assign` (substitui tudo por `77`,
/// `count=3`).
pub unsafe fn dynarray_removeat_selftest() {
    let mut buf: Vec<i32> = vec![10, 20, 30, 40, 50];
    let entries_ptr = buf.as_mut_ptr() as *mut u8;
    let mut header = [0u8; 16];
    core::ptr::write_unaligned(header.as_mut_ptr() as *mut u64, entries_ptr as u64);
    core::ptr::write_unaligned(header.as_mut_ptr().add(8) as *mut u32, 5u32); // capacity
    core::ptr::write_unaligned(header.as_mut_ptr().add(12) as *mut u32, 5u32); // size

    let ok = dynarray_remove_at(header.as_mut_ptr(), 4, 2); // remove índice 2 (valor 30)

    let new_size = (header.as_ptr().add(12) as *const u32).read_unaligned();
    let vals: Vec<i32> = (0..new_size as usize)
        .map(|i| (entries_ptr.add(i * 4) as *const i32).read_unaligned())
        .collect();

    crate::log(&format!(
        "[dynarrtest] RemoveAt(idx=2) ok={ok} new_size={new_size} vals={vals:?} (esperado: true 4 [10,20,40,50])"
    ));

    let fill_val: i32 = 77;
    let ok2 = dynarray_assign_fill(header.as_mut_ptr(), 4, 3, &fill_val.to_ne_bytes());
    let new_size2 = (header.as_ptr().add(12) as *const u32).read_unaligned();
    let vals2: Vec<i32> = (0..new_size2 as usize)
        .map(|i| (entries_ptr.add(i * 4) as *const i32).read_unaligned())
        .collect();
    crate::log(&format!(
        "[dynarrtest] Assign(count=3,val=77) ok={ok2} new_size={new_size2} vals={vals2:?} (esperado: true 3 [77,77,77])"
    ));
}

#[cfg(test)]
mod cstring_tests {
    use super::read_cstring;

    /// Monta um buffer de 0x20 bytes no layout do RED4ext::CString (SSO inline): os primeiros
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

/// RED4ext.SDK `#55` — núcleo de decisão de nome do `resolve_global_function_robust`, testado
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
        assert!(global_fn_name_matches("GetText", "Codeware.Localization.GetText"));
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
