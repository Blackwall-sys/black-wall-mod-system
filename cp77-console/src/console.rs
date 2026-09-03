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

/// `give` sem log e com o `gameItemID` em cache. Existe pro caminho do TICK: `give` loga cinco
/// linhas por chamada, o que afogaria o console em segundos, e `from_tdbid` é uma chamada RTTI
/// que não precisa se repetir — o ItemID de um nome não muda durante a sessão.
unsafe fn give_quiet(reg: &Registry, captured_player: *mut c_void, tx: *mut c_void, name: &str, qty: u32) {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Vec<(String, [u8; 16])>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let item = {
        let mut g = cache.lock().unwrap();
        match g.iter().find(|(n, _)| n == name) {
            Some((_, it)) => *it,
            None => match rtti::from_tdbid(reg, name) {
                Some(it) => {
                    g.push((name.to_string(), it));
                    it
                }
                None => return,
            },
        }
    };
    let owner = auth_or(reg, captured_player);
    // `resolve_func` varre a tabela de funções da classe e a cadeia de pais A CADA chamada. No
    // caminho do tick isso se repetiria quatro vezes por disparo sem nunca mudar de resposta.
    static GIVE: std::sync::OnceLock<(usize, usize, bool)> = std::sync::OnceLock::new();
    let cached = GIVE.get_or_init(|| match rtti::resolve_func(reg, "gameTransactionSystem", "GiveItem") {
        Some(g) => (g.func as usize, g.ret_type as usize, g.is_static),
        None => (0, 0, false),
    });
    if cached.0 == 0 {
        return;
    }
    let gi = rtti::ResolvedFn {
        func: cached.0 as *mut c_void,
        ret_type: cached.1 as *mut c_void,
        is_static: cached.2,
    };
    rtti::call_func(&gi, tx, &[Arg::Handle(owner, refcnt()), Arg::Item16(item), Arg::I32(qty)]);
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
    // Os DOIS tipos, não o primeiro que resolver: `Invulnerable` é "não toma dano" e `Immortal`
    // é "não morre". Parar no primeiro deixava um buraco de semântica — e, se o primeiro fosse o
    // membro errado, o comando dizia ON tendo mandado outra coisa.
    let mut sent = 0;
    for mem in ["Invulnerable", "Immortal"] {
        if let Some(ev) = rtti::resolve_enum_value(reg, "gameGodModeType", mem) {
            crate::log(&format!("[god] {fname}({mem}={ev})"));
            rtti::call_func(
                &f,
                sys,
                &[Arg::Raw(eid), Arg::Enum(ev), Arg::CName(cname("Console"))],
            );
            sent += 1;
        }
    }
    if sent == 0 {
        crate::log("[god] nenhum membro de gameGodModeType resolveu");
        return false;
    }
    // O jogo responde: `HasGodMode` é a mesma leitura que o menu ESC usa pro rótulo.
    crate::log(&format!(
        "[god] {} enviado ({sent} tipo(s)) — jogo confirma: {}",
        if on { "ON" } else { "OFF" },
        match hasgod(reg, captured_player) {
            Some(true) => "SIM",
            Some(false) => "nao",
            None => "?(HasGodMode não resolveu)",
        }
    ));
    true
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
/// Escreve um StatPool do jogador (Health/Stamina/Memory) via
/// `gameStatPoolsSystem::RequestSettingStatPoolValue`. Base compartilhada de `heal` e `ram`:
/// os três pools usam a MESMA chamada, só muda o valor do enum `gamedataStatPoolType`.
unsafe fn set_pool(reg: &Registry, captured_player: *mut c_void, pool: &str, value: f32, tag: &str) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sps = system_flex(reg, owner, gi, "gameStatPoolsSystem", "GetStatPoolsSystem");
    if !rtti::sane(sps) {
        crate::log(&format!("[{tag}] StatPoolsSystem inacessível"));
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let rs = match rtti::resolve_any(reg, &["gameStatPoolsSystem"], "RequestSettingStatPoolValue") {
        Some(g) => g,
        None => {
            crate::log(&format!("[{tag}] RequestSettingStatPoolValue não resolvido"));
            return false;
        }
    };
    let pv = match rtti::resolve_enum_value(reg, "gamedataStatPoolType", pool) {
        Some(v) => v,
        None => {
            crate::log(&format!("[{tag}] enum gamedataStatPoolType::{pool} não resolvido"));
            return false;
        }
    };
    // (gameStatsObjectID eid, gamedataStatPoolType pool, Float value, source null16, Bool, Bool)
    rtti::call_func(
        &rs,
        sps,
        &[
            Arg::Raw(eid),
            Arg::Enum(pv),
            Arg::F32(value),
            Arg::Raw([0u8; 16]),
            Arg::Bool(false),
            Arg::Bool(false),
        ],
    );
    true
}

pub unsafe fn heal(reg: &Registry, captured_player: *mut c_void) -> bool {
    let ok = set_pool(reg, captured_player, "Health", 100.0, "heal");
    if ok {
        crate::log("[heal] Health=100 enviado");
    }
    ok
}

/// "NPCs não te veem": aplica o efeito de camuflagem ÓPTICA do próprio jogo no jogador.
///
/// Cyberpunk já tem essa mecânica — o Optical Camo — e o estado dela é o status effect
/// `BaseStatusEffect.Cloaked`, VERIFICADO como record existente no tweakdb.bin deste build (o
/// TweakDBID bate na tabela de records). Usar o efeito do jogo em vez de mexer na percepção dos
/// NPCs um a um significa que a IA continua se comportando como o jogo espera: quem trata
/// "alvo camuflado" já está escrito e testado pela CDPR.
///
/// `ApplyStatusEffect(entityID, TweakDBID)` / `RemoveStatusEffect(entityID, TweakDBID)` no
/// `gameStatusEffectSystem`. O nº de params é logado: se este build pedir mais, aparece no log em
/// vez de falhar em silêncio.
/// Aplica/remove um status effect do jogo no jogador, por nome de record.
///
/// Base de `cloak` e `infinite_ammo`: os dois são o MESMO mecanismo — um record que o jogo já
/// define e cuja lógica a CDPR já escreveu. Usar o efeito pronto em vez de mexer no sistema por
/// baixo (percepção dos NPCs, contador de munição) mantém a IA e o HUD coerentes.
///
/// Loga a aridade REAL e os tipos dos params, lidos da RTTI: mandar menos argumento do que a
/// assinatura pede é a causa nº1 de "o comando diz ON e nada acontece", e isso vira um número na
/// tela em vez de suposição.
pub unsafe fn status_effect(
    reg: &Registry,
    captured_player: *mut c_void,
    effect: &str,
    on: bool,
    tag: &str,
) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let ses = system_flex(reg, owner, gi, "gameStatusEffectSystem", "GetStatusEffectSystem");
    if !rtti::sane(ses) {
        crate::log(&format!("[{tag}] StatusEffectSystem inacessível"));
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => {
            crate::log(&format!("[{tag}] GetEntityID falhou"));
            return false;
        }
    };
    let fname = if on { "ApplyStatusEffect" } else { "RemoveStatusEffect" };
    let f = match rtti::resolve_any(reg, &["gameStatusEffectSystem"], fname) {
        Some(g) => g,
        None => {
            crate::log(&format!("[{tag}] {fname} não resolvido"));
            return false;
        }
    };
    let tdbid = crate::cname::tweak_db_id(effect).to_le_bytes();
    let np = rtti::param_count(&f) as usize;
    // O tipo de RETORNO importa pra leitura do resultado: se for Void, o `01` que líamos como
    // "deu certo" é lixo do buffer, não resposta. Logado uma vez, junto da assinatura.
    let rty = rtti::fn_ret_type(f.func);
    let rname = if rty.is_null() {
        "Void".to_string()
    } else {
        crate::cname::resolve_cname(rtti::type_name_getname(rty))
    };
    let ptypes: Vec<String> = (0..np)
        .map(|i| crate::cname::resolve_cname(rtti::fn_param_type(f.func, i)))
        .collect();
    crate::log(&format!(
        "[{tag}] {fname}({}) -> {rname} params={np} static={} '{effect}' tdbid={:#018x}",
        ptypes.join(", "),
        f.is_static,
        u64::from_le_bytes(tdbid)
    ));
    // Monta a lista com o TAMANHO pedido. Preenche por TIPO (não por posição), pra continuar
    // valendo se outra assinatura aparecer.
    //
    // `instigator`: entEntityID que NÃO seja o 1o (o 1o é o alvo). Zerado = "ninguém"; vários
    // sistemas do jogo descartam um efeito sem instigador.
    let build_args = |instigator: [u8; 16], n: usize| -> Vec<Arg> {
        let mut v: Vec<Arg> = vec![Arg::Raw(eid), Arg::Tdb(tdbid)];
        for i in v.len()..n {
            let ty = crate::cname::resolve_cname(rtti::fn_param_type(f.func, i));
            v.push(match ty.as_str() {
                "TweakDBID" => Arg::Tdb([0u8; 8]),
                "Bool" => Arg::Bool(false),
                // este Uint32 é a contagem de pilhas: 0 aplicaria "nenhuma".
                "Uint32" => Arg::I32(1),
                "Int32" => Arg::I32(0),
                "entEntityID" => Arg::Raw(instigator),
                _ => Arg::Raw([0u8; 16]), // Vector4 e afins: 16 bytes zerados
            });
        }
        v
    };
    // Três formas plausíveis da MESMA chamada. O valor de retorno não é prova (pode ser Void),
    // então cada uma é seguida de `HasStatusEffect` — o jogo respondendo se o efeito ESTÁ no
    // jogador. É a única leitura que separa "não aplicou" de "aplicou e o record é inerte".
    let variants: [(&str, Vec<Arg>); 3] = [
        ("assinatura completa, instigator=0", build_args([0u8; 16], np)),
        ("assinatura completa, instigator=player", build_args(eid, np)),
        ("só os 2 obrigatórios (resto omitido)", build_args([0u8; 16], np.min(2))),
    ];
    let mut landed = false;
    for (why, args) in variants {
        let argc = args.len();
        let r = rtti::call_func(&f, ses, &args);
        let has = has_effect(reg, ses, eid, tdbid);
        crate::log(&format!(
            "[{tag}] {why}: argc={argc} ret={:02x?} has={}",
            r.map(|v| v[0]),
            match has {
                Some(true) => "SIM",
                Some(false) => "nao",
                None => "?(HasStatusEffect não resolveu)",
            }
        ));
        // Ligar: parar na 1a forma que o jogo confirma. Desligar: seguir até o efeito sumir.
        if has == Some(on) {
            landed = true;
            break;
        }
    }
    // DESLIGAR: cada `RemoveStatusEffect` tira UMA pilha. Rodar `cloak` várias vezes empilhou o
    // efeito, então uma chamada só o deixa aplicado — foi o que prendeu o personagem invisível.
    // Repete até o jogo dizer que sumiu, com teto pra nunca virar laço infinito no frame.
    if !on && !landed {
        let args = build_args([0u8; 16], np);
        for i in 1..=32 {
            rtti::call_func(&f, ses, &args);
            if has_effect(reg, ses, eid, tdbid) == Some(false) {
                crate::log(&format!("[{tag}] removido após {i} pilha(s)"));
                landed = true;
                break;
            }
        }
        if !landed {
            crate::log(&format!("[{tag}] 32 remoções e o jogo ainda diz has=SIM"));
        }
    }
    crate::log(&format!(
        "[{tag}] {} — jogo confirma: {}",
        if on { "ON" } else { "OFF" },
        if landed { "SIM" } else { "NAO (nenhuma forma pegou)" }
    ));
    landed
}

