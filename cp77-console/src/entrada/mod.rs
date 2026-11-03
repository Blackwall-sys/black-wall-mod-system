//! Entrada: eventos de teclado, eixo e caractere vindos do jogador.

#![allow(dead_code)]

use crate::tipos_do_motor::*;
use std::ffi::c_void;
use std::sync::Mutex;
use crate::rtti::{self, Registry};
use crate::register::*;

pub(crate) unsafe extern "C" fn tramp_is_character_input(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let key = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    let ok = is_character_input_key(key);
    crate::log_dev!("[widgetinput] BwmsIsCharacterInput({key}) -> {ok}");
    write_bool_ret(out, ok);
}

pub(crate) unsafe extern "C" fn tramp_to_character_key(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let key = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    let shift = args.get(1).map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let s = to_character_key(key, shift);
    crate::log_dev!("[widgetinput] BwmsToCharacterKey({key}, shift={shift}) -> '{s}'");
    write_cstring_inline_ret(out, &s);
}

/// Sentinela `EInputKey.IK_None` — "sem filtro de key" na fonte real. Enum VANILLA do jogo
/// (`orphans.script:3223`); espelhado aqui pelo mesmo motivo de `IACT_NONE`, e com o
/// mesmo tipo: as duas enums são `int32_t`.
pub(crate) const IK_NONE: i32 = 0;

/// TAG de qual das 2 factories construiu o alvo (mesmo padrão de
/// `ComponentTargetCriterion`/`EntityTargetCriterion`). **Não é conveniência — é fidelidade**: no
/// `InputTarget`, `action` e `threshold` são CAMPOS
/// SEPARADOS da struct, e `Axis()` só escreve `key`+`threshold`, deixando `action` no default
/// `IACT_None` (= wildcard). A tupla daqui compartilha um slot só entre os dois, então sem a tag
/// o `threshold` de um `.Axis(axis, 0.5)` (bits `0x3F000000` = 1056964608) seria lido como se
/// fosse um `action` — comparação de lixo contra `EInputAction`, filtrando fora eventos legítimos.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum InputTargetCriterion {
    Key,
    Axis,
}

/// `InputTarget.Key`/`.Axis` — `(ptr, criterion, key, action_ou_threshold_bits)`: o 4º campo é
/// `action` (`EInputAction`) quando `criterion == Key` e `threshold` (Float em bits, via
/// `f32::from_bits`) quando `Axis`. A tag diz qual — ver `InputTargetCriterion`.
pub(crate) static INPUTTARGET_STATES: Mutex<Vec<(usize, InputTargetCriterion, i64, i64)>> = Mutex::new(Vec::new());

/// `InputTarget.Key(key: EInputKey, opt action: EInputAction) -> ref<InputTarget>` — 2 args,
/// AMBOS enum: `EInputKey` (enum vanilla REAL, `orphans.script:3223`, pré-primado) e
/// `EInputAction` (enum de SCRIPT nosso, `ponte-callbacksystem.reds`, já usado como TIPO DE
/// RETORNO em `KeyInputEvent.GetAction`). Params-tipo-enum são lidos por `ertti==5` em
/// `rtti::read_params_inner_ext` — categoria JÁ PROVADA ao vivo como ARGUMENTO (não só retorno)
/// por `VehicleLightControlEvent.IsLightType(lightType: vehicleELightType)`/
/// `CallbackSystemHandler.SetRunMode(runMode: CallbackRunMode)`.
///
/// `action` é `opt` SEM valor-default explícito no `.reds` REAL (`Callback/Targets/InputTarget.
/// reds:2`) — mas como é `native func` (não script-implementado), o compilador redscript AINDA
/// bakeia um valor CONCRETO no bytecode do call-site quando o argumento é omitido (mesmo padrão
/// já confirmado neste arquivo: `CallbackSystem.UnregisterCallback`'s `opt function: CName` lê
/// hash=`0` quando omitido — o CName-vazio DEFAULT que o compilador injeta, não um "buraco" no
/// array). A gotcha documentada no `CLAUDE.md` ("`callg` não preenche `opt`") é sobre o CANAL DE
/// TESTE `callg` (que não passa pelo compilador redscript, logo não bakeia defaults) — NÃO sobre
/// bytecode compilado real de um `.reds` de mod. Logo `args.get(1)` deve vir preenchido (com o
/// default do tipo — 1ª constante de `EInputAction`, que é `IACT_None = 0`) mesmo quando o
/// `.reds` chamador omite `action`; `.unwrap_or(0)` é mantido por defensividade (MESMO estilo de
/// todo `opt` já neste arquivo — `tramp_cbs_unregister_callback` etc.), não porque o array venha
/// genuinamente mais curto no caminho feliz. Esse default cai EXATAMENTE no sentinela de "sem
/// filtro de action" do `InputTarget`, então omitir `action` no `.reds`
/// dá wildcard. (Uma versão anterior desta nota dizia "`Press=0`";
/// ficou obsoleta em 2026-08-05, quando o enum foi corrigido pros valores reais — `IACT_None=0`,
/// `IACT_Press=1`.)
pub(crate) unsafe extern "C" fn tramp_it_key(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let key = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    // A declaração do jogo (2.5) troca a ação por `opt threshold: Float`: sem filtro de ação, como o `IACT_None`.
    let action = match args.get(1) {
        Some(&(v, tc)) if tc != crate::cname::cname("Float") => v as i32,
        _ => 0,
    };
    let obj = crate::entidades::alvo_novo("InputTarget", |o| {
        if let Ok(mut states) = INPUTTARGET_STATES.lock() {
            states.push((o, InputTargetCriterion::Key, key as i64, action as i64));
        }
    });
    crate::log_dev!(
        "[it] InputTarget.Key(key={key}, action={action}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))"
    );
    rtti::make_handle(out, obj);
}

