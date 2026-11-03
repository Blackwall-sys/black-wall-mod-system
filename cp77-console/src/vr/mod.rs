//! Realidade virtual (CPVR): modo estéreo, pose da cabeça e das mãos do headset, gatilho.

#![allow(dead_code)]

#[cfg(feature = "capture")]
pub(crate) mod captura;

/// A aba CPVR do terminal: acende quando um headset entra em sessão.
pub(crate) mod painel;

/// As mãos do headset, do lado do jogo (o redscript as consulta). Mãos e cabeça só têm leitor nas natives da feature `cpvr`.
#[cfg(any(test, feature = "cpvr"))]
pub(crate) mod maos;

#[cfg(any(test, feature = "cpvr"))]
pub(crate) mod cabeca;

// Os itens deste módulo que usam estes nomes são todos da feature `cpvr`.
#[cfg(feature = "cpvr")]
use crate::mods_do_usuario::*;
#[cfg(feature = "cpvr")]
use std::ffi::c_void;
#[cfg(feature = "cpvr")]
use std::sync::atomic::Ordering;
#[cfg(feature = "cpvr")]
use crate::rtti;
#[cfg(feature = "cpvr")]
use crate::register::*;

// ===== Toggle do CPVR (VR) — DEV-ONLY (feature `cpvr`). A UI redscript liga/desliga; persiste no
// marcador `~/.bwms-cpvr`, que os `CpvrXxxEnabled()` dos reds do CPVR leem p/ ativar/no-opar o modo
// VR. Native INERTE (só marcador de arquivo) — zero offset. O modo CPVR em si (reds do CPVR + a
// captura + o stream) NUNCA entra no build público; estas natives ficam atrás de `cpvr`. =====
#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-cpvr"))
}

/// Estado do MODO CPVR (0 Desligado / 1 v1 leve / 2 v1.1 câmera / 3 v2.0 Alyx). Fonte = arquivo
/// `~/.bwms-cpvr-mode` (conteúdo = dígito). O marcador booleano `~/.bwms-cpvr` é DERIVADO (presente
/// iff modo>0) — o gate `.cpvr-ingame` e `CpvrStereoEnabled` dependem dele.
#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_mode_marker() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-cpvr-mode"))
}

#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_write_mode(mode: i32) {
    let m = mode.clamp(0, 3);
    if let Some(p) = cpvr_mode_marker() {
        let _ = std::fs::write(&p, m.to_string().as_bytes());
    }
    // sincroniza o booleano ~/.bwms-cpvr (presente iff modo>0)
    if let Some(p) = cpvr_marker() {
        if m > 0 {
            let _ = std::fs::write(&p, b"");
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// `BwmsCpvrOn() -> Bool` — cria o marcador (modo VR LIGADO).
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_on(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = cpvr_marker() { let _ = std::fs::write(&p, b""); }
    crate::log_dev!("[cpvr] toggle -> LIGADO (via UI)");
    write_bool_ret(out, true);
}

/// `BwmsCpvrOff() -> Bool` — remove o marcador (modo VR DESLIGADO).
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_off(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    if let Some(p) = cpvr_marker() { let _ = std::fs::remove_file(&p); }
    crate::log_dev!("[cpvr] toggle -> DESLIGADO (via UI)");
    write_bool_ret(out, true);
}

/// `BwmsCpvrState() -> Bool` — true se o modo VR está ligado (marcador existe). Os reds do CPVR
/// chamam isto em `CpvrStereoEnabled()/CpvrHudEnabled()/CpvrNoAnimsEnabled()` p/ ligar/no-opar.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_state(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let on = cpvr_marker().map(|p| p.exists()).unwrap_or(false);
    write_bool_ret(out, on);
}

/// `BwmsVrMaoPresente(direita: Bool) -> Bool` — o headset está vendo essa mão agora?
/// As mãos vêm do navegador (WebXR) pelo transporte; aqui só são entregues ao script, que é quem
/// decide o que fazer com elas dentro do jogo.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_mao_presente(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let direita = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    write_bool_ret(out, crate::vr::maos::mao(direita).presente);
}

