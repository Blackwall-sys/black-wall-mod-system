// -----------------------------------------------------------------------------
// Codeware.World.StaticEntitySystem — item `#75` do catálogo exaustivo
// -----------------------------------------------------------------------------
//
// O `StaticEntitySystem` do Codeware (`enablers/Codeware/scripts/World/StaticEntitySystem.reds`), irmão do `DynamicEntitySystem`
// (`#73`) com registro de tags PRÓPRIO (Static e Dynamic são estados independentes na fonte, nunca compartilhados). As tags são
// bookkeeping do Rust (`Map<EntityID, Set<CName>>` e o espelho inverso, como o C++ do Codeware); o spawn é pelo `EntitySpawner` do
// motor. A API do Codeware não tem `IsRestored` nem `DeleteTagged` (o `StaticEntitySystem.hpp`
// não declara), e eles não são inventados aqui. Os getters indexados (`Count()`+`ByIndex(i)`) são do BWMS, além dos arrays.

public native class StaticEntitySystem extends IGameSystem {
    public native func IsReady() -> Bool

    public native func IsManaged(id: EntityID) -> Bool
    public native func IsTagged(id: EntityID, tag: CName) -> Bool

    public native func IsPopulated(tag: CName) -> Bool
    public native func AssignTag(id: EntityID, tag: CName) -> Void
    public native func UnassignTag(id: EntityID, tag: CName) -> Void

    // Getters indexados do BWMS, além dos arrays do Codeware abaixo.
    public native func GetTaggedID(tag: CName) -> EntityID
    public native func GetTaggedCount(tag: CName) -> Int32
    public native func GetTaggedIDByIndex(tag: CName, index: Int32) -> EntityID
    public native func GetTagCount(id: EntityID) -> Int32
    public native func GetTagByIndex(id: EntityID, index: Int32) -> CName

    // Spawn por template pelo EntitySpawner do motor (RuntimeSystemEntity), attach/detach, despawn (Dispose), arrays e
    // operações por tag. Id transiente: nada vai pro save.
    public native func SpawnEntity(spec: ref<StaticEntitySpec>) -> EntityID
    public native func DespawnEntity(id: EntityID) -> Bool
    public native func AttachEntity(id: EntityID) -> Bool
    public native func DetachEntity(id: EntityID) -> Bool
    public native func IsSpawned(id: EntityID) -> Bool
    public native func IsSpawning(id: EntityID) -> Bool
    public native func GetEntity(id: EntityID) -> ref<Entity>
    public native func GetTags(id: EntityID) -> array<CName>
    public native func GetTagged(tag: CName) -> array<ref<Entity>>
    public native func GetTaggedIDs(tag: CName) -> array<EntityID>
    public native func DespawnTagged(tag: CName) -> Void
    public native func AttachTagged(tag: CName) -> Void
    public native func DetachTagged(tag: CName) -> Void
}

public static native func BwmsGetStaticEntitySystem() -> ref<StaticEntitySystem>
