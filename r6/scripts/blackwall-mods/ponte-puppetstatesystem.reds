// ponte-puppetstatesystem.reds — `PuppetStateSystem`: o `scriptedSystem` dos 4 registros de sufixo que o runtime cria
// (`registros_de_sufixo_tick`):
//
//   itemsFactoryAppearanceSuffix.BodyType  → GetBodyTypeSuffix
//   itemsFactoryAppearanceSuffix.ArmsState → GetArmsStateSuffix
//   itemsFactoryAppearanceSuffix.LegsState → GetFeetStateSuffix
//   itemsFactoryAppearanceSuffix.FeetState → GetFeetStateSuffix
//
// O motor chama as funções pelo NOME, com a assinatura do `EquipmentSystem.GetHairSuffix` vanilla: nomes e assinaturas são contrato.

public class PuppetStateSystem extends ScriptableSystem {
    // O runtime não lê os tipos de corpo dos arquivos de mod: todo corpo é o base.
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
