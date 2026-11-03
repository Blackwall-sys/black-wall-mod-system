// bwms-iplacedcomponent-worldtransform.reds — a transformação do componente no
// espaço do mundo, com nome do BWMS (`BwmsGetWorld*`).
//
// Declarar `@addField(IPlacedComponent) public native let worldTransform: WorldTransform` é `native let` reivindicando storage
// nativo, a categoria do `CMesh`, que derruba o bind. A classe vanilla `IPlacedComponent` já tem `GetLocalToWorld() -> Matrix`
// (`orphans.script:16676-16697`), que é por definição a matriz do componente no espaço do mundo, e `Matrix` (`native struct` vanilla,
// `orphans.script:25902`) tem `GetTranslation(m)`/`GetRotation(m)`: a mesma posição e orientação, sem RE e sem forja.
//
// Diferença: `Vector4`/`EulerAngles` em vez de `WorldPosition`/`Quaternion` — perde a precisão dupla de coordenadas MUITO longe da
// origem do mundo, irrelevante na escala normal de gameplay.

@addMethod(IPlacedComponent)
public func BwmsGetWorldTransformMatrix() -> Matrix {
    return this.GetLocalToWorld();
}

@addMethod(IPlacedComponent)
public func BwmsGetWorldPosition() -> Vector4 {
    return Matrix.GetTranslation(this.GetLocalToWorld());
}

@addMethod(IPlacedComponent)
public func BwmsGetWorldOrientation() -> EulerAngles {
    return Matrix.GetRotation(this.GetLocalToWorld());
}
