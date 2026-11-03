//! Interface (widgets ink).

use crate::register;
use crate::rtti;
use std::ffi::c_void;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
pub(crate) const WIDGET_DISPARAR_EVENTO_VM: u64 = 0x1_043a_c60c;

/// `InkWidgetLibrary::SpawnFromLocal(biblioteca, item) -> Handle<inkWidgetLibraryItemInstance>`: a saída vem pelo `x8` (`mov x19, x8` no prólogo,
/// assinatura da tabela de endereços). O prólogo é o `stp x24, x23, [sp, #-0x40]!`.
pub(crate) const SPAWN_FROM_LOCAL_VM: u64 = 0x1_0435_402c;
pub(crate) const SPAWN_FROM_LOCAL_PROLOGO: u32 = 0xa9bc_5ff8;

/// `InkWidgetLibrary::SpawnFromExternal(biblioteca, caminho da externa, item) -> Handle<inkWidgetLibraryItemInstance>`, a saída pelo `x8`. O prólogo é
/// o `sub sp, sp, #0x50`.
pub(crate) const SPAWN_FROM_EXTERNAL_VM: u64 = 0x1_0435_424c;
pub(crate) const SPAWN_FROM_EXTERNAL_PROLOGO: u32 = 0xd101_43ff;

/// `InkWidgetLibrary::AsyncSpawnFromLocal(biblioteca, info, item, flag) -> bool`: acha o item pelo nome e passa o `info` (a função da entrega:
/// tabela de ops em `+0x20`, guarda inline ou em `[info]` pelo `+0x30`) ao `AsyncSpawnFromDefPack` (`0x10431d76c`).
/// Item sem esse nome: devolve 0 sem efeito. O prólogo é o `stp x26, x25, [sp, #-0x50]!`.
pub(crate) const ASYNC_SPAWN_FROM_LOCAL_VM: u64 = 0x1_0435_3a7c;
pub(crate) const ASYNC_SPAWN_FROM_LOCAL_PROLOGO: u32 = 0xa9bb_67fa;

/// `InkWidgetLibrary::AsyncSpawnFromExternal(biblioteca, info, caminho da externa, item, flag) -> bool`: o `FindExternalLibrary` (`0x104353edc`) e a
/// mesma busca do local. O prólogo é o `sub sp, sp, #0x60`.
pub(crate) const ASYNC_SPAWN_FROM_EXTERNAL_VM: u64 = 0x1_0435_3d74;
pub(crate) const ASYNC_SPAWN_FROM_EXTERNAL_PROLOGO: u32 = 0xd101_83ff;

/// Entrega do spawn assíncrono (o `FinishAsyncSpawn` da referência): a chamada da função que o processador de pedidos (`0x104368b58`) põe no
/// `info`, entrada `[0]` da tabela de ops `0x107e81078` (a única referência a ele). `x0` = a guarda da função (`WeakHandle` em `+0x00`, `SharedPtr` do
/// pedido em `+0x10`); `x1` = o `Handle` da instância que o `Instantiate` acabou de criar. Copia a raiz (`+0x38`) e o `gameController` (`+0x48`) da
/// instância pro pedido e marca o pedido pronto. O `AsyncSpawnFromDefPack` chama em `0x10431d7dc` com o pacote carregado e pelo job da carga sem ele.
/// Com o byte `0x109d03e78` em 1 (o `Item:Controller` nativo do 2.5, ligado e desligado por `0x104156600`/`0x104156610`), chama antes o `0x104156130`,
/// que troca o controller pelo nome do pedido; o hook roda antes dele. O prólogo é o `sub sp, sp, #0x50`.
pub(crate) const ENTREGA_DO_SPAWN_ASSINCRONO_VM: u64 = 0x1_0436_8f34;
pub(crate) const ENTREGA_DO_SPAWN_ASSINCRONO_PROLOGO: u32 = 0xd101_43ff;

