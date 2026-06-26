// BWMS — pagina nativa (Mods > Cheats) via bypass do ConfigVar. 100% redscript, sem CET, sem NativeSettings.
// Aba "Cheats": 9 cheats (Modo Imortal + faceis). Toggles guardam estado em campos do PlayerPuppet (persiste ao reabrir).
// Gate = !IsDefined(this.m_SettingsEntry): nossos controllers tem entry null; os do jogo nao.
// Clique unico: OnShortcutPress/OnShortcutRepeat neutralizados pros nossos (corpo/setas ja alteram).

@addField(SettingsSelectorControllerBool) let m_bwmsGame: GameInstance;
@addField(SettingsSelectorControllerBool) let m_bwmsCheat: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsMin: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsMax: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsStep: Int32;
@addField(SettingsSelectorControllerFloat) let m_bwmsMinF: Float;
@addField(SettingsSelectorControllerFloat) let m_bwmsMaxF: Float;
@addField(SettingsSelectorControllerFloat) let m_bwmsStepF: Float;
@addField(SettingsSelectorControllerListString) let m_bwmsElems: array<String>;
@addField(SettingsSelectorControllerListString) let m_bwmsIdx: Int32;

// estado de cheat persistido no proprio jogador (sobrevive a reabrir a aba; NAO vai pro save = runtime-only)
@addField(PlayerPuppet) let m_bwmsCarry: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsDmg: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsRam: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsSlow: Bool;

@addMethod(SettingsCategoryController)
public func BWMSSetText(text: String) -> Void {
  inkTextRef.SetText(this.m_label, text);
}

// ===== i18n: idioma do jogo (en/pt/zh). Le /language OnScreen (sem player/save).
// Default = EN se a leitura falhar ou for outro idioma → nunca quebra, nunca fica vazio.
@addMethod(SettingsMainGameController)
public func BWMSLang() -> Int32 {
  let v: ref<ConfigVarListName> =
    this.GetSystemRequestsHandler().GetUserSettings().GetVar(n"/language", n"OnScreen") as ConfigVarListName;
  if !IsDefined(v) { return 0; };
  let code: String = NameToString(v.GetValue());
  if StrBeginsWith(code, "pt") { return 1; };
  if StrBeginsWith(code, "zh") { return 2; };
  return 0;
}
@addMethod(SettingsMainGameController)
public func L(en: String, pt: String, zh: String) -> String {
  switch this.BWMSLang() {
    case 1: return pt;
    case 2: return zh;
  };
  return en;
}

// ===== injecao das abas =====
@wrapMethod(SettingsMainGameController)
private final func PopulateSettingsData() -> Void {
  wrappedMethod();
  let g: SettingsCategory;
  g.label = StringToName(this.L("Cheats", "Cheats", "作弊")); g.groupPath = n"/bwms"; g.isEmpty = false;
  ArrayPush(this.m_data, g);
  let h: SettingsCategory;
  h.label = StringToName(this.L("BWMS Help", "BWMS Ajuda", "BWMS 帮助")); h.groupPath = n"/bwms-help"; h.isEmpty = false;
  ArrayPush(this.m_data, h);
}

@addMethod(SettingsMainGameController)
private final func BWMSDocLine(text: String) -> Void {
  let cc: ref<SettingsCategoryController> =
    this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsCategory")
        .GetController() as SettingsCategoryController;
  if IsDefined(cc) { cc.BWMSSetText(text); };
}

@addMethod(SettingsMainGameController)
private final func BWMSCheat(label: String, cheatId: Int32, game: GameInstance) -> Void {
  let cb: ref<SettingsSelectorControllerBool> =
    this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsSelectorBool")
        .GetController() as SettingsSelectorControllerBool;
  if IsDefined(cb) { cb.BWMSSetupBool(label, cheatId, game); ArrayPush(this.m_settingsElements, cb); };
}

