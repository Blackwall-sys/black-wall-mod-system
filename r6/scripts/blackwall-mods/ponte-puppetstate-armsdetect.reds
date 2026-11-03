// ponte-puppetstate-armsdetect.reds — ArchiveXL #29 `PuppetStateHandler` (2026-09-12).
//
// Port do `PuppetState/Handler.cpp`: um callback de slot por boneco recebe `OnItemEquippedComplete`/`OnItemUnequippedComplete`, guarda o
// estado dos braços (cyberware de braço pelo `.cyberwareType` do item equipado nas mãos) e dos pés (1º item visível dos slots de pé) e
// reprocessa a aparência dos itens que dependem desse estado. O listener nativo de script do Mac (`gameAttachmentSlotsScriptListener`)
// repassa os 13 eventos de slot POR NOME pro objeto de script, com `(slot, item)` — RE 2026-09-12: cada método do listener filtra pelo
// `slotID`/`itemID` do callback, carrega o CName do evento (`OnItemEquippedComplete` em `0x10352229c`) e chama a função de mesmo nome
// do script. Por isso o handler é redscript puro, sem hook.
//
// Divergências conscientes da referência:
// - quem ganha handler: o V, pelo `BwmsAlwaysRun_estado_do_boneco`, que o `BwmsAoEntrarNoJogo` chama na entrada no jogo. A
//   referência cria um por `GenitalsController::OnAttach`, que no Mac não dispara.
// - visibilidade: `TransactionSystem.GetItemAppearance(dono, item)`, o nome que o `EquipmentSystem` troca por
//   `empty_appearance_default` pra esconder a peça, no lugar da vfunc `ItemObject::GetAppearanceName`.
// - slots relacionados e dependentes: os da referência sem slot de mod (Torso e Feet sozinhos, dependentes Torso→Chest e Feet→Legs).
//   Os slots de mod (`.parentSlot`/`.dependencySlots`, que o TweakDB base não tem) não entram: varrer os registros `AttachmentSlot`
//   daqui exigiria o `BwmsGetRecordsArray`, cujo array local depende do trailer emprestado dos componentes do player.

public enum PuppetArmsState {
  BaseArms = 0,
  MantisBlades = 1,
  Monowire = 2,
  ProjectileLauncher = 3
}

public enum PuppetFeetState {
  None = 0,
  Flat = 1,
  Lifted = 2,
  HighHeels = 3,
  FlatShoes = 4
}

public class PuppetStateHandler extends AttachmentSlotsScriptCallback {

  private let m_puppet: wref<GameObject>;

  private let m_torsoSlots: array<TweakDBID>;

  private let m_feetSlots: array<TweakDBID>;

  private let m_headDependentSlots: array<TweakDBID>;

  private let m_torsoDependentSlots: array<TweakDBID>;

  private let m_handsDependentSlots: array<TweakDBID>;

  private let m_feetDependentSlots: array<TweakDBID>;

  private let m_pendingRefreshes: array<TweakDBID>;

  private let m_armsState: PuppetArmsState;

  private let m_feetState: PuppetFeetState;

  private let m_gender: CName;

  public static func Create(puppet: ref<GameObject>) -> ref<PuppetStateHandler> {
    let handler: ref<PuppetStateHandler> = new PuppetStateHandler();
    handler.m_puppet = puppet;
    handler.m_armsState = PuppetArmsState.BaseArms;
    handler.m_feetState = PuppetFeetState.None;
    handler.CollectSlots();
    handler.UpdateFeetState(puppet);
    return handler;
  }

  // O handler do dono, se ele tiver um.
  public static func ForOwner(owner: wref<GameObject>) -> ref<PuppetStateHandler> {
    let obj: ref<GameObject> = owner;
    let player: ref<PlayerPuppet> = obj as PlayerPuppet;
    if IsDefined(player) {
      return player.m_puppetStateHandler;
    };
    return null;
  }

  public final func GetArmsState() -> PuppetArmsState {
    return this.m_armsState;
  }

