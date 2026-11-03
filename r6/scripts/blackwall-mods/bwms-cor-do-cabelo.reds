// bwms-cor-do-cabelo.reds — a cor de cabelo ativa pelas opções de customização da cabeça. Helper do
// BWMS, à parte da aparência dinâmica (o estado dinâmico da entidade calcula a cor por conta própria).
//
// Cadeia só de script vanilla: `GameInstance.GetCharacterCustomizationSystem(game).GetHeadOptions()` dá as opções; a de cor é a do slot
// `hair_color` — o `GetSlotName` vanilla agrupa estilo e cor sob `"UI_Hairs"` só pro enquadramento da câmera, então esse rótulo não
// separa a opção de cor. `option.info as gameuiAppearanceInfo` traz `definitions`, e `option.currIndex` indexa a definição ativa, com o
// campo `color`. `GetHeadOptions` só vem preenchido na criação de personagem e no espelho; fora disso a cor volta zerada. Sem prova ao
// vivo deste helper.

public class BwmsCharacterCustomizationHelper {
    // Núcleo puro (testável sem GameInstance): dado o array de opções já obtido, acha a opção
    // de cor de cabelo e devolve a cor ATIVA. `Color` zerado (0,0,0,0) = não achou/sem dado.
    public static func GetHairColor(options: array<ref<CharacterCustomizationOption>>) -> Color {
        let empty: Color = new Color(Cast<Uint8>(0), Cast<Uint8>(0), Cast<Uint8>(0), Cast<Uint8>(0));
        let i: Int32 = 0;
        while i < ArraySize(options) {
            let opt: ref<CharacterCustomizationOption> = options[i];
            if IsDefined(opt) && Equals(opt.bodyPart, gameuiCharacterCustomizationPart.Head)
                && Equals(opt.info.uiSlot, n"hair_color") {
                let appearance: ref<gameuiAppearanceInfo> = opt.info as gameuiAppearanceInfo;
                if IsDefined(appearance) && Cast<Int32>(opt.currIndex) < ArraySize(appearance.definitions) {
                    return appearance.definitions[Cast<Int32>(opt.currIndex)].color;
                };
                return empty;
            };
            i += 1;
        };
        return empty;
    }

    // Conveniência ponta-a-ponta: resolve o singleton + busca as opções de cabeça + filtra.
    public static func GetPlayerHairColor(game: GameInstance) -> Color {
        let sys: ref<gameuiICharacterCustomizationSystem> = GameInstance.GetCharacterCustomizationSystem(game);
        if !IsDefined(sys) {
            return new Color(Cast<Uint8>(0), Cast<Uint8>(0), Cast<Uint8>(0), Cast<Uint8>(0));
        };
        return BwmsCharacterCustomizationHelper.GetHairColor(sys.GetHeadOptions());
    }
}
