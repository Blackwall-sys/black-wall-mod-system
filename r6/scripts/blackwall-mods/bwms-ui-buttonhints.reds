// bwms-ui-buttonhints.reds — a barra de dicas de botão do próprio jogo dentro de qualquer widget de mod (catálogo Codeware #97 e #98).
//
// A barra é a do jogo: biblioteca `base\gameplay\gui\common\buttonhints.inkwidget`, item `Root`, controller vanilla `ButtonHints`. Ela já
// sabe desenhar o ícone certo da ação pro dispositivo em uso (teclado ou controle), localizar o rótulo e se parecer com as telas do jogo.
// O que falta a um mod é criar a barra dentro do widget DELE e mexer nela depois — é isso que este arquivo dá.
//
// Como:
//   - `Spawn(controller, pai)` cria a barra direto no pai. Primeiro pelo `SpawnFromExternal` do game controller que o mod já tem na mão, que
//     só funciona quando a biblioteca da barra é dependência da biblioteca desse controller. Na raiz do HUD não é: o síncrono deu nulo (boot
//     253), continuou nulo mesmo com a biblioteca carregada pelo loader do motor, e o `AsyncSpawnFromExternal` respondeu com widget nulo
//     (boots 254 e 256). Na falta, a barra nasce de uma barra viva do jogo na árvore do controller, pelo `SpawnFromLocal` do controller dela —
//     pra ela a biblioteca é local (boot 256, HUD). `SpawnPath()` diz qual via criou. A barra nasce montada na árvore e o dono dela é o pai;
//     não há molde guardado, serviço global nem gancho em tela do jogo.
//   - O objeto devolvido só guarda referências fracas. O mod segura o objeto se quiser mexer depois; soltar o objeto não apaga a barra.
//   - `Lock` congela a barra: inclusões e remoções são ignoradas até `Unlock` (um popup não tem as dicas trocadas por baixo dele).
//   - `SetStyle(n"popup")` dá o espaçamento e o texto dos rodapés de popup do jogo; `n""` volta ao padrão.
//   - `ActionKey(action)` devolve o nome do ícone que o jogo resolveu pra ação no dispositivo atual, lido do `inkInputDisplayController`
//     de um item da própria barra.

@addMethod(ButtonHintListItem)
public func BwmsInputDisplay() -> wref<inkInputDisplayController> {
    return this.m_buttonHint;
}

@addMethod(ButtonHintListItem)
public func BwmsRotulo() -> wref<inkText> {
    let w: wref<inkWidget> = inkWidgetRef.Get(this.m_label);
    let forte: ref<inkWidget> = w;
    return forte as inkText;
}

public class BwmsButtonHints extends IScriptable {

    private let m_raiz: wref<inkWidget>;
    private let m_barra: wref<ButtonHints>;
    private let m_travada: Bool;
    private let m_estilo: CName;
    private let m_via: Int32;

    // --- Criação -------------------------------------------------------------------------------------------------------------------------

    public static func Spawn(controller: ref<inkGameController>, parent: ref<inkCompoundWidget>) -> ref<BwmsButtonHints> {
        if !IsDefined(controller) || !IsDefined(parent) {
            return null;
        };
        let criado: wref<inkWidget> = controller.SpawnFromExternal(parent, r"base\\gameplay\\gui\\common\\buttonhints.inkwidget", n"Root");
        let raiz: ref<inkWidget> = criado;
        let via: Int32 = 1;
        if !IsDefined(raiz) {
            via = 2;
            let raizDoController: wref<inkWidget> = controller.GetRootWidget();
            let raizForte: ref<inkWidget> = raizDoController;
            let existente: ref<ButtonHints> = BwmsButtonHints.AcharBarra(raizForte);
            if IsDefined(existente) {
                let viaLocal: wref<inkWidget> = existente.SpawnFromLocal(parent, n"Root");
                raiz = viaLocal;
            };
        };
        let h: ref<BwmsButtonHints> = BwmsButtonHints.Wrap(raiz);
        if IsDefined(h) {
            h.m_via = via;
        };
        return h;
    }

    // Qual via criou a barra: 1 `SpawnFromExternal` direto, 2 a partir de uma barra viva do jogo na árvore do controller; 0 no `Wrap`.
    public func SpawnPath() -> Int32 {
        return this.m_via;
    }

