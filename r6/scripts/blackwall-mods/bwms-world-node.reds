// bwms-world-node.reds — com nome BWMS: instância de nó do mundo e `worldInstancedMeshNode` de setor, só leitura.
//
// `worldINodeInstance` (6 getters) e `worldInstancedMeshNode` (9 campos + 3 arrays) não são declarados como tipos nativos
// (categoria CMesh: o corpus vanilla não declara nenhum deles) e os getters são compostos sobre leitores tipados, como no `bwms-world-node-setup.reds`.
// - `BwmsWorldNodeInstance.FromID(id)`: `id` é o hash do GlobalNodeRef (ex.: `BwmsEntityIDToHash(Cast<EntityID>(ResolveNodeRef(ref, raiz)))`);
//   `FromSectorSetup(setor, i)` usa o GlobalNodeID do setup `i` de um setor no cache. Cada getter acha a instância de novo no registro do mundo, então um nó
//   que saiu do streaming devolve NaN/0 em vez de lixo. `GetNode` vira `GetNodeClass` (o `worldNode` não é declarável) e o GlobalNodeID sai como Uint64.
// - `BwmsInstancedMeshNode.FromSector(setor, i)`: nó `i` da lista `nodes` de um setor no cache do depot; os campos saem pelo offset do registro RTTI desta
//   build. Os 3 arrays de runtime (bounding boxes, distâncias de streaming e lotes de instância) saem por tamanho e por elemento.
native func BwmsNodeInstanceF32(id: Uint64, target: Int32, offset: Int32) -> Float
native func BwmsNodeInstanceU64(id: Uint64, target: Int32, offset: Int32) -> Uint64
native func BwmsNodeInstanceNodeClass(id: Uint64) -> CName
native func BwmsSectorNodeField(path: String, index: Int32, name: String, inner: Int32, width: Int32) -> Uint64
native func BwmsSectorNodeArraySize(path: String, index: Int32, offset: Int32) -> Int32
native func BwmsSectorNodeArrayF32(path: String, index: Int32, offset: Int32, element: Int32, stride: Int32, inner: Int32) -> Float
native func BwmsSectorNodeArrayU32(path: String, index: Int32, offset: Int32, element: Int32, stride: Int32, inner: Int32) -> Uint32

// Declarada aqui porque este arquivo a usa. Escopo global de
// propósito: `native func` dentro de `module` faz o bind falhar calado e o jogo morrer no boot.
native func BwmsSectorNodeClassHash(path: String, i: Int32) -> Uint64;

public class BwmsWorldNodeInstance {
    private let m_id: Uint64;

    // Nulo se o registro do mundo não tiver instância pra esse id agora.
    public static func FromID(id: Uint64) -> ref<BwmsWorldNodeInstance> {
        let classe: CName = BwmsNodeInstanceNodeClass(id);
        if !IsNameValid(classe) {
            return null;
        }
        let i: ref<BwmsWorldNodeInstance> = new BwmsWorldNodeInstance();
        i.m_id = id;
        return i;
    }

    // Instância do setup `index` de um setor no cache; nula se o setup não tiver GlobalNodeID ou instância no registro.
    public static func FromSectorSetup(sector: String, index: Int32) -> ref<BwmsWorldNodeInstance> {
        let id: Uint64 = BwmsSectorSetupU64(sector, index, 88);
        if id == 0ul {
            return null;
        }
        let i: ref<BwmsWorldNodeInstance> = BwmsWorldNodeInstance.FromID(id);
        return i;
    }

    public func GetID() -> Uint64 {
        return this.m_id;
    }

    // Classe do nó (`worldNode` ou derivada); o nó em si não é entregue ao script.
    public func GetNodeClass() -> CName {
        let classe: CName = BwmsNodeInstanceNodeClass(this.m_id);
        return classe;
    }

    public func GetPosition() -> Vector4 {
        let x: Float = BwmsNodeInstanceF32(this.m_id, 0, 48);
        let y: Float = BwmsNodeInstanceF32(this.m_id, 0, 52);
        let z: Float = BwmsNodeInstanceF32(this.m_id, 0, 56);
        let w: Float = BwmsNodeInstanceF32(this.m_id, 0, 60);
        return Vector4(x, y, z, w);
    }

