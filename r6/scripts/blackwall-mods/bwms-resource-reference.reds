// bwms-resource-reference.reds — leitura de `ResourceReference<T>`/`ResourceAsyncReference<T>` numa property de um objeto, por RTTI (resolve_prop_in_class + type_kind +
// field_ptr), sem endereço nativo, e a carga pelo loader do motor.
//
// `BwmsGetResourceReferencePath`: dado um objeto e o NOME de uma property do tipo
// `ResourceReference<T>`/`ResourceAsyncReference<T>`, devolve o hash FNV1a64 do path apontado — 0
// se a property não existir/não for do tipo certo/o valueHolder ainda não foi inicializado.
native func BwmsGetResourceReferencePath(owner: ref<IScriptable>, propName: CName) -> Uint64

// `BwmsIsResourceReferenceLoaded`/`BwmsGetResourceReferenceResource`: só pra property `ResourceReference<T>`, que tem token
// (`ResourceAsyncReference<T>` não tem: devolvem `false`/`null`). `IsLoaded` = `token.finished && !token.error` (campos de dado,
// `ResourceLoader.hpp`); `GetReferenceResource` = `token.resource`, o `Handle<T>` reconstruído pela rotina de handle do
// próprio motor (`rtti::make_handle`).
native func BwmsIsResourceReferenceLoaded(owner: ref<IScriptable>, propName: CName) -> Bool
native func BwmsGetResourceReferenceResource(owner: ref<IScriptable>, propName: CName) -> ref<IScriptable>

// `BwmsLoadResourceReference` — o `LoadReferenceResource`. Grava `path` (hash FNV-1a64 do ResRef) na property
// `ResourceReference<T>` e põe no token o que o loader do motor devolve, esperando o job terminar
// numa fila. Devolve false se a property não existir ou não for `ResourceReference`.
// A espera é pelo job, não por tempo, e o token anterior do campo não é solto.
native func BwmsLoadResourceReference(owner: ref<IScriptable>, propName: CName, path: Uint64) -> Bool
