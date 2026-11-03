// ponte-puppetstate-armsdetect.reds — `PuppetStateHandler`: estado dos braços e dos pés do V para os sufixos de aparência.
//
// O listener de slots do motor entrega os eventos pelo NOME do método, com `(slot, item)`: por isso o handler recebe `OnItemEquippedComplete`
// mesmo sem o vanilla declarar esse método. Os nomes do enum viram o texto do sufixo que os arquivos dos mods casam.

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

  private let m_owner: wref<GameObject>;

  private let m_arms: PuppetArmsState;

  private let m_feet: PuppetFeetState;

  // `n""` até a primeira consulta; depois `n"Male"` ou `n"Female"`. Ler os componentes do corpo a cada evento custaria uma native por troca de roupa.
  private let m_gender: CName;

  // Slots dependentes cujo item ainda carregava quando o refresh foi pedido; o próximo evento do slot resolve.
  private let m_pendingSlots: array<TweakDBID>;

  public static func Create(puppet: ref<GameObject>) -> ref<PuppetStateHandler> {
    let handler: ref<PuppetStateHandler> = new PuppetStateHandler();
    handler.m_owner = puppet;
    handler.m_arms = PuppetArmsState.BaseArms;
    handler.m_feet = handler.ResolveFeetState(puppet);
    return handler;
  }

  public static func ForOwner(owner: wref<GameObject>) -> ref<PuppetStateHandler> {
    let dono: ref<GameObject> = owner;
    let player: ref<PlayerPuppet> = dono as PlayerPuppet;
    if IsDefined(player) {
      return player.m_puppetStateHandler;
    };
    return null;
  }

  public final func GetArmsState() -> PuppetArmsState {
    return this.m_arms;
  }

  public final func GetFeetState() -> PuppetFeetState {
    return this.m_feet;
  }

  public static func ArmsStateName(state: PuppetArmsState) -> String {
    switch state {
      case PuppetArmsState.BaseArms:
        return "BaseArms";
      case PuppetArmsState.MantisBlades:
        return "MantisBlades";
      case PuppetArmsState.Monowire:
        return "Monowire";
      case PuppetArmsState.ProjectileLauncher:
        return "ProjectileLauncher";
    };
    return "";
  }

  // `None` (masculino) não tem texto: o sufixo vazio é o que os arquivos dos mods esperam do corpo sem estado de pé.
  public static func FeetStateName(state: PuppetFeetState) -> String {
    switch state {
      case PuppetFeetState.Flat:
        return "Flat";
      case PuppetFeetState.Lifted:
        return "Lifted";
      case PuppetFeetState.HighHeels:
        return "HighHeels";
      case PuppetFeetState.FlatShoes:
        return "FlatShoes";
    };
    return "";
  }

  public func OnItemEquippedComplete(slot: TweakDBID, item: ItemID) -> Void {
    this.ProcessSlotEvent(slot, item, true);
  }

  public func OnItemUnequippedComplete(slot: TweakDBID, item: ItemID) -> Void {
    this.ProcessSlotEvent(slot, item, false);
  }

  // O jogo marca o braço pelo `cyberwareType` do record; o monowire do jogo é `NanoWires`. Braço gorila (`StrongArms`) não muda o modelo do braço.
  public static func ResolveArmsState(item: ItemID, equipped: Bool) -> PuppetArmsState {
    if !equipped {
      return PuppetArmsState.BaseArms;
    };
    let flat: TweakDBID = ItemID.GetTDBID(item);
    TDBID.Append(flat, t".cyberwareType");
    switch TweakDBInterface.GetCName(flat, n"None") {
      case n"MantisBlades":
        return PuppetArmsState.MantisBlades;
      case n"NanoWires":
        return PuppetArmsState.Monowire;
      case n"ProjectileLauncher":
        return PuppetArmsState.ProjectileLauncher;
    };
    return PuppetArmsState.BaseArms;
  }

  public final func UpdateFeetState(puppet: ref<GameObject>) -> Bool {
    let novo: PuppetFeetState = this.ResolveFeetState(puppet);
    if Equals(novo, this.m_feet) {
      return false;
    };
    this.m_feet = novo;
    return true;
  }

  // O corpo do V é um componente de nome fixo (`pma` masculino, `pwa` feminino); a busca vai do último componente ao primeiro.
  public static func IsMaleEntity(entity: ref<Entity>) -> Bool {
    if !IsDefined(entity) {
      return false;
    };
    let componentes: array<ref<IComponent>> = BwmsEntityGetComponents(entity);
    let i: Int32 = ArraySize(componentes) - 1;
    while i >= 0 {
      if IsDefined(componentes[i]) {
        let nome: CName = componentes[i].GetName();
        if Equals(nome, n"t0_000_pma_base__full") {
          return true;
        };
        if Equals(nome, n"t0_000_pwa_base__full") || Equals(nome, n"t0_000_pwa_fpp__torso") {
          return false;
        };
      };
      i -= 1;
    };
    return false;
  }

  public static func AppearanceSuffixes(item: ItemID) -> array<TweakDBID> {
    let flat: TweakDBID = ItemID.GetTDBID(item);
    TDBID.Append(flat, t".appearanceSuffixes");
    return TweakDBInterface.GetForeignKeyArray(flat);
  }

  // Conjuntos de slots: só os do jogo base. O TweakDB base não tem `parentSlot`/`dependencySlots`, então slot de mod não entra por herança.
  private static func TorsoSlots() -> array<TweakDBID> {
    return [t"AttachmentSlots.Torso"];
  }

  private static func FeetSlots() -> array<TweakDBID> {
    return [t"AttachmentSlots.Feet"];
  }

  private static func TorsoDependents() -> array<TweakDBID> {
    return [t"AttachmentSlots.Chest"];
  }

  private static func FeetDependents() -> array<TweakDBID> {
    return [t"AttachmentSlots.Legs"];
  }

  private static func HeadDependents() -> array<TweakDBID> {
    let nenhum: array<TweakDBID>;
    return nenhum;
  }

  private static func HandDependents() -> array<TweakDBID> {
    let nenhum: array<TweakDBID>;
    return nenhum;
  }

  private final func IsMale() -> Bool {
    if !IsNameValid(this.m_gender) {
      let dono: ref<GameObject> = this.m_owner;
      // Sem dono ainda não dá para decidir: não guarda, tenta de novo na próxima consulta.
      if !IsDefined(dono) {
        return false;
      };
      this.m_gender = PuppetStateHandler.IsMaleEntity(dono) ? n"Male" : n"Female";
    };
    return Equals(this.m_gender, n"Male");
  }

  // Masculino não tem estado de pé. Feminino: o primeiro item visível nos slots de pé decide.
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
    for slot in PuppetStateHandler.FeetSlots() {
      let calcado: ref<ItemObject> = ts.GetItemInSlot(puppet, slot);
      if IsDefined(calcado) && PuppetStateHandler.IsVisible(ts, puppet, calcado.GetItemID()) {
        if ts.MatchVisualTag(calcado, n"HighHeels", false) {
          return PuppetFeetState.HighHeels;
        };
        if ts.MatchVisualTag(calcado, n"FlatShoes", false) || ts.MatchVisualTag(calcado, n"force_FlatFeet", false) {
          return PuppetFeetState.FlatShoes;
        };
        return PuppetFeetState.Lifted;
      };
    };
    return PuppetFeetState.Flat;
  }

  // `empty_appearance_default` é a aparência com que o jogo esconde a peça (a prévia do inventário usa o mesmo nome).
  private static func IsVisible(ts: ref<TransactionSystem>, owner: ref<GameObject>, item: ItemID) -> Bool {
    let aparencia: CName = ts.GetItemAppearance(owner, item);
    return IsNameValid(aparencia) && !Equals(aparencia, n"empty_appearance_default");
  }

  // `%` no nome da aparência = aparência dinâmica de mod, que o motor recompõe com os sufixos a cada reset.
  private static func ReactsToSuffix(ts: ref<TransactionSystem>, owner: ref<GameObject>, item: ItemID, sufixos: array<TweakDBID>) -> Bool {
    if !PuppetStateHandler.IsVisible(ts, owner, item) {
      return false;
    };
    if StrContains(NameToString(ts.GetItemAppearance(owner, item)), "%") {
      return true;
    };
    let doItem: array<TweakDBID> = PuppetStateHandler.AppearanceSuffixes(item);
    for sufixo in sufixos {
      if ArrayContains(doItem, sufixo) {
        return true;
      };
    };
    return false;
  }

  // O reset faz o motor recompor o nome da aparência, agora com o sufixo novo.
  private final func Refresh(ts: ref<TransactionSystem>, owner: ref<GameObject>, item: ItemID) -> Void {
    ts.ResetItemAppearance(owner, item);
  }

  private final func RefreshDependents(ts: ref<TransactionSystem>, owner: ref<GameObject>, dependentes: array<TweakDBID>, sufixos: array<TweakDBID>) -> Void {
    for slot in dependentes {
      let peca: ref<ItemObject> = ts.GetItemInSlot(owner, slot);
      if IsDefined(peca) {
        if PuppetStateHandler.ReactsToSuffix(ts, owner, peca.GetItemID(), sufixos) {
          this.Refresh(ts, owner, peca.GetItemID());
        };
      } else {
        if ts.IsSlotSpawningAnyItem(owner, slot) && !ArrayContains(this.m_pendingSlots, slot) {
          ArrayPush(this.m_pendingSlots, slot);
        };
      };
    };
  }

  private final func ProcessSlotEvent(slot: TweakDBID, item: ItemID, equipado: Bool) -> Void {
    let owner: ref<GameObject> = this.m_owner;
    if !IsDefined(owner) || owner.IsDead() {
      return;
    };
    let ts: ref<TransactionSystem> = GameInstance.GetTransactionSystem(owner.GetGame());
    if !IsDefined(ts) {
      return;
    };
    if ArrayContains(this.m_pendingSlots, slot) {
      ArrayRemove(this.m_pendingSlots, slot);
      let chegou: ref<ItemObject> = ts.GetItemInSlot(owner, slot);
      if IsDefined(chegou) && PuppetStateHandler.IsVisible(ts, owner, chegou.GetItemID()) {
        this.Refresh(ts, owner, chegou.GetItemID());
      };
    };
    if slot == t"AttachmentSlots.WeaponRight" || slot == t"AttachmentSlots.WeaponLeft" {
      let bracos: PuppetArmsState = PuppetStateHandler.ResolveArmsState(item, equipado);
      if !Equals(bracos, this.m_arms) {
        this.m_arms = bracos;
        this.RefreshDependents(ts, owner, PuppetStateHandler.HandDependents(), [t"itemsFactoryAppearanceSuffix.ArmsState"]);
      };
    };
    // `hide_S1` é a tag com que o jogo esconde o pé: peça de outro slot que a tem também muda o estado dos pés.
    if ArrayContains(PuppetStateHandler.FeetSlots(), slot) || ts.MatchVisualTagByItemID(item, owner, n"hide_S1") {
      if this.UpdateFeetState(owner) {
        this.RefreshDependents(ts, owner, PuppetStateHandler.FeetDependents(), [t"itemsFactoryAppearanceSuffix.FeetState", t"itemsFactoryAppearanceSuffix.LegsState"]);
      };
    };
    // Casaco com `hide_T1part` arregaça a manga: a camisa por baixo troca entre `&Full` e `&Part`, como o jogo faz no boneco do inventário.
    if ArrayContains(PuppetStateHandler.TorsoSlots(), slot) && ts.MatchVisualTagByItemID(item, owner, n"hide_T1part") {
      this.RefreshDependents(ts, owner, PuppetStateHandler.TorsoDependents(), [t"itemsFactoryAppearanceSuffix.Partial"]);
    };
    if slot == t"AttachmentSlots.TppHead" {
      this.RefreshDependents(ts, owner, PuppetStateHandler.HeadDependents(), [t"itemsFactoryAppearanceSuffix.Camera"]);
    };
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
