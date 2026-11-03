//! Corpo e rosto: o menu de personalização do personagem (opções de mod mescladas nas listas do jogo: cabelo, maquiagem, corpo), os eventos de personalização e o corpo inteiro em primeira pessoa. A câmera em terceira pessoa está em `camera`.

#![allow(dead_code)]

use bwms_primitives::targets::{CamposDoAlvoDeEntidade, CamposDoEventoDeEntidade};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::cname::cname;
use crate::rtti::{self, Registry};
use crate::register::*;
use crate::entidades::*;
use crate::aparencia::*;
use crate::ganchos::*;
use crate::diagnostico::CUST_SO_MONTADAS;
#[cfg(feature = "devtools")]
use crate::diagnostico::{PUPPETSTATE_MIRROR_PTR, PUPPETSTATE_RECOVER_AT};
// O teste do `#11`/`#47` e a contagem que só o selftest lê.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    a11_alias, a11_pares_gravados, a47_tamanho, A47_CICLO_CHAMADAS, A47_GET_AMOSTRA, A47_INIT_CHAMADAS, A47_INIT_ESTADO_MAX, A47_INIT_PARTES, A47_TESTE,
    CUST_MARCAS,
};
#[cfg(any(test, feature = "devtools"))]
use crate::diagnostico::{A47_GET_CHAMADAS, A47_GET_DESCRITORES, A47_GET_TROCADOS};
use crate::tipos_do_motor::*;
use crate::arquivos_do_jogo::*;
use crate::entrada::*;
use crate::veiculos::*;
use crate::interface::*;
use crate::npcs::*;

/// `BwmsTppState() -> Int32` — Skill 1 (VER-O-V, câmera 3ª pessoa autônoma). Lê `~/.bwms-tppcam`:
/// "1" = ligar câmera 3ª pessoa, senão 0. O `bwms-tppcam.reds` (poller DelayCallback) consulta isto
/// a cada 0.25s em gameplay e, na transição, faz `player.QueueEvent(new ActivateTPPRepresentationEvent())`
/// (o `new` do redscript cuida do Handle/refcount — o lado Rust não conseguiria com segurança).
/// O assistente controla o estado ESCREVENDO o marcador pelo canal/bash (sem HID/Acessibilidade).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_tpp_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-tppcam")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| if s.trim() == "1" { 1u64 } else { 0 })
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}





/// `DynamicEntityTarget::Matches` (2026-08-12) — lê o `EntityID` (hash `u64`) do
/// campo REAL `Entity::entityID`, em `+0x48`. É campo de DADO, não
/// slot de vtable — não sofre o shift Itanium Mac=Windows+0x08 (regra já estabelecida neste projeto
/// pra offsets de campo, ex. `Vehicle::RecordID@+0x638` acima; o do puppet é +0x25C no Mac, não o +0x260 do header). `Red::
/// EntityID` é `{hash: u64}` (já confirmado/usado pelo
/// `DynamicEntitySystem`/`#73` — mesma chave `id` que `dynent_tags()` já indexa) — leitura direta
/// de 8 bytes, sem RTTI/vtable nenhum (mais barata e mais segura que `categoria_e_record`, que resolve a classe pelo RTTI).
pub(crate) unsafe fn entity_id_hash(entity_ptr: *mut c_void) -> Option<u64> {
    read_u64_field(entity_ptr, 0x48)
}

/// Deriva o ponteiro do COMPONENTE real associado a um disparo, a partir do 1º arg (o objeto
/// evento) — funciona pra eventos construídos via `make_entitycomponentevent_arg` ("Component/
/// Toggle", hoje o único que registra o componente em `ENTITYCOMPONENT_STATES`). Mesmo formato de
/// `event_entity_ptr`, mas indexando a tabela do `#9` (`EntityComponentEvent.GetComponent`, já
/// fechado 2026-08-11) em vez de `ENTITYLIFECYCLE_STATES`.
pub(crate) unsafe fn event_component_ptr(args: &[rtti::Arg]) -> Option<*mut c_void> {
    let (inst, _refc) = match args.first() {
        Some(rtti::Arg::Handle(i, r)) => (*i, *r),
        _ => return None,
    };
    ENTITYCOMPONENT_STATES
        .lock()
        .ok()
        .and_then(|states| states.iter().rev().find(|(p, ..)| *p == inst as usize).map(|(_, c, _)| *c as *mut c_void))
}

/// Coleta o par de critérios `(key, action)` de todos os `InputTarget` em `target_ptrs` — busca
/// por VALOR em `INPUTTARGET_STATES`, nunca dereferencia o objeto C++ (mesma técnica dos outros 3
/// filtros). `IK_None`/`IACT_None`, o não-setado do filtro, saem como `None`. Um alvo
/// `.Axis(axis, threshold)` entra com ação `None`: o 4º campo dele é
/// `threshold` em bits, NÃO um `action` — ler aquilo como enum compararia lixo (ver
/// `InputTargetCriterion`). O alvo de eixo não filtra por ação.
pub(crate) unsafe fn inputtarget_criteria(target_ptrs: &[usize]) -> Vec<(Option<u32>, Option<u32>)> {
    if target_ptrs.is_empty() {
        return Vec::new();
    }
    let states = match INPUTTARGET_STATES.lock() {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    // O enum vai ao núcleo como os 32 bits do `int32_t`, como o `alvo_casa` passa a tecla e a ação do evento.
    let setado = |v: i64, nao_setado: i32| (v != nao_setado as i64).then_some(v as u32);
    target_ptrs
        .iter()
        .filter_map(|p| {
            states.iter().find(|(sp, ..)| *sp == *p).map(|(_, kind, key, extra)| match kind {
                InputTargetCriterion::Key => (setado(*key, IK_NONE), setado(*extra, IACT_NONE)),
                InputTargetCriterion::Axis => (setado(*key, IK_NONE), None),
            })
        })
        .collect()
}

/// Deriva o par `(key, action)` do evento "Input/Key"/"Input/KeyUp" a partir do 1º arg — lookup
/// direto em `KEYINPUT_STATES` (mesma tabela que os getters `KeyInputEvent.GetKey()`/`GetAction()`
/// já leem), zero dereferência de C++ real. Uma passada/um lock pros 2 campos.
pub(crate) unsafe fn event_input_key_action(args: &[rtti::Arg]) -> (Option<i64>, Option<i64>) {
    let inst = match args.first() {
        Some(rtti::Arg::Handle(i, _)) => *i,
        _ => return (None, None),
    };
    KEYINPUT_STATES
        .lock()
        .ok()
        .and_then(|states| {
            // Do mais novo para o mais velho, como os getters: o endereço do evento pode ter sido reaproveitado.
            states
                .iter()
                .rev()
                .find(|(p, ..)| *p == inst as usize)
                .map(|(_, action, key, ..)| (Some(*key as i64), Some(*action as i64)))
        })
        .unwrap_or((None, None))
}

/// Recurso do `Resource/Load` em despacho (o objeto que o `GetResource` entrega), pelo evento do 1º arg.
pub(crate) unsafe fn recurso_do_evento(args: &[rtti::Arg]) -> Option<u64> {
    let Some(rtti::Arg::Handle(inst, _)) = args.first() else { return None };
    RESOURCEEVENT_RECURSOS.lock().ok()?.iter().rev().find(|e| e.0 == *inst as usize).map(|e| e.1[0]).filter(|r| *r != 0)
}

/// Critérios `Path` conferidos no despacho e quantos tinham padrão: mede se o ramo de padrão vive fora do teste c6 (o `bateria` reporta).
#[cfg(feature = "devtools")]
pub(crate) static CAMINHO_CASA_CRITERIOS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
pub(crate) static CAMINHO_CASA_PADRAO: AtomicU64 = AtomicU64::new(0);

/// Caminho do evento contra o do alvo. Com texto registrado com `*` ou `regex:`, o padrão sobre o texto do caminho do evento: sem texto não casa
/// (padrão não decide sobre hash), e padrão recusado também não. Sem padrão, hash igual.
// TODO: o padrão é compilado a cada evento. Guardar o `CompiledResourceTarget` por hash só depois que `CAMINHO_CASA_PADRAO` (comando `bateria`)
// mostrar o ramo vivo num boot sem o teste c6, que hoje é o único a registrar texto de padrão.
pub(crate) fn caminho_casa(alvo: u64, evento: u64) -> bool {
    #[cfg(feature = "devtools")]
    CAMINHO_CASA_CRITERIOS.fetch_add(1, Ordering::Relaxed);
    match caminho_texto(alvo).and_then(|t| bwms_primitives::restarget::CompiledResourceTarget::compilar(&t)) {
        Some(padrao) => {
            #[cfg(feature = "devtools")]
            CAMINHO_CASA_PADRAO.fetch_add(1, Ordering::Relaxed);
            caminho_texto(evento).is_some_and(|t| padrao.casa(&t) == Some(true))
        }
        None => alvo == evento,
    }
}

/// Deriva o `path` (hash `ResourcePath`) do evento "Resource/Load" a partir do 1º arg — lookup
/// direto em `RESOURCEEVENT_STATES` (a MESMA tabela que `ResourceEvent.GetPath()` já lê; populada
/// só por `make_resourceevent_arg`, ligado ao hook REAL `resource.link`, path genuíno, não
/// fabricado).
pub(crate) unsafe fn event_resource_path(args: &[rtti::Arg]) -> Option<u64> {
    let (inst, _refc) = match args.first() {
        Some(rtti::Arg::Handle(i, r)) => (*i, *r),
        _ => return None,
    };
    RESOURCEEVENT_STATES
        .lock()
        .ok()
        .and_then(|states| states.iter().rev().find(|(p, _)| *p == inst as usize).map(|(_, path)| *path))
}

/// Alvos `inkWidgetTarget` entre os alvos do handler, como `(libraryPath, itemName, controllerType)`; valor 0 é o não-setado da fábrica
/// e sai `None`.
pub(crate) fn inkwidgettarget_alvos(target_ptrs: &[usize]) -> Vec<(Option<u64>, Option<u64>, Option<u64>)> {
    let Ok(states) = INKWIDGETTARGET_STATES.lock() else { return Vec::new() };
    let setado = |v: u64| (v != 0).then_some(v);
    target_ptrs
        .iter()
        .filter_map(|p| {
            states.iter().find(|s| s.0 == *p).map(|s| match s.1 {
                InkWidgetTargetCriterion::Library => (setado(s.2), setado(s.3), None),
                InkWidgetTargetCriterion::Controller => (None, None, setado(s.2)),
            })
        })
        .collect()
}

/// `(libraryPath, itemName, instância)` do evento de spawn de widget, pela tabela que o `inkWidgetSpawnEvent` lê.
pub(crate) unsafe fn event_widget_spawn(args: &[rtti::Arg]) -> Option<(u64, u64, usize)> {
    let inst = match args.first() {
        Some(rtti::Arg::Handle(i, _)) => *i,
        _ => return None,
    };
    INKWIDGETSPAWN_STATES
        .lock()
        .ok()
        .and_then(|states| states.iter().rev().find(|(p, ..)| *p == inst as usize).map(|(_, lib, item, instancia)| (*lib, *item, *instancia)))
}

/// Deriva o ponteiro da entidade REAL envolvida num disparo, a partir do 1º arg (o objeto evento),
/// pela tabela de estado dele (`entidade_do_evento`): a entidade do ciclo de vida
/// (`ENTITYLIFECYCLE_STATES`), a dona do componente no `Component/Toggle` (`ENTITYCOMPONENT_STATES`)
/// ou o veículo no `Vehicle/ToggleAuxLights` (`VEHICLELIGHT_STATES`). Evento sem entidade
/// (`Input/Key`, `Session/*`) devolve `None` — o filtro de RecordID então nunca bloqueia esses
/// eventos (fail-open).
pub(crate) unsafe fn event_entity_ptr(args: &[rtti::Arg]) -> Option<*mut c_void> {
    let Some(rtti::Arg::Handle(inst, _)) = args.first() else { return None };
    let entidade = entidade_do_evento(*inst as usize);
    (entidade != 0).then_some(entidade as *mut c_void)
}

/// Tipo do evento despachado, pela tabela de estado em que o objeto do 1º argumento está.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TipoDoEvento {
    Builder,
    Ciclo,
    Componente,
    LuzDoVeiculo,
    Tecla,
    Recurso,
    SpawnDeWidget,
    Outro,
}

/// Família de um alvo, pela tabela da fábrica que o criou.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FamiliaDoAlvo {
    Entidade,
    EntidadeDinamica,
    EntidadeEstatica,
    Componente,
    Tecla,
    Recurso,
    Widget,
}

/// Eventos que cada família de alvo sabe filtrar; os outros passam pelo alvo sem consulta.
pub(crate) fn familia_suporta(f: FamiliaDoAlvo, t: TipoDoEvento) -> bool {
    use TipoDoEvento::*;
    match f {
        FamiliaDoAlvo::Entidade => matches!(t, Builder | Ciclo | Componente | LuzDoVeiculo),
        FamiliaDoAlvo::Componente => t == Componente,
        FamiliaDoAlvo::EntidadeDinamica | FamiliaDoAlvo::EntidadeEstatica => t == Ciclo,
        FamiliaDoAlvo::Tecla => t == Tecla,
        FamiliaDoAlvo::Recurso => t == Recurso,
        FamiliaDoAlvo::Widget => t == SpawnDeWidget,
    }
}

pub(crate) unsafe fn tipo_do_evento(args: &[rtti::Arg]) -> TipoDoEvento {
    let Some(rtti::Arg::Handle(inst, _)) = args.first() else { return TipoDoEvento::Outro };
    let p = *inst as usize;
    if EBE_ESTADOS.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::Builder
    } else if ENTITYLIFECYCLE_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::Ciclo
    } else if ENTITYCOMPONENT_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::Componente
    } else if VEHICLELIGHT_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::LuzDoVeiculo
    } else if KEYINPUT_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::Tecla
    } else if RESOURCEEVENT_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::Recurso
    } else if INKWIDGETSPAWN_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        TipoDoEvento::SpawnDeWidget
    } else {
        TipoDoEvento::Outro
    }
}

/// Os alvos são retidos pra sempre pelas fábricas, então um endereço nunca aparece em 2 tabelas.
pub(crate) fn familia_do_alvo(p: usize) -> Option<FamiliaDoAlvo> {
    if ENTITYTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) || ENTITYTARGET_DEFINITION_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::Entidade)
    } else if DYNAMICENTITYTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::EntidadeDinamica)
    } else if STATICENTITYTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::EntidadeEstatica)
    } else if COMPONENTTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::Componente)
    } else if INPUTTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::Tecla)
    } else if RESOURCETARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::Recurso)
    } else if INKWIDGETTARGET_STATES.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p)) {
        Some(FamiliaDoAlvo::Widget)
    } else {
        None
    }
}

pub(crate) fn campos_do_alvo_de_entidade(p: usize) -> CamposDoAlvoDeEntidade {
    // Valor 0 é o não-setado de cada fábrica (`EntityTarget.ID(0)`, `.Definition(0, 0)`): não vira filtro de valor 0.
    let setado = |v: u64| (v != 0).then_some(v);
    let mut c = CamposDoAlvoDeEntidade::default();
    if let Ok(s) = ENTITYTARGET_STATES.lock() {
        for (_, criterio, v) in s.iter().filter(|e| e.0 == p) {
            match criterio {
                EntityTargetCriterion::Id => c.entity_id = setado(*v),
                EntityTargetCriterion::Type => c.tipo = setado(*v),
                EntityTargetCriterion::RecordId => c.record_id = setado(*v),
                EntityTargetCriterion::Template => c.template_path = setado(*v),
                EntityTargetCriterion::Appearance => c.appearance_name = setado(*v),
            }
        }
    }
    if let Some((_, caminho, definicao)) = ENTITYTARGET_DEFINITION_STATES.lock().ok().and_then(|s| s.iter().find(|e| e.0 == p).copied()) {
        c.appearance_path = setado(caminho);
        c.definition_name = setado(definicao);
    }
    c
}

/// Valor do critério `Type` com nome que não resolve para classe. Fica fora da faixa de ponteiro (`rtti::sane`), então o `class_is_kind_of` do
/// despacho nunca casa e o alvo bloqueia todo evento; 0 continua sendo o critério não-setado.
pub(crate) const CLASSE_NAO_RESOLVIDA: u64 = u64::MAX;

/// Valor que as fábricas `EntityTarget.Type` e `ResourceTarget.Type` gravam: a classe de `nome` resolvida uma vez na fábrica, e o despacho
/// compara ponteiro por evento. `n"None"` (hash 0) fica 0, o não-setado. Nome que não resolve vira `CLASSE_NAO_RESOLVIDA` e bloqueia: guardar
/// nulo desligaria o filtro e o handler dispararia para toda entidade (F1-02, selftests c6 e 134). O veredito sai 1× por alvo, porque a
/// fábrica roda uma vez por alvo.
pub(crate) unsafe fn classe_do_alvo(nome: u64, fabrica: &str) -> u64 {
    if nome == 0 {
        return 0;
    }
    let classe = rtti::Registry::obtain().map_or(std::ptr::null_mut(), |reg| reg.script_class(nome));
    if !classe.is_null() {
        return classe as u64;
    }
    #[cfg(feature = "devlog")]
    crate::log_veredito(&format!("[alvo] {fabrica}('{}') não resolve para classe: o alvo bloqueia todo evento", crate::cname::resolve_cname(nome)));
    #[cfg(not(feature = "devlog"))]
    let _ = fabrica;
    CLASSE_NAO_RESOLVIDA
}

/// `u64` da propriedade `nome` de `obj`, pelo RTTI da classe dele (propriedade de script mora no holder de `obj+0x38`).
unsafe fn propriedade_u64(obj: *mut c_void, nome: &str) -> Option<u64> {
    let (voff, _, holder) = rtti::resolve_prop_in_class(rtti::class_of(obj), nome)?;
    let campo = rtti::field_ptr(obj, voff, holder);
    if campo.is_null() {
        return None;
    }
    crate::memoria::read_u64(campo as usize)
}

/// Record de um device: o `tweakDBRecord` do estado persistente que o `GetPS` do `controller` devolve.
pub(crate) unsafe fn record_do_device(device: *mut c_void) -> Option<u64> {
    let controller = propriedade_u64(device, "controller")? as *mut c_void;
    if !rtti::sane(controller) {
        return None;
    }
    let get_ps = rtti::resolve_in_class(rtti::class_of(controller), "GetPS")?;
    let retorno = rtti::call_func(&get_ps, controller, &[])?;
    let mut estado = [
        u64::from_le_bytes(retorno[0..8].try_into().ok()?),
        u64::from_le_bytes(retorno[8..16].try_into().ok()?),
    ];
    let record = if rtti::sane(estado[0] as *mut c_void) { propriedade_u64(estado[0] as *mut c_void, "tweakDBRecord") } else { None };
    // O `GetPS` entrega o handle com +1 forte: sem soltar, o estado persistente do device nunca morre.
    crate::jogo::objetos::soltar_handle(&mut estado);
    record
}

/// Alvo de entidade contra a entidade de um evento de ciclo de vida, de componente ou de luz de veículo.
pub(crate) unsafe fn alvo_de_entidade_casa_entidade(c: &CamposDoAlvoDeEntidade, entidade: *mut c_void) -> bool {
    let ler = |off: usize| read_u64_field(entidade, off).unwrap_or(0);
    let evento = CamposDoEventoDeEntidade { entity_id: ler(0x48), template_path: ler(0x60), appearance_name: ler(0x50) };
    if !bwms_primitives::targets::entity_evento_bate(c, &evento) {
        return false;
    }
    if let Some(tipo) = c.tipo {
        if !class_is_kind_of(rtti::class_of(entidade), tipo as *mut c_void) {
            return false;
        }
    }
    // Categoria e record pedem RTTI e, no device, uma chamada de script: só com o filtro de record setado.
    if c.record_id.is_some() {
        let (categoria, record) = categoria_e_record(entidade);
        return bwms_primitives::targets::entity_record_bate(c.record_id, categoria, record);
    }
    true
}

/// Teto de aparências lidas da lista do builder: acima disso a lista é lixo, não conteúdo.
const BUILDER_APARENCIAS_MAX: usize = 256;

/// Aparência candidata de um `EntityBuilderAppearance` em `aparencia`: caminho do recurso e nome da definição.
unsafe fn candidata_do_builder(aparencia: u64) -> bwms_primitives::targets::AparenciaCandidata {
    bwms_primitives::targets::AparenciaCandidata {
        resource_path: eb_u64(eb_u64(aparencia, EBA_RECURSO), RES_CAMINHO),
        definition_name: eb_u64(eb_u64(aparencia, EBA_DEFINICAO), DEF_NOME),
    }
}

