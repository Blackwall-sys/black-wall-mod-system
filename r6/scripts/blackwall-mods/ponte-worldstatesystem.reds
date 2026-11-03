// ponte-worldstatesystem.reds — Codeware `#84` (`WorldStateSystem`).
//
// A classe da referência é um sistema de jogo nativo novo; aqui é um objeto de script sem estado com os mesmos métodos e aliases, criado pelo
// `GameInstance.GetWorldStateSystem()` (todo o estado fica no motor). Tudo é native do BWMS (mundo/mod.rs, bloco do `#84`), que acha os sistemas da
// sessão pelo lado nativo — o `GetCommunitySystem(GetGameInstance())` do script volta nulo quando chamado pelo tick do BWMS:
// - `IsReady`/`GetStreamingWorld`: `QuestsSystem` com a fase raiz e o `worldStreamingWorld` do runtime system 27 (dentro de um `BwmsSerializable`).
// - `ToggleNode`/`ToggleVariant`: `questWorldDataManagerNodeDefinition` sintético executado pelo `ExecuteNode` do motor na fase raiz, como o
//   `QuestPhaseExecutor` da referência.
// - Comunidade e spawner: o NodeRef resolvido pelo `ResolveNodeRef` vanilla, os slots do `CommunitySystem` do motor (`#77`) e o
//   `CommunitySystem::Update(true)` do motor depois, como a referência. O spawner volta como o `BwmsPopulationSpawner` do `#80`.

native func BwmsWorldStateIsReady() -> Bool
native func BwmsWorldStateStreamingWorld() -> ref<IScriptable>
native func BwmsWorldStateToggleNode(node: NodeRef, state: Bool) -> Int32
native func BwmsWorldStateToggleVariant(prefab: NodeRef, name: CName, state: Bool) -> Int32
native func BwmsWorldStateCommunity(op: Int32, community: NodeRef, entry: CName, phase: CName) -> Bool
native func BwmsWorldStateSpawner(op: Int32, spawner: NodeRef) -> Bool
native func BwmsWorldStateGetCommunity(community: NodeRef) -> ref<CommunityWrapper>
native func BwmsWorldStateSpawnerId(spawner: NodeRef) -> Uint64
native func BwmsSessionGame() -> GameInstance

public class WorldStateSystem extends IScriptable {
    public func IsReady() -> Bool {
        let pronto: Bool = BwmsWorldStateIsReady();
        return pronto;
    }

    public func GetStreamingWorld() -> ref<IScriptable> {
        let mundo: ref<IScriptable> = BwmsWorldStateStreamingWorld();
        return mundo;
    }

    public func GetCommunity(community: NodeRef) -> ref<CommunityWrapper> {
        let wrapper: ref<CommunityWrapper> = BwmsWorldStateGetCommunity(community);
        return wrapper;
    }

    public func ActivateCommunity(community: NodeRef, opt entry: CName) {
        let ok: Bool = BwmsWorldStateCommunity(0, community, entry, n"None");
    }

    public func DeactivateCommunity(community: NodeRef, opt entry: CName) {
        let ok: Bool = BwmsWorldStateCommunity(1, community, entry, n"None");
    }

    public func ResetCommunity(community: NodeRef) {
        let ok: Bool = BwmsWorldStateCommunity(3, community, n"None", n"None");
    }

    public func SetCommunityPhase(community: NodeRef, entry: CName, phase: CName) {
        let ok: Bool = BwmsWorldStateCommunity(2, community, entry, phase);
    }

    public func GetPopulationSpawner(spawner: NodeRef) -> ref<BwmsPopulationSpawner> {
        let id: Uint64 = BwmsWorldStateSpawnerId(spawner);
        if id == 0ul {
            return null;
        }
        // O GameInstance vai numa local: native passada direto como argumento desloca os parâmetros (o id chegava 0 no boot 251).
        let game: GameInstance = BwmsSessionGame();
        return BwmsPopulationSpawner.Get(game, id);
    }

    public func ActivatePopulationSpawner(spawner: NodeRef) {
        let ok: Bool = BwmsWorldStateSpawner(0, spawner);
    }

    public func DeactivatePopulationSpawner(spawner: NodeRef) {
        let ok: Bool = BwmsWorldStateSpawner(1, spawner);
    }

    public func ResetPopulationSpawner(spawner: NodeRef) {
        let ok: Bool = BwmsWorldStateSpawner(2, spawner);
    }

    public func ToggleNode(node: NodeRef, state: Bool) {
        let r: Int32 = BwmsWorldStateToggleNode(node, state);
    }

    public func ToggleVariant(prefab: NodeRef, name: CName, state: Bool) {
        let r: Int32 = BwmsWorldStateToggleVariant(prefab, name, state);
    }

    public func TogglePrefab(node: NodeRef, state: Bool) {
        this.ToggleNode(node, state);
    }

    public func TogglePrefabVariant(prefab: NodeRef, name: CName, state: Bool) {
        this.ToggleVariant(prefab, name, state);
    }
}

@addMethod(GameInstance)
public static func GetWorldStateSystem() -> ref<WorldStateSystem> {
    return new WorldStateSystem();
}
