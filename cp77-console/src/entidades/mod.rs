//! Entidades vivas do mundo: spawn e controle por script (ses/sespec), busca de entidades, objetos de jogo, tags e os sistemas de entidade. Veículos, nós da cena e estado do puppet estão em `veiculos`, `mundo` e `corpo_e_rosto`.

#![allow(dead_code)]

use crate::jogo::sessao::sistema_da_sessao;
use crate::jogo::objetos::destravar_fraco;
use crate::jogo::objetos::travar_fraco;
use crate::jogo::sessao::auth_or;
use crate::jogo::sessao::game_instance;
use crate::jogo::sessao::sistema_pelo_getter;
use crate::rtti::Arg;
use crate::ganchos;
use crate::ganchos::original_publicado;
use crate::aparencia::install_job_do_builder_hook;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU8;


use crate::diagnostico::{C130B_CONGELADO, C130C_CONGELADO};
// Só o handler de prova e os ramos de teste usam, e o público não cita nenhum dos dois.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    a51f_registrar, C130B_ANOTOU, C130B_ATUAL, C130B_DESPACHOS, C130B_SEM_ANOTACAO, C130B_VARIOS_OUVINTES, C130C_ANOTOU, C130C_ATUAL, C130C_DESPACHOS,
    C130C_HOOK_OK, C130C_MONTADAS, C130C_SEM_ANOTACAO, C130C_TESTE, c130e_array_tem, C130E_NO_ARRAY_NOVO, C130E_PROPAGADOS, C130E_SOMADO,
    ENTITYBUILDERWRAPPER_STATES, ORIG_C130_ATTACH, ORIG_C130_DETACH, ORIG_C130_DISPOSE, ORIG_C130_INITIALIZE, ORIG_C130_REASSEMBLE,
    ORIG_C130_REQUESTCOMPONENTS, ORIG_C130_UNINITIALIZE, RECORDS_PRE_RESOLVIDOS, tramp_c220_anotar,
};

use crate::tipos_do_motor::*;

use crate::arquivos_do_jogo::*;










use crate::corpo_e_rosto::*;
use crate::veiculos::*;
use crate::aparencia::*;
use crate::npcs::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b001;






/// Objeto `classe` novo das fábricas estáticas de alvo (`EntityTarget.Type`, `InputTarget.Key`...), retido antes de `registrar` gravar a
/// entrada na tabela do alvo: a tabela guarda o ponteiro por toda a sessão e os filtros a leem de volta (heap corruption de 2026-08-12
/// sem a retenção). Nulo se a classe não instancia; a tabela fica intacta.
pub(crate) unsafe fn alvo_novo(classe: &str, registrar: impl FnOnce(usize)) -> *mut c_void {
    let obj = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |reg| rtti::new_object(&reg, classe));
    if obj.is_null() {
        crate::log_dev!("[alvo] {classe}: new_object falhou");
    } else {
        retain_strong_ref_leaked(obj);
        registrar(obj as usize);
    }
    obj
}

/// `EntityTarget.ID(entityID: EntityID) -> ref<EntityTarget>` — fábrica estática; o `EntityID` chega como os 8 B do hash (o mesmo
/// parâmetro que o `BwmsEntityIDToHash` lê).
pub(crate) unsafe extern "C" fn tramp_et_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let obj = alvo_novo("EntityTarget", |o| {
        if let Ok(mut states) = ENTITYTARGET_STATES.lock() {
            states.push((o, EntityTargetCriterion::Id, id));
        }
    });
    crate::log_dev!("[et] EntityTarget.ID({id:#018x}) -> {obj:p}");
    rtti::make_handle(out, obj);
}

/// `EntityTarget.Type(entityType: CName) -> ref<EntityTarget>` — fábrica estática, 1 arg CName
/// (mesma receita de `tramp_ct_name`). Grava a classe resolvida aqui (`classe_do_alvo`), não o nome.
pub(crate) unsafe extern "C" fn tramp_et_type(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ty = args.first().map(|(v, _)| *v).unwrap_or(0);
    let classe = classe_do_alvo(ty, "EntityTarget.Type");
    let obj = alvo_novo("EntityTarget", |o| {
        if let Ok(mut states) = ENTITYTARGET_STATES.lock() {
            states.push((o, EntityTargetCriterion::Type, classe));
        }
    });
    crate::log_dev!(
        "[et] EntityTarget.Type('{}' hash {ty:#018x}) -> {obj:p} classe {classe:#018x} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))",
        crate::cname::resolve_cname(ty)
    );
    rtti::make_handle(out, obj);
}

/// `EntityTarget.RecordID(recordID: TweakDBID) -> ref<EntityTarget>` — fábrica estática, 1 arg
/// TweakDBID (u64-shaped escalar, MESMA categoria já provada em `CallbackSystemHandler`/testes de
/// clone TweakDB deste projeto — `"TweakDBID"` já está no whitelist `escalar` de `rtti.rs`).
/// Compartilha `ENTITYTARGET_STATES` com `tramp_et_type` (mesmo padrão de `ComponentTarget.ID`/
/// `.Name` compartilhando `COMPONENTTARGET_STATES` — o critério de filtro é oportunístico/
/// permissivo, não filtra fundo hoje).
pub(crate) unsafe extern "C" fn tramp_et_recordid(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let obj = alvo_novo("EntityTarget", |o| {
        if let Ok(mut states) = ENTITYTARGET_STATES.lock() {
            states.push((o, EntityTargetCriterion::RecordId, id));
        }
    });
    crate::log_dev!(
        "[et] EntityTarget.RecordID({id:#018x}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))"
    );
    rtti::make_handle(out, obj);
}

/// `EntityTarget.Template(templatePath: ResRef) -> ref<EntityTarget>` — 2026-07-24, 2ª leva: ResRef
/// agora lê como escalar. Compartilha `ENTITYTARGET_STATES` com `Type`/`RecordID`.
pub(crate) unsafe extern "C" fn tramp_et_template(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let path_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let obj = alvo_novo("EntityTarget", |o| {
        if let Ok(mut states) = ENTITYTARGET_STATES.lock() {
            states.push((o, EntityTargetCriterion::Template, path_hash));
        }
    });
    crate::log_dev!(
        "[et] EntityTarget.Template(hash={path_hash:#018x}) -> {obj:p} (fábrica estática, ResRef lido via escalar novo)"
    );
    rtti::make_handle(out, obj);
}

/// `EntityTarget.Definition(appearancePath: ResRef, opt definitionName: CName) -> ref<EntityTarget>`
/// — 2026-07-24, 2ª leva: ResRef lê como escalar; `definitionName` opcional lido pelo mesmo esquema
/// já confirmado pra `opt` neste projeto (default baked em compile-time pelo `scc`).
pub(crate) unsafe extern "C" fn tramp_et_definition(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let path_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let def_name = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let obj = alvo_novo("EntityTarget", |o| {
        if let Ok(mut states) = ENTITYTARGET_DEFINITION_STATES.lock() {
            states.push((o, path_hash, def_name));
        }
    });
    crate::log_dev!(
        "[et] EntityTarget.Definition(hash={path_hash:#018x} def='{}') -> {obj:p} (fábrica estática, ResRef lido via escalar novo)",
        crate::cname::resolve_cname(def_name)
    );
    rtti::make_handle(out, obj);
}

/// Valor da fixture sintética do wrapper (o smoke `BwmsMakeTestEntityBuilderWrapper`), pro objeto que não é wrapper de builder vivo.
#[cfg(feature = "devtools")]
pub(crate) fn ebw_fixture<T: Default>(c: *mut c_void, f: impl FnOnce(&(usize, u64, u64, u64, bool, bool, bool)) -> T) -> T {
    ENTITYBUILDERWRAPPER_STATES.lock().ok().and_then(|s| s.iter().rev().find(|e| e.0 == c as usize).map(f)).unwrap_or_default()
}

/// Sem `devtools` não existe fixture (só o smoke do dev a cria): o objeto que não é wrapper de builder vivo lê o valor padrão.
#[cfg(not(feature = "devtools"))]
pub(crate) fn ebw_fixture<T: Default>(_c: *mut c_void, _f: impl FnOnce(&(usize, u64, u64, u64, bool, bool, bool)) -> T) -> T {
    T::default()
}

pub(crate) unsafe extern "C" fn tramp_ebw_get_record_id(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_REQUEST), EBR_RECORD)).unwrap_or_else(|| ebw_fixture(c, |e| e.1));
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_ebw_get_entity_id(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_REQUEST), EBR_ENTITY_ID)).unwrap_or_else(|| ebw_fixture(c, |e| e.2));
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_ebw_get_appearance_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_REQUEST), EBR_APARENCIA)).unwrap_or_else(|| ebw_fixture(c, |e| e.3));
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_ebw_has_entity(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(b, EB_EXTRATOR) != 0).unwrap_or_else(|| ebw_fixture(c, |e| e.4));
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ebw_has_appearance(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(b + EB_APARENCIA, EBA_RECURSO) != 0).unwrap_or_else(|| ebw_fixture(c, |e| e.5));
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ebw_has_custom_appearances(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| eb_u64(b + EB_APARENCIAS, 0x08) >> 32 != 0).unwrap_or_else(|| ebw_fixture(c, |e| e.6));
    write_bool_ret(out, v);
}

/// Wrapper novo de `classe` sobre o builder de `pai`, com `aparencia` (0 no do template): handle forte, estado registrado.
pub(crate) unsafe fn ebw_novo(pai: *mut c_void, classe: &str, aparencia: u64) -> [u64; 2] {
    let Some((builder, rc)) = EBW_VIVOS.lock().ok().and_then(|s| s.iter().rev().find(|e| e.0 == pai as usize).map(|e| (e.1, e.2))) else {
        return [0, 0];
    };
    let Some(reg) = rtti::Registry::obtain() else { return [0, 0] };
    let obj = rtti::new_object(&reg, classe);
    if obj.is_null() {
        return [0, 0];
    }
    if let Ok(mut s) = EBW_VIVOS.lock() {
        s.push((obj as usize, builder, rc, aparencia));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    [(buf.as_ptr() as *const u64).read_unaligned(), (buf.as_ptr().add(8) as *const u64).read_unaligned()]
}

/// `GetTemplatePath() -> ResRef`: caminho do `EntityTemplate` do builder. Também é o `GetResourcePath` do
/// `EntityBuilderTemplateWrapper`: o wrapper do template aponta para o mesmo builder, então o caminho é o mesmo.
pub(crate) unsafe extern "C" fn tramp_ebw_get_template_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_TEMPLATE), RES_CAMINHO)).unwrap_or(0), 8);
}

/// `GetEntityType() -> CName`: classe do objeto raiz dos resultados do extrator.
pub(crate) unsafe extern "C" fn tramp_ebw_get_entity_type(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ebw_com_builder(c, |b, _| {
        let (raiz, _) = eb_raiz_e_componentes(b);
        if raiz == 0 { 0 } else { rtti::type_name_hash(rtti::class_of(raiz as *mut c_void)) }
    });
    write_uint_ret(out, v.unwrap_or(0), 8);
}

/// `HasEntityParams() -> Bool` (forma própria: o `entEntityParametersStorage` não é declarado no corpus vanilla).
pub(crate) unsafe extern "C" fn tramp_ebw_has_entity_params(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, ebw_com_builder(c, |b, _| eb_u64(eb_u64(b, EB_REQUEST), EBR_PARAMS) != 0).unwrap_or(false));
}

pub(crate) unsafe extern "C" fn tramp_ebw_get_template(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let vivo = ebw_com_builder(c, |_, _| ()).is_some();
    entregar_handle(out, if vivo { ebw_novo(c, "EntityBuilderTemplateWrapper", 0) } else { [0, 0] });
}

pub(crate) unsafe extern "C" fn tramp_ebw_get_appearance(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let aparencia = ebw_com_builder(c, |b, _| if eb_u64(b + EB_APARENCIA, EBA_RECURSO) != 0 { b + EB_APARENCIA } else { 0 }).unwrap_or(0);
    entregar_handle(out, if aparencia != 0 { ebw_novo(c, "EntityBuilderAppearanceWrapper", aparencia) } else { [0, 0] });
}

/// `GetCustomAppearances() -> array<ref<EntityBuilderAppearanceWrapper>>`: um wrapper por elemento de `appearances` (0x30 B).
pub(crate) unsafe extern "C" fn tramp_ebw_get_custom_appearances(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let aparencias = ebw_com_builder(c, |b, _| {
        let (entradas, tam) = (eb_u64(b + EB_APARENCIAS, 0), (eb_u64(b + EB_APARENCIAS, 0x08) >> 32).min(256));
        (b + EB_APARENCIAS, (0..tam).map(|i| entradas + i * EBA_TAM).collect::<Vec<u64>>())
    });
    let Some((fonte, elementos)) = aparencias.filter(|(_, e)| !e.is_empty()) else {
        eb_array_ret(out, &[], 0, 0);
        return;
    };
    let handles: Vec<[u64; 2]> = elementos.iter().map(|a| ebw_novo(c, "EntityBuilderAppearanceWrapper", *a)).filter(|h| h[0] != 0).collect();
    eb_array_ret(out, &handles, fonte, EBA_TAM);
}

pub(crate) unsafe extern "C" fn tramp_ebt_get_entity(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_handle_ret(out, ebw_com_builder(c, |b, _| eb_raiz_e_componentes(b).0).unwrap_or(0));
}

pub(crate) unsafe extern "C" fn tramp_ebt_get_components(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    match ebw_com_builder(c, |b, _| (eb_raiz_e_componentes(b).1, eb_u64(b, EB_EXTRATOR) + EXT_RESULTADOS)) {
        Some((handles, resultados)) => eb_componentes_ret(out, &handles, resultados),
        None => eb_array_ret(out, &[], 0, 0),
    }
}

pub(crate) unsafe extern "C" fn tramp_ebt_add_component(c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let extrator = ebw_com_builder(c, |b, _| eb_u64(b, EB_EXTRATOR)).unwrap_or(0);
    let ok = eb_adicionar_componente(frame, extrator);
    crate::log_dev!("[ebw] EntityBuilderTemplateWrapper.AddComponent em {c:p}: extrator {extrator:#x} -> {ok}");
}

