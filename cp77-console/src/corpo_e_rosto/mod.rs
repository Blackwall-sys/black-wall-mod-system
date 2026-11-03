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





/// Codeware `#133` (`DynamicEntityTarget::Matches`, 2026-08-12) — lê o `EntityID` (hash `u64`) do
/// campo REAL `Entity::entityID`, offset confirmado no header vendorizado (`enablers/Codeware/src/
/// Red/Entity.hpp`: `using EntityID = Core::OffsetPtr<0x48, Red::EntityID>;`). É campo de DADO, não
/// slot de vtable — não sofre o shift Itanium Mac=Windows+0x08 (regra já estabelecida neste projeto
/// pra offsets de campo, ex. `Vehicle::RecordID@+0x638` acima; o do puppet é +0x25C no Mac, não o +0x260 do header). `Red::
/// EntityID` é `{hash: u64}` (RED4ext.SDK `entEntityID.hpp`, já confirmado/usado pelo
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
/// filtros). `IK_None`/`IACT_None`, o não-setado da referência, saem como `None`. Um alvo
/// `.Axis(axis, threshold)` entra com ação `None`: o 4º campo dele é
/// `threshold` em bits, NÃO um `action` — ler aquilo como enum compararia lixo (ver
/// `InputTargetCriterion`). Isso reproduz o C++, onde `Axis()` deixa `action` no default.
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

/// Caminho do evento contra o do alvo. Com texto registrado com `*` ou `regex:`, o padrão sobre o texto do caminho do evento: sem texto não casa, como
/// na referência (`ResourceTarget.hpp:33`), e padrão recusado também não. Sem padrão, hash igual.
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

/// Alvos `inkWidgetTarget` entre os alvos do handler, como `(libraryPath, itemName, controllerType)`; valor 0 é o não-setado da referência
/// (`if (libraryPath && ...)`, `InkWidgetTarget.hpp:16-24`) e sai `None`.
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

/// `Supports` de cada família na referência.
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
    // Valor 0 é o não-setado da referência (`if (entityID && ...)`, `if (recordID)`, `if (appearancePath || definitionName)`; `EntityTarget.hpp:25-109`).
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

/// Valor que as fábricas `EntityTarget.Type` e `ResourceTarget.Type` gravam: a classe de `nome` resolvida uma vez, como o `GetClass` da fábrica
/// da referência (`EntityTarget.hpp:175`, `ResourceTarget.hpp:92`), e o despacho compara ponteiro por evento. `n"None"` (hash 0) fica 0, o
/// não-setado. Nome que não resolve vira `CLASSE_NAO_RESOLVIDA` e bloqueia: a referência guarda nulo e desliga o filtro, e o BWMS não copia isso
/// (F1-02, selftests c6 e 134). O veredito sai 1× por alvo, porque a fábrica roda uma vez por alvo.
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

/// `recordID` de um device como a referência: propriedade `controller` do device, o estado persistente dele (`GetPS`) e a propriedade
/// `tweakDBRecord` do estado.
pub(crate) unsafe fn record_do_device(device: *mut c_void) -> Option<u64> {
    let valor = |obj: *mut c_void, campo: &str| -> Option<u64> {
        let (off, _, no_holder) = rtti::resolve_prop_in_class(rtti::class_of(obj), campo)?;
        let base = if no_holder { read_u64_field(obj, 0x38)? as *mut c_void } else { obj };
        if base.is_null() {
            return None;
        }
        read_u64_field(base, off as usize)
    };
    let controller = valor(device, "controller")? as *mut c_void;
    if !rtti::sane(controller) {
        return None;
    }
    let rf = rtti::resolve_in_class(rtti::class_of(controller), "GetPS")?;
    let r = rtti::call_func(&rf, controller, &[])?;
    let mut ps = [u64::from_le_bytes(r[0..8].try_into().ok()?), u64::from_le_bytes(r[8..16].try_into().ok()?)];
    let record = if rtti::sane(ps[0] as *mut c_void) { valor(ps[0] as *mut c_void, "tweakDBRecord") } else { None };
    crate::jogo::objetos::soltar_handle(&mut ps);
    record
}

/// `EntityTarget::Matches` no ramo dos eventos de entidade.
pub(crate) unsafe fn alvo_de_entidade_casa_entidade(c: &CamposDoAlvoDeEntidade, entidade: *mut c_void) -> bool {
    let campo = |off: usize| read_u64_field(entidade, off).unwrap_or(0);
    // Offsets do `Entity` (`Entity.hpp:50-52`).
    let evento = CamposDoEventoDeEntidade { entity_id: campo(0x48), template_path: campo(0x60), appearance_name: campo(0x50) };
    if !bwms_primitives::targets::entity_evento_bate(c, &evento) {
        return false;
    }
    if c.tipo.is_some_and(|classe| !class_is_kind_of(rtti::class_of(entidade), classe as *mut c_void)) {
        return false;
    }
    if c.record_id.is_none() {
        return true;
    }
    let (categoria, record) = categoria_e_record(entidade);
    bwms_primitives::targets::entity_record_bate(c.record_id, categoria, record)
}

/// `EntityTarget::Matches` no ramo do `EntityBuilderEvent`, com o builder travado pelo despacho.
pub(crate) unsafe fn alvo_de_entidade_casa_builder(c: &CamposDoAlvoDeEntidade, builder: u64) -> bool {
    use bwms_primitives::targets::AparenciaCandidata;
    let req = eb_u64(builder, EB_REQUEST);
    if c.entity_id.is_some_and(|id| req != 0 && id != eb_u64(req, EBR_ENTITY_ID)) {
        return false;
    }
    if let Some(classe) = c.tipo {
        let (raiz, _) = eb_raiz_e_componentes(builder);
        if eb_u64(builder, EB_EXTRATOR) == 0 || raiz == 0 || !class_is_kind_of(rtti::class_of(raiz as *mut c_void), classe as *mut c_void) {
            return false;
        }
    }
    // A regra do `campo_bate` (`bwms_primitives::targets`), escrita aqui para ler o template só com o critério setado: cada `eb_u64` é uma syscall.
    if c.template_path.is_some_and(|caminho| caminho != eb_u64(eb_u64(builder, EB_TEMPLATE), RES_CAMINHO)) {
        return false;
    }
    if c.appearance_name.is_some_and(|nome| req != 0 && nome != eb_u64(req, EBR_APARENCIA)) {
        return false;
    }
    if c.record_id.is_some_and(|record| req != 0 && record != eb_u64(req, EBR_RECORD)) {
        return false;
    }
    let candidata = |a: u64| AparenciaCandidata { resource_path: eb_u64(eb_u64(a, EBA_RECURSO), RES_CAMINHO), definition_name: eb_u64(eb_u64(a, EBA_DEFINICAO), DEF_NOME) };
    let aparencia = builder + EB_APARENCIA;
    let unica = (eb_u64(aparencia, EBA_RECURSO) != 0).then(|| candidata(aparencia));
    let entradas = eb_u64(builder + EB_APARENCIAS, 0);
    let lista: Vec<AparenciaCandidata> = if entradas == 0 { Vec::new() } else { (0..(eb_u64(builder + EB_APARENCIAS, 0x08) >> 32).min(256)).map(|i| candidata(entradas + i * EBA_TAM)).collect() };
    bwms_primitives::targets::entity_builder_aparencia_bate(c.appearance_path, c.definition_name, unica, &lista)
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
                // `EntityComponentEvent` e `VehicleLightControlEvent` herdam o `EntityLifecycleEvent`: casam pela entidade dona, como na referência.
                TipoDoEvento::Ciclo | TipoDoEvento::Componente | TipoDoEvento::LuzDoVeiculo => event_entity_ptr(args).is_some_and(|e| alvo_de_entidade_casa_entidade(&campos, e)),
                _ => true,
            }
        }
        FamiliaDoAlvo::EntidadeDinamica | FamiliaDoAlvo::EntidadeEstatica => {
            let (alvos, tabela) = if f == FamiliaDoAlvo::EntidadeDinamica { (&DYNAMICENTITYTARGET_STATES, dynent_tags()) } else { (&STATICENTITYTARGET_STATES, statent_tags()) };
            // `Matches` com `tags` vazio casa sem ler a entidade do evento (`DynamicEntityTarget.hpp:18-19`, `StaticEntityTarget.hpp:18-19`).
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
            // Valor 0 é o não-setado da referência (`if (componentName && ...)`) e fica `None`.
            let criterio = COMPONENTTARGET_STATES.lock().ok().and_then(|s| s.iter().find(|e| e.0 == p).copied()).filter(|e| e.2 != 0);
            let (nome, id) = match criterio {
                Some((_, ComponentTargetCriterion::Name, v)) => (Some(v), None),
                Some((_, ComponentTargetCriterion::Id, v)) => (None, Some(v)),
                None => (None, None),
            };
            // `ComponentTarget::Matches` só lê o componente para campo setado (`ComponentTarget.hpp:16-22`): o alvo todo curinga casa
            // sem dado do evento. Campo setado com componente que não resolve continua filtrado.
            if nome.is_none() && id.is_none() {
                return true;
            }
            event_component_ptr(args).and_then(|c| component_id_name(c)).is_some_and(|(ev_id, ev_nome)| bwms_primitives::targets::component_bate(nome, id, ev_nome, ev_id))
        }
        FamiliaDoAlvo::Tecla => {
            let Some((tecla, acao)) = inputtarget_criteria(&[p]).first().copied() else { return false };
            // `InputTarget::Matches` só lê o evento para campo setado (`InputTarget.hpp:16-22`): o alvo todo curinga casa sem tecla
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
            // `ResourceTarget::Matches`: todo critério do alvo bate — caminho igual (ou pelo padrão registrado) e classe do recurso derivada da
            // classe que a fábrica resolveu (`classe_do_alvo`).
            // Valor 0 é critério não-setado e não filtra, como o `if (path)` e o `if (type && ...)` da referência (`ResourceTarget.hpp:28`, `:46`):
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

/// Os alvos de um handler cuja família suporta o evento, na ordem do handler: os que o `AddTarget` da referência aceitaria.
pub(crate) fn alvos_que_suportam(alvos: &[usize], tipo: TipoDoEvento) -> impl Iterator<Item = (usize, FamiliaDoAlvo)> + '_ {
    alvos.iter().filter_map(|p| familia_do_alvo(*p).map(|f| (*p, f))).filter(move |(_, f)| familia_suporta(*f, tipo))
}

