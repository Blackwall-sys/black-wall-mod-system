// BWMS — AUTO-CONTINUE: pular ATÉ O JOGO, 100% redscript (sem injeção, sem Acessibilidade).
// Gate = ESPELHO do QuickLoad real do jogo (singleplayerMenu.script:864-871):
// m_savesReady && metadata(saveIndex 0) && m_savesCount > 0. NÃO usar HasLastCheckpoint():
// contradiz os campos do controller e nunca dispara (bug diagnosticado 2026-07-04 via [ac-dbg]).
// Load replica o clique: LoadModdedSave(0) p/ save modado, senão LoadLastCheckpoint(false) —
// a via não-modada num save modado abre o diálogo que trava.

native func BwmsAutoContinue() -> Bool;
native func BwmsAutoContinueDoBoot() -> Bool;
native func BwmsAcFired() -> Bool;
native func BwmsMenuReadySplashOff() -> Bool;

// TENTATIVAS DE SINAL PRECISO PRA "carregamento terminou de verdade" — AMBAS REFUTADAS
// in-game 2026-07-12, deixadas aqui como nota pra não reinventar:
// 1. blackboard FastTRavelSystem.FastTravelLoadingScreenFinished (o mesmo que
//    fastTravelSystem.script usa) — NUNCA disparou pro nosso load. É específico de requests
//    REAIS do FastTravelSystem (GetFastTravelSystem().QueueRequest(...)); só pedir o
//    loading-type visual FastTravel (o que fazemos) não é o bastante pra alimentar esse sinal.
// 2. PlayerPuppet.OnGameAttached (gameObject.script) — dispara CEDO DEMAIS: existe um
//    PlayerPuppet placeholder/preview no menu que TAMBÉM aciona esse callback (mesmo
//    "player espúrio" já documentado no projeto pra current_player()).
// O splash sai no fim do load: 3ª leitura de phase 5 e o executor assentado
// (`splash_fora_quando_o_executor_assenta`, inicio/boot.rs).
// Se algum dia achar o sinal certo: o candidato mais promissor não testado ainda é o
// HUD ficar visível (hudCoreGameController ou similar).

@addField(SingleplayerMenuGameController) let m_bwmsContinued: Bool;
@addField(SingleplayerMenuGameController) let m_bwmsMetaReady: Bool;
@addField(SingleplayerMenuGameController) let m_bwmsMetaInvalid: Bool;

// Índice do save que o auto-continue abre, na ordem do `saves` de `OnSavesForLoadReady` (o mais recente primeiro, a mesma lista do
// menu): 0 no build público. No de desenvolvimento, o marcador `~/.bwms-autocontinue-saveindex` escolhe outro, pra comparar a duração
// do boot por região do mapa.
native func BwmsAutoContinueSaveIndex() -> Int32;

@addMethod(SingleplayerMenuGameController)
public func BwmsDoContinue() -> Void {
  BwmsAcFired();
  // A tela nativa pós-load "APERTE [espaço] PARA CONTINUAR" (≠ engagement do boot, sem controller
  // redscript) só aparece no loading-type padrão. FastTravel pula direto pro jogo — mesmo truque de
  // worldMap.script:937-939 / pauseMenu.script:163-164. Sem isto, o zero-input trava aqui esperando
  // SPACE humano (achado 2026-07-12: phase=5/autocontinue-disparou no log NÃO prova isso sozinho).
  let nextLoadingTypeEvt: ref<inkSetNextLoadingScreenEvent> = new inkSetNextLoadingScreenEvent();
  nextLoadingTypeEvt.SetNextLoadingScreenType(inkLoadingScreenType.FastTravel);
  this.QueueBroadcastEvent(nextLoadingTypeEvt);
  // `m_isModded` é metadado do save (gravado quando o save foi criado), não o estado da sessão. Com índice 0 vale a escolha vanilla
  // entre as duas APIs; índice diferente de 0 vai sempre por `LoadModdedSave`, a única que aceita índice.
  let idx: Int32 = BwmsAutoContinueSaveIndex();
  if idx < 0 || idx >= this.m_savesCount {
    idx = 0;
  };
  if idx != 0 {
    Print("[autocontinue] LoadModdedSave(idx=" + ToString(idx) + " de " + ToString(this.m_savesCount) + " saves, override de índice, m_isModded=" + ToString(this.m_isModded) + ")");
    this.LoadModdedSave(idx);
  } else if this.m_isModded {
    Print("[autocontinue] LoadModdedSave(idx=0, comportamento default)");
    this.LoadModdedSave(0);
  } else {
    this.GetSystemRequestsHandler().LoadLastCheckpoint(false);
  };
}

// Nível 1 ("Até o menu" com saves): o lever (bwms-skipintro.reds) já ativou o save-system de
// verdade, mas BwmsAutoContinue()==false, então NÃO carrega sozinho — só desliga a splash (senão
// ela fica acesa pra sempre: sem auto-load não existe phase==5, de onde vem o fim do load que
// tira a splash em inicio/boot.rs) e deixa o usuário escolher CONTINUAR/CARREGAR JOGO manualmente.
@addMethod(SingleplayerMenuGameController)
public func BwmsTryContinue() -> Void {
  if this.m_bwmsContinued { return; };
  // Sem save, ou com o save 0 inválido, não há o que continuar: o menu é o fim da tarefa. Sem isto o splash só sairia pelo watchdog, por cima do menu.
  if this.m_savesReady && (this.m_savesCount == 0 || this.m_bwmsMetaInvalid) {
    BwmsMenuReadySplashOff();
    return;
  };
  if !(this.m_savesReady && this.m_bwmsMetaReady && this.m_savesCount > 0) { return; };
  // A native é a variante do boot: só carrega sozinho enquanto o boot não resolveu a tarefa dele no processo; num menu seguinte o
  // load não é automático, porque sairia sem splash (`tramp_autocontinue_do_boot`, inicio/natives.rs).
  let auto: Bool = BwmsAutoContinueDoBoot();
  if auto {
    this.m_bwmsContinued = true;
    this.BwmsDoContinue();
  } else {
    BwmsMenuReadySplashOff();
  };
}

// isModded conhecido (metadados do último checkpoint, saveIndex 0)
@wrapMethod(SingleplayerMenuGameController)
protected cb func OnSaveMetadataReady(info: ref<SaveMetadataInfo>) -> Bool {
  let r: Bool = wrappedMethod(info);
  if info.saveIndex == 0 {
    if info.isValid {
      this.m_bwmsMetaReady = true;
    } else {
      this.m_bwmsMetaInvalid = true;
    };
  };
  this.BwmsTryContinue();
  return r;
}

// saves prontos + menu construído (o wrapped seta m_savesReady/m_savesCount antes do Try)
@wrapMethod(SingleplayerMenuGameController)
protected cb func OnSavesForLoadReady(saves: [String]) -> Bool {
  let r: Bool = wrappedMethod(saves);
  this.BwmsTryContinue();
  return r;
}