// `GameObject.AddTag(CName)` (item #27, agente de pesquisa 2026-08-11): `tags.tags.PushBack(tag)`.
// `tags: red::TagList{DynArray<CName> tags@+0x00}` (0x10 bytes, layout CONFIRMADO pela reflection
// real do jogo) — como o DynArray fica no offset 0 da TagList,
// o ponteiro pra TagList JÁ É o ponteiro pro DynArray, sem indireção extra. Composição pura:
// `resolve_prop_in_class` acha o offset REAL da property "tags" na RTTI viva (não hardcoded —
// funciona mesmo se o Mac tiver layout diferente do Windows), `field_ptr` resolve o ponteiro
// final, `dynarray_push_ptr` (já provado em `#52`/`ReflectionEnum.AddConstant` e no grow de
// `register_method`) faz o append com realloc seguro se necessário. `GameObject.HasTag` já é
// NATIVA VANILLA REAL (confirmado em `redscript-src/core/entity/gameObject.script:182`) — se
// "tags" não existir como property RTTI (hipótese não-zero, sem header confirmando), a função
// devolve `false` com segurança (resolve_prop_in_class retorna None, sem tocar memória nenhuma).
pub(crate) unsafe extern "C" fn tramp_gameobject_add_tag(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(obj) && tag_hash != 0 {
        if let Some((voff, _ty, in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(obj), "tags") {
            let tag_list_ptr = rtti::field_ptr(obj, voff, in_holder);
            if !tag_list_ptr.is_null() {
                ok = dynarray_push_ptr(tag_list_ptr, tag_hash).is_some();
            }
        }
    }
    crate::log_dev!("[gameobject] AddTag(obj={obj:p}, tag='{}') ok={ok}", crate::cname::resolve_cname(tag_hash));
    write_bool_ret(out, ok);
}

// `TagList.RemoveTag` (item #32, 2026-08-11, candidato barato) — DIVERGÊNCIA DE ESCOPO
// CONSCIENTE: sem expor `TagList` como STRUCT VALUE-TYPE próprio (`native struct TagList
// { native let tags: array<CName> }`), risco ALTO já documentado (mesma categoria do #30/CMesh —
// tipo sem backing script-facing confirmado). Em vez de declarar esse struct, removo a mesma
// entrada da MESMA property RTTI "tags" que `AddTag`/#27 (já FECHADO) já escreve — 100%
// composição de primitivas já provadas: `resolve_prop_in_class`+`field_ptr` (idênticos ao
// AddTag) acham o `DynArray<CName>` real; um scan linear (CName é só um hash u64, comparação
// trivial) acha o índice; `dynarray_remove_at` (`Erase`/
// `ShiftEntries`, lógica de memmove pura, ZERO endereço nativo) faz a remoção com shift correto.
// Zero campo novo, zero struct novo, zero endereço nativo novo.
pub(crate) unsafe extern "C" fn tramp_gameobject_remove_tag(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let tag_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(obj) && tag_hash != 0 {
        if let Some((voff, _ty, in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(obj), "tags") {
            let tag_list_ptr = rtti::field_ptr(obj, voff, in_holder);
            if !tag_list_ptr.is_null() && crate::memoria::is_readable(tag_list_ptr as *const c_void, 0x10) {
                let entries = (tag_list_ptr as *const u64).read_unaligned() as *const u64;
                let size = (tag_list_ptr.add(12) as *const u32).read_unaligned();
                if !entries.is_null() {
                    for i in 0..size as usize {
                        if crate::memoria::is_readable(entries.add(i) as *const c_void, 8)
                            && entries.add(i).read_unaligned() == tag_hash
                        {
                            ok = rtti::dynarray_remove_at(tag_list_ptr as *mut u8, 8, i);
                            break;
                        }
                    }
                }
            }
        }
    }
    crate::log_dev!("[gameobject] RemoveTag(obj={obj:p}, tag='{}') ok={ok}", crate::cname::resolve_cname(tag_hash));
    write_bool_ret(out, ok);
}

// `TagList.IsEmpty`/parte de leitura (item #32) — mesma resolução de campo do
// AddTag/RemoveTag, só lê `size` (offset +0x0C do header DynArray, já batalha-testado) sem
// mutar nada. `IsEmpty()`/`HasTags()` (redscript) compõem a partir daqui.
pub(crate) unsafe extern "C" fn tramp_gameobject_tag_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut count: u32 = 0;
    if rtti::sane(obj) {
        if let Some((voff, _ty, in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(obj), "tags") {
            let tag_list_ptr = rtti::field_ptr(obj, voff, in_holder);
            if !tag_list_ptr.is_null() && crate::memoria::is_readable(tag_list_ptr as *const c_void, 0x10) {
                count = (tag_list_ptr.add(12) as *const u32).read_unaligned();
            }
        }
    }
    write_uint_ret(out, count as u64, 4);
}

// `TagList.Clear` (item #32) — mesma resolução de campo, só zera `size` (offset +0x0C).
// Não mexe em `entries`/`capacity` (buffer permanece alocado, mesmo idioma "clear sem
// desalocar" já usado por `dynarray_remove_at`/`Assign` — nunca falha por já estar vazio).
pub(crate) unsafe extern "C" fn tramp_gameobject_clear_tags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let obj = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if rtti::sane(obj) {
        if let Some((voff, _ty, in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(obj), "tags") {
            let tag_list_ptr = rtti::field_ptr(obj, voff, in_holder);
            if !tag_list_ptr.is_null() && crate::memoria::is_readable(tag_list_ptr as *const c_void, 0x10) {
                core::ptr::write_unaligned(tag_list_ptr.add(12) as *mut u32, 0u32);
                ok = true;
            }
        }
    }
    crate::log_dev!("[gameobject] ClearTags(obj={obj:p}) ok={ok}");
    write_bool_ret(out, ok);
}

// ===== Construtor de entidade (2026-07-24) — atalho pragmático: NÃO forja `Red::EntityBuilder`/
// `EntityBuilderWrapper`/`EntityBuilderSpec` (3 classes novas em cima de um objeto com
// `SharedSpinLock`/`WeakPtr`/`SharedPtr<ObjectPackageExtractor>` — RE+forge bem mais pesados que
// qualquer classe já forjada no projeto). Em vez disso, GLOBAL nova pro caso comum "adicionar 1
// componente", que ESTENDE a classe `Entity` já existente. Offset
// GROUND-TRUTH, sem RE nova: `ComponentsStorage@Entity+0x70` +
// `components: DynArray<Handle<IComponent>> @+0x30` (`ComponentsStorage` de 0x40 B, reflection
// do jogo) → `Entity + 0x70 + 0x30 = Entity + 0xA0`.
// PushBack via `dynarray_push_handle16` (16B = `{instance,refCount}`, layout `SharedPtrBase<T>`).
//
// Exposta como GLOBAL (`BwmsEntityAddComponent`, via `register_global_composed`/
// `compose_params_from_types` — a receita 100% provada de `BwmsCallMethod`), não como
// `register_method(reg, "Entity", ...)`: `register_method` só tem precedente comprovado em
// classes FORJADAS por nós (`CallbackSystem`/`CallbackSystemHandler`/`EntityLifecycleEvent`/...) —
// nunca numa classe REAL VANILLA já populada pelo motor como `Entity`. A API literal
// `Entity.AddComponent(component)` é reconstruída no `.reds`
// via `@addMethod(Entity)` — mecanismo do PRÓPRIO COMPILADOR redscript, já usado dezenas de vezes
// neste projeto em classes vanilla reais (`PlayerPuppet`/`SettingsSelectorControllerBool`/
// `SingleplayerMenuGameController`/...) — chamando esta global por dentro. Zero risco de
// RTTI-forge em NENHUMA das duas camadas.
//
// Divergência/hipótese fundamentada (documentada, NÃO testada em boot): a suposição de que isto
// basta pro componente "pegar" (sem reimplementar lógica de anexação) vem da assinatura de
// `Raw::Entity::Attach(Entity*, uintptr_t)` — SEM array de componentes separado — sugerindo que
// `Attach` lê `entity->components` direto. O disparo de `Entity/Assemble` (lib.rs, abaixo) roda
// ANTES do `Attach` nativo, então um componente empurrado nesse instante DEVERIA ser pego por
// `Attach` sem lógica extra nossa. Ver investigação completa do construtor de entidade (2026-07-24).
pub(crate) unsafe extern "C" fn tramp_entity_add_component(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let component = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut ok = false;
    if rtti::sane(entity) && rtti::sane(component) {
        let mut buf = [0u8; 16];
        rtti::make_handle(buf.as_mut_ptr() as *mut c_void, component);
        let inst = (buf.as_ptr() as *const u64).read_unaligned();
        let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned();
        let arr = (entity as *mut u8).add(0xA0) as *mut c_void; // Entity+0x70(ComponentsStorage)+0x30(components)
        if crate::memoria::is_readable(arr as *const c_void, 0x10) {
            match dynarray_push_handle16(arr, inst, refc) {
                Some(slot) => {
                    ok = true;
                    crate::log_dev!(
                        "[entity] BwmsEntityAddComponent: entity={entity:p} component={component:p} pushed slot={slot} @entity+0xA0"
                    );
                }
                None => crate::log_dev!("[entity] BwmsEntityAddComponent: push falhou (array ilegível/alloc falhou)"),
            }
        } else {
            crate::log_dev!("[entity] BwmsEntityAddComponent: entity+0xA0 (ComponentsStorage.components) ilegível");
        }
    } else {
        crate::log_dev!(
            "[entity] BwmsEntityAddComponent: entity({entity:p}) ou component({component:p}) inválido"
        );
    }
    write_bool_ret(out, ok);
}

// Construtor de entidade (2026-08-10, cont.187): `BwmsEntityFindComponentByType(entity, type:
// CName) -> ref<IComponent>` — leitura da MESMA `DynArray<Handle<IComponent>>` que
// `BwmsEntityAddComponent` escreve (`Entity+0xA0`, layout já confirmado). Percorre os slots de
// 16 bytes (`{instance,refCount}`), resolve a classe de cada instância (`class_of`) e sobe a
// cadeia de pais (`class_is_kind_of`) atrás da classe do `type` pedido (`Registry::script_class`, uma vez por chamada) — primeiro componente que deriva vence, devolve via `write_handle_ret` (raw handle, sem refcount
// próprio — o componente já é gerenciado pelo `Entity` dono, mesma categoria seguro de
// `BwmsGetController`/etc.). Zero RE nova: mesma DynArray, mesmo `class_of`/`type_name_getname`
// já provados dezenas de vezes.
pub(crate) unsafe extern "C" fn tramp_entity_find_component_by_type(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let type_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut found: u64 = 0;
    // Sem classe, nenhum componente casa.
    let tipo = if rtti::sane(entity) { rtti::Registry::obtain().map_or(std::ptr::null_mut(), |reg| reg.script_class(type_hash)) } else { std::ptr::null_mut() };
    if rtti::sane(tipo) {
        let arr = (entity as *mut u8).add(0xA0) as *const c_void;
        if crate::memoria::is_readable(arr, 0x10) {
            let entries = rd_u64(arr, 0x00) as *const u8;
            let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
            if !entries.is_null() && size < 10_000 {
                for i in 0..size as usize {
                    let slot = entries.add(i * 16);
                    if !crate::memoria::is_readable(slot as *const c_void, 8) {
                        continue;
                    }
                    let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
                    if !rtti::sane(inst) {
                        continue;
                    }
                    let cls = rtti::class_of(inst);
                    if cls.is_null() {
                        continue;
                    }
                    // Herança, como o `IsA` da referência: pedir uma base (`entIPlacedComponent`) acha a filha.
                    if class_is_kind_of(cls, tipo) {
                        found = inst as u64;
                        crate::log_dev!(
                            "[entity] BwmsEntityFindComponentByType: entity={entity:p} type_hash={type_hash:#x} -> achado slot={i} inst={inst:p}"
                        );
                        break;
                    }
                }
            }
        } else {
            crate::log_dev!("[entity] BwmsEntityFindComponentByType: entity+0xA0 ilegível");
        }
    }
    if found == 0 {
        crate::log_dev!(
            "[entity] BwmsEntityFindComponentByType: entity={entity:p} type_hash={type_hash:#x} -> nao achado"
        );
    }
    write_handle_ret(out, found);
}

/// `#24` (`Entity.GetComponents() -> array<ref<IComponent>>`) — 2026-08-10 (cont.190).
/// 3º método do item, e o 2º array-retorno deste projeto (o 1º, `GetRecords`,
/// cont.189, achou+corrigiu o crash de teardown de array-local — trailer de allocator-vft
/// FALTANDO). Aqui o fix é ainda mais direto: a fonte dos dados JÁ É `entity+0xA0`
/// (`DynArray<Handle<IComponent>>` real do motor) — em vez de emprestar o trailer de OUTRO
/// array (como o `GetRecords` precisou), uso o trailer do PRÓPRIO `entity+0xA0` que estou lendo
/// (mesma classe de array, garantidamente populado pelo motor de verdade).
pub(crate) unsafe extern "C" fn tramp_entity_get_components(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut n: usize = 0;
    let mut entries_ptr: *mut c_void = std::ptr::null_mut();
    if rtti::sane(entity) {
        let arr = (entity as *mut u8).add(0xA0) as *const c_void;
        if crate::memoria::is_readable(arr, 0x10) {
            let d_entries = rd_u64(arr, 0x00) as *const u8;
            let d_cap = core::ptr::read_unaligned((arr as *const u8).add(0x08) as *const u32);
            let d_size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
            if !d_entries.is_null() && d_size <= d_cap && d_cap < 10_000 {
                // trailer do PRÓPRIO array (mesmo mecanismo do fix do `GetRecords`, mas sem
                // precisar de doador externo — a fonte já é o array que estou copiando).
                let alloc_vft: u64 = if d_cap > 0 {
                    let tr = (d_entries as u64 + d_cap as u64 * 16 + 7) & !7;
                    if crate::memoria::is_readable(tr as *const c_void, 8) { rd_u64(tr as *const c_void, 0) } else { 0 }
                } else {
                    0
                };
                if d_size > 0 {
                    let total = d_size as usize * 16 + 8;
                    let buf = rtti::pool_alloc(total, 8) as *mut u8;
                    if !buf.is_null() {
                        let mut w = 0usize;
                        for i in 0..d_size as usize {
                            let slot = d_entries.add(i * 16);
                            if !crate::memoria::is_readable(slot as *const c_void, 8) {
                                continue;
                            }
                            let inst = core::ptr::read_unaligned(slot as *const u64);
                            if !rtti::sane(inst as *mut c_void) {
                                continue;
                            }
                            let dst = buf.add(w * 16);
                            core::ptr::write_unaligned(dst as *mut u64, inst);
                            core::ptr::write_unaligned(dst.add(8) as *mut u64, 0u64); // raw, sem refcount nosso
                            w += 1;
                        }
                        core::ptr::write_unaligned(buf.add(w * 16) as *mut u64, alloc_vft);
                        n = w;
                        entries_ptr = buf as *mut c_void;
                    }
                }
            }
        } else {
            crate::log_dev!("[entity] BwmsEntityGetComponents: entity+0xA0 ilegível");
        }
    }
    crate::log_dev!(
        "[entity] BwmsEntityGetComponents: entity={entity:p} -> {n} componentes (entries={entries_ptr:p})"
    );
    core::ptr::write_unaligned(out as *mut u64, entries_ptr as u64);
    core::ptr::write_unaligned((out as *mut u8).add(8) as *mut u32, n as u32);
    core::ptr::write_unaligned((out as *mut u8).add(12) as *mut u32, n as u32);
}

/// `#24` (`Entity.GetTemplatePath() -> ResRef`) — 2026-08-10 (cont.191), 4º/6 métodos.
/// Achado que destravou: o layout REAL completo
/// de `Entity`: `templatePath: ResourcePath @ +0x60`. `ResourcePath`
/// É só um `uint64_t hash` (FNV-1a64, mesma convenção de `resource_path_hash` já usada em todo o
/// projeto) — tamanho 0x8, confirmado. Zero RE nova: leitura direta
/// de campo + o MESMO padrão de retorno já usado por `BwmsHashToResRef` (`write_uint_ret`, raw
/// 8 bytes — `ResRef` em runtime JÁ É só o hash, sem wrapper adicional).
pub(crate) unsafe extern "C" fn tramp_entity_get_template_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut hash: u64 = 0;
    if rtti::sane(entity) {
        let field = (entity as *const u8).add(0x60) as *const c_void;
        if crate::memoria::is_readable(field, 8) {
            hash = rd_u64(field, 0);
        }
    }
    crate::log_dev!("[entity] BwmsEntityGetTemplatePath: entity={entity:p} -> hash={hash:#018x}");
    write_uint_ret(out, hash, 8);
}

/// `#24` (`Entity.SetWorldTransform(WorldTransform)`) — 2026-09-11. A peça nativa é só
/// o ponteiro `transformComponent@0xB0`; a escrita fica com o native VANILLA
/// `IPlacedComponent.SetLocalTransform(pos, rot)` no `.reds` — no componente raiz, local = mundo. O
/// layout do `IPlacedComponent` no Mac bate com o do Windows (registro RTTI `0x101953a68`:
/// `parentTransform@0x90`, `localTransform@0xc0`). Só devolve o objeto se a classe dele descender de
/// `entIPlacedComponent`; o handle sai por `make_handle` (acha o bloco de refcount que já existe).
pub(crate) unsafe extern "C" fn tramp_entity_get_transform_component(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let mut comp: *mut c_void = std::ptr::null_mut();
    if rtti::sane(entity) {
        let field = (entity as *const u8).add(0xB0) as *const c_void;
        if crate::memoria::is_readable(field, 8) {
            let candidato = rd_u64(field, 0) as *mut c_void;
            if rtti::sane(candidato) && classe_deriva_de_nome(rtti::class_of(candidato), cname("entIPlacedComponent")) {
                comp = candidato;
            }
        }
    }
    crate::log_dev!("[entity] BwmsEntityGetTransformComponent: entity={entity:p} -> {comp:p}");
    rtti::make_handle(out, comp);
}












pub(crate) const TRACERESULT_RESULTID_OFF: usize = 0x48;




pub(crate) const NODEINSTANCE_SETUP_OFF: usize = 0x70;

















pub(crate) static STATENT_BY_TAG_CELL: std::sync::OnceLock<StatentMap> = std::sync::OnceLock::new();

/// `IsReady` (#75): o RuntimeSystemEntity e o spawner da sessão resolvem. Consome o frame (native sem parâmetro).
pub(crate) unsafe extern "C" fn tramp_ses_is_ready(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    write_bool_ret(out, statent_sistemas().is_some());
}

pub(crate) unsafe extern "C" fn tramp_ses_is_managed(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let v = statent_gerenciada(id) || statent_tags().lock().map(|m| m.contains_key(&id)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ses_is_tagged(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let v = statent_tags().lock().map(|m| m.get(&id).is_some_and(|s| s.contains(&tag))).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ses_is_populated(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let v = statent_by_tag().lock().map(|m| m.get(&tag).is_some_and(|s| !s.is_empty())).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ses_assign_tag(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let motor = tag_do_motor(gametagsystem_da_sessao(), true, id, tag);
    crate::log_dev!("[statent] AssignTag no GameTagSystem: {motor}");
    if let Ok(mut m) = statent_tags().lock() {
        m.entry(id).or_default().insert(tag);
    }
    if let Ok(mut m) = statent_by_tag().lock() {
        m.entry(tag).or_default().insert(id);
    }
    crate::log_dev!("[statent] AssignTag id={id:#x} tag={tag:#x} ({})", crate::cname::resolve_cname(tag));
}

pub(crate) unsafe extern "C" fn tramp_ses_unassign_tag(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let tag = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let motor = tag_do_motor(gametagsystem_da_sessao(), false, id, tag);
    crate::log_dev!("[statent] UnassignTag no GameTagSystem: {motor}");
    if let Ok(mut m) = statent_tags().lock() {
        if let Some(s) = m.get_mut(&id) {
            s.remove(&tag);
        }
    }
    if let Ok(mut m) = statent_by_tag().lock() {
        if let Some(s) = m.get_mut(&tag) {
            s.remove(&id);
        }
    }
    crate::log_dev!("[statent] UnassignTag id={id:#x} tag={tag:#x}");
}

/// Consultas de tag dos dois sistemas de entidade (dinâmico e estático): o estado de cada um mora no mapa que o trampolim
/// trava, não aqui. O mapa pode ser tag→ids ou id→tags; a função só vê um conjunto por chave.
/// `#[inline(always)]`: com 4 trampolins chamando, o `opt-s` deixaria `conjunto_no_indice` fora de linha (medido no `nm` do público).
/// Menor membro: determinístico, porque o `HashSet` não tem ordem estável.
#[inline(always)]
pub(crate) fn conjunto_menor(m: &std::collections::HashMap<u64, std::collections::HashSet<u64>>, chave: u64) -> u64 {
    m.get(&chave).and_then(|s| s.iter().min().copied()).unwrap_or(0)
}

#[inline(always)]
pub(crate) fn conjunto_tamanho(m: &std::collections::HashMap<u64, std::collections::HashSet<u64>>, chave: u64) -> i32 {
    m.get(&chave).map(|s| s.len() as i32).unwrap_or(0)
}

/// Membro no índice, na ordem crescente: o índice só é estável se o conjunto for ordenado antes.
#[inline(always)]
pub(crate) fn conjunto_no_indice(m: &std::collections::HashMap<u64, std::collections::HashSet<u64>>, chave: u64, idx: i32) -> u64 {
    if idx < 0 {
        return 0;
    }
    match m.get(&chave) {
        Some(s) => {
            let mut v: Vec<u64> = s.iter().copied().collect();
            v.sort_unstable();
            v.get(idx as usize).copied().unwrap_or(0)
        }
        None => 0,
    }
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tagged_id(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let id = statent_by_tag().lock().map(|m| conjunto_menor(&m, tag)).unwrap_or(0);
    write_uint_ret(out, id, 8);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tagged_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let n = statent_by_tag().lock().map(|m| conjunto_tamanho(&m, tag)).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tagged_id_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let id = statent_by_tag().lock().map(|m| conjunto_no_indice(&m, tag, idx)).unwrap_or(0);
    write_uint_ret(out, id, 8);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tag_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let n = statent_tags().lock().map(|m| conjunto_tamanho(&m, id)).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tag_by_index(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    let idx = args.get(1).map(|(v, _)| *v as i32).unwrap_or(-1);
    let tag = statent_tags().lock().map(|m| conjunto_no_indice(&m, id, idx)).unwrap_or(0);
    write_uint_ret(out, tag, 8);
}

// ===== `#75` — StaticEntitySystem pelo EntitySpawner do motor (2026-09-18) =====================================================
// RE em `proofs/20260918-*-75-re-offline.log`. Id transiente do `DynamicEntityIDSystem`: nada vai pro save.
pub(crate) const RSE_VTBL_VM: u64 = 0x1_078d_2a40;

pub(crate) const SPAWNER_SPAWN_VM: u64 = 0x1_019f_2f3c;

pub(crate) const RSE_ATTACH_VM: u64 = 0x1_01a3_d6fc;

pub(crate) const RSE_DETACH_VM: u64 = 0x1_01a3_d704;

pub(crate) const ENTITY_DISPOSE_VM: u64 = 0x1_019a_b7f8;

pub(crate) const REGISTRY_REGISTER_VM: u64 = 0x1_01a2_bb60;

/// Status do `entEntity` (`+0x156`).
pub(crate) const ENT_DETACHED: u8 = 2;

pub(crate) const ENT_ATTACHING: u8 = 3;

pub(crate) const ENT_ATTACHED: u8 = 4;

pub(crate) const ENT_DETACHING: u8 = 5;

pub(crate) static STATENT_CALLBACK: TabelaCallback = TabelaCallback {
    invoke: statent_cb_invoke,
    copiar: statent_cb_copiar,
    mover: statent_cb_mover,
    destruir: dynent_cb_destruir,
};

pub(crate) unsafe extern "C" fn statent_cb_copiar(dst: *mut u8, src: *const u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x18);
}

pub(crate) unsafe extern "C" fn statent_cb_mover(dst: *mut u8, src: *mut u8) {
    std::ptr::copy_nonoverlapping(src, dst, 0x18);
}

/// (RuntimeSystemEntity, RuntimeEntityRegistry, EntitySpawner) da sessão pela tabela do contexto do V: `[[player+0xb8]+0x8]` em `+0x50` e
/// `+0x68`, com o spawner em `rse+0x58`; conferidos pela vtable do RSE e pelo `spawner+0x60 == rse`.
pub(crate) unsafe fn statent_sistemas() -> Option<(usize, usize, usize)> {
    let player = crate::current_player() as usize;
    let ler = |a: usize| -> usize {
        if a == 0 || !crate::memoria::is_readable(a as *const c_void, 8) {
            0
        } else {
            (a as *const u64).read_unaligned() as usize
        }
    };
    let tabela = ler(ler(player.checked_add(0xb8)?).checked_add(0x8)?);
    let (rse, registry) = (ler(tabela.checked_add(0x50)?), ler(tabela.checked_add(0x68)?));
    if rse == 0 || registry == 0 || ler(rse) != crate::rebase(RSE_VTBL_VM) as usize {
        return None;
    }
    let spawner = ler(rse + 0x58);
    (spawner != 0 && ler(spawner + 0x60) == rse).then_some((rse, registry, spawner))
}

pub(crate) unsafe fn statent_eh_gameobject(ent: *mut c_void) -> bool {
    classe_deriva_de_nome(rtti::class_of(ent), crate::cname::cname("gameObject"))
}

/// Invoke do callback do spawn, com o `SharedPtr` do token da entrada do spawner.
pub(crate) unsafe extern "C" fn statent_cb_invoke(buffer: *mut u8, token_sp: *mut [u64; 2]) {
    if buffer.is_null() || token_sp.is_null() {
        return;
    }
    let token = (*token_sp)[0];
    if token == 0 {
        return;
    }
    let id = (buffer as *const u64).read_unaligned();
    let Ok(mut estados) = STATENT_ESTADOS.lock() else { return };
    #[cfg(feature = "devtools")]
    statent_trilhar(id, "callback", token as i64);
    let Some(e) = estados.get_mut(&id) else { return };
    let mut guardado = std::mem::take(&mut e.token);
    let t = token as *mut u8;
    let ent = (t.add(0x10) as *const [u64; 2]).read_unaligned();
    let (falhou, abortou) = (t.add(0x21).read() != 0, t.add(0x22).read() != 0);
    if ent[0] != 0 && !falhou && !abortou {
        // O handle sai do token: o destrutor dele não descarta mais a entidade, que passa a ser do estado.
        (t.add(0x10) as *mut [u64; 2]).write_unaligned([0, 0]);
        e.ent = ent;
        if !statent_eh_gameobject(ent[0] as *mut c_void) {
            let registrar: unsafe extern "C" fn(usize, u64) -> u64 = std::mem::transmute(crate::rebase(REGISTRY_REGISTER_VM));
            e.registrada = registrar(e.registry, ent[0]) & 0xff != 0;
        }
    } else {
        estados.remove(&id);
        drop(estados);
        statent_esquecer_tags(id);
    }
    dynent_soltar_token(std::mem::take(&mut guardado));
}

/// `EntitySpawnerRequest` do spec, com o callback do id.
fn statent_pedido(id: u64, spec: &StaticEntitySpecFields) -> PedidoSpawner {
    let mut p = PedidoSpawner([0; 0xF0]);
    p.put(0x00, spec.template_path);
    for k in 0..3 {
        p.put(0x10 + 4 * k, (spec.position[k] * FIXEDPOINT_ESCALA) as i32);
    }
    let orientacao = if spec.orientation.iter().all(|v| *v == 0.0) { [0.0, 0.0, 0.0, 1.0] } else { spec.orientation };
    p.put(0x20, orientacao);
    p.put(0x30, id);
    p.put(0x38, spec.appearance_name);
    p.put(0x48, id);
    p.put(0x60, &STATENT_CALLBACK as *const TabelaCallback as u64);
    p.put(0x78, !spec.attached as u8);
    p.put(0xE1, 1u8);
    p
}

/// `SpawnEntity`; devolve o id ou 0.
pub(crate) unsafe fn statent_spawnar(spec: &StaticEntitySpecFields) -> u64 {
    if spec.template_path == 0 {
        return 0;
    }
    let ids = sistema_da_sessao("gameDynamicEntityIDSystem", Some(IDSYS_VTBL_VM));
    let Some((rse, registry, spawner)) = statent_sistemas().filter(|_| !ids.is_null()) else {
        crate::log_dev!("[statent] SpawnEntity sem sistemas do motor");
        return 0;
    };
    let proximo: unsafe extern "C" fn(*mut c_void) -> u64 = std::mem::transmute(crate::rebase(ID_NEXT_TRANSIENT_VM));
    let id = proximo(ids) & 0xffff_ffff;
    if id == 0 {
        return 0;
    }
    let pedido = statent_pedido(id, spec);
    // O estado entra antes do pedido: o callback pode vir de outra thread logo depois.
    match STATENT_ESTADOS.lock() {
        Ok(mut m) => {
            m.insert(id, EntidadeEstatica { rse, registry, token: [0; 2], ent: [0; 2], registrada: false, descartada: false });
        }
        Err(_) => return 0,
    }
    let mut sp = [0u64; 2];
    crate::jogo::chamada::sret_chamar(crate::rebase(SPAWNER_SPAWN_VM), [spawner as u64, &pedido as *const PedidoSpawner as u64, 0, 0], sp.as_mut_ptr() as *mut c_void);
    #[cfg(feature = "devtools")]
    statent_trilhar(id, "pedido", sp[0] as i64);
    if sp[0] == 0 {
        if let Ok(mut m) = STATENT_ESTADOS.lock() {
            m.remove(&id);
        }
        return 0;
    }
    let (ficou, gerenciada) = match STATENT_ESTADOS.lock() {
        Ok(mut m) => match m.get_mut(&id) {
            Some(e) if e.ent[0] == 0 => {
                e.token = sp;
                (true, true)
            }
            Some(_) => (false, true),
            None => (false, false),
        },
        Err(_) => (false, false),
    };
    if !ficou {
        dynent_soltar_token(sp);
    }
    if !gerenciada {
        return id;
    }
    if let (Ok(mut porid), Ok(mut portag)) = (statent_tags().lock(), statent_by_tag().lock()) {
        for &tag in &spec.tags {
            porid.entry(id).or_default().insert(tag);
            portag.entry(tag).or_default().insert(id);
        }
    }
    id
}

/// `DespawnEntity`.
pub(crate) unsafe fn statent_despawnar(id: u64) -> bool {
    let Ok(mut estados) = STATENT_ESTADOS.lock() else { return false };
    let Some(e) = estados.get_mut(&id).filter(|e| !e.descartada) else { return false };
    let token = std::mem::take(&mut e.token);
    let ent = e.ent[0];
    if ent != 0 {
        // O estado fica até o status 7: o `statent_tick` solta o handle e tira do registry.
        e.descartada = true;
    } else {
        estados.remove(&id);
    }
    drop(estados);
    if ent != 0 {
        let descartar: unsafe extern "C" fn(u64) = std::mem::transmute(crate::rebase(ENTITY_DISPOSE_VM));
        descartar(ent);
    } else {
        statent_esquecer_tags(id);
    }
    if token[0] != 0 {
        dynent_soltar_token(token);
    }
    token[0] != 0 || ent != 0
}

/// `(RuntimeSystemEntity, handle da entidade, status)` da entidade gerenciada no mundo.
pub(crate) fn statent_status(id: u64) -> Option<(usize, [u64; 2], u8)> {
    let estados = STATENT_ESTADOS.lock().ok()?;
    let e = estados.get(&id).filter(|e| e.ent[0] != 0 && !e.descartada)?;
    let status = (e.ent[0] as usize).checked_add(ENT_STATUS_OFF)? as *const u8;
    // SAFETY: o handle do estado segura a entidade; o `is_readable` cobre um estado corrompido.
    unsafe { crate::memoria::is_readable(status as *const c_void, 1).then(|| (e.rse, e.ent, status.read())) }
}

/// `AttachEntity`/`DetachEntity`.
pub(crate) unsafe fn statent_ligar(id: u64, ligar: bool) -> bool {
    let Some((rse, ent, status)) = statent_status(id) else { return false };
    if ligar {
        if status == ENT_ATTACHING || status == ENT_ATTACHED {
            return false;
        }
        // O motor copia o handle do pedido com +1: o do estado só é emprestado.
        let mut pedido = [0u64; 8];
        pedido[0] = ent[0];
        pedido[1] = ent[1];
        let anexar: unsafe extern "C" fn(usize, *const u64) = std::mem::transmute(crate::rebase(RSE_ATTACH_VM));
        anexar(rse, pedido.as_ptr());
    } else {
        if status == ENT_DETACHED || status == ENT_DETACHING {
            return false;
        }
        let soltar: unsafe extern "C" fn(usize, *const [u64; 2]) = std::mem::transmute(crate::rebase(RSE_DETACH_VM));
        soltar(rse, &ent);
    }
    #[cfg(feature = "devtools")]
    statent_trilhar(id, if ligar { "attach" } else { "detach" }, status as i64);
    true
}

pub(crate) fn statent_ids_com_tag(tag: u64) -> Vec<u64> {
    statent_by_tag().lock().ok().and_then(|m| m.get(&tag).map(|s| s.iter().copied().collect())).unwrap_or_default()
}

pub(crate) fn statent_gerenciada(id: u64) -> bool {
    STATENT_ESTADOS.lock().map(|m| m.contains_key(&id)).unwrap_or(false)
}

/// Handle novo (+1) da entidade no mundo, ou vazio.
pub(crate) unsafe fn statent_handle(id: u64) -> [u64; 2] {
    match statent_status(id) {
        Some((_, ent, _)) => {
            let mut h = [0u64; 2];
            rtti::make_handle(h.as_mut_ptr() as *mut c_void, ent[0] as *mut c_void);
            h
        }
        None => [0; 2],
    }
}

/// `SpawnEntity(spec: ref<StaticEntitySpec>) -> EntityID`
pub(crate) unsafe extern "C" fn tramp_ses_spawn_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let spec_obj = dynent_id_do_frame(frame) as usize;
    let spec = ses_spec_map().lock().ok().and_then(|m| m.get(&spec_obj).cloned());
    let id = spec.map_or(0, |s| statent_spawnar(&s));
    crate::log_dev!("[statent] SpawnEntity(spec={spec_obj:#x}) -> {id}");
    write_uint_ret(out, id, 8);
}

pub(crate) unsafe extern "C" fn tramp_ses_despawn_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, statent_despawnar(id));
}

pub(crate) unsafe extern "C" fn tramp_ses_attach_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, statent_ligar(id, true));
}

pub(crate) unsafe extern "C" fn tramp_ses_detach_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, statent_ligar(id, false));
}