/// Pergunta ao jogo se o efeito ESTÁ no jogador. `None` = não deu pra perguntar (método não
/// resolvido) — diferente de `Some(false)`, que é o jogo dizendo "não está".
unsafe fn has_effect(
    reg: &Registry,
    ses: *mut c_void,
    eid: [u8; 16],
    tdbid: [u8; 8],
) -> Option<bool> {
    let f = rtti::resolve_any(reg, &["gameStatusEffectSystem"], "HasStatusEffect")?;
    let np = rtti::param_count(&f) as usize;
    let mut v: Vec<Arg> = vec![Arg::Raw(eid), Arg::Tdb(tdbid)];
    v.truncate(np);
    let r = rtti::call_func(&f, ses, &v)?;
    Some(r[0] != 0)
}

/// Aplica/remove um efeito com UMA chamada (assinatura completa) e uma linha de log.
///
/// `status_effect` tenta três formas e mede com `HasStatusEffect`, o que é certo pra INVESTIGAR.
/// Pra os efeitos instantâneos isso é ruído: eles rodam a lógica e não deixam camada, então
/// `has` responde `nao` mesmo tendo funcionado — foi medido em jogo (`BlockTargetingPlayer`,
/// `DontShootAtMe`, `SetFriendly` deram `has=nao` e os NPCs pararam de atacar). Aqui a chamada é
/// uma só e o log diz o que foi ENVIADO, sem afirmar um resultado que esta leitura não sustenta.
unsafe fn effect_once(reg: &Registry, captured_player: *mut c_void, effect: &str, on: bool, tag: &str) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let ses = system_flex(reg, owner, gi, "gameStatusEffectSystem", "GetStatusEffectSystem");
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    if !rtti::sane(ses) {
        return false;
    }
    let fname = if on { "ApplyStatusEffect" } else { "RemoveStatusEffect" };
    let f = match rtti::resolve_any(reg, &["gameStatusEffectSystem"], fname) {
        Some(g) => g,
        None => return false,
    };
    let tdbid = crate::cname::tweak_db_id(effect).to_le_bytes();
    let np = rtti::param_count(&f) as usize;
    let mut v: Vec<Arg> = vec![Arg::Raw(eid), Arg::Tdb(tdbid)];
    for i in v.len()..np {
        let ty = crate::cname::resolve_cname(rtti::fn_param_type(f.func, i));
        v.push(match ty.as_str() {
            "TweakDBID" => Arg::Tdb([0u8; 8]),
            "Bool" => Arg::Bool(false),
            "Uint32" => Arg::I32(1),
            "Int32" => Arg::I32(0),
            _ => Arg::Raw([0u8; 16]),
        });
    }
    let ok = rtti::call_func(&f, ses, &v).is_some();
    crate::log(&format!("[{tag}] {} {effect}", if on { "aplicado:" } else { "removido:" }));
    ok
}

