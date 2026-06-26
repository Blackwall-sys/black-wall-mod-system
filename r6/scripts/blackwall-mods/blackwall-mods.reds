// blackwall-mods.reds — passo 2: pagina de cheats IN-LOCO na lista do menu de pausa.
// Reusa Clear/AddMenuItem/Refresh herdados de gameuiMenuItemListGameController (texto limpo).
// Zero CET, zero hook de runtime. So menu de PAUSA (precisa de player vivo).
//
// Modo Imortal le o ESTADO REAL do God Mode via GodModeSystem.HasGodMode (sem campo proprio
// que reseta). Precedente vanilla: cyberpunk/UI/Player/healthbar.script:683.

@addField(PauseMenuGameController) let m_bwInModsPage: Bool;

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
        sys.RemoveGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable, n"BlackwallMods");
      } else {
        sys.AddGodMode(owner.GetEntityID(), gameGodModeType.Invulnerable, n"BlackwallMods");
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
