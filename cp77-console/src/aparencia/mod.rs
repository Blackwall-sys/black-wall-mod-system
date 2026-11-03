//! Aparência das coisas no jogo (do personagem e dos NPCs): malhas, componentes, render proxies, controladores visuais, aparências dinâmicas e overrides de aparência.

#![allow(dead_code)]

pub(crate) mod do_record;
#[cfg(test)]
mod testes_da_mescla;

use crate::jogo::cstring::destroi_cstring_da_pilha;
use crate::mods_do_usuario::tramp_require;
use crate::mods_do_usuario::tramp_version;
use crate::mods_do_usuario::PLUGUE_ARCHIVEXL;
use crate::jogo::travas::TRAVA_TENTAR_ESCRITA_VM;
use crate::jogo::recursos::hash_do_caminho_do_recurso;
use crate::jogo::recursos::token_do_path;
use crate::jogo::objetos::handle_forte_de;
use crate::jogo::objetos::gravar_handle_fraco;
use crate::jogo::objetos::destravar_fraco;
use crate::jogo::objetos::travar_fraco;
use crate::jogo::aparencia::APPDEF_NAME_OFF;
use crate::jogo::aparencia::APPDEF_PARTSVALUES_OFF;
use crate::jogo::aparencia::APPDEF_VISUALTAGS_OFF;
use crate::jogo::listas::MOVER_HANDLES_VM;
use crate::jogo::listas::dynarray_no_campo;
use crate::marcador_individual;
use crate::tweakdb::natives::a9_registro;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use crate::ganchos::original_publicado;
use crate::ganchos::instalar_conferindo_prologo;
use crate::mundo::*;
use crate::ganchos::*;
use crate::diagnostico::{
    a11_alias, a11_anotar, a11_descritores, a11_pares_gravados, a17_classes, a17_garment_observar, a17e_alvos_de_skin, a17e_amostrar,
    a51f_teste_hashes, A51fAmostra, A11_PRE, A11_TESTE, A17_CHAMADAS, A17_GARMENT_APLICADOS, A17_OBSERVAR, A17_PATCHES_NAO_PRONTOS, A17_VISTAS,
    A48_ESTADOS_CRIADOS, A48_PATCHES_REGISTRADOS, A48_TRAVA_OCUPADA, A51B_TESTE_ALVO, A51F2_AMOSTRAS, A51F2_RECURSOS, A51F2_TESTE, A51F2_TESTE_NONE,
    A51F3_AMOSTRAS, A51F3_COM_TAG, A51F3_DEFINICOES, A51F3_INCLUSOES, A51F3_LOGADAS, A51F3_NAO_PRONTAS, A51F3_PARTES, A51F3_SCHEMAS_COM_TAG,
    A51F3_SEM_PACOTE, A51F3_TESTE, A51F_AMOSTRAS, C130D_ANOTOU, C130D_ATUAL, C130D_CONGELADO, C130D_DESPACHOS, C130D_NO_BUILDER, C130D_SEM_ANOTACAO,
    C130D_SOMADO, C130D_SOMADOS, C130D_TESTE, C134E_CONGELADO, C134E_REDESPACHANDO, C134E_REDESPACHAR, C134E_TOGGLE_POR_HOOK,
};
// Só os handlers de prova usam, e o `corpo_de_prova!` público não os cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{tramp_component_state_restore, tramp_component_state_save, tramp_visual_tag_state_restore, tramp_visual_tag_state_save};
use crate::tipos_do_motor::*;
use crate::arquivos_do_jogo::*;
use crate::texto_e_numeros::*;
#[cfg(feature = "devtools")]
use crate::tarefas::*;
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

/// Codeware `#25`: `EntityBuilderAppearanceWrapper` (pai `IScriptable`), sobre uma `EntityBuilderAppearance` do builder.
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

// ===== Codeware `#33`/`#167` — `entVisualControllerComponent.reds`/`VisualControllerComponentEx.hpp`
// ("possível pista pro full-body"): probes observe-only via a técnica NOVA descoberta
// 2026-08-12 (aplicada hoje ao ArchiveXL pela 1ª vez neste projeto, aqui aplicada ao Codeware pela
// PRIMEIRA vez) — minerar `cp77-symbols/symbols-demangled.txt` (68654 símbolos, binário macOS
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
// componentes referenciam (item Codeware `#167`/`VisualControllerComponentEx.hpp`:
// `Raw::VisualController::LoadDependencies` + `Raw::VisualControllerComponent::Controller@0xD8`,
// ponteiro pra uma struct interna SEPARADA `VisualController{dependencies[],looseDependencies[],
// job,status,lock}@0x80`, offsets confirmados no header vendorizado real, item `#213` ✅). NUNCA
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

// ===== Codeware `#213` (`Red/VisualController.hpp`) — leitura de campo DIRETA da struct
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
// padrão do `#168`/`ComponentWrapper`), NÃO `entVisualControllerComponent` (que é `native class`
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

// ===== Codeware `#33` (`entVisualControllerComponent.reds`) — leitura PURA dos campos de TOPO
// do componente (distinto do `#213` acima, que lê a sub-struct C++ interna `VisualController`
// apontada por `Controller@0xD8`; aqui são os campos DECLARADOS DIRETO na classe RTTI real,
// offsets 0x90-0xF3).
//
// **CORREÇÃO DE RISCO 2026-08-18 (offline, mesma técnica que destravou o item `#107` nesta
// sessão) — a nota de risco de 2026-08-12 pra este item ("declarar a classe arriscaria o crash de
// bind RTTI, precisa de RE de endereço via hook, tipo possivelmente ausente da RTTI") checou só
// `redscript-src` (grep, zero hits pro nome da classe) e NUNCA cruzou contra o header GERADO da
// reflection real do próprio jogo (`RED4ext.SDK/include/RED4ext/Scripting/Natives/Generated/ent/
// VisualControllerComponent.hpp`, comentário "This file is generated from the Game's Reflection
// data" — confiança máxima, mesma categoria já usada pra fechar `#107`/`#197`/`#160`). Esse header
// confirma os 4 campos citados na nota original do item com offsets EXATOS e AUTO-CONSISTENTES —
// cross-referenciados contra 2 headers vendorizados INDEPENDENTES (`ent/IComponent.hpp` pro
// tamanho do parent, `ResourceReference.hpp` pro tamanho de `Ref<T>`/`RaRef<T>`, `ent/
// VisualControllerDependency.hpp` pro stride de cada entry do array):
//   `IComponent` = 0x90 bytes exato (RED4EXT_ASSERT_SIZE, header separado) → `meshProxy` começa
//   em 0x90 (bate); `Ref<CMesh>` = `ResourcePath path@+0x00` + `SharedPtr<...> token@+0x08` =
//   0x18 bytes (RED4EXT_ASSERT_SIZE(ResourceReference<>,0x18)) → 0x90+0x18=0xA8 bate com
//   `appearanceDependency`; `DynArray<T>` = 0x10 bytes (`entries*@+0x00/capacity(u32)@+0x08/
//   size(u32)@+0x0C`, mesmo layout já batalha-testado em dezenas de itens deste projeto) →
//   0xA8+0x10=0xB8 bate com `cookedAppearanceData`; `RaRef<T>` = `ResourcePath path@+0x00` = 0x8
//   bytes (RED4EXT_ASSERT_SIZE(ResourceAsyncReference<>,0x8)) → 0xB8+0x8=0xC0 bate com o filler
//   documentado antes de `forcedLodDistance@0xF0`. NENHUMA lacuna, NENHUMA sobreposição — layout
//   genuinamente confirmado por 4 fontes cruzadas, não candidato de RE.
//
// **A "correção" de 2026-08-12 continua certa sobre 1 coisa**: declarar `native class
// entVisualControllerComponent extends IComponent {...}` continuaria arriscando o crash de bind
// RTTI (mesma categoria `#30`/CMesh-trap — a CLASSE em si tem zero ocorrência em `redscript-src`,
// não sabemos se o bind aceita a ancestralidade `IComponent` sem crashar; risco não-testado,
// não-assumido). **Mas os CAMPOS não precisam da classe declarada** — mesma técnica já usada
// pelo item irmão `#213` (`visualcontroller_ptr`, leitura via `Controller@0xD8`) e por `#71`
// (VehicleObject)/`#168` (ComponentWrapper): o parâmetro é sempre `ref<IComponent>` (tipo vanilla
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
//     header GERADO `ent/VisualControllerDependency.hpp`) — count + 3 getters indexados (mesmo
//     padrão `GetX(i)`/`GetXCount()` já usado em dezenas de arrays deste projeto, evita o risco de
//     `array<T>`-retorno que já causou crash real noutro item — ver `TweakXL #43`).
//   - `cookedAppearanceData: RaRef<appearance::CookedAppearanceData>` (0xB8) → hash do
//     `ResourcePath` embutido.
//   - `forcedLodDistance: ent::ForcedLodDistance` (0xF0, enum C++ de 1 byte, valores 0-8) →
//     `Int32` cru (sem declarar enum redscript-facing novo, evita mais um tipo).
//
// `LoadAppearanceDependencies()` (o único MÉTODO do item real) fica no bloco
// `Codeware #33 — LoadAppearanceDependencies`, abaixo.
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

// ===== Codeware `#43` (`Mesh/CMaterialInstance.reds`/`CMeshMaterialEntry.reds`/
// `meshMeshAppearance.reds`) — correção de risco 2026-08-18, mesma técnica que destravou
// `#107`/`#33`/`#11` hoje: a nota antiga (2026-08-12) dizia CMesh-trap pros 3 nomes porque
// `grep redscript-src` = zero hits — CONFIRMADO de novo aqui contra os headers `Generated/`
// (`CMaterialInstance.hpp`/`CMeshMaterialEntry.hpp`/`mesh/MeshAppearance.hpp`, "confiança
// máxima"): os 3 tipos SÃO genuínos (offsets exatos documentados), mas a cadeia de ancestrais
// inteira é CMesh-trap de verdade — `CMaterialInstance extends IMaterial extends CResource
// extends ISerializable` e `meshMeshAppearance extends ISerializable` (direto) têm ZERO
// ocorrências em `redscript-src` em CADA elo (`IMaterial`/`CResource`/`CMaterialInstance`
// checados agora, nenhum hit). Diferente de `#33`, aqui NÃO existe declarar-nada e usar
// `IComponent` widened — o único "dono" nativo de `CMeshMaterialEntry`/`CMaterialInstance`/
// `meshMeshAppearance` é o próprio `CMesh` (`materialEntries@0xE0`/`localMaterialInstances@0x100`/
// `appearances@0x1E0`, `CMesh.hpp`), e não existe atalho pra chegar num `CMesh*` VIVO: o único
// campo que aponta pra lá (`entVisualControllerComponent.meshProxy: Ref<CMesh>@0x90`, o mesmo
// já lido pelo `#33` acima) é um `Ref<T>` = `{path:ResourcePath@0x00, token:SharedPtr<
// ResourceToken<CMesh>>@0x08}` (`ResourceReference.hpp`) — o `token` só existe se ALGUÉM (o
// motor) já chamou `LoadAsync`/`Load` nesse `Ref` antes; nada neste projeto dispara isso.
//
// Implementado MESMO ASSIM, de forma 100% observe-only e sem NUNCA declarar `CMesh`/
// `CMaterialInstance`/`CMeshMaterialEntry`/`meshMeshAppearance`: se o `token` ACONTECER de já
// estar populado (carregado por outro caminho do motor, fora do nosso controle) e
// `ResourceToken::IsLoaded()` (`finished!=0 && error==0`, offsets `+0x58`/`+0x5C`,
// `ResourceLoader.hpp`, "confiança máxima") for `true`, seguimos até `resource: Handle<CMesh>@
// +0x28` → `.instance@+0x00` (`Handle<T>` = `SharedPtrBase<T>{instance*,refCount*}`,
// `Handle.hpp`/`SharedPtr.hpp`, MESMO layout já usado em toda `rtti::make_handle` deste
// projeto) → `CMesh*` genuíno. Com isso, lemos os 2 arrays que seguram os 3 tipos do item:
// `materialEntries: DynArray<CMeshMaterialEntry>@0xE0` (stride 0x30, `name:CName@+0x20`/
// `index:Uint16@+0x28`; o 4º campo do `.reds` real, `material`, NÃO aparece no dump de
// reflection — 5 bytes de `unk` sobram até 0x30, sem espaço documentado pra um ponteiro,
// então NÃO lido, evita chute) e `appearances: DynArray<Handle<mesh::MeshAppearance>>@0x1E0`
// (stride 0x10, cada slot é outro `Handle<T>.instance` → `MeshAppearance*` → `name:CName@+0x30`
// do próprio objeto). `CMaterialInstance` (a partir de `localMaterialInstances@0x100`, mesmo
// padrão `Handle<T>` stride 0x10) fica de fora desta rodada por escopo (o array já demonstra o
// mesmo mecanismo; ver nota de fechamento no catálogo).
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
/// (`RED4EXT_ASSERT_SIZE(MeshAppearance,0xA8)`).
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
/// (CName, `mesh::MeshAppearance` real, `Generated/mesh/MeshAppearance.hpp`).
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

// ===== Codeware `#33` — `LoadAppearanceDependencies` (2026-09-13) =====
// O `LoadDependencies` do Mac é `0x10199e498` (`proofs/20260911-codeware-213-visualcontroller-loaddependencies.log`): `x0` controller, `w1` flag,
// `x8` = JobHandle de 8 B, que ele mesmo constrói (`0x104ddf648`). A ordem é a da referência: sem controller, falso; cada `appearanceDependency` do
// componente (passo 0x18: mesh@0, appearanceName@8) cujo mesh não está em `controller.dependencies` (passo 0x30: mesh@0x20, appearanceName@0x28)
// ganha uma entrada. A entrada nasce pelo Resize do próprio motor (`0x10199e36c`, o que o agendador `0x10199deac` usa: realoca com o mover do tipo e
// constrói zerado), com a trava exclusiva `lock@7C` pegada (`0x1000015f0`; soltar é limpar o bit 0x80, como o motor faz). A trava é solta antes do
// `LoadDependencies`, que trava de novo. A espera usa o `IsDone` do JobHandle (`0x104ddf8e8`: contador de pendentes do job zerado), com teto de 5 s,
// e o handle é destruído por `0x104ddf828`.
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

#[cfg(feature = "devtools")]
pub(crate) type VcDependenciasResizeFn = unsafe extern "C" fn(*mut u8, u32);

#[cfg(feature = "devtools")]
pub(crate) unsafe fn vc_travar(ctrl: *mut u8) {
    crate::jogo::travas::travar_escrita((ctrl.add(0x7C)) as *mut u8);
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn vc_destravar(ctrl: *mut u8) {
    crate::jogo::travas::soltar_escrita((ctrl.add(0x7C)) as *mut u8);
}

/// `controller.dependencies`: (entradas, tamanho); `None` se ilegível.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vc_dependencias(ctrl: *const u8) -> Option<(*mut u8, u32)> {
    let entradas = ctrl.add(0x10).cast::<u64>().read_unaligned() as *mut u8;
    let tam = ctrl.add(0x1C).cast::<u32>().read_unaligned();
    if tam > 4096 || (tam != 0 && (entradas.is_null() || !crate::memoria::is_readable(entradas as *const c_void, tam as usize * VC_DEPENDENCIA_PASSO))) {
        return None;
    }
    Some((entradas, tam))
}

/// Tamanho de `controller.dependencies` e `status`; `None` sem controller.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_controller_estado(comp: *mut c_void) -> Option<(u32, i32)> {
    vcc_ptr(comp)?;
    let ctrl = visualcontroller_ptr(comp)? as *const u8;
    let (_, tam) = vc_dependencias(ctrl)?;
    Some((tam, ctrl.add(0x78).cast::<i32>().read_unaligned()))
}

/// Entrada `idx` de `controller.dependencies`: (token, instância da aparência, mesh, appearanceName).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_controller_dependencia(comp: *mut c_void, idx: u32) -> Option<(u64, u64, u64, u64)> {
    vcc_ptr(comp)?;
    let ctrl = visualcontroller_ptr(comp)? as *const u8;
    let (entradas, tam) = vc_dependencias(ctrl)?;
    if idx >= tam {
        return None;
    }
    let e = entradas.add(idx as usize * VC_DEPENDENCIA_PASSO) as *const u64;
    Some((e.read_unaligned(), e.add(2).read_unaligned(), e.add(4).read_unaligned(), e.add(5).read_unaligned()))
}

/// `LoadAppearanceDependencies` sem a espera: (entradas acrescentadas, JobHandle vivo ou 0, que fica com quem chamou); `None` sem controller.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_carregar_dependencias(comp: *mut c_void) -> Option<(u32, u64)> {
    vcc_ptr(comp)?;
    let ctrl = visualcontroller_ptr(comp)? as *mut u8;
    let lista = (comp as *const u64).add(0xA8 / 8).read_unaligned() as *const u64;
    let n = (comp as *const u32).add(0xB4 / 4).read_unaligned();
    if n > 4096 || (n != 0 && (lista.is_null() || !crate::memoria::is_readable(lista as *const c_void, n as usize * VCC_DEPENDENCIA_PASSO))) {
        return None;
    }
    let redimensionar: VcDependenciasResizeFn = std::mem::transmute(crate::rebase(VCC_DEPENDENCIAS_RESIZE_VM));
    let mut novas = 0u32;
    vc_travar(ctrl);
    for i in 0..n as usize {
        let (mesh, aparencia) = (lista.add(i * 3).read_unaligned(), lista.add(i * 3 + 1).read_unaligned());
        let Some((entradas, tam)) = vc_dependencias(ctrl) else { break };
        if (0..tam as usize).any(|j| (entradas.add(j * VC_DEPENDENCIA_PASSO + 0x20) as *const u64).read_unaligned() == mesh) {
            continue;
        }
        redimensionar(ctrl.add(0x10), tam + 1);
        let Some((entradas, tam_novo)) = vc_dependencias(ctrl) else { break };
        if tam_novo != tam + 1 {
            break;
        }
        let nova = entradas.add(tam as usize * VC_DEPENDENCIA_PASSO) as *mut u64;
        nova.add(4).write_unaligned(mesh);
        nova.add(5).write_unaligned(aparencia);
        novas += 1;
    }
    vc_destravar(ctrl);
    if novas == 0 {
        return Some((0, 0));
    }
    let mut job = 0u64;
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(VCC_LOADDEPENDENCIES_VM),
        in("x0") ctrl as u64,
        in("x1") 0u64,
        in("x8") &mut job as *mut u64,
        clobber_abi("C"),
    );
    Some((novas, job))
}

/// Acrescenta `{mesh, appearanceName}` em `appearanceDependency` do componente (o campo que a referência expõe ao script); devolve o índice.
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

/// Tira a última entrada de `controller.dependencies` se o mesh for `mesh`, pelo Resize do motor (destrói a entrada e solta o token).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_remover_ultima_do_controller(comp: *mut c_void, mesh: u64) -> bool {
    if vcc_ptr(comp).is_none() {
        return false;
    }
    let Some(ctrl) = visualcontroller_ptr(comp) else { return false };
    let ctrl = ctrl as *mut u8;
    let redimensionar: VcDependenciasResizeFn = std::mem::transmute(crate::rebase(VCC_DEPENDENCIAS_RESIZE_VM));
    vc_travar(ctrl);
    let removeu = match vc_dependencias(ctrl) {
        Some((entradas, tam)) if tam > 0 && (entradas.add((tam as usize - 1) * VC_DEPENDENCIA_PASSO + 0x20) as *const u64).read_unaligned() == mesh => {
            redimensionar(ctrl.add(0x10), tam - 1);
            true
        }
        _ => false,
    };
    vc_destravar(ctrl);
    removeu
}

/// Tira a última entrada de `appearanceDependency` do componente se o mesh for `mesh` (dados simples: só o tamanho volta).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_remover_ultima_do_componente(comp: *mut c_void, mesh: u64) -> bool {
    if vcc_ptr(comp).is_none() {
        return false;
    }
    let arr = (comp as *mut u8).add(0xA8);
    let entradas = arr.cast::<u64>().read_unaligned() as *const u64;
    let tam = arr.add(0x0C).cast::<u32>().read_unaligned();
    if tam == 0 || entradas.is_null() || !crate::memoria::is_readable(entradas as *const c_void, tam as usize * VCC_DEPENDENCIA_PASSO) {
        return false;
    }
    if entradas.add((tam as usize - 1) * 3).read_unaligned() != mesh {
        return false;
    }
    arr.add(0x0C).cast::<u32>().write_unaligned(tam - 1);
    true
}

/// `entVisualControllerComponent` do V, ou nulo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn vcc_do_jogador(reg: &Registry) -> *mut c_void {
    componente_do_jogador(reg, "entVisualControllerComponent")
}

/// `BwmsVccLoadAppearanceDependencies(component: ref<IComponent>, wait: Bool) -> Bool` — verdadeiro se acrescentou dependência ao controller.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_vcc_load_appearance_dependencies(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let esperar = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let carga = if rtti::sane(inst) { vcc_carregar_dependencias(inst) } else { None };
    let mut espera = None;
    if let Some((_, mut job)) = carga {
        if esperar && job != 0 {
            espera = Some(jobhandle_esperar(job, VCC_ESPERA_MS));
        }
        jobhandle_soltar(&mut job);
    }
    let novas = carga.map_or(0, |c| c.0);
    crate::log_dev!(
        "[cw-vcc-33] BwmsVccLoadAppearanceDependencies(component={inst:p}, wait={esperar}) -> controller={} novas={novas} concluiu={espera:?}",
        carga.is_some()
    );
    write_bool_ret(out, novas > 0);
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
    let p1 = compose_params_from_types(reg, &["handle:IComponent", "Bool"]);
    let ok1 = params_compostos(p1)
        && registro_de_prova!(register_global_composed(reg, proto, p1, "BwmsVccLoadAppearanceDependencies", "BwmsVccLoadAppearanceDependencies", corpo_de_prova!(tramp_vcc_load_appearance_dependencies)));
    let p2 = compose_params_from_types(reg, &["handle:IComponent", "ResRef", "CName"]);
    let ok2 = params_compostos(p2)
        && registro_de_prova!(register_global_composed(reg, proto, p2, "BwmsVccAddAppearanceDependency", "BwmsVccAddAppearanceDependency", corpo_de_prova!(tramp_vcc_add_appearance_dependency)));
    crate::log_dev!("[reg] register_all: cw-vcc-33 LoadAppearanceDependencies={ok1} AddAppearanceDependency={ok2}");
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

/// `BwmsMeshRefScan(obj, ate) -> Int32`/// `BwmsMeshRefScan(obj, ate) -> Int32` — loga TODO `u64` alinhado em `[0, ate)` do objeto.
///
/// Existe porque o `0x228` do header oficial **não** entregou um `ResourcePath` no
/// `entSkinnedMeshComponent` desta build: o valor lido (`0xebada5168620c5fe`) não corresponde a
/// nenhum dos **663.364** paths do install (`bwms-catalog::hash_para_path`, ferramenta validada
/// contra 2 hashes que o próprio motor produziu no `#18`). O header declara
/// `uint8_t unk1E0[0x228-0x1E0]` — ou seja, o próprio SDK não conhece essa faixa, então o layout
/// pode divergir aqui.
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

/// Codeware `#30` (`MeshComponent.meshResource: ref<CMesh>`) — 2026-08-10 (cont.192). Achado de
/// pesquisa (agente background): `enablers/Codeware/src/App/Entity/MeshComponentEx.hpp` —
/// `RTTI_EXPAND_CLASS(Red::entMeshComponent, {RTTI_PROPERTY(Red::Handle<Red::CMesh>, 0x158,
/// "meshResource")})` — offset REAL do C++ vendorizado, campo Handle já existente (16 bytes,
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

/// axl-fullbody-torso (2026-08-07, cont.41): em vez de esperar um evento disparar pra um
/// componente de torso (3 candidatos já testados, todos silenciosos ou só-mundo — ver
/// `notes/GARMENT-APPEARANCE-COMPONENT-FACTS.md` §3), ENUMERA DIRETO os componentes reais do
/// player AGORA — mesmo offset GROUND-TRUTH já usado em `cw-entity-builder`
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

/// `ComponentWrapper::GetResourcePath` (`Wrapper.cpp:149-165`) — campo cru de 8 bytes (o
/// `RaRef<T>` inteiro É o `ResourcePath`, ver `garment_mesh_path_offset`).
pub(crate) unsafe fn component_get_resource_path(inst: *mut c_void, kind: GarmentComponentKind) -> Option<u64> {
    let off = garment_mesh_path_offset(inst, kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64(inst, off))
}

/// `ComponentWrapper::SetResourcePath` (`Wrapper.cpp:167-183`) — `aComponent->mesh = aPath;
/// return true;` (ou `morphResource` pro kind `MorphTarget`). Campo cru, fiel à fonte real.
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

/// `ComponentWrapper::GetChunkMask` (Wrapper.cpp:350-366) — campo cru, sem native nenhuma.
pub(crate) unsafe fn component_get_chunk_mask(inst: *mut c_void, kind: GarmentComponentKind) -> Option<u64> {
    let off = garment_chunk_mask_offset(inst, kind)?;
    if !crate::memoria::is_readable((inst as *const u8).add(off) as *const c_void, 8) {
        return None;
    }
    Some(rd_u64(inst, off))
}

/// `ComponentWrapper::SetChunkMask` (Wrapper.cpp:368-384) — `aComponent->chunkMask = X;
/// return true;`, fielmente replicado: NÃO chama `RefreshAppearance`/nenhuma outra nativa
/// sozinho (quem orquestra o refresh + a combinação multi-mod de qual valor final aplicar
/// é `aplicar_mascara_composta`, sobre o `ComponentState` de `bwms_primitives::override_state` —
/// esta função Rust é deliberadamente só a ponta "componente vivo").
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

/// `ComponentWrapper::GetAppearanceName`/`SetAppearanceName` (Wrapper.cpp:276-310) — mesma
/// categoria (campo cru, `CName` = `uint64_t` hash puro, mesmo formato já usado em toda a
/// codebase pra CName). Expostas ao redscript pelas natives `BwmsComponentGetMeshAppearanceName`/
/// `BwmsComponentSetAppearanceName` (seção Codeware `#168`, abaixo).
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

/// Nome da aparência (`meshAppearance`) do componente de mesh; `None` fora de mesh.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn componente_aparencia(reg: &crate::rtti::Registry, inst: *mut c_void) -> Option<u64> {
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::Unsupported {
        return None;
    }
    component_get_appearance_name(inst, kind)
}