  public final func GetFeetState() -> PuppetFeetState {
    return this.m_feetState;
  }

  // O valor de sufixo de cada estado: o nome do membro do enum, e vazio pro `None` dos pés.
  public static func ArmsStateName(state: PuppetArmsState) -> String {
    if Equals(state, PuppetArmsState.BaseArms) {
      return "BaseArms";
    };
    if Equals(state, PuppetArmsState.MantisBlades) {
      return "MantisBlades";
    };
    if Equals(state, PuppetArmsState.Monowire) {
      return "Monowire";
    };
    if Equals(state, PuppetArmsState.ProjectileLauncher) {
      return "ProjectileLauncher";
    };
    return "";
  }

  public static func FeetStateName(state: PuppetFeetState) -> String {
    if Equals(state, PuppetFeetState.Flat) {
      return "Flat";
    };
    if Equals(state, PuppetFeetState.Lifted) {
      return "Lifted";
    };
    if Equals(state, PuppetFeetState.HighHeels) {
      return "HighHeels";
    };
    if Equals(state, PuppetFeetState.FlatShoes) {
      return "FlatShoes";
    };
    return "";
  }

  public func OnItemEquippedComplete(slot: TweakDBID, item: ItemID) -> Void {
    this.FinalizeAppearanceChange(item, slot, true);
  }

  public func OnItemUnequippedComplete(slot: TweakDBID, item: ItemID) -> Void {
    this.FinalizeAppearanceChange(item, slot, false);
  }

  private final func FinalizeAppearanceChange(item: ItemID, slot: TweakDBID, equipped: Bool) -> Void {
    let puppet: ref<GameObject> = this.m_puppet;
    if !IsDefined(puppet) {
      return;
    };
    let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(puppet.GetGame());
    if !IsDefined(ts) {
      return;
    };
    if this.PullPendingRefresh(slot) {
      let pending: ref<ItemObject> = ts.GetItemInSlot(puppet, slot);
      if IsDefined(pending) && this.IsVisible(ts, puppet, pending) {
        this.RefreshItemAppearance(ts, puppet, pending);
      };
    };
    if slot == t"AttachmentSlots.WeaponRight" || slot == t"AttachmentSlots.WeaponLeft" {
      if this.UpdateArmsState(item, equipped) {
        this.RefreshDependentAppearances(ts, puppet, this.m_handsDependentSlots, t"itemsFactoryAppearanceSuffix.ArmsState", TDBID.None());
      };
    };
    if ArrayContains(this.m_feetSlots, slot) || ts.MatchVisualTagByItemID(item, puppet, n"hide_S1") {
      if this.UpdateFeetState(puppet) {
        this.RefreshDependentAppearances(ts, puppet, this.m_feetDependentSlots, t"itemsFactoryAppearanceSuffix.FeetState", t"itemsFactoryAppearanceSuffix.LegsState");
      };
    };
    if ArrayContains(this.m_torsoSlots, slot) && ts.MatchVisualTagByItemID(item, puppet, n"hide_T1part") {
      this.RefreshDependentAppearances(ts, puppet, this.m_torsoDependentSlots, t"itemsFactoryAppearanceSuffix.Partial", TDBID.None());
    };
    if slot == t"AttachmentSlots.TppHead" {
      this.RefreshDependentAppearances(ts, puppet, this.m_headDependentSlots, t"itemsFactoryAppearanceSuffix.Camera", TDBID.None());
    };
  }

  private final func UpdateArmsState(item: ItemID, equipped: Bool) -> Bool {
    let state: PuppetArmsState = PuppetStateHandler.ResolveArmsState(item, equipped);
    let updated: Bool = NotEquals(this.m_armsState, state);
    this.m_armsState = state;
    return updated;
  }

