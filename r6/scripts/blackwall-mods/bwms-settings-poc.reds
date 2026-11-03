// BWMS — pagina nativa (Mods > Cheats) via bypass do ConfigVar. 100% redscript, sem CET, sem NativeSettings.
// Aba "Cheats": os cheats do BWMS (Modo Imortal + fáceis) e o seletor "Pular boot". Carga, dano, RAM e câmera lenta guardam estado em
// campos do PlayerPuppet (persiste ao reabrir); Invencível e Invisível leem o estado do próprio jogo, que vai e volta com o save.
// Gate = !IsDefined(this.m_SettingsEntry): nossos controllers tem entry null; os do jogo nao.
// Clique unico: OnShortcutPress/OnShortcutRepeat neutralizados pros nossos (corpo/setas ja alteram).

// Seletor "Pular boot" (kind 3): grava a escolha no `red4ext/bwms.toml` (`pular_intro`, `continuar_automatico`), que vale no próximo
// boot. Níveis 1 E 2 armam o lever zero-input (o save-system ativa de verdade nos dois); só o nível 2 carrega o save sozinho
// (ver BWMSApplyBoot).
native func BwmsAutoContinueOn() -> Bool;
native func BwmsAutoContinueOff() -> Bool;
native func BwmsFireStartOn() -> Bool;
native func BwmsFireStartOff() -> Bool;
// redscript-mod-persistence: config fora do save (~/.bwms-modconfig.txt), sobrevive ao reboot.
native func BwmsConfigGet(key: String) -> String;
native func BwmsConfigSet(key: String, value: String) -> Bool;

// Contrato ÚNICO de extensão p/ mods de 3os (redscript puro, dispatch por método virtual — sem
// tocar RTTI/native, sem @wrapMethod em BWMSRun/BWMSIsOn): um cheatId numérico (nosso, switch
// legado abaixo, intocado) OU um handler de objeto (novo, pro 3o). Se handler != null, ele MANDA;
// senão cai no switch de sempre. Um 3o soma 1 cheat com efeito próprio via 1 ArrayPush em
// BWMSCheats() (subclasse de BWMSCheatHandler) — sem tocar/wrapar nenhum .reds do BWMS.
public abstract class BWMSCheatHandler {
  public func BWMSOnToggle(pp: ref<PlayerPuppet>, game: GameInstance) -> Void {}
  public func BWMSOnQuery(pp: ref<PlayerPuppet>, game: GameInstance) -> Bool { return false; }
  // 2026-08-05 (achado de auditoria): kind=2 (setas ±) só funcionava pros 16 cheats internos
  // (id+switch fixo em BWMSDoAction) — um mod de 3o só conseguia registrar toggle/ação (kind
  // 0/1). Widget mais rico pro contrato: BWMSOnArrow dispara a cada clique de seta (forward=
  // direita/soma), BWMSArrowValue devolve o texto exibido (ex. "Nv 5", "+10"). Default no-op —
  // zero quebra pra handler existente que só implementa Toggle/Query.
  public func BWMSOnArrow(pp: ref<PlayerPuppet>, game: GameInstance, forward: Bool) -> Void {}
  public func BWMSArrowValue(pp: ref<PlayerPuppet>, game: GameInstance) -> String { return ""; }
}

