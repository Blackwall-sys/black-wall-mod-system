// BWMS — o CallbackSystem do Codeware sobre as declarações que a 2.5 traz no bundle do jogo. `CallbackSystem`, `CallbackSystemHandler`,
// `CallbackSystemEvent`, `CallbackSystemTarget`, `ResourceTarget`, `InputTarget`, `GameSessionEvent` e `ResourceEvent` são declarados pelo
// jogo; este arquivo só acrescenta o que o jogo não tem. O nome de script dessas classes aponta pras classes do BWMS
// (`inicio::boot::religar_familia_do_callbacksystem`): o handler do jogo chama virtuais C++ do alvo (`AddTarget` lê o slot `+0x120` da vtable
// do alvo, 2.5 Mac `0x10504d774`), e os alvos do BWMS (`EntityTarget` etc.) não têm essas virtuais.

// `CallbackRunMode`/`CallbackLifetime`: os nomes do Codeware, com os valores de `gameCallbackRunMode`/`gameCallbackLifetime` do jogo.
public enum CallbackRunMode {
    Default = 0,
    Once = 1,
    OncePerTarget = 2,
}
public enum CallbackLifetime {
    Session = 0,
    Forever = 1,
}

// O jogo declara `SetRunMode`/`SetLifetime` com os enums dele; mod do Codeware passa os enums acima.
@addMethod(CallbackSystemHandler)
public func SetRunMode(runMode: CallbackRunMode) -> ref<CallbackSystemHandler> {
    return this.SetRunMode(IntEnum<gameCallbackRunMode>(EnumInt(runMode)));
}

@addMethod(CallbackSystemHandler)
public func SetLifetime(lifetime: CallbackLifetime) -> ref<CallbackSystemHandler> {
    return this.SetLifetime(IntEnum<gameCallbackLifetime>(EnumInt(lifetime)));
}

// Só do BWMS (o Codeware não tem): diz se o alvo está no filtro do handler.
@addMethod(CallbackSystemHandler)
public native func HasTarget(target: ref<CallbackSystemTarget>) -> Bool

// O jogo declara só o campo `eventName`; o Codeware expõe o getter.
@addMethod(CallbackSystemEvent)
public native func GetEventName() -> CName

// O jogo declara `RegisterStaticCallback` com o alvo `ref<IScriptable>` (a classe do objeto vira o contexto); mod do Codeware anterior à 2.5
// passa o nome da classe.
@addMethod(CallbackSystem)
public func RegisterStaticCallback(eventName: CName, target: CName, function: CName, opt sticky: Bool) -> ref<CallbackSystemHandler> {
    let handler: ref<CallbackSystemHandler> = BwmsRegisterStaticCallback(eventName, target, function, sticky);
    return handler;
}

public static native func BwmsRegisterStaticCallback(eventName: CName, target: CName, function: CName, sticky: Bool) -> ref<CallbackSystemHandler>

// O jogo declara `InputTarget.Key(key, opt threshold: Float)`, sem filtro de ação; mod do Codeware anterior à 2.5 filtra pela ação.
@addMethod(InputTarget)
public static func Key(key: EInputKey, action: EInputAction) -> ref<InputTarget> {
    let alvo: ref<InputTarget> = BwmsInputTargetKey(key, action);
    return alvo;
}

public static native func BwmsInputTargetKey(key: EInputKey, action: EInputAction) -> ref<InputTarget>

// Os alvos do Codeware que o jogo não tem (`Callback/Targets/*.reds`), todos `extends CallbackSystemTarget` direto. As fábricas são estáticas;
// o casamento com o evento é feito no despacho, no Rust. Parâmetro `ResRef` chega ao Rust como o hash do caminho.
public native class ComponentTarget extends CallbackSystemTarget {
    public static native func ID(id: CRUID) -> ref<ComponentTarget>
    public static native func Name(name: CName) -> ref<ComponentTarget>
}

public native class inkWidgetTarget extends CallbackSystemTarget {
    public static native func Controller(type: CName) -> ref<inkWidgetTarget>
    public static native func Library(library: ResRef, opt item: CName) -> ref<inkWidgetTarget>
}

public native class DynamicEntityTarget extends CallbackSystemTarget {
    public static native func Tag(tag: CName) -> ref<DynamicEntityTarget>
    // Todas as tags do array no mesmo conjunto do `Tag(x)`: casa com qualquer uma, como o `Matches()` do Codeware (`tramp_dyt_tags`,
    // npcs/mod.rs).
    public static native func Tags(tags: array<CName>) -> ref<DynamicEntityTarget>
}

