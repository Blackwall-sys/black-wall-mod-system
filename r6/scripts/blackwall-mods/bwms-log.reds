// Log de mod: `ModLog(mod, texto)` e `Print(texto)` gravam em `r6/logs/bwms-mods.log` quando esse arquivo existe (criar o arquivo liga,
// apagar desliga). Sem ele, nada é gravado.
native func ModLog(mod: CName, text: String) -> Void;
native func Print(text: String) -> Void;
