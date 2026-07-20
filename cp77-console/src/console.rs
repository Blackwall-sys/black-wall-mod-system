//! console.rs — comandos give/money. O player capturado pela sonda costuma ser um
//! "puppet transiente" (o CP2077 tem vários PlayerPuppet: preview de menu, photo
//! mode…), e dar item nele é no-op silencioso. Então obtemos o player
//! AUTORITATIVO pela cadeia `PlayerPuppet.GetGame → GameInstance.GetPlayerSystem →
//! gamePlayerSystem.GetLocalPlayerControlledGameObject` (a receita que funciona).

use std::ffi::c_void;

use crate::cname::cname;
use crate::rtti::{self, Arg, Registry};

/// `call_func` devolve até 0x20 bytes (alargado p/ caber `String` de retorno — ver nota em
/// `rtti::call_func`), mas GameInstance/EntityID são valores de 16 bytes de verdade — trunca
/// os bytes extras (sempre lixo/zero pra esses tipos) antes de passar pra `Arg::Raw`/funções
/// que ainda modelam o tipo como `[u8;16]` fixo.
#[inline]
fn trunc16(v: [u8; 0x20]) -> [u8; 16] {
    let mut o = [0u8; 16];
    o.copy_from_slice(&v[..16]);
    o
}

/// Refcount fake alto (não libera o handle) — sentinela 0x00100000_00100000 dele.
pub(crate) unsafe fn refcnt() -> *mut c_void {
    static mut REFCNT: u64 = 0x0010_0000_0010_0000;
    std::ptr::addr_of_mut!(REFCNT) as *mut c_void
}

/// Player autoritativo (dono real do inventário) a partir de QUALQUER player
/// capturado (mesmo transiente serve de semente pro GetGame). Null se a cadeia
/// quebrar (o caller faz fallback pro capturado).
unsafe fn auth_player(reg: &Registry, captured: *mut c_void) -> *mut c_void {
    // 1. GetGame no player capturado → GameInstance (buffer de 16B).
    let gg = match rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
        Some(g) => g,
        None => {
            crate::log("[auth] PlayerPuppet.GetGame não resolvido");
            return std::ptr::null_mut();
        }
    };
    let gi = match rtti::call_func(&gg, captured, &[]) {
        Some(b) => trunc16(b),
        None => {
            crate::log("[auth] GetGame falhou");
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
            crate::log("[auth] GetPlayerSystem não resolvido");
            return std::ptr::null_mut();
        }
    };
    let ps = rtti::call_ptr(&getps, captured, &[Arg::Raw(gi)]);
    if !rtti::sane(ps) {
        crate::log(&format!("[auth] PlayerSystem inválido ({ps:p})"));
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
                    crate::log(&format!("[auth] player autoritativo via {cls}.{mn} = {o:p}"));
                    return o;
                }
            }
        }
    }
    crate::log("[auth] nenhum getter de local-player resolveu");
    std::ptr::null_mut()
}

/// Dá `qty` do item `name` no inventário do player, via `GiveItem`.
/// Retorna o buffer de resultado (res[0] = Bool de sucesso do GiveItem).
pub unsafe fn give(
    reg: &Registry,
    captured_player: *mut c_void,
    tx: *mut c_void,
    name: &str,
    qty: u32,
) -> Option<[u8; 0x20]> {
    let owner = {
        let a = auth_player(reg, captured_player);
        if a.is_null() {
            crate::log("[give] auth_player falhou; fallback pro player capturado");
            captured_player
        } else {
            a
        }
    };
    let gi = match rtti::resolve_func(reg, "gameTransactionSystem", "GiveItem") {
        Some(g) => g,
        None => {
            crate::log("[give] GiveItem NÃO resolvido");
            return None;
        }
    };
    crate::log(&format!(
        "[give] GiveItem func={:p} params={} static={}",
        gi.func,
        rtti::param_count(&gi),
        gi.is_static
    ));
    let item = rtti::from_tdbid(reg, name)?;
    crate::log(&format!(
        "[give] ctx(tx)={tx:p} owner={owner:p} (capturado={captured_player:p}) qty={qty}"
    ));
    let r = rtti::call_func(
        &gi,
        tx,
        &[Arg::Handle(owner, refcnt()), Arg::Item16(item), Arg::I32(qty)],
    );
    crate::log(&format!("[give] call_func -> {:02x?}", r));
    r
}

