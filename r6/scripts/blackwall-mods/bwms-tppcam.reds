// BWMS — Skill 1: VER-O-V (câmera 3ª pessoa autônoma), sem HID/Acessibilidade.
// [bisect save-load 2026-07-15] versão SEM trigger — só natives + as 4 classes DelayCallback.
// O start dos pollers (que estava em @wrapMethod) foi removido pra isolar se as CLASSES/NATIVES
// (footprint de load-time) crasham o world-load do save-load, ou se era só o trigger.

native func BwmsTppState() -> Int32;
native func BwmsCamBack() -> Int32;
native func BwmsCamX() -> Int32;
native func BwmsCamZ() -> Int32;
native func BwmsForceLook() -> Int32;
native func BwmsEquipState() -> Int32;
// BUG REAL achado 2026-07-17: este arquivo usa Print(...) (linhas de diagnóstico) mas NUNCA declarou
// `native func Print` — o bundle shipado compilava por acidente porque um .reds de TESTE/exemplo
// (fora do blackwall-mods-dev canônico) declarava Print e ficava compilado JUNTO. Sem esse arquivo
// externo presente, `scc -compile` no bundle oficial sozinho FALHA ("function 'Print' not found").
// Declarado aqui pra o bundle canônico compilar standalone, sem depender de nenhum arquivo externo.
native func Print(text: String) -> Void;

public class BwmsCamPoller extends DelayCallback {
  let m_game: GameInstance;

  public func Call() -> Void {
    let maxY: Float = Cast<Float>(BwmsCamBack()) / 100.0;
    let maxZ: Float = Cast<Float>(BwmsCamZ()) / 100.0;
    let player: ref<PlayerPuppet> = GameInstance.GetPlayerSystem(this.m_game).GetLocalPlayerControlledGameObject() as PlayerPuppet;
    if IsDefined(player) {
      let cam: ref<FPPCameraComponent> = player.GetFPPCameraComponent();
      if IsDefined(cam) {
        let pitch: Float = Matrix.GetRotation(cam.GetLocalToWorld()).Pitch;
        let startP: Float = -8.0;
        let fullP: Float = -60.0;
        let downF: Float = ClampF((startP - pitch) / (startP - fullP), 0.0, 1.0);
        cam.SetLocalPosition(new Vector4(0.0, maxY * downF, maxZ * downF, 1.0));
      };
    };
    // ACHADO 2026-07-15: reagendar um objeto NOVO (`new BwmsCamPoller()`) a cada Call() fazia o
    // poller disparar 1x e nunca mais — ao contrário do padrão REAL do motor
    // (`TemporalPrereqDelayCallback`, core/gameplay/prereqs/temporalPrereq.script), que reusa a
    // MESMA instância indefinidamente. FIX: reagenda `this` (mesmo objeto, campos já atuais).
    let ds: ref<DelaySystem> = GameInstance.GetDelaySystem(this.m_game);
    if IsDefined(ds) {
      ds.DelayCallback(this, 0.03);
    };
  }
}
native func BwmsEquipLog(step: Int32) -> Bool;
native func BwmsInvState() -> Int32;
native func BwmsEquipCheck() -> Int32;
native func BwmsEquipReadback(area: Int32, id: TweakDBID) -> Bool;
native func BwmsPollerTick(label: String, n: Int32) -> Void;

public class BwmsInvPoller extends DelayCallback {
  let m_game: GameInstance;
  let m_last: Int32;

  public func Call() -> Void {
    let want: Int32 = BwmsInvState();
    if want != this.m_last {
      if want == 1 {
        let ev: ref<inkMenuInstance_SpawnEvent> = new inkMenuInstance_SpawnEvent();
        ev.Init(n"OnSwitchToInventory");
        GameInstance.GetUISystem(this.m_game).QueueEvent(ev);
      };
      this.m_last = want;
    };
    // FIX 2026-07-15 (mesmo achado do BwmsCamPoller): reagenda `this`, não um objeto novo.
    let ds: ref<DelaySystem> = GameInstance.GetDelaySystem(this.m_game);
    if IsDefined(ds) {
      ds.DelayCallback(this, 0.3);
    };
  }
}

public class BwmsEquipPoller extends DelayCallback {
  let m_game: GameInstance;
  let m_last: Int32;
  let m_lastCheck: Int32;
  let m_calls: Int32;