/// `InputTarget.Axis(axis: EInputKey, opt threshold: Float) -> ref<InputTarget>` — `axis` mesmo
/// tratamento de `key` acima; `threshold` (Float, `opt` SEM default explícito na fonte real) lido
/// via `f32::from_bits` — MESMO padrão já provado por `BwmsEchoF`/`AxisInputEvent.GetValue`.
/// Mesma nota sobre `opt` sem default de `tramp_it_key` aplica aqui (o compilador bakeia `0.0`
/// quando omitido).
pub(crate) unsafe extern "C" fn tramp_it_axis(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let axis = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    let threshold_raw = args.get(1).map(|(v, _)| *v).unwrap_or(0);
    let threshold = f32::from_bits(threshold_raw as u32);
    let obj = crate::entidades::alvo_novo("InputTarget", |o| {
        if let Ok(mut states) = INPUTTARGET_STATES.lock() {
            states.push((o, InputTargetCriterion::Axis, axis as i64, threshold_raw as i64));
        }
    });
    crate::log_dev!(
        "[it] InputTarget.Axis(axis={axis}, threshold={threshold}) -> {obj:p} (fábrica estática, mesmo mecanismo PROVADO 2026-08-05 (ver ComponentTarget))"
    );
    rtti::make_handle(out, obj);
}

/// Forja `InputTarget extends CallbackSystemTarget` + registra `Key`/`Axis`.
pub unsafe fn register_inputtarget(reg: &Registry) -> String {
    let cst = reg.class_by_name("CallbackSystemTarget");
    if !rtti::sane(cst) {
        return "[reg] register_inputtarget: 'CallbackSystemTarget' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "InputTarget", "gameuiInGameMenuGameController", cst);
    if forged.is_null() {
        return "[reg] register_inputtarget: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_inputtarget: classe=OK mas sem protótipo — Key/Axis NÃO registrados".into(),
    };
    let m1 = register_method(reg, "InputTarget", proto, "Key", "Key", tramp_it_key, true);
    let m2 = register_method(reg, "InputTarget", proto, "Axis", "Axis", tramp_it_axis, true);
    format!(
        "[reg] InputTarget: classe=OK (INSTANCIÁVEL, parent=CallbackSystemTarget real) Key={m1} Axis={m2} \
— ambos params-enum/Float (categoria já provada); is_static+construct+handle NUMA CLASSE FORJADA: PROVADO 2026-08-05"
    )
}

/// `KeyInputEvent extends CallbackSystemEvent` — classes de evento-alvo/input cru com nome real,
/// RETRY 2026-07-18 (sessão `handle-ctor-re`), 3ª tentativa (2 anteriores em 2026-07-13/15
/// crasharam — ver histórico em `ponte-callbacksystem.reds`). Diferença desta vez: `parent` é a
/// classe `CallbackSystemEvent` REAL, forjada nativa (`register_type_min`, chamada acima) — não
/// mais um parent não-nativo (causa da Tentativa 1) nem `IScriptable` direto pulando a hierarquia
/// real (Tentativa 2). Forjada via `register_type_instantiable_with_parent` (o forge ROBUSTO, com
/// `fix_embedded_allocator_vtables` — o fix que fechou o handler de callback/`ScriptableService`
/// hoje mais cedo). Campos reais (`action: EInputAction`, `key: EInputKey`, `state: KeyboardState`)
/// NÃO são lidos da memória crua do objeto C++ (RE de layout fora de escopo) — mantidos num
/// registry Rust por-instância (`KEYINPUT_STATES`, mesmo padrão de `HANDLER_STATES`),
/// populado por `make_keyinputevent_arg` (a entrega real de `Input/Key`/
/// `Input/KeyUp`, `disparar_evento_de_tecla` chamado do `cp77_tick`), por
/// `make_axisinputevent_arg_com_tecla` e pela native de teste `BwmsMakeTestKeyInputEvent`, e lido
/// pelos getters.
pub(crate) static KEYINPUT_STATES: Mutex<Vec<(usize, i32, i32, bool, bool, bool)>> = Mutex::new(Vec::new());

/// Valores REAIS de `EInputAction` (`ponte-callbacksystem.reds`, corrigidos em 2026-08-05). Espelhados aqui porque o lado Rust
/// grava/compara esse enum como escalar cru (`KEYINPUT_STATES`/`INPUTTARGET_STATES`) — sem eles o
/// código volta a carregar literais mágicos, exatamente o que produziu o bug do `action` fixo.
/// `IACT_None` é o SENTINELA de "sem filtro" no `InputTarget`.
pub(crate) const IACT_NONE: i32 = 0;

pub(crate) const IACT_PRESS: i32 = 1;

pub(crate) const IACT_RELEASE: i32 = 2;

#[cfg(feature = "devtools")]
pub(crate) const IACT_AXIS: i32 = 3;

pub(crate) unsafe extern "C" fn tramp_kie_get_action(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0i32;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, a, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *a;
        }
    }
    crate::log_dev!("[kie] KeyInputEvent.GetAction() on {c:p} -> {v}");
    write_uint_ret(out, v as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_kie_get_key(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0i32;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, k, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *k;
        }
    }
    crate::log_dev!("[kie] KeyInputEvent.GetKey() on {c:p} -> {v}");
    write_uint_ret(out, v as u32 as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_kie_is_shift_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, s, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *s;
        }
    }
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_kie_is_control_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, _, ctl, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *ctl;
        }
    }
    write_bool_ret(out, v);
}