    // A 1ª barra de dicas viva na árvore (busca em largura, até 4096 nós).
    private static func AcharBarra(raiz: ref<inkWidget>) -> ref<ButtonHints> {
        let fila: array<ref<inkWidget>>;
        if IsDefined(raiz) {
            ArrayPush(fila, raiz);
        };
        let i: Int32 = 0;
        while i < ArraySize(fila) && i < 4096 {
            let w: ref<inkWidget> = fila[i];
            let c: wref<inkLogicController> = w.GetController();
            let forte: ref<inkLogicController> = c;
            let barra: ref<ButtonHints> = forte as ButtonHints;
            if IsDefined(barra) {
                return barra;
            };
            let comp: ref<inkCompoundWidget> = w as inkCompoundWidget;
            if IsDefined(comp) {
                let n: Int32 = comp.GetNumChildren();
                let k: Int32 = 0;
                while k < n {
                    let f: wref<inkWidget> = comp.GetWidgetByIndex(k);
                    let ff: ref<inkWidget> = f;
                    if IsDefined(ff) {
                        ArrayPush(fila, ff);
                    };
                    k += 1;
                };
            };
            i += 1;
        };
        return null;
    }

    // Envolve uma barra que já existe (a de uma tela do jogo, ou uma criada pelo mod de outro jeito).
    public static func Wrap(root: ref<inkWidget>) -> ref<BwmsButtonHints> {
        if !IsDefined(root) {
            return null;
        };
        let c: wref<inkLogicController> = root.GetController();
        let forte: ref<inkLogicController> = c;
        let barra: ref<ButtonHints> = forte as ButtonHints;
        if !IsDefined(barra) {
            return null;
        };
        let h: ref<BwmsButtonHints> = new BwmsButtonHints();
        h.m_raiz = root;
        h.m_barra = barra;
        return h;
    }

    public func IsValid() -> Bool {
        return IsDefined(this.m_barra);
    }

    public func GetRoot() -> wref<inkWidget> {
        return this.m_raiz;
    }

    public func GetController() -> wref<ButtonHints> {
        return this.m_barra;
    }

    // Tira a barra do pai. Depois disto o objeto fica inválido.
    public func Destroy() -> Void {
        let raiz: ref<inkWidget> = this.m_raiz;
        let pai: ref<inkWidget> = BwmsWidgetFind.Parent(raiz);
        let paiComposto: ref<inkCompoundWidget> = pai as inkCompoundWidget;
        if IsDefined(paiComposto) {
            paiComposto.RemoveChild(raiz);
        };
        this.m_raiz = null;
        this.m_barra = null;
    }

    // --- Dicas ---------------------------------------------------------------------------------------------------------------------------

