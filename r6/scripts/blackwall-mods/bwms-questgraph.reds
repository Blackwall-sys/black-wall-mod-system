// bwms-questgraph.reds — leitura do grafo de fase de quest pelos campos do motor, a base do `QuestPhaseGraphAccessor`.
// A API completa (os 14 `Find*` e `GetAllGraphNodePaths`) é a
// `BwmsQuestPhaseGraphAccessor` (`bwms-quest-accessor.reds`); aqui ficam o grafo da fase raiz, o filtro de nós por tipo
// (`GetNodesOfType<T>()`) e os leitores genéricos por offset.
//
// Cadeia de acesso, só offsets de CAMPO (sem deslocamento de vtable):
//   GameInstance.GetQuestsSystem(game) -> ref<QuestsSystem>   [native vanilla, orphans.script:11445]
//   QuestsSystem+0x68       = RootPhase: Handle<questPhaseInstance>   [header curado]
//   QuestPhaseInstance+0x40 = Graph: Handle<questGraphDefinition>     [mesmo header]
//   questGraphDefinition+0x30 = nodes: DynArray<Handle<GraphNodeDefinition>>  [header gerado da reflection,
//                                                                       Generated/graph/GraphDefinition.hpp]
//
// Os tipos concretos de nó (`questInputNodeDefinition`/`questEventManagerNodeDefinition`/`questJournalNodeDefinition`/...) não
// aparecem no corpus vanilla: declarar a ancestralidade às cegas é a categoria do `CMesh`, que derruba o bind. Todo handle de nó sai
// como `ref<IScriptable>`, o tipo concreto vem do `GetClassName()`, e o filtro é pelo `CName` que o mod passa
// (`n"questInputNodeDefinition"` etc.).

native func BwmsQuestPhaseGetRootGraph(sys: ref<QuestsSystem>) -> ref<IScriptable>;
native func BwmsQuestGraphNodeCount(graph: ref<IScriptable>, typeName: CName) -> Int32;
native func BwmsQuestGraphNodeAt(graph: ref<IScriptable>, typeName: CName, index: Int32) -> ref<IScriptable>;

@addMethod(QuestsSystem)
public func BwmsGetRootQuestGraph() -> ref<IScriptable> {
    return BwmsQuestPhaseGetRootGraph(this);
}

// -----------------------------------------------------------------------------------------
// Leitores genéricos (campo `Handle<T>` e array `DynArray<Handle<T>>` num offset qualquer): compõem em redscript a leitura de
// `.actions`/`.condition`/`.type`, sem declarar tipo de nó. Offsets dos headers gerados da
// reflection do jogo:
//   Generated/quest/SpawnManagerNodeDefinition.hpp
//     -> actions: DynArray<SpawnManagerNodeActionEntry> @ +0x48
//   Generated/quest/SpawnManagerNodeActionEntry.hpp
//     -> struct de 0x10 bytes = o próprio Handle<SpawnManagerNodeType> (offset 0 do entry)
//   Generated/quest/FactsDBManagerNodeDefinition.hpp
//     -> type: Handle<IFactsDBManagerNodeType> @ +0x48
//   Generated/quest/PauseConditionNodeDefinition.hpp
//     -> condition: Handle<IBaseCondition> @ +0x48

native func BwmsQuestNodeHandleField(node: ref<IScriptable>, offset: Int32) -> ref<IScriptable>;
native func BwmsQuestNodeArrayCount(node: ref<IScriptable>, offset: Int32, typeName: CName) -> Int32;
native func BwmsQuestNodeArrayAt(node: ref<IScriptable>, offset: Int32, typeName: CName, index: Int32) -> ref<IScriptable>;

// Offset dos campos `Handle<T>` únicos (`FactsDBManagerNodeDefinition.type`, `PauseConditionNodeDefinition.condition`): 0x48 = 72 nos
// dois headers gerados (redscript não tem literal hex).
public static func BwmsQuestSingleHandleFieldOffset() -> Int32 {
    return 72; // 0x48
}

// Offset do array `actions` de `questSpawnManagerNodeDefinition` (mesmo header gerado acima).
public static func BwmsQuestSpawnActionsOffset() -> Int32 {
    return 72; // 0x48
}
