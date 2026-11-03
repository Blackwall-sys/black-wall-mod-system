// bwms-inksystem-layer-e-foco.reds — Codeware `#100` (`inkSystem`) com nomes BWMS: layer por nome e foco.
//
// `GetLayer(name)` do Codeware devolve o layer pela classe; aqui é composição sobre `BwmsInkSystemFindLayerIndexByClassName` e `BwmsInkSystemGetLayer`
// (handle do layer existente). Foco: `SetFocus(widget)`/`ResetFocus()` usam o `RequestSetFocus` vanilla do game controller do layer de notificações,
// e o handler grava o widget em `InkSystem+0x2E8` no mesmo call.

public func BwmsInkSystemGetLayerByName(name: CName) -> ref<IScriptable> {
    let i: Int32 = BwmsInkSystemFindLayerIndexByClassName(name);
    if i < 0 {
        return null;
    };
    let layer: ref<IScriptable> = BwmsInkSystemGetLayer(i);
    return layer;
}

public func BwmsInkSystemFocusController() -> ref<inkIGameController> {
    let layer: ref<IScriptable> = BwmsInkSystemGetLayerByName(n"inkGameNotificationsLayer");
    if !IsDefined(layer) {
        return null;
    };
    let fraco: wref<IScriptable> = BwmsInkLayerGetGameController(layer);
    let c: ref<IScriptable> = fraco;
    return c as inkIGameController;
}

public func BwmsInkSystemSetFocus(widget: ref<inkWidget>) -> Bool {
    let c: ref<inkIGameController> = BwmsInkSystemFocusController();
    if !IsDefined(c) {
        return false;
    };
    c.RequestSetFocus(widget);
    return true;
}

public func BwmsInkSystemResetFocus() -> Bool {
    let c: ref<inkIGameController> = BwmsInkSystemFocusController();
    if !IsDefined(c) {
        return false;
    };
    c.RequestSetFocus(null);
    return true;
}
