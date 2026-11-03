//! O motor do jogo (`CBaseEngine`), a camada comum de quem pede algo ao motor inteiro. O objeto vem do global `0x109d0e288`, o que o `Run`
//! do app (`0x104d6a564`) e o `OnUpdate` de Running leem; o `buildString` dele em `+0x150` é o do executável ("3.0.5814798 …" na 2.5; o 2.31 dava
//! "3.0.5314055 … (FINAL)" no boot 288), vtable estática `0x10784d418`.
//!
//! Screenshot: o slot `+0xE8` (`0x10158d7d0`) recebe (motor, modo, caminho `CString`, callback de 0x50 B), monta o
//! `rendSingleScreenShotData` (modo em `+0x34`, caminho de saída em `+0x38`, formato em `+0x8C`: PNG nos modos normais, EXR nos 5 e 6) e o
//! entrega ao objeto em `motor+0x390` (`0x104d56010`); sem esse objeto a chamada não faz nada. No SDK do Windows o slot é o `+0xE0`; no Mac o
//! destrutor ocupa dois slots.

use core::ffi::c_void;

const MOTOR_GLOBAL_VM: u64 = 0x1_09d0_e288;
const SCREENSHOT_VM: u64 = 0x1_0158_d7d0;
const SLOT_SCREENSHOT: u64 = 0xE8;
/// `rendScreenshotMode::NORMAL`.
pub(crate) const MODO_NORMAL: i32 = 1;

pub(crate) fn motor() -> *mut c_void {
    unsafe {
        let g = crate::rebase(MOTOR_GLOBAL_VM) as *const u64;
        if !crate::memoria::is_readable(g as *const c_void, 8) {
            return std::ptr::null_mut();
        }
        let m = g.read_unaligned() as *mut c_void;
        if crate::memoria::is_readable(m, 0x398) { m } else { std::ptr::null_mut() }
    }
}

#[repr(C, align(16))]
struct CallbackVazio([u8; 0x50]);

/// Pede ao motor um screenshot em `caminho`. `false` sem motor, com a vtable fora do lugar ou sem o objeto de captura (`+0x390`).
pub(crate) unsafe fn screenshot(caminho: &str, modo: i32) -> bool {
    let m = motor();
    if m.is_null() {
        return false;
    }
    let vt = (m as *const u64).read_unaligned();
    if !crate::memoria::is_readable((vt + SLOT_SCREENSHOT) as *const c_void, 8) {
        return false;
    }
    let f = ((vt + SLOT_SCREENSHOT) as *const u64).read_unaligned();
    if f != crate::rebase(SCREENSHOT_VM) as u64 {
        crate::log_dev!("[motor] slot do screenshot fora do lugar ({:#x})", crate::un_rebase(f as *const c_void));
        return false;
    }
    if ((m as *const u8).add(0x390) as *const u64).read_unaligned() == 0 {
        return false;
    }
    let mut cs = [0u8; 0x20];
    crate::jogo::cstring::nova(cs.as_mut_ptr(), caminho);
    let cb = CallbackVazio([0; 0x50]);
    let tirar: unsafe extern "C" fn(*mut c_void, i32, *const u8, *const CallbackVazio) = std::mem::transmute(f as usize);
    tirar(m, modo, cs.as_ptr(), &cb);
    crate::jogo::cstring::destroi(cs.as_mut_ptr());
    true
}

