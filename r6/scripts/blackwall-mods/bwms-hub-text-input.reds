// bwms-hub-text-input.reds — campo de texto com a cara dos filtros dos menus do jogo (Codeware `#116`), 2026-09-13.
//
// PARA QUE SERVE: um campo que combina com as telas de inventário, mapa e diário, sem o mod desenhar moldura. É o `BwmsTextInput` inteiro, com cinco
// camadas de imagem da forma `sorting` do atlas de formas do jogo, cores pelo estilo `main_colors` e transições curtas:
//   - fundo escuro, sempre;
//   - preenchimento azul bem fraco com foco;
//   - moldura, sempre;
//   - moldura vermelha no hover;
//   - moldura azul com foco.
// Desabilitado, o campo inteiro cai pra 30% de opacidade e as molduras de hover e foco somem.
//
// USO: `let campo = BwmsHubTextInput.Create(520.0); campo.Mount(raiz); campo.Focus(controller);`. `SetAnimated(false)` troca as transições por
// mudança imediata.
//
// De Codeware: `HubTextInput` → `BwmsHubTextInput`.

public class BwmsHubTextInput extends BwmsTextInput {
    private let m_hubFill: ref<inkImage>;
    private let m_hubHover: ref<inkImage>;
    private let m_hubFocus: ref<inkImage>;
    private let m_hubLayers: Int32;
    private let m_animated: Bool;
    private let m_fillProxy: ref<inkAnimProxy>;
    private let m_hoverProxy: ref<inkAnimProxy>;
    private let m_focusProxy: ref<inkAnimProxy>;
    private let m_rootProxy: ref<inkAnimProxy>;

    public static func Atlas() -> ResRef = r"base\\gameplay\\gui\\common\\shapes\\atlas_shapes_sync.inkatlas"
    public static func Style() -> ResRef = r"base\\gameplay\\gui\\common\\main_colors.inkstyle"

    public static func Create(width: Float) -> ref<BwmsHubTextInput> {
        let input: ref<BwmsHubTextInput> = new BwmsHubTextInput();
        input.m_width = width;
        input.m_fontSize = 38;
        input.m_height = 80.0;
        input.m_padding = 18.0;
        input.m_animated = true;
        input.Construct();
        return input;
    }

    public func SetAnimated(animated: Bool) -> Void {
        this.m_animated = animated;
    }

    // Quantas camadas de tema entraram na moldura (5 quando o atlas e o estilo carregaram as partes).
    public func LayerCount() -> Int32 {
        return this.m_hubLayers;
    }

    public func FocusLayerOpacity() -> Float {
        return this.m_hubFocus.GetOpacity();
    }

    public func HoverLayerOpacity() -> Float {
        return this.m_hubHover.GetOpacity();
    }

    public func FillLayerOpacity() -> Float {
        return this.m_hubFill.GetOpacity();
    }

    public func RootOpacity() -> Float {
        return this.m_frame.GetOpacity();
    }

    protected func DecorateFrame(frame: ref<inkCanvas>) -> Void {
        // Da camada de trás pra da frente; cada uma entra no índice 0 antes da seguinte, então a ordem de criação é a inversa.
        this.m_hubFocus = this.Layer(frame, n"hub_focus", n"sorting_fg", n"MainColors.Blue", 0.0);
        this.m_hubHover = this.Layer(frame, n"hub_hover", n"sorting_fg", n"MainColors.Red", 0.0);
        this.Layer(frame, n"hub_frame", n"sorting_fg", n"MainColors.Fullscreen_SecondaryBackground4", 1.0);
        this.m_hubFill = this.Layer(frame, n"hub_fill", n"sorting_bg", n"MainColors.Blue", 0.0);
        this.Layer(frame, n"hub_bg", n"sorting_bg", n"MainColors.Fullscreen_PrimaryBackgroundDarkest", 0.61);
    }

    private func Layer(frame: ref<inkCanvas>, name: CName, part: CName, color: CName, opacity: Float) -> ref<inkImage> {
        let image: ref<inkImage> = new inkImage();
        image.SetName(name);
        image.SetAtlasResource(BwmsHubTextInput.Atlas());
        let partOk: Bool = image.SetTexturePart(part);
        image.SetStyle(BwmsHubTextInput.Style());
        let colorOk: Bool = image.BindProperty(n"tintColor", color);
        image.SetAnchor(inkEAnchor.Fill);
        // O script deste build não declara `SetNineSliceScale`/`SetNineSliceGrid` no `inkImage`: a native liga o nine-slice pelos campos do RTTI.
        let nineSlice: Bool = BwmsImageNineSlice(image, 50.0, 30.0, 100.0, 30.0);
        image.SetOpacity(opacity);
        image.Reparent(frame, 0);
        if partOk && colorOk && nineSlice {
            this.m_hubLayers += 1;
        }
        return image;
    }

    protected func OnStateChanged() -> Void {
        super.OnStateChanged();
        if !IsDefined(this.m_hubFocus) {
            return;
        }
        let focused: Bool = this.m_focused && !this.m_disabled;
        let hovered: Bool = this.m_hovered && !this.m_disabled;
        this.m_fillProxy = this.Fade(this.m_hubFill, this.m_fillProxy, focused ? 0.02 : 0.0, 0.1);
        this.m_focusProxy = this.Fade(this.m_hubFocus, this.m_focusProxy, focused ? 1.0 : 0.0, 0.15);
        this.m_hoverProxy = this.Fade(this.m_hubHover, this.m_hoverProxy, hovered ? 0.6 : 0.0, 0.15);
        this.m_rootProxy = this.Fade(this.m_frame, this.m_rootProxy, this.m_disabled ? 0.3 : 1.0, 0.05);
    }

    // Transição da opacidade atual até `target`; sem animação, o valor entra direto.
    private func Fade(widget: ref<inkWidget>, running: ref<inkAnimProxy>, target: Float, duration: Float) -> ref<inkAnimProxy> {
        if IsDefined(running) {
            running.Stop();
        }
        if !this.m_animated {
            widget.SetOpacity(target);
            return null;
        }
        let step: ref<inkAnimTransparency> = new inkAnimTransparency();
        step.SetStartTransparency(widget.GetOpacity());
        step.SetEndTransparency(target);
        step.SetDuration(duration);
        let def: ref<inkAnimDef> = new inkAnimDef();
        def.AddInterpolator(step);
        let proxy: ref<inkAnimProxy> = widget.PlayAnimation(def);
        return proxy;
    }
}

// Liga o nine-slice do `inkImage` com a grade `left`/`top`/`right`/`bottom`: grava `useNineSliceScale` e `nineSliceScale` pelos offsets que o RTTI reporta.
// `false` quando a classe não tem os dois campos.
public static native func BwmsImageNineSlice(image: ref<inkImage>, left: Float, top: Float, right: Float, bottom: Float) -> Bool
