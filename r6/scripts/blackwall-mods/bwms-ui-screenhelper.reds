// bwms-ui-screenhelper.reds — reimplementação PRÓPRIA do helper de tela (catálogo Codeware #112), escrita a partir do COMPORTAMENTO,
// nunca da fonte alheia. Só a API pública é reconhecível, porque o nome que um mod chama é a forma do plugue e não é de ninguém.
//
// DEPENDÊNCIAS, todas confirmadas por grep em `cp77-symbols/redscript-all.reds` ANTES de escrever:
//   - `GameInstance.GetSettingsSystem(self) -> ref<UserSettings>`      (`:14212`, native vanilla)
//   - `UserSettings.GetVar(groupPath, varName) -> ref<ConfigVar>`      (`:171396`, native vanilla)
//   - `ConfigVarListString.GetValue() -> String`                        (`:208513`, native vanilla)
//   - `StrSplit`/`StringToInt`                                          (`:535769`/`:241042`)
// Zero native próprio, zero endereço, zero forge de classe.
//
// DECISÕES DE DESIGN QUE SÃO NOSSAS (e divergem de propósito de qualquer porte):
//   1. A resolução vira um DADO TIPADO (`BwmsScreenSize`), não uma string que cada mod parseia de
//      novo do seu jeito. Parsear string de configuração em N lugares é onde bug de UI nasce.
//   2. O parse é DEFENSIVO: qualquer formato inesperado devolve o fallback 1920x1080 em vez de
//      zero. Um widget dimensionado com 0 desaparece silenciosamente — o modo de falha mais caro
//      de depurar em UI. Melhor entregar um tamanho plausível e sinalizar por `IsValid`.
//   3. `GetSmartScale` é razão contra **1080p de altura**, e isso está documentado aqui em vez de
//      implícito: um mod que escala fonte/margem quer saber contra o que está escalando.

public struct BwmsScreenSize {
    public let Width: Int32;
    public let Height: Int32;
    // `false` quando a leitura falhou e os campos são o fallback — deixa o chamador decidir se
    // aceita o palpite ou se prefere não desenhar nada.
    public let IsValid: Bool;
    // A mesma medida em `Float`, com os nomes que os mods do Windows leem do `Vector2` (`ScreenHelper.GetScreenSize(game).X`).
    public let X: Float;
    public let Y: Float;
}

public abstract class ScreenHelper {

    // Largura/altura de referência. Toda a UI do jogo é desenhada contra 1080p, então é a base
    // honesta pra escalar. Constantes nomeadas, não números soltos no meio do cálculo.
    public static func ReferenceWidth() -> Float = 1920.0
    public static func ReferenceHeight() -> Float = 1080.0

    // A string crua que o jogo guarda em `/video/display Resolution` (ex.: "2560x1440").
    // Devolve "" se a config não resolver — nunca crasha, nunca inventa.
    public static func GetResolution(game: GameInstance) -> String {
        let settings: ref<UserSettings> = GameInstance.GetSettingsSystem(game);
        if !IsDefined(settings) {
            return "";
        }
        let raw: ref<ConfigVar> = settings.GetVar(n"/video/display", n"Resolution");
        let asList: ref<ConfigVarListString> = raw as ConfigVarListString;
        if !IsDefined(asList) {
            return "";
        }
        return asList.GetValue();
    }

    // O parse é PÚBLICO de propósito: assim dá para testá-lo contra qualquer resolução, e não só
    // contra a da máquina de quem escreveu o código. Quem instala o mod pode estar em 1080p,
    // 1440p, 4K ou ultrawide — e o parse tem que valer para todas.
    public static func ParseResolution(text: String) -> BwmsScreenSize {
        let out: BwmsScreenSize;
        out.Width = 1920;
        out.Height = 1080;
        out.IsValid = false;
        out.X = 1920.0;
        out.Y = 1080.0;

        if StrLen(text) == 0 {
            return out;
        }
        // O jogo escreve "LARGURAxALTURA". Aceito só essa forma: duas partes, ambas positivas.
        let parts: array<String> = StrSplit(text, "x");
        if ArraySize(parts) != 2 {
            return out;
        }
        let w: Int32 = StringToInt(parts[0], 0);
        let h: Int32 = StringToInt(parts[1], 0);
        if w <= 0 || h <= 0 {
            return out;
        }
        out.Width = w;
        out.Height = h;
        out.X = Cast<Float>(w);
        out.Y = Cast<Float>(h);
        out.IsValid = true;
        return out;
    }

    // O mesmo dado, já tipado e validado. É este que os outros helpers usam.
    public static func GetScreenSize(game: GameInstance) -> BwmsScreenSize {
        return ScreenHelper.ParseResolution(ScreenHelper.GetResolution(game));
    }

    // Todas as resoluções que a máquina oferece. Serve para o mod montar um seletor próprio — e
    // serve para PROVAR que o parse aguenta o que existe de verdade nesta máquina, não só a
    // resolução em uso no momento.
    public static func GetAvailableResolutions(game: GameInstance) -> array<String> {
        let vazio: array<String>;
        let settings: ref<UserSettings> = GameInstance.GetSettingsSystem(game);
        if !IsDefined(settings) {
            return vazio;
        }
        let asList: ref<ConfigVarListString> = settings.GetVar(n"/video/display", n"Resolution") as ConfigVarListString;
        if !IsDefined(asList) {
            return vazio;
        }
        return asList.GetValues();
    }

    // Quantas das resoluções oferecidas pela máquina o parse consegue ler. Se este número for
    // menor que o total, existe formato que o mod não entende — e é melhor saber disso por
    // medição do que por relato de usuário.
    public static func CountParseable(game: GameInstance) -> Int32 {
        let ok: Int32 = 0;
        for entry in ScreenHelper.GetAvailableResolutions(game) {
            if ScreenHelper.ParseResolution(entry).IsValid {
                ok += 1;
            }
        }
        return ok;
    }

    // Atalhos, pra um mod não ter que lidar com o struct quando só quer um número.
    public static func GetScreenWidth(game: GameInstance) -> Int32 {
        return ScreenHelper.GetScreenSize(game).Width;
    }

    public static func GetScreenHeight(game: GameInstance) -> Int32 {
        return ScreenHelper.GetScreenSize(game).Height;
    }

    // Quanto a tela atual é maior/menor que a referência de 1080p, na ALTURA (é a dimensão que
    // manda em tipografia e espaçamento vertical; largura varia com ultrawide sem que o texto
    // deva crescer junto).
    public static func GetSmartScale(game: GameInstance) -> Float {
        let size: BwmsScreenSize = ScreenHelper.GetScreenSize(game);
        return Cast<Float>(size.Height) / ScreenHelper.ReferenceHeight();
    }

    // Razão de aspecto — útil pra decidir layout (ultrawide vs 16:9) sem refazer a conta.
    public static func GetAspectRatio(game: GameInstance) -> Float {
        let size: BwmsScreenSize = ScreenHelper.GetScreenSize(game);
        if size.Height <= 0 {
            return 16.0 / 9.0;
        }
        return Cast<Float>(size.Width) / Cast<Float>(size.Height);
    }
}
