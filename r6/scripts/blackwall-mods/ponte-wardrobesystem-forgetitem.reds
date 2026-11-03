// ponte-wardrobesystem-forgetitem.reds — Codeware `Player/WardrobeSystem.reds`:
// `@addMethod(WardrobeSystem) public native func ForgetItemID(itemID: ItemID) -> Bool` (itens #46/#208).
//
// A fonte do Codeware (`WardrobeSystemEx::ForgetItemID`, `App/Player/WardrobeSystemEx.hpp`) faz 2 coisas: (1) deriva `appearanceName`
// via `GetFlatValue<CName>({itemID.tdbid, ".appearanceName"})` e tenta remover por essa chave; (2) se falhar, varre o
// `HashMap<CName,ItemID>` (`store.ForEach`) comparando `tdbid` e remove TODAS as chaves que baterem.
//
// Aqui só a 2ª, o SCAN por `tdbid`, como via única: não depende de o item ter o flat `.appearanceName` no TweakDB, e usa o layout do
// `HashMap` conferido ao vivo contra um save real (`wardrobe_forget_by_tdbid`, roupas_e_armas/mod.rs; `ItemID.tdbid@node+0x10`,
// `RED4EXT_ASSERT_SIZE(ItemID, 0x10)` no `RED4ext.SDK/include/RED4ext/NativeTypes.hpp`).
//
// `ItemID.GetTDBID(itemID) -> TweakDBID` e `TDBID.ToNumber(tweakDBID) -> Uint64` são natives vanilla (`orphans.script:18613`).
native func BwmsWardrobeForgetByTDBID(sys: ref<IScriptable>, tdbid: Uint64) -> Int32

@addMethod(WardrobeSystem)
public func ForgetItemID(itemID: ItemID) -> Bool {
    let tdbid: TweakDBID = ItemID.GetTDBID(itemID);
    // Lido em LOCAL antes de comparar: a comparação INLINE do retorno de um native dá resultado incoerente neste compilador.
    let n: Int32 = BwmsWardrobeForgetByTDBID(this, TDBID.ToNumber(tdbid));
    return n > 0;
}

// Sugar de conveniência: a via de scan não precisa de um ItemID completo, só do TweakDBID — útil
// pra quem já tem o TDBID e quer evitar montar um ItemID sintético só pra chamar ForgetItemID.
@addMethod(WardrobeSystem)
public func BwmsForgetByTDBID(tdbid: TweakDBID) -> Int32 {
    return BwmsWardrobeForgetByTDBID(this, TDBID.ToNumber(tdbid));
}
