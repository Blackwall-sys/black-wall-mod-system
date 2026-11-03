//! Diagnóstico: provas, sondas, medição, dumps e ferramentas de RE (natives Cxxx, eventos de teste, comandos de engenharia reversa). Não é produto.
//! Fica aqui o que o código de produto alcança nos dois builds; o resto mora em `so_dev.rs` e nos submódulos sob `devtools`, que o export
//! do GitHub deixa de fora.

#![allow(dead_code)]

/// Comandos de engenharia reversa do terminal. Só existem no dev, porque a única saída deles é o log de dev.
#[cfg(feature = "devtools")]
pub(crate) mod terminal_re;

#[cfg(feature = "devtools")]
pub(crate) mod vtabledump;

#[cfg(any(test, feature = "devtools"))]
pub(crate) mod censo;

#[cfg(feature = "devtools")]
pub(crate) mod camscan;

#[cfg(feature = "devtools")]
pub(crate) mod ai;

// O reexport deixa quem chama no caminho `crate::diagnostico::…` também para o que mora no `so_dev`.
#[cfg(feature = "devtools")]
mod so_dev;
#[cfg(feature = "devtools")]
pub(crate) use so_dev::*;

use crate::vm_do_jogo;
use crate::tipos_do_motor::classe_do_vptr;
use crate::arquivos_do_jogo::PATCHES_DE_RECURSO;
use crate::arquivos_do_jogo::A17_PROP_COMPONENTS;
use bwms_primitives::resource_path_hash;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::AtomicBool;
// Aqui só o `input_target_targets_allow`, que é de teste, usa os dois; o `so_dev` importa os dele.
#[cfg(test)]
use crate::entrada::{IACT_NONE, IK_NONE};
#[cfg(any(test, feature = "devtools"))]
use crate::tipos_do_motor::CALLBACKS;
use crate::tipos_do_motor::STATIC_CALLBACKS;
use crate::tipos_do_motor::ativar_hooks_do_evento;
use crate::mundo::*;
use crate::aparencia::*;
use crate::corpo_e_rosto::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "devtools")]
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
/// Bloco deste arquivo no `PROVA_NO_PUBLICO` (register.rs).
#[cfg(not(feature = "devtools"))]
const BLOCO_DE_PROVA: u8 = 0b010;

// Ficam fora do `terminal_re` (só dev) porque o tick de produto lê o hook nos dois builds. Quem instala o hook e grava o original é o `selftest`.
pub(crate) static ORIG_INK_PROCESS_INPUT: AtomicU64 = AtomicU64::new(0);

/// `true` com o hook do `ProcessInputEvents` instalado: a tecla vem da fila do motor e o caminho antigo do AppKit fica mudo.
pub(crate) fn rawinput_hook_ativo() -> bool {
    ORIG_INK_PROCESS_INPUT.load(Ordering::Relaxed) != 0
}

/// `BlackwallPing() -> Bool` — escreve `true` no retorno. Prova que o registro
/// entrou no RTTI e o executor chama nosso handler.
pub(crate) unsafe extern "C" fn tramp_ping(_ctx: *mut c_void, _frame: *mut c_void, out: *mut c_void, _rt: i64) {
    crate::log_dev!("[reg] >>> BlackwallPing chamado (handler nativo rodou!)");
    write_bool_ret(out, true);
}

/// Quantos eventos têm ouvinte agora, de instância ou estático — os que ficam ativados (`ActivateEvent` no 1º ouvinte,
/// `DeactivateEvent` quando sai o último). Lê a tabela que decide os hooks (`has_listener_for`), não um estado de instalação: conta eventos, não hooks.
#[cfg(any(test, feature = "devtools"))]
pub fn eventos_ativos() -> usize {
    let mut eventos: Vec<u64> = CALLBACKS.lock().map(|c| c.iter().map(|(ev, _, _, _)| *ev).collect()).unwrap_or_default();
    if let Ok(sc) = STATIC_CALLBACKS.lock() {
        eventos.extend(sc.iter().map(|(ev, _, _)| *ev));
    }
    eventos.sort_unstable();
    eventos.dedup();
    eventos.len()
}

/// Registra `BwmsScriptRefStrEcho(data: script_ref<String>) -> String`.
pub(crate) unsafe fn register_scriptref_proof(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["script_ref:String"]);
    let ok = if params.1 == 1 {
        registro_de_prova!(register_global_composed(reg, proto, params, "BwmsScriptRefStrEcho", "BwmsScriptRefStrEcho", corpo_de_prova!(tramp_scriptref_str_echo)))
    } else {
        crate::log_dev!("[reg] register_scriptref_proof: compose_params_from_types falhou (script_ref:String)");
        false
    };
    crate::log_dev!("[reg] register_all: BwmsScriptRefStrEcho={ok}");
}

pub(crate) unsafe fn register_ativacao_118_utils(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["String", "Int32"], "BwmsHookAtivacao", "BwmsHookAtivacao", corpo_de_prova!(tramp_hook_ativacao)));
    crate::log_dev!("[reg] register_all: HookAtivacao={a}");
}

pub(crate) unsafe fn register_variant_natives(reg: &Registry, proto: *mut c_void) {
    let sv = rtti::resolve_func(reg, "gameIBlackboard", "SetVariant");
    let ok = if let Some(ref sv) = sv {
        let params = compose_params(&[(sv.func, 1), (sv.func, 1)]);
        if params.1 == 2 {
            registro_de_prova!(register_global_composed(reg, proto, params, "BwmsVariantLog", "BwmsVariantLog", corpo_de_prova!(tramp_variant_log)))
        } else {
            crate::log_dev!("[reg] cw-variant-marshalling: compose_params Variant falhou");
            false
        }
    } else {
        crate::log_dev!("[reg] cw-variant-marshalling: gameIBlackboard::SetVariant não resolveu");
        false
    };
    crate::log_dev!("[reg] cw-variant-marshalling BwmsVariantLog={ok}");
}

pub(crate) unsafe fn register_c142_natives(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IMappin", "Int32"], "BwmsC142SetPhase", "BwmsC142SetPhase", corpo_de_prova!(tramp_c142_set_phase)));
    let b = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Uint64"], "BwmsC142Anotar", "BwmsC142Anotar", corpo_de_prova!(tramp_c142_anotar)));
    crate::log_dev!("[reg] register_all: c142 SetPhase={a} Anotar={b}");
}

pub(crate) unsafe fn register_c73b_natives(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Uint64"], "BwmsC73bAnotar", "BwmsC73bAnotar", corpo_de_prova!(tramp_c73b_anotar)));
    crate::log_dev!("[reg] register_all: c73b Anotar={a}");
}

pub(crate) unsafe fn register_c77_natives(reg: &Registry, proto: *mut c_void) {
    let valor = compose_params_from_types(reg, &["Int32", "Uint64"]);
    let ok_valor = valor.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, valor, "BwmsC77Anotar", "BwmsC77Anotar", corpo_de_prova!(tramp_c77_anotar)));
    let (ok_template, ok_entry) = match rtti::resolve_func(reg, "gameIBlackboard", "SetVariant") {
        Some(sv) => {
            let (template, entry) = (compose_params(&[(sv.func, 1)]), compose_params(&[(sv.func, 1)]));
            (
                template.1 == 1 && registro_de_prova!(register_global_composed(reg, proto, template, "BwmsC77VarianteTemplate", "BwmsC77VarianteTemplate", corpo_de_prova!(tramp_c77_variante_template))),
                entry.1 == 1 && registro_de_prova!(register_global_composed(reg, proto, entry, "BwmsC77VarianteEntry", "BwmsC77VarianteEntry", corpo_de_prova!(tramp_c77_variante_entry))),
            )
        }
        None => (false, false),
    };
    crate::log_dev!("[reg] register_all: c77 BwmsC77Anotar={ok_valor} BwmsC77VarianteTemplate={ok_template} BwmsC77VarianteEntry={ok_entry}");
    // 77P: varredura dos arrays do spawner e conferência dos ids pelos getters de produção.
    let p = [
        registro_de_prova!(register_argful_by_types(reg, proto, &["handle:CommunitySystem"], "BwmsC77pVarrer", "BwmsC77pVarrer", corpo_de_prova!(tramp_c77p_varrer))),
        registro_de_prova!(register_argful_by_types(reg, proto, &["Int32"], "BwmsC77pCandidatoId", "BwmsC77pCandidatoId", corpo_de_prova!(tramp_c77p_candidato_id))),
        registro_de_prova!(register_argful_by_types(reg, proto, &["Int32"], "BwmsC77pCandidatoEntry", "BwmsC77pCandidatoEntry", corpo_de_prova!(tramp_c77p_candidato_entry))),
        registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Int32", "Uint64"], "BwmsC77pElemento", "BwmsC77pElemento", corpo_de_prova!(tramp_c77p_elemento))),
    ];
    crate::log_dev!("[reg] register_all: c77p Varrer/CandidatoId/CandidatoEntry/Elemento={p:?}");
}

pub(crate) unsafe fn register_c118_natives(reg: &Registry, proto: *mut c_void) {
    let parte = compose_params_from_types(reg, &["Int32"]);
    let ok = parte.1 == 1 && registro_de_prova!(register_global_composed(reg, proto, parte, "BwmsC118Anotar", "BwmsC118Anotar", corpo_de_prova!(tramp_c118_anotar)));
    crate::log_dev!("[reg] register_all: c118 BwmsC118Anotar={ok}");
}

pub(crate) unsafe fn register_c122_natives(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(reg, proto, &["handle:IScriptable", "Uint64", "Uint64", "Bool"], "BwmsC122Anotar", "BwmsC122Anotar", corpo_de_prova!(tramp_c122_anotar)));
    crate::log_dev!("[reg] register_all: c122 BwmsC122Anotar={ok}");
}

pub(crate) unsafe fn register_c122sb_natives(reg: &Registry, proto: *mut c_void) {
    let ok = registro_de_prova!(register_argful_by_types(reg, proto, &["Int32", "Int32"], "BwmsC122SbAnotar", "BwmsC122SbAnotar", corpo_de_prova!(tramp_c122sb_anotar)));
    crate::log_dev!("[reg] register_all: c122sb BwmsC122SbAnotar={ok}");
}

pub(crate) unsafe fn register_c130b_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["Int32", "handle:Entity"]);
    let ok = params.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, params, "BwmsC130bAnotar", "BwmsC130bAnotar", corpo_de_prova!(tramp_c130b_anotar)));
    crate::log_dev!("[reg] register_all: c130b BwmsC130bAnotar={ok}");
}

pub(crate) unsafe fn register_c130d_natives(reg: &Registry, proto: *mut c_void) {
    let valor = compose_params_from_types(reg, &["Int32", "Uint64"]);
    let objeto = compose_params_from_types(reg, &["Int32", "handle:IScriptable"]);
    let ok_valor = valor.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, valor, "BwmsC130dAnotar", "BwmsC130dAnotar", corpo_de_prova!(tramp_c130d_anotar)));
    let ok_objeto = objeto.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, objeto, "BwmsC130dObjeto", "BwmsC130dObjeto", corpo_de_prova!(tramp_c130d_anotar)));
    crate::log_dev!("[reg] register_all: c130d BwmsC130dAnotar={ok_valor} BwmsC130dObjeto={ok_objeto}");
}

pub(crate) unsafe fn register_c134_natives(reg: &Registry, proto: *mut c_void) {
    let valor = compose_params_from_types(reg, &["Int32", "Uint64"]);
    let objeto = compose_params_from_types(reg, &["Int32", "handle:IScriptable"]);
    let parte = compose_params_from_types(reg, &["Int32"]);
    let ok_valor = valor.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, valor, "BwmsC134Anotar", "BwmsC134Anotar", corpo_de_prova!(tramp_c134_anotar)));
    let ok_objeto = objeto.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, objeto, "BwmsC134Objeto", "BwmsC134Objeto", corpo_de_prova!(tramp_c134_anotar)));
    let ok_pode = parte.1 == 1 && registro_de_prova!(register_global_composed(reg, proto, parte, "BwmsC134PodeArmar", "BwmsC134PodeArmar", corpo_de_prova!(tramp_c134_pode_armar)));
    crate::log_dev!("[reg] register_all: c134 BwmsC134Anotar={ok_valor} BwmsC134Objeto={ok_objeto} BwmsC134PodeArmar={ok_pode}");
}

pub(crate) unsafe fn register_c133_natives(reg: &Registry, proto: *mut c_void) {
    let valor = compose_params_from_types(reg, &["Int32", "Uint64"]);
    let id = compose_params_from_types(reg, &["Uint64"]);
    let ok_anotar = valor.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, valor, "BwmsC133Anotar", "BwmsC133Anotar", corpo_de_prova!(tramp_c133_anotar)));
    let ok_marcar = id.1 == 1 && registro_de_prova!(register_global_composed(reg, proto, id, "BwmsC133Marcar", "BwmsC133Marcar", corpo_de_prova!(tramp_c133_marcar)));
    crate::log_dev!("[reg] register_all: c133 BwmsC133Anotar={ok_anotar} BwmsC133Marcar={ok_marcar}");
}

pub(crate) unsafe fn register_c6_natives(reg: &Registry, proto: *mut c_void) {
    let p = compose_params_from_types(reg, &["Int32", "handle:IScriptable"]);
    let ok = p.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC6Anotar", "BwmsC6Anotar", corpo_de_prova!(tramp_c6_anotar)));
    crate::log_dev!("[reg] register_all: c6 BwmsC6Anotar={ok}");
}

/// Os 4 handlers só servem ao teste de troca de clima (`bwms-c81-troca-teste.reds`, fora do pacote): no público são o stub.
pub(crate) unsafe fn register_c81t_natives(reg: &Registry, proto: *mut c_void) {
    let passo = registro_de_prova!(register_global(reg, proto, "BwmsC81tPasso", "BwmsC81tPasso", corpo_de_prova!(tramp_c81t_passo)));
    let um = |nome: &str, tramp: NativeHandler| {
        let p = compose_params_from_types(reg, &["Int32"]);
        p.1 == 1 && register_global_composed(reg, proto, p, nome, nome, tramp)
    };
    let nome = registro_de_prova!(um("BwmsC81tNome", corpo_de_prova!(tramp_c81t_nome)));
    let prioridade = registro_de_prova!(um("BwmsC81tPrioridade", corpo_de_prova!(tramp_c81t_prioridade)));
    let p = compose_params_from_types(reg, &["Int32", "Uint64"]);
    let anotar = p.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC81tAnotar", "BwmsC81tAnotar", corpo_de_prova!(tramp_c81t_anotar)));
    crate::log_dev!("[reg] register_all: c81t Passo={passo} Nome={nome} Prioridade={prioridade} Anotar={anotar}");
}

pub(crate) unsafe fn register_c71_natives(reg: &Registry, proto: *mut c_void) {
    let p = compose_params_from_types(reg, &["handle:IScriptable", "Bool"]);
    let ok = p.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsC71Conferir", "BwmsC71Conferir", corpo_de_prova!(tramp_c71_conferir)));
    crate::log_dev!("[reg] register_all: c71 BwmsC71Conferir={ok}");
}

pub(crate) unsafe fn register_c130c_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["Int32", "handle:IScriptable"]);
    let ok = params.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, params, "BwmsC130cAnotar", "BwmsC130cAnotar", corpo_de_prova!(tramp_c130c_anotar)));
    crate::log_dev!("[reg] register_all: c130c BwmsC130cAnotar={ok}");
}

pub(crate) unsafe fn register_c130e_natives(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["Int32", "handle:IScriptable"]);
    let ok = params.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, params, "BwmsC130eAnotar", "BwmsC130eAnotar", corpo_de_prova!(tramp_c130e_anotar)));
    crate::log_dev!("[reg] register_all: c130e BwmsC130eAnotar={ok}");
}

/// A native de etapa do teste do `#11` (`bwms-a11-teste.reds`).
pub(crate) unsafe fn register_a11_natives(reg: &Registry, proto: *mut c_void) {
    let p = compose_params_from_types(reg, &["Int32", "Int32"]);
    let ok = p.1 == 2 && registro_de_prova!(register_global_composed(reg, proto, p, "BwmsA11Etapa", "BwmsA11Etapa", corpo_de_prova!(tramp_a11_etapa)));
    crate::log_dev!("[reg] register_all: a11 BwmsA11Etapa={ok}");
}

// CClassFunction da 2.5: base 0xF0 (dois `Set` em +0xB0/+0xC8 e redirecionamento em +0xE8, zerados = vazios),
// pai em +0xF0, regIndex em +0xF8. O global usa até +0xF8; um tamanho só serve aos dois.
pub(crate) const FUNC_POD_SIZE: usize = 0x100;

/// `BwmsPuppetStateArmRecovery(self: ref<IScriptable>) -> Bool` — 2026-08-02, iteração 2 do
/// estado do puppet. A saída IMEDIATA (`this.SwitchToScenario(Idle)` na mesma call-chain
/// síncrona de `OnEnterScenario`) já provou escapar o soft-lock sem crash, mas é rápida demais:
/// zero disparo real confirmado de `GenitalsController::OnAttach`/`HairstyleController::OnDetach`
/// (o puppet-setup assíncrono do `OpenMenu` nunca chega a rodar). Esta native captura o ponteiro
/// vivo da instância `MenuScenario_CharacterCustomizationMirror` (`self`, passado pelo wrap) e
/// AGENDA a recuperação pra alguns ticks depois (mesmo padrão `PENDING_HANDLE_EVENTS_AT`/
/// `HANDLE_EVENTS_DELAY_TICKS` já provado em `cp77_tick`) — dando uma janela real pro puppet
/// montar antes da saída forçada. O disparo em si (`SwitchToScenario(Idle)`) acontece do lado
/// Rust via `rtti::call_func` direto na instância capturada (`puppetstate_recovery_tick`,
/// chamada de `cp77_tick`) — evita por completo o problema de não ter `GameInstance` disponível
/// dentro do contexto do `MenuScenario` pra usar `DelaySystem` redscript.
#[cfg(feature = "devtools")]
pub(crate) static PUPPETSTATE_MIRROR_PTR: AtomicUsize = AtomicUsize::new(0);

#[cfg(feature = "devtools")]
pub(crate) static PUPPETSTATE_RECOVER_AT: AtomicU64 = AtomicU64::new(0); // tick-alvo; 0 = nada pendente

/// Chamada de `cp77_tick` todo tick — dispara `SwitchToScenario(MenuScenario_Idle)` direto via
/// `rtti::call_func` na instância capturada por `BwmsPuppetStateArmRecovery`, quando o tick-alvo é
/// atingido. Resolve o método por CLASSE do objeto vivo (`inkMenuScenario`, onde `SwitchToScenario`
/// é `native func` real — já confirmado chamável com sucesso via script nesta mesma sessão, ao
/// contrário do `QueueRequest` quebrado) — zero forge/offset novo, só reusa o mecanismo de despacho
/// já provado (`fire_event_args_by_hash` usa o mesmo `resolve_func`+`call_func` pra CallbackSystem).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn puppetstate_recovery_tick(reg: &Registry) {
    let pending = PUPPETSTATE_RECOVER_AT.load(Ordering::Relaxed);
    if pending == 0 || crate::ticks() < pending {
        return;
    }
    PUPPETSTATE_RECOVER_AT.store(0, Ordering::Relaxed);
    let scenario = PUPPETSTATE_MIRROR_PTR.load(Ordering::Relaxed) as *mut c_void;
    if !rtti::sane(scenario) {
        crate::log_dev!("[axl-puppet] recuperação atrasada: ponteiro do scenario inválido (coletado?), abortando");
        return;
    }
    match rtti::resolve_func(reg, "inkMenuScenario", "SwitchToScenario") {
        Some(rf) => {
            let idle = crate::cname::cname("MenuScenario_Idle");
            let r = rtti::call_func(&rf, scenario, &[rtti::Arg::CName(idle)]);
            crate::log_dev!(
                "[axl-puppet] recuperação ATRASADA disparada (tick={}) — SwitchToScenario(Idle) via call_func direto no scenario={scenario:p}, ok={}",
                crate::ticks(), r.is_some()
            );
        }
        None => crate::log_dev!("[axl-puppet] recuperação atrasada: resolve_func(inkMenuScenario::SwitchToScenario) falhou"),
    }
}

/// Garment/transmog (2026-08-03, /goal): 4ª arquitetura, evita os 2 bugs reais
/// já diagnosticados nesta madrugada (ver HISTORICO.md cont.17/19): (1) 2 `call_func` TOP-LEVEL
/// consecutivas retornando `ref<T>` crasham na 2ª (`equiprawv2`); (2) 1 native com 2 args
/// `handle:IScriptable` chamado ANINHADO de bytecode crasha no despacho (`equiprawonce`/`equipctxonce`).
/// Esta via usa só **1** `call_func` TOP-LEVEL (`Void`, não `ref<T>`) que internamente faz 2 chamadas
/// nativas ANINHADAS, cada uma com **1 SÓ** arg `handle:IScriptable` — mesmo shape de `BwmsCallMethod`
/// (acima), já provado seguro há sessões. As 2 nativas só ESCREVEM o ponteiro num atomic (não fazem
/// nada arriscado); Rust lê os atomics DEPOIS que o `call_func` único retorna, e faz o `transmute`
/// pro endereço cru (`0x102445e8c`) inteiramente do lado Rust — sem native aninhado nem 2ª `call_func`.
#[cfg(feature = "devtools")]
pub static CAPTURED_ES_PTR: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub static CAPTURED_REQ_PTR: AtomicU64 = AtomicU64::new(0);

/// Só de teste (decisão do dono D5, 2026-10-03): o despacho NÃO usa esta
/// função. A regra viva é `bwms_primitives::targets::entity_record_bate`, chamada pelo `alvo_casa`, e
/// ela BLOQUEIA quando falta o record da entidade; aqui a falta de dado deixa passar.
/// `criteria` = todos os valores `RecordID` (TweakDBID) de `EntityTarget`s associados ao
/// handler (via `AddTarget`) que foram construídos especificamente por `.RecordID(...)` (nunca
/// `.Type`/`.Template`/`.Appearance` — ver `EntityTargetCriterion`). `entity_record_id` = o
/// `RecordID` real da entidade do evento específico (`None` = indeterminado — entidade não é
/// Puppet/Vehicle, ou leitura de memória falhou).
#[cfg(test)]
pub(crate) fn entity_recordid_targets_allow(criteria: &[u64], entity_record_id: Option<u64>) -> bool {
    if criteria.is_empty() {
        // Nenhum EntityTarget(RecordID) associado — este filtro específico não se aplica (pode
        // haver OUTROS tipos de Target associados ao handler, fora de escopo desta rodada; não é
        // este filtro que decide por eles). Comportamento idêntico ao pré-existente.
        return true;
    }
    match entity_record_id {
        // Não sabemos o RecordID real da entidade do evento — FAIL-OPEN, nunca inventa um "não
        // bate" sem dado concreto (preserva o comportamento pré-existente pra qualquer entidade
        // fora de Puppet/Vehicle, ex. device — ver nota grande acima).
        None => true,
        // Semântica OR: `.RecordID(a).RecordID(b)` (2 AddTarget no mesmo handler) = "qualquer um".
        Some(rid) => criteria.iter().any(|c| *c == rid),
    }
}

/// Só de teste (decisão do dono D5): o despacho NÃO usa esta função. A regra viva é
/// `bwms_primitives::targets::component_bate`, chamada pelo `alvo_casa`, que BLOQUEIA quando o alvo
/// tem campo setado e o componente do evento não resolve (alvo todo curinga casa sem ler o
/// componente); aqui a falta de dado deixa passar.
/// O `Matches` trata `ID`/`Name` como 2 checks AND dentro da MESMA
/// instância — mas cada instância nossa (via `.ID(...)`/`.Name(...)`, 2 factories SEPARADAS) só
/// popula UM dos 2 campos por vez, e o dispatcher real (`CallbackSystemHandler::operator()`,
/// `std::ranges::find_if`) já usa semântica OR entre TODOS os targets da lista (confirmado lendo o
/// header real, não suposição) — logo "algum ID-target bate OU algum Name-target bate" reproduz
/// fielmente o comportamento real pra qualquer combinação de `.ID`/`.Name` no mesmo handler.
#[cfg(test)]
pub(crate) fn component_target_targets_allow(
    id_criteria: &[u64],
    name_criteria: &[u64],
    component: Option<(u64, u64)>,
) -> bool {
    if id_criteria.is_empty() && name_criteria.is_empty() {
        // Nenhum ComponentTarget associado — este filtro não se aplica.
        return true;
    }
    match component {
        // Componente indeterminado (ponteiro nulo/ilegível) — FAIL-OPEN, nunca inventa "não bate".
        None => true,
        Some((cid, cname)) => {
            id_criteria.iter().any(|c| *c == cid) || name_criteria.iter().any(|c| *c == cname)
        }
    }
}

/// `InputTarget::Matches` — só de teste (decisão do dono D5): o despacho NÃO usa
/// esta função. A regra viva é `bwms_primitives::targets::input_bate`, chamada pelo `alvo_casa` com os
/// critérios de `inputtarget_criteria`, e o `alvo_casa` BLOQUEIA quando o alvo tem campo setado e o evento não
/// tem tecla ou ação registrada (alvo `(IK_NONE, IACT_NONE)` casa sem ler o evento); aqui a falta de
/// dado deixa passar.
///
/// **Escopo AMPLIADO em 2026-08-21 — agora filtra por `key` E `action`, as 2 dimensões da fonte
/// real.** A versão de 2026-08-12 filtrava só por `key`, e a justificativa registrada era honesta
/// pro que se sabia então: o `action` de todo evento carregava um PLACEHOLDER fixo (`2`), e
/// filtrar contra um valor sempre-igual daria falso positivo/negativo silencioso. **O placeholder
/// era o bug** — corrigido em `make_keyinputevent_arg` (o caller agora informa `IACT_PRESS` na
/// descida e `IACT_RELEASE` na subida), então `action` passou a ser dado REAL e a razão de
/// excluí-lo deixou de existir.
///
/// As 2 guardas do `Matches`, cada uma com sentinela próprio:
/// `if (key != IK_None && key != event->key) return false;` e
/// `if (action != IACT_None && action != event->action) return false;`. Com o evento resolvido,
/// esta função decide igual; a diferença é só o evento sem dado.
///
/// Cada critério é o par `(key, action)` de UM `InputTarget`; alvos são OR entre si (um handler
/// com N targets dispara se QUALQUER um casar — mesma semântica de `CallbackSystemHandler::
/// operator()`, `std::ranges::find_if`, já confirmada por leitura da fonte no `#132`), mas as 2
/// dimensões DENTRO de um alvo são AND. Um alvo `.Axis(...)` entra sempre com `action=IACT_None`
/// (wildcard) — é o que o C++ faz, já que `Axis()` nunca escreve o campo `action`.
#[cfg(test)]
pub(crate) fn input_target_targets_allow(
    criteria: &[(i64, i64)],
    event_key: Option<i64>,
    event_action: Option<i64>,
) -> bool {
    if criteria.is_empty() {
        return true;
    }
    criteria.iter().any(|(key, action)| {
        // Dimensão indeterminada no evento = fail-open (nunca deveria acontecer pra "Input/Key",
        // que sempre grava em `KEYINPUT_STATES` no construtor — disciplina dos outros 3 filtros).
        let key_ok = *key == IK_NONE as i64 || event_key.is_none_or(|k| *key == k);
        let action_ok = *action == IACT_NONE as i64 || event_action.is_none_or(|a| *action == a);
        key_ok && action_ok
    })
}

/// `ResourceTarget::Matches` — só de teste (decisão do dono D5): o despacho NÃO
/// usa esta função. A regra viva, com `Path` e `Type`, está no `alvo_casa` e BLOQUEIA quando o
/// evento não tem caminho; aqui a falta de dado deixa passar.
#[cfg(test)]
pub(crate) fn resource_target_targets_allow(path_criteria: &[u64], event_path: Option<u64>) -> bool {
    if path_criteria.is_empty() {
        return true;
    }
    match event_path {
        None => true,
        Some(p) => path_criteria.iter().any(|c| *c == p),
    }
}

/// `EntityBuilderWrapper` (2026-08-18, madrugada
/// continuação 20 — item NUNCA tocado desde o catálogo original de 2026-08-07, 0/21 métodos
/// feitos): TODOS os 21 métodos
/// (12 em `EntityBuilderWrapper` + 5 em `EntityBuilderTemplateWrapper` + 4 em
/// `EntityBuilderAppearanceWrapper`) são leitura pura de campo sobre `Red::EntityBuilder`
/// (offsets confirmados: `appearance@0x20`/`appearances@0x80`/`entityTemplate@0x90`/`request@0xA0`/`components@0xC0`/
/// `visualTags@0xF8`/`flags@0x110`) — ZERO `Core::RawFunc`, exceto `ExtractComponentsJob`/
/// `ScheduleExtractComponentsJob` (agendamento assíncrono, fora do escopo desta rodada — nenhum
/// método exposto ao redscript precisa deles). Nenhuma das 3 classes é CMesh-trap: todas
/// `extends Red::IScriptable` DIRETO (mesmo ancestral já usado com segurança dezenas de vezes
/// neste projeto — `CallbackSystemHandler`/`StaticEntitySpec`/`TweakDBBatch`), nunca `ISerializable`.
///
/// **Escopo desta rodada, deliberadamente estreito**: só os 6 métodos de `EntityBuilderWrapper`
/// que dependem SÓ de `EntityBuilder`+`EntityBuilderRequest` (campos `WorldTransform`/`EntityID`/
/// `CName`/`TweakDBID`, todos escalares já com marshalling de retorno provado neste projeto) —
/// `GetRecordID`/`GetEntityID`/`GetAppearanceName`/`HasEntity`/`HasAppearance`/
/// `HasCustomAppearances`. FICAM DE FORA de propósito: `GetTemplatePath` (precisa dereferenciar
/// `entityTemplate->path`, um campo de `res::StreamedResource`/`CResource` — hierarquia de
/// RECURSO, mesma família que já causou crash de bind se DECLARADA em redscript sem backing —
/// aqui não declararíamos a classe, só leríamos o campo cru via Rust, mas o offset exato de
/// `path` dentro de `CResource` nunca foi confirmado neste projeto, então fica pra uma rodada
/// futura dedicada); `GetEntityType`/`GetTemplate`/`GetAppearance`/`GetCustomAppearances`
/// (precisam navegar `entityExtractor->results`/`appearances[]`, arrays de `Handle<IComponent>`/
/// `EntityBuilderAppearance`, capacidade real mas MUITO mais superfície de risco pra 1 rodada);
/// `AddComponent`s dos 2 wrappers-filho (mutação, nunca sem trigger real confirmado).
///
/// **Achado honesto que limita o valor prático desta rodada**: diferente de `#33`/`#11` (que têm
/// um caminho JÁ CONHECIDO pra obter uma instância viva — `FindComponentByType`/fixture — mesmo
/// que não-testado ainda), `EntityBuilderWrapper` NÃO TEM nenhum caminho conhecido pra obter um
/// `EntityBuilder*` REAL: o único disparo é `EntityBuilderEvent` (Callback, item `#8`), que o
/// motor só constrói de dentro de `Raw::EntityBuilder::ExtractComponentsJob`/
/// `ScheduleExtractComponentsJob` (`Core::RawFunc`, endereço Mac NUNCA resolvido, zero candidato
/// levantado nesta rodada — não investigado, fora de escopo). Por isso a fixture abaixo constrói
/// um `EntityBuilderWrapper` 100% SINTÉTICO (dados fabricados, side-table Rust — MESMO padrão já
/// usado por `inkWidgetSpawnEvent`/`AxisInputEvent`/etc.) — prova o MECANISMO (forge+métodos+
/// round-trip), não a captura de um builder real. Item avança de "0/21, nunca tocado" pra "6/21
/// codados via fixture (mecanismo provado offline), trigger real ainda sem endereço" — mantido
/// REAL_GAP (regra de ouro: RE offline nunca fecha item sozinha, e aqui nem há RE de endereço
/// nenhuma resolvida ainda pro trigger real).
#[cfg(feature = "devtools")]
pub(crate) static ENTITYBUILDERWRAPPER_STATES: Mutex<Vec<(usize, u64, u64, u64, bool, bool, bool)>> = Mutex::new(Vec::new());

/// Teste 130d: o que a leitura `parte` do `bwms-c130d-extract-teste.reds` tem que dar no builder `b` (travado por quem chama), lido direto do layout;
/// `None` pra parte sem valor no builder. Componentes contam só os objetos sãos, como os getters.
pub(crate) unsafe fn ebw_esperado(parte: usize, b: u64) -> Option<u64> {
    let req = eb_u64(b, EB_REQUEST);
    let aparencia = b + EB_APARENCIA;
    let vivos = |h: Vec<[u64; 2]>| h.into_iter().filter(|h| rtti::sane(h[0] as *mut c_void)).map(|h| h[0]).collect::<Vec<u64>>();
    let componentes_da_aparencia = || {
        let extrator = eb_u64(aparencia, EBA_EXTRATOR);
        if extrator == 0 { Vec::new() } else { vivos(eb_handles(extrator + EXT_RESULTADOS, 0)) }
    };
    let (raiz, componentes) = eb_raiz_e_componentes(b);
    let componentes = vivos(componentes);
    Some(match parte {
        0 => eb_u64(req, EBR_RECORD),
        1 => eb_u64(req, EBR_ENTITY_ID),
        2 | 11 => eb_u64(req, EBR_APARENCIA),
        3 => (eb_u64(b, EB_EXTRATOR) != 0) as u64,
        4 | 23 => (eb_u64(aparencia, EBA_RECURSO) != 0) as u64,
        5 => (eb_u64(b + EB_APARENCIAS, 0x08) >> 32 != 0) as u64,
        6 | 10 => eb_u64(eb_u64(b, EB_TEMPLATE), RES_CAMINHO),
        7 if raiz == 0 => 0,
        7 => rtti::type_name_hash(rtti::class_of(raiz as *mut c_void)),
        8 => (eb_u64(req, EBR_PARAMS) != 0) as u64,
        9 => (eb_u64(b + EB_APARENCIAS, 0x08) >> 32).min(256),
        12 | 16 => componentes.len() as u64,
        13 => eb_u64(eb_u64(aparencia, EBA_RECURSO), RES_CAMINHO),
        14 => eb_u64(eb_u64(aparencia, EBA_DEFINICAO), DEF_NOME),
        15 => componentes_da_aparencia().len() as u64,
        20 => raiz,
        21 => componentes.first().copied().unwrap_or(0),
        22 => componentes_da_aparencia().first().copied().unwrap_or(0),
        24 => componentes.last().copied().unwrap_or(0),
        _ => return None,
    })
}

