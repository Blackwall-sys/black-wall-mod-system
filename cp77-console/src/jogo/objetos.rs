//! Objetos do motor: criar instância, contagem de referência e soltura.

use crate::register::handle_strong_soma;
use std::ffi::c_void;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
/// Soltura de `SharedPtr<ResourceToken>` como o loader (`0x1010e7254`): bloco, destrutor do token e liberação.
pub(crate) const FRACO_SOLTAR_BLOCO_VM: u64 = 0x1_00f3_7704;

pub(crate) const HANDLE_ULTIMA_REF_VM: u64 = 0x1_0102_5e3c;

/// AL-2c (2026-09-12, RE `proofs/20260911-allocator-al2-createinstance-offline.log`): `CClass::CreateInstance` real do Mac, `0x1010c36a0(cls, _, zerar)`.
/// Aloca pelo allocator da classe (ou do 1º ancestral nativo), roda o ConstructCls, cria o `valueHolder` pelo mesmo allocator e constrói os defaults
/// das props; devolve nulo pra classe abstrata. É o par certo da destruição do motor (que libera pelo allocator do `vt[0x10]`). Só na game thread.
pub(crate) const OBJETO_CRIAR_VM: u64 = 0x1_010c_36a0;

/// Stubs do pool: `PoolStorageProxy<PoolRefCount>::Allocate(8)` e `Free<PoolEngine>(ptr, 1)`.
pub(crate) const CONTAGEM_ALOCAR_VM: u64 = 0x1_052a_61dc;

pub(crate) const POOL_LIBERAR_VM: u64 = 0x1_052a_7388;

/// Primitivas que o motor usa inline pra soltar um `Handle`: fraco-1 (libera o bloco no zero), virtual `+0xd8` do objeto, destruição.
pub(crate) const HANDLE_SOLTA_FRACO_VM: u64 = 0x1_0102_5dc8;

pub(crate) const HANDLE_DESTROI_VM: u64 = 0x1_0102_5e4c;

/// Fraco-1 no bloco do handle, pela função do motor ([`HANDLE_SOLTA_FRACO_VM`]); o bloco é liberado quando chega a zero.
#[inline(always)]
pub(crate) unsafe fn soltar_fraco(h: *mut [u64; 2]) {
    let f: unsafe extern "C" fn(*mut [u64; 2]) = std::mem::transmute(crate::rebase(HANDLE_SOLTA_FRACO_VM));
    f(h)
}

/// O objeto do bloco ainda existe: bloco presente e forte > 0, o `Expired` do `WeakHandle` da referência (sem bloco a contagem de uso é 0). Ler o forte
/// pede um fraco segurando o bloco.
pub(crate) unsafe fn fraco_vivo(bloco: u64) -> bool {
    bloco != 0 && (*(bloco as *const std::sync::atomic::AtomicU32)).load(Ordering::SeqCst) > 0
}

/// Solta um `Handle<T>` forte como o código do motor faz inline: forte-1; se era o último, fraco-1, pergunta ao objeto
/// (virtual `+0xd8`) e destrói se ele responder diferente de zero.
pub(crate) unsafe fn soltar_handle(h: &mut [u64; 2]) {
    let bloco = h[1];
    if bloco != 0 {
        let forte = &*(bloco as *const std::sync::atomic::AtomicU32);
        if forte.fetch_sub(1, std::sync::atomic::Ordering::SeqCst) == 1 {
            let ultima_ref: unsafe extern "C" fn(*mut [u64; 2]) -> u32 = std::mem::transmute(crate::rebase(HANDLE_ULTIMA_REF_VM));
            let destroi: unsafe extern "C" fn(*mut [u64; 2]) = std::mem::transmute(crate::rebase(HANDLE_DESTROI_VM));
            soltar_fraco(h);
            if ultima_ref(h) != 0 {
                destroi(h);
            }
        }
    }
    *h = [0, 0];
}

// ===== Crash de load, AL-2 — ciclos de criação e soltura pelo motor (comando `al2ciclos <classe> <n>`, 2026-09-12) =====
// Com o hook da destruição genérica instalado (`~/.bwms-hook-handle-destroi`), cria `n` objetos
// da classe pelo `new_object`, dá dono a cada um com `make_handle` e solta pelo `soltar_handle` (a sequência inline do motor). Conta quantos o
// motor destruiu (hook em `0x101025e4c`), quantos endereços voltaram a ser usados (bloco devolvido ao pool certo) e os casos novos da guarda de
// vptr nulo. Sem o hook, recusa: não haveria como contar o que o motor destruiu.
pub(crate) static CICLO_OBJ: AtomicU64 = AtomicU64::new(0);