/// `IsSpawned`: a entidade saiu do token e não foi descartada.
pub(crate) unsafe extern "C" fn tramp_ses_is_spawned(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    write_bool_ret(out, statent_status(id).is_some());
}

/// `IsSpawning`: o token ainda é do BWMS (o callback não veio).
pub(crate) unsafe extern "C" fn tramp_ses_is_spawning(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let v = STATENT_ESTADOS.lock().map(|m| m.get(&id).is_some_and(|e| e.token[0] != 0)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_entity(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    if out.is_null() {
        return;
    }
    entregar_handle(out, statent_handle(id));
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let id = dynent_id_do_frame(frame);
    let tags: Vec<u64> = statent_tags().lock().ok().and_then(|m| m.get(&id).map(|s| s.iter().copied().collect())).unwrap_or_default();
    array_u64_do_motor_ret(out, &tags);
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tagged_ids(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let tag = dynent_id_do_frame(frame);
    array_u64_do_motor_ret(out, &statent_ids_com_tag(tag));
}

pub(crate) unsafe extern "C" fn tramp_ses_get_tagged(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let tag = dynent_id_do_frame(frame);
    if out.is_null() {
        return;
    }
    let handles: Vec<[u64; 2]> = statent_ids_com_tag(tag).into_iter().map(|id| statent_handle(id)).filter(|h| h[0] != 0).collect();
    array_do_motor_ret(out, handles.len(), 16, |buf| {
        for (i, h) in handles.iter().enumerate() {
            core::ptr::write_unaligned(buf.add(i * 16) as *mut [u64; 2], *h);
        }
    });
}

pub(crate) unsafe extern "C" fn tramp_ses_despawn_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    for id in statent_ids_com_tag(dynent_id_do_frame(frame)) {
        statent_despawnar(id);
    }
}

pub(crate) unsafe extern "C" fn tramp_ses_attach_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    for id in statent_ids_com_tag(dynent_id_do_frame(frame)) {
        statent_ligar(id, true);
    }
}

pub(crate) unsafe extern "C" fn tramp_ses_detach_tagged(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    for id in statent_ids_com_tag(dynent_id_do_frame(frame)) {
        statent_ligar(id, false);
    }
}

// ===== `#76` (`StaticEntitySpec`) — implementação NOVA, 2026-08-15 (rodada 9) ==============
//
// Companion struct do `#75`/`StaticEntitySystem` (acima, FECHADO nesta mesma sessão) — catalogado
// como "companion struct do #75, nunca tocado, 0%" (confirmado por grep triplo antes de codar: zero
// ocorrência de "StaticEntitySpec" em `register.rs`/`cp77-console/src/`/`blackwall-mods-dev/*.reds`/
// `bwms-core/`). Struct de dado PURO (`StaticEntitySpec extends
// IScriptable`), ZERO método/native func, só 6 propriedades (não persistentes — ao contrário
// do irmão `DynamicEntitySpec`/`#74`, este NÃO é salvo no save): `templatePath: RaRef<>`,
// `appearanceName: CName`, `position: Vector4`, `orientation: Quaternion`, `tags: DynArray<CName>`,
// `attached: bool`.
//
// Receita reusada: EXATAMENTE a de `CallbackSystemHandler` (`register_callbacksystemhandler`, acima)
// — `extends IScriptable` DIRETO (mesmo parent), INSTANCIÁVEL (`register_type_instantiable_with_
// parent`), estado por-instância guardado num registry Rust keyed por ponteiro `this` (`c as usize`),
// mesmo idioma de `HANDLER_STATES`/`tramp_csh_set_run_mode`/`tramp_csh_add_target`. Zero endereço
// nativo, zero campo cru lido de memória do motor — é 100% dado NOSSO, construído do zero (o consumidor é o `SpawnEntity` do `#75`, que passa estes campos ao
// `EntitySpawner` do motor em `statent_spawnar`).
//
// `templatePath` tipado como `ResRef` (não `RaRef<T>`/`ResourceAsyncReference`, tipo nunca
// declarado neste projeto) — mesma divergência já aceita pro item `#63`/Casts.reds (`ResRef`/
// `redResourceReferenceScriptToken` já provado nos 2 sentidos, parâmetro E retorno, desde
// 2026-08-10 via `BwmsResRefToHash`/`BwmsHashToResRef`).
//
// `position`(Vector4)/`orientation`(Quaternion) — 2026-08-15, rodada 10: a rodada anterior deixou
// isso de fora ("struct de 16 bytes como PARÂMETRO nunca exercitado"). Investigação desta rodada
// (leitura completa de `rtti::read_params_inner_ext3`) achou que a máquina genérica já usada pra
// LER `CString`(0x20B, `read_cstring`)/`Variant`(24B) COMO PARÂMETRO já é 100% type-agnostic:
// `type_inst_alloc(ptype)` aloca `pinst` pelo TAMANHO REAL do tipo (via vtable `GetSize`,
// CONFIRMADO 16 pra Vector4 desde 2026-06-20) e o MESMO dispatch de opcode (`h(ctx,frame,pinst,
// scriptref_out)`) que decodifica QUALQUER outro param decodifica o valor ali dentro — é o idioma
// genérico que a VM usa pra QUALQUER native (nossa ou vanilla) receber `new Vector4(...)` como
// argumento. Só faltava capturar os 16 bytes de `pinst` no final (antes só 8 bytes eram lidos,
// pro caso escalar) — feito em `rtti::read_params_consuming_with_raw16` (novo). Vector4/Quaternion
// são POD puro (4 floats, zero ownership/allocator) — mais simples que o caso já provado de
// CString (SSO+heap+allocator). O lado RETORNO (`GetPosition()->Vector4`) usa `write_raw16_ret`
// (novo, register.rs), memcpy direto — `out` já vem dimensionado pelo tipo de retorno DECLARADO
// no `.reds` (mesma garantia já documentada em `write_cstring_inline_ret`, que escreve 0x20B no
// mesmo slot). Testado ISOLADO primeiro via `BwmsVector4RoundTrip` (native global separada, zero
// dependência desta classe) antes de aplicar aqui — ver `register_all`.
#[derive(Clone, Default)]
pub(crate) struct StaticEntitySpecFields {
    pub(crate) appearance_name: u64,
    pub(crate) template_path: u64,
    pub(crate) attached: bool,
    pub(crate) tags: Vec<u64>,
    pub(crate) position: [f32; 4],
    pub(crate) orientation: [f32; 4],
}

pub(crate) type SesSpecMap = std::sync::Mutex<std::collections::HashMap<usize, StaticEntitySpecFields>>;

pub(crate) static SES_SPEC_CELL: std::sync::OnceLock<SesSpecMap> = std::sync::OnceLock::new();

pub(crate) fn ses_spec_map() -> &'static SesSpecMap {
    SES_SPEC_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// `StaticEntitySpec.Create() -> ref<StaticEntitySpec>` — fábrica estática, mesma receita de
/// `ComponentTarget.ID`/`tramp_ct_id`: constrói instância NOVA (`rtti::new_object`) + retenção forte
/// (`retain_strong_ref_leaked`, aplicada ANTES de qualquer setter poder ler o registry de volta —
/// mesma disciplina "desde a origem" já estabelecida pro `#132`/`#133`/`#136`) + entry vazia no
/// registry keyed por ponteiro.
pub(crate) unsafe extern "C" fn tramp_sespec_create(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let obj = alvo_novo("StaticEntitySpec", |o| {
        if let Ok(mut m) = ses_spec_map().lock() {
            m.insert(o, StaticEntitySpecFields::default());
        }
    });
    crate::log_dev!("[sespec] StaticEntitySpec.Create() -> {obj:p}");
    rtti::make_handle(out, obj);
}

pub(crate) unsafe extern "C" fn tramp_sespec_set_appearance_name(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let name = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = ses_spec_map().lock() {
        m.entry(c as usize).or_default().appearance_name = name;
    }
    crate::log_dev!("[sespec] SetAppearanceName({name:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_sespec_get_appearance_name(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.appearance_name)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

pub(crate) unsafe extern "C" fn tramp_sespec_set_template_path(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let path = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = ses_spec_map().lock() {
        m.entry(c as usize).or_default().template_path = path;
    }
    crate::log_dev!("[sespec] SetTemplatePath({path:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_sespec_get_template_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.template_path)).unwrap_or(0);
    write_uint_ret(out, v, 8);
}

/// **BUG REAL achado+corrigido (2026-08-15, rodada 11)** — a anomalia "`SetAttached(false)` seguido
/// de `GetAttached()` sempre lê `true`" (documentada nas rodadas 9/10 como pré-existente/não-
/// relacionada) tinha causa raiz aqui: `rtti::read_params_inner_ext3::type_inst_alloc` só ZERA
/// `size` bytes da instância tipada (`GetSize()` de `Bool` = **1 byte**) — mas o `read8` genérico (linha
/// "`let raw = if read8 && ganchos::is_readable(pinst,8) { (pinst as *const u64).read_unaligned() }`")
/// sempre lê **8 bytes** de `pinst`, INDEPENDENTE do tamanho real do tipo. Pra `Bool`/`Int32`/etc.
/// (tipos <8 bytes), os bytes ALÉM do tamanho real nunca são zerados — ficam com o conteúdo STALE
/// da alocação anterior no mesmo bloco do pool (reusado, ex. um `CName`/8B ou `Vector4`/16B da
/// mesma sequência de chamadas). `SetAttached(true)`: byte0=1, `*v != 0` sempre dá `true` (lixo nos
/// bytes 1-7 é irrelevante). `SetAttached(false)`: byte0=0 (opcode `FalseConst` real do motor grava
/// certo), mas SE os bytes 1-7 (lixo determinístico, mesmo padrão de reuso do pool) tiverem QUALQUER
/// bit setado, `*v != 0` (comparação nos 8 bytes INTEIROS) dá `true` por engano — exatamente o
/// sintoma observado (sempre `true`, nunca flaky, pois o lixo é determinístico). Tipos como
/// `Int32`/`Float` escapam desse bug em outros call-sites porque quase sempre são lidos com
/// `as i32`/`as u32` (trunca pros 4 bytes baixos, ignora o lixo alto) — `Bool` aqui comparava o
/// qword INTEIRO sem máscara. **Fix**: mascarar pro byte baixo antes de comparar (`rtti::bool_do_script`),
/// MESMO padrão já usado (e correto) em `tramp_refl_prop_set_value_bool` (`Reflection.PropSetValueBool`,
/// linha ~8905: `rtti::bool_do_script`) — só que aqui nunca tinha sido aplicado.
pub(crate) unsafe extern "C" fn tramp_sespec_set_attached(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let v = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    if let Ok(mut m) = ses_spec_map().lock() {
        m.entry(c as usize).or_default().attached = v;
    }
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_sespec_get_attached(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.attached)).unwrap_or(false);
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_sespec_add_tag(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    if let Ok(mut m) = ses_spec_map().lock() {
        m.entry(c as usize).or_default().tags.push(tag);
    }
    crate::log_dev!("[sespec] AddTag({tag:#018x}) on {c:p}");
    write_bool_ret(out, true);
}

pub(crate) unsafe extern "C" fn tramp_sespec_get_tag_count(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let n = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.tags.len())).unwrap_or(0);
    write_uint_ret(out, n as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_sespec_get_tag_by_index(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let idx = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let v = if idx < 0 {
        0
    } else {
        ses_spec_map()
            .lock()
            .ok()
            .and_then(|m| m.get(&(c as usize)).and_then(|f| f.tags.get(idx as usize).copied()))
            .unwrap_or(0)
    };
    write_uint_ret(out, v, 8);
}

/// `SetPosition(v: Vector4) -> Bool` — 1ª native deste projeto a receber uma struct de 16 bytes
/// COMO PARÂMETRO (ver o comentário completo em `StaticEntitySpecFields` acima). Composição pura:
/// `read_params_consuming_with_raw16` já entrega os 16 bytes crus de `pinst` — só reinterpreta
/// como `[f32;4]` (mesmo layout de `Red::Vector4{x,y,z,w}`, todos IEEE754 LE) e guarda no registry.
pub(crate) unsafe extern "C" fn tramp_sespec_set_position(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, raw16) = rtti::read_params_consuming_with_raw16(func, frame);
    let bytes = raw16.first().and_then(|r| *r);
    let ok = bytes.is_some();
    if let Some(b) = bytes {
        let mut v = [0f32; 4];
        for i in 0..4 {
            v[i] = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        if let Ok(mut m) = ses_spec_map().lock() {
            m.entry(c as usize).or_default().position = v;
        }
        crate::log_dev!("[sespec] SetPosition({v:?}) on {c:p}");
    } else {
        crate::log_dev!("[sespec] SetPosition: raw16 vazio (tipo do param não bateu Vector4?) on {c:p}");
    }
    write_bool_ret(out, ok);
}

/// `GetPosition() -> Vector4` — 1ª native deste projeto a DEVOLVER uma struct de 16 bytes própria
/// (não composição de natives vanilla, ex. `#29`). `write_raw16_ret` faz memcpy direto dos 4
/// floats guardados — POD puro, zero risco de ownership/allocator.
pub(crate) unsafe extern "C" fn tramp_sespec_get_position(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.position)).unwrap_or([0f32; 4]);
    let mut b = [0u8; 16];
    for i in 0..4 {
        b[i * 4..i * 4 + 4].copy_from_slice(&v[i].to_le_bytes());
    }
    write_raw16_ret(out, &b);
}

/// `SetOrientation(q: Quaternion) -> Bool` — mesma técnica de `SetPosition`, tipo `Quaternion`
/// (também 16B/4 floats, `Red::Quaternion{i,j,k,r}` — mesmo layout cru de Vector4 pro propósito
/// de armazenar/devolver os 4 floats; a SEMÂNTICA dos componentes é do chamador, não nossa).
pub(crate) unsafe extern "C" fn tramp_sespec_set_orientation(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, raw16) = rtti::read_params_consuming_with_raw16(func, frame);
    let bytes = raw16.first().and_then(|r| *r);
    let ok = bytes.is_some();
    if let Some(b) = bytes {
        let mut v = [0f32; 4];
        for i in 0..4 {
            v[i] = f32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap_or([0; 4]));
        }
        if let Ok(mut m) = ses_spec_map().lock() {
            m.entry(c as usize).or_default().orientation = v;
        }
        crate::log_dev!("[sespec] SetOrientation({v:?}) on {c:p}");
    } else {
        crate::log_dev!("[sespec] SetOrientation: raw16 vazio (tipo do param não bateu Quaternion?) on {c:p}");
    }
    write_bool_ret(out, ok);
}