@addField(SettingsSelectorControllerBool) let m_bwmsGame: GameInstance;
@addField(SettingsSelectorControllerBool) let m_bwmsCheat: Int32;
@addField(SettingsSelectorControllerBool) let m_bwmsIsToggle: Bool;
@addField(SettingsSelectorControllerBool) let m_bwmsHandler: ref<BWMSCheatHandler>;
@addField(SettingsSelectorControllerInt) let m_bwmsMin: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsMax: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsStep: Int32;
// 2026-08-06: kind=4 (slider Int, valor exato) — completa o framework BWMSSetupInt/BWMSPaintInt
// (já existia, nunca tinha sido ligado a um cheat real nem ao dispatcher `BWMSCheat`). Rastreia o
// valor ANTES do ajuste pra computar o delta certo no commit (GiveItem/RemoveItem trabalham por
// delta, não por valor absoluto).
@addField(SettingsSelectorControllerInt) let m_bwmsIntCheat: Int32;
@addField(SettingsSelectorControllerInt) let m_bwmsIntGame: GameInstance;
@addField(SettingsSelectorControllerInt) let m_bwmsIntOld: Int32;
@addField(SettingsSelectorControllerFloat) let m_bwmsMinF: Float;
@addField(SettingsSelectorControllerFloat) let m_bwmsMaxF: Float;
@addField(SettingsSelectorControllerFloat) let m_bwmsStepF: Float;
@addField(SettingsSelectorControllerListString) let m_bwmsElems: array<String>;
@addField(SettingsSelectorControllerListString) let m_bwmsIdx: Int32;
@addField(SettingsSelectorControllerListString) let m_bwmsAct: Int32;
@addField(SettingsSelectorControllerListString) let m_bwmsActGame: GameInstance;
@addField(SettingsSelectorControllerListString) let m_bwmsNet: Int32;
// Modo do seletor de lista: opções, o "Pular boot" ou setas ± de ação. A ação vem de cheat nosso (pelo id) ou de mod de terceiro (pelo
// handler, com id 0): o modo não pode sair do id.
public enum BwmsSeletorModo {
  Lista = 0,
  Boot = 1,
  Acao = 2,
}
@addField(SettingsSelectorControllerListString) let m_bwmsModo: BwmsSeletorModo;
// kind=2 (setas ±) pra handler de 3o — ver BWMSCheatHandler.BWMSOnArrow/BWMSArrowValue.
@addField(SettingsSelectorControllerListString) let m_bwmsArrowHandler: ref<BWMSCheatHandler>;

// estado de cheat persistido no proprio jogador (sobrevive a reabrir a aba; NAO vai pro save = runtime-only)
@addField(PlayerPuppet) let m_bwmsCarry: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsDmg: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsRam: ref<gameStatModifierData>;
@addField(PlayerPuppet) let m_bwmsSlow: Bool;
@addField(PlayerPuppet) let m_bwmsInvisPedido: Int32;
@addField(PlayerPuppet) let m_bwmsInvisPedidoEm: Float;

// Invisível: o sistema de efeitos só aplica e tira o Cloaked num passo seguinte da simulação (medido em jogo: o HasStatusEffect lido no
// mesmo quadro do Apply e do Remove devolve o valor anterior a eles), e com o menu aberto a simulação está parada. Até 1 s de simulação depois
// do clique vale o que o clique pediu (1 ligar, 2 desligar); depois, o estado do jogo. Sem isso a aba pinta o estado velho e o 2º clique
// aplica o efeito de novo.
@addMethod(PlayerPuppet)
public func BwmsInvisivelLigado() -> Bool {
  let real: Bool = GameInstance.GetStatusEffectSystem(this.GetGame()).HasStatusEffect(this.GetEntityID(), t"BaseStatusEffect.Cloaked");
  let agora: Float = EngineTime.ToFloat(GameInstance.GetSimTime(this.GetGame()));
  let pendente: Bool = this.m_bwmsInvisPedido != 0 && agora - this.m_bwmsInvisPedidoEm < 1.0;
  if pendente {
    return this.m_bwmsInvisPedido == 1;
  };
  return real;
}
@addMethod(PlayerPuppet)
public func BwmsInvisivelAlternar() -> Void {
  let ligado: Bool = this.BwmsInvisivelLigado();
  if ligado {
    StatusEffectHelper.RemoveStatusEffect(this, t"BaseStatusEffect.Cloaked");
    this.m_bwmsInvisPedido = 2;
  } else {
    StatusEffectHelper.ApplyStatusEffect(this, t"BaseStatusEffect.Cloaked");
    this.m_bwmsInvisPedido = 1;
  };
  this.m_bwmsInvisPedidoEm = EngineTime.ToFloat(GameInstance.GetSimTime(this.GetGame()));
}

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

