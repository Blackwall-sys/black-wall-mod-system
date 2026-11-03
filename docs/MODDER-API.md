# BWMS — API pra quem escreve mods

**O BWMS não é o Codeware/TweakXL/ArchiveXL/CET portados pro Mac — é um runtime próprio, nativo (Rust + ARM64 + Metal, zero emulação), que resolve os MESMOS problemas que essas ferramentas resolvem no Windows, do jeito que fizer mais sentido aqui.** Duas regras diferentes, dependendo de onde a API vem:

- **Vem do jogo de verdade (CD Projekt Red)** — classes/métodos vanilla (`PlayerPuppet`, `GameInstance`, etc.), o `native func`/`@wrapMethod`/`@addMethod` do redscript em cima delas: **nome bate exatamente igual ao Windows, sem exceção.** É o contrato real do jogo, não uma escolha nossa.
- **Vem de um framework de terceiros (Codeware, TweakXL, ArchiveXL, CET)** — essas são invenções de outros modders, não da CDPR. O BWMS reimplementa o que elas fazem, com **nome e forma próprios**, sem obrigação de replicar bug/decisão estranha da versão original. Se você está migrando um mod Windows que dependia de uma dessas, esta doc tem a seção "De Codeware/TweakXL pro BWMS" abaixo — curta, só o essencial pra trocar a chamada.

Jogo: Cyberpunk 2077 **2.5** (Steam, Mac). A 2.3.1 deixou de ser suportada.

## Como usar (funções utilitárias próprias do BWMS)

Nome próprio, prefixo `Bwms` (2026-08-07: renomeado do nome Codeware original — ver critério no topo desta doc). Exceção deliberada: `Print`/`ModLog` mantêm o nome original porque são usados internamente em vários `.reds` do próprio BWMS e são utilitário genérico de debug, não API distintiva de framework de terceiro.

No seu `.reds`, declare a função com o nome BWMS e chame normalmente. Não precisa de `import` nem de nenhuma configuração — o runtime do BWMS já deixa essas funções registradas antes do seu mod carregar.

```swift
// no seu mod (qualquer arquivo .reds em r6/scripts/seu-mod/)
native func BwmsBitTest32(value: Uint32, bit: Int32) -> Bool
native func BwmsFNV1a64(data: String, opt seed: Uint64) -> Uint64
native func BwmsParseInt32(str: String, opt base: Int32) -> Int32
native func BwmsUTF8StrLeft(str: String, length: Int32) -> String

public func Exemplo() -> Void {
  let ligado = BwmsBitTest32(5u, 0); // true (bit 0 de 0b101)
  let hash = BwmsFNV1a64("hello", 0xCBF29CE484222325ul); // 0xa430d84680aabd0b
  let numero = BwmsParseInt32("42", 10); // 42
  let inicio = BwmsUTF8StrLeft("café com leite", 4); // "café"
}
```

## Funções disponíveis hoje

### Bitwise em 8/16/32/64 bits
`BwmsBitTest8/16/32/64(value, bit) -> Bool` · `BwmsBitSet8/16/32/64(value, bit, state) -> UintN` · `BwmsBitShiftL8/16/32/64(value, bits) -> UintN` · `BwmsBitShiftR8/16/32/64(value, bits) -> UintN`

### Hashing
- `BwmsFNV1a64(data: String, opt seed: Uint64) -> Uint64` (seed padrão `0xCBF29CE484222325`)
- `BwmsFNV1a32(data: String, opt seed: Uint32) -> Uint32` (seed padrão `0x811C9DC5`)
- `BwmsMurmur3(data: String, opt seed: Uint32) -> Uint32` (seed padrão `0x5EEDBA5E`)

### Parsing de string pra número
`BwmsParseInt8/16/32/64(str: String, opt base: Int32) -> IntN` · `BwmsParseUint8/16/32/64(str: String, opt base: Int32) -> UintN`
Base `0` auto-detecta prefixo (`0x`=hex, `0`=octal); string com lixo sobrando (espaço, letra) retorna `0`.

### String UTF-8-aware
- `BwmsUTF8StrLen(str: String) -> Int32` — conta caracteres (não bytes; acentos/emoji contam como 1)
- `BwmsUTF8StrLeft/Right(str: String, length: Int32) -> String`
- `BwmsUTF8StrMid(str: String, offset: Int32, length: Int32) -> String`
- `BwmsUTF8StrLower/Upper(str: String) -> String`
- Texto de retorno de qualquer tamanho (até a 0.1.4 era cortado em 19 bytes).

### Conversão hash↔tipo
`BwmsHashToName(value: Uint64) -> CName` · `BwmsNameToHash(value: CName) -> Uint64` · `BwmsHashToCRUID(value: Uint64) -> CRUID` · `BwmsCRUIDToHash(value: CRUID) -> Uint64` · `BwmsHashToNodeRef(value: Uint64) -> NodeRef` · `BwmsNodeRefToHash(value: NodeRef) -> Uint64`

