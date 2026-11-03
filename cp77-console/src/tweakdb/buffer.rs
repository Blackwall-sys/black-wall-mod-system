//! Crescimento do buffer de valores do TweakDB (`flatDataBuffer`, RED4ext `#334`/`#348`). O vanilla vem quase cheio (boot 262: 53.928 B
//! livres de 4.291.664, 1,3%) e todo valor novo de mod mora nele; quando acaba, valor novo falha. O motor não cresce o buffer sozinho —
//! o TweakDB não foi feito pra mudar em runtime —, então o BWMS faz o que o `UpsizeFlatDataBuffer` do SDK faz:
//!
//! 1. buffer novo de 16 MB (o offset do flat tem 24 bits) pelo pool do próprio TweakDB (`PoolGMPL_TDB`), o mesmo que o jogo usa pra liberar;
//! 2. cópia do conteúdo — os flats guardam OFFSET relativo, não ponteiro, então continuam valendo;
//! 3. os ponteiros absolutos do `defaultValues` (`+0x108`, `HashMap<CName, FlatValue*>`) re-baseados no buffer novo;
//! 4. o ponteiro (`+0x148`), o fim (`+0x158`), a capacidade (`+0x150`) e a cópia que o acessor de campo lê (`+0x00`, `*(*(0x108c9c640))`,
//!    RE de 2026-07-17) trocados sob a trava `mutex00`.
//!
//! O buffer velho nunca é liberado (quem estiver com um `FlatValue*` dele no meio de um quadro continua lendo memória válida — mesma
//! escolha do SDK). Antes de mexer, confere que `+0x00` é mesmo a cópia do ponteiro e que todo valor do `defaultValues` aponta pra dentro
//! do buffer; se não, não faz nada.

use super::{mutex00_lock, mutex00_unlock, singleton, FLAT_DATA_BUFFER_OFF, FLAT_DATA_CAP_OFF, FLAT_DATA_END_OFF};
use core::ffi::c_void;

/// `red::memory::PoolStorageProxy<game::PoolGMPL_TDB>::AllocateAligned(u64 tamanho, u32 alinhamento) -> Block { ptr, tamanho }`.
const TDB_ALOCAR_ALINHADO_VM: u64 = 0x1_0375_fd74;
/// Cópia do ponteiro do buffer que o acessor de campo dos records lê.
const BUFFER_ESTATICO_OFF: usize = 0x00;
/// `defaultValues`: `HashMap<CName, FlatValue*>` (índice +0x00, size +0x08, cap +0x0c, nós +0x10, passo +0x1c; valor no nó +0x10).
pub(crate) const VALORES_PADRAO_OFF: usize = 0x108;
/// O `tdbOffset` do flat tem 24 bits.
pub(crate) const CAPACIDADE_MAXIMA: u32 = 0x00FF_FFFF;

unsafe fn rd64(p: *const u8) -> u64 {
    (p as *const u64).read_unaligned()
}
unsafe fn rd32(p: *const u8) -> u32 {
    (p as *const u32).read_unaligned()
}

/// Endereços dos slots de valor (`FlatValue*`) de todos os nós do `defaultValues`, andando o índice e as cadeias como o motor.
unsafe fn slots_dos_padroes(t: *const u8) -> Option<Vec<*mut u64>> {
    crate::jogo::mapas::nos(t.add(VALORES_PADRAO_OFF)).map(|nos| nos.into_iter().map(|n| n.add(0x10) as *mut u64).collect())
}

/// Bloco novo do pool do TweakDB: `(ponteiro, tamanho)`.
unsafe fn alocar_no_pool_do_tweakdb(tamanho: u64) -> Option<(u64, u64)> {
    let f = crate::rebase(TDB_ALOCAR_ALINHADO_VM);
    if !crate::memoria::is_readable(f as *const c_void, 4) {
        return None;
    }
    let alocar: extern "C" fn(u64, u32) -> [u64; 2] = std::mem::transmute(f);
    let [p, n] = alocar(tamanho, 8);
    (p != 0 && n >= tamanho).then_some((p, n))
}

/// Cresce o buffer até a capacidade máxima. `Ok((antigo, novo, capacidade nova, padrões re-baseados))`.
pub(crate) unsafe fn crescer() -> Result<(u64, u64, u32, usize), String> {
    let t = singleton().ok_or("sem TweakDB")?;
    let antigo = rd64(t.add(FLAT_DATA_BUFFER_OFF));
    let cap = rd32(t.add(FLAT_DATA_CAP_OFF));
    if cap >= CAPACIDADE_MAXIMA {
        return Err(format!("capacidade já no máximo ({cap:#x})"));
    }
    if rd64(t.add(BUFFER_ESTATICO_OFF)) != antigo {
        return Err(format!("TweakDB+0x00 ({:#x}) não é a cópia do buffer ({antigo:#x})", rd64(t.add(BUFFER_ESTATICO_OFF))));
    }
    let slots = slots_dos_padroes(t).ok_or("defaultValues ilegível")?;
    let fora = slots.iter().filter(|s| !(antigo..antigo + cap as u64).contains(&s.read_unaligned())).count();
    if fora > 0 {
        return Err(format!("{fora} de {} valores padrão fora do buffer", slots.len()));
    }
    let (novo, tamanho) = alocar_no_pool_do_tweakdb(CAPACIDADE_MAXIMA as u64).ok_or("pool do TweakDB não alocou")?;
    if !mutex00_lock(t) {
        return Err("mutex00 ocupado".into());
    }
    // Relido sob a trava: um valor criado entre a checagem e a trava também vai pro buffer novo.
    let (antigo, fim) = (rd64(t.add(FLAT_DATA_BUFFER_OFF)), rd64(t.add(FLAT_DATA_END_OFF)));
    let usado = fim - antigo;
    std::ptr::copy_nonoverlapping(antigo as *const u8, novo as *mut u8, usado as usize);
    for s in &slots {
        s.write_unaligned(novo + (s.read_unaligned() - antigo));
    }
    (t.add(FLAT_DATA_BUFFER_OFF) as *mut u64).write_unaligned(novo);
    (t.add(FLAT_DATA_END_OFF) as *mut u64).write_unaligned(novo + usado);
    (t.add(FLAT_DATA_CAP_OFF) as *mut u32).write_unaligned(tamanho.min(CAPACIDADE_MAXIMA as u64) as u32);
    (t.add(BUFFER_ESTATICO_OFF) as *mut u64).write_unaligned(novo);
    mutex00_unlock(t);
    crate::log_dev!("[tweakdb] buffer de valores: {antigo:#x} -> {novo:#x}, {usado} B copiados, capacidade {cap:#x} -> {:#x}, {} padrões re-baseados", tamanho.min(CAPACIDADE_MAXIMA as u64), slots.len());
    Ok((antigo, novo, tamanho.min(CAPACIDADE_MAXIMA as u64) as u32, slots.len()))
}
