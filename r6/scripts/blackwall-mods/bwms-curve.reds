// bwms-curve.reds — curvas de dados do jogo por propriedade de objeto, 2026-09-13.
//
// PARA QUE SERVE: ler e editar em runtime as curvas `curveData` que o jogo guarda nos objetos — exposição, luz do sol, névoa, curvas de avaliador — sem
// arquivo novo. Um ponto é um tempo e um valor; a curva tem o tipo de interpolação e o de ligação entre segmentos.
//
// COMO CHEGA NA CURVA: neste build não existem classes de script pra curva. A curva é uma propriedade nativa (`curveData:Float`, `:Vector2`, `:Vector3`,
// `:Vector4`, `:HDRColor`); `BwmsCurve.Of(objeto, n"nomeDaPropriedade")` aponta pra ela e as chamadas vão pelo nome. Propriedade inexistente ou de outro tipo
// dá `IsValid() == false`, tamanho -1 e escrita recusada.
//
// TIPOS DE INTERPOLAÇÃO: 0 constante, 1 linear, 2 Bézier quadrática, 3 Bézier cúbica, 4 Hermite. LIGAÇÃO: 0 normal, 1 suave, 2 suave simétrica.
//
// USO: `let curva = BwmsCurve.Of(config, n"sunColor"); curva.SetSize(2); curva.SetPointHDRColor(1, 0.5, new HDRColor(1.0, 0.8, 0.6, 1.0));`
//
// `BwmsCurve` é uma classe só, com métodos por tipo de valor.

public class BwmsCurve {
    private let m_owner: wref<IScriptable>;
    private let m_property: CName;

    public static func Of(owner: ref<IScriptable>, property: CName) -> ref<BwmsCurve> {
        let curve: ref<BwmsCurve> = new BwmsCurve();
        curve.m_owner = owner;
        curve.m_property = property;
        return curve;
    }

    // 0 Float, 1 Vector2, 2 Vector3, 3 Vector4, 4 HDRColor; -1 sem curva.
    public func GetValueKind() -> Int32 {
        let owner: ref<IScriptable> = this.m_owner;
        let kind: Int32 = BwmsCurveKind(owner, this.m_property);
        return kind;
    }

    public func IsValid() -> Bool {
        let kind: Int32 = this.GetValueKind();
        return kind >= 0;
    }

    public func GetSize() -> Int32 {
        let owner: ref<IScriptable> = this.m_owner;
        let size: Int32 = BwmsCurveSize(owner, this.m_property);
        return size;
    }

    public func SetSize(size: Int32) -> Bool {
        let owner: ref<IScriptable> = this.m_owner;
        let ok: Bool = BwmsCurveResize(owner, this.m_property, size);
        return ok;
    }

    public func GetInterpolationType() -> Int32 {
        let owner: ref<IScriptable> = this.m_owner;
        let value: Int32 = BwmsCurveMode(owner, this.m_property, false);
        return value;
    }

    public func SetInterpolationType(value: Int32) -> Bool {
        let owner: ref<IScriptable> = this.m_owner;
        let ok: Bool = BwmsCurveSetMode(owner, this.m_property, false, value);
        return ok;
    }

    public func GetLinkType() -> Int32 {
        let owner: ref<IScriptable> = this.m_owner;
        let value: Int32 = BwmsCurveMode(owner, this.m_property, true);
        return value;
    }

    public func SetLinkType(value: Int32) -> Bool {
        let owner: ref<IScriptable> = this.m_owner;
        let ok: Bool = BwmsCurveSetMode(owner, this.m_property, true, value);
        return ok;
    }

    // Tempo do ponto; um valor gigante (1e30) fora da faixa.
    public func GetPoint(index: Int32) -> Float {
        let owner: ref<IScriptable> = this.m_owner;
        let point: Float = BwmsCurvePoint(owner, this.m_property, index);
        return point;
    }

    public func GetValueComponent(index: Int32, component: Int32) -> Float {
        let owner: ref<IScriptable> = this.m_owner;
        let value: Float = BwmsCurveValue(owner, this.m_property, index, component);
        return value;
    }

    // Tempo e os 4 componentes de uma vez; componentes além do tipo são ignorados.
    public func SetPointComponents(index: Int32, point: Float, x: Float, y: Float, z: Float, w: Float) -> Bool {
        let owner: ref<IScriptable> = this.m_owner;
        let pointOk: Bool = BwmsCurveSetPoint(owner, this.m_property, index, point);
        let valueOk: Bool = BwmsCurveSetValue(owner, this.m_property, index, x, y, z, w);
        return pointOk && valueOk;
    }

    public func GetValueFloat(index: Int32) -> Float {
        return this.GetValueComponent(index, 0);
    }

    public func GetValueVector2(index: Int32) -> Vector2 {
        return new Vector2(this.GetValueComponent(index, 0), this.GetValueComponent(index, 1));
    }

    public func GetValueVector3(index: Int32) -> Vector3 {
        return new Vector3(this.GetValueComponent(index, 0), this.GetValueComponent(index, 1), this.GetValueComponent(index, 2));
    }

    public func GetValueVector4(index: Int32) -> Vector4 {
        return new Vector4(this.GetValueComponent(index, 0), this.GetValueComponent(index, 1), this.GetValueComponent(index, 2), this.GetValueComponent(index, 3));
    }

    public func GetValueHDRColor(index: Int32) -> HDRColor {
        return new HDRColor(this.GetValueComponent(index, 0), this.GetValueComponent(index, 1), this.GetValueComponent(index, 2), this.GetValueComponent(index, 3));
    }

    public func SetPointFloat(index: Int32, point: Float, value: Float) -> Bool {
        return this.SetPointComponents(index, point, value, 0.0, 0.0, 0.0);
    }

    public func SetPointVector2(index: Int32, point: Float, value: Vector2) -> Bool {
        return this.SetPointComponents(index, point, value.X, value.Y, 0.0, 0.0);
    }

    public func SetPointVector3(index: Int32, point: Float, value: Vector3) -> Bool {
        return this.SetPointComponents(index, point, value.X, value.Y, value.Z, 0.0);
    }

    public func SetPointVector4(index: Int32, point: Float, value: Vector4) -> Bool {
        return this.SetPointComponents(index, point, value.X, value.Y, value.Z, value.W);
    }

    public func SetPointHDRColor(index: Int32, point: Float, value: HDRColor) -> Bool {
        return this.SetPointComponents(index, point, value.Red, value.Green, value.Blue, value.Alpha);
    }
}

public static native func BwmsCurveKind(owner: ref<IScriptable>, property: CName) -> Int32
public static native func BwmsCurveSize(owner: ref<IScriptable>, property: CName) -> Int32
public static native func BwmsCurveResize(owner: ref<IScriptable>, property: CName, size: Int32) -> Bool
public static native func BwmsCurveMode(owner: ref<IScriptable>, property: CName, link: Bool) -> Int32
public static native func BwmsCurveSetMode(owner: ref<IScriptable>, property: CName, link: Bool, value: Int32) -> Bool
public static native func BwmsCurvePoint(owner: ref<IScriptable>, property: CName, index: Int32) -> Float
public static native func BwmsCurveValue(owner: ref<IScriptable>, property: CName, index: Int32, component: Int32) -> Float
public static native func BwmsCurveSetPoint(owner: ref<IScriptable>, property: CName, index: Int32, point: Float) -> Bool
public static native func BwmsCurveSetValue(owner: ref<IScriptable>, property: CName, index: Int32, x: Float, y: Float, z: Float, w: Float) -> Bool