  // Desequipar volta pra `BaseArms` sem olhar o item, como a referência. O `.cyberwareType` do monowire é `NanoWires`.
  public static func ResolveArmsState(item: ItemID, equipped: Bool) -> PuppetArmsState {
    if equipped {
      let flat: TweakDBID = ItemID.GetTDBID(item);
      TDBID.Append(flat, t".cyberwareType");
      let cyberwareType: CName = TweakDBInterface.GetCName(flat, n"None");
      if Equals(cyberwareType, n"MantisBlades") {
        return PuppetArmsState.MantisBlades;
      };
      if Equals(cyberwareType, n"NanoWires") {
        return PuppetArmsState.Monowire;
      };
      if Equals(cyberwareType, n"ProjectileLauncher") {
        return PuppetArmsState.ProjectileLauncher;
      };
    };
    return PuppetArmsState.BaseArms;
  }

  public final func UpdateFeetState(puppet: ref<GameObject>) -> Bool {
    let state: PuppetFeetState = this.ResolveFeetState(puppet);
    let updated: Bool = NotEquals(this.m_feetState, state);
    this.m_feetState = state;
    return updated;
  }

  // Corpo masculino não tem estado de pé. No feminino o 1º item visível dos slots de pé decide e a busca para nele; nada visível é `Flat`.
  private final func ResolveFeetState(puppet: ref<GameObject>) -> PuppetFeetState {
    if this.IsMale() {
      return PuppetFeetState.None;
    };
    if !IsDefined(puppet) {
      return PuppetFeetState.Flat;
    };
    let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(puppet.GetGame());
    if !IsDefined(ts) {
      return PuppetFeetState.Flat;
    };
    let i: Int32 = 0;
    while i < ArraySize(this.m_feetSlots) {
      let itemObject: ref<ItemObject> = ts.GetItemInSlot(puppet, this.m_feetSlots[i]);
      if IsDefined(itemObject) && this.IsVisible(ts, puppet, itemObject) {
        if ts.MatchVisualTag(itemObject, n"HighHeels", false) {
          return PuppetFeetState.HighHeels;
        };
        if ts.MatchVisualTag(itemObject, n"FlatShoes", false) || ts.MatchVisualTag(itemObject, n"force_FlatFeet", false) {
          return PuppetFeetState.FlatShoes;
        };
        return PuppetFeetState.Lifted;
      };
      i += 1;
    };
    return PuppetFeetState.Flat;
  }

  private final func IsMale() -> Bool {
    if !IsNameValid(this.m_gender) {
      this.m_gender = PuppetStateHandler.IsMaleEntity(this.m_puppet) ? n"Male" : n"Female";
    };
    return Equals(this.m_gender, n"Male");
  }

  // Componentes de trás pra frente: o 1º corpo achado decide, e sem corpo nenhum é feminino.
  public static func IsMaleEntity(entity: ref<Entity>) -> Bool {
    if !IsDefined(entity) {
      return false;
    };
    let components: array<ref<IComponent>> = BwmsEntityGetComponents(entity);
    let i: Int32 = ArraySize(components) - 1;
    while i >= 0 {
      if IsDefined(components[i]) {
        let name: CName = components[i].GetName();
        if Equals(name, n"t0_000_pma_base__full") {
          return true;
        };
        if Equals(name, n"t0_000_pwa_base__full") || Equals(name, n"t0_000_pwa_fpp__torso") {
          return false;
        };
      };
      i -= 1;
    };
    return false;
  }

  private final func IsVisible(ts: ref<TransactionSystem>, puppet: ref<GameObject>, itemObject: ref<ItemObject>) -> Bool {
    let appearance: CName = ts.GetItemAppearance(puppet, itemObject.GetItemID());
    return IsNameValid(appearance) && NotEquals(appearance, n"empty_appearance_default");
  }

  // Aparência dinâmica tem o marcador de contexto `%` no nome.
  private final func IsDynamicAppearance(ts: ref<TransactionSystem>, puppet: ref<GameObject>, itemObject: ref<ItemObject>) -> Bool {
    let appearance: CName = ts.GetItemAppearance(puppet, itemObject.GetItemID());
    return StrContains(NameToString(appearance), "%");
  }