### Logging (nome original mantido, ver nota acima)
`Print(text: String) -> Void` · `ModLog(mod: CName, text: String) -> Void`
Escrevem em `r6/logs/bwms-mods.log` quando esse arquivo existe (crie um vazio pra ligar; sem ele nada é gravado). Teto de 8 MB e 200 linhas por segundo.
`Log`, `LogChannel`, `LogWarning`, `LogError` e `Trace`: a 2.5 tirou a declaração deles do jogo; o BWMS as declara de novo para mod do Windows compilar. No jogo de varejo elas não escrevem em lugar nenhum, como antes. Para log que você lê, use `ModLog`.

### Argumentos de lançamento
`BwmsGetLaunchArg(name: String) -> String` · `BwmsHasLaunchArg(name: String) -> Bool`
Lê o que veio na linha de comando do jogo. Aceita `-nome valor`, `--nome valor` e `-nome=valor`; o nome não diferencia maiúscula. Argumento sem valor existe com valor vazio. Já vêm declaradas pelo BWMS (não declare de novo no seu mod), assim como as de valor da reflexão abaixo.
```swift
let depurar: Bool = BwmsHasLaunchArg("meuModDebug");
let nivel: String = BwmsGetLaunchArg("meuModNivel");
```

### Campos do motor como `native let`
O BWMS cria antes do bind dos scripts as propriedades de campos do motor que o jogo não expõe ao script, e o seu `.reds` lê direto, sem getter. Hoje: `inkWidget.parentWidget` (o pai na árvore de UI), já declarado pelo BWMS (não declare de novo).
```swift
let pai: wref<inkWidget> = widget.parentWidget;
```
Plugins nativos criam a própria classe e as próprias propriedades pela `BwmsApi` (v17: `class_create` e `class_add_property`, chamadas no carregamento do plugin) e declaram no `.reds` deles como `public native class` e `native let`. O tipo declarado no script tem de ser o mesmo do motor (ex.: o `fontSize` do `inkText` é `Uint32`, não `Int32`): se não for, o jogo não liga os scripts, e o BWMS escreve no log qual classe reprovou.

### Screenshot
`BwmsTakeScreenshot(caminho: String) -> Bool`
O próprio motor grava um PNG do quadro em `caminho` (caminho absoluto, ex.: `/Users/<você>/Pictures/cena.png`), na mesma rotina de captura do jogo. `false` se o motor ainda não está pronto (antes do jogo carregar). Já vem declarada.
```swift
let pediu: Bool = BwmsTakeScreenshot("/tmp/meu-mod-cena.png");
```

### Estágios do frame
`BwmsFrameDeltaTime() -> Float` · `BwmsFrameTimeDilation() -> Float`
Os eventos `Pipeline/FrameBegin`, `Pipeline/PreBuckets` (antes das entidades atualizarem) e `Pipeline/PreRenderUpdate` (antes do render) chegam pelo `BwmsGetCallbackSystem()`, uma vez por frame, nessa ordem. Dentro deles, as duas natives dão o tempo do frame em segundos e a dilatação de tempo do jogo (1.0 sem câmera lenta). Já vêm declaradas.
```swift
BwmsGetCallbackSystem().RegisterCallback(n"Pipeline/PreBuckets", this, n"OnAntesDasEntidades");
// no handler:
let dt: Float = BwmsFrameDeltaTime() * BwmsFrameTimeDilation();
```

### Pasta `Mods/` e compilação na abertura
O usuário joga a pasta do seu mod como vem no zip (`archive/pc/mod/`, `r6/scripts/`, `r6/tweaks/` ou arquivos soltos) em
`<pasta do jogo>/Mods/` (ao lado do `Cyberpunk2077.app`), e no boot seguinte, sem passo manual, o BWMS usa:
- `.archive`: vai para a pasta de mods do próprio jogo (`archive/Mac/mod/BWMS-Mods/`, por clone, sem gastar disco), e é o jogo quem o
  monta, com a prioridade mais alta: vence o jogo base e o Phantom Liberty. A pasta espelha a `Mods/` a cada abertura;
- `.xl` (`*.archive.xl` e `*.xl`): as tabelas de runtime são geradas no boot. Fora da `Mods/`, `.xl` não entra (o `bwms mods` que
  instalava saiu com a biblioteca; tabela escrita à mão em `red4ext/` fica). `localization.onscreens` (textos de interface) e `factories`
  vão em `.json` para o ArchiveXL próprio da 2.5, que o jogo aplica (o BWMS gera em `archive/Mac/mod/BWMS-Mods/xl/`, espelhando a
  `Mods/`; textos provados com Equipment-EX e Virtual Atelier, `factories` provado em 2026-10-24). O BWMS não tem gancho nem tabela
  para o `factories`: se o jogo não puder aplicar (ex.: o `.csv` não está em nenhum archive), o mod fica amarelo com o motivo. As outras
  seções continuam aplicadas pelo BWMS, e os textos também quando mod do mod.io ou outra pasta de mods do jogo traz textos junto. O
  `Mods/BWMS-RELATORIO.txt` diz, por `.xl`, quem aplica cada seção;
