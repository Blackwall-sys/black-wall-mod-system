// bwms-worldwidget.reds — o componente 3D por trás de um widget desenhado no mundo.
// 2026-08-22.
//
// PARA QUE SERVE: um widget que aparece NO MUNDO (holograma, painel de loja, terminal, placa) está
// preso a um componente de entidade. Saber qual é a ponte entre a camada de UI e a geometria — e
// é o que permite seguir, posicionar ou reagir a esse widget em 3D. Para VR isso é a diferença
// entre um HUD colado na tela e um elemento que existe no espaço.
//
// A FICHA DO CATÁLOGO DIZIA "REFUTADO COM CERTEZA". Verificado: são **dois derefs**, sem
// `RawFunc` e sem endereço nativo nenhum —
//   `infos+0x40` → `SharedPtr<worlduiWorldLayerWidget>` → `wrapper+0x08` → `WeakHandle<IComponent>`
// com os dois offsets confirmados nos headers GERADOS da reflection real do jogo
// (`Generated/ink/WorldWidgetInfos.hpp:20` e `Generated/world/ui/WorldLayerWidget.hpp:20`).
//
// O MÉTODO FICA EM `inkUserData`, o PAI — de propósito. O tipo `inkWorldWidgetInfos` não existe
// para o compilador redscript, e declarar tipo que só existe no motor é exatamente a armadilha
// que já derrubou o boot neste projeto (o CMesh-trap, item `#30`). `GetUserData` é native vanilla
// e já devolve `ref<inkUserData>`, então o mod chama o método no que ele tem na mão.
//
// DEPENDÊNCIA confirmada por grep antes de escrever: `inkWidget.GetUserData(CName) ->
// ref<inkUserData>` e `public native class inkUserData extends IScriptable`
// (`cp77-symbols/redscript-all.reds`). O corpo mora no Rust (`register_worldwidget_helper`).

@addMethod(inkUserData)
public native func GetWorldWidgetComponent() -> wref<IComponent>

public abstract class BwmsWorldWidget {

    // O nome que o motor usa para guardar os dados de widget-de-mundo no user-data.
    public static func InfosKey() -> CName = n"inkWorldWidgetInfos"

    // Dado um widget, devolve o componente 3D a que ele está preso — ou `null` se não for um
    // widget de mundo, que é o caso comum e não é erro.
    public static func ComponentOf(widget: ref<inkWidget>) -> wref<IComponent> {
        let vazio: wref<IComponent>;
        if !IsDefined(widget) {
            return vazio;
        }
        let data: ref<inkUserData> = widget.GetUserData(BwmsWorldWidget.InfosKey());
        if !IsDefined(data) {
            return vazio;
        }
        return data.GetWorldWidgetComponent();
    }

    // Este widget vive no mundo 3D? A pergunta barata, para um mod não pagar o resto quando não é.
    public static func IsWorldWidget(widget: ref<inkWidget>) -> Bool {
        return IsDefined(BwmsWorldWidget.ComponentOf(widget));
    }

    // O nome do componente — serve para log e para casar com a entidade dona.
    public static func ComponentNameOf(widget: ref<inkWidget>) -> CName {
        let comp: wref<IComponent> = BwmsWorldWidget.ComponentOf(widget);
        if !IsDefined(comp) {
            return n"";
        }
        return comp.GetName();
    }
}
