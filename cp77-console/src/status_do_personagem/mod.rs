//! Status do personagem: vida, stamina, nível, pontos e invencibilidade do V (os cheats do terminal).

#![allow(dead_code)]

use crate::rtti::refcnt;
use crate::jogo::sessao::auth_or;
use crate::jogo::sessao::auth_player;
use crate::jogo::sessao::game_instance;
use crate::jogo::sessao::sistema_de_script;
use crate::jogo::sessao::sistema_de_script_ou_getter;
use crate::jogo::sessao::trunc16;
use crate::jogo::sessao::sistema_pelo_getter;
use std::ffi::c_void;
use crate::cname::cname;
use crate::rtti::{self, Arg, Registry};

/// `PlayerDevelopmentData` do player autoritativo: GetGame → PlayerDevelopmentSystem
/// → GetDevelopmentData(owner).
pub(crate) unsafe fn dev_data(reg: &Registry, captured_player: *mut c_void) -> *mut c_void {
    let owner = {
        let a = auth_player(reg, captured_player);
        if a.is_null() {
            captured_player
        } else {
            a
        }
    };
    let gi = match crate::jogo::sessao::game_instance(reg, owner) {
        Some(b) => b,
        None => return std::ptr::null_mut(),
    };
    let sys = sistema_de_script(reg, owner, gi, "PlayerDevelopmentSystem");
    if !rtti::sane(sys) {
        crate::log_dev!("[dev] PlayerDevelopmentSystem inacessível");
        return std::ptr::null_mut();
    }
    let gdd = match rtti::resolve_func(reg, "PlayerDevelopmentSystem", "GetDevelopmentData") {
        Some(g) => g,
        None => {
            crate::log_dev!("[dev] GetDevelopmentData não resolvido");
            return std::ptr::null_mut();
        }
    };
    rtti::call_ptr(&gdd, sys, &[Arg::Handle(owner, refcnt())])
}

/// Adiciona `n` pontos de desenvolvimento do tipo `member`
/// (gamedataDevelopmentPointType: "Attribute"=attrs, "Primary"=perks, "Espionage"=relic).
pub unsafe fn add_points(reg: &Registry, captured_player: *mut c_void, n: u32, member: &str) -> bool {
    let dd = dev_data(reg, captured_player);
    if !rtti::sane(dd) {
        crate::log_dev!("[points] sem PlayerDevelopmentData");
        return false;
    }
    let adp = match rtti::resolve_func(reg, "PlayerDevelopmentData", "AddDevelopmentPoints") {
        Some(g) => g,
        None => {
            crate::log_dev!("[points] AddDevelopmentPoints não resolvido");
            return false;
        }
    };
    let ev = match rtti::resolve_enum_value(reg, "gamedataDevelopmentPointType", member) {
        Some(v) => v,
        None => {
            crate::log_dev!("[points] enum gamedataDevelopmentPointType::{member} não resolvido");
            return false;
        }
    };
    crate::log_dev!("[points] dd={dd:p} AddDevelopmentPoints n={n} {member}={ev}");
    rtti::call_func(&adp, dd, &[Arg::I32(n), Arg::Enum(ev)]);
    crate::log_dev!("[points] +{n} {member} enviado");
    true
}

/// entEntityID (nos 8 primeiros bytes do buffer) via `gameObject.GetEntityID`.
pub(crate) unsafe fn entity_id(reg: &Registry, player: *mut c_void) -> Option<[u8; 16]> {
    let g = rtti::resolve_any(reg, &["gameObject", "gameEntity"], "GetEntityID")?;
    rtti::call_func(&g, player, &[]).map(trunc16)
}

