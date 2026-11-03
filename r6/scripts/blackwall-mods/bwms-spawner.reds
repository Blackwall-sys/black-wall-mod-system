// bwms-spawner.reds — Codeware `#80` (`PopulationSpawnerWrapper`): leitura de um spawner de população, por ponteiro.
// `ptr` é o `Uint64` devolvido por `BwmsCommunityGetSpawner(sys, id)` (0 = não achado). `BwmsSpawnerRecordId(ptr, true)` lê o TweakDBID no record
// do handle `@60`, pra conferir com o `recordID@40`. `BwmsSpawnerIdsCount(ptr, 0|1|2)` = restored/spawned/reserved (-1 inválido ou size > cap).
native func BwmsSpawnerIsActive(ptr: Uint64) -> Bool
native func BwmsSpawnerIsInitialized(ptr: Uint64) -> Bool
native func BwmsSpawnerRecordId(ptr: Uint64, fromRecord: Bool) -> Uint64
native func BwmsSpawnerAppearanceName(ptr: Uint64) -> CName
native func BwmsSpawnerIdsCount(ptr: Uint64, which: Int32) -> Int32