/// Teste 130d, depois da original do job: (entidade do builder em `+0xB0`, componentes do builder em `+0xC0`).
pub(crate) unsafe fn ebw_depois_do_job(b: u64) -> (u64, Vec<u64>) {
    (eb_u64(b, 0xb0), eb_handles(b + 0xc0, 0).into_iter().map(|h| h[0]).collect())
}

/// Breadth Reflection: probe num objeto VIVO (player), 1x, gated `~/.bwms-reflection-test`. Roda do
/// `cp77_tick` (gameplay) — em `register_all` as classes de script ainda não têm props populadas
/// (provado: PlayerPuppet não resolvia ali). Pega `class_of(player)`, dumpa props (confirma o layout
/// do CProperty no macOS) + GET no objeto vivo + round-trip set/get em objeto fake. Ver
/// rtti::reflection_probe_cls.
#[cfg(feature = "devtools")]
pub(crate) static REFL_LIVE_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub unsafe fn reflection_live_once(player: *mut c_void) {
    let on = crate::marcador_de_dev(".bwms-reflection-test");
    if !on || player.is_null() {
        return;
    }
    if REFL_LIVE_DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let cls = rtti::class_of(player);
    crate::log(&rtti::reflection_probe_cls(cls, "player(class_of)", player));
}

/// Igual a [`res_loader_signature`], mas para o layout **Windows** documentado: `tokens` no offset
/// `0x00` da struct (`indexTable@+0x00`, `size@+0x08`, `capacity@+0x0C`, `nodes@+0x10`,
/// stride@`+0x1C`), com `failed`@0x30 e `tokenLock`@0x40 depois.
///
/// Existe porque a RE deixou DUAS hipóteses concorrentes e a varredura com só uma voltou vazia: a
/// assinatura `+0x20` veio da página de DEBUG `"RESOURCE LOADER"`, cujo objeto pode ser um wrapper
/// e não o loader. Testar as duas na mesma passada custa um `read_chunk` a mais e elimina a chance
/// de a busca falhar por ter escolhido a hipótese errada.
pub(crate) fn res_loader_signature_sdk(win: &[u8], min_size: u32) -> Option<(u32, u32, u32)> {
    if win.len() < 0x20 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([win[o], win[o + 1], win[o + 2], win[o + 3]]);
    let u64_at = |o: usize| u64::from_le_bytes(win[o..o + 8].try_into().unwrap());
    let index_table = u64_at(0x00);
    let size = u32_at(0x08);
    let capacity = u32_at(0x0c);
    let nodes = u64_at(0x10);
    let stride = u32_at(0x1c);
    if index_table < 0x1_0000_0000 || nodes < 0x1_0000_0000 {
        return None;
    }
    if capacity == 0 || size > capacity || capacity > (1 << 22) || size < min_size.max(1) {
        return None;
    }
    if stride < 8 || stride > 256 || stride % 4 != 0 {
        return None;
    }
    Some((size, capacity, stride))
}

/// Assinatura estrutural do objeto que a página de debug `"RESOURCE LOADER"` percorre
/// (`0x1029e8fec`, achado por string-xref em 2026-08-21 — 1 xref EXATO em 19,4M instruções).
///
/// A função trava um spinlock em `this+0x18` e anda um **HashMap do RED** em `this+0x20`:
/// `indexTable@+0x20`, `size@+0x28`, `capacity@+0x2c`, `nodes@+0x30`, stride-do-nó@`+0x3c`.
/// `ResourceLoader::FindToken` é LOOKUP nesse mapa — então achar o objeto vale mais
/// que resolver a função: vira leitura de dado.
///
/// Núcleo PURO (recebe a janela de 0x40 bytes já lida) porque scan de memória viva é onde um
/// critério frouxo custa caro: sem estes invariantes, qualquer lixo com um ponteiro plausível vira
/// "candidato" e o resultado é ruído em vez de sinal.
pub(crate) fn res_loader_signature(win: &[u8], min_size: u32) -> Option<(u32, u32, u32)> {
    if win.len() < 0x40 {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes([win[o], win[o + 1], win[o + 2], win[o + 3]]);
    let u64_at = |o: usize| u64::from_le_bytes(win[o..o + 8].try_into().unwrap());

    let index_table = u64_at(0x20);
    let size = u32_at(0x28);
    let capacity = u32_at(0x2c);
    let nodes = u64_at(0x30);
    let stride = u32_at(0x3c);

    // Ponteiros têm que existir e parecer endereço de verdade (heap/imagem do macOS arm64 fica
    // acima de 4GB; qualquer coisa menor é contador/flag, não ponteiro).
    if index_table < 0x1_0000_0000 || nodes < 0x1_0000_0000 {
        return None;
    }
    // Invariantes do HashMap: `size <= capacity`, capacidade viva e não-absurda.
    if capacity == 0 || size > capacity || capacity > (1 << 22) {
        return None;
    }
    // Um loader VIVO em gameplay tem MUITOS tokens (milhares de recursos carregados). O piso é
    // parametrizável porque a 1ª varredura (2026-08-21) mostrou que `size >= 1` deixa passar
    // dezenas de mapinhas de 3-40 entradas que obviamente não são o mapa de tokens.
    if size < min_size.max(1) {
        return None;
    }
    // Stride do nó: o mapa guarda `{next,hashedKey,key,value}` — dezenas de bytes, nunca 0 nem KB.
    if stride < 8 || stride > 256 || stride % 4 != 0 {
        return None;
    }
    Some((size, capacity, stride))
}

/// Offsets dos elementos de um `DynArray` — núcleo PURO da varredura de campo *wrapped*
/// (`array<inkWidgetBrush>`), separado justamente porque a aritmética é onde um erro vira
/// leitura fora dos limites em memória viva do jogo.
///
/// O `stride` NÃO é chutado: vem de `rtti::type_size(inner_ty)` (`IType::GetSize`, vtable+0x18 —
/// o MESMO getter que `new_object_from_class_named` já usa pra alocar). Era exatamente esse
/// "stride/ABI nunca confirmados" que mantinha o ramo `wrapped_match` do `res11walk` apenas
/// LOGANDO desde 2026-08-18.
pub(crate) fn wrapped_array_elem_offsets(cap: u32, size: u32, stride: usize, max: usize) -> Vec<usize> {
    // `size > cap` = header incoerente (lemos um objeto que não é DynArray, ou memória em
    // transição) — recusa em vez de confiar. Teto de stride evita que um `GetSize` absurdo
    // (tipo não resolvido) vire aritmética de ponteiro gigante.
    if stride == 0 || stride > 4096 || size > cap {
        return Vec::new();
    }
    (0..(size as usize).min(max)).map(|i| i * stride).collect()
}

/// Núcleo de `probe_resource_helper_type11` contra qualquer instância — a sintética
/// do `res11probe` ou a REAL do `res11rec` (record de TweakDB vivo, achado via `get_records_of_class`).
/// Parametrizado pelo `obj`/label do log, pra não duplicar a leitura de token.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn probe_resource_helper_type11_on(obj: *mut c_void, class: &str, prop: &str, tag: &str) {
    let cls_of_obj = rtti::class_of(obj);
    let resolved = rtti::resolve_prop_in_class(cls_of_obj, prop);
    let (voff, ty, in_holder) = match resolved {
        Some(t) => t,
        None => {
            crate::log_dev!("[{tag}] {class}.{prop} não resolveu na instância ({obj:p})");
            return;
        }
    };
    let kind = rtti::type_kind(ty);
    crate::log_dev!("[{tag}] {class}.{prop} @ {obj:p}: type_kind={kind:?} (esperado Some(11))");
    if kind != Some(11) {
        crate::log_dev!("[{tag}] categoria != 11, abortando (branch type_kind==11 não alcançável aqui)");
        return;
    }
    let vp = rtti::field_ptr(obj, voff, in_holder);
    if vp.is_null() || !crate::memoria::is_readable(vp as *const c_void, 0x10) {
        crate::log_dev!("[{tag}] field_ptr ilegível — parando (não é crash, é guard normal)");
        return;
    }
    let token_ptr = ((vp as *const u8).add(0x08) as *const u64).read_unaligned() as *const u8;
    if token_ptr.is_null() {
        crate::log_dev!("[{tag}] token@+0x08 == null -> loaded=false, resource=null (instância {obj:p}).");
        return;
    }
    if !crate::memoria::is_readable(token_ptr as *const c_void, 0x38) {
        crate::log_dev!("[{tag}] token não-nulo ({token_ptr:p}) mas ilegível — parando");
        return;
    }
    let finished = (token_ptr.add(0x58) as *const i32).read_unaligned();
    let error = *token_ptr.add(0x5C);
    let loaded = crate::arquivos_do_jogo::resource_token_is_loaded(finished, error);
    let inst = (token_ptr.add(0x28) as *const u64).read_unaligned() as *mut c_void;
    crate::log_dev!(
        "[{tag}] token não-nulo! finished={finished} error={error} loaded={loaded} resource_inst_sane={} >>> {} <<<",
        rtti::sane(inst),
        if loaded { "CAMINHO POSITIVO ALCANÇADO" } else { "ainda negativo/guard" }
    );
}

pub(crate) unsafe fn register_targets_utils(reg: &Registry, proto: *mut c_void) {
    let a = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Uint64", "Uint64", "Uint64"], "BwmsTargetInkWidgetMatches", "BwmsTargetInkWidgetMatches", corpo_de_prova!(tramp_target_inkwidget)));
    let b = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint64", "Uint64", "Uint64", "Uint64"], "BwmsTargetComponentMatches", "BwmsTargetComponentMatches", corpo_de_prova!(tramp_target_component)));
    let c = registro_de_prova!(register_argful_by_types(reg, proto, &["Uint32", "Uint32", "Uint32", "Uint32"], "BwmsTargetInputMatches", "BwmsTargetInputMatches", corpo_de_prova!(tramp_target_input)));
    crate::log_dev!("[reg] register_all: alvos InkWidget={a} Component={b} Input={c}");
}

/// Maior clonagem desde o boot: objetos clonados e handles que ficaram no original pelo teto.
pub(crate) static CLONE_MAIOR: Mutex<(usize, u32)> = Mutex::new((0, 0));

/// Corpos de job de script que o motor rodou e as threads onde rodaram (teste do `#204`).
#[cfg(feature = "devtools")]
pub(crate) static JOB_CORPOS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static JOB_THREADS: Mutex<Vec<std::thread::ThreadId>> = Mutex::new(Vec::new());

/// A instância em `obj` e a classe do objeto do Handle em `off` — sem `off`, o offset da tabela pro tipo da instância (fora dos 5 tipos: sem objeto). O Handle
/// só é lido se couber no tamanho da classe. `None` com instância inválida.
pub(crate) unsafe fn node_instance_da_memoria(obj: *mut c_void, off: Option<usize>) -> Option<NodeInstanceLida> {
    if !rtti::sane(obj) {
        return None;
    }
    let cls = rtti::class_of(obj);
    if !rtti::sane(cls) {
        return None;
    }
    let tamanho = rtti::type_size(cls) as usize;
    if tamanho < NODEINSTANCE_LEITURA_FIM || !crate::memoria::is_readable(obj as *const c_void, tamanho) {
        return None;
    }
    let tipo = nodeinstance_tipo(cls);
    let carga = off
        .or_else(|| tipo.map(|k| NODEINSTANCE_CARGAS[k].1))
        .filter(|o| o + 16 <= tamanho)
        .and_then(|o| handle_serializavel((obj as *const u8).add(o) as *const c_void))
        .map_or(std::ptr::null_mut(), |p| rtti::class_of(p));
    Some(NodeInstanceLida {
        obj,
        classe: cls,
        nome: rtti::type_name_hash(cls),
        estado: ((obj as *const u8).add(NODEINSTANCE_ESTADO_OFF) as *const u16).read_unaligned(),
        tipo,
        carga,
    })
}

/// Contagem da varredura do `#237`, somada entre setores.
#[derive(Default)]
#[cfg(feature = "devtools")]
pub(crate) struct C237Conta {
    /// Por tipo: (instâncias, carga certa, inicializadas, anexadas, visíveis, controle falso).
    pub(crate) por_tipo: [(u32, u32, u32, u32, u32, u32); 5],
    pub(crate) total: u32,
    pub(crate) outras: u32,
    pub(crate) incoerentes: u32,
    pub(crate) com_id: u32,
    /// Setups com id e sem instância, pelo motivo de `node_instance_motivo`.
    pub(crate) sem: [u32; 4],
    pub(crate) varridos: u32,
    pub(crate) setups: u32,
    /// Setups pulados em setores sem instância nos primeiros ids.
    pub(crate) pulados: u32,
    /// Por classe da instância (hash do nome).
    pub(crate) classes: std::collections::HashMap<u64, InstanciasDaClasse>,
    /// Instâncias já contadas: a mesma pode vir pelo id e pelo registro vizinho.
    pub(crate) vistas: std::collections::HashSet<usize>,
    /// Registros vizinhos válidos percorridos, instâncias contadas por eles antes do id, e instância achada pelo id cujo registro não confere.
    pub(crate) registros: u32,
    pub(crate) pelo_registro: u32,
    pub(crate) registro_invalido: u32,
    /// Setores cujos registros vizinhos já foram percorridos.
    pub(crate) setores_com_registros: u32,
}

/// A instância achada pelo id do setup; devolve o ponteiro dela (nulo sem instância) pro passeio pelos registros vizinhos.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn c237_contar(bases: &[*mut c_void], id: u64, conta: &mut C237Conta) -> *mut c_void {
    conta.com_id += 1;
    let Some(l) = node_instance_objeto(id, None) else {
        conta.sem[node_instance_motivo(id)] += 1;
        return std::ptr::null_mut();
    };
    let obj = l.obj;
    c237_contar_lida(bases, l, conta);
    obj
}

/// Conta uma instância lida, uma vez por instância, no tipo do header mais próximo na hierarquia da classe dela.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn c237_contar_lida(bases: &[*mut c_void], l: NodeInstanceLida, conta: &mut C237Conta) {
    if !conta.vistas.insert(l.obj as usize) {
        return;
    }
    conta.total += 1;
    let amostra = conta.classes.entry(l.nome).or_default();
    amostra.n += 1;
    amostra.classe = l.classe as usize;
    amostra.tipo = l.tipo;
    if amostra.n <= AMOSTRA_POR_CLASSE {
        if let Some(achados) = node_instance_sondar(l.obj, [bases[0], bases[2], bases[4]]) {
            for (i, por_base) in achados.iter().enumerate() {
                for j in 0..3 {
                    amostra.hits[i][j] += por_base[j] as u32;
                }
            }
        }
    }
    let Some(k) = l.tipo else {
        conta.outras += 1;
        return;
    };
    let t = &mut conta.por_tipo[k];
    t.0 += 1;
    if rtti::sane(l.carga) && class_is_kind_of(l.carga, bases[k]) {
        t.1 += 1;
    }
    let (inicializada, anexada, visivel) = (l.estado & 0x2 != 0, l.estado & 0x20 != 0, l.estado & 0x200 != 0);
    t.2 += inicializada as u32;
    t.3 += anexada as u32;
    t.4 += visivel as u32;
    conta.incoerentes += (anexada && !inicializada) as u32;
    let controle = if NODEINSTANCE_CARGAS[k].1 == 0x98 { 0xA0 } else { 0x98 };
    if let Some(c) = node_instance_da_memoria(l.obj, Some(controle)) {
        if rtti::sane(c.carga) && class_is_kind_of(c.carga, bases[k]) {
            conta.por_tipo[k].5 += 1;
        }
    }
}

#[cfg(feature = "devtools")]
pub(crate) fn c237_resumo(conta: &C237Conta) -> (String, bool) {
    let por_tipo = &conta.por_tipo;
    let presentes: Vec<usize> = (0..5).filter(|k| por_tipo[*k].0 > 0).collect();
    let ok = presentes.len() >= 2
        && conta.incoerentes == 0
        && presentes.iter().all(|k| {
            let (n, certa, _, anexadas, _, falso) = por_tipo[*k];
            certa >= 1 && certa * 10 >= anexadas.min(n) * 9 && falso == 0
        });
    // Offset com mais Handles da base esperada, somado nas classes de cada tipo.
    let melhor = |k: usize| -> String {
        let j = BASES_SONDADAS.iter().position(|b| *b == NODEINSTANCE_CARGAS[k].2).unwrap_or(0);
        let mut soma = [0u32; SLOTS_SONDADOS];
        let mut amostradas = 0;
        for c in conta.classes.values().filter(|c| c.tipo == Some(k)) {
            amostradas += c.n.min(AMOSTRA_POR_CLASSE);
            for (i, h) in c.hits.iter().enumerate() {
                soma[i] += h[j];
            }
        }
        match soma.iter().enumerate().max_by_key(|(_, h)| **h) {
            Some((i, h)) if *h > 0 => format!("{:#x} {h}/{amostradas}", 0x90 + i * 8),
            _ => format!("nenhum em {amostradas}"),
        }
    };
    let linhas: Vec<String> = presentes
        .iter()
        .map(|k| {
            let (n, certa, ini, anexadas, vis, falso) = por_tipo[*k];
            format!(
                "{} {n} (objeto {} em {:#x}: {certa}, melhor offset {}, inicializadas {ini}, anexadas {anexadas}, visíveis {vis}, controle falso {falso})",
                NODEINSTANCE_CARGAS[*k].0,
                NODEINSTANCE_CARGAS[*k].2,
                NODEINSTANCE_CARGAS[*k].1,
                melhor(*k)
            )
        })
        .collect();
    let mut classes: Vec<(&u64, &InstanciasDaClasse)> = conta.classes.iter().collect();
    classes.sort_by(|a, b| b.1.n.cmp(&a.1.n));
    let resumo_classes: Vec<String> = classes
        .iter()
        .enumerate()
        .filter(|(i, (_, c))| *i < 12 || c.tipo.is_some())
        .map(|(_, (nome, c))| {
            let amostradas = c.n.min(AMOSTRA_POR_CLASSE);
            let achados: Vec<String> = BASES_SONDADAS
                .iter()
                .enumerate()
                .filter_map(|(j, base)| {
                    let (i, h) = c.hits.iter().enumerate().map(|(i, v)| (i, v[j])).max_by_key(|(_, h)| *h)?;
                    (h > 0).then(|| format!("{base}@{:#x} {h}/{amostradas}", 0x90 + i * 8))
                })
                .collect();
            let tipo = c.tipo.map_or("-", |k| NODEINSTANCE_CARGAS[k].0);
            // SAFETY: a classe veio do `class_of` de uma instância viva; os pais só são lidos com `is_readable`.
            let pais = unsafe { pais_da_classe(c.classe) };
            format!("{} {} <{pais}> tipo {tipo} [{}]", crate::cname::resolve_cname(**nome), c.n, achados.join(", "))
        })
        .collect();
    (
        format!(
            "{} instância(s) em {} setor(es) e {} setups, {} pulados em setor sem instância nos primeiros ids ({} com id; sem instância: registro {}, id \
             sem instância {}, inválida {}, tamanho {}), registros vizinhos {} em {} setor(es) ({} instância(s) contadas por eles, {} registro(s) que não \
             conferem), {} de outros tipos, {} anexada(s) sem inicializada; {}; classes: {}",
            conta.total,
            conta.varridos,
            conta.setups,
            conta.pulados,
            conta.com_id,
            conta.sem[0],
            conta.sem[1],
            conta.sem[2],
            conta.sem[3],
            conta.registros,
            conta.setores_com_registros,
            conta.pelo_registro,
            conta.registro_invalido,
            conta.outras,
            conta.incoerentes,
            linhas.join("; "),
            resumo_classes.join(" · ")
        ),
        ok,
    )
}

/// Arquivo de hashes de comunidade no HOME de quem está jogando; o alternativo é `npcs::COMMUNITY_HASHES_ARQUIVO_ALT`.
#[cfg(feature = "devtools")]
pub(crate) const COMMUNITY_HASHES_ARQUIVO: &str = ".bwms-community-hashes.txt";

/// Records pré-resolvidos no `Initialize` (puppet, veículo).
#[cfg(feature = "devtools")]
pub(crate) static RECORDS_PRE_RESOLVIDOS: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

/// O `Session/Ready` do spawn do objeto principal do jogador, anotado pelo hook `selftest::player_main_object_spawned_hook` (a thread do
/// spawner não é conhecida) e despachado aqui, na thread do jogo. `restored` é o que o hook anotou (a carga de save marcada pelo `OnGameLoad`);
/// `pregame` vem do script `BwmsSessaoEstado`, o `IsPreGame` do handler de pedidos do sistema (o que a referência lê no `OnWorldAttached`). Como a
/// referência, sessão carregada de save não despacha aqui (o dela sai do `OnGameRestored`). Cada despacho fica em `C129_DESPACHOS` como
/// `(pregame, restored, callbacks)`.
pub(crate) static C129_DESPACHOS: Mutex<Vec<(bool, bool, usize)>> = Mutex::new(Vec::new());

/// Teste: callback estático `classe.funcao` pro `evento`, registrado pelo Rust (antes de haver script rodando no mundo).
pub(crate) fn registrar_callback_estatico(evento: &str, classe: &str, funcao: &str) -> bool {
    let eh = cname(evento);
    let novo = STATIC_CALLBACKS.lock().is_ok_and(|mut sc| {
        let dup = sc.iter().any(|(e, c, f)| *e == eh && c == classe && f == funcao);
        if !dup {
            sc.push((eh, classe.to_string(), funcao.to_string()));
        }
        !dup
    });
    if novo {
        ativar_hooks_do_evento(eh);
    }
    novo
}

/// Esquece os nomes de componente ligados às partes da entidade.
pub(crate) fn es20_esquecer_partes(entidade: u64) {
    if let Ok(mut m) = es20_componentes_da_parte().lock() {
        m.retain(|(e, _), _| *e != entidade);
    }
}

/// Esquece os originais de máscara, aparência e caminho de um componente da entidade.
pub(crate) fn es20_esquecer_originais(entidade: u64, id: u64) {
    if let Ok(mut m) = es20_originais().lock() {
        m.retain(|(e, i, _), _| !(*e == entidade && *i == id));
    }
}

/// Parte do nome base de volta ao valor anterior (`None` tira a ligação): o teste desfaz o `LinkComponentToPart` dele.
pub(crate) fn es20_restaurar_parte(entidade: u64, base: u64, anterior: Option<u64>) {
    let Ok(mut g) = ES22_COMPONENT_APPEARANCE.lock() else { return };
    match anterior {
        Some(parte) => {
            if let Some((_, p)) = g.iter_mut().find(|(k, _)| *k == (entidade, base)) {
                *p = parte;
            }
        }
        None => g.retain(|(k, _)| *k != (entidade, base)),
    }
}

/// Tira o estado de recurso da parte (teste).
pub(crate) fn es20_desligar_parte(entidade: u64, parte: u64) {
    if let Ok(mut g) = ES22_RESOURCE_STATE.lock() {
        g.retain(|(k, _)| *k != (entidade, parte));
    }
}

/// O estado de aparência que o smoke de `BwmsAparencia` (o do component wrapper) muda no componente do V: o `ComponentState` do nome base,
/// a parte ligada a ele e o `ResourceState` da parte de teste. `BwmsAparencia` grava na loja que o pipeline do #20 lê; sem a volta, o override do smoke
/// seria aplicado em todo `ReassembleAppearance` do V.
struct Es22EstadoGuardado {
    entidade: u64,
    base: u64,
    estado: Option<bwms_primitives::override_state::ComponentState>,
    parte: Option<u64>,
    parte_de_teste: u64,
    recurso: Option<DynamicAppearanceName>,
}

static ES22_ESTADO_GUARDADO: Mutex<Option<Es22EstadoGuardado>> = Mutex::new(None);

/// Guarda o estado do nome base e o `ResourceState` da parte de teste antes do setup do smoke; uma guarda nova troca a anterior.
pub(crate) fn es22_guardar_estado(entidade: u64, base: u64, parte_de_teste: u64) -> bool {
    let recurso = ES22_RESOURCE_STATE.lock().ok().and_then(|g| g.iter().find(|(k, _)| *k == (entidade, parte_de_teste)).map(|(_, s)| s.clone()));
    let guardado = Es22EstadoGuardado {
        entidade,
        base,
        estado: es20_estado(entidade, base),
        parte: es22_parte_do_componente(entidade, base),
        parte_de_teste,
        recurso,
    };
    let Ok(mut g) = ES22_ESTADO_GUARDADO.lock() else { return false };
    *g = Some(guardado);
    true
}

/// Volta o que `es22_guardar_estado` guardou: sem estado ou sem parte antes, a entrada sai. `false` sem guarda.
pub(crate) fn es22_restaurar_estado() -> bool {
    let Some(g) = ES22_ESTADO_GUARDADO.lock().ok().and_then(|mut g| g.take()) else { return false };
    if let Ok(mut m) = es20_componentes().lock() {
        match g.estado {
            Some(estado) => {
                m.insert((g.entidade, g.base), estado);
            }
            None => {
                m.remove(&(g.entidade, g.base));
            }
        }
    }
    es20_restaurar_parte(g.entidade, g.base, g.parte);
    match &g.recurso {
        Some(seletor) => es22_ligar_parte_seletor(g.entidade, g.parte_de_teste, seletor),
        None => es20_desligar_parte(g.entidade, g.parte_de_teste),
    }
    true
}

/// O estado que o smoke das tags visuais (override de tag com vários mods) muda na entidade: o `ComponentState` de cada nome das 14
/// tags. A remoção deixa o estado com `ChangesChunkMask` (a original volta a cada aplicação, como na referência); sem a volta, todo
/// `ReassembleAppearance` do V no pipeline do #20 gravaria a máscara desses componentes fora do teste.
struct Es20TagsGuardadas {
    entidade: u64,
    /// `(nome, estado na loja)`; `None` = o nome não tinha estado.
    estados: Vec<(u64, Option<bwms_primitives::override_state::ComponentState>)>,
}

static ES20_TAGS_GUARDADAS: Mutex<Option<Es20TagsGuardadas>> = Mutex::new(None);

/// Guarda o estado de todos os nomes da entidade na loja; uma guarda nova troca a anterior.
pub(crate) fn es20_guardar_tags(entidade: u64) -> bool {
    let Ok(m) = es20_componentes().lock() else { return false };
    let estados = m.iter().filter(|((e, _), _)| *e == entidade).map(|((_, nome), s)| (*nome, Some(s.clone()))).collect();
    drop(m);
    let Ok(mut g) = ES20_TAGS_GUARDADAS.lock() else { return false };
    *g = Some(Es20TagsGuardadas { entidade, estados });
    true
}

/// Volta o que `es20_guardar_tags` guardou: a entidade fica na loja só com os nomes e estados da guarda. `false` sem guarda.
pub(crate) fn es20_restaurar_tags() -> bool {
    let Some(g) = ES20_TAGS_GUARDADAS.lock().ok().and_then(|mut g| g.take()) else { return false };
    let Ok(mut m) = es20_componentes().lock() else { return false };
    m.retain(|(e, _), _| *e != g.entidade);
    for (nome, estado) in g.estados {
        match estado {
            Some(estado) => {
                m.insert((g.entidade, nome), estado);
            }
            None => {
                m.remove(&(g.entidade, nome));
            }
        }
    }
    true
}

/// Põe (`true`) ou tira um alvo do escopo, pro teste em jogo.
pub(crate) fn es20_escopo_marcar(escopo: u64, alvo: u64, dentro: bool) {
    if let Ok(mut g) = es20_escopos().write() {
        if dentro {
            g.entry(escopo).or_default().insert(alvo);
        } else if let Some(s) = g.get_mut(&escopo) {
            s.remove(&alvo);
        }
    }
}

/// Tira as ligações de teste da entidade (nomes base e partes dados).
pub(crate) fn es22_esquecer(entidade: u64, nomes_base: &[u64], partes: &[u64]) {
    if let Ok(mut g) = ES22_COMPONENT_APPEARANCE.lock() {
        g.retain(|((e, n), _)| !(*e == entidade && nomes_base.contains(n)));
    }
    if let Ok(mut g) = ES22_RESOURCE_STATE.lock() {
        g.retain(|((e, p), _)| !(*e == entidade && partes.contains(p)));
    }
}

pub(crate) const FACT_REMOVER_VM: u64 = 0x1_027f_2508;

/// Saídas e pilha da última execução pela native de produção, pro teste ler sem repetir a chamada.
pub(crate) static QUESTS_ULTIMA: std::sync::Mutex<Option<(Vec<u64>, u32, u32)>> = std::sync::Mutex::new(None);

/// Resultado da última execução de `ToggleNode`/`ToggleVariant` (o do `ExecuteNode`, ou o erro negativo), pro teste.
#[cfg(feature = "devtools")]
pub(crate) static WS_ULTIMO: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(i64::MIN);

/// Self-test do `register_method` com realloc (gated `~/.bwms-regmethod-test`). DEV.
/// T1 = lógica de realloc ISOLADA (DynArray nosso, ZERO classe do jogo): array cheio (cap==size)
/// + 1 push → deve realocar, PRESERVAR os entries antigos e ANEXAR o novo. T2 = `register_method`
/// numa CClass real (gameGodModeSystem) — caminho integrado, 1 método (poluição mínima, some no
/// reboot). Prova o gap "V1 não realoca" fechado, com integridade verificada por igualdade de ponteiro.
pub(crate) unsafe fn regmethod_selftest(reg: &Registry) {
    let on = crate::marcador_de_dev(".bwms-regmethod-test");
    if !on {
        return;
    }
    // --- T1: realloc isolado (memória nossa) ---
    let buf0 = rtti::pool_alloc(2 * 8, 8) as *mut u64;
    if buf0.is_null() {
        crate::log_dev!("[regmethod-test] T1: pool_alloc null — abortando");
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
    crate::log_dev!(
        "[regmethod-test] T1 realloc-isolado: cap 2->{ncap} size {nsize} slot={slot:?} preservou=[{:#x},{:#x}] append={:#x} OK={ok1}",
        ne.read_unaligned(),
        ne.add(1).read_unaligned(),
        ne.add(2).read_unaligned()
    );
    // --- T2: register_method numa classe REAL (caminho integrado) ---
    let class = "gameGodModeSystem";
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => {
            crate::log_dev!("[regmethod-test] T2: sem protótipo (AddGodMode não resolveu) — só T1");
            return;
        }
    };
    let cls = reg.class_by_name(class);
    if !rtti::sane(cls) {
        crate::log_dev!("[regmethod-test] T2: classe não resolveu — só T1");
        return;
    }
    let arr = (cls as *mut u8).add(0x48) as *mut c_void;
    let size_before = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    let ok2 = register_method(
        reg, class, proto, "gameGodModeSystem::BwmsRegTest", "BwmsRegTest", tramp_ping, false,
    );
    let size_after = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    crate::log_dev!(
        "[regmethod-test] T2 register_method real: {class}.BwmsRegTest ok={ok2} size {size_before}->{size_after} (esperado +1)"
    );
}

/// Todas as entradas `(chave, valor)` do mapa, pelos buckets e cadeias (no máximo `size`).
#[cfg(any(test, feature = "devtools"))]
pub(crate) unsafe fn c82_mapa_entradas(mapa: *const u8) -> Vec<(u64, u64)> {
    let mut v = Vec::new();
    let Some((indices, nos, tamanho, capacidade, passo)) = hashmap_cabecalho(mapa) else { return v };
    for b in 0..capacidade.min(1 << 20) as usize {
        let mut i = indices.add(b).read_unaligned();
        while i != u32::MAX && v.len() < tamanho as usize {
            let no = nos.add(i as usize * passo);
            v.push(((no.add(8) as *const u64).read_unaligned(), (no.add(0x10) as *const u64).read_unaligned()));
            i = (no as *const u32).read_unaligned();
        }
    }
    v
}

/// Teste: com `true`, a mescla usa só as fontes montadas em memória (o 47c não mistura a fonte da tabela do 47d).
pub(crate) static CUST_SO_MONTADAS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Marcas dos hooks dos `Initialize*Option`: nome de mod posto nas opções do estado, opção de mod ligada.
#[cfg(feature = "devtools")]
pub(crate) static CUST_MARCAS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_INITIALIZE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_REQUESTCOMPONENTS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_REASSEMBLE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_ATTACH: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_DETACH: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_UNINITIALIZE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static ORIG_C130_DISPOSE: AtomicU64 = AtomicU64::new(0);

// Teste 130b (`~/.bwms-hook-c130b-teste`): despacho dos eventos `Entity/*` pelos hooks do ciclo de vida da entidade (endereços do 130a). Como a referência
// registra, `Entity/Initialize` e `Entity/Reassemble` saem do `RequestComponents` (array da própria entidade = inicialização, outro array = troca de
// aparência), `Entity/Attach` antes e `Entity/AfterAttach` depois do `Attach`, `Entity/Detach` antes do `Detach`, e `Entity/Uninitialize` antes do
// `Uninitialize` e do `Dispose` que desinicializa em linha (status de 2 a 5, sem cena). Os hooks só leem a máscara atômica de eventos com ouvinte e
// despacham na thread em que o motor chama (83–87% fora da thread do jogo no boot 164). O handler de script devolve a entidade pelo `BwmsC130bAnotar`, e o
// Rust confere na mesma thread se é a do despacho e se o status é o do momento do evento.
/// Antes do veredito o despacho para, pra nenhum evento ficar no meio da contagem.
pub(crate) static C130B_CONGELADO: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// Entidade do despacho em curso nesta thread (0 fora de despacho).
    pub(crate) static C130B_ATUAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Anotações feitas nesta thread durante o despacho em curso.
    pub(crate) static C130B_ANOTOU: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Componentes que ouvintes do `Entity/Reassemble` somaram na entidade e o hook copiou pro array novo da troca de aparência.
#[cfg(feature = "devtools")]
pub(crate) static C130E_PROPAGADOS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static C130E_NO_ARRAY_NOVO: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// Componente que o handler somou no despacho do `Entity/Reassemble` em curso nesta thread.
    pub(crate) static C130E_SOMADO: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `componentes` (`DynArray<Handle<IComponent>>`) tem a instância `inst`.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn c130e_array_tem(componentes: *mut c_void, inst: u64) -> bool {
    if !crate::memoria::is_readable(componentes as *const c_void, 0x10) {
        return false;
    }
    let entradas = (componentes as *const u64).read_unaligned() as *const u64;
    let n = ((componentes as *const u8).add(0x0c) as *const u32).read_unaligned() as usize;
    !entradas.is_null() && n < 100_000 && crate::memoria::is_readable(entradas as *const c_void, n * 16) && (0..n).any(|i| entradas.add(i * 2).read_unaligned() == inst)
}

