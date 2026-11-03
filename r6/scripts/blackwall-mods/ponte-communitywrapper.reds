// ponte-communitywrapper.reds — Codeware `#77` (`CommunityWrapper`/`CommunityEntryWrapper`), leitura das comunidades vivas do `CommunitySystem`.
//
// Cada wrapper guarda a comunidade pelo `WeakPtr` que o `GetCommunity` do motor devolve e trava a comunidade a cada leitura: com a comunidade
// descarregada, os getters devolvem vazio. `GetActiveEntityIDs` é o virtual do motor (`game::Community`, slot 0 da vtable).
//
// Forma própria: `communitySpawnEntry` e `communityCommunityTemplateData` não são declarados no corpus vanilla (declarar é a categoria do `CMesh`),
// então `GetTemplateEntryData()` e `GetTemplateData()` devolvem `Variant` com o handle do objeto nativo. A entrega da referência é
// `WorldStateSystem.GetCommunity(NodeRef)` (Codeware `#84`); aqui é `BwmsCommunityGetWrapper(sistema, id)`.

public native class CommunityEntryWrapper extends IScriptable {
    public native func IsActive() -> Bool
    public native func GetName() -> CName
    public native func GetPhases() -> array<CName>
    public native func GetRestoredEntityIDs() -> array<EntityID>
    public native func GetSpawnedEntityIDs() -> array<EntityID>
    public native func GetSpawningEntityIDs() -> array<EntityID>
    public native func GetReservedEntityIDs() -> array<EntityID>
    public native func GetTemplateEntryData() -> Variant
}

public native class CommunityWrapper extends IScriptable {
    public native func GetEntries() -> array<ref<CommunityEntryWrapper>>
    public native func GetActiveEntityIDs() -> array<EntityID>
    public native func GetTemplateData() -> Variant
}

public native func BwmsCommunityGetWrapper(sys: ref<CommunitySystem>, communityId: Uint64) -> ref<CommunityWrapper>
