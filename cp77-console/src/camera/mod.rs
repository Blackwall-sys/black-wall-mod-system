//! Câmera: terceira pessoa, olhar forçado, modo de visão (braindance), foto e captura de tela.

#![allow(dead_code)]

use std::ffi::c_void;
use crate::rtti::{self};
use crate::register::*;
// Só os handlers de prova da captura usam, e o `corpo_de_prova!` público não os cita.
#[cfg(feature = "devtools")]
use std::sync::atomic::Ordering;
#[cfg(feature = "devtools")]
use crate::diagnostico::{CAPTURED_ES_PTR, CAPTURED_REQ_PTR};

/// `BwmsCamBack() -> Int32` — mod "ver corpo" / 3ª pessoa DE VERDADE (sem o crouch do
/// ActivateTPPRepresentation). Lê `~/.bwms-camback` = distância pra trás em CENTÍMETROS (0 =
/// vanilla/1ª pessoa). O poller redscript aplica isso como offset LOCAL do `FPPCameraComponent`
/// (o corpo do V já é renderizado sempre; só a câmera vai pra trás → V aparece em pé, andando
/// normal). O assistente seta o valor e o Perrotta pede "mais/menos" até a distância perfeita —
/// dá pra tunar SÓ trocando o número no marcador, sem recompilar.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_cam_back(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-camback")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}

/// `BwmsCamX() -> Int32` — eixo X (lateral) do offset local da câmera, em CM (assinado), lido de `~/.bwms-camx`.
/// Registrada e declarada, mas nenhum `.reds` lê: os `SetLocalPosition` da câmera usam X = 0.0 (`bwms-tppcam.reds`).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_cam_x(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-camx")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}

/// `BwmsCamZ() -> Int32` — eixo Z (altura) do offset local da câmera, em CM (assinado). Ver `BwmsCamX`.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_cam_z(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-camz")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0);
    write_uint_ret(out, v as u64, 4);
}

/// `BwmsForceLook() -> Int32` — TESTE (2026-07-15): sem HID pra olhar pra baixo de verdade, o
/// assistente força a rotação da câmera FPP pra provar/refutar visualmente se o torso (anexado via
/// ActivateTPPRepresentation) aparece ao olhar pra baixo. Lê `~/.bwms-forcelook` ("1"=forçar
/// pitch pra baixo + aplicar offset máximo; senão 0 = solta a câmera, comportamento normal do
/// jogo). Gated/opt-in, só ativo quando o marcador existe — não interfere no controle normal.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_force_look(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let v = crate::caminho_de_marcador(".bwms-forcelook")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| if s.trim() == "1" { 1u64 } else { 0 })
        .unwrap_or(0);
    write_uint_ret(out, v, 4);
}


#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_capture_es(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let p = args.first().map(|(v, _)| *v).unwrap_or(0);
    CAPTURED_ES_PTR.store(p, Ordering::Relaxed);
    crate::log_dev!("[capture] es={p:#x}");
}

#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_capture_req(_c: *mut c_void, frame: *mut c_void, _out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let args = rtti::read_params_consuming(func, frame);
    let p = args.first().map(|(v, _)| *v).unwrap_or(0);
    CAPTURED_REQ_PTR.store(p, Ordering::Relaxed);
    crate::log_dev!("[capture] req={p:#x}");
}

/// `BwmsTakeScreenshot(caminho) -> Bool`: o motor grava um screenshot PNG em `caminho` (RED4ext `#473`, `motor::screenshot`).
pub(crate) unsafe extern "C" fn tramp_take_screenshot(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let (_args, strings) = rtti::read_params_consuming_with_strings(current_native_func(), frame);
    let caminho = strings.first().cloned().flatten().unwrap_or_default();
    write_bool_ret(out, !caminho.is_empty() && crate::jogo::motor::screenshot(&caminho, crate::jogo::motor::MODO_NORMAL));
}

// `VisionModeSystem::SetBraindanceMode` (Codeware `#254`, 2ª rodada de RE, 2026-09-11): grava o modo em `+0x2378`; o motor lê o subobjeto
// `+0x2400`, que só existe com o mundo anexado.
#[cfg(feature = "devtools")]
pub(crate) const VISIONMODE_SET_BRAINDANCE_VM: u64 = 0x1_017b_e58c;

#[cfg(feature = "devtools")]
pub(crate) const VISIONMODE_BRAINDANCE_OFF: usize = 0x2378;

#[cfg(feature = "devtools")]
pub(crate) const VISIONMODE_MUNDO_OFF: usize = 0x2400;

/// `BwmsVisionModeBraindanceMode(system) -> Int32`: o modo gravado em `+0x2378`; `-1` sem sistema legível.
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_visionmode_braindance_mode(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let modo = if rtti::sane(sys) && crate::memoria::is_readable(sys as *const c_void, VISIONMODE_BRAINDANCE_OFF + 4) {
        ((sys as *const u8).add(VISIONMODE_BRAINDANCE_OFF) as *const u32).read_unaligned() as i32
    } else {
        -1
    };
    write_uint_ret(out, modo as u32 as u64, 4);
}

/// `BwmsVisionModeSetBraindanceMode(system, mode) -> Bool`: `0x1017be58c`, só com o mundo anexado (`+0x2400` não nulo).
#[cfg(feature = "devtools")]
pub(crate) unsafe extern "C" fn tramp_visionmode_set_braindance_mode(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let sys = args.first().map(|(v, _)| *v as *mut c_void).unwrap_or(std::ptr::null_mut());
    let modo = args.get(1).map(|(v, _)| *v as u32).unwrap_or(0);
    let pronto = rtti::sane(sys)
        && crate::memoria::is_readable(sys as *const c_void, VISIONMODE_MUNDO_OFF + 8)
        && rd_u64(sys, VISIONMODE_MUNDO_OFF) != 0;
    if pronto {
        let f: unsafe extern "C" fn(*mut c_void, u32) = std::mem::transmute(crate::rebase(VISIONMODE_SET_BRAINDANCE_VM));
        f(sys, modo);
    }
    crate::log_dev!("[visionmode] SetBraindanceMode(system={sys:p}, {modo}) -> {pronto}");
    write_bool_ret(out, pronto);
}


