// A superfície de API de `ResourceAsyncRef` sobre `ResRef`, por composição.
//
// O QUE O ITEM PEDE. `ResourceAsyncRef` (a versão assíncrona) expõe 4 métodos:
// `SetPath`/`GetPath`/`GetHash`/`IsEmpty`. No BWMS o tipo vivo correspondente é o `ResRef`
// VANILLA, e as primitivas já existem e já foram provadas ao vivo — o que faltava era a
// superfície: um mod não alcança uma native sem declaração, e chamar `BwmsResRefIsEmpty(hash)`
// não é a mesma coisa que ter a API do item.
//
// NOMES PRÓPRIOS, DE PROPÓSITO. Pela regra de nome do projeto, API que não é do jogo
// leva nome do BWMS — só nome VANILLA da CDPR precisa bater exato.
// Por isso `ResRefIsEmpty` e não `ResourceAsyncRef.IsEmpty`.
//
// `GetPath` devolveria texto, mas `ResourceAsyncReference` guarda SÓ o `ResourcePath` (u64); o nome que diz o que sai é
// `ResRefGetPathHash`. O texto do caminho, quando ele passou como texto pelo BWMS, sai do `ResRefToString` (`ponte-resourceref.reds`).

native func BwmsResRefToHash(value: ResRef) -> Uint64;
native func BwmsResRefIsEmpty(hash: Uint64) -> Bool;
native func BwmsResRefIsLoaded(hash: Uint64) -> Bool;
native func BwmsResRefResourceClass(hash: Uint64) -> String;
native func BwmsResRefFromPath(path: String) -> Uint64;

// --- A superfície -----------------------------------------------------------------------------

// Hash do path guardado na referência. Mesma leitura que `ResRefGetHash` (item já provado);
// existe aqui com o nome da família pra a API ficar coerente.
public func ResRefGetPathHash(self: ResRef) -> Uint64 {
    return BwmsResRefToHash(self);
}

// `IsEmpty` — referência sem path. Lê em LOCAL antes de comparar: comparar retorno de native
// INLINE é a forma quebrada que o gate `[4/5]` da auditoria reprova.
public func ResRefIsEmpty(self: ResRef) -> Bool {
    let h: Uint64 = BwmsResRefToHash(self);
    return BwmsResRefIsEmpty(h);
}

// O recurso apontado já está carregado no depot? (`#18`, provado 2026-09-05.)
public func ResRefIsLoaded(self: ResRef) -> Bool {
    let h: Uint64 = BwmsResRefToHash(self);
    return BwmsResRefIsLoaded(h);
}

// Classe do recurso apontado, se carregado. `""` quando não está.
public func ResRefResourceClass(self: ResRef) -> String {
    let h: Uint64 = BwmsResRefToHash(self);
    return BwmsResRefResourceClass(h);
}

// `SetPath` na forma que o BWMS consegue oferecer: devolve o HASH de um path textual. Não
// constrói um valor `ResRef` (o motor não expõe construtor script-visível), então quem precisa
// do valor guarda o hash — que é literalmente o conteúdo do struct.
public func ResRefHashFromPath(path: String) -> Uint64 {
    return BwmsResRefFromPath(path);
}