/// Bytes de liga/desliga, em `__common`, dos pedaços nativos do 2.5 que repetem o ArchiveXL e o Codeware: `Item:Controller` no spawn e na entrega
/// (`0x109d03e78`), anexar a árvore no `Reparent`/`AddChild` e filtrar o `OnInitialize` legado (`0x109d03e64`) e o `OnInputKey` antes do
/// `ProcessCharacterEvent` (`0x109d03e68`). No binário só escrevem neles os pares liga/desliga (`0x104156600`/`0x104156610`, `0x10415413c`/`0x10415414c`,
/// `0x104154b2c`/`0x104154b3c`), slots `+0x20`/`+0x28` de vtables (`0x107bea168`, `0x107bea2d8`, `0x107bea428`) que nenhum código nem fixup cita: sem
/// escritor alcançável, os três ficam no zero do `__common`.
pub(crate) const NATIVO_ITEM_CONTROLLER_VM: u64 = 0x1_09d0_3e78;
pub(crate) const NATIVO_ANEXAR_ARVORE_VM: u64 = 0x1_09d0_3e64;
pub(crate) const NATIVO_TECLA_NO_WIDGET_VM: u64 = 0x1_09d0_3e68;

/// Veredito dev dos 3 bytes nativos, uma vez no menu (antes do 1º phase 5; o menu tem o player dele, então não se separa pelo player) e uma na
/// gameplay real. O endereço passa pelo `rebase` (slide da imagem); fora do Steam, sem tradução, sai `sem-endereço`.
#[cfg(feature = "devtools")]
pub(crate) fn bytes_nativos_tick(com_player: bool) {
    static VISTOS: [std::sync::atomic::AtomicBool; 2] = [const { std::sync::atomic::AtomicBool::new(false) }; 2];
    let fase = if crate::inicio::boot::menu_do_boot_resolvido() && !crate::inicio::boot::PHASE_REACHED_5.load(Ordering::Relaxed) {
        0
    } else if com_player && crate::inicio::boot::gameplay_real() {
        1
    } else {
        return;
    };
    if VISTOS[fase].swap(true, Ordering::Relaxed) {
        return;
    }
    let ler = |vm: u64| -> String {
        let p = crate::rebase(vm) as *const u8;
        // SAFETY: só lê o byte depois de conferir que a página está mapeada.
        unsafe {
            if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 1) {
                return "sem-endereço".into();
            }
            p.read_volatile().to_string()
        }
    };
    crate::veredito_dev!(
        "[nativo-2.5] bytes 0x109d03e78={} 0x109d03e64={} 0x109d03e68={} ({}; slide {:#x})",
        ler(NATIVO_ITEM_CONTROLLER_VM),
        ler(NATIVO_ANEXAR_ARVORE_VM),
        ler(NATIVO_TECLA_NO_WIDGET_VM),
        ["menu", "gameplay"][fase],
        crate::game_base().wrapping_sub(0x1_0000_0000)
    );
}

/// Cache do ponteiro `Red::InkSystem*` já cross-validado nesta sessão de boot (2026-08-14,
/// Codeware `#100`/`#120`) — `ink_system()` grava aqui na 1ª descoberta bem
/// sucedida pra nunca re-varrer a BSS a cada chamada de `GetLayers`/`GetLayer`/etc. 0 = ainda
/// não resolvido nesta sessão.
pub(crate) static INKSYSTEM_CACHED: AtomicU64 = AtomicU64::new(0);