/// A#22 22E — `EntityState::ApplyAppearanceOverride` composto, sem a resolução de nome dinâmico: os 2 retornos cedo da referência, a aparência do
/// componente, senão a do prefixo, senão a original (`override_state::aparencia_aplicada`), `SetAppearanceName` e `RefreshAppearance`. O refresh só roda
/// com vtable e slot 81 iguais a um par medido (`es20_refresh_medido`); fora disso volta `None` com o nome já gravado, como o `false` da referência
/// quando o refresh falha.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn aplicar_aparencia_composta(
    reg: &crate::rtti::Registry,
    inst: *mut c_void,
    original: u64,
    componente: Option<&bwms_primitives::override_state::ComponentState>,
    prefixo: Option<&bwms_primitives::override_state::ComponentState>,
) -> Option<u64> {
    if inst.is_null() || (inst as *const u8).add(0x8b).read_unaligned() == 0 {
        return None;
    }
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::Unsupported {
        return None;
    }
    let fim = bwms_primitives::override_state::aparencia_aplicada(|| original, componente, prefixo)?;
    if !component_set_appearance_name(inst, kind, fim) {
        return None;
    }
    let refresh = es20_refresh_medido(inst)?;
    refresh(inst);
    Some(fim)
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

// ===== Codeware `#168` (`App::ComponentWrapper.GetChunkMask`/`SetChunkMask`), 2026-08-12
// (round 2 do sweep de Codeware) — expõe pro REDSCRIPT o MESMO mecanismo do ArchiveXL `#22`
// (funções acima) — ZERO RE nova, reusa `detect_garment_component_kind`/
// `component_get_chunk_mask`/`component_set_chunk_mask` byte-a-byte (mesmos offsets
// confirmados contra o header RTTI oficial gerado da Reflection do jogo real, ver comentário
// grande no topo desta seção). O alvo é o `this` que o PRÓPRIO redscript já tem em mãos (ex.
// de dentro de um `@wrapMethod`/evento de componente), como na API real
// `ComponentWrapper::GetChunkMask (const)`/`SetChunkMask` (`Wrapper.cpp:316-350`), que opera
// sobre UM componente já identificado, não sobre uma busca por nome.
//
// **Achado de bookkeeping que acompanha esta implementação**: a OUTRA metade do item #168
// (`ComponentWrapper::IsEnabled`/`SetEnabled`) não precisa de NENHUM código novo — são
// `m_component->isEnabled` puro no C++ real (`Wrapper.cpp:138-147`), e o redscript já tem
// exatamente essa capacidade como NATIVA VANILLA REAL em `IComponent` (não forjada por nós):
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

/// `BwmsComponentRefreshAppearance(component: ref<IComponent>) -> Bool` — `ComponentWrapper::
/// RefreshAppearance` (`Wrapper.cpp:352-359`): `if (!IsMeshComponent()) return false;
/// Raw::MeshComponent::RefreshAppearance(m_component); return true;`, chamada virtual pelo slot 0x280
/// (Codeware `Red/Mesh.hpp:37-39`, `RawVFunc`). Aqui o alvo é fixo: `0x101991450`, o slot 81 de
/// `entSkinnedMeshComponent`, provado ao vivo 2x (`APPEARANCE_REBUILD_PROXY_VM`,
/// `crate::selftest::run_toggle_and_rebuild`/comando `togglerebuild`, 2026-08-07), chamado em toda a
/// `SkinnedFamily`; `MeshComponent` e `MorphTarget` devolvem `false` sem chamar nada. O
/// `entGarmentSkinnedMeshComponent` tem slot 81 próprio, `0x10198860c` (`ES20_REFRESH_MEDIDOS`, boot 201): no garment esta
/// native chama a implementação do `entSkinnedMeshComponent`, não a da classe, e diverge da referência. O desmonte dos dois
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
/// usado por `ComponentWrapper::GetUniqueId()` — `Wrapper.cpp:125-136` — junto com `appearancePath`,
/// TAMBÉM da base). A native VANILLA `IComponent.GetAppearanceName()` (`orphans.script:16659`) lê
/// o campo da BASE, não `meshAppearance` do subtipo — confirmado ao vivo: escrever em
/// `meshAppearance` via `SetAppearanceName` e reler via `GetAppearanceName()` (vanilla) sempre
/// deu `None`/`matchesExpected=false`, mesmo com `wroteOk=true` (a escrita em si funcionou,
/// só não é observável pela native errada). Por isso este getter tem nome DIFERENTE da fonte
/// real (`ComponentWrapper::GetAppearanceName`, que colide com o native vanilla de `IComponent`
/// — nunca declarável via `@addMethod` sem sombrear um `final` já existente).
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

/// `BwmsComponentIsMeshComponent(component: ref<IComponent>) -> Bool` — `ComponentWrapper::
/// IsMeshComponent` (`Wrapper.hpp:20`, `m_componentType != ComponentType::Unsupported`).
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

/// `BwmsComponentLoadResource(component: ref<IComponent>) -> Bool` — `ComponentWrapper::LoadResource` (Codeware `#168`,
/// `ComponentWrapper.cpp:185-209`): fila de jobs + `MeshComponent::LoadResource(component, fila)` + destrutor. O slot
/// `0x268` é `0x10198f864` (lido em 2026-09-11: `LoadAsync` da referência do mesh em `+0x220`, `Wait` no job do token pela
/// fila e despacho do closure de conclusão). Só chama se o slot da vtable do componente for exatamente essa função.
/// Divergência honesta: sem o `aRefresh` (closure de `RefreshAppearance` na fila — chame `BwmsComponentRefreshAppearance`
/// depois) e sem o `aWait` (espera bloqueante de até 5 s na thread do jogo).
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

/// `MeshAppearanceEx::ResetMaterialCache` do Codeware, na mesma ordem: espera o job de material (até 5 s, e segue depois do teto
/// como o `WaitForJob` dele), lock exclusivo, solta os dados de render e zera o ponteiro, esvazia o mapa soltando cada
/// `Handle<IMaterial>` do jeito do destrutor do mapa (`0x100ff5a10`), marca o cache como não pronto e destrava. Os buffers do mapa
/// ficam (é `Clear`, não destrutor).
pub(crate) unsafe fn resetar_cache_de_material(app: *mut c_void) -> bool {
    if !crate::memoria::is_readable(app as *const c_void, 0xa8) {
        return false;
    }
    let a = app as usize;
    let terminou: extern "C" fn(*const c_void) -> u32 = std::mem::transmute(crate::rebase(crate::jogo::fila::TAREFA_TERMINOU_VM));
    let inicio = std::time::Instant::now();
    while terminou((a + MESHAPP_JOB) as *const c_void) == 0 {
        if inicio.elapsed() >= std::time::Duration::from_millis(5000) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    crate::jogo::travas::travar_escrita(((a + MESHAPP_LOCK) as *mut c_void) as *mut u8);
    let dados = ((a + MESHAPP_MATERIAL_DATA) as *const u64).read_unaligned();
    if dados != 0 {
        let soltar: extern "C" fn(u64) = std::mem::transmute(crate::rebase(RENDERDATA_RELEASE_VM));
        soltar(dados);
        ((a + MESHAPP_MATERIAL_DATA) as *mut u64).write_unaligned(0);
    }
    let valores = ((a + MESHAPP_MAPA_VALORES) as *const u64).read_unaligned() as *mut [u64; 2];
    let n = ((a + MESHAPP_MAPA_VALORES_N) as *const u32).read_unaligned();
    if !valores.is_null() {
        for i in (0..n as usize).rev() {
            crate::jogo::objetos::soltar_handle(&mut *valores.add(i));
        }
    }
    ((a + MESHAPP_MAPA_VALORES_N) as *mut u32).write_unaligned(0);
    ((a + MESHAPP_MAPA_CHAVES_N) as *mut u32).write_unaligned(0);
    ((a + MESHAPP_CACHE_PRONTO) as *mut u8).write(0);
    crate::jogo::travas::soltar_escrita(((a + MESHAPP_LOCK)) as *mut u8);
    true
}

/// Aparência do mesh de um componente pelo nome (`CMesh::appearances`), como o `ComponentWrapper::ResetMaterialCache` do Codeware
/// acha. `None` = tipo fora do escopo (o MorphTarget usa o `baseMesh` do handle, não coberto), sem mesh ou nome não achado.
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

/// `BwmsComponentResetMaterialCache(component: ref<IComponent>) -> Bool` — Codeware `ComponentWrapper::ResetMaterialCache` (item
/// `#168`, 2026-09-11): reseta o cache de material da aparência do mesh do componente. O motor reconstrói na próxima aplicação.
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

/// `BwmsComponentLoadResourceToken(component: ref<IComponent>) -> Uint64` — Codeware `ComponentWrapper::LoadResourceToken` (item
/// `#168`, 2026-09-11): pede a carga do path do mesh do componente (offset pelo RTTI) ao loader do motor e devolve o id do token
/// guardado (0 = tipo fora do escopo, path vazio ou sem loader). Sem o `aWait` de 5 s do Codeware: `ResourceToken.WaitForJob()` espera.
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
    // `BwmsComponentGetAppearanceName` (nome real do `ComponentWrapper.hpp`) NÃO é declarado —
    // achado desta rodada (`redscript-src/orphans.script:16659`): `IComponent.GetAppearanceName()
    // -> CName` já é `final native const` VANILLA REAL (usado por código de jogo genuíno,
    // `weapon.script:642`) — mas confirmado AO VIVO nesta mesma rodada que ela lê um campo
    // DIFERENTE (`IComponent::appearanceName`, da classe BASE) do que `SetAppearanceName`
    // escreve (`meshAppearance`, do SUBTIPO) — ver nota grande em
    // `tramp_componentwrapper_get_mesh_appearance_name`. Por isso o getter do campo que
    // `SetAppearanceName` realmente usa é exposto sob um nome PRÓPRIO (`GetMeshAppearanceName`,
    // sem colisão), não o nome real (que colidiria com o `final` vanilla).
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

/// Nome completo (`value`) da aparência ligada à parte da entidade.
pub(crate) fn es22_aparencia_da_parte(entidade: u64, parte: u64) -> Option<u64> {
    ES22_RESOURCE_STATE.lock().ok()?.iter().find(|(k, _)| *k == (entidade, parte)).map(|(_, s)| s.value)
}

// ===== ArchiveXL `#20` 20b — overrides de um item no estado da entidade (2026-09-14) =====
// Porte de `Extension.cpp:679-737` sobre `States.cpp:329-381`. `RegisterComponentOverrides(estado, hash, definição)`: cada `partsOverrides` sem
// `partResource` registra, pra cada componente, a máscara como esconder (o `aShow = false` padrão) e a aparência (vazio e `default` não entram); cada tag
// de `visualTags` que o gerenciador de tags conhece registra as máscaras dela, mostrar ou esconder conforme a tag. `RegisterOffsetOverrides`: cada parte de
// `partsValues` recebe o deslocamento do item. `UpdatePartAttributes`: cada parte fica ligada à aparência de nome igual ao da definição. Tudo sob o hash do
// item; os `Unregister*` tiram só o dele. O estado por componente é o `ComponentState` do 22D/22E, por `(entidade, nome do componente)`.
pub(crate) type Es20Estados = std::collections::HashMap<(u64, u64), bwms_primitives::override_state::ComponentState>;

/// `EntityState::AddAppearanceOverride(hash, nome, aparência)`: vazio e `default` não entram. `true` se registrou.
pub(crate) fn es20_adicionar_aparencia(entidade: u64, hash: u64, nome: CName, aparencia: CName) -> bool {
    let (nome, aparencia) = (nome.valor(), aparencia.valor());
    if nome == 0 || aparencia == 0 || aparencia == crate::cname::cname("default") {
        return false;
    }
    let Ok(mut m) = es20_componentes().lock() else { return false };
    m.entry((entidade, nome)).or_default().sobrescrever_aparencia(hash, aparencia);
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

/// `EntityState::ApplyAppearanceOverride`: `Some(aparência escrita)` com o `RefreshAppearance` feito. O override do componente, senão o do prefixo,
/// senão a original quando algum dos dois já mudou. A referência lê a parte do estado do componente sem checar; aqui ela tem que existir (a mesma
/// divergência do 22E). O refresh só roda com vtable e slot 81 medidos (`es20_refresh_medido`); fora disso o nome fica escrito e volta `None`.
pub(crate) unsafe fn es20_aplicar_aparencia(m: &Es20Malhas, entidade: u64, inst: *mut c_void) -> Option<u64> {
    if inst.is_null() || (inst as *const u8).add(0x8b).read_unaligned() == 0 {
        return None;
    }
    let tipo = es20_tipo(m, inst)?;
    let (base, componente, prefixo) = es20_estados_do_componente(entidade, inst as *const u8);
    let campo = (inst as *mut u8).add(m.campos[tipo].1) as *mut u64;
    let original = || es20_original(entidade, es20_id_do_componente(m.base, inst as *const u8), 1, || campo.read_unaligned());
    let fim = bwms_primitives::override_state::aparencia_aplicada(original, componente.as_ref(), prefixo.as_ref())?;
    // A parte só depois do nome final, na ordem de `States.cpp:689-692`.
    es22_parte_do_componente(entidade, base)?;
    campo.write_unaligned(fim);
    let refresh = es20_refresh_medido(inst)?;
    refresh(inst);
    Some(fim)
}

pub(crate) const ES20_CAMINHO_VAZIO: &str = "base\\characters\\appearances\\player\\items\\empty_appearance.app";

/// Tag visual do template → marca no texto do seletor.
pub(crate) const ES20_TAGS_DE_VAZIA: [(&str, &str); 3] = [("EmptyAppearance:FPP", "&FPP"), ("EmptyAppearance:Male", "&Male"), ("EmptyAppearance:Female", "&Female")];

/// A `TemplateAppearance` vazia (0x18 B: `name`, `appearanceResource`, `appearanceName` `default`), com vida de processo.
pub(crate) fn es20_aparencia_vazia() -> *mut u8 {
    static VAZIA: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *VAZIA.get_or_init(|| {
        let v: &'static mut [u64; 3] =
            Box::leak(Box::new([crate::cname::cname(ES20_APARENCIA_VAZIA), bwms_primitives::resource_path_hash(ES20_CAMINHO_VAZIO), crate::cname::cname("default")]));
        v.as_mut_ptr() as usize
    }) as *mut u8
}

/// `FindResourceState(parte)`: a aparência ativa ligada à parte ou, com só deslocamento registrado, a vazia; `None` sem estado de recurso.
pub(crate) fn es20_aparencia_da_parte(entidade: u64, parte: u64) -> Option<DynamicAppearanceName> {
    let ligada = ES22_RESOURCE_STATE.lock().ok()?.iter().find(|(k, _)| *k == (entidade, parte)).map(|(_, s)| s.clone());
    ligada.or_else(|| ES67_RESOURCE_OFFSET.lock().ok()?.iter().any(|(k, _)| *k == (entidade, parte)).then(DynamicAppearanceName::default))
}

/// Aparência ativa da parte ligada ao nome base (`FindComponentState` com `IsFromAppearancePart` e `FindResourceState`).
pub(crate) fn es20_aparencia_do_componente(entidade: u64, base: u64) -> Option<DynamicAppearanceName> {
    es22_parte_do_componente(entidade, base).and_then(|p| es20_aparencia_da_parte(entidade, p))
}

/// `EmptyAppearanceTags`: alguma tag do template com a marca dela no texto do seletor.
pub(crate) fn es20_pede_aparencia_vazia(tags: &[u64], seletor: &str) -> bool {
    ES20_TAGS_DE_VAZIA.iter().any(|(tag, marca)| tags.contains(&crate::cname::cname(tag)) && seletor.contains(marca))
}

/// `appearances.EmplaceBack(*s_emptyAppearance)` com `name` = seletor: a entrada nova.
pub(crate) unsafe fn es20_anexar_aparencia_vazia(tpl: *mut u8, seletor: u64) -> Option<*mut u8> {
    let arr = tpl.add(ES22_TEMPLATE_APARENCIAS);
    let tam = (arr.add(0x0c) as *const u32).read_unaligned();
    let nova = es22_vaga_no_fim(arr, ES22_TEMPLATE_ENTRADA)?;
    std::ptr::copy_nonoverlapping(es20_aparencia_vazia(), nova, ES22_TEMPLATE_ENTRADA);
    (nova as *mut u64).write_unaligned(seletor);
    (arr.add(0x0c) as *mut u32).write_unaligned(tam + 1);
    Some(nova)
}

/// `BwmsAparencia.LinkComponentToPart(entity: wref<Entity>, component: ref<IComponent>,
/// resourcePath: Uint64) -> Bool` — porte de `EntityState::LinkComponentToPart` (`States.cpp:
/// 390-399`), simplificado pra 1 entidade EXPLÍCITA em vez do loop `OverrideStateManager::
/// LinkComponentToPart` sobre TODAS as entidades rastreadas (divergência de escopo consciente,
/// documentada — este projeto não tem um `OverrideStateManager` multi-entidade, só o registry
/// key-by-entity-pointer desta seção). Setup necessário ANTES de `ApplyAppearanceOverride`/
/// `ToggleConditionalComponents` — sem isso o nome base fica sem parte em `ES22_COMPONENT_APPEARANCE`
/// (`IsFromAppearancePart()==false`), e os 2 métodos abaixo recusam aplicar (early-return fiel,
/// nunca crash).
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
/// String) -> Bool` — porte de `EntityState::LinkPartToAppearance(Red::ResourcePath,
/// Red::CName)` (`States.cpp:409-415`), a via que recebe uma STRING crua e parseia via
/// `DynamicAppearanceController::ParseAppearance` (item `#23`, `DynamicAppearanceName::parse`,
/// já portado/provado). Setup necessário pra `ToggleConditionalComponents`
/// (`ResourceState::GetActiveAppearance()`) e pro safety-net de `ApplyAppearanceOverride`.
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
/// modHash: Uint64, appearance: CName) -> Bool` — porte de `EntityState::AddAppearanceOverride
/// (uint64_t aHash, Red::CName aComponentName, Red::CName aAppearance)` (`States.cpp:348-357`),
/// divergência de forma: recebe o COMPONENTE vivo (não o `CName` do nome) — parseia
/// `componentRef.name` internamente, mesma disciplina das outras funções desta seção. Grava no
/// `ComponentState` de `es20_componentes` pela chave `(entidade, nome base)`, a mesma loja que o es20 usa, então
/// os dois caminhos decidem a aparência igual. Fiel ao guard real: `aAppearance` vazio ou == `"default"` não registra
/// nada (`if (aAppearance && aAppearance != DefaultAppearanceName)`).
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

/// `EntityState::ApplyAppearanceOverride` (`States.cpp:659-697`) — a composição alvo desta
/// rodada. Réplica fiel dos 2 early-returns (`!isEnabled`/`!IsMeshComponent()`), da decisão de
/// `finalAppearance` pela regra única `override_state::aparencia_aplicada` sobre o estado do nome base e o do prefixo em
/// `es20_componentes` (com 2 mods vence o menor hash) e a original em `es20_originais`, lida só na restauração, e do
/// composto final `SetAppearanceName + RefreshAppearance` (ambos já provados ao vivo, rodada
/// 14/`#168`). **Divergência de segurança consciente**: a fonte real dereferencia
/// `componentState->GetAppearancePart()` SEM checar null nesse ponto (`States.cpp:692`) — se
/// `componentState` for null ali (chegou só via `prefixState`), a fonte real teria UB/crash.
/// Nosso safety net EXIGE o nome base com parte ligada (via `LinkComponentToPart`) antes
/// de prosseguir, devolvendo `false` em vez de arriscar — nunca reproduzimos um crash da fonte
/// original de propósito. `ResolveName` (`Dynamic.cpp:341-360`): nome com `*` passa pelo estado da
/// entidade calculado na hora (`selftest::resolver_nome_da_entidade`, `#23`), nome comum volta igual.
/// Devolve o texto do log e se a aparência foi escrita — o retorno da native sai do bool, nunca da redação do log.
pub unsafe fn run_apply_appearance_override(reg: &crate::rtti::Registry, entity: *mut c_void, inst: *mut c_void) -> (String, bool) {
    let mut escreveu = false;
    let texto = aplicar_aparencia_texto(reg, entity, inst, &mut escreveu);
    (texto, escreveu)
}

/// O que o `BwmsAparencia.ApplyAppearanceOverride` lê da loja do es20 antes de escrever (`States.cpp:670-692`).
pub(crate) struct Es22DecisaoDeAparencia {
    /// Override do nome base e do prefixo, para o log.
    pub(crate) fontes: (Option<u64>, Option<u64>),
    /// `aparencia_aplicada` sobre o estado do nome base e o do prefixo; `None` = sai sem escrever.
    pub(crate) aparencia: Option<u64>,
    /// Parte ligada ao nome base (`LinkComponentToPart`).
    pub(crate) parte: Option<u64>,
}

/// A decisão do `BwmsAparencia.ApplyAppearanceOverride` sobre o componente vivo, com a mesma loja e a mesma regra do `es20_aplicar_aparencia`.
/// `original` só roda no ramo de restauração.
pub(crate) unsafe fn es22_decidir_aparencia(entidade: u64, inst: *const u8, original: impl FnOnce() -> u64) -> Es22DecisaoDeAparencia {
    let (base, componente, prefixo) = es20_estados_do_componente(entidade, inst);
    Es22DecisaoDeAparencia {
        fontes: (componente.as_ref().and_then(|s| s.aparencia_sobrescrita()), prefixo.as_ref().and_then(|s| s.aparencia_sobrescrita())),
        aparencia: bwms_primitives::override_state::aparencia_aplicada(original, componente.as_ref(), prefixo.as_ref()),
        parte: es22_parte_do_componente(entidade, base),
    }
}

unsafe fn aplicar_aparencia_texto(reg: &crate::rtti::Registry, entity: *mut c_void, inst: *mut c_void, escreveu: &mut bool) -> String {
    if !rtti::sane(inst) {
        return "[applyappearance] component ptr insano".to_string();
    }
    let enabled = (inst as *const u8).add(0x8b).read_unaligned() != 0;
    if !enabled {
        return format!("[applyappearance] component={inst:p} isEnabled=false -> ApplyAppearanceOverride retornaria false (early-return fiel, States.cpp:661-662)");
    }
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::Unsupported {
        return format!("[applyappearance] component={inst:p} IsMeshComponent()==false -> ApplyAppearanceOverride retornaria false (States.cpp:666-667)");
    }
    let name_hash = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
    let name_str = crate::cname::resolve_cname(name_hash);
    let entity_key = entity as u64;
    // Sem `Es20Malhas` não há `GetUniqueId` para a chave da original: 0, que a regra trata como recusa.
    let original = || es20_malhas(reg).map_or(0, |m| es20_original_da_aparencia(m.base(), entity_key, inst, kind));
    let decisao = es22_decidir_aparencia(entity_key, inst as *const u8, original);
    let fontes = format!("override do componente {:#x?}, do prefixo {:#x?}", decisao.fontes.0, decisao.fontes.1);
    let Some(final_appearance) = decisao.aparencia else {
        return format!(
            "[applyappearance] component={inst:p} kind={kind:?} nome='{name_str}' nada a aplicar ({fontes}; sem ChangesAppearance ou original 0) -> retornaria false"
        );
    };

    let Some(part) = decisao.parte else {
        return format!(
            "[applyappearance] component={inst:p} finalAppearance={final_appearance:#018x} ({fontes}) MAS o nome base sem parte ligada (LinkComponentToPart nunca chamado) -> a fonte real dereferenciaria null aqui (States.cpp:692); nosso safety net devolve false em vez de arriscar"
        );
    };
    let has_resource_state = ES22_RESOURCE_STATE
        .lock()
        .map(|g| g.iter().any(|((e, r), _)| *e == entity_key && *r == part))
        .unwrap_or(false);
    if !has_resource_state {
        return format!(
            "[applyappearance] component={inst:p} finalAppearance={final_appearance:#018x} ({fontes}) part={part:#018x} MAS nenhum ResourceState vinculado (LinkPartToAppearanceName nunca chamado) -> nosso safety net devolve false"
        );
    }
    // ResolveName: nome dinâmico (`*...`) troca os atributos pelo estado da entidade (`#23`); nome comum passa igual.
    let nome_final = crate::cname::resolve_cname(final_appearance);
    let resolved = if nome_final.starts_with('*') {
        crate::cname::cname(&crate::aparencia::resolver_nome_da_entidade(reg, entity, &nome_final))
    } else {
        final_appearance
    };

    let before = component_get_appearance_name(inst, kind);
    let wrote = component_set_appearance_name(inst, kind, resolved);
    *escreveu = wrote;
    let refreshed = es20_refresh_medido(inst).map(|refresh| refresh(inst)).is_some();
    let after = component_get_appearance_name(inst, kind);
    format!(
        "[applyappearance] component={inst:p} kind={kind:?} nome='{name_str}' ({fontes}) finalAppearance={resolved:#018x} antes={before:#018x?} escrita_ok={wrote} refresh_ok={refreshed} depois={after:#018x?}"
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

/// `EntityState::ToggleConditionalComponents` (`States.cpp:535-592`) — a 2ª composição alvo
/// desta rodada. **Divergência de forma consciente, documentada**: a fonte real recebe
/// `Red::DynArray<Red::Handle<Red::IComponent>>&` POR PARÂMETRO (uma técnica de marshalling —
/// array de handles como ENTRADA de uma native — nunca exercitada nesta codebase, categoria de
/// risco genuinamente nova); em vez disso, esta função lê `Entity+0xA0` DIRETO (o mesmo
/// `ComponentsStorage.components` que `Entity.GetComponents()`/item Codeware `#24`, já fechado,
/// e `componentes_da_tag` abaixo já leem com sucesso) — semanticamente idêntico (a
/// fonte real É chamada com TODOS os componentes da entidade na prática, via
/// `GarmentAssembler`), sem inventar marshalling novo. Réplica fiel do 1º loop (classifica cada
/// componente via `ParseReference` em condicional/não-condicional, escreve `isEnabled =
/// MatchReference(...)` pros condicionais com estado vinculado) + o 2º loop de desempate
/// (`toggle_conditional_group_disable_indices` acima).
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

/// axl-fullbody-torso (2026-08-07, cont.42, continuação): 2º agente de RE offline achou que
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

// ===== Codeware `#254` (`Rendering.hpp` / `RenderProxy`, 2026-09-11) =====
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

// `SetMaterialOverride` (Codeware `#254` parte 3, 2026-09-11; RE em `proofs/20260911-codeware-254-setmaterialoverride-parte2-caminho-render-offline.log`):
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

/// `ArchiveXL.RegisterArchive(path: String) -> Bool` (`PENDENCIAS-UNIFICADAS.md`, Facade.hpp) —
/// achado de 2026-07-13 nunca implementado até agora: generaliza `axl-pathb-injection-arbitrary`
/// (já PROVADO ao vivo, `lib.rs::load_archive_into_group`) numa API chamável por QUALQUER mod em
/// runtime (ex. `OnAttach`), sem precisar que o nome do `.archive` bata o glob nativo.
pub(crate) unsafe extern "C" fn tramp_registrar_archive(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::arquivos_do_jogo::facade_register_archive(&path);
    crate::log_dev!("[reg] >>> ArchiveXL.RegisterArchive('{path}') -> {ok}");
    write_bool_ret(out, ok);
}

/// `ArchiveXL.RegisterDir(path: String) -> Bool` — registra todos os `.archive` de um diretório
/// (não-recursivo) via `facade_register_archive` um por um.
pub(crate) unsafe extern "C" fn tramp_registrar_pasta_de_archives(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let ok = crate::arquivos_do_jogo::facade_register_dir(&path);
    crate::log_dev!("[reg] >>> ArchiveXL.RegisterDir('{path}') -> {ok}");
    write_bool_ret(out, ok);
}

/// ArchiveXL `#27` (`PENDENCIAS-UNIFICADAS.md`, 2026-08-11) — `RegisterLink(path, link) -> Bool`:
/// via "oficial" redscript-callable de registrar 1 par de `resource.link` em runtime, mesma
/// tabela (`RESLINK_MAP`) que `resource.link`/`InitializeArchives` já usam — reusa
/// `selftest::reslink_path` (mecanismo PROVADO ao vivo desde 2026-07-13, `axl-localization-
/// apply`/etc.), zero RE nova. `path` = o alias que um mod quer que resolva pra `link` (o
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

/// ArchiveXL `#27`, metade leitura: `GetAliasCount(path) -> Int32` — quantos caminhos são o mesmo
/// recurso que `path`, nos dois sentidos, por `resource.link`/`RegisterLink` e por `resource.copy`
/// (`reslink_aliases_of`). Composto com `GetAliasHash` abaixo pra enumerar sem precisar de um
/// `array<T>`-retorno-de-native (a
/// categoria de risco que já CROU este projeto — `TweakXL #43`/`GetRecordsArray`, 2026-08-10 —
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

/// ArchiveXL `#27`, `GetAliasHash(path, index) -> Uint64` — devolve o HASH (não o path decodificado
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

/// `ArchiveXL.Reload() -> Void` — re-lê `bwms-reslink.txt`/`bwms-factories.txt` e as outras tabelas
/// geradas por `bwms install` a partir dos `.xl` ativos (`recarregar_tabelas_de_runtime`) sem precisar rebootar,
/// mesmo padrão já fechado de `TweakXL.Reload` (reusa `apply_mod_tweaks()`). Útil depois de editar
/// um `.xl` de mod ativo em disco + regerar as tabelas via `bwms install`. As tabelas em memória viram as dos arquivos
/// (`substituir`): o par de `RegisterLink` sai, como no `Configure` da referência.
pub(crate) unsafe extern "C" fn tramp_recarregar_tabelas_de_runtime(_c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    crate::log_dev!("[reg] >>> ArchiveXL.Reload chamado — recarregando reslink/factory/patch");
    crate::mods_do_usuario::pipeline::recarregar_tabelas_de_runtime(true);
}

/// ArchiveXL #97 (2026-08-11, achado direto após fechar #32): `Facade::GetBodyType(aPuppet)` —
/// fonte real (`App/Facade.cpp:18-21`) é um repasse de 1 linha pra
/// `PuppetStateExtension::GetBodyType(aPuppet)`, A MESMA função já lida por completo pra fechar
/// o item #32 (`PuppetStateSystem`). Já confirmado por leitura de `Extension.cpp:142-190`:
/// `if (!aPuppet || s_bodyTags.empty()) return s_baseBodyType;` — `s_bodyTags` só é populado pelo
/// `.xl` `player.bodyTypes`, nunca wired num runtime consumido por este projeto, logo SEMPRE
/// vazio aqui — `GetBodyType` é DETERMINÍSTICO, sempre devolve `s_baseBodyType` = `"BaseBody"`
/// (`BaseBodyName`, `Extension.hpp:18`), independente do puppet passado. Zero endereço nativo
/// novo, zero RE — mesma bookkeeping-closure category já usada váras vezes nesta sessão.
/// `puppet` (arg) é ignorado de propósito (a precondição do projeto já garante o resultado).
pub(crate) unsafe extern "C" fn tramp_tipo_de_corpo(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // Drena o param `puppet` do frame (mesmo se não usado) — mesma disciplina de qualquer
    // native argful deste projeto, evita deixar o cursor da CScriptStackFrame desalinhado.
    let func = current_native_func();
    let _ = rtti::read_params_consuming(func, frame);
    let h = crate::cname::intern("BaseBody");
    crate::log_dev!("[reg] >>> ArchiveXL.GetBodyType chamado -> 'BaseBody' (determinístico, s_bodyTags sempre vazio neste projeto)");
    write_uint_ret(out, h, 8);
}

/// `BwmsAparencia.IsDynamicValue(value: String) -> Bool` — checagem trivial (marcador `*` inicial),
/// mesma semântica das 5 sobrecargas C++ reais (todas colapsam pra isso, `Dynamic.cpp:674-697`).
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

/// ArchiveXL `#23` (`DynamicAppearanceController::ResolveName`/`ResolvePath`, `Dynamic.cpp:341-398`) pro redscript, com assinatura reduzida
/// `(entity, valor)`: o estado dinâmico da entidade (sufixos pelo `#9`, customização e cor do cabelo) é calculado na hora por
/// `selftest::resolver_nome_da_entidade`, e o `ProcessString` troca os atributos (`*t_{gender}_{camera}`). Divergências: sem o `aVariant` da
/// referência (locais vazios) e sem item equipado no estado — a referência guarda o último `UpdateState` da entidade, feito por item de roupa.
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

/// ArchiveXL `#20` (`GarmentExtension::IsUniqueAppearanceName`, `Garment/Extension.cpp:909`,
/// `PENDENCIAS-UNIFICADAS.md` item `#20`, 2026-08-12 round 3) — porte 1:1, ZERO RE/ZERO endereço
/// nativo: `return aName && aName != DefaultAppearanceName && aName != RandomAppearanceName &&
/// aName != EmptyAppearanceName;`. As 3 constantes (`Red::CName("default")`/`Red::CName("random")`/
/// `Red::CName("empty_appearance_default")`) confirmadas por grep no C++ real — o MESMO sentinela
/// `EmptyAppearanceName` aparece idêntico em `Extension.cpp:15`/`Attachment/Extension.cpp:23`/
/// `PuppetState/Handler.cpp:34` (3 arquivos diferentes, mesma constante). Decide se um nome de
/// aparência é "único" (vale consultar Dynamic Appearance pra ele) vs. um dos 3 placeholders —
/// usado nos 2 call-sites reais de `SelectDynamicAppearance`/`MarkDynamicAppearanceName`
/// (`Extension.cpp:186`/`762`) como guarda de entrada.
pub(crate) fn is_unique_appearance_name(hash: u64) -> bool {
    hash != 0
        && hash != crate::cname::cname("default")
        && hash != crate::cname::cname("random")
        && hash != crate::cname::cname("empty_appearance_default")
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

/// `DynamicAppearanceController::GetBaseAppearanceName` (`Dynamic.cpp:768-779`) — substring ANTES
/// do primeiro marcador de QUALQUER tipo (`!`/`%`/`&`) — mais amplo que só `!` (o campo `name` de
/// `DynamicAppearanceName::parse` só corta no `!`). Implementação genuinamente nova (item `#23`).
pub(crate) fn get_base_appearance_name(name: &str) -> &str {
    match name.find(['!', '%', '&']) {
        Some(pos) => &name[..pos],
        None => name,
    }
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

/// `DynamicAppearanceController::MatchReference` (`Dynamic.cpp:316-339`) — composição PURA:
/// `entity_conditions=None` reproduz fielmente o ramo `m_states.find(aEntity)==m_states.end()`
/// da fonte real (entidade sem estado registrado) — sempre `false` se a referência tiver
/// condições, independente da variante bater. Decisão de escopo consciente: a via
/// redscript-facing abaixo (`AppearanceMatchesReference`) usa SEMPRE este ramo (sem acoplar a
/// `EntityState`/`#22`, que vive num registry global separado) — decisivo e 100% autocontido
/// (não depende de nenhum estado de entidade/save real), mesmo sendo um subconjunto da API C++
/// completa.
pub(crate) fn appearance_matches_reference(reference: &DynamicAppearanceRef, appearance: &DynamicAppearanceName) -> bool {
    appearance_matches_reference_com(reference, appearance, None)
}

/// `MatchReference` com o estado da entidade (22F): as condições da referência precisam estar nas condições da entidade (`atributo=valor` do `#23`) ou
/// nos overrides da aparência; `None` = entidade sem estado, e referência com condição não casa.
pub(crate) fn appearance_matches_reference_com(
    reference: &DynamicAppearanceRef,
    appearance: &DynamicAppearanceName,
    condicoes: Option<&std::collections::BTreeSet<u64>>,
) -> bool {
    if !reference.variants.is_empty() && !reference.matches_variant(appearance.variant) {
        return false;
    }
    if !reference.conditions.is_empty() {
        let Some(presentes) = condicoes else { return false };
        if !reference.matches_conditions_with_overrides(presentes, &appearance.overrides) {
            return false;
        }
    }
    true
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
/// variante, +1 por condição, truncado em `int8` como `Dynamic.cpp:263`) já usado internamente pelo ArchiveXL real pra escolher entre
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

/// ArchiveXL `#48` (`MeshExtension::ExpandResourcePath`, `Mesh/Extension.cpp:900-925`, catálogo
/// item `#48`, 2026-08-12 round 3) — reusa o núcleo `is_dynamic_value`/
/// `process_dynamic_string_single_attr` (item `#21`/`#23`, já wired ao dylib mais cedo hoje) com
/// o MESMO atributo LOCAL pré-computado que a fonte real injeta (`material`, `Red::CName
/// ("material")`, `Mesh/Extension.cpp:15`, `MaterialAttr`). A referência chama o mesmo
/// `ProcessString` do `#23` com esse atributo local (`Mesh/Extension.cpp:913`), então o núcleo
/// é o de `process_dynamic_string_com`, sem cópia. Fiel ao par
/// `(ResourcePath, bool)` real:
/// - `pathStr` não-dinâmico → devolve `pathStr` inalterado (mesmo `{aPath, false}` da fonte);
/// - processamento inválido (chave sem fechar/nenhum atributo na string) → devolve string vazia
///   (convenção "vazio = falhou" já usada em `ProcessDynamicString`);
/// - atributo faltando (`missed`) → devolve string vazia (mesma convenção);
/// - sucesso → devolve o path expandido.
/// Divergência de escopo consciente:
/// a fonte real interna o resultado via `ResourcePathRegistry::RegisterPath` (produz um
/// `ResourcePath` hash novo, side-effect num registro global nativo); aqui devolvemos a STRING
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
/// contagem de `ResourceState`s afetados (diagnóstico; a fonte real é `void`, expor a contagem é
/// zero-custo e confirma o round-trip num teste ao vivo sem precisar de um 2º comando).
pub(crate) unsafe extern "C" fn tramp_remover_offsets_de_roupa_do_mod(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mod_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let n = if rtti::sane(entity) { remove_offset_overrides(entity as u64, mod_hash) } else { 0 };
    crate::log_dev!(
        "[reg] >>> BwmsAparencia.RemoveOffsetOverrides(entity={entity:p}, modHash={mod_hash:#018x}) -> resourceStates afetados={n}"
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

/// Os 8 métodos da fachada de script do ArchiveXL (`Facade.hpp:25-32`), o nome que mod do Windows chama. Declarados em
/// `ponte-fachada-de-recursos.reds`.
const METODOS_ARCHIVEXL: [(&str, NativeHandler); 8] = [
    ("Version", tramp_version::<PLUGUE_ARCHIVEXL>),
    ("Require", tramp_require::<PLUGUE_ARCHIVEXL>),
    ("RegisterArchive", tramp_registrar_archive),
    ("RegisterDir", tramp_registrar_pasta_de_archives),
    ("Reload", tramp_recarregar_tabelas_de_runtime),
    ("EnableGarmentOffsets", tramp_offsets_de_roupa_ligar),
    ("DisableGarmentOffsets", tramp_offsets_de_roupa_desligar),
    ("GetBodyType", tramp_tipo_de_corpo),
];

pub unsafe fn registrar_fachada_de_recursos(reg: &Registry) -> String {
    register_facade_methods(reg, "ArchiveXL", &METODOS_ARCHIVEXL)
}

/// As funções de aparência que o BWMS criou (aliases de `resource.link`, aparência e caminho dinâmicos, tags visuais, override de
/// aparência por componente e deslocamento de roupa). A referência não as expõe a script e nenhum mod do Windows as chama, então moram
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
    // Rodada 15 (2026-08-17), item `#22` — `EntityState::ApplyAppearanceOverride`/
    // `ToggleConditionalComponents` (ver seção grande "ArchiveXL `#22`" acima neste
    // arquivo, logo antes do Codeware `#58`). `LinkComponentToPart`/
    // `LinkPartToAppearanceName`/`AddAppearanceOverrideForComponent` são o setup
    // (equivalentes aos métodos públicos homônimos de `EntityState` que constroem o
    // bookkeeping consumido pelos 2 métodos de aplicação).
    ("LinkComponentToPart", tramp_ligar_componente_a_parte),
    ("LinkPartToAppearanceName", tramp_ligar_parte_a_aparencia),
    ("AddAppearanceOverrideForComponent", tramp_adicionar_aparencia_do_componente),
    ("ApplyAppearanceOverride", tramp_aplicar_aparencia_do_componente),
    ("ToggleConditionalComponents", tramp_alternar_componentes_condicionais),
    // Item `#67`/`ComputePlayerGarment` (2026-08-16) — `EntityState::AddOffsetOverride`/
    // `RemoveOffsetOverrides`/`GetOffsetOverride` (ver seção grande "ArchiveXL #67" logo
    // acima, imediatamente antes desta função). Zero mutação de memória nativa — só
    // registry próprio do BWMS; a leitura de `ComputePlayerGarment.resources`/`.offsets`
    // (via probe observe-only) e a ESCRITA de volta em `aOffsets` ficam pra rodada futura
    // com boot ao vivo.
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
    use super::{METODOS_ARCHIVEXL, METODOS_BWMSAPARENCIA};
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

    /// Sob `ArchiveXL` ficam só os 8 da fachada da referência (`Facade.hpp:25-32`); o que o BWMS criou vai para `BwmsAparencia`.
    #[test]
    fn archivexl_so_com_os_metodos_da_referencia() {
        let referencia: BTreeSet<String> =
            ["Version", "Require", "RegisterArchive", "RegisterDir", "Reload", "EnableGarmentOffsets", "DisableGarmentOffsets", "GetBodyType"]
                .iter()
                .map(|n| n.to_string())
                .collect();
        assert_eq!(registradas(&METODOS_ARCHIVEXL), referencia);
        let bwms = registradas(&METODOS_BWMSAPARENCIA);
        assert_eq!(bwms.len(), METODOS_BWMSAPARENCIA.len(), "nome repetido em `BwmsAparencia`");
        assert!(bwms.is_disjoint(&referencia), "método da referência registrado em `BwmsAparencia`: {:?}", bwms.intersection(&referencia).collect::<Vec<_>>());
    }

    /// Native declarada no `.reds` e não registrada derruba o bind (SIGTRAP); registrada e não declarada fica inalcançável. Sem a pasta
    /// `blackwall-mods-dev/` em nenhum ancestral (a suíte do export para o GitHub), não há o que comparar.
    #[test]
    fn declaradas_no_reds_batem_com_as_registradas() {
        for (arquivo, classe, metodos) in [
            ("ponte-fachada-de-recursos.reds", "ArchiveXL", &METODOS_ARCHIVEXL[..]),
            ("bwms-aparencia.reds", "BwmsAparencia", &METODOS_BWMSAPARENCIA[..]),
        ] {
            let Some(no_reds) = declaradas(arquivo, classe) else { return };
            assert_eq!(no_reds, registradas(metodos), "{arquivo}: `{classe}` declarada × registrada");
        }
    }
}

/// Alguém ouve `Component/Toggle`/`Vehicle/ToggleAuxLights`: os hooks do `Toggle` e do `ToggleLights` só leem estes atômicos.
pub static COMPONENT_TOGGLE_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Codeware `#127`: `Component/Toggle` com o componente e a entidade dona, só com dono, como o `ComponentToggleHook` da referência. O evento sai do
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
// Fonte real: `Callback/Targets/ComponentTarget.reds` — `public static native func ID(id: CRUID)
// -> ref<ComponentTarget>` / `Name(name: CName) -> ref<ComponentTarget>`. Cada fábrica lê 1 arg
// escalar (CRUID/CName = u64 puro, mesmo tratamento de `register_hash_wrapper_utils`) e devolve
// uma instância NOVA da PRÓPRIA classe carregando o critério (consumido depois pelo filtro
// "permissivo" de `CallbackSystemHandler.AddTarget`/`HasTarget` — não filtra fundo hoje).
//
// Mecanismos que as fábricas reusam, cada um com prova própria:
//   (1) `is_static=true` LENDO um arg via `read_params_consuming` dentro de um MÉTODO DE CLASSE
//       real registrado por `register_method(..., true)` — `Codeware.Require(version: String)`
//       (`tramp_require`, `registrar_fachada_de_versao`), PROVADO ao vivo (Facade fechada 2026-07-13,
//       boot completo até GAMEPLAY REAL). MAS a classe "Codeware" ali vem do `.reds` (compilador
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
//       só na classe "Codeware" NÃO-forjada.
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
// como campo já-populado lido de objetos reais). `cw-event-target-classes` fecha a família inteira.
/// Codeware `#132` (`ComponentTarget::Matches`, 2026-08-12): TAG de qual das 2 factories construiu
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

/// `EntityComponentEvent extends EntityLifecycleEvent` — `cw-event-target-classes` (2026-07-24).
/// Fonte real: `public native class EntityComponentEvent extends EntityLifecycleEvent {
/// GetComponent()->wref<IComponent> }`. **Divergência FECHADA (2026-08-11, item Codeware `#9`)**:
/// `wref<T>` agora usa `rtti::make_weak_handle` (mesma técnica do RED4ext.SDK `#387`), assinatura
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
    // Codeware `#9` (2026-08-11): `wref<IComponent>` REAL via `rtti::make_weak_handle` — mesma
    // técnica que fechou RED4ext.SDK `#387` pra `EntityLifecycleEvent.GetEntity()`. Substitui o
    // `write_uint_ret` cru (nem escrevia os 16 bytes completos de um Handle) documentado como
    // "ref forte" — na real nunca tinha refcount block nenhum, categoria de risco do mesmo tipo
    // já corrigida pro `#387`.
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

/// Constrói um `EntityComponentEvent` com o componente e a entidade dona (`GetEntity` do evento), como o `ComponentToggleHook` da referência.
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

#[cfg(feature = "devtools")]
pub(crate) const VCC_ESPERA_MS: u64 = 5000;

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

/// Codeware `#130` (`Entity/Reassemble`): copia pro array de componentes novos da troca de aparência os componentes que os ouvintes somaram em
/// `entity+0xA0` durante o despacho (do índice `antes` em diante), cada um com referência forte própria — como o `OnRequestComponents` da
/// referência. Devolve quantos copiou.
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

/// axl-fullbody-torso (2026-08-07, cont.41, continuação): `toggle` sozinho (escreve `enabled=1`)
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
    /// Nenhum dos 4 tipos suportados — `ComponentWrapper::IsMeshComponent()` seria `false`.
    Unsupported,
}

/// Nome da property RTTI de cada campo por tipo. O offset vem do `CProperty::valueOffset` da classe real
/// (`prop_value_offset`), nunca de tabela: a tabela antiga copiava o header do RED4ext.SDK, que erra a família skinned
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

/// Grava o `chunkMask` do componente de mesh (offset do RTTI); falso fora dos tipos de mesh.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn componente_mascara_gravar(reg: &crate::rtti::Registry, inst: *mut c_void, valor: u64) -> bool {
    let kind = detect_garment_component_kind(reg, inst);
    kind != GarmentComponentKind::Unsupported && component_set_chunk_mask(inst, kind, valor)
}

/// A#22 22D — `EntityState::ApplyChunkMaskOverride` composto: os 2 retornos cedo da referência (`isEnabled` em `+0x8b`, componente de mesh), a máscara
/// do componente e a do prefixo na ordem da aplicação real (`override_state::mascara_aplicada`: OR do mostrar, depois AND do esconder) sobre a original,
/// e a escrita no `chunkMask`. `None` = a referência voltaria falso sem escrever (desligado, fora de mesh, sem mudança de máscara ou escrita recusada).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn aplicar_mascara_composta(
    reg: &crate::rtti::Registry,
    inst: *mut c_void,
    original: u64,
    componente: Option<&bwms_primitives::override_state::ComponentState>,
    prefixo: Option<&bwms_primitives::override_state::ComponentState>,
) -> Option<u64> {
    if inst.is_null() || (inst as *const u8).add(0x8b).read_unaligned() == 0 {
        return None;
    }
    let kind = detect_garment_component_kind(reg, inst);
    if kind == GarmentComponentKind::Unsupported {
        return None;
    }
    let fim = bwms_primitives::override_state::mascara_aplicada(|| original, componente, prefixo)?;
    component_set_chunk_mask(inst, kind, fim).then_some(fim)
}

/// Entrada de uma tag: `(CName do componente ou do prefixo, mostrar, máscara)`. O `mostrar` é o `chunkMask.set` que o
/// `AddChunkMaskOverride(hash, nome, mask, set)` recebe (`Extension.cpp:696`): duas fontes no mesmo componente somam o mostrar (OR) e o esconder
/// (AND), e sem ele o `s(.., &[0, 1, 2])` do HighHeels esconderia tudo fora desses 3 chunks.
pub(crate) type EntradaDeTag = (CName, bool, u64);

/// As 14 tags embutidas do `OverrideTagManager` da referência (`Tags.cpp:3-156`), `(CName da tag, entradas)`, calculadas na compilação
/// (`fnv1a64` e `mascara_de_chunks` são `const fn`). A busca é `tag_por_hash`; só a guarda do smoke (`es20_guardar_tags`) lê a tabela inteira,
/// porque guarda todos os nomes.
pub(crate) static TAGS_EMBUTIDAS: [(u64, &[EntradaDeTag]); 14] = {
    use bwms_primitives::fnv1a64;
    use bwms_primitives::override_state::mascara_de_chunks;
    const fn entrada(nome: &str, mostrar: bool, chunks: &[u8]) -> EntradaDeTag {
        match mascara_de_chunks(mostrar, chunks) {
            Some(mascara) => (CName::cname(nome), mostrar, mascara),
            // Só roda na avaliação da `static`: chunk acima de 63 nesta tabela é erro de compilação, nunca pânico no jogo.
            None => panic!("tabela embutida: chunk acima de 63"),
        }
    }
    const fn tag(nome: &str) -> u64 {
        fnv1a64(nome.as_bytes())
    }
    const fn ha(nome: &str) -> EntradaDeTag {
        entrada(nome, false, &[])
    }
    const fn h(nome: &str, chunks: &[u8]) -> EntradaDeTag {
        entrada(nome, false, chunks)
    }
    const fn s(nome: &str, chunks: &[u8]) -> EntradaDeTag {
        entrada(nome, true, chunks)
    }
    [
        (
            tag("hide_Head"),
            &[
                ha("h0_"),
                ha("he_"),
                ha("heb_"),
                ha("ht_"),
                ha("hx_"),
                ha("i1_"),
                ha("beard"),
                ha("beard_"),
                ha("MorphTargetSkinnedMesh3637"),
                ha("MorphTargetSkinnedMesh6675"),
                ha("MorphTargetSkinnedMesh7243"),
                ha("MorphTargetSkinnedMesh7561"),
            ],
        ),
        (tag("hide_Arms"), &[ha("a0_"), ha("left_arm"), ha("right_arm")]),
        (
            tag("hide_Torso"),
            &[
                ha("n0_"),
                ha("tx_"),
                h("t0_000_pma_base__full", &[0, 1, 2, 3]),
                ha("t0_000_pma_base__full_seamfix"),
                h("t0_000_pwa_base__full", &[0, 1, 2, 3]),
                ha("t0_000_pwa_base__full_seamfix"),
                h("t0_000_pwa_fpp__torso", &[0, 1, 2, 3]),
                ha("MorphTargetSkinnedMesh0531"),
            ],
        ),
        (
            tag("hide_LowerAbdomen"),
            &[h("t0_000_pma_base__full", &[3]), h("t0_000_pwa_base__full", &[3]), h("t0_000_pwa_fpp__torso", &[3])],
        ),
        (
            tag("hide_UpperAbdomen"),
            &[h("t0_000_pma_base__full", &[2]), h("t0_000_pwa_base__full", &[2]), h("t0_000_pwa_fpp__torso", &[2])],
        ),
        (
            tag("hide_CollarBone"),
            &[h("t0_000_pma_base__full", &[1]), h("t0_000_pwa_base__full", &[1]), h("t0_000_pwa_fpp__torso", &[1])],
        ),
        (
            tag("hide_Chest"),
            &[
                h("t0_000_pma_base__full", &[0]),
                h("t0_000_pwa_base__full", &[0]),
                h("t0_000_pwa_fpp__torso", &[0]),
                ha("MorphTargetSkinnedMesh0531"),
            ],
        ),
        (
            tag("hide_Legs"),
            &[
                ha("l0_"),
                ha("s0_"),
                h("t0_000_pma_base__full", &[4, 5, 6, 7]),
                h("t0_000_pwa_base__full", &[4, 5, 6, 7]),
                h("t0_000_pwa_fpp__torso", &[4, 5, 6, 7]),
            ],
        ),
        (
            tag("hide_Thighs"),
            &[h("t0_000_pma_base__full", &[4]), h("t0_000_pwa_base__full", &[4]), h("t0_000_pwa_fpp__torso", &[4])],
        ),
        (
            tag("hide_Calves"),
            &[
                h("t0_000_pma_base__full", &[5]),
                h("l0_000_pma_base__high_heels", &[0]),
                h("l0_000_pma_base__flat_shoes", &[0]),
                h("t0_000_pwa_base__full", &[5]),
                h("t0_000_pwa_fpp__torso", &[5]),
                h("l0_000_pwa_base__cs_flat", &[0]),
                h("l0_000_pwa_base__high_heels", &[0]),
                h("l0_000_pwa_base__flat_shoes", &[0]),
            ],
        ),
        (
            tag("hide_Ankles"),
            &[
                ha("s0_"),
                h("t0_000_pma_base__full", &[6]),
                h("l0_000_pma_base__high_heels", &[1]),
                h("l0_000_pma_base__flat_shoes", &[1]),
                h("t0_000_pwa_base__full", &[6]),
                h("t0_000_pwa_fpp__torso", &[6]),
                h("l0_000_pwa_base__cs_flat", &[1]),
                h("l0_000_pwa_base__high_heels", &[1]),
                h("l0_000_pwa_base__flat_shoes", &[1]),
            ],
        ),
        (
            tag("hide_Feet"),
            &[
                h("t0_000_pma_base__full", &[7]),
                h("l0_000_pma_base__high_heels", &[2]),
                h("l0_000_pma_base__flat_shoes", &[2]),
                h("t0_000_pwa_base__full", &[7]),
                h("t0_000_pwa_fpp__torso", &[7]),
                h("l0_000_pwa_base__cs_flat", &[2]),
                h("l0_000_pwa_base__high_heels", &[2]),
                h("l0_000_pwa_base__flat_shoes", &[2]),
            ],
        ),
        (
            tag("HighHeels"),
            &[
                h("t0_000_pma_base__full", &[5, 6, 7]),
                s("l0_000_pma_base__high_heels", &[0, 1, 2]),
                h("t0_000_pwa_base__full", &[5, 6, 7]),
                h("t0_000_pwa_fpp__torso", &[5, 6, 7]),
                s("l0_000_pwa_base__high_heels", &[0, 1, 2]),
            ],
        ),
        (
            tag("FlatShoes"),
            &[
                h("t0_000_pma_base__full", &[5, 6, 7]),
                s("l0_000_pma_base__flat_shoes", &[0, 1, 2]),
                h("t0_000_pwa_base__full", &[5, 6, 7]),
                h("t0_000_pwa_fpp__torso", &[5, 6, 7]),
                s("l0_000_pwa_base__flat_shoes", &[0, 1, 2]),
            ],
        ),
    ]
};

/// Entradas da tag embutida de `CName` `tag`. A busca é pelo hash, como o `GetOverrides(tag)` da referência: a tag dos `visualTags` de uma definição
/// não precisa ter texto no pool do motor (boot 197), e a native faz `cname(&tag)` uma vez. `None` = tag desconhecida (a referência devolve a
/// definição vazia).
pub(crate) fn tag_por_hash(tag: u64) -> Option<&'static [EntradaDeTag]> {
    TAGS_EMBUTIDAS.iter().find(|(t, _)| *t == tag).map(|(_, entradas)| *entradas)
}

/// Componentes de `Entity+0xA0` com estado na tabela da tag: o nome base ou o prefixo do nome (`chaves_do_componente`) é chave da tabela, como a
/// referência acha o estado do componente (`States.cpp:710-711`). As chaves "n0_"/"h0_"/"l0_"/"tx_"/… são prefixos: nenhum componente se chama
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

/// O `ApplyVisualTagOverrideForMod` sem o RTTI: cada entrada da tag entra na loja do es20 sob a fonte (tag, mod) (`es20_registrar_tag`, o
/// `AddChunkMaskOverride` de `Extension.cpp:692-700`) e `aplicar` roda em cada componente que bate. `(verificados, que batem, aplicados)`; tag
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
/// roda de novo em cada componente que bate. O estado fica com `ChangesChunkMask`, então componente sem fonte nenhuma recebe a original
/// (`States.cpp:713-717`). `(nomes com estado, nomes com algo da fonte, reaplicados)`.
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

/// `ApplyChunkMaskOverride` num componente da tag e, com `refresh`, o `UpdateRenderer` depois da escrita (`ApplyComponentOverrides` forçado,
/// `Extension.cpp:810-826`) pelo portão medido (`es20_refresh_medido`). `true` se escreveu.
unsafe fn es20_aplicar_mascara_da_tag(m: &Es20Malhas, entidade: u64, inst: *mut c_void, refresh: bool) -> bool {
    let escreveu = es20_aplicar_mascara(m, entidade, inst).is_some();
    if escreveu && refresh {
        if let Some(f) = es20_refresh_medido(inst) {
            f(inst);
        }
    }
    escreveu
}

/// `ApplyVisualTagOverrideForMod(WithRefresh)`: `tag_aplicar_na_loja` com o `ApplyChunkMaskOverride` do es20 (`isEnabled`, componente de mesh, estado
/// do nome base e do prefixo, original em `es20_originais`), a mesma loja e a mesma regra do pipeline do #20. Sem as classes de mesh no RTTI
/// (`es20_malhas`) nada entra na loja. `(verificados, que batem, escritos)`.
// TODO(A#22, sem prova ao vivo desta loja): a prova `proofs/2026-08-14-archivexl-21-22-tagoverride-multimod-PROVADO.log` não passa por
// `es20_componentes`. Falta 1 boot dev com o smoke `archivexl-tagoverride-multimod-smoke.reds`: veredito 9702 `consistente=true`, que relê o
// `chunkMask` de cada alvo a cada apply e remove, e o 9703 do "esconder vence" quando o V tiver `l0_*_high_heels`; o log vai para `proofs/`.
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

/// `ComponentState::m_part` (`GetAppearancePart`, `States.hpp:12-49`) por `(entidade, nome base)`: a parte de recurso a que o componente foi ligado
/// (`LinkComponentToPart`). Sem entrada = `IsFromAppearancePart()==false`. O nome base é `DynamicAppearanceRef::parse(nome_real).name`, não o hash cru
/// do `IComponent::name`, como a referência indexa (`componentRef.name`). Os overrides de aparência e de máscara do mesmo `ComponentState`, dos itens
/// e das natives de `BwmsAparencia`, ficam em `es20_componentes`.
pub(crate) static ES22_COMPONENT_APPEARANCE: Mutex<Vec<((u64, u64), u64)>> = Mutex::new(Vec::new());

/// A#22 22F — `LinkComponentToPart` pelo nome base (`componentRef.name`) direto do Rust. `true` se ligou.
pub(crate) fn es22_ligar_componente(entidade: u64, nome_base: u64, parte: u64) -> bool {
    let Ok(mut g) = ES22_COMPONENT_APPEARANCE.lock() else { return false };
    let chave = (entidade, nome_base);
    match g.iter_mut().find(|(k, _)| *k == chave) {
        Some((_, p)) => *p = parte,
        None => g.push((chave, parte)),
    }
    true
}

/// A#22 22F — `LinkPartToAppearance(parte, nome)` direto do Rust.
pub(crate) fn es22_ligar_parte(entidade: u64, parte: u64, aparencia: &str) {
    es22_ligar_parte_seletor(entidade, parte, &DynamicAppearanceName::parse(aparencia));
}

/// A#22 22H — `LinkPartToAppearance(parte, seletor)` com o seletor já lido.
pub(crate) fn es22_ligar_parte_seletor(entidade: u64, parte: u64, seletor: &DynamicAppearanceName) {
    if let Ok(mut g) = ES22_RESOURCE_STATE.lock() {
        let chave = (entidade, parte);
        match g.iter_mut().find(|(k, _)| *k == chave) {
            Some((_, s)) => *s = seletor.clone(),
            None => g.push((chave, seletor.clone())),
        }
    }
}

// ===== ArchiveXL `#22` 22C — índice do `OverrideStateManager` pelas 4 chaves (2026-09-14) =====
// Porte de `States.cpp:790-935`. `GetEntityState` cria o estado da entidade na 1ª vez e liga a ele os caminhos `<hash do TemplatePath>_0.app`, `_1.app` e
// `_2.app`; `LinkEntityToAssembler` liga o contexto de processamento só a entidade que já tem estado; `LinkEntityToPointer` liga o ponteiro do estado do
// montador (`GarmentAssemblerState.unk00`), criando o estado se preciso; a passada do `LinkComponentToPart` tira das entidades as expiradas (os outros 3
// mapas continuam apontando pra elas, como na referência); `ClearStates` esvazia tudo. Cada busca devolve a entidade dona do estado; os dados por
// componente e por parte seguem nos registros `ES22_*` por entidade. A entidade guarda o handle fraco dela, pra validade.
#[derive(Default)]
pub(crate) struct Es22Indice {
    /// `(entidade, handle fraco)` na ordem de criação.
    pub(crate) entidades: Vec<(u64, [u64; 2])>,
    pub(crate) por_caminho: std::collections::HashMap<u64, u64>,
    pub(crate) por_contexto: std::collections::HashMap<u64, u64>,
    pub(crate) por_ponteiro: std::collections::HashMap<u64, u64>,
}

/// Caminho dinâmico `k` (0 a 2) do template: o `ResourcePath` de `<hash decimal do template>_<k>.app`.
pub(crate) fn es22_caminho_dinamico(template: u64, k: u8) -> u64 {
    bwms_primitives::resource_path_hash(&format!("{template}_{k}.app"))
}

/// Esquece os estados de componente da entidade.
pub(crate) fn es20_esquecer(entidade: u64) {
    if let Ok(mut m) = es20_componentes().lock() {
        m.retain(|(e, _), _| *e != entidade);
    }
}

impl Es22Indice {
    /// `GetEntityState(entidade)`: a entidade, criando o estado e os 3 caminhos na 1ª vez; `None` sem entidade.
    pub(crate) fn estado(&mut self, entidade: u64, fraco: [u64; 2], template: u64) -> Option<u64> {
        if entidade == 0 {
            return None;
        }
        if !self.entidades.iter().any(|(e, _)| *e == entidade) {
            self.entidades.push((entidade, fraco));
            for k in 0..3 {
                self.por_caminho.insert(es22_caminho_dinamico(template, k), entidade);
            }
        }
        Some(entidade)
    }

    /// `GetFirstEntityState`.
    pub(crate) fn primeiro(&self) -> Option<u64> {
        self.entidades.first().map(|(e, _)| *e)
    }

    /// Entidades com estado.
    pub(crate) fn quantas(&self) -> usize {
        self.entidades.len()
    }

    /// `FindEntityState(Entity*)`.
    pub(crate) fn por_entidade(&self, entidade: u64) -> Option<u64> {
        (entidade != 0 && self.entidades.iter().any(|(e, _)| *e == entidade)).then_some(entidade)
    }

    /// `FindEntityState(ResourcePath)`.
    pub(crate) fn por_caminho(&self, caminho: u64) -> Option<u64> {
        self.por_caminho.get(&caminho).copied().filter(|_| caminho != 0)
    }

    /// `FindEntityState(GarmentProcessingContext*)`.
    pub(crate) fn por_contexto(&self, contexto: u64) -> Option<u64> {
        self.por_contexto.get(&contexto).copied().filter(|_| contexto != 0)
    }

    /// `FindEntityState(uintptr_t)`.
    pub(crate) fn por_ponteiro(&self, ponteiro: u64) -> Option<u64> {
        self.por_ponteiro.get(&ponteiro).copied().filter(|_| ponteiro != 0)
    }

    /// `LinkEntityToAssembler`: liga só entidade que já tem estado.
    pub(crate) fn ligar_contexto(&mut self, entidade: u64, contexto: u64) -> bool {
        let tem = self.por_entidade(entidade).is_some();
        if tem {
            self.por_contexto.insert(contexto, entidade);
        }
        tem
    }

    /// `LinkEntityToPointer`: liga, criando o estado se a entidade não tem.
    pub(crate) fn ligar_ponteiro(&mut self, entidade: u64, fraco: [u64; 2], template: u64, ponteiro: u64) {
        if self.estado(entidade, fraco, template).is_some() {
            self.por_ponteiro.insert(ponteiro, entidade);
        }
    }

    /// Passada do `LinkComponentToPart`/`UpdateDynamicAttributes`: `(entidades vivas, handles fracos das expiradas)`, com as expiradas fora das entidades.
    pub(crate) fn vivas(&mut self, valida: impl Fn([u64; 2]) -> bool) -> (Vec<u64>, Vec<[u64; 2]>) {
        let (vivas, expiradas): (Vec<_>, Vec<_>) = self.entidades.drain(..).partition(|(_, f)| valida(*f));
        self.entidades = vivas;
        (self.entidades.iter().map(|(e, _)| *e).collect(), expiradas.into_iter().map(|(_, f)| f).collect())
    }

    /// Handle fraco guardado da entidade.
    pub(crate) fn fraco(&self, entidade: u64) -> Option<[u64; 2]> {
        self.entidades.iter().find(|(e, _)| *e == entidade).map(|(_, f)| *f)
    }

    /// `ClearStates`: devolve os handles fracos pra soltar. Sem chamador no produto: falta o gatilho (TODO F2-04 no fim de `register.rs`).
    pub(crate) fn limpar(&mut self) -> Vec<[u64; 2]> {
        self.por_caminho.clear();
        self.por_contexto.clear();
        self.por_ponteiro.clear();
        self.entidades.drain(..).map(|(_, f)| f).collect()
    }
}

/// O índice do processo, como o `s_stateManager` da referência.
pub(crate) fn es22_indice() -> &'static Mutex<Es22Indice> {
    static INDICE: std::sync::OnceLock<Mutex<Es22Indice>> = std::sync::OnceLock::new();
    INDICE.get_or_init(|| Mutex::new(Es22Indice::default()))
}

pub(crate) fn es20_componentes() -> &'static Mutex<Es20Estados> {
    static ESTADOS: std::sync::OnceLock<Mutex<Es20Estados>> = std::sync::OnceLock::new();
    ESTADOS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// `EntityState::AddChunkMaskOverride(hash, nome, máscara, mostrar)`.
pub(crate) fn es20_adicionar_mascara(entidade: u64, hash: u64, nome: CName, mascara: u64, mostrar: bool) {
    if nome.valor() == 0 {
        return;
    }
    if let Ok(mut m) = es20_componentes().lock() {
        let st = m.entry((entidade, nome.valor())).or_default();
        if mostrar {
            st.mostrar(hash, mascara);
        } else {
            st.esconder(hash, mascara);
        }
    }
}

/// As máscaras de uma tag (`GetOverrides(tag)`) sob `hash`, como o `RegisterComponentOverrides` faz com cada tag visual de um item
/// (`Extension.cpp:692-700`).
pub(crate) fn es20_registrar_tag(entidade: u64, hash: u64, tabela: &[EntradaDeTag]) {
    for (nome, mostrar, mascara) in tabela {
        es20_adicionar_mascara(entidade, hash, *nome, *mascara, *mostrar);
    }
}

/// `RemoveChunkMaskOverrides(hash)` só nos nomes de uma tag (`tag_por_hash`): `(nomes com estado, nomes com algo do hash)`. A aparência e os outros
/// nomes ficam; `es20_remover_componentes` tira máscara e aparência de todo componente da entidade. O estado fica mesmo sem fonte: o
/// `ChangesChunkMask` faz a próxima aplicação gravar a original.
pub(crate) fn es20_remover_mascara(entidade: u64, hash: u64, tabela: &[EntradaDeTag]) -> (usize, usize) {
    let Ok(mut m) = es20_componentes().lock() else { return (0, 0) };
    let (mut com_estado, mut removidos) = (0, 0);
    for (nome, _, _) in tabela {
        if let Some(st) = m.get_mut(&(entidade, nome.valor())) {
            com_estado += 1;
            removidos += usize::from(st.remover_fonte(hash));
        }
    }
    (com_estado, removidos)
}

/// `RemoveChunkMaskOverrides(hash)` + `RemoveAppearanceOverrides(hash)` nos componentes da entidade; devolve em quantos havia algo do hash.
pub(crate) fn es20_remover_componentes(entidade: u64, hash: u64) -> usize {
    let Ok(mut m) = es20_componentes().lock() else { return 0 };
    let mut n = 0;
    for ((e, _), st) in m.iter_mut() {
        if *e == entidade {
            let mascara = st.remover_fonte(hash);
            let aparencia = st.remover_aparencia(hash);
            n += usize::from(mascara || aparencia);
        }
    }
    n
}

/// Estado do componente da entidade (cópia), se algum item registrou nele.
pub(crate) fn es20_estado(entidade: u64, nome: u64) -> Option<bwms_primitives::override_state::ComponentState> {
    es20_componentes().lock().ok()?.get(&(entidade, nome)).cloned()
}

/// Resultado do `RegisterComponentOverrides`: `partsOverrides` sem recurso, componentes registrados e tags conhecidas.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Es20Registro {
    pub partes_sem_recurso: usize,
    pub componentes: usize,
    pub tags: usize,
}

/// Um componente de `appearancePartComponentOverrides` (`componentName`, `meshAppearance`, `chunkMask`), lido da memória do motor por
/// `es20_componentes_de_overrides`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Es20OverrideDeComponente {
    pub nome: CName,
    pub aparencia: CName,
    pub mascara: u64,
}

/// `(overrides de parte sem recurso, componentes deles)` de um `DynArray<AppearancePartOverrides>` (0x18 B cada, os componentes 0x50 B), com os
/// campos já resolvidos (thread de job, sem RTTI).
pub(crate) unsafe fn es20_componentes_de_overrides(campos: [usize; 3], overrides: *const u8) -> (usize, Vec<Es20OverrideDeComponente>) {
    let [o_nome, o_aparencia, o_mascara] = campos;
    let b = rd_u64(overrides as *const c_void, 0) as *const u8;
    let n = if b.is_null() { 0 } else { (overrides.add(0x0c) as *const u32).read_unaligned() as usize };
    let (mut partes, mut lista) = (0, Vec::new());
    for i in 0..n.min(4096) {
        let parte = b.add(i * 0x18);
        if (parte as *const u64).read_unaligned() != 0 {
            continue;
        }
        partes += 1;
        let cb = rd_u64(parte.add(8) as *const c_void, 0) as *const u8;
        let cn = if cb.is_null() { 0 } else { (parte.add(0x14) as *const u32).read_unaligned() as usize };
        for k in 0..cn.min(4096) {
            let ler = |o: usize| (cb.add(k * 0x50 + o) as *const u64).read_unaligned();
            lista.push(Es20OverrideDeComponente {
                nome: CName::lido_do_motor(ler(o_nome)),
                aparencia: CName::lido_do_motor(ler(o_aparencia)),
                mascara: ler(o_mascara),
            });
        }
    }
    (partes, lista)
}

/// `RegisterComponentOverrides(estado, hash, overrides)` com os componentes já lidos: a máscara como esconder e a aparência.
pub(crate) fn es20_registrar_lista(entidade: u64, hash: u64, lista: &[Es20OverrideDeComponente]) {
    for c in lista {
        es20_adicionar_mascara(entidade, hash, c.nome, c.mascara, false);
        es20_adicionar_aparencia(entidade, hash, c.nome, c.aparencia);
    }
}

/// `RegisterComponentOverrides(estado, hash, definição)` com os campos de `appearancePartComponentOverrides` já resolvidos (thread de job, sem RTTI).
pub(crate) unsafe fn es20_registrar_componentes_com(campos: [usize; 3], entidade: u64, hash: u64, definicao: *const u8) -> Es20Registro {
    let (partes_sem_recurso, lista) = es20_componentes_de_overrides(campos, definicao.add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF));
    es20_registrar_lista(entidade, hash, &lista);
    let mut r = Es20Registro { partes_sem_recurso, componentes: lista.len(), tags: 0 };
    let tags = definicao.add(crate::jogo::aparencia::APPDEF_VISUALTAGS_OFF);
    let tb = rd_u64(tags as *const c_void, 0) as *const u64;
    let tn = if tb.is_null() { 0 } else { (tags.add(0x0c) as *const u32).read_unaligned() as usize };
    for i in 0..tn.min(256) {
        if let Some(tabela) = tag_por_hash(tb.add(i).read_unaligned()) {
            r.tags += 1;
            es20_registrar_tag(entidade, hash, tabela);
        }
    }
    r
}

/// `UpdatePartAttributes(estado, definição)`: cada parte ligada à aparência de nome igual ao da definição; devolve quantas.
pub(crate) unsafe fn es20_atualizar_partes(entidade: u64, definicao: *const u8) -> usize {
    let nome = crate::cname::resolve_cname((definicao.add(crate::jogo::aparencia::APPDEF_NAME_OFF) as *const u64).read_unaligned());
    let partes = es20_partes(definicao);
    for parte in &partes {
        es22_ligar_parte(entidade, *parte, &nome);
    }
    partes.len()
}

/// `UnregisterComponentOverrides` + `UnregisterOffsetOverrides` do item: `(componentes com algo dele, recursos com deslocamento dele)`.
pub(crate) unsafe fn es20_desregistrar(entidade: u64, hash: u64) -> (usize, u32) {
    (es20_remover_componentes(entidade, hash), remove_offset_overrides(entidade, hash))
}

// ===== ArchiveXL `#20` 20c-2 — `UpdatePartAssignments`, `ApplyComponentOverrides` e `ApplyOffsetOverrides` (2026-09-14) =====
// Porte de `Extension.cpp:739-881` sobre `States.cpp:390-399` e `659-755`, pro `ComputePlayerGarment` (thread de job). Classes e offsets dos tipos de
// componente de mesh saem do RTTI na thread do jogo (`es20_resolver_malhas`); no job, `class_of` é o `GetType` virtual do próprio objeto e `class_is_kind_of` só
// anda a cadeia de pais. Os originais ficam por entidade e `GetUniqueId` (`FNV1a64(appearancePath, FNV1a64(appearanceName, name))` do `IComponent`), lidos
// na 1ª vez que a referência pede. O `ResolveName` do apply precisa do estado dinâmico do `UpdateDynamicAttributes` (20f); sem ele o nome segue igual, como
// a referência sem entrada em `m_states`.
pub(crate) struct Es20Malhas {
    /// `entMeshComponent`, `entMorphTargetSkinnedMeshComponent`, `entGarmentSkinnedMeshComponent`, `entSkinnedMeshComponent` (ordem do `ComponentWrapper`).
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

/// Classes e offsets de `Es20Malhas` pelo RTTI (thread do jogo). `appearanceName`/`appearancePath` sem propriedade no RTTI caem nos offsets do SDK
/// (`0x48`/`0x68`); o `bool` diz se vieram do RTTI.
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

/// Tipo de mesh do componente na ordem do `ComponentWrapper` (0 mesh, 1 família skinned, 2 morph target); `None` = `IsMeshComponent` falso.
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

/// `ComponentWrapper::GetUniqueId`: `name` como semente, depois os 8 bytes de `appearanceName` e os de `appearancePath`.
pub(crate) unsafe fn es20_id_do_componente(base: (usize, usize), inst: *const u8) -> u64 {
    let ler = |o: usize| (inst.add(o) as *const u64).read_unaligned();
    let id = bwms_primitives::fnv1a64_seeded(&ler(base.0).to_le_bytes(), ler(0x40));
    bwms_primitives::fnv1a64_seeded(&ler(base.1).to_le_bytes(), id)
}

/// `(entidade, id do componente, 0 máscara / 1 aparência / 2 caminho do recurso)` → original.
pub(crate) fn es20_originais() -> &'static Mutex<std::collections::HashMap<(u64, u64, u8), u64>> {
    static ORIGINAIS: std::sync::OnceLock<Mutex<std::collections::HashMap<(u64, u64, u8), u64>>> = std::sync::OnceLock::new();
    ORIGINAIS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// `GetOriginalChunkMask`/`GetOriginalAppearance`: o valor guardado, ou `atual` guardado na 1ª vez.
pub(crate) fn es20_original(entidade: u64, id: u64, qual: u8, atual: impl FnOnce() -> u64) -> u64 {
    es20_originais().lock().map(|mut m| *m.entry((entidade, id, qual)).or_insert_with(atual)).unwrap_or(0)
}

/// `GetOriginalAppearance` das natives de `BwmsAparencia`, no cache do es20 (`es20_original` 1). Leitura que falha devolve 0 e fica fora do cache: guardada,
/// a recusa valeria para sempre naquele componente.
pub(crate) unsafe fn es20_original_da_aparencia(base: (usize, usize), entidade: u64, inst: *mut c_void, kind: GarmentComponentKind) -> u64 {
    let Some(atual) = component_get_appearance_name(inst, kind) else { return 0 };
    es20_original(entidade, es20_id_do_componente(base, inst as *const u8), 1, || atual)
}

/// `(entidade, parte)` → nomes dos componentes ligados a ela (`m_partComponents`, o `GetPartComponentNames` do 20d).
pub(crate) fn es20_componentes_da_parte() -> &'static Mutex<std::collections::HashMap<(u64, u64), std::collections::BTreeSet<u64>>> {
    static PARTES: std::sync::OnceLock<Mutex<std::collections::HashMap<(u64, u64), std::collections::BTreeSet<u64>>>> = std::sync::OnceLock::new();
    PARTES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// Nomes dos componentes ligados à parte na entidade.
pub(crate) fn es20_nomes_da_parte(entidade: u64, parte: u64) -> Vec<u64> {
    es20_componentes_da_parte().lock().ok().and_then(|m| m.get(&(entidade, parte)).map(|s| s.iter().copied().collect())).unwrap_or_default()
}

/// Parte ligada ao nome base do componente na entidade (`ComponentState::GetAppearancePart`).
pub(crate) fn es22_parte_do_componente(entidade: u64, nome_base: u64) -> Option<u64> {
    ES22_COMPONENT_APPEARANCE.lock().ok()?.iter().find(|(k, _)| *k == (entidade, nome_base)).map(|(_, parte)| *parte)
}

/// `EntityState::LinkComponentToPart(componente, parte)`: o nome base (`ParseReference(name).name`) ligado à parte e o nome do componente na lista
/// da parte. Devolve o nome base. O estado do componente não é criado aqui: o vazio da referência não muda nenhuma aplicação.
pub(crate) unsafe fn es20_ligar_componente(entidade: u64, inst: *const u8, parte: u64) -> u64 {
    let base = es20_nome_base(inst);
    es22_ligar_componente(entidade, base, parte);
    if let Ok(mut m) = es20_componentes_da_parte().lock() {
        m.entry((entidade, parte)).or_default().insert((inst.add(0x40) as *const u64).read_unaligned());
    }
    base
}

/// Nome base do componente (`ParseReference(name).name`).
pub(crate) unsafe fn es20_nome_base(inst: *const u8) -> u64 {
    parse_reference_live((inst.add(0x40) as *const u64).read_unaligned()).name
}

/// As duas chaves com que a referência acha o estado do componente de nome `nome` (`IComponent::name@0x40`): o nome base
/// (`ParseReference(name).name`, `States.cpp:710`) e o prefixo do nome (`GetPrefix(name)`, `States.cpp:711`; `Prefix.cpp:3-27`).
pub(crate) unsafe fn chaves_do_componente(nome: u64) -> (u64, Option<u64>) {
    (parse_reference_live(nome).name, component_prefix_of(&crate::cname::name_of(nome).unwrap_or_default()))
}

/// Estados do nome base do componente e do prefixo do nome dele (`FindComponentState(componentRef.name)` e `FindComponentState(GetPrefix(name))`).
pub(crate) unsafe fn es20_estados_do_componente(
    entidade: u64,
    inst: *const u8,
) -> (u64, Option<bwms_primitives::override_state::ComponentState>, Option<bwms_primitives::override_state::ComponentState>) {
    let (base, prefixo) = chaves_do_componente((inst.add(0x40) as *const u64).read_unaligned());
    (base, es20_estado(entidade, base), prefixo.and_then(|p| es20_estado(entidade, p)))
}

/// A máscara que o `ApplyChunkMaskOverride` grava no componente, pelo estado do nome base e do prefixo na loja (`States.cpp:710-731`); `None` = sem
/// mudança de máscara. `original` só roda depois desse teste.
pub(crate) unsafe fn es20_decidir_mascara(entidade: u64, inst: *const u8, original: impl FnOnce() -> u64) -> Option<u64> {
    let (_, componente, prefixo) = es20_estados_do_componente(entidade, inst);
    bwms_primitives::override_state::mascara_aplicada(original, componente.as_ref(), prefixo.as_ref())
}

/// `EntityState::ApplyChunkMaskOverride`: `Some(máscara escrita)`; `None` = a referência sai sem escrever (desligado, fora de mesh, sem mudança).
pub(crate) unsafe fn es20_aplicar_mascara(m: &Es20Malhas, entidade: u64, inst: *mut c_void) -> Option<u64> {
    if inst.is_null() || (inst as *const u8).add(0x8b).read_unaligned() == 0 {
        return None;
    }
    let tipo = es20_tipo(m, inst)?;
    let campo = (inst as *mut u8).add(m.campos[tipo].0) as *mut u64;
    let original = || es20_original(entidade, es20_id_do_componente(m.base, inst as *const u8), 0, || campo.read_unaligned());
    let fim = es20_decidir_mascara(entidade, inst as *const u8, original)?;
    campo.write_unaligned(fim);
    Some(fim)
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

/// Um componente do `ApplyComponentOverrides(estado, componentes, forçar)`: aparência e depois máscara; forçado, com alguma escrita, o `UpdateRenderer`
/// (o refresh medido). `(aparência escrita, máscara escrita, UpdateRenderer chamado)`.
pub(crate) unsafe fn es20_aplicar_componente(m: &Es20Malhas, entidade: u64, inst: *mut c_void, forcar: bool) -> (bool, bool, bool) {
    let aparencia = es20_aplicar_aparencia(m, entidade, inst).is_some();
    let mascara = es20_aplicar_mascara(m, entidade, inst).is_some();
    let render = forcar && (aparencia || mascara) && es20_refresh_medido(inst).map(|f| f(inst)).is_some();
    (aparencia, mascara, render)
}

/// `componentName`, `meshAppearance`, `chunkMask` e `acceptDismemberment` de `appearancePartComponentOverrides`, pelo RTTI.
pub(crate) unsafe fn es20_campos_do_remendo(reg: &rtti::Registry) -> Option<[usize; 4]> {
    let [nome, aparencia, mascara] = es20_campos_de_componente(reg)?;
    let c = reg.class_by_name("appearancePartComponentOverrides");
    let desmembramento = rtti::resolve_prop_in_class(c, "acceptDismemberment").map(|(o, _, _)| o as usize).filter(|o| *o < 0x50)?;
    Some([nome, aparencia, mascara, desmembramento])
}

/// `DummyAppearancePartPath`.
pub(crate) const ES20_PARTE_FICTICIA: u64 = bwms_primitives::resource_path_hash("#non_existing_part.ent");

/// Algum override de `partsOverrides` com recurso vazio, igual a `parte` ou o fictício já tem componente `nome`?
pub(crate) unsafe fn es20_tem_override(campos: [usize; 4], definicao: *const u8, parte: u64, nome: u64) -> bool {
    let externo = definicao.add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF);
    let b = rd_u64(externo as *const c_void, 0) as *const u8;
    let n = if b.is_null() { 0 } else { (externo.add(0x0c) as *const u32).read_unaligned() as usize };
    (0..n.min(4096)).any(|i| {
        let p = b.add(i * 0x18);
        let recurso = (p as *const u64).read_unaligned();
        if recurso != 0 && recurso != parte && recurso != ES20_PARTE_FICTICIA {
            return false;
        }
        let cb = rd_u64(p.add(8) as *const c_void, 0) as *const u8;
        let cn = if cb.is_null() { 0 } else { (p.add(0x14) as *const u32).read_unaligned() as usize };
        (0..cn.min(4096)).any(|k| (cb.add(k * 0x50 + campos[0]) as *const u64).read_unaligned() == nome)
    })
}

/// Anexa em `partsOverrides` um override de parte com o caminho fictício e o componente `nome` (`default`, máscara cheia, desmembramento ligado).
pub(crate) unsafe fn es20_anexar_override_de_cabeca(campos: [usize; 4], definicao: *mut u8, nome: u64) -> bool {
    let externo = definicao.add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF);
    let Some(parte) = es22_vaga_no_fim(externo, 0x18) else { return false };
    (parte as *mut u64).write_unaligned(ES20_PARTE_FICTICIA);
    crate::jogo::listas::reservar(parte.add(8) as *mut c_void, 1, 0x50, 0x10, std::ptr::null());
    let cb = rd_u64(parte.add(8) as *const c_void, 0) as *mut u8;
    if cb.is_null() || (parte.add(0x10) as *const u32).read_unaligned() < 1 {
        return false;
    }
    std::ptr::write_bytes(cb, 0, 0x50);
    (cb.add(campos[0]) as *mut u64).write_unaligned(nome);
    (cb.add(campos[1]) as *mut u64).write_unaligned(crate::cname::cname("default"));
    (cb.add(campos[2]) as *mut u64).write_unaligned(u64::MAX);
    cb.add(campos[3]).write(1);
    (parte.add(0x14) as *mut u32).write_unaligned(1);
    (externo.add(0x0c) as *mut u32).write_unaligned((externo.add(0x0c) as *const u32).read_unaligned() + 1);
    true
}

/// `PatchHeadGarmentOverrides(estado, definição)`: `None` fora da cabeça agregada; `Some(overrides anexados)`.
pub(crate) unsafe fn es20_remendar_cabeca(campos: [usize; 4], entidade: u64, definicao: *mut u8) -> Option<usize> {
    if definicao.is_null() || (definicao.add(crate::jogo::aparencia::APPDEF_NAME_OFF) as *const u64).read_unaligned() != crate::cname::cname("AppearanceName1") {
        return None;
    }
    static TRAVA: Mutex<()> = Mutex::new(());
    let _trava = TRAVA.lock().ok()?;
    let mut anexados = 0;
    for parte in es20_partes(definicao) {
        for nome in es20_nomes_da_parte(entidade, parte) {
            if !es20_tem_override(campos, definicao, parte, nome) && es20_anexar_override_de_cabeca(campos, definicao, nome) {
                anexados += 1;
            }
        }
    }
    Some(anexados)
}

// ===== ArchiveXL `#20` 20e — `OnResolveAppearance`/`OnResolveDefinition` (2026-09-14) =====
// Peças de `Extension.cpp:178-285` que não dependem de hook: a aparência vazia da referência (`s_emptyAppearance`), o `IsUniqueAppearanceName`, as tags
// visuais do template, o `SupportsDynamicAppearance`, as `EmptyAppearanceTags` e a poda de `partsValues` pelo depot. Os hooks ficam em `selftest.rs`
// (20a). O ramo não dinâmico do `OnResolveDefinition` (`FixCustomizationAppearance`) depende do escopo de customização do `.xl` (A#47) e fica fora.
pub(crate) const ES20_APARENCIA_VAZIA: &str = "empty_appearance_default";

/// `IsUniqueAppearanceName`: nem vazio, nem `default`, `random` ou a aparência vazia.
pub(crate) fn es20_nome_unico(nome: u64) -> bool {
    nome != 0 && !["default", "random", ES20_APARENCIA_VAZIA].iter().any(|n| crate::cname::cname(n) == nome)
}

/// Tags visuais do template (`visualTagsSchema` `+0x98` → `visualTags.tags` `+0x30`).
pub(crate) unsafe fn es20_tags_do_template(tpl: *const u8) -> Vec<u64> {
    let schema = (tpl.add(0x98) as *const u64).read_unaligned() as *const u8;
    if schema.is_null() {
        return Vec::new();
    }
    let b = (schema.add(0x30) as *const u64).read_unaligned() as *const u64;
    let n = if b.is_null() { 0 } else { (schema.add(0x3c) as *const u32).read_unaligned() as usize };
    (0..n.min(256)).map(|k| b.add(k).read_unaligned()).collect()
}

/// `SupportsDynamicAppearance`: a tag `DynamicAppearance` no template.
pub(crate) fn es20_suporta_dinamica(tags: &[u64]) -> bool {
    tags.contains(&crate::cname::cname("DynamicAppearance"))
}

/// Seletor dinâmico (`ParseAppearance(...).isDynamic`): `Some(contexto)`, com 0 sem `%contexto`; `None` se não é dinâmico. Registra a variante no pool como
/// o 22G.
pub(crate) fn es20_contexto_dinamico(texto: &str) -> Option<u64> {
    let seletor = es22_seletor(texto);
    seletor.is_dynamic.then_some(seletor.context)
}

// ===== ArchiveXL `#20` 20f — estado dinâmico por entidade (2026-09-14) =====
// O `UpdateState` da referência roda no job do ItemFactory; no BWMS o estado sai do `estado_dinamico_do_player`, que chama RTTI e script e só roda na thread do jogo. O
// hook do ItemFactory pede o recálculo (`es20_pedir_estado`), o tick drena os pedidos (`es20_pedidos_de_estado`) e guarda o resultado
// (`es20_guardar_estado`), que o `ResolveName`/`ResolvePath` do apply e as condições do 22H leem (`es20_estado_dinamico`). Divergência: entre o pedido e o
// tick, a montagem vê o estado anterior (a referência atualiza no próprio job).
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

/// Texto do `MarkDynamicAppearanceName(nome, entidade)` (`Dynamic.cpp:723-746`, 20f): corta do 1º `%` ou `&` até o fim, põe `!` se o nome não tem e anexa `%` com
/// o ponteiro da entidade em decimal. Quem chama registra o resultado no pool do motor (`CNamePool::Add`).
pub(crate) fn es20_marcar_nome_dinamico(texto: &str, entidade: u64) -> String {
    let base = texto.find(['%', '&']).map_or(texto, |p| &texto[..p]);
    let variante = if base.contains('!') { "" } else { "!" };
    format!("{base}{variante}%{entidade}")
}

/// Combinações de reserva do `ResolvePath` (`s_fallbackVariants`, `Dynamic.cpp:74-79` e `403-418`, 20g): pra `n` atributos com padrão (`body`, `feet`),
/// as linhas 1..2^n na ordem da referência, cada uma dizendo por atributo se entra o padrão (`true`) ou o valor do estado.
pub(crate) fn es20_combinacoes_de_reserva(n: usize) -> Vec<Vec<bool>> {
    (1..1usize << n.min(2)).map(|i| (0..n.min(2)).map(|j| (i >> j) & 1 == 1).collect()).collect()
}

/// `DynamicAppearanceController::ResolvePath` (`Dynamic.cpp:366-440`, 20g) com as dependências injetadas: `texto_de` é o `GetPathString` (no jogo, o registro
/// de caminhos dinâmicos do 48c-1), `existe` é o depot e `globais` o estado dinâmico da entidade (`None` = sem estado). Devolve o caminho final e, quando a
/// reserva por padrão (`base_body`/`flat` nos atributos `body`/`feet` usados e não fixados pela variante) entrou, o par `(resolvido, reserva)` do
/// `RegisterLink`. Opcional com atributo faltando dá caminho vazio.
pub(crate) fn es20_resolver_caminho(
    caminho: u64,
    globais: Option<&std::collections::HashMap<u64, String>>,
    locais: &std::collections::HashMap<u64, String>,
    texto_de: impl Fn(u64) -> Option<String>,
    existe: impl Fn(u64) -> bool,
) -> (u64, Option<(u64, u64)>) {
    let Some(texto) = texto_de(caminho).filter(|t| is_dynamic_value(t)) else { return (caminho, None) };
    let Some(globais) = globais else { return (caminho, None) };
    let r = crate::aparencia::processar_texto_dinamico(globais, locais, &texto);
    if !r.valid {
        return (caminho, None);
    }
    if r.optional && r.missed {
        return (0, None);
    }
    let resolvido = bwms_primitives::resource_path_hash(&r.value);
    if existe(resolvido) {
        return (resolvido, None);
    }
    let reserva: Vec<(u64, &str)> = [("body", "base_body"), ("feet", "flat")]
        .iter()
        .map(|(a, p)| (bwms_primitives::fnv1a64(a.as_bytes()), *p))
        .filter(|(h, _)| r.atributos.contains(h) && !locais.contains_key(h))
        .collect();
    for combinacao in es20_combinacoes_de_reserva(reserva.len()) {
        let mut estado = globais.clone();
        for ((h, padrao), usa) in reserva.iter().zip(&combinacao) {
            if *usa {
                estado.insert(*h, padrao.to_string());
            }
        }
        let alternativa = crate::aparencia::processar_texto_dinamico(&estado, locais, &texto);
        if alternativa.valid {
            let h = bwms_primitives::resource_path_hash(&alternativa.value);
            if existe(h) {
                return (h, Some((resolvido, h)));
            }
        }
    }
    (resolvido, None)
}

/// `IsDynamicAppearanceName`: `!` em qualquer posição do nome.
pub(crate) fn es20_nome_dinamico(texto: &str) -> bool {
    texto.contains('!')
}

/// Textos que o `DynamicAppearanceName(aAppearance)` registra no pool (`ExtractName(..., true)`, `Dynamic.cpp:136-176`): a variante inteira e, por parte
/// separada por `+`, o nome e o valor de `nome=valor` ou o valor posicional.
pub(crate) fn es20_textos_do_seletor(aparencia: &str) -> Vec<&str> {
    let Some(variante) = es22_texto_da_variante(aparencia) else { return Vec::new() };
    let mut textos = vec![variante];
    for parte in variante.split('+') {
        match parte.find('=') {
            Some(i) => textos.extend([&parte[..i], &parte[i + 1..]]),
            None => textos.push(parte),
        }
    }
    textos.retain(|t| !t.is_empty());
    textos
}

/// `DynamicPartList` como atributos locais do `ProcessString`: o texto de cada valor (`CName::ToString`); sem texto, vazio (a referência não escreve nada).
pub(crate) fn es20_locais(
    partes: &std::collections::BTreeMap<u64, u64>,
    texto_de: impl Fn(u64) -> Option<String>,
) -> std::collections::HashMap<u64, String> {
    partes.iter().map(|(k, v)| (*k, texto_de(*v).unwrap_or_default())).collect()
}

/// `DynamicAppearanceController::ResolveName` (`Dynamic.cpp:341-364`) com as dependências injetadas: `texto_de` é o `CName::ToString`, `registrar` o
/// `CNamePool::Add` e `globais` o estado dinâmico da entidade (`None` = sem estado). Opcional com atributo faltando dá o nome vazio.
pub(crate) fn es20_resolver_nome(
    nome: u64,
    globais: Option<&std::collections::HashMap<u64, String>>,
    locais: &std::collections::HashMap<u64, String>,
    texto_de: impl Fn(u64) -> Option<String>,
    registrar: impl Fn(&str) -> u64,
) -> u64 {
    let Some(texto) = texto_de(nome).filter(|t| is_dynamic_value(t)) else { return nome };
    let Some(globais) = globais else { return nome };
    let r = crate::aparencia::processar_texto_dinamico(globais, locais, &texto);
    if !r.valid {
        return nome;
    }
    if r.optional && r.missed {
        return 0;
    }
    registrar(&r.value)
}

/// `EntityState::ToggleConditionalComponents` sobre um `DynArray<Handle<IComponent>>` (`b`, `n`): componente de nome condicional com parte ligada e estado
/// de recurso fica ligado pelo `MatchReference` com as condições da entidade (sem estado dinâmico, referência com condição não casa); nos grupos de nome base
/// com peso, só o 1º de maior peso fica ligado. `(condicionais decididos, desligados pelo peso)`.
pub(crate) unsafe fn es20_alternar_condicionais(entidade: u64, b: *const u64, n: usize) -> (usize, usize) {
    let condicoes = es20_estado_dinamico(entidade).map(|(_, c)| c);
    let componente = |k: usize| b.add(k * 2).read_unaligned() as *mut u8;
    let mut entradas = Vec::with_capacity(n);
    let mut decididos = 0;
    for k in 0..n {
        let inst = componente(k);
        if inst.is_null() {
            entradas.push((0, 0, false));
            continue;
        }
        let referencia = parse_reference_live((inst.add(0x40) as *const u64).read_unaligned());
        if !referencia.is_conditional {
            entradas.push((referencia.name, 0, true));
            continue;
        }
        let Some(aparencia) = es20_aparencia_do_componente(entidade, referencia.name) else {
            entradas.push((referencia.name, referencia.weight, false));
            continue;
        };
        let ligado = appearance_matches_reference_com(&referencia, &aparencia, condicoes.as_ref());
        inst.add(0x8b).write_unaligned(u8::from(ligado));
        decididos += 1;
        entradas.push((referencia.name, referencia.weight, ligado));
    }
    let desligar = toggle_conditional_group_disable_indices(&entradas);
    for k in &desligar {
        componente(*k).add(0x8b).write_unaligned(0);
    }
    (decididos, desligar.len())
}

/// `EntityState::ApplyDynamicAppearance(componente)`: componente ligado, com parte e estado de recurso — a variante ativa da parte, com o nome só sem
/// `ChangesAppearance` no estado do componente. `(caminho escrito, aparência escrita)`.
pub(crate) unsafe fn es20_aplicar_dinamica(m: &Es20Malhas, entidade: u64, inst: *mut c_void) -> (bool, bool) {
    if inst.is_null() || (inst as *const u8).add(0x8b).read_unaligned() == 0 {
        return (false, false);
    }
    let base = es20_nome_base(inst as *const u8);
    let Some(aparencia) = es20_aparencia_do_componente(entidade, base) else { return (false, false) };
    let muda = es20_estado(entidade, base).is_some_and(|s| s.muda_aparencia());
    es20_aplicar_variante(m, entidade, inst, &aparencia.parts, !muda)
}

/// `ApplyDynamicAppearance(componente, variante, aplicar nome)` num componente de mesh: `ResolvePath` do recurso original e `ResolveName` da aparência
/// original, cada um escrito só se difere do original e do atual; o par de reserva vai pro `resource.link`.
pub(crate) unsafe fn es20_aplicar_variante(
    m: &Es20Malhas,
    entidade: u64,
    inst: *mut c_void,
    partes: &std::collections::BTreeMap<u64, u64>,
    nome: bool,
) -> (bool, bool) {
    let Some(tipo) = es20_tipo(m, inst) else { return (false, false) };
    let globais = es20_estado_dinamico(entidade).map(|(g, _)| g);
    let locais = es20_locais(partes, crate::cname::name_of);
    let id = es20_id_do_componente(m.base, inst as *const u8);
    let trocar = |campo: *mut u64, original: u64, fim: u64| {
        let muda = fim != original && fim != campo.read_unaligned();
        if muda {
            campo.write_unaligned(fim);
        }
        muda
    };
    let mut caminho = false;
    if let Some(o) = m.recurso(tipo) {
        let campo = (inst as *mut u8).add(o) as *mut u64;
        let original = es20_original(entidade, id, 2, || campo.read_unaligned());
        let (fim, link) =
            es20_resolver_caminho(original, globais.as_ref(), &locais, crate::selftest::a48c_caminho_dinamico, |h| crate::resource_exists_hash(h).unwrap_or(true));
        if let Some((de, para)) = link {
            crate::arquivos_do_jogo::reslink_add(de, para);
        }
        caminho = trocar(campo, original, fim);
    }
    let mut aparencia = false;
    if nome {
        let campo = (inst as *mut u8).add(m.campos[tipo].1) as *mut u64;
        let original = es20_original(entidade, id, 1, || campo.read_unaligned());
        let fim = es20_resolver_nome(original, globais.as_ref(), &locais, crate::cname::name_of, crate::cname::registrar_no_pool);
        aparencia = trocar(campo, original, fim);
    }
    (caminho, aparencia)
}

/// `GarmentExtension::ApplyDynamicAppearance(estado, componentes)`: condicionais e depois cada componente. `[condicionais decididos, desligados pelo peso,
/// caminhos escritos, aparências escritas]`.
pub(crate) unsafe fn es20_aplicar_dinamica_na_lista(m: &Es20Malhas, entidade: u64, b: *const u64, n: usize) -> [usize; 4] {
    if b.is_null() {
        return [0; 4];
    }
    let (decididos, desligados) = es20_alternar_condicionais(entidade, b, n);
    let mut r = [decididos, desligados, 0, 0];
    for k in 0..n {
        let (caminho, aparencia) = es20_aplicar_dinamica(m, entidade, b.add(k * 2).read_unaligned() as *mut c_void);
        r[2] += usize::from(caminho);
        r[3] += usize::from(aparencia);
    }
    r
}

/// Tira de `partsValues` (8 B cada), de trás pra frente, as partes que `existe` recusa (o `RemoveAt` da referência); devolve quantas tirou.
pub(crate) unsafe fn es20_podar_partes(definicao: *mut u8, existe: impl Fn(u64) -> bool) -> usize {
    let arr = definicao.add(crate::jogo::aparencia::APPDEF_PARTSVALUES_OFF);
    let b = rd_u64(arr as *const c_void, 0) as *mut u64;
    let mut n = if b.is_null() { 0 } else { ((arr.add(0x0c) as *const u32).read_unaligned() as usize).min(4096) };
    let mut tiradas = 0;
    for i in (0..n).rev() {
        if !existe(b.add(i).read_unaligned()) {
            std::ptr::copy(b.add(i + 1), b.add(i), n - i - 1);
            n -= 1;
            tiradas += 1;
        }
    }
    if tiradas > 0 {
        (arr.add(0x0c) as *mut u32).write_unaligned(n as u32);
    }
    tiradas
}

// ===== ArchiveXL `#22` 22G/22H — `EntityState::SelectDynamicAppearance` (2026-09-14) =====
// Os 2 overloads da referência (`States.cpp:427-533`). 22G, no `EntityTemplate`: sem entrada dada, a entrada da aparência base (o nome do seletor, pelo
// `FindAppearance` do motor) é copiada pro fim de `appearances` com o recurso e o nome de aparência dela (o nome da entrada, se ela não tem), e esse nome
// ganha a variante e o contexto do seletor (`nome!variante%contexto`, registrado no pool do motor); a entrada passa a se chamar pelo nome de aparência.
// 22H, no `AppearanceResource`: entre as definições cujo nome base é o do seletor, a sem condição vale 0 e a condicional que casa (variante do seletor e
// condições da entidade ou overrides do seletor) vale o peso dela; a de maior peso vence, e no empate a primeira. Se o nome dela não é o do seletor, uma
// cópia propriedade a propriedade com o nome do seletor entra no fim de `appearances` sob a trava exclusiva do recurso (`+0xF0`). Cada parte da escolhida é
// ligada à aparência do seletor. Sem nenhuma que case, com candidatas e seletor dinâmico, sai uma definição vazia com o nome do seletor, fora do recurso.
pub(crate) const ES22_TEMPLATE_APARENCIAS: usize = 0x50;

pub(crate) const ES22_TEMPLATE_ENTRADA: usize = 0x18;

/// Texto da variante de um nome de aparência dinâmico, o que a referência registra no pool ao ler o nome: depois do 1º `!`, sem o `%contexto` final e sem
/// as condições `&`. `None` sem `!` ou com a variante vazia.
pub(crate) fn es22_texto_da_variante(aparencia: &str) -> Option<&str> {
    let marca = aparencia.find('!')?;
    let mut fim = aparencia.len();
    if let Some(p) = aparencia.rfind('%') {
        fim = fim.min(p);
    }
    if let Some(p) = aparencia[..fim].find('&') {
        fim = fim.min(p);
    }
    (fim > marca + 1).then(|| &aparencia[marca + 1..fim])
}

/// Nome do `MarkDynamicAppearanceName(nome, seletor)`: `base!variante`, com `%contexto` quando o contexto não é 0.
pub(crate) fn es22_nome_marcado(base: &str, variante: Option<&str>, contexto: u64) -> String {
    let mut nome = format!("{base}!{}", variante.unwrap_or(""));
    if contexto != 0 {
        nome.push('%');
        nome.push_str(&contexto.to_string());
    }
    nome
}

/// Escolha do 22H sobre `(nome base, condicional, casa, peso)` de cada definição: índice da vencedora e quantas candidatas têm o nome base do seletor.
pub(crate) fn es22_escolher_definicao(nome_base: u64, definicoes: &[(u64, bool, bool, i32)]) -> (Option<usize>, usize) {
    let (mut peso_max, mut vencedora, mut candidatas) = (-1, None, 0);
    for (i, (base, condicional, casa, peso)) in definicoes.iter().enumerate() {
        if *base != nome_base {
            continue;
        }
        candidatas += 1;
        if !condicional {
            if peso_max < 0 {
                peso_max = 0;
                vencedora = Some(i);
            }
        } else if *casa && peso_max < *peso {
            peso_max = *peso;
            vencedora = Some(i);
        }
    }
    (vencedora, candidatas)
}

/// Seletor lido de `texto`, com a variante e as partes registradas no pool do motor como a referência faz ao ler o nome (o `ResolveName`/`ResolvePath` do
/// 20g lê o texto dos valores das partes).
pub(crate) fn es22_seletor(texto: &str) -> DynamicAppearanceName {
    let seletor = DynamicAppearanceName::parse(texto);
    if seletor.is_dynamic {
        for t in es20_textos_do_seletor(texto) {
            crate::cname::registrar_no_pool(t);
        }
    }
    seletor
}

/// 22G — `SelectDynamicAppearance(seletor, template, aparência)` num `EntityTemplate` vivo: `(escolheu, entrada)`, com a entrada dada ou a copiada.
pub(crate) unsafe fn es22_selecionar_no_template(texto: &str, tpl: *mut u8, aparencia: *mut u8) -> (bool, *mut u8) {
    let seletor = es22_seletor(texto);
    if !seletor.is_dynamic || tpl.is_null() {
        return (false, aparencia);
    }
    let mut aparencia = aparencia;
    if aparencia.is_null() {
        let achar: unsafe extern "C" fn(*mut u8, u64) -> *mut u8 = std::mem::transmute(crate::rebase(crate::jogo::aparencia::ENTITY_TEMPLATE_FIND_APP_VM));
        let base = achar(tpl, seletor.name) as usize;
        let arr = tpl.add(ES22_TEMPLATE_APARENCIAS);
        let entradas = rd_u64(arr as *const c_void, 0) as usize;
        let tam = (arr.add(0x0c) as *const u32).read_unaligned();
        if base == 0 || entradas == 0 || base < entradas || base >= entradas + tam as usize * ES22_TEMPLATE_ENTRADA {
            return (false, std::ptr::null_mut());
        }
        let indice = (base - entradas) / ES22_TEMPLATE_ENTRADA;
        let Some(nova) = es22_vaga_no_fim(arr, ES22_TEMPLATE_ENTRADA) else { return (false, std::ptr::null_mut()) };
        let base = (rd_u64(arr as *const c_void, 0) as *const u8).add(indice * ES22_TEMPLATE_ENTRADA);
        let ler = |off: usize| (base.add(off) as *const u64).read_unaligned();
        let nome_aparencia = if ler(0x10) != 0 { ler(0x10) } else { ler(0) };
        let marcado = crate::cname::registrar_no_pool(&es22_nome_marcado(&crate::cname::resolve_cname(nome_aparencia), es22_texto_da_variante(texto), seletor.context));
        (nova.add(0x08) as *mut u64).write_unaligned(ler(0x08));
        (nova.add(0x10) as *mut u64).write_unaligned(marcado);
        (nova as *mut u64).write_unaligned(marcado);
        (arr.add(0x0c) as *mut u32).write_unaligned(tam + 1);
        aparencia = nova;
    }
    (aparencia as *mut u64).write_unaligned((aparencia.add(0x10) as *const u64).read_unaligned());
    (true, aparencia)
}

/// Resultado do 22H: definição escolhida (handle forte, o chamador solta), nome da candidata vencedora (0 sem vencedora), candidatas vistas e se uma cópia
/// entrou no recurso.
#[derive(Debug, Default)]
pub(crate) struct Es22Escolha {
    pub definicao: Option<[u64; 2]>,
    pub vencedora: u64,
    pub candidatas: usize,
    pub anexada: bool,
}

/// Anexa uma cópia forte de `h` no fim de `appearances` do recurso, sob a trava exclusiva.
pub(crate) unsafe fn es22_anexar_definicao(recurso: *mut u8, h: [u64; 2]) -> bool {
    es22_com_trava(recurso, || es22_anexar_definicao_travado(recurso, h))
}

/// 22H — `SelectDynamicAppearance(seletor, recurso, definição)` num `AppearanceResource` vivo, pra `entidade` com as `condicoes` do estado dinâmico dela
/// (`None` = entidade sem estado).
pub(crate) unsafe fn es22_selecionar_definicao(
    entidade: u64,
    texto: &str,
    recurso: *mut u8,
    condicoes: Option<&std::collections::BTreeSet<u64>>,
) -> Es22Escolha {
    let seletor = es22_seletor(texto);
    let mut escolha = Es22Escolha::default();
    let arr = recurso.add(crate::jogo::aparencia::APPRESOURCE_APPEARANCES_OFF);
    let buffer = rd_u64(arr as *const c_void, 0) as *const u64;
    let n = if buffer.is_null() { 0 } else { (arr.add(0x0c) as *const u32).read_unaligned() as usize };
    let handles: Vec<[u64; 2]> = (0..n.min(65536)).map(|i| [buffer.add(i * 2).read_unaligned(), buffer.add(i * 2 + 1).read_unaligned()]).collect();
    let nome_de = |h: &[u64; 2]| ((h[0] as usize + crate::jogo::aparencia::APPDEF_NAME_OFF) as *const u64).read_unaligned();
    let definicoes: Vec<(u64, bool, bool, i32)> = handles
        .iter()
        .map(|h| {
            if h[0] == 0 {
                return (0, false, false, 0);
            }
            let referencia = parse_reference_live(nome_de(h));
            let casa = referencia.is_conditional && appearance_matches_reference_com(&referencia, &seletor, condicoes);
            (referencia.name, referencia.is_conditional, casa, referencia.weight)
        })
        .collect();
    let (vencedora, candidatas) = if seletor.name == 0 { (None, 0) } else { es22_escolher_definicao(seletor.name, &definicoes) };
    escolha.candidatas = candidatas;
    if let Some(i) = vencedora {
        let m = handles[i];
        escolha.vencedora = nome_de(&m);
        let definicao = if escolha.vencedora != seletor.value {
            let Some(h) = es22_definicao_nova(m[0] as *mut c_void, seletor.value, true) else { return escolha };
            escolha.anexada = es22_anexar_definicao(recurso, h);
            h
        } else {
            handle_strong_soma(&m, 1);
            m
        };
        for parte in es20_partes(definicao[0] as *const u8) {
            es22_ligar_parte_seletor(entidade, parte, &seletor);
        }
        escolha.definicao = Some(definicao);
    } else if seletor.is_dynamic && candidatas > 0 {
        let modelo = definicoes.iter().position(|d| d.0 == seletor.name).map(|i| handles[i][0] as *mut c_void);
        escolha.definicao = modelo.and_then(|m| es22_definicao_nova(m, seletor.value, false));
    }
    escolha
}

/// Chave = `(entity_ptr, resource_path_hash)` — porte de `App::ResourceState` (`States.hpp:
/// 51-76`), só o campo `m_appearance` (`DynamicAppearanceName`, o que `GetActiveAppearance()`
/// devolve) — `m_overridenOffsets` não é usado por nenhum dos 2 métodos desta rodada, fora de
/// escopo.
pub(crate) static ES22_RESOURCE_STATE: Mutex<Vec<((u64, u64), DynamicAppearanceName)>> = Mutex::new(Vec::new());

/// `DynamicAppearanceController::ParseReference(component->name)` sobre um componente VIVO —
/// resolve o hash cru do campo via o pool NATIVO do motor (`crate::cname::resolve_cname`, a
/// MESMA rotina real `CNamePool::Get` já usada em produção há semanas neste projeto) pra
/// recuperar a STRING, então reusa `DynamicAppearanceRef::parse` (já portado/provado, item
/// `#23`). Se o hash não resolver (CName nunca "adicionado" ao pool nativo — categoria já vista
/// noutras funções deste projeto), cai pro mesmo comportamento do ramo "sem marcador" da fonte
/// real (`name=value=hash cru`, `is_dynamic=false`) — nunca fabrica um nome fictício.
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

/// `ComponentPrefixResolver::GetPrefix` (`Prefix.cpp:3-27`) — porte fiel, 100% string, sem cache
/// (a fonte real cacheia por `Red::CName`; recomputamos toda vez, custo desprezível pra uma
/// string de poucos bytes, evita mais um registry). Devolve `None` == `Red::CName{}` (prefixo
/// vazio — nenhum `_` achado dentro dos primeiros 6 bytes).
pub(crate) fn component_prefix_of(name_str: &str) -> Option<u64> {
    let bytes = name_str.as_bytes();
    let prefix_max_len: usize = 6;
    let mut prefix_end: usize = 2;
    while prefix_end < prefix_max_len {
        match bytes.get(prefix_end) {
            Some(&b) if b != b'_' && b != 0 => prefix_end += 1,
            _ => break,
        }
    }
    if prefix_end < prefix_max_len && bytes.get(prefix_end) == Some(&b'_') {
        Some(bwms_primitives::fnv1a64(&bytes[..=prefix_end]))
    } else {
        None
    }
}

/// Núcleo PURO de decisão do 2º loop de `EntityState::ToggleConditionalComponents`
/// (`States.cpp:574-591`, o desempate por `WeightedComponentGroup`) — recebe já processado
/// `(nome_base, weight, matched)` por componente (índice = posição na lista original) e devolve
/// os ÍNDICES que devem ficar `isEnabled=false` por causa do desempate. Testável offline, zero
/// ponteiro de memória do jogo (mesma disciplina de "núcleo puro + wrapper unsafe" já usada em
/// toda a codebase pra lógica de decisão).
pub(crate) fn toggle_conditional_group_disable_indices(entries: &[(u64, i32, bool)]) -> Vec<usize> {
    use std::collections::HashMap;
    struct Group {
        max_weight: i32,
        matches: Vec<(i32, usize)>,
    }
    let mut groups: HashMap<u64, Group> = HashMap::new();
    for (idx, (name, weight, matched)) in entries.iter().enumerate() {
        if !*matched {
            continue;
        }
        let g = groups.entry(*name).or_insert(Group { max_weight: 0, matches: Vec::new() });
        g.matches.push((*weight, idx));
        if g.max_weight < *weight {
            g.max_weight = *weight;
        }
    }
    let mut disable = Vec::new();
    for g in groups.values() {
        if g.max_weight > 0 && g.matches.len() > 1 {
            let mut force_disable = false;
            for (w, idx) in &g.matches {
                if *w < g.max_weight || force_disable {
                    disable.push(*idx);
                } else {
                    force_disable = true;
                }
            }
        }
    }
    disable
}

/// `ToggleConditionalComponents` com as condições da entidade (22F); `None` reproduz a entidade sem estado.
pub(crate) unsafe fn run_toggle_conditional_components_com(
    reg: &crate::rtti::Registry,
    entity: *mut c_void,
    condicoes: Option<&std::collections::BTreeSet<u64>>,
) -> (String, i32) {
    let _ = reg;
    if entity.is_null() || !crate::memoria::is_readable(entity as *const c_void, 0x10) {
        return ("[togglecond] entity ptr insano".to_string(), -1);
    }
    let arr = (entity as *mut u8).add(0xA0) as *mut c_void;
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return ("[togglecond] Entity+0xA0 (ComponentsStorage) ilegível".to_string(), -1);
    }
    let entries_ptr = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries_ptr.is_null() || size == 0 || size > 4096 {
        return (format!("[togglecond] array de componentes vazio/implausível (size={size})"), -1);
    }
    let entity_key = entity as u64;

    struct Entry {
        name: u64,
        weight: i32,
        matched: bool,
        inst: *mut c_void,
        conditional: bool,
    }
    let mut entries: Vec<Entry> = Vec::with_capacity(size as usize);

    for i in 0..size as usize {
        let slot = entries_ptr.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 16) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if inst.is_null() || !crate::memoria::is_readable(inst as *const c_void, 0x60) {
            continue;
        }
        let name_hash = (inst as *const u8).add(0x40).cast::<u64>().read_unaligned();
        let reference = parse_reference_live(name_hash);

        if !reference.is_conditional {
            // `!componentRef.isConditional` -> entra no grupo com weight 0, NUNCA reescreve
            // `isEnabled` (States.cpp:543-548) — pass-through fiel.
            entries.push(Entry { name: reference.name, weight: 0, matched: true, inst, conditional: false });
            continue;
        }

        let part = match es22_parte_do_componente(entity_key, reference.name) {
            Some(p) => p,
            None => continue, // FindComponentState()==null || !IsFromAppearancePart() -> pula (States.cpp:552)
        };

        let appearance = ES22_RESOURCE_STATE
            .lock()
            .ok()
            .and_then(|g| g.iter().find(|((e, r), _)| *e == entity_key && *r == part).map(|(_, a)| a.clone()));
        let appearance = match appearance {
            Some(a) => a,
            None => continue, // FindResourceState()==null -> pula (States.cpp:557)
        };

        let matched = appearance_matches_reference_com(&reference, &appearance, condicoes);
        core::ptr::write_unaligned((inst as *mut u8).add(0x8b), matched as u8);
        entries.push(Entry { name: reference.name, weight: reference.weight, matched, inst, conditional: true });
    }

    let tuples: Vec<(u64, i32, bool)> = entries.iter().map(|e| (e.name, e.weight, e.matched)).collect();
    let disable_idx = toggle_conditional_group_disable_indices(&tuples);
    for &idx in &disable_idx {
        core::ptr::write_unaligned((entries[idx].inst as *mut u8).add(0x8b), 0u8);
    }

    let total = entries.len();
    let conditional = entries.iter().filter(|e| e.conditional).count();
    let matched_true = entries.iter().filter(|e| e.conditional && e.matched).count();
    (
        format!(
            "[togglecond] entity={entity:p} total_componentes={total} condicionais_avaliados={conditional} condicionais_matched={matched_true} desabilitados_por_desempate={}",
            disable_idx.len()
        ),
        total as i32,
    )
}

