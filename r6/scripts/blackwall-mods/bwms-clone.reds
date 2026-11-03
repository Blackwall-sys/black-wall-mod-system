// bwms-clone.reds — Codeware `#57` (`IScriptable.Clone()`): cópia de um objeto de script com a mesma classe e os mesmos valores.
// A cópia sai do `CreateInstance` da classe e cada propriedade passa pelo `Assign` do tipo dela. Handle forte vira cópia funda (um objeto novo por
// objeto de origem, então duas referências ao mesmo filho continuam apontando pro mesmo filho na cópia); handle fraco que aponta pra algo copiado passa a
// apontar pra cópia. Objeto nulo ou classe abstrata devolvem nulo.

native func BwmsCloneObject(obj: ref<IScriptable>) -> ref<IScriptable>;

@addMethod(IScriptable)
public func BwmsClone() -> ref<IScriptable> {
    let copia: ref<IScriptable> = BwmsCloneObject(this);
    return copia;
}
