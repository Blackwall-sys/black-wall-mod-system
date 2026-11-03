// bwms-ui-controller.reds — controller de UI customizado (catálogo Codeware #88/#90/#179): a base sobre a qual botões e popups são
// construídos.
//
// O QUE ESTA CLASSE RESOLVE: a engine só chama `OnInitialize` num controller que está anexado a
// um widget que existe. Um mod que constrói a própria UI vive o problema inverso — ele tem o
// controller antes de ter o widget. Sem uma máquina de estado explícita no meio, o resultado é
// UI que "não faz nada" sem erro nenhum no log, que é o modo de falha mais caro de depurar.
//
// TRÊS DECISÕES QUE VÊM DE BUG REAL PAGO POR ESTE PROJETO — não são preferência de estilo:
//
//  1. **Keep-alive forte do widget-raiz** (`m_rootStrong`). `wref<>` não segura nada: o widget
//     construído dentro de `OnCreate()` morre ao sair de escopo e a classe inteira vira no-op
//     silencioso. Isso já custou dois itens do catálogo antes de ser entendido (memória
//     `cw-ui-keepalive-wref`). Todo widget que esta classe guarda tem uma referência forte.
//
//  2. **Fila de callbacks pendentes** (`m_pending`). Registrar um callback antes de a raiz
//     existir era descartado em silêncio. Aqui o pedido fica na fila e é drenado no instante em
//     que a raiz aparece — a ordem em que o mod chama as coisas deixa de importar.
//
//  3. **Callback vai no WIDGET, não no game controller.** Registrar no lugar errado faz
//     `OnEnter`/`OnPress`/clique simplesmente nunca dispararem, também sem erro. Este é o bug
//     concreto que a correção de 2026-08-21 achou.
//
// DIVERGÊNCIA DE API, deliberada: a engine dispara `OnCreate` sozinha só para controller anexado
// pela via nativa real, que um mod não tem. Aqui a construção é EXPLÍCITA (`Construct()`), e
// idempotente — chamar duas vezes não constrói duas árvores.
//
// SHADOW de `GetRootWidget`/`GetRootCompoundWidget`: o pai (`inkLogicController`) declara os dois
// como `final native`, mas eles leem o campo VANILLA do motor, que só a via nativa popula — para
// um controller nosso devolveriam null para sempre. O shadow por tipo estático da subclasse é
// padrão já provado ao vivo neste projeto.
//
// DEPENDÊNCIAS confirmadas por grep ANTES de escrever (`cp77-symbols/redscript-all.reds`):
//   - `class inkLogicController extends inkILogicController`                (`:183022`)
//   - `inkWidget.RegisterToCallback(eventName, object, functionName)`       (`:54380`)
//   - `inkWidget.Reparent(parent, index)`                                   (`:54808`)
//   - `inkCompoundWidget.GetNumChildren`/`GetWidgetByIndex`                 (`:55017`/`:55025`)
//   - `inkWidget.BwmsAttachController(...)` — contorno do item `#87`, já provado ao vivo.

// Um pedido de callback que chegou antes da raiz existir.
public struct BwmsPendingCallback {
    public let EventName: CName;
    public let Target: ref<IScriptable>;
    public let FunctionName: CName;
}

public abstract class BwmsUIController extends inkLogicController {

    private let m_root: wref<inkWidget>;
    // Referência FORTE — ver decisão 1 no topo. Sem isto, nada disto funciona.
    private let m_rootStrong: ref<inkWidget>;
    private let m_pending: array<BwmsPendingCallback>;
    private let m_created: Bool;
    private let m_initialized: Bool;

    // --- Ciclo de vida ---------------------------------------------------------------------

    // Constrói a árvore da subclasse. Idempotente de propósito: um mod pode chamar antes de
    // montar e a montagem chama de novo, sem duplicar nada.
    public func Construct() -> Void {
        if this.m_created {
            return;
        }
        this.m_created = true;
        let root: ref<inkWidget> = this.OnCreate();
        if IsDefined(root) {
            this.SetRoot(root);
        }
    }