/// Alvo de entidade contra o builder travado do `EntityBuilderEvent`. Os critérios do request só valem com request.
pub(crate) unsafe fn alvo_de_entidade_casa_builder(c: &CamposDoAlvoDeEntidade, builder: u64) -> bool {
    let request = eb_u64(builder, EB_REQUEST);
    let do_request_difere = |filtro: Option<u64>, off: u64| filtro.is_some_and(|v| request != 0 && eb_u64(request, off) != v);
    if do_request_difere(c.entity_id, EBR_ENTITY_ID) {
        return false;
    }
    if let Some(tipo) = c.tipo {
        if eb_u64(builder, EB_EXTRATOR) == 0 {
            return false;
        }
        let raiz = eb_raiz_e_componentes(builder).0;
        if raiz == 0 || !class_is_kind_of(rtti::class_of(raiz as *mut c_void), tipo as *mut c_void) {
            return false;
        }
    }
    if c.template_path.is_some_and(|t| eb_u64(eb_u64(builder, EB_TEMPLATE), RES_CAMINHO) != t) {
        return false;
    }
    if do_request_difere(c.appearance_name, EBR_APARENCIA) || do_request_difere(c.record_id, EBR_RECORD) {
        return false;
    }
    let aparencia = builder + EB_APARENCIA;
    let unica = (eb_u64(aparencia, EBA_RECURSO) != 0).then(|| candidata_do_builder(aparencia));
    // A lista só decide sem aparência única e com filtro de caminho; cada candidata custa 4 leituras protegidas.
    if c.appearance_path.is_none() || unica.is_some() {
        return bwms_primitives::targets::entity_builder_aparencia_bate(c.appearance_path, c.definition_name, unica, &[]);
    }
    let (entradas, tam) = (eb_u64(builder + EB_APARENCIAS, 0), ((eb_u64(builder + EB_APARENCIAS, 0x08) >> 32) as usize).min(BUILDER_APARENCIAS_MAX));
    let mut lista = [bwms_primitives::targets::AparenciaCandidata { resource_path: 0, definition_name: 0 }; BUILDER_APARENCIAS_MAX];
    let n = if entradas == 0 { 0 } else { tam };
    for (i, candidata) in lista.iter_mut().enumerate().take(n) {
        *candidata = candidata_do_builder(entradas + i as u64 * EBA_TAM);
    }
    bwms_primitives::targets::entity_builder_aparencia_bate(c.appearance_path, c.definition_name, unica, &lista[..n])
}

/// Builder travado do `EntityBuilderEvent` em despacho: evento → wrapper → builder.
pub(crate) fn builder_do_evento(args: &[rtti::Arg]) -> Option<u64> {
    let Some(rtti::Arg::Handle(inst, _)) = args.first() else { return None };
    let wrapper = EBE_ESTADOS.lock().ok()?.iter().rev().find(|e| e.0 == *inst as usize)?.1[0];
    EBW_VIVOS.lock().ok()?.iter().rev().find(|e| e.0 == wrapper as usize).map(|e| e.1)
}

/// `Matches` do alvo `p` da família `f` contra o evento.
pub(crate) unsafe fn alvo_casa(p: usize, f: FamiliaDoAlvo, tipo: TipoDoEvento, args: &[rtti::Arg]) -> bool {
    match f {
        FamiliaDoAlvo::Entidade => {
            let campos = campos_do_alvo_de_entidade(p);
            match tipo {
                TipoDoEvento::Builder => builder_do_evento(args).is_some_and(|b| alvo_de_entidade_casa_builder(&campos, b)),
                // `EntityComponentEvent` e `VehicleLightControlEvent` herdam o `EntityLifecycleEvent`: casam pela entidade dona.
                TipoDoEvento::Ciclo | TipoDoEvento::Componente | TipoDoEvento::LuzDoVeiculo => event_entity_ptr(args).is_some_and(|e| alvo_de_entidade_casa_entidade(&campos, e)),
                _ => true,
            }
        }
        FamiliaDoAlvo::EntidadeDinamica | FamiliaDoAlvo::EntidadeEstatica => {
            let (alvos, tabela) = if f == FamiliaDoAlvo::EntidadeDinamica { (&DYNAMICENTITYTARGET_STATES, dynent_tags()) } else { (&STATICENTITYTARGET_STATES, statent_tags()) };
            // `Tags([])` casa sem ler a entidade do evento: lista vazia não restringe, e o evento sem entidade também passa.
            let le_a_entidade = alvos.lock().is_ok_and(|s| s.iter().any(|e| e.0 == p && !e.1.is_empty()));
            // O `IsTagged` do sistema de entidades do alvo, pelo id da entidade do evento: a tabela do `DynamicEntitySystem` ou a do `StaticEntitySystem`.
            // Lida antes de travar a tabela de alvos para comparar: nenhuma trava fica aninhada neste ramo.
            let da_entidade = if le_a_entidade {
                event_entity_ptr(args).and_then(|e| entity_id_hash(e)).and_then(|id| tabela.lock().ok().and_then(|m| m.get(&id).cloned()))
            } else {
                None
            };
            alvos.lock().is_ok_and(|s| {
                s.iter().find(|e| e.0 == p).is_some_and(|e| bwms_primitives::targets::bate_por_tags(&e.1, |t| da_entidade.as_ref().is_some_and(|d| d.contains(&t))))
            })
        }
        FamiliaDoAlvo::Componente => {
            // Valor 0 é o não-setado da fábrica e fica `None`.
            let criterio = COMPONENTTARGET_STATES.lock().ok().and_then(|s| s.iter().find(|e| e.0 == p).copied()).filter(|e| e.2 != 0);
            let (nome, id) = match criterio {
                Some((_, ComponentTargetCriterion::Name, v)) => (Some(v), None),
                Some((_, ComponentTargetCriterion::Id, v)) => (None, Some(v)),
                None => (None, None),
            };
            // O componente só é lido para campo setado: o alvo todo curinga casa
            // sem dado do evento. Campo setado com componente que não resolve continua filtrado.
            if nome.is_none() && id.is_none() {
                return true;
            }
            event_component_ptr(args).and_then(|c| component_id_name(c)).is_some_and(|(ev_id, ev_nome)| bwms_primitives::targets::component_bate(nome, id, ev_nome, ev_id))
        }
        FamiliaDoAlvo::Tecla => {
            let Some((tecla, acao)) = inputtarget_criteria(&[p]).first().copied() else { return false };
            // O evento só é lido para campo setado: o alvo todo curinga casa sem tecla
            // registrada. Campo setado com evento sem tecla continua filtrado.
            if tecla.is_none() && acao.is_none() {
                return true;
            }
            let (ev_tecla, ev_acao) = event_input_key_action(args);
            match (ev_tecla, ev_acao) {
                (Some(k), Some(a)) => bwms_primitives::targets::input_bate(tecla, acao, k as u32, a as u32),
                _ => false,
            }
        }
        FamiliaDoAlvo::Recurso => {
            // Todo critério do alvo bate — caminho igual (ou pelo padrão registrado) e classe do recurso derivada da
            // classe que a fábrica resolveu (`classe_do_alvo`).
            // Valor 0 é critério não-setado e não filtra:
            // `Path(r"")` chega com hash 0 e `caminho_casa(0, _)` casa só evento de caminho 0: hash 0 não é registrado e cai no `alvo == evento`.
            let criterios: Vec<(ResourceTargetCriterion, u64)> =
                RESOURCETARGET_STATES.lock().map_or(Vec::new(), |s| s.iter().filter(|e| e.0 == p).map(|e| (e.1, e.2)).collect());
            let caminho = event_resource_path(args);
            let recurso = recurso_do_evento(args);
            criterios.iter().all(|(k, v)| match k {
                ResourceTargetCriterion::Path => *v == 0 || caminho.is_some_and(|c| caminho_casa(*v, c)),
                ResourceTargetCriterion::Type => *v == 0 || recurso.is_some_and(|r| class_is_kind_of(rtti::class_of(r as *mut c_void), *v as *mut c_void)),
            })
        }
        FamiliaDoAlvo::Widget => {
            let Some(alvo) = inkwidgettarget_alvos(&[p]).first().copied() else { return false };
            event_widget_spawn(args).is_some_and(|(lib, item, instancia)| ink_widget_alvo_casa(alvo, lib, item, |t| widget_controller_herda(instancia, t)))
        }
    }
}

/// Os alvos de um handler cuja família suporta o evento, na ordem do handler.
pub(crate) fn alvos_que_suportam(alvos: &[usize], tipo: TipoDoEvento) -> impl Iterator<Item = (usize, FamiliaDoAlvo)> + '_ {
    alvos.iter().filter_map(|p| familia_do_alvo(*p).map(|f| (*p, f))).filter(move |(_, f)| familia_suporta(*f, tipo))
}

/// Os alvos de um handler contra o evento. `true` = handler
/// filtrado: algum alvo suporta o evento e nenhum casa; `false` = dispara. Uma passada, que para no 1º alvo que casa.
pub(crate) unsafe fn alvos_barram_o_disparo(alvos: &[usize], tipo: TipoDoEvento, args: &[rtti::Arg]) -> bool {
    let mut tem_suportado = false;
    for (p, f) in alvos_que_suportam(alvos, tipo) {
        if alvo_casa(p, f, tipo, args) {
            return false;
        }
        tem_suportado = true;
    }
    tem_suportado
}

/// Extensão de customização (2026-08-14): `CustomizationEvent extends
/// CallbackSystemEvent` — MESMA receita robusta (parent nativo real já forjado). Forja MÍNIMA,
/// zero método próprio (herda `GetEventName()` do parent, mesmo padrão de `GameSessionEvent`) —
/// serve só de ENVELOPE pra `fire_event_args` despachar os 3 eventos nativos reais de
/// customização de personagem
/// (`gameuiCharacterCustomizationSystem_On{AppearanceApplied,AppearanceSwitched,OptionUpdated}Event`,
/// `orphans.script:54181-54213`, confirmados em uso vanilla real dentro de
/// `characterCreationBodyMorphMenu.script`) — os 3 se distinguem pelo NOME de despacho, não por
/// campo do objeto (mesmo padrão já usado por "Session/Start"/"Session/End" e "Entity/Attach").
/// `characterCreationBodyMorphMenu` NÃO é exclusiva do pregame — é a MESMA tela que
/// `MenuScenario_CharacterCustomizationMirror.OnCCOPuppetReady` abre no espelho real do mundo
/// (confirmado por `bwms-customization-apply.reds`, já wrapping 3 métodos DIFERENTES da mesma
/// classe) — então o dispatch cobre pregame E o espelho de customização in-game.
pub unsafe fn register_customizationevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_customizationevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "CustomizationEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_customizationevent: forja da classe FALHOU".into();
    }
    "[reg] CustomizationEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real, zero método próprio)".into()
}

/// Composição do argumento p/ `fire_event_args("Customization/...", ...)` — MESMA receita de
/// `make_resourceevent_arg` acima (`rtti::new_object`+`rtti::make_handle`). `CustomizationEvent`
/// não carrega estado próprio (os 3 eventos se distinguem pelo NOME de despacho).
pub(crate) unsafe fn make_customizationevent_arg() -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "CustomizationEvent");
    if obj.is_null() {
        return None;
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// EDGE FLAGS — deliberadamente NÃO despacha (`fire_event_args`, que faz `resolve_in_class`+
/// `call_func`, dispatch RTTI real) de dentro deste trampolim: este native é chamado SINCRONAMENTE
/// por bytecode redscript (dentro do `@wrapMethod` em `characterCreationBodyMorphMenu`), e
/// despachar OUTRO evento de dentro dessa call-site seria a MESMA categoria de risco de
/// reentrância que motivou o `ExecDepthGuard` (2026-07-31). Em vez disso, só marca um atomic —
/// MESMO padrão já provado e vivo em produção de `Resource/Load`/`WATCH_RES_SEEN`
/// (`selftest.rs`/`lib.rs::cp77_tick`): o hook marca a flag, o TICK LOOP (contexto seguro, fora de
/// qualquer nesting de bytecode) faz o construct+dispatch de verdade. `kind`: 0=AppearanceApplied,
/// 1=AppearanceSwitched, 2=OptionUpdated.
pub(crate) static CUSTOMIZATION_EDGE_APPLIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) static CUSTOMIZATION_EDGE_SWITCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) static CUSTOMIZATION_EDGE_OPTIONUPDATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe extern "C" fn tramp_customization_edge(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let kind = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let which = match kind {
        0 => {
            CUSTOMIZATION_EDGE_APPLIED.store(true, Ordering::Relaxed);
            "AppearanceApplied"
        }
        1 => {
            CUSTOMIZATION_EDGE_SWITCHED.store(true, Ordering::Relaxed);
            "AppearanceSwitched"
        }
        2 => {
            CUSTOMIZATION_EDGE_OPTIONUPDATED.store(true, Ordering::Relaxed);
            "OptionUpdated"
        }
        _ => "?",
    };
    crate::log_dev!("[customization] BwmsCustomizationEdge(kind={kind}) -> marcado ({which})");
}

/// Drena os 3 edge flags (chamado 1x por tick de `cp77_tick`, MESMO ponto onde `Resource/Load`/
/// `Session/*`/etc. já despacham) — constrói o `CustomizationEvent` e chama `fire_event_args`.
pub unsafe fn drain_customization_edges() {
    if CUSTOMIZATION_EDGE_APPLIED.swap(false, Ordering::Relaxed) {
        if let Some(arg) = make_customizationevent_arg() {
            let n = fire_event_args("Customization/AppearanceApplied", &[arg]);
            crate::log_dev!("[cbs] Customization/AppearanceApplied → {n} callback(s)");
        }
    }
    if CUSTOMIZATION_EDGE_SWITCHED.swap(false, Ordering::Relaxed) {
        if let Some(arg) = make_customizationevent_arg() {
            let n = fire_event_args("Customization/AppearanceSwitched", &[arg]);
            crate::log_dev!("[cbs] Customization/AppearanceSwitched → {n} callback(s)");
        }
    }
    if CUSTOMIZATION_EDGE_OPTIONUPDATED.swap(false, Ordering::Relaxed) {
        if let Some(arg) = make_customizationevent_arg() {
            let n = fire_event_args("Customization/OptionUpdated", &[arg]);
            crate::log_dev!("[cbs] Customization/OptionUpdated → {n} callback(s)");
        }
    }
}

pub(crate) const TPL_ROOT_INDEX: u64 = 0x1d8;

/// Teto de slots lidos do `AttachmentSlots` do dono na composição: o V usa ~30.
const TPP_SLOTS_MAX: usize = 64;

/// Depois do `OnAttach` do `TPPRepresentationComponent`: todo slot do dono cujo `.parentSlot` (flat de mod) é Head ou Eyes entra em
/// `affectedAppearanceSlots` do componente, e o item que já está nele é registrado como afetado. Conteúdo vanilla não tem `.parentSlot`
/// e não escreve nada. Devolve o relatório do veredito.
pub unsafe fn compor_slots_da_terceira_pessoa(reg: &crate::rtti::Registry, componente: *mut c_void) -> String {
    use crate::roupas_e_armas::terceira_pessoa as tpp;
    if componente.is_null() || !crate::memoria::is_readable(componente as *const c_void, 0xC0) {
        return format!("[tpp-slots] compor: componente {componente:p} ilegível, nada feito");
    }
    let mut dono = rd_u64(componente as *const c_void, tpp::COMPONENTE_DONO_OFF) as *mut c_void;
    if dono.is_null() {
        dono = crate::current_player();
    }
    let Some(slots) = find_attachslots_component(reg, dono) else {
        return format!("[tpp-slots] compor: componente {componente:p} dono {dono:p} sem AttachmentSlots, nada feito");
    };
    if crate::tweakdb::singleton().is_none() {
        return format!("[tpp-slots] compor: componente {componente:p} sem TweakDB, nada feito");
    }
    let dados = (slots as *const u8).add(tpp::ATTACHSLOTS_DADOS_OFF);
    let (entradas, n) = ((dados as *const u64).read_unaligned() as *const u8, (dados.add(0x0C) as *const u32).read_unaligned() as usize);
    let afetados = tpp::slots_afetados();
    let mut sub = [0u64; TPP_SLOTS_MAX];
    let (mut checados, mut achados) = (0usize, 0usize);
    if !entradas.is_null() {
        for i in 0..n.min(TPP_SLOTS_MAX) {
            let e = entradas.add(i * tpp::SLOTDATA_TAM);
            if !crate::memoria::is_readable(e as *const c_void, 8) {
                continue;
            }
            let slot = (e as *const u64).read_unaligned();
            if slot == 0 {
                continue;
            }
            checados += 1;
            // A mesma regra dos handlers de equip e unequip: os dois caminhos têm de concordar sobre o que é sub-slot.
            if tpp::precisa_ramo_afetado(slot, tpp::slot_base(slot), afetados) {
                sub[achados] = slot;
                achados += 1;
            }
        }
    }
    if achados == 0 {
        return format!("[tpp-slots] compor: componente {componente:p} AttachmentSlots {slots:p} slots_checados={checados} extra_slots_achados=0 — zero extra-slots, nada escrito");
    }
    let lista = (componente as *mut u8).add(tpp::SLOTS_AFETADOS_OFF) as *mut c_void;
    let mut relato = Vec::with_capacity(achados);
    for &slot in &sub[..achados] {
        let chave = bwms_primitives::tweak_db_id_chave(slot);
        let (ini, tam) = (rd_u64(lista, 0) as *const u64, rd_u32(lista, 0x0C) as usize);
        let ja_esta = !ini.is_null()
            && crate::memoria::is_readable(ini as *const c_void, tam * 8)
            && (0..tam).any(|k| bwms_primitives::tweak_db_id_chave(ini.add(k).read_unaligned()) == chave);
        let anexou = !ja_esta && dynarray_push_ptr(lista, slot).is_some();
        let item = crate::roupas_e_armas::terceira_pessoa::registrar_item_do_slot(componente, slot);
        relato.push(format!("{slot:#x}: anexado={anexou} já_estava={ja_esta} item={item:x?}"));
    }
    format!(
        "[tpp-slots] compor: componente {componente:p} AttachmentSlots {slots:p} slots_checados={checados} extra_slots_achados={achados} [{}]",
        relato.join("; ")
    )
}


pub(crate) type Es20Escopos = std::collections::HashMap<u64, std::collections::HashSet<u64>>;


pub(crate) const ES22_RECURSO_TRAVA: usize = 0xf0;

pub(crate) static STATENT_TAGS_CELL: std::sync::OnceLock<StatentMap> = std::sync::OnceLock::new();

pub(crate) const CUST_TIPOS: [CustTipo; 3] = [CustTipo::App, CustTipo::Morph, CustTipo::Switcher];

/// Lista de escolhas de um tipo: tipo do elemento, passo, offset da lista na opção, tipo do array, `index` e a chave (`name` no app, `localizedName` nos
/// outros).
#[derive(Clone, Copy)]
pub(crate) struct CustLista {
    pub tipo: usize,
    pub passo: usize,
    pub campo: usize,
    pub tipo_array: usize,
    pub indice: usize,
    pub chave: usize,
}

pub(crate) const CUST_GRUPO_PASSO: usize = 0x18;