- `.yaml`/`.tweak`: pelo mesmo caminho dos tweaks de mod, aplicados ao carregar o save. Um mod de teste do BWMS com `.yaml` e `.tweak`
  foi aplicado em jogo pela `Mods/` (13 de 13 valores no TweakDB, também na 2.5); mod real de TweakXL do Nexus ainda sem prova em jogo;
- `.reds`: compilados na abertura do jogo (abaixo).

Ordem: a `Mods/` vence o jogo e, no `.archive`, o mod.io (o jogo monta o mod.io depois); entre duas pastas da `Mods/`, no archive e no
`.xl` vence a que vem antes em ordem de caminho (letra a letra, maiúscula antes de minúscula), e no tweak, a que vem depois.

Os `.reds` de `r6/scripts`, da `Mods/` e os scripts de plugin são compilados na abertura do jogo, antes de o jogo ler os scripts, só quando
algo mudou. Medido fora do jogo, com mudança: ~0,7 s sem mod quebrado, ~1,3 s com 1, ~2 s em cascata (mod quebrado por outro
quebrado); 3–5 ms sem mudança. Num boot real, 877 ms com Virtual Atelier e Equipment-EX, 48 `.reds` da `Mods/`
(`cp77-symbols/notes/proofs/20261018-mods-reais-dev.log`). Sem mudança, medido também no jogo: 7 ms (boot de 2026-10-18).

Mod da `Mods/` com `.reds` que não compila sai sozinho: só aquele mod fica de fora; os outros mods e o BWMS entram e o jogo abre. O mod
fica vermelho no relatório com o arquivo:linha do erro e a ação ("corrija o arquivo, ou tire o mod da pasta"). Nada é apagado nem movido.
Provado em jogo em 2026-10-24 (build de desenvolvimento). Erro no próprio BWMS ou em mod posto direto em `r6/scripts` (estilo Windows):
o jogo abre com os scripts da última compilação que deu certo e o erro sai no relatório com arquivo:linha; não precisa rodar o instalador
de novo. Esse caso foi testado fora do jogo, não em jogo.

O relatório é `Mods/BWMS-RELATORIO.txt`, regravado a cada abertura, em PT-BR e inglês: por mod, o que entrou (archives, `.xl`, tweaks,
`.reds`), o que ficou de fora e por quê, archives recusados com o motivo, conflitos de recurso entre archives (quem vence e como inverter:
renomear a pasta ou o arquivo), seções de `.xl` que o BWMS ainda não aplica e a seção Scripts. Cada mod tem a linha `luz / light` (verde,
amarelo ou vermelho); a seção `[mod.io]` lista os mods do mod.io; os caminhos são relativos. `.lua` sai como "mod de CET: o BWMS roda
pela pasta Mods; o que depende de .dll não"; `.dll`, como "plugin de Windows, não carrega no Mac".

### Aplicar o `.reds` no jogo aberto, sem reiniciar
Crie o arquivo vazio `red4ext/bwms-recarga-de-scripts` na pasta do jogo. Com o jogo aberto, salvar
qualquer `.reds` de `r6/scripts` faz o BWMS recompilar o bundle em segundos e **aplicar a mudança no
jogo que está rodando**: ele compara o bundle novo com o que o motor carregou, traduz o bytecode das
funções cujo corpo mudou e troca só o corpo delas. O resultado (ou o motivo de não ter aplicado) sai
em `r6/logs/bwms-mods.log`. Apague o arquivo pra desligar.

O que aplica a quente: **mudança no corpo** de função ou método que já existe. O que pede reiniciar:
criar ou remover função, classe, campo, local novo ou literal de texto novo — nesse caso o BWMS
recusa a aplicação inteira (em vez de aplicar metade) e avisa no log. A razão é a mesma que torna a
recarga completa do motor inútil no varejo: criar objeto de RTTI invalida os ponteiros que o código
nativo do jogo já guardou, e o jogo cai em menos de um segundo. Trocar só o corpo não mexe em objeto
nenhum, então nada aponta pra lugar velho.

### Entidade de mod que sobrevive a salvar e carregar
`DynamicEntitySpec.SetPersistSpawn(true)` passa a valer. O BWMS guarda o spec da entidade (record,
template, aparência, posição, orientação e tags) num arquivo **dele**, um por save, em
`r6/storage/bwms/entidades/<save>.toml`, e recria a entidade quando aquele save é carregado.

```swift
let spec: ref<DynamicEntitySpec> = DynamicEntitySpec.Create();
spec.SetRecordID(t"Character.afterlife_merc_generic_medium_range_friendly");
spec.SetPosition(pos);
spec.SetPersistSpawn(true);          // <- o spec vai pro arquivo do save
spec.AddTag(n"meu_mod");
let id: EntityID = BwmsGetDynamicEntitySystem().CreateEntity(spec);
```

