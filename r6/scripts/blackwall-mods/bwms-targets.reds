// bwms-targets.reds — nativos auxiliares dos alvos do `CallbackSystem` e helpers de `ResRef`, declarados pra
// que um mod os alcance: native registrada no Rust sem declaração `.reds` existe no RTTI e ninguém chama.
//
// `native func` fica em escopo GLOBAL de propósito: dentro de `module X.Y` o bind RTTI crasha (o `fullName` precisa ser bare).

// ── Helpers de `ResRef` (usados pelo ResourceTarget e pelo depot) ────────────────────────────
native func BwmsResRefFromPath(path: String) -> Uint64;
native func BwmsResRefIsEmpty(hash: Uint64) -> Bool;
native func BwmsResRefIsLoaded(hash: Uint64) -> Bool;
native func BwmsResRefResourceClass(hash: Uint64) -> String;
