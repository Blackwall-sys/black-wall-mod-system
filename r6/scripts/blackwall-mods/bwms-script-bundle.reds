// bwms-script-bundle.reds — Codeware `#251` (`ScriptBundle`): o BWMS lê o bundle de redscript no validador do motor, durante a carga dos scripts, e guarda
// quantas definições de cada tipo vieram e de onde veio cada função (arquivo e linha). Dono vazio (`n""`) = função global.

native func BwmsScriptSourceFileMatches(owner: CName, name: CName, suffix: String) -> Bool;