/// `GetOrientation() -> Quaternion` — espelho de `GetPosition`.
pub(crate) unsafe extern "C" fn tramp_sespec_get_orientation(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = ses_spec_map().lock().ok().and_then(|m| m.get(&(c as usize)).map(|f| f.orientation)).unwrap_or([0f32; 4]);
    let mut b = [0u8; 16];
    for i in 0..4 {
        b[i * 4..i * 4 + 4].copy_from_slice(&v[i].to_le_bytes());
    }
    write_raw16_ret(out, &b);
}


pub(crate) static DYNENT_INSTANCE: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// `BwmsGetDynamicEntitySystem()` — GLOBAL, não `@addMethod(GameInstance)` (mesmo motivo pragmático
/// de `BwmsGetCallbackSystem`: `GameInstance` valida MUITO cedo
/// no bundle, `@addMethod` direto nela já crashou o boot em 2026-07-13). Instância lazy, cacheada.
pub(crate) unsafe extern "C" fn tramp_get_dynamicentitysystem(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // Consome o `ParamEnd`: a native é usada inline (`BwmsGetDynamicEntitySystem().X()`).
    rtti::read_params_consuming(current_native_func(), frame);
    let mut ptr = DYNENT_INSTANCE.load(Ordering::Acquire);
    if ptr.is_null() {
        if let Some(reg) = rtti::Registry::obtain() {
            let obj = rtti::new_object(&reg, "DynamicEntitySystem");
            if !obj.is_null() {
                DYNENT_INSTANCE.store(obj, Ordering::Release);
                ptr = obj;
                crate::log_dev!("[dynent] BwmsGetDynamicEntitySystem: instância construída {obj:p} (lazy, 1a chamada)");
            } else {
                crate::log_dev!("[dynent] BwmsGetDynamicEntitySystem: new_object('DynamicEntitySystem') falhou");
            }
        }
    }
    write_handle_ret(out, ptr as u64);
}


pub(crate) unsafe fn tagsystem_tramp(frame: *mut c_void, out: *mut c_void, marcar: bool) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map_or(std::ptr::null_mut(), |(v, _)| *v as *mut c_void);
    let id = args.get(1).map_or(0, |(v, _)| *v);
    let tag = args.get(2).map_or(0, |(v, _)| *v);
    write_bool_ret(out, tag_do_motor(sys, marcar, id, tag));
}

/// `BwmsTagSystemAssignTag(sys: ref<IScriptable>, entityID: EntityID, tag: CName) -> Bool`
pub(crate) unsafe extern "C" fn tramp_tagsystem_assign(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    tagsystem_tramp(frame, out, true);
}

/// `BwmsTagSystemUnassignTag(sys: ref<IScriptable>, entityID: EntityID, tag: CName) -> Bool`
pub(crate) unsafe extern "C" fn tramp_tagsystem_unassign(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    tagsystem_tramp(frame, out, false);
}

pub(crate) static STATENT_INSTANCE: std::sync::atomic::AtomicPtr<c_void> = std::sync::atomic::AtomicPtr::new(std::ptr::null_mut());

/// `BwmsGetStaticEntitySystem()` — GLOBAL, não `@addMethod(GameInstance)` (mesmo motivo pragmático já
/// documentado pro `#73`/`BwmsGetDynamicEntitySystem`: `GameInstance` valida MUITO cedo no bundle).
pub(crate) unsafe extern "C" fn tramp_get_staticentitysystem(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    let mut ptr = STATENT_INSTANCE.load(Ordering::Acquire);
    if ptr.is_null() {
        if let Some(reg) = rtti::Registry::obtain() {
            let obj = rtti::new_object(&reg, "StaticEntitySystem");
            if !obj.is_null() {
                STATENT_INSTANCE.store(obj, Ordering::Release);
                ptr = obj;
                crate::log_dev!("[statent] BwmsGetStaticEntitySystem: instância construída {obj:p} (lazy, 1a chamada)");
            } else {
                crate::log_dev!("[statent] BwmsGetStaticEntitySystem: new_object('StaticEntitySystem') falhou");
            }
        }
    }
    write_handle_ret(out, ptr as u64);
}

/// Entidade de um evento pela tabela de estado do objeto: a do ciclo de vida, a dona do componente (`EntityComponentEvent`) ou o veículo
/// (`VehicleLightControlEvent`) — os 2 últimos herdam o `EntityLifecycleEvent`. 0 sem entidade.
pub(crate) fn entidade_do_evento(evento: usize) -> usize {
    ENTITYLIFECYCLE_STATES
        .lock()
        .ok()
        .and_then(|s| s.iter().rev().find(|e| e.0 == evento).map(|e| e.1))
        .or_else(|| ENTITYCOMPONENT_STATES.lock().ok().and_then(|s| s.iter().rev().find(|e| e.0 == evento).map(|e| e.2)))
        .or_else(|| VEHICLELIGHT_STATES.lock().ok().and_then(|s| s.iter().rev().find(|e| e.0 == evento).map(|e| e.3)))
        .unwrap_or(0)
}

/// Categoria da entidade e o `recordID` pelo caminho da categoria (puppet `+0x25C` no Mac, veículo `+0x658`, device pelo estado persistente).
pub(crate) unsafe fn categoria_e_record(entidade: *mut c_void) -> (bwms_primitives::targets::CategoriaEntidade, Option<u64>) {
    use bwms_primitives::targets::CategoriaEntidade;
    let Some(reg) = rtti::Registry::obtain() else { return (CategoriaEntidade::Outra, None) };
    let cls = rtti::class_of(entidade);
    let herda = |nomes: &[&str]| {
        nomes.iter().any(|n| {
            let c = reg.class_by_name(n);
            rtti::sane(c) && class_is_kind_of(cls, c)
        })
    };
    if herda(&["gamePuppetBase"]) {
        (CategoriaEntidade::Puppet, read_u64_field(entidade, 0x25c))
    } else if herda(&["vehicleBaseObject", "VehicleObject"]) {
        (CategoriaEntidade::Vehicle, read_u64_field(entidade, 0x658))
    } else if herda(&["gameDeviceBase"]) {
        (CategoriaEntidade::Device, record_do_device(entidade))
    } else {
        (CategoriaEntidade::Outra, None)
    }
}

/// Eventos do ciclo de vida da entidade que os hooks despacham, na ordem dos bits de [`ENTIDADE_EVENTOS_ARMADOS`].
pub const ENTIDADE_EVENTOS: [&str; 6] = ["Entity/Initialize", "Entity/Reassemble", "Entity/Attach", "Entity/AfterAttach", "Entity/Detach", "Entity/Uninitialize"];

/// Bit `k` ligado = alguém ouve `ENTIDADE_EVENTOS[k]`. Os hooks rodam milhares de vezes por minuto e quase sempre fora da thread do jogo, então só leem
/// este atômico; quem trava `CALLBACKS` pra montá-lo são o registro e a remoção de ouvinte, em [`atualizar_eventos_de_entidade`].
pub static ENTIDADE_EVENTOS_ARMADOS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Alguém ouve `ENTIDADE_EVENTOS[k]`.
pub(crate) fn evento_de_entidade_armado(k: usize) -> bool {
    ENTIDADE_EVENTOS_ARMADOS.load(Ordering::Relaxed) & (1 << k) != 0
}

/// `StaticEntityTarget.Tags(tags: array<CName>) -> ref<StaticEntityTarget>` — 2026-09-14: a mesma fábrica plural do `DynamicEntityTarget`.
pub(crate) unsafe extern "C" fn tramp_stt_tags(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    alvo_por_tags(frame, out, "StaticEntityTarget", &STATICENTITYTARGET_STATES);
}

/// Fábrica plural dos alvos por tag: um objeto `classe` retido, com a linha dele gravada por [`gravar_alvo_por_tags`].
pub(crate) unsafe fn alvo_por_tags(frame: *mut c_void, out: *mut c_void, classe: &str, estados: &Mutex<Vec<(usize, Box<[u64]>)>>) {
    let (_, arrs) = rtti::read_params_consuming_with_cnamearray(current_native_func(), frame);
    let tags: Vec<u64> = arrs.into_iter().next().flatten().unwrap_or_default();
    let quantas = tags.len();
    let obj = alvo_novo(classe, |o| gravar_alvo_por_tags(estados, o, tags));
    crate::log_dev!("[alvo] {classe}.Tags({quantas} tags) -> {obj:p}");
    rtti::make_handle(out, obj);
}

/// A linha `(alvo, tags)` do `Tags` na tabela `estados`, a mesma do `Tag`. Array vazio também grava: `Matches` com `tags` vazio casa todo evento,
/// e o alvo sem linha sairia dos alvos suportados no despacho.
pub(crate) fn gravar_alvo_por_tags(estados: &Mutex<Vec<(usize, Box<[u64]>)>>, alvo: usize, tags: Vec<u64>) {
    if let Ok(mut s) = estados.lock() {
        s.push((alvo, tags.into_boxed_slice()));
    }
}

/// Uma linha por `StaticEntityTarget`: (ponteiro do alvo, tags do `Tag`/`Tags`; vazio = casa todo evento).
pub(crate) static STATICENTITYTARGET_STATES: Mutex<Vec<(usize, Box<[u64]>)>> = Mutex::new(Vec::new());

/// `StaticEntityTarget.Tag(tag: CName) -> ref<StaticEntityTarget>` — mesma receita de
/// `tramp_dyt_tag`.
pub(crate) unsafe extern "C" fn tramp_stt_tag(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let tag = args.first().map(|(v, _)| *v).unwrap_or(0);
    let obj = alvo_novo("StaticEntityTarget", |o| {
        if let Ok(mut states) = STATICENTITYTARGET_STATES.lock() {
            states.push((o, Box::new([tag])));
        }
    });
    crate::log_dev!(
        "[stt] StaticEntityTarget.Tag('{}' hash {tag:#018x}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))",
        crate::cname::resolve_cname(tag)
    );
    rtti::make_handle(out, obj);
}

/// Forja `StaticEntityTarget extends CallbackSystemTarget` + registra `Tag`/`Tags`.
pub unsafe fn register_staticentitytarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_staticentitytarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "StaticEntityTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_staticentitytarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_staticentitytarget: classe=OK mas sem protótipo — Tag NÃO registrado".into(),
    };
    let m1 = register_method(reg, "StaticEntityTarget", proto, "Tag", "Tag", tramp_stt_tag, true);
    let m2 = register_method(reg, "StaticEntityTarget", proto, "Tags", "Tags", tramp_stt_tags, true);
    format!("[reg] StaticEntityTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) Tag={m1} Tags={m2}")
}

/// `EntityTarget` (dispatch-time filtering, 2026-08-12): TAG de qual das 4
/// factories construiu um `EntityTarget` — `Type`/`RecordID`/`Template`/`Appearance` compartilhavam
/// `ENTITYTARGET_STATES` como um `u64` cru sem distinguir a PROVENIÊNCIA (achado documentado no
/// item `#6` do catálogo: "o critério de filtro é oportunístico/permissivo, não filtra fundo
/// hoje"). Pra implementar filtragem REAL de `RecordID` sem risco de comparar por engano um valor de
/// `Type`/`Template`(ResRef)/`Appearance`(CName) contra um `RecordID`(TweakDBID) — os 4 são
/// só `u64` opacos na struct, não há como distinguir sem esta tag — cada trampolina agora marca seu
/// critério. Puramente aditivo/interno (nunca exposto ao redscript, só bookkeeping Rust). O `u64` de
/// `Type` é a classe que a fábrica resolveu (`classe_do_alvo`: `CClass*`, 0 ou `CLASSE_NAO_RESOLVIDA`), não o `CName`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum EntityTargetCriterion {
    Id,
    Type,
    RecordId,
    Template,
    Appearance,
}

pub(crate) static ENTITYTARGET_STATES: Mutex<Vec<(usize, EntityTargetCriterion, u64)>> = Mutex::new(Vec::new());

