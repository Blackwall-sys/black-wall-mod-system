//! Buffers do motor (RawBuffer e afins): o allocator pela vtable do buffer e a realocação que o motor faz ao crescer.

#![allow(dead_code)]

use crate::curvas::A51G_RAWBUFFER_ALOCADOR_VT;
use std::ffi::c_void;

/// Vtable do `RawBufferAllocator` em `obj` (o `+0x10` do `RawBuffer`) quando é a conferida.
pub(crate) unsafe fn vtable_do_alocador_rawbuffer(obj: *const u8) -> Option<u64> {
    let vt = (obj as *const u64).read_unaligned();
    (vt == crate::rebase(A51G_RAWBUFFER_ALOCADOR_VT) as u64 && crate::memoria::is_readable(vt as *const c_void, 0x18)).then_some(vt)
}

/// Realoca o buffer pelo slot 0 do `RawBufferAllocator` em `obj`; com `dados` nulo o pool aloca (os chamadores do motor alocam assim). Alinhamento que não é
/// potência de 2 vira 8, o dos chamadores do motor. Devolve os dados novos ou 0.
pub(crate) unsafe fn realocar_rawbuffer(obj: *mut u8, dados: u64, tam: u32, alinhamento: u32) -> u64 {
    let Some(vt) = vtable_do_alocador_rawbuffer(obj) else { return 0 };
    let alin = if alinhamento.is_power_of_two() { alinhamento } else { 8 };
    let f: unsafe extern "C" fn(*mut u8, u64, u32, u32) -> u64 = std::mem::transmute((vt as *const u64).read_unaligned());
    f(obj, dados, tam, alin)
}

