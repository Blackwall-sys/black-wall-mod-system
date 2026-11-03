//! Listas do motor (`DynArray`).

use std::ffi::c_void;

/// `reserve(lista, capacidade, tamanho_do_elemento, alinhamento, mover)` do próprio motor. A memória nova vem do alocador do motor, então
/// quem solta depois é ele. `mover` nulo quando a lista está vazia, como o `RegisterSpots` chama.
pub(crate) const LISTA_RESERVAR_VM: u64 = 0x1_0002_8674;

/// Garante capacidade na lista pelo reserve do motor ([`LISTA_RESERVAR_VM`]). Rota única: o código montava esta chamada à mão em 30
/// lugares, cada um repetindo a assinatura.
#[inline(always)]
pub(crate) unsafe fn reservar(lista: *mut c_void, capacidade: u32, tamanho_do_elemento: u32, alinhamento: u32, mover: *const c_void) {
    let f: unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const c_void) = std::mem::transmute(crate::rebase(LISTA_RESERVAR_VM));
    f(lista, capacidade, tamanho_do_elemento, alinhamento, mover)
}

/// Buffer novo de `DynArray` pelo alocador do próprio array ([`alocar_pelo_alocador`]), com capacidade max(cap×2, size+4, 4) e os `size`
/// elementos de `passo` B copiados. O motor guarda o alocador do array na palavra alinhada a 8 logo depois dos `cap` elementos e é por ela
/// que solta o buffer: o novo reserva essa palavra e recebe a do buffer antigo. Array sem buffer (`entries` nulo ou `cap` 0) devolve `None`: não há alocador de onde copiar, e esse caso é do reserve do
/// motor (`es22_vaga_no_fim`). O antigo vaza (o dono pode ainda estar lendo). No boot 168 a falta dessa palavra derrubou o jogo quando a VM
/// soltou um array que a tinha emprestado de resultados realocados pelo `AddComponent`.
pub(crate) unsafe fn dynarray_crescer(entries: *mut u8, cap: u32, size: u32, passo: usize) -> Option<(*mut u8, u32)> {
    if entries.is_null() || cap == 0 {
        return None;
    }
    let nova_cap = cap.saturating_mul(2).max(size + 4).max(4);
    let fim = |base: usize, c: u32| (base + c as usize * passo + 7) & !7;
    let palavra = fim(entries as usize, cap);
    if !crate::memoria::is_readable(palavra as *const c_void, 8) {
        return None;
    }
    let alocador = (palavra as *const u64).read_unaligned();
    let novo = alocar_pelo_alocador(alocador, fim(0, nova_cap) + 8, 8);
    if novo.is_null() {
        return None;
    }
    if size > 0 {
        core::ptr::copy_nonoverlapping(entries, novo, size as usize * passo);
    }
    core::ptr::write_unaligned(fim(novo as usize, nova_cap) as *mut u64, alocador);
    Some((novo, nova_cap))
}

/// Retorno do `AllocAligned` do `IAllocator` do motor.
#[repr(C)]
pub(crate) struct Alocado {
    memoria: u64,
    tamanho: u64,
}

/// `tamanho` B alinhados a `alinhamento` pelo alocador `alocador`, a palavra do fim de um `DynArray` do motor: o objeto `IAllocator` é a própria
/// palavra (a vtable) e o `AllocAligned` mora em `+0x08` (o reserve do motor chama por `0x10001b1e0` com uma cópia da palavra na pilha). Buffer
/// que o BWMS põe num `DynArray` do motor sai daqui, porque o motor solta pela mesma palavra (`0x10002858c`) — inclusive ao descarregar o TweakDB
/// na recarga que a instalação de mod do mod.io faz. Com memória do Rust e a palavra copiada, o `Free` do pool recebia um ponteiro que não era
/// dele e o índice de flats recarregado voltava inválido (F2c, 2026-10-26: afiliação nula nos 1379 veículos e queda na varredura da polícia).
/// Nulo se a palavra não é legível ou o alocador falhou.
pub(crate) unsafe fn alocar_pelo_alocador(alocador: u64, tamanho: usize, alinhamento: usize) -> *mut u8 {
    if !crate::memoria::is_readable(alocador as *const c_void, 0x10) {
        return std::ptr::null_mut();
    }
    let alocar: unsafe extern "C" fn(*mut u64, u64, u64) -> Alocado = std::mem::transmute(((alocador + 8) as *const u64).read_unaligned());
    let mut palavra = alocador;
    alocar(&mut palavra, tamanho as u64, alinhamento.max(8) as u64).memoria as *mut u8
}

/// Alocador falso para teste: vtable com o `AllocAligned` em `+0x08` sobre o alocador do Rust (o slot 0 nunca é chamado).
#[cfg(test)]
pub(crate) fn alocador_de_teste() -> u64 {
    unsafe extern "C" fn alocar(_: *mut u64, tamanho: u64, alinhamento: u64) -> Alocado {
        let l = std::alloc::Layout::from_size_align(tamanho as usize, alinhamento as usize).expect("layout de teste");
        Alocado { memoria: unsafe { std::alloc::alloc_zeroed(l) } as u64, tamanho }
    }
    type Alocar = unsafe extern "C" fn(*mut u64, u64, u64) -> Alocado;
    static VTABLE: [Alocar; 2] = [alocar, alocar];
    VTABLE.as_ptr() as u64
}

pub(crate) const MOVER_HANDLES_VM: u64 = 0x1_0106_ea78;

/// `(buffer, tamanho, capacidade)` de um `DynArray` em `obj + off`.
pub(crate) unsafe fn dynarray_no_campo(obj: *const u8, off: usize) -> (*mut u8, usize, usize) {
    let a = obj.add(off);
    ((a as *const u64).read_unaligned() as *mut u8, (a.add(0x0C) as *const u32).read_unaligned() as usize, (a.add(0x08) as *const u32).read_unaligned() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Array sem buffer não tem palavra do alocador para levar: `entries` nulo ou `cap` 0 devolvem `None` antes de pedir memória ao pool do jogo,
    /// então rodam fora dele, e o buffer de quem chamou fica intacto. Esse caso é do reserve do motor ([`reservar`]).
    #[test]
    fn dynarray_crescer_sem_buffer_devolve_none() {
        let mut buffer = [0xABu8; 16];
        unsafe {
            assert!(dynarray_crescer(std::ptr::null_mut(), 4, 0, 8).is_none());
            assert!(dynarray_crescer(buffer.as_mut_ptr(), 0, 0, 8).is_none());
        }
        assert_eq!(buffer, [0xABu8; 16]);
    }
}