/// Teste estrutural (2026-08-14) de
/// "isto PARECE um `Red::InkSystem*` vivo?", read-only, sem crash possível (só
/// `ganchos::is_readable` + reads).
///
/// Layout confirmado no header vendorizado REAL (`enablers/Codeware/src/Red/InkSystem.hpp`,
/// não chutado): `inputWidget: WeakHandle<inkWidget>@0x2E8` (16B: {instance*,rc_block*}),
/// `keyboardState: KeyboardState@0x2F8` (u16 bitfield, só 6 bits reais + 10 bits SEMPRE
/// reservados/zero, `enablers/Codeware/src/Red/Input.hpp`), `requestsHandler:
/// WeakHandle<ISystemRequestsHandler>@0x370` (16B), `layerManagers:
/// DynArray<SharedPtr<InkLayerManager>>@0x380` (16B: {entries*,cap:u32,size:u32}).
/// `InkSystem::Instance` é `Core::RawPtr` (ponteiro GLOBAL cru, MESMA categoria de
/// `ResourceGameDepot`/`JournalManager` — não vtable/singleton-Get()) — por isso NÃO dá pra
/// usar a técnica de `findcgameengine` (vtable-match): não há vtable conhecida pra comparar.
///
/// **RODADA 2026-08-14 (apertando o filtro — antes ruidoso demais).** A versão anterior deste
/// filtro (só "ponteiro plausível-ou-null" por campo, faixa+alinhamento, SEM dereferenciar +
/// `size<=cap<=64` aceitando até `cap==0`) classificou **987 de 1020 objetos visitados** como
/// candidato num BFS real (`walkinksystem <root> 3 4000`) — ruído demais pra ser um sinal útil,
/// e a validação cruzada independente (`requestsHandler` vs `BwmsGetSystemRequestsHandler()`)
/// testou os 4 candidatos mais fortes dessa rodada e NENHUM bateu (ver `HISTORICO.md`
/// 2026-08-14, "inksystemvalidate"). Reescrito pra exigir MÚLTIPLOS sinais convergindo
/// SIMULTANEAMENTE (o mesmo padrão de rigor que já funcionou nesta base noutros achados —
/// "vários sinais fracos juntos, nenhum sozinho decisivo" é exatamente o que faltava aqui):
///
/// 1. **Ponteiro "mapeado", não só "plausível"** (`is_mapped_ptr_or_null`): além de
///    faixa+alinhamento, agora DEREFERENCIA o valor (`ganchos::is_readable(v, 8)`) — um qword que
///    parece ponteiro mas não aponta pra memória mapeada é descartado na hora, não só aceito
///    como "ponteiro plausível" (o critério antigo, que nunca lia o que o ponteiro apontava).
/// 2. **Invariante de nulidade PAREADA nos 2 `WeakHandle`** (`inputWidget`/`requestsHandler`):
///    por `SharedPtrBase<T>` (`RED4ext.SDK/Memory/SharedPtr.hpp`), TODO construtor real seta
///    `instance`/`refCount` JUNTOS — nunca um null e o outro não (ctor default: os 2 null;
///    copy/move/swap: os 2 sempre movidos/copiados como par). Exigir `(instance==0) ==
///    (refCount==0)` elimina candidatos onde só 1 dos 2 slots vira ponteiro por coincidência.
/// 3. **`RefCnt` plausível quando presente**: se `refCount!=0`, lê os 2 `u32`
///    (`strongRefs`/`weakRefs`, `RED4ext.SDK/Memory/SharedPtr.hpp::RefCnt`) e exige valores
///    pequenos e sãos (`<=10_000` cada) — um refcount real nunca tem bilhões de referências.
/// 4. **`keyboardState` — os 10 bits reservados TÊM que ser 0**: `Red::KeyboardState`
///    (`Red/Input.hpp`) só define 6 bits reais (`shiftLeft/Right`,`controlLeft/Right`,
///    `altLeft/Right`); os outros 10 (`b10`) são padding NUNCA escrito pelo motor — exigir
///    `kb & 0xFC00 == 0` é invariante estrutural rígido, não heurística frouxa.
/// 5. **`layerManagers` NÃO-VAZIO** (`cap>=1`, ERA `cap>=0`): a lista de layer managers
///    (HUD/Popup/Menu/etc.) é povoada no BOOT — um `InkSystem` vivo NUNCA tem `cap==0`. A
///    versão antiga aceitava `cap=0/size=0` como caso trivial-válido — exatamente o ruído
///    estrutural que dominava os 987 falsos-candidatos (a maioria de um heap grande tem VÁRIOS
///    arrays vazios por coincidência, então "cap==size==0" é um sinal quase inútil sozinho).
///    Mantido o teto `<=64` (nº plausível de layer managers nomeados).
/// 6. **1º elemento do array de fato inspecionado** (NOVO — a versão antiga nunca olhava
///    DENTRO do array, só os 2 campos de tamanho): lê `layerManagers.entries[0]` como
///    `SharedPtr<InkLayerManager>` (16B, mesmo layout `SharedPtrBase`) e aplica os MESMOS
///    testes 1+2 (mapeado + nulidade pareada) + exige `instance!=0` — com `size>=1` já
///    garantido pelo sinal 5, o slot 0 tem que conter um layer manager de verdade, não lixo
///    herdado de outro array.
///
/// Resultado esperado: de "987 candidatos" pra "poucas dezenas ou menos" no mesmo BFS. Devolve
/// `(keyboardState, layerManagers.cap, layerManagers.size)` se bater, `None` senão.
pub(crate) unsafe fn inksystem_shape_matches(candidate: u64) -> Option<(u16, u32, u32)> {
    if candidate < 0x10000 || candidate % 8 != 0 {
        return None;
    }
    let is_plausible_ptr_or_null = |v: u64| v == 0 || (v > 0x10000 && v % 8 == 0);
    // Sinal 1: não só "parece ponteiro" — DEREFERENCIA pra confirmar que aponta pra memória
    // mapeada de verdade. Um qword 8-alinhado>0x10000 aleatório raramente sobrevive a isso.
    let is_mapped_ptr_or_null =
        |v: u64| v == 0 || (is_plausible_ptr_or_null(v) && crate::memoria::is_readable(v as *const c_void, 8));

    let f_inputwidget = candidate + 0x2E8;
    let f_kbstate = candidate + 0x2F8;
    let f_reqhandler = candidate + 0x370;
    let f_layermanagers = candidate + 0x380;
    if !crate::memoria::is_readable(f_inputwidget as *const c_void, 16)
        || !crate::memoria::is_readable(f_kbstate as *const c_void, 2)
        || !crate::memoria::is_readable(f_reqhandler as *const c_void, 16)
        || !crate::memoria::is_readable(f_layermanagers as *const c_void, 16)
    {
        return None;
    }

    // Sinais 1+2 — inputWidget: WeakHandle{instance,refCount}, mapeados + nulidade pareada.
    let iw0 = (f_inputwidget as *const u64).read_unaligned(); // instance
    let iw1 = (f_inputwidget as *const u64).add(1).read_unaligned(); // refCount
    if !is_mapped_ptr_or_null(iw0) || !is_mapped_ptr_or_null(iw1) || (iw0 == 0) != (iw1 == 0) {
        return None;
    }

    // Sinal 4 — keyboardState: os 10 bits reservados (b10) TÊM que ser 0 (nunca escritos pelo
    // motor real; `Red::KeyboardState` só define os 6 bits baixos).
    let kb = (f_kbstate as *const u16).read_unaligned();
    if kb & 0xFC00 != 0 {
        return None;
    }

    // Sinais 1+2+3 — requestsHandler: mesma checagem de inputWidget + sanidade do RefCnt
    // apontado (se presente).
    let rh0 = (f_reqhandler as *const u64).read_unaligned(); // instance
    let rh1 = (f_reqhandler as *const u64).add(1).read_unaligned(); // refCount (RefCnt*)
    if !is_mapped_ptr_or_null(rh0) || !is_mapped_ptr_or_null(rh1) || (rh0 == 0) != (rh1 == 0) {
        return None;
    }
    if rh1 != 0 {
        // RefCnt{strongRefs:u32, weakRefs:u32} — contadores reais são sempre pequenos.
        let strong = (rh1 as *const u32).read_unaligned();
        let weak = (rh1 as *const u32).add(1).read_unaligned();
        if strong > 10_000 || weak > 10_000 {
            return None;
        }
    }

    // Sinal 5 — layerManagers: NÃO-VAZIO (cap>=1, era >=0) + size dentro do cap + invariante
    // cap==0<=>entries==0 (defesa em profundidade, redundante com cap>=1 mas barata de manter).
    let lm_entries = (f_layermanagers as *const u64).read_unaligned();
    let lm_cap = (f_layermanagers.wrapping_add(8) as *const u32).read_unaligned();
    let lm_size = (f_layermanagers.wrapping_add(12) as *const u32).read_unaligned();
    if !is_mapped_ptr_or_null(lm_entries) {
        return None;
    }
    if lm_cap < 1 || lm_cap > 64 || lm_size < 1 || lm_size > lm_cap {
        return None;
    }
    if (lm_cap == 0) != (lm_entries == 0) {
        return None;
    }

    // Sinal 6 — 1º elemento do array de fato inspecionado (a versão antiga nunca olhava DENTRO
    // do array): SharedPtr<InkLayerManager>{instance,refCount}, mesmos testes de mapeamento +
    // nulidade pareada + `instance!=0` (com size>=1 já garantido, o slot 0 tem que ser um layer
    // manager real, não lixo herdado de um array de outro tipo).
    if !crate::memoria::is_readable(lm_entries as *const c_void, 16) {
        return None;
    }
    let e0_instance = (lm_entries as *const u64).read_unaligned();
    let e0_refcount = (lm_entries as *const u64).add(1).read_unaligned();
    if e0_instance == 0
        || !is_mapped_ptr_or_null(e0_instance)
        || !is_mapped_ptr_or_null(e0_refcount)
        || (e0_instance == 0) != (e0_refcount == 0)
    {
        return None;
    }

    Some((kb, lm_cap, lm_size))
}

