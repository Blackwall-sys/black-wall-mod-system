//! A sessão de jogo vista pelo BWMS: o jogador autoritativo, o GameInstance e os sistemas do jogo a partir dele (por nome de
//! script, por getter estático ou pelo mapa de sistemas). Usado por vários conjuntos.

#![allow(dead_code)]

use crate::cname::cname;
use crate::rtti::Arg;
use crate::rtti::Registry;
use crate::rtti;
use std::ffi::c_void;

/// `call_func` devolve até 0x20 bytes (alargado p/ caber `String` de retorno — ver nota em
/// `rtti::call_func`), mas GameInstance/EntityID são valores de 16 bytes de verdade — trunca
/// os bytes extras (sempre lixo/zero pra esses tipos) antes de passar pra `Arg::Raw`/funções
/// que ainda modelam o tipo como `[u8;16]` fixo.
#[inline]
pub(crate) fn trunc16(v: [u8; 0x20]) -> [u8; 16] {
    let mut o = [0u8; 16];
    o.copy_from_slice(&v[..16]);
    o
}

/// Player autoritativo (dono real do inventário) a partir de QUALQUER player
/// capturado (mesmo transiente serve de semente pro GetGame). Null se a cadeia
/// quebrar (o caller faz fallback pro capturado).
pub(crate) unsafe fn auth_player(reg: &Registry, captured: *mut c_void) -> *mut c_void {
    // 1. GetGame no player capturado → GameInstance (buffer de 16B).
    let gi = match game_instance(reg, captured) {
        Some(b) => b,
        None => {
            crate::log_dev!("[auth] GetGame falhou");
            return std::ptr::null_mut();
        }
    };
    // 2. GameInstance.GetPlayerSystem(gi) → PlayerSystem (ctx = player capturado,
    //    arg = a GameInstance crua, igual ao getViaGetter dele).
    let getps = match rtti::resolve_any(
        reg,
        &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"],
        "GetPlayerSystem",
    ) {
        Some(g) => g,
        None => {
            crate::log_dev!("[auth] GetPlayerSystem não resolvido");
            return std::ptr::null_mut();
        }
    };
    let ps = rtti::call_ptr(&getps, captured, &[Arg::Raw(gi)]);
    if !rtti::sane(ps) {
        crate::log_dev!("[auth] PlayerSystem inválido ({ps:p})");
        return std::ptr::null_mut();
    }
    // 3. PlayerSystem.GetLocalPlayer…() → player autoritativo (testa variantes).
    for cls in ["gamePlayerSystem", "cpPlayerSystem", "PlayerSystem"] {
        for mn in [
            "GetLocalPlayerControlledGameObject",
            "GetLocalPlayerMainGameObject",
            "GetLocalPlayer",
            "GetPlayerControlledGameObject",
            "GetPlayer",
        ] {
            if let Some(m) = rtti::resolve_func(reg, cls, mn) {
                let o = rtti::call_ptr(&m, ps, &[]);
                if rtti::sane(o) {
                    crate::log_dev!("[auth] player autoritativo via {cls}.{mn} = {o:p}");
                    return o;
                }
            }
        }
    }
    crate::log_dev!("[auth] nenhum getter de local-player resolveu");
    std::ptr::null_mut()
}

/// Sistema scriptável pela via da VM: `GameInstance.GetScriptableSystemsContainer(gi).Get(CName)`.
pub(crate) unsafe fn sistema_de_script(
    reg: &Registry,
    owner: *mut c_void,
    gi: [u8; 16],
    sys_name: &str,
) -> *mut c_void {
    let gsc = match rtti::resolve_any(
        reg,
        &["GameInstance", "ScriptGameInstance", "gameScriptGameInstance"],
        "GetScriptableSystemsContainer",
    ) {
        Some(g) => g,
        None => {
            crate::log_dev!("[sys] GetScriptableSystemsContainer não resolvido");
            return std::ptr::null_mut();
        }
    };
    let cont = rtti::call_ptr(&gsc, owner, &[Arg::Raw(gi)]);
    if !rtti::sane(cont) {
        crate::log_dev!("[sys] container inválido ({cont:p})");
        return std::ptr::null_mut();
    }
    let get = match rtti::resolve_any(
        reg,
        &["ScriptableSystemsContainer", "gameScriptableSystemsContainer"],
        "Get",
    ) {
        Some(g) => g,
        None => {
            crate::log_dev!("[sys] container.Get não resolvido");
            return std::ptr::null_mut();
        }
    };
    rtti::call_ptr(&get, cont, &[Arg::CName(cname(sys_name))])
}