/// ArchiveXL `#21`/`#23` (`PENDENCIAS-UNIFICADAS.md`, 2026-08-12) — núcleo de
/// `App::DynamicAppearanceController` exposto ao redscript. `IsDynamicValue` (`Garment/Dynamic.cpp:674-697`):
/// as 5 sobrecargas da referência fazem a mesma checagem, a string começa com o marcador `*`
/// (`DynamicValueMarker`). Sem endereço nativo: é texto puro, igual nas duas plataformas.
///
/// O par `IsDynamicValue`/`ProcessString` (`:442-556`) tem uma implementação só no projeto, a deste
/// módulo, no dylib que o jogo carrega. O install (`bwms_primitives::instalacao`) não expande aparência
/// dinâmica: o valor dos atributos só existe em runtime, no estado
/// da entidade.
///
/// Dá ao item `#21` (`GetDynamicAppearanceController()`) uma contraparte real: um mod processa uma
/// string de aparência dinâmica (`"*{attr}_suffix"`) via redscript, com o MESMO mecanismo que
/// `MeshExtension::ExpandResourcePath`/item `#48` usa internamente pro material dinâmico.
///
/// Divergência de escopo CONSCIENTE (mesma categoria já aceita várias vezes neste projeto): em vez
/// de expor um objeto `DynamicAppearanceController` com mapa arbitrário de atributos (exigiria
/// marshalling de `array<String>`/dicionário, risco desproporcional ao ganho — categoria de risco
/// que já crashou este projeto uma vez, `TweakXL #43`/array-retorno), aceita 1 par nome/valor LOCAL
/// — cobre o caso majoritário e é literalmente o mesmo padrão que `ExpandResourcePath` já usa
/// internamente (só injeta 1 atributo `material`). `attrName` vazio = nenhum atributo local — replica
/// fielmente `missed=true` se a string tiver `{algumAttr}` (nenhum atributo bate). Esta peça cobre
/// só o núcleo de templating puro, não o sistema inteiro.
pub(crate) fn is_dynamic_value(s: &str) -> bool {
    s.starts_with('*')
}

