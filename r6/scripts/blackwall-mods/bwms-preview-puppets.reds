// bwms-preview-puppets.reds — os bonecos de pré-visualização do V.
//
// Toda tela que mostra o personagem cria um `gamePuppet` próprio, separado do V: o do inventário (paperdoll), o da criação de personagem e
// do espelho, e o do modo foto. É nele que um mod de roupa, pose ou câmera precisa mexer pra o efeito aparecer na tela; mas quem guarda o
// boneco é o controller da tela, e mod nenhum tem o controller.
//
// Aqui cada boneco é anotado no V no momento em que o jogo termina de prepará-lo:
//   - inventário e personagem: `inkPuppetPreviewGameController.OnPreviewInitialized`, separados pela classe nativa do controller
//     (`gameuiInventoryPuppetPreviewGameController` e `gameuiCharacterCreationPuppetPreviewGameController`, conferida ao vivo em 2026-08-11);
//   - modo foto: `PhotoModePlayerEntityComponent.SetupInventory`, onde o jogo pega o `fakePuppet`.
// As anotações ficam no `PlayerPuppet` (classe de script, campo acrescentado sem risco) e são `wref`: não seguram o boneco, quem decide
// quando ele morre é o jogo (3 s depois de pedir o fechamento do inventário ele ainda existia, boot 253); a leitura volta nula quando o jogo o solta.
//
// Leitura: `BwmsPreviewPuppets.Inventory(player)` e as irmãs; `...Of(game)` pra quem só tem o `GameInstance`.

@addField(PlayerPuppet)
private let m_bwmsBonecoInventario: wref<gamePuppet>;

@addField(PlayerPuppet)
private let m_bwmsBonecoPersonagem: wref<gamePuppet>;

@addField(PlayerPuppet)
private let m_bwmsBonecoFoto: wref<gamePuppet>;

// 1 = inventário, 2 = personagem, 3 = foto.
@addMethod(PlayerPuppet)
public func BwmsAnotarBoneco(tela: Int32, boneco: wref<gamePuppet>) -> Void {
    if tela == 1 {
        this.m_bwmsBonecoInventario = boneco;
    } else {
        if tela == 2 {
            this.m_bwmsBonecoPersonagem = boneco;
        } else {
            if tela == 3 {
                this.m_bwmsBonecoFoto = boneco;
            };
        };
    };
}

@addMethod(PlayerPuppet)
public func BwmsBonecoDaTela(tela: Int32) -> wref<gamePuppet> {
    if tela == 1 {
        return this.m_bwmsBonecoInventario;
    };
    if tela == 2 {
        return this.m_bwmsBonecoPersonagem;
    };
    if tela == 3 {
        return this.m_bwmsBonecoFoto;
    };
    return null;
}

public abstract class BwmsPreviewPuppets {

    public static func Inventory(player: ref<PlayerPuppet>) -> wref<gamePuppet> {
        return BwmsPreviewPuppets.DaTela(player, 1);
    }

    public static func Customization(player: ref<PlayerPuppet>) -> wref<gamePuppet> {
        return BwmsPreviewPuppets.DaTela(player, 2);
    }

    public static func Photo(player: ref<PlayerPuppet>) -> wref<gamePuppet> {
        return BwmsPreviewPuppets.DaTela(player, 3);
    }

    public static func InventoryOf(game: GameInstance) -> wref<gamePuppet> {
        let v: ref<PlayerPuppet> = BwmsPreviewPuppets.JogadorDe(game);
        return BwmsPreviewPuppets.DaTela(v, 1);
    }

    public static func CustomizationOf(game: GameInstance) -> wref<gamePuppet> {
        let v: ref<PlayerPuppet> = BwmsPreviewPuppets.JogadorDe(game);
        return BwmsPreviewPuppets.DaTela(v, 2);
    }

    public static func PhotoOf(game: GameInstance) -> wref<gamePuppet> {
        let v: ref<PlayerPuppet> = BwmsPreviewPuppets.JogadorDe(game);
        return BwmsPreviewPuppets.DaTela(v, 3);
    }

    // Qual tela a classe nativa do controller de preview representa; 0 pra preview que não é do V (loja, ripperdoc...).
    public static func TelaDoController(classe: CName) -> Int32 {
        if Equals(classe, n"gameuiInventoryPuppetPreviewGameController") {
            return 1;
        };
        if Equals(classe, n"gameuiCharacterCreationPuppetPreviewGameController") {
            return 2;
        };
        return 0;
    }

    private static func DaTela(player: ref<PlayerPuppet>, tela: Int32) -> wref<gamePuppet> {
        if !IsDefined(player) {
            return null;
        };
        let b: wref<gamePuppet> = player.BwmsBonecoDaTela(tela);
        return b;
    }

    private static func JogadorDe(game: GameInstance) -> ref<PlayerPuppet> {
        let sistema: ref<PlayerSystem> = GameInstance.GetPlayerSystem(game);
        if !IsDefined(sistema) {
            return null;
        };
        let obj: ref<GameObject> = sistema.GetLocalPlayerMainGameObject();
        return obj as PlayerPuppet;
    }
}

@wrapMethod(inkPuppetPreviewGameController)
protected cb func OnPreviewInitialized() -> Bool {
    let r: Bool = wrappedMethod();
    let classe: CName = this.GetClassName();
    let tela: Int32 = BwmsPreviewPuppets.TelaDoController(classe);
    if tela > 0 {
        let obj: ref<GameObject> = this.GetPlayerControlledObject();
        let v: ref<PlayerPuppet> = obj as PlayerPuppet;
        let boneco: wref<gamePuppet> = this.GetGamePuppet();
        if IsDefined(v) {
            v.BwmsAnotarBoneco(tela, boneco);
        };
    };
    return r;
}

@wrapMethod(PhotoModePlayerEntityComponent)
private final func SetupInventory(isCurrentPlayerObjectCustomizable: Bool) -> Void {
    wrappedMethod(isCurrentPlayerObjectCustomizable);
    let v: ref<PlayerPuppet> = this.mainPuppet;
    if IsDefined(v) {
        v.BwmsAnotarBoneco(3, this.fakePuppet);
    };
}
