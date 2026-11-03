//! Travas do motor (`SharedSpinLock`, 1 byte): a de leitura compartilhada e a de escrita exclusiva. Tomar passa pela função do motor;
//! soltar é a operação atômica que o próprio motor faz no byte, e mora só aqui para que cada tomada tenha a soltura certa.

use std::sync::atomic::{AtomicU8, Ordering};

/// Bit 7 do byte da trava: a escrita está tomada; quem soma 1 pela leitura espera ele apagar.
pub(crate) const TRAVA_ESCRITA: u8 = 0x80;

pub(crate) const TRAVA_LEITURA_VM: u64 = 0x1_0000_15c8;

/// Lock exclusivo de `SharedSpinLock` (espera até pegar); o destrave do motor é `ldclralb` com `0x80` no byte do lock.
pub(crate) const TRAVA_ESCRITA_VM: u64 = 0x1_0000_15f0;

/// Toma a trava de leitura (compartilhada) pela função do motor.
#[inline(always)]
pub(crate) unsafe fn travar_leitura(trava: *mut u8) {
    let f: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(TRAVA_LEITURA_VM));
    f(trava)
}

/// Solta a trava de leitura: tira 1 leitor do byte.
#[inline(always)]
pub(crate) unsafe fn soltar_leitura(trava: *mut u8) {
    (*(trava as *const AtomicU8)).fetch_sub(1, Ordering::AcqRel);
}

/// Toma a trava de escrita (exclusiva) pela função do motor; espera até pegar.
#[inline(always)]
pub(crate) unsafe fn travar_escrita(trava: *mut u8) {
    let f: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(TRAVA_ESCRITA_VM));
    f(trava)
}

/// Solta a trava de escrita como o motor: `ldclralb` do bit `0x80` (`fetch_and` com `AcqRel` gera a mesma instrução).
#[inline(always)]
pub(crate) unsafe fn soltar_escrita(trava: *mut u8) {
    (*(trava as *const AtomicU8)).fetch_and(!TRAVA_ESCRITA, Ordering::AcqRel);
}

/// Tenta a trava exclusiva do motor (CAS do byte de 0 pra 0x80, devolve se pegou); soltar limpa o bit 7.
pub(crate) const TRAVA_TENTAR_ESCRITA_VM: u64 = 0x1_0000_1614;

