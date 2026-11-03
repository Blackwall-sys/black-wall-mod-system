//! Veículos: estado de direção (aceleração, derrapagem, ré, no chão) e a garagem.

#![allow(dead_code)]

use crate::jogo::sessao::auth_or;
use crate::jogo::sessao::game_instance;
use crate::jogo::sessao::sistema_de_script_ou_getter;
use crate::ganchos::original_publicado;
use crate::ganchos::instalar_conferindo_prologo;
use crate::diagnostico::C134E_CONGELADO;
use crate::tipos_do_motor::*;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use crate::rtti::{self, Registry};
use crate::register::*;

/// Lê `inst(VehicleObject*)` inteiro (1 syscall, `memoria::read_chunk`, que já falha sem crash em ponteiro inválido) e decodifica os 5 campos.
/// `None` em qualquer falha de leitura/ponteiro nulo/insano — nunca dereferencia sem checar.
pub(crate) unsafe fn vehicleobject_read_telemetry(inst: *mut c_void) -> Option<(bool, f32, f32, bool, f32)> {
    if inst.is_null() || !rtti::sane(inst) {
        return None;
    }
    let mut buf = [0u8; VEHICLEOBJECT_CHUNK_LEN];
    if !crate::memoria::read_chunk(inst as usize, &mut buf) {
        return None;
    }
    decode_vehicleobject_telemetry(&buf)
}

/// `BwmsVehicleObjectGetIsOnGround(veh: ref<VehicleObject>) -> Bool`
pub(crate) unsafe extern "C" fn tramp_vehicleobject_get_is_on_ground(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = vehicleobject_read_telemetry(inst).map(|(g, _, _, _, _)| g).unwrap_or(false);
    crate::log_dev!("[vehicleobject] BwmsVehicleObjectGetIsOnGround(veh={inst:p}) -> {val}");
    write_bool_ret(out, val);
}

/// `BwmsVehicleObjectGetAcceleration(veh: ref<VehicleObject>) -> Float`
pub(crate) unsafe extern "C" fn tramp_vehicleobject_get_acceleration(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = vehicleobject_read_telemetry(inst).map(|(_, a, _, _, _)| a).unwrap_or(0.0);
    crate::log_dev!("[vehicleobject] BwmsVehicleObjectGetAcceleration(veh={inst:p}) -> {val}");
    write_uint_ret(out, val.to_bits() as u64, 4);
}

/// `BwmsVehicleObjectGetDeceleration(veh: ref<VehicleObject>) -> Float`
pub(crate) unsafe extern "C" fn tramp_vehicleobject_get_deceleration(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = vehicleobject_read_telemetry(inst).map(|(_, _, d, _, _)| d).unwrap_or(0.0);
    crate::log_dev!("[vehicleobject] BwmsVehicleObjectGetDeceleration(veh={inst:p}) -> {val}");
    write_uint_ret(out, val.to_bits() as u64, 4);
}

/// `BwmsVehicleObjectGetIsReversing(veh: ref<VehicleObject>) -> Bool`
pub(crate) unsafe extern "C" fn tramp_vehicleobject_get_is_reversing(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = vehicleobject_read_telemetry(inst).map(|(_, _, _, r, _)| r).unwrap_or(false);
    crate::log_dev!("[vehicleobject] BwmsVehicleObjectGetIsReversing(veh={inst:p}) -> {val}");
    write_bool_ret(out, val);
}

/// `BwmsVehicleObjectGetBurnout(veh: ref<VehicleObject>) -> Float`
pub(crate) unsafe extern "C" fn tramp_vehicleobject_get_burnout(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let inst = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let val = vehicleobject_read_telemetry(inst).map(|(_, _, _, _, b)| b).unwrap_or(0.0);
    crate::log_dev!("[vehicleobject] BwmsVehicleObjectGetBurnout(veh={inst:p}) -> {val}");
    write_uint_ret(out, val.to_bits() as u64, 4);
}


