// bwms-sessao.reds — `PlayerSpawnedHook` (o `Session/Ready` de sessão que não veio de save).
//
// O despacho é do Rust: hook no `cp::PlayerSystem::OnPlayerMainObjectSpawned` do motor (`0x103fd4d70`), contado no hook e despachado no tick da thread
// do jogo (`session_ready_tick`, inicio/natives.rs), depois da função original. A sessão carregada de
// save é marcada no Rust pelo `OnGameLoad` do `questWorldStateSystem`; daqui sai só o `IsPreGame` do handler de
// pedidos do sistema.

// Estado da sessão pro despacho: 1 = pregame.
public static func BwmsSessaoEstado() -> Int32 {
    let pregame: Bool = false;
    let handler: wref<inkISystemRequestsHandler> = BwmsGetSystemRequestsHandler();
    if IsDefined(handler) {
        pregame = handler.IsPreGame();
    };
    return pregame ? 1 : 0;
}

// Elo vazio que o dispatcher de smoke ainda chama; o despacho é do Rust.
@addMethod(PlayerPuppet)
public func BwmsAlwaysRun_player_spawned() -> Void {}
