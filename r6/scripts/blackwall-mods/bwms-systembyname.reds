// bwms-systembyname.reds — `BwmsGetSystemByName`, Codeware itens #75/#76/#138/#139/#141/#143.
//
// Generaliza pro redscript o comando de diagnóstico `sysbymap`/`gisysmapdump` (já provado ao
// vivo, RED4ext `#472`) — lê `GameInstance+0x08` (`systemMap: HashMap<IType*,
// Handle<IGameSystem>>`) por NOME, sem executar nenhum código do motor (mesma disciplina
// read-only já estabelecida). Cobre a lacuna real: `GameInstance.GetXSystem()` só existe pra uma
// fração dos ~149 sistemas registrados — qualquer sistema SEM accessor vanilla dedicado
// (`gamePopulationSystem`/`gamePersistencySystem`/`gameContainerManager`/`gameJournalManager`/
// `questWorldStateSystem`/`gameDynamicEntityIDSystem`/`gameEntityStubSystem`/
// `gameEntitySpawnerEventsBroadcasterImpl`/`gameGameTagSystem`/etc.) fica acessível assim.
//
// `wref<IScriptable>` (não-owning) — os `IGameSystem` são singletons ENGINE-gerenciados, vivem
// pelo resto da sessão, nunca precisam de retenção forte (mesma semântica já usada por
// `BwmsInkLayerGetWindow`/`GetGameController`).
//
// Resolver o ponteiro não abre despacho de método: sistema sem método exposto no RTTI (ex. `questWorldStateSystem`/
// `gamePopulationSystem`) precisa do slot de vtable medido por classe, como `MappinSystem`, `CommunitySystem` e
// `bwms-persistency-population.reds`.
native func BwmsGetSystemByName(name: CName) -> wref<IScriptable>;