/// Despachos de `Component/Toggle` com ouvinte, por hook: 0 o handler de script do `Toggle`, 1 a função C++.
pub(crate) static C134E_TOGGLE_POR_HOOK: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Cargas de save vistas pelo hook, pro teste.
pub(crate) static C129_CARGAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static C129_CHAMADAS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

pub(crate) static C130C_CONGELADO: AtomicBool = AtomicBool::new(false);

pub(crate) static C130C_HOOK_OK: AtomicBool = AtomicBool::new(false);

thread_local! {
    pub(crate) static C130C_ATUAL: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(crate) static C130C_ANOTOU: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub static UPDATE_REGISTRAR_CAPTURED_PTR: AtomicU64 = AtomicU64::new(0);

/// O teste 134 para os despachos destes 2 eventos antes de contar.
pub(crate) static C134E_CONGELADO: AtomicBool = AtomicBool::new(false);

pub(crate) static ANIM_FRAMEBEGIN_HOOK_CNT: AtomicU64 = AtomicU64::new(0);

pub(crate) static ESTAGIO_VEZES: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// Quantas vezes o estágio veio logo depois do anterior (FrameBegin → PreBuckets → PreRenderUpdate) no mesmo frame.
pub(crate) static ESTAGIO_EM_ORDEM: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

pub(crate) static ESTAGIO_ULTIMO: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(u8::MAX);

/// Marca a passagem pelo estágio `ordem` (0, 1, 2) e conta se o anterior foi o de antes no frame.
pub(crate) fn marcar_estagio(ordem: u8) {
    ESTAGIO_VEZES[ordem as usize].fetch_add(1, Ordering::Relaxed);
    let anterior = ESTAGIO_ULTIMO.swap(ordem, Ordering::Relaxed);
    if ordem > 0 && anterior == ordem - 1 {
        ESTAGIO_EM_ORDEM[ordem as usize - 1].fetch_add(1, Ordering::Relaxed);
    }
}

pub(crate) static C175_ATIVO: AtomicBool = AtomicBool::new(false);

/// Árvores anexadas depois do `Reparent` e do `AddChildWidget` de script.
pub(crate) static C175_ANEXADAS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// `OnInitialize` que o motor não chamou por ser de controller legado.
pub(crate) static C175_LEGADOS_PULADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static RELOCREAL_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) static ATTACHDETACH_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) static DERIVETEST_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) static COPYTEST_DONE: AtomicBool = AtomicBool::new(false);

pub(crate) static COPYTEST_ARMED: AtomicBool = AtomicBool::new(false);

pub(crate) static EQUIP_DEFERRED_DONE: AtomicBool = AtomicBool::new(false);


/// Relocador universal (in-game) — prova o relocador arm64 num prólogo REAL do jogo com o
/// caso DIFÍCIL (BL, PC-relativo). Alvo: `game::MuppetStateMachines<MuppetLogicStateMachineState>
/// ::~dtor` @vm 0x104697a94 — prólogo [stp x29,x30,[sp,#-0x10]! | mov x29,sp | BL <helper> | ldp
/// x29,x30]. NO MENU esse dtor de IA de NPC está dormente (mundo não carregado) → hookar+dumpar+
/// reverter é colisão ~zero; por isso o gatilho por marcador roda ANTES do gate de player (menu).
/// Guard nos 16 bytes exatos do prólogo: se o build divergir, aborta sem tocar em nada.
/// PROVA INDEPENDENTE: (A) decodifica o alvo absoluto do BL pelo offset relativo do próprio insn;
/// (B) decodifica o movz/movk x17 que o trampolim materializou; A==B ⇒ o relocador traduziu
/// PC-relativo→absoluto correto. Depois reverte e confere os 16 bytes originais byte-exato.
///
/// # Safety
/// Só toca o alvo se os 16 bytes do prólogo baterem com a assinatura; hook+dump+revert é síncrono
/// (sem yield) e o revert é imediato — nenhuma thread chega a chamar a função hookada.
pub(crate) unsafe fn prove_relocreal() {
    // dummy nunca é chamado (revertemos antes de qualquer thread poder invocar o alvo).
    unsafe extern "C" fn reloc_dummy() {}
    const VM: u64 = 0x1_0469_7a94;
    const SIG: [u32; 4] = [0xa9bf_7bfd, 0x9100_03fd, 0x9430_3f5b, 0xa8c1_7bfd];
    let target = crate::rebase(VM);
    if !crate::memoria::is_readable(target as *const c_void, 16) {
        crate::log_dev!("[relocreal] alvo ilegível (slide/patch?) — abortado sem tocar");
        return;
    }
    let mut orig = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, orig.as_mut_ptr(), 16);
    let got: [u32; 4] = core::array::from_fn(|i| {
        u32::from_le_bytes([orig[i * 4], orig[i * 4 + 1], orig[i * 4 + 2], orig[i * 4 + 3]])
    });
    if got != SIG {
        crate::log_dev!(
            "[relocreal] prólogo divergiu do esperado (build diferente?) got={got:08x?} esperado={SIG:08x?} — abortado"
        );
        return;
    }
    // (A) alvo absoluto do BL (insn[2]), calculado do endereço RUNTIME do próprio BL.
    let t = target as u64;
    let bl = got[2];
    let mut off = (bl & 0x03FF_FFFF) as i64;
    if off & (1 << 25) != 0 {
        off |= !0x03FF_FFFFi64;
    }
    let bl_target_a = ((t + 8) as i64).wrapping_add(off << 2) as u64;

    // hook: relocate_prologue roda no prólogo REAL; devolve o ponteiro do trampolim.
    let tramp = match crate::ganchos::instalar(target, reloc_dummy as *mut c_void) {
        Some(tr) => tr as *const u8,
        None => {
            crate::log_dev!("[relocreal] replace() RECUSOU — relocador não tratou o prólogo (falha real)");
            return;
        }
    };
    // o site agora começa com um abs-jump (não é mais o prólogo original)?
    let mut site4 = [0u8; 4];
    core::ptr::copy_nonoverlapping(target as *const u8, site4.as_mut_ptr(), 4);
    let site_changed = site4 != [orig[0], orig[1], orig[2], orig[3]];

    // (B) movz/movk x17 no trampolim: offset 8 = após stp,mov copiados verbatim (4B cada).
    let mut mm = [0u8; 20]; // 16B (movz/movk) + 4B (blr x17)
    core::ptr::copy_nonoverlapping(tramp.add(8), mm.as_mut_ptr(), 20);
    let materialized_b = {
        let mut v = 0u64;
        for i in 0..4 {
            let insn = u32::from_le_bytes([mm[i * 4], mm[i * 4 + 1], mm[i * 4 + 2], mm[i * 4 + 3]]);
            let hw = ((insn >> 21) & 0x3) as u64;
            let imm16 = ((insn >> 5) & 0xFFFF) as u64;
            v |= imm16 << (16 * hw);
        }
        v
    };
    let blr = u32::from_le_bytes([mm[16], mm[17], mm[18], mm[19]]);
    let blr_ok = blr == (0xD63F_0000u32 | (17 << 5)); // blr x17
    // o BL original NÃO deve aparecer verbatim no trampolim (foi expandido, não copiado).
    let mut tramp32 = [0u8; 32];
    core::ptr::copy_nonoverlapping(tramp, tramp32.as_mut_ptr(), 32);
    let bl_absent = !tramp32
        .chunks_exact(4)
        .any(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]) == bl);

    // revert + confirma byte-exato.
    crate::ganchos::remover(target);
    let mut after = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, after.as_mut_ptr(), 16);
    let byte_exact = after == orig;

    let reloc_ok = bl_target_a == materialized_b;
    let verdict = if reloc_ok && byte_exact && site_changed && blr_ok && bl_absent {
        ">>> RELOC-UNIVERSAL OK: BL real relocado (A==B) + blr x17 + BL não-verbatim + site patchado + revert byte-exato <<<"
    } else {
        "FALHA/verificar: alguma condição não bateu"
    };
    crate::log_dev!(
        "[relocreal] alvo@{t:#x} | BL->A={bl_target_a:#x} tramp movz/movk->B={materialized_b:#x} A==B:{reloc_ok} | blr_x17:{blr_ok}({blr:#010x}) BL_expandido:{bl_absent} site_patchado:{site_changed} | revert_byte_exato:{byte_exact} | {verdict}"
    );
}

/// Registro de nome de tweak — prova a fn nativa PURA de derive de TweakDBID (`CreateTweakDBID` core,
/// achada por RE 2026-07-16 @vmaddr 0x102176964): `u64 derive(const char* data, u32 len, u64 base)`
/// = CRC32-IEEE seeded com length telescópico. base=0 → `tweak_db_id(name)` (nome→id, o forward que
/// o gap pede: "resolver 'Items.X' → id no jogo vivo"). base=parent_id → `tweak_db_id_derive`.
/// A fn é PURA (0 escrita de memória, 0 lock) → chamável direto sem risco. Prova = o resultado
/// nativo BATE com `bwms_primitives` (nossa reimplementação, já provada offline vs 3.5M pares) —
/// confirma o endereço sob ASLR E que o hash do jogo == o nosso. Roda no MENU (não precisa player).
///
/// # Safety
/// Só chama a fn pura no endereço rebaseado, com args C-ABI corretos (ptr/len/base); sem efeito colateral.
pub(crate) unsafe fn prove_derive() {
    type DeriveFn = unsafe extern "C" fn(*const u8, u32, u64) -> u64;
    let addr = crate::rebase(0x1_0217_6964);
    if !crate::memoria::is_readable(addr as *const c_void, 4) {
        crate::log_dev!("[derivetest] endereço da derive ilegível (slide/versão?) — abortado");
        return;
    }
    let derive: DeriveFn = core::mem::transmute(addr);
    // teste 1 — base=0 (== tweak_db_id): nome cheio → id.
    let n1 = "Items.Preset_Lexington";
    let got1 = derive(n1.as_ptr(), n1.len() as u32, 0);
    let exp1 = bwms_primitives::tweak_db_id(n1);
    let ok1 = got1 == exp1;
    // teste 2 — base=parent (== tweak_db_id_derive): telescoping com seed != 0.
    let suf = ".Cool";
    let got2 = derive(suf.as_ptr(), suf.len() as u32, got1);
    let exp2 = bwms_primitives::tweak_db_id_derive(got1, suf);
    let exp2_full = bwms_primitives::tweak_db_id("Items.Preset_Lexington.Cool");
    let ok2 = got2 == exp2 && got2 == exp2_full;
    let verdict = if ok1 && ok2 {
        ">>> REGISTERNAME OK: derive nativo do jogo == bwms_primitives (nome→id forward provado no jogo vivo) <<<"
    } else {
        "FALHA/verificar: derive nativo divergiu do bwms_primitives"
    };
    crate::log_dev!(
        "[derivetest] nativo@{addr:p} | derive('{n1}',22,0)={got1:#x} vs bwms_primitives={exp1:#x} ok={ok1} | derive('{suf}',5,base)={got2:#x} vs derive={exp2:#x}/full={exp2_full:#x} ok={ok2} | {verdict}"
    );
}

// ===== Redirect de path inexistente: fazer um path INEXISTENTE "existir" via redirect (resource.copy) =====
// Achado por RE 2026-07-16 (workflow #2): ResourceDepot::CheckResource/ResourceExists @0x10151aa2c
// `bool(depot* x0, ResourcePath x1 /*u64 hash*/)`. O `resource.copy` usa um gancho antes dessa fn: se o
// path pedido é o "copy" (inexistente), reescreve pro path do asset real → o path passa a "existir".
// Depot singleton = [rebase(0x109d23188)+0x1f8] (já usado no projeto). PROVA log-only, menu-safe.
pub(crate) type CheckResFn = unsafe extern "C" fn(*mut c_void, u64) -> u8;

pub(crate) static COPY_ORIG: AtomicU64 = AtomicU64::new(0);

pub(crate) static COPY_DEPOT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut()); // `this` REAL capturado do jogo

pub(crate) static COPY_X: AtomicU64 = AtomicU64::new(0); // path inexistente (o "copy")

pub(crate) static COPY_REAL_Y: AtomicU64 = AtomicU64::new(0); // path real capturado (orig devolveu true)

pub(crate) static COPY_ARMED: AtomicBool = AtomicBool::new(false); // redirect X→Y ativo?

pub(crate) static COPY_PHASE: AtomicU32 = AtomicU32::new(0);

/// Replacement do CheckResource: (1) CAPTURA o `this` real do depot (x0) e o 1º path real (orig=true)
/// das chamadas naturais do jogo — corrige o crash anterior (o singleton [0x109d23188+0x1f8] era o
/// subobjeto ERRADO); (2) quando ARMADO, redireciona o path X (nosso copy inexistente) pro Y real
/// (estilo resource.copy) → CheckResource passa a devolver true pra X.
pub(crate) unsafe extern "C" fn check_res_replacement(depot: *mut c_void, path: u64) -> u8 {
    let orig = COPY_ORIG.load(Ordering::Relaxed);
    let f: CheckResFn = core::mem::transmute(orig);
    let x = COPY_X.load(Ordering::Relaxed);
    if COPY_ARMED.load(Ordering::Relaxed) && path == x {
        let y = COPY_REAL_Y.load(Ordering::Relaxed);
        if y != 0 {
            return f(depot, y); // "faz X existir" servindo Y
        }
    }
    let r = f(depot, path);
    if COPY_DEPOT.load(Ordering::Relaxed).is_null() && !depot.is_null() {
        COPY_DEPOT.store(depot, Ordering::Relaxed); // `this` autêntico do jogo
    }
    if r != 0 && COPY_REAL_Y.load(Ordering::Relaxed) == 0 && path != x && path != 0 {
        COPY_REAL_Y.store(path, Ordering::Relaxed); // path real (existe) capturado do jogo
    }
    r
}

/// Redirect de path inexistente (menu, log-only) — máquina de estados DIRIGIDA PELO TICK (não one-shot):
/// fase 0 instala o hook (captura o `this` real do depot + um Y real das chamadas do jogo), fase 1
/// espera a captura, fase 2 roda o golden (CheckResource(X) false→true via redirect) e reverte.
/// Corrige o crash da 1ª tentativa (usava o depot-singleton, subobjeto errado → SIGSEGV no field-deref).
///
/// # Safety
/// Só instala hook próprio no CheckResource + chama a fn com o `this` AUTÊNTICO capturado do jogo
/// (mesmo ponteiro que o jogo passa, garantidamente válido); redirect só pro nosso X único.
pub(crate) unsafe fn prove_copy_tick() {
    let check_addr = crate::rebase(0x1_0151_aa2c);
    match COPY_PHASE.load(Ordering::Relaxed) {
        0 => {
            if !crate::memoria::is_readable(check_addr as *const c_void, 4) {
                crate::log_dev!("[copytest] CheckResource ilegível — abortado");
                COPY_PHASE.store(9, Ordering::Relaxed);
                return;
            }
            COPY_X.store(
                bwms_primitives::resource_path_hash("base\\bwms\\nonexistent_copy_probe_zzz.mesh"),
                Ordering::Relaxed,
            );
            match crate::ganchos::instalar_com_threads_paradas(check_addr, check_res_replacement as *mut c_void, Some(&COPY_ORIG)) {
                Some(_) => {
                    COPY_PHASE.store(1, Ordering::Relaxed);
                    crate::log_dev!("[copytest] hook em CheckResource instalado — capturando o depot+Y reais das chamadas do jogo...");
                }
                None => {
                    crate::log_dev!("[copytest] replace() de CheckResource RECUSOU — abortado");
                    COPY_PHASE.store(9, Ordering::Relaxed);
                }
            }
        }
        1 => {
            // espera capturar o `this` real + um path real (o jogo chama CheckResource constantemente).
            if !COPY_DEPOT.load(Ordering::Relaxed).is_null() && COPY_REAL_Y.load(Ordering::Relaxed) != 0 {
                COPY_PHASE.store(2, Ordering::Relaxed);
            }
        }
        2 => {
            let depot = COPY_DEPOT.load(Ordering::Relaxed);
            let x = COPY_X.load(Ordering::Relaxed);
            let y = COPY_REAL_Y.load(Ordering::Relaxed);
            let check: CheckResFn = core::mem::transmute(check_addr); // hookado → passa pelo replacement
            COPY_ARMED.store(false, Ordering::Relaxed);
            let x_before = check(depot, x); // esperado 0 (não existe)
            let y_exists = check(depot, y); // esperado != 0 (sanidade do depot+Y)
            COPY_ARMED.store(true, Ordering::Relaxed);
            let x_after = check(depot, x); // redirect X→Y → esperado != 0
            COPY_ARMED.store(false, Ordering::Relaxed);
            crate::ganchos::remover(check_addr);
            COPY_PHASE.store(3, Ordering::Relaxed);
            let ok = x_before == 0 && y_exists != 0 && x_after != 0;
            let verdict = if ok {
                ">>> COPY-MAKEEXIST OK: path inexistente passou a EXISTIR (CheckResource false→true) via redirect (resource.copy) <<<"
            } else {
                "FALHA/verificar: esperado X_antes=0, Y_existe!=0, X_depois!=0"
            };
            crate::log_dev!(
                "[copytest] depot@{depot:p}(capturado do jogo) | Y#{y:#018x} existe={y_exists} | X#{x:#018x} existe_ANTES={x_before} existe_DEPOIS_do_redirect={x_after} | {verdict}"
            );
        }
        _ => {}
    }
}

// ===== Contrato de attach/detach: 2 hooks EMPILHADOS no MESMO alvo (LIFO) + Detach único =====
// Alvo: um stub JIT NOSSO de 1 instrução (`ret`), NÃO uma fn Rust compilada no nosso próprio
// dylib — ver `ganchos::stub_que_so_retorna` (achado 2026-07-16: hookar uma fn compilada aqui mesmo
// arriscou auto-modificação da página que o PRÓPRIO `prove_attach_detach` estava executando,
// crash silencioso confirmado ao vivo). Isola o que está sendo provado (o CONTRATO de
// empilhamento attach/detach) do relocador de opcode (já provado em separado, gap
// relocador universal) E de qualquer risco de self-modificação.
pub(crate) static AD_ORIG_HITS: AtomicU32 = AtomicU32::new(0);

pub(crate) static AD_H1_HITS: AtomicU32 = AtomicU32::new(0);

pub(crate) static AD_H2_HITS: AtomicU32 = AtomicU32::new(0);

pub(crate) static AD_TRAMP1: AtomicU64 = AtomicU64::new(0);

pub(crate) static AD_TRAMP2: AtomicU64 = AtomicU64::new(0);

pub(crate) type AdFn = unsafe extern "C" fn();

/// Substituto do hook 1 (o mais ANTIGO): chama através de `AD_TRAMP1` (a "orig" que o hook1
/// capturou ao instalar = o stub `ret` verdadeiro, já que foi o 1º) e só incrementa
/// `AD_ORIG_HITS` DEPOIS do call+ret voltar limpo — prova que a chamada através do trampolim
/// realmente alcançou o fim da cadeia e devolveu o controle certinho.
#[inline(never)]
pub(crate) unsafe extern "C" fn ad_repl1() {
    crate::log_dev!("[attachdetach] repl1 entrou");
    AD_H1_HITS.fetch_add(1, Ordering::Relaxed);
    let t = AD_TRAMP1.load(Ordering::Relaxed);
    if t != 0 {
        crate::log_dev!("[attachdetach] repl1 antes do through-call (tramp1)");
        let f: AdFn = core::mem::transmute(t as *const ());
        f();
        crate::log_dev!("[attachdetach] repl1 DEPOIS do through-call (voltou de tramp1)");
        AD_ORIG_HITS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Substituto do hook 2 (o mais NOVO): chama através de `AD_TRAMP2` (a "orig" que o hook2
/// capturou ao instalar = o site JÁ patchado pelo hook1 → encadeia pra `ad_repl1`).
#[inline(never)]
pub(crate) unsafe extern "C" fn ad_repl2() {
    crate::log_dev!("[attachdetach] repl2 entrou");
    AD_H2_HITS.fetch_add(1, Ordering::Relaxed);
    let t = AD_TRAMP2.load(Ordering::Relaxed);
    if t != 0 {
        crate::log_dev!("[attachdetach] repl2 antes do through-call (tramp2)");
        let f: AdFn = core::mem::transmute(t as *const ());
        f();
        crate::log_dev!("[attachdetach] repl2 DEPOIS do through-call (voltou de tramp2)");
    }
}

/// Contrato de attach/detach (in-process, menu-oneshot): prova (1) 2 hooks empilhados no
/// MESMO alvo simultaneamente, (2) a cadeia LIFO (repl2 chama repl1 chama o stub — cada um
/// EXATAMENTE 1x), (3) `remover_todos` (Detach) remove OS DOIS num call só e restaura os 16 bytes
/// ORIGINAIS byte-exato, (4) depois do Detach só o stub roda (os replacements não disparam
/// mais). `br` (usado no trampolim) não mexe em LR — por isso a cadeia de chamadas aninhadas
/// (`blr tramp2` → `br` pro repl1 → `blr tramp1` → `br` pro stub → `ret` → `ret` → `ret`)
/// devolve o controle certinho em cada nível, sem precisar de contabilidade extra.
///
/// # Safety
/// Só toca no stub JIT alocado aqui (não mexe em nada do motor do jogo, nem em código nosso já
/// compilado); hook+chamada+revert é síncrono, sem outra thread envolvida.
pub(crate) unsafe fn prove_attach_detach() {
    crate::log_dev!("[attachdetach] início — alocando stub JIT");
    let target = match crate::ganchos::stub_que_so_retorna() {
        Some(t) => t,
        None => {
            crate::log_dev!("[attachdetach] FALHA ao alocar o stub JIT — abortado");
            return;
        }
    };
    crate::log_dev!("[attachdetach] stub alocado @{target:p}");
    AD_ORIG_HITS.store(0, Ordering::Relaxed);
    AD_H1_HITS.store(0, Ordering::Relaxed);
    AD_H2_HITS.store(0, Ordering::Relaxed);
    AD_TRAMP1.store(0, Ordering::Relaxed);
    AD_TRAMP2.store(0, Ordering::Relaxed);

    let mut orig16 = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, orig16.as_mut_ptr(), 16);


    crate::log_dev!("[attachdetach] instalando hook1...");
    // attach hook1 (captura o stub `ret` verdadeiro).
    let tramp1 = match crate::ganchos::instalar(target, ad_repl1 as *mut c_void) {
        Some(t) => t,
        None => {
            crate::log_dev!("[attachdetach] FALHA ao instalar hook1 — abortado (alvo recusado pelo relocador?)");
            return;
        }
    };
    AD_TRAMP1.store(tramp1 as u64, Ordering::Relaxed);
    let hooks_after_1 = crate::ganchos::ganchos_no_endereco(target);
    crate::log_dev!("[attachdetach] hook1 instalado, tramp1={tramp1:p} ganchos_no_endereco={hooks_after_1}");

    crate::log_dev!("[attachdetach] instalando hook2 (empilhado)...");
    // attach hook2 EMPILHADO no MESMO alvo (captura o site já patchado pelo hook1).
    let tramp2 = match crate::ganchos::instalar(target, ad_repl2 as *mut c_void) {
        Some(t) => t,
        None => {
            crate::log_dev!("[attachdetach] FALHA ao instalar hook2 empilhado — desfazendo hook1 e abortando");
            crate::ganchos::remover(target);
            return;
        }
    };
    AD_TRAMP2.store(tramp2 as u64, Ordering::Relaxed);
    let hooks_after_2 = crate::ganchos::ganchos_no_endereco(target);
    crate::log_dev!("[attachdetach] hook2 instalado, tramp2={tramp2:p} ganchos_no_endereco={hooks_after_2}");

    crate::log_dev!("[attachdetach] chamando o alvo (deve disparar repl2->repl1->stub)...");
    // chama o alvo: deve disparar repl2 -> repl1 -> original, cada um exatamente 1x (LIFO).
    let f: AdFn = core::mem::transmute(target as *const ());
    f();
    let (h2, h1, o1) = (
        AD_H2_HITS.load(Ordering::Relaxed),
        AD_H1_HITS.load(Ordering::Relaxed),
        AD_ORIG_HITS.load(Ordering::Relaxed),
    );
    let chain_ok = h2 == 1 && h1 == 1 && o1 == 1;
    crate::log_dev!("[attachdetach] chamada voltou: h2={h2} h1={h1} orig={o1} chain_ok={chain_ok}");

    crate::log_dev!("[attachdetach] remover_todos (Detach)...");
    // Detach: remover_todos remove OS DOIS num call só.
    crate::ganchos::remover_todos(target);
    let hooks_after_detach = crate::ganchos::ganchos_no_endereco(target);
    let mut after16 = [0u8; 16];
    core::ptr::copy_nonoverlapping(target as *const u8, after16.as_mut_ptr(), 16);
    let byte_exact = after16 == orig16;
    crate::log_dev!(
        "[attachdetach] remover_todos voltou: ganchos_no_endereco={hooks_after_detach} byte_exato={byte_exact}"
    );

    crate::log_dev!("[attachdetach] chamando o alvo de novo (pos-detach)...");
    // chama de novo: o site agora é o stub `ret` puro (sem side-effect observável) — a única
    // coisa que dá pra confirmar é que os REPLACEMENTS não disparam mais (contadores parados) E
    // que a chamada não crashou (chegamos até aqui pra checar).
    f();
    crate::log_dev!("[attachdetach] 2a chamada voltou (nao crashou)");
    let (h2b, h1b) = (AD_H2_HITS.load(Ordering::Relaxed), AD_H1_HITS.load(Ordering::Relaxed));
    let only_orig_after_detach = h2b == h2 && h1b == h1;

    let ok = hooks_after_1 == 1
        && hooks_after_2 == 2
        && chain_ok
        && hooks_after_detach == 0
        && byte_exact
        && only_orig_after_detach;
    let verdict = if ok {
        ">>> ATTACH-DETACH-CONTRACT OK: 2 hooks empilhados no MESMO alvo (LIFO, cada um chamou o próximo 1x) + Detach removeu OS DOIS num call só + prólogo byte-exato + só a original roda depois <<<"
    } else {
        "FALHA/verificar: alguma condição não bateu"
    };
    crate::log_dev!(
        "[attachdetach] hooks_apos_1={hooks_after_1} hooks_apos_2={hooks_after_2} | cadeia(h2={h2},h1={h1},orig={o1}):{chain_ok} | hooks_apos_detach={hooks_after_detach} byte_exato={byte_exact} | replacements_pararam_depois(h2={h2b},h1={h1b}):{only_orig_after_detach} | {verdict}"
    );
}

pub(crate) static A17_CHAMADAS: AtomicU64 = AtomicU64::new(0);

/// Partes vistas pelo hook: `(token com 1 strong a mais, campo +0x48 do token, tamanho, classes dos objetos)`.
pub(crate) static A17_VISTAS: std::sync::Mutex<Vec<([u64; 2], u64, u32, Vec<u64>)>> = std::sync::Mutex::new(Vec::new());

/// Classes dos objetos (`Handle<ISerializable>`, passo 0x10) do array extraído, até `limite`.
pub(crate) unsafe fn a17_classes(entries: u64, tam: u32, limite: usize) -> Vec<u64> {
    let n = (tam as usize).min(limite);
    if entries == 0 || n == 0 || !crate::memoria::is_readable(entries as *const c_void, n * 16) {
        return Vec::new();
    }
    (0..n)
        .map(|i| {
            let obj = ((entries as usize + i * 16) as *const u64).read_unaligned() as *mut c_void;
            crate::rtti::type_name_hash(crate::rtti::class_of(obj))
        })
        .collect()
}

pub(crate) static A17_PATCHES_APLICADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A17_PATCHES_NAO_PRONTOS: AtomicU64 = AtomicU64::new(0);

/// Com `~/.bwms-hook-a17-partes`: o hook guarda as primeiras partes vistas (17a).
pub(crate) static A17_OBSERVAR: AtomicBool = AtomicBool::new(false);

pub(crate) static A17_GARMENT_CHAMADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A17_GARMENT_APLICADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A17_GARMENT_OFFSET_LOGADO: AtomicBool = AtomicBool::new(false);

/// 17e, sonda: `[classe, chamadas, 1º objeto, +0x30 dele]` do objeto em `params+0x00` do job de garment. O `0x1019ca448` só soma `0x1d8` (é o template da
/// parte), mas o `+0x30` visto nos boots 95 e 114 não era caminho; o nome da classe sai no tick (`~/.bwms-hook-a17e-sonda`).
pub(crate) static A17_GARMENT_CLASSES: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

/// 17e v3: tamanho da lista de `THandle<ent::ISkinTargetComponent>` do contexto do garment (`+0x10`, tamanho em `+0x1c`), onde o job anexa os componentes
/// extraídos com skin (desmonte de `0x1017ee34c`: reserve `0x100028674` com o `MoveAfterReallocation` desse tipo); `u32::MAX` sem contexto legível.
pub(crate) unsafe fn a17e_alvos_de_skin(contexto: u64) -> u32 {
    if contexto == 0 || !crate::memoria::is_readable((contexto + 0x10) as *const c_void, 0x10) {
        return u32::MAX;
    }
    ((contexto + 0x1c) as *const u32).read_unaligned()
}

/// `CName` (`+0x40` do `entIComponent`) das entradas `[de, ate)` da lista de skin do contexto, até 16.
pub(crate) unsafe fn a17e_nomes_de_skin(contexto: u64, de: u32, ate: u32) -> Vec<u64> {
    if contexto == 0 || ate == u32::MAX || de >= ate {
        return Vec::new();
    }
    let dados = ((contexto + 0x10) as *const u64).read_unaligned();
    let n = (ate - de).min(16) as u64;
    if dados == 0 || !crate::memoria::is_readable((dados + (de as u64) * 0x10) as *const c_void, (n * 0x10) as usize) {
        return Vec::new();
    }
    (0..n)
        .filter_map(|i| {
            let obj = ((dados + (de as u64 + i) * 0x10) as *const u64).read_unaligned();
            (obj != 0 && crate::memoria::is_readable((obj + 0x40) as *const c_void, 8)).then(|| ((obj + 0x40) as *const u64).read_unaligned())
        })
        .collect()
}

/// `(caminho da parte, lista de skin antes, depois da parte, depois dos patches, nomes anexados pela parte, nomes anexados pelos patches)`, até 16.
pub(crate) static A17E_V3_AMOSTRAS: std::sync::Mutex<Vec<(u64, u32, u32, u32, Vec<u64>, Vec<u64>)>> = std::sync::Mutex::new(Vec::new());

pub(crate) unsafe fn a17e_amostrar(contexto: u64, caminho: u64, t0: u32, t1: u32) {
    let t2 = a17e_alvos_de_skin(contexto);
    let (parte, patches) = (a17e_nomes_de_skin(contexto, t0, t1), a17e_nomes_de_skin(contexto, t1, t2));
    if let Ok(mut g) = A17E_V3_AMOSTRAS.try_lock() {
        if g.len() < 16 {
            g.push((caminho, t0, t1, t2, parte, patches));
        }
    }
}

/// 17e, antes da original: template da parte, sondas e patches que casariam, só lendo.
pub(crate) unsafe fn a17_garment_observar(params: *mut [u64; 2]) {
    let tpl = (*params)[0];
    if tpl == 0 || !crate::memoria::is_readable((tpl + 0x30) as *const c_void, 8) {
        return;
    }
    let caminho = ((tpl + 0x30) as *const u64).read_unaligned();
    let n = A17_GARMENT_CHAMADAS.fetch_add(1, Ordering::Relaxed);
    // Sonda v2 (boot 118: `class_of` nulo em todas as chamadas): vptr com o endereço estático, primeiras palavras do objeto, objetos do pacote em `+0x1d8` e
    // as primeiras palavras dos params, pra identificar o objeto offline.
    if n < 4 {
        let q = |o: u64| if crate::memoria::is_readable((tpl + o) as *const c_void, 8) { ((tpl + o) as *const u64).read_unaligned() } else { 0 };
        let base = (crate::rebase(0x1_0000_0000) as u64).wrapping_sub(0x1_0000_0000);
        let (ini, fim) = (q(0x1d8 + 0x20), q(0x1d8 + 0x28));
        let objetos = if ini != 0 && fim >= ini { (fim - ini) / 8 } else { 0 };
        let p = params as *const u8;
        let palavra = |k: usize| if crate::memoria::is_readable(p.add(k * 8) as *const c_void, 8) { (p.add(k * 8) as *const u64).read_unaligned() } else { 0 };
        crate::veredito_dev!(
            "[a17e] objeto #{n} {tpl:#x}: vptr {:#x} (estático {:#x}); +0x08..+0x38 [{}]; pacote em +0x1d8 com {objetos} objeto(s); params {params:p} [{}]",
            q(0),
            q(0).wrapping_sub(base),
            (1..8u64).map(|k| format!("{:#x}", q(k * 8))).collect::<Vec<_>>().join(" "),
            (0..8usize).map(|k| format!("{:#x}", palavra(k))).collect::<Vec<_>>().join(" ")
        );
    }
    if n < 256 {
        let cls = crate::rtti::class_of(tpl as *mut c_void) as u64;
        if let Ok(mut g) = A17_GARMENT_CLASSES.try_lock() {
            if let Some(i) = g.iter().position(|e| e[0] == cls) {
                g[i][1] += 1;
            } else if g.len() < 16 {
                g.push([cls, 1, tpl, caminho]);
            }
        }
    }
    // Template da parte alvo: o do token pedido na carga da tabela (`+0x28`), comparado por ponteiro. No boot 95 o `+0x30` do template visto aqui não era hash
    // de caminho (ponteiros, zeros e bytes de texto) e nenhuma chamada casou pelo caminho.
    let instancia = |t: [u64; 2]| -> u64 {
        if t[0] != 0 && crate::register::status_do_token(t[0]) == 1 && crate::memoria::is_readable((t[0] + 0x28) as *const c_void, 8) {
            ((t[0] + 0x28) as *const u64).read_unaligned()
        } else {
            0
        }
    };
    let patches: Vec<(u64, [u64; 2], u64)> = match PATCHES_DE_RECURSO.try_read() {
        Ok(g) => g
            .do_alvo_ou_da_instancia(Some(caminho), tpl, |p| p.props & A17_PROP_COMPONENTS != 0, instancia)
            .map(|p| (p.patch, p.token, p.alvo))
            .collect(),
        Err(_) => return,
    };
    if n < 24 || !patches.is_empty() {
        crate::veredito_dev!(
            "[a17] garment job #{n} (antes da original): template {tpl:#x}, +0x30 {caminho:#018x}, {} patch(es) da tabela casariam (extração na v3)",
            patches.len()
        );
    }
    if let Some((_, _, alvo)) = patches.first() {
        if !A17_GARMENT_OFFSET_LOGADO.swap(true, Ordering::Relaxed) && crate::memoria::is_readable(tpl as *const c_void, 0x100) {
            let offsets: Vec<String> =
                (0..0x100usize).step_by(8).filter(|o| ((tpl as usize + o) as *const u64).read_unaligned() == *alvo).map(|o| format!("{o:#x}")).collect();
            crate::veredito_dev!("[a17] garment: hash do caminho da parte alvo {alvo:#018x} no template em [{}]", offsets.join(", "));
        }
    }
    let _ = &A17_GARMENT_APLICADOS;
}

pub(crate) static A51_APLICADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51_NAO_PRONTOS: AtomicU64 = AtomicU64::new(0);

pub(crate) const A51B_TESTE_ALVO: &str = "base\\characters\\garment\\player_equipment\\torso\\t1_001_pma_shirt__militech_agent.ent";

// ===== `#51` — checagens: depot e classe do recurso do patch (2026-09-12) =====
// O `Configure` tira patch que o depot não tem; o `OnAppearanceResourceLoad`/`OnEntityTemplateLoad` pulam `partsValues`/`resolvedDependencies` inexistentes; o
// `GetPatchToken` só devolve recurso do tipo pedido (`IsInstanceOf<T>`). Depot = `ResourceDepot::CheckResource` (`0x10151aa2c`, `bool(depot, caminho)`) com o
// depot autêntico capturado no `InitializeArchives` (`PATHB_DEPOT`, o mesmo do `BwmsResourceExists`), chamado direto, sem hook e sem log — o jogo chama a
// função o tempo todo das threads de carga.
pub(crate) static A17_PATCHES_INEXISTENTES: AtomicU64 = AtomicU64::new(0);

/// 1ª cópia de `partsOverrides` feita pelo hook, pro teste da 51c-4: `[definição do alvo, índice, buffer novo, definição do patch, índice no patch]`. Com
/// alvo e patch iguais byte a byte, só o endereço da cópia separa o elemento anexado do original.
pub(crate) static A51C4_AMOSTRA: std::sync::Mutex<Option<[u64; 5]>> = std::sync::Mutex::new(None);

pub(crate) static A51D2_AMOSTRAS: std::sync::Mutex<Vec<A51d2Amostra>> = std::sync::Mutex::new(Vec::new());

/// Uma mescla v2 no hook: caminho antes e do patch com a decisão, pra `baseMesh` e `baseTexture`; `targets` antes e depois, substituídos, anexados, falhas e
/// decisão.
#[derive(Clone, Debug)]
pub(crate) struct A51d2Amostra {
    pub(crate) alvo: u64,
    pub(crate) patch: u64,
    pub(crate) mesh: (u64, u64, bool),
    pub(crate) tex: (u64, u64, bool),
    pub(crate) targets: (usize, usize, usize, usize, usize, bool),
}

/// Templates FPP do V pro teste da 51f: um deles é o do save. Com Phantom Liberty o V sai do `_ep1` (boot 108: o job de 125 objetos era o
/// `player_wa_fpp_ep1.ent`, pelo hash do caminho em `+0x30`).
pub(crate) const A51F_TESTE_ALVOS: [&str; 4] = [
    "ep1\\characters\\entities\\player\\player_wa_fpp_ep1.ent",
    "ep1\\characters\\entities\\player\\player_ma_fpp_ep1.ent",
    "base\\characters\\entities\\player\\player_wa_fpp.ent",
    "base\\characters\\entities\\player\\player_ma_fpp.ent",
];

/// Uma amostra por mescla com substituídos (até 8): no boot 109 os dois templates FPP `_ep1` foram mesclados e a 1ª amostra era a do builder que não era o V.
pub(crate) static A51F_AMOSTRAS: std::sync::Mutex<Vec<A51fAmostra>> = std::sync::Mutex::new(Vec::new());

/// 51f-1b: `[entidade, template, tamanho dos resultados, hash do caminho do template]` de cada job, lidos do builder depois da original (entidade em
/// `+0xb0`), pra achar o builder do V no tick. No boot 107 a mescla não rodou: nenhum template de builder bateu por ponteiro com o dos tokens de teste.
#[cfg(feature = "devtools")]
pub(crate) static A51F_ENTIDADES: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

/// 1ª mescla com substituídos: builder, template, tamanho dos resultados antes e depois, contagens e até 8 pares `(instância nova no resultado, trocada)`.
#[derive(Clone)]
pub(crate) struct A51fAmostra {
    pub(crate) builder: u64,
    pub(crate) template: u64,
    pub(crate) antes: usize,
    pub(crate) depois: usize,
    pub(crate) substituidos: usize,
    pub(crate) anexados: usize,
    pub(crate) pares: Vec<(u64, u64)>,
}

/// 51f-1b, depois da original com o builder ainda travado: guarda a entidade que o job montou, com o template e o tamanho dos resultados de antes.
/// O registro só alimenta o teste 51f do harness: no público não há leitor.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51f_registrar(builder: u64, tam: usize) {
    if !crate::memoria::is_readable(builder as *const c_void, 0xb8) {
        return;
    }
    let entidade = ((builder + 0xb0) as *const u64).read_unaligned();
    let tpl = ((builder + 0x90) as *const u64).read_unaligned();
    if entidade == 0 {
        return;
    }
    if let Ok(mut v) = A51F_ENTIDADES.try_lock() {
        if v.len() < 0x4000 {
            v.push([entidade, tpl, tam as u64, crate::jogo::recursos::hash_do_caminho_do_recurso(tpl)]);
        }
    }
}

/// Com `~/.bwms-hook-a51f2-teste`: nos builders dos templates FPP do V, o próprio `.app` da aparência entra como patch (identidade, prova por ponteiro).
pub(crate) static A51F2_TESTE: AtomicBool = AtomicBool::new(false);

/// Teste da definição `None` (`~/.bwms-hook-a51f2-none-teste`): os nomes das definições do patch contam como `None`, então a 1ª vale pra toda aparência.
pub(crate) static A51F2_TESTE_NONE: AtomicBool = AtomicBool::new(false);

pub(crate) static A51F2_AMOSTRAS: std::sync::Mutex<Vec<A51fAmostra>> = std::sync::Mutex::new(Vec::new());

/// Teste: `(template, recurso .app, nome da definição)` das aparências mescladas por identidade.
pub(crate) static A51F2_RECURSOS: std::sync::Mutex<Vec<[u64; 3]>> = std::sync::Mutex::new(Vec::new());

/// Hashes dos caminhos dos templates FPP do V (`A51F_TESTE_ALVOS`).
pub(crate) fn a51f_teste_hashes() -> &'static [u64] {
    static H: std::sync::OnceLock<Vec<u64>> = std::sync::OnceLock::new();
    H.get_or_init(|| A51F_TESTE_ALVOS.iter().map(|c| bwms_primitives::resource_path_hash(c)).collect())
}