/// `se <record> [on|off]` do console: aplica/remove QUALQUER status effect por nome. Existe pra
/// que testar um candidato não custe recompilar e reiniciar o jogo — o ciclo caro aqui.
pub unsafe fn status_effect_named(
    reg: &Registry,
    captured_player: *mut c_void,
    effect: &str,
    on: bool,
) -> bool {
    status_effect(reg, captured_player, effect, on, "se")
}

/// `se? <record>`: só pergunta, não aplica nada.
pub unsafe fn status_effect_query(
    reg: &Registry,
    captured_player: *mut c_void,
    effect: &str,
) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let ses = system_flex(reg, owner, gi, "gameStatusEffectSystem", "GetStatusEffectSystem");
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    if !rtti::sane(ses) {
        crate::log("[se] StatusEffectSystem inacessível");
        return false;
    }
    let tdbid = crate::cname::tweak_db_id(effect).to_le_bytes();
    crate::log(&format!(
        "[se] '{effect}' has={}",
        match has_effect(reg, ses, eid, tdbid) {
            Some(true) => "SIM",
            Some(false) => "nao",
            None => "?(HasStatusEffect não resolveu)",
        }
    ));
    true
}

/// "Os NPCs te deixam em paz": três records de comportamento + o stat de visibilidade.
///
/// Limite honesto, porque o nome do comando promete mais: isto NÃO é invisibilidade. Os NPCs
/// continuam ENXERGANDO o jogador; o que muda é que param de atacar (a polícia desce do carro e
/// fica parada).
///
/// Vale registrar o que NÃO funciona, pra ninguém repetir o caminho: `BaseStatusEffect.Cloaked`
/// entra no jogador (`HasStatusEffect` responde SIM) e a percepção não muda — é o efeito do lado
/// dos NPCs (Oda/MaxTac). `OpticalCamoIsActive` aceita a escrita (0 -> 1 confirmado pelo jogo) e
/// não liga nada: o jogo LÊ esse stat, não obedece a ele. `VisibilityReduction` idem.
pub unsafe fn cloak(reg: &Registry, captured_player: *mut c_void, on: bool) -> bool {
    // Os três records vêm de MEDIÇÃO em jogo, não de leitura de nome: com eles aplicados os NPCs
    // continuam enxergando o jogador mas param de atacar, e a polícia desce do carro e fica
    // parada. `Visibility` entra junto porque é o stat de que o jogo monta a detecção.
    //
    // O que NÃO entra, e por quê: `BaseStatusEffect.Cloaked` (entra no jogador, `has=SIM`, e não
    // muda percepção — é o efeito do lado dos NPCs, Oda/MaxTac) e `OpticalCamoIsActive` (aceita a
    // escrita, 0 -> 1 confirmado pelo jogo, e não liga nada — é um stat que o jogo LÊ).
    const RECORDS: [&str; 3] = [
        "BaseStatusEffect.BlockTargetingPlayer",
        "BaseStatusEffect.DontShootAtMe",
        "BaseStatusEffect.SetFriendly",
    ];
    if on {
        for r in RECORDS {
            effect_once(reg, captured_player, r, true, "cloak");
        }
        stat_mod(reg, captured_player, "Visibility", -1000.0, StatTarget::Player, "cloak");
        crate::log("[cloak] ON (NPCs param de atacar; ainda podem ENXERGAR você)");
        true
    } else {
        for r in RECORDS {
            effect_once(reg, captured_player, r, false, "cloak");
        }
        stat_unmod(reg, captured_player, "Visibility", "cloak");
        // Herança: o `BaseStatusEffect.Cloaked` das tentativas anteriores entra de verdade e vai
        // pro SAVE junto, deixando o personagem invisível ao carregar. Desligar tem que limpá-lo.
        status_effect(reg, captured_player, "BaseStatusEffect.Cloaked", false, "cloak");
        crate::log("[cloak] OFF");
        true
    }
}