/// Sistema scriptável pela via da VM: `GameInstance.GetScriptableSystemsContainer(gi).Get(CName)`.
unsafe fn scriptable_system(
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
            crate::log("[sys] GetScriptableSystemsContainer não resolvido");
            return std::ptr::null_mut();
        }
    };
    let cont = rtti::call_ptr(&gsc, owner, &[Arg::Raw(gi)]);
    if !rtti::sane(cont) {
        crate::log(&format!("[sys] container inválido ({cont:p})"));
        return std::ptr::null_mut();
    }
    let get = match rtti::resolve_any(
        reg,
        &["ScriptableSystemsContainer", "gameScriptableSystemsContainer"],
        "Get",
    ) {
        Some(g) => g,
        None => {
            crate::log("[sys] container.Get não resolvido");
            return std::ptr::null_mut();
        }
    };
    rtti::call_ptr(&get, cont, &[Arg::CName(cname(sys_name))])
}

/// `PlayerDevelopmentData` do player autoritativo: GetGame → PlayerDevelopmentSystem
/// → GetDevelopmentData(owner).
unsafe fn dev_data(reg: &Registry, captured_player: *mut c_void) -> *mut c_void {
    let owner = {
        let a = auth_player(reg, captured_player);
        if a.is_null() {
            captured_player
        } else {
            a
        }
    };
    let gg = match rtti::resolve_func(reg, "PlayerPuppet", "GetGame") {
        Some(g) => g,
        None => return std::ptr::null_mut(),
    };
    let gi = match rtti::call_func(&gg, owner, &[]) {
        Some(b) => trunc16(b),
        None => return std::ptr::null_mut(),
    };
    let sys = scriptable_system(reg, owner, gi, "PlayerDevelopmentSystem");
    if !rtti::sane(sys) {
        crate::log("[dev] PlayerDevelopmentSystem inacessível");
        return std::ptr::null_mut();
    }
    let gdd = match rtti::resolve_func(reg, "PlayerDevelopmentSystem", "GetDevelopmentData") {
        Some(g) => g,
        None => {
            crate::log("[dev] GetDevelopmentData não resolvido");
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
        crate::log("[points] sem PlayerDevelopmentData");
        return false;
    }
    let adp = match rtti::resolve_func(reg, "PlayerDevelopmentData", "AddDevelopmentPoints") {
        Some(g) => g,
        None => {
            crate::log("[points] AddDevelopmentPoints não resolvido");
            return false;
        }
    };
    let ev = match rtti::resolve_enum_value(reg, "gamedataDevelopmentPointType", member) {
        Some(v) => v,
        None => {
            crate::log(&format!("[points] enum gamedataDevelopmentPointType::{member} não resolvido"));
            return false;
        }
    };
    crate::log(&format!("[points] dd={dd:p} AddDevelopmentPoints n={n} {member}={ev}"));
    rtti::call_func(&adp, dd, &[Arg::I32(n), Arg::Enum(ev)]);
    crate::log(&format!("[points] +{n} {member} enviado"));
    true
}

/// Game.GetSingleton(nome): resolve um sistema SCRIPTÁVEL por nome via
/// GetScriptableSystemsContainer(gi).Get(CName). Retorna o ptr (vira Handle no Lua).
/// (Sistemas de engine via getter estático — ex. GetTransactionSystem — = futuro.)
pub(crate) unsafe fn get_singleton(
    reg: &Registry,
    captured_player: *mut c_void,
    sys_name: &str,
) -> *mut c_void {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(g) => g,
        None => return std::ptr::null_mut(),
    };
    scriptable_system(reg, owner, gi, sys_name)
}

// TweakDB READ: a via in-game (gamedataTweakDBInterface.GetFloat com ctx=null) TRAVA
// o jogo (call_func estático entra em loop). Substituída pelo bake offline:
// `crate::tweakdb_bake::lookup` lê `$HOME/.blackwall_tweakdb.bin` (gerado por
// `tweakdb-tool bake`), sem chamar o jogo.

// ---- helpers compartilhados (player autoritativo, GameInstance, sistemas) ----

unsafe fn auth_or(reg: &Registry, captured: *mut c_void) -> *mut c_void {
    let a = auth_player(reg, captured);
    if a.is_null() {
        captured
    } else {
        a
    }
}