O BWMS não grava dado próprio no arquivo do save, que é a razão de não replicar o
`DynamicEntitySystemPS` do Windows (que serializa as entidades dentro do save). O que o próprio jogo
grava fica no save: God Mode e invisibilidade ligados na hora de salvar continuam nele depois de
desinstalar o BWMS. `DeleteEntity` também esquece o spec, então a entidade não volta no próximo carregamento.

Três funções pra quem quiser conferir ou forçar:

```swift
native func BwmsEntidadesPersistentes() -> Int32;              // quantas a sessão tem agora
native func BwmsGravarEntidadesDoSave(save: String) -> Bool;   // grava o arquivo de um save na hora
native func BwmsRecriarEntidades() -> Int32;                   // recria as do save corrente
```

### Janela do plugin nativo (BwmsApi)
Plugin nativo desenha a própria janela sobre o jogo pela `BwmsApi`, sem linkar biblioteca de interface. Registre um callback com `register_draw_callback` (só dentro do `bwms_plugin_main`); com o terminal aberto, o BWMS o chama a cada quadro, e nele o plugin chama `janela_abrir(titulo)`, `janela_texto(texto)` e `janela_fechar()`. A janela nasce em (60, 60), na frente, e o tamanho segue o texto. `"Nome##id"` mostra só `Nome`; `"Nome###id"` é sempre a mesma janela, com qualquer texto antes do `###`. `\n` quebra a linha. `janela_abrir` devolve `false` com a janela recolhida; fora do callback as três não fazem nada. Os slots são os mesmos desde a v10: plugin compilado desde a v10 desenha sem recompilar. Exemplo completo: `example-plugin` (`example_draw_callback`).

### Aparência de componente por mod
`BwmsAparencia.AddAppearanceOverrideForComponent(entity, component, modHash, appearance)` guarda a aparência do seu mod naquele componente; chamar de novo com o mesmo `modHash` troca a sua. Com dois ou mais mods no mesmo componente da mesma entidade, `BwmsAparencia.ApplyAppearanceOverride` aplica sempre a do **menor `modHash`** (no ArchiveXL do Windows quem vence é a ordem interna de um mapa sem ordem, sem regra). Já vêm declaradas pelo BWMS (não declare de novo).

### Tags visuais por mod
`BwmsAparencia.ApplyVisualTagOverrideForMod(entity, tag, modHash)` guarda a tag do seu mod no estado do componente e grava, em cada componente **ligado** de mesh cujo nome ou prefixo (`n0_`, `l0_`…) está na tag, a máscara original com todas as tags de todos os mods: "show" soma chunks, "hide" tira, e o **hide vence** em qualquer ordem. `RemoveVisualTagOverrideForMod(entity, tag, modHash)` tira só aquela tag do seu mod; sem nenhuma tag, o componente volta à máscara original. O retorno é quantos componentes foram escritos; `GetVisualTagOverrideCount` conta também os desligados. `ApplyVisualTagOverride(entity, tag)` é o mesmo com `modHash` 0 (desfaça com `RemoveVisualTagOverrideForMod(entity, tag, 0ul)`). O ArchiveXL do Windows não tem essas funções: lá a tag vem dos `visualTags` da aparência do item equipado.

## Ainda não disponível (não declare, vai travar o bind do seu mod)
- `CreateNodeRef` (precisa do algoritmo de hash de path-de-cena do motor, ainda não mapeado)
- `CreateLocalizationString` / `ExtractLocalizationString`
- Módulos inteiros do Codeware sem equivalente BWMS ainda: `Depot`, `Device`, `Engine`, `Mesh`, `Physics`, `Player`, `Quest`, `Vehicle` (exceto o caso pontual documentado abaixo)
- O que o `Codeware.UI` tem além da ponte (a ponte cobre o que o Equipment-EX e o Virtual Atelier usam — ver abaixo)

Se seu mod declara uma dessas (ou qualquer outra `native func`/`native class` que o BWMS não registrou), o carregamento do script vai falhar — **remova a declaração** ou espere uma atualização do BWMS. Com `native func` sem registro: na pasta `Mods/`, o mod fica fora da compilação, com a linha "fora / left out" no `Mods/BWMS-RELATORIO.txt`; em `r6/scripts`, a compilação dos scripts para, e se os scripts compilados em uso (em `r6/cache`) já têm essa native, o jogo fecha ao ligar os scripts até o mod sair de `r6/scripts` (se não têm, o jogo abre com os da última compilação que deu certo).

## De Codeware/TweakXL (Windows) pro BWMS

