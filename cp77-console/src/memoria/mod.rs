//! Memória do processo: ler sem crashar em ponteiro inválido (mach_vm_read_overwrite) e alocar fora do alocador do jogo.

#![allow(dead_code)]

use std::ffi::c_void;
use crate::macos::{mach_task_self_, mach_vm_allocate, mach_vm_read_overwrite, KERN_SUCCESS};

pub(crate) const VM_FLAGS_ANYWHERE: i32 = 0x1;

/// True se [address, address+len) está mapeado e legível AGORA. Não crasha. Usado
/// pelos resolvedores/diagnósticos que leem memória do jogo a partir de offsets crus.
pub unsafe fn is_readable(address: *const c_void, len: usize) -> bool {
    if address.is_null() || len == 0 {
        return false;
    }
    let n = len.min(512);
    let mut buf = [0u8; 512];
    let mut outsz: u64 = 0;
    #[cfg(test)]
    IS_READABLE_CHAMADAS.with(|c| c.set(c.get() + 1));
    let kr = mach_vm_read_overwrite(mach_task_self_, address as u64, n as u64, buf.as_mut_ptr() as u64, &mut outsz);
    kr == KERN_SUCCESS && outsz as usize >= n
}

#[cfg(test)]
thread_local! {
    /// Idas ao kernel do [`is_readable`] nesta thread: o teste de um gancho conta as dele sem ver as dos testes em paralelo.
    pub(crate) static IS_READABLE_CHAMADAS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Lê um bloco arbitrariamente maior (`buf.len()` bytes, sem o cap de 512 de `is_readable`) —
/// pensado pra varreduras de memória que precisam ler páginas inteiras
/// SEM 1 syscall por qword. Devolve `false` (buffer intocado) se a página não é legível de
/// verdade — nunca lê parcial silenciosamente.
pub unsafe fn read_chunk(address: usize, buf: &mut [u8]) -> bool {
    if address == 0 || buf.is_empty() {
        return false;
    }
    let mut outsz: u64 = 0;
    let kr = mach_vm_read_overwrite(mach_task_self_, address as u64, buf.len() as u64, buf.as_mut_ptr() as u64, &mut outsz);
    kr == KERN_SUCCESS && outsz as usize >= buf.len()
}

/// Lê 8 bytes (LE) em `address` via syscall (nunca crasha em ponteiro inválido/desalinhado —
/// mesma garantia de `is_readable`, mas devolve o VALOR em vez de só um bool). Pensado pra ler
/// campo de struct C++ real (offset conhecido, ponteiro capturado num hook) sem risco de
/// segfault mesmo que o objeto tenha sido liberado/realocado entre a captura e a leitura.
/// `register::read_u64_field` e `register::eb_u64` são formas desta leitura; a que não confere é `register::rd_u64`.
pub unsafe fn read_u64(address: usize) -> Option<u64> {
    if address == 0 {
        return None;
    }
    let mut buf = [0u8; 8];
    let mut outsz: u64 = 0;
    let kr = mach_vm_read_overwrite(mach_task_self_, address as u64, 8, buf.as_mut_ptr() as u64, &mut outsz);
    if kr == KERN_SUCCESS && outsz == 8 {
        Some(u64::from_le_bytes(buf))
    } else {
        None
    }
}

/// Allocate writable anonymous memory anywhere in the process (for devtools use).
/// Returns null on failure. Memory is NOT tracked by the game's allocator.
pub unsafe fn alloc_anywhere(bytes: usize) -> *mut u8 {
    let mut addr: u64 = 0;
    let kr = mach_vm_allocate(mach_task_self_, &mut addr, bytes as u64, VM_FLAGS_ANYWHERE);
    if kr == KERN_SUCCESS && addr != 0 { addr as *mut u8 } else { std::ptr::null_mut() }
}

#[cfg(test)]
mod testes_de_leitura {
    use super::read_u64;

    // 0x1000 cai no __PAGEZERO do executável de teste (4 GiB sem mapa no arm64): leitura direta ali derruba o processo.
    const FORA_DO_MAPA: usize = 0x1000;

    #[test]
    fn le_u64_desalinhado_em_little_endian() {
        let bytes: [u8; 16] = std::array::from_fn(|i| 0x10 + i as u8);
        let esperado = u64::from_le_bytes([0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a]);
        assert_eq!(unsafe { read_u64(bytes.as_ptr() as usize + 3) }, Some(esperado));
    }

    #[test]
    fn nulo_e_fora_do_mapa_dao_none() {
        assert_eq!(unsafe { read_u64(0) }, None);
        assert_eq!(unsafe { read_u64(FORA_DO_MAPA) }, None);
    }
}
