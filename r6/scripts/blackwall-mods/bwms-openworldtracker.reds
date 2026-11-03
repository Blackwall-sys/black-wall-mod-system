// bwms-openworldtracker.reds — Codeware `#142` (`OpenWorldTracker`).
//
// Quando um ponto de interesse chega na fase `CompletedPhase`, o hook do Rust em `PointOfInterestMappin::SetPhase` (`0x1042eaf14`) guarda o
// hash de journal do PoI (`GetJournalQuestPathHash`) e o tick do jogo chama esta função: a entrada passa por Inactive e volta a Active,
// sem notificação — o que o Codeware faz, nessa ordem e sem condição.
// Devolve os 3 estados como `antes*100 + meio*10 + depois` (valores de `gameJournalEntryState`), ou negativo sem entrada.
// `game` vem do Rust (`PlayerPuppet.GetGame`): dentro de uma chamada crua, `GetGameInstance()` devolve instância morta.
// Liga com `openworldtracker = true` no `red4ext/bwms.toml`; desligado por padrão porque o estado da entrada de journal vai pro save.
public func BwmsOpenWorldTrackerAplicar(game: GameInstance, hash: Uint32) -> Int32 {
    let jm: ref<JournalManager> = GameInstance.GetJournalManager(game);
    if !IsDefined(jm) {
        return -1;
    };
    let entrada: wref<JournalEntry> = jm.GetEntry(hash);
    if !IsDefined(entrada) {
        return -2;
    };
    let estadoAntes: gameJournalEntryState = jm.GetEntryState(entrada);
    let antes: Int32 = EnumInt(estadoAntes);
    jm.ChangeEntryStateByHash(hash, gameJournalEntryState.Inactive, JournalNotifyOption.DoNotNotify);
    let estadoMeio: gameJournalEntryState = jm.GetEntryState(entrada);
    let meio: Int32 = EnumInt(estadoMeio);
    jm.ChangeEntryStateByHash(hash, gameJournalEntryState.Active, JournalNotifyOption.DoNotNotify);
    let estadoDepois: gameJournalEntryState = jm.GetEntryState(entrada);
    let depois: Int32 = EnumInt(estadoDepois);
    return antes * 100 + meio * 10 + depois;
}