**Spawn de widget, no build do usuário:** `Item:Controller` no nome do item do `SpawnFromLocal`/`SpawnFromExternal` funciona sem adaptar, nos spawns síncronos e nos assíncronos (`AsyncSpawnFromLocal`/`AsyncSpawnFromExternal` e a entrega do item) (o controller do item vira a classe pedida; no `SpawnFromExternal`, antes do spawn, a biblioteca externa entra na lista de dependências de quem chama). O evento `InkWidget/Spawn` com o filtro `inkWidgetTarget` dispara só nos spawns síncronos, registrado pelo `BwmsGetCallbackSystem()` (abaixo) no lugar do `GameInstance.GetCallbackSystem()`; no evento, `GetLibraryPath()` devolve o hash (`Uint64`) e `GetItemInstance()` entrega a instância dentro de um `BwmsSerializable` (sem prova em jogo). Fora da thread principal o BWMS não troca controller, não injeta a dependência nem dispara o evento: o motor também chama o `SpawnFrom*` de outras threads, e ali o spawn fica como o do jogo. Na 2.5 (build de desenvolvimento, 2026-10-23), as telas do Virtual Atelier (loja e lista) e do Equipment-EX (guarda-roupa pelo inventário e pelo menu do jogo) abriram com os itens e o controller do mod, 4 de 4 (`cp77-symbols/notes/proofs/20261023-provas-25-cinco-blocos-pass.log`).

**Sem adaptar (ponte):** `import Codeware.UI.*` com `inkCustomController`, `inkComponent`, `SimpleButton`, `HubTextInput`, `InMenuPopup` (+ `Content`/`Footer`/`PopupButton`), `UTF8StrLower`, `GameFileExists`, `GameInstance.GetResourceDepot()`, `TweakDBInterface.GetRecords`, `GenericMessageNotification.ShowInput`, `inkImage.SetNineSliceScale`/`SetTileHAlign`, `inkText.SetWrapping` e os campos do `inkScrollController` funcionam com o nome do Windows (Virtual Atelier e Equipment-EX rodam assim). O resto desta seção é o que ainda pede troca.

Se você está adaptando um mod que dependia de uma dessas chamadas, troque como abaixo. **Não são apelidos — são reimplementações próprias**, então o comportamento pode variar em detalhe fino (marcado onde relevante).

**Cyberpunk 2.5:** o jogo 2.5 trouxe nativos o `CallbackSystem` (só eventos `Session/*` e `Resource/*`), o `ScriptableService`, o `GameInstance.GetCallbackSystem()` e métodos de interface (`GetParentWidget`, foco, quebra de linha do texto, `SetPath`). O BWMS usa o do jogo onde serve e completa onde falta: os eventos `Entity/*`, `Input/*` e `Component/*` e os filtros de alvo continuam do BWMS. No Mac o jogo 2.5 não dispara os eventos de sessão (não cria o próprio `CallbackSystem`, medido em jogo): o BWMS entrega `Session/Start`, `Session/Ready` e `Session/End`, também entre o menu e o save; `BeforeSave`, `AfterSave`, `Pause` e `Resume` não disparam. Mod escrito para o Codeware continua compilando como antes.

**`GameInstance.GetCallbackSystem(game)` → `BwmsGetCallbackSystem()`**
Sem parâmetro `game` (o BWMS resolve o singleton sozinho). `RegisterCallback`/`UnregisterCallback`/`RegisterEvent`/`DispatchEventAs` funcionam igual depois disso. Testado e estável.
```swift
// Windows: let cbs = GameInstance.GetCallbackSystem(game);
let cbs = BwmsGetCallbackSystem();
cbs.RegisterCallback(n"MeuEvento", this, n"OnMeuEvento");
```

**`GameInstance.GetScriptableServiceContainer(game)` → `BwmsGetScriptableServiceContainer()`**
Mesma ideia, sem `game`. `GetService(name)` devolve a instância do serviço daquela classe. Ciclo de vida igual ao do Codeware: cada subclasse concreta
de `ScriptableService` ganha uma instância quando os scripts carregam, com `OnLoad`, e recebe `OnInitialize` uma vez, quando o motor inicializa o game
instance (antes de carregar um save). `OnUninitialize` na saída e o estado gravado entre sessões ainda não existem.
```swift
// Windows: let svc = GameInstance.GetScriptableServiceContainer(game).GetService(n"MeuServico");
let svc = BwmsGetScriptableServiceContainer().GetService(n"MeuServico");
```