    // A subclasse devolve aqui o widget-raiz que ela montou. Retorna widget (e não Bool como um
    // `cb func`) porque quem constrói é quem sabe qual é a raiz.
    protected func OnCreate() -> ref<inkWidget> {
        return null;
    }

    // Chamado uma vez, assim que existe raiz. É onde a subclasse liga listeners e preenche dados.
    protected cb func OnInitialize() -> Bool {
        return true;
    }

    // Chamado quando o controller é desmontado. A subclasse solta o que segurou.
    protected cb func OnUninitialize() -> Bool {
        return true;
    }

    // --- Raiz ------------------------------------------------------------------------------

    // Define a raiz, prende a referência forte, anexa o controller ao widget e — o ponto que
    // importa — DRENA a fila de callbacks que chegaram cedo demais.
    public func SetRoot(root: ref<inkWidget>) -> Void {
        if !IsDefined(root) {
            return;
        }
        this.m_root = root;
        this.m_rootStrong = root;
        root.BwmsAttachController(this);

        let queued: Int32 = ArraySize(this.m_pending);
        if queued > 0 {
            let i: Int32 = 0;
            while i < queued {
                let item: BwmsPendingCallback = this.m_pending[i];
                // No WIDGET — ver decisão 3 no topo.
                root.RegisterToCallback(item.EventName, item.Target, item.FunctionName);
                i += 1;
            }
            ArrayClear(this.m_pending);
        }

        if !this.m_initialized {
            this.m_initialized = true;
            this.OnInitialize();
        }
    }

    public func GetRoot() -> wref<inkWidget> {
        return this.m_root;
    }

    // Shadow — ver nota no topo.
    public func GetRootWidget() -> wref<inkWidget> {
        return this.m_root;
    }

    public func GetRootCompoundWidget() -> wref<inkCompoundWidget> {
        return this.m_root as inkCompoundWidget;
    }

    public func IsConstructed() -> Bool {
        return this.m_created;
    }

    public func IsInitialized() -> Bool {
        return this.m_initialized;
    }

    // --- Callbacks -------------------------------------------------------------------------

    // Liga um callback do widget. Se a raiz ainda não existe, o pedido entra na fila em vez de
    // ser perdido — ver decisão 2 no topo.
    public func On(eventName: CName, target: ref<IScriptable>, functionName: CName) -> Void {
        let root: ref<inkWidget> = this.m_rootStrong;
        if IsDefined(root) {
            root.RegisterToCallback(eventName, target, functionName);
        } else {
            let item: BwmsPendingCallback;
            item.EventName = eventName;
            item.Target = target;
            item.FunctionName = functionName;
            ArrayPush(this.m_pending, item);
        }
    }

    // Atalho para o caso dominante: o próprio controller é quem trata o evento.
    public func OnSelf(eventName: CName, functionName: CName) -> Void {
        this.On(eventName, this, functionName);
    }

    public func PendingCallbackCount() -> Int32 {
        return ArraySize(this.m_pending);
    }

    // --- Montagem --------------------------------------------------------------------------

    // Constrói (se preciso) e pendura a raiz sob `parent`. É a chamada que um mod faz.
    public func Mount(parent: ref<inkCompoundWidget>) -> Bool {
        return this.MountAt(parent, -1);
    }

    public func MountAt(parent: ref<inkCompoundWidget>, index: Int32) -> Bool {
        this.Construct();
        let root: ref<inkWidget> = this.m_rootStrong;
        if !IsDefined(root) || !IsDefined(parent) {
            return false;
        }
        root.Reparent(parent, index);
        return true;
    }

    // Tira a raiz da árvore e avisa a subclasse. Mantém a referência forte: um controller
    // desmontado pode ser remontado sem reconstruir tudo.
    public func Unmount() -> Void {
        if this.m_initialized {
            this.m_initialized = false;
            this.OnUninitialize();
        }
    }
}
