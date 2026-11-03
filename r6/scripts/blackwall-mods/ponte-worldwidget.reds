// Codeware #96 — `worlduiIGameController::GetWorldWidgetComponent` + `inkWorldWidgetInfos::GetComponent`.
//
// Porte fiel do original (`Codeware/scripts/UI/Core/worlduiIGameController.reds`), com UMA
// diferença deliberada: o original faz `... as inkWorldWidgetInfos` e chama `.GetComponent()`.
// `inkWorldWidgetInfos` é `abstract native class` com ZERO ocorrência no corpus do jogo — o
// Codeware a registra em runtime, coisa que só acontece no Windows. Declará-la aqui é a
// armadilha do CMesh (crash no bind de RTTI), então o cast sai e a leitura vai pra native.
//
// O cast não estava filtrando nada que o `GetUserData(n"inkWorldWidgetInfos")` já não filtre:
// o próprio lookup é POR NOME DE TIPO. A semântica se preserva.

native func BwmsInkWorldWidgetComponent(userData: ref<IScriptable>) -> ref<IScriptable>;
// O original sobe a árvore com `widget.parentWidget`; aqui a subida é pela native `BwmsWidgetParent`, que lê o mesmo campo do motor
// (`WeakHandle<Widget> parentWidget // 168` no `inkWidget.hpp` do RED4ext.SDK).
native func BwmsWidgetParent(widget: ref<IScriptable>) -> ref<IScriptable>;

@addMethod(worlduiIGameController)
public func GetWorldWidgetComponent() -> ref<IComponent> {
    let widget: ref<inkWidget> = this.GetRootWidget();
    while IsDefined(widget) {
        let userData: ref<inkUserData> = widget.GetUserData(n"inkWorldWidgetInfos");
        if IsDefined(userData) {
            return BwmsInkWorldWidgetComponent(userData) as IComponent;
        };
        widget = BwmsWidgetParent(widget) as inkWidget;
    };
    return null;
}
