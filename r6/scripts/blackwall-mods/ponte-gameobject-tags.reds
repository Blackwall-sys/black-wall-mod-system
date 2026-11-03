// ponte-gameobject-tags.reds — lista de tags (8 métodos:
// `IsEmpty`/`Clear`/`HasTag`/`AddTag`/`RemoveTag`/`HasTags`/`AddTags`/`RemoveTags`), 2026-08-11,
// sessão de candidatos baratos.
//
// ESCOPO (documentado): `TagList` como STRUCT
// VALUE-TYPE próprio (`native struct TagList { native let tags: array<CName> }`), que um mod
// obteria via `entity.GetTags()` e manipularia como objeto solto, é risco ALTO já
// documentado no catálogo (mesma categoria do `#30`/CMesh — tipo sem backing script-facing
// confirmado, `redscript-src` tem ZERO ocorrências de `TagList`). NÃO declarado.
//
// Em vez disso, esta implementação expõe a MESMA capacidade prática (consultar/mutar o conjunto
// de tags de um `GameObject`) diretamente em cima da property REAL `GameObject.tags` — a MESMA
// que `AddTag`/#27 (`ponte-gameobject-addtag.reds`, já FECHADO e provado ao vivo) já
// escreve, e que `GameObject.HasTag` (NATIVA VANILLA REAL, `redscript-src/core/entity/
// gameObject.script:182`) já lê. `RemoveTag`/`IsEmpty`/`Clear` (Rust, `tramp_gameobject_
// remove_tag`/`tramp_gameobject_tag_count`/`tramp_gameobject_clear_tags`) reusam EXATAMENTE as
// mesmas 2 primitivas RTTI do AddTag (`resolve_prop_in_class`+`field_ptr`) mais
// `dynarray_remove_at` (memmove puro, zero endereço nativo).
// `HasTags`/`AddTags`/`RemoveTags` (múltiplos) são 100% redscript, compostos sobre os métodos
// singulares — zero código Rust adicional.
native func BwmsGameObjectRemoveTag(obj: ref<IScriptable>, tag: CName) -> Bool
native func BwmsGameObjectTagCount(obj: ref<IScriptable>) -> Int32
native func BwmsGameObjectClearTags(obj: ref<IScriptable>) -> Bool

@addMethod(GameObject)
public func RemoveTag(tag: CName) -> Void {
    BwmsGameObjectRemoveTag(this, tag);
}

@addMethod(GameObject)
public func TagCount() -> Int32 {
    return BwmsGameObjectTagCount(this);
}

@addMethod(GameObject)
public func IsEmptyOfTags() -> Bool {
    return this.TagCount() == 0;
}

@addMethod(GameObject)
public func ClearTags() -> Void {
    BwmsGameObjectClearTags(this);
}

@addMethod(GameObject)
public func HasTags(tags: array<CName>) -> Bool {
    let i: Int32 = 0;
    while i < ArraySize(tags) {
        if !this.HasTag(tags[i]) {
            return false;
        }
        i += 1;
    }
    return true;
}

@addMethod(GameObject)
public func AddTags(tags: array<CName>) -> Void {
    let i: Int32 = 0;
    while i < ArraySize(tags) {
        this.AddTag(tags[i]);
        i += 1;
    }
}

@addMethod(GameObject)
public func RemoveTags(tags: array<CName>) -> Void {
    let i: Int32 = 0;
    while i < ArraySize(tags) {
        this.RemoveTag(tags[i]);
        i += 1;
    }
}
