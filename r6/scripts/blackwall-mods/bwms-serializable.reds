// bwms-serializable.reds — objeto do motor que não deriva de `IScriptable` entregue ao script pelo BWMS.
//
// Recursos (`CResource`) e nós de grafo de quest derivam só de `ISerializable`. No Mac o `as`, o `IsA` e os métodos chamados num `ref` leem o tipo no
// `+0x30` do objeto, e nesses objetos esse campo é outra coisa (num recurso, o hash do caminho): um `as X` direto derruba o jogo. Por isso toda API do
// BWMS que devolve um objeto assim entrega um `BwmsSerializable`, com o objeto guardado num campo privado; faça o `as BwmsSerializable` e use os métodos
// abaixo. Passado de volta a uma native do BWMS, ele é desembrulhado sozinho. Objetos que já derivam de `IScriptable` chegam como são.

public class BwmsSerializable extends IScriptable {
    private let object: ref<IScriptable>;

    // Nome da classe do objeto no motor, como `entEntityTemplate` ou `questPhaseNodeDefinition`.
    public func GetObjectClassName() -> CName {
        let nome: CName = BwmsSerializableClassName(this);
        return nome;
    }

    // `true` se o objeto é da classe `className` ou de uma filha dela.
    public func IsObjectA(className: CName) -> Bool {
        let ok: Bool = BwmsSerializableIsA(this, className);
        return ok;
    }

    // Hash do caminho, se o objeto é um recurso (0 fora de recurso ou em recurso sem caminho).
    public func GetResourcePathHash() -> Uint64 {
        let hash: Uint64 = BwmsSerializableResourceHash(this);
        return hash;
    }
}

public native func BwmsSerializableClassName(obj: ref<IScriptable>) -> CName

public native func BwmsSerializableIsA(obj: ref<IScriptable>, className: CName) -> Bool

public native func BwmsSerializableResourceHash(obj: ref<IScriptable>) -> Uint64