pub(crate) unsafe fn cust_resolver(reg: &Registry) -> Result<CustCampos, String> {
    let classe = |n: &str| -> Result<*mut c_void, String> {
        let c = reg.class_by_name(n);
        if rtti::sane(c) { Ok(c) } else { Err(format!("classe {n}")) }
    };
    let campo = |c: *mut c_void, n: &str| -> Result<(usize, usize), String> {
        rtti::resolve_prop_in_class(c, n).map(|(o, t, _)| (o as usize, t as usize)).ok_or_else(|| format!("campo {n}"))
    };
    let lista = |c: *mut c_void, n: &str, chave: &str| -> Result<CustLista, String> {
        let (campo_off, tipo_array) = campo(c, n)?;
        let tipo = rtti::get_inner_type(tipo_array as *mut c_void).ok_or_else(|| format!("tipo interno de {n}"))? as usize;
        let passo = rtti::type_size(tipo as *mut c_void) as usize;
        if passo == 0 || passo > 0x400 {
            return Err(format!("passo {passo:#x} de {n}"));
        }
        let (indice, _) = campo(tipo as *mut c_void, "index")?;
        let (chave, _) = campo(tipo as *mut c_void, chave)?;
        Ok(CustLista { tipo, passo, campo: campo_off, tipo_array, indice, chave })
    };
    let info = classe("gameuiCharacterCustomizationInfo")?;
    let (app, morph, switcher) = (classe("gameuiAppearanceInfo")?, classe("gameuiMorphInfo")?, classe("gameuiSwitcherInfo")?);
    let listas = [lista(app, "definitions", "name")?, lista(morph, "morphNames", "localizedName")?, lista(switcher, "options", "localizedName")?];
    let (tags, _) = campo(listas[0].tipo as *mut c_void, "tags")?;
    let (icone, _) = campo(listas[0].tipo as *mut c_void, "icon")?;
    if campo(classe("redTagList")?, "tags")?.0 != 0 {
        return Err("redTagList.tags fora do +0".into());
    }
    let opcao = classe("gameuiCharacterCustomizationOption")?;
    let recurso = classe("gameuiCharacterCustomizationInfoResource")?;
    let nomes_opcoes = ["headCustomizationOptions", "bodyCustomizationOptions", "armsCustomizationOptions"];
    let nomes_grupos = ["headGroups", "bodyGroups", "armsGroups"];
    let mut tipos = [0usize; 2];
    for k in 0..3 {
        let (o, t) = campo(recurso, nomes_opcoes[k])?;
        let (g, tg) = campo(recurso, nomes_grupos[k])?;
        if o != CUST_OPCOES_OFF[k] || g != CUST_GRUPOS_OFF[k] {
            return Err(format!("{} em {o:#x}, {} em {g:#x}", nomes_opcoes[k], nomes_grupos[k]));
        }
        tipos = [t, tg];
    }
    let tipo_grupo = rtti::get_inner_type(tipos[1] as *mut c_void).ok_or("tipo do grupo")? as usize;
    let (g_nome, g_opcoes) = (campo(tipo_grupo as *mut c_void, "name")?.0, campo(tipo_grupo as *mut c_void, "options")?.0);
    if g_nome != 0 || g_opcoes != 8 || rtti::type_size(tipo_grupo as *mut c_void) as usize != CUST_GRUPO_PASSO {
        return Err(format!("gameuiOptionsGroup name {g_nome:#x} options {g_opcoes:#x}"));
    }
    let (tipo_string, tipo_cnames) = (get_type(reg, "String") as usize, get_type(reg, "array:CName") as usize);
    if !rtti::sane(tipo_string as *mut c_void) || !rtti::sane(tipo_cnames as *mut c_void) {
        return Err("tipos String/array:CName".into());
    }
    Ok(CustCampos {
        classes: [app as usize, morph as usize, switcher as usize],
        nome: campo(info, "name")?.0,
        slot: campo(info, "uiSlot")?.0,
        link: campo(info, "link")?.0,
        oculto: campo(info, "hidden")?.0,
        ligado: campo(info, "enabled").ok().map(|c| c.0),
        recurso: campo(app, "resource")?.0,
        listas,
        tags,
        icone,
        opcao_info: campo(opcao, "info")?.0,
        opcao_ativo: campo(opcao, "isActive")?.0,
        tipo_string,
        tipo_cnames,
        tipo_arr_opcoes: tipos[0],
        tipo_arr_grupos: tipos[1],
        tipo_grupo,
        classe_recurso: recurso as usize,
    })
}

pub(crate) static CUST_CAMPOS: Mutex<Option<CustCampos>> = Mutex::new(None);

/// Campos guardados, resolvidos na 1ª chamada.
pub(crate) unsafe fn cust_campos() -> Option<CustCampos> {
    let mut g = CUST_CAMPOS.lock().ok()?;
    if g.is_none() {
        match cust_resolver(&Registry::obtain()?) {
            Ok(c) => *g = Some(c),
            Err(e) => {
                crate::veredito_dev!("[a47c] campos de customização pelo RTTI: {e}");
                return None;
            }
        }
    }
    *g
}

pub(crate) unsafe fn cu64(a: usize) -> u64 {
    (a as *const u64).read_unaligned()
}

pub(crate) unsafe fn cu8(a: usize) -> u8 {
    *(a as *const u8)
}

/// `(entries, tamanho, capacidade)` de um `DynArray`.
pub(crate) unsafe fn cust_arr(a: usize) -> (usize, usize, usize) {
    (cu64(a) as usize, ((a + 0xc) as *const u32).read_unaligned() as usize, ((a + 8) as *const u32).read_unaligned() as usize)
}

/// Garante 1 vaga no fim do `DynArray`. Cheio, cresce pelo reserve do motor: o buffer novo sai do alocador do motor, que é quem o solta.
pub(crate) unsafe fn cust_reservar_mais_um(arr: usize, passo: usize) -> bool {
    let (_, tamanho, capacidade) = cust_arr(arr);
    if tamanho < capacidade {
        return true;
    }
    // Meia capacidade a mais: a mescla anexa uma a uma, e dobrar a lista de opções do jogo a cada estouro desperdiça a metade.
    let nova = (capacidade + capacidade / 2).max(tamanho + 1).max(4);
    crate::jogo::listas::reservar(arr as *mut c_void, nova as u32, passo as u32, 8, std::ptr::null());
    cust_arr(arr).2 > tamanho
}

/// `vt+off` do tipo (`0x40` Construct, `0x48` Destruct).
pub(crate) unsafe fn cust_tipo_fn(tipo: usize, off: usize, valor: usize) {
    let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(cu64(cu64(tipo) as usize + off));
    f(tipo as *mut c_void, valor as *mut c_void);
}

/// Anexa uma cópia do valor `fonte` (construído e atribuído pelo tipo). Devolve o endereço do novo.
pub(crate) unsafe fn cust_anexar_valor(arr: usize, tipo: usize, passo: usize, fonte: usize) -> Option<usize> {
    if !cust_reservar_mais_um(arr, passo) {
        return None;
    }
    let (e, n, _) = cust_arr(arr);
    let vaga = e + n * passo;
    std::ptr::write_bytes(vaga as *mut u8, 0, passo);
    cust_tipo_fn(tipo, 0x40, vaga);
    if fonte != 0 {
        rtti::type_assign(tipo as *mut c_void, vaga as *mut c_void, fonte as *const c_void);
    }
    ((arr + 0xc) as *mut u32).write_unaligned(n as u32 + 1);
    Some(vaga)
}

/// Anexa um `u64` (CName) num `DynArray<CName>`.
pub(crate) unsafe fn cust_anexar_u64(arr: usize, v: u64) -> bool {
    if !cust_reservar_mais_um(arr, 8) {
        return false;
    }
    let (e, n, _) = cust_arr(arr);
    ((e + n * 8) as *mut u64).write_unaligned(v);
    ((arr + 0xc) as *mut u32).write_unaligned(n as u32 + 1);
    true
}

