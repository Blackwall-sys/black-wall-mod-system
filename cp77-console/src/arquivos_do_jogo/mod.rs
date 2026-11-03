//! Arquivos do jogo: recursos (.ent, .mesh, .app), referências, tokens de carga, depot e existência de arquivo nos archives.

#![allow(dead_code)]

pub(crate) mod depot;

use crate::jogo::fila::com_fila_de_jobs;
use crate::jogo::buffers::realocar_rawbuffer;
use crate::jogo::objetos::travar_fraco;
use crate::jogo::aparencia::APPRESOURCE_APPEARANCES_OFF;
use crate::jogo::aparencia::APPDEF_NAME_OFF;
use crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF;
use crate::jogo::aparencia::APPDEF_PARTSVALUES_OFF;
use crate::jogo::aparencia::APPDEF_VISUALTAGS_OFF;
use crate::jogo::listas::dynarray_no_campo;
use crate::marcador_individual;
use crate::curvas::A51G_ATIVO;
use crate::curvas::A51G_CLASSE_CURVESET;
use bwms_primitives::resource_path_hash;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicPtr;
use crate::som::BANCOS_DE_SOM_REENVIO_PENDENTE;
use crate::jogo::recursos::resloader_this;
use crate::jogo::recursos::token_do_path;
use crate::texto_e_numeros::UTILS_DE_HASH_E_NOME;
use crate::roupas_e_armas::*;
use crate::entidades::*;
use crate::mundo::*;
use crate::ganchos::*;
use crate::diagnostico::{
    register_ativacao_118_utils, register_targets_utils, A17_PATCHES_INEXISTENTES, A51B_TESTE_ALVO, A51F2_TESTE, A51F2_TESTE_NONE, A51F3_TESTE,
    C122_CONTEXTOS, C122_DESPACHOS,
};
// Só os ramos de teste da tabela de patches usam, e o público não os cita.
#[cfg(feature = "devtools")]
use crate::diagnostico::{A51F_TESTE_ALVOS, A51G_TESTE, A51H_TESTE};
use crate::tipos_do_motor::*;
use crate::aparencia::*;
use crate::interface::*;
use crate::corpo_e_rosto::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b001;

// `ResourceHelper.GetReferencePath` (2026-08-11, cont.225): resolve a
// property por CName na classe do owner, confirma que é categoria `ERTTIType::ResourceReference`
// (11), lê `ResourceReference<>.path@+0x00` (u64, o mesmo hash FNV1a64 de `resource_path_hash`) —
// composição pura de `resolve_prop_in_class`+`type_kind`+`field_ptr`, todos já provados em outras
// natives desta sessão. Zero RE de endereço nativo. `0` = property não achada / não é
// ResourceReference / holder ainda não inicializado (todos os casos "sem dado").
//
// ESCOLHA CONSCIENTE 2026-08-12 (agente dedicado, rodada de composição barata, offline,
// NÃO é bug fix — a nota anterior já tinha testado isto ao vivo em 2026-08-11 e confirmado o
// `0` devolvido pra `entMeshComponent.mesh` com o aceite só da categoria 11): o smoke
// test canônico deste item (o smoke do resource helper) mira `entMeshComponent.mesh`,
// que é `RaRef<CMesh> mesh` — ou seja `ResourceAsyncReference<CMesh>` (`ERTTIType::
// ResourceAsyncReference = 12`, SEM `token`, só `path@+0x00`), não `ResourceReference` (`=11`,
// COM token). Aceitar TAMBÉM a categoria 12 é seguro
// porque `path` mora no MESMO offset (+0x00) nas duas variantes (`ResourceReference<>` e
// `ResourceAsyncReference<>`, ambos com `ResourcePath path` como 1º campo) — cobre o caso mais
// comum do jogo real (a maioria dos campos de recurso em componentes vanilla é `RaRef<T>`, não
// `Ref<T>`). Com isso,
// `entMeshComponent.mesh` deve passar a devolver o hash real (não mais `0`) — reteste recomendado
// no próximo boot. `IsReferenceLoaded`/`GetReferenceResource` (abaixo) SEGUEM exigindo
// estritamente `type_kind==11` — só ESSE layout tem `token` pra ler.
pub(crate) unsafe extern "C" fn tramp_resource_helper_get_reference_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let owner = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut path: u64 = 0;
    if rtti::sane(owner) && prop_hash != 0 {
        let prop_name = crate::cname::resolve_cname(prop_hash);
        if let Some((voff, ty, in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(owner), &prop_name) {
            // 11=ResourceReference(com token)/12=ResourceAsyncReference(só path) — path sempre em +0x00 nos dois.
            if matches!(rtti::type_kind(ty), Some(rtti::ertti::RESOURCE_REFERENCE) | Some(rtti::ertti::RESOURCE_ASYNC_REFERENCE)) {
                let vp = rtti::field_ptr(owner, voff, in_holder);
                if !vp.is_null() && crate::memoria::is_readable(vp as *const c_void, 8) {
                    path = (vp as *const u64).read_unaligned();
                }
            }
        }
    }
    write_uint_ret(out, path, 8);
}

/// `ResourceHelper.IsReferenceLoaded`/`GetReferenceResource`. O 3º
/// método, `LoadReferenceResource`, é `tramp_resource_helper_load_reference` (abaixo). Resolve a
/// property (mesmo mecanismo de `GetReferencePath`,
/// mas AQUI restrito a `type_kind==11`/`ResourceReference` — só esse layout tem o campo `token`
/// que os 2 métodos precisam), lê `ResourceReference<>.token@+0x08` (`SharedPtr<ResourceToken>`,
/// `{instance*@+0x00, refCount*@+0x08}`, 16 bytes —
/// mesmo layout de `Handle<T>` já usado em toda `make_handle`/`make_weak_handle` deste projeto).
/// Se `token.instance` for null (nunca carregado), `IsReferenceLoaded`→`false`/
/// `GetReferenceResource`→`ref` nulo.
///
/// `token->finished@+0x58`(i32)/`token->error@+0x5C`(u8) — offsets de `ResourceToken<T>`, ZERO
/// shift Itanium — são CAMPOS DE DADOS puros, não slots de vtable, mesma regra já estabelecida no projeto inteiro.
/// `token->resource@+0x28` (`Handle<T>`, 16 bytes) — `GetReferenceResource` lê o `.instance`
/// (1º qword) e reconstrói um handle FORTE de verdade via `rtti::make_handle` (a rotina real do
/// motor acha/reusa o bloco de refcount já existente do objeto, não duplica).
pub(crate) unsafe extern "C" fn tramp_resource_helper_is_reference_loaded(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // status 1 = finished && !error, o mesmo `resource_token_is_loaded`.
    write_bool_ret(out, token_da_property(frame).is_some_and(|t| status_do_token(t) == 1));
}

/// `BwmsLoadResourceReference(owner: ref<IScriptable>, propName: CName, pathHash: Uint64) -> Bool` —
/// `ResourceHelper.LoadReferenceResource` (2026-09-11): resolve a property `ResourceReference<T>` (type_kind 11)
/// pelo nome, grava o path e põe no token (`+0x08`) o `SharedPtr` que o loader devolve pelo caminho do motor
/// (`selftest::token_do_path`, `0x1010f0d34`, provado no boot 15), e espera o job numa fila
/// (`selftest::esperar_token`). Sem espera bloqueante, e o token que o campo tinha antes NÃO é solto — vaza 1 referência em vez de mexer no alocador do motor.
pub(crate) unsafe extern "C" fn tramp_resource_helper_load_reference(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let owner = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let path_hash = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    let mut motivo = "owner, property ou path inválido";
    if rtti::sane(owner) && prop_hash != 0 && path_hash != 0 {
        let prop_name = crate::cname::resolve_cname(prop_hash);
        match rtti::resolve_prop_in_class(rtti::class_of(owner), &prop_name) {
            Some((voff, ty, in_holder)) if rtti::type_kind(ty) == Some(rtti::ertti::RESOURCE_REFERENCE) => {
                let vp = rtti::field_ptr(owner, voff, in_holder);
                if vp.is_null() || !crate::memoria::is_readable(vp as *const c_void, 0x18) {
                    motivo = "campo ilegível";
                } else {
                    match crate::jogo::recursos::token_do_path(path_hash) {
                        Some([inst, ctrl]) if inst != 0 => {
                            // ResourceReference<>: path@+0x00, token SharedPtr {instância@+0x08, contagem@+0x10}.
                            let campo = vp as *mut u64;
                            let antigo = (campo.add(1).read_unaligned(), campo.add(2).read_unaligned());
                            campo.write_unaligned(path_hash);
                            campo.add(1).write_unaligned(inst);
                            campo.add(2).write_unaligned(ctrl);
                            crate::arquivos_do_jogo::esperar_token(inst);
                            ok = true;
                            motivo = "token gravado e job esperado na fila";
                            crate::log_dev!(
                                "[resourcehelper] token anterior do campo (não solto): {:#x}/{:#x} -> novo {inst:#x}/{ctrl:#x}",
                                antigo.0, antigo.1
                            );
                        }
                        Some(_) => motivo = "loader devolveu token nulo",
                        None => motivo = "sem loader",
                    }
                }
            }
            Some(_) => motivo = "property não é ResourceReference (type_kind != 11)",
            None => motivo = "property não achada",
        }
    }
    crate::log_dev!("[resourcehelper] BwmsLoadResourceReference(owner={owner:p}, path={path_hash:#x}) -> {ok} ({motivo})");
    write_bool_ret(out, ok);
}

/// `BwmsDepotRemoveResourceFromCache(path: Uint64) -> Bool` — `ResourceDepot.RemoveResourceFromCache`
/// (2026-09-11): `ResourceBank::ForgetResource` (`0x1010e4454`) no bank do loader, soltando o handle devolvido como o motor solta.
/// `true` = havia loader e a chamada foi feita; o log diz se o bank tinha o recurso.
pub(crate) unsafe extern "C" fn tramp_depot_remove_resource_from_cache(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let path = args.first().map(|(v, _)| *v).unwrap_or(0);
    let r = if path != 0 { crate::arquivos_do_jogo::esquecer_recurso(path) } else { None };
    crate::log_dev!("[depot] BwmsDepotRemoveResourceFromCache(path={path:#x}) -> tinha_recurso={r:?}");
    write_bool_ret(out, r.is_some());
}

/// Token guardado sob o id do 1º parâmetro (`#198` `ResourceDepot.LoadResource`, `#168` `LoadResourceToken`).
pub(crate) unsafe fn token_do_id(frame: *mut c_void) -> Option<u64> {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let id = args.first().map(|(v, _)| *v).unwrap_or(0);
    crate::arquivos_do_jogo::token_guardado(id)
}

/// Token da property `ResourceReference` (type_kind 11) de um objeto, lidos (owner, propName) do frame.
pub(crate) unsafe fn token_da_property(frame: *mut c_void) -> Option<u64> {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let owner = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    token_de(owner, prop_hash)
}

/// Espera o token terminar. A fila com `Wait` no job não bloqueia neste binário — no boot 22 voltou com o token ainda carregando —
/// então a espera confere o `finished` do próprio token a cada 1 ms, com teto de 2 s.
/// `true` = terminou (carregado ou falhou) dentro do teto.
pub(crate) unsafe fn esperar_terminar(t: u64) -> bool {
    crate::arquivos_do_jogo::esperar_token(t);
    let inicio = std::time::Instant::now();
    while status_do_token(t) == 0 {
        if inicio.elapsed() >= std::time::Duration::from_millis(2000) {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    true
}

/// `BwmsResourceTokenWatch(token, target, function, id, owner, propName) -> Bool` — `ResourceToken.RegisterCallback`
/// (2026-09-11): põe o token no vigia do tick (`selftest::vigiar_tokens`), que chama `target.function(token)` quando carrega.
/// `id != 0` = token guardado por id; senão, a property `propName` de `owner`. `false` = parâmetro inválido.
pub(crate) unsafe extern "C" fn tramp_resource_token_watch(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let token = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let target = args.get(1).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let funcao = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let id = args.get(3).map(|(v, _)| *v).unwrap_or(0);
    let owner = args.get(4).map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop = args.get(5).map(|(v, _)| *v).unwrap_or(0);
    let mut ok = false;
    if rtti::sane(token) && rtti::sane(target) && funcao != 0 && (id != 0 || (rtti::sane(owner) && prop != 0)) {
        let origem = if id != 0 {
            crate::arquivos_do_jogo::OrigemToken::Id(id)
        } else {
            let mut dono = [0u64; 2];
            rtti::make_handle(dono.as_mut_ptr() as *mut c_void, owner);
            crate::arquivos_do_jogo::OrigemToken::Property { dono, prop }
        };
        let mut token_script = [0u64; 2];
        rtti::make_handle(token_script.as_mut_ptr() as *mut c_void, token);
        ok = crate::arquivos_do_jogo::vigiar(crate::arquivos_do_jogo::Vigia {
            token_script,
            ouvinte: crate::arquivos_do_jogo::handle_fraco(target),
            funcao,
            origem,
        });
    }
    crate::log_dev!("[token-cb] BwmsResourceTokenWatch(target={target:p}, id={id}) -> {ok}");
    write_bool_ret(out, ok);
}

/// `BwmsDepotLoadResourceToken(path: Uint64) -> Uint64` — `ResourceDepot.LoadResource` (2026-09-11): carga
/// assíncrona pelo loader do motor (`0x1010f0d34`) sem esperar; devolve o id do token guardado (0 = sem loader).
pub(crate) unsafe extern "C" fn tramp_depot_load_resource_token(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let path = args.first().map(|(v, _)| *v).unwrap_or(0);
    let id = if path != 0 { crate::arquivos_do_jogo::carregar_e_guardar(path) } else { 0 };
    crate::log_dev!("[depot] BwmsDepotLoadResourceToken(path={path:#x}) -> id={id}");
    write_uint_ret(out, id, 8);
}

/// `BwmsResourceTokenIdStatus(id: Uint64) -> Int32` — 0 carregando, 1 carregado, 2 falhou, -1 id inválido.
pub(crate) unsafe extern "C" fn tramp_token_id_status(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_i32_ret(out, token_do_id(frame).map(|t| status_do_token(t)).unwrap_or(-1));
}

/// `BwmsResourceTokenIdHash(id: Uint64) -> Uint64` — o `ResourcePath` do token.
pub(crate) unsafe extern "C" fn tramp_token_id_hash(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, token_do_id(frame).map(|t| ((t + 0x48) as *const u64).read_unaligned()).unwrap_or(0), 8);
}

/// `BwmsResourceTokenIdResource(id: Uint64) -> ref<IScriptable>` — o recurso do token, handle pela rotina do motor.
pub(crate) unsafe extern "C" fn tramp_token_id_resource(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let inst = token_do_id(frame)
        .map(|t| ((t + 0x28) as *const u64).read_unaligned() as *mut c_void)
        .filter(|p| rtti::sane(*p))
        .unwrap_or(std::ptr::null_mut());
    entregar_recurso(out, inst);
}

/// `BwmsResourceTokenIdJob(id: Uint64) -> Uint64` — `ResourceToken.GetJobHandle`: o `JobHandle` do token, opaco.
pub(crate) unsafe extern "C" fn tramp_token_id_job(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, token_do_id(frame).map(|t| ((t + 0x50) as *const u64).read_unaligned()).unwrap_or(0), 8);
}

/// `BwmsResourceTokenIdWait(id: Uint64) -> Bool` — espera o token terminar (`esperar_terminar`, teto de 2 s).
pub(crate) unsafe extern "C" fn tramp_token_id_wait(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let terminou = token_do_id(frame).is_some_and(|t| esperar_terminar(t));
    write_bool_ret(out, terminou);
}

/// `BwmsGetResourceReferenceTokenJob(owner, propName) -> Uint64` — `GetJobHandle` do token de uma property (item `#18`).
pub(crate) unsafe extern "C" fn tramp_reference_token_job(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    write_uint_ret(out, token_da_property(frame).map(|t| ((t + 0x50) as *const u64).read_unaligned()).unwrap_or(0), 8);
}

/// `BwmsResourceReferenceTokenWait(owner, propName) -> Bool` — espera o token de uma property terminar (teto de 2 s).
pub(crate) unsafe extern "C" fn tramp_reference_token_wait(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let terminou = token_da_property(frame).is_some_and(|t| esperar_terminar(t));
    write_bool_ret(out, terminou);
}

pub(crate) unsafe extern "C" fn tramp_resource_helper_get_reference_resource(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    // ResourceToken<T>.resource@+0x28 (Handle<T>): instance@+0x00
    let inst = token_da_property(frame)
        .map(|t| ((t + 0x28) as *const u64).read_unaligned() as *mut c_void)
        .filter(|p| rtti::sane(*p))
        .unwrap_or(std::ptr::null_mut());
    entregar_recurso(out, inst);
}

/// Glue RTTI de `resource_token_status` — o token da property sai de `token_da_property`, e o status packed
/// (`status_do_token`) responde num Int32 só; 0 também quando a property não resolve.
pub(crate) unsafe extern "C" fn tramp_resource_helper_token_status(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let status: i32 = token_da_property(frame).map(|t| status_do_token(t)).unwrap_or(0);
    write_uint_ret(out, status as u64, 4);
}

/// Diagnóstico decisivo pro item `#18` (`ResourceToken`, achado 2026-08-15: `mesh`/`rig` deram
/// `loaded=true` E `failed=true` SIMULTANEAMENTE em `resource_token_status`, matematicamente
/// impossível pra uma leitura ESTÁVEL — só pode acontecer se chamadas diferentes de
/// `resolve_prop_in_class`+`type_kind` pro MESMO (owner,propName) devolverem resultados
/// DIFERENTES entre si, ou se o deref de `token_ptr` lê memória que muda entre as 3 chamadas
/// separadas de `IsFinished`/`IsLoaded`/`IsFailed`). Este native devolve o `type_kind` CRU
/// resolvido (0-15, ver `rtti::decode_ertti_type`) ou `-1` se a property não resolveu — chamado
/// MÚLTIPLAS vezes seguidas pro mesmo (owner,propName) no smoke test do resource token
/// pra confirmar/refutar a hipótese "resolução de tipo instável" vs. "campo
/// genuinamente não é tipo 11 (Ref), e a leitura de status é que lê lixo por outro motivo" —
/// nunca antes isolado, a nota do catálogo exaustivo (item `#18`) pedia
/// exatamente esta instrumentação como "próximo passo real" antes de testar outro alvo.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_resource_helper_debug_typekind(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let owner = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let prop_hash = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let mut kind: i32 = -1;
    if rtti::sane(owner) && prop_hash != 0 {
        let prop_name = crate::cname::resolve_cname(prop_hash);
        if let Some((_voff, ty, _in_holder)) = rtti::resolve_prop_in_class(rtti::class_of(owner), &prop_name) {
            kind = rtti::type_kind(ty).map(|k| k as i32).unwrap_or(-1);
        }
    }
    // sign-extend (Rust `as` de i32->u64 preserva o bit pattern de complemento-de-2, -1 vira
    // 0xFFFF...FFFF); só os 4 bytes baixos são escritos, mesmo idioma já usado em
    // `resource_token_status` acima — lido de volta como Int32 dá -1 corretamente.
    write_uint_ret(out, kind as u64, 4);
}

// ===== `GameFileExists` — **achado 2026-08-09: as utilidades de número e de texto já
// estavam 100% implementadas** (`register_number_utils`/`register_string_utils`, acima —
// primeira tentativa desta rodada duplicou esse trabalho por não ter checado antes; revertida).
// `GameFileExists` é o único item genuinamente novo desse lote: existência de arquivo
// com guarda de path-traversal, zero dependência de RTTI/endereço nativo.
pub(crate) unsafe extern "C" fn tramp_game_file_exists(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let rel = strings.first().cloned().flatten().unwrap_or_default();
    // Guarda de path-traversal (rejeita path que normalize pra fora do jogo).
    let exists = if rel.is_empty() || rel.contains("..") {
        false
    } else {
        crate::game_dir_from_dylib()
            .map(|root| std::path::Path::new(&root).join(&rel).exists())
            .unwrap_or(false)
    };
    write_bool_ret(out, exists);
}

pub(crate) unsafe fn register_compatibility_utils(reg: &Registry, proto: *mut c_void) {
    let ok1 = register_argful_by_types(reg, proto, &["String"], "BwmsGameFileExists", "BwmsGameFileExists", tramp_game_file_exists);
    crate::log_dev!("[reg] register_all: cw-utils Compatibility.GameFileExists={ok1}");
}

// ===== `ResourceDepot.ResourceExists` —
// ponta redscript-callable do mecanismo interno `crate::resource_exists` (PROVADO ao vivo
// 2026-08-10 via comando de canal `resexists`, composição de `PATHB_DEPOT`+
// `CheckResource@0x10151aa2c`). Mesmo padrão de trampolim String→Bool que `GameFileExists`
// já usa e já provou funcionar (`read_params_consuming_with_strings`+`write_bool_ret`).
pub(crate) unsafe extern "C" fn tramp_resource_exists(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let exists = if path.is_empty() { false } else { crate::resource_exists(&path) };
    write_bool_ret(out, exists);
}

/// `ResourceDepot.ArchiveExists` — ponta redscript-callable de
/// `crate::archive_exists_by_name` (layout de `ArchiveInfo`/RedString CONFIRMADO ao vivo 2026-08-10
/// via `archivenamedump`, refutando a incerteza de 2026-07-17). Mesmo padrão String→Bool.
pub(crate) unsafe extern "C" fn tramp_archive_exists(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let name = strings.first().cloned().flatten().unwrap_or_default();
    let exists = if name.is_empty() { false } else { crate::archive_exists_by_name(&name) };
    write_bool_ret(out, exists);
}

/// `ResourceDepot.LoadResource` — ponta redscript-callable da consulta ao
/// **cache de recursos vivos** do motor, identificado em 2026-08-21.
///
/// **Como o mapa foi identificado (por CONTEÚDO, não por forma):** `reslinkdump` capturou hashes
/// que o PRÓPRIO MOTOR gerou pra caminhos reais; `mapfind` procurou um deles
/// (`...atlas_common.inkatlas` = `0xcb91e1aeaf64dd2c`) dentro do `HashMap` no bank `[loader+0x48]`,
/// achou a chave no bucket 6855, e o RTTI nomeou o valor: **`inkTextureAtlas`** — o tipo exato de
/// um `.inkatlas`. Logo: `HashMap<ResourcePath, CResource*>`, ~55 mil entradas.
///
/// **Por que NÃO devolve `ref<>`:** esta mesma sessão provou que o crash de boot é despacho virtual
/// através de um `Handle<T>` cujo objeto tem **vtable nula** (frame 0 dentro da rotina de refcount
/// de `Handle<T>` em `0x101025e3c`). Entregar um `CResource*` cru pro redscript faria a camada de
/// script tomar refcount de um objeto que ela não possui — exatamente a classe de bug que mata
/// boot aqui. Então a ponta expõe os FATOS do recurso (está carregado? qual é o tipo?), que é o
/// que o mod de fato consome, sem deixar handle nenhum escapar.
pub(crate) unsafe extern "C" fn tramp_depot_resource_loaded(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let classe = depot_loaded_resource_class(&path);
    crate::log_dev!("[depot] IsResourceLoaded('{path}') = {} (classe='{classe}')", !classe.is_empty());
    write_bool_ret(out, !classe.is_empty());
}

/// Companheira da anterior: devolve o NOME DA CLASSE RTTI do recurso carregado
/// (ex.: `"inkTextureAtlas"` pra um `.inkatlas`), ou `""` se não estiver no cache. É o que
/// substitui, com segurança, o "me dê o recurso": o mod descobre o tipo real sem tocar no handle.
pub(crate) unsafe extern "C" fn tramp_depot_resource_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let nome = depot_loaded_resource_class(&path);
    crate::log_dev!("[depot] LoadedResourceClass('{path}') = '{nome}'");
    let _ = write_cstring_inline_ret(out, &nome);
}

// ===== `ResourceRef`/`ResourceAsyncRef` — 4 dos ~15 métodos, habilitados pelo
// cache de recursos identificado em 2026-09-04 (o mapa no bank `[loader+0x48]`).
//
// O item estava 2/~15 (`OperatorEqual`/`NotEqual` + `GetHash`). Estes 4 saem de graça agora:
//   `FromPath`  — é `resource_path_hash`, FNV-1a64 com sanitização, já provado FIEL ao motor
//                 (`bwms-primitives/examples/path_hashes.rs` bate com hashes que o jogo gerou).
//   `IsEmpty`   — `hash == 0`, definição da própria struct (é um u64 e nada mais).
//   `IsLoaded`  — consulta ao cache vivo: presente E com classe RTTI resolvível.
//   `GetResource` (parcial) — devolve a CLASSE do recurso carregado, não o handle. Mesma
//                 escolha do `LoadResource` acima: entregar `ref<>` faria o redscript tomar
//                 refcount de objeto que não possui, que é o mecanismo de crash já provado.
pub(crate) unsafe extern "C" fn tramp_resref_from_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let path = strings.first().cloned().flatten().unwrap_or_default();
    let h = if path.is_empty() { 0 } else { caminho_registrar(&path) };
    write_uint_ret(out, h, 8);
}

/// Calcula o hash do caminho e guarda o texto pro `BwmsResourcePathPart`.
pub(crate) fn caminho_registrar(path: &str) -> u64 {
    let h = bwms_primitives::resource_path_hash(path);
    if h != 0 {
        if let Ok(mut m) = CAMINHOS_POR_HASH.lock() {
            let mapa = m.get_or_insert_with(std::collections::HashMap::new);
            if mapa.len() < 65536 || mapa.contains_key(&h) {
                mapa.insert(h, path.to_string());
            }
        }
    }
    h
}

/// `BwmsResourcePathPart(hash, parte) -> String`: pedaço `parte` (19 bytes) do caminho registrado pra esse hash; vazio no fim ou se desconhecido.
pub(crate) unsafe extern "C" fn tramp_resource_path_part(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let h = args.first().map(|(v, _)| *v).unwrap_or(0);
    let parte = args.get(1).map(|(v, _)| *v as u32 as usize).unwrap_or(0).min(64);
    let caminho = caminho_texto(h).unwrap_or_default();
    write_cstring_chunk_ret(out, &caminho, parte);
}

pub(crate) unsafe extern "C" fn tramp_resref_is_empty(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let h = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_bool_ret(out, h == 0);
}

pub(crate) unsafe extern "C" fn tramp_resref_is_loaded(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let h = args.first().map(|(v, _)| *v).unwrap_or(0);
    write_bool_ret(out, h != 0 && !depot_loaded_resource_class_by_hash(h).is_empty());
}

pub(crate) unsafe extern "C" fn tramp_resref_resource_class(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let h = args.first().map(|(v, _)| *v).unwrap_or(0);
    let nome = if h == 0 { String::new() } else { depot_loaded_resource_class_by_hash(h) };
    let _ = write_cstring_inline_ret(out, &nome);
}

// ===== `ResourceTarget` — filtro de path por wildcard/regex.
//
// Lógica pura: não toca
// memória do jogo, não chama nada nativo — por isso é exatamente o tipo de gap que dá pra fechar
// sem depender de RE. O núcleo vive em `bwms_primitives::restarget` (motor em `bwms_primitives::regex`),
// testado offline; aqui é só a ponta redscript.
//
// 1 = casou, 0 = avaliado e não casou, -1 = `regex:` recusado pelo motor (fora do subconjunto,
// inválido ou grande demais). "Não sei" é diferente de "não casou".
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_restarget_matches(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let padrao = strings.first().cloned().flatten().unwrap_or_default();
    let path = strings.get(1).cloned().flatten().unwrap_or_default();
    let r = match bwms_primitives::restarget::casa(&padrao, &path) {
        Some(true) => 1i64,
        Some(false) => 0i64,
        None => -1i64,
    };
    write_uint_ret(out, r as u64, 4);
}

/// Devolve a regex montada pro padrão — auditável.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_restarget_regex(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let padrao = strings.first().cloned().flatten().unwrap_or_default();
    let r = bwms_primitives::restarget::wildcard_para_regex(&padrao);
    let _ = write_cstring_inline_ret(out, &r);
}

pub(crate) unsafe fn register_restarget_137_utils(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "String"], "BwmsResTargetMatches", "BwmsResTargetMatches", corpo_de_prova!(tramp_restarget_matches)));
    let b = registro_de_prova!(register_argful_by_types(reg, proto, &["String"], "BwmsResTargetRegex", "BwmsResTargetRegex", corpo_de_prova!(tramp_restarget_regex)));
    crate::log_dev!("[reg] register_all: ResTargetMatches={a} ResTargetRegex={b}");
}

pub(crate) unsafe fn register_resref_17_utils(reg: &Registry, proto: *mut c_void) {
    let a = register_argful_by_types(reg, proto, &["String"], "BwmsResRefFromPath", "BwmsResRefFromPath", tramp_resref_from_path as NativeHandler);
    let b = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResRefIsEmpty", "BwmsResRefIsEmpty", tramp_resref_is_empty as NativeHandler);
    let c = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResRefIsLoaded", "BwmsResRefIsLoaded", tramp_resref_is_loaded as NativeHandler);
    let d = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResRefResourceClass", "BwmsResRefResourceClass", tramp_resref_resource_class as NativeHandler);
    // `BwmsHashToResRef` já é registrado no bloco dos casts (item `#63`).
    let f17 = register_argful_by_types(reg, proto, &["Uint64", "Int32"], "BwmsResourcePathPart", "BwmsResourcePathPart", tramp_resource_path_part as NativeHandler);
    crate::log_dev!("[reg] register_all: cw-17 ResourcePathPart={f17}");
    crate::log_dev!("[reg] register_all: ResRef FromPath={a} IsEmpty={b} IsLoaded={c} ResourceClass={d}");
}

pub(crate) unsafe fn register_resourcedepot_utils(reg: &Registry, proto: *mut c_void) {
    let ok = register_argful_by_types(reg, proto, &["String"], "BwmsResourceExists", "BwmsResourceExists", tramp_resource_exists);
    crate::log_dev!("[reg] register_all: Depot.ResourceExists (BwmsResourceExists)={ok}");
    let ok2 = register_argful_by_types(reg, proto, &["String"], "BwmsArchiveExists", "BwmsArchiveExists", tramp_archive_exists);
    crate::log_dev!("[reg] register_all: Depot.ArchiveExists (BwmsArchiveExists)={ok2}");
    register_resref_17_utils(reg, proto);
    register_restarget_137_utils(reg, proto);
    register_targets_utils(reg, proto);
    register_textedit_115_utils(reg, proto);
    register_sector_86_utils(reg, proto);
    register_ativacao_118_utils(reg, proto);
    let ok3 = register_argful_by_types(reg, proto, &["String"], "BwmsDepotIsResourceLoaded", "BwmsDepotIsResourceLoaded", tramp_depot_resource_loaded);
    let ok4 = register_argful_by_types(reg, proto, &["String"], "BwmsDepotLoadedResourceClass", "BwmsDepotLoadedResourceClass", tramp_depot_resource_class);
    crate::log_dev!("[reg] register_all: Depot.LoadResource (BwmsDepotIsResourceLoaded={ok3} BwmsDepotLoadedResourceClass={ok4})");
}

// ---------------------------------------------------------------------------------------------
// `ProcessPostLoad` (round 9 — 2026-08-12, agente
// offline dedicado). Ver nota grande em `rtti::iserializable_process_post_load` pro raciocínio
// completo (offset 0x30 já confirmado 2x independentemente via `CMESH_POSTLOAD_VM`/
// `MORPHTGT_POSTLOAD_VM`). **Escopo deliberado**: declarar
// `public abstract native class ISerializable` cai na mesma categoria CMesh-trap já
// documentada à exaustão neste catálogo (`ISerializable` tem ZERO ocorrências em
// `redscript-src`, grep confirmado). Expus como `@addMethod(IScriptable)` em vez de
// `ISerializable` — `IScriptable` É um tipo vanilla real/seguro (`orphans.script:11256`,
// `public native class IScriptable`, já usado extensivamente por todo o projeto) e herda a
// MESMA posição de slot de vtable pra `PostLoad` (raiz da hierarquia RTTI, slots base nunca se
// movem por herança). `RefreshResource` (`this.ProcessPostLoad(...)`) é sugar puro.
pub(crate) unsafe extern "C" fn tramp_iscriptable_process_postload(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let disable_pre_init = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let ok = rtti::iserializable_process_post_load(inst, disable_pre_init);
    crate::log_dev!(
        "[iserializable] BwmsProcessPostLoad(obj={inst:p}, disablePreInit={disable_pre_init}) -> ok={ok}"
    );
    write_bool_ret(out, ok);
}

pub(crate) unsafe fn register_iscriptable_postload_native(reg: &Registry, proto: *mut c_void) {
    let ok = register_argful_by_types(
        reg,
        proto,
        &["handle:IScriptable", "Bool"],
        "BwmsProcessPostLoad",
        "BwmsProcessPostLoad",
        tramp_iscriptable_process_postload,
    );
    crate::log_dev!("[reg] register_all: cw-iserializable-postload (#58) BwmsProcessPostLoad={ok}");
}


/// Texto do caminho registrado pra `hash` (caminhos que passaram como texto pelo BWMS).
pub(crate) fn caminho_texto(hash: u64) -> Option<String> {
    CAMINHOS_POR_HASH.lock().ok().and_then(|m| m.as_ref().and_then(|mapa| mapa.get(&hash).cloned()))
}

// ===== 6 subclasses NOVAS `extends CallbackSystemTarget` (2026-07-24, rodada 2 — mesma sessão de
// `ComponentTarget` acima) =====================================================================
//
// `ResourceTarget`, `InkWidgetTarget`, `DynamicEntityTarget`, `StaticEntityTarget`, `EntityTarget` e
// `InputTarget`: as 6 `extends CallbackSystemTarget` DIRETO (nenhuma via parent
// intermediário; `inkWidgetTarget`/`EntityTarget`/`InputTarget` não passam por `ComponentTarget`
// nem entre si).
/// `ResourceTarget::Matches` (2026-08-12) — TAG de qual das 2 factories construiu
/// o alvo, mesmo motivo do `EntityTargetCriterion`/`ComponentTargetCriterion` (`Type`/`Path`
/// compartilhavam o mesmo slot `u64` cru sem distinguir a proveniência). O `u64` de `Type` é a classe que a
/// fábrica resolveu (`classe_do_alvo`); o de `Path`, o hash do `ResourcePath`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ResourceTargetCriterion {
    Type,
    Path,
}

pub(crate) static RESOURCETARGET_STATES: Mutex<Vec<(usize, ResourceTargetCriterion, u64)>> = Mutex::new(Vec::new());

/// `ResourceTarget.Type(resourceType: CName) -> ref<ResourceTarget>` — fábrica estática, 1 arg
/// CName (mesma receita de `tramp_ct_name`). Grava a classe resolvida aqui (`classe_do_alvo`), não o nome.
pub(crate) unsafe extern "C" fn tramp_rt_type(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let ty = args.first().map(|(v, _)| *v).unwrap_or(0);
    let classe = classe_do_alvo(ty, "ResourceTarget.Type");
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "ResourceTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix preventivo 2026-08-12: mesma retenção do #6/#134/#133/#132/#136
        if let Ok(mut states) = RESOURCETARGET_STATES.lock() {
            states.push((obj as usize, ResourceTargetCriterion::Type, classe));
        }
    } else {
        crate::log_dev!("[rt] ResourceTarget.Type: new_object('ResourceTarget') falhou");
    }
    crate::log_dev!(
        "[rt] ResourceTarget.Type('{}' hash {ty:#018x}) -> {obj:p} classe {classe:#018x} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))",
        crate::cname::resolve_cname(ty)
    );
    rtti::make_handle(out, obj);
}

/// `ResourceTarget.Path(resourcePath: ResRef) -> ref<ResourceTarget>` — 2026-07-24, 2ª leva: agora
/// que `"ResRef"` entrou no whitelist `escalar` de `rtti.rs`, o valor lido é o hash `ResourcePath`
/// (FNV-1a64) cru — MESMA representação canônica que `resource.link`/`bwms-primitives::
/// resource_path_hash` já usam em todo o projeto. Compartilha `RESOURCETARGET_STATES` com `Type`.
pub(crate) unsafe extern "C" fn tramp_rt_path(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let path_hash = args.first().map(|(v, _)| *v).unwrap_or(0);
    let mut obj: *mut c_void = std::ptr::null_mut();
    if let Some(reg) = rtti::Registry::obtain() {
        obj = rtti::new_object(&reg, "ResourceTarget");
    }
    if !obj.is_null() {
        retain_strong_ref_leaked(obj); // fix preventivo 2026-08-12: mesma retenção do #6/#134/#133/#132/#136
        if let Ok(mut states) = RESOURCETARGET_STATES.lock() {
            states.push((obj as usize, ResourceTargetCriterion::Path, path_hash));
        }
    } else {
        crate::log_dev!("[rt] ResourceTarget.Path: new_object('ResourceTarget') falhou");
    }
    crate::log_dev!("[rt] ResourceTarget.Path(hash={path_hash:#018x}) -> {obj:p} (fábrica estática, ResRef lido via escalar novo)");
    #[cfg(feature = "devlog")]
    if let Some(aviso) = rt_path_recusado(path_hash) { crate::log_veredito(&aviso); }
    rtti::make_handle(out, obj);
}

/// Forja `ResourceTarget extends CallbackSystemTarget` (mesma receita robusta de
/// `register_componenttarget`) + registra `Type`/`Path`.
pub unsafe fn register_resourcetarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_resourcetarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "ResourceTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_resourcetarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_resourcetarget: classe=OK mas sem protótipo — Type/Path NÃO registrados".into(),
    };
    let m1 = register_method(reg, "ResourceTarget", proto, "Type", "Type", tramp_rt_type, true);
    let m2 = register_method(reg, "ResourceTarget", proto, "Path", "Path", tramp_rt_path, true);
    format!(
        "[reg] ResourceTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) Type={m1} Path={m2} \
— is_static+construct+handle NUMA CLASSE FORJADA: PROVADO 2026-08-05"
    )
}

/// `ResourceEvent extends CallbackSystemEvent` — controller de recurso (2026-07-19). Forma:
/// `public native class ResourceEvent extends CallbackSystemEvent { GetResource()->ref<CResource>;
/// GetPath()->ResRef; GetJobGroup()->JobGroup }`. MESMA
/// receita 100% segura de `GameSessionEvent`/`KeyInputEvent` (registry Rust por-instância +
/// `register_type_instantiable_with_parent`, parent `CallbackSystemEvent` real).
///
/// **Divergência documentada:** `GetPath()` devolve `Uint64` — o hash FNV-1a64 do `ResourcePath`,
/// a MESMA representação canônica que `resource.link`/`reslinkdump`/`bwms-primitives::resource_path_
/// hash` usam em TODO o resto do projeto (nenhum resultado novo é "menos real": é como este
/// engine trata paths internamente) — em vez do tipo `ResRef` real
/// (marshalling de retorno String/ResRef é RE nova, fora de escopo desta rodada, mesma categoria
/// do gap residual documentado em `array:String` do TweakDB). `GetResource`/`GetJobGroup` fora
/// (não fazem parte do `proof_needed` literal, que só pede "path correto").
pub(crate) static RESOURCEEVENT_STATES: Mutex<Vec<(usize, u64)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_re_get_path(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u64;
    if let Ok(states) = RESOURCEEVENT_STATES.lock() {
        if let Some((_, p)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *p;
        }
    }
    crate::log_dev!("[re] ResourceEvent.GetPath() on {c:p} -> {v:#018x}");
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
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_resourceevent: classe=OK mas sem protótipo — GetPath NÃO registrado".into(),
    };
    let m1 = register_method(reg, "ResourceEvent", proto, "GetPath", "GetPath", tramp_re_get_path, false);
    // O `CResource` do evento despachado pelo hook do serializer.
    let m2 = register_method(reg, "ResourceEvent", proto, "GetResource", "GetResource", tramp_re_get_resource, false);
    format!("[reg] ResourceEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetPath={m1} GetResource={m2}")
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

/// `(instância do ResourceEvent, handle forte do CResource)` dos eventos despachados pelo hook do serializer.
pub(crate) static RESOURCEEVENT_RECURSOS: Mutex<Vec<(usize, [u64; 2])>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_re_get_resource(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let recurso = RESOURCEEVENT_RECURSOS.lock().ok().and_then(|r| r.iter().rev().find(|(p, _)| *p == c as usize).map(|(_, h)| *h)).unwrap_or([0, 0]);
    let mut h = recurso_para_script(recurso);
    if out.is_null() {
        crate::jogo::objetos::soltar_handle(&mut h);
        return;
    }
    core::ptr::write_unaligned(out as *mut u64, h[0]);
    core::ptr::write_unaligned((out as *mut u64).add(1), h[1]);
}

/// `Resource/Load` com o `CResource` de `recurso` (`{instância, bloco}` do array de serializables, +1 no forte durante o despacho) e o hash
/// do caminho (`+0x30`). Depois dos ouvintes, o evento e o recurso saem das tabelas e são soltos.
/// `Resource/Load` só com o caminho (o `watchres` do terminal): dispara, tira o estado e solta o handle do evento.
pub(crate) unsafe fn disparar_resource_load_do_caminho(caminho: u64) -> usize {
    let Some(arg) = make_resourceevent_arg(caminho) else {
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args("Resource/Load", &[arg]);
    if let Ok(mut s) = RESOURCEEVENT_STATES.lock() {
        s.retain(|(p, _)| *p as u64 != inst);
    }
    let mut he = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut he);
    enviados
}

pub unsafe fn disparar_resource_load(recurso: [u64; 2]) -> usize {
    let campo = (recurso[0] + 0x30) as *const u64;
    let caminho = if crate::memoria::is_readable(campo as *const c_void, 8) { campo.read_unaligned() } else { 0 };
    let Some(arg) = make_resourceevent_arg(caminho) else {
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    handle_strong_soma(&recurso, 1);
    if let Ok(mut r) = RESOURCEEVENT_RECURSOS.lock() {
        r.push((inst as usize, recurso));
    }
    let enviados = fire_event_args("Resource/Load", &[arg]);
    if let Ok(mut s) = RESOURCEEVENT_STATES.lock() {
        s.retain(|(p, _)| *p as u64 != inst);
    }
    let mut hr = RESOURCEEVENT_RECURSOS.lock().ok().and_then(|mut r| r.iter().position(|(p, _)| *p as u64 == inst).map(|i| r.remove(i).1)).unwrap_or([0, 0]);
    crate::jogo::objetos::soltar_handle(&mut hr);
    let mut he = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut he);
    enviados
}

/// `ResourceToken.IsLoaded` a partir dos campos do token do motor (`finished` i32 em `+0x58`, `error` u8 em `+0x5C`): terminou e sem erro.
pub(crate) fn resource_token_is_loaded(finished: i32, error: u8) -> bool {
    finished != 0 && error == 0
}

/// Token da property `ResourceReference` (type_kind 11) de um objeto — o mesmo caminho das natives do `#199`.
pub(crate) unsafe fn token_de(owner: *mut c_void, prop_hash: u64) -> Option<u64> {
    if !rtti::sane(owner) || prop_hash == 0 {
        return None;
    }
    let nome = crate::cname::resolve_cname(prop_hash);
    let (voff, ty, in_holder) = rtti::resolve_prop_in_class(rtti::class_of(owner), &nome)?;
    if rtti::type_kind(ty) != Some(rtti::ertti::RESOURCE_REFERENCE) {
        return None;
    }
    let vp = rtti::field_ptr(owner, voff, in_holder);
    if vp.is_null() || !crate::memoria::is_readable(vp as *const c_void, 0x10) {
        return None;
    }
    let t = (vp as *const u64).add(1).read_unaligned();
    (t != 0 && crate::memoria::is_readable(t as *const c_void, 0x60)).then_some(t)
}

/// Núcleo PURO do status combinado de `ResourceToken` (finalizado = carregado ou falhou,
/// carregado = `finished && !error`, falhou = `error`) — 0=ainda carregando (nunca finalizado), 1=carregado com sucesso,
/// 2=falhou (2026-08-12, round 6): a classe
/// redscript `ResourceToken` expõe `IsFinished`/`IsLoaded`/`IsFailed` como 3 métodos separados; o status packed
/// responde os 3 com 1 leitura do token (`register::status_do_token`, token por `token_de`).
pub(crate) fn resource_token_status(finished: i32, error: u8) -> i32 {
    if error != 0 {
        2
    } else if finished != 0 {
        1
    } else {
        0
    }
}

pub(crate) unsafe fn register_hash_wrapper_utils(reg: &Registry, proto: *mut c_void) {
    let mut registradas = 0usize;
    for (nome, tipos, handler) in UTILS_DE_HASH_E_NOME {
        if register_argful_by_types(reg, proto, tipos, nome, nome, *handler) {
            registradas += 1;
        }
    }
    // `CreateNodeRef` (item #63, rodada 20, 2026-08-15) — ver `tramp_string_to_noderef`.
    // Print/ModLog ficam com o nome que os mods chamam, de propósito (2026-08-07): usados internamente
    // em ~5 .reds do próprio BWMS (bwms-autocontinue/bwms-blindspot-tests/etc.) — renomear tem
    // risco real de quebra por pouco ganho (utilitário de debug genérico, não API distintiva).
    // `ResourceHelper.GetReferencePath` (2026-08-11) — composição pura de
    // primitivas já provadas, zero endereço nativo novo (ver `tramp_resource_helper_get_reference_path`).
    let ok14 = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsGetResourceReferencePath", "BwmsGetResourceReferencePath",
        tramp_resource_helper_get_reference_path,
    );
    // `ResourceHelper.IsReferenceLoaded`/`GetReferenceResource`, ver
    // `tramp_resource_helper_is_reference_loaded`/`tramp_resource_helper_get_reference_resource`.
    let ok25 = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsIsResourceReferenceLoaded", "BwmsIsResourceReferenceLoaded",
        tramp_resource_helper_is_reference_loaded,
    );
    let ok26 = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsGetResourceReferenceResource", "BwmsGetResourceReferenceResource",
        tramp_resource_helper_get_reference_resource,
    );
    // `ResourceHelper.LoadReferenceResource` (2026-09-11) — o loader do motor já carrega.
    let ok_load_ref = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName", "Uint64"],
        "BwmsLoadResourceReference", "BwmsLoadResourceReference",
        tramp_resource_helper_load_reference,
    );
    crate::log_dev!("[reg] register_all: cw-199 BwmsLoadResourceReference={ok_load_ref}");
    // `ResourceDepot.RemoveResourceFromCache` (2026-09-11) — `ResourceBank::ForgetResource` no bank do loader.
    let ok_forget = register_argful_by_types(
        reg, proto, &["Uint64"],
        "BwmsDepotRemoveResourceFromCache", "BwmsDepotRemoveResourceFromCache",
        tramp_depot_remove_resource_from_cache,
    );
    crate::log_dev!("[reg] register_all: cw-198 BwmsDepotRemoveResourceFromCache={ok_forget}");
    let ok_callarg = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName", "handle:IScriptable"],
        "BwmsCallMethodWithArg", "BwmsCallMethodWithArg", tramp_callmethod_with_arg,
    );
    crate::log_dev!("[reg] register_all: BwmsCallMethodWithArg={ok_callarg}");
    let ok_watch = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "handle:IScriptable", "CName", "Uint64", "handle:IScriptable", "CName"],
        "BwmsResourceTokenWatch", "BwmsResourceTokenWatch", tramp_resource_token_watch,
    );
    crate::log_dev!("[reg] register_all: BwmsResourceTokenWatch={ok_watch}");
    // Tokens por id (#198 `LoadResource`, #168 `LoadResourceToken`) e job/espera (#18 `GetJobHandle`) — 2026-09-11.
    let ok_tk1 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsDepotLoadResourceToken", "BwmsDepotLoadResourceToken", tramp_depot_load_resource_token as NativeHandler);
    let ok_tk2 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResourceTokenIdStatus", "BwmsResourceTokenIdStatus", tramp_token_id_status as NativeHandler);
    let ok_tk3 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResourceTokenIdHash", "BwmsResourceTokenIdHash", tramp_token_id_hash as NativeHandler);
    let ok_tk4 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResourceTokenIdResource", "BwmsResourceTokenIdResource", tramp_token_id_resource as NativeHandler);
    let ok_tk5 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResourceTokenIdJob", "BwmsResourceTokenIdJob", tramp_token_id_job as NativeHandler);
    let ok_tk6 = register_argful_by_types(reg, proto, &["Uint64"], "BwmsResourceTokenIdWait", "BwmsResourceTokenIdWait", tramp_token_id_wait as NativeHandler);
    let ok_tk7 = register_argful_by_types(reg, proto, &["handle:IScriptable", "CName"], "BwmsGetResourceReferenceTokenJob", "BwmsGetResourceReferenceTokenJob", tramp_reference_token_job as NativeHandler);
    let ok_tk8 = register_argful_by_types(reg, proto, &["handle:IScriptable", "CName"], "BwmsResourceReferenceTokenWait", "BwmsResourceReferenceTokenWait", tramp_reference_token_wait as NativeHandler);
    crate::log_dev!(
        "[reg] register_all: tokens-por-id LoadResourceToken={ok_tk1} Status={ok_tk2} Hash={ok_tk3} Resource={ok_tk4} Job={ok_tk5} Wait={ok_tk6} RefJob={ok_tk7} RefWait={ok_tk8}"
    );
    // `ResourceToken` (2026-08-12, round 6) — status packed
    // (0=carregando/1=carregado/2=falhou), ver `tramp_resource_helper_token_status`. Completa
    // `IsFinished`/`IsLoaded`/`IsFailed` do wrapper `ponte-resourcetoken.reds` com 1 native só.
    let ok29 = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsGetResourceReferenceTokenStatus", "BwmsGetResourceReferenceTokenStatus",
        tramp_resource_helper_token_status,
    );
    // Diagnóstico decisivo pro `#18` (2026-08-17): expõe `type_kind` cru resolvido por
    // (owner,propName), ver `tramp_resource_helper_debug_typekind` — chamado repetido no smoke
    // test pra confirmar/refutar resolução instável antes de testar outro alvo.
    registro_de_prova!(register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsResourceTokenDebugTypeKind", "BwmsResourceTokenDebugTypeKind", corpo_de_prova!(tramp_resource_helper_debug_typekind),
    ));
    // `EngineTime.FromTicks`/`ToTicks` (2026-08-12) — identidade pura, zero
    // endereço nativo (ver `tramp_identidade_8_bytes`).
    // `DistrictResolver.GetDistrict`/`GetArea` (2026-08-11) — dado puro, zero
    // endereço nativo (ver `crate::mundo::distrito`, 6/6 testes offline).
    // `GameObject.AddTag` (2026-08-11) — composição pura de RTTI já provado
    // (ver `tramp_gameobject_add_tag`).
    register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsGameObjectAddTag", "BwmsGameObjectAddTag",
        tramp_gameobject_add_tag,
    );
    // `TagList.RemoveTag`/`TagCount`/`Clear` (2026-08-11) — mesma composição
    // do AddTag, ver `tramp_gameobject_remove_tag`/`tramp_gameobject_tag_count`/
    // `tramp_gameobject_clear_tags`.
    register_argful_by_types(
        reg, proto, &["handle:IScriptable", "CName"],
        "BwmsGameObjectRemoveTag", "BwmsGameObjectRemoveTag",
        tramp_gameobject_remove_tag,
    );
    register_argful_by_types(
        reg, proto, &["handle:IScriptable"],
        "BwmsGameObjectTagCount", "BwmsGameObjectTagCount",
        tramp_gameobject_tag_count,
    );
    register_argful_by_types(
        reg, proto, &["handle:IScriptable"],
        "BwmsGameObjectClearTags", "BwmsGameObjectClearTags",
        tramp_gameobject_clear_tags,
    );
    // `Scripting.GetStackTrace` (2026-08-11) — getter indexado (composição
    // pura de offsets de campo já confirmados, ver `stack_frame_at`).
    // `WardrobeSystem.ForgetItemID` (2026-08-11, 3ª rodada) — ver
    // `tramp_wardrobe_forget_by_tdbid`/`selftest::wardrobe_forget_by_tdbid`.
    let ok24 = register_argful_by_types(
        reg, proto, &["handle:IScriptable", "Uint64"],
        "BwmsWardrobeForgetByTDBID", "BwmsWardrobeForgetByTDBID",
        tramp_wardrobe_forget_by_tdbid,
    );
    // `PersistentID.ToHash` (2026-08-14) — completa o algoritmo real de hash
    // (`PersistentID::GetHash()`, ver `persistent_id_hash` acima) pro caso
    // COMPONENT, nunca replicado até hoje.
    crate::log_dev!(
        "[reg] register_all: cw-utils {registradas} de {} pela tabela; ResourceReferencePath={ok14} Wardrobe={ok24} Token={ok25}/{ok26} Persistent={ok29}",
        UTILS_DE_HASH_E_NOME.len()
    );
}

/// Núcleo compartilhado das 2 pontas: devolve o nome da classe RTTI do recurso carregado, ou `""`.
///
/// **Por que a existência é definida pela classe, e não pela presença no mapa** (achado AO VIVO,
/// boot de 2026-08-21): em t=6s o alvo já estava no cache mas o RTTI ainda não resolvia
/// (`carregado=true classe=''`) — o `CResource` entra no mapa ANTES de a vtable ser escrita, e só
/// vira `inkTextureAtlas` depois. Se `IsResourceLoaded` respondesse "está no mapa", devolveria
/// `true` pra um objeto meio-construído e um mod agiria em cima dele — exatamente o mecanismo que
/// esta mesma sessão provou ser a causa do crash de boot (despacho virtual com vtable nula). Então
/// as duas pontas compartilham UM significado: existe = presente **e** utilizável.
pub(crate) unsafe fn depot_loaded_resource_class(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }
    let res = depot_loaded_resource(bwms_primitives::resource_path_hash(path));
    if res.is_null() || !crate::memoria::is_readable(res, 0x40) {
        return String::new();
    }
    let cls = rtti::class_of(res);
    if cls.is_null() {
        return String::new();
    }
    crate::cname::resolve_cname(rtti::type_name_getname(cls))
}

// Registro hash → caminho de recurso (2026-09-12). O `ResourcePath` guarda só o
// hash (FNV-1a64 do caminho normalizado), então só volta a texto caminho que passou como texto por uma native do BWMS; hash desconhecido fica vazio.
// Teto de 65536 caminhos.
// BACKLOG (F1-28): gravar também todo `ResourcePath::Create` do motor. Aqui o gancho do
// motor (`HashSanitized` 0x10106c8c4) só existe no selftest 48c-1, com marcador e registro próprio (`A48C_DINAMICOS`). Por isso um `ResourceTarget` com
// `*` ou `regex:` só casa `Resource/Load` de caminho que passou pelo `BwmsResRefFromPath`, e o texto de caminho dinâmico do `es20_resolver_caminho` só
// existe com esse marcador (no build público o stub devolve None). Para levar o gancho ao produto falta medir hashes únicos e memória deste mapa por
// boot e o custo do gancho (497010 chamadas até o veredito do boot 156, prova de 2026-09-13 do registro de caminhos), e
// provar que um `ResRef` literal de script passa pelo gancho.
pub(crate) static CAMINHOS_POR_HASH: std::sync::Mutex<Option<std::collections::HashMap<u64, String>>> = std::sync::Mutex::new(None);


/// No `RegisterCallback` de `Resource/Load`, liga o despacho e instala o hook do `OnDependenciesReady` com as outras threads paradas,
/// quando o boot não o instalou. Quem chama serializa ativação e desativação.
pub(crate) unsafe fn garantir_hooks_de_recurso() {
    if CLASSE_CRESOURCE.load(Ordering::Relaxed).is_null() {
        if let Some(reg) = crate::rtti::Registry::obtain() {
            CLASSE_CRESOURCE.store(reg.class_by_name("CResource"), Ordering::Relaxed);
        }
    }
    RESOURCE_LOAD_ATIVO.store(true, Ordering::Relaxed);
    // A leitura dos soundbanks já passou quando um script registra: o ouvinte novo recebe o reenvio no próximo tick.
    BANCOS_DE_SOM_REENVIO_PENDENTE.store(true, Ordering::Relaxed);
    let ok = garantir_gancho_de_dependencias();
    crate::veredito_dev!(
        "[c122] hook do OnDependenciesReady pelo RegisterCallback de Resource/Load: {ok} (CResource {:p})",
        CLASSE_CRESOURCE.load(Ordering::Relaxed)
    );
}

/// `Wait` do job do token (`+0x50`) numa fila do motor — o que os chamadores do motor fazem com ele. Sem espera bloqueante.
pub(crate) unsafe fn esperar_token(obj: u64) {
    let wait: extern "C" fn(*mut c_void, *const c_void) = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_ESPERAR_VM));
    com_fila_de_jobs(&mut |fila| wait(fila, (obj as usize + 0x50) as *const c_void));
}

/// `ResourceBank::ForgetResource` (`RemoveResourceFromCache`, 2026-09-11): `x0` = bank (`[loader+0x48]`, lido assim pelo
/// `0x1010e710c`), `x1` = path, `x8` = sret `Handle`. Com os locks do próprio
/// bank (`+0x82` cache de jogo, `+0x80` mapa), tira o recurso do cache de jogo e a entrada do mapa, devolvendo um handle forte.
/// Achado na TU `resourceBank.cpp`; os 2 chamadores (`0x1010e8374`, `0x1010e8ecc`) soltam o handle devolvido como `soltar_handle`.
pub(crate) const RESBANK_FORGET_VM: u64 = 0x1_010e_4454;

pub(crate) const LOADER_BANK_OFF: u64 = 0x48;

/// `RemoveResourceFromCache(path)`: `Some(true)` se o bank tinha o recurso (o handle devolvido é solto na hora), `Some(false)` se
/// não tinha, `None` sem loader ou bank legível.
pub(crate) unsafe fn esquecer_recurso(path_hash: u64) -> Option<bool> {
    let loader = resloader_this();
    if loader == 0 {
        return None;
    }
    let slot = (loader + LOADER_BANK_OFF) as *const u64;
    if !crate::memoria::is_readable(slot as *const c_void, 8) {
        return None;
    }
    let bank = slot.read_unaligned();
    if bank == 0 || !crate::memoria::is_readable(bank as *const c_void, 0x88) {
        return None;
    }
    let mut recurso = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(RESBANK_FORGET_VM) as u64,
        in("x0") bank,
        in("x1") path_hash,
        in("x8") recurso.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    let tinha = recurso[0] != 0;
    crate::jogo::objetos::soltar_handle(&mut recurso);
    Some(tinha)
}

/// Tokens de carga guardados pelo BWMS pra `ResourceToken` sem property dona (`ResourceDepot.LoadResource`,
/// `LoadResourceToken`). O `SharedPtr` devolvido pelo loader fica aqui com a referência forte dele; o id é o índice + 1. Nunca é
/// solto — a mesma escolha do `LoadReferenceResource` (o redscript não avisa o BWMS quando o wrapper morre).
pub(crate) static TOKENS_GUARDADOS: std::sync::Mutex<Vec<[u64; 2]>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn guardar_token(token: [u64; 2]) -> u64 {
    match TOKENS_GUARDADOS.lock() {
        Ok(mut v) => {
            v.push(token);
            v.len() as u64
        }
        Err(_) => 0,
    }
}

/// Ponteiro do `ResourceToken` guardado sob `id`, se legível (o objeto tem 0x60 bytes).
pub(crate) unsafe fn token_guardado(id: u64) -> Option<u64> {
    let inst = TOKENS_GUARDADOS.lock().ok()?.get((id as usize).checked_sub(1)?)?[0];
    (inst != 0 && crate::memoria::is_readable(inst as *const c_void, 0x60)).then_some(inst)
}

/// Carga assíncrona por path, guardada: id do token (0 sem loader ou token nulo). Não espera.
pub(crate) unsafe fn carregar_e_guardar(path_hash: u64) -> u64 {
    match token_do_path(path_hash) {
        Some(t) if t[0] != 0 => guardar_token(t),
        _ => 0,
    }
}

/// De onde o vigia relê o token a cada tick: guardado por id, ou a property `ResourceReference` de um objeto (seguro por handle forte).
pub(crate) enum OrigemToken {
    Id(u64),
    Property { dono: [u64; 2], prop: u64 },
}

/// `ResourceToken.RegisterCallback`: `ouvinte.funcao(token)` quando o token carrega. `vigiar_tokens` confere a cada tick de
/// gameplay. O `ResourceToken` do script fica seguro
/// por handle forte até avisar; o ouvinte, por handle fraco — ouvinte morto não é chamado.
pub(crate) struct Vigia {
    pub token_script: [u64; 2],
    pub ouvinte: [u64; 2],
    pub funcao: u64,
    pub origem: OrigemToken,
}

pub(crate) static VIGIAS: std::sync::Mutex<Vec<Vigia>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn vigiar(v: Vigia) -> bool {
    VIGIAS.lock().map(|mut l| l.push(v)).is_ok()
}

/// Handle fraco sem sobra na contagem forte: o construtor do motor soma 1 no forte, o fraco ganha 1 e o forte devolve o 1 (quem
/// chama ainda segura o objeto, então o forte não chega a zero aqui). O `rtti::make_weak_handle` não devolve esse 1.
pub(crate) unsafe fn handle_fraco(obj: *mut c_void) -> [u64; 2] {
    let mut h = [0u64; 2];
    if obj.is_null() {
        return h;
    }
    crate::rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj);
    if h[1] != 0 {
        let bloco = h[1] as *const std::sync::atomic::AtomicU32;
        (*bloco.add(1)).fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        (*bloco).fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    h
}

pub(crate) unsafe fn status_da_origem(o: &OrigemToken) -> i32 {
    let t = match o {
        OrigemToken::Id(id) => token_guardado(*id),
        OrigemToken::Property { dono, prop } => crate::arquivos_do_jogo::token_de(dono[0] as *mut c_void, *prop),
    };
    t.map(|t| crate::register::status_do_token(t)).unwrap_or(-1)
}

/// A cada tick de gameplay (depois do gate de player do `cp77_tick`): avisa os ouvintes dos tokens que carregaram e solta os que
/// falharam, sumiram ou perderam o ouvinte. Sem vigia registrado não chama nada — o `call_func` só roda quando um token terminou.
/// O token vai pro ouvinte com um +1 próprio que não é solto depois da chamada: se o frame chamado soltar o parâmetro, a conta
/// fecha; se não soltar, sobra 1 referência. Nunca solta a mais.
pub(crate) unsafe fn vigiar_tokens() {
    let prontos: Vec<(Vigia, i32)> = {
        let Ok(mut l) = VIGIAS.try_lock() else { return };
        if l.is_empty() {
            return;
        }
        let mut prontos = Vec::new();
        let mut i = 0;
        while i < l.len() {
            let status = status_da_origem(&l[i].origem);
            if status != 0 {
                prontos.push((l.swap_remove(i), status));
            } else {
                i += 1;
            }
        }
        prontos
    };
    for (mut v, status) in prontos {
        let vivo = crate::jogo::objetos::fraco_vivo(v.ouvinte[1]);
        let mut chamou = false;
        if status == 1 && vivo {
            let alvo = v.ouvinte[0] as *mut c_void;
            let nome = crate::cname::resolve_cname(v.funcao);
            if let Some(rf) = crate::rtti::resolve_in_class(crate::rtti::class_of(alvo), &nome) {
                let mut arg = [0u64; 2];
                crate::rtti::make_handle(arg.as_mut_ptr() as *mut c_void, v.token_script[0] as *mut c_void);
                chamou = crate::rtti::call_func(&rf, alvo, &[crate::rtti::Arg::Handle(arg[0] as *mut c_void, arg[1] as *mut c_void)]).is_some();
            }
        }
        crate::veredito_dev!("[token-cb] status={status} ouvinte_vivo={vivo} chamou={chamou}");
        if v.ouvinte[1] != 0 {
            crate::jogo::objetos::soltar_fraco(&mut v.ouvinte);
        }
        crate::jogo::objetos::soltar_handle(&mut v.token_script);
        if let OrigemToken::Property { mut dono, .. } = v.origem {
            crate::jogo::objetos::soltar_handle(&mut dono);
        }
    }
}


// Injeção de archive arbitrário (rota B): injeta um .archive fora do glob no content-group via um `replace`
// no InitializeArchives (0x10151a118). A replacement chama a original (constrói TODOS os archives do
// boot) e DEPOIS injeta — na THREAD DO JOGO, com o depot REAL (x0 do InitializeArchives, ≠ o singleton
// [0x109d23188+0x1f8] que tem +0x10=NULL vivo — mesmo erro do cont.78), segurando o lock@depot+0x78,
// pré-streaming. Os offsets da RE (grupos@depot+0x10 stride 0x38, key@g+0x30, count@g+0xc, lock@+0x78)
// são CORRETOS pro `this` real. Ver HISTORICO cont.80/81, notes/RE-archiveinfo-inject-2026-07-17.md.
pub(crate) static INIT_ARCH_ORIG: AtomicU64 = AtomicU64::new(0);

pub(crate) static PATHB_HOOK_ON: AtomicBool = AtomicBool::new(false);

/// `RegisterArchive`/`RegisterDir` da fachada de recursos (`PENDENCIAS-UNIFICADAS.md`, achado já
/// desde 2026-07-13 nunca implementado): o `depot` REAL capturado 1x por `init_archives_replacement`
/// (sempre instalado agora, não mais só sob `~/.bwms-pathbtest`) fica cacheado aqui pra qualquer
/// chamada FUTURA de `RegisterArchive` (redscript, tempo de execução do mod) reusar sem precisar
/// re-hookar nada — `InitializeArchives` só roda 1x no boot inteiro.
pub(crate) static PATHB_DEPOT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Replacement do InitializeArchives (0x10151a118): `u64(depot@x0)`. Chama a original (constrói TODOS
/// os archives do boot no depot REAL), guarda o depot e confere quais `.archive` da pasta `Mods/` o jogo
/// montou da pasta nativa — na THREAD DO JOGO, com o depot construído, ANTES do streaming do mundo. Uma vez.
pub(crate) unsafe extern "C" fn init_archives_replacement(depot: *mut c_void) -> u64 {
    let orig = INIT_ARCH_ORIG.load(Ordering::Relaxed);
    let f: unsafe extern "C" fn(*mut c_void) -> u64 = core::mem::transmute(orig);
    let ret = f(depot); // constrói os ~N archives do boot no depot
    if !depot.is_null() && crate::memoria::is_readable(depot as *const c_void, 0x80) {
        // Cacheia SEMPRE — base de `facade_register_archive`/`facade_register_dir`, chamáveis a
        // qualquer momento depois (InitializeArchives só roda esta 1x por boot).
        PATHB_DEPOT.store(depot, Ordering::Relaxed);
        // Com o depot construído: o que o jogo montou da pasta nativa e o que ficou fora vão para o relatório da `Mods/`.
        crate::mods_do_usuario::pasta_mods::conferir_montagem(depot as *mut u8);
    }
    ret
}

/// Instala o `replace` no InitializeArchives (uma vez, cedo — do on_load/thread do jogo). Ver
/// `init_archives_replacement`. **Sempre instalado agora** (não mais só sob `~/.bwms-pathbtest`) —
/// vira base de uma capacidade shipada (`RegisterArchive`), não mais só probe de dev.
/// Risco: baixo — a replacement SEMPRE chama a original primeiro (constrói os archives do boot
/// normalmente); o único efeito extra incondicional é guardar 1 ponteiro num `AtomicPtr`.
pub(crate) unsafe fn install_pathb_capture() {
    if PATHB_HOOK_ON.swap(true, Ordering::Relaxed) {
        return;
    }
    let addr = crate::rebase(0x1_0151_a118);
    if !crate::memoria::is_readable(addr as *const c_void, 4) {
        return;
    }
    match crate::ganchos::instalar_com_threads_paradas(addr, init_archives_replacement as *mut c_void, Some(&INIT_ARCH_ORIG)) {
        Some(_) => {
            crate::log_dev!("[pathb] replace no InitializeArchives (0x10151a118) instalado — injeta pós-build, thread do jogo...");
        }
        None => crate::log_dev!("[pathb] replace() do InitializeArchives RECUSOU — pathb não pode injetar"),
    }
}

/// Acha o grupo REAL de content do depot: o de MAIOR count, HEAP (não sentinela estático de
/// imagem). Extraído da injeção de archive arbitrário (rota B) (achado 2026-07-17: key==3 fica vazio,
/// arr sentinela; os archives do boot vão pra key=1/2, arr HEAP — injetar no de maior count).
/// `groups@depot+0x10` inline {ptr@+0x10,cap@+0x18,count@+0x1c}, stride 0x38; count@g+0x0c.
pub(crate) unsafe fn find_pathb_content_group(depot: *mut u8) -> Option<*mut u8> {
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        crate::log_dev!("[pathb] grupos inválidos (ptr={groups:p} n={gcount}) — abortado");
        return None;
    }
    let mut g: *mut u8 = core::ptr::null_mut();
    let mut best = 0u32;
    for i in 0..gcount as usize {
        let gi = groups.add(i * 0x38);
        if !crate::memoria::is_readable(gi as *const c_void, 0x38) {
            continue;
        }
        let cnt = (gi.add(0x0c) as *const u32).read();
        let arr = (gi as *const *const u8).read();
        // exige arr em HEAP (não a faixa de imagem 0x1_0000_0000..0x1_1000_0000 = sentinela estático).
        let arr_heap = (arr as usize) > 0x1_1000_0000;
        if cnt > best && cnt < 100_000 && arr_heap {
            best = cnt;
            g = gi;
        }
    }
    if g.is_null() || best == 0 {
        crate::log_dev!("[pathb] nenhum grupo heap com archives entre {gcount} — abortado");
        return None;
    }
    Some(g)
}

/// `ResourceDepot.ArchiveExists`: varre `depot->groups` e compara o nome de arquivo de `archive.path`
/// contra o nome pedido. Layout de `ArchiveInfo` (name RedString@+0x10, stride 0x50) CONFIRMADO
/// ao vivo em 2026-08-10 (`archivenamedump`, refutando a incerteza registrada em
/// `notes/RE-archiveinfo-inject-2026-07-17.md` sobre a RE estática original) — paths reais lidos
/// corretos (`basegame_1_engine.archive` etc.). **Escolha documentada**: em vez de filtrar por
/// `scope==Mod` (campo de scope do GRUPO ainda não mapeado com certeza — `+0x30` do grupo é
/// "key" 3=content/1=memoryresident por uma nota antiga, não confirmadamente "Mod"), varre TODOS
/// os grupos — superset seguro (nunca dá falso-negativo pra archive de mod real; risco de
/// falso-positivo só em colisão de nome de arquivo entre mod e vanilla, praticamente nulo).
pub unsafe fn archive_exists_by_name(name: &str) -> bool {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed) as *mut u8;
    if depot.is_null() {
        crate::log_dev!("[archiveexists] depot ainda não capturado (InitializeArchives não rodou) — abortado");
        return false;
    }
    let groups = (depot.add(0x10) as *const *mut u8).read();
    let gcount = (depot.add(0x1c) as *const u32).read();
    if groups.is_null() || gcount == 0 || gcount > 64 {
        return false;
    }
    for gi_idx in 0..gcount as usize {
        let gi = groups.add(gi_idx * 0x38);
        if !crate::memoria::is_readable(gi as *const c_void, 0x38) {
            continue;
        }
        let cnt = (gi.add(0x0c) as *const u32).read();
        let arr = (gi as *const *const u8).read();
        if arr.is_null() || cnt == 0 || cnt > 100_000 {
            continue;
        }
        for i in 0..cnt as usize {
            let entry = arr.add(i * 0x50);
            if !crate::memoria::is_readable(entry as *const c_void, 0x50) {
                continue;
            }
            let path = crate::rtti::red_string_read(entry.add(0x10));
            let filename = path.rsplit(['/', '\\']).next().unwrap_or(&path);
            if filename == name {
                return true;
            }
        }
    }
    false
}

/// O scope do conjunto do `depot` que montou um archive cujo caminho, em minúsculas, termina em `sufixo` (já em minúsculas). Leitura pura dos
/// conjuntos (`depot+0x10`, stride 0x38, `DynArray` de `ArchiveInfo` de 0x50 com o caminho em `+0x10`, scope `u32` em `+0x30`, `0x10151f6e8`).
pub(crate) unsafe fn scope_do_archive_no_depot(depot: *mut u8, sufixo: &str) -> Option<u32> {
    let grupos = (depot.add(0x10) as *const *mut u8).read();
    let n = (depot.add(0x1c) as *const u32).read();
    if grupos.is_null() || n == 0 || n > 64 {
        return None;
    }
    for gi in 0..n as usize {
        let g = grupos.add(gi * 0x38);
        if !crate::memoria::is_readable(g as *const c_void, 0x38) {
            continue;
        }
        let cnt = (g.add(0x0c) as *const u32).read();
        let arr = (g as *const *const u8).read();
        if arr.is_null() || cnt == 0 || cnt > 100_000 {
            continue;
        }
        for i in 0..cnt as usize {
            let entrada = arr.add(i * 0x50);
            if crate::memoria::is_readable(entrada as *const c_void, 0x50)
                && crate::rtti::red_string_read(entrada.add(0x10)).to_lowercase().ends_with(sufixo)
            {
                return Some((g.add(0x30) as *const u32).read());
            }
        }
    }
    None
}

/// `<jogo>/`: o pai da pasta da dylib (`crate::dylib_dir`) quando ela é a pasta do runtime, o layout instalado
/// (`<jogo>/red4ext/libcp77_console.dylib`). Fora dele, como no executável de teste em `target/`, dá `None` em vez do pai de uma
/// pasta qualquer.
pub(crate) fn game_dir_from_dylib() -> Option<String> {
    pasta_do_jogo(&crate::dylib_dir()?).map(str::to_string)
}

/// O pai de `pasta_da_dylib` se ela se chama `bwms_primitives::PASTA_DO_RUNTIME`; `None` em qualquer outra.
fn pasta_do_jogo(pasta_da_dylib: &str) -> Option<&str> {
    pasta_da_dylib.strip_suffix(bwms_primitives::PASTA_DO_RUNTIME)?.strip_suffix('/')
}

/// Injeção de archive arbitrário (rota B): injeta um .archive REAL cujo nome NÃO casa o glob nativo
/// (basegame_*/audio_*/lang_*) no grupo `g` do ResourceDepot via `LoadArchives` @0x10151b18c
/// (abre+parseia+APENDA, BYPASSA o filtro do glob). `depot`/`g` já resolvidos pelo chamador
/// (`find_pathb_content_group`); `abs_path` é o `.archive` a carregar (path absoluto real).
/// Segura o lock exclusivo do depot durante a injeção. Devolve `true` se `count` subiu.
pub(crate) unsafe fn load_archive_into_group(depot: *mut u8, g: *mut u8, abs_path: &str) -> bool {
    let Some(recursos) = archive_valido(abs_path) else {
        crate::log_dev!("[pathb] '{abs_path}' ausente ou não é um .archive válido — abortado");
        return false;
    };
    let count0 = (g.add(0x0c) as *const u32).read();
    let falhas = montar_no_grupo(depot, g, &[(abs_path, recursos)]);
    let count1 = (g.add(0x0c) as *const u32).read();
    crate::log_dev!(
        "[pathb] g={g:p} | archives ANTES={count0} DEPOIS={count1} | path={abs_path} | {}",
        if falhas.is_empty() { "montado" } else { "o motor não montou" }
    );
    falhas.is_empty()
}

/// Recursos de um `.archive`, lidos do cabeçalho e conferidos antes de entregar ao motor (um arquivo inválido cai no assert dele,
/// `0x103edee14`): `RDAR`, versão 11/12/20, índice alinhado em 4 KB (como o do jogo e o de todo mod medido) e inteiro dentro do arquivo.
/// O cabeçalho, o do índice e o limite do índice são os do `bwms_primitives::rdar`, o mesmo leitor do `bwms archive`; versão e alinhamento
/// são do motor. `None` = não entregar.
pub(crate) fn archive_valido(caminho: &str) -> Option<u32> {
    abrir_archive_valido(caminho).map(|(_, _, recursos)| recursos)
}

/// O `.archive` em `caminho` aberto, com o cabeçalho e os recursos do índice, se passa na regra de [`archive_valido`].
fn abrir_archive_valido(caminho: impl AsRef<std::path::Path>) -> Option<(bwms_primitives::rdar::Cabecalho, std::fs::File, u32)> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(caminho).ok()?;
    let tamanho = f.metadata().ok()?.len();
    let mut h = [0u8; bwms_primitives::rdar::TAMANHO_DO_CABECALHO];
    f.read_exact(&mut h).ok()?;
    let c = bwms_primitives::rdar::cabecalho(&h)?;
    if !matches!(c.versao, 11 | 12 | 20) || c.posicao_do_indice % 4096 != 0 || !c.indice_cabe(tamanho) {
        return None;
    }
    f.seek(SeekFrom::Start(c.posicao_do_indice)).ok()?;
    let mut hi = [0u8; bwms_primitives::rdar::TAMANHO_DO_CABECALHO_DO_INDICE];
    f.read_exact(&mut hi).ok()?;
    let recursos = bwms_primitives::rdar::cabecalho_do_indice(&hi)?.recursos;
    (recursos <= 1_000_000).then_some((c, f, recursos))
}

/// O hash de caminho de cada recurso do `.archive` em `caminho`, lido do índice sem ler os dados (os conflitos entre mods da pasta `Mods/`,
/// `pasta_mods::conflitos`). `None` se o arquivo não passa em [`archive_valido`] ou a tabela de recursos não cabe no índice: o motor não o
/// monta, e ele não disputa recurso com ninguém.
pub(crate) fn hashes_do_archive(caminho: &std::path::Path) -> Option<Vec<u64>> {
    use std::io::{Read, Seek, SeekFrom};
    let (c, mut f, recursos) = abrir_archive_valido(caminho)?;
    let tamanho = bwms_primitives::rdar::TAMANHO_DO_CABECALHO_DO_INDICE + recursos as usize * bwms_primitives::rdar::TAMANHO_DO_RECURSO;
    if tamanho > c.tamanho_do_indice as usize {
        return None;
    }
    f.seek(SeekFrom::Start(c.posicao_do_indice)).ok()?;
    let mut indice = vec![0u8; tamanho];
    f.read_exact(&mut indice).ok()?;
    let hashes: Vec<u64> = bwms_primitives::rdar::hashes_dos_recursos(&indice)?.collect();
    Some(hashes)
}

/// Por que um `.archive` ficou fora: a sincronização da pasta `Mods/` (`pasta_nativa::sincronizar`, `Invalido` e `NomeRepetido`, com o texto no
/// relatório da pasta) e a entrega ao depot de `RegisterArchive`/`RegisterDir` (`montar_no_grupo`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Recusa {
    /// Não passou em [`archive_valido`]: o motor cairia num assert ao abri-lo.
    Invalido,
    /// Já existe um archive carregado com esse nome de arquivo.
    NomeRepetido,
    /// O caminho tem byte nulo e não vira `CString`.
    ByteNulo,
    /// O `LoadArchives` voltou sem a contagem do grupo subir.
    MotorNaoMontou,
    /// A trava do depot seguiu ocupada pela carga do jogo durante a espera.
    DepotOcupado,
}

impl Recusa {
    /// `(PT-BR, inglês)`, cada um com a ação.
    pub(crate) fn texto(self) -> (&'static str, &'static str) {
        match self {
            Recusa::Invalido => (
                "não é um .archive válido; baixe o mod de novo",
                "not a valid .archive; download the mod again",
            ),
            Recusa::NomeRepetido => (
                "já há um archive carregado com este nome; renomeie o arquivo",
                "an archive with this name is already loaded; rename the file",
            ),
            Recusa::ByteNulo => ("o caminho tem um byte nulo; renomeie a pasta", "the path has a null byte; rename the folder"),
            Recusa::MotorNaoMontou => (
                "o jogo não montou o arquivo; baixe o mod de novo",
                "the game did not mount the file; download the mod again",
            ),
            Recusa::DepotOcupado => (
                "o jogo estava carregando arquivos; o mod pode chamar de novo depois da carga",
                "the game was loading files; the mod can call again after loading",
            ),
        }
    }
}

/// Entrega `arquivos` (caminho, recursos pelo cabeçalho) ao grupo `g` do depot, um por chamada ao `LoadArchives` @0x10151b18c, na
/// ordem dada, com a trava exclusiva do depot. Devolve os que o motor não montou (a contagem do grupo não subiu).
unsafe fn montar_no_grupo(depot: *mut u8, g: *mut u8, arquivos: &[(&str, u32)]) -> Vec<(String, Recusa)> {
    use core::arch::asm;
    if arquivos.is_empty() {
        return Vec::new();
    }
    #[repr(C)]
    struct FileList {
        ptr: *const u8,
        cap: u32,
        count: u32,
    }
    #[repr(C)]
    struct ListaHash {
        ptr: *mut u64,
        cap: u32,
        count: u32,
    }
    // O motor põe um hash por recurso nessa lista; a capacidade cobre o maior archive para ele nunca realocar um buffer que não é dele.
    let cap = arquivos.iter().map(|a| a.1).max().unwrap_or(0) + 16;
    let mut buf = vec![0u64; cap as usize].into_boxed_slice();
    let mut falhas = Vec::new();
    // Na 2.5 a carga do jogo segura a trava do depot no 1º tick (`0x88` medido, lane B de 2026-10-28); a espera do motor (`0x1000015f0`) prendia
    // a thread do tick até o jogo parar. A tentativa do motor (`0x100001614`: `casalb` 0 → 0x80, devolve se pegou) vai por até 50 ms; ocupada,
    // nada é entregue.
    let trava = depot.add(0x78);
    let tentar: unsafe extern "C" fn(*mut u8) -> u8 = std::mem::transmute(crate::rebase(crate::jogo::travas::TRAVA_TENTAR_ESCRITA_VM));
    if !(0..500).any(|i| {
        if i > 0 {
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        tentar(trava) & 1 != 0
    }) {
        crate::log_dev!("[pathb] montar: trava do depot ocupada ({:#04x}) por 50 ms; nada entregue", trava.read());
        return arquivos.iter().map(|&(c, _)| (c.to_string(), Recusa::DepotOcupado)).collect();
    }
    for &(c, _) in arquivos {
        let Ok(cpath) = std::ffi::CString::new(c) else {
            falhas.push((c.to_string(), Recusa::ByteNulo));
            continue;
        };
        let mut s_path = [0u8; 0x20];
        asm!("blr {f}", f = in(reg) crate::rebase(0x1_0002_ccac), in("x0") s_path.as_mut_ptr(), in("x1") cpath.as_ptr(), clobber_abi("C"));
        let flist = FileList { ptr: s_path.as_ptr(), cap: 1, count: 1 };
        let mut lista = ListaHash { ptr: buf.as_mut_ptr(), cap, count: 0 };
        let k0 = (g.add(0x0c) as *const u32).read();
        asm!("blr {f}", f = in(reg) crate::rebase(0x1_0151_b18c),
            in("x0") 0usize, in("x1") g, in("x2") &flist as *const FileList,
            in("x3") &mut lista as *mut ListaHash, in("x4") 0usize, in("x5") 2usize,
            clobber_abi("C"));
        // O `LoadArchives` só lê o caminho: abre o arquivo pelo `c_str` e copia o texto para o nome do `ArchiveInfo` (`0x10002aa74`), sem
        // guardar o ponteiro, então destruir na volta é seguro. Caminho de 20 bytes ou mais mora num bloco do `PoolString`.
        #[cfg(feature = "devtools")]
        let antes = s_path;
        crate::jogo::cstring::destroi_cstring_da_pilha(&mut s_path);
        #[cfg(feature = "devtools")]
        registrar_caminho_solto(c, &antes, &s_path);
        if (g.add(0x0c) as *const u32).read() == k0 {
            falhas.push((c.to_string(), Recusa::MotorNaoMontou));
        }
    }
    // Solta só o bit de escritor, como o motor (`ldclralb 0x80`): zerar o byte inteiro apaga leitores e prende o SharedSpinLock.
    std::sync::atomic::AtomicU8::from_ptr(trava).fetch_and(0x7f, Ordering::Release);
    falhas
}

/// O que o destrutor do motor (`0x10002914c` → `0x10002c3d4(cs, 0)`) fez com o `CString` do caminho, pelos 0x20 bytes antes e depois.
/// Cabeçalho em `+0x14`: modo nos 2 bits altos (0 inline, 1 bloco do alocador, 2 vista sem dono, 3 texto por ponteiro que o destrutor não
/// solta), tamanho nos 30 baixos; capacidade em `+0x10`. O construtor `0x10002ccac` só monta modo 0 ou modo 1 com capacidade `tamanho | 7`.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Soltura {
    /// Modo 1 com capacidade de 1 a 0xFFFF_FFF7 e o objeto vazio depois: o único ramo do `0x10002c3d4(cs, 0)` que chama o
    /// `Free<PoolString>` (capacidade 0 desvia em `0x10002c50c` e de 0xFFFF_FFF8 para cima em `0x10002c524`, os dois só zerando). O bloco
    /// voltou ao `PoolString`.
    Bloco,
    /// Modo 0: o texto morava no próprio objeto e não havia bloco.
    Inline,
    /// O cabeçalho ou o 1º byte não zeraram. Com modo 0 ou 1 antes: o destrutor não rodou, ou o endereço não é o destrutor desta build.
    /// Com modo 2 ou 3 antes, é o que o destrutor faz (o modo 2 ele nem toca, `b.eq` em `0x100029158`; no 3 o `ands` em `0x10002c56c`
    /// guarda os 2 bits do modo e o `strb` zera o 1º byte do texto apontado, não o do objeto): a `CString` não saiu do construtor desta build.
    NaoEsvaziou,
    /// Modo 1 com capacidade 0 ou de 0xFFFF_FFF8 para cima e o objeto vazio depois: o destrutor zerou sem soltar bloco nenhum, e o
    /// construtor `0x10002ccac` não monta essa capacidade.
    ForaDoConstrutor,
    /// Modo 2 ou 3 e o objeto vazio depois, o que o destrutor do desmonte não faz: a rotina em `0x10002914c` não é a do desmonte.
    DestrutorDiferente,
}

#[cfg(any(test, feature = "devtools"))]
fn soltura(antes: &[u8; 0x20], depois: &[u8; 0x20]) -> Soltura {
    let palavra = |b: &[u8; 0x20], o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
    if palavra(depois, 0x14) != 0 || depois[0] != 0 {
        return Soltura::NaoEsvaziou;
    }
    match palavra(antes, 0x14) >> 30 {
        0 => Soltura::Inline,
        1 if (1..0xFFFF_FFF8).contains(&palavra(antes, 0x10)) => Soltura::Bloco,
        1 => Soltura::ForaDoConstrutor,
        _ => Soltura::DestrutorDiferente,
    }
}

/// Um caminho entregue por [`montar_no_grupo`] desde o boot: onde o texto da `CString` morava (o bloco, ou o próprio objeto se inline) e
/// o que o destrutor fez com ela.
#[cfg(feature = "devtools")]
struct CaminhoSolto {
    caminho: String,
    texto_em: u64,
    soltura: Soltura,
}

#[cfg(feature = "devtools")]
static CAMINHOS_SOLTOS: std::sync::Mutex<Vec<CaminhoSolto>> = std::sync::Mutex::new(Vec::new());

/// `depois` é a própria `CString` da pilha, já destruída; `antes`, a cópia dos 0x20 bytes tirada antes do destrutor.
#[cfg(feature = "devtools")]
fn registrar_caminho_solto(caminho: &str, antes: &[u8; 0x20], depois: &[u8; 0x20]) {
    let s = soltura(antes, depois);
    let texto_em = if s == Soltura::Inline {
        depois.as_ptr() as u64
    } else {
        u64::from_le_bytes([antes[0], antes[1], antes[2], antes[3], antes[4], antes[5], antes[6], antes[7]])
    };
    if let Ok(mut v) = CAMINHOS_SOLTOS.lock() {
        v.push(CaminhoSolto { caminho: caminho.to_string(), texto_em, soltura: s });
    }
}

/// Onde mora o texto do nome do `ArchiveInfo` cujo nome é `caminho`, em qualquer grupo do depot. Leitura pura.
#[cfg(feature = "devtools")]
unsafe fn texto_do_nome_no_depot(depot: *mut u8, caminho: &str) -> Option<u64> {
    if depot.is_null() {
        return None;
    }
    let grupos = (depot.add(0x10) as *const *mut u8).read();
    let n = (depot.add(0x1c) as *const u32).read();
    if grupos.is_null() || n == 0 || n > 64 {
        return None;
    }
    for gi in 0..n as usize {
        let g = grupos.add(gi * 0x38);
        if !crate::memoria::is_readable(g as *const c_void, 0x38) {
            continue;
        }
        let arr = (g as *const *const u8).read();
        let cnt = (g.add(0x0c) as *const u32).read();
        if arr.is_null() || cnt > 100_000 {
            continue;
        }
        for i in 0..cnt as usize {
            let nome = arr.add(i * 0x50 + 0x10);
            if !crate::memoria::is_readable(nome as *const c_void, 0x20) || crate::rtti::red_string_read(nome) != caminho {
                continue;
            }
            let palavra = (nome.add(0x14) as *const u32).read_unaligned();
            return Some(if palavra >> 30 == 0 { nome as u64 } else { (nome as *const u64).read_unaligned() });
        }
    }
    None
}

/// `pathbsolta`: o veredito do destrutor do caminho em [`montar_no_grupo`], uma linha. Por archive entregue desde o boot: a `CString` ficou
/// vazia (com o texto num bloco, o bloco voltou ao `PoolString`), e o `ArchiveInfo` do depot tem o caminho inteiro num texto que não mora
/// onde morava o da `CString`: o `LoadArchives` copiou o caminho em vez de guardar o ponteiro. `ok` pede ao menos um caminho com bloco, o caso
/// que vazava. Leitura pura.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn pathbsolta() -> String {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed) as *mut u8;
    let registros: Vec<(String, u64, Soltura)> = match CAMINHOS_SOLTOS.lock() {
        Ok(v) => v.iter().map(|r| (r.caminho.clone(), r.texto_em, r.soltura)).collect(),
        Err(_) => Vec::new(),
    };
    let (mut blocos, mut inline, mut copias) = (0usize, 0usize, 0usize);
    let mut falhas: Vec<String> = Vec::new();
    for (caminho, texto_em, s) in &registros {
        match s {
            Soltura::Bloco => blocos += 1,
            Soltura::Inline => inline += 1,
            Soltura::NaoEsvaziou => falhas.push(format!("{caminho}: CString não ficou vazia")),
            Soltura::ForaDoConstrutor => falhas.push(format!("{caminho}: CString fora do construtor, sem bloco a soltar")),
            Soltura::DestrutorDiferente => {
                falhas.push(format!("{caminho}: CString esvaziou no modo 2/3, que o destrutor do desmonte não esvazia"))
            }
        }
        match texto_do_nome_no_depot(depot, caminho) {
            None => falhas.push(format!("{caminho}: nome ausente no depot")),
            Some(p) if p == *texto_em => falhas.push(format!("{caminho}: nome no texto solto {p:#x}")),
            Some(_) => copias += 1,
        }
    }
    let ok = blocos > 0 && falhas.is_empty();
    format!(
        "[pathb-solta] VEREDITO-PATHB-SOLTA archives={} bloco-solto={blocos} inline={inline} nome-copiado={copias} falhas={falhas:?} ok={ok}",
        registros.len()
    )
}

/// Prefixos vanilla (`basegame_`/`audio_`/`lang_`/`dlc_`, o próprio glob nativo que
/// `RegisterArchive`/Path-B existe pra CONTORNAR): um archive com um desses nomes quase certamente
/// já está carregado pela via normal. Achado 2026-08-09: re-injetar `basegame_4_gamedata.archive`
/// (produção, grande) via este bypass CROU (`EXC_BAD_ACCESS`, thread `redDispatcher5`, dentro de
/// `load_archive_into_group`) — causa raiz não isolada, mas o caso de uso é sempre ilegítimo (um
/// mod nunca precisa re-registrar conteúdo vanilla), então recusar aqui é estritamente mais seguro
/// sem perder capacidade real nenhuma.
pub(crate) fn is_vanilla_archive_name(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    ["basegame_", "audio_", "lang_", "dlc_"]
        .iter()
        .any(|pfx| name.starts_with(pfx))
}

pub unsafe fn facade_register_archive(path: &str) -> bool {
    if is_vanilla_archive_name(path) {
        crate::log_dev!(
            "[facade] RegisterArchive: '{path}' tem nome de prefixo VANILLA (basegame_/audio_/lang_/dlc_) — \
             recusado (já carregado pela via normal; ver achado 2026-08-09 de crash ao re-injetar)"
        );
        return false;
    }
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() {
        crate::log_dev!("[facade] RegisterArchive: depot ainda não capturado (InitializeArchives não rodou, ou hook recusou) — abortado");
        return false;
    }
    let abs = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        match game_dir_from_dylib() {
            Some(gd) => format!("{gd}/{path}"),
            None => {
                crate::log_dev!("[facade] RegisterArchive: path relativo mas não achei o dir do jogo — abortado");
                return false;
            }
        }
    };
    crate::log_dev!("[facade] RegisterArchive: depot {depot:p}, procurando o grupo de '{abs}'");
    let Some(g) = find_pathb_content_group(depot as *mut u8) else { return false };
    crate::log_dev!("[facade] RegisterArchive: grupo {g:p} com {} archive(s); entregando ao LoadArchives", (g.add(0x0c) as *const u32).read());
    load_archive_into_group(depot as *mut u8, g, &abs)
}

/// `RegisterDir(path: String) -> Bool` da fachada de recursos — registra TODOS os `.archive` (não-recursivo)
/// de um diretório via `facade_register_archive` (mesma regra path absoluto/relativo-ao-jogo).
/// Devolve `true` se o diretório foi lido e PELO MENOS 1 arquivo foi injetado com sucesso.
pub unsafe fn facade_register_dir(path: &str) -> bool {
    let abs_dir = if std::path::Path::new(path).is_absolute() {
        path.to_string()
    } else {
        match game_dir_from_dylib() {
            Some(gd) => format!("{gd}/{path}"),
            None => {
                crate::log_dev!("[facade] RegisterDir: path relativo mas não achei o dir do jogo — abortado");
                return false;
            }
        }
    };
    let entries = match std::fs::read_dir(&abs_dir) {
        Ok(e) => e,
        Err(err) => {
            crate::log_dev!("[facade] RegisterDir: não consegui ler '{abs_dir}': {err}");
            return false;
        }
    };
    let mut any_ok = false;
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) == Some("archive") {
            if let Some(s) = p.to_str() {
                if facade_register_archive(s) {
                    any_ok = true;
                }
            }
        }
    }
    any_ok
}

pub(crate) static CLASSE_DE_COMPONENTE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static CLASSE_DE_COMPONENTE_EXTERNO: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static TIPO_DO_ARRAY_DE_HANDLES: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// ===== 17d — `resource.patch` nas partes, pela tabela (2026-09-12) =====
// `bwms-resource-patch.txt` (gerado de `.xl` por `bwms_primitives::instalacao::tabelas_de_runtime::emit_resource_patch_table`) entra na carga das tabelas, na game thread: classes e
// tipo da mescla guardados com os offsets conferidos, token de cada patch pedido ali e o hook do `ExtractPartComponents` instalado. O que a extração de
// uma parte alvo faz com os patches dela: `aplicar_patches_na_parte`.
// Fatos usados ali: caminho da parte no token em `+0x48`, recurso em `+0x28`; `rootIndex` (i16) do `entEntityTemplate` em `+0x1d8`; a extração do
// motor lê o `rootIndex` uma vez no começo e, com ele ≥ 0, apaga o bit desse objeto no bitset de objetos (`bic` em `0x1017d32f0`), então a raiz
// não vem na extração.
#[derive(Clone)]
pub(crate) struct PatchDeRecurso {
    pub(crate) mod_id: String,
    pub(crate) patch: u64,
    pub(crate) alvo: u64,
    /// Props do patch em bits (`A17_PROP_*`); `u32::MAX` = props vazio no `.xl` (todas).
    pub(crate) props: u32,
    pub(crate) token: [u64; 2],
    /// Token da parte alvo, pedido na carga da tabela só com `components`/`entity`: o hook do job de garment casa pelo ponteiro do template (`+0x28` do
    /// token). Fica retido (1 template por alvo, consciente).
    pub(crate) alvo_token: [u64; 2],
    /// Alvo `.app`: o patch também entra nos extratores de aparência do `EntityBuilder` (51f-2).
    pub(crate) app: bool,
}

/// Tabela viva do `resource.patch` com os índices que os ganchos de carga consultam. Tudo entra junto sob a trava de `PATCHES_DE_RECURSO`, então um índice
/// nunca aponta para outra versão de `patches`.
pub(crate) struct TabelaDePatches {
    pub(crate) patches: Vec<PatchDeRecurso>,
    /// `(alvo, índice em patches)` ordenado pelo alvo e, no mesmo alvo, pela ordem da tabela: o gancho de cada carga busca o caminho do recurso em
    /// O(log n), em vez de varrer `patches`. Vec ordenado e não `HashMap`: cabe no `static` const e, de 32 a 1.000 patches,
    /// empatou ou ganhou do `HashMap` (`busca_por_alvo_custo`).
    por_alvo: Vec<(u64, u32)>,
    /// Índices com token do alvo pedido (`alvo_token[0] != 0`). Só eles casam pelo ponteiro do template (51f, 17e): o ponteiro existe só depois que o
    /// token resolve, então esse casamento não cabe num índice montado na publicação.
    com_alvo_token: Vec<u32>,
    /// Índices dos patches de alvo `.app` (51f-2).
    apps: Vec<u32>,
}

impl TabelaDePatches {
    pub(crate) const VAZIA: TabelaDePatches = TabelaDePatches { patches: Vec::new(), por_alvo: Vec::new(), com_alvo_token: Vec::new(), apps: Vec::new() };

    pub(crate) fn nova(patches: Vec<PatchDeRecurso>) -> TabelaDePatches {
        let mut por_alvo: Vec<(u64, u32)> = patches.iter().zip(0u32..).map(|(p, i)| (p.alvo, i)).collect();
        // Estável: no mesmo alvo os patches saem na ordem da tabela, que é a ordem em que a mescla os aplica.
        por_alvo.sort_by_key(|&(alvo, _)| alvo);
        let indices = |f: fn(&PatchDeRecurso) -> bool| patches.iter().zip(0u32..).filter(|(p, _)| f(p)).map(|(_, i)| i).collect::<Vec<u32>>();
        let com_alvo_token = indices(|p| p.alvo_token[0] != 0);
        let apps = indices(|p| p.app);
        TabelaDePatches { patches, por_alvo, com_alvo_token, apps }
    }

    /// Índices dos patches cujo alvo é `caminho`, na ordem da tabela.
    fn indices_do_alvo(&self, caminho: u64) -> impl Iterator<Item = u32> + '_ {
        let ini = self.por_alvo.partition_point(|&(alvo, _)| alvo < caminho);
        self.por_alvo[ini..].iter().take_while(move |&&(alvo, _)| alvo == caminho).map(|&(_, i)| i)
    }

    /// Patches cujo alvo é `caminho`, na ordem da tabela.
    pub(crate) fn do_alvo(&self, caminho: u64) -> impl Iterator<Item = &PatchDeRecurso> + '_ {
        self.indices_do_alvo(caminho).filter_map(|i| self.patches.get(i as usize))
    }

    /// Patches cujo alvo é `caminho` ou cujo template alvo é a instância `tpl` (não nula), na ordem da tabela e sem repetição. `instancia` lê o
    /// template do token (`+0x28`, com o `is_readable` por syscall), então só roda para patch com token do alvo que passou em `aceita` e não casou
    /// pelo caminho.
    pub(crate) fn do_alvo_ou_da_instancia(
        &self,
        caminho: Option<u64>,
        tpl: u64,
        aceita: impl Fn(&PatchDeRecurso) -> bool,
        instancia: impl Fn([u64; 2]) -> u64,
    ) -> impl Iterator<Item = &PatchDeRecurso> + '_ {
        let patch = |i: u32| self.patches.get(i as usize);
        let mut indices: Vec<u32> = caminho.map_or_else(Vec::new, |c| self.indices_do_alvo(c).filter(|&i| patch(i).is_some_and(|p| aceita(p))).collect());
        let pelo_caminho = indices.len();
        indices.extend(self.com_alvo_token.iter().copied().filter(|&i| {
            patch(i).is_some_and(|p| aceita(p) && caminho != Some(p.alvo) && instancia(p.alvo_token) == tpl)
        }));
        if pelo_caminho > 0 && indices.len() > pelo_caminho {
            indices.sort_unstable();
        }
        indices.into_iter().filter_map(|i| self.patches.get(i as usize))
    }

    /// Patches de alvo `.app`, na ordem da tabela.
    pub(crate) fn apps(&self) -> impl Iterator<Item = &PatchDeRecurso> + '_ {
        self.apps.iter().filter_map(|&i| self.patches.get(i as usize))
    }
}

pub(crate) static PATCHES_DE_RECURSO: std::sync::RwLock<TabelaDePatches> = std::sync::RwLock::new(TabelaDePatches::VAZIA);

pub(crate) static PATCHES_DE_RECURSO_ATIVOS: AtomicBool = AtomicBool::new(false);

/// Props do `resource.patch` em bits: a tabela de nomes mora no `bwms-primitives`, que o install também usa para gravar a máscara.
pub(crate) use bwms_primitives::{
    A17_PROP_APPEARANCES, A17_PROP_BASEMESH, A17_PROP_BASEMESHAPP, A17_PROP_BASETEXTURE, A17_PROP_BASETEXTUREPARAM, A17_PROP_BLOB, A17_PROP_BLOCKS,
    A17_PROP_BOUNDINGBOX, A17_PROP_CENSORSHIP, A17_PROP_COMPONENTS, A17_PROP_DEPENDENCIES, A17_PROP_ENTITY, A17_PROP_PARTSOVERRIDES, A17_PROP_PARTSVALUES,
    A17_PROP_RENDERBLOB, A17_PROP_TARGETS, A17_PROP_VISUALTAGS,
};

/// Linhas da tabela como estão no arquivo (`bwms_primitives::tabelas::ResourcePatch`; linha fora do formato fica de fora). O install já resolveu os mods
/// entre si (`bwms_primitives::instalacao::tabelas_de_runtime::emit_resource_patch_table`: alvo que é patch sai, par repetido entra uma vez, props do último config
/// do patch). Props em máscara hex (`0x…`); `-` liga todas; tabela posta à mão pode
/// trazer os nomes (`bwms_primitives::a17_props_mascara`).
pub(crate) fn linhas_da_tabela_de_patches(texto: &str) -> Vec<bwms_primitives::tabelas::ResourcePatch<'_>> {
    bwms_primitives::tabelas::ler(texto, bwms_primitives::tabelas::ResourcePatch::ler).flatten().collect()
}

/// Troca a tabela viva inteira, com os índices montados fora da trava. Toda mescla do `#17`/`#51` consulta `PATCHES_DE_RECURSO`: tabela vazia desliga a mescla.
fn publicar_tabela_de_patches(patches: Vec<PatchDeRecurso>) -> usize {
    let n = patches.len();
    let tabela = TabelaDePatches::nova(patches);
    if let Ok(mut g) = PATCHES_DE_RECURSO.write() {
        *g = tabela;
    }
    PATCHES_DE_RECURSO_ATIVOS.store(n > 0, Ordering::Relaxed);
    if n == 0 {
        limpar_esperas();
    }
    n
}

/// Os patches de recurso que o depot não tem na última carga da tabela, `(mod_id, caminho do patch)`: a linha deles sai da tabela, e o mod
/// dono fica amarelo (`mods_do_usuario::pasta_mods::estados_dos_mods`).
static PATCHES_FORA_DO_DEPOT: std::sync::Mutex<Vec<(String, String)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn patches_fora_do_depot() -> Vec<(String, String)> {
    PATCHES_FORA_DO_DEPOT.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Troca a lista da carga anterior e põe os da `Mods/` (id `Mods/<mod>/…`, `apply::xl_com_id`) na seção Recursos do relatório dela, que só
/// existe com algum: a carga sem nenhum tira a seção da carga anterior.
fn guardar_patches_fora_do_depot(fora: Vec<(String, String)>) {
    let prefixo = format!("{}/", bwms_primitives::instalacao::apply::PASTA_MODS);
    let linhas: Vec<String> = fora
        .iter()
        .filter(|(id, _)| id.starts_with(&prefixo))
        .map(|(id, patch)| {
            format!(
                "aviso / warning: {id} — o .xl aponta um recurso que o jogo não tem ({patch}); essa parte do mod ficou sem efeito: confira se o \
                 .archive do mod está na pasta / the .xl points to a resource the game does not have ({patch}); that part of the mod has no \
                 effect: check that the mod's .archive is in the folder"
            )
        })
        .collect();
    let havia = !std::mem::replace(&mut *PATCHES_FORA_DO_DEPOT.lock().unwrap_or_else(std::sync::PoisonError::into_inner), fora).is_empty();
    if !linhas.is_empty() || havia {
        crate::mods_do_usuario::pasta_mods::anexar_ao_relatorio("Recursos / Resources", &linhas);
    }
}

/// Carrega `bwms-resource-patch.txt` na carga das tabelas (game thread): classes e tipo da mescla, token de cada patch e o hook. Devolve os patches ligados.
/// Arquivo ausente ou carga que não monta deixa a tabela vazia: a mescla desliga em vez de ficar com a tabela da carga anterior.
pub(crate) unsafe fn carregar_tabela_de_patches(arquivo: &str) -> usize {
    let Ok(texto) = std::fs::read_to_string(arquivo) else { return publicar_tabela_de_patches(Vec::new()) };
    let linhas = linhas_da_tabela_de_patches(&texto);
    let Some(reg) = crate::rtti::Registry::obtain() else { return publicar_tabela_de_patches(Vec::new()) };
    if let Err(motivo) = guardar_classes_da_mescla(&reg) {
        crate::veredito_dev!("[a17] tabela {arquivo}: mescla desligada — {motivo}");
        return publicar_tabela_de_patches(Vec::new());
    }
    let mut patches: Vec<PatchDeRecurso> = Vec::new();
    // Classes dos recursos de patch conferidas nos hooks (`recurso_do_patch_da_classe`): patch de outra classe fica de fora.
    CLASSE_ENTENTITYTEMPLATE.store(reg.class_by_name("entEntityTemplate"), Ordering::Relaxed);
    CLASSE_APPEARANCERESOURCE.store(reg.class_by_name("appearanceAppearanceResource"), Ordering::Relaxed);
    CLASSE_MORPHTARGETMESH.store(reg.class_by_name("MorphTargetMesh"), Ordering::Relaxed);
    // Os hooks do `PostLoad` do template (51a) e do `AppearanceResource` (51c) entram antes de pedir os tokens: alvo que carregar depois disso passa pela
    // mescla. O tipo do alvo sai da extensão do caminho.
    let alvo_de = |l: &bwms_primitives::tabelas::ResourcePatch, ext: &str| l.alvo.to_ascii_lowercase().ends_with(ext);
    if linhas.iter().any(|l| alvo_de(l, ".ent") && l.props & (A17_PROP_APPEARANCES | A17_PROP_DEPENDENCIES | A17_PROP_VISUALTAGS) != 0) {
        guardar_offsets_do_template(&reg);
        install_template_postload_hook();
    }
    if linhas.iter().any(|l| alvo_de(l, ".app") && l.props & (A17_PROP_APPEARANCES | A17_PROP_CENSORSHIP) != 0) {
        guardar_offsets_do_app(&reg);
        install_app_postload_hook();
    }
    if linhas.iter().any(|l| alvo_de(l, ".morphtarget") && l.props & PROPS_DO_MORPH != 0) {
        guardar_offsets_do_morph(&reg);
        install_morph_postload_hook();
    }
    if linhas.iter().any(|l| alvo_de(l, ".streamingworld") && l.props & A17_PROP_BLOCKS != 0) {
        guardar_offsets_do_mundo(&reg);
        install_world_postload_hook();
    }
    // 51g: alvo `.curveset` passa pelo `OnDependenciesReady` do serializer.
    if linhas.iter().any(|l| alvo_de(l, ".curveset")) && guardar_offsets_do_curveset(&reg) {
        #[cfg(feature = "devtools")]
        A51G_TESTE.store(marcador_individual("a51g-teste"), Ordering::Relaxed);
        A51G_ATIVO.store(true, Ordering::Relaxed);
        install_dependencias_prontas_hook();
        crate::veredito_dev!(
            "[a51g] hook do OnDependenciesReady {}",
            if GANCHO_DEPENDENCIAS_NO_AR.load(Ordering::Relaxed) { "instalado" } else { "FALHOU" }
        );
    }
    // Alvo `.inkanim` passa pelo mesmo hook do `OnDependenciesReady` (sequências por nome).
    if linhas.iter().any(|l| alvo_de(l, ".inkanim")) && crate::interface::guardar_offsets_do_inkanim(&reg) {
        crate::interface::MESCLA_DE_INKANIM_ATIVA.store(true, Ordering::Relaxed);
        install_dependencias_prontas_hook();
        crate::veredito_dev!(
            "[inkanim] hook do OnDependenciesReady {}",
            if GANCHO_DEPENDENCIAS_NO_AR.load(Ordering::Relaxed) { "instalado" } else { "FALHOU" }
        );
    }
    // 51i-2: alvo `.mesh` com `appearances` passa pelo hook do `CMesh::PostLoad` (o mesmo do fix de nomes).
    if linhas.iter().any(|l| alvo_de(l, ".mesh") && l.props & (A17_PROP_APPEARANCES | A17_PROP_RENDERBLOB) != 0) && guardar_offsets_do_cmesh(&reg) {
        CLASSE_CMESH.store(reg.class_by_name("CMesh"), Ordering::Relaxed);
        CLASSE_MESHMESHAPPEARANCE.store(reg.class_by_name("meshMeshAppearance"), Ordering::Relaxed);
        // 51i-3: classe do blob de render do mesh.
        CLASSE_RENDERMESHBLOB.store(reg.class_by_name("rendRenderMeshBlob"), Ordering::Relaxed);
        let classes_ok = !CLASSE_CMESH.load(Ordering::Relaxed).is_null() && !CLASSE_MESHMESHAPPEARANCE.load(Ordering::Relaxed).is_null();
        MESCLA_DE_APARENCIA_DO_MESH_ATIVA.store(classes_ok, Ordering::Relaxed);
        MESCLA_DE_RENDER_BLOB_ATIVA.store(classes_ok && !CLASSE_RENDERMESHBLOB.load(Ordering::Relaxed).is_null(), Ordering::Relaxed);
        if classes_ok {
            install_cmesh_postload_hook();
        }
        crate::veredito_dev!(
            "[a51i] 51i-2: aparências de mesh {}; hook do CMesh::PostLoad {}",
            if classes_ok { "ligadas" } else { "desligadas (classe sem RTTI)" },
            if ORIG_CMESH_POSTLOAD.load(Ordering::Relaxed) == 0 { "ausente" } else { "instalado" }
        );
    }
    // 51h: alvo `.devices` passa pelo mesmo hook do `OnDependenciesReady`.
    if linhas.iter().any(|l| alvo_de(l, ".devices")) && guardar_offsets_do_device_resource(&reg) {
        #[cfg(feature = "devtools")]
        A51H_TESTE.store(marcador_individual("a51h-teste"), Ordering::Relaxed);
        MESCLA_DE_DEVICES_ATIVA.store(true, Ordering::Relaxed);
        install_dependencias_prontas_hook();
        crate::veredito_dev!(
            "[a51h] hook do OnDependenciesReady {}",
            if GANCHO_DEPENDENCIAS_NO_AR.load(Ordering::Relaxed) { "instalado" } else { "FALHOU" }
        );
    }
    // 51f: o job de componentes do `EntityBuilder` mescla nos resultados do extrator quando algum alvo `.ent` tem `components`/`entity`.
    let teste_51f = marcador_individual("a51f-teste");
    // 51f-2: alvo `.app` também passa pelo job do builder (componentes da definição homônima do patch nos extratores de aparência).
    let teste_51f2 = marcador_individual("a51f2-teste");
    let app_51f2 = teste_51f2 || linhas.iter().any(|l| alvo_de(l, ".app"));
    let builder_51f =
        teste_51f || app_51f2 || linhas.iter().any(|l| alvo_de(l, ".ent") && l.props & (A17_PROP_COMPONENTS | A17_PROP_ENTITY) != 0);
    let mut inexistentes: Vec<u64> = Vec::new();
    let mut fora: Vec<(String, String)> = Vec::new();
    for bwms_primitives::tabelas::ResourcePatch { mod_id, patch, alvo, props } in linhas {
        let app = alvo.to_ascii_lowercase().ends_with(".app");
        let h = bwms_primitives::resource_path_hash(patch);
        // Patch que o depot não tem sai da tabela, com erro no log uma vez por caminho.
        if !existe_no_depot(h) {
            let par = (mod_id.to_string(), patch.to_string());
            if !fora.contains(&par) {
                fora.push(par);
            }
            if !inexistentes.contains(&h) {
                inexistentes.push(h);
                A17_PATCHES_INEXISTENTES.fetch_add(1, Ordering::Relaxed);
                crate::veredito_dev!("[a17] patch \"{patch}\" ({h:#018x}) não existe no depot; linha(s) dele fora da tabela");
            }
            continue;
        }
        let a = bwms_primitives::resource_path_hash(alvo);
        let token = match patches.iter().find(|p| p.patch == h) {
            Some(p) => p.token,
            None => token_do_path(h).unwrap_or([0; 2]),
        };
        // Só a extração de parte (17d/17e) usa o token do alvo; o template alvo da 51a carrega quando o jogo pede e passa pelo hook do `PostLoad`.
        let alvo_token = match patches.iter().find(|p| p.alvo == a && p.alvo_token[0] != 0) {
            Some(p) => p.alvo_token,
            None if props & (A17_PROP_COMPONENTS | A17_PROP_ENTITY) != 0 => token_do_path(a).unwrap_or([0; 2]),
            None => [0; 2],
        };
        patches.push(PatchDeRecurso { mod_id: mod_id.to_string(), patch: h, alvo: a, props, token, alvo_token, app });
    }
    guardar_patches_fora_do_depot(fora);
    // Teste da 51f (`~/.bwms-hook-a51f-teste`): os templates do V entram como patch de si mesmos com `components`. O install tira alvo que também é patch,
    // então a linha não sai do texto; a mescla no builder do V troca cada componente pela instância da extração do patch, com o mesmo conteúdo.
    #[cfg(feature = "devtools")]
    if teste_51f {
        for c in A51F_TESTE_ALVOS {
            let h = bwms_primitives::resource_path_hash(c);
            let t = token_do_path(h).unwrap_or([0; 2]);
            patches.push(PatchDeRecurso { mod_id: "bwms-teste-51f".to_string(), patch: h, alvo: h, props: A17_PROP_COMPONENTS, token: t, alvo_token: t, app: false });
        }
    }
    let n = patches.len();
    let mods = patches.iter().map(|p| p.mod_id.as_str()).collect::<std::collections::BTreeSet<_>>().len();
    let sem_token = patches.iter().filter(|p| p.token[0] == 0).count();
    // 51k: alvo → tokens dos patches que ele espera no `Deserialize` (sem ciclo; o teste da 51f, patch de si mesmo, sai por ciclo).
    let (esperas, fora_por_ciclo) = esperas_da_tabela(&patches);
    publicar_tabela_de_patches(patches);
    preparar_esperas(esperas, fora_por_ciclo);
    MESCLA_NO_BUILDER_ATIVA.store(n > 0 && builder_51f, Ordering::Relaxed);
    if n > 0 {
        install_extract_part_components_hook();
        // 17e v3 provada no boot 124 (`garment-ok=true`): roda com a tabela; o marcador só liga o veredito do tick.
        install_garment_extract_job_hook();
        if builder_51f {
            MESCLA_DE_APARENCIA_ATIVA.store(app_51f2, Ordering::Relaxed);
            A51F2_TESTE.store(teste_51f2, Ordering::Relaxed);
            let teste_none = marcador_individual("a51f2-none-teste");
            A51F2_TESTE_NONE.store(teste_none, Ordering::Relaxed);
            if teste_none {
                crate::veredito_dev!("[a51f2] teste com as definições do patch sem nome (None): a 1ª definição vale pra toda aparência");
            }
            install_job_do_builder_hook();
            crate::veredito_dev!(
                "[a51f] hook do job de componentes do EntityBuilder {}",
                if ORIG_JOB_DO_BUILDER.load(Ordering::Relaxed) == 0 { "FALHOU" } else { "instalado" }
            );
        }
    }
    // 51f-3: `IncludeAppearanceParts` não depende de patch — liga com qualquer linha na tabela e com o teste (`~/.bwms-hook-a51f3-teste`, que pede já na
    // carga o token da parte de teste, pra ela estar pronta quando o builder do V rodar).
    let teste_51f3 = marcador_individual("a51f3-teste");
    if n > 0 || teste_51f3 {
        A51F3_TESTE.store(teste_51f3, Ordering::Relaxed);
        if teste_51f3 {
            let _ = token_da_parte(bwms_primitives::resource_path_hash(A51B_TESTE_ALVO));
        }
        INCLUSAO_DE_PARTES_ATIVA.store(true, Ordering::Relaxed);
        install_job_do_builder_hook();
        crate::veredito_dev!(
            "[a51f3] inclusão de partes ligada (teste {teste_51f3}, tag AppearanceParts {:#018x}); hook do job de componentes do EntityBuilder {}",
            tag_appearance_parts(),
            if ORIG_JOB_DO_BUILDER.load(Ordering::Relaxed) == 0 { "FALHOU" } else { "instalado" }
        );
    }
    crate::veredito_dev!("[a17] tabela {arquivo}: {n} patch(es) de parte de {mods} mod(s), {sem_token} sem token");
    n
}

/// `(token, props)` dos patches da tabela viva cujo alvo é `caminho` e cujas props cruzam `mascara`, na ordem da tabela. A cópia sai da trava antes
/// de qualquer chamada ao motor: a recarga da tabela não espera um gancho de carga.
fn patches_do_alvo(caminho: u64, mascara: u32) -> Vec<([u64; 2], u32)> {
    if caminho == 0 || !PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) {
        return Vec::new();
    }
    let Ok(t) = PATCHES_DE_RECURSO.read() else { return Vec::new() };
    t.do_alvo(caminho).filter(|p| p.props & mascara != 0).map(|p| (p.token, p.props)).collect()
}

/// Os valores de 8 B de um `DynArray` em `obj + off` (`CName`, `ResourcePath`), ou `None` com a lista ilegível ou maior que 0x10000.
unsafe fn lista_de_u64(obj: *const u8, off: usize) -> Option<Vec<u64>> {
    let (b, n, _) = dynarray_no_campo(obj, off);
    if n > 0x1_0000 || (n > 0 && (b.is_null() || !crate::memoria::is_readable(b as *const c_void, n * 8))) {
        return None;
    }
    Some((0..n).map(|i| (b.add(i * 8) as *const u64).read_unaligned()).collect())
}

/// Anexa o handle `h` (já com o forte de quem o recebe) a um `DynArray<Handle<T>>`; cresce pelo reserve do motor com o mover de handles quando a lista
/// tem elementos. `false` sem capacidade, sem mexer na lista.
unsafe fn anexar_handle_no_array(arr: *mut u8, h: [u64; 2]) -> bool {
    let (_, n, cap) = dynarray_no_campo(arr, 0);
    if n + 1 > cap {
        let mover = if n == 0 { std::ptr::null() } else { crate::rebase(crate::jogo::listas::MOVER_HANDLES_VM) as *const c_void };
        crate::jogo::listas::reservar(arr as *mut c_void, (n + 1) as u32, 0x10, 8, mover);
        if dynarray_no_campo(arr, 0).2 < n + 1 {
            return false;
        }
    }
    let b = dynarray_no_campo(arr, 0).0;
    (b.add(n * 16) as *mut u64).write_unaligned(h[0]);
    (b.add(n * 16 + 8) as *mut u64).write_unaligned(h[1]);
    (arr.add(0x0c) as *mut u32).write_unaligned((n + 1) as u32);
    true
}

/// Recurso do token (`+0x28`) já carregado e legível até `tam` B, sem espera; nulo nos outros casos.
unsafe fn recurso_carregado(t: u64, tam: usize) -> *const u8 {
    if t == 0 || !crate::memoria::is_readable(t as *const c_void, 0x60) || crate::register::status_do_token(t) != 1 {
        return std::ptr::null();
    }
    let r = ((t + 0x28) as *const u64).read_unaligned() as *const u8;
    if r.is_null() || !crate::memoria::is_readable(r as *const c_void, tam) {
        return std::ptr::null();
    }
    r
}

/// `rootIndex` (i16) do `entEntityTemplate`.
const TEMPLATE_ROOTINDEX_OFF: usize = 0x1d8;

/// A#17 17d: chamado pelo gancho de extração de partes (`aparencia::extract_part_components_em`), com a `saida` já extraída pelo motor. Cada patch
/// pronto da parte é extraído pela mesma rotina; os componentes entram pela mescla de componentes e, com `entity`, a raiz do resultado é trocada
/// pela do patch, depois dos componentes. O que sobrar no array do patch é solto no fim.
pub(crate) unsafe fn aplicar_patches_na_parte(orig: *mut c_void, token: *const [u64; 2], saida: *mut [u64; 2]) {
    if token.is_null() || saida.is_null() || !crate::memoria::is_readable(token as *const c_void, 16) {
        return;
    }
    let t = *token;
    if t[0] == 0 || !crate::memoria::is_readable((t[0] + 0x48) as *const c_void, 8) {
        return;
    }
    let caminho = ((t[0] + 0x48) as *const u64).read_unaligned();
    let patches = patches_do_alvo(caminho, A17_PROP_COMPONENTS | A17_PROP_ENTITY);
    if patches.is_empty() {
        return;
    }
    let classe = CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed);
    for (pt, props) in patches {
        if recurso_do_patch_da_classe(pt, TEMPLATE_ROOTINDEX_OFF + 2, classe).is_null() {
            crate::diagnostico::A17_PATCHES_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut extraidos = [0u64; 2];
        let com_entidade = props & A17_PROP_ENTITY != 0;
        let raiz_p = if com_entidade { extrair_com_raiz(orig, pt, &mut extraidos) } else { None };
        if !com_entidade || raiz_p.is_none() {
            extract_part_components_em(orig, &pt, &mut extraidos);
        }
        let mut componentes = None;
        if props & A17_PROP_COMPONENTS != 0 {
            componentes = mesclar_componentes(saida as *mut u8, extraidos.as_mut_ptr() as *mut u8, false).map(|p| p.len());
        }
        let mut troca = None;
        if let (true, Some(rp)) = (com_entidade, raiz_p) {
            let raiz_r = raiz_do_token(t).unwrap_or(-1);
            troca = mesclar_entidade(saida as *mut u8, extraidos.as_mut_ptr() as *mut u8, raiz_r, rp);
        }
        if extraidos[0] != 0 && !soltar_dynarray_de_handles(extraidos.as_mut_ptr() as *mut u8) {
            crate::log_dev!("[a17] array extraído do patch sem o tipo pra soltar; fica retido");
        }
        crate::diagnostico::A17_PATCHES_APLICADOS.fetch_add(1, Ordering::Relaxed);
        crate::log_dev!("[a17] parte {caminho:#018x}: patch com props {props:#x}, componentes {componentes:?}, troca da raiz {troca:?}");
    }
}

pub(crate) static CLASSE_ENTENTITY: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// A troca da raiz (`entity`): `Some((raiz do resultado, raiz do patch))` só com os dois índices dentro dos arrays e o objeto raiz do patch de
/// classe derivada de `entEntity`.
pub(crate) fn plano_de_mescla_da_entidade(raiz_r: i16, tam_r: usize, raiz_p: i16, tam_p: usize, patch_derivada: bool) -> Option<(usize, usize)> {
    let dentro = |i: i16, tam: usize| i >= 0 && (i as usize) < tam;
    (dentro(raiz_r, tam_r) && dentro(raiz_p, tam_p) && patch_derivada).then_some((raiz_r as usize, raiz_p as usize))
}

/// `rootIndex` do `entEntityTemplate` do token carregado; `None` se não carregou ou é ilegível.
pub(crate) unsafe fn raiz_do_token(t: [u64; 2]) -> Option<i16> {
    let r = recurso_carregado(t[0], TEMPLATE_ROOTINDEX_OFF + 2);
    (!r.is_null()).then(|| (r.add(TEMPLATE_ROOTINDEX_OFF) as *const i16).read_unaligned())
}

/// Extração do patch que inclui o objeto raiz: a extração do motor lê o `rootIndex` uma vez no começo e apaga do bitset o bit desse objeto, então o
/// índice fica em -1 só durante a chamada e volta ao valor de antes. Devolve o `rootIndex` original; `None` com o template não carregado.
pub(crate) unsafe fn extrair_com_raiz(orig: *mut c_void, t: [u64; 2], saida: *mut [u64; 2]) -> Option<i16> {
    let r = recurso_carregado(t[0], TEMPLATE_ROOTINDEX_OFF + 2) as *mut u8;
    if r.is_null() {
        return None;
    }
    let campo = r.add(TEMPLATE_ROOTINDEX_OFF) as *mut i16;
    let raiz = campo.read_unaligned();
    if raiz >= 0 {
        campo.write_unaligned(-1);
    }
    extract_part_components_em(orig, &t, saida);
    campo.write_unaligned(raiz);
    Some(raiz)
}

/// Objeto cuja classe herda de `entEntity` sem ser `entEntity`.
pub(crate) unsafe fn entidade_derivada(obj: u64) -> bool {
    if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, 8) {
        return false;
    }
    let mut ent = CLASSE_ENTENTITY.load(Ordering::Relaxed);
    if ent.is_null() {
        ent = crate::rtti::Registry::obtain().map_or(std::ptr::null_mut(), |r| r.class_by_name("entEntity"));
        if ent.is_null() {
            return false;
        }
        CLASSE_ENTENTITY.store(ent, Ordering::Relaxed);
    }
    let cls = crate::rtti::class_of(obj as *mut c_void);
    cls != ent && crate::register::class_is_kind_of(cls, ent)
}

/// Troca a raiz do resultado pela do patch (`plano_de_mescla_da_entidade`): os dois handles trocam de lugar e o antigo sai com o array do patch.
pub(crate) unsafe fn mesclar_entidade(resultado: *mut u8, patch: *mut u8, raiz_r: i16, raiz_p: i16) -> Option<(usize, usize)> {
    let tam = |arr: *const u8| (arr.add(0x0c) as *const u32).read_unaligned() as usize;
    let (r, p) = (handles_do_dynarray(resultado), handles_do_dynarray(patch));
    if r.len() != tam(resultado) || p.len() != tam(patch) {
        return None;
    }
    let derivada = raiz_p >= 0 && (raiz_p as usize) < p.len() && entidade_derivada(p[raiz_p as usize][0]);
    let (i, j) = plano_de_mescla_da_entidade(raiz_r, r.len(), raiz_p, p.len(), derivada)?;
    let (rb, pb) = ((resultado as *const u64).read_unaligned() as *mut [u64; 2], (patch as *const u64).read_unaligned() as *mut [u64; 2]);
    rb.add(i).write_unaligned(p[j]);
    pb.add(j).write_unaligned(r[i]);
    Some((i, j))
}

// ===== A#51 51a/51b: `entEntityTemplate` =====
// Layout (2.5, Mac, conferido pelo RTTI na carga da tabela e pelo `FindAppearance` `0x1019c5e2c`): caminho do recurso `+0x30`; `appearances`
// `DynArray<entTemplateAppearance>` de 0x18 B em `+0x50` (tamanho em `+0x5c`); `resolvedDependencies` `DynArray` de 8 B em `+0x270`.
pub(crate) const TEMPLATE_APPEARANCES_OFF: usize = 0x50;

pub(crate) const TEMPLATE_DEPENDENCIES_OFF: usize = 0x270;

pub(crate) const TEMPLATE_APPEARANCE_TAM: usize = 0x18;

pub(crate) static TEMPLATE_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

/// 51b: `visualTagsSchema` (`Handle<entVisualTagsSchema>`) no template e o `visualTags` (`TagList` = `DynArray<CName>`) no schema.
pub(crate) const TEMPLATE_VISUALTAGSSCHEMA_OFF: usize = 0x98;

pub(crate) const SCHEMA_VISUALTAGS_OFF: usize = 0x30;

pub(crate) static SCHEMA_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

/// Para cada nome do patch, em ordem: `Substitui(i)` com a 1ª ocorrência do nome (nos existentes ou entre os anexados antes nesta chamada), senão
/// `Anexa`, e o nome passa a existir no índice seguinte ao fim.
pub(crate) fn plano_por_nome(existentes: &[u64], patch: &[u64]) -> Vec<PassoDaMescla> {
    let mut nomes = existentes.to_vec();
    patch
        .iter()
        .map(|&nome| match nomes.iter().position(|&n| n == nome) {
            Some(i) => PassoDaMescla::Substitui(i),
            None => {
                nomes.push(nome);
                PassoDaMescla::Anexa
            }
        })
        .collect()
}

/// `true` para o valor do patch que ainda não está nos existentes nem foi marcado antes nesta chamada.
pub(crate) fn plano_de_novos(existentes: &[u64], patch: &[u64]) -> Vec<bool> {
    let mut vistos = existentes.to_vec();
    patch
        .iter()
        .map(|&v| {
            let novo = !vistos.contains(&v);
            if novo {
                vistos.push(v);
            }
            novo
        })
        .collect()
}

/// Aplica `plano` num `DynArray` de elementos sem dono de `passo` B em `alvo`; `patch` tem um elemento por passo do plano. `Substitui(i)` copia o
/// elemento do patch por cima do `i` (que pode ser um anexado antes neste plano), `Anexa` põe no fim, `Pula` não faz nada. Cresce pelo reserve do
/// motor; sem capacidade devolve `false` com o alvo intacto.
pub(crate) unsafe fn aplicar_plano_no_array(alvo: *mut u8, patch: *const u8, passo: usize, plano: &[PassoDaMescla]) -> bool {
    let (_, tam, cap) = dynarray_no_campo(alvo, 0);
    let anexas = plano.iter().filter(|p| matches!(p, PassoDaMescla::Anexa)).count();
    let final_ = tam + anexas;
    if final_ > cap {
        crate::jogo::listas::reservar(alvo as *mut c_void, final_ as u32, passo as u32, 8, std::ptr::null());
        if dynarray_no_campo(alvo, 0).2 < final_ {
            return false;
        }
    }
    let b = dynarray_no_campo(alvo, 0).0;
    let mut atual = tam;
    for (j, p) in plano.iter().enumerate() {
        let origem = patch.add(j * passo);
        match *p {
            PassoDaMescla::Substitui(i) if i < atual => std::ptr::copy(origem, b.add(i * passo), passo),
            PassoDaMescla::Anexa => {
                std::ptr::copy(origem, b.add(atual * passo), passo);
                atual += 1;
            }
            _ => {}
        }
    }
    (alvo.add(0x0c) as *mut u32).write_unaligned(atual as u32);
    true
}

/// Resumo de uma lista mesclada para o log: `"<campo> <antes> → <depois> (ok|sem capacidade); "`.
fn resumo_da_lista(campo: &str, antes: usize, depois: usize, ok: bool) -> String {
    format!("{campo} {antes} → {depois} ({}); ", if ok { "ok" } else { "sem capacidade" })
}

/// Junta numa lista de 8 B do alvo (`alvo + off`) os valores da do patch (`patch + off`) que ela não tem; com `no_depot`, só os caminhos que o
/// depot tem. Devolve `(antes, depois, ok)`, ou `None` com uma das listas ilegível.
unsafe fn juntar_lista_de_u64(alvo: *mut u8, patch: *const u8, off: usize, no_depot: Option<&str>) -> Option<(usize, usize, bool)> {
    let (existentes, novos) = (lista_de_u64(alvo, off)?, lista_de_u64(patch, off)?);
    let plano: Vec<PassoDaMescla> = plano_de_novos(&existentes, &novos)
        .into_iter()
        .zip(&novos)
        .map(|(novo, &v)| match (novo, no_depot) {
            (true, Some(onde)) if !existe_no_depot_ou_conta(v, onde) => PassoDaMescla::Pula,
            (true, _) => PassoDaMescla::Anexa,
            (false, _) => PassoDaMescla::Pula,
        })
        .collect();
    if !plano.iter().any(|p| matches!(p, PassoDaMescla::Anexa)) {
        return Some((existentes.len(), existentes.len(), true));
    }
    let ok = aplicar_plano_no_array(alvo.add(off), novos.as_ptr() as *const u8, 8, &plano);
    Some((existentes.len(), dynarray_no_campo(alvo, off).1, ok))
}

/// Offsets de `appearances`/`resolvedDependencies` do `entEntityTemplate` pelo RTTI, conferidos contra as constantes (game thread, na carga da tabela).
pub(crate) unsafe fn guardar_offsets_do_template(reg: &crate::rtti::Registry) {
    let cls = reg.class_by_name("entEntityTemplate");
    let off = |campo: &str| if cls.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls, campo).map(|(o, _, _)| o as usize) };
    let (a, d) = (off("appearances"), off("resolvedDependencies"));
    let ok = a == Some(TEMPLATE_APPEARANCES_OFF) && d == Some(TEMPLATE_DEPENDENCIES_OFF);
    TEMPLATE_OFFSETS_OK.store(ok, Ordering::Relaxed);
    // 51b: flag própria, pra um offset errado das tags não desligar a mescla de aparências.
    let schema = off("visualTagsSchema");
    let cls_schema = reg.class_by_name("entVisualTagsSchema");
    CLASSE_VISUALTAGSSCHEMA.store(cls_schema, Ordering::Relaxed);
    let tags = if cls_schema.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls_schema, "visualTags").map(|(o, _, _)| o as usize) };
    let tags_ok = schema == Some(TEMPLATE_VISUALTAGSSCHEMA_OFF) && tags == Some(SCHEMA_VISUALTAGS_OFF);
    SCHEMA_OFFSETS_OK.store(tags_ok, Ordering::Relaxed);
    crate::veredito_dev!(
        "[a51] offsets do entEntityTemplate pelo RTTI: appearances {:?}, resolvedDependencies {:?}; mescla {}; visualTagsSchema {:?}, visualTags do schema {:?}; tags {}",
        a.map(|o| format!("{o:#x}")),
        d.map(|o| format!("{o:#x}")),
        if ok { "ligada" } else { "desligada" },
        schema.map(|o| format!("{o:#x}")),
        tags.map(|o| format!("{o:#x}")),
        if tags_ok { "ligadas" } else { "desligadas" }
    );
}

/// A#51 51b: as `visualTags` do schema do template de patch que o schema do alvo não tem entram nele. Alvo sem schema ganha um
/// `entVisualTagsSchema` novo antes; patch sem schema ou sem tags não muda nada. Devolve o resumo pro log.
pub(crate) unsafe fn mesclar_tags_do_schema(alvo: *mut u8, ptpl: *const u8) -> String {
    let schema_do = |tpl: *const u8| (tpl.add(TEMPLATE_VISUALTAGSSCHEMA_OFF) as *const u64).read_unaligned() as *mut u8;
    let pschema = schema_do(ptpl);
    if pschema.is_null() || !crate::memoria::is_readable(pschema as *const c_void, SCHEMA_VISUALTAGS_OFF + 0x10) {
        return String::new();
    }
    let Some(tags) = lista_de_u64(pschema, SCHEMA_VISUALTAGS_OFF) else { return "visualTags do patch ilegíveis; ".into() };
    if tags.is_empty() {
        return String::new();
    }
    let mut schema = schema_do(alvo);
    if schema.is_null() {
        let cls = CLASSE_VISUALTAGSSCHEMA.load(Ordering::Relaxed);
        let obj = crate::rtti::create_instance(cls);
        if obj.is_null() {
            return "schema novo não criado; ".into();
        }
        crate::rtti::make_handle(alvo.add(TEMPLATE_VISUALTAGSSCHEMA_OFF) as *mut c_void, obj);
        schema = schema_do(alvo);
        crate::diagnostico::A51B_SCHEMAS_CRIADOS.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = crate::diagnostico::A51B_CRIADOS.try_lock() {
            if g.len() < 16 {
                g.push(alvo as u64);
            }
        }
    }
    if schema.is_null() || !crate::memoria::is_readable(schema as *const c_void, SCHEMA_VISUALTAGS_OFF + 0x10) {
        return "schema do alvo ilegível; ".into();
    }
    match juntar_lista_de_u64(schema, pschema, SCHEMA_VISUALTAGS_OFF, None) {
        Some((antes, depois, ok)) => resumo_da_lista("visualTags", antes, depois, ok),
        None => "visualTags do alvo ilegíveis; ".into(),
    }
}

/// A#51 51a, no `PostLoad` do `entEntityTemplate` (thread de job), antes da original: `appearances` por nome (struct inteira), `resolvedDependencies`
/// que o alvo não tem e que o depot tem, e as tags do schema (51b).
pub(crate) unsafe fn mesclar_patches_no_template(tpl: *mut c_void) {
    if !TEMPLATE_OFFSETS_OK.load(Ordering::Relaxed) || tpl.is_null() {
        return;
    }
    let alvo = tpl as *mut u8;
    let fim = TEMPLATE_DEPENDENCIES_OFF + 0x10;
    if !crate::memoria::is_readable(alvo as *const c_void, fim) {
        return;
    }
    let caminho = (alvo.add(0x30) as *const u64).read_unaligned();
    let patches = patches_do_alvo(caminho, A17_PROP_APPEARANCES | A17_PROP_DEPENDENCIES | A17_PROP_VISUALTAGS);
    let classe = CLASSE_ENTENTITYTEMPLATE.load(Ordering::Relaxed);
    for (pt, props) in patches {
        let ptpl = recurso_do_patch_da_classe(pt, fim, classe);
        if ptpl.is_null() {
            crate::diagnostico::A51_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut resumo = String::new();
        if props & A17_PROP_APPEARANCES != 0 {
            let nomes_de = |obj: *const u8| -> Option<Vec<u64>> {
                let (b, n, _) = dynarray_no_campo(obj, TEMPLATE_APPEARANCES_OFF);
                if n > 0x1_0000 || (n > 0 && !crate::memoria::is_readable(b as *const c_void, n * TEMPLATE_APPEARANCE_TAM)) {
                    return None;
                }
                Some((0..n).map(|i| (b.add(i * TEMPLATE_APPEARANCE_TAM) as *const u64).read_unaligned()).collect())
            };
            match (nomes_de(alvo), nomes_de(ptpl)) {
                (Some(existentes), Some(novos)) if !novos.is_empty() => {
                    let plano = plano_por_nome(&existentes, &novos);
                    let pbuf = dynarray_no_campo(ptpl, TEMPLATE_APPEARANCES_OFF).0;
                    let ok = aplicar_plano_no_array(alvo.add(TEMPLATE_APPEARANCES_OFF), pbuf, TEMPLATE_APPEARANCE_TAM, &plano);
                    resumo += &resumo_da_lista("appearances", existentes.len(), dynarray_no_campo(alvo, TEMPLATE_APPEARANCES_OFF).1, ok);
                }
                (Some(_), Some(_)) => {}
                _ => resumo += "appearances ilegíveis; ",
            }
        }
        if props & A17_PROP_DEPENDENCIES != 0 {
            match juntar_lista_de_u64(alvo, ptpl, TEMPLATE_DEPENDENCIES_OFF, Some("resolvedDependencies do template")) {
                Some((antes, depois, ok)) => resumo += &resumo_da_lista("resolvedDependencies", antes, depois, ok),
                None => resumo += "resolvedDependencies ilegíveis; ",
            }
        }
        if props & A17_PROP_VISUALTAGS != 0 && SCHEMA_OFFSETS_OK.load(Ordering::Relaxed) {
            resumo += &mesclar_tags_do_schema(alvo, ptpl);
        }
        crate::diagnostico::A51_APLICADOS.fetch_add(1, Ordering::Relaxed);
        crate::log_dev!("[a51] template {caminho:#018x}: {resumo}");
    }
}

/// Teto da espera por um patch que ainda carrega, dentro do gancho de carga do alvo. Nos boots 119-123 meio segundo bastou.
const ESPERA_DO_PATCH_MS: u64 = 500;
const PASSO_DA_ESPERA_MS: u64 = 5;

/// Recurso do token do patch pronto (recurso em `token+0x28`, legível até `tam` B), ou nulo. Token carregando espera até `ESPERA_DO_PATCH_MS` e
/// desiste; token que falhou dá nulo.
pub(crate) unsafe fn recurso_do_patch_pronto(pt: [u64; 2], tam: usize) -> *const u8 {
    let t = pt[0];
    if t == 0 || !crate::memoria::is_readable(t as *const c_void, 0x60) {
        return std::ptr::null();
    }
    let mut esperado = 0;
    while crate::register::status_do_token(t) == 0 && esperado < ESPERA_DO_PATCH_MS {
        std::thread::sleep(std::time::Duration::from_millis(PASSO_DA_ESPERA_MS));
        esperado += PASSO_DA_ESPERA_MS;
    }
    recurso_carregado(t, tam)
}

pub(crate) static REFERENCIAS_FORA_DO_DEPOT: AtomicU64 = AtomicU64::new(0);

pub(crate) static RECURSOS_DE_CLASSE_ERRADA: AtomicU64 = AtomicU64::new(0);

pub(crate) static CLASSE_ENTENTITYTEMPLATE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static CLASSE_APPEARANCERESOURCE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static CLASSE_MORPHTARGETMESH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// O depot tem o caminho (`CheckResource` do motor, `0x10151aa2c`). Sem depot capturado ou caminho 0 conta como existente.
pub(crate) unsafe fn existe_no_depot(caminho: u64) -> bool {
    let depot = crate::arquivos_do_jogo::PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() || caminho == 0 {
        return true;
    }
    let f: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(crate::rebase(0x1_0151_aa2c));
    f(depot, caminho) != 0
}

/// `existe_no_depot` que conta e loga (8 vezes) a referência pulada.
pub(crate) unsafe fn existe_no_depot_ou_conta(caminho: u64, onde: &str) -> bool {
    let existe = existe_no_depot(caminho);
    if !existe && crate::primeiras(&REFERENCIAS_FORA_DO_DEPOT, 8) {
        crate::veredito_dev!("[a51] {onde}: {caminho:#018x} não existe no depot; fica de fora");
    }
    existe
}

/// `recurso_do_patch_pronto` que também exige a classe `cls` (herança pelo RTTI); `cls` nulo não confere. Classe errada dá nulo e é contada.
pub(crate) unsafe fn recurso_do_patch_da_classe(pt: [u64; 2], tam: usize, cls: *mut c_void) -> *const u8 {
    let r = recurso_do_patch_pronto(pt, tam);
    if r.is_null() || cls.is_null() {
        return r;
    }
    if !crate::register::class_is_kind_of(crate::rtti::class_of(r as *mut c_void), cls) {
        if crate::primeiras(&RECURSOS_DE_CLASSE_ERRADA, 8) {
            crate::veredito_dev!("[a51] recurso de patch {r:p} não é da classe esperada; fica de fora");
        }
        return std::ptr::null();
    }
    r
}

pub(crate) const APPRESOURCE_CENSORSHIPMAPPING_OFF: usize = 0xA8;

pub(crate) const APPDEF_DEPENDENCIES_OFF: usize = 0x1B8;

pub(crate) static APP_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static PATCHES_DE_APP_APLICADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static PATCHES_DE_APP_NAO_PRONTOS: AtomicU64 = AtomicU64::new(0);

/// Passo de uma definição do patch (51c).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PassoDaDefinicao {
    /// Junta as partes nas definições do alvo destes índices (todas com nome `None`, a 1ª com o mesmo nome; vazio se um patch anterior anexou o nome).
    Modifica(Vec<usize>),
    /// Vai pro fim como definição nova.
    Nova,
}

/// Plano das definições do patch sobre os nomes do alvo (`0` = nome `None`): `None` modifica todas (inclusive as anexadas antes nesta chamada);
/// nome que um patch anterior do mesmo recurso anexou (`anexados`) não modifica nenhuma; nome existente modifica a 1ª ocorrência; nome novo é
/// `Nova`, vai pro fim dos nomes e pra `anexados`.
pub(crate) fn plano_das_definicoes(existentes: &[u64], patch: &[u64], anexados: &mut Vec<u64>) -> Vec<PassoDaDefinicao> {
    let mut nomes = existentes.to_vec();
    patch
        .iter()
        .map(|&nome| {
            if nome == 0 {
                return PassoDaDefinicao::Modifica((0..nomes.len()).collect());
            }
            match nomes.iter().position(|&n| n == nome) {
                Some(_) if anexados.contains(&nome) => PassoDaDefinicao::Modifica(Vec::new()),
                Some(i) => PassoDaDefinicao::Modifica(vec![i]),
                None => {
                    nomes.push(nome);
                    anexados.push(nome);
                    PassoDaDefinicao::Nova
                }
            }
        })
        .collect()
}

/// Offsets do `appearanceAppearanceResource` e da `appearanceAppearanceDefinition` pelo RTTI, conferidos contra as constantes (game thread, na carga da tabela).
pub(crate) unsafe fn guardar_offsets_do_app(reg: &crate::rtti::Registry) {
    let prop = |classe: &str, campo: &str| {
        let c = reg.class_by_name(classe);
        if c.is_null() { None } else { crate::rtti::resolve_prop_in_class(c, campo).map(|(o, _, _)| o as usize) }
    };
    let lidos = [
        ("appearances", prop("appearanceAppearanceResource", "appearances"), APPRESOURCE_APPEARANCES_OFF),
        ("censorshipMapping", prop("appearanceAppearanceResource", "censorshipMapping"), APPRESOURCE_CENSORSHIPMAPPING_OFF),
        ("name", prop("appearanceAppearanceDefinition", "name"), APPDEF_NAME_OFF),
        ("partsValues", prop("appearanceAppearanceDefinition", "partsValues"), APPDEF_PARTSVALUES_OFF),
        ("visualTags", prop("appearanceAppearanceDefinition", "visualTags"), APPDEF_VISUALTAGS_OFF),
        ("resolvedDependencies", prop("appearanceAppearanceDefinition", "resolvedDependencies"), APPDEF_DEPENDENCIES_OFF),
        ("partsOverrides", prop("appearanceAppearanceDefinition", "partsOverrides"), APPDEF_PARTSOVERRIDES_OFF),
    ];
    let ok = lidos.iter().all(|(_, lido, esperado)| *lido == Some(*esperado));
    APP_OFFSETS_OK.store(ok, Ordering::Relaxed);
    let texto: Vec<String> = lidos.iter().map(|(nome, lido, _)| format!("{nome} {}", lido.map_or("?".to_string(), |o| format!("{o:#x}")))).collect();
    crate::veredito_dev!("[a51c] offsets pelo RTTI: {}; mescla {}", texto.join(", "), if ok { "ligada" } else { "desligada" });
}

/// A#51 51c: uma definição do patch (`pd`) numa definição do alvo (`d`), conforme `props`; devolve quantos entraram de `partsValues`,
/// `resolvedDependencies`, `visualTags` e `partsOverrides`.
pub(crate) unsafe fn mesclar_definicao(d: *mut u8, pd: *const u8, props: u32) -> [usize; 4] {
    let mut n = [0usize; 4];
    if d as *const u8 == pd {
        return n;
    }
    let entraram = |r: Option<(usize, usize, bool)>| r.map_or(0, |(antes, depois, _)| depois.saturating_sub(antes));
    if props & A17_PROP_PARTSVALUES != 0 {
        n[0] = entraram(juntar_lista_de_u64(d, pd, APPDEF_PARTSVALUES_OFF, Some("partsValues da definição")));
    }
    if props & A17_PROP_DEPENDENCIES != 0 {
        n[1] = entraram(juntar_lista_de_u64(d, pd, APPDEF_DEPENDENCIES_OFF, Some("resolvedDependencies da definição")));
    }
    if props & A17_PROP_VISUALTAGS != 0 {
        n[2] = entraram(juntar_lista_de_u64(d, pd, APPDEF_VISUALTAGS_OFF, None));
    }
    if props & A17_PROP_PARTSOVERRIDES != 0 {
        n[3] = copiar_parts_overrides(d, pd);
    }
    n
}

/// `partsOverrides`: elemento de 0x18 B, `partResource` (8 B) e a lista interna de 0x50 B, alinhada a 0x10, em `+0x08`.
const PARTSOVERRIDE_TAM: usize = 0x18;
const COMPONENTOVERRIDE_TAM: usize = 0x50;

/// A#51 51c-4: todas as `partsOverrides` da definição do patch anexadas à do alvo como cópias fundas (lista interna com buffer próprio). Item com
/// lista interna ilegível é pulado. Devolve quantas entraram.
pub(crate) unsafe fn copiar_parts_overrides(d: *mut u8, pd: *const u8) -> usize {
    let (pb, pn, _) = dynarray_no_campo(pd, APPDEF_PARTSOVERRIDES_OFF);
    if pn == 0 || pn > 0x1_0000 || !crate::memoria::is_readable(pb as *const c_void, pn * PARTSOVERRIDE_TAM) {
        return 0;
    }
    let arr = d.add(APPDEF_PARTSOVERRIDES_OFF);
    let (_, n, cap) = dynarray_no_campo(d, APPDEF_PARTSOVERRIDES_OFF);
    if n + pn > cap {
        crate::jogo::listas::reservar(arr as *mut c_void, (n + pn) as u32, PARTSOVERRIDE_TAM as u32, 8, std::ptr::null());
        if dynarray_no_campo(d, APPDEF_PARTSOVERRIDES_OFF).2 < n + pn {
            return 0;
        }
    }
    let b = dynarray_no_campo(d, APPDEF_PARTSOVERRIDES_OFF).0;
    let mut atual = n;
    for j in 0..pn {
        let (src, dst) = (pb.add(j * PARTSOVERRIDE_TAM), b.add(atual * PARTSOVERRIDE_TAM));
        let (ib, inn, _) = dynarray_no_campo(src, 8);
        if inn > 0x1_0000 || (inn > 0 && !crate::memoria::is_readable(ib as *const c_void, inn * COMPONENTOVERRIDE_TAM)) {
            continue;
        }
        std::ptr::write_bytes(dst, 0, PARTSOVERRIDE_TAM);
        (dst as *mut u64).write_unaligned((src as *const u64).read_unaligned());
        if !copiar_dynarray(dst.add(8), src.add(8), COMPONENTOVERRIDE_TAM, 0x10) {
            continue;
        }
        if atual == n {
            if let Ok(mut a) = crate::diagnostico::A51C4_AMOSTRA.try_lock() {
                *a = Some([d as u64, atual as u64, dynarray_no_campo(dst, 8).0 as u64, pd as u64, j as u64]);
            }
        }
        atual += 1;
        (arr.add(0x0c) as *mut u32).write_unaligned(atual as u32);
    }
    atual - n
}

/// Handles de definições (`DynArray<Handle<AppearanceDefinition>>` em `recurso + 0x98`), com cada definição legível até as dependências; `None` se algo não é.
pub(crate) unsafe fn definicoes_do_app(recurso: *const u8) -> Option<Vec<[u64; 2]>> {
    let (b, n, _) = dynarray_no_campo(recurso, APPRESOURCE_APPEARANCES_OFF);
    if n > 0 && !crate::memoria::is_readable(b as *const c_void, n * 16) {
        return None;
    }
    let hs: Vec<[u64; 2]> = (0..n).map(|i| [(b.add(i * 16) as *const u64).read_unaligned(), (b.add(i * 16 + 8) as *const u64).read_unaligned()]).collect();
    hs.iter().all(|h| h[0] != 0 && h[1] != 0 && crate::memoria::is_readable(h[0] as *const c_void, APPDEF_DEPENDENCIES_OFF + 0x10)).then_some(hs)
}

/// A#51 51c: definições (`appearances`) do recurso do patch no recurso alvo; devolve o resumo pro log.
pub(crate) unsafe fn mesclar_definicoes(alvo: *mut u8, pres: *const u8, props: u32, anexados: &mut Vec<u64>) -> String {
    let (Some(mut defs), Some(pdefs)) = (definicoes_do_app(alvo), definicoes_do_app(pres)) else { return "definições ilegíveis; ".into() };
    let nome = |h: &[u64; 2]| ((h[0] as *const u8).add(APPDEF_NAME_OFF) as *const u64).read_unaligned();
    let existentes: Vec<u64> = defs.iter().map(nome).collect();
    let novos: Vec<u64> = pdefs.iter().map(nome).collect();
    let plano = plano_das_definicoes(&existentes, &novos, anexados);
    let arr = alvo.add(APPRESOURCE_APPEARANCES_OFF);
    let (mut novas, mut sem_capacidade, mut modificadas, mut itens) = (0usize, 0usize, 0usize, [0usize; 4]);
    for (passo, ph) in plano.iter().zip(&pdefs) {
        match passo {
            PassoDaDefinicao::Nova => {
                // O alvo passa a dividir a definição do patch: o forte é do array do alvo.
                crate::register::handle_strong_soma(ph, 1);
                if anexar_handle_no_array(arr, *ph) {
                    defs.push(*ph);
                    novas += 1;
                } else {
                    crate::register::handle_strong_soma(ph, -1);
                    sem_capacidade += 1;
                }
            }
            PassoDaDefinicao::Modifica(indices) => {
                for &i in indices {
                    let Some(d) = defs.get(i) else { continue };
                    let n = mesclar_definicao(d[0] as *mut u8, ph[0] as *const u8, props);
                    for k in 0..4 {
                        itens[k] += n[k];
                    }
                    modificadas += 1;
                }
            }
        }
    }
    format!(
        "definições {} → {} ({novas} nova(s), {modificadas} mesclada(s), {sem_capacidade} sem capacidade; partsValues +{}, resolvedDependencies +{}, visualTags +{}, partsOverrides +{}); ",
        existentes.len(),
        defs.len(),
        itens[0],
        itens[1],
        itens[2],
        itens[3]
    )
}

/// `censorshipMapping`: elemento de 0x18 B.
const CENSORSHIPMAPPING_TAM: usize = 0x18;

/// A#51 51c, no `PostLoad` do `appearanceAppearanceResource` (thread de job), antes da original. `anexados` é um por recurso alvo, dividido entre os
/// patches dele.
pub(crate) unsafe fn mesclar_patches_no_app(res: *mut c_void) {
    if !APP_OFFSETS_OK.load(Ordering::Relaxed) || res.is_null() {
        return;
    }
    let alvo = res as *mut u8;
    let fim = (APPRESOURCE_APPEARANCES_OFF.max(APPRESOURCE_CENSORSHIPMAPPING_OFF)) + 0x10;
    if !crate::memoria::is_readable(alvo as *const c_void, fim) {
        return;
    }
    let caminho = (alvo.add(0x30) as *const u64).read_unaligned();
    let mascara = A17_PROP_APPEARANCES | A17_PROP_CENSORSHIP;
    let patches = patches_do_alvo(caminho, mascara);
    let classe = CLASSE_APPEARANCERESOURCE.load(Ordering::Relaxed);
    let mut anexados = Vec::new();
    for (pt, props) in patches {
        let pres = recurso_do_patch_da_classe(pt, fim, classe);
        if pres.is_null() {
            PATCHES_DE_APP_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let mut resumo = String::new();
        if props & A17_PROP_APPEARANCES != 0 {
            resumo += &mesclar_definicoes(alvo, pres, props, &mut anexados);
        }
        if props & A17_PROP_CENSORSHIP != 0 {
            let (pb, pn, _) = dynarray_no_campo(pres, APPRESOURCE_CENSORSHIPMAPPING_OFF);
            if pn > 0 && pn <= 0x1_0000 && crate::memoria::is_readable(pb as *const c_void, pn * CENSORSHIPMAPPING_TAM) {
                let antes = dynarray_no_campo(alvo, APPRESOURCE_CENSORSHIPMAPPING_OFF).1;
                let plano = vec![PassoDaMescla::Anexa; pn];
                let ok = aplicar_plano_no_array(alvo.add(APPRESOURCE_CENSORSHIPMAPPING_OFF), pb, CENSORSHIPMAPPING_TAM, &plano);
                resumo += &resumo_da_lista("censorshipMapping", antes, dynarray_no_campo(alvo, APPRESOURCE_CENSORSHIPMAPPING_OFF).1, ok);
            }
        }
        PATCHES_DE_APP_APLICADOS.fetch_add(1, Ordering::Relaxed);
        crate::log_dev!("[a51c] recurso {caminho:#018x}: {resumo}");
    }
}

// ===== A#51 51d: `MorphTargetMesh` =====
// `PostLoad` do `MorphTargetMesh` em `0x1051ca1a4`. Campos sem dono: `baseMeshAppearance` (CName), `baseTextureParamName` (CName), `boundingBox` (`Box`
// 0x20 B: Min `+0x00`, Max `+0x10`). Offsets pelo RTTI na carga da tabela (`guardar_offsets_do_morph`).
pub(crate) const PROPS_DO_MORPH: u32 = A17_PROP_BASEMESHAPP | A17_PROP_BASETEXTUREPARAM | A17_PROP_BOUNDINGBOX | PROPS_DO_MORPH_COM_DONO;

pub(crate) const BOX_TAM: usize = 0x20;

pub(crate) static MORPH_BASEMESHAPPEARANCE_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_BASETEXTUREPARAMNAME_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_BOUNDINGBOX_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static PATCHES_DE_MORPH_APLICADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static PATCHES_DE_MORPH_NAO_PRONTOS: AtomicU64 = AtomicU64::new(0);

// 51d v2: `baseMesh` e `baseTexture` (`Ref` = `ResourceReference`: caminho `+0x00`, token `+0x08`, contador `+0x10`) e `targets`
// (`MorphTargetMeshEntry` 0x38 B: nome `+0x00`, nomes e região até `+0x18`, `boneNames` `DynArray<CName>` `+0x18`, `boneRigMatrices` `DynArray<Matrix>`
// de 0x40 B alinhada a 0x10 em `+0x28`).
pub(crate) const PROPS_DO_MORPH_COM_DONO: u32 = A17_PROP_BASEMESH | A17_PROP_BASETEXTURE | A17_PROP_TARGETS | A17_PROP_BLOB;

pub(crate) const MORPHTARGETMESHENTRY_TAM: usize = 0x38;

pub(crate) static MORPH_BASEMESH_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_BASETEXTURE_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_TARGETS_OFF: AtomicU64 = AtomicU64::new(0);

pub(crate) static MORPH_COM_DONO_OFFSETS_OK: AtomicBool = AtomicBool::new(false);

pub(crate) static MORPH_COM_DONO_LOGADOS: AtomicU64 = AtomicU64::new(0);

/// 51d v2: `(baseMesh, baseTexture, targets)` mudam ou não.
pub(crate) fn plano_do_morph_com_dono(props: u32, mesh_patch: u64, tex_patch: u64) -> (bool, bool, bool) {
    let todas = props == u32::MAX;
    let ligada = |bit: u32| todas || props & bit != 0;
    let malha = mesh_patch != 0 && ligada(A17_PROP_BASEMESH);
    let textura = if todas { tex_patch != 0 } else { props & A17_PROP_BASETEXTURE != 0 };
    (malha, textura, ligada(A17_PROP_TARGETS))
}

/// Copia o `Ref` do patch pro alvo subindo o strong do token; token morrendo (strong 0) vai nulo, só com o caminho.
pub(crate) unsafe fn copiar_ref(dst: *mut u8, src: *const u8) {
    let (caminho, token, contador) =
        ((src as *const u64).read_unaligned(), (src.add(8) as *const u64).read_unaligned(), (src.add(0x10) as *const u64).read_unaligned());
    let vivo = token != 0 && contador != 0 && travar_fraco(contador);
    (dst as *mut u64).write_unaligned(caminho);
    (dst.add(8) as *mut u64).write_unaligned(if vivo { token } else { 0 });
    (dst.add(0x10) as *mut u64).write_unaligned(if vivo { contador } else { 0 });
}

/// `DynArray` de `elem` B (cabeçalho em `arr`) com o conteúdo de `origem`, reserve do motor com `alin`; `false` sem capacidade ou origem ilegível.
pub(crate) unsafe fn copiar_dynarray(arr: *mut u8, origem: *const u8, elem: usize, alin: u32) -> bool {
    let (ob, on, _) = dynarray_no_campo(origem, 0);
    if on > 0x1_0000 || (on > 0 && !crate::memoria::is_readable(ob as *const c_void, on * elem)) {
        return false;
    }
    if on > dynarray_no_campo(arr, 0).2 {
        crate::jogo::listas::reservar(arr as *mut c_void, on as u32, elem as u32, alin, std::ptr::null());
        if dynarray_no_campo(arr, 0).2 < on {
            return false;
        }
    }
    if on > 0 {
        std::ptr::copy(ob, dynarray_no_campo(arr, 0).0, on * elem);
    }
    (arr.add(0x0c) as *mut u32).write_unaligned(on as u32);
    true
}

/// `MorphTargetMeshEntry` do patch na entrada do alvo.
pub(crate) unsafe fn copiar_morph_target_entry(dst: *mut u8, src: *const u8) -> bool {
    std::ptr::copy_nonoverlapping(src, dst, 0x18);
    copiar_dynarray(dst.add(0x18), src.add(0x18), 8, 8) && copiar_dynarray(dst.add(0x28), src.add(0x28), 0x40, 0x10)
}

/// `targets` do patch no alvo, por nome (`plano_por_nome`): a entrada de mesmo nome é substituída, a nova é anexada zerada e preenchida. Lista com
/// mais de 4096 entradas não é tocada. Devolve `(substituídas, anexadas, falhas)`.
pub(crate) unsafe fn mesclar_targets_do_morph(alvo: *mut u8, pres: *const u8, off: usize) -> (usize, usize, usize) {
    let (pb, pn, _) = dynarray_no_campo(pres, off);
    let (_, n, cap) = dynarray_no_campo(alvo, off);
    if pn == 0 || pn > 4096 || n > 4096 || !crate::memoria::is_readable(pb as *const c_void, pn * MORPHTARGETMESHENTRY_TAM) {
        return (0, 0, 0);
    }
    let nomes_de = |b: *const u8, n: usize| -> Vec<u64> { (0..n).map(|i| (b.add(i * MORPHTARGETMESHENTRY_TAM) as *const u64).read_unaligned()).collect() };
    let ab = dynarray_no_campo(alvo, off).0;
    if n > 0 && !crate::memoria::is_readable(ab as *const c_void, n * MORPHTARGETMESHENTRY_TAM) {
        return (0, 0, 0);
    }
    let plano = plano_por_nome(&nomes_de(ab, n), &nomes_de(pb, pn));
    let anexas = plano.iter().filter(|p| matches!(p, PassoDaMescla::Anexa)).count();
    let arr = alvo.add(off);
    if n + anexas > cap {
        crate::jogo::listas::reservar(arr as *mut c_void, (n + anexas) as u32, MORPHTARGETMESHENTRY_TAM as u32, 8, std::ptr::null());
        if dynarray_no_campo(alvo, off).2 < n + anexas {
            return (0, 0, pn);
        }
    }
    let b = dynarray_no_campo(alvo, off).0;
    let (mut substituidas, mut anexadas, mut falhas, mut atual) = (0, 0, 0, n);
    for (j, p) in plano.iter().enumerate() {
        let src = pb.add(j * MORPHTARGETMESHENTRY_TAM);
        match *p {
            PassoDaMescla::Substitui(i) if i < atual => {
                if copiar_morph_target_entry(b.add(i * MORPHTARGETMESHENTRY_TAM), src) {
                    substituidas += 1;
                } else {
                    falhas += 1;
                }
            }
            PassoDaMescla::Anexa => {
                let dst = b.add(atual * MORPHTARGETMESHENTRY_TAM);
                std::ptr::write_bytes(dst, 0, MORPHTARGETMESHENTRY_TAM);
                if copiar_morph_target_entry(dst, src) {
                    atual += 1;
                    anexadas += 1;
                    (arr.add(0x0c) as *mut u32).write_unaligned(atual as u32);
                } else {
                    falhas += 1;
                }
            }
            _ => falhas += 1,
        }
    }
    (substituidas, anexadas, falhas)
}

/// 51d: `(baseMeshAppearance, baseTextureParamName, boundingBox)` mudam ou não. Props todas: muda o campo cujo valor no patch não é vazio (caixa com
/// `max.x > min.x`). Props explícitas: os dois nomes mudam com a prop ligada, mesmo vazios; a caixa pede a prop e `max.x > min.x`.
pub(crate) fn plano_do_morph(props: u32, app_patch: u64, param_patch: u64, box_min_x: f32, box_max_x: f32) -> (bool, bool, bool) {
    let caixa_valida = box_max_x > box_min_x;
    if props == u32::MAX {
        return (app_patch != 0, param_patch != 0, caixa_valida);
    }
    (props & A17_PROP_BASEMESHAPP != 0, props & A17_PROP_BASETEXTUREPARAM != 0, props & A17_PROP_BOUNDINGBOX != 0 && caixa_valida)
}

/// Offsets dos 3 campos do `MorphTargetMesh` pelo RTTI (game thread, na carga da tabela).
pub(crate) unsafe fn guardar_offsets_do_morph(reg: &crate::rtti::Registry) {
    let cls = reg.class_by_name("MorphTargetMesh");
    let off = |campo: &str| if cls.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls, campo).map(|(o, _, _)| o as u64) };
    let (app, param, caixa) = (off("baseMeshAppearance"), off("baseTextureParamName"), off("boundingBox"));
    let ok = matches!((app, param, caixa), (Some(a), Some(p), Some(b)) if a >= 0x30 && p >= 0x30 && b >= 0x30 && a < 0x400 && p < 0x400 && b < 0x400);
    if ok {
        MORPH_BASEMESHAPPEARANCE_OFF.store(app.unwrap_or(0), Ordering::Relaxed);
        MORPH_BASETEXTUREPARAMNAME_OFF.store(param.unwrap_or(0), Ordering::Relaxed);
        MORPH_BOUNDINGBOX_OFF.store(caixa.unwrap_or(0), Ordering::Relaxed);
    }
    MORPH_OFFSETS_OK.store(ok, Ordering::Relaxed);
    let hex = |o: Option<u64>| o.map_or("?".to_string(), |o| format!("{o:#x}"));
    crate::veredito_dev!(
        "[a51d] offsets do MorphTargetMesh pelo RTTI: baseMeshAppearance {} (SDK 0x98), baseTextureParamName {} (SDK 0x78), boundingBox {} (SDK 0x40); mescla {}",
        hex(app),
        hex(param),
        hex(caixa),
        if ok { "ligada" } else { "desligada" }
    );
    // 51d v2: offsets dos 3 campos com dono e o tamanho do `MorphTargetMeshEntry` (`+0x68` da classe), conferido contra `MORPHTARGETMESHENTRY_TAM`.
    let (mesh, tex, tg) = (off("baseMesh"), off("baseTexture"), off("targets"));
    let entrada = reg.class_by_name("MorphTargetMeshEntry") as *const u8;
    let tam_entrada = if !entrada.is_null() && crate::memoria::is_readable(entrada as *const c_void, 0x78) { (entrada.add(0x68) as *const u32).read_unaligned() } else { 0 };
    let ok2 = ok
        && matches!((mesh, tex, tg), (Some(m), Some(t), Some(g)) if m >= 0x30 && t >= 0x30 && g >= 0x30 && m < 0x400 && t < 0x400 && g < 0x400)
        && tam_entrada as usize == MORPHTARGETMESHENTRY_TAM;
    if ok2 {
        MORPH_BASEMESH_OFF.store(mesh.unwrap_or(0), Ordering::Relaxed);
        MORPH_BASETEXTURE_OFF.store(tex.unwrap_or(0), Ordering::Relaxed);
        MORPH_TARGETS_OFF.store(tg.unwrap_or(0), Ordering::Relaxed);
    }
    MORPH_COM_DONO_OFFSETS_OK.store(ok2, Ordering::Relaxed);
    crate::veredito_dev!(
        "[a51d] v2 pelo RTTI: baseMesh {} (SDK 0x60), baseTexture {} (SDK 0x80), targets {} (SDK 0xb0), MorphTargetMeshEntry {tam_entrada:#x} (SDK 0x38); v2 {}",
        hex(mesh),
        hex(tex),
        hex(tg),
        if ok2 { "ligada" } else { "desligada" }
    );
    // 51d v3: `MorphTargetMesh.blob`, os campos do `rendRenderMorphTargetMeshBlob` e os tipos do header e do `textureDiffsBuffer` (pro `Assign`).
    let blob_off = off("blob");
    let cls_mb = reg.class_by_name("rendRenderMorphTargetMeshBlob");
    let cls_meshb = reg.class_by_name("rendRenderMeshBlob");
    let campo = |nome: &str| if cls_mb.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls_mb, nome) };
    let (ph, pd, pbase, pdiff, pmap) = (campo("header"), campo("textureDiffsBuffer"), campo("baseBlob"), campo("diffsBuffer"), campo("mappingBuffer"));
    let ok3 = ok2
        && blob_off == Some(0xa0)
        && !cls_meshb.is_null()
        && matches!(ph, Some((0x40, t, _)) if crate::rtti::sane(t))
        && matches!(pd, Some((0x110, t, _)) if crate::rtti::sane(t))
        && matches!(pbase, Some((0x30, _, _)))
        && matches!(pdiff, Some((0xc0, _, _)))
        && matches!(pmap, Some((0xe8, _, _)));
    if ok3 {
        CLASSE_RENDERMORPHTARGETMESHBLOB.store(cls_mb, Ordering::Relaxed);
        TIPO_MORPH_BLOB_HEADER.store(ph.map_or(std::ptr::null_mut(), |p| p.1), Ordering::Relaxed);
        TIPO_DEFERREDDATABUFFER.store(pd.map_or(std::ptr::null_mut(), |p| p.1), Ordering::Relaxed);
        CLASSE_RENDERMESHBLOB.store(cls_meshb, Ordering::Relaxed);
    }
    MORPH_BLOB_OFFSETS_OK.store(ok3, Ordering::Relaxed);
    let o = |p: Option<(u32, *mut c_void, bool)>| p.map_or("?".to_string(), |(x, _, _)| format!("{x:#x}"));
    crate::veredito_dev!(
        "[a51d] v3 pelo RTTI: blob {} (SDK 0xa0); rendRenderMorphTargetMeshBlob header {} (SDK 0x40), textureDiffsBuffer {} (SDK 0x110), baseBlob {} (SDK 0x30), diffsBuffer {} (SDK 0xc0), mappingBuffer {} (SDK 0xe8); v3 {}",
        hex(blob_off),
        o(ph),
        o(pd),
        o(pbase),
        o(pdiff),
        o(pmap),
        if ok3 { "ligada" } else { "desligada" }
    );
}

/// `MorphTargetMesh.blob` (`Handle<rendRenderMorphTargetMeshBlob>`).
const MORPH_BLOB_OFF: usize = 0xa0;

/// A#51 51d, no `PostLoad` do `MorphTargetMesh` (thread de job), antes da original: os 3 campos sem dono (`plano_do_morph`), o blob novo (v3) e os
/// campos com dono (v2, `plano_do_morph_com_dono`).
pub(crate) unsafe fn mesclar_patches_no_morph(res: *mut c_void) {
    if !MORPH_OFFSETS_OK.load(Ordering::Relaxed) || res.is_null() {
        return;
    }
    let ld = |a: &AtomicU64| a.load(Ordering::Relaxed) as usize;
    let (o_app, o_param, o_caixa) = (ld(&MORPH_BASEMESHAPPEARANCE_OFF), ld(&MORPH_BASETEXTUREPARAMNAME_OFF), ld(&MORPH_BOUNDINGBOX_OFF));
    let com_dono = MORPH_COM_DONO_OFFSETS_OK.load(Ordering::Relaxed);
    let (o_mesh, o_tex, o_tg) = (ld(&MORPH_BASEMESH_OFF), ld(&MORPH_BASETEXTURE_OFF), ld(&MORPH_TARGETS_OFF));
    let mut fim = (o_app + 8).max(o_param + 8).max(o_caixa + BOX_TAM).max(MORPH_BLOB_OFF + 0x10);
    if com_dono {
        fim = fim.max(o_mesh + 0x18).max(o_tex + 0x18).max(o_tg + 0x10);
    }
    let alvo = res as *mut u8;
    if !crate::memoria::is_readable(alvo as *const c_void, fim) {
        return;
    }
    let caminho = (alvo.add(0x30) as *const u64).read_unaligned();
    let patches = patches_do_alvo(caminho, PROPS_DO_MORPH);
    let classe = CLASSE_MORPHTARGETMESH.load(Ordering::Relaxed);
    for (pt, props) in patches {
        let pres = recurso_do_patch_da_classe(pt, fim, classe);
        if pres.is_null() {
            PATCHES_DE_MORPH_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let rd = |p: *const u8, o: usize| (p.add(o) as *const u64).read_unaligned();
        let rf = |p: *const u8, o: usize| (p.add(o) as *const f32).read_unaligned();
        let (app, param, caixa) = plano_do_morph(props, rd(pres, o_app), rd(pres, o_param), rf(pres, o_caixa), rf(pres, o_caixa + 0x10));
        if app {
            std::ptr::copy_nonoverlapping(pres.add(o_app), alvo.add(o_app), 8);
        }
        if param {
            std::ptr::copy_nonoverlapping(pres.add(o_param), alvo.add(o_param), 8);
        }
        if caixa {
            std::ptr::copy_nonoverlapping(pres.add(o_caixa), alvo.add(o_caixa), BOX_TAM);
        }
        let mut blob = "sem troca".to_string();
        if MORPH_BLOB_OFFSETS_OK.load(Ordering::Relaxed) {
            let (pb, ab) = (rd(pres, MORPH_BLOB_OFF), rd(alvo, MORPH_BLOB_OFF));
            let da_classe = pb != 0
                && crate::memoria::is_readable(pb as *const c_void, 8)
                && crate::register::class_is_kind_of(crate::rtti::class_of(pb as *mut c_void), CLASSE_RENDERMORPHTARGETMESHBLOB.load(Ordering::Relaxed));
            if morph_blob_muda(props, pb != 0, ab != 0) && da_classe {
                match novo_morph_blob(pb as *const u8) {
                    Some(novo) => {
                        let slot = alvo.add(MORPH_BLOB_OFF) as *mut [u64; 2];
                        let mut antigo = slot.read_unaligned();
                        slot.write_unaligned(novo);
                        crate::jogo::objetos::soltar_handle(&mut antigo);
                        crate::diagnostico::A51D3_COPIADOS.fetch_add(1, Ordering::Relaxed);
                        if let Ok(mut g) = crate::diagnostico::A51D3_AMOSTRAS.try_lock() {
                            if g.len() < 16 {
                                g.push([res as u64, novo[0], pb, caminho]);
                            }
                        }
                        blob = format!("blob novo {:#x} do patch {pb:#x}", novo[0]);
                    }
                    None => {
                        crate::diagnostico::A51D3_FALHAS.fetch_add(1, Ordering::Relaxed);
                        blob = "cópia do blob falhou; alvo intacto".into();
                    }
                }
            }
        }
        let mut v2 = String::new();
        if com_dono {
            let (pmesh, ptex) = (rd(pres, o_mesh), rd(pres, o_tex));
            let (amesh, atex) = (rd(alvo, o_mesh), rd(alvo, o_tex));
            let (malha, textura, alvos) = plano_do_morph_com_dono(props, pmesh, ptex);
            // O token do `Ref` antigo do alvo não é solto: a soltura de `SharedPtr<ResourceToken>` pelo motor não está medida; fica 1 forte
            // retido por troca.
            if malha {
                copiar_ref(alvo.add(o_mesh), pres.add(o_mesh));
            }
            if textura {
                copiar_ref(alvo.add(o_tex), pres.add(o_tex));
            }
            let antes = dynarray_no_campo(alvo, o_tg).1;
            let (s, a, f) = if alvos { mesclar_targets_do_morph(alvo, pres, o_tg) } else { (0, 0, 0) };
            let depois = dynarray_no_campo(alvo, o_tg).1;
            if let Ok(mut g) = crate::diagnostico::A51D2_AMOSTRAS.try_lock() {
                if g.len() < 16 {
                    g.push(crate::diagnostico::A51d2Amostra {
                        alvo: res as u64,
                        patch: pres as u64,
                        mesh: (amesh, pmesh, malha),
                        tex: (atex, ptex, textura),
                        targets: (antes, depois, s, a, f, alvos),
                    });
                }
            }
            v2 = format!("; baseMesh {malha}, baseTexture {textura}, targets {antes} → {depois} ({s} substituída(s), {a} anexada(s), {f} falha(s))");
        }
        PATCHES_DE_MORPH_APLICADOS.fetch_add(1, Ordering::Relaxed);
        crate::log_dev!("[a51d] morph {caminho:#018x}: baseMeshAppearance {app}, baseTextureParamName {param}, boundingBox {caixa}; {blob}{v2}");
    }
}

// ===== A#51 51e: `worldStreamingWorld` =====
// `PostLoad` do `worldStreamingWorld` em `0x101c09aa8` (resolvido pelo `postloadprobe`, boot 102). Desmontado, ele anda o `blockRefs` (array de 0x18 B em
// `+0x60` no Mac: caminho `+0x00`, `SharedPtr` do token `+0x08`/`+0x10`), resolve cada ref, confere a classe `worldStreamingBlock` e anexa o handle no
// array de 0x10 B logo depois (`+0x70`). Offset do `blockRefs` pelo RTTI na carga da tabela (`guardar_offsets_do_mundo`).
pub(crate) const WORLD_POSTLOAD_VM: u64 = 0x1_01c0_9aa8;

pub(crate) static ORIG_WORLD_POSTLOAD: AtomicU64 = AtomicU64::new(0);

/// Offset do `blockRefs` do `worldStreamingWorld` pelo RTTI (game thread, na carga da tabela).
pub(crate) unsafe fn guardar_offsets_do_mundo(reg: &crate::rtti::Registry) {
    let cls = reg.class_by_name("worldStreamingWorld");
    let refs = if cls.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls, "blockRefs").map(|(o, _, _)| o as u64) };
    let ok = matches!(refs, Some(o) if (0x40..0x200).contains(&o));
    if let Some(o) = refs.filter(|_| ok) {
        WORLD_BLOCKREFS_OFF.store(o, Ordering::Relaxed);
    }
    WORLD_OFFSETS_OK.store(ok, Ordering::Relaxed);
    crate::veredito_dev!(
        "[a51e] offset do worldStreamingWorld pelo RTTI: blockRefs {} (SDK 0x70, desmonte do PostLoad 0x60); mescla {}",
        refs.map_or("?".to_string(), |o| format!("{o:#x}")),
        if ok { "ligada" } else { "desligada" }
    );
}

/// `blockRefs`: elemento de 0x18 B (caminho, token, contador do `SharedPtr`).
const BLOCKREF_TAM: usize = 0x18;

/// A#51 51e, no `PostLoad` do `worldStreamingWorld` (thread de job), antes da original: cada bloco de `streaming.blocks` (patch do mundo com
/// `blocks`) que o `blockRefs` ainda não tem entra nele, com o forte do token +1. Quem resolve a ref é o resto do `PostLoad`.
pub(crate) unsafe fn mesclar_blocos_no_mundo(mundo: *mut c_void) {
    if !WORLD_OFFSETS_OK.load(Ordering::Relaxed) || mundo.is_null() {
        return;
    }
    let off = WORLD_BLOCKREFS_OFF.load(Ordering::Relaxed) as usize;
    let m = mundo as *mut u8;
    if !crate::memoria::is_readable(m as *const c_void, off + 0x10) {
        return;
    }
    let caminho = (m.add(0x30) as *const u64).read_unaligned();
    let blocos: Vec<([u64; 2], u64)> = {
        if caminho == 0 || !PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) {
            return;
        }
        let Ok(t) = PATCHES_DE_RECURSO.read() else { return };
        t.do_alvo(caminho).filter(|p| p.props & A17_PROP_BLOCKS != 0).map(|p| (p.token, p.patch)).collect()
    };
    if blocos.is_empty() {
        return;
    }
    let (mut anexados, mut presentes, mut sem_token) = (0usize, 0usize, 0usize);
    for (token, bloco) in blocos {
        if token[0] == 0 || token[1] == 0 {
            sem_token += 1;
            continue;
        }
        let (b, n, cap) = dynarray_no_campo(m, off);
        if n > 0x1_0000 || (n > 0 && !crate::memoria::is_readable(b as *const c_void, n * BLOCKREF_TAM)) {
            return;
        }
        if (0..n).any(|i| (b.add(i * BLOCKREF_TAM) as *const u64).read_unaligned() == bloco) {
            presentes += 1;
            continue;
        }
        let arr = m.add(off);
        if n + 1 > cap {
            crate::jogo::listas::reservar(arr as *mut c_void, (n + 1) as u32, BLOCKREF_TAM as u32, 8, std::ptr::null());
            if dynarray_no_campo(m, off).2 < n + 1 {
                crate::log_dev!("[a51e] blockRefs sem capacidade; bloco {bloco:#018x} fora");
                return;
            }
        }
        crate::register::handle_strong_soma(&token, 1);
        let e = dynarray_no_campo(m, off).0.add(n * BLOCKREF_TAM) as *mut u64;
        e.write_unaligned(bloco);
        e.add(1).write_unaligned(token[0]);
        e.add(2).write_unaligned(token[1]);
        (arr.add(0x0c) as *mut u32).write_unaligned((n + 1) as u32);
        anexados += 1;
    }
    crate::log_dev!("[a51e] mundo {caminho:#018x}: {anexados} bloco(s) anexado(s), {presentes} já presente(s), {sem_token} sem token");
}

/// Gancho do `PostLoad` do `worldStreamingWorld` (`WORLD_POSTLOAD_VM`): guarda o mundo pra 51j (o último visto), mescla os blocos e chama a original.
pub(crate) unsafe extern "C" fn world_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    crate::diagnostico::A51J_MUNDO.store(this as u64, Ordering::Relaxed);
    mesclar_blocos_no_mundo(this);
    let orig: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(original_publicado(&ORIG_WORLD_POSTLOAD));
    orig(this, a1, a2);
}

/// Pares `(instância nova no resultado, instância trocada que foi pro patch)` dos substituídos do plano, sobre os dois arrays já mesclados.
pub(crate) fn pares_substituidos(plano: &[PassoDaMescla], resultado: &[[u64; 2]], patch: &[[u64; 2]]) -> Vec<(u64, u64)> {
    plano
        .iter()
        .enumerate()
        .filter_map(|(j, passo)| match *passo {
            PassoDaMescla::Substitui(i) if i < resultado.len() && j < patch.len() => Some((resultado[i][0], patch[j][0])),
            _ => None,
        })
        .collect()
}

pub(crate) static TOKENS_DE_PARTE: std::sync::Mutex<Vec<(u64, [u64; 2])>> = std::sync::Mutex::new(Vec::new());

/// `CurveSet` e offset de `curves` pelo RTTI (thread do jogo, na carga da tabela). `true` com a mescla ligável.
pub(crate) unsafe fn guardar_offsets_do_curveset(reg: &crate::rtti::Registry) -> bool {
    let cls = reg.class_by_name("CurveSet");
    let curvas = if cls.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls, "curves").map(|(o, _, _)| o as u64) };
    let ok = !cls.is_null() && curvas == Some(0x40);
    if ok {
        A51G_CLASSE_CURVESET.store(cls, Ordering::Relaxed);
    }
    crate::veredito_dev!(
        "[a51g] CurveSet pelo RTTI: classe {cls:p}, curves {} (SDK 0x40); mescla {}",
        curvas.map_or("?".to_string(), |o| format!("{o:#x}")),
        if ok { "ligada" } else { "desligada" }
    );
    ok
}

pub(crate) static RESOURCE_LOAD_ATIVO: AtomicBool = AtomicBool::new(false);

pub(crate) static CLASSE_CRESOURCE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// `Resource/Load` pra cada `CResource` nos serializables (`+0x230`) do contexto.
pub(crate) unsafe fn despachar_resource_load_dos_serializables(ctx: *const u8) {
    let classe = CLASSE_CRESOURCE.load(Ordering::Relaxed);
    let (b, n, _) = dynarray_no_campo(ctx, 0x230);
    if classe.is_null() || n == 0 || n > 4096 || !crate::memoria::is_readable(b as *const c_void, n * 16) {
        return;
    }
    C122_CONTEXTOS.fetch_add(1, Ordering::Relaxed);
    for i in 0..n {
        let h = [(b.add(i * 16) as *const u64).read_unaligned(), (b.add(i * 16 + 8) as *const u64).read_unaligned()];
        if h[0] == 0 || h[1] == 0 || !crate::memoria::is_readable(h[0] as *const c_void, 0x38) {
            continue;
        }
        if crate::register::class_is_kind_of(crate::rtti::class_of(h[0] as *mut c_void), classe) {
            crate::arquivos_do_jogo::disparar_resource_load(h);
            C122_DESPACHOS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// `gameDeviceResource` e offset de `data` pelo RTTI (thread do jogo, na carga da tabela).
pub(crate) unsafe fn guardar_offsets_do_device_resource(reg: &crate::rtti::Registry) -> bool {
    let cls = reg.class_by_name("gameDeviceResource");
    let dado = if cls.is_null() { None } else { crate::rtti::resolve_prop_in_class(cls, "data").map(|(o, _, _)| o as u64) };
    let ok = !cls.is_null() && dado == Some(0x40);
    if ok {
        CLASSE_GAMEDEVICERESOURCE.store(cls, Ordering::Relaxed);
    }
    crate::veredito_dev!(
        "[a51h] gameDeviceResource pelo RTTI: classe {cls:p}, data {} (SDK 0x40); mescla {}",
        dado.map_or("?".to_string(), |o| format!("{o:#x}")),
        if ok { "ligada" } else { "desligada" }
    );
    ok
}

/// A#51 51i-1, no `PostLoad` do `CMesh`: nomes de material do `resource.fix` (`aparencia::FIXES_DE_NOME_DO_MESH`).
pub(crate) unsafe fn aplicar_fix_de_nomes(mesh: *mut c_void) {
    if !CMESH_OFFSETS_OK.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let m = mesh as *mut u8;
    if !crate::memoria::is_readable(m as *const c_void, CMESH_APPEARANCES_OFF + 0x10) {
        return;
    }
    let caminho = (m.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let pares = match FIXES_DE_NOME_DO_MESH.read() {
        Ok(g) => match g.as_ref().and_then(|t| t.get(&caminho)) {
            Some(p) if !p.is_empty() => p.clone(),
            _ => return,
        },
        Err(_) => return,
    };
    let novo_de = |nome: u64| pares.iter().find(|(antigo, _)| *antigo == nome).map(|&(_, novo)| novo);
    let mut trocados = 0u64;
    let (ab, an, _) = dynarray_no_campo(m, CMESH_APPEARANCES_OFF);
    if an <= 4096 && (an == 0 || crate::memoria::is_readable(ab as *const c_void, an * 16)) {
        for i in 0..an {
            let ap = (ab.add(i * 16) as *const u64).read_unaligned() as *mut u8;
            if ap.is_null() || !crate::memoria::is_readable(ap as *const c_void, MESHAPPEARANCE_CHUNKMATERIALS_OFF + 0x10) {
                continue;
            }
            let (cb, cn, _) = dynarray_no_campo(ap, MESHAPPEARANCE_CHUNKMATERIALS_OFF);
            if cn > 0x1_0000 || (cn > 0 && !crate::memoria::is_readable(cb as *const c_void, cn * 8)) {
                continue;
            }
            for k in 0..cn {
                let slot = cb.add(k * 8) as *mut u64;
                if let Some(novo) = novo_de(slot.read_unaligned()) {
                    slot.write_unaligned(novo);
                    trocados += 1;
                }
            }
        }
    }
    let (eb, en, _) = dynarray_no_campo(m, CMESH_MATERIALENTRIES_OFF);
    if en <= 0x1_0000 && (en == 0 || crate::memoria::is_readable(eb as *const c_void, en * CMESHMATERIALENTRY_TAMANHO)) {
        for k in 0..en {
            let slot = eb.add(k * CMESHMATERIALENTRY_TAMANHO + 0x20) as *mut u64;
            if let Some(novo) = novo_de(slot.read_unaligned()) {
                slot.write_unaligned(novo);
                trocados += 1;
            }
        }
    }
    crate::diagnostico::A51I_MESHES.fetch_add(1, Ordering::Relaxed);
    crate::diagnostico::A51I_NOMES.fetch_add(trocados, Ordering::Relaxed);
    if crate::primeiras(&crate::diagnostico::A51I_LOGADOS, 8) {
        crate::veredito_dev!("[a51i] fix de nomes no mesh {caminho:#018x}: {trocados} nome(s) trocado(s)");
    }
}

// ===== A#51 51i-2: `appearances` do `CMesh` =====
// Layout do `CMesh` (2.5, Mac): caminho `+0x30`; `appearances` `DynArray<Handle<meshMeshAppearance>>` em `+0x1e0`; `forceLoadAllAppearances` (u8) `+0x6d`;
// `materialEntries` (0x30 B, nome em `+0x20`) em `+0xe0`; `renderResourceBlob` (`Handle`) em `+0x1f8`. `meshMeshAppearance`: nome `+0x30`,
// `chunkMaterials` `DynArray<CName>` `+0x38`, `tags` `DynArray<CName>` `+0x58`.
pub(crate) static MESCLA_DE_APARENCIA_DO_MESH_ATIVA: AtomicBool = AtomicBool::new(false);

pub(crate) static CLASSE_CMESH: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// A#51 51i-2, no `PostLoad` do `CMesh` (thread de job).
pub(crate) unsafe fn mesclar_aparencias_no_mesh(mesh: *mut c_void) {
    if !MESCLA_DE_APARENCIA_DO_MESH_ATIVA.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let alvo = mesh as *mut u8;
    let fim = CMESH_LEGIVEL;
    if !crate::memoria::is_readable(alvo as *const c_void, fim) {
        return;
    }
    let caminho = (alvo.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let patches = patches_do_alvo(caminho, A17_PROP_APPEARANCES);
    let classe = CLASSE_CMESH.load(Ordering::Relaxed);
    let nome_de = |ap: u64| -> Option<u64> {
        (ap != 0 && crate::memoria::is_readable(ap as *const c_void, 0x68)).then(|| ((ap + 0x30) as *const u64).read_unaligned())
    };
    for (pt, _) in patches {
        let pres = recurso_do_patch_da_classe(pt, fim, classe) as *mut u8;
        if pres.is_null() {
            crate::diagnostico::A51I2_NAO_PRONTOS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let paps = handles_do_dynarray(pres.add(CMESH_APPEARANCES_OFF));
        if paps.is_empty() {
            continue;
        }
        let origem = registrar_patch_de_mesh(alvo, pres);
        let mut expansao = 0u64;
        let (mut trocadas, mut anexadas, mut falhas) = (0u64, 0u64, 0u64);
        for ph in &paps {
            let pinst = ph[0] as *const u8;
            if nome_de(ph[0]).is_none() {
                falhas += 1;
                continue;
            }
            // A tag única de uma aparência do patch passa a ser a expansão dela e das seguintes.
            if let Some(tags) = lista_de_u64(pinst, 0x58) {
                if tags.len() == 1 {
                    expansao = tags[0];
                }
            }
            let Some(mut clone) = clonar_aparencia_do_mesh(pinst, expansao, origem) else {
                falhas += 1;
                continue;
            };
            let Some(nome) = nome_de(clone[0]) else {
                crate::jogo::objetos::soltar_handle(&mut clone);
                falhas += 1;
                continue;
            };
            let arr = alvo.add(CMESH_APPEARANCES_OFF);
            let atuais = handles_do_dynarray(arr);
            if dynarray_no_campo(alvo, CMESH_APPEARANCES_OFF).1 != atuais.len() {
                crate::jogo::objetos::soltar_handle(&mut clone);
                falhas += 1;
                continue;
            }
            let tem_chunks = |ap: u64| lista_de_u64(ap as *const u8, MESHAPPEARANCE_CHUNKMATERIALS_OFF).is_some_and(|c| !c.is_empty());
            match atuais.iter().position(|h| nome_de(h[0]) == Some(nome)) {
                Some(i) => {
                    if !tem_chunks(atuais[i][0]) || tem_chunks(clone[0]) {
                        let slot = (dynarray_no_campo(alvo, CMESH_APPEARANCES_OFF).0 as *mut [u64; 2]).add(i);
                        let mut antigo = slot.read_unaligned();
                        slot.write_unaligned(clone);
                        crate::jogo::objetos::soltar_handle(&mut antigo);
                        trocadas += 1;
                    } else {
                        crate::jogo::objetos::soltar_handle(&mut clone);
                    }
                }
                None => {
                    if anexar_handle_no_array(arr, clone) {
                        anexadas += 1;
                    } else {
                        crate::jogo::objetos::soltar_handle(&mut clone);
                        falhas += 1;
                        continue;
                    }
                }
            }
            if let Ok(mut g) = crate::diagnostico::A51I2_AMOSTRAS.try_lock() {
                if g.len() < 32 {
                    g.push([caminho, nome, clone[0], ph[0]]);
                }
            }
        }
        // `forceLoadAllAppearances` desligado: com as aparências do patch, o mesh carrega só a pedida.
        alvo.add(0x6d).write(0);
        crate::diagnostico::A51I2_TROCADAS.fetch_add(trocadas, Ordering::Relaxed);
        crate::diagnostico::A51I2_ANEXADAS.fetch_add(anexadas, Ordering::Relaxed);
        crate::diagnostico::A51I2_FALHAS.fetch_add(falhas, Ordering::Relaxed);
        if crate::primeiras(&crate::diagnostico::A51I2_LOGADOS, 8) {
            crate::veredito_dev!("[a51i] mesh {caminho:#018x}: {} aparência(s) do patch, {trocadas} trocada(s), {anexadas} anexada(s), {falhas} falha(s)", paps.len());
        }
    }
}

/// `CMesh` legível até aqui (o último campo usado pelas mesclas é o `MaterialLock` em `+0x218`).
const CMESH_LEGIVEL: usize = 0x230;
/// `CMesh.renderResourceBlob` (`Handle<rendRenderMeshBlob>`).
const CMESH_RENDERBLOB_OFF: usize = 0x1f8;

pub(crate) static CLASSE_RENDERMESHBLOB: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Cópia funda de um `DynArray` cujos elementos (`elem` B) têm listas internas `(offset, elemento, alinhamento)`: cópia rasa dos elementos e cada lista
/// interna em buffer próprio (o cabeçalho herdado do patch é zerado antes).
pub(crate) unsafe fn copiar_lista_aninhada(dst: *mut u8, src: *const u8, elem: usize, internas: &[(usize, usize, u32)]) -> bool {
    let (sb, sn, _) = dynarray_no_campo(src, 0);
    if sn > 0x1_0000 || (sn > 0 && !crate::memoria::is_readable(sb as *const c_void, sn * elem)) || !copiar_dynarray(dst, src, elem, 8) {
        return false;
    }
    let db = dynarray_no_campo(dst, 0).0;
    for i in 0..sn {
        let (de, se) = (db.add(i * elem), sb.add(i * elem));
        for &(off, e, alin) in internas {
            std::ptr::write_bytes(de.add(off), 0, 0x10);
            if !copiar_dynarray(de.add(off), se.add(off), e, alin) {
                return false;
            }
        }
    }
    true
}

/// `rendRenderMeshBlobHeader` (0xE0 B, em `blob+0x30`) do patch no blob novo, com cópia funda das listas.
pub(crate) unsafe fn copiar_header_do_blob(dst: *mut u8, src: *const u8) -> bool {
    let tipo = tipo_do_header_do_render_blob();
    !tipo.is_null() && crate::rtti::type_assign(tipo, dst as *mut c_void, src as *const c_void)
}

/// `rendRenderMeshBlob.header` em `+0x30`.
const RENDERBLOB_HEADER_OFF: usize = 0x30;
/// `rendRenderMeshBlob.renderBuffer` (`DataBuffer`) em `+0x110`; o blob é legível até 0x140.
const RENDERBLOB_BUFFER_OFF: usize = 0x110;
const RENDERBLOB_LEGIVEL: usize = 0x140;

static TIPO_RENDER_BLOB_HEADER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Tipo do `header` do `rendRenderMeshBlob` pelo RTTI, conferido no offset. A cópia do header vai pelo `Assign` do próprio tipo, que copia cada
/// lista (inclusive as aninhadas) em memória do motor: o layout das listas não precisa ser repetido aqui.
unsafe fn tipo_do_header_do_render_blob() -> *mut c_void {
    let t = TIPO_RENDER_BLOB_HEADER.load(Ordering::Relaxed);
    if !t.is_null() {
        return t;
    }
    let cls = CLASSE_RENDERMESHBLOB.load(Ordering::Relaxed);
    if cls.is_null() {
        return std::ptr::null_mut();
    }
    match crate::rtti::resolve_prop_in_class(cls, "header") {
        Some((off, tipo, _)) if off as usize == RENDERBLOB_HEADER_OFF && crate::rtti::sane(tipo) => {
            TIPO_RENDER_BLOB_HEADER.store(tipo, Ordering::Relaxed);
            tipo
        }
        lido => {
            crate::log_dev!("[a51i] header do rendRenderMeshBlob pelo RTTI fora do esperado: {:?}", lido.map(|(o, _, _)| o));
            std::ptr::null_mut()
        }
    }
}

/// `DataBuffer` do patch no destino: alocador copiado, dados alocados por ele com o tamanho e o alinhamento do patch e copiados. Patch sem dados não muda nada.
pub(crate) unsafe fn copiar_databuffer(dst: *mut u8, src: *const u8) -> bool {
    let (dados, tam, alin) = ((src as *const u64).read_unaligned(), (src.add(8) as *const u32).read_unaligned(), (src.add(0x0c) as *const u32).read_unaligned());
    if dados == 0 {
        return true;
    }
    if tam == 0 || !crate::memoria::is_readable(dados as *const c_void, tam as usize) {
        return false;
    }
    std::ptr::copy_nonoverlapping(src.add(0x10), dst.add(0x10), 0x10);
    let novo = realocar_rawbuffer(dst.add(0x10), 0, tam, alin);
    if novo == 0 {
        std::ptr::write_bytes(dst.add(0x10), 0, 0x10);
        return false;
    }
    std::ptr::copy_nonoverlapping(dados as *const u8, novo as *mut u8, tam as usize);
    (dst as *mut u64).write_unaligned(novo);
    (dst.add(8) as *mut u32).write_unaligned(tam);
    (dst.add(0x0c) as *mut u32).write_unaligned(alin);
    true
}

/// Campos de topo do `CMesh` que acompanham o blob na 51i-3.
pub(crate) unsafe fn copiar_campos_do_mesh(dst: *mut u8, src: *const u8) -> bool {
    // As listas primeiro: só elas podem falhar (reserve sem capacidade), e aí o mesh fica com os escalares de antes.
    for &(off, elem, alin) in CMESH_LISTAS_DO_BLOB {
        if !copiar_dynarray(dst.add(off), src.add(off), elem, alin) {
            return false;
        }
    }
    for &(off, tam) in CMESH_BYTES_DO_BLOB {
        std::ptr::copy_nonoverlapping(src.add(off), dst.add(off), tam);
    }
    true
}

/// Campos de topo do `CMesh` que descrevem a geometria do blob: limites e área (bloco de 0x2c B em `+0x40`), bytes de configuração em `+0x6c`,
/// `+0x6e..+0x6f`, `+0x208..+0x20b` e `+0x219`, `(offset, tamanho)`. O `forceLoadAllAppearances` (`+0x6d`) fica de fora: é das aparências, e a
/// mescla delas (51i-2) o deixa em 0.
const CMESH_BYTES_DO_BLOB: &[(usize, usize)] = &[(0x40, 0x2c), (0x6c, 1), (0x6e, 2), (0x208, 4), (0x219, 1)];
/// Listas do `CMesh` que o blob indexa (ossos, matrizes de rig, epsilons, máscaras e LODs), `(offset, elemento, alinhamento)`.
const CMESH_LISTAS_DO_BLOB: &[(usize, usize, u32)] = &[(0x80, 8, 8), (0x90, 0x40, 0x10), (0xa0, 4, 4), (0xb0, 1, 1), (0xc0, 8, 8), (0xd0, 4, 4)];

/// A#51 51i-3, no `PostLoad` do `CMesh` (thread de job), antes da original: o alvo recebe um `rendRenderMeshBlob` NOVO com o conteúdo do blob do
/// patch e os campos de topo que o acompanham; o handle antigo é solto. Cópia que falha deixa o alvo intacto.
pub(crate) unsafe fn mesclar_blob_no_mesh(mesh: *mut c_void) {
    if !MESCLA_DE_RENDER_BLOB_ATIVA.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let alvo = mesh as *mut u8;
    if !crate::memoria::is_readable(alvo as *const c_void, CMESH_LEGIVEL) {
        return;
    }
    let caminho = (alvo.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    let patches = patches_do_alvo(caminho, A17_PROP_RENDERBLOB);
    let (classe, classe_blob) = (CLASSE_CMESH.load(Ordering::Relaxed), CLASSE_RENDERMESHBLOB.load(Ordering::Relaxed));
    for (pt, props) in patches {
        let pres = recurso_do_patch_da_classe(pt, CMESH_LEGIVEL, classe);
        if pres.is_null() {
            continue;
        }
        let pb = (pres.add(CMESH_RENDERBLOB_OFF) as *const u64).read_unaligned();
        let ab = (alvo.add(CMESH_RENDERBLOB_OFF) as *const u64).read_unaligned();
        let da_classe = pb != 0
            && crate::memoria::is_readable(pb as *const c_void, RENDERBLOB_LEGIVEL)
            && crate::register::class_is_kind_of(crate::rtti::class_of(pb as *mut c_void), classe_blob);
        if !render_blob_muda(props, pb != 0, ab != 0) || !da_classe {
            continue;
        }
        let Some(mut novo) = novo_render_mesh_blob(pb as *const u8) else {
            crate::diagnostico::A51I3_FALHAS.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        if !copiar_campos_do_mesh(alvo, pres) {
            crate::jogo::objetos::soltar_handle(&mut novo);
            crate::diagnostico::A51I3_FALHAS.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        let slot = alvo.add(CMESH_RENDERBLOB_OFF) as *mut [u64; 2];
        let mut antigo = slot.read_unaligned();
        slot.write_unaligned(novo);
        crate::jogo::objetos::soltar_handle(&mut antigo);
        crate::diagnostico::A51I3_COPIADOS.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut g) = crate::diagnostico::A51I3_AMOSTRAS.try_lock() {
            if g.len() < 16 {
                g.push([caminho, novo[0], pb, ab]);
            }
        }
        if crate::primeiras(&crate::diagnostico::A51I3_LOGADOS, 8) {
            crate::veredito_dev!("[a51i] blob do patch no mesh {caminho:#018x}: blob novo {:#x} (do patch {pb:#x}), alvo tinha blob {}", novo[0], ab != 0);
        }
    }
}

pub(crate) static CLASSE_RENDERMORPHTARGETMESHBLOB: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static TIPO_MORPH_BLOB_HEADER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) static TIPO_DEFERREDDATABUFFER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// `rendRenderMeshBlob` novo, num `Handle` do motor, com o conteúdo do blob `pb` do patch.
pub(crate) unsafe fn novo_render_mesh_blob(pb: *const u8) -> Option<[u64; 2]> {
    let mut h = novo_objeto_com_handle(CLASSE_RENDERMESHBLOB.load(Ordering::Relaxed))?;
    let novo = h[0] as *mut u8;
    if !copiar_header_do_blob(novo.add(RENDERBLOB_HEADER_OFF), pb.add(RENDERBLOB_HEADER_OFF))
        || !copiar_databuffer(novo.add(RENDERBLOB_BUFFER_OFF), pb.add(RENDERBLOB_BUFFER_OFF))
    {
        crate::jogo::objetos::soltar_handle(&mut h);
        return None;
    }
    Some(h)
}

/// Instância nova da classe pelo `CreateInstance` do motor, num `Handle` com o forte de quem chama; `None` com classe nula ou criação falha.
unsafe fn novo_objeto_com_handle(cls: *mut c_void) -> Option<[u64; 2]> {
    if cls.is_null() {
        return None;
    }
    let obj = crate::rtti::create_instance(cls);
    if obj.is_null() {
        return None;
    }
    let mut h = [0u64; 2];
    crate::rtti::make_handle(h.as_mut_ptr() as *mut c_void, obj);
    (h[0] != 0 && h[1] != 0).then_some(h)
}

/// `rendRenderMorphTargetMeshBlob`: `baseBlob` `+0x30`, `header` `+0x40`, `diffsBuffer` `+0xc0`, `mappingBuffer` `+0xe8`, `textureDiffsBuffer`
/// `+0x110`; legível até 0x168.
const MORPHBLOB_BASE_OFF: usize = 0x30;
const MORPHBLOB_HEADER_OFF: usize = 0x40;
const MORPHBLOB_DIFFS_OFF: usize = 0xc0;
const MORPHBLOB_MAPPING_OFF: usize = 0xe8;
const MORPHBLOB_TEXDIFFS_OFF: usize = 0x110;
const MORPHBLOB_LEGIVEL: usize = 0x168;

/// `rendRenderMorphTargetMeshBlob` novo, num `Handle` do motor, com o conteúdo do blob `pb` do patch: com o `baseBlob` do patch sendo
/// `rendRenderMeshBlob`, o novo recebe o header, um `baseBlob` novo, os dois `DataBuffer` e o `textureDiffsBuffer`. Passo que falha solta o novo.
pub(crate) unsafe fn novo_morph_blob(pb: *const u8) -> Option<[u64; 2]> {
    if !crate::memoria::is_readable(pb as *const c_void, MORPHBLOB_LEGIVEL) {
        return None;
    }
    let mut h = novo_objeto_com_handle(CLASSE_RENDERMORPHTARGETMESHBLOB.load(Ordering::Relaxed))?;
    let novo = h[0] as *mut u8;
    let base = (pb.add(MORPHBLOB_BASE_OFF) as *const u64).read_unaligned();
    let base_ok = base != 0
        && crate::memoria::is_readable(base as *const c_void, RENDERBLOB_LEGIVEL)
        && crate::register::class_is_kind_of(crate::rtti::class_of(base as *mut c_void), CLASSE_RENDERMESHBLOB.load(Ordering::Relaxed));
    if !base_ok {
        return Some(h);
    }
    let copiou = (|| {
        if !crate::rtti::type_assign(TIPO_MORPH_BLOB_HEADER.load(Ordering::Relaxed), novo.add(MORPHBLOB_HEADER_OFF) as *mut c_void, pb.add(MORPHBLOB_HEADER_OFF) as *const c_void) {
            return false;
        }
        let Some(nb) = novo_render_mesh_blob(base as *const u8) else { return false };
        (novo.add(MORPHBLOB_BASE_OFF) as *mut [u64; 2]).write_unaligned(nb);
        copiar_databuffer(novo.add(MORPHBLOB_DIFFS_OFF), pb.add(MORPHBLOB_DIFFS_OFF))
            && copiar_databuffer(novo.add(MORPHBLOB_MAPPING_OFF), pb.add(MORPHBLOB_MAPPING_OFF))
            && crate::rtti::type_assign(
                TIPO_DEFERREDDATABUFFER.load(Ordering::Relaxed),
                novo.add(MORPHBLOB_TEXDIFFS_OFF) as *mut c_void,
                pb.add(MORPHBLOB_TEXDIFFS_OFF) as *const c_void,
            )
    })();
    if !copiou {
        crate::jogo::objetos::soltar_handle(&mut h);
        return None;
    }
    Some(h)
}

/// Classe `entVisualTagsSchema` (resolvida em `guardar_offsets_do_template`).
pub(crate) static CLASSE_VISUALTAGSSCHEMA: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

// ===== A#51 51k: o alvo espera o patch na desserialização =====
// Mac (desmonte, 2026-09-12): `Deserialize` = `0x10120aff4` (8 argumentos; `x3` = `JobHandle&` recém-criado
// pelo chamador, que depois espera nele antes do `WaitForDependencies`; `x4` = request, com 2 `ResourcePath` em `+0x28` e `+0x30`); `JobHandle::Join` =
// `0x104ddf868` (`x0` = handle que espera, `x1` = handle esperado; `[job+0x18]` zero = nada a esperar). Os tokens dos patches já ficam retidos pela tabela;
// o job do token fica em `+0x50`.
pub(crate) const DESERIALIZE_VM: u64 = 0x1_0120_aff4;

pub(crate) const JOB_JOIN_VM: u64 = 0x1_04dd_f868;

pub(crate) static ORIG_DESERIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ESPERA_DE_PATCH_ATIVA: AtomicBool = AtomicBool::new(false);

/// Caminho do alvo → instâncias dos tokens dos patches que ele espera (retidos pela tabela ou pelo teste).
pub(crate) static ESPERAS_DE_PATCH: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<u64>>>> = std::sync::RwLock::new(None);

pub(crate) type DeserializeFn = unsafe extern "C" fn(u64, u64, u64, *mut u64, *const u8, u64, u64, u64) -> u64;

/// Pares `(alvo, patch)` que podem esperar.
pub(crate) fn esperas_sem_ciclo(pares: &[(u64, u64)]) -> Vec<(u64, u64)> {
    // Um par fica só se o patch não alcança o alvo pelas arestas alvo → patch: senão a espera fecharia um ciclo (o patch de si mesmo inclusive) e
    // os jobs se esperariam para sempre.
    let alcanca = |de: u64, ate: u64| -> bool {
        let (mut fila, mut vistos) = (vec![de], vec![de]);
        while let Some(n) = fila.pop() {
            if n == ate {
                return true;
            }
            for &(a, p) in pares {
                if a == n && !vistos.contains(&p) {
                    vistos.push(p);
                    fila.push(p);
                }
            }
        }
        false
    };
    let mut mantidos: Vec<(u64, u64)> = Vec::new();
    for &(alvo, patch) in pares {
        if !mantidos.contains(&(alvo, patch)) && !alcanca(patch, alvo) {
            mantidos.push((alvo, patch));
        }
    }
    mantidos
}

/// Na carga da tabela: alvo → instâncias dos tokens dos patches que ele espera (só patch com token, sem ciclo, sem repetir), e quantos pares
/// distintos ficaram fora.
pub(crate) fn esperas_da_tabela(patches: &[PatchDeRecurso]) -> (std::collections::HashMap<u64, Vec<u64>>, usize) {
    let mut pares: Vec<(u64, u64)> = Vec::new();
    for p in patches.iter().filter(|p| p.token[0] != 0) {
        if !pares.contains(&(p.alvo, p.patch)) {
            pares.push((p.alvo, p.patch));
        }
    }
    let mantidos = esperas_sem_ciclo(&pares);
    let mut mapa: std::collections::HashMap<u64, Vec<u64>> = std::collections::HashMap::new();
    for p in patches.iter().filter(|p| p.token[0] != 0 && mantidos.contains(&(p.alvo, p.patch))) {
        let tokens = mapa.entry(p.alvo).or_default();
        if !tokens.contains(&p.token[0]) {
            tokens.push(p.token[0]);
        }
    }
    (mapa, pares.len() - mantidos.len())
}

/// Job do token (`+0x50`) e se ainda tem pendência (`[job+0x18]`, o que o `Join` confere); `None` sem job legível.
pub(crate) unsafe fn job_do_token(token: u64) -> Option<(u64, bool)> {
    if token == 0 || !crate::memoria::is_readable((token + 0x50) as *const c_void, 0x10) {
        return None;
    }
    let j = ((token + 0x50) as *const u64).read_unaligned();
    if j == 0 || !crate::memoria::is_readable((j + 0x18) as *const c_void, 4) {
        return None;
    }
    Some((j, ((j + 0x18) as *const u32).read_unaligned() != 0))
}

/// O job do alvo espera o job do token do patch.
pub(crate) unsafe fn juntar_job_do_patch(job: *mut u64, token: u64, caminho: u64, off: u64) {
    match job_do_token(token) {
        Some((_, true)) if !job.is_null() && !crate::rebase(JOB_JOIN_VM).is_null() => {
            let juntar: unsafe extern "C" fn(*mut u64, *const u64) = std::mem::transmute(crate::rebase(JOB_JOIN_VM));
            juntar(job, (token + 0x50) as *const u64);
            let n = crate::diagnostico::A51K_JUNCOES.fetch_add(1, Ordering::Relaxed);
            if n < 8 {
                crate::veredito_dev!("[a51k] alvo {caminho:#018x} (request+{off:#x}) espera o job do patch (token {token:#x})");
            }
        }
        _ => {
            crate::diagnostico::A51K_JA_PRONTOS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Gancho do `Deserialize` (`DESERIALIZE_VM`); a original sai de `original_publicado(&ORIG_DESERIALIZE)`. Com a espera ativa e o 1º caminho da
/// request (`+0x28`) alvo do mapa, o job novo do chamador (`x3`) passa a esperar o job de cada patch dele. O 2º caminho (`+0x30`) só é contado:
/// casar por ele prendeu outra desserialização em espera mútua (boot 123). A original recebe os 8 argumentos como chegaram.
pub(crate) unsafe extern "C" fn deserialize_hook(a0: u64, a1: u64, a2: u64, job: *mut u64, req: *const u8, a5: u64, a6: u64, a7: u64) -> u64 {
    let orig: DeserializeFn = std::mem::transmute(original_publicado(&ORIG_DESERIALIZE));
    if ESPERA_DE_PATCH_ATIVA.load(Ordering::Relaxed) && !req.is_null() {
        crate::diagnostico::A51K_CHAMADAS.fetch_add(1, Ordering::Relaxed);
        let (c28, c30) = ((req.add(0x28) as *const u64).read_unaligned(), (req.add(0x30) as *const u64).read_unaligned());
        let tokens = match ESPERAS_DE_PATCH.read() {
            Ok(g) => {
                let mapa = g.as_ref();
                if mapa.is_some_and(|m| c30 != c28 && m.contains_key(&c30)) {
                    crate::diagnostico::A51K_CASADOS_30.fetch_add(1, Ordering::Relaxed);
                }
                mapa.and_then(|m| m.get(&c28)).cloned()
            }
            Err(_) => None,
        };
        if let Some(tokens) = tokens {
            crate::diagnostico::A51K_CASADOS_28.fetch_add(1, Ordering::Relaxed);
            for t in tokens {
                juntar_job_do_patch(job, t, c28, 0x28);
            }
        }
    }
    orig(a0, a1, a2, job, req, a5, a6, a7)
}

/// As esperas saem com a tabela que as gerou; o gancho, se já subiu, fica e passa reto.
fn limpar_esperas() {
    if let Ok(mut g) = ESPERAS_DE_PATCH.write() {
        *g = None;
    }
    ESPERA_DE_PATCH_ATIVA.store(false, Ordering::Relaxed);
}

/// Na carga da tabela (game thread): mapa de esperas e gancho do `Deserialize`. Sem espera e sem o teste (`a51k-teste`), limpa.
pub(crate) unsafe fn preparar_esperas(esperas: std::collections::HashMap<u64, Vec<u64>>, fora: usize) {
    let teste = marcador_individual("a51k-teste");
    if esperas.is_empty() && !teste {
        limpar_esperas();
        return;
    }
    let alvos = esperas.len();
    if let Ok(mut g) = ESPERAS_DE_PATCH.write() {
        *g = Some(esperas);
    }
    install_deserialize_hook();
    let no_ar = ORIG_DESERIALIZE.load(Ordering::Relaxed) != 0;
    ESPERA_DE_PATCH_ATIVA.store(no_ar, Ordering::Relaxed);
    if teste {
        install_cmesh_postload_hook();
    }
    crate::veredito_dev!(
        "[a51k] {alvos} alvo(s) esperam patch no Deserialize, {fora} par(es) fora por ciclo; gancho {}{}",
        if no_ar { "no ar" } else { "AUSENTE" },
        if teste { " (teste)" } else { "" }
    );
}

/// Pool de CName (`CNamePool::Get` `0x104e38560`) e o byte de pronto (bit 0).
pub(crate) const CNAMEPOOL_VM: u64 = 0x1_0944_ec00;

pub(crate) const CNAMEPOOL_PRONTO_VM: u64 = 0x1_0984_ecc0;

pub(crate) type EstadosDosMeshes = std::collections::HashMap<u64, std::sync::Arc<std::sync::Mutex<EstadoDoMesh>>>;

pub(crate) static ESTADOS_DOS_MESHES: std::sync::RwLock<Option<EstadosDosMeshes>> = std::sync::RwLock::new(None);

/// Estado do mesh pelo caminho (`ESTADOS_DOS_MESHES`), criado quando falta.
pub(crate) unsafe fn adquirir_estado_do_mesh(mesh: *const u8) -> Option<std::sync::Arc<std::sync::Mutex<EstadoDoMesh>>> {
    if mesh.is_null() {
        return None;
    }
    let caminho = (mesh.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned();
    if caminho == 0 {
        return None;
    }
    if let Some(e) = estado_do_mesh(caminho) {
        return Some(e);
    }
    // Preenchido fora da trava: ler os materiais é memória do mesh. Com corrida, o primeiro a gravar fica e o outro estado é descartado.
    let mut novo = EstadoDoMesh::default();
    novo.preencher_materiais(mesh);
    let novo = std::sync::Arc::new(std::sync::Mutex::new(novo));
    let mut g = ESTADOS_DOS_MESHES.write().ok()?;
    let mapa = g.get_or_insert_with(std::collections::HashMap::new);
    let mut criado = false;
    let e = mapa
        .entry(caminho)
        .or_insert_with(|| {
            criado = true;
            novo
        })
        .clone();
    if criado {
        crate::diagnostico::A48_ESTADOS_CRIADOS.fetch_add(1, Ordering::Relaxed);
    }
    Some(e)
}

// `resource.patch`: `PostLoad` de `entEntityTemplate` e `appearanceAppearanceResource`, lidos do vtable+0x30
// por `probe_postload_addresses()` ao vivo (2026-08-12). Hooks em `entitytemplate_postload_replacement`/`appearanceresource_postload_replacement`.
pub(crate) const ENTITYTEMPLATE_POSTLOAD_VM: u64 = 0x1_019c_6380; // entEntityTemplate::PostLoad

pub(crate) const APPEARANCERESOURCE_POSTLOAD_VM: u64 = 0x1_017d_f10c; // appearanceAppearanceResource::PostLoad

pub(crate) static ORIG_CMESH_POSTLOAD: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_MORPHTGT_POSTLOAD: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_ENTITYTEMPLATE_POSTLOAD:     AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_APPEARANCERESOURCE_POSTLOAD: AtomicU64 = AtomicU64::new(0);

/// Gancho do `PostLoad` do `CMesh`; a original sai de `original_publicado(&ORIG_CMESH_POSTLOAD)`.
pub(crate) unsafe extern "C" fn cmesh_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    #[cfg(feature = "devtools")]
    crate::diagnostico::a51k_observar_postload(this);
    #[cfg(feature = "devlog")]
    if !this.is_null() && crate::primeiras(&crate::diagnostico::POSTLOAD_CNT_CMESH, 8) {
        let m = this as *const u8;
        let (_, n, cap) = dynarray_no_campo(m, CMESH_APPEARANCES_OFF);
        crate::log(&format!("[postload] CMesh {:#018x}: appearances {n}/{cap}", (m.add(CMESH_CAMINHO_OFF) as *const u64).read_unaligned()));
    }
    // Tudo antes da original: o `PostLoad` do motor monta o estado do mesh a partir dos nomes, aparências e blob que estão nele.
    aplicar_fix_de_nomes(this);
    mesclar_aparencias_no_mesh(this);
    mesclar_blob_no_mesh(this);
    preparar_estado_do_mesh(this);
    injetar_contexto_do_mesh(this);
    let orig: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(original_publicado(&ORIG_CMESH_POSTLOAD));
    orig(this, a1, a2);
    #[cfg(feature = "devtools")]
    crate::diagnostico::a51i3_apos_original(this);
}

/// As 8 primeiras chamadas de um `PostLoad`, com as palavras `+0x08..fim` do recurso, pro log de dev.
#[cfg(feature = "devlog")]
unsafe fn logar_postload(rotulo: &str, contador: &AtomicU64, this: *mut c_void, fim: usize) {
    if this.is_null() || !crate::primeiras(contador, 8) || !crate::memoria::is_readable(this as *const c_void, fim) {
        return;
    }
    let palavras: Vec<String> = (0x08..fim).step_by(8).map(|o| format!("{:#x}", ((this as *const u8).add(o) as *const u64).read_unaligned())).collect();
    crate::log(&format!("[postload] {rotulo} {this:p}: [{}]", palavras.join(" ")));
}

/// Gancho do `PostLoad` do `MorphTargetMesh`; a original sai de `original_publicado(&ORIG_MORPHTGT_POSTLOAD)`.
pub(crate) unsafe extern "C" fn morphtgt_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    #[cfg(feature = "devlog")]
    logar_postload("MorphTargetMesh", &crate::diagnostico::POSTLOAD_CNT_MORPHTGT, this, 0x70);
    mesclar_patches_no_morph(this);
    let orig: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(original_publicado(&ORIG_MORPHTGT_POSTLOAD));
    orig(this, a1, a2);
    #[cfg(feature = "devtools")]
    crate::diagnostico::a51d3_apos_original(this);
}

/// Gancho do `PostLoad` do `entEntityTemplate` (`ENTITYTEMPLATE_POSTLOAD_VM`): mescla (51a) e a original.
pub(crate) unsafe extern "C" fn entitytemplate_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    #[cfg(feature = "devlog")]
    logar_postload("entEntityTemplate", &crate::diagnostico::POSTLOAD_CNT_ENTITYTEMPLATE, this, 0x40);
    mesclar_patches_no_template(this);
    let orig: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(original_publicado(&ORIG_ENTITYTEMPLATE_POSTLOAD));
    orig(this, a1, a2);
}

/// Gancho do `PostLoad` do `appearanceAppearanceResource` (`APPEARANCERESOURCE_POSTLOAD_VM`): mescla (51c) e a original.
pub(crate) unsafe extern "C" fn appearanceresource_postload_replacement(this: *mut c_void, a1: u64, a2: u64) {
    #[cfg(feature = "devlog")]
    logar_postload("appearanceAppearanceResource", &crate::diagnostico::POSTLOAD_CNT_APPEARANCERESOURCE, this, 0x40);
    mesclar_patches_no_app(this);
    let orig: unsafe extern "C" fn(*mut c_void, u64, u64) = std::mem::transmute(original_publicado(&ORIG_APPEARANCERESOURCE_POSTLOAD));
    orig(this, a1, a2);
}

/// `ResourceAsyncReference::LoadAsync` do motor (2026-09-11, `0x1010f0d34`): `x0` = referência (o hash do path em `+0`),
/// `w1` = flag, `x8` = sret do token. Monta a `ResourceRequest` com a função do próprio motor (`0x1010e9ccc`) e chama o
/// loader por `0x1010e710c` — não pelo `0x1010e7e1c` que o `resloadreq` usa com request montada em Rust. É o caminho do
/// `MeshComponent::LoadResource` (slot `0x268`, `0x10198f864`).
pub(crate) const RESREF_LOADASYNC_VM: u64 = 0x1_010f_0d34;

/// O hook do `PostLoad` do `EntityTemplate` pela tabela de `resource.patch` (51a), sem marcador; o do probe (`entitytemplate-postload`) é o mesmo hook.
pub(crate) unsafe fn install_template_postload_hook() {
    if ORIG_ENTITYTEMPLATE_POSTLOAD.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        ENTITYTEMPLATE_POSTLOAD_VM, entitytemplate_postload_replacement, ORIG_ENTITYTEMPLATE_POSTLOAD,
        &format!("[a51] EntityTemplate::PostLoad hook @ {ENTITYTEMPLATE_POSTLOAD_VM:#012x}"),
        "[a51] FALHA ao hookar o EntityTemplate::PostLoad"
    );
}

/// O hook do `PostLoad` do `AppearanceResource` pela tabela de `resource.patch` (51c), sem marcador; o do probe (`appearanceresource-postload`) é o mesmo hook.
pub(crate) unsafe fn install_app_postload_hook() {
    if ORIG_APPEARANCERESOURCE_POSTLOAD.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        APPEARANCERESOURCE_POSTLOAD_VM, appearanceresource_postload_replacement, ORIG_APPEARANCERESOURCE_POSTLOAD,
        &format!("[a51c] AppearanceResource::PostLoad hook @ {APPEARANCERESOURCE_POSTLOAD_VM:#012x}"),
        "[a51c] FALHA ao hookar o AppearanceResource::PostLoad"
    );
}

/// O hook do `PostLoad` do `worldStreamingWorld` pela tabela de `resource.patch` (51e).
pub(crate) unsafe fn install_world_postload_hook() {
    if ORIG_WORLD_POSTLOAD.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        WORLD_POSTLOAD_VM, world_postload_replacement, ORIG_WORLD_POSTLOAD,
        &format!("[a51e] worldStreamingWorld::PostLoad hook @ {WORLD_POSTLOAD_VM:#012x}"),
        "[a51e] FALHA ao hookar o worldStreamingWorld::PostLoad"
    );
}

/// 51i: hook do `CMesh::PostLoad` pela tabela de fix de nomes (o `install_postload_hooks` instala o mesmo endereço só em dev; a guarda evita o 2º `replace`).
pub(crate) unsafe fn install_cmesh_postload_hook() {
    if ORIG_CMESH_POSTLOAD.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        CMESH_POSTLOAD_VM, cmesh_postload_replacement, ORIG_CMESH_POSTLOAD,
        "[a51i] CMesh::PostLoad hook @ 0x10519a58c",
        "[a51i] FALHA ao hookar o CMesh::PostLoad"
    );
}

/// 51k: hook do `ResourceSerializer::Deserialize` (a espera do patch).
pub(crate) unsafe fn install_deserialize_hook() {
    if ORIG_DESERIALIZE.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        DESERIALIZE_VM, deserialize_hook, ORIG_DESERIALIZE,
        &format!("[a51k] ResourceSerializer::Deserialize hook @ {DESERIALIZE_VM:#012x}"),
        "[a51k] FALHA ao hookar o ResourceSerializer::Deserialize"
    );
}

/// O hook do `OnDependenciesReady` do serializer pela tabela de `resource.patch` (51g, alvos `.curveset`).
pub(crate) unsafe fn install_dependencias_prontas_hook() {
    // A tabela roda no tick (qualquer thread) e no `Reload` da fachada de recursos; o `RegisterCallback` de `Resource/Load` e o `desativar_hooks_de_recurso`
    // instalam e tiram o mesmo hook sob esta trava. Sem ela, checar-e-instalar concorrente empilha 2 hooks e a relocação da segunda copia o
    // salto da primeira: o trampolim salta para o próprio hook.
    let _grupo = crate::tipos_do_motor::HOOKS_DE_EVENTO.lock();
    garantir_gancho_de_dependencias();
}

/// O hook do `PostLoad` do `MorphTargetMesh` pela tabela de `resource.patch` (51d); o de `install_postload_hooks` é o mesmo hook.
pub(crate) unsafe fn install_morph_postload_hook() {
    if ORIG_MORPHTGT_POSTLOAD.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        MORPHTGT_POSTLOAD_VM, morphtgt_postload_replacement, ORIG_MORPHTGT_POSTLOAD,
        &format!("[a51d] MorphTargetMesh::PostLoad hook @ {MORPHTGT_POSTLOAD_VM:#012x}"),
        "[a51d] FALHA ao hookar o MorphTargetMesh::PostLoad"
    );
}

// ===== RESOURCE.LINK: hook de swap em 0x1010efcec (ResourcePath->ResourceReference constructor) =====
// Shim NAKED (`reslink_shim`): se o path (x0) está na tabela, troca x0 pelo alvo (= resource.link). Preserva x1-x8 (x8 = indirect-result do
// constructor) e x30 — só usa x9-x13 (scratch). Tail-call ao original. No dev, `reslink_shim_com_ring` grava antes o path no ring do `reslinkdump`.
#[repr(C)]
pub(crate) struct ResLink {
    pub(crate) idx: AtomicU64,        // @0  contador de construções (só o shim de dev grava)
    pub(crate) count: AtomicU64,      // @8  # de pares no mapa (gate do asm: 0 = pula a chamada Rust)
    pub(crate) swapcnt: AtomicU64,    // @16 swaps disparados (hits)
    pub(crate) _pad: AtomicU64,       // @24
    pub(crate) ring: [AtomicU64; 64], // @32 últimos paths construídos (só o shim de dev grava; o `reslinkdump` lê)
}

pub(crate) static RESLINK: ResLink = ResLink {
    idx: AtomicU64::new(0),
    count: AtomicU64::new(0),
    swapcnt: AtomicU64::new(0),
    _pad: AtomicU64::new(0),
    ring: [const { AtomicU64::new(0) }; 64],
};

/// Bits do filtro de cada `TabelaDeRedirecionamento`: 2^16 posições, 8 KiB por tabela.
const BITS_DO_FILTRO: u32 = 16;

pub(crate) const PALAVRAS_DO_FILTRO: usize = 1 << (BITS_DO_FILTRO - 6);

/// Tabela `caminho → caminho` (hash para hash) do `resource.link` (`RESLINK_MAP`) e do `resource.copy` (`depot::COPIAS`), lida de qualquer thread
/// pelos ganchos do loader (construção de `ResourcePath`) e do depot (`RequestResource`, `CheckResource`):
/// - a origem repetida fica com o último alvo;
/// - o par com um lado no caminho nulo (hash 0) não entra: o gancho trocaria todo `ResourcePath` 0 (o `CheckResource` diria que ele existe), ou
///   mandaria o pedido para o nulo. É a guarda de todo caminho de entrada (`bwms-reslink.txt`, `RegisterLink`, console);
/// - `contagem` é o número de origens, o que o shim do gancho lê para sair do caminho direto, gravada sob a trava junto com a mudança dos pares.
///
/// Os pares ficam ordenados por origem, para a busca binária. O `filtro` (um bit por posição de origem) responde "não está" sem trava para quase todo
/// caminho que o jogo constrói. O bit é ligado antes de o par entrar e nunca é desligado: a origem que sai (`remover`, `limpar`, `substituir`) deixa o
/// bit, que só manda a busca à tabela. O filtro mora num `static` próprio, todo em zero, para não ir ao arquivo do dylib com os ponteiros da tabela.
pub(crate) struct TabelaDeRedirecionamento {
    nome: &'static str,
    pares: std::sync::RwLock<Vec<(u64, u64)>>,
    filtro: &'static [AtomicU64; PALAVRAS_DO_FILTRO],
    contagem: &'static AtomicU64,
}

/// Ordena `v` por origem e deixa uma entrada por origem, com o alvo da última ocorrência: a ordenação estável mantém a ordem de chegada entre origens
/// iguais.
fn ordenar_mantendo_o_ultimo(v: &mut Vec<(u64, u64)>) {
    v.sort_by_key(|&(origem, _)| origem);
    v.dedup_by(|seguinte, mantido| {
        let mesma_origem = seguinte.0 == mantido.0;
        if mesma_origem {
            mantido.1 = seguinte.1;
        }
        mesma_origem
    });
}

impl TabelaDeRedirecionamento {
    pub(crate) const fn nova(nome: &'static str, filtro: &'static [AtomicU64; PALAVRAS_DO_FILTRO], contagem: &'static AtomicU64) -> Self {
        Self { nome, pares: std::sync::RwLock::new(Vec::new()), filtro, contagem }
    }

    /// Palavra e bit da origem no filtro, pelos 16 bits baixos do hash.
    fn posicao_no_filtro(origem: u64) -> (usize, u64) {
        let i = (origem & ((1 << BITS_DO_FILTRO) - 1)) as usize;
        (i >> 6, 1 << (i & 63))
    }

    /// Recusa o par com caminho nulo; no aceito, liga o bit da origem, antes de o par entrar na tabela e de a contagem mudar.
    fn aceitar(&self, origem: u64, alvo: u64) -> bool {
        if origem == 0 || alvo == 0 {
            crate::log_dev!("[{}] par com caminho nulo recusado: {origem:#018x} -> {alvo:#018x}", self.nome);
            return false;
        }
        let (palavra, bit) = Self::posicao_no_filtro(origem);
        self.filtro[palavra].fetch_or(bit, Ordering::Relaxed);
        true
    }

    /// Os pares aceitos de `pares`, ordenados e sem origem repetida, e quantos foram aceitos (a origem repetida conta cada vez). Roda fora da trava.
    fn preparar(&self, pares: &[(u64, u64)]) -> (Vec<(u64, u64)>, usize) {
        let mut novos: Vec<(u64, u64)> = pares.iter().copied().filter(|&(origem, alvo)| self.aceitar(origem, alvo)).collect();
        let aceitos = novos.len();
        ordenar_mantendo_o_ultimo(&mut novos);
        (novos, aceitos)
    }

    /// Um par por cima dos que já estão.
    pub(crate) fn acrescentar(&self, origem: u64, alvo: u64) -> bool {
        if !self.aceitar(origem, alvo) {
            return false;
        }
        let Ok(mut v) = self.pares.write() else { return false };
        match v.binary_search_by_key(&origem, |&(o, _)| o) {
            Ok(i) => v[i].1 = alvo,
            Err(i) => v.insert(i, (origem, alvo)),
        }
        self.contagem.store(v.len() as u64, Ordering::Relaxed);
        true
    }

    /// Os `pares` por cima dos que já estão, de uma vez (a carga do boot e o console), e quantos foram aceitos.
    pub(crate) fn acrescentar_lote(&self, pares: &[(u64, u64)]) -> usize {
        let (novos, aceitos) = self.preparar(pares);
        let Ok(mut v) = self.pares.write() else { return 0 };
        // Os novos depois dos que já estão: a ordenação estável junta as duas sequências já ordenadas, e na origem repetida fica a nova.
        v.extend(novos);
        ordenar_mantendo_o_ultimo(&mut v);
        self.contagem.store(v.len() as u64, Ordering::Relaxed);
        aceitos
    }

    /// Troca os pares todos pelos `pares` (a recarga do `Reload` da fachada de recursos; lista vazia esvazia) e devolve quantos foram aceitos. A tabela nova é
    /// montada fora da trava e entra com a contagem numa seção só sob ela: o shim nunca vê a tabela vazia ou pela metade.
    pub(crate) fn substituir(&self, pares: &[(u64, u64)]) -> usize {
        let (novos, aceitos) = self.preparar(pares);
        let Ok(mut v) = self.pares.write() else { return 0 };
        *v = novos;
        self.contagem.store(v.len() as u64, Ordering::Relaxed);
        aceitos
    }

    /// Alvo da `origem`. Sem trava quando a tabela está vazia ou o bit da origem está desligado.
    pub(crate) fn alvo(&self, origem: u64) -> Option<u64> {
        let (palavra, bit) = Self::posicao_no_filtro(origem);
        if self.contagem.load(Ordering::Relaxed) == 0 || self.filtro[palavra].load(Ordering::Relaxed) & bit == 0 {
            return None;
        }
        let v = self.pares.read().ok()?;
        v.binary_search_by_key(&origem, |&(o, _)| o).ok().map(|i| v[i].1)
    }

    /// Tira o par da `origem`; `true` se havia.
    pub(crate) fn remover(&self, origem: u64) -> bool {
        let Ok(mut v) = self.pares.write() else { return false };
        let Ok(i) = v.binary_search_by_key(&origem, |&(o, _)| o) else { return false };
        v.remove(i);
        self.contagem.store(v.len() as u64, Ordering::Relaxed);
        true
    }

    /// Esvazia a tabela; com a contagem em 0, o shim volta ao caminho direto.
    pub(crate) fn limpar(&self) {
        if let Ok(mut v) = self.pares.write() {
            v.clear();
            self.contagem.store(0, Ordering::Relaxed);
        }
    }

    /// Os pares, ordenados por origem, para as varreduras raras (os apelidos do `GetAliases`, a contagem do console).
    pub(crate) fn ler(&self) -> Option<std::sync::RwLockReadGuard<'_, Vec<(u64, u64)>>> {
        self.pares.read().ok()
    }
}

static RESLINK_FILTRO: [AtomicU64; PALAVRAS_DO_FILTRO] = [const { AtomicU64::new(0) }; PALAVRAS_DO_FILTRO];

/// Tabela do `resource.link` (pedido → servido), com a contagem que o shim lê em `RESLINK.count`.
pub(crate) static RESLINK_MAP: TabelaDeRedirecionamento = TabelaDeRedirecionamento::nova("reslink", &RESLINK_FILTRO, &RESLINK.count);

pub(crate) static ORIG_RESLINK: AtomicU64 = AtomicU64::new(0);

pub(crate) const RESLINK_VM: u64 = 0x1_010e_fcec;

/// Controller de recurso (2026-07-19): hash observado por `watchres <path>` + flag "visto" — o
/// `reslink_lookup` (o shim do gancho, em TODA construção real de `ResourcePath`, o chama para o
/// hash com o bit ligado no filtro; o `watchres` grava o par h → h, que liga o bit do observado)
/// marca `WATCH_RES_SEEN` quando o hash observado bate. O
/// `cp77_tick` (thread do jogo, edge-triggered, MESMO padrão seguro de `Player/Spawned`) drena a
/// flag e dispara `Resource/Load` (`ResourceEvent` real) com o hash — sem hookar
/// `ResourceSerializer::SchedulePostLoadJobs` (RE nova, fora de escopo desta sessão). O disparo em
/// si é 100% real: só acontece quando o jogo de fato constrói aquele `ResourcePath` específico.
pub(crate) static WATCH_RES_HASH: AtomicU64 = AtomicU64::new(0);

pub(crate) static WATCH_RES_SEEN: AtomicBool = AtomicBool::new(false);

/// Chamado pelo shim só quando a tabela tem pares e o bit do path está ligado no filtro (os dois testes no asm, os mesmos do começo de
/// `TabelaDeRedirecionamento::alvo`). Se a tabela tem o path, devolve o alvo (swap = resource.link) e conta o hit; senão (bit colidido) devolve o
/// path intacto.
pub(crate) unsafe extern "C" fn reslink_lookup(path: u64) -> u64 {
    let Some(t) = RESLINK_MAP.alvo(path) else { return path };
    RESLINK.swapcnt.fetch_add(1, Ordering::Relaxed);
    // Diagnóstico de dev, fora do build público: o `.ent` do PLAYER redirecionado (não o animgraph) confirma que o template do player passa pelo
    // hook no spawn; `contar_alias_do_teste_resolvido` conta os aliases do teste a11, que só o selftest lê.
    #[cfg(feature = "devlog")]
    {
        if path == 0x1ab9_05fe_c596_cb22 || path == 0x1bcd_09f6_f70a_7818 || path == 0x58b1_6007_a8cd_0f7c || path == 0x5ea3_4357_4414_752e {
            crate::log_dev!("[reslink] >>> PLAYER .ent redirecionado: {path:#x} -> {t:#x}");
        }
        contar_alias_do_teste_resolvido(path);
    }
    if path == WATCH_RES_HASH.load(Ordering::Relaxed) && path != 0 {
        WATCH_RES_SEEN.store(true, Ordering::Relaxed);
    }
    t
}

/// O shim do gancho do construtor de `ResourcePath` (`RESLINK_VM`): roda em toda construção, de qualquer thread. O path fora da tabela, quase todo,
/// vai ao original sem empilhar nada e sem chamar Rust: tabela vazia (`count`) ou bit do path desligado no `RESLINK_FILTRO`, na posição de
/// `TabelaDeRedirecionamento::posicao_no_filtro` (palavra = bits 6 a `BITS_DO_FILTRO`-1 do hash; bit = os 6 bits baixos).
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn reslink_shim() {
    core::arch::naked_asm!(
        "adrp x9, {r}@PAGE",
        "add  x9, x9, {r}@PAGEOFF",    // x9 = &RESLINK
        "ldr  x13, [x9, #8]",          // count (# de pares)
        "cbz  x13, 2f",                // sem links -> tail-call direto
        "adrp x10, {f}@PAGE",
        "add  x10, x10, {f}@PAGEOFF",  // x10 = &RESLINK_FILTRO
        "ubfx x11, x0, #6, #{largura}", // palavra do path no filtro
        "ldr  x12, [x10, x11, lsl #3]",
        "lsr  x12, x12, x0",           // o deslocamento por registrador usa só os 6 bits baixos de x0: o bit do path vai para o bit 0
        "tbz  x12, #0, 2f",            // bit desligado: o path não está na tabela -> tail-call direto
        "stp  x1, x2, [sp, #-0x60]!",  // salva args + indirect-result (x8) do constructor
        "stp  x3, x4, [sp, #0x10]",
        "stp  x5, x6, [sp, #0x20]",
        "stp  x7, x8, [sp, #0x30]",
        "str  x30, [sp, #0x40]",
        "bl   {lookup}",               // x0 = reslink_lookup(x0)  (swap se hit = resource.link!)
        "ldr  x30, [sp, #0x40]",
        "ldp  x7, x8, [sp, #0x30]",
        "ldp  x5, x6, [sp, #0x20]",
        "ldp  x3, x4, [sp, #0x10]",
        "ldp  x1, x2, [sp], #0x60",
        "2:",
        "adrp x13, {o}@PAGE",
        "add  x13, x13, {o}@PAGEOFF",
        "ldr  x13, [x13]",
        "br   x13",                    // tail-call original (x8/x1..x7/x30 intactos)
        r = sym RESLINK,
        f = sym RESLINK_FILTRO,
        largura = const BITS_DO_FILTRO - 6,
        o = sym ORIG_RESLINK,
        lookup = sym reslink_lookup,
    )
}

/// O shim do dev: grava o path (x0) no ring que o `reslinkdump` do selftest lê e segue pelo `reslink_shim`. O público não tem leitor do ring e
/// instala o `reslink_shim` direto.
#[cfg(feature = "devtools")]
#[unsafe(naked)]
pub(crate) unsafe extern "C" fn reslink_shim_com_ring() {
    core::arch::naked_asm!(
        "adrp x9, {r}@PAGE",
        "add  x9, x9, {r}@PAGEOFF",    // x9 = &RESLINK
        "ldr  x10, [x9]",
        "add  x11, x10, #1",
        "str  x11, [x9]",              // idx++
        "and  x10, x10, #0x3f",
        "add  x12, x9, #32",
        "str  x0, [x12, x10, lsl #3]", // ring[idx&63] = x0 (path)
        "b    {shim}",                 // x0-x8 e x30 intactos
        r = sym RESLINK,
        shim = sym reslink_shim,
    )
}

pub(crate) unsafe fn install_reslink() {
    if ORIG_RESLINK.load(Ordering::Relaxed) != 0 {
        return; // já instalado (idempotente: dev-selftest + auto-load de mod)
    }
    let t = crate::rebase(RESLINK_VM);
    if !crate::memoria::is_readable(t as *const c_void, 16) {
        crate::log_dev!("[reslink] alvo 0x1010efcec ilegível");
        return;
    }
    #[cfg(feature = "devtools")]
    let shim = reslink_shim_com_ring as *mut c_void;
    #[cfg(not(feature = "devtools"))]
    let shim = reslink_shim as *mut c_void;
    match crate::ganchos::instalar_com_threads_paradas(t, shim, Some(&ORIG_RESLINK)) {
        Some(_) => {
            crate::log_dev!("[reslink] hook NAKED em 0x1010efcec (ResourcePath->ref) — reslinkdump / reslink <src> <tgt> / reslinkstat");
        }
        None => crate::log_dev!("[reslink] replace None"),
    }
}

/// Adiciona UM par à tabela, por cima dos que já estão (`TabelaDeRedirecionamento::acrescentar`).
pub(crate) fn reslink_add(src: u64, tgt: u64) -> bool {
    if !RESLINK_MAP.acrescentar(src, tgt) {
        return false;
    }
    crate::log_dev!("[reslink] +par {src:#018x} -> {tgt:#018x} (tabela: {} pares)", RESLINK.count.load(Ordering::Relaxed));
    true
}

/// Alvo do par de `src` na tabela do `resource.link`.
pub(crate) fn reslink_alvo(src: u64) -> Option<u64> {
    RESLINK_MAP.alvo(src)
}

/// Adiciona um par a partir dos PATHS (strings) — hasheia com FNV-1a64.
pub(crate) fn reslink_path(src: &str, tgt: &str) {
    let (s, t) = (resource_path_hash(src), resource_path_hash(tgt));
    crate::log_dev!("[reslink] path '{src}' (#{s:#018x}) -> '{tgt}'");
    reslink_add(s, t);
}

/// Carrega N pares de um arquivo (linhas de `bwms_primitives::tabelas::Reslink`, `#`=comentário). É o que um mod
/// com `.xl` usa: o mod-manager gera esse arquivo do `.xl` (resource.link/copy) e o runtime
/// popula a tabela. Caminho default = `<red4ext>/bwms-reslink.txt`.
///
/// Com `substituir` (o `Reload` da fachada de recursos), as tabelas do `resource.link` e do `resource.copy` viram as do arquivo, e o arquivo ausente as esvazia (o
/// `bwms install` o apaga quando nenhum mod gera par): saem o par da carga anterior e o de `RegisterLink`. Sem `substituir` (a carga do boot e o console), os pares entram por cima dos que já estão, porque o
/// `RegisterLink` do `OnLoad` dos `ScriptableService` e os pares de teste do `on_load` são gravados antes da carga do boot.
pub(crate) fn reslink_file(path: &str, substituir: bool) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if substituir && e.kind() == std::io::ErrorKind::NotFound => {
            reslink_texto("", true);
            return crate::log_dev!("[reslink] '{path}' ausente: tabelas do resource.link e do resource.copy vazias");
        }
        Err(e) => return crate::log_dev!("[reslink] não leu '{path}': {e}"),
    };
    let n = reslink_texto(&content, substituir);
    unsafe { crate::arquivos_do_jogo::depot::instalar() };
    crate::log_dev!("[reslink] {n} pares carregados de '{path}'");
}

/// Linhas do `bwms-reslink.txt` nas tabelas do `resource.link` e do `resource.copy`, sem instalar gancho. Conta só o par que a regra aceitou
/// (`TabelaDeRedirecionamento`: sem caminho nulo; a origem repetida conta cada vez). Sem `substituir`, os pares de cada tabela entram por cima dos que
/// já estão de uma vez (`acrescentar_lote`); com `substituir`, cada tabela vira a do texto (texto vazio a esvazia). Nos dois casos a mudança e a
/// contagem que o shim lê entram numa seção só sob a trava da tabela: o shim, que roda em outras threads, nunca vê a tabela vazia ou pela metade.
fn reslink_texto(content: &str, substituir: bool) -> usize {
    let (mut links, mut copias) = (Vec::new(), Vec::new());
    // `<pedido> | <servido>` é link (loader); com a 3ª coluna `copy`, a cópia é servida no depot (`crate::arquivos_do_jogo::depot`). Linha fora do
    // formato (`bwms_primitives::tabelas::Reslink`) fica de fora.
    for l in bwms_primitives::tabelas::ler(content, bwms_primitives::tabelas::Reslink::ler).flatten() {
        let par = (resource_path_hash(l.pedido), resource_path_hash(l.servido));
        if l.copia { copias.push(par) } else { links.push(par) }
    }
    if substituir {
        RESLINK_MAP.substituir(&links) + depot::COPIAS.substituir(&copias)
    } else {
        RESLINK_MAP.acrescentar_lote(&links) + depot::COPIAS.acrescentar_lote(&copias)
    }
}

/// `GetAliases(path) -> Set<ResourcePath>` (`PENDENCIAS-UNIFICADAS.md`): os caminhos que são o mesmo recurso que `path`, nos dois
/// sentidos, pelos links (`RESLINK_MAP`, que `resource.link` e `RegisterLink` escrevem) e pelas cópias (`depot`), em ordem crescente e sem repetição. É
/// a varredura sobre cópias ∪ links: o link `a → b` dá `b` para `a`
/// e `a` para `b`; a cópia `c` de `b` dá `b` para `c` e `c` para `b`; a chave que está nas duas tabelas responde só pelo link. Leitura das tabelas em
/// memória, sem I/O nem chamada nativa.
pub(crate) fn reslink_aliases_of(path: u64) -> Vec<u64> {
    let mut apelidos = Vec::new();
    let chaves_de_link: Vec<u64> = match RESLINK_MAP.ler() {
        Some(m) => {
            for &(s, t) in m.iter() {
                if s == path {
                    apelidos.push(t);
                } else if t == path {
                    apelidos.push(s);
                }
            }
            m.iter().map(|&(s, _)| s).collect()
        }
        None => Vec::new(),
    };
    // A trava do `RESLINK_MAP` sai antes de pegar a do depot: o `reslink_lookup` a pega na construção de `ResourcePath` com o bit da origem ligado, e
    // nenhuma função segura as duas juntas (o `CheckResource` do depot pega uma depois da outra), então não há ordem de trava a manter.
    depot::apelidos_de_copia(path, &mut apelidos, &chaves_de_link);
    apelidos.sort_unstable();
    apelidos.dedup();
    apelidos
}


pub(crate) type DependenciasProntasFn = unsafe extern "C" fn(*mut c_void);

/// Original do `OnDependenciesReady`, publicado antes do patch. Não volta a zero quando o gancho sai: uma thread já dentro do handler ainda o lê.
pub(crate) static ORIG_DEPENDENCIAS: AtomicU64 = AtomicU64::new(0);

/// O gancho do `OnDependenciesReady` está no ar. Quem usa liga só o seu interruptor: `resource.patch` (`A51G_ATIVO`/`MESCLA_DE_DEVICES_ATIVA`) e
/// `Resource/Load` (`RESOURCE_LOAD_ATIVO`).
pub(crate) static GANCHO_DEPENDENCIAS_NO_AR: AtomicBool = AtomicBool::new(false);

/// Rota única do gancho do `OnDependenciesReady`: o `resource.patch` (no boot) e o `Resource/Load` (no `RegisterCallback`) chamam esta mesma
/// função; o handler despacha pra quem estiver ligado. Instala com as outras threads paradas e publica o original antes do patch.
/// Quem chama segura `tipos_do_motor::HOOKS_DE_EVENTO`; pegar aqui trava o `RegisterCallback`, que já a segura (`Mutex` não reentrante).
pub(crate) unsafe fn garantir_gancho_de_dependencias() -> bool {
    if GANCHO_DEPENDENCIAS_NO_AR.load(Ordering::Relaxed) {
        return true;
    }
    // `None` é este gancho já no alvo: no ar também.
    let ok = instalar_conferindo_prologo(
        crate::jogo::recursos::DEPENDENCIAS_PRONTAS_VM,
        crate::jogo::recursos::DEPENDENCIAS_PRONTAS_PROLOGO,
        dependencias_prontas_hook as *mut c_void,
        &ORIG_DEPENDENCIAS,
    ) != Some(false);
    GANCHO_DEPENDENCIAS_NO_AR.store(ok, Ordering::Relaxed);
    ok
}

/// Gancho do "dependências prontas" do recurso (`jogo::recursos::DEPENDENCIAS_PRONTAS_VM`, contexto com `serializables` em `+0x230`); a original sai
/// de `ORIG_DEPENDENCIAS`. Antes da original: o `Resource/Load` dos `CResource` do contexto (com ouvinte) e as mesclas que pedem o recurso já com as
/// dependências (curvas, devices, inkanim), cada uma só com objeto da classe exata dela.
pub(crate) unsafe extern "C" fn dependencias_prontas_hook(ctx: *mut c_void) {
    let orig: DependenciasProntasFn = std::mem::transmute(original_publicado(&ORIG_DEPENDENCIAS));
    let c = ctx as *const u8;
    let legivel = !ctx.is_null() && crate::memoria::is_readable(ctx as *const c_void, 0x240);
    if legivel && RESOURCE_LOAD_ATIVO.load(Ordering::Relaxed) && crate::tipos_do_motor::tem_ouvinte("Resource/Load") {
        despachar_resource_load_dos_serializables(c);
    }
    let (curvas, devices, inkanim) = (
        A51G_ATIVO.load(Ordering::Relaxed),
        MESCLA_DE_DEVICES_ATIVA.load(Ordering::Relaxed),
        crate::interface::MESCLA_DE_INKANIM_ATIVA.load(Ordering::Relaxed),
    );
    if legivel && (curvas || devices || inkanim) {
        let (b, n, _) = dynarray_no_campo(c, 0x230);
        if n > 0 && n <= 4096 && crate::memoria::is_readable(b as *const c_void, n * 16) {
            let (cls_curvas, cls_devices, cls_inkanim) = (
                A51G_CLASSE_CURVESET.load(Ordering::Relaxed),
                CLASSE_GAMEDEVICERESOURCE.load(Ordering::Relaxed),
                crate::interface::CLASSE_INKANIM_BIBLIOTECA.load(Ordering::Relaxed),
            );
            for i in 0..n {
                let obj = (b.add(i * 16) as *const u64).read_unaligned();
                if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, 8) {
                    continue;
                }
                let cls = crate::rtti::class_of(obj as *mut c_void);
                if cls.is_null() {
                    continue;
                }
                if curvas && cls == cls_curvas {
                    crate::curvas::a51g_mesclar_curvas(obj);
                } else if devices && cls == cls_devices {
                    crate::mundo::mesclar_devices(obj);
                } else if inkanim && cls == cls_inkanim {
                    crate::interface::mesclar_inkanim(obj);
                }
            }
        }
    }
    orig(ctx);
}


use crate::ganchos::install_hook_probe;

// ===== A#51 51j: `persistentStateData` do streaming world =====
// Candidata offline ao `SetPersistentStateData(a1, DataBuffer& dados, a3, u32 a4)` do sistema de persistência: `0x102d0d090` — guarda `x0`..`x3` e usa
// `w3` como `u32`, lê dados e tamanho do `x1` pelos acessores de `DataBuffer` (`0x101212ea4`/`0x101212ebc`) e põe o buffer num leitor (`0x101074b44`);
// chamada pelo carregador `PSData` do save (`0x102d0cd8c`) e por `0x102d12650`. Campos pelo RTTI: `worldStreamingWorld.persistentStateData` (`Ref`:
// caminho `+0x00`, token `+0x08`) e `gamePersistentStateDataResource.buffer` (`DataBuffer`).
const SET_PERSISTENT_STATE_DATA_VM: u64 = 0x1_02d0_d090;
static ORIG_SET_PERSISTENT_STATE_DATA: AtomicU64 = AtomicU64::new(0);
type SetPersistentStateDataFn = unsafe extern "C" fn(u64, *mut u8, u64, u32) -> u64;

/// Offset do `persistentStateData` no `worldStreamingWorld` e do `buffer` no `gamePersistentStateDataResource`, pelo RTTI; 0 = não lido.
static MUNDO_PSD_OFF: AtomicU64 = AtomicU64::new(0);
static PSD_BUFFER_DO_RECURSO_OFF: AtomicU64 = AtomicU64::new(0);
static CLASSE_DO_RECURSO_DE_PSD: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Com o marcador `a51j-teste`, o buffer do próprio mundo entra como patch dele.
static PSD_TESTE: AtomicBool = AtomicBool::new(false);
static PSD_REAPLICADOS: AtomicU64 = AtomicU64::new(0);

/// `(caminho do Ref, buffer)` do `persistentStateData` do último mundo visto (`A51J_MUNDO`): `Ref` em `mundo+off` → token `+0x08` → recurso
/// `token+0x28` → `+buffer`. `None` sem mundo, sem offsets ou com algo ilegível.
unsafe fn psd_do_mundo() -> Option<(u64, *mut u8)> {
    let (mundo, off, buf) = (
        crate::diagnostico::A51J_MUNDO.load(Ordering::Relaxed),
        MUNDO_PSD_OFF.load(Ordering::Relaxed) as usize,
        PSD_BUFFER_DO_RECURSO_OFF.load(Ordering::Relaxed) as usize,
    );
    if mundo == 0 || off == 0 || buf == 0 || !crate::memoria::is_readable((mundo as usize + off) as *const c_void, 0x18) {
        return None;
    }
    let r = (mundo as usize + off) as *const u64;
    let (caminho, token) = (r.read_unaligned(), r.add(1).read_unaligned());
    let recurso = recurso_carregado(token, buf + 0x10);
    (!recurso.is_null()).then(|| (caminho, recurso.add(buf) as *mut u8))
}

/// Gancho da candidata ao `SetPersistentStateData`; a original sai de `original_publicado(&ORIG_SET_PERSISTENT_STATE_DATA)`. A original roda
/// primeiro; quando os `dados` são o buffer do `persistentStateData` do último mundo visto, ela roda de novo com o buffer de cada patch pronto
/// daquele `Ref`. Devolve o retorno da 1ª chamada.
unsafe extern "C" fn set_persistent_state_data_hook(a1: u64, dados: *mut u8, a3: u64, a4: u32) -> u64 {
    let orig: SetPersistentStateDataFn = std::mem::transmute(original_publicado(&ORIG_SET_PERSISTENT_STATE_DATA));
    let r = orig(a1, dados, a3, a4);
    let Some((caminho, buffer)) = psd_do_mundo() else { return r };
    if dados != buffer {
        return r;
    }
    let buf = PSD_BUFFER_DO_RECURSO_OFF.load(Ordering::Relaxed) as usize;
    let classe = CLASSE_DO_RECURSO_DE_PSD.load(Ordering::Relaxed);
    let mut buffers: Vec<*mut u8> = patches_do_alvo(caminho, u32::MAX)
        .into_iter()
        .filter_map(|(pt, _)| {
            let p = recurso_do_patch_da_classe(pt, buf + 0x10, classe);
            (!p.is_null()).then(|| p.add(buf) as *mut u8)
        })
        .collect();
    if PSD_TESTE.load(Ordering::Relaxed) {
        buffers.push(buffer);
    }
    for b in &buffers {
        orig(a1, *b, a3, a4);
    }
    if !buffers.is_empty() {
        let n = PSD_REAPLICADOS.fetch_add(buffers.len() as u64, Ordering::Relaxed);
        if n < 8 {
            crate::veredito_dev!("[a51j] persistentStateData {caminho:#018x}: {} buffer(s) de patch reaplicado(s) depois do do mundo", buffers.len());
        }
    }
    r
}

/// Na carga das tabelas (game thread, `mods_do_usuario::pipeline`): só com a tabela de patches ativa ou com os marcadores `a51j-sonda`/`a51j-teste`.
/// Lê os offsets pelo RTTI e instala o gancho do `PostLoad` do mundo (que guarda o último mundo) e o da candidata ao `SetPersistentStateData`.
pub(crate) unsafe fn preparar_hook_de_psd() {
    let teste = marcador_individual("a51j-teste");
    if !PATCHES_DE_RECURSO_ATIVOS.load(Ordering::Relaxed) && !teste && !marcador_individual("a51j-sonda") {
        return;
    }
    // Sem a imagem do jogo (build sem tradução de endereço) não há o que ganchar nem RTTI a ler.
    if crate::rebase(SET_PERSISTENT_STATE_DATA_VM).is_null() {
        return;
    }
    let Some(reg) = crate::rtti::Registry::obtain() else { return };
    let prop = |classe: &str, campo: &str| {
        let c = reg.class_by_name(classe);
        if c.is_null() { None } else { crate::rtti::resolve_prop_in_class(c, campo).map(|(o, _, _)| o as u64) }
    };
    let (psd, buffer) = (prop("worldStreamingWorld", "persistentStateData"), prop("gamePersistentStateDataResource", "buffer"));
    let classe = reg.class_by_name("gamePersistentStateDataResource");
    let (Some(psd), Some(buffer)) = (psd, buffer) else {
        crate::veredito_dev!("[a51j] offsets pelo RTTI: persistentStateData {psd:?}, buffer {buffer:?}; gancho fora");
        return;
    };
    MUNDO_PSD_OFF.store(psd, Ordering::Relaxed);
    PSD_BUFFER_DO_RECURSO_OFF.store(buffer, Ordering::Relaxed);
    CLASSE_DO_RECURSO_DE_PSD.store(classe, Ordering::Relaxed);
    PSD_TESTE.store(teste, Ordering::Relaxed);
    install_world_postload_hook();
    install_set_persistent_state_data_hook();
    crate::veredito_dev!(
        "[a51j] persistentStateData {psd:#x}, buffer {buffer:#x}; gancho do PostLoad do mundo {}, do SetPersistentStateData {}{}",
        if ORIG_WORLD_POSTLOAD.load(Ordering::Relaxed) == 0 { "AUSENTE" } else { "no ar" },
        if ORIG_SET_PERSISTENT_STATE_DATA.load(Ordering::Relaxed) == 0 { "AUSENTE" } else { "no ar" },
        if teste { " (teste)" } else { "" }
    );
}

/// A#51 51j: gancho da candidata ao `SetPersistentStateData`.
unsafe fn install_set_persistent_state_data_hook() {
    if ORIG_SET_PERSISTENT_STATE_DATA.load(Ordering::Relaxed) != 0 {
        return;
    }
    install_hook_probe!(
        SET_PERSISTENT_STATE_DATA_VM, set_persistent_state_data_hook, ORIG_SET_PERSISTENT_STATE_DATA,
        &format!("[a51j] candidata ao SetPersistentStateData hook @ {SET_PERSISTENT_STATE_DATA_VM:#012x}"),
        "[a51j] FALHA ao hookar a candidata ao SetPersistentStateData"
    );
}

/// Aviso de dev da fábrica `tramp_rt_path` para `ResourceTarget.Path` cujo texto registrado é `regex:` que o motor recusa: o despacho
/// (`caminho_casa`) trata o recusado como "não casou", e o mod fica sem eventos e sem mensagem. Vai pelo veredito porque o `log()` descarta
/// 7 de cada 8 linhas depois da 800ª, e a fábrica roda quando o mod monta o alvo do callback, em geral depois disso. Não alcança texto registrado depois da fábrica
/// nem `ResRef` literal sem texto registrado (F1-28). Mora no fim do código de produção: no meio, as linhas de baixo andariam e, com elas, a
/// linha gravada nos `Location` de pânico do build público, que não leva esta função.
#[cfg(any(feature = "devlog", test))]
fn rt_path_recusado(path_hash: u64) -> Option<String> {
    let texto = caminho_texto(path_hash)?;
    let bwms_primitives::restarget::CompiledResourceTarget::Recusado(erro) = bwms_primitives::restarget::CompiledResourceTarget::compilar(&texto)? else {
        return None;
    };
    // O caso `Complexo` passa de 4096 bytes.
    let curto: String = texto.chars().take(160).collect();
    Some(format!("[rt] ResourceTarget.Path '{curto}' recusado: {erro}"))
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::mods_do_usuario::pipeline::testes::travar_tabelas_xl;

    /// `game_dir_from_dylib` dá a pasta do jogo só quando a dylib mora na pasta do runtime (`<jogo>/red4ext/`). No executável de teste a
    /// pasta da dylib é `target/<perfil>/deps`, e a pasta do jogo fica desconhecida.
    #[test]
    fn pasta_do_jogo_so_no_layout_instalado() {
        assert_eq!(pasta_do_jogo("/Jogos/Cyberpunk 2077/red4ext"), Some("/Jogos/Cyberpunk 2077"));
        assert_eq!(pasta_do_jogo("/Jogos/Cyberpunk 2077/nao_red4ext"), None);
        assert_eq!(pasta_do_jogo("/Jogos/Cyberpunk 2077/red4ext/plugins"), None);
        assert_eq!(pasta_do_jogo("red4ext"), None);
        let dylib = crate::dylib_dir().expect("dladdr no executável de teste");
        assert_eq!(pasta_do_jogo(&dylib), None, "{dylib}");
        assert_eq!(game_dir_from_dylib(), None);
    }

    /// `ativ:N5`: a tabela de `resource.patch`, o `RegisterCallback` de `Resource/Load` e o `desativar_hooks_de_recurso` instalam e tiram o hook
    /// do `OnDependenciesReady` sob `HOOKS_DE_EVENTO`, cada rota pegando a trava uma vez só (`Mutex` não reentrante). Instalar pede o binário do
    /// jogo, então o teste confere o texto de `src/`:
    /// 1. `install_dependencias_prontas_hook` e `ativar_hooks_do_evento` travam antes de garantir, e `desativar_hooks_sem_ouvinte` antes de tirar;
    /// 2. `garantir_gancho_de_dependencias` e `garantir_hooks_de_recurso`, que rodam sob a trava, não a pegam de novo;
    /// 3. em `src/` inteiro, fora as definições e este módulo de teste, `garantir_gancho_de_dependencias(` aparece só em `install_dependencias_prontas_hook` e
    ///    `garantir_hooks_de_recurso`. A contagem pelo nome com `(` pega também o caminho completo (`crate::arquivos_do_jogo::`) e a chamada dentro
    ///    de expressão (`if garantir_gancho_de_dependencias() {`);
    /// 4. `garantir_hooks_de_recurso` entra só como ponteiro na linha `Resource/Load` de `GRUPOS_DE_EVENTO` (o nome fora de crase aparece uma vez em
    ///    `src/`, fora a definição), e a coluna de instalação da tabela só roda em `ativar_hooks_do_evento` (`garantir()` uma vez no código de
    ///    `tipos_do_motor`, fora o módulo de teste).
    #[test]
    fn gancho_de_dependencias_instala_sob_hooks_de_evento_sem_travar_duas_vezes() {
        const MARCA_DO_TESTE: &str = "#[cfg(test)]\nmod testes {";
        let fonte = include_str!("mod.rs");
        let producao = &fonte[..fonte.find(MARCA_DO_TESTE).expect("módulo de teste de arquivos_do_jogo")];
        let motor = include_str!("../tipos_do_motor/mod.rs");
        let corpo = |texto: &'static str, assinatura: &str| -> &'static str {
            let resto = &texto[texto.find(assinatura).unwrap_or_else(|| panic!("`{assinatura}` sumiu"))..];
            &resto[..resto.find("\n}\n").expect("fim da função")]
        };

        let tabela = corpo(producao, "pub(crate) unsafe fn install_dependencias_prontas_hook()");
        let trava = tabela.find("crate::tipos_do_motor::HOOKS_DE_EVENTO.lock();").expect("a tabela de resource.patch instala sem a trava");
        assert!(trava < tabela.find("garantir_gancho_de_dependencias();").expect("a tabela não garante o gancho"));
        let grupos = &motor[motor.find("const GRUPOS_DE_EVENTO:").expect("`GRUPOS_DE_EVENTO` sumiu")..];
        let grupos = &grupos[..grupos.find("\n];\n").expect("fim de GRUPOS_DE_EVENTO")];
        let linha_do_recurso = "(&[\"Resource/Load\"], crate::arquivos_do_jogo::garantir_hooks_de_recurso, Some(desativar_hooks_de_recurso), None)";
        assert!(grupos.contains(linha_do_recurso), "Resource/Load fora da tabela de grupos");
        let ativar = corpo(motor, "pub(crate) fn ativar_hooks_do_evento(");
        let trava = ativar.find("let _grupo = HOOKS_DE_EVENTO.lock();").expect("RegisterCallback sem a trava");
        assert!(trava < ativar.find("garantir()").expect("RegisterCallback sem instalar o grupo do evento"));
        let motor_producao = &motor[..motor.find(MARCA_DO_TESTE).expect("módulo de teste de tipos_do_motor")];
        assert_eq!(motor_producao.matches("garantir()").count(), 1, "a coluna de instalação da tabela roda fora de ativar_hooks_do_evento");
        let desativar = corpo(motor, "pub(crate) fn desativar_hooks_sem_ouvinte(");
        let trava = desativar.find("let _grupo = HOOKS_DE_EVENTO.lock();").expect("desativar sem a trava");
        assert!(trava < desativar.find("desativar()").expect("desativar sem tirar o grupo sem ouvinte"));

        let recurso = corpo(producao, "pub(crate) unsafe fn garantir_hooks_de_recurso()");
        assert!(recurso.contains("garantir_gancho_de_dependencias("), "o RegisterCallback não garante o gancho");
        assert!(!recurso.contains("HOOKS_DE_EVENTO"));
        assert!(!corpo(producao, "pub(crate) unsafe fn garantir_gancho_de_dependencias()").contains("HOOKS_DE_EVENTO"));

        let src = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let este = src.join("arquivos_do_jogo/mod.rs");
        let motor_rs = src.join("tipos_do_motor/mod.rs");
        let mut chamadas_do_gancho = Vec::new();
        let mut usos_do_recurso = Vec::new();
        let mut pastas = vec![src];
        while let Some(pasta) = pastas.pop() {
            for item in std::fs::read_dir(&pasta).expect("pasta de src/ legível") {
                let caminho = item.expect("item de src/").path();
                if caminho.is_dir() {
                    pastas.push(caminho);
                    continue;
                }
                if caminho.extension().map_or(true, |ext| ext != "rs") {
                    continue;
                }
                let bytes = std::fs::read(&caminho).expect("fonte .rs legível");
                let texto = String::from_utf8_lossy(&bytes);
                let texto = if caminho == este { &texto[..texto.find(MARCA_DO_TESTE).expect("módulo de teste de arquivos_do_jogo")] } else { &texto[..] };
                // `fn nome(` também casa com `nome(`: a definição sai da contagem.
                let chamadas = |nome: &str| texto.matches(&format!("{nome}(")).count() - texto.matches(&format!("fn {nome}(")).count();
                chamadas_do_gancho.extend(std::iter::repeat(caminho.clone()).take(chamadas("garantir_gancho_de_dependencias")));
                // Chamada ou ponteiro contam igual; a definição e a citação entre crases em comentário saem.
                let nome = "garantir_hooks_de_recurso";
                let usos = texto.matches(nome).count() - texto.matches(&format!("fn {nome}(")).count() - texto.matches(&format!("`{nome}`")).count();
                usos_do_recurso.extend(std::iter::repeat(caminho.clone()).take(usos));
            }
        }
        assert_eq!(chamadas_do_gancho, vec![este.clone(), este], "install_dependencias_prontas_hook e garantir_hooks_de_recurso");
        assert_eq!(usos_do_recurso, vec![motor_rs], "só a linha Resource/Load de GRUPOS_DE_EVENTO");
    }

    /// NOVO-01: um lado com o caminho nulo (hash 0) não entra nem no `resource.link` nem no `resource.copy`. Sem a guarda, ` | x` faria o shim trocar todo
    /// `ResourcePath` 0 por `x`, e `x | ` mandaria o pedido de `x` para o nulo. `RESLINK_MAP` é do processo de teste inteiro: a prova é o que a carga
    /// aceitou e a ausência do 0 na tabela, não o tamanho dela.
    #[test]
    fn par_com_caminho_nulo_nao_entra_nas_tabelas() {
        let _trava = travar_tabelas_xl();
        let nulas = " | base\\c.mesh\nbase\\a.mesh | \n | base\\orig.mesh | copy\nbase\\k.mesh |  | copy\n/ | base\\x.mesh\n";
        assert_eq!(reslink_texto(nulas, false), 0);
        assert_eq!(reslink_texto(nulas, true), 0, "a recarga aplica a mesma guarda");
        assert!(RESLINK_MAP.ler().expect("tabela do link").iter().all(|&(s, t)| s != 0 && t != 0));
        assert_eq!(reslink_alvo(0), None);
        assert_eq!(depot::original_de(0), None);
        assert_eq!(depot::original_de(resource_path_hash("base\\k.mesh")), None);
        assert!(!reslink_add(0, 1) && !reslink_add(1, 0));
        assert!(!depot::adicionar_copia(0, 1) && !depot::adicionar_copia(1, 0));
        assert_eq!(reslink_alvo(1), None);

        assert_eq!(reslink_texto("# bwms-reslink\nbase\\a.mesh | base\\c.mesh\n", false), 1);
        assert_eq!(reslink_alvo(resource_path_hash("base\\a.mesh")), Some(resource_path_hash("base\\c.mesh")));
    }

    /// NOVO-04: `GetAliases` responde nos dois sentidos, por link e por cópia, em ordem crescente e sem repetição, e a chave que está nas duas tabelas
    /// responde só pelo link. As tabelas são do processo de teste inteiro: os caminhos `bwms\f125\…` são só deste teste.
    #[test]
    fn apelidos_nos_dois_sentidos_por_link_e_por_copia() {
        let _trava = travar_tabelas_xl();
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f125\\{nome}.mesh"));
        let (a, b, c) = (h("a"), h("b"), h("c"));
        assert_eq!(reslink_texto("bwms\\f125\\a.mesh | bwms\\f125\\b.mesh\nbwms\\f125\\c.mesh | bwms\\f125\\b.mesh | copy\n", false), 2);
        let mut a_e_c = vec![a, c];
        a_e_c.sort_unstable();
        assert_eq!(reslink_aliases_of(b), a_e_c);
        assert_eq!(reslink_aliases_of(a), vec![b]);
        assert_eq!(reslink_aliases_of(c), vec![b]);
        assert!(reslink_aliases_of(h("nenhum")).is_empty());

        // Cópia `k` de `o` e depois `RegisterLink(k, z)`: o link manda em `k`, e a cópia sai da união.
        let (k, o, z) = (h("k"), h("o"), h("z"));
        assert!(depot::adicionar_copia(k, o) && reslink_add(k, z));
        assert_eq!(reslink_aliases_of(k), vec![z]);
        assert!(reslink_aliases_of(o).is_empty());
        assert_eq!(reslink_aliases_of(z), vec![k]);

        // Os links entram do maior hash para o menor, e `e` é apelido de `d` duas vezes: pelo link `e → d` e pela cópia `d` de `e`.
        let (d, e) = (h("d"), h("e"));
        let mut pedidos = ["p1", "p2", "p3", "p4"].map(h).to_vec();
        pedidos.sort_unstable_by(|x, y| y.cmp(x));
        for &p in &pedidos {
            assert!(reslink_add(p, d));
        }
        assert!(reslink_add(e, d) && depot::adicionar_copia(d, e));
        let mut esperado = pedidos.clone();
        esperado.push(e);
        esperado.sort_unstable();
        assert_eq!(reslink_aliases_of(d), esperado);
        assert_eq!(reslink_aliases_of(e), vec![d]);
    }

    /// `apply-03`, a folha: com `substituir` (o que o `Reload` da fachada de recursos passa), `reslink_texto`/`reslink_file` trocam as tabelas do `resource.link` e do
    /// `resource.copy` pelas do arquivo. O par e a cópia que saíram do arquivo somem, o par de `RegisterLink` também, e o
    /// arquivo ausente esvazia as duas. Sem `substituir` (o boot), o par de `RegisterLink` gravado antes fica. A fiação do `Reload` até aqui é testada em
    /// `mods_do_usuario::pipeline::testes`. Os caminhos `bwms\f127\…` são só deste teste.
    #[test]
    fn substituir_troca_as_tabelas_do_link_e_do_copy_pelas_do_arquivo() {
        let _trava = travar_tabelas_xl();
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f127\\{nome}.mesh"));
        let (a, b, alvo, c, d, orig, registrado, servido) = (h("a"), h("b"), h("alvo"), h("c"), h("d"), h("orig"), h("registrado"), h("servido"));

        assert!(reslink_add(registrado, servido), "RegisterLink no OnLoad, antes da carga do boot");
        let dois_e_duas = "bwms\\f127\\a.mesh | bwms\\f127\\alvo.mesh\nbwms\\f127\\b.mesh | bwms\\f127\\alvo.mesh\n\
                           bwms\\f127\\c.mesh | bwms\\f127\\orig.mesh | copy\nbwms\\f127\\d.mesh | bwms\\f127\\orig.mesh | copy\n";
        assert_eq!(reslink_texto(dois_e_duas, false), 4);
        assert_eq!([a, b, registrado].map(reslink_alvo), [Some(alvo), Some(alvo), Some(servido)]);
        assert_eq!([c, d].map(depot::original_de), [Some(orig); 2]);

        assert_eq!(reslink_texto("bwms\\f127\\a.mesh | bwms\\f127\\alvo.mesh\nbwms\\f127\\c.mesh | bwms\\f127\\orig.mesh | copy\n", true), 2);
        assert_eq!([a, b, registrado].map(reslink_alvo), [Some(alvo), None, None]);
        assert_eq!([c, d].map(depot::original_de), [Some(orig), None]);
        assert_eq!(*RESLINK_MAP.ler().expect("tabela do link"), vec![(a, alvo)]);
        assert_eq!(RESLINK.count.load(Ordering::Relaxed), 1, "a contagem que o shim lê é a da tabela nova");

        let ausente = std::env::temp_dir().join(format!("bwms-f127-sem-reslink-{}.txt", std::process::id()));
        assert!(!ausente.exists());
        reslink_file(&ausente.to_string_lossy(), false);
        assert_eq!(reslink_alvo(a), Some(alvo), "sem substituir, arquivo ausente não mexe na tabela");
        reslink_file(&ausente.to_string_lossy(), true);
        assert_eq!(RESLINK.count.load(Ordering::Relaxed), 0);
        assert!(RESLINK_MAP.ler().expect("tabela do link").is_empty());
        assert_eq!(reslink_alvo(a), None);
        assert_eq!(depot::original_de(c), None);
    }

    /// F4-05: a carga do boot com N origens e N/10 delas repetidas no fim fica com o último alvo de cada uma, e a contagem que o shim lê é a de
    /// origens distintas. Imprime, para `--nocapture`, o tempo da carga e o de 1.000.000 de buscas do shim em 4 threads (10% de acerto), com 1.000 e
    /// com 10.000 pares.
    #[test]
    fn carga_de_dez_mil_pares_fica_com_o_ultimo() {
        use std::fmt::Write;
        let _trava = travar_tabelas_xl();
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f405\\{nome}.mesh"));
        for n in [1_000, 10_000] {
            let mut texto = String::new();
            for i in 0..n {
                writeln!(texto, "bwms\\f405\\p{i}.mesh | bwms\\f405\\a{i}.mesh").expect("texto");
            }
            for i in 0..n / 10 {
                writeln!(texto, "bwms\\f405\\p{i}.mesh | bwms\\f405\\b{i}.mesh").expect("texto");
            }
            reslink_texto("", true);
            let inicio = std::time::Instant::now();
            assert_eq!(reslink_texto(&texto, false), n + n / 10);
            let carga = inicio.elapsed();
            assert_eq!(RESLINK.count.load(Ordering::Relaxed), n as u64);
            let ultimo_repetido = format!("p{}", n / 10 - 1);
            let primeiro_unico = format!("p{}", n / 10);
            let ultimo = format!("p{}", n - 1);
            assert_eq!(
                [h("p0"), h(&ultimo_repetido), h(&primeiro_unico), h(&ultimo)].map(reslink_alvo),
                [Some(h("b0")), Some(h(&format!("b{}", n / 10 - 1))), Some(h(&format!("a{}", n / 10))), Some(h(&format!("a{}", n - 1)))]
            );

            let pedidos: Vec<u64> = (0..250_000).map(|i| if i % 10 == 0 { h(&format!("p{}", i % n)) } else { h(&format!("q{i}")) }).collect();
            let inicio = std::time::Instant::now();
            let acertos: usize = std::thread::scope(|s| {
                let threads: Vec<_> = (0..4).map(|_| s.spawn(|| pedidos.iter().filter(|&&p| unsafe { reslink_lookup(p) } != p).count())).collect();
                threads.into_iter().map(|t| t.join().expect("thread de busca")).sum()
            });
            let buscas = inicio.elapsed();
            assert_eq!(acertos, 4 * 25_000);
            eprintln!("[f405] {n} pares: carga de {} linhas {carga:?}; 1.000.000 de buscas em 4 threads {buscas:?}", n + n / 10);
        }
        reslink_texto("", true);
    }

    /// F4-05: pelas entradas reais (`RegisterLink`/console e `resource.copy`), 3 pares com a mesma origem deixam um só, com o último alvo, no link e
    /// na cópia; a contagem que cada shim lê sobe 1. Os caminhos `bwms\f405u\…` são só deste teste.
    #[test]
    fn tres_pares_com_a_mesma_origem_ficam_com_o_ultimo_no_link_e_na_copia() {
        let _trava = travar_tabelas_xl();
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f405u\\{nome}.mesh"));
        let link_antes = RESLINK.count.load(Ordering::Relaxed);
        for alvo in ["a", "b", "c"] {
            assert!(reslink_add(h("pedido"), h(alvo)));
        }
        assert_eq!(reslink_alvo(h("pedido")), Some(h("c")));
        assert_eq!(RESLINK.count.load(Ordering::Relaxed), link_antes + 1);
        assert_eq!(RESLINK_MAP.ler().expect("tabela do link").iter().filter(|&&(o, _)| o == h("pedido")).count(), 1);

        let copias_antes = depot::COPIAS.ler().expect("tabela de cópias").len();
        for original in ["x", "y", "z"] {
            assert!(depot::adicionar_copia(h("copia"), h(original)));
        }
        assert_eq!(depot::original_de(h("copia")), Some(h("z")));
        assert_eq!(depot::COPIAS.ler().expect("tabela de cópias").len(), copias_antes + 1);
    }

    /// F4-05: a primitiva das duas tabelas, numa instância só do teste. O caminho com o bit desligado responde "não está" sem trava (a resposta chega
    /// com a trava de escrita presa em outra thread); o caminho que divide o bit com uma origem (colisão) vai à busca, que pega a trava e responde pela
    /// tabela. A origem que sai deixa o bit ligado e não volta a responder. O lote fica com o último alvo de cada origem por cima dos pares que já
    /// estão, conta os pares aceitos (sem caminho nulo; a origem repetida conta cada vez) e deixa a tabela ordenada; a substituição troca tudo.
    #[test]
    fn filtro_responde_acerto_erro_e_bit_colidido() {
        use std::sync::mpsc::RecvTimeoutError;
        use std::time::Duration;
        static FILTRO: [AtomicU64; PALAVRAS_DO_FILTRO] = [const { AtomicU64::new(0) }; PALAVRAS_DO_FILTRO];
        static CONTAGEM: AtomicU64 = AtomicU64::new(0);
        static TABELA: TabelaDeRedirecionamento = TabelaDeRedirecionamento::nova("teste", &FILTRO, &CONTAGEM);
        let (origem, colidida, longe) = (0x0001_0000_0000_a5a5_u64, 0x0002_0000_0000_a5a5_u64, 0x0001_0000_0000_5a5a_u64);
        let (palavra, bit) = TabelaDeRedirecionamento::posicao_no_filtro(origem);
        assert_eq!(TabelaDeRedirecionamento::posicao_no_filtro(colidida), (palavra, bit));
        assert_ne!(TabelaDeRedirecionamento::posicao_no_filtro(longe), (palavra, bit));

        assert_eq!(TABELA.alvo(origem), None);
        assert!(TABELA.acrescentar(origem, 0x22));
        assert_eq!((TABELA.alvo(origem), CONTAGEM.load(Ordering::Relaxed)), (Some(0x22), 1));
        assert_eq!(TABELA.alvo(colidida), None);

        let escrita = TABELA.pares.write().expect("trava de escrita");
        let (tx, rx) = std::sync::mpsc::channel();
        let erro = std::thread::spawn({
            let tx = tx.clone();
            move || tx.send(("longe", TABELA.alvo(longe))).expect("canal")
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).expect("o erro pelo filtro não espera a trava"), ("longe", None));
        let colisao = std::thread::spawn(move || tx.send(("colidida", TABELA.alvo(colidida))).expect("canal"));
        assert_eq!(rx.recv_timeout(Duration::from_millis(200)), Err(RecvTimeoutError::Timeout), "o bit colidido vai à busca, sob a trava");
        drop(escrita);
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).expect("a busca termina com a trava solta"), ("colidida", None));
        erro.join().expect("thread do erro");
        colisao.join().expect("thread da colisão");

        assert!(TABELA.remover(origem));
        assert!(!TABELA.remover(origem));
        assert_ne!(FILTRO[palavra].load(Ordering::Relaxed) & bit, 0, "a origem que sai deixa o bit");
        assert_eq!((TABELA.alvo(origem), CONTAGEM.load(Ordering::Relaxed)), (None, 0));

        let (a, b, c) = (0x0000_0000_0007_000a_u64, 0x0000_0000_0001_000b_u64, 0x0000_0000_0004_000c_u64);
        assert_eq!(TABELA.acrescentar_lote(&[(a, 1), (b, 2), (a, 3), (0, 4), (c, 0)]), 3);
        assert_eq!(*TABELA.ler().expect("tabela"), vec![(b, 2), (a, 3)]);
        assert_eq!(TABELA.acrescentar_lote(&[(b, 9), (c, 5), (b, 8)]), 3);
        assert_eq!(*TABELA.ler().expect("tabela"), vec![(b, 8), (c, 5), (a, 3)]);
        assert_eq!(CONTAGEM.load(Ordering::Relaxed), 3);
        assert_eq!([a, b, c].map(|o| TABELA.alvo(o)), [Some(3), Some(8), Some(5)]);

        assert_eq!(TABELA.substituir(&[(c, 6), (0, 1)]), 1);
        assert_eq!(*TABELA.ler().expect("tabela"), vec![(c, 6)]);
        assert_eq!([a, b, c].map(|o| TABELA.alvo(o)), [None, None, Some(6)]);
        TABELA.limpar();
        assert_eq!((TABELA.alvo(c), CONTAGEM.load(Ordering::Relaxed)), (None, 0));
    }

    /// x0-x7 como o original de teste os recebeu; 64 bytes voltam pela memória que o x8 aponta, como o resultado do construtor de `ResourcePath`.
    #[repr(C)]
    #[derive(Debug, PartialEq, Clone, Copy)]
    struct Argumentos([u64; 8]);

    extern "C" fn original_que_devolve_os_argumentos(x0: u64, x1: u64, x2: u64, x3: u64, x4: u64, x5: u64, x6: u64, x7: u64) -> Argumentos {
        Argumentos([x0, x1, x2, x3, x4, x5, x6, x7])
    }

    /// Um path com o bit desligado no `RESLINK_FILTRO` agora; o filtro é do processo de teste inteiro e nunca desliga bit.
    fn path_com_bit_desligado(nome: &str) -> u64 {
        (0..)
            .map(|i| resource_path_hash(&format!("bwms\\f502\\{nome}{i}.mesh")))
            .find(|&p| {
                let (palavra, bit) = TabelaDeRedirecionamento::posicao_no_filtro(p);
                RESLINK_FILTRO[palavra].load(Ordering::Relaxed) & bit == 0
            })
            .expect("path com o bit desligado")
    }

    /// F5-02: o shim pelo asm, com um original de teste no `ORIG_RESLINK`. Com a tabela vazia, com o bit do path desligado, com o bit colidido e no
    /// acerto, o original recebe x1-x7 e o x8 como o chamador os passou (o resultado de 64 bytes volta pelo x8 do chamador); só o acerto troca o x0 e
    /// conta o swap. 64 origens espalhadas acertam todas: a posição que o asm lê no filtro é a de `posicao_no_filtro`. Os caminhos `bwms\f502\…` são só
    /// deste teste.
    #[test]
    fn shim_troca_so_o_path_da_tabela_e_preserva_os_argumentos() {
        let _trava = travar_tabelas_xl();
        ORIG_RESLINK.store(original_que_devolve_os_argumentos as *const () as usize as u64, Ordering::Relaxed);
        let shim: extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> Argumentos = unsafe { std::mem::transmute(reslink_shim as *const ()) };
        let chamar = |path: u64| shim(path, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77);
        let esperado = |path: u64| Argumentos([path, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f502\\{nome}.mesh"));
        let (pedido, servido) = (h("pedido"), h("servido"));
        let colidido = pedido ^ (1 << 40);

        reslink_texto("", true);
        let swaps = RESLINK.swapcnt.load(Ordering::Relaxed);
        assert_eq!(chamar(pedido), esperado(pedido), "tabela vazia");
        assert!(reslink_add(pedido, servido));
        assert_eq!(TabelaDeRedirecionamento::posicao_no_filtro(colidido), TabelaDeRedirecionamento::posicao_no_filtro(pedido));
        assert_eq!(chamar(pedido), esperado(servido), "acerto");
        assert_eq!(chamar(colidido), esperado(colidido), "bit colidido, fora da tabela");
        let fora = path_com_bit_desligado("fora");
        assert_eq!(chamar(fora), esperado(fora), "bit desligado");
        assert_eq!(RESLINK.swapcnt.load(Ordering::Relaxed), swaps + 1, "só o acerto conta swap");

        let origens: Vec<u64> = (0..64).map(|i| h(&format!("origem{i}"))).collect();
        for &o in &origens {
            assert!(reslink_add(o, servido));
        }
        assert!(origens.iter().all(|&o| chamar(o) == esperado(servido)), "toda origem da tabela acerta pelo asm");

        reslink_texto("", true);
        ORIG_RESLINK.store(0, Ordering::Relaxed);
    }

    /// F5-02: o shim do dev grava o path no ring e conta a construção, e o swap continua pelo `reslink_shim`.
    #[cfg(feature = "devtools")]
    #[test]
    fn shim_de_dev_grava_o_ring_e_segue_pelo_do_publico() {
        let _trava = travar_tabelas_xl();
        ORIG_RESLINK.store(original_que_devolve_os_argumentos as *const () as usize as u64, Ordering::Relaxed);
        let shim: extern "C" fn(u64, u64, u64, u64, u64, u64, u64, u64) -> Argumentos =
            unsafe { std::mem::transmute(reslink_shim_com_ring as *const ()) };
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f502d\\{nome}.mesh"));
        let (pedido, servido, fora) = (h("pedido"), h("servido"), path_com_bit_desligado("fora_dev"));
        reslink_texto("", true);
        assert!(reslink_add(pedido, servido));
        let idx = RESLINK.idx.load(Ordering::Relaxed);
        assert_eq!(shim(pedido, 1, 2, 3, 4, 5, 6, 7), Argumentos([servido, 1, 2, 3, 4, 5, 6, 7]));
        assert_eq!(shim(fora, 1, 2, 3, 4, 5, 6, 7), Argumentos([fora, 1, 2, 3, 4, 5, 6, 7]));
        assert_eq!(RESLINK.idx.load(Ordering::Relaxed), idx + 2);
        assert_eq!(RESLINK.ring[(idx & 63) as usize].load(Ordering::Relaxed), pedido);
        assert_eq!(RESLINK.ring[((idx + 1) & 63) as usize].load(Ordering::Relaxed), fora);
        reslink_texto("", true);
        ORIG_RESLINK.store(0, Ordering::Relaxed);
    }

    /// F5-02: custo do shim por construção de `ResourcePath`, impresso para `--nocapture`, com 1 e com 4 threads, cada thread com 1.000.000 de
    /// construções: tabela vazia; 1.000 pares e o path fora da tabela com o bit desligado (quase toda construção com `bwms-reslink.txt`); 1.000 pares
    /// e acerto. Cada chamada confere o resultado. É microbenchmark do shim, não medida do jogo.
    #[test]
    fn shim_custo_por_construcao() {
        use std::fmt::Write;
        extern "C" fn original_identidade(path: u64) -> u64 {
            path
        }
        const CHAMADAS: usize = 1_000_000;
        let _trava = travar_tabelas_xl();
        ORIG_RESLINK.store(original_identidade as *const () as usize as u64, Ordering::Relaxed);
        let shim: extern "C" fn(u64) -> u64 = unsafe { std::mem::transmute(reslink_shim as *const ()) };
        let medir = |rotulo: &str, casos: &[(u64, u64)]| {
            for threads in [1, 4] {
                let inicio = std::time::Instant::now();
                std::thread::scope(|s| {
                    for _ in 0..threads {
                        s.spawn(|| {
                            let certos = casos.iter().cycle().take(CHAMADAS).filter(|&&(p, e)| shim(p) == e).count();
                            assert_eq!(certos, CHAMADAS, "{rotulo}");
                        });
                    }
                });
                let ns = inicio.elapsed().as_nanos() as f64 / CHAMADAS as f64;
                eprintln!("[f502] {rotulo}, {threads} thread(s): {ns:.2} ns por construção em cada thread");
            }
        };
        let h = |nome: &str| resource_path_hash(&format!("bwms\\f502b\\{nome}.mesh"));

        reslink_texto("", true);
        let vazia: Vec<(u64, u64)> = (0..4096).map(|i| h(&format!("q{i}"))).map(|p| (p, p)).collect();
        medir("tabela vazia", &vazia);

        let mut texto = String::new();
        for i in 0..1_000 {
            writeln!(texto, "bwms\\f502b\\p{i}.mesh | bwms\\f502b\\a{i}.mesh").expect("texto");
        }
        assert_eq!(reslink_texto(&texto, false), 1_000);
        let fora: Vec<(u64, u64)> = (0..4096).map(|i| path_com_bit_desligado(&format!("b{i}_"))).map(|p| (p, p)).collect();
        medir("1.000 pares, path fora com o bit desligado", &fora);
        let acerto: Vec<(u64, u64)> = (0..1_000).map(|i| (h(&format!("p{i}")), h(&format!("a{i}")))).collect();
        medir("1.000 pares, acerto", &acerto);

        reslink_texto("", true);
        ORIG_RESLINK.store(0, Ordering::Relaxed);
    }

    /// F1-29: `ResourceTarget.Path` com `regex:` que o motor recusa dá 1 aviso na criação e 0 por evento. O aviso traz o texto, cortado em 160
    /// caracteres, e o motivo em PT-BR; literal, glob, regex aceita e hash sem texto não avisam. O despacho continua sem casar e sem avisar: a
    /// função é privada deste módulo, e a fonte mostra uma chamada só, dentro de `tramp_rt_path`, que roda uma vez por alvo.
    #[test]
    fn padrao_recusado_avisa_na_fabrica_e_nao_no_despacho() {
        let alvo = caminho_registrar(r"regex:^\d$");
        let aviso = rt_path_recusado(alvo).expect("`\\d` fica fora do subconjunto");
        assert!(aviso.starts_with(r"[rt] ResourceTarget.Path 'regex:^\d$' recusado: recurso fora do subconjunto"), "{aviso}");
        assert!(!crate::corpo_e_rosto::caminho_casa(alvo, caminho_registrar("7")), "o despacho trata o recusado como não casou");

        let longo = caminho_registrar(&format!("regex:{}", "ã".repeat(2100)));
        let aviso = rt_path_recusado(longo).expect("mais de 4096 bytes é recusado");
        let citado = &aviso[aviso.find('\'').expect("abre aspas") + 1..aviso.rfind("' recusado").expect("fecha aspas")];
        assert_eq!(citado.chars().count(), 160);
        assert!(aviso.ends_with("divida em padrões menores"), "{aviso}");

        for (texto, caso) in [
            (r"base\bwms_teste_f129\a.mesh", "literal"),
            (r"base\bwms_teste_f129\*.mesh", "glob"),
            (r"regex:^base\\bwms_teste_f129\\[0-9]$", "regex aceita"),
        ] {
            assert_eq!(rt_path_recusado(caminho_registrar(texto)), None, "{caso}");
        }
        assert_eq!(rt_path_recusado(0x0f12_9f12_9f12_9f12), None, "hash sem texto registrado");

        const MARCA_DO_TESTE: &str = "#[cfg(test)]\nmod testes {";
        let fonte = include_str!("mod.rs");
        let producao = &fonte[..fonte.find(MARCA_DO_TESTE).expect("módulo de teste de arquivos_do_jogo")];
        let fabrica = &producao[producao.find("fn tramp_rt_path(").expect("tramp_rt_path")..];
        let fabrica = &fabrica[..fabrica.find("\n}\n").expect("fim de tramp_rt_path")];
        let chamadas = producao.matches("rt_path_recusado(").count() - producao.matches("fn rt_path_recusado(").count();
        assert_eq!((chamadas, fabrica.matches("rt_path_recusado(").count()), (1, 1), "o aviso sai só da fábrica");
        assert!(!include_str!("depot.rs").contains("rt_path_recusado"), "o submódulo também enxerga a função privada");
    }

    /// F6-05: os ganchos acham pelo índice as mesmas linhas, na mesma ordem, que a varredura linear de `patches` acha. Cobre o caminho com alvo
    /// repetido e intercalado, ausente e `0`; a lista `.app`; e o casamento por caminho ou pela instância do token do alvo, com o filtro do 51f (caminho
    /// `0` não casa pelo caminho) e o do 17e. Uma tabela à mão e 40 tabelas sorteadas.
    #[test]
    fn indice_por_alvo_acha_as_mesmas_linhas_que_a_varredura() {
        let mascara_51f = A17_PROP_COMPONENTS | A17_PROP_ENTITY;
        // Template carregado de cada token do alvo; `0x200` ainda carrega.
        let instancia = |t: [u64; 2]| match t[0] {
            0x300 => 0x3000,
            0x400 => 0x4000,
            _ => 0,
        };
        let conferir = |patches: &[PatchDeRecurso], caminhos: &[u64], rotulo: &str| {
            let tabela = TabelaDePatches::nova(patches.to_vec());
            let varredura = |f: &dyn Fn(&PatchDeRecurso) -> bool| patches.iter().filter(|p| f(p)).map(|p| p.patch).collect::<Vec<u64>>();
            assert_eq!(tabela.apps().map(|p| p.patch).collect::<Vec<_>>(), varredura(&|p| p.app), "{rotulo}: apps");
            for &c in caminhos {
                assert_eq!(tabela.do_alvo(c).map(|p| p.patch).collect::<Vec<_>>(), varredura(&|p| p.alvo == c), "{rotulo}: alvo {c:#x}");
                for tpl in [0x3000, 0x4000, 0x5000] {
                    let esperado = varredura(&|p| p.props & mascara_51f != 0 && ((c != 0 && p.alvo == c) || instancia(p.alvo_token) == tpl));
                    let achado = tabela.do_alvo_ou_da_instancia((c != 0).then_some(c), tpl, |p| p.props & mascara_51f != 0, instancia);
                    assert_eq!(achado.map(|p| p.patch).collect::<Vec<_>>(), esperado, "{rotulo}: 51f, alvo {c:#x}, template {tpl:#x}");
                    let esperado = varredura(&|p| p.props & A17_PROP_COMPONENTS != 0 && (p.alvo == c || instancia(p.alvo_token) == tpl));
                    let achado = tabela.do_alvo_ou_da_instancia(Some(c), tpl, |p| p.props & A17_PROP_COMPONENTS != 0, instancia);
                    assert_eq!(achado.map(|p| p.patch).collect::<Vec<_>>(), esperado, "{rotulo}: 17e, alvo {c:#x}, template {tpl:#x}");
                }
            }
        };
        let p = |patch: u64, alvo: u64, props: u32, alvo_token: u64, app: bool| PatchDeRecurso {
            mod_id: format!("m{patch}"),
            patch,
            alvo,
            props,
            token: [patch, 0],
            alvo_token: [alvo_token, 0],
            app,
        };
        let a_mao = [
            p(1, 30, A17_PROP_COMPONENTS, 0x300, false),
            p(2, 10, A17_PROP_APPEARANCES, 0, true),
            p(3, 30, u32::MAX, 0x300, false),
            p(4, 20, A17_PROP_ENTITY, 0x200, false),
            p(5, 10, A17_PROP_COMPONENTS, 0, true),
            p(6, 0, A17_PROP_COMPONENTS, 0, false),
            p(7, 30, A17_PROP_APPEARANCES, 0x300, false),
            p(8, 40, A17_PROP_COMPONENTS, 0x400, true),
            p(9, 20, A17_PROP_COMPONENTS, 0x200, false),
            p(10, 10, A17_PROP_ENTITY, 0x300, false),
        ];
        conferir(&a_mao, &[0, 10, 20, 30, 40, 50, u64::MAX], "à mão");
        // Caminho e instância juntos saem na ordem da tabela: alvo 10 com o template 0x3000 dá 1, 3, 5, 10.
        let t = TabelaDePatches::nova(a_mao.to_vec());
        assert_eq!(t.do_alvo_ou_da_instancia(Some(10), 0x3000, |p| p.props & mascara_51f != 0, instancia).map(|p| p.patch).collect::<Vec<_>>(), [1, 3, 5, 10]);
        conferir(&[], &[0, 10], "vazia");

        let mut estado = 0x9e37_79b9_7f4a_7c15u64;
        let mut sorteio = |teto: u64| {
            estado ^= estado << 13;
            estado ^= estado >> 7;
            estado ^= estado << 17;
            estado % teto
        };
        let props = [A17_PROP_COMPONENTS, A17_PROP_ENTITY, A17_PROP_APPEARANCES, u32::MAX, A17_PROP_BLOCKS, A17_PROP_COMPONENTS | A17_PROP_VISUALTAGS];
        let tokens = [0, 0, 0x200, 0x300, 0x400];
        for rodada in 0..40 {
            let n = sorteio(60) as usize;
            let tabela: Vec<PatchDeRecurso> = (0..n)
                .map(|i| p(i as u64 + 1, sorteio(12), props[sorteio(props.len() as u64) as usize], tokens[sorteio(tokens.len() as u64) as usize], sorteio(3) == 0))
                .collect();
            conferir(&tabela, &(0..13).collect::<Vec<u64>>(), &format!("sorteio {rodada}"));
        }
    }

    /// F6-05: custo de uma busca de gancho de carga, impresso com `--nocapture`, na forma do 51a (trava de leitura, filtro de props, `Vec` coletado):
    /// a varredura linear de `patches`, o índice ordenado de `TabelaDePatches`, um `HashMap<u64, Vec<u32>>` por alvo e o piso (só a trava).
    /// Tabelas de 1 a 3.240 patches (1 a 4 é o que os boots de dev logaram, 240 é o corpus de referência do F4-03 e 3.240 o teto do sintético dele),
    /// com ~1,5 patch por alvo. Duas cargas: recurso fora da tabela (quase toda carga do jogo) e alvo da tabela. Mediana de 5 rodadas intercaladas, e o
    /// custo de montar os índices na publicação. É microbenchmark offline, não medida do jogo:
    /// `cargo test --release --lib -- --ignored --nocapture busca_por_alvo_custo`.
    #[test]
    #[ignore]
    fn busca_por_alvo_custo() {
        use std::hint::black_box;
        type Mapa = std::collections::HashMap<u64, Vec<u32>, std::hash::BuildHasherDefault<std::hash::DefaultHasher>>;
        type Achado = Vec<(u64, [u64; 2], u32)>;
        const MASCARA: u32 = A17_PROP_APPEARANCES | A17_PROP_DEPENDENCIES | A17_PROP_VISUALTAGS;
        fn medir<F: Fn(u64) -> Achado>(busca: F, casos: &[u64], chamadas: usize) -> f64 {
            let inicio = std::time::Instant::now();
            let mut achados = 0usize;
            for &c in casos.iter().cycle().take(chamadas) {
                achados += black_box(busca(black_box(c))).len();
            }
            black_box(achados);
            inicio.elapsed().as_nanos() as f64 / chamadas as f64
        }
        let mediana = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let props = [A17_PROP_APPEARANCES, A17_PROP_COMPONENTS, u32::MAX, A17_PROP_COMPONENTS | A17_PROP_ENTITY, A17_PROP_VISUALTAGS, A17_PROP_BLOCKS];
        let h = |nome: String| resource_path_hash(&format!("bwms\\f605\\{nome}"));
        for n in [1usize, 2, 4, 8, 16, 32, 240, 1000, 3240] {
            let patches: Vec<PatchDeRecurso> = (0..n)
                .map(|i| PatchDeRecurso {
                    mod_id: format!("mod{}", i % 7),
                    patch: h(format!("p{i}.ent")),
                    alvo: h(format!("a{}.ent", i * 2 / 3)),
                    props: props[i % props.len()],
                    token: [i as u64 + 1, 0],
                    alvo_token: [0; 2],
                    app: i % 5 == 0,
                })
                .collect();
            let mut mapa = Mapa::default();
            for (i, p) in patches.iter().enumerate() {
                mapa.entry(p.alvo).or_default().push(i as u32);
            }
            let copias: Vec<Vec<PatchDeRecurso>> = (0..32).map(|_| patches.clone()).collect();
            let inicio = std::time::Instant::now();
            for c in copias {
                black_box(TabelaDePatches::nova(c));
            }
            let publicar_us = inicio.elapsed().as_nanos() as f64 / 32_000.0;
            let mut dentro: Vec<u64> = patches.iter().map(|p| p.alvo).collect();
            dentro.dedup();
            let trava = std::sync::RwLock::new(TabelaDePatches::nova(patches));
            let fora: Vec<u64> = (0..4096).map(|i| h(format!("fora{i}.ent"))).collect();
            let varredura = |c: u64| -> Achado {
                match trava.try_read() {
                    Ok(g) => g.patches.iter().filter(|p| p.alvo == c && p.props & MASCARA != 0).map(|p| (p.patch, p.token, p.props)).collect(),
                    Err(_) => Vec::new(),
                }
            };
            let indice = |c: u64| -> Achado {
                match trava.try_read() {
                    Ok(g) => g.do_alvo(c).filter(|p| p.props & MASCARA != 0).map(|p| (p.patch, p.token, p.props)).collect(),
                    Err(_) => Vec::new(),
                }
            };
            let por_hash = |c: u64| -> Achado {
                match trava.try_read() {
                    Ok(g) => mapa
                        .get(&c)
                        .into_iter()
                        .flatten()
                        .filter_map(|&i| g.patches.get(i as usize))
                        .filter(|p| p.props & MASCARA != 0)
                        .map(|p| (p.patch, p.token, p.props))
                        .collect(),
                    Err(_) => Vec::new(),
                }
            };
            let piso = |c: u64| -> Achado {
                match trava.try_read() {
                    Ok(g) => {
                        black_box((&*g, c));
                        Vec::new()
                    }
                    Err(_) => Vec::new(),
                }
            };
            for &c in fora.iter().take(64).chain(&dentro) {
                let v = varredura(c);
                assert_eq!(indice(c), v);
                assert_eq!(por_hash(c), v);
            }
            let chamadas = (200_000_000 / (n + 50)).clamp(20_000, 1_000_000);
            for (rotulo, casos) in [("carga fora da tabela", &fora), ("carga de alvo", &dentro)] {
                let (mut v, mut i, mut m, mut t) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                for rodada in 0..5 {
                    for k in 0..4 {
                        match (rodada + k) % 4 {
                            0 => v.push(medir(varredura, casos, chamadas)),
                            1 => i.push(medir(indice, casos, chamadas)),
                            2 => m.push(medir(por_hash, casos, chamadas)),
                            _ => t.push(medir(piso, casos, chamadas)),
                        }
                    }
                }
                let (v, i, m, t) = (mediana(v), mediana(i), mediana(m), mediana(t));
                eprintln!(
                    "[f605] {n:>5} patches, {rotulo}: só a trava {t:.1} ns; varredura {v:.1} ns, índice {i:.1} ns ({:+.0}%), HashMap {m:.1} ns ({:+.0}%)",
                    (i / v - 1.0) * 100.0,
                    (m / v - 1.0) * 100.0
                );
            }
            eprintln!("[f605] {n:>5} patches: montar os índices na publicação {publicar_us:.2} µs");
        }
    }

    /// F7-26: o que o boot (`bwms_primitives::instalacao::apply::gravar_tabelas_de_runtime`, com os `.xl` da `Mods/`) grava em `red4ext/`, os leitores do runtime leem, linha por linha, pelo
    /// mesmo formato (`bwms_primitives::tabelas`). O `reslink_file` instala os ganchos do depot e o `questphase_tabela` e o `cust_tabela` pedem o RTTI
    /// do jogo depois de ler: aqui o reslink entra pelo `reslink_texto`, que o `reslink_file` chama, e as outras duas tabelas passam pela leitura que eles
    /// fazem, sem recusa. Os caminhos `bwms\f726\…` são só deste teste.
    #[test]
    fn o_que_o_boot_grava_os_leitores_do_runtime_leem() {
        use bwms_primitives::tabelas;
        let _trava = travar_tabelas_xl();
        let h = |nome: &str| bwms_primitives::resource_path_hash(&format!("bwms\\f726\\{nome}"));
        let g = std::env::temp_dir().join(format!("bwms-f726-install-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let (a, b) = (g.join("Mods/ModA"), g.join("Mods/ModB"));
        for (pasta, archive) in [(&a, "a.archive"), (&b, "b.archive")] {
            std::fs::create_dir_all(pasta).expect("pasta do mod");
            std::fs::write(pasta.join(archive), b"x").expect("archive do mod");
        }
        std::fs::write(
            a.join("alfa.xl"),
            "resource:\n  link:\n    bwms\\f726\\fake.mesh: bwms\\f726\\real.mesh\n  copy:\n    bwms\\f726\\orig.mesh: bwms\\f726\\copia.mesh\n  scope:\n    \
             bwms\\f726\\grupo.ent:\n      - bwms\\f726\\x.ent\n  fix:\n    bwms\\f726\\m.mesh:\n      names:\n        old_mat: new_mat\n      paths:\n        \
             bwms\\f726\\antigo.app: bwms\\f726\\novo.app\nfactories:\n  - bwms\\f726\\f.csv\njournal:\n  - bwms\\f726\\q.journal\nlocalization:\n  onscreens:\n    \
             en-us: bwms\\f726\\en.json\ncustomizations:\n  female: bwms\\f726\\f.inkcharcustomization\n",
        )
        .expect(".xl do ModA");
        std::fs::write(
            b.join("beta.archive.xl"),
            "resource:\n  patch:\n    bwms\\f726\\p.ent:\n      props: [ components ]\n      targets: [ bwms\\f726\\grupo.ent, bwms\\f726\\z.ent ]\nquest:\n  phases:\n    \
             - path: bwms\\f726\\fase.questphase\n      parent: bwms\\f726\\pai.questphase\n      input:\n        node: [9, 17]\n        socket: CutDestination\n      \
             intercept: true\nstreaming:\n  sectors:\n    - path: bwms\\f726\\s.streamingsector\n      expectedNodes: 5\n      nodeDeletions:\n        - index: 1\n          \
             type: worldStaticMeshNode\n        - type: worldInstancedMeshNode\n          index: 2\n          expectedInstances: 4\n          instanceDeletions: [0, 3]\n",
        )
        .expect(".xl do ModB");
        let pasta = bwms_primitives::instalacao::apply::ler_pasta_mods(&g);
        let (_, avisos) = bwms_primitives::instalacao::apply::gravar_tabelas_de_runtime(&g, &pasta).expect("tabelas do jogo falso");
        assert!(avisos.is_empty(), "{avisos:?}");
        let red4 = g.join(bwms_primitives::PASTA_DO_RUNTIME);
        let arquivo = |t: tabelas::Tabela| red4.join(t.arquivo);
        let texto = |t: tabelas::Tabela| std::fs::read_to_string(arquivo(t)).expect("tabela que o boot grava");
        let caminho = |t: tabelas::Tabela| arquivo(t).to_string_lossy().into_owned();
        for t in tabelas::TODAS {
            assert!(arquivo(t).is_file(), "{} não foi gravada", t.arquivo);
        }

        assert_eq!(reslink_texto(&texto(tabelas::RESLINK), true), 2);
        assert_eq!(reslink_alvo(h("fake.mesh")), Some(h("real.mesh")));
        assert_eq!(depot::original_de(h("copia.mesh")), Some(h("orig.mesh")));

        crate::mundo::streaming_file(&caminho(tabelas::STREAMING));
        let setor: Vec<(String, u64, usize, usize, String, String, String)> = crate::mundo::DELECOES_DE_SETOR
            .lock()
            .expect("deleções de setor")
            .iter()
            .map(|d| (d.modulo.clone(), d.setor, d.esperado, d.indice, d.tipo.clone(), d.op.clone(), d.valores.clone()))
            .collect();
        let delecao = |indice, tipo: &str, op: &str, valores: &str| {
            ("Mods/ModB/beta.archive".to_string(), h("s.streamingsector"), 5, indice, tipo.to_string(), op.to_string(), valores.to_string())
        };
        assert_eq!(setor, vec![delecao(1, "worldStaticMeshNode", "", ""), delecao(2, "worldInstancedMeshNode", "subdel", "4 0 3")]);

        crate::mundo::JOURNAL_MODS.lock().expect("journals de mod").clear();
        assert_eq!(crate::mundo::journal_tabela(&caminho(tabelas::JOURNAL), true), 1);
        let journals: Vec<(String, String)> =
            crate::mundo::JOURNAL_MODS.lock().expect("journals de mod").iter().map(|j| (j.mod_id.clone(), j.caminho.clone())).collect();
        assert_eq!(journals, [("Mods/ModA/alfa".to_string(), "bwms\\f726\\q.journal".to_string())]);

        let loc = crate::localizacao::ler_tabela(&texto(tabelas::LOCALIZACAO));
        let linha_de_loc =
            crate::localizacao::Linha { modulo: "Mods/ModA/alfa".into(), tipo: "onscreens".into(), idioma: "en-us".into(), caminho: "bwms\\f726\\en.json".into() };
        assert_eq!(loc, vec![linha_de_loc]);

        let patch = texto(tabelas::RESOURCE_PATCH);
        let linha_de_patch = |alvo| tabelas::ResourcePatch { mod_id: "Mods/ModB/beta.archive", patch: "bwms\\f726\\p.ent", alvo, props: A17_PROP_COMPONENTS };
        assert_eq!(linhas_da_tabela_de_patches(&patch), vec![linha_de_patch("bwms\\f726\\x.ent"), linha_de_patch("bwms\\f726\\z.ent")]);

        let (nomes, caminhos) = crate::mods_do_usuario::pipeline::resfixes_linhas(&texto(tabelas::RESFIXES));
        let cname = |s: &str| bwms_primitives::fnv1a64(s.as_bytes());
        assert_eq!(nomes, std::collections::HashMap::from([(h("m.mesh"), vec![(cname("old_mat"), cname("new_mat"))])]));
        assert_eq!(caminhos, std::collections::HashMap::from([(h("m.mesh"), vec![(h("antigo.app"), h("novo.app"))])]));

        assert_eq!(crate::corpo_e_rosto::es20_escopos_linhas(&texto(tabelas::RESSCOPES)), vec![(h("grupo.ent"), h("x.ent"))]);

        let fases = texto(tabelas::QUESTPHASE);
        let fase: Vec<_> = tabelas::ler(&fases, tabelas::Questphase::ler).collect();
        assert_eq!(
            fase,
            vec![Ok(tabelas::Questphase {
                mod_id: "Mods/ModB/beta.archive",
                pai: "bwms\\f726\\pai.questphase",
                fase: "bwms\\f726\\fase.questphase",
                entrada: vec![9, 17],
                socket_entrada: Some("CutDestination"),
                saida: Vec::new(),
                socket_saida: None,
                intercept: true,
            })]
        );
        let customizacoes = texto(tabelas::CUSTOMIZATIONS);
        let customizacao: Vec<_> = tabelas::ler(&customizacoes, tabelas::Customizacao::ler).collect();
        assert_eq!(customizacao, vec![Ok(tabelas::Customizacao { feminino: true, caminho: "bwms\\f726\\f.inkcharcustomization" })]);

        // As tabelas do processo de teste voltam vazias, como a recarga sem arquivo as deixa.
        let ausente = g.join("ausente.txt").to_string_lossy().into_owned();
        reslink_texto("", true);
        crate::mundo::streaming_file(&ausente);
        crate::mundo::journal_tabela(&ausente, true);
        std::fs::remove_dir_all(&g).expect("jogo falso do teste");
    }

    /// `soltura` segue o destrutor `0x10002914c` → `0x10002c3d4(cs, 0)`: no modo 1 ele zera o cabeçalho e o 1º byte e só chama o
    /// `Free<PoolString>` com capacidade de 1 a 0xFFFF_FFF7 (`cbz w10` em `0x10002c50c`, `cmn w10, #9; b.hi` em `0x10002c520`); no modo 0
    /// só zera; o modo 2 (vista) ele nem toca; no modo 3 zera só o tamanho e o 1º byte do texto apontado.
    #[test]
    fn soltura_segue_o_destrutor_do_motor() {
        let cstring = |ponteiro_ou_texto: [u8; 8], capacidade: u32, cabecalho: u32| {
            let mut b = [0u8; 0x20];
            b[..8].copy_from_slice(&ponteiro_ou_texto);
            b[0x10..0x14].copy_from_slice(&capacidade.to_le_bytes());
            b[0x14..0x18].copy_from_slice(&cabecalho.to_le_bytes());
            b
        };
        // Nos modos 0 e 1 o destrutor deixa o resto do objeto como estava: só o 1º byte e o cabeçalho viram zero.
        let destruida = |antes: [u8; 0x20]| {
            let mut b = antes;
            b[0] = 0;
            b[0x14..0x18].copy_from_slice(&0u32.to_le_bytes());
            b
        };
        let bloco = 0x0000_0070_2561_0780u64.to_le_bytes();
        let caminho_longo = cstring(bloco, 0x57, 0x4000_0050);
        assert_eq!(soltura(&caminho_longo, &destruida(caminho_longo)), Soltura::Bloco);
        assert_eq!(soltura(&caminho_longo, &caminho_longo), Soltura::NaoEsvaziou, "destrutor não rodou");

        let curto = cstring(*b"/a/b.ar\0", 0, 12);
        assert_eq!(soltura(&curto, &destruida(curto)), Soltura::Inline);
        let mut sem_zero = destruida(curto);
        sem_zero[0] = b'/';
        assert_eq!(soltura(&curto, &sem_zero), Soltura::NaoEsvaziou, "1º byte não zerado");

        let menor = cstring(bloco, 1, 0x4000_0001);
        assert_eq!(soltura(&menor, &destruida(menor)), Soltura::Bloco);
        let sem_capacidade = cstring(bloco, 0, 0x4000_0050);
        assert_eq!(
            soltura(&sem_capacidade, &destruida(sem_capacidade)),
            Soltura::ForaDoConstrutor,
            "capacidade 0: o destrutor zera sem chamar o Free"
        );
        let limite = cstring(bloco, 0xFFFF_FFF7, 0x4000_0050);
        assert_eq!(soltura(&limite, &destruida(limite)), Soltura::Bloco);
        let sentinela = cstring(bloco, 0xFFFF_FFF8, 0x4000_0050);
        assert_eq!(soltura(&sentinela, &destruida(sentinela)), Soltura::ForaDoConstrutor, "capacidade em que o destrutor não solta");
        // Modo 2: `b.eq` em `0x100029158` pula o `0x10002c3d4`, e o objeto fica intacto. Modo 3: o `ands` em `0x10002c56c` guarda os 2 bits
        // do modo e o `strb` zera o 1º byte do texto apontado, não o do objeto.
        let vista = cstring(bloco, 0x57, 0x8000_0050);
        assert_eq!(soltura(&vista, &vista), Soltura::NaoEsvaziou, "modo 2: o destrutor não toca");
        let modo3 = cstring(bloco, 0x57, 0xC000_0050);
        let modo3_destruida = cstring(bloco, 0x57, 0xC000_0000);
        assert_eq!(soltura(&modo3, &modo3_destruida), Soltura::NaoEsvaziou, "modo 3: o destrutor zera só o tamanho");
        // Objeto vazio depois de modo 2 ou 3 o destrutor do desmonte não produz.
        assert_eq!(soltura(&vista, &destruida(vista)), Soltura::DestrutorDiferente);
        assert_eq!(soltura(&modo3, &destruida(modo3)), Soltura::DestrutorDiferente);
    }

    /// Em `montar_no_grupo`, a `CString` do caminho nasce do construtor, vai ao `LoadArchives` e é destruída antes do fim da volta, sem
    /// saída no meio: todo caminho construído é solto. Construir pede o binário do jogo, então o teste confere o texto de `src/`.
    #[test]
    fn montar_no_grupo_destroi_a_cstring_depois_do_loadarchives() {
        const MARCA_DO_TESTE: &str = "#[cfg(test)]\nmod testes {";
        let fonte = include_str!("mod.rs");
        let producao = &fonte[..fonte.find(MARCA_DO_TESTE).expect("módulo de teste de arquivos_do_jogo")];
        let inicio = producao.find("unsafe fn montar_no_grupo(").expect("`montar_no_grupo` sumiu");
        let corpo = &producao[inicio..inicio + producao[inicio..].find("\n}\n").expect("fim de `montar_no_grupo`")];
        let unico = |trecho: &str| {
            assert_eq!(corpo.matches(trecho).count(), 1, "`{trecho}` tem de aparecer uma vez em `montar_no_grupo`");
            corpo.find(trecho).expect("conferido acima")
        };
        let construtor = unico("crate::rebase(0x1_0002_ccac)");
        let load_archives = unico("crate::rebase(0x1_0151_b18c)");
        let destrutor = unico("crate::jogo::cstring::destroi_cstring_da_pilha(&mut s_path);");
        assert!(construtor < load_archives && load_archives < destrutor, "ordem: construtor, LoadArchives, destrutor");
        let entre = &corpo[construtor..destrutor];
        for saida in ["continue", "break", "return", "?;"] {
            assert!(!entre.contains(saida), "`{saida}` entre o construtor e o destrutor deixa a CString sem dono");
        }
    }
}