public native class StaticEntityTarget extends CallbackSystemTarget {
    public static native func Tag(tag: CName) -> ref<StaticEntityTarget>
    public static native func Tags(tags: array<CName>) -> ref<StaticEntityTarget>
}

public native class EntityTarget extends CallbackSystemTarget {
    public static native func ID(entityID: EntityID) -> ref<EntityTarget>
    public static native func Type(entityType: CName) -> ref<EntityTarget>
    public static native func RecordID(recordID: TweakDBID) -> ref<EntityTarget>
    public static native func Template(templatePath: ResRef) -> ref<EntityTarget>
    public static native func Appearance(appearanceName: CName) -> ref<EntityTarget>
    public static native func Definition(appearancePath: ResRef, opt definitionName: CName) -> ref<EntityTarget>
}

// O jogo registra `KeyInputEvent` no RTTI (`gameKeyInputEvent`) mas não o declara no bundle; `EInputAction` é o enum do jogo.
public native class KeyInputEvent extends CallbackSystemEvent {
    public native func GetAction() -> EInputAction
    public native func GetKey() -> EInputKey
    public native func IsShiftDown() -> Bool
    public native func IsControlDown() -> Bool
    public native func IsAltDown() -> Bool
}

// `AxisInputEvent extends KeyInputEvent` (Codeware `Events/AxisInputEvent.reds`).
public native class AxisInputEvent extends KeyInputEvent {
    public native func GetValue() -> Float
    public native func GetMouseX() -> Uint32
    public native func GetMouseY() -> Uint32
}

// `VehicleLightControlEvent extends EntityLifecycleEvent`; `vehicleELightType` é enum vanilla.
public native class VehicleLightControlEvent extends EntityLifecycleEvent {
    public native func IsEnabled() -> Bool
    public native func IsLightType(lightType: vehicleELightType) -> Bool
}

// `EntityComponentEvent extends EntityLifecycleEvent` (Codeware #9): `GetComponent` devolve `wref<IComponent>` real, como no Codeware.
public native class EntityComponentEvent extends EntityLifecycleEvent {
    public native func GetComponent() -> wref<IComponent>
}

// `inkWidgetSpawnEvent extends CallbackSystemEvent` (Codeware #11, `InkWidgetSpawnEvent.hpp`). Diferenças de tipo: `GetLibraryPath`
// devolve o hash (`Uint64`), o mesmo valor do `ResRef` de `ResourceEvent.GetPath`; `GetItemInstance` devolve `ref<IScriptable>`, porque
// `inkWidgetLibraryItemInstance` deriva só de `ISerializable` e declarar essa classe derruba o bind. A instância chega dentro de um
// `BwmsSerializable` (`as BwmsSerializable`, `GetObjectClassName()`); fora do despacho do evento, nulo.
public native class inkWidgetSpawnEvent extends CallbackSystemEvent {
    public native func GetLibraryPath() -> Uint64
    public native func GetItemName() -> CName
    public native func GetItemInstance() -> ref<IScriptable>
}

// `WidgetSpawningService.ToggleWidgetSpawnEvent` do Codeware (#123): chave do despacho de `InkWidget/Spawn`; devolve `true`. Nasce ligada,
// então o `RegisterCallback(n"InkWidget/Spawn", ...)` já liga sozinho, como no Codeware; `false` segura o despacho mesmo com ouvinte, até o
// `true`. O evento sai dos spawns síncronos (`SpawnFromLocal`/`SpawnFromExternal`); os assíncronos (`AsyncSpawnFrom*`) não despacham.
public static native func BwmsToggleWidgetSpawnEvent(state: Bool) -> Bool

// `EntityLifecycleEvent extends CallbackSystemEvent`: os eventos `Entity/*` do ciclo de vida saem dos hooks do motor, instalados quando
// alguém registra o evento e tirados quando o último ouvinte sai. `GetEntity()` devolve `wref<Entity>` real.
public native class EntityLifecycleEvent extends CallbackSystemEvent {
    public native func GetEntity() -> wref<Entity>
}

// O singleton do BWMS. `GameInstance.GetCallbackSystem()`, que o jogo declara, devolve o mesmo objeto (rota da native do jogo,
// `inicio::boot::religar_familia_do_callbacksystem`).
public static native func BwmsGetCallbackSystem() -> ref<CallbackSystem>