/// Munição infinita: o stat `MagazineAutoRefill` da ARMA — o carregador se reenche, então não há
/// recarga nem consumo. Fica na arma, não no jogador; trocar de arma exige rodar de novo.
///
/// `GameplayRestriction.InfiniteAmmo` foi abandonado com medição: nas TRÊS formas da chamada o
/// jogo respondeu `has=nao` — não é um record de status effect, então o sistema o descarta em
/// silêncio. Era isso que fazia o comando dizer ON e a munição continuar caindo.
pub unsafe fn infinite_ammo(reg: &Registry, captured_player: *mut c_void, on: bool) -> bool {
    if on {
        // MEDIDO em jogo: `MagazineCapacity` na arma resolve a recarga (o pente passa a durar
        // centenas de tiros). `MagazineAutoRefill` foi aplicado e confirmado pelo jogo (0 -> 1) e
        // mesmo assim não mudou nada — é um stat que o jogo LÊ, não um que aceita ordem.
        let mag = stat_mod(reg, captured_player, "MagazineCapacity", 999.0, StatTarget::Weapon, "ammo");
        // A reserva continua caindo, porque recarregar puxa dela. Repor a munição é o mesmo
        // desenho já provado da RAM infinita: o jogo não expõe "custo zero", então o que o
        // jogador percebe como munição que não acaba é a reserva sendo enchida de volta.
        crate::set_ammo_infinite(true);
        mag
    } else {
        crate::set_ammo_infinite(false);
        stat_unmod(reg, captured_player, "MagazineCapacity", "ammo")
    }
}

