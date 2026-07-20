// BWMS — Codeware Facade (redscript puro, RECRIADO 2026-07-12).
// Fonte original perdida no incidente de "perda da camada DEV" (2026-07-11) — este arquivo
// nunca tinha sido comitado em blackwall-mods-dev/, só existia na cópia do jogo que foi
// substituída. Recriado a partir da prova salva (cp77-symbols/notes/proofs/
// 2026-06-25-codeware-facade-redscript.log) + da API original do Codeware Windows
// (enablers/Codeware/scripts/Facade.reds: Require(version)->Bool, Version()->String).
//
// Por que "class" simples e não "abstract native class": a versão Windows é native de
// verdade (C++/RED4ext faz RegisterType). No Mac, register_codeware_facade (register.rs)
// NÃO registra a CLASSE (isso exigiria RegisterType, represa separada — ver
// cp77-registertype-cclass-forge) — ele só faz register_method NUM CLASSE JÁ EXISTENTE
// chamada "Codeware". Uma "class" comum (script-defined) é criada pelo próprio compilador/
// loader do jogo ao carregar o bundle — sem precisar de RegisterType nenhum. Os 2 métodos
// ficam "native" (corpo vazio) só pra reservar o slot que o register_method preenche depois.
// Isso é EXATAMENTE o padrão já provado em 2026-06-25 (boot chegou ao gameplay, classe
// apareceu no RTTI, sem crash de bind) — recriação, não experimento novo.
//
// Mods que checam `Codeware.Require("1.x.x")` no OnAttach/init e abortam se falhar passam
// a não abortar mais (mesmo sem versionamento semver real — ver cw-version-semver, F4).
public abstract native class Codeware {
  public static native func Require(version: String) -> Bool;
  public static native func Version() -> String;
}
