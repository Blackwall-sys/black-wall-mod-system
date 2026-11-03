// bwms-inklayer.reds — Codeware `#93` (`inkLayerWrapper.reds`): os métodos do `inkLayerWrapper` como globais do BWMS que
// recebem o handle do layer (de `BwmsInkSystemGetLayer`), sem forjar a classe `inkLayerWrapper` — o campo `layer: Handle<inkLayer>` dela
// pediria bind de classe nova, e o mesmo dado sai do handle. `GetGameControllers()` (array no Codeware) vira contagem + índice, como
// `GetLayerCount`/`GetLayer(index)`. Todos só leem.
native func BwmsInkLayerGetName(layer: ref<IScriptable>) -> CName;
native func BwmsInkLayerGetWindow(layer: ref<IScriptable>) -> wref<IScriptable>;
native func BwmsInkLayerGetGameController(layer: ref<IScriptable>) -> wref<IScriptable>;
native func BwmsInkLayerGetControllerCount(layer: ref<IScriptable>) -> Int32;
native func BwmsInkLayerGetControllerAt(layer: ref<IScriptable>, index: Int32) -> wref<IScriptable>;