  private func ItemFor(sel: Int32) -> TweakDBID {
    if sel == 1 { return t"Items.GOG_DLC_Jacket_Legendary"; };
    if sel == 2 { return t"Items.Fixer_01_Set_TShirt"; };
    if sel == 3 { return t"Items.Coat_04_rich_02_Crafting"; };
    return TDBID.None();
  }

  public func Call() -> Void {
    // CONTADOR (2026-07-15): registra a contagem EXATA de ciclos a cada 20 chamadas, pra correlacionar
    // o instante do crash por CICLO (não só tempo de parede) na próxima bisecção. O `/tmp/cp77-console.log`
    // continua no disco mesmo se o processo crashar logo depois — a última linha antes do crash dá o
    // nº de ciclos exato que o BwmsEquipPoller alcançou (a 0.3s/ciclo, ~1min55s ~= 383 ciclos esperados
    // se for tempo puro; se o nº real bater sempre no mesmo valor entre boots, confirma "por-ciclo").
    this.m_calls += 1;
    if this.m_calls % 20 == 0 {
      BwmsPollerTick("equip", this.m_calls);
    };
    let want: Int32 = BwmsEquipState();
    let player: ref<GameObject> = GameInstance.GetPlayerSystem(this.m_game).GetLocalPlayerControlledGameObject();
    if IsDefined(player) && want != this.m_last {
      if want > 0 {
        BwmsEquipLog(1);
        let id: TweakDBID = this.ItemFor(want);
        if TDBID.IsValid(id) {
          BwmsEquipLog(2);
          let req: ref<EquipRequest> = new EquipRequest();
          req.itemID = ItemID.FromTDBID(id);
          req.owner = player;
          req.addToInventory = true;
          req.slotIndex = -1;
          let es: ref<EquipmentSystem> = EquipmentSystem.GetInstance(player);
          if IsDefined(es) {
            BwmsEquipLog(3);
            es.QueueRequest(req);
            BwmsEquipLog(4);
          };
        };
      };
      this.m_last = want;
    };
    let chk: Int32 = BwmsEquipCheck();
    if chk != this.m_lastCheck {
      if chk == 1 && IsDefined(player) {
        let ed: ref<EquipmentSystemPlayerData> = EquipmentSystem.GetData(player);
        if IsDefined(ed) {
          BwmsEquipReadback(1, ItemID.GetTDBID(ed.GetActiveItem(gamedataEquipmentArea.OuterChest)));
          BwmsEquipReadback(2, ItemID.GetTDBID(ed.GetActiveItem(gamedataEquipmentArea.InnerChest)));
          BwmsEquipReadback(3, ItemID.GetTDBID(ed.GetActiveItem(gamedataEquipmentArea.Legs)));
          BwmsEquipReadback(4, ItemID.GetTDBID(ed.GetActiveItem(gamedataEquipmentArea.Feet)));
        };
      };
      this.m_lastCheck = chk;
    };
    // FIX 2026-07-15 (mesmo achado do BwmsCamPoller): reagenda `this`, não um objeto novo.
    let ds: ref<DelaySystem> = GameInstance.GetDelaySystem(this.m_game);
    if IsDefined(ds) {
      ds.DelayCallback(this, 0.3);
    };
  }
}

public class BwmsTppPoller extends DelayCallback {
  let m_game: GameInstance;
  let m_last: Int32;
  let m_stable: Int32;

