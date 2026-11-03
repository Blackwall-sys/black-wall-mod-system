// bwms-ui-button.reds — botão de mod, construído sobre `BwmsUIController`.
//
// O QUE ENTREGA: um botão que um mod monta em duas linhas, com estado visual de hover/press e um
// callback de clique — sem o mod ter que saber como a engine roteia evento de widget.
//
// A ARMADILHA QUE ISTO EVITA (bug real): callback registrado no game
// controller em vez de no WIDGET nunca dispara, e não há erro no log. Aqui os quatro eventos
// (`OnEnter`/`OnLeave`/`OnPress`/`OnRelease` — nomes confirmados no código do jogo) são ligados
// pelo `BwmsUIController.OnSelf`, que garante o alvo certo e ainda enfileira se a raiz ainda não
// existir.
//
// KEEP-ALIVE: todo filho (fundo e rótulo) fica em campo `ref<>` forte. Guardar só `wref<>` faz o
// widget sumir sem aviso — a lição mais cara desta camada.
//
// DEPENDÊNCIAS confirmadas por grep ANTES de escrever (`cp77-symbols/redscript-all.reds`):
//   - `inkCanvas extends inkCompoundWidget` (`:204025`), `inkRectangle` (`:264447`),
//     `inkText extends inkLeafWidget` (`:200611`)
//   - `inkWidget.SetSize`/`SetVisible`/`SetInteractive`/`SetOpacity`/`SetMargin` (`:54690`+)
//   - eventos `OnEnter`/`OnLeave`/`OnPress`/`OnRelease` (usados 33/33/248/353× pelo jogo)

// REGRA DESTA CAMADA (2026-08-22): **nenhum tamanho em pixel absoluto.** Quem instala pode estar
// em 1366x768, 1080p, 1440p, 4K ou ultrawide, em janela ou tela cheia. Um botão de 320x56 fixos
// fica razoável em 1080p, minúsculo em 4K e enorme em 720p. Todo tamanho aqui é uma medida BASE
// (pensada em 1080p) multiplicada pelo fator da tela real — `ScreenHelper.GetSmartScale`.
public class BwmsButton extends BwmsUIController {

    private let m_scale: Float;
    private let m_bg: ref<inkRectangle>;
    private let m_label: ref<inkText>;
    private let m_text: String;
    private let m_hovered: Bool;
    private let m_pressed: Bool;
    private let m_enabled: Bool;

    private let m_clickTarget: wref<IScriptable>;
    private let m_clickFn: CName;

    // --- Construção --------------------------------------------------------------------------

    // Forma recomendada: com o `game`, o botão se dimensiona para a tela de quem está jogando.
    public static func Create(text: String, game: GameInstance) -> ref<BwmsButton> {
        let btn: ref<BwmsButton> = new BwmsButton();
        btn.m_text = text;
        btn.m_enabled = true;
        btn.m_scale = ScreenHelper.GetSmartScale(game);
        btn.Construct();
        return btn;
    }

    // Sem `game` não há como saber a tela — assume 1080p e diz isso aqui, em vez de fingir que
    // o tamanho está certo. Use a forma acima sempre que tiver a instância do jogo.
    public static func CreateUnscaled(text: String) -> ref<BwmsButton> {
        let btn: ref<BwmsButton> = new BwmsButton();
        btn.m_text = text;
        btn.m_enabled = true;
        btn.m_scale = 1.0;
        btn.Construct();
        return btn;
    }

    // Medidas BASE, em unidades de 1080p. Quem quiser outro tamanho chama `SetBaseSize`.
    public static func BaseWidth() -> Float = 320.0
    public static func BaseHeight() -> Float = 56.0
    public static func BasePadX() -> Float = 16.0
    public static func BasePadY() -> Float = 14.0

    private func Scale() -> Float {
        // Um controller construído direto (sem `Create`) tem escala 0 — trata como 1.0 em vez de
        // colapsar o widget para tamanho zero, que some da tela sem erro nenhum.
        if this.m_scale <= 0.0 {
            return 1.0;
        }
        return this.m_scale;
    }

    // Redimensiona em unidades de 1080p; a escala da tela é aplicada por cima.
    public func SetBaseSize(width: Float, height: Float) -> Void {
        let k: Float = this.Scale();
        if IsDefined(this.m_bg) {
            this.m_bg.SetSize(width * k, height * k);
        }
        let root: ref<inkWidget> = this.GetRoot();
        if IsDefined(root) {
            root.SetSize(width * k, height * k);
        }
    }

