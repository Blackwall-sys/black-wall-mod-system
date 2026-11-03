// ponte-inkwidgetlibrary.reds — Codeware `scripts/UI/Core/inkWidgetLibraryReference.reds` (catálogo `#94`). `inkWidgetLibraryResource.SetPath`/
// `Create(path)` e `inkWidgetLibraryReference.Create(path)` são natives do jogo desde a 2.5; o jogo não tem o `Create` com o item.

@addMethod(inkWidgetLibraryReference)
public static func Create(path: ResRef, item: CName) -> inkWidgetLibraryReference {
    return inkWidgetLibraryReference(inkWidgetLibraryResource.Create(path), item);
}