/// Sistema scriptável por nome a partir de um player qualquer (o `Game.GetSingleton` do Lua): `sistema_de_script` com o
/// `GameInstance` do player autoritativo. Retorna o ptr (vira Handle no Lua).
pub(crate) unsafe fn sistema_de_script_do_jogador(
    reg: &Registry,
    captured_player: *mut c_void,
    sys_name: &str,
) -> *mut c_void {
    let owner = auth_or(reg, captured_player);
    let gi = match game_instance(reg, owner) {
        Some(g) => g,
        None => return std::ptr::null_mut(),
    };
    sistema_de_script(reg, owner, gi, sys_name)
}

pub(crate) unsafe fn auth_or(reg: &Registry, captured: *mut c_void) -> *mut c_void {
    let a = auth_player(reg, captured);
    if a.is_null() {
        captured
    } else {
        a
    }
}

/// O `GameInstance` (16 bytes, como o `PlayerPuppet.GetGame` devolve) a partir de um player qualquer: a fonte única do crate.
pub(crate) unsafe fn game_instance(reg: &Registry, owner: *mut c_void) -> Option<[u8; 16]> {
    let gg = rtti::resolve_func(reg, "PlayerPuppet", "GetGame")?;
    rtti::call_func(&gg, owner, &[]).map(trunc16)
}

/// Sistema de engine pelo getter estático do `GameInstance` (`GetCharacterCustomizationSystem` etc.), a partir de um player qualquer.
pub(crate) unsafe fn sistema_pelo_getter_do_jogador(reg: &Registry, captured_player: *mut c_void, getter: &str) -> *mut c_void {
    let owner = auth_or(reg, captured_player);
    match game_instance(reg, owner) {
        Some(gi) => sistema_pelo_getter(reg, owner, gi, getter),
        None => std::ptr::null_mut(),
    }
}

/// Sistema/facility via getter estático `GameInstance.GetXxx(gi)` (GetGodModeSystem etc.).
/// Tenta cada classe-alias × o gi cru de 16B E 8B (metade alta zerada), como o
/// getViaGetter dele — alguns getters só aceitam um dos tamanhos.
pub(crate) unsafe fn sistema_pelo_getter(reg: &Registry, owner: *mut c_void, gi: [u8; 16], getter: &str) -> *mut c_void {
    let mut gi8 = [0u8; 16];
    gi8[..8].copy_from_slice(&gi[..8]);
    for cls in ["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"] {
        if let Some(g) = rtti::resolve_func(reg, cls, getter) {
            for (tag, raw) in [("16", gi), ("8", gi8)] {
                let p = rtti::call_ptr(&g, owner, &[Arg::Raw(raw)]);
                crate::log_dev!("[getter] {cls}.{getter}(gi{tag}) -> {p:p}");
                if rtti::sane(p) {
                    return p;
                }
            }
        }
    }
    crate::log_dev!("[getter] {getter} não resolveu / retornou nulo");
    std::ptr::null_mut()
}

/// Sistema pelo container scriptável OU por getter estático (fallback).
pub(crate) unsafe fn sistema_de_script_ou_getter(
    reg: &Registry,
    owner: *mut c_void,
    gi: [u8; 16],
    script_name: &str,
    getter: &str,
) -> *mut c_void {
    let s = sistema_de_script(reg, owner, gi, script_name);
    if rtti::sane(s) {
        return s;
    }
    sistema_pelo_getter(reg, owner, gi, getter)
}

/// Sistema do motor no mapa de `game` pela chave do tipo (o nome que o `sysbymap` provou no mapa), se `aceita` aprovar; nulo senão.
pub(crate) unsafe fn sistema_do_motor_por_nome(reg: &Registry, game: *mut c_void, tipo: &str, aceita: impl FnOnce(*mut c_void) -> bool) -> *mut c_void {
    let sys = sistema_do_motor(game, reg.class_by_name(tipo));
    if !sys.is_null() && aceita(sys) { sys } else { std::ptr::null_mut() }
}

/// Sistema do motor no mapa do `GameInstance` da sessão pela chave do tipo; com `vtbl_vm`, só o objeto cuja vtable é a medida. Nulo fora
/// do gameplay.
pub(crate) unsafe fn sistema_da_sessao(tipo: &str, vtbl_vm: Option<u64>) -> *mut c_void {
    match (crate::registry(), game_instance_ptr()) {
        (Some(reg), Some(game)) => sistema_do_motor_por_nome(reg, game, tipo, |sys| sistema_com_vtable(sys, vtbl_vm)),
        _ => std::ptr::null_mut(),
    }
}

/// Objeto plausível e, com `vtbl_vm`, com a vtable medida.
pub(crate) unsafe fn sistema_com_vtable(sys: *mut c_void, vtbl_vm: Option<u64>) -> bool {
    rtti::sane(sys) && vtbl_vm.is_none_or(|vt| (sys as *const u64).read_unaligned() == crate::rebase(vt) as u64)
}