/// Tipos de munição deste build — os quatro confirmados na tabela de records.
pub const AMMO_TYPES: [&str; 4] = [
    "Ammo.HandgunAmmo",
    "Ammo.RifleAmmo",
    "Ammo.ShotgunAmmo",
    "Ammo.SniperRifleAmmo",
];

/// Quantidade de um item no inventário do jogador. `None` se não deu pra perguntar — e aí o
/// chamador repõe assim mesmo: é melhor uma notificação a mais do que um cheat que não repõe.
unsafe fn item_quantity(reg: &Registry, captured_player: *mut c_void, tx: *mut c_void, name: &str) -> Option<i32> {
    let f = rtti::resolve_any_name(reg, &["gameTransactionSystem"], &["GetItemQuantity"], "ammo")?;
    let item = rtti::from_tdbid(reg, name)?;
    let owner = auth_or(reg, captured_player);
    let r = rtti::call_func(&f, tx, &[Arg::Handle(owner, refcnt()), Arg::Item16(item)])?;
    Some(i32::from_le_bytes([r[0], r[1], r[2], r[3]]))
}

/// Repõe a reserva de munição. Chamada do tick enquanto `ammo on` está ligado. Silenciosa de
/// propósito: uma linha de log por tipo a cada tick afogaria o console em segundos.
pub unsafe fn refill_ammo(reg: &Registry, player: *mut c_void, tx: *mut c_void) {
    for a in AMMO_TYPES {
        // Só repõe o que FALTA. Dar munição a cada tick sem olhar quanto já existe enche a tela
        // de "picked up ..." — cada GiveItem levanta a notificação de item recebido do jogo,
        // mesmo com a reserva cheia. Perguntar a quantidade antes cala isso.
        match item_quantity(reg, player, tx, a) {
            Some(q) if q >= 100 => continue,
            _ => give_quiet(reg, player, tx, a, 500),
        }
    }
    // O modificador de pente vive na ARMA, então trocar de arma o deixaria pra trás e o cheat
    // pareceria ter desligado sozinho. Remover e reaplicar na arma ATUAL é mais simples do que
    // rastrear identidade de arma, e correto por construção. Mesma escolha feita no menu ESC.
    stat_unmod_ex(reg, player, "MagazineCapacity", "ammo", false);
    stat_mod_ex(reg, player, "MagazineCapacity", 999.0, StatTarget::Weapon, "ammo", false);
}

/// Alvo de um stat: o jogador ou a arma que ele tem na mão. São `StatsObjectID` diferentes —
/// `MagazineAutoRefill` mora na ARMA, `OpticalCamoIsActive` mora no JOGADOR.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StatTarget {
    Player,
    Weapon,
}

/// Arma ativa do jogador. Estático `GetActiveWeapon(GameObject)` — a assinatura está no bundle
/// de redscript deste build. A classe dona não está no bundle, então tenta as candidatas e loga
/// qual resolveu, em vez de fixar um palpite.
unsafe fn active_weapon(reg: &Registry, owner: *mut c_void) -> Option<*mut c_void> {
    // Nomes: o curto (caso seja nativa) e os MANGLED do bundle de redscript deste build —
    // `GetActiveWeapon;GameObject` (estático, recebe o dono) e `GetActiveWeapon;` (sem args).
    let f = rtti::resolve_any_name(
        reg,
        &["GameObject", "gameObject", "RPGManager", "gameRPGManager", "PlayerPuppet",
          "gamePuppet", "ScriptedPuppet", "gameScriptedPuppet", "WeaponObject", "gameweaponObject"],
        &["GetActiveWeapon;GameObject", "GetActiveWeapon;", "GetActiveWeapon"],
        "stat",
    )?;
    let w = if f.is_static {
        rtti::call_ptr(&f, std::ptr::null_mut(), &[Arg::Handle(owner, refcnt())])
    } else {
        rtti::call_ptr(&f, owner, &[])
    };
    if rtti::sane(w) {
        crate::log(&format!("[stat] arma ativa = {w:p}"));
        return Some(w);
    }
    crate::log("[stat] GetActiveWeapon resolveu mas devolveu nada (arma na mão?)");
    None
}

/// Lê um stat do alvo. É a MEDIÇÃO que diz se o modificador pegou — sem ela, "apliquei" é
/// palpite outra vez (foi exatamente o que custou as últimas rodadas com o status effect).
unsafe fn stat_value(
    reg: &Registry,
    sts: *mut c_void,
    target: [u8; 16],
    stype: u64,
) -> Option<f32> {
    let f = rtti::resolve_any_name(
        reg,
        &["gameStatsSystem"],
        &["GetStatValue", "GetStatValue;GameObjectStatsObjectIDgamedataStatType", "GetStatValue;GameObjectgamedataStatType"],
        "stat",
    )?;
    let r = rtti::call_func(&f, sts, &[Arg::Raw(target), Arg::Enum(stype)])?;
    Some(f32::from_le_bytes([r[0], r[1], r[2], r[3]]))
}

