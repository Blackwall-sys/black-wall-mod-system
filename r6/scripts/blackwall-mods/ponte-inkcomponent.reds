// ponte-inkcomponent.reds — `inkComponent`, o componente de UI que mods do Windows (Virtual Atelier) estendem sem importar módulo.
//
// Forma do plugue: `new X()` e `Reparent(pai)`; o mod devolve a raiz no `cb func OnCreate() -> ref<inkWidget>` e recebe `OnInitialize` e
// `OnUninitialize`. O componente é um controller do motor: `BwmsWidgetAttachController` (C#175) o põe como `logicController` da raiz, então a
// árvore de widgets segura o componente vivo, o motor chama `OnInitialize`/`OnUninitialize` quando a raiz entra ou sai de uma árvore na tela
// (gancho do `Reparent`, `bwms-widget-building.reds`) e os eventos que o controller pai enfileira chegam aos `cb func` tipados do mod.
// `QueueEvent`, `PlaySound`, `SpawnFromExternal` e os callbacks globais são as natives vanilla do `inkLogicController`.
//
// Classe de script, não nativa: `inkComponent` não existe no motor (string ausente do binário) e forjar tipo ausente é o bloqueio medido em
// `bwms-widget-reparent.reds`. Fica fora de `module` porque os mods usam o tipo sem `import`.

public abstract class inkComponent extends inkLogicController {
    // Forte até a raiz entrar numa árvore; depois quem segura a raiz é a árvore, e a raiz segura o componente (`logicController`).
    private let m_bwmsRaizForte: ref<inkWidget>;
    private let m_bwmsRaiz: wref<inkWidget>;
    private let m_bwmsCriado: Bool;

    protected cb func OnCreate() -> ref<inkWidget> {
        return null;
    }

    private func BwmsMontar() -> Void {
        if this.m_bwmsCriado {
            return;
        }
        this.m_bwmsCriado = true;
        let raiz: ref<inkWidget> = this.OnCreate();
        if !IsDefined(raiz) {
            return;
        }
        this.m_bwmsRaizForte = raiz;
        this.m_bwmsRaiz = raiz;
        // Fora da tela a ligação devolve `false`: o `OnInitialize` vem quando a árvore for anexada.
        let ligou: Bool = BwmsWidgetAttachController(raiz, this, false);
    }

    public func Reparent(newParent: wref<inkCompoundWidget>) -> Void {
        this.BwmsPendurar(newParent, -1);
    }

    public func Reparent(newParent: wref<inkCompoundWidget>, index: Int32) -> Void {
        this.BwmsPendurar(newParent, index);
    }

    private func BwmsPendurar(pai: wref<inkCompoundWidget>, indice: Int32) -> Void {
        this.BwmsMontar();
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        if !IsDefined(raiz) || !IsDefined(pai) {
            return;
        }
        raiz.Reparent(pai, indice);
        this.m_bwmsRaizForte = null;
    }

    public func GetRootWidget() -> wref<inkWidget> {
        return this.m_bwmsRaiz;
    }

    public func GetRootCompoundWidget() -> wref<inkCompoundWidget> {
        return this.m_bwmsRaiz as inkCompoundWidget;
    }

    public func IsAttached() -> Bool {
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        return IsDefined(raiz) && !IsDefined(this.m_bwmsRaizForte);
    }

    // Callbacks de widget vão na raiz: o callback registrado no widget errado não dispara e não avisa (bwms-ui-controller.reds, decisão 3).
    public func RegisterToCallback(eventName: CName, object: ref<IScriptable>, functionName: CName) -> Void {
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        if IsDefined(raiz) {
            raiz.RegisterToCallback(eventName, object, functionName);
        }
    }

    public func UnregisterFromCallback(eventName: CName, object: ref<IScriptable>, functionName: CName) -> Void {
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        if IsDefined(raiz) {
            raiz.UnregisterFromCallback(eventName, object, functionName);
        }
    }
}
