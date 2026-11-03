// ponte-casts.reds — conversões por hash e os utilitários de hash da mesma família.
// `Cast()` é açúcar sobre identidade de 8 bytes: CName/CRUID/NodeRef/EntityID/TweakDBID são todos um hash/u64.
// Aqui cada direção tem nome próprio e explícito: o `scc` resolve sobrecarga só por PARÂMETRO, não por tipo de retorno, e um
// `Cast(value: Uint64) -> X` genérico depende disso (`let cruid: CRUID = BwmsCast(h)` escolheria o 1º overload declarado).
native func BwmsHashToName(hash: Uint64) -> CName
native func BwmsNameToHash(value: CName) -> Uint64
native func BwmsHashToCRUID(hash: Uint64) -> CRUID
native func BwmsCRUIDToHash(value: CRUID) -> Uint64
native func BwmsHashToNodeRef(hash: Uint64) -> NodeRef
native func BwmsNodeRefToHash(value: NodeRef) -> Uint64
// `CreateNodeRef`: o `NodeRef` é só o hash do caminho (FNV-1a64 com as regras de `#` e `;alias` do SDK, `bwms_primitives::node_ref_hash`;
// `tramp_string_to_noderef`, mundo/mod.rs), sem endereço nativo.
native func BwmsCreateNodeRef(path: String) -> NodeRef
native func BwmsEntityIDToHash(value: EntityID) -> Uint64
native func BwmsHashToEntityID(hash: Uint64) -> EntityID

public func BwmsNodeRefToEntityID(value: NodeRef) -> EntityID = BwmsHashToEntityID(BwmsNodeRefToHash(value))
public func BwmsEntityIDToNodeRef(value: EntityID) -> NodeRef = BwmsHashToNodeRef(BwmsEntityIDToHash(value))

// TDBID.ToNumber já é NATIVA VANILLA do jogo (usada em dezenas de .script reais) — zero código
// novo.
public func BwmsTweakDBIDToUint64(value: TweakDBID) -> Uint64 = TDBID.ToNumber(value)
// `Uint64->TweakDBID`: a mesma identidade de 8 bytes dos outros pares.
native func BwmsHashToTweakDBID(hash: Uint64) -> TweakDBID
// `ResRef` é um `ResourcePath` (hash) só; `GetPath()` de um `ResourceAsyncRef` é identidade pura. Nome próprio hash<->ResRef
// (o `ResourceAsyncRef` não é modelado no BWMS).
native func BwmsHashToResRef(hash: Uint64) -> ResRef
native func BwmsResRefToHash(value: ResRef) -> Uint64

// `PersistentID.ToHash` (item #31) — algoritmo de `PersistentID::GetHash()`, ver a nota do `PersistentID` mais abaixo.
native func BwmsPersistentIDHash(entityHash: Uint64, componentHash: Uint64) -> Uint64

// `@addMethod(TDBID) FromNumber`: o nome que os mods chamam, sobre `BwmsHashToTweakDBID`.
@addMethod(TDBID)
public static func FromNumber(number: Uint64) -> TweakDBID = BwmsHashToTweakDBID(number)

// `ToEntityID`: vai direto no destino, sem passar por `EntityID.FromHash` (que não é vanilla). Sobrecarga por PARÂMETRO (Uint64 vs NodeRef), que o `scc` resolve.
public func ToEntityID(value: Uint64) -> EntityID = BwmsHashToEntityID(value)
public func ToEntityID(value: NodeRef) -> EntityID = BwmsNodeRefToEntityID(value)

// `EntityID.FromHash`/`.ToHash`. Os nomes de MÉTODO são os que os mods chamam (zero colisão com o
// corpus vanilla). Assinatura: `EntityID` por valor, não `script_ref<EntityID>`; do lado do mod a chamada é igual,
// `EntityID.ToHash(id)`.
@addMethod(EntityID)
public static func FromHash(hash: Uint64) -> EntityID = BwmsHashToEntityID(hash)
@addMethod(EntityID)
public static func ToHash(id: EntityID) -> Uint64 = BwmsEntityIDToHash(id)