pub(crate) unsafe extern "C" fn tramp_kie_is_alt_down(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = false;
    if let Ok(states) = KEYINPUT_STATES.lock() {
        if let Some((_, _, _, _, _, alt)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *alt;
        }
    }
    write_bool_ret(out, v);
}

pub unsafe fn register_keyinputevent(reg: &Registry) -> String {
    let cse = reg.class_by_name("CallbackSystemEvent");
    if !rtti::sane(cse) {
        return "[reg] register_keyinputevent: 'CallbackSystemEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "KeyInputEvent", "gameuiInGameMenuGameController", cse);
    if forged.is_null() {
        return "[reg] register_keyinputevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_keyinputevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    // GetAction()->EInputAction (2026-07-24): `EInputAction` agora existe como enum de SCRIPT
    // declarado em `ponte-callbacksystem.reds` — resolve no scc, testado offline (scratch-compile,
    // 0 erro). `tramp_kie_get_action` já lia `KEYINPUT_STATES` desde 2026-07-18, só faltava o
    // `register_method`.
    let m1 = register_method(reg, "KeyInputEvent", proto, "GetAction", "GetAction", tramp_kie_get_action, false);
    let m2 = register_method(reg, "KeyInputEvent", proto, "GetKey", "GetKey", tramp_kie_get_key, false);
    let m3 = register_method(reg, "KeyInputEvent", proto, "IsShiftDown", "IsShiftDown", tramp_kie_is_shift_down, false);
    let m4 = register_method(reg, "KeyInputEvent", proto, "IsControlDown", "IsControlDown", tramp_kie_is_control_down, false);
    let m5 = register_method(reg, "KeyInputEvent", proto, "IsAltDown", "IsAltDown", tramp_kie_is_alt_down, false);
    format!(
        "[reg] KeyInputEvent: classe=OK (INSTANCIÁVEL, parent=CallbackSystemEvent real) GetAction={m1} GetKey={m2} IsShiftDown={m3} IsControlDown={m4} IsAltDown={m5}"
    )
}

/// Mapeia o keycode do AppKit (`NSEvent.keyCode`, hardware scancode do macOS) + o caractere
/// capturado (`charactersIgnoringModifiers`) pro valor NUMÉRICO REAL do `EInputKey` do motor
/// (`cp77-symbols/redscript-src/orphans.script:3223`). **Achado-chave:** `EInputKey` bate EXATO
/// com os códigos Win32 `VK_*` — `IK_A=65`=ASCII `'A'`, `IK_0=48`=ASCII `'0'`, `IK_Space=32`=ASCII
/// `' '` — ou seja, pra letras/dígitos/espaço o valor de `EInputKey` É o código ASCII do
/// caractere capturado, SEM precisar de tabela. Pra teclas SEM caractere (setas/função/etc.),
/// tabela pequena pelo keyCode macOS (RE manual, valores confirmados contra o enum real). NÃO
/// exaustiva (cobre letras+dígitos+espaço+setas+alguns extras) — suficiente pro `proof_needed`
/// do input cru com nome real ("mod recebe `ref<KeyInputEvent>` e `GetKey()` bate com a tecla
/// real"); `0` (`IK_None`) se a tecla não mapear — não bloqueia, só não identifica.
pub fn map_macos_keycode_to_einputkey(kc: i32, ch: Option<char>) -> i32 {
    if let Some(c) = ch {
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return c as i32;
        }
        if c.is_ascii_lowercase() {
            return c.to_ascii_uppercase() as i32;
        }
        if c == ' ' {
            return 32; // IK_Space
        }
    }
    match kc {
        123 => 37,  // Left     -> IK_Left
        126 => 38,  // Up       -> IK_Up
        124 => 39,  // Right    -> IK_Right
        125 => 40,  // Down     -> IK_Down
        36 => 13,   // Return   -> IK_Enter
        48 => 9,    // Tab      -> IK_Tab
        51 => 8,    // Delete   -> IK_Backspace
        53 => 27,   // Escape   -> IK_Escape
        49 => 32,   // Space (fallback se `ch` vier vazio) -> IK_Space
        122 => 112, // F1       -> IK_F1
        120 => 113, // F2       -> IK_F2
        99 => 114,  // F3       -> IK_F3
        118 => 115, // F4       -> IK_F4
        _ => 0,     // IK_None — não mapeado
    }
}

/// Dispara `evento` com um `KeyInputEvent` novo; devolve quantos ouvintes receberam. Depois do disparo o estado sai do registro e o handle do evento é
/// solto (o `call_func` só copia o par; o +1 é nosso): sem isso, cada tecla com ouvinte deixava 1 evento vivo e 1 entrada no registro.
pub(crate) unsafe fn disparar_evento_de_tecla(evento: &str, action: i32, key: i32, shift: bool, control: bool, alt: bool) -> usize {
    let Some(arg) = make_keyinputevent_arg(action, key, shift, control, alt) else {
        return 0;
    };
    let (inst, refc) = match &arg {
        rtti::Arg::Handle(i, r) => (*i as u64, *r as u64),
        _ => (0, 0),
    };
    let enviados = fire_event_args(evento, &[arg]);
    if let Ok(mut states) = KEYINPUT_STATES.lock() {
        states.retain(|s| s.0 as u64 != inst);
    }
    let mut h = [inst, refc];
    crate::jogo::objetos::soltar_handle(&mut h);
    enviados
}

/// Input cru com nome real (2026-07-18, sessão `handle-ctor-re`, continuação direta das
/// classes de evento-alvo): constrói um `KeyInputEvent` REAL (mesma técnica provada —
/// `rtti::new_object`+`make_handle`) a partir de um valor JÁ MAPEADO de `EInputKey` +
/// modificadores capturados do AppKit, e devolve um `rtti::Arg::Handle` pronto pra
/// `fire_event_args`/`call_func` despachar POR REFERÊNCIA (não mais um escalar Int32 cru).
/// `None` se a construção falhar (RTTI não pronto etc.) — o caller descarta a tecla, não crasha.
pub unsafe fn make_keyinputevent_arg(
    action: i32,
    key: i32,
    shift: bool,
    control: bool,
    alt: bool,
) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "KeyInputEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = KEYINPUT_STATES.lock() {
        // `action` = o `EInputAction` REAL do evento, informado pelo caller (`IACT_PRESS` pra
        // "Input/Key", `IACT_RELEASE` pra "Input/KeyUp").
        //
        // **Corrige um bug semântico real (2026-08-21)**: até aqui gravava-se o literal `2` FIXO,
        // justificado por 2 premissas que CADUCARAM desde 2026-07-18 — (a) "`GetAction()` não é
        // exposto" (hoje É declarado, `ponte-callbacksystem.reds:222`) e (b) "`EInputAction` não
        // é resolvível" (hoje é enum de script com os valores REAIS, corrigidos em 2026-08-05:
        // `IACT_None=0/IACT_Press=1/IACT_Release=2/IACT_Axis=3`). Com `2` fixo, uma tecla
        // DESCENDO reportava "Release": todo mod que checasse `action == EInputAction.IACT_Press`
        // (o idioma normal dos mods) silenciosamente NUNCA disparava, e `IACT_Release`
        // disparava 2x (descida + subida). O valor não era inofensivo — era uma mentira.
        states.push((obj as usize, action, key, shift, control, alt));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// `AxisInputEvent extends KeyInputEvent` — classes de evento-alvo (2026-07-24). Fonte real:
/// `public native class AxisInputEvent extends KeyInputEvent { GetValue()->Float;
/// GetMouseX()->Uint32; GetMouseY()->Uint32 }`. MESMA
/// receita 100% segura de `KeyInputEvent` — parent JÁ forjado (não `CallbackSystemEvent` direto,
/// `AxisInputEvent` estende `KeyInputEvent` na fonte real). Zero tipo novo: `Float`/`Uint32` já
/// usados em dezenas de natives deste projeto.
pub(crate) static AXISINPUT_STATES: Mutex<Vec<(usize, f32, u32, u32)>> = Mutex::new(Vec::new());

pub(crate) unsafe extern "C" fn tramp_aie_get_value(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0.0f32;
    if let Ok(states) = AXISINPUT_STATES.lock() {
        if let Some((_, val, ..)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *val;
        }
    }
    core::ptr::write_unaligned(out as *mut u32, v.to_bits());
}

pub(crate) unsafe extern "C" fn tramp_aie_get_mouse_x(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u32;
    if let Ok(states) = AXISINPUT_STATES.lock() {
        if let Some((_, _, x, _)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *x;
        }
    }
    write_uint_ret(out, v as u64, 4);
}

pub(crate) unsafe extern "C" fn tramp_aie_get_mouse_y(c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let mut v = 0u32;
    if let Ok(states) = AXISINPUT_STATES.lock() {
        if let Some((_, _, _, y)) = states.iter().rev().find(|(p, ..)| *p == c as usize) {
            v = *y;
        }
    }
    write_uint_ret(out, v as u64, 4);
}

pub unsafe fn register_axisinputevent(reg: &Registry) -> String {
    let kie = reg.class_by_name("KeyInputEvent");
    if !rtti::sane(kie) {
        return "[reg] register_axisinputevent: 'KeyInputEvent' não resolveu (parent ausente)".into();
    }
    let forged = register_type_instantiable_with_parent(reg, "AxisInputEvent", "gameuiInGameMenuGameController", kie);
    if forged.is_null() {
        return "[reg] register_axisinputevent: forja da classe FALHOU".into();
    }
    let proto = match method_donor_proto(reg) {
        Some(p) => p,
        None => return "[reg] register_axisinputevent: classe=OK mas sem protótipo — métodos NÃO registrados".into(),
    };
    let m1 = register_method(reg, "AxisInputEvent", proto, "GetValue", "GetValue", tramp_aie_get_value, false);
    let m2 = register_method(reg, "AxisInputEvent", proto, "GetMouseX", "GetMouseX", tramp_aie_get_mouse_x, false);
    let m3 = register_method(reg, "AxisInputEvent", proto, "GetMouseY", "GetMouseY", tramp_aie_get_mouse_y, false);
    format!("[reg] AxisInputEvent: classe=OK (INSTANCIÁVEL, parent=KeyInputEvent real) GetValue={m1} GetMouseX={m2} GetMouseY={m3}")
}

/// Constrói um `AxisInputEvent` REAL (mesma técnica de `make_keyinputevent_arg`).
#[cfg(feature = "devtools")]
pub unsafe fn make_axisinputevent_arg(value: f32, mouse_x: u32, mouse_y: u32) -> Option<rtti::Arg> {
    let reg = rtti::Registry::obtain()?;
    let obj = rtti::new_object(&reg, "AxisInputEvent");
    if obj.is_null() {
        return None;
    }
    if let Ok(mut states) = AXISINPUT_STATES.lock() {
        states.push((obj as usize, value, mouse_x, mouse_y));
    }
    let mut buf = [0u8; 16];
    rtti::make_handle(buf.as_mut_ptr() as *mut c_void, obj);
    let inst = (buf.as_ptr() as *const u64).read_unaligned() as *mut c_void;
    let refc = (buf.as_ptr().add(8) as *const u64).read_unaligned() as *mut c_void;
    Some(rtti::Arg::Handle(inst, refc))
}

/// `AxisInputEvent` com a tecla do eixo: `GetKey`/`GetAction` saem de `KEYINPUT_STATES` (ação `IACT_AXIS`), o valor de `AXISINPUT_STATES`.
/// Pra fila do `ProcessInputEvents` (2026-09-11).
#[cfg(feature = "devtools")]
pub unsafe fn make_axisinputevent_arg_com_tecla(key: i32, value: f32, mouse_x: u32, mouse_y: u32) -> Option<rtti::Arg> {
    let arg = make_axisinputevent_arg(value, mouse_x, mouse_y)?;
    if let rtti::Arg::Handle(inst, _) = &arg {
        if let Ok(mut states) = KEYINPUT_STATES.lock() {
            states.push((*inst as usize, IACT_AXIS, key, false, false, false));
        }
    }
    Some(arg)
}

/// Mantém só os últimos 1024 estados de evento de input. A fila do motor gera dezenas de eventos por segundo e o objeto do evento é
/// temporário; a busca dos getters vai do mais novo pro mais velho, então um endereço reusado acha o estado certo.
#[cfg(feature = "devtools")]
pub fn podar_estados_de_input() {
    const MAX: usize = 1024;
    if let Ok(mut s) = KEYINPUT_STATES.lock() {
        if s.len() > MAX {
            let sobra = s.len() - MAX;
            s.drain(..sobra);
        }
    }
    if let Ok(mut s) = AXISINPUT_STATES.lock() {
        if s.len() > MAX {
            let sobra = s.len() - MAX;
            s.drain(..sobra);
        }
    }
}

// Teclas que viram caractere digitado num widget ink (`BwmsIsCharacterInput`, `BwmsToCharacterKey`). O valor de `EInputKey`
// (`redscript-src/orphans.script:3223`) de espaço, dígito e letra é o ASCII do caractere (`IK_Space=32`, `IK_0..IK_9=48..57`, `IK_A..IK_Z=65..90`).
// `to_character_key` usa tabela US-QWERTY estática, com `shift` como único modificador (não lê layout nem Caps Lock do sistema).
/// `true` se `key` (valor de `EInputKey`) produz caractere.
pub(crate) fn is_character_input_key(key: i32) -> bool {
    matches!(key, 32 | 48..=57 | 65..=90 | 96..=111 | 186..=192 | 219..=222)
}

/// Tabela US-QWERTY estática (ver nota acima). `Separator`
/// (`IK_Separator=108`, tecla numérica rara/dependente de locale mesmo no Win32 real) mapeia
/// pra vírgula por convenção — nunca confirmado contra hardware real, decisão consciente.
pub(crate) fn to_character_key(key: i32, shift: bool) -> String {
    if !is_character_input_key(key) {
        return String::new();
    }
    let ch: char = match key {
        32 => ' ',
        48..=57 => {
            if shift {
                [')', '!', '@', '#', '$', '%', '^', '&', '*', '('][(key - 48) as usize]
            } else {
                (b'0' + (key - 48) as u8) as char
            }
        }
        // Maiúscula só com shift, como o `ToUnicode` do Windows e o `ProcessCharacterEvent` do Mac (`0x10427036c`: sem shift, `tecla + 0x20`).
        65..=90 => if shift { key as u8 as char } else { (key as u8 + 0x20) as char },
        96..=105 => (b'0' + (key - 96) as u8) as char, // IK_NumPad0..9
        106 => '*',  // IK_NumStar
        107 => '+',  // IK_NumPlus
        108 => ',',  // IK_Separator (divergência, ver nota da função)
        109 => '-',  // IK_NumMinus
        110 => '.',  // IK_NumPeriod
        111 => '/',  // IK_NumSlash
        186 => if shift { ':' } else { ';' },
        187 => if shift { '+' } else { '=' },
        188 => if shift { '<' } else { ',' },
        189 => if shift { '_' } else { '-' },
        190 => if shift { '>' } else { '.' },
        191 => if shift { '?' } else { '/' },
        192 => if shift { '~' } else { '`' },
        219 => if shift { '{' } else { '[' },
        220 => if shift { '|' } else { '\\' },
        221 => if shift { '}' } else { ']' },
        222 => if shift { '"' } else { '\'' },
        _ => return String::new(),
    };
    ch.to_string()
}

/// RawInput do CallbackSystem: TODAS as teclas (JÁ MAPEADAS pro valor real de `EInputKey`, ver
/// `entrada::map_macos_keycode_to_einputkey`) + modificadores (shift/control/alt) capturados no
/// sendEvent (gameplay), pra emitir o evento "Input/Key" com um `ref<KeyInputEvent>` REAL.
static RAW_KEYS: std::sync::Mutex<Vec<(i32, bool, bool, bool)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn push_raw_key(key: i32, shift: bool, control: bool, alt: bool) {
    if let Ok(mut q) = RAW_KEYS.lock() {
        if q.len() < 64 {
            q.push((key, shift, control, alt));
        }
    }
}

fn drain_raw_keys() -> Vec<(i32, bool, bool, bool)> {
    RAW_KEYS.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Fila IRMÃ de `RAW_KEYS` (`PENDENCIAS-UNIFICADAS.md`, 2026-08-11),
/// mesmo formato, mas pro evento SOLTA-tecla (KeyUp). Antes só existia captura de key-DOWN via
/// CallbackSystem — mods conseguiam detectar "tecla pressionada" mas nunca "tecla solta"/"tecla
/// mantida", bloqueando detecção de combo/input-contínuo (que mods redscript podem construir
/// em cima disto). Mesmo mecanismo já provado (input cru com
/// nome real), só o evento nome/timing divergem — zero RE nova.
static RAW_KEYS_UP: std::sync::Mutex<Vec<(i32, bool, bool, bool)>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn push_raw_key_up(key: i32, shift: bool, control: bool, alt: bool) {
    if let Ok(mut q) = RAW_KEYS_UP.lock() {
        if q.len() < 64 {
            q.push((key, shift, control, alt));
        }
    }
}

fn drain_raw_keys_up() -> Vec<(i32, bool, bool, bool)> {
    RAW_KEYS_UP.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// Teclas do AppKit que `cp77_tick` tira das duas filas logo depois da guarda de re-entrância, com ou sem ouvinte:
/// o overlay enfileira em menu, load e gameplay, e sem esvaziar por tick a fila retém as 64 teclas mais
/// velhas e descarta as novas. Única saída das filas para fora deste módulo: `RAW_KEYS`, `RAW_KEYS_UP`, os `drain_*` e os
/// campos são privados. Toda saída antecipada do tick descarta o lote no drop.
pub(crate) struct TeclasDoTick {
    down: Vec<(i32, bool, bool, bool)>,
    up: Vec<(i32, bool, bool, bool)>,
}

impl TeclasDoTick {
    pub(crate) fn esvaziar_filas() -> Self {
        TeclasDoTick { down: drain_raw_keys(), up: drain_raw_keys_up() }
    }

    /// Passa cada tecla do lote a `disparar`: key-down como `Input/Key` com `IACT_PRESS`, key-up como `Input/KeyUp` com
    /// `IACT_RELEASE` (o `action` do evento segue a direção da tecla, ver `make_keyinputevent_arg`). Com o hook do
    /// `ProcessInputEvents` (`rawinput_motor`) a tecla vem da fila do motor e o lote inteiro cai no drop, senão cada tecla
    /// dispararia 2x. Sem ouvinte do evento, aquele lado cai no drop: `tem_ouvinte` só evita
    /// construir um `KeyInputEvent` por tecla.
    pub(crate) fn despachar(
        self,
        rawinput_motor: bool,
        tem_ouvinte: impl Fn(&str) -> bool,
        mut disparar: impl FnMut(&'static str, i32, i32, bool, bool, bool),
    ) {
        if rawinput_motor {
            return;
        }
        if tem_ouvinte("Input/Key") {
            for (key, shift, control, alt) in self.down {
                disparar("Input/Key", IACT_PRESS, key, shift, control, alt);
            }
        }
        if tem_ouvinte("Input/KeyUp") {
            for (key, shift, control, alt) in self.up {
                disparar("Input/KeyUp", IACT_RELEASE, key, shift, control, alt);
            }
        }
    }
}

#[cfg(test)]
mod teste {
    use super::*;

    type Disparo = (&'static str, i32, i32, bool, bool, bool);

    fn tamanho_das_filas() -> (usize, usize) {
        (RAW_KEYS.lock().expect("RAW_KEYS envenenada").len(), RAW_KEYS_UP.lock().expect("RAW_KEYS_UP envenenada").len())
    }

    /// Roda ticks com o mesmo par de `cp77_tick` (`esvaziar_filas` no topo, `despachar` no fim) e troca só o que pede o
    /// RTTI do jogo: `tem_ouvinte` e `disparar_evento_de_tecla` viram fechos que anotam o disparo. Os 3 ticks sem ouvinte
    /// recebem 90 teclas, acima do teto de 64: se o esvaziamento dependesse do ouvinte, a fila encheria e a tecla do tick
    /// com ouvinte seria descartada. Único teste que toca `RAW_KEYS`/`RAW_KEYS_UP`: as filas são globais do processo.
    #[test]
    fn tick_sem_ouvinte_esvazia_as_filas_e_o_ouvinte_recebe_so_a_tecla_nova() {
        drop(TeclasDoTick::esvaziar_filas());
        let mut disparos: Vec<Disparo> = Vec::new();

        for tick in 0..3 {
            for k in 0..30 {
                push_raw_key(tick * 30 + k, false, false, false);
            }
            push_raw_key_up(tick, false, false, false);
            TeclasDoTick::esvaziar_filas().despachar(false, |_| false, |e, a, k, s, c, al| disparos.push((e, a, k, s, c, al)));
            assert!(disparos.is_empty(), "sem ouvinte nada dispara");
            assert_eq!(tamanho_das_filas(), (0, 0), "o tick {tick} sem ouvinte zera as duas filas");
        }

        // Saída antecipada (sem registry ou sem player): o lote cai no drop.
        push_raw_key(90, false, false, false);
        push_raw_key_up(90, false, false, false);
        drop(TeclasDoTick::esvaziar_filas());
        assert_eq!(tamanho_das_filas(), (0, 0), "a saída antecipada descarta o lote");

        push_raw_key(91, false, false, false);
        push_raw_key_up(91, false, false, false);
        TeclasDoTick::esvaziar_filas().despachar(true, |_| true, |e, a, k, s, c, al| disparos.push((e, a, k, s, c, al)));
        assert!(disparos.is_empty(), "com o hook do motor a tecla do AppKit não dispara");
        assert_eq!(tamanho_das_filas(), (0, 0));

        push_raw_key(68, true, false, false);
        push_raw_key_up(69, false, true, false);
        TeclasDoTick::esvaziar_filas()
            .despachar(false, |evento| evento == "Input/Key", |e, a, k, s, c, al| disparos.push((e, a, k, s, c, al)));
        assert_eq!(disparos, vec![("Input/Key", IACT_PRESS, 68, true, false, false)], "o ouvinte recebe só a tecla nova");
        assert_eq!(tamanho_das_filas(), (0, 0), "o key-up sem ouvinte também sai da fila");

        disparos.clear();
        push_raw_key_up(70, false, false, true);
        TeclasDoTick::esvaziar_filas()
            .despachar(false, |evento| evento == "Input/KeyUp", |e, a, k, s, c, al| disparos.push((e, a, k, s, c, al)));
        assert_eq!(disparos, vec![("Input/KeyUp", IACT_RELEASE, 70, false, false, true)]);
    }

    /// Confere no texto de `src/` o que o teste acima não alcança, porque `cp77_tick` pede o RTTI do jogo:
    /// 1. em `cp77_tick`, o lote sai das filas entre a guarda de re-entrância e as saídas sem registry e sem
    ///    player, e o despacho no fim repassa `rawinput_motor`, o `tem_ouvinte` real e cada argumento ao
    ///    `disparar_evento_de_tecla` real;
    /// 2. `esvaziar_filas(` aparece 2 vezes em `src/`, a definição aqui e a chamada em `lib.rs` (este módulo de teste fica
    ///    fora); a contagem pelo nome do método pega também `Self::` e apelido de import;
    /// 3. nenhum outro arquivo cita `RAW_KEYS` ou `drain_raw_keys`, e aqui só `push_*`/`drain_*` travam as filas e cada
    ///    `drain_*` tem uma chamada só (a de `esvaziar_filas`, que o teste acima exercita).
    #[test]
    fn cp77_tick_esvazia_no_topo_e_despacha_no_fim_e_mais_ninguem_le_as_filas() {
        const MARCA_DO_TESTE: &str = "#[cfg(test)]\nmod teste {";
        let lib = include_str!("../lib.rs");
        let ini = lib.find("pub extern \"C\" fn cp77_tick()").expect("cp77_tick sumiu de lib.rs");
        let corpo = &lib[ini..];
        let corpo = &corpo[..corpo.find("\n}\n").expect("fim de cp77_tick")];
        let achar = |trecho: &str| corpo.find(trecho).unwrap_or_else(|| panic!("cp77_tick sem `{trecho}`"));
        let guarda = achar("let _tg = TickGuard;");
        let lote = achar("let teclas = TeclasDoTick::esvaziar_filas();");
        let saida_sem_registry = achar("let reg = match registry()");
        let saida_sem_player = achar("if player.is_null() || tx.is_null()");
        let motor = achar("let rawinput_motor = ");
        let despacho = achar(
            "teclas.despachar(rawinput_motor, tipos_do_motor::tem_ouvinte, |evento, acao, key, shift, control, alt| unsafe {\n        \
             entrada::disparar_evento_de_tecla(evento, acao, key, shift, control, alt);\n    });",
        );
        assert!(guarda < lote && lote < saida_sem_registry && saida_sem_registry < saida_sem_player);
        assert!(saida_sem_player < motor && motor < despacho);

        let src = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
        let este = src.join("entrada/mod.rs");
        let mut chamadas = Vec::new();
        let mut leitores = Vec::new();
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
                let texto = if caminho == este {
                    &texto[..texto.find(MARCA_DO_TESTE).expect("módulo de teste de entrada")]
                } else {
                    if texto.contains("RAW_KEYS") || texto.contains("drain_raw_keys") {
                        leitores.push(caminho.clone());
                    }
                    &texto[..]
                };
                for _ in texto.matches("esvaziar_filas(") {
                    chamadas.push(caminho.clone());
                }
            }
        }
        chamadas.sort();
        assert_eq!(chamadas, vec![este, std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))]);
        assert!(leitores.is_empty(), "filas citadas fora de entrada: {leitores:?}");

        let fonte = include_str!("mod.rs");
        let producao = &fonte[..fonte.find(MARCA_DO_TESTE).expect("módulo de teste de entrada")];
        assert_eq!(producao.matches("RAW_KEYS.lock()").count(), 2, "push_raw_key e drain_raw_keys");
        assert_eq!(producao.matches("RAW_KEYS_UP.lock()").count(), 2, "push_raw_key_up e drain_raw_keys_up");
        assert_eq!(producao.matches("drain_raw_keys()").count(), 2, "a definição e a chamada em esvaziar_filas");
        assert_eq!(producao.matches("drain_raw_keys_up()").count(), 2, "a definição e a chamada em esvaziar_filas");
    }
}