/// Anexa uma cópia do handle (forte +1) num `DynArray<Handle<T>>`.
pub(crate) unsafe fn cust_anexar_handle(arr: usize, h: [u64; 2]) -> bool {
    if h[0] == 0 || !cust_reservar_mais_um(arr, 0x10) {
        return false;
    }
    if h[1] != 0 {
        (*(h[1] as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
    }
    let (e, n, _) = cust_arr(arr);
    ((e + n * 0x10) as *mut [u64; 2]).write_unaligned(h);
    ((arr + 0xc) as *mut u32).write_unaligned(n as u32 + 1);
    true
}

/// Leva o `DynArray` de volta a `tamanho`, destruindo o que passa dele.
pub(crate) unsafe fn cust_truncar(arr: usize, tamanho: usize, el: CustElemento) {
    let (e, n, _) = cust_arr(arr);
    for k in (tamanho..n).rev() {
        match el {
            CustElemento::Handle => {
                let mut h = ((e + k * 0x10) as *const [u64; 2]).read_unaligned();
                crate::jogo::objetos::soltar_handle(&mut h);
            }
            CustElemento::CName => {}
            CustElemento::Valor { tipo, passo } => cust_tipo_fn(tipo, 0x48, e + k * passo),
        }
        ((arr + 0xc) as *mut u32).write_unaligned(k as u32);
    }
}

/// Handle da opção `k` de um `DynArray<Handle<gameuiCharacterCustomizationInfo>>`.
pub(crate) unsafe fn cust_handle(arr: usize, k: usize) -> [u64; 2] {
    ((cust_arr(arr).0 + k * 0x10) as *const [u64; 2]).read_unaligned()
}

pub(crate) unsafe fn cust_tipo_da_opcao(c: &CustCampos, obj: usize) -> Option<CustTipo> {
    if obj == 0 {
        return None;
    }
    let cls = rtti::class_of(obj as *mut c_void);
    CUST_TIPOS.into_iter().find(|t| class_is_kind_of(cls, c.classes[*t as usize] as *mut c_void))
}

pub(crate) unsafe fn cust_chave(l: &CustLista, tipo: CustTipo, elem: usize) -> CustChave {
    if tipo == CustTipo::App {
        CustChave::Nome(cu64(elem + l.chave))
    } else {
        CustChave::Texto(rtti::red_string_read((elem + l.chave) as *const u8))
    }
}

/// Nome registrado de uma escolha: o `name` no app; o texto nos outros, com o hash continuado do nome da opção (o mesmo texto em opções
/// diferentes não colide).
pub(crate) fn cust_nome_da_escolha(dono: u64, chave: &CustChave) -> u64 {
    match chave {
        CustChave::Nome(n) => *n,
        CustChave::Texto(t) => bwms_primitives::fnv1a64_seeded(t.as_bytes(), dono),
    }
}

/// Curinga de `uiSlot`/`link` no passe de slots: texto terminado em `*` casa por prefixo.
pub(crate) fn cust_curinga(texto: &str) -> Option<&str> {
    texto.strip_suffix('*')
}

/// `uiSlot`/`link` do alvo casa com o da fonte: igual, ou prefixo com curinga.
pub(crate) fn cust_casa(alvo: u64, alvo_texto: impl FnOnce() -> String, fonte: u64, prefixo: Option<&str>) -> bool {
    match prefixo {
        Some(p) => alvo_texto().starts_with(p),
        None => alvo == fonte,
    }
}

/// Posição de `nome` no array ordenado do estado: `Ok` se já está, `Err` com onde entra para manter a ordem.
pub(crate) fn cust_posicao_ordenada(v: &[u64], nome: u64) -> Result<usize, usize> {
    v.binary_search(&nome)
}

/// Nomes de opção e de escolha que a mescla pôs nas listas do jogo.
pub(crate) static CUST_NOMES: std::sync::RwLock<Option<std::collections::HashSet<u64>>> = std::sync::RwLock::new(None);

/// Tamanho de [`CUST_NOMES`]: os ganchos do `Initialize*Option` leem só este atômico quando não há mescla.
pub(crate) static CUST_NOMES_N: AtomicU64 = AtomicU64::new(0);

pub(crate) fn cust_registrar_nome(n: u64) {
    // CName vazio não é nome: registrá-lo marcaria toda opção sem nome como de mod.
    if n == 0 {
        return;
    }
    if let Ok(mut g) = CUST_NOMES.write() {
        let nomes = g.get_or_insert_with(Default::default);
        nomes.insert(n);
        CUST_NOMES_N.store(nomes.len() as u64, Ordering::Relaxed);
    }
}

pub(crate) fn cust_nome_de_mod(n: u64) -> bool {
    CUST_NOMES_N.load(Ordering::Relaxed) != 0 && CUST_NOMES.read().is_ok_and(|g| g.as_ref().is_some_and(|s| s.contains(&n)))
}

#[cfg(feature = "devtools")]
pub(crate) fn cust_nomes_n() -> u64 {
    CUST_NOMES_N.load(Ordering::Relaxed)
}

/// Escolha do jogo substituída: o valor de antes, construído num buffer nosso.
pub(crate) struct CustTroca {
    pub(crate) dono: usize,
    pub(crate) tipo: CustTipo,
    pub(crate) pos: usize,
    pub(crate) copia: Vec<u64>,
}

pub(crate) static CUST: Mutex<Option<CustMescla>> = Mutex::new(None);

/// Fontes por gênero (masculino, feminino): o `SharedPtr<ResourceToken>` pedido na carga da tabela, ou (teste) o endereço de um recurso já montado.
#[derive(Clone, Copy)]
pub(crate) enum CustFonte {
    Token([u64; 2]),
    Recurso(usize),
}

pub(crate) static CUST_FONTES: Mutex<[Vec<CustFonte>; 2]> = Mutex::new([Vec::new(), Vec::new()]);

#[cfg(feature = "devtools")]
pub(crate) fn cust_fonte_teste(masc: bool, recurso: usize, ligar: bool) {
    if let Ok(mut f) = CUST_FONTES.lock() {
        let lista = &mut f[usize::from(!masc)];
        if ligar {
            lista.push(CustFonte::Recurso(recurso));
        } else {
            lista.retain(|x| !matches!(x, CustFonte::Recurso(r) if *r == recurso));
        }
    }
}

/// Caminhos do `resource.fix` por recurso (`FixCustomizationOptions`): `recurso → [(antigo, novo)]`, das linhas `paths` de `bwms-resfixes.txt`
/// (`mods_do_usuario::pipeline::resfixes_linhas`, um novo por antigo).
pub(crate) static CUST_FIX_CAMINHOS: std::sync::RwLock<Option<std::collections::HashMap<u64, Vec<(u64, u64)>>>> = std::sync::RwLock::new(None);

/// Teste: põe (ou tira) um par `antigo → novo` do `resource.fix` do recurso base `recurso`, fora da tabela.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn cust_fix_caminho_teste(recurso: u64, antigo: u64, novo: u64, ligar: bool) {
    if let Ok(mut g) = CUST_FIX_CAMINHOS.write() {
        let m = g.get_or_insert_with(Default::default);
        let lista = m.entry(recurso).or_default();
        lista.retain(|p| *p != (antigo, novo));
        if ligar {
            lista.push((antigo, novo));
        }
        if lista.is_empty() {
            m.remove(&recurso);
        }
    }
}

pub(crate) fn cust_fix_caminhos_carregar(caminhos: std::collections::HashMap<u64, Vec<(u64, u64)>>) -> usize {
    let n = caminhos.values().map(Vec::len).sum();
    if let Ok(mut g) = CUST_FIX_CAMINHOS.write() {
        *g = Some(caminhos);
    }
    n
}

/// Objeto da opção `i` de um `DynArray<Handle<gameuiCharacterCustomizationInfo>>`.
unsafe fn cust_opcao(opcoes: usize, i: usize) -> usize {
    cust_handle(opcoes, i)[0] as usize
}

/// `resource.fix` com `paths` do recurso base `caminho_alvo`: a opção de app cujo `resource` é um caminho antigo passa ao novo, e as
/// aparências das definições dela, pedidas pelo caminho antigo ou por um alias do novo, trocam para o novo.
pub(crate) unsafe fn cust_consertar_caminhos(c: &CustCampos, m: &mut CustMescla, caminho_alvo: u64, opcoes: usize) {
    let pares = match CUST_FIX_CAMINHOS.read() {
        Ok(g) => match g.as_ref().and_then(|t| t.get(&caminho_alvo)) {
            Some(p) if !p.is_empty() => p.clone(),
            _ => return,
        },
        Err(_) => return,
    };
    let l = c.listas[CustTipo::App as usize];
    for i in 0..cust_arr(opcoes).1 {
        let t = cust_opcao(opcoes, i);
        if cust_tipo_da_opcao(c, t) != Some(CustTipo::App) {
            continue;
        }
        let antigo = cu64(t + c.recurso);
        let Some(&(_, novo)) = pares.iter().find(|(a, n)| antigo != 0 && *a == antigo && *n != *a) else { continue };
        m.caminhos.push((t, antigo));
        ((t + c.recurso) as *mut u64).write_unaligned(novo);
        m.contagem.caminhos += 1;
        let aliases = crate::arquivos_do_jogo::reslink_aliases_of(novo);
        let (e, n, _) = cust_arr(t + l.campo);
        for k in 0..n {
            let aparencia = cu64(e + k * l.passo + l.chave);
            m.override_(antigo, novo, aparencia);
            for alias in &aliases {
                m.override_(*alias, novo, aparencia);
            }
        }
    }
}

/// Entradas de cada grupo da fonte vão ao fim do grupo homônimo do alvo. Grupo da fonte sem homônimo não vira grupo novo: a tela só
/// mostra os grupos que o recurso do jogo declara.
pub(crate) unsafe fn cust_mesclar_grupos(m: &mut CustMescla, alvo: usize, fonte: usize) {
    for i in 0..cust_arr(fonte).1 {
        let grupo_f = cust_arr(fonte).0 + i * CUST_GRUPO_PASSO;
        let (nome, entradas_f) = (cu64(grupo_f), grupo_f + 8);
        for j in 0..cust_arr(alvo).1 {
            let grupo_t = cust_arr(alvo).0 + j * CUST_GRUPO_PASSO;
            if cu64(grupo_t) != nome {
                continue;
            }
            let entradas_t = grupo_t + 8;
            for k in 0..cust_arr(entradas_f).1 {
                let entrada = cu64(cust_arr(entradas_f).0 + k * 8);
                m.anotar(entradas_t, CustElemento::CName);
                if cust_anexar_u64(entradas_t, entrada) {
                    m.contagem.grupos += 1;
                    cust_registrar_nome(entrada);
                }
            }
        }
    }
}

/// Opção de app sem nome e sem definições herda as definições (e o recurso, se não tem) da última opção sem nome cheia antes dela: o
/// modder declara as definições uma vez e repete só o slot. Devolve quantas opções receberam.
pub(crate) unsafe fn cust_expandir(c: &CustCampos, fonte: usize) -> u32 {
    let l = c.listas[CustTipo::App as usize];
    let mut cheia: Option<usize> = None;
    let mut expandidas = 0;
    for i in 0..cust_arr(fonte).1 {
        let o = cust_opcao(fonte, i);
        if cust_tipo_da_opcao(c, o) != Some(CustTipo::App) || cu64(o + c.nome) != 0 {
            continue;
        }
        let definicoes = o + l.campo;
        if cust_arr(definicoes).1 != 0 {
            cheia = Some(o);
            continue;
        }
        let Some(f) = cheia else { continue };
        let (e, n, _) = cust_arr(f + l.campo);
        let mut recebeu = false;
        for k in 0..n {
            recebeu |= cust_anexar_valor(definicoes, l.tipo, l.passo, e + k * l.passo).is_some();
        }
        if cu64(o + c.recurso) == 0 {
            ((o + c.recurso) as *mut u64).write_unaligned(cu64(f + c.recurso));
        }
        expandidas += u32::from(recebeu);
    }
    expandidas
}

/// Escolhas da opção `src` na opção `t` do mesmo tipo: chave igual substitui, chave nova vai ao fim.
pub(crate) unsafe fn cust_mesclar_escolhas(c: &CustCampos, m: &mut CustMescla, t: usize, src: usize, tipo: CustTipo) {
    let l = c.listas[tipo as usize];
    let (lista_t, lista_s) = (t + l.campo, src + l.campo);
    let dono = cu64(t + c.nome);
    let app = tipo == CustTipo::App;
    // `resource` e `uiSlot` de cor de cabelo só existem no app: no morph e no switcher o offset é de outro campo.
    let (recurso_t, recurso_s) = if app { (cu64(t + c.recurso), cu64(src + c.recurso)) } else { (0, 0) };
    let cabelo = app && cu64(t + c.slot) == cname("hair_color");
    for j in 0..cust_arr(lista_s).1 {
        let es = cust_arr(lista_s).0 + j * l.passo;
        let chave = cust_chave(&l, tipo, es);
        let (et, nt, _) = cust_arr(lista_t);
        match (0..nt).find(|k| cust_chave(&l, tipo, et + k * l.passo) == chave) {
            Some(pos) => {
                let el = et + pos * l.passo;
                // A escolha que outra fonte já pôs sai pelo truncamento ou pela cópia guardada da 1ª troca: guardar de novo voltaria a de mod.
                let guardada = m.trocas.iter().any(|tr| tr.dono == t && tr.tipo == tipo && tr.pos == pos);
                if !guardada && !cust_nome_de_mod(cust_nome_da_escolha(dono, &chave)) {
                    let mut copia = vec![0u64; l.passo.div_ceil(8)];
                    let p = copia.as_mut_ptr() as usize;
                    cust_tipo_fn(l.tipo, 0x40, p);
                    rtti::type_assign(l.tipo as *mut c_void, p as *mut c_void, el as *const c_void);
                    m.trocas.push(CustTroca { dono: t, tipo, pos, copia });
                }
                rtti::type_assign(l.tipo as *mut c_void, el as *mut c_void, es as *const c_void);
                m.contagem.trocas += 1;
            }
            None => {
                m.anotar(lista_t, CustElemento::Valor { tipo: l.tipo, passo: l.passo });
                if cust_anexar_valor(lista_t, l.tipo, l.passo, es).is_none() {
                    continue;
                }
                m.contagem.escolhas[tipo as usize] += 1;
                // A definição nova é pedida ao recurso do alvo: a troca de aparência leva o pedido ao recurso de mod.
                if app && recurso_s != 0 && recurso_s != recurso_t {
                    m.override_(recurso_t, recurso_s, cu64(es + l.chave));
                }
            }
        }
        cust_registrar_nome(cust_nome_da_escolha(dono, &chave));
        if cabelo {
            let (tags, n, _) = cust_arr(es + c.tags);
            for k in 0..n {
                let tag = cu64(tags + k * 8);
                if tag != 0 && !m.cores.contains(&tag) {
                    m.cores.push(tag);
                    m.contagem.cores += 1;
                }
            }
        }
    }
}

/// Um par (opção do alvo, opção da fonte) casado: classe diferente ou alvo de tipo desconhecido não recebe (o layout das escolhas é o da
/// classe), e o app casado por curinga só recebe se já tem recurso e definições (curinga não cria opção visível do nada).
unsafe fn cust_mesclar_par(c: &CustCampos, m: &mut CustMescla, t: usize, s: usize, curinga: bool) {
    let Some(tipo) = cust_tipo_da_opcao(c, s) else { return };
    if cust_tipo_da_opcao(c, t).is_none() || rtti::class_of(t as *mut c_void) != rtti::class_of(s as *mut c_void) {
        m.contagem.incompativeis += 1;
        return;
    }
    if curinga && tipo == CustTipo::App && (cu64(t + c.recurso) == 0 || cust_arr(t + c.listas[0].campo).1 == 0) {
        return;
    }
    cust_mesclar_escolhas(c, m, t, s, tipo);
}

/// Opções da fonte nas do alvo. Por nome (`slots = false`): a opção com nome casa a homônima do alvo, e sem homônima vai ao fim; com
/// `link`, as escolhas vêm da opção da fonte com o nome do link. Por slot (`slots = true`): a opção sem nome casa toda opção do alvo de
/// `uiSlot` (e `link`) igual, ou por prefixo quando o texto termina em `*`; nunca anexa.
pub(crate) unsafe fn cust_mesclar_opcoes(c: &CustCampos, m: &mut CustMescla, alvo: usize, fonte: usize, slots: bool) {
    let n_f = cust_arr(fonte).1;
    for j in 0..n_f {
        let h = cust_handle(fonte, j);
        let f = h[0] as usize;
        if f == 0 {
            continue;
        }
        let (nome, slot, link) = (cu64(f + c.nome), cu64(f + c.slot), cu64(f + c.link));
        if !slots {
            if nome == 0 {
                continue;
            }
            let src = if link == 0 { Some(f) } else { (0..n_f).map(|k| cust_opcao(fonte, k)).find(|o| *o != 0 && cu64(*o + c.nome) == link) };
            let mut casou = false;
            for i in 0..cust_arr(alvo).1 {
                let t = cust_opcao(alvo, i);
                if t == 0 || cu64(t + c.nome) != nome {
                    continue;
                }
                casou = true;
                // Link sem a opção de destino na fonte: a homônima existe no alvo, então nada a mesclar nem a anexar.
                match src {
                    Some(s) if s != t => cust_mesclar_par(c, m, t, s, false),
                    _ => {}
                }
            }
            if !casou {
                m.anotar(alvo, CustElemento::Handle);
                if cust_anexar_handle(alvo, h) {
                    m.contagem.opcoes += 1;
                    cust_registrar_nome(nome);
                }
            }
            continue;
        }
        if nome != 0 || (slot == 0 && link == 0) {
            continue;
        }
        let (texto_slot, texto_link) = (crate::cname::resolve_cname(slot), crate::cname::resolve_cname(link));
        let prefixo_slot = if slot != 0 { cust_curinga(&texto_slot) } else { None };
        let prefixo_link = if link != 0 { cust_curinga(&texto_link) } else { None };
        for i in 0..cust_arr(alvo).1 {
            let t = cust_opcao(alvo, i);
            if t == 0 || t == f {
                continue;
            }
            let (slot_t, link_t) = (cu64(t + c.slot), cu64(t + c.link));
            let casa_slot = slot == 0 || cust_casa(slot_t, || crate::cname::resolve_cname(slot_t), slot, prefixo_slot);
            // O curinga do link compara com o link do alvo, não com o slot dele.
            let casa_link = link == 0 || cust_casa(link_t, || crate::cname::resolve_cname(link_t), link, prefixo_link);
            if casa_slot && casa_link {
                cust_mesclar_par(c, m, t, f, prefixo_slot.is_some() || prefixo_link.is_some());
            }
        }
    }
}

/// `index` de cada escolha = posição dela na lista: a tela e o save escolhem pelo índice, e a mescla deslocou as posições.
pub(crate) unsafe fn cust_regenerar(c: &CustCampos, m: &mut CustMescla, opcoes: usize) {
    for i in 0..cust_arr(opcoes).1 {
        let t = cust_opcao(opcoes, i);
        let Some(tipo) = cust_tipo_da_opcao(c, t) else { continue };
        let l = c.listas[tipo as usize];
        let (e, n, _) = cust_arr(t + l.campo);
        for pos in 0..n {
            let campo = (e + pos * l.passo + l.indice) as *mut i32;
            let antes = campo.read_unaligned();
            if antes != pos as i32 {
                m.indices.push((t, tipo, pos, antes));
                campo.write_unaligned(pos as i32);
                m.contagem.indices += 1;
            }
        }
    }
}

/// Mescla as `fontes` (recursos de mod da mesma variante) no recurso base `alvo` de caminho `caminho`, parte por parte (braços, corpo,
/// cabeça). Os passes por slot rodam depois de todas as fontes entrarem por nome: o curinga casa também o que outra fonte anexou.
pub(crate) unsafe fn cust_mesclar_recurso(c: &CustCampos, m: &mut CustMescla, alvo: usize, caminho: u64, fontes: &[usize]) {
    m.contagem.alvos += 1;
    m.contagem.fontes += fontes.len() as u32;
    for k in [2, 1, 0] {
        let (opcoes, grupos) = (alvo + CUST_OPCOES_OFF[k], alvo + CUST_GRUPOS_OFF[k]);
        cust_consertar_caminhos(c, m, caminho, opcoes);
        for f in fontes {
            cust_mesclar_grupos(m, grupos, f + CUST_GRUPOS_OFF[k]);
            m.contagem.expandidas += cust_expandir(c, f + CUST_OPCOES_OFF[k]);
            cust_mesclar_opcoes(c, m, opcoes, f + CUST_OPCOES_OFF[k], false);
        }
        for f in fontes {
            cust_mesclar_opcoes(c, m, opcoes, f + CUST_OPCOES_OFF[k], true);
        }
        cust_regenerar(c, m, opcoes);
    }
}

/// Recurso de customização de um `SharedPtr<ResourceToken>` terminado, com a classe conferida; espera até `espera_ms` o token terminar.
pub(crate) unsafe fn cust_recurso_do_token(c: &CustCampos, token: u64, espera_ms: u64) -> Option<[u64; 2]> {
    if token == 0 || !crate::memoria::is_readable(token as *const c_void, 0x60) {
        return None;
    }
    let inicio = std::time::Instant::now();
    while ((token + 0x58) as *const i32).read_volatile() == 0 && ((token + 0x5c) as *const i32).read_volatile() == 0 {
        if inicio.elapsed().as_millis() as u64 >= espera_ms {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let h = [cu64(token as usize + 0x28), cu64(token as usize + 0x30)];
    (h[0] != 0 && h[1] != 0 && class_is_kind_of(rtti::class_of(h[0] as *mut c_void), c.classe_recurso as *mut c_void)).then_some(h)
}

/// Recurso base de cada variante no sistema de customização: `SharedPtr<ResourceToken>` masculino e feminino.
const CUST_SISTEMA_TOKENS: [usize; 2] = [0x48, 0x58];
/// Caminho do recurso no `ResourceToken`.
const CUST_TOKEN_CAMINHO: usize = 0x48;
/// Espera do token do recurso base no `Initialize`: a carga do save já pediu o recurso, e o hook não pode segurar a thread do jogo.
const CUST_ESPERA_DO_BASE_MS: u64 = 250;

/// No `Initialize` do sistema de customização, antes do original: mescla as fontes de mod nos 2 recursos base, uma vez por processo.
pub(crate) unsafe fn cust_mesclar_no_sistema(sys: u64) -> Option<CustContagem> {
    if !CUST_LIGADO.load(Ordering::Relaxed) || sys == 0 {
        return None;
    }
    let c = cust_campos()?;
    let mut g = CUST.lock().ok()?;
    if g.as_ref().is_some_and(|m| m.mesclado) {
        return None;
    }
    let fontes_por_variante = CUST_FONTES.lock().ok()?.clone();
    let so_montadas = CUST_SO_MONTADAS.load(Ordering::Relaxed);
    let mut m = CustMescla::default();
    for (k, off) in CUST_SISTEMA_TOKENS.into_iter().enumerate() {
        let token = cu64(sys as usize + off);
        let Some(base) = cust_recurso_do_token(&c, token, CUST_ESPERA_DO_BASE_MS) else { continue };
        let fontes: Vec<usize> = fontes_por_variante[k]
            .iter()
            .filter_map(|f| match f {
                // Token da tabela que ainda não terminou fica de fora: esperar cada fonte aqui atrasaria a carga do save.
                CustFonte::Token(t) if !so_montadas => cust_recurso_do_token(&c, t[0], 0).map(|h| h[0] as usize),
                CustFonte::Token(_) => None,
                CustFonte::Recurso(r) => Some(*r),
            })
            .collect();
        // O recurso base fica retido enquanto a mescla existir: a volta escreve nele.
        (*(base[1] as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
        m.alvos.push(base);
        cust_mesclar_recurso(&c, &mut m, base[0] as usize, cu64(token as usize + CUST_TOKEN_CAMINHO), &fontes);
    }
    if !m.cores.is_empty() {
        if let Some(t) = crate::tweakdb::singleton() {
            let id = bwms_primitives::tweak_db_id(CUST_CORES_FLAT);
            let antes = crate::tweakdb::get_flat_array_u64_by_id(t as *const u8, id).unwrap_or_default();
            let novas: Vec<u64> = m.cores.iter().copied().filter(|cor| !antes.contains(cor)).collect();
            if !novas.is_empty() {
                let mut depois = antes.clone();
                depois.extend(novas);
                crate::tweakdb::set_flat_array_u64_by_id(t, id, id, &depois);
                m.cores_antes = Some(antes);
            }
        }
    }
    m.mesclado = true;
    let contagem = m.contagem;
    *g = Some(m);
    crate::veredito_dev!("[a47c] mescla no Initialize: {contagem:?}");
    Some(contagem)
}

/// `RemoveCustomEntries`: as listas voltam ao tamanho de antes, as escolhas substituídas, os índices e os caminhos voltam, e saem os overrides, as cores
/// e os nomes. `false` se não havia mescla.
pub(crate) unsafe fn cust_remover() -> bool {
    let Some(c) = cust_campos() else { return false };
    let Ok(mut g) = CUST.lock() else { return false };
    let Some(mut m) = g.take() else { return false };
    if !m.mesclado {
        return false;
    }
    for (arr, tamanho, el) in m.tamanhos.iter().rev() {
        cust_truncar(*arr, *tamanho, *el);
    }
    for tr in m.trocas.drain(..) {
        let l = c.listas[tr.tipo as usize];
        let (e, n, _) = cust_arr(tr.dono + l.campo);
        let copia = tr.copia.as_ptr() as usize;
        if tr.pos < n {
            rtti::type_assign(l.tipo as *mut c_void, (e + tr.pos * l.passo) as *mut c_void, copia as *const c_void);
        }
        cust_tipo_fn(l.tipo, 0x48, copia);
    }
    for (dono, tipo, pos, antigo) in m.indices.drain(..).rev() {
        let l = c.listas[tipo as usize];
        let (e, n, _) = cust_arr(dono + l.campo);
        if pos < n {
            ((e + pos * l.passo + l.indice) as *mut i32).write_unaligned(antigo);
        }
    }
    for (dono, antigo) in m.caminhos.drain(..).rev() {
        ((dono + c.recurso) as *mut u64).write_unaligned(antigo);
    }
    for (recurso, aparencia) in m.overrides.drain(..) {
        remover_app_override(recurso, aparencia);
    }
    if let (Some(antes), Some(t)) = (m.cores_antes.take(), crate::tweakdb::singleton()) {
        let id = bwms_primitives::tweak_db_id(CUST_CORES_FLAT);
        crate::tweakdb::set_flat_array_u64_by_id(t, id, id, &antes);
    }
    for mut h in m.alvos.drain(..) {
        crate::jogo::objetos::soltar_handle(&mut h);
    }
    if let Ok(mut n) = CUST_NOMES.write() {
        *n = None;
    }
    CUST_NOMES_N.store(0, Ordering::Relaxed);
    true
}

#[cfg(feature = "devtools")]
pub(crate) fn cust_contagem() -> Option<CustContagem> {
    CUST.lock().ok()?.as_ref().filter(|m| m.mesclado).map(|m| m.contagem)
}

/// `(objeto da opção, nome, visível e ligada)` do `Handle<gameuiCharacterCustomizationOption>` em `opcao`.
pub(crate) unsafe fn cust_opcao_do_estado(opcao: u64) -> Option<(usize, u64, bool)> {
    if CUST_NOMES_N.load(Ordering::Relaxed) == 0 || opcao == 0 {
        return None;
    }
    let c = (*CUST_CAMPOS.try_lock().ok()?)?;
    let obj = cu64(opcao as usize) as usize;
    if obj == 0 || !crate::memoria::is_readable(obj as *const c_void, c.opcao_ativo + 1) {
        return None;
    }
    let info = cu64(obj + c.opcao_info) as usize;
    if info == 0 || !crate::memoria::is_readable(info as *const c_void, c.nome + 8) {
        return None;
    }
    let ligada = c.ligado.is_none_or(|o| cu8(info + o) != 0);
    Some((obj, cu64(info + c.nome), cu8(info + c.oculto) == 0 && ligada))
}

/// Objeto da opção de mod visível em `opcao`, ou `None` (sem mescla, ilegível, oculta, desligada ou do jogo).
unsafe fn cust_opcao_de_mod_visivel(opcao: u64) -> Option<(usize, u64)> {
    let (obj, nome, visivel) = cust_opcao_do_estado(opcao)?;
    (visivel && cust_nome_de_mod(nome)).then_some((obj, nome))
}

/// No `InitializeAppOption`/`InitializeMorphOption`, antes do original: a opção de mod entra nas opções do estado do save (array
/// ordenado), senão o motor não a monta e a escolha salva some.
pub(crate) unsafe fn cust_marcar_estado(opcao: u64, estado: u64) -> bool {
    let Some((_, nome)) = cust_opcao_de_mod_visivel(opcao) else { return false };
    if estado == 0 || !crate::memoria::is_readable(estado as *const c_void, 0x10) {
        return false;
    }
    let arr = estado as usize;
    let (e, n, _) = cust_arr(arr);
    if n != 0 && (e == 0 || !crate::memoria::is_readable(e as *const c_void, n * 8)) {
        return false;
    }
    let atual: &[u64] = if n == 0 { &[] } else { std::slice::from_raw_parts(e as *const u64, n) };
    let Err(pos) = cust_posicao_ordenada(atual, nome) else { return false };
    if !cust_reservar_mais_um(arr, 8) {
        return false;
    }
    let e = cust_arr(arr).0 as *mut u64;
    std::ptr::copy(e.add(pos), e.add(pos + 1), n - pos);
    e.add(pos).write_unaligned(nome);
    ((arr + 0xc) as *mut u32).write_unaligned(n as u32 + 1);
    #[cfg(feature = "devtools")]
    CUST_MARCAS[0].fetch_add(1, Ordering::Relaxed);
    true
}

/// No `InitializeSwitcherOption`, depois do original: a opção de mod fica ativa (o motor só ativa as que conhece).
pub(crate) unsafe fn cust_marcar_ativo(opcao: u64) -> bool {
    let Some((obj, _)) = cust_opcao_de_mod_visivel(opcao) else { return false };
    let Some(c) = CUST_CAMPOS.try_lock().ok().and_then(|g| *g) else { return false };
    let ativo = (obj + c.opcao_ativo) as *mut u8;
    if *ativo != 0 {
        return false;
    }
    *ativo = 1;
    #[cfg(feature = "devtools")]
    CUST_MARCAS[1].fetch_add(1, Ordering::Relaxed);
    true
}

/// `bwms-customizations.txt` (`<male|female> | <caminho>`, `bwms_primitives::tabelas::Customizacao`), na carga das tabelas (game thread): desfaz a mescla
/// anterior (a tabela nova substitui a velha inteira), pede o token de cada recurso e liga os hooks. Devolve as fontes.
pub(crate) unsafe fn cust_tabela(texto: &str) -> usize {
    cust_remover();
    // Índice em `CUST_FONTES`: 0 = masculino, 1 = feminino.
    let linhas: Vec<(usize, u64)> = bwms_primitives::tabelas::ler(texto, bwms_primitives::tabelas::Customizacao::ler)
        .flatten()
        .map(|c| (usize::from(c.feminino), bwms_primitives::resource_path_hash(c.caminho)))
        .collect();
    let mut n = 0;
    if let Ok(mut f) = CUST_FONTES.lock() {
        f[0].retain(|x| matches!(x, CustFonte::Recurso(_)));
        f[1].retain(|x| matches!(x, CustFonte::Recurso(_)));
        for (k, h) in &linhas {
            if let Some(t) = crate::jogo::recursos::token_do_path(*h).filter(|t| t[0] != 0) {
                f[*k].push(CustFonte::Token(t));
                n += 1;
            }
        }
    }
    let mescla_tem_fonte = cust_ligar_mescla(&CUST_LIGADO, n, || cust_campos().is_some());
    if mescla_tem_fonte {
        crate::corpo_e_rosto::install_a47_produto();
    }
    crate::veredito_dev!("[a47c] customizations: {} linha(s), {n} token(s) pedido(s), mescla ligada {mescla_tem_fonte}", linhas.len());
    n
}

/// Grava a mescla ligada nos dois sentidos: só com fonte da tabela e os campos do RTTI. Tabela sem fonte (vazia, ou ausente, que a recarga passa vazia)
/// desliga a mescla da anterior, pra o `Initialize` não mesclar fonte que saiu da tabela.
pub(crate) fn cust_ligar_mescla(ligado: &std::sync::atomic::AtomicBool, fontes: usize, campos: impl FnOnce() -> bool) -> bool {
    let mescla_tem_fonte = fontes > 0 && campos();
    ligado.store(mescla_tem_fonte, Ordering::Relaxed);
    mescla_tem_fonte
}

/// Fonte pedida por tabela (`Token`) ainda guardada; a de teste (`Recurso`) não conta.
pub(crate) fn cust_tem_fonte_de_tabela(fontes: &[Vec<CustFonte>; 2]) -> bool {
    fontes.iter().flatten().any(|f| matches!(f, CustFonte::Token(_)))
}

/// Carga das tabelas sem `bwms-customizations.txt` (o `bwms install` apaga a tabela que ficaria sem linha): com fonte da tabela anterior, a tabela entra
/// vazia, desfaz a mescla, tira os tokens das fontes e desliga `CUST_LIGADO`. Sem fonte anterior não há o que desfazer, e o boot
/// sem customizations não resolve os campos do RTTI.
pub(crate) unsafe fn cust_tabela_ausente() {
    if CUST_FONTES.lock().is_ok_and(|f| cust_tem_fonte_de_tabela(&f)) {
        cust_tabela("");
    }
}


/// `BwmsPuppetStateFireOnce() -> Bool` — teste do estado do boneco: dispara `MenuScenario_CharacterCustomizationMirror`
/// (SwitchToScenario, native pública em `inkMenuScenario`) SEM exigir que o usuário caminhe até um
/// espelho no mundo (world-nav) — tenta exercitar `GenitalsController::OnAttach`/`HairstyleController::OnDetach`
/// (hooks já instalados em `selftest.rs::install_postload_hooks`, vmaddrs GENITAL_ONATTACH_VM/
/// HAIRSTYLE_ONDETACH_VM) a partir do `MenuScenario_Idle` (gameplay real). Gate = marcador
/// `~/.bwms-puppetstate-fire` (opt-in, consumido 1x por processo via `PUPPETSTATE_FIRED` — evita
/// tentar de novo/loop se a troca de scenario falhar e o jogo voltar pro Idle). EXPERIMENTAL: o
/// mecanismo normal (interação com um device de espelho no mundo) provavelmente prepara estado de
/// cena que este atalho pula — pode não produzir puppet nenhum (sem crash) ou, na pior hipótese,
/// crashar; ambos os resultados são informativos e serão documentados.
#[cfg(feature = "devtools")]
pub(crate) static PUPPETSTATE_FIRED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_puppetstate_fire_once(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let want = crate::marcador_de_dev(".bwms-puppetstate-fire");
    let fire = want && !PUPPETSTATE_FIRED.swap(true, Ordering::Relaxed);
    if fire {
        crate::log_dev!("[axl-puppet] BwmsPuppetStateFireOnce -> true (disparando MenuScenario_CharacterCustomizationMirror sem world-nav)");
    }
    write_bool_ret(out, fire);
}

/// `BwmsPuppetStateShouldRecover() -> Bool` — 2026-08-05, achado: `BwmsPuppetStateArmRecovery`
/// era chamada INCONDICIONALMENTE em toda entrada de `MenuScenario_CharacterCustomizationMirror`,
/// inclusive quando o jogador chega lá por um espelho REAL do mundo (world-nav genuíno, sem
/// `BwmsPuppetStateFireOnce`) — nesse caso o device físico já prepara o puppet de verdade e
/// `OnCCOPuppetReady` chegaria sozinho, mas a saída forçada em ~180 ticks (3-6s) interrompia a
/// cena ANTES disso, provavelmente matando a chance real de `GenitalsController::OnAttach`/
/// `HairstyleController::OnDetach` dispararem. Fix: só recuperar quando o disparo foi o SINTÉTICO
/// (`PUPPETSTATE_FIRED`==true) — espelho real fica com o fluxo 100% vanilla, sem interferência.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_puppetstate_should_recover(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, PUPPETSTATE_FIRED.load(Ordering::Relaxed));
}

#[cfg(feature = "devtools")]
pub(crate) const PUPPETSTATE_RECOVER_DELAY_TICKS: u64 = 180; // ~3-6s, mesmo ritmo de HANDLE_EVENTS_DELAY_TICKS

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_puppetstate_arm_recovery(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let scenario = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let armed = rtti::sane(scenario);
    if armed {
        PUPPETSTATE_MIRROR_PTR.store(scenario as usize, Ordering::Relaxed);
        let target = crate::ticks() + PUPPETSTATE_RECOVER_DELAY_TICKS;
        PUPPETSTATE_RECOVER_AT.store(target, Ordering::Relaxed);
        crate::log_dev!(
            "[axl-puppet] BwmsPuppetStateArmRecovery: scenario={scenario:p} recuperação agendada pra tick>={target} (atual={})",
            crate::ticks()
        );
    } else {
        crate::log_dev!("[axl-puppet] BwmsPuppetStateArmRecovery: scenario={scenario:p} inválido, recuperação NÃO agendada");
    }
    write_bool_ret(out, armed);
}

/// `Entity.ApplyMorphTarget` (2026-09-11): `MorphTargetManager::ApplyMorphTarget` = `0x10197f17c`
/// (`x0` manager, `x1` target, `x2` region, `s0` value, `w3` bool), achado offline
/// (`proofs/20260911-*-24-applymorphtarget-identificado.log`). O manager é o 1º componente da entidade cuja
/// classe descende de `entMorphTargetManagerComponent`; os pesos ficam num `DynArray<entMorphTargetWeightEntry>` em
/// `+0x98` (entrada de 0x18: target, region, peso f32 em `+0x10`).
pub(crate) const MORPH_APPLY_VM: u64 = 0x1_0197_f17c;

#[cfg(feature = "devtools")]
pub(crate) const MORPH_PESOS_OFF: usize = 0x98;

#[cfg(feature = "devtools")]
pub(crate) const MORPH_ENTRADA_TAM: usize = 0x18;

/// Teto de componentes percorridos: acima disso a lista é lixo, não entidade.
const MORPH_COMPONENTES_MAX: usize = 10_000;

/// O 1º componente da entidade cuja classe deriva de `entMorphTargetManagerComponent`; nulo sem ele ou com entidade ilegível.
pub(crate) unsafe fn morph_manager_de(entity: *mut c_void) -> *mut c_void {
    if entity.is_null() || !crate::memoria::is_readable(entity as *const c_void, 0xB0) {
        return std::ptr::null_mut();
    }
    let componentes = (entity as *const u8).add(0xA0) as *const c_void;
    let (entradas, n) = (rd_u64(componentes, 0), rd_u32(componentes, 0x0C) as usize);
    if entradas == 0 {
        return std::ptr::null_mut();
    }
    let alvo = cname("entMorphTargetManagerComponent");
    for i in 0..n.min(MORPH_COMPONENTES_MAX) {
        let Some(comp) = crate::memoria::read_u64(entradas as usize + i * 16) else { break };
        let comp = comp as *mut c_void;
        if !comp.is_null() && classe_deriva_de_nome(rtti::class_of(comp), alvo) {
            return comp;
        }
    }
    std::ptr::null_mut()
}

/// Entradas de peso do manager: (início, quantidade).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn morph_entradas(manager: *mut c_void) -> Option<(*const u8, usize)> {
    if manager.is_null() || !crate::memoria::is_readable(manager as *const c_void, MORPH_PESOS_OFF + 0x10) {
        return None;
    }
    let arr = (manager as *const u8).add(MORPH_PESOS_OFF);
    let entradas = (arr as *const u64).read_unaligned() as *const u8;
    let n = (arr.add(0x0C) as *const u32).read_unaligned() as usize;
    if n == 0 {
        return Some((entradas, 0));
    }
    if entradas.is_null() || n > 4096 || !crate::memoria::is_readable(entradas as *const c_void, n * MORPH_ENTRADA_TAM) {
        return None;
    }
    Some((entradas, n))
}

#[cfg(feature = "devtools")]
pub(crate) unsafe fn morph_peso(manager: *mut c_void, target: u64, region: u64) -> Option<f32> {
    let (e, n) = morph_entradas(manager)?;
    (0..n)
        .map(|i| e.add(i * MORPH_ENTRADA_TAM))
        .find(|p| (*p as *const u64).read_unaligned() == target && (p.add(8) as *const u64).read_unaligned() == region)
        .map(|p| (p.add(0x10) as *const f32).read_unaligned())
}

/// `BwmsEntityApplyMorphTarget(entity, target, region, value) -> Bool`: `true` quando a entidade tem o manager, e então o motor aplica
/// o peso (diferença abaixo de 0,01 o motor ignora, e o retorno continua `true`).
pub(crate) unsafe extern "C" fn tramp_entity_apply_morph_target(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let region = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let valor = f32::from_bits(args.get(3).map(|(v, _)| *v as u32).unwrap_or(0));
    let manager = morph_manager_de(entity);
    if manager.is_null() {
        write_bool_ret(out, false);
        return;
    }
    // TODO(B4 §B.7): efeito do 5º argumento (`w3`) não medido; medir comparando `morph_peso` depois de chamar com 0 e com 1 no mesmo alvo.
    let aplicar: unsafe extern "C" fn(*mut c_void, u64, u64, f32, u32) = std::mem::transmute(crate::rebase(MORPH_APPLY_VM));
    aplicar(manager, target, region, valor, 0);
    write_bool_ret(out, true);
}

/// `BwmsEntityGetMorphTargetWeight(entity: ref<Entity>, target: CName, region: CName) -> Float` — -1 sem o par.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_entity_get_morph_target_weight(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let region = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let peso = morph_peso(morph_manager_de(entity), target, region).unwrap_or(-1.0);
    write_uint_ret(out, u64::from(peso.to_bits()), 4);
}

/// `BwmsEntityGetMorphTargetCount(entity: ref<Entity>) -> Int32` — entradas de peso do manager; -1 sem manager.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_entity_get_morph_target_count(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let n = morph_entradas(morph_manager_de(entity)).map(|(_, n)| n as i32).unwrap_or(-1);
    write_uint_ret(out, u64::from(n as u32), 4);
}

/// `BwmsEntityGetMorphTargetName(entity: ref<Entity>, index: Int32, region: Bool) -> CName` — target (ou region) da
/// entrada `index`; `None` fora da faixa.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_entity_get_morph_target_name(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let indice = args.get(1).map(|(v, _)| *v as u32 as i32).unwrap_or(-1);
    let regiao = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let nome = match morph_entradas(morph_manager_de(entity)) {
        Some((e, n)) if indice >= 0 && (indice as usize) < n => {
            (e.add(indice as usize * MORPH_ENTRADA_TAM + if regiao { 8 } else { 0 }) as *const u64).read_unaligned()
        }
        _ => 0,
    };
    write_uint_ret(out, nome, 8);
}


