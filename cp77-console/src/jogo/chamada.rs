//! Chamar função do motor com retorno por sret (x8): o jeito do Mac de devolver struct grande.

#![allow(dead_code)]

use std::ffi::c_void;

/// Chama `orig` com x0–x3 e a saída explícita no `x8` (retorno de struct não trivial do motor, sret no Itanium): o motor escreve só o tamanho dele.
pub(crate) unsafe fn sret_chamar(orig: *mut c_void, x: [u64; 4], saida: *mut c_void) {
    core::arch::asm!(
        "blr {f}",
        f = in(reg) orig as u64,
        in("x0") x[0],
        in("x1") x[1],
        in("x2") x[2],
        in("x3") x[3],
        in("x8") saida as u64,
        clobber_abi("C"),
    );
}