// ---------------------------------------------------------------------------------------------
// `VehicleSystem.ToggleGarageVehicle` (2026-08-12): o endereço já foi RE'd e confirmado ao vivo via hook-probe OBSERVE-ONLY em
// 2026-07-29 (`jogo::mundo::VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM`, ABI
// documentada lá: `x0=this(VehicleSystem*), x1=&GarageVehicleID{TweakDBID recordID@0, CName
// name@8}(16B, by-ref), w2=bool enable, w0=bool ret`, ALTA confiança — 3 callers validados,
// prólogo real não-thunk) — mas até hoje só tinha sido usado pra OBSERVAR chamadas que o
// próprio jogo já fazia (hook passivo), nunca exposto como native genuinamente CHAMÁVEL a
// partir do redscript. Nenhum mod real conseguia disparar isso de propósito. Fecha essa
// lacuna: chama o MESMO endereço já confirmado, com a MESMA ABI já documentada — zero RE nova,
// só compõe o que já existia (critério "endereço já sentado em `selftest.rs`, nunca creditado
// como CALL direto").
//
// Risco reconhecido, documentado sem esconder: isto CHAMA código real do motor que ativa/
// desativa um veículo do garage do player — mutação genuína de estado do mundo, não leitura
// passiva. Mitigado por: (1) o endereço já está CONFIRMADO correto (não é candidato
// especulativo — foi observado disparando de verdade 2026-07-29); (2) a ABI já está
// documentada com alta confiança, não adivinhada agora; (3) mesma categoria de risco já aceita
// neste arquivo pra `tramp_componentwrapper_refresh_appearance` (chama
// `APPEARANCE_REBUILD_PROXY_VM` direto, também função real do motor, também já confirmada).
// Por isso NÃO tem smoke test automático que dispare sozinho — só compila e fica pronta pro próximo boot confirmar com um veículo real.
pub(crate) unsafe extern "C" fn tramp_vehiclesystem_toggle_garage_vehicle(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let record_id = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let enable = args.get(2).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let mut ret = false;
    if rtti::sane(sys) {
        let target = crate::rebase(crate::jogo::mundo::VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM);
        if !target.is_null() && crate::memoria::is_readable(target as *const c_void, 4) {
            // GarageVehicleID{TweakDBID recordID@0, CName name@8} — 16 bytes, by-ref (ABI
            // documentada em `jogo/mundo.rs`). `name` fica default-zerado: todo uso conhecido só
            // popula `recordID`.
            let mut garage_id = [0u8; 16];
            garage_id[0..8].copy_from_slice(&record_id.to_le_bytes());
            type ToggleGarageVehicleFn = unsafe extern "C" fn(*mut c_void, *const u8, u8) -> u8;
            let f: ToggleGarageVehicleFn = core::mem::transmute(target);
            let r = f(sys, garage_id.as_ptr(), enable as u8);
            ret = r != 0;
        }
    }
    crate::log_dev!(
        "[vehiclesystem] BwmsVehicleSystemToggleGarageVehicle(sys={sys:p}, recordID={record_id:#018x}, enable={enable}) -> {ret}"
    );
    write_bool_ret(out, ret);
}

pub(crate) unsafe fn register_vehiclesystem_toggle_garage_native(reg: &Registry, proto: *mut c_void) {
    let params = compose_params_from_types(reg, &["handle:VehicleSystem", "TweakDBID", "Bool"]);
    let ok = if params_compostos(params) {
        register_global_composed(
            reg, proto, params,
            "BwmsVehicleSystemToggleGarageVehicle", "BwmsVehicleSystemToggleGarageVehicle",
            tramp_vehiclesystem_toggle_garage_vehicle,
        )
    } else {
        crate::log_dev!("[reg] register_vehiclesystem_toggle_garage_native: compose_params_from_types falhou (handle:VehicleSystem, TweakDBID, Bool)");
        false
    };
    crate::log_dev!("[reg] register_all: cw-vehiclesystem-togglegarage (#72) BwmsVehicleSystemToggleGarageVehicle={ok}");
}

