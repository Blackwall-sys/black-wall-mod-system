// bwms-text-measurer.reds — largura de texto medida pelo próprio motor, 2026-09-13.
//
// PARA QUE SERVE: um campo de texto precisa saber onde desenhar o caret e a seleção, e o script não tem tipografia. A largura sai do layout do
// próprio motor, com a mesma família e tamanho de fonte do texto visível.
//
// A medida é assíncrona: caracteres entram numa fila, um vai para a sombra por vez, e a largura desejada dela é lida na volta seguinte do
// gatilho; o ouvinte é avisado quando a fila esvazia.
//
// O espaço sozinho não mede (o texto é aparado); sai de `"M M"` menos duas vezes `"M"`. Kerning é ignorado: a largura de um trecho é a soma dos
// caracteres. Os caracteres entram por `BwmsUTF8Split`, um por codepoint: acento pré-composto conta como 1; decomposto (letra + U+0301), como 2.
//
// O texto é quebrado em caracteres uma vez e guardado com as larguras acumuladas: `Queue` e os `Offset` do mesmo texto leem o cache. A quebra
// refaz quando o texto muda; as somas, quando o texto muda ou uma largura entra ou sai.
//
// USO: `let m = BwmsTextMeasurer.Create(); m.Mount(raiz); m.SetFont(fonte, 38); m.OnReady(this, n"OnMedido"); m.Queue(texto);` e, depois do aviso,
// `m.Offset(texto, posicao)`.

public class BwmsTextMeasurer extends BwmsUIController {
    private let m_shadow: ref<inkText>;
    private let m_tick: ref<inkAnimProxy>;
    private let m_widths: ref<inkStringMap>;
    private let m_queue: array<String>;
    private let m_current: String;
    private let m_readyTarget: wref<IScriptable>;
    private let m_readyFn: CName;
    private let m_font: String;
    private let m_fontSize: Int32;
    private let m_ticks: Int32;
    // Último texto quebrado e os caracteres dele.
    private let m_splitText: String;
    private let m_chars: array<String>;
    // `m_prefix[k]` = soma das larguras dos `k` primeiros caracteres de `m_chars`, até o primeiro sem largura. Vazio = recalcular.
    private let m_prefix: array<Float>;

    // Quantas vezes o tick de medida rodou.
    public func TickCount() -> Int32 {
        return this.m_ticks;
    }

    public static func DefaultFont() -> String = "base\\gameplay\\gui\\fonts\\raj\\raj.inkfontfamily"
    public static func SpaceProbe() -> String = "M M"

    public static func Create() -> ref<BwmsTextMeasurer> {
        let m: ref<BwmsTextMeasurer> = new BwmsTextMeasurer();
        m.Construct();
        return m;
    }

    // A raiz é o próprio texto de medida. Opacidade 0 em vez de `SetVisible(false)`: o widget escondido sai do layout e o motor deixa de
    // calcular o tamanho desejado dele, que é a medida.
    protected func OnCreate() -> ref<inkWidget> {
        if Equals(this.m_font, "") {
            this.m_font = BwmsTextMeasurer.DefaultFont();
            this.m_fontSize = 38;
        }
        if !IsDefined(this.m_widths) {
            this.m_widths = new inkStringMap();
        }
        let shadow: ref<inkText> = new inkText();
        shadow.SetName(n"bwmsTextMeasurerShadow");
        // Sem família de fonte o `inkText` mede (0,0).
        shadow.SetFontFamily(this.m_font);
        shadow.SetFontSize(this.m_fontSize);
        shadow.SetOpacity(0.0);
        this.m_shadow = shadow;
        return shadow;
    }

    // Duração de uma volta do gatilho, em segundos: uma volta por quadro a 60 Hz.
    public static func TickPeriod() -> Float = 0.0166

