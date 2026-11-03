//! Interface: widgets e camadas ink, texto editável, clipboard, eventos de widget e widgets no mundo. Teclado e eixo estão em `entrada`.

#![allow(dead_code)]

pub(crate) mod ink_mundo;
pub(crate) mod spawn_de_widget;
pub(crate) mod tecla_no_widget;
/// Teste ao vivo das telas de guarda-roupa e de loja de mods (comando `telasmod`): só no dev (fora do build público e do export do GitHub).
#[cfg(feature = "devtools")]
pub(crate) mod teste_telas_de_mod;

use crate::jogo::listas::MOVER_HANDLES_VM;
use crate::jogo::objetos::handle_forte_de;
use crate::ganchos::original_publicado;
use crate::ganchos::instalar_conferindo_prologo;
use crate::marcador_individual;
use crate::tipos_do_motor::*;
use crate::diagnostico::{C175_ANEXADAS, C175_ATIVO, C175_LEGADOS_PULADOS};
// Só as sondas de prova do brush usam.
#[cfg(feature = "devtools")]
use crate::diagnostico::{probe_resource_helper_type11_on, wrapped_array_elem_offsets};
// Só o handler de prova usa, e o `corpo_de_prova!` público não o cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::tramp_make_test_inkcharacterevent;
use crate::entrada::*;








#[cfg(feature = "devtools")]
use bwms_primitives::enderecos::LINK_BASE;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b001;
use crate::aparencia::*;



/// `inkWidgetTarget::Matches`: um alvo `(libraryPath, itemName, controllerType)` casa quando cada campo setado bate — biblioteca e item
/// iguais (`None` = curinga, pela regra única do `bwms-primitives::targets`) e, com `controllerType`, `controller_casa(tipo)`. O handler despacha quando algum
/// dos alvos casa. O alvo chega de `inkwidgettarget_alvos`, que converte o 0 da referência em `None`.
pub(crate) fn ink_widget_alvo_casa(alvo: (Option<u64>, Option<u64>, Option<u64>), ev_lib: u64, ev_item: u64, controller_casa: impl Fn(u64) -> bool) -> bool {
    let (lib, item, controller) = alvo;
    bwms_primitives::targets::inkwidget_bate(lib, item, ev_lib, ev_item) && controller.is_none_or(controller_casa)
}

/// `IsInstanceOf(instância.gameController, tipo)`: o Handle `gameController` do `inkWidgetLibraryItemInstance` (`+0x48`) não é nulo e a classe dele herda
/// de `tipo`, pelo nome nativo ou de script. Resolve o nome a cada evento.
pub(crate) unsafe fn widget_controller_herda(instancia: usize, tipo: u64) -> bool {
    if instancia == 0 || !crate::memoria::is_readable((instancia + 0x48) as *const c_void, 8) {
        return false;
    }
    let ctrl = ((instancia + 0x48) as *const u64).read_unaligned() as *mut c_void;
    rtti::sane(ctrl) && class_is_kind_of(rtti::class_of(ctrl), rtti::Registry::obtain().map_or(std::ptr::null_mut(), |reg| reg.script_class(tipo)))
}

/// Forja `inkWidgetTarget extends CallbackSystemTarget` + registra `Controller`/`Library`. Nome da
/// classe MINÚSCULO (`inkWidgetTarget`, não `InkWidgetTarget`) — é a classe que os
/// mods declaram (`public native class inkWidgetTarget`).
pub unsafe fn register_inkwidgettarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_inkwidgettarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "inkWidgetTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_inkwidgettarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_inkwidgettarget: classe=OK mas sem protótipo — Controller/Library NÃO registrados".into(),
    };
    let m1 = register_method(reg, "inkWidgetTarget", proto, "Controller", "Controller", tramp_iwt_controller, true);
    let m2 = register_method(reg, "inkWidgetTarget", proto, "Library", "Library", tramp_iwt_library, true);
    format!(
        "[reg] inkWidgetTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) Controller={m1} Library={m2} \
— is_static+construct+handle NUMA CLASSE FORJADA: PROVADO 2026-08-05"
    )
}

pub unsafe fn register_inkwidgetspawnevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_inkwidgetspawnevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "inkWidgetSpawnEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_inkwidgetspawnevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_inkwidgetspawnevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "inkWidgetSpawnEvent", proto, "GetLibraryPath", "GetLibraryPath", tramp_iwse_get_library_path, false);
    let m2 = register_method(reg, "inkWidgetSpawnEvent", proto, "GetItemName", "GetItemName", tramp_iwse_get_item_name, false);
    let m3 = register_method(reg, "inkWidgetSpawnEvent", proto, "GetItemInstance", "GetItemInstance", tramp_iwse_get_item_instance, false);
    format!("[reg] inkWidgetSpawnEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetLibraryPath={m1} GetItemName={m2} GetItemInstance={m3}")
}


/// O `inkWidgetSpawnEvent` de um spawn, pro `fire_event_args("InkWidget/Spawn", ...)` do
/// `spawn_de_widget::disparar_spawn`. O objeto sai do `rtti::new_object` e os campos entram em [`INKWIDGETSPAWN_STATES`], que os 3 getters leem. Quem
/// chama confere antes `spawn_de_widget::evento_ligado` (`WIDGET_SPAWN_ARMADO` e `WIDGET_SPAWN_EVENT_ENABLED`, abaixo), o `s_widgetSpawnEventEnabled`
/// da referência. `item_instance_ptr` é o `inkWidgetLibraryItemInstance*` do 1º campo da saída do spawn: aqui só é guardado; o `GetItemInstance` o entrega
/// ao script dentro de um `BwmsSerializable` (ver a tabela).
pub(crate) unsafe fn make_inkwidgetspawnevent_arg(
    library_path_hash: u64,
    item_name_hash: u64,
    item_instance_ptr: usize,
) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "inkWidgetSpawnEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = INKWIDGETSPAWN_STATES.lock() {
        states.push((obj as usize, library_path_hash, item_name_hash, item_instance_ptr));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// Há ouvinte de `InkWidget/Spawn`. É o atômico da linha do evento em `GRUPOS_DE_EVENTO`, que todo registro e remoção de ouvinte, de
/// qualquer evento, regrava pela tabela (o `OnActivateEvent`/`OnDeactivateEvent` do `InkSpawningController` da referência).
pub(crate) static WIDGET_SPAWN_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A chave do `BwmsToggleWidgetSpawnEvent`, que os hooks do spawn (`spawn_de_widget::evento_ligado`) leem junto com
/// [`WIDGET_SPAWN_ARMADO`]. Atômico próprio porque o da tabela é regravado a cada registro; nasce ligada, então o registro sozinho liga o despacho, e
/// desligada segura o despacho mesmo com ouvinte.
pub(crate) static WIDGET_SPAWN_EVENT_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub(crate) unsafe extern "C" fn tramp_toggle_widget_spawn_event(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let state = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    WIDGET_SPAWN_EVENT_ENABLED.store(state, Ordering::Relaxed);
    crate::log_dev!("[iwse] BwmsToggleWidgetSpawnEvent({state}): chave={state}, ouvinte={}", WIDGET_SPAWN_ARMADO.load(Ordering::Relaxed));
    write_bool_ret(out, true);
}

pub(crate) unsafe fn register_toggle_widget_spawn_event(reg: &Registry, proto: *mut c_void) -> bool {
    let params = compose_params_from_types(reg, &["Bool"]);
    if !params_compostos(params) {
        crate::log_dev!("[reg] register_toggle_widget_spawn_event: compose_params_from_types falhou (Bool)");
        return false;
    }
    register_global_composed(reg, proto, params, "BwmsToggleWidgetSpawnEvent", "BwmsToggleWidgetSpawnEvent", tramp_toggle_widget_spawn_event as NativeHandler)
}

/// Sonda um campo cujo tipo ENVOLVE `inkWidgetBrush` (`array<inkWidgetBrush>` / `ref<>` / `wref<>`).
///
/// Existe porque a varredura de widgets vivos (`res11walk`) nunca acha o campo na forma DIRETA —
/// 10.759 widgets visitados em 2026-08-18, zero match direto — e o ramo wrapped só LOGAVA.
/// Alcançar um `inkWidgetBrush` VIVO importa porque `textureAtlas` é a única propriedade
/// `type_kind == 11` (ResourceReference) confirmada nesta build, e um brush vivo é o único
/// candidato conhecido a ter um token GENUINAMENTE CARREGADO — o caminho POSITIVO que falta
/// tanto pro `#18` quanto pro `#199` (os dois batem nessa mesma parede).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn probe_wrapped_brush(field_ptr: *mut c_void, wrapper_ty: *mut c_void, inner_ty: *mut c_void, prop: &str) {
    const MAX_ELEMS: usize = 8; // amostra, não varredura — é memória viva do jogo
    let kind = rtti::type_kind(wrapper_ty).unwrap_or(255);
    match kind {
        3 => {
            // DynArray: `{entries@0x00, cap@0x08 (u32), size@0x0C (u32)}` — o MESMO header que
            // `dynarray_remove_at`/`mkarr` já exercitam em produção.
            if !crate::memoria::is_readable(field_ptr as *const c_void, 0x10) {
                return;
            }
            let entries = rd_u64(field_ptr as *const c_void, 0x00) as *mut u8;
            let cap = rd_u32(field_ptr as *const c_void, 0x08);
            let size = rd_u32(field_ptr as *const c_void, 0x0C);
            let stride = rtti::type_size(inner_ty) as usize;
            let offs = wrapped_array_elem_offsets(cap, size, stride, MAX_ELEMS);
            crate::log_dev!(
                "[res11walk] wrapped array '{prop}': size={size} cap={cap} stride={stride} -> {} elemento(s)",
                offs.len()
            );
            if entries.is_null() {
                return;
            }
            for off in offs {
                let elem = entries.add(off) as *mut c_void;
                if crate::memoria::is_readable(elem as *const c_void, stride.min(0x40)) {
                    probe_resource_helper_type11_on(elem, "inkWidgetBrush", "textureAtlas", "res11walk-wrapped");
                }
            }
        }
        9 | 10 => {
            // Handle/WeakHandle: `{instance@0x00, refcount@0x08}` — só a instância interessa.
            if !crate::memoria::is_readable(field_ptr as *const c_void, 0x08) {
                return;
            }
            let inst = rd_u64(field_ptr as *const c_void, 0x00) as *mut c_void;
            crate::log_dev!(
                "[res11walk] wrapped {} '{prop}': instance={inst:p}",
                rtti::decode_ertti_type(kind)
            );
            if rtti::sane(inst) {
                probe_resource_helper_type11_on(inst, "inkWidgetBrush", "textureAtlas", "res11walk-wrapped");
            }
        }
        _ => {
            // StaticArray/NativeArray/FixedArray têm layout DIFERENTE do DynArray — nunca deref às
            // cegas; loga pra decidir com dado, se algum dia aparecer.
            crate::log_dev!(
                "[res11walk] wrapped '{prop}': wrapper {} ({kind}) sem leitura implementada — nao dereferenciado",
                rtti::decode_ertti_type(kind)
            );
        }
    }
}

