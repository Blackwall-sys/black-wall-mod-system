// bwms-communitysystem-noderef.reds — atalhos do BWMS sobre o `CommunitySystem` vanilla por `NodeRef`: ativar, desativar,
// resetar e mudar a fase de comunidade, e ativar, desativar e resetar spawner (sufixo `ByNodeRef`), mais os 2 getters.
//
// O `NodeRef` é resolvido pelo `ResolveNodeRef` vanilla contra a raiz global, e o
// `GlobalNodeRef` vira `EntityID` pelo `Cast` vanilla (`orphans.script:58498`/`31096`/`23679`/`31114`/`23725`). As mutações vão
// pelos slots do `CommunitySystem` (`bwms-communitysystem.reds`) e NÃO chamam o `CommunitySystem::Update`
// depois. A API com os mesmos métodos, os aliases e o `Update` é o `WorldStateSystem`
// (`GameInstance.GetWorldStateSystem()`, `ponte-worldstatesystem.reds`).

@addMethod(CommunitySystem)
public func BwmsActivateCommunityByNodeRef(nodeRef: NodeRef, opt entryName: CName) -> Bool {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return false;
    }
    this.BwmsActivateCommunity(communityId, entryName);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsDeactivateCommunityByNodeRef(nodeRef: NodeRef, opt entryName: CName) -> Bool {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return false;
    }
    this.BwmsDeactivateCommunity(communityId, entryName);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsResetCommunityByNodeRef(nodeRef: NodeRef, opt entryName: CName) -> Bool {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return false;
    }
    this.BwmsResetCommunity(communityId, entryName);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsSetCommunityPhaseByNodeRef(nodeRef: NodeRef, entryName: CName, phaseName: CName) -> Bool {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return false;
    }
    this.BwmsSetCommunityPhase(communityId, entryName, phaseName);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsActivateSpawnerByNodeRef(nodeRef: NodeRef) -> Bool {
    let spawnerId: Uint64;
    if !BwmsResolveSpawnerNodeRef(nodeRef, spawnerId) {
        return false;
    }
    this.BwmsActivateSpawner(spawnerId);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsDeactivateSpawnerByNodeRef(nodeRef: NodeRef) -> Bool {
    let spawnerId: Uint64;
    if !BwmsResolveSpawnerNodeRef(nodeRef, spawnerId) {
        return false;
    }
    this.BwmsDeactivateSpawner(spawnerId);
    return true;
}

@addMethod(CommunitySystem)
public func BwmsResetSpawnerByNodeRef(nodeRef: NodeRef) -> Bool {
    let spawnerId: Uint64;
    if !BwmsResolveSpawnerNodeRef(nodeRef, spawnerId) {
        return false;
    }
    this.BwmsResetSpawner(spawnerId);
    return true;
}

// Getters, só leitura: `BwmsGetCommunity`/
// `BwmsGetSpawner` devolvem o ponteiro `instance` cru do `WeakPtr<T>`, 0 = indefinido.
@addMethod(CommunitySystem)
public func BwmsGetCommunityByNodeRef(nodeRef: NodeRef) -> Uint64 {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return 0ul;
    }
    return this.BwmsGetCommunity(EntityID.ToHash(communityId));
}

@addMethod(CommunitySystem)
public func BwmsGetSpawnerByNodeRef(nodeRef: NodeRef) -> Uint64 {
    let spawnerId: Uint64;
    if !BwmsResolveSpawnerNodeRef(nodeRef, spawnerId) {
        return 0ul;
    }
    return this.BwmsGetSpawner(spawnerId);
}

// --- núcleo de resolução, compartilhado pelos 9 wrappers acima. Espelha byte-a-byte a lógica
// real de `Red::ResolveNodeRef`/`Red::ResolveEntityID` (WorldStateSystem.cpp linhas 41/57/68/
// etc.): resolve o NodeRef contra a raiz global (mesmo idioma usado em dezenas de `.script`
// vanilla reais, ex. `stimBroadcasterComponent.script:306`), falha limpo (false/EntityID
// indefinido) se o NodeRef não resolver — NUNCA chama a mutação de vtable com um ID inválido.
func BwmsResolveCommunityNodeRef(nodeRef: NodeRef, out communityId: EntityID) -> Bool {
    if !IsNodeRefDefined(nodeRef) {
        return false;
    }
    let resolved: GlobalNodeRef = ResolveNodeRef(nodeRef, Cast<GlobalNodeRef>(GlobalNodeID.GetRoot()));
    if !GlobalNodeRef.IsDefined(resolved) {
        return false;
    }
    communityId = Cast<EntityID>(resolved);
    return true;
}

func BwmsResolveSpawnerNodeRef(nodeRef: NodeRef, out spawnerId: Uint64) -> Bool {
    let communityId: EntityID;
    if !BwmsResolveCommunityNodeRef(nodeRef, communityId) {
        return false;
    }
    spawnerId = EntityID.ToHash(communityId);
    return true;
}