    // O gatilho é uma animação em laço infinito na própria sombra (opacidade 0 → 0, nada visível muda); cada fim de volta chama `OnMeasureTick`.
    // Nasce aqui, e não no `OnInitialize`: este roda dentro do `Construct`, antes de a sombra estar numa árvore que o motor anima.
    private func EnsureTick() -> Void {
        if IsDefined(this.m_tick) || !IsDefined(this.m_shadow) {
            return;
        }
        let fade: ref<inkAnimTransparency> = new inkAnimTransparency();
        fade.SetStartTransparency(0.0);
        fade.SetEndTransparency(0.0);
        fade.SetDuration(BwmsTextMeasurer.TickPeriod());
        let def: ref<inkAnimDef> = new inkAnimDef();
        def.AddInterpolator(fade);
        let options: inkAnimOptions;
        options.loopType = inkanimLoopType.Cycle;
        options.loopInfinite = true;
        this.m_tick = this.m_shadow.PlayAnimationWithOptions(def, options);
        if IsDefined(this.m_tick) {
            this.m_tick.RegisterToCallback(inkanimEventType.OnEndLoop, this, n"OnMeasureTick");
        }
    }

    // Troca a fonte da sombra. As larguras guardadas eram de outra fonte e são descartadas.
    public func SetFont(family: String, size: Int32) -> Void {
        this.m_font = family;
        this.m_fontSize = size;
        this.m_widths.Clear();
        ArrayClear(this.m_prefix);
        if IsDefined(this.m_shadow) {
            this.m_shadow.SetFontFamily(family);
            this.m_shadow.SetFontSize(size);
        }
    }

    // Quem é avisado quando a fila esvazia. Um ouvinte por medidor.
    public func OnReady(target: ref<IScriptable>, functionName: CName) -> Void {
        this.m_readyTarget = target;
        this.m_readyFn = functionName;
    }

    public func IsBusy() -> Bool {
        return NotEquals(this.m_current, "") || ArraySize(this.m_queue) > 0;
    }

    public func Knows(char: String) -> Bool {
        return this.m_widths.KeyExist(char);
    }

    // Largura de um caractere já medido, ou -1.
    public func Width(char: String) -> Float {
        let known: Bool = this.m_widths.KeyExist(char);
        if !known {
            return -1.0;
        }
        let stored: Uint64 = this.m_widths.Get(char);
        return Cast<Float>(stored) / 1000.0;
    }

    // Põe na fila os caracteres de `text` que ainda não têm largura. Devolve quantos entraram.
    public func Queue(text: String) -> Int32 {
        // TODO(F1-53): Queue e Offset medem 1 codepoint por vez, então uma marca decomposta (U+0301) é medida sozinha, fora da base. Com prova
        // de NFD chegando pela colagem ou pelo SetText, os dois andam pela native BwmsTextClusterLen, a mesma fronteira do EditorTexto.
        this.Split(text);
        let count: Int32 = ArraySize(this.m_chars);
        let added: Int32 = 0;
        let i: Int32 = 0;
        while i < count {
            let char: String = this.m_chars[i];
            if !this.Knows(char) && !this.InQueue(char) {
                ArrayPush(this.m_queue, char);
                added += 1;
            }
            i += 1;
        }
        let space: Bool = this.InQueue(" ");
        if space && !this.Knows("M") && !this.InQueue("M") {
            ArrayInsert(this.m_queue, 0, "M");
            added += 1;
        }
        if !this.IsBusy() {
            this.NotifyReady();
        } else {
            if Equals(this.m_current, "") {
                this.Advance();
            }
        }
        return added;
    }

    // Soma das larguras dos `position` primeiros caracteres de `text`, ou -1 se algum ainda não foi medido.
    public func Offset(text: String, position: Int32) -> Float {
        this.Split(text);
        if ArraySize(this.m_prefix) == 0 {
            this.SumWidths();
        }
        let limit: Int32 = Min(position, ArraySize(this.m_chars));
        if limit <= 0 {
            return 0.0;
        }
        if limit >= ArraySize(this.m_prefix) {
            return -1.0;
        }
        return this.m_prefix[limit];
    }