/// `BwmsVrMaoEixo(direita: Bool, eixo: Int32) -> Float` — um número da mão por vez, porque o
/// redscript não tem como receber uma struct nossa sem forjar tipo.
/// eixo: 0,1,2 = posição x,y,z · 3,4,5,6 = orientação x,y,z,w · 7 = gatilho · 8 = agarrar.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_mao_eixo(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let direita = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let eixo = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let m = crate::vr::maos::mao(direita);
    let v = match eixo {
        0..=2 => m.posicao[eixo as usize],
        3..=6 => m.orientacao[(eixo - 3) as usize],
        7 => m.gatilho,
        8 => m.agarrar,
        _ => 0.0,
    };
    write_f32_ret(out, v);
}

/// `BwmsVrMaoBotoes(direita: Bool) -> Int32` — bit 0 = A/X, bit 1 = B/Y, bit 2 = analógico pressionado.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_mao_botoes(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let direita = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    write_uint_ret(out, crate::vr::maos::mao(direita).botoes as u64, 4);
}

/// `BwmsVrLeituras() -> Int32` — quantas atualizações de mão chegaram. O script usa pra saber que o
/// dado é novo sem comparar pose a pose, e pra detectar que o headset parou de mandar.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_leituras(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, crate::vr::maos::leituras() & 0x7FFF_FFFF, 4);
}

/// `BwmsVrGatilho(direita: Bool, qual: Int32) -> Bool` — qual: 0 = apertado agora (com histerese),
/// 1 = ACABOU de apertar. A histerese e a borda são calculadas no Rust porque o script não tem onde
/// guardar estado por quadro sem escrever no `stateContext`, que os gates de arma recebem como
/// `const`. A borda vale o quadro inteiro: chamar duas vezes no mesmo quadro dá o mesmo valor.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_gatilho(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let direita = args.first().map(|(v, _)| rtti::bool_do_script(*v)).unwrap_or(false);
    let qual = args.get(1).map(|(v, _)| *v as i32).unwrap_or(0);
    let v = if qual == 1 {
        crate::vr::maos::gatilho_borda(direita)
    } else {
        crate::vr::maos::gatilho_segurado(direita)
    };
    write_bool_ret(out, v);
}

/// `BwmsVrCabecaEixo(eixo: Int32) -> Float` — onde a cabeça está: 0 = yaw, 1 = pitch, em radianos.
/// ABSOLUTO, não delta: o script compara com o que já aplicou na câmera. Com delta, um arquivo
/// perdido viraria desvio permanente na mira.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_cabeca_eixo(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let args = rtti::read_params_consuming(current_native_func(), frame);
    let eixo = args.first().map(|(v, _)| *v as i32).unwrap_or(0);
    let c = crate::vr::cabeca::cabeca();
    write_f32_ret(out, if eixo == 1 { c.pitch } else { c.yaw });
}

/// `BwmsVrCabecaSeq() -> Int32` — cresce a cada leitura nova. O script usa pra não girar a câmera
/// duas vezes com o mesmo dado, e pra saber que o headset parou de mandar.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_vr_cabeca_seq(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    rtti::read_params_consuming(current_native_func(), frame);
    write_uint_ret(out, crate::vr::cabeca::cabeca().sequencia as u64 & 0x7FFF_FFFF, 4);
}

/// `BwmsCpvrMode() -> Int32` — 0 Desligado / 1 v1 / 2 v1.1 / 3 v2.0. O seletor do menu pinta por isto.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_mode(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_i32_ret(out, cpvr_read_mode());
}

/// `BwmsCpvrModeNext() -> Bool` — avança o modo (clamp 0..3). Seta direita do seletor.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_mode_next(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let m = (cpvr_read_mode() + 1).clamp(0, 3);
    cpvr_write_mode(m);
    crate::log_dev!("[cpvr] modo -> {m} (via UI)");
    write_bool_ret(out, true);
}

/// `BwmsCpvrModePrev() -> Bool` — recua o modo (clamp 0..3). Seta esquerda do seletor.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_mode_prev(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    let m = (cpvr_read_mode() - 1).clamp(0, 3);
    cpvr_write_mode(m);
    crate::log_dev!("[cpvr] modo -> {m} (via UI)");
    write_bool_ret(out, true);
}

