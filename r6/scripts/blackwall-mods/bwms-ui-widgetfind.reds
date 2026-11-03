// bwms-ui-widgetfind.reds — busca na árvore de widgets (catálogo Codeware #89).
//
// As duas direções, porque servem a perguntas diferentes: descer responde "o que existe dentro disto" (montagem, busca por tipo) e
// subir responde "onde isto está" (contexto, ancestralidade) sem precisar da raiz na mão. Subir usa o `GetParentWidget` que o BWMS
// adiciona a `inkWidget` (`bwms-ui-inkwidget-native.reds`); o redscript vanilla não tem esse método.
//
// TRAVESSIA POR PILHA EXPLÍCITA, não recursão: é o estilo do projeto (procedural/data-oriented),
// e aqui tem razão técnica — árvore de UI vinda de mod é de profundidade desconhecida, e o teto
// de passos abaixo é uma proteção real contra árvore cíclica ou monstruosa travando um frame.
//
// DEPENDÊNCIAS confirmadas por grep ANTES de escrever (`cp77-symbols/redscript-all.reds`):
//   - `inkCompoundWidget.GetNumChildren() -> Int32`                    (`:55017`)
//   - `inkCompoundWidget.GetWidgetByIndex(index) -> wref<inkWidget>`   (`:55025`)
//   - `inkWidget.GetControllers() -> [wref<inkLogicController>]`       (`:54597`)
//   - `IScriptable.IsA(className: CName) -> Bool`                      (`:11260`)
//   - `inkWidget.BwmsGetController()` — contorno do item `#87`, já provado ao vivo, mora em
//     `bwms-inkwidget-ext.reds`.

public abstract class BwmsWidgetFind {

    // Teto de nós visitados. Árvore de UI real tem dezenas de nós; milhares indica ciclo ou
    // engano do chamador, e travar um frame de UI é pior do que devolver busca incompleta.
    public static func MaxNodes() -> Int32 = 4096

    // Todos os controllers de um tipo, na subárvore de `root` (inclusive o próprio `root`).
    // Devolve quantos achou. Considera tanto os controllers VANILLA do widget quanto o que foi
    // anexado pelo contorno do `#87` — um mod não deveria precisar saber por qual via o
    // controller chegou ali.
    public static func CollectByType(root: ref<inkWidget>, typeName: CName,
                                     out found: array<wref<inkLogicController>>) -> Int32 {
        ArrayClear(found);
        if !IsDefined(root) {
            return 0;
        }
        let pending: array<wref<inkWidget>>;
        ArrayPush(pending, root);
        let visited: Int32 = 0;

        while ArraySize(pending) > 0 && visited < BwmsWidgetFind.MaxNodes() {
            let last: Int32 = ArraySize(pending) - 1;
            let current: wref<inkWidget> = pending[last];
            ArrayErase(pending, last);
            visited += 1;
            // redscript não tem `continue` (achado registrado 2026-08-21) — guarda invertida.
            if IsDefined(current) {
                BwmsWidgetFind.CollectFromWidget(current, typeName, found);

                // Empilha os filhos, se for container.
                let container: ref<inkCompoundWidget> = current as inkCompoundWidget;
                if IsDefined(container) {
                    let count: Int32 = container.GetNumChildren();
                    let i: Int32 = 0;
                    while i < count {
                        let child: wref<inkWidget> = container.GetWidgetByIndex(i);
                        if IsDefined(child) {
                            ArrayPush(pending, child);
                        }
                        i += 1;
                    }
                }
            }
        }
        return ArraySize(found);
    }

    // O primeiro que casar, sem montar a lista inteira — o caso comum ("me dá o controller X").
    public static func FirstByType(root: ref<inkWidget>, typeName: CName) -> wref<inkLogicController> {
        let all: array<wref<inkLogicController>>;
        if BwmsWidgetFind.CollectByType(root, typeName, all) > 0 {
            return all[0];
        }
        let empty: wref<inkLogicController>;
        return empty;
    }

    // `target` está na subárvore de `root`? Substitui honestamente o antigo `InWindowTree`:
    // mesma pergunta ("este widget está montado dentro daquilo?"), só que respondida de cima
    // para baixo, que é o que a engine permite.
    public static func IsUnder(root: ref<inkWidget>, target: ref<inkWidget>) -> Bool {
        if !IsDefined(root) || !IsDefined(target) {
            return false;
        }
        let pending: array<wref<inkWidget>>;
        ArrayPush(pending, root);
        let visited: Int32 = 0;

        while ArraySize(pending) > 0 && visited < BwmsWidgetFind.MaxNodes() {
            let last: Int32 = ArraySize(pending) - 1;
            let current: wref<inkWidget> = pending[last];
            ArrayErase(pending, last);
            visited += 1;
            if IsDefined(current) {
                if current == target {
                    return true;
                }
                let container: ref<inkCompoundWidget> = current as inkCompoundWidget;
                if IsDefined(container) {
                    let count: Int32 = container.GetNumChildren();
                    let i: Int32 = 0;
                    while i < count {
                        let child: wref<inkWidget> = container.GetWidgetByIndex(i);
                        if IsDefined(child) {
                            ArrayPush(pending, child);
                        }
                        i += 1;
                    }
                }
            }
        }
        return false;
    }