/// GameInstance (16B) via `PlayerPuppet.GetGame`.
unsafe fn get_gi(reg: &Registry, owner: *mut c_void) -> Option<[u8; 16]> {
    let gg = rtti::resolve_func(reg, "PlayerPuppet", "GetGame")?;
    rtti::call_func(&gg, owner, &[]).map(trunc16)
}

/// Sistema/facility via getter estático `GameInstance.GetXxx(gi)` (GetGodModeSystem etc.).
/// Tenta cada classe-alias × o gi cru de 16B E 8B (metade alta zerada), como o
/// getViaGetter dele — alguns getters só aceitam um dos tamanhos.
unsafe fn via_getter(reg: &Registry, owner: *mut c_void, gi: [u8; 16], getter: &str) -> *mut c_void {
    let mut gi8 = [0u8; 16];
    gi8[..8].copy_from_slice(&gi[..8]);
    for cls in ["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"] {
        if let Some(g) = rtti::resolve_func(reg, cls, getter) {
            for (tag, raw) in [("16", gi), ("8", gi8)] {
                let p = rtti::call_ptr(&g, owner, &[Arg::Raw(raw)]);
                crate::log(&format!("[getter] {cls}.{getter}(gi{tag}) -> {p:p}"));
                if rtti::sane(p) {
                    return p;
                }
            }
        }
    }
    crate::log(&format!("[getter] {getter} não resolveu / retornou nulo"));
    std::ptr::null_mut()
}

/// Sistema pelo container scriptável OU por getter estático (fallback).
unsafe fn system_flex(
    reg: &Registry,
    owner: *mut c_void,
    gi: [u8; 16],
    script_name: &str,
    getter: &str,
) -> *mut c_void {
    let s = scriptable_system(reg, owner, gi, script_name);
    if rtti::sane(s) {
        return s;
    }
    via_getter(reg, owner, gi, getter)
}

/// entEntityID (nos 8 primeiros bytes do buffer) via `gameObject.GetEntityID`.
unsafe fn entity_id(reg: &Registry, player: *mut c_void) -> Option<[u8; 16]> {
    let g = rtti::resolve_any(reg, &["gameObject", "gameEntity"], "GetEntityID")?;
    rtti::call_func(&g, player, &[]).map(trunc16)
}

/// Godmode REAL via `gameGodModeType::Invulnerable` (não toma dano — vence o
/// "Immortal" que só evita morte). `AddGodMode(eid, type, CName 'Console')`.
pub unsafe fn godmode(reg: &Registry, captured_player: *mut c_void, on: bool) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sys = via_getter(reg, owner, gi, "GetGodModeSystem");
    if !rtti::sane(sys) {
        crate::log("[god] GetGodModeSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => {
            crate::log("[god] GetEntityID falhou");
            return false;
        }
    };
    let fname = if on { "AddGodMode" } else { "RemoveGodMode" };
    let f = match rtti::resolve_any(reg, &["gameGodModeSystem"], fname) {
        Some(g) => g,
        None => {
            crate::log(&format!("[god] {fname} não resolvido"));
            return false;
        }
    };
    // Invulnerable primeiro (dano zero); Immortal/Default como fallback.
    for mem in ["Invulnerable", "Immortal", "Default"] {
        if let Some(ev) = rtti::resolve_enum_value(reg, "gameGodModeType", mem) {
            crate::log(&format!("[god] {fname}({mem}={ev})"));
            rtti::call_func(
                &f,
                sys,
                &[Arg::Raw(eid), Arg::Enum(ev), Arg::CName(cname("Console"))],
            );
            crate::log(&format!("[god] godmode {} ({mem}) enviado", if on { "ON" } else { "OFF" }));
            return true;
        }
    }
    crate::log("[god] nenhum membro de gameGodModeType resolveu");
    false
}

