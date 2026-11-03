//! Camada de UI no mundo (`inkWorldLayer`): as telas, anúncios e hologramas que o jogo desenha como widget no mundo 3D.
//!
//! Teto de world widgets (ArchiveXL `#102`): o job que atualiza os componentes do layer (`0x1043181c8`, corpo de `ParallelFor` sobre a faixa
//! de índices) só manda os 10 primeiros componentes marcados (bit 1 do byte `+0x8` do objeto que o componente devolve em `vt+0x100`) pra
//! atualização (`0x104315560`); do 11º em diante, `cmp x22, #0x9; b.hi` desvia pra `0x10431422c`. Um mod com mais de 10 widgets no mundo via o
//! resto parado. O patch troca o `b.hi` (`0x1043182f8`) por `nop`: todo componente marcado vai pra atualização, como na referência (onde é o
//! `jae` depois do `cmp ebx, 0xA`).

use core::ffi::c_void;
#[cfg(feature = "devtools")]
use std::sync::atomic::{AtomicU64, Ordering};

const DESVIO_DO_TETO_VM: u64 = 0x1_0431_82f8;
const DESVIO_DO_TETO: u32 = 0x5400_04a8; // b.hi +0x94
const COMPARACAO_DO_TETO_VM: u64 = 0x1_0431_82f4;
const COMPARACAO_DO_TETO: u32 = 0xf100_26df; // cmp x22, #0x9
const NOP: u32 = 0xd503_201f;

/// O teto de 10 sai (no carregamento; com o teste ligado, só na 2ª metade do teste).
pub(crate) unsafe fn tirar_teto() -> bool {
    let cmp = crate::rebase(COMPARACAO_DO_TETO_VM);
    let desvio = crate::rebase(DESVIO_DO_TETO_VM);
    if !crate::memoria::is_readable(cmp as *const c_void, 8) || (cmp as *const u32).read_unaligned() != COMPARACAO_DO_TETO {
        crate::log_dev!("[ink] teto de world widgets: comparação fora do lugar — patch não aplicado");
        return false;
    }
    if (desvio as *const u32).read_unaligned() == NOP {
        return true;
    }
    let ok = crate::ganchos::trocar_instrucao(desvio, DESVIO_DO_TETO, NOP);
    crate::log_dev!("[ink] teto de 10 world widgets retirado: {ok}");
    ok
}

pub(crate) unsafe fn instalar() {
    // O teste tira o teto ele mesmo, na 2ª metade (`teste_tick`).
    #[cfg(feature = "devtools")]
    if crate::marcador_individual("widgets-teste") {
        teste_instalar();
        return;
    }
    tirar_teto();
}

// ===== Teste (`~/.bwms-hook-widgets-teste`) ==============================================================================================
// O save do teste tem 8 world widgets e nenhum pede o modo marcado (boot 297: 0 chamadas nas duas fases), então a cena não exercita o teto. O
// teste chama o próprio corpo do job do motor (`0x1043181c8`) com um layer e 12 componentes sintéticos marcados (vtable com o slot `+0x100`
// devolvendo um objeto com o bit 1 em `+0x8`, contagem de referência alta pra nunca soltar), antes e depois do patch. Os ganchos da atualização
// (`0x104315560`) e do desvio (`0x10431422c`) contam e devolvem na hora quando o layer é o sintético, sem chamar a original; o resto passa. Com o
// teto: 10 atualizados e 2 desviados; sem: 12 e 0. O job grava um byte de TLS no fim (`0x104df0d14`, 0xff): o valor da thread é guardado antes
// (`0x104df0d3c`) e devolvido.

#[cfg(feature = "devtools")]
const JOB_VM: u64 = 0x1_0431_81c8;
#[cfg(feature = "devtools")]
const JOB_PROLOGO: u64 = 0xa903_5ff8_d101_c3ff;
#[cfg(feature = "devtools")]
const ATUALIZA_VM: u64 = 0x1_0431_5560;
#[cfg(feature = "devtools")]
const ATUALIZA_PROLOGO: u64 = 0xa902_5ff8_d101_83ff;
#[cfg(feature = "devtools")]
const ALEM_DO_TETO_VM: u64 = 0x1_0431_422c;
#[cfg(feature = "devtools")]
const ALEM_DO_TETO_PROLOGO: u64 = 0xa903_5ff8_d101_c3ff;
#[cfg(feature = "devtools")]
const RETORNO_DO_TETO_VM: u64 = 0x1_0431_8390;
#[cfg(feature = "devtools")]
const TLS_LE_VM: u64 = 0x1_04df_0d3c;
#[cfg(feature = "devtools")]
const COMPONENTES: usize = 12;

