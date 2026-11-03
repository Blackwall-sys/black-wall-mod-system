// ponte-dynamicentitysystem.reds — o `DynamicEntitySystem` do Codeware (`World/DynamicEntitySystem.reds`, item #73): classe nativa
// forjada pelo BWMS, `extends IGameSystem`, com os métodos do Codeware e 4 getters indexados do BWMS. Sem `module` de propósito:
// declaração nativa dentro de `module X.Y` ganha o prefixo no `fullName`, o bind não casa com o nome registrado e o jogo trava no boot.
//
// As entidades saem do motor (`EntityStubSystem` + `PopulationSystem`); os ouvintes de tag recebem `DynamicEntityEvent` com
// `Created`/`Deleted` (do `CreateEntity`/`DeleteEntity`) e `Spawned`/`Despawned`/`Dead` (do broadcaster de spawn do motor). Ouvintes são
// da sessão; entidade com `persistSpawn` volta na carga pelo arquivo do BWMS (`bwms-entidades-persistentes.reds`), nunca pelo save.

public native class DynamicEntitySystem extends IGameSystem {
    public native func IsReady() -> Bool
    public native func IsRestored() -> Bool

    public native func IsManaged(id: EntityID) -> Bool
    public native func IsTagged(id: EntityID, tag: CName) -> Bool

    public native func IsPopulated(tag: CName) -> Bool
    public native func AssignTag(id: EntityID, tag: CName) -> Void
    public native func UnassignTag(id: EntityID, tag: CName) -> Void
    public native func DeleteTagged(tag: CName) -> Void

    // Getters indexados do BWMS, além dos arrays do Codeware abaixo. `GetTaggedID(tag)` devolve a entidade tagueada de menor id.
    public native func GetTaggedID(tag: CName) -> EntityID
    public native func GetTaggedCount(tag: CName) -> Int32
    public native func GetTaggedIDByIndex(tag: CName, index: Int32) -> EntityID
    public native func GetTagCount(id: EntityID) -> Int32
    public native func GetTagByIndex(id: EntityID, index: Int32) -> CName

    // Ciclo de entidade pelo motor. Só id transiente: o persistente iria pro
    // save, e o BWMS não grava dado de mod no save. Spec sem `recordID` (só template) devolve id vazio.
    public native func CreateEntity(spec: ref<DynamicEntitySpec>) -> EntityID
    public native func DeleteEntity(id: EntityID) -> Bool
    public native func IsSpawned(id: EntityID) -> Bool
    public native func IsSpawning(id: EntityID) -> Bool
    public native func GetEntity(id: EntityID) -> ref<Entity>

    // `DeleteTagged` apaga as entidades do `CreateEntity`; `Created`/`Deleted` saem do `CreateEntity`/`DeleteEntity` e
    // `Spawned`/`Despawned`/`Dead` do broadcaster de spawn do motor, pros ouvintes da tag.
    public native func EnableEntity(id: EntityID) -> Bool
    public native func DisableEntity(id: EntityID) -> Bool
    public native func GetTags(id: EntityID) -> array<CName>
    public native func GetTagged(tag: CName) -> array<ref<Entity>>
    public native func GetTaggedIDs(tag: CName) -> array<EntityID>
    public native func EnableTagged(tag: CName) -> Void
    public native func DisableTagged(tag: CName) -> Void
    public native func RegisterListener(tag: CName, target: ref<IScriptable>, function: CName) -> Void
    public native func UnregisterListener(tag: CName, target: ref<IScriptable>, function: CName) -> Void
    public native func UnregisterListeners(tag: CName) -> Void
}

public enum DynamicEntityEventType {
    Created = 0,
    Deleted = 1,
    Spawned = 2,
    Despawned = 3,
    Dead = 4
}

// Evento dos ouvintes do `DynamicEntitySystem`. Classe de script do BWMS (a referência é nativa): mesmos 3 getters.
public class DynamicEntityEvent {
    private let bwmsTipo: Int32;
    private let bwmsId: EntityID;
    private let bwmsTag: CName;

    public func GetEventType() -> DynamicEntityEventType = IntEnum<DynamicEntityEventType>(this.bwmsTipo)
    public func GetEntityID() -> EntityID = this.bwmsId
    public func GetEntityTag() -> CName = this.bwmsTag

    public static func BwmsNovo(tipo: Int32, id: EntityID, tag: CName) -> ref<DynamicEntityEvent> {
        let ev: ref<DynamicEntityEvent> = new DynamicEntityEvent();
        ev.bwmsTipo = tipo;
        ev.bwmsId = id;
        ev.bwmsTag = tag;
        return ev;
    }
}

// Chamada pelo tick do BWMS pra cada ouvinte da tag: monta o evento e chama `alvo.funcao(evento)`. O evento fica num local: como temporário no
// argumento, a VM o soltava logo depois de ler os parâmetros da native e o ouvinte recebia um objeto já liberado (boot 233).
public func BwmsDynamicEntityEventDispatch(alvo: ref<IScriptable>, funcao: CName, tipo: Int32, id: Uint64, tag: CName) -> Void {
    let ev: ref<DynamicEntityEvent> = DynamicEntityEvent.BwmsNovo(tipo, BwmsHashToEntityID(id), tag);
    BwmsCallMethodWithArg(alvo, funcao, ev);
}

public static native func BwmsGetDynamicEntitySystem() -> ref<DynamicEntitySystem>
