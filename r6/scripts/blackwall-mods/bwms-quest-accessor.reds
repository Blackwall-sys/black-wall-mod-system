// bwms-quest-accessor.reds — `QuestPhaseGraphAccessor` no BWMS: consulta de grafo de fase de quest.
//
// Nós por nome de classe (só o grafo ou com as fases aninhadas, nesta ordem: nós do grafo e depois cada `phaseGraph` não nulo), os 14 `Find*`
// e `GetAllGraphNodePaths`. Entrada por um grafo qualquer; `BwmsQuestPhaseResourceGraph` dá o de uma fase carregada. Os tipos concretos de nó não são
// declarados em script: todo handle volta como `ref<IScriptable>` e a classe real sai de `GetClassName()`. Só leitura.
native func BwmsQuestPhaseResourceGraph(resource: ref<IScriptable>) -> ref<IScriptable>
native func BwmsQuestAccessorCount(graph: ref<IScriptable>, recursive: Int32, typeName: CName) -> Int32
native func BwmsQuestAccessorAt(graph: ref<IScriptable>, recursive: Int32, typeName: CName, index: Int32) -> ref<IScriptable>
native func BwmsQuestAccessorFindCount(graph: ref<IScriptable>, recursive: Int32, kind: CName) -> Int32
native func BwmsQuestAccessorFindAt(graph: ref<IScriptable>, recursive: Int32, kind: CName, index: Int32) -> ref<IScriptable>
native func BwmsQuestAccessorPathCount(graph: ref<IScriptable>) -> Int32
native func BwmsQuestAccessorPathDepth(graph: ref<IScriptable>, index: Int32) -> Int32
native func BwmsQuestAccessorPathId(graph: ref<IScriptable>, index: Int32, level: Int32) -> Int32

public struct BwmsQuestNodeKey {
    public let phasePath: array<Int32>;
    public let nodeId: Int32;
}

public class BwmsQuestPhaseGraphAccessor {
    private let m_graph: ref<IScriptable>;
    private let m_recursive: Int32;

    public static func Create(graph: ref<IScriptable>, recursive: Bool) -> ref<BwmsQuestPhaseGraphAccessor> {
        let a: ref<BwmsQuestPhaseGraphAccessor> = new BwmsQuestPhaseGraphAccessor();
        a.m_graph = graph;
        a.m_recursive = recursive ? 1 : 0;
        return a;
    }

    public static func FromPhaseResource(resource: ref<IScriptable>, recursive: Bool) -> ref<BwmsQuestPhaseGraphAccessor> {
        let graph: ref<IScriptable> = BwmsQuestPhaseResourceGraph(resource);
        return BwmsQuestPhaseGraphAccessor.Create(graph, recursive);
    }

    public func GetNodesOfType(typeName: CName) -> array<ref<IScriptable>> {
        let nodes: array<ref<IScriptable>>;
        let n: Int32 = BwmsQuestAccessorCount(this.m_graph, this.m_recursive, typeName);
        let i: Int32 = 0;
        while i < n {
            ArrayPush(nodes, BwmsQuestAccessorAt(this.m_graph, this.m_recursive, typeName, i));
            i += 1;
        };
        return nodes;
    }

    private func Find(kind: CName) -> array<ref<IScriptable>> {
        let found: array<ref<IScriptable>>;
        let n: Int32 = BwmsQuestAccessorFindCount(this.m_graph, this.m_recursive, kind);
        let i: Int32 = 0;
        while i < n {
            ArrayPush(found, BwmsQuestAccessorFindAt(this.m_graph, this.m_recursive, kind, i));
            i += 1;
        };
        return found;
    }

    private func FindFirst(kind: CName) -> ref<IScriptable> {
        let n: Int32 = BwmsQuestAccessorFindCount(this.m_graph, this.m_recursive, kind);
        if n > 0 {
            return BwmsQuestAccessorFindAt(this.m_graph, this.m_recursive, kind, 0);
        };
        return null;
    }

    public func FindInputNode() -> ref<IScriptable> { return this.FindFirst(n"InputNode"); }
    public func FindPointOfInterestMappin() -> ref<IScriptable> { return this.FindFirst(n"PointOfInterestMappin"); }
    public func FindCompletedPointOfInterestMappin() -> ref<IScriptable> { return this.FindFirst(n"CompletedPointOfInterestMappin"); }
    public func FindCommunities() -> array<ref<IScriptable>> { return this.Find(n"Communities"); }
    public func FindSpawnSets() -> array<ref<IScriptable>> { return this.Find(n"SpawnSets"); }
    public func FindSpawners() -> array<ref<IScriptable>> { return this.Find(n"Spawners"); }
    public func FindFactChanges() -> array<ref<IScriptable>> { return this.Find(n"FactChanges"); }
    public func FindFactConditions() -> array<ref<IScriptable>> { return this.Find(n"FactConditions"); }
    public func FindManagerEvents() -> array<ref<IScriptable>> { return this.Find(n"ManagerEvents"); }
    public func FindJournalEntries() -> array<ref<IScriptable>> { return this.Find(n"JournalEntries"); }
    public func FindJournalConditions() -> array<ref<IScriptable>> { return this.Find(n"JournalConditions"); }
    public func FindLootConditions() -> array<ref<IScriptable>> { return this.Find(n"LootConditions"); }
    public func FindLootContainerCondition() -> ref<IScriptable> { return this.FindFirst(n"LootContainerCondition"); }
    public func FindCharacterKillCondition() -> ref<IScriptable> { return this.FindFirst(n"CharacterKillCondition"); }

    // Caminho de cada nó de quest do grafo, com `phaseNodePath` na frente (os ids das fases de fora até este grafo).
    public func GetAllGraphNodePaths(phaseNodePath: array<Int32>) -> array<BwmsQuestNodeKey> {
        let keys: array<BwmsQuestNodeKey>;
        let n: Int32 = BwmsQuestAccessorPathCount(this.m_graph);
        let i: Int32 = 0;
        while i < n {
            let key: BwmsQuestNodeKey;
            let depth: Int32 = BwmsQuestAccessorPathDepth(this.m_graph, i);
            let path: array<Int32>;
            for id in phaseNodePath {
                ArrayPush(path, id);
            };
            let level: Int32 = 0;
            while level < depth {
                ArrayPush(path, BwmsQuestAccessorPathId(this.m_graph, i, level));
                level += 1;
            };
            key.phasePath = path;
            key.nodeId = BwmsQuestAccessorPathId(this.m_graph, i, depth);
            ArrayPush(keys, key);
            i += 1;
        };
        return keys;
    }
}
