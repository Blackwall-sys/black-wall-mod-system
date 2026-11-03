// bwms-aparencia.reds — funções do BWMS sobre aliases de `resource.link`, aparência dinâmica, tags visuais, override de aparência e
// deslocamento de roupa. O ArchiveXL resolve esses problemas por dentro, sem expor a script, e nenhum mod do Windows as chama: por isso
// moram numa classe do BWMS, e `ArchiveXL` fica só com a fachada da referência (`ponte-fachada-de-recursos.reds`).
//
// `RegisterLink` registra um par de `resource.link` na mesma tabela do `.xl`; `ResourcePath` é só o hash, então os aliases saem como
// `Uint64` e a lista vira `GetAliasCount` + `GetAliasHash(índice)`.
//
// Aparência dinâmica (o `DynamicAppearanceController` do ArchiveXL):
//   - `IsDynamicValue`: valor dinâmico começa com `*`. `ProcessDynamicString` troca `{attrName}` por `attrValue` (um atributo só) e
//     devolve vazio se a string não resolve; `ExpandResourcePath` é o mesmo com o atributo `material`.
//   - `ResolveName`/`ResolvePath`: resolvem os atributos pelo estado da entidade (gênero, câmera, sufixos de aparência, customização, cor
//     do cabelo). Assinatura reduzida `(entity, valor)`: sem a variante da referência e sem o item equipado no estado.
//   - `IsDynamicAppearanceRef`/`AppearanceRef*` leem a referência `nome!v1!v2&c1&c2`; `IsDynamicAppearanceName`/`AppearanceName*` leem o
//     nome `nome!variante[+parte[=valor]]...[&condição...][%contexto]` — o contexto só vale no fim da string, como no ArchiveXL. Os
//     hashes saem como `Uint64`.
//   - `AppearanceMatchesReference`: o `MatchReference` sem estado de entidade, então referência com condição nunca casa.
//   - `IsUniqueAppearanceName`: falso para `default`, `random` e `empty_appearance_default`.
//
// Tags visuais (o `OverrideTagManager`, 14 tags): `GetVisualTagOverrideCount` conta os componentes cujo nome base ou prefixo de nome
// ("n0_", "h0_", "l0_"…) está na tabela da tag, sem mudar nada. `…ForMod` guarda a contribuição por tag e `modHash` no estado do
// componente e grava em cada componente ligado de mesh a máscara original com todas as contribuições ("show" soma os chunks, "hide"
// tira, e o "hide" vence); devolve quantos escreveu. `Remove…ForMod` desfaz só aquela tag daquele mod, e sem nenhuma contribuição o
// componente volta à máscara original. `ApplyVisualTagOverride` é o `…ForMod` com `modHash` 0. `…WithRefresh` também refaz a aparência
// dos componentes escritos cuja classe tem o refresh medido.
public abstract native class BwmsAparencia {
  public static native func RegisterLink(path: String, link: String) -> Bool;
  public static native func GetAliasCount(path: String) -> Int32;
  public static native func GetAliasHash(path: String, index: Int32) -> Uint64;
  public static native func IsDynamicValue(value: String) -> Bool;
  public static native func ProcessDynamicString(input: String, attrName: String, attrValue: String) -> String;
  public static native func ResolveName(entity: wref<Entity>, name: String) -> String;
  public static native func ResolvePath(entity: wref<Entity>, pathStr: String) -> String;
  public static native func GetVisualTagOverrideCount(entity: wref<Entity>, tag: String) -> Int32;
  public static native func ApplyVisualTagOverride(entity: wref<Entity>, tag: String) -> Int32;
  public static native func ApplyVisualTagOverrideForMod(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32;
  public static native func RemoveVisualTagOverrideForMod(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32;
  public static native func ApplyVisualTagOverrideForModWithRefresh(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32;
  public static native func RemoveVisualTagOverrideForModWithRefresh(entity: wref<Entity>, tag: String, modHash: Uint64) -> Int32;
  public static native func IsUniqueAppearanceName(name: CName) -> Bool;
  public static native func IsDynamicAppearanceRef(reference: String) -> Bool;
  public static native func AppearanceRefName(reference: String) -> Uint64;
  public static native func AppearanceRefWeight(reference: String) -> Int32;
  public static native func AppearanceRefMatchesVariant(reference: String, variant: String) -> Bool;
  public static native func ExpandResourcePath(pathStr: String, materialName: String) -> String;
  public static native func IsDynamicAppearanceName(name: String) -> Bool;
  public static native func AppearanceNameBase(name: String) -> Uint64;
  public static native func AppearanceNameVariant(name: String) -> Uint64;
  public static native func AppearanceNameContext(name: String) -> Uint64;
  public static native func AppearanceMatchesReference(appearanceName: String, reference: String) -> Bool;
  public static native func GetBaseAppearanceName(name: String) -> String;

  // O estado de aparência por entidade (`EntityState` do ArchiveXL). `LinkComponentToPart`/`LinkPartToAppearanceName`/
  // `AddAppearanceOverrideForComponent` montam o estado; sem ele, `ApplyAppearanceOverride` devolve `false` e
  // `ToggleConditionalComponents` pula os componentes condicionais. `ApplyAppearanceOverride` grava a aparência do override no
  // componente e refaz a aparência; `ToggleConditionalComponents` liga e desliga os componentes condicionais da entidade pelo
  // `MatchReference` e devolve quantos componentes leu (0 com entidade inválida). Diferenças de forma: `ToggleConditionalComponents` recebe a entidade em vez da lista
  // de componentes (lê a lista da própria entidade, a mesma de `Entity.GetComponents()`), e `resourcePath` é o hash (`Uint64`), não
  // `ResRef`.
  public static native func LinkComponentToPart(entity: wref<Entity>, component: ref<IComponent>, resourcePath: Uint64) -> Bool;
  public static native func LinkPartToAppearanceName(entity: wref<Entity>, resourcePath: Uint64, appearance: String) -> Bool;
  public static native func AddAppearanceOverrideForComponent(entity: wref<Entity>, component: ref<IComponent>, modHash: Uint64, appearance: CName) -> Bool;
  public static native func ApplyAppearanceOverride(entity: wref<Entity>, component: ref<IComponent>) -> Bool;
  public static native func ToggleConditionalComponents(entity: wref<Entity>) -> Int32;

  // Deslocamento de roupa por entidade e recurso (`AddOffsetOverride`/`RemoveOffsetOverrides`/`GetOffsetOverride` do `EntityState`).
  // Só o registro do BWMS muda, nunca a memória do motor; quem lê é a montagem da roupa (`ComputePlayerGarment`), com
  // `ArchiveXL.EnableGarmentOffsets` ligado. `resourcePath` é o hash; `modHash` identifica quem registrou, pra desfazer só o dele ao desequipar.
  // BACKLOG: o hook do `ComputePlayerGarment` só é instalado pelo teste do `#22` (selftest); fora dele os deslocamentos não chegam à roupa.
  public static native func AddOffsetOverride(entity: wref<Entity>, modHash: Uint64, resourcePath: Uint64, offset: Int32) -> Bool;
  public static native func RemoveOffsetOverrides(entity: wref<Entity>, modHash: Uint64) -> Uint32;
  public static native func GetOffsetOverride(entity: wref<Entity>, resourcePath: Uint64) -> Int32;
}