// Registro DATA-DRIVEN de cheats. Os do BWMS usam `id`+switch (kind 0 a 4); mod de terceiro usa `handler` (kind 0/1/2, ver
// BWMSCheatHandler acima): 1 ArrayPush em BWMSCheats() com uma subclasse de BWMSCheatHandler, sem @wrapMethod em BWMSRun/BWMSIsOn
// (esquecer um dos 3 wraps deixava o cheat aparecer sem efeito ou estado). `kind` decide o render: 0=toggle Bool, 1=ação tiro-único
// Bool, 2=setas ± ListString, 3=seletor de boot (interno), 4=slider Int.
public struct BWMSCheatDef {
  public let label: String;
  public let id: Int32;
  public let kind: Int32;
  public let handler: ref<BWMSCheatHandler>;
}
@addMethod(SettingsMainGameController)
private final func BWMSDef(label: String, id: Int32, opt kind: Int32) -> BWMSCheatDef {
  let d: BWMSCheatDef;
  d.label = label;
  d.id = id;
  d.kind = kind;
  return d;
}
// PONTO DE EXTENSÃO p/ 3os DE VERDADE (contrato único, sem @wrapMethod em BWMSRun/BWMSIsOn):
// 1 ArrayPush(this.BWMSCheats(), this.BWMSDefH(label, handlerInstance)) — kind 0=toggle,
// 1=ação tiro-único, 2=setas ± (BWMSOnArrow/BWMSArrowValue no handler, mesmo widget
// que os cheats internos usam). `id` fica 0 (não usado — quem manda é o handler).
@addMethod(SettingsMainGameController)
public final func BWMSDefH(label: String, handler: ref<BWMSCheatHandler>, opt kind: Int32) -> BWMSCheatDef {
  let d: BWMSCheatDef;
  d.label = label;
  d.id = 0;
  d.kind = kind;
  d.handler = handler;
  return d;
}
// PONTO DE EXTENSÃO: o array de cheats da aba (os do BWMS). Mod de 3o faz @wrapMethod e dá
// ArrayPush no resultado pra somar o dele — via BWMSDefH (handler, SEM tocar switch nenhum) ou,
// pro estilo antigo, via BWMSDef+id (aí precisa @wrapMethod BWMSRun/BWMSIsOn pro id novo).
@addMethod(SettingsMainGameController)
public func BWMSCheats() -> array<BWMSCheatDef> {
  let c: array<BWMSCheatDef>;
  ArrayPush(c, this.BWMSDef(this.L("Invincible (God Mode)", "Invencível (Modo Imortal)", "无敌（上帝模式）"), 1));
  ArrayPush(c, this.BWMSDef(this.L("Infinite carry weight", "Carga infinita", "无限负重"), 2));
  ArrayPush(c, this.BWMSDef(this.L("Massive damage (+1000%)", "Dano massivo (+1000%)", "巨额伤害（+1000%）"), 3));
  ArrayPush(c, this.BWMSDef(this.L("Infinite cyberdeck RAM", "RAM do cyberdeck infinita", "无限赛博硬件内存"), 4));
  ArrayPush(c, this.BWMSDef(this.L("Slow motion", "Câmera lenta", "慢动作"), 5));
  ArrayPush(c, this.BWMSDef(this.L("Invisibility", "Invisibilidade", "隐身"), 10));
  ArrayPush(c, this.BWMSDef(this.L("Eddies (arrows: +/- 10,000)", "Eddies (seta: +/- 10.000)", "欧元币（箭头：±10,000）"), 6, 2));
  ArrayPush(c, this.BWMSDef(this.L("Attribute Point (arrows: +/- 1)", "Ponto de Atributo (seta: +/- 1)", "属性点（箭头：±1）"), 7, 2));
  ArrayPush(c, this.BWMSDef(this.L("Perk Point (arrows: +/- 1)", "Ponto de Perk (seta: +/- 1)", "专长点（箭头：±1）"), 8, 2));
  ArrayPush(c, this.BWMSDef(this.L("Street Cred (arrows: +/- 1 level)", "Street Cred (seta: +/- 1 nível)", "街头声望（箭头：±1级）"), 13, 2));
  ArrayPush(c, this.BWMSDef(this.L("Unlock all vehicles", "Desbloquear todos os veículos", "解锁所有载具"), 9, 1));
  ArrayPush(c, this.BWMSDef(this.L("Clear wanted level", "Zerar nível de procurado", "清除通缉等级"), 11, 1));
  ArrayPush(c, this.BWMSDef(this.L("Set time to noon", "Ajustar hora p/ meio-dia", "时间设为正午"), 12, 1));
  ArrayPush(c, this.BWMSDef(this.L("Summon vehicle", "Chamar veículo", "召唤载具"), 14, 1));
  ArrayPush(c, this.BWMSDef(this.L("Skip boot (next boot)", "Pular boot (próx. boot)", "跳过启动（下次启动）"), 15, 3));
  ArrayPush(c, this.BWMSDef(this.L("Full heal", "Curar (vida cheia)", "满血治疗"), 16, 1));
  // kind=4: slider Int (valor exato, seed = quantia real via GetItemQuantity)
  ArrayPush(c, this.BWMSDef(this.L("Eddies (exact value)", "Eddies (valor exato)", "欧元币（精确数值）"), 17, 4));
  return c;
}

