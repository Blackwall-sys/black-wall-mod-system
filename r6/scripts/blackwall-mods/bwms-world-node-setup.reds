// bwms-world-node-setup.reds — Codeware `#83` (`WorldNodeSetupWrapper`): leitura e escrita do setup compilado de um nó de um setor carregado
// (`worldCompiledNodeInstanceSetupInfo`, 0x90 B no `DataBuffer` do setor). O setor vem pelo caminho e o setup pelo índice. Offsets: position 0x00,
// orientation 0x10, scale 0x20, secondaryRefPointPosition 0x2C, streamingRefPoint 0x38, ponteiro do nó 0x50, globalNodeID 0x58, proxyNodeID 0x60,
// nodeIndex nos 19 bits de baixo de 0x70, secondaryRefPointDistance 0x78, streamingDistance 0x7C (layout da 2.5; setor da 2.31 o jogo converte no
// carregamento). Os escritores recusam o ponteiro do nó e o campo do índice (só `SetNodeIndex` troca os dois, junto do
// índice, pelo nó do array do setor) e qualquer campo fora dos 0x90 B. A mudança vale pra instância que o streaming criar depois dela.

native func BwmsSectorCount(path: String, qual: Int32) -> Int32;
native func BwmsSectorSetup(path: String, i: Int32, offset: Int32) -> Int32;
native func BwmsSectorSetupF32(path: String, i: Int32, offset: Int32) -> Float;
native func BwmsSectorSetupU64(path: String, i: Int32, offset: Int32) -> Uint64;
native func BwmsSectorSetupSetF32(path: String, i: Int32, offset: Int32, value: Float) -> Bool;
native func BwmsSectorSetupSetU64(path: String, i: Int32, offset: Int32, value: Uint64) -> Bool;
native func BwmsSectorSetupSetNodeIndex(path: String, i: Int32, index: Int32) -> Bool;
native func BwmsHashToNodeRef(hash: Uint64) -> NodeRef;
native func BwmsNodeRefToHash(value: NodeRef) -> Uint64;

public class BwmsWorldNodeSetup extends IScriptable {
    private let m_path: String;
    private let m_index: Int32;

    public static func Create(path: String, index: Int32) -> ref<BwmsWorldNodeSetup> {
        let setup: ref<BwmsWorldNodeSetup> = new BwmsWorldNodeSetup();
        setup.m_path = path;
        setup.m_index = index;
        return setup;
    }

    public func GetSectorPath() -> String {
        return this.m_path;
    }

    public func GetIndex() -> Int32 {
        return this.m_index;
    }

    public func IsValid() -> Bool {
        let total: Int32 = BwmsSectorCount(this.m_path, 2);
        return this.m_index >= 0 && this.m_index < total;
    }

    public func GetNodeIndex() -> Int32 {
        let bruto: Int32 = BwmsSectorSetup(this.m_path, this.m_index, 112);
        return bruto & 524287;
    }

    public func GetTransform() -> Transform {
        let transform: Transform;
        transform.position = this.GetPosition();
        transform.orientation = this.GetOrientation();
        return transform;
    }

    public func GetPosition() -> Vector4 {
        return new Vector4(this.F(0), this.F(4), this.F(8), this.F(12));
    }

    public func GetOrientation() -> Quaternion {
        let q: Quaternion;
        q.i = this.F(16);
        q.j = this.F(20);
        q.k = this.F(24);
        q.r = this.F(28);
        return q;
    }

    public func GetScale() -> Vector3 {
        return new Vector3(this.F(32), this.F(36), this.F(40));
    }

    public func GetNodeRef() -> NodeRef {
        let ref: NodeRef = BwmsHashToNodeRef(this.U(88));
        return ref;
    }

    public func GetGlobalNodeID() -> Uint64 {
        return this.U(88);
    }

    public func GetProxyNodeID() -> Uint64 {
        return this.U(96);
    }

    public func GetStreamingPosition() -> Vector3 {
        return new Vector3(this.F(56), this.F(60), this.F(64));
    }

    public func GetStreamingDistance() -> Float {
        return this.F(124);
    }

    public func GetSecondaryRefPointPosition() -> Vector3 {
        return new Vector3(this.F(44), this.F(48), this.F(52));
    }

    public func GetSecondaryRefPointDistance() -> Float {
        return this.F(120);
    }

    public func SetNodeIndex(index: Int32) -> Bool {
        let ok: Bool = BwmsSectorSetupSetNodeIndex(this.m_path, this.m_index, index);
        return ok;
    }

    public func SetTransform(transform: Transform) -> Bool {
        let posicao: Bool = this.SetPosition(transform.position);
        let orientacao: Bool = this.SetOrientation(transform.orientation);
        return posicao && orientacao;
    }

    public func SetPosition(position: Vector4) -> Bool {
        return this.SetF(0, position.X) && this.SetF(4, position.Y) && this.SetF(8, position.Z) && this.SetF(12, position.W);
    }

    public func SetOrientation(orientation: Quaternion) -> Bool {
        return this.SetF(16, orientation.i) && this.SetF(20, orientation.j) && this.SetF(24, orientation.k) && this.SetF(28, orientation.r);
    }

    public func SetScale(scale: Vector3) -> Bool {
        return this.SetF(32, scale.X) && this.SetF(36, scale.Y) && this.SetF(40, scale.Z);
    }

    public func SetNodeRef(ref: NodeRef) -> Bool {
        let hash: Uint64 = BwmsNodeRefToHash(ref);
        return this.SetU(88, hash);
    }

    public func SetGlobalNodeID(id: Uint64) -> Bool {
        return this.SetU(88, id);
    }

    public func SetProxyNodeID(id: Uint64) -> Bool {
        return this.SetU(96, id);
    }

    public func SetStreamingPosition(position: Vector3) -> Bool {
        return this.SetF(56, position.X) && this.SetF(60, position.Y) && this.SetF(64, position.Z);
    }

    public func SetStreamingDistance(distance: Float) -> Bool {
        return this.SetF(124, distance);
    }

    public func SetSecondaryRefPointPosition(position: Vector3) -> Bool {
        return this.SetF(44, position.X) && this.SetF(48, position.Y) && this.SetF(52, position.Z);
    }

    public func SetSecondaryRefPointDistance(distance: Float) -> Bool {
        return this.SetF(120, distance);
    }

    private func F(offset: Int32) -> Float {
        let valor: Float = BwmsSectorSetupF32(this.m_path, this.m_index, offset);
        return valor;
    }

    private func U(offset: Int32) -> Uint64 {
        let valor: Uint64 = BwmsSectorSetupU64(this.m_path, this.m_index, offset);
        return valor;
    }

    private func SetF(offset: Int32, value: Float) -> Bool {
        let ok: Bool = BwmsSectorSetupSetF32(this.m_path, this.m_index, offset, value);
        return ok;
    }

    private func SetU(offset: Int32, value: Uint64) -> Bool {
        let ok: Bool = BwmsSectorSetupSetU64(this.m_path, this.m_index, offset, value);
        return ok;
    }
}
