// bwms-sessao.reds — Codeware `#129` (`PlayerSpawnedHook`, o `Session/Ready` de sessão que não veio de save).
//
// O despacho é do Rust: hook no `cp::PlayerSystem::OnPlayerMainObjectSpawned` do motor (`0x103fd4d70`), contado no hook e despachado no tick da thread
// do jogo (`session_ready_tick`, inicio/natives.rs), como o HookAfter da referência (`App/Callback/Controllers/PlayerSpawnedHook.hpp`). A sessão carregada de
// save é marcada no Rust pelo `OnGameLoad` do `questWorldStateSystem`, como o `OnGameLoad` da referência; daqui sai só o `IsPreGame` do handler de
// pedidos do sistema (o que a referência lê no `OnWorldAttached`).

// Estado da sessão pro despacho: 1 = pregame.
public static func BwmsSessaoEstado() -> Int32 {
    let pregame: Bool = false;
    let handler: wref<inkISystemRequestsHandler> = BwmsGetSystemRequestsHandler();
    if IsDefined(handler) {
        pregame = handler.IsPreGame();
    };
    return pregame ? 1 : 0;
}
