// Codeware #20 — módulo `Device/`: reset de rede de sistema de segurança.
//
// O tipo `ResetSecuritySystemNetwork` não existe no RTTI desta build: o Codeware o declara `native` porque o C++ dele registra a
// classe. Aqui não precisa ser native — `Event` é `importonly class Event extends IScriptable` no corpus vanilla, que tem 1317
// subclasses de script puras de `Event`, e a classe de evento do BWMS é mais uma.
//
// O resto do módulo é redscript sobre classe VANILLA: `SecuritySystemControllerPS` é
// `public class ... extends DeviceSystemBaseControllerPS`, então `@addMethod` nela é risco zero —
// a mesma categoria usada em dezenas de extensões deste projeto.
//
// NOMES: API de framework de TERCEIRO leva nome do BWMS. O handler tem que casar o nome da classe
// de evento (o despacho do redscript é por `On<NomeDaClasse>`), daí `OnBwmsResetSecuritySystemNetwork`.

public class BwmsResetSecuritySystemNetwork extends Event {}

// O núcleo: zera o attitude group e reatribui, forçando o motor a reprocessar a rede.
@addMethod(SecuritySystemControllerPS)
public final func BwmsReinitSecurityAttitudeGroup() -> Void {
  let attitudeGroup: TweakDBID = this.m_attitudeGroup;
  this.m_attitudeGroup = TDBID.None();
  this.SetSecuritySystemAttitudeGroup(attitudeGroup);
}

// Enfileira um evento na ENTIDADE de cada agente da rede (e opcionalmente na própria).
@addMethod(SecuritySystemControllerPS)
public func BwmsQueueSecuritySystemEvent(evt: ref<Event>, opt paraSi: Bool) -> Void {
  if paraSi {
    this.QueueEntityEvent(PersistentID.ExtractEntityID(this.GetID()), evt);
  };
  if IsDefined(this.m_agentsRegistry) {
    for agent in this.m_agentsRegistry.GetAgents() {
      this.QueueEntityEvent(PersistentID.ExtractEntityID(this.GetPS(agent.link).GetID()), evt);
    };
  };
}

// Idem, mas no PERSISTENT STATE de cada agente (é o que as ações de quest consomem).
@addMethod(SecuritySystemControllerPS)
public func BwmsQueueSecuritySystemPSEvent(evt: ref<Event>, opt paraSi: Bool) -> Void {
  if paraSi {
    this.QueuePSEvent(this, evt);
  };
  if IsDefined(this.m_agentsRegistry) {
    for agent in this.m_agentsRegistry.GetAgents() {
      this.QueuePSEvent(this.GetPS(agent.link), evt);
    };
  };
}

// O handler do evento — a sequência completa de reset, na ordem que o módulo original usa.
@addMethod(SecuritySystemControllerPS)
public func OnBwmsResetSecuritySystemNetwork(evt: ref<BwmsResetSecuritySystemNetwork>) -> EntityNotificationType {
  if !this.m_agentsRegistry.IsReady() {
    this.CompileSecurityAgentRegistry();
  };
  this.BwmsQueueSecuritySystemPSEvent(this.ActionQuestForceDisabled());
  this.BwmsQueueSecuritySystemPSEvent(this.ActionQuestResetDeviceToInitialState());
  this.BwmsReinitSecurityAttitudeGroup();
  this.BwmsQueueSecuritySystemPSEvent(this.ActionQuestRestoreQuickHacks());
  this.BwmsQueueSecuritySystemPSEvent(this.ActionQuestForceEnabled());
  this.OnQuestForceON(this.ActionQuestForceON());
  this.BwmsQueueSecuritySystemPSEvent(this.ActionQuestResetPerfomedActionsStorage(), true);
  return EntityNotificationType.SendThisEventToEntity;
}
