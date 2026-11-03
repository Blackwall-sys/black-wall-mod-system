// bwms-visualcontrollercomponent.reds — os campos do
// `entVisualControllerComponent` pelos offsets do header gerado da reflection do jogo (`Generated/ent/
// VisualControllerComponent.hpp`; `register_vcc_33_natives`, aparencia/mod.rs). A classe NÃO é declarada (a categoria do `CMesh`, que
// derruba o bind): o parâmetro é sempre `ref<IComponent>`, obtido por `Entity.FindComponentByType(n"entVisualControllerComponent")`,
// que busca só pelo nome.
//
// Campos expostos (leitura pura, zero mutação, zero RawFunc/vtable):
//   - meshProxy: Ref<CMesh>@0x90 → hash do ResourcePath embutido (BwmsVccMeshProxyHash).
//   - appearanceDependency: DynArray<VisualControllerDependency>@0xA8 (stride 0x18/entry:
//     mesh:RaRef<CMesh>@+0x00/appearanceName:CName@+0x08/componentName:CName@+0x10) → count +
//     3 getters indexados (BwmsVccAppearanceDependencyCount/MeshHash/AppearanceName/ComponentName).
//   - cookedAppearanceData: RaRef<CookedAppearanceData>@0xB8 → hash do ResourcePath embutido
//     (BwmsVccCookedAppearanceDataHash).
//   - forcedLodDistance: enum C++ de 1 byte (valores 0-8)@0xF0 → Int32 cru
//     (BwmsVccForcedLodDistance; sentinela -1 = componente inválido/ilegível).
native func BwmsVccMeshProxyHash(component: ref<IComponent>) -> Uint64
native func BwmsVccAppearanceDependencyCount(component: ref<IComponent>) -> Int32
native func BwmsVccAppearanceDependencyMeshHash(component: ref<IComponent>, idx: Int32) -> Uint64
native func BwmsVccAppearanceDependencyAppearanceName(component: ref<IComponent>, idx: Int32) -> CName
native func BwmsVccAppearanceDependencyComponentName(component: ref<IComponent>, idx: Int32) -> CName
native func BwmsVccCookedAppearanceDataHash(component: ref<IComponent>) -> Uint64
native func BwmsVccForcedLodDistance(component: ref<IComponent>) -> Int32
