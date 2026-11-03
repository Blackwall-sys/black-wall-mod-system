// -----------------------------------------------------------------------------
// DynamicEntitySpec
// -----------------------------------------------------------------------------
//
// Companion struct do `DynamicEntitySystem`, no MESMO template estrutural do irmão `StaticEntitySpec`
// (`ponte-staticentityspec.reds`, FECHADO 2026-08-15): a investigação de 2026-08-19 (offline)
// confirmou que é a MESMA receita (`extends IScriptable` direto, mesmo registry-Rust-
// por-ponteiro, mesmo `Vector4`/`Quaternion`-como-parâmetro já resolvido, mesmo idioma Count+ByIndex
// pra tags) — só com 12 campos em vez de 6:
//
//   Red::TweakDBID            recordID;       // NOVO
//   Red::RaRef<>              templatePath;   // igual StaticEntitySpec (ResRef)
//   uint64_t                  templateHash;   // NOVO
//   Red::CName                appearanceName; // igual
//   Red::Vector4               position;       // igual
//   Red::Quaternion            orientation;    // igual
//   bool                      persistState;   // NOVO
//   bool                      persistSpawn;   // NOVO
//   bool                      alwaysSpawned;  // NOVO
//   bool                      spawnInView;    // NOVO
//   Red::DynArray<Red::CName> tags;           // igual
//   bool                      active;         // NOVO
//
// `IsRecord()`/`IsTemplate()`/`PrepareForSaving()`/`RestoreAfterLoading()` (métodos C++ inline,
// não `RTTI_METHOD`) ficam FORA de escopo — mesma decisão de design do `StaticEntitySpec`
// (Create+Setters/Getters em vez de campos `native let` diretos).

public native class DynamicEntitySpec extends IScriptable {
    public static native func Create() -> ref<DynamicEntitySpec>

    public native func SetRecordID(id: TweakDBID) -> Bool
    public native func GetRecordID() -> TweakDBID

    public native func SetTemplatePath(path: ResRef) -> Bool
    public native func GetTemplatePath() -> ResRef

    public native func SetTemplateHash(hash: Uint64) -> Bool
    public native func GetTemplateHash() -> Uint64

    public native func SetAppearanceName(name: CName) -> Bool
    public native func GetAppearanceName() -> CName

    public native func SetPosition(v: Vector4) -> Bool
    public native func GetPosition() -> Vector4

    public native func SetOrientation(q: Quaternion) -> Bool
    public native func GetOrientation() -> Quaternion

    public native func SetPersistState(v: Bool) -> Bool
    public native func GetPersistState() -> Bool

    public native func SetPersistSpawn(v: Bool) -> Bool
    public native func GetPersistSpawn() -> Bool

    public native func SetAlwaysSpawned(v: Bool) -> Bool
    public native func GetAlwaysSpawned() -> Bool

    public native func SetSpawnInView(v: Bool) -> Bool
    public native func GetSpawnInView() -> Bool

    public native func SetActive(v: Bool) -> Bool
    public native func GetActive() -> Bool

    public native func AddTag(tag: CName) -> Bool
    public native func GetTagCount() -> Int32
    public native func GetTagByIndex(index: Int32) -> CName
}