/// Aplica um modificador de stat — o mecanismo que o PRÓPRIO jogo usa pra ligar camuflagem
/// óptica e recarga automática, em vez de um status effect que o sistema recusa em silêncio.
///
/// POR QUE ESTE CAMINHO: a medição com `HasStatusEffect` mostrou que
/// `GameplayRestriction.InfiniteAmmo` NUNCA entra (`has=nao` nas três formas da chamada), e que
/// `BaseStatusEffect.Cloaked` entra mas não muda a percepção dos NPCs — é o efeito do lado dos
/// NPCs (Oda/Maxtac), não do jogador. O que o jogo usa pro jogador são STATS:
/// `BaseStats.OpticalCamoIsActive` e `BaseStats.MagazineAutoRefill`, ambos confirmados na tabela
/// de records deste build.
///
/// Loga o valor ANTES e DEPOIS: o jogo respondendo se o modificador pegou.
pub unsafe fn stat_mod(
    reg: &Registry,
    captured_player: *mut c_void,
    stat: &str,
    value: f32,
    target: StatTarget,
    tag: &str,
) -> bool {
    stat_mod_ex(reg, captured_player, stat, value, target, tag, true)
}

/// `verbose=false` é o caminho do TICK: mesma aplicação, sem a linha de antes/depois. Aquela
/// linha é a MEDIÇÃO — vale ouro quando se está investigando, e afoga o console se repetida de
/// segundo em segundo.
pub unsafe fn stat_mod_ex(
    reg: &Registry,
    captured_player: *mut c_void,
    stat: &str,
    value: f32,
    target: StatTarget,
    tag: &str,
    verbose: bool,
) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sts = system_flex(reg, owner, gi, "gameStatsSystem", "GetStatsSystem");
    if !rtti::sane(sts) {
        crate::log(&format!("[{tag}] StatsSystem inacessível"));
        return false;
    }
    let obj = match target {
        StatTarget::Player => owner,
        StatTarget::Weapon => match active_weapon(reg, owner) {
            Some(w) => w,
            None => return false,
        },
    };
    let tid = match entity_id(reg, obj) {
        Some(b) => b,
        None => {
            crate::log(&format!("[{tag}] GetEntityID do alvo falhou"));
            return false;
        }
    };
    let stype = match rtti::resolve_enum_value(reg, "gamedataStatType", stat) {
        Some(v) => v,
        None => {
            crate::log(&format!("[{tag}] gamedataStatType::{stat} não existe neste build"));
            return false;
        }
    };
    // As duas leituras do stat são a MEDIÇÃO, não a ação: no caminho do tick elas seriam duas
    // chamadas RTTI por segundo sem ninguém pra ler o resultado.
    let before = if verbose { stat_value(reg, sts, tid, stype) } else { None };
    // `Additive` soma ao valor base; pra um stat booleano (0/1) é o que liga.
    let mtype = match rtti::resolve_enum_value(reg, "gameStatModifierType", "Additive") {
        Some(v) => v,
        None => {
            crate::log(&format!("[{tag}] gameStatModifierType::Additive não resolveu"));
            return false;
        }
    };
    let cm = match rtti::resolve_any_name(
        reg,
        &["RPGManager", "gameRPGManager"],
        &["CreateStatModifier;gamedataStatTypegameStatModifierTypeFloat", "CreateStatModifier"],
        tag,
    ) {
        Some(f) => f,
        None => return false,
    };
    // Devolve um `ref<gameStatModifierData>` = handle de 16B {ponteiro, refcount}.
    let m = match rtti::call_func(&cm, std::ptr::null_mut(), &[Arg::Enum(stype), Arg::Enum(mtype), Arg::F32(value)]) {
        Some(b) => trunc16(b),
        None => {
            crate::log(&format!("[{tag}] CreateStatModifier falhou"));
            return false;
        }
    };
    if u64::from_le_bytes(m[..8].try_into().unwrap()) == 0 {
        crate::log(&format!("[{tag}] CreateStatModifier devolveu handle nulo"));
        return false;
    }
    let add = match rtti::resolve_any_name(reg, &["gameStatsSystem"], &["AddModifier", "AddModifier;"], tag) {
        Some(f) => f,
        None => return false,
    };
    let np = rtti::param_count(&add) as usize;
    rtti::call_func(&add, sts, &[Arg::Raw(tid), Arg::Raw(m)]);
    // Guarda o handle: desligar é `RemoveModifier(mesmo alvo, MESMO handle)`. Sem isto, o único
    // "off" possível seria empilhar um modificador negativo por cima — que zera o valor mas
    // deixa lixo acumulado no alvo a cada liga/desliga.
    applied().lock().unwrap().push((stat.to_string(), tid, m));
    if !verbose {
        return true;
    }
    let after = stat_value(reg, sts, tid, stype);
    let moved = matches!((before, after), (Some(a), Some(b)) if (b - a).abs() > 0.0001);
    crate::log(&format!(
        "[{tag}] {stat} {} {:?} -> {:?} (AddModifier params={np}) | jogo confirma: {}",
        if matches!(target, StatTarget::Weapon) { "na ARMA:" } else { "no JOGADOR:" },
        before,
        after,
        if moved { "SIM" } else { "NAO (valor não mudou)" }
    ));
    moved
}

