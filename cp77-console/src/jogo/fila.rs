//! Fila de trabalho do motor (jobs): criar, esperar, sincronizar, fechar.

use std::ffi::c_void;
pub(crate) const FILA_CRIAR_VM: u64 = 0x1_04dd_f080;

pub(crate) const FILA_SINCRONIZAR_VM: u64 = 0x1_04dd_f354;

pub(crate) const FILA_ESPERAR_VM: u64 = 0x1_04dd_f3bc;

pub(crate) const FILA_FECHAR_VM: u64 = 0x1_04dd_f488;

/// "Job terminou?" do dispatcher (salta pra `0x104de2e1c`), sobre um `&JobHandle` — o `LoadMaterialSetupAsync` usa sobre `+0x90`.
pub(crate) const TAREFA_TERMINOU_VM: u64 = 0x1_04dd_f8e8;

/// `SetLocalThreadParam` (u8; 255 no fim do corpo).
pub(crate) const THREAD_GRAVAR_VM: u64 = 0x1_04df_0d14;

pub(crate) const FILA_CRIAR_NO_GRUPO_VM: u64 = 0x1_04dd_f1c4;

pub(crate) const FILA_DESPACHAR_VM: u64 = 0x1_04dd_f514;

pub(crate) const FILA_CAPTURAR_VM: u64 = 0x1_04dd_f3c4;

/// Família estática de um job do motor (a do `MeshComponent::LoadResource`), só pro profiler.
pub(crate) const FAMILIA_DO_JOB_VM: u64 = 0x1_0841_67b0;

/// Fila de jobs do motor na pilha: nova (parâmetros 0, como o `LoadMaterialsAsync`) ou no grupo do job atual.
pub(crate) unsafe fn fila_de_jobs(grupo: *const c_void) -> [u64; 16] {
    let mut fila = [0u64; 16];
    if grupo.is_null() {
        let ctor: unsafe extern "C" fn(*mut u64, u32, u64) -> *mut u64 = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_CRIAR_VM));
        ctor(fila.as_mut_ptr(), 0, 0);
    } else {
        let ctor: unsafe extern "C" fn(*mut u64, *const c_void) -> *mut u64 = std::mem::transmute(crate::rebase(FILA_CRIAR_NO_GRUPO_VM));
        ctor(fila.as_mut_ptr(), grupo);
    }
    fila
}

pub(crate) unsafe fn fila_esperar(fila: &mut [u64; 16], job: *const u64) {
    let esperar: unsafe extern "C" fn(*mut u64, *const u64) = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_ESPERAR_VM));
    esperar(fila.as_mut_ptr(), job);
}

/// `DispatchJob`: descritor {corpo, alvo, família, 0} pelo `0x104ddf514` com o byte de parâmetros da fila (+0x28) e os handles +0x10/+0x18.
pub(crate) unsafe fn fila_despachar_solto(fila: &mut [u64; 16], corpo: unsafe extern "C" fn(*mut c_void, *const c_void), alvo: *mut c_void) {
    let descritor: [u64; 4] = [corpo as usize as u64, alvo as u64, crate::rebase(FAMILIA_DO_JOB_VM) as u64, 0];
    let despachar: unsafe extern "C" fn(*const u64, u32, *mut u64, *mut u64) = std::mem::transmute(crate::rebase(FILA_DESPACHAR_VM));
    let parametros = (fila.as_ptr() as *const u8).add(0x28).read() as u32;
    despachar(descritor.as_ptr(), parametros, fila.as_mut_ptr().add(2), fila.as_mut_ptr().add(3));
}

/// `SyncWait` da fila (`0x104ddf354`), que o `MeshComponent::LoadResource` chama depois do despacho.
pub(crate) unsafe fn fila_sincronizar(fila: &mut [u64; 16]) {
    let sync: unsafe extern "C" fn(*mut u64) = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_SINCRONIZAR_VM));
    sync(fila.as_mut_ptr());
}

/// `Capture` (JobHandle em sret) e destrutor da fila.
pub(crate) unsafe fn fila_capturar_e_fechar(fila: &mut [u64; 16]) -> u64 {
    let mut job = 0u64;
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(FILA_CAPTURAR_VM) as u64,
        in("x0") fila.as_mut_ptr() as u64,
        in("x8") &mut job as *mut u64 as u64,
        clobber_abi("C"),
    );
    fila_fechar(fila);
    job
}

pub(crate) unsafe fn fila_fechar(fila: &mut [u64; 16]) {
    let dtor: unsafe extern "C" fn(*mut u64) = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_FECHAR_VM));
    dtor(fila.as_mut_ptr());
}

/// Parâmetro de thread do job como o corpo do motor: `[[[grupo+0x20]]+0x21]` no começo (se houver), 255 no fim.
pub(crate) unsafe fn gravar_param_de_thread_do_job(grupo: *const c_void, fim: bool) {
    let definir: unsafe extern "C" fn(u32) = std::mem::transmute(crate::rebase(crate::jogo::fila::THREAD_GRAVAR_VM));
    if fim {
        definir(255);
        return;
    }
    let p = (grupo as *const u64).add(4).read();
    if p != 0 {
        let q = (p as *const u64).read();
        definir((q as *const u8).add(0x21).read() as u32);
    }
}

/// Fila de jobs do motor na pilha: ctor `(q, 2, 0)` (como `0x101b0bf78`) -> `corpo(q)` -> `SyncWait` -> destrutor. Serve pra
/// esperar o job de um token (`resloadfila`/`resloadref`) e pra passar a fila a quem a recebe (`MeshComponent::LoadResource`,
/// Codeware `#168`). 0x80 bytes: o construtor escreve até +0x31 que se veja; folga pro que não se viu.
pub(crate) unsafe fn com_fila_de_jobs(corpo: &mut dyn FnMut(*mut c_void)) {
    let mut fila = [0u64; 16];
    let ctor: extern "C" fn(*mut u64, u32, u64) -> *mut u64 = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_CRIAR_VM));
    let sync: extern "C" fn(*mut u64) = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_SINCRONIZAR_VM));
    let dtor: extern "C" fn(*mut u64) -> *mut u64 = std::mem::transmute(crate::rebase(crate::jogo::fila::FILA_FECHAR_VM));
    ctor(fila.as_mut_ptr(), 2, 0);
    corpo(fila.as_mut_ptr() as *mut c_void);
    sync(fila.as_mut_ptr());
    dtor(fila.as_mut_ptr());
}

