// ponte-gameobject-addtag.reds — `GameObject.AddTag` (2026-08-11) —
// composição pura de RTTI já provado (resolve_prop_in_class+field_ptr+dynarray_push_ptr), zero
// endereço nativo. `GameObject.HasTag`/`GetTags` já são NATIVAS VANILLA REAIS (confirmadas em
// `redscript-src/core/entity/gameObject.script`) — `AddTag` fecha o trio leitura+leitura+escrita.
native func BwmsGameObjectAddTag(obj: ref<IScriptable>, tag: CName) -> Bool

@addMethod(GameObject)
public func AddTag(tag: CName) -> Void {
    BwmsGameObjectAddTag(this, tag);
}