  public func Call() -> Void {
    let want: Int32 = BwmsTppState();
    let player: ref<GameObject> = GameInstance.GetPlayerSystem(this.m_game).GetLocalPlayerControlledGameObject();
    if IsDefined(player) {
      if this.m_stable < 1000 { this.m_stable += 1; };
      if this.m_stable % 20 == 0 {
        BwmsPollerTick("tpp", this.m_stable);
      };
      if want == 1 {
        // MANTER SUAVE (2026-07-15): o ActivateTPPRepresentation é TRANSIENTE em free-roam (o jogo
        // reverte pra FPP; persiste ~25s). Re-disparar a cada 0.5s INTERROMPE o Activate antes de aplicar
        // (fica sempre FPP). Então: dispara no 0→1 E re-estabelece a cada ~15s (m_stable%30, tick=0.5s) —
        // suave, deixa cada Activate aplicar + persistir, re-fixa antes de reverter. Guarda m_stable>=4
        // (~2s): os pollers já nascem PÓS world-load (dylib chama BwmsBootFullbody só em gameplay via callg).
        if this.m_stable >= 4 && (this.m_last != 1 || this.m_stable % 10 == 0) {
          player.QueueEvent(new ActivateTPPRepresentationEvent());
          this.m_last = 1;
        };
      } else {
        if this.m_last == 1 {
          player.QueueEvent(new DeactivateTPPRepresentationEvent());
        };
        this.m_last = 0;
      };
      // TESTE 2026-07-15 (BwmsForceLook, opt-in via ~/.bwms-forcelook): sem HID pra olhar pra baixo
      // de verdade, força a rotação da câmera FPP pra baixo + aplica o offset máximo — só pra
      // provar/refutar visualmente se o torso (anexado via ActivateTPPRepresentation) aparece.
      // Reusa BwmsCamBack/BwmsCamZ (já existentes) como o offset a aplicar no pitch forçado.
      if BwmsForceLook() == 1 {
        let pp: ref<PlayerPuppet> = player as PlayerPuppet;
        if IsDefined(pp) {
          let cam: ref<FPPCameraComponent> = pp.GetFPPCameraComponent();
          if IsDefined(cam) {
            let euler: EulerAngles;
            euler.Pitch = -60.0;
            euler.Yaw = 0.0;
            euler.Roll = 0.0;
            cam.SetLocalOrientation(EulerAngles.ToQuat(euler));
            let maxY: Float = Cast<Float>(BwmsCamBack()) / 100.0;
            let maxZ: Float = Cast<Float>(BwmsCamZ()) / 100.0;
            cam.SetLocalPosition(new Vector4(0.0, maxY, maxZ, 1.0));
          };
        };
      };
    };
    // FIX 2026-07-15 (mesmo achado do BwmsCamPoller): reagenda `this`, não um objeto novo.
    let ds: ref<DelaySystem> = GameInstance.GetDelaySystem(this.m_game);
    if IsDefined(ds) {
      ds.DelayCallback(this, 0.5);
    };
  }
}

// [FIX save-load 2026-07-15] NÃO wrapa NENHUMA classe do world-load (provado: wrapar PlayerPuppet OU
// BaseSubtitles corrompe o SystemsUpdater no world-load — o crash é da REGISTRAÇÃO do wrap linkando os
// pollers→ActivateTPPRepresentationEvent→TakeOverControlSystem, não da execução; stack 100% nativo).
// Em vez disso: função GLOBAL que o DYLIB chama (callg) quando detecta gameplay (pós-load). Pega o
// GameInstance via GetGameInstance() (não precisa de 'this' nem de wrapar classe). Chamável pelo canal:
// `callg BwmsBootFullbody`.
// ACHADO 2026-07-15: `GetGameInstance()` chamado de dentro desta função quando invocada via callg
// (call_func cru, ctx=null) devolvia uma GameInstance MORTA (GetDelaySystem/GetPlayerSystem
// retornavam undefined mesmo em gameplay real). FIX: o Rust agora passa a GameInstance JÁ
// RESOLVIDA (via PlayerPuppet.GetGame com ctx=player real) como ARGUMENTO — não conjuramos mais
// sozinhos aqui. Ver proofs/2026-07-15-callg-global-getgameinstance-dead-ACHADO.log.
// RECEITA REAL DO JB TPP MOD (2026-07-16): o corpo em pé de verdade NÃO vem só do
// ActivateTPPRepresentation — vem de ATIVAR o componente `tppCamera` nativo do player (é ele que
// bota o jogo no modo TPP real → corpo renderiza+anima em pé, sem IK comprimido). O mod faz:
// (1) ActivateTPPRepresentationEvent COM playerController setado; (2) FindComponentByName('tppCamera')
// .Activate(); (3) posiciona a câmera. Pro NOSSO caso (1ª pessoa olhando pra baixo): ativa o tppCamera
// mas posiciona PERTO DA CABEÇA (y≈0, z=1.7=altura da cabeça) em vez de atrás → visão FPP + corpo em pé.
// `@addMethod(PlayerPuppet)` (seguro, igual @addField já usado — NÃO é @wrapMethod, que crasha world-load).
// `FindComponentByName` é protected → só acessível de DENTRO de um método do próprio player (aqui).

