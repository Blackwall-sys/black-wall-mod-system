// ponte-campos-do-motor.reds — campos e tipos que o motor tem no RTTI e o script do jogo não declara, com os nomes que mods do Windows
// usam direto (Equipment-EX, Virtual Atelier). Nome, tipo e existência medidos pelo `propdump` no boot de 2026-09-28: `native let` de campo
// que o motor não tem derruba o bind (`tipos_do_motor::CAMPOS_NATIVOS`).

// `inkScrollController`: `direction` +0xf4, `position` +0xfc, `scrollDelta` +0x108, `viewportSize` +0x10c, `contentSize` +0x114.
@addField(inkScrollController)
public native let direction: inkEScrollDirection;

@addField(inkScrollController)
public native let position: Float;

@addField(inkScrollController)
public native let scrollDelta: Float;

@addField(inkScrollController)
public native let viewportSize: Vector2;

@addField(inkScrollController)
public native let contentSize: Vector2;

// Câmera da prévia do personagem nos menus: `inkPuppetPreviewGameController` (no motor `gameuiPuppetPreviewGameController`) guarda o
// controller da câmera embutido em +0x128, e ele guarda os enquadramentos em `cameraSetup` (+0).
public native struct gameuiPuppetPreviewCameraSetup {
    public native let slotName: CName;
    public native let cameraZoom: Float;
    public native let interpolationTime: Float;
}

public native struct gameuiPuppetPreviewCameraController {
    public native let cameraSetup: array<gameuiPuppetPreviewCameraSetup>;
    public native let transitionDelay: Float;
}

@addField(inkPuppetPreviewGameController)
public native let cameraController: gameuiPuppetPreviewCameraController;

// `inkImage` (no motor `inkImageWidget`): os campos `useNineSliceScale`, `nineSliceScale`, `tileHAlign` e `tileVAlign` são declarados pelo
// jogo; aqui entram os acessores do Codeware.
@addMethod(inkImage)
public func SetNineSliceScale(enable: Bool) -> Void {
    this.useNineSliceScale = enable;
}

@addMethod(inkImage)
public func UsesNineSliceScale() -> Bool {
    return this.useNineSliceScale;
}

@addMethod(inkImage)
public func SetNineSliceGrid(grid: inkMargin) -> Void {
    this.nineSliceScale = grid;
}

@addMethod(inkImage)
public func GetNineSliceGrid() -> inkMargin {
    return this.nineSliceScale;
}

@addMethod(inkImage)
public func GetTileHAlign() -> inkEHorizontalAlign {
    return this.tileHAlign;
}

@addMethod(inkImage)
public func SetTileHAlign(align: inkEHorizontalAlign) -> Void {
    this.tileHAlign = align;
}

@addMethod(inkImage)
public func GetTileVAlign() -> inkEVerticalAlign {
    return this.tileVAlign;
}

@addMethod(inkImage)
public func SetTileVAlign(align: inkEVerticalAlign) -> Void {
    this.tileVAlign = align;
}