    protected func OnCreate() -> ref<inkWidget> {
        let k: Float = this.Scale();
        let w: Float = BwmsButton.BaseWidth() * k;
        let h: Float = BwmsButton.BaseHeight() * k;
        let padX: Float = BwmsButton.BasePadX() * k;
        let padY: Float = BwmsButton.BasePadY() * k;

        let root: ref<inkCanvas> = new inkCanvas();
        root.SetName(n"BwmsButton");
        root.SetSize(w, h);
        root.SetInteractive(true);

        let bg: ref<inkRectangle> = new inkRectangle();
        bg.SetName(n"bg");
        bg.SetSize(w, h);
        bg.SetOpacity(0.25);
        bg.Reparent(root, -1);
        this.m_bg = bg;

        let label: ref<inkText> = new inkText();
        label.SetName(n"label");
        label.SetText(this.m_text);
        label.SetFitToContent(true);
        label.SetMargin(padX, padY, padX, padY);
        label.Reparent(root, -1);
        this.m_label = label;

        return root;
    }

    protected cb func OnInitialize() -> Bool {
        // No WIDGET, via controller — ver a armadilha descrita no topo.
        this.OnSelf(n"OnEnter", n"OnBtnEnter");
        this.OnSelf(n"OnLeave", n"OnBtnLeave");
        this.OnSelf(n"OnPress", n"OnBtnPress");
        this.OnSelf(n"OnRelease", n"OnBtnRelease");
        this.Repaint();
        return true;
    }

    // --- API do mod --------------------------------------------------------------------------

    public func SetText(text: String) -> Void {
        this.m_text = text;
        if IsDefined(this.m_label) {
            this.m_label.SetText(text);
        }
    }

    public func GetText() -> String {
        return this.m_text;
    }

    // Quem é avisado no clique. Um alvo por botão — mais de um vira lista, e lista de callback
    // sem dono é fonte de vazamento.
    public func OnClick(target: ref<IScriptable>, functionName: CName) -> Void {
        this.m_clickTarget = target;
        this.m_clickFn = functionName;
    }

    public func SetEnabled(enabled: Bool) -> Void {
        this.m_enabled = enabled;
        let root: ref<inkWidget> = this.GetRoot();
        if IsDefined(root) {
            root.SetInteractive(enabled);
        }
        this.Repaint();
    }

    public func IsEnabled() -> Bool {
        return this.m_enabled;
    }

    public func IsHovered() -> Bool {
        return this.m_hovered;
    }

    // --- Eventos -----------------------------------------------------------------------------

    protected cb func OnBtnEnter(evt: ref<inkPointerEvent>) -> Bool {
        this.m_hovered = true;
        this.Repaint();
        return true;
    }

    protected cb func OnBtnLeave(evt: ref<inkPointerEvent>) -> Bool {
        this.m_hovered = false;
        this.m_pressed = false;
        this.Repaint();
        return true;
    }

    protected cb func OnBtnPress(evt: ref<inkPointerEvent>) -> Bool {
        if !this.m_enabled {
            return false;
        }
        this.m_pressed = true;
        this.Repaint();
        return true;
    }

    // O clique conta no RELEASE, não no press: é o que o usuário espera (dá pra desistir
    // arrastando o cursor pra fora antes de soltar).
    protected cb func OnBtnRelease(evt: ref<inkPointerEvent>) -> Bool {
        let wasPressed: Bool = this.m_pressed;
        this.m_pressed = false;
        this.Repaint();
        if this.m_enabled && wasPressed && this.m_hovered {
            this.Fire();
        }
        return true;
    }

    private func Fire() -> Void {
        let target: ref<IScriptable> = this.m_clickTarget;
        if IsDefined(target) && NotEquals(this.m_clickFn, n"") {
            let root: ref<inkWidget> = this.GetRoot();
            if IsDefined(root) {
                // Reusa o despacho da própria engine: emite o evento no widget, que entrega ao
                // alvo registrado. Evita inventar mecanismo de chamada dinâmica.
                root.CallCustomCallback(this.m_clickFn);
            }
        }
    }

    // Estado visual num lugar só — hover/press/disabled não se contradizem.
    private func Repaint() -> Void {
        if !IsDefined(this.m_bg) {
            return;
        }
        let opacity: Float = 0.25;
        if !this.m_enabled {
            opacity = 0.10;
        } else {
            if this.m_pressed {
                opacity = 0.65;
            } else {
                if this.m_hovered {
                    opacity = 0.45;
                }
            }
        }
        this.m_bg.SetOpacity(opacity);
    }
}