/// `ink_system(reg)` (2026-08-14, Codeware `#100`/`#120`, continuação da sessão que
/// CONFIRMOU `Red::InkSystem::Get()` via `findinksystembss`+`inksystemvalidate`) — versão
/// CHAMÁVEL/CACHEADA do mesmo mecanismo de descoberta decisivo (varredura de forma na BSS +
/// cross-validação contra `BwmsGetSystemRequestsHandler()` no MESMO instante), pra qualquer
/// native que precise do ponteiro `Red::InkSystem*` sem repetir a varredura manual toda vez
/// (`GetLayers`/`GetLayer`/`GetWorldWidgets` chamam isto internamente). Reusa
/// `inksystem_shape_matches` (filtro estrutural já endurecido) + o MESMO padrão de
/// cross-validação já provado (`candidato+0x370` == `BwmsGetSystemRequestsHandler()`, 2
/// mecanismos independentes convergindo).
///
/// Cache (`INKSYSTEM_CACHED`): uma vez confirmado nesta sessão de boot, reusa o ponteiro sem
/// re-varrer — só re-valida a FORMA (barato, sem I/O de rede/RTTI) antes de reusar, pra pegar o
/// caso raro de heap realocado embaixo do candidato antigo.
pub(crate) unsafe fn ink_system(reg: &rtti::Registry) -> Option<u64> {
    // 1) cache já validado nesta sessão — revalida só a FORMA (barato) antes de reusar.
    let cached = INKSYSTEM_CACHED.load(Ordering::SeqCst);
    if cached != 0 && inksystem_shape_matches(cached).is_some() {
        return Some(cached);
    }

    // 2) resolve o 2º mecanismo (despacho RTTI real) UMA VEZ — reusado pra cada candidato.
    let f = register::get_function(reg, "BwmsGetSystemRequestsHandler");
    if !rtti::sane(f) {
        crate::log_dev!("[get_inksystem_singleton] BwmsGetSystemRequestsHandler não resolveu (declaração ausente do bundle?) — abortando");
        return None;
    }
    let rf = rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
    let res = rtti::call_func(&rf, std::ptr::null_mut(), &[])?;
    let handler_instance = u64::from_le_bytes(res[0..8].try_into().unwrap());
    if handler_instance == 0 {
        crate::log_dev!("[get_inksystem_singleton] BwmsGetSystemRequestsHandler()->instance==0 — abortando (sem handler vivo ainda)");
        return None;
    }

    // 2b) o global que o próprio motor lê. `InkSystem::Get()` = `0x104274c18` (12 B:
    // `adrp x8; ldr x0, [x8, #0x830]; ret`, 562 chamadores), e o método `0x104272f10` usa o retorno
    // como `this` lendo `layerManagers@0x380/0x38c` — achado offline em 2026-09-11. O endereço cai
    // dentro da faixa que a varredura abaixo já cobria (`0x108e70ca0` + 4 MB); a varredura fica de
    // reserva. Mesma validação de forma e o mesmo cruzamento com o handler do passo 2.
    const INKSYSTEM_INSTANCE_VM: u64 = 0x1_08ec_e4d0;
    let global = crate::rebase(INKSYSTEM_INSTANCE_VM) as *const u64;
    if crate::memoria::is_readable(global as *const c_void, 8) {
        let candidate = global.read_unaligned();
        let forma = inksystem_shape_matches(candidate).is_some();
        let f_reqhandler = candidate.wrapping_add(0x370);
        let cruzado = forma
            && crate::memoria::is_readable(f_reqhandler as *const c_void, 16)
            && (f_reqhandler as *const u64).read_unaligned() == handler_instance;
        crate::log_dev!(
            "[get_inksystem_singleton] global 0x108ece4d0 -> {candidate:#x} forma={forma} cruzado={cruzado}"
        );
        if cruzado {
            INKSYSTEM_CACHED.store(candidate, Ordering::SeqCst);
            return Some(candidate);
        }
    }

    // 3) varre os offsets do cluster BSS já confirmados nesta investigação (default +
    // o offset alternativo que achou o candidato decisivo em 2026-08-14) — cross-valida CADA
    // candidato de forma contra o handler já resolvido no passo 2, no MESMO instante.
    const SCAN_BASES: [u64; 2] = [0x1_099d_8e00, 0x108e70ca0];
    const CHUNK: usize = 65536;
    const SIZE_MB: u64 = 4;
    const MAX_SHAPE_CHECKS: u32 = 20000;
    let mut buf = vec![0u8; CHUNK];
    for &start_static in &SCAN_BASES {
        let scan_start = crate::rebase(start_static) as usize;
        let scan_end = scan_start + (SIZE_MB * 1024 * 1024) as usize;
        let mut p = scan_start;
        let mut checks = 0u32;
        'scan: while p < scan_end {
            let this_len = CHUNK.min(scan_end - p);
            if crate::memoria::read_chunk(p, &mut buf[..this_len]) {
                let mut i = 0usize;
                while i + 8 <= this_len {
                    let candidate = u64::from_le_bytes(buf[i..i + 8].try_into().unwrap());
                    if candidate > 0x10000 && candidate % 8 == 0 {
                        checks += 1;
                        if checks > MAX_SHAPE_CHECKS {
                            break 'scan;
                        }
                        if inksystem_shape_matches(candidate).is_some() {
                            let f_reqhandler = candidate + 0x370;
                            if crate::memoria::is_readable(f_reqhandler as *const c_void, 16) {
                                let cand_instance = (f_reqhandler as *const u64).read_unaligned();
                                if cand_instance != 0 && cand_instance == handler_instance {
                                    INKSYSTEM_CACHED.store(candidate, Ordering::SeqCst);
                                    crate::log_dev!(
                                        "[get_inksystem_singleton] CONFIRMADO {candidate:#x} (cross-validado contra BwmsGetSystemRequestsHandler(), cacheado)"
                                    );
                                    return Some(candidate);
                                }
                            }
                        }
                    }
                    i += 8;
                }
            }
            p += this_len;
        }
    }
    crate::log_dev!("[get_inksystem_singleton] nenhum candidato bateu a cross-validação nesta varredura — None");
    None
}