pub(crate) static A51F3_TESTE: AtomicBool = AtomicBool::new(false);

pub(crate) static A51F3_DEFINICOES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_COM_TAG: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_SCHEMAS_COM_TAG: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_INCLUSOES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_PARTES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_NAO_PRONTAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_SEM_PACOTE: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_LOGADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51F3_AMOSTRAS: std::sync::Mutex<Vec<A51fAmostra>> = std::sync::Mutex::new(Vec::new());

pub(crate) static A51G_CONTEXTOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_CURVESETS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_COPIADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_SEM_CAPACIDADE: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_TIPO_DIFERENTE: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_NOVAS_PENDENTES: AtomicU64 = AtomicU64::new(0);

/// `[curveset alvo, hash do patch, nome da curva, bytes do alvo já iguais aos do patch antes da cópia]` das cópias (até 32).
pub(crate) static A51G_AMOSTRAS: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

/// 51g-b: caminhos dos `CurveSet` que passaram pelo hook (até 1024) e patches que não ficaram prontos na espera. No boot 113 passaram 181 `CurveSet` e nenhuma
/// cópia saiu, com o alvo e o patch prontos no tick.
pub(crate) static A51G_VISTOS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

pub(crate) static A51G_PATCH_NAO_PRONTO: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_REALOCADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51G_ANEXADAS: AtomicU64 = AtomicU64::new(0);

/// Contextos do serializer com ouvinte e `Resource/Load` despachados.
pub(crate) static C122_CONTEXTOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static C122_DESPACHOS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51H_DEVICES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51H_ATRIBUIDAS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51H_NOVAS_PENDENTES: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51H_PATCH_NAO_PRONTO: AtomicU64 = AtomicU64::new(0);

/// `[recurso alvo, hash do patch, chave, valor do alvo já igual ao do patch antes]` das atribuições (até 32).
#[cfg(feature = "devtools")]
pub(crate) static A51H_AMOSTRAS: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) static A51H_INSERIDAS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51H_CRESCIMENTOS: AtomicU64 = AtomicU64::new(0);

/// Valor de device igual: `unk00`, `className`, posição e as duas listas.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51h_valor_igual(a: *const u8, b: *const u8) -> bool {
    let fixo = |p: *const u8| -> Vec<u8> { [0x00usize..0x10, 0x30..0x40].iter().flat_map(|r| std::slice::from_raw_parts(p.add(r.start), r.len()).to_vec()).collect() };
    let lista = |p: *const u8| -> Vec<u64> {
        let (b, n, _) = crate::jogo::listas::dynarray_no_campo(p, 0);
        if n == 0 || n > 0x1_0000 || !crate::memoria::is_readable(b as *const c_void, n * 8) { Vec::new() } else { (0..n).map(|i| (b.add(i * 8) as *const u64).read_unaligned()).collect() }
    };
    fixo(a) == fixo(b) && lista(a.add(0x10)) == lista(b.add(0x10)) && lista(a.add(0x20)) == lista(b.add(0x20))
}

pub(crate) static A51I_MESHES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I_NOMES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I_LOGADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I2_TROCADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I2_ANEXADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I2_FALHAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I2_NAO_PRONTOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I2_LOGADOS: AtomicU64 = AtomicU64::new(0);

/// `[alvo, patch, nome, instância do clone]` das aparências trocadas ou anexadas (até 32).
pub(crate) static A51I2_AMOSTRAS: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

pub(crate) static A51I3_COPIADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I3_FALHAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I3_LOGADOS: AtomicU64 = AtomicU64::new(0);

/// `[caminho do mesh alvo, patch, blob novo, blob do patch]` (até 16).
pub(crate) static A51I3_AMOSTRAS: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

/// Cópias com o `renderBuffer` conferido (tamanho e bytes do patch, em memória própria) na hora da cópia, e as que divergiram.
pub(crate) static A51I3_CONFERIDAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51I3_DIVERGENTES: AtomicU64 = AtomicU64::new(0);

/// Teste: `(caminho, palavras do renderBuffer)` do alvo e do controle logo depois do `PostLoad` original (até 16).
#[cfg(feature = "devtools")]
pub(crate) static A51I3_APOS: std::sync::Mutex<Vec<(u64, String)>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) static A51I3_CONTROLE: AtomicU64 = AtomicU64::new(0);

/// `DataBuffer` `a` com o tamanho e os bytes de `b`, em memória própria (ou os 2 sem dados).
pub(crate) unsafe fn a51_databuffer_igual(a: *const u8, b: *const u8) -> bool {
    let (da, ta) = ((a as *const u64).read_unaligned(), (a.add(8) as *const u32).read_unaligned() as usize);
    let (db, tb) = ((b as *const u64).read_unaligned(), (b.add(8) as *const u32).read_unaligned() as usize);
    ta == tb
        && (tb == 0
            || (da != db
                && da != 0
                && db != 0
                && crate::memoria::is_readable(da as *const c_void, ta)
                && crate::memoria::is_readable(db as *const c_void, tb)
                && std::slice::from_raw_parts(da as *const u8, ta) == std::slice::from_raw_parts(db as *const u8, tb)))
}

/// As 5 palavras de um `DataBuffer` (dados, tamanho e alinhamento, alocador, última) pro log.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51_databuffer_palavras(p: *const u8) -> String {
    let w: Vec<String> = (0..5).map(|i| format!("{:#x}", (p.add(i * 8) as *const u64).read_unaligned())).collect();
    format!("[{}]", w.join(" "))
}

/// 51i-3 (teste): palavras do `renderBuffer` do blob do alvo acompanhado e do controle logo depois do `PostLoad` original. Só memória.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51i3_apos_original(mesh: *mut c_void) {
    if !MESCLA_DE_RENDER_BLOB_ATIVA.load(Ordering::Relaxed) || mesh.is_null() {
        return;
    }
    let caminho = ((mesh as *const u8).add(0x30) as *const u64).read_unaligned();
    let acompanhado = caminho != 0
        && (caminho == A51I3_CONTROLE.load(Ordering::Relaxed) || A51I3_AMOSTRAS.try_lock().map(|g| g.iter().any(|a| a[0] == caminho)).unwrap_or(false));
    if !acompanhado {
        return;
    }
    let blob = ((mesh as *const u8).add(0x1f8) as *const u64).read_unaligned();
    let palavras =
        if blob != 0 && crate::memoria::is_readable((blob + 0x110) as *const c_void, 0x28) { a51_databuffer_palavras((blob + 0x110) as *const u8) } else { "sem blob".to_string() };
    if let Ok(mut g) = A51I3_APOS.try_lock() {
        if g.len() < 16 {
            g.push((caminho, palavras));
        }
    }
}

pub(crate) static A51D3_COPIADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51D3_FALHAS: AtomicU64 = AtomicU64::new(0);

/// `[alvo, patch, blob novo, blob do patch]` (até 16).
pub(crate) static A51D3_AMOSTRAS: std::sync::Mutex<Vec<[u64; 4]>> = std::sync::Mutex::new(Vec::new());

/// Cópias com os 3 `DataBuffer` conferidos na hora da cópia, e as que divergiram.
pub(crate) static A51D3_CONFERIDAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51D3_DIVERGENTES: AtomicU64 = AtomicU64::new(0);

/// Teste: `(caminho, buffers)` do alvo e do controle logo depois do `PostLoad` original (até 16).
#[cfg(feature = "devtools")]
pub(crate) static A51D3_APOS: std::sync::Mutex<Vec<(u64, String)>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) static A51D3_CONTROLE: AtomicU64 = AtomicU64::new(0);

/// Os 3 `DataBuffer` do blob de morph `a` (o `renderBuffer` do `baseBlob`, `diffsBuffer` e `mappingBuffer`) iguais aos de `b`, em memória própria.
pub(crate) unsafe fn a51d3_buffers_iguais(a: *const u8, b: *const u8) -> bool {
    let (ba, bb) = ((a.add(0x30) as *const u64).read_unaligned(), (b.add(0x30) as *const u64).read_unaligned());
    ba != 0
        && bb != 0
        && ba != bb
        && a51_databuffer_igual((ba + 0x110) as *const u8, (bb + 0x110) as *const u8)
        && a51_databuffer_igual(a.add(0xc0), b.add(0xc0))
        && a51_databuffer_igual(a.add(0xe8), b.add(0xe8))
}

/// As palavras dos 3 `DataBuffer` de um blob de morph, pro log.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51d3_buffers_texto(blob: *const u8) -> String {
    let base = (blob.add(0x30) as *const u64).read_unaligned();
    let base_txt =
        if base != 0 && crate::memoria::is_readable((base + 0x110) as *const c_void, 0x28) { a51_databuffer_palavras((base + 0x110) as *const u8) } else { "sem baseBlob".to_string() };
    format!("baseBlob.renderBuffer {base_txt}, diffs {}, mapping {}", a51_databuffer_palavras(blob.add(0xc0)), a51_databuffer_palavras(blob.add(0xe8)))
}

/// 51d v3 (teste): buffers do blob do alvo acompanhado e do controle logo depois do `PostLoad` original. Só memória.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51d3_apos_original(res: *mut c_void) {
    if !MORPH_BLOB_OFFSETS_OK.load(Ordering::Relaxed) || res.is_null() {
        return;
    }
    let caminho = ((res as *const u8).add(0x30) as *const u64).read_unaligned();
    let acompanhado = caminho != 0
        && (caminho == A51D3_CONTROLE.load(Ordering::Relaxed) || A51D3_AMOSTRAS.try_lock().map(|g| g.iter().any(|a| a[0] == res as u64)).unwrap_or(false));
    if !acompanhado {
        return;
    }
    let blob = ((res as *const u8).add(0xa0) as *const u64).read_unaligned();
    let texto = if blob != 0 && crate::memoria::is_readable(blob as *const c_void, 0x110) { a51d3_buffers_texto(blob as *const u8) } else { "sem blob".to_string() };
    if let Ok(mut g) = A51D3_APOS.try_lock() {
        if g.len() < 16 {
            g.push((caminho, texto));
        }
    }
}

pub(crate) static A51B_SCHEMAS_CRIADOS: AtomicU64 = AtomicU64::new(0);

/// Templates que receberam schema novo no hook (até 16).
pub(crate) static A51B_CRIADOS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

pub(crate) static A51J_MUNDO: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51K_CHAMADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51K_CASADOS_28: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51K_CASADOS_30: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51K_JUNCOES: AtomicU64 = AtomicU64::new(0);

pub(crate) static A51K_JA_PRONTOS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn a51k_job_texto(j: Option<(u64, bool)>) -> String {
    match j {
        Some((j, true)) => format!("job {j:#x} pendente"),
        Some((j, false)) => format!("job {j:#x} sem pendência"),
        None => "sem job".to_string(),
    }
}

#[cfg(feature = "devtools")]
pub(crate) static A51K_TESTE_ALVO: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51K_TESTE_CONTROLE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51K_TESTE_PATCH_TOKEN: AtomicU64 = AtomicU64::new(0);

/// Estado do patch no `PostLoad` do alvo e do controle: 0 sem `PostLoad`, 1 patch sem pendência, 2 patch pendente.
#[cfg(feature = "devtools")]
pub(crate) static A51K_VISTO_ALVO: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A51K_VISTO_CONTROLE: AtomicU64 = AtomicU64::new(0);

/// Teste da 51k no `PostLoad` do `CMesh` (thread de job, só memória): o patch já terminou quando o alvo e o controle chegam aqui?
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a51k_observar_postload(mesh: *mut c_void) {
    let token = A51K_TESTE_PATCH_TOKEN.load(Ordering::Relaxed);
    if token == 0 || mesh.is_null() {
        return;
    }
    let caminho = ((mesh as *const u8).add(0x30) as *const u64).read_unaligned();
    let destino = if caminho == A51K_TESTE_ALVO.load(Ordering::Relaxed) {
        &A51K_VISTO_ALVO
    } else if caminho == A51K_TESTE_CONTROLE.load(Ordering::Relaxed) {
        &A51K_VISTO_CONTROLE
    } else {
        return;
    };
    destino.store(if matches!(crate::arquivos_do_jogo::job_do_token(token), Some((_, true))) { 2 } else { 1 }, Ordering::Relaxed);
}

pub(crate) static A48_PATCHES_REGISTRADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A48_ESTADOS_CRIADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A48_TRAVA_OCUPADA: AtomicU64 = AtomicU64::new(0);

/// `Entity/Extract`: despachos com algum ouvinte feitos no hook do job do builder.
pub(crate) static C130D_DESPACHOS: AtomicU64 = AtomicU64::new(0);

pub(crate) static C130D_SEM_ANOTACAO: AtomicU64 = AtomicU64::new(0);

pub(crate) static C130D_TESTE: AtomicBool = AtomicBool::new(false);

pub(crate) static C130D_CONGELADO: AtomicBool = AtomicBool::new(false);

/// Componentes somados achados nos componentes do builder depois da original.
pub(crate) static C130D_NO_BUILDER: AtomicU64 = AtomicU64::new(0);

/// (handle forte do componente somado, raiz do builder, entidade do builder depois da original, dono visto: 0 nenhum, 1 outro, 2 a raiz ou a entidade).
pub(crate) static C130D_SOMADOS: std::sync::Mutex<Vec<([u64; 2], u64, u64, u8)>> = std::sync::Mutex::new(Vec::new());

thread_local! {
    /// Builder do despacho em curso nesta thread (0 fora de despacho).
    pub(crate) static C130D_ATUAL: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// Anotações feitas nesta thread durante o despacho em curso.
    pub(crate) static C130D_ANOTOU: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Componente somado no despacho em curso e contagem de componentes do template lida antes da soma.
    pub(crate) static C130D_SOMADO: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    pub(crate) static C130D_CONTAGEM: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

thread_local! {
    /// Raiz do builder sendo armado nesta thread (as partes 40–43 chegam logo depois da 16).
    pub(crate) static C134_ARMANDO: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// 134e: o handler armou o ramo do builder neste despacho (o hook do job redespacha uma vez) e o redespacho em curso.
    pub(crate) static C134E_REDESPACHAR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    pub(crate) static C134E_REDESPACHANDO: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// 134e: entidade da parte 21 no despacho de `Component/Toggle` em curso.
    pub(crate) static C134E_TOGGLE_ENTIDADE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// `(root, recurso)` → fase filha da raiz com esse recurso, 0 sem ela (o próprio `Start` sai cedo com ela).
#[cfg(feature = "devtools")]
pub(crate) const A34_FILHO_DO_RECURSO_VM: u64 = 0x1_0281_20e8;

pub(crate) static A34_CHAMADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A34_FACTS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A34_RECURSO_TESTE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) static A34_RECUSADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) static A35_RECRIADAS: AtomicU64 = AtomicU64::new(0);

// ===== `#55`/`#35`: início forçado de nó de fase na restauração (2026-09-18) =====
// O hook do `QuestsSystem::OnGameRestored` (`0x102a0b26c`, slot `+0x158`) chama `register::questphase_forcar_na_restauracao` antes da original. Com
// `~/.bwms-hook-a55-teste` e a trava confirmada (depois do #34 no mesmo boot), a mesma função de produção roda sobre uma quest viva: um nó
// `questFactsDBManagerNodeDefinition` sintético (grava `bwms_fs_test = 1`, sem sockets) entra no fim do `nodes` do grafo da fase dela e no registro de
// forçados; o `ForceStartNode` do motor o executa na hora, e o fact de teste e o fact do nó forçado passam a 1. Controle: um id que nenhum nó usa não
// muda nada. Volta: `RemoveFacts` dos 2 facts, o `nodes` de volta ao tamanho de antes e o registro vazio; a chave que a referência insere no NodeHashMap
// quando falta fica na memória (o motor não tem remoção por chave, e o `OnGameSave` não grava o mapa).
pub(crate) static A55_FORCADOS_NA_RESTAURACAO: AtomicU64 = AtomicU64::new(0);

// ===== `#35`: patch de `.quest` na recriação e início forçado do nó de mod (2026-09-18) =====
// Com `~/.bwms-hook-a35-teste` e a tabela `bwms-questphase.txt` do teste (`bwms-teste-35q`: nó de fase de mod `mod\bwms\teste\fase_35_quest.questphase`,
// que não existe, pendurado no nó de início do `base\quest\cyberpunk2077.quest`): na carga do save, o hook da recriação (`0x102812f24`) aplica o patch
// no `.quest` antes da instância ser recriada e registra o nó como forçado. O `OnGameRestored` roda antes da trava de save, então o teste adia o
// forçamento e, com a trava confirmada, chama a mesma função de produção: o `ForceStartNode` do motor executa o nó de fase, que pede o recurso ao
// `QuestResourceLoader`, não acha e estaciona sem saída (`+0x5c` = 1, RE de 09-18); o fact do nó passa a 1. Volta: `RemoveFacts` e o registro vazio;
// o nó estacionado fica na sessão, que o runner descarta sem salvar.
pub(crate) static A35_RESTAURACOES_ADIADAS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devlog")]
pub(crate) static POSTLOAD_CNT_CMESH: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devlog")]
pub(crate) static POSTLOAD_CNT_MORPHTGT: AtomicU64 = AtomicU64::new(0);

// `#51`: EntityTemplate::PostLoad / AppearanceResource::PostLoad (candidatos 2026-08-12).
#[cfg(feature = "devlog")]
pub(crate) static POSTLOAD_CNT_ENTITYTEMPLATE:      AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devlog")]
pub(crate) static POSTLOAD_CNT_APPEARANCERESOURCE:  AtomicU64 = AtomicU64::new(0);

// `#49`: worldStreamingSector::PostLoad candidato #2 (diverge de SL26, ver nota do const).
pub(crate) static POSTLOAD_CNT_WSSECTOR_C2:  AtomicU64 = AtomicU64::new(0);

pub(crate) static QUESTSYS_ONGAMERESTORED_CNT: AtomicU64 = AtomicU64::new(0);

// ===== `#49` parte 4b (2026-09-11): `worldStreamingSector::PostLoad` (`0x101c05360`), só observando, gate PRÓPRIO =====
// Layout da parte 4a (DATABASE, "layout do setor e do setup"): hash do path em `+0x30`; `nodeSetups` (DataBuffer) em `+0x40`, dados em
// `+0x00` e tamanho u32 em `+0x08`, elemento `0x90`; `nodes` (`DynArray<Handle<worldNode>>`) em `+0x70`, size `+0x7c` (2.5); no setup, `node*` em
// `+0x50` e índice de 19 bits em `+0x70` (2.5). Chama a original PRIMEIRO — o bind dela é quem grava `setup+0x50` — e só então lê. Roda em
// thread de job do streaming: só leitura de campo e log. Detalha os 3 primeiros setores e o `always_loaded_0` (critério: 5689 nós,
// 15072 setups e `setup[0].node == nodes[idx]`). O dump genérico de 2026-08-12 procurava DynArray às cegas; o layout agora é conhecido.
pub(crate) const SETOR_ALWAYS_LOADED_0_HASH: u64 = 0x9a88_5b98_7f8f_0678;

pub(crate) static WSSECTOR_ALVO_LOGS: AtomicU64 = AtomicU64::new(0);

// Parte 4c-1 (2026-09-11, censo só leitura): vptr estático do `worldStaticMeshNode` (construct no slot `0xe0` da vtable de classe; ver
// DATABASE). Na thread de job não há RTTI: a classe do nó sai da comparação do vptr rebased.
pub(crate) const WORLDSTATICMESHNODE_VPTR_VM: u64 = 0x1_07c5_f3c8;

pub(crate) const WORLDINSTANCEDDESTRUCTIBLEMESHNODE_VPTR_VM: u64 = 0x1_078c_eb28;

pub(crate) const WORLDCOLLISIONNODE_VPTR_VM: u64 = 0x1_078e_0d08;

pub(crate) const WORLDADVERTISEMENTNODE_VPTR_VM: u64 = 0x1_07c7_c8a8;

/// Classes com deleção própria; as outras vão pela genérica (4e-4b).
#[cfg(feature = "devtools")]
pub(crate) const VPTRS_COM_TRATAMENTO: [u64; 5] =
    [WORLDSTATICMESHNODE_VPTR_VM, crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM, WORLDINSTANCEDDESTRUCTIBLEMESHNODE_VPTR_VM, WORLDCOLLISIONNODE_VPTR_VM, WORLDADVERTISEMENTNODE_VPTR_VM];

#[cfg(feature = "devtools")]
pub(crate) static WSSECTOR_CENSO_LOGS: AtomicU64 = AtomicU64::new(0);

// Parte 4c-2 (2026-09-11): deleção de teste com `~/.bwms-hook-wssector-delete-teste` (lido 1x no instalador; a thread de job não abre
// arquivo). Regra pra `worldStaticMeshNode`: zerar o `scale` do setup. Cada regra confere a contagem de setups, o índice e o
// vptr do nó antes de gravar. Alvo do censo do boot 46: setor `0xbdf83cfce7456500`, 20 setups, static mesh no 18. Os outros 3 são controles
// que têm que recusar sem gravar.
#[cfg(feature = "devtools")]
pub(crate) static WSSECTOR_DELETE_TESTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static WSSECTOR_INST_TESTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static INST_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) const SETOR_DELECAO_TESTE_HASH: u64 = 0xbdf8_3cfc_e745_6500;

#[cfg(feature = "devtools")]
pub(crate) const DELECAO_TESTE: [(u64, usize, usize, &str); 4] = [
    (SETOR_DELECAO_TESTE_HASH, 20, 20, "indice-fora"),
    (SETOR_DELECAO_TESTE_HASH, 21, 18, "contagem-errada"),
    (SETOR_ALWAYS_LOADED_0_HASH, 15072, 0, "tipo-errado"),
    (SETOR_DELECAO_TESTE_HASH, 20, 18, "positivo"),
];

// `#80` parte 80b (2026-09-11): censo dos `worldPopulationSpawnerNode` (vptr `0x107c6da60`, RE da 80a) no mesmo hook, guardando o
// `globalNodeID` do setup (`+0x58`) — candidato a chave do `CommunitySystem::GetSpawner` (busca binária num array de u64; a instância do
// spawner usa `+0x80` no attach). A 80c chama o `GetSpawner` com esses ids em gameplay. Só leitura; até 256 ids distintos.
#[cfg(feature = "devtools")]
pub(crate) const WORLDPOPULATIONSPAWNERNODE_VPTR_VM: u64 = 0x1_07c6_da60;