    public func Add(action: CName, label: String) -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.AddButtonHint(action, label);
        this.EstilizarItem(this.Item(action));
    }

    // Rótulo por chave de localização.
    public func AddLocalized(action: CName, labelKey: CName) -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.AddButtonHint(action, labelKey);
        this.EstilizarItem(this.Item(action));
    }

    // Dica de "segurar" (o jogo prefixa o rótulo com o texto de segurar).
    public func AddHold(action: CName, labelKey: CName) -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.AddButtonHint(action, labelKey, true);
        this.EstilizarItem(this.Item(action));
    }

    // A dica de girar o personagem das telas de inventário e espelho.
    public func AddRotate() -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.AddCharacterRoatateButtonHint();
    }

    public func Remove(action: CName) -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.RemoveButtonHint(action);
    }

    public func Clear() -> Void {
        if this.m_travada || !IsDefined(this.m_barra) {
            return;
        };
        this.m_barra.ClearButtonHints();
    }

    public func Has(action: CName) -> Bool {
        let item: ref<ButtonHintListItem> = this.Item(action);
        return IsDefined(item);
    }

    // Quantas dicas a barra mostra (a de girar conta).
    public func Count() -> Int32 {
        let holder: ref<inkCompoundWidget> = this.Holder();
        if !IsDefined(holder) {
            return 0;
        };
        return holder.GetNumChildren();
    }

    // --- Visibilidade e trava ------------------------------------------------------------------------------------------------------------

    public func Show() -> Void {
        if IsDefined(this.m_barra) {
            this.m_barra.Show();
        };
    }

    public func Hide() -> Void {
        if IsDefined(this.m_barra) {
            this.m_barra.Hide();
        };
    }

    public func IsVisible() -> Bool {
        if !IsDefined(this.m_barra) {
            return false;
        };
        return this.m_barra.IsVisible();
    }

    public func Lock() -> Void {
        this.m_travada = true;
    }

    public func Unlock() -> Void {
        this.m_travada = false;
    }

    public func IsLocked() -> Bool {
        return this.m_travada;
    }

    // --- Estilo --------------------------------------------------------------------------------------------------------------------------

    public func SetStyle(style: CName) -> Void {
        this.m_estilo = style;
        let holder: ref<inkCompoundWidget> = this.Holder();
        if !IsDefined(holder) {
            return;
        };
        if Equals(style, n"popup") {
            holder.SetChildMargin(new inkMargin(30.0, 0.0, 0.0, 0.0));
        } else {
            holder.SetChildMargin(new inkMargin(0.0, 0.0, 0.0, 0.0));
        };
        let n: Int32 = holder.GetNumChildren();
        let i: Int32 = 0;
        while i < n {
            let w: wref<inkWidget> = holder.GetWidgetByIndex(i);
            let c: wref<inkLogicController> = w.GetController();
            let forte: ref<inkLogicController> = c;
            this.EstilizarItem(forte as ButtonHintListItem);
            i += 1;
        };
    }

    public func GetStyle() -> CName {
        return this.m_estilo;
    }

    // --- Tecla da ação -------------------------------------------------------------------------------------------------------------------

    // Nome do ícone que o jogo mostra pra ação no dispositivo atual (a parte da textura do ícone de entrada). Usa o item da ação se a barra
    // já tiver um; senão põe um item provisório e tira em seguida (se a barra estiver travada, não põe e devolve vazio).
    public func ActionKey(action: CName) -> String {
        if !IsDefined(this.m_barra) {
            return "";
        };
        let item: ref<ButtonHintListItem> = this.Item(action);
        let provisorio: Bool = false;
        if !IsDefined(item) {
            if this.m_travada {
                return "";
            };
            this.m_barra.AddButtonHint(action, "");
            item = this.Item(action);
            provisorio = true;
        };
        let nome: String = "";
        if IsDefined(item) {
            let disp: wref<inkInputDisplayController> = item.BwmsInputDisplay();
            let dispForte: ref<inkInputDisplayController> = disp;
            if IsDefined(dispForte) {
                dispForte.SetInputAction(action);
                let icone: ref<inkImage> = BwmsButtonHints.IconeDe(dispForte);
                if IsDefined(icone) {
                    let parte: CName = icone.GetTexturePart();
                    nome = NameToString(parte);
                };
            };
        };
        if provisorio {
            this.m_barra.RemoveButtonHint(action);
        };
        return nome;
    }

    // --- Interno -------------------------------------------------------------------------------------------------------------------------

    private func Holder() -> ref<inkCompoundWidget> {
        if !IsDefined(this.m_barra) {
            return null;
        };
        let w: wref<inkWidget> = inkWidgetRef.Get(this.m_barra.m_horizontalHolder);
        let forte: ref<inkWidget> = w;
        return forte as inkCompoundWidget;
    }

    private func Item(action: CName) -> ref<ButtonHintListItem> {
        let holder: ref<inkCompoundWidget> = this.Holder();
        if !IsDefined(holder) {
            return null;
        };
        let n: Int32 = holder.GetNumChildren();
        let i: Int32 = 0;
        while i < n {
            let w: wref<inkWidget> = holder.GetWidgetByIndex(i);
            let c: wref<inkLogicController> = w.GetController();
            let forte: ref<inkLogicController> = c;
            let item: ref<ButtonHintListItem> = forte as ButtonHintListItem;
            if IsDefined(item) {
                let casa: Bool = item.CheckAction(action);
                if casa {
                    return item;
                };
            };
            i += 1;
        };
        return null;
    }

    // Estilo popup: rótulo em caixa alta, semi-negrito, corpo 38, como os rodapés de popup do jogo.
    private func EstilizarItem(item: ref<ButtonHintListItem>) -> Void {
        if !IsDefined(item) || !Equals(this.m_estilo, n"popup") {
            return;
        };
        let fraco: wref<inkText> = item.BwmsRotulo();
        let rotulo: ref<inkText> = fraco;
        if IsDefined(rotulo) {
            rotulo.SetFontSize(38);
            rotulo.SetFontStyle(n"Semi-Bold");
            rotulo.SetLetterCase(textLetterCase.UpperCase);
        };
    }

    private static func IconeDe(disp: ref<inkInputDisplayController>) -> ref<inkImage> {
        let porNome: wref<inkWidget> = disp.GetWidget(n"inputRoot/inputIcon");
        let porNomeForte: ref<inkWidget> = porNome;
        let img: ref<inkImage> = porNomeForte as inkImage;
        if IsDefined(img) {
            return img;
        };
        let raiz: wref<inkWidget> = disp.GetRootWidget();
        let raizForte: ref<inkWidget> = raiz;
        return BwmsButtonHints.PrimeiraImagem(raizForte, 0);
    }

    private static func PrimeiraImagem(w: ref<inkWidget>, nivel: Int32) -> ref<inkImage> {
        if !IsDefined(w) || nivel > 8 {
            return null;
        };
        let img: ref<inkImage> = w as inkImage;
        if IsDefined(img) {
            return img;
        };
        let comp: ref<inkCompoundWidget> = w as inkCompoundWidget;
        if !IsDefined(comp) {
            return null;
        };
        let n: Int32 = comp.GetNumChildren();
        let i: Int32 = 0;
        while i < n {
            let filho: wref<inkWidget> = comp.GetWidgetByIndex(i);
            let filhoForte: ref<inkWidget> = filho;
            let achado: ref<inkImage> = BwmsButtonHints.PrimeiraImagem(filhoForte, nivel + 1);
            if IsDefined(achado) {
                return achado;
            };
            i += 1;
        };
        return null;
    }
}