/// `redscript-cheat-effects-proof` (2026-07-13) — checagem READ-ONLY de `HasGodMode` (mesma
/// chamada que `blackwall-mods.reds::BWHasGodMode` faz), pra provar que um toggle disparado
/// PELO CAMINHO .reds (não pelo `godmode()` acima, que é só console) teve efeito real e
/// observável. Não muta nada — só lê e loga.
pub unsafe fn hasgod(reg: &Registry, captured_player: *mut c_void) -> Option<bool> {
    let owner = auth_or(reg, captured_player);
    let gi = get_gi(reg, owner)?;
    let sys = via_getter(reg, owner, gi, "GetGodModeSystem");
    if !rtti::sane(sys) {
        crate::log("[hasgod] GetGodModeSystem inacessível");
        return None;
    }
    let eid = entity_id(reg, owner)?;
    let f = rtti::resolve_any(reg, &["gameGodModeSystem"], "HasGodMode")?;
    for mem in ["Invulnerable", "Immortal", "Default"] {
        if let Some(ev) = rtti::resolve_enum_value(reg, "gameGodModeType", mem) {
            let r = rtti::call_func(&f, sys, &[Arg::Raw(eid), Arg::Enum(ev)]);
            let has = r.map(|b| b[0] != 0);
            crate::log(&format!("[hasgod] HasGodMode({mem}) = {has:?}"));
            return has;
        }
    }
    None
}

/// Seta o nível via `PlayerDevelopmentData.SetLevel(Level, n, reason=0, true)`.
pub unsafe fn level(reg: &Registry, captured_player: *mut c_void, n: u32) -> bool {
    let dd = dev_data(reg, captured_player);
    if !rtti::sane(dd) {
        crate::log("[level] sem PlayerDevelopmentData");
        return false;
    }
    let sl = match rtti::resolve_any(reg, &["PlayerDevelopmentData"], "SetLevel") {
        Some(g) => g,
        None => {
            crate::log("[level] SetLevel não resolvido");
            return false;
        }
    };
    let prof = match rtti::resolve_enum_value(reg, "gamedataProficiencyType", "Level") {
        Some(v) => v,
        None => {
            crate::log("[level] enum gamedataProficiencyType::Level não resolvido");
            return false;
        }
    };
    crate::log(&format!("[level] SetLevel(Level={prof}, {n})"));
    rtti::call_func(
        &sl,
        dd,
        &[Arg::Enum(prof), Arg::I32(n), Arg::Enum(0), Arg::Bool(true)],
    );
    crate::log(&format!("[level] level {n} enviado"));
    true
}

/// Cura a Health pro máximo via `gameStatPoolsSystem.RequestSettingStatPoolValue`.
pub unsafe fn heal(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sps = system_flex(reg, owner, gi, "gameStatPoolsSystem", "GetStatPoolsSystem");
    if !rtti::sane(sps) {
        crate::log("[heal] StatPoolsSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let rs = match rtti::resolve_any(reg, &["gameStatPoolsSystem"], "RequestSettingStatPoolValue") {
        Some(g) => g,
        None => {
            crate::log("[heal] RequestSettingStatPoolValue não resolvido");
            return false;
        }
    };
    let hp = match rtti::resolve_enum_value(reg, "gamedataStatPoolType", "Health") {
        Some(v) => v,
        None => {
            crate::log("[heal] enum gamedataStatPoolType::Health não resolvido");
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
    crate::log("[heal] Health=100 enviado");
    true
}

/// Remove `qty` do item `name` do inventário, via `RemoveItem` (espelho do give).
pub unsafe fn remove(
    reg: &Registry,
    captured_player: *mut c_void,
    tx: *mut c_void,
    name: &str,
    qty: u32,
) -> Option<[u8; 0x20]> {
    let owner = auth_or(reg, captured_player);
    let rf = rtti::resolve_func(reg, "gameTransactionSystem", "RemoveItem")?;
    let item = rtti::from_tdbid(reg, name)?;
    let r = rtti::call_func(
        &rf,
        tx,
        &[Arg::Handle(owner, refcnt()), Arg::Item16(item), Arg::I32(qty)],
    );
    crate::log(&format!("[remove] {name} x{qty} -> {:02x?}", r));
    r
}

/// Alterna o modo de invocação do veículo (chama seu carro), via `ToggleSummonMode`.
pub unsafe fn summon(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let vs = system_flex(reg, owner, gi, "gameVehicleSystem", "GetVehicleSystem");
    if !rtti::sane(vs) {
        crate::log("[summon] VehicleSystem inacessível");
        return false;
    }
    let e = match rtti::resolve_any(reg, &["gameVehicleSystem"], "ToggleSummonMode") {
        Some(g) => g,
        None => {
            crate::log("[summon] ToggleSummonMode não resolvido");
            return false;
        }
    };
    rtti::call_func(&e, vs, &[]);
    crate::log("[summon] ToggleSummonMode enviado");
    true
}