#[cfg(feature = "devtools")]
pub(crate) static SPAWNER_IDS_CENSO: std::sync::Mutex<Vec<(u64, u64, u32)>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) static SPAWNER_CENSO_LOGS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_censo_spawners(this: *mut c_void, hash: u64) {
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups == 0 || setups > 200_000 || setups_ptr.is_null() || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let vptr = crate::rebase(WORLDPOPULATIONSPAWNERNODE_VPTR_VM) as u64;
    for i in 0..setups {
        let setup = setups_ptr.add(i * 0x90);
        let no = (setup.add(0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 || (no as *const u64).read_unaligned() != vptr {
            continue;
        }
        let id = (setup.add(0x58) as *const u64).read_unaligned();
        if let Ok(mut ids) = SPAWNER_IDS_CENSO.lock() {
            if ids.len() < 256 && !ids.iter().any(|(x, ..)| *x == id) {
                ids.push((id, hash, i as u32));
            }
        }
        if SPAWNER_CENSO_LOGS.fetch_add(1, Ordering::Relaxed) < 24 {
            crate::veredito_dev!("[c80-censo] setor={hash:#018x} setup={i} spawner-node globalNodeID={id:#018x}");
        }
    }
}

/// vptr VM da classe de node suportada pela deleção, pelo nome do `.xl`.
pub(crate) fn vptr_do_tipo_de_node(tipo: &str) -> Option<u64> {
    match tipo {
        "worldStaticMeshNode" => Some(WORLDSTATICMESHNODE_VPTR_VM),
        "worldInstancedMeshNode" => Some(crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM),
        "worldInstancedDestructibleMeshNode" => Some(WORLDINSTANCEDDESTRUCTIBLEMESHNODE_VPTR_VM),
        "worldCollisionNode" => Some(WORLDCOLLISIONNODE_VPTR_VM),
        "worldAdvertisementNode" => Some(WORLDADVERTISEMENTNODE_VPTR_VM),
        _ => None,
    }
}

// Instâncias de um `worldInstancedMeshNode` na 2.5, nó cozido (`worldCookedNode`; RE `proofs/20261027-no-instanciado-25-offline.log`, o leitor do
// motor `0x101b1debc`): a faixa do setup é `cookedRanges[instanceIndex]` (`DynArray<{start u16, count u16}>` em `+0x80`; `instanceIndex` u32 no
// setup `+0x80`) dentro do `DataBuffer` do nó em `+0x38` (dados `+0x00`, tamanho em bytes `+0x08`); elemento de 0x30 B com o scale em `+0x20`. Com
// `transforms` (`+0x60`) não vazio o motor usa ele no lugar do buffer, e o BWMS recusa. O scale de todas as instâncias da faixa tem que ser plausível
// antes de liberar a escrita.
pub(crate) const INSTANCIA_TAM: usize = 0x30;
pub(crate) const NO_INSTANCIADO_BUFFER: usize = 0x38;
pub(crate) const NO_INSTANCIADO_TRANSFORMS: usize = 0x60;
pub(crate) const NO_INSTANCIADO_DADOS: usize = 0x70;
pub(crate) const NO_INSTANCIADO_FAIXAS: usize = 0x80;
pub(crate) const SETUP_INDICE_DE_INSTANCIA: usize = 0x80;
/// Elemento de `cookedInstances` (0x40 B): `mesh` (caminho, 8 B) e `meshAppearance` (CName); é onde a 2.5 guarda o que era campo do nó.
pub(crate) const DADO_COZIDO_TAM: usize = 0x40;
pub(crate) const DADO_COZIDO_MESH: usize = 0x20;
pub(crate) const DADO_COZIDO_APARENCIA: usize = 0x28;

/// `[sdb+0x38]` é tamanho em BYTES: nos boots 62–75 todo tamanho logado é múltiplo de 0x30 (32448 = 676×0x30, 29136 = 607×0x30).
/// A faixa inteira tem que caber; o `is_readable` que vem depois só confere 512 B.
pub(crate) fn instancias_cabem(inicio: usize, n: usize, tam_bytes: usize) -> bool {
    (inicio + n) * INSTANCIA_TAM <= tam_bytes
}

pub(crate) const INSTANCIA_SCALE_OFF: usize = 0x20;

pub(crate) static INSTANCIAS_LOGADAS: AtomicU64 = AtomicU64::new(0);

pub(crate) enum PlanoDelecao {
    /// Static mesh: `scale` do setup `i` em 0.
    ScaleSetup(usize),
    /// Instanced mesh: `scale` em 0 nas instâncias `faixa` (índices absolutos no buffer) do nó com instâncias `[inicio, inicio+n)`.
    Instancias { dados: *mut u8, inicio: usize, n: usize, faixa: Vec<usize>, vizinho: Option<usize> },
    /// Instanced destructible: `position.Z` em -2000 nas instâncias `faixa` do nó com instâncias `[inicio, inicio+n)`.
    Destrutiveis { dados: *mut u8, inicio: usize, n: usize, faixa: Vec<usize> },
    /// Colisão: `Position.z` em -2000 m nos atores `faixa` (índices do nó) dos `n` atores a partir de `atores`.
    Atores { atores: *mut u8, n: usize, faixa: Vec<usize> },
    /// Colisão: as `shapes` (índices na lista de shapes) do nó passam a usar o preset `No Collision`, achado nos presets do nó ou
    /// acrescentado no fim deles (o buffer do nó é realocado pelo alocador dele).
    ShapesSemColisao { no: *mut u8, shapes: Vec<usize> },
    /// Troca de nó: setup `indice` apontando pro nó `doador` (o [`no_fantasma`]); `vizinho` é conferido byte a byte.
    Doador { indice: usize, doador: u64, vizinho: Option<usize> },
    /// Genérica: setup `indice` em Z -2000, `globalNodeID` 0 e `instanceIndex` -1, apontando pro `doador`; `vizinho` conferido byte a byte.
    Generica { indice: usize, doador: u64, vizinho: Option<usize> },
    /// Mutação 4f-1: `valores` (f32) gravados a partir de `setup+off` (position `0x00`, orientation `0x10`, scale `0x20`); `vizinho` conferido byte a byte.
    Setup { indice: usize, off: usize, valores: Vec<f32>, vizinho: Option<usize> },
    /// Mutação 4f-2 em sub-nó: `valores` (f32) no elemento `j` (índice absoluto) de um buffer de `tam` B por elemento, a partir de `off`; os outros
    /// elementos `[inicio, inicio+n)` conferidos byte a byte.
    SubNo { dados: *mut u8, tam: usize, j: usize, off: usize, valores: Vec<f32>, inicio: usize, n: usize },
    /// Mutação 4f-2c: o mesmo, com `valores` i32 (posição em ponto fixo do ator de colisão, 1/131072 m).
    SubNoI32 { dados: *mut u8, tam: usize, j: usize, off: usize, valores: Vec<i32>, inicio: usize, n: usize },
    /// Mutação 4f-3 num campo do nó: `valor` gravado (ou somado, no `proxy`) em `no+off`, com `largura` 8 ou 4 bytes.
    Campo { no: *mut u8, off: usize, largura: usize, valor: u64, soma: bool },
    /// Mutação 4f-4 no destrutível: o elemento `j` guarda o transform da instância relativo ao setup. Na aplicação o elemento atual vira mundo
    /// (`local * setup`), a posição (`pos`) ou a orientação (`rot`) do mundo é trocada e o resultado volta pra local; a componente não pedida fica
    /// como está.
    SubNoLocal { dados: *mut u8, j: usize, inicio: usize, n: usize, setup: Xf, pos: Option<[f32; 3]>, rot: Option<[f32; 4]> },
}

// Parte 4e-2 (2026-09-12): buffer do `worldInstancedDestructibleMeshNode` (RE da 4e) — `cookedInstanceTransforms` em `+0x60` com o mesmo layout de buffer
// do instanced mesh; elemento de 0x20 B com `position.Z` (f32) em `+0x08`. Antes da escrita, todo Z do nó tem que ser finito e dentro de ±20000
// (controle do layout).
pub(crate) const DESTRUTIVEL_BUFFER_OFF: usize = 0x60;

pub(crate) const DESTRUTIVEL_TAM: usize = 0x20;

pub(crate) const DESTRUTIVEL_Z_OFF: usize = 0x08;

pub(crate) const DESTRUTIVEL_Z_DELETADO: f32 = -2000.0;

#[cfg(feature = "devtools")]
pub(crate) static DEST_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe fn destrutivel_z(dados: *const u8, j: usize) -> f32 {
    (dados.add(j * DESTRUTIVEL_TAM + DESTRUTIVEL_Z_OFF) as *const f32).read_unaligned()
}

/// `(dados, início, n)` do buffer de um nó destrutível, conferido.
pub(crate) unsafe fn destrutiveis_do_no(no: *const u8) -> Result<(*mut u8, usize, usize), String> {
    if !crate::memoria::is_readable(no as *const c_void, DESTRUTIVEL_BUFFER_OFF + 0x18) {
        return Err("nó ilegível".to_string());
    }
    let buf = no.add(DESTRUTIVEL_BUFFER_OFF);
    let sdb = (buf as *const u64).read_unaligned() as *const u8;
    let inicio = (buf.add(0x10) as *const u32).read_unaligned() as usize;
    let n = (buf.add(0x14) as *const u32).read_unaligned() as usize;
    if sdb.is_null() || !crate::memoria::is_readable(sdb as *const c_void, 0x40) {
        return Err("shared buffer ilegível".to_string());
    }
    let dados = (sdb.add(0x30) as *const u64).read_unaligned() as *mut u8;
    let tam = (sdb.add(0x38) as *const u32).read_unaligned() as usize;
    let cabe = (inicio + n) * DESTRUTIVEL_TAM <= tam || inicio + n <= tam;
    if n == 0 || dados.is_null() || !cabe || !crate::memoria::is_readable(dados.add(inicio * DESTRUTIVEL_TAM) as *const c_void, n * DESTRUTIVEL_TAM) {
        return Err(format!("faixa de destrutíveis inválida (início {inicio}, n {n}, tamanho {tam})"));
    }
    if !(inicio..inicio + n).all(|j| {
        let z = destrutivel_z(dados, j);
        z.is_finite() && z.abs() < 20000.0
    }) {
        return Err(format!("Z implausível nas instâncias (início {inicio}, n {n}): layout não confirmado"));
    }
    Ok((dados, inicio, n))
}

// Atores do `worldCollisionNode` na 2.5 (`proofs/20261027-colisao-25-offline.log`: slot `0x30` `0x101af6ca8` e o leitor `0x101af6d70`): os dados
// compilados ficam no `DataBuffer` em `+0x58` (dados `+0x00`, bytes `+0x08`); cada lista é um offset u32 dentro dele, que o slot `0x30` grava a partir
// das contagens u16 (atores: offset `+0x34`, contagem `+0x88`). Ator de 0x30 B com `Position.z` i32 (FixedPoint, 1/131072 m) `@0x08`. Lista vazia (o
// slot ainda não rodou) ou que não cabe no buffer é recusada.
pub(crate) const COLISAO_DADOS_OFF: usize = 0x58;

pub(crate) const COLISAO_ATORES_OFF: usize = 0x34;

pub(crate) const COLISAO_NUM_ATORES_OFF: usize = 0x88;

pub(crate) const ATOR_TAM: usize = 0x30;

pub(crate) const ATOR_Z_OFF: usize = 0x08;

pub(crate) const ATOR_Z_DELETADO: i32 = -2000 * 131072;

/// As 9 listas compactas do `worldCollisionNode` da 2.5: offset u32 de cada uma no `DataBuffer` (`+0x34..+0x54`) e a contagem u16 (`+0x88..+0x98`), na
/// ordem atores, shapes, posições, rotações, escalas, materiais, presets, índices de material e bytes (`proofs/20261027-colisao-25-offline.log`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ListasDeColisao {
    pub(crate) offsets: [u32; 9],
    pub(crate) contagens: [u16; 9],
}

pub(crate) const COLISAO_LISTA_PRESETS: usize = 6;

/// Preset de física sem colisão (`engine\physics\collision_presets.json` da 2.5).
pub(crate) const PRESET_SEM_COLISAO: &str = "No Collision";

/// `presetIndex` (u8) da shape de colisão de 0x20 B.
pub(crate) const SHAPE_PRESET_OFF: usize = 0x18;

/// Tamanho do elemento de cada lista; o da última (bytes soltos) é 1.
pub(crate) const COLISAO_ELEMENTO: [usize; 9] = [0x30, 0x20, 0xc, 0x10, 0xc, 8, 8, 4, 1];

/// Índice do preset `nome` (CName) já presente nas listas do nó.
pub(crate) fn indice_do_preset(dados: &[u8], l: &ListasDeColisao, nome: u64) -> Option<u8> {
    let ini = l.offsets[COLISAO_LISTA_PRESETS] as usize;
    let presets = dados.get(ini..ini.checked_add(l.contagens[COLISAO_LISTA_PRESETS] as usize * 8)?)?;
    let i = presets.chunks_exact(8).position(|p| p == nome.to_le_bytes())?;
    u8::try_from(i).ok()
}

/// Fim (exclusivo) da lista `k` no buffer.
fn fim_da_lista(l: &ListasDeColisao, k: usize) -> usize {
    l.offsets[k] as usize + l.contagens[k] as usize * COLISAO_ELEMENTO[k]
}

/// O buffer do nó com `nome` acrescentado no fim dos presets, as listas novas e o índice do preset novo.
pub(crate) fn buffer_com_preset(dados: &[u8], l: &ListasDeColisao, nome: u64) -> Option<(Vec<u8>, ListasDeColisao, u8)> {
    if (0..9).any(|k| fim_da_lista(l, k) > dados.len()) {
        return None;
    }
    // `presetIndex` da shape é u8: o 256º preset não teria índice.
    let indice = u8::try_from(l.contagens[COLISAO_LISTA_PRESETS]).ok().filter(|&i| i < u8::MAX)?;
    let ponto = fim_da_lista(l, COLISAO_LISTA_PRESETS);
    let mut novo = Vec::with_capacity(dados.len() + 8);
    novo.extend_from_slice(&dados[..ponto]);
    novo.extend_from_slice(&nome.to_le_bytes());
    novo.extend_from_slice(&dados[ponto..]);
    let mut listas = *l;
    listas.contagens[COLISAO_LISTA_PRESETS] += 1;
    for k in COLISAO_LISTA_PRESETS + 1..9 {
        listas.offsets[k] += 8;
    }
    Some((novo, listas, indice))
}

/// `(dados, bytes, listas)` do nó de colisão, conferidos.
pub(crate) unsafe fn listas_de_colisao(no: *const u8) -> Result<(*mut u8, usize, ListasDeColisao), String> {
    if no.is_null() || !crate::memoria::is_readable(no as *const c_void, 0x9a) {
        return Err("nó ilegível".to_string());
    }
    let dados = (no.add(COLISAO_DADOS_OFF) as *const u64).read_unaligned() as *mut u8;
    let bytes = (no.add(COLISAO_DADOS_OFF + 8) as *const u32).read_unaligned() as usize;
    if dados.is_null() || bytes == 0 {
        return Err("buffer do nó vazio: o slot 0x30 ainda não rodou".to_string());
    }
    let mut l = ListasDeColisao { offsets: [0; 9], contagens: [0; 9] };
    for k in 0..9 {
        l.offsets[k] = (no.add(COLISAO_ATORES_OFF + 4 * k) as *const u32).read_unaligned();
        l.contagens[k] = (no.add(COLISAO_NUM_ATORES_OFF + 2 * k) as *const u16).read_unaligned();
    }
    if (0..9).any(|k| fim_da_lista(&l, k) > bytes) || !crate::memoria::is_readable(dados as *const c_void, bytes) {
        return Err(format!("listas do nó não cabem no buffer de {bytes:#x} B"));
    }
    Ok((dados, bytes, l))
}

/// `<atores esperados> <ator> <shape> [...]` lido e conferido contra `atores` e as shapes de cada ator: os pares `(ator, shape)` na ordem.
/// `cabe(ator, shape)` é a conferência extra da shape contra a lista do nó.
fn pares_do_subshape(
    valores: &str,
    atores: usize,
    shapes_do_ator: impl Fn(usize) -> usize,
    cabe: impl Fn(usize, usize) -> bool,
) -> Result<Vec<(usize, usize)>, String> {
    let campos = valores.split_whitespace().map(|c| c.parse::<usize>()).collect::<Result<Vec<usize>, _>>().map_err(|_| "subshape com número inválido".to_string())?;
    let (&esperados, pares) = campos.split_first().ok_or_else(|| "subshape sem contagem".to_string())?;
    if esperados != atores {
        return Err(format!("atores (nó tem {atores}, .xl espera {esperados})"));
    }
    if pares.is_empty() || pares.len() % 2 != 0 {
        return Err("subshape sem pares [ator, shape]".to_string());
    }
    pares
        .chunks_exact(2)
        .map(|par| {
            let (a, s) = (par[0], par[1]);
            if a >= atores {
                return Err(format!("ator {a} >= {atores}"));
            }
            let n = shapes_do_ator(a);
            if s >= n || !cabe(a, s) {
                return Err(format!("shape {s} do ator {a} fora das {n} dele"));
            }
            Ok((a, s))
        })
        .collect()
}

/// Shapes (índice na lista de shapes do nó) de um `subshape <atores esperados> <ator> <shape> ...`.
pub(crate) unsafe fn shapes_do_subshape(valores: &str, dados: *const u8, l: &ListasDeColisao) -> Result<Vec<usize>, String> {
    let ator = |a: usize| dados.add(l.offsets[0] as usize + a * COLISAO_ELEMENTO[0]);
    let atores = l.contagens[0] as usize;
    let shapes_do_no = l.contagens[1] as usize;
    // `shapeStartIndex` u16 `+0x20` e `numShapes` u16 `+0x22` do ator; a shape que passa da lista do nó conta como fora do ator.
    let inicio = |a: usize| (ator(a).add(0x20) as *const u16).read_unaligned() as usize;
    let numero = |a: usize| (ator(a).add(0x22) as *const u16).read_unaligned() as usize;
    let pares = pares_do_subshape(valores, atores, numero, |a, s| inicio(a) + s < shapes_do_no)?;
    Ok(pares.into_iter().map(|(a, s)| inicio(a) + s).collect())
}

/// Atores que um `subshape <atores esperados> <ator> <shape> ...` desce em colisão cozida.
pub(crate) fn atores_do_subshape_cozido(valores: &str, atores: usize, shapes_do_ator: impl Fn(usize) -> usize) -> Result<Vec<usize>, String> {
    let pares = pares_do_subshape(valores, atores, &shapes_do_ator, |_, _| true)?;
    // O nó cozido não tem preset para trocar por shape: só o ator de 1 shape desce inteiro.
    let mut descem: Vec<usize> = Vec::new();
    for (a, _) in pares {
        if shapes_do_ator(a) == 1 && !descem.contains(&a) {
            descem.push(a);
        }
    }
    Ok(descem)
}

/// O nó invisível que toma o lugar do nó apagado, criado uma vez na thread do jogo; o `PostLoad`, em thread de job, só lê o ponteiro. Os setores da
/// 2.5 não têm `worldStaticMeshNode` para servir de doador (0 de 91 na amostra).
static NO_FANTASMA: AtomicU64 = AtomicU64::new(0);

/// Cria o nó invisível (thread do jogo, no `register_all`).
pub(crate) unsafe fn criar_no_fantasma(reg: &crate::rtti::Registry) {
    if NO_FANTASMA.load(Ordering::Acquire) != 0 {
        return;
    }
    let no = crate::rtti::new_object(reg, "worldStaticMeshNode");
    if no.is_null() {
        return;
    }
    let Some((off, _, no_holder)) = crate::rtti::resolve_prop_in_class(crate::rtti::class_of(no), "isVisibleInGame") else {
        crate::log_dev!("[axl-streaming-xl] nó invisível: worldStaticMeshNode sem isVisibleInGame; fica sem doador");
        return;
    };
    let campo = crate::rtti::field_ptr(no, off, no_holder) as *mut u8;
    if campo.is_null() {
        return;
    }
    campo.write(0);
    NO_FANTASMA.store(no as u64, Ordering::Release);
}

/// O nó invisível, ou o erro de quando ainda não existe.
pub(crate) fn no_fantasma() -> Result<u64, String> {
    match NO_FANTASMA.load(Ordering::Acquire) {
        0 => Err("nó invisível de deleção ainda não criado".to_string()),
        no => Ok(no),
    }
}

/// Vetores `B5|diag|*` de `B5-vetores.txt`. No harness o `rebase` dá 0: o vptr esperado de nó com vtable conhecida é 0.
#[cfg(test)]
mod vetores_b5 {
    use super::*;

    fn str_err<T>(r: Result<T, String>) -> String {
        r.err().unwrap_or_default()
    }

    /// Buffer com as 9 listas contíguas, cada elemento preenchido com o número da lista + 1.
    fn sintetico(contagens: [u16; 9]) -> (Vec<u8>, ListasDeColisao) {
        let mut dados = Vec::new();
        let mut offsets = [0u32; 9];
        for k in 0..9 {
            offsets[k] = dados.len() as u32;
            dados.extend(std::iter::repeat(k as u8 + 1).take(contagens[k] as usize * COLISAO_ELEMENTO[k]));
        }
        (dados, ListasDeColisao { offsets, contagens })
    }

    #[test]
    fn preset_sem_colisao() {
        assert_eq!(cname(PRESET_SEM_COLISAO), 0xd50f938bf00ea64e);
        let nome = cname(PRESET_SEM_COLISAO);
        let (dados, l) = sintetico([2, 3, 1, 1, 1, 2, 2, 3, 5]);
        assert_eq!((l.offsets, dados.len()), ([0, 96, 192, 204, 220, 232, 248, 264, 276], 281));
        let (novo, n, idx) = buffer_com_preset(&dados, &l, nome).expect("preset novo");
        assert_eq!((idx, n.offsets, n.contagens), (2, [0, 96, 192, 204, 220, 232, 248, 272, 284], [2, 3, 1, 1, 1, 2, 3, 3, 5]));
        let hex: String = novo.iter().map(|b| format!("{b:02x}")).collect();
        let esperado = "01".repeat(96) + &"02".repeat(96) + &"03".repeat(12) + &"04".repeat(16) + &"05".repeat(12) + &"06".repeat(16) + &"07".repeat(16) + "4ea60ef08b930fd5" + &"08".repeat(12) + &"09".repeat(5);
        assert_eq!(hex, esperado);
        assert_eq!((indice_do_preset(&novo, &n, nome), indice_do_preset(&dados, &l, nome)), (Some(2), None));

        let mut com_nome = dados.clone();
        com_nome[256..264].copy_from_slice(&nome.to_le_bytes());
        assert_eq!((indice_do_preset(&com_nome, &l, nome), indice_do_preset(&com_nome, &l, 7)), (Some(1), None));

        let (um, l1) = sintetico([1; 9]);
        assert!(buffer_com_preset(&um[..um.len() - 1], &l1, nome).is_none(), "buffer 1 B curto");
        let (d255, l255) = sintetico([1, 1, 1, 1, 1, 1, 255, 1, 1]);
        assert!(buffer_com_preset(&d255, &l255, nome).is_none(), "255 presets");
        let (d254, l254) = sintetico([1, 1, 1, 1, 1, 1, 254, 1, 1]);
        assert_eq!(buffer_com_preset(&d254, &l254, nome).map(|(_, l, i)| (i, l.contagens[COLISAO_LISTA_PRESETS])), Some((254, 255)));
    }

    /// Nó de colisão de 0xa0 B com o `DataBuffer` em `+0x58` e as listas.
    fn no_de_colisao(dados: &[u8], bytes: u32, l: &ListasDeColisao) -> [u8; 0xa0] {
        let mut no = [0u8; 0xa0];
        no[COLISAO_DADOS_OFF..COLISAO_DADOS_OFF + 8].copy_from_slice(&(dados.as_ptr() as u64).to_le_bytes());
        no[COLISAO_DADOS_OFF + 8..COLISAO_DADOS_OFF + 12].copy_from_slice(&bytes.to_le_bytes());
        for k in 0..9 {
            no[COLISAO_ATORES_OFF + 4 * k..COLISAO_ATORES_OFF + 4 * k + 4].copy_from_slice(&l.offsets[k].to_le_bytes());
            no[COLISAO_NUM_ATORES_OFF + 2 * k..COLISAO_NUM_ATORES_OFF + 2 * k + 2].copy_from_slice(&l.contagens[k].to_le_bytes());
        }
        no
    }

    #[test]
    fn listas() {
        let (dados, l) = sintetico([2, 3, 1, 1, 1, 2, 2, 3, 5]);
        let r = unsafe { listas_de_colisao(no_de_colisao(&dados, 281, &l).as_ptr()) }.map(|(d, b, ls)| (d as usize == dados.as_ptr() as usize, b, ls));
        assert_eq!(r, Ok((true, 281, l)));
        assert_eq!(str_err(unsafe { listas_de_colisao(no_de_colisao(&dados, 280, &l).as_ptr()) }), "listas do nó não cabem no buffer de 0x118 B");
        assert_eq!(str_err(unsafe { listas_de_colisao(no_de_colisao(&dados, 0, &l).as_ptr()) }), "buffer do nó vazio: o slot 0x30 ainda não rodou");
        let mut nulo = no_de_colisao(&dados, 281, &l);
        nulo[COLISAO_DADOS_OFF..COLISAO_DADOS_OFF + 8].fill(0);
        assert_eq!(str_err(unsafe { listas_de_colisao(nulo.as_ptr()) }), "buffer do nó vazio: o slot 0x30 ainda não rodou");
    }

    #[test]
    fn subshape() {
        // Atores `[início 0, n 1]` e `[início 1, n 2]`; o nó tem 3 shapes.
        let mut dados = vec![0u8; 2 * 0x30];
        dados[0x22..0x24].copy_from_slice(&1u16.to_le_bytes());
        dados[0x30 + 0x20..0x30 + 0x22].copy_from_slice(&1u16.to_le_bytes());
        dados[0x30 + 0x22..0x30 + 0x24].copy_from_slice(&2u16.to_le_bytes());
        let mut l = ListasDeColisao { offsets: [0; 9], contagens: [0; 9] };
        l.contagens[0] = 2;
        l.contagens[1] = 3;
        let s = |v: &str| unsafe { shapes_do_subshape(v, dados.as_ptr(), &l) };
        let ok = |v: Vec<usize>| -> Result<Vec<usize>, String> { Ok(v) };
        let err = |m: &str| -> Result<Vec<usize>, String> { Err(m.to_string()) };
        assert_eq!(s("2 0 0"), ok(vec![0]));
        assert_eq!(s("2 1 0 1 1"), ok(vec![1, 2]));
        assert_eq!(s("2 1 2"), err("shape 2 do ator 1 fora das 2 dele"));
        assert_eq!(s("2 2 0"), err("ator 2 >= 2"));
        assert_eq!(s("3 0 0"), err("atores (nó tem 2, .xl espera 3)"));
        assert_eq!(s("2 0"), err("subshape sem pares [ator, shape]"));
        assert_eq!(s("2"), err("subshape sem pares [ator, shape]"));
        assert_eq!(s("2 a b"), err("subshape com número inválido"));
        assert_eq!(s("2 0 0 1 1 0 0"), ok(vec![0, 2, 0]));
        dados[0x30 + 0x20..0x30 + 0x22].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(unsafe { shapes_do_subshape("2 1 1", dados.as_ptr(), &l) }, err("shape 1 do ator 1 fora das 2 dele"));

        let c = |v: &str| atores_do_subshape_cozido(v, 4, |a| [1, 3, 1, 0][a]);
        assert_eq!(c("4 0 0 1 2 2 0"), ok(vec![0, 2]));
        assert_eq!(c("4 0 0 0 0"), ok(vec![0]));
        assert_eq!(c("4 1 0"), ok(vec![]));
        assert_eq!(c("4 2 0 0 0"), ok(vec![2, 0]));
        assert_eq!(c("4 0 1"), err("shape 1 do ator 0 fora das 1 dele"));
        assert_eq!(c("4 4 0"), err("ator 4 >= 4"));
        assert_eq!(c("3 0 0"), err("atores (nó tem 4, .xl espera 3)"));
        assert_eq!(c("4 0"), err("subshape sem pares [ator, shape]"));
        assert_eq!(c("4"), err("subshape sem pares [ator, shape]"));
        assert_eq!(c(""), err("subshape sem contagem"));
        assert_eq!(c("4 x 0"), err("subshape com número inválido"));
        assert_eq!(c("4 3 0"), err("shape 0 do ator 3 fora das 0 dele"));
        assert_eq!(c("  4   0   0  "), ok(vec![0]));
    }

    #[test]
    fn no_cozido() {
        let tipos = ["worldCookedStaticMeshNode", "worldCookedGenericProxyMeshNode", "worldCookedStaticDecalNode", "worldCookedAcousticSectorNode", "worldStaticMeshNode", "worldCookedCollisionNode"];
        let mut tabela = Vec::new();
        for t in tipos {
            for op in ["res", "app", "proxy", "rec", "", "subdel"] {
                if let Some(c) = campo_do_item_cozido(t, op) {
                    tabela.push((t, op, c));
                }
            }
        }
        assert_eq!(
            tabela,
            vec![
                ("worldCookedStaticMeshNode", "res", (33, 0, 8)),
                ("worldCookedStaticMeshNode", "app", (33, 8, 8)),
                ("worldCookedGenericProxyMeshNode", "res", (41, 0, 8)),
                ("worldCookedGenericProxyMeshNode", "app", (41, 8, 8)),
                ("worldCookedGenericProxyMeshNode", "proxy", (41, 33, 4)),
                ("worldCookedStaticDecalNode", "res", (36, 0, 8)),
                ("worldCookedAcousticSectorNode", "res", (21, 0, 8)),
            ]
        );

        let itens = vec![0u8; 3 * 0x21];
        let mut no = [0u8; 0x58];
        no[NO_COZIDO_DADOS..NO_COZIDO_DADOS + 8].copy_from_slice(&(itens.as_ptr() as u64).to_le_bytes());
        no[NO_COZIDO_BYTES..NO_COZIDO_BYTES + 4].copy_from_slice(&(itens.len() as u32).to_le_bytes());
        no[NO_COZIDO_PASSO..NO_COZIDO_PASSO + 4].copy_from_slice(&0x21u32.to_le_bytes());
        let item = |no: &[u8; 0x58], idx: u32, passo: usize| {
            let mut setup = [0u8; 0x90];
            setup[SETUP_INDICE_DE_INSTANCIA..SETUP_INDICE_DE_INSTANCIA + 4].copy_from_slice(&idx.to_le_bytes());
            unsafe { item_cozido_do_setup(no.as_ptr(), setup.as_ptr(), passo) }.map(|p| p as usize - itens.as_ptr() as usize)
        };
        assert_eq!(item(&no, 0, 0x21), Ok(0));
        assert_eq!(item(&no, 2, 0x21), Ok(0x42));
        assert_eq!(item(&no, 3, 0x21), Err("instanceIndex 3 fora dos 3 item(ns) do nó cozido".to_string()));
        assert_eq!(item(&no, 1, 0x29), Err("passo do item cozido 0x21, o tipo espera 0x29".to_string()));
        assert_eq!(item(&no, u32::MAX, 0x21), Err("instanceIndex 4294967295 fora dos 3 item(ns) do nó cozido".to_string()));
        no[NO_COZIDO_DADOS..NO_COZIDO_DADOS + 8].fill(0);
        assert_eq!(item(&no, 0, 0x21), Err("instanceIndex 0 fora dos 3 item(ns) do nó cozido".to_string()));
    }

    #[test]
    fn constantes_e_no_fantasma() {
        assert_eq!((NO_COZIDO_DADOS, NO_COZIDO_BYTES, NO_COZIDO_PASSO, COLISAO_COZIDA_DADOS_OFF), (0x40, 0x4c, 0x50, 0x58));
        assert_eq!((ATOR_Z_DELETADO, DESTRUTIVEL_Z_DELETADO), (-262144000, -2000.0));
        assert_eq!((COLISAO_ELEMENTO, SHAPE_PRESET_OFF), ([0x30, 0x20, 0xc, 0x10, 0xc, 8, 8, 4, 1], 0x18));
        assert_eq!(no_fantasma(), Err("nó invisível de deleção ainda não criado".to_string()));
    }

    /// O plano como o vetor o escreve, com os ponteiros relativos aos buffers do teste.
    fn texto(p: &PlanoDelecao, base: &[(&str, usize)]) -> String {
        let rel = |d: *mut u8| {
            base.iter().filter_map(|(n, b)| (d as usize).checked_sub(*b).map(|o| (n, o))).min_by_key(|(_, o)| *o).map(|(n, o)| format!("{n}+{o:#x}")).unwrap_or_default()
        };
        match p {
            PlanoDelecao::ScaleSetup(i) => format!("ScaleSetup({i})"),
            PlanoDelecao::Setup { indice, off, valores, vizinho } => format!("Setup{{indice={indice}, off={off:#x}, valores={valores:?}, vizinho={vizinho:?}}}"),
            PlanoDelecao::Instancias { dados, inicio, n, faixa, vizinho } => format!("Instancias{{dados={}, inicio={inicio}, n={n}, faixa={faixa:?}, vizinho={vizinho:?}}}", rel(*dados)),
            PlanoDelecao::Atores { atores, n, faixa } => format!("Atores{{atores={}, n={n}, faixa={faixa:?}}}", rel(*atores)),
            PlanoDelecao::SubNo { dados, tam, j, off, valores, inicio, n } => {
                format!("SubNo{{dados={}, tam={tam:#x}, j={j}, off={off:#x}, valores={valores:?}, inicio={inicio}, n={n}}}", rel(*dados))
            }
            PlanoDelecao::SubNoI32 { dados, tam, j, off, valores, inicio, n } => {
                format!("SubNoI32{{dados={}, tam={tam:#x}, j={j}, off={off:#x}, valores={valores:?}, inicio={inicio}, n={n}}}", rel(*dados))
            }
            _ => "outro".to_string(),
        }
    }

    /// Setor de 3 setups: 0 static mesh, 1 instanced mesh (instâncias 1..4 de 5 no buffer), 2 colisão (2 atores em `+0x10` dos dados).
    #[test]
    fn planejar() {
        let mut inst = vec![0u8; 5 * INSTANCIA_TAM];
        for e in 0..5 {
            inst[e * INSTANCIA_TAM + 0x1c..e * INSTANCIA_TAM + 0x20].copy_from_slice(&1f32.to_le_bytes());
            for k in 0..3 {
                inst[e * INSTANCIA_TAM + 0x20 + 4 * k..e * INSTANCIA_TAM + 0x24 + 4 * k].copy_from_slice(&1f32.to_le_bytes());
            }
        }
        let faixas = [1u16, 3];
        let mut no_inst = vec![0u8; 0x98];
        no_inst[NO_INSTANCIADO_BUFFER..NO_INSTANCIADO_BUFFER + 8].copy_from_slice(&(inst.as_ptr() as u64).to_le_bytes());
        no_inst[NO_INSTANCIADO_BUFFER + 8..NO_INSTANCIADO_BUFFER + 12].copy_from_slice(&(inst.len() as u32).to_le_bytes());
        no_inst[NO_INSTANCIADO_FAIXAS..NO_INSTANCIADO_FAIXAS + 8].copy_from_slice(&(faixas.as_ptr() as u64).to_le_bytes());
        no_inst[NO_INSTANCIADO_FAIXAS + 0x0c..NO_INSTANCIADO_FAIXAS + 0x10].copy_from_slice(&1u32.to_le_bytes());

        let mut atores = vec![0u8; 0x10 + 2 * ATOR_TAM];
        for a in 0..2 {
            atores[0x10 + a * ATOR_TAM + 0x1c..0x10 + a * ATOR_TAM + 0x20].copy_from_slice(&1f32.to_le_bytes());
        }
        let mut no_col = vec![0u8; 0xa0];
        no_col[COLISAO_DADOS_OFF..COLISAO_DADOS_OFF + 8].copy_from_slice(&(atores.as_ptr() as u64).to_le_bytes());
        no_col[COLISAO_DADOS_OFF + 8..COLISAO_DADOS_OFF + 12].copy_from_slice(&(atores.len() as u32).to_le_bytes());
        no_col[COLISAO_ATORES_OFF..COLISAO_ATORES_OFF + 4].copy_from_slice(&0x10u32.to_le_bytes());
        no_col[COLISAO_ATORES_OFF + 4..COLISAO_ATORES_OFF + 8].copy_from_slice(&0x70u32.to_le_bytes());
        no_col[COLISAO_NUM_ATORES_OFF..COLISAO_NUM_ATORES_OFF + 2].copy_from_slice(&2u16.to_le_bytes());

        let mut no_static = vec![0u8; 0x60];
        let mut setups = vec![0u8; 3 * 0x90];
        for (i, no) in [no_static.as_ptr(), no_inst.as_ptr(), no_col.as_ptr()].into_iter().enumerate() {
            setups[i * 0x90 + 0x50..i * 0x90 + 0x58].copy_from_slice(&(no as u64).to_le_bytes());
        }
        let base = [("inst", inst.as_ptr() as usize), ("atores", atores.as_ptr() as usize)];
        let mut plano = |esperado: usize, indice: usize, tipo: &str, op: &str, valores: &str, setups_n: usize, legivel: bool| {
            let d = DelecaoDeNode { modulo: "m".into(), setor: 1, esperado, indice, tipo: tipo.into(), op: op.into(), valores: valores.into() };
            match unsafe { planejar_delecao(&d, setups.as_mut_ptr(), setups_n, legivel) } {
                Ok(p) => texto(&p, &base),
                Err(e) => format!("Err({e:?})"),
            }
        };
        let casos: &[(usize, usize, &str, &str, &str, &str)] = &[
            (4, 0, "worldStaticMeshNode", "", "", r#"Err("contagem (setor tem 3, .xl espera 4)")"#),
            (3, 3, "worldStaticMeshNode", "", "", r#"Err("índice 3 >= 3")"#),
            (3, 0, "worldStaticMeshNode", "", "", "ScaleSetup(0)"),
            (3, 0, "worldCookedStaticMeshNode", "", "", r#"Err("índice 0 não é worldCookedStaticMeshNode (vptr 0x0, classe None)")"#),
            (3, 0, "worldStaticMeshNode", "pos", "1 2 3 1", "Setup{indice=0, off=0x0, valores=[1.0, 2.0, 3.0, 1.0], vizinho=Some(1)}"),
            (3, 0, "worldStaticMeshNode", "rot", "0 0 0 1", "Setup{indice=0, off=0x10, valores=[0.0, 0.0, 0.0, 1.0], vizinho=Some(1)}"),
            (3, 0, "worldStaticMeshNode", "scale", "2 2 2", "Setup{indice=0, off=0x20, valores=[2.0, 2.0, 2.0], vizinho=Some(1)}"),
            (3, 2, "worldCollisionNode", "pos", "1 2 3 1", "Setup{indice=2, off=0x0, valores=[1.0, 2.0, 3.0, 1.0], vizinho=Some(1)}"),
            (3, 0, "worldStaticMeshNode", "pos", "1 2 3", r#"Err("pos com 3 valor(es), espera 4")"#),
            (3, 0, "worldStaticMeshNode", "scale", "1 NaN 1", r#"Err("scale com 3 valor(es), espera 3")"#),
            (3, 0, "worldStaticMeshNode", "scale", "1 x 1", r#"Err("scale com valor inválido")"#),
            (3, 0, "worldStaticMeshNode", "foo", "", r#"Err("op foo sem suporte")"#),
            (3, 1, "worldInstancedMeshNode", "", "", "Instancias{dados=inst+0x0, inicio=1, n=3, faixa=[1, 2, 3], vizinho=Some(4)}"),
            (3, 1, "worldInstancedMeshNode", "subdel", "3 0 2", "Instancias{dados=inst+0x0, inicio=1, n=3, faixa=[1, 3], vizinho=Some(4)}"),
            (3, 1, "worldInstancedMeshNode", "subdel", "4 0", r#"Err("instâncias (nó tem 3, .xl espera 4)")"#),
            (3, 1, "worldInstancedMeshNode", "subdel", "3 3", r#"Err("instância 3 >= 3")"#),
            (3, 1, "worldInstancedMeshNode", "subdel", "3", r#"Err("subdel sem índices")"#),
            (3, 1, "worldInstancedMeshNode", "subscale", "3 1 2 2 2", "SubNo{dados=inst+0x0, tam=0x30, j=2, off=0x20, valores=[2.0, 2.0, 2.0], inicio=1, n=3}"),
            (3, 1, "worldInstancedMeshNode", "subrot", "3 0 0 0 0 1", "SubNo{dados=inst+0x0, tam=0x30, j=1, off=0x10, valores=[0.0, 0.0, 0.0, 1.0], inicio=1, n=3}"),
            (3, 1, "worldInstancedMeshNode", "subpos", "3 2 1 2 3 1", "SubNoI32{dados=inst+0x0, tam=0x30, j=3, off=0x0, valores=[131072, 262144, 393216], inicio=1, n=3}"),
            (3, 1, "worldInstancedMeshNode", "subpos", "3 2 1 2 16000 1", r#"Err("subpos fora do alcance do ponto fixo (±16000 m)")"#),
            (3, 1, "worldInstancedMeshNode", "subscale", "3 1 2 2", r#"Err("subscale com 2 valor(es), espera 3")"#),
            (3, 2, "worldCollisionNode", "subdel", "2 1", "Atores{atores=atores+0x10, n=2, faixa=[1]}"),
            (3, 2, "worldCollisionNode", "subpos", "2 1 1.5 -2 3 1", "SubNoI32{dados=atores+0x10, tam=0x30, j=1, off=0x0, valores=[196608, -262144, 393216], inicio=0, n=2}"),
            (3, 2, "worldCollisionNode", "subrot", "2 0 0 0 0 1", "SubNo{dados=atores+0x10, tam=0x30, j=0, off=0x10, valores=[0.0, 0.0, 0.0, 1.0], inicio=0, n=2}"),
            (3, 2, "worldCollisionNode", "subscale", "2 0 1 1 1", r#"Err("subscale não cabe no ator de colisão (o WorldTransform não tem scale)")"#),
            (3, 2, "worldCollisionNode", "subpos", "2 1 1 2 3", r#"Err("subpos com 3 valor(es), espera 4")"#),
            (3, 2, "worldCollisionNode", "subpos", "3 1 1 2 3 1", r#"Err("atores (nó tem 2, .xl espera 3)")"#),
            (3, 0, "worldAdvertisementNode", "", "", r#"Err("nó invisível de deleção ainda não criado")"#),
            (3, 0, "worldStaticMeshNode", "res", "0xabc", r#"Err("classe do nó não decodificada")"#),
            (3, 2, "worldStaticMeshNode", "pos", "1 2 3 1", "Setup{indice=2, off=0x0, valores=[1.0, 2.0, 3.0, 1.0], vizinho=Some(1)}"),
        ];
        for &(esperado, indice, tipo, op, valores, saida) in casos {
            assert_eq!(plano(esperado, indice, tipo, op, valores, 3, true), saida, "{tipo} {op:?} {valores:?}");
        }
        for (op, valores) in [("res", "0xdeadbeef"), ("app", "beef"), ("proxy", "3"), ("proxy", "0"), ("proxy", "-2"), ("res", "zz")] {
            assert_eq!(plano(3, 0, "worldCookedGenericProxyMeshNode", op, valores, 3, true), r#"Err("índice 0 não é worldCookedGenericProxyMeshNode (vptr 0x0, classe None)")"#);
        }
        assert_eq!(plano(3, 0, "worldStaticMeshNode", "", "", 3, false), r#"Err("setups ilegíveis")"#);
        assert_eq!(plano(1, 0, "worldStaticMeshNode", "pos", "1 2 3 1", 1, true), "Setup{indice=0, off=0x0, valores=[1.0, 2.0, 3.0, 1.0], vizinho=None}");
        no_static[..8].copy_from_slice(&0xdead_0000u64.to_le_bytes());
        let d = DelecaoDeNode { modulo: "m".into(), setor: 1, esperado: 3, indice: 0, tipo: "worldStaticMeshNode".into(), op: String::new(), valores: String::new() };
        assert_eq!(str_err(unsafe { planejar_delecao(&d, setups.as_mut_ptr(), 3, true) }), "índice 0 não é worldStaticMeshNode (vptr 0xdead0000)");
    }
}

#[cfg(test)]
mod testes_do_subshape_cozido {
    use super::atores_do_subshape_cozido;

    #[test]
    fn ator_de_uma_shape_desce_e_o_de_varias_fica() {
        let shapes = |a: usize| [1, 3, 1][a];
        assert_eq!(atores_do_subshape_cozido("3 0 0 1 2 2 0", 3, shapes), Ok(vec![0, 2]));
        assert_eq!(atores_do_subshape_cozido("3 0 0 0 0", 3, shapes), Ok(vec![0]));
    }

    #[test]
    fn indice_fora_recusa_o_grupo() {
        let shapes = |a: usize| [1, 3][a];
        assert!(atores_do_subshape_cozido("2 0 1", 2, shapes).is_err(), "shape além das do ator");
        assert!(atores_do_subshape_cozido("2 2 0", 2, shapes).is_err(), "ator além dos do nó");
        assert!(atores_do_subshape_cozido("3 0 0", 2, shapes).is_err(), "contagem do .xl diferente");
        assert!(atores_do_subshape_cozido("2 0", 2, shapes).is_err(), "par incompleto");
    }
}

#[cfg(test)]
mod testes_do_preset_de_colisao {
    use super::*;

    /// Buffer sintético com as 9 listas contíguas, cada elemento preenchido com o número da lista.
    fn sintetico(contagens: [u16; 9]) -> (Vec<u8>, ListasDeColisao) {
        let mut dados = Vec::new();
        let mut offsets = [0u32; 9];
        for k in 0..9 {
            offsets[k] = dados.len() as u32;
            dados.extend(std::iter::repeat(k as u8 + 1).take(contagens[k] as usize * COLISAO_ELEMENTO[k]));
        }
        (dados, ListasDeColisao { offsets, contagens })
    }

    #[test]
    fn preset_novo_entra_no_fim_dos_presets_e_as_listas_seguintes_andam_8() {
        let (dados, l) = sintetico([2, 3, 1, 1, 1, 2, 2, 3, 5]);
        let nome = 0x1122_3344_5566_7788u64;
        let (novo, n, idx) = buffer_com_preset(&dados, &l, nome).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(novo.len(), dados.len() + 8);
        assert_eq!(n.contagens[COLISAO_LISTA_PRESETS], 3);
        assert_eq!(n.offsets[..7], l.offsets[..7]);
        assert_eq!(n.offsets[7], l.offsets[7] + 8);
        assert_eq!(n.offsets[8], l.offsets[8] + 8);
        let p = n.offsets[COLISAO_LISTA_PRESETS] as usize + 2 * 8;
        assert_eq!(u64::from_le_bytes(novo[p..p + 8].try_into().unwrap()), nome);
        // Tudo antes do preset novo e tudo depois dele é o original, byte a byte.
        assert_eq!(novo[..p], dados[..p]);
        assert_eq!(novo[p + 8..], dados[p..]);
        assert_eq!(indice_do_preset(&novo, &n, nome), Some(2));
        assert_eq!(indice_do_preset(&dados, &l, nome), None);
    }

    #[test]
    fn recusa_buffer_curto_e_presets_cheios() {
        let (dados, l) = sintetico([1, 1, 1, 1, 1, 1, 1, 1, 1]);
        assert!(buffer_com_preset(&dados[..dados.len() - 1], &l, 7).is_none());
        let mut cheio = l;
        cheio.contagens[COLISAO_LISTA_PRESETS] = 255;
        assert!(buffer_com_preset(&dados, &cheio, 7).is_none());
    }
}

#[cfg(feature = "devtools")]
pub(crate) static COLISAO_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static COLISAO_CENSO: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "devlog")]
pub(crate) unsafe fn ator_z(atores: *const u8, j: usize) -> i32 {
    (atores.add(j * ATOR_TAM + ATOR_Z_OFF) as *const i32).read_unaligned()
}

/// `(atores, n)` de um nó de colisão, conferido: faixa não vazia, do tamanho de `numActors`, legível.
pub(crate) unsafe fn atores_do_no(no: *const u8) -> Result<(*mut u8, usize), String> {
    if !crate::memoria::is_readable(no as *const c_void, COLISAO_NUM_ATORES_OFF + 2) {
        return Err("nó ilegível".to_string());
    }
    let dados = (no.add(COLISAO_DADOS_OFF) as *const u64).read_unaligned() as usize;
    let bytes = (no.add(COLISAO_DADOS_OFF + 8) as *const u32).read_unaligned() as usize;
    let inicio = (no.add(COLISAO_ATORES_OFF) as *const u32).read_unaligned() as usize;
    let fim = (no.add(COLISAO_ATORES_OFF + 4) as *const u32).read_unaligned() as usize;
    let n = (no.add(COLISAO_NUM_ATORES_OFF) as *const u16).read_unaligned() as usize;
    if dados == 0 || n == 0 || fim <= inicio {
        return Err(format!("lista de atores vazia (numActors {n}): o slot 0x30 ainda não rodou"));
    }
    if fim - inicio != n * ATOR_TAM || fim > bytes || !crate::memoria::is_readable((dados + inicio) as *const c_void, fim - inicio) {
        return Err(format!("lista de atores de {:#x} B em {inicio:#x} não bate com numActors {n} × 0x30 num buffer de {bytes:#x} B", fim - inicio));
    }
    Ok(((dados + inicio) as *mut u8, n))
}

/// `compiledData` (`DataBuffer`: dados `+0`, bytes `+8`) do `worldCookedCollisionNode` (construtor `0x101af70e4`).
pub(crate) const COLISAO_COZIDA_DADOS_OFF: usize = 0x58;

/// Atores do item cozido do setup (`worldCookedCollisionNode`, item de 0x30 B): as seções do item ficam contíguas em `compiledData` a partir do
/// `dataOffset` (`+0x18`), atores primeiro (`numActors` u16 `+0x1c`, 0x30 B cada).
pub(crate) unsafe fn atores_do_no_cozido(no: *const u8, setup: *const u8) -> Result<(*mut u8, usize), String> {
    let item = item_cozido_do_setup(no, setup, 0x30)?;
    if !crate::memoria::is_readable(no.add(COLISAO_COZIDA_DADOS_OFF) as *const c_void, 0x10) {
        return Err("compiledData ilegível".to_string());
    }
    let dados = (no.add(COLISAO_COZIDA_DADOS_OFF) as *const u64).read_unaligned() as usize;
    let bytes = (no.add(COLISAO_COZIDA_DADOS_OFF + 8) as *const u32).read_unaligned() as usize;
    let inicio = (item.add(0x18) as *const u32).read_unaligned() as usize;
    let n = (item.add(0x1c) as *const u16).read_unaligned() as usize;
    if dados == 0 || n == 0 {
        return Err(format!("item de colisão cozida sem atores (numActors {n})"));
    }
    if inicio + n * ATOR_TAM > bytes || !crate::memoria::is_readable((dados + inicio) as *const c_void, n * ATOR_TAM) {
        return Err(format!("{n} atores em {inicio:#x} não cabem no compiledData de {bytes:#x} B"));
    }
    Ok(((dados + inicio) as *mut u8, n))
}

/// Atores do nó de colisão pelo tipo: o cozido pelo item do setup, o `worldCollisionNode` pelas listas do nó.
pub(crate) unsafe fn atores_por_tipo(tipo: &str, no: *const u8, setup: *const u8) -> Result<(*mut u8, usize), String> {
    if tipo == "worldCookedCollisionNode" {
        atores_do_no_cozido(no, setup)
    } else {
        atores_do_no(no)
    }
}

/// Índices absolutos de um `subdel <esperadas> <i>...` num buffer com `n` instâncias a partir de `inicio`.
pub(crate) fn faixa_subdel(valores: &str, inicio: usize, n: usize) -> Result<Vec<usize>, String> {
    let mut campos = valores.split_whitespace().map(|v| v.parse::<usize>());
    let esperadas = campos.next().and_then(Result::ok).ok_or_else(|| "subdel sem contagem".to_string())?;
    if esperadas != n {
        return Err(format!("instâncias (nó tem {n}, .xl espera {esperadas})"));
    }
    let mut v = Vec::new();
    for c in campos {
        let i = c.map_err(|_| "subdel com índice inválido".to_string())?;
        if i >= n {
            return Err(format!("instância {i} >= {n}"));
        }
        v.push(inicio + i);
    }
    if v.is_empty() {
        return Err("subdel sem índices".to_string());
    }
    Ok(v)
}

pub(crate) unsafe fn instancia_scale(dados: *const u8, j: usize) -> [f32; 3] {
    let s = dados.add(j * INSTANCIA_TAM + INSTANCIA_SCALE_OFF) as *const f32;
    [s.read_unaligned(), s.add(1).read_unaligned(), s.add(2).read_unaligned()]
}

// Nós cozidos da 2.5 (`worldCooked*Node`, cobertura 1.28): `mesh`/`meshAppearance`/`material`/`data`/`nbNodesUnderProxy` moram no
// item de `cookedData` (`DynArray<u8>`: dados `+0x40`, bytes `+0x4c`) do `instanceIndex` do setup, com o passo do item gravado pelo construtor em
// `+0x50` (`worldCookedStaticMeshNode`, `0x1046c6774`: `0x21`). O passo lido no nó tem de bater com o item do tipo.
pub(crate) const NO_COZIDO_DADOS: usize = 0x40;

pub(crate) const NO_COZIDO_BYTES: usize = 0x4c;

pub(crate) const NO_COZIDO_PASSO: usize = 0x50;

/// `(passo do item, offset no item, largura)` do campo da op (`res`, `app`, `proxy`) no item cozido do tipo.
pub(crate) fn campo_do_item_cozido(tipo: &str, op: &str) -> Option<(usize, usize, usize)> {
    match (tipo, op) {
        ("worldCookedStaticMeshNode", "res") => Some((0x21, 0, 8)),
        ("worldCookedStaticMeshNode", "app") => Some((0x21, 8, 8)),
        ("worldCookedGenericProxyMeshNode", "res") => Some((0x29, 0, 8)),
        ("worldCookedGenericProxyMeshNode", "app") => Some((0x29, 8, 8)),
        ("worldCookedGenericProxyMeshNode", "proxy") => Some((0x29, 0x21, 4)),
        ("worldCookedStaticDecalNode", "res") => Some((0x24, 0, 8)),
        ("worldCookedAcousticSectorNode", "res") => Some((0x15, 0, 8)),
        _ => None,
    }
}

/// Item do `instanceIndex` do setup em `cookedData` do nó cozido, com o passo do nó igual a `passo`.
pub(crate) unsafe fn item_cozido_do_setup(no: *const u8, setup: *const u8, passo: usize) -> Result<*mut u8, String> {
    if no.is_null()
        || setup.is_null()
        || !crate::memoria::is_readable(no.add(NO_COZIDO_DADOS) as *const c_void, NO_COZIDO_PASSO + 4 - NO_COZIDO_DADOS)
        || !crate::memoria::is_readable(setup as *const c_void, 0x90)
    {
        return Err("nó cozido ou setup ilegível".to_string());
    }
    let lido = (no.add(NO_COZIDO_PASSO) as *const u32).read_unaligned() as usize;
    if lido != passo {
        return Err(format!("passo do item cozido {lido:#x}, o tipo espera {passo:#x}"));
    }
    let dados = (no.add(NO_COZIDO_DADOS) as *const u64).read_unaligned() as *mut u8;
    let bytes = (no.add(NO_COZIDO_BYTES) as *const u32).read_unaligned() as usize;
    let idx = (setup.add(SETUP_INDICE_DE_INSTANCIA) as *const u32).read_unaligned() as usize;
    let fora = || format!("instanceIndex {idx} fora dos {} item(ns) do nó cozido", bytes / passo.max(1));
    let fim = idx.checked_add(1).and_then(|n| n.checked_mul(passo)).ok_or_else(fora)?;
    if dados.is_null() || passo == 0 || fim > bytes {
        return Err(fora());
    }
    let item = dados.add(idx * passo);
    if !crate::memoria::is_readable(item as *const c_void, passo) {
        return Err(fora());
    }
    Ok(item)
}

#[cfg(test)]
mod testes_do_no_cozido {
    use super::{campo_do_item_cozido, item_cozido_do_setup, NO_COZIDO_BYTES, NO_COZIDO_DADOS, NO_COZIDO_PASSO, SETUP_INDICE_DE_INSTANCIA};

    #[test]
    fn campos_por_tipo() {
        assert_eq!(campo_do_item_cozido("worldCookedStaticMeshNode", "app"), Some((0x21, 8, 8)));
        assert_eq!(campo_do_item_cozido("worldCookedGenericProxyMeshNode", "proxy"), Some((0x29, 0x21, 4)));
        assert_eq!(campo_do_item_cozido("worldCookedStaticDecalNode", "res"), Some((0x24, 0, 8)));
        assert_eq!(campo_do_item_cozido("worldCookedAcousticSectorNode", "app"), None);
        assert_eq!(campo_do_item_cozido("worldCookedStaticMeshNode", "proxy"), None);
        assert_eq!(campo_do_item_cozido("worldStaticMeshNode", "res"), None);
    }

    /// Nó de 0x58 B com 3 itens de mesh empacotados (0x21 B cada) e setup com `instanceIndex` 2.
    #[test]
    fn item_pelo_instance_index_e_pelo_passo_do_no() {
        let itens = vec![0u8; 3 * 0x21];
        let mut no = [0u8; 0x58];
        no[NO_COZIDO_DADOS..NO_COZIDO_DADOS + 8].copy_from_slice(&(itens.as_ptr() as u64).to_le_bytes());
        no[NO_COZIDO_BYTES..NO_COZIDO_BYTES + 4].copy_from_slice(&(itens.len() as u32).to_le_bytes());
        no[NO_COZIDO_PASSO..NO_COZIDO_PASSO + 4].copy_from_slice(&0x21u32.to_le_bytes());
        let mut setup = [0u8; 0x90];
        setup[SETUP_INDICE_DE_INSTANCIA..SETUP_INDICE_DE_INSTANCIA + 4].copy_from_slice(&2u32.to_le_bytes());
        let item = unsafe { item_cozido_do_setup(no.as_ptr(), setup.as_ptr(), 0x21) }.expect("item 2");
        assert_eq!(item as usize, itens.as_ptr() as usize + 2 * 0x21);
        // Passo do tipo diferente do nó: recusa.
        assert!(unsafe { item_cozido_do_setup(no.as_ptr(), setup.as_ptr(), 0x29) }.is_err());
        // Índice além dos itens: recusa.
        setup[SETUP_INDICE_DE_INSTANCIA..SETUP_INDICE_DE_INSTANCIA + 4].copy_from_slice(&3u32.to_le_bytes());
        assert!(unsafe { item_cozido_do_setup(no.as_ptr(), setup.as_ptr(), 0x21) }.is_err());
    }

    /// Colisão cozida: item 1 (0x30 B) com `dataOffset` 0x40 e 3 atores; `compiledData` de 0x100 B.
    #[test]
    fn atores_da_colisao_cozida_pelo_data_offset_do_item() {
        use super::{atores_do_no_cozido, ATOR_TAM, COLISAO_COZIDA_DADOS_OFF};
        let mut itens = vec![0u8; 2 * 0x30];
        itens[0x30 + 0x18..0x30 + 0x1c].copy_from_slice(&0x40u32.to_le_bytes());
        itens[0x30 + 0x1c..0x30 + 0x1e].copy_from_slice(&3u16.to_le_bytes());
        let compilado = vec![0u8; 0x100];
        let mut no = [0u8; 0x80];
        no[NO_COZIDO_DADOS..NO_COZIDO_DADOS + 8].copy_from_slice(&(itens.as_ptr() as u64).to_le_bytes());
        no[NO_COZIDO_BYTES..NO_COZIDO_BYTES + 4].copy_from_slice(&(itens.len() as u32).to_le_bytes());
        no[NO_COZIDO_PASSO..NO_COZIDO_PASSO + 4].copy_from_slice(&0x30u32.to_le_bytes());
        no[COLISAO_COZIDA_DADOS_OFF..COLISAO_COZIDA_DADOS_OFF + 8].copy_from_slice(&(compilado.as_ptr() as u64).to_le_bytes());
        no[COLISAO_COZIDA_DADOS_OFF + 8..COLISAO_COZIDA_DADOS_OFF + 12].copy_from_slice(&(compilado.len() as u32).to_le_bytes());
        let mut setup = [0u8; 0x90];
        setup[SETUP_INDICE_DE_INSTANCIA..SETUP_INDICE_DE_INSTANCIA + 4].copy_from_slice(&1u32.to_le_bytes());
        let (atores, n) = unsafe { atores_do_no_cozido(no.as_ptr(), setup.as_ptr()) }.expect("3 atores");
        assert_eq!((atores as usize, n), (compilado.as_ptr() as usize + 0x40, 3));
        // 0x40 + 5 atores passa dos 0x100 B: recusa.
        itens[0x30 + 0x1c..0x30 + 0x1e].copy_from_slice(&5u16.to_le_bytes());
        assert!(unsafe { atores_do_no_cozido(no.as_ptr(), setup.as_ptr()) }.is_err());
        assert_eq!(ATOR_TAM, 0x30);
    }
}

/// `instanceIndex` do setup e o `DynArray` de `tam` bytes por elemento em `+off` do nó: `(entradas, índice)` se o índice cabe nele.
pub(crate) unsafe fn elemento_do_setup(no: *const u8, setup: *const u8, off: usize, tam: usize) -> Result<(*mut u8, usize), String> {
    if !crate::memoria::is_readable(no as *const c_void, 0x98) || !crate::memoria::is_readable(setup as *const c_void, 0x90) {
        return Err("nó ou setup ilegível".to_string());
    }
    let idx = (setup.add(SETUP_INDICE_DE_INSTANCIA) as *const u32).read_unaligned() as usize;
    let entradas = (no.add(off) as *const u64).read_unaligned() as *mut u8;
    let total = (no.add(off + 0x0c) as *const u32).read_unaligned() as usize;
    if entradas.is_null() || idx >= total || !crate::memoria::is_readable(entradas.add(idx * tam) as *const c_void, tam) {
        return Err(format!("instanceIndex {idx} fora dos {total} elementos do nó cozido"));
    }
    Ok((entradas, idx))
}

/// `(dados, início, n, tamanho, vizinho)` do buffer do nó para o setup, conferido; o vizinho é a 1ª instância depois da faixa, se legível (controle de
/// escrita).
pub(crate) unsafe fn instancias_do_no(no: *const u8, setup: *const u8) -> Result<(*mut u8, usize, usize, usize, Option<usize>), String> {
    let (faixas, idx) = elemento_do_setup(no, setup, NO_INSTANCIADO_FAIXAS, 4)?;
    if (no.add(NO_INSTANCIADO_TRANSFORMS + 0x0c) as *const u32).read_unaligned() != 0 {
        return Err("nó com transforms de runtime no lugar do buffer cozido".to_string());
    }
    let inicio = (faixas.add(idx * 4) as *const u16).read_unaligned() as usize;
    let n = (faixas.add(idx * 4 + 2) as *const u16).read_unaligned() as usize;
    let dados = (no.add(NO_INSTANCIADO_BUFFER) as *const u64).read_unaligned() as *mut u8;
    let tam = (no.add(NO_INSTANCIADO_BUFFER + 8) as *const u32).read_unaligned() as usize;
    let cabe = instancias_cabem(inicio, n, tam);
    let k = INSTANCIAS_LOGADAS.fetch_add(1, Ordering::Relaxed);
    if n == 0 || dados.is_null() || !cabe || !crate::memoria::is_readable(dados.add(inicio * INSTANCIA_TAM) as *const c_void, n * INSTANCIA_TAM) {
        return Err(format!("faixa de instâncias inválida (início {inicio}, n {n}, tamanho {tam})"));
    }
    let s0 = instancia_scale(dados, inicio);
    if k < 16 {
        crate::veredito_dev!(
            "[axl-streaming-xl] instâncias do nó {no:p}: início {inicio} n {n} tamanho {tam} B scale[início]={s0:?}"
        );
    }
    let plausivel = (inicio..inicio + n).all(|j| instancia_scale(dados, j).iter().all(|v| v.is_finite() && *v > 0.0 && *v < 1000.0));
    if !plausivel {
        return Err(format!("scale implausível nas instâncias (início {inicio}, n {n}): layout não confirmado"));
    }
    let vizinho = (inicio + n) * INSTANCIA_TAM + INSTANCIA_TAM <= tam && crate::memoria::is_readable(dados.add((inicio + n) * INSTANCIA_TAM) as *const c_void, INSTANCIA_TAM);
    Ok((dados, inicio, n, tam, vizinho.then_some(inicio + n)))
}

/// Confere 1 linha da tabela contra o setor (`setups` setups de 0x90 B a partir de `setups_ptr`) e devolve o plano de escrita ou o motivo da recusa.
pub(crate) unsafe fn planejar_delecao(d: &DelecaoDeNode, setups_ptr: *mut u8, setups: usize, legivel: bool) -> Result<PlanoDelecao, String> {
    if d.esperado != setups {
        return Err(format!("contagem (setor tem {setups}, .xl espera {})", d.esperado));
    }
    let i = d.indice;
    if i >= setups {
        return Err(format!("índice {i} >= {setups}"));
    }
    if !legivel {
        return Err("setups ilegíveis".to_string());
    }
    let (tipo, op, valores) = (d.tipo.as_str(), d.op.as_str(), d.valores.as_str());
    let setup = setups_ptr.add(i * SETUP_TAM);
    let no = (setup.add(SETUP_NO_OFF) as *const u64).read_unaligned() as *mut u8;
    let vptr = if no as u64 >= 0x1_0000_0000 && no as u64 & 7 == 0 && crate::memoria::is_readable(no as *const c_void, 8) {
        (no as *const u64).read_unaligned()
    } else {
        0
    };
    match vptr_do_tipo_de_node(tipo) {
        Some(vm) if vptr != crate::rebase(vm) as u64 => return Err(format!("índice {i} não é {tipo} (vptr {:#x})", vm_do_jogo(vptr))),
        Some(_) => {}
        None => {
            let classe = if vptr != 0 { classe_hash_do_vptr(vptr) } else { None };
            if classe != Some(tipo_hash(tipo)) {
                return Err(format!("índice {i} não é {tipo} (vptr {:#x}, classe {:?})", vm_do_jogo(vptr), classe.map(|h| format!("{h:#x}"))));
            }
        }
    }
    if let Some((passo, off, largura)) = campo_do_item_cozido(tipo, op) {
        let item = item_cozido_do_setup(no, setup, passo)?;
        return Ok(PlanoDelecao::Campo { no: item, off, largura, valor: valor_do_campo(op, valores)?, soma: op == "proxy" });
    }
    let vizinho = if i + 1 < setups { Some(i + 1) } else { i.checked_sub(1) };
    let colisao = matches!(tipo, "worldCollisionNode" | "worldCookedCollisionNode");
    let instanciado = tipo == "worldInstancedMeshNode";
    let destrutivel = tipo == "worldInstancedDestructibleMeshNode";
    let sub = matches!(op, "subpos" | "subrot" | "subscale");
    match (tipo, op) {
        ("worldStaticMeshNode" | "worldCookedStaticMeshNode", "") => Ok(PlanoDelecao::ScaleSetup(i)),
        ("worldInstancedMeshNode", "" | "subdel") => {
            let (dados, inicio, n, _, vizinho) = instancias_do_no(no, setup)?;
            let faixa = if op.is_empty() { (inicio..inicio + n).collect() } else { faixa_subdel(valores, inicio, n)? };
            Ok(PlanoDelecao::Instancias { dados, inicio, n, faixa, vizinho })
        }
        ("worldInstancedDestructibleMeshNode", "subdel") => {
            let (dados, inicio, n) = destrutiveis_do_no(no)?;
            Ok(PlanoDelecao::Destrutiveis { dados, inicio, n, faixa: faixa_subdel(valores, inicio, n)? })
        }
        ("worldCollisionNode" | "worldCookedCollisionNode", "subdel") => {
            let (atores, n) = atores_por_tipo(tipo, no, setup)?;
            Ok(PlanoDelecao::Atores { atores, n, faixa: faixa_subdel(valores, 0, n)? })
        }
        ("worldCookedCollisionNode", "") => {
            let (atores, n) = atores_do_no_cozido(no, setup)?;
            Ok(PlanoDelecao::Atores { atores, n, faixa: (0..n).collect() })
        }
        ("worldCollisionNode", "subshape") => {
            let (dados, bytes, listas) = listas_de_colisao(no)?;
            let shapes = shapes_do_subshape(valores, dados, &listas)?;
            let tem_preset = indice_do_preset(std::slice::from_raw_parts(dados, bytes), &listas, cname(PRESET_SEM_COLISAO)).is_some();
            // Sem o preset no nó, ele entra num buffer novo pelo alocador do `RawBuffer` do nó.
            if !tem_preset && crate::jogo::buffers::vtable_do_alocador_rawbuffer(no.add(COLISAO_DADOS_OFF + 0x10)).is_none() {
                return Err("buffer do nó sem o alocador conferido: o preset No Collision não pode entrar".to_string());
            }
            Ok(PlanoDelecao::ShapesSemColisao { no, shapes })
        }
        ("worldCookedCollisionNode", "subshape") => {
            let (atores, n) = atores_do_no_cozido(no, setup)?;
            let faixa = atores_do_subshape_cozido(valores, n, |a| (atores.add(a * ATOR_TAM + 0x22) as *const u16).read_unaligned() as usize)?;
            Ok(PlanoDelecao::Atores { atores, n, faixa })
        }
        ("worldAdvertisementNode", "") => Ok(PlanoDelecao::Doador { indice: i, doador: no_fantasma()?, vizinho }),
        (_, _) if colisao && sub => {
            let (atores, n) = atores_por_tipo(tipo, no, setup)?;
            if op == "subscale" {
                return Err("subscale não cabe no ator de colisão (o WorldTransform não tem scale)".to_string());
            }
            let (j, v) = valores_do_sub(op, valores, n, "atores", "ator", 4)?;
            if !(0..n).all(|a| unitaria(f32s::<4>(atores.add(a * ATOR_TAM + 0x10)))) {
                return Err("orientação não unitária nos atores de colisão (layout do ator não confirmado)".to_string());
            }
            if op == "subpos" {
                return Ok(PlanoDelecao::SubNoI32 { dados: atores, tam: ATOR_TAM, j, off: 0, valores: ponto_fixo(&v[..3])?, inicio: 0, n });
            }
            Ok(PlanoDelecao::SubNo { dados: atores, tam: ATOR_TAM, j, off: 0x10, valores: v, inicio: 0, n })
        }
        (_, _) if (instanciado || destrutivel) && sub => {
            let (dados, inicio, n, tam) = if instanciado {
                let (dados, inicio, n, _, _) = instancias_do_no(no, setup)?;
                (dados, inicio, n, INSTANCIA_TAM)
            } else {
                let (dados, inicio, n) = destrutiveis_do_no(no)?;
                (dados, inicio, n, DESTRUTIVEL_TAM)
            };
            let (off, qtd) = match op {
                "subpos" => (0, 4),
                "subrot" => (0x10, 4),
                _ => (0x20, 3),
            };
            if off + 4 * qtd > tam {
                return Err(format!("{op} não cabe no elemento de {tam:#x} B de {tipo}"));
            }
            let (k, v) = valores_do_sub(op, valores, n, "instâncias", "instância", qtd)?;
            let j = inicio + k;
            if !(inicio..inicio + n).all(|e| unitaria(f32s::<4>(dados.add(e * tam + 0x10)))) {
                return Err(format!("orientação não unitária em {tipo} (layout do elemento não confirmado)"));
            }
            if destrutivel {
                let xf = Xf { p: f32s::<3>(setup), q: f32s::<4>(setup.add(0x10)) };
                if !unitaria(xf.q) || !xf.p.iter().all(|c| c.is_finite()) {
                    return Err("orientação do setup não unitária (layout do setup não confirmado)".to_string());
                }
                let (pos, rot) = if op == "subpos" { (Some([v[0], v[1], v[2]]), None) } else { (None, Some([v[0], v[1], v[2], v[3]])) };
                return Ok(PlanoDelecao::SubNoLocal { dados, j, inicio, n, setup: xf, pos, rot });
            }
            if op == "subpos" {
                let plausivel = (inicio..inicio + n).all(|e| {
                    let z = (dados.add(e * tam + 0x08) as *const i32).read_unaligned() as f64 / FIXEDPOINT_ESCALA as f64;
                    (-1000.0..3000.0).contains(&z)
                });
                if !plausivel {
                    return Err("posição do instanced mesh fora do ponto fixo plausível (layout do elemento não confirmado)".to_string());
                }
                return Ok(PlanoDelecao::SubNoI32 { dados, tam, j, off: 0, valores: ponto_fixo(&v[..3])?, inicio, n });
            }
            Ok(PlanoDelecao::SubNo { dados, tam, j, off, valores: v, inicio, n })
        }
        ("worldInstancedMeshNode", "res" | "app") => {
            let (dados, idx) = elemento_do_setup(no, setup, NO_INSTANCIADO_DADOS, DADO_COZIDO_TAM)?;
            let off = idx * DADO_COZIDO_TAM + if op == "res" { DADO_COZIDO_MESH } else { DADO_COZIDO_APARENCIA };
            Ok(PlanoDelecao::Campo { no: dados, off, largura: 8, valor: valor_do_campo(op, valores)?, soma: false })
        }
        (_, "res" | "app" | "rec" | "proxy") => {
            let classe = if vptr != 0 { classe_do_vptr(vptr) } else { None }.ok_or_else(|| "classe do nó não decodificada".to_string())?;
            let nomes: &[&str] = match op {
                "res" => &["mesh", "entityTemplate", "effect", "material", "meshRef"],
                "app" => &["meshAppearance", "appearanceName"],
                "rec" => &["objectRecordId"],
                _ => &["nbNodesUnderProxy"],
            };
            let largura = if op == "proxy" { 4 } else { 8 };
            let off = nomes
                .iter()
                .find_map(|n| offset_do_campo(classe as *mut c_void, bwms_primitives::fnv1a64(n.as_bytes())))
                .ok_or_else(|| format!("{op}: a classe {tipo} não tem {}", nomes.join("/")))?;
            let valor = valor_do_campo(op, valores)?;
            if !crate::memoria::is_readable(no.add(off) as *const c_void, largura) {
                return Err(format!("{op}: campo ilegível"));
            }
            Ok(PlanoDelecao::Campo { no, off, largura, valor, soma: op == "proxy" })
        }
        (_, "pos" | "rot" | "scale") => {
            let (off, n) = match op {
                "pos" => (0, 4),
                "rot" => (0x10, 4),
                _ => (0x20, 3),
            };
            let v = floats_do_xl(op, valores.split_whitespace(), n)?;
            Ok(PlanoDelecao::Setup { indice: i, off, valores: v, vizinho })
        }
        (_, "") if vptr_do_tipo_de_node(tipo).is_none() => Ok(PlanoDelecao::Generica { indice: i, doador: no_fantasma()?, vizinho }),
        (_, "") => Err(format!("tipo {tipo} sem suporte")),
        (_, _) => Err(format!("op {op} sem suporte")),
    }
}

/// Setup de nó no setor: 0x90 B, com o ponteiro do nó em `+0x50`.
const SETUP_TAM: usize = 0x90;

const SETUP_NO_OFF: usize = 0x50;

/// `N` floats a partir de `p`.
unsafe fn f32s<const N: usize>(p: *const u8) -> [f32; N] {
    std::array::from_fn(|k| (p.add(4 * k) as *const f32).read_unaligned())
}

/// Quaternion de módulo 1 com folga de 0,01: o controle de que o offset lido é mesmo uma orientação.
fn unitaria(q: [f32; 4]) -> bool {
    (q.iter().map(|v| v * v).sum::<f32>() - 1.0).abs() < 0.01
}

/// Posição em metros no ponto fixo do motor (1/131072 m); fora de ±16000 m o `i32` estoura.
fn ponto_fixo(v: &[f32]) -> Result<Vec<i32>, String> {
    if v.iter().any(|c| c.abs() >= 16000.0) {
        return Err("subpos fora do alcance do ponto fixo (±16000 m)".to_string());
    }
    Ok(v.iter().map(|&c| (c as f64 * FIXEDPOINT_ESCALA as f64).round() as i32).collect())
}

/// `n` floats finitos dos campos.
fn floats_do_xl<'a>(op: &str, campos: impl Iterator<Item = &'a str>, n: usize) -> Result<Vec<f32>, String> {
    let v = campos.map(|c| c.parse::<f32>()).collect::<Result<Vec<f32>, _>>().map_err(|_| format!("{op} com valor inválido"))?;
    if v.len() != n || !v.iter().all(|c| c.is_finite()) {
        return Err(format!("{op} com {} valor(es), espera {n}", v.len()));
    }
    Ok(v)
}

/// `<contagem> <índice> <valores...>` de uma mutação de sub-nó: o índice conferido contra os `n` elementos e `qtd` floats.
fn valores_do_sub(op: &str, valores: &str, n: usize, elementos: &str, elemento: &str, qtd: usize) -> Result<(usize, Vec<f32>), String> {
    let mut campos = valores.split_whitespace();
    let contagem = campos.next().and_then(|c| c.parse::<usize>().ok()).ok_or_else(|| format!("{op} sem contagem"))?;
    if contagem != n {
        return Err(format!("{elementos} (nó tem {n}, .xl espera {contagem})"));
    }
    let i = campos.next().and_then(|c| c.parse::<usize>().ok()).ok_or_else(|| format!("{op} sem índice"))?;
    if i >= n {
        return Err(format!("{elemento} {i} >= {n}"));
    }
    Ok((i, floats_do_xl(op, campos, qtd)?))
}

/// Valor de uma mutação de campo: `proxy` é a diferença decimal positiva que se soma; as outras, hash em hex, com ou sem `0x`.
fn valor_do_campo(op: &str, valores: &str) -> Result<u64, String> {
    let t = valores.trim();
    if op == "proxy" {
        let v = t.parse::<i64>().map_err(|_| "proxy com valor inválido".to_string())?;
        if v <= 0 {
            return Err("proxy só soma diferença positiva".to_string());
        }
        return Ok(v as u64);
    }
    let hex = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")).unwrap_or(t);
    u64::from_str_radix(hex, 16).map_err(|_| format!("{op} com valor inválido"))
}

#[cfg(feature = "devtools")]
pub(crate) static VPTR_CENSO_SETORES: AtomicU64 = AtomicU64::new(0);

// `#49` 4e-1, teste automático (`~/.bwms-hook-wssector-inst-teste`, 2026-09-11): o boot 54 mostrou que o índice 3 do setor do menu não é nó
// de instâncias. Aqui o alvo sai do próprio setor. O hook do `PostLoad` roda em thread de job e só lê memória: a 1ª versão perguntava a classe pelo
// RTTI (GetType + nome do CName) e o boot 55 parou no 1º setor. O alvo agora é achado pelo vptr (`worldInstancedMeshNode` `0x1078e5130`), e os 4
// primeiros setores logam os vptrs distintos de nó (pra conferir a constante offline). No 1º setor com esse vptr e buffer válido com ≥ 2 instâncias,
// entram na tabela em memória um controle (contagem de instâncias errada), a sub-deleção da instância 0 e, se houver um 2º nó de instâncias (outro
// objeto) no setor, a deleção desse nó inteiro. O caminho do `.xl` aplica e as linhas saem da tabela no fim.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_instancias(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || INST_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let no_do_setup = |i: usize| (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
    let alvo_vt = crate::rebase(crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM) as u64;
    if VPTR_CENSO_SETORES.fetch_add(1, Ordering::Relaxed) < 4 {
        let mut distintos: Vec<(u64, usize)> = Vec::new();
        for i in 0..setups {
            let no = no_do_setup(i);
            if no < 0x1_0000_0000 || no & 7 != 0 {
                continue;
            }
            let vt = vm_do_jogo((no as *const u64).read_unaligned());
            match distintos.iter().position(|(v, _)| *v == vt) {
                Some(p) => distintos[p].1 += 1,
                None if distintos.len() < 24 => distintos.push((vt, 1)),
                None => {}
            }
        }
        let lista: Vec<String> = distintos.iter().map(|(v, c)| format!("{v:#x}×{c}")).collect();
        crate::veredito_dev!("[axl-inst-teste] censo de vptr setor={hash:#018x} setups={setups}: {}", lista.join(" "));
    }
    let mut alvos: Vec<(usize, usize, u64)> = Vec::new();
    for i in 0..setups {
        let no = no_do_setup(i);
        if no < 0x1_0000_0000 || no & 7 != 0 || alvos.iter().any(|(j, _, _)| no_do_setup(*j) == no) {
            continue;
        }
        if (no as *const u64).read_unaligned() != alvo_vt {
            continue;
        }
        if let Ok((_, _, n, _, _)) = instancias_do_no(no as *const u8, setups_ptr.add(i * 0x90)) {
            if n >= 2 {
                alvos.push((i, n, (no as *const u64).read_unaligned()));
                if alvos.len() == 2 {
                    break;
                }
            }
        }
    }
    if alvos.is_empty() {
        return;
    }
    INST_TESTE_FEITO.store(true, Ordering::Relaxed);
    let (i0, n0, vt0) = alvos[0];
    let segundo = alvos.get(1).map_or(String::new(), |(i1, n1, _)| format!("; 2º alvo índice {i1} com {n1} instâncias"));
    crate::veredito_dev!(
        "[axl-inst-teste] setor={hash:#018x} setups={setups} alvo índice {i0} com {n0} instâncias, vptr {:#x} (constante {:#x}: {}){segundo}",
        vm_do_jogo(vt0),
        crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM,
        if vm_do_jogo(vt0) == crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM { "certa" } else { "errada" }
    );
    let linha = |modulo: &str, indice: usize, op: &str, valores: String| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice,
        tipo: "worldInstancedMeshNode".to_string(),
        op: op.to_string(),
        valores,
    };
    let mut novas = vec![linha("auto-inst-subcontagem", i0, "subdel", format!("{} 0", n0 + 7)), linha("auto-inst-sub0", i0, "subdel", format!("{n0} 0"))];
    if let Some((i1, _, _)) = alvos.get(1) {
        novas.push(linha("auto-inst-inteiro", *i1, "", String::new()));
    }
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(novas),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-inst-"));
    }
    crate::veredito_dev!("[axl-inst-teste] VEREDITO setor {hash:#018x}: grupos auto-inst-* aplicados pelo caminho do .xl (linhas [axl-streaming-xl] acima)");
    // 2.5: `app` em nó instanciado grava no elemento do setup em `cookedInstances`. O valor é o `meshAppearance` de outro elemento do mesmo nó (uma
    // aparência que existe), relido e devolvido; os outros elementos têm de ficar como estavam.
    let (no0, setup0) = (no_do_setup(i0) as *const u8, setups_ptr.add(i0 * 0x90));
    let Ok((dadosc, idx)) = elemento_do_setup(no0, setup0, NO_INSTANCIADO_DADOS, DADO_COZIDO_TAM) else {
        crate::veredito_dev!("[axl-inst-teste] VEREDITO-APP setor {hash:#018x}: sem elemento do setup em cookedInstances; app-ok=false");
        return;
    };
    let total = (no0.add(NO_INSTANCIADO_DADOS + 0x0c) as *const u32).read_unaligned() as usize;
    let aparencia = |k: usize| (dadosc.add(k * DADO_COZIDO_TAM + DADO_COZIDO_APARENCIA) as *const u64).read_unaligned();
    let antes = aparencia(idx);
    let outros_antes: Vec<u64> = (0..total).filter(|&k| k != idx).map(aparencia).collect();
    let valor = (0..total).map(aparencia).find(|&v| v != antes).unwrap_or(antes ^ 1);
    let linha_app = DelecaoDeNode {
        modulo: "auto-inst-app".to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldInstancedMeshNode".to_string(),
        op: "app".to_string(),
        valores: format!("{valor:x}"),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.push(linha_app),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-inst-"));
    }
    let gravado = aparencia(idx);
    let outros_intactos = (0..total).filter(|&k| k != idx).map(aparencia).collect::<Vec<_>>() == outros_antes;
    (dadosc.add(idx * DADO_COZIDO_TAM + DADO_COZIDO_APARENCIA) as *mut u64).write_unaligned(antes);
    let ok = gravado == valor && outros_intactos && aparencia(idx) == antes;
    crate::veredito_dev!(
        "[axl-inst-teste] VEREDITO-APP setor {hash:#018x}: elemento {idx} de {total} em cookedInstances, meshAppearance {antes:#x} → {gravado:#x} (pedido \
         {valor:#x}), outros intactos {outros_intactos}, devolvido; app-ok={ok}"
    );
}

/// vptr do `worldCookedStaticMeshNode` da 2.5, a última gravada pelo construtor `0x1046c6774` (`+124`; a de `+56`, `0x1078f0aa0`, é a da base
/// `worldCookedMeshNode`).
#[cfg(feature = "devtools")]
pub(crate) const NO_MESH_COZIDO_VPTR_VM: u64 = 0x1_07c5_f588;

#[cfg(feature = "devtools")]
pub(crate) static COZIDO_TESTE_FEITO: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "devtools")]
static COZIDO_SETORES_VISTOS: AtomicU64 = AtomicU64::new(0);

// Nós cozidos da 2.5 (cobertura 1.28), teste automático (mesmo marcador do teste de instâncias): no 1º setor com `worldCookedStaticMeshNode` cujo
// item do setup está dentro de `cookedData`, um `app` pelo caminho do `.xl` com o `meshAppearance` de outro item do mesmo nó; relido, os outros itens
// intactos, devolvido. Sem nó cozido nos primeiros 60 setores, o veredito diz isso.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_no_cozido(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || COZIDO_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let vt = crate::rebase(NO_MESH_COZIDO_VPTR_VM) as u64;
    let no_do_setup = |i: usize| (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
    let alvo = (0..setups).find(|&i| {
        let no = no_do_setup(i);
        no >= 0x1_0000_0000 && no & 7 == 0 && (no as *const u64).read_unaligned() == vt && item_cozido_do_setup(no as *const u8, setups_ptr.add(i * 0x90), 0x21).is_ok()
    });
    let Some(i0) = alvo else {
        if COZIDO_SETORES_VISTOS.fetch_add(1, Ordering::Relaxed) == 59 {
            crate::veredito_dev!("[axl-cozido-teste] VEREDITO-COZIDO sem worldCookedStaticMeshNode nos primeiros 60 setores; cozido-ok=false");
        }
        return;
    };
    COZIDO_TESTE_FEITO.store(true, Ordering::Relaxed);
    let no = no_do_setup(i0) as *const u8;
    let (dados, bytes) = ((no.add(NO_COZIDO_DADOS) as *const u64).read_unaligned() as *mut u8, (no.add(NO_COZIDO_BYTES) as *const u32).read_unaligned() as usize);
    let total = bytes / 0x21;
    let aparencia = |k: usize| (dados.add(k * 0x21 + 8) as *const u64).read_unaligned();
    let Ok(item) = item_cozido_do_setup(no, setups_ptr.add(i0 * 0x90), 0x21) else { return };
    let idx = (item as usize - dados as usize) / 0x21;
    let antes = aparencia(idx);
    let outros_antes: Vec<u64> = (0..total).filter(|&k| k != idx).map(aparencia).collect();
    let valor = (0..total).map(aparencia).find(|&v| v != antes).unwrap_or(antes ^ 1);
    let linha = DelecaoDeNode {
        modulo: "auto-cozido-app".to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldCookedStaticMeshNode".to_string(),
        op: "app".to_string(),
        valores: format!("{valor:x}"),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.push(linha),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| d.modulo != "auto-cozido-app");
    }
    let gravado = aparencia(idx);
    let outros_intactos = (0..total).filter(|&k| k != idx).map(aparencia).collect::<Vec<_>>() == outros_antes;
    (item.add(8) as *mut u64).write_unaligned(antes);
    let ok = gravado == valor && outros_intactos && aparencia(idx) == antes;
    crate::veredito_dev!(
        "[axl-cozido-teste] VEREDITO-COZIDO setor {hash:#018x}: setup {i0}, item {idx} de {total} em cookedData (passo 0x21), meshAppearance {antes:#x} → \
         {gravado:#x} (pedido {valor:#x}), outros intactos {outros_intactos}, devolvido; cozido-ok={ok}"
    );
}

/// vptr do `worldCookedCollisionNode` da 2.5, a última gravada pelo construtor `0x101af70e4` (`+124`).
#[cfg(feature = "devtools")]
pub(crate) const NO_COLISAO_COZIDA_VPTR_VM: u64 = 0x1_078e_0b30;

#[cfg(feature = "devtools")]
pub(crate) static COLISAO_COZIDA_TESTE_FEITO: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "devtools")]
static COLISAO_COZIDA_SETORES_VISTOS: AtomicU64 = AtomicU64::new(0);

// Colisão cozida da 2.5 (cobertura 1.28, mesmo marcador): no 1º `worldCookedCollisionNode` com 2+ atores no item do setup, o ator 0 vai a Z -2000 pelo caminho
// do `.xl` (`subdel`), o ator 1 fica; o Z do ator 0 volta.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_colisao_cozida(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || COLISAO_COZIDA_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let vt = crate::rebase(NO_COLISAO_COZIDA_VPTR_VM) as u64;
    let alvo = (0..setups).find_map(|i| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 || (no as *const u64).read_unaligned() != vt {
            return None;
        }
        atores_do_no_cozido(no as *const u8, setups_ptr.add(i * 0x90)).ok().filter(|(_, n)| *n >= 2).map(|(a, n)| (i, a, n))
    });
    let Some((i0, atores, n0)) = alvo else {
        if COLISAO_COZIDA_SETORES_VISTOS.fetch_add(1, Ordering::Relaxed) == 59 {
            crate::veredito_dev!("[axl-cozido-teste] VEREDITO-COLISAO-COZIDA sem worldCookedCollisionNode com 2+ atores nos primeiros 60 setores; colisao-cozida-ok=false");
        }
        return;
    };
    COLISAO_COZIDA_TESTE_FEITO.store(true, Ordering::Relaxed);
    let (z0, z1) = (ator_z(atores, 0), ator_z(atores, 1));
    let linha = DelecaoDeNode {
        modulo: "auto-cozido-colisao".to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldCookedCollisionNode".to_string(),
        op: "subdel".to_string(),
        valores: format!("{n0} 0"),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.push(linha),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| d.modulo != "auto-cozido-colisao");
    }
    let (gravado, vizinho) = (ator_z(atores, 0), ator_z(atores, 1));
    (atores.add(ATOR_Z_OFF) as *mut i32).write_unaligned(z0);
    let ok = gravado == ATOR_Z_DELETADO && vizinho == z1 && ator_z(atores, 0) == z0;
    crate::veredito_dev!(
        "[axl-cozido-teste] VEREDITO-COLISAO-COZIDA setor {hash:#018x}: setup {i0}, {n0} atores no item; Z do ator 0 {z0} → {gravado} (esperado \
         {ATOR_Z_DELETADO}), ator 1 intacto {}, devolvido; colisao-cozida-ok={ok}",
        vizinho == z1
    );
}

// Parte 4f-3 (2026-09-12): mutações de campo do nó vindas do `.xl`, como a referência — `res <hash>` grava o `ResourcePath` do campo de recurso da
// classe (`mesh`, `entityTemplate`, `effect`, `material` ou `meshRef`), `app <CName>` a aparência (`meshAppearance`/`appearanceName`), `rec <TweakDBID>`
// o `objectRecordId` e `proxy <diff>` soma no `nbNodesUnderProxy`. O campo sai da `CClass` decodificada do vptr, subindo pelos pais (`+0x10`) e
// comparando o hash do nome nas propriedades (`class_own_properties`, só leitura) — serve no thread de job, sem tabela de offset por classe. Teste
// automático (mesmo marcador da 4e-1): no 1º setor com `worldStaticMeshNode`, um controle (`rec` num static mesh, que não tem `objectRecordId`) e o grupo
// com `res` e `app` nesse nó, relidos; o nó volta aos valores originais no mesmo passo.
#[cfg(feature = "devtools")]
pub(crate) static CAMPO_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Offset do campo com esse hash de nome na classe ou nos pais, só lendo memória.
pub(crate) unsafe fn offset_do_campo(cls: *mut c_void, hash_nome: u64) -> Option<usize> {
    let mut c = cls;
    let mut niveis = 0;
    while !c.is_null() && niveis < 16 && crate::memoria::is_readable(c as *const c_void, 0x20) {
        for p in crate::rtti::class_own_properties(c) {
            let pb = p as *const u8;
            if crate::memoria::is_readable(p as *const c_void, 0x28) && (pb.add(0x08) as *const u64).read_unaligned() == hash_nome {
                return Some((pb.add(0x20) as *const u32).read_unaligned() as usize);
            }
        }
        c = ((c as *const u8).add(0x10) as *const u64).read_unaligned() as *mut c_void;
        niveis += 1;
    }
    None
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_campo(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || CAMPO_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let Some((i0, doador)) = doador_do_setor(setups_ptr, setups) else { return };
    let Some(cls) = classe_do_vptr((doador as *const u64).read_unaligned()) else { return };
    CAMPO_TESTE_FEITO.store(true, Ordering::Relaxed);
    let offs = (
        offset_do_campo(cls as *mut c_void, bwms_primitives::fnv1a64(b"mesh")),
        offset_do_campo(cls as *mut c_void, bwms_primitives::fnv1a64(b"meshAppearance")),
    );
    let (Some(off_res), Some(off_app)) = offs else {
        crate::veredito_dev!("[axl-campo-teste] setor={hash:#018x}: static mesh sem mesh/meshAppearance nas propriedades ({:?})", offs);
        return;
    };
    let no = doador as *mut u8;
    let ler = |off: usize| (no.add(off) as *const u64).read_unaligned();
    let (res_antes, app_antes) = (ler(off_res), ler(off_app));
    let res_novo = bwms_primitives::fnv1a64(b"base\\bwms\\teste_4f3.mesh");
    let app_novo = bwms_primitives::fnv1a64(b"bwms_teste_4f3");
    crate::veredito_dev!(
        "[axl-campo-teste] setor={hash:#018x} static mesh no índice {i0}: mesh @{off_res:#x} = {res_antes:#x}, meshAppearance @{off_app:#x} = {app_antes:#x}"
    );
    let linha = |modulo: &str, op: &str, valores: String| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldStaticMeshNode".to_string(),
        op: op.to_string(),
        valores,
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![
            linha("auto-campo-rec", "rec", "0x1234".to_string()),
            linha("auto-campo-no", "res", format!("{res_novo:#x}")),
            linha("auto-campo-no", "app", format!("{app_novo:#x}")),
        ]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-campo-"));
    }
    let (res_depois, app_depois) = (ler(off_res), ler(off_app));
    (no.add(off_res) as *mut u64).write_unaligned(res_antes);
    (no.add(off_app) as *mut u64).write_unaligned(app_antes);
    let devolvido = ler(off_res) == res_antes && ler(off_app) == app_antes;
    crate::veredito_dev!(
        "[axl-campo-teste] VEREDITO setor {hash:#018x}: mesh {} e meshAppearance {} depois do .xl; valores originais devolvidos={devolvido}",
        res_depois == res_novo,
        app_depois == app_novo
    );
}

// Parte 4f-2 (2026-09-12): mutações de sub-nó vindas do `.xl` (`subpos`/`subrot`/`subscale <esperadas> <i> <valores>`). O elemento do buffer do
// instanced mesh (0x30 B) segue a disposição do setup — position `@0x00`, orientation `@0x10`, scale `@0x20` —; o do destrutível (0x20 B) tem
// position e orientation, e a scale não cabe. O planejador só grava se a orientação de todas as instâncias do nó for um quaternion unitário (controle
// do layout). Teste automático (mesmo marcador da 4e-1): pra cada um dos dois tipos, no 1º nó com ≥ 2 instâncias, um controle de contagem errada
// (e, no destrutível, a scale que não cabe) e o grupo com `subpos` (Z+1), `subrot` (a mesma) e `subscale` (×1,5, só no instanced) na instância 0;
// o elemento volta inteiro no mesmo passo.
// Parte 4f-2c (2026-09-12): o mesmo teste nos atores do `worldCollisionNode` — posição em ponto fixo (i32 `@0x00/0x04/0x08`, 1/131072 m) e
// orientação `@0x10`; `subscale` recusado (sem scale no ator). Como um ator de colisão pode ser o chão debaixo do V, o ator volta no mesmo passo.
#[cfg(feature = "devtools")]
pub(crate) static SUBNO_TESTE_FEITO: [std::sync::atomic::AtomicBool; 3] =
    [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)];

#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_subno(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let tipos = [
        ("worldInstancedMeshNode", crate::jogo::mundo::NO_MALHA_INSTANCIADA_VPTR_VM, INSTANCIA_TAM),
        ("worldInstancedDestructibleMeshNode", WORLDINSTANCEDDESTRUCTIBLEMESHNODE_VPTR_VM, DESTRUTIVEL_TAM),
        ("worldCollisionNode", WORLDCOLLISIONNODE_VPTR_VM, ATOR_TAM),
    ];
    for (k, (tipo, vptr_vm, tam)) in tipos.into_iter().enumerate() {
        if SUBNO_TESTE_FEITO[k].load(Ordering::Relaxed) {
            continue;
        }
        let vt = crate::rebase(vptr_vm) as u64;
        let alvo = (0..setups).find_map(|i| {
            let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
            if no < 0x1_0000_0000 || no & 7 != 0 || (no as *const u64).read_unaligned() != vt {
                return None;
            }
            let buffer = if k == 0 {
                instancias_do_no(no as *const u8, setups_ptr.add(i * 0x90)).ok().map(|(d, s, n, _, _)| (d, s, n))
            } else if k == 1 {
                destrutiveis_do_no(no as *const u8).ok()
            } else {
                atores_do_no(no as *const u8).ok().map(|(d, n)| (d, 0, n))
            };
            buffer.filter(|(_, _, n)| *n >= 2).map(|(d, s, n)| (i, d, s, n))
        });
        let Some((i0, dados, inicio, n)) = alvo else { continue };
        SUBNO_TESTE_FEITO[k].store(true, Ordering::Relaxed);
        let e = dados.add(inicio * tam);
        let antes: Vec<u8> = std::slice::from_raw_parts(e as *const u8, tam).to_vec();
        let f = |o: usize| (e.add(o) as *const f32).read_unaligned();
        let fixo = |o: usize| (e.add(o) as *const i32).read_unaligned() as f32 / 131072.0;
        let s = setups_ptr.add(i0 * 0x90) as *const f32;
        let setup_xf = Xf {
            p: [s.read_unaligned(), s.add(1).read_unaligned(), s.add(2).read_unaligned()],
            q: [s.add(4).read_unaligned(), s.add(5).read_unaligned(), s.add(6).read_unaligned(), s.add(7).read_unaligned()],
        };
        // Instância 0 no mundo (4f-4): ponto fixo no instanced mesh e na colisão; no destrutível o elemento é relativo ao setup.
        let mundo = || {
            if k == 1 {
                Xf { p: [f(0x00), f(0x04), f(0x08)], q: [f(0x10), f(0x14), f(0x18), f(0x1C)] }.mul(setup_xf)
            } else {
                Xf { p: [fixo(0x00), fixo(0x04), fixo(0x08)], q: [f(0x10), f(0x14), f(0x18), f(0x1C)] }
            }
        };
        let m0 = mundo();
        crate::veredito_dev!(
            "[axl-subno-teste] setor={hash:#018x} {tipo} no índice {i0} com {n} instâncias; instância 0 no mundo: pos ({:.2} {:.2} {:.2}), rot ({:.3} {:.3} {:.3} {:.3})",
            m0.p[0], m0.p[1], m0.p[2], m0.q[0], m0.q[1], m0.q[2], m0.q[3]
        );
        let linha = |modulo: String, op: &str, valores: String| DelecaoDeNode {
            modulo,
            setor: hash,
            esperado: setups,
            indice: i0,
            tipo: tipo.to_string(),
            op: op.to_string(),
            valores,
        };
        let mut linhas = vec![
            linha(format!("auto-subno{k}-contagem"), "subpos", format!("{} 0 1 1 1 1", n + 7)),
            linha(format!("auto-subno{k}-transform"), "subpos", format!("{n} 0 {} {} {} 1", m0.p[0], m0.p[1], m0.p[2] + 1.0)),
            linha(format!("auto-subno{k}-transform"), "subrot", format!("{n} 0 {} {} {} {}", m0.q[0], m0.q[1], m0.q[2], m0.q[3])),
        ];
        // O ator de colisão tem 0x30 B mas não tem scale: a `subscale` dele vai num grupo à parte, senão recusa o grupo do transform inteiro (boot 60).
        if tam >= 0x2C && k != 2 {
            linhas.push(linha(format!("auto-subno{k}-transform"), "subscale", format!("{n} 0 {} {} {}", f(0x20) * 1.5, f(0x24) * 1.5, f(0x28) * 1.5)));
        } else {
            linhas.push(linha(format!("auto-subno{k}-escala-nao-cabe"), "subscale", format!("{n} 0 1 1 1")));
        }
        match DELECOES_DE_SETOR.lock() {
            Ok(mut t) => t.extend(linhas),
            Err(_) => return,
        }
        aplicar_delecoes_do_setor(this, hash);
        let prefixo = format!("auto-subno{k}-");
        if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
            t.retain(|d| !d.modulo.starts_with(&prefixo));
        }
        let m1 = mundo();
        let dz = m1.p[2] - m0.p[2];
        let xy_iguais = (m1.p[0] - m0.p[0]).abs() < 0.01 && (m1.p[1] - m0.p[1]).abs() < 0.01;
        let dif = |sinal: f32| (0..4).map(|i| (m1.q[i] - sinal * m0.q[i]).abs()).fold(0.0f32, f32::max);
        let rot_igual = dif(1.0) < 1e-3 || dif(-1.0) < 1e-3;
        std::ptr::copy_nonoverlapping(antes.as_ptr(), e, tam);
        let restaurado = std::slice::from_raw_parts(e as *const u8, tam) == &antes[..];
        let ok = (dz - 1.0).abs() < 0.01 && xy_iguais && rot_igual && restaurado;
        crate::veredito_dev!(
            "[axl-subno-teste] VEREDITO setor {hash:#018x} {tipo}: Z no mundo da instância 0 {:.3} → {:.3} (Δ {dz:.3}), x/y iguais={xy_iguais}, orientação no mundo igual={rot_igual}; elemento devolvido={restaurado}; subno-ok={ok}",
            m0.p[2], m1.p[2]
        );
    }
}

// Parte 4f-1 (2026-09-12): mutações do transform do setup vindas do `.xl` (emissor v2: `pos x y z w`, `rot i j k r`, `scale x y z`). O setup de 0x90 B
// guarda position (Vector4) em `+0x00`, orientation (Quaternion) em `+0x10` e scale (Vector3) em `+0x20`; o tipo do nó é conferido como na deleção
// (vptr conhecido ou classe decodificada) e a quantidade de valores tem que bater. Teste automático (mesmo marcador da 4e-1): no 1º setor com
// `worldStaticMeshNode`, dois controles (scale com 2 valores; o tipo declarado num setup de outra classe) e o grupo com pos (Z+1), rot (a mesma) e
// scale (×1,5) no setup do static mesh, pelo caminho do `.xl`; o setup volta inteiro no mesmo passo.
#[cfg(feature = "devtools")]
pub(crate) static MUT_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_mutacao(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || MUT_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *mut u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups < 2 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let Some((i0, doador)) = doador_do_setor(setups_ptr as *const u8, setups) else { return };
    let outro = (0..setups).find(|i| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        no != doador && no >= 0x1_0000_0000 && no & 7 == 0 && (no as *const u64).read_unaligned() != crate::rebase(WORLDSTATICMESHNODE_VPTR_VM) as u64
    });
    let Some(i_outro) = outro else { return };
    MUT_TESTE_FEITO.store(true, Ordering::Relaxed);
    let e = setups_ptr.add(i0 * 0x90);
    let antes: Vec<u8> = std::slice::from_raw_parts(e as *const u8, 0x90).to_vec();
    let f = |k: usize| (e.add(k) as *const f32).read_unaligned();
    let pos = format!("{} {} {} {}", f(0x00), f(0x04), f(0x08) + 1.0, f(0x0C));
    let rot = format!("{} {} {} {}", f(0x10), f(0x14), f(0x18), f(0x1C));
    let escala = format!("{} {} {}", f(0x20) * 1.5, f(0x24) * 1.5, f(0x28) * 1.5);
    crate::veredito_dev!(
        "[axl-mut-teste] setor={hash:#018x} setups={setups} static mesh no índice {i0} (pos Z {:.2}, scale {:.2}), controle de tipo no índice {i_outro}",
        f(0x08),
        f(0x20)
    );
    let linha = |modulo: &str, indice: usize, op: &str, valores: String| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice,
        tipo: "worldStaticMeshNode".to_string(),
        op: op.to_string(),
        valores,
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![
            linha("auto-mut-contagem", i0, "scale", "1 1".to_string()),
            linha("auto-mut-tipo", i_outro, "scale", "1 1 1".to_string()),
            linha("auto-mut-transform", i0, "pos", pos),
            linha("auto-mut-transform", i0, "rot", rot),
            linha("auto-mut-transform", i0, "scale", escala),
        ]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-mut-"));
    }
    let z_depois = f(0x08);
    let escala_depois = f(0x20);
    std::ptr::copy_nonoverlapping(antes.as_ptr(), e, 0x90);
    let restaurado = std::slice::from_raw_parts(e as *const u8, 0x90) == &antes[..];
    crate::veredito_dev!(
        "[axl-mut-teste] VEREDITO setor {hash:#018x}: depois do .xl pos Z {z_depois:.2} e scale {escala_depois:.2}; setup devolvido={restaurado}"
    );
}