// ===== Teste (`~/.bwms-hook-screenshot-teste`) ===========================================================================================
// Em gameplay, o `BwmsTakeScreenshot` de produção com um caminho em /tmp; depois de alguns segundos, o arquivo tem de existir e ser PNG, com
// largura e altura lidas do IHDR.

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: AtomicU64 = AtomicU64::new(0);
    static QUANDO: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    const BASE: &str = "/tmp/bwms-screenshot-teste";
    if FASE.load(Ordering::Relaxed) >= 2 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("screenshot-teste") {
        return;
    }
    if !crate::inicio::boot::gameplay_real() {
        return;
    }
    let Ok(mut q) = QUANDO.lock() else { return };
    let agora = std::time::Instant::now();
    let desde = *q.get_or_insert(agora);
    let candidatos = || -> Vec<(std::path::PathBuf, u64)> {
        let mut v = Vec::new();
        for e in std::fs::read_dir("/tmp").into_iter().flatten().flatten() {
            let p = e.path();
            if p.to_string_lossy().starts_with(BASE) {
                if p.is_dir() {
                    for f in std::fs::read_dir(&p).into_iter().flatten().flatten() {
                        v.push((f.path(), f.metadata().map(|m| m.len()).unwrap_or(0)));
                    }
                } else {
                    v.push((p.clone(), e.metadata().map(|m| m.len()).unwrap_or(0)));
                }
            }
        }
        v
    };
    match FASE.load(Ordering::Relaxed) {
        0 => {
            if desde.elapsed().as_secs() < 15 {
                return;
            }
            for (p, _) in candidatos() {
                let _ = std::fs::remove_file(p);
            }
            let pediu = unsafe { screenshot(&format!("{BASE}.png"), MODO_NORMAL) };
            crate::log_dev!("[motor] screenshot pedido {pediu}");
            *q = Some(agora);
            FASE.store(if pediu { 1 } else { 2 }, Ordering::Relaxed);
            if !pediu {
                crate::veredito_dev!("[motor] VEREDITO-SCREENSHOT o motor recusou o pedido (sem motor, slot fora do lugar ou sem o objeto de captura); screenshot-ok=false");
            }
        }
        _ => {
            let achados = candidatos();
            let png = achados.iter().find_map(|(p, n)| {
                let b = std::fs::read(p).ok()?;
                (b.len() >= 24 && b[..8] == [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]).then(|| {
                    let l = u32::from_be_bytes([b[16], b[17], b[18], b[19]]);
                    let a = u32::from_be_bytes([b[20], b[21], b[22], b[23]]);
                    (p.clone(), *n, l, a)
                })
            });
            if png.is_none() && desde.elapsed().as_secs() < 20 {
                return;
            }
            FASE.store(2, Ordering::Relaxed);
            let lista: Vec<String> = achados.iter().map(|(p, n)| format!("{} ({n} B)", p.display())).collect();
            let ok = png.as_ref().is_some_and(|(_, n, l, a)| *n > 1000 && *l > 0 && *a > 0);
            crate::veredito_dev!(
                "[motor] VEREDITO-SCREENSHOT BwmsTakeScreenshot pelo slot +0xE8 do motor; arquivos {lista:?}; PNG {:?}; screenshot-ok={ok}",
                png.map(|(p, n, l, a)| format!("{} {n} B {l}x{a}", p.display()))
            );
        }
    }
}

/// `CGameFramework*` no motor do Mac (no SDK do Windows, `+0x308`): medido no boot da lane L-6 de 2026-10-28, o ponteiro cujo `+0x10` é o
/// `GameInstance` do V (`proofs/20261028-lane-L-6-framework-no-motor/`).
const FRAMEWORK_NO_MOTOR: usize = 0x378;
const GAME_INSTANCE_NO_FRAMEWORK: usize = 0x10;

/// `GameInstance*` do motor (`motor → framework → gameInstance`), o da camada Lua: existe no menu, antes do V.
pub(crate) fn game_instance() -> Option<*mut c_void> {
    let m = motor();
    if m.is_null() {
        return None;
    }
    unsafe {
        let fw = ((m as *const u8).add(FRAMEWORK_NO_MOTOR) as *const u64).read_unaligned() as *const u8;
        if !crate::rtti::sane(fw as *mut c_void) || !crate::memoria::is_readable(fw.add(GAME_INSTANCE_NO_FRAMEWORK) as *const c_void, 8) {
            return None;
        }
        let gi = (fw.add(GAME_INSTANCE_NO_FRAMEWORK) as *const u64).read_unaligned() as *mut c_void;
        crate::rtti::sane(gi).then_some(gi)
    }
}