/// `(ptr, appearancePath_hash, definitionName_hash)` — estado PRÓPRIO de `Definition` (2 args),
/// separado de `ENTITYTARGET_STATES` (que só guarda 1 critério) pra não forçar os outros 4 métodos
/// a carregar um campo extra sempre-zero.
pub(crate) static ENTITYTARGET_DEFINITION_STATES: Mutex<Vec<(usize, u64, u64)>> = Mutex::new(Vec::new());

/// Forja `EntityTarget extends CallbackSystemTarget` + registra `ID`/`Type`/`RecordID`/`Template`/
/// `Appearance`/`Definition`.
pub unsafe fn register_entitytarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_entitytarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "EntityTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_entitytarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_entitytarget: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m0 = register_method(reg, "EntityTarget", proto, "ID", "ID", tramp_et_id, true);
    crate::log_dev!("[reg] EntityTarget.ID={m0}");
    let m1 = register_method(reg, "EntityTarget", proto, "Type", "Type", tramp_et_type, true);
    let m2 = register_method(reg, "EntityTarget", proto, "RecordID", "RecordID", tramp_et_recordid, true);
    let m3 = register_method(reg, "EntityTarget", proto, "Template", "Template", tramp_et_template, true);
    let m4 = register_method(reg, "EntityTarget", proto, "Appearance", "Appearance", tramp_et_appearance, true);
    let m5 = register_method(reg, "EntityTarget", proto, "Definition", "Definition", tramp_et_definition, true);
    format!(
        "[reg] EntityTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) ID={m0} Type={m1} RecordID={m2} Template={m3} Appearance={m4} Definition={m5} \
— is_static+construct+handle NUMA CLASSE FORJADA: PROVADO 2026-08-05"
    )
}

/// `EntityLifecycleEvent extends CallbackSystemEvent` — controller de entidade (2026-07-18,
/// sessão `handle-ctor-re`, 7ª rodada). Declaração: `public native class EntityLifecycleEvent
/// extends CallbackSystemEvent { public native func GetEntity() -> wref<Entity> }`. É o evento
/// REAL de "Entity/Attach" e "Entity/Assemble" — RE confirmou que "Entity/Attach" carrega
/// `EntityLifecycleEvent`, NÃO `EntityBuilderEvent` (esse último é específico de "Entity/Extract",
/// que embrulha um `Red::EntityBuilder` inteiro via `EntityBuilderWrapper`: `register_entitybuilderevent`/
/// `register_entitybuilderwrapper`, abaixo).
///
/// `GetEntity()` devolve `wref<Entity>`, como o bundle declara (`ponte-callbacksystem.reds`), montado
/// por `rtti::make_weak_handle`: o handle `{ptr,0}` de `write_handle_ret` vale pra `ref<T>`, mas num
/// `wref` o `IsDefined()` lê `false` (prova de 2026-08-11 do weak handle).
pub(crate) static ENTITYLIFECYCLE_STATES: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_ele_get_entity(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    // `EntityComponentEvent` e `VehicleLightControlEvent` usam este `GetEntity` herdado: a dona do componente e o veículo.
    let entity = entidade_do_evento(c as usize);
    // 2026-08-11: wref<Entity> REAL via rtti::make_weak_handle — substitui o
    // handle "raw" write_handle_ret{ptr,0} que fazia IsDefined() falhar pra wref (revertido em
    // 2026-07-18). Ver nota completa acima e a prova de 2026-08-11 do weak handle.
    rtti::make_weak_handle(out, entity as *mut c_void);
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
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_entitylifecycleevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "EntityLifecycleEvent", proto, "GetEntity", "GetEntity", tramp_ele_get_entity, false);
    format!("[reg] EntityLifecycleEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetEntity={m1}")
}

/// Constrói um `EntityLifecycleEvent` REAL (mesma técnica de `make_keyinputevent_arg`) — o
/// OBJETO EVENTO em si é um `ref<>` FORTE (`make_handle`); o campo `entity` que `GetEntity()`
/// devolve é um **`wref<Entity>` de verdade** (`rtti::make_weak_handle`).
///
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
        // Um evento anterior no mesmo endereço que nunca saiu do estado (o despacho por presença do player não solta) faria o `GetEntity` achar a entidade
        // velha: no boot 166, 1 `Entity/Initialize` e 1 `Entity/Attach` chegaram ao handler com outra entidade.
        states.retain(|(p, _)| *p != obj as usize);
        states.push((obj as usize, entity_ptr as usize));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// Dispara `evento` com um `EntityLifecycleEvent` de `entity` e solta o evento na hora — a entrada do `GetEntity` sai do estado e o
/// handle forte criado aqui é solto como no #120 3b-2 (um ouvinte que guardou o evento continua com a própria referência, e o `GetEntity` dele passa a
/// devolver vazio). Quem chama são hooks do ciclo de vida da entidade, milhares de vezes por minuto e quase sempre fora da thread do jogo (boot 164):
/// sem soltar, o estado crescia um item por evento.
pub unsafe fn disparar_evento_de_entidade(evento: &str, entity: *mut c_void) -> usize {
    let Some(arg) = make_entitylifecycleevent_arg(entity) else {
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args(evento, &[arg]);
    if let Ok(mut states) = ENTITYLIFECYCLE_STATES.lock() {
        states.retain(|(p, _)| *p as u64 != inst);
    }
    let mut h = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut h);
    enviados
}

/// (objeto do wrapper, builder, bloco de referência do `WeakPtr`, aparência): aparência 0 no wrapper do builder e no do template.
pub(crate) static EBW_VIVOS: Mutex<Vec<(usize, u64, u64, u64)>> = Mutex::new(Vec::new());

/// (objeto do `EntityBuilderEvent`, handle forte do wrapper do builder).
pub(crate) static EBE_ESTADOS: Mutex<Vec<(usize, [u64; 2])>> = Mutex::new(Vec::new());

/// Alguém ouve `Entity/Extract`: o hook do job do builder só lê este atômico.
pub static EXTRACT_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Alguém ouve `Entity/Assemble`: o hook do `Entity::Assemble` só lê este atômico.
pub static ASSEMBLE_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe extern "C" fn tramp_ebe_get_entity_builder(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let wrapper = EBE_ESTADOS.lock().ok().and_then(|s| s.iter().rev().find(|e| e.0 == c as usize).map(|e| e.1[0])).unwrap_or(0);
    if wrapper == 0 {
        write_handle_ret(out, 0);
    } else {
        rtti::make_handle(out, wrapper as *mut c_void);
    }
}

/// `Entity/Extract`: dispara o evento com um `EntityBuilderWrapper` do builder `builder` (já travado por quem chama) e solta tudo depois
/// — os wrappers desse builder saem do estado, então um ouvinte que guardou algum lê vazio depois do despacho.
pub unsafe fn disparar_extract(builder: u64, rc: u64) -> usize {
    let Some(reg) = rtti::Registry::obtain() else { return 0 };
    let (wrapper, evento) = (rtti::new_object(&reg, "EntityBuilderWrapper"), rtti::new_object(&reg, "EntityBuilderEvent"));
    if wrapper.is_null() || evento.is_null() {
        return 0;
    }
    let handle_de = |obj: *mut c_void| {
        let mut buf = [0u8; 16];
        rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
        [(buf.as_ptr() as *const u64).read_unaligned(), (buf.as_ptr().add(8) as *const u64).read_unaligned()]
    };
    let mut hw = handle_de(wrapper);
    let mut he = handle_de(evento);
    if let Ok(mut s) = EBW_VIVOS.lock() {
        s.push((wrapper as usize, builder, rc, 0));
    }
    if let Ok(mut s) = EBE_ESTADOS.lock() {
        s.push((evento as usize, hw));
    }
    let enviados = fire_event_args("Entity/Extract", &[rtti::Arg::Handle(he[0] as *mut c_void, he[1] as *mut c_void)]);
    if let Ok(mut s) = EBE_ESTADOS.lock() {
        s.retain(|e| e.0 != evento as usize);
    }
    if let Ok(mut s) = EBW_VIVOS.lock() {
        s.retain(|e| e.1 != builder);
    }
    crate::jogo::objetos::soltar_handle(&mut he);
    crate::jogo::objetos::soltar_handle(&mut hw);
    enviados
}

pub unsafe fn register_entitybuilderwrapper(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) {
        return "[reg] register_entitybuilderwrapper: 'IScriptable' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "EntityBuilderWrapper", "gameuiInGameMenuGameController", iscriptable);
    if forged.is_null() {
        return "[reg] register_entitybuilderwrapper: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_entitybuilderwrapper: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "EntityBuilderWrapper", proto, "GetRecordID", "GetRecordID", tramp_ebw_get_record_id, false);
    let m2 = register_method(reg, "EntityBuilderWrapper", proto, "GetEntityID", "GetEntityID", tramp_ebw_get_entity_id, false);
    let m3 = register_method(reg, "EntityBuilderWrapper", proto, "GetAppearanceName", "GetAppearanceName", tramp_ebw_get_appearance_name, false);
    let m4 = register_method(reg, "EntityBuilderWrapper", proto, "HasEntity", "HasEntity", tramp_ebw_has_entity, false);
    let m5 = register_method(reg, "EntityBuilderWrapper", proto, "HasAppearance", "HasAppearance", tramp_ebw_has_appearance, false);
    let m6 = register_method(reg, "EntityBuilderWrapper", proto, "HasCustomAppearances", "HasCustomAppearances", tramp_ebw_has_custom_appearances, false);
    // Os outros 6 métodos sobre o builder vivo (formas próprias onde o corpus vanilla não declara o tipo de retorno).
    let m7 = register_method(reg, "EntityBuilderWrapper", proto, "GetTemplatePath", "GetTemplatePath", tramp_ebw_get_template_path, false);
    let m8 = register_method(reg, "EntityBuilderWrapper", proto, "GetEntityType", "GetEntityType", tramp_ebw_get_entity_type, false);
    let m9 = register_method(reg, "EntityBuilderWrapper", proto, "HasEntityParams", "HasEntityParams", tramp_ebw_has_entity_params, false);
    let m10 = register_method(reg, "EntityBuilderWrapper", proto, "GetTemplate", "GetTemplate", tramp_ebw_get_template, false);
    let m11 = register_method(reg, "EntityBuilderWrapper", proto, "GetAppearance", "GetAppearance", tramp_ebw_get_appearance, false);
    let m12 = register_method(reg, "EntityBuilderWrapper", proto, "GetCustomAppearances", "GetCustomAppearances", tramp_ebw_get_custom_appearances, false);
    format!(
        "[reg] EntityBuilderWrapper: classe=OK (INSTANCIÁVEL, parent=IScriptable) GetRecordID={m1} GetEntityID={m2} GetAppearanceName={m3} HasEntity={m4} HasAppearance={m5} HasCustomAppearances={m6} GetTemplatePath={m7} GetEntityType={m8} HasEntityParams={m9} GetTemplate={m10} GetAppearance={m11} GetCustomAppearances={m12}"
    )
}

/// `EntityBuilderTemplateWrapper` (pai `IScriptable`), sobre o template e os resultados do extrator do builder.
pub unsafe fn register_ebw_template(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) || register_type_instantiable_with_parent(reg, "EntityBuilderTemplateWrapper", "gameuiInGameMenuGameController", iscriptable).is_null() {
        return "[reg] register_ebw_template: forja da classe FALHOU".into();
    }
    let Some(proto) = method_donor_proto(reg) else {
        return "[reg] register_ebw_template: classe=OK mas sem protótipo".into();
    };
    let c = "EntityBuilderTemplateWrapper";
    let r = [
        register_method(reg, c, proto, "GetResourcePath", "GetResourcePath", tramp_ebw_get_template_path, false),
        register_method(reg, c, proto, "GetAppearanceName", "GetAppearanceName", tramp_ebt_get_appearance_name, false),
        register_method(reg, c, proto, "GetEntity", "GetEntity", tramp_ebt_get_entity, false),
        register_method(reg, c, proto, "GetComponents", "GetComponents", tramp_ebt_get_components, false),
        register_method(reg, c, proto, "AddComponent", "AddComponent", tramp_ebt_add_component, false),
    ];
    format!("[reg] EntityBuilderTemplateWrapper: classe=OK métodos {r:?}")
}

/// `EntityBuilderEvent` (pai `CallbackSystemEvent`) com `GetEntityBuilder() -> ref<EntityBuilderWrapper>`.
pub unsafe fn register_entitybuilderevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) || register_type_instantiable_with_parent(reg, "EntityBuilderEvent", "gameuiInGameMenuGameController", cse).is_null() {
        return "[reg] register_entitybuilderevent: forja da classe FALHOU".into();
    }
    let Some(proto) = method_donor_proto(reg) else {
        return "[reg] register_entitybuilderevent: classe=OK mas sem protótipo".into();
    };
    let m = register_method(reg, "EntityBuilderEvent", proto, "GetEntityBuilder", "GetEntityBuilder", tramp_ebe_get_entity_builder, false);
    format!("[reg] EntityBuilderEvent: classe=OK GetEntityBuilder={m}")
}

/// `redTagSystem` de um `GameTagSystem` conferido pela classe; nulo com outro objeto.
pub(crate) unsafe fn tagsystem_do_jogo(sys: *mut c_void) -> *mut c_void {
    if sys.is_null() || !rtti::sane(sys) {
        return std::ptr::null_mut();
    }
    let classe = rtti::type_name_getname(rtti::class_of(sys));
    #[cfg(feature = "devtools")] crate::selftest::c220_classe(classe); // anotação do teste 220, só no dev
    if classe != crate::cname::cname("gameGameTagSystem") {
        return std::ptr::null_mut();
    }
    (sys as *mut u8).add(GAMETAGSYSTEM_TAGSYSTEM_OFF) as *mut c_void
}

/// `TagSystem::AssignTag` (`marcar`) ou `UnassignTag` do motor no `GameTagSystem` do script; `false` sem sistema, id ou tag.
pub(crate) unsafe fn tag_do_motor(sys: *mut c_void, marcar: bool, id: u64, tag: u64) -> bool {
    let ts = tagsystem_do_jogo(sys);
    if ts.is_null() || id == 0 || tag == 0 {
        return false;
    }
    let f: extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(crate::rebase(if marcar { TAG_ASSIGN_VM } else { TAG_UNASSIGN_VM }));
    f(ts, id, tag);
    true
}

/// `GameTagSystem` da sessão (`PlayerPuppet.GetGame` + mapa de sistemas do `GameInstance`), pro `AssignTag`/`UnassignTag` do `DynamicEntitySystem` e do
/// `StaticEntitySystem`, que não recebem o sistema; nulo fora do gameplay.
pub(crate) unsafe fn gametagsystem_da_sessao() -> *mut c_void {
    sistema_da_sessao("gameGameTagSystem", None)
}

/// As 2 natives de tag (`bwms-tags.reds`) e a anotação do teste.
pub(crate) unsafe fn register_tagsystem_natives(reg: &Registry, proto: *mut c_void) {
    let tag = |nome: &str, tramp: NativeHandler| {
        let p = compose_params_from_types(reg, &["handle:IScriptable", "EntityID", "CName"]);
        params_compostos(p) && register_global_composed(reg, proto, p, nome, nome, tramp)
    };
    let marcar = tag("BwmsTagSystemAssignTag", tramp_tagsystem_assign);
    let desmarcar = tag("BwmsTagSystemUnassignTag", tramp_tagsystem_unassign);
    let anotar = {
        let p = compose_params_from_types(reg, &["Int32", "Int32"]);
        params_compostos(p) && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC220Anotar", "BwmsC220Anotar", corpo_de_prova!(tramp_c220_anotar)))
    };
    crate::log_dev!("[reg] register_all: c220 tags do motor AssignTag={marcar} UnassignTag={desmarcar} BwmsC220Anotar={anotar}");
}

/// `ResolveRecordID` do Mac (`0x1020a096c`): `TweakDBID f(entidade, PopulationSpawnParameter*)` — o 1º `gameRecordIdSpawnModifier` do parâmetro.
pub(crate) const RESOLVE_RECORD_ID_VM: u64 = 0x1020a096c;

/// Setter do record do puppet (`0x1020a0954`): grava em `+0x25C` só id válido.
pub(crate) const PUPPET_GRAVA_RECORD_VM: u64 = 0x1020a0954;

/// Antes do `Initialize`: o record de puppet e veículo pelo `populationParam` do request (`+0x90`, `Handle<PopulationSpawnParameter>`). O motor só
/// resolve nos virtuais `+0x148`/`+0x150`, depois do `RequestComponents` onde o `Entity/Initialize` sai.
pub(crate) unsafe fn pre_resolver_record(entidade: *mut c_void, request: *mut c_void) {
    if entidade.is_null() || request.is_null() || !crate::memoria::is_readable(request as *const c_void, 0xa0) {
        return;
    }
    let Some(reg) = rtti::Registry::obtain() else { return };
    let cls = rtti::class_of(entidade);
    if cls.is_null() {
        return;
    }
    let herda = |nome: &str| {
        let c = reg.class_by_name(nome);
        rtti::sane(c) && class_is_kind_of(cls, c)
    };
    let puppet = herda("gamePuppetBase");
    if !puppet && !herda("vehicleBaseObject") && !herda("VehicleObject") {
        return;
    }
    let param = ((request as *const u8).add(0x90) as *const u64).read_unaligned();
    let resolver: unsafe extern "C" fn(*mut c_void, u64) -> u64 = std::mem::transmute(crate::rebase(RESOLVE_RECORD_ID_VM));
    let id = resolver(entidade, param);
    if id & 0xffff_ffff == 0 || (id >> 32) & 0xff == 0 {
        return;
    }
    if puppet {
        let gravar: unsafe extern "C" fn(*mut c_void, u64) = std::mem::transmute(crate::rebase(PUPPET_GRAVA_RECORD_VM));
        gravar(entidade, id);
        #[cfg(feature = "devtools")]
        RECORDS_PRE_RESOLVIDOS[0].fetch_add(1, Ordering::Relaxed);
    } else {
        let campo = (entidade as *mut u8).add(0x658);
        if crate::memoria::is_readable(campo as *const c_void, 8) {
            (campo as *mut u64).write_unaligned(id);
            #[cfg(feature = "devtools")]
            RECORDS_PRE_RESOLVIDOS[1].fetch_add(1, Ordering::Relaxed);
        }
    }
}