**`Reflection.GetClass(name)` → `BwmsReflGetClass(name)`; `.GetProperty(name)`/`.GetPropertyByIndex(i)` → `BwmsReflClassGetProperty`/`BwmsReflClassGetPropertyByIndex`**
Campo de qualquer tipo, lido e gravado pelo próprio tipo do campo (a mesma cópia, comparação e texto que o jogo usa no `=`, no `==` e no `ToString`): `BwmsReflPropGetValue(obj, prop) -> Variant` (desembrulhe com `FromVariant<T>`), `BwmsReflPropSetValue(obj, prop, value: Variant) -> Bool` (recusa tipo que não cabe; handle de classe filha cabe em campo da mãe), o par `BwmsReflPropGetValueString`/`SetValueString`, `BwmsReflPropToString(obj, prop) -> String`, `BwmsReflPropIsEqual(a, b, prop) -> Bool`, e pra qualquer valor `BwmsValueToString(v: Variant) -> String` e `BwmsValueIsEqual(a: Variant, b: Variant) -> Bool`. Os atalhos tipados (`Float`, `Bool`, `Int32`, `CName`) continuam. Funciona em campo de classe de script (o valor mora fora do objeto, e o BWMS acha). `ReflectionType`/`ReflectionEnum`/`ReflectionBitfield` (navegação de tipo completa) ainda não existem.
```swift
let cls = BwmsReflGetClass(n"MinhaClasse");
let prop = BwmsReflClassGetProperty(cls, n"itens");
let v: Variant = BwmsReflPropGetValue(objetoAlvo, prop);
let itens: array<TweakDBID> = FromVariant<array<TweakDBID>>(v);
let texto: String = BwmsReflPropToString(objetoAlvo, prop);
```

**`worldINodeInstance` / `worldInstancedMeshNode` → `BwmsWorldNodeInstance` / `BwmsInstancedMeshNode` (só leitura)**
No Mac esses tipos não podem ser declarados em redscript, porque o corpus vanilla não os declara. O BWMS dá getters por id de nó e por nó de setor carregado. `GetNode()` vira `GetNodeClass()` (devolve o `CName` da classe) e o `GlobalNodeID` sai como `Uint64`. Os 3 arrays de runtime do mesh instanciado saem por tamanho e por elemento. Testado em boot (boots 85–86).
```swift
// Windows: let t = instancia.GetTransform();
let raiz: GlobalNodeRef = Cast<GlobalNodeRef>(GlobalNodeID.GetRoot());
let inst = BwmsWorldNodeInstance.FromID(BwmsEntityIDToHash(Cast<EntityID>(ResolveNodeRef(ref, raiz))));
let t = inst.GetTransform();
let no = BwmsInstancedMeshNode.FromSector("base\\worlds\\03_night_city\\_compiled\\default\\exterior_-27_3_0_0.streamingsector", 12);
let sombras = no.GetCastShadows(); // 0 Default, 1 Always, 2 Never
```

**`ResourceEvent.GetResource() -> ref<CResource>` → `ref<IScriptable>` com o recurso dentro de um `BwmsSerializable`**
No Mac, um `as` feito direto sobre um recurso cru derruba o jogo (o cast lê o tipo num campo que, num recurso, é o hash do caminho). Por isso toda API do BWMS que devolve recurso ou nó de grafo de quest entrega um `BwmsSerializable`: faça `as BwmsSerializable` e use `GetObjectClassName()`, `IsObjectA(n"...")` e `GetResourcePathHash()`. Passado de volta a uma native do BWMS, ele é aberto sozinho. Testado em boot (boots 187–188).
```swift
// Windows: let tpl = evt.GetResource() as entEntityTemplate;
let res = evt.GetResource() as BwmsSerializable;
if IsDefined(res) && res.IsObjectA(n"entEntityTemplate") { let caminho = res.GetResourcePathHash(); }
```

**`ObjectSerializer::SaveToFile`/`ReadFromFile` (C++ do Codeware) → `BwmsObjectSave(obj, path)` / `BwmsObjectLoad(path)`**
Chamável do seu `.reds`. O arquivo fica em `r6/storage/bwms/objetos/<path>` (caminho relativo, sem `..`) e guarda o nome da classe, então a leitura não pede o tipo. Nomes (`CName`) precisam existir no jogo — os escritos no script com `n"..."` existem. Testado em boot (boot 188).
```swift
let ok: Bool = BwmsObjectSave(meuObjeto, "meumod/estado.bwob");
let copia: ref<IScriptable> = BwmsObjectLoad("meumod/estado.bwob");
```

**`inkWidgetRef.Create(widget)` e as irmãs tipadas → `BwmsWidgetRef.Of(widget)`, `.Text(...)`, `.Image(...)`, …; `inkWidgetRef.Set` → `BwmsWidgetRef.Bind(ref, widget)`**
Monta uma referência pra um widget que o seu mod criou; daí toda a API vanilla de `inkWidgetRef`/`inkTextRef`/`inkCompoundRef` funciona nele. Uma função por tipo, com o nome do widget sem `ink` (`HorizontalPanel`, `UniformGrid`, `VirtualCompound`...). Não há `Grid`: o `inkGrid` não existe nesta versão do jogo. Testado em boot (boot 253).
```swift
// Windows: let r = inkTextRef.Create(meuTexto);
let r: inkTextRef = BwmsWidgetRef.Text(meuTexto);
inkTextRef.SetText(r, "Olá");
```