@addMethod(SettingsMainGameController)
private final func BWMSCheat(label: String, cheatId: Int32, kind: Int32, game: GameInstance, opt handler: ref<BWMSCheatHandler>) -> Void {
  if kind == 3 {
    let elems: array<String>;
    ArrayPush(elems, this.L("Off", "Desligado", "关闭"));
    ArrayPush(elems, this.L("To the menu", "Até o menu", "到主菜单"));
    ArrayPush(elems, this.L("To gameplay", "Até a gameplay", "到游戏内"));
    let cboot: ref<SettingsSelectorControllerListString> =
      this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsSelectorStringList")
          .GetController() as SettingsSelectorControllerListString;
    if IsDefined(cboot) { cboot.BWMSSetupBoot(label, elems); ArrayPush(this.m_settingsElements, cboot); };
    return;
  };
  if kind == 2 {
    let cl: ref<SettingsSelectorControllerListString> =
      this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsSelectorStringList")
          .GetController() as SettingsSelectorControllerListString;
    if IsDefined(cl) { cl.BWMSSetupAction(label, cheatId, game, handler); ArrayPush(this.m_settingsElements, cl); };
    return;
  };
  if kind == 4 {
    let pl: ref<GameObject> = GameInstance.GetPlayerSystem(game).GetLocalPlayerControlledGameObject();
    let cur: Int32 = 0;
    if IsDefined(pl) {
      cur = GameInstance.GetTransactionSystem(game).GetItemQuantity(pl, ItemID.FromTDBID(t"Items.money"));
    };
    let ci: ref<SettingsSelectorControllerInt> =
      this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsSelectorInt")
          .GetController() as SettingsSelectorControllerInt;
    if IsDefined(ci) { ci.BWMSSetupInt(label, 0, 999999999, 1000, cur, cheatId, game); ArrayPush(this.m_settingsElements, ci); };
    return;
  };
  let cb: ref<SettingsSelectorControllerBool> =
    this.SpawnFromLocal(inkWidgetRef.Get(this.m_settingsOptionsList), n"settingsSelectorBool")
        .GetController() as SettingsSelectorControllerBool;
  if IsDefined(cb) { cb.BWMSSetupBool(label, cheatId, kind == 0, game, handler); ArrayPush(this.m_settingsElements, cb); };
}

