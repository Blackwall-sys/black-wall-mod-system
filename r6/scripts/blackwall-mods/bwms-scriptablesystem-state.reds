// bwms-scriptablesystem-state.reds — Codeware `#188` (`ScriptableServiceContainerState`, `SaveState`/`LoadState`).
//
// No Codeware (`App/Scripting/ScriptableServiceContainer.hpp`), `SaveState`/`LoadState` são métodos PRIVADOS do contêiner: serializam
// todos os serviços registrados de uma vez num arquivo próprio, fora do save, no boot e no shutdown — infraestrutura interna, não API
// de mod. O efeito prático (estado de mod que sobrevive a reiniciar o jogo, fora do save) no BWMS é o `bwms-config-por-mod.reds`
// (`BwmsModConfigGet`/`BwmsModConfigSet`, separado por mod, no arquivo externo `~/.bwms-modconfig.txt`), pra qualquer classe.
//
// Aqui, 2 métodos em `ScriptableSystem` (a base vanilla dos serviços: o `ScriptableService` não se auto-descobre no motor deste build,
// o `ScriptableSystem` sim) que separam a chave pelo `GetClassName()` do sistema: `this.BwmsSaveState("volume", "0.8")` /
// `this.BwmsLoadState("volume")`. Prefixo `Bwms*` de propósito: é capacidade do BWMS, não método do Codeware.

@addMethod(ScriptableSystem)
public func BwmsSaveState(key: String, value: String) -> Bool {
    return BwmsModConfigSet(NameToString(this.GetClassName()), key, value);
}

@addMethod(ScriptableSystem)
public func BwmsLoadState(key: String) -> String {
    return BwmsModConfigGet(NameToString(this.GetClassName()), key);
}
