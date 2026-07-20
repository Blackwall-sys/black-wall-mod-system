// BWMS — `cw-utils` cauda (Vector2 helpers + Casts entre Vector2/3/4, 2026-07-15): declaração
// IDÊNTICA à fonte real (enablers/Codeware/scripts/Utils/Vector2.reds). 100% REDSCRIPT PURO —
// Vector2/Vector3/Vector4 já são tipos NATIVOS do motor; zero native/RTTI nosso envolvido.

public func OperatorAdd(a: Vector2, b: Vector2) -> Vector2 {
  a.X += b.X;
  a.Y += b.Y;
  return a;
}

public func OperatorSubtract(a: Vector2, b: Vector2) -> Vector2 {
  a.X -= b.X;
  a.Y -= b.Y;
  return a;
}

public func OperatorMultiply(a: Vector2, b: Vector2) -> Vector2 {
  a.X *= b.X;
  a.Y *= b.Y;
  return a;
}

public func OperatorDivide(a: Vector2, b: Vector2) -> Vector2 {
  if b.X != 0.0 {
    a.X /= b.X;
  }
  if b.Y != 0.0 {
    a.Y /= b.Y;
  }
  return a;
}

public func OperatorAssignAdd(out a: Vector2, b: Vector2) -> Vector2 {
  a = a + b;
  return a;
}

public func OperatorAssignSubtract(out a: Vector2, b: Vector2) -> Vector2 {
  a = a - b;
  return a;
}

public func OperatorAssignMultiply(out a: Vector2, b: Vector2) -> Vector2 {
  a = a * b;
  return a;
}

public func OperatorAssignDivide(out a: Vector2, b: Vector2) -> Vector2 {
  a = a / b;
  return a;
}

public func OperatorEqual(a: Vector2, b: Vector2) -> Bool {
  return a.X == b.X && a.Y == b.Y;
}

public func OperatorGreater(a: Vector2, b: Vector2) -> Bool {
  return a.X > b.X && a.Y > b.Y;
}

public func OperatorGreaterEqual(a: Vector2, b: Vector2) -> Bool {
  return a.X >= b.X && a.Y >= b.Y;
}

public func OperatorLess(a: Vector2, b: Vector2) -> Bool {
  return a.X < b.X && a.Y < b.Y;
}

public func OperatorLessEqual(a: Vector2, b: Vector2) -> Bool {
  return a.X <= b.X && a.Y <= b.Y;
}

public func Cast(value: Vector3) -> Vector2 {
  let result: Vector2;
  result.X = value.X;
  result.Y = value.Y;
  return result;
}

public func Cast(value: Vector2) -> Vector3 {
  let result: Vector3;
  result.X = value.X;
  result.Y = value.Y;
  return result;
}

public func Cast(value: Vector4) -> Vector2 {
  let result: Vector2;
  result.X = value.X;
  result.Y = value.Y;
  return result;
}

public func Cast(value: Vector2) -> Vector4 {
  let result: Vector4;
  result.X = value.X;
  result.Y = value.Y;
  return result;
}

// `cw-utils`: TDBID.ToNumber JÁ EXISTE NATIVAMENTE no motor (redscript-all.reds:14687,
// `public final static native func ToNumber(tdbID: TweakDBID) -> Uint64`) — zero risco, é só
// expor com o nome de conveniência do Codeware (wrapper redscript sobre native do PRÓPRIO JOGO,
// não nossa). A direção inversa (TDBID.FromNumber) NÃO existe nativamente — precisa de RTTI_
// EXPAND_CLASS no motor (fonte real: App/Utils/TweakDBID.hpp) e fica fora desta fatia (mesma
// categoria de risco do @addMethod em classe existente, não tentado sem RE dedicada).
public func Cast(value: TweakDBID) -> Uint64 = TDBID.ToNumber(value)
