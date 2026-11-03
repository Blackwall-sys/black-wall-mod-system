// -----------------------------------------------------------------------------
// Codeware.World.StaticEntitySpec — item `#76` do catálogo exaustivo
// -----------------------------------------------------------------------------
//
// O spec do `StaticEntitySystem` (`#75`). No Codeware (`enablers/Codeware/src/App/World/StaticEntitySpec.hpp`) é um struct de dado PURO
// (`struct StaticEntitySpec : Red::IScriptable`), sem método, com 6 `RTTI_PROPERTY`:
//
//   Red::RaRef<>              templatePath;
//   Red::CName                appearanceName;
//   Red::Vector4               position;
//   Red::Quaternion            orientation;
//   Red::DynArray<Red::CName> tags;
//   bool                      attached;
//
// Aqui a classe é `extends IScriptable` direto, instanciável (`Create()`), com o estado por instância num registro do Rust indexado
// pelo ponteiro `this` (a mesma receita do `CallbackSystemHandler`); quem lê o spec é o `SpawnEntity` do `StaticEntitySystem`.
// `templatePath` sai como `ResRef` (o `RaRef<T>` não é declarável), `tags` por `AddTag` + `Count()`/`ByIndex(i)`, e
// `position`(Vector4)/`orientation`(Quaternion) passam como parâmetro struct de 16 bytes pela leitura genérica de parâmetro.

public native class StaticEntitySpec extends IScriptable {
    public static native func Create() -> ref<StaticEntitySpec>

    public native func SetAppearanceName(name: CName) -> Bool
    public native func GetAppearanceName() -> CName

    public native func SetTemplatePath(path: ResRef) -> Bool
    public native func GetTemplatePath() -> ResRef

    public native func SetAttached(attached: Bool) -> Bool
    public native func GetAttached() -> Bool

    public native func AddTag(tag: CName) -> Bool
    public native func GetTagCount() -> Int32
    public native func GetTagByIndex(index: Int32) -> CName

    public native func SetPosition(v: Vector4) -> Bool
    public native func GetPosition() -> Vector4

    public native func SetOrientation(q: Quaternion) -> Bool
    public native func GetOrientation() -> Quaternion
}