/// Modificadores que NÓS aplicamos, pra poder removê-los depois pelo handle exato.
fn applied() -> &'static std::sync::Mutex<Vec<(String, [u8; 16], [u8; 16])>> {
    static A: std::sync::OnceLock<std::sync::Mutex<Vec<(String, [u8; 16], [u8; 16])>>> =
        std::sync::OnceLock::new();
    A.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Remove os modificadores que aplicamos pra este stat. Só os nossos: um `RemoveModifier` com
/// handle de terceiros mexeria em buff de perk/cyberware do jogador.
pub unsafe fn stat_unmod(reg: &Registry, captured_player: *mut c_void, stat: &str, tag: &str) -> bool {
    stat_unmod_ex(reg, captured_player, stat, tag, true)
}

pub unsafe fn stat_unmod_ex(reg: &Registry, captured_player: *mut c_void, stat: &str, tag: &str, verbose: bool) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sts = system_flex(reg, owner, gi, "gameStatsSystem", "GetStatsSystem");
    if !rtti::sane(sts) {
        crate::log(&format!("[{tag}] StatsSystem inacessível"));
        return false;
    }
    let rm = match rtti::resolve_any_name(reg, &["gameStatsSystem"], &["RemoveModifier", "RemoveModifier;"], tag) {
        Some(f) => f,
        None => return false,
    };
    let mut guard = applied().lock().unwrap();
    let mut n = 0;
    guard.retain(|(name, tid, m)| {
        if name != stat {
            return true;
        }
        rtti::call_func(&rm, sts, &[Arg::Raw(*tid), Arg::Raw(*m)]);
        n += 1;
        false
    });
    if verbose {
        crate::log(&format!("[{tag}] {stat}: {n} modificador(es) removido(s)"));
    }
    n > 0
}

/// `stat?`/`statw?`: só lê, não aplica nada.
pub unsafe fn stat_read(
    reg: &Registry,
    captured_player: *mut c_void,
    stat: &str,
    target: StatTarget,
) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sts = system_flex(reg, owner, gi, "gameStatsSystem", "GetStatsSystem");
    if !rtti::sane(sts) {
        crate::log("[stat] StatsSystem inacessível");
        return false;
    }
    let obj = match target {
        StatTarget::Player => owner,
        StatTarget::Weapon => active_weapon(reg, owner).unwrap_or(std::ptr::null_mut()),
    };
    if !rtti::sane(obj) {
        return false;
    }
    let tid = match entity_id(reg, obj) {
        Some(b) => b,
        None => return false,
    };
    let stype = match rtti::resolve_enum_value(reg, "gamedataStatType", stat) {
        Some(v) => v,
        None => {
            crate::log(&format!("[stat] gamedataStatType::{stat} não existe neste build"));
            return false;
        }
    };
    crate::log(&format!("[stat] {stat} = {:?}", stat_value(reg, sts, tid, stype)));
    true
}

/// RAM do cyberdeck (quickhacks). É o pool `Memory` — o mesmo mecanismo de Health/Stamina,
/// confirmado no binário: `gamedataStatPoolType` só tem Health, Stamina e Memory.
/// Enche até 100%; o "ilimitado" é isto reaplicado no tick (ver `RAM_INFINITE` em lib.rs).
pub unsafe fn ram(reg: &Registry, captured_player: *mut c_void) -> bool {
    set_pool(reg, captured_player, "Memory", 100.0, "ram")
}