// Parte 4e-4b (2026-09-12): deleção genérica. Nó de classe sem tratamento próprio sai com o setup em Z -2000, scale 0 e `globalNodeID` 0, apontando
// pro `worldStaticMeshNode` doador do setor. A classe vem do vptr sem chamar nada: o `GetType` em `vt+8` é `adrp x8, pág; ldr x0, [x8, #off]; ret`
// (5 vtables medidas offline) e o global `pág+off` guarda a `CClass`, com o hash do nome em `+0x18`; o tipo do `.xl` entra pelo FNV-1a64 do nome, ou
// `#0x<hash>` nas linhas do teste automático. Só leitura, serve no thread de job.
#[cfg(feature = "devtools")]
pub(crate) static GENERICA_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static GENERICA_CENSO: std::sync::Mutex<Vec<(u64, u64)>> = std::sync::Mutex::new(Vec::new());

/// Endereço do global lido por um `GetType` de 3 instruções em `pc` (`adrp xN; ldr x0, [xN, #off]; ret`), ou `None` se o formato for outro.
pub(crate) fn global_do_gettype(pc: u64, i0: u32, i1: u32, i2: u32) -> Option<u64> {
    if i0 & 0x9F00_0000 != 0x9000_0000 || i1 & 0xFFC0_0000 != 0xF940_0000 || i2 != 0xD65F_03C0 {
        return None;
    }
    let rd = i0 & 0x1F;
    if i1 & 0x1F != 0 || (i1 >> 5) & 0x1F != rd {
        return None;
    }
    let imm21 = ((((i0 >> 5) & 0x7_FFFF) << 2) | ((i0 >> 29) & 3)) as i64;
    let pagina = ((pc & !0xFFF) as i64).wrapping_add(((imm21 << 43) >> 43) << 12) as u64;
    Some(pagina.wrapping_add(((i1 >> 10) & 0xFFF) as u64 * 8))
}