#[cfg(feature = "devtools")]
static ORIG_ATUALIZA: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static ORIG_ALEM: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static RETORNO_DO_TETO: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static LAYER_SINTETICO: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static CONTA_ATUALIZA: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static CONTA_ALEM: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static SINT_ATUALIZA: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "devtools")]
static SINT_ALEM: AtomicU64 = AtomicU64::new(0);

#[unsafe(naked)]
#[cfg(feature = "devtools")]
unsafe extern "C" fn shim_atualiza() {
    core::arch::naked_asm!(
        ".arch_extension lse",
        "adrp x16, {l}@PAGE",
        "add  x16, x16, {l}@PAGEOFF",
        "ldr  x16, [x16]",
        "cmp  x16, x0",
        "b.ne 1f",
        "adrp x16, {s}@PAGE",
        "add  x16, x16, {s}@PAGEOFF",
        "mov  x17, #1",
        "ldadd x17, x17, [x16]",
        "ret",
        "1:",
        "adrp x16, {c}@PAGE",
        "add  x16, x16, {c}@PAGEOFF",
        "mov  x17, #1",
        "ldadd x17, x17, [x16]",
        "adrp x16, {o}@PAGE",
        "add  x16, x16, {o}@PAGEOFF",
        "ldr  x16, [x16]",
        "br   x16",
        l = sym LAYER_SINTETICO,
        s = sym SINT_ATUALIZA,
        c = sym CONTA_ATUALIZA,
        o = sym ORIG_ATUALIZA,
    )
}

/// Conta as chamadas que voltam pro ponto depois do teto (componente do 11º em diante); as outras 2 chamadas de `0x10431422c` passam sem contar.
#[unsafe(naked)]
#[cfg(feature = "devtools")]
unsafe extern "C" fn shim_alem() {
    core::arch::naked_asm!(
        ".arch_extension lse",
        "adrp x16, {l}@PAGE",
        "add  x16, x16, {l}@PAGEOFF",
        "ldr  x16, [x16]",
        "cmp  x16, x0",
        "b.ne 1f",
        "adrp x16, {s}@PAGE",
        "add  x16, x16, {s}@PAGEOFF",
        "mov  x17, #1",
        "ldadd x17, x17, [x16]",
        "ret",
        "1:",
        "adrp x16, {r}@PAGE",
        "add  x16, x16, {r}@PAGEOFF",
        "ldr  x16, [x16]",
        "cmp  x16, x30",
        "b.ne 2f",
        "adrp x16, {c}@PAGE",
        "add  x16, x16, {c}@PAGEOFF",
        "mov  x17, #1",
        "ldadd x17, x17, [x16]",
        "2:",
        "adrp x16, {o}@PAGE",
        "add  x16, x16, {o}@PAGEOFF",
        "ldr  x16, [x16]",
        "br   x16",
        l = sym LAYER_SINTETICO,
        s = sym SINT_ALEM,
        r = sym RETORNO_DO_TETO,
        c = sym CONTA_ALEM,
        o = sym ORIG_ALEM,
    )
}

#[cfg(feature = "devtools")]
unsafe fn teste_instalar() {
    let (a, b) = (crate::rebase(ATUALIZA_VM), crate::rebase(ALEM_DO_TETO_VM));
    let confere = |p: *mut c_void, v: u64| crate::memoria::is_readable(p as *const c_void, 8) && (p as *const u64).read_unaligned() == v;
    if !confere(a, ATUALIZA_PROLOGO) || !confere(b, ALEM_DO_TETO_PROLOGO) || !confere(crate::rebase(JOB_VM), JOB_PROLOGO) {
        return crate::log_dev!("[ink] teste de world widgets: prólogos fora do esperado — sem contadores");
    }
    RETORNO_DO_TETO.store(crate::rebase(RETORNO_DO_TETO_VM) as u64, Ordering::Release);
    let ok_a = crate::ganchos::instalar_com_threads_paradas(a, shim_atualiza as *mut c_void, Some(&ORIG_ATUALIZA)).is_some();
    let ok_b = crate::ganchos::instalar_com_threads_paradas(b, shim_alem as *mut c_void, Some(&ORIG_ALEM)).is_some();
    crate::log_dev!("[ink] teste de world widgets: contadores na atualização {ok_a} e além do teto {ok_b}");
}

/// Contagem de referência que nunca chega a 1 (o job trava e solta cada handle).
#[repr(C)]
#[cfg(feature = "devtools")]
struct Contagem {
    forte: u32,
    fraca: u32,
}

