// blackwall-mods.reds — pagina de cheats IN-LOCO na lista do menu de pausa (ESC > MODS).
// Reusa Clear/AddMenuItem/Refresh herdados de gameuiMenuItemListGameController (texto limpo).
// Zero CET, zero hook de runtime. So menu de PAUSA (precisa de player vivo).
//
// Modo Imortal le o ESTADO REAL do God Mode via GodModeSystem.HasGodMode (sem campo proprio
// que reseta). Precedente vanilla: cyberpunk/UI/Player/healthbar.script:683.
//
// RAM / NPCs / Municao seguem a mesma disciplina do resto do projeto: cada um usa o mecanismo
// que o PROPRIO jogo usa, e cada escolha aqui saiu de medicao em jogo, nao de leitura de nome.
// O que foi descartado, com o porque, esta anotado em cada bloco — pra ninguem refazer o caminho.

@addField(PauseMenuGameController) let m_bwInModsPage: Bool;

// Estado dos cheats continuos. Fica no PLAYER, nao no controller: o controller do menu de pausa
// e destruido quando o menu fecha, e o estado tem que sobreviver a isso.
@addField(PlayerPuppet) let m_bwCloakOn: Bool;
@addField(PlayerPuppet) let m_bwVisMod: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwAmmoOn: Bool;
@addField(PlayerPuppet) let m_bwMagMod: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwMagWeapon: wref<WeaponObject>;
@addField(PlayerPuppet) let m_bwRamOn: Bool;
@addField(PlayerPuppet) let m_bwTicking: Bool;

// Callback do tick. Classe dedicada em vez do despacho por nome do BwmsDelay: aquele caminho
// depende de uma nativa do BWMS, e este arquivo nao precisa de nenhuma — so API do jogo.
public class BwmsCheatTick extends DelayCallback {
  public let Player: wref<PlayerPuppet>;
  public func Call() -> Void {
    if IsDefined(this.Player) {
      this.Player.BWMSCheatTick();
    };
  }
}

// Reagenda enquanto algum cheat continuo estiver ligado. `m_bwTicking` evita dois tickers
// vivos ao mesmo tempo se o jogador ligar RAM e Municao em seguida.
//
// 2 segundos, nao um: o custo aqui nao e a conta, e o `GiveItem` — cada um levanta os eventos de
// inventario do jogo. Nem a RAM nem a reserva de municao caem rapido o bastante pra justificar
// mais que isso.
@addMethod(PlayerPuppet)
public func BWMSCheatSchedule() -> Void {
  if this.m_bwTicking { return; };
  if !this.m_bwRamOn && !this.m_bwAmmoOn { return; };
  this.m_bwTicking = true;
  let cb: ref<BwmsCheatTick> = new BwmsCheatTick();
  cb.Player = this;
  GameInstance.GetDelaySystem(this.GetGame()).DelayCallback(cb, 2.00, false);
}

@addMethod(PlayerPuppet)
public func BWMSCheatTick() -> Void {
  this.m_bwTicking = false;
  let game: GameInstance = this.GetGame();
  // RAM ilimitada: o jogo nao expoe "custo zero", o que existe e o POOL. Entao "ilimitado" aqui
  // e o pool Memory sendo enchido de volta — que e o que o jogador percebe.
  if this.m_bwRamOn {
    GameInstance.GetStatPoolsSystem(game).RequestSettingStatPoolValue(
      Cast<StatsObjectID>(this.GetEntityID()), gamedataStatPoolType.Memory, 100.00, this, true);
  };
  // Municao: o pente grande mata a recarga, mas a RESERVA continua caindo porque e dela que a
  // recarga puxa. Repor a reserva e a outra metade — sem ela o cheat parece nao funcionar.
  if this.m_bwAmmoOn {
    this.BWMSTopUpAmmo(t"Ammo.HandgunAmmo");
    this.BWMSTopUpAmmo(t"Ammo.RifleAmmo");
    this.BWMSTopUpAmmo(t"Ammo.ShotgunAmmo");
    this.BWMSTopUpAmmo(t"Ammo.SniperRifleAmmo");
    this.BWMSApplyMagazine();
  };
  this.BWMSCheatSchedule();
}