@wrapMethod(SettingsMainGameController)
private final func PopulateCategorySettingsOptions(idx: Int32) -> Void {
  let realIdx: Int32 = idx < 0 ? this.m_groupsSelectorCtrl.GetToggledIndex() : idx;
  if realIdx < 0 || realIdx >= ArraySize(this.m_data) { wrappedMethod(idx); return; };
  let gp: CName = this.m_data[realIdx].groupPath;

  if Equals(gp, n"/bwms") {
    ArrayClear(this.m_settingsElements);
    inkCompoundRef.RemoveAllChildren(this.m_settingsOptionsList);
    inkTextRef.SetText(this.m_descriptionText, ""); inkWidgetRef.SetVisible(this.m_descriptionText, false);
    let pl: ref<GameObject> = this.GetPlayerControlledObject();
    if IsDefined(pl) {
      let g: GameInstance = pl.GetGame();
      // data-driven: itera o registro (extensível por 3os via @wrapMethod BWMSCheats)
      let defs: array<BWMSCheatDef> = this.BWMSCheats();
      let i: Int32 = 0;
      while i < ArraySize(defs) {
        this.BWMSCheat(defs[i].label, defs[i].id, defs[i].kind, g, defs[i].handler);
        i += 1;
      };
    } else {
      this.BWMSDocLine(this.L("Load a save to use the cheats (you need a living V).", "Carregue um save para usar os cheats (precisa de um V vivo).", "载入存档后才能使用作弊（需要存活的 V）。"));
    };
    this.m_groupsSelectorCtrl.SetSelectedIndex(realIdx);
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
    this.m_groupsSelectorCtrl.SetSelectedIndex(realIdx);
    return;
  };

  wrappedMethod(idx);
}