**`ButtonHintsManager.GetInstance().SpawnButtonHints(pai)` → `BwmsButtonHints.Spawn(controller, pai)`**
A barra de dicas do próprio jogo criada direto no seu widget, a partir de um game controller seu, sem serviço global nem gancho de boot. Se a biblioteca da barra não for dependência da biblioteca do seu controller (na raiz do HUD não é), a barra nasce de uma barra do jogo já viva na árvore desse controller; `SpawnPath()` diz qual via criou e `Spawn` devolve nulo se não houver nenhuma. `AddButtonHint` → `Add`/`AddLocalized`/`AddHold`, `GetActionKey` → `ActionKey` (devolve o nome do ícone, ex. `kb_f`); `Lock`, `Show`/`Hide` e `SetStyle(n"popup")` iguais; `Destroy()` tira a barra do pai. Testado em boot (boot 256, na raiz do HUD).
```swift
// Windows: let dicas = ButtonHintsManager.GetInstance().SpawnButtonHints(pai);
let dicas: ref<BwmsButtonHints> = BwmsButtonHints.Spawn(this, pai);
dicas.Add(n"UI_Apply", "Confirmar");
```

**`PlayerSystem.GetInventoryPuppet()`/`GetCustomizationPuppet()`/`GetPhotoPuppet()` → `BwmsPreviewPuppets.Inventory(player)`/`Customization(player)`/`Photo(player)`**
Também por `GameInstance`: `InventoryOf(game)` e as irmãs. É `wref`: não segura o boneco e volta nulo quando o jogo o solta (não necessariamente no instante em que a tela fecha). Não há `Set*Puppet` — quem anota o boneco é o BWMS, quando a tela o prepara. Testado em boot com o inventário (boot 253); espelho e modo foto usam o mesmo caminho, sem teste automatizado.
```swift
// Windows: let boneco = GameInstance.GetPlayerSystem(game).GetInventoryPuppet();
let boneco: wref<gamePuppet> = BwmsPreviewPuppets.Inventory(player);
```

**`ThemeColors.Red()` (devolvia uma cor fixa) → `ThemeColors.Apply(widget, ThemeColors.Red())`**
No BWMS a cor do tema é resolvida pelo motor, no estilo do widget, e acompanha a paleta do jogo. `ThemeColors.Red()` devolve o caminho do estilo (`n"MainColors.Red"`); pra pintar use `Apply`, pra ler a cor use `ResolveFrom(widget, cor)` com o widget já montado na árvore. Os nomes são os da paleta real (`Red`, `Blue`, `ActiveBlue`, `PanelRed`...). Testado em boot (boot 253).
```swift
// Windows: meuTexto.SetTintColor(ThemeColors.Red());
ThemeColors.Apply(meuTexto, ThemeColors.Red());
```

**`VehicleSystem.ToggleGarageVehicle`/`WardrobeSystem.ForgetItemID`** — mecanismo interno já provado (o BWMS sabe fazer isso), mas **ainda não exposto como `native func` pro seu mod chamar**. Se seu mod precisa disso hoje, ainda não dá — é próximo passo natural (só falta a "porta de entrada" redscript, o motor já funciona).

**`TweakDBManager.SetFlat`/`CreateRecord`/`CloneRecord`/`UpdateRecord`/`RegisterName`** — natives reais, já declaradas pelo BWMS. `SetFlat` grava num valor próprio do flat (o TweakDB do jogo divide valores iguais entre records, e mudar o valor dividido mudaria todos), com a largura do tipo do flat; toda mudança pode ser desfeita na recarga (`TweakXL.Reload()`). Arquivos `.yaml`/`.yml`/`.tweak` de mod passam pelo mesmo caminho, com `$base` de qualquer classe e `$type` nascendo com os valores padrão de cada tipo. `SetFlat` de lista e de texto e `CreateRecord` de slot pelo nome curto são aceitos (medido com o Equipment-EX na 2.5: 1898 de 1898 e 54 de 54).

Genealogia e nomes dos records de mod (o TweakDB guarda só o hash): `BwmsTweakIsOriginalRecord(id) -> Bool` (existe e não foi criado por mod), `BwmsTweakGetOriginalParent(id) -> TweakDBID` (o primeiro ancestral do jogo de um clone, mesmo clone de clone), `BwmsTweakGetDescendantCount(id) -> Int32` e `BwmsTweakGetDescendant(id, índice) -> TweakDBID` (os clones de um record), `BwmsTweakGetName(id) -> String` (nome de record ou flat escrito por arquivo de tweak ou registrado) e `BwmsTweakRegisterName(name: String) -> TweakDBID`. `BwmsTweakRemoveRecord(id) -> Bool` tira um record criado por mod (record do jogo é recusado). `TweakXL.Version()`/`Require(...)`/`Reload()` também são natives reais.

