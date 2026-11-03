// bwms-ui-themecolors.reds — as cores de tema.
//
// A cor NÃO é um número guardado aqui. Componentes fixos de `HDRColor` teriam dois problemas:
//   1. **Envelhece calado.** Se um patch do jogo (ou um mod de tema) mexer na paleta, as cores
//      fixas passam a destoar do resto da UI e ninguém percebe até ver na tela.
//   2. **O nome tem que existir no jogo.** Grepando o código do próprio jogo, a paleta real
//      é `MainColors.Red`/`Blue`/`PanelRed`/`PanelDarkRed`/`SupBlue`/`MildRed`/`ActiveBlue`/
//      `ActiveRed`/`ActiveGreen`/`ActiveYellow`/`DarkRed`/`Purple`/`Grey` — nomes como
//      "ElectricBlue" não são cor do Cyberpunk.
//
// Então aqui a cor NÃO é um número que a gente guarda: é resolvida pelo MOTOR, em runtime, pelo
// mesmo mecanismo que o jogo usa na própria UI (`BindProperty(n"tintColor", n"MainColors.X")` —
// idioma confirmado no código do jogo, `redscript-all.reds:341883`). A cor fica sempre certa,
// de graça.
//
// HONESTIDADE SOBRE O LIMITE: `BindProperty` é resolvido pelo estilo que o widget herda da ÁRVORE.
// Um widget solto, fora da árvore, não tem estilo pra resolver. Por isso NÃO existe aqui um
// `ThemeColors.Red() -> HDRColor` sem argumento: seria uma promessa que a engine não cumpre fora
// de contexto. Quem quer a cor entrega o widget; quem só quer pintar usa `Apply`.
//
// DEPENDÊNCIAS confirmadas por grep em `cp77-symbols/redscript-all.reds` ANTES de escrever:
//   - `inkWidget.BindProperty(propertyName: CName, stylePath: CName) -> Bool`   (`:54804`)
//   - `inkWidget.GetTintColor() -> HDRColor`                                     (`:54726`)
//   - `inkWidget.SetTintColor(color: HDRColor)`                                  (`:54728`)
//   - `struct HDRColor { Red, Green, Blue, Alpha: Float }`                       (`:54948`)
// Zero native próprio, zero endereço, zero forge de classe.

public abstract class ThemeColors {

    // --- A paleta REAL do jogo -------------------------------------------------------------
    // Nomes extraídos do código do próprio Cyberpunk, não inventados. São os caminhos de estilo
    // que `Apply`/`ResolveFrom` aceitam.
    public static func Red()          -> CName = n"MainColors.Red"
    public static func DarkRed()      -> CName = n"MainColors.DarkRed"
    public static func MildRed()      -> CName = n"MainColors.MildRed"
    public static func PanelRed()     -> CName = n"MainColors.PanelRed"
    public static func PanelDarkRed() -> CName = n"MainColors.PanelDarkRed"
    public static func Blue()         -> CName = n"MainColors.Blue"
    public static func SupBlue()      -> CName = n"MainColors.SupBlue"
    public static func ActiveBlue()   -> CName = n"MainColors.ActiveBlue"
    public static func ActiveRed()    -> CName = n"MainColors.ActiveRed"
    public static func ActiveGreen()  -> CName = n"MainColors.ActiveGreen"
    public static func ActiveYellow() -> CName = n"MainColors.ActiveYellow"
    public static func Purple()       -> CName = n"MainColors.Purple"
    public static func Grey()         -> CName = n"MainColors.Grey"

    // --- Uso ------------------------------------------------------------------------------

    // Pinta o widget com uma cor do tema. É esta a via recomendada: o motor passa a manter a cor
    // sincronizada com o tema, sem o mod guardar número nenhum.
    // Devolve `false` se o widget não existe ou se o caminho de estilo não resolveu.
    public static func Apply(widget: ref<inkWidget>, color: CName) -> Bool {
        return ThemeColors.ApplyTo(widget, n"tintColor", color);
    }

    // A mesma coisa, para qualquer propriedade de estilo (borda, fundo, etc.), não só a tinta.
    public static func ApplyTo(widget: ref<inkWidget>, property: CName, color: CName) -> Bool {
        if !IsDefined(widget) {
            return false;
        }
        return widget.BindProperty(property, color);
    }

    // Devolve o `HDRColor` que o tema resolve para essa cor, usando o widget dado como contexto
    // de estilo. O widget precisa estar na árvore — ver a nota de honestidade no topo.
    //
    // Preserva a cor original: aplica o binding, lê o resultado, desfaz o binding e restaura o que
    // estava lá. Um helper de leitura não pode deixar rastro no widget de quem chamou — sem o
    // `UnbindProperty`, a próxima atualização de estilo repintaria o widget com a cor lida.
    // Limite: se a tinta do widget já estava ligada a um estilo, essa ligação sai junto.
    public static func ResolveFrom(widget: ref<inkWidget>, color: CName) -> HDRColor {
        let fallback: HDRColor;
        fallback.Red = 1.0;
        fallback.Green = 1.0;
        fallback.Blue = 1.0;
        fallback.Alpha = 1.0;
        if !IsDefined(widget) {
            return fallback;
        }
        let original: HDRColor = widget.GetTintColor();
        if !widget.BindProperty(n"tintColor", color) {
            return fallback;
        }
        let resolved: HDRColor = widget.GetTintColor();
        widget.UnbindProperty(n"tintColor");
        widget.SetTintColor(original);
        return resolved;
    }

    // Constrói um `HDRColor` avulso, para quem precisa de uma cor que NÃO é do tema (destaque de
    // mod, cor de marca). Explicitamente fora da paleta — quem chama sabe que está saindo dela.
    public static func Custom(r: Float, g: Float, b: Float, a: Float) -> HDRColor {
        let c: HDRColor;
        c.Red = r;
        c.Green = g;
        c.Blue = b;
        c.Alpha = a;
        return c;
    }
}