/// Os alvos de um handler contra o evento, como o `CallbackSystemHandler::operator()` (`CallbackSystemHandler.hpp:56-65`). `true` = handler
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

/// ArchiveXL `#47` (`CustomizationExtension`, 2026-08-14): `CustomizationEvent extends
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

/// ArchiveXL `#40` (2026-08-19, continuação direta) — composição REAL da metade "extra-slots" de
/// `AttachmentExtension::OnAttachTPP` (Extension.cpp:140-185), usando o hook confirmado ao vivo
/// `crate::jogo::anexos::ANEXO_AO_ANEXAR_VM` (2 disparos reais, padrão consistente com `OnAttach` genuíno, sessão de
/// hoje mais cedo) como gatilho. Reproduz fielmente a fonte C++ real (comentário na fonte):
///
/// `for slotID in {HeadSlot,FaceSlot}: subSlots = s_extraSlots[slotID]; for subSlotID in subSlots:`
/// `affectedAppearanceSlots.PushBack(subSlotID) se ainda não tiver; se slotData.itemObject, chama`
/// `RegisterAffectedItem(component, itemID, itemObject)`.
///
/// Peças usadas, TODAS já provadas nesta sessão/anteriores, ZERO endereço nativo novo:
/// - `TPPAffectedSlots={Head,Eyes}` — constante hardcoded (hash já conhecido).
/// - "extra slots via `.parentSlot`" — em vez de hookar `AttachmentSlots::InitializeSlots`
///   (endereço Mac desconhecido, gap SEPARADO) e manter um mapa `s_extraSlots` cacheado, fazemos a
///   MESMA pergunta on-demand pra cada slot REAL do player (via `find_attachslots_component`, já
///   provado ontem/hoje, item #54): `get_flat_tdbid(t, tweak_db_id_derive(slotID, ".parentSlot"))`
///   — leitura dinâmica de flat já provada (`tramp_get_flat_float`, TweakXL #43), mesma técnica.
///   Equivalente semântico exato (mesma pergunta, resolvida sob demanda em vez de cacheada).
/// - `FindSlotData(owner, slotID==subSlotID)` — em vez do vtable call nativo (endereço não
///   resolvido, e a predicate/lambda C++ não é chamável em ABI seguro do Rust sem RE nova), fazemos
///   a MESMA busca por composição pura: scan do array `AttachmentSlotData` real (já provado #54).
/// - `itemID = ItemObject::ItemID(itemObject).tdbid` — offset `0x288` (`Raw::ItemObject::ItemID`,
///   `enablers/ArchiveXL/src/Red/ItemObject.hpp`, campo de dado, confiança alta).
/// - `affectedAppearanceSlots.PushBack` — `dynarray_push_ptr` (já provado, usado por `AddTag`/etc.),
///   sobre o campo REAL `TPPRepresentationComponent+0xB0` (`DynArray<TweakDBID>`, offset CONFIRMADO
///   pelo header GERADO da reflection real do jogo — `TPPRepresentationComponent.hpp`).
///
/// `RegisterAffectedItem` = `0x101ea0738` (identificado em 2026-09-11, chamado via `tpp_attach.rs`): o item
/// achado no sub-slot é registrado de verdade. O dono vem do próprio componente (`IComponent+0x50`).
/// Achado OFFLINE decisivo desta mesma sessão (grep exaustivo em `tweakdbstr.kark`, 3.529.116
/// nomes conhecidos): ZERO record `AttachmentSlots.*` do jogo base (2.31) tem flat
/// `.parentSlot`/`.dependencySlots` definido — confirmado que `s_extraSlots` fica SEMPRE VAZIO pra
/// conteúdo vanilla (a feature só existe pra sub-slots que um MOD declare via `.xl`/TweakDB). Esta
/// função reconfirma isso AO VIVO por mecanismo INDEPENDENTE (lookup por-slot em vez de busca
/// global de nome).
pub unsafe fn onattachtpp_compose(reg: &crate::rtti::Registry, a_component: *mut c_void) -> String {
    if a_component.is_null() || !crate::memoria::is_readable(a_component as *const c_void, 0xC0) {
        return "[axl-40-onattachtpp] aComponent inválido/ilegível, abortado".to_string();
    }
    let head = bwms_primitives::tweak_db_id("AttachmentSlots.Head");
    let eyes = bwms_primitives::tweak_db_id("AttachmentSlots.Eyes");

    // O dono do componente (`IComponent+0x50`); o player global só se o componente não tiver dono.
    let dono = rd_u64(a_component as *const c_void, 0x50) as *mut c_void;
    let player = if dono.is_null() { crate::current_player() } else { dono };
    let attach_inst = match find_attachslots_component(reg, player) {
        Some(p) => p,
        None => return "[axl-40-onattachtpp] AttachmentSlots do dono não achado, abortado".to_string(),
    };
    let t = match crate::tweakdb::singleton() {
        Some(s) => s,
        None => return "[axl-40-onattachtpp] TweakDB singleton indisponível, abortado".to_string(),
    };

    let arr1_ptr = (attach_inst as *const u8).add(0xB0).cast::<u64>().read_unaligned();
    let arr1_capsize = (attach_inst as *const u8).add(0xB8).cast::<u64>().read_unaligned();
    let arr1_cap = (arr1_capsize & 0xFFFF_FFFF) as u32;
    let arr1_size = (arr1_capsize >> 32) as u32;
    if arr1_ptr == 0 {
        return format!("[axl-40-onattachtpp] aComponent={a_component:p}: array de AttachmentSlotData vazio");
    }
    let n = arr1_size.min(arr1_cap).min(64) as usize;

    let mut found: Vec<(u64, u64, u64)> = Vec::new(); // (base_slot, sub_slot, item_obj_instance)
    let mut checked = 0usize;
    for i in 0..n {
        let base = arr1_ptr as usize + i * 0x90;
        if !crate::memoria::is_readable(base as *const c_void, 0x90) {
            continue;
        }
        let rd = |off: usize| ((base + off) as *const u64).read_unaligned();
        let slot_id = rd(0x00);
        if slot_id == 0 {
            continue;
        }
        checked += 1;
        let parent_flat_id = bwms_primitives::tweak_db_id_derive(slot_id, ".parentSlot");
        if let Some(parent_val) = crate::tweakdb::get_flat_tdbid(t, parent_flat_id) {
            if parent_val == head || parent_val == eyes {
                found.push((parent_val, slot_id, rd(0x08)));
            }
        }
    }

    let mut out = format!(
        "[axl-40-onattachtpp] aComponent={a_component:p} AttachmentSlots={attach_inst:p} slots_checados={checked} extra_slots_achados={}",
        found.len()
    );

    if found.is_empty() {
        out.push_str(" — ZERO extra-slots (esperado: nenhum record AttachmentSlots.* do jogo base tem .parentSlot; s_extraSlots morto pra conteúdo vanilla, confirmado offline+ao-vivo)");
        return out;
    }

    // Composição real: dedupe + PushBack em affectedAppearanceSlots@+0xB0 do TPPRepresentationComponent
    // (offset confirmado pelo header GERADO da reflection real do jogo).
    let aff_arr = (a_component as *mut u8).add(0xB0) as *mut c_void;
    for (base_slot, sub_slot, item_obj) in &found {
        out.push_str(&format!(
            "\n  extra_slot base={base_slot:#018x} sub={sub_slot:#018x} item_obj={item_obj:#018x}"
        ));
        let aff_entries = rd_u64(aff_arr, 0x00) as *const u8;
        let aff_size = core::ptr::read_unaligned((aff_arr as *const u8).add(0x0C) as *const u32);
        let mut already = false;
        if !aff_entries.is_null() {
            for j in 0..aff_size.min(4096) {
                let ev = (aff_entries.add(j as usize * 8) as *const u64).read_unaligned();
                if ev == *sub_slot {
                    already = true;
                    break;
                }
            }
        }
        if already {
            out.push_str(" (já em affectedAppearanceSlots, dedupe correto — nada escrito)");
        } else {
            match dynarray_push_ptr(aff_arr, *sub_slot) {
                Some(idx) => out.push_str(&format!(" -> PushBack em affectedAppearanceSlots[{idx}] OK")),
                None => out.push_str(" -> PushBack FALHOU (array ilegível)"),
            }
        }
        if *item_obj != 0 {
            match crate::roupas_e_armas::terceira_pessoa::registrar_item_do_slot(a_component, *sub_slot) {
                Ok(item_id) => out.push_str(&format!(" — RegisterAffectedItem(itemID.tdbid={item_id:#018x}) chamado")),
                Err(motivo) => out.push_str(&format!(" — RegisterAffectedItem não chamado: {motivo}")),
            }
        }
    }

    out
}