    // Quantos nós a subárvore tem — diagnóstico barato pra mod que suspeita ter montado UI demais.
    public static func CountNodes(root: ref<inkWidget>) -> Int32 {
        if !IsDefined(root) {
            return 0;
        }
        let pending: array<wref<inkWidget>>;
        ArrayPush(pending, root);
        let visited: Int32 = 0;
        while ArraySize(pending) > 0 && visited < BwmsWidgetFind.MaxNodes() {
            let last: Int32 = ArraySize(pending) - 1;
            let current: wref<inkWidget> = pending[last];
            ArrayErase(pending, last);
            visited += 1;
            let container: ref<inkCompoundWidget> = current as inkCompoundWidget;
            if IsDefined(container) {
                let count: Int32 = container.GetNumChildren();
                let i: Int32 = 0;
                while i < count {
                    let child: wref<inkWidget> = container.GetWidgetByIndex(i);
                    if IsDefined(child) {
                        ArrayPush(pending, child);
                    }
                    i += 1;
                }
            }
        }
        return visited;
    }

    // --- Subir a árvore (recuperado em 2026-08-22) ------------------------------------------

    // O ancestral imediato. `null` no topo — quem chama distingue "sou raiz" de "não achei".
    public static func Parent(widget: ref<inkWidget>) -> ref<inkWidget> {
        if !IsDefined(widget) {
            return null;
        }
        return widget.GetParentWidget();
    }

    // Sobe até o topo da árvore em que o widget está montado. Serve para responder "de qual
    // janela/tela isto faz parte" sem o mod precisar guardar a raiz.
    public static func Root(widget: ref<inkWidget>) -> ref<inkWidget> {
        let atual: ref<inkWidget> = widget;
        let passos: Int32 = 0;
        while IsDefined(atual) && passos < BwmsWidgetFind.MaxNodes() {
            let pai: ref<inkWidget> = atual.GetParentWidget();
            if !IsDefined(pai) {
                return atual;
            }
            atual = pai;
            passos += 1;
        }
        return atual;
    }

    // Profundidade a partir do topo — diagnóstico barato de "montei onde?".
    public static func Depth(widget: ref<inkWidget>) -> Int32 {
        let atual: ref<inkWidget> = widget;
        let n: Int32 = 0;
        while IsDefined(atual) && n < BwmsWidgetFind.MaxNodes() {
            let pai: ref<inkWidget> = atual.GetParentWidget();
            if !IsDefined(pai) {
                return n;
            }
            atual = pai;
            n += 1;
        }
        return n;
    }

    // O widget está montado numa árvore de verdade, ou solto? Um widget solto não renderiza e não
    // resolve estilo — é a causa nº 1 de "criei e não apareceu".
    public static func IsMounted(widget: ref<inkWidget>) -> Bool {
        if !IsDefined(widget) {
            return false;
        }
        return IsDefined(widget.GetParentWidget());
    }

    // O ancestral mais próximo de um tipo — responde "estou dentro de qual container/tela".
    public static func FirstAncestorOfType(widget: ref<inkWidget>, typeName: CName) -> ref<inkWidget> {
        let atual: ref<inkWidget> = BwmsWidgetFind.Parent(widget);
        let passos: Int32 = 0;
        while IsDefined(atual) && passos < BwmsWidgetFind.MaxNodes() {
            if atual.IsA(typeName) {
                return atual;
            }
            atual = atual.GetParentWidget();
            passos += 1;
        }
        return null;
    }

    // Junta, para UM widget, os controllers vanilla e o anexado pelo contorno do `#87`.
    private static func CollectFromWidget(widget: wref<inkWidget>, typeName: CName,
                                          out found: array<wref<inkLogicController>>) -> Void {
        let vanilla: array<wref<inkLogicController>> = widget.GetControllers();
        for ctrl in vanilla {
            if IsDefined(ctrl) && ctrl.IsA(typeName) && !ArrayContains(found, ctrl) {
                ArrayPush(found, ctrl);
            }
        }
        let attached: ref<inkLogicController> = widget.BwmsGetController();
        if IsDefined(attached) && attached.IsA(typeName) {
            let asWeak: wref<inkLogicController> = attached;
            if !ArrayContains(found, asWeak) {
                ArrayPush(found, asWeak);
            }
        }
    }
}
