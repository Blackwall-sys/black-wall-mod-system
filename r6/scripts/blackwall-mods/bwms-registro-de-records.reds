// Genealogia e nome dos records que os mods criam (o TweakDB do jogo só guarda o hash): se o record é original (não foi criado por mod), o
// primeiro ancestral original de um clone, os clones de um record em qualquer geração (por contagem e índice) e o nome legível de um id.
native func BwmsTweakIsOriginalRecord(id: TweakDBID) -> Bool;
native func BwmsTweakGetOriginalParent(id: TweakDBID) -> TweakDBID;
native func BwmsTweakGetDescendantCount(id: TweakDBID) -> Int32;
native func BwmsTweakGetDescendant(id: TweakDBID, index: Int32) -> TweakDBID;
native func BwmsTweakGetName(id: TweakDBID) -> String;
native func BwmsTweakRegisterName(name: String) -> TweakDBID;
native func BwmsTweakRemoveRecord(id: TweakDBID) -> Bool;