// So repoe o que FALTA. Dar municao de dois em dois segundos sem olhar quanto ja existe enche a
// tela de "ALINDI: ..." — cada GiveItem levanta a notificacao de item recebido do jogo, mesmo
// quando a reserva ja esta cheia. Perguntar a quantidade antes custa uma chamada e cala o resto.
@addMethod(PlayerPuppet)
public func BWMSTopUpAmmo(ammo: TweakDBID) -> Void {
  let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(this.GetGame());
  let id: ItemID = ItemID.FromTDBID(ammo);
  // Limiar alto e reposicao grande de proposito: assim a notificacao aparece raramente, e quando
  // aparece corresponde a municao de verdade gasta.
  if ts.GetItemQuantity(this, id) < 100 {
    ts.GiveItem(this, id, 500);
  };
}

// O modificador vive na ARMA, entao trocar de arma o deixaria para tras. Remover e reaplicar a
// cada tick e mais simples e sempre correto — mais barato que guardar identidade de arma.
@addMethod(PlayerPuppet)
public func BWMSApplyMagazine() -> Void {
  this.BWMSClearMagazine();
  let w: ref<WeaponObject> = GameObject.GetActiveWeapon(this);
  if !IsDefined(w) { return; };
  this.m_bwMagMod = RPGManager.CreateStatModifier(
    gamedataStatType.MagazineCapacity, gameStatModifierType.Additive, 999.00);
  GameInstance.GetStatsSystem(this.GetGame())
    .AddModifier(Cast<StatsObjectID>(w.GetEntityID()), this.m_bwMagMod);
  this.m_bwMagWeapon = w;
}

@addMethod(PlayerPuppet)
public func BWMSClearMagazine() -> Void {
  if IsDefined(this.m_bwMagMod) && IsDefined(this.m_bwMagWeapon) {
    GameInstance.GetStatsSystem(this.GetGame())
      .RemoveModifier(Cast<StatsObjectID>(this.m_bwMagWeapon.GetEntityID()), this.m_bwMagMod);
  };
  this.m_bwMagMod = null;
  this.m_bwMagWeapon = null;
}

// "Os NPCs te deixam em paz". Limite honesto, porque o nome promete mais: NAO e invisibilidade —
// os NPCs continuam ENXERGANDO; o que muda e que param de atacar.
//
// Estes tres records vieram de medicao em jogo. O que foi descartado, e por que:
//   BaseStatusEffect.Cloaked  — ENTRA no jogador (HasStatusEffect responde SIM) e a percepcao
//                               nao muda: e o efeito do lado dos NPCs (Oda/MaxTac).
//   OpticalCamoIsActive       — aceita a escrita (0 -> 1 confirmado) e nao liga nada: e um stat
//                               que o jogo LE, nao um que obedece.
@addMethod(PlayerPuppet)
public func BWMSSetCloak(on: Bool) -> Void {
  let game: GameInstance = this.GetGame();
  let ses: ref<StatusEffectSystem> = GameInstance.GetStatusEffectSystem(game);
  let ss: ref<StatsSystem> = GameInstance.GetStatsSystem(game);
  let id: EntityID = this.GetEntityID();
  let soid: StatsObjectID = Cast<StatsObjectID>(id);
  if on {
    ses.ApplyStatusEffect(id, t"BaseStatusEffect.BlockTargetingPlayer");
    ses.ApplyStatusEffect(id, t"BaseStatusEffect.DontShootAtMe");
    ses.ApplyStatusEffect(id, t"BaseStatusEffect.SetFriendly");
    if !IsDefined(this.m_bwVisMod) {
      this.m_bwVisMod = RPGManager.CreateStatModifier(
        gamedataStatType.Visibility, gameStatModifierType.Additive, -1000.00);
      ss.AddModifier(soid, this.m_bwVisMod);
    };
  } else {
    ses.RemoveStatusEffect(id, t"BaseStatusEffect.BlockTargetingPlayer");
    ses.RemoveStatusEffect(id, t"BaseStatusEffect.DontShootAtMe");
    ses.RemoveStatusEffect(id, t"BaseStatusEffect.SetFriendly");
    // Heranca: as tentativas anteriores aplicavam `Cloaked`, que entra de verdade e vai junto pro
    // SAVE, deixando o personagem invisivel ao carregar. Desligar tem que limpa-lo.
    ses.RemoveStatusEffect(id, t"BaseStatusEffect.Cloaked");
    if IsDefined(this.m_bwVisMod) {
      ss.RemoveModifier(soid, this.m_bwVisMod);
      this.m_bwVisMod = null;
    };
  };
  this.m_bwCloakOn = on;
}

