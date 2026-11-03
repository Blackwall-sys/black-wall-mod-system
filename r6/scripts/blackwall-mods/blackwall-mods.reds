// blackwall-mods.reds — o botão MODS e a página de cheats na lista do menu de pausa, e a entrada do V no jogo (`BwmsAoEntrarNoJogo`).
// A página reusa Clear/AddMenuItem/Refresh herdados de gameuiMenuItemListGameController (texto limpo), sem CET e sem hook de runtime;
// só no menu de PAUSA (precisa de player vivo).
//
// Modo Imortal le o ESTADO REAL do God Mode via GodModeSystem.HasGodMode (sem campo proprio
// que reseta). Precedente vanilla: cyberpunk/UI/Player/healthbar.script:683.

@addField(PauseMenuGameController) let m_bwInModsPage: Bool;

// Desliga o God Mode do BWMS: a fonte atual e as antigas (`BlackwallMods` do botão MODS e `Console` do terminal até a 0.1.5). O jogo grava o
// GodModeSystem no save, então um save antigo traz essas entradas e tirar só a `BWMS` deixa o V invulnerável. A chave `godmode` da config externa
// zera junto: o `BwmsAoEntrarNoJogo` religa por ela na carga, e o OFF de qualquer caminho tem de valer no próximo save carregado.
public func BwmsGodModeDesligar(sys: ref<GodModeSystem>, id: EntityID) -> Void {
  let fontes: array<CName> = [n"BWMS", n"BlackwallMods", n"Console"];
  for fonte in fontes {
    let voltas: Int32 = 0;
    while voltas < 8 && sys.RemoveGodMode(id, gameGodModeType.Invulnerable, fonte) {
      voltas += 1;
    };
  };
  BwmsConfigSet("godmode", "0");
}

// Entrada no jogo: o Rust chama isto uma vez por player de gameplay real (fim do load), com a GameInstance do próprio player. Disparador
// único do que o produto faz com o V vivo; pelo tick e não por wrapper de `OnGameAttached`, que também recebe o player do menu.
public func BwmsAoEntrarNoJogo(game: GameInstance) -> Void {
  let pp: ref<PlayerPuppet> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject() as PlayerPuppet;
  if !IsDefined(pp) {
    return;
  };
  // ArchiveXL #29: o PuppetStateHandler do V (estado de braços e pés para os sufixos de aparência).
  pp.BwmsAlwaysRun_estado_do_boneco();
  // God Mode ligado na config externa (fora do save) volta. O save pode já trazer a entrada `BWMS`: só põe sem invulnerabilidade, para não
  // duplicar a fonte.
  let cfgGod: String = BwmsConfigGet("godmode");
  if Equals(cfgGod, "1") {
    let gms: ref<GodModeSystem> = GameInstance.GetGodModeSystem(game);
    if !gms.HasGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable) {
      gms.AddGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
    };
  };
}

// i18n: mesma deteccao do bwms-settings-poc (idioma do jogo via /language OnScreen, default EN).
@addMethod(PauseMenuGameController)
public func BWMSLang() -> Int32 {
  let v: ref<ConfigVarListName> =
    this.GetSystemRequestsHandler().GetUserSettings().GetVar(n"/language", n"OnScreen") as ConfigVarListName;
  if !IsDefined(v) { return 0; };
  let code: String = NameToString(v.GetValue());
  if StrBeginsWith(code, "pt") { return 1; };
  if StrBeginsWith(code, "zh") { return 2; };
  return 0;
}
@addMethod(PauseMenuGameController)
public func L(en: String, pt: String, zh: String) -> String {
  switch this.BWMSLang() {
    case 1: return pt;
    case 2: return zh;
  };
  return en;
}

@wrapMethod(PauseMenuGameController)
private func PopulateMenuItemList() -> Void {
  wrappedMethod();
  this.AddMenuItem(this.L("MODS", "MODS", "模组"), n"BWModsRoot");
  this.m_menuListController.Refresh();
}

@addMethod(PauseMenuGameController)
private final func BWHasGodMode() -> Bool {
  let owner: ref<GameObject> = this.GetPlayerControlledObject();
  if !IsDefined(owner) { return false; };
  return GameInstance.GetGodModeSystem(owner.GetGame())
    .HasGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable);
}

@addMethod(PauseMenuGameController)
private final func BWShowModsPage() -> Void {
  this.Clear();
  let g: String = this.BWHasGodMode() ? this.L("God Mode: ON", "Modo Imortal: LIGADO", "上帝模式：开") : this.L("God Mode: OFF", "Modo Imortal: DESLIGADO", "上帝模式：关");
  this.AddMenuItem(g, n"BWModsGod");
  this.AddMenuItem(this.L("Heal", "Curar", "治疗"), n"BWModsHeal");
  this.AddMenuItem(this.L("+10,000 Eddies", "+10.000 Eddies", "+10,000 欧元币"), n"BWModsMoney");
  this.AddMenuItem(this.L("Back", "Voltar", "返回"), n"BWModsBack");
  this.m_menuListController.Refresh();
  this.SetCursorOverWidget(inkCompoundRef.GetWidgetByIndex(this.m_menuList, 0), 0.00, true);
}

@wrapMethod(PauseMenuGameController)
protected cb func OnMenuItemActivated(index: Int32, target: ref<ListItemController>) -> Bool {
  let data: ref<PauseMenuListItemData> = target.GetData() as PauseMenuListItemData;
  if !IsDefined(data) { return wrappedMethod(index, target); };
  let owner: ref<GameObject> = this.GetPlayerControlledObject();

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
      let sys: ref<GodModeSystem> = GameInstance.GetGodModeSystem(owner.GetGame());
      if sys.HasGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable) {
        BwmsGodModeDesligar(sys, owner.GetEntityID());
      } else {
        sys.AddGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
        BwmsConfigSet("godmode", "1");
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

  return wrappedMethod(index, target);
}