@addMethod(PlayerPuppet)
public func BwmsActivateTppCam(atHead: Bool) -> Void {
  // NOTA: o campo `playerController` existe no runtime (o CET seta) mas NÃO está no stub redscript
  // do ActivateTPPRepresentationEvent (importonly, campo não exposto) → não dá pra setar aqui. A peça
  // central é o tppCamera; testar sem playerController primeiro. Se precisar, setar via reflexão Rust.
  this.QueueEvent(new ActivateTPPRepresentationEvent());
  // 2026-07-16: o `tppCamera` NÃO existe no player vanilla — quem o ADICIONA é o .archive do JB mod
  // (entity do player patchada: player_ma_fpp.ent/player_wa_fpp.ent + player_locomotion.animgraph),
  // agora instalado como archive/Mac/content/basegame_zzzz_jbtpp.archive (Path A, aprovado pelo
  // Perrotta). Com o archive carregado, o FindComponentByName deve ACHAR o tppCamera.
  // (Descartado: vehicleTPPCamera a pé — ativa mas o transform fica quebrado sem veículo, câmera
  // presa longe do corpo. Provado 2026-07-16.)
  let c1: ref<IComponent> = this.FindComponentByName(n"tppCamera");
  let cam: ref<CameraComponent> = c1 as CameraComponent;
  Print("[bwms-tppcam] tppCamera raw=" + ToString(IsDefined(c1)) + " asCameraComponent=" + ToString(IsDefined(cam)));
  // SONDA DECISIVA (2026-07-16): `WorldSpaceBlendCamera` está no BUFFER do .ent vanilla mas NÃO é
  // requisitado via RequestComponent no player.script. Se EXISTIR no player → componentes de buffer
  // instanciam sem request (culpado = save-spawn não re-lê o .ent). Se NÃO existir → só instancia o
  // que é requisitado (culpado = falta RequestComponent pro tppCamera). Escolhe a rota (a) vs (b).
  let probe: ref<IComponent> = this.FindComponentByName(n"WorldSpaceBlendCamera");
  Print("[bwms-tppcam] sonda WorldSpaceBlendCamera=" + ToString(IsDefined(probe)));
  if IsDefined(cam) {
    cam.Activate(0.2);
    if atHead {
      cam.SetLocalPosition(new Vector4(0.0, -0.3, 1.7, 1.0));
    };
    Print("[bwms-tppcam] tppCamera ATIVADO (atHead=" + ToString(atHead) + ")");
  };
  // OLHAR PRA BAIXO uma vez, aqui no one-shot (sem depender do poller que crasha), pra o screenshot
  // capturar a POSE do corpo COM o anim-graph do JB servido (reslink). Se as pernas estiverem
  // esticadas = o anim-graph do JB já resolve, e o tppCamera nem é necessário pro objetivo.
  let fcam: ref<FPPCameraComponent> = this.GetFPPCameraComponent();
  if IsDefined(fcam) {
    let e: EulerAngles;
    e.Pitch = -60.0; e.Yaw = 0.0; e.Roll = 0.0;
    fcam.SetLocalOrientation(EulerAngles.ToQuat(e));
    Print("[bwms-tppcam] olhar-pra-baixo forcado uma vez (pitch -60)");
  };
}

