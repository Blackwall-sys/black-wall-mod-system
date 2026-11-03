//! Aparência das coisas no jogo (do personagem e dos NPCs): malhas, componentes, render proxies, controladores visuais, aparências dinâmicas e overrides de aparência.

#![allow(dead_code)]

pub(crate) mod do_record;
#[cfg(test)]
mod testes_da_mescla;

use crate::jogo::cstring::destroi_cstring_da_pilha;
use crate::mods_do_usuario::tramp_require;
use crate::mods_do_usuario::tramp_version;
use crate::mods_do_usuario::PLUGUE_RECURSOS;
use crate::jogo::objetos::destravar_fraco;
use crate::jogo::objetos::travar_fraco;
use crate::jogo::listas::MOVER_HANDLES_VM;
use crate::marcador_individual;
use crate::tweakdb::natives::a9_registro;
use crate::jogo::aparencia::{APPDEF_NAME_OFF, APPDEF_PARTSVALUES_OFF, APPDEF_VISUALTAGS_OFF};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use crate::ganchos::original_publicado;
use crate::ganchos::instalar_conferindo_prologo;
use crate::mundo::*;
use crate::ganchos::*;
use crate::diagnostico::{
    a11_alias, a11_anotar, a11_descritores, a11_pares_gravados, a17_classes, A11_PRE, A11_TESTE, A17_CHAMADAS, A17_OBSERVAR, A17_VISTAS, C130D_ANOTOU, C130D_ATUAL, C130D_CONGELADO, C130D_DESPACHOS, C130D_NO_BUILDER, C130D_SEM_ANOTACAO,
    C130D_SOMADO, C130D_SOMADOS, C130D_TESTE, C134E_CONGELADO, C134E_REDESPACHANDO, C134E_REDESPACHAR, C134E_TOGGLE_POR_HOOK,
};
// Só os handlers de prova usam, e o `corpo_de_prova!` público não os cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{tramp_component_state_restore, tramp_component_state_save, tramp_visual_tag_state_restore, tramp_visual_tag_state_save};
use crate::tipos_do_motor::*;
use crate::arquivos_do_jogo::*;
use crate::texto_e_numeros::*;
#[cfg(feature = "devtools")] use crate::camera::*; // aqui só os handlers de prova usam, e o `corpo_de_prova!` público não os cita
use crate::corpo_e_rosto::*;
use crate::roupas_e_armas::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::rtti::{self, Registry};
use crate::cname::CName;
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b010;
use crate::entidades::*;

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
/// Devolve o PEDAÇO `i` de `s` (19 bytes por pedaço), ou `""` quando acabou.
///
/// **Por que existe.** `write_cstring_inline_ret` recusa string de 20+ bytes — o buffer inline da
/// `RED::CString` não cabe — e os ~38 call-sites fazem `let _ = ...`, então o redscript recebia
/// `""` e lia isso como "não achei". Já custou duas investigações: o `classe=''` do `#86`
/// (`worldStaticMarkerNode`, 21 bytes) e o `resolvidos=0` do `A#49` (`worldStreamingSector`,
/// exatos 20). O caminho de HEAP da `CString` resolveria de vez, mas exige o alocador do MOTOR:
/// se o engine liberar com o alocador dele um ponteiro que eu aloquei com o meu, é crash — e a
/// forma de descobrir o discriminante inline-vs-heap é adivinhar layout, que este projeto já
/// aprendeu a não fazer ([[cp77-offset-lixo-plausivel]]).
///
/// Fatiar é seguro e não depende de layout nenhum: cada pedaço cabe folgado no inline, e quem
/// junta é o redscript. O corte é por CARACTERE (`chars()`), nunca por byte — cortar UTF-8 no meio
/// de um codepoint produziria string inválida.
pub(crate) unsafe fn write_cstring_chunk_ret(out: *mut c_void, s: &str, i: usize) -> bool {
    write_cstring_inline_ret(out, &pedaco_utf8(s, i))
}

/// `EntityTarget.Appearance(appearanceName: CName) -> ref<EntityTarget>` — achado 2026-07-24 (2ª
/// leva): apesar de listada junto de `Template`/`Definition` na rodada anterior, a assinatura REAL
/// (`EntityTarget.reds`) usa `CName`, NÃO `ResRef` — nunca precisou do whitelist novo, só tinha
/// ficado de fora por escopo explícito da rodada passada. Mesma receita de `tramp_et_type`.
pub(crate) unsafe extern "C" fn tramp_et_appearance(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let name = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "EntityTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix 2026-08-12: retenção forte p/ a cópia de vida longa (ver doc da fn)
        if let Ok(mut states) = ENTITYTARGET_STATES.lock() {
            states.push((obj as usize, EntityTargetCriterion::Appearance, name));
        }
    } else {
        crate::log_dev!("[et] EntityTarget.Appearance: new_object('EntityTarget') falhou");
    }
    crate::log_dev!(
        "[et] EntityTarget.Appearance('{}' hash {name:#018x}) -> {obj:p} (fábrica estática)",
        crate::cname::resolve_cname(name)
    );
    rtti::make_handle(out, obj);
}

pub(crate) unsafe extern "C" fn tramp_ebt_get_appearance_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_REQUEST), EBR_APARENCIA)).unwrap_or(0), 8);
}

/// `EntityBuilderAppearanceWrapper` (pai `IScriptable`), sobre uma `EntityBuilderAppearance` do builder.
pub unsafe fn register_ebw_appearance(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) || register_type_instantiable_with_parent(reg, "EntityBuilderAppearanceWrapper", "gameuiInGameMenuGameController", iscriptable).is_null() {
        return "[reg] register_ebw_appearance: forja da classe FALHOU".into();
    }
    let Some(proto) = method_donor_proto(reg) else {
        return "[reg] register_ebw_appearance: classe=OK mas sem protótipo".into();
    };
    let c = "EntityBuilderAppearanceWrapper";
    let r = [
        register_method(reg, c, proto, "GetResourcePath", "GetResourcePath", tramp_eba_get_resource_path, false),
        register_method(reg, c, proto, "GetDefinitionName", "GetDefinitionName", tramp_eba_get_definition_name, false),
        register_method(reg, c, proto, "GetComponents", "GetComponents", tramp_eba_get_components, false),
        register_method(reg, c, proto, "AddComponent", "AddComponent", tramp_eba_add_component, false),
    ];
    format!("[reg] EntityBuilderAppearanceWrapper: classe=OK métodos {r:?}")
}

// ===== `entVisualControllerComponent`
// ("possível pista pro full-body"): probes observe-only via a técnica NOVA descoberta
// 2026-08-12 — minerar `cp77-symbols/symbols-demangled.txt` (68654 símbolos, binário macOS
// NÃO-stripado) pelo padrão `rtti::RegisterEventConnector<ClasseX, EventoY>`: o compilador emite um
// símbolo demangled PRÓPRIO pro thunk `__invoke(void* closureData, ISerializable& receiver,
// THandle<red::Event> const& event)` de cada par (classe, evento) que uma classe do motor escuta via
// esse mecanismo do RTTI — dá um endereço CONFIRMADO POR SÍMBOLO do compilador, não um candidato
// heurístico de disassembly/padrão-de-bytes.
//
// `grep "RegisterEventConnector<ent::VisualControllerComponent" symbols-demangled.txt` achou
// EXATAMENTE 2 eventos reais que esta classe escuta:
//   - `ent::AppearanceEvent`                       → __invoke @ 0x1019a050c
//   - `ent::AppearanceLODsDistanceOverrideEvent`    → __invoke @ 0x1019a0544
//
// **Por que isso é um candidato NOVO e não uma repetição da família já "razoavelmente esgotada"
// da saga full-body:** as sessões de 2026-08-06/07 (ver `notes/GARMENT-APPEARANCE-COMPONENT-
// FACTS.md` §41, `HISTORICO.md` cont.39-41) investigaram exaustivamente a família
// `RegisterEventConnector<ent::SkinnedMeshComponent, ...>` (DissolveFinish/MeshLoaded/StatusEvent)
// — 3 candidatos testados com rigor crescente, todos convergindo pra "não é este mecanismo" (o
// torso/mesh existe, `enabled=0`, mas o consumidor que aplicaria o 2º gate de visibilidade nunca
// foi achado NESSA família). `ent::VisualControllerComponent` é uma classe ARQUITETURALMENTE
// DIFERENTE — não é o componente de mesh em si, é o gerenciador de DEPENDÊNCIAS VISUAIS que outros
// componentes referenciam (`VisualController::LoadDependencies` + `VisualControllerComponent::Controller@0xD8`,
// ponteiro pra uma struct interna SEPARADA `VisualController{dependencies[],looseDependencies[],
// job,status,lock}@0x80`, offsets confirmados, item `#213` ✅). NUNCA
// cogitada como candidata ao "2º gate" hipotetizado na saga do full-body — groundwork novo, não
// retestе do que já foi refutado.
//
// **Decisão de escopo:** hookar o `__invoke` CONFIRMADO (não o handler real por trás dele — a
// mesma limitação já documentada nos probes irmãos de hoje se aplica: o ponteiro-membro do handler
// não é resolvível por ADRP+ADD estático no nível do `__invoke`, é despachado via indireção;
// resolver isso é RE adicional fora do escopo desta rodada 100% offline). O `receiver` (x1) é uma
// APROXIMAÇÃO de `ent::VisualControllerComponent*` (sem aplicar o ajuste de `this` do thunk —
// pode divergir por um pequeno offset em caso de herança múltipla, mesma ressalva já aceita nos
// probes TPP/AttachmentSlots de hoje). Dump lê `receiver+0xD8` (offset de CAMPO, categoria de
// confiança mais alta que endereço de função) e, se plausível/legível, os primeiros 0x90 bytes da
// struct `VisualController` apontada — groundwork de RE pura, sem hipótese fechada ainda sobre o
// que cada campo contém no runtime real desta build Mac.
//
// 100% OBSERVE-ONLY: sempre chama a original ANTES de logar, nunca muta nada, nunca influi no
// retorno (a função é `void`). CODADO, NUNCA TESTADO AO VIVO (sessão 100% offline, zero boot
// disponível). Gate PRÓPRIO por evento (`~/.bwms-hook-visualcontroller-appearance-invoke` /
// `~/.bwms-hook-visualcontroller-lods-invoke` / `~/.bwms-hook` liga tudo), OFF por padrão — mesma
// convenção de marcador já usada nos ~30 probes de `install_postload_hooks()` (`selftest.rs`),
// reimplementada aqui localmente (função própria, não a macro `install_hook_probe!`, que vive em
// `selftest.rs` e não é importável textualmente por `register.rs` nesta ordem de módulos) pra não
// tocar naquele arquivo.
pub(crate) type VcEventInvokeFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

pub(crate) const VC_APPEARANCE_INVOKE_VM: u64 = 0x1_019a_050c;

pub(crate) static VC_APPEARANCE_INVOKE_CNT: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_VC_APPEARANCE_INVOKE: AtomicU64 = AtomicU64::new(0);

pub(crate) const VC_LODS_INVOKE_VM: u64 = 0x1_019a_0544;

pub(crate) static VC_LODS_INVOKE_CNT: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_VC_LODS_INVOKE: AtomicU64 = AtomicU64::new(0);

/// Dump comum aos 2 probes: `receiver+0xD8` (Controller ptr, offset de campo confirmado) + até
/// 0x90 bytes da struct `VisualController` apontada, se o ponteiro parecer plausível/legível.
pub(crate) unsafe fn dump_visualcontroller_via_receiver(receiver: *mut c_void) -> String {
    if receiver.is_null() || !crate::memoria::is_readable(receiver, 0xE0) {
        return "receiver ilegível (<0xE0 bytes)".to_string();
    }
    let controller_ptr = (receiver as *const u64).add(0xD8 / 8).read_unaligned();
    if controller_ptr == 0 || !crate::memoria::is_readable(controller_ptr as *const c_void, 0x90) {
        return format!("Controller@0xD8={controller_ptr:#018x} (nulo ou ilegível)");
    }
    let mut words = [0u64; 18]; // 0x90 bytes / 8
    for (i, w) in words.iter_mut().enumerate() {
        *w = (controller_ptr as *const u64).add(i).read_unaligned();
    }
    format!("Controller@0xD8={controller_ptr:#018x} VisualController[0..0x90)={words:#018x?}")
}

pub(crate) unsafe extern "C" fn vc_appearance_invoke_probe(closure: *mut c_void, receiver: *mut c_void, evt_ref: *mut c_void) {
    let n = VC_APPEARANCE_INVOKE_CNT.fetch_add(1, Ordering::Relaxed);
    if n == 0 {
        crate::log_dev!("[cw-visualcontroller] game_base={:#018x}", crate::game_base());
    }
    if n < 8 {
        let dump = dump_visualcontroller_via_receiver(receiver);
        crate::log_dev!(
            "[cw-visualcontroller] AppearanceEvent#{n} closure={closure:p} receiver(VisualControllerComponent*)={receiver:p} evt_ref={evt_ref:p} {dump}"
        );
    }
    let orig = ORIG_VC_APPEARANCE_INVOKE.load(Ordering::Relaxed) as *mut c_void;
    if !orig.is_null() {
        let f: VcEventInvokeFn = std::mem::transmute(orig);
        f(closure, receiver, evt_ref);
    }
}

pub(crate) unsafe extern "C" fn vc_lods_invoke_probe(closure: *mut c_void, receiver: *mut c_void, evt_ref: *mut c_void) {
    let n = VC_LODS_INVOKE_CNT.fetch_add(1, Ordering::Relaxed);
    if n < 8 {
        let dump = dump_visualcontroller_via_receiver(receiver);
        crate::log_dev!(
            "[cw-visualcontroller] AppearanceLODsDistanceOverrideEvent#{n} closure={closure:p} receiver(VisualControllerComponent*)={receiver:p} evt_ref={evt_ref:p} {dump}"
        );
    }
    let orig = ORIG_VC_LODS_INVOKE.load(Ordering::Relaxed) as *mut c_void;
    if !orig.is_null() {
        let f: VcEventInvokeFn = std::mem::transmute(orig);
        f(closure, receiver, evt_ref);
    }
}

/// Marcador individual (`~/.bwms-hook-<nome>`) OU o geral (`~/.bwms-hook`) — mesma convenção já
/// usada em `selftest.rs::install_postload_hooks::bwms_hook_enabled`, reimplementada localmente
/// (função privada daquele arquivo, não acessível daqui) sem os aliases legados `.bwms-garmentprobe*`
/// (esta é uma família de probe nova, nunca teve esse nome antigo).
pub(crate) fn vc_hook_enabled(individual: &str) -> bool {
    if !cfg!(feature = "devtools") {
        return false;
    }
    let Ok(home) = std::env::var("HOME") else { return false };
    std::path::Path::new(&format!("{home}/.bwms-hook")).exists()
        || std::path::Path::new(&format!("{home}/.bwms-hook-{individual}")).exists()
}

/// Instala os 2 probes acima, cada um atrás do próprio gate individual, nunca ligado por padrão.
/// `AtomicBool` local por probe, checa legibilidade do alvo antes de patchear, guarda o trampolim original, loga sucesso/falha.
pub(crate) unsafe fn install_visualcontroller_event_probes() {
    static APPEARANCE_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if vc_hook_enabled("visualcontroller-appearance-invoke") {
        let target = crate::rebase(VC_APPEARANCE_INVOKE_VM);
        if crate::memoria::is_readable(target as *const c_void, 16) && !APPEARANCE_INSTALLED.swap(true, Ordering::Relaxed) {
            match crate::ganchos::instalar_com_threads_paradas(target, vc_appearance_invoke_probe as *mut c_void, Some(&ORIG_VC_APPEARANCE_INVOKE)) {
                Some(_) => {
                    crate::log_dev!("[cw-visualcontroller] AppearanceEvent invoke-thunk probe @ {VC_APPEARANCE_INVOKE_VM:#010x}");
                }
                None => {
                    APPEARANCE_INSTALLED.store(false, Ordering::Relaxed);
                    crate::log_dev!("[cw-visualcontroller] FALHA AppearanceEvent invoke-thunk probe");
                }
            }
        }
    }
    static LODS_INSTALLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if vc_hook_enabled("visualcontroller-lods-invoke") {
        let target = crate::rebase(VC_LODS_INVOKE_VM);
        if crate::memoria::is_readable(target as *const c_void, 16) && !LODS_INSTALLED.swap(true, Ordering::Relaxed) {
            match crate::ganchos::instalar_com_threads_paradas(target, vc_lods_invoke_probe as *mut c_void, Some(&ORIG_VC_LODS_INVOKE)) {
                Some(_) => {
                    crate::log_dev!("[cw-visualcontroller] AppearanceLODsDistanceOverrideEvent invoke-thunk probe @ {VC_LODS_INVOKE_VM:#010x}");
                }
                None => {
                    LODS_INSTALLED.store(false, Ordering::Relaxed);
                    crate::log_dev!("[cw-visualcontroller] FALHA AppearanceLODsDistanceOverrideEvent invoke-thunk probe");
                }
            }
        }
    }
}

// ===== `VisualController` (`#213`) — leitura de campo DIRETA da struct
// `VisualController` via um `entVisualControllerComponent*` já obtido pelo PRÓPRIO redscript
// (não mais o `receiver` aproximado do thunk `__invoke` de `#167`/`#33` acima — aqui o alvo é o
// componente que o mod já tem em mãos, ex. via `entity.FindComponentByType(n"
// entVisualControllerComponent")`, item `#24`, PROVADO AO VIVO desde 2026-08-10). Round 12,
// 2026-08-12: achado de bookkeeping (o offset `+0xD8`/o layout de `VisualController` já
// estavam codados como parte do probe observe-only de `#167`, nunca expostos como capacidade
// PRÓPRIA e reusável, independente do evento disparar) + composição nova (zero endereço/RE
// adicional): reusa `dump_visualcontroller_via_receiver`'s mesmo offset `+0xD8` e o layout
// documentado no header vendorizado real (`VisualController{self@0x00,
// dependencies:DynArray<AppearanceDependency>@0x10, looseDependencies:DynArray<
// SharedPtr<ResourceToken<>>>@0x20, job:JobHandle@0x70, status:i32@0x78, lock@0x7C}`, 0x80
// bytes) — DynArray no layout já batalha-testado em dezenas de outros itens do projeto
// (`entries(ptr)@+0x00, capacity(u32)@+0x08, size(u32)@+0x0C`).
//
// **ZERO risco de tipo novo**: parâmetro é `ref<IComponent>` (tipo vanilla já seguro, mesmo
// padrão do `#168`), NÃO `entVisualControllerComponent` (que é `native class`
// sem NENHUMA ocorrência em `redscript-src` — mesma categoria CMesh-trap já corretamente
// flagada pro item irmão `#33`/`entVisualControllerComponent.reds`, NÃO declarada aqui nem em
// nenhum outro lugar do projeto). O caller passa qualquer `ref<IComponent>` (idealmente um
// achado real via `FindComponentByType`); se não for genuinamente um `VisualControllerComponent`,
// o offset `+0xD8` lê lixo — sempre atrás de `ganchos::is_readable`, então NUNCA crasha, só devolve
// valor implausível (mesma divergência honesta já aceita pro `receiver` aproximado do `#167`).
// **100% observe-only, zero mutação, zero chamada de função nativa/vtable.**
pub(crate) unsafe fn visualcontroller_ptr(component: *mut c_void) -> Option<*const u64> {
    if component.is_null() || !crate::memoria::is_readable(component, 0xE0) {
        return None;
    }
    let controller_ptr = (component as *const u64).add(0xD8 / 8).read_unaligned();
    if controller_ptr == 0 || !crate::memoria::is_readable(controller_ptr as *const c_void, 0x80) {
        return None;
    }
    Some(controller_ptr as *const u64)
}

/// `BwmsVisualControllerHasController(component: ref<IComponent>) -> Bool` — `Controller@0xD8`
/// não-nulo e legível (0x80 bytes, o tamanho inteiro da struct `VisualController` documentado).
pub(crate) unsafe extern "C" fn tramp_visualcontroller_has_controller(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let has = rtti::sane(inst) && visualcontroller_ptr(inst).is_some();
    crate::log_dev!("[cw-visualcontroller-213] BwmsVisualControllerHasController(component={inst:p}) -> {has}");
    write_bool_ret(out, has);
}

/// `BwmsVisualControllerStatus(component: ref<IComponent>) -> Int32` — `status@0x78` (o campo
/// `volatile int32_t status` do header real; sem enum nomeado no C++ vendorizado, valor cru).
pub(crate) unsafe extern "C" fn tramp_visualcontroller_status(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut status: i32 = 0;
    if rtti::sane(inst) {
        if let Some(ctrl) = visualcontroller_ptr(inst) {
            status = (ctrl as *const i32).add(0x78 / 4).read_unaligned();
        }
    }
    crate::log_dev!("[cw-visualcontroller-213] BwmsVisualControllerStatus(component={inst:p}) -> {status}");
    write_uint_ret(out, status as u32 as u64, 4);
}

/// `BwmsVisualControllerDependencyCount(component: ref<IComponent>) -> Int32` — `size` do
/// `dependencies: DynArray<AppearanceDependency>@0x10` (size do DynArray @ +0x10+0x0C = +0x1C).
pub(crate) unsafe extern "C" fn tramp_visualcontroller_dependency_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(inst) {
        if let Some(ctrl) = visualcontroller_ptr(inst) {
            count = (ctrl as *const u32).add(0x1C / 4).read_unaligned();
        }
    }
    crate::log_dev!("[cw-visualcontroller-213] BwmsVisualControllerDependencyCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u64, 4);
}

/// `BwmsVisualControllerLooseDependencyCount(component: ref<IComponent>) -> Int32` — `size` do
/// `looseDependencies: DynArray<SharedPtr<ResourceToken<>>>@0x20` (size @ +0x20+0x0C = +0x2C).
pub(crate) unsafe extern "C" fn tramp_visualcontroller_loose_dependency_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(inst) {
        if let Some(ctrl) = visualcontroller_ptr(inst) {
            count = (ctrl as *const u32).add(0x2C / 4).read_unaligned();
        }
    }
    crate::log_dev!("[cw-visualcontroller-213] BwmsVisualControllerLooseDependencyCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u64, 4);
}

pub(crate) unsafe fn register_visualcontroller_213_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok1 = if params_compostos(p1) {
        register_global_composed(reg, proto, p1, "BwmsVisualControllerHasController", "BwmsVisualControllerHasController", tramp_visualcontroller_has_controller)
    } else {
        crate::log_dev!("[reg] register_visualcontroller_213_natives: compose_params_from_types falhou (handle:IComponent) [has]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok2 = if params_compostos(p2) {
        register_global_composed(reg, proto, p2, "BwmsVisualControllerStatus", "BwmsVisualControllerStatus", tramp_visualcontroller_status)
    } else {
        crate::log_dev!("[reg] register_visualcontroller_213_natives: compose_params_from_types falhou (handle:IComponent) [status]");
        false
    };
    let p3 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok3 = if params_compostos(p3) {
        register_global_composed(reg, proto, p3, "BwmsVisualControllerDependencyCount", "BwmsVisualControllerDependencyCount", tramp_visualcontroller_dependency_count)
    } else {
        crate::log_dev!("[reg] register_visualcontroller_213_natives: compose_params_from_types falhou (handle:IComponent) [depcount]");
        false
    };
    let p4 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok4 = if params_compostos(p4) {
        register_global_composed(reg, proto, p4, "BwmsVisualControllerLooseDependencyCount", "BwmsVisualControllerLooseDependencyCount", tramp_visualcontroller_loose_dependency_count)
    } else {
        crate::log_dev!("[reg] register_visualcontroller_213_natives: compose_params_from_types falhou (handle:IComponent) [loosedepcount]");
        false
    };
    crate::log_dev!(
        "[reg] register_all: cw-visualcontroller-213 HasController={ok1} Status={ok2} DependencyCount={ok3} LooseDependencyCount={ok4}"
    );
}

// ===== `entVisualControllerComponent` (`#33`) — leitura PURA dos campos de TOPO
// do componente (distinto do `#213` acima, que lê a sub-struct C++ interna `VisualController`
// apontada por `Controller@0xD8`; aqui são os campos DECLARADOS DIRETO na classe RTTI real,
// offsets 0x90-0xF3).
//
// **CORREÇÃO DE RISCO 2026-08-18 (offline, mesma técnica que destravou o item `#107` nesta
// sessão) — a nota de risco de 2026-08-12 pra este item ("declarar a classe arriscaria o crash de
// bind RTTI, precisa de RE de endereço via hook, tipo possivelmente ausente da RTTI") checou só
// `redscript-src` (grep, zero hits pro nome da classe) e NUNCA cruzou contra o layout GERADO da
// reflection real do próprio jogo (confiança máxima, mesma categoria já usada pra fechar
// `#107`/`#197`/`#160`). Esse layout
// confirma os 4 campos citados na nota original do item com offsets EXATOS e AUTO-CONSISTENTES —
// cross-referenciados contra o tamanho do parent, o tamanho de `Ref<T>`/`RaRef<T>` e o stride de
// cada entry do array:
//   `IComponent` = 0x90 bytes exato → `meshProxy` começa
//   em 0x90 (bate); `Ref<CMesh>` = `ResourcePath path@+0x00` + `SharedPtr<...> token@+0x08` =
//   0x18 bytes → 0x90+0x18=0xA8 bate com
//   `appearanceDependency`; `DynArray<T>` = 0x10 bytes (`entries*@+0x00/capacity(u32)@+0x08/
//   size(u32)@+0x0C`, mesmo layout já batalha-testado em dezenas de itens deste projeto) →
//   0xA8+0x10=0xB8 bate com `cookedAppearanceData`; `RaRef<T>` = `ResourcePath path@+0x00` = 0x8
//   bytes → 0xB8+0x8=0xC0 bate com o filler
//   documentado antes de `forcedLodDistance@0xF0`. NENHUMA lacuna, NENHUMA sobreposição — layout
//   genuinamente confirmado por 4 fontes cruzadas, não candidato de RE.
//
// **A "correção" de 2026-08-12 continua certa sobre 1 coisa**: declarar `native class
// entVisualControllerComponent extends IComponent {...}` continuaria arriscando o crash de bind
// RTTI (mesma categoria `#30`/CMesh-trap — a CLASSE em si tem zero ocorrência em `redscript-src`,
// não sabemos se o bind aceita a ancestralidade `IComponent` sem crashar; risco não-testado,
// não-assumido). **Mas os CAMPOS não precisam da classe declarada** — mesma técnica já usada
// pelo item irmão `#213` (`visualcontroller_ptr`, leitura via `Controller@0xD8`) e por `#71`
// (VehicleObject)/`#168`: o parâmetro é sempre `ref<IComponent>` (tipo vanilla
// 100% seguro, já usado em dezenas de itens), obtido via `Entity.FindComponentByType(
// n"entVisualControllerComponent")` (item `#24`, PROVADO AO VIVO desde 2026-08-10 — usa só o
// CName pra busca, nunca declara a classe) — `entVisualControllerComponent` NUNCA é declarada em
// lugar nenhum, só os offsets de campo (0x90/0xA8/0xB8/0xF0) são lidos via ponteiro cru, sempre
// atrás de `ganchos::is_readable`.
//
// Campos expostos (todos leitura pura, zero mutação, zero RawFunc/vtable):
//   - `meshProxy: Ref<CMesh>` (0x90) → só o hash do `ResourcePath` embutido (`path@+0x00` do
//     `Ref<T>`), mesma divergência já aceita em várias natives deste projeto que devolvem hash de
//     resource em vez do handle carregado (ex. `ResourceEvent.GetPath`).
//   - `appearanceDependency: DynArray<VisualControllerDependency>` (0xA8, cada entry 0x18 bytes:
//     `mesh: RaRef<CMesh>@+0x00`/`appearanceName: CName@+0x08`/`componentName: CName@+0x10`,
//     layout GERADO) — count + 3 getters indexados (mesmo
//     padrão `GetX(i)`/`GetXCount()` já usado em dezenas de arrays deste projeto, evita o risco de
//     `array<T>`-retorno que já causou crash real noutro item — ver `GetRecordsArray`).
//   - `cookedAppearanceData: RaRef<appearance::CookedAppearanceData>` (0xB8) → hash do
//     `ResourcePath` embutido.
//   - `forcedLodDistance: ent::ForcedLodDistance` (0xF0, enum C++ de 1 byte, valores 0-8) →
//     `Int32` cru (sem declarar enum redscript-facing novo, evita mais um tipo).
//
// `LoadAppearanceDependencies()` (o único MÉTODO do item real) não tem implementação: a native fica
// registrada com corpo vazio (`register_vcc_33_carga_natives`).
pub(crate) unsafe fn vcc_ptr(component: *mut c_void) -> Option<*mut c_void> {
    if component.is_null() || !crate::memoria::is_readable(component, 0xF4) {
        return None;
    }
    Some(component)
}

/// `BwmsVccMeshProxyHash(component: ref<IComponent>) -> Uint64` — `meshProxy.path@0x90`.
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_proxy_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut hash: u64 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        hash = (inst as *const u64).add(0x90 / 8).read_unaligned();
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccMeshProxyHash(component={inst:p}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccAppearanceDependencyCount(component: ref<IComponent>) -> Int32` — `size` do
/// `appearanceDependency: DynArray<...>@0xA8` (size@0xA8+0x0C=0xB4).
pub(crate) unsafe extern "C" fn tramp_vcc_appearance_dependency_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        count = (inst as *const u32).add(0xB4 / 4).read_unaligned();
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccAppearanceDependencyCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u64, 4);
}

/// Resolve o ponteiro do entry `idx` de `appearanceDependency@0xA8` (stride 0x18), com bound-check
/// duplo (contra `size` E `capacity`, mesma disciplina defensiva já usada noutros arrays deste
/// projeto) + `ganchos::is_readable` no entry resolvido antes de devolver.
pub(crate) unsafe fn vcc_dependency_entry(inst: *mut c_void, idx: i32) -> Option<*const u8> {
    if idx < 0 {
        return None;
    }
    let entries_ptr = (inst as *const u64).add(0xA8 / 8).read_unaligned();
    let cap = (inst as *const u32).add(0xB0 / 4).read_unaligned();
    let size = (inst as *const u32).add(0xB4 / 4).read_unaligned();
    if entries_ptr == 0 || (idx as u32) >= size || (idx as u32) >= cap {
        return None;
    }
    let entry = (entries_ptr as *const u8).add((idx as usize) * 0x18);
    if !crate::memoria::is_readable(entry as *const c_void, 0x18) {
        return None;
    }
    Some(entry)
}

/// `BwmsVccAppearanceDependencyMeshHash(component, idx) -> Uint64` — `entries[idx].mesh.path@+0x00`.
pub(crate) unsafe extern "C" fn tramp_vcc_appearance_dependency_mesh_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut hash: u64 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        if let Some(entry) = vcc_dependency_entry(inst, idx) {
            hash = entry.cast::<u64>().read_unaligned();
        }
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccAppearanceDependencyMeshHash(component={inst:p}, idx={idx}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccAppearanceDependencyAppearanceName(component, idx) -> CName` — `entries[idx].appearanceName@+0x08`.
pub(crate) unsafe extern "C" fn tramp_vcc_appearance_dependency_appearance_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut hash: u64 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        if let Some(entry) = vcc_dependency_entry(inst, idx) {
            hash = entry.add(0x08).cast::<u64>().read_unaligned();
        }
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccAppearanceDependencyAppearanceName(component={inst:p}, idx={idx}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccAppearanceDependencyComponentName(component, idx) -> CName` — `entries[idx].componentName@+0x10`.
pub(crate) unsafe extern "C" fn tramp_vcc_appearance_dependency_component_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut hash: u64 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        if let Some(entry) = vcc_dependency_entry(inst, idx) {
            hash = entry.add(0x10).cast::<u64>().read_unaligned();
        }
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccAppearanceDependencyComponentName(component={inst:p}, idx={idx}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccCookedAppearanceDataHash(component: ref<IComponent>) -> Uint64` — `cookedAppearanceData.path@0xB8`.
pub(crate) unsafe extern "C" fn tramp_vcc_cooked_appearance_data_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut hash: u64 = 0;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        hash = (inst as *const u64).add(0xB8 / 8).read_unaligned();
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccCookedAppearanceDataHash(component={inst:p}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccForcedLodDistance(component: ref<IComponent>) -> Int32` — `forcedLodDistance@0xF0`
/// (enum `int8_t`, valores 0-8; devolvido cru como Int32, sem enum nomeado redscript-facing).
/// Sentinela `-1` = componente inválido/ilegível (nunca um valor real do enum, que é sempre >=0).
pub(crate) unsafe extern "C" fn tramp_vcc_forced_lod_distance(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut val: i32 = -1;
    if rtti::sane(inst) && vcc_ptr(inst).is_some() {
        val = (inst as *const u8).add(0xF0).read_unaligned() as i32;
    }
    crate::log_dev!("[cw-vcc-33] BwmsVccForcedLodDistance(component={inst:p}) -> {val}");
    write_uint_ret(out, val as u32 as u64, 4);
}

// ===== `CMaterialInstance`/`CMeshMaterialEntry`/`meshMeshAppearance` pelo `CMesh` do controller =====
// Nenhum dos 3 tipos (nem a cadeia de ancestrais) aparece em `redscript-src`: declará-los no script arrisca o bind RTTI. O único dono nativo
// deles é o `CMesh` (`materialEntries` `0xE0`, `localMaterialInstances` `0x100`, `appearances` `0x1E0`, `DATABASE.md`), e o único campo que
// leva a um `CMesh` vivo é `entVisualControllerComponent.meshProxy` (`Ref<CMesh>` em `0x90` = `{path +0x00, token +0x08}`). O token só
// existe se o motor já carregou esse `Ref`; nada aqui dispara a carga.
//
// Só leitura: com o token carregado (`finished` `+0x58` ≠ 0 e `error` `+0x5C` = 0), `resource` `+0x28` → instância → `CMesh*`. Dali,
// `materialEntries` (passo 0x30, `name` `+0x20`, `index` u16 `+0x28`; o `material` não é lido) e `appearances` (passo 0x10, cada instância
// com `name` `+0x30`). `localMaterialInstances` fica de fora.
//
// Toda a cadeia (7 saltos de ponteiro no pior caso: componente→meshProxy→token→
// finished/error→resource.instance→CMesh→array→entry/Handle→objeto) é guardada por
// `ganchos::is_readable` em CADA salto — qualquer falha devolve sentinela (0/false/-1), zero panic,
// zero mutação. `LoadAppearanceDependencies()` nunca é invocado (só leitura passiva de estado
// já resolvido, se resolvido).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_cmesh_ptr(inst: *mut c_void) -> Option<*mut c_void> {
    vcc_ptr(inst)?;
    // meshProxy: Ref<CMesh>@0x90 -> token: SharedPtr<ResourceToken<CMesh>>.instance@0x90+0x08=0x98
    let token_ptr = (inst as *const u64).add(0x98 / 8).read_unaligned() as *mut c_void;
    if token_ptr.is_null() || !crate::memoria::is_readable(token_ptr, 0x60) {
        return None;
    }
    // ResourceToken::IsLoaded() == finished@+0x58 (i32) != 0 && error@+0x5C (u8) == 0
    let finished = (token_ptr as *const i32).add(0x58 / 4).read_unaligned();
    let error = (token_ptr as *const u8).add(0x5C).read_unaligned();
    if finished == 0 || error != 0 {
        return None;
    }
    // resource: Handle<CMesh>@+0x28 -> .instance@+0x00
    let mesh_ptr = (token_ptr as *const u64).add(0x28 / 8).read_unaligned() as *mut c_void;
    if mesh_ptr.is_null() || !crate::memoria::is_readable(mesh_ptr, 0x230) {
        return None;
    }
    Some(mesh_ptr)
}

/// `BwmsVccMeshIsLoaded(component: ref<IComponent>) -> Bool` — `true` só se a cadeia inteira
/// `meshProxy.token` resolver e `IsLoaded()` bater; nunca dispara/espera load.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_is_loaded(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let loaded = rtti::sane(inst) && vcc_cmesh_ptr(inst).is_some();
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshIsLoaded(component={inst:p}) -> {loaded}");
    write_bool_ret(out, loaded);
}

/// `BwmsVccMeshMaterialEntryCount(component) -> Int32` — `size` de `materialEntries@0xE0`
/// (size@0xE0+0x0C=0xEC), 0 se o `CMesh` ainda não estiver carregado.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_material_entry_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(inst) {
        if let Some(mesh) = vcc_cmesh_ptr(inst) {
            count = (mesh as *const u32).add(0xEC / 4).read_unaligned();
        }
    }
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshMaterialEntryCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u64, 4);
}

/// Resolve o ponteiro do entry `idx` de `materialEntries@0xE0` (stride 0x30), bound-check duplo
/// (cap@0xE8/size@0xEC) + `ganchos::is_readable` no entry.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_mesh_material_entry(mesh: *mut c_void, idx: i32) -> Option<*const u8> {
    if idx < 0 {
        return None;
    }
    let entries_ptr = (mesh as *const u64).add(0xE0 / 8).read_unaligned();
    let cap = (mesh as *const u32).add(0xE8 / 4).read_unaligned();
    let size = (mesh as *const u32).add(0xEC / 4).read_unaligned();
    if entries_ptr == 0 || (idx as u32) >= size || (idx as u32) >= cap {
        return None;
    }
    let entry = (entries_ptr as *const u8).add((idx as usize) * 0x30);
    if !crate::memoria::is_readable(entry as *const c_void, 0x30) {
        return None;
    }
    Some(entry)
}

/// `BwmsVccMeshMaterialEntryNameHash(component, idx) -> Uint64` — `entries[idx].name@+0x20` (CName).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_material_entry_name_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut hash: u64 = 0;
    if rtti::sane(inst) {
        if let Some(mesh) = vcc_cmesh_ptr(inst) {
            if let Some(entry) = vcc_mesh_material_entry(mesh, idx) {
                hash = entry.add(0x20).cast::<u64>().read_unaligned();
            }
        }
    }
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshMaterialEntryNameHash(component={inst:p}, idx={idx}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsVccMeshMaterialEntryIndex(component, idx) -> Int32` — `entries[idx].index@+0x28` (Uint16).
/// Sentinela `-1` = fora de alcance/não-lido (índice real nunca é negativo).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_material_entry_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut val: i32 = -1;
    if rtti::sane(inst) {
        if let Some(mesh) = vcc_cmesh_ptr(inst) {
            if let Some(entry) = vcc_mesh_material_entry(mesh, idx) {
                val = entry.add(0x28).cast::<u16>().read_unaligned() as i32;
            }
        }
    }
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshMaterialEntryIndex(component={inst:p}, idx={idx}) -> {val}");
    write_uint_ret(out, val as u32 as u64, 4);
}

/// `BwmsVccMeshAppearanceCount(component) -> Int32` — `size` de `appearances@0x1E0`
/// (size@0x1E0+0x0C=0x1EC).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_appearance_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(inst) {
        if let Some(mesh) = vcc_cmesh_ptr(inst) {
            count = (mesh as *const u32).add(0x1EC / 4).read_unaligned();
        }
    }
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshAppearanceCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u64, 4);
}

/// Resolve `MeshAppearance*` vivo em `appearances[idx]@0x1E0` (stride 0x10, `Handle<T>.instance@
/// +0x00`), bound-check duplo (cap@0x1E8/size@0x1EC) + `ganchos::is_readable` no objeto apontado
/// (`MeshAppearance` de 0xA8 bytes).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_mesh_appearance_ptr(mesh: *mut c_void, idx: i32) -> Option<*mut c_void> {
    if idx < 0 {
        return None;
    }
    let entries_ptr = (mesh as *const u64).add(0x1E0 / 8).read_unaligned();
    let cap = (mesh as *const u32).add(0x1E8 / 4).read_unaligned();
    let size = (mesh as *const u32).add(0x1EC / 4).read_unaligned();
    if entries_ptr == 0 || (idx as u32) >= size || (idx as u32) >= cap {
        return None;
    }
    let slot = (entries_ptr as *const u8).add((idx as usize) * 0x10);
    if !crate::memoria::is_readable(slot as *const c_void, 0x10) {
        return None;
    }
    let appearance_ptr = slot.cast::<u64>().read_unaligned() as *mut c_void;
    if appearance_ptr.is_null() || !crate::memoria::is_readable(appearance_ptr, 0xA8) {
        return None;
    }
    Some(appearance_ptr)
}

/// `BwmsVccMeshAppearanceNameHash(component, idx) -> Uint64` — `appearances[idx].name@+0x30`
/// (CName da `mesh::MeshAppearance`, `DATABASE.md`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_mesh_appearance_name_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut hash: u64 = 0;
    if rtti::sane(inst) {
        if let Some(mesh) = vcc_cmesh_ptr(inst) {
            if let Some(app) = vcc_mesh_appearance_ptr(mesh, idx) {
                hash = (app as *const u64).add(0x30 / 8).read_unaligned();
            }
        }
    }
    crate::log_dev!("[cw-vcc-43] BwmsVccMeshAppearanceNameHash(component={inst:p}, idx={idx}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

pub(crate) unsafe fn register_cmesh_via_vcc_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok1 = if params_compostos(p1) {
        registro_de_prova!(register_global_composed(reg, proto, p1, "BwmsVccMeshIsLoaded", "BwmsVccMeshIsLoaded", corpo_de_prova!(tramp_vcc_mesh_is_loaded)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent) [meshloaded]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok2 = if params_compostos(p2) {
        registro_de_prova!(register_global_composed(reg, proto, p2, "BwmsVccMeshMaterialEntryCount", "BwmsVccMeshMaterialEntryCount", corpo_de_prova!(tramp_vcc_mesh_material_entry_count)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent) [entrycount]");
        false
    };
    let p3 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok3 = if params_compostos(p3) {
        registro_de_prova!(register_global_composed(reg, proto, p3, "BwmsVccMeshMaterialEntryNameHash", "BwmsVccMeshMaterialEntryNameHash", corpo_de_prova!(tramp_vcc_mesh_material_entry_name_hash)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent,Int32) [entryname]");
        false
    };
    let p4 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok4 = if params_compostos(p4) {
        registro_de_prova!(register_global_composed(reg, proto, p4, "BwmsVccMeshMaterialEntryIndex", "BwmsVccMeshMaterialEntryIndex", corpo_de_prova!(tramp_vcc_mesh_material_entry_index)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent,Int32) [entryindex]");
        false
    };
    let p5 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok5 = if params_compostos(p5) {
        registro_de_prova!(register_global_composed(reg, proto, p5, "BwmsVccMeshAppearanceCount", "BwmsVccMeshAppearanceCount", corpo_de_prova!(tramp_vcc_mesh_appearance_count)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent) [appcount]");
        false
    };
    let p6 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok6 = if params_compostos(p6) {
        registro_de_prova!(register_global_composed(reg, proto, p6, "BwmsVccMeshAppearanceNameHash", "BwmsVccMeshAppearanceNameHash", corpo_de_prova!(tramp_vcc_mesh_appearance_name_hash)))
    } else {
        crate::log_dev!("[reg] register_cmesh_via_vcc_natives: compose_params_from_types falhou (handle:IComponent,Int32) [appname]");
        false
    };
    crate::log_dev!(
        "[reg] register_all: cw-vcc-43(CMesh via meshProxy) MeshIsLoaded={ok1} MaterialEntryCount={ok2} MaterialEntryNameHash={ok3} MaterialEntryIndex={ok4} AppearanceCount={ok5} AppearanceNameHash={ok6}"
    );
}

pub(crate) unsafe fn register_vcc_33_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok1 = if params_compostos(p1) {
        register_global_composed(reg, proto, p1, "BwmsVccMeshProxyHash", "BwmsVccMeshProxyHash", tramp_vcc_mesh_proxy_hash)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent) [meshproxy]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok2 = if params_compostos(p2) {
        register_global_composed(reg, proto, p2, "BwmsVccAppearanceDependencyCount", "BwmsVccAppearanceDependencyCount", tramp_vcc_appearance_dependency_count)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent) [depcount]");
        false
    };
    let p3 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok3 = if params_compostos(p3) {
        register_global_composed(reg, proto, p3, "BwmsVccAppearanceDependencyMeshHash", "BwmsVccAppearanceDependencyMeshHash", tramp_vcc_appearance_dependency_mesh_hash)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent,Int32) [depmesh]");
        false
    };
    let p4 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok4 = if params_compostos(p4) {
        register_global_composed(reg, proto, p4, "BwmsVccAppearanceDependencyAppearanceName", "BwmsVccAppearanceDependencyAppearanceName", tramp_vcc_appearance_dependency_appearance_name)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent,Int32) [depappearance]");
        false
    };
    let p5 = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
    let ok5 = if params_compostos(p5) {
        register_global_composed(reg, proto, p5, "BwmsVccAppearanceDependencyComponentName", "BwmsVccAppearanceDependencyComponentName", tramp_vcc_appearance_dependency_component_name)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent,Int32) [depcomponent]");
        false
    };
    let p6 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok6 = if params_compostos(p6) {
        register_global_composed(reg, proto, p6, "BwmsVccCookedAppearanceDataHash", "BwmsVccCookedAppearanceDataHash", tramp_vcc_cooked_appearance_data_hash)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent) [cooked]");
        false
    };
    let p7 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok7 = if params_compostos(p7) {
        register_global_composed(reg, proto, p7, "BwmsVccForcedLodDistance", "BwmsVccForcedLodDistance", tramp_vcc_forced_lod_distance)
    } else {
        crate::log_dev!("[reg] register_vcc_33_natives: compose_params_from_types falhou (handle:IComponent) [lod]");
        false
    };
    crate::log_dev!(
        "[reg] register_all: cw-vcc-33 MeshProxyHash={ok1} DependencyCount={ok2} DependencyMeshHash={ok3} DependencyAppearanceName={ok4} DependencyComponentName={ok5} CookedAppearanceDataHash={ok6} ForcedLodDistance={ok7}"
    );
    register_vcc_33_carga_natives(reg, proto);
}

#[cfg(feature = "devtools")]
pub(crate) const VCC_LOADDEPENDENCIES_VM: u64 = 0x1_0199_e498;

#[cfg(feature = "devtools")]
pub(crate) const VCC_DEPENDENCIAS_RESIZE_VM: u64 = 0x1_0199_e36c;

#[cfg(feature = "devtools")]
pub(crate) const JOBHANDLE_DESTRUIR_VM: u64 = 0x1_04dd_f828;

#[cfg(feature = "devtools")]
pub(crate) const VCC_DEPENDENCIA_PASSO: usize = 0x18;

#[cfg(feature = "devtools")]
pub(crate) const VC_DEPENDENCIA_PASSO: usize = 0x30;

/// Acrescenta `{mesh, appearanceName}` em `appearanceDependency` do componente (o campo que o script lê); devolve o índice.
/// Com folga escreve no lugar; sem folga cresce pelo reserve do motor com mover nulo, que realoca pelo alocador copiando os bytes e zera o
/// resto (`0x1000287a0`) — certo aqui, porque a entrada é só dado simples (dois hashes e um CName).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_acrescentar_dependencia(comp: *mut c_void, mesh: u64, aparencia: u64) -> Option<u32> {
    vcc_ptr(comp)?;
    let arr = (comp as *mut u8).add(0xA8);
    let mut cap = arr.add(0x08).cast::<u32>().read_unaligned();
    let tam = arr.add(0x0C).cast::<u32>().read_unaligned();
    if tam >= cap {
        crate::jogo::listas::reservar(arr as *mut c_void, tam.saturating_mul(2).max(4), VCC_DEPENDENCIA_PASSO as u32, 8, std::ptr::null());
        cap = arr.add(0x08).cast::<u32>().read_unaligned();
        if cap <= tam {
            return None;
        }
    }
    let entradas = arr.cast::<u64>().read_unaligned() as *mut u64;
    if entradas.is_null() || !crate::memoria::is_readable(entradas as *const c_void, cap as usize * VCC_DEPENDENCIA_PASSO) {
        return None;
    }
    let nova = entradas.add(tam as usize * 3);
    nova.write_unaligned(mesh);
    nova.add(1).write_unaligned(aparencia);
    nova.add(2).write_unaligned(0);
    arr.add(0x0C).cast::<u32>().write_unaligned(tam + 1);
    Some(tam)
}

/// `entVisualControllerComponent` do V, ou nulo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_do_jogador(reg: &Registry) -> *mut c_void {
    componente_do_jogador(reg, "entVisualControllerComponent")
}

/// `BwmsVccAddAppearanceDependency(component: ref<IComponent>, mesh: ResRef, appearanceName: CName) -> Int32` — índice novo, -1 se não coube.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_add_appearance_dependency(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mesh = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let aparencia = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let idx = if rtti::sane(inst) && mesh != 0 { vcc_acrescentar_dependencia(inst, mesh, aparencia).map_or(-1, |i| i as i32) } else { -1 };
    crate::log_dev!("[cw-vcc-33] BwmsVccAddAppearanceDependency(component={inst:p}, mesh={mesh:#018x}, appearanceName={aparencia:#018x}) -> {idx}");
    write_uint_ret(out, idx as u32 as u64, 4);
}

pub(crate) unsafe fn register_vcc_33_carga_natives(reg: &Registry, proto: *mut c_void) {
    let p2 = compose_params_from_types(reg, &["handle:IComponent", "ResRef", "CName"]);
    let ok2 = params_compostos(p2)
        && registro_de_prova!(register_global_composed(reg, proto, p2, "BwmsVccAddAppearanceDependency", "BwmsVccAddAppearanceDependency", corpo_de_prova!(tramp_vcc_add_appearance_dependency)));
    crate::log_dev!("[reg] register_all: vcc-33 AddAppearanceDependency={ok2}");
}

/// `(entries, size)` de um `DynArray` num offset de propriedade do objeto.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn array_da_prop(obj: *mut c_void, classe: &str, prop: &str) -> Option<(usize, u32)> {
    let off = prop_offset_por_classe(classe, prop)?;
    sector_array(obj, off)
}

/// O `CMesh*` vivo por trás de um hash de path, ou nulo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn cmesh_por_hash(hash: u64) -> *mut c_void {
    if hash == 0 {
        return std::ptr::null_mut();
    }
    depot_loaded_resource(hash)
}

/// `BwmsMeshMaterialCount(hash) -> Int32` · `-1` = mesh fora do cache · `-2` = layout incoerente.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_material_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    if mesh.is_null() {
        write_i32_ret(out, -1);
        return;
    }
    match array_da_prop(mesh, "CMesh", "materialEntries") {
        Some((_, n)) => write_i32_ret(out, n as i32),
        None => write_i32_ret(out, -2),
    }
}

/// `BwmsMeshMaterialInfo(hash, i, campo) -> Uint64` — `campo`: 0=`name` 1=`index`
/// 2=`isLocalInstance` 3=ponteiro de `material`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_material_info(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let campo = a.get(2).map(|(v, _)| *v as i32).unwrap_or(0);
    let mut v = 0u64;
    if let Some((entries, n)) = array_da_prop(mesh, "CMesh", "materialEntries") {
        // O passo do array é o tamanho do struct — perguntado ao RTTI, nunca ao header.
        if let Some(passo) = struct_size_por_classe("CMeshMaterialEntry") {
            if i < n as usize {
                let base = entries + i * passo;
                let campo_nome = match campo {
                    1 => "index",
                    2 => "isLocalInstance",
                    3 => "material",
                    // `material` veio 0 com `isLocalInstance=1` na 1ª medição — hipótese: numa
                    // instância LOCAL o ponteiro mora no `materialWeak` (WeakHandle) em vez do
                    // Handle forte. Campo 4 mede isso em vez de eu decidir por dedução.
                    4 => "materialWeak",
                    _ => "name",
                };
                // `material`/`materialWeak` não são properties de RTTI — caem no offset do
                // header, mas só depois de conferir a impressão digital do layout.
                let achado = if campo >= 3 {
                    entry_handle_off(campo).map(|o| (o, "header conferido"))
                } else {
                    prop_offset_por_classe("CMeshMaterialEntry", campo_nome).map(|o| (o, "RTTI"))
                };
                if let Some((off, fonte)) = achado {
                    let largura = if campo == 1 { 2 } else if campo == 2 { 1 } else { 8 };
                    crate::log_dev!("[cw43] entry[{i}].{campo_nome} @ {off:#x} ({fonte}, passo {passo:#x})");
                    if crate::memoria::is_readable((base + off) as *const c_void, largura) {
                        v = match largura {
                            1 => ((base + off) as *const u8).read_unaligned() as u64,
                            2 => ((base + off) as *const u16).read_unaligned() as u64,
                            _ => rd_u64((base + off) as *const c_void, 0),
                        };
                    }
                }
            }
        }
    }
    write_uint_ret(out, v, 8);
}

/// Offset dos DOIS handles de `CMeshMaterialEntry` — os únicos campos do struct que o RTTI **não**
/// expõe (medido ao vivo 2026-09-06: há property pra `name`/`index`/`isLocalInstance`, nenhuma
/// pra `material`/`materialWeak`; são handles de runtime, não campos serializados).
///
/// Aqui o offset vem do header — e isso só é aceitável porque dá pra CONFERIR o layout antes de
/// usar. O RTTI reportou `name@0x20`, `index@0x28`, `isLocalInstance@0x2a` e tamanho `0x30`,
/// **exatamente** o que o header diz. Se os quatro batem, os dois campos restantes do mesmo
/// struct estão onde o header diz; se algum divergir, esta build tem outro layout e a função
/// recusa em vez de ler no escuro. É a diferença entre um fallback conferido e o palpite que
/// custou o `#30`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn entry_handle_off(campo: i32) -> Option<usize> {
    const NAME: usize = 0x20;
    const INDEX: usize = 0x28;
    const IS_LOCAL: usize = 0x2a;
    const SIZE: usize = 0x30;
    if struct_size_por_classe("CMeshMaterialEntry")? != SIZE
        || prop_offset_por_classe("CMeshMaterialEntry", "name")? != NAME
        || prop_offset_por_classe("CMeshMaterialEntry", "index")? != INDEX
        || prop_offset_por_classe("CMeshMaterialEntry", "isLocalInstance")? != IS_LOCAL
    {
        crate::log_dev!("[cw43] layout de CMeshMaterialEntry DIVERGE do header — recusando ler os handles");
        return None;
    }
    match campo {
        3 => Some(0x10), // Handle<IMaterial> material
        4 => Some(0x00), // WeakHandle<IMaterial> materialWeak
        _ => None,
    }
}

/// Offset de `CMaterialInstance::params` (`DynArray<MaterialParameterInstance>` @ `0x78`), com o
/// MESMO crivo de impressão digital do [`entry_handle_off`].
///
/// **Por que precisa de fallback de header.** Em 3 boots seguidos (2026-09-06) o `#43` reprovou o
/// único critério `params > 0` com `params=0`. A causa não é o mesh: `params` não é uma property
/// SERIALIZADA, então `find_property_in_class` devolve nulo e a leitura por RTTI vira 0 **em
/// silêncio** — a mesma armadilha já documentada para `CMeshMaterialEntry.material`/`materialWeak`.
///
/// A impressão digital aqui é forte: os 3 vizinhos (`enableMask`/`resourceVersion`/`audioTag`) SÃO
/// properties de RTTI e já leram `0x88`/`0x89`/`0x90` ao vivo, batendo com o header exatamente.
/// Confirmados esses 3 mais o tamanho, o `0x78` do header vale para esta build.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn material_params_off() -> Option<usize> {
    const PARAMS: usize = 0x78;
    const SIZE: usize = 0x98;
    if struct_size_por_classe("CMaterialInstance")? != SIZE
        || prop_offset_por_classe("CMaterialInstance", "enableMask")? != 0x88
        || prop_offset_por_classe("CMaterialInstance", "resourceVersion")? != 0x89
        || prop_offset_por_classe("CMaterialInstance", "audioTag")? != 0x90
    {
        crate::log_dev!("[cw43] layout de CMaterialInstance DIVERGE do header — recusando ler params");
        return None;
    }
    Some(PARAMS)
}

/// Tamanho de um struct RTTI (`IType::GetSize`, slot já usado no projeto). Sem ele o passo do
/// array viria de header — a fonte que o `#30` provou não confiável.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn struct_size_por_classe(classe: &str) -> Option<usize> {
    let reg = rtti::Registry::obtain()?;
    let cls = reg.class_by_name(classe);
    if !rtti::sane(cls) {
        return None;
    }
    let n = rtti::type_size(cls);
    (n > 0 && n < (1 << 16)).then_some(n as usize)
}

/// `BwmsMeshAppearanceCount(hash) -> Int32` — quantas `meshMeshAppearance` o mesh tem.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_appearance_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    if mesh.is_null() {
        write_i32_ret(out, -1);
        return;
    }
    match array_da_prop(mesh, "CMesh", "appearances") {
        Some((_, n)) => write_i32_ret(out, n as i32),
        None => write_i32_ret(out, -2),
    }
}

/// `BwmsMeshAppearanceName(hash, i) -> Uint64` — o `CName` da aparência `i`.
///
/// `appearances` é `DynArray<Handle<MeshAppearance>>`, então o passo é `HANDLE_STRIDE` (16, já
/// medido no `#86`) e o elemento é um ponteiro a dereferenciar — não um struct inline.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_appearance_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let mut v = 0u64;
    if let Some((entries, n)) = array_da_prop(mesh, "CMesh", "appearances") {
        if i < n as usize && crate::memoria::is_readable((entries + i * HANDLE_STRIDE) as *const c_void, 8) {
            let ap = rd_u64((entries + i * HANDLE_STRIDE) as *const c_void, 0) as *mut c_void;
            if rtti::sane(ap) {
                if let Some(off) = prop_offset_por_classe("meshMeshAppearance", "name") {
                    if crate::memoria::is_readable((ap as usize + off) as *const c_void, 8) {
                        v = rd_u64(ap as *const c_void, off);
                    }
                }
            }
        }
    }
    write_uint_ret(out, v, 8);
}

/// Resolve a `meshMeshAppearance` `i` de um mesh, ou nulo. `appearances` é
/// `DynArray<Handle<MeshAppearance>>`: passo `HANDLE_STRIDE` (16, medido no `#86`) e o elemento é
/// ponteiro a dereferenciar, não struct inline.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn mesh_appearance_em(mesh: *mut c_void, i: usize) -> *mut c_void {
    let Some((entries, n)) = array_da_prop(mesh, "CMesh", "appearances") else {
        return std::ptr::null_mut();
    };
    if i >= n as usize || !crate::memoria::is_readable((entries + i * HANDLE_STRIDE) as *const c_void, 8) {
        return std::ptr::null_mut();
    }
    let ap = rd_u64((entries + i * HANDLE_STRIDE) as *const c_void, 0) as *mut c_void;
    if rtti::sane(ap) { ap } else { std::ptr::null_mut() }
}

/// `BwmsMeshAppearanceTagCount(hash, i) -> Int32` — o 3º campo de `meshMeshAppearance` (`tags`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_appearance_tag_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let ap = mesh_appearance_em(mesh, i);
    if ap.is_null() {
        write_i32_ret(out, -1);
        return;
    }
    match array_da_prop(ap, "meshMeshAppearance", "tags") {
        Some((_, c)) => write_i32_ret(out, c as i32),
        None => write_i32_ret(out, -2),
    }
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_material_param(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let campo = a.get(2).map(|(v, _)| *v as i32).unwrap_or(0);
    let mut v = 0u64;
    if let (Some((entries, n)), Some(passo)) = (
        array_da_prop(mesh, "CMesh", "materialEntries"),
        struct_size_por_classe("CMeshMaterialEntry"),
    ) {
        if i < n as usize {
            let base = entries + i * passo;
            // o material pode vir do Handle forte OU do WeakHandle — numa instância local o
            // motor popula só um dos dois (medido 2026-09-06)
            let mut mat = std::ptr::null_mut::<c_void>();
            for (campo_h, nome) in [(3i32, "material"), (4i32, "materialWeak")] {
                let _ = nome;
                if mat.is_null() {
                    if let Some(off) = entry_handle_off(campo_h) {
                        if crate::memoria::is_readable((base + off) as *const c_void, 8) {
                            let p = rd_u64((base + off) as *const c_void, 0) as *mut c_void;
                            if rtti::sane(p) {
                                mat = p;
                            }
                        }
                    }
                }
            }
            if !mat.is_null() {
                if campo == 3 {
                    v = match array_da_prop(mat, "CMaterialInstance", "params") {
                        Some((_, c)) => {
                            crate::log_dev!("[cw43] params via RTTI = {c}");
                            c as u64
                        }
                        // RTTI não conhece `params` (não é property serializada). Cai pro header,
                        // mas só depois do crivo — e LOGA qual caminho foi usado, pra o resultado
                        // nunca ser mudo como foi nos 3 boots anteriores.
                        None => match material_params_off() {
                            Some(off)
                                if crate::memoria::is_readable((mat as usize + off) as *const c_void, 0x10) =>
                            {
                                let (e, sz, cap) = dynarray_em((mat as usize + off) as *const c_void);
                                let ok = dynarray_coerente(e, sz, cap);
                                crate::log_dev!(
                                    "[cw43] params via HEADER @{off:#x}: entries={e:#x} size={sz} cap={cap} coerente={ok}"
                                );
                                if ok { sz as u64 } else { u64::MAX }
                            }
                            _ => {
                                // SENTINELA, não 0. Um `0` aqui significaria as DUAS coisas —
                                // "array vazio" e "não consegui ler" — e foi essa ambiguidade que
                                // fez 4 boots lerem `params=0` como reprovação quando o array
                                // estava só vazio (medido: entries válido, size=0, cap=0,
                                // coerente). `u64::MAX` separa os dois casos.
                                crate::log_dev!("[cw43] params: RTTI não tem e o header foi recusado -> ILEGIVEL");
                                u64::MAX
                            }
                        },
                    };
                } else {
                    let nome = match campo {
                        1 => "resourceVersion",
                        2 => "audioTag",
                        _ => "enableMask",
                    };
                    if let Some(off) = prop_offset_por_classe("CMaterialInstance", nome) {
                        let largura = if campo == 2 { 8 } else { 1 };
                        if crate::memoria::is_readable((mat as usize + off) as *const c_void, largura) {
                            v = if largura == 1 {
                                ((mat as usize + off) as *const u8).read_unaligned() as u64
                            } else {
                                rd_u64((mat as usize + off) as *const c_void, 0)
                            };
                        }
                        crate::log_dev!("[cw43] material.{nome} @ {off:#x} (RTTI) = {v:#x}");
                    }
                }
            }
        }
    }
    write_uint_ret(out, v, 8);
}

/// `BwmsMeshAppearanceChunkCount(hash, i) -> Int32` — quantos `chunkMaterials` a aparência tem.
///
/// É o elo que liga aparência de mesh ao `chunkMask` do garment: cada chunk tem um material, e o
/// bit correspondente no mask decide se aparece. Serve direto ao trabalho de full-body.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_appearance_chunk_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let mesh = cmesh_por_hash(a.first().map(|(v, _)| *v).unwrap_or(0));
    let i = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0);
    let mut r = -1i32;
    if let Some((entries, n)) = array_da_prop(mesh, "CMesh", "appearances") {
        if i < n as usize && crate::memoria::is_readable((entries + i * HANDLE_STRIDE) as *const c_void, 8) {
            let ap = rd_u64((entries + i * HANDLE_STRIDE) as *const c_void, 0) as *mut c_void;
            if rtti::sane(ap) {
                r = match array_da_prop(ap, "meshMeshAppearance", "chunkMaterials") {
                    Some((_, c)) => c as i32,
                    None => -2,
                };
            }
        }
    }
    write_i32_ret(out, r);
}

/// `BwmsMeshRefScan(obj, ate) -> Int32` — loga TODO `u64` alinhado em `[0, ate)` do objeto.
///
/// Existe porque o `0x228` não entregou um `ResourcePath` no `entSkinnedMeshComponent` desta build: o valor lido (`0xebada5168620c5fe`)
/// não corresponde a nenhum dos **663.364** paths do install (`bwms-catalog::hash_para_path`, validada contra 2 hashes que o próprio
/// motor produziu no `#18`), e a faixa `0x1E0..0x228` não tem campo conhecido.
///
/// Em vez de tentar outro offset no palpite, despeja os candidatos e deixa a REVERSÃO OFFLINE
/// decidir: um `u64` que reverte pra um path real do dump é o campo; os outros não são. Mesmo
/// idioma "achar por CONTEÚDO, não por forma" do cache do depot e do `DataBuffer`.
///
/// Devolve quantos slots leu. Só leitura, com guarda de legibilidade por slot.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_mesh_ref_scan(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let a = rtti::read_params_consuming(current_native_func(), frame);
    let obj = a.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let ate = a.get(1).map(|(v, _)| *v as usize).unwrap_or(0x400).min(0x800);
    if obj.is_null() || !rtti::sane(obj) {
        write_i32_ret(out, -1);
        return;
    }
    let cls = rtti::class_of(obj);
    let nome = crate::cname::resolve_cname(rtti::type_name_getname(cls));
    crate::log_dev!("[cw30-scan] {obj:p} classe='{nome}' varrendo [0,{ate:#x})");
    let mut n = 0i32;
    let mut off = 0usize;
    while off + 8 <= ate {
        if crate::memoria::is_readable((obj as usize + off) as *const c_void, 8) {
            let v = rd_u64(obj as *const c_void, off);
            // 0 e ponteiros do próprio espaço de endereço não são ResourcePath — filtrar aqui
            // encurta o log sem esconder candidato (path hash é FNV-1a64, distribuído no u64
            // inteiro; ponteiro do processo fica na faixa baixa).
            if v != 0 && v > 0x1_0000_0000_0000 {
                crate::log_dev!("[cw30-scan]   +{off:#05x} = {v:#018x}");
                n += 1;
            }
        }
        off += 8;
    }
    crate::log_dev!("[cw30-scan] {n} candidato(s)");
    write_i32_ret(out, n);
}

/// `MeshComponent.meshResource: ref<CMesh>` — 2026-08-10 (cont.192): `Handle<CMesh>` em `0x158` do
/// `entMeshComponent`, campo Handle já existente (16 bytes,
/// `{instance,refCount}`, mesmo layout já lido/escrito dezenas de vezes neste projeto). Genérico
/// (aceita QUALQUER `ref<IComponent>`, não resolve classe por nome — evita a categoria de risco
/// "alias vs CNAME real" já achada pra PlayerSystem/gamePlayerSystem, cont.185): o caller decide
/// qual componente passar (via `FindComponentByType`, já provado). Handle devolvido via
/// `write_handle_ret` (raw, sem refcount nosso — o mesh resource já é gerenciado pelo motor).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_meshcomponent_get_mesh_resource(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let component = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut inst: u64 = 0;
    if rtti::sane(component) {
        let field = (component as *const u8).add(0x158) as *const c_void;
        if crate::memoria::is_readable(field, 8) {
            let candidate = rd_u64(field, 0);
            if rtti::sane(candidate as *mut c_void) {
                inst = candidate;
            }
        }
    }
    crate::log_dev!("[entity] BwmsMeshComponentGetMeshResource: component={component:p} -> inst={inst:#018x}");
    write_handle_ret(out, inst);
}

/// Corpo inteiro/torso (2026-08-07, cont.41): em vez de esperar um evento disparar pra um
/// componente de torso (3 candidatos já testados, todos silenciosos ou só-mundo — ver
/// `notes/GARMENT-APPEARANCE-COMPONENT-FACTS.md` §3), ENUMERA DIRETO os componentes reais do
/// player AGORA — mesmo offset GROUND-TRUTH já usado no construtor de entidade
/// (`Entity+0x70(ComponentsStorage)+0x30(components) = Entity+0xA0`, `DynArray<Handle<
/// IComponent>>`: entries@+0x00/cap@+0x08/size@+0x0C, stride 16B `{instance,refcount}`).
/// Resolve a CClass* de `SkinnedMeshComponent` 1x via RTTI e compara por IGUALDADE DE PONTEIRO
/// contra `rtti::class_of()` de cada componente (evita precisar resolver nome de classe a
/// partir de CClass*, que este projeto não tem pronto) — leitura 100% observe-only, zero
/// mutação, seguro mesmo em produção. Reusa o mesmo par de offsets já confirmado no probe
/// `rebuild-proxy` (`enabled@+0x8b`, `res_ptr@+0x200`) pra cada instância achada.
#[cfg(feature = "devtools")]
pub unsafe fn run_enum_skinnedmesh(reg: &crate::rtti::Registry, player: *mut c_void) -> String {
    if player.is_null() || !crate::memoria::is_readable(player as *const c_void, 0x10) {
        return "[enumcomp] player nulo/ilegível".to_string();
    }
    // Nome RTTI real confirmado neste projeto (selftest.rs, sondas de postload já provadas):
    // `entSkinnedMeshComponent` (prefixo `ent`, não o nome bare). `entGarmentSkinnedMeshComponent`
    // é a SUBCLASSE usada por peças de roupa/garment (torso é garment) — como pointer-equality
    // não pega subclasses, o match abaixo caminha a cadeia de PARENT de cada componente (mesmo
    // offset `cls+0x10` já usado em `rtti::class_of`/dump_class_funcs) até achar `target_cls` ou
    // chegar em null, cobrindo `entSkinnedMeshComponent` e QUALQUER subclasse dele (incl. Garment).
    let target_cls = reg.class_by_name("entSkinnedMeshComponent");
    if target_cls.is_null() {
        return "[enumcomp] classe 'entSkinnedMeshComponent' não achada no registry".to_string();
    }
    let arr = (player as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return "[enumcomp] player+0xA0 (ComponentsStorage.components) ilegível".to_string();
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size > 4096 {
        return format!("[enumcomp] array inválido: entries={entries:p} size={size}");
    }
    let mut total = 0usize;
    let mut matched = 0usize;
    let mut out = format!("[enumcomp] player={player:p} target_cls(SkinnedMeshComponent)={target_cls:p} size={size}\n");
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if inst.is_null() {
            continue;
        }
        total += 1;
        if class_is_kind_of(crate::rtti::class_of(inst), target_cls) {
            matched += 1;
            let leaf_cls = crate::rtti::class_of(inst);
            let (enabled, res_ptr) = if crate::memoria::is_readable(inst as *const c_void, 0x200) {
                let e = (inst as *const u8).add(0x8b).read_unaligned();
                let r = ((inst as *const u8).add(0x200) as *const u64).read_unaligned();
                (Some(e), Some(r))
            } else {
                (None, None)
            };
            out.push_str(&format!(
                "  #{matched} entSkinnedMeshComponent(ou subclasse) inst={inst:p} leaf_cls={leaf_cls:p} enabled={enabled:?} res_ptr={res_ptr:#x?}\n"
            ));
            // (cont.41): scan_cnames-lite nas instâncias DESABILITADAS (candidatas a "hide-in-fpp")
            // E também nas HABILITADAS cujo hash bate com candidatos de interesse específico
            // (ex. componente de pescoço FPP, já confirmado visível desde cont.33 — comparação
            // mais informativa que um componente habilitado genérico qualquer).
            let interesting_enabled = enabled == Some(1) && {
                let h = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
                h == crate::cname::cname("n0_000_pma_base__fpp__neck")
                    || h == crate::cname::cname("neck")
            };
            if (enabled == Some(0) || interesting_enabled) && crate::memoria::is_readable(inst as *const c_void, 0x90) {
                for off in (0x08..0x90usize).step_by(8) {
                    let w = (inst as *const u8).add(off).cast::<u64>().read_unaligned();
                    if w == 0 {
                        continue;
                    }
                    let name = crate::cname::resolve_cname(w);
                    if !name.is_empty() && name != "None" {
                        out.push_str(&format!(
                            "      scan +{off:#04x}={w:#018x} CName='{name}'\n"
                        ));
                    }
                }
            }
        }
    }
    out.push_str(&format!(
        "[enumcomp] total_componentes_lidos={total} SkinnedMeshComponent_achados={matched}"
    ));
    out
}

pub(crate) fn nome_prop_aparencia(kind: GarmentComponentKind) -> Option<&'static str> {
    (kind != GarmentComponentKind::Unsupported).then_some("meshAppearance")
}

pub(crate) fn nome_prop_chunk_mask(kind: GarmentComponentKind) -> Option<&'static str> {
    (kind != GarmentComponentKind::Unsupported).then_some("chunkMask")
}

/// Caminho do recurso do componente: campo cru de 8 bytes (o `RaRef<T>` inteiro É o `ResourcePath`, ver `garment_mesh_path_offset`).
pub(crate) unsafe fn component_get_resource_path(inst: *mut c_void, kind: GarmentComponentKind) -> Option<u64> {
    let off = garment_mesh_path_offset(inst, kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64(inst, off))
}

/// Grava o caminho do recurso (`mesh`, ou `morphResource` no `MorphTarget`). Campo cru: a recarga é de quem chama.
pub(crate) unsafe fn component_set_resource_path(inst: *mut c_void, kind: GarmentComponentKind, value: u64) -> bool {
    let off = match garment_mesh_path_offset(inst, kind) {
        Some(o) => o,
        None => return false,
    };
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return false;
    }
    wr_u64(inst, off, value);
    true
}

/// `chunkMask` do componente: campo cru, sem native nenhuma.
pub(crate) unsafe fn component_get_chunk_mask(inst: *mut c_void, kind: GarmentComponentKind) -> Option<u64> {
    let off = garment_chunk_mask_offset(inst, kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64(inst, off))
}

/// Grava o `chunkMask`. Não chama `RefreshAppearance`: a combinação das fontes é da loja de override (`es20_aplicar_mascara`) e a
/// recarga visual é de quem chama.
pub(crate) unsafe fn component_set_chunk_mask(inst: *mut c_void, kind: GarmentComponentKind, value: u64) -> bool {
    let off = match garment_chunk_mask_offset(inst, kind) {
        Some(o) => o,
        None => return false,
    };
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return false;
    }
    wr_u64(inst, off, value);
    true
}

/// `meshAppearance` do componente (campo cru, `CName`). Expostas ao redscript pelas natives `BwmsComponentGetMeshAppearanceName`/
/// `BwmsComponentSetAppearanceName` (abaixo).
pub(crate) unsafe fn component_get_appearance_name(inst: *mut c_void, kind: GarmentComponentKind) -> Option<u64> {
    let off = garment_mesh_appearance_offset(inst, kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64(inst, off))
}

pub(crate) unsafe fn component_set_appearance_name(inst: *mut c_void, kind: GarmentComponentKind, cname_hash: u64) -> bool {
    let off = match garment_mesh_appearance_offset(inst, kind) {
        Some(o) => o,
        None => return false,
    };
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return false;
    }
    wr_u64(inst, off, cname_hash);
    true
}


/// `BwmsAparencia.GetVisualTagOverrideCount(entity: wref<Entity>, tag: String) -> Int32`: quantos componentes batem com a tabela da tag pelo nome base
/// ou pelo prefixo do nome (`componentes_da_tag`), sem mudar nada. Conta também os que o apply não escreve (desligados ou fora de mesh).
pub(crate) unsafe extern "C" fn tramp_contar_componentes_da_tag(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let (checked, matched) = match tag_por_hash(crate::cname::cname(&tag)) {
        Some(tabela) if rtti::sane(entity) => componentes_da_tag(entity, tabela).map_or((0, 0), |(verificados, batem)| (verificados, batem.len())),
        _ => (0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.GetVisualTagOverrideCount(entity={entity:p}, tag='{tag}') -> checked={checked} matched={matched}"
    );
    write_uint_ret(out, matched as u64, 4);
}

/// `BwmsAparencia.ApplyVisualTagOverride(entity: wref<Entity>, tag: String) -> Int32`: o `ApplyVisualTagOverrideForMod` com `modHash` 0, na mesma loja.
/// Chamadas com tags diferentes no mesmo componente se combinam como as de mods diferentes (o esconder vence, em qualquer ordem), e
/// `RemoveVisualTagOverrideForMod(entity, tag, 0)` desfaz. Devolve quantos componentes foram escritos.
pub(crate) unsafe extern "C" fn tramp_aplicar_tag_visual(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let (checked, matched, applied) = match crate::registry() {
        Some(reg) if rtti::sane(entity) => apply_visual_tag_override_for_mod(reg, entity, &tag, 0, false),
        _ => (0, 0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.ApplyVisualTagOverride(entity={entity:p}, tag='{tag}') -> checked={checked} matched={matched} applied={applied}"
    );
    write_uint_ret(out, applied as u64, 4);
}

/// `BwmsAparencia.ApplyVisualTagOverrideForMod(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32`: a tag entra no estado de componente da
/// entidade sob a fonte (tag, mod) e cada componente que bate recebe a máscara combinada de todas as fontes (`apply_visual_tag_override_for_mod`).
/// Devolve quantos componentes foram escritos.
pub(crate) unsafe extern "C" fn tramp_aplicar_tag_visual_do_mod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let mod_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let (checked, matched, applied) = match crate::registry() {
        Some(reg) if rtti::sane(entity) => apply_visual_tag_override_for_mod(reg, entity, &tag, mod_hash, false),
        _ => (0, 0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.ApplyVisualTagOverrideForMod(entity={entity:p}, tag='{tag}', mod_hash={mod_hash:#018x}) -> checked={checked} matched={matched} applied={applied}"
    );
    write_uint_ret(out, applied as u64, 4);
}

/// `BwmsAparencia.RemoveVisualTagOverrideForMod(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32`: desfaz só a fonte (tag, mod), com os
/// overrides de outros mods e de outras tags do mesmo mod no mesmo componente intactos (removal-safety, a regra-mãe do projeto). Componente sem
/// fonte nenhuma volta à máscara original (`es20_originais`). Devolve quantos componentes foram reescritos.
pub(crate) unsafe extern "C" fn tramp_remover_tag_visual_do_mod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let mod_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let (with_state, removed, rewritten) = match crate::registry() {
        Some(reg) if rtti::sane(entity) => remove_visual_tag_override_for_mod(reg, entity, &tag, mod_hash, false),
        _ => (0, 0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.RemoveVisualTagOverrideForMod(entity={entity:p}, tag='{tag}', mod_hash={mod_hash:#018x}) -> com_estado={with_state} removidos={removed} reescritos={rewritten}"
    );
    write_uint_ret(out, rewritten as u64, 4);
}

/// `BwmsAparencia.ApplyVisualTagOverrideForModWithRefresh(entity, tag, modHash) -> Int32`: o `ApplyVisualTagOverrideForMod` com o `UpdateRenderer` em
/// cada componente escrito, pelo portão medido (`es20_refresh_medido`: vtable e slot 81 de um par de `ES20_REFRESH_MEDIDOS`). A variante sem refresh
/// só grava o campo.
pub(crate) unsafe extern "C" fn tramp_aplicar_tag_visual_do_mod_com_refresh(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let mod_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let (checked, matched, applied) = match crate::registry() {
        Some(reg) if rtti::sane(entity) => apply_visual_tag_override_for_mod(reg, entity, &tag, mod_hash, true),
        _ => (0, 0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.ApplyVisualTagOverrideForModWithRefresh(entity={entity:p}, tag='{tag}', mod_hash={mod_hash:#018x}) -> checked={checked} matched={matched} applied={applied}"
    );
    write_uint_ret(out, applied as u64, 4);
}

/// `BwmsAparencia.RemoveVisualTagOverrideForModWithRefresh(entity, tag, modHash) -> Int32`: o `RemoveVisualTagOverrideForMod` com o mesmo refresh da
/// variante de aplicar.
pub(crate) unsafe extern "C" fn tramp_remover_tag_visual_do_mod_com_refresh(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag = strings.get(1).cloned().flatten().unwrap_or_default();
    let mod_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let (with_state, removed, rewritten) = match crate::registry() {
        Some(reg) if rtti::sane(entity) => remove_visual_tag_override_for_mod(reg, entity, &tag, mod_hash, true),
        _ => (0, 0, 0),
    };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.RemoveVisualTagOverrideForModWithRefresh(entity={entity:p}, tag='{tag}', mod_hash={mod_hash:#018x}) -> com_estado={with_state} removidos={removed} reescritos={rewritten}"
    );
    write_uint_ret(out, rewritten as u64, 4);
}

// ===== `BwmsComponentGetChunkMask`/`SetChunkMask`: o `chunkMask` de UM componente que o script já tem em mãos (de um `@wrapMethod` ou evento
// de componente), pelos mesmos acessores dos componentes condicionais (`component_get_chunk_mask`/`component_set_chunk_mask`, offset pelo RTTI).
//
// `IsEnabled`/`Toggle` não precisam de native nossa: o `isEnabled` do componente já é exposto como native vanilla em `IComponent`:
// `redscript-src/orphans.script:16667` (`IsEnabled() -> Bool`) e `:16668`
// (`Toggle(on: Bool) -> Void`, native, dispara `Component/Toggle` — mesmo mecanismo já
// catalogado no item `#127`). Qualquer mod real já pode chamar `component.IsEnabled()`/
// `component.Toggle(true)` hoje, sem NENHUM trabalho do BWMS — zero gap genuíno nessa
// metade do item.
//
// Só `GetChunkMask`/`SetChunkMask` seguem genuinamente ausentes do redscript (zero hit em
// `redscript-src/` inteiro pra "chunkMask" — não é reflexão RTTI, é campo cru, sem atalho de
// Reflection possível) — fecham a metade que faltava.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_get_chunk_mask(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut mask: u64 = 0;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            mask = component_get_chunk_mask(inst, kind).unwrap_or(0);
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentGetChunkMask(component={inst:p}) -> {mask:#018x}");
    write_uint_ret(out, mask, 8);
}

/// `BwmsComponentSetChunkMask(component: ref<IComponent>, mask: Uint64) -> Bool` — escrita de
/// campo cru (`component_set_chunk_mask`), nenhum `RefreshAppearance` disparado junto de
/// propósito: o efeito visual vem de `BwmsComponentRefreshAppearance` (abaixo), chamada depois.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_set_chunk_mask(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mask = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            ok = component_set_chunk_mask(inst, kind, mask);
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentSetChunkMask(component={inst:p}, mask={mask:#018x}) -> ok={ok}");
    write_bool_ret(out, ok);
}

/// `BwmsComponentRefreshAppearance(component: ref<IComponent>) -> Bool`: recarga visual do componente de mesh; fora das classes de mesh,
/// `false`. O alvo é fixo: `0x101991450`, o slot 81 de
/// `entSkinnedMeshComponent`, provado ao vivo 2x (`APPEARANCE_REBUILD_PROXY_VM`,
/// `crate::selftest::run_toggle_and_rebuild`/comando `togglerebuild`, 2026-08-07), chamado em toda a
/// `SkinnedFamily`; `MeshComponent` e `MorphTarget` devolvem `false` sem chamar nada. O
/// `entGarmentSkinnedMeshComponent` tem slot 81 próprio, `0x10198860c` (`ES20_REFRESH_MEDIDOS`, boot 201): no garment esta
/// native chama a implementação do `entSkinnedMeshComponent`, não a da classe. O desmonte dos dois
/// (DATABASE.md, 2026-09-14) dá a mesma soltura do proxy em `+0x1d8` e os mesmos virtuais `+0x2d0`/`+0x2d8`.
// TODO(C#168, portão de refresh único): trocar o alvo fixo por `es20_refresh_medido(inst)`, o portão das natives de tag e do
// `ApplyAppearanceOverride`. Muda o que a native chama no garment (`0x10198860c`) e passa a cobrir o morph (`0x101983030`); pede
// prova ao vivo própria (`BwmsComponentRefreshAppearance` num garment e num morph do V devolvendo `true`, malha refeita), em `proofs/`.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_refresh_appearance(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            if kind == GarmentComponentKind::SkinnedFamily {
                let target = crate::rebase(0x1_0199_1450);
                if !target.is_null() && crate::memoria::is_readable(target as *const c_void, 4) {
                    type OneArgVoidFn = unsafe extern "C" fn(*mut c_void);
                    let rebuild: OneArgVoidFn = core::mem::transmute(target);
                    rebuild(inst);
                    ok = true;
                }
            }
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentRefreshAppearance(component={inst:p}) -> ok={ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_componentwrapper_get_resource_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut hash: u64 = 0;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            hash = component_get_resource_path(inst, kind).unwrap_or(0);
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentGetResourcePath(component={inst:p}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsComponentSetResourcePath(component: ref<IComponent>, path: ResRef) -> Bool` — mesma
/// disciplina de escrita cru já usada por `tramp_componentwrapper_set_chunk_mask` (campo puro,
/// nenhuma nativa de load/refresh disparada junto — quem decidir recarregar o mesh chama
/// `LoadResource`/`RefreshAppearance` depois, fora desta função, mesma separação de
/// responsabilidade da fonte real).
pub(crate) unsafe extern "C" fn tramp_componentwrapper_set_resource_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let path = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            ok = component_set_resource_path(inst, kind, path);
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentSetResourcePath(component={inst:p}, path={path:#018x}) -> ok={ok}");
    write_bool_ret(out, ok);
}

/// `BwmsComponentSetAppearanceName(component: ref<IComponent>, appearance: CName) -> Bool` —
/// `component_set_appearance_name` exposta ao redscript.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_set_appearance_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let name_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            ok = component_set_appearance_name(inst, kind, name_hash);
        }
    }
    crate::log_dev!(
        "[componentwrapper] BwmsComponentSetAppearanceName(component={inst:p}, appearance={name_hash:#018x}) -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

/// `BwmsComponentGetMeshAppearanceName(component: ref<IComponent>) -> CName` — leitura do MESMO
/// campo que `SetAppearanceName` escreve (`meshAppearance`, por-`kind`, ver `garment_mesh_
/// appearance_offset`). **ACHADO REAL desta rodada (não bookkeeping — confirmado ao vivo por um
/// negativo real)**: `meshAppearance` (campo do SUBTIPO `entMeshComponent`/`entSkinnedMeshComponent`/
/// etc.) é um campo GENUINAMENTE DIFERENTE de `IComponent::appearanceName` (campo da CLASSE BASE,
/// que entra na identidade do componente, `es20_id_do_componente`, junto com `appearancePath`, TAMBÉM da base). A native VANILLA `IComponent.GetAppearanceName()` (`orphans.script:16659`) lê
/// o campo da BASE, não `meshAppearance` do subtipo — confirmado ao vivo: escrever em
/// `meshAppearance` via `SetAppearanceName` e reler via `GetAppearanceName()` (vanilla) sempre
/// deu `None`/`matchesExpected=false`, mesmo com `wroteOk=true` (a escrita em si funcionou,
/// só não é observável pela native errada). Por isso este getter não se chama `GetAppearanceName`: o nome colide com o native vanilla de
/// `IComponent`, nunca declarável via `@addMethod` sem sombrear um `final` já existente.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_get_mesh_appearance_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut hash: u64 = 0;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            hash = component_get_appearance_name(inst, kind).unwrap_or(0);
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentGetMeshAppearanceName(component={inst:p}) -> {hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `BwmsComponentIsMeshComponent(component: ref<IComponent>) -> Bool`: o componente é de uma das 4 classes de mesh.
/// Zero-custo: `detect_garment_component_kind` já roda em toda chamada das outras funções
/// desta seção — expor o resultado do gate como método próprio evita qualquer `.reds` futuro
/// ter que replicar a lógica de classificação (que só existe em Rust).
pub(crate) unsafe extern "C" fn tramp_componentwrapper_is_mesh_component(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut is_mesh = false;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            is_mesh = kind != GarmentComponentKind::Unsupported;
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentIsMeshComponent(component={inst:p}) -> {is_mesh}");
    write_bool_ret(out, is_mesh);
}

/// `BwmsComponentLoadResource(component: ref<IComponent>) -> Bool`: fila de jobs + `MeshComponent::LoadResource(component, fila)` +
/// destrutor. O slot `0x268` é `0x10198f864` (lido em 2026-09-11: `LoadAsync` do `Ref` do mesh em `+0x220`, `Wait` no job do token pela
/// fila e despacho do closure de conclusão). Só chama se o slot da vtable do componente for exatamente essa função. Não recarrega o
/// visual (chame `BwmsComponentRefreshAppearance` depois) nem espera: uma espera de segundos travaria a thread do jogo.
pub(crate) const MESH_LOADRESOURCE_VM: u64 = 0x1_0198_f864;

pub(crate) const MESH_LOADRESOURCE_SLOT: usize = 0x268;

pub(crate) unsafe extern "C" fn tramp_componentwrapper_load_resource(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    let mut motivo = "componente inválido";
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            if detect_garment_component_kind(reg, inst) == GarmentComponentKind::Unsupported {
                motivo = "não é componente de mesh";
            } else {
                let vt = rd_u64(inst as *const c_void, 0) as *const c_void;
                let slot = if crate::memoria::is_readable(vt, MESH_LOADRESOURCE_SLOT + 8) { rd_u64(vt, MESH_LOADRESOURCE_SLOT) } else { 0 };
                if slot == 0 || slot != crate::rebase(MESH_LOADRESOURCE_VM) as u64 {
                    motivo = "slot 0x268 não é o MeshComponent::LoadResource conhecido";
                } else {
                    let kind = detect_garment_component_kind(reg, inst);
                    let off_path = garment_mesh_path_offset(inst, kind);
                    let path = off_path.map(|o| rd_u64(inst as *const c_void, o)).unwrap_or(0);
                    crate::log_dev!(
                        "[componentwrapper] LoadResource offsets RTTI mesh@{:?} meshAppearance@{:?} chunkMask@{:?} path={path:#018x}",
                        off_path,
                        garment_mesh_appearance_offset(inst, kind),
                        garment_chunk_mask_offset(inst, kind)
                    );
                    let carregar: unsafe extern "C" fn(*mut c_void, *mut c_void) = core::mem::transmute(slot as usize);
                    crate::jogo::fila::com_fila_de_jobs(&mut |fila| carregar(inst, fila));
                    ok = true;
                    motivo = "fila + LoadResource + SyncWait + destrutor";
                }
            }
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentLoadResource(component={inst:p}) -> ok={ok} ({motivo})");
    write_bool_ret(out, ok);
}

/// `mesh::MeshAppearance` (`meshMeshAppearance`, 0xa8 B no Mac; `name` 0x30, `chunkMaterials` 0x38, `tags` 0x58 conferidos no registro
/// RTTI em `0x1051a2f80`): dados de render do material `+0x48` (`RenderDataPtr`), dono `+0x50`, `Map<CName, Handle<IMaterial>>`
/// `+0x68` (chaves `DynArray` +0x68 com n em +0x74, valores `DynArray` +0x78 com n em +0x84, flags +0x88), job `+0x90`
/// (`JobHandle`), lock `+0x98` (`SharedSpinLock`, contador +0x9c), cache pronto `+0xa0`. Lido no código do motor: construtores
/// `0x1051a3464`.., destrutor `0x1051a39bc`, `LoadMaterialSetupAsync` `0x1051a3c24`.
pub(crate) const MESHAPP_MATERIAL_DATA: usize = 0x48;

pub(crate) const MESHAPP_MAPA_CHAVES_N: usize = 0x74;

pub(crate) const MESHAPP_MAPA_VALORES: usize = 0x78;

pub(crate) const MESHAPP_MAPA_VALORES_N: usize = 0x84;

pub(crate) const MESHAPP_JOB: usize = 0x90;

pub(crate) const MESHAPP_LOCK: usize = 0x98;

pub(crate) const MESHAPP_CACHE_PRONTO: usize = 0xa0;

/// `RenderData::Release` — o destrutor do `MeshAppearance` chama sobre `+0x48`: contagem em `data+8`, destrói pelo `vt[1]` no zero.
pub(crate) const RENDERDATA_RELEASE_VM: u64 = 0x1_04fe_0fb8;

/// Estado do cache de material de uma aparência: (dados de render, entradas no mapa, cache pronto).
pub(crate) unsafe fn cache_de_material(app: *mut c_void) -> (u64, u32, u8) {
    let a = app as usize;
    (
        ((a + MESHAPP_MATERIAL_DATA) as *const u64).read_unaligned(),
        ((a + MESHAPP_MAPA_VALORES_N) as *const u32).read_unaligned(),
        ((a + MESHAPP_CACHE_PRONTO) as *const u8).read(),
    )
}

/// Teto da espera pelo job de material antes de esvaziar o cache: passado ele, segue assim mesmo (a native não pode travar o jogo).
const ESPERA_DO_JOB_DE_MATERIAL: std::time::Duration = std::time::Duration::from_secs(5);

/// Deixa o cache de material da `MeshAppearance` vazio e não pronto, para o motor recarregar os materiais na próxima aplicação da aparência.
/// O mapa é esvaziado (cada material perde a referência forte), não destruído: os buffers ficam para o motor reusar.
pub(crate) unsafe fn resetar_cache_de_material(app: *mut c_void) -> bool {
    if app.is_null() || !crate::memoria::is_readable(app as *const c_void, 0xa8) {
        return false;
    }
    let a = app as *mut u8;
    let job = a.add(MESHAPP_JOB) as *const u64;
    let terminou: unsafe extern "C" fn(*const u64) -> bool = std::mem::transmute(crate::rebase(crate::jogo::fila::TAREFA_TERMINOU_VM));
    let fim = std::time::Instant::now() + ESPERA_DO_JOB_DE_MATERIAL;
    while job.read_unaligned() != 0 && !terminou(job) && std::time::Instant::now() < fim {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let trava = a.add(MESHAPP_LOCK);
    crate::jogo::travas::travar_escrita(trava);
    let dados = a.add(MESHAPP_MATERIAL_DATA) as *mut u64;
    let d = dados.read_unaligned();
    if d != 0 {
        let soltar: unsafe extern "C" fn(u64) = std::mem::transmute(crate::rebase(RENDERDATA_RELEASE_VM));
        soltar(d);
        dados.write_unaligned(0);
    }
    let valores = (a.add(MESHAPP_MAPA_VALORES) as *const u64).read_unaligned() as *mut [u64; 2];
    let n = (a.add(MESHAPP_MAPA_VALORES_N) as *const u32).read_unaligned() as usize;
    if !valores.is_null() && n > 0 && crate::memoria::is_readable(valores as *const c_void, n * 16) {
        for i in 0..n {
            crate::jogo::objetos::soltar_handle(&mut *valores.add(i));
        }
    }
    (a.add(MESHAPP_MAPA_VALORES_N) as *mut u32).write_unaligned(0);
    (a.add(MESHAPP_MAPA_CHAVES_N) as *mut u32).write_unaligned(0);
    a.add(MESHAPP_CACHE_PRONTO).write(0);
    crate::jogo::travas::soltar_escrita(trava);
    true
}

/// Aparência do mesh de um componente pelo nome (`CMesh::appearances`). `None` = tipo fora do escopo (o MorphTarget usa o `baseMesh` do handle, não coberto), sem mesh ou nome não achado.
pub(crate) unsafe fn aparencia_do_componente(inst: *mut c_void) -> Result<*mut c_void, String> {
    if !rtti::sane(inst) {
        return Err("componente inválido".into());
    }
    let Some(reg) = crate::registry() else { return Err("sem registry".into()) };
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::MorphTarget || kind == GarmentComponentKind::Unsupported {
        return Err(format!("tipo fora do escopo ({kind:?})"));
    }
    let nome = component_get_appearance_name(inst, kind).unwrap_or(0);
    let Some(cmesh) = component_get_mesh_handle_instance(inst, kind) else { return Err(format!("sem mesh (kind={kind:?})")) };
    cmesh_resolve_appearance(cmesh, nome).ok_or_else(|| {
        format!("aparência {nome:#x} não achada no mesh {cmesh:p} (entradas={})", cmesh_appearances_count(cmesh).unwrap_or(0))
    })
}

/// `BwmsComponentResetMaterialCache(component: ref<IComponent>) -> Bool`: reseta o cache de material da aparência do mesh do componente. O motor reconstrói na próxima aplicação.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_reset_material_cache(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let (ok, detalhe) = match aparencia_do_componente(inst) {
        Ok(app) => {
            let antes = cache_de_material(app);
            let ok = resetar_cache_de_material(app);
            let depois = cache_de_material(app);
            (ok, format!(
                "aparência={app:p} antes(dados={:#x} mapa={} pronto={}) depois(dados={:#x} mapa={} pronto={})",
                antes.0, antes.1, antes.2, depois.0, depois.1, depois.2
            ))
        }
        Err(e) => (false, e),
    };
    crate::veredito_dev!("[componentwrapper] ResetMaterialCache(component={inst:p}) -> {ok} {detalhe}");
    write_bool_ret(out, ok);
}

/// `BwmsComponentMaterialCacheCount(component: ref<IComponent>) -> Int32` — helper do BWMS pra prova do `ResetMaterialCache`:
/// entradas no mapa de material da aparência do componente (-1 = aparência não achada). Loga os dados de render e o flag.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_componentwrapper_material_cache_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = match aparencia_do_componente(inst) {
        Ok(app) => {
            let (dados, n, pronto) = cache_de_material(app);
            crate::veredito_dev!("[componentwrapper] MaterialCache(component={inst:p}) aparência={app:p} dados={dados:#x} mapa={n} pronto={pronto}");
            n as i32
        }
        Err(e) => {
            crate::veredito_dev!("[componentwrapper] MaterialCache(component={inst:p}) -> {e}");
            -1
        }
    };
    write_i32_ret(out, n);
}

/// `BwmsComponentLoadResourceToken(component: ref<IComponent>) -> Uint64`: pede a carga do path do mesh do componente (offset pelo RTTI) ao loader do motor e devolve o id do token
/// guardado (0 = tipo fora do escopo, path vazio ou sem loader). Não espera: `ResourceToken.WaitForJob()` espera.
pub(crate) unsafe extern "C" fn tramp_componentwrapper_load_resource_token(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut path = 0u64;
    let mut id = 0u64;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            if let Some(off) = garment_mesh_path_offset(inst, kind) {
                path = rd_u64(inst as *const c_void, off);
                if path != 0 {
                    id = crate::arquivos_do_jogo::carregar_e_guardar(path);
                }
            }
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentLoadResourceToken(component={inst:p}) path={path:#018x} -> id={id}");
    write_uint_ret(out, id, 8);
}

/// `BwmsComponentGetMeshInstance(component: ref<IComponent>) -> Uint64` — helper do BWMS pra prova do `LoadResource`: o
/// `CMesh*` já resolvido no `meshHandle` do componente (`+0x158` mesh simples, `+0x200` família skinned na 2.5 — onde o closure de
/// conclusão do `LoadResource` `0x10199264c` grava; `0` sem mesh ou tipo fora do escopo). O `+8` desse offset
/// é o bloco de refcount do Handle; os dois vão pro log. O path (`GetResourcePath`) diz o que pedir; este diz o que o motor de fato carregou.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_componentwrapper_mesh_instance(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut cmesh: u64 = 0;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            let off = match kind {
                GarmentComponentKind::SkinnedFamily => Some(0x200usize),
                GarmentComponentKind::MeshComponent => Some(0x158usize),
                _ => None,
            };
            if let Some(off) = off {
                if crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 0x10) {
                    cmesh = rd_u64(inst as *const c_void, off);
                    let vizinho = rd_u64(inst as *const c_void, off + 8);
                    crate::log_dev!("[componentwrapper] GetMeshInstance(component={inst:p}) kind={kind:?} +{off:#x}={cmesh:#x} +{:#x}={vizinho:#x}", off + 8);
                }
            }
        }
    }
    write_uint_ret(out, cmesh, 8);
}

/// Lê `Handle<CMesh>.instance` (8 bytes, offset 0 do Handle de 16 bytes) no offset de
/// `meshHandle` do componente — devolve o `CMesh*` já RESOLVIDO (distinto do `RaRef`/path que
/// `GetResourcePath` lê).
pub(crate) unsafe fn component_get_mesh_handle_instance(inst: *mut c_void, kind: GarmentComponentKind) -> Option<*mut c_void> {
    let off = garment_mesh_handle_offset(kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 16) {
        return None;
    }
    let cmesh = rd_u64(inst, off) as *mut c_void;
    if cmesh.is_null() {
        return None;
    }
    Some(cmesh)
}

/// `CMesh::FindAppearance(CMesh*, CName) -> Handle<mesh::MeshAppearance>&` — a regra do motor (`0x10519c4b0`, lida no
/// disassembly em 2026-09-11): scan de `CMesh::appearances@0x1E0` (`DynArray<Handle<mesh::MeshAppearance>>`, stride 16
/// bytes) comparando `MeshAppearance::name@0x30`, pulando entrada vazia; sem casamento, `None` ou o nome padrão (CName
/// global do motor, `default`) caem na PRIMEIRA entrada. Devolve o `MeshAppearance*` (o `.instance`) — leitura pura.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn cmesh_find_appearance(cmesh: *mut c_void, target_name_hash: u64) -> Option<*mut c_void> {
    let entradas = cmesh_appearance_entries(cmesh)?;
    let i = escolher_aparencia(&entradas, target_name_hash, cname_aparencia_padrao())?;
    let inst = entradas[i].0;
    (inst != 0).then_some(inst as *mut c_void)
}

/// O que o componente usa de fato: `0x10519e118` (chamado no fim do `MeshComponent::LoadResource` com o `meshAppearance`)
/// procura o nome e, se não vier instância, procura de novo com o nome padrão. Um `meshAppearance` = `default` sem
/// aparência com esse nome resolve pra primeira entrada.
pub(crate) unsafe fn cmesh_resolve_appearance(cmesh: *mut c_void, target_name_hash: u64) -> Option<*mut c_void> {
    let entradas = cmesh_appearance_entries(cmesh)?;
    let i = resolver_aparencia(&entradas, target_name_hash, cname_aparencia_padrao())?;
    let inst = entradas[i].0;
    (inst != 0).then_some(inst as *mut c_void)
}

/// `(instância, nome)` de cada entrada de `CMesh::appearances`; entrada ilegível conta como vazia.
pub(crate) unsafe fn cmesh_appearance_entries(cmesh: *mut c_void) -> Option<Vec<(u64, u64)>> {
    if cmesh.is_null() || !crate::memoria::is_readable(cmesh as *const c_void, 0x1F0) {
        return None;
    }
    let arr = (cmesh as *mut u8).add(0x1E0) as *mut c_void;
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if size == 0 {
        return Some(Vec::new());
    }
    if entries.is_null() || size > 4096 {
        return None;
    }
    let mut v = Vec::with_capacity(size as usize);
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        let mut inst = if crate::memoria::is_readable(slot as *const c_void, 16) {
            core::ptr::read_unaligned(slot as *const u64)
        } else {
            0
        };
        let mut nome = 0;
        if inst != 0 {
            if crate::memoria::is_readable(inst as *const c_void, 0x38) {
                nome = ((inst as *const u8).add(0x30) as *const u64).read_unaligned();
            } else {
                inst = 0;
            }
        }
        v.push((inst, nome));
    }
    Some(v)
}

/// Índice que o `FindAppearance` do motor devolve: casamento exato (pulando entrada vazia); sem casamento, `None` ou o
/// nome padrão → a primeira entrada.
pub(crate) fn escolher_aparencia(entradas: &[(u64, u64)], nome: u64, padrao: u64) -> Option<usize> {
    if let Some(i) = entradas.iter().position(|&(inst, n)| inst != 0 && n == nome) {
        return Some(i);
    }
    (!entradas.is_empty() && (nome == 0 || nome == padrao)).then_some(0)
}

/// O wrapper `0x10519e118`: sem entrada com instância, repete a busca com o nome padrão.
pub(crate) fn resolver_aparencia(entradas: &[(u64, u64)], nome: u64, padrao: u64) -> Option<usize> {
    match escolher_aparencia(entradas, nome, padrao) {
        Some(i) if entradas[i].0 != 0 => Some(i),
        _ => escolher_aparencia(entradas, padrao, padrao),
    }
}

/// Global do CName padrão que o `FindAppearance` compara (`adrp 0x109cc2e00; add #0xb40`).
pub(crate) const CNAME_APARENCIA_PADRAO_VM: u64 = 0x1_09d2_4818;

/// O CName padrão lido do motor; loga uma vez o valor contra o `default`.
pub(crate) unsafe fn cname_aparencia_padrao() -> u64 {
    static LOGOU: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let p = crate::rebase(CNAME_APARENCIA_PADRAO_VM) as *const u64;
    let v = if crate::memoria::is_readable(p as *const c_void, 8) { p.read_unaligned() } else { 0 };
    if !LOGOU.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::veredito_dev!(
            "[componentwrapper] CName padrão do FindAppearance: global={v:#x} default={CNAME_DEFAULT:#x} igual={}",
            v == CNAME_DEFAULT
        );
    }
    if v == 0 { CNAME_DEFAULT } else { v }
}

/// Contagem de `CMesh::appearances` — diagnóstico bônus (cross-validação: se o componente tem
/// `meshAppearance` não-vazia mas a contagem é 0, algo está errado no offset/CMesh resolvido).
pub(crate) unsafe fn cmesh_appearances_count(cmesh: *mut c_void) -> Option<u32> {
    if cmesh.is_null() || !crate::memoria::is_readable(cmesh as *const c_void, 0x1F0) {
        return None;
    }
    let arr = (cmesh as *mut u8).add(0x1E0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    Some(core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32))
}

/// `BwmsComponentFindAppearanceProbe(component: ref<IComponent>) -> CName` — diagnóstico
/// OBSERVE-ONLY (zero escrita) que exercita a cadeia inteira: `meshHandle` (Handle<CMesh> já
/// resolvido) -> `CMesh::appearances` (scan) -> compara contra `meshAppearance` (o nome que o
/// PRÓPRIO componente já está usando, lido via `component_get_appearance_name`, já provado ao
/// vivo desde 2026-08-16). Devolve o hash do NOME ACHADO na array (0 se não achou nada) — a
/// cross-validação decisiva é comparar esse retorno contra `BwmsComponentGetMeshAppearanceName`
/// no MESMO componente: se baterem, a cadeia inteira (offset do Handle + layout do DynArray +
/// offset do campo `name`) está confirmada correta por um mecanismo INDEPENDENTE (scan de array
/// real vs. campo cru já provado), não coincidência.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_componentwrapper_find_appearance_probe(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut found_hash: u64 = 0;
    let mut mesh_ptr: *mut c_void = std::ptr::null_mut();
    let mut count: i64 = -1;
    let mut search_hash: u64 = 0;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            if let Some(cmesh) = component_get_mesh_handle_instance(inst, kind) {
                mesh_ptr = cmesh;
                count = cmesh_appearances_count(cmesh).map(|v| v as i64).unwrap_or(-1);
                search_hash = component_get_appearance_name(inst, kind).unwrap_or(0);
                if let Some(app) = cmesh_find_appearance(cmesh, search_hash) {
                    if crate::memoria::is_readable((app as *const u8).add(0x30) as *const c_void, 8) {
                        found_hash = core::ptr::read_unaligned((app as *const u8).add(0x30) as *const u64);
                    }
                }
            }
        }
    }
    crate::log_dev!(
        "[componentwrapper] BwmsComponentFindAppearanceProbe(component={inst:p}) mesh={mesh_ptr:p} appearances_count={count} search={search_hash:#018x} found={found_hash:#018x} match={}",
        found_hash != 0 && found_hash == search_hash
    );
    write_uint_ret(out, found_hash, 8);
}

/// `BwmsComponentAppearanceCount(component: ref<IComponent>) -> Int32` — diagnóstico bônus
/// (`-1` se `meshHandle` não resolveu, senão a contagem real de `CMesh::appearances`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_componentwrapper_appearance_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: i32 = -1;
    if rtti::sane(inst) {
        if let Some(reg) = crate::registry() {
            let kind = detect_garment_component_kind(reg, inst);
            if let Some(cmesh) = component_get_mesh_handle_instance(inst, kind) {
                count = cmesh_appearances_count(cmesh).unwrap_or(0) as i32;
            }
        }
    }
    crate::log_dev!("[componentwrapper] BwmsComponentAppearanceCount(component={inst:p}) -> {count}");
    write_uint_ret(out, count as u32 as u64, 4);
}

pub(crate) unsafe fn register_componentwrapper_extra_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok1 = if params_compostos(p1) {
        register_global_composed(reg, proto, p1, "BwmsComponentGetResourcePath", "BwmsComponentGetResourcePath", tramp_componentwrapper_get_resource_path)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent) [get-path]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:IComponent", "ResRef"]);
    let ok2 = if params_compostos(p2) {
        register_global_composed(reg, proto, p2, "BwmsComponentSetResourcePath", "BwmsComponentSetResourcePath", tramp_componentwrapper_set_resource_path)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent, ResRef) [set-path]");
        false
    };
    // `BwmsComponentGetAppearanceName` NÃO é declarado (`redscript-src/orphans.script:16659`): `IComponent.GetAppearanceName()
    // -> CName` já é `final native const` VANILLA REAL (usado por código de jogo genuíno,
    // `weapon.script:642`) — mas confirmado AO VIVO nesta mesma rodada que ela lê um campo
    // DIFERENTE (`IComponent::appearanceName`, da classe BASE) do que `SetAppearanceName`
    // escreve (`meshAppearance`, do SUBTIPO) — ver nota grande em
    // `tramp_componentwrapper_get_mesh_appearance_name`. Por isso o getter do campo que
    // `SetAppearanceName` realmente usa é exposto sob um nome PRÓPRIO (`GetMeshAppearanceName`,
    // sem colisão com o `final` vanilla).
    let p3 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok3 = if params_compostos(p3) {
        register_global_composed(reg, proto, p3, "BwmsComponentGetMeshAppearanceName", "BwmsComponentGetMeshAppearanceName", tramp_componentwrapper_get_mesh_appearance_name)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent) [get-mesh-appearance]");
        false
    };
    let p4 = compose_params_from_types(reg, &["handle:IComponent", "CName"]);
    let ok4 = if params_compostos(p4) {
        register_global_composed(reg, proto, p4, "BwmsComponentSetAppearanceName", "BwmsComponentSetAppearanceName", tramp_componentwrapper_set_appearance_name)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent, CName) [set-appearance]");
        false
    };
    let p5 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok5 = if params_compostos(p5) {
        register_global_composed(reg, proto, p5, "BwmsComponentIsMeshComponent", "BwmsComponentIsMeshComponent", tramp_componentwrapper_is_mesh_component)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent) [is-mesh]");
        false
    };
    // 2026-08-15: `CMesh::FindAppearance` via composição pura (zero RE de endereço nativo —
    // ver nota grande em `garment_mesh_handle_offset`/`cmesh_find_appearance` acima). Diagnóstico
    // observe-only, cross-validado contra `GetMeshAppearanceName` já provado.
    let p6 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok6 = if params_compostos(p6) {
        registro_de_prova!(register_global_composed(reg, proto, p6, "BwmsComponentFindAppearanceProbe", "BwmsComponentFindAppearanceProbe", corpo_de_prova!(tramp_componentwrapper_find_appearance_probe)))
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent) [find-appearance]");
        false
    };
    let p7 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok7 = if params_compostos(p7) {
        registro_de_prova!(register_global_composed(reg, proto, p7, "BwmsComponentAppearanceCount", "BwmsComponentAppearanceCount", corpo_de_prova!(tramp_componentwrapper_appearance_count)))
    } else {
        crate::log_dev!("[reg] register_componentwrapper_extra_natives: compose falhou (handle:IComponent) [appearance-count]");
        false
    };
    // `#168` (2026-09-11): `LoadResource` = fila de jobs + slot 0x268 (`0x10198f864`) + destrutor.
    let ok8 = {
        let p8 = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p8)
            && register_global_composed(reg, proto, p8, "BwmsComponentLoadResource", "BwmsComponentLoadResource", tramp_componentwrapper_load_resource as NativeHandler)
    };
    let ok9 = {
        let p9 = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p9)
            && registro_de_prova!(register_global_composed(reg, proto, p9, "BwmsComponentGetMeshInstance", "BwmsComponentGetMeshInstance", corpo_de_prova!(tramp_componentwrapper_mesh_instance)))
    };
    let ok10 = {
        let p10 = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p10)
            && register_global_composed(reg, proto, p10, "BwmsComponentLoadResourceToken", "BwmsComponentLoadResourceToken", tramp_componentwrapper_load_resource_token as NativeHandler)
    };
    let ok11 = {
        let p11 = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p11)
            && register_global_composed(reg, proto, p11, "BwmsComponentResetMaterialCache", "BwmsComponentResetMaterialCache", tramp_componentwrapper_reset_material_cache as NativeHandler)
    };
    let ok12 = {
        let p12 = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p12)
            && registro_de_prova!(register_global_composed(reg, proto, p12, "BwmsComponentMaterialCacheCount", "BwmsComponentMaterialCacheCount", corpo_de_prova!(tramp_componentwrapper_material_cache_count)))
    };
    // O smoke de `BwmsAparencia` guarda e devolve o estado de aparência do V em volta do setup dele (`es22_guardar_estado`).
    let ok13 = {
        let p13 = compose_params_from_types(reg, &["handle:Entity", "handle:IComponent", "Uint64"]);
        params_compostos(p13)
            && registro_de_prova!(register_global_composed(reg, proto, p13, "BwmsComponentStateSave", "BwmsComponentStateSave", corpo_de_prova!(tramp_component_state_save)))
    };
    let ok14 = registro_de_prova!(register_global(reg, proto, "BwmsComponentStateRestore", "BwmsComponentStateRestore", corpo_de_prova!(tramp_component_state_restore)));
    // O smoke das tags visuais guarda e devolve o estado de máscara do V em volta das chamadas dele (`es20_guardar_tags`).
    let ok15 = {
        let p15 = compose_params_from_types(reg, &["handle:Entity"]);
        params_compostos(p15)
            && registro_de_prova!(register_global_composed(reg, proto, p15, "BwmsVisualTagStateSave", "BwmsVisualTagStateSave", corpo_de_prova!(tramp_visual_tag_state_save)))
    };
    let ok16 = registro_de_prova!(register_global(reg, proto, "BwmsVisualTagStateRestore", "BwmsVisualTagStateRestore", corpo_de_prova!(tramp_visual_tag_state_restore)));
    crate::log_dev!(
        "[reg] register_all: cw-componentwrapper-extra (#168/#22) GetResourcePath={ok1} SetResourcePath={ok2} GetMeshAppearanceName={ok3} SetAppearanceName={ok4} IsMeshComponent={ok5} FindAppearanceProbe={ok6} AppearanceCount={ok7} LoadResource={ok8} GetMeshInstance={ok9} LoadResourceToken={ok10} ResetMaterialCache={ok11} MaterialCacheCount={ok12} StateSave={ok13} StateRestore={ok14} TagStateSave={ok15} TagStateRestore={ok16}"
    );
}

pub(crate) unsafe fn register_componentwrapper_chunkmask_natives(reg: &Registry, proto: *mut c_void) {
    let p1 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok1 = if params_compostos(p1) {
        register_global_composed(reg, proto, p1, "BwmsComponentGetChunkMask", "BwmsComponentGetChunkMask", tramp_componentwrapper_get_chunk_mask)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_chunkmask_natives: compose_params_from_types falhou (handle:IComponent) [get]");
        false
    };
    let p2 = compose_params_from_types(reg, &["handle:IComponent", "Uint64"]);
    let ok2 = if params_compostos(p2) {
        register_global_composed(reg, proto, p2, "BwmsComponentSetChunkMask", "BwmsComponentSetChunkMask", tramp_componentwrapper_set_chunk_mask)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_chunkmask_natives: compose_params_from_types falhou (handle:IComponent, Uint64) [set]");
        false
    };
    let p3 = compose_params_from_types(reg, &["handle:IComponent"]);
    let ok3 = if params_compostos(p3) {
        register_global_composed(reg, proto, p3, "BwmsComponentRefreshAppearance", "BwmsComponentRefreshAppearance", tramp_componentwrapper_refresh_appearance)
    } else {
        crate::log_dev!("[reg] register_componentwrapper_chunkmask_natives: compose_params_from_types falhou (handle:IComponent) [refresh]");
        false
    };
    crate::log_dev!(
        "[reg] register_all: cw-componentwrapper-chunkmask (#168) BwmsComponentGetChunkMask={ok1} BwmsComponentSetChunkMask={ok2} BwmsComponentRefreshAppearance={ok3}"
    );
}


/// Loja de override por `(entidade, nome do componente)`.
pub(crate) type Es20Estados = std::collections::HashMap<(u64, u64), bwms_primitives::override_state::ComponentState>;

/// Aparência `aparencia` da fonte `hash` na chave `nome`. Recusa nome 0, aparência 0 e `default` (pedir `default` é não pedir override).
pub(crate) fn es20_adicionar_aparencia(entidade: u64, hash: u64, nome: CName, aparencia: CName) -> bool {
    if nome.valor() == 0 || aparencia.valor() == 0 || aparencia.valor() == CNAME_DEFAULT {
        return false;
    }
    let Ok(mut m) = es20_componentes().lock() else { return false };
    m.entry((entidade, nome.valor())).or_default().sobrescrever_aparencia(hash, aparencia.valor());
    true
}

/// `RefreshAppearance`/`UpdateRenderer` (Windows `0x280`, os 2 no mesmo slot; redscript `RefreshAppearance()`) = slot 81 da vtable no Mac, medido por classe:
/// `0x101991450` nas 3 vtables de antes e, no boot 201, `0x10198860c` na do `entGarmentSkinnedMeshComponent` e `0x101983030` na do
/// `entMorphTargetSkinnedMeshComponent`. O desmonte dos 3 dá o mesmo corpo: solta o render proxy (`+0x1d8`; `+0x1e0` no morph) e chama os virtuais `+0x2d0` e
/// `+0x2d8`. `(vtable, alvo)` estáticos.
pub(crate) const ES20_REFRESH_MEDIDOS: [(u64, u64); 5] = [
    (0x1_078b_a628, 0x1_0199_1450),
    (0x1_078c_3a88, 0x1_0199_1450),
    (0x1_078c_3d78, 0x1_0199_1450),
    (0x1_078c_32a8, 0x1_0198_860c),
    (0x1_078c_2ab8, 0x1_0198_3030),
];


/// `BwmsAparencia.LinkComponentToPart(entity: wref<Entity>, component: ref<IComponent>,
/// resourcePath: Uint64) -> Bool`: liga o nome base do componente à parte, numa entidade EXPLÍCITA (a loja é por ponteiro de entidade).
/// Setup necessário ANTES de `ApplyAppearanceOverride`/`ToggleConditionalComponents`: sem parte em `ES22_COMPONENT_APPEARANCE`, os 2
/// recusam aplicar.
pub(crate) unsafe extern "C" fn tramp_ligar_componente_a_parte(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let component = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let resource_path = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let ok = es22_ligar_componente_vivo(entity, component, resource_path);
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.LinkComponentToPart(entity={entity:p}, component={component:p}, resourcePath={resource_path:#018x}) -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

/// O corpo do `BwmsAparencia.LinkComponentToPart`, fora da leitura dos argumentos: a parte vai para o nome base do componente vivo, na loja que o
/// pipeline do #20 também lê (`es22_parte_do_componente`).
pub(crate) unsafe fn es22_ligar_componente_vivo(entity: *mut c_void, component: *mut c_void, parte: u64) -> bool {
    rtti::sane(entity) && rtti::sane(component) && es22_ligar_componente(entity as u64, es20_nome_base(component as *const u8), parte)
}

/// `BwmsAparencia.LinkPartToAppearanceName(entity: wref<Entity>, resourcePath: Uint64, appearance:
/// String) -> Bool`: liga a parte à aparência ativa, lida do texto por `DynamicAppearanceName::parse`. Setup necessário pra
/// `ToggleConditionalComponents` (a aparência ativa da parte) e pra guarda de `ApplyAppearanceOverride`.
pub(crate) unsafe extern "C" fn tramp_ligar_parte_a_aparencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let resource_path = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let appearance = strings.get(2).cloned().flatten().unwrap_or_default();
    let mut ok = false;
    if rtti::sane(entity) {
        let parsed = DynamicAppearanceName::parse(&appearance);
        if let Ok(mut g) = ES22_RESOURCE_STATE.lock() {
            let key = (entity as u64, resource_path);
            match g.iter_mut().find(|(k, _)| *k == key) {
                Some((_, s)) => *s = parsed,
                None => g.push((key, parsed)),
            }
            ok = true;
        }
    }
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.LinkPartToAppearanceName(entity={entity:p}, resourcePath={resource_path:#018x}, appearance='{appearance}') -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

/// `BwmsAparencia.AddAppearanceOverrideForComponent(entity: wref<Entity>, component: ref<IComponent>,
/// modHash: Uint64, appearance: CName) -> Bool`: recebe o COMPONENTE vivo e grava no `ComponentState` de `es20_componentes` pela chave
/// `(entidade, nome base)`, a mesma loja do registro de item, então os dois caminhos decidem a aparência igual. Aparência vazia ou
/// `default` não registra nada (`es20_adicionar_aparencia`).
pub(crate) unsafe extern "C" fn tramp_adicionar_aparencia_do_componente(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let component = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mod_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let appearance = args.get(3).map(|(v, _)| *v).unwrap_or(0);
    let ok = es22_adicionar_aparencia_do_componente_vivo(entity, component, mod_hash, appearance);
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.AddAppearanceOverrideForComponent(entity={entity:p}, component={component:p}, modHash={mod_hash:#018x}, appearance={appearance:#018x}) -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

/// O corpo do `BwmsAparencia.AddAppearanceOverrideForComponent`, fora da leitura dos argumentos: o override entra no `ComponentState` do nome base do
/// componente vivo em `es20_componentes`, a loja do es20.
pub(crate) unsafe fn es22_adicionar_aparencia_do_componente_vivo(entity: *mut c_void, component: *mut c_void, mod_hash: u64, aparencia: u64) -> bool {
    rtti::sane(entity)
        && rtti::sane(component)
        && es20_adicionar_aparencia(entity as u64, mod_hash, CName::lido_do_motor(es20_nome_base(component as *const u8)), CName::lido_do_motor(aparencia))
}

/// Corpo do `BwmsAparencia.ApplyAppearanceOverride`: `(log, escreveu)`. O Bool da native é o `escreveu`, nunca derivado do texto.
pub unsafe fn run_apply_appearance_override(reg: &crate::rtti::Registry, entity: *mut c_void, inst: *mut c_void) -> (String, bool) {
    let mut escreveu = false;
    let texto = aplicar_aparencia_texto(reg, entity, inst, &mut escreveu);
    (texto, escreveu)
}

/// O que o `BwmsAparencia.ApplyAppearanceOverride` decide antes de escrever.
pub(crate) struct Es22DecisaoDeAparencia {
    /// Override do nome base e do prefixo, para o log.
    pub(crate) fontes: (Option<u64>, Option<u64>),
    /// Aparência a escrever; `None` = sai sem escrever.
    pub(crate) aparencia: Option<u64>,
    /// Parte ligada ao nome base.
    pub(crate) parte: Option<u64>,
}

/// `original` só é chamada no ramo de restauração.
pub(crate) unsafe fn es22_decidir_aparencia(entidade: u64, inst: *const u8, original: impl FnOnce() -> u64) -> Es22DecisaoDeAparencia {
    let (base, estado, prefixo) = es20_estados_do_componente(entidade, inst);
    Es22DecisaoDeAparencia {
        fontes: (estado.as_ref().and_then(|s| s.aparencia_sobrescrita()), prefixo.as_ref().and_then(|s| s.aparencia_sobrescrita())),
        aparencia: bwms_primitives::override_state::aparencia_aplicada(original, estado.as_ref(), prefixo.as_ref()),
        parte: es22_parte_do_componente(entidade, base),
    }
}

/// Grava só com componente de mesh ligado, aparência decidida e o nome base ligado a uma parte que tem aparência ligada: sem o
/// `LinkComponentToPart`/`LinkPartToAppearanceName`, o override não tem a quem pertencer e o componente fica como o jogo deixou.
unsafe fn aplicar_aparencia_texto(reg: &crate::rtti::Registry, entity: *mut c_void, inst: *mut c_void, escreveu: &mut bool) -> String {
    *escreveu = false;
    if !rtti::sane(entity) || !rtti::sane(inst) {
        return format!("[aparencia] entidade {entity:p} ou componente {inst:p} insano");
    }
    if (inst as *const u8).add(0x8b).read() == 0 {
        return "[aparencia] componente desligado".to_string();
    }
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::Unsupported {
        return "[aparencia] componente não é de mesh".to_string();
    }
    let entidade = entity as u64;
    let base = es20_malhas(reg).map(Es20Malhas::base);
    let d = es22_decidir_aparencia(entidade, inst as *const u8, || base.map_or(0, |b| es20_original_da_aparencia(b, entidade, inst, kind)));
    let Some(aparencia) = d.aparencia else {
        return format!("[aparencia] fontes {:x?}: nada a gravar", d.fontes);
    };
    let Some(parte) = d.parte else {
        return format!("[aparencia] fontes {:x?}: nome base sem parte ligada", d.fontes);
    };
    let ligada = ES22_RESOURCE_STATE.lock().map(|g| g.iter().any(|(k, _)| *k == (entidade, parte))).unwrap_or(false);
    if !ligada {
        return format!("[aparencia] parte {parte:#x} sem aparência ligada");
    }
    let valor = match crate::cname::name_of(aparencia) {
        Some(texto) if is_dynamic_value(&texto) => {
            let resolvido = resolver_nome_da_entidade(reg, entity, &texto);
            if resolvido.is_empty() { 0 } else { crate::cname::registrar_no_pool(&resolvido) }
        }
        _ => aparencia,
    };
    if valor == 0 {
        return format!("[aparencia] nome dinâmico {aparencia:#x} resolveu vazio");
    }
    *escreveu = component_set_appearance_name(inst, kind, valor);
    let recarga = if *escreveu { es20_refresh_medido(inst) } else { None };
    if let Some(recarregar) = recarga {
        recarregar(inst);
    }
    format!(
        "[aparencia] fontes {:x?} parte {parte:#x} aparência {valor:#x} escreveu={} recarga={}",
        d.fontes,
        *escreveu,
        recarga.is_some()
    )
}

pub(crate) unsafe extern "C" fn tramp_aplicar_aparencia_do_componente(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let component = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    let mut detail = String::new();
    if let Some(reg) = crate::registry() {
        (detail, ok) = run_apply_appearance_override(reg, entity, component);
    }
    crate::log_dev!("[reg] >>> BwmsAparencia.ApplyAppearanceOverride(entity={entity:p}, component={component:p}) -> ok={ok} | {detail}");
    write_bool_ret(out, ok);
}

/// Corpo do `BwmsAparencia.ToggleConditionalComponents`: a native não passa estado da entidade, então critério com condição não casa.
pub unsafe fn run_toggle_conditional_components(reg: &crate::rtti::Registry, entity: *mut c_void) -> (String, i32) {
    run_toggle_conditional_components_com(reg, entity, None)
}

pub(crate) unsafe extern "C" fn tramp_alternar_componentes_condicionais(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let (detail, total) = match crate::registry() {
        Some(reg) => run_toggle_conditional_components(reg, entity),
        None => ("[togglecond] sem registry".to_string(), -1),
    };
    crate::log_dev!("[reg] >>> BwmsAparencia.ToggleConditionalComponents(entity={entity:p}) -> total={total} | {detail}");
    write_uint_ret(out, total.max(0) as u64, 4);
}

/// Corpo inteiro/torso (2026-08-07, cont.42, continuação): 2º agente de RE offline achou que
/// `+0x24a` é um ENUM de estado (0/1/2, tipo "fase de dissolve"), não um bool — `+0x24a==1`
/// satisfaz o gate de `vtable+0x2d0` mas BLOQUEIA explicitamente o de `vtable+0x2d8`
/// (`cmp w8,#2` em `0x1019911a4`, dentro de `0x1019910f4`). O manager pra onde o registro vai
/// (`world::RenderProxyDissolveHandler`, achado por símbolo vizinho) é um sistema de DISSOLVE/
/// fade, não um toggle binário — a progressão real provavelmente é 0→1→2 ao longo de vários
/// frames/dispatches, não uma escrita única. Esta função simula essa progressão: seta
/// `enabled=1`+`+0x253`bit2+`+0x24a=1`, chama o dispatcher (`0x101991450`) — depois seta
/// `+0x24a=2` e chama de NOVO, testando se o 2º dispatch (que exigia especificamente ==2)
/// dispara o resto da cadeia de dissolve.
#[cfg(feature = "devtools")]
pub unsafe fn run_toggle_dissolve_sequence(
    reg: &crate::rtti::Registry,
    player: *mut c_void,
    wanted_name: &str,
) -> String {
    if player.is_null() || !crate::memoria::is_readable(player as *const c_void, 0x10) {
        return "[dissolve] player nulo/ilegível".to_string();
    }
    let target_cls = reg.class_by_name("entSkinnedMeshComponent");
    if target_cls.is_null() {
        return "[dissolve] classe 'entSkinnedMeshComponent' não achada".to_string();
    }
    let hash_wanted = crate::cname::cname(wanted_name);
    let arr = (player as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return "[dissolve] player+0xA0 ilegível".to_string();
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size > 4096 {
        return format!("[dissolve] array inválido: entries={entries:p} size={size}");
    }
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if inst.is_null() {
            continue;
        }
        if !class_is_kind_of(crate::rtti::class_of(inst), target_cls) || !crate::memoria::is_readable(inst as *const c_void, 0x300) {
            continue;
        }
        let name_hash = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
        if name_hash != hash_wanted {
            continue;
        }
        type OneArgVoidFn = unsafe extern "C" fn(*mut c_void);
        let rebuild: OneArgVoidFn = core::mem::transmute(crate::rebase(0x1_0199_1450));

        core::ptr::write_unaligned((inst as *mut u8).add(0x8b), 1u8);
        let cur_253 = (inst as *const u8).add(0x25b).read_unaligned();
        core::ptr::write_unaligned((inst as *mut u8).add(0x25b), cur_253 | 0x04);
        core::ptr::write_unaligned((inst as *mut u8).add(0x252), 1u8);
        rebuild(inst);
        let after_pass1_24a = (inst as *const u8).add(0x252).read_unaligned();
        let after_pass1_12c = (inst as *const u8).add(0x12c).read_unaligned();

        core::ptr::write_unaligned((inst as *mut u8).add(0x252), 2u8);
        let cur_253_2 = (inst as *const u8).add(0x25b).read_unaligned();
        core::ptr::write_unaligned((inst as *mut u8).add(0x25b), cur_253_2 | 0x04);
        rebuild(inst);
        let after_pass2_24a = (inst as *const u8).add(0x252).read_unaligned();
        let after_pass2_12c = (inst as *const u8).add(0x12c).read_unaligned();
        let final_enabled = (inst as *const u8).add(0x8b).read_unaligned();
        let final_253 = (inst as *const u8).add(0x25b).read_unaligned();

        return format!(
            "[dissolve] '{wanted_name}' inst={inst:p} pass1(+0x24a={after_pass1_24a:#x},+0x12c={after_pass1_12c:#x}) pass2(+0x24a={after_pass2_24a:#x},+0x12c={after_pass2_12c:#x}) final(enabled={final_enabled},+0x253={final_253:#x})"
        );
    }
    format!("[dissolve] '{wanted_name}' (hash={hash_wanted:#x}) NÃO achado")
}

// ===== `RenderProxy` (2026-09-11) =====
// RE offline (DATABASE, seção `RenderProxy`): o proxy é o 1º qword de um SharedPtr no componente, num offset que depende da classe, e
// as funções do motor recebem o proxy em x0 (retorno tratado como void). `SetMaterialOverride` (`0x104fe59bc`) vem no fim do bloco,
// com o material de fallback do motor.
#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_VIS_BIT2_VM: u64 = 0x1_04fe_4ca0; // bit 0x02 de +0x23 (provável `SetVisibility`; o motor reescreve no recálculo)

#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_VIS_BIT4_VM: u64 = 0x1_04fe_4f3c; // bit 0x04 de +0x23 (provável `UpdateVisibility`; ninguém mais grava)

#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_HIGHLIGHT_VM: u64 = 0x1_04fe_57c4; // (proxy, const HighlightParams*), grava +0x21

#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_SCANNING_VM: u64 = 0x1_04fe_5898; // (proxy, u8), grava +0x22 se mudou

/// Offset do `renderProxy` pela classe do componente, a mais derivada primeiro. Na 2.5 os dois skinned andaram 8 bytes (2.31: `0x1e0`/`0x1d8`), pelos
/// chamadores do scanning do proxy `0x104fe5898` (`proofs/20261027-renderproxy-25-offsets-offline.log`).
pub(crate) unsafe fn renderproxy_offset(reg: &Registry, inst: *mut c_void) -> Option<usize> {
    let cls = rtti::class_of(inst);
    [
        ("entMorphTargetSkinnedMeshComponent", 0x1e8usize),
        ("entSkinnedMeshComponent", 0x1e0),
        ("entPhysicalDestructionComponent", 0x2f0),
        ("entMeshComponent", 0x140),
        ("entDecalComponent", 0x140),
    ]
    .into_iter()
    .find(|(nome, _)| {
        let alvo = reg.class_by_name(nome);
        !alvo.is_null() && class_is_kind_of(cls, alvo)
    })
    .map(|(_, off)| off)
}

/// O proxy vivo do componente: `None` se a classe não tem proxy conhecido ou se ele está nulo/ilegível.
pub(crate) unsafe fn renderproxy_de(inst: *mut c_void) -> Option<*mut c_void> {
    if !rtti::sane(inst) {
        return None;
    }
    let reg = crate::registry()?;
    let off = renderproxy_offset(reg, inst)?;
    if !crate::memoria::is_readable(inst as *const c_void, off + 8) {
        return None;
    }
    let proxy = rd_u64(inst, off) as *mut c_void;
    (!proxy.is_null() && crate::memoria::is_readable(proxy as *const c_void, 0x24)).then_some(proxy)
}

/// Componente (1º parâmetro) e o byte do proxy em `off`; `-1` sem proxy vivo.
pub(crate) unsafe fn renderproxy_byte(frame: *mut c_void, off: usize) -> i32 {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    renderproxy_de(inst).map_or(-1, |p| (p as *const u8).add(off).read_unaligned() as i32)
}

/// `BwmsRenderProxyFlags(component) -> Int32`: byte de flags `+0x23` (`IsVisible` = máscara 6 ligada); `-1` sem proxy.
pub(crate) unsafe extern "C" fn tramp_renderproxy_flags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, renderproxy_byte(frame, 0x23) as u32 as u64, 4);
}

/// `BwmsRenderProxyScanState(component) -> Int32`: estado de scan `+0x22`; `-1` sem proxy.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_scan_state(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, renderproxy_byte(frame, 0x22) as u32 as u64, 4);
}

/// `BwmsRenderProxyHighlight(component) -> Int32`: byte de highlight `+0x21` (os parâmetros codificados); `-1` sem proxy.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_highlight(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, renderproxy_byte(frame, 0x21) as u32 as u64, 4);
}

/// Visibilidade por uma das duas funções do motor (`vm`), com o bool do 2º parâmetro.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn renderproxy_visibilidade(frame: *mut c_void, out: *mut c_void, vm: u64, nome: &str) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let visivel = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(true);
    let ok = match renderproxy_de(inst) {
        Some(p) => {
            let f: unsafe extern "C" fn(*mut c_void, u32) = std::mem::transmute(crate::rebase(vm));
            f(p, visivel as u32);
            true
        }
        None => false,
    };
    crate::log_dev!("[renderproxy] {nome}(component={inst:p}, {visivel}) -> {ok}");
    write_bool_ret(out, ok);
}

/// `BwmsRenderProxySetVisibility(component, visible) -> Bool`: bit `0x02` de `+0x23` (`0x104fe4ca0`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_set_visibility(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    renderproxy_visibilidade(frame, out, RENDERPROXY_VIS_BIT2_VM, "SetVisibility");
}

/// `BwmsRenderProxyUpdateVisibility(component, visible) -> Bool`: bit `0x04` de `+0x23` (`0x104fe4f3c`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_update_visibility(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    renderproxy_visibilidade(frame, out, RENDERPROXY_VIS_BIT4_VM, "UpdateVisibility");
}

/// `BwmsRenderProxySetScanningState(component, state) -> Bool`: `Off=0`, `Scanning=2`, `Cancelled=3`, `Complete=4`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_set_scanning_state(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let estado = args.get(1).map(|(v, _)| *v as u32 & 0xff).unwrap_or(0);
    let ok = match renderproxy_de(inst) {
        Some(p) => {
            let f: unsafe extern "C" fn(*mut c_void, u32) = std::mem::transmute(crate::rebase(RENDERPROXY_SCANNING_VM));
            f(p, estado);
            true
        }
        None => false,
    };
    crate::log_dev!("[renderproxy] SetScanningState(component={inst:p}, {estado}) -> {ok}");
    write_bool_ret(out, ok);
}

/// `BwmsRenderProxySetHighlightParams(component, fillIndex, outlineIndex, opacity, seeThroughWalls) -> Bool`: monta o
/// `HighlightParams` como o motor lê (fill `+2`, outline `+3`, opacity `+4`, see-through `+8`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_set_highlight_params(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let fill = args.get(1).map(|(v, _)| *v as u8).unwrap_or(0);
    let outline = args.get(2).map(|(v, _)| *v as u8).unwrap_or(0);
    let opacidade = args.get(3).map(|(v, _)| f32::from_bits(*v as u32)).unwrap_or(0.0);
    let atraves = args.get(4).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let mut params = [0u8; 16];
    params[2] = fill;
    params[3] = outline;
    params[4..8].copy_from_slice(&opacidade.to_le_bytes());
    params[8] = atraves as u8;
    let ok = match renderproxy_de(inst) {
        Some(p) => {
            let f: unsafe extern "C" fn(*mut c_void, *const u8) = std::mem::transmute(crate::rebase(RENDERPROXY_HIGHLIGHT_VM));
            f(p, params.as_ptr());
            true
        }
        None => false,
    };
    crate::log_dev!(
        "[renderproxy] SetHighlightParams(component={inst:p}, fill={fill}, outline={outline}, opacity={opacidade}, atraves={atraves}) -> {ok}"
    );
    write_bool_ret(out, ok);
}

/// `BwmsRenderProxyFindComponent(entity) -> CName`: nome do 1º componente da entidade com proxy vivo (`None` se nenhum). Helper do
/// BWMS pra prova — o corpo do V em primeira pessoa não tem proxy.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_find_component(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let ent = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let (mut achado, mut total, mut com_proxy) = (0u64, 0usize, 0usize);
    let arr = (ent as *const u8).wrapping_add(0xA0) as *const c_void;
    if rtti::sane(ent) && crate::memoria::is_readable(arr, 0x10) {
        let entries = rd_u64(arr, 0) as *const u8;
        let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
        if !entries.is_null() && size > 0 && size <= 4096 {
            for i in 0..size as usize {
                let slot = entries.add(i * 16);
                if !crate::memoria::is_readable(slot as *const c_void, 16) {
                    continue;
                }
                let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
                if inst.is_null() || !crate::memoria::is_readable(inst as *const c_void, 0x48) {
                    continue;
                }
                total += 1;
                if renderproxy_de(inst).is_some() {
                    com_proxy += 1;
                    if achado == 0 {
                        achado = rd_u64(inst, 0x40);
                    }
                }
            }
        }
    }
    crate::veredito_dev!(
        "[renderproxy] FindComponent(entity={ent:p}) componentes={total} com_proxy={com_proxy} escolhido='{}'",
        crate::cname::resolve_cname(achado)
    );
    write_uint_ret(out, achado, 8);
}

// `SetMaterialOverride` (parte 3, 2026-09-11; RE em `proofs/20260911-*-254-setmaterialoverride-parte2-caminho-render-offline.log`):
// o motor enfileira o comando "SetMeshMaterialOverlay" e a thread de render grava o material na extensão
// `RenderProxyCustomData_MiscMeshParams` do objeto de render (`ro = [proxy+0x10]`, lista em `[ro+0x48]`, próximo nó em `+0x8`):
// `+0xd0` material, `+0xd8` alternativo (igual ao `+0xd0` quando o 2º `RenderDataPtr` vai nulo), `+0xe1` bit1 replace e bit0
// multilayered. A releitura é só de ponteiro, da thread de jogo, e acha a extensão pelo vptr (nunca chama o `vt[0x8]` dela).
#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_SET_MATERIAL_OVERRIDE_VM: u64 = 0x1_04fe_59bc; // (proxy, RenderDataPtr* material, RenderDataPtr* alternativo, u8 flag)

#[cfg(feature = "devtools")]
pub(crate) const RENDERDATA_FALLBACK_MATERIAL_VM: u64 = 0x1_04d5_1e78; // sret x8: RenderDataPtr do `engine\materials\fallback.remt` com AddRef; 0 sem carga

#[cfg(feature = "devtools")]
pub(crate) const MISCMESHPARAMS_VPTR_VM: u64 = 0x1_0783_fdd0;

#[cfg(feature = "devtools")]
pub(crate) const RENDERPROXY_RO_OFF: usize = 0x10;

#[cfg(feature = "devtools")]
pub(crate) const RENDEROBJ_EXT_LISTA_OFF: usize = 0x48;

#[cfg(feature = "devtools")]
pub(crate) const MISCMESHPARAMS_MATERIAL_OFF: usize = 0xd0;

#[cfg(feature = "devtools")]
pub(crate) const MISCMESHPARAMS_ALT_OFF: usize = 0xd8;

#[cfg(feature = "devtools")]
pub(crate) const MISCMESHPARAMS_FLAGS_OFF: usize = 0xe1;

/// Valor do último material enviado por `BwmsRenderProxySetFallbackMaterialOverride` (só pra comparar; a referência já foi solta).
#[cfg(feature = "devtools")]
pub(crate) static CW254_ULTIMO_MATERIAL: AtomicU64 = AtomicU64::new(0);

/// Extensão `MiscMeshParams` do proxy: `(material, alternativo, flags)`; `None` sem objeto de render ou sem a extensão na lista (a
/// cabeça da lista fica nula por um instante quando a thread de render insere um nó).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn renderproxy_material_override(proxy: *mut c_void) -> Option<(u64, u64, u8)> {
    let ro = rd_u64(proxy, RENDERPROXY_RO_OFF) as *mut c_void;
    if ro.is_null() || !crate::memoria::is_readable(ro as *const c_void, RENDEROBJ_EXT_LISTA_OFF + 8) {
        return None;
    }
    let vptr = crate::rebase(MISCMESHPARAMS_VPTR_VM) as u64;
    let mut no = rd_u64(ro, RENDEROBJ_EXT_LISTA_OFF) as *mut c_void;
    for _ in 0..64 {
        if no.is_null() || !crate::memoria::is_readable(no as *const c_void, 0x100) {
            return None;
        }
        if rd_u64(no, 0) == vptr {
            let flags = (no as *const u8).add(MISCMESHPARAMS_FLAGS_OFF).read_unaligned();
            return Some((rd_u64(no, MISCMESHPARAMS_MATERIAL_OFF), rd_u64(no, MISCMESHPARAMS_ALT_OFF), flags));
        }
        no = rd_u64(no, 0x8) as *mut c_void;
    }
    None
}

/// `SetMaterialOverride` do motor com os ponteiros dos dois `RenderDataPtr` (o motor faz AddRef no que enfileira).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn renderproxy_set_material_override(proxy: *mut c_void, material: u64, flag: u32) {
    let alternativo: u64 = 0;
    let f: unsafe extern "C" fn(*mut c_void, *const u64, *const u64, u32) =
        std::mem::transmute(crate::rebase(RENDERPROXY_SET_MATERIAL_OVERRIDE_VM));
    f(proxy, &material, &alternativo, flag);
}

/// `BwmsRenderProxyMaterialOverrideState(component) -> Int32`: `-1` sem proxy; senão bit0 extensão achada, bit1 material gravado,
/// bit2 material e alternativo iguais e não nulos, bits 8-9 os flags de `+0xe1` (`0x200` = replace).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_material_override_state(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let estado = match renderproxy_de(inst) {
        None => -1,
        Some(p) => match renderproxy_material_override(p) {
            None => 0,
            Some((mat, alt, flags)) => {
                crate::veredito_dev!(
                    "[cw254-mat] estado component={inst:p} proxy={p:p} material={mat:#x} alternativo={alt:#x} flags={flags:#04x}"
                );
                1 | (((mat != 0) as i32) << 1) | (((mat != 0 && mat == alt) as i32) << 2) | ((flags as i32 & 3) << 8)
            }
        },
    };
    if estado <= 0 {
        crate::veredito_dev!("[cw254-mat] estado component={inst:p} -> {estado} (sem proxy ou sem extensão)");
    }
    write_uint_ret(out, estado as u32 as u64, 4);
}

/// `BwmsRenderProxySetFallbackMaterialOverride(component, replace) -> Bool`: o material de fallback do motor por cima do componente
/// (`replace` = flag 1). `false` sem proxy ou com o `fallback.remt` descarregado — material nulo LIMPARIA o override em vez de aplicar.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_set_fallback_material_override(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let replace = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(true);
    let mut mat: u64 = 0;
    let ok = match renderproxy_de(inst) {
        Some(p) => {
            let slot: *mut u64 = &mut mat;
            core::arch::asm!(
                "blr {f}",
                f = in(reg) crate::rebase(RENDERDATA_FALLBACK_MATERIAL_VM),
                in("x8") slot,
                clobber_abi("C"),
            );
            if mat != 0 {
                renderproxy_set_material_override(p, mat, replace as u32);
                let soltar: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(crate::rebase(RENDERDATA_RELEASE_VM));
                soltar(mat as *mut c_void);
                CW254_ULTIMO_MATERIAL.store(mat, Ordering::Relaxed);
            }
            mat != 0
        }
        None => false,
    };
    crate::veredito_dev!("[cw254-mat] set component={inst:p} replace={replace} mat={mat:#x} -> {ok}");
    write_bool_ret(out, ok);
}

/// `BwmsRenderProxyClearMaterialOverride(component) -> Bool`: material nulo e flag 0 (o motor zera `+0xd0`/`+0xd8` e os flags).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_clear_material_override(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let ok = match renderproxy_de(inst) {
        Some(p) => {
            renderproxy_set_material_override(p, 0, 0);
            true
        }
        None => false,
    };
    crate::veredito_dev!("[cw254-mat] clear component={inst:p} -> {ok}");
    write_bool_ret(out, ok);
}

/// `BwmsRenderProxyLastOverrideMaterialMatches(component) -> Bool`: o material na extensão é o último enviado pelo BWMS.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_renderproxy_last_override_material_matches(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let ultimo = CW254_ULTIMO_MATERIAL.load(Ordering::Relaxed);
    let gravado = renderproxy_de(inst).and_then(|p| renderproxy_material_override(p)).map_or(0, |(mat, _, _)| mat);
    let ok = ultimo != 0 && gravado == ultimo;
    crate::veredito_dev!("[cw254-mat] confere component={inst:p} gravado={gravado:#x} enviado={ultimo:#x} -> {ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe fn register_renderproxy_natives(reg: &Registry, proto: *mut c_void) {
    let ok1 = {
        let p = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p) && register_global_composed(reg, proto, p, "BwmsRenderProxyFlags", "BwmsRenderProxyFlags", tramp_renderproxy_flags as NativeHandler)
    };
    let ok2 = {
        let p = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyScanState", "BwmsRenderProxyScanState", corpo_de_prova!(tramp_renderproxy_scan_state)))
    };
    let ok3 = {
        let p = compose_params_from_types(reg, &["handle:IComponent"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyHighlight", "BwmsRenderProxyHighlight", corpo_de_prova!(tramp_renderproxy_highlight)))
    };
    let ok4 = {
        let p = compose_params_from_types(reg, &["handle:IComponent", "Bool"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxySetVisibility", "BwmsRenderProxySetVisibility", corpo_de_prova!(tramp_renderproxy_set_visibility)))
    };
    let ok5 = {
        let p = compose_params_from_types(reg, &["handle:IComponent", "Bool"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyUpdateVisibility", "BwmsRenderProxyUpdateVisibility", corpo_de_prova!(tramp_renderproxy_update_visibility)))
    };
    let ok6 = {
        let p = compose_params_from_types(reg, &["handle:IComponent", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxySetScanningState", "BwmsRenderProxySetScanningState", corpo_de_prova!(tramp_renderproxy_set_scanning_state)))
    };
    let ok7 = {
        let p = compose_params_from_types(reg, &["handle:IComponent", "Int32", "Int32", "Float", "Bool"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxySetHighlightParams", "BwmsRenderProxySetHighlightParams", corpo_de_prova!(tramp_renderproxy_set_highlight_params)))
    };
    let ok8 = {
        let p = compose_params_from_types(reg, &["handle:Entity"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyFindComponent", "BwmsRenderProxyFindComponent", corpo_de_prova!(tramp_renderproxy_find_component)))
    };
    // `handle:IScriptable`, não `handle:VisionModeSystem`: o registro roda antes do bind dos scripts e o tipo handle do sistema só existe
    // depois (boot 40: `GetType` nulo). E sem gate de runtime: native declarado no reds e não registrado derruba o bind (SIGTRAP ~20 s).
    let ok9 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsVisionModeBraindanceMode", "BwmsVisionModeBraindanceMode", corpo_de_prova!(tramp_visionmode_braindance_mode)))
    };
    let ok10 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsVisionModeSetBraindanceMode", "BwmsVisionModeSetBraindanceMode", corpo_de_prova!(tramp_visionmode_set_braindance_mode)))
    };
    let ok11 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyMaterialOverrideState", "BwmsRenderProxyMaterialOverrideState", corpo_de_prova!(tramp_renderproxy_material_override_state)))
    };
    let ok12 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "Bool"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxySetFallbackMaterialOverride", "BwmsRenderProxySetFallbackMaterialOverride", corpo_de_prova!(tramp_renderproxy_set_fallback_material_override)))
    };
    let ok13 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyClearMaterialOverride", "BwmsRenderProxyClearMaterialOverride", corpo_de_prova!(tramp_renderproxy_clear_material_override)))
    };
    let ok14 = {
        let p = compose_params_from_types(reg, &["handle:IScriptable"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsRenderProxyLastOverrideMaterialMatches", "BwmsRenderProxyLastOverrideMaterialMatches", corpo_de_prova!(tramp_renderproxy_last_override_material_matches)))
    };
    crate::log_dev!(
        "[reg] register_all: cw-254 renderproxy Flags={ok1} ScanState={ok2} Highlight={ok3} SetVisibility={ok4} UpdateVisibility={ok5} SetScanningState={ok6} SetHighlightParams={ok7} FindComponent={ok8} | visionmode BraindanceMode={ok9} SetBraindanceMode={ok10} | material State={ok11} SetFallback={ok12} Clear={ok13} Matches={ok14}"
    );
}

/// `RegisterArchive(path: String) -> Bool` da fachada de recursos (`PENDENCIAS-UNIFICADAS.md`) —
/// achado de 2026-07-13 nunca implementado até agora: generaliza a injeção de archive arbitrário (rota B)
/// (já PROVADO ao vivo, `lib.rs::load_archive_into_group`) numa API chamável por QUALQUER mod em
/// runtime (ex. `OnAttach`), sem precisar que o nome do `.archive` bata o glob nativo.
pub(crate) unsafe extern "C" fn tramp_registrar_archive(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::arquivos_do_jogo::facade_register_archive(&path);
    crate::log_dev!("[reg] >>> RegisterArchive('{path}') -> {ok}");
    write_bool_ret(out, ok);
}

/// `RegisterDir(path: String) -> Bool` da fachada de recursos — registra todos os `.archive` de um diretório
/// (não-recursivo) via `facade_register_archive` um por um.
pub(crate) unsafe extern "C" fn tramp_registrar_pasta_de_archives(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::arquivos_do_jogo::facade_register_dir(&path);
    crate::log_dev!("[reg] >>> RegisterDir('{path}') -> {ok}");
    write_bool_ret(out, ok);
}

/// `RegisterLink(path, link) -> Bool` (`PENDENCIAS-UNIFICADAS.md`, 2026-08-11):
/// via "oficial" redscript-callable de registrar 1 par de `resource.link` em runtime, mesma
/// tabela (`RESLINK_MAP`) que `resource.link`/`InitializeArchives` já usam — reusa
/// `selftest::reslink_path` (mecanismo PROVADO ao vivo desde 2026-07-13, aplicação de
/// localização/etc.), zero RE nova. `path` = o alias que um mod quer que resolva pra `link` (o
/// recurso REAL). Sempre devolve `true` se os 2 argumentos forem strings válidas (mesma
/// convenção de `tweakdb_clone_record` — aceito pra processamento, não confirmação de troca
/// real; ver `reslinkstat` pra confirmar swaps de verdade acontecendo).
pub(crate) unsafe extern "C" fn tramp_registrar_alias(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let link = strings.get(1).cloned().flatten().unwrap_or_default();
    crate::arquivos_do_jogo::reslink_path(&path, &link);
    crate::log_dev!("[reg] >>> BwmsAparencia.RegisterLink('{path}' -> '{link}') -> true");
    write_bool_ret(out, true);
}

/// Metade leitura dos aliases: `GetAliasCount(path) -> Int32` — quantos caminhos são o mesmo
/// recurso que `path`, nos dois sentidos, por `resource.link`/`RegisterLink` e por `resource.copy`
/// (`reslink_aliases_of`). Composto com `GetAliasHash` abaixo pra enumerar sem precisar de um
/// `array<T>`-retorno-de-native (a
/// categoria de risco que já CROU este projeto — `GetRecordsArray`, 2026-08-10 —
/// evitada de propósito aqui: 2 natives escalares em vez de 1 native com array de saída).
pub(crate) unsafe extern "C" fn tramp_contar_aliases(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let hash = bwms_primitives::resource_path_hash(&path);
    let n = crate::arquivos_do_jogo::reslink_aliases_of(hash).len();
    crate::log_dev!("[reg] >>> BwmsAparencia.GetAliasCount('{path}') -> {n}");
    write_uint_ret(out, n as u64, 4);
}

/// `GetAliasHash(path, index) -> Uint64` — devolve o HASH (não o path decodificado
/// — `ResourcePath` é hash-only, mesma divergência já aceita em `BwmsHashToResRef`/etc.) do
/// alias no índice `index` (0-based) dentre os de `GetAliasCount`, em ordem crescente de hash.
/// `index` fora do alcance devolve `0`.
pub(crate) unsafe extern "C" fn tramp_hash_do_alias(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let index = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let hash = bwms_primitives::resource_path_hash(&path);
    let aliases = crate::arquivos_do_jogo::reslink_aliases_of(hash);
    let v = if index >= 0 { aliases.get(index as usize).copied().unwrap_or(0) } else { 0 };
    crate::log_dev!("[reg] >>> BwmsAparencia.GetAliasHash('{path}', {index}) -> {v:#018x}");
    write_uint_ret(out, v, 8);
}

/// `Reload() -> Void` da fachada de recursos — re-lê `bwms-reslink.txt`/`bwms-factories.txt` e as outras tabelas
/// geradas por `bwms install` a partir dos `.xl` ativos (`recarregar_tabelas_de_runtime`) sem precisar rebootar,
/// mesmo padrão já fechado do `Reload` dos tweaks (reusa `apply_mod_tweaks()`). Útil depois de editar
/// um `.xl` de mod ativo em disco + regerar as tabelas via `bwms install`. As tabelas em memória viram as dos arquivos
/// (`substituir`): o par de `RegisterLink` sai.
pub(crate) unsafe extern "C" fn tramp_recarregar_tabelas_de_runtime(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log_dev!("[reg] >>> Reload chamado — recarregando reslink/factory/patch");
    crate::mods_do_usuario::pipeline::recarregar_tabelas_de_runtime(true);
}

/// `GetBodyType(puppet)` da fachada de recursos (2026-08-11, achado direto após fechar #32):
/// sem tags de corpo devolve o corpo base — as tags só vêm do
/// `.xl` `player.bodyTypes`, nunca wired num runtime consumido por este projeto, logo SEMPRE
/// vazias aqui — `GetBodyType` é DETERMINÍSTICO, sempre devolve `"BaseBody"`,
/// independente do puppet passado. Zero endereço nativo
/// novo, zero RE — mesma bookkeeping-closure category já usada váras vezes nesta sessão.
/// `puppet` (arg) é ignorado de propósito (a precondição do projeto já garante o resultado).
pub(crate) unsafe extern "C" fn tramp_tipo_de_corpo(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // Drena o param `puppet` do frame (mesmo se não usado) — mesma disciplina de qualquer
    // native argful deste projeto, evita deixar o cursor da CScriptStackFrame desalinhado.
    let func = current_native_func();
    let _ = rtti::read_params_consuming(func, frame);
    let h = crate::cname::intern("BaseBody");
    crate::log_dev!("[reg] >>> GetBodyType chamado -> 'BaseBody' (determinístico, tags de corpo sempre vazias neste projeto)");
    write_uint_ret(out, h, 8);
}

/// `BwmsAparencia.IsDynamicValue(value: String) -> Bool`: o texto começa com o marcador `*`.
pub(crate) unsafe extern "C" fn tramp_valor_e_dinamico(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let value = strings.first().cloned().flatten().unwrap_or_default();
    let dynamic = is_dynamic_value(&value);
    crate::log_dev!("[reg] >>> BwmsAparencia.IsDynamicValue('{value}') -> {dynamic}");
    write_bool_ret(out, dynamic);
}

/// `BwmsAparencia.ProcessDynamicString(input: String, attrName: String, attrValue: String) -> String` —
/// processa `input` substituindo `{attrName}` por `attrValue`; devolve string vazia se `input` não
/// for dinâmico/processamento inválido/atributo não bater (mesma convenção de retorno "vazio =
/// falhou" já usada por outras natives de string deste projeto, ex. `BwmsUTF8Str*`). Escapa cedo se
/// `input` nem começa com `*`, devolvendo o próprio `input` inalterado (mesmo par `{aPath,false}` já
/// documentado em `expand_resource_path`/item `#48`).
pub(crate) unsafe extern "C" fn tramp_processar_texto_dinamico(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let input = strings.first().cloned().flatten().unwrap_or_default();
    let attr_name = strings.get(1).cloned().flatten().unwrap_or_default();
    let attr_value = strings.get(2).cloned().flatten().unwrap_or_default();
    if !is_dynamic_value(&input) {
        let ok = write_cstring_inline_ret(out, &input);
        crate::log_dev!("[reg] >>> BwmsAparencia.ProcessDynamicString('{input}',...) -> não-dinâmico, devolve inalterado (escrito={ok})");
        return;
    }
    let result = process_dynamic_string_single_attr(&attr_name, &attr_value, &input);
    let r = if result.valid && !result.missed { result.value.as_str() } else { "" };
    let ok = write_cstring_inline_ret(out, r);
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.ProcessDynamicString('{input}', attr='{attr_name}'='{attr_value}') -> '{r}' (valid={} missed={} optional={} escrito={ok})",
        result.valid, result.missed, result.optional
    );
}

/// `DynamicAppearanceController::ResolveName`/`ResolvePath` pro redscript, com assinatura reduzida
/// `(entity, valor)`: o estado dinâmico da entidade (sufixos, customização e cor do cabelo) é calculado na hora por
/// `selftest::resolver_nome_da_entidade`, e o `ProcessString` troca os atributos (`*t_{gender}_{camera}`). Sem variante
/// (locais vazios) e sem item equipado no estado.
pub(crate) unsafe extern "C" fn tramp_resolver_nome_dinamico(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let name = strings.get(1).cloned().flatten().unwrap_or_default();
    let resolvido = match crate::registry() {
        Some(reg) if rtti::sane(entity) => crate::aparencia::resolver_nome_da_entidade(reg, entity, &name),
        _ => name.clone(),
    };
    let ok = write_cstring_inline_ret(out, &resolvido);
    crate::log_dev!("[reg] >>> BwmsAparencia.ResolveName(entity={entity:p}, name='{name}') -> '{resolvido}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_resolver_caminho_dinamico(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let path_str = strings.get(1).cloned().flatten().unwrap_or_default();
    let resolvido = match crate::registry() {
        Some(reg) if rtti::sane(entity) => crate::aparencia::resolver_nome_da_entidade(reg, entity, &path_str),
        _ => path_str.clone(),
    };
    let ok = write_cstring_inline_ret(out, &resolvido);
    crate::log_dev!("[reg] >>> BwmsAparencia.ResolvePath(entity={entity:p}, path='{path_str}') -> '{resolvido}' (escrito={ok})");
}

/// Nomes que o motor reserva na troca de aparência: quem pede um deles não pede uma aparência própria.
const NOMES_DE_APARENCIA_RESERVADOS: [u64; 3] = [
    crate::cname::cname("default"),
    crate::cname::cname("random"),
    crate::cname::cname("empty_appearance_default"),
];

pub(crate) fn is_unique_appearance_name(hash: u64) -> bool {
    hash != 0 && !NOMES_DE_APARENCIA_RESERVADOS.contains(&hash)
}

/// `BwmsAparencia.IsUniqueAppearanceName(name: CName) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_aparencia_e_unica(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let unique = is_unique_appearance_name(hash);
    crate::log_dev!("[reg] >>> BwmsAparencia.IsUniqueAppearanceName(hash={hash:#018x}) -> {unique}");
    write_bool_ret(out, unique);
}

pub(crate) fn get_base_appearance_name(name: &str) -> &str {
    name.find(['!', '%', '&']).map_or(name, |i| &name[..i])
}

/// `BwmsAparencia.GetBaseAppearanceName(name: String) -> String`.
pub(crate) unsafe extern "C" fn tramp_nome_base_da_aparencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let base = get_base_appearance_name(&name);
    let ok = write_cstring_inline_ret(out, base);
    crate::log_dev!("[reg] >>> BwmsAparencia.GetBaseAppearanceName('{name}') -> '{base}' (escrito={ok})");
}

pub(crate) fn appearance_matches_reference(reference: &DynamicAppearanceRef, appearance: &DynamicAppearanceName) -> bool {
    appearance_matches_reference_com(reference, appearance, None)
}

/// O nome base não entra: o critério casa pela variante e pelas condições. Sem estado da entidade, critério com condição não casa.
pub(crate) fn appearance_matches_reference_com(
    reference: &DynamicAppearanceRef,
    appearance: &DynamicAppearanceName,
    condicoes: Option<&std::collections::BTreeSet<u64>>,
) -> bool {
    if !reference.variants.is_empty() && !reference.matches_variant(appearance.variant) {
        return false;
    }
    if reference.conditions.is_empty() {
        return true;
    }
    condicoes.is_some_and(|c| reference.matches_conditions_with_overrides(c, &appearance.overrides))
}

/// `BwmsAparencia.IsDynamicAppearanceName(name: String) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_aparencia_e_dinamica(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let n = DynamicAppearanceName::parse(&name);
    crate::log_dev!("[reg] >>> BwmsAparencia.IsDynamicAppearanceName('{name}') -> {}", n.is_dynamic);
    write_bool_ret(out, n.is_dynamic);
}

/// `BwmsAparencia.AppearanceNameBase(name: String) -> Uint64` — o hash do NOME (parte antes de `!`;
/// a aparência inteira se não houver marcador nenhum).
pub(crate) unsafe extern "C" fn tramp_hash_do_nome_da_aparencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let n = DynamicAppearanceName::parse(&name);
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceNameBase('{name}') -> base={:#018x}", n.name);
    write_uint_ret(out, n.name, 8);
}

/// `BwmsAparencia.AppearanceNameVariant(name: String) -> Uint64` — o hash da variante (trecho
/// inteiro após `!`, ANTES de qualquer separação por `+`) — `0` se não-dinâmico.
pub(crate) unsafe extern "C" fn tramp_hash_da_variante_da_aparencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let n = DynamicAppearanceName::parse(&name);
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceNameVariant('{name}') -> variant={:#018x}", n.variant);
    write_uint_ret(out, n.variant, 8);
}

/// `BwmsAparencia.AppearanceNameContext(name: String) -> Uint64` — tag numérica de contexto
/// (`%numero`, só parseia se for o FINAL LITERAL da string — achado de RE documentado no parser).
pub(crate) unsafe extern "C" fn tramp_contexto_da_aparencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let n = DynamicAppearanceName::parse(&name);
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceNameContext('{name}') -> context={}", n.context);
    write_uint_ret(out, n.context, 8);
}

/// `BwmsAparencia.AppearanceMatchesReference(appearanceName: String, reference: String) -> Bool` —
/// composição de `DynamicAppearanceName::parse`+`DynamicAppearanceRef::parse`+
/// `MatchReference` (sem estado de entidade acoplado, ver `appearance_matches_reference`).
pub(crate) unsafe extern "C" fn tramp_aparencia_casa_referencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let appearance_name = strings.first().cloned().flatten().unwrap_or_default();
    let reference = strings.get(1).cloned().flatten().unwrap_or_default();
    let n = DynamicAppearanceName::parse(&appearance_name);
    let r = DynamicAppearanceRef::parse(&reference);
    let matched = appearance_matches_reference(&r, &n);
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.AppearanceMatchesReference('{appearance_name}','{reference}') -> {matched}"
    );
    write_bool_ret(out, matched);
}

/// `BwmsAparencia.IsDynamicAppearanceRef(reference: String) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_referencia_e_dinamica(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let reference = strings.first().cloned().flatten().unwrap_or_default();
    let r = DynamicAppearanceRef::parse(&reference);
    crate::log_dev!("[reg] >>> BwmsAparencia.IsDynamicAppearanceRef('{reference}') -> {}", r.is_dynamic);
    write_bool_ret(out, r.is_dynamic);
}

/// `BwmsAparencia.AppearanceRefName(reference: String) -> Uint64` — o hash do NOME (parte antes de
/// `!`/`&`; a referência inteira se não houver marcador nenhum).
pub(crate) unsafe extern "C" fn tramp_hash_do_nome_da_referencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let reference = strings.first().cloned().flatten().unwrap_or_default();
    let r = DynamicAppearanceRef::parse(&reference);
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceRefName('{reference}') -> name={:#018x}", r.name);
    write_uint_ret(out, r.name, 8);
}

/// `BwmsAparencia.AppearanceRefWeight(reference: String) -> Int32` — peso de desempate (100 se tem
/// variante, +1 por condição, truncado em `int8`) usado pra escolher entre
/// múltiplos `DynamicAppearanceRef` candidatos batendo o mesmo contexto.
pub(crate) unsafe extern "C" fn tramp_peso_da_referencia(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let reference = strings.first().cloned().flatten().unwrap_or_default();
    let r = DynamicAppearanceRef::parse(&reference);
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceRefWeight('{reference}') -> weight={}", r.weight);
    write_uint_ret(out, r.weight as u64, 4);
}

/// `BwmsAparencia.AppearanceRefMatchesVariant(reference: String, variant: String) -> Bool`.
pub(crate) unsafe extern "C" fn tramp_referencia_casa_variante(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let reference = strings.first().cloned().flatten().unwrap_or_default();
    let variant = strings.get(1).cloned().flatten().unwrap_or_default();
    let r = DynamicAppearanceRef::parse(&reference);
    let matched = r.matches_variant(bwms_primitives::fnv1a64(variant.as_bytes()));
    crate::log_dev!("[reg] >>> BwmsAparencia.AppearanceRefMatchesVariant('{reference}','{variant}') -> {matched}");
    write_bool_ret(out, matched);
}

/// `ExpandResourcePath` do mesh (2026-08-12 round 3) — reusa o núcleo `is_dynamic_value`/
/// `process_dynamic_string_single_attr` (já wired ao dylib mais cedo hoje) com
/// o atributo LOCAL pré-computado `material`, então o núcleo
/// é o de `process_dynamic_string_com`, sem cópia. Par
/// `(ResourcePath, bool)`:
/// - `pathStr` não-dinâmico → devolve `pathStr` inalterado (`{aPath, false}`);
/// - processamento inválido (chave sem fechar/nenhum atributo na string) → devolve string vazia
///   (convenção "vazio = falhou" já usada em `ProcessDynamicString`);
/// - atributo faltando (`missed`) → devolve string vazia (mesma convenção);
/// - sucesso → devolve o path expandido.
/// Escopo consciente: em vez de internar o resultado num registro global nativo de caminhos, devolvemos a STRING
/// final — o chamador registra o hash pelo mecanismo já provado do BWMS (`resource.link`/
/// `BwmsAparencia.RegisterLink`, ambos já expostos ao redscript).
pub(crate) unsafe extern "C" fn tramp_expandir_caminho_do_recurso(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path_str = strings.first().cloned().flatten().unwrap_or_default();
    let material_name = strings.get(1).cloned().flatten().unwrap_or_default();
    if !is_dynamic_value(&path_str) {
        let ok = write_cstring_inline_ret(out, &path_str);
        crate::log_dev!(
            "[reg] >>> BwmsAparencia.ExpandResourcePath('{path_str}', material='{material_name}') -> não-dinâmico, devolve inalterado (escrito={ok})"
        );
        return;
    }
    let result = process_dynamic_string_single_attr("material", &material_name, &path_str);
    let r = if result.valid && !result.missed { result.value.as_str() } else { "" };
    let ok = write_cstring_inline_ret(out, r);
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.ExpandResourcePath('{path_str}', material='{material_name}') -> '{r}' (valid={} missed={} optional={} escrito={ok})",
        result.valid, result.missed, result.optional
    );
}

/// `BwmsAparencia.AddOffsetOverride(entity: wref<Entity>, modHash: Uint64, resourcePath: Uint64,
/// offset: Int32) -> Bool` — ZERO mutação de memória nativa (só escreve no registry próprio do
/// BWMS) — mesma categoria de risco ZERO dos outros `Add*Override` do item `#22` acima.
pub(crate) unsafe extern "C" fn tramp_adicionar_offset_de_roupa(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mod_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let resource_path = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let offset = args.get(3).map(|(v, _)| *v as i32).unwrap_or(0);
    let ok = rtti::sane(entity);
    if ok {
        add_offset_override(entity as u64, mod_hash, resource_path, offset);
    }
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.AddOffsetOverride(entity={entity:p}, modHash={mod_hash:#018x}, resourcePath={resource_path:#018x}, offset={offset}) -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

/// `BwmsAparencia.RemoveOffsetOverrides(entity: wref<Entity>, modHash: Uint64) -> Uint32` — devolve a
/// contagem de recursos que tinham o mod: confirma o round-trip num teste ao vivo sem precisar de um 2º comando.
pub(crate) unsafe extern "C" fn tramp_remover_offsets_de_roupa_do_mod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mod_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let n = if rtti::sane(entity) { remove_offset_overrides(entity as u64, mod_hash) } else { 0 };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.RemoveOffsetOverrides(entity={entity:p}, modHash={mod_hash:#018x}) -> recursos afetados={n}"
    );
    write_uint_ret(out, n as u64, 4);
}

/// `BwmsAparencia.GetOffsetOverride(entity: wref<Entity>, resourcePath: Uint64) -> Int32` — leitura
/// pura, zero mutação, confirma o round-trip Add/Remove num teste ao vivo.
pub(crate) unsafe extern "C" fn tramp_offset_de_roupa(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let resource_path = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let v = if rtti::sane(entity) { get_offset_override(entity as u64, resource_path) } else { 0 };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.GetOffsetOverride(entity={entity:p}, resourcePath={resource_path:#018x}) -> {v}"
    );
    write_i32_ret(out, v);
}

/// Os 8 métodos da fachada de recursos, o nome que mod do Windows chama. Declarados em
/// `ponte-fachada-de-recursos.reds`.
const METODOS_DA_FACHADA_DE_RECURSOS: [(&str, NativeHandler); 8] = [
    ("Version", tramp_version::<PLUGUE_RECURSOS>),
    ("Require", tramp_require::<PLUGUE_RECURSOS>),
    ("RegisterArchive", tramp_registrar_archive),
    ("RegisterDir", tramp_registrar_pasta_de_archives),
    ("Reload", tramp_recarregar_tabelas_de_runtime),
    ("EnableGarmentOffsets", tramp_offsets_de_roupa_ligar),
    ("DisableGarmentOffsets", tramp_offsets_de_roupa_desligar),
    ("GetBodyType", tramp_tipo_de_corpo),
];

pub unsafe fn registrar_fachada_de_recursos(reg: &Registry) -> String {
    register_facade_methods(reg, "ArchiveXL", &METODOS_DA_FACHADA_DE_RECURSOS)
}

/// As funções de aparência que o BWMS criou (aliases de `resource.link`, aparência e caminho dinâmicos, tags visuais, override de
/// aparência por componente e deslocamento de roupa). Nenhum mod do Windows as chama, então moram
/// numa classe própria, forjada pela linha dela em `CLASSES_FORJADAS`. Declaradas em `bwms-aparencia.reds`.
const METODOS_BWMSAPARENCIA: [(&str, NativeHandler); 33] = [
    ("RegisterLink", tramp_registrar_alias),
    ("GetAliasCount", tramp_contar_aliases),
    ("GetAliasHash", tramp_hash_do_alias),
    ("IsDynamicValue", tramp_valor_e_dinamico),
    ("ProcessDynamicString", tramp_processar_texto_dinamico),
    ("ResolveName", tramp_resolver_nome_dinamico),
    ("ResolvePath", tramp_resolver_caminho_dinamico),
    ("GetVisualTagOverrideCount", tramp_contar_componentes_da_tag),
    ("ApplyVisualTagOverride", tramp_aplicar_tag_visual),
    ("ApplyVisualTagOverrideForMod", tramp_aplicar_tag_visual_do_mod),
    ("RemoveVisualTagOverrideForMod", tramp_remover_tag_visual_do_mod),
    ("ApplyVisualTagOverrideForModWithRefresh", tramp_aplicar_tag_visual_do_mod_com_refresh),
    ("RemoveVisualTagOverrideForModWithRefresh", tramp_remover_tag_visual_do_mod_com_refresh),
    ("IsUniqueAppearanceName", tramp_aparencia_e_unica),
    ("IsDynamicAppearanceRef", tramp_referencia_e_dinamica),
    ("AppearanceRefName", tramp_hash_do_nome_da_referencia),
    ("AppearanceRefWeight", tramp_peso_da_referencia),
    ("AppearanceRefMatchesVariant", tramp_referencia_casa_variante),
    ("ExpandResourcePath", tramp_expandir_caminho_do_recurso),
    ("IsDynamicAppearanceName", tramp_aparencia_e_dinamica),
    ("AppearanceNameBase", tramp_hash_do_nome_da_aparencia),
    ("AppearanceNameVariant", tramp_hash_da_variante_da_aparencia),
    ("AppearanceNameContext", tramp_contexto_da_aparencia),
    ("AppearanceMatchesReference", tramp_aparencia_casa_referencia),
    ("GetBaseAppearanceName", tramp_nome_base_da_aparencia),
    // `#22`: `LinkComponentToPart`/`LinkPartToAppearanceName`/`AddAppearanceOverrideForComponent` montam as ligações e a loja que
    // `ApplyAppearanceOverride`/`ToggleConditionalComponents` consomem.
    ("LinkComponentToPart", tramp_ligar_componente_a_parte),
    ("LinkPartToAppearanceName", tramp_ligar_parte_a_aparencia),
    ("AddAppearanceOverrideForComponent", tramp_adicionar_aparencia_do_componente),
    ("ApplyAppearanceOverride", tramp_aplicar_aparencia_do_componente),
    ("ToggleConditionalComponents", tramp_alternar_componentes_condicionais),
    // `#67`: offsets de roupa por mod na loja do BWMS (`ES67_RESOURCE_OFFSET`); o gancho do `ComputePlayerGarment` lê dela.
    ("AddOffsetOverride", tramp_adicionar_offset_de_roupa),
    ("RemoveOffsetOverrides", tramp_remover_offsets_de_roupa_do_mod),
    ("GetOffsetOverride", tramp_offset_de_roupa),
];

pub unsafe fn register_bwmsaparencia(reg: &Registry) -> String {
    register_facade_methods(reg, "BwmsAparencia", &METODOS_BWMSAPARENCIA)
}

#[cfg(test)]
mod testes_do_contexto_do_mesh {
    /// A `std::string` da libc++ que o `PrefillContext` copia (S12a §2): curta no lugar com o tamanho em `+0x17`; longa com o bit 63 da capacidade.
    #[test]
    fn string_da_libcxx_curta_e_longa() {
        let curta = std::ffi::CString::new("dread").unwrap();
        let s = super::string_da_libcxx(&curta);
        assert_eq!(&s[..5], b"dread");
        assert_eq!(s[23], 5);
        let longa = std::ffi::CString::new("base\\characters\\common\\hair\\x.mi").unwrap();
        let l = super::string_da_libcxx(&longa);
        assert_eq!(u64::from_le_bytes(l[..8].try_into().unwrap()), longa.as_ptr() as u64);
        assert_eq!(u64::from_le_bytes(l[8..16].try_into().unwrap()), longa.as_bytes().len() as u64);
        assert_eq!(l[23] & 0x80, 0x80);
        let limite = std::ffi::CString::new("a".repeat(22)).unwrap();
        assert_eq!(super::string_da_libcxx(&limite)[23], 22);
    }
}

#[cfg(test)]
mod testes_das_fachadas {
    use super::{METODOS_DA_FACHADA_DE_RECURSOS, METODOS_BWMSAPARENCIA};
    use std::collections::BTreeSet;

    /// Os `static native func` de `class <classe>` num `.reds` do bundle de dev. Uma classe por arquivo, sem `{` aninhado no corpo.
    fn declaradas(arquivo: &str, classe: &str) -> Option<BTreeSet<String>> {
        let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().map(|d| d.join("blackwall-mods-dev")).find(|p| p.is_dir())?;
        let caminho = dev.join(arquivo);
        let texto = std::fs::read_to_string(&caminho).unwrap_or_else(|e| panic!("{caminho:?} ilegível ({e}): o registro de `{classe}` aponta para ele"));
        let cabeca = format!("public abstract native class {classe} {{");
        let corpo = texto.split_once(&cabeca).unwrap_or_else(|| panic!("{caminho:?} sem `{cabeca}`")).1;
        let corpo = corpo.split_once("\n}").map_or(corpo, |(c, _)| c);
        Some(
            corpo
                .lines()
                .filter_map(|l| l.trim().strip_prefix("public static native func "))
                .map(|r| r.split('(').next().unwrap_or(r).to_string())
                .collect(),
        )
    }

    fn registradas(metodos: &[(&str, crate::register::NativeHandler)]) -> BTreeSet<String> {
        metodos.iter().map(|(n, _)| n.to_string()).collect()
    }

    /// Na fachada de recursos ficam só os 8 métodos que mods chamam; o que o BWMS criou vai para `BwmsAparencia`.
    #[test]
    fn fachada_de_recursos_so_com_os_metodos_de_contrato() {
        let contrato: BTreeSet<String> =
            ["Version", "Require", "RegisterArchive", "RegisterDir", "Reload", "EnableGarmentOffsets", "DisableGarmentOffsets", "GetBodyType"]
                .iter()
                .map(|n| n.to_string())
                .collect();
        assert_eq!(registradas(&METODOS_DA_FACHADA_DE_RECURSOS), contrato);
        let bwms = registradas(&METODOS_BWMSAPARENCIA);
        assert_eq!(bwms.len(), METODOS_BWMSAPARENCIA.len(), "nome repetido em `BwmsAparencia`");
        assert!(bwms.is_disjoint(&contrato), "método da fachada de recursos registrado em `BwmsAparencia`: {:?}", bwms.intersection(&contrato).collect::<Vec<_>>());
    }

    /// Native declarada no `.reds` e não registrada derruba o bind (SIGTRAP); registrada e não declarada fica inalcançável. Sem a pasta
    /// `blackwall-mods-dev/` em nenhum ancestral (a suíte do export para o GitHub), não há o que comparar.
    #[test]
    fn declaradas_no_reds_batem_com_as_registradas() {
        for (arquivo, classe, metodos) in [
            ("ponte-fachada-de-recursos.reds", "ArchiveXL", &METODOS_DA_FACHADA_DE_RECURSOS[..]),
            ("bwms-aparencia.reds", "BwmsAparencia", &METODOS_BWMSAPARENCIA[..]),
        ] {
            let Some(no_reds) = declaradas(arquivo, classe) else { return };
            assert_eq!(no_reds, registradas(metodos), "{arquivo}: `{classe}` declarada × registrada");
        }
    }
}

/// Alguém ouve `Component/Toggle`/`Vehicle/ToggleAuxLights`: os hooks do `Toggle` e do `ToggleLights` só leem estes atômicos.
pub static COMPONENT_TOGGLE_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `Component/Toggle` com o componente e a entidade dona, só com dono. O evento sai do
/// estado e o handle é solto depois do despacho, como no ciclo de vida.
pub unsafe fn disparar_toggle_de_componente(componente: *mut c_void) -> usize {
    let dono = dono_do_componente(componente);
    if dono == 0 {
        return 0;
    }
    let Some(arg) = make_entitycomponentevent_arg(componente, dono) else { return 0 };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args("Component/Toggle", &[arg]);
    if let Ok(mut states) = ENTITYCOMPONENT_STATES.lock() {
        states.retain(|e| e.0 as u64 != inst);
    }
    let mut h = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut h);
    enviados
}

// ===== `ComponentTarget extends CallbackSystemTarget` — fábricas STATIC =====
//
// Forma: `public static native func ID(id: CRUID)
// -> ref<ComponentTarget>` / `Name(name: CName) -> ref<ComponentTarget>`. Cada fábrica lê 1 arg
// escalar (CRUID/CName = u64 puro, mesmo tratamento de `register_hash_wrapper_utils`) e devolve
// uma instância NOVA da PRÓPRIA classe carregando o critério (consumido depois pelo filtro
// "permissivo" de `CallbackSystemHandler.AddTarget`/`HasTarget` — não filtra fundo hoje).
//
// Mecanismos que as fábricas reusam, cada um com prova própria:
//   (1) `is_static=true` LENDO um arg via `read_params_consuming` dentro de um MÉTODO DE CLASSE
//       real registrado por `register_method(..., true)` — `Require(version: String)` da fachada de versão
//       (`tramp_require`, `registrar_fachada_de_versao`), PROVADO ao vivo (fachada fechada 2026-07-13,
//       boot completo até GAMEPLAY REAL). MAS a classe da fachada de versão vem do `.reds` (compilador
//       real cria a CClass) — NÃO é uma classe que NÓS forjamos via `register_type_instantiable_
//       with_parent`.
//   (2) uma trampolina que LÊ um arg, CONSTRÓI uma instância NOVA (`rtti::new_object`), guarda o
//       critério NUM REGISTRY POR PONTEIRO (`Mutex<Vec<(usize, u64)>>`, mesmo formato de
//       `RESOURCEEVENT_STATES`) e devolve via `rtti::make_handle` — `ScriptableServiceContainer.
//       GetService(name)`, PROVADO ao vivo (boot→GAMEPLAY, `callon`,
//       2026-07-18). Essa trampolina nem LÊ o `c` (self) — primeiro parâmetro é `_c`, descartado —
//       então o caminho JÁ PROVADO nunca dependeu de `c` ser uma instância válida. Isso é evidência
//       forte (não prova direta) de que `c` sendo null/sem sentido numa chamada STATIC não quebra
//       nada aqui: nenhuma das 2 trampolinas abaixo toca `c`.
//   (3) `register_method`/`dynarray_push_ptr` tratam `funcs@+0x48` (instância) e `staticFuncs@+0x58`
//       (estático) como o MESMO `DynArray<T*>` genérico — a única diferença de código é o offset
//       somado ANTES de chamar `dynarray_push_ptr`; a função de push em si não sabe/não liga se é
//       0x48 ou 0x58. Push em 0x48 já rodou com sucesso em ~9 classes FORJADAS (KeyInputEvent,
//       ResourceEvent, ScriptableServiceContainer, ...); push em 0x58 já rodou com sucesso 2x mas
//       só na classe da fachada de versão, NÃO-forjada.
//
// PROVADO (2026-08-05) para `is_static=true` + `new_object`/`make_handle` numa classe que NÓS
// forjamos (`register_type_instantiable_with_parent`): `bwms-blindspot-tests.reds` chamou as 13
// fábricas static de TODA a família Target (ComponentTarget.Name, ResourceTarget.Type/Path,
// inkWidgetTarget.Controller/Library,
// DynamicEntityTarget.Tag, StaticEntityTarget.Tag, EntityTarget.Type/RecordID/Template/Appearance/
// Definition, InputTarget.Key/Axis) numa chamada só, mesmo boot: todas devolveram handle não-null
// (`IsDefined()==true`), zero crash, gameplay sobreviveu depois. `ComponentTarget.ID(id: CRUID)`
// ficou de fora — achado novo: o compilador redscript não tem NENHUM `Cast<T>` pra CRUID a partir
// de tipo literal (`NO_MATCHING_OVERLOAD` testado com `Cast<CRUID>(777ul)`), então não há via de
// SCRIPT pra construir um CRUID do zero — não é bug daqui, é limite do compilador (CRUID só existe
// como campo já-populado lido de objetos reais). As classes de evento-alvo fecham a família inteira.
/// `ComponentTarget::Matches` (2026-08-12): TAG de qual das 2 factories construiu
/// o alvo — `ID`(CRUID)/`Name`(CName) compartilhavam o mesmo slot `u64` cru sem distinguir a
/// proveniência (mesmo achado já documentado pro `EntityTarget`/`EntityTargetCriterion`). Sem isso,
/// um valor de `.ID(...)` poderia ser comparado por engano contra o NAME hash do componente real
/// (ou vice-versa) — mesma classe de bug, agora prevenida desde a origem.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ComponentTargetCriterion {
    Id,
    Name,
}

pub(crate) static COMPONENTTARGET_STATES: Mutex<Vec<(usize, ComponentTargetCriterion, u64)>> = Mutex::new(Vec::new());

/// `ComponentTarget.ID(id: CRUID) -> ref<ComponentTarget>` — fábrica estática. Lê o CRUID (u64) do
/// frame, constrói uma instância NOVA de `ComponentTarget` e guarda `(ptr, criterion, id)` em
/// `COMPONENTTARGET_STATES` (mesmo padrão de `RESOURCEEVENT_STATES`/`make_resourceevent_arg`).
pub(crate) unsafe extern "C" fn tramp_ct_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "ComponentTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix preventivo 2026-08-12: mesma retenção do #6/#134/#133, aplicada ANTES de qualquer filtro ler esta tabela de volta
        if let Ok(mut states) = COMPONENTTARGET_STATES.lock() {
            states.push((obj as usize, ComponentTargetCriterion::Id, id));
        }
    } else {
        crate::log_dev!("[ct] ComponentTarget.ID: new_object('ComponentTarget') falhou");
    }
    crate::log_dev!("[ct] ComponentTarget.ID({id:#018x}) -> {obj:p} (fábrica estática, is_static=true PROVADO 2026-08-05 em classe forjada)");
    rtti::make_handle(out, obj);
}

/// `ComponentTarget.Name(name: CName) -> ref<ComponentTarget>` — mesma receita de `tramp_ct_id`,
/// critério = hash de CName em vez de CRUID (ambos u64 puro nesta codebase).
pub(crate) unsafe extern "C" fn tramp_ct_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let name = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "ComponentTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix preventivo 2026-08-12: mesma retenção do #6/#134/#133, aplicada ANTES de qualquer filtro ler esta tabela de volta
        if let Ok(mut states) = COMPONENTTARGET_STATES.lock() {
            states.push((obj as usize, ComponentTargetCriterion::Name, name));
        }
    } else {
        crate::log_dev!("[ct] ComponentTarget.Name: new_object('ComponentTarget') falhou");
    }
    crate::log_dev!(
        "[ct] ComponentTarget.Name('{}' hash {name:#018x}) -> {obj:p} (fábrica estática, is_static=true PROVADO 2026-08-05 em classe forjada)",
        crate::cname::resolve_cname(name)
    );
    rtti::make_handle(out, obj);
}

/// Forja `ComponentTarget extends CallbackSystemTarget` (INSTANCIÁVEL — `register_type_
/// instantiable_with_parent`, mesma receita robusta de `ResourceEvent`/`AxisInputEvent`, inclui
/// `fix_embedded_allocator_vtables`) + registra as 2 fábricas STATIC (`ID`/`Name`). O parent
/// `CallbackSystemTarget` já precisa estar forjado (`register_callbacksystemtarget`, chamado antes
/// no validador — ver ordem do hook em inicio/boot.rs).
pub unsafe fn register_componenttarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_componenttarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "ComponentTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_componenttarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_componenttarget: classe=OK mas sem protótipo — ID/Name NÃO registrados".into(),
    };
    let m1 = register_method(reg, "ComponentTarget", proto, "ID", "ID", tramp_ct_id, true);
    let m2 = register_method(reg, "ComponentTarget", proto, "Name", "Name", tramp_ct_name, true);
    format!("[reg] ComponentTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) ID={m1} Name={m2}")
}

/// `EntityComponentEvent extends EntityLifecycleEvent` — classes de evento-alvo (2026-07-24).
/// Forma: `public native class EntityComponentEvent extends EntityLifecycleEvent {
/// GetComponent()->wref<IComponent> }`. **Divergência FECHADA (2026-08-11)**:
/// `wref<T>` agora usa `rtti::make_weak_handle` (mesma técnica do `EntityLifecycleEvent.GetEntity()`), assinatura
/// original restaurada — sem mais handle raw sem refcount. Estado: (evento, componente, entidade dona).
pub(crate) static ENTITYCOMPONENT_STATES: Mutex<Vec<(usize, usize, usize)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_ece_get_component(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut ptr: usize = 0;
    if let Ok(states) = ENTITYCOMPONENT_STATES.lock() {
        if let Some((_, p, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            ptr = *p;
        }
    }
    crate::log_dev!("[ece] EntityComponentEvent.GetComponent() on {c:p} -> {ptr:#x}");
    // 2026-08-11: `wref<IComponent>` REAL via `rtti::make_weak_handle` — mesma
    // técnica que fechou `EntityLifecycleEvent.GetEntity()`. Substitui o
    // `write_uint_ret` cru (nem escrevia os 16 bytes completos de um Handle) documentado como
    // "ref forte" — na real nunca tinha refcount block nenhum, categoria de risco do mesmo tipo
    // já corrigida no `GetEntity()`.
    rtti::make_weak_handle(out, ptr as *mut c_void);
}

pub unsafe fn register_entitycomponentevent(reg: &Registry) -> String {
    let ele = reg.class_by_name("EntityLifecycleEvent");
    if !rtti::sane(ele) {
        return "[reg] register_entitycomponentevent: 'EntityLifecycleEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "EntityComponentEvent", "gameuiInGameMenuGameController", ele);
    if forged.is_null() {
        return "[reg] register_entitycomponentevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_entitycomponentevent: classe=OK mas sem protótipo — método NÃO registrado".into(),
    };
    let m1 = register_method(reg, "EntityComponentEvent", proto, "GetComponent", "GetComponent", tramp_ece_get_component, false);
    format!("[reg] EntityComponentEvent: classe=OK (INSTANCIÁVEL, parent=EntityLifecycleEvent real, GetComponent devolve wref REAL via make_weak_handle) GetComponent={m1}")
}

/// Constrói um `EntityComponentEvent` com o componente e a entidade dona (`GetEntity` do evento): o ouvinte do evento de componente lê a entidade por ele.
pub unsafe fn make_entitycomponentevent_arg(component_ptr: *mut c_void, entidade: usize) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "EntityComponentEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = ENTITYCOMPONENT_STATES.lock() {
        // Um evento anterior no mesmo endereço que nunca saiu do estado faria os getters acharem o estado velho.
        states.retain(|(p, ..)| *p != obj as usize);
        states.push((obj as usize, component_ptr as usize, entidade));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

pub(crate) unsafe extern "C" fn tramp_eba_get_resource_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, ebw_com_builder(c, |_, a| eb_u64(eb_u64(a, EBA_RECURSO), RES_CAMINHO)).unwrap_or(0), 8);
}

/// `GetDefinitionName() -> CName` (forma própria: `appearanceAppearanceDefinition` não é declarado no corpus vanilla).
pub(crate) unsafe extern "C" fn tramp_eba_get_definition_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, ebw_com_builder(c, |_, a| eb_u64(eb_u64(a, EBA_DEFINICAO), DEF_NOME)).unwrap_or(0), 8);
}

pub(crate) unsafe extern "C" fn tramp_eba_get_components(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    match ebw_com_builder(c, |_, a| {
        let resultados = eb_u64(a, EBA_EXTRATOR) + EXT_RESULTADOS;
        (eb_handles(resultados, 0), resultados)
    }) {
        Some((handles, resultados)) if resultados != EXT_RESULTADOS => eb_componentes_ret(out, &handles, resultados),
        _ => eb_array_ret(out, &[], 0, 0),
    }
}

pub(crate) unsafe extern "C" fn tramp_eba_add_component(c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let extrator = ebw_com_builder(c, |_, a| eb_u64(a, EBA_EXTRATOR)).unwrap_or(0);
    let ok = eb_adicionar_componente(frame, extrator);
    crate::log_dev!("[ebw] EntityBuilderAppearanceWrapper.AddComponent em {c:p}: extrator {extrator:#x} -> {ok}");
}

/// Componente do V com a classe exata `classe` (em `ComponentsStorage.components` = `Entity+0xA0`), ou nulo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn componente_do_jogador(reg: &Registry, classe: &str) -> *mut c_void {
    let player = crate::current_player();
    let cls = reg.class_by_name(classe);
    if player.is_null() || cls.is_null() || !crate::memoria::is_readable(player as *const c_void, 0xB0) {
        return std::ptr::null_mut();
    }
    let arr = (player as *const u8).add(0xA0);
    let entradas = arr.cast::<u64>().read_unaligned() as *const u64;
    let tam = arr.add(0x0C).cast::<u32>().read_unaligned();
    if entradas.is_null() || tam > 4096 || !crate::memoria::is_readable(entradas as *const c_void, tam as usize * 16) {
        return std::ptr::null_mut();
    }
    (0..tam as usize)
        .map(|i| entradas.add(i * 2).read_unaligned() as *mut c_void)
        .find(|&c| !c.is_null() && crate::rtti::class_of(c) == cls)
        .unwrap_or(std::ptr::null_mut())
}

/// Tamanho do array de componentes da entidade (`entity+0xA0`).
pub(crate) unsafe fn entidade_num_componentes(entidade: *mut c_void) -> usize {
    let proprio = (entidade as *const u8).add(0xA0) as *const c_void;
    if !crate::memoria::is_readable(proprio, 0x10) {
        return 0;
    }
    core::ptr::read_unaligned((proprio as *const u8).add(0x0C) as *const u32) as usize
}

/// `Entity/Reassemble`: copia pro array de componentes novos da troca de aparência os componentes que os ouvintes somaram em
/// `entity+0xA0` durante o despacho (do índice `antes` em diante), cada um com referência forte própria. Devolve quantos copiou.
pub(crate) unsafe fn entidade_propagar_componentes(entidade: *mut c_void, componentes: *mut c_void, antes: usize) -> usize {
    let proprio = (entidade as *const u8).add(0xA0) as *const c_void;
    if !crate::memoria::is_readable(proprio, 0x10) || !crate::memoria::is_readable(componentes as *const c_void, 0x10) {
        return 0;
    }
    let entradas = rd_u64(proprio, 0) as *const u8;
    let tam = entidade_num_componentes(entidade);
    if entradas.is_null() || tam <= antes || !crate::memoria::is_readable(entradas.add(antes * 16) as *const c_void, (tam - antes) * 16) {
        return 0;
    }
    let mut copiados = 0;
    for i in antes..tam.min(antes + 64) {
        let slot = entradas.add(i * 16);
        let h = [core::ptr::read_unaligned(slot as *const u64), core::ptr::read_unaligned(slot.add(8) as *const u64)];
        if h[0] != 0 && dynarray_push_handle16(componentes, h[0], h[1]).is_some() {
            handle_strong_soma(&h, 1);
            copiados += 1;
        }
    }
    copiados
}

/// Corpo inteiro/torso (2026-08-07, cont.41, continuação): `toggle` sozinho (escreve `enabled=1`)
/// PERSISTE mas NÃO muda o resultado visual (confirmado por screenshot real, câmera pitch=-60,
/// antes/depois idênticos). Hipótese: o render proxy precisa ser RECONSTRUÍDO pra refletir o novo
/// `enabled` — a mesma função virtual já mapeada e testada como `rebuild-proxy` neste projeto
/// (`vtable+0x288` de `entSkinnedMeshComponent`, `0x101991450`, 144B, observe-only 30/30 amostras
/// sem crash em sessões anteriores). Esta função: acha o componente (mesma travessia), seta
/// `enabled=1`, e CHAMA `0x101991450(inst)` direto (1-arg void, ABI já confirmada) — mesma
/// categoria de risco BAIXA (não-Find/resolve, já observada sem efeito colateral).
#[cfg(feature = "devtools")]
pub unsafe fn run_toggle_and_rebuild(
    reg: &crate::rtti::Registry,
    player: *mut c_void,
    wanted_name: &str,
) -> String {
    if player.is_null() || !crate::memoria::is_readable(player as *const c_void, 0x10) {
        return "[togglerebuild] player nulo/ilegível".to_string();
    }
    let target_cls = reg.class_by_name("entSkinnedMeshComponent");
    if target_cls.is_null() {
        return "[togglerebuild] classe 'entSkinnedMeshComponent' não achada no registry".to_string();
    }
    let hash_wanted = crate::cname::cname(wanted_name);
    let arr = (player as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return "[togglerebuild] player+0xA0 ilegível".to_string();
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size > 4096 {
        return format!("[togglerebuild] array inválido: entries={entries:p} size={size}");
    }
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if inst.is_null() {
            continue;
        }
        if !class_is_kind_of(crate::rtti::class_of(inst), target_cls) || !crate::memoria::is_readable(inst as *const c_void, 0x200) {
            continue;
        }
        let name_hash = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
        if name_hash != hash_wanted {
            continue;
        }
        let before = (inst as *const u8).add(0x8b).read_unaligned();
        core::ptr::write_unaligned((inst as *mut u8).add(0x8b), 1u8);
        let after_write = (inst as *const u8).add(0x8b).read_unaligned();
        let res_before = ((inst as *const u8).add(0x200) as *const u64).read_unaligned();
        type OneArgVoidFn = unsafe extern "C" fn(*mut c_void);
        let rebuild: OneArgVoidFn = core::mem::transmute(crate::rebase(0x1_0199_1450));
        rebuild(inst);
        let res_after = ((inst as *const u8).add(0x200) as *const u64).read_unaligned();
        return format!(
            "[togglerebuild] '{wanted_name}' inst={inst:p} enabled ANTES={before} DEPOIS={after_write} res_ptr ANTES-rebuild={res_before:#x} DEPOIS-rebuild={res_after:#x}"
        );
    }
    format!("[togglerebuild] '{wanted_name}' (hash={hash_wanted:#x}) NÃO achado entre os componentes do player")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GarmentComponentKind {
    /// `entMeshComponent` — folha, ninguém mais `IsA` ele (é IRMÃO de `ISkinTargetComponent`,
    /// não ancestral — por isso vem checado primeiro na mesma ordem do C++ real).
    MeshComponent,
    /// `entSkinnedMeshComponent` OU `entGarmentSkinnedMeshComponent` — offsets IDÊNTICOS
    /// (Garment não soma campo antes de 0x2D0), então tratados como 1 kind só.
    SkinnedFamily,
    /// `entMorphTargetSkinnedMeshComponent`.
    MorphTarget,
    /// Nenhum dos 4 tipos de mesh.
    Unsupported,
}

/// Nome da property RTTI de cada campo por tipo. O offset vem do `CProperty::valueOffset` da classe real
/// (`prop_value_offset`), nunca de tabela: a tabela antiga copiava um header gerado de layout, que erra a família skinned
/// inteira por 8 bytes nesta build — `mesh` `0x220` (header `0x228`), `meshAppearance` `0x228` (header `0x230`), `chunkMask`
/// `0x240` (header `0x248`). Medido pelo RTTI e pelo `MeshComponent::LoadResource` (`0x10198f864`), que carrega `+0x220`.
pub(crate) fn nome_prop_path(kind: GarmentComponentKind) -> Option<&'static str> {
    match kind {
        GarmentComponentKind::MeshComponent | GarmentComponentKind::SkinnedFamily => Some("mesh"),
        GarmentComponentKind::MorphTarget => Some("morphResource"),
        GarmentComponentKind::Unsupported => None,
    }
}

/// A#22 22D — componentes da entidade (`Entity+0xA0`, `DynArray<Handle<IComponent>>`), só os legíveis.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn componentes_da_entidade(entidade: *mut c_void) -> Vec<*mut c_void> {
    if entidade.is_null() || !crate::memoria::is_readable(entidade as *const c_void, 0xB0) {
        return Vec::new();
    }
    let arr = (entidade as *mut u8).add(0xA0) as *mut c_void;
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size == 0 || size > 4096 || !crate::memoria::is_readable(entries as *const c_void, size as usize * 16) {
        return Vec::new();
    }
    (0..size as usize)
        .map(|i| core::ptr::read_unaligned(entries.add(i * 16) as *const u64) as *mut c_void)
        .filter(|c| !c.is_null() && crate::memoria::is_readable(*c as *const c_void, 0x90))
        .collect()
}

/// Estado de máscara do componente: `(chunkMask, isEnabled, é de mesh)`; `chunkMask` `None` fora dos tipos de mesh.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn componente_mascara(reg: &crate::rtti::Registry, inst: *mut c_void) -> (Option<u64>, bool, bool) {
    let kind = detect_garment_component_kind(reg, inst);
    let malha = kind != GarmentComponentKind::Unsupported;
    let mascara = if malha { component_get_chunk_mask(inst, kind) } else { None };
    (mascara, (inst as *const u8).add(0x8b).read_unaligned() != 0, malha)
}


/// Entrada de uma tag visual: `(CName do componente ou do prefixo, mostrar, máscara)`.
pub(crate) type EntradaDeTag = (CName, bool, u64);

/// Esconder o componente inteiro: máscara 0 na interseção.
const fn esconder_tudo(chave: &str) -> EntradaDeTag {
    (CName::cname(chave), false, 0)
}

/// Esconder só os chunks da região da tag (chunk N = bit `1 << N` do `chunkMask` do mesh do componente). Chunk acima de 63 para a
/// compilação: a tabela é `static`, e a máscara errada só apareceria no V.
const fn esconder_parte(chave: &str, chunks: &[u8]) -> EntradaDeTag {
    let Some(mascara) = bwms_primitives::override_state::mascara_de_chunks(false, chunks) else { panic!("chunk acima de 63 na tabela de tags") };
    (CName::cname(chave), false, mascara)
}

/// Mostrar os chunks do calçado, como em [`esconder_parte`].
const fn mostrar_parte(chave: &str, chunks: &[u8]) -> EntradaDeTag {
    let Some(mascara) = bwms_primitives::override_state::mascara_de_chunks(true, chunks) else { panic!("chunk acima de 63 na tabela de tags") };
    (CName::cname(chave), true, mascara)
}

const TORSO_PMA: &str = "t0_000_pma_base__full";
const TORSO_PWA: &str = "t0_000_pwa_base__full";
const TORSO_PWA_FPP: &str = "t0_000_pwa_fpp__torso";

/// Os 3 meshes de torso do V repartem os mesmos chunks: 0 peito, 1 clavícula, 2 abdômen superior, 3 abdômen inferior, 4 coxas,
/// 5 panturrilhas, 6 tornozelos, 7 pés (`B4c-chunks-das-tags.md`).
const fn torso_parcial(chunks: &[u8]) -> [EntradaDeTag; 3] {
    [esconder_parte(TORSO_PMA, chunks), esconder_parte(TORSO_PWA, chunks), esconder_parte(TORSO_PWA_FPP, chunks)]
}

/// Torso e as 5 pernas de calçado; nas pernas de calçado o chunk 0 é a panturrilha, 1 o tornozelo, 2 o pé.
const fn pernas_e_pes(torso: &[u8], calcado: &[u8]) -> [EntradaDeTag; 8] {
    [
        esconder_parte(TORSO_PMA, torso),
        esconder_parte("l0_000_pma_base__high_heels", calcado),
        esconder_parte("l0_000_pma_base__flat_shoes", calcado),
        esconder_parte(TORSO_PWA, torso),
        esconder_parte(TORSO_PWA_FPP, torso),
        esconder_parte("l0_000_pwa_base__cs_flat", calcado),
        esconder_parte("l0_000_pwa_base__high_heels", calcado),
        esconder_parte("l0_000_pwa_base__flat_shoes", calcado),
    ]
}

const TAG_PANTURRILHAS: [EntradaDeTag; 8] = pernas_e_pes(&[5], &[0]);
const TAG_TORNOZELOS: [EntradaDeTag; 8] = pernas_e_pes(&[6], &[1]);
const TAG_PES: [EntradaDeTag; 8] = pernas_e_pes(&[7], &[2]);

/// As 14 tags visuais que o mod escreve no `visualTags` do item (caixa exata) e os componentes do corpo do V que cada uma toca. Chave terminada
/// em `_` é prefixo de nome ([`component_prefix_of`]).
pub(crate) static TAGS_EMBUTIDAS: [(u64, &[EntradaDeTag]); 14] = [
    (
        crate::cname::cname("hide_Head"),
        &[
            esconder_tudo("h0_"),
            esconder_tudo("he_"),
            esconder_tudo("heb_"),
            esconder_tudo("ht_"),
            esconder_tudo("hx_"),
            esconder_tudo("i1_"),
            esconder_tudo("beard"),
            esconder_tudo("beard_"),
            esconder_tudo("MorphTargetSkinnedMesh3637"),
            esconder_tudo("MorphTargetSkinnedMesh6675"),
            esconder_tudo("MorphTargetSkinnedMesh7243"),
            esconder_tudo("MorphTargetSkinnedMesh7561"),
        ],
    ),
    (crate::cname::cname("hide_Arms"), &[esconder_tudo("a0_"), esconder_tudo("left_arm"), esconder_tudo("right_arm")]),
    (
        crate::cname::cname("hide_Torso"),
        &[
            esconder_tudo("n0_"),
            esconder_tudo("tx_"),
            esconder_parte(TORSO_PMA, &[0, 1, 2, 3]),
            esconder_tudo("t0_000_pma_base__full_seamfix"),
            esconder_parte(TORSO_PWA, &[0, 1, 2, 3]),
            esconder_tudo("t0_000_pwa_base__full_seamfix"),
            esconder_parte(TORSO_PWA_FPP, &[0, 1, 2, 3]),
            esconder_tudo("MorphTargetSkinnedMesh0531"),
        ],
    ),
    (crate::cname::cname("hide_LowerAbdomen"), &torso_parcial(&[3])),
    (crate::cname::cname("hide_UpperAbdomen"), &torso_parcial(&[2])),
    (crate::cname::cname("hide_CollarBone"), &torso_parcial(&[1])),
    (
        crate::cname::cname("hide_Chest"),
        &[esconder_parte(TORSO_PMA, &[0]), esconder_parte(TORSO_PWA, &[0]), esconder_parte(TORSO_PWA_FPP, &[0]), esconder_tudo("MorphTargetSkinnedMesh0531")],
    ),
    (
        crate::cname::cname("hide_Legs"),
        &[
            esconder_tudo("l0_"),
            esconder_tudo("s0_"),
            esconder_parte(TORSO_PMA, &[4, 5, 6, 7]),
            esconder_parte(TORSO_PWA, &[4, 5, 6, 7]),
            esconder_parte(TORSO_PWA_FPP, &[4, 5, 6, 7]),
        ],
    ),
    (crate::cname::cname("hide_Thighs"), &torso_parcial(&[4])),
    (crate::cname::cname("hide_Calves"), &TAG_PANTURRILHAS),
    (
        crate::cname::cname("hide_Ankles"),
        &[
            esconder_tudo("s0_"),
            TAG_TORNOZELOS[0],
            TAG_TORNOZELOS[1],
            TAG_TORNOZELOS[2],
            TAG_TORNOZELOS[3],
            TAG_TORNOZELOS[4],
            TAG_TORNOZELOS[5],
            TAG_TORNOZELOS[6],
            TAG_TORNOZELOS[7],
        ],
    ),
    (crate::cname::cname("hide_Feet"), &TAG_PES),
    (
        crate::cname::cname("HighHeels"),
        &[
            esconder_parte(TORSO_PMA, &[5, 6, 7]),
            mostrar_parte("l0_000_pma_base__high_heels", &[0, 1, 2]),
            esconder_parte(TORSO_PWA, &[5, 6, 7]),
            esconder_parte(TORSO_PWA_FPP, &[5, 6, 7]),
            mostrar_parte("l0_000_pwa_base__high_heels", &[0, 1, 2]),
        ],
    ),
    (
        crate::cname::cname("FlatShoes"),
        &[
            esconder_parte(TORSO_PMA, &[5, 6, 7]),
            mostrar_parte("l0_000_pma_base__flat_shoes", &[0, 1, 2]),
            esconder_parte(TORSO_PWA, &[5, 6, 7]),
            esconder_parte(TORSO_PWA_FPP, &[5, 6, 7]),
            mostrar_parte("l0_000_pwa_base__flat_shoes", &[0, 1, 2]),
        ],
    ),
];

/// Entradas da tag pelo CName (caixa importa); 0 ou desconhecida → `None`.
pub(crate) fn tag_por_hash(tag: u64) -> Option<&'static [EntradaDeTag]> {
    if tag == 0 {
        return None;
    }
    TAGS_EMBUTIDAS.iter().find(|(h, _)| *h == tag).map(|(_, t)| *t)
}

/// Componentes de `Entity+0xA0` com estado na tabela da tag: o nome base ou o prefixo do nome (`chaves_do_componente`) é chave da tabela (as mesmas duas
/// chaves com que a loja decide o componente, `es20_estados_do_componente`). As chaves "n0_"/"h0_"/"l0_"/"tx_"/… são prefixos: nenhum componente se chama
/// assim (o V tem `n0_000_pma_base__full`, `n0_000_pma_fpp__neck2814`). `(verificados, que batem)`; `None` sem lista de componentes coerente.
pub(crate) unsafe fn componentes_da_tag(entity: *mut c_void, tabela: &[EntradaDeTag]) -> Option<(usize, Vec<*mut c_void>)> {
    if entity.is_null() || !crate::memoria::is_readable(entity as *const c_void, 0x10) {
        return None;
    }
    let arr = (entity as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    // `size > capacity` é offset errado lido como dado, e a máscara iria para lixo. O teto de 4096 fica além do
    // `dynarray_coerente` (que aceita até 1<<20): o laço abaixo paga 2 `is_readable` por elemento.
    let (e, size, cap) = dynarray_em(arr);
    if e == 0 || size == 0 || !dynarray_coerente(e, size, cap) || size > 4096 {
        return None;
    }
    let entries = e as *const u8;
    let chave = |nome: u64| tabela.iter().any(|(chave, _, _)| chave.valor() == nome);
    let mut verificados = 0;
    let mut batem = Vec::new();
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        // 0x90 cobre o nome (+0x40) e o `isEnabled` (+0x8b) que o `es20_aplicar_mascara` lê.
        if inst.is_null() || !crate::memoria::is_readable(inst as *const c_void, 0x90) {
            continue;
        }
        verificados += 1;
        let (base, prefixo) = chaves_do_componente((inst as *const u8).add(0x40).cast::<u64>().read_unaligned());
        if chave(base) || prefixo.is_some_and(chave) {
            batem.push(inst);
        }
    }
    Some((verificados, batem))
}

/// Fonte de um override de tag: o par (tag, mod). Com só o mod, remover uma tag levaria a contribuição de outra tag do mesmo mod nos
/// componentes que as duas compartilham (`t0_000_pma_base__full` está em hide_Torso e em hide_Legs), porque o esconder é AND por fonte.
/// `ApplyVisualTagOverride`, sem mod, é a fonte (tag, 0).
pub(crate) fn fonte_da_tag(tag: &str, mod_hash: u64) -> u64 {
    bwms_primitives::fnv1a64_seeded(tag.as_bytes(), mod_hash)
}

/// O `ApplyVisualTagOverrideForMod` sem o RTTI: cada entrada da tag entra na loja do es20 sob a fonte (tag, mod) (`es20_registrar_tag`) e `aplicar` roda em cada componente que bate. `(verificados, que batem, aplicados)`; tag
/// desconhecida ou entidade sem lista de componentes coerente não entram na loja.
pub(crate) unsafe fn tag_aplicar_na_loja(
    entity: *mut c_void,
    tag: &str,
    mod_hash: u64,
    mut aplicar: impl FnMut(*mut c_void) -> bool,
) -> Option<(usize, usize, usize)> {
    let tabela = tag_por_hash(crate::cname::cname(tag))?;
    let (verificados, batem) = componentes_da_tag(entity, tabela)?;
    es20_registrar_tag(entity as u64, fonte_da_tag(tag, mod_hash), tabela);
    let aplicados = batem.iter().filter(|&&inst| aplicar(inst)).count();
    Some((verificados, batem.len(), aplicados))
}

/// O `RemoveVisualTagOverrideForMod` sem o RTTI: tira a fonte (tag, mod) só dos nomes da tag (`es20_remover_mascara`) e, se tirou algo, `aplicar`
/// roda de novo em cada componente que bate. O estado fica com a mudança de máscara ligada, então componente sem fonte nenhuma recebe a
/// original. `(nomes com estado, nomes com algo da fonte, reaplicados)`.
pub(crate) unsafe fn tag_remover_da_loja(
    entity: *mut c_void,
    tag: &str,
    mod_hash: u64,
    mut aplicar: impl FnMut(*mut c_void) -> bool,
) -> Option<(usize, usize, usize)> {
    let tabela = tag_por_hash(crate::cname::cname(tag))?;
    let (_, batem) = componentes_da_tag(entity, tabela)?;
    let (com_estado, removidos) = es20_remover_mascara(entity as u64, fonte_da_tag(tag, mod_hash), tabela);
    let reaplicados = if removidos == 0 { 0 } else { batem.iter().filter(|&&inst| aplicar(inst)).count() };
    Some((com_estado, removidos, reaplicados))
}

/// Grava a máscara decidida pela loja e, com `refresh`, recarrega o visual pelo slot medido. `true` se gravou.
unsafe fn es20_aplicar_mascara_da_tag(m: &Es20Malhas, entidade: u64, inst: *mut c_void, refresh: bool) -> bool {
    if es20_aplicar_mascara(m, entidade, inst).is_none() {
        return false;
    }
    if refresh {
        if let Some(recarregar) = es20_refresh_medido(inst) {
            recarregar(inst);
        }
    }
    true
}

/// `ApplyVisualTagOverrideForMod(WithRefresh)`: registra a tag na loja e aplica nos componentes da entidade. `(verificados, que batem, escritos)`.
pub(crate) unsafe fn apply_visual_tag_override_for_mod(
    reg: &crate::rtti::Registry,
    entity: *mut c_void,
    tag: &str,
    mod_hash: u64,
    refresh: bool,
) -> (usize, usize, usize) {
    let Some(m) = es20_malhas(reg) else { return (0, 0, 0) };
    let entidade = entity as u64;
    tag_aplicar_na_loja(entity, tag, mod_hash, |inst| es20_aplicar_mascara_da_tag(m, entidade, inst, refresh)).unwrap_or_default()
}

/// `RemoveVisualTagOverrideForMod(WithRefresh)`: `tag_remover_da_loja` com a mesma aplicação de `apply_visual_tag_override_for_mod`. Desinstalar um mod
/// pede um Remove por tag aplicada. Sem `es20_malhas` a loja fica intacta: tirar a fonte sem reaplicar deixaria a máscara combinada no campo.
/// `(nomes com estado, nomes com algo da fonte, reescritos)`.
pub(crate) unsafe fn remove_visual_tag_override_for_mod(
    reg: &crate::rtti::Registry,
    entity: *mut c_void,
    tag: &str,
    mod_hash: u64,
    refresh: bool,
) -> (usize, usize, usize) {
    let Some(m) = es20_malhas(reg) else { return (0, 0, 0) };
    let entidade = entity as u64;
    tag_remover_da_loja(entity, tag, mod_hash, |inst| es20_aplicar_mascara_da_tag(m, entidade, inst, refresh)).unwrap_or_default()
}

/// FNV-1a64 de `default` — reserva se o global do motor não puder ser lido.
pub(crate) const CNAME_DEFAULT: u64 = 0xebad_a516_8620_c5fe;

/// Parte de recurso a que o componente foi ligado, por `(entidade, nome base)`.
pub(crate) static ES22_COMPONENT_APPEARANCE: Mutex<Vec<((u64, u64), u64)>> = Mutex::new(Vec::new());

/// Liga (ou religa) o nome base à parte. `false` só com a trava envenenada.
pub(crate) fn es22_ligar_componente(entidade: u64, nome_base: u64, parte: u64) -> bool {
    let Ok(mut g) = ES22_COMPONENT_APPEARANCE.lock() else { return false };
    match g.iter_mut().find(|(k, _)| *k == (entidade, nome_base)) {
        Some((_, p)) => *p = parte,
        None => g.push(((entidade, nome_base), parte)),
    }
    true
}

pub(crate) fn es22_ligar_parte(entidade: u64, parte: u64, aparencia: &str) {
    es22_ligar_parte_seletor(entidade, parte, &DynamicAppearanceName::parse(aparencia));
}

pub(crate) fn es22_ligar_parte_seletor(entidade: u64, parte: u64, seletor: &DynamicAppearanceName) {
    let Ok(mut g) = ES22_RESOURCE_STATE.lock() else { return };
    match g.iter_mut().find(|(k, _)| *k == (entidade, parte)) {
        Some((_, s)) => *s = seletor.clone(),
        None => g.push(((entidade, parte), seletor.clone())),
    }
}

/// Tira da loja de override todas as chaves da entidade.
pub(crate) fn es20_esquecer(entidade: u64) {
    if let Ok(mut m) = es20_componentes().lock() {
        m.retain(|(e, _), _| *e != entidade);
    }
}


pub(crate) fn es20_componentes() -> &'static Mutex<Es20Estados> {
    static ESTADOS: std::sync::OnceLock<Mutex<Es20Estados>> = std::sync::OnceLock::new();
    ESTADOS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn es20_adicionar_mascara(entidade: u64, hash: u64, nome: CName, mascara: u64, mostrar: bool) {
    if nome.valor() == 0 {
        return;
    }
    let Ok(mut m) = es20_componentes().lock() else { return };
    let estado = m.entry((entidade, nome.valor())).or_default();
    if mostrar {
        estado.mostrar(hash, mascara);
    } else {
        estado.esconder(hash, mascara);
    }
}

pub(crate) fn es20_registrar_tag(entidade: u64, hash: u64, tabela: &[EntradaDeTag]) {
    for &(chave, mostrar, mascara) in tabela {
        es20_adicionar_mascara(entidade, hash, chave, mascara, mostrar);
    }
}

/// Tira a máscara da fonte só nas chaves da tag; o estado fica (com a mudança ligada, a original volta na aplicação).
/// `(chaves da tag com estado, chaves em que a fonte tinha algo)`.
pub(crate) fn es20_remover_mascara(entidade: u64, hash: u64, tabela: &[EntradaDeTag]) -> (usize, usize) {
    let Ok(mut m) = es20_componentes().lock() else { return (0, 0) };
    let (mut com_estado, mut removidos) = (0, 0);
    for (chave, _, _) in tabela {
        if let Some(estado) = m.get_mut(&(entidade, chave.valor())) {
            com_estado += 1;
            removidos += usize::from(estado.remover_fonte(hash));
        }
    }
    (com_estado, removidos)
}

/// Tira máscara e aparência da fonte em todas as chaves da entidade; devolve em quantas havia algo dela.
pub(crate) fn es20_remover_componentes(entidade: u64, hash: u64) -> usize {
    let Ok(mut m) = es20_componentes().lock() else { return 0 };
    m.iter_mut()
        .filter(|((e, _), _)| *e == entidade)
        .map(|(_, estado)| {
            let mascara = estado.remover_fonte(hash);
            let aparencia = estado.remover_aparencia(hash);
            usize::from(mascara || aparencia)
        })
        .sum()
}

pub(crate) fn es20_estado(entidade: u64, nome: u64) -> Option<bwms_primitives::override_state::ComponentState> {
    es20_componentes().lock().ok()?.get(&(entidade, nome)).cloned()
}


pub(crate) struct Es20Malhas {
    /// `entMeshComponent`, `entMorphTargetSkinnedMeshComponent`, `entGarmentSkinnedMeshComponent`, `entSkinnedMeshComponent` (garment antes de skinned: a garment é subclasse).
    pub(crate) classes: [u64; 4],
    /// `(chunkMask, meshAppearance)` por tipo: 0 mesh, 1 família skinned, 2 morph target.
    pub(crate) campos: [(usize, usize); 3],
    /// `appearanceName` e `appearancePath` do `IComponent`.
    pub(crate) base: (usize, usize),
    /// Caminho do recurso por tipo (`GetResourcePath`/`SetResourcePath`, 20g): `mesh` do mesh e da família skinned, `morphResource` do morph target; `None`
    /// sem os 3 no RTTI.
    pub(crate) recursos: Option<[usize; 3]>,
}

impl Es20Malhas {
    pub(crate) fn campos(&self, tipo: usize) -> (usize, usize) {
        self.campos[tipo]
    }

    pub(crate) fn recurso(&self, tipo: usize) -> Option<usize> {
        self.recursos.map(|r| r[tipo])
    }

    /// `appearanceName` e `appearancePath` do `IComponent` (o `GetUniqueId`).
    pub(crate) fn base(&self) -> (usize, usize) {
        self.base
    }
}

/// Classes e offsets de `Es20Malhas` pelo RTTI (thread do jogo). `appearanceName`/`appearancePath` sem propriedade no RTTI caem nos offsets
/// medidos (`0x48`/`0x68`, `DATABASE.md`); o `bool` diz se vieram do RTTI.
pub(crate) unsafe fn es20_resolver_malhas(reg: &rtti::Registry) -> Option<(Es20Malhas, bool)> {
    let classe = |n: &str| Some(reg.class_by_name(n)).filter(|c| !c.is_null());
    let [mesh, morph, garment, skinned, base] = [
        classe("entMeshComponent")?,
        classe("entMorphTargetSkinnedMeshComponent")?,
        classe("entGarmentSkinnedMeshComponent")?,
        classe("entSkinnedMeshComponent")?,
        classe("entIComponent")?,
    ];
    let par = |c: *mut c_void| Some((prop_value_offset(c, "chunkMask")?, prop_value_offset(c, "meshAppearance")?));
    let (nome, caminho) = (prop_value_offset(base, "appearanceName"), prop_value_offset(base, "appearancePath"));
    let pelo_rtti = nome.is_some() && caminho.is_some();
    let recursos = (|| Some([prop_value_offset(mesh, "mesh")?, prop_value_offset(skinned, "mesh")?, prop_value_offset(morph, "morphResource")?]))();
    Some((
        Es20Malhas {
            classes: [mesh as u64, morph as u64, garment as u64, skinned as u64],
            campos: [par(mesh)?, par(skinned)?, par(morph)?],
            base: (nome.unwrap_or(0x48), caminho.unwrap_or(0x68)),
            recursos,
        },
        pelo_rtti,
    ))
}

/// `Es20Malhas` das natives de `BwmsAparencia` (thread do jogo), resolvido na 1ª chamada que acha as classes de mesh no RTTI; `None` não fica guardado.
pub(crate) unsafe fn es20_malhas(reg: &rtti::Registry) -> Option<&'static Es20Malhas> {
    static MALHAS: std::sync::OnceLock<Es20Malhas> = std::sync::OnceLock::new();
    if let Some(m) = MALHAS.get() {
        return Some(m);
    }
    let (m, _) = es20_resolver_malhas(reg)?;
    Some(MALHAS.get_or_init(|| m))
}

/// Tipo de mesh do componente (0 mesh, 1 família skinned, 2 morph target); `None` fora das classes de mesh.
pub(crate) unsafe fn es20_tipo(m: &Es20Malhas, inst: *mut c_void) -> Option<usize> {
    let cls = rtti::class_of(inst);
    if cls.is_null() {
        return None;
    }
    let [mesh, morph, garment, skinned] = m.classes.map(|c| c as *mut c_void);
    if class_is_kind_of(cls, mesh) {
        Some(0)
    } else if class_is_kind_of(cls, morph) {
        Some(2)
    } else if class_is_kind_of(cls, garment) || class_is_kind_of(cls, skinned) {
        Some(1)
    } else {
        None
    }
}

/// Identidade do componente vivo para o cache de originais: nome, `appearanceName` e `appearancePath` encadeados. Dois componentes de mesmo
/// nome com aparência ou caminho diferentes não dividem a original.
pub(crate) unsafe fn es20_id_do_componente(base: (usize, usize), inst: *const u8) -> u64 {
    let ler = |off: usize| (inst.add(off) as *const u64).read_unaligned();
    let com_aparencia = bwms_primitives::fnv1a64_seeded(&ler(base.0).to_le_bytes(), ler(0x40));
    bwms_primitives::fnv1a64_seeded(&ler(base.1).to_le_bytes(), com_aparencia)
}

/// `(entidade, id do componente, 0 máscara / 1 aparência / 2 caminho do recurso)` → original.
pub(crate) fn es20_originais() -> &'static Mutex<std::collections::HashMap<(u64, u64, u8), u64>> {
    static ORIGINAIS: std::sync::OnceLock<Mutex<std::collections::HashMap<(u64, u64, u8), u64>>> = std::sync::OnceLock::new();
    ORIGINAIS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// A 1ª leitura fica: depois do override, o campo vivo já não é a original. Trava envenenada → 0.
pub(crate) fn es20_original(entidade: u64, id: u64, qual: u8, atual: impl FnOnce() -> u64) -> u64 {
    let Ok(mut m) = es20_originais().lock() else { return 0 };
    *m.entry((entidade, id, qual)).or_insert_with(atual)
}

/// Aparência original (campo 1 do cache). Leitura que falha devolve 0 sem guardar: um 0 guardado viraria "original" para sempre.
pub(crate) unsafe fn es20_original_da_aparencia(base: (usize, usize), entidade: u64, inst: *mut c_void, kind: GarmentComponentKind) -> u64 {
    let id = es20_id_do_componente(base, inst as *const u8);
    if let Some(v) = es20_originais().lock().ok().and_then(|m| m.get(&(entidade, id, 1)).copied()) {
        return v;
    }
    match component_get_appearance_name(inst, kind) {
        Some(atual) => es20_original(entidade, id, 1, || atual),
        None => 0,
    }
}

/// `(entidade, parte)` → nomes dos componentes ligados a ela (`m_partComponents`, o `GetPartComponentNames` do 20d).
pub(crate) fn es20_componentes_da_parte() -> &'static Mutex<std::collections::HashMap<(u64, u64), std::collections::BTreeSet<u64>>> {
    static PARTES: std::sync::OnceLock<Mutex<std::collections::HashMap<(u64, u64), std::collections::BTreeSet<u64>>>> = std::sync::OnceLock::new();
    PARTES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Nomes completos dos componentes ligados à parte, em ordem crescente de hash.
pub(crate) fn es20_nomes_da_parte(entidade: u64, parte: u64) -> Vec<u64> {
    es20_componentes_da_parte().lock().ok().and_then(|m| m.get(&(entidade, parte)).map(|s| s.iter().copied().collect())).unwrap_or_default()
}

pub(crate) fn es22_parte_do_componente(entidade: u64, nome_base: u64) -> Option<u64> {
    ES22_COMPONENT_APPEARANCE.lock().ok()?.iter().find(|(k, _)| *k == (entidade, nome_base)).map(|(_, p)| *p)
}

/// Liga o nome base do componente vivo à parte e põe o nome completo na lista da parte; não cria estado na loja de override.
pub(crate) unsafe fn es20_ligar_componente(entidade: u64, inst: *const u8, parte: u64) -> u64 {
    let base = es20_nome_base(inst);
    es22_ligar_componente(entidade, base, parte);
    let nome = (inst.add(0x40) as *const u64).read_unaligned();
    if let Ok(mut m) = es20_componentes_da_parte().lock() {
        m.entry((entidade, parte)).or_default().insert(nome);
    }
    base
}

/// Nome base do CName em `inst+0x40`: o texto antes do 1º `!`/`&`; sem texto conhecido, o próprio hash.
pub(crate) unsafe fn es20_nome_base(inst: *const u8) -> u64 {
    let nome = (inst.add(0x40) as *const u64).read_unaligned();
    chaves_do_componente(nome).0
}

/// `(nome base, prefixo do nome completo)`. O texto vem do espelho do BWMS e depois do pool do motor; sem texto, sem prefixo.
pub(crate) unsafe fn chaves_do_componente(nome: u64) -> (u64, Option<u64>) {
    if nome == 0 {
        return (0, None);
    }
    let Some(texto) = crate::cname::name_of(nome) else { return (nome, None) };
    let base = DynamicAppearanceRef::parse(&texto).name;
    (base, component_prefix_of(&texto))
}

pub(crate) unsafe fn es20_estados_do_componente(
    entidade: u64,
    inst: *const u8,
) -> (u64, Option<bwms_primitives::override_state::ComponentState>, Option<bwms_primitives::override_state::ComponentState>) {
    let (base, prefixo) = chaves_do_componente((inst.add(0x40) as *const u64).read_unaligned());
    let Ok(m) = es20_componentes().lock() else { return (base, None, None) };
    let estado = m.get(&(entidade, base)).cloned();
    let do_prefixo = prefixo.and_then(|p| m.get(&(entidade, p)).cloned());
    (base, estado, do_prefixo)
}

pub(crate) unsafe fn es20_decidir_mascara(entidade: u64, inst: *const u8, original: impl FnOnce() -> u64) -> Option<u64> {
    let (_, estado, prefixo) = es20_estados_do_componente(entidade, inst);
    bwms_primitives::override_state::mascara_aplicada(original, estado.as_ref(), prefixo.as_ref())
}

/// Grava no `chunkMask` a máscara que a loja decide; `None` sem escrita (nulo, desligado, fora das classes de mesh ou nada a decidir). A
/// original é a do cache, lida do próprio campo na 1ª vez.
pub(crate) unsafe fn es20_aplicar_mascara(m: &Es20Malhas, entidade: u64, inst: *mut c_void) -> Option<u64> {
    if inst.is_null() || (inst as *const u8).add(0x8b).read() == 0 {
        return None;
    }
    let (campo, _) = m.campos(es20_tipo(m, inst)?);
    let alvo = (inst as *mut u8).add(campo) as *mut u64;
    let id = es20_id_do_componente(m.base(), inst as *const u8);
    let mascara = es20_decidir_mascara(entidade, inst as *const u8, || es20_original(entidade, id, 0, || alvo.read_unaligned()))?;
    alvo.write_unaligned(mascara);
    Some(mascara)
}

/// O `RefreshAppearance` do componente, só com vtable e slot 81 iguais a um par medido.
pub(crate) unsafe fn es20_refresh_medido(inst: *mut c_void) -> Option<unsafe extern "C" fn(*mut c_void)> {
    if inst.is_null() {
        return None;
    }
    let vtable = (inst as *const u64).read_unaligned();
    if vtable == 0 {
        return None;
    }
    let alvo = ((vtable + 81 * 8) as *const u64).read_unaligned();
    let base = crate::rebase(0x1_0000_0000) as u64;
    let estatico = |x: u64| x.wrapping_sub(base).wrapping_add(0x1_0000_0000);
    ES20_REFRESH_MEDIDOS
        .iter()
        .any(|(v, a)| *v == estatico(vtable) && *a == estatico(alvo))
        .then(|| std::mem::transmute::<usize, unsafe extern "C" fn(*mut c_void)>(alvo as usize))
}


// ===== Estado dinâmico por entidade =====
// O estado sai do `estado_dinamico_do_player`, que chama RTTI e script e só roda na thread do jogo: quem monta pede (`es20_pedir_estado`),
// o tick drena (`es20_pedidos_de_estado`) e guarda (`es20_guardar_estado`). Entre o pedido e o tick, a montagem vê o estado anterior.
/// Valores globais (hash do atributo → valor) e condições.
pub(crate) type Es20EstadoDinamico = (std::collections::HashMap<u64, String>, std::collections::BTreeSet<u64>);

pub(crate) static ES20_FILA_DE_ESTADO: Mutex<Vec<(u64, u64)>> = Mutex::new(Vec::new());

pub(crate) fn es20_estados_dinamicos() -> &'static Mutex<std::collections::HashMap<u64, Es20EstadoDinamico>> {
    static ESTADOS: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, Es20EstadoDinamico>>> = std::sync::OnceLock::new();
    ESTADOS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Pede o recálculo do estado da entidade com o item equipado (o último pedido da entidade vale).
pub(crate) fn es20_pedir_estado(entidade: u64, item: u64) {
    if let Ok(mut f) = ES20_FILA_DE_ESTADO.lock() {
        match f.iter().position(|(e, _)| *e == entidade) {
            Some(i) => f[i].1 = item,
            None if f.len() < 256 => f.push((entidade, item)),
            None => {}
        }
    }
}

/// Os pedidos pendentes, esvaziando a fila.
pub(crate) fn es20_pedidos_de_estado() -> Vec<(u64, u64)> {
    ES20_FILA_DE_ESTADO.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default()
}

pub(crate) fn es20_guardar_estado(entidade: u64, globais: std::collections::HashMap<u64, String>, condicoes: std::collections::BTreeSet<u64>) {
    if let Ok(mut m) = es20_estados_dinamicos().lock() {
        m.insert(entidade, (globais, condicoes));
    }
}

/// O último estado guardado da entidade (`m_states[entidade]`).
pub(crate) fn es20_estado_dinamico(entidade: u64) -> Option<Es20EstadoDinamico> {
    es20_estados_dinamicos().lock().ok()?.get(&entidade).cloned()
}

/// `RemoveState`.
pub(crate) fn es20_esquecer_estado_dinamico(entidade: u64) {
    if let Ok(mut m) = es20_estados_dinamicos().lock() {
        m.remove(&entidade);
    }
}



/// Aparência ativa (nome de aparência analisado) por `(entidade, caminho da parte)`.
pub(crate) static ES22_RESOURCE_STATE: Mutex<Vec<((u64, u64), DynamicAppearanceName)>> = Mutex::new(Vec::new());

/// Critério do nome de um componente vivo: o texto do `CName` pelo pool do motor e `DynamicAppearanceRef::parse`; hash sem texto no pool vira
/// critério não dinâmico com `name = value = hash`.
pub(crate) unsafe fn parse_reference_live(name_hash: u64) -> DynamicAppearanceRef {
    if name_hash == 0 {
        return DynamicAppearanceRef::default();
    }
    // Espelho do BWMS primeiro (nome internado pelo próprio BWMS), depois o pool do motor.
    let s = crate::cname::name_of(name_hash).unwrap_or_default();
    if s.is_empty() {
        return DynamicAppearanceRef { value: name_hash, name: name_hash, ..Default::default() };
    }
    DynamicAppearanceRef::parse(&s)
}

/// Prefixo do nome = os bytes até o 1º `_` nas posições 2..=5, com o `_`. A janela é a que casa as chaves de prefixo da tabela de tags
/// (`h0_`, `heb_`, `beard_`) sem pegar o `_` de nomes completos mais longos.
pub(crate) fn component_prefix_of(name_str: &str) -> Option<u64> {
    let b = name_str.as_bytes();
    let fim = b.iter().take(6).enumerate().skip(2).find(|(_, c)| **c == b'_').map(|(i, _)| i)?;
    Some(bwms_primitives::fnv1a64(&b[..=fim]))
}

/// Índices a desligar: por nome base, entre os que casaram, um grupo com mais de 1 e peso máximo > 0 fica só com o PRIMEIRO (na ordem
/// de entrada) de peso máximo.
pub(crate) fn toggle_conditional_group_disable_indices(entries: &[(u64, i32, bool)]) -> Vec<usize> {
    let mut grupos: Vec<(u64, Vec<usize>)> = Vec::new();
    for (i, (base, _, casou)) in entries.iter().enumerate() {
        if !*casou {
            continue;
        }
        match grupos.iter_mut().find(|(b, _)| b == base) {
            Some((_, v)) => v.push(i),
            None => grupos.push((*base, vec![i])),
        }
    }
    let mut desligar = Vec::new();
    for (_, membros) in grupos.iter().filter(|(_, v)| v.len() > 1) {
        let maximo = membros.iter().map(|&i| entries[i].1).max().unwrap_or(0);
        if maximo <= 0 {
            continue;
        }
        let fica = membros.iter().copied().find(|&i| entries[i].1 == maximo);
        desligar.extend(membros.iter().copied().filter(|&i| Some(i) != fica));
    }
    desligar
}

/// `isEnabled` (`+0x8b`) de cada componente condicional da entidade pela aparência ativa da parte dele; depois o desempate por nome base.
/// Componente não condicional só entra no desempate (pode ser desligado, nunca ligado aqui). Total = componentes avaliados; -1 sem lista.
pub(crate) unsafe fn run_toggle_conditional_components_com(
    _reg: &crate::rtti::Registry,
    entity: *mut c_void,
    condicoes: Option<&std::collections::BTreeSet<u64>>,
) -> (String, i32) {
    if entity.is_null() || !crate::memoria::is_readable(entity as *const c_void, 0xB0) {
        return ("[togglecond] entity ptr insano".to_string(), -1);
    }
    let (e, n, cap) = dynarray_em((entity as *const u8).add(0xA0) as *const c_void);
    if e == 0 || n == 0 || n > 4096 || !dynarray_coerente(e, n, cap) || !crate::memoria::is_readable(e as *const c_void, n as usize * 16) {
        return (format!("[togglecond] entity={entity:p} sem lista de componentes legível (n={n})"), -1);
    }
    let entidade = entity as u64;
    let mut entradas: Vec<(u64, i32, bool)> = Vec::new();
    let mut componentes: Vec<*mut u8> = Vec::new();
    let (mut avaliados, mut casados) = (0, 0);
    for i in 0..n as usize {
        let comp = ((e + i * 16) as *const u64).read_unaligned() as *mut u8;
        if comp.is_null() || !crate::memoria::is_readable(comp as *const c_void, 0x90) {
            continue;
        }
        let referencia = parse_reference_live((comp.add(0x40) as *const u64).read_unaligned());
        if !referencia.is_dynamic || referencia.weight <= 0 {
            entradas.push((referencia.name, 0, true));
            componentes.push(comp);
            continue;
        }
        let Some(parte) = es22_parte_do_componente(entidade, referencia.name) else { continue };
        let ativa = ES22_RESOURCE_STATE.lock().ok().and_then(|g| g.iter().find(|(k, _)| *k == (entidade, parte)).map(|(_, a)| a.clone()));
        let Some(aparencia) = ativa else { continue };
        let casou = appearance_matches_reference_com(&referencia, &aparencia, condicoes);
        comp.add(0x8b).write(u8::from(casou));
        avaliados += 1;
        casados += usize::from(casou);
        entradas.push((referencia.name, referencia.weight, casou));
        componentes.push(comp);
    }
    let desligar = toggle_conditional_group_disable_indices(&entradas);
    for &i in &desligar {
        componentes[i].add(0x8b).write(0);
    }
    (
        format!(
            "[togglecond] entity={entity:p} total_componentes={} condicionais_avaliados={avaliados} condicionais_matched={casados} desabilitados_por_desempate={}",
            entradas.len(),
            desligar.len()
        ),
        entradas.len() as i32,
    )
}

pub(crate) fn is_dynamic_value(s: &str) -> bool {
    s.as_bytes().first() == Some(&b'*')
}

pub(crate) fn process_dynamic_string_single_attr(attr_name: &str, attr_value: &str, input: &str) -> TextoProcessado {
    let alvo = (!attr_name.is_empty()).then(|| bwms_primitives::fnv1a64(attr_name.as_bytes()));
    process_dynamic_string_com(input, |h| (Some(h) == alvo).then_some(attr_value))
}

/// Teto da saída da expansão: texto literal que passaria dele invalida; o valor de atributo que estoura no fim é truncado.
const TEXTO_DINAMICO_MAX: usize = 512;

/// Expande `{atributo}` por `valor_de(hash do nome)`. Um `*` inicial é ignorado; `?` no fim da saída marca opcional. Sem nenhum `{…}`, ou com `{`
/// sem `}`, o texto é inválido.
pub(crate) fn process_dynamic_string_com<'a>(input: &str, valor_de: impl Fn(u64) -> Option<&'a str>) -> TextoProcessado {
    let invalido = TextoProcessado::default();
    let b = input.as_bytes();
    let mut i = usize::from(b.first() == Some(&b'*'));
    let mut saida: Vec<u8> = Vec::with_capacity(TEXTO_DINAMICO_MAX.min(b.len() * 2));
    let mut atributos = std::collections::BTreeSet::new();
    let (mut missed, mut algum) = (false, false);
    // Saída cheia com texto por consumir = inválido; um valor truncado também conta como cheio.
    let mut cheia = false;
    while i < b.len() {
        if cheia || saida.len() >= TEXTO_DINAMICO_MAX {
            return invalido;
        }
        if b[i] != b'{' {
            saida.push(b[i]);
            i += 1;
            continue;
        }
        let Some(fim) = b[i + 1..].iter().position(|&c| c == b'}').map(|p| i + 1 + p) else { return invalido };
        let h = bwms_primitives::fnv1a64(&b[i + 1..fim]);
        atributos.insert(h);
        algum = true;
        match valor_de(h) {
            Some(v) => {
                let livre = TEXTO_DINAMICO_MAX - saida.len();
                if v.len() > livre {
                    let corte = (0..=livre).rev().find(|&c| v.is_char_boundary(c)).unwrap_or(0);
                    saida.extend_from_slice(&v.as_bytes()[..corte]);
                    cheia = true;
                } else {
                    saida.extend_from_slice(v.as_bytes());
                }
            }
            None => missed = true,
        }
        i = fim + 1;
    }
    if !algum {
        return invalido;
    }
    let optional = saida.last() == Some(&b'?');
    if optional {
        saida.pop();
    }
    // Byte de texto literal cortado no teto só acontece no ramo inválido; a saída válida é UTF-8 inteiro.
    let Ok(value) = String::from_utf8(saida) else { return invalido };
    TextoProcessado { valid: true, missed, optional, value, atributos }
}

/// Critério de nome de componente/aparência (`nome!variantes&condições`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DynamicAppearanceRef {
    #[allow(dead_code)]
    pub(crate) value: u64,
    pub(crate) name: u64,
    pub(crate) variants: std::collections::BTreeSet<u64>,
    #[allow(dead_code)]
    pub(crate) conditions: std::collections::BTreeSet<u64>,
    pub(crate) is_dynamic: bool,
    #[allow(dead_code)]
    pub(crate) is_conditional: bool,
    pub(crate) weight: i32,
}

/// Hash de um trecho do nome: vazio = 0 (o "nenhum" do formato).
pub(crate) fn dynref_extract_name_hash(s: &str) -> u64 {
    if s.is_empty() { 0 } else { bwms_primitives::fnv1a64(s.as_bytes()) }
}

/// Peso que a variante dá ao critério; cada condição soma 1.
const PESO_DA_VARIANTE: i32 = 100;

impl DynamicAppearanceRef {
    /// `nome!v1!v2&c1&c2`: as variantes vão até o 1º `&`; na parte de condição o `!` é texto. Trecho vazio entre marcadores vale 0.
    pub(crate) fn parse(reference: &str) -> Self {
        let value = dynref_extract_name_hash(reference);
        let Some(marca) = reference.find(['!', '&']) else {
            return Self { value, name: value, ..Default::default() };
        };
        let pedacos = |trecho: &str, sep: char| -> std::collections::BTreeSet<u64> {
            if trecho.is_empty() { Default::default() } else { trecho.split(sep).map(dynref_extract_name_hash).collect() }
        };
        let resto = &reference[marca..];
        let (variantes, condicoes) = match resto.strip_prefix('!') {
            Some(v) => match v.find('&') {
                Some(e) => (&v[..e], &v[e + 1..]),
                None => (v, ""),
            },
            None => ("", &resto[1..]),
        };
        let variants = pedacos(variantes, '!');
        let conditions = pedacos(condicoes, '&');
        // Sem estouro: a variante sempre pesa mais que qualquer número de condições sem ela.
        let weight = if variants.is_empty() { 0 } else { PESO_DA_VARIANTE }.saturating_add(i32::try_from(conditions.len()).unwrap_or(i32::MAX));
        Self {
            value,
            name: dynref_extract_name_hash(&reference[..marca]),
            variants,
            conditions,
            is_dynamic: true,
            is_conditional: weight > 0,
            weight,
        }
    }

    pub(crate) fn matches_variant(&self, variant_hash: u64) -> bool {
        self.variants.contains(&variant_hash)
    }

    /// Cada condição do critério está no estado da entidade ou nos overrides do nome pedido.
    pub(crate) fn matches_conditions_with_overrides(
        &self,
        conditions_present: &std::collections::BTreeSet<u64>,
        overrides: &std::collections::BTreeSet<u64>,
    ) -> bool {
        self.conditions.iter().all(|c| conditions_present.contains(c) || overrides.contains(c))
    }
}

/// Valor de nome de aparência (`nome!variante+partes%contexto&condições`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct DynamicAppearanceName {
    #[allow(dead_code)]
    pub(crate) value: u64,
    pub(crate) name: u64,
    pub(crate) variant: u64,
    pub(crate) parts: std::collections::BTreeMap<u64, u64>,
    pub(crate) overrides: std::collections::BTreeSet<u64>,
    pub(crate) context: u64,
    pub(crate) is_dynamic: bool,
}

impl DynamicAppearanceName {
    /// `nome!variante[+parte|+chave=valor]…[&condição…][%contexto]`. O contexto é o número depois do ÚLTIMO `%`; as condições do nome não são
    /// guardadas. `!` depois do corte (`%` ou `&`) não abre variante: o texto inteiro vale como nome não dinâmico.
    pub(crate) fn parse(appearance: &str) -> Self {
        let value = dynref_extract_name_hash(appearance);
        let nao_dinamico = Self { value, name: value, ..Default::default() };
        let (corpo, contexto) = match appearance.rfind('%') {
            Some(p) => (&appearance[..p], &appearance[p + 1..]),
            None => (appearance, ""),
        };
        let corpo = corpo.find('&').map_or(corpo, |e| &corpo[..e]);
        let Some(bang) = corpo.find('!') else { return nao_dinamico };
        let context = if !contexto.is_empty() && contexto.bytes().all(|c| c.is_ascii_digit()) { contexto.parse::<u64>().unwrap_or(0) } else { 0 };
        let mut nome = Self { value, name: dynref_extract_name_hash(&corpo[..bang]), context, is_dynamic: true, ..Default::default() };
        let secao = &corpo[bang + 1..];
        if secao.is_empty() {
            return nome;
        }
        nome.variant = dynref_extract_name_hash(secao);
        nome.parts.insert(crate::cname::cname("variant"), nome.variant);
        // Pedaço sem `=` entra como `variant.<c>`, com `c` um byte que começa em '1' e sobe um código por pedaço.
        let mut contador = b'1';
        for pedaco in secao.split('+').filter(|p| !p.is_empty()) {
            match pedaco.find('=') {
                None => {
                    let mut chave = *b"variant._";
                    chave[8] = contador;
                    nome.parts.insert(bwms_primitives::fnv1a64(&chave), dynref_extract_name_hash(pedaco));
                    contador = contador.wrapping_add(1);
                }
                Some(eq) => {
                    nome.parts.insert(dynref_extract_name_hash(&pedaco[..eq]), dynref_extract_name_hash(&pedaco[eq + 1..]));
                    // O override é o hash dos primeiros `tamanho - posição do '='` bytes do pedaço: é contra esse hash que os critérios `&x`
                    // dos mods casam nos componentes condicionais.
                    nome.overrides.insert(bwms_primitives::fnv1a64(&pedaco.as_bytes()[..pedaco.len() - eq]));
                }
            }
        }
        nome
    }
}

/// Offsets de roupa por recurso, um por mod.
#[derive(Debug, Clone, Default)]
pub(crate) struct Es67ResourceOffset {
    /// `(mod_hash, offset)`.
    pub(crate) overrides: Vec<(u64, i32)>,
}

impl Es67ResourceOffset {
    /// Um valor por mod; o mesmo mod atualiza no lugar, sem perder a posição.
    pub(crate) fn add_offset_override(&mut self, mod_hash: u64, offset: i32) {
        match self.overrides.iter_mut().find(|(m, _)| *m == mod_hash) {
            Some((_, o)) => *o = offset,
            None => self.overrides.push((mod_hash, offset)),
        }
    }

    pub(crate) fn remove_offset_override(&mut self, mod_hash: u64) -> bool {
        let antes = self.overrides.len();
        self.overrides.retain(|(m, _)| *m != mod_hash);
        self.overrides.len() != antes
    }

    /// O do mod que entrou primeiro: a roupa fica com o deslocamento que o 1º mod carregado pediu.
    pub(crate) fn get_overridden_offset(&self) -> i32 {
        self.overrides.first().map_or(0, |(_, o)| *o)
    }
}

/// Chave = `(entidade, hash do caminho do recurso)`.
pub(crate) static ES67_RESOURCE_OFFSET: Mutex<Vec<((u64, u64), Es67ResourceOffset)>> = Mutex::new(Vec::new());

pub(crate) unsafe fn add_offset_override(entity: u64, mod_hash: u64, resource_path: u64, offset: i32) {
    let Ok(mut g) = ES67_RESOURCE_OFFSET.lock() else { return };
    let chave = (entity, resource_path);
    match g.iter_mut().find(|(k, _)| *k == chave) {
        Some((_, s)) => s.add_offset_override(mod_hash, offset),
        None => {
            let mut s = Es67ResourceOffset::default();
            s.add_offset_override(mod_hash, offset);
            g.push((chave, s));
        }
    }
}

/// Tira o mod de todos os recursos da entidade; devolve quantos recursos tinham o mod.
pub(crate) unsafe fn remove_offset_overrides(entity: u64, mod_hash: u64) -> u32 {
    let Ok(mut g) = ES67_RESOURCE_OFFSET.lock() else { return 0 };
    g.iter_mut().filter(|((e, _), _)| *e == entity).map(|(_, s)| u32::from(s.remove_offset_override(mod_hash))).sum()
}

pub(crate) unsafe fn get_offset_override(entity: u64, resource_path: u64) -> i32 {
    ES67_RESOURCE_OFFSET.lock().ok().and_then(|g| g.iter().find(|(k, _)| *k == (entity, resource_path)).map(|(_, s)| s.get_overridden_offset())).unwrap_or(0)
}

/// Tamanho da tabela sem travar: com 0 os hooks só repassam (o `m_customAppOverrides.empty()`).
pub(crate) static APP_OVERRIDES_N: AtomicU64 = AtomicU64::new(0);

pub(crate) fn app_overrides() -> &'static std::sync::RwLock<std::collections::HashMap<(u64, u64), u64>> {
    static T: std::sync::OnceLock<std::sync::RwLock<std::collections::HashMap<(u64, u64), u64>>> = std::sync::OnceLock::new();
    T.get_or_init(Default::default)
}

/// `RegisterAppOverride`: `(recurso, aparência)` passa a carregar `substituto`.
pub(crate) fn registrar_app_override(recurso: u64, substituto: u64, aparencia: u64) {
    if let Ok(mut t) = app_overrides().write() {
        t.insert((recurso, aparencia), substituto);
        APP_OVERRIDES_N.store(t.len() as u64, Ordering::Relaxed);
    }
}

/// Tira o override de `(recurso, aparência)`; `true` se havia.
pub(crate) fn remover_app_override(recurso: u64, aparencia: u64) -> bool {
    let Ok(mut t) = app_overrides().write() else { return false };
    let havia = t.remove(&(recurso, aparencia)).is_some();
    APP_OVERRIDES_N.store(t.len() as u64, Ordering::Relaxed);
    havia
}

/// `ApplyAppOverride` num descritor `[recurso, aparência]`: só com os 2 campos e com override. `true` se trocou.
pub(crate) fn trocar_por_app_override(desc: &mut [u64; 2], substituto: impl Fn(u64, u64) -> Option<u64>) -> bool {
    if desc[0] == 0 || desc[1] == 0 {
        return false;
    }
    substituto(desc[0], desc[1]).map(|s| desc[0] = s).is_some()
}

/// Elementos de uma `Range<AppearanceDescriptor>` passada por valor (início, fim); `None` com range nulo ou torto.
pub(crate) fn descritores_do_range(inicio: u64, fim: u64) -> Option<usize> {
    let bytes = fim.checked_sub(inicio)?;
    (inicio != 0 && bytes % 16 == 0 && bytes / 16 <= 4096).then_some((bytes / 16) as usize)
}

/// Aplica os overrides em `n` descritores a partir de `p`; devolve quantos trocaram.
pub(crate) unsafe fn aplicar_app_overrides(p: *mut u64, n: usize) -> usize {
    if APP_OVERRIDES_N.load(Ordering::Relaxed) == 0 || p.is_null() {
        return 0;
    }
    let Ok(t) = app_overrides().read() else { return 0 };
    let mut trocados = 0;
    for i in 0..n {
        let d = p.add(i * 2) as *mut [u64; 2];
        let mut v = d.read_unaligned();
        if trocar_por_app_override(&mut v, |r, a| t.get(&(r, a)).copied()) {
            d.write_unaligned(v);
            trocados += 1;
        }
    }
    trocados
}

/// Aliases do teste e resoluções deles pelo `resource.link` (`reslink_lookup`).
pub(crate) static ALIASES_DO_TESTE_DE_OVERRIDE: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];

#[cfg(any(test, feature = "devlog"))]
pub(crate) static ALIASES_DO_TESTE_RESOLVIDOS: AtomicU64 = AtomicU64::new(0);

/// `reslink_lookup`: conta a resolução de um alias do teste.
#[cfg(any(test, feature = "devlog"))]
pub(crate) fn contar_alias_do_teste_resolvido(src: u64) {
    if src != 0 && ALIASES_DO_TESTE_DE_OVERRIDE.iter().any(|a| a.load(Ordering::Relaxed) == src) {
        ALIASES_DO_TESTE_RESOLVIDOS.fetch_add(1, Ordering::Relaxed);
    }
}


// Controller de recurso: IComponent::Toggle native scripting handler vmaddr
// Achado 2026-07-28: classe "IComponent" NÃO existe por esse nome no RTTI (nativefunc IComponent
// Toggle -> "classe não encontrada"); mas `nativefunc gameuiCharacterCustomizationGenitalsController
// Toggle` resolve (Toggle está na hierarquia herdada). rf.func @+0x00 = CName hash duplicado
// (não código); campo +0x38 (e +0x70, idêntico) aponta pra um endereço que peekq confirma ser
// DADO (array de 4 ponteiros), não código — dereferenciando esses 4 ponteiros e comparando contra
// symbols-demangled.txt, todos caem dentro de `red::memory::PoolStorageProxy<red::PoolRTTIFunction>`
// (Allocate/AllocateAligned/Reallocate/Free) — ou seja, é o vtable do proxy de alocação embutido
// na struct CClassFunction, NÃO o ponteiro de Toggle. Pista falsa, descartada. +0x40 (offset que o
// plano original esperava) leu 0x0 nesta classe — layout não bate com o assumido. Ainda não achado.
// RESOLVIDO 2026-07-28 (RE estática offline dedicada, string-xref+disasm em __cstring/__text):
// `IComponent::Toggle` (scripting handler nativo) = 0x1019ba498. Prólogo real confirmado
// (STP x29,x30 + SUB SP,#0x30), NÃO é thunk. Cross-validado 4x: ordem dos 8 métodos nativos de
// IComponent no binário bate 1:1 com a ordem declarada em redscript-src/orphans.script:16657;
// flag w6=0x2000 bate com `const`/não-const de cada método; IsEnabled e Toggle tocam o mesmo
// campo this+0x8b (Toggle grava, IsEnabled só lê); Toggle consome exatamente 1 param Bool.
// Bônus da mesma RE (não usado ainda): GetAppearanceName 0x1019ba220, GetEntity 0x1019ba23c,
// FindComponentByName 0x1019ba358, GetName 0x1019ba460, IsEnabled 0x1019ba47c,
// QueueEntityEvent 0x1019ba538, RegisterRenderDebug 0x1019ba5f8.
pub(crate) const ICOMP_TOGGLE_VM: u64 = 0x1_019b_a498;

/// Original do handler de script do `IComponent::Toggle`, publicado pelo `instalar_com_threads_paradas` (`garantir_hooks_de_toggle`).
pub(crate) static ORIG_ICOMP_TOGGLE: AtomicU64 = AtomicU64::new(0);

// ===== `Component/Toggle` e `Vehicle/ToggleAuxLights` =====
// A original roda e, com ouvinte do evento, o evento sai com o componente (ou o controller do veículo) e a
// entidade dona (`owner` em `+0x50` do `entIComponent`; o `vehicleController` herda `gameComponent`), só quando há dono. No Mac o `IComponent::Toggle`
// C++ (`0x1019b95f4`: compara com `+0x8b`, grava e, com o bit 1 de `+0x88`, chama o virtual `+0x1e8`) e o handler de script do `Toggle` (`0x1019ba498`)
// têm o mesmo corpo em linha — o handler não chama a função —, então os 2 são hookados. O `vehicleController::ToggleLights` (`0x102b3d974`, RE de
// 2026-07-28) recebe `this`, o `on` e o tipo de luz em `w2`. Os 3 são instalados no primeiro `RegisterCallback` do evento e só leem o atômico de ouvinte.
pub(crate) const ICOMP_TOGGLE_PROLOGO: u32 = 0xd100_c3ff;

pub(crate) const ICOMP_TOGGLE_CPP_VM: u64 = 0x1_019b_95f4;

pub(crate) const ICOMP_TOGGLE_CPP_PROLOGO: u32 = 0x3942_2c08;

pub(crate) static ORIG_ICOMP_TOGGLE_CPP: AtomicU64 = AtomicU64::new(0);

/// `Component/Toggle` com ouvinte; `hook` 0 é o handler de script, 1 a função C++ (contados pro teste 134).
pub(crate) unsafe fn toggle_despachar(componente: *mut c_void, hook: usize) {
    if !componente.is_null()
        && crate::aparencia::COMPONENT_TOGGLE_ARMADO.load(Ordering::Relaxed)
        && !C134E_CONGELADO.load(Ordering::Relaxed)
        && crate::aparencia::disparar_toggle_de_componente(componente) > 0
    {
        C134E_TOGGLE_POR_HOOK[hook].fetch_add(1, Ordering::Relaxed);
    }
}

/// Handler de script do `Toggle` (`ScriptingFunction_t`: contexto, frame, retorno, tipo do retorno).
pub(crate) unsafe extern "C" fn icomp_toggle_handler(ctx: *mut c_void, frame: *mut c_void, ret: *mut c_void, ret_type: *mut c_void) {
    let f: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(original_publicado(&ORIG_ICOMP_TOGGLE));
    f(ctx, frame, ret, ret_type);
    toggle_despachar(ctx, 0);
}

pub(crate) unsafe extern "C" fn icomp_toggle_cpp(componente: *mut c_void, ligado: u8) {
    let f: unsafe extern "C" fn(*mut c_void, u8) = std::mem::transmute(original_publicado(&ORIG_ICOMP_TOGGLE_CPP));
    f(componente, ligado);
    toggle_despachar(componente, 1);
}

/// Instala os 2 hooks do `Toggle` no `RegisterCallback` de `Component/Toggle`, de novo depois de uma desinstalação. Quem chama
/// serializa ativação e desativação.
pub(crate) unsafe fn garantir_hooks_de_toggle() {
    let script = instalar_conferindo_prologo(ICOMP_TOGGLE_VM, ICOMP_TOGGLE_PROLOGO, icomp_toggle_handler as *mut c_void, &ORIG_ICOMP_TOGGLE);
    let cpp = instalar_conferindo_prologo(ICOMP_TOGGLE_CPP_VM, ICOMP_TOGGLE_CPP_PROLOGO, icomp_toggle_cpp as *mut c_void, &ORIG_ICOMP_TOGGLE_CPP);
    if script.is_some() || cpp.is_some() {
        crate::veredito_dev!("[toggle] hooks do IComponent::Toggle pelo RegisterCallback (None = já no ar): handler de script {script:?}, função C++ {cpp:?}");
    }
}


pub(crate) const CMESH_POSTLOAD_VM: u64 = 0x1_0519_a58c;

// ===== `ExtractPartComponents` ao vivo, partes 17a e 17b (2026-09-12) =====
// 17a, com `~/.bwms-hook-a17-partes`: hook só observando em `0x1017d3158` (Mac: `x0` = &SharedPtr<ResourceToken<EntityTemplate>> da parte, `x8` = DynArray
// de saída; RE em `proofs/20260911-*-17-extractpartcomponents-sret-identificado.log`). Um trampolim nu passa o `x8` do chamador como 2º argumento; o
// hook chama a original com ele explícito (`extract_part_components_em`) e guarda, das primeiras partes com objetos, o token (com 1 strong a mais, pra seguir
// vivo), o campo `+0x48` do token, o tamanho do array e as classes dos objetos. 17b, no tick de gameplay: chama a original de novo com o token guardado e
// confere tamanho e classes com o que o motor extraiu; controle com o token de um caminho que não existe (token com falha: array vazio). É a base da 17c — o
// `resource.patch` se aplica extraindo os componentes do `.ent` de patch e mesclando por nome e id no array da parte. Os arrays das chamadas do teste
// ficam sem soltar (vazamento pequeno e consciente de teste).
pub(crate) const EXTRACT_PART_COMPONENTS_VM: u64 = 0x1_017d_3158;

pub(crate) static ORIG_EXTRACT_PART_COMPONENTS: AtomicU64 = AtomicU64::new(0);

/// Chama a original do `ExtractPartComponents` com a saída explícita no `x8`: `saida` (16 B, `DynArray<Handle<ISerializable>>` vazio) recebe os objetos. Sem
/// struct de retorno em Rust: no build da 17d o `-> SretPad32` do hook deixou de repassar o `x8` do chamador (a original escrevia num local e a volta copiava
/// 32 B num array de 16 B, visto no desmonte antes do deploy), então a saída vai sempre por ponteiro.
pub(crate) unsafe fn extract_part_components_em(orig: *mut c_void, token: *const [u64; 2], saida: *mut [u64; 2]) {
    core::arch::asm!(
        "blr {f}",
        f = in(reg) orig as u64,
        in("x0") token as u64,
        in("x8") saida as u64,
        clobber_abi("C"),
    );
}

/// Entrada do hook: o `x8` do chamador (a saída) vira o 2º argumento do `extract_part_components_hook`, que volta direto pro chamador.
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn extract_part_components_tramp() {
    core::arch::naked_asm!("mov x1, x8", "b {hook}", hook = sym extract_part_components_hook)
}

pub(crate) unsafe extern "C" fn extract_part_components_hook(token: *const [u64; 2], saida: *mut [u64; 2]) {
    let orig = original_publicado(&ORIG_EXTRACT_PART_COMPONENTS) as *mut c_void;
    extract_part_components_em(orig, token, saida);
    let n = A17_CHAMADAS.fetch_add(1, Ordering::Relaxed);
    let tam = ((*saida)[1] >> 32) as u32;
    if A17_OBSERVAR.load(Ordering::Relaxed) && n < 256 && tam > 0 && !token.is_null() && crate::memoria::is_readable(token as *const c_void, 16) {
        if let Ok(mut v) = A17_VISTAS.try_lock() {
            if v.len() < 4 {
                let par = *token;
                let campo = if par[0] != 0 && crate::memoria::is_readable((par[0] + 0x48) as *const c_void, 8) { ((par[0] + 0x48) as *const u64).read_unaligned() } else { 0 };
                if par[1] != 0 && crate::memoria::is_readable(par[1] as *const c_void, 4) {
                    (*(par[1] as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
                    v.push((par, campo, tam, a17_classes((*saida)[0], tam, 32)));
                }
            }
        }
    }
    if PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) {
        aplicar_patches_na_parte(orig, token, saida);
    }
}

/// Passo da mescla pra um objeto do patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PassoDaMescla {
    Substitui(usize),
    Anexa,
    Pula,
}

/// Um passo por objeto do patch, na ordem do patch. O componente casa pela chave `(name, id)` com a 1ª posição do resultado, ou com um anexado
/// antes no mesmo patch (que ocupa `resultado.len() + ordem de anexação`): dois objetos do patch com a mesma chave viram um anexado e uma troca dele.
/// Em mescla de parte o componente externo novo não entra.
pub(crate) fn plano_da_mescla(resultado: &[Option<(u64, u64)>], patch: &[Option<(u64, u64, bool)>], parte: bool) -> Vec<PassoDaMescla> {
    let mut anexadas: Vec<(u64, u64)> = Vec::new();
    let mut plano = Vec::with_capacity(patch.len());
    for objeto in patch {
        let Some((nome, id, externo)) = *objeto else {
            plano.push(PassoDaMescla::Pula);
            continue;
        };
        let chave = (nome, id);
        let passo = if let Some(i) = resultado.iter().position(|r| *r == Some(chave)) {
            PassoDaMescla::Substitui(i)
        } else if let Some(j) = anexadas.iter().position(|a| *a == chave) {
            PassoDaMescla::Substitui(resultado.len() + j)
        } else if parte && externo {
            PassoDaMescla::Pula
        } else {
            anexadas.push(chave);
            PassoDaMescla::Anexa
        };
        plano.push(passo);
    }
    plano
}

/// O plano sobre cópias dos dois arrays. Substituir TROCA os handles: o do alvo vai para o array do patch e é solto com ele, então nenhuma
/// contagem de referência muda. O anexado deixa `[0, 0]` no patch pelo mesmo motivo. Passo além do patch ou troca fora do resultado não mexe.
pub(crate) fn arrays_com_o_plano(resultado: &[[u64; 2]], patch: &[[u64; 2]], plano: &[PassoDaMescla]) -> (Vec<[u64; 2]>, Vec<[u64; 2]>) {
    let mut r = Vec::with_capacity(resultado.len() + patch.len());
    r.extend_from_slice(resultado);
    let mut p = patch.to_vec();
    for (j, passo) in plano.iter().enumerate().take(p.len()) {
        match *passo {
            PassoDaMescla::Substitui(i) if i < r.len() => core::mem::swap(&mut r[i], &mut p[j]),
            PassoDaMescla::Anexa => {
                r.push(p[j]);
                p[j] = [0, 0];
            }
            _ => {}
        }
    }
    (r, p)
}

/// `entIComponent` lido em `name` (`+0x40`) e `id` (`+0x60`).
const MESCLA_COMPONENTE_LEGIVEL: usize = 0x68;

/// `(name, id, é entExternalComponent)` do objeto; `None` para objeto ilegível ou que não é componente. Sem as classes guardadas
/// (`guardar_classes_da_mescla`) nada é componente e o plano só pula.
pub(crate) unsafe fn chave_de_mescla(obj: u64) -> Option<(u64, u64, bool)> {
    let componente = CLASSE_DE_COMPONENTE.load(Ordering::Relaxed);
    if obj == 0 || componente.is_null() || !crate::memoria::is_readable(obj as *const c_void, MESCLA_COMPONENTE_LEGIVEL) {
        return None;
    }
    let classe = rtti::class_of(obj as *mut c_void);
    if !crate::register::class_is_kind_of(classe, componente) {
        return None;
    }
    let base = obj as *const u8;
    let nome = base.add(0x40).cast::<u64>().read_unaligned();
    let id = base.add(0x60).cast::<u64>().read_unaligned();
    let externo = crate::register::class_is_kind_of(classe, CLASSE_DE_COMPONENTE_EXTERNO.load(Ordering::Relaxed));
    Some((nome, id, externo))
}

/// Teto de objetos de um array de extração: acima disso o cabeçalho é lixo, não um pacote.
const MESCLA_TETO_DE_OBJETOS: u32 = 0x4000;

/// Tamanho de um `DynArray` (`+0x0C`), `None` com o cabeçalho ilegível.
unsafe fn tamanho_do_dynarray(arr: *const u8) -> Option<u32> {
    if arr.is_null() || !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return None;
    }
    Some(arr.add(0x0C).cast::<u32>().read_unaligned())
}

/// Os handles `{instância, bloco}` de um `DynArray<Handle<ISerializable>>`, nulos inclusive (a posição importa para o plano). Vazio com o array
/// ilegível, sem buffer ou acima do teto.
pub(crate) unsafe fn handles_do_dynarray(arr: *const u8) -> Vec<[u64; 2]> {
    let Some(tamanho) = tamanho_do_dynarray(arr) else { return Vec::new() };
    let buffer = arr.cast::<u64>().read_unaligned() as *const u64;
    if tamanho == 0 || tamanho > MESCLA_TETO_DE_OBJETOS || buffer.is_null() || !crate::memoria::is_readable(buffer as *const c_void, tamanho as usize * 16) {
        return Vec::new();
    }
    (0..tamanho as usize).map(|i| [buffer.add(i * 2).read_unaligned(), buffer.add(i * 2 + 1).read_unaligned()]).collect()
}

/// Mescla os componentes do `patch` no `resultado` (os dois `DynArray<Handle<ISerializable>>` de uma extração) e devolve o plano aplicado.
/// `None`, sem escrever nada, quando um array não se lê inteiro ou o resultado não cresce. Os handles só trocam de lugar: soltar o `patch`
/// depois (`soltar_dynarray_de_handles`) solta exatamente os substituídos. O tamanho do patch não muda: as vagas anexadas ficam `[0, 0]`.
pub(crate) unsafe fn mesclar_componentes(resultado: *mut u8, patch: *mut u8, parte: bool) -> Option<Vec<PassoDaMescla>> {
    let r = handles_do_dynarray(resultado);
    let p = handles_do_dynarray(patch);
    if tamanho_do_dynarray(resultado)? as usize != r.len() || tamanho_do_dynarray(patch)? as usize != p.len() {
        return None;
    }
    let chaves_r: Vec<Option<(u64, u64)>> = r.iter().map(|h| chave_de_mescla(h[0]).map(|(nome, id, _)| (nome, id))).collect();
    let chaves_p: Vec<Option<(u64, u64, bool)>> = p.iter().map(|h| chave_de_mescla(h[0])).collect();
    let plano = plano_da_mescla(&chaves_r, &chaves_p, parte);
    let (novo_r, novo_p) = arrays_com_o_plano(&r, &p, &plano);
    let (_, _, capacidade) = crate::jogo::listas::dynarray_no_campo(resultado, 0);
    if novo_r.len() > capacidade {
        // Com elementos, o reserve do motor tem de mover handles pelo mover do próprio tipo; vazio, não há o que mover.
        let mover = if r.is_empty() { std::ptr::null() } else { crate::rebase(MOVER_HANDLES_VM) as *const c_void };
        crate::jogo::listas::reservar(resultado as *mut c_void, novo_r.len() as u32, 0x10, 8, mover);
    }
    let (buffer_r, _, capacidade) = crate::jogo::listas::dynarray_no_campo(resultado, 0);
    if novo_r.len() > capacidade || (!novo_r.is_empty() && (buffer_r.is_null() || !crate::memoria::is_readable(buffer_r as *const c_void, novo_r.len() * 16))) {
        return None;
    }
    let (buffer_p, _, _) = crate::jogo::listas::dynarray_no_campo(patch, 0);
    if !novo_r.is_empty() {
        core::ptr::copy_nonoverlapping(novo_r.as_ptr() as *const u8, buffer_r, novo_r.len() * 16);
    }
    if !novo_p.is_empty() {
        core::ptr::copy_nonoverlapping(novo_p.as_ptr() as *const u8, buffer_p, novo_p.len() * 16);
    }
    resultado.add(0x0C).cast::<u32>().write_unaligned(novo_r.len() as u32);
    Some(plano)
}

/// Solta um `DynArray<Handle<ISerializable>>` pelo `Destruct` do tipo (`vt+0x48`): handles e buffer pelo motor. `true` se o `Destruct` foi chamado.
pub(crate) unsafe fn soltar_dynarray_de_handles(arr: *mut u8) -> bool {
    let tipo = TIPO_DO_ARRAY_DE_HANDLES.load(Ordering::Relaxed);
    if !crate::rtti::sane(tipo) {
        return false;
    }
    let vt = (tipo as *const u64).read_unaligned() as *const u64;
    let destruct = if vt.is_null() { 0 } else { vt.add(9).read_unaligned() };
    if !crate::rtti::sane(destruct as *mut c_void) {
        return false;
    }
    let d: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(destruct);
    d(tipo, arr as *mut c_void);
    true
}

/// Classes e tipo da mescla guardados (game thread), com os offsets de `name` e `id` conferidos pelo RTTI contra `+0x40`/`+0x60`. `Err` com o motivo, sem
/// guardar nada.
pub(crate) unsafe fn guardar_classes_da_mescla(reg: &crate::rtti::Registry) -> Result<(), String> {
    let componente = reg.class_by_name("entIComponent");
    let externo = reg.class_by_name("entExternalComponent");
    let tipo = crate::register::get_type(reg, "array:handle:ISerializable");
    let offset = |campo: &str| if componente.is_null() { None } else { crate::rtti::resolve_prop_in_class(componente, campo).map(|(o, _, _)| o) };
    let (nome_off, id_off) = (offset("name"), offset("id"));
    if componente.is_null() || externo.is_null() || !crate::rtti::sane(tipo) || nome_off != Some(0x40) || id_off != Some(0x60) {
        return Err(format!(
            "entIComponent {componente:p}, entExternalComponent {externo:p}, tipo {tipo:p}, name {:?}, id {:?}",
            nome_off.map(|o| format!("{o:#x}")),
            id_off.map(|o| format!("{o:#x}"))
        ));
    }
    CLASSE_DE_COMPONENTE.store(componente, Ordering::Relaxed);
    CLASSE_DE_COMPONENTE_EXTERNO.store(externo, Ordering::Relaxed);
    TIPO_DO_ARRAY_DE_HANDLES.store(tipo, Ordering::Relaxed);
    CLASSE_ENTENTITY.store(reg.class_by_name("entEntity"), Ordering::Relaxed);
    Ok(())
}

pub(crate) const GARMENT_EXTRACT_JOB_VM: u64 = 0x1_017e_e34c;

pub(crate) static ORIG_GARMENT_EXTRACT_JOB: AtomicU64 = AtomicU64::new(0);

pub(crate) type GarmentExtractJobFn = unsafe extern "C" fn(*mut [u64; 2], *const c_void);

/// Job de extração do montador de roupa: a roupa do V não passa pela extração de parte, então o `components` do `resource.patch` entra como
/// extração extra no mesmo contexto de montagem — a original roda de novo uma vez por patch, com params novos que apontam o template do patch.
pub(crate) unsafe extern "C" fn garment_extract_job_hook(params: *mut [u64; 2], grupo: *const c_void) {
    let original: GarmentExtractJobFn = std::mem::transmute(original_publicado(&ORIG_GARMENT_EXTRACT_JOB));
    if !PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) || params.is_null() || !crate::memoria::is_readable(params as *const c_void, 0x30) {
        original(params, grupo);
        return;
    }
    crate::diagnostico::a17_garment_observar(params);
    // O job solta os params no fim: tudo o que se lê deles vem antes da original.
    let tpl = (*params)[0];
    let contexto = (*params.add(1))[0];
    let caminho = crate::jogo::recursos::hash_do_caminho_do_recurso(tpl);
    let blocos = params_do_job_por_patch(params);
    let antes = crate::diagnostico::a17e_alvos_de_skin(contexto);
    original(params, grupo);
    if blocos.is_empty() {
        return;
    }
    let depois_da_parte = crate::diagnostico::a17e_alvos_de_skin(contexto);
    for bloco in blocos {
        original(bloco, grupo);
        crate::diagnostico::A17_GARMENT_APLICADOS.fetch_add(1, Ordering::Relaxed);
    }
    crate::diagnostico::a17e_amostrar(contexto, caminho, antes, depois_da_parte);
}

/// `PoolStorageProxy<job::PoolJobs2Data>::Allocate(tamanho)` e `Free<job::PoolJobs2Data>(bloco, 1)` (stubs; o agendador `0x1017ee098` aloca os params assim).
pub(crate) const POOL_JOBS2_ALLOCATE_VM: u64 = 0x1_052a_488c;

pub(crate) const POOL_JOBS2_FREE_VM: u64 = 0x1_052a_6584;

/// Recurso carregado do token (`+0x28`), sem espera; 0 se o token ainda não carregou.
unsafe fn recurso_do_token(t: [u64; 2]) -> u64 {
    if t[0] == 0 || crate::register::status_do_token(t[0]) != 1 || !crate::memoria::is_readable((t[0] + 0x28) as *const c_void, 0x10) {
        return 0;
    }
    ((t[0] + 0x28) as *const u64).read_unaligned()
}

/// Um bloco novo de params (0x30 B, do pool dos params de job) por patch de `components` da parte: cópia dos params com o template do patch em
/// `+0x00`. O job solta uma referência fraca do template e do contexto no fim, então cada bloco soma uma em cada. Falha num passo devolve o
/// bloco ao pool e pula o patch.
pub(crate) unsafe fn params_do_job_por_patch(params: *mut [u64; 2]) -> Vec<*mut [u64; 2]> {
    let tpl = (*params)[0];
    let caminho = crate::jogo::recursos::hash_do_caminho_do_recurso(tpl);
    let tokens: Vec<[u64; 2]> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo_ou_da_instancia(Some(caminho), tpl, |p| p.props & A17_PROP_COMPONENTS != 0, |t| recurso_do_token(t)).map(|p| p.token).collect(),
        Err(_) => return Vec::new(),
    };
    if tokens.is_empty() {
        return Vec::new();
    }
    let alocar: unsafe extern "C" fn(u64) -> *mut u8 = std::mem::transmute(crate::rebase(POOL_JOBS2_ALLOCATE_VM));
    let liberar: unsafe extern "C" fn(*mut u8, u32) = std::mem::transmute(crate::rebase(POOL_JOBS2_FREE_VM));
    let classe = CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed);
    let contexto_rc = (*params.add(1))[1];
    let mut blocos = Vec::with_capacity(tokens.len());
    for t in tokens {
        if t[0] == 0 || recurso_do_patch_da_classe(t, 0x248, classe).is_null() {
            continue;
        }
        let handle = [((t[0] + 0x28) as *const u64).read_unaligned(), ((t[0] + 0x30) as *const u64).read_unaligned()];
        if handle[0] == 0 || handle[1] == 0 || contexto_rc == 0 {
            continue;
        }
        let bloco = alocar(0x30);
        if bloco.is_null() {
            continue;
        }
        if !crate::memoria::is_readable(bloco as *const c_void, 0x30) {
            liberar(bloco, 1);
            continue;
        }
        std::ptr::copy_nonoverlapping(params as *const u8, bloco, 0x30);
        let b = bloco as *mut [u64; 2];
        *b = handle;
        for rc in [handle[1], contexto_rc] {
            (*((rc + 4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
        }
        blocos.push(b);
    }
    blocos
}

pub(crate) const JOB_DO_BUILDER_VM: u64 = 0x1_019b_74b0;

pub(crate) const EXTRATOR_CRIAR_VM: u64 = 0x1_0107_63f0;

pub(crate) const EXTRATOR_EXTRAIR_VM: u64 = 0x1_0107_5dbc;

pub(crate) const EXTRATOR_TOMAR_RESULTADOS_VM: u64 = 0x1_0107_6170;

pub(crate) const EXTRATOR_DESTRUIR_VM: u64 = 0x1_0107_5714;

pub(crate) type JobDoBuilderFn = unsafe extern "C" fn(*mut [u64; 3], *const c_void);

/// `EntityTemplate`: cabeçalho do pacote compilado (`rootIndex` i16 no começo).
const TEMPLATE_PACOTE_OFF: u64 = 0x1d8;

/// `ObjectPackageExtractor`: bitset de objetos (ponteiro `+0x68`, palavras `+0x74`), resultados `+0xa8`, flags (5 B) `+0xb8`.
const EXTRATOR_BITSET_OFF: u64 = 0x68;
const EXTRATOR_RESULTADOS_OFF: u64 = 0xa8;
const EXTRATOR_FLAGS_OFF: u64 = 0xb8;

/// `EntityBuilder`: extrator da entidade `+0x10`, template `+0x90`, flags `+0x110`; `appearance` `+0x20`/`+0x30`/`+0x40`; `appearances` `+0x80`.
const BUILDER_EXTRATOR_OFF: u64 = 0x10;
const BUILDER_TEMPLATE_OFF: u64 = 0x90;
const BUILDER_FLAGS_OFF: u64 = 0x110;

unsafe fn flags_do_extrator(ext: u64) -> [u8; 5] {
    let mut f = [0u8; 5];
    std::ptr::copy_nonoverlapping((ext + EXTRATOR_FLAGS_OFF) as *const u8, f.as_mut_ptr(), 5);
    f
}

pub(crate) unsafe fn extrair_pacote_do_template(template: u64, flags: [u8; 5], bitset: Option<Vec<u32>>, saida: *mut [u64; 2]) -> Option<usize> {
    if template == 0 {
        return None;
    }
    extrair_pacote_do_cabecalho(template + TEMPLATE_PACOTE_OFF, flags, bitset, saida)
}

/// Controle da extração própria: o próprio template extraído com as flags e o bitset do motor dá o mesmo número de objetos, com a mesma
/// classe em cada posição e instâncias novas. Sem isso, mesclar no resultado do motor seria trocar objetos de um pacote lido de outro jeito.
pub(crate) unsafe fn conferir_extracao_propria(ext: u64, tpl: u64, flags: [u8; 5]) -> bool {
    let bits = (ext + EXTRATOR_BITSET_OFF) as *const u64;
    let palavras = ((ext + EXTRATOR_BITSET_OFF + 0x0c) as *const u32).read_unaligned() as usize;
    let dados = bits.read_unaligned() as *const u32;
    let bitset = ((1..=0x800).contains(&palavras) && !dados.is_null() && crate::memoria::is_readable(dados as *const c_void, palavras * 4))
        .then(|| std::slice::from_raw_parts(dados, palavras).to_vec());
    let motor = handles_do_dynarray((ext + EXTRATOR_RESULTADOS_OFF) as *const u8);
    let mut saida = [0u64; 2];
    let extraiu = extrair_pacote_do_template(tpl, flags, bitset, &mut saida).is_some();
    let proprio = handles_do_dynarray(saida.as_ptr() as *const u8);
    let classe = |h: &[u64; 2]| if h[0] == 0 { std::ptr::null_mut() } else { rtti::class_of(h[0] as *mut c_void) };
    let ok = extraiu
        && !motor.is_empty()
        && proprio.len() == motor.len()
        && motor.iter().zip(&proprio).all(|(m, p)| classe(m) == classe(p) && (p[0] == 0 || p[0] != m[0]));
    if saida[0] != 0 {
        soltar_dynarray_de_handles(saida.as_mut_ptr() as *mut u8);
    }
    ok
}

/// `Ok` mescla sem conferir; `EmCurso` (outro builder ainda conferindo) confere no próprio template em vez de recusar, para a entidade não sair
/// sem o patch até a próxima carga; `Pendente`/`Falhou` não mesclam.
pub(crate) fn mescla_no_builder_segue(controle: ControleDaExtracao, conferir_no_proprio: impl FnOnce() -> bool) -> bool {
    match controle {
        ControleDaExtracao::Ok => true,
        ControleDaExtracao::EmCurso => conferir_no_proprio(),
        ControleDaExtracao::Pendente | ControleDaExtracao::Falhou => false,
    }
}

/// Amostra de teste de uma mescla com substituídos (até 8 por lista).
unsafe fn amostrar_mescla(lista: &Mutex<Vec<crate::diagnostico::A51fAmostra>>, builder: u64, template: u64, resultados: u64, patch: u64, antes: usize, plano: &[PassoDaMescla]) {
    let substituidos = plano.iter().filter(|p| matches!(p, PassoDaMescla::Substitui(_))).count();
    if substituidos == 0 {
        return;
    }
    let Ok(mut g) = lista.try_lock() else { return };
    if g.len() >= 8 {
        return;
    }
    let (r, p) = (handles_do_dynarray(resultados as *const u8), handles_do_dynarray(patch as *const u8));
    g.push(crate::diagnostico::A51fAmostra {
        builder,
        template,
        antes,
        depois: r.len(),
        substituidos,
        anexados: plano.iter().filter(|p| **p == PassoDaMescla::Anexa).count(),
        pares: pares_substituidos(plano, &r, &p).into_iter().take(8).collect(),
    });
}

/// `components` e `entity` do `resource.patch` nos resultados do extrator da entidade, com o builder travado e antes da original do job.
/// Devolve o tamanho dos resultados antes da mescla.
pub(crate) unsafe fn mesclar_no_builder_travado(builder: u64) -> usize {
    if builder == 0 || !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return 0;
    }
    let ext = ((builder + BUILDER_EXTRATOR_OFF) as *const u64).read_unaligned();
    if ext == 0 || !crate::memoria::is_readable(ext as *const c_void, 0x120) {
        return 0;
    }
    let resultados = ext + EXTRATOR_RESULTADOS_OFF;
    let tam = ((resultados + 0x0c) as *const u32).read_unaligned() as usize;
    let tpl = ((builder + BUILDER_TEMPLATE_OFF) as *const u64).read_unaligned();
    if tpl == 0 || tam == 0 {
        return tam;
    }
    let flags = flags_do_extrator(ext);
    // O 1º builder com resultados decide o controle pelo processo inteiro.
    if CONTROLE_DA_EXTRACAO
        .compare_exchange(ControleDaExtracao::Pendente as u8, ControleDaExtracao::EmCurso as u8, Ordering::AcqRel, Ordering::Relaxed)
        .is_ok()
    {
        let ok = conferir_extracao_propria(ext, tpl, flags);
        CONTROLE_DA_EXTRACAO.store(if ok { ControleDaExtracao::Ok } else { ControleDaExtracao::Falhou } as u8, Ordering::Release);
        crate::veredito_dev!("[a51f] controle da extração própria no template {tpl:#x}: {tam} objeto(s); extracao-ok={ok}");
    }
    let caminho = crate::jogo::recursos::hash_do_caminho_do_recurso(tpl);
    let patches: Vec<([u64; 2], u32)> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g
            .do_alvo_ou_da_instancia(Some(caminho), tpl, |p| p.props & (A17_PROP_COMPONENTS | A17_PROP_ENTITY) != 0, |t| recurso_do_token(t))
            .map(|p| (p.token, p.props))
            .collect(),
        Err(_) => return tam,
    };
    if patches.is_empty() {
        return tam;
    }
    let controle = ControleDaExtracao::do_byte(CONTROLE_DA_EXTRACAO.load(Ordering::Acquire));
    if !mescla_no_builder_segue(controle, || conferir_extracao_propria(ext, tpl, flags)) {
        MESCLAS_NO_BUILDER_RECUSADAS.fetch_add(1, Ordering::Relaxed);
        return tam;
    }
    // `rootIndex` (i16) no começo do pacote; -1 = sem raiz, e a troca de raiz não acontece.
    let raiz = |t: u64| {
        let p = (t + TEMPLATE_PACOTE_OFF) as *const i16;
        if crate::memoria::is_readable(p as *const c_void, 2) { p.read_unaligned() } else { -1 }
    };
    let raiz_r = raiz(tpl);
    for (token, props) in patches {
        let ptpl = recurso_do_token(token);
        if ptpl == 0 {
            crate::diagnostico::A17_PATCHES_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut saida = [0u64; 2];
        if extrair_pacote_do_template(ptpl, flags, None, &mut saida).is_none() {
            continue;
        }
        let patch = saida.as_mut_ptr() as *mut u8;
        let antes = ((resultados + 0x0c) as *const u32).read_unaligned() as usize;
        if props & A17_PROP_COMPONENTS != 0 {
            if let Some(plano) = mesclar_componentes(resultados as *mut u8, patch, false) {
                amostrar_mescla(&crate::diagnostico::A51F_AMOSTRAS, builder, tpl, resultados, patch as u64, antes, &plano);
            }
        }
        if props & A17_PROP_ENTITY != 0 {
            mesclar_entidade(resultados as *mut u8, patch, raiz_r, raiz(ptpl));
        }
        soltar_dynarray_de_handles(patch);
        MESCLAS_NO_BUILDER_APLICADAS.fetch_add(1, Ordering::Relaxed);
    }
    tam
}

pub(crate) const APPDEF_COMPILEDDATA_OFF: u64 = 0xf0;

pub(crate) const APPDEF_CABECALHO_DO_PACOTE_OFF: u64 = 0x148;

pub(crate) const LEITOR_DO_PACOTE_CRIAR_VM: u64 = 0x1_019b_e2fc;

pub(crate) const LEITOR_DO_PACOTE_LER_VM: u64 = 0x1_019b_e360;

pub(crate) const LEITOR_DO_PACOTE_SOLTAR_VM: u64 = 0x1_019b_e348;

pub(crate) static MESCLA_DE_APARENCIA_ATIVA: AtomicBool = AtomicBool::new(false);

/// 1ª definição do patch, na ordem do patch, com o nome do alvo ou sem nome (sem nome vale para qualquer alvo).
pub(crate) fn definicao_do_patch(nomes: &[u64], alvo: u64) -> Option<usize> {
    nomes.iter().position(|&n| n == 0 || n == alvo)
}

pub(crate) static MESCLAS_DE_APARENCIA_APLICADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLAS_DE_APARENCIA_SEM_DEFINICAO: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLAS_DE_APARENCIA_SEM_DADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static CABECALHOS_DE_DEFINICAO_LIDOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static LEITURA_DE_DEFINICAO: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Objetos do pacote pelos ponteiros de início (`+0x20`) e fim (`+0x28`), 8 B cada.
pub(crate) fn objetos_do_cabecalho(cabecalho: &[u8; 0x70]) -> usize {
    let ler = |o: usize| u64::from_le_bytes([cabecalho[o], cabecalho[o + 1], cabecalho[o + 2], cabecalho[o + 3], cabecalho[o + 4], cabecalho[o + 5], cabecalho[o + 6], cabecalho[o + 7]]);
    let (inicio, fim) = (ler(0x20), ler(0x28));
    if inicio == 0 || fim < inicio { 0 } else { ((fim - inicio) / 8) as usize }
}

/// Cópia do cabeçalho do pacote da definição (`+0x148`); zerado com a definição ilegível.
pub(crate) unsafe fn cabecalho_da_definicao(def: u64) -> [u8; 0x70] {
    let mut c = [0u8; 0x70];
    let origem = def.wrapping_add(APPDEF_CABECALHO_DO_PACOTE_OFF);
    if def != 0 && crate::memoria::is_readable(origem as *const c_void, 0x70) {
        std::ptr::copy_nonoverlapping(origem as *const u8, c.as_mut_ptr(), 0x70);
    }
    c
}

/// `(dados, tamanho)` do `DataBuffer` do `compiledData` da definição; `None` sem dados carregados.
pub(crate) unsafe fn compileddata_da_definicao(def: u64) -> Option<(u64, u32)> {
    let buffer = def.wrapping_add(APPDEF_COMPILEDDATA_OFF + 0x20);
    if def == 0 || !crate::memoria::is_readable(buffer as *const c_void, 0x0c) {
        return None;
    }
    let dados = (buffer as *const u64).read_unaligned();
    let tamanho = ((buffer + 8) as *const u32).read_unaligned();
    (dados != 0 && tamanho != 0).then_some((dados, tamanho))
}

/// Garante o cabeçalho do pacote da definição: se ainda não tem objetos, lê o do `compiledData` carregado pelo leitor do motor e grava em
/// `+0x148`. Uma leitura por vez (a definição é do recurso compartilhado), conferindo de novo depois de travar.
pub(crate) unsafe fn abrir_definicao(def: u64) -> bool {
    if def == 0 || !crate::memoria::is_readable(def as *const c_void, 0x1c0) {
        return false;
    }
    if objetos_do_cabecalho(&cabecalho_da_definicao(def)) > 0 {
        return true;
    }
    let _trava = LEITURA_DE_DEFINICAO.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if objetos_do_cabecalho(&cabecalho_da_definicao(def)) > 0 {
        return true;
    }
    let Some((dados, tamanho)) = compileddata_da_definicao(def) else {
        MESCLAS_DE_APARENCIA_SEM_DADOS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    // Leitor local do motor (0x88 B; folga até 0x100).
    let mut leitor = [0u8; 0x100];
    let mut cabecalho = [0u8; 0x70];
    let criar: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(LEITOR_DO_PACOTE_CRIAR_VM));
    let buffer: unsafe extern "C" fn(*mut u8, u64, u64) = std::mem::transmute(crate::rebase(crate::jogo::recursos::LEITOR_BUFFER_VM));
    let soltar: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(LEITOR_DO_PACOTE_SOLTAR_VM));
    criar(leitor.as_mut_ptr());
    buffer(leitor.as_mut_ptr(), dados, u64::from(tamanho));
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(LEITOR_DO_PACOTE_LER_VM) as u64,
        in("x0") leitor.as_mut_ptr() as u64,
        in("x8") cabecalho.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    soltar(leitor.as_mut_ptr());
    if objetos_do_cabecalho(&cabecalho) == 0 {
        return false;
    }
    std::ptr::copy_nonoverlapping(cabecalho.as_ptr(), (def + APPDEF_CABECALHO_DO_PACOTE_OFF) as *mut u8, 0x70);
    CABECALHOS_DE_DEFINICAO_LIDOS.fetch_add(1, Ordering::Relaxed);
    true
}

/// `components` dos patches `.app` numa aparência do builder (definição `def`, recurso `.app` `res`, extrator `ext`): a definição do patch com o
/// nome da do alvo (ou sem nome) tem o pacote extraído com as flags do extrator da aparência e mesclado nos resultados dele. Com `identidade`
/// (teste) o próprio recurso entra como patch.
pub(crate) unsafe fn mesclar_patch_na_aparencia(builder: u64, tpl: u64, def: u64, res: u64, ext: u64, identidade: bool) {
    if def == 0 || res == 0 || ext == 0 || !crate::memoria::is_readable(ext as *const c_void, 0x120) || !crate::memoria::is_readable(def as *const c_void, 0x40) {
        return;
    }
    let caminho = crate::jogo::recursos::hash_do_caminho_do_recurso(res);
    let mut recursos: Vec<u64> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.apps().filter(|p| p.alvo == caminho && p.props & A17_PROP_COMPONENTS != 0).map(|p| recurso_do_token(p.token)).filter(|r| *r != 0).collect(),
        Err(_) => return,
    };
    if identidade {
        recursos.push(res);
    }
    let nome_do_alvo = ((def + APPDEF_NAME_OFF as u64) as *const u64).read_unaligned();
    let sem_nome = crate::diagnostico::A51F2_TESTE_NONE.load(Ordering::Relaxed);
    let flags = flags_do_extrator(ext);
    let resultados = ext + EXTRATOR_RESULTADOS_OFF;
    for recurso in recursos {
        let Some(definicoes) = definicoes_do_app(recurso as *const u8) else { continue };
        let nomes: Vec<u64> = definicoes
            .iter()
            .map(|h| if sem_nome || h[0] == 0 || !crate::memoria::is_readable((h[0] + APPDEF_NAME_OFF as u64) as *const c_void, 8) { 0 } else { ((h[0] + APPDEF_NAME_OFF as u64) as *const u64).read_unaligned() })
            .collect();
        let Some(i) = definicao_do_patch(&nomes, nome_do_alvo) else {
            MESCLAS_DE_APARENCIA_SEM_DEFINICAO.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let pdef = definicoes[i][0];
        if !abrir_definicao(pdef) {
            continue;
        }
        let mut saida = [0u64; 2];
        if extrair_pacote_do_cabecalho(pdef + APPDEF_CABECALHO_DO_PACOTE_OFF, flags, None, &mut saida).is_none() {
            continue;
        }
        let patch = saida.as_mut_ptr() as *mut u8;
        let antes = ((resultados + 0x0c) as *const u32).read_unaligned() as usize;
        if let Some(plano) = mesclar_componentes(resultados as *mut u8, patch, false) {
            MESCLAS_DE_APARENCIA_APLICADAS.fetch_add(1, Ordering::Relaxed);
            amostrar_mescla(&crate::diagnostico::A51F2_AMOSTRAS, builder, tpl, resultados, patch as u64, antes, &plano);
        }
        soltar_dynarray_de_handles(patch);
        if identidade {
            if let Ok(mut g) = crate::diagnostico::A51F2_RECURSOS.try_lock() {
                if g.len() < 64 {
                    g.push([tpl, res, nome_do_alvo]);
                }
            }
        }
    }
}

/// Flags do `EntityBuilder` em `+0x110` (u8): `ExtractAppearance` (bit 2) e `ExtractAppearances` (bit 3).
const BUILDER_EXTRAI_APARENCIA: u8 = 1 << 2;
const BUILDER_EXTRAI_APARENCIAS: u8 = 1 << 3;

/// Item de `appearances` do builder (0x30 B): definição `+0x00`, recurso `+0x10`, extrator `+0x20`.
const BUILDER_APARENCIAS_OFF: u64 = 0x80;
const BUILDER_APARENCIA_PASSO: u64 = 0x30;

/// As aparências que o builder extrai (`appearance` com o bit 2, cada uma de `appearances` com o bit 3) como `(definição, recurso, extrator)`.
unsafe fn aparencias_do_builder(builder: u64) -> (Option<[u64; 3]>, Vec<[u64; 3]>) {
    let flags = ((builder + BUILDER_FLAGS_OFF) as *const u8).read();
    let ler = |p: u64| (p as *const u64).read_unaligned();
    let unica = (flags & BUILDER_EXTRAI_APARENCIA != 0).then(|| [ler(builder + 0x20), ler(builder + 0x30), ler(builder + 0x40)]);
    let mut lista = Vec::new();
    if flags & BUILDER_EXTRAI_APARENCIAS != 0 {
        let dados = ler(builder + BUILDER_APARENCIAS_OFF);
        let n = ((builder + BUILDER_APARENCIAS_OFF + 0x0c) as *const u32).read_unaligned() as u64;
        if dados != 0 && (1..=64).contains(&n) && crate::memoria::is_readable(dados as *const c_void, (n * BUILDER_APARENCIA_PASSO) as usize) {
            lista = (0..n).map(|i| dados + i * BUILDER_APARENCIA_PASSO).map(|e| [ler(e), ler(e + 0x10), ler(e + 0x20)]).collect();
        }
    }
    (unica, lista)
}

pub(crate) unsafe fn mesclar_patches_nas_aparencias(builder: u64) {
    if !MESCLA_DE_APARENCIA_ATIVA.load(Ordering::Relaxed) || builder == 0 || !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return;
    }
    let tpl = ((builder + BUILDER_TEMPLATE_OFF) as *const u64).read_unaligned();
    let identidade = crate::diagnostico::A51F2_TESTE.load(Ordering::Relaxed)
        && crate::diagnostico::a51f_teste_hashes().contains(&crate::jogo::recursos::hash_do_caminho_do_recurso(tpl));
    let (unica, lista) = aparencias_do_builder(builder);
    for [def, res, ext] in unica.into_iter().chain(lista) {
        mesclar_patch_na_aparencia(builder, tpl, def, res, ext, identidade);
    }
}

pub(crate) static INCLUSAO_DE_PARTES_ATIVA: AtomicBool = AtomicBool::new(false);

/// Hash do `CName` `AppearanceParts` (FNV-1a 64 do texto, como o construtor de `CName`).
pub(crate) fn tag_appearance_parts() -> u64 {
    static H: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *H.get_or_init(|| bwms_primitives::fnv1a64(b"AppearanceParts"))
}

/// A definição inclui as partes dela se tem parte e a tag (na definição, ou forçada pelo schema de tags do template).
pub(crate) fn definicao_inclui_partes(partes: usize, tags_definicao: &[u64], forcar: bool, tag: u64) -> bool {
    partes > 0 && (forcar || tags_definicao.contains(&tag))
}

/// Instâncias dos objetos anexados, lidas do patch ANTES da mescla (depois dela a vaga do anexado fica nula).
pub(crate) fn instancias_anexadas(plano: &[PassoDaMescla], patch: &[[u64; 2]]) -> Vec<u64> {
    plano.iter().zip(patch).filter(|(p, _)| **p == PassoDaMescla::Anexa).map(|(_, h)| h[0]).collect()
}

/// Valores de 8 B de um `DynArray` (`CName` de `TagList`, caminho de `partsValues`); vazio se ilegível ou com mais de 256.
pub(crate) unsafe fn valores_u64_do_dynarray(arr: *const u8) -> Vec<u64> {
    if arr.is_null() || !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return Vec::new();
    }
    let dados = (arr as *const u64).read_unaligned() as *const u64;
    let n = (arr.add(0x0c) as *const u32).read_unaligned() as usize;
    if dados.is_null() || n == 0 || n > 256 || !crate::memoria::is_readable(dados as *const c_void, n * 8) {
        return Vec::new();
    }
    (0..n).map(|i| dados.add(i).read_unaligned()).collect()
}

/// Teto de tokens de parte retidos: cada um segura o template carregado.
const TOKENS_DE_PARTE_MAX: usize = 4096;

/// Token de carga da parte pelo caminho, pedido UMA vez e retido (o template fica carregado para as próximas montagens). Pedido que falha
/// não é guardado: a próxima chamada pede de novo.
pub(crate) unsafe fn token_da_parte(caminho: u64) -> [u64; 2] {
    let Ok(mut g) = TOKENS_DE_PARTE.lock() else { return [0, 0] };
    if let Some((_, t)) = g.iter().find(|(c, _)| *c == caminho) {
        return *t;
    }
    let Some(t) = crate::jogo::recursos::token_do_path(caminho).filter(|t| t[0] != 0) else { return [0, 0] };
    if g.len() < TOKENS_DE_PARTE_MAX {
        g.push((caminho, t));
    }
    t
}

/// Tamanho legível do `EntityTemplate` que a inclusão de partes lê.
const TEMPLATE_TAMANHO_LIDO: usize = 0x248;

/// Partes da definição (`partsValues`) entram nos resultados do extrator da aparência como mescla de PARTE (componente externo novo fica de
/// fora). Parte sem template carregado é pulada: a montagem segue sem ela.
pub(crate) unsafe fn incluir_partes_na_aparencia(builder: u64, tpl: u64, def: u64, res: u64, ext: u64, forcar: bool, teste: bool) {
    let _ = res;
    if def == 0 || ext == 0 || !crate::memoria::is_readable(ext as *const c_void, 0x120) || !crate::memoria::is_readable(def as *const c_void, 0x98) {
        return;
    }
    let (partes, forcar) = if teste {
        (vec![bwms_primitives::resource_path_hash(crate::diagnostico::A51B_TESTE_ALVO)], true)
    } else {
        (valores_u64_do_dynarray((def + APPDEF_PARTSVALUES_OFF as u64) as *const u8), forcar)
    };
    let tags = valores_u64_do_dynarray((def + APPDEF_VISUALTAGS_OFF as u64) as *const u8);
    let tag = tag_appearance_parts();
    crate::diagnostico::A51F3_DEFINICOES.fetch_add(1, Ordering::Relaxed);
    if tags.contains(&tag) {
        crate::diagnostico::A51F3_COM_TAG.fetch_add(1, Ordering::Relaxed);
    }
    if !definicao_inclui_partes(partes.len(), &tags, forcar, tag) {
        return;
    }
    crate::diagnostico::A51F3_INCLUSOES.fetch_add(1, Ordering::Relaxed);
    let flags = flags_do_extrator(ext);
    let resultados = ext + EXTRATOR_RESULTADOS_OFF;
    let classe = CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed);
    for parte in partes {
        crate::diagnostico::A51F3_PARTES.fetch_add(1, Ordering::Relaxed);
        let modelo = recurso_do_patch_da_classe(token_da_parte(parte), TEMPLATE_TAMANHO_LIDO, classe) as u64;
        if modelo == 0 {
            crate::diagnostico::A51F3_NAO_PRONTAS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut saida = [0u64; 2];
        if extrair_pacote_do_template(modelo, flags, None, &mut saida).is_none() {
            crate::diagnostico::A51F3_SEM_PACOTE.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let patch = saida.as_mut_ptr() as *mut u8;
        let antes = ((resultados + 0x0c) as *const u32).read_unaligned() as usize;
        if let Some(plano) = mesclar_componentes(resultados as *mut u8, patch, true) {
            amostrar_mescla(&crate::diagnostico::A51F3_AMOSTRAS, builder, tpl, resultados, patch as u64, antes, &plano);
        }
        soltar_dynarray_de_handles(patch);
    }
}

/// Inclusão de partes nas aparências que o builder extrai. O schema de tags do template com `AppearanceParts` força a inclusão em toda
/// definição com parte.
pub(crate) unsafe fn incluir_partes_no_builder(builder: u64) {
    if !INCLUSAO_DE_PARTES_ATIVA.load(Ordering::Relaxed) || builder == 0 || !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return;
    }
    let tpl = ((builder + BUILDER_TEMPLATE_OFF) as *const u64).read_unaligned();
    if tpl == 0 || !crate::memoria::is_readable(tpl as *const c_void, TEMPLATE_TAMANHO_LIDO) {
        return;
    }
    let tag = tag_appearance_parts();
    let schema = ((tpl + TEMPLATE_VISUALTAGSSCHEMA_OFF as u64) as *const u64).read_unaligned();
    let forcar = schema != 0 && valores_u64_do_dynarray((schema + SCHEMA_VISUALTAGS_OFF as u64) as *const u8).contains(&tag);
    if forcar {
        crate::diagnostico::A51F3_SCHEMAS_COM_TAG.fetch_add(1, Ordering::Relaxed);
    }
    let teste = crate::diagnostico::A51F3_TESTE.load(Ordering::Relaxed)
        && crate::diagnostico::a51f_teste_hashes().contains(&crate::jogo::recursos::hash_do_caminho_do_recurso(tpl));
    let (unica, lista) = aparencias_do_builder(builder);
    if let Some([def, res, ext]) = unica {
        incluir_partes_na_aparencia(builder, tpl, def, res, ext, forcar, teste);
        // No teste a parte de teste entra uma vez: com a `appearance` feita, as `appearances` ficam de fora.
        if teste {
            return;
        }
    }
    for [def, res, ext] in lista {
        incluir_partes_na_aparencia(builder, tpl, def, res, ext, forcar, teste);
    }
}

pub(crate) static CMESH_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static FIXES_DE_NOME_DO_MESH: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<(u64, u64)>>>> = std::sync::RwLock::new(None);

/// Confere pelo RTTI os 4 offsets que a troca de nomes de material escreve; só com os 4 iguais aos medidos o fix de nomes liga.
pub(crate) unsafe fn guardar_offsets_do_cmesh(reg: &crate::rtti::Registry) -> bool {
    let campo = |classe: &str, nome: &str| prop_value_offset(reg.class_by_name(classe), nome);
    let ok = campo("CMesh", "appearances") == Some(CMESH_APPEARANCES_OFF)
        && campo("CMesh", "materialEntries") == Some(CMESH_MATERIALENTRIES_OFF)
        && campo("meshMeshAppearance", "chunkMaterials") == Some(MESHAPPEARANCE_CHUNKMATERIALS_OFF)
        && campo("CMeshMaterialEntry", "name") == Some(0x20);
    CMESH_OFFSETS_OK.store(ok, Ordering::Relaxed);
    ok
}

/// Tabela dos `names` do `resource.fix` (hash do mesh → pares (antigo, novo)). Sem linha nem marcador de teste: a recarga (`substituir`) apaga a
/// anterior e o gancho do `PostLoad`, se instalado, fica. Com linha: só com os offsets do `CMesh` conferidos a tabela entra e o gancho instala.
/// Devolve os pares carregados.
pub(crate) unsafe fn carregar_fixes_de_nome_do_mesh(nomes: std::collections::HashMap<u64, Vec<(u64, u64)>>, substituir: bool) -> usize {
    if nomes.is_empty() && !marcador_individual("a51i-teste") {
        if substituir {
            if let Ok(mut g) = FIXES_DE_NOME_DO_MESH.write() {
                *g = None;
            }
        }
        return 0;
    }
    let Some(reg) = crate::rtti::Registry::obtain() else { return 0 };
    if !guardar_offsets_do_cmesh(&reg) {
        crate::veredito_dev!("[a51i] offsets do CMesh divergem do medido; fix de nomes desligado");
        return 0;
    }
    let n = nomes.values().map(Vec::len).sum();
    if let Ok(mut g) = FIXES_DE_NOME_DO_MESH.write() {
        *g = Some(nomes);
    }
    install_cmesh_postload_hook();
    n
}

/// `context` do `resource.fix` (S12): caminho do mesh → atributos `(CName do nome em snake_case, valor)`, entregues ao estado de mesh dinâmico
/// NATIVO da 2.5 (`z040/relatorios/S12a.md`): o jogo faz o mesh dinâmico sozinho, mas nada chama o `PrepareMeshState` dele com contexto.
pub(crate) static CONTEXTOS_DO_MESH: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<(u64, String)>>>> = std::sync::RwLock::new(None);

/// Preparo do estado de mesh dinâmico nativo da 2.5 (`mesh, &contexto`, contexto de 0x28 B, S12a): cria o estado, zera o `contextToken` (o
/// `@context` do mesh fica de fora: o contexto do fix já é o dado), insere os atributos sem sobrescrever e resolve o `appearance_expansion_source`.
const PREPARE_MESH_STATE_VM: u64 = 0x1_01ce_476c;
/// Os 16 bytes do prólogo do `PrepareMeshState` no Steam 2.5 (S12a §5.2): outro build não recebe a chamada.
const PREPARE_MESH_STATE_PROLOGO: [u32; 4] = [0xd101_83ff, 0xa902_5ff8, 0xa903_57f6, 0xa904_4ff4];

/// Carga do `context` de `bwms-resfixes.txt` (game thread; `mods_do_usuario::pipeline::resfixes_contexto`): registra cada nome no pool de CName
/// aqui, fora do job do `PostLoad`, e instala o hook do `CMesh::PostLoad` também quando só há contexto. Devolve os atributos carregados.
pub(crate) unsafe fn carregar_contextos_do_mesh(contextos: std::collections::HashMap<u64, Vec<(String, String)>>, substituir: bool) -> usize {
    if contextos.is_empty() {
        if substituir {
            if let Ok(mut g) = CONTEXTOS_DO_MESH.write() {
                *g = None;
            }
        }
        return 0;
    }
    let p = crate::rebase(PREPARE_MESH_STATE_VM) as *const u32;
    let prologo_ok = crate::memoria::is_readable(p as *const c_void, 16) && (0..4).all(|i| p.add(i).read_unaligned() == PREPARE_MESH_STATE_PROLOGO[i]);
    if !prologo_ok {
        crate::veredito_dev!("[s12] contexto do resource.fix: PrepareMeshState com outro prólogo neste build; nada injetado");
        return 0;
    }
    let mapa: std::collections::HashMap<u64, Vec<(u64, String)>> = contextos
        .into_iter()
        .map(|(mesh, pares)| (mesh, pares.into_iter().map(|(nome, valor)| (crate::cname::registrar_no_pool(&nome), valor)).collect()))
        .collect();
    let n = mapa.values().map(Vec::len).sum();
    if let Ok(mut g) = CONTEXTOS_DO_MESH.write() {
        *g = Some(mapa);
    }
    install_cmesh_postload_hook();
    crate::veredito_dev!(
        "[s12] contexto do resource.fix: {n} atributo(s); hook do CMesh::PostLoad {}",
        if crate::arquivos_do_jogo::ORIG_CMESH_POSTLOAD.load(Ordering::Relaxed) == 0 { "FALHOU" } else { "instalado" }
    );
    n
}

/// `std::string` da libc++ (24 B) que aponta `texto` sem copiar: curta (até 22 B) no lugar, longa `{ptr, tamanho, capacidade | 1<<63}` (S12a §2).
/// A longa só vale enquanto `texto` vive; o `PrefillContext` copia.
fn string_da_libcxx(texto: &std::ffi::CString) -> [u8; 24] {
    let b = texto.as_bytes();
    let mut s = [0u8; 24];
    if b.len() <= 22 {
        s[..b.len()].copy_from_slice(b);
        s[23] = b.len() as u8;
    } else {
        s[..8].copy_from_slice(&(texto.as_ptr() as u64).to_le_bytes());
        s[8..16].copy_from_slice(&(b.len() as u64).to_le_bytes());
        s[16..24].copy_from_slice(&((b.len() as u64 + 1) | 1 << 63).to_le_bytes());
    }
    s
}

/// No `PostLoad` do mesh (job, antes da original): o `context` do fix deste mesh no estado nativo, pelo `PrepareMeshState` do jogo. O contexto
/// vai no formato que o `PrefillContext` lê (S12a §3.2): chaves `CName[N]` em `+0`, N em `+0xc`, valores `std::string[N]` em `+0x10` e as flags
/// em `+0x20` zeradas (com o bit 0, o jogo ordenaria os arrays no lugar).
pub(crate) unsafe fn injetar_contexto_do_mesh(mesh: *mut c_void) {
    if mesh.is_null() {
        return;
    }
    let caminho = (mesh.cast::<u8>().add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let Ok(g) = CONTEXTOS_DO_MESH.try_read() else { return };
    let Some(pares) = g.as_ref().and_then(|m| m.get(&caminho)) else { return };
    let textos: Vec<std::ffi::CString> = pares.iter().map(|(_, v)| std::ffi::CString::new(v.as_str()).unwrap_or_default()).collect();
    let chaves: Vec<u64> = pares.iter().map(|(k, _)| *k).collect();
    let valores: Vec<[u8; 24]> = textos.iter().map(string_da_libcxx).collect();
    let n = chaves.len() as u32;
    let mut contexto = [0u8; 0x28];
    contexto[0x00..0x08].copy_from_slice(&(chaves.as_ptr() as u64).to_le_bytes());
    contexto[0x08..0x0c].copy_from_slice(&n.to_le_bytes());
    contexto[0x0c..0x10].copy_from_slice(&n.to_le_bytes());
    contexto[0x10..0x18].copy_from_slice(&(valores.as_ptr() as u64).to_le_bytes());
    contexto[0x18..0x1c].copy_from_slice(&n.to_le_bytes());
    contexto[0x1c..0x20].copy_from_slice(&n.to_le_bytes());
    let f: unsafe extern "C" fn(*mut c_void, *const u8) = std::mem::transmute(crate::rebase(PREPARE_MESH_STATE_VM));
    f(mesh, contexto.as_ptr());
    #[cfg(feature = "devlog")]
    {
        static LOGADOS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if LOGADOS.fetch_add(1, Ordering::Relaxed) < 8 {
            crate::log_dev!("[s12] contexto injetado no mesh {caminho:#x}: {n} atributo(s)");
        }
    }
}

/// `meshctx <caminho do mesh> <atributo> <valor esperado> <fonte esperada>` (dev, S12c, só leitura): o estado nativo do mesh no `s_states` da 2.5
/// (S12a §1–2), sob a trava de leitura dele: `contextToken` (`+0x20`, 0 com o contexto do fix), o atributo em `contextAttrs` (`std::map` da
/// libc++ em `+0x40`, nó com `CName` em `+0x20` e `std::string` em `+0x28`) e o `expansionSource` (`+0x58`).
/// `caminho` pode ser `a.mesh,b.mesh` (o corpo do V muda com o save): passa se o estado de um deles está certo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn conferir_contexto_do_mesh(caminhos: &str, atributo: &str, esperado: &str, fonte: &str) -> String {
    let vereditos: Vec<String> = caminhos.split(',').map(|c| conferir_contexto_de_um_mesh(c, atributo, esperado, fonte)).collect();
    vereditos.iter().find(|v| v.ends_with("record-ok=true")).or_else(|| vereditos.iter().find(|v| !v.contains("sem estado"))).unwrap_or(&vereditos[0]).clone()
}

#[cfg(feature = "devtools")]
unsafe fn conferir_contexto_de_um_mesh(caminho: &str, atributo: &str, esperado: &str, fonte: &str) -> String {
    const S_STATES_VM: u64 = 0x1_07da_a650;
    const S_STATELOCK_VM: u64 = 0x1_07da_a6e8;
    let alvo = bwms_primitives::resource_path_hash(caminho);
    let trava = crate::rebase(S_STATELOCK_VM) as *mut u8;
    crate::jogo::travas::travar_leitura(trava);
    let leitura = (|| {
        let no = crate::jogo::mapas::nos(crate::rebase(S_STATES_VM) as *const u8)?
            .into_iter()
            .find(|n| (n.add(0x08) as *const u64).read_unaligned() == alvo)?;
        let estado = (no.add(0x10) as *const u64).read_unaligned() as *const u8;
        if !crate::memoria::is_readable(estado as *const c_void, 0x60) {
            return None;
        }
        let token = (estado.add(0x20) as *const u64).read_unaligned();
        let n = (estado.add(0x50) as *const u64).read_unaligned();
        let fonte_lida = (estado.add(0x58) as *const u64).read_unaligned();
        // Em ordem pelo `std::map` (raiz em `+0x48`; nó: esquerda `+0`, direita `+8`), com teto pelo tamanho.
        let mut pilha: Vec<*const u8> = Vec::new();
        let mut atual = (estado.add(0x48) as *const u64).read_unaligned() as *const u8;
        let mut achado = None;
        let mut visitados = 0u64;
        while (!atual.is_null() || !pilha.is_empty()) && visitados <= n {
            while !atual.is_null() && crate::memoria::is_readable(atual as *const c_void, 0x40) && pilha.len() < 64 {
                pilha.push(atual);
                atual = (atual as *const u64).read_unaligned() as *const u8;
            }
            let Some(nodo) = pilha.pop() else { break };
            visitados += 1;
            if (nodo.add(0x20) as *const u64).read_unaligned() == crate::cname::cname(atributo) {
                let s = nodo.add(0x28);
                achado = Some(if s.add(0x17).read() & 0x80 == 0 {
                    String::from_utf8_lossy(std::slice::from_raw_parts(s, s.add(0x17).read() as usize)).into_owned()
                } else {
                    let (p, l) = ((s as *const u64).read_unaligned() as *const u8, (s.add(8) as *const u64).read_unaligned() as usize);
                    String::from_utf8_lossy(std::slice::from_raw_parts(p, l.min(4096))).into_owned()
                });
            }
            atual = (nodo.add(0x08) as *const u64).read_unaligned() as *const u8;
        }
        Some((token, n, achado, fonte_lida))
    })();
    crate::jogo::travas::soltar_leitura(trava);
    match leitura {
        None => format!("[recconf] VEREDITO meshctx {caminho}: sem estado nativo do mesh; record-ok=false"),
        Some((token, n, achado, fonte_lida)) => {
            let ok = token == 0 && achado.as_deref() == Some(esperado) && fonte_lida == crate::cname::cname(fonte);
            format!(
                "[recconf] VEREDITO meshctx {caminho}: contextToken {token:#x}, {n} atributo(s), {atributo}={achado:?} (esperado '{esperado}'), \
                 expansionSource {fonte_lida:#x} (esperado '{fonte}' = {:#x}); record-ok={ok}",
                crate::cname::cname(fonte)
            )
        }
    }
}

pub(crate) static CLASSE_MESHMESHAPPEARANCE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Tag `ResourcePatch` das aparências clonadas.
pub(crate) fn tag_resource_patch() -> u64 {
    static H: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *H.get_or_init(|| bwms_primitives::fnv1a64(b"ResourcePatch"))
}

/// `meshMeshAppearance`: `name` `+0x30`, `tags` `+0x58` (`DynArray<CName>`).
const MESHAPPEARANCE_NOME_OFF: usize = 0x30;
const MESHAPPEARANCE_TAGS_OFF: usize = 0x58;

/// Aparência nova (handle forte) com o nome e os `chunkMaterials` da aparência do patch e exatamente as tags `ResourcePatch`, `expansao`,
/// `origem`, nessa ordem: as tags marcam de onde ela veio para a expansão do mesh achar os materiais do patch. Falha solta o que criou.
pub(crate) unsafe fn clonar_aparencia_do_mesh(pinst: *const u8, expansao: u64, origem: u64) -> Option<[u64; 2]> {
    let classe = CLASSE_MESHMESHAPPEARANCE.load(Ordering::Relaxed);
    if pinst.is_null() || classe.is_null() || !crate::memoria::is_readable(pinst as *const c_void, MESHAPPEARANCE_TAGS_OFF + 0x10) {
        return None;
    }
    let nova = rtti::create_instance(classe);
    if nova.is_null() {
        return None;
    }
    let mut h = [0u64; 2];
    rtti::make_handle(h.as_mut_ptr() as *mut c_void, nova);
    if h[0] == 0 {
        return None;
    }
    let n = nova as *mut u8;
    (n.add(MESHAPPEARANCE_NOME_OFF) as *mut u64).write_unaligned((pinst.add(MESHAPPEARANCE_NOME_OFF) as *const u64).read_unaligned());
    if !copiar_dynarray(n.add(MESHAPPEARANCE_CHUNKMATERIALS_OFF), pinst.add(MESHAPPEARANCE_CHUNKMATERIALS_OFF), 8, 8) {
        crate::jogo::objetos::soltar_handle(&mut h);
        return None;
    }
    let tags = n.add(MESHAPPEARANCE_TAGS_OFF);
    crate::jogo::listas::reservar(tags as *mut c_void, 3, 8, 8, std::ptr::null());
    let (buffer, _, capacidade) = crate::jogo::listas::dynarray_no_campo(tags, 0);
    if buffer.is_null() || capacidade < 3 {
        crate::jogo::objetos::soltar_handle(&mut h);
        return None;
    }
    for (i, t) in [tag_resource_patch(), expansao, origem].into_iter().enumerate() {
        (buffer.add(i * 8) as *mut u64).write_unaligned(t);
    }
    (tags.add(0x0c) as *mut u32).write_unaligned(3);
    Some(h)
}

pub(crate) static MESCLA_DE_RENDER_BLOB_ATIVA: AtomicBool = AtomicBool::new(false);

/// Blob do patch entra com a prop escrita (mesmo com o alvo tendo blob) ou, com todas as props (`-`), só no alvo sem blob.
fn blob_muda(props: u32, bit: u32, patch_tem: bool, alvo_tem: bool) -> bool {
    patch_tem && if props == u32::MAX { !alvo_tem } else { props & bit != 0 }
}

pub(crate) fn render_blob_muda(props: u32, patch_tem: bool, alvo_tem: bool) -> bool {
    blob_muda(props, A17_PROP_RENDERBLOB, patch_tem, alvo_tem)
}

pub(crate) static MORPH_BLOB_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) fn morph_blob_muda(props: u32, patch_tem: bool, alvo_tem: bool) -> bool {
    blob_muda(props, A17_PROP_BLOB, patch_tem, alvo_tem)
}

pub(crate) const MARCADOR_DE_TEMPLATE: char = '@';

pub(crate) fn material_de_template(nome: &str) -> bool {
    nome.starts_with(MARCADOR_DE_TEMPLATE)
}

pub(crate) fn material_dinamico(nome: &str) -> bool {
    nome.contains(MARCADOR_DE_TEMPLATE)
}

/// Mesh pede o estado dinâmico se não tem material, tem material de template, não tem aparência, tem aparência sem chunk ou algum chunk usa
/// `@`. Material com `@` no meio (não template) sozinho não torna o mesh dinâmico.
pub(crate) fn mesh_dinamico<M, C>(materiais: &[M], aparencias: &[Vec<C>], template: impl Fn(&M) -> bool, dinamico: impl Fn(&C) -> bool) -> bool {
    materiais.is_empty()
        || materiais.iter().any(template)
        || aparencias.is_empty()
        || aparencias.iter().any(|chunks| chunks.is_empty() || chunks.iter().any(&dinamico))
}

/// `CMesh`: caminho, `surfaceAreaPerAxis.X`, `forceLoadAllAppearances`, `materialEntries`, `appearances`, `renderResourceBlob`, `MaterialLock`.
pub(crate) const CMESH_CAMINHO_OFF: usize = 0x30;

pub(crate) const CMESH_MATERIALENTRIES_OFF: usize = 0xe0;

pub(crate) const CMESH_APPEARANCES_OFF: usize = 0x1e0;

pub(crate) const CMESH_MATERIALLOCK_OFF: usize = 0x218;

/// `CMeshMaterialEntry` (0x30 B): `material` 0x00, `materialWeak` 0x10, `name` 0x20, `isLocalInstance` 0x2a.
pub(crate) const CMESHMATERIALENTRY_TAMANHO: usize = 0x30;

pub(crate) const MESHAPPEARANCE_CHUNKMATERIALS_OFF: usize = 0x38;

pub(crate) static MESH_DINAMICO_ATIVO: AtomicBool = AtomicBool::new(false);

/// `s_dummyMaterial`: o material das entradas com o nome da tag de origem (48d-1).
pub(crate) static MATERIAL_VAZIO: AtomicPtr<[u64; 2]> = AtomicPtr::new(std::ptr::null_mut());

/// Bits do [`bits_de_arroba`]: o nome tem `@` ([`material_dinamico`]) e começa com `@` ([`material_de_template`]).
const BIT_MATERIAL_DINAMICO: u8 = 1 << 0;
const BIT_MATERIAL_DE_TEMPLATE: u8 = 1 << 1;

/// Teto do cache de `bits_de_arroba`: cada nome de material visto fica nele.
const BITS_DE_ARROBA_MAX: usize = 1 << 20;

/// [`BIT_MATERIAL_DINAMICO`] e [`BIT_MATERIAL_DE_TEMPLATE`] do texto do CName, guardados por hash: o mesmo nome é consultado em toda carga de
/// mesh. Hash 0 ou sem texto dá 0, sem guardar (o texto pode aparecer depois no pool).
pub(crate) unsafe fn bits_de_arroba(hash: u64) -> u8 {
    static CACHE: std::sync::OnceLock<Mutex<std::collections::HashMap<u64, u8>>> = std::sync::OnceLock::new();
    if hash == 0 {
        return 0;
    }
    let cache = CACHE.get_or_init(Default::default);
    if let Some(b) = cache.lock().ok().and_then(|m| m.get(&hash).copied()) {
        return b;
    }
    let Some(texto) = crate::cname::name_of(hash) else { return 0 };
    let bits = if material_dinamico(&texto) { BIT_MATERIAL_DINAMICO } else { 0 } | if material_de_template(&texto) { BIT_MATERIAL_DE_TEMPLATE } else { 0 };
    if let Ok(mut m) = cache.lock() {
        if m.len() < BITS_DE_ARROBA_MAX {
            m.insert(hash, bits);
        }
    }
    bits
}

/// Teto de itens nas leituras de listas do mesh.
const LISTA_DO_MESH_MAX: usize = 65536;

/// Os `CName` de um `DynArray<CName>` em `obj + off`; vazio com buffer nulo, ilegível ou acima do teto.
pub(crate) unsafe fn cnames_do_dynarray(obj: *const u8, off: usize) -> Vec<u64> {
    let (b, n, _) = crate::jogo::listas::dynarray_no_campo(obj, off);
    if b.is_null() || n > LISTA_DO_MESH_MAX || !crate::memoria::is_readable(b as *const c_void, n * 8) {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * 8) as *const u64).read_unaligned()).collect()
}

/// Instâncias de `CMesh.appearances` (nula na entrada vazia).
pub(crate) unsafe fn aparencias_do_mesh(mesh: *const u8) -> Vec<*mut u8> {
    let (b, n, _) = crate::jogo::listas::dynarray_no_campo(mesh, CMESH_APPEARANCES_OFF);
    if b.is_null() || n > LISTA_DO_MESH_MAX || !crate::memoria::is_readable(b as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * 16) as *const u64).read_unaligned() as *mut u8).collect()
}

/// `name` de cada `CMesh.materialEntries`.
pub(crate) unsafe fn nomes_dos_materiais_do_mesh(mesh: *const u8) -> Vec<u64> {
    let (b, n, _) = crate::jogo::listas::dynarray_no_campo(mesh, CMESH_MATERIALENTRIES_OFF);
    if b.is_null() || n > LISTA_DO_MESH_MAX || !crate::memoria::is_readable(b as *const c_void, n * CMESHMATERIALENTRY_TAMANHO) {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * CMESHMATERIALENTRY_TAMANHO + 0x20) as *const u64).read_unaligned()).collect()
}

/// Estado de mesh com materiais de template e comuns por nome, origens de patch e contexto.
#[derive(Default)]
pub(crate) struct EstadoDoMesh {
    pub(crate) templates: std::collections::HashMap<u64, usize>,
    pub(crate) materiais: std::collections::HashMap<u64, usize>,
    /// Tag de origem (hash do caminho do patch) → `WeakHandle<CMesh>` (com 1 fraco nosso).
    pub(crate) origens: std::collections::HashMap<u64, [u64; 2]>,
    /// `appearance_expansion_source` do contexto; 0 sem contexto.
    pub(crate) fonte_expansao: u64,
    pub(crate) contexto_lido: bool,
    /// Nome do parâmetro `CName` em snake_case → valor.
    pub(crate) atributos: std::collections::HashMap<u64, u64>,
    pub(crate) parametros_contexto: Vec<u64>,
    /// Instância do contexto (fraca, com 1 fraco nosso).
    pub(crate) contexto: [u64; 2],
}

impl EstadoDoMesh {
    /// Refaz os materiais comuns (nome → índice); os de template (`@` no começo) acumulam em `templates` entre chamadas.
    pub(crate) unsafe fn preencher_materiais(&mut self, mesh: *const u8) {
        self.materiais.clear();
        for (i, nome) in nomes_dos_materiais_do_mesh(mesh).into_iter().enumerate() {
            if bits_de_arroba(nome) & BIT_MATERIAL_DE_TEMPLATE != 0 {
                self.templates.insert(nome, i);
            } else {
                self.materiais.insert(nome, i);
            }
        }
    }
}

pub(crate) fn estado_do_mesh(caminho: u64) -> Option<std::sync::Arc<std::sync::Mutex<EstadoDoMesh>>> {
    ESTADOS_DOS_MESHES.read().ok()?.as_ref()?.get(&caminho).cloned()
}

/// Cria o estado do mesh dinâmico na carga (só leitura de memória); o 1º estado guardado para o caminho fica.
pub(crate) unsafe fn preparar_estado_do_mesh(mesh: *mut c_void) {
    if !MESH_DINAMICO_ATIVO.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let m = mesh as *const u8;
    let caminho = (m.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    if caminho == 0 || estado_do_mesh(caminho).is_some() {
        return;
    }
    let materiais = nomes_dos_materiais_do_mesh(m);
    let aparencias: Vec<Vec<u64>> =
        aparencias_do_mesh(m).into_iter().filter(|a| !a.is_null()).map(|a| cnames_do_dynarray(a, MESHAPPEARANCE_CHUNKMATERIALS_OFF)).collect();
    let dinamico = mesh_dinamico(
        &materiais,
        &aparencias,
        |n| bits_de_arroba(*n) & BIT_MATERIAL_DE_TEMPLATE != 0,
        |c| bits_de_arroba(*c) & BIT_MATERIAL_DINAMICO != 0,
    );
    if !dinamico {
        return;
    }
    let mut estado = EstadoDoMesh::default();
    estado.preencher_materiais(m);
    if let Ok(mut g) = ESTADOS_DOS_MESHES.write() {
        g.get_or_insert_with(Default::default).entry(caminho).or_insert_with(|| std::sync::Arc::new(Mutex::new(estado)));
    }
}

/// Liga o patch ao mesh alvo pela tag de origem (o caminho do patch): o estado do alvo guarda a referência FRACA ao patch sob a tag e, se o
/// alvo já tem materiais, ganha uma entrada local com o nome da tag apontando o material vazio. Devolve a tag (0 sem efeito).
pub(crate) unsafe fn registrar_patch_de_mesh(alvo: *mut u8, patch: *mut u8) -> u64 {
    let vazio = MATERIAL_VAZIO.load(Ordering::Relaxed);
    if !MESH_DINAMICO_ATIVO.load(Ordering::Relaxed) || alvo.is_null() || patch.is_null() || vazio.is_null() {
        return 0;
    }
    if crate::jogo::listas::dynarray_no_campo(patch, CMESH_MATERIALENTRIES_OFF).1 == 0 {
        return 0;
    }
    let (Some(estado_alvo), Some(_)) = (adquirir_estado_do_mesh(alvo), adquirir_estado_do_mesh(patch)) else { return 0 };
    let tag = (patch.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let mut forte = crate::jogo::objetos::handle_forte_de(patch);
    if tag == 0 || forte[0] == 0 || forte[1] == 0 {
        if forte[0] != 0 {
            crate::jogo::objetos::soltar_handle(&mut forte);
        }
        return 0;
    }
    (*((forte[1] + 4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
    let fraco = forte;
    crate::jogo::objetos::soltar_handle(&mut forte);
    let Ok(mut estado) = estado_alvo.lock() else {
        let mut f = fraco;
        crate::jogo::objetos::soltar_fraco(&mut f);
        return 0;
    };
    if let Some(mut anterior) = estado.origens.insert(tag, fraco) {
        crate::jogo::objetos::soltar_fraco(&mut anterior);
    }
    let tem_materiais = crate::jogo::listas::dynarray_no_campo(alvo, CMESH_MATERIALENTRIES_OFF).1 > 0;
    if tem_materiais && !estado.materiais.contains_key(&tag) {
        criar_entradas_locais(alvo, &mut estado, &[tag], *vazio);
    }
    tag
}

/// Tentativas da trava de material do mesh antes de desistir: a carga não pode esperar um mesh que outro job segura.
const TENTATIVAS_DA_TRAVA_DE_MATERIAL: u32 = 10_000;

pub(crate) static ENTRADAS_LOCAIS_SEM_TRAVA: AtomicU64 = AtomicU64::new(0);

/// Entradas locais novas em `materialEntries` (uma por nome, com `material` forte e fraco, `isLocalInstance` = 1), sob a trava de material do
/// mesh, registradas em `estado.materiais`.
pub(crate) unsafe fn criar_entradas_locais(mesh: *mut u8, estado: &mut EstadoDoMesh, nomes: &[u64], material: [u64; 2]) -> bool {
    if nomes.is_empty() || mesh.is_null() {
        return false;
    }
    let trava = mesh.add(CMESH_MATERIALLOCK_OFF);
    let tentar: unsafe extern "C" fn(*mut u8) -> bool = std::mem::transmute(crate::rebase(crate::jogo::travas::TRAVA_TENTAR_ESCRITA_VM));
    if !(0..TENTATIVAS_DA_TRAVA_DE_MATERIAL).any(|_| tentar(trava)) {
        ENTRADAS_LOCAIS_SEM_TRAVA.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let lista = mesh.add(CMESH_MATERIALENTRIES_OFF);
    let (_, n, cap) = crate::jogo::listas::dynarray_no_campo(lista, 0);
    let total = n + nomes.len();
    if cap < total {
        // Entrada é movida byte a byte (handles mudam de lugar sem mudar contagem): mover nulo.
        crate::jogo::listas::reservar(lista as *mut c_void, total as u32, CMESHMATERIALENTRY_TAMANHO as u32, 8, std::ptr::null());
    }
    let (b, _, cap) = crate::jogo::listas::dynarray_no_campo(lista, 0);
    if b.is_null() || cap < total {
        crate::jogo::travas::soltar_escrita(trava);
        return false;
    }
    for (i, &nome) in nomes.iter().enumerate() {
        let e = b.add((n + i) * CMESHMATERIALENTRY_TAMANHO);
        std::ptr::write_bytes(e, 0, CMESHMATERIALENTRY_TAMANHO);
        if material[1] != 0 {
            handle_strong_soma(&material, 1);
            (*((material[1] + 4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
        }
        for off in [0x00, 0x10] {
            (e.add(off) as *mut u64).write_unaligned(material[0]);
            (e.add(off + 8) as *mut u64).write_unaligned(material[1]);
        }
        (e.add(0x20) as *mut u64).write_unaligned(nome);
        e.add(0x2a).write(1);
        estado.materiais.insert(nome, n + i);
    }
    (lista.add(0x0c) as *mut u32).write_unaligned(total as u32);
    crate::jogo::travas::soltar_escrita(trava);
    true
}

/// Hook do job do builder, antes da original: despacha o `Entity/Extract` com o builder travado e devolve (builder, bloco) pra soltar depois da original.
pub(crate) unsafe fn extract_antes_do_job(params: *mut [u64; 3]) -> Option<(u64, u64)> {
    if !crate::entidades::EXTRACT_ARMADO.load(Ordering::Relaxed)
        || C130D_CONGELADO.load(Ordering::Relaxed)
        || params.is_null()
        || !crate::memoria::is_readable(params as *const c_void, 0x18)
    {
        return None;
    }
    let [builder, _, rc] = *params;
    if builder == 0 || !travar_fraco(rc) {
        return None;
    }
    let anterior = C130D_ATUAL.with(|c| c.replace(builder));
    let anotacoes_de_fora = C130D_ANOTOU.with(|c| c.replace(0));
    C130D_SOMADO.with(|c| c.set(0));
    let enviados = crate::entidades::disparar_extract(builder, rc);
    // Teste 134e: o handler armou o ramo do builder neste despacho — o mesmo builder sai de novo, uma vez, pros alvos recém-armados.
    if C134E_REDESPACHAR.with(|c| c.replace(false)) {
        static REDESPACHOS: AtomicU64 = AtomicU64::new(0);
        C134E_REDESPACHANDO.with(|c| c.set(true));
        let n = crate::entidades::disparar_extract(builder, rc);
        C134E_REDESPACHANDO.with(|c| c.set(false));
        if crate::primeiras(&REDESPACHOS, 4) {
            crate::veredito_dev!("[c134] redespacho do builder {builder:#x}: {n} ouvinte(s)");
        }
    }
    let anotacoes = C130D_ANOTOU.with(|c| c.replace(anotacoes_de_fora));
    C130D_ATUAL.with(|c| c.set(anterior));
    if enviados > 0 {
        let n = C130D_DESPACHOS.fetch_add(1, Ordering::Relaxed);
        if C130D_TESTE.load(Ordering::Relaxed) {
            C130D_SEM_ANOTACAO.fetch_add((anotacoes == 0) as u64, Ordering::Relaxed);
            if n < 4 {
                let ler = |k| crate::diagnostico::ebw_esperado(k, builder).unwrap_or(0);
                crate::veredito_dev!(
                    "[c130d] despacho #{n}: builder {builder:#x}, template {:#018x}, tipo {}, record {:#x}, aparência '{}', {} componente(s) do template, {anotacoes} anotações",
                    ler(6),
                    crate::cname::resolve_cname(ler(7)),
                    ler(0),
                    crate::cname::resolve_cname(ler(2)),
                    ler(12)
                );
            }
        }
    }
    Some((builder, rc))
}

/// Depois da original: o componente somado no despacho tem que estar nos componentes do builder. Solta a trava.
pub(crate) unsafe fn extract_depois_do_job(builder: u64, rc: u64) {
    let somado = C130D_SOMADO.with(|c| c.replace(0));
    if somado != 0 {
        let (entidade, componentes) = crate::diagnostico::ebw_depois_do_job(builder);
        let no_builder = componentes.contains(&somado);
        C130D_NO_BUILDER.fetch_add(no_builder as u64, Ordering::Relaxed);
        if let Ok(mut s) = C130D_SOMADOS.lock() {
            if let Some(e) = s.iter_mut().find(|e| e.0[0] == somado) {
                e.2 = entidade;
            }
        }
        crate::veredito_dev!(
            "[c130d] componente somado {somado:#x} no builder {builder:#x}: depois da original, entidade {entidade:#x} com {} componente(s) no builder, o somado entre eles = {no_builder}",
            componentes.len()
        );
    }
    destravar_fraco(rc);
}

/// O hook do `ExtractPartComponents`, pela 17a ou pela tabela de `resource.patch` (17d). Fica depois da `install_hook_probe!`, que só vale abaixo da
/// definição dela.
pub(crate) unsafe fn install_extract_part_components_hook() {
    if ORIG_EXTRACT_PART_COMPONENTS.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        EXTRACT_PART_COMPONENTS_VM, extract_part_components_tramp, ORIG_EXTRACT_PART_COMPONENTS,
        &format!("[a17] ExtractPartComponents hook @ {EXTRACT_PART_COMPONENTS_VM:#012x}"),
        "[a17] FALHA ao hookar o ExtractPartComponents"
    );
}

/// O hook do `GarmentAssembler::ExtractComponentsJob` (17e), pela tabela de `resource.patch`. Fica depois da `install_hook_probe!`.
pub(crate) unsafe fn install_garment_extract_job_hook() {
    if ORIG_GARMENT_EXTRACT_JOB.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        GARMENT_EXTRACT_JOB_VM, garment_extract_job_hook, ORIG_GARMENT_EXTRACT_JOB,
        &format!("[a17] GarmentAssembler::ExtractComponentsJob hook @ {GARMENT_EXTRACT_JOB_VM:#012x}"),
        "[a17] FALHA ao hookar o ExtractComponentsJob"
    );
}

/// O hook do corpo do job de extração de componentes do `EntityBuilder` pela tabela de `resource.patch` (51f).
pub(crate) unsafe fn install_job_do_builder_hook() {
    if ORIG_JOB_DO_BUILDER.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        JOB_DO_BUILDER_VM, job_do_builder_hook, ORIG_JOB_DO_BUILDER,
        &format!("[a51f] job de componentes do EntityBuilder hook @ {JOB_DO_BUILDER_VM:#012x}"),
        "[a51f] FALHA ao hookar o job de componentes do EntityBuilder"
    );
}

pub(crate) const SUFIXO_CLASSE_PUPPET_VM: u64 = 0x1_0208_8fdc;

pub(crate) const SUFIXO_GENDER_VM: u64 = 0x1_0209_521c;

pub(crate) const SUFIXO_CAMERA_COMP_VM: u64 = 0x1_01c6_e2e4;

pub(crate) const SUFIXO_CAMERA_ALT_VM: u64 = 0x1_0201_ee54;

pub(crate) const SUFIXO_CLASSE_CAMERA_VM: u64 = 0x1_016f_099c;

pub(crate) const SUFIXO_CLASSE_TRANSACAO_VM: u64 = 0x1_0219_b218;

pub(crate) const SUFIXO_PREDICADO_TORSO_VT: u64 = 0x1_0794_8200;

pub(crate) const SUFIXO_ITEMID_VALIDO_VM: u64 = 0x1_0171_2af4;

/// Função do motor que devolve `Handle` pelo `x8`.
pub(crate) unsafe fn chamar_sret_handle(vm: u64, x0: u64, x1: u64) -> [u64; 2] {
    let mut h = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(vm) as u64,
        in("x0") x0,
        in("x1") x1,
        in("x8") h.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    h
}

pub(crate) unsafe fn classe_pelo_getter(vm: u64) -> *mut c_void {
    let f: unsafe extern "C" fn() -> *mut c_void = std::mem::transmute(crate::rebase(vm));
    f()
}

pub(crate) unsafe fn e_da_classe_pelo_getter(obj: u64, vm: u64) -> bool {
    obj != 0 && class_is_kind_of(crate::rtti::class_of(obj as *mut c_void), classe_pelo_getter(vm))
}

/// Valor de 8 B de um flat escalar (`CName`, `TweakDBID`) pelo `GetDataPtr` da vtable do `FlatValue` (`vt+0xF0`).
pub(crate) unsafe fn flat_escalar_u64(t: *const u8, id: u64) -> Option<u64> {
    let fv = crate::tweakdb::get_flat_value_by_id(t, id)?;
    let vt = (fv as *const u64).read_unaligned();
    if !crate::memoria::is_readable(vt as *const c_void, 0xF8) {
        return None;
    }
    let gdp = ((vt as *const u8).add(0xF0) as *const u64).read_unaligned();
    if !crate::rtti::sane(gdp as *mut c_void) {
        return None;
    }
    let f: extern "C" fn(*mut c_void) -> *mut c_void = std::mem::transmute(gdp);
    let p = f(fv as *mut c_void);
    crate::memoria::is_readable(p, 8).then(|| (p as *const u64).read_unaligned())
}

pub(crate) unsafe fn sufixo_gender(dono: u64) -> String {
    if !e_da_classe_pelo_getter(dono, SUFIXO_CLASSE_PUPPET_VM) {
        return String::new();
    }
    let g: unsafe extern "C" fn(u64) -> u64 = std::mem::transmute(crate::rebase(SUFIXO_GENDER_VM));
    crate::cname::resolve_cname(g(dono))
}

pub(crate) unsafe fn sufixo_camera(dono: u64) -> String {
    if dono == 0 {
        return "TPP".to_string();
    }
    let mut h = chamar_sret_handle(SUFIXO_CAMERA_COMP_VM, dono, 0);
    let v = if h[0] != 0 {
        let tpp: unsafe extern "C" fn(u64) -> u8 = std::mem::transmute(crate::rebase(crate::jogo::anexos::ANEXO_SLOT_AFETADO_VM));
        if tpp(h[0]) & 1 != 0 { "TPP" } else { "FPP" }
    } else {
        let mut h2 = chamar_sret_handle(SUFIXO_CAMERA_ALT_VM, dono, 0);
        let v2 = if h2[0] != 0 || !e_da_classe_pelo_getter(dono, SUFIXO_CLASSE_PUPPET_VM) {
            "TPP"
        } else {
            let comp = ((dono + 0x168) as *const u64).read_unaligned();
            if e_da_classe_pelo_getter(comp, SUFIXO_CLASSE_CAMERA_VM) && ((comp + 0x72) as *const u8).read() == 0 { "FPP" } else { "TPP" }
        };
        if h2[0] != 0 {
            crate::jogo::objetos::soltar_handle(&mut h2);
        }
        v2
    };
    if h[0] != 0 {
        crate::jogo::objetos::soltar_handle(&mut h);
    }
    v.to_string()
}

pub(crate) unsafe fn sufixo_partial(dono_h: &[u64; 2], item: &[u8; 16]) -> String {
    let dono = dono_h[0];
    let gi_ptr = if dono != 0 { ((dono + 0xc0) as *const u64).read_unaligned() } else { 0 };
    let gi = if gi_ptr != 0 && crate::memoria::is_readable(gi_ptr as *const c_void, 8) { (gi_ptr as *const u64).read_unaligned() } else { 0 };
    if gi == 0 {
        return "Full".to_string();
    }
    // `GetSystem` do game instance pela classe: `vt+0x10`.
    let vt_gi = (gi as *const u64).read_unaligned();
    let get: unsafe extern "C" fn(u64, *mut c_void) -> u64 = std::mem::transmute(((vt_gi + 0x10) as *const u64).read_unaligned());
    let sistema = get(gi, classe_pelo_getter(SUFIXO_CLASSE_TRANSACAO_VM));
    if sistema == 0 {
        return "Full".to_string();
    }
    let vt = (sistema as *const u64).read_unaligned();
    // Predicado do motor: `+0x00` vtable, `+0x18` ponteiro pra ele mesmo (sem captura; o slot 6 compara o `slotID` com `AttachmentSlots.Torso`).
    let mut predicado = [0u64; 4];
    predicado[0] = crate::rebase(SUFIXO_PREDICADO_TORSO_VT) as u64;
    predicado[3] = predicado.as_mut_ptr() as u64;
    let achar: unsafe extern "C" fn(u64, u64, *mut u64) -> u64 = std::mem::transmute(((vt + 0x458) as *const u64).read_unaligned());
    let slot = achar(sistema, dono, predicado.as_mut_ptr());
    if slot == 0 {
        return "Full".to_string();
    }
    let valido: unsafe extern "C" fn(*const u8) -> u8 = std::mem::transmute(crate::rebase(SUFIXO_ITEMID_VALIDO_VM));
    let Some(p) = [0x28u64, 0x18, 0x58].into_iter().map(|o| (slot + o) as *const u8).find(|p| valido(*p) & 1 != 0) else {
        return "Full".to_string();
    };
    let mut id = [0u8; 16];
    std::ptr::copy_nonoverlapping(p, id.as_mut_ptr(), 16);
    if id[15] != item[15] {
        return "Full".to_string();
    }
    let casa: unsafe extern "C" fn(u64, *const u8, *const u64, u64) -> u8 = std::mem::transmute(((vt + 0x4e8) as *const u64).read_unaligned());
    if casa(sistema, id.as_ptr(), dono_h.as_ptr(), bwms_primitives::fnv1a64(b"hide_T1part")) & 1 != 0 { "Part" } else { "Full" }.to_string()
}

pub(crate) unsafe fn sufixo_generico(reg: &crate::rtti::Registry, dono_h: &[u64; 2], item: &[u8; 16], sufixo: u64) -> String {
    let Some(t) = crate::tweakdb::singleton() else { return String::new() };
    let base = bwms_primitives::tweak_db_id_chave(sufixo);
    let (Some(sistema), Some(funcao)) = (
        flat_escalar_u64(t as *const u8, bwms_primitives::tweak_db_id_derive(base, ".scriptedSystem")),
        flat_escalar_u64(t as *const u8, bwms_primitives::tweak_db_id_derive(base, ".scriptedFunction")),
    ) else {
        return String::new();
    };
    if sistema == 0 || funcao == 0 {
        return String::new();
    }
    let (sn, fnn) = (crate::cname::resolve_cname(sistema), crate::cname::resolve_cname(funcao));
    let sys = crate::jogo::sessao::sistema_de_script_do_jogador(reg, dono_h[0] as *mut c_void, &sn);
    let Some(rf) = (if crate::rtti::sane(sys) { crate::rtti::resolve_func(reg, &sn, &fnn) } else { None }) else {
        return String::new();
    };
    let mut rec = a9_registro(base);
    let r = crate::rtti::call_func(
        &rf,
        sys,
        &[
            crate::rtti::Arg::Item16(*item),
            crate::rtti::Arg::Handle(dono_h[0] as *mut c_void, dono_h[1] as *mut c_void),
            crate::rtti::Arg::Handle(rec[0] as *mut c_void, rec[1] as *mut c_void),
        ],
    );
    if rec[0] != 0 {
        crate::jogo::objetos::soltar_handle(&mut rec);
    }
    let Some(mut s) = r else { return String::new() };
    let texto = crate::rtti::read_cstring(s.as_ptr()).unwrap_or_default();
    destroi_cstring_da_pilha(&mut s);
    texto
}

pub(crate) const ATRIBUTOS_DE_SUFIXO: [(&str, &str); 7] = [
    ("gender", "itemsFactoryAppearanceSuffix.Gender"),
    ("camera", "itemsFactoryAppearanceSuffix.Camera"),
    ("body", "itemsFactoryAppearanceSuffix.BodyType"),
    ("arms", "itemsFactoryAppearanceSuffix.ArmsState"),
    ("feet", "itemsFactoryAppearanceSuffix.FeetState"),
    ("sleeves", "itemsFactoryAppearanceSuffix.Partial"),
    ("hair_type", "itemsFactoryAppearanceSuffix.HairType"),
];

/// Resultado da expansão de texto dinâmico.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TextoProcessado {
    pub valid: bool,
    pub missed: bool,
    pub optional: bool,
    pub value: String,
    /// Hash de cada `{atributo}` lido, com ou sem valor.
    pub atributos: std::collections::BTreeSet<u64>,
}

/// O local vence o global; valor vazio presente conta como achado.
pub(crate) fn processar_texto_dinamico(globais: &std::collections::HashMap<u64, String>, locais: &std::collections::HashMap<u64, String>, entrada: &str) -> TextoProcessado {
    process_dynamic_string_com(entrada, |h| locais.get(&h).or_else(|| globais.get(&h)).map(String::as_str))
}

/// Condição do estado = CName de `atributo=valor`, o texto que o critério `&atributo=valor` dos mods escreve.
pub(crate) fn condicao_do_estado(atributo: &str, valor: &str) -> u64 {
    let ate_o_igual = bwms_primitives::fnv1a64_seeded(b"=", bwms_primitives::fnv1a64(atributo.as_bytes()));
    bwms_primitives::fnv1a64_seeded(valor.as_bytes(), ate_o_igual)
}

/// Nome sem `*` ou texto inválido volta igual; opcional com atributo faltando vira vazio (o mod pediu "só se existir").
pub(crate) fn resolver_nome_dinamico(valores: &std::collections::HashMap<u64, String>, locais: &std::collections::HashMap<u64, String>, nome: &str) -> String {
    if !is_dynamic_value(nome) {
        return nome.to_string();
    }
    let t = processar_texto_dinamico(valores, locais, nome);
    match (t.valid, t.optional && t.missed) {
        (false, _) => nome.to_string(),
        (true, true) => String::new(),
        (true, false) => t.value,
    }
}

/// Estado dinâmico de uma entidade: valor e sufixo por atributo (hash do nome), e as condições.
#[derive(Debug, Default)]
pub(crate) struct EstadoDinamico {
    pub valores: std::collections::HashMap<u64, (String, String)>,
    pub condicoes: std::collections::BTreeSet<u64>,
}

impl EstadoDinamico {
    pub fn valor(&self, atributo: &str) -> &str {
        self.valores.get(&bwms_primitives::fnv1a64(atributo.as_bytes())).map(|(v, _)| v.as_str()).unwrap_or("")
    }
    pub fn globais(&self) -> std::collections::HashMap<u64, String> {
        self.valores.iter().map(|(k, (v, _))| (*k, v.clone())).collect()
    }
}

/// `(valor no estado, texto cru do sufixo)` do item `equipado` (TweakDBID; 0 = nenhum) para o dono. Registro de sufixo ausente → cru vazio.
/// `Gender` vira `m`/`w` (o que os `.app` de mod escrevem em `{gender}`); os outros, snake case do cru.
pub(crate) unsafe fn dado_do_sufixo(reg: &crate::rtti::Registry, dono_h: &[u64; 2], sufixo: u64, equipado: u64) -> (String, String) {
    let mut registro = a9_registro(bwms_primitives::tweak_db_id_chave(sufixo));
    let cru = if registro[0] == 0 {
        String::new()
    } else {
        crate::jogo::objetos::soltar_handle(&mut registro);
        let mut item = [0u8; 16];
        item[..8].copy_from_slice(&equipado.to_le_bytes());
        item[8..12].copy_from_slice(&2u32.to_le_bytes());
        a9_valor_do_sufixo(reg, dono_h, &item, sufixo)
    };
    let genero = bwms_primitives::tweak_db_id_chave(bwms_primitives::tweak_db_id("itemsFactoryAppearanceSuffix.Gender"));
    let valor = if bwms_primitives::tweak_db_id_chave(sufixo) == genero {
        if cru == "Male" { "m" } else { "w" }.to_string()
    } else {
        para_snake_case(&cru)
    };
    (valor, cru)
}

/// Texto do `meshAppearance` do componente pelo RTTI da classe dele; vazio sem o campo, sem leitura ou com valor 0.
pub(crate) unsafe fn nome_da_aparencia_do_componente(comp: *mut c_void) -> String {
    if !rtti::sane(comp) {
        return String::new();
    }
    let classe = rtti::class_of(comp);
    let Some((off, _, _)) = (if classe.is_null() { None } else { rtti::resolve_prop_in_class(classe, "meshAppearance") }) else { return String::new() };
    let campo = (comp as *const u8).add(off as usize);
    if !crate::memoria::is_readable(campo as *const c_void, 8) {
        return String::new();
    }
    match (campo as *const u64).read_unaligned() {
        0 => String::new(),
        h => crate::cname::name_of(h).unwrap_or_default(),
    }
}

/// Componentes de unha do V (`{nails_color}`): masculino, feminino em primeira pessoa e feminino em terceira.
pub(crate) const COMPONENTES_DE_UNHA: [u64; 3] = [
    crate::cname::cname("a0_000_pma_base__nails_l"),
    crate::cname::cname("a0_000_pwa_fpp__nails_l"),
    crate::cname::cname("a0_000_pwa_base_nails_l"),
];

/// Componentes que carregam a cor dos olhos do V.
const COMPONENTES_DE_OLHOS: [u64; 2] = [crate::cname::cname("he_000_pma__basehead"), crate::cname::cname("MorphTargetSkinnedMesh3637")];

/// `(é masculino, pele, olhos, unhas)` pelos componentes do corpo do V em `Entity+0xA0`. A lista é visitada do fim para o começo e o último
/// visitado sobrescreve: com mais de um componente do mesmo papel, vence o de menor índice.
pub(crate) unsafe fn dados_de_customizacao(entidade: *mut c_void) -> (bool, String, String, String) {
    let vazio = (false, String::new(), String::new(), String::new());
    if !rtti::sane(entidade) {
        return vazio;
    }
    let (e, n, cap) = dynarray_em((entidade as *mut u8).add(0xA0) as *mut c_void);
    if e == 0 || n > 4096 || !dynarray_coerente(e, n, cap) || !crate::memoria::is_readable(e as *const c_void, n as usize * 16) {
        return vazio;
    }
    let (mut masc, mut pele, mut olhos, mut unhas) = vazio;
    for i in (0..n as usize).rev() {
        let comp = ((e as usize + i * 16) as *const u64).read_unaligned() as *mut c_void;
        if comp.is_null() || !crate::memoria::is_readable(comp as *const c_void, 0x48) {
            continue;
        }
        let nome = ((comp as *const u8).add(0x40) as *const u64).read_unaligned();
        if nome == CORPO_MASCULINO {
            masc = true;
            pele = nome_da_aparencia_do_componente(comp);
        } else if CORPOS_FEMININOS.contains(&nome) {
            masc = false;
            pele = nome_da_aparencia_do_componente(comp);
        } else if COMPONENTES_DE_OLHOS.contains(&nome) {
            olhos = nome_da_aparencia_do_componente(comp);
        } else if COMPONENTES_DE_UNHA.contains(&nome) {
            unhas = nome_da_aparencia_do_componente(comp);
        }
    }
    (masc, pele, olhos, unhas)
}

/// `GetHairColor` (inlinado no `0x102045888`): estado pela `vt+0x1f8(sistema, masc)`, 1º nome da lista de cores com `vt+0x240(estado, nome)`.
pub(crate) unsafe fn cor_do_cabelo(reg: &crate::rtti::Registry, player: *mut c_void, masc: bool) -> String {
    let sistema = crate::jogo::sessao::sistema_pelo_getter_do_jogador(reg, player, "GetCharacterCustomizationSystem");
    let Some(t) = crate::tweakdb::singleton() else { return String::new() };
    if !crate::rtti::sane(sistema) {
        return String::new();
    }
    let vt = (sistema as *const u64).read_unaligned();
    let mut estado = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) ((vt + 0x1f8) as *const u64).read_unaligned(),
        in("x0") sistema as u64,
        in("x1") masc as u64,
        in("x8") estado.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    if estado[0] == 0 {
        return String::new();
    }
    let cores = crate::tweakdb::get_flat_array_u64_by_id(t as *const u8, bwms_primitives::tweak_db_id(A23_CORES_DO_CABELO)).unwrap_or_default();
    let vt_e = (estado[0] as *const u64).read_unaligned();
    let tem: unsafe extern "C" fn(u64, u64) -> u8 = std::mem::transmute(((vt_e + 0x240) as *const u64).read_unaligned());
    let achada = cores.iter().copied().find(|c| tem(estado[0], *c) & 1 != 0);
    crate::jogo::objetos::soltar_handle(&mut estado);
    achada.map(crate::cname::resolve_cname).unwrap_or_default()
}

/// Estado dinâmico do V (thread do jogo): os 7 sufixos do item `equipado` (0 = nenhum) e as cores de pele, olhos, unhas e cabelo. As cores
/// guardam o cru vazio. Condição = `atributo=valor` de todo atributo com valor.
pub(crate) unsafe fn estado_dinamico_do_player(reg: &crate::rtti::Registry, player: *mut c_void, equipado: u64) -> Option<EstadoDinamico> {
    if player.is_null() {
        return None;
    }
    let mut dono = [0u64; 2];
    rtti::make_handle(dono.as_mut_ptr() as *mut c_void, player);
    if dono[0] == 0 {
        return None;
    }
    let mut estado = EstadoDinamico::default();
    let mut gravar = |atributo: &str, valor: String, cru: String| {
        estado.valores.insert(bwms_primitives::fnv1a64(atributo.as_bytes()), (valor, cru));
    };
    for (atributo, registro) in ATRIBUTOS_DE_SUFIXO {
        let (valor, cru) = dado_do_sufixo(reg, &dono, bwms_primitives::tweak_db_id(registro), equipado);
        gravar(atributo, valor, cru);
    }
    crate::jogo::objetos::soltar_handle(&mut dono);
    let (masc, pele, olhos, unhas) = dados_de_customizacao(player);
    let unhas = if unhas.is_empty() {
        let sistema = crate::jogo::sessao::sistema_pelo_getter_do_jogador(reg, player, "GetCharacterCustomizationSystem");
        cor_da_unha_pela_customizacao(sistema as u64, masc)
    } else {
        unhas
    };
    gravar("skin_color", pele, String::new());
    gravar("eyes_color", olhos, String::new());
    gravar("nails_color", unhas, String::new());
    gravar("hair_color", cor_do_cabelo(reg, player, masc), String::new());
    let condicoes = ATRIBUTOS_DE_SUFIXO
        .iter()
        .map(|(a, _)| *a)
        .chain(["skin_color", "eyes_color", "nails_color", "hair_color"])
        .filter_map(|a| Some(estado.valor(a)).filter(|v| !v.is_empty()).map(|v| condicao_do_estado(a, v)))
        .collect();
    estado.condicoes = condicoes;
    Some(estado)
}

// `{nails_color}` no estado dinâmico NATIVO da 2.5: o `CollectStateData` do motor (`0x101cc3660`, x0 mapa de estados por entidade, x1
// entidade) grava pele/cabelo/olhos pelo `GetCustomizationData` (`0x101cc4598`, só corpo e cabeça) e refaz as condições no fim (`0x101cc5994`, estado
// `+0x30`). O hook grava `nails_color` no mapa de atributos da entidade antes da original, e as condições já saem com a unha.
pub(crate) const COLETAR_ESTADO_VM: u64 = 0x1_01cc_3660;

pub(crate) const COLETAR_ESTADO_PROLOGO: u32 = 0xd103_c3ff;

/// `Map<Entity*, estado>::operator[]` (pega ou cria; x1 `&entidade`, x2 rascunho): o estado começa pelo mapa de atributos.
const ESTADO_DA_ENTIDADE_VM: u64 = 0x1_01cc_53d4;

/// `Map<CName, DynamicAttributeData>::operator[]` (x1 `&CName`, x2 rascunho): valor = 2 `std::string` da libc++, valor em `+0` e sufixo em `+0x18`.
const ATRIBUTO_DO_ESTADO_VM: u64 = 0x1_01cc_5f30;

/// Embrulho de componente do motor 2.5 (`Handle<IComponent>&`, x0 saída de 0x18 B) e o leitor de aparência dele (campo `+0x190`, `+0x230` ou `+0x238` pelo tipo).
const EMBRULHO_DO_COMPONENTE_VM: u64 = 0x1_01cd_7a08;

const APARENCIA_DO_EMBRULHO_VM: u64 = 0x1_01cd_84f4;

pub(crate) static ORIG_COLETAR_ESTADO: AtomicU64 = AtomicU64::new(0);

static CNAME_COR_DA_UNHA: AtomicU64 = AtomicU64::new(0);

static STRING_ASSIGN: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static GRAVACOES_DA_UNHA: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// O `GetCustomizationData` nativo guarda o `CharacterCustomizationSystem` num estático com guarda (`0x108487af0`, guarda `0x108487a00`): 0 antes
/// da 1ª chamada dele, e a 1ª coleta de estado sai sem a reserva da unha.
const SISTEMA_DE_CUSTOMIZACAO_VM: u64 = 0x1_0848_7af0;

const SISTEMA_DE_CUSTOMIZACAO_GUARDA_VM: u64 = 0x1_0848_7a00;

/// Corpo de V pelos componentes, como o `GetCustomizationData`: masculino `t0_000_pma_base__full`, feminino `t0_000_pwa_base__full`/`t0_000_pwa_fpp__torso`.
const CORPO_MASCULINO: u64 = crate::cname::cname("t0_000_pma_base__full");

const CORPOS_FEMININOS: [u64; 2] = [crate::cname::cname("t0_000_pwa_base__full"), crate::cname::cname("t0_000_pwa_fpp__torso")];

/// Reserva sem componente de unha: grupo dos braços e opção do estado de customização; o `definition` perde o prefixo pelo tamanho (o masculino,
/// `a0_000_pma_base__nails_`, tem o mesmo).
const GRUPO_DA_UNHA: u64 = crate::cname::cname("unholstered_mantis");

const OPCAO_DA_UNHA: u64 = crate::cname::cname("u_mantise_nails_color");

const PREFIXO_DA_UNHA: &str = "a0_000_pwa_base__nails_";

/// Aparência do componente de unha da entidade pelo `GetAppearanceName` do motor (último em ordem reversa, como o `GetCustomizationData`; 0 sem
/// componente) e o corpo de V (`Some(masculino)`; `None` sem corpo de V). Só memória e funções do motor: o `CollectStateData` não roda só na
/// game thread.
pub(crate) unsafe fn unha_e_corpo_no_motor(entidade: u64) -> (u64, Option<bool>) {
    let arr = (entidade + 0xA0) as *const u8;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return (0, None);
    }
    let (dados, n) = ((arr as *const u64).read_unaligned(), (arr.add(0x0c) as *const u32).read_unaligned() as u64);
    if dados == 0 || n > 4096 || !crate::memoria::is_readable(dados as *const c_void, (n * 16) as usize) {
        return (0, None);
    }
    let (embrulhar, aparencia) = (crate::rebase(EMBRULHO_DO_COMPONENTE_VM), crate::rebase(APARENCIA_DO_EMBRULHO_VM));
    if embrulhar.is_null() || aparencia.is_null() {
        return (0, None);
    }
    let embrulhar: unsafe extern "C" fn(*mut u64, *const u64) = std::mem::transmute(embrulhar);
    let aparencia: unsafe extern "C" fn(*const u64) -> u64 = std::mem::transmute(aparencia);
    let (mut cor, mut corpo) = (0, None);
    for i in (0..n).rev() {
        let h = (dados + i * 16) as *const u64;
        let comp = h.read_unaligned();
        if comp == 0 || !crate::memoria::is_readable((comp + 0x40) as *const c_void, 8) {
            continue;
        }
        let nome = ((comp + 0x40) as *const u64).read_unaligned();
        if COMPONENTES_DE_UNHA.contains(&nome) {
            let mut embrulho = [0u64; 4];
            embrulhar(embrulho.as_mut_ptr(), h);
            cor = aparencia(embrulho.as_ptr());
        } else if nome == CORPO_MASCULINO {
            corpo = Some(true);
        } else if CORPOS_FEMININOS.contains(&nome) {
            corpo = Some(false);
        }
    }
    (cor, corpo)
}

/// `CharacterCustomizationSystem` do estático do `GetCustomizationData` nativo; 0 antes da guarda ligada.
unsafe fn sistema_de_customizacao_do_motor() -> u64 {
    let (guarda, sistema) = (crate::rebase(SISTEMA_DE_CUSTOMIZACAO_GUARDA_VM) as *const u8, crate::rebase(SISTEMA_DE_CUSTOMIZACAO_VM) as *const u64);
    if guarda.is_null() || sistema.is_null() || !crate::memoria::is_readable(guarda as *const c_void, 1) || !crate::memoria::is_readable(sistema as *const c_void, 8) {
        return 0;
    }
    if (*(guarda as *const std::sync::atomic::AtomicU8)).load(Ordering::Acquire) & 1 == 0 {
        return 0;
    }
    sistema.read_unaligned()
}

/// `definition` da opção de unha nos grupos dos braços do estado de customização (`+0x90`; grupo 0x28 B com nome `+0`, opções `+0x8` e contagem
/// `+0x14`; opção 0x28 B com nome `+0x10` e `definition` `+0x20`, como o `GetArmsAppearances` `0x104bbfc78` lê); 0 sem ela. O grupo é o último
/// com o nome.
pub(crate) unsafe fn definicao_da_unha_no_estado(estado: u64) -> u64 {
    let arr = (estado + 0x90) as *const u8;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return 0;
    }
    let (grupos, n) = ((arr as *const u64).read_unaligned(), (arr.add(0x0c) as *const u32).read_unaligned() as u64);
    if grupos == 0 || n > 4096 || !crate::memoria::is_readable(grupos as *const c_void, (n * 0x28) as usize) {
        return 0;
    }
    let Some(g) = (0..n).rev().map(|i| grupos + i * 0x28).find(|g| (*g as *const u64).read_unaligned() == GRUPO_DA_UNHA) else { return 0 };
    let (opcoes, m) = (((g + 8) as *const u64).read_unaligned(), ((g + 0x14) as *const u32).read_unaligned() as u64);
    if opcoes == 0 || m > 4096 || !crate::memoria::is_readable(opcoes as *const c_void, (m * 0x28) as usize) {
        return 0;
    }
    (0..m)
        .map(|j| opcoes + j * 0x28)
        .find(|o| ((o + 0x10) as *const u64).read_unaligned() == OPCAO_DA_UNHA)
        .map(|o| ((o + 0x20) as *const u64).read_unaligned())
        .unwrap_or(0)
}

/// Cor da unha pelo estado de customização de `sistema`, pego pela `vt+0x1f8(sistema, masculino)` como o `GetHairColor` nativo (`0x10204577c`).
pub(crate) unsafe fn cor_da_unha_pela_customizacao(sistema: u64, masc: bool) -> String {
    if sistema == 0 || !crate::rtti::sane(sistema as *mut c_void) {
        return String::new();
    }
    let vt = (sistema as *const u64).read_unaligned();
    let mut estado = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) ((vt + 0x1f8) as *const u64).read_unaligned(),
        in("x0") sistema,
        in("x1") masc as u64,
        in("x8") estado.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    if estado[0] == 0 {
        return String::new();
    }
    let definicao = definicao_da_unha_no_estado(estado[0]);
    crate::jogo::objetos::soltar_handle(&mut estado);
    if definicao == 0 {
        return String::new();
    }
    crate::cname::resolve_cname(definicao).get(PREFIXO_DA_UNHA.len()..).unwrap_or("").to_string()
}

/// `nails_color` da entidade pelo motor: o componente de unha; sem ele, numa entidade com corpo de V, a reserva pelo estado de customização.
pub(crate) unsafe fn cor_da_unha_do_motor(entidade: u64) -> String {
    match unha_e_corpo_no_motor(entidade) {
        (0, Some(masc)) => cor_da_unha_pela_customizacao(sistema_de_customizacao_do_motor(), masc),
        (0, None) => String::new(),
        (cor, _) => crate::cname::resolve_cname(cor),
    }
}

/// `std::string::assign(const char*)` da libc++ do processo: o mapa do motor solta o texto com o `operator delete` da mesma libc++.
unsafe fn atribuir_string_da_libcxx(s: *mut u8, texto: &std::ffi::CStr) -> bool {
    let mut f = STRING_ASSIGN.load(Ordering::Relaxed);
    if f == 0 {
        use crate::macos::{dlsym, RTLD_DEFAULT};
        f = dlsym(RTLD_DEFAULT, c"_ZNSt3__112basic_stringIcNS_11char_traitsIcEENS_9allocatorIcEEE6assignEPKc".as_ptr()) as u64;
        if f == 0 {
            return false;
        }
        STRING_ASSIGN.store(f, Ordering::Relaxed);
    }
    let assign: unsafe extern "C" fn(*mut u8, *const std::ffi::c_char) -> *mut u8 = std::mem::transmute(f);
    assign(s, texto.as_ptr());
    true
}

/// `nails_color` no mapa de atributos da entidade, valor e sufixo iguais como o motor grava as outras cores; vazio sem unha.
unsafe fn gravar_cor_da_unha(estados: u64, entidade: u64) {
    let (estado_de, atributo_de) = (crate::rebase(ESTADO_DA_ENTIDADE_VM), crate::rebase(ATRIBUTO_DO_ESTADO_VM));
    if estado_de.is_null() || atributo_de.is_null() {
        return;
    }
    let mut chave = CNAME_COR_DA_UNHA.load(Ordering::Relaxed);
    if chave == 0 {
        // Registrado no pool do motor: o mapa de atributos do estado nativo é lido por nome.
        chave = crate::cname::registrar_no_pool("nails_color");
        CNAME_COR_DA_UNHA.store(chave, Ordering::Relaxed);
    }
    let Ok(texto) = std::ffi::CString::new(cor_da_unha_do_motor(entidade)) else { return };
    let estado_de: unsafe extern "C" fn(u64, *mut u64, *mut u64) -> u64 = std::mem::transmute(estado_de);
    let atributo_de: unsafe extern "C" fn(u64, *const u64, *mut u64) -> *mut u8 = std::mem::transmute(atributo_de);
    let (mut ent, mut rascunho) = (entidade, [0u64; 8]);
    let estado = estado_de(estados, &mut ent, rascunho.as_mut_ptr());
    if estado == 0 {
        return;
    }
    let valor = atributo_de(estado, &chave, rascunho.as_mut_ptr());
    if valor.is_null() || !atribuir_string_da_libcxx(valor, &texto) {
        return;
    }
    atribuir_string_da_libcxx(valor.add(0x18), &texto);
    #[cfg(feature = "devtools")]
    {
        if GRAVACOES_DA_UNHA.fetch_add(1, Ordering::Relaxed) < 4 {
            crate::veredito_dev!("[unha] nails_color='{}' entidade={entidade:#x} estado={estado:#x}", texto.to_string_lossy());
        }
        if !texto.as_bytes().is_empty() {
            if let Ok(mut u) = ULTIMA_UNHA_GRAVADA.lock() {
                *u = (entidade, texto.to_string_lossy().into_owned());
            }
        }
    }
}

/// Última gravação não vazia do hook: `(entidade, texto)`, para o teste comparar com o estado do BWMS no V.
#[cfg(feature = "devtools")]
pub(crate) static ULTIMA_UNHA_GRAVADA: std::sync::Mutex<(u64, String)> = std::sync::Mutex::new((0, String::new()));

/// `x2` vem dos chamadores (`0` ou o `x1` deles, `0x101cd02c0`/`0x101cd02d4`); a original o sobrescreve sem ler, e ele segue repassado.
pub(crate) unsafe extern "C" fn coletar_estado_hook(estados: u64, entidade: u64, x2: u64) -> u64 {
    if estados != 0 && entidade != 0 {
        gravar_cor_da_unha(estados, entidade);
    }
    let f: unsafe extern "C" fn(u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_COLETAR_ESTADO));
    f(estados, entidade, x2)
}

/// No `on_load`, sempre: o `CollectStateData` só roda para item de aparência dinâmica.
pub(crate) unsafe fn install_cor_da_unha() {
    let r = instalar_conferindo_prologo(COLETAR_ESTADO_VM, COLETAR_ESTADO_PROLOGO, coletar_estado_hook as *mut c_void, &ORIG_COLETAR_ESTADO);
    crate::veredito_dev!("[unha] hook do CollectStateData: {r:?}");
}

#[cfg(test)]
mod testes_da_unha {
    use super::{definicao_da_unha_no_estado, GRUPO_DA_UNHA, OPCAO_DA_UNHA, PREFIXO_DA_UNHA};
    use crate::cname::cname;

    /// Opção de 0x28 B: nome `+0x10`, `definition` `+0x20`.
    fn opcao(nome: u64, definicao: u64) -> [u64; 5] {
        [0, 0, nome, 0, definicao]
    }

    /// Grupo de 0x28 B: nome `+0`, `DynArray` de opções em `+0x8` (contagem u32 em `+0x14`).
    fn grupo(nome: u64, opcoes: &[[u64; 5]]) -> [u64; 5] {
        [nome, opcoes.as_ptr() as u64, (opcoes.len() as u64) << 32, 0, 0]
    }

    fn estado(grupos: &[[u64; 5]]) -> [u64; 0x14] {
        let mut e = [0u64; 0x14];
        e[0x90 / 8] = grupos.as_ptr() as u64;
        e[0x98 / 8] = (grupos.len() as u64) << 32;
        e
    }

    #[test]
    fn definicao_vem_do_ultimo_grupo_com_o_nome() {
        let a = [opcao(cname("outra"), 1), opcao(OPCAO_DA_UNHA, 0xAA)];
        let b = [opcao(OPCAO_DA_UNHA, 0xBB)];
        let grupos = [grupo(GRUPO_DA_UNHA, &a), grupo(cname("hands"), &a), grupo(GRUPO_DA_UNHA, &b)];
        let e = estado(&grupos);
        assert_eq!(unsafe { definicao_da_unha_no_estado(e.as_ptr() as u64) }, 0xBB);
    }

    #[test]
    fn sem_opcao_no_ultimo_grupo_nao_procura_nos_outros() {
        let a = [opcao(OPCAO_DA_UNHA, 0xAA)];
        let b = [opcao(cname("outra"), 1)];
        let grupos = [grupo(GRUPO_DA_UNHA, &a), grupo(GRUPO_DA_UNHA, &b)];
        let e = estado(&grupos);
        assert_eq!(unsafe { definicao_da_unha_no_estado(e.as_ptr() as u64) }, 0);
        let sem_grupo = estado(&[]);
        assert_eq!(unsafe { definicao_da_unha_no_estado(sem_grupo.as_ptr() as u64) }, 0);
    }

    #[test]
    fn prefixo_masculino_tem_o_tamanho_do_feminino() {
        assert_eq!("a0_000_pma_base__nails_".len(), PREFIXO_DA_UNHA.len());
        assert_eq!("a0_000_pwa_base__nails_red".get(PREFIXO_DA_UNHA.len()..), Some("red"));
    }
}

// Nome composto de material: `a+b+c` dá `{material.1}`..`{material.N}` no caminho dinâmico do material, além do `{material}`. Na 2.5
// o `ExpandResourcePath` nativo (`0x101ce2d38`) monta os locais do `ProcessString` (`0x101ce4040`) só com `{material, nome}`, pelo construtor de
// `std::map<CName, CName>` (`0x101ce84ac`: x0 saída, x1 pares de 16 B, x2 contagem), que só ele chama. O hook troca esse par único pelos pares com
// as partes.
pub(crate) const LOCAIS_DO_MATERIAL_VM: u64 = 0x1_01ce_84ac;

pub(crate) const LOCAIS_DO_MATERIAL_PROLOGO: u32 = 0xa9bb_67fa;

pub(crate) static ORIG_LOCAIS_DO_MATERIAL: AtomicU64 = AtomicU64::new(0);

const ATRIBUTO_MATERIAL: u64 = crate::cname::cname("material");

/// Partes de um nome composto de material, na ordem: `+` no começo, repetido ou no fim não gera parte; sem `+`, o nome inteiro é a parte 1.
pub(crate) fn partes_do_material(nome: &str) -> Vec<&str> {
    nome.split('+').filter(|p| !p.is_empty()).collect()
}

/// Pares `{material, nome}` seguidos de `{material.N, parte N}`, com cada nome registrado no pool do motor: o expansor nativo lê os nomes pelo pool.
unsafe fn pares_do_material_composto(valor: u64) -> Vec<[u64; 2]> {
    let mut pares = vec![[ATRIBUTO_MATERIAL, valor]];
    let nome = crate::cname::resolve_cname(valor);
    for (i, parte) in partes_do_material(&nome).into_iter().enumerate() {
        pares.push([crate::cname::registrar_no_pool(&format!("material.{}", i + 1)), crate::cname::registrar_no_pool(parte)]);
    }
    pares
}

pub(crate) unsafe extern "C" fn locais_do_material_hook(saida: u64, pares: *const [u64; 2], n: u64, x3: u64) -> u64 {
    let f: unsafe extern "C" fn(u64, *const [u64; 2], u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_LOCAIS_DO_MATERIAL));
    if n == 1 && !pares.is_null() {
        let [chave, valor] = pares.read_unaligned();
        if chave == ATRIBUTO_MATERIAL && valor != 0 {
            let compostos = pares_do_material_composto(valor);
            return f(saida, compostos.as_ptr(), compostos.len() as u64, x3);
        }
    }
    f(saida, pares, n, x3)
}

/// No `on_load`, sempre: o construtor só roda para caminho de material dinâmico (`*`).
pub(crate) unsafe fn install_material_composto() {
    let r = instalar_conferindo_prologo(LOCAIS_DO_MATERIAL_VM, LOCAIS_DO_MATERIAL_PROLOGO, locais_do_material_hook as *mut c_void, &ORIG_LOCAIS_DO_MATERIAL);
    crate::veredito_dev!("[composto] hook dos locais do ExpandResourcePath: {r:?}");
}

/// `std::map<CName, CName>` da libc++ (`{begin, raiz, tamanho}`; nó com filhos `+0`/`+8`, chave `+0x20`, valor `+0x28`) em ordem, para o teste.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn pares_do_mapa_de_cnames(mapa: *const u64) -> Vec<(u64, u64)> {
    unsafe fn visitar(no: u64, saida: &mut Vec<(u64, u64)>) {
        if no == 0 || saida.len() > 64 || !crate::memoria::is_readable(no as *const c_void, 0x30) {
            return;
        }
        let p = no as *const u64;
        visitar(p.read_unaligned(), saida);
        saida.push((p.add(4).read_unaligned(), p.add(5).read_unaligned()));
        visitar(p.add(1).read_unaligned(), saida);
    }
    let mut saida = Vec::new();
    visitar(mapa.add(1).read_unaligned(), &mut saida);
    saida
}

/// Destrutor de subárvore do `std::map` (`0x1000b1108`: x0 mapa, x1 nó).
#[cfg(feature = "devtools")]
pub(crate) const DESTRUIR_ARVORE_DO_MAPA_VM: u64 = 0x1_000b_1108;

#[cfg(test)]
mod testes_do_material_composto {
    use super::partes_do_material;

    #[test]
    fn partes_ignoram_mais_vazio() {
        assert_eq!(partes_do_material("a+b+c"), ["a", "b", "c"]);
        assert_eq!(partes_do_material("+a++b+"), ["a", "b"]);
        assert_eq!(partes_do_material("inteiro"), ["inteiro"]);
        assert!(partes_do_material("").is_empty());
        assert!(partes_do_material("+").is_empty());
    }
}

/// `ResolveName`/`ResolvePath` de produção (`BwmsAparencia.ResolveName`/`ResolvePath` e o `ApplyAppearanceOverride` do `#22`): nome sem `*` volta igual sem
/// calcular estado; com `*`, o estado da entidade na hora, sem item equipado: a native não sabe qual item está sendo montado.
pub(crate) unsafe fn resolver_nome_da_entidade(reg: &crate::rtti::Registry, entidade: *mut c_void, nome: &str) -> String {
    if !nome.starts_with('*') {
        return nome.to_string();
    }
    match estado_dinamico_do_player(reg, entidade, 0) {
        Some(estado) => resolver_nome_dinamico(&estado.globais(), &std::collections::HashMap::new(), nome),
        None => nome.to_string(),
    }
}

// `ChangeAppearance`/`ChangeAppearances` do `RuntimeSystemEntityAppearanceChanger` com o `ApplyAppOverride` antes da original
// (tabela em `app_overrides`). Desmonte de 2026-09-14: `ChangeAppearance` `0x101a59e30` (x0 sistema, x1
// `AppearanceChangeRequest*` de 0x38 B {WeakHandle @0, velho @0x10, novo @0x20, flags @0x30}, x2 `std::function&`, w3); `ChangeAppearances` `0x101a5c6e0`
// (x0 sistema, x1 `Handle<Puppet>&`, as 2 `Range<AppearanceDescriptor>` por valor em x2/x3 e x4/x5, início e fim, 16 B por elemento, x6
// `std::function&`, w7 u8). Quem chama: o `gameuiAttachmentSlotsListener` do V (slots
// Chest/Legs/Torso/Outfit/Underwear, `0x104ba9ef8`–`0x104bafcb8`) e o `Entity.ScheduleAppearanceChange` (`0x1019ad5d8` → `0x101a59da0`, pedido com
// recurso 0, que o `operator bool` do descritor deixa de fora).
pub(crate) const CHANGE_APPEARANCE_VM: u64 = 0x1_01a5_9e30;

// 2.5: as duas originais chamam o ouvinte global `[0x109cc59a0]` (slot `+0x60` com o pedido no `ChangeAppearance`) antes de usar os descritores; o hook
// roda antes da original, e o ouvinte vê o pedido já com o override.
pub(crate) const CHANGE_APPEARANCE_PROLOGO: u32 = 0xd107_03ff;

pub(crate) const CHANGE_APPEARANCES_VM: u64 = 0x1_01a5_c6e0;

pub(crate) const CHANGE_APPEARANCES_PROLOGO: u32 = 0xd106_03ff;

pub(crate) static ORIG_CHANGE_APPEARANCE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CHANGE_APPEARANCES: AtomicU64 = AtomicU64::new(0);

pub(crate) unsafe extern "C" fn change_appearance_hook(sistema: u64, pedido: *mut u64, funcao: u64, flag: u64) -> u64 {
    if !pedido.is_null() {
        let antes = A11_TESTE.load(Ordering::Relaxed).then(|| (a11_descritores(pedido.add(2), 1), a11_descritores(pedido.add(4), 1)));
        let trocados = crate::aparencia::aplicar_app_overrides(pedido.add(2), 1) + crate::aparencia::aplicar_app_overrides(pedido.add(4), 1);
        if let Some((velho, novo)) = antes {
            a11_anotar(0, pedido.read_unaligned(), velho, novo, trocados);
        }
    }
    let f: unsafe extern "C" fn(u64, *mut u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_CHANGE_APPEARANCE));
    f(sistema, pedido, funcao, flag)
}

pub(crate) unsafe extern "C" fn change_appearances_hook(
    sistema: u64,
    boneco: *const u64,
    velho_ini: *mut u64,
    velho_fim: *mut u64,
    novo_ini: *mut u64,
    novo_fim: *mut u64,
    funcao: u64,
    flag: u64,
) -> u64 {
    let nv = crate::aparencia::descritores_do_range(velho_ini as u64, velho_fim as u64).unwrap_or(0);
    let nn = crate::aparencia::descritores_do_range(novo_ini as u64, novo_fim as u64).unwrap_or(0);
    let antes = A11_TESTE.load(Ordering::Relaxed).then(|| (a11_descritores(velho_ini, nv), a11_descritores(novo_ini, nn)));
    let trocados = crate::aparencia::aplicar_app_overrides(velho_ini, nv) + crate::aparencia::aplicar_app_overrides(novo_ini, nn);
    if let Some((velho, novo)) = antes {
        a11_anotar(1, if boneco.is_null() { 0 } else { boneco.read_unaligned() }, velho, novo, trocados);
    }
    let f: unsafe extern "C" fn(u64, *const u64, *mut u64, *mut u64, *mut u64, *mut u64, u64, u64) -> u64 =
        std::mem::transmute(original_publicado(&ORIG_CHANGE_APPEARANCES));
    f(sistema, boneco, velho_ini, velho_fim, novo_ini, novo_fim, funcao, flag)
}

/// No `on_load`: os 2 hooks, sempre (com a tabela vazia só repassam); com `~/.bwms-hook-a11-teste`, as anotações e o `resource.link`.
pub(crate) unsafe fn install_app_overrides() {
    let teste = marcador_individual("a11-teste");
    A11_TESTE.store(teste, Ordering::Relaxed);
    let mut pre = 0;
    if teste {
        install_reslink();
        // Pares gravados pelo teste anterior recebem override antes da carga: o `ChangeAppearances` só roda nela (boot 212).
        let pares: Vec<[u64; 3]> = a11_pares_gravados().into_iter().map(|(r, a)| [r, a, a11_alias(r)]).collect();
        for (i, [r, a, alias]) in pares.iter().enumerate() {
            reslink_add(*alias, *r);
            ALIASES_DO_TESTE_DE_OVERRIDE[i].store(*alias, Ordering::Relaxed);
            crate::aparencia::registrar_app_override(*r, *alias, *a);
        }
        pre = pares.len();
        if let Ok(mut p) = A11_PRE.lock() {
            *p = pares;
        }
    }
    let um = instalar_conferindo_prologo(CHANGE_APPEARANCE_VM, CHANGE_APPEARANCE_PROLOGO, change_appearance_hook as *mut c_void, &ORIG_CHANGE_APPEARANCE);
    let varios = instalar_conferindo_prologo(CHANGE_APPEARANCES_VM, CHANGE_APPEARANCES_PROLOGO, change_appearances_hook as *mut c_void, &ORIG_CHANGE_APPEARANCES);
    crate::veredito_dev!(
        "[a11] hooks no on_load: ChangeAppearance {um:?}, ChangeAppearances {varios:?} (teste {teste}, {pre} par(es) com override antes da carga)"
    );
}


/// Bitset de objetos do extrator com os `n` primeiros ligados, em palavras de 32 bits; a última mascarada como no `ExtractPartComponents`.
pub(crate) fn bitset_do_extrator(n: usize) -> Vec<u32> {
    let mut v = vec![u32::MAX; n.div_ceil(32)];
    if n % 32 != 0 {
        if let Some(u) = v.last_mut() {
            *u = (1u32 << (n % 32)) - 1;
        }
    }
    v
}

/// Bloco de params do extrator (0xa0 B) na forma do `ExtractPartComponents`: `+0x00`–`+0x18` = cabeçalho `+0x18`–`+0x30`, `+0x18`–`+0x58` = cabeçalho
/// `+0x30`–`+0x70`, `+0x58` loader, `+0x60` as 5 flags, `+0x68` bitset `{ponteiro, capacidade, tamanho}`, `+0x78` total de objetos, resto zerado.
pub(crate) fn params_do_extrator(cabecalho: &[u8; 0x70], loader: u64, flags: [u8; 5], bitset: u64, palavras: u32, objetos: u32) -> [u8; 0xa0] {
    let mut p = [0u8; 0xa0];
    p[0x00..0x18].copy_from_slice(&cabecalho[0x18..0x30]);
    p[0x18..0x58].copy_from_slice(&cabecalho[0x30..0x70]);
    p[0x58..0x60].copy_from_slice(&loader.to_le_bytes());
    p[0x60..0x65].copy_from_slice(&flags);
    p[0x68..0x70].copy_from_slice(&bitset.to_le_bytes());
    p[0x70..0x74].copy_from_slice(&palavras.to_le_bytes());
    p[0x74..0x78].copy_from_slice(&palavras.to_le_bytes());
    p[0x78..0x7c].copy_from_slice(&objetos.to_le_bytes());
    p
}

/// Extrai o pacote a partir do cabeçalho de 0x70 B (o `+0x1d8` do template ou o `+0x148` da definição de aparência), com o mesmo contrato do `extrair_pacote_do_template`.
pub(crate) unsafe fn extrair_pacote_do_cabecalho(cabecalho: u64, flags: [u8; 5], bitset: Option<Vec<u32>>, saida: *mut [u64; 2]) -> Option<usize> {
    if cabecalho == 0 || !crate::memoria::is_readable(cabecalho as *const c_void, 0x70) {
        return None;
    }
    let mut cab = [0u8; 0x70];
    std::ptr::copy_nonoverlapping(cabecalho as *const u8, cab.as_mut_ptr(), 0x70);
    let ini = u64::from_le_bytes(cab[0x20..0x28].try_into().ok()?);
    let fim = u64::from_le_bytes(cab[0x28..0x30].try_into().ok()?);
    let n = (fim.checked_sub(ini)? / 8) as usize;
    if ini == 0 || n == 0 || n > 0x10000 {
        return None;
    }
    let bits = bitset.unwrap_or_else(|| bitset_do_extrator(n));
    if bits.len() != n.div_ceil(32) {
        return None;
    }
    let loader = (crate::rebase(crate::jogo::recursos::CARREGADOR_VM) as *const u64).read_unaligned();
    let params = params_do_extrator(&cab, loader, flags, bits.as_ptr() as u64, bits.len() as u32, n as u32);
    let mut ext: u64 = 0;
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(EXTRATOR_CRIAR_VM) as u64,
        in("x0") params.as_ptr() as u64,
        in("x8") &mut ext as *mut u64 as u64,
        clobber_abi("C"),
    );
    drop(bits);
    if ext == 0 {
        return None;
    }
    let destruir: unsafe extern "C" fn(u64) = std::mem::transmute(crate::rebase(EXTRATOR_DESTRUIR_VM));
    let liberar: unsafe extern "C" fn(u64, u32) = std::mem::transmute(crate::rebase(crate::jogo::objetos::POOL_LIBERAR_VM));
    let alocar: unsafe extern "C" fn(u64) -> *mut u8 = std::mem::transmute(crate::rebase(crate::jogo::objetos::CONTAGEM_ALOCAR_VM));
    let rc = alocar(8);
    if rc.is_null() {
        destruir(ext);
        liberar(ext, 1);
        return None;
    }
    (rc as *mut u32).write_unaligned(1);
    (rc.add(4) as *mut u32).write_unaligned(1);
    // Weak próprio em `+0x00`, como o `ExtractPartComponents` põe antes de extrair.
    (*(rc.add(4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
    (ext as *mut u64).write_unaligned(ext);
    ((ext + 8) as *mut u64).write_unaligned(rc as u64);
    let extrair: unsafe extern "C" fn(u64) = std::mem::transmute(crate::rebase(EXTRATOR_EXTRAIR_VM));
    extrair(ext);
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(EXTRATOR_TOMAR_RESULTADOS_VM) as u64,
        in("x0") ext,
        in("x8") saida as u64,
        clobber_abi("C"),
    );
    // Soltura do `SharedPtr` como a do motor: strong 1 → 0, weak do ponteiro, destrutor (solta o weak próprio e o bloco) e a memória do extrator.
    if (*(rc as *const std::sync::atomic::AtomicU32)).fetch_sub(1, Ordering::SeqCst) == 1 {
        let mut sp = [ext, rc as u64];
        let soltar: unsafe extern "C" fn(*mut [u64; 2]) = std::mem::transmute(crate::rebase(crate::jogo::objetos::FRACO_SOLTAR_BLOCO_VM));
        soltar(&mut sp);
        destruir(ext);
        liberar(ext, 1);
    }
    Some(((*saida)[1] >> 32) as usize)
}


/// Ordem padrão dos sufixos de aparência (o registro cuja lista `appearanceSuffixes` o motor percorre).
pub(crate) const ORDEM_PADRAO_DE_SUFIXOS: &str = "itemsFactoryAppearanceSuffix.ItemsFactoryAppearanceSuffixOrderDefault";

// ===== `PuppetStateHandler` (2026-09-12) =====
// O handler é redscript (`ponte-puppetstate-armsdetect.reds`): o listener nativo de script repassa os eventos de slot por nome. Aqui ficam os 4
// registros de sufixo que o handler responde e o teste, que chama os 5 passos do
// `BwmsPuppetStateTeste` e confere pelo caminho do motor (`a9_valor_do_sufixo`: flats do registro → sistema de script → função) que o sufixo devolve o
// estado do handler.
const REGISTROS_DE_SUFIXO: [(&str, &str); 4] = [
    ("itemsFactoryAppearanceSuffix.BodyType", "GetBodyTypeSuffix"),
    ("itemsFactoryAppearanceSuffix.ArmsState", "GetArmsStateSuffix"),
    ("itemsFactoryAppearanceSuffix.LegsState", "GetFeetStateSuffix"),
    ("itemsFactoryAppearanceSuffix.FeetState", "GetFeetStateSuffix"),
];
/// O único sufixo vanilla com sistema de script: o clone herda os 3 flats (`instantSwitch=false`, `scriptedSystem`, `scriptedFunction`) com o tipo certo.
const MOLDE_DE_SUFIXO: &str = "itemsFactoryAppearanceSuffix.HairType";
#[cfg(feature = "devtools")]
pub(crate) const VALORES_DE_BRACOS: [&str; 4] = ["BaseArms", "MantisBlades", "Monowire", "ProjectileLauncher"];
#[cfg(feature = "devtools")]
pub(crate) const VALORES_DE_PES: [&str; 5] = ["", "Flat", "Lifted", "HighHeels", "FlatShoes"];
pub(crate) static REGISTROS_DE_SUFIXO_OK: AtomicBool = AtomicBool::new(false);

/// Os 4 registros de sufixo, uma vez por processo, no 1º tick com o TweakDB carregado e o executor livre (no menu, antes do load do
/// save). Registro que já existe fica como está, e o id só entra no fim da ordem padrão se faltar.
pub(crate) fn registros_de_sufixo_tick() {
    static FEITO: AtomicBool = AtomicBool::new(false);
    if FEITO.load(Ordering::Relaxed) || crate::inicio::boot::exec_nested() {
        return;
    }
    unsafe {
        let (Some(reg), Some(t)) = (crate::rtti::Registry::obtain(), crate::tweakdb::singleton()) else { return };
        if !crate::memoria::is_readable(t.add(0x38) as *const c_void, 1) || *t.add(0x38) != 1 {
            return;
        }
        FEITO.store(true, Ordering::Relaxed);
        let molde = bwms_primitives::tweak_db_id(MOLDE_DE_SUFIXO);
        let doador = bwms_primitives::tweak_db_id_derive(molde, ".scriptedSystem");
        let sistema = crate::cname::cname("PuppetStateSystem");
        let lista = bwms_primitives::tweak_db_id(&format!("{ORDEM_PADRAO_DE_SUFIXOS}.appearanceSuffixes"));
        let mut falhas: Vec<String> = Vec::new();
        for (nome, funcao) in REGISTROS_DE_SUFIXO {
            let id = bwms_primitives::tweak_db_id(nome);
            if crate::tweakdb::get_record_instance(t, id).is_null() {
                let criado = crate::tweakdb::clone_record_by_id(&reg, id, molde)
                    && crate::tweakdb::set_flat_nonscalar_by_id(t, bwms_primitives::tweak_db_id_derive(id, ".scriptedSystem"), doador, &sistema.to_le_bytes(), 8).is_some()
                    && crate::tweakdb::set_flat_nonscalar_by_id(
                        t,
                        bwms_primitives::tweak_db_id_derive(id, ".scriptedFunction"),
                        doador,
                        &crate::cname::cname(funcao).to_le_bytes(),
                        8,
                    )
                    .is_some()
                    && crate::tweakdb::update_record_by_id(id);
                if !criado {
                    falhas.push(format!("{nome}: registro"));
                }
            }
            if !crate::tweakdb::array_append_by_id(t, lista, id, true) {
                falhas.push(format!("{nome}: ordem"));
            }
        }
        if !crate::tweakdb::update_record_by_id(bwms_primitives::tweak_db_id(ORDEM_PADRAO_DE_SUFIXOS)) {
            falhas.push("UpdateRecord da ordem".into());
        }
        REGISTROS_DE_SUFIXO_OK.store(falhas.is_empty(), Ordering::Relaxed);
        crate::veredito_dev!(
            "[a29] registros de sufixo: {}",
            if falhas.is_empty() { "BodyType/ArmsState/LegsState/FeetState prontos e na ordem padrão".to_string() } else { format!("falhou {}", falhas.join(", ")) }
        );
    }
}

// ===== 51f-2: carga do `compiledData` das definições dos `.app` de patch (pedida no tick, retida pelo handle). =====
const PEDIR_CARGA_DO_BUFFER_VM: u64 = 0x1_0121_5524;

/// Recursos de patch com a carga já pedida e os handles das cargas (retidos: a contagem de carga do buffer segura os dados).
static CARGAS_DE_DEFINICAO_PEDIDAS: std::sync::Mutex<(Vec<u64>, Vec<Box<[u8; 0x40]>>)> = std::sync::Mutex::new((Vec::new(), Vec::new()));

/// Pede a carga do `compiledData` da definição (`DeferredDataBuffer/CreateLoadingJobs`) e retém o handle.
pub(crate) unsafe fn pedir_carga_da_definicao(def: u64) {
    let mut saida = Box::new([0u8; 0x40]);
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(PEDIR_CARGA_DO_BUFFER_VM) as u64,
        in("x0") def + APPDEF_COMPILEDDATA_OFF,
        in("x1") 0u64,
        in("x8") saida.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    if let Ok(mut g) = CARGAS_DE_DEFINICAO_PEDIDAS.lock() {
        g.1.push(saida);
    }
}

/// Carga pedida nas definições de um `.app` de patch que ainda não têm cabeçalho nem dados. Devolve quantas.
unsafe fn pedir_cargas_do_app(recurso: u64) -> usize {
    let Some(defs) = definicoes_do_app(recurso as *const u8) else { return 0 };
    let fechadas: Vec<u64> = defs.iter().map(|h| h[0]).filter(|d| objetos_do_cabecalho(&cabecalho_da_definicao(*d)) == 0 && compileddata_da_definicao(*d).is_none()).collect();
    for d in &fechadas {
        pedir_carga_da_definicao(*d);
    }
    fechadas.len()
}

/// 51f-2 no tick (thread do jogo): pede a carga do `compiledData` das definições de cada `.app` de patch assim que o token fica pronto.
pub(crate) fn cargas_de_definicao_tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if !MESCLA_DE_APARENCIA_ATIVA.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 60) {
        return;
    }
    unsafe {
        let recursos: Vec<(u64, u64)> = match PATCHES_DE_RECURSO.try_read() {
            Ok(g) => g
                .apps()
                .filter(|p| p.token[0] != 0 && crate::register::status_do_token(p.token[0]) == 1 && crate::memoria::is_readable((p.token[0] + 0x28) as *const c_void, 8))
                .map(|p| (p.patch, ((p.token[0] + 0x28) as *const u64).read_unaligned()))
                .filter(|(_, r)| *r != 0)
                .collect(),
            Err(_) => return,
        };
        let pedidos = CARGAS_DE_DEFINICAO_PEDIDAS.lock().map(|g| g.0.clone()).unwrap_or_default();
        for (patch, r) in recursos.into_iter().filter(|(_, r)| !pedidos.contains(r)) {
            let n = pedir_cargas_do_app(r);
            if let Ok(mut g) = CARGAS_DE_DEFINICAO_PEDIDAS.lock() {
                g.0.push(r);
            }
            crate::veredito_dev!("[a51f2] carga do compiledData pedida em {n} definição(ões) do .app de patch {patch:#018x} ({r:#x})");
        }
    }
}

/// Nome e texto dinâmico de aparência.
#[cfg(test)]
mod testes_do_nome_dinamico {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet, HashMap};

    fn h(s: &str) -> u64 {
        bwms_primitives::fnv1a64(s.as_bytes())
    }

    fn conjunto(itens: &[&str]) -> BTreeSet<u64> {
        itens.iter().map(|s| if s.is_empty() { 0 } else { h(s) }).collect()
    }

    fn criterio(texto: &str) -> (u64, u64, BTreeSet<u64>, BTreeSet<u64>, bool, bool, i32) {
        let r = DynamicAppearanceRef::parse(texto);
        (r.value, r.name, r.variants, r.conditions, r.is_dynamic, r.is_conditional, r.weight)
    }

    #[test]
    fn criterio_separa_variantes_e_condicoes() {
        assert_eq!(criterio(""), (0, 0, conjunto(&[]), conjunto(&[]), false, false, 0));
        assert_eq!(criterio("torso"), (h("torso"), h("torso"), conjunto(&[]), conjunto(&[]), false, false, 0));
        assert_eq!(criterio("torso!v1!v2&c1&c2"), (h("torso!v1!v2&c1&c2"), h("torso"), conjunto(&["v1", "v2"]), conjunto(&["c1", "c2"]), true, true, 102));
        assert_eq!(criterio("eyes&male"), (h("eyes&male"), h("eyes"), conjunto(&[]), conjunto(&["male"]), true, true, 1));
        assert_eq!(criterio("&cond1").1, 0);
        assert_eq!(criterio("!"), (h("!"), 0, conjunto(&[]), conjunto(&[]), true, false, 0));
        assert_eq!(criterio("!!").2, conjunto(&[""]));
        assert_eq!(criterio("&&").3, conjunto(&[""]));
        assert_eq!((criterio("a!").5, criterio("a&").5), (false, false));
        assert_eq!(criterio("a!!b").2, conjunto(&["", "b"]));
        assert_eq!(criterio("a!b!!c").2, conjunto(&["", "b", "c"]));
        assert_eq!((criterio("a&b&&c").3, criterio("a&b&&c").6), (conjunto(&["", "b", "c"]), 3));
        assert_eq!((criterio("a!b&").2, criterio("a!b&").3), (conjunto(&["b"]), conjunto(&[])));
        assert_eq!((criterio("a!&c").2, criterio("a!&c").3), (conjunto(&[]), conjunto(&["c"])));
        assert_eq!(criterio("a&c!v").3, conjunto(&["c!v"]));
        assert_eq!((criterio("a!v&c!w").2, criterio("a!v&c!w").3), (conjunto(&["v"]), conjunto(&["c!w"])));
        assert_eq!(criterio("a!v1!v1").2, conjunto(&["v1"]));
        assert_eq!(criterio("a!b%3").2, conjunto(&["b%3"]));
        assert_eq!(criterio("a%3!b").1, h("a%3"));
        assert_eq!(criterio("ç!ã&é"), (h("ç!ã&é"), h("ç"), conjunto(&["ã"]), conjunto(&["é"]), true, true, 101));
        assert_eq!(dynref_extract_name_hash(""), 0);
        assert_eq!(dynref_extract_name_hash("a!b"), h("a!b"));
    }

    /// Peso sem estouro: com variante, qualquer número de condições continua acima de qualquer critério sem variante com menos de 100.
    #[test]
    fn peso_conta_variante_e_cada_condicao_sem_estouro() {
        let com = |n: usize, variante: bool| {
            let mut t = String::from(if variante { "x!v" } else { "x" });
            for i in 0..n {
                t.push_str(&format!("&c{i}"));
            }
            let r = DynamicAppearanceRef::parse(&t);
            (r.weight, r.is_conditional)
        };
        assert_eq!(com(26, false), (26, true));
        assert_eq!(com(27, true), (127, true));
        assert_eq!(com(28, true), (128, true));
        assert_eq!(com(128, false), (128, true));
        assert_eq!(com(156, true), (256, true));
        assert_eq!(com(256, true), (356, true));
    }

    #[test]
    fn criterio_casa_variante_e_condicoes_com_overrides() {
        let r = DynamicAppearanceRef::parse("torso!v1&c1&c2");
        assert!(r.matches_variant(h("v1")));
        assert!(!r.matches_variant(h("v2")) && !r.matches_variant(0) && !r.matches_variant(h("torso")));
        assert!(!r.matches_conditions_with_overrides(&conjunto(&["c1"]), &conjunto(&[])));
        assert!(r.matches_conditions_with_overrides(&conjunto(&["c1"]), &conjunto(&["c2"])));
        assert!(r.matches_conditions_with_overrides(&conjunto(&[]), &conjunto(&["c1", "c2"])));
        assert!(DynamicAppearanceRef::parse("torso!v1").matches_conditions_with_overrides(&conjunto(&[]), &conjunto(&[])));
    }

    fn partes(pares: &[(&str, &str)]) -> BTreeMap<u64, u64> {
        pares.iter().map(|(k, v)| (h(k), if v.is_empty() { 0 } else { h(v) })).collect()
    }

    #[test]
    fn nome_le_variante_partes_overrides_e_contexto() {
        let n = DynamicAppearanceName::parse("torso!v1+color=red+size=xl");
        assert_eq!((n.name, n.variant, n.is_dynamic), (h("torso"), h("v1+color=red+size=xl"), true));
        assert_eq!(n.parts, partes(&[("color", "red"), ("size", "xl"), ("variant", "v1+color=red+size=xl"), ("variant.1", "v1")]));
        assert_eq!(n.overrides, conjunto(&["colo", "siz"]));
        let n = DynamicAppearanceName::parse("casaco!azul+gola=alta+curta&gender=w%123");
        assert_eq!((n.variant, n.context), (h("azul+gola=alta+curta"), 123));
        assert_eq!(n.parts, partes(&[("gola", "alta"), ("variant", "azul+gola=alta+curta"), ("variant.1", "azul"), ("variant.2", "curta")]));
        assert_eq!(n.overrides, conjunto(&["gola="]));
        let n = DynamicAppearanceName::parse("torso!azul++x=");
        assert_eq!(n.parts, partes(&[("variant", "azul++x="), ("variant.1", "azul"), ("x", "")]));
        assert_eq!(n.overrides, conjunto(&["x"]));
        let n = DynamicAppearanceName::parse("a!=v");
        assert_eq!((n.parts.get(&0).copied(), n.overrides.clone()), (Some(h("v")), conjunto(&["=v"])));
        assert_eq!(DynamicAppearanceName::parse("a!k=v=w").overrides, conjunto(&["k=v="]));
        assert_eq!(DynamicAppearanceName::parse("a!+k=v").overrides, conjunto(&["k="]));
        assert_eq!(DynamicAppearanceName::parse("ç!ã+é=ü").overrides, conjunto(&["é="]));
        let doze = DynamicAppearanceName::parse("a!p0+p1+p2+p3+p4+p5+p6+p7+p8+p9+p10+p11");
        assert_eq!((doze.parts.get(&h("variant.:")).copied(), doze.parts.get(&h("variant.<")).copied()), (Some(h("p9")), Some(h("p11"))));
    }

    #[test]
    fn nome_corta_no_contexto_e_na_condicao_e_nao_entra_em_panico() {
        let ctx = |t: &str| DynamicAppearanceName::parse(t).context;
        assert_eq!(ctx("torso!v1&cond1%42"), 42);
        assert_eq!(ctx("torso!v1%42&cond1"), 0);
        assert_eq!((ctx("a!b%abc"), ctx("a!b%12x"), ctx("a!b%-1"), ctx("a!b%18446744073709551616"), ctx("a!b%")), (0, 0, 0, 0, 0));
        assert_eq!(ctx("a!b%18446744073709551615"), u64::MAX);
        let n = DynamicAppearanceName::parse("a!b%3%4");
        assert_eq!((n.variant, n.context), (h("b%3"), 4));
        assert_eq!(DynamicAppearanceName::parse("a!b&c&d").variant, h("b"));
        assert_eq!(DynamicAppearanceName::parse("a!b!c").variant, h("b!c"));
        for t in ["%1&c!v1", "a&b!c", "a%5!b", "torso", ""] {
            let n = DynamicAppearanceName::parse(t);
            assert_eq!((n.name, n.is_dynamic, n.variant), (n.value, false, 0), "{t}");
        }
        let vazio = DynamicAppearanceName::parse("torso!");
        assert_eq!((vazio.is_dynamic, vazio.variant, vazio.parts.len()), (true, 0, 0));
        assert_eq!(DynamicAppearanceName::parse("!v").name, 0);
    }

    #[test]
    fn criterio_casa_nome_pela_variante_e_pelas_condicoes() {
        let casa = |r: &str, a: &str, c: Option<&[&str]>| {
            let condicoes = c.map(conjunto);
            appearance_matches_reference_com(&DynamicAppearanceRef::parse(r), &DynamicAppearanceName::parse(a), condicoes.as_ref())
        };
        assert!(casa("torso!v1!v2", "torso!v1", None));
        assert!(!casa("torso!v1!v2", "torso!v9", None) && !casa("torso!v1!v2", "torso", None));
        assert!(!casa("torso!v1&c1", "torso!v1", None));
        assert!(casa("torso!v1&c1", "torso!v1", Some(&["c1"])));
        assert!(!casa("torso!v1&c1&c2", "torso!v1", Some(&["c1"])));
        assert!(casa("torso&gender=w", "outro!qualquer", Some(&["gender=w"])));
        assert!(!casa("torso&gender=w", "outro!qualquer", Some(&["gender=m"])));
        assert!(casa("torso", "x!y", None));
        for r in ["a!v1&cor", "a!v1&cor=", "a!v1&cor=red"] {
            assert!(!casa(r, "b!v1+cor=red", Some(&[])), "{r}");
        }
        assert!(appearance_matches_reference(&DynamicAppearanceRef::parse("torso"), &DynamicAppearanceName::parse("x!y")));
    }

    fn texto(attr: &str, valor: &str, entrada: &str) -> (bool, bool, bool, String) {
        let t = process_dynamic_string_single_attr(attr, valor, entrada);
        (t.valid, t.missed, t.optional, t.value)
    }

    #[test]
    fn texto_troca_atributos_marca_falta_e_opcional() {
        assert_eq!(texto("gender", "fem", "*t0_{gender}_body"), (true, false, false, "t0_fem_body".into()));
        assert_eq!(texto("gender", "fem", "*t0_{outroattr}_body"), (true, true, false, "t0__body".into()));
        assert_eq!(texto("gender", "fem", "*sem_marcador"), (false, false, false, String::new()));
        assert_eq!(texto("gender", "fem", "*t0_{gender_sem_fechar"), (false, false, false, String::new()));
        assert_eq!(texto("gender", "fem", "*t0_{gender}?"), (true, false, true, "t0_fem".into()));
        assert_eq!(texto("gender", "fem", ""), (false, false, false, String::new()));
        assert_eq!(texto("x", "1", "{x}"), (true, false, false, "1".into()));
        assert_eq!(texto("x", "1", "**{x}"), (true, false, false, "*1".into()));
        assert_eq!(texto("", "", "*a_{x}"), (true, true, false, "a_".into()));
        assert_eq!(texto("x", "", "*a_{x}_b"), (true, false, false, "a__b".into()));
        assert_eq!(texto("x", "1", "*{x}{x}{x}"), (true, false, false, "111".into()));
        assert_eq!(texto("x", "1", "*a{{x}}b"), (true, true, false, "a}b".into()));
        assert_eq!(texto("x", "1", "*a}{x}"), (true, false, false, "a}1".into()));
        assert_eq!(texto("x", "1", "*a{x}b}c"), (true, false, false, "a1b}c".into()));
        assert_eq!(texto("X", "1", "*{x}"), (true, true, false, String::new()));
        assert_eq!(texto("x", "1", "*fim?{x}?"), (true, false, true, "fim?1".into()));
        assert_eq!(process_dynamic_string_single_attr("x", "1", "*{}").atributos, BTreeSet::from([bwms_primitives::FNV1A64_OFFSET]));
        assert_eq!(process_dynamic_string_single_attr("x", "1", "*a{{x}}b").atributos, conjunto(&["{x"]));
    }

    /// Texto literal que passa do teto invalida; valor de atributo que estoura no fim é truncado.
    #[test]
    fn texto_no_teto_de_512_bytes() {
        let com = |antes: &str, valor: &str, depois: &str| {
            let t = process_dynamic_string_single_attr("x", valor, &format!("*{antes}{{x}}{depois}"));
            (t.valid, t.value.len(), t.value.get(t.value.len().saturating_sub(4)..).unwrap_or("").to_string())
        };
        assert_eq!(com(&"a".repeat(600), "1", ""), (false, 0, String::new()));
        assert_eq!(com(&"a".repeat(511), "1", ""), (true, 512, "aaa1".into()));
        assert_eq!(com(&"a".repeat(512), "1", ""), (false, 0, String::new()));
        assert_eq!(com("", "1", &"b".repeat(600)), (false, 0, String::new()));
        assert_eq!(com(&"a".repeat(510), "zz", ""), (true, 512, "aazz".into()));
        assert_eq!(com("", &"v".repeat(600), ""), (true, 512, "vvvv".into()));
    }

    #[test]
    fn texto_com_estado_prefere_local_e_resolve_nome() {
        let globais: HashMap<u64, String> = [(h("gender"), "w".into()), (h("camera"), "fpp".into()), (h("skin_color"), String::new())].into();
        let locais: HashMap<u64, String> = [(h("variant"), "azul".into()), (h("gender"), "LOCAL".into())].into();
        let t = processar_texto_dinamico(&globais, &locais, "*{gender}_{camera}");
        assert_eq!((t.valid, t.missed, t.value.as_str()), (true, false, "LOCAL_fpp"));
        let t = processar_texto_dinamico(&globais, &locais, "*{skin_color}_x");
        assert_eq!((t.missed, t.value.as_str()), (false, "_x"));
        let t = processar_texto_dinamico(&globais, &locais, "*x_{camera}_{nada}?");
        assert_eq!((t.valid, t.missed, t.optional, t.value.as_str()), (true, true, true, "x_fpp_"));
        assert!(!processar_texto_dinamico(&globais, &locais, "plano").valid);
        let g: HashMap<u64, String> = [(h("gender"), "w".into())].into();
        let l: HashMap<u64, String> = [(h("variant"), "azul".into())].into();
        let r = |n: &str| resolver_nome_dinamico(&g, &l, n);
        assert_eq!((r("plano"), r("*{gender}_{variant}"), r("*x_{nada}?"), r("*x_{nada}")), ("plano".into(), "w_azul".into(), String::new(), "x_".into()));
        assert_eq!((r("*sem_chaves"), r("*aberto_{gender"), r("*{gender}?"), r(""), r("*")), ("*sem_chaves".into(), "*aberto_{gender".into(), "w".into(), String::new(), "*".into()));
    }

    #[test]
    fn nome_base_unico_dinamico_e_condicao_do_estado() {
        for (t, base) in [("torso!v1", "torso"), ("torso%42", "torso"), ("torso&cond", "torso"), ("torso", "torso"), ("", ""), ("!x", ""), ("a&b!c", "a"), ("a%1!b", "a"), ("ç!ã", "ç")] {
            assert_eq!(get_base_appearance_name(t), base, "{t}");
        }
        for t in ["default", "random", "empty_appearance_default"] {
            assert!(!is_unique_appearance_name(h(t)), "{t}");
        }
        assert!(!is_unique_appearance_name(0));
        assert!(is_unique_appearance_name(h("Default")) && is_unique_appearance_name(h("None")) && is_unique_appearance_name(h("t0_000_fem_body")));
        assert!(is_dynamic_value("*{attr}_x") && is_dynamic_value("*"));
        assert!(!is_dynamic_value("valor_fixo") && !is_dynamic_value("") && !is_dynamic_value(" *x") && !is_dynamic_value("x*"));
        assert_eq!(condicao_do_estado("gender", "w"), 0xa141_a980_10ec_99b0);
        assert_eq!(condicao_do_estado("skin_color", "01_ca_pale"), h("skin_color=01_ca_pale"));
        assert_eq!((condicao_do_estado("", ""), condicao_do_estado("a", "")), (h("="), h("a=")));
    }

    /// O snake case que o estado aplica nos sufixos (`texto_e_numeros::para_snake_case`).
    #[test]
    fn snake_case_dos_sufixos_do_estado() {
        for (t, s) in [
            ("FPP", "fpp"),
            ("BaseBody", "base_body"),
            ("foo_Bar", "foo_bar"),
            ("v2Model", "v2_model"),
            ("çãoFoo", "ção_foo"),
            ("ãB", "ã_b"),
            ("", ""),
            ("aB_C", "a_b_c"),
            ("_A", "_a"),
            ("A_", "a_"),
            ("__X", "__x"),
            ("XMLHttp", "xmlhttp"),
            ("a b C", "a b _c"),
        ] {
            assert_eq!(para_snake_case(t), s, "{t}");
        }
    }

    #[test]
    fn criterio_vivo_le_o_texto_internado() {
        assert_eq!(unsafe { parse_reference_live(0) }, DynamicAppearanceRef::default());
        let nome = crate::cname::intern("bwms_b4_torso!v1&c1");
        assert_eq!(unsafe { parse_reference_live(nome) }, DynamicAppearanceRef::parse("bwms_b4_torso!v1&c1"));
    }
}

/// Loja de override por componente, tags, prefixo, condicionais, offsets de roupa e override de aparência de customização.
/// Cada teste usa entidades próprias: as lojas são do processo.
#[cfg(test)]
mod testes_da_loja_de_override {
    use super::*;
    use bwms_primitives::override_state::ComponentState;

    fn h(s: &str) -> u64 {
        bwms_primitives::fnv1a64(s.as_bytes())
    }

    /// Componente em memória: nome `+0x40`, `appearanceName` `+0x48`, `appearancePath` `+0x68`, máscara de teste `+0x80`, `isEnabled` `+0x8b`.
    fn componente(nome: &str, mascara: u64) -> *mut u8 {
        let c: &'static mut [u64; 32] = Box::leak(Box::new([0u64; 32]));
        c[0x40 / 8] = crate::cname::intern(nome);
        c[0x80 / 8] = mascara;
        let p = c.as_mut_ptr() as *mut u8;
        unsafe { p.add(0x8b).write(7) };
        p
    }

    /// Entidade com a lista de componentes em `+0xA0`.
    fn entidade(componentes: &[*mut u8]) -> *mut c_void {
        let handles: &'static mut Vec<[u64; 2]> = Box::leak(Box::new(componentes.iter().map(|c| [*c as u64, 0]).collect()));
        let e: &'static mut [u64; 32] = Box::leak(Box::new([0u64; 32]));
        e[0xA0 / 8] = if handles.is_empty() { 0 } else { handles.as_mut_ptr() as u64 };
        e[0xA8 / 8] = (handles.len() as u64) | ((handles.len() as u64) << 32);
        e.as_mut_ptr() as *mut c_void
    }

    unsafe fn mascara(c: *mut u8) -> u64 {
        (c.add(0x80) as *const u64).read_unaligned()
    }

    /// A aplicação dos testes: a decisão da loja gravada na máscara de teste, com a original do cache.
    unsafe fn aplicar(e: u64, c: *mut c_void) -> bool {
        let id = es20_id_do_componente((0x48, 0x68), c as *const u8);
        let alvo = (c as *mut u8).add(0x80) as *mut u64;
        match es20_decidir_mascara(e, c as *const u8, || es20_original(e, id, 0, || alvo.read_unaligned())) {
            Some(m) => {
                alvo.write_unaligned(m);
                true
            }
            None => false,
        }
    }

    #[test]
    fn tabela_tem_as_14_tags_com_as_chaves_de_cada_uma() {
        let contagem: Vec<(u64, usize)> = TAGS_EMBUTIDAS.iter().map(|(t, e)| (*t, e.len())).collect();
        let esperado = [
            ("hide_Head", 12),
            ("hide_Arms", 3),
            ("hide_Torso", 8),
            ("hide_LowerAbdomen", 3),
            ("hide_UpperAbdomen", 3),
            ("hide_CollarBone", 3),
            ("hide_Chest", 4),
            ("hide_Legs", 5),
            ("hide_Thighs", 3),
            ("hide_Calves", 8),
            ("hide_Ankles", 9),
            ("hide_Feet", 8),
            ("HighHeels", 5),
            ("FlatShoes", 5),
        ];
        assert_eq!(contagem, esperado.map(|(t, n)| (h(t), n)).to_vec());
        for (t, _) in esperado {
            assert!(tag_por_hash(h(t)).is_some(), "{t}");
        }
        assert!(tag_por_hash(h("tag_que_nao_existe")).is_none() && tag_por_hash(0).is_none() && tag_por_hash(h("hide_torso")).is_none());
        let efeito = |tag: &str, chave: &str| tag_por_hash(h(tag)).and_then(|t| t.iter().find(|(c, _, _)| c.valor() == h(chave)).map(|&(_, m, x)| (m, x)));
        assert_eq!(efeito("hide_Head", "beard_"), Some((false, 0)), "esconder tudo = máscara 0");
        assert_eq!(efeito("hide_Torso", "n0_"), Some((false, 0)));
        assert_eq!(efeito("hide_Torso", "t0_000_pma_base__full"), Some((false, !0b1111)));
        assert_eq!(efeito("hide_Chest", "t0_000_pwa_fpp__torso"), Some((false, !0b1)));
        assert_eq!(efeito("hide_Thighs", "t0_000_pwa_base__full"), Some((false, !0b1_0000)));
        assert_eq!(efeito("HighHeels", "l0_000_pwa_base__high_heels"), Some((true, 0b111)));
        assert_eq!(efeito("FlatShoes", "t0_000_pma_base__full"), Some((false, !0b1110_0000)));
        assert_eq!(efeito("hide_Feet", "l0_000_pwa_base__cs_flat"), Some((false, !0b100)));
        assert_eq!(efeito("hide_Ankles", "t0_000_pma_base__full"), Some((false, !0b100_0000)));
        assert_eq!(efeito("hide_Ankles", "s0_"), Some((false, 0)));
    }

    #[test]
    fn fonte_da_tag_e_o_texto_com_semente_do_mod() {
        assert_eq!(fonte_da_tag("HighHeels", 0), 0x4516_9748_795a_cdd3);
        assert_eq!(fonte_da_tag("HighHeels", 0xa), 0xc165_675f_06b3_88f9);
        assert_eq!(fonte_da_tag("hide_Legs", 0x5a5a_5a5a_5a5a_5a5a), 0x3dc5_60d5_f3e6_860e);
        assert_eq!(fonte_da_tag("hide_Torso", 0x5a5a_5a5a_5a5a_5a5a), 0x4bfc_0df6_1293_a04a);
        assert_ne!(fonte_da_tag("HighHeels", 0), h("HighHeels"));
    }

    #[test]
    fn prefixo_e_o_trecho_ate_o_sublinhado_nas_posicoes_2_a_5() {
        for (nome, prefixo) in [
            ("n0_something", Some("n0_")),
            ("abcde_resto", Some("abcde_")),
            ("abcdef_resto", None),
            ("abcdefgh", None),
            ("ab", None),
            ("a_bcdefgh", None),
            ("_abc_x", Some("_abc_")),
            ("ab_", Some("ab_")),
            ("heb_000_pma__basehead", Some("heb_")),
            ("beard_x", Some("beard_")),
            ("beard", None),
            ("MorphTargetSkinnedMesh3637", None),
            ("", None),
            ("abc", None),
            ("ab__cd", Some("ab_")),
            ("éa_b", Some("éa_")),
        ] {
            assert_eq!(component_prefix_of(nome), prefixo.map(h), "{nome}");
        }
        assert_eq!(component_prefix_of("n0_something"), Some(0x21d1_ed19_25f3_ecc4));
    }

    #[test]
    fn componentes_da_tag_casam_por_nome_base_ou_prefixo() {
        let nomes = ["n0_000_pma_fpp__neck2814", "x9_nada", "t0_000_pma_base__full", "t0_000_pma_base__full!pele", "l0_000_pma_base__high_heels"];
        let comps: Vec<*mut u8> = nomes.iter().map(|n| componente(n, 0)).collect();
        let e = entidade(&comps);
        let indices = |tag: &str| {
            unsafe { componentes_da_tag(e, tag_por_hash(h(tag)).unwrap_or(&[])) }
                .map(|(v, b)| (v, b.iter().map(|p| comps.iter().position(|c| *c as *mut c_void == *p).unwrap_or(99)).collect::<Vec<_>>()))
        };
        assert_eq!(indices("hide_Torso"), Some((5, vec![0, 2, 3])));
        for tag in ["hide_Legs", "hide_Calves", "HighHeels"] {
            assert_eq!(indices(tag), Some((5, vec![2, 3, 4])), "{tag}");
        }
        assert_eq!(indices("hide_Head"), Some((5, vec![])));
        assert!(unsafe { componentes_da_tag(std::ptr::null_mut(), &[]) }.is_none());
        assert!(unsafe { componentes_da_tag(entidade(&[]), &[]) }.is_none());
        let incoerente = entidade(&comps);
        unsafe { (incoerente as *mut u8).add(0xA8).cast::<u32>().write_unaligned(2) };
        let sem_nada = |_: *mut c_void| true;
        assert!(unsafe { tag_aplicar_na_loja(incoerente, "hide_Torso", 0xB4A2_0701, sem_nada) }.is_none());
        assert!(es20_estado(incoerente as u64, h("n0_")).is_none(), "lista incoerente não entra na loja");
        assert!(unsafe { tag_aplicar_na_loja(e, "tag_que_nao_existe", 0, sem_nada) }.is_none());
    }

    /// V08: remover tira só a fonte (tag, mod) e só nas chaves da tag; o torso volta à parte de `hide_Torso`.
    #[test]
    fn tag_aplicada_e_removida_so_pela_propria_fonte() {
        let (torso, pescoco, salto) = (componente("t0_000_pma_base__full", !0), componente("n0_000_pma_fpp__neck2814", !0), componente("l0_000_pma_base__high_heels", !0));
        let e = entidade(&[torso, pescoco, salto]);
        let ent = e as u64;
        let ap = |inst: *mut c_void| unsafe { aplicar(ent, inst) };
        const M: u64 = 0xB4A2_0801;
        let fora = CName::cname("bwms_fora_da_tag");
        crate::cname::intern("bwms_fora_da_tag");
        es20_adicionar_mascara(ent, fonte_da_tag("hide_Legs", M), fora, !1, false);
        assert!(es20_adicionar_aparencia(ent, 0x17E4, CName::cname("t0_000_pma_base__full"), CName::cname("bwms_item_aparencia")));
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "hide_Torso", M, ap) }, Some((3, 2, 2)));
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "hide_Legs", M, ap) }, Some((3, 2, 2)));
        assert_eq!(unsafe { (mascara(torso), mascara(pescoco), mascara(salto)) }, (0xffff_ffff_ffff_ff00, 0, 0));
        assert_eq!(unsafe { tag_remover_da_loja(e, "hide_Legs", M, ap) }, Some((5, 5, 2)));
        assert_eq!(unsafe { (mascara(torso), mascara(pescoco), mascara(salto)) }, (0xffff_ffff_ffff_fff0, 0, !0));
        let l0 = es20_estado(ent, h("l0_")).map(|s| (s.fontes(), s.muda_mascara()));
        assert_eq!(l0, Some((0, true)));
        assert_eq!(es20_estado(ent, fora.valor()).map(|s| s.fontes()), Some(1), "a mesma fonte fora da tag fica");
        assert!(es20_estado(ent, h("t0_000_pma_base__full")).and_then(|s| s.aparencia_sobrescrita()).is_some());
        assert_eq!(unsafe { tag_remover_da_loja(e, "hide_Legs", M, ap) }, Some((5, 0, 0)));
        unsafe { tag_remover_da_loja(e, "hide_Torso", M, ap) };
        assert_eq!(unsafe { (mascara(torso), mascara(pescoco), mascara(salto)) }, (!0, !0, !0));
        assert_eq!(unsafe { tag_remover_da_loja(entidade(&[torso]), "hide_Legs", M, |_| true) }, Some((0, 0, 0)));
    }

    /// V08 com a tabela real: `HighHeels` mostra os chunks 0-2 do salto e esconde 5-7 do torso; `hide_Calves` esconde o chunk 0 do salto, e
    /// esconder vence mostrar no mesmo chunk em qualquer ordem das fontes.
    #[test]
    fn esconder_de_uma_tag_vence_o_mostrar_de_outra_em_qualquer_ordem() {
        const MOD_A: u64 = 0xB4A2_0811;
        const MOD_B: u64 = 0xB4A2_0812;
        let montar = || {
            let comps = ["l0_000_pma_base__high_heels", "t0_000_pma_base__full", "n0_000_pma_fpp__neck2814", "x9_nada"].map(|n| componente(n, 0xF1));
            (entidade(&comps), comps)
        };
        let mascaras = |c: [*mut u8; 4]| unsafe { c.map(|x| mascara(x)) };

        let (e, comps) = montar();
        let ap = |inst: *mut c_void| unsafe { aplicar(e as u64, inst) };
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "HighHeels", MOD_A, ap) }, Some((4, 2, 2)));
        assert_eq!(mascaras(comps), [0xF7, 0x11, 0xF1, 0xF1]);
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "hide_Calves", MOD_B, ap) }, Some((4, 2, 2)));
        assert_eq!(mascaras(comps), [0xF6, 0x11, 0xF1, 0xF1]);

        let (e, comps) = montar();
        let ap = |inst: *mut c_void| unsafe { aplicar(e as u64, inst) };
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "hide_Calves", MOD_B, ap) }, Some((4, 2, 2)));
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "HighHeels", 0, ap) }, Some((4, 2, 2)));
        assert_eq!(mascaras(comps), [0xF6, 0x11, 0xF1, 0xF1], "ordem inversa");
        assert_eq!(unsafe { tag_aplicar_na_loja(e, "HighHeels", 0, ap) }, Some((4, 2, 2)));
        assert_eq!(mascaras(comps), [0xF6, 0x11, 0xF1, 0xF1], "reaplicar a mesma fonte é idempotente");
    }

    #[test]
    fn loja_por_entidade_e_nome() {
        const E: u64 = 0xB4A2_0901;
        let nome = CName::cname("bwms_b4a2_v09");
        assert!(es20_estado(E, nome.valor()).is_none());
        assert!(es20_adicionar_aparencia(E, 0xAA, nome, CName::lido_do_motor(0x111)));
        assert!(es20_adicionar_aparencia(E, 0xAA, nome, CName::lido_do_motor(0x222)));
        assert_eq!(es20_estado(E, nome.valor()).and_then(|s| s.aparencia_sobrescrita()), Some(0x222));
        assert!(!es20_adicionar_aparencia(E, 0xAA, nome, CName::lido_do_motor(0)));
        assert!(!es20_adicionar_aparencia(E, 0xAA, nome, CName::lido_do_motor(CNAME_DEFAULT)));
        assert!(!es20_adicionar_aparencia(E, 0xAA, CName::lido_do_motor(0), CName::lido_do_motor(0x111)));
        es20_adicionar_mascara(E, 0xAA, CName::lido_do_motor(0), !1, false);
        assert!(es20_estado(E, 0).is_none());
        es20_adicionar_mascara(E, 0xAA, nome, !1, false);
        es20_adicionar_mascara(E, 0xBB, nome, 4, true);
        let s = es20_estado(E, nome.valor()).unwrap_or_default();
        assert_eq!((s.fontes(), s.mascara_de_mostrar(), s.mascara_de_esconder()), (2, 4, !1));
        let bracos = tag_por_hash(h("hide_Arms")).unwrap_or(&[]);
        es20_registrar_tag(E, 0xCC, bracos);
        assert!(bracos.iter().all(|(c, _, _)| es20_estado(E, c.valor()).map(|s| s.fontes()) == Some(1)));
        assert_eq!(es20_remover_mascara(E, 0xCC, bracos), (3, 3));
        assert_eq!(es20_remover_mascara(E, 0xCC, bracos), (3, 0));
        assert_eq!(es20_remover_mascara(E, 0xCC, tag_por_hash(h("hide_Torso")).unwrap_or(&[])), (0, 0));
        es20_adicionar_aparencia(E, 0xAA, CName::cname("bwms_b4a2_v09_b"), CName::lido_do_motor(0x333));
        assert_eq!(es20_remover_componentes(E, 0xAA), 2);
        let s = es20_estado(E, nome.valor()).unwrap_or_default();
        assert_eq!((s.fontes(), s.aparencia_sobrescrita(), s.muda_mascara(), s.muda_aparencia()), (1, None, true, true));
        assert_eq!(es20_remover_componentes(E, 0xAA), 0);
        assert_eq!(es20_remover_componentes(E + 1, 0xBB), 0);
        assert!(es20_estado(E + 1, nome.valor()).is_none());
        es20_esquecer(E);
        assert!(es20_estado(E, nome.valor()).is_none());
    }

    #[test]
    fn ligacoes_componente_parte_e_aparencia() {
        const E: u64 = 0xB4A2_1001;
        let base = h("bwms_b4a2_base");
        assert_eq!(es22_parte_do_componente(E, base), None);
        assert!(es22_ligar_componente(E, base, 0x10));
        assert!(es22_ligar_componente(E, base, 0x20));
        assert_eq!(es22_parte_do_componente(E, base), Some(0x20));
        assert_eq!(es22_parte_do_componente(E + 1, base), None);
        es22_ligar_parte(E, 0x20, "bwms_apar!azul");
        let ativa = |p: u64| ES22_RESOURCE_STATE.lock().ok().and_then(|g| g.iter().find(|(k, _)| *k == (E, p)).map(|(_, s)| s.value));
        assert_eq!(ativa(0x20), Some(0xc369_0441_e1ae_f8ca));
        es22_ligar_parte(E, 0x20, "bwms_apar!verde");
        assert_eq!(ativa(0x20), Some(h("bwms_apar!verde")));
        assert_eq!(ativa(0x99), None);
        let c = componente("bwms_b4a2_comp!var&cond", 0);
        assert_eq!(unsafe { es20_nome_base(c) }, crate::cname::cname("bwms_b4a2_comp"));
        let simples = componente("bwms_b4a2_simples", 0);
        assert_eq!(unsafe { es20_nome_base(simples) }, h("bwms_b4a2_simples"));
        assert_eq!(unsafe { es20_nome_base(componente("", 0)) }, 0);
        assert_eq!(unsafe { es20_ligar_componente(E, c, 0x40) }, h("bwms_b4a2_comp"));
        assert_eq!(es22_parte_do_componente(E, h("bwms_b4a2_comp")), Some(0x40));
        assert_eq!(es20_nomes_da_parte(E, 0x40), vec![h("bwms_b4a2_comp!var&cond")]);
        unsafe {
            es20_ligar_componente(E, simples, 0x40);
            es20_ligar_componente(E, c, 0x40);
        }
        assert_eq!(es20_nomes_da_parte(E, 0x40).len(), 2);
        assert!(es20_nomes_da_parte(E, 0x41).is_empty());
        assert!(es20_estado(E, h("bwms_b4a2_comp")).is_none(), "ligar não cria estado na loja");
        crate::diagnostico::es20_restaurar_parte(E, base, Some(0x10));
        assert_eq!(es22_parte_do_componente(E, base), Some(0x10));
        crate::diagnostico::es20_restaurar_parte(E, base, None);
        assert_eq!(es22_parte_do_componente(E, base), None);
    }

    #[test]
    fn identidade_e_original_do_componente() {
        let c: &'static mut [u64; 32] = Box::leak(Box::new([0u64; 32]));
        (c[0x40 / 8], c[0x48 / 8], c[0x68 / 8]) = (0x1111, 0x2222, 0x3333);
        let p = c.as_ptr() as *const u8;
        let id = unsafe { es20_id_do_componente((0x48, 0x68), p) };
        assert_eq!(id, 0xb807_f11d_8957_3e8f);
        c[0x68 / 8] = 0x3334;
        assert_ne!(unsafe { es20_id_do_componente((0x48, 0x68), p) }, id, "caminho diferente, identidade diferente");
        const E: u64 = 0xB4A2_1101;
        assert_eq!(es20_original(E, id, 0, || 0x55), 0x55);
        assert_eq!(es20_original(E, id, 0, || 0x99), 0x55);
        assert_eq!(es20_original(E, id, 1, || 0x77), 0x77);
        assert_eq!(es20_original(E + 1, id, 0, || 0x66), 0x66);
        // Leitura que falha (componente sem vtable): 0 e nada no cache.
        let sem_vtable = componente("bwms_b4a2_sem_vtable", 0);
        let id2 = unsafe { es20_id_do_componente((0x48, 0x68), sem_vtable) };
        assert_eq!(unsafe { es20_original_da_aparencia((0x48, 0x68), E, sem_vtable as *mut c_void, GarmentComponentKind::MeshComponent) }, 0);
        assert!(es20_originais().lock().map(|m| !m.contains_key(&(E, id2, 1))).unwrap_or(false));
    }

    #[test]
    fn desempate_de_condicionais_deixa_o_primeiro_de_maior_peso() {
        let d = |e: &[(u64, i32, bool)]| {
            let mut v = toggle_conditional_group_disable_indices(e);
            v.sort_unstable();
            v
        };
        assert!(d(&[(1, 0, true), (2, 0, true)]).is_empty());
        assert!(d(&[(1, 100, false), (1, 100, false)]).is_empty());
        assert!(d(&[(1, 102, true)]).is_empty());
        assert_eq!(d(&[(7, 100, true), (7, 102, true)]), [0]);
        assert_eq!(d(&[(7, 102, true), (7, 100, true)]), [1]);
        assert_eq!(d(&[(9, 100, true), (9, 100, true), (9, 100, true)]), [1, 2]);
        assert_eq!(d(&[(9, 101, true), (9, 100, true), (9, 101, true)]), [1, 2]);
        assert_eq!(d(&[(1, 100, true), (1, 102, true), (2, 50, true), (2, 51, true)]), [0, 2]);
        assert_eq!(d(&[(3, 0, true), (3, 101, true)]), [0]);
        assert!(d(&[(3, 0, true), (3, 0, true), (3, 0, true)]).is_empty());
        assert!(d(&[(4, -5, true), (4, -3, true)]).is_empty());
        assert_eq!(d(&[(5, 101, true), (5, 101, false), (5, 100, true)]), [2]);
        assert!(d(&[]).is_empty());
    }

    #[test]
    fn condicionais_seguem_a_aparencia_da_parte_e_as_condicoes() {
        let nomes = ["cap!azul", "cap!verde", "cap!azul&male", "cap", "luva&male", "semparte!azul"];
        let comps: Vec<*mut u8> = nomes.iter().map(|n| componente(n, 0)).collect();
        let e = entidade(&comps);
        let ent = e as u64;
        for base in ["cap", "luva"] {
            es22_ligar_componente(ent, crate::cname::intern(base), 0x500);
        }
        es22_ligar_parte(ent, 0x500, "itemx!azul");
        let reg = std::mem::MaybeUninit::<crate::rtti::Registry>::uninit();
        let rodar = |condicoes: Option<&std::collections::BTreeSet<u64>>| {
            for c in &comps {
                unsafe { c.add(0x8b).write(7) };
            }
            let (_, total) = unsafe { run_toggle_conditional_components_com(&*reg.as_ptr(), e, condicoes) };
            (total, comps.iter().map(|c| unsafe { c.add(0x8b).read() }).collect::<Vec<_>>())
        };
        let vazio = std::collections::BTreeSet::new();
        assert_eq!(rodar(Some(&vazio)), (5, vec![1, 0, 0, 0, 0, 7]));
        let homem = std::collections::BTreeSet::from([h("male")]);
        assert_eq!(rodar(Some(&homem)), (5, vec![0, 0, 1, 0, 1, 7]));
        assert_eq!(rodar(None), (5, vec![1, 0, 0, 0, 0, 7]));
        es22_ligar_parte(ent, 0x500, "itemx!verde+male");
        assert_eq!(rodar(Some(&vazio)), (5, vec![0, 0, 0, 7, 0, 7]));
        assert_eq!(unsafe { run_toggle_conditional_components_com(&*reg.as_ptr(), std::ptr::null_mut(), None) }.1, -1);
        assert_eq!(unsafe { run_toggle_conditional_components_com(&*reg.as_ptr(), entidade(&[]), None) }.1, -1);
    }

    #[test]
    fn decisao_de_mascara_e_aparencia_pelo_nome_base_e_prefixo() {
        const E: u64 = 0xB4A2_1501;
        let c = componente("bwms_b4a2_x!var", 0);
        let lida = std::cell::Cell::new(false);
        let original = |v: u64| {
            let lida = &lida;
            move || {
                lida.set(true);
                v
            }
        };
        assert_eq!(unsafe { es20_decidir_mascara(E, c, original(0xF0)) }, None);
        assert!(!lida.get());
        es20_adicionar_mascara(E, 0xA, CName::cname("bwms_b4a2_x"), !0x10, false);
        es20_adicionar_mascara(E, 0xB, CName::cname("bwms_"), 0x1, true);
        assert_eq!(unsafe { es20_decidir_mascara(E, c, original(0xF0)) }, Some(0xE1));
        assert!(lida.get());
        let d = unsafe { es22_decidir_aparencia(E, c, || 0x999) };
        assert_eq!((d.fontes, d.aparencia, d.parte), ((None, None), None, None));
        es20_adicionar_aparencia(E, 0xC, CName::cname("bwms_"), CName::lido_do_motor(0x444));
        let d = unsafe { es22_decidir_aparencia(E, c, || 0x999) };
        assert_eq!((d.fontes, d.aparencia), ((None, Some(0x444)), Some(0x444)));
        es20_adicionar_aparencia(E, 0xBB, CName::cname("bwms_b4a2_x"), CName::lido_do_motor(0x222));
        es20_adicionar_aparencia(E, 0xAA, CName::cname("bwms_b4a2_x"), CName::lido_do_motor(0x111));
        es22_ligar_componente(E, h("bwms_b4a2_x"), 0x30);
        let d = unsafe { es22_decidir_aparencia(E, c, || 0x999) };
        assert_eq!((d.fontes, d.aparencia, d.parte), ((Some(0x111), Some(0x444)), Some(0x111), Some(0x30)));
        let pescoco = crate::cname::intern("n0_000_pma_fpp__neck2814");
        assert_eq!(unsafe { chaves_do_componente(pescoco) }, (pescoco, Some(h("n0_"))));
        assert_eq!(unsafe { chaves_do_componente(crate::cname::intern("bwms_b4a2_x!var")) }, (h("bwms_b4a2_x"), Some(h("bwms_"))));
    }

    /// A aparência que veio da native e a do registro de item caem na MESMA loja: menor hash vence nas duas ordens.
    #[test]
    fn aparencia_de_duas_vias_na_mesma_loja_vence_o_menor_hash() {
        for (e, ordem) in [(0xB4A2_1502u64, [(0xAAu64, 0x111u64), (0xBB, 0x222)]), (0xB4A2_1503, [(0xBB, 0x222), (0xAA, 0x111)])] {
            let c = componente("bwms_b4a2_duas_vias", 0);
            for (fonte, ap) in ordem {
                es20_adicionar_aparencia(e, fonte, CName::cname("bwms_b4a2_duas_vias"), CName::lido_do_motor(ap));
            }
            assert_eq!(unsafe { es22_decidir_aparencia(e, c, || 0x999) }.aparencia, Some(0x111));
        }
    }

    #[test]
    fn guardas_de_tag_e_de_estado_devolvem_a_loja_de_antes() {
        const E: u64 = 0xB4A2_1201;
        let base = h("bwms_b4a2_guarda");
        crate::cname::intern("bwms_b4a2_guarda");
        es20_adicionar_mascara(E, 0x1, CName::lido_do_motor(base), !1, false);
        let antes = es20_estado(E, base);
        assert!(crate::diagnostico::es20_guardar_tags(E));
        es20_registrar_tag(E, 0x2, tag_por_hash(h("hide_Arms")).unwrap_or(&[]));
        es20_remover_mascara(E, 0x2, tag_por_hash(h("hide_Arms")).unwrap_or(&[]));
        es20_adicionar_mascara(E, 0x3, CName::lido_do_motor(base), !2, false);
        assert!(crate::diagnostico::es20_restaurar_tags());
        assert_eq!(es20_estado(E, base), antes);
        assert!(es20_estado(E, h("a0_")).is_none());
        assert!(!crate::diagnostico::es20_restaurar_tags(), "sem guarda");

        const PARTE: u64 = 0xB4A2_1230;
        assert!(crate::diagnostico::es22_guardar_estado(E, base, PARTE));
        es20_adicionar_aparencia(E, 0x4, CName::lido_do_motor(base), CName::lido_do_motor(0x444));
        es22_ligar_componente(E, base, PARTE);
        es22_ligar_parte(E, PARTE, "bwms_b4a2!teste");
        assert!(crate::diagnostico::es22_restaurar_estado());
        assert_eq!(es20_estado(E, base), antes);
        assert_eq!(es22_parte_do_componente(E, base), None);
        assert!(ES22_RESOURCE_STATE.lock().map(|g| !g.iter().any(|(k, _)| *k == (E, PARTE))).unwrap_or(false));
        let _: Option<ComponentState> = es20_estado(E, base);
    }

    #[test]
    fn offset_de_roupa_vence_o_primeiro_mod_que_entrou() {
        let mut s = Es67ResourceOffset::default();
        assert_eq!(s.get_overridden_offset(), 0);
        s.add_offset_override(0xAAAA, 42);
        s.add_offset_override(0xAAAA, 20);
        assert_eq!((s.get_overridden_offset(), s.overrides.len()), (20, 1));
        let mut s = Es67ResourceOffset::default();
        s.add_offset_override(0xBBBB, 5);
        s.add_offset_override(0xAAAA, 99);
        assert_eq!(s.get_overridden_offset(), 5);
        s.add_offset_override(0xBBBB, 7);
        assert_eq!(s.get_overridden_offset(), 7);
        assert!(s.remove_offset_override(0xBBBB));
        assert_eq!(s.get_overridden_offset(), 99);
        assert!(!s.remove_offset_override(0xBBBB));
        s.add_offset_override(0xBBBB, 8);
        assert_eq!(s.get_overridden_offset(), 99);
        s.add_offset_override(0xAAAA, -3);
        assert_eq!(s.get_overridden_offset(), -3);

        const E: u64 = 0xB4A2_1601;
        unsafe {
            assert_eq!(get_offset_override(E, 0x1111), 0);
            add_offset_override(E, 0xAAAA, 0x1111, 100);
            add_offset_override(E, 0xAAAA, 0x2222, 200);
            add_offset_override(E, 0xBBBB, 0x2222, 300);
            add_offset_override(E + 1, 0xAAAA, 0x1111, 999);
            assert_eq!((get_offset_override(E, 0x1111), get_offset_override(E, 0x2222), get_offset_override(E + 1, 0x1111)), (100, 200, 999));
            assert_eq!(remove_offset_overrides(E, 0xAAAA), 2);
            assert_eq!((get_offset_override(E, 0x1111), get_offset_override(E, 0x2222), get_offset_override(E + 1, 0x1111)), (0, 300, 999));
            assert_eq!(remove_offset_overrides(E, 0xAAAA), 0);
            assert_eq!(remove_offset_overrides(E + 7, 0xAAAA), 0);
        }
    }

    #[test]
    fn override_de_aparencia_de_customizacao_troca_so_o_recurso() {
        let tabela = |r: u64, a: u64| (r == 11 && a == 22).then_some(33);
        let troca = |d: [u64; 2]| {
            let mut d = d;
            (trocar_por_app_override(&mut d, tabela), d)
        };
        assert_eq!(troca([11, 22]), (true, [33, 22]));
        for d in [[0, 22], [11, 0], [11, 44], [0, 0]] {
            assert_eq!(troca(d), (false, d));
        }
        let mut d = [0, 22];
        assert!(!trocar_por_app_override(&mut d, |_, _| Some(33)), "recurso 0 nunca troca");
        assert_eq!(descritores_do_range(0x1000, 0x1030), Some(3));
        assert_eq!(descritores_do_range(0x1000, 0x1000), Some(0));
        assert_eq!(descritores_do_range(0x1000, 0x11000), Some(4096));
        for (i, f) in [(0, 0), (0x1000, 0x1008), (0x1010, 0x1000), (0x1000, 0x11010), (0, 0x10)] {
            assert_eq!(descritores_do_range(i, f), None, "{i:#x}..{f:#x}");
        }
        const R: u64 = 0xB4A2_17AA;
        registrar_app_override(R, 0xB4A2_17BB, 0xB4A2_17CC);
        let mut buf = [R, 0xB4A2_17CC, R, 0xB4A2_17DD, 0, 0xB4A2_17CC];
        assert_eq!(unsafe { aplicar_app_overrides(buf.as_mut_ptr(), 3) }, 1);
        assert_eq!(buf, [0xB4A2_17BB, 0xB4A2_17CC, R, 0xB4A2_17DD, 0, 0xB4A2_17CC]);
        registrar_app_override(R, 0xB4A2_17EE, 0xB4A2_17CC);
        let mut buf = [R, 0xB4A2_17CC];
        unsafe { aplicar_app_overrides(buf.as_mut_ptr(), 1) };
        assert_eq!(buf, [0xB4A2_17EE, 0xB4A2_17CC]);
        assert_eq!(unsafe { aplicar_app_overrides(std::ptr::null_mut(), 3) }, 0);
        assert!(remover_app_override(R, 0xB4A2_17CC));
        assert!(!remover_app_override(R, 0xB4A2_17CC));
    }
}

/// Peças puras do patch de recurso e do mesh.
#[cfg(test)]
mod testes_do_patch_de_recurso {
    use super::*;

    #[test]
    fn cabecalho_conta_objetos_inteiros_entre_inicio_e_fim() {
        let cab = |i: u64, f: u64| {
            let mut c = [0u8; 0x70];
            c[0x20..0x28].copy_from_slice(&i.to_le_bytes());
            c[0x28..0x30].copy_from_slice(&f.to_le_bytes());
            objetos_do_cabecalho(&c)
        };
        assert_eq!((cab(0, 0x40), cab(0x1000, 0x1047), cab(0x1000, 0x1008), cab(0x1000, 0x1000)), (0, 8, 1, 0));
    }

    #[test]
    fn definicao_do_patch_casos_de_borda() {
        assert_eq!(definicao_do_patch(&[], 7), None);
        assert_eq!(definicao_do_patch(&[7, 7], 7), Some(0));
        assert_eq!(definicao_do_patch(&[5, 6], 0), None);
        assert_eq!(definicao_do_patch(&[0, 0], 0), Some(0));
    }

    #[test]
    fn blob_muda_pela_prop_ou_so_no_alvo_sem_blob() {
        use crate::arquivos_do_jogo::{A17_PROP_APPEARANCES as AP, A17_PROP_BLOB as B, A17_PROP_RENDERBLOB as R, A17_PROP_TARGETS as T};
        for (props, patch, alvo, render, morph) in [
            (u32::MAX, false, true, false, false),
            (u32::MAX, true, false, true, true),
            (u32::MAX, true, true, false, false),
            (R, true, true, true, false),
            (R, false, false, false, false),
            (B, true, true, false, true),
            (T, true, false, false, false),
            (AP, true, false, false, false),
            (0, true, false, false, false),
            (B | R, true, true, true, true),
        ] {
            assert_eq!((render_blob_muda(props, patch, alvo), morph_blob_muda(props, patch, alvo)), (render, morph), "{props:#x} {patch} {alvo}");
        }
    }

    #[test]
    fn material_com_arroba_e_mesh_dinamico() {
        for (nome, template, dinamico) in [("", false, false), ("@", true, true), ("skin@", false, true), ("skin", false, false), (" @x", false, true), ("ＡＢ@", false, true)] {
            assert_eq!((material_de_template(nome), material_dinamico(nome)), (template, dinamico), "{nome}");
        }
        let d = |m: &[&str], a: &[&[&str]]| {
            let ap: Vec<Vec<&str>> = a.iter().map(|c| c.to_vec()).collect();
            mesh_dinamico(m, &ap, |n| material_de_template(n), |c| material_dinamico(c))
        };
        assert!(d(&[], &[&["a"]]));
        assert!(d(&["a"], &[]));
        assert!(!d(&["a"], &[&["a"]]));
        assert!(d(&["a", "@t"], &[&["a"]]));
        assert!(d(&["a"], &[&["a"], &[]]));
        assert!(d(&["a"], &[&["a", "x@y"]]));
        assert!(!d(&["a", "b"], &[&["a", "b"], &["b"]]));
        assert!(!d(&["x@y"], &[&["a"]]));
    }

    #[test]
    fn tags_do_patch_sao_os_cnames_do_formato() {
        assert_eq!(tag_appearance_parts(), 0xff98_df43_873a_42b3);
        assert_eq!(tag_resource_patch(), 0xad1f_0871_ff62_0e19);
        assert_eq!(instancias_anexadas(&[PassoDaMescla::Anexa, PassoDaMescla::Anexa], &[[10, 1]]), vec![10]);
        assert!(definicao_inclui_partes(3, &[tag_appearance_parts()], true, tag_appearance_parts()));
        assert!(!definicao_inclui_partes(1, &[18417716157256188596], false, tag_appearance_parts()));
    }
}
