// bwms-ui-popup.reds — diálogo modal de mod.
//
// Os objetos ficam em `array<wref<T>>` no `ScriptableSystem`, nunca `array<ref<T>>`: com `ref`, o motor trata os objetos como posse
// do sistema e, ao destruí-lo no world-swap do autocontinue (`phase=-1`), tenta liberá-los — crash em `red::memory::Free<PoolEngine>`,
// null deref na GameThread. Com `wref` o sistema só observa; quem criou o objeto continua dono dele.
//
// A ESTRUTURA, e por que é uma só: um tipo por combinação (popup de
// jogo, popup de menu, conteúdo, cabeçalho, rodapé...) multiplica arquivo sem multiplicar
// capacidade — as diferenças reais entre eles são o texto e quais botões aparecem. Aqui é UM
// popup com três faixas (cabeçalho, corpo, rodapé) e uma pilha que sabe qual está por cima.
//
// A PILHA existe por um motivo concreto: dois mods podem abrir diálogo ao mesmo tempo. Sem
// alguém rastreando ordem, o ESC fecha o errado, ou fecha os dois, ou nenhum. `BwmsPopupStack`
// responde "quem está no topo" e fecha na ordem certa.
//
// Construído sobre `BwmsUIController` (fila de callbacks + keep-alive) e `BwmsButton`.

// MESMA REGRA DO BOTÃO: nada de pixel absoluto. As margens abaixo são medidas BASE em unidades
// de 1080p, multiplicadas pelo fator da tela real de quem está jogando.
public class BwmsPopup extends BwmsUIController {

    private let m_scale: Float;
    private let m_header: ref<inkText>;
    private let m_body: ref<inkVerticalPanel>;
    private let m_footer: ref<inkHorizontalPanel>;
    private let m_title: String;
    private let m_message: String;
    private let m_buttons: array<ref<BwmsButton>>;
    private let m_visible: Bool;

    // Forma recomendada: com o `game`, o diálogo se dimensiona para a tela de quem joga.
    public static func Create(title: String, message: String, game: GameInstance) -> ref<BwmsPopup> {
        let popup: ref<BwmsPopup> = new BwmsPopup();
        popup.m_title = title;
        popup.m_message = message;
        popup.m_scale = ScreenHelper.GetSmartScale(game);
        popup.Construct();
        return popup;
    }

    // Sem `game`, assume 1080p — e diz isso, em vez de fingir.
    public static func CreateUnscaled(title: String, message: String) -> ref<BwmsPopup> {
        let popup: ref<BwmsPopup> = new BwmsPopup();
        popup.m_title = title;
        popup.m_message = message;
        popup.m_scale = 1.0;
        popup.Construct();
        return popup;
    }

    public static func BasePadX() -> Float = 24.0
    public static func BasePadTop() -> Float = 20.0

    private func Scale() -> Float {
        if this.m_scale <= 0.0 {
            return 1.0;
        }
        return this.m_scale;
    }

    protected func OnCreate() -> ref<inkWidget> {
        let k: Float = this.Scale();
        let padX: Float = BwmsPopup.BasePadX() * k;
        let padTop: Float = BwmsPopup.BasePadTop() * k;

        let root: ref<inkVerticalPanel> = new inkVerticalPanel();
        root.SetName(n"BwmsPopup");
        root.SetFitToContent(true);
        root.SetInteractive(true);

        let header: ref<inkText> = new inkText();
        header.SetName(n"header");
        header.SetText(this.m_title);
        header.SetFitToContent(true);
        header.SetMargin(padX, padTop, padX, padTop * 0.4);
        header.Reparent(root, -1);
        this.m_header = header;

        let body: ref<inkVerticalPanel> = new inkVerticalPanel();
        body.SetName(n"body");
        body.SetFitToContent(true);
        body.SetMargin(padX, 0.0, padX, padTop * 0.8);
        body.Reparent(root, -1);
        this.m_body = body;

        // A mensagem é o primeiro item do corpo; conteúdo extra entra depois via `AddContent`.
        let message: ref<inkText> = new inkText();
        message.SetName(n"message");
        message.SetText(this.m_message);
        message.SetFitToContent(true);
        message.Reparent(body, -1);

        let footer: ref<inkHorizontalPanel> = new inkHorizontalPanel();
        footer.SetName(n"footer");
        footer.SetFitToContent(true);
        footer.SetMargin(padX, 0.0, padX, padTop);
        footer.Reparent(root, -1);
        this.m_footer = footer;

        return root;
    }

    // --- Conteúdo ----------------------------------------------------------------------------