func BwmsBootFullbody(game: GameInstance) -> Void {
  let ds: ref<DelaySystem> = GameInstance.GetDelaySystem(game);
  Print("[bwms-fb-diag] ds=" + ToString(IsDefined(ds)));
  // ── TESTE DECISIVO 2026-07-15 (tpp_oneshot) ────────────────────────────────────────────────
  // Achado: o crash `SystemsUpdater::Node::LinkJob` (~1m57s, consistente) acontece com o TPP poller
  // agendado MESMO QUE o Call() do poller NUNCA dispare (0 ticks provados). Logo o crash é do próprio
  // ATO de agendar o DelayCallback (objeto script cai fora de escopo → engine toca ref pendente),
  // não do que o poller faz. Este caminho dispara o ActivateTPPRepresentation UMA VEZ, DIRETO, SEM
  // DelayCallback/poller — isola (a) o corpo em pé aparece? (b) sem o DelayCallback, o crash some?
  // Gate: ~/.bwms-modconfig.txt tpp_oneshot=1.
  // BUG ACHADO+CORRIGIDO 2026-07-15: ler BwmsConfigGet INLINE dentro do `Equals(...)` faz o marshalling
  // do retorno String falhar (Equals dá false mesmo com valor "1") — o MESMO bug que eu já tinha
  // corrigido nos pollers (ler em local primeiro), reintroduzido sem querer aqui. Provado: o boot
  // leu tpp_oneshot='1' (log do native) mas o bloco NÃO rodou (nenhum Print, nenhum screenshot).
  let cfgOneshot: String = BwmsConfigGet("tpp_oneshot");
  let cfgNativeCam: String = BwmsConfigGet("tppcam_native");
  if Equals(cfgOneshot, "1") {
    let p1: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
    Print("[bwms-oneshot] player IsDefined=" + ToString(IsDefined(p1)));
    if IsDefined(p1) {
      if Equals(cfgNativeCam, "1") {
        // RECEITA JB TPP: ativa o tppCamera nativo (corpo em pé real), câmera na cabeça (1ª pessoa).
        let pp: ref<PlayerPuppet> = p1 as PlayerPuppet;
        if IsDefined(pp) {
          pp.BwmsActivateTppCam(true);
          Print("[bwms-oneshot] via BwmsActivateTppCam (tppCamera nativo, atHead)");
        };
      } else {
        p1.QueueEvent(new ActivateTPPRepresentationEvent());
        Print("[bwms-oneshot] ActivateTPPRepresentationEvent enfileirado (direto, sem poller/DelayCallback)");
      };
    };
  };
  // BISECT 2026-07-15: o crash `SystemsUpdater::Node::LinkJob_NoFence` foi isolado na infra do
  // poller/auto-trigger em si (sobrevive 240s sem NENHUM poller criado), mas ainda não sabemos QUAL
  // dos 3 é a causa. Gateia cada um atrás de `~/.bwms-modconfig.txt` (poller_tpp/poller_equip/
  // poller_inv = "1") pra testar 1 por vez sem precisar recompilar o dylib entre tentativas.
  // FIX DEFENSIVO (2026-07-15, mesma continuação, mais tarde): observado ao vivo que com
  // poller_tpp='1'/poller_equip=''/poller_inv='' CONFIRMADOS no log, `BwmsEquipPoller` ainda assim
  // rodou (3000+ ciclos) enquanto `BwmsTppPoller` NUNCA tickou (nem 1x, mesmo com tppcam=0, sem
  // tentar ActivateTPPRepresentation) — ou seja, a criação/agendamento do próprio poller falha, não
  // é o Activate que mata. Suspeito: reler `BwmsConfigGet` 3x seguidas (mesmo native, args String)
  // direto dentro de 3 `if Equals(...)` pode ter uma interação de marshalling entre chamadas
  // consecutivas. Fix defensivo: ler os 3 valores em variáveis locais PRIMEIRO, comparar depois.
  let cfgTpp: String = BwmsConfigGet("poller_tpp");
  let cfgEquip: String = BwmsConfigGet("poller_equip");
  let cfgInv: String = BwmsConfigGet("poller_inv");
  if IsDefined(ds) {
    if Equals(cfgTpp, "1") {
      let cam: ref<BwmsTppPoller> = new BwmsTppPoller();
      Print("[bwms-tpp-diag] new BwmsTppPoller() IsDefined=" + ToString(IsDefined(cam)));
      cam.m_game = game; cam.m_last = 0;
      ds.DelayCallback(cam, 0.5);
      Print("[bwms-tpp-diag] DelayCallback(cam,0.5) chamado");
    };
    // BwmsCamPoller (offset adaptativo do FPPCameraComponent) REMOVIDO do boot: ele força a posição
    // da câmera FPP a cada 0.03s e BRIGA com a câmera 3ª-pessoa que o ActivateTPPRepresentation põe,
    // revertendo o corpo pra FPP. Sem ele, o Activate manda na câmera. (Teste 2026-07-15.)
    if Equals(cfgEquip, "1") {
      let eq: ref<BwmsEquipPoller> = new BwmsEquipPoller();   // Skill 2: equipar por código
      eq.m_game = game; eq.m_last = 0; eq.m_lastCheck = 0;
      ds.DelayCallback(eq, 0.6);
    };
    if Equals(cfgInv, "1") {
      let inv: ref<BwmsInvPoller> = new BwmsInvPoller();      // Skill 1b: preview do inventário
      inv.m_game = game; inv.m_last = 0;
      ds.DelayCallback(inv, 0.7);
    };
  };
  // redscript-mod-persistence: restaura o God Mode se a config externa (~/.bwms-modconfig.txt,
  // FORA do save) diz que estava ligado — completa o round-trip que faltava (a primitiva
  // BwmsConfigGet/Set já era provada desde 2026-07-13, mas nenhum cheat real a usava ainda).
  // Roda aqui (BwmsBootFullbody) porque este ponto já tem GameInstance+player resolvidos de
  // forma confiável (fix do achado GetGameInstance-morto, mesma sessão).
  // (mesmo fix inline→local do one-shot acima: o godmode-restore também lia BwmsConfigGet inline,
  // então provavelmente NUNCA restaurou de fato — bug latente, corrigido aqui de tabela.)
  let cfgGod: String = BwmsConfigGet("godmode");
  if Equals(cfgGod, "1") {
    let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
    let pp: ref<PlayerPuppet> = pl as PlayerPuppet;
    if IsDefined(pp) {
      GameInstance.GetGodModeSystem(game).AddGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
    };
  };
}

