// ponte-resourceref.reds — `ResourceRef`/`ResourceAsyncRef` com nomes BWMS, sobre `ResRef` (2026-09-12).
//
// A referência forja duas structs nativas (`ResourceRef` = caminho + token, `ResourceAsyncRef` = só caminho). O BWMS não forja struct: aqui são
// classes de script em cima do que já está provado — hash do `ResRef` (`BwmsResRefToHash`), carga por token-id do `#18`
// (`BwmsDepotLoadResourceToken`, status 0 carregando / 1 carregado / 2 falhou, recurso pelo token) e duas natives novas: hash → `ResRef` e o texto do
// caminho registrado. O `ResourcePath` guarda só o hash; `ResRefToString` devolve o texto quando o caminho passou como texto por
// `ResRefHashFromPath`/`BwmsResRefFromPath` (o limite de um `ResourcePathRegistry`) e vazio quando não passou.
// Divergências: `GetToken` devolve o token-id (a API de token do BWMS é por id) e `ToVariant`/`FromVariant` carregam o `ResRef`.
// `BwmsHashToResRef` é declarado em ponte-casts.reds.
native func BwmsResourcePathPart(hash: Uint64, part: Int32) -> String

public func ResRefFromHash(hash: Uint64) -> ResRef {
    let r: ResRef = BwmsHashToResRef(hash);
    return r;
}

public func ResRefToString(self: ResRef) -> String {
    let h: Uint64 = BwmsResRefToHash(self);
    let texto: String = "";
    let parte: Int32 = 0;
    while parte < 16 {
        let pedaco: String = BwmsResourcePathPart(h, parte);
        if StrLen(pedaco) == 0 {
            return texto;
        };
        texto = texto + pedaco;
        parte += 1;
    };
    return texto;
}

public class BwmsResourceRef {
    private let m_hash: Uint64;
    private let m_token: Uint64;

    public static func FromPath(path: ResRef) -> ref<BwmsResourceRef> {
        let r: ref<BwmsResourceRef> = new BwmsResourceRef();
        r.LoadPath(path);
        return r;
    }

    public static func FromVariant(value: Variant) -> ref<BwmsResourceRef> {
        let path: ResRef = FromVariant<ResRef>(value);
        return BwmsResourceRef.FromPath(path);
    }

    public func LoadPath(path: ResRef) -> Void {
        this.m_hash = BwmsResRefToHash(path);
        this.m_token = 0ul;
        if this.m_hash != 0ul {
            this.m_token = BwmsDepotLoadResourceToken(this.m_hash);
        };
    }

    public func GetPath() -> ResRef {
        let r: ResRef = BwmsHashToResRef(this.m_hash);
        return r;
    }

    public func GetHash() -> Uint64 {
        return this.m_hash;
    }

    public func GetToken() -> Uint64 {
        return this.m_token;
    }

    public func Wait() -> Bool {
        if this.m_token == 0ul {
            return false;
        };
        let terminou: Bool = BwmsResourceTokenIdWait(this.m_token);
        return terminou;
    }

    private func Status() -> Int32 {
        if this.m_token == 0ul {
            return -1;
        };
        let st: Int32 = BwmsResourceTokenIdStatus(this.m_token);
        return st;
    }

    public func GetResource() -> ref<IScriptable> {
        if this.Status() != 1 {
            return null;
        };
        let r: ref<IScriptable> = BwmsResourceTokenIdResource(this.m_token);
        return r;
    }

    public func IsEmpty() -> Bool {
        return this.m_hash == 0ul;
    }

    public func IsLoaded() -> Bool {
        return this.Status() == 1;
    }

    public func IsFailed() -> Bool {
        return this.Status() == 2;
    }

    public func ToVariant() -> Variant {
        let path: ResRef = this.GetPath();
        return ToVariant(path);
    }
}

public class BwmsResourceAsyncRef {
    private let m_hash: Uint64;

    public func SetPath(path: ResRef) -> Void {
        this.m_hash = BwmsResRefToHash(path);
    }

    public func GetPath() -> ResRef {
        let r: ResRef = BwmsHashToResRef(this.m_hash);
        return r;
    }

    public func GetHash() -> Uint64 {
        return this.m_hash;
    }

    public func IsEmpty() -> Bool {
        return this.m_hash == 0ul;
    }
}

public func OperatorAssignMultiply(out res: ref<BwmsResourceRef>, path: ResRef) -> Void {
    if !IsDefined(res) {
        res = new BwmsResourceRef();
    };
    res.LoadPath(path);
}

public func OperatorAssignMultiply(out res: ref<BwmsResourceAsyncRef>, path: ResRef) -> Void {
    if !IsDefined(res) {
        res = new BwmsResourceAsyncRef();
    };
    res.SetPath(path);
}
