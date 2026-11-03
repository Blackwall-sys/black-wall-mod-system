// `worlduiIGameController.GetWorldWidgetComponent`: o componente 3D a que o widget do controller está preso.
//
// O tipo do motor que guarda os dados de widget-de-mundo não existe para o compilador redscript (declará-lo é a armadilha do CMesh: crash no
// bind de RTTI), então a leitura do componente e a subida da árvore são natives do BWMS. O user-data é procurado pelo NOME do tipo
// (`GetUserData`), que não pede o tipo no script.

native func BwmsInkWorldWidgetComponent(userData: ref<IScriptable>) -> ref<IScriptable>;
// A subida da árvore é pela native `BwmsWidgetParent`, que lê o campo `parentWidget` do motor
// (`WeakHandle<Widget> parentWidget`, offset 168 no `inkWidget.hpp` gerado).
native func BwmsWidgetParent(widget: ref<IScriptable>) -> ref<IScriptable>;

@addMethod(worlduiIGameController)
public func GetWorldWidgetComponent() -> ref<IComponent> {
    let widget: ref<inkWidget> = this.GetRootWidget();
    // Teto de níveis: a árvore de widgets é rasa; o teto só impede laço se o pai lido vier corrompido.
    let niveis: Int32 = 0;
    while IsDefined(widget) && niveis < 256 {
        let dados: ref<inkUserData> = widget.GetUserData(n"inkWorldWidgetInfos");
        if IsDefined(dados) {
            return BwmsInkWorldWidgetComponent(dados) as IComponent;
        }
        widget = BwmsWidgetParent(widget) as inkWidget;
        niveis += 1;
    }
    return null;
}