/// `App::DynamicAppearanceController::ProcessString` (`Garment/Dynamic.cpp:442-556`) sobre o núcleo
/// `process_dynamic_string_com`, especializado pra 1
/// único atributo LOCAL (ver divergência de escopo documentada acima) — sem mapa `global_attrs`
/// (sempre vazio aqui, já que o BWMS não tem `EntityState::GetContextAttrs()` real). Substitui
/// marcadores `{attr}` embutidos na string por `attr_value` quando `attr` bate `attr_name`
/// (FNV1a64, `bwms_primitives::fnv1a64` — mesma função já usada em toda native de hash deste projeto);
/// pula o `*` inicial se presente; se sobrar texto não-processado (par `{`/`}` incompleto) OU
/// nenhum atributo foi encontrado na string, o resultado é inválido (mesma regra exata do C++:
/// `if (*str || attributes.empty()) return result;`); um `?` final marca `optional=true` e é
/// removido do valor. Limite de 512 bytes de saída (igual à fonte, `buffer[MaxLength+1]`).
pub(crate) fn process_dynamic_string_single_attr(attr_name: &str, attr_value: &str, input: &str) -> TextoProcessado {
    let attr_hash = if attr_name.is_empty() { None } else { Some(bwms_primitives::fnv1a64(attr_name.as_bytes())) };
    process_dynamic_string_com(input, |hash| (attr_hash == Some(hash)).then_some(attr_value))
}

