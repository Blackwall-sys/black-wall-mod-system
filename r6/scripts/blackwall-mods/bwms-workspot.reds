// bwms-workspot.reds — a instância de workspot de um ator: a pose ou animação scriptada em que o NPC está (sentado, encostado, trabalhando).
// Catálogo Codeware `#82`, 2026-09-14.
//
// A referência devolve `ref<workWorkspotInstance>`, tipo que existe no motor mas não é visível ao redscript desta build. O BWMS entrega os campos pelo
// `EntityID` do ator, lidos do `InstanceMap` do `workWorkspotSystem` sob a trava dele; o corpo mora no Rust (`register::register_c82_natives`).

// O ator está numa instância de workspot agora?
public native func BwmsWorkspotHasInstance(actorID: EntityID) -> Bool

// Animação corrente da instância (`None` sem instância).
public native func BwmsWorkspotAnimName(actorID: EntityID) -> CName

// Animação ociosa da instância.
public native func BwmsWorkspotIdleAnim(actorID: EntityID) -> CName

// Id da entrada da árvore do workspot (`workWorkEntryId`).
public native func BwmsWorkspotEntryId(actorID: EntityID) -> Uint32

// Hash do caminho do recurso `.workspot` da instância.
public native func BwmsWorkspotResourcePath(actorID: EntityID) -> Uint64
