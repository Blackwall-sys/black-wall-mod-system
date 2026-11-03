// BWMS — settings persistentes NAMESPACED por mod de 3os (2026-08-05, achado de auditoria).
//
// `BwmsConfigGet/Set(key, value)` (redscript-mod-persistence, FECHADO 2026-07-13) já é um
// arquivo externo persistente (`~/.bwms-modconfig.txt`, fora do save) — mas é um namespace FLAT
// GLOBAL: só o próprio God Mode do BWMS usa. O CET real dá a CADA mod Lua seu próprio settings
// store (`GetMod():GetSettings()`). Aqui: 100% redscript, ZERO native nova — só prefixa a key
// com o id do mod antes de chamar o mesmo native já provado. Um mod de 3os chama
// `BwmsModConfigSet("meu-mod", "volume", "0.8")` e nunca colide com outro mod ou com o BWMS.
// (BwmsConfigGet/Set já declaradas em bwms-settings-poc.reds, mesmo escopo global — sem module
// aqui, então visíveis sem import, igual ToString/Print/etc.)

public static func BwmsModConfigGet(modId: String, key: String) -> String {
  return BwmsConfigGet("mod." + modId + "." + key);
}

public static func BwmsModConfigSet(modId: String, key: String, value: String) -> Bool {
  return BwmsConfigSet("mod." + modId + "." + key, value);
}