    public func SetTitle(title: String) -> Void {
        this.m_title = title;
        if IsDefined(this.m_header) {
            this.m_header.SetText(title);
        }
    }

    // Pendura qualquer widget do mod no corpo — é o que faz este popup servir pra mais de um caso.
    public func AddContent(widget: ref<inkWidget>) -> Bool {
        if !IsDefined(widget) || !IsDefined(this.m_body) {
            return false;
        }
        widget.Reparent(this.m_body, -1);
        return true;
    }

    // Adiciona botão no rodapé. Guarda referência FORTE — um botão só em `wref` some.
    public func AddButton(text: String, target: ref<IScriptable>, functionName: CName) -> ref<BwmsButton> {
        if !IsDefined(this.m_footer) {
            return null;
        }
        // O botão herda a escala do popup — senão um diálogo escalado ganharia botões fora de
        // proporção em qualquer tela que não seja 1080p.
        let btn: ref<BwmsButton> = BwmsButton.CreateUnscaled(text);
        btn.SetBaseSize(BwmsButton.BaseWidth() * this.Scale(), BwmsButton.BaseHeight() * this.Scale());
        btn.OnClick(target, functionName);
        btn.Mount(this.m_footer);
        ArrayPush(this.m_buttons, btn);
        return btn;
    }

    public func ButtonCount() -> Int32 {
        return ArraySize(this.m_buttons);
    }

    // --- Visibilidade ------------------------------------------------------------------------

    // Monta sob `parent` e entra na pilha. É a chamada que abre o diálogo.
    // `game` é pedido explicitamente porque a pilha vive num `ScriptableSystem` — redscript não
    // tem campo estático mutável (o compilador recusa: `UNSUPPORTED`), então não existe pilha
    // global de verdade sem passar pelo motor.
    public func Show(parent: ref<inkCompoundWidget>, game: GameInstance) -> Bool {
        if !this.Mount(parent) {
            return false;
        }
        this.m_visible = true;
        let root: ref<inkWidget> = this.GetRoot();
        if IsDefined(root) {
            root.SetVisible(true);
        }
        let stack: ref<BwmsPopupStack> = BwmsPopupStack.Get(game);
        if IsDefined(stack) {
            stack.Push(this);
        }
        return true;
    }

    public func Hide(game: GameInstance) -> Void {
        this.m_visible = false;
        let root: ref<inkWidget> = this.GetRoot();
        if IsDefined(root) {
            root.SetVisible(false);
        }
        let stack: ref<BwmsPopupStack> = BwmsPopupStack.Get(game);
        if IsDefined(stack) {
            stack.Remove(this);
        }
        this.Unmount();
    }

    public func IsVisible() -> Bool {
        return this.m_visible;
    }
}

// Pilha de diálogos abertos. É um `ScriptableSystem` porque a ordem é global por natureza (quem
// está por cima não depende de qual mod perguntou) e redscript não permite campo estático
// mutável — o motor instancia e mantém isto sozinho.
public class BwmsPopupStack extends ScriptableSystem {

    private let m_open: array<wref<BwmsPopup>>;

    public static func Get(game: GameInstance) -> ref<BwmsPopupStack> {
        let container: ref<ScriptableSystemsContainer> = GameInstance.GetScriptableSystemsContainer(game);
        if !IsDefined(container) {
            return null;
        }
        return container.Get(n"BwmsPopupStack") as BwmsPopupStack;
    }

    public func Push(popup: wref<BwmsPopup>) -> Void {
        if IsDefined(popup) && !ArrayContains(this.m_open, popup) {
            ArrayPush(this.m_open, popup);
        }
    }

    public func Remove(popup: wref<BwmsPopup>) -> Void {
        let i: Int32 = ArraySize(this.m_open) - 1;
        while i >= 0 {
            if this.m_open[i] == popup {
                ArrayErase(this.m_open, i);
            }
            i -= 1;
        }
    }

    // Quem está por cima — o que o ESC deve fechar.
    public func Top() -> wref<BwmsPopup> {
        let n: Int32 = ArraySize(this.m_open);
        if n == 0 {
            let vazio: wref<BwmsPopup>;
            return vazio;
        }
        return this.m_open[n - 1];
    }

    public func Count() -> Int32 {
        return ArraySize(this.m_open);
    }

    // Fecha só o de cima. Fechar todos de uma vez é decisão do mod, não default.
    public func CloseTop(game: GameInstance) -> Bool {
        let top: wref<BwmsPopup> = this.Top();
        if !IsDefined(top) {
            return false;
        }
        top.Hide(game);
        return true;
    }
}
