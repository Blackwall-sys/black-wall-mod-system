// -----------------------------------------------------------------------------
// inkWidget.BwmsAttachController/BwmsGetController — `AttachController`/`GetController` com nome do BWMS
// -----------------------------------------------------------------------------
//
// `inkWidget.GetController() -> wref<inkLogicController>` já é native VANILLA (`ui/baseWidgets/abstractWidgets.script:8`): um
// `@addMethod GetController()` colide com ela e o `scc` não recusa — a chamada resolve pra native vanilla, que devolve o link do motor
// (nulo pro controller do mod). Daí os nomes próprios. O campo guarda `wref<T>`: com `ref<T>` num `@addField` de widget criado com
// `new`, a leitura nunca achava o valor; as assinaturas públicas usam `ref<T>`.
// `GetControllerByType(CName)`/`GetControllers()` também são natives vanilla de `inkWidget`, usáveis direto.
//
// Mod que espera `widget.AttachController(...)` chama `widget.BwmsAttachController(...)` no BWMS.
//
// `module` só pode ser o 1º statement do arquivo (o `scc` recusa no meio) — por isso esta extensão de `inkWidget`, classe vanilla
// global, mora num arquivo próprio, fora de `module Codeware.UI`.

@addField(inkWidget) let m_bwmsLogicController: wref<inkLogicController>;

@addMethod(inkWidget)
public func BwmsAttachController(controller: ref<inkLogicController>) -> Void {
    this.m_bwmsLogicController = controller;
}

@addMethod(inkWidget)
public func BwmsGetController() -> ref<inkLogicController> {
    let result: ref<inkLogicController> = this.m_bwmsLogicController;
    return result;
}