@wrapMethod(SettingsMainGameController)
private final func PopulateCategorySettingsOptions(idx: Int32) -> Void {
  let realIdx: Int32 = idx < 0 ? this.m_selectorCtrl.GetToggledIndex() : idx;
  if realIdx < 0 || realIdx >= ArraySize(this.m_data) { wrappedMethod(idx); return; };
  let gp: CName = this.m_data[realIdx].groupPath;

  if Equals(gp, n"/bwms") {
    ArrayClear(this.m_settingsElements);
    inkCompoundRef.RemoveAllChildren(this.m_settingsOptionsList);
    inkTextRef.SetText(this.m_descriptionText, ""); inkWidgetRef.SetVisible(this.m_descriptionText, false);
    let pl: ref<GameObject> = this.GetPlayerControlledObject();
    if IsDefined(pl) {
      let g: GameInstance = pl.GetGame();
      this.BWMSCheat(this.L("Invincible (God Mode)", "Invencível (Modo Imortal)", "无敌（上帝模式）"), 1, g);
      this.BWMSCheat(this.L("Infinite carry weight", "Carga infinita", "无限负重"), 2, g);
      this.BWMSCheat(this.L("Massive damage (+1000%)", "Dano massivo (+1000%)", "巨额伤害（+1000%）"), 3, g);
      this.BWMSCheat(this.L("Infinite cyberdeck RAM", "RAM do cyberdeck infinita", "无限赛博硬件内存"), 4, g);
      this.BWMSCheat(this.L("Slow motion", "Câmera lenta", "慢动作"), 5, g);
      this.BWMSCheat(this.L("+10,000 Eddies", "+10.000 Eddies", "+10,000 欧元币"), 6, g);
      this.BWMSCheat(this.L("+1 Attribute Point", "+1 Ponto de Atributo", "+1 属性点"), 7, g);
      this.BWMSCheat(this.L("+1 Perk Point", "+1 Ponto de Perk", "+1 专长点数"), 8, g);
      this.BWMSCheat(this.L("Unlock all vehicles", "Desbloquear todos os veículos", "解锁所有载具"), 9, g);
    } else {
      this.BWMSDocLine(this.L("Load a save to use the cheats (you need a living V).", "Carregue um save para usar os cheats (precisa de um V vivo).", "载入存档后才能使用作弊（需要存活的 V）。"));
    };
    this.m_selectorCtrl.SetSelectedIndex(realIdx);
    return;
  };

  if Equals(gp, n"/bwms-help") {
    ArrayClear(this.m_settingsElements);
    inkCompoundRef.RemoveAllChildren(this.m_settingsOptionsList);
    inkTextRef.SetText(this.m_descriptionText, ""); inkWidgetRef.SetVisible(this.m_descriptionText, false);
    this.BWMSDocLine(this.L("BWMS — Add your mod's options to this screen", "BWMS — Adicionar as opcoes do seu mod nesta tela", "BWMS — 将你的 mod 选项添加到此界面"));
    this.BWMSDocLine(this.L("All in redscript, no CET and no ConfigVar. Steps:", "Tudo em redscript, sem CET e sem ConfigVar. Passos:", "全部用 redscript，无需 CET 或 ConfigVar。步骤："));
    this.BWMSDocLine(this.L("1. Create a .reds file in r6/scripts/your-mod/", "1. Crie um arquivo .reds em r6/scripts/seu-mod/", "1. 在 r6/scripts/your-mod/ 创建一个 .reds 文件"));
    this.BWMSDocLine("2. @wrapMethod(SettingsMainGameController) PopulateSettingsData:");
    this.BWMSDocLine(this.L("   ArrayPush(this.m_data) a SettingsCategory with a unique groupPath (e.g. n\"/yourmod\").", "   ArrayPush(this.m_data) uma SettingsCategory com groupPath unico (ex n\"/seumod\").", "   ArrayPush(this.m_data) 一个带唯一 groupPath 的 SettingsCategory（例如 n\"/yourmod\"）。"));
    this.BWMSDocLine(this.L("3. @wrapMethod PopulateCategorySettingsOptions: when it matches your groupPath,", "3. @wrapMethod PopulateCategorySettingsOptions: quando for o seu groupPath,", "3. @wrapMethod PopulateCategorySettingsOptions：当匹配你的 groupPath 时，"));
    this.BWMSDocLine(this.L("   spawn settingsSelectorBool/Int/Float/StringList WITHOUT calling Setup().", "   spawne settingsSelectorBool/Int/Float/StringList SEM chamar Setup().", "   生成 settingsSelectorBool/Int/Float/StringList，但不要调用 Setup()。"));
    this.BWMSDocLine(this.L("4. Gate everything by !IsDefined(this.m_SettingsEntry); override Refresh and AcceptValue.", "4. Gate tudo por !IsDefined(this.m_SettingsEntry); sobrescreva Refresh e AcceptValue.", "4. 用 !IsDefined(this.m_SettingsEntry) 作为判定；重写 Refresh 和 AcceptValue。"));
    this.BWMSDocLine(this.L("5. Compile with redscript (scc -compile r6/scripts). Your tab shows up here.", "5. Compile com o redscript (scc -compile r6/scripts). Sua aba aparece aqui.", "5. 用 redscript 编译（scc -compile r6/scripts）。你的标签页会出现在这里。"));
    this.m_selectorCtrl.SetSelectedIndex(realIdx);
    return;
  };

  wrappedMethod(idx);
}