    public func GetOrientation() -> Quaternion {
        let i: Float = BwmsNodeInstanceF32(this.m_id, 0, 64);
        let j: Float = BwmsNodeInstanceF32(this.m_id, 0, 68);
        let k: Float = BwmsNodeInstanceF32(this.m_id, 0, 72);
        let r: Float = BwmsNodeInstanceF32(this.m_id, 0, 76);
        return Quaternion(i, j, k, r);
    }

    public func GetTransform() -> Transform {
        let t: Transform;
        t.position = this.GetPosition();
        t.orientation = this.GetOrientation();
        return t;
    }

    public func GetScale() -> Vector3 {
        let x: Float = BwmsNodeInstanceF32(this.m_id, 0, 80);
        let y: Float = BwmsNodeInstanceF32(this.m_id, 0, 84);
        let z: Float = BwmsNodeInstanceF32(this.m_id, 0, 88);
        return Vector3(x, y, z);
    }

    // Hash do GlobalNodeID da instância; nó de colisão não tem id próprio (0).
    public func GetGlobalNodeID() -> Uint64 {
        let v: Uint64 = BwmsNodeInstanceU64(this.m_id, 0, 128);
        return v;
    }

    public func GetProxyNodeID() -> Uint64 {
        let v: Uint64 = BwmsNodeInstanceU64(this.m_id, 1, 96);
        return v;
    }

    public func GetStreamingDistance() -> Float {
        let v: Float = BwmsNodeInstanceF32(this.m_id, 1, 124);
        return v;
    }

    // Classe da instância (`worldMeshNodeInstance` etc.); `None` se ela saiu do streaming.
    public func GetInstanceClass() -> CName {
        let h: Uint64 = BwmsNodeInstanceInfo(this.m_id, 0);
        let classe: CName = BwmsHashToName(h);
        return classe;
    }

    // `WorldNodeInstanceNodeState` cru: bit 0 inicializando, 1 inicializado, 2 anexando, 3 desanexando, 5 anexado, 9 visível.
    public func GetState() -> Int32 {
        let v: Uint64 = BwmsNodeInstanceInfo(this.m_id, 1);
        return Cast<Int32>(v);
    }

    public func IsInitialized() -> Bool {
        return (this.GetState() / 2) % 2 == 1;
    }

    public func IsAttached() -> Bool {
        return (this.GetState() / 32) % 2 == 1;
    }

    public func IsVisible() -> Bool {
        return (this.GetState() / 512) % 2 == 1;
    }

    // Classe do objeto que a instância carrega (`CMesh`, o material do decal ou a entidade); `None` pra outros tipos ou objeto ainda não carregado.
    public func GetLoadedObjectClass() -> CName {
        let h: Uint64 = BwmsNodeInstanceInfo(this.m_id, 2);
        let classe: CName = BwmsHashToName(h);
        return classe;
    }
}

// `what` 0 = hash da classe da instância, 1 = `WorldNodeInstanceNodeState`, 2 = hash da classe do objeto carregado
// (mesh em `worldMeshNodeInstance`/`worldBendedMeshNodeInstance`/`worldFoliageNodeInstance`, material em `worldStaticDecalNodeInstance`, entidade em
// `worldEntityNodeInstance`); 0 sem instância.
native func BwmsNodeInstanceInfo(id: Uint64, what: Int32) -> Uint64

public class BwmsInstancedMeshNode {
    private let m_sector: String;
    private let m_index: Int32;

    // Nulo se o setor não estiver no cache ou o nó não for `worldInstancedMeshNode`.
    public static func FromSector(sector: String, index: Int32) -> ref<BwmsInstancedMeshNode> {
        let h: Uint64 = BwmsSectorNodeClassHash(sector, index);
        let classe: CName = BwmsHashToName(h);
        if !Equals(classe, n"worldInstancedMeshNode") {
            return null;
        }
        let n: ref<BwmsInstancedMeshNode> = new BwmsInstancedMeshNode();
        n.m_sector = sector;
        n.m_index = index;
        return n;
    }

    private func Field(name: String, inner: Int32, width: Int32) -> Uint64 {
        let v: Uint64 = BwmsSectorNodeField(this.m_sector, this.m_index, name, inner, width);
        return v;
    }

    public func GetMesh() -> ResRef {
        let h: Uint64 = this.Field("mesh", 0, 8);
        let r: ResRef = BwmsHashToResRef(h);
        return r;
    }

