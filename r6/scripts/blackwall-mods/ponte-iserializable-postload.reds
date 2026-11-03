// ponte-iserializable-postload.reds — Codeware `Scripting/ISerializable.reds::ProcessPostLoad`/`RefreshResource` (item #58), em
// `IScriptable`.
//
// No Codeware os dois ficam em `ISerializable`, que não aparece no corpus vanilla — declarar ou `@addMethod` sobre ele é a categoria do
// `CMesh`, que derruba o bind. `IScriptable` é vanilla e seguro, e na hierarquia C++ herda o mesmo slot de vtable do `PostLoad` (slot de
// classe-base não se move por herança). O slot (Mac +0x30, Windows +0x28 no `RED4ext.SDK/include/RED4ext/ISerializable.hpp`) foi
// conferido em classes independentes; o despacho lê a vtable do PRÓPRIO objeto em runtime (`iserializable_process_post_load`, rtti.rs).
//
// ⚠️ Efeito real, não leitura: `PostLoad` reprocessa o objeto como recém-carregado do disco.
native func BwmsProcessPostLoad(obj: ref<IScriptable>, disablePreInitialization: Bool) -> Bool

@addMethod(IScriptable)
public func ProcessPostLoad(opt disablePreInitialization: Bool) -> Bool {
    return BwmsProcessPostLoad(this, disablePreInitialization);
}

// Nome real da fonte (`ISerializable.reds:8-10`): `RefreshResource` é sugar puro sobre
// `ProcessPostLoad`, cópia fiel do corpo (`this.ProcessPostLoad(disablePreInitialization)`).
@addMethod(IScriptable)
public func RefreshResource(opt disablePreInitialization: Bool) -> Bool {
    return this.ProcessPostLoad(disablePreInitialization);
}