/// Ponteiro do `GameInstance` (1ª palavra do que `PlayerPuppet.GetGame` devolve), na game thread.
pub(crate) unsafe fn game_instance_ptr() -> Option<*mut c_void> {
    let player = crate::current_player();
    if player.is_null() {
        return None;
    }
    let gi = game_instance(crate::registry()?, player)?;
    let g = u64::from_le_bytes(gi[..8].try_into().ok()?) as *mut c_void;
    rtti::sane(g).then_some(g)
}


/// Sistema do motor pelo tipo nativo da implementação (`IType*`, ex. `cpPlayerSystem`, `gameCommunitySystem`): o `systemMap` do
/// `GameInstance` (`+0x08`) é um `HashMap` do motor, e a busca é a do `jogo::mapas::no`. O valor do nó é um `Handle`; devolve a instância.
/// Nulo com `game`/`tipo` implausíveis, cabeçalho ilegível ou tipo ausente — nunca lê um `game` que não passou no `sane`.
pub(crate) unsafe fn sistema_do_motor(game: *mut c_void, tipo: *mut c_void) -> *mut c_void {
    if !rtti::sane(game) || !rtti::sane(tipo) {
        return std::ptr::null_mut();
    }
    let mapa = (game as *const u8).add(0x08);
    if !crate::memoria::is_readable(mapa as *const c_void, 0x24) {
        return std::ptr::null_mut();
    }
    match crate::jogo::mapas::no(mapa, tipo as u64) {
        Some(n) => (n.add(0x10) as *const u64).read_unaligned() as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GameInstance` falso com um `systemMap` de verdade na forma do motor: cabeçalho em `+0x08`, tabela, nós de 0x20
    /// (`next`, hash, chave, valor). Devolve `(memória, game)`; a memória vive enquanto o teste usa o `game`.
    fn game_falso(chaves: &[(u64, u64)], cap: usize) -> (Vec<u64>, *mut c_void) {
        let passo = 0x20usize;
        let cap_nos = chaves.len().max(1);
        let palavras = (0x40 + cap * 4 + cap_nos * passo) / 8 + 8;
        let mut mem = vec![0u64; palavras];
        let base = mem.as_mut_ptr() as *mut u8;
        unsafe {
            let tabela = base.add(0x40) as *mut u32;
            let nos = base.add(0x40 + cap * 4 + 4) as *mut u8;
            for b in 0..cap {
                tabela.add(b).write_unaligned(u32::MAX);
            }
            for (i, (chave, valor)) in chaves.iter().enumerate() {
                let n = nos.add(i * passo);
                let h = crate::jogo::mapas::hash_da_chave(*chave);
                let balde = tabela.add(h as usize % cap);
                (n as *mut u32).write_unaligned(balde.read_unaligned());
                (n.add(4) as *mut u32).write_unaligned(h);
                (n.add(8) as *mut u64).write_unaligned(*chave);
                (n.add(0x10) as *mut u64).write_unaligned(*valor);
                balde.write_unaligned(i as u32);
            }
            let mapa = base.add(0x08);
            (mapa as *mut u64).write_unaligned(tabela as u64);
            (mapa.add(0x08) as *mut u32).write_unaligned(chaves.len() as u32);
            (mapa.add(0x0c) as *mut u32).write_unaligned(cap as u32);
            (mapa.add(0x10) as *mut u64).write_unaligned(nos as u64);
            (mapa.add(0x18) as *mut u32).write_unaligned(cap_nos.max(cap) as u32);
            (mapa.add(0x1c) as *mut u32).write_unaligned(passo as u32);
        }
        (mem, base as *mut c_void)
    }

    /// Ponteiros plausíveis pro `sane` (acima da página zero, abaixo do fim do espaço de usuário) que servem de chave.
    fn tipo(i: u64) -> u64 {
        0x1_2000_0000 + i * 0x40
    }

    #[test]
    fn sistema_do_motor_acha_cada_chave() {
        // 159 baldes como o systemMap real, com colisão forçada: 200 chaves em 159 baldes dão cadeias longas.
        let chaves: Vec<(u64, u64)> = (0..200).map(|i| (tipo(i), 0x7_0000_0000 + i)).collect();
        let (_mem, game) = game_falso(&chaves, 159);
        unsafe {
            for (k, v) in &chaves {
                let novo = sistema_do_motor(game, *k as *mut c_void);
                assert_eq!(novo as u64, *v);
            }
            // Chave ausente, balde vazio, game nulo e game implausível: nulo, sem crash.
            let ausente = tipo(10_000) as *mut c_void;
            assert!(sistema_do_motor(game, ausente).is_null());
            assert!(sistema_do_motor(std::ptr::null_mut(), tipo(0) as *mut c_void).is_null());
            assert!(sistema_do_motor(0x10 as *mut c_void, tipo(0) as *mut c_void).is_null());
            assert!(sistema_do_motor(game, std::ptr::null_mut()).is_null());
        }
        let (_vazio, g0) = game_falso(&[], 7);
        unsafe {
            assert!(sistema_do_motor(g0, tipo(1) as *mut c_void).is_null());
        }
    }
}
