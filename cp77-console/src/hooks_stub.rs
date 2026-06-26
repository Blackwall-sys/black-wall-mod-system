//! Stub no-op do roteador de method-hook estilo CET, quando a feature `lua` está OFF.
//! Sem Lua não há callbacks pra rotear: nada é vigiado, o dispatch não suprime nem reescreve nada.
//! Assinaturas idênticas às de `hooks.rs` (as chamadas do executor em lib.rs/selfboot.rs).

use crate::rtti::Registry;
use std::ffi::c_void;

pub fn has_pending() -> bool {
    false
}
pub unsafe fn drain_pending(_reg: &Registry) {}
pub unsafe fn watched_before(
    _func: *mut c_void,
    _ctx: *mut c_void,
    _frame: *mut c_void,
    _res: *mut c_void,
) -> (bool, u64) {
    (false, 0)
}
pub unsafe fn watched_after(_mcname: u64, _ctx: *mut c_void, _res: *mut c_void) {}
pub unsafe fn dispatch_before(
    _mcname: u64,
    _func: *mut c_void,
    _ctx: *mut c_void,
    _frame: *mut c_void,
    _res: *mut c_void,
) -> bool {
    false
}
pub unsafe fn dispatch_after(_mcname: u64, _ctx: *mut c_void) {}
pub unsafe fn dispatch_override(_mcname: u64, _ctx: *mut c_void, _res: *mut c_void) {}
