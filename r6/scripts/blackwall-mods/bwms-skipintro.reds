// BWMS — skip do boot (camada DEV, fonte canônica em mods-research/blackwall-mods-dev/).
// Divisão nativo↔redscript:
//  - LOGOS: bink-skip nativo (falha o open) em selfboot.rs.
//  - ENGAGEMENT ("APERTE [espaço] PARA CONTINUAR"), 3 níveis no seletor "Pular boot" (kind 3):
//    · nível 0 "Desligado": nada acontece, boot nativo normal.
//    · nível 1 "Até o menu" (skipintro ON, fire-start ON, autocontinue OFF) e
//      nível 2 "Até a gameplay" (skipintro+fire-start+autocontinue ON) usam o MESMO lever —
//      timer-anim 8s → BwmsFireStart() (native escreve [SM+0xd4]=2, gated ~/.bwms-fire-start,
//      guardas state==1/phase==1/1x) → reload de content REAL → save-system ativa de verdade
//      (CONTINUAR + lista de saves funcionam nos dois níveis). A diferença entre 1 e 2 é só se
//      `bwms-autocontinue.reds` carrega o save sozinho depois (nível 2) ou para no menu com os
//      saves prontos pro usuário escolher (nível 1) — ver BwmsTryContinue.
//    · ACHADO 2026-07-12 (Perrotta testando): a versão ANTERIOR do nível 1 despachava
//      ShowEngagementScreen{show=false} direto (dismiss por evento, sem passar pelo lever) —
//      chegava no menu com zero input, MAS o save-system NUNCA ativava (sem CONTINUAR, sem
//      lista de saves), porque esse dismiss não é o mesmo caminho que ativa o save-system (só
//      o INPUT/lever real faz isso, achado já documentado desde 2026-07-05). Trocado pro lever
//      unificado acima — agora nível 1 tem saves funcionando também.
//  - TOGGLE: BwmsSkipIntroOn/Off/State (persiste em ~/.bwms-skipintro).
// Timer = anim-proxy (o DelaySystem NÃO ticka no pregame): padrão de hackingMinigameUtils.script.

native func BwmsEngagementOn() -> Bool;
native func BwmsEngagementOff() -> Bool;
native func BwmsSkipIntroOn() -> Bool;
native func BwmsSkipIntroOff() -> Bool;
native func BwmsSkipIntroState() -> Bool;
native func BwmsFireStart() -> Bool;
native func BwmsFireStartState() -> Bool;

@addField(EngagementScreenGameController) let m_bwmsTimerProxy: ref<inkAnimProxy>;

@addMethod(EngagementScreenGameController)
private final func BwmsStartTimer(seconds: Float, callback: CName) -> Void {
  let def: ref<inkAnimDef> = new inkAnimDef();
  let interp: ref<inkAnimTranslation> = new inkAnimTranslation();
  interp.SetDuration(seconds);
  def.AddInterpolator(interp);
  if IsDefined(this.m_bwmsTimerProxy) {
    this.m_bwmsTimerProxy.UnregisterFromAllCallbacks(inkanimEventType.OnFinish);
  };
  this.m_bwmsTimerProxy = this.GetRootWidget().PlayAnimation(def);
  this.m_bwmsTimerProxy.RegisterToCallback(inkanimEventType.OnFinish, this, callback);
}

// Fallback SEM saves (mesmo efeito do SPACE — preGameScenarios.OnHandleEngagementScreen(show=false)):
// só é alcançado se skipintro estiver ON com fire-start OFF, combinação que o seletor da UI (kind 3,
// bwms-settings-poc.reds) não produz mais (nível 1 e 2 ligam fire-start junto) — mantido como
// rede de segurança defensiva, não é mais um nível do seletor.
@addMethod(EngagementScreenGameController)
protected cb func OnBwmsDismiss(e: ref<inkAnimProxy>) -> Bool {
  let evt: ref<ShowEngagementScreen>;
  if IsDefined(this.m_menuEventDispatcher) {
    evt = new ShowEngagementScreen();
    evt.show = false;
    this.m_menuEventDispatcher.SpawnEvent(n"OnHandleEngagementScreen", evt);
  };
  return true;
}

// Níveis 1 e 2: o lever zero-input (ativa o save-system de verdade). Guardas ficam no lado
// nativo (re-disparo é inócuo). BwmsTryContinue (bwms-autocontinue.reds) decide depois se
// carrega sozinho (nível 2) ou para no menu com os saves prontos (nível 1).
@addMethod(EngagementScreenGameController)
protected cb func OnBwmsFireStart(e: ref<inkAnimProxy>) -> Bool {
  BwmsFireStart();
  return true;
}

@wrapMethod(EngagementScreenGameController)
protected cb func OnInitialize() -> Bool {
  let r: Bool = wrappedMethod();
  BwmsEngagementOn();
  if BwmsSkipIntroState() {
    if BwmsFireStartState() {
      this.BwmsStartTimer(8.0, n"OnBwmsFireStart");
    } else {
      this.BwmsStartTimer(6.0, n"OnBwmsDismiss");
    };
  };
  return r;
}
@wrapMethod(EngagementScreenGameController)
protected cb func OnUninitialize() -> Bool {
  BwmsEngagementOff();
  return wrappedMethod();
}

@wrapMethod(InitializeUserScreenGameController)
protected cb func OnInitialize() -> Bool {
  let r: Bool = wrappedMethod();
  BwmsEngagementOn();
  return r;
}
@wrapMethod(InitializeUserScreenGameController)
protected cb func OnUninitialize() -> Bool {
  BwmsEngagementOff();
  return wrappedMethod();
}