#[cfg(feature = "devtools")]
pub(crate) unsafe fn register_puppetstate_recovery_native(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["handle:IScriptable"]);
    let ok = if params_compostos(params) {
        register_global_composed(reg, proto, params, "BwmsPuppetStateArmRecovery", "BwmsPuppetStateArmRecovery", tramp_puppetstate_arm_recovery)
    } else {
        crate::log_dev!("[reg] register_puppetstate_recovery_native: compose_params_from_types falhou (handle:IScriptable)");
        false
    };
    crate::log_dev!("[reg] register_all: axl-puppet-state-apply BwmsPuppetStateArmRecovery={ok}");
}

/// Linhas de `bwms-resscopes.txt` (`<hash do escopo> | <caminho do alvo>`, `bwms_primitives::tabelas::Resscope`, do
/// `bwms_primitives::instalacao::tabelas_de_runtime::emit_resscopes`): `(escopo, alvo)` em hash. Linha fora do formato fica de fora.
pub(crate) fn es20_escopos_linhas(texto: &str) -> Vec<(u64, u64)> {
    bwms_primitives::tabelas::ler(texto, bwms_primitives::tabelas::Resscope::ler).flatten().map(|s| (s.escopo, bwms_primitives::resource_path_hash(s.alvo))).collect()
}

pub(crate) fn es20_escopos() -> &'static std::sync::RwLock<Es20Escopos> {
    static ESCOPOS: std::sync::OnceLock<std::sync::RwLock<Es20Escopos>> = std::sync::OnceLock::new();
    ESCOPOS.get_or_init(Default::default)
}

/// Troca a tabela de escopos pelas linhas do texto; devolve quantos pares entraram.
pub(crate) fn es20_carregar_escopos(texto: &str) -> usize {
    let linhas = es20_escopos_linhas(texto);
    if let Ok(mut g) = es20_escopos().write() {
        g.clear();
        for (escopo, alvo) in &linhas {
            g.entry(*escopo).or_default().insert(*alvo);
        }
    }
    linhas.len()
}

/// `ResourceMetaExtension::InScope(escopo, alvo)`.
pub(crate) fn es20_no_escopo(escopo: u64, alvo: u64) -> bool {
    es20_escopos().read().map(|g| g.get(&escopo).is_some_and(|s| s.contains(&alvo))).unwrap_or(false)
}


/// Tipo de uma opção: `gameuiAppearanceInfo` (lista `definitions`), `gameuiMorphInfo` (`morphNames`), `gameuiSwitcherInfo` (`options`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CustTipo {
    App = 0,
    Morph = 1,
    Switcher = 2,
}

/// Campos da customização pelo RTTI.
#[derive(Clone, Copy)]
pub(crate) struct CustCampos {
    pub classes: [usize; 3],
    pub nome: usize,
    pub slot: usize,
    pub link: usize,
    pub oculto: usize,
    pub ligado: Option<usize>,
    pub recurso: usize,
    pub listas: [CustLista; 3],
    /// `tags` (`redTagList`) e `icon` da definição.
    pub tags: usize,
    pub icone: usize,
    /// `info` e `isActive` do `gameuiCharacterCustomizationOption`.
    pub opcao_info: usize,
    pub opcao_ativo: usize,
    pub tipo_string: usize,
    pub tipo_cnames: usize,
    /// Tipos dos arrays do recurso: opções e grupos; o grupo (`gameuiOptionsGroup`).
    pub tipo_arr_opcoes: usize,
    pub tipo_arr_grupos: usize,
    pub tipo_grupo: usize,
    pub classe_recurso: usize,
}

/// Offsets no recurso (`0x104cd5458`, DATABASE): opções da cabeça, do corpo e dos braços, e os grupos na mesma ordem.
pub(crate) const CUST_OPCOES_OFF: [usize; 3] = [0x40, 0x50, 0x60];

pub(crate) const CUST_GRUPOS_OFF: [usize; 3] = [0x70, 0x80, 0x90];

pub(crate) const CUST_CORES_FLAT: &str = "ItemFactory.HairColors.hairColors";

/// O que cada `DynArray` tocado guarda, pra voltar ao tamanho de antes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CustElemento {
    Handle,
    CName,
    Valor { tipo: usize, passo: usize },
}

/// Chave de uma escolha: `name` (CName) no app, `localizedName` (String) nos outros.
#[derive(PartialEq, Eq, Debug, Clone)]
pub(crate) enum CustChave {
    Nome(u64),
    Texto(String),
}

/// Contagem da última mescla.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct CustContagem {
    pub alvos: u32,
    pub fontes: u32,
    pub grupos: u32,
    pub opcoes: u32,
    pub escolhas: [u32; 3],
    pub trocas: u32,
    pub expandidas: u32,
    pub overrides: u32,
    pub cores: u32,
    pub indices: u32,
    pub caminhos: u32,
    pub incompativeis: u32,
}

#[derive(Default)]
pub(crate) struct CustMescla {
    /// Cada `DynArray` tocado, com o tamanho de antes, na ordem em que foi tocado.
    pub(crate) tamanhos: Vec<(usize, usize, CustElemento)>,
    pub(crate) trocas: Vec<CustTroca>,
    pub(crate) indices: Vec<(usize, CustTipo, usize, i32)>,
    pub(crate) caminhos: Vec<(usize, u64)>,
    pub(crate) overrides: Vec<(u64, u64)>,
    pub(crate) cores: Vec<u64>,
    pub(crate) cores_antes: Option<Vec<u64>>,
    pub(crate) alvos: Vec<[u64; 2]>,
    pub(crate) mesclado: bool,
    pub(crate) contagem: CustContagem,
}

impl CustMescla {
    pub(crate) fn anotar(&mut self, arr: usize, el: CustElemento) {
        if !self.tamanhos.iter().any(|t| t.0 == arr) {
            self.tamanhos.push((arr, unsafe { cust_arr(arr).1 }, el));
        }
    }
    pub(crate) fn override_(&mut self, recurso: u64, substituto: u64, aparencia: u64) {
        registrar_app_override(recurso, substituto, aparencia);
        if !self.overrides.contains(&(recurso, aparencia)) {
            self.overrides.push((recurso, aparencia));
        }
        self.contagem.overrides += 1;
    }
}

/// Liga a mescla no hook do `Initialize` (tabela com fonte, ou teste).
pub(crate) static CUST_LIGADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);


pub(crate) const MORPHTGT_POSTLOAD_VM: u64 = 0x1_051c_a1a4;

pub(crate) const A23_CORES_DO_CABELO: &str = "ItemFactory.HairColors.hairColors";

pub(crate) const A47_GET_VM: [u64; 6] = [0x1_04bb_f004, 0x1_04bb_f438, 0x1_04bb_f6f8, 0x1_04bb_f9b8, 0x1_04bb_fc78, 0x1_04bb_ff38];

pub(crate) const A47_GET_PROLOGO: u32 = 0xd101_c3ff;

pub(crate) static ORIG_A47_GET: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// `InitializeAppOption`/`InitializeMorphOption`/`InitializeSwitcherOption`: endereço, prólogo.
pub(crate) const A47_INIT: [(u64, u32); 3] = [(0x1_04bd_3f5c, 0xd102_03ff), (0x1_04bd_448c, 0xd102_83ff), (0x1_04bd_4a70, 0xd101_03ff)];

pub(crate) static ORIG_A47_INIT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// Teto de descritores na saída de um `Get*Appearances`: acima disso o array é lixo.
const A47_DESCRITORES_MAX: usize = 4096;

/// Corpo dos 6 ganchos `Get{Head,Body,Arms}Appearances{1,2}`: o original preenche a saída (`DynArray` de descritores `[recurso, aparência]`)
/// e a troca de aparência de recurso vale nela. Tabela vazia (o caso sem mod) repassa sem ir ao kernel.
pub(crate) unsafe fn a47_get(k: usize, estado: u64, grupo: u64, fpp: u64, saida: *mut u64) -> u64 {
    let original: unsafe extern "C" fn(u64, u64, u64, *mut u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_GET[k]));
    let r = original(estado, grupo, fpp, saida);
    #[cfg(any(test, feature = "devtools"))]
    A47_GET_CHAMADAS[k].fetch_add(1, Ordering::Relaxed);
    if crate::aparencia::APP_OVERRIDES_N.load(Ordering::Relaxed) == 0 {
        return r;
    }
    if saida.is_null() || !crate::memoria::is_readable(saida as *const c_void, 0x10) {
        return r;
    }
    let (entradas, n) = (saida.read_unaligned() as *mut u64, ((saida as *const u8).add(0x0C) as *const u32).read_unaligned() as usize);
    if entradas.is_null() || n == 0 || n > A47_DESCRITORES_MAX {
        return r;
    }
    let descritor = |i: usize| (entradas.add(i * 2) as *const [u64; 2]).read_unaligned();
    #[cfg(feature = "devtools")]
    let antes: Vec<[u64; 2]> = (0..n.min(64)).map(descritor).collect();
    let trocados = crate::aparencia::aplicar_app_overrides(entradas, n);
    #[cfg(any(test, feature = "devtools"))]
    {
        A47_GET_DESCRITORES[k].fetch_add(n as u64, Ordering::Relaxed);
        A47_GET_TROCADOS[k].fetch_add(trocados as u64, Ordering::Relaxed);
    }
    #[cfg(feature = "devtools")]
    if let Some(i) = (0..antes.len()).find(|i| descritor(*i) != antes[*i]) {
        if let Ok(mut amostra) = A47_GET_AMOSTRA.lock() {
            if amostra.len() < 8 {
                amostra.push((k, grupo, fpp, descritor(i)));
            }
        }
    }
    #[cfg(not(feature = "devtools"))]
    let _ = descritor;
    #[cfg(not(any(test, feature = "devtools")))]
    let _ = trocados;
    r
}

macro_rules! a47_get_hook {
    ($nome:ident, $k:expr) => {
        unsafe extern "C" fn $nome(estado: u64, grupo: u64, fpp: u64, saida: *mut u64) -> u64 {
            a47_get($k, estado, grupo, fpp, saida)
        }
    };
}

a47_get_hook!(a47_get0, 0);

a47_get_hook!(a47_get1, 1);

a47_get_hook!(a47_get2, 2);

a47_get_hook!(a47_get3, 3);

a47_get_hook!(a47_get4, 4);

a47_get_hook!(a47_get5, 5);

/// Contagem do teste 47a por `Initialize*Option` (0 app, 1 morph, 2 switcher): chamadas, partes vistas e maior estado.
#[cfg(feature = "devtools")]
unsafe fn a47_anotar_init(k: usize, parte: Option<u64>, estado: Option<u64>) {
    A47_INIT_CHAMADAS[k].fetch_add(1, Ordering::Relaxed);
    if let Some(p) = parte.filter(|p| *p < 3) {
        A47_INIT_PARTES[k].fetch_or(1 << p, Ordering::Relaxed);
    }
    if let Some(e) = estado {
        A47_INIT_ESTADO_MAX[k].fetch_max(a47_tamanho(e), Ordering::Relaxed);
    }
}

/// `InitializeAppOption(sys, parte, &opção, &estado, &slots)`: a opção de mod entra no estado antes do original montá-la.
pub(crate) unsafe extern "C" fn a47_init_app(sys: u64, parte: u64, opcao: u64, estado: u64, slots: u64) -> u64 {
    cust_marcar_estado(opcao, estado);
    #[cfg(feature = "devtools")]
    a47_anotar_init(0, Some(parte), Some(estado));
    let original: unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[0]));
    original(sys, parte, opcao, estado, slots)
}

/// `InitializeMorphOption(sys, &opção, &estado, &slots)`: como o app, sem parte.
pub(crate) unsafe extern "C" fn a47_init_morph(sys: u64, opcao: u64, estado: u64, slots: u64) -> u64 {
    cust_marcar_estado(opcao, estado);
    #[cfg(feature = "devtools")]
    a47_anotar_init(1, None, Some(estado));
    let original: unsafe extern "C" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[1]));
    original(sys, opcao, estado, slots)
}

/// `InitializeSwitcherOption(sys, parte, &opção, índice, &slots)`: o original monta, e a opção de mod fica ativa depois.
pub(crate) unsafe extern "C" fn a47_init_switcher(sys: u64, parte: u64, opcao: u64, indice: u64, slots: u64) -> u64 {
    let original: unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[2]));
    let r = original(sys, parte, opcao, indice, slots);
    cust_marcar_ativo(opcao);
    #[cfg(feature = "devtools")]
    a47_anotar_init(2, Some(parte), None);
    r
}

/// Os 3 ganchos dos `Initialize*Option`, na ordem de [`A47_INIT`] e [`ORIG_A47_INIT`].
fn a47_ganchos_de_init() -> [*mut c_void; 3] {
    [
        a47_init_app as unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 as *mut c_void,
        a47_init_morph as unsafe extern "C" fn(u64, u64, u64, u64) -> u64 as *mut c_void,
        a47_init_switcher as unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 as *mut c_void,
    ]
}

