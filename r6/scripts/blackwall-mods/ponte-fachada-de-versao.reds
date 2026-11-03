// BWMS — a fachada `Codeware` (`Codeware.Require(version)`/`Codeware.Version()`). A classe é
// nativa sem `extends` explícito (pai `IScriptable`); o Rust preenche os 2 métodos no validador do motor (`registrar_fachada_de_versao`,
// tipos_do_motor/mod.rs). `Require` compara a versão pedida (major.minor.patch) com a que `Version()` devolve e devolve
// `false` quando o mod pede mais do que isso.
public abstract native class Codeware {
  public static native func Require(version: String) -> Bool;
  public static native func Version() -> String;
}