/// `vehicleGarageComponent::LoadSummonSetup(component*)`: relê o `Vehicle.summon_setup` (TweakDBID em `this+0x330`) para os campos de chamada do
/// veículo (`+0x33c..+0x3b0`); o motor só a chama ao anexar o componente (`0x102aa8790`). GOG `0x101730940`.
pub(crate) const CARREGAR_SETUP_DE_CHAMADA_VM: u64 = 0x1_02aa_79a8;

/// `GarageComponent.LoadSummonSetup()`, registrada na visita do validador ao `GarageComponent`, que existe no RTTI e o script
/// vanilla da 2.5 não declara (`ponte-vehiclesystem.reds`): o mod mexe no record e pede a releitura sem esperar o próximo anexo.
pub(crate) unsafe fn registrar_garagem(reg: &Registry) -> String {
    let Some(proto) = method_donor_proto(reg) else {
        return "protótipo doador não resolveu".into();
    };
    // O registro procura a classe pelo nome nativo do RTTI: pelo nome de script (`GarageComponent`) não acha, a native não entra e o bind reprova
    // a classe declarada na ponte (SIGTRAP no boot).
    let m = register_method(reg, "vehicleGarageComponent", proto, "LoadSummonSetup", "LoadSummonSetup", tramp_carregar_setup_de_chamada, false);
    format!("vehicleGarageComponent (GarageComponent): classe=VANILLA (sem forge) LoadSummonSetup={m}")
}

pub(crate) unsafe extern "C" fn tramp_carregar_setup_de_chamada(c: *mut c_void, _f: *mut c_void, _out: *mut c_void, _rt: i64) {
    if !rtti::sane(c) || !crate::memoria::is_readable(c as *const c_void, 0x3b8) {
        return;
    }
    let f: extern "C" fn(*mut c_void) = std::mem::transmute(crate::rebase(CARREGAR_SETUP_DE_CHAMADA_VM));
    f(c);
}


pub static VEHICLE_LIGHT_ARMADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `Vehicle/ToggleAuxLights` com o veículo dono do controller, o `on` e o tipo de luz, só com dono.
pub unsafe fn disparar_luz_de_veiculo(controller: *mut c_void, ligado: bool, tipo: u32) -> usize {
    let dono = dono_do_componente(controller);
    if dono == 0 {
        return 0;
    }
    let Some(arg) = make_vehiclelightcontrolevent_arg(ligado, tipo as i32, dono) else { return 0 };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args("Vehicle/ToggleAuxLights", &[arg]);
    if let Ok(mut states) = VEHICLELIGHT_STATES.lock() {
        states.retain(|e| e.0 as u64 != inst);
    }
    let mut h = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut h);
    enviados
}