#[repr(C)]
#[cfg(feature = "devtools")]
struct Sinteticos {
    contagem: Contagem,
    objeto: [u8; 0x40],
    vtable: [u64; 0x22],
    componente: [u64; 4],
    handles: [[u64; 2]; COMPONENTES],
    array: [u64; 2],
    layer: [u8; 0x100],
    fechamento: [u64; 3],
    contexto: [u64; 8],
}

#[cfg(feature = "devtools")]
unsafe extern "C" fn objeto_do_componente(_c: *mut u8) -> *mut u8 {
    SINTETICOS.with(|s| s.get().cast::<u8>().add(core::mem::offset_of!(Sinteticos, objeto)))
}

#[cfg(feature = "devtools")]
thread_local! {
    static SINTETICOS: std::cell::UnsafeCell<Sinteticos> = const { std::cell::UnsafeCell::new(unsafe { core::mem::zeroed() }) };
}

/// Uma passada do job do motor sobre os 12 componentes sintéticos: `(atualizados, desviados)`.
#[cfg(feature = "devtools")]
unsafe fn job_sintetico() -> (u64, u64) {
    SINT_ATUALIZA.store(0, Ordering::Relaxed);
    SINT_ALEM.store(0, Ordering::Relaxed);
    SINTETICOS.with(|c| {
        let s = &mut *c.get();
        s.contagem = Contagem { forte: 1_000_000, fraca: 1_000_000 };
        s.objeto[8] = 2;
        s.vtable[0x100 / 8] = objeto_do_componente as *const () as usize as u64;
        s.componente[0] = s.vtable.as_ptr() as u64;
        let rc = &s.contagem as *const Contagem as u64;
        for h in s.handles.iter_mut() {
            *h = [s.componente.as_ptr() as u64, rc];
        }
        s.array = [s.handles.as_ptr() as u64, (COMPONENTES as u64) << 32 | COMPONENTES as u64];
        LAYER_SINTETICO.store(s.layer.as_ptr() as u64, Ordering::Release);
        s.fechamento = [s.layer.as_ptr() as u64, rc, s.array.as_ptr() as u64];
        s.contexto = [0; 8];
        let le: extern "C" fn() -> u8 = std::mem::transmute(crate::rebase(TLS_LE_VM));
        let antes = le();
        let job: extern "C" fn(*const u64, u64, u32, u32, *const u64) = std::mem::transmute(crate::rebase(JOB_VM));
        job(s.fechamento.as_ptr(), 0, 0, COMPONENTES as u32, s.contexto.as_ptr());
        let grava: extern "C" fn(u8) = std::mem::transmute(crate::rebase(crate::jogo::fila::THREAD_GRAVAR_VM));
        grava(antes);
    });
    (SINT_ATUALIZA.load(Ordering::Relaxed), SINT_ALEM.load(Ordering::Relaxed))
}

#[cfg(feature = "devtools")]
pub(crate) fn teste_tick() {
    static FEITO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static DESDE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("widgets-teste") || !crate::inicio::boot::gameplay_real() {
        return;
    }
    if DESDE.get_or_init(std::time::Instant::now).elapsed().as_secs() < 20 {
        return;
    }
    FEITO.store(true, Ordering::Relaxed);
    if ORIG_ATUALIZA.load(Ordering::Relaxed) == 0 || ORIG_ALEM.load(Ordering::Relaxed) == 0 {
        return crate::veredito_dev!("[ink] VEREDITO-WIDGETS contadores não instalados; widgets-ok=false");
    }
    let (reais_atualiza, reais_alem) = (CONTA_ATUALIZA.load(Ordering::Relaxed), CONTA_ALEM.load(Ordering::Relaxed));
    let com_teto = unsafe { job_sintetico() };
    let tirou = unsafe { tirar_teto() };
    let nop = unsafe { (crate::rebase(DESVIO_DO_TETO_VM) as *const u32).read_unaligned() == NOP };
    let sem_teto = unsafe { job_sintetico() };
    let ok = com_teto == (10, 2) && tirou && nop && sem_teto == (COMPONENTES as u64, 0);
    crate::veredito_dev!(
        "[ink] VEREDITO-WIDGETS job do motor com {COMPONENTES} componentes marcados: com o teto {} atualizados e {} desviados; patch {tirou} (nop no lugar \
         {nop}); sem o teto {} atualizados e {} desviados; na cena até aqui {reais_atualiza} atualizações e {reais_alem} desviados reais; widgets-ok={ok}",
        com_teto.0, com_teto.1, sem_teto.0, sem_teto.1
    );
}
