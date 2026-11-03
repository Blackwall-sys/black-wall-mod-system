// bwms-widget-building.reds — Codeware `#175` (`WidgetBuildingService`) no BWMS.
//
// Widget criado por script e ligado a uma árvore que já está na tela (`Reparent`, `AddChildWidget`) entra na camada dela: o BWMS anexa a árvore com o
// `AttachWidgetTree` do motor, e os controllers dela recebem widget, `layerProxy` e `OnInitialize`. `BwmsBindController` é o `AttachController` do Codeware
// com forma própria: liga o controller ao widget (principal ou secundário) e, com o widget já na tela, registra o controller na camada e chama o
// `OnInitialize`; fora da tela devolve `false` e o `OnInitialize` vem quando a árvore for anexada. Controller com as propriedades `detachedWidget` e
// `gameController` (o `inkCustomController` antigo) chama o próprio `OnInitialize`, e o motor não chama por ele.
//
// Este arquivo no bundle liga, na carga da dylib, os hooks do `Reparent`, do `AddChildWidget` e do `OnInitialize`.

public native func BwmsWidgetAttachTree(widget: ref<inkWidget>) -> Bool;

public native func BwmsWidgetAttachController(widget: ref<inkWidget>, controller: ref<inkLogicController>, secondary: Bool) -> Bool;

@addMethod(inkWidget)
public func BwmsAttachWidgetTree() -> Bool {
    let anexou: Bool = BwmsWidgetAttachTree(this);
    return anexou;
}

@addMethod(inkWidget)
public func BwmsBindController(controller: ref<inkLogicController>, opt secondary: Bool) -> Bool {
    let ligou: Bool = BwmsWidgetAttachController(this, controller, secondary);
    return ligou;
}