/// `VehicleLightControlEvent extends EntityLifecycleEvent` — classes de evento-alvo
/// (2026-07-24). Declaração: `public native class VehicleLightControlEvent extends
/// EntityLifecycleEvent { IsEnabled()->Bool; IsLightType(lightType: vehicleELightType)->Bool }`.
/// `vehicleELightType` é um enum REAL vanilla (`orphans.script:401`, pré-primado, mesma categoria
/// de `EInputKey`). `IsLightType` lê o param via `read_params_consuming` — MESMO padrão já
/// provado ao vivo em `CallbackSystemHandler.SetRunMode`/`SetLifetime` (enum como argumento de
/// método, não só retorno). Estado: (evento, ligado, tipo de luz, veículo dono do controller).
pub(crate) static VEHICLELIGHT_STATES: Mutex<Vec<(usize, bool, i32, usize)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_vlce_is_enabled(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = VEHICLELIGHT_STATES.lock() {
        if let Some((_, en, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *en;
        }
    }
    write_bool_ret(out, v);
}

/// `IsLightType(lightType)`: como a referência, algum bit em comum entre o tipo do evento e o perguntado.
pub(crate) unsafe extern "C" fn tramp_vlce_is_light_type(c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let queried = args.first().map(|(v, _)| *v as i32).unwrap_or(-1);
    let mut v = false;
    if let Ok(states) = VEHICLELIGHT_STATES.lock() {
        if let Some((_, _, lt, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *lt as u32 & queried as u32 != 0;
        }
    }
    crate::log_dev!("[vlce] VehicleLightControlEvent.IsLightType({queried}) on {c:p} -> {v}");
    write_bool_ret(out, v);
}

pub unsafe fn register_vehiclelightcontrolevent(reg: &Registry) -> String {
    let ele = reg.class_by_name("EntityLifecycleEvent");
    if !rtti::sane(ele) {
        return "[reg] register_vehiclelightcontrolevent: 'EntityLifecycleEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "VehicleLightControlEvent", "gameuiInGameMenuGameController", ele);
    if forged.is_null() {
        return "[reg] register_vehiclelightcontrolevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_vehiclelightcontrolevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "VehicleLightControlEvent", proto, "IsEnabled", "IsEnabled", tramp_vlce_is_enabled, false);
    let m2 = register_method(reg, "VehicleLightControlEvent", proto, "IsLightType", "IsLightType", tramp_vlce_is_light_type, false);
    format!("[reg] VehicleLightControlEvent: classe=OK (INSTANCIÁVEL, parent=EntityLifecycleEvent real) IsEnabled={m1} IsLightType={m2}")
}

/// Constrói um `VehicleLightControlEvent` com o veículo dono do controller (`GetEntity` do evento), como o `VehicleLightControlHook` da referência.
pub unsafe fn make_vehiclelightcontrolevent_arg(enabled: bool, light_type: i32, entidade: usize) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "VehicleLightControlEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = VEHICLELIGHT_STATES.lock() {
        // Um evento anterior no mesmo endereço que nunca saiu do estado faria os getters acharem o estado velho.
        states.retain(|(p, ..)| *p != obj as usize);
        states.push((obj as usize, enabled, light_type, entidade));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

// ---------------------------------------------------------------------------------------------
// `VehicleObject` (2026-08-12) — leitura pura de campo, ZERO RawFunc/AddressLib novo. Offsets Windows
// CONFIRMADOS do layout de `vehicleBaseObject` (não chutados):
//   isOnGround   : bool  @ +0x25C
//   acceleration : float @ +0x264
//   deceleration : float @ +0x268
//   isReversing  : bool  @ +0x2A3
//   burnout      : float @ +0x2BC (Mac 2.5: +0x2C4)
// Todos campo de DADO (não vtable/RawFunc) — regra já estabelecida no projeto (reconfirmada
// hoje mesmo pro MappinSystem e pro WeatherSystem): offset de campo porta 1:1
// Windows->Mac, sem o shift Itanium (+0x08) que só afeta slots de VTABLE/método.
//
// Sem `@addField(VehicleObject) public native let X`: `native let` reivindica uma property RTTI
// que o motor não registra para esses campos, então declarar o campo como `native let`
// arriscaria o crash de bind do CMesh (validação de ancestralidade em RUNTIME, não
// compile-time). Padrão seguro já estabelecido no projeto: GETTER via FUNÇÃO nativa lendo o
// offset real, nunca `@addField...native let` direto — mesma disciplina já aplicada hoje em
// WeatherSystem e ComponentWrapper.
//
// `VehicleObject` (alias Windows de `vehicleBaseObject`) confirmado vanilla real e
// redscript-visível: é o tipo de retorno de natives vanilla já usadas em scripts reais do jogo
// (`GameObject.GetMountedVehicle() -> wref<VehicleObject>`, `core/gameplay/vehicles.script:1490`).
pub(crate) const VEHICLEOBJECT_IS_ON_GROUND_OFF: usize = 0x25C;

pub(crate) const VEHICLEOBJECT_ACCELERATION_OFF: usize = 0x264;

// 2.5: entram 8 B entre +0x264 e +0x2A8 (o 1.0f do construtor vai de +0x2A8 a +0x2B0); se antes ou depois de `deceleration`/`isReversing`
// é [não medido]: o construtor zera o bloco inteiro e nenhuma função com símbolo lê esses dois. Só a leitura ao vivo decide.
pub(crate) const VEHICLEOBJECT_DECELERATION_OFF: usize = 0x268;

pub(crate) const VEHICLEOBJECT_IS_REVERSING_OFF: usize = 0x2A3;

// 2.5: o construtor zera o u32 de +0x2C4 e grava o par de floats em +0x2C8 (na 2.31, +0x2BC e +0x2C0).
pub(crate) const VEHICLEOBJECT_BURNOUT_OFF: usize = 0x2C4;

pub(crate) const VEHICLEOBJECT_CHUNK_LEN: usize = 0x2C8; // cobre até burnout+4, sem folga extra

/// Núcleo PURO (testável offline, sem memória real): decodifica os 5 campos de telemetria a
/// partir de um buffer que espelha os primeiros `VEHICLEOBJECT_CHUNK_LEN` bytes do
/// `VehicleObject*` real. Espelha byte-a-byte os offsets confirmados acima.
pub(crate) fn decode_vehicleobject_telemetry(buf: &[u8]) -> Option<(bool, f32, f32, bool, f32)> {
    if buf.len() < VEHICLEOBJECT_CHUNK_LEN {
        return None;
    }
    let is_on_ground = buf[VEHICLEOBJECT_IS_ON_GROUND_OFF] != 0;
    let acceleration = f32::from_le_bytes(buf[VEHICLEOBJECT_ACCELERATION_OFF..VEHICLEOBJECT_ACCELERATION_OFF + 4].try_into().ok()?);
    let deceleration = f32::from_le_bytes(buf[VEHICLEOBJECT_DECELERATION_OFF..VEHICLEOBJECT_DECELERATION_OFF + 4].try_into().ok()?);
    let is_reversing = buf[VEHICLEOBJECT_IS_REVERSING_OFF] != 0;
    let burnout = f32::from_le_bytes(buf[VEHICLEOBJECT_BURNOUT_OFF..VEHICLEOBJECT_BURNOUT_OFF + 4].try_into().ok()?);
    Some((is_on_ground, acceleration, deceleration, is_reversing, burnout))
}

pub(crate) unsafe fn register_vehicleobject_telemetry_natives(reg: &Registry, proto: *mut c_void) {
    let targets: [(&str, NativeHandler); 5] = [
        ("BwmsVehicleObjectGetIsOnGround", tramp_vehicleobject_get_is_on_ground),
        ("BwmsVehicleObjectGetAcceleration", tramp_vehicleobject_get_acceleration),
        ("BwmsVehicleObjectGetDeceleration", tramp_vehicleobject_get_deceleration),
        ("BwmsVehicleObjectGetIsReversing", tramp_vehicleobject_get_is_reversing),
        ("BwmsVehicleObjectGetBurnout", tramp_vehicleobject_get_burnout),
    ];
    let mut summary = String::new();
    for (name, handler) in targets {
        let params = compose_params_from_types(reg, &["handle:VehicleObject"]);
        let ok = if params_compostos(params) {
            register_global_composed(reg, proto, params, name, name, handler)
        } else {
            crate::log_dev!("[reg] register_vehicleobject_telemetry_natives: compose_params_from_types falhou (handle:VehicleObject) [{name}]");
            false
        };
        summary.push_str(&format!(" {name}={ok}"));
    }
    crate::log_dev!("[reg] register_all: cw-vehicleobject-telemetry (#71){summary}");
}


// RESOLVIDO 2026-07-28 (RE estática offline dedicada, mesma técnica do IComponent::Toggle):
// classe real = `vehicleController` (redscript-src/orphans.script:37307-37324, extends GameComponent),
// método real = `ToggleLights(on, opt lightType, opt inTime, opt lerpCurve, opt loop)` — NÃO
// "VehicleController::ToggleAuxLights" (esse nome/classe não existem nesta build). Cadeia de 5 elos:
// __cstring com os 8 nomes nativos na ordem exata da declaração + registrador de classe (0x102b3b504)
// + bloco lazy-init do 7º método (0x102b3bb98, índice 6 = objeto @0x108815530) + CClassFunction ctor
// (0x10109b8a4, mesma infra RTTI já confirmada) + handler (0x102b3ea78, lê 5 params mas só repassa
// 2 — inTime/lerpCurve/loop são lidos e DESCARTADOS nesta build) -> `bl 0x102b3d974(x0=this,w1=on,w2=lightType)`.
// 0x102b3d974 tem prólogo real (SUB SP+STP, não thunk), só 2 callers no binário (o handler + 1 caller
// C++ interno). ABI confirmada bate com o handler Rust já codado (ctrl*, u8 enable, u32 light_type).
pub(crate) const VEHICLE_AUX_VM: u64 = 0x1_02b3_d974;

/// Original do `vehicleController::ToggleLights`, publicado pelo `instalar_com_threads_paradas` (`garantir_hook_de_luz`).
pub(crate) static ORIG_VEHICLE_AUX: AtomicU64 = AtomicU64::new(0);

pub(crate) const VEHICLE_AUX_PROLOGO: u32 = 0xd101_03ff;

pub(crate) unsafe extern "C" fn vehicle_aux_lights_handler(ctrl: *mut c_void, enable: u8, light_type: u32) {
    let f: unsafe extern "C" fn(*mut c_void, u8, u32) = std::mem::transmute(original_publicado(&ORIG_VEHICLE_AUX));
    f(ctrl, enable, light_type);
    if !ctrl.is_null() && crate::veiculos::VEHICLE_LIGHT_ARMADO.load(Ordering::Relaxed) && !C134E_CONGELADO.load(Ordering::Relaxed) {
        crate::veiculos::disparar_luz_de_veiculo(ctrl, enable != 0, light_type);
    }
}

/// Instala o hook do `vehicleController::ToggleLights` no `RegisterCallback` de `Vehicle/ToggleAuxLights`, de novo depois de uma
/// desinstalação.
pub(crate) unsafe fn garantir_hook_de_luz() {
    if let Some(ok) = instalar_conferindo_prologo(VEHICLE_AUX_VM, VEHICLE_AUX_PROLOGO, vehicle_aux_lights_handler as *mut c_void, &ORIG_VEHICLE_AUX) {
        crate::veredito_dev!("[luz] hook do vehicleController::ToggleLights pelo RegisterCallback: {ok}");
    }
}


/// Alterna o modo de invocação do veículo (chama seu carro), via `ToggleSummonMode`.
pub unsafe fn summon(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match game_instance(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let vs = sistema_de_script_ou_getter(reg, owner, gi, "gameVehicleSystem", "GetVehicleSystem");
    if !rtti::sane(vs) {
        crate::log_dev!("[summon] VehicleSystem inacessível");
        return false;
    }
    let e = match rtti::resolve_any(reg, &["gameVehicleSystem"], "ToggleSummonMode") {
        Some(g) => g,
        None => {
            crate::log_dev!("[summon] ToggleSummonMode não resolvido");
            return false;
        }
    };
    rtti::call_func(&e, vs, &[]);
    crate::log_dev!("[summon] ToggleSummonMode enviado");
    true
}

