// Idioma dos textos em uso (configuração /language, OnScreen), lido pelo BWMS pra aplicar os textos de mod do idioma certo.
@addMethod(PlayerPuppet)
public func BwmsIdiomaDosTextos() -> CName {
  let jogo: GameInstance = this.GetGame();
  let config: ref<UserSettings> = GameInstance.GetSettingsSystem(jogo);
  if !IsDefined(config) {
    return n"en-us";
  }
  let bruto: ref<ConfigVar> = config.GetVar(n"/language", n"OnScreen");
  let variavel: ref<ConfigVarListName> = bruto as ConfigVarListName;
  if !IsDefined(variavel) {
    return n"en-us";
  }
  let valor: CName = variavel.GetValue();
  return valor;
}