// ===== Bool selector = despachante de CHEAT =====
// cheatId: 1 GodMode 2 Carga 3 Dano 4 RAM 5 Slow 10 Invisível (kind 0 = toggle Bool)
//        | 9 Veiculos 11 Zerar-procurado 12 Meio-dia 14 Chamar-veiculo 16 Curar (kind 1 = ação Bool)
//        | 6 Eddies 7 Atrib 8 Perk 13 StreetCred (kind 2 = setas ± no ListString, BWMSDoAction)
//        | 15 Pular-boot (kind 3 = seletor de boot, BWMSApplyBoot)
//        | 17 Eddies-valor-exato (kind 4 = slider Int, BWMSDoIntAction)
@addMethod(SettingsSelectorControllerBool)
public func BWMSPlayer() -> ref<GameObject> {
  return GameInstance.GetPlayerSystem(this.m_bwmsGame).GetLocalPlayerControlledGameObject();
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSSetupBool(label: String, cheatId: Int32, isToggle: Bool, game: GameInstance, opt handler: ref<BWMSCheatHandler>) -> Void {
  this.m_bwmsGame = game;
  this.m_bwmsCheat = cheatId;
  this.m_bwmsIsToggle = isToggle;
  this.m_bwmsHandler = handler;
  inkTextRef.SetText(this.m_LabelText, label);
  this.BWMSPaint();
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSIsToggle() -> Bool { return this.m_bwmsIsToggle; }
@addMethod(SettingsSelectorControllerBool)
public func BWMSIsOn() -> Bool {
  let p: ref<GameObject> = this.BWMSPlayer();
  let pp: ref<PlayerPuppet> = p as PlayerPuppet;
  if !IsDefined(pp) { return false; };
  if IsDefined(this.m_bwmsHandler) { return this.m_bwmsHandler.BWMSOnQuery(pp, this.m_bwmsGame); };
  switch this.m_bwmsCheat {
    case 1: return GameInstance.GetGodModeSystem(this.m_bwmsGame).HasGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable);
    case 2: return IsDefined(pp.m_bwmsCarry);
    case 3: return IsDefined(pp.m_bwmsDmg);
    case 4: return IsDefined(pp.m_bwmsRam);
    case 5: return pp.m_bwmsSlow;
    case 10: return pp.BwmsInvisivelLigado();
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
// Cheat "Zerar nível de procurado": o pedido de desescalada forçada do próprio jogo, o mesmo da máscara de cyberware (`UseCWMask`) e do evento
// de quest (`OnSetWantedLevel`, preventionSystem.script). As estrelas piscam e o sistema cai no Heat_0 pela limpeza dele (polícia, viaturas, os
// fatos `wanted_level`/`wanted_chase_active`), com o save travado enquanto há procurado (`IsSavingLocked`). Recusa com piso de quest (fora dos
// níveis padrão o `ChangeHeatStage` prende o Heat_0 no mínimo e a limpeza rodaria com o procurado de pé) e com o sistema travado (o
// `execInstructionSafe` sai antes do Heat_0 e as estrelas ficam piscando).
@addMethod(PreventionSystem)
public final static func BwmsZerarProcurado(game: GameInstance) -> Bool {
  let ps: ref<PreventionSystem> = GameInstance.GetScriptableSystemsContainer(game).Get(n"PreventionSystem") as PreventionSystem;
  if !IsDefined(ps) || !ps.IsChasingPlayer() || !ps.IsSystemEnabled() || ps.IsSystemLocked() {
    return false;
  };
  if !ps.m_defaultHeatLevels && NotEquals(ps.m_minHeatLevel, EPreventionHeatStage.Heat_0) {
    return false;
  };
  let pedido: ref<PreventionForceDeescalateRequest> = new PreventionForceDeescalateRequest();
  pedido.fakeBlinkingDuration = TweakDBInterface.GetFloat(t"PreventionSystem.setup.forcedDeescalationUIStarsBlinkingDurationSeconds", 4.00);
  pedido.telemetryInfo = "BWMS";
  ps.QueueRequest(pedido);
  return true;
}
@addMethod(SettingsSelectorControllerBool)
public func BWMSRun() -> Void {
  let p: ref<GameObject> = this.BWMSPlayer();
  let pp: ref<PlayerPuppet> = p as PlayerPuppet;
  if !IsDefined(pp) { return; };
  let game: GameInstance = this.m_bwmsGame;
  if IsDefined(this.m_bwmsHandler) { this.m_bwmsHandler.BWMSOnToggle(pp, game); return; };
  let ss: ref<StatsSystem> = GameInstance.GetStatsSystem(game);
  let soid: StatsObjectID = Cast<StatsObjectID>(pp.GetEntityID());
  switch this.m_bwmsCheat {
    case 1:
      if GameInstance.GetGodModeSystem(game).HasGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable) {
        BwmsGodModeDesligar(GameInstance.GetGodModeSystem(game), pp.GetEntityID());
      } else {
        GameInstance.GetGodModeSystem(game).AddGodMode(pp.GetEntityID(), gameGodModeType.Invulnerable, n"BWMS");
        BwmsConfigSet("godmode", "1"); // redscript-mod-persistence: sobrevive ao reboot, fora do save
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
    case 9:
      GameInstance.GetVehicleSystem(game).EnableAllPlayerVehicles();
      break;
    case 10:
      // Estado lido do sistema de efeitos, como o God Mode: o TweakDB marca o Cloaked como `savable`, ele volta com o save e um campo
      // nosso zeraria na carga, com a aba dizendo desligado e o V invisível. O atraso do sistema de efeitos fica no BwmsInvisivelLigado.
      pp.BwmsInvisivelAlternar();
      break;
    case 11:
      PreventionSystem.BwmsZerarProcurado(game);
      break;
    case 12:
      GameInstance.GetTimeSystem(game).SetGameTimeByHMS(12, 0, 0, n"BWMS");
      break;
    case 14:
      GameInstance.GetVehicleSystem(game).SpawnActivePlayerVehicle(gamedataVehicleType.Car);
      break;
    case 16:
      GameInstance.GetStatPoolsSystem(game).RequestSettingStatPoolValue(soid, gamedataStatPoolType.Health, 100.0, pp, true);
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
public func BWMSSetupInt(label: String, mn: Int32, mx: Int32, step: Int32, cur: Int32, opt cheatId: Int32, opt game: GameInstance) -> Void {
  this.m_bwmsMin = mn; this.m_bwmsMax = mx; this.m_bwmsStep = step;
  this.m_newValue = cur;
  this.m_bwmsIntCheat = cheatId;
  this.m_bwmsIntGame = game;
  this.m_bwmsIntOld = cur;
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
// Aplica o valor do slider no COMMIT (solta a alça) — delta contra o valor antigo, porque
// GiveItem/RemoveItem trabalham por quantidade a somar/tirar, não por valor absoluto.
@addMethod(SettingsSelectorControllerInt)
public func BWMSDoIntAction() -> Void {
  if this.m_bwmsIntCheat == 0 { return; };
  let delta: Int32 = this.m_newValue - this.m_bwmsIntOld;
  if delta == 0 { return; };
  let pp: ref<PlayerPuppet> =
    GameInstance.GetPlayerSystem(this.m_bwmsIntGame).GetLocalPlayerControlledGameObject() as PlayerPuppet;
  if !IsDefined(pp) { return; };
  switch this.m_bwmsIntCheat {
    case 17: // Eddies — valor exato
      if delta > 0 {
        GameInstance.GetTransactionSystem(this.m_bwmsIntGame).GiveItem(pp, ItemID.FromTDBID(t"Items.money"), delta);
      } else {
        GameInstance.GetTransactionSystem(this.m_bwmsIntGame).RemoveItem(pp, ItemID.FromTDBID(t"Items.money"), -delta);
      };
      break;
  };
  this.m_bwmsIntOld = this.m_newValue;
}
@wrapMethod(SettingsSelectorControllerInt)
private func ChangeValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    let step: Int32 = forward ? this.m_bwmsStep : -this.m_bwmsStep;
    this.m_newValue = Clamp(this.m_newValue + step, this.m_bwmsMin, this.m_bwmsMax);
    // clique de seta = passo único, sem "soltar alça" — aplica na hora (mesmo padrão do kind=2)
    this.BWMSDoIntAction();
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
  if !IsDefined(this.m_SettingsEntry) { this.BWMSDoIntAction(); this.BWMSPaintInt(); return true; };
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
// ===== StringList em modo BOOT: seletor "Pular boot" de 3 níveis =====
// idx 0=Desligado 1=Até o menu (com saves) 2=Até a gameplay. Estado inicial lido do bwms.toml;
// cada mudança re-aplica os 3 toggles (idempotente). 1 e 2 usam o MESMO lever (save-system real);
// só o nível 2 auto-carrega o save depois.
@addMethod(SettingsSelectorControllerListString)
public func BWMSSetupBoot(label: String, elems: array<String>) -> Void {
  this.m_bwmsModo = BwmsSeletorModo.Boot;
  this.m_bwmsElems = elems;
  // Retorno de native em local antes de decidir (inline dá resultado incoerente).
  let pulaIntro: Bool = BwmsSkipIntroState();
  let continua: Bool = BwmsAutoContinue();
  this.m_bwmsIdx = pulaIntro ? (continua ? 2 : 1) : 0;
  inkTextRef.SetText(this.m_LabelText, label);
  this.PopulateDots(ArraySize(elems));
  this.BWMSPaintList();
}
@addMethod(SettingsSelectorControllerListString)
public func BWMSApplyBoot() -> Void {
  if this.m_bwmsIdx == 0 { BwmsSkipIntroOff(); } else { BwmsSkipIntroOn(); };
  // Níveis 1 E 2 ligam o lever (fire-start) — os dois ativam o save-system de verdade
  // (CONTINUAR + lista de saves funcionando). Só o autocontinue muda: nível 2 carrega
  // sozinho, nível 1 para no menu com os saves prontos (ver BwmsTryContinue).
  if this.m_bwmsIdx >= 1 { BwmsFireStartOn(); } else { BwmsFireStartOff(); };
  if this.m_bwmsIdx == 2 { BwmsAutoContinueOn(); } else { BwmsAutoContinueOff(); };
}

// ===== StringList em modo AÇÃO (setas ±): seta direita soma, esquerda subtrai =====
// (recuperado do bundle de 25/jun via decompile — a "forma de setas" da aba Cheats)
@addMethod(SettingsSelectorControllerListString)
public func BWMSSetupAction(label: String, actId: Int32, game: GameInstance, opt handler: ref<BWMSCheatHandler>) -> Void {
  this.m_bwmsModo = BwmsSeletorModo.Acao;
  this.m_bwmsAct = actId;
  this.m_bwmsActGame = game;
  this.m_bwmsNet = 0;
  this.m_bwmsArrowHandler = handler;
  inkTextRef.SetText(this.m_LabelText, label);
  this.PopulateDots(0);
  this.BWMSPaintAction();
}
@addMethod(SettingsSelectorControllerListString)
public func BWMSPaintAction() -> Void {
  let v: String = "";
  if IsDefined(this.m_bwmsArrowHandler) {
    let pp: ref<PlayerPuppet> =
      GameInstance.GetPlayerSystem(this.m_bwmsActGame).GetLocalPlayerControlledGameObject() as PlayerPuppet;
    if IsDefined(pp) { v = this.m_bwmsArrowHandler.BWMSArrowValue(pp, this.m_bwmsActGame); };
    inkTextRef.SetText(this.m_ValueText, v);
    return;
  };
  if this.m_bwmsAct == 13 {
    let pp: ref<PlayerPuppet> =
      GameInstance.GetPlayerSystem(this.m_bwmsActGame).GetLocalPlayerControlledGameObject() as PlayerPuppet;
    if IsDefined(pp) {
      v = "Nv " + IntToString(PlayerDevelopmentSystem.GetData(pp).GetProficiencyLevel(gamedataProficiencyType.StreetCred));
    } else { v = "?"; };
  } else {
    if this.m_bwmsNet == 0 { v = "Add"; }
    else { v = (this.m_bwmsNet > 0 ? "+" : "") + IntToString(this.m_bwmsNet); };
  };
  inkTextRef.SetText(this.m_ValueText, v);
}
@addMethod(SettingsSelectorControllerListString)
public func BWMSDoAction(forward: Bool) -> Void {
  let pp: ref<PlayerPuppet> =
    GameInstance.GetPlayerSystem(this.m_bwmsActGame).GetLocalPlayerControlledGameObject() as PlayerPuppet;
  if !IsDefined(pp) { return; };
  let game: GameInstance = this.m_bwmsActGame;
  if IsDefined(this.m_bwmsArrowHandler) {
    this.m_bwmsArrowHandler.BWMSOnArrow(pp, game, forward);
    this.BWMSPaintAction();
    return;
  };
  let sign: Int32 = forward ? 1 : -1;
  switch this.m_bwmsAct {
    case 6:
      if forward {
        GameInstance.GetTransactionSystem(game).GiveItem(pp, ItemID.FromTDBID(t"Items.money"), 10000);
      } else {
        GameInstance.GetTransactionSystem(game).RemoveItem(pp, ItemID.FromTDBID(t"Items.money"), 10000);
      };
      this.m_bwmsNet += sign * 10000;
      break;
    case 7:
      PlayerDevelopmentSystem.GetData(pp).AddDevelopmentPoints(sign, gamedataDevelopmentPointType.Attribute);
      this.m_bwmsNet += sign;
      break;
    case 8:
      PlayerDevelopmentSystem.GetData(pp).AddDevelopmentPoints(sign, gamedataDevelopmentPointType.Primary);
      this.m_bwmsNet += sign;
      break;
    case 13:
      PlayerDevelopmentSystem.GetData(pp).SetLevel(
        gamedataProficiencyType.StreetCred,
        Clamp(PlayerDevelopmentSystem.GetData(pp).GetProficiencyLevel(gamedataProficiencyType.StreetCred) + sign, 0, 50),
        telemetryLevelGainReason.Gameplay);
      break;
  };
  this.BWMSPaintAction();
}
@wrapMethod(SettingsSelectorControllerListString)
private func ChangeValue(forward: Bool) -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    if Equals(this.m_bwmsModo, BwmsSeletorModo.Acao) {
      this.BWMSDoAction(forward);
    } else {
      let n: Int32 = ArraySize(this.m_bwmsElems);
      if n > 0 {
        this.m_bwmsIdx = (this.m_bwmsIdx + (forward ? 1 : -1) + n) % n;
        if Equals(this.m_bwmsModo, BwmsSeletorModo.Boot) { this.BWMSApplyBoot(); };
        this.BWMSPaintList();
      };
    };
  } else { wrappedMethod(forward); };
}
@wrapMethod(SettingsSelectorControllerListString)
public func Refresh() -> Void {
  if !IsDefined(this.m_SettingsEntry) {
    if Equals(this.m_bwmsModo, BwmsSeletorModo.Acao) { this.BWMSPaintAction(); } else { this.BWMSPaintList(); };
  } else { wrappedMethod(); };
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