/// Gancho no `vm` conferindo o prólogo; já posto (o `orig` publicado) conta como posto.
unsafe fn a47_instalar_um((vm, prologo): (u64, u32), destino: *mut c_void, orig: &AtomicU64) -> Option<bool> {
    if orig.load(Ordering::Relaxed) != 0 {
        return Some(true);
    }
    instalar_conferindo_prologo(vm, prologo, destino, orig)
}

/// No `on_load`, sempre: a troca de aparência de recurso nos 6 `Get*Appearances` (cabeça, corpo e braços do V). Com a tabela vazia os
/// ganchos só repassam.
pub(crate) unsafe fn install_a47a() {
    type Get = unsafe extern "C" fn(u64, u64, u64, *mut u64) -> u64;
    let ganchos: [Get; 6] = [a47_get0, a47_get1, a47_get2, a47_get3, a47_get4, a47_get5];
    let gets: Vec<Option<bool>> =
        (0..6).map(|k| a47_instalar_um((A47_GET_VM[k], A47_GET_PROLOGO), ganchos[k] as *mut c_void, &ORIG_A47_GET[k])).collect();
    #[cfg(feature = "devtools")]
    let (teste, inits, pares) = if crate::marcador_individual("a47a-teste") {
        A47_TESTE.store(true, Ordering::Relaxed);
        let ganchos_init = a47_ganchos_de_init();
        let inits: Vec<Option<bool>> = (0..3).map(|k| a47_instalar_um(A47_INIT[k], ganchos_init[k], &ORIG_A47_INIT[k])).collect();
        // Pares gravados pelo teste anterior: os `Get*Appearances` rodam na carga, antes de qualquer comando do canal.
        let pares = a11_pares_gravados();
        for (recurso, aparencia) in &pares {
            let alias = a11_alias(*recurso);
            crate::arquivos_do_jogo::reslink_add(alias, *recurso);
            crate::aparencia::registrar_app_override(*recurso, alias, *aparencia);
        }
        (true, inits, pares.len())
    } else {
        (false, Vec::new(), 0)
    };
    #[cfg(not(feature = "devtools"))]
    let (teste, inits, pares): (bool, Vec<Option<bool>>, usize) = (false, Vec::new(), 0);
    crate::veredito_dev!("[a47a] hooks no on_load: Get*Appearances {gets:?}, Initialize*Option {inits:?} (teste {teste}, {pares} par(es) com troca antes da carga)");
    let _ = (teste, inits, pares);
}

pub(crate) const A47_INITIALIZE: (u64, u32) = (0x1_04bc_779c, 0xd101_c3ff);

pub(crate) const A47_UNINITIALIZE: (u64, u32) = (0x1_04bc_83a8, 0xa9be_4ff4);

pub(crate) static ORIG_A47_INITIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_A47_UNINITIALIZE: AtomicU64 = AtomicU64::new(0);

/// `Initialize(sys, puppet, isMale, functor)`: a mescla entra antes do original, que monta as opções a partir dos recursos base.
pub(crate) unsafe extern "C" fn a47_initialize(sys: u64, puppet: u64, masc: u64, functor: u64) {
    #[cfg(feature = "devtools")]
    A47_CICLO_CHAMADAS[0].fetch_add(1, Ordering::Relaxed);
    cust_mesclar_no_sistema(sys);
    let original: unsafe extern "C" fn(u64, u64, u64, u64) = std::mem::transmute(original_publicado(&ORIG_A47_INITIALIZE));
    original(sys, puppet, masc, functor)
}

/// `Uninitialize(sys)`: só repassa. A mescla fica nos recursos base, que seguem em memória para o próximo `Initialize`.
pub(crate) unsafe extern "C" fn a47_uninitialize(sys: u64) {
    #[cfg(feature = "devtools")]
    A47_CICLO_CHAMADAS[1].fetch_add(1, Ordering::Relaxed);
    let original: unsafe extern "C" fn(u64) = std::mem::transmute(original_publicado(&ORIG_A47_UNINITIALIZE));
    original(sys)
}

/// Ganchos do ciclo do sistema de customização: `Initialize`, `Uninitialize` e os 3 `Initialize*Option`, cada um uma vez.
pub(crate) unsafe fn a47_instalar_ciclo() -> (Option<bool>, Option<bool>, Vec<Option<bool>>) {
    let i = a47_instalar_um(A47_INITIALIZE, a47_initialize as unsafe extern "C" fn(u64, u64, u64, u64) as *mut c_void, &ORIG_A47_INITIALIZE);
    let u = a47_instalar_um(A47_UNINITIALIZE, a47_uninitialize as unsafe extern "C" fn(u64) as *mut c_void, &ORIG_A47_UNINITIALIZE);
    let ganchos = a47_ganchos_de_init();
    let inits = (0..3).map(|k| a47_instalar_um(A47_INIT[k], ganchos[k], &ORIG_A47_INIT[k])).collect();
    (i, u, inits)
}

/// Liga os ganchos da mescla quando a tabela `bwms-customizations.txt` tem fonte.
pub(crate) unsafe fn install_a47_produto() {
    let (i, u, inits) = a47_instalar_ciclo();
    crate::veredito_dev!("[a47c] hooks da mescla: Initialize {i:?}, Uninitialize {u:?}, Initialize*Option {inits:?}");
    let _ = (i, u, inits);
}


#[cfg(test)]
mod estado_mais_novo {
    use super::*;

    /// Endereço de evento reaproveitado: o filtro lê a tecla de agora (66), não a 1ª que usou o endereço (65).
    #[test]
    fn filtro_de_tecla_le_o_estado_mais_novo() {
        let inst = 0x7000_0000_1230usize;
        {
            let mut s = KEYINPUT_STATES.lock().unwrap();
            s.retain(|e| e.0 != inst);
            s.push((inst, 1, 65, false, false, false));
            s.push((inst, 1, 66, false, false, false));
        }
        let args = [rtti::Arg::Handle(inst as *mut c_void, std::ptr::null_mut())];
        let lido = unsafe { event_input_key_action(&args) };
        KEYINPUT_STATES.lock().unwrap().retain(|e| e.0 != inst);
        assert_eq!(lido, (Some(66), Some(1)));
    }
}

#[cfg(test)]
mod caminho_por_padrao {
    use super::*;

    /// O despacho casa o padrão do texto registrado pro hash do alvo; texto novo no mesmo hash troca o padrão.
    #[test]
    fn padrao_registrado_casa_no_despacho() {
        let alvo = caminho_registrar(r"regex:^base\\bwms_teste_c137\\.*\.mesh$");
        let dentro = caminho_registrar(r"base\bwms_teste_c137\a.mesh");
        let fora = caminho_registrar(r"base\bwms_teste_c137\a.ent");
        assert!(caminho_casa(alvo, dentro));
        assert!(!caminho_casa(alvo, fora));
        assert!(!caminho_casa(alvo, 0x1234_5678_9abc_def0), "evento sem texto registrado não casa padrão");
        assert!(caminho_casa(dentro, dentro), "literal compara por hash");
        assert!(!caminho_casa(dentro, fora));
        let glob = caminho_registrar(r"base\bwms_teste_c137\*.ent");
        assert!(caminho_casa(glob, fora));
        assert!(!caminho_casa(glob, dentro));
        let recusado = caminho_registrar(r"regex:^base\\bwms_teste_c137\\\d$");
        let um = caminho_registrar(r"base\bwms_teste_c137\1");
        assert!(!caminho_casa(recusado, um), "padrão recusado não casa nada");
        // Mesmo hash (o hash ignora caixa), texto que não é padrão: o prefixo `regex:` é sensível a caixa.
        let exato = caminho_registrar(r"regex:^base\\bwms_teste_c137\\b\.mesh$");
        let b = caminho_registrar(r"base\bwms_teste_c137\b.mesh");
        assert!(caminho_casa(exato, b));
        assert_eq!(caminho_registrar(r"REGEX:^BASE\\bwms_teste_c137\\b\.MESH$"), exato);
        assert!(!caminho_casa(exato, b));
    }
}

#[cfg(test)]
mod fronteira_dos_alvos {
    use super::*;

    /// O critério do `InputTarget` sai da tabela por valor, `IK_None`/`IACT_None` saem como não-setado, e o 4º campo do `.Axis`
    /// (threshold em bits) nunca chega como ação: o alvo de eixo entra com ação não-setada.
    #[test]
    fn criterio_de_input_eixo_entra_com_acao_curinga() {
        let (tecla, sem_filtro, eixo) = (0x7000_0000_2310usize, 0x7000_0000_2320usize, 0x7000_0000_2330usize);
        {
            let mut s = INPUTTARGET_STATES.lock().unwrap();
            s.retain(|e| ![tecla, sem_filtro, eixo].contains(&e.0));
            s.push((tecla, InputTargetCriterion::Key, 49, IACT_PRESS as i64));
            s.push((sem_filtro, InputTargetCriterion::Key, IK_NONE as i64, IACT_NONE as i64));
            s.push((eixo, InputTargetCriterion::Axis, 49, 0x3F00_0000));
        }
        let lido = unsafe { inputtarget_criteria(&[tecla, sem_filtro, eixo]) };
        INPUTTARGET_STATES.lock().unwrap().retain(|e| ![tecla, sem_filtro, eixo].contains(&e.0));
        assert_eq!(lido, vec![(Some(49), Some(IACT_PRESS as u32)), (None, None), (Some(49), None)]);
    }

