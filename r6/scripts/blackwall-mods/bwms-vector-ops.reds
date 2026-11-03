// bwms-vector-ops.reds — aritmética de `Vector2`/`Vector3`.
//
// POR QUE ISTO É UM GAP DE VERDADE (checado, não suposto): o jogo declara 39 sobrecargas de
// `OperatorAdd` no redscript vanilla — e **todas** são de `Vector4`. `Vector2` e `Vector3` têm
// ZERO operador: as structs (`redscript-all.reds:54829` e `:107138`) trazem só `Normalize` e, no
// caso do `Vector3`, `Lerp`. Somar dois `Vector2` em redscript puro não é possível sem isto.
//
// Escopo escolhido: o que UI e geometria de mod realmente usam. Nada de matriz/quaternion — se
// entrar, entra quando houver consumidor, não por completude decorativa.

// --- Vector2 -------------------------------------------------------------------------------

public static func OperatorAdd(a: Vector2, b: Vector2) -> Vector2 {
    let r: Vector2;
    r.X = a.X + b.X;
    r.Y = a.Y + b.Y;
    return r;
}

public static func OperatorSubtract(a: Vector2, b: Vector2) -> Vector2 {
    let r: Vector2;
    r.X = a.X - b.X;
    r.Y = a.Y - b.Y;
    return r;
}

public static func OperatorMultiply(a: Vector2, k: Float) -> Vector2 {
    let r: Vector2;
    r.X = a.X * k;
    r.Y = a.Y * k;
    return r;
}

public static func OperatorDivide(a: Vector2, k: Float) -> Vector2 {
    let r: Vector2;
    // Divisão por zero em layout de UI produz NaN que se espalha silencioso pela árvore de
    // widgets. Devolver o vetor intacto é o comportamento menos destrutivo.
    if AbsF(k) < 0.00001 {
        return a;
    }
    r.X = a.X / k;
    r.Y = a.Y / k;
    return r;
}

// --- Vector3 -------------------------------------------------------------------------------

public static func OperatorAdd(a: Vector3, b: Vector3) -> Vector3 {
    let r: Vector3;
    r.X = a.X + b.X;
    r.Y = a.Y + b.Y;
    r.Z = a.Z + b.Z;
    return r;
}

public static func OperatorSubtract(a: Vector3, b: Vector3) -> Vector3 {
    let r: Vector3;
    r.X = a.X - b.X;
    r.Y = a.Y - b.Y;
    r.Z = a.Z - b.Z;
    return r;
}

public static func OperatorMultiply(a: Vector3, k: Float) -> Vector3 {
    let r: Vector3;
    r.X = a.X * k;
    r.Y = a.Y * k;
    r.Z = a.Z * k;
    return r;
}

public static func OperatorDivide(a: Vector3, k: Float) -> Vector3 {
    let r: Vector3;
    if AbsF(k) < 0.00001 {
        return a;
    }
    r.X = a.X / k;
    r.Y = a.Y / k;
    r.Z = a.Z / k;
    return r;
}

// --- Utilidades que faltavam junto ----------------------------------------------------------

public abstract class BwmsVec {

    public static func Make2(x: Float, y: Float) -> Vector2 {
        let r: Vector2;
        r.X = x;
        r.Y = y;
        return r;
    }

    public static func Make3(x: Float, y: Float, z: Float) -> Vector3 {
        let r: Vector3;
        r.X = x;
        r.Y = y;
        r.Z = z;
        return r;
    }

    public static func Dot2(a: Vector2, b: Vector2) -> Float {
        return a.X * b.X + a.Y * b.Y;
    }

    public static func Dot3(a: Vector3, b: Vector3) -> Float {
        return a.X * b.X + a.Y * b.Y + a.Z * b.Z;
    }

    public static func Length2(a: Vector2) -> Float {
        return SqrtF(a.X * a.X + a.Y * a.Y);
    }

    public static func Length3(a: Vector3) -> Float {
        return SqrtF(a.X * a.X + a.Y * a.Y + a.Z * a.Z);
    }

    public static func Lerp2(a: Vector2, b: Vector2, t: Float) -> Vector2 {
        let r: Vector2;
        r.X = a.X + (b.X - a.X) * t;
        r.Y = a.Y + (b.Y - a.Y) * t;
        return r;
    }
}