**Stats novos** (o bônus próprio de um cyberware ou perk de mod): um record `Stat` com `enumName` que o jogo não tem vira um stat de verdade, com a faixa do próprio record. Não precisa de nada além do tweak:
```yaml
BaseStats.MeuModResistenciaNeural:
  $type: gamedataStat_Record
  enumName: MeuModResistenciaNeural
  min: 0
  max: 100
```
No script, o valor sai pelo nome (o compilador não conhece o stat novo, então não existe `gamedataStatType.MeuModResistenciaNeural`):
```swift
let valor: Int64 = EnumValueFromName(n"gamedataStatType", n"MeuModResistenciaNeural");
let tipo: gamedataStatType = IntEnum<gamedataStatType>(Cast<Int32>(valor));
```
Modificadores de TweakDB que apontam pro record (`statType: BaseStats.MeuModResistenciaNeural`) funcionam direto. O stat entra no fim de cada lote de tweaks e depois de cada `OnApply` de `ScriptableTweak`.

## Adicionando um cheat na aba "Cheats" do BWMS

A aba Cheats (Mods → Cheats, 100% redscript, sem CET/Lua/ConfigVar) tem um **ponto de extensão único**: você adiciona 1 entrada com `label` + um objeto de **comportamento** (`BWMSCheatHandler`), sem precisar tocar em nenhum arquivo do BWMS nem entender o switch interno dos cheats nativos.

### O contrato

```swift
// bwms-settings-poc.reds já declara isso — não precisa redeclarar, só herdar:
public abstract class BWMSCheatHandler {
  public func BWMSOnToggle(pp: ref<PlayerPuppet>, game: GameInstance) -> Void {}
  public func BWMSOnQuery(pp: ref<PlayerPuppet>, game: GameInstance) -> Bool { return false; }
}
```

- `BWMSOnQuery` — devolve `true` se o efeito está LIGADO agora (decide o desenho do toggle na UI).
- `BWMSOnToggle` — roda quando o usuário clica; aplica ou remove o efeito.

**Importante:** o handler é um objeto SEM ESTADO PRÓPRIO — ele é reconstruído toda vez que a aba abre. Guarde o estado do SEU efeito num campo em `PlayerPuppet` (`@addField`), nunca numa variável do próprio handler.

### Exemplo completo, copiável (arquivo `.reds` próprio, fora do BWMS)

```swift
// seu-mod.reds — adiciona 1 cheat com efeito real, sem tocar em nenhum arquivo do BWMS.
native func Print(text: String) -> Void;  // se seu mod ainda não declarou

@addField(PlayerPuppet) let m_meuBoostDeVida: ref<gameStatModifierData>;

public class MeuCheatDeVida extends BWMSCheatHandler {
  public func BWMSOnQuery(pp: ref<PlayerPuppet>, game: GameInstance) -> Bool {
    return IsDefined(pp.m_meuBoostDeVida);
  }
  public func BWMSOnToggle(pp: ref<PlayerPuppet>, game: GameInstance) -> Void {
    let ss: ref<StatsSystem> = GameInstance.GetStatsSystem(game);
    let soid: StatsObjectID = Cast<StatsObjectID>(pp.GetEntityID());
    if IsDefined(pp.m_meuBoostDeVida) {
      ss.RemoveModifier(soid, pp.m_meuBoostDeVida);
      pp.m_meuBoostDeVida = null;
    } else {
      pp.m_meuBoostDeVida = RPGManager.CreateStatModifier(gamedataStatType.BonusHealth, gameStatModifierType.Additive, 500.0);
      ss.AddModifier(soid, pp.m_meuBoostDeVida);
    };
  }
}

// PONTO DE EXTENSÃO: @wrapMethod em BWMSCheats() só pra somar SEU cheat na lista — não
// precisa @wrapMethod em BWMSRun/BWMSIsOn nem tocar no switch interno dos cheats nativos.
@wrapMethod(SettingsMainGameController)
public func BWMSCheats() -> array<BWMSCheatDef> {
  let base: array<BWMSCheatDef> = wrappedMethod();
  ArrayPush(base, this.BWMSDefH("Meu cheat: +500 de vida", new MeuCheatDeVida()));
  return base;
}
```

Coloque esse arquivo em `r6/scripts/seu-mod/seu-mod.reds` (diretório PRÓPRIO, separado de `blackwall-mods/`) ou numa pasta da `Mods/` e abra o jogo (o BWMS compila na abertura) — seu cheat aparece na aba Cheats junto com os do BWMS, com efeito real, sem editar nada do BWMS.

`BWMSDefH(label, handler, opt kind)` — `kind` opcional: `0` (default) = toggle liga/desliga; `1` = ação de disparo único (clique executa e não fica "ligado").

### Testado

O mesmo handler (com outros nomes de classe e de campo), num diretório de scripts separado de `blackwall-mods/`, compila limpo junto com o bundle do BWMS e foi validado em boot real via o mecanismo de diagnóstico (`BWMSOnQuery`/`BWMSOnToggle` chamados pela via virtual, efeito real de `StatModifier` aplicado/removido: vida bônus 0 → 500 → 0) — `cp77-symbols/notes/proofs/2026-07-17-redscript-thirdparty-cheat-INGAME-PROVADO.log`; o clique na aba Cheats, em `2026-07-18-thirdparty-cheat-click-screenshots/`.
