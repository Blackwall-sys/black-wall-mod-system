// BWMS — AUTO-CONTINUE: pular ATÉ O JOGO, 100% redscript (sem injeção, sem Acessibilidade).
// Gate = ESPELHO do QuickLoad real do jogo (singleplayerMenu.script:864-871):
// m_savesReady && metadata(saveIndex 0) && m_savesCount > 0. NÃO usar HasLastCheckpoint():
// contradiz os campos do controller e nunca dispara (bug diagnosticado 2026-07-04 via [ac-dbg]).
// Load replica o clique: LoadModdedSave(0) p/ save modado, senão LoadLastCheckpoint(false) —
// a via não-modada num save modado abre o diálogo que trava.
// BwmsDbgTry(flags): bitmask do gate no log (1=savesReady, 2=metaReady, 4=count>0) — 7 = dispara.

native func BwmsAutoContinue() -> Bool;
native func BwmsAcFired() -> Bool;
native func BwmsDbgTry(flags: Int32) -> Bool;
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
// Sem um sinal confiável achado, a splash sai por REDE DE SEGURANÇA (grace period após
// phase==5 em selfboot.rs) — empiricamente validado (2 boots reais via steam://, tela certa).
// Se algum dia achar o sinal certo: o candidato mais promissor não testado ainda é o
// HUD ficar visível (hudCoreGameController ou similar).

@addField(SingleplayerMenuGameController) let m_bwmsContinued: Bool;
@addField(SingleplayerMenuGameController) let m_bwmsMetaReady: Bool;

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
  if this.m_isModded {
    this.LoadModdedSave(0);
  } else {
    this.GetSystemRequestsHandler().LoadLastCheckpoint(false);
  };
}

// Nível 1 ("Até o menu" com saves): o lever (bwms-skipintro.reds) já ativou o save-system de
// verdade, mas BwmsAutoContinue()==false, então NÃO carrega sozinho — só desliga a splash (senão
// ela fica acesa pra sempre: sem auto-load não existe phase==5, que é o gatilho normal do
// grace-period em selfboot.rs) e deixa o usuário escolher CONTINUAR/CARREGAR JOGO manualmente.
@addMethod(SingleplayerMenuGameController)
public func BwmsTryContinue() -> Void {
  // DIAGNÓSTICO TEMPORÁRIO (2026-07-18, sessão full-body): 4 boots seguidos hoje NÃO dispararam o
  // autocontinue (0 linhas "[autocontinue] disparou") — achado novo, nunca visto antes nesta
  // sessão. Print isolado (1 native só, sem chamada consecutiva) loga os 3 valores do gate +
  // se já continuou, TODA VEZ que este método roda, pra achar qual precondição está falhando.
  let contFlag: Bool = this.m_bwmsContinued;
  let savesReadyFlag: Bool = this.m_savesReady;
  let metaReadyFlag: Bool = this.m_bwmsMetaReady;
  let countVal: Int32 = this.m_savesCount;
  Print("[ac-diag] cont=" + ToString(contFlag) + " savesReady=" + ToString(savesReadyFlag) + " metaReady=" + ToString(metaReadyFlag) + " count=" + ToString(countVal));
  if this.m_bwmsContinued { return; };
  if !(this.m_savesReady && this.m_bwmsMetaReady && this.m_savesCount > 0) { return; };
  // BUG DE MARSHALLING RESOLVIDO (2026-07-16): o `BwmsDbgTry(flags)` que ficava AQUI (native de
  // diagnóstico) era a 1ª de 2 natives consecutivas — corrompia o marshalling do retorno da 2ª
  // (`BwmsAutoContinue()` logo abaixo), que vinha false de forma boot-dependente → boot parava no
  // menu. Removido o BwmsDbgTry: agora `BwmsAutoContinue()` é a ÚNICA native do método → retorno
  // determinístico. (O diagnóstico dos flags saiu; o gate acima já garante os pré-requisitos.)
  // BUG DO AUTOCONTINUE RACY (2026-07-16, causa-raiz REAL): `m_bwmsContinued=true` estava sendo
  // setado AQUI, ANTES de checar `BwmsAutoContinue()`. Se a leitura viesse racy-false uma vez (a
  // native às vezes marshalha o retorno errado logo após outra native), o mod TRAVAVA no menu pra
  // sempre — o guard `if m_bwmsContinued return` no topo bloqueava toda re-tentativa nos gatilhos
  // seguintes (OnSavesForLoadReady). FIX: só marcar `continued` DEPOIS de REALMENTE continuar (dentro
  // do `if auto`), pra os 2+ gatilhos poderem re-tentar até a leitura vir true. `auto=false` genuíno
  // (modo "só menu") continua parando no menu corretamente (todos os gatilhos leem false).
  let auto: Bool = BwmsAutoContinue();
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
  // DIAGNÓSTICO TEMPORÁRIO (2026-07-18): confirma que o wrap É CHAMADO PELO MOTOR + os valores
  // reais de saveIndex/isValid que chegam (isolado do Print seguinte, sem native consecutiva).
  let idx: Int32 = info.saveIndex;
  let valid: Bool = info.isValid;
  Print("[ac-diag] OnSaveMetadataReady chamado: saveIndex=" + ToString(idx) + " isValid=" + ToString(valid));
  if info.saveIndex == 0 && info.isValid {
    this.m_bwmsMetaReady = true;
  };
  this.BwmsTryContinue();
  return r;
}

// saves prontos + menu construído (o wrapped seta m_savesReady/m_savesCount antes do Try)
@wrapMethod(SingleplayerMenuGameController)
protected cb func OnSavesForLoadReady(saves: [String]) -> Bool {
  let r: Bool = wrappedMethod(saves);
  // DIAGNÓSTICO TEMPORÁRIO (2026-07-18): confirma que o wrap É CHAMADO + o nº real de saves.
  let n: Int32 = ArraySize(saves);
  Print("[ac-diag] OnSavesForLoadReady chamado: ArraySize(saves)=" + ToString(n));
  this.BwmsTryContinue();
  return r;
}
