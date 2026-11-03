// bwms-text-input.reds — campo de texto do BWMS, 2026-09-13.
//
// PARA QUE SERVE: digitar texto numa UI de mod — nome de preset, busca numa lista, valor de uma opção. Com foco, o campo recebe a tecla pelo
// `OnBwmsInputKey` (`#176`), edita pelo modelo do BWMS (`BwmsTextEdit`: buffer, cursor e seleção em codepoint, no Rust) e desenha texto, seleção e caret
// com as larguras do `BwmsTextMeasurer` (115c).
//
// TECLAS (só no aperto): caractere insere; Backspace e Delete apagam, a seleção primeiro; setas andam, por palavra com Ctrl e estendendo a seleção com
// Shift; Home e End vão às pontas; Ctrl+A seleciona tudo; Ctrl+C copia, Ctrl+X recorta e Ctrl+V cola pelo clipboard do jogo (`#100`); Enter avisa quem
// ouve `OnSubmit`. `HandleKey` é a mesma decisão pra quem já tem a tecla na mão. Campo desabilitado ignora tecla.
//
// FOCO: o motor só entrega tecla ao widget com foco, e só grava o foco num widget que aceita foco. `Focus(controller)` liga a marca e pede o foco a um
// game controller qualquer; `Blur(controller)` devolve.
//
// VARIANTES: `DecorateFrame` recebe a moldura antes do primeiro layout (camadas penduradas no índice 0 ficam atrás do texto) e `OnStateChanged` roda a
// cada mudança de foco, hover ou desabilitado. `m_padding` afasta o texto da borda. `BwmsHubTextInput` é a variante com a cara dos menus do jogo.
//
// DONO: como toda `BwmsUIController`, o widget guarda o controller só por referência fraca — quem cria o campo segura um `ref<BwmsTextInput>`.
//
// USO: `let campo = BwmsTextInput.Create(420.0, 38); campo.Mount(raiz); campo.OnChange(this, n"OnTexto"); campo.Focus(controller);`
//
// Sem `Viewport` próprio: o conteúdo desloca por `SetTranslation` com o deslocamento que o modelo calcula.

public class BwmsTextInput extends BwmsUIController {
    private let m_editor: ref<BwmsTextEdit>;
    private let m_measurer: ref<BwmsTextMeasurer>;
    private let m_content: ref<inkCanvas>;
    private let m_text: ref<inkText>;
    private let m_selection: ref<inkRectangle>;
    private let m_caret: ref<inkRectangle>;
    private let m_scroll: Float;
    private let m_caretX: Float;
    private let m_changeTarget: wref<IScriptable>;
    private let m_changeFn: CName;
    private let m_submitTarget: wref<IScriptable>;
    private let m_submitFn: CName;

    protected let m_frame: ref<inkCanvas>;
    protected let m_width: Float;
    protected let m_height: Float;
    protected let m_padding: Float;
    protected let m_fontSize: Int32;
    protected let m_focused: Bool;
    protected let m_hovered: Bool;
    protected let m_disabled: Bool;

    public static func CaretWidth() -> Float = 2.0

    public static func Create(width: Float, fontSize: Int32) -> ref<BwmsTextInput> {
        let input: ref<BwmsTextInput> = new BwmsTextInput();
        input.m_width = width;
        input.m_fontSize = fontSize;
        input.Construct();
        return input;
    }

    protected func OnCreate() -> ref<inkWidget> {
        if this.m_width <= 0.0 {
            this.m_width = 400.0;
        }
        if this.m_fontSize <= 0 {
            this.m_fontSize = 38;
        }
        if this.m_height <= 0.0 {
            this.m_height = Cast<Float>(this.m_fontSize) * 1.4;
        }
        this.m_editor = BwmsTextEdit.Abrir();

        let frame: ref<inkCanvas> = new inkCanvas();
        frame.SetName(n"bwms_text_input");
        frame.SetSize(new Vector2(this.m_width, this.m_height));
        frame.SetInteractive(true);
        frame.SetSupportFocus(true);

        let content: ref<inkCanvas> = new inkCanvas();
        content.SetName(n"content");
        content.SetSize(new Vector2(this.m_width, this.m_height));
        content.Reparent(frame);

        let selection: ref<inkRectangle> = new inkRectangle();
        selection.SetName(n"selection");
        selection.SetSize(new Vector2(0.0, this.m_height));
        selection.SetOpacity(0.35);
        selection.SetVisible(false);
        selection.Reparent(content);

        let text: ref<inkText> = new inkText();
        text.SetName(n"text");
        text.SetFontFamily(BwmsTextMeasurer.DefaultFont());
        text.SetFontSize(this.m_fontSize);
        text.SetHorizontalAlignment(textHorizontalAlignment.Left);
        text.SetVerticalAlignment(textVerticalAlignment.Center);
        text.SetAnchor(inkEAnchor.CenterLeft);
        text.SetAnchorPoint(new Vector2(0.0, 0.5));
        text.SetText("");
        text.Reparent(content);

        let caret: ref<inkRectangle> = new inkRectangle();
        caret.SetName(n"caret");
        caret.SetSize(new Vector2(BwmsTextInput.CaretWidth(), this.m_height));
        caret.SetVisible(false);
        caret.Reparent(content);

        this.m_frame = frame;
        this.m_content = content;
        this.m_selection = selection;
        this.m_text = text;
        this.m_caret = caret;
        this.DecorateFrame(frame);
        return frame;
    }