    // Hash do caminho do mesh (FNV-1a64 do caminho, como o `ResRef`).
    public func GetMeshHash() -> Uint64 {
        let h: Uint64 = this.Field("mesh", 0, 8);
        return h;
    }

    public func GetMeshAppearance() -> CName {
        let h: Uint64 = this.Field("meshAppearance", 0, 8);
        let n: CName = BwmsHashToName(h);
        return n;
    }

    // shadowsShadowCastingMode: 0 Default, 1 Always, 2 Never.
    public func GetCastShadows() -> Int32 {
        let v: Uint64 = this.Field("castShadows", 0, 1);
        return Cast<Int32>(v);
    }

    public func GetCastLocalShadows() -> Int32 {
        let v: Uint64 = this.Field("castLocalShadows", 0, 1);
        return Cast<Int32>(v);
    }

    // visWorldOccluderType: 0 Default, 1 None, 2 Detail, 3 MinorInterior, 4 MajorInterior, 5 Exterior.
    public func GetOccluderType() -> Int32 {
        let v: Uint64 = this.Field("occluderType", 0, 1);
        return Cast<Int32>(v);
    }

    public func GetMeshLODScales() -> Uint32 {
        let v: Uint64 = this.Field("meshLODScales", 0, 4);
        return Cast<Uint32>(v);
    }

    public func GetOccluderAutohideDistanceScale() -> Uint8 {
        let v: Uint64 = this.Field("occluderAutohideDistanceScale", 0, 1);
        return Cast<Uint8>(v);
    }

    public func GetVersion() -> Uint8 {
        let v: Uint64 = this.Field("version", 0, 1);
        return Cast<Uint8>(v);
    }

    // `worldTransformsBuffer`: startIndex e numElements dentro do buffer compartilhado do setor.
    public func GetTransformsStartIndex() -> Uint32 {
        let v: Uint64 = this.Field("worldTransformsBuffer", 16, 4);
        return Cast<Uint32>(v);
    }

    public func GetTransformsCount() -> Uint32 {
        let v: Uint64 = this.Field("worldTransformsBuffer", 20, 4);
        return Cast<Uint32>(v);
    }

    // Arrays de runtime (fora do CR2W): tamanho, -1 se ilegível.
    public func GetBoundingBoxCount() -> Int32 {
        let n: Int32 = BwmsSectorNodeArraySize(this.m_sector, this.m_index, 80);
        return n;
    }

    public func GetStreamingDistanceCount() -> Int32 {
        let n: Int32 = BwmsSectorNodeArraySize(this.m_sector, this.m_index, 96);
        return n;
    }

    public func GetInstanceBatchCount() -> Int32 {
        let n: Int32 = BwmsSectorNodeArraySize(this.m_sector, this.m_index, 112);
        return n;
    }

    private func BoxValue(i: Int32, inner: Int32) -> Float {
        let v: Float = BwmsSectorNodeArrayF32(this.m_sector, this.m_index, 80, i, 32, inner);
        return v;
    }

    // Caixa `i` (Min em +0, Max em +16); NaN fora do array.
    public func GetBoundingBox(i: Int32) -> Box {
        let b: Box;
        let minX: Float = this.BoxValue(i, 0);
        let minY: Float = this.BoxValue(i, 4);
        let minZ: Float = this.BoxValue(i, 8);
        let minW: Float = this.BoxValue(i, 12);
        let maxX: Float = this.BoxValue(i, 16);
        let maxY: Float = this.BoxValue(i, 20);
        let maxZ: Float = this.BoxValue(i, 24);
        let maxW: Float = this.BoxValue(i, 28);
        b.Min = Vector4(minX, minY, minZ, minW);
        b.Max = Vector4(maxX, maxY, maxZ, maxW);
        return b;
    }

    // Distância de streaming `i`; NaN fora do array.
    public func GetStreamingDistance(i: Int32) -> Float {
        let v: Float = BwmsSectorNodeArrayF32(this.m_sector, this.m_index, 96, i, 4, 0);
        return v;
    }

    // Lote de instâncias `i`; 0xFFFFFFFF fora do array.
    public func GetInstanceBatch(i: Int32) -> Uint32 {
        let v: Uint32 = BwmsSectorNodeArrayU32(this.m_sector, this.m_index, 112, i, 4, 0);
        return v;
    }
}
