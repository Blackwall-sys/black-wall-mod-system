// bwms-inksystem-worldwidgets.reds — `GetWorldWidgets` com nome BWMS.
//
// Os controllers dos componentes de widget do world layer (telas de device, placas etc.), lidos direto do layer sem chamar nada. Só entram os
// componentes com janela e controller vivo.
native func BwmsInkWorldWidgetCount() -> Int32
native func BwmsInkWorldWidgetController(index: Int32) -> wref<IScriptable>

public func BwmsInkSystemGetWorldWidgets() -> array<wref<inkIGameController>> {
    let lista: array<wref<inkIGameController>>;
    let n: Int32 = BwmsInkWorldWidgetCount();
    let i: Int32 = 0;
    while i < n {
        let fraco: wref<IScriptable> = BwmsInkWorldWidgetController(i);
        let forte: ref<IScriptable> = fraco;
        let c: ref<inkIGameController> = forte as inkIGameController;
        if IsDefined(c) {
            ArrayPush(lista, c);
        };
        i += 1;
    };
    return lista;
}