/// Godmode REAL via `gameGodModeType::Invulnerable` (não toma dano — vence o
/// "Immortal" que só evita morte). `AddGodMode(eid, type, n"BWMS")`.
pub unsafe fn godmode(reg: &Registry, captured_player: *mut c_void, on: bool) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match game_instance(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sys = sistema_pelo_getter(reg, owner, gi, "GetGodModeSystem");
    if !rtti::sane(sys) {
        crate::log_dev!("[god] GetGodModeSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => {
            crate::log_dev!("[god] GetEntityID falhou");
            return false;
        }
    };
    let fname = if on { "AddGodMode" } else { "RemoveGodMode" };
    let f = match rtti::resolve_any(reg, &["gameGodModeSystem"], fname) {
        Some(g) => g,
        None => {
            crate::log_dev!("[god] {fname} não resolvido");
            return false;
        }
    };
    // Invulnerable primeiro (dano zero); Immortal/Default como fallback.
    for mem in ["Invulnerable", "Immortal", "Default"] {
        if let Some(ev) = rtti::resolve_enum_value(reg, "gameGodModeType", mem) {
            crate::log_dev!("[god] {fname}({mem}={ev})");
            // Mesma fonte (`sourceInfo`) do menu MODS, do painel de Configurações e do restore: o GodModeSystem conta por fonte, e
            // remover com outra fonte não tira o God Mode posto por um caminho diferente. Desligar tira também as fontes de até a 0.1.5
            // (`BlackwallMods`, `Console`), que saves antigos trazem porque o jogo grava o GodModeSystem no save (`BwmsGodModeDesligar`).
            let fontes: &[&str] = if on { &["BWMS"] } else { &["BWMS", "BlackwallMods", "Console"] };
            for fonte in fontes {
                rtti::call_func(&f, sys, &[Arg::Raw(eid), Arg::Enum(ev), Arg::CName(cname(fonte))]);
            }
            // A chave `godmode` da config externa é a que o `BwmsAoEntrarNoJogo` lê pra religar na carga: o terminal grava igual ao menu
            // MODS e à aba Cheats, senão um OFF daqui volta ligado no próximo save carregado e o jogo o grava no save de novo.
            crate::configuracao::modconfig_set("godmode", if on { "1" } else { "0" });
            crate::log_dev!("[god] godmode {} ({mem}) enviado", if on { "ON" } else { "OFF" });
            return true;
        }
    }
    crate::log_dev!("[god] nenhum membro de gameGodModeType resolveu");
    false
}

/// `redscript-cheat-effects-proof` (2026-07-13) — checagem READ-ONLY de `HasGodMode` (mesma
/// chamada que `blackwall-mods.reds::BWHasGodMode` faz), pra provar que um toggle disparado
/// PELO CAMINHO .reds (não pelo `godmode()` acima, que é só console) teve efeito real e
/// observável. Não muta nada — só lê e loga.
pub unsafe fn hasgod(reg: &Registry, captured_player: *mut c_void) -> Option<bool> {
    let owner = auth_or(reg, captured_player);
    let gi = game_instance(reg, owner)?;
    let sys = sistema_pelo_getter(reg, owner, gi, "GetGodModeSystem");
    if !rtti::sane(sys) {
        crate::log_dev!("[hasgod] GetGodModeSystem inacessível");
        return None;
    }
    let eid = entity_id(reg, owner)?;
    let f = rtti::resolve_any(reg, &["gameGodModeSystem"], "HasGodMode")?;
    for mem in ["Invulnerable", "Immortal", "Default"] {
        if let Some(ev) = rtti::resolve_enum_value(reg, "gameGodModeType", mem) {
            let r = rtti::call_func(&f, sys, &[Arg::Raw(eid), Arg::Enum(ev)]);
            let has = r.map(|b| rtti::bool_do_retorno(&b));
            // Quantas entradas (uma por AddGodMode): separa "ligado 2× pela mesma fonte" de "ligado por outra fonte".
            #[cfg(feature = "devlog")]
            {
                let n = rtti::resolve_any(reg, &["gameGodModeSystem"], "GetGodModeCount")
                    .and_then(|g| rtti::call_func(&g, sys, &[Arg::Raw(eid), Arg::Enum(ev)]))
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                // `GetGodModeSources` devolve array<CName> {entries, cap, size}; o buffer devolvido fica (diagnóstico de dev, 1 por comando).
                let fontes: Vec<String> = rtti::resolve_any(reg, &["gameGodModeSystem"], "GetGodModeSources")
                    .and_then(|g| rtti::call_func(&g, sys, &[Arg::Raw(eid), Arg::Enum(ev)]))
                    .map(|b| {
                        let ent = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as *const u64;
                        let tam = u32::from_le_bytes([b[12], b[13], b[14], b[15]]) as usize;
                        if ent.is_null() || tam > 64 || !crate::memoria::is_readable(ent as *const c_void, tam * 8) {
                            return Vec::new();
                        }
                        (0..tam).map(|i| crate::cname::resolve_cname(ent.add(i).read_unaligned())).collect()
                    })
                    .unwrap_or_default();
                crate::log_dev!("[hasgod] HasGodMode({mem}) = {has:?}, GetGodModeCount = {n:?}, fontes {fontes:?}");
            }
            return has;
        }
    }
    None
}