    protected cb func OnInitialize() -> Bool {
        this.OnSelf(n"OnBwmsInputKey", n"OnKey");
        this.OnSelf(n"OnHoverOver", n"OnFrameHoverOver");
        this.OnSelf(n"OnHoverOut", n"OnFrameHoverOut");
        let measurer: ref<BwmsTextMeasurer> = BwmsTextMeasurer.Create();
        measurer.SetFont(BwmsTextMeasurer.DefaultFont(), this.m_fontSize);
        measurer.Mount(this.m_frame);
        measurer.OnReady(this, n"OnMeasured");
        this.m_measurer = measurer;
        this.OnStateChanged();
        return true;
    }

    protected cb func OnUninitialize() -> Bool {
        if IsDefined(this.m_editor) {
            this.m_editor.Fechar();
            this.m_editor = null;
        }
        return true;
    }

    // O editor do campo. `OnUninitialize` solta o id e o `MountAt` não roda `OnCreate` de novo: um campo remontado abre um editor
    // novo no primeiro uso, em vez de editar um id que o Rust já soltou (no-op).
    private func Editor() -> ref<BwmsTextEdit> {
        if !IsDefined(this.m_editor) {
            this.m_editor = BwmsTextEdit.Abrir();
        }
        return this.m_editor;
    }

    // Camadas extras na moldura, antes do primeiro layout. A base não tem nenhuma.
    protected func DecorateFrame(frame: ref<inkCanvas>) -> Void {}

    // Foco, hover ou desabilitado mudou. A base só mostra ou esconde o caret.
    protected func OnStateChanged() -> Void {
        this.m_caret.SetVisible(this.m_focused && !this.m_disabled);
    }

    // --- Foco e estado ------------------------------------------------------------------------

    public func Focus(controller: ref<inkGameController>) -> Void {
        this.m_frame.SetSupportFocus(true);
        controller.RequestSetFocus(this.m_frame);
        this.m_focused = true;
        this.OnStateChanged();
        this.Layout();
    }

    public func Blur(controller: ref<inkGameController>) -> Void {
        controller.RequestSetFocus(null);
        this.m_focused = false;
        this.OnStateChanged();
    }

    public func SetDisabled(disabled: Bool) -> Void {
        this.m_disabled = disabled;
        this.OnStateChanged();
    }

    public func IsFocused() -> Bool {
        return this.m_focused;
    }

    public func IsHovered() -> Bool {
        return this.m_hovered;
    }

    public func IsDisabled() -> Bool {
        return this.m_disabled;
    }

    protected cb func OnFrameHoverOver(evt: ref<inkPointerEvent>) -> Bool {
        this.m_hovered = true;
        this.OnStateChanged();
        return false;
    }

    protected cb func OnFrameHoverOut(evt: ref<inkPointerEvent>) -> Bool {
        this.m_hovered = false;
        this.OnStateChanged();
        return false;
    }

    // --- Texto --------------------------------------------------------------------------------

    public func SetText(text: String) -> Void {
        this.Editor().DefinirTexto(text);
        this.Refresh();
    }

    public func GetText() -> String {
        return this.Editor().Texto();
    }

    public func GetSelectedText() -> String {
        return this.Editor().TextoSelecionado();
    }

    public func GetCaretPosition() -> Int32 {
        return this.Editor().Cursor();
    }

    public func GetSelectionStart() -> Int32 {
        return this.Editor().SelecaoInicio();
    }

    public func GetSelectionEnd() -> Int32 {
        return this.Editor().SelecaoFim();
    }

    // Posição do caret no conteúdo, em unidades de layout, e o deslocamento do conteúdo (negativo com o caret além da borda direita).
    public func GetCaretOffset() -> Float {
        return this.m_caretX;
    }

    public func GetScroll() -> Float {
        return this.m_scroll;
    }

    public func OnChange(target: ref<IScriptable>, functionName: CName) -> Void {
        this.m_changeTarget = target;
        this.m_changeFn = functionName;
    }

    public func OnSubmit(target: ref<IScriptable>, functionName: CName) -> Void {
        this.m_submitTarget = target;
        this.m_submitFn = functionName;
    }

    // --- Teclas -------------------------------------------------------------------------------

    protected cb func OnKey(evt: ref<BwmsInkKeyEvent>) -> Bool {
        let action: Int32 = EnumInt(evt.GetAction());
        if action != 1 {
            return false;
        }
        let key: Int32 = EnumInt(evt.GetKey());
        let character: String = evt.GetCharacter();
        let isCharacter: Bool = evt.IsCharacter();
        let shift: Bool = evt.IsShiftDown();
        let ctrl: Bool = evt.IsControlDown();
        let handled: Bool = this.HandleKey(key, character, isCharacter, shift, ctrl);
        return handled;
    }