/// 2026-08-18 (continuação da continuação da continuação): `res11walk [max_layers]
/// [max_nodes]` — os 2 pivôs de TweakDB (`res11rec` contra `vehicleAudioCurvesParam`) deram 0
/// records reais (a classe existe na RTTI mas nunca é instanciada como record top-level de
/// `recordsByID` — é um struct EMBUTIDO por valor em algum outro record/componente, não achável
/// por esse mecanismo). Pivô final: andar a árvore REAL de widgets a partir de cada
/// `InkLayer->libraryItem->gameController` (offsets já confirmados pelo `#93`/`inkLayerWrapper`,
/// `ink_layers_array`/`read_handle_field` acima), usando só natives VANILLA já confirmadas no
/// `redscript-src` real (`abstractWidgets.script:8/232/240` — `GetController`/`GetNumChildren`/
/// `GetWidgetByIndex` em `inkWidget`; `baseControllers/widgetController.script:4/6` —
/// `GetRootWidget`/`GetRootCompoundWidget` em `inkLogicController`) via `rtti::call_func` direto
/// (sem precisar de nenhum `.reds` novo — os métodos já existem no motor, só nunca chamados
/// deste lado por Rust puro). BFS bounded (`max_nodes`), cada ponteiro lido é validado
/// (`rtti::sane`/`ganchos::is_readable`) antes de avançar — pior caso de um `wref<T>`/ABI mal-lido é
/// um `false`/ponteiro-implausível descartado pela validação, nunca um deref cego.
///
/// Pra cada widget visitado, anda a cadeia de parents (`cls+0x10`) verificando se alguma
/// propriedade PRÓPRIA de alguma classe da cadeia tem TIPO `inkWidgetBrush` (não assume nome de
/// campo — `inkImage`/`inkText` não têm presença nenhuma em `redscript-src`, então o nome real do
/// campo "brush" nunca foi confirmado; comparar pelo NOME DO TIPO da propriedade, já provado
/// funcionar em `scan_type_kind_across_hierarchy`, é robusto a isso). Ao achar, reusa
/// `probe_resource_helper_type11_on` sobre o `field_ptr` do campo achado — se este widget já
/// estiver renderizando de verdade (HUD/menu abertos no momento do disparo), o token pode estar
/// genuinamente `loaded=true`, fechando o item pela 1ª vez.
///
/// **ACHADO REAL (2026-08-18, 1ª tentativa) — CROU.** `res11walk 30 800` achou `layer[4]`
/// (`gameuiPopupsManager`) com `GetRootWidget()` resolvendo um ponteiro `sane()` plausível
/// (`0x7e99c136c0`), mas o boot morreu logo depois (`EXC_BAD_ACCESS`/`KERN_INVALID_ADDRESS at
/// 0x100000002` — o padrão clássico de 2 valores de 32 bits, ex. `{n=2, algo=1}`, sendo lido como
/// 1 ponteiro de 64 bits) com o topo da pilha DENTRO desta própria função (não dentro do
/// `crate::jogo::scripts::EXECUTOR_VM` do motor — `atos` confirma o PC caindo no símbolo `probe_widget_tree_for_brush`,
/// não em código do jogo). Causa-raiz NÃO isolada com certeza (sem DWARF de linha no release
/// build) — suspeita mais forte: o par `GetNumChildren`/`GetWidgetByIndex` chamado via
/// `rtti::call_func` cru é a 1ª vez neste projeto que se chama um método VANILLA arbitrário que
/// devolve `wref<T>` a partir de Rust puro (todo uso anterior de `call_func` era ou escalar/Vec4,
/// ou contra native FORJADA nossa com ABI conhecida) — a suposição de que `res[0..8]` sempre
/// contém o ponteiro `instance` de um `WeakHandle` pode estar errada pra esse par específico.
/// `sane()` (só range-check) deixa passar um valor "plausível" que `class_of()` deveria filtrar
/// via `is_readable`, mas o crash aconteceu ANTES de qualquer leitura chegar nesse gate — condiz
/// com a hipótese de que o próprio valor lido de `res[0..8]` já é usado de um jeito que não passa
/// pelas guardas (`sane`/`is_readable`) antes de ser tratado como ponteiro em algum ponto do
/// código acima. **NÃO retestar sem reforçar TODAS as leituras com `ganchos::is_readable` explícito
/// (não só `sane`) e sem isolar o `GetNumChildren`/`GetWidgetByIndex` num teste MENOR e MAIS
/// CAUTELOSO primeiro (1 widget conhecido, log de cada byte do buffer de retorno antes de
/// interpretar).** Ambiente confirmado limpo pós-crash (101/64 baseline intacto, saves 44/44,
/// processo morreu sozinho sem deixar zumbi). Item `#199` mantido REAL_GAP.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn probe_widget_tree_for_brush(reg: &Registry, max_layers: usize, max_nodes: usize) {
    let Some((entries, size)) = ink_layers_array(reg) else {
        crate::log_dev!("[res11walk] InkSystem/layers indisponível — GetLayerManager não resolveu");
        return;
    };
    let brush_ty_hash = cname("inkWidgetBrush");
    let mut total_visited = 0usize;
    let mut brush_hits = 0usize;
    for li in 0..(size as usize).min(max_layers) {
        if total_visited >= max_nodes {
            break;
        }
        let slot = (entries as *mut u8).add(li * 16) as *const c_void;
        if !crate::memoria::is_readable(slot, 8) {
            continue;
        }
        let layer = rd_u64(slot, 0x00) as *mut c_void;
        if !rtti::sane(layer) {
            continue;
        }
        let library_item = read_handle_field(layer, 0x50);
        if library_item.is_null() || !rtti::sane(library_item) {
            continue;
        }
        let controller = read_handle_field(library_item, 0x48);
        if controller.is_null() || !rtti::sane(controller) {
            continue;
        }
        let ctrl_cls = rtti::class_of(controller);
        if !rtti::sane(ctrl_cls) {
            continue;
        }
        let ctrl_name = crate::cname::name_of(rtti::type_name_getname(ctrl_cls)).unwrap_or_default();
        let root = rtti::resolve_in_class(ctrl_cls, "GetRootWidget")
            .filter(|rf| rtti::sane(rf.func) && crate::memoria::is_readable(rf.func as *const c_void, 0x40))
            .and_then(|rf| {
                rtti::call_func(&rf, controller, &[]).and_then(|r| {
                    let p = u64::from_le_bytes(r[0..8].try_into().unwrap()) as *mut c_void;
                    if rtti::sane(p) && crate::memoria::is_readable(p as *const c_void, 8) { Some(p) } else { None }
                })
            });
        crate::log_dev!(
            "[res11walk] layer[{li}] controller={controller:p} ({ctrl_name}) GetRootWidget={:?}",
            root
        );
        let Some(root) = root else { continue };

        let mut queue: std::collections::VecDeque<*mut c_void> = std::collections::VecDeque::new();
        queue.push_back(root);
        let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
        while let Some(w) = queue.pop_front() {
            if total_visited >= max_nodes {
                crate::log_dev!("[res11walk] limite de {max_nodes} nós atingido — parando");
                break;
            }
            if !seen.insert(w as usize) || !rtti::sane(w) {
                continue;
            }
            total_visited += 1;
            let wcls = rtti::class_of(w);
            if !rtti::sane(wcls) {
                continue;
            }
            // cadeia de parents, checando propriedades PRÓPRIAS de cada nível por tipo inkWidgetBrush
            let mut cls_walk = wcls;
            let mut depth = 0;
            while !cls_walk.is_null() && depth < 12 && crate::memoria::is_readable(cls_walk as *const c_void, 0x18) {
                for p in rtti::class_own_properties(cls_walk).iter() {
                    if !crate::memoria::is_readable(*p as *const c_void, 0x10) {
                        continue;
                    }
                    let ty = rd_u64(*p as *const c_void, 0x00) as *mut c_void;
                    if ty.is_null() || !rtti::sane(ty) || !crate::memoria::is_readable(ty as *const c_void, 8) {
                        continue;
                    }
                    // ALVO REAL (2026-08-21): o que o `#18`/`#199` precisam NÃO é um
                    // `inkWidgetBrush` — é **qualquer propriedade `type_kind == 11`
                    // (ResourceReference) num objeto VIVO**, porque só ali pode existir um token
                    // genuinamente carregado. Caçar a CLASSE brush era um proxy indireto, e a
                    // varredura de 2026-08-21 provou que é proxy RUIM: 4826 widgets vivos, ZERO
                    // campo `inkWidgetBrush` em qualquer forma (nem direto nem wrapped) — o
                    // `inkWidgetBrush.textureAtlas` achado pelo `scan11` só existe em instância
                    // SINTÉTICA. Esta checagem vai direto ao critério que importa.
                    if rtti::type_kind(ty) == Some(rtti::ertti::RESOURCE_REFERENCE) {
                        let nh = rd_u64(*p as *const c_void, 0x08);
                        let pn = crate::cname::name_of(nh).unwrap_or_default();
                        let cn = crate::cname::name_of(rtti::type_name_getname(cls_walk)).unwrap_or_default();
                        crate::log_dev!(
                            "[res11walk] >>> TYPE-11 VIVO: {cn}.{pn} em widget {w:p} <<<"
                        );
                        if !pn.is_empty() && !cn.is_empty() {
                            probe_resource_helper_type11_on(w, &cn, &pn, "res11walk-type11");
                        }
                    }
                    let direct_match = rtti::type_name_getname(ty) == brush_ty_hash;
                    // 10.759 widgets visitados sem NENHUM match direto (2026-08-18, 3ª tentativa)
                    // sugere que o campo real pode estar WRAPPED (`array<inkWidgetBrush>`, comum
                    // pra estados 9-slice de imagem) — `get_inner_type` (já provado pelo #197)
                    // resolve o tipo interno de Array/Handle/WeakHandle sem RE nova. Só LOGA por
                    // enquanto (não deref o conteúdo do wrapper — stride/ABI de array-de-struct
                    // nunca confirmados neste projeto, deixado pra uma rodada de fechamento se
                    // isso realmente achar algo).
                    let wrapped_match = !direct_match
                        && rtti::get_inner_type(ty)
                            .map(|inner| rtti::type_name_getname(inner) == brush_ty_hash)
                            .unwrap_or(false);
                    if !direct_match && !wrapped_match {
                        continue;
                    }
                    let name_hash = rd_u64(*p as *const c_void, 0x08);
                    let prop_name = crate::cname::name_of(name_hash).unwrap_or_default();
                    let wcls_name = crate::cname::name_of(rtti::type_name_getname(wcls)).unwrap_or_default();
                    crate::log_dev!(
                        "[res11walk] >>> widget {w:p} (classe {wcls_name}) TEM campo '{prop_name}': inkWidgetBrush ({}) <<<",
                        if direct_match { "DIRETO" } else { "WRAPPED array/handle" }
                    );
                    brush_hits += 1;
                    if direct_match {
                        if let Some((voff, _, in_holder)) = rtti::resolve_prop_in_class(wcls, &prop_name) {
                            let brush_ptr = rtti::field_ptr(w, voff, in_holder);
                            if !brush_ptr.is_null() {
                                probe_resource_helper_type11_on(brush_ptr, "inkWidgetBrush", "textureAtlas", "res11walk");
                            }
                        }
                    }
                    // wrapped_match: DEREFERENCIADO de verdade desde 2026-08-21 — o bloqueio
                    // registrado aqui ("stride/ABI nunca confirmados") caiu: o stride sai de
                    // `rtti::type_size(inner)` (`IType::GetSize`, o mesmo getter que o
                    // `new_object` já usa), e o header do `DynArray` (`{entries, cap, size}`) é o
                    // MESMO que `dynarray_remove_at`/`mkarr` já exercitam em produção. Sem isso o
                    // `res11walk` nunca alcançava um `inkWidgetBrush` VIVO — 10.759 widgets
                    // visitados, zero match DIRETO, e o campo real é justamente o wrapped.
                    if wrapped_match {
                        if let Some((voff, _, in_holder)) = rtti::resolve_prop_in_class(wcls, &prop_name) {
                            let fptr = rtti::field_ptr(w, voff, in_holder);
                            let inner = rtti::get_inner_type(ty);
                            if !fptr.is_null() {
                                if let Some(inner) = inner {
                                    probe_wrapped_brush(fptr, ty, inner, &prop_name);
                                }
                            }
                        }
                    }
                }
                cls_walk = rd_u64(cls_walk as *const c_void, 0x10) as *mut c_void; // parent
                depth += 1;
            }
            // filhos (GetNumChildren/GetWidgetByIndex, vanilla, abstractWidgets.script) — 2026-08-18
            // (2ª tentativa, pós-crash): endurecido com `is_readable` explícito no PRÓPRIO
            // `rf.func` antes de deixar `call_func` tocar nele (`call_func` faz `rd_ptr`/`rd_u32`
            // crus em `fb+0x28`/`fb+0x30` sem guard nenhum — se `resolve_in_class` alguma vez
            // devolver algo não-genuinamente-chamável pra este par nome/classe, o crash acontece
            // ALI, não na leitura do retorno) + log do buffer CRU antes de interpretar como
            // ponteiro (nunca confia em `sane()` sozinho pra decidir empilhar um filho).
            let func_is_callable = |f: *mut c_void| -> bool {
                rtti::sane(f) && crate::memoria::is_readable(f as *const c_void, 0x40)
            };
            let gnc = rtti::resolve_in_class(wcls, "GetNumChildren").filter(|rf| func_is_callable(rf.func));
            if let Some(rf) = gnc {
                if let Some(r) = rtti::call_func(&rf, w, &[]) {
                    let n = i32::from_le_bytes([r[0], r[1], r[2], r[3]]).max(0) as usize;
                    if n > 0 && n < 4096 {
                        let gwi = rtti::resolve_in_class(wcls, "GetWidgetByIndex").filter(|rf| func_is_callable(rf.func));
                        if let Some(rf2) = gwi {
                            for i in 0..n.min(64) {
                                if let Some(r2) = rtti::call_func(&rf2, w, &[rtti::Arg::I32(i as u32)]) {
                                    let child = u64::from_le_bytes(r2[0..8].try_into().unwrap()) as *mut c_void;
                                    // 3 guardas antes de confiar no ponteiro: range plausível,
                                    // memória genuinamente mapeada (syscall real, não só range),
                                    // E diferente do PRÓPRIO widget (evita ciclo trivial).
                                    if rtti::sane(child)
                                        && crate::memoria::is_readable(child as *const c_void, 8)
                                        && child != w
                                    {
                                        queue.push_back(child);
                                    } else if total_visited <= 3 {
                                        crate::log_dev!(
                                            "[res11walk]   GetWidgetByIndex({i}) bytes cru={:02x?} — descartado (guard)",
                                            &r2[..16]
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    crate::log_dev!(
        "[res11walk] terminado: {total_visited} widget(s) visitados, {brush_hits} campo(s) inkWidgetBrush achado(s)"
    );
}


// ===== `inkSystem.GetClipboardText`/`SetClipboardText` — candidato barato
// achado 2026-08-11 (4ª rodada de mineração): o clipboard NÃO precisa de
// nenhuma função nativa do MOTOR ("zero RE de motor necessária"). O resto de
// `inkSystem` (`GetLayers`/`GetLayer`/`GetWorldWidgets`) PRECISA do singleton `Red::InkSystem::Get()`
// (endereço `Red::AddressLib::InkSystem_Instance`, nunca resolvido no Mac) — fora de escopo aqui,
// deliberadamente. Mas o clipboard é 100% independente da engine: este projeto JÁ TEM uma ponte
// NSPasteboard PROVADA EM PRODUÇÃO (o Cmd+C/Cmd+X/Cmd+V do campo de texto do terminal passa por ela —
// `overlay.rs::clipboard_get`/`clipboard_set`, reusadas aqui, zero duplicação).
// Divergência de escopo consciente (mesma categoria já aceita pelo projeto pra `BwmsGetCallbackSystem`
// etc.): exposto como globais `BwmsGetClipboardText`/`BwmsSetClipboardText` em vez de
// `@addMethod(GameInstance) GetInkSystem()`+forjar a classe `inkSystem` inteira (que exigiria
// registrar mais 1 classe nativa nova só pra 2 métodos que não precisam de estado nenhum de
// instância — risco desproporcional ao ganho). `write_cstring_inline_ret` devolve o texto de
// qualquer tamanho (`jogo::cstring::nova`, alocador do motor).
pub(crate) unsafe extern "C" fn tramp_ink_get_clipboard_text(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let text = crate::terminal::overlay::clipboard_get().unwrap_or_default();
    let ok = write_cstring_inline_ret(out, &text);
    crate::log_dev!("[ink-clipboard] GetClipboardText() len={} escrito={ok}", text.len());
}

pub(crate) unsafe extern "C" fn tramp_ink_set_clipboard_text(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let text = strings.first().cloned().flatten().unwrap_or_default();
    crate::terminal::overlay::clipboard_set(&text);
    crate::log_dev!("[ink-clipboard] SetClipboardText(len={}) -> OK", text.len());
    write_bool_ret(out, true);
}

// Parte 100-3 (2026-09-12): clipboard sem o limite de 19 B. O texto sai em pedaços (`write_cstring_chunk_ret`) e o reds junta; o
// teste guarda o texto do usuário antes e devolve no fim (`BwmsClipboardKeep`), sem mexer quando o clipboard não tem texto.
#[cfg(feature = "devtools")]
pub(crate) enum ClipboardGuardado {
    Texto(String),
    Vazio,
}

#[cfg(feature = "devtools")]
pub(crate) static CLIPBOARD_GUARDADO: std::sync::Mutex<Option<ClipboardGuardado>> = std::sync::Mutex::new(None);

/// `BwmsClipboardTextPart(part) -> String`: pedaço `part` do texto atual do clipboard; vazio no fim.
pub(crate) unsafe extern "C" fn tramp_clipboard_text_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let parte = args.first().map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let texto = crate::terminal::overlay::clipboard_get().unwrap_or_default();
    let _ = write_cstring_chunk_ret(out, &texto, parte);
}

/// `BwmsClipboardKeep(action) -> Int32`: 1 guarda o conteúdo atual — os caracteres do texto, 0 se o clipboard está vazio de verdade (nenhum tipo) e -1
/// se há outro conteúdo (imagem etc.), que não dá pra devolver (boot 58: o clipboard sem texto deixou o teste inconclusivo); 2 devolve o guardado
/// (texto de volta ou clipboard esvaziado; 1, ou -1 nada guardado); 3 compara o atual com o guardado (1 igual, 0 diferente, -1 nada guardado).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_clipboard_keep(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let acao = args.first().map(|(v, _)| *v as u32 as i32).unwrap_or(0);
    let r = match CLIPBOARD_GUARDADO.lock() {
        Err(_) => -9,
        Ok(mut g) => match acao {
            1 => match (crate::terminal::overlay::clipboard_get(), crate::terminal::overlay::clipboard_tipos()) {
                (Some(t), _) => {
                    let n = t.chars().count() as i32;
                    *g = Some(ClipboardGuardado::Texto(t));
                    n
                }
                (None, Some(0)) => {
                    *g = Some(ClipboardGuardado::Vazio);
                    0
                }
                _ => {
                    *g = None;
                    -1
                }
            },
            2 => match g.as_ref() {
                Some(ClipboardGuardado::Texto(t)) => {
                    crate::terminal::overlay::clipboard_set(t);
                    1
                }
                Some(ClipboardGuardado::Vazio) => {
                    crate::terminal::overlay::clipboard_limpar();
                    1
                }
                None => -1,
            },
            3 => match g.as_ref() {
                Some(ClipboardGuardado::Texto(t)) => (crate::terminal::overlay::clipboard_get().as_deref() == Some(t.as_str())) as i32,
                Some(ClipboardGuardado::Vazio) => (crate::terminal::overlay::clipboard_tipos() == Some(0)) as i32,
                None => -1,
            },
            _ => 0,
        },
    };
    crate::veredito_dev!("[cw100c] ClipboardKeep ação {acao} -> {r}");
    write_uint_ret(out, r as u32 as u64, 4);
}

pub(crate) unsafe fn register_ink_clipboard_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &[], "BwmsGetClipboardText", "BwmsGetClipboardText", tramp_ink_get_clipboard_text);
    let ok2 = register_argful_by_types(reg, proto, &["String"], "BwmsSetClipboardText", "BwmsSetClipboardText", tramp_ink_set_clipboard_text);
    crate::log_dev!("[reg] register_all: cw-100 inkSystem clipboard GetClipboardText={ok1} SetClipboardText={ok2}");
    let ok3 = register_argful_by_types(reg, proto, &["Int32"], "BwmsClipboardTextPart", "BwmsClipboardTextPart", tramp_clipboard_text_part as NativeHandler);
    let ok4 = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32"], "BwmsClipboardKeep", "BwmsClipboardKeep", corpo_de_prova!(tramp_clipboard_keep)));
    crate::log_dev!("[reg] register_all: cw-100 100-3 ClipboardTextPart={ok3} ClipboardKeep={ok4}");
}

// ===================================================================================
// `InkSystem` — 2026-08-14 (mesma sessão que confirmou
// `Red::InkSystem::Get()` por cross-validação decisiva, `findinksystembss`+`inksystemvalidate`,
// ver `lib.rs::ink_system`, agora uma rotina CHAMÁVEL/CACHEADA em vez de exigir
// varredura manual). Implementa os métodos que dependiam do singleton nunca resolvido antes —
// o bloqueador estrutural mais antigo do item `#100` (aberto desde 2026-06-21)
// está genuinamente resolvido; resta compor a navegação até os layers de
// verdade usando os offsets JÁ CONFIRMADOS:
//
//   InkSystem+0x380 = layerManagers: DynArray<SharedPtr<InkLayerManager>>
//   layerManagers[0].instance = InkLayerManager*
//   InkLayerManager+0x38 = layers: DynArray<Handle<inkLayer>> (`layerManagers[0]->layers`).
//
// Divergência de escopo CONSCIENTE (documentada, não escondida — mesma disciplina já aplicada
// ao clipboard acima, item `#100`): `inkLayer`/`inkWorldLayer` têm ZERO ocorrência em
// `redscript-src` (1764 arquivos, grep confirmado) E zero símbolo em `symbols-demangled.txt`
// (68k símbolos) — diferente de `CMesh` (que TINHA símbolo RTTI real e mesmo assim CROU o bind
// ao ser declarado `native class` sem backing confirmado, 2026-08-10) — aqui não há NENHUM sinal de que o tipo esteja registrado na RTTI deste build.
// Forjar `native class inkLayer extends IScriptable {}` às cegas repetiria exatamente essa
// categoria de risco já provada real neste projeto. As natives abaixo devolvem
// `CName`/`Int32`/`Uint64` (identidade/contagem/índice) em vez de um `ref<inkLayer>` forjado —
// entrega a capacidade PRÁTICA (enumerar layers reais do `InkSystem` vivo + achar um por
// classe) com risco zero de forge de tipo sem backing confirmado.
//
// `GetWorldWidgets()` fica só como GROUNDWORK (`FindWorldLayerIndex`, acha o índice do layer
// `inkWorldLayer` se existir) — a parte que falta (`worldLayer->components` + `GetWindow()`/
// `GetGameController()` por componente) precisa de offsets nunca confirmados, fora do escopo
// desta rodada.
pub(crate) unsafe fn ink_layers_array(reg: &Registry) -> Option<(u64, u32)> {
    let sys = crate::jogo::interface::ink_system(reg)?;
    let lm_field = (sys as *mut u8).add(0x380) as *const c_void;
    if !crate::memoria::is_readable(lm_field, 16) {
        return None;
    }
    let lm_entries = rd_u64(lm_field, 0x00);
    let lm_size = core::ptr::read_unaligned((lm_field as *const u8).add(0x0C) as *const u32);
    if lm_entries == 0 || lm_size == 0 || !crate::memoria::is_readable(lm_entries as *const c_void, 8) {
        return None;
    }
    // layerManagers[0].instance — SharedPtr<InkLayerManager>{instance,refCount}.
    let manager = rd_u64(lm_entries as *const c_void, 0x00);
    if manager == 0 || !crate::memoria::is_readable(manager as *const c_void, 0x38 + 16) {
        return None;
    }
    // InkLayerManager+0x38 = layers: DynArray<Handle<inkLayer>>
    let layers_field = (manager as *mut u8).add(0x38) as *const c_void;
    let layers_entries = rd_u64(layers_field, 0x00);
    let layers_size = core::ptr::read_unaligned((layers_field as *const u8).add(0x0C) as *const u32);
    Some((layers_entries, layers_size))
}

/// Núcleo comum de busca: percorre `entries[0..size)` (array de `Handle<inkLayer>`, 16B/slot)
/// e devolve o índice do 1º cujo `type_name_getname(class_of(instance))` bate `want` — `-1` se
/// não achar/array vazio.
pub(crate) unsafe fn find_layer_index_by_hash(entries: u64, size: u32, want: u64) -> i32 {
    if entries == 0 {
        return -1;
    }
    for i in 0..size {
        let slot = (entries as *mut u8).add(i as usize * 16) as *const c_void;
        if !crate::memoria::is_readable(slot, 8) {
            continue;
        }
        let inst = rd_u64(slot, 0x00) as *mut c_void;
        if !rtti::sane(inst) {
            continue;
        }
        let cls = rtti::class_of(inst);
        if !rtti::sane(cls) {
            continue;
        }
        if rtti::type_name_getname(cls) == want {
            return i as i32;
        }
    }
    -1
}

/// `BwmsInkSystemGetLayerManagerCount() -> Int32` — `layerManagers.size` (leitura pura).
pub(crate) unsafe extern "C" fn tramp_inksystem_get_layermanager_count(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let n = match crate::registry() {
        Some(reg) => match crate::jogo::interface::ink_system(reg) {
            Some(sys) => {
                let lm_field = (sys as *mut u8).add(0x380) as *const c_void;
                if crate::memoria::is_readable(lm_field, 16) {
                    core::ptr::read_unaligned((lm_field as *const u8).add(0x0C) as *const u32)
                } else {
                    0
                }
            }
            None => 0,
        },
        None => 0,
    };
    crate::log_dev!("[inksystem] BwmsInkSystemGetLayerManagerCount() -> {n}");
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsInkSystemGetLayerCount() -> Int32` — `layerManagers[0]->layers.size`.
pub(crate) unsafe extern "C" fn tramp_inksystem_get_layer_count(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let n = match crate::registry() {
        Some(reg) => ink_layers_array(reg).map(|(_, n)| n).unwrap_or(0),
        None => 0,
    };
    crate::log_dev!("[inksystem] BwmsInkSystemGetLayerCount() -> {n}");
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsInkSystemGetLayerClassNameAt(index: Int32) -> CName` — nome REAL da classe nativa do
/// layer no índice (0 se índice fora do alcance/ponteiro inválido).
pub(crate) unsafe extern "C" fn tramp_inksystem_get_layer_classname_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let index = args.first().map(|(v, _)| *v as i64 as i32).unwrap_or(-1);
    let mut name_hash: u64 = 0;
    if let Some(reg) = crate::registry() {
        if let Some((entries, size)) = ink_layers_array(reg) {
            if index >= 0 && (index as u32) < size && entries != 0 {
                let slot = (entries as *mut u8).add(index as usize * 16) as *const c_void;
                if crate::memoria::is_readable(slot, 8) {
                    let inst = rd_u64(slot, 0x00) as *mut c_void;
                    if rtti::sane(inst) {
                        let cls = rtti::class_of(inst);
                        if rtti::sane(cls) {
                            name_hash = rtti::type_name_getname(cls);
                        }
                    }
                }
            }
        }
    }
    crate::log_dev!("[inksystem] BwmsInkSystemGetLayerClassNameAt({index}) -> hash={name_hash:#x}");
    write_uint_ret(out, name_hash, 8);
}

/// `BwmsInkSystemFindLayerIndexByClassName(name: CName) -> Int32` — espelha a semântica real de
/// `GetLayer(layer: CName)` (`layer->GetNativeType()->name == aLayerName`), devolvendo o
/// ÍNDICE em vez de um handle forjado; `-1` se não achar.
pub(crate) unsafe extern "C" fn tramp_inksystem_find_layer_by_classname(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let want = args.first().map(|(v, _)| *v).unwrap_or(0);
    let found = match crate::registry() {
        Some(reg) => match ink_layers_array(reg) {
            Some((entries, size)) => find_layer_index_by_hash(entries, size, want),
            None => -1,
        },
        None => -1,
    };
    crate::log_dev!("[inksystem] BwmsInkSystemFindLayerIndexByClassName({want:#x}) -> {found}");
    write_uint_ret(out, found as i64 as u64, 4);
}

/// `BwmsInkSystemFindWorldLayerIndex() -> Int32` — groundwork de `GetWorldWidgets()`: acha o
/// índice do layer cuja classe se chama `inkWorldLayer` (se existir/estiver registrado nesta
/// RTTI — achado honesto, não garantia; ver nota grande acima). `-1` se não achar.
pub(crate) unsafe extern "C" fn tramp_inksystem_find_world_layer_index(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let want = cname("inkWorldLayer");
    let found = match crate::registry() {
        Some(reg) => match ink_layers_array(reg) {
            Some((entries, size)) => find_layer_index_by_hash(entries, size, want),
            None => -1,
        },
        None => -1,
    };
    crate::log_dev!("[inksystem] BwmsInkSystemFindWorldLayerIndex() -> {found} (procura 'inkWorldLayer')");
    write_uint_ret(out, found as i64 as u64, 4);
}

/// `BwmsInkSystemGetLayer(index: Int32) -> ref<IScriptable>` — 2026-08-14 (continuação da mesma
/// sessão): fecha a divergência das 3 natives acima (que devolviam CONTAGEM/HASH, não um HANDLE)
/// devolvendo um HANDLE REAL pro layer C++ no índice dado, usando o MESMO mecanismo já maduro
/// deste projeto — `rtti::make_handle`/`ADDR_HANDLE_CTOR` (a rotina do PRÓPRIO MOTOR, não uma
/// reimplementação de refcount nossa; ver nota grande em `rtti.rs::make_handle`). O objeto já
/// vive dentro de `layerManagers[0]->layers` (`DynArray<Handle<inkLayer>>`) — logo já tem um bloco de refcount genuíno, criado pelo motor quando o layer foi
/// registrado; a rotina ACHA esse bloco existente (via a tabela de weak-owner já documentada,
/// mesmo idioma de `AddTarget`/`SetRunMode` devolvendo `self`) e só incrementa, nunca duplica —
/// diferente de `write_handle_ret` (usada pra instância NOVA/sem dono, que escreve refcount=0).
///
/// Retorno declarado `ref<IScriptable>` (base vanilla real, sempre segura) em vez do tipo
/// concreto do layer (`inkLayer`/subclasses, sem backing redscript confirmado — ver nota grande
/// acima, mesma categoria de risco que crashou `CMesh` quando forjado às cegas) — a IDENTIDADE
/// REAL do objeto (classe concreta, ex. o hash já lido por `GetLayerClassNameAt`) continua
/// acessível no handle devolvido via `rttidump`/`class_of`/`IsA` do canal de debug; só o TIPO
/// ESTÁTICO declarado no `.reds` é genérico — mesmo padrão já usado pra `EntityComponentEvent.
/// GetComponent()->wref<IComponent>` (item `#9`) e outros handles de objeto C++ real já vivo.
///
/// `index` fora do alcance/ponteiro inválido: `inst` fica null, `make_handle(out, null)` produz
/// um handle nulo válido (a própria rotina trata isso, ver doc de `make_handle`) — nunca crasha.
pub(crate) unsafe extern "C" fn tramp_inksystem_get_layer(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let index = args.first().map(|(v, _)| *v as i64 as i32).unwrap_or(-1);
    let mut inst: *mut c_void = core::ptr::null_mut();
    if let Some(reg) = crate::registry() {
        if let Some((entries, size)) = ink_layers_array(reg) {
            if index >= 0 && (index as u32) < size && entries != 0 {
                let slot = (entries as *mut u8).add(index as usize * 16) as *const c_void;
                if crate::memoria::is_readable(slot, 8) {
                    let candidate = rd_u64(slot, 0x00) as *mut c_void;
                    if rtti::sane(candidate) {
                        inst = candidate;
                    }
                }
            }
        }
    }
    crate::log_dev!(
        "[inksystem] BwmsInkSystemGetLayer({index}) -> inst={inst:p} (handle real via make_handle, {})",
        if inst.is_null() { "NULO" } else { "achou o bloco de refcount existente" }
    );
    rtti::make_handle(out, inst);
}

pub(crate) unsafe fn register_inksystem_layers_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(
        reg, proto, &[], "BwmsInkSystemGetLayerManagerCount", "BwmsInkSystemGetLayerManagerCount", tramp_inksystem_get_layermanager_count,
    );
    let ok2 = register_argful_by_types(reg, proto, &[], "BwmsInkSystemGetLayerCount", "BwmsInkSystemGetLayerCount", tramp_inksystem_get_layer_count);
    let ok3 = register_argful_by_types(
        reg, proto, &["Int32"], "BwmsInkSystemGetLayerClassNameAt", "BwmsInkSystemGetLayerClassNameAt", tramp_inksystem_get_layer_classname_at,
    );
    let ok4 = register_argful_by_types(
        reg, proto, &["CName"], "BwmsInkSystemFindLayerIndexByClassName", "BwmsInkSystemFindLayerIndexByClassName", tramp_inksystem_find_layer_by_classname,
    );
    let ok5 = register_argful_by_types(
        reg, proto, &[], "BwmsInkSystemFindWorldLayerIndex", "BwmsInkSystemFindWorldLayerIndex", tramp_inksystem_find_world_layer_index,
    );
    let ok6 = register_argful_by_types(reg, proto, &["Int32"], "BwmsInkSystemGetLayer", "BwmsInkSystemGetLayer", tramp_inksystem_get_layer);
    crate::log_dev!(
        "[reg] register_all: cw-100-120 inkSystem layers ManagerCount={ok1} LayerCount={ok2} ClassNameAt={ok3} FindByClassName={ok4} FindWorldLayer={ok5} GetLayer={ok6}"
    );
}

// ===== `#100`/`GetWorldWidgets` — diagnóstico OBSERVE-ONLY, 2026-08-16 (rodada dedicada
// de RE ao vivo — a última peça do item, deixada como "groundwork" em 2026-08-14: `FindWorldLayerIndex`
// acha o ÍNDICE do layer `inkWorldLayer` se existir, mas `worldLayer->components` +
// `component->GetWindow()`/`GetGameController()` nunca foram tentados por falta de offset).
//
// Achado novo desta rodada: o layout Windows de
// `Red::ink::WorldLayer : ink::Layer` (mesma categoria de confiança já usada pro `window@+0x40`/
// `controllerProcessor@+0x60` do `#93`): `components: DynArray<SharedPtr<
// IWidgetComponentWrapper>>` @ Windows +0x170, tamanho 0x1B8. E o vtable INTEIRO de
// `IWidgetComponentWrapper` — interface pura (tamanho 0x8,
// só vtable ptr) com `GetWindow()@vtbl+0x38` e `GetGameController()@vtbl+0x98` (slots Windows/MSVC).
//
// Esses 2 layouts descrevem o binário WINDOWS (MSVC) — NUNCA confirmados
// contra o binário Mac (clang/Itanium ABI). Este diagnóstico testa os 2 candidatos mais plausíveis
// pro offset de `components` (+0x170 bruto e +0x178 = +0x08, o MESMO shift Itanium-por-dtor-extra
// já confirmado repetidas vezes neste projeto pra outros structs C++ com vtable — ver
// [[cp77-macos-rtti-vtable-offsets]]) e, se achar um candidato bem-formado, LOGA (nunca chama) o
// 1º qword de cada componente achado (o ponteiro de vtable cru) + o endereço estático (vmaddr)
// correspondente, pronto pra disassembly OFFLINE
// identificar os slots reais de `GetWindow()`/`GetGameController()` SEM nenhum risco de invocar
// função nativa às cegas (a mesma disciplina "capturar vtable, converter runtime->vmaddr via
// slide, disassemblar depois" que já resolveu `TPPRepresentationSlotListener` e
// (parcialmente) `CGameEngine::Get()`).
//
// ZERO escrita, ZERO chamada de função nativa/vtable — só leitura guardada
// (`ganchos::is_readable` antes de CADA deref, nunca cru). Se `inkWorldLayer` não existir nesta RTTI
// (`FindWorldLayerIndex` já devolvia `-1` desde 2026-08-14, nunca confirmado ao vivo), este
// diagnóstico confirma isso com certeza (achado negativo decisivo, não é falha de offset).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn dyn_array_shape(base: u64, off: usize) -> Option<(u64, u32, u32)> {
    let field = (base as *mut u8).add(off) as *const c_void;
    if !crate::memoria::is_readable(field, 16) {
        return None;
    }
    let entries = rd_u64(field, 0x00);
    let cap = core::ptr::read_unaligned((field as *const u8).add(0x08) as *const u32);
    let size = core::ptr::read_unaligned((field as *const u8).add(0x0C) as *const u32);
    Some((entries, cap, size))
}

/// Classifica um `(entries,cap,size)` lido cru em 3 estados, em vez de um booleano só
/// "sane"/"não" (1ª versão desta rodada tratava `cap==0` como sempre-lixo — errado: um
/// `DynArray<T>` vazio de verdade TAMBÉM tem `cap==0`/`size==0`, só que com `entries` apontando
/// pra um sentinel ESTÁTICO válido, não lixo de heap). 3 rótulos:
/// - "GARBAGE": `entries==0` com `cap!=0` (ponteiro nulo não pode ter capacidade — inconsistente),
///   OU `size>cap` (viola o invariante do container), OU `entries!=0` mas ilegível.
/// - "EMPTY_VALID": `cap==0 && size==0` E (`entries==0` OU `entries` aponta pra memória legível) —
///   container genuinamente vazio, plausível.
/// - "POPULATED_VALID": `cap>=1` dentro de um teto plausível, `size<=cap`, `entries` legível.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn dyn_array_classify(entries: u64, cap: u32, size: u32) -> &'static str {
    if size > cap {
        return "GARBAGE(size>cap)";
    }
    if cap == 0 {
        if entries == 0 || crate::memoria::is_readable(entries as *const c_void, 8) {
            return "EMPTY_VALID";
        }
        return "GARBAGE(entries ilegivel c/ cap=0)";
    }
    if entries == 0 {
        return "GARBAGE(entries=0 c/ cap!=0)";
    }
    if cap > 4096 {
        return "GARBAGE(cap absurdo)";
    }
    if !crate::memoria::is_readable(entries as *const c_void, 8) {
        return "GARBAGE(entries ilegivel)";
    }
    "POPULATED_VALID"
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn is_plausible(label: &str) -> bool {
    label == "EMPTY_VALID" || label == "POPULATED_VALID"
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_inksystem_worldwidgets_diag(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let base = crate::game_base() as u64;
    let to_vmaddr = |runtime: u64| -> u64 { if runtime == 0 || runtime < base { 0 } else { bwms_primitives::enderecos::LINK_BASE + (runtime - base) } };

    // Verdito: exige que os 2 shifts CANDIDATOS (raw=Windows puro, shift=+0x08 Itanium) sejam
    // avaliados em 2 CAMPOS INDEPENDENTES da MESMA struct (`components@0x170` e o vizinho
    // `unk158: DynArray<void*>@0x158`, ambos DynArray, distância relativa de 0x18 bytes entre
    // eles preservada em qualquer shift uniforme) — cross-validação: se o MESMO shift (raw OU
    // +0x08) faz os 2 campos parecerem plausíveis SIMULTANEAMENTE, é evidência bem mais forte
    // que checar 1 campo isolado (mesmo espírito de `inksystemvalidate`: 2 sinais convergindo).
    let mut shift_verdict: [Option<bool>; 2] = [None, None]; // [raw, shifted] -> ambos campos plausíveis?
    match crate::registry() {
        None => crate::log_dev!("[worldwidgets-diag] registry indisponível"),
        Some(reg) => match ink_layers_array(reg) {
            None => crate::log_dev!("[worldwidgets-diag] ink_layers_array() falhou (singleton/layerManagers indisponível)"),
            Some((entries, size)) => {
                let want = cname("inkWorldLayer");
                let idx = find_layer_index_by_hash(entries, size, want);
                crate::log_dev!("[worldwidgets-diag] FindWorldLayerIndex -> {idx} (de {size} layers)");
                if idx < 0 || entries == 0 {
                    crate::log_dev!("[worldwidgets-diag] inkWorldLayer NÃO achado nesta RTTI — capacidade genuinamente ausente/inacessível por nome nesta build, não é falha de offset");
                } else {
                    let slot = (entries as *mut u8).add(idx as usize * 16) as *const c_void;
                    if !crate::memoria::is_readable(slot, 8) {
                        crate::log_dev!("[worldwidgets-diag] slot do layer ilegível");
                    } else {
                        let world_layer = rd_u64(slot, 0x00);
                        crate::log_dev!("[worldwidgets-diag] worldLayer instance={world_layer:#x} (heap, sem vmaddr estático)");
                        if world_layer != 0 {
                            // pares (unk158@raw/shift, components@raw/shift) — MESMO índice de
                            // shift em ambos os campos, testados juntos.
                            let pairs: [(usize, usize, &str); 2] =
                                [(0x158, 0x170, "raw(Windows puro)"), (0x160, 0x178, "shift(+0x08 Itanium)")];
                            for (i, (unk158_off, components_off, label)) in pairs.iter().enumerate() {
                                let unk158 = dyn_array_shape(world_layer, *unk158_off);
                                let comp = dyn_array_shape(world_layer, *components_off);
                                let (u_label, u_ok) = match unk158 {
                                    None => ("ILEGIVEL".to_string(), false),
                                    Some((e, c, s)) => {
                                        let cl = dyn_array_classify(e, c, s);
                                        (format!("{cl} (entries={e:#x} cap={c} size={s})"), is_plausible(cl))
                                    }
                                };
                                let (c_label, c_ok, comp_shape) = match comp {
                                    None => ("ILEGIVEL".to_string(), false, None),
                                    Some((e, c, s)) => {
                                        let cl = dyn_array_classify(e, c, s);
                                        (format!("{cl} (entries={e:#x} cap={c} size={s})"), is_plausible(cl), Some((e, c, s)))
                                    }
                                };
                                let both_ok = u_ok && c_ok;
                                shift_verdict[i] = Some(both_ok);
                                crate::log_dev!(
                                    "[worldwidgets-diag] candidato {label}: unk158@+{unk158_off:#x}={u_label} | components@+{components_off:#x}={c_label} | AMBOS_PLAUSIVEIS={both_ok}"
                                );
                                // se components ficou plausível E populado (cap>=1), dump dos
                                // primeiros slots (SharedPtr<IWidgetComponentWrapper>) — só
                                // leitura, nunca chama vtable.
                                if let Some((e, _cap, sz)) = comp_shape {
                                    if c_ok && sz >= 1 {
                                        for j in 0..sz.min(6) {
                                            let cslot = (e as *mut u8).add(j as usize * 16) as *const c_void;
                                            if !crate::memoria::is_readable(cslot, 16) {
                                                crate::log_dev!("[worldwidgets-diag]   [{j}] slot ilegível");
                                                continue;
                                            }
                                            let inst = rd_u64(cslot, 0x00);
                                            let refc = rd_u64(cslot, 0x08);
                                            if inst != 0 && crate::memoria::is_readable(inst as *const c_void, 8) {
                                                let vtbl = rd_u64(inst as *const c_void, 0x00);
                                                crate::log_dev!(
                                                    "[worldwidgets-diag]   [{j}] instance={inst:#x} refCount={refc:#x} vtbl={vtbl:#x} (vmaddr={:#x})",
                                                    to_vmaddr(vtbl)
                                                );
                                            } else {
                                                crate::log_dev!(
                                                    "[worldwidgets-diag]   [{j}] instance={inst:#x} refCount={refc:#x} (instance nula/ilegível)"
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        },
    }
    let verdict = shift_verdict[0].unwrap_or(false) || shift_verdict[1].unwrap_or(false);
    crate::log_dev!(
        "[worldwidgets-diag] veredito: raw_ok={:?} shift_ok={:?} game_base={base:#x} link_base={LINK_BASE:#x}",
        shift_verdict[0], shift_verdict[1]
    );
    write_bool_ret(out, verdict);
}

pub(crate) unsafe fn register_inksystem_worldwidgets_diag(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(
        reg, proto, &[], "BwmsInkSystemWorldWidgetsDiag", "BwmsInkSystemWorldWidgetsDiag", corpo_de_prova!(tramp_inksystem_worldwidgets_diag),
    ));
    crate::log_dev!("[reg] register_all: cw-100 GetWorldWidgets diag(observe-only)={ok}");
}

// ===== `#93` (`inkLayerWrapper` — bloqueio original: "só é
// útil se algo já entregar uma instância pronta ... que precisa do singleton
// Red::InkSystem::Get()") — 2026-08-14, MESMA sessão que confirmou o singleton por
// cross-validação decisiva (`inksystemvalidate`) e provou `BwmsInkSystemGetLayer(index)`
// devolvendo um HANDLE REAL/utilizável (`layerHandle.GetClassName()` bateu exato). Com o
// bloqueador resolvido, os 4 métodos que `inkLayerWrapper` expõe (`GetLayerName`/
// `GetVirtualWindow`/`GetGameController`/`GetGameControllers`) ficam implementáveis por
// COMPOSIÇÃO DE CAMPO sobre o layer HANDLE já obtido — sem forjar a classe `inkLayerWrapper`
// em si (`RTTI_DEFINE_CLASS` real, mas `layer: Handle<inkLayer>` como campo próprio exigiria
// bind de classe nova; mesma categoria de risco desproporcional já evitada pro `#100`/clipboard
// com a mesma técnica de "global BWMS-própria recebendo o handle como parâmetro").
//
// Layout `Red::ink::Layer` confirmado (offset conferido em 3 dos 4 campos usados aqui):
//   window: Handle<VirtualWindow> @ +0x40 (conferido)
//   libraryItem: Handle<WidgetLibraryItemInstance> @ +0x50 (não conferido individualmente, mas
//     contíguo no mesmo struct entre 2 campos que SÃO conferidos — mesma confiança prática já
//     aceita pra campos vizinhos em outros layouts deste projeto)
//   controllerProcessor: Handle<ControllerProcessor> @ +0x60 (conferido)
// `WidgetLibraryItemInstance::gameController` @ +0x48 vem da reflection real do jogo —
// confiança máxima, mesma categoria já usada pro `#132`/`ComponentTarget`.
// `ControllerProcessor::controllers` (DynArray<Handle<IWidgetController>>) @ +0x50, também
// conferido.
//
// `GetGameControllers()` (array) NÃO vira um retorno de `array<T>` de verdade (categoria de
// risco que já crashou este projeto antes de a técnica do "trailer de allocator-vft" ser
// achada) — em vez disso, expõe count+index (mesmo padrão seguro já
// usado pra `layerManagers`/`layers` acima, `GetLayerCount`+`GetLayer(index)`).
pub(crate) unsafe fn read_handle_field(base: *mut c_void, field_offset: usize) -> *mut c_void {
    if base.is_null() || !crate::memoria::is_readable(base, field_offset + 8) {
        return core::ptr::null_mut();
    }
    let field = (base as *mut u8).add(field_offset) as *const c_void;
    rd_u64(field, 0x00) as *mut c_void
}

/// `BwmsInkLayerGetName(layer: ref<IScriptable>) -> CName` — `layer->GetNativeType()->name`
/// == `type_name_getname(class_of(layer))`, já provado. Diferente de
/// `GetLayerClassNameAt(index)` (que só enumera `layerManagers[0]->layers`), aceita QUALQUER
/// handle de layer já obtido — capacidade mais geral.
pub(crate) unsafe extern "C" fn tramp_inklayer_get_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let layer = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(core::ptr::null_mut());
    let mut name_hash: u64 = 0;
    if !layer.is_null() && rtti::sane(layer) {
        let cls = rtti::class_of(layer);
        if rtti::sane(cls) {
            name_hash = rtti::type_name_getname(cls);
        }
    }
    crate::log_dev!("[inklayer] BwmsInkLayerGetName({layer:p}) -> hash={name_hash:#x}");
    write_uint_ret(out, name_hash, 8);
}

/// `BwmsInkLayerGetWindow(layer: ref<IScriptable>) -> wref<IScriptable>` — `layer->window`
/// (`inkLayer+0x40`, campo assertado). `wref` = não-owning (mesma semântica do `WeakHandle<T>`
/// real via `rtti::make_weak_handle`, já provado).
pub(crate) unsafe extern "C" fn tramp_inklayer_get_window(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let layer = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(core::ptr::null_mut());
    let window = if !layer.is_null() && rtti::sane(layer) { read_handle_field(layer, 0x40) } else { core::ptr::null_mut() };
    crate::log_dev!("[inklayer] BwmsInkLayerGetWindow({layer:p}) -> {window:p}");
    rtti::make_weak_handle(out, window);
}

/// `BwmsInkLayerGetGameController(layer: ref<IScriptable>) -> wref<IScriptable>` —
/// `layer->libraryItem->gameController` (`inkLayer+0x50` -> `WidgetLibraryItemInstance+0x48`).
pub(crate) unsafe extern "C" fn tramp_inklayer_get_game_controller(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let layer = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(core::ptr::null_mut());
    let mut controller: *mut c_void = core::ptr::null_mut();
    if !layer.is_null() && rtti::sane(layer) {
        let library_item = read_handle_field(layer, 0x50);
        if !library_item.is_null() && rtti::sane(library_item) {
            controller = read_handle_field(library_item, 0x48);
        }
    }
    crate::log_dev!("[inklayer] BwmsInkLayerGetGameController({layer:p}) -> {controller:p}");
    rtti::make_weak_handle(out, controller);
}

/// Núcleo comum: `layer->controllerProcessor->controllers` (`DynArray<Handle<IWidgetController>>`
/// @ `ControllerProcessor+0x50`) — devolve `(entries,size)` ou `None`.
pub(crate) unsafe fn ink_layer_controllers_array(layer: *mut c_void) -> Option<(u64, u32)> {
    if layer.is_null() || !rtti::sane(layer) {
        return None;
    }
    let processor = read_handle_field(layer, 0x60);
    if processor.is_null() || !rtti::sane(processor) || !crate::memoria::is_readable(processor, 0x50 + 16) {
        return None;
    }
    let controllers_field = (processor as *mut u8).add(0x50) as *const c_void;
    let entries = rd_u64(controllers_field, 0x00);
    let size = core::ptr::read_unaligned((controllers_field as *const u8).add(0x0C) as *const u32);
    Some((entries, size))
}

/// `BwmsInkLayerGetControllerCount(layer: ref<IScriptable>) -> Int32` — parte 1/2 do
/// equivalente índice-seguro de `GetGameControllers()->array<wref<IWidgetController>>`.
pub(crate) unsafe extern "C" fn tramp_inklayer_get_controller_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let layer = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(core::ptr::null_mut());
    let n = ink_layer_controllers_array(layer).map(|(_, n)| n).unwrap_or(0);
    crate::log_dev!("[inklayer] BwmsInkLayerGetControllerCount({layer:p}) -> {n}");
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsInkLayerGetControllerAt(layer: ref<IScriptable>, index: Int32) -> wref<IScriptable>` —
/// parte 2/2. Índice fora do alcance/array vazio devolve handle NULO, nunca crasha (mesmo
/// idioma de `BwmsInkSystemGetLayer`).
pub(crate) unsafe extern "C" fn tramp_inklayer_get_controller_at(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let layer = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(core::ptr::null_mut());
    let index = args.get(1).map(|(v, _)| *v as i64 as i32).unwrap_or(-1);
    let mut controller: *mut c_void = core::ptr::null_mut();
    if let Some((entries, size)) = ink_layer_controllers_array(layer) {
        if index >= 0 && (index as u32) < size && entries != 0 {
            let slot = (entries as *mut u8).add(index as usize * 16) as *const c_void;
            if crate::memoria::is_readable(slot, 8) {
                controller = rd_u64(slot, 0x00) as *mut c_void;
            }
        }
    }
    crate::log_dev!("[inklayer] BwmsInkLayerGetControllerAt({layer:p}, {index}) -> {controller:p}");
    rtti::make_weak_handle(out, controller);
}

pub(crate) unsafe fn register_inklayer_field_utils(reg: &Registry, proto: *mut c_void) {
    let hc = &["handle:IScriptable"];
    let hci = &["handle:IScriptable", "Int32"];
    let ok1 = register_argful_by_types(reg, proto, hc, "BwmsInkLayerGetName", "BwmsInkLayerGetName", tramp_inklayer_get_name);
    let ok2 = register_argful_by_types(reg, proto, hc, "BwmsInkLayerGetWindow", "BwmsInkLayerGetWindow", tramp_inklayer_get_window);
    let ok3 = register_argful_by_types(
        reg, proto, hc, "BwmsInkLayerGetGameController", "BwmsInkLayerGetGameController", tramp_inklayer_get_game_controller,
    );
    let ok4 = register_argful_by_types(
        reg, proto, hc, "BwmsInkLayerGetControllerCount", "BwmsInkLayerGetControllerCount", tramp_inklayer_get_controller_count,
    );
    let ok5 = register_argful_by_types(
        reg, proto, hci, "BwmsInkLayerGetControllerAt", "BwmsInkLayerGetControllerAt", tramp_inklayer_get_controller_at,
    );
    crate::log_dev!(
        "[reg] register_all: cw-93 inkLayer field utils Name={ok1} Window={ok2} GameController={ok3} ControllerCount={ok4} ControllerAt={ok5}"
    );
}


/// Editores vivos e o próximo id a emitir. O contador mora no mesmo `Mutex` do mapa: emitir o id e inserir o editor são um passo só,
/// então dois campos nunca recebem o mesmo id.
pub(crate) struct EditoresAbertos {
    proximo_id: i32,
    pub(crate) por_id: std::collections::HashMap<i32, bwms_primitives::editor_texto::EditorTexto>,
}

impl EditoresAbertos {
    pub(crate) fn vazio() -> Self {
        Self { proximo_id: 1, por_id: std::collections::HashMap::new() }
    }

    /// Insere um editor vazio e devolve o id dele, sempre ≥ 1: `arg_i32` devolve 0 quando o parâmetro não se lê, e um editor de id 0
    /// receberia a edição de toda chamada quebrada. Depois de `i32::MAX` o contador volta a 1 e pula os ids ainda abertos.
    pub(crate) fn abrir(&mut self) -> i32 {
        loop {
            let id = self.proximo_id;
            self.proximo_id = id.checked_add(1).unwrap_or(1);
            if let std::collections::hash_map::Entry::Vacant(v) = self.por_id.entry(id) {
                v.insert(bwms_primitives::editor_texto::EditorTexto::default());
                return id;
            }
        }
    }
}

pub(crate) static EDITORES: std::sync::OnceLock<Mutex<EditoresAbertos>> = std::sync::OnceLock::new();

pub(crate) fn editores() -> &'static Mutex<EditoresAbertos> {
    EDITORES.get_or_init(|| Mutex::new(EditoresAbertos::vazio()))
}

/// Roda `f` sobre o editor de `id`. Só `BwmsTextEditOpen` insere: id que ele não emitiu, ou que `BwmsTextEditDrop` já soltou, devolve
/// `None` sem criar nada — leitura ou tecla atrasada de um campo fechado não ressuscita uma entrada que ninguém soltaria de novo.
pub(crate) fn com_editor<R>(id: i32, f: impl FnOnce(&mut bwms_primitives::editor_texto::EditorTexto) -> R) -> Option<R> {
    let mut m = editores().lock().unwrap_or_else(|e| e.into_inner());
    m.por_id.get_mut(&id).map(f)
}

/// Códigos de `Movimento` que a `.reds` manda. Valor fora da faixa não mexe no cursor.
pub(crate) fn movimento_de_codigo(c: i32) -> Option<bwms_primitives::editor_texto::Movimento> {
    use bwms_primitives::editor_texto::Movimento::*;
    Some(match c {
        0 => Esquerda,
        1 => Direita,
        2 => PalavraEsquerda,
        3 => PalavraDireita,
        4 => Inicio,
        5 => Fim,
        _ => return None,
    })
}

pub(crate) unsafe fn arg_i32(a: &[(u64, u64)], i: usize) -> i32 {
    a.get(i).map(|(v, _)| *v as i32).unwrap_or(0)
}

/// `BwmsTextEditOpen() -> Int32`: id de um editor vazio novo (ver `EditoresAbertos::abrir`).
pub(crate) unsafe extern "C" fn tramp_textedit_open(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let _ = rtti::read_params_consuming(current_native_func(), frame);
    let id = editores().lock().unwrap_or_else(|e| e.into_inner()).abrir();
    write_i32_ret(out, id);
}

/// `BwmsTextEditSet`/`Insert`/`Move`/`Erase`/`Select` devolvem `Int32` com os bits de `editor_texto::MUDOU_TEXTO`/`MUDOU_CARET`: 0 com id
/// desconhecido, movimento inválido ou operação sem efeito. A `.reds` declara `-> Int32`: escrever 4 bytes num retorno declarado `Bool` passa do slot.
pub(crate) unsafe extern "C" fn tramp_textedit_set(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let t = strings.get(1).cloned().flatten().unwrap_or_default();
    let bits = com_editor(id, |e| e.definir_texto(&t)).unwrap_or(0);
    write_i32_ret(out, i32::from(bits));
}

pub(crate) unsafe extern "C" fn tramp_textedit_insert(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (a, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let t = strings.get(1).cloned().flatten().unwrap_or_default();
    let bits = com_editor(id, |e| e.inserir(&t)).unwrap_or(0);
    write_i32_ret(out, i32::from(bits));
}

pub(crate) unsafe extern "C" fn tramp_textedit_move(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let cod = arg_i32(&a, 1);
    let estender = a.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let bits = match movimento_de_codigo(cod) {
        Some(m) => com_editor(id, |e| e.mover(m, estender)).unwrap_or(0),
        None => 0,
    };
    write_i32_ret(out, i32::from(bits));
}

pub(crate) unsafe extern "C" fn tramp_textedit_erase(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    // `depois=true` é Delete; `false` é Backspace. Um só nativo porque as duas
    // operações compartilham a regra de "seleção primeiro".
    let depois = a.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let bits = com_editor(id, |e| if depois { e.apagar_depois() } else { e.apagar_antes() }).unwrap_or(0);
    write_i32_ret(out, i32::from(bits));
}

pub(crate) unsafe extern "C" fn tramp_textedit_select(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let tudo = a.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let bits = com_editor(id, |e| if tudo { e.selecionar_tudo() } else { e.limpar_selecao() }).unwrap_or(0);
    write_i32_ret(out, i32::from(bits));
}

pub(crate) unsafe extern "C" fn tramp_textedit_cut(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let s = com_editor(arg_i32(&a, 0), |e| e.recortar()).unwrap_or_default();
    let _ = write_cstring_inline_ret(out, &s);
}

pub(crate) unsafe extern "C" fn tramp_textedit_text(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let selecionado = a.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let s = com_editor(id, |e| if selecionado { e.texto_selecionado() } else { e.texto() }).unwrap_or_default();
    let _ = write_cstring_inline_ret(out, &s);
}

pub(crate) unsafe extern "C" fn tramp_textedit_num(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    // 0=cursor, 1=comprimento, 2=início da seleção, 3=fim da seleção.
    // Sem seleção, 2 e 3 devolvem o cursor — assim a `.reds` desenha um caret
    // sem precisar de um caminho separado pro caso "nada selecionado".
    let qual = arg_i32(&a, 1);
    let v = com_editor(id, |e| match qual {
        1 => e.comprimento() as i32,
        2 => e.intervalo().map(|(i, _)| i).unwrap_or_else(|| e.cursor()) as i32,
        3 => e.intervalo().map(|(_, f)| f).unwrap_or_else(|| e.cursor()) as i32,
        _ => e.cursor() as i32,
    })
    .unwrap_or(0);
    write_i32_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_textedit_drop(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let id = arg_i32(&a, 0);
    let existia = editores().lock().unwrap_or_else(|e| e.into_inner()).por_id.remove(&id).is_some();
    write_bool_ret(out, existia);
}

pub(crate) unsafe extern "C" fn tramp_textedit_scroll(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let g = |i: usize| f32::from_bits(a.get(i).map(|(v, _)| *v as u32).unwrap_or(0));
    let r = bwms_primitives::editor_texto::deslocamento_para_caret(g(0), g(1), g(2), g(3), g(4));
    core::ptr::write_unaligned(out as *mut f32, r);
}

#[cfg(test)]
mod editores_abertos_tests {
    use super::*;

    #[test]
    fn abrir_emite_ids_distintos_e_nunca_zero() {
        let mut r = EditoresAbertos::vazio();
        let a = r.abrir();
        let b = r.abrir();
        assert!(a >= 1 && b >= 1, "id 0 é o fallback de arg_i32: a={a} b={b}");
        assert_ne!(a, b);
        assert_eq!(r.por_id.len(), 2);
    }

    #[test]
    fn contador_volta_a_1_pulando_id_aberto() {
        let mut r = EditoresAbertos::vazio();
        assert_eq!(r.abrir(), 1);
        r.proximo_id = i32::MAX;
        assert_eq!(r.abrir(), i32::MAX);
        // 1 continua aberto: a volta pula para 2, sem emitir 0 nem negativo.
        assert_eq!(r.abrir(), 2);
    }

    // Único teste que toca o registro global; os outros usam um `EditoresAbertos` local, então a contagem não corre com outro teste.
    #[test]
    fn id_desconhecido_nao_cria_editor() {
        let tamanho = || editores().lock().unwrap_or_else(|e| e.into_inner()).por_id.len();
        let id = editores().lock().unwrap_or_else(|e| e.into_inner()).abrir();
        assert_ne!(id, 0);
        use bwms_primitives::editor_texto::{MUDOU_CARET, MUDOU_TEXTO};
        assert_eq!(com_editor(id, |e| e.inserir("olá")), Some(MUDOU_TEXTO | MUDOU_CARET));
        assert_eq!(com_editor(id, |e| e.texto()).as_deref(), Some("olá"));

        let antes = tamanho();
        for desconhecido in [0, -1, i32::MIN] {
            assert_eq!(com_editor(desconhecido, |e| e.texto()), None);
            assert_eq!(com_editor(desconhecido, |e| e.inserir("x")), None);
        }
        assert_eq!(tamanho(), antes);

        // Leitura e escrita depois do Drop não recriam a entrada.
        assert!(editores().lock().unwrap_or_else(|e| e.into_inner()).por_id.remove(&id).is_some());
        let depois_do_drop = tamanho();
        assert_eq!(com_editor(id, |e| e.texto()), None);
        assert_eq!(com_editor(id, |e| e.definir_texto("volta")), None);
        assert_eq!(tamanho(), depois_do_drop);
        assert!(!editores().lock().unwrap_or_else(|e| e.into_inner()).por_id.contains_key(&id));
    }
}

/// `unk40` — onde mora o `SharedPtr<worlduiWorldLayerWidget>` dentro de `inkWorldWidgetInfos`.
pub(crate) const WWI_WRAPPER_OFF: usize = 0x40;

/// `unk08` — onde mora o `WeakHandle<IComponent>` dentro do wrapper.
pub(crate) const WWI_COMPONENT_OFF: usize = 0x08;

/// `inkWidget::parentWidget` — `WeakHandle<Widget>@0x168`. Campo de DADO: porta 1:1, sem shift Itanium.
pub(crate) const INKWIDGET_PARENT_OFF: usize = 0x168;

/// Pai de `w` pelo `WeakHandle` em `INKWIDGET_PARENT_OFF`, ou 0: raiz (pai nulo), pai destruído (o bloco segue vivo pelo weak do filho,
/// sem strong) ou vtable lixo. O único leitor do campo; `BwmsWidgetParent` e `inkWidget.GetParentWidget` passam por aqui.
pub(crate) unsafe fn pai_do_widget(w: *const c_void) -> u64 {
    if w.is_null() {
        return 0;
    }
    match weak_vivo(w as *const u8, INKWIDGET_PARENT_OFF) {
        Some(p) if rtti::sane(p as *mut c_void) && !rtti::class_of(p as *mut c_void).is_null() => p,
        _ => 0,
    }
}

/// `BwmsWidgetParent(widget) -> ref<IScriptable>` — sobe UM nível na árvore de widget.
///
/// Sem `@addField(inkWidget) public native let parentWidget`: o CAMPO existe no motor (o offset
/// acima); o que não existe é a property script-visível.
/// Ler pelo Rust entrega a capacidade sem declarar property nenhuma.
pub(crate) unsafe extern "C" fn tramp_widget_parent(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let w = a.first().map(|(v, _)| *v as *const c_void).unwrap_or(std::ptr::null());
    write_handle_ret(out, pai_do_widget(w));
}

/// `BwmsInkWorldWidgetComponent(userData) -> ref<IScriptable>` — o `GetComponent()` do item.
///
/// Devolve handle nulo em qualquer incerteza (userData nulo, wrapper nulo, memória ilegível) —
/// nunca um ponteiro "quase certo". `wrapper` nulo é o caso NORMAL, não erro: um widget que não é de mundo simplesmente não tem componente.
pub(crate) unsafe extern "C" fn tramp_ink_world_widget_component(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let ud = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let comp = ink_world_widget_component(ud);
    crate::log_dev!("[cw96] userData={ud:p} -> component={comp:#x}");
    write_handle_ret(out, comp);
}

/// Núcleo dos 2 saltos, separado do trampolim pra ficar legível e ter um só ponto de guarda.
pub(crate) unsafe fn ink_world_widget_component(ud: *mut c_void) -> u64 {
    if ud.is_null() || !crate::memoria::is_readable(ud, WWI_WRAPPER_OFF + 8) {
        return 0;
    }
    let wrapper = rd_u64(ud as *const c_void, WWI_WRAPPER_OFF) as usize;
    // `!wrapper`: ausência legítima, não falha.
    if wrapper == 0 || !crate::memoria::is_readable(wrapper as *const c_void, WWI_COMPONENT_OFF + 8) {
        return 0;
    }
    let comp = rd_u64(wrapper as *const c_void, WWI_COMPONENT_OFF);
    // `WeakHandle` guarda `{instance*, refcount*}`: só o 1º ponteiro interessa, e ele tem que
    // ser um objeto RTTI de verdade — senão devolveríamos lixo tipado como `ref<>`, que crasha
    // do lado redscript no primeiro uso.
    if comp == 0 || !rtti::sane(comp as *mut c_void) || rtti::class_of(comp as *mut c_void).is_null() {
        return 0;
    }
    comp
}

pub(crate) unsafe fn register_textedit_115_utils(reg: &Registry, proto: *mut c_void) {
    let o = register_argful_by_types(reg, proto, &[], "BwmsTextEditOpen", "BwmsTextEditOpen", tramp_textedit_open as NativeHandler);
    let a = register_argful_by_types(reg, proto, &["Int32", "String"], "BwmsTextEditSet", "BwmsTextEditSet", tramp_textedit_set as NativeHandler);
    let b = register_argful_by_types(reg, proto, &["Int32", "String"], "BwmsTextEditInsert", "BwmsTextEditInsert", tramp_textedit_insert as NativeHandler);
    let c = register_argful_by_types(reg, proto, &["Int32", "Int32", "Bool"], "BwmsTextEditMove", "BwmsTextEditMove", tramp_textedit_move as NativeHandler);
    let d = register_argful_by_types(reg, proto, &["Int32", "Bool"], "BwmsTextEditErase", "BwmsTextEditErase", tramp_textedit_erase as NativeHandler);
    let e = register_argful_by_types(reg, proto, &["Int32", "Bool"], "BwmsTextEditSelect", "BwmsTextEditSelect", tramp_textedit_select as NativeHandler);
    let f = register_argful_by_types(reg, proto, &["Int32"], "BwmsTextEditCut", "BwmsTextEditCut", tramp_textedit_cut as NativeHandler);
    let g = register_argful_by_types(reg, proto, &["Int32", "Bool"], "BwmsTextEditText", "BwmsTextEditText", tramp_textedit_text as NativeHandler);
    let h = register_argful_by_types(reg, proto, &["Int32", "Int32"], "BwmsTextEditNum", "BwmsTextEditNum", tramp_textedit_num as NativeHandler);
    let i = register_argful_by_types(reg, proto, &["Int32"], "BwmsTextEditDrop", "BwmsTextEditDrop", tramp_textedit_drop as NativeHandler);
    let j = register_argful_by_types(reg, proto, &["Float", "Float", "Float", "Float", "Float"], "BwmsTextEditScroll", "BwmsTextEditScroll", tramp_textedit_scroll as NativeHandler);
    crate::log_dev!(
        "[reg] register_all: TextEdit open={o} set={a} insert={b} move={c} erase={d} select={e} cut={f} text={g} num={h} drop={i} scroll={j}"
    );
}




pub(crate) unsafe fn register_inkcharacterevent_utils(reg: &Registry, proto: *mut c_void) -> String {
    let ok = registro_de_prova!(register_argful_by_types(
        reg, proto, &["EInputAction", "inkCharacterEventType"],
        "BwmsMakeTestInkCharacterEvent", "BwmsMakeTestInkCharacterEvent", corpo_de_prova!(tramp_make_test_inkcharacterevent),
    ));
    format!("MakeTest={ok}")
}

// ===== 176b — `BwmsInkKeyEvent extends inkInputEvent` (2026-09-13) =====
// O evento de tecla entregue ao widget com foco, com nome do BWMS. Pai nativo `inkInputEvent`
// (0x90 B); doador `inkCharacterEvent` (0xA0 B; o construtor zera um byte em `@0x98`), que dá a vtable, o construtor e o tamanho. Os dados moram no próprio objeto: ação `@0x90` e caractere
// `@0x94` (os campos do `inkCharacterEvent`), tecla `@0x96` (u16, no alinhamento do doador) e o `&keyboardState` do `InkSystem` em `@0x68`, de onde os
// `Is*Down()` herdados leem. Layout: prova de 2026-09-13 (RE offline que identificou o `ProcessCharacterEvent`).
pub(crate) const BIKE_TECLADO: usize = 0x68;

pub(crate) const BIKE_TAMANHO: u32 = 0xA0;

/// Forja `BwmsInkKeyEvent` e registra os 4 getters. Chamado pelo validador de classe quando o `.reds` declara a classe.
pub unsafe fn register_bwmsinkkeyevent(reg: &Registry) -> String {
    let pai = reg.class_by_name("inkInputEvent");
    let doador = reg.class_by_name("inkCharacterEvent");
    if !rtti::sane(pai) || !rtti::sane(doador) {
        return format!("[reg] register_bwmsinkkeyevent: inkInputEvent {pai:p} ou inkCharacterEvent {doador:p} não resolveu");
    }
    let tamanho = ((doador as *const u8).add(0x68) as *const u32).read_unaligned();
    BIKE_LAYOUT_OK.store(tamanho == BIKE_TAMANHO, std::sync::atomic::Ordering::Relaxed);
    let forjada = register_type_instantiable_with_parent(reg, "BwmsInkKeyEvent", "inkCharacterEvent", pai);
    if forjada.is_null() {
        return "[reg] register_bwmsinkkeyevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_bwmsinkkeyevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "BwmsInkKeyEvent", proto, "GetKey", "GetKey", tramp_bike_get_key, false);
    let m2 = register_method(reg, "BwmsInkKeyEvent", proto, "GetAction", "GetAction", tramp_bike_get_action, false);
    let m3 = register_method(reg, "BwmsInkKeyEvent", proto, "GetCharacter", "GetCharacter", tramp_bike_get_character, false);
    let m4 = register_method(reg, "BwmsInkKeyEvent", proto, "IsCharacter", "IsCharacter", tramp_bike_is_character, false);
    format!(
        "[reg] BwmsInkKeyEvent: classe=OK (pai inkInputEvent, doador inkCharacterEvent {tamanho:#x}, layout {}) GetKey={m1} GetAction={m2} GetCharacter={m3} IsCharacter={m4}",
        if tamanho == BIKE_TAMANHO { "ok" } else { "DIFERENTE" }
    )
}

/// Cria um `BwmsInkKeyEvent` com tecla, ação, caractere (US-QWERTY com shift, a conversão do `ProcessCharacterEvent`) e o `&keyboardState` do
/// `InkSystem`. Nulo se a classe não foi forjada com o layout esperado. Só na game thread.
pub(crate) unsafe fn bwms_ink_key_event_novo(reg: &Registry, keyboard_state: *const u8, tecla: u32, acao: u32) -> *mut c_void {
    if !BIKE_LAYOUT_OK.load(std::sync::atomic::Ordering::Relaxed) {
        return std::ptr::null_mut();
    }
    let obj = rtti::new_object(reg, "BwmsInkKeyEvent");
    if obj.is_null() || !crate::memoria::is_readable(obj as *const c_void, BIKE_TAMANHO as usize) {
        return std::ptr::null_mut();
    }
    let shift = !keyboard_state.is_null()
        && crate::memoria::is_readable(keyboard_state as *const c_void, 2)
        && (keyboard_state as *const u16).read_unaligned() & 0b11 != 0;
    let b = obj as *mut u8;
    (b.add(BIKE_TECLADO) as *mut *const u8).write_unaligned(keyboard_state);
    (b.add(BIKE_ACAO) as *mut u32).write_unaligned(acao);
    b.add(BIKE_CARACTERE).write(to_character_key(tecla as i32, shift).bytes().next().unwrap_or(0));
    (b.add(BIKE_TECLA) as *mut u16).write_unaligned(tecla as u16);
    obj
}

pub(crate) unsafe fn register_widgetinputservice_utils(reg: &Registry, proto: *mut c_void) -> String {
    let ok1 = register_argful_by_types(reg, proto, &["EInputKey"], "BwmsIsCharacterInput", "BwmsIsCharacterInput", tramp_is_character_input);
    let ok2 = register_argful_by_types(reg, proto, &["EInputKey", "Bool"], "BwmsToCharacterKey", "BwmsToCharacterKey", tramp_to_character_key);
    format!("IsCharacterInput={ok1} ToCharacterKey={ok2}")
}

/// `BwmsWidgetAttachTree(widget) -> Bool` — [`c175_anexar_arvore`].
pub(crate) unsafe extern "C" fn tramp_widget_attach_tree(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let widget = a.first().map_or(0, |(v, _)| *v) as *mut u8;
    write_bool_ret(out, !widget.is_null() && crate::interface::c175_anexar_arvore(widget));
}

/// `BwmsWidgetAttachController(widget, controller, secondary) -> Bool` — o `inkWidget.AttachController`.
pub(crate) unsafe extern "C" fn tramp_widget_attach_controller(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let widget = a.first().map_or(0, |(v, _)| *v) as *mut u8;
    let controller = a.get(1).map_or(0, |(v, _)| *v) as *mut u8;
    let secundario = a.get(2).is_some_and(|(v, _)| rtti::bool_do_script(*v));
    write_bool_ret(out, !widget.is_null() && !controller.is_null() && crate::interface::c175_ligar_controller(widget, controller, secundario));
}

/// Teste da cobertura 1.21.1 (`bwms-c121-teste.reds`): `BwmsC121Medir(widget, controller) -> Int32` lê, depois do `AttachController` vanilla da 2.5
/// (`0x1043a399c` → `AttachController` nativo `0x104154158`), o estado que decide o `OnInitialize`: bit 0 widget com
/// `layerProxy` (`+0x148`), bit 1 camada viva em `proxy+0x48` (a que o nativo trava), bit 2 camada viva em `proxy+0x38` (a do `c175`, medido na
/// 2.31), bit 3 controller com proxy (`+0x50`), bit 4 controller com widget (`+0x40`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_c121_medir(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let widget = a.first().map_or(0, |(v, _)| *v) as *const u8;
    let controller = a.get(1).map_or(0, |(v, _)| *v) as *const u8;
    let viva = |proxy: u64, off: usize| proxy != 0 && c175_travar(proxy as *const u8, off).map(|mut h| crate::jogo::objetos::soltar_handle(&mut h)).is_some();
    let mut bits = 0i32;
    if !widget.is_null() && !controller.is_null() {
        let proxy = c175_campo(widget, C175_WIDGET_PROXY);
        let proxy = if proxy == u64::MAX { 0 } else { proxy };
        bits |= (proxy != 0) as i32;
        bits |= (viva(proxy, 0x48) as i32) << 1;
        bits |= (viva(proxy, C175_PROXY_CAMADA) as i32) << 2;
        let tem = |p: *const u8, o: usize| !matches!(c175_campo(p, o), 0 | u64::MAX);
        bits |= (tem(controller, C175_CONTROLLER_PROXY) as i32) << 3;
        bits |= (tem(controller, 0x40) as i32) << 4;
    }
    if !out.is_null() {
        (out as *mut i32).write_unaligned(bits);
    }
}

// ---------------------------------------------------------------------------------------------
// `inkWidgetRef` (2026-08-12): a ÚNICA
// native que faltava (`Set`) pra destravar de vez a família inteira de 17 wrappers tipados
// (`inkBorderRef`/`inkCanvasRef`/`inkTextRef`/etc., cada um só um `Create(widget)` de 4 linhas
// 100% redscript em cima de `Set`, catalogado como "barato de fechar em bloco se `Set` for
// resolvido").
//
// ACHADO IMPORTANTE (muda a categoria de risco pra BAIXA): `inkWidgetRef` NÃO é invenção de
// framework — é um `native struct` VANILLA REAL do próprio jogo
// (`redscript-src/core/ui/widgetReference.script:6`), já usado extensivamente por
// `core/ui/settingsSelectorController.script` (a MESMA fonte que o BWMS Settings já explora há
// sessões — `m_onState`/`m_offState`/etc. são campos `inkWidgetRef`), com API própria rica
// (Get/IsValid/Equals/SetVisible/PlayAnimation/Reparent/etc., todas nativas vanilla reais, zero
// RE necessária). Só falta a via de CONSTRUÇÃO — apontar o ref pra um widget arbitrário via
// ponteiro direto, sem navegação por path/nome — que é exatamente o que `Set`
// faz (`widget = aWidget;` sobre `Red::ink::WidgetReference`, zero função nativa nova no motor).
//
// Layout de `WidgetReference` (0x18: 2 virtuais
// (`GetNativeType`/`GetType`, vtable ptr@0x00) + `WeakHandle<Widget> widget@0x08`); `Set` só
// atribui `widget = aWidget`, nenhum outro campo tocado — CAMPO PURO, zero endereço nativo, zero RawFunc.
//
// Escrita via `rtti::make_weak_handle` (mesma técnica já provada pro
// `EntityComponentEvent.GetComponent`) dentro do ponteiro real por trás do
// `self: script_ref<inkWidgetRef>` (generalizado nesta rodada em
// `rtti::read_params_consuming_with_scriptrefs` — antes só extraía `String`, agora dá o
// ponteiro cru pra QUALQUER `T`). Registrado como GLOBAL (não `native` direto em `@addMethod`,
// mesma disciplina já estabelecida pro `ResRef.GetHash` — `@addMethod` NATIVO sobre `struct`
// tem histórico de bug real no compilador, ver `ponte-depot-resref-ops.reds`; aqui o
// `@addMethod(inkWidgetRef) static func Set(...)` do `.reds` fica PURO REDSCRIPT, só delega pra
// este global).
pub(crate) unsafe extern "C" fn tramp_inkwidgetref_set(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let (args, srefs) = rtti::read_params_consuming_with_scriptrefs(current_native_func(), frame);
    let self_ptr = srefs.first().copied().unwrap_or(0) as *mut c_void;
    let widget_ptr = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut wrote = false;
    if !self_ptr.is_null() && crate::memoria::is_readable(self_ptr as *const c_void, 0x18) {
        // widget@+0x08 (16 bytes, WeakHandle) — vtable ptr@0x00 fica intocado (já escrito pelo
        // motor quando a local `let ref: inkWidgetRef;` foi declarada).
        let field_ptr = (self_ptr as *mut u8).add(0x08) as *mut c_void;
        if widget_ptr.is_null() || !rtti::sane(widget_ptr) {
            // Set(widget=null) limpa a referência
            // (Handle<inkWidget>{} vazio atribuído a `widget`).
            if crate::memoria::is_readable(field_ptr as *const c_void, 0x10) {
                std::ptr::write_bytes(field_ptr as *mut u8, 0, 0x10);
                wrote = true;
            }
        } else {
            rtti::make_weak_handle(field_ptr, widget_ptr);
            wrote = true;
        }
    }
    crate::log_dev!(
        "[inkwidgetref] BwmsInkWidgetRefSet(self={self_ptr:p}, widget={widget_ptr:p}) -> wrote={wrote}"
    );
}

pub(crate) unsafe fn register_inkwidgetref_set(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["script_ref:inkWidgetRef", "handle:inkWidget"]);
    let ok = if params_compostos(params) {
        register_global_composed(reg, proto, params, "BwmsInkWidgetRefSet", "BwmsInkWidgetRefSet", tramp_inkwidgetref_set as NativeHandler)
    } else {
        crate::log_dev!("[reg] register_inkwidgetref_set: compose_params_from_types falhou (script_ref:inkWidgetRef, handle:inkWidget)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-inkwidgetref-set (#95) BwmsInkWidgetRefSet={ok}");
}

// Parte 100-4 (2026-09-12): `GetWorldWidgets` por leitura direta, pelo plano offline de 09-12. No `inkWorldLayer` (vptr `0x107c144c0`) os
// componentes ficam em `DynArray<SharedPtr<wrapper>>` `@0x168` (tamanho `+0x174`), sob a trava de byte `@0x160` (bit 0x80 exclusivo: a leitura tenta
// pegar e desiste se estiver ocupada). O wrapper (vptr `0x107c78d88`) guarda o WeakHandle do componente `@0x08`; no componente, a janela fica `@0x140`
// e o WeakHandle do game controller `@0x150`. Só leitura: nada é chamado.
pub(crate) const INKWORLDLAYER_VPTR_VM: u64 = 0x1_07c1_44c0;

pub(crate) const INKWORLDLAYER_TRAVA_OFF: usize = 0x160;

pub(crate) const INKWORLDLAYER_COMPONENTES_OFF: usize = 0x168;

pub(crate) const INKWIDGETCOMPONENTWRAPPER_VPTR_VM: u64 = 0x1_07c7_8d88;

pub(crate) const INKCOMPONENTE_JANELA_OFF: usize = 0x140;

pub(crate) const INKCOMPONENTE_CONTROLLER_OFF: usize = 0x150;

pub(crate) static INK_WORLD_STAT: std::sync::Mutex<[i32; 4]> = std::sync::Mutex::new([0; 4]);

/// Instância de um WeakHandle `{instância, bloco}` em `base+off`, se o bloco ainda tem strong.
pub(crate) unsafe fn weak_vivo(base: *const u8, off: usize) -> Option<u64> {
    if !crate::memoria::is_readable(base.add(off) as *const c_void, 16) {
        return None;
    }
    let inst = (base.add(off) as *const u64).read_unaligned();
    let bloco = (base.add(off + 8) as *const u64).read_unaligned();
    if inst == 0 || bloco == 0 || !crate::memoria::is_readable(bloco as *const c_void, 4) {
        return None;
    }
    ((bloco as *const u32).read_unaligned() > 0).then_some(inst)
}

/// `[componentes, com janela, wrappers pulados pelo vptr, componentes mortos]` e os controllers vivos dos componentes com janela. `Err`: -1 sem world
/// layer, -2 trava ocupada.
pub(crate) unsafe fn ink_world_widgets() -> Result<([i32; 4], Vec<u64>), i32> {
    ink_world_widget_pares().map(|(stat, pares)| (stat, pares.into_iter().map(|(_, c)| c).collect()))
}

/// Game controller do layer do `inkSystem` cuja classe se chama `classe` (`layer+0x50` → `libraryItem+0x48`); 0 sem layer ou sem controller.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn ink_layer_controller_por_classe(classe: &str) -> u64 {
    let Some(reg) = crate::registry() else { return 0 };
    let Some((entries, size)) = ink_layers_array(reg) else { return 0 };
    let idx = find_layer_index_by_hash(entries, size, cname(classe));
    if idx < 0 || entries == 0 {
        return 0;
    }
    let layer = rd_u64((entries as *mut u8).add(idx as usize * 16) as *const c_void, 0) as *mut c_void;
    if layer.is_null() || !rtti::sane(layer) {
        return 0;
    }
    let item = read_handle_field(layer, 0x50);
    if item.is_null() || !rtti::sane(item) {
        return 0;
    }
    read_handle_field(item, 0x48) as u64
}

/// Os pares `(componente, game controller)` dos componentes com janela do `inkWorldLayer`, com as mesmas contagens do `#100`.
pub(crate) unsafe fn ink_world_widget_pares() -> Result<([i32; 4], Vec<(u64, u64)>), i32> {
    let reg = crate::registry().ok_or(-1)?;
    let (entries, size) = ink_layers_array(reg).ok_or(-1)?;
    let idx = find_layer_index_by_hash(entries, size, cname("inkWorldLayer"));
    if idx < 0 || entries == 0 {
        return Err(-1);
    }
    let layer = rd_u64((entries as *mut u8).add(idx as usize * 16) as *const c_void, 0) as *const u8;
    if layer.is_null() || !crate::memoria::is_readable(layer as *const c_void, 0x1b0) || (layer as *const u64).read_unaligned() != crate::rebase(INKWORLDLAYER_VPTR_VM) as u64 {
        return Err(-1);
    }
    let trava = &*(layer.add(INKWORLDLAYER_TRAVA_OFF) as *const std::sync::atomic::AtomicU8);
    let mut pegou = false;
    for _ in 0..64 {
        if trava.compare_exchange(0, 0x80, std::sync::atomic::Ordering::SeqCst, std::sync::atomic::Ordering::SeqCst).is_ok() {
            pegou = true;
            break;
        }
        std::thread::yield_now();
    }
    if !pegou {
        return Err(-2);
    }
    let arr = layer.add(INKWORLDLAYER_COMPONENTES_OFF);
    let ptr = (arr as *const u64).read_unaligned() as *const u64;
    let n = (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    let mut stat = [0i32; 4];
    let mut pares = Vec::new();
    if !ptr.is_null() && n <= 4096 && crate::memoria::is_readable(ptr as *const c_void, n * 16) {
        let vt_wrapper = crate::rebase(INKWIDGETCOMPONENTWRAPPER_VPTR_VM) as u64;
        for i in 0..n {
            stat[0] += 1;
            let w = ptr.add(i * 2).read_unaligned() as *const u8;
            if w.is_null() || !crate::memoria::is_readable(w as *const c_void, 0x18) || (w as *const u64).read_unaligned() != vt_wrapper {
                stat[2] += 1;
                continue;
            }
            let Some(comp) = weak_vivo(w, 0x08) else {
                stat[3] += 1;
                continue;
            };
            let comp = comp as *const u8;
            if !crate::memoria::is_readable(comp as *const c_void, INKCOMPONENTE_CONTROLLER_OFF + 16) {
                stat[3] += 1;
                continue;
            }
            if (comp.add(INKCOMPONENTE_JANELA_OFF) as *const u64).read_unaligned() == 0 {
                continue;
            }
            stat[1] += 1;
            if let Some(c) = weak_vivo(comp, INKCOMPONENTE_CONTROLLER_OFF) {
                pares.push((comp as u64, c));
            }
        }
    }
    crate::jogo::travas::soltar_escrita(trava.as_ptr());
    Ok((stat, pares))
}

/// `BwmsInkWorldWidgetCount() -> Int32`: controllers vivos dos componentes com janela no world layer; -1 sem world layer, -2 trava ocupada.
pub(crate) unsafe extern "C" fn tramp_ink_world_widget_count(_c: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let r = match ink_world_widgets() {
        Ok((stat, cs)) => {
            crate::veredito_dev!(
                "[cw100m] world layer: {} componentes, {} com janela, {} wrappers pulados pelo vptr, {} mortos -> {} controllers",
                stat[0],
                stat[1],
                stat[2],
                stat[3],
                cs.len()
            );
            if let Ok(mut g) = INK_WORLD_STAT.lock() {
                *g = stat;
            }
            cs.len() as i32
        }
        Err(e) => e,
    };
    write_uint_ret(out, r as u32 as u64, 4);
}

/// `BwmsInkWorldWidgetController(index) -> wref<IScriptable>`: o controller `index` da lista (nulo fora dela).
pub(crate) unsafe extern "C" fn tramp_ink_world_widget_controller(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let i = args.first().map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let c = ink_world_widgets().ok().and_then(|(_, cs)| usize::try_from(i).ok().and_then(|i| cs.get(i).copied())).unwrap_or(0);
    rtti::make_weak_handle(out, c as *mut c_void);
}

/// `BwmsInkWorldWidgetStat(which) -> Int32`: 0 componentes, 1 com janela, 2 wrappers pulados pelo vptr, 3 mortos, da última contagem; -1 fora da faixa.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_ink_world_widget_stat(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let q = args.first().map(|(v, _)| *v as u32 as usize).unwrap_or(usize::MAX);
    let v = INK_WORLD_STAT.lock().map(|a| a.get(q).copied().unwrap_or(-1)).unwrap_or(-1);
    write_uint_ret(out, v as u32 as u64, 4);
}

/// `BwmsInkWorldWidgetHas(controller) -> Bool`: o objeto está entre os controllers do world layer.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_ink_world_widget_has(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tem = obj != 0 && ink_world_widgets().map_or(false, |(_, cs)| cs.contains(&obj));
    write_bool_ret(out, tem);
}

pub(crate) unsafe fn register_ink_world_widget_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &[], "BwmsInkWorldWidgetCount", "BwmsInkWorldWidgetCount", tramp_ink_world_widget_count as NativeHandler);
    let ok2 = register_argful_by_types(reg, proto, &["Int32"], "BwmsInkWorldWidgetController", "BwmsInkWorldWidgetController", tramp_ink_world_widget_controller as NativeHandler);
    let ok3 = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32"], "BwmsInkWorldWidgetStat", "BwmsInkWorldWidgetStat", corpo_de_prova!(tramp_ink_world_widget_stat)));
    let ok4 = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable"], "BwmsInkWorldWidgetHas", "BwmsInkWorldWidgetHas", corpo_de_prova!(tramp_ink_world_widget_has)));
    crate::log_dev!("[reg] register_all: cw-100 100-4 WorldWidgetCount={ok1} Controller={ok2} Stat={ok3} Has={ok4}");
}

// Parte 100-2 (2026-09-12): foco de input do inkSystem. O `RequestSetFocus` vanilla (handler `0x1041a7624`) chama
// `0x10426e550(inkSystem, &request)`, que grava o WeakHandle do widget em `InkSystem+0x2E8` (bloco de refcount em `+0x2F0`) no mesmo call.
#[cfg(feature = "devtools")]
pub(crate) const INKSYSTEM_FOCO_OFF: usize = 0x2E8;

/// `BwmsInkSystemFocusIs(widget) -> Int32`: 1 se o widget com foco é `widget`, 0 se é outro, -1 sem foco (vazio ou já destruído), -2 sem inkSystem.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_inksystem_focus_is(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let w = args.first().map(|(v, _)| *v).unwrap_or(0);
    let (r, foco) = match crate::registry().and_then(|reg| crate::jogo::interface::ink_system(reg)) {
        Some(ink) if crate::memoria::is_readable(ink as *const c_void, INKSYSTEM_FOCO_OFF + 0x10) => {
            let foco = rd_u64(ink as *const c_void, INKSYSTEM_FOCO_OFF);
            let bloco = rd_u64(ink as *const c_void, INKSYSTEM_FOCO_OFF + 8);
            let vivo = foco != 0 && bloco != 0 && crate::memoria::is_readable(bloco as *const c_void, 4) && rd_u32(bloco as *const c_void, 0) > 0;
            (if !vivo { -1 } else if foco == w { 1 } else { 0 }, foco)
        }
        _ => (-2, 0),
    };
    // A gravação do foco (`0x10426e550`) só acontece se `0x1043a28a4(widget)` = `ldrb w0, [widget, #0x1f9]` for verdadeiro (boot 57: widget novo
    // recusado). Os dois bytes do widget vão no log pra ver qual deles o `SetInteractive(true)` liga.
    let bytes = if w >= 0x1_0000_0000 && crate::memoria::is_readable(w as *const c_void, 0x1fa) {
        format!("+0x1f8={} +0x1f9={}", ((w + 0x1f8) as *const u8).read(), ((w + 0x1f9) as *const u8).read())
    } else {
        "bytes ilegíveis".to_string()
    };
    crate::veredito_dev!("[cw100] FocusIs widget={w:#x} ({bytes}) foco={foco:#x} -> {r}");
    write_uint_ret(out, r as u32 as u64, 4);
}

/// Registra o `#96` em `inkUserData` — classe VANILLA, sem forjar nada.
///
/// Fica em `inkUserData` (o pai) e não em `inkWorldWidgetInfos` porque este último não existe
/// para o compilador redscript: `GetUserData` já devolve `ref<inkUserData>`, então o mod chama o
/// método no que ele tem na mão. Widget que não é de mundo devolve `null` — caso legítimo.
pub unsafe fn register_worldwidget_helper(reg: &Registry) -> String {
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_worldwidget_helper: protótipo doador não resolveu".into(),
    };
    let m = register_method(
        reg,
        "inkUserData",
        proto,
        "GetWorldWidgetComponent",
        "GetWorldWidgetComponent",
        tramp_worldwidget_component,
        false,
    );
    format!("[reg] inkUserData: classe=VANILLA (sem forge) GetWorldWidgetComponent={m}")
}

/// O componente 3D por trás de um widget-de-mundo.
///
/// PARA QUE SERVE: um widget desenhado NO MUNDO (holograma, painel de loja, terminal) está preso a
/// um componente de entidade. Descobrir qual é a ponte entre a UI e a geometria — e é o que
/// permite posicionar, seguir ou reagir a esse widget em 3D. Para o CPVR isso é a diferença entre
/// um HUD colado na tela e um elemento que existe no espaço.
///
/// A FICHA DO CATÁLOGO DIZ "REFUTADO COM CERTEZA". Está errada: são **dois derefs**, sem
/// `RawFunc`, sem `AddressLib`, sem endereço nenhum:
///   `infos+0x40` → `SharedPtr<worlduiWorldLayerWidget>` → `wrapper+0x08` → `WeakHandle<IComponent>`
///
/// Offsets confirmados pela reflection real do jogo:
///   `ink::WorldWidgetInfos`  = 0x50, `unk40` em +0x40
///   `world::ui::WorldLayerWidget` = 0x70, `unk08` em +0x08
///
/// E o tipo NUNCA precisa ser declarado no redscript — o que evita o CMesh-trap. `GetUserData`
/// é native vanilla e devolve `ref<inkUserData>`, que é o tipo-pai; esta função recebe esse
/// handle e faz a aritmética.
/// `this` é o próprio `inkUserData` (o método é de INSTÂNCIA) — chega em
/// `ctx`, o mesmo idioma de `tramp_ece_get_component`.
pub(crate) unsafe extern "C" fn tramp_worldwidget_component(ctx: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let infos = ctx as usize;
    let mut inst: usize = 0;
    if infos != 0 && crate::memoria::is_readable((infos as *const u8).add(0x40) as *const c_void, 8) {
        // +0x40: SharedPtr<WorldLayerWidget> — o 1º campo do SharedPtr é a instância.
        let wrapper = ((infos as *const u8).add(0x40) as *const u64).read_unaligned() as usize;
        if wrapper != 0 && crate::memoria::is_readable((wrapper as *const u8).add(0x08) as *const c_void, 8) {
            // +0x08: WeakHandle<IComponent> — o 1º campo é a instância.
            inst = ((wrapper as *const u8).add(0x08) as *const u64).read_unaligned() as usize;
        }
    }
    crate::log_dev!(
        "[cw-96] GetWorldWidgetComponent() on {ctx:p} -> component={inst:#x}"
    );
    // `wref<IComponent>` de verdade — o campo é `WeakHandle`, e o projeto já tem a técnica
    // certa para isso (`make_weak_handle`). Zero = o widget não é
    // de mundo, que é caso legítimo e não erro.
    rtti::make_weak_handle(out, inst as *mut c_void);
}


/// `LocalizationString` (6ª rodada de mineração 2026-08-11) — trivial: `CreateLocalizationString(value) -> {0,
/// value}` / `ExtractLocalizationString(value) -> value.unk08`, sobre o struct
/// `Red::LocalizationString{int64 unk00; CString unk08}` (0x28 bytes).
/// Tipo CONFIRMADO redscript-visível (não injetado por framework, categoria diferente do
/// `CMesh`-trap): `orphans.script:21668` declara `LocalizationStringComponent.GetString(key:
/// CName) -> LocalizationString` como NATIVA VANILLA real do próprio jogo, então `native func
/// X(...) -> LocalizationString` compila e binda com segurança. Zero endereço nativo novo — só
/// composição de primitivas já provadas: leitura via `rtti::read_params_consuming_with_strings`
/// (extensão nova, ver `rtti.rs`, branch `tc==cname("LocalizationString")` lendo `pinst+0x08`) +
/// escrita via `write_cstring_inline_ret` no offset `+0x08` do `out` (o campo `unk00` fica zerado
/// antes, o `{0, value}`). Criação e extração aceitam texto de qualquer
/// tamanho (criação por `jogo::cstring::nova`; extração por `read_cstring`). Exposto como globais BWMS-própria.
pub(crate) unsafe extern "C" fn tramp_create_locstring(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strs) = rtti::read_params_consuming_with_strings(func, frame);
    let text = strs.first().cloned().flatten().unwrap_or_default();
    let ok = if !out.is_null() {
        core::ptr::write_bytes(out as *mut u8, 0, 8); // unk00 (int64) = 0
        write_cstring_inline_ret((out as *mut u8).add(8) as *mut c_void, &text)
    } else {
        false
    };
    crate::log_dev!(
        "[locstring] BwmsCreateLocalizationString('{text}', {} bytes) -> escrito={ok}",
        text.len()
    );
}

pub(crate) unsafe extern "C" fn tramp_extract_locstring(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strs) = rtti::read_params_consuming_with_strings(func, frame);
    let text = strs.first().cloned().flatten().unwrap_or_default();
    let ok = write_cstring_inline_ret(out, &text);
    crate::log_dev!(
        "[locstring] BwmsExtractLocalizationString() -> '{text}' ({} bytes, escrito={ok})",
        text.len()
    );
}

/// Registra `BwmsCreateLocalizationString(String)->LocalizationString` +
/// `BwmsExtractLocalizationString(LocalizationString)->String`. Params via `GetType("String")`/
/// `GetType("LocalizationString")` (já provado pra QUALQUER nome de tipo, ex. `handle:IScriptable`
/// em `BwmsCallMethod`) — dispensa proto donor com assinatura exata.
pub(crate) unsafe fn register_localizationstring_utils(reg: &Registry, proto: *mut c_void) -> String {
    let params_s = compose_params_from_types(reg, &["String"]);
    let ok1 = if params_compostos(params_s) {
        register_global_composed(reg, proto, params_s, "BwmsCreateLocalizationString", "BwmsCreateLocalizationString", tramp_create_locstring)
    } else {
        false
    };
    let params_ls = compose_params_from_types(reg, &["LocalizationString"]);
    let ok2 = if params_compostos(params_ls) {
        register_global_composed(reg, proto, params_ls, "BwmsExtractLocalizationString", "BwmsExtractLocalizationString", tramp_extract_locstring)
    } else {
        false
    };
    format!("BwmsCreateLocalizationString={ok1} BwmsExtractLocalizationString={ok2} (params_ls.count={})", params_ls.1)
}

pub(crate) unsafe extern "C" fn tramp_bike_get_key(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, bike_campo::<u16>(c, BIKE_TECLA) as u64, 4);
}

/// `BwmsImageNineSlice(image, left, top, right, bottom) -> Bool`. O script deste build não declara `SetNineSliceScale`/`SetNineSliceGrid`
/// no `inkImage`; a native grava `useNineSliceScale` (1 B) e `nineSliceScale` (`inkMargin`, 4 `Float` na ordem left/top/right/bottom) pelos offsets que o
/// RTTI reporta. `false` quando a classe do objeto não tem os dois campos.
pub(crate) unsafe extern "C" fn tramp_image_nine_slice(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let f = |k: usize| f32::from_bits(args.get(k).map(|(v, _)| *v as u32).unwrap_or(0));
    let ok = match (prop_em(obj, "useNineSliceScale"), prop_em(obj, "nineSliceScale")) {
        (Some((_, _, usar)), Some((_, _, grade))) if usar != 0 && grade != 0 && crate::memoria::is_readable(grade as *const c_void, 16) => {
            (usar as *mut u8).write(1);
            for (i, v) in [f(1), f(2), f(3), f(4)].into_iter().enumerate() {
                ((grade + i * 4) as *mut f32).write_unaligned(v);
            }
            true
        }
        _ => false,
    };
    write_bool_ret(out, ok);
}


/// `inkWidgetTarget::Matches` (2026-08-12) — TAG de qual das 2 factories
/// construiu o alvo, mesmo motivo do `EntityTargetCriterion`/`ComponentTargetCriterion`/
/// `ResourceTargetCriterion` (`Controller`/`Library` compartilhavam os mesmos 2 slots `u64` cru
/// sem distinguir a proveniência).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InkWidgetTargetCriterion {
    Controller,
    Library,
}

/// `(ptr, criterion, valor, item)` — `Controller` guarda o hash do tipo em `valor`; `Library` guarda o hash do `libraryPath` em `valor` e o `itemName` (0 =
/// qualquer) em `item`.
pub(crate) static INKWIDGETTARGET_STATES: Mutex<Vec<(usize, InkWidgetTargetCriterion, u64, u64)>> = Mutex::new(Vec::new());

/// `inkWidgetTarget.Controller(type: CName) -> ref<inkWidgetTarget>` — fábrica estática, 1 arg
/// CName (mesma receita de `tramp_ct_name`).
pub(crate) unsafe extern "C" fn tramp_iwt_controller(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ty = args.first().map(|(v, _)| *v).unwrap_or(0);
    let obj = crate::entidades::alvo_novo("inkWidgetTarget", |o| {
        if let Ok(mut states) = INKWIDGETTARGET_STATES.lock() {
            states.push((o, InkWidgetTargetCriterion::Controller, ty, 0));
        }
    });
    crate::log_dev!(
        "[iwt] inkWidgetTarget.Controller('{}' hash {ty:#018x}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))",
        crate::cname::resolve_cname(ty)
    );
    rtti::make_handle(out, obj);
}

/// `inkWidgetTarget.Library(library: ResRef, opt item: CName) -> ref<inkWidgetTarget>` — 2026-07-24,
/// 2ª leva: `ResRef` agora lê como escalar (hash `ResourcePath`). `item` (CName, opcional) também
/// lido — compilador baked um default quando omitido, mesmo esquema já confirmado pra outros `opt`
/// deste projeto (ver nota de `InputTarget`/`implement-more-targets`).
pub(crate) unsafe extern "C" fn tramp_iwt_library(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let lib_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let item = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let obj = crate::entidades::alvo_novo("inkWidgetTarget", |o| {
        if let Ok(mut states) = INKWIDGETTARGET_STATES.lock() {
            states.push((o, InkWidgetTargetCriterion::Library, lib_hash, item));
        }
    });
    crate::log_dev!(
        "[iwt] inkWidgetTarget.Library(hash={lib_hash:#018x} item='{}') -> {obj:p} (fábrica estática, ResRef lido via escalar novo)",
        crate::cname::resolve_cname(item)
    );
    rtti::make_handle(out, obj);
}

/// `inkWidgetSpawnEvent extends CallbackSystemEvent`. Os campos do evento ficam
/// nesta tabela, indexada pelo ponteiro do objeto, e não em offset dele: a classe é forjada pelo BWMS. Quem dispara é `spawn_de_widget::disparar_spawn`,
/// em todo spawn síncrono (`SpawnFromLocal`/`SpawnFromExternal`) com ouvinte de `InkWidget/Spawn`, no build público; a linha sai da tabela logo depois
/// do despacho, e um evento que o script guardou responde vazio dali em diante. A fixture de dev (`BwmsMakeTestInkWidgetSpawnEvent`) usa a mesma tabela.
///
/// `(evento, libraryPath, itemName, itemInstance)`, os 3 campos da referência. `GetLibraryPath` devolve o hash (`Uint64`), a forma do
/// `ResourceEvent.GetPath`. `itemInstance` é o `inkWidgetLibraryItemInstance*` que o spawn devolveu, vivo durante o despacho porque a saída do hook
/// segura o `Handle` até voltar ao chamador. Essa classe deriva só de `ISerializable`: no `+0x30` tem `rootResolution`, no `+0x38` o `rootWidget` e no
/// `+0x48` o `gameController`, não o `nativeType`/`valueHolder` do `IScriptable`. Declarar a classe no `.reds` derruba o bind (`#48`/`#49`), por isso o
/// `GetItemInstance` é `ref<IScriptable>`; e o `as`, o `IsA` e os métodos chamados num `ref` leem o tipo no `+0x30` (`IScriptable::GetType` inlinado no
/// handler do cast `0x101202b54`), então a instância nunca sobe crua: vai dentro de um `BwmsSerializable`.
pub(crate) static INKWIDGETSPAWN_STATES: Mutex<Vec<(usize, u64, u64, usize)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_iwse_get_library_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u64;
    if let Ok(states) = INKWIDGETSPAWN_STATES.lock() {
        if let Some((_, lp, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *lp;
        }
    }
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_iwse_get_item_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u64;
    if let Ok(states) = INKWIDGETSPAWN_STATES.lock() {
        if let Some((_, _, name, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *name;
        }
    }
    write_uint_ret(out, v, 8);
}

/// `GetItemInstance() -> ref<IScriptable>`: a instância do evento pelo `register::entregar_recurso`, com um handle forte novo. O
/// `inkWidgetLibraryItemInstance` do spawn vai dentro de um `BwmsSerializable` (`(x as BwmsSerializable).GetObjectClassName()`); objeto `IScriptable`,
/// como o da fixture de dev, vai direto. Nulo fora do despacho, quando a linha já saiu da tabela.
pub(crate) unsafe extern "C" fn tramp_iwse_get_item_instance(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut inst: usize = 0;
    if let Ok(states) = INKWIDGETSPAWN_STATES.lock() {
        if let Some((_, _, _, ii)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            inst = *ii;
        }
    }
    crate::register::entregar_recurso(out, inst as *mut c_void);
}

pub(crate) const BIKE_ACAO: usize = 0x90;

pub(crate) const BIKE_CARACTERE: usize = 0x94;

pub(crate) const BIKE_TECLA: usize = 0x96;

// Doador com o tamanho esperado: sem isso a classe ainda é forjada (a declaração do `.reds` precisa dela no bind), mas os getters devolvem o padrão
// e o `bwms_ink_key_event_novo` recusa.
pub(crate) static BIKE_LAYOUT_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe fn bike_campo<T: Copy + Default>(c: *mut c_void, off: usize) -> T {
    let p = (c as *const u8).wrapping_add(off);
    if c.is_null() || !BIKE_LAYOUT_OK.load(std::sync::atomic::Ordering::Relaxed) || !crate::memoria::is_readable(p as *const c_void, std::mem::size_of::<T>()) {
        return T::default();
    }
    (p as *const T).read_unaligned()
}

pub(crate) unsafe extern "C" fn tramp_bike_get_action(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, bike_campo::<u32>(c, BIKE_ACAO) as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_bike_get_character(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let ch = bike_campo::<u8>(c, BIKE_CARACTERE);
    write_cstring_inline_ret(out, &if ch == 0 { String::new() } else { (ch as char).to_string() });
}

pub(crate) unsafe extern "C" fn tramp_bike_is_character(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, bike_campo::<u8>(c, BIKE_CARACTERE) != 0);
}


// ===== Widget e controller de script ligados à camada da tela (2026-09-13) =====
// RE no Mac (Steam 2.31). `inkWidget::GetLayer` `0x10439c7e8` (widget, sret `Handle<inkLayerProxy>`: sem `layerProxy` em `+0x148` sobe pelos pais e grava
// o achado) e `inkWidget::SetLayer` `0x1043a0288` (widget, `&Handle`). `inkLayer::AttachWidgetTree` `0x1042d3a38` (camada, `&Handle` do widget, `&Handle`
// do proxy): percorre a árvore pelo slot `+0x118` do widget (container dos filhos; `+0xe0` quantos, `+0xe8` filho por índice), faz `SetLayer` em cada um,
// grava o widget nos controllers de cada um (`0x1041a9978`, `+0x40`) e registra cada controller no proxy (`0x1043a0104`); no fim grava o `layerProxy` deles
// (`0x1041a9aec`, `+0x50`, a partir do `+0x70` da camada) e chama o slot `+0x110`. `inkLayer::RegisterListener` `0x1042d4728` (camada, `&Handle`; trava de
// byte em `+0x130`, conjunto em `+0xd0`) é o que o `0x1042dd554` do proxy chama com a camada do `WeakHandle` em `+0x38` (a outra classe, em `+0x48`, tem
// `AttachWidgetTree` `0x1042ceba4` e registro `0x1042cd324`). Vtable do `inkWidgetLogicController` (vptr `0x107c7b8d8`, pelos
// fixups): `OnInitialize` `0x1041abc10` em `+0x110`, `OnUninitialize` `0x1041abd94` em `+0x118` (o `0x10439ba20` percorre a árvore chamando esse),
// `OnArrangeChildrenComplete` `0x1041abea4` em `+0x120`. Handlers de script: `inkWidget.Reparent` `0x1043a7644`, `inkCompoundWidget.AddChildWidget`
// `0x104393324`.
pub(crate) const C175_GET_LAYER_VM: u64 = 0x1_0439_c7e8;

pub(crate) const C175_SET_LAYER_VM: u64 = 0x1_043a_0288;

pub(crate) const C175_ATTACH_TREE_VM: u64 = 0x1_042d_3a38;

pub(crate) const C175_REGISTER_LISTENER_VM: u64 = 0x1_042d_4728;

pub(crate) const C175_CONTROLLER_WIDGET_VM: u64 = 0x1_041a_9978;

pub(crate) const C175_CONTROLLER_PROXY_VM: u64 = 0x1_041a_9aec;

pub(crate) const C175_REPARENT_VM: u64 = 0x1_043a_7644;

// 2.5: depois do reparent, com o byte `0x109d03e64` em 1, o handler chama `0x1041543c4`, que anexa a árvore ao layer do motor quando o `+0x148` é nulo; o
// hook roda depois e só anexa com o `+0x148` ainda nulo. O mesmo byte desvia o `OnInitialize` (`0x1041abbf4`) pro filtro de `detachedWidget`/`gameController`
// do motor, que, sem as duas props na classe, cai no `0x1041abc10` hookado.
pub(crate) const C175_REPARENT_PROLOGO: u32 = 0xd101_c3ff;

pub(crate) const C175_ADD_CHILD_WIDGET_VM: u64 = 0x1_0439_3324;

pub(crate) const C175_ADD_CHILD_WIDGET_PROLOGO: u32 = 0xd101_03ff;

pub(crate) const C175_ON_INITIALIZE_VM: u64 = 0x1_041a_bc10;

pub(crate) const C175_ON_INITIALIZE_PROLOGO: u32 = 0xd101_43ff;

pub(crate) const C175_WIDGET_SECUNDARIOS: usize = 0x108;

pub(crate) const C175_WIDGET_CONTROLLER: usize = 0x128;

pub(crate) const C175_WIDGET_PROXY: usize = 0x148;

pub(crate) const C175_PROXY_CAMADA: usize = 0x38;

pub(crate) const C175_CONTROLLER_PROXY: usize = 0x50;

pub(crate) const C175_VT_FILHOS: usize = 0x118;

pub(crate) const C175_VT_QUANTOS: usize = 0xe0;

pub(crate) const C175_VT_FILHO: usize = 0xe8;

pub(crate) const C175_VT_ON_INITIALIZE: usize = 0x110;

pub(crate) static ORIG_C175_REPARENT: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_C175_ADD_CHILD_WIDGET: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_C175_ON_INITIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) type C175HandlerFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i64);

/// Forte travado de um `WeakHandle` `{instância, bloco}` em `base+off`: +1 no forte por CAS, como o motor; `None` nulo ou morto.
pub(crate) unsafe fn c175_travar(base: *const u8, off: usize) -> Option<[u64; 2]> {
    if base.is_null() || !crate::memoria::is_readable(base.wrapping_add(off) as *const c_void, 16) {
        return None;
    }
    let inst = (base.add(off) as *const u64).read_unaligned();
    let bloco = (base.add(off + 8) as *const u64).read_unaligned();
    if inst == 0 || bloco == 0 || !crate::memoria::is_readable(bloco as *const c_void, 8) {
        return None;
    }
    let forte = &*(bloco as *const std::sync::atomic::AtomicU32);
    let mut s = forte.load(Ordering::Relaxed);
    while s != 0 {
        match forte.compare_exchange_weak(s, s + 1, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Some([inst, bloco]),
            Err(atual) => s = atual,
        }
    }
    None
}

/// `u64` em `p+off`; `u64::MAX` ilegível.
pub(crate) unsafe fn c175_campo(p: *const u8, off: usize) -> u64 {
    if p.is_null() || !crate::memoria::is_readable(p.wrapping_add(off) as *const c_void, 8) {
        return u64::MAX;
    }
    (p.add(off) as *const u64).read_unaligned()
}

/// Função da vtable de `obj` no deslocamento `off`.
pub(crate) unsafe fn c175_slot(obj: *const u8, off: usize) -> usize {
    (obj as *const *const usize).read_unaligned().add(off / 8).read_unaligned()
}

type C175GetLayerFn = unsafe extern "C" fn(*mut u8);
type C175SetLayerFn = unsafe extern "C" fn(*mut u8, *const u64);
type C175AttachTreeFn = unsafe extern "C" fn(u64, *const u64, *const u64);
type C175RegisterListenerFn = unsafe extern "C" fn(u64, *const u64);
type C175GravarNoControllerFn = unsafe extern "C" fn(*mut u8, *const u64);
type C175OnInitializeFn = unsafe extern "C" fn(*mut u8);

/// `RegisterListener(camada, &controller)`, soltura da camada travada e o `OnInitialize` do controller pela vtable (`+0x110`): o que o motor faz com
/// cada controller da árvore.
unsafe fn c175_registrar_e_inicializar(mut camada: [u64; 2], handle_do_controller: *const u64, controller: *mut u8) {
    let registrar: C175RegisterListenerFn = std::mem::transmute(crate::rebase(C175_REGISTER_LISTENER_VM));
    registrar(camada[0], handle_do_controller);
    crate::jogo::objetos::soltar_handle(&mut camada);
    let on_initialize: C175OnInitializeFn = std::mem::transmute(c175_slot(controller, C175_VT_ON_INITIALIZE));
    on_initialize(controller);
}

/// Liga a árvore do `widget` à camada da tela.
pub(crate) unsafe fn c175_anexar_arvore(widget: *mut u8) -> bool {
    match c175_campo(widget, C175_WIDGET_PROXY) {
        u64::MAX => return false,
        0 => {}
        _ => return true,
    }
    let mut proxy = [0u64; 2];
    let get_layer: C175GetLayerFn = std::mem::transmute(crate::rebase(C175_GET_LAYER_VM));
    crate::jogo::chamada::sret_chamar(get_layer as *mut c_void, [widget as u64, 0, 0, 0], proxy.as_mut_ptr() as *mut c_void);
    let Some(mut camada) = c175_travar(proxy[0] as *const u8, C175_PROXY_CAMADA) else {
        crate::jogo::objetos::soltar_handle(&mut proxy);
        return false;
    };
    let mut h = handle_forte_de(widget);
    let tinha_forte = h[0] != 0 && h[1] != 0;
    if tinha_forte {
        let set_layer: C175SetLayerFn = std::mem::transmute(crate::rebase(C175_SET_LAYER_VM));
        let attach_tree: C175AttachTreeFn = std::mem::transmute(crate::rebase(C175_ATTACH_TREE_VM));
        set_layer(widget, proxy.as_ptr());
        attach_tree(camada[0], h.as_ptr(), proxy.as_ptr());
    }
    crate::jogo::objetos::soltar_handle(&mut h);
    crate::jogo::objetos::soltar_handle(&mut camada);
    crate::jogo::objetos::soltar_handle(&mut proxy);
    tinha_forte
}

/// Guarda `referencia` (forte, já contada) no fim dos secundários do widget; sem vaga, solta.
unsafe fn c175_guardar_secundario(widget: *mut u8, mut referencia: [u64; 2]) {
    let lista = widget.add(C175_WIDGET_SECUNDARIOS);
    if !crate::memoria::is_readable(lista as *const c_void, 0x10) {
        crate::jogo::objetos::soltar_handle(&mut referencia);
        return;
    }
    let (_, n, cap) = crate::jogo::listas::dynarray_no_campo(lista, 0);
    if n >= cap {
        // Boot 184: a lista nasce vazia sem alocador no fim, e só o reserve do motor grava o que a soltura dele espera ali.
        let mover = if n == 0 { std::ptr::null() } else { crate::rebase(MOVER_HANDLES_VM) as *const c_void };
        crate::jogo::listas::reservar(lista as *mut c_void, n as u32 + 1, 16, 8, mover);
    }
    let (entradas, n, cap) = crate::jogo::listas::dynarray_no_campo(lista, 0);
    if entradas.is_null() || n >= cap {
        crate::jogo::objetos::soltar_handle(&mut referencia);
        return;
    }
    (entradas.add(n * 16) as *mut [u64; 2]).write_unaligned(referencia);
    (lista.add(0x0c) as *mut u32).write_unaligned(n as u32 + 1);
}

/// `inkWidget.AttachController` de mod: o controller vira o do widget (ou secundário) e é ligado à camada.
pub(crate) unsafe fn c175_ligar_controller(widget: *mut u8, controller: *mut u8, secundario: bool) -> bool {
    if c175_campo(widget, C175_WIDGET_PROXY) == u64::MAX {
        return false;
    }
    let proxy_do_controller = c175_campo(controller, C175_CONTROLLER_PROXY);
    if proxy_do_controller == u64::MAX {
        return false;
    }
    let mut hc = handle_forte_de(controller);
    if hc[0] == 0 || hc[1] == 0 {
        crate::jogo::objetos::soltar_handle(&mut hc);
        return false;
    }
    // A referência do widget é dele: +1 no forte, solta pelo motor junto com o widget.
    (*(hc[1] as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
    if secundario {
        c175_guardar_secundario(widget, hc);
    } else {
        let campo = widget.add(C175_WIDGET_CONTROLLER) as *mut [u64; 2];
        let mut antigo = campo.read_unaligned();
        campo.write_unaligned(hc);
        crate::jogo::objetos::soltar_handle(&mut antigo);
    }
    let ligado = c175_ligar_a_camada(widget, controller, &hc, proxy_do_controller);
    crate::jogo::objetos::soltar_handle(&mut hc);
    ligado
}

/// Widget e proxy gravados no controller, registro na camada e `OnInitialize`; `true` também com o controller já ligado antes.
unsafe fn c175_ligar_a_camada(widget: *mut u8, controller: *mut u8, hc: &[u64; 2], proxy_do_controller: u64) -> bool {
    if proxy_do_controller != 0 {
        return true;
    }
    let mut hw = handle_forte_de(widget);
    if hw[0] != 0 {
        let gravar_widget: C175GravarNoControllerFn = std::mem::transmute(crate::rebase(C175_CONTROLLER_WIDGET_VM));
        gravar_widget(controller, hw.as_ptr());
    }
    crate::jogo::objetos::soltar_handle(&mut hw);
    let proxy = c175_campo(widget, C175_WIDGET_PROXY);
    if proxy == 0 || proxy == u64::MAX {
        return false;
    }
    let gravar_proxy: C175GravarNoControllerFn = std::mem::transmute(crate::rebase(C175_CONTROLLER_PROXY_VM));
    gravar_proxy(controller, widget.add(C175_WIDGET_PROXY) as *const u64);
    let Some(camada) = c175_travar(proxy as *const u8, C175_PROXY_CAMADA) else {
        return false;
    };
    c175_registrar_e_inicializar(camada, hc.as_ptr(), controller);
    true
}

// Eventos de controller ink de script na 2.5: o `AttachController` nativo do motor (`0x104154158`, só chamado pelo
// `inkWidget.AttachController` `0x1043a399c`; x0 `&Handle<inkWidget>`, x1 `&Handle<inkLogicController>`, devolve bool) grava widget e proxy no
// controller e trava a camada em `proxy+0x48`, morta nos widgets do HUD (lane I de 2026-10-28, `BwmsC121Medir` = 0b11101: `+0x48` morta, `+0x38` viva):
// sai com 0 sem registrar nem chamar o `OnInitialize`.
pub(crate) const ANEXAR_CONTROLLER_NATIVO_VM: u64 = 0x1_0415_4158;

pub(crate) const ANEXAR_CONTROLLER_NATIVO_PROLOGO: u32 = 0xd101_83ff;

pub(crate) static ORIG_ANEXAR_CONTROLLER: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static CONTROLLERS_COMPLETADOS: AtomicU64 = AtomicU64::new(0);

/// Gancho do `AttachController` nativo (`ANEXAR_CONTROLLER_NATIVO_VM`).
pub(crate) unsafe extern "C" fn anexar_controller_hook(widget: *const u64, controller: *const u64) -> u64 {
    type AnexarControllerFn = unsafe extern "C" fn(*const u64, *const u64) -> u64;
    let original: AnexarControllerFn = std::mem::transmute(original_publicado(&ORIG_ANEXAR_CONTROLLER));
    let r = original(widget, controller);
    if r & 1 != 0 || controller.is_null() || !crate::memoria::is_readable(controller as *const c_void, 16) {
        return r;
    }
    let obj = controller.read_unaligned() as *mut u8;
    // Proxy 0: widget fora de camada; a árvore liga o controller quando entrar numa.
    let proxy = c175_campo(obj, C175_CONTROLLER_PROXY);
    if proxy == 0 || proxy == u64::MAX {
        return r;
    }
    let Some(camada) = c175_travar(proxy as *const u8, C175_PROXY_CAMADA) else {
        return r;
    };
    c175_registrar_e_inicializar(camada, controller, obj);
    #[cfg(feature = "devtools")]
    CONTROLLERS_COMPLETADOS.fetch_add(1, Ordering::Relaxed);
    1
}

/// No `on_load`, sempre: só o `inkWidget.AttachController` chama o `0x104154158`.
pub(crate) unsafe fn install_anexar_controller() {
    let r = instalar_conferindo_prologo(ANEXAR_CONTROLLER_NATIVO_VM, ANEXAR_CONTROLLER_NATIVO_PROLOGO, anexar_controller_hook as *mut c_void, &ORIG_ANEXAR_CONTROLLER);
    crate::veredito_dev!("[c121] hook do AttachController nativo: {r:?}");
}

/// Gancho do handler de script `inkWidget.Reparent`.
pub(crate) unsafe extern "C" fn c175_depois_do_reparent(widget: *mut c_void, frame: *mut c_void, out: *mut c_void, rt: i64) {
    let original: C175HandlerFn = std::mem::transmute(original_publicado(&ORIG_C175_REPARENT));
    original(widget, frame, out, rt);
    let w = widget as *mut u8;
    if c175_campo(w, C175_WIDGET_PROXY) == 0 && c175_anexar_arvore(w) {
        C175_ANEXADAS[0].fetch_add(1, Ordering::Relaxed);
    }
}

/// Último filho do widget composto; `None` sem filhos.
pub(crate) unsafe fn c175_ultimo_filho(pai: *const u8) -> Option<*mut u8> {
    type FilhosFn = unsafe extern "C" fn(*const u8) -> *const u8;
    type QuantosFn = unsafe extern "C" fn(*const u8) -> u32;
    type FilhoFn = unsafe extern "C" fn(*const u8, u32) -> *mut u8;
    if pai.is_null() {
        return None;
    }
    let filhos: FilhosFn = std::mem::transmute(c175_slot(pai, C175_VT_FILHOS));
    let container = filhos(pai);
    if container.is_null() {
        return None;
    }
    let quantos: QuantosFn = std::mem::transmute(c175_slot(container, C175_VT_QUANTOS));
    let n = quantos(container);
    if n == 0 {
        return None;
    }
    let filho: FilhoFn = std::mem::transmute(c175_slot(container, C175_VT_FILHO));
    let f = filho(container, n - 1);
    (!f.is_null()).then_some(f)
}

/// Gancho do handler de script `inkCompoundWidget.AddChildWidget`.
pub(crate) unsafe extern "C" fn c175_depois_do_add_child_widget(pai: *mut c_void, frame: *mut c_void, out: *mut c_void, rt: i64) {
    let original: C175HandlerFn = std::mem::transmute(original_publicado(&ORIG_C175_ADD_CHILD_WIDGET));
    original(pai, frame, out, rt);
    let proxy_do_pai = c175_campo(pai as *const u8, C175_WIDGET_PROXY);
    if proxy_do_pai == 0 || proxy_do_pai == u64::MAX {
        return;
    }
    if let Some(filho) = c175_ultimo_filho(pai as *const u8) {
        if c175_campo(filho, C175_WIDGET_PROXY) == 0 && c175_anexar_arvore(filho) {
            C175_ANEXADAS[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Resposta de `c175_classe_legada` por classe (`CClass*`), negativa inclusive: a resposta só depende das props da classe e dos pais, e o único
/// escritor de props de classe em runtime é `tipos_do_motor::propriedade_nativa`, que limpa o mapa (o teste
/// `resposta_fica_por_classe_ate_prop_nova` lê as chaves).
static C175_CLASSES_LEGADAS: Mutex<Option<std::collections::HashMap<usize, bool>>> = Mutex::new(None);

pub(crate) fn c175_esquecer_classes_legadas() {
    *C175_CLASSES_LEGADAS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Controller de script legado pela classe dele.
pub(crate) unsafe fn c175_controller_legado(controller: *mut u8) -> bool {
    c175_classe_legada(rtti::class_of(controller as *mut c_void))
}

/// Classe de controller legado (guardada por classe em [`C175_CLASSES_LEGADAS`]).
pub(crate) unsafe fn c175_classe_legada(cls: *mut c_void) -> bool {
    if cls.is_null() {
        return false;
    }
    let mut guardadas = C175_CLASSES_LEGADAS.lock().unwrap_or_else(|e| e.into_inner());
    let mapa = guardadas.get_or_insert_with(std::collections::HashMap::new);
    if let Some(&legada) = mapa.get(&(cls as usize)) {
        return legada;
    }
    // O controller antigo com `detachedWidget` e `gameController` se inicializa sozinho; o `OnInitialize` da árvore seria o 2º.
    let legada =
        !rtti::find_property_in_class(cls, "detachedWidget").is_null() && !rtti::find_property_in_class(cls, "gameController").is_null();
    mapa.insert(cls as usize, legada);
    legada
}

/// Gancho do `OnInitialize` do `inkWidgetLogicController` (slot `+0x110`).
pub(crate) unsafe extern "C" fn c175_on_initialize(controller: *mut u8) {
    if !controller.is_null() && c175_controller_legado(controller) {
        C175_LEGADOS_PULADOS.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let original: unsafe extern "C" fn(*mut u8) = std::mem::transmute(original_publicado(&ORIG_C175_ON_INITIALIZE));
    original(controller);
}

#[cfg(test)]
mod c175_classes_legadas_tests {
    use super::{c175_classe_legada, c175_esquecer_classes_legadas, C175_CLASSES_LEGADAS};
    use crate::cname::cname;
    use std::ffi::c_void;

    /// Classe falsa com o layout que `props_array` lê: props em `+0x28` (capacidade `+0x30`, tamanho `+0x34`), pai em `+0x10`;
    /// cada prop com o CName em `+0x08` e o offset do valor em `+0x20`.
    struct ClasseFalsa {
        classe: Box<[u64; 16]>,
        props: Box<[u64; 2]>,
        nomes: Box<[[u64; 6]; 2]>,
    }

    impl ClasseFalsa {
        fn nova(nomes: &[&str], pai: *mut c_void) -> Self {
            let mut c = ClasseFalsa { classe: Box::new([0; 16]), props: Box::new([0; 2]), nomes: Box::new([[0; 6]; 2]) };
            for (i, nome) in nomes.iter().enumerate() {
                c.nomes[i][1] = cname(nome);
                c.props[i] = c.nomes[i].as_ptr() as u64;
            }
            c.classe[0x10 / 8] = pai as u64;
            c.classe[0x28 / 8] = c.props.as_ptr() as u64;
            c.declarar(nomes.len() as u64);
            c
        }

        /// Quantas das props montadas a classe declara: 0 apaga a lista, voltar a 1 é o push de uma prop nova.
        fn declarar(&mut self, quantas: u64) {
            self.classe[0x30 / 8] = (quantas << 32) | 2;
        }

        fn cls(&mut self) -> *mut c_void {
            self.classe.as_mut_ptr() as *mut c_void
        }
    }

    // Um teste só: o mapa é global, e um `c175_esquecer_classes_legadas` em paralelo apagaria a resposta que este confere no mapa.
    #[test]
    fn resposta_fica_por_classe_ate_prop_nova() {
        let mut legada = ClasseFalsa::nova(&["detachedWidget", "gameController"], std::ptr::null_mut());
        let mut pai = ClasseFalsa::nova(&["detachedWidget"], std::ptr::null_mut());
        pai.declarar(0);
        let mut filha = ClasseFalsa::nova(&["gameController"], pai.cls());
        let (legada_cls, filha_cls) = (legada.cls(), filha.cls());
        unsafe {
            assert!(!c175_classe_legada(std::ptr::null_mut()));
            assert!(c175_classe_legada(legada_cls));
            assert!(!c175_classe_legada(filha_cls), "controller comum: a busca anda até a raiz e não acha detachedWidget");
            // Sem as props na classe, só o mapa ainda diz que ela é legada: a 2ª chamada não passa pela cadeia.
            legada.declarar(0);
            assert!(c175_classe_legada(legada_cls));
            // Prop nova no pai muda a resposta da filha guardada; quem limpa o mapa é `propriedade_nativa`, logo depois do push.
            pai.declarar(1);
            assert!(!c175_classe_legada(filha_cls), "a resposta negativa também fica guardada");
            c175_esquecer_classes_legadas();
            assert!(c175_classe_legada(filha_cls));
            assert!(!c175_classe_legada(legada_cls));
            assert!(!c175_classe_legada(std::ptr::null_mut()));
        }
        let guardadas = C175_CLASSES_LEGADAS.lock().unwrap_or_else(|e| e.into_inner());
        let mut chaves: Vec<usize> = guardadas.as_ref().map(|m| m.keys().copied().collect()).unwrap_or_default();
        chaves.sort_unstable();
        let mut esperadas = vec![legada_cls as usize, filha_cls as usize];
        esperadas.sort_unstable();
        assert_eq!(chaves, esperadas, "classe nula nunca entra no mapa");
    }
}

/// No `on_load`: os ganchos da ligação de widget e controller de mod à camada.
pub(crate) unsafe fn install_c175_widget_building() {
    let no_bundle = crate::dylib_dir()
        .is_some_and(|d| std::path::Path::new(&d).join("../r6/scripts/blackwall-mods/bwms-widget-building.reds").exists());
    if !no_bundle && !marcador_individual("c175-teste") {
        return;
    }
    let reparent = instalar_conferindo_prologo(C175_REPARENT_VM, C175_REPARENT_PROLOGO, c175_depois_do_reparent as *mut c_void, &ORIG_C175_REPARENT);
    let filho = instalar_conferindo_prologo(
        C175_ADD_CHILD_WIDGET_VM,
        C175_ADD_CHILD_WIDGET_PROLOGO,
        c175_depois_do_add_child_widget as *mut c_void,
        &ORIG_C175_ADD_CHILD_WIDGET,
    );
    let iniciar = instalar_conferindo_prologo(C175_ON_INITIALIZE_VM, C175_ON_INITIALIZE_PROLOGO, c175_on_initialize as *mut c_void, &ORIG_C175_ON_INITIALIZE);
    let ativo = ![reparent, filho, iniciar].contains(&Some(false));
    C175_ATIVO.store(ativo, Ordering::Relaxed);
    crate::veredito_dev!(
        "[c175] ganchos da camada: Reparent {reparent:?}, AddChildWidget {filho:?}, OnInitialize {iniciar:?}; reds no bundle={no_bundle}; ativo={ativo}"
    );
}






// ===== `resource.patch` de `.inkanim` pelo `OnDependenciesReady` do serializer =====
// O patch troca as `sequences` da biblioteca alvo pelas do patch com o mesmo `name` e
// acrescenta as que o alvo não tem; nada mais é mesclado. O gancho é o da 51g (`arquivos_do_jogo::dependencias_prontas_hook`). `sequences` é
// `DynArray<Handle<inkanimSequence>>` (16 B por handle); os offsets dela e do `name` da sequência saem do RTTI na carga da tabela. A sequência que entra
// ganha +1 no contador forte (o patch continua dono da dele); a que sai fica com a referência que tinha.
pub(crate) static MESCLA_DE_INKANIM_ATIVA: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub(crate) static CLASSE_INKANIM_BIBLIOTECA: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());
static INKANIM_SEQUENCIAS_OFF: AtomicU64 = AtomicU64::new(0);
static INKANIM_NOME_OFF: AtomicU64 = AtomicU64::new(0);
pub(crate) static INKANIM_TROCADAS: AtomicU64 = AtomicU64::new(0);
pub(crate) static INKANIM_ACRESCENTADAS: AtomicU64 = AtomicU64::new(0);

/// Classe da biblioteca e offsets de `sequences` e de `inkanimSequence.name` pelo RTTI (thread do jogo, na carga da tabela). `true` com a mescla ligável.
pub(crate) unsafe fn guardar_offsets_do_inkanim(reg: &Registry) -> bool {
    let cls = reg.class_by_name("inkanimAnimationLibraryResource");
    let seq = reg.class_by_name("inkanimSequence");
    let sequencias = if cls.is_null() { None } else { rtti::resolve_prop_in_class(cls, "sequences").map(|(o, _, _)| o as u64) };
    let nome = if seq.is_null() { None } else { rtti::resolve_prop_in_class(seq, "name").map(|(o, _, _)| o as u64) };
    let ok = sequencias.is_some() && nome.is_some();
    if let (Some(s), Some(n)) = (sequencias, nome) {
        CLASSE_INKANIM_BIBLIOTECA.store(cls, Ordering::Relaxed);
        INKANIM_SEQUENCIAS_OFF.store(s, Ordering::Relaxed);
        INKANIM_NOME_OFF.store(n, Ordering::Relaxed);
    }
    crate::veredito_dev!(
        "[inkanim] biblioteca pelo RTTI: classe {cls:p}, sequences {sequencias:x?}, inkanimSequence.name {nome:x?}; mescla {}",
        if ok { "ligada" } else { "desligada" }
    );
    ok
}

/// Para cada sequência do patch (pelo nome), o índice da do alvo com o mesmo nome, ou `None` para acrescentar no fim.
pub(crate) fn plano_inkanim(alvo: &[u64], patch: &[u64]) -> Vec<Option<usize>> {
    patch.iter().map(|n| alvo.iter().position(|a| *n != 0 && a == n)).collect()
}

/// Entradas de `sequences` e o `name` de cada sequência (0 se ilegível); `None` com o array ilegível.
unsafe fn sequencias_do_inkanim(res: u64) -> Option<(*mut u8, Vec<u64>)> {
    let so = INKANIM_SEQUENCIAS_OFF.load(Ordering::Relaxed) as usize;
    if res == 0 || so == 0 || !crate::memoria::is_readable(res as *const c_void, so + 0x10) {
        return None;
    }
    let (b, n, _) = crate::jogo::listas::dynarray_no_campo(res as *const u8, so);
    if n > 4096 || (n > 0 && !crate::memoria::is_readable(b as *const c_void, n * 16)) {
        return None;
    }
    let off = INKANIM_NOME_OFF.load(Ordering::Relaxed) as usize;
    let nomes = (0..n)
        .map(|i| {
            let obj = (b.add(i * 16) as *const u64).read_unaligned();
            if obj != 0 && crate::memoria::is_readable((obj as usize + off) as *const c_void, 8) { ((obj as usize + off) as *const u64).read_unaligned() } else { 0 }
        })
        .collect();
    Some((b, nomes))
}

/// No `OnDependenciesReady` (thread de job): sequências de cada patch da tabela cujo alvo é esta biblioteca.
pub(crate) unsafe fn mesclar_inkanim(alvo: u64) {
    let caminho = crate::jogo::recursos::hash_do_caminho_do_recurso(alvo);
    let patches: Vec<[u64; 2]> = match crate::arquivos_do_jogo::PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo(caminho).map(|p| p.token).collect(),
        Err(_) => return,
    };
    let classe = CLASSE_INKANIM_BIBLIOTECA.load(Ordering::Relaxed);
    let arr = (alvo as *mut u8).add(INKANIM_SEQUENCIAS_OFF.load(Ordering::Relaxed) as usize) as *mut c_void;
    for pt in patches {
        let pres = if pt[0] == 0 { 0 } else { crate::arquivos_do_jogo::recurso_do_patch_da_classe(pt, INKANIM_SEQUENCIAS_OFF.load(Ordering::Relaxed) as usize + 0x10, classe) as u64 };
        let (Some((_, nomes_alvo)), Some((pb, nomes_patch))) = (sequencias_do_inkanim(alvo), sequencias_do_inkanim(pres)) else { continue };
        let plano = plano_inkanim(&nomes_alvo, &nomes_patch);
        #[cfg(feature = "devtools")]
        let (antes, trocadas_antes, acrescentadas_antes) =
            (nomes_alvo.len(), INKANIM_TROCADAS.load(Ordering::Relaxed), INKANIM_ACRESCENTADAS.load(Ordering::Relaxed));
        // Trocas antes dos acréscimos: o acréscimo pode realocar o array do alvo.
        for (j, passo) in plano.iter().enumerate() {
            let h = [(pb.add(j * 16) as *const u64).read_unaligned(), (pb.add(j * 16 + 8) as *const u64).read_unaligned()];
            if let (Some(i), true) = (passo, h[0] != 0) {
                let (tb, n, _) = crate::jogo::listas::dynarray_no_campo(arr as *const u8, 0);
                if *i < n {
                    std::ptr::copy_nonoverlapping(h.as_ptr() as *const u8, tb.add(i * 16), 16);
                    handle_strong_soma(&h, 1);
                    INKANIM_TROCADAS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        for (j, passo) in plano.iter().enumerate() {
            let h = [(pb.add(j * 16) as *const u64).read_unaligned(), (pb.add(j * 16 + 8) as *const u64).read_unaligned()];
            if passo.is_none() && h[0] != 0 {
                let bytes: [u8; 16] = std::mem::transmute(h);
                if dynarray_push_bytes(arr, &bytes).is_some() {
                    handle_strong_soma(&h, 1);
                    INKANIM_ACRESCENTADAS.fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        // Prova: a lista relida do alvo tem as do patch nas posições do plano (trocadas pelo nome, as novas no fim).
        #[cfg(feature = "devtools")]
        {
            let depois = sequencias_do_inkanim(alvo).map(|(_, n)| n).unwrap_or_default();
            let novas: Vec<u64> = plano.iter().zip(&nomes_patch).filter(|(p, _)| p.is_none()).map(|(_, n)| *n).collect();
            let fim_ok = depois.len() == antes + novas.len() && depois[antes..] == novas[..];
            let trocas_ok = plano.iter().zip(&nomes_patch).all(|(p, n)| p.map_or(true, |i| depois.get(i) == Some(n)));
            crate::veredito_dev!(
                "[inkanim] VEREDITO alvo {caminho:#018x}: {antes} sequência(s) antes, {} no patch; {} trocada(s), {} acrescentada(s), {} depois; novas no fim {fim_ok}, trocas pelo nome {trocas_ok}; inkanim-ok={}",
                nomes_patch.len(),
                INKANIM_TROCADAS.load(Ordering::Relaxed) - trocadas_antes,
                INKANIM_ACRESCENTADAS.load(Ordering::Relaxed) - acrescentadas_antes,
                depois.len(),
                fim_ok && trocas_ok
            );
        }
    }
}

#[cfg(test)]
mod testes_inkanim {
    use super::plano_inkanim;

    #[test]
    fn plano_troca_pelo_nome_e_acrescenta_o_resto() {
        assert_eq!(plano_inkanim(&[10, 20, 30], &[20, 40, 10]), vec![Some(1), None, Some(0)]);
        // Sequência sem nome (0) nunca casa com outra sem nome: entra no fim.
        assert_eq!(plano_inkanim(&[0, 5], &[0]), vec![None]);
        assert!(plano_inkanim(&[1], &[]).is_empty());
    }
}
