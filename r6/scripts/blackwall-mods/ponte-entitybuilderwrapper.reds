// ponte-entitybuilderwrapper.reds — `EntityBuilderWrapper`, `EntityBuilderEvent`: API do BWMS sobre o `EntityBuilder` vivo.
//
// O `Entity/Extract` sai do hook do corpo do job de componentes do builder (`0x100ca41e0`), antes da original; o evento entrega um
// `EntityBuilderWrapper` que lê o builder real (layout do header conferido no desmonte do job no Mac) e trava o builder pelo `WeakPtr` a cada leitura —
// com o builder morto, os getters devolvem vazio.
//
// Formas próprias do BWMS onde o corpus vanilla NÃO declara o tipo de retorno (declarar tipo sem presença no corpus é a categoria do
// `CMesh`): `GetResourcePath()` no lugar de `GetResource()` (template e aparência), `GetDefinitionName()` no lugar de `GetDefinition()`,
// `HasEntityParams()` no lugar de `GetEntityParams()`.

public native class EntityBuilderWrapper extends IScriptable {
    public native func GetRecordID() -> TweakDBID
    public native func GetEntityID() -> EntityID
    public native func GetAppearanceName() -> CName
    public native func HasEntity() -> Bool
    public native func HasAppearance() -> Bool
    public native func HasCustomAppearances() -> Bool
    public native func GetTemplatePath() -> ResRef
    public native func GetEntityType() -> CName
    public native func HasEntityParams() -> Bool
    public native func GetTemplate() -> ref<EntityBuilderTemplateWrapper>
    public native func GetAppearance() -> ref<EntityBuilderAppearanceWrapper>
    public native func GetCustomAppearances() -> array<ref<EntityBuilderAppearanceWrapper>>
}

public native class EntityBuilderTemplateWrapper extends IScriptable {
    public native func GetResourcePath() -> ResRef
    public native func GetAppearanceName() -> CName
    public native func GetEntity() -> ref<Entity>
    public native func GetComponents() -> array<ref<IComponent>>
    public native func AddComponent(component: ref<IComponent>) -> Void
}

public native class EntityBuilderAppearanceWrapper extends IScriptable {
    public native func GetResourcePath() -> ResRef
    public native func GetDefinitionName() -> CName
    public native func GetComponents() -> array<ref<IComponent>>
    public native func AddComponent(component: ref<IComponent>) -> Void
}

public native class EntityBuilderEvent extends CallbackSystemEvent {
    public native func GetEntityBuilder() -> ref<EntityBuilderWrapper>
}