/// `BwmsCpvrStereoPing() -> Bool` — DIAGNÓSTICO do v2.0: o CpvrStereoOffsetCamera (redscript) chama
/// isto por-frame; logamos throttled p/ PROVAR que o hook OnUpdate dispara em gameplay (se o offset
/// não aparecer visualmente MAS este log sair = a câmera reverte no tick nativo = a "parede" real).
/// Chamado no TOPO do OnUpdate (antes do gate/player/cam). Se ISTO logar em gameplay → o hook
/// LocomotionEventsTransition.OnUpdate DISPARA e a falha do v2.0 é downstream (gate/player/cam ou
/// SetLocalTransform revertido). Se NEM isto logar → a classe é a errada p/ o estado (próximo passo).
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_stereo_ping(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n == 0 || crate::tempo::a_cada(u64::from(n), 180) {
        crate::log_dev!("[cpvr-stereo] OnUpdate TOPO rodou (chamada #{n}) — o HOOK DISPARA");
    }
    write_bool_ret(out, true);
}

/// `BwmsCpvrIsV2() -> Bool` — TRUE se o modo é v2.0 (3). Gate do CpvrStereoEnabled via BOOL (proven),
/// contornando a dúvida do retorno Int32 do BwmsCpvrMode (proto Float pode reinterpretar os bytes).
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_is_v2(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    write_bool_ret(out, cpvr_read_mode() == 3);
}

/// Chamado LOGO ANTES do SetLocalTransform (após gate+player+cam). Se ISTO logar mas a vista não
/// deslocar → SetLocalTransform RODA mas é REVERTIDO pelo tick nativo = a parede real do make-or-break.
/// Se NÃO logar (mas o topo sim) → o gate/player/cam bloqueia antes do apply.
#[cfg(feature = "cpvr")]
pub(crate) unsafe extern "C" fn tramp_cpvr_apply_ping(_c: *mut c_void, _f: *mut c_void, out: *mut c_void, _rt: i64) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    if n == 0 || crate::tempo::a_cada(u64::from(n), 180) {
        crate::log_dev!("[cpvr-stereo] vai chamar SetLocalTransform (chamada #{n}) — passou gate+player+cam");
    }
    write_bool_ret(out, true);
}


#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_read_mode() -> i32 {
    cpvr_mode_marker()
        .and_then(|p| std::fs::read_to_string(&p).ok())
        .and_then(|s| s.trim().parse::<i32>().ok())
        .unwrap_or(0)
        .clamp(0, 3)
}


/// CPVR (dev): remove o marcador `.cpvr-ingame` stale no boot (1x, no on_load), pra o cpvr.js
/// começar com `gameplayActive=false` — sem capturar no menu/boot. O tick recria só com player vivo.
/// Sem isso, o arquivo persiste em disco entre sessões e todo boot herda o marcador preso.
#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_clear_stale_ingame() {
    if let Some(red4) = std::path::Path::new(&mods_dir()).parent() {
        let _ = std::fs::remove_file(red4.join("logs").join(".cpvr-ingame"));
    }
}

/// CPVR (dev, opt-in): avisa o cpvr.js que estamos EM JOGO (player+tx vivos), pra a captura VR
/// armar só no gameplay — nunca no menu/boot. NÃO muda a captura, só o "quando". Age só na
/// TRANSIÇÃO (não toca disco todo tick) e só com CPVR ligado (~/.bwms-cpvr).
#[cfg(feature = "cpvr")]
pub(crate) fn cpvr_ingame_marker(in_game: bool) {
    static LAST: std::sync::atomic::AtomicI8 = std::sync::atomic::AtomicI8::new(-1);
    let want = in_game as i8;
    if LAST.swap(want, Ordering::Relaxed) == want {
        return; // sem mudança de estado → não mexe no disco
    }
    let home = std::env::var("HOME").unwrap_or_default();
    if home.is_empty() || !std::path::Path::new(&format!("{home}/.bwms-cpvr")).exists() {
        return; // CPVR desligado → não sinaliza
    }
    let red4 = match std::path::Path::new(&mods_dir()).parent() {
        Some(p) => p.to_path_buf(),
        None => return,
    };
    let marker = red4.join("logs").join(".cpvr-ingame");
    if in_game {
        let _ = std::fs::File::create(&marker);
    } else {
        let _ = std::fs::remove_file(&marker);
    }
}