pub(crate) static CICLO_DESTRUIDOS: AtomicU64 = AtomicU64::new(0);

/// Solta o handle como o motor e diz se o objeto foi destruído na soltura (visto pelo hook da destruição genérica; `false` sem o hook).
pub(crate) unsafe fn soltar_handle_contando(h: &mut [u64; 2]) -> bool {
    CICLO_DESTRUIDOS.store(0, Ordering::Relaxed);
    CICLO_OBJ.store(h[0], Ordering::Relaxed);
    crate::jogo::objetos::soltar_handle(h);
    CICLO_OBJ.store(0, Ordering::Relaxed);
    CICLO_DESTRUIDOS.load(Ordering::Relaxed) > 0
}

pub(crate) static FRACOS_TRAVADOS_RETIDOS: AtomicU64 = AtomicU64::new(0);

/// Trava o `WeakPtr` como o motor: strong +1 se ainda vivo.
pub(crate) unsafe fn travar_fraco(rc: u64) -> bool {
    if rc == 0 || !crate::memoria::is_readable(rc as *const c_void, 8) {
        return false;
    }
    let s = &*(rc as *const std::sync::atomic::AtomicU32);
    let mut v = s.load(Ordering::SeqCst);
    while v != 0 {
        match s.compare_exchange_weak(v, v + 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return true,
            Err(x) => v = x,
        }
    }
    false
}

/// Solta a trava sem levar o strong a zero (destruir o builder é do motor): com strong 1 a trava fica e é contada.
pub(crate) unsafe fn destravar_fraco(rc: u64) {
    let s = &*(rc as *const std::sync::atomic::AtomicU32);
    let mut v = s.load(Ordering::SeqCst);
    while v > 1 {
        match s.compare_exchange_weak(v, v - 1, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return,
            Err(x) => v = x,
        }
    }
    FRACOS_TRAVADOS_RETIDOS.fetch_add(1, Ordering::Relaxed);
}

/// Handle forte do objeto (`ISerializable` com auto-referência) pelo construtor do motor (`0x1010b0e90`, sret).
pub(crate) unsafe fn handle_forte_de(obj: *mut u8) -> [u64; 2] {
    let mut h = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(HANDLE_DO_OBJETO_VM) as u64,
        in("x0") obj as u64,
        in("x8") h.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    h
}

/// `Handle<T>` de um objeto `ISerializable` (sret em x8, forte +1).
pub(crate) const HANDLE_DO_OBJETO_VM: u64 = 0x1_010b_0e90;

/// Grava em `slot` uma cópia fraca de `h`, soltando o fraco que estava lá.
pub(crate) unsafe fn gravar_handle_fraco(slot: *mut u64, h: [u64; 2]) {
    let mut antigo = [slot.read(), slot.add(1).read()];
    if h[1] != 0 {
        (*((h[1] + 4) as *const std::sync::atomic::AtomicU32)).fetch_add(1, Ordering::SeqCst);
    }
    slot.write(h[0]);
    slot.add(1).write(h[1]);
    if antigo[1] != 0 {
        crate::jogo::objetos::soltar_fraco(&mut antigo);
    }
}


/// Grava em `slot` uma cópia forte de `h`, soltando o Handle que estava lá.
pub(crate) unsafe fn gravar_handle_forte(slot: *mut u64, h: [u64; 2]) {
    let mut antigo = [slot.read(), slot.add(1).read()];
    handle_strong_soma(&h, 1);
    slot.write(h[0]);
    slot.add(1).write(h[1]);
    crate::jogo::objetos::soltar_handle(&mut antigo);
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Sem o hook da destruição genérica ninguém soma em `CICLO_DESTRUIDOS`: a soltura devolve `false`, zera o handle e não deixa `CICLO_OBJ`
    /// apontando para o objeto solto. Com forte 2 a soltura só tira 1 e não chama o motor, então roda fora do jogo.
    #[test]
    fn soltar_handle_contando_sem_hook_devolve_falso_e_limpa() {
        let mut bloco = [2u32, 1u32];
        let mut h = [0x1234u64, bloco.as_mut_ptr() as u64];
        let destruido = unsafe { soltar_handle_contando(&mut h) };
        assert!(!destruido);
        assert_eq!(h, [0, 0]);
        assert_eq!(bloco, [1, 1]);
        assert_eq!(CICLO_OBJ.load(Ordering::Relaxed), 0);
    }
}
