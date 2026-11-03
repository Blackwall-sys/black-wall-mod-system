// bwms-populationspawner.reds — `PopulationSpawnerWrapper` com nome BWMS, só leitura.
//
// `BwmsPopulationSpawner.Get(game, id)` devolve o wrapper quando o `CommunitySystem` tem um spawner com esse id (o `globalNodeID` do nó
// `worldPopulationSpawnerNode`). O wrapper guarda só o id e resolve o spawner a cada chamada,
// então nunca segura ponteiro de um spawner que o motor já liberou. Os arrays de `EntityID` saem por índice (hash lido pela native e convertido
// com `BwmsHashToEntityID`), sem alocar nada no heap do motor. `GetActiveEntityIDs` segue o `vt[0x00]` do motor: stubs do pool e depois spawned.
native func BwmsSpawnerEntityHash(ptr: Uint64, which: Int32, index: Int32) -> Uint64
native func BwmsSpawnerPosition(ptr: Uint64) -> Vector4
native func BwmsSpawnerOrientation(ptr: Uint64) -> Quaternion
native func BwmsSpawnerActiveCount(ptr: Uint64) -> Int32
native func BwmsSpawnerActiveEntityHash(ptr: Uint64, index: Int32) -> Uint64

public class BwmsPopulationSpawner {
    private let m_game: GameInstance;
    private let m_id: Uint64;

    public static func Get(game: GameInstance, spawnerId: Uint64) -> ref<BwmsPopulationSpawner> {
        let sys: ref<CommunitySystem> = GameInstance.GetCommunitySystem(game);
        if !IsDefined(sys) {
            return null;
        };
        let ptr: Uint64 = BwmsCommunityGetSpawner(sys, spawnerId);
        if ptr == 0ul {
            return null;
        };
        let w: ref<BwmsPopulationSpawner> = new BwmsPopulationSpawner();
        w.m_game = game;
        w.m_id = spawnerId;
        return w;
    }

    public func GetId() -> Uint64 {
        return this.m_id;
    }

    private func Resolver() -> Uint64 {
        let sys: ref<CommunitySystem> = GameInstance.GetCommunitySystem(this.m_game);
        if !IsDefined(sys) {
            return 0ul;
        };
        let ptr: Uint64 = BwmsCommunityGetSpawner(sys, this.m_id);
        return ptr;
    }

    public func IsActive() -> Bool {
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return false;
        };
        let v: Bool = BwmsSpawnerIsActive(ptr);
        return v;
    }

    public func IsInitialized() -> Bool {
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return false;
        };
        let v: Bool = BwmsSpawnerIsInitialized(ptr);
        return v;
    }

    public func GetRecordID() -> TweakDBID {
        let ptr: Uint64 = this.Resolver();
        let raw: Uint64 = 0ul;
        if ptr != 0ul {
            raw = BwmsSpawnerRecordId(ptr, false);
        };
        let id: TweakDBID = BwmsHashToTweakDBID(raw);
        return id;
    }

    public func GetAppearanceName() -> CName {
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return n"None";
        };
        let v: CName = BwmsSpawnerAppearanceName(ptr);
        return v;
    }

    public func GetTransform() -> WorldTransform {
        let t: WorldTransform;
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return t;
        };
        let pos: Vector4 = BwmsSpawnerPosition(ptr);
        let rot: Quaternion = BwmsSpawnerOrientation(ptr);
        WorldTransform.SetPosition(t, pos);
        WorldTransform.SetOrientation(t, rot);
        return t;
    }

    public func GetActiveEntityIDs() -> array<EntityID> {
        let out: array<EntityID>;
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return out;
        };
        let n: Int32 = BwmsSpawnerActiveCount(ptr);
        let i: Int32 = 0;
        while i < n {
            let h: Uint64 = BwmsSpawnerActiveEntityHash(ptr, i);
            let e: EntityID = BwmsHashToEntityID(h);
            ArrayPush(out, e);
            i += 1;
        };
        return out;
    }

    public func GetRestoredEntityIDs() -> array<EntityID> {
        return this.Ids(0);
    }

    public func GetSpawnedEntityIDs() -> array<EntityID> {
        return this.Ids(1);
    }

    public func GetReservedEntityIDs() -> array<EntityID> {
        return this.Ids(2);
    }

    private func Ids(which: Int32) -> array<EntityID> {
        let out: array<EntityID>;
        let ptr: Uint64 = this.Resolver();
        if ptr == 0ul {
            return out;
        };
        let n: Int32 = BwmsSpawnerIdsCount(ptr, which);
        let i: Int32 = 0;
        while i < n {
            let h: Uint64 = BwmsSpawnerEntityHash(ptr, which, i);
            let e: EntityID = BwmsHashToEntityID(h);
            ArrayPush(out, e);
            i += 1;
        };
        return out;
    }
}
