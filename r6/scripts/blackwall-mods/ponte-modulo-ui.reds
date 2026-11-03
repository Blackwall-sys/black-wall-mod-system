module Codeware.UI

// ponte-modulo-ui.reds — o módulo `Codeware.UI` que mods do Windows importam (`import Codeware.UI.*`): `inkCustomController`,
// `SimpleButton`, `HubTextInput` e o popup de menu (`InMenuPopup`, `InMenuPopupContent`, `InMenuPopupFooter`, `PopupButton`).
//
// Do Windows vem só a forma do plugue (nome, assinatura, ordem das chamadas que os mods fazem); o corpo é do BWMS, escrito a partir do uso
// em Equipment-EX e Virtual Atelier. Cada controller é controller do motor: `BwmsWidgetAttachController` (C#175) o liga à raiz como
// `logicController`, a árvore de widgets o segura vivo, o motor chama `OnInitialize` quando a raiz entra numa árvore na tela, e
// `QueueEvent`, `PlaySound`, `SpawnFromExternal` e os callbacks globais são as natives vanilla do `inkLogicController`.
//
// Só classes de script aqui: native declarada dentro de `module` derruba o bind (memória cp77-redscript-module-plus-nativefunc-crash); as
// natives usadas estão declaradas em escopo global. Nenhuma classe tem os campos `detachedWidget` e `gameController`: com os dois o gancho do
// `OnInitialize` trata o controller como o `inkCustomController` antigo e o motor não o chama (`bwms-widget-building.reds`).

public abstract class inkCustomController extends inkLogicController {
    // Forte até a raiz entrar numa árvore; depois a árvore segura a raiz e a raiz segura o controller (`logicController`).
    private let m_bwmsRaizForte: ref<inkWidget>;
    private let m_bwmsRaiz: wref<inkWidget>;
    private let m_bwmsCriado: Bool;
    private let m_bwmsControladorDoJogo: wref<inkGameController>;

    // O mod monta os widgets e chama `SetRootWidget`.
    protected cb func OnCreate() -> Void {}

    protected func OnReparent(parent: ref<inkCompoundWidget>) -> Void {}

    public func CreateInstance() -> Void {
        if this.m_bwmsCriado {
            return;
        }
        this.m_bwmsCriado = true;
        this.OnCreate();
    }

    public func SetRootWidget(root: ref<inkWidget>) -> Void {
        if !IsDefined(root) {
            return;
        }
        this.m_bwmsRaizForte = root;
        this.m_bwmsRaiz = root;
        // Fora da tela a ligação devolve `false`: o `OnInitialize` vem quando a árvore for anexada.
        let ligou: Bool = BwmsWidgetAttachController(root, this, false);
    }

    public func GetRootWidget() -> wref<inkWidget> {
        return this.m_bwmsRaiz;
    }

    public func GetRootCompoundWidget() -> wref<inkCompoundWidget> {
        return this.m_bwmsRaiz as inkCompoundWidget;
    }

    // Onde os filhos que se penduram neste controller entram. Popup, conteúdo e rodapé devolvem o painel interno.
    public func GetContainerWidget() -> wref<inkCompoundWidget> {
        return this.m_bwmsRaiz as inkCompoundWidget;
    }

    public func GetGame() -> GameInstance {
        // O `GetGameInstance()` global vem inútil quando o script é chamado pelo Rust (memória cp77-getgameinstance-nulo-pelo-tick).
        let jogo: GameInstance = BwmsSessionGame();
        return jogo;
    }

    public func SetName(name: CName) -> Void {
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        if IsDefined(raiz) {
            raiz.SetName(name);
        }
    }

    public func IsAttached() -> Bool {
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        return IsDefined(raiz) && !IsDefined(this.m_bwmsRaizForte);
    }

    public func Reparent(parent: wref<inkCompoundWidget>) -> Void {
        this.BwmsPendurar(parent, -1);
    }

    public func Reparent(parent: wref<inkCompoundWidget>, index: Int32) -> Void {
        this.BwmsPendurar(parent, index);
    }