/// Seta o nível via `PlayerDevelopmentData.SetLevel(Level, n, reason=0, true)`.
pub unsafe fn level(reg: &Registry, captured_player: *mut c_void, n: u32) -> bool {
    let dd = dev_data(reg, captured_player);
    if !rtti::sane(dd) {
        crate::log_dev!("[level] sem PlayerDevelopmentData");
        return false;
    }
    let sl = match rtti::resolve_any(reg, &["PlayerDevelopmentData"], "SetLevel") {
        Some(g) => g,
        None => {
            crate::log_dev!("[level] SetLevel não resolvido");
            return false;
        }
    };
    let prof = match rtti::resolve_enum_value(reg, "gamedataProficiencyType", "Level") {
        Some(v) => v,
        None => {
            crate::log_dev!("[level] enum gamedataProficiencyType::Level não resolvido");
            return false;
        }
    };
    crate::log_dev!("[level] SetLevel(Level={prof}, {n})");
    rtti::call_func(
        &sl,
        dd,
        &[Arg::Enum(prof), Arg::I32(n), Arg::Enum(0), Arg::Bool(true)],
    );
    crate::log_dev!("[level] level {n} enviado");
    true
}

/// Cura a Health pro máximo via `gameStatPoolsSystem.RequestSettingStatPoolValue`.
pub unsafe fn heal(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match game_instance(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sps = sistema_de_script_ou_getter(reg, owner, gi, "gameStatPoolsSystem", "GetStatPoolsSystem");
    if !rtti::sane(sps) {
        crate::log_dev!("[heal] StatPoolsSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let rs = match rtti::resolve_any(reg, &["gameStatPoolsSystem"], "RequestSettingStatPoolValue") {
        Some(g) => g,
        None => {
            crate::log_dev!("[heal] RequestSettingStatPoolValue não resolvido");
            return false;
        }
    };
    let hp = match rtti::resolve_enum_value(reg, "gamedataStatPoolType", "Health") {
        Some(v) => v,
        None => {
            crate::log_dev!("[heal] enum gamedataStatPoolType::Health não resolvido");
            return false;
        }
    };
    // (gameStatsObjectID eid, gamedataStatPoolType Health, Float 100, source null16, Bool, Bool)
    rtti::call_func(
        &rs,
        sps,
        &[
            Arg::Raw(eid),
            Arg::Enum(hp),
            Arg::F32(100.0),
            Arg::Raw([0u8; 16]),
            Arg::Bool(false),
            Arg::Bool(false),
        ],
    );
    crate::log_dev!("[heal] Health=100 enviado");
    true
}

/// DIAGNÓSTICO read-only (2026-08-06): lê Health/Stamina/Sprint (StaminaRegen/estado) direto do
/// save real via `GetStatPoolValue` (nunca escreve nada) — pra investigar relato do usuário de
/// "não consigo mais correr segurando Shift depois de mexer nos últimos gaps". Não modifica
/// nenhum estado do jogo, só lê e loga.
#[cfg(feature = "devtools")]
pub unsafe fn staminacheck(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match game_instance(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sps = sistema_de_script_ou_getter(reg, owner, gi, "gameStatPoolsSystem", "GetStatPoolsSystem");
    if !rtti::sane(sps) {
        crate::log_dev!("[staminacheck] StatPoolsSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let gv = match rtti::resolve_any(reg, &["gameStatPoolsSystem"], "GetStatPoolValue") {
        Some(g) => g,
        None => {
            crate::log_dev!("[staminacheck] GetStatPoolValue não resolvido");
            return false;
        }
    };
    for (label, enum_name) in [("Stamina", "Stamina"), ("Health", "Health")] {
        let Some(v) = rtti::resolve_enum_value(reg, "gamedataStatPoolType", enum_name) else {
            crate::log_dev!("[staminacheck] enum gamedataStatPoolType::{enum_name} não resolvido");
            continue;
        };
        let abs = rtti::call_func(&gv, sps, &[Arg::Raw(eid), Arg::Enum(v), Arg::Bool(false)])
            .map(|r| f32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        let pct = rtti::call_func(&gv, sps, &[Arg::Raw(eid), Arg::Enum(v), Arg::Bool(true)])
            .map(|r| f32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        crate::log_dev!("[staminacheck] {label}: abs={abs:?} pct={pct:?}");
    }
    true
}

