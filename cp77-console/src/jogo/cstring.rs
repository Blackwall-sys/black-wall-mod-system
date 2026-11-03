//! `CString` do motor (`red::CString`, 0x20 bytes), a camada comum de quem cria, copia, lê ou destrói texto do jogo: os textos da
//! localização e o retorno `String` das natives. Até 2026-09-19 o retorno de native só escrevia o buffer inline (até 19 bytes) e ~38
//! natives fatiavam texto maior; o construtor do motor aloca pelo alocador do próprio jogo, então o texto pode ter qualquer tamanho.
//!
//! RE: `CString(destino, dados, tamanho, &alocador)` sobre destino cru (`0x10002d170`: grava o alocador em `+0x18`, zera, reserva, copia e
//! termina em zero), com o alocador padrão do motor (`0x1077cdd10`, o valor que `0x1000289a4` põe no objeto de alocador); `CString(const CString&)` sobre destino cru (`0x10002b0ac`: zera os campos e
//! copia inline ou aloca); destrutor (`0x10002914c`: libera se não for inline); `c_str` (`0x100029328`).

use core::ffi::c_void;

const NOVA_VM: u64 = 0x1_0002_d170;
const ALOCADOR_PADRAO_VM: u64 = 0x1_077c_dd10;
const COPIA_VM: u64 = 0x1_0002_b0ac;
const TEXTO_VM: u64 = 0x1_0002_9328;

/// Monta em `dst` (0x20 bytes crus) um `CString` com `texto`. Até o boot 266 usava `0x100f40ec4`, que monta e passa o texto inteiro por
/// `ToUpper` (`0x10002ba80` -> `0x100001b90`): o valor de 60 caracteres do teste de argumentos voltou com o tamanho certo e em maiúsculas.
pub(crate) unsafe fn nova(dst: *mut u8, texto: &str) {
    let alocador = crate::rebase(ALOCADOR_PADRAO_VM) as u64;
    let f: unsafe extern "C" fn(*mut u8, *const u8, u32, *const u64) -> *mut u8 = std::mem::transmute(crate::rebase(NOVA_VM));
    f(dst, texto.as_ptr(), texto.len() as u32, &alocador);
}

/// Copia `src` pra `dst` (cru).
pub(crate) unsafe fn copia(dst: *mut u8, src: *const u8) {
    let f: unsafe extern "C" fn(*mut u8, *const u8) -> *mut u8 = std::mem::transmute(crate::rebase(COPIA_VM));
    f(dst, src);
}

pub(crate) unsafe fn destroi(cs: *mut u8) {
    let f: unsafe extern "C" fn(*mut u8) -> *mut u8 = std::mem::transmute(crate::rebase(crate::jogo::texto::TEXTO_DESTRUIR_VM));
    f(cs);
}

/// Texto de um `CString` do motor.
pub(crate) unsafe fn texto(cs: *const u8) -> String {
    let f: unsafe extern "C" fn(*const u8) -> *const i8 = std::mem::transmute(crate::rebase(TEXTO_VM));
    let p = f(cs);
    if p.is_null() || !crate::memoria::is_readable(p as *const c_void, 1) {
        return String::new();
    }
    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
}

pub(crate) unsafe fn destroi_cstring_da_pilha(s: &mut [u8; 0x20]) {
    let f: unsafe extern "C" fn(*mut u8) = std::mem::transmute(crate::rebase(crate::jogo::texto::TEXTO_DESTRUIR_VM));
    f(s.as_mut_ptr());
}

