//! Stub no-op do módulo Lua quando a feature `lua` está OFF (core 0% Lua, sem luajit no binário).
//! Mesmas assinaturas públicas que `lua.rs` (só as chamadas DE FORA) — os call sites não mudam.
//! `call_hook*` não entram aqui: só `hooks.rs` os chama, e ele também é stub nesta config.

pub unsafe fn fire_hotkey(_c: char) {}
pub unsafe fn fire_input(_c: char, _down: bool) {}
pub unsafe fn reset() {}
pub unsafe fn run_code(_code: &str) {}
pub unsafe fn run_mod(_name: &str, _code: &str, _dir: &std::path::Path) {}
pub unsafe fn run_event(_name: &str) {}
pub unsafe fn run_event_draw() {}