    public func Reparent(parent: wref<inkCompoundWidget>, gameController: ref<inkGameController>) -> Void {
        this.m_bwmsControladorDoJogo = gameController;
        this.BwmsPendurar(parent, -1);
    }

    // O game controller que o mod informou ao pendurar; nulo quando pendurou sem ele.
    public func GetGameController() -> wref<inkGameController> {
        return this.m_bwmsControladorDoJogo;
    }

    public func Reparent(parent: ref<inkCustomController>) -> Void {
        if !IsDefined(parent) {
            return;
        }
        this.BwmsPendurar(parent.GetContainerWidget(), -1);
        parent.BwmsFilhoAnexado(this);
    }

    // Aviso ao pai de que `filho` entrou nele; o rodapé guarda os botões e o popup guarda o rodapé.
    public func BwmsFilhoAnexado(filho: ref<inkCustomController>) -> Void {}

    protected func BwmsPendurar(pai: wref<inkCompoundWidget>, indice: Int32) -> Void {
        this.CreateInstance();
        let raiz: ref<inkWidget> = this.m_bwmsRaiz;
        if !IsDefined(raiz) || !IsDefined(pai) {
            return;
        }
        raiz.Reparent(pai, indice);
        this.m_bwmsRaizForte = null;
        this.OnReparent(pai);
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

// Botão com a cara dos menus do jogo (atlas e estilo vanilla). O mod estende, chama `super.CreateWidgets()` e ajusta `m_root`/`m_label`.
// O nome vai na raiz, que é o único widget interativo: é ela que chega como alvo do clique (`evt.GetTarget().GetName()`).
public class SimpleButton extends inkCustomController {
    protected let m_root: wref<inkCompoundWidget>;
    protected let m_bg: wref<inkImage>;
    protected let m_frame: wref<inkImage>;
    protected let m_label: wref<inkText>;
    private let m_bwmsDesabilitado: Bool;

    public static func Create() -> ref<SimpleButton> {
        let botao: ref<SimpleButton> = new SimpleButton();
        botao.CreateInstance();
        return botao;
    }

    protected cb func OnCreate() -> Void {
        this.CreateWidgets();
    }

    protected func CreateWidgets() -> Void {
        let raiz: ref<inkCanvas> = new inkCanvas();
        raiz.SetName(n"button");
        raiz.SetSize(Vector2(360.0, 80.0));
        raiz.SetInteractive(true);

        let fundo: ref<inkImage> = SimpleButton.BwmsCamada(n"bg", n"cell_bg", n"MainColors.Fullscreen_PrimaryBackgroundDarkest", 0.8);
        fundo.Reparent(raiz);
        let moldura: ref<inkImage> = SimpleButton.BwmsCamada(n"frame", n"cell_fg", n"MainColors.Red", 1.0);
        moldura.Reparent(raiz);

        let rotulo: ref<inkText> = new inkText();
        rotulo.SetName(n"label");
        rotulo.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
        rotulo.SetFontStyle(n"Medium");
        rotulo.SetFontSize(38);
        rotulo.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
        rotulo.BindProperty(n"tintColor", n"MainColors.Red");
        rotulo.SetAnchor(inkEAnchor.Centered);
        rotulo.SetAnchorPoint(Vector2(0.5, 0.5));
        rotulo.SetHorizontalAlignment(textHorizontalAlignment.Center);
        rotulo.SetVerticalAlignment(textVerticalAlignment.Center);
        rotulo.Reparent(raiz);

        this.m_root = raiz;
        this.m_bg = fundo;
        this.m_frame = moldura;
        this.m_label = rotulo;
        this.SetRootWidget(raiz);
        raiz.RegisterToCallback(n"OnHoverOver", this, n"BwmsBotaoSobre");
        raiz.RegisterToCallback(n"OnHoverOut", this, n"BwmsBotaoFora");
    }

    private static func BwmsCamada(nome: CName, parte: CName, cor: CName, opacidade: Float) -> ref<inkImage> {
        let imagem: ref<inkImage> = new inkImage();
        imagem.SetName(nome);
        imagem.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
        imagem.SetTexturePart(parte);
        imagem.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
        imagem.BindProperty(n"tintColor", cor);
        imagem.SetAnchor(inkEAnchor.Fill);
        let noveFatias: Bool = BwmsImageNineSlice(imagem, 50.0, 30.0, 100.0, 30.0);
        imagem.SetOpacity(opacidade);
        return imagem;
    }

    public func SetText(text: String) -> Void {
        let rotulo: ref<inkText> = this.m_label;
        if IsDefined(rotulo) {
            rotulo.SetText(text);
        }
    }

    public func GetText() -> String {
        let rotulo: ref<inkText> = this.m_label;
        return IsDefined(rotulo) ? rotulo.GetText() : "";
    }

    // Desabilitado só muda a cara e o `IsEnabled`: o clique ainda chega, e o mod filtra (Virtual Atelier, `SearchEngineComponent`).
    public func SetDisabled(disabled: Bool) -> Void {
        this.m_bwmsDesabilitado = disabled;
        let raiz: ref<inkWidget> = this.m_root;
        if IsDefined(raiz) {
            raiz.SetOpacity(disabled ? 0.35 : 1.0);
        }
    }

    public func IsEnabled() -> Bool {
        return !this.m_bwmsDesabilitado;
    }

    public func IsDisabled() -> Bool {
        return this.m_bwmsDesabilitado;
    }

    protected cb func BwmsBotaoSobre(evt: ref<inkPointerEvent>) -> Bool {
        let fundo: ref<inkImage> = this.m_bg;
        if IsDefined(fundo) && !this.m_bwmsDesabilitado {
            fundo.BindProperty(n"tintColor", n"MainColors.Red");
            fundo.SetOpacity(0.25);
        }
        return false;
    }

    protected cb func BwmsBotaoFora(evt: ref<inkPointerEvent>) -> Bool {
        let fundo: ref<inkImage> = this.m_bg;
        if IsDefined(fundo) {
            fundo.BindProperty(n"tintColor", n"MainColors.Fullscreen_PrimaryBackgroundDarkest");
            fundo.SetOpacity(0.8);
        }
        return false;
    }
}

// Campo de texto com a cara do hub. A edição, o caret e as teclas são do `BwmsHubTextInput` (`bwms-text-input.reds`); aqui entram o
// placeholder, o limite de tamanho, a caixa das letras e o aviso `OnInput` que os mods registram.
public class HubTextInput extends inkCustomController {
    protected let m_root: wref<inkCompoundWidget>;
    protected let m_frame: wref<inkImage>;
    protected let m_hover: wref<inkImage>;
    protected let m_focus: wref<inkImage>;
    private let m_bwmsCampo: ref<BwmsCampoDaPonte>;

    public static func Create() -> ref<HubTextInput> {
        let campo: ref<HubTextInput> = new HubTextInput();
        campo.CreateInstance();
        return campo;
    }

    protected cb func OnCreate() -> Void {
        this.CreateWidgets();
    }

    protected func CreateWidgets() -> Void {
        let campo: ref<BwmsCampoDaPonte> = BwmsCampoDaPonte.Novo(600.0);
        campo.OnChange(this, n"BwmsAoMudar");
        let moldura: ref<inkCompoundWidget> = campo.GetRoot() as inkCompoundWidget;
        this.m_bwmsCampo = campo;
        this.m_root = moldura;
        this.m_frame = moldura.GetWidgetByPathName(n"hub_frame") as inkImage;
        this.m_hover = moldura.GetWidgetByPathName(n"hub_hover") as inkImage;
        this.m_focus = moldura.GetWidgetByPathName(n"hub_focus") as inkImage;
        this.SetRootWidget(moldura);
    }

    public func BwmsAoMudar() -> Void {
        this.CallCustomCallback(n"OnInput");
    }

    public func SetText(text: String) -> Void {
        this.m_bwmsCampo.SetText(text);
    }

    public func GetText() -> String {
        return this.m_bwmsCampo.GetText();
    }

    public func SetDefaultText(text: String) -> Void {
        this.m_bwmsCampo.DefinirDica(text);
    }

    public func SetMaxLength(maxLength: Int32) -> Void {
        this.m_bwmsCampo.DefinirMaximo(maxLength);
    }

    public func SetLetterCase(letterCase: textLetterCase) -> Void {
        this.m_bwmsCampo.DefinirCaixa(letterCase);
    }

    public func SetDisabled(disabled: Bool) -> Void {
        this.m_bwmsCampo.SetDisabled(disabled);
    }

    // Foco pedido por quem tem um game controller na mão (o `ShowInput` foca o campo ao abrir).
    public func BwmsFocus(controller: ref<inkGameController>) -> Void {
        this.m_bwmsCampo.Focus(controller);
    }
}

// O `BwmsHubTextInput` com o que o `HubTextInput` do Windows tem a mais. Os ganchos `DecorateFrame`/`OnStateChanged`/`HandleKey` são do
// próprio `BwmsTextInput`.
class BwmsCampoDaPonte extends BwmsHubTextInput {
    private let m_bwmsDica: wref<inkText>;
    private let m_bwmsMaximo: Int32;

    public static func Novo(largura: Float) -> ref<BwmsCampoDaPonte> {
        let campo: ref<BwmsCampoDaPonte> = new BwmsCampoDaPonte();
        campo.m_width = largura;
        campo.m_fontSize = 38;
        campo.m_height = 80.0;
        campo.m_padding = 18.0;
        campo.SetAnimated(true);
        campo.Construct();
        return campo;
    }

    protected func DecorateFrame(frame: ref<inkCanvas>) -> Void {
        super.DecorateFrame(frame);
        let dica: ref<inkText> = new inkText();
        dica.SetName(n"bwms_dica");
        dica.SetFontFamily(BwmsTextMeasurer.DefaultFont());
        dica.SetFontSize(this.m_fontSize);
        dica.SetStyle(BwmsHubTextInput.Style());
        dica.BindProperty(n"tintColor", n"MainColors.Red");
        dica.SetOpacity(0.45);
        dica.SetAnchor(inkEAnchor.CenterLeft);
        dica.SetAnchorPoint(Vector2(0.0, 0.5));
        dica.SetMargin(this.m_padding, 0.0, this.m_padding, 0.0);
        dica.Reparent(frame);
        this.m_bwmsDica = dica;
        frame.RegisterToCallback(n"OnRelease", this, n"BwmsAoClicar");
    }

    protected func OnStateChanged() -> Void {
        super.OnStateChanged();
        this.BwmsAtualizarDica();
    }

    private func BwmsAtualizarDica() -> Void {
        let dica: ref<inkText> = this.m_bwmsDica;
        if IsDefined(dica) {
            dica.SetVisible(StrLen(this.GetText()) == 0 && !this.m_focused);
        }
    }

    // O motor dá o foco ao widget focável que recebe o clique; aqui só o caret e a dica acompanham.
    protected cb func BwmsAoClicar(evt: ref<inkPointerEvent>) -> Bool {
        if evt.IsAction(n"click") && !this.m_focused {
            this.m_focused = true;
            this.OnStateChanged();
        }
        return false;
    }

    public func HandleKey(key: Int32, character: String, isCharacter: Bool, shift: Bool, ctrl: Bool) -> Bool {
        if !this.m_focused {
            this.m_focused = true;
            this.OnStateChanged();
        }
        if isCharacter && !ctrl && this.m_bwmsMaximo > 0 && StrLen(this.GetText()) >= this.m_bwmsMaximo {
            return true;
        }
        let tratou: Bool = super.HandleKey(key, character, isCharacter, shift, ctrl);
        let texto: String = this.GetText();
        if this.m_bwmsMaximo > 0 && StrLen(texto) > this.m_bwmsMaximo {
            this.SetText(StrLeft(texto, this.m_bwmsMaximo));
        }
        this.BwmsAtualizarDica();
        return tratou;
    }

    public func DefinirDica(texto: String) -> Void {
        let dica: ref<inkText> = this.m_bwmsDica;
        if IsDefined(dica) {
            dica.SetText(texto);
        }
        this.BwmsAtualizarDica();
    }

    public func DefinirMaximo(maximo: Int32) -> Void {
        this.m_bwmsMaximo = maximo;
    }

    public func DefinirCaixa(caixa: textLetterCase) -> Void {
        let texto: ref<inkText> = this.m_frame.GetWidgetByPathName(n"content/text") as inkText;
        if IsDefined(texto) {
            texto.SetLetterCase(caixa);
        }
        let dica: ref<inkText> = this.m_bwmsDica;
        if IsDefined(dica) {
            dica.SetLetterCase(caixa);
        }
    }
}

// Popup dentro de um menu: escurece a tela do menu que pediu e mostra uma janela com conteúdo e rodapé. A ação de entrada de um
// `PopupButton` do rodapé (tecla, controle ou clique) fecha o popup: `cancel`/`back` chamam `OnCancel`, as outras chamam `OnConfirm`.
public class InMenuPopup extends inkCustomController {
    private let m_bwmsSolicitante: wref<inkGameController>;
    private let m_bwmsCorpo: wref<inkVerticalPanel>;
    private let m_bwmsRodape: wref<InMenuPopupFooter>;
    private let m_bwmsAberto: Bool;
    // Fechado, a raiz sai da tela do solicitante e só esta referência a mantém viva para o `Open` seguinte pendurar de novo.
    private let m_bwmsRaizFechada: ref<inkWidget>;

    protected cb func OnCreate() -> Void {
        let raiz: ref<inkCanvas> = new inkCanvas();
        raiz.SetName(n"bwms_in_menu_popup");
        raiz.SetAnchor(inkEAnchor.Fill);
        raiz.SetInteractive(true);

        let sombra: ref<inkRectangle> = new inkRectangle();
        sombra.SetName(n"shade");
        sombra.SetAnchor(inkEAnchor.Fill);
        sombra.SetTintColor(new HDRColor(0.0, 0.0, 0.0, 1.0));
        sombra.SetOpacity(0.72);
        sombra.Reparent(raiz);

        let janela: ref<inkCanvas> = new inkCanvas();
        janela.SetName(n"window");
        janela.SetAnchor(inkEAnchor.Centered);
        janela.SetAnchorPoint(Vector2(0.5, 0.5));
        janela.SetFitToContent(true);
        janela.Reparent(raiz);

        let fundo: ref<inkImage> = InMenuPopup.BwmsCamada(n"bg", n"cell_bg", n"MainColors.Fullscreen_PrimaryBackgroundDarkest", 0.97);
        fundo.Reparent(janela);
        let moldura: ref<inkImage> = InMenuPopup.BwmsCamada(n"frame", n"cell_fg", n"MainColors.Red", 0.6);
        moldura.Reparent(janela);

        let corpo: ref<inkVerticalPanel> = new inkVerticalPanel();
        corpo.SetName(n"body");
        corpo.SetFitToContent(true);
        corpo.SetMargin(48.0, 36.0, 48.0, 36.0);
        corpo.Reparent(janela);

        this.m_bwmsCorpo = corpo;
        this.SetRootWidget(raiz);
        // Pendura já: o `OnCreate` do mod segue depois do `super.OnCreate()` e usa `SpawnFromExternal`, que precisa do controller na tela.
        let solicitante: ref<inkGameController> = this.m_bwmsSolicitante;
        if IsDefined(solicitante) {
            this.BwmsPendurar(solicitante.GetRootCompoundWidget(), -1);
        }
    }

    private static func BwmsCamada(nome: CName, parte: CName, cor: CName, opacidade: Float) -> ref<inkImage> {
        let imagem: ref<inkImage> = new inkImage();
        imagem.SetName(nome);
        imagem.SetAtlasResource(r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas");
        imagem.SetTexturePart(parte);
        imagem.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
        imagem.BindProperty(n"tintColor", cor);
        imagem.SetAnchor(inkEAnchor.Fill);
        let noveFatias: Bool = BwmsImageNineSlice(imagem, 50.0, 30.0, 100.0, 30.0);
        imagem.SetOpacity(opacidade);
        return imagem;
    }

    public func GetContainerWidget() -> wref<inkCompoundWidget> {
        return this.m_bwmsCorpo;
    }

    public func BwmsFilhoAnexado(filho: ref<inkCustomController>) -> Void {
        let rodape: ref<InMenuPopupFooter> = filho as InMenuPopupFooter;
        if IsDefined(rodape) {
            this.m_bwmsRodape = rodape;
            rodape.BwmsDefinirDono(this);
        }
    }

    public func Open(requester: ref<inkGameController>) -> Void {
        if this.m_bwmsAberto || !IsDefined(requester) {
            return;
        }
        this.m_bwmsAberto = true;
        this.m_bwmsSolicitante = requester;
        // Na 1ª vez o `OnCreate` monta e pendura na tela do solicitante; reaberto, ele não roda de novo e a raiz guardada volta pra tela.
        let reaberto: Bool = IsDefined(this.m_bwmsRaizFechada);
        this.CreateInstance();
        if reaberto {
            this.Reparent(requester.GetRootCompoundWidget(), -1);
            this.m_bwmsRaizFechada = null;
        }
        this.RegisterToGlobalInputCallback(n"OnPostOnRelease", this, n"BwmsPopupEntrada");
        this.OnShown();
    }

    public func Close() -> Void {
        if !this.m_bwmsAberto {
            return;
        }
        this.m_bwmsAberto = false;
        this.UnregisterFromGlobalInputCallback(n"OnPostOnRelease", this, n"BwmsPopupEntrada");
        let raiz: ref<inkWidget> = this.GetRootWidget();
        let solicitante: ref<inkGameController> = this.m_bwmsSolicitante;
        if IsDefined(raiz) && IsDefined(solicitante) {
            let pai: ref<inkCompoundWidget> = solicitante.GetRootCompoundWidget();
            if IsDefined(pai) {
                this.m_bwmsRaizFechada = raiz;
                pai.RemoveChild(raiz);
            }
        }
    }

    public func IsOpen() -> Bool {
        return this.m_bwmsAberto;
    }

    protected cb func OnConfirm() -> Void {}

    protected func OnCancel() -> Void {}

    protected func OnShown() -> Void {}

    protected cb func BwmsPopupEntrada(evt: ref<inkPointerEvent>) -> Bool {
        if !this.m_bwmsAberto {
            return false;
        }
        let rodape: ref<InMenuPopupFooter> = this.m_bwmsRodape;
        if IsDefined(rodape) {
            for acao in rodape.BwmsAcoes() {
                if evt.IsAction(acao) {
                    this.BwmsAcionar(acao);
                    return true;
                }
            }
        }
        if evt.IsAction(n"cancel") || evt.IsAction(n"back") {
            this.BwmsAcionar(n"cancel");
            return true;
        }
        return false;
    }

    public func BwmsAcionar(acao: CName) -> Void {
        if !this.m_bwmsAberto {
            return;
        }
        if Equals(acao, n"cancel") || Equals(acao, n"back") {
            this.OnCancel();
        } else {
            this.OnConfirm();
        }
        this.Close();
    }
}

public class InMenuPopupContent extends inkCustomController {
    private let m_bwmsTitulo: wref<inkText>;
    private let m_bwmsConteudo: wref<inkVerticalPanel>;

    public static func Create() -> ref<InMenuPopupContent> {
        let conteudo: ref<InMenuPopupContent> = new InMenuPopupContent();
        conteudo.CreateInstance();
        return conteudo;
    }

    protected cb func OnCreate() -> Void {
        let raiz: ref<inkVerticalPanel> = new inkVerticalPanel();
        raiz.SetName(n"content");
        raiz.SetFitToContent(true);

        let titulo: ref<inkText> = new inkText();
        titulo.SetName(n"title");
        titulo.SetFontFamily("base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily");
        titulo.SetFontStyle(n"Medium");
        titulo.SetFontSize(52);
        titulo.SetStyle(r"base\\gameplay\\gui\\common\\main_colors.inkstyle");
        titulo.BindProperty(n"tintColor", n"MainColors.Red");
        titulo.SetMargin(0.0, 0.0, 0.0, 20.0);
        titulo.Reparent(raiz);
        titulo.SetLetterCase(textLetterCase.UpperCase);

        let conteudo: ref<inkVerticalPanel> = new inkVerticalPanel();
        conteudo.SetName(n"container");
        conteudo.SetFitToContent(true);
        conteudo.Reparent(raiz);

        this.m_bwmsTitulo = titulo;
        this.m_bwmsConteudo = conteudo;
        this.SetRootWidget(raiz);
    }

    public func SetTitle(title: String) -> Void {
        let titulo: ref<inkText> = this.m_bwmsTitulo;
        if IsDefined(titulo) {
            titulo.SetText(title);
        }
    }

    public func GetContainerWidget() -> wref<inkCompoundWidget> {
        return this.m_bwmsConteudo;
    }
}

public class InMenuPopupFooter extends inkCustomController {
    private let m_bwmsBotoes: array<wref<PopupButton>>;
    private let m_bwmsDono: wref<InMenuPopup>;

    public static func Create() -> ref<InMenuPopupFooter> {
        let rodape: ref<InMenuPopupFooter> = new InMenuPopupFooter();
        rodape.CreateInstance();
        return rodape;
    }

    protected cb func OnCreate() -> Void {
        let raiz: ref<inkHorizontalPanel> = new inkHorizontalPanel();
        raiz.SetName(n"footer");
        raiz.SetHAlign(inkEHorizontalAlign.Right);
        raiz.SetMargin(0.0, 32.0, 0.0, 0.0);
        raiz.SetChildMargin(new inkMargin(20.0, 0.0, 0.0, 0.0));
        this.SetRootWidget(raiz);
    }

    public func BwmsFilhoAnexado(filho: ref<inkCustomController>) -> Void {
        let botao: ref<PopupButton> = filho as PopupButton;
        if IsDefined(botao) {
            ArrayPush(this.m_bwmsBotoes, botao);
            botao.BwmsDefinirRodape(this);
        }
    }

    public func BwmsDefinirDono(dono: ref<InMenuPopup>) -> Void {
        this.m_bwmsDono = dono;
    }

    public func BwmsAcoes() -> array<CName> {
        let acoes: array<CName>;
        for botao in this.m_bwmsBotoes {
            let vivo: ref<PopupButton> = botao;
            if IsDefined(vivo) && NotEquals(vivo.GetInputAction(), n"") {
                ArrayPush(acoes, vivo.GetInputAction());
            }
        }
        return acoes;
    }

    public func BwmsAcionar(acao: CName) -> Void {
        let dono: ref<InMenuPopup> = this.m_bwmsDono;
        if IsDefined(dono) {
            dono.BwmsAcionar(acao);
        }
    }
}

// Botão do rodapé: o texto e a ação de entrada que ele representa. O clique dispara a mesma ação que a tecla.
public class PopupButton extends SimpleButton {
    private let m_bwmsAcao: CName;
    private let m_bwmsRodape: wref<InMenuPopupFooter>;

    public static func Create() -> ref<PopupButton> {
        let botao: ref<PopupButton> = new PopupButton();
        botao.CreateInstance();
        return botao;
    }

    protected func CreateWidgets() -> Void {
        super.CreateWidgets();
        let raiz: ref<inkWidget> = this.m_root;
        raiz.SetSize(Vector2(300.0, 72.0));
        raiz.RegisterToCallback(n"OnRelease", this, n"BwmsBotaoSolto");
    }

    public func SetInputAction(action: CName) -> Void {
        this.m_bwmsAcao = action;
    }

    public func GetInputAction() -> CName {
        return this.m_bwmsAcao;
    }

    public func BwmsDefinirRodape(rodape: ref<InMenuPopupFooter>) -> Void {
        this.m_bwmsRodape = rodape;
    }

    protected cb func BwmsBotaoSolto(evt: ref<inkPointerEvent>) -> Bool {
        let rodape: ref<InMenuPopupFooter> = this.m_bwmsRodape;
        if evt.IsAction(n"click") && IsDefined(rodape) && this.IsEnabled() {
            rodape.BwmsAcionar(this.m_bwmsAcao);
            return true;
        }
        return false;
    }
}