pub(crate) unsafe fn register_entity_builder_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["handle:Entity", "handle:IComponent"]);
    let ok = if params_compostos(params) {
        register_global_composed(reg, proto, params, "BwmsEntityAddComponent", "BwmsEntityAddComponent", tramp_entity_add_component)
    } else {
        crate::log_dev!("[reg] register_entity_builder_natives: compose_params_from_types falhou (handle:Entity/handle:IComponent)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-entity-builder BwmsEntityAddComponent={ok}");
    // `#24` (cont.187): `FindComponentByType` — mesma DynArray de `AddComponent`, leitura em vez
    // de escrita.
    let params2 = compose_params_from_types(reg, &["handle:Entity", "CName"]);
    let ok2 = if params_compostos(params2) {
        register_global_composed(
            reg,
            proto,
            params2,
            "BwmsEntityFindComponentByType",
            "BwmsEntityFindComponentByType",
            tramp_entity_find_component_by_type,
        )
    } else {
        crate::log_dev!("[reg] register_entity_builder_natives: compose_params_from_types falhou (handle:Entity/CName)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-entity-builder BwmsEntityFindComponentByType={ok2}");
    // `#24` (cont.190): `GetComponents` — 2º array-retorno deste projeto, mesma técnica de
    // trailer-emprestado do `GetRecords` (cont.189), mas usando o trailer do PRÓPRIO array lido.
    let params3 = compose_params_from_types(reg, &["handle:Entity"]);
    let ok3 = if params_compostos(params3) {
        register_global_composed(reg, proto, params3, "BwmsEntityGetComponents", "BwmsEntityGetComponents", tramp_entity_get_components)
    } else {
        crate::log_dev!("[reg] register_entity_builder_natives: compose_params_from_types falhou (handle:Entity)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-entity-builder BwmsEntityGetComponents={ok3}");
    // `#24` (cont.191): `GetTemplatePath` — leitura direta de campo (`Entity+0x60`), zero RE nova.
    let params4 = compose_params_from_types(reg, &["handle:Entity"]);
    let ok4 = if params_compostos(params4) {
        register_global_composed(reg, proto, params4, "BwmsEntityGetTemplatePath", "BwmsEntityGetTemplatePath", tramp_entity_get_template_path)
    } else {
        crate::log_dev!("[reg] register_entity_builder_natives: compose_params_from_types falhou (handle:Entity, GetTemplatePath)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-entity-builder BwmsEntityGetTemplatePath={ok4}");
    // `#24` (2026-09-11): `SetWorldTransform` por composição — só o `transformComponent@0xB0` é nativo.
    let params_t = compose_params_from_types(reg, &["handle:Entity"]);
    let ok_t = params_compostos(params_t)
        && register_global_composed(
            reg,
            proto,
            params_t,
            "BwmsEntityGetTransformComponent",
            "BwmsEntityGetTransformComponent",
            tramp_entity_get_transform_component,
        );
    crate::log_dev!("[reg] register_all: cw-entity-builder BwmsEntityGetTransformComponent={ok_t}");
    // `#24` (2026-09-11): `ApplyMorphTarget` = `0x10197f17c`; as 3 leituras dos pesos servem à prova e a diagnóstico.
    let p_aplicar = compose_params_from_types(reg, &["handle:Entity", "CName", "CName", "Float"]);
    let ok_aplicar = params_compostos(p_aplicar)
        && register_global_composed(reg, proto, p_aplicar, "BwmsEntityApplyMorphTarget", "BwmsEntityApplyMorphTarget", tramp_entity_apply_morph_target as NativeHandler);
    let p_peso = compose_params_from_types(reg, &["handle:Entity", "CName", "CName"]);
    let ok_peso = params_compostos(p_peso)
        && registro_de_prova!(register_global_composed(reg, proto, p_peso, "BwmsEntityGetMorphTargetWeight", "BwmsEntityGetMorphTargetWeight", corpo_de_prova!(tramp_entity_get_morph_target_weight)));
    let p_n = compose_params_from_types(reg, &["handle:Entity"]);
    let ok_n = params_compostos(p_n)
        && registro_de_prova!(register_global_composed(reg, proto, p_n, "BwmsEntityGetMorphTargetCount", "BwmsEntityGetMorphTargetCount", corpo_de_prova!(tramp_entity_get_morph_target_count)));
    let p_nome = compose_params_from_types(reg, &["handle:Entity", "Int32", "Bool"]);
    let ok_nome = params_compostos(p_nome)
        && registro_de_prova!(register_global_composed(reg, proto, p_nome, "BwmsEntityGetMorphTargetName", "BwmsEntityGetMorphTargetName", corpo_de_prova!(tramp_entity_get_morph_target_name)));
    crate::log_dev!(
        "[reg] register_all: cw-entity-builder BwmsEntityApplyMorphTarget={ok_aplicar} GetMorphTargetWeight={ok_peso} GetMorphTargetCount={ok_n} GetMorphTargetName={ok_nome}"
    );
    // `#30` (cont.192): `MeshComponent.meshResource` — genérico (usa "handle:IComponent" já provado por AddComponent).
    // Só no dev: nenhum `.reds` declara (quarentena do CMesh-trap).
    #[cfg(feature = "devtools")]
    {
        let params5 = compose_params_from_types(reg, &["handle:IComponent"]);
        let ok5 = if params_compostos(params5) {
            register_global_composed(
                reg,
                proto,
                params5,
                "BwmsMeshComponentGetMeshResource",
                "BwmsMeshComponentGetMeshResource",
                tramp_meshcomponent_get_mesh_resource,
            )
        } else {
            crate::log_dev!("[reg] register_entity_builder_natives: compose_params_from_types falhou (handle:IComponent, GetMeshResource)");
            false
        };
        crate::log_dev!("[reg] register_all: cw-meshcomponent BwmsMeshComponentGetMeshResource={ok5}");
    }
}

// ===== `#75` (`StaticEntitySystem`) — implementação NOVA, 2026-08-15 =========================
//
// Módulo IRMÃO de `DynamicEntitySystem` (#73, FECHADO acima), nunca implementado antes desta rodada
// (confirmado por grep: zero ocorrência de "StaticEntitySystem"/"tramp_ses_" neste arquivo e em
// `blackwall-mods-dev/*.reds` antes desta mudança). 18 métodos. Mesmo subconjunto de risco BAIXO já aplicado ao `#73`:
// tag-bookkeeping puro (registry Rust-side, `HashMap<u64,HashSet<u64>>`, zero ponteiro de Entity,
// zero endereço nativo) — `IsManaged`/`IsTagged`/`AssignTag`/`UnassignTag`/`IsPopulated` (mapa
// `EntityID → Set<CName>` + o espelho `CName → Set<EntityID>` — o MESMO par de mapas já usado no `#73`).
//
// `SpawnEntity`/`DespawnEntity`/`AttachEntity`/`DetachEntity`/`IsSpawned`/`IsSpawning`/`GetEntity`/
// `GetTags`/`GetTagged`/`GetTaggedIDs`/`DespawnTagged`/`AttachTagged`/`DetachTagged` vão pelo
// `EntitySpawner` do motor (bloco de `SPAWNER_SPAWN_VM`). `IsManaged` = spawnada pelo BWMS
// (`statent_gerenciada`) ou já tagueada.
//
// `#75` NÃO tem `IsRestored` nem `DeleteTagged` na API real (diferença genuína vs `#73`) — não
// inventados aqui.
// Idioma "Count()+ByIndex(i)" (5 getters escalares) reusa a MESMA disciplina anti-array-retorno já
// estabelecida pro `#73`/round 8, `ReflectionClass`, `Scripting.GetStackTrace`.
pub(crate) type StatentMap = std::sync::Mutex<std::collections::HashMap<u64, std::collections::HashSet<u64>>>;

pub(crate) fn statent_tags() -> &'static StatentMap {
    STATENT_TAGS_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn statent_by_tag() -> &'static StatentMap {
    STATENT_BY_TAG_CELL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(crate) const REGISTRY_UNREGISTER_VM: u64 = 0x1_01a2_bf84;

pub(crate) const ENT_STATUS_OFF: usize = 0x156;

pub(crate) const ENT_UNINITIALIZED: u8 = 7;

/// `EntitySpawnerRequest` (0xF0 B, zerado): caminho `+0x00`, posição 3×i32 `+0x10`, quaternion `+0x20`, id `+0x30`, aparência `+0x38`,
/// FlexCallback `+0x48` (buffer 24 B com o id, tabela `+0x60`, alocador `+0x68`, tamanho estendido `+0x70` = 0), detached `+0x78`,
/// prioridade `+0xE1` (1 = Normal).
#[repr(C, align(16))]
pub(crate) struct PedidoSpawner(pub(crate) [u8; 0xF0]);

impl PedidoSpawner {
    pub(crate) fn put<T: Copy>(&mut self, off: usize, v: T) {
        unsafe { std::ptr::write_unaligned(self.0.as_mut_ptr().add(off) as *mut T, v) };
    }
}

/// Estado de uma entidade do `SpawnEntity`: o `SharedPtr<EntitySpawnerToken>` enquanto o spawn corre, o `Handle<Entity>` depois do callback.
pub(crate) struct EntidadeEstatica {
    pub(crate) rse: usize,
    pub(crate) registry: usize,
    pub(crate) token: [u64; 2],
    pub(crate) ent: [u64; 2],
    pub(crate) registrada: bool,
    pub(crate) descartada: bool,
}

pub(crate) static STATENT_ESTADOS: std::sync::Mutex<std::collections::BTreeMap<u64, EntidadeEstatica>> = std::sync::Mutex::new(std::collections::BTreeMap::new());

#[cfg(feature = "devtools")]
pub(crate) static STATENT_TRILHA: std::sync::Mutex<Vec<(u64, &'static str, i64)>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) fn statent_trilhar(id: u64, passo: &'static str, valor: i64) {
    crate::log_dev!("[statent] id={id} {passo} {valor}");
    if let Ok(mut t) = STATENT_TRILHA.lock() {
        if t.len() < 128 {
            t.push((id, passo, valor));
        }
    }
}

pub(crate) fn statent_esquecer_tags(id: u64) {
    if let (Ok(mut porid), Ok(mut portag)) = (statent_tags().lock(), statent_by_tag().lock()) {
        for tag in porid.remove(&id).unwrap_or_default() {
            if let Some(s) = portag.get_mut(&tag) {
                s.remove(&id);
            }
        }
    }
}

/// Tick da thread do jogo: limpeza das entidades descartadas.
pub(crate) unsafe fn statent_tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    if !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) {
        return;
    }
    let desinicializada = |ent: u64| {
        let s = (ent as usize).wrapping_add(ENT_STATUS_OFF) as *const u8;
        ent != 0 && crate::memoria::is_readable(s as *const c_void, 1) && s.read() == ENT_UNINITIALIZED
    };
    let soltas: Vec<(u64, EntidadeEstatica)> = match STATENT_ESTADOS.lock() {
        Ok(mut m) => {
            let ids: Vec<u64> = m.iter().filter(|(_, e)| e.descartada && desinicializada(e.ent[0])).map(|(id, _)| *id).collect();
            ids.into_iter().filter_map(|id| m.remove(&id).map(|e| (id, e))).collect()
        }
        Err(_) => return,
    };
    for (id, mut e) in soltas {
        if e.registrada {
            let desregistrar: unsafe extern "C" fn(usize, u64) = std::mem::transmute(crate::rebase(REGISTRY_UNREGISTER_VM));
            desregistrar(e.registry, e.ent[0]);
        }
        crate::jogo::objetos::soltar_handle(&mut e.ent);
        statent_esquecer_tags(id);
        #[cfg(feature = "devtools")]
        statent_trilhar(id, "solta", 0);
    }
}

/// Fim de sessão: o shutdown do RSE cancela os spawns e descarta as entidades com o mundo; o BWMS só solta o que segura e esquece.
pub(crate) unsafe fn statent_fim_de_sessao() {
    let todos: Vec<EntidadeEstatica> = STATENT_ESTADOS.lock().map(|mut m| std::mem::take(&mut *m).into_values().collect()).unwrap_or_default();
    for mut e in todos {
        dynent_soltar_token(e.token);
        crate::jogo::objetos::soltar_handle(&mut e.ent);
    }
    if let (Ok(mut porid), Ok(mut portag)) = (statent_tags().lock(), statent_by_tag().lock()) {
        porid.clear();
        portag.clear();
    }
}

pub unsafe fn register_staticentitysystem(reg: &Registry) -> String {
    let igs = reg.class_by_name("IGameSystem");
    let igs = if rtti::sane(igs) { igs } else { resolve_class_via_validator_getclass("IGameSystem") };
    if !rtti::sane(igs) {
        return "[reg] register_staticentitysystem: 'IGameSystem' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "StaticEntitySystem", "gameuiInGameMenuGameController", igs);
    if forged.is_null() {
        return "[reg] register_staticentitysystem: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_staticentitysystem: sem protótipo de método".into(),
    };
    let m1 = register_method(reg, "StaticEntitySystem", proto, "IsReady", "IsReady", tramp_ses_is_ready, false);
    let m2 = register_method(reg, "StaticEntitySystem", proto, "IsManaged", "IsManaged", tramp_ses_is_managed, false);
    let m3 = register_method(reg, "StaticEntitySystem", proto, "IsTagged", "IsTagged", tramp_ses_is_tagged, false);
    let m4 = register_method(reg, "StaticEntitySystem", proto, "IsPopulated", "IsPopulated", tramp_ses_is_populated, false);
    let m5 = register_method(reg, "StaticEntitySystem", proto, "AssignTag", "AssignTag", tramp_ses_assign_tag, false);
    let m6 = register_method(reg, "StaticEntitySystem", proto, "UnassignTag", "UnassignTag", tramp_ses_unassign_tag, false);
    let m7 = register_method(reg, "StaticEntitySystem", proto, "GetTaggedID", "GetTaggedID", tramp_ses_get_tagged_id, false);
    let m8 = register_method(reg, "StaticEntitySystem", proto, "GetTaggedCount", "GetTaggedCount", tramp_ses_get_tagged_count, false);
    let m9 = register_method(reg, "StaticEntitySystem", proto, "GetTaggedIDByIndex", "GetTaggedIDByIndex", tramp_ses_get_tagged_id_by_index, false);
    let m10 = register_method(reg, "StaticEntitySystem", proto, "GetTagCount", "GetTagCount", tramp_ses_get_tag_count, false);
    let m11 = register_method(reg, "StaticEntitySystem", proto, "GetTagByIndex", "GetTagByIndex", tramp_ses_get_tag_by_index, false);
    // #75 (2026-09-18): spawn por template pelo EntitySpawner do motor, attach/detach, despawn, arrays e operações por tag.
    let novos = [
        ("SpawnEntity", tramp_ses_spawn_entity as NativeHandler),
        ("DespawnEntity", tramp_ses_despawn_entity),
        ("AttachEntity", tramp_ses_attach_entity),
        ("DetachEntity", tramp_ses_detach_entity),
        ("IsSpawned", tramp_ses_is_spawned),
        ("IsSpawning", tramp_ses_is_spawning),
        ("GetEntity", tramp_ses_get_entity),
        ("GetTags", tramp_ses_get_tags),
        ("GetTagged", tramp_ses_get_tagged),
        ("GetTaggedIDs", tramp_ses_get_tagged_ids),
        ("DespawnTagged", tramp_ses_despawn_tagged),
        ("AttachTagged", tramp_ses_attach_tagged),
        ("DetachTagged", tramp_ses_detach_tagged),
    ];
    let registrados: Vec<String> = novos.iter().map(|(nome, f)| format!("{nome}={}", register_method(reg, "StaticEntitySystem", proto, nome, nome, *f, false))).collect();
    format!(
        "[reg] StaticEntitySystem: classe=OK (INSTANCIÁVEL, parent=IGameSystem real, mesma receita de DynamicEntitySystem) \
IsReady={m1} IsManaged={m2} IsTagged={m3} IsPopulated={m4} AssignTag={m5} UnassignTag={m6} \
GetTaggedID={m7} GetTaggedCount={m8} GetTaggedIDByIndex={m9} GetTagCount={m10} GetTagByIndex={m11} {}",
        registrados.join(" ")
    )
}

pub unsafe fn register_staticentityspec(reg: &Registry) -> String {
    let iscriptable = reg.class_by_name("IScriptable");
    if !rtti::sane(iscriptable) {
        return "[reg] register_staticentityspec: 'IScriptable' não resolveu".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "StaticEntitySpec", "gameuiInGameMenuGameController", iscriptable);
    if forged.is_null() {
        return "[reg] register_staticentityspec: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_staticentityspec: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "StaticEntitySpec", proto, "Create", "Create", tramp_sespec_create, true);
    let m2 = register_method(reg, "StaticEntitySpec", proto, "SetAppearanceName", "SetAppearanceName", tramp_sespec_set_appearance_name, false);
    let m3 = register_method(reg, "StaticEntitySpec", proto, "GetAppearanceName", "GetAppearanceName", tramp_sespec_get_appearance_name, false);
    let m4 = register_method(reg, "StaticEntitySpec", proto, "SetTemplatePath", "SetTemplatePath", tramp_sespec_set_template_path, false);
    let m5 = register_method(reg, "StaticEntitySpec", proto, "GetTemplatePath", "GetTemplatePath", tramp_sespec_get_template_path, false);
    let m6 = register_method(reg, "StaticEntitySpec", proto, "SetAttached", "SetAttached", tramp_sespec_set_attached, false);
    let m7 = register_method(reg, "StaticEntitySpec", proto, "GetAttached", "GetAttached", tramp_sespec_get_attached, false);
    let m8 = register_method(reg, "StaticEntitySpec", proto, "AddTag", "AddTag", tramp_sespec_add_tag, false);
    let m9 = register_method(reg, "StaticEntitySpec", proto, "GetTagCount", "GetTagCount", tramp_sespec_get_tag_count, false);
    let m10 = register_method(reg, "StaticEntitySpec", proto, "GetTagByIndex", "GetTagByIndex", tramp_sespec_get_tag_by_index, false);
    // 2026-08-15 rodada 10: position(Vector4)/orientation(Quaternion) — struct-de-16-bytes COMO
    // PARÂMETRO, 1ª vez nesta codebase (ver comentário completo em `StaticEntitySpecFields`).
    let m11 = register_method(reg, "StaticEntitySpec", proto, "SetPosition", "SetPosition", tramp_sespec_set_position, false);
    let m12 = register_method(reg, "StaticEntitySpec", proto, "GetPosition", "GetPosition", tramp_sespec_get_position, false);
    let m13 = register_method(reg, "StaticEntitySpec", proto, "SetOrientation", "SetOrientation", tramp_sespec_set_orientation, false);
    let m14 = register_method(reg, "StaticEntitySpec", proto, "GetOrientation", "GetOrientation", tramp_sespec_get_orientation, false);
    format!(
        "[reg] StaticEntitySpec: classe=OK (INSTANCIÁVEL, parent=IScriptable real, mesma receita de CallbackSystemHandler) \
Create={m1} SetAppearanceName={m2} GetAppearanceName={m3} SetTemplatePath={m4} GetTemplatePath={m5} \
SetAttached={m6} GetAttached={m7} AddTag={m8} GetTagCount={m9} GetTagByIndex={m10} \
SetPosition={m11} GetPosition={m12} SetOrientation={m13} GetOrientation={m14} \
— 6/6 campos reais cobertos (rodada 10: position/orientation Vector4/Quaternion via struct-de-16-bytes-como-PARÂMETRO, prova ao vivo pendente)"
    )
}


pub(crate) struct AlvoDoCicloDeVida {
    pub(crate) nome: &'static str,
    pub(crate) vm: u64,
    /// 1ª instrução da função, conferida antes do patch.
    pub(crate) prologo: u32,
    /// Status (`EntityStatus`) aceitos na entrada, um bit por valor; `0xff` = qualquer.
    pub(crate) status: u8,
}

pub(crate) const ALVOS_DO_CICLO_DE_VIDA: [AlvoDoCicloDeVida; 7] = [
    // Grava 1 (`Initializing`) logo depois do prólogo.
    AlvoDoCicloDeVida { nome: "Initialize", vm: 0x1_019a_a7d8, prologo: 0xa9ba_6ffc, status: 1 << 0 },
    // Chamado pelo `Initialize` (status 1, array da própria entidade) e pelo thunk do builder numa troca de aparência.
    AlvoDoCicloDeVida { nome: "RequestComponents", vm: 0x1_019a_4d04, prologo: 0xd107_c3ff, status: 1 << 1 | 1 << 2 | 1 << 3 | 1 << 4 },
    AlvoDoCicloDeVida { nome: "Reassemble", vm: 0x1_019a_7a30, prologo: 0xd106_03ff, status: 1 << 2 | 1 << 3 | 1 << 4 },
    // Grava 3 e sai com 4.
    AlvoDoCicloDeVida { nome: "Attach", vm: 0x1_0529_ec50, prologo: 0xd101_83ff, status: 1 << 2 },
    // Grava 5 e sai com 2.
    AlvoDoCicloDeVida { nome: "Detach", vm: 0x1_019a_b4dc, prologo: 0xd101_83ff, status: 1 << 4 },
    // Grava 6 e sai com 7.
    AlvoDoCicloDeVida { nome: "Uninitialize", vm: 0x1_019a_b2d4, prologo: 0xd101_03ff, status: 1 << 1 | 1 << 2 },
    // Desinicializa em linha só com status de 2 a 5 e sem cena.
    AlvoDoCicloDeVida { nome: "Dispose", vm: 0x1_019a_b7f8, prologo: 0xd101_03ff, status: 0xff },
];

pub(crate) static ORIG_CICLO_INITIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CICLO_REQUESTCOMPONENTS: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CICLO_ATTACH: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CICLO_DETACH: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CICLO_UNINITIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_CICLO_DISPOSE: AtomicU64 = AtomicU64::new(0);

/// Hooks do 130b no ar, bit `k` = hook de `ALVOS_DO_CICLO_DE_VIDA[k]`: o despacho do player pela presença pergunta pelo hook do próprio evento
/// ([`evento_do_player`]). Gravado pelo estado físico ([`gravar_hooks_do_ciclo_no_ar`]) no fim de cada instalação e desinstalação do grupo: a desinstalação pula
/// endereço sem hook, e um contador deixaria bit velho.
pub(crate) static HOOKS_DO_CICLO_NO_AR: AtomicU8 = AtomicU8::new(0);

/// Algum hook do 130b no ar. Só o selftest pergunta: o despacho do player decide hook a hook.
#[cfg(feature = "devtools")]
pub(crate) fn hooks_do_ciclo_ativos() -> bool {
    HOOKS_DO_CICLO_NO_AR.load(Ordering::Relaxed) != 0
}

/// Evento do ciclo de vida que `lib.rs` entrega ao player pela presença, ou `None`: `Entity/Attach` no tick atrasado depois da entrada (`presente`),
/// `Entity/Detach` na saída, com o último ponteiro. Sai só com entidade, com o hook de mesmo nome fora do ar (bit em `no_ar`, de [`HOOKS_DO_CICLO_NO_AR`]) e com
/// ouvinte: com o hook no ar o player chega duas vezes, e numa instalação parcial do grupo, com esse hook fora, a presença é a única entrega.
pub(crate) fn evento_do_player(presente: bool, entidade: *mut c_void, no_ar: u8, tem_ouvinte: impl Fn(&str) -> bool) -> Option<&'static str> {
    // Bits 3 e 4 de `no_ar`: os índices de `Attach` e `Detach` em `ALVOS_DO_CICLO_DE_VIDA`.
    let (evento, bit) = if presente { ("Entity/Attach", 3) } else { ("Entity/Detach", 4) };
    (!entidade.is_null() && no_ar & (1 << bit) == 0 && tem_ouvinte(evento)).then_some(evento)
}

/// Máscara de [`HOOKS_DO_CICLO_NO_AR`]: bit `k` ligado com hook nosso no endereço de `ALVOS_DO_CICLO_DE_VIDA[k]`, que `endereco` traduz do vm.
fn mascara_no_ar(endereco: impl Fn(u64) -> *mut c_void) -> u8 {
    hooks_do_ciclo().iter().fold(0, |mascara, &(k, _, _)| mascara | ((ganchos::ganchos_no_endereco(endereco(ALVOS_DO_CICLO_DE_VIDA[k].vm)) > 0) as u8) << k)
}

/// Grava [`HOOKS_DO_CICLO_NO_AR`] pelo estado físico. Quem chama segura `HOOKS_DE_EVENTO`, como a instalação e a desinstalação do grupo.
pub(crate) fn gravar_hooks_do_ciclo_no_ar() {
    HOOKS_DO_CICLO_NO_AR.store(mascara_no_ar(crate::rebase), Ordering::Relaxed);
}

/// Despacha o evento `ENTIDADE_EVENTOS[k]` com a entidade.
pub(crate) unsafe fn despachar_evento_do_ciclo(k: usize, entidade: *mut c_void) {
    let Some(&evento) = ENTIDADE_EVENTOS.get(k) else { return };
    // Só o atômico: o hook roda milhares de vezes por minuto, quase sempre fora da thread do jogo.
    if entidade.is_null() || C130B_CONGELADO.load(Ordering::Relaxed) || !evento_de_entidade_armado(k) {
        return;
    }
    #[cfg(not(feature = "devtools"))]
    disparar_evento_de_entidade(evento, entidade);
    #[cfg(feature = "devtools")]
    {
        let atual = C130B_ATUAL.with(|c| c.replace(entidade as usize));
        let anotacoes = C130B_ANOTOU.with(|c| c.replace(0));
        let enviados = disparar_evento_de_entidade(evento, entidade);
        let anotou = C130B_ANOTOU.with(|c| c.replace(anotacoes));
        C130B_ATUAL.with(|c| c.set(atual));
        if enviados > 0 {
            C130B_DESPACHOS[k].fetch_add(1, Ordering::Relaxed);
            if anotou == 0 {
                C130B_SEM_ANOTACAO[k].fetch_add(1, Ordering::Relaxed);
            }
            if enviados > 1 {
                C130B_VARIOS_OUVINTES[k].fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

/// Gancho de `Entity::Initialize` (`ALVOS_DO_CICLO_DE_VIDA[0]`).
pub(crate) unsafe extern "C" fn ciclo_initialize_hook(entidade: *mut c_void, fila: *mut c_void, request: *mut c_void) {
    if evento_de_entidade_armado(0) {
        pre_resolver_record(entidade, request);
    }
    let original: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) = std::mem::transmute(original_publicado(&ORIG_CICLO_INITIALIZE));
    original(entidade, fila, request);
}

/// Gancho de `RequestComponents` (`ALVOS_DO_CICLO_DE_VIDA[1]`).
pub(crate) unsafe extern "C" fn ciclo_requestcomponents_hook(entidade: *mut c_void, a2: u64, componentes: *mut c_void) {
    // O `Initialize` passa o array da própria entidade; outro array é a troca de aparência pelo builder.
    if !entidade.is_null() && componentes as usize == entidade as usize + 0xa0 {
        despachar_evento_do_ciclo(0, entidade);
    } else if !entidade.is_null() && evento_de_entidade_armado(1) {
        let antes = crate::aparencia::entidade_num_componentes(entidade);
        despachar_evento_do_ciclo(1, entidade);
        let _propagados = crate::aparencia::entidade_propagar_componentes(entidade, componentes, antes);
        #[cfg(feature = "devtools")]
        {
            C130E_PROPAGADOS.fetch_add(_propagados as u64, Ordering::Relaxed);
            let somado = C130E_SOMADO.with(|c| c.replace(0));
            if somado != 0 && c130e_array_tem(componentes, somado) {
                C130E_NO_ARRAY_NOVO.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    let original: unsafe extern "C" fn(*mut c_void, u64, *mut c_void) = std::mem::transmute(original_publicado(&ORIG_CICLO_REQUESTCOMPONENTS));
    original(entidade, a2, componentes);
}

/// Gancho de `Attach` (`ALVOS_DO_CICLO_DE_VIDA[3]`).
pub(crate) unsafe extern "C" fn ciclo_attach_hook(entidade: *mut c_void, contexto: u64) {
    despachar_evento_do_ciclo(2, entidade);
    let original: unsafe extern "C" fn(*mut c_void, u64) = std::mem::transmute(original_publicado(&ORIG_CICLO_ATTACH));
    original(entidade, contexto);
    despachar_evento_do_ciclo(3, entidade);
}

/// Gancho de `Detach` (`ALVOS_DO_CICLO_DE_VIDA[4]`).
pub(crate) unsafe extern "C" fn ciclo_detach_hook(entidade: *mut c_void) {
    despachar_evento_do_ciclo(4, entidade);
    let original: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(original_publicado(&ORIG_CICLO_DETACH));
    original(entidade);
}

/// Gancho de `Uninitialize` (`ALVOS_DO_CICLO_DE_VIDA[5]`).
pub(crate) unsafe extern "C" fn ciclo_uninitialize_hook(entidade: *mut c_void) {
    despachar_evento_do_ciclo(5, entidade);
    let original: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(original_publicado(&ORIG_CICLO_UNINITIALIZE));
    original(entidade);
}

/// Gancho de `Dispose` (`ALVOS_DO_CICLO_DE_VIDA[6]`).
pub(crate) unsafe extern "C" fn ciclo_dispose_hook(entidade: *mut c_void) {
    // O `Dispose` do Mac desinicializa em linha só com status de 2 a 5 e sem cena; aí o `Uninitialize` não passa pelo gancho dele.
    if !entidade.is_null() && evento_de_entidade_armado(5) && crate::memoria::is_readable(entidade as *const c_void, ENT_STATUS_OFF + 1) {
        let e = entidade as *const u8;
        let status = e.add(ENT_STATUS_OFF).read();
        let cena = (e.add(0xb8) as *const u64).read_unaligned();
        if (ENT_DETACHED..=ENT_DETACHING).contains(&status) && cena == 0 {
            despachar_evento_do_ciclo(5, entidade);
        }
    }
    let original: unsafe extern "C" fn(*mut c_void) = std::mem::transmute(original_publicado(&ORIG_CICLO_DISPOSE));
    original(entidade);
}

/// (índice em `ALVOS_DO_CICLO_DE_VIDA`, hook, original) dos 6 hooks do ciclo de vida.
pub(crate) fn hooks_do_ciclo() -> [(usize, *mut c_void, &'static AtomicU64); 6] {
    [
        (0, ciclo_initialize_hook as *mut c_void, &ORIG_CICLO_INITIALIZE),
        (1, ciclo_requestcomponents_hook as *mut c_void, &ORIG_CICLO_REQUESTCOMPONENTS),
        (3, ciclo_attach_hook as *mut c_void, &ORIG_CICLO_ATTACH),
        (4, ciclo_detach_hook as *mut c_void, &ORIG_CICLO_DETACH),
        (5, ciclo_uninitialize_hook as *mut c_void, &ORIG_CICLO_UNINITIALIZE),
        (6, ciclo_dispose_hook as *mut c_void, &ORIG_CICLO_DISPOSE),
    ]
}

/// Instala os hooks do ciclo de vida da entidade no `RegisterCallback` de um evento `Entity/*` — o mod e o teste 130b passam pelo
/// mesmo caminho —, de novo depois de uma desinstalação. Quem chama serializa ativação e desativação (`register::ativar_hooks_do_evento`). Só instala onde
/// a 1ª instrução é o prólogo: com o hook no ar ela é o salto, e não empilha. Recusa se as sondas do 130a estão ativas (dois hooks no mesmo endereço entram
/// em laço). O original publicado nunca volta a zero: uma thread que ainda está no hook depois da desinstalação chama o trampolim antigo, que continua
/// mapeado.
pub(crate) unsafe fn garantir_hooks_do_ciclo_de_vida() {
    // As sondas do 130a só existem no selftest.
    #[cfg(feature = "devtools")]
    let sondas_130a = [
        &ORIG_C130_INITIALIZE,
        &ORIG_C130_REQUESTCOMPONENTS,
        &ORIG_C130_REASSEMBLE,
        &ORIG_C130_ATTACH,
        &ORIG_C130_DETACH,
        &ORIG_C130_UNINITIALIZE,
        &ORIG_C130_DISPOSE,
    ]
    .iter()
    .any(|o| o.load(Ordering::Relaxed) != 0);
    #[cfg(not(feature = "devtools"))]
    let sondas_130a = false;
    let mut estado = Vec::new();
    for (k, destino, orig) in hooks_do_ciclo() {
        let alvo = &ALVOS_DO_CICLO_DE_VIDA[k];
        let end = crate::rebase(alvo.vm) as *mut c_void;
        if crate::ganchos::ganchos_no_endereco(end) > 0 {
            continue;
        }
        let lido = if crate::memoria::is_readable(end as *const c_void, 16) { (end as *const u32).read_unaligned() } else { 0 };
        let ok = !sondas_130a && lido == alvo.prologo && ganchos::instalar_com_threads_paradas(end, destino, Some(orig)).is_some();
        estado.push(format!("{} {}", alvo.nome, if ok { "ok" } else { "não" }));
    }
    gravar_hooks_do_ciclo_no_ar();
    if !estado.is_empty() {
        crate::veredito_dev!("[c130b] hooks do ciclo de vida pelo RegisterCallback: {} (sondas do 130a ativas: {sondas_130a})", estado.join(", "));
    }
}

/// O `Entity/Extract` sai do hook do job de componentes do builder, que a tabela de patches instala na carga só quando tem linha.
pub(crate) unsafe fn garantir_hook_do_extract() {
    install_job_do_builder_hook();
}

// `Entity::Assemble` (`0x1019aa048`, RE de 2026-09-13: chamado pela finalização do `EntityBuilder` com `x0` = entidade e `x1` = contexto com
// template, aparência e componentes). Na montagem a entidade ainda não tem id nem status (os 2 vêm no `Initialize`) e já tem os componentes.
// Teste 130c (`~/.bwms-hook-c130c-teste`): o handler de script anota a entidade do `Entity/Assemble` (parte 0) e a do `Entity/Initialize` (parte 1);
// o Rust confere na mesma thread o ponteiro, o id e o status zerados e os componentes, e que a entidade inicializada passou antes pela montagem.
pub(crate) const ENTITY_ASSEMBLE_VM: u64 = 0x1019aa048;

pub(crate) const ENTITY_ASSEMBLE_PROLOGO: u32 = 0xd10343ff;

pub(crate) static ORIG_ENTITY_ASSEMBLE: AtomicU64 = AtomicU64::new(0);

/// Gancho de `Entity::Assemble`.
pub(crate) unsafe extern "C" fn entity_assemble_hook(entidade: *mut c_void, contexto: *mut c_void) -> u64 {
    let original: unsafe extern "C" fn(*mut c_void, *mut c_void) -> u64 = std::mem::transmute(original_publicado(&ORIG_ENTITY_ASSEMBLE));
    let r = original(entidade, contexto);
    if entidade.is_null() || !ASSEMBLE_ARMADO.load(Ordering::Relaxed) || C130C_CONGELADO.load(Ordering::Relaxed) {
        return r;
    }
    #[cfg(not(feature = "devtools"))]
    disparar_evento_de_entidade("Entity/Assemble", entidade);
    #[cfg(feature = "devtools")]
    {
        if C130C_TESTE.load(Ordering::Relaxed) {
            if let Ok(mut m) = C130C_MONTADAS.lock() {
                if m.len() >= 4096 {
                    m.pop_front();
                }
                m.push_back(entidade as usize);
            }
        }
        let atual = C130C_ATUAL.with(|c| c.replace(entidade as usize));
        let anotacoes = C130C_ANOTOU.with(|c| c.replace(0));
        let enviados = disparar_evento_de_entidade("Entity/Assemble", entidade);
        let anotou = C130C_ANOTOU.with(|c| c.replace(anotacoes));
        C130C_ATUAL.with(|c| c.set(atual));
        if enviados > 0 {
            C130C_DESPACHOS.fetch_add(1, Ordering::Relaxed);
            if anotou == 0 {
                C130C_SEM_ANOTACAO.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    r
}

/// Instala o hook do `Entity::Assemble` no `RegisterCallback` de `Entity/Assemble`, de novo depois de uma desinstalação, só com o
/// prólogo no endereço. Quem chama serializa ativação e desativação.
pub(crate) unsafe fn garantir_hook_do_assemble() {
    let end = crate::rebase(ENTITY_ASSEMBLE_VM) as *mut c_void;
    if crate::ganchos::ganchos_no_endereco(end) > 0 {
        return;
    }
    let lido = if crate::memoria::is_readable(end as *const c_void, 16) { (end as *const u32).read_unaligned() } else { 0 };
    let ok = lido == ENTITY_ASSEMBLE_PROLOGO && ganchos::instalar_com_threads_paradas(end, entity_assemble_hook as *mut c_void, Some(&ORIG_ENTITY_ASSEMBLE)).is_some();
    #[cfg(feature = "devtools")]
    C130C_HOOK_OK.store(ok, Ordering::Relaxed);
    crate::veredito_dev!("[c130c] hook do Entity::Assemble pelo RegisterCallback: {} (prólogo {lido:#010x})", if ok { "instalado" } else { "não instalado" });
}


pub(crate) static ORIG_JOB_DO_BUILDER: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLA_NO_BUILDER_ATIVA: AtomicBool = AtomicBool::new(false);

pub(crate) static JOBS_DO_BUILDER_VISTOS: AtomicU64 = AtomicU64::new(0);

/// Controle da extração própria (51f): o 1º builder com resultados passa de `Pendente` a `EmCurso` e grava `Ok` ou `Falhou`. `Falhou` vale até o fim do
/// processo: o controle valida a réplica do extrator (params, flags, bitset e endereços fixos), que é do binário e não muda com recarga nem com save.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ControleDaExtracao {
    Pendente = 0,
    EmCurso = 1,
    Ok = 2,
    Falhou = 3,
}

impl ControleDaExtracao {
    /// Byte fora dos 4 estados conta como `Falhou`: sem controle conhecido, não há mescla.
    pub(crate) fn do_byte(v: u8) -> Self {
        match v {
            0 => Self::Pendente,
            1 => Self::EmCurso,
            2 => Self::Ok,
            _ => Self::Falhou,
        }
    }
}

pub(crate) static CONTROLE_DA_EXTRACAO: AtomicU8 = AtomicU8::new(ControleDaExtracao::Pendente as u8);

pub(crate) static MESCLAS_NO_BUILDER_APLICADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static MESCLAS_NO_BUILDER_RECUSADAS: AtomicU64 = AtomicU64::new(0);

/// Gancho do job de componentes do `EntityBuilder` (`params` = `[builder, _, bloco de contagem]`).
pub(crate) unsafe extern "C" fn job_do_builder_hook(params: *mut [u64; 3], grupo: *const c_void) {
    let patches = MESCLA_NO_BUILDER_ATIVA.load(Ordering::Relaxed) && PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed);
    let mut travado: Option<(u64, u64)> = None;
    let mut _tamanho = 0usize;
    if (patches || INCLUSAO_DE_PARTES_ATIVA.load(Ordering::Relaxed)) && !params.is_null() && crate::memoria::is_readable(params as *const c_void, 0x18) {
        let [builder, _, bloco] = params.read_unaligned();
        if builder != 0 && travar_fraco(bloco) {
            travado = Some((builder, bloco));
            incluir_partes_no_builder(builder);
            if patches {
                _tamanho = mesclar_no_builder_travado(builder);
                mesclar_patches_nas_aparencias(builder);
            }
        }
    }
    let extract = extract_antes_do_job(params);
    let original: JobDoBuilderFn = std::mem::transmute(original_publicado(&ORIG_JOB_DO_BUILDER));
    original(params, grupo);
    if let Some((builder, bloco)) = extract {
        extract_depois_do_job(builder, bloco);
    }
    if let Some((_builder, bloco)) = travado {
        #[cfg(feature = "devtools")]
        a51f_registrar(_builder, _tamanho);
        destravar_fraco(bloco);
    }
}


/// Depot do mundo (2026-07-24) — atalho pragmático (achado pela investigação paralela do mesmo
/// nome): em vez de forjar `DynamicEntitySystem`/`DynamicEntitySpec` (native-class-forge + RE de
/// endereço nativo desconhecido pro pipeline real de spawn), chama a API REAL VANILLA `CompanionSystem.SpawnSubcharacterOnPosition(recordID:
/// TweakDBID, pos: Vector3)` direto via `call_func` — a MESMA usada pelo próprio jogo pra spawnar
/// o Spiderbot (`cyberpunk/systems/subCharacterSystem.script`), acessada pelo MESMO caminho já
/// provado `give`/`sistema_de_script_ou_getter` usam (`GetGame`→getter estático `GameInstance.GetXxx(gi)`). Zero
/// classe nova, zero RE de endereço novo — satisfaz o `proof_needed` literal do gap ("spawna uma
/// entidade visível") sem tocar o marshalling de Variant.
///
/// `pos` é escrito como 3 floats consecutivos num slot de 16 bytes (`Arg::Raw`): a largura REAL da
/// struct `Vector3` (12 vs 16 bytes, não confirmada por RE) não importa pra correção — o motor lê
/// x/y/z pelo SEU PRÓPRIO descritor de tipo (`p_entries`/`ptype` do `SpawnSubcharacterOnPosition`
/// REAL, resolvido por `resolve_func`), não pelo nosso; os 4 bytes finais (usados só se a struct
/// real for 16B) ficam zerados, o mesmo que um `w`/padding implícito.
///
/// NÃO testado em boot nenhum ainda — comando de console novo (`spawnsub`), mesma categoria de
/// `cwprobe`: opt-in, nunca auto-disparado.
/// Posição do V (`GameObject.GetWorldPosition`, `Vector4`), a mesma leitura do spawn ao lado do V provado no boot 297 (`aparencia/do_record.rs`).
pub(crate) unsafe fn posicao_do_jogador(player: *mut c_void) -> Option<[f32; 3]> {
    if !rtti::sane(player) {
        return None;
    }
    let rf = rtti::resolve_in_class(rtti::class_of(player), "GetWorldPosition")?;
    let b = rtti::call_func(&rf, player, &[])?;
    let p = [0, 1, 2].map(|k| f32::from_le_bytes([b[k * 4], b[k * 4 + 1], b[k * 4 + 2], b[k * 4 + 3]]));
    p.iter().all(|c| c.is_finite()).then_some(p)
}

pub unsafe fn spawn_subcharacter(
    reg: &Registry,
    captured_player: *mut c_void,
    record_name: &str,
    pos: (f32, f32, f32),
) -> Option<[u8; 0x20]> {
    let owner = auth_or(reg, captured_player);
    let gi = game_instance(reg, owner)?;
    let cs = sistema_pelo_getter(reg, owner, gi, "GetCompanionSystem");
    if !rtti::sane(cs) {
        crate::log_dev!("[spawnsub] CompanionSystem inacessível ({cs:p})");
        return None;
    }
    // Diagnostico: CName hash da classe do objeto cs.
    let cs_class = rtti::class_of(cs);
    let cs_cn = if !cs_class.is_null() { unsafe { rtti::type_name_getname(cs_class) } } else { 0 };
    crate::log_dev!("[spawnsub] cs={cs:p} class_cname={cs_cn:#018x} ({:?})", crate::cname::resolve_cname(cs_cn));
    // Tenta a classe pelo nome redscript; fallback p/ nome c++ gameCompanionSystem.
    let spawn = rtti::resolve_func(reg, "CompanionSystem", "SpawnSubcharacterOnPosition")
        .or_else(|| rtti::resolve_func(reg, "gameCompanionSystem", "SpawnSubcharacterOnPosition"))
        .or_else(|| rtti::resolve_func(reg, "ICompanionSystem", "SpawnSubcharacterOnPosition"))?;
    let tdbid = bwms_primitives::tweak_db_id(record_name).to_le_bytes();
    let mut posb = [0u8; 16];
    posb[0..4].copy_from_slice(&pos.0.to_le_bytes());
    posb[4..8].copy_from_slice(&pos.1.to_le_bytes());
    posb[8..12].copy_from_slice(&pos.2.to_le_bytes());
    crate::log_dev!(
        "[spawnsub] CompanionSystem.SpawnSubcharacterOnPosition('{record_name}' tdbid={:#018x}, pos={pos:?}) cs={cs:p}",
        u64::from_le_bytes(tdbid)
    );
    let r = rtti::call_func(&spawn, cs, &[Arg::Tdb(tdbid), Arg::Raw(posb)]);
    crate::log_dev!("[spawnsub] call_func -> {:02x?}", r);
    r
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Bit de `ALVOS_DO_CICLO_DE_VIDA` do hook `nome`.
    fn bit(nome: &str) -> u8 {
        1 << ALVOS_DO_CICLO_DE_VIDA.iter().position(|a| a.nome == nome).expect("hook fora de ALVOS_DO_CICLO_DE_VIDA")
    }

    /// `ativ:A1`: o despacho do player pela presença entrega só o evento da transição (`Entity/Attach` presente, `Entity/Detach` ausente), e só com o
    /// hook do próprio evento fora do ar, com entidade e com ouvinte. Numa instalação parcial do grupo, sai o evento do hook que ficou fora.
    #[test]
    fn presenca_entrega_so_o_evento_do_hook_fora_do_ar() {
        // O ponteiro nunca é lido: a decisão só pergunta se é nulo.
        let player = std::ptr::NonNull::<c_void>::dangling().as_ptr();
        let ouve_tudo = |_: &str| true;
        let grupo = hooks_do_ciclo().iter().fold(0u8, |m, &(k, _, _)| m | 1 << k);
        assert_eq!(grupo, bit("Initialize") | bit("RequestComponents") | bit("Attach") | bit("Detach") | bit("Uninitialize") | bit("Dispose"));

        assert_eq!(evento_do_player(true, player, 0, ouve_tudo), Some("Entity/Attach"), "grupo fora: a entrada entrega o Entity/Attach");
        assert_eq!(evento_do_player(false, player, 0, ouve_tudo), Some("Entity/Detach"), "grupo fora: a saída entrega o Entity/Detach");
        assert_eq!(evento_do_player(true, player, grupo, ouve_tudo), None, "grupo no ar: o Entity/Attach do player já sai do hook");
        assert_eq!(evento_do_player(false, player, grupo, ouve_tudo), None, "grupo no ar: o Entity/Detach do player já sai do hook");

        let sem_attach = grupo & !bit("Attach");
        assert_eq!(evento_do_player(true, player, sem_attach, ouve_tudo), Some("Entity/Attach"), "só o Attach fora: a presença é a única entrega");
        assert_eq!(evento_do_player(false, player, sem_attach, ouve_tudo), None, "só o Attach fora: o Detach no ar já entrega");
        let sem_detach = grupo & !bit("Detach");
        assert_eq!(evento_do_player(true, player, sem_detach, ouve_tudo), None, "só o Detach fora: o Attach no ar já entrega");
        assert_eq!(evento_do_player(false, player, sem_detach, ouve_tudo), Some("Entity/Detach"), "só o Detach fora: a presença é a única entrega");

        assert_eq!(evento_do_player(true, player, 0, |ev| ev != "Entity/Attach"), None, "sem ouvinte do Entity/Attach");
        assert_eq!(evento_do_player(false, player, 0, |ev| ev != "Entity/Detach"), None, "sem ouvinte do Entity/Detach");
        assert_eq!(evento_do_player(true, player, 0, |ev| ev == "Entity/Attach"), Some("Entity/Attach"), "a entrada pergunta pelo ouvinte do Attach");
        assert_eq!(evento_do_player(false, player, 0, |ev| ev == "Entity/Detach"), Some("Entity/Detach"), "a saída pergunta pelo ouvinte do Detach");

        assert_eq!(evento_do_player(true, std::ptr::null_mut(), 0, ouve_tudo), None, "entrada sem entidade");
        assert_eq!(evento_do_player(false, std::ptr::null_mut(), 0, ouve_tudo), None, "saída sem o último ponteiro");
    }

    /// `ativ:A1`: a máscara sai da lista de hooks, um bit por hook do grupo com 1 ou mais hooks empilhados no endereço. Endereços falsos, acima de todo
    /// endereço de usuário do arm64 e longe do alvo do teste da API (`0x00FF_…`): entram na lista sem patch e não cruzam com os hooks de verdade dos
    /// testes de `ganchos`, que rodam em paralelo.
    #[test]
    fn mascara_no_ar_le_a_lista_de_hooks() {
        let falso = |vm: u64| (0x00FE_0000_0000_0000 | vm) as *mut c_void;
        let end = |nome: &str| falso(ALVOS_DO_CICLO_DE_VIDA.iter().find(|a| a.nome == nome).expect("hook fora de ALVOS_DO_CICLO_DE_VIDA").vm) as u64;
        let antes = mascara_no_ar(falso);
        // Todos menos o `Attach`, 2 no `Detach` e 1 no `Reassemble`, que não é do grupo.
        let empilhados = ["Initialize", "RequestComponents", "Detach", "Detach", "Uninitialize", "Dispose", "Reassemble"];
        for nome in empilhados {
            crate::ganchos::empilhar_sem_patch(end(nome), 1);
        }
        let parcial = mascara_no_ar(falso);
        for nome in empilhados {
            crate::ganchos::esquecer_sem_restaurar(end(nome));
        }
        let depois = mascara_no_ar(falso);

        assert_eq!(antes, 0, "sem hook na lista, nenhum bit");
        assert_eq!(parcial, bit("Initialize") | bit("RequestComponents") | bit("Detach") | bit("Uninitialize") | bit("Dispose"));
        assert_eq!(depois, 0, "a lista vazia de novo apaga todos os bits");
        let player = std::ptr::NonNull::<c_void>::dangling().as_ptr();
        assert_eq!(evento_do_player(true, player, parcial, |_| true), Some("Entity/Attach"));
        assert_eq!(evento_do_player(false, player, parcial, |_| true), None);
    }
}

/// Vetores `B5|statent|*` e `B5|entidades|evento_do_player` de `B5-vetores.txt`. Os estados são globais: cada teste usa ids próprios.
#[cfg(test)]
mod vetores_b5 {
    use super::*;
    use std::sync::atomic::AtomicU32;

    fn estado(rse: usize, token: [u64; 2], ent: u64, descartada: bool) -> EntidadeEstatica {
        EntidadeEstatica { rse, registry: 0, token, ent: [ent, 0], registrada: false, descartada }
    }

    fn com_tag(id: u64, tag: u64) {
        statent_tags().lock().expect("tags").entry(id).or_default().insert(tag);
        statent_by_tag().lock().expect("tags").entry(tag).or_default().insert(id);
    }

    /// `(estado existe, tags do id, id no índice da tag)`.
    fn rastro(id: u64, tag: u64) -> (bool, bool, Option<bool>) {
        (
            STATENT_ESTADOS.lock().expect("estados").contains_key(&id),
            statent_tags().lock().expect("tags").contains_key(&id),
            statent_by_tag().lock().expect("tags").get(&tag).map(|s| s.contains(&id)),
        )
    }

    /// Callback sem entidade, com falha ou com aborto: o estado e as tags saem e a referência do token que o estado guardava é solta (3 → 2).
    #[test]
    fn cb_invoke_sem_entidade_falha_aborto() {
        for (k, (ent, falhou, abortou)) in [(0u64, 0u8, 0u8), (0x1111, 1, 0), (0x1111, 0, 1)].into_iter().enumerate() {
            let id = 0xb5_7001 + k as u64;
            let mut token = [0u8; 0x30];
            token[0x10..0x18].copy_from_slice(&ent.to_le_bytes());
            token[0x21] = falhou;
            token[0x22] = abortou;
            let bloco = AtomicU32::new(3);
            let sp = [token.as_ptr() as u64, &bloco as *const AtomicU32 as u64];
            STATENT_ESTADOS.lock().expect("estados").insert(id, estado(0x99, sp, 0, false));
            com_tag(id, 0x81);
            let mut buffer = [0u8; 0x18];
            buffer[..8].copy_from_slice(&id.to_le_bytes());
            let mut invocado = sp;
            unsafe { statent_cb_invoke(buffer.as_mut_ptr(), &mut invocado) };
            assert_eq!(rastro(id, 0x81), (false, false, Some(false)));
            assert_eq!(bloco.load(Ordering::Relaxed), 2);
            assert_eq!(u64::from_le_bytes(token[0x10..0x18].try_into().expect("8 B")), ent, "entidade no token");
        }
        let mut buffer = [0u8; 0x18];
        buffer[..8].copy_from_slice(&0xb5_7fffu64.to_le_bytes());
        let token = [0u8; 0x30];
        unsafe {
            statent_cb_invoke(buffer.as_mut_ptr(), &mut [token.as_ptr() as u64, 0]);
            statent_cb_invoke(buffer.as_mut_ptr(), std::ptr::null_mut());
            statent_cb_invoke(buffer.as_mut_ptr(), &mut [0, 0]);
        }
    }

    /// `statent_status` e as recusas de `statent_ligar` pelo status (nenhum caso chega ao motor).
    #[test]
    fn status_e_ligar() {
        let mut ent = [0u8; 0x160];
        let id = 0xb5_7101u64;
        for s in [ENT_DETACHED, ENT_ATTACHING, ENT_ATTACHED, ENT_DETACHING] {
            ent[ENT_STATUS_OFF] = s;
            STATENT_ESTADOS.lock().expect("estados").insert(id, estado(153, [0; 2], ent.as_ptr() as u64, false));
            assert_eq!(statent_status(id), Some((153, [ent.as_ptr() as u64, 0], s)));
            match s {
                ENT_DETACHED | ENT_DETACHING => assert!(!unsafe { statent_ligar(id, false) }),
                _ => assert!(!unsafe { statent_ligar(id, true) }),
            }
        }
        STATENT_ESTADOS.lock().expect("estados").insert(id, estado(153, [0; 2], ent.as_ptr() as u64, true));
        assert_eq!(statent_status(id), None, "descartada");
        assert!(!unsafe { statent_ligar(id, true) } && !unsafe { statent_ligar(id, false) });
        STATENT_ESTADOS.lock().expect("estados").remove(&id);
        assert_eq!(statent_status(0xb5_71ff), None);
        assert!(!unsafe { statent_ligar(0xb5_71ff, true) });
    }

    #[test]
    fn despawnar() {
        let id = 0xb5_7201u64;
        let bloco = AtomicU32::new(2);
        STATENT_ESTADOS.lock().expect("estados").insert(id, estado(0x99, [0x7000, &bloco as *const AtomicU32 as u64], 0, false));
        com_tag(id, 0x82);
        assert!(unsafe { statent_despawnar(id) }, "sem entidade, com token");
        assert_eq!((rastro(id, 0x82), bloco.load(Ordering::Relaxed)), ((false, false, Some(false)), 1));

        STATENT_ESTADOS.lock().expect("estados").insert(id, estado(0x99, [0; 2], 0, false));
        assert!(!unsafe { statent_despawnar(id) }, "sem entidade, sem token");
        assert!(!STATENT_ESTADOS.lock().expect("estados").contains_key(&id));

        STATENT_ESTADOS.lock().expect("estados").insert(id, estado(0x99, [0; 2], 0x1234, true));
        assert!(!unsafe { statent_despawnar(id) }, "já descartada");
        let ficou = STATENT_ESTADOS.lock().expect("estados").get(&id).map(|e| (e.token, e.ent[0] != 0, e.registrada, e.descartada));
        assert_eq!(ficou, Some(([0, 0], true, false, true)));
        STATENT_ESTADOS.lock().expect("estados").remove(&id);
        assert!(!unsafe { statent_despawnar(0xb5_72ff) }, "id desconhecido");
    }

    /// Os 96 casos do vetor: presença × entidade nula × máscara (0x00, 0x08, 0x10, 0x18, 0x67, 0x7f) × ouvinte (nenhum, Attach, Detach, todos).
    #[test]
    fn evento_do_player_todos() {
        let player = std::ptr::NonNull::<c_void>::dangling().as_ptr();
        let mut saidas = Vec::new();
        for presente in [true, false] {
            for entidade in [std::ptr::null_mut(), player] {
                for no_ar in [0x00u8, 0x08, 0x10, 0x18, 0x67, 0x7f] {
                    for ouvinte in ["nenhum", "Attach", "Detach", "todos"] {
                        let ouve = |ev: &str| ouvinte == "todos" || ev.strip_prefix("Entity/") == Some(ouvinte);
                        let r = evento_do_player(presente, entidade, no_ar, ouve);
                        let esperado = match (presente, entidade.is_null(), ouvinte) {
                            (_, true, _) | (_, _, "nenhum") => None,
                            (true, false, "Attach" | "todos") if no_ar & 0x08 == 0 => Some("Entity/Attach"),
                            (false, false, "Detach" | "todos") if no_ar & 0x10 == 0 => Some("Entity/Detach"),
                            _ => None,
                        };
                        assert_eq!(r, esperado, "presente={presente} nula={} no_ar={no_ar:#04x} ouvinte={ouvinte}", entidade.is_null());
                        saidas.push(r);
                    }
                }
            }
        }
        assert_eq!(saidas.len(), 96);
        assert_eq!((saidas.iter().filter(|r| **r == Some("Entity/Attach")).count(), saidas.iter().filter(|r| **r == Some("Entity/Detach")).count()), (6, 6));
    }
}
