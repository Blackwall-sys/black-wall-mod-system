// bwms-communitysystem.reds — os 9 métodos do `Raw::CommunitySystem` do Codeware (`Red/CommunitySystem.hpp`, item #77) sobre o
// `CommunitySystem` vanilla (`GameInstance.GetCommunitySystem(game)`, `orphans.script:11401`). São slots de VTABLE (`RawVFunc`, offset
// Windows em bytes), chamados no Mac com o deslocamento Itanium de +0x08:
//
//   Raw::CommunitySystem::ActivateCommunity   = RawVFunc<0x1B0, void(uint64_t, CName)>
//   Raw::CommunitySystem::DeactivateCommunity = RawVFunc<0x1B8, void(uint64_t, CName)>
//   Raw::CommunitySystem::SetCommunityPhase   = RawVFunc<0x1C0, void(uint64_t, CName, CName)>
//   Raw::CommunitySystem::ResetCommunity      = RawVFunc<0x1C8, void(uint64_t, CName)>
//   Raw::CommunitySystem::ActivateSpawner     = RawVFunc<0x1D8, void(uint64_t)>
//   Raw::CommunitySystem::DeactivateSpawner   = RawVFunc<0x1E0, void(uint64_t)>
//   Raw::CommunitySystem::ResetSpawner        = RawVFunc<0x1E8, void(uint64_t)>
//
// `GetCommunity`(0x228)/`GetSpawner`(0x260) devolvem o ponteiro `instance` do `WeakPtr<T>` como `Uint64` (0 = não achado), sem forjar
// `Community`/`Spawner` nem `wref<T>`. Os ids são o hash cru do `EntityID`/`gameCommunityID`, passados como `EntityID`/`Uint64`, sem
// forjar o wrapper `gameCommunityID`.
//
// As 7 mutações mudam estado real de mundo (ativam e desativam spawn de NPC e multidão) e não chamam o `CommunitySystem::Update` que o
// Codeware chama depois de cada uma; a API completa, com o `Update`, é o `WorldStateSystem` (`ponte-worldstatesystem.reds`).

native func BwmsCommunityActivate(sys: ref<CommunitySystem>, communityId: EntityID, entryName: CName) -> Void;
native func BwmsCommunityDeactivate(sys: ref<CommunitySystem>, communityId: EntityID, entryName: CName) -> Void;
native func BwmsCommunityReset(sys: ref<CommunitySystem>, communityId: EntityID, entryName: CName) -> Void;
native func BwmsCommunitySetPhase(sys: ref<CommunitySystem>, communityId: EntityID, entryName: CName, phaseName: CName) -> Void;
native func BwmsCommunitySpawnerActivate(sys: ref<CommunitySystem>, spawnerId: Uint64) -> Void;
native func BwmsCommunitySpawnerDeactivate(sys: ref<CommunitySystem>, spawnerId: Uint64) -> Void;
native func BwmsCommunitySpawnerReset(sys: ref<CommunitySystem>, spawnerId: Uint64) -> Void;
native func BwmsCommunityGetCommunity(sys: ref<CommunitySystem>, communityId: Uint64) -> Uint64;
native func BwmsCommunityGetSpawner(sys: ref<CommunitySystem>, spawnerId: Uint64) -> Uint64;

@addMethod(CommunitySystem)
public func BwmsActivateCommunity(communityId: EntityID, entryName: CName) -> Void {
    BwmsCommunityActivate(this, communityId, entryName);
}
@addMethod(CommunitySystem)
public func BwmsDeactivateCommunity(communityId: EntityID, entryName: CName) -> Void {
    BwmsCommunityDeactivate(this, communityId, entryName);
}
@addMethod(CommunitySystem)
public func BwmsResetCommunity(communityId: EntityID, entryName: CName) -> Void {
    BwmsCommunityReset(this, communityId, entryName);
}
@addMethod(CommunitySystem)
public func BwmsSetCommunityPhase(communityId: EntityID, entryName: CName, phaseName: CName) -> Void {
    BwmsCommunitySetPhase(this, communityId, entryName, phaseName);
}
@addMethod(CommunitySystem)
public func BwmsActivateSpawner(spawnerId: Uint64) -> Void {
    BwmsCommunitySpawnerActivate(this, spawnerId);
}
@addMethod(CommunitySystem)
public func BwmsDeactivateSpawner(spawnerId: Uint64) -> Void {
    BwmsCommunitySpawnerDeactivate(this, spawnerId);
}
@addMethod(CommunitySystem)
public func BwmsResetSpawner(spawnerId: Uint64) -> Void {
    BwmsCommunitySpawnerReset(this, spawnerId);
}
@addMethod(CommunitySystem)
public func BwmsGetCommunity(communityId: Uint64) -> Uint64 {
    return BwmsCommunityGetCommunity(this, communityId);
}
@addMethod(CommunitySystem)
public func BwmsGetSpawner(spawnerId: Uint64) -> Uint64 {
    return BwmsCommunityGetSpawner(this, spawnerId);
}