  public static func AppearanceSuffixes(item: ItemID) -> array<TweakDBID> {
    let flat: TweakDBID = ItemID.GetTDBID(item);
    TDBID.Append(flat, t".appearanceSuffixes");
    return TweakDBInterface.GetForeignKeyArray(flat);
  }

  private static func ReactsToSuffix(itemObject: ref<ItemObject>, suffixA: TweakDBID, suffixB: TweakDBID) -> Bool {
    let suffixes: array<TweakDBID> = PuppetStateHandler.AppearanceSuffixes(itemObject.GetItemID());
    if ArrayContains(suffixes, suffixA) {
      return true;
    };
    return TDBID.IsValid(suffixB) && ArrayContains(suffixes, suffixB);
  }

  private final func RefreshDependentAppearances(ts: ref<TransactionSystem>, puppet: ref<GameObject>, slots: array<TweakDBID>, suffixA: TweakDBID, suffixB: TweakDBID) -> Void {
    let i: Int32 = 0;
    while i < ArraySize(slots) {
      let itemObject: ref<ItemObject> = ts.GetItemInSlot(puppet, slots[i]);
      if IsDefined(itemObject) {
        if this.IsVisible(ts, puppet, itemObject) && (this.IsDynamicAppearance(ts, puppet, itemObject) || PuppetStateHandler.ReactsToSuffix(itemObject, suffixA, suffixB)) {
          this.RefreshItemAppearance(ts, puppet, itemObject);
        };
      } else {
        if ts.IsSlotSpawningAnyItem(puppet, slots[i]) {
          this.AddPendingRefresh(slots[i]);
        };
      };
      i += 1;
    };
  }

  private final func RefreshItemAppearance(ts: ref<TransactionSystem>, puppet: ref<GameObject>, itemObject: ref<ItemObject>) -> Void {
    ts.ResetItemAppearance(puppet, itemObject.GetItemID());
  }

  private final func AddPendingRefresh(slot: TweakDBID) -> Void {
    if !ArrayContains(this.m_pendingRefreshes, slot) {
      ArrayPush(this.m_pendingRefreshes, slot);
    };
  }

  private final func PullPendingRefresh(slot: TweakDBID) -> Bool {
    return ArrayRemove(this.m_pendingRefreshes, slot);
  }

  // Os conjuntos da referência sem slot de mod.
  private final func CollectSlots() -> Void {
    ArrayPush(this.m_torsoSlots, t"AttachmentSlots.Torso");
    ArrayPush(this.m_feetSlots, t"AttachmentSlots.Feet");
    ArrayPush(this.m_torsoDependentSlots, t"AttachmentSlots.Chest");
    ArrayPush(this.m_feetDependentSlots, t"AttachmentSlots.Legs");
  }
}

// O V guarda o handler e o listener: o sistema de transação só observa, e o `OnDetach` desregistra como o vanilla faz com o listener da mão.
@addField(PlayerPuppet)
public let m_puppetStateHandler: ref<PuppetStateHandler>;

@addField(PlayerPuppet)
public let m_puppetStateListener: ref<AttachmentSlotsScriptListener>;

@addMethod(PlayerPuppet)
public func BwmsAlwaysRun_estado_do_boneco() -> Void {
  if IsDefined(this.m_puppetStateHandler) {
    return;
  };
  let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(this.GetGame());
  if !IsDefined(ts) {
    return;
  };
  let handler: ref<PuppetStateHandler> = PuppetStateHandler.Create(this);
  this.m_puppetStateHandler = handler;
  this.m_puppetStateListener = ts.RegisterAttachmentSlotListener(this, handler);
}

@wrapMethod(PlayerPuppet)
protected cb func OnDetach() -> Bool {
  if IsDefined(this.m_puppetStateListener) {
    let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(this.GetGame());
    if IsDefined(ts) {
      ts.UnregisterAttachmentSlotListener(this, this.m_puppetStateListener);
    };
  };
  this.m_puppetStateListener = null;
  this.m_puppetStateHandler = null;
  return wrappedMethod();
}
