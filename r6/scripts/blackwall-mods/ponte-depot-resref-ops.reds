// ponte-depot-resref-ops.reds — Codeware `Depot/ResourceReference.reds`, a parte de operadores e hash sobre o `ResRef` vanilla
// (orphans.script:23714).
//
// `OperatorEqual`/`OperatorNotEqual(ResRef, ResRef)` habilitam `==`/`!=` pro `ResRef` sobre os builtins `Equals`/`NotEquals`.
// `ResRefGetHash(self)` é o `GetHash()` do Codeware (`return aReference.resource.path;`): a mesma leitura de `BwmsResRefToHash`. É
// função GLOBAL com nome do BWMS, não `@addMethod(ResRef)`: dentro de `@addMethod` num struct o `this` resolve pra `ref<ResRef>` e a
// chamada não casa (`NO_MATCHING_OVERLOAD`). O texto do caminho (`ResRefToString`) e as classes `ResourceRef`/`ResourceAsyncRef` do
// BWMS estão em `ponte-resourceref.reds`.
public func OperatorEqual(lhs: ResRef, rhs: ResRef) -> Bool {
    return Equals(lhs, rhs);
}

public func OperatorNotEqual(lhs: ResRef, rhs: ResRef) -> Bool {
    return NotEquals(lhs, rhs);
}

public func ResRefGetHash(self: ResRef) -> Uint64 {
    return BwmsResRefToHash(self);
}