// ===== Bool selector = despachante de CHEAT =====
// cheatId: 1 GodMode 2 Carga 3 Dano 4 RAM 5 Slow (toggles) | 6 Eddies 7 Atributo 8 Perk 9 Veiculos (acoes)
@addMethod(SettingsSelectorControllerBool)
public func BWMSPlayer() -> ref<GameObject> {
  return GameInstance.GetPlayerSystem(this.m_bwmsGame).GetLocalPlayerControlledGameObject();
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSSetupBool(label: String, cheatId: Int32, game: GameInstance) -> Void {
  this.m_bwmsGame = game;
  this.m_bwmsCheat = cheatId;
  inkTextRef.SetText(this.m_LabelText, label);
  this.BWMSPaint();
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSIsToggle() -> Bool { return this.m_bwmsCheat <= 5; }
@addMethod(SettingsSelectorControllerBool)
public func BWMSIsOn() -> Bool {
  let p: ref<GameObject> = this.BWMSPlayer();
  let pp: ref<PlayerPuppet> = p as PlayerPuppet;
  if !IsDefined(pp) { return false; };
  switch this.m_bwmsCheat {
    case 1: return GameInstance.GetGodModeSystem(this.m_bwmsGame).HasGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable);
    case 2: return IsDefined(pp.m_bwmsCarry);
    case 3: return IsDefined(pp.m_bwmsDmg);
    case 4: return IsDefined(pp.m_bwmsRam);
    case 5: return pp.m_bwmsSlow;
  };
  return false;
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSPaint() -> Void {
  let p: ref<GameObject> = this.BWMSPlayer();
  if !IsDefined(p) { return; };
  if this.BWMSIsToggle() {
    let on: Bool = this.BWMSIsOn();
    inkWidgetRef.SetVisible(this.m_onState, on);
    inkWidgetRef.SetVisible(this.m_offState, !on);
  } else {
    inkWidgetRef.SetVisible(this.m_onState, false);
    inkWidgetRef.SetVisible(this.m_offState, true);
  };
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSRun() -> Void {
  let p: ref<GameObject> = this.BWMSPlayer();
  let pp: ref<PlayerPuppet> = p as PlayerPuppet;
  if !IsDefined(pp) { return; };
  let game: GameInstance = this.m_bwmsGame;
  let ss: ref<StatsSystem> = GameInstance.GetStatsSystem(game);
  let soid: StatsObjectID = Cast<StatsObjectID>(pp.GetEntityID());
  switch this.m_bwmsCheat {
    case 1:
      if GameInstance.GetGodModeSystem(game).HasGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable) {
        GameInstance.GetGodModeSystem(game).RemoveGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
      } else {
        GameInstance.GetGodModeSystem(game).AddGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
      };
      break;
    case 2:
      if IsDefined(pp.m_bwmsCarry) {
        ss.RemoveModifier(soid, pp.m_bwmsCarry); pp.m_bwmsCarry = null;
      } else {
        pp.m_bwmsCarry = RPGManager.CreateStatModifier(gamedataStatType.CarryCapacity, gameStatModifierType.Additive, 100000.0);
        ss.AddModifier(soid, pp.m_bwmsCarry);
      };
      break;
    case 3:
      if IsDefined(pp.m_bwmsDmg) {
        ss.RemoveModifier(soid, pp.m_bwmsDmg); pp.m_bwmsDmg = null;
      } else {
        pp.m_bwmsDmg = RPGManager.CreateStatModifier(gamedataStatType.AllDamageDonePercentBonus, gameStatModifierType.Additive, 1000.0);
        ss.AddModifier(soid, pp.m_bwmsDmg);
      };
      break;
    case 4:
      if IsDefined(pp.m_bwmsRam) {
        ss.RemoveModifier(soid, pp.m_bwmsRam); pp.m_bwmsRam = null;
      } else {
        pp.m_bwmsRam = RPGManager.CreateStatModifier(gamedataStatType.Memory, gameStatModifierType.Additive, 9999.0);
        ss.AddModifier(soid, pp.m_bwmsRam);
      };
      break;
    case 5:
      if pp.m_bwmsSlow {
        GameInstance.GetTimeSystem(game).UnsetTimeDilation(n"bwms_slow");
        pp.m_bwmsSlow = false;
      } else {
        GameInstance.GetTimeSystem(game).SetTimeDilation(n"bwms_slow", 0.30);
        pp.m_bwmsSlow = true;
      };
      break;
    case 6:
      GameInstance.GetTransactionSystem(game).GiveItem(pp, ItemID.FromTDBID(t"Items.money"), 10000);
      break;
    case 7:
      PlayerDevelopmentSystem.GetData(pp).AddDevelopmentPoints(1, gamedataDevelopmentPointType.Attribute);
      break;
    case 8:
      PlayerDevelopmentSystem.GetData(pp).AddDevelopmentPoints(1, gamedataDevelopmentPointType.Primary);
      break;
    case 9:
      GameInstance.GetVehicleSystem(game).EnableAllPlayerVehicles();
      break;
  };
}
@wrapMethod(SettingsSelectorControllerBool)
public func Refresh() -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaint(); } else { wrappedMethod(); };
}
@wrapMethod(SettingsSelectorControllerBool)
private func AcceptValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    this.BWMSRun();
    this.BWMSPaint();
  } else { wrappedMethod(forward); };
}