/// Hash do nome da classe dona do vptr `vt` (runtime), pelo `GetType` decodificado.
pub(crate) unsafe fn classe_hash_do_vptr(vt: u64) -> Option<u64> {
    let cls = classe_do_vptr(vt)?;
    let h = ((cls + 0x18) as *const u64).read_unaligned();
    (h != 0).then_some(h)
}

/// Hash do tipo de uma linha: FNV-1a64 do nome, ou o hash direto em `#0x<hex>`.
pub(crate) fn tipo_hash(tipo: &str) -> u64 {
    match tipo.strip_prefix("#0x") {
        Some(hex) => u64::from_str_radix(hex, 16).unwrap_or(0),
        None => bwms_primitives::fnv1a64(tipo.as_bytes()),
    }
}

// 4e-4b, teste automático (mesmo marcador da 4e-1): censo dos vptrs com classe decodificada (até 24, nomes resolvidos depois no tick) e, no 1º setor
// com doador e um nó de classe sem tratamento próprio, um controle (o índice do doador declarado com a classe do alvo) e a deleção genérica do alvo,
// pelo caminho do `.xl`. Um nó genérico pode ser de quest ou de comunidade: o teste relê e devolve o setup inteiro no mesmo passo.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_generica(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || GENERICA_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *mut u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let especiais: Vec<u64> = VPTRS_COM_TRATAMENTO.iter().map(|v| crate::rebase(*v) as u64).collect();
    let mut alvo: Option<(usize, u64, u64)> = None;
    for i in 0..setups {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 {
            continue;
        }
        let vt = (no as *const u64).read_unaligned();
        let Some(h) = classe_hash_do_vptr(vt) else { continue };
        if let Ok(mut c) = GENERICA_CENSO.try_lock() {
            if c.len() < 24 && !c.iter().any(|(v, _)| *v == vt) {
                c.push((vt, h));
            }
        }
        if alvo.is_none() && !especiais.contains(&vt) {
            alvo = Some((i, vt, h));
        }
    }
    let Some((i0, vt0, classe)) = alvo else { return };
    let vptr_de = |i: usize| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no >= 0x1_0000_0000 && no & 7 == 0 { (no as *const u64).read_unaligned() } else { 0 }
    };
    let Some(i_controle) = (0..setups).find(|&i| vptr_de(i) != 0 && vptr_de(i) != vt0) else { return };
    GENERICA_TESTE_FEITO.store(true, Ordering::Relaxed);
    let doador = match no_fantasma() {
        Ok(n) => n,
        Err(erro) => {
            crate::veredito_dev!("[axl-generica-teste] setor={hash:#018x}: {erro}");
            return;
        }
    };
    let e = setups_ptr.add(i0 * 0x90);
    let antes: Vec<u8> = std::slice::from_raw_parts(e as *const u8, 0x90).to_vec();
    crate::veredito_dev!(
        "[axl-generica-teste] setor={hash:#018x} setups={setups} alvo índice {i0} (vptr {:#x}, classe {classe:#x}), controle no índice {i_controle}",
        vm_do_jogo(vt0)
    );
    let tipo = format!("#{classe:#x}");
    let linha = |modulo: &str, indice: usize| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice,
        tipo: tipo.clone(),
        op: String::new(),
        valores: String::new(),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![linha("auto-generica-classe-errada", i_controle), linha("auto-generica-del", i0)]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-generica-"));
    }
    let no_depois = (e.add(0x50) as *const u64).read_unaligned();
    let z_depois = (e.add(0x08) as *const f32).read_unaligned();
    let gid_depois = (e.add(0x58) as *const u64).read_unaligned();
    let instancia_depois = (e.add(SETUP_INDICE_DE_INSTANCIA) as *const i32).read_unaligned();
    std::ptr::copy_nonoverlapping(antes.as_ptr(), e, 0x90);
    let restaurado = std::slice::from_raw_parts(e as *const u8, 0x90) == &antes[..];
    crate::veredito_dev!(
        "[axl-generica-teste] VEREDITO setor {hash:#018x}: nó do setup {i0} -> doador={} Z={z_depois} globalNodeID={gid_depois} instanceIndex={instancia_depois}, setup devolvido={restaurado}",
        no_depois == doador
    );
}