// i18n: mesma deteccao do bwms-settings-poc (idioma do jogo via /language OnScreen, default EN).
// Turco incluido porque o menu aparecia em ingles dentro de um jogo em turco — o mesmo defeito
// que ja tinha sido corrigido no console.
@addMethod(PauseMenuGameController)
public func BWMSLang() -> Int32 {
  let v: ref<ConfigVarListName> =
    this.GetSystemRequestsHandler().GetUserSettings().GetVar(n"/language", n"OnScreen") as ConfigVarListName;
  if !IsDefined(v) { return 0; };
  let code: String = NameToString(v.GetValue());
  if StrBeginsWith(code, "pt") { return 1; };
  if StrBeginsWith(code, "zh") { return 2; };
  if StrBeginsWith(code, "tr") { return 3; };
  return 0;
}
@addMethod(PauseMenuGameController)
public func L(en: String, pt: String, zh: String, tr: String) -> String {
  switch this.BWMSLang() {
    case 1: return pt;
    case 2: return zh;
    case 3: return tr;
  };
  return en;
}

@wrapMethod(PauseMenuGameController)
private func PopulateMenuItemList() -> Void {
  wrappedMethod();
  this.AddMenuItem(this.L("MODS", "MODS", "模组", "MODLAR"), n"BWModsRoot");
  this.m_menuListController.Refresh();
}

@addMethod(PauseMenuGameController)
private final func BWHasGodMode() -> Bool {
  let owner: ref<GameObject> = this.GetPlayerControlledObject();
  if !IsDefined(owner) { return false; };
  let sys: ref<GodModeSystem> = GameInstance.GetGodModeSystem(owner.GetGame());
  return sys.HasGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable)
      || sys.HasGodMode(owner.GetEntityID(), gameGodModeType.Immortal);
}

@addMethod(PauseMenuGameController)
private final func BWPlayer() -> ref<PlayerPuppet> {
  return this.GetPlayerControlledObject() as PlayerPuppet;
}

// Sufixo de estado para os itens que ligam/desligam. Mostrar o estado no proprio rotulo e o que
// o God Mode ja fazia; sem isso, apertar um toggle nao da retorno nenhum.
@addMethod(PauseMenuGameController)
private final func BWOnOff(on: Bool) -> String {
  return on ? this.L("ON", "LIGADO", "开", "AÇIK") : this.L("OFF", "DESLIGADO", "关", "KAPALI");
}

@addMethod(PauseMenuGameController)
private final func BWShowModsPage() -> Void {
  this.Clear();
  let pp: ref<PlayerPuppet> = this.BWPlayer();
  let cloakOn: Bool = IsDefined(pp) && pp.m_bwCloakOn;
  let ammoOn: Bool = IsDefined(pp) && pp.m_bwAmmoOn;
  let ramOn: Bool = IsDefined(pp) && pp.m_bwRamOn;

  let g: String = this.BWHasGodMode()
    ? this.L("God Mode: ON", "Modo Imortal: LIGADO", "上帝模式：开", "Tanrı Modu: AÇIK")
    : this.L("God Mode: OFF", "Modo Imortal: DESLIGADO", "上帝模式：关", "Tanrı Modu: KAPALI");
  this.AddMenuItem(g, n"BWModsGod");
  this.AddMenuItem(this.L("Heal", "Curar", "治疗", "İyileştir"), n"BWModsHeal");
  this.AddMenuItem(this.L("+10,000 Eddies", "+10.000 Eddies", "+10,000 欧元币", "+10.000 Eddie"), n"BWModsMoney");
  this.AddMenuItem(this.L("Unlimited RAM: ", "RAM ilimitada: ", "无限内存：", "Sınırsız RAM: ")
    + this.BWOnOff(ramOn), n"BWModsRam");
  this.AddMenuItem(this.L("Unlimited ammo: ", "Municao ilimitada: ", "无限弹药：", "Sınırsız Mermi: ")
    + this.BWOnOff(ammoOn), n"BWModsAmmo");
  this.AddMenuItem(this.L("NPCs leave you alone: ", "NPCs te deixam em paz: ", "NPC 不再攻击你：", "NPC'ler Saldırmasın: ")
    + this.BWOnOff(cloakOn), n"BWModsCloak");
  this.AddMenuItem(this.L("Back", "Voltar", "返回", "Geri"), n"BWModsBack");
  this.m_menuListController.Refresh();
  this.SetCursorOverWidget(inkCompoundRef.GetWidgetByIndex(this.m_menuList, 0), 0.00, true);
}

