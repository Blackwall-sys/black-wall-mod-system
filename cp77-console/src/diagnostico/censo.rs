//! Censo de hooks observe-only (2026-09-11).
//!
//! Um boot confirma de uma vez os endereços achados offline naquele dia. Cada alvo recebe um stub
//! NAKED que conta a chamada, guarda as primeiras `AMOSTRAS` de (x0, x1, x2, x3, x8, x30) e dá `br`
//! no trampolim do original. O stub só usa x9..x13: não toca registrador de argumento, nem x8 (sret),
//! nem os de ponto flutuante — serve pra qualquer assinatura, inclusive as de retorno por sret e as
//! que recebem float (clima).
//!
//! Gate: `~/.bwms-hook-censo` (desligado por padrão). Relatório no canal de veredito: comando `censo`,
//! e sozinho a cada 30 s nos primeiros 10 min (só as linhas que mudaram).
//!
//! Alvos que dividem página de código com outro alvo ficaram de fora (`RegisterAffectedItem`,
//! o 2º setter de visibilidade, `FindAppearance`): um hook por página, por cautela.
//!
//! Os 3 alvos do #53 saíram em 2026-09-11 (confirmados nos boots 10 e 11) e viraram hooks de produção em
//! `tpp_attach.rs` — dois hooks no mesmo alvo entram em loop no relocador.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

/// Tem que bater com o `cmp x10, #4` do stub.
const AMOSTRAS: usize = 4;
const REGS: usize = 6;

#[repr(C)]
pub(crate) struct Slot {
    orig: AtomicU64,                       // +0x00 trampolim do original
    count: AtomicU64,                      // +0x08 chamadas
    reg: [AtomicU64; AMOSTRAS * REGS],     // +0x10 amostras: x0 x1 x2 x3 x8 x30 (48 B cada)
}

impl Slot {
    const fn new() -> Self {
        Slot {
            orig: AtomicU64::new(0),
            count: AtomicU64::new(0),
            reg: [const { AtomicU64::new(0) }; AMOSTRAS * REGS],
        }
    }
}

macro_rules! stub {
    ($stub:ident, $slot:ident) => {
        static $slot: Slot = Slot::new();
        #[unsafe(naked)]
        unsafe extern "C" fn $stub() {
            core::arch::naked_asm!(
                "adrp x9, {slot}@PAGE",
                "add  x9, x9, {slot}@PAGEOFF",
                "ldr  x10, [x9, #8]",
                "add  x11, x10, #1",
                "str  x11, [x9, #8]",
                "cmp  x10, #4",
                "b.hs 2f",
                "mov  x12, #48",
                "mul  x12, x10, x12",
                "add  x12, x12, #0x10",
                "add  x12, x9, x12",
                "stp  x0, x1, [x12]",
                "stp  x2, x3, [x12, #16]",
                "stp  x8, x30, [x12, #32]",
                "2:",
                "ldr  x13, [x9]",
                "br   x13",
                slot = sym $slot,
            )
        }
    };
}

stub!(s00, S00);
stub!(s01, S01);
stub!(s02, S02);
stub!(s03, S03);
stub!(s04, S04);
stub!(s05, S05);
stub!(s06, S06);
stub!(s07, S07);
stub!(s08, S08);
stub!(s09, S09);
stub!(s10, S10);
stub!(s11, S11);
stub!(s12, S12);
stub!(s13, S13);
stub!(s14, S14);
stub!(s15, S15);

struct Alvo {
    item: &'static str,
    nome: &'static str,
    vm: u64,
    stub: unsafe extern "C" fn(),
    slot: &'static Slot,
}

const N: usize = 16;

