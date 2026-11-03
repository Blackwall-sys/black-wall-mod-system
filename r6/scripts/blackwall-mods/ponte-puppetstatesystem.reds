// ponte-puppetstatesystem.reds — ArchiveXL `App::PuppetStateSystem` (item #32; estado de braços e pés pelo #29).
// `extends ScriptableSystem`, sem native novo. É o `scriptedSystem` dos 4 registros de sufixo que o runtime cria
// (`registros_de_sufixo_tick`, como o `CreateSuffixRecord` do `OnTweakDBReady` da referência):
//
//   itemsFactoryAppearanceSuffix.BodyType  → GetBodyTypeSuffix
//   itemsFactoryAppearanceSuffix.ArmsState → GetArmsStateSuffix
//   itemsFactoryAppearanceSuffix.LegsState → GetFeetStateSuffix
//   itemsFactoryAppearanceSuffix.FeetState → GetFeetStateSuffix
//
// Braços e pés vêm do `PuppetStateHandler` do dono (`ponte-puppetstate-armsdetect.reds`, item #29); dono sem handler devolve os
// padrões da referência (`BaseArms` e `None`, que vira sufixo vazio). `GetBodyTypeSuffix` devolve sempre `"BaseBody"`: os tipos de corpo
// vêm do `player.bodyTypes` dos `.xl`, que o runtime não lê.

public class PuppetStateSystem extends ScriptableSystem {
    public func GetBodyTypeSuffix(itemID: ItemID, owner: wref<GameObject>, suffixRecord: ref<ItemsFactoryAppearanceSuffixBase_Record>) -> String {
        return "BaseBody";
    }

    public func GetArmsStateSuffix(itemID: ItemID, owner: wref<GameObject>, suffixRecord: ref<ItemsFactoryAppearanceSuffixBase_Record>) -> String {
        let handler: ref<PuppetStateHandler> = PuppetStateHandler.ForOwner(owner);
        if !IsDefined(handler) {
            return "BaseArms";
        };
        return PuppetStateHandler.ArmsStateName(handler.GetArmsState());
    }

    public func GetFeetStateSuffix(itemID: ItemID, owner: wref<GameObject>, suffixRecord: ref<ItemsFactoryAppearanceSuffixBase_Record>) -> String {
        let handler: ref<PuppetStateHandler> = PuppetStateHandler.ForOwner(owner);
        if !IsDefined(handler) {
            return "";
        };
        return PuppetStateHandler.FeetStateName(handler.GetFeetState());
    }

    public static func GetInstance(game: GameInstance) -> ref<PuppetStateSystem> {
        return GameInstance.GetScriptableSystemsContainer(game).Get(n"PuppetStateSystem") as PuppetStateSystem;
    }
}