/// `ProcessString` com a busca de atributo do chamador: hash FNV-1a64 do nome → valor. A 48c passa o `material` local e, atrás dele, os atributos do
/// `@context` (a mesma prioridade local > global da referência).
pub(crate) fn process_dynamic_string_com<'a>(input: &str, valor_de: impl Fn(u64) -> Option<&'a str>) -> TextoProcessado {
    const MAX_LENGTH: usize = 512;
    let mut result = TextoProcessado::default();
    if input.is_empty() {
        return result;
    }
    let bytes = input.as_bytes();
    let mut idx = if bytes[0] == b'*' { 1 } else { 0 };
    let mut out: Vec<u8> = Vec::with_capacity(MAX_LENGTH + 1);
    let mut attrs_found = 0usize;

    while idx < bytes.len() {
        let attr_open = match bytes[idx..].iter().position(|&b| b == b'{') {
            Some(p) => idx + p,
            None => {
                while idx < bytes.len() && out.len() < MAX_LENGTH {
                    out.push(bytes[idx]);
                    idx += 1;
                }
                break;
            }
        };
        let attr_close = match bytes[attr_open..].iter().position(|&b| b == b'}') {
            Some(p) => attr_open + p,
            None => break,
        };
        while idx != attr_open && out.len() < MAX_LENGTH {
            out.push(bytes[idx]);
            idx += 1;
        }
        if out.len() == MAX_LENGTH {
            break;
        }
        idx = attr_close + 1;
        let name = &input[attr_open + 1..attr_close];
        let hash = bwms_primitives::fnv1a64(name.as_bytes());
        attrs_found += 1;
        result.atributos.insert(hash);
        if let Some(attr_value) = valor_de(hash) {
            let vbytes = attr_value.as_bytes();
            let mut vi = 0;
            while vi < vbytes.len() && out.len() < MAX_LENGTH {
                out.push(vbytes[vi]);
                vi += 1;
            }
            if out.len() == MAX_LENGTH {
                break;
            }
        } else {
            result.missed = true;
        }
    }

    if idx < bytes.len() || attrs_found == 0 {
        return result;
    }
    if out.last() == Some(&b'?') {
        result.optional = true;
        out.pop();
    }
    result.value = String::from_utf8_lossy(&out).into_owned();
    result.valid = true;
    result
}

/// ArchiveXL `#23` (`App::DynamicAppearanceRef`, `Garment/Dynamic.cpp:194-266`, catálogo item
/// `#23`, 2026-08-12 round 3), no dylib que o jogo carrega: segue o ctor
/// `DynamicAppearanceRef(Red::CName aReference)` byte a byte, e os vetores estão em `register.rs`
/// (`archivexl_dynamic_appearance_tests` e `archivexl_dynamic_appearance_parse_tests`). É a
/// implementação única do projeto: o install (`bwms_primitives::instalacao`) não avalia o critério, que só
/// é avaliado em runtime, contra o estado da entidade (`EntityState`, item `#22`).
/// Parseia o NOME de
/// um componente/aparência que pode embutir uma lista de VARIANTES (`nome!v1!v2`) e/ou CONDIÇÕES
/// (`nome&c1&c2`, ou combinando os 2) — a sintaxe que `EntityState::ToggleConditionalComponents`/
/// `SelectDynamicAppearance` (item `#22`) usam pra decidir qual componente/aparência condicional
/// está ativo dado o contexto do personagem. 100% hash puro (FNV1a64 via `bwms_primitives::fnv1a64`,
/// zero registro no `CNamePool` — confirmado ser o único caminho usado aqui, mesma convenção já
/// usada em `process_dynamic_string_single_attr`). Os 7 campos são os do `Garment/Dynamic.hpp:28-43`.
/// `weight` é guardado em `i32` porque
/// `toggle_conditional_group_disable_indices`, `es22_escolher_definicao` e o retorno `Int32` de
/// `AppearanceRefWeight` comparam em `i32`, mas o valor é truncado mod 256 como o `int8_t` da
/// referência (`Garment/Dynamic.hpp:42`, `Dynamic.cpp:263-264`): de 28 a 155 condições com variante
/// (de 128 a 255 sem variante) o peso fica negativo e `is_conditional` = false.
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

/// `App::ExtractName(aName, aOffset, aSize)` (`Garment/Dynamic.cpp:81-93`): FNV-1a64 do trecho, 0 se vazio.
pub(crate) fn dynref_extract_name_hash(s: &str) -> u64 {
    if s.is_empty() { 0 } else { bwms_primitives::fnv1a64(s.as_bytes()) }
}

impl DynamicAppearanceRef {
    /// Porte byte-a-byte do ctor `DynamicAppearanceRef(Red::CName aReference)`.
    pub(crate) fn parse(reference: &str) -> Self {
        let mut r = DynamicAppearanceRef { value: dynref_extract_name_hash(reference), ..Default::default() };

        let Some(marker_pos) = reference.find(['!', '&']) else {
            // sem marcador nenhum -> não-dinâmico, `name` = a própria referência.
            r.name = r.value;
            return r;
        };

        r.is_dynamic = true;
        r.name = dynref_extract_name_hash(&reference[..marker_pos]);
        let mut rest = &reference[marker_pos..];

        if rest.len() > 1 {
            if rest.as_bytes()[0] == b'!' {
                rest = &rest[1..];
                loop {
                    if rest.is_empty() {
                        break;
                    }
                    match rest.find(['!', '&']) {
                        Some(0) if rest.as_bytes()[0] == b'&' => break,
                        None => {
                            r.variants.insert(dynref_extract_name_hash(rest));
                            break;
                        }
                        Some(p) => {
                            r.variants.insert(dynref_extract_name_hash(&rest[..p]));
                            if rest.as_bytes()[p] != b'!' {
                                rest = &rest[p..];
                                break;
                            }
                            rest = &rest[p + 1..];
                        }
                    }
                }
            }

            if rest.as_bytes().first() == Some(&b'&') {
                rest = &rest[1..];
                loop {
                    if rest.is_empty() {
                        break;
                    }
                    match rest.find('&') {
                        None => {
                            r.conditions.insert(dynref_extract_name_hash(rest));
                            break;
                        }
                        Some(p) => {
                            r.conditions.insert(dynref_extract_name_hash(&rest[..p]));
                            rest = &rest[p + 1..];
                        }
                    }
                }
            }

            // Soma em `usize` e trunca mod 256 em `i8` como o `static_cast<int8_t>` de `Dynamic.cpp:263`: de 28 a 155 condições
            // com variante (de 128 a 255 sem variante) o peso fica negativo e `is_conditional` = false. O campo segue `i32`
            // porque os consumidores comparam em `i32`.
            r.weight = (((if r.variants.is_empty() { 0usize } else { 100 }) + r.conditions.len()) as i8) as i32;
            r.is_conditional = r.weight > 0;
        }

        r
    }