    /// O despacho vivo do `InputTarget` bloqueia o evento sem tecla registrada quando o alvo tem campo setado; a cópia só
    /// de teste (`input_target_targets_allow`) deixa passar — quem vale é esta.
    #[test]
    fn alvo_de_tecla_bloqueia_evento_sem_dado() {
        let (alvo, alvo_sem_acao, alvo_eixo) = (0x7000_0000_2410usize, 0x7000_0000_2420usize, 0x7000_0000_2430usize);
        let (sem_dado, aperto, soltura) = (0x7000_0000_2440usize, 0x7000_0000_2450usize, 0x7000_0000_2460usize);
        {
            let mut s = INPUTTARGET_STATES.lock().unwrap();
            s.retain(|e| ![alvo, alvo_sem_acao, alvo_eixo].contains(&e.0));
            s.push((alvo, InputTargetCriterion::Key, 49, IACT_PRESS as i64));
            s.push((alvo_sem_acao, InputTargetCriterion::Key, 49, IACT_NONE as i64));
            s.push((alvo_eixo, InputTargetCriterion::Axis, 49, 0x3F00_0000));
        }
        {
            let mut s = KEYINPUT_STATES.lock().unwrap();
            s.retain(|e| ![sem_dado, aperto, soltura].contains(&e.0));
            s.push((aperto, IACT_PRESS, 49, false, false, false));
            s.push((soltura, IACT_RELEASE, 49, false, false, false));
        }
        let casa = |p: usize, ev: usize| unsafe {
            alvo_casa(p, FamiliaDoAlvo::Tecla, TipoDoEvento::Tecla, &[rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())])
        };
        let obtido = [casa(alvo, sem_dado), casa(alvo, aperto), casa(alvo, soltura), casa(alvo_sem_acao, soltura), casa(alvo_eixo, soltura)];
        INPUTTARGET_STATES.lock().unwrap().retain(|e| ![alvo, alvo_sem_acao, alvo_eixo].contains(&e.0));
        KEYINPUT_STATES.lock().unwrap().retain(|e| ![sem_dado, aperto, soltura].contains(&e.0));
        assert_eq!(obtido, [false, true, false, true, true]);
    }

    /// Alvo de tecla ou de componente com todos os campos não-setados casa sem dado do evento;
    /// com um campo setado, o evento sem dado continua filtrado.
    #[test]
    fn alvo_todo_curinga_casa_sem_dado_do_evento() {
        let (tecla_curinga, tecla_setada) = (0x7000_0000_2610usize, 0x7000_0000_2620usize);
        let (componente_curinga, componente_setado, componente_lido) = (0x7000_0000_2630usize, 0x7000_0000_2640usize, 0x7000_0000_2650usize);
        let (sem_tecla, sem_componente, com_componente) = (0x7000_0000_2660usize, 0x7000_0000_2670usize, 0x7000_0000_2680usize);
        let mut componente = [0u64; 0x68 / 8];
        componente[0x40 / 8] = 0xC0FE;
        {
            let mut s = INPUTTARGET_STATES.lock().unwrap();
            s.retain(|e| ![tecla_curinga, tecla_setada].contains(&e.0));
            s.push((tecla_curinga, InputTargetCriterion::Key, IK_NONE as i64, IACT_NONE as i64));
            s.push((tecla_setada, InputTargetCriterion::Key, 49, IACT_NONE as i64));
        }
        {
            let mut s = COMPONENTTARGET_STATES.lock().unwrap();
            s.retain(|e| ![componente_curinga, componente_setado, componente_lido].contains(&e.0));
            s.push((componente_curinga, ComponentTargetCriterion::Name, 0));
            s.push((componente_setado, ComponentTargetCriterion::Name, 0xC0FE));
            s.push((componente_lido, ComponentTargetCriterion::Name, 0xC0FE));
        }
        {
            let mut s = ENTITYCOMPONENT_STATES.lock().unwrap();
            s.retain(|e| ![sem_componente, com_componente].contains(&e.0));
            s.push((sem_componente, 0, 0));
            s.push((com_componente, componente.as_mut_ptr() as usize, 0));
        }
        KEYINPUT_STATES.lock().unwrap().retain(|e| e.0 != sem_tecla);
        let casa = |p: usize, f: FamiliaDoAlvo, t: TipoDoEvento, ev: usize| unsafe {
            alvo_casa(p, f, t, &[rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())])
        };
        let obtido = [
            casa(tecla_curinga, FamiliaDoAlvo::Tecla, TipoDoEvento::Tecla, sem_tecla),
            casa(tecla_setada, FamiliaDoAlvo::Tecla, TipoDoEvento::Tecla, sem_tecla),
            casa(componente_curinga, FamiliaDoAlvo::Componente, TipoDoEvento::Componente, sem_componente),
            casa(componente_setado, FamiliaDoAlvo::Componente, TipoDoEvento::Componente, sem_componente),
            casa(componente_lido, FamiliaDoAlvo::Componente, TipoDoEvento::Componente, com_componente),
        ];
        INPUTTARGET_STATES.lock().unwrap().retain(|e| ![tecla_curinga, tecla_setada].contains(&e.0));
        COMPONENTTARGET_STATES.lock().unwrap().retain(|e| ![componente_curinga, componente_setado, componente_lido].contains(&e.0));
        ENTITYCOMPONENT_STATES.lock().unwrap().retain(|e| ![sem_componente, com_componente].contains(&e.0));
        assert_eq!(obtido, [true, false, true, false, true]);
    }

    /// `ResourceTarget` com `Path` ou `Type` de valor 0 (`Path(r"")`, `Type(n"None")`) não filtra; com valor setado,
    /// o evento que não bate continua filtrado, inclusive o `Type` de nome que a fábrica não resolveu (`CLASSE_NAO_RESOLVIDA`).
    #[test]
    fn alvo_de_recurso_com_valor_zero_nao_filtra() {
        let (caminho_zero, tipo_zero, os_dois_zero) = (0x7000_0000_2510usize, 0x7000_0000_2520usize, 0x7000_0000_2530usize);
        let (caminho_setado, tipo_setado, tipo_nao_resolvido) = (0x7000_0000_2540usize, 0x7000_0000_2550usize, 0x7000_0000_2570usize);
        let alvos = [caminho_zero, tipo_zero, os_dois_zero, caminho_setado, tipo_setado, tipo_nao_resolvido];
        let evento = 0x7000_0000_2560usize;
        {
            let mut s = RESOURCETARGET_STATES.lock().unwrap();
            s.retain(|e| !alvos.contains(&e.0));
            s.push((caminho_zero, ResourceTargetCriterion::Path, 0));
            s.push((tipo_zero, ResourceTargetCriterion::Type, 0));
            s.push((os_dois_zero, ResourceTargetCriterion::Path, 0));
            s.push((os_dois_zero, ResourceTargetCriterion::Type, 0));
            s.push((caminho_setado, ResourceTargetCriterion::Path, 0x1234_5678_9abc_def1));
            s.push((tipo_setado, ResourceTargetCriterion::Type, 0x1234_5678_9abc_def2));
            s.push((tipo_nao_resolvido, ResourceTargetCriterion::Type, CLASSE_NAO_RESOLVIDA));
        }
        {
            let mut s = RESOURCEEVENT_STATES.lock().unwrap();
            s.retain(|e| e.0 != evento);
            s.push((evento, 0x0fed_cba9_8765_4321));
        }
        let casa = |p: usize| unsafe { alvo_casa(p, FamiliaDoAlvo::Recurso, TipoDoEvento::Recurso, &[rtti::Arg::Handle(evento as *mut c_void, std::ptr::null_mut())]) };
        let obtido = alvos.map(casa);
        RESOURCETARGET_STATES.lock().unwrap().retain(|e| !alvos.contains(&e.0));
        RESOURCEEVENT_STATES.lock().unwrap().retain(|e| e.0 != evento);
        assert_eq!(obtido, [true, true, true, false, false, false]);
    }

    /// Cada filtro do ramo de entidade lê o seu offset: `templatePath` em +0x60 e `appearanceName` em +0x50 (`DATABASE.md`).
    /// Os dois são hashes de 8 bytes; trocar os offsets compila e só aparece aqui.
    #[test]
    fn ramo_de_entidade_le_cada_campo_no_seu_offset() {
        let alvo = CamposDoAlvoDeEntidade { template_path: Some(0xA), appearance_name: Some(0xB), ..Default::default() };
        let mut entidade = [0u64; 0x68 / 8];
        entidade[0x60 / 8] = 0xA;
        entidade[0x50 / 8] = 0xB;
        assert!(unsafe { alvo_de_entidade_casa_entidade(&alvo, entidade.as_mut_ptr() as *mut c_void) });
        entidade[0x60 / 8] = 0xB;
        entidade[0x50 / 8] = 0xA;
        assert!(!unsafe { alvo_de_entidade_casa_entidade(&alvo, entidade.as_mut_ptr() as *mut c_void) });
    }

    /// Os adaptadores leem o valor 0 da tabela como não-setado:
    /// `EntityTarget.ID(0)`, `.Definition(0, 0)`, `inkWidgetTarget.Library(0, item)` e `.Controller(n"None")` não viram filtro de valor 0.
    #[test]
    fn adaptadores_leem_valor_zero_como_nao_setado() {
        let (id_zero, id_setado, definicao_zero) = (0x7000_0000_2B10usize, 0x7000_0000_2B20usize, 0x7000_0000_2B30usize);
        let (biblioteca_zero, controller_zero) = (0x7000_0000_2B40usize, 0x7000_0000_2B50usize);
        {
            let mut s = ENTITYTARGET_STATES.lock().unwrap();
            s.retain(|e| ![id_zero, id_setado].contains(&e.0));
            s.push((id_zero, EntityTargetCriterion::Id, 0));
            s.push((id_setado, EntityTargetCriterion::Id, 0xE1));
        }
        {
            let mut s = ENTITYTARGET_DEFINITION_STATES.lock().unwrap();
            s.retain(|e| e.0 != definicao_zero);
            s.push((definicao_zero, 0, 0));
        }
        {
            let mut s = INKWIDGETTARGET_STATES.lock().unwrap();
            s.retain(|e| ![biblioteca_zero, controller_zero].contains(&e.0));
            s.push((biblioteca_zero, InkWidgetTargetCriterion::Library, 0, 0x1111));
            s.push((controller_zero, InkWidgetTargetCriterion::Controller, 0, 0));
        }
        let campos = [id_zero, id_setado, definicao_zero].map(campos_do_alvo_de_entidade);
        let widgets = inkwidgettarget_alvos(&[biblioteca_zero, controller_zero]);
        ENTITYTARGET_STATES.lock().unwrap().retain(|e| ![id_zero, id_setado].contains(&e.0));
        ENTITYTARGET_DEFINITION_STATES.lock().unwrap().retain(|e| e.0 != definicao_zero);
        INKWIDGETTARGET_STATES.lock().unwrap().retain(|e| ![biblioteca_zero, controller_zero].contains(&e.0));
        assert_eq!(campos.map(|c| c.entity_id), [None, Some(0xE1), None]);
        assert_eq!((campos[2].appearance_path, campos[2].definition_name), (None, None));
        assert_eq!(widgets, vec![(None, Some(0x1111), None), (None, None, None)]);
        // `Definition(0, 0)` não é filtro de aparência: o ramo de entidade, que recusa `appearancePath` setado, casa.
        let mut entidade = [0u64; 0x68 / 8];
        assert!(unsafe { alvo_de_entidade_casa_entidade(&campos[2], entidade.as_mut_ptr() as *mut c_void) });
    }

    /// `EntityTarget.Type` guarda a classe que a fábrica resolveu: `n"None"` sai 0 da `classe_do_alvo` sem chamar o RTTI e é critério não-setado;
    /// o nome que não resolve (`CLASSE_NAO_RESOLVIDA`) é critério setado e bloqueia a entidade, em vez de virar curinga.
    #[test]
    fn tipo_de_entidade_nao_resolvido_bloqueia() {
        let (tipo_zero, tipo_nao_resolvido) = (0x7000_0000_2C10usize, 0x7000_0000_2C20usize);
        let valor_zero = unsafe { classe_do_alvo(0, "EntityTarget.Type") };
        {
            let mut s = ENTITYTARGET_STATES.lock().unwrap();
            s.retain(|e| ![tipo_zero, tipo_nao_resolvido].contains(&e.0));
            s.push((tipo_zero, EntityTargetCriterion::Type, valor_zero));
            s.push((tipo_nao_resolvido, EntityTargetCriterion::Type, CLASSE_NAO_RESOLVIDA));
        }
        let campos = [tipo_zero, tipo_nao_resolvido].map(campos_do_alvo_de_entidade);
        ENTITYTARGET_STATES.lock().unwrap().retain(|e| ![tipo_zero, tipo_nao_resolvido].contains(&e.0));
        assert_eq!(valor_zero, 0);
        assert_eq!(campos.map(|c| c.tipo), [None, Some(CLASSE_NAO_RESOLVIDA)]);
        let mut entidade = [0u64; 0x68 / 8];
        let p = entidade.as_mut_ptr() as *mut c_void;
        assert_eq!(campos.map(|c| unsafe { alvo_de_entidade_casa_entidade(&c, p) }), [true, false]);
    }

    /// `DynamicEntityTarget.Tags([])`/`StaticEntityTarget.Tags([])` casam todo evento de ciclo, com ou sem entidade,
    /// e o handler dispara se ALGUM alvo suportado casa: `Tags([])` + `Tag(x)` que não casa dispara; só `Tag(x)`
    /// filtra. As linhas saem da gravação da fábrica (`gravar_alvo_por_tags`) e o veredito sai do filtro do despacho (`alvos_barram_o_disparo`).
    #[test]
    fn alvo_por_tags_vazio_continua_no_despacho() {
        let (dinamico_vazio, dinamico_errado, dinamico_misto, dinamico_certo) =
            (0x7000_0000_2710usize, 0x7000_0000_2720usize, 0x7000_0000_2730usize, 0x7000_0000_2770usize);
        let (estatico_vazio, estatico_errado) = (0x7000_0000_2740usize, 0x7000_0000_2750usize);
        let dinamicos = [dinamico_vazio, dinamico_errado, dinamico_misto, dinamico_certo];
        let estaticos = [estatico_vazio, estatico_errado];
        let (com_entidade, sem_entidade) = (0x7000_0000_2760usize, 0x7000_0000_2780usize);
        let (tag_certa, tag_errada) = (0xB133_0000_0000_0001u64, 0xB133_0000_0000_0002u64);
        let id = 0xB133_0000_0000_00AAu64;
        let mut entidade = [0u64; 0x50 / 8];
        entidade[0x48 / 8] = id;
        DYNAMICENTITYTARGET_STATES.lock().unwrap().retain(|e| !dinamicos.contains(&e.0));
        STATICENTITYTARGET_STATES.lock().unwrap().retain(|e| !estaticos.contains(&e.0));
        gravar_alvo_por_tags(&DYNAMICENTITYTARGET_STATES, dinamico_vazio, Vec::new());
        gravar_alvo_por_tags(&DYNAMICENTITYTARGET_STATES, dinamico_errado, vec![tag_errada]);
        gravar_alvo_por_tags(&DYNAMICENTITYTARGET_STATES, dinamico_misto, vec![tag_errada, tag_certa]);
        gravar_alvo_por_tags(&DYNAMICENTITYTARGET_STATES, dinamico_certo, vec![tag_certa]);
        gravar_alvo_por_tags(&STATICENTITYTARGET_STATES, estatico_vazio, Vec::new());
        gravar_alvo_por_tags(&STATICENTITYTARGET_STATES, estatico_errado, vec![tag_errada]);
        {
            let mut s = ENTITYLIFECYCLE_STATES.lock().unwrap();
            s.retain(|e| e.0 != com_entidade && e.0 != sem_entidade);
            s.push((com_entidade, entidade.as_mut_ptr() as usize));
            s.push((sem_entidade, 0));
        }
        dynent_tags().lock().unwrap().insert(id, std::collections::HashSet::from([tag_certa]));
        statent_tags().lock().unwrap().insert(id, std::collections::HashSet::from([tag_certa]));
        let dispara = |alvos: &[usize], evento: usize| unsafe {
            let args = [rtti::Arg::Handle(evento as *mut c_void, std::ptr::null_mut())];
            !alvos_barram_o_disparo(alvos, tipo_do_evento(&args), &args)
        };
        let obtido = [
            dispara(&[dinamico_vazio], com_entidade),
            dispara(&[dinamico_vazio, dinamico_errado], com_entidade),
            dispara(&[dinamico_errado], com_entidade),
            dispara(&[dinamico_misto], com_entidade),
            dispara(&[estatico_vazio, estatico_errado], com_entidade),
            dispara(&[estatico_errado], com_entidade),
            dispara(&[dinamico_vazio, dinamico_certo], sem_entidade),
            dispara(&[dinamico_certo], sem_entidade),
        ];
        DYNAMICENTITYTARGET_STATES.lock().unwrap().retain(|e| !dinamicos.contains(&e.0));
        STATICENTITYTARGET_STATES.lock().unwrap().retain(|e| !estaticos.contains(&e.0));
        ENTITYLIFECYCLE_STATES.lock().unwrap().retain(|e| e.0 != com_entidade && e.0 != sem_entidade);
        dynent_tags().lock().unwrap().remove(&id);
        statent_tags().lock().unwrap().remove(&id);
        assert_eq!(obtido, [true, true, false, true, true, false, true, false]);
    }

    /// `alvo_casa` do `ComponentTarget` com campo setado devolve `false` para o componente que não resolve: componente nulo, ou evento fora de
    /// `ENTITYCOMPONENT_STATES` chamado com o tipo `Componente`. O mesmo alvo casa o componente lido, então o `false` vem da falta de dado, não do critério.
    /// No despacho só o componente nulo chega a esse `false` e barra o handler: o evento fora da tabela sai `Outro` de `tipo_do_evento`, a família
    /// `Componente` não o suporta e o handler dispara.
    #[test]
    fn alvo_de_componente_bloqueia_componente_nao_resolvido() {
        let alvo = 0x7000_0000_2810usize;
        let (sem_estado, componente_nulo, componente_lido) = (0x7000_0000_2830usize, 0x7000_0000_2840usize, 0x7000_0000_2850usize);
        let eventos = [sem_estado, componente_nulo, componente_lido];
        let mut componente = [0u64; 0x68 / 8];
        componente[0x40 / 8] = 0xC0FE;
        componente[0x60 / 8] = 0x1D;
        {
            let mut s = COMPONENTTARGET_STATES.lock().unwrap();
            s.retain(|e| e.0 != alvo);
            s.push((alvo, ComponentTargetCriterion::Id, 0x1D));
        }
        {
            let mut s = ENTITYCOMPONENT_STATES.lock().unwrap();
            s.retain(|e| !eventos.contains(&e.0));
            s.push((componente_nulo, 0, 0));
            s.push((componente_lido, componente.as_mut_ptr() as usize, 0));
        }
        let casa = |ev: usize| unsafe {
            alvo_casa(alvo, FamiliaDoAlvo::Componente, TipoDoEvento::Componente, &[rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())])
        };
        let dispara = |ev: usize| unsafe {
            let args = [rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())];
            !alvos_barram_o_disparo(&[alvo], tipo_do_evento(&args), &args)
        };
        let obtido = [casa(sem_estado), casa(componente_nulo), casa(componente_lido), dispara(sem_estado), dispara(componente_nulo)];
        COMPONENTTARGET_STATES.lock().unwrap().retain(|e| e.0 != alvo);
        ENTITYCOMPONENT_STATES.lock().unwrap().retain(|e| !eventos.contains(&e.0));
        assert_eq!(obtido, [false, false, true, true, false]);
    }

    /// `alvo_casa` do `ResourceTarget` com `Path` setado devolve `false` para o evento sem caminho (fora de `RESOURCEEVENT_STATES`, chamado com o
    /// tipo `Recurso`) e casa o evento de caminho igual. No despacho esse `false` não acontece: o evento fora da tabela sai `Outro` de
    /// `tipo_do_evento`, a família `Recurso` não o suporta e o handler dispara; o evento que sai `Recurso` está na tabela que `event_resource_path`
    /// lê, e as 3 origens do `Resource/Load` só o tiram de lá depois do `fire_event_args`. O `Type` setado contra evento sem recurso já está em
    /// `alvo_de_recurso_com_valor_zero_nao_filtra`.
    #[test]
    fn alvo_de_recurso_bloqueia_evento_sem_caminho() {
        let alvo = 0x7000_0000_2910usize;
        let (sem_caminho, com_caminho) = (0x7000_0000_2920usize, 0x7000_0000_2930usize);
        let eventos = [sem_caminho, com_caminho];
        let caminho = 0x1234_5678_9abc_def3u64;
        {
            let mut s = RESOURCETARGET_STATES.lock().unwrap();
            s.retain(|e| e.0 != alvo);
            s.push((alvo, ResourceTargetCriterion::Path, caminho));
        }
        {
            let mut s = RESOURCEEVENT_STATES.lock().unwrap();
            s.retain(|e| !eventos.contains(&e.0));
            s.push((com_caminho, caminho));
        }
        let casa = |ev: usize| unsafe { alvo_casa(alvo, FamiliaDoAlvo::Recurso, TipoDoEvento::Recurso, &[rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())]) };
        let dispara = |ev: usize| unsafe {
            let args = [rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())];
            !alvos_barram_o_disparo(&[alvo], tipo_do_evento(&args), &args)
        };
        let obtido = [casa(sem_caminho), casa(com_caminho), dispara(sem_caminho)];
        RESOURCETARGET_STATES.lock().unwrap().retain(|e| e.0 != alvo);
        RESOURCEEVENT_STATES.lock().unwrap().retain(|e| !eventos.contains(&e.0));
        assert_eq!(obtido, [false, true, true]);
    }

    /// O despacho vivo do `DynamicEntityTarget`/`StaticEntityTarget` com tag bloqueia a entidade fora da tabela de tags do sistema do alvo
    /// (`dynent_tags` ou `statent_tags`), inclusive a que tem a tag só na tabela do outro sistema; com a tag na tabela do próprio sistema, casa.
    /// O `Tag` dinâmico contra evento sem entidade já está em `alvo_por_tags_vazio_continua_no_despacho`.
    #[test]
    fn alvo_de_entidade_por_tag_bloqueia_entidade_fora_das_tags() {
        let (dinamico, estatico) = (0x7000_0000_2A10usize, 0x7000_0000_2A20usize);
        let (fora, so_dinamica, so_estatica) = (0x7000_0000_2A40usize, 0x7000_0000_2A50usize, 0x7000_0000_2A60usize);
        let eventos = [fora, so_dinamica, so_estatica];
        let tag = 0xF105_0000_0000_0001u64;
        let (id_fora, id_dinamica, id_estatica) = (0xF105_0000_0000_00A1u64, 0xF105_0000_0000_00A2u64, 0xF105_0000_0000_00A3u64);
        let (mut entidade_fora, mut entidade_dinamica, mut entidade_estatica) = ([0u64; 0x50 / 8], [0u64; 0x50 / 8], [0u64; 0x50 / 8]);
        entidade_fora[0x48 / 8] = id_fora;
        entidade_dinamica[0x48 / 8] = id_dinamica;
        entidade_estatica[0x48 / 8] = id_estatica;
        DYNAMICENTITYTARGET_STATES.lock().unwrap().retain(|e| e.0 != dinamico);
        STATICENTITYTARGET_STATES.lock().unwrap().retain(|e| e.0 != estatico);
        gravar_alvo_por_tags(&DYNAMICENTITYTARGET_STATES, dinamico, vec![tag]);
        gravar_alvo_por_tags(&STATICENTITYTARGET_STATES, estatico, vec![tag]);
        {
            let mut s = ENTITYLIFECYCLE_STATES.lock().unwrap();
            s.retain(|e| !eventos.contains(&e.0));
            s.push((fora, entidade_fora.as_mut_ptr() as usize));
            s.push((so_dinamica, entidade_dinamica.as_mut_ptr() as usize));
            s.push((so_estatica, entidade_estatica.as_mut_ptr() as usize));
        }
        dynent_tags().lock().unwrap().insert(id_dinamica, std::collections::HashSet::from([tag]));
        statent_tags().lock().unwrap().insert(id_estatica, std::collections::HashSet::from([tag]));
        let casa = |p: usize, f: FamiliaDoAlvo, ev: usize| unsafe {
            alvo_casa(p, f, TipoDoEvento::Ciclo, &[rtti::Arg::Handle(ev as *mut c_void, std::ptr::null_mut())])
        };
        let obtido = [
            casa(dinamico, FamiliaDoAlvo::EntidadeDinamica, fora),
            casa(dinamico, FamiliaDoAlvo::EntidadeDinamica, so_estatica),
            casa(dinamico, FamiliaDoAlvo::EntidadeDinamica, so_dinamica),
            casa(estatico, FamiliaDoAlvo::EntidadeEstatica, fora),
            casa(estatico, FamiliaDoAlvo::EntidadeEstatica, so_dinamica),
            casa(estatico, FamiliaDoAlvo::EntidadeEstatica, so_estatica),
        ];
        DYNAMICENTITYTARGET_STATES.lock().unwrap().retain(|e| e.0 != dinamico);
        STATICENTITYTARGET_STATES.lock().unwrap().retain(|e| e.0 != estatico);
        ENTITYLIFECYCLE_STATES.lock().unwrap().retain(|e| !eventos.contains(&e.0));
        dynent_tags().lock().unwrap().remove(&id_dinamica);
        statent_tags().lock().unwrap().remove(&id_estatica);
        assert_eq!(obtido, [false, false, true, false, false, true]);
    }

    /// O filtro em passada única do `alvos_barram_o_disparo`: alvo sem família ou de família que não suporta o evento não barra nem
    /// conta como suportado, em qualquer posição; com algum suportado, o handler dispara se o que casa vier antes, no meio ou no fim.
    #[test]
    fn filtro_em_passada_unica_ignora_alvo_sem_suporte_em_qualquer_posicao() {
        let (casa, nao_casa, recurso, sem_familia) = (0x7000_0000_2D10usize, 0x7000_0000_2D20usize, 0x7000_0000_2D30usize, 0x7000_0000_2D40usize);
        let aperto = 0x7000_0000_2D50usize;
        {
            let mut s = INPUTTARGET_STATES.lock().unwrap();
            s.retain(|e| e.0 != casa && e.0 != nao_casa);
            s.push((casa, InputTargetCriterion::Key, 49, IACT_PRESS as i64));
            s.push((nao_casa, InputTargetCriterion::Key, 50, IACT_PRESS as i64));
        }
        {
            let mut s = RESOURCETARGET_STATES.lock().unwrap();
            s.retain(|e| e.0 != recurso);
            s.push((recurso, ResourceTargetCriterion::Path, 0x2D2D));
        }
        {
            let mut s = KEYINPUT_STATES.lock().unwrap();
            s.retain(|e| e.0 != aperto);
            s.push((aperto, IACT_PRESS, 49, false, false, false));
        }
        let args = [rtti::Arg::Handle(aperto as *mut c_void, std::ptr::null_mut())];
        let tipo = unsafe { tipo_do_evento(&args) };
        let dispara = |alvos: &[usize]| unsafe { !alvos_barram_o_disparo(alvos, tipo, &args) };
        let obtido = [
            dispara(&[]),
            dispara(&[recurso]),
            dispara(&[sem_familia]),
            dispara(&[recurso, nao_casa]),
            dispara(&[nao_casa, recurso]),
            dispara(&[sem_familia, nao_casa, sem_familia]),
            dispara(&[casa, nao_casa]),
            dispara(&[nao_casa, casa, recurso]),
            dispara(&[recurso, nao_casa, casa]),
        ];
        let suportados: Vec<usize> = alvos_que_suportam(&[recurso, nao_casa, sem_familia, casa], tipo).map(|s| s.0).collect();
        INPUTTARGET_STATES.lock().unwrap().retain(|e| e.0 != casa && e.0 != nao_casa);
        RESOURCETARGET_STATES.lock().unwrap().retain(|e| e.0 != recurso);
        KEYINPUT_STATES.lock().unwrap().retain(|e| e.0 != aperto);
        assert_eq!(tipo, TipoDoEvento::Tecla);
        assert_eq!(obtido, [true, true, true, false, false, false, true, true, true]);
        assert_eq!(suportados, vec![nao_casa, casa]);
    }
}

#[cfg(test)]
mod tabela_de_customizations {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// Carga da tabela sem fonte (vazia, ou ausente, que a recarga passa vazia) desliga a mescla da tabela anterior; fonte sem os campos do RTTI também.
    #[test]
    fn tabela_sem_fonte_desliga_a_mescla_anterior() {
        let ligado = AtomicBool::new(true);
        assert!(!cust_ligar_mescla(&ligado, 0, || unreachable!("sem fonte não resolve os campos")));
        assert!(!ligado.load(Ordering::Relaxed), "tabela sem fonte deixou a mescla da anterior ligada");
        ligado.store(true, Ordering::Relaxed);
        assert!(!cust_ligar_mescla(&ligado, 2, || false));
        assert!(!ligado.load(Ordering::Relaxed), "fonte sem campos deixou a mescla ligada");
        assert!(cust_ligar_mescla(&ligado, 1, || true));
        assert!(ligado.load(Ordering::Relaxed));
    }

    /// A recarga sem o arquivo só tem o que desfazer com fonte pedida por tabela (`Token`); a de teste (`Recurso`) não conta.
    #[test]
    fn so_fonte_de_tabela_pede_a_volta_sem_o_arquivo() {
        assert!(!cust_tem_fonte_de_tabela(&[vec![], vec![]]));
        assert!(!cust_tem_fonte_de_tabela(&[vec![CustFonte::Recurso(0x10)], vec![]]));
        assert!(cust_tem_fonte_de_tabela(&[vec![CustFonte::Recurso(0x10)], vec![CustFonte::Token([0x20, 0x28])]]));
        assert!(cust_tem_fonte_de_tabela(&[vec![CustFonte::Token([0x20, 0x28])], vec![]]));
    }
}

#[cfg(test)]
mod a47_tabela_vazia {
    use super::*;

    unsafe extern "C" fn original_falsa(_estado: u64, _grupo: u64, _fpp: u64, _saida: *mut u64) -> u64 {
        0x47
    }

    /// Tabela do `#11` vazia: o gancho devolve o retorno da original e conta a chamada sem `is_readable` nem leitura da saída.
    /// Com um par registrado, lê a saída (1 `is_readable`) e troca o descritor.
    #[test]
    fn tabela_vazia_repassa_sem_ir_ao_kernel() {
        const K: usize = 5;
        let (recurso, aparencia, substituto) = (0x47a0_0000_0001u64, 0x47a0_0000_0002u64, 0x47a0_0000_0003u64);
        let leituras = || crate::memoria::IS_READABLE_CHAMADAS.with(|c| c.get());
        let contadores = || {
            [&A47_GET_CHAMADAS, &A47_GET_DESCRITORES, &A47_GET_TROCADOS].map(|c| c[K].load(Ordering::Relaxed))
        };
        ORIG_A47_GET[K].store(original_falsa as *const () as u64, Ordering::Relaxed);
        let mut desc = [recurso, aparencia];
        // DynArray: dados em +0, capacidade (u32) em +8, tamanho (u32) em +0xc.
        let mut saida = [desc.as_mut_ptr() as u64, 1 | (1 << 32)];
        let base = contadores();
        {
            // A leitura segura a tabela vazia contra o teste do `#11` que grava nela em paralelo: o `APP_OVERRIDES_N` só muda com a escrita.
            let vazia = (0..100_000)
                .find_map(|_| {
                    let t = crate::aparencia::app_overrides().read().unwrap_or_else(|e| e.into_inner());
                    if t.is_empty() {
                        Some(t)
                    } else {
                        drop(t);
                        std::thread::yield_now();
                        None
                    }
                })
                .expect("a tabela do #11 nunca ficou vazia");
            let antes = leituras();
            assert_eq!(unsafe { a47_get(K, 0, 0, 0, saida.as_mut_ptr()) }, 0x47);
            assert_eq!(leituras() - antes, 0, "tabela vazia foi ao kernel");
            drop(vazia);
        }
        let depois_vazia = contadores();
        assert_eq!(desc, [recurso, aparencia]);

        crate::aparencia::registrar_app_override(recurso, substituto, aparencia);
        saida[0] = desc.as_mut_ptr() as u64;
        let antes = leituras();
        let r = unsafe { a47_get(K, 0, 0, 0, saida.as_mut_ptr()) };
        let lidas = leituras() - antes;
        crate::aparencia::remover_app_override(recurso, aparencia);
        ORIG_A47_GET[K].store(0, Ordering::Relaxed);
        let fim = contadores();
        assert_eq!(r, 0x47);
        assert_eq!(lidas, 1, "com par registrado a saída é provada uma vez");
        assert_eq!(desc, [substituto, aparencia]);
        // [chamadas, descritores, trocados]: a chamada conta sempre (o "hook vivo" do veredito 47A); descritores e trocados só com par.
        assert_eq!([depois_vazia[0] - base[0], depois_vazia[1] - base[1], depois_vazia[2] - base[2]], [1, 0, 0]);
        assert_eq!([fim[0] - base[0], fim[1] - base[1], fim[2] - base[2]], [2, 1, 1]);
    }
}

