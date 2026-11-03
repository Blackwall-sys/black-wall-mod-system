// bwms-inksystem-layers.reds — os layers do `InkSystem` vivo. O singleton sai de
// `ink_system()` (jogo/interface.rs), achado e guardado em cache.
//
// Navegação até os layers (offsets do `InkSystem`):
//   InkSystem+0x380 = layerManagers: DynArray<SharedPtr<InkLayerManager>>
//   layerManagers[0].instance = InkLayerManager*
//   InkLayerManager+0x38 = layers: DynArray<Handle<inkLayer>>
//
// `inkLayer`/`inkWorldLayer` não aparecem no corpus vanilla nem nos símbolos do binário: declará-los é a categoria do `CMesh`, que
// derruba o bind. Por isso as natives devolvem `CName`/`Int32` (identidade, contagem, índice) e o handle do layer sai como
// `ref<IScriptable>`. O `GetWorldWidgets()` está em `bwms-inksystem-worldwidgets.reds`. Todas só leem.
native func BwmsInkSystemGetLayerManagerCount() -> Int32;
native func BwmsInkSystemGetLayerCount() -> Int32;
native func BwmsInkSystemGetLayerClassNameAt(index: Int32) -> CName;
native func BwmsInkSystemFindLayerIndexByClassName(name: CName) -> Int32;
native func BwmsInkSystemFindWorldLayerIndex() -> Int32;

// `BwmsInkSystemGetLayer(index)`: handle real do layer no índice, pela rotina do próprio motor (`rtti::make_handle`), que acha o bloco
// de refcount que o objeto já tem em `layers` e só incrementa. Tipo estático `ref<IScriptable>`; a classe concreta sai do
// `GetClassName()` do handle. Índice fora do alcance ou ponteiro inválido devolve nulo.
native func BwmsInkSystemGetLayer(index: Int32) -> ref<IScriptable>;

// `BwmsInkSystemGetLayers()`: os layers por composição sobre `BwmsInkSystemGetLayerCount` + `BwmsInkSystemGetLayer(i)`,
// no mesmo espaço de índice (`layerManagers[0]->layers`), com o retorno genérico do `GetLayer`.
public func BwmsInkSystemGetLayers() -> array<ref<IScriptable>> {
    let result: array<ref<IScriptable>>;
    let count: Int32 = BwmsInkSystemGetLayerCount();
    let i: Int32 = 0;
    while i < count {
        ArrayPush(result, BwmsInkSystemGetLayer(i));
        i += 1;
    }
    return result;
}