static ALVOS: [Alvo; N] = [
    Alvo { item: "A#44", nome: "FindCookedMappin(recurso,hash)", vm: 0x1_030f_2ff0, stub: s00, slot: &S00 },
    Alvo { item: "A#44", nome: "FindCookedPoi(recurso,hash)", vm: 0x1_0311_38f4, stub: s01, slot: &S01 },
    Alvo { item: "A#44", nome: "JournalRootFolderEntry::Initialize", vm: 0x1_0520_854c, stub: s02, slot: &S02 },
    Alvo { item: "A#44", nome: "MappinSystem::OnStreamingWorldLoaded", vm: 0x1_030f_b9f0, stub: s03, slot: &S03 },
    Alvo { item: "A#48", nome: "CMesh::ShouldPreloadAppearances", vm: 0x1_0519_ada4, stub: s04, slot: &S04 },
    Alvo { item: "A#48", nome: "CMesh::AddStubAppearance", vm: 0x1_0519_a844, stub: s05, slot: &S05 },
    Alvo { item: "A#48", nome: "CMesh::LoadMaterialsAsync(sret)", vm: 0x1_0519_c6dc, stub: s06, slot: &S06 },
    Alvo { item: "A#17", nome: "ExtractPartComponents(sret)", vm: 0x1_017d_3158, stub: s07, slot: &S07 },
    Alvo { item: "A#34", nome: "questRootInstance slot29 (Start?)", vm: 0x1_0281_1df0, stub: s08, slot: &S08 },
    Alvo { item: "C#157", nome: "questPhaseInstance::ExecuteGraph", vm: 0x1_0280_b7d0, stub: s09, slot: &S09 },
    Alvo { item: "C#157", nome: "questPhaseInstance::ExecuteNode", vm: 0x1_0280_df0c, stub: s10, slot: &S10 },
    Alvo { item: "C#254", nome: "RenderProxy visibilidade bit2", vm: 0x1_04fe_4ca0, stub: s11, slot: &S11 },
    Alvo { item: "C#81", nome: "SetWeatherByName", vm: 0x1_01a7_30fc, stub: s12, slot: &S12 },
    Alvo { item: "C#81", nome: "SetWeatherByIndex", vm: 0x1_01a7_0298, stub: s13, slot: &S13 },
    Alvo { item: "C#44", nome: "GetHitObject(sret,id,idx)", vm: 0x1_0458_3398, stub: s14, slot: &S14 },
    Alvo { item: "C#44", nome: "GetHitEntity(sret,id)", vm: 0x1_0156_e184, stub: s15, slot: &S15 },
];

/// Instala os hooks se o marcador existir. Chamado logo depois do hook do executor (`inicio/boot.rs`).
pub(crate) unsafe fn install() {
    if !crate::marcador_de_dev(".bwms-hook-censo") {
        return;
    }
    let mut ok = 0usize;
    for a in ALVOS.iter() {
        let alvo = crate::rebase(a.vm);
        if alvo.is_null() || !crate::memoria::is_readable(alvo as *const c_void, 16) {
            crate::veredito_dev!("[censo] {} {} {:#x}: alvo ilegível, pulado", a.item, a.nome, a.vm);
            continue;
        }
        // O trampolim é publicado no slot ANTES do salto ir pro alvo: o stub lê o original do slot.
        match crate::ganchos::instalar_com_threads_paradas(alvo, a.stub as *mut c_void, Some(&a.slot.orig)) {
            Some(_) => ok += 1,
            None => crate::veredito_dev!("[censo] {} {} {:#x}: replace recusou", a.item, a.nome, a.vm),
        }
    }
    crate::veredito_dev!("[censo] {ok}/{} hooks instalados (gate ~/.bwms-hook-censo)", ALVOS.len());
    std::thread::spawn(|| {
        let mut ultimo = [u64::MAX; N];
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_secs(30));
            relatorio_mudancas(&mut ultimo);
        }
    });
}

fn linha(a: &Alvo) -> String {
    let n = a.slot.count.load(Ordering::Relaxed);
    let mut s = format!("[censo] {} {} {:#x}: {n} chamada(s)", a.item, a.nome, a.vm);
    for i in 0..AMOSTRAS.min(n as usize) {
        let r = |k: usize| a.slot.reg[i * REGS + k].load(Ordering::Relaxed);
        s.push_str(&format!(
            " | #{i} x0={:#x} x1={:#x} x2={:#x} x3={:#x} x8={:#x} lr={:#x}",
            r(0),
            r(1),
            r(2),
            r(3),
            r(4),
            crate::un_rebase(r(5) as *const c_void)
        ));
    }
    s
}

