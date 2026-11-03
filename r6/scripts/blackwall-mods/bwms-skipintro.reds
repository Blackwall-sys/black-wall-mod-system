// BWMS — pular o boot: os logos e a tela de engagement.
// Divisão nativo↔redscript:
//  - LOGOS: o open do bink falha no nativo (`bink_open_replacement`, inicio/boot.rs) e o jogo pula o vídeo.
//  - ENGAGEMENT ("APERTE [espaço] PARA CONTINUAR"), 3 níveis no seletor "Pular boot" (kind 3), gravados no `bwms.toml`
//    (`pular_intro`, `continuar_automatico`) e valendo no próximo boot:
//    · nível 0 "Desligado": `pular_intro = false` — logos e tela "APERTE ESPAÇO" de volta.
//    · nível 1 "Até o menu" e nível 2 "Até a gameplay" usam o MESMO lever —
//      timer-anim 8s → BwmsFireStart() (native escreve [SM+0xd4]=2, gated pular_intro,
//      guardas state==1/phase==1/1x) → reload de content REAL → save-system ativa de verdade
//      (CONTINUAR + lista de saves funcionam nos dois níveis). A diferença entre 1 e 2 é só se
//      `bwms-autocontinue.reds` carrega o save sozinho depois (nível 2) ou para no menu com os
//      saves prontos pro usuário escolher (nível 1) — ver BwmsTryContinue.
//    · Dispensar a tela por evento (`ShowEngagementScreen{show=false}`) chega no menu sem input, mas o save-system não ativa (sem
//      CONTINUAR, sem lista de saves): só o input/lever real ativa. Por isso o lever.
//  - TOGGLE: BwmsSkipIntroOn/Off/State (`pular_intro` no `bwms.toml`).
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
  // BwmsFireStartState vale pular_intro (bwms.toml ou ~/.bwms-skipintro): com o skip ligado o splash cobre esta tela e o lever a dispensa.
  let lever: Bool = BwmsFireStartState();
  if lever {
    this.BwmsStartTimer(8.0, n"OnBwmsFireStart");
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
