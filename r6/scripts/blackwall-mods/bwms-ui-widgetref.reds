// bwms-ui-widgetref.reds — referência de widget (`inkWidgetRef` e as filhas tipadas) a partir de um widget criado pelo mod (catálogo Codeware #95).
//
// O jogo só preenche `inkWidgetRef` pelo editor: é o campo `edit` de um controller, ligado ao widget na hora em que a biblioteca é instanciada.
// Script não tem como montar um pra um widget criado em runtime, e metade da API de UI do jogo (tudo que recebe `inkTextRef`, `inkImageRef`,
// `inkCompoundRef`...) fica fora do alcance de um mod que monta a própria tela.
//
// O struct vanilla tem o campo `widget` no RTTI, mas o script não o declara. A escrita vai pela native `BwmsInkWidgetRefSet` (Rust,
// `tramp_inkwidgetref_set`, interface/mod.rs), que grava o handle no campo e mais nada. Montada a referência, a API vanilla inteira
// (`inkWidgetRef.Get`, `SetName`, `SetVisible`, `inkTextRef.SetText`, `inkTextRef.GetFontSize`...) funciona sobre o widget do mod.
//
// Uma função por tipo porque o tipo da referência é o que a API vanilla pede: `inkTextRef.SetText` não aceita `inkWidgetRef`.
// `inkGrid` não existe nesta build do jogo (o `inkGridRef` existe, mas não há widget pra pôr nele), então não há `Grid`.
// `inkBorder`, `inkCacheWidget` e `inkVirtualCompoundWidget` são abstratos: script não os cria, mas o motor sim, e a referência
// serve pra instância que o mod achar na árvore.

native func BwmsInkWidgetRefSet(self: script_ref<inkWidgetRef>, widget: ref<inkWidget>) -> Void;

public abstract class BwmsWidgetRef {

    // Liga uma referência que já existe (campo de controller, variável) a outro widget.
    public static func Bind(target: script_ref<inkWidgetRef>, widget: ref<inkWidget>) -> Void {
        BwmsInkWidgetRefSet(target, widget);
    }

    public static func Of(widget: ref<inkWidget>) -> inkWidgetRef {
        let r: inkWidgetRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Text(widget: ref<inkText>) -> inkTextRef {
        let r: inkTextRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Image(widget: ref<inkImage>) -> inkImageRef {
        let r: inkImageRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Video(widget: ref<inkVideo>) -> inkVideoRef {
        let r: inkVideoRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Mask(widget: ref<inkMask>) -> inkMaskRef {
        let r: inkMaskRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Border(widget: ref<inkBorder>) -> inkBorderRef {
        let r: inkBorderRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Rectangle(widget: ref<inkRectangle>) -> inkRectangleRef {
        let r: inkRectangleRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Circle(widget: ref<inkCircle>) -> inkCircleRef {
        let r: inkCircleRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Shape(widget: ref<inkShape>) -> inkShapeRef {
        let r: inkShapeRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Canvas(widget: ref<inkCanvas>) -> inkCanvasRef {
        let r: inkCanvasRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Flex(widget: ref<inkFlex>) -> inkFlexRef {
        let r: inkFlexRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func HorizontalPanel(widget: ref<inkHorizontalPanel>) -> inkHorizontalPanelRef {
        let r: inkHorizontalPanelRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func VerticalPanel(widget: ref<inkVerticalPanel>) -> inkVerticalPanelRef {
        let r: inkVerticalPanelRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func UniformGrid(widget: ref<inkUniformGrid>) -> inkUniformGridRef {
        let r: inkUniformGridRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func ScrollArea(widget: ref<inkScrollArea>) -> inkScrollAreaRef {
        let r: inkScrollAreaRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func Cache(widget: ref<inkCacheWidget>) -> inkCacheRef {
        let r: inkCacheRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }

    public static func VirtualCompound(widget: ref<inkVirtualCompoundWidget>) -> inkVirtualCompoundRef {
        let r: inkVirtualCompoundRef;
        BwmsInkWidgetRefSet(r, widget);
        return r;
    }
}