    /// `Match(aVariant)`.
    pub(crate) fn matches_variant(&self, variant_hash: u64) -> bool {
        self.variants.contains(&variant_hash)
    }

    /// `Match(aConditions, aOverrides)` — condição satisfeita se estiver em QUALQUER um dos 2 sets.
    pub(crate) fn matches_conditions_with_overrides(
        &self,
        conditions_present: &std::collections::BTreeSet<u64>,
        overrides: &std::collections::BTreeSet<u64>,
    ) -> bool {
        self.conditions.iter().all(|c| conditions_present.contains(c) || overrides.contains(c))
    }
}

/// ArchiveXL `#23` (`App::DynamicAppearanceName`, `Garment/Dynamic.hpp:12-26`+`Dynamic.cpp:96-192`,
/// item `#23` do catálogo, 2026-08-15), no dylib que o jogo carrega: implementação única do
/// projeto, como a do `DynamicAppearanceRef` irmão acima, com os 7 campos de
/// `Garment/Dynamic.hpp:12-26`. Os vetores estão nos mesmos dois módulos de teste de `register.rs`
/// (`archivexl_dynamic_appearance_tests` e `archivexl_dynamic_appearance_parse_tests`).
/// É o VALOR real de uma aparência/componente que o
/// motor tenta casar contra um `DynamicAppearanceRef` (o CRITÉRIO, já portado acima) — sintaxe
/// `nome!variante[+parte[=valor]]...[%contexto][&condicao...]`. 100% composição pura sobre
/// `bwms_primitives::fnv1a64`/`fnv1a64_seeded` (já provados), zero endereço nativo, zero campo cru.
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
    /// Porte do ctor `DynamicAppearanceName(Red::CName aAppearance)` (`Dynamic.cpp:102-186`), com o
    /// corte por índice (`end`). Com '%'/'&' antes do '!' o parse degrada para não-dinâmico: no C++ o
    /// `remove_prefix` passaria do tamanho (UB), e texto vindo de redscript não pode derrubar o jogo.
    pub(crate) fn parse(appearance: &str) -> Self {
        let mut n = DynamicAppearanceName { value: dynref_extract_name_hash(appearance), ..Default::default() };

        let Some(marker_pos) = appearance.find('!') else {
            n.name = n.value;
            return n;
        };

        // contexto: ÚLTIMO '%' na string ORIGINAL (achado de RE: só parseia se `%numero` for o
        // final LITERAL da string — a ordem válida é "nome!variante&condicao%numero").
        let mut end = appearance.len();
        if let Some(context_pos) = appearance.rfind('%') {
            if let Ok(v) = appearance[context_pos + 1..].parse::<u64>() {
                n.context = v;
            }
            end = end.min(context_pos);
        }
        if let Some(cond_pos) = appearance[..end].find('&') {
            end = end.min(cond_pos);
        }
        // guarda de segurança (sem equivalente no C++ — string malformada vinda de redscript ao
        // vivo nunca pode panicar o processo do jogo).
        if end <= marker_pos {
            n.name = n.value;
            return n;
        }

        n.is_dynamic = true;
        n.name = dynref_extract_name_hash(&appearance[..marker_pos]);

        let mut rest = &appearance[marker_pos + 1..end];

        if !rest.is_empty() {
            n.variant = dynref_extract_name_hash(rest);
            let variant_attr = bwms_primitives::fnv1a64(b"variant");
            n.parts.insert(variant_attr, n.variant);

            let mut part_counter: u8 = b'1';

            loop {
                if rest.is_empty() {
                    break;
                }
                match rest.find('+') {
                    Some(0) => {
                        rest = &rest[1..];
                        continue;
                    }
                    found => {
                        let (marker2, skip) = match found {
                            Some(p) => (p, p + 1),
                            None => (rest.len(), rest.len()),
                        };
                        let assign_pos = rest.find('=').filter(|&ap| ap < marker2);
                        if let Some(ap) = assign_pos {
                            let part_name = dynref_extract_name_hash(&rest[..ap]);
                            let part_value = dynref_extract_name_hash(&rest[ap + 1..marker2]);
                            n.parts.insert(part_name, part_value);
                            // `Dynamic.cpp:168` faz `FNV1a64(str.data(), markerPos - assignPos)`: os `marker2 - ap` primeiros bytes da PARTE, não a chave, o valor
                            // nem "chave=valor". Anomalia da referência, replicada de propósito: a condição do mod casa contra esse mesmo hash lá.
                            let quirky_len = (marker2 - ap).min(rest.len());
                            n.overrides.insert(bwms_primitives::fnv1a64(rest[..quirky_len].as_bytes()));
                        } else {
                            let seed = bwms_primitives::fnv1a64(b"variant");
                            let part_name = bwms_primitives::fnv1a64_seeded(&[b'.', part_counter], seed);
                            let part_value = dynref_extract_name_hash(&rest[..marker2]);
                            n.parts.insert(part_name, part_value);
                            part_counter = part_counter.wrapping_add(1);
                        }
                        rest = &rest[skip..];
                    }
                }
            }
        }

        n
    }
}

/// Porte de `App::ResourceState` (`States.hpp:51-76`), só o campo `m_overridenOffsets` (os outros
/// campos — `m_appearance`/`m_path` — já têm contraparte em `ES22_RESOURCE_STATE`, registry
/// separado, mesma disciplina "1 registry por campo/rodada" já estabelecida acima).
///
/// Semântica (`ResourceState::GetOverriddenOffset`, `States.cpp:187-190`):
/// `!m_overridenOffsets.empty() ? m_overridenOffsets.begin()->second : 0`. O `Core::Map` é `tsl::hopscotch_map`
/// (`Stl.hpp:40`), sem ordem: com 2+ mods registrando offset pro MESMO resource a referência não tem regra. Aqui vence
/// a 1ª entrada INSERIDA (`Vec` linear); a aparência do componente usa o menor hash (`ComponentState::aparencia_sobrescrita`).
#[derive(Debug, Clone, Default)]
pub(crate) struct Es67ResourceOffset {
    /// `(mod_hash, offset)` — 1 entrada por mod; `AddOffsetOverride` atualiza in-place se o
    /// `mod_hash` já existir (fiel: `EntityState::AddOffsetOverride`→`ResourceState::
    /// AddOffsetOverride`, `States.cpp:172-180`: `it==end() ? emplace(hash,0) : it`, sempre
    /// seguido por `it.value() = aOffset`).
    pub(crate) overrides: Vec<(u64, i32)>,
}

impl Es67ResourceOffset {
    pub(crate) fn add_offset_override(&mut self, mod_hash: u64, offset: i32) {
        if let Some(e) = self.overrides.iter_mut().find(|(h, _)| *h == mod_hash) {
            e.1 = offset;
        } else {
            self.overrides.push((mod_hash, offset));
        }
    }

    /// `ResourceState::RemoveOffsetOverride(aHash) -> bool` (`States.cpp:182-185`) —
    /// `m_overridenOffsets.erase(aHash)`, devolve se algo foi de fato removido.
    pub(crate) fn remove_offset_override(&mut self, mod_hash: u64) -> bool {
        let before = self.overrides.len();
        self.overrides.retain(|(h, _)| *h != mod_hash);
        self.overrides.len() != before
    }

    pub(crate) fn get_overridden_offset(&self) -> i32 {
        self.overrides.first().map(|(_, o)| *o).unwrap_or(0)
    }
}

/// Chave = `(entity_ptr, resource_path_hash)` — MESMO esquema de `ES22_RESOURCE_STATE`/
/// `ES22_COMPONENT_APPEARANCE` (registry por-rodada dedicado; não reusa os outros 2, cada um
/// guarda um campo diferente do `ResourceState`/`ComponentState` real).
pub(crate) static ES67_RESOURCE_OFFSET: Mutex<Vec<((u64, u64), Es67ResourceOffset)>> = Mutex::new(Vec::new());

/// `EntityState::AddOffsetOverride(aHash, aResourcePath, aOffset)` (`States.cpp:367-372`).
/// `pub(crate)` desde a rodada 44 (2026-08-16) — o probe de escrita sintética em `selftest.rs`
/// precisa registrar um override de TESTE diretamente (sem passar pela trampolim nativa
/// `tramp_adicionar_offset_de_roupa`, que exige um frame de chamada redscript real).
pub(crate) unsafe fn add_offset_override(entity: u64, mod_hash: u64, resource_path: u64, offset: i32) {
    if let Ok(mut g) = ES67_RESOURCE_OFFSET.lock() {
        let key = (entity, resource_path);
        match g.iter_mut().find(|(k, _)| *k == key) {
            Some((_, s)) => s.add_offset_override(mod_hash, offset),
            None => {
                let mut s = Es67ResourceOffset::default();
                s.add_offset_override(mod_hash, offset);
                g.push((key, s));
            }
        }
    }
}

/// `EntityState::RemoveOffsetOverrides(aHash)` (`States.cpp:375-380`) — remove `aHash` de TODOS
/// os `ResourceState` da entidade (loop `for (auto& [path, resourceState] : m_resourceStates)`),
/// não só de 1 resource — fiel ao caso real ("mod X foi desequipado, limpa tudo que X registrou").
/// Devolve a contagem de `ResourceState`s afetados (diagnóstico; a fonte real é `void`).
pub(crate) unsafe fn remove_offset_overrides(entity: u64, mod_hash: u64) -> u32 {
    let mut n = 0u32;
    if let Ok(mut g) = ES67_RESOURCE_OFFSET.lock() {
        for ((e, _), s) in g.iter_mut() {
            if *e == entity && s.remove_offset_override(mod_hash) {
                n += 1;
            }
        }
    }
    n
}

/// `EntityState::GetOffsetOverride(aResourcePath)` (`States.cpp:748-754`) — `0` se não houver
/// `ResourceState` pra esse path (fiel: `!resourceState ? 0 : resourceState->GetOverriddenOffset()`).
pub(crate) unsafe fn get_offset_override(entity: u64, resource_path: u64) -> i32 {
    ES67_RESOURCE_OFFSET
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|(k, _)| *k == (entity, resource_path)).map(|(_, s)| s.get_overridden_offset()))
        .unwrap_or(0)
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


// cw-controller-misc: IComponent::Toggle native scripting handler vmaddr
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

// ===== Codeware `#127`/`#128`/`#134`: `Component/Toggle` e `Vehicle/ToggleAuxLights` =====
// Como os controllers da referência (`HookAfter`): a original roda e, com ouvinte do evento, o evento sai com o componente (ou o controller do veículo) e a
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

/// Codeware `#127`/`#118`: instala os 2 hooks do `Toggle` no `RegisterCallback` de `Component/Toggle`, de novo depois de uma desinstalação. Quem chama
/// serializa ativação e desativação.
pub(crate) unsafe fn garantir_hooks_de_toggle() {
    let script = instalar_conferindo_prologo(ICOMP_TOGGLE_VM, ICOMP_TOGGLE_PROLOGO, icomp_toggle_handler as *mut c_void, &ORIG_ICOMP_TOGGLE);
    let cpp = instalar_conferindo_prologo(ICOMP_TOGGLE_CPP_VM, ICOMP_TOGGLE_CPP_PROLOGO, icomp_toggle_cpp as *mut c_void, &ORIG_ICOMP_TOGGLE_CPP);
    if script.is_some() || cpp.is_some() {
        crate::veredito_dev!("[toggle] hooks do IComponent::Toggle pelo RegisterCallback (None = já no ar): handler de script {script:?}, função C++ {cpp:?}");
    }
}


pub(crate) const CMESH_POSTLOAD_VM: u64 = 0x1_0519_a58c;

// ===== ArchiveXL `#17` — `ExtractPartComponents` ao vivo, partes 17a e 17b (2026-09-12) =====
// 17a, com `~/.bwms-hook-a17-partes`: hook só observando em `0x1017d3158` (Mac: `x0` = &SharedPtr<ResourceToken<EntityTemplate>> da parte, `x8` = DynArray
// de saída; RE em `proofs/20260911-archivexl-17-extractpartcomponents-sret-identificado.log`). Um trampolim nu passa o `x8` do chamador como 2º argumento; o
// hook chama a original com ele explícito (`extract_part_components_em`) e guarda, das primeiras partes com objetos, o token (com 1 strong a mais, pra seguir
// vivo), o campo `+0x48` do token, o tamanho do array e as classes dos objetos. 17b, no tick de gameplay: chama a original de novo com o token guardado e
// confere tamanho e classes com o que o motor extraiu; controle com o token de um caminho que não existe (token com falha: array vazio). É a base da 17c — a
// referência aplica `resource.patch` extraindo os componentes do `.ent` de patch e mesclando por nome e id no array da parte. Os arrays das chamadas do teste
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

/// Núcleo puro do `MergeComponents` da referência: `resultado[i]` = `(nome, id)` do objeto se ele é componente; `patch[j]` = `(nome, id, externo)`. Os
/// anexados entram nas buscas seguintes, como o `PushBack` da referência.
pub(crate) fn plano_da_mescla(resultado: &[Option<(u64, u64)>], patch: &[Option<(u64, u64, bool)>], parte: bool) -> Vec<PassoDaMescla> {
    let mut chaves = resultado.to_vec();
    patch
        .iter()
        .map(|p| {
            let Some((nome, id, externo)) = *p else { return PassoDaMescla::Pula };
            if let Some(i) = chaves.iter().position(|c| *c == Some((nome, id))) {
                PassoDaMescla::Substitui(i)
            } else if parte && externo {
                PassoDaMescla::Pula
            } else {
                chaves.push(Some((nome, id)));
                PassoDaMescla::Anexa
            }
        })
        .collect()
}

/// Os dois arrays de handles `{instância, bloco}` depois de aplicar `plano`: substituídos trocados de lugar, anexados em ordem no fim com a vaga nula.
pub(crate) fn arrays_com_o_plano(resultado: &[[u64; 2]], patch: &[[u64; 2]], plano: &[PassoDaMescla]) -> (Vec<[u64; 2]>, Vec<[u64; 2]>) {
    let (mut r, mut p) = (resultado.to_vec(), patch.to_vec());
    for (j, passo) in plano.iter().enumerate().take(p.len()) {
        match *passo {
            PassoDaMescla::Substitui(i) if i < r.len() => std::mem::swap(&mut r[i], &mut p[j]),
            PassoDaMescla::Anexa => {
                r.push(p[j]);
                p[j] = [0; 2];
            }
            _ => {}
        }
    }
    (r, p)
}

/// Chave de mescla do objeto: `(nome, id, externo)` se ele é componente.
pub(crate) unsafe fn chave_de_mescla(obj: u64) -> Option<(u64, u64, bool)> {
    if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, 8) {
        return None;
    }
    let cls = crate::rtti::class_of(obj as *mut c_void);
    if !class_is_kind_of(cls, CLASSE_DE_COMPONENTE.load(Ordering::Relaxed)) {
        return None;
    }
    let (id, nome) = crate::register::component_id_name(obj as *mut c_void)?;
    Some((nome, id, class_is_kind_of(cls, CLASSE_DE_COMPONENTE_EXTERNO.load(Ordering::Relaxed))))
}

/// Handles de um `DynArray<Handle<ISerializable>>` (cabeçalho de 16 B), até 16384 (o pacote do V passa de 4096); vazio se ilegível.
pub(crate) unsafe fn handles_do_dynarray(arr: *const u8) -> Vec<[u64; 2]> {
    let v = (arr as *const u64).read_unaligned() as *const u64;
    let n = (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    if v.is_null() || n == 0 || n > 0x4000 || !crate::memoria::is_readable(v as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n).map(|i| [v.add(i * 2).read_unaligned(), v.add(i * 2 + 1).read_unaligned()]).collect()
}

/// Mescla `patch` em `resultado` (cabeçalhos de 16 B de `DynArray<Handle<ISerializable>>`) e devolve o plano aplicado; `None` sem efeito com array ilegível
/// ou reserve sem capacidade.
pub(crate) unsafe fn mesclar_componentes(resultado: *mut u8, patch: *mut u8, parte: bool) -> Option<Vec<PassoDaMescla>> {
    let tam = |arr: *const u8| (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    let (r0, p0) = (handles_do_dynarray(resultado), handles_do_dynarray(patch));
    if r0.len() != tam(resultado) || p0.len() != tam(patch) {
        return None;
    }
    let chaves_r: Vec<Option<(u64, u64)>> = r0.iter().map(|h| chave_de_mescla(h[0]).map(|(nome, id, _)| (nome, id))).collect();
    let chaves_p: Vec<Option<(u64, u64, bool)>> = p0.iter().map(|h| chave_de_mescla(h[0])).collect();
    let plano = plano_da_mescla(&chaves_r, &chaves_p, parte);
    let (r1, p1) = arrays_com_o_plano(&r0, &p0, &plano);
    if r1.len() as u32 > (resultado.add(0x08) as *const u32).read_unaligned() {
        let mover = if r0.is_empty() { std::ptr::null() } else { crate::rebase(MOVER_HANDLES_VM) as *const c_void };
        crate::jogo::listas::reservar(resultado as *mut c_void, r1.len() as u32, 0x10, 8, mover);
        if ((resultado.add(0x08) as *const u32).read_unaligned() as usize) < r1.len() {
            return None;
        }
    }
    let rv = (resultado as *const u64).read_unaligned() as *mut u64;
    let pv = (patch as *const u64).read_unaligned() as *mut u64;
    for (i, h) in r1.iter().enumerate() {
        rv.add(i * 2).write_unaligned(h[0]);
        rv.add(i * 2 + 1).write_unaligned(h[1]);
    }
    for (j, h) in p1.iter().enumerate() {
        pv.add(j * 2).write_unaligned(h[0]);
        pv.add(j * 2 + 1).write_unaligned(h[1]);
    }
    (resultado.add(0x0C) as *mut u32).write_unaligned(r1.len() as u32);
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

// ===== ArchiveXL `#17` 17e — `resource.patch` no job de extração de garment (2026-09-12) =====
// A roupa do V não passa só pelo `ExtractPartComponents`: o `GarmentAssembler::ExtractComponentsJob` (`0x1017ee34c`, offline — `x0` =
// `GarmentExtractionParams{partTemplate Handle @0x00, context SharedPtr @0x10}`, `x1` = `JobGroup&`; lê o bloco de pacote `+0x1d8` do template, monta o
// próprio extrator, chama o `ExtractSync` `0x101075dbc` direto e percorre os componentes com o `context` de `params+0x10`) extrai o pacote da parte por conta
// própria. No boot 93 o hook da 17d mesclou na jaqueta do V (7 → 11) e nenhum componente da camisa apareceu. A referência hookeia esse job depois da original
// (`OnGarmentPackageExtract`): pra cada patch da parte com `Modifies("components")`, troca o `partTemplate` dos params pelo template do patch, chama a original de
// novo com o mesmo `JobGroup` e devolve o template. Aqui igual, com a troca crua dos 16 B do handle (nada é solto e o original volta no lugar). O caminho do
// template sai de `+0x30` (o `path` do `CResource`, o mesmo do hook do `PostLoad` do `CMesh`); as primeiras chamadas vão pro log com o hash, pra conferir o
// offset contra o hash da parte alvo. O template do patch é o `Handle` em `+0x28` do token pedido na carga da tabela.
pub(crate) const GARMENT_EXTRACT_JOB_VM: u64 = 0x1_017e_e34c;

pub(crate) static ORIG_GARMENT_EXTRACT_JOB: AtomicU64 = AtomicU64::new(0);

pub(crate) type GarmentExtractJobFn = unsafe extern "C" fn(*mut [u64; 2], *const c_void);

pub(crate) unsafe extern "C" fn garment_extract_job_hook(params: *mut [u64; 2], grupo: *const c_void) {
    let f: GarmentExtractJobFn = std::mem::transmute(original_publicado(&ORIG_GARMENT_EXTRACT_JOB));
    // O job solta os próprios params no fim (`Free<job::PoolJobs2Data>` com o `x19`, `0x1017eed3c`): tudo que se lê deles vem ANTES da original, e a original
    // não roda de novo com o mesmo bloco. A 17e v1/v2 lia depois (bloco já solto — a 1ª palavra virava o elo da lista livre do pool, o que as sondas dos boots
    // 118 e 120 mostraram) e reusava o bloco. A extração do patch fica pra v3, com params novos do pool do job.
    let mut copias: Vec<*mut [u64; 2]> = Vec::new();
    let (mut contexto, mut caminho) = (0u64, 0u64);
    if PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) && !params.is_null() && crate::memoria::is_readable(params as *const c_void, 0x30) {
        a17_garment_observar(params);
        copias = params_do_job_por_patch(params);
        if !copias.is_empty() {
            contexto = ((params as *const u8).add(0x10) as *const u64).read_unaligned();
            let tpl = (*params)[0];
            if tpl != 0 && crate::memoria::is_readable((tpl + 0x30) as *const c_void, 8) {
                caminho = ((tpl + 0x30) as *const u64).read_unaligned();
            }
        }
    }
    let t0 = a17e_alvos_de_skin(contexto);
    f(params, grupo);
    let t1 = a17e_alvos_de_skin(contexto);
    // 17e v3: a extração de cada patch depois da original (o `HookAfter` da referência), cada uma com params próprios, que o job solta no fim.
    let aplicar = !copias.is_empty();
    for c in copias {
        f(c, grupo);
        A17_GARMENT_APLICADOS.fetch_add(1, Ordering::Relaxed);
    }
    if aplicar {
        a17e_amostrar(contexto, caminho, t0, t1);
    }
}

/// `PoolStorageProxy<job::PoolJobs2Data>::Allocate(tamanho)` e `Free<job::PoolJobs2Data>(bloco, 1)` (stubs; o agendador `0x1017ee098` aloca os params assim).
pub(crate) const POOL_JOBS2_ALLOCATE_VM: u64 = 0x1_052a_488c;

pub(crate) const POOL_JOBS2_FREE_VM: u64 = 0x1_052a_6584;

/// Params novos (0x30 B do pool do job) pra cada patch com `components` da parte deste job: cópia dos params com o `Handle` do template do patch em `+0x00`
/// e strong+1 nos dois `Handle` (`+0x00` e `+0x10`), que o job solta no fim. Chamado antes da original, com os params ainda vivos.
pub(crate) unsafe fn params_do_job_por_patch(params: *mut [u64; 2]) -> Vec<*mut [u64; 2]> {
    let p = params as *const u8;
    let tpl = (p as *const u64).read_unaligned();
    if tpl == 0 || !crate::memoria::is_readable((tpl + 0x30) as *const c_void, 8) {
        return Vec::new();
    }
    let caminho = ((tpl + 0x30) as *const u64).read_unaligned();
    let tokens: Vec<[u64; 2]> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo(caminho).filter(|x| x.props & A17_PROP_COMPONENTS != 0).map(|x| x.token).collect(),
        Err(_) => return Vec::new(),
    };
    let alocar: unsafe extern "C" fn(u64) -> *mut u8 = std::mem::transmute(crate::rebase(POOL_JOBS2_ALLOCATE_VM));
    let soltar: unsafe extern "C" fn(*mut u8, u32) = std::mem::transmute(crate::rebase(POOL_JOBS2_FREE_VM));
    let mut v = Vec::new();
    for pt in tokens {
        if recurso_do_patch_da_classe(pt, 0x248, CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed)).is_null() || !crate::memoria::is_readable((pt[0] + 0x28) as *const c_void, 16) {
            continue;
        }
        let h = [((pt[0] + 0x28) as *const u64).read_unaligned(), ((pt[0] + 0x30) as *const u64).read_unaligned()];
        if h[0] == 0 || h[1] == 0 {
            continue;
        }
        let bloco = alocar(0x30);
        if bloco.is_null() {
            continue;
        }
        std::ptr::copy_nonoverlapping(p, bloco, 0x30);
        let segundo = (p.add(0x18) as *const u64).read_unaligned();
        if !travar_fraco(h[1]) || (segundo != 0 && !travar_fraco(segundo)) {
            soltar(bloco, 1);
            continue;
        }
        (bloco as *mut [u64; 2]).write_unaligned(h);
        v.push(bloco as *mut [u64; 2]);
        crate::veredito_dev!("[a17e] v3: params {:p} do patch (template {:#x}) pra parte {caminho:#018x}", bloco, h[0]);
    }
    v
}

// ===== ArchiveXL `#51` 51f — `OnEntityPackageLoad`: `resource.patch` nos resultados do extrator do `EntityBuilder` (2026-09-12) =====
// A referência hookeia antes o `EntityBuilder::ScheduleExtractComponentsJob` e despacha na mesma fila, antes do job do motor, o `PatchPackageResults` nos
// resultados do extrator da entidade (e das aparências). No Mac o agendador é o `0x1019b3e70` (rótulo `EntityBuilder_ExtractComponents`; `x0` = builder,
// `x1` = fila; ele mesmo monta os `EntityBuilderJobParams` de 0x18 B) e o corpo do job é o `0x1019b74b0` (`x0` = params `{builder, WeakPtr}`, `x1` =
// `JobGroup`): trava o weak, toma os resultados do `entityExtractor` (`+0x10`) pelo `0x101076170` e lê o template em `+0x90` — o layout da referência.
// Aqui o hook fica no corpo, antes da original: os resultados já estão prontos e ainda no extrator (`+0xa8`). Pra cada patch da tabela com
// `components`/`entity` cujo alvo é o template do builder (ponteiro do token pedido na carga), o pacote do template do patch (`+0x1d8`) é extraído com as
// flags do extrator do builder (`+0xb8`–`+0xbc`; mudam por chamada no builder) e mesclado pela 17c/17f. O extrator é montado como o `ExtractPartComponents`
// (`0x1017d3158`) monta: bloco de params com o cabeçalho, o loader, as flags e o bitset de objetos; `0x1010763f0` cria, `SharedPtr` com weak próprio em
// `+0x00`, `0x101075dbc` extrai, `0x101076170` toma os resultados e a soltura segue a do motor. Antes da 1ª mescla, um controle: o template do próprio
// builder extraído com as flags e o bitset dele tem que dar o tamanho e as classes dos resultados do motor, com instâncias distintas; sem isso a mescla fica
// desligada. As mesclas de aparência (`PatchPackageResults` do `.app` e `IncludeAppearanceParts`) ficam pra outras partes.
pub(crate) const JOB_DO_BUILDER_VM: u64 = 0x1_019b_74b0;

pub(crate) const EXTRATOR_CRIAR_VM: u64 = 0x1_0107_63f0;

pub(crate) const EXTRATOR_EXTRAIR_VM: u64 = 0x1_0107_5dbc;

pub(crate) const EXTRATOR_TOMAR_RESULTADOS_VM: u64 = 0x1_0107_6170;

pub(crate) const EXTRATOR_DESTRUIR_VM: u64 = 0x1_0107_5714;

pub(crate) type JobDoBuilderFn = unsafe extern "C" fn(*mut [u64; 3], *const c_void);

/// Extrai o pacote compilado do template (`+0x1d8`) pra `saida` (`DynArray<Handle<ISerializable>>` vazio) com as `flags`; `bitset` `None` liga todos os
/// objetos. Devolve o tamanho; `None` sem pacote, com bitset de tamanho errado ou sem extrator.
pub(crate) unsafe fn extrair_pacote_do_template(template: u64, flags: [u8; 5], bitset: Option<Vec<u32>>, saida: *mut [u64; 2]) -> Option<usize> {
    if template == 0 {
        return None;
    }
    extrair_pacote_do_cabecalho(template + 0x1d8, flags, bitset, saida)
}

/// Controle da extração própria: template do builder com as flags e o bitset do extrator dele contra os resultados do motor. Devolve se bateu; o estado
/// global (`CONTROLE_DA_EXTRACAO`) é gravado só pelo 1º builder com resultados.
pub(crate) unsafe fn conferir_extracao_propria(ext: u64, tpl: u64, flags: [u8; 5]) -> bool {
    let ler32 = |o: u64| ((ext + o) as *const u32).read_unaligned();
    let (bits, palavras, objetos) = (((ext + 0x68) as *const u64).read_unaligned(), ler32(0x74) as usize, ler32(0x78) as usize);
    let bitset: Option<Vec<u32>> = (bits != 0 && palavras > 0 && palavras <= 0x800 && crate::memoria::is_readable(bits as *const c_void, palavras * 4))
        .then(|| (0..palavras).map(|i| ((bits as usize + i * 4) as *const u32).read_unaligned()).collect());
    let motor = handles_do_dynarray((ext + 0xa8) as *const u8);
    let mut meu = [0u64; 2];
    let tam = extrair_pacote_do_template(tpl, flags, bitset.clone(), &mut meu);
    let proprio = if tam.is_some() { handles_do_dynarray(meu.as_ptr() as *const u8) } else { Vec::new() };
    let classe = |h: &[u64; 2]| if h[0] == 0 { 0 } else { crate::rtti::type_name_hash(crate::rtti::class_of(h[0] as *mut c_void)) };
    let iguais = proprio.iter().zip(&motor).filter(|(a, b)| classe(a) == classe(b)).count();
    let distintas = proprio.iter().zip(&motor).all(|(a, b)| a[0] == 0 || a[0] != b[0]);
    let ok = !motor.is_empty() && proprio.len() == motor.len() && iguais == motor.len() && distintas;
    if tam.is_some() {
        soltar_dynarray_de_handles(meu.as_mut_ptr() as *mut u8);
    }
    crate::veredito_dev!(
        "[a51f] controle da extração no template {tpl:#x}: motor {} objeto(s), própria {tam:?} ({iguais} com a mesma classe, instâncias distintas {distintas}), bitset do extrator {} palavra(s) de {objetos} objeto(s), flags {flags:02x?}; extracao-ok={ok}",
        motor.len(),
        bitset.as_ref().map_or(0, |b| b.len())
    );
    ok
}

/// Se a mescla 51f deste builder segue, pelo controle global lido depois do CAS. Com o controle `EmCurso` (o 1º builder ainda extrai), o builder confere a
/// extração no próprio template: recusar monta a entidade sem o patch até a próxima carga, e esperar prenderia o worker do pool atrás da extração de outro
/// job. `Pendente` não acontece depois do CAS e recusa como `Falhou`.
pub(crate) fn mescla_no_builder_segue(controle: ControleDaExtracao, conferir_no_proprio: impl FnOnce() -> bool) -> bool {
    match controle {
        ControleDaExtracao::Ok => true,
        ControleDaExtracao::EmCurso => conferir_no_proprio(),
        ControleDaExtracao::Pendente | ControleDaExtracao::Falhou => false,
    }
}