    public func HandleKey(key: Int32, character: String, isCharacter: Bool, shift: Bool, ctrl: Bool) -> Bool {
        if this.m_disabled {
            return false;
        }
        let handled: Bool = true;
        let changed: Int32 = 0;
        if ctrl && key == 65 {
            changed = this.Editor().SelecionarTudo();
        } else if ctrl && key == 67 {
            let selected: String = this.Editor().TextoSelecionado();
            if StrLen(selected) > 0 {
                BwmsSetClipboardText(selected);
            }
        } else if ctrl && key == 88 {
            let cut: String = this.Editor().Recortar();
            if StrLen(cut) > 0 {
                BwmsSetClipboardText(cut);
                // Recorte não vazio tirou a seleção do texto: texto e caret mudaram.
                changed = BwmsTextEditMudou.Texto() | BwmsTextEditMudou.Caret();
            }
        } else if ctrl && key == 86 {
            let pasted: String = BwmsInkSystemGetClipboardTextFull();
            if StrLen(pasted) > 0 {
                changed = this.Editor().Inserir(pasted);
            }
        } else if key == 8 {
            changed = this.Editor().Backspace();
        } else if key == 46 {
            changed = this.Editor().Delete();
        } else if key == 37 {
            changed = this.Editor().Mover(ctrl ? BwmsTextEditMov.PalavraEsquerda() : BwmsTextEditMov.Esquerda(), shift);
        } else if key == 39 {
            changed = this.Editor().Mover(ctrl ? BwmsTextEditMov.PalavraDireita() : BwmsTextEditMov.Direita(), shift);
        } else if key == 36 {
            changed = this.Editor().Mover(BwmsTextEditMov.Inicio(), shift);
        } else if key == 35 {
            changed = this.Editor().Mover(BwmsTextEditMov.Fim(), shift);
        } else if key == 13 {
            let target: ref<IScriptable> = this.m_submitTarget;
            if IsDefined(target) {
                BwmsCallMethod(target, this.m_submitFn);
            }
        } else if isCharacter && !ctrl {
            changed = this.Editor().Inserir(character);
        } else {
            handled = false;
        }
        // Texto mudou: widget, medidor e quem ouve `OnChange`. Só caret ou seleção, com o medidor ocioso: layout na hora, com as larguras que ele
        // já tem. Com o medidor ocupado, o `OnMeasured` do fim da fila faz o layout: aqui o `Offset` de um caractere pendente depois do caret dá
        // -1 e zeraria o deslocamento. Nada: nada (Home no início, Ctrl+C, Enter), sem refazer o desenho nem avisar quem ouve.
        let textChanged: Int32 = BwmsTextEditMudou.Texto();
        if (changed & textChanged) != 0 {
            this.Refresh();
        } else if changed != 0 {
            let measuring: Bool = this.m_measurer.IsBusy();
            if !measuring {
                this.Layout();
            }
        }
        return handled;
    }

    // --- Desenho ------------------------------------------------------------------------------

    // Texto novo no widget e na fila do medidor; o layout sai quando as larguras chegam (na hora, se todas já são conhecidas). Avisa quem ouve
    // `OnChange`.
    private func Refresh() -> Void {
        let text: String = this.Editor().Texto();
        this.m_text.SetText(text);
        this.m_measurer.Queue(text);
        let target: ref<IScriptable> = this.m_changeTarget;
        if IsDefined(target) {
            BwmsCallMethod(target, this.m_changeFn);
        }
    }

    protected cb func OnMeasured() -> Bool {
        this.Layout();
        return true;
    }

    // Caret, seleção e deslocamento pelas larguras medidas. Com algum caractere ainda sem largura, espera o próximo aviso do medidor.
    private func Layout() -> Void {
        let text: String = this.Editor().Texto();
        let cursor: Int32 = this.Editor().Cursor();
        let caretX: Float = this.m_measurer.Offset(text, cursor);
        if caretX < 0.0 {
            return;
        }
        let length: Int32 = this.Editor().Comprimento();
        let contentWidth: Float = this.m_measurer.Offset(text, length);
        let window: Float = this.m_width - 2.0 * this.m_padding;
        let scroll: Float = BwmsTextEditScroll(contentWidth, window, caretX, BwmsTextInput.CaretWidth(), this.m_scroll);
        this.m_scroll = scroll;
        this.m_caretX = caretX;
        this.m_content.SetTranslation(this.m_padding + scroll, 0.0);
        this.m_caret.SetTranslation(caretX, 0.0);
        let hasSelection: Bool = this.Editor().TemSelecao();
        if hasSelection {
            let start: Float = this.m_measurer.Offset(text, this.Editor().SelecaoInicio());
            let end: Float = this.m_measurer.Offset(text, this.Editor().SelecaoFim());
            this.m_selection.SetTranslation(start, 0.0);
            this.m_selection.SetSize(new Vector2(end - start, this.m_height));
            this.m_selection.SetVisible(true);
        } else {
            this.m_selection.SetVisible(false);
        }
    }
}