// `PersistentID` — `ForEntity`/`ForComponent`/`ToHash`.
// `ForEntity` é o `Cast(EntityID)->PersistentID` vanilla (`orphans.script:16770`): devolve um `PersistentID` do próprio motor.
// `ForComponent` recebe o componente (`ref<IComponent>`), não `(id: EntityID, component: CName)`, que dependeria de um
// `HashMap` interno: o `PersistentID` sai do `GameComponent.GetPersistentID()` vanilla, calculado pelo motor. Componente que não é
// `GameComponent` (a maioria dos `entMeshComponent`) volta com `PersistentID` indefinido — o motor não tem um pra ele.
// `ToHash` segue o `PersistentID::GetHash()` do header gerado da reflection (`gamePersistentID.hpp`, MurmurHash64A-like,
// `magic=0xC6A4A7935BD1E995`; `persistent_id_hash`, salvamento/mod.rs): com `GetComponentName` = `None` (hash 0), o caso entidade,
// devolve o hash da entidade puro; com componente, mistura os dois.
@addMethod(PersistentID)
public static func ForEntity(id: EntityID) -> PersistentID = Cast(id)
@addMethod(PersistentID)
public static func ToHash(id: PersistentID) -> Uint64 {
    let entityHash: Uint64 = BwmsEntityIDToHash(PersistentID.ExtractEntityID(id));
    let compHash: Uint64 = BwmsNameToHash(PersistentID.GetComponentName(id));
    return BwmsPersistentIDHash(entityHash, compHash);
}
@addMethod(PersistentID)
public static func ForComponent(component: ref<IComponent>) -> PersistentID {
    let gc: ref<GameComponent> = component as GameComponent;
    if IsDefined(gc) {
        return gc.GetPersistentID();
    }
    let undef: PersistentID;
    return undef;
}

// `String↔LocalizationString` (parte do #63) mora em `ponte-localizationstring.reds`.

// CName/CRUID/NodeRef — os nomes GLOBAIS que os mods chamam (funções soltas, não
// `@addMethod`), sem colisão com o corpus vanilla. `CreateNodeRef(path: String)` recebe `String` por valor, não
// `script_ref<String>`.
public func HashToName(value: Uint64) -> CName = BwmsHashToName(value)
public func NameToHash(value: CName) -> Uint64 = BwmsNameToHash(value)
public func ToName(value: Uint64) -> CName = BwmsHashToName(value)
public func ToName(value: String) -> CName = StringToName(value)

public func HashToCRUID(value: Uint64) -> CRUID = BwmsHashToCRUID(value)
public func CRUIDToHash(value: CRUID) -> Uint64 = BwmsCRUIDToHash(value)
public func CreateCRUID(value: Uint64) -> CRUID = BwmsHashToCRUID(value)
public func ToCRUID(value: Uint64) -> CRUID = BwmsHashToCRUID(value)

public func HashToNodeRef(value: Uint64) -> NodeRef = BwmsHashToNodeRef(value)
public func NodeRefToHash(value: NodeRef) -> Uint64 = BwmsNodeRefToHash(value)
public func ToNodeRef(value: Uint64) -> NodeRef = BwmsHashToNodeRef(value)
public func ToNodeRef(value: EntityID) -> NodeRef = BwmsEntityIDToNodeRef(value)
// A 3ª sobrecarga de `ToNodeRef` (Uint64/EntityID/String) e o `CreateNodeRef`, sobre `BwmsCreateNodeRef`.
public func CreateNodeRef(path: String) -> NodeRef = BwmsCreateNodeRef(path)
public func ToNodeRef(value: String) -> NodeRef = BwmsCreateNodeRef(value)

// `GameInstance.GetSystemRequestsHandler()`: o handler de pedidos do sistema, pelo mesmo caminho que os menus do
// jogo usam (`GetSystemRequestsHandler()` de um `inkMenuScenario`), como função global do BWMS.
public func BwmsGetSystemRequestsHandler() -> wref<inkISystemRequestsHandler> = new inkMenuScenario().GetSystemRequestsHandler()
