// ponte-componentwrapper.reds — o `ComponentWrapper` do Codeware (item #168) como `@addMethod(IComponent)`: máscara de chunks,
// caminho do recurso, aparência do mesh, refazer a aparência, carga do recurso e cache de material. No Codeware ele é um helper C++
// interno que despacha pelo TIPO do componente (`entMeshComponent`/`entSkinnedMeshComponent`/`entGarmentSkinnedMeshComponent`/
// `entMorphTargetSkinnedMeshComponent`) e lê e escreve campos crus; aqui é a mesma ideia, com nome do BWMS onde o Codeware não expõe
// nada ao script. Os offsets dos campos vêm do RTTI da classe real (`CProperty::valueOffset`), nunca de tabela: o header do
// RED4ext.SDK erra a família skinned por 8 bytes nesta build.
//
// `IsEnabled`/`SetEnabled` (`m_component->isEnabled`) não precisam de nada: `IComponent.IsEnabled()` e `IComponent.Toggle(on)` já são
// nativas vanilla (`orphans.script:16667-16668`).
native func BwmsComponentGetChunkMask(component: ref<IComponent>) -> Uint64
native func BwmsComponentSetChunkMask(component: ref<IComponent>, mask: Uint64) -> Bool
native func BwmsComponentRefreshAppearance(component: ref<IComponent>) -> Bool

@addMethod(IComponent)
public func GetChunkMask() -> Uint64 {
    return BwmsComponentGetChunkMask(this);
}

@addMethod(IComponent)
public func SetChunkMask(mask: Uint64) -> Bool {
    return BwmsComponentSetChunkMask(this, mask);
}

// `RefreshAppearance() -> Bool`, o nome do Codeware (`IComponent.reds`). Só para componentes da família skinned
// (`entSkinnedMeshComponent`/`entGarmentSkinnedMeshComponent`, ex. 'torso'); devolve `false` pra `MeshComponent`/`MorphTarget`.
@addMethod(IComponent)
public func RefreshAppearance() -> Bool {
    return BwmsComponentRefreshAppearance(this);
}

// `SetResourcePath`/`GetResourcePath`, `SetAppearanceName` e `IsMeshComponent` (`ComponentWrapper.hpp:20`, o gate do Codeware):
// campo cru pelo offset do RTTI. O `GetAppearanceName` do `ComponentWrapper` NÃO é declarado: `IComponent.GetAppearanceName()` já é
// `final native const` vanilla (`orphans.script:16659`) e lê `IComponent::appearanceName`, o campo da classe base, enquanto
// `SetAppearanceName` escreve `meshAppearance`, o campo do subtipo de mesh. O getter desse campo é `GetMeshAppearanceName`.
native func BwmsComponentGetResourcePath(component: ref<IComponent>) -> ResRef
native func BwmsComponentSetResourcePath(component: ref<IComponent>, path: ResRef) -> Bool
native func BwmsComponentGetMeshAppearanceName(component: ref<IComponent>) -> CName
native func BwmsComponentSetAppearanceName(component: ref<IComponent>, appearance: CName) -> Bool
native func BwmsComponentIsMeshComponent(component: ref<IComponent>) -> Bool
// Codeware `#168` `LoadResource`: fila de jobs + `MeshComponent::LoadResource` (slot 0x268) + destrutor. Sem o `aRefresh` e o `aWait`
// do Codeware — chame `RefreshAppearance()` depois.
native func BwmsComponentLoadResource(component: ref<IComponent>) -> Bool
// `#168` `LoadResourceToken`: pede a carga do path do mesh do componente e devolve o token. Sem o `aWait` de 5 s do Codeware —
// `WaitForJob()` no token espera.
native func BwmsComponentLoadResourceToken(component: ref<IComponent>) -> Uint64
// `#168` `ResetMaterialCache`: espera o job de material da aparência do componente, solta os dados de render e esvazia o mapa de
// materiais — o motor reconstrói na próxima carga do recurso (`LoadResource()`; o `RefreshAppearance` não pede material).
native func BwmsComponentResetMaterialCache(component: ref<IComponent>) -> Bool

@addMethod(IComponent)
public func LoadResource() -> Bool {
    return BwmsComponentLoadResource(this);
}

@addMethod(IComponent)
public func LoadResourceToken() -> ref<ResourceToken> {
    let id: Uint64 = BwmsComponentLoadResourceToken(this);
    if id == 0ul {
        return null;
    }
    return ResourceToken.FromId(id);
}

@addMethod(IComponent)
public func ResetMaterialCache() -> Bool {
    return BwmsComponentResetMaterialCache(this);
}

@addMethod(IComponent)
public func GetResourcePath() -> ResRef {
    return BwmsComponentGetResourcePath(this);
}

@addMethod(IComponent)
public func SetResourcePath(path: ResRef) -> Bool {
    return BwmsComponentSetResourcePath(this, path);
}

// Nome diferente do `ComponentWrapper::GetAppearanceName` de propósito — ver a nota acima.
@addMethod(IComponent)
public func GetMeshAppearanceName() -> CName {
    return BwmsComponentGetMeshAppearanceName(this);
}

@addMethod(IComponent)
public func SetAppearanceName(appearance: CName) -> Bool {
    return BwmsComponentSetAppearanceName(this, appearance);
}

// Nome do Codeware (`ComponentWrapper.hpp:20`); sem colisão no corpus vanilla.
@addMethod(IComponent)
public func IsMeshComponent() -> Bool {
    return BwmsComponentIsMeshComponent(this);
}