/// Escolha do `FixCustomizationAppearance` pelo texto do seletor e pelos nomes das definições do recurso: `(índice da fonte, texto do meshAppearance)`;
/// `None` quando a referência sai sem mexer (recurso sem definição ou texto curto). `NN_resto` usa a 2ª definição (a 1ª se só há uma) e o `resto`;
/// `prefixo__resto` usa a definição cujo nome começa pelo `prefixo` (a única, se só há uma; senão a padrão) e o `resto` sem o `NN_` inicial.
pub(crate) fn es20_fonte_de_customizacao(seletor: &str, nomes: &[String]) -> Option<(usize, String)> {
    if nomes.is_empty() {
        return None;
    }
    let numerado = |t: &str| t.len() >= 3 && t.as_bytes()[2] == b'_' && t.as_bytes()[0].is_ascii_digit();
    let padrao = usize::from(nomes.len() > 1);
    if seletor.len() < 3 {
        return None;
    }
    if numerado(seletor) {
        return Some((padrao, seletor.get(3..)?.to_string()));
    }
    let Some(pos) = seletor.rfind("__") else { return Some((padrao, seletor.to_string())) };
    let fonte = if nomes.len() == 1 {
        Some(0)
    } else {
        nomes.iter().position(|n| n.len() >= pos && n.as_bytes()[..pos] == seletor.as_bytes()[..pos])
    };
    let mut resto = seletor.get(pos + 2..)?;
    if resto.len() < 3 {
        return None;
    }
    if numerado(resto) {
        resto = resto.get(3..)?;
    }
    Some((fonte.unwrap_or(padrao), resto.to_string()))
}

pub(crate) type Es20Escopos = std::collections::HashMap<u64, std::collections::HashSet<u64>>;

/// `ResourceMetaExtension::CustomizationApp`.
pub(crate) const ES20_ESCOPO_DE_CUSTOMIZACAO: u64 = bwms_primitives::resource_path_hash("player_customization.app");

/// `Es20Customizacao` pelo RTTI, com o tamanho dos 2 tipos conferido (`GetSize` `vt+0x18`).
pub(crate) unsafe fn es20_tipos_de_customizacao(reg: &rtti::Registry) -> Option<Es20Customizacao> {
    let tipo = |nome: &str, tamanho: u32| {
        let c = reg.class_by_name(nome);
        if c.is_null() {
            return None;
        }
        let obter: extern "C" fn(*mut c_void) -> u32 = std::mem::transmute(rd_u64(rd_u64(c as *const c_void, 0) as *const c_void, 0x18));
        (obter(c) == tamanho).then_some(c as usize)
    };
    let [nome, aparencia, _] = es20_campos_de_componente(reg)?;
    Some(Es20Customizacao { tipos: [tipo("appearanceAppearancePartOverrides", 0x18)?, tipo("appearancePartComponentOverrides", 0x50)?], campos: [nome, aparencia] })
}

/// Constrói um valor do tipo em `vaga` pelo construtor do RTTI (`vt+0x40`), como o `EmplaceBack`.
pub(crate) unsafe fn es20_construir(tipo: usize, vaga: *mut u8) {
    let construir: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(rd_u64(rd_u64(tipo as *const c_void, 0) as *const c_void, 0x40));
    construir(tipo as *mut c_void, vaga as *mut c_void);
}

/// Leitores do `compiledData` no Mac: `(vtable estática, GetType, maker)`. O maker é o `BufferReader::MakeType0`/`MakeType1` da referência: `x0` = campos do
/// leitor de origem a partir do `+0x08`, `x8` = saída do leitor novo (RE em `DATABASE.md`).
pub(crate) const ES20_LEITORES: [(u64, u8, u64); 2] = [(0x1_0783_87d0, 0, 0x1_0121_743c), (0x1_0783_8788, 1, 0x1_0121_74ac)];

/// `IsFixedCustomizationAppearance`: 1 override de parte com 1 override de componente sem `componentName`.
pub(crate) unsafe fn es20_customizacao_fixa(c: &Es20Customizacao, definicao: *const u8) -> bool {
    let externo = definicao.add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF);
    let b = rd_u64(externo as *const c_void, 0) as *const u8;
    if b.is_null() || (externo.add(0x0c) as *const u32).read_unaligned() != 1 {
        return false;
    }
    let cb = rd_u64(b.add(8) as *const c_void, 0) as *const u8;
    !cb.is_null() && (b.add(0x14) as *const u32).read_unaligned() == 1 && (cb.add(c.campos[0]) as *const u64).read_unaligned() == 0
}