/// 51f com o builder travado: mescla nos resultados do extrator da entidade os patches da tabela cujo alvo é o template do builder. Devolve o tamanho dos
/// resultados antes da mescla.
pub(crate) unsafe fn mesclar_no_builder_travado(builder: u64) -> usize {
    if !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return 0;
    }
    let ler = |o: u64| ((builder + o) as *const u64).read_unaligned();
    let (ext, tpl) = (ler(0x10), ler(0x90));
    let n = JOBS_DO_BUILDER_VISTOS.fetch_add(1, Ordering::Relaxed);
    let legivel = ext != 0 && crate::memoria::is_readable(ext as *const c_void, 0x120);
    let resultados = (ext + 0xa8) as *mut u8;
    let tamanho = || ((ext + 0xb4) as *const u32).read_unaligned() as usize;
    let tam = if legivel { tamanho() } else { 0 };
    let flags: [u8; 5] = if legivel { core::array::from_fn(|i| ((ext + 0xb8 + i as u64) as *const u8).read()) } else { [0; 5] };
    let caminho = hash_do_caminho_do_recurso(tpl);
    if n < 12 {
        crate::veredito_dev!(
            "[a51f] job de componentes #{n}: builder {builder:#x}, template {tpl:#x} (caminho {caminho:#018x}), extrator {ext:#x} com {tam} objeto(s), flags do extrator {flags:02x?}, flags do builder {:#04x}",
            ((builder + 0x110) as *const u8).read()
        );
    }
    if !legivel || tpl == 0 || tam == 0 {
        return tam;
    }
    if CONTROLE_DA_EXTRACAO
        .compare_exchange(ControleDaExtracao::Pendente as u8, ControleDaExtracao::EmCurso as u8, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        let ok = conferir_extracao_propria(ext, tpl, flags);
        CONTROLE_DA_EXTRACAO.store(if ok { ControleDaExtracao::Ok } else { ControleDaExtracao::Falhou } as u8, Ordering::Relaxed);
    }
    let instancia = |t: [u64; 2]| -> u64 {
        if t[0] != 0 && crate::register::status_do_token(t[0]) == 1 && crate::memoria::is_readable((t[0] + 0x28) as *const c_void, 8) {
            ((t[0] + 0x28) as *const u64).read_unaligned()
        } else {
            0
        }
    };
    let patches: Vec<(u64, [u64; 2], u32)> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g
            .do_alvo_ou_da_instancia((caminho != 0).then_some(caminho), tpl, |p| p.props & (A17_PROP_COMPONENTS | A17_PROP_ENTITY) != 0, instancia)
            .map(|p| (p.patch, p.token, p.props))
            .collect(),
        Err(_) => return tam,
    };
    if patches.is_empty() {
        return tam;
    }
    let controle = ControleDaExtracao::do_byte(CONTROLE_DA_EXTRACAO.load(Ordering::Relaxed));
    if !mescla_no_builder_segue(controle, || conferir_extracao_propria(ext, tpl, flags)) {
        if crate::primeiras(&MESCLAS_NO_BUILDER_RECUSADAS, 8) {
            crate::veredito_dev!("[a51f] template {tpl:#x} com {} patch(es), mescla recusada: controle da extração {controle:?}", patches.len());
        }
        return tam;
    }
    let raiz = |t: u64| -> i16 { if crate::memoria::is_readable((t + 0x1d8) as *const c_void, 2) { ((t + 0x1d8) as *const i16).read_unaligned() } else { -1 } };
    for (patch, pt, props) in patches {
        let ptpl = instancia(pt);
        if ptpl == 0 {
            A17_PATCHES_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let antes = tamanho();
        let mut p = [0u64; 2];
        let Some(tam_p) = extrair_pacote_do_template(ptpl, flags, None, &mut p) else {
            crate::veredito_dev!("[a51f] patch {patch:#018x} sem pacote extraível (template {ptpl:#x})");
            continue;
        };
        // A ordem da referência: `MergeComponents`, depois `MergeEntity`.
        let plano = if props & A17_PROP_COMPONENTS != 0 { mesclar_componentes(resultados, p.as_mut_ptr() as *mut u8, false) } else { Some(Vec::new()) };
        let troca = if props & A17_PROP_ENTITY != 0 { mesclar_entidade(resultados, p.as_mut_ptr() as *mut u8, raiz(tpl), raiz(ptpl)) } else { None };
        let depois = tamanho();
        let (s, x) = plano.as_ref().map_or((0, 0), |pl| {
            (pl.iter().filter(|q| matches!(q, PassoDaMescla::Substitui(_))).count(), pl.iter().filter(|q| **q == PassoDaMescla::Anexa).count())
        });
        if let Some(pl) = plano.as_ref() {
            let pares = pares_substituidos(pl, &handles_do_dynarray(resultados), &handles_do_dynarray(p.as_ptr() as *const u8));
            if let Ok(mut g) = A51F_AMOSTRAS.try_lock() {
                if g.len() < 8 && !pares.is_empty() {
                    g.push(A51fAmostra { builder, template: tpl, antes, depois, substituidos: s, anexados: x, pares: pares.into_iter().take(8).collect() });
                }
            }
        }
        soltar_dynarray_de_handles(p.as_mut_ptr() as *mut u8);
        if crate::primeiras(&MESCLAS_NO_BUILDER_APLICADAS, 16) {
            crate::veredito_dev!(
                "[a51f] patch {patch:#018x} no builder {builder:#x} (template {tpl:#x}): {tam_p} objeto(s) do patch, resultados {antes} → {depois}, {s} substituído(s), {x} anexado(s), raiz trocada {troca:?}{}",
                if plano.is_none() { ", mescla recusada" } else { "" }
            );
        }
    }
    tam
}

// ===== ArchiveXL `#51` 51f-2 — `PatchPackageResults(AppearanceResource)` nos extratores de aparência do `EntityBuilder` (2026-09-12) =====
// No mesmo job do builder, a referência aplica aos resultados do extrator de cada aparência (`appearance`, se a flag `ExtractAppearance` está ligada, e cada
// item de `appearances`, com `ExtractAppearances`) os componentes da definição homônima de cada `.app` de patch do recurso da aparência. O `compiledData`
// da definição do patch (buffer diferido em `+0xf0`) precisa estar carregado e com o cabeçalho lido em `+0x148`: a referência pede a carga e espera na
// fila. Aqui a carga é pedida no tick, com o token do patch pronto (`DeferredDataBuffer/CreateLoadingJobs` `0x101215524`, handle retido), e o cabeçalho é
// lido no hook quando falta, como o job `AppearanceDefinition_OpenCompiledData` (`0x1017d8d88`) lê: leitor `0x1019be2fc`, buffer `0x101074b44`, leitura
// `0x1019be360`, cópia de 0x70 B pra `+0x148`, destrutor `0x1019be348`. A extração usa o cabeçalho da definição e as flags do extrator da aparência; a
// mescla é a 17c sem parte. Layout de aparência do builder conferido no corpo do job: `appearance` em `+0x20` (definição), `+0x30` (recurso), `+0x40`
// (extrator); `appearances` em `+0x80`, elemento de 0x30 B com definição `+0x00`, recurso `+0x10`, extrator `+0x20`.
pub(crate) const APPDEF_COMPILEDDATA_OFF: u64 = 0xf0;

pub(crate) const APPDEF_CABECALHO_DO_PACOTE_OFF: u64 = 0x148;

pub(crate) const LEITOR_DO_PACOTE_CRIAR_VM: u64 = 0x1_019b_e2fc;

pub(crate) const LEITOR_DO_PACOTE_LER_VM: u64 = 0x1_019b_e360;

pub(crate) const LEITOR_DO_PACOTE_SOLTAR_VM: u64 = 0x1_019b_e348;

pub(crate) static MESCLA_DE_APARENCIA_ATIVA: AtomicBool = AtomicBool::new(false);

/// Definição do patch pra uma definição do alvo, como o registro do `OnAppearanceResourceLoad` da referência: a 1ª, na ordem do patch, com o mesmo nome ou sem
/// nome (`None`, patch multi-alvo).
pub(crate) fn definicao_do_patch(nomes: &[u64], alvo: u64) -> Option<usize> {
    nomes.iter().position(|n| *n == alvo || *n == 0)
}

pub(crate) static MESCLAS_DE_APARENCIA_APLICADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLAS_DE_APARENCIA_SEM_DEFINICAO: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLAS_DE_APARENCIA_SEM_DADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static CABECALHOS_DE_DEFINICAO_LIDOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static LEITURA_DE_DEFINICAO: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Objetos de um cabeçalho de pacote de 0x70 B: `(fim - início) / 8` dos ponteiros em `+0x20`/`+0x28`; 0 com cabeçalho vazio ou invertido.
pub(crate) fn objetos_do_cabecalho(cabecalho: &[u8; 0x70]) -> usize {
    let ler = |o: usize| u64::from_le_bytes([
        cabecalho[o],
        cabecalho[o + 1],
        cabecalho[o + 2],
        cabecalho[o + 3],
        cabecalho[o + 4],
        cabecalho[o + 5],
        cabecalho[o + 6],
        cabecalho[o + 7],
    ]);
    let (ini, fim) = (ler(0x20), ler(0x28));
    if ini == 0 || fim < ini { 0 } else { ((fim - ini) / 8) as usize }
}

/// Cópia do cabeçalho da definição (`+0x148`).
pub(crate) unsafe fn cabecalho_da_definicao(def: u64) -> [u8; 0x70] {
    let mut c = [0u8; 0x70];
    std::ptr::copy_nonoverlapping((def + APPDEF_CABECALHO_DO_PACOTE_OFF) as *const u8, c.as_mut_ptr(), 0x70);
    c
}

/// Dados e tamanho do `compiledData` carregado da definição (`DataBuffer` em `+0x20` do buffer diferido); `None` sem dados.
pub(crate) unsafe fn compileddata_da_definicao(def: u64) -> Option<(u64, u32)> {
    let obj = ((def + APPDEF_COMPILEDDATA_OFF + 0x20) as *const u64).read_unaligned();
    if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, 0x10) {
        return None;
    }
    let (dados, tam) = ((obj as *const u64).read_unaligned(), ((obj + 8) as *const u32).read_unaligned());
    (dados != 0 && tam > 0).then_some((dados, tam))
}

/// Cabeçalho da definição lido do `compiledData` carregado quando ainda está vazio, como o `OpenCompiledData` do motor. `true` com o cabeçalho pronto.
pub(crate) unsafe fn abrir_definicao(def: u64) -> bool {
    if def == 0 || !crate::memoria::is_readable(def as *const c_void, 0x1c0) {
        return false;
    }
    if objetos_do_cabecalho(&cabecalho_da_definicao(def)) > 0 {
        return true;
    }
    let Ok(_trava) = LEITURA_DE_DEFINICAO.lock() else { return false };
    if objetos_do_cabecalho(&cabecalho_da_definicao(def)) > 0 {
        return true;
    }
    let Some((dados, tam)) = compileddata_da_definicao(def) else {
        MESCLAS_DE_APARENCIA_SEM_DADOS.fetch_add(1, Ordering::Relaxed);
        return false;
    };
    let mut leitor = [0u8; 0x100];
    let mut local = [0u8; 0x70];
    let criar: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(LEITOR_DO_PACOTE_CRIAR_VM));
    let buffer: unsafe extern "C" fn(*mut u8, u64, u64) = std::mem::transmute(crate::rebase(crate::jogo::recursos::LEITOR_BUFFER_VM));
    let soltar: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(LEITOR_DO_PACOTE_SOLTAR_VM));
    criar(leitor.as_mut_ptr());
    buffer(leitor.as_mut_ptr(), dados, tam as u64);
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(LEITOR_DO_PACOTE_LER_VM) as u64,
        in("x0") leitor.as_mut_ptr() as u64,
        in("x8") local.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    let pronto = objetos_do_cabecalho(&local) > 0;
    if pronto {
        std::ptr::copy_nonoverlapping(local.as_ptr(), (def + APPDEF_CABECALHO_DO_PACOTE_OFF) as *mut u8, 0x70);
        CABECALHOS_DE_DEFINICAO_LIDOS.fetch_add(1, Ordering::Relaxed);
    }
    soltar(leitor.as_mut_ptr());
    pronto
}

/// 51f-2 com o builder travado: componentes da definição homônima de cada `.app` de patch do recurso nos resultados do extrator de uma aparência; com
/// `identidade` (teste), o próprio recurso entra como patch.
pub(crate) unsafe fn mesclar_patch_na_aparencia(builder: u64, tpl: u64, def: u64, res: u64, ext: u64, identidade: bool) {
    if def == 0 || res == 0 || ext == 0 || !crate::memoria::is_readable(ext as *const c_void, 0x120) || !crate::memoria::is_readable(def as *const c_void, 0x1c0) {
        return;
    }
    let caminho = hash_do_caminho_do_recurso(res);
    let nome = ((def + APPDEF_NAME_OFF as u64) as *const u64).read_unaligned();
    let instancia = |t: [u64; 2]| -> u64 {
        if t[0] != 0 && crate::register::status_do_token(t[0]) == 1 && crate::memoria::is_readable((t[0] + 0x28) as *const c_void, 8) {
            ((t[0] + 0x28) as *const u64).read_unaligned()
        } else {
            0
        }
    };
    let mut patches: Vec<(u64, u64)> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g.do_alvo(caminho).filter(|p| p.app && p.props & A17_PROP_COMPONENTS != 0).map(|p| (p.patch, instancia(p.token))).filter(|(_, r)| *r != 0).collect(),
        Err(_) => return,
    };
    if identidade {
        patches.push((caminho, res));
    }
    let resultados = (ext + 0xa8) as *mut u8;
    let tamanho = || ((ext + 0xb4) as *const u32).read_unaligned() as usize;
    let flags: [u8; 5] = core::array::from_fn(|i| ((ext + 0xb8 + i as u64) as *const u8).read());
    for (patch, pres) in patches {
        let pdef = definicoes_do_app(pres as *const u8)
            .and_then(|d| {
                let sem_nome = A51F2_TESTE_NONE.load(Ordering::Relaxed);
                let nomes: Vec<u64> = d.iter().map(|h| if sem_nome { 0 } else { ((h[0] + APPDEF_NAME_OFF as u64) as *const u64).read_unaligned() }).collect();
                definicao_do_patch(&nomes, nome).map(|j| d[j][0])
            })
            .unwrap_or(0);
        if pdef == 0 {
            MESCLAS_DE_APARENCIA_SEM_DEFINICAO.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        if !abrir_definicao(pdef) {
            continue;
        }
        let antes = tamanho();
        let mut p = [0u64; 2];
        let Some(tam_p) = extrair_pacote_do_cabecalho(pdef + APPDEF_CABECALHO_DO_PACOTE_OFF, flags, None, &mut p) else {
            crate::veredito_dev!("[a51f2] definição {nome:#018x} do patch {patch:#018x} sem pacote extraível");
            continue;
        };
        let hp = handles_do_dynarray(p.as_ptr() as *const u8);
        let plano = mesclar_componentes(resultados, p.as_mut_ptr() as *mut u8, false);
        let depois = tamanho();
        let (s, x) = plano.as_ref().map_or((0, 0), |pl| {
            (pl.iter().filter(|q| matches!(q, PassoDaMescla::Substitui(_))).count(), pl.iter().filter(|q| **q == PassoDaMescla::Anexa).count())
        });
        if let Some(pl) = plano.as_ref() {
            let mut pares = pares_substituidos(pl, &handles_do_dynarray(resultados), &handles_do_dynarray(p.as_ptr() as *const u8));
            pares.extend(instancias_anexadas(pl, &hp).into_iter().map(|i| (i, 0)));
            if let Ok(mut g) = A51F2_AMOSTRAS.try_lock() {
                if g.len() < 8 && !pares.is_empty() {
                    g.push(A51fAmostra { builder, template: tpl, antes, depois, substituidos: s, anexados: x, pares: pares.into_iter().take(8).collect() });
                }
            }
        }
        if identidade {
            if let Ok(mut g) = A51F2_RECURSOS.try_lock() {
                if g.len() < 16 {
                    g.push([tpl, res, nome]);
                }
            }
        }
        soltar_dynarray_de_handles(p.as_mut_ptr() as *mut u8);
        if crate::primeiras(&MESCLAS_DE_APARENCIA_APLICADAS, 16) {
            crate::veredito_dev!(
                "[a51f2] patch {patch:#018x} na aparência {nome:#018x} do builder {builder:#x} (template {tpl:#x}, .app {caminho:#018x}): {tam_p} objeto(s) do patch, resultados {antes} → {depois}, {s} substituído(s), {x} anexado(s){}",
                if plano.is_none() { ", mescla recusada" } else { "" }
            );
        }
    }
}

/// Flags do `EntityBuilder` em `+0x110` (u8): `ExtractAppearance` (bit 2) e `ExtractAppearances` (bit 3).
const BUILDER_EXTRAI_APARENCIA: u8 = 1 << 2;
const BUILDER_EXTRAI_APARENCIAS: u8 = 1 << 3;

/// 51f-2 no builder travado: a `appearance` (flag `ExtractAppearance`, bit 2 de `+0x110`) e cada item de `appearances` (`ExtractAppearances`, bit 3).
pub(crate) unsafe fn mesclar_patches_nas_aparencias(builder: u64) {
    if !MESCLA_DE_APARENCIA_ATIVA.load(Ordering::Relaxed) || !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return;
    }
    let ler = |o: u64| ((builder + o) as *const u64).read_unaligned();
    let tpl = ler(0x90);
    let flags = ((builder + 0x110) as *const u8).read();
    let identidade = A51F2_TESTE.load(Ordering::Relaxed) && a51f_teste_hashes().contains(&hash_do_caminho_do_recurso(tpl));
    if flags & BUILDER_EXTRAI_APARENCIA != 0 {
        mesclar_patch_na_aparencia(builder, tpl, ler(0x20), ler(0x30), ler(0x40), identidade);
    }
    if flags & BUILDER_EXTRAI_APARENCIAS != 0 {
        let (b, n, _) = dynarray_no_campo(builder as *const u8, 0x80);
        if n > 0 && n <= 64 && crate::memoria::is_readable(b as *const c_void, n * 0x30) {
            for i in 0..n {
                let e = b.add(i * 0x30) as u64;
                let r = |o: u64| ((e + o) as *const u64).read_unaligned();
                mesclar_patch_na_aparencia(builder, tpl, r(0x00), r(0x10), r(0x20), identidade);
            }
        }
    }
}

// ===== ArchiveXL `#51` 51f-3 — `IncludeAppearanceParts` no job de componentes do `EntityBuilder` (2026-09-12) =====
// A referência, antes do `PatchPackageResults`, inclui nos resultados do extrator de cada aparência do builder os componentes dos templates de `partsValues`
// da definição quando ela tem a tag `AppearanceParts` no `visualTags`, ou em todas quando o schema de tags do template do builder tem a tag: carrega cada
// parte (`LoadAsync` + espera do job), extrai o pacote do template da parte com as flags do extrator da aparência e mescla como parte (componente externo
// novo fica de fora). Aqui o token da parte é pedido uma vez e retido, e a espera é a da 51a (até 500 ms) no lugar da junção de job — parte que não ficou
// pronta sai daquela montagem e é contada. Liga com qualquer linha na tabela, sem depender de patch.
pub(crate) static INCLUSAO_DE_PARTES_ATIVA: AtomicBool = AtomicBool::new(false);

/// Hash do `CName` `AppearanceParts` (FNV-1a 64 do texto, como o construtor de `CName`).
pub(crate) fn tag_appearance_parts() -> u64 {
    static H: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *H.get_or_init(|| bwms_primitives::fnv1a64(b"AppearanceParts"))
}

/// Núcleo puro do `IncludeAppearanceParts`: a definição inclui as partes quando tem alguma e a tag no `visualTags`, ou quando o schema do template força.
pub(crate) fn definicao_inclui_partes(partes: usize, tags_definicao: &[u64], forcar: bool, tag: u64) -> bool {
    partes > 0 && (forcar || tags_definicao.contains(&tag))
}

/// Instâncias do patch que o plano anexa, com `patch` lido antes da mescla (que move os handles).
pub(crate) fn instancias_anexadas(plano: &[PassoDaMescla], patch: &[[u64; 2]]) -> Vec<u64> {
    plano.iter().enumerate().filter(|(j, p)| **p == PassoDaMescla::Anexa && *j < patch.len()).map(|(j, _)| patch[j][0]).collect()
}

/// Valores de 8 B de um `DynArray` (`CName` de `TagList`, caminho de `partsValues`), até 256. O cabeçalho (0x10 B) já vem conferido por quem
/// chama (`is_readable(def, 0x1c0)` e `is_readable(schema, 0x40)`): aqui só o buffer.
pub(crate) unsafe fn valores_u64_do_dynarray(arr: *const u8) -> Vec<u64> {
    if arr.is_null() {
        return Vec::new();
    }
    let (b, n, _) = dynarray_no_campo(arr, 0);
    if n == 0 || n > 256 || !crate::memoria::is_readable(b as *const c_void, n * 8) {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * 8) as *const u64).read_unaligned()).collect()
}

/// Token da parte pelo caminho, pedido uma vez pelo `LoadAsync` do motor (a referência pede na thread de job) e retido.
pub(crate) unsafe fn token_da_parte(caminho: u64) -> [u64; 2] {
    let Ok(mut g) = TOKENS_DE_PARTE.lock() else { return [0; 2] };
    if let Some((_, t)) = g.iter().find(|(c, _)| *c == caminho) {
        return *t;
    }
    let t = token_do_path(caminho).unwrap_or([0; 2]);
    if t[0] != 0 && g.len() < 4096 {
        g.push((caminho, t));
    }
    t
}

/// 51f-3 numa aparência do builder: partes da definição nos resultados do extrator dela. `teste` troca a lista pela parte de teste e força.
pub(crate) unsafe fn incluir_partes_na_aparencia(builder: u64, tpl: u64, def: u64, res: u64, ext: u64, forcar: bool, teste: bool) {
    if def == 0 || res == 0 || ext == 0 || !crate::memoria::is_readable(ext as *const c_void, 0x120) || !crate::memoria::is_readable(def as *const c_void, 0x1c0) {
        return;
    }
    let tag = tag_appearance_parts();
    let tags = valores_u64_do_dynarray((def + APPDEF_VISUALTAGS_OFF as u64) as *const u8);
    if tags.contains(&tag) {
        A51F3_COM_TAG.fetch_add(1, Ordering::Relaxed);
    }
    let partes = if teste { vec![bwms_primitives::resource_path_hash(A51B_TESTE_ALVO)] } else { valores_u64_do_dynarray((def + APPDEF_PARTSVALUES_OFF as u64) as *const u8) };
    if !partes.is_empty() {
        A51F3_DEFINICOES.fetch_add(1, Ordering::Relaxed);
    }
    if !definicao_inclui_partes(partes.len(), &tags, forcar || teste, tag) {
        return;
    }
    A51F3_INCLUSOES.fetch_add(1, Ordering::Relaxed);
    let nome = ((def + APPDEF_NAME_OFF as u64) as *const u64).read_unaligned();
    let resultados = (ext + 0xa8) as *mut u8;
    let tamanho = || ((ext + 0xb4) as *const u32).read_unaligned() as usize;
    let flags: [u8; 5] = core::array::from_fn(|i| ((ext + 0xb8 + i as u64) as *const u8).read());
    for parte in partes {
        let pres = recurso_do_patch_da_classe(token_da_parte(parte), 0x248, CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed)) as u64;
        if pres == 0 {
            if crate::primeiras(&A51F3_NAO_PRONTAS, 8) {
                crate::veredito_dev!("[a51f3] parte {parte:#018x} da aparência {nome:#018x} (.app {:#018x}) não ficou pronta na espera", hash_do_caminho_do_recurso(res));
            }
            continue;
        }
        let mut p = [0u64; 2];
        let Some(tam_p) = extrair_pacote_do_template(pres, flags, None, &mut p) else {
            A51F3_SEM_PACOTE.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let antes = tamanho();
        let hp = handles_do_dynarray(p.as_ptr() as *const u8);
        let plano = mesclar_componentes(resultados, p.as_mut_ptr() as *mut u8, true);
        let depois = tamanho();
        let (mut s, mut x) = (0usize, 0usize);
        if let Some(pl) = plano.as_ref() {
            A51F3_PARTES.fetch_add(1, Ordering::Relaxed);
            let anexados = instancias_anexadas(pl, &hp);
            s = pl.iter().filter(|q| matches!(q, PassoDaMescla::Substitui(_))).count();
            x = anexados.len();
            let mut pares = pares_substituidos(pl, &handles_do_dynarray(resultados), &handles_do_dynarray(p.as_ptr() as *const u8));
            pares.extend(anexados.iter().map(|&i| (i, 0)));
            if let Ok(mut g) = A51F3_AMOSTRAS.try_lock() {
                if g.len() < 8 && !pares.is_empty() {
                    g.push(A51fAmostra { builder, template: tpl, antes, depois, substituidos: s, anexados: x, pares: pares.into_iter().take(16).collect() });
                }
            }
        }
        soltar_dynarray_de_handles(p.as_mut_ptr() as *mut u8);
        if crate::primeiras(&A51F3_LOGADAS, 16) {
            crate::veredito_dev!(
                "[a51f3] parte {parte:#018x} na aparência {nome:#018x} do builder {builder:#x} (template {tpl:#x}, caminho {:#018x}, .app {:#018x}): {tam_p} objeto(s) da parte, resultados {antes} → {depois}, {s} substituído(s), {x} anexado(s){}",
                hash_do_caminho_do_recurso(tpl),
                hash_do_caminho_do_recurso(res),
                if plano.is_none() { ", mescla recusada" } else { "" }
            );
        }
    }
}

/// 51f-3 no job do builder, antes do `PatchPackageResults` (a ordem da referência): aparência atual e lista de aparências.
pub(crate) unsafe fn incluir_partes_no_builder(builder: u64) {
    if !INCLUSAO_DE_PARTES_ATIVA.load(Ordering::Relaxed) || !crate::memoria::is_readable(builder as *const c_void, 0x118) {
        return;
    }
    let ler = |o: u64| ((builder + o) as *const u64).read_unaligned();
    let tpl = ler(0x90);
    if tpl == 0 || !crate::memoria::is_readable(tpl as *const c_void, 0x248) {
        return;
    }
    let schema = ((tpl + TEMPLATE_VISUALTAGSSCHEMA_OFF as u64) as *const u64).read_unaligned();
    let forcar = schema != 0
        && crate::memoria::is_readable(schema as *const c_void, SCHEMA_VISUALTAGS_OFF + 0x10)
        && valores_u64_do_dynarray((schema + SCHEMA_VISUALTAGS_OFF as u64) as *const u8).contains(&tag_appearance_parts());
    if forcar {
        A51F3_SCHEMAS_COM_TAG.fetch_add(1, Ordering::Relaxed);
    }
    let teste = A51F3_TESTE.load(Ordering::Relaxed) && a51f_teste_hashes().contains(&hash_do_caminho_do_recurso(tpl));
    let flags = ((builder + 0x110) as *const u8).read();
    let extrai_aparencia = flags & BUILDER_EXTRAI_APARENCIA != 0;
    if extrai_aparencia {
        incluir_partes_na_aparencia(builder, tpl, ler(0x20), ler(0x30), ler(0x40), forcar, teste);
    }
    // No teste só a aparência atual recebe a parte: a lista de aparências pode repetir o extrator e trocaria as instâncias da 1ª inclusão.
    if flags & BUILDER_EXTRAI_APARENCIAS != 0 && !(teste && extrai_aparencia) {
        let (b, n, _) = dynarray_no_campo(builder as *const u8, 0x80);
        if n > 0 && n <= 64 && crate::memoria::is_readable(b as *const c_void, n * 0x30) {
            for i in 0..n {
                let e = b.add(i * 0x30) as u64;
                let r = |o: u64| ((e + o) as *const u64).read_unaligned();
                incluir_partes_na_aparencia(builder, tpl, r(0x00), r(0x10), r(0x20), forcar, teste);
            }
        }
    }
}

// ===== ArchiveXL `#51` 51i-1 — `resource.fix` names no `OnMeshResourceLoad` (2026-09-12) =====
// A referência, no começo do `OnMeshResourceLoad`, troca cada `chunkMaterials` de cada aparência e o `name` de cada `materialEntries` pelo nome mapeado no
// fix do caminho do mesh (`ResourceMetaExtension::GetFix`; configs juntadas pelo `Merge`, o nome antigo repetido fica com o novo do último config). Aqui a
// tabela `bwms-resfixes.txt` (do `emit_resfixes`) entra na carga das tabelas e a troca roda no hook do `CMesh::PostLoad`, antes da original. Layout (SDK,
// conferido pelo RTTI): `CMesh.appearances` `+0x1E0` (`Handle<meshMeshAppearance>`), `materialEntries` `+0xE0` (`CMeshMaterialEntry` 0x30 B, `name` `+0x20`),
// `meshMeshAppearance.chunkMaterials` `+0x38`.
pub(crate) static CMESH_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static FIXES_DE_NOME_DO_MESH: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<(u64, u64)>>>> = std::sync::RwLock::new(None);

/// Offsets do `CMesh`, do `meshMeshAppearance` e do `CMeshMaterialEntry` pelo RTTI (game thread), conferidos contra o SDK.
pub(crate) unsafe fn guardar_offsets_do_cmesh(reg: &crate::rtti::Registry) -> bool {
    let off = |classe: &str, campo: &str| {
        let c = reg.class_by_name(classe);
        if c.is_null() { None } else { crate::rtti::resolve_prop_in_class(c, campo).map(|(o, _, _)| o as usize) }
    };
    let lidos = [
        ("CMesh.appearances", off("CMesh", "appearances"), 0x1e0),
        ("CMesh.materialEntries", off("CMesh", "materialEntries"), 0xe0),
        ("meshMeshAppearance.chunkMaterials", off("meshMeshAppearance", "chunkMaterials"), 0x38),
        ("CMeshMaterialEntry.name", off("CMeshMaterialEntry", "name"), 0x20),
    ];
    let ok = lidos.iter().all(|(_, o, e)| *o == Some(*e));
    CMESH_OFFSETS_OK.store(ok, Ordering::Relaxed);
    crate::veredito_dev!(
        "[a51i] offsets pelo RTTI: {}; fix de nomes {}",
        lidos.iter().map(|(n, o, e)| format!("{n} {} (SDK {e:#x})", o.map_or("?".to_string(), |o| format!("{o:#x}")))).collect::<Vec<_>>().join(", "),
        if ok { "ligado" } else { "desligado" }
    );
    ok
}

/// Carga da tabela de fix de nomes (game thread) com os `names` de `bwms-resfixes.txt` (`mods_do_usuario::pipeline::resfixes_linhas`; vazio sem o
/// arquivo). Com linha, ou com `~/.bwms-hook-a51i-teste` (o teste registra o fix no tick), confere os offsets e instala o hook. Devolve as linhas carregadas.
/// Sem linha, com `substituir` (o `ArchiveXL.Reload`), o fix da carga anterior sai; o hook fica, sem fix a aplicar.
pub(crate) unsafe fn carregar_fixes_de_nome_do_mesh(nomes: std::collections::HashMap<u64, Vec<(u64, u64)>>, substituir: bool) -> usize {
    let teste = marcador_individual("a51i-teste");
    if nomes.is_empty() && !teste {
        if substituir {
            if let Ok(mut g) = FIXES_DE_NOME_DO_MESH.write() {
                *g = None;
            }
        }
        return 0;
    }
    let Some(reg) = crate::rtti::Registry::obtain() else { return 0 };
    if !guardar_offsets_do_cmesh(&reg) {
        return 0;
    }
    let (linhas, recursos) = (nomes.values().map(Vec::len).sum::<usize>(), nomes.len());
    if let Ok(mut g) = FIXES_DE_NOME_DO_MESH.write() {
        *g = Some(nomes);
    }
    install_cmesh_postload_hook();
    crate::veredito_dev!(
        "[a51i] fix de nomes: {linhas} linha(s) de {recursos} recurso(s), teste {teste}; hook do CMesh::PostLoad {}",
        if ORIG_CMESH_POSTLOAD.load(Ordering::Relaxed) == 0 { "FALHOU" } else { "instalado" }
    );
    linhas
}