@wrapMethod(PauseMenuGameController)
protected cb func OnMenuItemActivated(index: Int32, target: ref<ListItemController>) -> Bool {
  let data: ref<PauseMenuListItemData> = target.GetData() as PauseMenuListItemData;
  if !IsDefined(data) { return wrappedMethod(index, target); };
  let owner: ref<GameObject> = this.GetPlayerControlledObject();
  let pp: ref<PlayerPuppet> = this.BWPlayer();

  if Equals(data.eventName, n"BWModsRoot") {
    this.PlaySound(n"Button", n"OnPress");
    this.m_bwInModsPage = true; this.BWShowModsPage(); return true;
  };
  if Equals(data.eventName, n"BWModsBack") {
    this.PlaySound(n"Button", n"OnPress");
    this.m_bwInModsPage = false; this.ShowActionsList(); return true;
  };
  if Equals(data.eventName, n"BWModsGod") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(owner) {
      // Os DOIS tipos, nao so um: `Invulnerable` e "nao toma dano" e `Immortal` e "nao morre".
      // Ligar so um deixa um buraco de semantica — e era o unico ligado aqui.
      let sys: ref<GodModeSystem> = GameInstance.GetGodModeSystem(owner.GetGame());
      let id: EntityID = owner.GetEntityID();
      if this.BWHasGodMode() {
        sys.RemoveGodMode(id, gameGodModeType.Invulnerable, n"BlackwallMods");
        sys.RemoveGodMode(id, gameGodModeType.Immortal, n"BlackwallMods");
      } else {
        sys.AddGodMode(id, gameGodModeType.Invulnerable, n"BlackwallMods");
        sys.AddGodMode(id, gameGodModeType.Immortal, n"BlackwallMods");
        // O God Mode nao repoe o que ja foi perdido: sem isto, ligar com pouca vida deixa o
        // jogador ligado e quase morto, o que se parece com "nao funcionou".
        GameInstance.GetStatPoolsSystem(owner.GetGame())
          .RequestSettingStatPoolValue(Cast<StatsObjectID>(id),
                                       gamedataStatPoolType.Health, 100.00, owner, true);
      };
    };
    this.BWShowModsPage(); return true;
  };
  if Equals(data.eventName, n"BWModsHeal") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(owner) {
      GameInstance.GetStatPoolsSystem(owner.GetGame())
        .RequestSettingStatPoolValue(Cast<StatsObjectID>(owner.GetEntityID()),
                                     gamedataStatPoolType.Health, 100.00, owner, true);
    };
    return true;
  };
  if Equals(data.eventName, n"BWModsMoney") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(owner) {
      GameInstance.GetTransactionSystem(owner.GetGame())
        .GiveItem(owner, ItemID.FromTDBID(t"Items.money"), 10000);
    };
    return true;
  };
  if Equals(data.eventName, n"BWModsRam") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(pp) {
      pp.m_bwRamOn = !pp.m_bwRamOn;
      if pp.m_bwRamOn {
        // Enche na hora, sem esperar o primeiro tick: o retorno tem que ser imediato.
        GameInstance.GetStatPoolsSystem(pp.GetGame())
          .RequestSettingStatPoolValue(Cast<StatsObjectID>(pp.GetEntityID()),
                                       gamedataStatPoolType.Memory, 100.00, pp, true);
        pp.BWMSCheatSchedule();
      };
    };
    this.BWShowModsPage(); return true;
  };
  if Equals(data.eventName, n"BWModsAmmo") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(pp) {
      pp.m_bwAmmoOn = !pp.m_bwAmmoOn;
      if pp.m_bwAmmoOn {
        pp.BWMSApplyMagazine();
        pp.BWMSCheatSchedule();
      } else {
        pp.BWMSClearMagazine();
      };
    };
    this.BWShowModsPage(); return true;
  };
  if Equals(data.eventName, n"BWModsCloak") {
    this.PlaySound(n"Button", n"OnPress");
    if IsDefined(pp) {
      pp.BWMSSetCloak(!pp.m_bwCloakOn);
    };
    this.BWShowModsPage(); return true;
  };

  return wrappedMethod(index, target);
}