/// `FixCustomizationAppearance(recurso, definição, seletor)` com a saída ainda vazia: `Some(anexada)` quando pôs uma definição na saída (a nova, anexada ao
/// recurso, ou a própria fonte), `None` quando a referência sai sem mexer. Recurso fora do escopo `player_customization.app` não muda. Divergência: leitor
/// nulo na fonte não é clonado (a referência chama o `Clone` sem conferir).
pub(crate) unsafe fn es20_consertar_customizacao(c: &Es20Customizacao, recurso: *mut u8, saida: *mut [u64; 2], seletor: u64) -> Option<bool> {
    let arr = recurso.add(crate::jogo::aparencia::APPRESOURCE_APPEARANCES_OFF);
    if saida.is_null() || rd_u64(arr as *const c_void, 0) == 0 || (arr.add(0x0c) as *const u32).read_unaligned() == 0 {
        return None;
    }
    if !es20_no_escopo(ES20_ESCOPO_DE_CUSTOMIZACAO, (recurso.add(0x30) as *const u64).read_unaligned()) {
        return None;
    }
    let texto = crate::cname::name_of(seletor)?;
    if texto.len() < 3 {
        return None;
    }
    es22_com_trava(recurso, || {
        let b = rd_u64(arr as *const c_void, 0) as *const u64;
        let n = ((arr.add(0x0c) as *const u32).read_unaligned() as usize).min(65536);
        let handles: Vec<[u64; 2]> = (0..n).map(|i| [b.add(i * 2).read_unaligned(), b.add(i * 2 + 1).read_unaligned()]).collect();
        let nome_de = |h: &[u64; 2]| ((h[0] as usize + crate::jogo::aparencia::APPDEF_NAME_OFF) as *const u64).read_unaligned();
        let nomes: Vec<String> = handles.iter().map(|h| if h[0] == 0 { String::new() } else { crate::cname::name_of(nome_de(h)).unwrap_or_default() }).collect();
        let (i, aparencia) = es20_fonte_de_customizacao(&texto, &nomes)?;
        let fonte = handles[i];
        if fonte[0] == 0 {
            return None;
        }
        let definicao = fonte[0] as *mut u8;
        let externo = definicao.add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF);
        let mesh = crate::cname::registrar_no_pool(&aparencia);
        if (externo.add(0x0c) as *const u32).read_unaligned() > 0 {
            if es20_customizacao_fixa(c, definicao) {
                handle_strong_soma(&fonte, 1);
                *saida = fonte;
                return Some(false);
            }
            let nova = es22_definicao_nova(definicao as *mut c_void, seletor, true)?;
            es20_clonar_leitor((nova[0] as usize + ES20_DEF_LEITOR) as *mut u64, rd_u64(definicao as *const c_void, ES20_DEF_LEITOR) as *const u8);
            let p = rd_u64((nova[0] as *const u8).add(crate::jogo::aparencia::APPDEF_PARTSOVERRIDES_OFF) as *const c_void, 0) as *const u8;
            if !p.is_null() {
                let cb = rd_u64(p.add(8) as *const c_void, 0) as *mut u8;
                let cn = if cb.is_null() { 0 } else { (p.add(0x14) as *const u32).read_unaligned() as usize };
                for k in 0..cn.min(4096) {
                    (cb.add(k * 0x50 + c.campos[1]) as *mut u64).write_unaligned(mesh);
                }
            }
            let anexada = es22_anexar_definicao_travado(recurso, nova);
            *saida = nova;
            return Some(anexada);
        }
        (definicao.add(crate::jogo::aparencia::APPDEF_NAME_OFF) as *mut u64).write_unaligned(seletor);
        let parte = es22_vaga_no_fim(externo, 0x18)?;
        es20_construir(c.tipos[0], parte);
        (externo.add(0x0c) as *mut u32).write_unaligned((externo.add(0x0c) as *const u32).read_unaligned() + 1);
        let componentes = parte.add(8);
        let componente = es22_vaga_no_fim(componentes, 0x50)?;
        es20_construir(c.tipos[1], componente);
        (componentes.add(0x0c) as *mut u32).write_unaligned((componentes.add(0x0c) as *const u32).read_unaligned() + 1);
        (componente.add(c.campos[1]) as *mut u64).write_unaligned(mesh);
        handle_strong_soma(&fonte, 1);
        *saida = fonte;
        Some(false)
    })
}

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