// BwmsTppRefire(game) — RE-DISPARO do full-body dirigido pelo TICK LOOP DO RUST (estável), NÃO por
// DelayCallback de redscript (que crasha: o objeto script cai de escopo no contexto callg-anormal e
// o motor toca a ref pendente ~1m57s depois → SystemsUpdater::Node::LinkJob). Cada chamada é um
// ONE-SHOT independente: pega o player, e se o toggle da câmera-TPP (~/.bwms-tppcam) está ligado,
// enfileira UM ActivateTPPRepresentationEvent (o efeito é transiente em free-roam, ~25s, então o
// Rust re-chama isto a cada ~10s pra manter o corpo). Zero objeto de vida-longa, zero DelayCallback.
// O dylib resolve esta global (get_function) e chama via call_func passando a GameInstance real —
// mesmo caminho já provado de BwmsBootFullbody.
func BwmsTppRefire(game: GameInstance) -> Void {
  if BwmsTppState() != 1 { return; };
  let player: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  if IsDefined(player) {
    // PLANO B (2026-07-16, gated `no_reactivate=1`): hipótese = segurar `isTPP`=true continuamente
    // (BwmsLegsHold, ~1.5s) já MANTÉM o corpo anexado, tornando o re-disparo do ActivateTPP
    // DESNECESSÁRIO. Como o re-disparo é a CAUSA da oscilação da pose (reseta pra encolhido), pular
    // ele deve dar pernas em pé ESTÁVEIS. Se o corpo sumir (destacar) sem o re-disparo, a hipótese
    // cai e volta pro plano A (refire + hold rápido). A/B sem recompilar (modconfig).
    let noReact: String = BwmsConfigGet("no_reactivate");
    if NotEquals(noReact, "1") {
      player.QueueEvent(new ActivateTPPRepresentationEvent());
    };
    // StandEnter uma vez por refire (evento de transição — dispara a entrada no estado em pé).
    let legs: String = BwmsConfigGet("legs");
    if Equals(legs, "1") {
      AnimationControllerComponent.PushEvent(player, n"StandEnter");
    };
  };
}

