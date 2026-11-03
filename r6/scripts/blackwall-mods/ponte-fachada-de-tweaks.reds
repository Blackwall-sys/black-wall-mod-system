// BWMS — a fachada `TweakXL` (`TweakXL.Require(version)`/`TweakXL.Version()`/`TweakXL.Reload()`; fonte em
// `enablers/TweakXL/src/App/Facade.hpp`). Classe nativa sem `extends` explícito, como a `Codeware` (`ponte-fachada-de-versao.reds`): o Rust
// (`registrar_fachada_de_tweaks`, tweakdb/natives.rs) preenche os 3 métodos no validador do motor (`class_validate_probe_hook`,
// inicio/boot.rs).
//
// `Reload()` é o gatilho MANUAL do TweakXL (`Facade::Reload()`): reaplica os `.yaml` já carregados sem reiniciar o jogo. Não é
// recarga automática ao salvar o arquivo.
public abstract native class TweakXL {
  public static native func Require(version: String) -> Bool;
  public static native func Version() -> String;
  public static native func Reload() -> Void;
}