#[cfg(test)]
mod mescla_de_customizacao {
    use super::*;

    /// `CUST_NOMES` e `CUST_CAMPOS` são do processo: os testes deste módulo que os tocam rodam um de cada vez.
    static SERIE: Mutex<()> = Mutex::new(());

    fn zerar_nomes() {
        *CUST_NOMES.write().unwrap() = None;
        CUST_NOMES_N.store(0, Ordering::Relaxed);
    }

    /// Vetores `[B1]`: o app registra o `name`; morph e switcher, o texto com o hash continuado do nome da opção.
    #[test]
    fn nome_da_escolha_continua_o_hash_do_nome_da_opcao() {
        let cabelo = cname("hair_style");
        assert_eq!(cabelo, 0x8d48_5e57_210a_cd37);
        assert_eq!(cust_nome_da_escolha(cabelo, &CustChave::Nome(7)), 7);
        assert_eq!(cust_nome_da_escolha(cabelo, &CustChave::Nome(0)), 0);
        assert_eq!(cust_nome_da_escolha(cabelo, &CustChave::Nome(15555357983233036507)), 0xd7df_bbab_80a4_50db);
        let texto = |dono: &str, t: &str| cust_nome_da_escolha(cname(dono), &CustChave::Texto(t.into()));
        assert_eq!(texto("hair_style", "Mod A"), 0xe5ae_3998_fdb9_556a);
        assert_eq!(texto("outra", "Mod A"), 0x1947_fc01_f92b_77b9);
        assert_eq!(texto("hair_style", ""), cabelo);
        assert_eq!(texto("eyes", "Olhos Azuis"), 0x1624_847d_d00d_b6d0);
        assert_eq!(texto("makeup_lips", "Batom 01"), 0x1498_852d_fb7a_5377);
        // Equivale ao hash do texto do nome da opção seguido do texto da escolha.
        assert_eq!(texto("eyes", "Olhos Azuis"), bwms_primitives::fnv1a64(b"eyesOlhos Azuis"));
    }

    /// Vetores `[B2]`: `*` no fim do texto da fonte casa por prefixo e lê o texto do alvo; sem curinga compara hash sem ler texto.
    #[test]
    fn curinga_casa_por_prefixo_e_sem_curinga_por_hash() {
        assert_eq!(cust_curinga("hair_*"), Some("hair_"));
        assert_eq!(cust_curinga("hair_color"), None);
        assert_eq!(cust_curinga("*"), Some(""));
        assert_eq!(cust_curinga(""), None);
        assert_eq!(cust_curinga("a*b"), None);
        assert_eq!(cust_curinga("a**"), Some("a*"));
        let casa = |alvo: u64, texto: &str, fonte: u64, prefixo: Option<&str>| {
            let mut leu = false;
            let r = cust_casa(alvo, || {
                leu = true;
                texto.to_string()
            }, fonte, prefixo);
            (r, leu)
        };
        assert_eq!(casa(1, "hair_color", 2, Some("hair_")), (true, true));
        assert_eq!(casa(1, "eyes", 2, Some("hair_")), (false, true));
        assert_eq!(casa(5, "x", 5, None), (true, false));
        assert_eq!(casa(5, "x", 6, None), (false, false));
        assert_eq!(casa(1, "hair_", 2, Some("hair_")), (true, true));
        assert_eq!(casa(1, "anything", 2, Some("")), (true, true));
    }

    /// Vetores `[B3]`: busca no array ordenado do estado do save.
    #[test]
    fn posicao_ordenada_do_estado() {
        let v = [3u64, 9, 20];
        assert_eq!(cust_posicao_ordenada(&v, 9), Ok(1));
        assert_eq!(cust_posicao_ordenada(&v, 1), Err(0));
        assert_eq!(cust_posicao_ordenada(&v, 10), Err(2));
        assert_eq!(cust_posicao_ordenada(&v, 99), Err(3));
        assert_eq!(cust_posicao_ordenada(&v, 3), Ok(0));
        assert_eq!(cust_posicao_ordenada(&v, 20), Ok(2));
        assert_eq!(cust_posicao_ordenada(&[], 99), Err(0));
    }

    /// Vetores `[B4]`: o conjunto de nomes de mod não conta repetido, não tem o vazio e some ao zerar.
    #[test]
    fn nomes_de_mod_contam_sem_repetir() {
        let _serie = SERIE.lock().unwrap_or_else(|e| e.into_inner());
        zerar_nomes();
        assert!(!cust_nome_de_mod(0x10));
        for n in [0x10, 0x20, 0x10] {
            cust_registrar_nome(n);
        }
        let obtido = (CUST_NOMES_N.load(Ordering::Relaxed), [0x10, 0x20, 0x30, 0].map(cust_nome_de_mod));
        zerar_nomes();
        assert_eq!(obtido, (2, [true, true, false, false]));
        assert!(!cust_nome_de_mod(0x10));
    }

    /// Memória que o código da mescla escreve por ponteiro cru: no heap, vazada e mutável. Na pilha, o endereço da thread de teste é reaproveitado
    /// pela próxima, e o array imutável escrito por ponteiro tirado de `&` é comportamento indefinido.
    fn vazado<T>(v: T) -> &'static mut T {
        Box::leak(Box::new(v))
    }

    /// `DynArray` de teste: `[entries, capacidade | tamanho << 32]`.
    fn arr(buf: &mut [u64], tamanho: usize) -> [u64; 2] {
        [buf.as_mut_ptr() as u64, buf.len() as u64 | (tamanho as u64) << 32]
    }

    fn lidos(a: &[u64; 2]) -> Vec<u64> {
        let (e, n, _) = unsafe { cust_arr(a.as_ptr() as usize) };
        (0..n).map(|k| unsafe { cu64(e + k * 8) }).collect()
    }

    /// Vetores `[B5]`: com vaga, nada vai ao motor e o array não muda.
    #[test]
    fn reservar_com_vaga_nao_vai_ao_motor() {
        let mut buf = [0u64; 4];
        let a = arr(&mut buf, 2);
        assert!(unsafe { cust_reservar_mais_um(a.as_ptr() as usize, 8) });
        assert_eq!(unsafe { cust_arr(a.as_ptr() as usize) }.1, 2);
    }

    /// `cust_truncar` de CName só devolve o tamanho: o elemento não tem destrutor.
    #[test]
    fn truncar_cname_volta_o_tamanho() {
        let mut buf = [1u64, 2, 3, 4];
        let mut a = arr(&mut buf, 4);
        unsafe { cust_truncar(a.as_mut_ptr() as usize, 1, CustElemento::CName) };
        assert_eq!(lidos(&a), vec![1]);
        assert_eq!(buf[3], 4, "a memória além do tamanho fica como estava");
    }

    /// Vetores `[B6]`: as entradas de cada grupo da fonte vão ao grupo homônimo do alvo; grupo sem homônimo não cria grupo.
    #[test]
    fn grupos_da_fonte_vao_ao_homonimo_do_alvo() {
        let _serie = SERIE.lock().unwrap_or_else(|e| e.into_inner());
        zerar_nomes();
        let (hairs, eyes, beard) = (0x4A11, 0xE7E5, 0xBEA4);
        let (alvo_hairs, alvo_eyes) = (vazado([0xA1u64, 0xA2, 0, 0]), vazado([0xB1u64, 0]));
        let (fonte_hairs, fonte_beard) = (vazado([0xF1u64, 0xF2]), vazado([0xF9u64]));
        // Grupo (0x18 B): `name` em +0, `options` (`DynArray<CName>`) em +8.
        let grupo = |nome: u64, a: [u64; 2]| [nome, a[0], a[1]];
        let ga = vazado([grupo(hairs, arr(alvo_hairs, 2)), grupo(eyes, arr(alvo_eyes, 1))]);
        let gf = vazado([grupo(hairs, arr(fonte_hairs, 2)), grupo(beard, arr(fonte_beard, 1))]);
        let alvo = vazado([ga.as_mut_ptr() as u64, 2 | 2 << 32]);
        let fonte = vazado([gf.as_mut_ptr() as u64, 2 | 2 << 32]);
        let mut m = CustMescla::default();
        unsafe { cust_mesclar_grupos(&mut m, alvo.as_mut_ptr() as usize, fonte.as_mut_ptr() as usize) };
        let opcoes = |g: &[u64; 3]| lidos(&[g[1], g[2]]);
        let obtido = (opcoes(&ga[0]), opcoes(&ga[1]), unsafe { cust_arr(alvo.as_ptr() as usize) }.1);
        let registrados = [0xF1, 0xF2, 0xF9].map(cust_nome_de_mod);
        zerar_nomes();
        assert_eq!(obtido, (vec![0xA1, 0xA2, 0xF1, 0xF2], vec![0xB1], 2));
        assert_eq!(m.contagem.grupos, 2);
        assert_eq!(m.tamanhos.iter().map(|t| t.1).collect::<Vec<_>>(), vec![2]);
        assert_eq!(registrados, [true, true, false]);
    }

    /// Campos de teste para `cust_opcao_do_estado`: no `info`, `name` em +0, `hidden` em +8, `enabled` em +0x10; no estado da opção, `info` em +0
    /// e `isActive` em +8.
    fn campos_de_teste() -> CustCampos {
        let lista = CustLista { tipo: 0, passo: 0, campo: 0, tipo_array: 0, indice: 0, chave: 0 };
        CustCampos {
            classes: [0; 3],
            nome: 0,
            slot: 0x18,
            link: 0x20,
            oculto: 0x08,
            ligado: Some(0x10),
            recurso: 0x28,
            listas: [lista; 3],
            tags: 0,
            icone: 0,
            opcao_info: 0,
            opcao_ativo: 0x08,
            tipo_string: 0,
            tipo_cnames: 0,
            tipo_arr_opcoes: 0,
            tipo_arr_grupos: 0,
            tipo_grupo: 0,
            classe_recurso: 0,
        }
    }

    /// Vetores `[B7]`: a opção visível (não oculta, ligada) de mod entra no estado na posição ordenada e fica ativa; o que já está no estado,
    /// a oculta, a desligada e a do jogo não mudam nada.
    #[test]
    fn opcao_de_mod_visivel_entra_no_estado_e_fica_ativa() {
        let _serie = SERIE.lock().unwrap_or_else(|e| e.into_inner());
        zerar_nomes();
        *CUST_CAMPOS.lock().unwrap() = Some(campos_de_teste());
        for n in [0x50, 0x05, 0xA0, 0x90] {
            cust_registrar_nome(n);
        }
        let caso = |nome: u64, oculto: u64, ligado: u64, estado_antes: &[u64]| {
            let info = vazado([nome, oculto, ligado, 0, 0, 0]);
            let opcao = vazado([info.as_ptr() as u64, 0]);
            let handle = vazado([opcao.as_mut_ptr() as u64, 0]);
            let buf = vazado([0u64; 4]);
            buf[..estado_antes.len()].copy_from_slice(estado_antes);
            let estado = vazado(arr(buf, estado_antes.len()));
            let marcou = unsafe { cust_marcar_estado(handle.as_ptr() as u64, estado.as_mut_ptr() as u64) };
            let estado_depois = lidos(estado);
            let ativou = unsafe { cust_marcar_ativo(handle.as_ptr() as u64) };
            let ativo = unsafe { cu8(opcao.as_ptr() as usize + 8) };
            (marcou, estado_depois, ativou, ativo)
        };
        let obtido = [
            caso(0x50, 0, 1, &[0x10, 0x90]),
            caso(0x05, 0, 1, &[0x10, 0x90]),
            caso(0xA0, 0, 1, &[0x10, 0x90]),
            caso(0x90, 0, 1, &[0x10, 0x90]),
            caso(0x50, 1, 1, &[0x10, 0x90]),
            caso(0x50, 0, 0, &[0x10, 0x90]),
            caso(0x77, 0, 1, &[0x10, 0x90]),
            caso(0x50, 0, 1, &[]),
        ];
        *CUST_CAMPOS.lock().unwrap() = None;
        zerar_nomes();
        assert_eq!(
            obtido,
            [
                (true, vec![0x10, 0x50, 0x90], true, 1),
                (true, vec![0x05, 0x10, 0x90], true, 1),
                (true, vec![0x10, 0x90, 0xA0], true, 1),
                (false, vec![0x10, 0x90], true, 1),
                (false, vec![0x10, 0x90], false, 0),
                (false, vec![0x10, 0x90], false, 0),
                (false, vec![0x10, 0x90], false, 0),
                (true, vec![0x50], true, 1),
            ]
        );
    }

    /// Sem nome de mod registrado (a mescla não rodou), o gancho do `Initialize*Option` não lê a opção.
    #[test]
    fn sem_mescla_a_opcao_nao_e_lida() {
        let _serie = SERIE.lock().unwrap_or_else(|e| e.into_inner());
        zerar_nomes();
        let mut buf = [0u64; 2];
        let estado = arr(&mut buf, 0);
        // A opção aponta para memória fora do mapa: ler derrubaria o teste.
        assert!(!unsafe { cust_marcar_estado(0x1000, estado.as_ptr() as u64) });
        assert!(!unsafe { cust_marcar_ativo(0x1000) });
    }

    /// `resource.fix` com `paths`: tabela por recurso; o mesmo antigo repetido fica com o último novo na posição do 1º; novo vazio não entra
    /// nem apaga.
    #[test]
    fn caminhos_do_resfixes_por_recurso() {
        let h = bwms_primitives::resource_path_hash;
        let texto = "base\\a.inkcharcustomization | paths | x\\velho.app | x\\novo.app\n\
                     base\\a.inkcharcustomization | paths | y\\velho.app | y\\novo.app\n\
                     base\\b.inkcharcustomization | paths | x\\velho.app | x\\outro.app\n\
                     base\\a.inkcharcustomization | paths | x\\velho.app | x\\ultimo.app\n\
                     base\\a.inkcharcustomization | paths | y\\velho.app | \n\
                     base\\a.inkcharcustomization | names | n_velho | n_novo\n";
        let (nomes, caminhos) = crate::mods_do_usuario::pipeline::resfixes_linhas(texto);
        assert_eq!(caminhos.len(), 2);
        assert_eq!(
            caminhos[&h("base\\a.inkcharcustomization")],
            vec![(h("x\\velho.app"), h("x\\ultimo.app")), (h("y\\velho.app"), h("y\\novo.app"))]
        );
        assert_eq!(caminhos[&h("base\\b.inkcharcustomization")], vec![(h("x\\velho.app"), h("x\\outro.app"))]);
        assert_eq!(nomes.len(), 1, "names vai à outra tabela");
    }
}

#[cfg(test)]
mod alvos_de_entidade_vetores {
    use super::*;

    /// Vetores `[B11]`: entidade com `entityID` em +0x48, `appearanceName` em +0x50 e `templatePath` em +0x60.
    #[test]
    fn entidade_le_os_3_campos_e_recusa_filtro_de_aparencia() {
        let mut entidade = [0u64; 0x68 / 8];
        entidade[0x48 / 8] = 0x1234;
        entidade[0x50 / 8] = 0xAAAA;
        entidade[0x60 / 8] = 0xBBBB;
        let p = entidade.as_mut_ptr() as *mut c_void;
        let f = CamposDoAlvoDeEntidade::default();
        let casos = [
            (f, true),
            (CamposDoAlvoDeEntidade { entity_id: Some(0x1234), ..f }, true),
            (CamposDoAlvoDeEntidade { template_path: Some(0xBBBB), ..f }, true),
            (CamposDoAlvoDeEntidade { appearance_name: Some(0xAAAA), ..f }, true),
            (CamposDoAlvoDeEntidade { template_path: Some(0xAAAA), ..f }, false),
            (CamposDoAlvoDeEntidade { definition_name: Some(1), ..f }, false),
        ];
        for (filtro, esperado) in casos {
            assert_eq!(unsafe { alvo_de_entidade_casa_entidade(&filtro, p) }, esperado, "{filtro:?}");
        }
    }

    /// Objeto de teste com `valor` em `off`.
    fn objeto(off: usize, valor: u64) -> Box<[u64; 0x20]> {
        let mut o = Box::new([0u64; 0x20]);
        o[off / 8] = valor;
        o
    }

    /// Vetores `[B12]`: builder com template 0x7E7, request (id 0x77, aparência 0xA9, record 0x5E), lista [(0x111, 0xD1), (0x222, 0xD2)] e
    /// aparência única (0x222, 0xD2) opcional. Sem request, os critérios do request não filtram; com única, a lista não conta.
    #[test]
    fn builder_vetores_b12() {
        let (d1, d2) = (objeto(0x30, 0xD1), objeto(0x30, 0xD2));
        let (r1, r2) = (objeto(0x30, 0x111), objeto(0x30, 0x222));
        let template = objeto(0x30, 0x7E7);
        // Request: `entityID` em +0x20, aparência em +0x28, record em +0xC0.
        let mut request = objeto(0x20, 0x77);
        request[0x28 / 8] = 0xA9;
        request[0xC0 / 8] = 0x5E;
        let p = |o: &[u64; 0x20]| o.as_ptr() as u64;
        // Lista de `EntityBuilderAppearance` (0x30 B): definição em +0, recurso em +0x10.
        let lista = [p(&d1), 0, p(&r1), 0, 0, 0, p(&d2), 0, p(&r2), 0, 0, 0];
        let builder = |com_request: bool, com_unica: bool, com_lista: bool| {
            let mut b = Box::new([0u64; 0x20]);
            if com_unica {
                b[0x20 / 8] = p(&d2);
                b[0x30 / 8] = p(&r2);
            }
            if com_lista {
                b[0x80 / 8] = lista.as_ptr() as u64;
                b[0x88 / 8] = 2 | 2 << 32;
            }
            b[0x90 / 8] = p(&template);
            if com_request {
                b[0xA0 / 8] = p(&request);
            }
            b
        };
        let f = CamposDoAlvoDeEntidade::default();
        let filtros = [
            f,
            CamposDoAlvoDeEntidade { entity_id: Some(0x77), ..f },
            CamposDoAlvoDeEntidade { entity_id: Some(0x78), ..f },
            CamposDoAlvoDeEntidade { template_path: Some(0x7E7), ..f },
            CamposDoAlvoDeEntidade { template_path: Some(0x7E8), ..f },
            CamposDoAlvoDeEntidade { appearance_name: Some(0xA9), ..f },
            CamposDoAlvoDeEntidade { appearance_name: Some(0xAA), ..f },
            CamposDoAlvoDeEntidade { record_id: Some(0x5E), ..f },
            CamposDoAlvoDeEntidade { record_id: Some(0x5F), ..f },
            CamposDoAlvoDeEntidade { appearance_path: Some(0x111), ..f },
            CamposDoAlvoDeEntidade { appearance_path: Some(0x222), ..f },
            CamposDoAlvoDeEntidade { appearance_path: Some(0x222), definition_name: Some(0xD2), ..f },
            CamposDoAlvoDeEntidade { appearance_path: Some(0x222), definition_name: Some(0xD1), ..f },
            CamposDoAlvoDeEntidade { definition_name: Some(0xD9), ..f },
        ];
        let rodar = |b: &[u64; 0x20]| filtros.map(|c| unsafe { alvo_de_entidade_casa_builder(&c, b.as_ptr() as u64) });
        const T: bool = true;
        const F: bool = false;
        assert_eq!(rodar(&builder(true, false, true)), [T, T, F, T, F, T, F, T, F, T, T, T, F, T], "com request, sem única, com lista");
        assert_eq!(rodar(&builder(true, true, true)), [T, T, F, T, F, T, F, T, F, F, T, T, F, T], "com request, com única, com lista");
        assert_eq!(rodar(&builder(false, false, false)), [T, T, T, T, F, T, T, T, T, F, F, F, F, T], "sem request, sem única, sem lista");
    }
}