// ===== Int: slider (framework, p/ outros mods) =====
@addMethod(SettingsSelectorControllerInt)
public func BWMSSetupInt(label: String, mn: Int32, mx: Int32, step: Int32, cur: Int32) -> Void {
  this.m_bwmsMin = mn; this.m_bwmsMax = mx; this.m_bwmsStep = step;
  this.m_newValue = cur;
  inkTextRef.SetText(this.m_LabelText, label);
  this.m_sliderController = inkWidgetRef.GetControllerByType(this.m_sliderWidget, n"inkSliderController") as inkSliderController;
  if IsDefined(this.m_sliderController) {
    this.m_sliderController.Setup(Cast<Float>(mn), Cast<Float>(mx), Cast<Float>(cur), Cast<Float>(step));
    this.m_sliderController.RegisterToCallback(n"OnSliderValueChanged", this, n"OnSliderValueChanged");
    this.m_sliderController.RegisterToCallback(n"OnSliderHandleReleased", this, n"OnHandleReleased");
  };
  this.BWMSPaintInt();
}
@addMethod(SettingsSelectorControllerInt)
public func BWMSPaintInt() -> Void {
  inkTextRef.SetText(this.m_ValueText, IntToString(this.m_newValue));
  if IsDefined(this.m_sliderController) { this.m_sliderController.ChangeValue(Cast<Float>(this.m_newValue)); };
}
@wrapMethod(SettingsSelectorControllerInt)
private func ChangeValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    let step: Int32 = forward ? this.m_bwmsStep : -this.m_bwmsStep;
    this.m_newValue = Clamp(this.m_newValue + step, this.m_bwmsMin, this.m_bwmsMax);
    this.BWMSPaintInt();
  } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerInt)
private func AcceptValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.ChangeValue(forward); } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerInt)
public func Refresh() -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaintInt(); } else { wrappedMethod(); };
}
@wrapMethod(SettingsSelectorControllerInt)
protected cb func OnHandleReleased() -> Bool {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaintInt(); return true; };
  return wrappedMethod();
}
@wrapMethod(SettingsSelectorControllerInt)
protected cb func OnUpdateValue() -> Bool {
  if !IsDefined(this.m_SettingsEntry) { return true; };
  return wrappedMethod();
}

// ===== Float: slider (framework) =====
@addMethod(SettingsSelectorControllerFloat)
public func BWMSSetupFloat(label: String, mn: Float, mx: Float, step: Float, cur: Float) -> Void {
  this.m_bwmsMinF = mn; this.m_bwmsMaxF = mx; this.m_bwmsStepF = step;
  this.m_newValue = cur;
  inkTextRef.SetText(this.m_LabelText, label);
  this.m_sliderController = inkWidgetRef.GetControllerByType(this.m_sliderWidget, n"inkSliderController") as inkSliderController;
  if IsDefined(this.m_sliderController) {
    this.m_sliderController.Setup(mn, mx, cur, step);
    this.m_sliderController.RegisterToCallback(n"OnSliderValueChanged", this, n"OnSliderValueChanged");
    this.m_sliderController.RegisterToCallback(n"OnSliderHandleReleased", this, n"OnHandleReleased");
  };
  this.BWMSPaintFloat();
}
@addMethod(SettingsSelectorControllerFloat)
public func BWMSPaintFloat() -> Void {
  inkTextRef.SetText(this.m_ValueText, FloatToStringPrec(this.m_newValue, 2));
  if IsDefined(this.m_sliderController) { this.m_sliderController.ChangeValue(this.m_newValue); };
}
@wrapMethod(SettingsSelectorControllerFloat)
private func ChangeValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    let step: Float = forward ? this.m_bwmsStepF : -this.m_bwmsStepF;
    this.m_newValue = ClampF(this.m_newValue + step, this.m_bwmsMinF, this.m_bwmsMaxF);
    this.BWMSPaintFloat();
  } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerFloat)