/// `context` do `resource.fix` (S12): caminho do mesh → atributos `(CName do nome em snake_case, valor)`, entregues ao estado de mesh dinâmico
/// NATIVO da 2.5 (`z040/relatorios/S12a.md`): o jogo faz o mesh dinâmico do ArchiveXL sozinho, mas nada chama o `PrepareMeshState` dele com contexto.
pub(crate) static CONTEXTOS_DO_MESH: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<(u64, String)>>>> = std::sync::RwLock::new(None);

/// `App::MeshExtension::PrepareMeshState(CMesh*, const Core::Map<CName, std::string>&)` da 2.5: cria o estado, zera o `contextToken` (o `@context`
/// do mesh fica de fora, como na referência), insere os atributos sem sobrescrever e resolve o `appearance_expansion_source`.
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

/// Clone de uma `meshMeshAppearance` do patch com as 3 tags, num `Handle` novo; `None` sem instância.
pub(crate) unsafe fn clonar_aparencia_do_mesh(pinst: *const u8, expansao: u64, origem: u64) -> Option<[u64; 2]> {
    let obj = crate::rtti::create_instance(CLASSE_MESHMESHAPPEARANCE.load(Ordering::Relaxed));
    if obj.is_null() {
        return None;
    }
    let mut h = [0u64; 2];
    crate::rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj);
    if h[0] == 0 || h[1] == 0 {
        return None;
    }
    let o = obj as *mut u8;
    (o.add(0x30) as *mut u64).write_unaligned((pinst.add(0x30) as *const u64).read_unaligned());
    let chunks_ok = copiar_dynarray(o.add(0x38), pinst.add(0x38), 8, 8);
    let tags = o.add(0x58);
    crate::jogo::listas::reservar(tags as *mut c_void, 3, 8, 8, std::ptr::null());
    if !chunks_ok || dynarray_no_campo(tags, 0).2 < 3 {
        crate::jogo::objetos::soltar_handle(&mut h);
        return None;
    }
    let tb = dynarray_no_campo(tags, 0).0;
    for (k, v) in [tag_resource_patch(), expansao, origem].into_iter().enumerate() {
        (tb.add(k * 8) as *mut u64).write_unaligned(v);
    }
    (tags.add(0x0c) as *mut u32).write_unaligned(3);
    Some(h)
}

// ===== ArchiveXL `#51` 51i-3 — `renderResourceBlob` do `OnMeshResourceLoad` (`CopyRenderBlob`, 2026-09-12) =====
// Referência: com blob no patch e `Modifies(renderResourceBlob, alvo tem blob)`, o alvo recebe um `rendRenderMeshBlob` novo com o `header` do patch (cópia) e
// o `renderBuffer` copiado (`Initialize` com o alocador, o tamanho e o alinhamento do patch + `memcpy`), mais 16 campos do mesh do patch (listas de ossos, LOD e
// trilhas, caixa, área por eixo, tipo de objeto e flags de sombra). Layout (SDK): blob `header` `+0x30` (0xE0 B), `renderBuffer` `+0x110`; header com 10
// `DynArray` — `bonePositions` (Vector4), `renderLODs` (float), `renderChunks` e mais 5 listas de u8, `renderChunkInfos` (`rend::Chunk` 0xF8 B, `materialId`
// `DynArray<CName>` em `+0xD8`), `topology` (`rend::TopologyData` 0x28 B, `data`/`metadata` u8 em `+0x08`/`+0x18`) — e os escalares de `+0x00` e de `+0xA8` a
// `+0xE0`. Blob novo e `Handle` pelo motor, como os clones da 51i-2; o `renderBuffer` aloca pelo `RawBufferAllocator` copiado do patch (51g v2).
pub(crate) static MESCLA_DE_RENDER_BLOB_ATIVA: AtomicBool = AtomicBool::new(false);

/// Blob muda com blob no patch e, com props escritas, a prop `renderResourceBlob`; com `-`, só se o alvo não tem blob (o `Modifies(p, alvo tem blob)`).
pub(crate) fn render_blob_muda(props: u32, patch_tem: bool, alvo_tem: bool) -> bool {
    patch_tem && if props == u32::MAX { !alvo_tem } else { props & A17_PROP_RENDERBLOB != 0 }
}

// ===== ArchiveXL `#51` 51d v3 — `blob` do `OnMorphTargetResourceLoad` (`CopyRenderBlob` do morph, 2026-09-12) =====
// Referência: com blob no patch e `Modifies(blob, alvo tem blob)`, e o blob do patch sendo `rendRenderMorphTargetMeshBlob`, o alvo recebe um blob novo — se o
// `baseBlob` do patch é `rendRenderMeshBlob`: o header do patch (cópia), o `baseBlob` copiado (o `CopyRenderBlob` do mesh), os `DataBuffer` `diffsBuffer` e
// `mappingBuffer` copiados e o `DeferredDataBuffer` `textureDiffsBuffer` clonado; senão o blob novo fica vazio. Aqui o header
// (`rend::RenderMorphTargetMeshBlobHeader`, 0x80 B com listas de listas) e o `textureDiffsBuffer` saem pelo `Assign` do tipo do motor (`vt+0x58`, conferido ao
// vivo na A#44) — o operador de cópia que a referência emula —; o `baseBlob` e os `DataBuffer`, pelas cópias da 51i-3.
pub(crate) static MORPH_BLOB_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

/// Blob do morph muda com blob no patch e, com props escritas, a prop `blob`; com `-`, só se o alvo não tem blob (o `Modifies(p, alvo tem blob)`).
pub(crate) fn morph_blob_muda(props: u32, patch_tem: bool, alvo_tem: bool) -> bool {
    patch_tem && if props == u32::MAX { !alvo_tem } else { props & A17_PROP_BLOB != 0 }
}

// ===== ArchiveXL `#48` — `MeshExtension`, lógica pura (2026-09-13) =====
// Nomes da `MeshExtension` da referência, sem motor: material de template começa com `@`, material dinâmico tem `@` em qualquer ponto, e a aparência sem
// `chunkMaterials` é expandida a partir da aparência de origem (a da tag de expansão do `#51`, senão a do atributo `appearance_expansion_source` do contexto,
// senão a 1ª). Os hooks (`FindAppearance`/`AddStubAppearance`/`ShouldPreloadAppearances`/`LoadMaterialsAsync`) usam isto em cima da memória do `CMesh`.
pub(crate) const MARCADOR_DE_TEMPLATE: char = '@';

/// `IsTemplateMaterial`: o nome começa com `@`.
pub(crate) fn material_de_template(nome: &str) -> bool {
    nome.starts_with(MARCADOR_DE_TEMPLATE)
}

/// `IsDynamicMaterial`: o nome tem `@` em qualquer ponto.
pub(crate) fn material_dinamico(nome: &str) -> bool {
    nome.contains(MARCADOR_DE_TEMPLATE)
}

/// `IsDynamicMesh`: sem entradas de material, com entrada de template, sem aparências, com aparência sem `chunkMaterials` ou com material dinâmico.
pub(crate) fn mesh_dinamico<M, C>(materiais: &[M], aparencias: &[Vec<C>], template: impl Fn(&M) -> bool, dinamico: impl Fn(&C) -> bool) -> bool {
    materiais.is_empty()
        || materiais.iter().any(&template)
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

/// Bits do texto do CName: 1 = tem `@`, 2 = começa com `@`. Com cache por hash, porque o `PostLoad` confere todos os nomes de cada mesh; sem texto: 0, fora do
/// cache.
pub(crate) unsafe fn bits_de_arroba(hash: u64) -> u8 {
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<u64, u8>>> = std::sync::Mutex::new(None);
    if hash == 0 {
        return 0;
    }
    if let Some(v) = CACHE.lock().ok().and_then(|g| g.as_ref().and_then(|m| m.get(&hash).copied())) {
        return v;
    }
    let Some(t) = texto_do_cname(hash) else { return 0 };
    let v = material_dinamico(&t) as u8 * BIT_MATERIAL_DINAMICO | material_de_template(&t) as u8 * BIT_MATERIAL_DE_TEMPLATE;
    if let Ok(mut g) = CACHE.lock() {
        let m = g.get_or_insert_with(std::collections::HashMap::new);
        if m.len() < 1 << 20 {
            m.insert(hash, v);
        }
    }
    v
}

/// Hashes de um `DynArray<CName>` em `obj + off`.
pub(crate) unsafe fn cnames_do_dynarray(obj: *const u8, off: usize) -> Vec<u64> {
    let (b, n, _) = dynarray_no_campo(obj, off);
    if b.is_null() || n > 0x1_0000 {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * 8) as *const u64).read_unaligned()).collect()
}

/// Instâncias das aparências do mesh (nula na entrada vazia).
pub(crate) unsafe fn aparencias_do_mesh(mesh: *const u8) -> Vec<*mut u8> {
    let (b, n, _) = dynarray_no_campo(mesh, CMESH_APPEARANCES_OFF);
    if b.is_null() || n > 0x1_0000 {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * 16) as *const u64).read_unaligned() as *mut u8).collect()
}

/// Nomes das entradas de material do mesh.
pub(crate) unsafe fn nomes_dos_materiais_do_mesh(mesh: *const u8) -> Vec<u64> {
    let (b, n, _) = dynarray_no_campo(mesh, CMESH_MATERIALENTRIES_OFF);
    if b.is_null() || n > 0x1_0000 {
        return Vec::new();
    }
    (0..n).map(|i| (b.add(i * CMESHMATERIALENTRY_TAMANHO + 0x20) as *const u64).read_unaligned()).collect()
}

/// `MeshState` sem contexto (48d-2): entradas de template (`@…`) e comuns, por nome, e as malhas de patch registradas como origem (48d-1).
#[derive(Default)]
pub(crate) struct EstadoDoMesh {
    pub(crate) templates: std::collections::HashMap<u64, usize>,
    pub(crate) materiais: std::collections::HashMap<u64, usize>,
    /// `sources` do `RegisterMeshPatch`: tag de origem (hash do caminho do patch) → `WeakHandle<CMesh>` (com 1 fraco nosso).
    pub(crate) origens: std::collections::HashMap<u64, [u64; 2]>,
    /// `appearance_expansion_source` do contexto (48d-2); 0 sem contexto.
    pub(crate) fonte_expansao: u64,
    /// 48d-2: o `@context` já foi consultado (`EnsureContextReady` roda uma vez).
    pub(crate) contexto_lido: bool,
    /// 48d-2 `contextAttrs`: nome do parâmetro `CName` em snake_case → valor.
    pub(crate) atributos: std::collections::HashMap<u64, u64>,
    /// 48d-2 `contextParams`: nomes de todos os parâmetros da instância de contexto (os valores ficam pro 48c).
    pub(crate) parametros_contexto: Vec<u64>,
    /// 48c-2: a instância do `@context` (fraca, com 1 fraco nosso), pros valores dos parâmetros no clone.
    pub(crate) contexto: [u64; 2],
}

impl EstadoDoMesh {
    /// `FillMaterials`: refaz o mapa de entradas comuns; os templates acumulam, como na referência.
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

/// `PrepareMeshState` no `PostLoad` do mesh (thread de job, antes da original): estado novo pro mesh dinâmico, só com leitura de memória.
pub(crate) unsafe fn preparar_estado_do_mesh(mesh: *mut c_void) {
    if !MESH_DINAMICO_ATIVO.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let m = mesh as *const u8;
    let caminho = (m.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    if caminho == 0 || estado_do_mesh(caminho).is_some() {
        return;
    }
    let aparencias: Vec<Vec<u64>> = aparencias_do_mesh(m).iter().filter(|a| !a.is_null()).map(|a| cnames_do_dynarray(*a, MESHAPPEARANCE_CHUNKMATERIALS_OFF)).collect();
    if !mesh_dinamico(&nomes_dos_materiais_do_mesh(m), &aparencias, |n| bits_de_arroba(*n) & BIT_MATERIAL_DE_TEMPLATE != 0, |c| bits_de_arroba(*c) & BIT_MATERIAL_DINAMICO != 0) {
        return;
    }
    let mut estado = EstadoDoMesh::default();
    estado.preencher_materiais(m);
    if let Ok(mut g) = ESTADOS_DOS_MESHES.write() {
        g.get_or_insert_with(std::collections::HashMap::new)
            .entry(caminho)
            .or_insert_with(|| std::sync::Arc::new(std::sync::Mutex::new(estado)));
        A48_ESTADOS_CRIADOS.fetch_add(1, Ordering::Relaxed);
    }
}

/// 48d-1 `RegisterMeshPatch`, no `PostLoad` do alvo (51i-2, antes de clonar as aparências do patch): estados do alvo e do patch, o patch guardado fraco
/// no alvo pela tag de origem (o hash do caminho do patch) e, com o alvo já tendo entradas, uma entrada local com o nome da tag apontando o material
/// vazio. Devolve a tag; 0 com o 48a desligado ou patch sem `materialEntries`. Divergência: a entrada da tag não repete quando o mesmo patch registra de
/// novo no mesmo mesh.
pub(crate) unsafe fn registrar_patch_de_mesh(alvo: *mut u8, patch: *mut u8) -> u64 {
    let vazio = MATERIAL_VAZIO.load(Ordering::Relaxed);
    if !MESH_DINAMICO_ATIVO.load(Ordering::Relaxed) || alvo.is_null() || patch.is_null() || vazio.is_null() || dynarray_no_campo(patch, CMESH_MATERIALENTRIES_OFF).1 == 0 {
        return 0;
    }
    let tag = (patch.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let (Some(estado), Some(_)) = (adquirir_estado_do_mesh(alvo), adquirir_estado_do_mesh(patch)) else { return 0 };
    let mut forte = handle_forte_de(patch);
    if tag == 0 || forte[0] == 0 {
        return 0;
    }
    let Ok(mut e) = estado.lock() else {
        crate::jogo::objetos::soltar_handle(&mut forte);
        return 0;
    };
    let mut fraco = [0u64; 2];
    gravar_handle_fraco(fraco.as_mut_ptr(), forte);
    if let Some(mut antigo) = e.origens.insert(tag, fraco) {
        crate::jogo::objetos::soltar_fraco(&mut antigo);
    }
    crate::jogo::objetos::soltar_handle(&mut forte);
    if dynarray_no_campo(alvo, CMESH_MATERIALENTRIES_OFF).1 > 0 && !e.materiais.contains_key(&tag) {
        criar_entradas_locais(alvo, &mut e, &[tag], vazio.read());
    }
    A48_PATCHES_REGISTRADOS.fetch_add(1, Ordering::Relaxed);
    tag
}

/// Entradas locais novas pros `nomes` apontando `material` (forte e fraco), sob o `MaterialLock` do mesh — o `LoadMaterialsAsync` do motor lê as entradas
/// com ele. Trava ocupada por tempo demais: nada entra (conta).
pub(crate) unsafe fn criar_entradas_locais(mesh: *mut u8, estado: &mut EstadoDoMesh, nomes: &[u64], material: [u64; 2]) -> bool {
    if nomes.is_empty() {
        return false;
    }
    let trava = mesh.add(CMESH_MATERIALLOCK_OFF);
    let tentar: unsafe extern "C" fn(*mut u8) -> bool = std::mem::transmute(crate::rebase(TRAVA_TENTAR_ESCRITA_VM));
    let mut pegou = false;
    for _ in 0..10_000 {
        if tentar(trava) {
            pegou = true;
            break;
        }
        std::hint::spin_loop();
    }
    if !pegou {
        A48_TRAVA_OCUPADA.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    let mut feito = false;
    let lista = mesh.add(CMESH_MATERIALENTRIES_OFF);
    let (_, tam, cap) = dynarray_no_campo(lista, 0);
    let novo = tam + nomes.len();
    if novo > cap {
        crate::jogo::listas::reservar(lista as *mut c_void, novo as u32, CMESHMATERIALENTRY_TAMANHO as u32, 8, std::ptr::null());
    }
    if dynarray_no_campo(lista, 0).2 >= novo {
        let b = dynarray_no_campo(lista, 0).0;
        let [inst, bloco] = material;
        for (k, nome) in nomes.iter().enumerate() {
            let e = b.add((tam + k) * CMESHMATERIALENTRY_TAMANHO);
            std::ptr::write_bytes(e, 0, CMESHMATERIALENTRY_TAMANHO);
            handle_strong_soma(&[inst, bloco], 1);
            (*((bloco as usize + 4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
            for (off, v) in [(0x00, inst), (0x08, bloco), (0x10, inst), (0x18, bloco), (0x20, *nome)] {
                (e.add(off) as *mut u64).write_unaligned(v);
            }
            e.add(0x2a).write(1);
            estado.materiais.insert(*nome, tam + k);
        }
        (lista.add(0x0c) as *mut u32).write_unaligned(novo as u32);
        feito = true;
    }
    crate::jogo::travas::soltar_escrita((trava) as *mut u8);
    feito
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

// ===== ArchiveXL `#23` — estado do `DynamicAppearanceController` (2026-09-12) =====
// `UpdateState(entidade, item equipado)`: `gender`/`camera`/`body`/`arms`/`feet`/`sleeves`/`hair_type` pelo `GetSuffixData` (o valor do sufixo do `#9`,
// com `Male` → `m` e o resto em snake_case) e `skin_color`/`hair_color`/`eyes_color` pelo `GetCustomizationData` (aparência dos componentes de corpo e olhos,
// último na ordem reversa vence; cor do cabelo pelo `GetHairColor`). Condições = FNV-1a64 de `atributo=valor` (o encadeado da referência dá o mesmo hash).
// `ResolveName`/`ResolvePath`: `ProcessString` com os valores do estado como globais e a variante como locais. `GetHairColor` também é inlinado no Mac
// (`0x102045888`, chamado pelo ItemFactory em `0x10200defc`/`0x102014998`): estado pela `vt+0x1f8(sistema, é masculino)` (sret), e a cor é o 1º `CName` do
// flat `ItemFactory.HairColors.hairColors` que passa na `vt+0x240` do estado.
pub(crate) const ATRIBUTOS_DE_SUFIXO: [(&str, &str); 7] = [
    ("gender", "itemsFactoryAppearanceSuffix.Gender"),
    ("camera", "itemsFactoryAppearanceSuffix.Camera"),
    ("body", "itemsFactoryAppearanceSuffix.BodyType"),
    ("arms", "itemsFactoryAppearanceSuffix.ArmsState"),
    ("feet", "itemsFactoryAppearanceSuffix.FeetState"),
    ("sleeves", "itemsFactoryAppearanceSuffix.Partial"),
    ("hair_type", "itemsFactoryAppearanceSuffix.HairType"),
];

/// Resultado do `ProcessString`.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct TextoProcessado {
    pub valid: bool,
    pub missed: bool,
    pub optional: bool,
    pub value: String,
    /// `result.attributes`: hash de cada `{atributo}` lido, com ou sem valor (a reserva do `ResolvePath`, ArchiveXL `#20` 20g, escolhe entre eles).
    pub atributos: std::collections::BTreeSet<u64>,
}

/// `DynamicAppearanceController::ProcessString`: `{atributo}` pelo local, senão pelo global (sem nenhum, `missed`); `*` inicial pulado; `?` final vira
/// `optional`; texto sem nenhum `{}` ou com par aberto é inválido; saída até 512 bytes.
pub(crate) fn processar_texto_dinamico(globais: &std::collections::HashMap<u64, String>, locais: &std::collections::HashMap<u64, String>, entrada: &str) -> TextoProcessado {
    process_dynamic_string_com(entrada, |h| locais.get(&h).or_else(|| globais.get(&h)).map(String::as_str))
}

/// Condição do estado: `FNV1a64(valor, FNV1a64("=", atributo))` = FNV-1a64 de `atributo=valor`.
pub(crate) fn condicao_do_estado(atributo: &str, valor: &str) -> u64 {
    bwms_primitives::fnv1a64(format!("{atributo}={valor}").as_bytes())
}

/// `ResolveName`: nome sem `*` volta igual; inválido volta igual; opcional com atributo faltando vira vazio.
pub(crate) fn resolver_nome_dinamico(valores: &std::collections::HashMap<u64, String>, locais: &std::collections::HashMap<u64, String>, nome: &str) -> String {
    if !nome.starts_with('*') {
        return nome.to_string();
    }
    let r = processar_texto_dinamico(valores, locais, nome);
    if !r.valid {
        nome.to_string()
    } else if r.optional && r.missed {
        String::new()
    } else {
        r.value
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

/// `GetSuffixData`: com registro de sufixo, o valor do `#9` pro item `{equipado}`; Gender vira `m`/`w`, os outros snake_case.
pub(crate) unsafe fn dado_do_sufixo(reg: &crate::rtti::Registry, dono_h: &[u64; 2], sufixo: u64, equipado: u64) -> (String, String) {
    let mut suf = String::new();
    let mut rec = a9_registro(sufixo);
    if rec[0] != 0 {
        crate::jogo::objetos::soltar_handle(&mut rec);
        // `ItemID{tdbid}`: semente 2 (o padrão do SDK), resto zerado.
        let mut item = [0u8; 16];
        item[..8].copy_from_slice(&equipado.to_le_bytes());
        item[8..12].copy_from_slice(&2u32.to_le_bytes());
        suf = a9_valor_do_sufixo(reg, dono_h, &item, sufixo);
    }
    let valor = if bwms_primitives::tweak_db_id_chave(sufixo) == bwms_primitives::tweak_db_id("itemsFactoryAppearanceSuffix.Gender") {
        if suf == "Male" { "m" } else { "w" }.to_string()
    } else if suf.is_empty() {
        String::new()
    } else {
        para_snake_case(&suf)
    };
    (valor, suf)
}

/// `meshAppearance` do componente pelo RTTI da classe dele (mesh, skinned e morph target); vazio sem o campo.
pub(crate) unsafe fn nome_da_aparencia_do_componente(comp: *mut c_void) -> String {
    let Some((off, _, _)) = crate::rtti::resolve_prop_in_class(crate::rtti::class_of(comp), "meshAppearance") else { return String::new() };
    let p = (comp as *const u8).add(off as usize) as *const u64;
    if !crate::memoria::is_readable(p as *const c_void, 8) {
        return String::new();
    }
    let h = p.read_unaligned();
    if h == 0 { String::new() } else { crate::cname::resolve_cname(h) }
}

/// Componentes de unha do V (ArchiveXL 1.27 `{nails_color}`): masculino, feminino em primeira pessoa e feminino em terceira.
pub(crate) const COMPONENTES_DE_UNHA: [u64; 3] = [
    crate::cname::cname("a0_000_pma_base__nails_l"),
    crate::cname::cname("a0_000_pwa_fpp__nails_l"),
    crate::cname::cname("a0_000_pwa_base_nails_l"),
];

/// `GetCustomizationData` sem a cor do cabelo: `(é masculino, pele, olhos, unhas)` pelos componentes em ordem reversa.
pub(crate) unsafe fn dados_de_customizacao(entidade: *mut c_void) -> (bool, String, String, String) {
    let (mut masc, mut pele, mut olhos, mut unhas) = (false, String::new(), String::new(), String::new());
    let arr = (entidade as *const u8).add(0xA0);
    if !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return (masc, pele, olhos, unhas);
    }
    let (dados, n) = ((arr as *const u64).read_unaligned(), (arr.add(0x0c) as *const u32).read_unaligned() as u64);
    if dados == 0 || n > 4096 || !crate::memoria::is_readable(dados as *const c_void, (n * 16) as usize) {
        return (masc, pele, olhos, unhas);
    }
    let h = |s: &str| bwms_primitives::fnv1a64(s.as_bytes());
    for i in (0..n).rev() {
        let comp = ((dados + i * 16) as *const u64).read_unaligned();
        if comp == 0 || !crate::memoria::is_readable((comp + 0x40) as *const c_void, 8) {
            continue;
        }
        let nome = ((comp + 0x40) as *const u64).read_unaligned();
        if nome == h("t0_000_pma_base__full") {
            masc = true;
            pele = nome_da_aparencia_do_componente(comp as *mut c_void);
        } else if nome == h("t0_000_pwa_base__full") || nome == h("t0_000_pwa_fpp__torso") {
            masc = false;
            pele = nome_da_aparencia_do_componente(comp as *mut c_void);
        } else if nome == h("he_000_pma__basehead") || nome == h("MorphTargetSkinnedMesh3637") {
            olhos = nome_da_aparencia_do_componente(comp as *mut c_void);
        } else if COMPONENTES_DE_UNHA.contains(&nome) {
            unhas = nome_da_aparencia_do_componente(comp as *mut c_void);
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

/// `UpdateState(entidade, item equipado)` pro player (game thread).
pub(crate) unsafe fn estado_dinamico_do_player(reg: &crate::rtti::Registry, player: *mut c_void, equipado: u64) -> Option<EstadoDinamico> {
    if player.is_null() {
        return None;
    }
    let mut dono = [0u64; 2];
    crate::rtti::make_handle(dono.as_mut_ptr() as *mut c_void, player);
    if dono[0] == 0 {
        return None;
    }
    let mut e = EstadoDinamico::default();
    let mut nomes: Vec<(&str, String)> = Vec::new();
    for (atributo, sufixo) in ATRIBUTOS_DE_SUFIXO {
        let (v, s) = dado_do_sufixo(reg, &dono, bwms_primitives::tweak_db_id(sufixo), equipado);
        nomes.push((atributo, v.clone()));
        e.valores.insert(bwms_primitives::fnv1a64(atributo.as_bytes()), (v, s));
    }
    crate::jogo::objetos::soltar_handle(&mut dono);
    let (masc, pele, olhos, unhas) = dados_de_customizacao(player);
    let cabelo = cor_do_cabelo(reg, player, masc);
    let unhas = if unhas.is_empty() {
        let sistema = crate::jogo::sessao::sistema_pelo_getter_do_jogador(reg, player, "GetCharacterCustomizationSystem");
        cor_da_unha_pela_customizacao(sistema as u64, masc)
    } else {
        unhas
    };
    for (atributo, v) in [("skin_color", pele), ("hair_color", cabelo), ("eyes_color", olhos), ("nails_color", unhas)] {
        nomes.push((atributo, v.clone()));
        e.valores.insert(bwms_primitives::fnv1a64(atributo.as_bytes()), (v, String::new()));
    }
    e.condicoes = nomes.iter().filter(|(_, v)| !v.is_empty()).map(|(a, v)| condicao_do_estado(a, v)).collect();
    Some(e)
}

// ArchiveXL 1.27 `{nails_color}` no estado dinâmico NATIVO da 2.5: o `CollectStateData` do motor (`0x101cc3660`, x0 mapa de estados por entidade, x1
// entidade) grava pele/cabelo/olhos pelo `GetCustomizationData` (`0x101cc4598`, só corpo e cabeça) e refaz as condições no fim (`0x101cc5994`, estado
// `+0x30`). O hook grava `nails_color` no mapa de atributos da entidade antes da original, e as condições já saem com a unha.
pub(crate) const COLETAR_ESTADO_VM: u64 = 0x1_01cc_3660;

pub(crate) const COLETAR_ESTADO_PROLOGO: u32 = 0xd103_c3ff;

/// `Map<Entity*, estado>::operator[]` (pega ou cria; x1 `&entidade`, x2 rascunho): o estado começa pelo mapa de atributos.
const ESTADO_DA_ENTIDADE_VM: u64 = 0x1_01cc_53d4;

/// `Map<CName, DynamicAttributeData>::operator[]` (x1 `&CName`, x2 rascunho): valor = 2 `std::string` da libc++, valor em `+0` e sufixo em `+0x18`.
const ATRIBUTO_DO_ESTADO_VM: u64 = 0x1_01cc_5f30;

/// `ComponentWrapper(Handle<IComponent>&)` (x0 saída de 0x18 B) e o `GetAppearanceName` dele (campo `+0x190`, `+0x230` ou `+0x238` pelo tipo).
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
/// com o nome, como na referência.
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
        // Como o `CName` construído do texto na referência: o nome fica legível no pool do motor.
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

// ArchiveXL 1.27 nome composto de material: `a+b+c` dá `{material.1}`..`{material.N}` no caminho dinâmico do material, além do `{material}`. Na 2.5
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

/// Pares `{material, nome}` seguidos de `{material.N, parte N}`, com cada nome registrado no pool do motor como o `ExtractDynamicName` da referência.
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
/// calcular estado; com `*`, o estado da entidade na hora, sem item equipado (a referência usa o último `UpdateState` da entidade, feito por item de roupa).
pub(crate) unsafe fn resolver_nome_da_entidade(reg: &crate::rtti::Registry, entidade: *mut c_void, nome: &str) -> String {
    if !nome.starts_with('*') {
        return nome.to_string();
    }
    match estado_dinamico_do_player(reg, entidade, 0) {
        Some(estado) => resolver_nome_dinamico(&estado.globais(), &std::collections::HashMap::new(), nome),
        None => nome.to_string(),
    }
}

// ArchiveXL `#11`: `ChangeAppearance`/`ChangeAppearances` do `RuntimeSystemEntityAppearanceChanger` com o `ApplyAppOverride` antes da original
// (`Customization/Extension.cpp:155-195`; tabela em `app_overrides`). Desmonte de 2026-09-14: `ChangeAppearance` `0x101a59e30` (x0 sistema, x1
// `AppearanceChangeRequest*` de 0x38 B {WeakHandle @0, velho @0x10, novo @0x20, flags @0x30}, x2 `std::function&`, w3); `ChangeAppearances` `0x101a5c6e0`
// (x0 sistema, x1 `Handle<Puppet>&`, as 2 `Range<AppearanceDescriptor>` por valor em x2/x3 e x4/x5, início e fim, 16 B por elemento, x6
// `std::function&`, w7 u8 — o `Range&` e o `a5` da referência são o ABI do Windows). Quem chama: o `gameuiAttachmentSlotsListener` do V (slots
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

/// No `on_load`: os 2 hooks, sempre (com a tabela vazia só repassam, como na referência); com `~/.bwms-hook-a11-teste`, as anotações e o `resource.link`.
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

// ===== ArchiveXL `#29` — `PuppetStateHandler` (2026-09-12) =====
// O handler é redscript (`ponte-puppetstate-armsdetect.reds`): o listener nativo de script repassa os eventos de slot por nome. Aqui ficam os 4
// registros de sufixo que a referência cria no `OnTweakDBReady` (`CreateSuffixRecord` + `ActivateSuffixRecords`) e o teste, que chama os 5 passos do
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

/// ArchiveXL `#29`: os 4 registros de sufixo, uma vez por processo, no 1º tick com o TweakDB carregado e o executor livre (no menu, antes do load do
/// save, como o `OnTweakDBReady` da referência). Registro que já existe fica como está, e o id só entra no fim da ordem padrão se faltar.
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