// BwmsLegsHold(game) — SEGURA as pernas em pé (2026-07-16). Achado: setar `fullbody`/`isTPP` uma vez
// a cada ~10s (junto do refire) faz a pose OSCILAR — o re-disparo do ActivateTPPRepresentation reseta
// pra pose encolhida e briga. Fix: aplicar as vars booleanas do anim-graph (`fullbody`/`isTPP`) numa
// cadência RÁPIDA (o dylib chama isto a cada ~90 ticks, ~1.5s), pra segurar a pose em pé ENTRE os
// resets do ActivateTPP. Só inputs contínuos (sem evento/ActivateTPP), crash-free (QueueEvent).
func BwmsLegsHold(game: GameInstance) -> Void {
  let legs: String = BwmsConfigGet("legs");
  if NotEquals(legs, "1") { return; };
  let player: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  if IsDefined(player) {
    AnimationControllerComponent.SetInputBool(player, n"fullbody", true);
    AnimationControllerComponent.SetInputBool(player, n"isTPP", true);
    // TESTE 2026-07-16 (achado por RE do CR2W do player_locomotion.animgraph — dump da
    // animAnimVariableContainer, chunk #3618): `crouch` é a 1ª de só 58 float-vars do graph —
    // candidata forte a controlar a transição de estado fpp_crouch_idle <-> fpp_idle_stand. Gate
    // `stand=1` (novo, independente de `legs`) — testar isolado antes de assumir que resolve.
    let stand: String = BwmsConfigGet("stand");
    if Equals(stand, "1") {
      AnimationControllerComponent.SetInputFloat(player, n"crouch", 0.0);
    };
    // MODO JOGÁVEL (foco no que o Perrotta pediu): SEM forçar o olhar (o jogador controla a câmera e
    // joga normal); só AFASTA a câmera pra trás/cima pra ver o corpo (3ª-pessoa-ish). Gate `look=1`.
    // Câmera ajustável ao vivo por BwmsCamBack (trás, Y-, CM) / BwmsCamZ (cima, Z, CM) — sem recompilar.
    let look: String = BwmsConfigGet("look");
    if Equals(look, "1") {
      let pp: ref<PlayerPuppet> = player as PlayerPuppet;
      if IsDefined(pp) {
        let fcam: ref<FPPCameraComponent> = pp.GetFPPCameraComponent();
        if IsDefined(fcam) {
          let back: Float = Cast<Float>(BwmsCamBack()) / 100.0;
          let up: Float = Cast<Float>(BwmsCamZ()) / 100.0;
          fcam.SetLocalPosition(new Vector4(0.0, back, up, 1.0));
        };
      };
    };
  };
}

// `BwmsCamTiltOnce(game)` — DIAGNÓSTICO 2026-07-17: isola se o SetLocalOrientation (a) nunca aplica
// de vez, ou (b) aplica e é revertido pela câmera nativa no frame seguinte. Recebe `game` já resolvido
// (padrão BwmsBootFullbody) e faz a inclinação DIRETO (sem poller/delay) — chamada e screenshot devem
// acontecer no MESMO instante (a chamada é síncrona; o screenshot é tirado logo em seguida pelo lado
// Rust, sem esperar nenhum tick).
public func BwmsCamTiltOnce(game: GameInstance) -> Void {
  let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  let pp: ref<PlayerPuppet> = pl as PlayerPuppet;
  if !IsDefined(pp) {
    Print("[camtilt] sem player");
    return;
  };
  let cam: ref<FPPCameraComponent> = pp.GetFPPCameraComponent();
  if !IsDefined(cam) {
    Print("[camtilt] GetFPPCameraComponent() falhou");
    return;
  };
  let euler: EulerAngles;
  euler.Pitch = -60.0;
  euler.Yaw = 0.0;
  euler.Roll = 0.0;
  cam.SetLocalOrientation(EulerAngles.ToQuat(euler));
  cam.SetLocalPosition(new Vector4(0.0, 0.0, 0.0, 1.0));
  // Lê de volta IMEDIATAMENTE (mesmo frame do script) — confirma se a escrita ficou visível já aqui.
  let q = cam.GetLocalOrientation();
  let e2: EulerAngles = Quaternion.ToEulerAngles(q);
  Print("[camtilt] SetLocalOrientation(pitch=-60) aplicado; readback imediato pitch=" + ToString(e2.Pitch));
}

// `BwmsFullbodyTiltTest(game)` — 2026-07-17: torso + tilt NUMA CHAMADA SÓ (menos round-trips de canal
// = menos superfície pra crash). Ângulo MODERADO (-35°, "olhar pro peito"/posição casaco, não -60°
// que só mostra os pés) — pra ver o TORSO anexado pelo ActivateTPPRepresentation, não só o chão.
public func BwmsFullbodyTiltTest(game: GameInstance) -> Void {
  let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  let pp: ref<PlayerPuppet> = pl as PlayerPuppet;
  if !IsDefined(pp) {
    Print("[fbtilt] sem player");
    return;
  };
  pl.QueueEvent(new ActivateTPPRepresentationEvent());
  let cam: ref<FPPCameraComponent> = pp.GetFPPCameraComponent();
  if !IsDefined(cam) {
    Print("[fbtilt] torso enfileirado; GetFPPCameraComponent() falhou");
    return;
  };
  let euler: EulerAngles;
  euler.Pitch = -35.0;
  euler.Yaw = 0.0;
  euler.Roll = 0.0;
  cam.SetLocalOrientation(EulerAngles.ToQuat(euler));
  Print("[fbtilt] torso enfileirado (ActivateTPPRepresentationEvent) + câmera pitch=-35 aplicado");
}

