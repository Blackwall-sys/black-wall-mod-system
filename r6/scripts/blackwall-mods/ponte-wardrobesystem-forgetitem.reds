// ponte-wardrobesystem-forgetitem.reds —
// `@addMethod(WardrobeSystem) public native func ForgetItemID(itemID: ItemID) -> Bool`.
//
// Uma via só: varre o `HashMap<CName,ItemID>` comparando `tdbid` e remove TODAS as chaves que baterem, em vez de derivar
// `appearanceName` do flat `.appearanceName` e remover por essa chave. O SCAN por `tdbid` não depende de o item ter o flat `.appearanceName` no TweakDB, e usa o layout do
// `HashMap` conferido ao vivo contra um save real (`wardrobe_forget_by_tdbid`, roupas_e_armas/mod.rs; `ItemID.tdbid@node+0x10`,
// `ItemID` de 0x10 bytes).
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
