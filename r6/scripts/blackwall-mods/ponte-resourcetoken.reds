// ponte-resourcetoken.reds — os 8 métodos do `ResourceToken`.
//
// O redscript nunca vê um `ResourceReference<>` por valor,
// então o wrapper é redscript puro: guarda (objeto dono, nome da property) e delega pras natives de `bwms-resource-reference.reds`,
// ou guarda o id de um token que o BWMS guarda do lado nativo. Sem forja de classe nativa.

public class ResourceToken extends IScriptable {
    // `ref` FORTE, não `wref`: referência fraca sem ninguém segurando um `ref` forte é a classe de bug que deixava `OnInitialize()`
    // sem disparar nos itens `#97`/`#106`. O dono é um objeto do MOTOR de vida longa; segurar `ref` forte é seguro e não cria ciclo
    // (o `ResourceToken` é descartável, criado por `FromReference`).
    private let m_owner: ref<IScriptable>;
    private let m_propName: CName;
    // Token guardado pelo BWMS, sem property dona: `BwmsDepotLoadResource` (#198) e `IComponent.LoadResourceToken` (#168).
    private let m_id: Uint64;

    // Nome próprio: um `ResourceReference<>` por valor não tem equivalente redscript
    // direto. `FromReference(owner, propName)` é a via PRÁTICA real: aponta pro MESMO objeto+
    // property já usados por `BwmsGetResourceReferencePath`.
    public static func FromReference(owner: ref<IScriptable>, propName: CName) -> ref<ResourceToken> {
        let self: ref<ResourceToken> = new ResourceToken();
        self.m_owner = owner;
        self.m_propName = propName;
        return self;
    }

    // Token por id — nome do BWMS.
    public static func FromId(id: Uint64) -> ref<ResourceToken> {
        let self: ref<ResourceToken> = new ResourceToken();
        self.m_id = id;
        return self;
    }

    // Tipo: não `ref<CResource>` — `CResource`
    // NÃO existe como tipo redscript-declarável nesta RTTI (grep em
    // `redscript-src` inteiro: zero hits, mesma ausência já documentada). `BwmsGetResourceReferenceResource`
    // já devolve `ref<IScriptable>` — mantido aqui sem cast pra tipo inexistente.
    public func GetResource() -> ref<IScriptable> {
        let id: Uint64 = this.m_id;
        if id != 0ul {
            let r: ref<IScriptable> = BwmsResourceTokenIdResource(id);
            return r;
        }
        return BwmsGetResourceReferenceResource(this.m_owner, this.m_propName);
    }

    public func GetPath() -> ResRef {
        return BwmsHashToResRef(this.GetHash());
    }

    public func GetHash() -> Uint64 {
        let id: Uint64 = this.m_id;
        if id != 0ul {
            let h: Uint64 = BwmsResourceTokenIdHash(id);
            return h;
        }
        return BwmsGetResourceReferencePath(this.m_owner, this.m_propName);
    }

    // 0 carregando, 1 carregado, 2 falhou (-1 = id inválido). Lido em local, nunca comparado inline.
    private func Status() -> Int32 {
        let id: Uint64 = this.m_id;
        if id != 0ul {
            let si: Int32 = BwmsResourceTokenIdStatus(id);
            return si;
        }
        let sp: Int32 = BwmsGetResourceReferenceTokenStatus(this.m_owner, this.m_propName);
        return sp;
    }

    // Status lido em LOCAL antes de comparar: comparar o retorno de native INLINE (`Bwms...(...) != 0`) dá resultado incoerente neste
    // compilador — as 3 comparações saíam verdadeiras e falsas ao mesmo tempo pro mesmo `Int32`.
    public func IsFinished() -> Bool {
        let s: Int32 = this.Status();
        return s == 1 || s == 2;
    }

    public func IsLoaded() -> Bool {
        let s: Int32 = this.Status();
        return s == 1;
    }

    public func IsFailed() -> Bool {
        let s: Int32 = this.Status();
        return s == 2;
    }

    // `GetJobHandle` (2026-09-11): o `JobHandle` do token como `Uint64` opaco — o tipo `JobHandle` não existe no RTTI do jogo.
    // Serve pra identidade e pra `WaitForJob`.
    public func GetJobHandle() -> Uint64 {
        let id: Uint64 = this.m_id;
        if id != 0ul {
            let ji: Uint64 = BwmsResourceTokenIdJob(id);
            return ji;
        }
        let jp: Uint64 = BwmsGetResourceReferenceTokenJob(this.m_owner, this.m_propName);
        return jp;
    }

    // Nome do BWMS: espera o job do token numa fila de jobs.
    public func WaitForJob() -> Bool {
        let id: Uint64 = this.m_id;
        if id != 0ul {
            let wi: Bool = BwmsResourceTokenIdWait(id);
            return wi;
        }
        let wp: Bool = BwmsResourceReferenceTokenWait(this.m_owner, this.m_propName);
        return wp;
    }

    // `RegisterCallback(target, function)` (2026-09-11): chama `target.function(token)` quando o token carrega. Em vez do
    // `ResourceToken::OnLoaded` do motor, o BWMS confere o token a cada tick de gameplay (o poll pelo `BwmsDelay` nunca rodou no
    // boot 22). O alvo é guardado como referência fraca.
    public func RegisterCallback(target: ref<IScriptable>, function: CName) -> Void {
        if !IsDefined(target) {
            return;
        }
        BwmsResourceTokenWatch(this, target, function, this.m_id, this.m_owner, this.m_propName);
    }
}

native func BwmsGetResourceReferenceTokenStatus(owner: ref<IScriptable>, propName: CName) -> Int32
native func BwmsGetResourceReferenceTokenJob(owner: ref<IScriptable>, propName: CName) -> Uint64
native func BwmsResourceReferenceTokenWait(owner: ref<IScriptable>, propName: CName) -> Bool
native func BwmsResourceTokenIdStatus(id: Uint64) -> Int32
native func BwmsResourceTokenIdHash(id: Uint64) -> Uint64
native func BwmsResourceTokenIdResource(id: Uint64) -> ref<IScriptable>
native func BwmsResourceTokenIdJob(id: Uint64) -> Uint64
native func BwmsResourceTokenIdWait(id: Uint64) -> Bool
native func BwmsResourceTokenWatch(token: ref<IScriptable>, target: ref<IScriptable>, function: CName, id: Uint64, owner: ref<IScriptable>, propName: CName) -> Bool