fn relatorio_mudancas(ultimo: &mut [u64; N]) {
    for (i, a) in ALVOS.iter().enumerate() {
        let n = a.slot.count.load(Ordering::Relaxed);
        if n != ultimo[i] {
            ultimo[i] = n;
            crate::log_veredito(&linha(a));
        }
    }
}

/// Comando `censo` no canal: todas as linhas, mudou ou não.
#[cfg(feature = "devtools")]
pub(crate) fn relatorio() {
    for a in ALVOS.iter() {
        crate::log_veredito(&linha(a));
    }
}

// Testes do stub no próprio processo de teste (arm64): o "original" é uma função local, então dá pra
// conferir contagem, amostras e que x0..x3, x8 e d0 chegam intactos — sem jogo.
#[cfg(test)]
mod tests {
    use super::*;

    extern "C" fn soma(a: u64, b: u64, c: u64, d: u64) -> u64 {
        a + b * 10 + c * 100 + d * 1000
    }

    #[test]
    fn stub_conta_amostra_so_as_primeiras_e_repassa_argumentos() {
        S00.orig.store(soma as extern "C" fn(u64, u64, u64, u64) -> u64 as usize as u64, Ordering::SeqCst);
        let f: extern "C" fn(u64, u64, u64, u64) -> u64 =
            unsafe { std::mem::transmute(s00 as unsafe extern "C" fn()) };
        for i in 0..6u64 {
            assert_eq!(f(1 + i, 2, 3, 4), (1 + i) + 20 + 300 + 4000);
        }
        assert_eq!(S00.count.load(Ordering::SeqCst), 6);
        for i in 0..AMOSTRAS {
            assert_eq!(S00.reg[i * REGS].load(Ordering::SeqCst), 1 + i as u64, "x0 da amostra {i}");
            assert_eq!(S00.reg[i * REGS + 1].load(Ordering::SeqCst), 2);
            assert_eq!(S00.reg[i * REGS + 3].load(Ordering::SeqCst), 4);
            assert_ne!(S00.reg[i * REGS + 5].load(Ordering::SeqCst), 0, "lr gravado");
        }
    }

    #[unsafe(naked)]
    unsafe extern "C" fn original_devolve_x8() {
        core::arch::naked_asm!("mov x0, x8", "ret")
    }

    #[test]
    fn stub_preserva_x8_do_sret() {
        S01.orig.store(original_devolve_x8 as unsafe extern "C" fn() as usize as u64, Ordering::SeqCst);
        let ret: u64;
        unsafe {
            core::arch::asm!(
                "blr {f}",
                f = in(reg) s01 as unsafe extern "C" fn() as usize,
                in("x8") 0xdead_beef_u64,
                lateout("x0") ret,
                clobber_abi("C"),
            );
        }
        assert_eq!(ret, 0xdead_beef);
        assert_eq!(S01.reg[4].load(Ordering::SeqCst), 0xdead_beef, "x8 amostrado");
    }

    #[unsafe(naked)]
    unsafe extern "C" fn original_devolve_d0() {
        core::arch::naked_asm!("fmov x0, d0", "ret")
    }

    #[test]
    fn stub_preserva_d0_de_argumento_float() {
        S02.orig.store(original_devolve_d0 as unsafe extern "C" fn() as usize as u64, Ordering::SeqCst);
        let ret: u64;
        unsafe {
            core::arch::asm!(
                "blr {f}",
                f = in(reg) s02 as unsafe extern "C" fn() as usize,
                in("d0") 1.5f64,
                lateout("x0") ret,
                clobber_abi("C"),
            );
        }
        assert_eq!(ret, 1.5f64.to_bits());
    }
}
