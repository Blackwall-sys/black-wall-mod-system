//! Remover record de mod (RED4ext `#345`, `RemoveRecord`): o inverso do `AddRecord` do motor (`0x1037cd264`). O record sai dos dois mapas do
//! TweakDB — por id (`+0x58`, `HashMap<TweakDBID, Handle>`) e por tipo (`+0x88`, tipo → `DynArray<Handle>`) — sob a trava exclusiva dos
//! records (`+0x21`: `0 → 0x80`, soltar limpa o bit), e os handles que os mapas seguravam são soltos depois de destravar (a destruição do
//! record não pode rodar com a trava). Os flats ficam, como no SDK; o registro do BWMS esquece a genealogia e o nome. Só record criado por mod:
//! o do jogo pode estar guardado por sistemas que não esperam ele sumir.

use super::{registro, singleton};
use crate::jogo::mapas;
use core::ffi::c_void;
use std::sync::atomic::{AtomicU8, Ordering};

const RECORDS_POR_ID: usize = 0x58;
const RECORDS_POR_TIPO: usize = 0x88;
const TRAVA_DOS_RECORDS: usize = 0x21;

unsafe fn travar(t: *mut u8) -> bool {
    let trava = &*(t.add(TRAVA_DOS_RECORDS) as *const AtomicU8);
    for i in 0..4_000_000u32 {
        if trava.compare_exchange(0, 0x80, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            return true;
        }
        if i & 511 == 511 {
            std::thread::yield_now();
        }
    }
    false
}

unsafe fn destravar(t: *mut u8) {
    (*(t.add(TRAVA_DOS_RECORDS) as *const AtomicU8)).fetch_and(0x7F, Ordering::Release);
}

unsafe fn rd64(p: *const u8) -> u64 {
    (p as *const u64).read_unaligned()
}

/// Onde está o handle de `rec` no array do tipo dele (`DynArray` de 16 B: entradas `+0x00`, tamanho `+0x0c`): `(array, posição)`.
unsafe fn posicao_no_tipo(t: *mut u8, tipo: u32, rec: u64) -> Option<(*mut u8, usize)> {
    let no = mapas::nos(t.add(RECORDS_POR_TIPO))?.into_iter().find(|n| (n.add(0x08) as *const u32).read_unaligned() == tipo)?;
    let arr = no.add(0x10);
    let (entradas, n) = (rd64(arr) as *const [u64; 2], (arr.add(0x0c) as *const u32).read_unaligned() as usize);
    if entradas.is_null() || !crate::memoria::is_readable(entradas as *const c_void, n * 16) {
        return None;
    }
    (0..n).find(|i| (*entradas.add(*i))[0] == rec).map(|pos| (arr, pos))
}

/// Tira a posição `pos` do array (os de depois descem uma casa) e devolve o handle que estava lá.
unsafe fn tirar_do_array(arr: *mut u8, pos: usize) -> [u64; 2] {
    let (entradas, n) = (rd64(arr) as *mut [u64; 2], (arr.add(0x0c) as *const u32).read_unaligned() as usize);
    let h = entradas.add(pos).read_unaligned();
    std::ptr::copy(entradas.add(pos + 1), entradas.add(pos), n - pos - 1);
    entradas.add(n - 1).write_unaligned([0, 0]);
    (arr.add(0x0c) as *mut u32).write_unaligned((n - 1) as u32);
    h
}

/// Remove o record `id` (de mod) do TweakDB. Acha o record nos dois mapas antes de mexer em qualquer um; `Ok(())` com ele fora dos dois.
pub(crate) unsafe fn remover_record(id: u64) -> Result<(), String> {
    if registro::original(id) {
        return Err("record do jogo: só sai record criado por mod".into());
    }
    let t = singleton().ok_or("TweakDB indisponível")?;
    let chave = bwms_primitives::tweak_db_id_chave(id);
    if !travar(t) {
        return Err("trava dos records ocupada".into());
    }
    let mut soltar: Vec<[u64; 2]> = Vec::new();
    let r = (|| -> Result<(), String> {
        let e_o_record = |n: *mut u8| bwms_primitives::tweak_db_id_chave(rd64(n.add(0x08))) == chave;
        let no = mapas::nos(t.add(RECORDS_POR_ID)).ok_or("mapa por id ilegível")?.into_iter().find(|n| e_o_record(*n)).ok_or("não está no mapa por id")?;
        let rec = rd64(no.add(0x10));
        if rec == 0 || !crate::memoria::is_readable(rec as *const c_void, 8) {
            return Err("handle do mapa por id vazio".into());
        }
        // O tipo é o que o `AddRecord` usou: `[vt+0x8]` do record.
        let tipo_fn = rd64((rd64(rec as *const u8) + 0x08) as *const u8);
        if !crate::rtti::sane(tipo_fn as *mut c_void) {
            return Err("vtable do record ilegível".into());
        }
        let tipo: extern "C" fn(u64) -> u32 = std::mem::transmute(tipo_fn);
        let (arr, pos) = posicao_no_tipo(t, tipo(rec), rec).ok_or("não está no array do tipo")?;
        let no = mapas::desligar(t.add(RECORDS_POR_ID), e_o_record).ok_or("sumiu do mapa por id")?;
        soltar.push([rd64(no.add(0x10)), rd64(no.add(0x18))]);
        std::ptr::write_bytes(no.add(0x08), 0, 0x18);
        soltar.push(tirar_do_array(arr, pos));
        Ok(())
    })();
    destravar(t);
    for mut h in soltar {
        crate::jogo::objetos::soltar_handle(&mut h);
    }
    if r.is_ok() {
        registro::esquecer(id);
    }
    r
}

/// Quantos handles o array do tipo de `rec` tem (o que o `GetRecords` do jogo lista).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn contagem_do_tipo(rec: u64) -> Option<usize> {
    let t = singleton()?;
    if rec == 0 || !crate::memoria::is_readable(rec as *const c_void, 8) {
        return None;
    }
    let tipo_fn = rd64((rd64(rec as *const u8) + 0x08) as *const u8);
    if !crate::rtti::sane(tipo_fn as *mut c_void) {
        return None;
    }
    let tipo: extern "C" fn(u64) -> u32 = std::mem::transmute(tipo_fn);
    let tipo = tipo(rec);
    let no = mapas::nos(t.add(RECORDS_POR_TIPO))?.into_iter().find(|n| (n.add(0x08) as *const u32).read_unaligned() == tipo)?;
    Some((no.add(0x1c) as *const u32).read_unaligned() as usize)
}