private func AcceptValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.ChangeValue(forward); } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerFloat)
public func Refresh() -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaintFloat(); } else { wrappedMethod(); };
}
@wrapMethod(SettingsSelectorControllerFloat)
protected cb func OnHandleReleased() -> Bool {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaintFloat(); return true; };
  return wrappedMethod();
}
@wrapMethod(SettingsSelectorControllerFloat)
protected cb func OnUpdateValue() -> Bool {
  if !IsDefined(this.m_SettingsEntry) { return true; };
  return wrappedMethod();
}

// ===== StringList: lista por dots (framework) =====
@addMethod(SettingsSelectorControllerListString)
public func BWMSSetupList(label: String, elems: array<String>, idx: Int32) -> Void {
  this.m_bwmsElems = elems;
  this.m_bwmsIdx = idx;
  inkTextRef.SetText(this.m_LabelText, label);
  this.PopulateDots(ArraySize(elems));
  this.BWMSPaintList();
}
@addMethod(SettingsSelectorControllerListString)
public func BWMSPaintList() -> Void {
  if this.m_bwmsIdx >= 0 && this.m_bwmsIdx < ArraySize(this.m_bwmsElems) {
    inkTextRef.SetText(this.m_ValueText, this.m_bwmsElems[this.m_bwmsIdx]);
  };
  this.SelectDot(this.m_bwmsIdx);
}
@wrapMethod(SettingsSelectorControllerListString)
private func ChangeValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    let n: Int32 = ArraySize(this.m_bwmsElems);
    if n > 0 {
      this.m_bwmsIdx = (this.m_bwmsIdx + (forward ? 1 : -1) + n) % n;
      this.BWMSPaintList();
    };
  } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerListString)
public func Refresh() -> Void {
  if !IsDefined(this.m_SettingsEntry) { this.BWMSPaintList(); } else { wrappedMethod(); };
}

// ===== base: derefs de m_SettingsEntry — clique unico =====
@wrapMethod(SettingsSelectorController)
protected cb func OnHoverOver(e: ref<inkPointerEvent>) -> Bool { if !IsDefined(this.m_SettingsEntry) { return true; }; return wrappedMethod(e); }
@wrapMethod(SettingsSelectorController)
protected cb func OnHoverOut(e: ref<inkPointerEvent>) -> Bool { if !IsDefined(this.m_SettingsEntry) { return true; }; return wrappedMethod(e); }
@wrapMethod(SettingsSelectorController)
protected cb func OnLeft(e: ref<inkPointerEvent>) -> Bool {
  if !IsDefined(this.m_SettingsEntry) { if e.IsAction(n"click") { this.AcceptValue(false); this.PlaySound(n"ButtonValueDown", n"OnPress"); }; return true; };
  return wrappedMethod(e);
}
@wrapMethod(SettingsSelectorController)
protected cb func OnRight(e: ref<inkPointerEvent>) -> Bool {
  if !IsDefined(this.m_SettingsEntry) { if e.IsAction(n"click") { this.AcceptValue(true); this.PlaySound(n"ButtonValueUp", n"OnPress"); }; return true; };
  return wrappedMethod(e);
}
@wrapMethod(SettingsSelectorController)
protected cb func OnShortcutPress(e: ref<inkPointerEvent>) -> Bool {
  if !IsDefined(this.m_SettingsEntry) { return true; };
  return wrappedMethod(e);
}
@wrapMethod(SettingsSelectorController)
protected cb func OnShortcutRepeat(e: ref<inkPointerEvent>) -> Bool {
  if !IsDefined(this.m_SettingsEntry) { return true; };
  return wrappedMethod(e);
}