// `BwmsLegsTiltTest(game)` — 2026-07-17: teste DEFINITIVO do problema REAL (pernas dobradas vs em pé,
// esclarecido pelo Perrotta — NÃO é câmera). Combina numa chamada só: torso (ActivateTPP) + pernas
// (fullbody/isTPP=true + crouch=0.0, a receita do BwmsLegsHold/2026-07-16) + câmera num ângulo MAIS
// ABERTO (-22°, "olhar mais reto" que enquadra torso+pernas, não só o peito) — pra ver se as pernas
// esticam OU seguem na pose casaco/agachada.
public func BwmsLegsTiltTest(game: GameInstance) -> Void {
  let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  let pp: ref<PlayerPuppet> = pl as PlayerPuppet;
  if !IsDefined(pp) {
    Print("[legstilt] sem player");
    return;
  };
  // torso
  pl.QueueEvent(new ActivateTPPRepresentationEvent());
  // pernas: mesma receita do BwmsLegsHold (fullbody/isTPP contínuos + crouch=0.0), aplicada 1x aqui
  // (o efeito de "contínuo" viria do refire, mas pra este teste único já mostra se a var MEXE a pose).
  AnimationControllerComponent.SetInputBool(pl, n"fullbody", true);
  AnimationControllerComponent.SetInputBool(pl, n"isTPP", true);
  AnimationControllerComponent.SetInputFloat(pl, n"crouch", 0.0);
  AnimationControllerComponent.PushEvent(pl, n"StandEnter");
  // câmera: ângulo mais aberto que o -35 do teste anterior, pra enquadrar torso+pernas (não só peito).
  let cam: ref<FPPCameraComponent> = pp.GetFPPCameraComponent();
  if IsDefined(cam) {
    let euler: EulerAngles;
    euler.Pitch = -22.0;
    euler.Yaw = 0.0;
    euler.Roll = 0.0;
    cam.SetLocalOrientation(EulerAngles.ToQuat(euler));
  };
  Print("[legstilt] torso+pernas(fullbody/isTPP/crouch=0/StandEnter)+câmera(-22) aplicados numa chamada");
}

// `BwmsGetSceneTier(game)` — 2026-07-18: achado do coordenador — o teste de `legstilt` anterior rodou
// contra um save cujo autocontinue pousou NUMA CENA/ANIMAÇÃO ROTEIRIZADA (V sentado, jornal na mão,
// cenário de barco), não em free-roam normal — confound real, não responde "as pernas esticam?".
// `PlayerPuppet.GetSceneTier` (static, player.script:4569) lê `PlayerStateMachineBlackboard.HighLevel`
// (`gamePSMHighLevel`, orphans.script:6676): 0=Default (free-roam de verdade), 1-5=SceneTier1..5
// (dialogo/cutscene/animação roteirizada), 6=Swimming. -1 = sem player (erro). Sinal CONFIÁVEL pra
// "V está andando/parado sob controle normal" vs "V está numa cena que o jogo está dirigindo" —
// nem phase5 nem "save carregou" garantem isso (phase5 só garante que o MUNDO carregou, não que o
// player já tem controle livre). Checar isto == 0 ANTES de disparar `legstilt` de novo.
public func BwmsGetSceneTier(game: GameInstance) -> Int32 {
  let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
  let pp: ref<PlayerPuppet> = pl as PlayerPuppet;
  if !IsDefined(pp) {
    Print("[scenetier] sem player");
    return -1;
  };
  let tier: Int32 = PlayerPuppet.GetSceneTier(pp);
  Print(s"[scenetier] tier=\(tier) (0=free-roam real, 1-5=cena/animação roteirizada, 6=nadando)");
  return tier;
}
