// BWMS — TweakDBBatch: o que `TweakDBManager.StartBatch()` devolve — classe redscript `"TweakDBBatch"`, a instância do batch
// que termina no `Commit()`.
//
// O BWMS não tem staging — cada método aplica NA HORA (os mesmos handlers do `TweakDBManager`) e `Commit()` é no-op. O
// resultado final é o mesmo de um batch sem erro; se um método no meio falhar, os anteriores JÁ foram aplicados. A classe é forjada com pai (`register_type_instantiable_with_parent`), como o `CallbackSystemHandler`.
public native class TweakDBBatch {
  public native func SetFlat(id: TweakDBID, value: Variant) -> Bool;
  public native func CreateRecord(id: TweakDBID, type: CName) -> Bool;
  public native func CloneRecord(id: TweakDBID, base: TweakDBID) -> Bool;
  public native func UpdateRecord(id: TweakDBID) -> Bool;
  public native func RegisterEnum(id: TweakDBID) -> Bool;
  public native func RegisterName(name: CName) -> Bool;
  public native func Commit() -> Void;

  // `ReinheritFlat`: método PRÓPRIO do BWMS (reponta `flatId` (já existente
  // no clone) pro offset de `sourceId + "." + appendix`). `flatId`/`sourceId` são TweakDBID de
  // verdade (não string) — `appendix` é o nome do campo SEM o ponto (ex. "damage", não ".damage").
  public native func ReinheritFlat(flatId: TweakDBID, sourceId: TweakDBID, appendix: String) -> Bool;

  // `AddFlat`: método PRÓPRIO do BWMS. `flatId`/`donorTypeFlat` são TweakDBID PRONTOS (`TDBID.Create(...)`,
  // resolvido em compile-time pelo scc) — `donorTypeFlat` é o TweakDBID de um flat JÁ EXISTENTE
  // do MESMO tipo que `value` (empresta a vtable nativa). Cria um flat GENUINAMENTE NOVO
  // (storage próprio, não compartilhado) — falha se `flatId` já existir (use SetFlat pra isso).
  public native func AddFlat(flatId: TweakDBID, donorTypeFlat: TweakDBID, value: Variant) -> Bool;
}