/// Garante espaço pra mais 1 no `DynArray` pelo reserve do motor (mover nulo: a realocação move os bytes), crescendo como o SDK (`+metade`, mínimo 4).
pub(crate) unsafe fn cust_reservar_mais_um(arr: usize, passo: usize) -> bool {
    let (_, n, cap) = cust_arr(arr);
    if n < cap {
        return true;
    }
    crate::jogo::listas::reservar(arr as *mut c_void, (cap + cap / 2).max(n + 1).max(4) as u32, passo as u32, 8, std::ptr::null());
    cust_arr(arr).2 > n
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

/// Nome registrado de uma escolha: o `name` da definição, ou o FNV-1a64 do subnome com o nome da opção como semente (`RegisterCustomEntryName(name, sub)`).
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

/// Posição de `v` num array ordenado de `u64`: `Ok` se já está, `Err(onde entra)` senão.
pub(crate) fn cust_posicao_ordenada(v: &[u64], nome: u64) -> Result<usize, usize> {
    v.binary_search(&nome)
}

/// Nomes de opção e de escolha de mod (`m_customEntryNames`).
pub(crate) static CUST_NOMES: std::sync::RwLock<Option<std::collections::HashSet<u64>>> = std::sync::RwLock::new(None);

pub(crate) static CUST_NOMES_N: AtomicU64 = AtomicU64::new(0);

pub(crate) fn cust_registrar_nome(n: u64) {
    if let Ok(mut g) = CUST_NOMES.write() {
        let s = g.get_or_insert_with(Default::default);
        s.insert(n);
        CUST_NOMES_N.store(s.len() as u64, Ordering::Relaxed);
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

/// `FixCustomizationOptions`: caminho mapeado pelo `resource.fix` do recurso base, com override do caminho antigo e dos aliases do caminho novo
/// (`reslink_aliases_of`: links e cópias, nos dois sentidos) pra cada definição.
pub(crate) unsafe fn cust_consertar_caminhos(c: &CustCampos, m: &mut CustMescla, caminho_alvo: u64, opcoes: usize) {
    let pares = match CUST_FIX_CAMINHOS.read() {
        Ok(g) => match g.as_ref().and_then(|t| t.get(&caminho_alvo)) {
            Some(p) => p.clone(),
            None => return,
        },
        Err(_) => return,
    };
    let l = c.listas[0];
    for k in 0..cust_arr(opcoes).1 {
        let t = cust_handle(opcoes, k)[0] as usize;
        if cust_tipo_da_opcao(c, t) != Some(CustTipo::App) {
            continue;
        }
        let antigo = cu64(t + c.recurso);
        let Some(novo) = pares.iter().find(|p| p.0 == antigo).map(|p| p.1) else { continue };
        if novo == antigo {
            continue;
        }
        ((t + c.recurso) as *mut u64).write_unaligned(novo);
        m.caminhos.push((t, antigo));
        m.contagem.caminhos += 1;
        let aliases = crate::arquivos_do_jogo::reslink_aliases_of(novo);
        let (e, n, _) = cust_arr(t + l.campo);
        for d in 0..n {
            let nome = cu64(e + d * l.passo + l.chave);
            m.override_(antigo, novo, nome);
            for a in &aliases {
                m.override_(*a, novo, nome);
            }
        }
    }
}

/// `MergeCustomGroups`: cada entrada de um grupo da fonte vai pro fim do grupo de mesmo nome no alvo.
pub(crate) unsafe fn cust_mesclar_grupos(m: &mut CustMescla, alvo: usize, fonte: usize) {
    let (fe, fnn, _) = cust_arr(fonte);
    let (ae, an, _) = cust_arr(alvo);
    for i in 0..fnn {
        let g = fe + i * CUST_GRUPO_PASSO;
        let (oe, on, _) = cust_arr(g + 8);
        for j in 0..an {
            let t = ae + j * CUST_GRUPO_PASSO;
            if cu64(t) != cu64(g) {
                continue;
            }
            m.anotar(t + 8, CustElemento::CName);
            for k in 0..on {
                let nome = cu64(oe + k * 8);
                if cust_anexar_u64(t + 8, nome) {
                    m.contagem.grupos += 1;
                }
                cust_registrar_nome(nome);
            }
        }
    }
}

/// `ExpandCustomizationOptions`: opção de app sem nome e sem definições herda as da última sem nome com definições (e o recurso, se não tiver).
pub(crate) unsafe fn cust_expandir(c: &CustCampos, fonte: usize) -> u32 {
    let l = c.listas[0];
    let (mut cheia, mut n) = (0usize, 0u32);
    for k in 0..cust_arr(fonte).1 {
        let o = cust_handle(fonte, k)[0] as usize;
        if cust_tipo_da_opcao(c, o) != Some(CustTipo::App) || cu64(o + c.nome) != 0 {
            continue;
        }
        if cust_arr(o + l.campo).1 != 0 {
            cheia = o;
            continue;
        }
        if cheia == 0 {
            continue;
        }
        rtti::type_assign(l.tipo_array as *mut c_void, (o + l.campo) as *mut c_void, (cheia + l.campo) as *const c_void);
        if cu64(o + c.recurso) == 0 && cu64(cheia + c.recurso) != 0 {
            ((o + c.recurso) as *mut u64).write_unaligned(cu64(cheia + c.recurso));
        }
        n += 1;
    }
    n
}

/// As escolhas de `src` entram em `t` (mesmo tipo): mesma chave substitui, chave nova vai pro fim e é registrada.
pub(crate) unsafe fn cust_mesclar_escolhas(c: &CustCampos, m: &mut CustMescla, t: usize, src: usize, tipo: CustTipo) {
    let l = c.listas[tipo as usize];
    let (lista_alvo, lista_fonte) = (t + l.campo, src + l.campo);
    let dono = cu64(t + c.nome);
    let cabelo = tipo == CustTipo::App && cu64(t + c.slot) == cname("hair_color");
    for k in 0..cust_arr(lista_fonte).1 {
        let se = cust_arr(lista_fonte).0 + k * l.passo;
        let chave = cust_chave(&l, tipo, se);
        let (te, tn, _) = cust_arr(lista_alvo);
        match (0..tn).find(|p| cust_chave(&l, tipo, te + p * l.passo) == chave) {
            Some(p) => {
                let vaga = te + p * l.passo;
                if !cust_nome_de_mod(cust_nome_da_escolha(dono, &chave)) && !m.trocas.iter().any(|x| x.dono == t && x.tipo == tipo && x.pos == p) {
                    let mut copia = vec![0u64; l.passo.div_ceil(8)];
                    let a = copia.as_mut_ptr() as usize;
                    cust_tipo_fn(l.tipo, 0x40, a);
                    rtti::type_assign(l.tipo as *mut c_void, a as *mut c_void, vaga as *const c_void);
                    m.trocas.push(CustTroca { dono: t, tipo, pos: p, copia });
                }
                rtti::type_assign(l.tipo as *mut c_void, vaga as *mut c_void, se as *const c_void);
                m.contagem.trocas += 1;
            }
            None => {
                m.anotar(lista_alvo, CustElemento::Valor { tipo: l.tipo, passo: l.passo });
                if cust_anexar_valor(lista_alvo, l.tipo, l.passo, se).is_some() {
                    m.contagem.escolhas[tipo as usize] += 1;
                    if let CustChave::Nome(nome_def) = chave {
                        let (r_alvo, r_fonte) = (cu64(t + c.recurso), cu64(src + c.recurso));
                        if r_fonte != 0 && r_fonte != r_alvo {
                            m.override_(r_alvo, r_fonte, nome_def);
                        }
                    }
                    cust_registrar_nome(cust_nome_da_escolha(dono, &chave));
                }
            }
        }
        if cabelo {
            let (te, tn, _) = cust_arr(se + c.tags);
            for q in 0..tn {
                let tag = cu64(te + q * 8);
                if tag != 0 && !m.cores.contains(&tag) {
                    m.cores.push(tag);
                    m.contagem.cores += 1;
                }
            }
        }
    }
}

/// `MergeCustomOptions(alvo, fonte, slots)`.
pub(crate) unsafe fn cust_mesclar_opcoes(c: &CustCampos, m: &mut CustMescla, alvo: usize, fonte: usize, slots: bool) {
    let n_fonte = cust_arr(fonte).1;
    for i in 0..n_fonte {
        let h_fonte = cust_handle(fonte, i);
        let mut src = h_fonte[0] as usize;
        if src == 0 {
            continue;
        }
        let (nome, slot, link) = (cu64(src + c.nome), cu64(src + c.slot), cu64(src + c.link));
        if (slots && (nome != 0 || (slot == 0 && link == 0))) || (!slots && nome == 0) {
            continue;
        }
        let texto = |n: u64| if slots && n != 0 { crate::cname::resolve_cname(n) } else { String::new() };
        let (slot_txt, link_txt) = (texto(slot), texto(link));
        let (curinga_slot, curinga_link) = (cust_curinga(&slot_txt), cust_curinga(&link_txt));
        let mut existe = false;
        for j in 0..cust_arr(alvo).1 {
            let t = cust_handle(alvo, j)[0] as usize;
            if t == 0 {
                continue;
            }
            if slots {
                if slot != 0 && !cust_casa(cu64(t + c.slot), || crate::cname::resolve_cname(cu64(t + c.slot)), slot, curinga_slot) {
                    continue;
                }
                if link != 0 && !cust_casa(cu64(t + c.link), || crate::cname::resolve_cname(cu64(t + c.link)), link, curinga_link) {
                    continue;
                }
            } else {
                if cu64(t + c.nome) != nome {
                    continue;
                }
                if link != 0 {
                    match (0..n_fonte).map(|k| cust_handle(fonte, k)[0] as usize).find(|o| *o != 0 && cu64(o + c.nome) == link) {
                        Some(o) => src = o,
                        None => continue,
                    }
                }
            }
            existe = true;
            let tipo = cust_tipo_da_opcao(c, t);
            if tipo.is_none() || rtti::class_of(t as *mut c_void) != rtti::class_of(src as *mut c_void) {
                m.contagem.incompativeis += 1;
                continue;
            }
            let tipo = tipo.unwrap_or(CustTipo::App);
            if tipo == CustTipo::App
                && (curinga_slot.is_some() || curinga_link.is_some())
                && (cu64(t + c.recurso) == 0 || cust_arr(t + c.listas[0].campo).1 == 0)
            {
                continue;
            }
            cust_mesclar_escolhas(c, m, t, src, tipo);
        }
        if !existe && !slots {
            m.anotar(alvo, CustElemento::Handle);
            if cust_anexar_handle(alvo, h_fonte) {
                cust_registrar_nome(nome);
                m.contagem.opcoes += 1;
            }
        }
    }
}

/// `RegenerateIndexes`: `index` = posição em cada lista de escolhas; o de antes fica guardado.
pub(crate) unsafe fn cust_regenerar(c: &CustCampos, m: &mut CustMescla, opcoes: usize) {
    for k in 0..cust_arr(opcoes).1 {
        let t = cust_handle(opcoes, k)[0] as usize;
        let Some(tipo) = cust_tipo_da_opcao(c, t) else { continue };
        let l = c.listas[tipo as usize];
        let (e, n, _) = cust_arr(t + l.campo);
        for p in 0..n {
            let campo = (e + p * l.passo + l.indice) as *mut i32;
            let antigo = campo.read_unaligned();
            if antigo != p as i32 {
                m.indices.push((t, tipo, p, antigo));
                campo.write_unaligned(p as i32);
                m.contagem.indices += 1;
            }
        }
    }
}

/// `MergeCustomEntries(alvo, fontes)` num recurso base: `resource.fix`, grupos, expansão e opções de cada fonte, o passe de slots e links, índices.
pub(crate) unsafe fn cust_mesclar_recurso(c: &CustCampos, m: &mut CustMescla, alvo: usize, caminho: u64, fontes: &[usize]) {
    for k in [2usize, 1, 0] {
        cust_consertar_caminhos(c, m, caminho, alvo + CUST_OPCOES_OFF[k]);
    }
    for f in fontes {
        for k in [2usize, 1, 0] {
            cust_mesclar_grupos(m, alvo + CUST_GRUPOS_OFF[k], f + CUST_GRUPOS_OFF[k]);
        }
        for k in [2usize, 1, 0] {
            m.contagem.expandidas += cust_expandir(c, f + CUST_OPCOES_OFF[k]);
        }
        for k in [2usize, 1, 0] {
            cust_mesclar_opcoes(c, m, alvo + CUST_OPCOES_OFF[k], f + CUST_OPCOES_OFF[k], false);
        }
    }
    for f in fontes {
        for k in [2usize, 1, 0] {
            cust_mesclar_opcoes(c, m, alvo + CUST_OPCOES_OFF[k], f + CUST_OPCOES_OFF[k], true);
        }
    }
    for k in [2usize, 1, 0] {
        cust_regenerar(c, m, alvo + CUST_OPCOES_OFF[k]);
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

/// `OnPrepareResource` → `MergeCustomEntries(sys)`: uma vez, nos recursos base de `sys+0x48` (masculino) e `+0x58` (feminino), com as fontes de cada
/// um; as cores de cabelo vão pro flat no fim. `None` se já mesclado ou desligado.
pub(crate) unsafe fn cust_mesclar_no_sistema(sys: u64) -> Option<CustContagem> {
    if !CUST_LIGADO.load(Ordering::Relaxed) || sys == 0 {
        return None;
    }
    let c = cust_campos()?;
    let mut g = CUST.lock().ok()?;
    let m = g.get_or_insert_with(Default::default);
    if m.mesclado {
        return None;
    }
    let fontes = CUST_FONTES.lock().ok()?.clone();
    m.contagem = CustContagem::default();
    for (k, off) in [(0usize, 0x48u64), (1, 0x58)] {
        let token = cu64((sys + off) as usize);
        let Some(alvo) = cust_recurso_do_token(&c, token, 250) else {
            crate::log_dev!("[a47c] recurso base {k} não pronto (token {token:#x})");
            continue;
        };
        let caminho = cu64(token as usize + 0x48);
        let so_montadas = CUST_SO_MONTADAS.load(Ordering::Relaxed);
        let lista: Vec<usize> = fontes[k]
            .iter()
            .filter_map(|f| match f {
                CustFonte::Token(_) if so_montadas => None,
                CustFonte::Token(t) => cust_recurso_do_token(&c, t[0], 0).map(|h| h[0] as usize),
                CustFonte::Recurso(r) => Some(*r),
            })
            .collect();
        m.contagem.fontes += lista.len() as u32;
        (*(alvo[1] as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
        m.alvos.push(alvo);
        m.contagem.alvos += 1;
        cust_mesclar_recurso(&c, m, alvo[0] as usize, caminho, &lista);
    }
    if !m.cores.is_empty() {
        if let Some(t) = crate::tweakdb::singleton() {
            let id = bwms_primitives::tweak_db_id(CUST_CORES_FLAT);
            if let Some(antes) = crate::tweakdb::get_flat_array_u64_by_id(t as *const u8, id) {
                let mut nova = antes.clone();
                nova.extend(m.cores.iter().filter(|x| !antes.contains(x)));
                if nova.len() > antes.len() && crate::tweakdb::set_flat_array_u64_by_id(t, id, id, &nova).is_some() {
                    m.cores_antes = Some(antes);
                }
            }
        }
    }
    m.mesclado = true;
    let n = m.contagem;
    crate::veredito_dev!("[a47c] mescla de customização: {n:?}");
    Some(n)
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

/// `OnInitAppOption`/`OnInitMorphOption` (antes da original): opção de mod fora das opções do estado entra nelas, na ordem (`SortedUniqueArray::Emplace`).
pub(crate) unsafe fn cust_marcar_estado(opcao: u64, estado: u64) -> bool {
    let Some((_, nome, visivel)) = cust_opcao_do_estado(opcao) else { return false };
    if !visivel || estado == 0 || !cust_nome_de_mod(nome) {
        return false;
    }
    let arr = estado as usize;
    let (e, n, _) = cust_arr(arr);
    let atuais: Vec<u64> = if n == 0 { Vec::new() } else { std::slice::from_raw_parts(e as *const u64, n).to_vec() };
    let Err(pos) = cust_posicao_ordenada(&atuais, nome) else { return false };
    if !cust_reservar_mais_um(arr, 8) {
        return false;
    }
    let e = cust_arr(arr).0;
    std::ptr::copy((e + pos * 8) as *const u64, (e + (pos + 1) * 8) as *mut u64, n - pos);
    ((e + pos * 8) as *mut u64).write_unaligned(nome);
    ((arr + 0xc) as *mut u32).write_unaligned(n as u32 + 1);
    #[cfg(feature = "devtools")]
    CUST_MARCAS[0].fetch_add(1, Ordering::Relaxed);
    true
}

/// `OnInitSwitcherOption` (depois da original): opção de mod inativa passa a ativa.
pub(crate) unsafe fn cust_marcar_ativo(opcao: u64) -> bool {
    let Some((obj, nome, visivel)) = cust_opcao_do_estado(opcao) else { return false };
    let Some(c) = CUST_CAMPOS.try_lock().ok().and_then(|g| *g) else { return false };
    if !visivel || cu8(obj + c.opcao_ativo) != 0 || !cust_nome_de_mod(nome) {
        return false;
    }
    *((obj + c.opcao_ativo) as *mut u8) = 1;
    #[cfg(feature = "devtools")]
    CUST_MARCAS[1].fetch_add(1, Ordering::Relaxed);
    true
}

/// `bwms-customizations.txt` (`<male|female> | <caminho>`, `bwms_primitives::tabelas::Customizacao`), na carga das tabelas (game thread): desfaz a mescla
/// anterior (o `Reload` da referência), pede o token de cada recurso e liga os hooks. Devolve as fontes.
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
/// vazia, desfaz a mescla, tira os tokens das fontes e desliga `CUST_LIGADO` (o `Reload` da referência). Sem fonte anterior não há o que desfazer, e o boot
/// sem customizations não resolve os campos do RTTI.
pub(crate) unsafe fn cust_tabela_ausente() {
    if CUST_FONTES.lock().is_ok_and(|f| cust_tem_fonte_de_tabela(&f)) {
        cust_tabela("");
    }
}


/// `BwmsPuppetStateFireOnce() -> Bool` — axl-puppet-state-apply: dispara `MenuScenario_CharacterCustomizationMirror`
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

/// Codeware `#24` `Entity.ApplyMorphTarget` (2026-09-11): `MorphTargetManager::ApplyMorphTarget` = `0x10197f17c`
/// (`x0` manager, `x1` target, `x2` region, `s0` value, `w3` bool), achado offline
/// (`proofs/20260911-codeware-24-applymorphtarget-identificado.log`). O manager é o 1º componente da entidade cuja
/// classe descende de `entMorphTargetManagerComponent`; os pesos ficam num `DynArray<entMorphTargetWeightEntry>` em
/// `+0x98` (entrada de 0x18: target, region, peso f32 em `+0x10`).
pub(crate) const MORPH_APPLY_VM: u64 = 0x1_0197_f17c;

#[cfg(feature = "devtools")]
pub(crate) const MORPH_PESOS_OFF: usize = 0x98;

#[cfg(feature = "devtools")]
pub(crate) const MORPH_ENTRADA_TAM: usize = 0x18;

pub(crate) unsafe fn morph_manager_de(entity: *mut c_void) -> *mut c_void {
    if !rtti::sane(entity) {
        return std::ptr::null_mut();
    }
    let arr = (entity as *const u8).add(0xA0) as *const c_void;
    if !crate::memoria::is_readable(arr, 0x10) {
        return std::ptr::null_mut();
    }
    let entries = rd_u64(arr, 0x00) as *const u8;
    let size = core::ptr::read_unaligned((arr as *const u8).add(0x0C) as *const u32);
    if entries.is_null() || size >= 10_000 {
        return std::ptr::null_mut();
    }
    let alvo = cname("entMorphTargetManagerComponent");
    for i in 0..size as usize {
        let slot = entries.add(i * 16);
        if !crate::memoria::is_readable(slot as *const c_void, 8) {
            continue;
        }
        let inst = core::ptr::read_unaligned(slot as *const u64) as *mut c_void;
        if !rtti::sane(inst) {
            continue;
        }
        if classe_deriva_de_nome(rtti::class_of(inst), alvo) {
            return inst;
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

/// `BwmsEntityApplyMorphTarget(entity: ref<Entity>, target: CName, region: CName, value: Float) -> Bool` — true
/// quando a entidade tem o manager (como o Codeware), mesmo que o motor não mude nada (limiar de 0.01).
pub(crate) unsafe extern "C" fn tramp_entity_apply_morph_target(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let entity = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let target = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let region = args.get(2).map(|(v, _)| *v).unwrap_or(0);
    let valor = f32::from_bits(args.get(3).map(|(v, _)| *v as u32).unwrap_or(0));
    let manager = morph_manager_de(entity);
    let ok = !manager.is_null();
    if ok {
        let aplicar: extern "C" fn(*mut c_void, u64, u64, f32, bool) = std::mem::transmute(crate::rebase(MORPH_APPLY_VM));
        aplicar(manager, target, region, valor, false);
    }
    crate::log_dev!(
        "[entity] BwmsEntityApplyMorphTarget(target={target:#x}, region={region:#x}, valor={valor}) manager={manager:p} -> {ok}"
    );
    write_bool_ret(out, ok);
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

/// Tipos e campos do `FixCustomizationAppearance`, pelo RTTI na thread do jogo: `appearanceAppearancePartOverrides` (0x18 B) e
/// `appearancePartComponentOverrides` (0x50 B), com o construtor de cada um no `EmplaceBack`, e `componentName`/`meshAppearance` do override de componente.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Es20Customizacao {
    pub(crate) tipos: [usize; 2],
    pub(crate) campos: [usize; 2],
}

impl Es20Customizacao {
    /// `componentName` e `meshAppearance` do override de componente.
    pub(crate) fn campos(&self) -> [usize; 2] {
        self.campos
    }
}

/// Leitor do `compiledData` da definição (`DeferredDataBuffer` `+0xF0`, leitor `+0x30`).
pub(crate) const ES20_DEF_LEITOR: usize = 0x120;

/// `BufferReader::Clone(destino, origem)`: o maker do tipo do leitor de origem (`GetType` `vt+0x28`) grava um leitor novo em `*destino`; `false` sem origem ou
/// com leitor fora dos 2 tipos medidos.
pub(crate) unsafe fn es20_clonar_leitor(destino: *mut u64, origem: *const u8) -> bool {
    if origem.is_null() || destino.is_null() {
        return false;
    }
    let vtable = (origem as *const u64).read_unaligned();
    let estatica = vtable.wrapping_sub(crate::rebase(0x1_0000_0000) as u64).wrapping_add(0x1_0000_0000);
    let Some((_, tipo, maker)) = ES20_LEITORES.iter().find(|(v, _, _)| *v == estatica) else { return false };
    let obter_tipo: extern "C" fn(*const u8) -> u8 = std::mem::transmute(((vtable + 0x28) as *const u64).read_unaligned());
    if obter_tipo(origem) != *tipo {
        return false;
    }
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(*maker) as u64,
        in("x0") origem as u64 + 8,
        in("x8") destino as u64,
        clobber_abi("C"),
    );
    true
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

// ===== ArchiveXL `#47` 47a: o `ApplyAppOverride` nos 6 `Get*Appearances` do estado de customização, e os 3 `Initialize*Option` observados =====
// RE offline de 2026-09-18 (`proofs/20260918-archivexl-47-re-offline.log`): os `Get{Head,Body,Arms}Appearances{1,2}` são virtuais do
// `CharacterCustomizationState` (vtable `0x107cb57e8`, slots `+0x198`..`+0x1c0`), `(estado, CName grupo, bool fpp, DynArray<AppearanceDescriptor>& saída)`,
// e empurram os descritores na saída pelo realloc do motor; os controllers de customização do V chamam as versões 2 com TPP na montagem. A referência
// (`Customization/Extension.cpp:145`) roda o `ApplyAppOverride` em cada descritor depois da original — a mesma tabela do `#11`. Os
// `InitializeAppOption`/`InitializeMorphOption`/`InitializeSwitcherOption` (`0x104bd3f5c`/`0x104bd448c`/`0x104bd4a70`) montam as opções na carga do save
// (`0x104bc4c48`, vt+0x150); na referência só agem com nomes de entrada de mod, que vêm da mescla dos recursos (47c). Aqui ficam só observados, com
// `~/.bwms-hook-a47a-teste`, sem RTTI dentro do hook (pode rodar em thread de job).
pub(crate) const A47_GET_VM: [u64; 6] = [0x1_04bb_f004, 0x1_04bb_f438, 0x1_04bb_f6f8, 0x1_04bb_f9b8, 0x1_04bb_fc78, 0x1_04bb_ff38];

pub(crate) const A47_GET_PROLOGO: u32 = 0xd101_c3ff;

pub(crate) static ORIG_A47_GET: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

/// `InitializeAppOption`/`InitializeMorphOption`/`InitializeSwitcherOption`: endereço, prólogo.
pub(crate) const A47_INIT: [(u64, u32); 3] = [(0x1_04bd_3f5c, 0xd102_03ff), (0x1_04bd_448c, 0xd102_83ff), (0x1_04bd_4a70, 0xd101_03ff)];

pub(crate) static ORIG_A47_INIT: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

pub(crate) unsafe fn a47_get(k: usize, estado: u64, grupo: u64, fpp: u64, saida: *mut u64) -> u64 {
    let f: unsafe extern "C" fn(u64, u64, u64, *mut u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_GET[k]));
    let r = f(estado, grupo, fpp, saida);
    #[cfg(any(test, feature = "devtools"))]
    A47_GET_CHAMADAS[k].fetch_add(1, Ordering::Relaxed);
    // Tabela do `#11` vazia (o público sem mod do `#11`/`#47`): nada a trocar, então nem o `is_readable` da saída (1 ida ao kernel por chamada).
    if crate::aparencia::APP_OVERRIDES_N.load(Ordering::Relaxed) == 0 {
        return r;
    }
    if saida.is_null() || !crate::memoria::is_readable(saida as *const c_void, 0x10) {
        return r;
    }
    let entradas = saida.read_unaligned();
    let n = ((saida as *const u8).add(0xc) as *const u32).read_unaligned() as usize;
    if entradas == 0 || n == 0 || n > 4096 {
        return r;
    }
    #[cfg(any(test, feature = "devtools"))]
    A47_GET_DESCRITORES[k].fetch_add(n as u64, Ordering::Relaxed);
    let trocados = crate::aparencia::aplicar_app_overrides(entradas as *mut u64, n);
    if trocados > 0 {
        #[cfg(any(test, feature = "devtools"))]
        A47_GET_TROCADOS[k].fetch_add(trocados as u64, Ordering::Relaxed);
        #[cfg(feature = "devtools")]
        if A47_TESTE.load(Ordering::Relaxed) {
            if let Ok(mut a) = A47_GET_AMOSTRA.try_lock() {
                if a.len() < 8 {
                    a.push((k, grupo, fpp & 0xff, (entradas as *const [u64; 2]).read_unaligned()));
                }
            }
        }
    }
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

/// Com a mescla do `#47` (47c), o `OnInitAppOption`/`OnInitMorphOption` antes da original e o `OnInitSwitcherOption` depois.
pub(crate) unsafe extern "C" fn a47_init_app(sys: u64, parte: u64, opcao: u64, estado: u64, slots: u64) -> u64 {
    #[cfg(feature = "devtools")]
    A47_INIT_CHAMADAS[0].fetch_add(1, Ordering::Relaxed);
    #[cfg(feature = "devtools")]
    A47_INIT_PARTES[0].fetch_or(1 << (parte & 0x3), Ordering::Relaxed);
    #[cfg(feature = "devtools")]
    A47_INIT_ESTADO_MAX[0].fetch_max(a47_tamanho(estado), Ordering::Relaxed);
    crate::corpo_e_rosto::cust_marcar_estado(opcao, estado);
    let f: unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[0]));
    f(sys, parte, opcao, estado, slots)
}

pub(crate) unsafe extern "C" fn a47_init_morph(sys: u64, opcao: u64, estado: u64, slots: u64) -> u64 {
    #[cfg(feature = "devtools")]
    A47_INIT_CHAMADAS[1].fetch_add(1, Ordering::Relaxed);
    #[cfg(feature = "devtools")]
    A47_INIT_ESTADO_MAX[1].fetch_max(a47_tamanho(estado), Ordering::Relaxed);
    crate::corpo_e_rosto::cust_marcar_estado(opcao, estado);
    let f: unsafe extern "C" fn(u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[1]));
    f(sys, opcao, estado, slots)
}

pub(crate) unsafe extern "C" fn a47_init_switcher(sys: u64, parte: u64, opcao: u64, indice: u64, slots: u64) -> u64 {
    #[cfg(feature = "devtools")]
    A47_INIT_CHAMADAS[2].fetch_add(1, Ordering::Relaxed);
    #[cfg(feature = "devtools")]
    A47_INIT_PARTES[2].fetch_or(1 << (parte & 0x3), Ordering::Relaxed);
    let f: unsafe extern "C" fn(u64, u64, u64, u64, u64) -> u64 = std::mem::transmute(original_publicado(&ORIG_A47_INIT[2]));
    let r = f(sys, parte, opcao, indice, slots);
    crate::corpo_e_rosto::cust_marcar_ativo(opcao);
    r
}

/// No `on_load`: os 6 hooks dos `Get*Appearances` sempre (com a tabela do `#11` vazia só repassam, como na referência); com `~/.bwms-hook-a47a-teste`,
/// os 3 `Initialize*Option` observados e os pares de teste do `#11` com override antes da carga.
pub(crate) unsafe fn install_a47a() {
    #[cfg(feature = "devtools")]
    let teste = crate::marcador_individual("a47a-teste");
    #[cfg(feature = "devtools")]
    A47_TESTE.store(teste, Ordering::Relaxed);
    let hooks: [unsafe extern "C" fn(u64, u64, u64, *mut u64) -> u64; 6] = [a47_get0, a47_get1, a47_get2, a47_get3, a47_get4, a47_get5];
    let gets: Vec<Option<bool>> =
        (0..6).map(|k| instalar_conferindo_prologo(A47_GET_VM[k], A47_GET_PROLOGO, hooks[k] as *mut c_void, &ORIG_A47_GET[k])).collect();
    #[cfg(feature = "devtools")]
    {
        let mut inits: Vec<Option<bool>> = Vec::new();
        let mut pre = 0;
        if teste {
            let destinos: [*mut c_void; 3] = [a47_init_app as *mut c_void, a47_init_morph as *mut c_void, a47_init_switcher as *mut c_void];
            inits = (0..3).map(|k| instalar_conferindo_prologo(A47_INIT[k].0, A47_INIT[k].1, destinos[k], &ORIG_A47_INIT[k])).collect();
            install_reslink();
            for (r, a) in a11_pares_gravados() {
                let alias = a11_alias(r);
                reslink_add(alias, r);
                crate::aparencia::registrar_app_override(r, alias, a);
                pre += 1;
            }
        }
        crate::veredito_dev!(
            "[a47a] hooks no on_load: Get*Appearances {gets:?}, Initialize*Option {inits:?} (teste {teste}, {pre} par(es) do #11 com override antes da carga)"
        );
    }
    #[cfg(not(feature = "devtools"))]
    let _ = gets;
}

pub(crate) const A47_INITIALIZE: (u64, u32) = (0x1_04bc_779c, 0xd101_c3ff);

pub(crate) const A47_UNINITIALIZE: (u64, u32) = (0x1_04bc_83a8, 0xa9be_4ff4);

pub(crate) static ORIG_A47_INITIALIZE: AtomicU64 = AtomicU64::new(0);

pub(crate) static ORIG_A47_UNINITIALIZE: AtomicU64 = AtomicU64::new(0);

/// Antes da original: a mescla do `#47` (o `OnPrepareResource` da referência — o `GetResource` é inlinado no `Initialize` do Mac).
pub(crate) unsafe extern "C" fn a47_initialize(sys: u64, puppet: u64, masc: u64, functor: u64) {
    #[cfg(feature = "devtools")]
    A47_CICLO_CHAMADAS[0].fetch_add(1, Ordering::Relaxed);
    crate::corpo_e_rosto::cust_mesclar_no_sistema(sys);
    let f: unsafe extern "C" fn(u64, u64, u64, u64) = std::mem::transmute(original_publicado(&ORIG_A47_INITIALIZE));
    f(sys, puppet, masc, functor);
}

pub(crate) unsafe extern "C" fn a47_uninitialize(sys: u64) {
    #[cfg(feature = "devtools")]
    A47_CICLO_CHAMADAS[1].fetch_add(1, Ordering::Relaxed);
    let f: unsafe extern "C" fn(u64) = std::mem::transmute(original_publicado(&ORIG_A47_UNINITIALIZE));
    f(sys);
}

/// Os 5 hooks do fluxo de edição, cada um uma vez: `Initialize`, `Uninitialize` e os 3 `Initialize*Option`.
pub(crate) unsafe fn a47_instalar_ciclo() -> (Option<bool>, Option<bool>, Vec<Option<bool>>) {
    let um = |vm: u64, prologo: u32, destino: *mut c_void, orig: &AtomicU64| {
        if orig.load(Ordering::Relaxed) != 0 { Some(true) } else { instalar_conferindo_prologo(vm, prologo, destino, orig) }
    };
    let i = um(A47_INITIALIZE.0, A47_INITIALIZE.1, a47_initialize as *mut c_void, &ORIG_A47_INITIALIZE);
    let u = um(A47_UNINITIALIZE.0, A47_UNINITIALIZE.1, a47_uninitialize as *mut c_void, &ORIG_A47_UNINITIALIZE);
    let destinos: [*mut c_void; 3] = [a47_init_app as *mut c_void, a47_init_morph as *mut c_void, a47_init_switcher as *mut c_void];
    let inits = (0..3).map(|k| um(A47_INIT[k].0, A47_INIT[k].1, destinos[k], &ORIG_A47_INIT[k])).collect();
    (i, u, inits)
}

/// Produção do `#47` com `customizations` nos `.xl` (`register::cust_tabela`): os hooks da mescla e da marcação.
pub(crate) unsafe fn install_a47_produto() {
    let (i, u, inits) = a47_instalar_ciclo();
    crate::veredito_dev!("[a47c] hooks da mescla: Initialize {i:?}, Uninitialize {u:?}, Initialize*Option {inits:?}");
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
        // Mesmo hash (o hash ignora caixa), texto que não é padrão: o `starts_with("regex:")` do original é sensível a caixa.
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
    /// (threshold em bits) nunca chega como ação: o alvo de eixo entra com ação não-setada, o default que o `Axis()` do original deixa.
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

    /// Alvo com todos os campos não-setados casa sem dado do evento, como `InputTarget.hpp:16-22` e `ComponentTarget.hpp:16-22`;
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

    /// `ResourceTarget` com `Path` ou `Type` de valor 0 (`Path(r"")`, `Type(n"None")`) não filtra, como na referência; com valor setado,
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

    /// Os adaptadores leem o valor 0 da tabela como não-setado, como a referência (`EntityTarget.hpp:25-109`, `InkWidgetTarget.hpp:16-24`):
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

    /// `DynamicEntityTarget.Tags([])`/`StaticEntityTarget.Tags([])` casam todo evento de ciclo, com ou sem entidade (`DynamicEntityTarget.hpp:18-19`),
    /// e o handler dispara se ALGUM alvo suportado casa (`CallbackSystemHandler.hpp:56-65`): `Tags([])` + `Tag(x)` que não casa dispara; só `Tag(x)`
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