/// DIAGNÓSTICO read-only (2026-08-06): lê Health/Stamina/Sprint (StaminaRegen/estado) direto do
/// save real via `GetStatPoolValue` (nunca escreve nada) — pra investigar relato do usuário de
/// "não consigo mais correr segurando Shift depois de mexer nos últimos gaps". Não modifica
/// nenhum estado do jogo, só lê e loga.
pub unsafe fn staminacheck(reg: &Registry, captured_player: *mut c_void) -> bool {
    let owner = auth_or(reg, captured_player);
    let gi = match get_gi(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let sps = system_flex(reg, owner, gi, "gameStatPoolsSystem", "GetStatPoolsSystem");
    if !rtti::sane(sps) {
        crate::log("[staminacheck] StatPoolsSystem inacessível");
        return false;
    }
    let eid = match entity_id(reg, owner) {
        Some(b) => b,
        None => return false,
    };
    let gv = match rtti::resolve_any(reg, &["gameStatPoolsSystem"], "GetStatPoolValue") {
        Some(g) => g,
        None => {
            crate::log("[staminacheck] GetStatPoolValue não resolvido");
            return false;
        }
    };
    for (label, enum_name) in [("Stamina", "Stamina"), ("Health", "Health")] {
        let Some(v) = rtti::resolve_enum_value(reg, "gamedataStatPoolType", enum_name) else {
            crate::log(&format!("[staminacheck] enum gamedataStatPoolType::{enum_name} não resolvido"));
            continue;
        };
        let abs = rtti::call_func(&gv, sps, &[Arg::Raw(eid), Arg::Enum(v), Arg::Bool(false)])
            .map(|r| f32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        let pct = rtti::call_func(&gv, sps, &[Arg::Raw(eid), Arg::Enum(v), Arg::Bool(true)])
            .map(|r| f32::from_le_bytes([r[0], r[1], r[2], r[3]]));
        crate::log(&format!("[staminacheck] {label}: abs={abs:?} pct={pct:?}"));
    }
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

/// `cw-world-depot` (2026-07-24) — atalho pragmático (achado pela investigação paralela do mesmo
/// nome): em vez de forjar `DynamicEntitySystem`/`DynamicEntitySpec` (native-class-forge + RE de
/// endereço nativo desconhecido pro pipeline real de spawn do Codeware — o `CreateEntity` XL
/// genuíno), chama a API REAL VANILLA `CompanionSystem.SpawnSubcharacterOnPosition(recordID:
/// TweakDBID, pos: Vector3)` direto via `call_func` — a MESMA usada pelo próprio jogo pra spawnar
/// o Spiderbot (`cyberpunk/systems/subCharacterSystem.script`), acessada pelo MESMO caminho já
/// provado `give`/`system_flex` usam (`GetGame`→getter estático `GameInstance.GetXxx(gi)`). Zero
/// classe nova, zero RE de endereço novo — satisfaz o `proof_needed` literal do gap ("spawna uma
/// entidade visível") sem tocar `cw-variant-marshalling`.
///
/// `pos` é escrito como 3 floats consecutivos num slot de 16 bytes (`Arg::Raw`): a largura REAL da
/// struct `Vector3` (12 vs 16 bytes, não confirmada por RE) não importa pra correção — o motor lê
/// x/y/z pelo SEU PRÓPRIO descritor de tipo (`p_entries`/`ptype` do `SpawnSubcharacterOnPosition`
/// REAL, resolvido por `resolve_func`), não pelo nosso; os 4 bytes finais (usados só se a struct
/// real for 16B) ficam zerados, o mesmo que um `w`/padding implícito.
///
/// NÃO testado em boot nenhum ainda — comando de console novo (`spawnsub`), mesma categoria de
/// `cwprobe`/`cloneprobe`: opt-in, nunca auto-disparado.
pub unsafe fn spawn_subcharacter(
    reg: &Registry,
    captured_player: *mut c_void,
    record_name: &str,
    pos: (f32, f32, f32),
) -> Option<[u8; 0x20]> {
    let owner = auth_or(reg, captured_player);
    let gi = get_gi(reg, owner)?;
    let cs = via_getter(reg, owner, gi, "GetCompanionSystem");
    if !rtti::sane(cs) {
        crate::log(&format!("[spawnsub] CompanionSystem inacessível ({cs:p})"));
        return None;
    }
    // Diagnostico: CName hash da classe do objeto cs.
    let cs_class = rtti::class_of(cs);
    let cs_cn = if !cs_class.is_null() { unsafe { rtti::type_name_getname(cs_class) } } else { 0 };
    crate::log(&format!("[spawnsub] cs={cs:p} class_cname={cs_cn:#018x} ({:?})", crate::cname::resolve_cname(cs_cn)));
    // Tenta a classe pelo nome redscript; fallback p/ nome c++ gameCompanionSystem.
    let spawn = rtti::resolve_func(reg, "CompanionSystem", "SpawnSubcharacterOnPosition")
        .or_else(|| rtti::resolve_func(reg, "gameCompanionSystem", "SpawnSubcharacterOnPosition"))
        .or_else(|| rtti::resolve_func(reg, "ICompanionSystem", "SpawnSubcharacterOnPosition"))?;
    let tdbid = crate::cname::tweak_db_id(record_name).to_le_bytes();
    let mut posb = [0u8; 16];
    posb[0..4].copy_from_slice(&pos.0.to_le_bytes());
    posb[4..8].copy_from_slice(&pos.1.to_le_bytes());
    posb[8..12].copy_from_slice(&pos.2.to_le_bytes());
    crate::log(&format!(
        "[spawnsub] CompanionSystem.SpawnSubcharacterOnPosition('{record_name}' tdbid={:#018x}, pos={pos:?}) cs={cs:p}",
        u64::from_le_bytes(tdbid)
    ));
    let r = rtti::call_func(&spawn, cs, &[Arg::Tdb(tdbid), Arg::Raw(posb)]);
    crate::log(&format!("[spawnsub] call_func -> {:02x?}", r));
    r
}
