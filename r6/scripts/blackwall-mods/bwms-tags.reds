// Tag de entidade no registro do motor — o `TagSystem::AssignTag`/`UnassignTag` que fica por baixo do `DynamicEntitySystem` e do
// `StaticEntitySystem`, com nome próprio do BWMS. A tag entra no registro do `GameTagSystem` (só em memória, não vai pro save), e as consultas
// vanilla `GetAnyMatchingEntity`/`GetAllMatchingEntities` passam a achar a entidade.
//
//   let tags: ref<GameTagSystem> = GameInstance.GetGameTagSystem(game);
//   tags.AssignEntityTag(npc.GetEntityID(), n"meu_mod_alvo");
//   let alvo: ref<Entity> = tags.GetAnyMatchingEntity(n"meu_mod_alvo");
//   tags.UnassignEntityTag(npc.GetEntityID(), n"meu_mod_alvo");

native func BwmsTagSystemAssignTag(sys: ref<IScriptable>, entityID: EntityID, tag: CName) -> Bool;
native func BwmsTagSystemUnassignTag(sys: ref<IScriptable>, entityID: EntityID, tag: CName) -> Bool;

@addMethod(GameTagSystem)
public func AssignEntityTag(entityID: EntityID, tag: CName) -> Bool {
    return BwmsTagSystemAssignTag(this, entityID, tag);
}

@addMethod(GameTagSystem)
public func UnassignEntityTag(entityID: EntityID, tag: CName) -> Bool {
    return BwmsTagSystemUnassignTag(this, entityID, tag);
}