    // Uma native copia o texto e quebra; as outras leem um caractere cada, sem receber o texto. Cada retorno vai pra uma local antes de ser usado.
    private func Split(text: String) -> Void {
        if Equals(text, this.m_splitText) {
            return;
        }
        let count: Int32 = BwmsUTF8Split(text);
        ArrayClear(this.m_chars);
        let i: Int32 = 0;
        while i < count {
            let char: String = BwmsUTF8SplitChar(i);
            ArrayPush(this.m_chars, char);
            i += 1;
        }
        this.m_splitText = text;
        ArrayClear(this.m_prefix);
    }

    // Soma do primeiro caractere ao último, um por vez: `m_prefix[k]` é a soma direta dos `k` primeiros, bit a bit (Float não é associativo). Para
    // no primeiro caractere sem largura.
    private func SumWidths() -> Void {
        let total: Float = 0.0;
        ArrayPush(this.m_prefix, total);
        let i: Int32 = 0;
        let count: Int32 = ArraySize(this.m_chars);
        while i < count {
            let w: Float = this.Width(this.m_chars[i]);
            if w < 0.0 {
                return;
            }
            total += w;
            ArrayPush(this.m_prefix, total);
            i += 1;
        }
    }

    private func InQueue(char: String) -> Bool {
        if Equals(this.m_current, char) {
            return true;
        }
        let i: Int32 = 0;
        let n: Int32 = ArraySize(this.m_queue);
        while i < n {
            if Equals(this.m_queue[i], char) {
                return true;
            }
            i += 1;
        }
        return false;
    }

    private func Advance() -> Void {
        if ArraySize(this.m_queue) == 0 {
            this.m_current = "";
            this.m_shadow.SetText("");
            if IsDefined(this.m_tick) {
                this.m_tick.Pause();
            }
            this.NotifyReady();
            return;
        }
        let next: String = this.m_queue[0];
        ArrayErase(this.m_queue, 0);
        this.m_current = next;
        if Equals(next, " ") {
            this.m_shadow.SetText(BwmsTextMeasurer.SpaceProbe());
        } else {
            this.m_shadow.SetText(next);
        }
        this.EnsureTick();
        this.m_tick.Resume();
    }

    // Roda a cada volta do gatilho; enquanto a fonte carrega a largura fica 0 e o mesmo caractere é lido de novo na volta seguinte.
    protected cb func OnMeasureTick(anim: ref<inkAnimProxy>) -> Bool {
        this.m_ticks += 1;
        if Equals(this.m_current, "") {
            if IsDefined(this.m_tick) {
                this.m_tick.Pause();
            }
            return true;
        }
        let width: Float = this.m_shadow.GetDesiredSize().X;
        if width < 0.01 {
            return true;
        }
        if Equals(this.m_current, " ") {
            width -= 2.0 * this.Width("M");
        }
        if width < 0.0 {
            width = 0.0;
        }
        let stored: Uint64 = Cast<Uint64>(width * 1000.0);
        // `inkStringMap.Set` só troca chave que já existe; chave nova entra por `Insert`.
        if this.m_widths.KeyExist(this.m_current) {
            this.m_widths.Set(this.m_current, stored);
        } else {
            this.m_widths.Insert(this.m_current, stored);
        }
        ArrayClear(this.m_prefix);
        this.Advance();
        return true;
    }

    private func NotifyReady() -> Void {
        let target: ref<IScriptable> = this.m_readyTarget;
        if IsDefined(target) {
            BwmsCallMethod(target, this.m_readyFn);
        }
    }
}

// Par do medidor (cp77-console `texto_e_numeros`): `BwmsUTF8Split` quebra o texto em codepoints e guarda na thread; `BwmsUTF8SplitChar` devolve o
// caractere `index` da última quebra, vazio fora do alcance. Chamar os dois na mesma função, sem script no meio.
native func BwmsUTF8Split(s: String) -> Int32
native func BwmsUTF8SplitChar(index: Int32) -> String