// Parte 4e-4a (2026-09-12): `worldAdvertisementNode` (vptr `0x107c7c8a8`) sai trocando o `node*` do setup (`+0x50`, gravado pelo bind do `PostLoad`
// sem refcount; o dono do nó é o Handle do setor) pelo [`no_fantasma`]. Sem ele ainda criado, o grupo é recusado.
#[cfg(feature = "devtools")]
pub(crate) static ANUNCIO_TESTE_FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) static ANUNCIO_SEM_DOADOR: AtomicU64 = AtomicU64::new(0);

/// 1º setup do setor cujo nó é `worldStaticMeshNode` (pelo vptr): `(índice, node*)`.
pub(crate) unsafe fn doador_do_setor(setups_ptr: *const u8, setups: usize) -> Option<(usize, u64)> {
    let vt = crate::rebase(WORLDSTATICMESHNODE_VPTR_VM) as u64;
    (0..setups).find_map(|i| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        (no >= 0x1_0000_0000 && no & 7 == 0 && (no as *const u64).read_unaligned() == vt).then_some((i, no))
    })
}

// 4e-4a, teste automático (mesmo marcador da 4e-1): no 1º setor com `worldAdvertisementNode`, entram na tabela em memória, em grupos
// separados, um controle (outro setup declarado como advertisement: vptr diferente) e a deleção do advertisement, aplicados pelo caminho do
// `.xl`; as linhas saem no fim.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_anuncio(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || ANUNCIO_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let alvo_vt = crate::rebase(WORLDADVERTISEMENTNODE_VPTR_VM) as u64;
    let Some(i0) = (0..setups).find(|i| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        no >= 0x1_0000_0000 && no & 7 == 0 && (no as *const u64).read_unaligned() == alvo_vt
    }) else {
        return;
    };
    let Some(i_controle) = (0..setups).find(|&i| {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        no >= 0x1_0000_0000 && no & 7 == 0 && (no as *const u64).read_unaligned() != alvo_vt
    }) else {
        return;
    };
    let doador = match no_fantasma() {
        Ok(n) => n,
        Err(erro) => {
            if ANUNCIO_SEM_DOADOR.fetch_add(1, Ordering::Relaxed) < 4 {
                crate::veredito_dev!("[axl-anuncio-teste] setor={hash:#018x}: advertisement no índice {i0}: {erro}");
            }
            return;
        }
    };
    ANUNCIO_TESTE_FEITO.store(true, Ordering::Relaxed);
    let no_antes = (setups_ptr.add(i0 * 0x90 + 0x50) as *const u64).read_unaligned();
    let controle_antes = (setups_ptr.add(i_controle * 0x90 + 0x50) as *const u64).read_unaligned();
    crate::veredito_dev!(
        "[axl-anuncio-teste] setor={hash:#018x} setups={setups} advertisement no índice {i0} (nó {no_antes:#x}), controle no índice {i_controle}, doador {doador:#x}"
    );
    let linha = |modulo: &str, indice: usize| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice,
        tipo: "worldAdvertisementNode".to_string(),
        op: String::new(),
        valores: String::new(),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![linha("auto-anuncio-tipo-errado", i_controle), linha("auto-anuncio-del", i0)]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-anuncio-"));
    }
    let no_depois = (setups_ptr.add(i0 * 0x90 + 0x50) as *const u64).read_unaligned();
    let controle_depois = (setups_ptr.add(i_controle * 0x90 + 0x50) as *const u64).read_unaligned();
    crate::veredito_dev!(
        "[axl-anuncio-teste] VEREDITO setor {hash:#018x}: nó do setup {i0} {no_antes:#x} -> {no_depois:#x} (doador={}), controle intacto={}",
        no_depois == doador,
        controle_depois == controle_antes
    );
}

// 4e-3, teste automático (mesmo marcador da 4e-1; o hook só lê memória, fora a escrita do teste): censo dos 8 primeiros nós de colisão (faixa de
// atores e `numActors`) e, no 1º com faixa válida e ≥ 2 atores, um controle (contagem errada) e o `subdel` do ator 0 pelo caminho do `.xl`. Um ator de
// colisão pode ser o chão debaixo do V: o teste relê o Z gravado e devolve o valor original no mesmo passo, antes de o motor criar os atores físicos.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_colisao(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || COLISAO_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let alvo_vt = crate::rebase(WORLDCOLLISIONNODE_VPTR_VM) as u64;
    let mut alvo: Option<(usize, *mut u8, usize)> = None;
    for i in 0..setups {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 || (no as *const u64).read_unaligned() != alvo_vt {
            continue;
        }
        let r = atores_do_no(no as *const u8);
        if COLISAO_CENSO.fetch_add(1, Ordering::Relaxed) < 8 {
            let desc = match &r {
                Ok((a, n)) => format!("{n} atores em {:p}", *a),
                Err(m) => m.clone(),
            };
            crate::veredito_dev!("[axl-colisao-teste] censo setor={hash:#018x} índice {i}: {desc}");
        }
        if let Ok((atores, n)) = r {
            if n >= 2 {
                alvo = Some((i, atores, n));
                break;
            }
        }
    }
    let Some((i0, atores, n0)) = alvo else { return };
    COLISAO_TESTE_FEITO.store(true, Ordering::Relaxed);
    let z0 = ator_z(atores, 0);
    crate::veredito_dev!(
        "[axl-colisao-teste] setor={hash:#018x} setups={setups} alvo índice {i0} com {n0} atores; Z do ator 0 = {:.2} m",
        z0 as f64 / 131072.0
    );
    let linha = |modulo: &str, valores: String| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldCollisionNode".to_string(),
        op: "subdel".to_string(),
        valores,
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![linha("auto-colisao-subcontagem", format!("{} 0", n0 + 7)), linha("auto-colisao-sub0", format!("{n0} 0"))]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-colisao-"));
    }
    let gravado = ator_z(atores, 0);
    (atores.add(ATOR_Z_OFF) as *mut i32).write_unaligned(z0);
    let restaurado = ator_z(atores, 0) == z0;
    crate::veredito_dev!(
        "[axl-colisao-teste] VEREDITO setor {hash:#018x}: Z do ator 0 depois do .xl = {gravado} (esperado {ATOR_Z_DELETADO}), restaurado={restaurado}"
    );
    // `subshape` (cobertura 1.27.0): no 1º ator do nó com 2+ shapes, a shape 0 passa ao preset `No Collision` pelo caminho do `.xl` e a shape 1 fica;
    // depois a shape volta ao preset de antes (o preset acrescentado e o buffer realocado ficam, sem efeito em shape nenhuma).
    let no = (setups_ptr.add(i0 * 0x90 + 0x50) as *const u64).read_unaligned() as *const u8;
    let fim_shape = |texto: String| crate::veredito_dev!("[axl-colisao-teste] VEREDITO-SHAPE setor {hash:#018x}: {texto}");
    let Ok((dados, _, l)) = listas_de_colisao(no) else {
        fim_shape("sem as listas do nó; shape-ok=false".into());
        return;
    };
    let ator = |d: *const u8, l: &ListasDeColisao, a: usize| d.add(l.offsets[0] as usize + a * ATOR_TAM);
    let Some(a) = (0..l.contagens[0] as usize).find(|&a| (ator(dados, &l, a).add(0x22) as *const u16).read_unaligned() >= 2) else {
        fim_shape("nenhum ator do nó com 2 shapes; shape-ok=false".into());
        return;
    };
    let inicio = (ator(dados, &l, a).add(0x20) as *const u16).read_unaligned() as usize;
    let preset_de = |d: *const u8, l: &ListasDeColisao, s: usize| d.add(l.offsets[1] as usize + s * 0x20 + SHAPE_PRESET_OFF).read();
    let (antes0, antes1, presets_antes) = (preset_de(dados, &l, inicio), preset_de(dados, &l, inicio + 1), l.contagens[COLISAO_LISTA_PRESETS]);
    let linha_shape = DelecaoDeNode {
        modulo: "auto-colisao-shape".to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldCollisionNode".to_string(),
        op: "subshape".to_string(),
        valores: format!("{} {a} 0", l.contagens[0]),
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.push(linha_shape),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-colisao-"));
    }
    let Ok((d2, _, l2)) = listas_de_colisao(no) else {
        fim_shape("listas ilegíveis depois do .xl; shape-ok=false".into());
        return;
    };
    let (depois0, depois1) = (preset_de(d2, &l2, inicio), preset_de(d2, &l2, inicio + 1));
    let nome = (d2.add(l2.offsets[COLISAO_LISTA_PRESETS] as usize + depois0 as usize * 8) as *const u64).read_unaligned();
    let sem_colisao = nome == crate::cname::cname(PRESET_SEM_COLISAO);
    d2.add(l2.offsets[1] as usize + inicio * 0x20 + SHAPE_PRESET_OFF).write(antes0);
    let ok = sem_colisao && depois1 == antes1 && preset_de(d2, &l2, inicio) == antes0;
    fim_shape(format!(
        "ator {a} (shapes a partir de {inicio}): shape 0 preset {antes0} → {depois0} (No Collision {sem_colisao}), shape 1 {antes1} → {depois1}; presets \
         {presets_antes} → {}; buffer realocado {}; restaurada; shape-ok={ok}",
        l2.contagens[COLISAO_LISTA_PRESETS],
        d2 != dados
    ));
}

// 4e-2, teste automático (mesmo marcador da 4e-1, só leitura de memória no `PostLoad`): no 1º setor com `worldInstancedDestructibleMeshNode` (pelo vptr)
// e buffer válido com ≥ 2 instâncias, entram na tabela em memória um controle (contagem errada) e o `subdel` da instância 0, aplicados pelo caminho do
// `.xl`; as linhas saem da tabela no fim.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_teste_destrutiveis(this: *mut c_void, hash: u64) {
    if !WSSECTOR_INST_TESTE.load(Ordering::Relaxed) || DEST_TESTE_FEITO.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups_ptr.is_null() || setups == 0 || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let alvo_vt = crate::rebase(WORLDINSTANCEDDESTRUCTIBLEMESHNODE_VPTR_VM) as u64;
    let mut alvo: Option<(usize, usize)> = None;
    for i in 0..setups {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 || (no as *const u64).read_unaligned() != alvo_vt {
            continue;
        }
        if let Ok((_, _, n)) = destrutiveis_do_no(no as *const u8) {
            if n >= 2 {
                alvo = Some((i, n));
                break;
            }
        }
    }
    let Some((i0, n0)) = alvo else { return };
    DEST_TESTE_FEITO.store(true, Ordering::Relaxed);
    crate::veredito_dev!("[axl-dest-teste] setor={hash:#018x} setups={setups} alvo índice {i0} com {n0} instâncias destrutíveis");
    let linha = |modulo: &str, valores: String| DelecaoDeNode {
        modulo: modulo.to_string(),
        setor: hash,
        esperado: setups,
        indice: i0,
        tipo: "worldInstancedDestructibleMeshNode".to_string(),
        op: "subdel".to_string(),
        valores,
    };
    match DELECOES_DE_SETOR.lock() {
        Ok(mut t) => t.extend(vec![linha("auto-dest-subcontagem", format!("{} 0", n0 + 7)), linha("auto-dest-sub0", format!("{n0} 0"))]),
        Err(_) => return,
    }
    aplicar_delecoes_do_setor(this, hash);
    if let Ok(mut t) = DELECOES_DE_SETOR.lock() {
        t.retain(|d| !d.modulo.starts_with("auto-dest-"));
    }
    crate::veredito_dev!("[axl-dest-teste] VEREDITO setor {hash:#018x}: grupos auto-dest-* aplicados pelo caminho do .xl (linhas [axl-streaming-xl] acima)");
}

/// Aplica as regras de `DELECAO_TESTE` que casam com o setor (só com o marcador). Loga o veredito de cada uma.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_delecao_teste(this: *mut c_void, hash: u64) {
    if !WSSECTOR_DELETE_TESTE.load(Ordering::Relaxed) {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *mut u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    let vptr = crate::rebase(WORLDSTATICMESHNODE_VPTR_VM) as u64;
    for (alvo, esperado, indice, regra) in DELECAO_TESTE {
        if alvo != hash {
            continue;
        }
        let veredito = if setups != esperado {
            format!("recusou:contagem (setor tem {setups}, regra espera {esperado})")
        } else if indice >= setups {
            format!("recusou:indice ({indice} >= {setups})")
        } else if setups_ptr.is_null() || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
            "recusou:setups ilegíveis".to_string()
        } else {
            let setup = setups_ptr.add(indice * 0x90);
            let no = (setup.add(0x50) as *const u64).read_unaligned();
            let vt = if no >= 0x1_0000_0000 && no & 7 == 0 { (no as *const u64).read_unaligned() } else { 0 };
            if vt != vptr {
                format!("recusou:tipo (vptr do nó {vt:#x} != worldStaticMeshNode)")
            } else {
                let e = setup.add(0x20) as *mut f32;
                let antes = (e.read_unaligned(), e.add(1).read_unaligned(), e.add(2).read_unaligned());
                for k in 0..3 {
                    e.add(k).write_unaligned(0.0);
                }
                let depois = (e.read_unaligned(), e.add(1).read_unaligned(), e.add(2).read_unaligned());
                format!("gravou scale ({:.3},{:.3},{:.3}) -> ({:.3},{:.3},{:.3})", antes.0, antes.1, antes.2, depois.0, depois.1, depois.2)
            }
        };
        crate::veredito_dev!("[axl-streaming-del] setor={hash:#018x} regra={regra} indice={indice} veredito={veredito}");
    }
}

/// Conta os setups cujo `node*` (`+0x50`) é `worldStaticMeshNode` e loga o 1º com o `scale` (`+0x20`), pros primeiros 16 setores que têm
/// algum, mais o `always_loaded_0` como controle (o CR2W dele não tem nenhum). Só leitura; o ponteiro do nó foi gravado pelo bind da original.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn setor_censo_static_mesh(this: *mut c_void, hash: u64) {
    // O censo só produz a linha de veredito do dev; no público ele pagaria 1 syscall + 1 passada pelos setups por PostLoad de setor.
    if !cfg!(feature = "devlog") {
        return;
    }
    let b = this as *const u8;
    let setups_ptr = (b.add(0x40) as *const u64).read_unaligned() as *const u8;
    let setups = ((b.add(0x48) as *const u32).read_unaligned() / 0x90) as usize;
    if setups == 0 || setups > 200_000 || setups_ptr.is_null() || !crate::memoria::is_readable(setups_ptr as *const c_void, setups * 0x90) {
        return;
    }
    let vptr = crate::rebase(WORLDSTATICMESHNODE_VPTR_VM) as u64;
    let (mut n, mut primeiro) = (0u32, usize::MAX);
    for i in 0..setups {
        let no = (setups_ptr.add(i * 0x90 + 0x50) as *const u64).read_unaligned();
        if no < 0x1_0000_0000 || no & 7 != 0 {
            continue;
        }
        if (no as *const u64).read_unaligned() == vptr {
            n += 1;
            if primeiro == usize::MAX {
                primeiro = i;
            }
        }
    }
    let controle = hash == SETOR_ALWAYS_LOADED_0_HASH;
    if (n > 0 || controle) && WSSECTOR_CENSO_LOGS.fetch_add(1, Ordering::Relaxed) < 16 {
        let (idx, escala) = if primeiro == usize::MAX {
            (-1i64, "-".to_string())
        } else {
            let e = setups_ptr.add(primeiro * 0x90 + 0x20);
            let (x, y, z) = ((e as *const f32).read_unaligned(), (e.add(4) as *const f32).read_unaligned(), (e.add(8) as *const f32).read_unaligned());
            (primeiro as i64, format!("({x:.3},{y:.3},{z:.3})"))
        };
        crate::veredito_dev!("[axl-streaming-censo] setor={hash:#018x} setups={setups} static-mesh={n} 1o-setup={idx} scale={escala}");
    }
}

// SHIM NAKED, e o porquê custou um crash. A 1ª versão usava replacement em Rust
// (`extern "C" fn(*mut c_void, u64)`) chamando a original — e MATOU o jogo:
// `KERN_PROTECTION_FAILURE`, com o `.ips` nomeando `resloader_findtoken_probe`. Causa: os 3
// devolvem `SharedPtr` de 16 bytes, ou seja **por `x8` (sret)**, e um shim `extern "C"` do Rust não
// preserva `x8`. A original escreveu o retorno num ponteiro lixo.
//
// O shim naked resolve por construção: usa só x9–x13 (scratch, fora dos registradores de argumento
// x0–x7 e do x8), grava no ring e faz `br` — tail-call que deixa x0/x1/x8/x30 INTACTOS.
//
// Ring de 64 entradas por função. `idx` cresce sempre; a leitura usa `idx & 63`.
pub(crate) const RESLOADER_RING_N: usize = 64;

#[repr(C)]
pub(crate) struct ResLoaderRing {
    pub(crate) idx: u64,
    pub(crate) buf: [u64; RESLOADER_RING_N],
    /// O `this` da última chamada — o ponteiro do `ResourceLoader`. Gravado pelo shim em
    /// `+8 + 64*8 = 520`; se mudar o tamanho do `buf`, o deslocamento no asm muda junto.
    pub(crate) ultimo_this: u64,
}

pub(crate) static MAPPIN_EMPURRADOS: AtomicU64 = AtomicU64::new(0);

pub(crate) const QUESTPHASE_RAIZ_PATH: u64 = 0x8464_2c97_f5c7_1116;

pub(crate) static QUESTPHASE_CARGAS: AtomicU64 = AtomicU64::new(0);

/// Liga o registro das cargas (parte 35c-1); com a tabela de patches o hook instala sem ele (35c-2d-3).
pub(crate) static QUESTPHASE_OBSERVAR: AtomicBool = AtomicBool::new(false);

/// Mesma regra de marcador do `bwms_hook_enabled` (que é local a `install_postload_hooks`).
pub(crate) fn bwms_hook_enabled_global(individual: &str) -> bool {
    if !cfg!(feature = "devtools") {
        return false;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&format!("{home}/.bwms-hook")).exists()
        || std::path::Path::new(&format!("{home}/.bwms-hook-{individual}")).exists()
}

pub(crate) static mut RING_FINDTOKEN: ResLoaderRing = ResLoaderRing { idx: 0, buf: [0; RESLOADER_RING_N], ultimo_this: 0 };

// Teste 11 (`~/.bwms-hook-a11-teste`): o tick tira e recoloca a roupa de cima do V 3 vezes pelo `bwms-a11-teste.reds` (`DelayCallback` do motor, como o
// ciclo do `#29`). A fase 1 observa; na 2, cada descritor completo visto na 1 ganha override pra um alias `bwms\a11\<recurso>.app` com `resource.link`
// de volta pro recurso (o motor carrega o mesmo `.app`); a 3 tira os overrides. Confere: troca nos 2 hooks, o alias resolvido pelo link, o alias
// voltando como descritor velho e o recurso original de volta na 3.
pub(crate) static A11_TESTE: AtomicBool = AtomicBool::new(false);

/// Fase do teste: 0 carga, 1–3 ciclos, 4 fim.
pub(crate) static A11_FASE: AtomicU64 = AtomicU64::new(0);

/// Chamadas por hook (`ChangeAppearance`, `ChangeAppearances`) e fase.
pub(crate) static A11_CHAMADAS: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];

pub(crate) struct A11Visto {
    pub(crate) tipo: u8,
    pub(crate) fase: u8,
    pub(crate) dono: u64,
    pub(crate) velho: Vec<[u64; 2]>,
    pub(crate) novo: Vec<[u64; 2]>,
}

pub(crate) static A11_VISTOS: std::sync::Mutex<Vec<A11Visto>> = std::sync::Mutex::new(Vec::new());

/// Trocas por hook e fase (sem o teto das anotações).
pub(crate) static A11_TROCAS: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];

/// Pares com override registrado no `on_load` a partir de `~/.bwms-a11-pares.txt`: `[recurso, aparência, alias]`.
pub(crate) static A11_PRE: std::sync::Mutex<Vec<[u64; 3]>> = std::sync::Mutex::new(Vec::new());

pub(crate) const A11_PARES_ARQUIVO: &str = ".bwms-a11-pares.txt";

/// Alias de teste de um recurso: `bwms\a11\<recurso>.app`, ligado de volta ao recurso pelo `resource.link`.
pub(crate) fn a11_alias(recurso: u64) -> u64 {
    resource_path_hash(&format!("bwms\\a11\\{recurso:016x}.app"))
}

/// Pares `(recurso, aparência)` gravados pelo teste anterior: uma linha `recurso_hex nome` cada.
pub(crate) fn a11_pares_gravados() -> Vec<(u64, u64)> {
    let Ok(home) = std::env::var("HOME") else { return Vec::new() };
    let Ok(texto) = std::fs::read_to_string(format!("{home}/{A11_PARES_ARQUIVO}")) else { return Vec::new() };
    texto
        .lines()
        .filter_map(|l| {
            let (r, nome) = l.trim().split_once(' ')?;
            let r = u64::from_str_radix(r.trim_start_matches("0x"), 16).ok()?;
            let nome = nome.trim();
            (r != 0 && !nome.is_empty()).then(|| (r, crate::cname::cname(nome)))
        })
        .take(ALIASES_DO_TESTE_DE_OVERRIDE.len())
        .collect()
}

pub(crate) unsafe fn a11_descritores(p: *const u64, n: usize) -> Vec<[u64; 2]> {
    (0..n.min(8)).map(|i| (p.add(i * 2) as *const [u64; 2]).read_unaligned()).collect()
}

/// Anotação do teste com os descritores como o motor passou (antes do override). Na carga, só as 32 primeiras.
pub(crate) fn a11_anotar(tipo: u8, dono: u64, velho: Vec<[u64; 2]>, novo: Vec<[u64; 2]>, trocados: usize) {
    let fase = A11_FASE.load(Ordering::Relaxed).min(3);
    A11_CHAMADAS[usize::from(tipo)][fase as usize].fetch_add(1, Ordering::Relaxed);
    A11_TROCAS[usize::from(tipo)][fase as usize].fetch_add(trocados as u64, Ordering::Relaxed);
    if let Ok(mut v) = A11_VISTOS.lock() {
        if v.len() < 512 && (fase > 0 || v.len() < 32) {
            v.push(A11Visto { tipo, fase: fase as u8, dono, velho, novo });
        }
    }
}

#[cfg(any(test, feature = "devtools"))]
pub(crate) static A47_GET_CHAMADAS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

#[cfg(any(test, feature = "devtools"))]
pub(crate) static A47_GET_DESCRITORES: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

#[cfg(any(test, feature = "devtools"))]
pub(crate) static A47_GET_TROCADOS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// Amostra das primeiras saídas com troca: (hook, grupo, fpp, descritor trocado).
#[cfg(feature = "devtools")]
pub(crate) static A47_GET_AMOSTRA: std::sync::Mutex<Vec<(usize, u64, u64, [u64; 2])>> = std::sync::Mutex::new(Vec::new());

#[cfg(feature = "devtools")]
pub(crate) static A47_INIT_CHAMADAS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Partes (0 cabeça, 1 corpo, 2 braços) vistas no app e no switcher, bit por parte.
#[cfg(feature = "devtools")]
pub(crate) static A47_INIT_PARTES: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Maior tamanho do `SortedUniqueArray<CName>` de opções do estado visto no app e no morph.
#[cfg(feature = "devtools")]
pub(crate) static A47_INIT_ESTADO_MAX: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

#[cfg(feature = "devtools")]
pub(crate) static A47_TESTE: AtomicBool = AtomicBool::new(false);

/// Tamanho de um `SortedUniqueArray` (mesmo cabeçalho do `DynArray`).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn a47_tamanho(arr: u64) -> u64 {
    if arr == 0 || !crate::memoria::is_readable(arr as *const c_void, 0x10) {
        return 0;
    }
    ((arr as *const u8).add(0xc) as *const u32).read_unaligned() as u64
}

/// Chamadas do `Initialize` e do `Uninitialize`.
#[cfg(feature = "devtools")]
pub(crate) static A47_CICLO_CHAMADAS: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Ritmo de um teste de script em passos armado pelo canal de dev: o passo corrente (0 = parado) é chamado no máximo uma vez por intervalo de
/// relógio, e o retorno do script decide o próximo (0 ou negativo encerra). O relógio vem de quem chama: com o menu aberto a simulação para, e os
/// testes de unidade passam o tempo à mão.
#[cfg(any(test, feature = "devtools"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RitmoDePassos {
    passo: i32,
    ultimo_ms: Option<u64>,
}

#[cfg(any(test, feature = "devtools"))]
impl RitmoDePassos {
    pub(crate) const PARADO: Self = Self { passo: 0, ultimo_ms: None };

    /// Recomeça do passo 1, com a 1ª chamada liberada na hora.
    pub(crate) fn armar(&mut self) {
        *self = Self { passo: 1, ultimo_ms: None };
    }

    /// Há passo a chamar, agora ou depois do intervalo.
    pub(crate) fn armado(&self) -> bool {
        self.passo > 0
    }

    /// O passo a chamar em `agora_ms`; nenhum parado ou antes de `intervalo_ms` desde a última chamada.
    pub(crate) fn vez(&self, agora_ms: u64, intervalo_ms: u64) -> Option<i32> {
        if self.passo <= 0 {
            return None;
        }
        match self.ultimo_ms {
            Some(t) if agora_ms.saturating_sub(t) < intervalo_ms => None,
            _ => Some(self.passo),
        }
    }

    /// Anota a chamada feita em `agora_ms` com o retorno do script: o próximo passo, ou parado com 0 ou negativo (o -99 da chamada que falhou também).
    pub(crate) fn registrar(&mut self, agora_ms: u64, retorno: i32) {
        self.ultimo_ms = Some(agora_ms);
        self.passo = retorno.max(0);
    }
}

/// Clique na aba Cheats das Configurações: o comando `abacheats` do canal arma aqui e o `selftest::aba_cheats_tick` chama os passos do
/// `bwms-aba-cheats-teste.reds`. O estado mora fora do `selftest` para o terminal não chamar o módulo de testes.
#[cfg(feature = "devtools")]
pub(crate) static ABA_CHEATS: std::sync::Mutex<RitmoDePassos> = std::sync::Mutex::new(RitmoDePassos::PARADO);

/// Ticks do executor com o teste armado desde a última chamada dos passos: `[0]` fora da thread principal, `[1]` nela. É a medida, no log de cada
/// passo, de onde o tick roda com o menu aberto.
#[cfg(feature = "devtools")]
pub(crate) static ABA_CHEATS_TICKS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

#[cfg(feature = "devtools")]
pub(crate) fn aba_cheats_armar() {
    if let Ok(mut r) = ABA_CHEATS.lock() {
        r.armar();
    }
    for t in &ABA_CHEATS_TICKS {
        t.store(0, Ordering::Relaxed);
    }
    crate::veredito_dev!("[aba] armado: passo 1 no próximo tick do executor em gameplay, só com a trava de save confirmada");
}

#[cfg(test)]
mod testes_de_faixa {
    #[test]
    fn faixa_que_passa_1_instancia_do_fim_e_recusada() {
        // 676 instâncias em 32448 B (boot 62): a faixa até o fim cabe, 1 a mais não.
        assert!(super::instancias_cabem(600, 76, 32448));
        assert!(!super::instancias_cabem(600, 77, 32448));
        assert!(!super::instancias_cabem(32448, 1, 32448));
    }
}

#[cfg(test)]
mod testes_de_eventos_ativos {
    use crate::cname::cname;
    use crate::tipos_do_motor::ativar_hooks_do_evento;
    use crate::tipos_do_motor::CALLBACKS;
    use crate::tipos_do_motor::STATIC_CALLBACKS;

    // Registro cujo hook não sobe não vira "instalado" em lugar nenhum. `Bwms/TesteAtivos` não tem hook no BWMS, então o
    // `ativar_hooks_do_evento` do registro não instala nada: o mesmo resultado físico de uma instalação que falhou. Offline é a única falha que não passa
    // pelo `rebase`, que aqui varreria o binário de teste atrás das assinaturas. O número segue só a tabela de ouvintes: sobe no registro de instância e
    // no estático e desce em qualquer saída da tabela (`Unregister` do handler, `RunMode.Once`, varredura do `Session/End`), não só no `UnregisterCallback`.
    #[test]
    fn eventos_ativos_segue_os_ouvintes_mesmo_sem_hook_instalado() {
        let _serie = crate::tipos_do_motor::TESTES_DE_CALLBACKS_EM_SERIE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ev = cname("Bwms/TesteAtivos");
        let base = super::eventos_ativos();
        CALLBACKS.lock().expect("trava de CALLBACKS").push((ev, 1, 2, 0));
        ativar_hooks_do_evento(ev);
        assert_eq!(super::eventos_ativos(), base + 1, "1 evento com ouvinte e nenhum hook no ar");
        CALLBACKS.lock().expect("trava de CALLBACKS").push((ev, 1, 3, 0));
        assert_eq!(super::eventos_ativos(), base + 1, "2 ouvintes do mesmo evento são 1 evento");
        CALLBACKS.lock().expect("trava de CALLBACKS").retain(|(e, _, _, _)| *e != ev);
        assert_eq!(super::eventos_ativos(), base, "saída da tabela fora do UnregisterCallback desconta");
        STATIC_CALLBACKS.lock().expect("trava de STATIC_CALLBACKS").push((ev, "BwmsTesteAtivos".to_string(), "OnEvento".to_string()));
        ativar_hooks_do_evento(ev);
        assert_eq!(super::eventos_ativos(), base + 1, "callback estático também é ouvinte");
        STATIC_CALLBACKS.lock().expect("trava de STATIC_CALLBACKS").retain(|(e, _, _)| *e != ev);
        assert_eq!(super::eventos_ativos(), base);
    }
}

#[cfg(test)]
mod testes_de_ritmo {
    use super::RitmoDePassos;

    #[test]
    fn parado_nao_chama_e_armado_chama_o_passo_1_na_hora() {
        let mut r = RitmoDePassos::PARADO;
        assert!(!r.armado());
        assert_eq!(r.vez(0, 3000), None);
        assert_eq!(r.vez(u64::MAX, 3000), None);
        r.armar();
        assert!(r.armado());
        assert_eq!(r.vez(0, 3000), Some(1));
    }

    #[test]
    fn segue_o_retorno_do_script_com_o_intervalo_entre_chamadas() {
        let mut r = RitmoDePassos::PARADO;
        r.armar();
        r.registrar(10_000, 2);
        assert_eq!(r.vez(12_999, 3000), None);
        assert_eq!(r.vez(13_000, 3000), Some(2));
        // Espera do script: devolve o mesmo passo e é chamado de novo depois do intervalo.
        r.registrar(13_000, 2);
        assert!(r.armado(), "armado também entre uma chamada e outra");
        assert_eq!(r.vez(16_000, 3000), Some(2));
        r.registrar(16_000, 50);
        assert_eq!(r.vez(19_000, 3000), Some(50));
    }

    #[test]
    fn retorno_zero_negativo_ou_chamada_que_falhou_para() {
        for fim in [0, -1, -99] {
            let mut r = RitmoDePassos::PARADO;
            r.armar();
            r.registrar(0, fim);
            assert!(!r.armado(), "retorno {fim}");
            assert_eq!(r.vez(60_000, 3000), None, "retorno {fim}");
        }
    }

    #[test]
    fn rearmar_no_meio_recomeca_do_1_sem_esperar() {
        let mut r = RitmoDePassos::PARADO;
        r.armar();
        r.registrar(5_000, 7);
        r.armar();
        assert_eq!(r.vez(5_001, 3000), Some(1));
    }

    #[test]
    fn relogio_atras_da_ultima_chamada_nao_libera() {
        let mut r = RitmoDePassos::PARADO;
        r.armar();
        r.registrar(10_000, 2);
        assert_eq!(r.vez(9_000, 3000), None);
    }
}
