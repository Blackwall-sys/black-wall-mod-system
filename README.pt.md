[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**Runtime nativo de modding de Cyberpunk 2077 para macOS / Apple Silicon.**

100% Rust + redscript. Sem máquina virtual, sem camada de compatibilidade, sem streaming do Windows — os
mods rodam diretamente dentro do build nativo macOS do jogo.

> **Status: BETA 0.1.5** — cedo, mas real. Este README é honesto sobre o que
> funciona hoje versus o que está no roadmap. Apenas single-player.

---

## O que faz

O BWMS é um runtime nativo e um conjunto de ferramentas de dados para o build Apple Silicon
do jogo. A partir da 0.1.5:

- **Console in-game** — um console de desenvolvedor desenhado sobre o jogo pelo
  renderizador Metal do próprio BWMS (sem biblioteca de interface de terceiro). É um **console de comandos nativo**,
  com zero Lua no código (0% Lua). Comandos: `heal`, `level`, `money`, `give`, `remove`,
  `godmode`, `help`. O Rust do BWMS também entende linha de console Lua de Windows colada
  (`Game.AddMoney(N)`, `Game.AddToInventory(...)` e `callm Namespace.Método(args)`) e a
  traduz em chamadas nativas.
- **Mods `.lua`** — mod de Windows feito de `.lua` roda pela pasta `Mods/`: o Rust do próprio BWMS
  lê e executa os `.lua` de Windows, sem código de terceiro. A janela dele sai no terminal do
  BWMS. Mod que precisa de `.dll` não roda. Testado em jogo com um mod de câmera em terceira pessoa.
- **Cheats** — god mode, capacidade de carga, toggles de dano e recursos, eddies,
  atributos, perks, veículos e conveniências single-player similares (~15), expostos
  como ações nativas em redscript num painel **Configurações > Mods** próprio (UI em
  redscript do BWMS).
- **Pular boot de 3 níveis** — um seletor (Desligado / até o menu / direto pra gameplay,
  zero input) com tela de carregamento no boot e barra de progresso real; vale a partir
  do próximo boot. A instalação nova já vem em "direto pra gameplay".
- **Edição ao vivo do TweakDB** — ler e editar records no TweakDB em execução
  (dano, stats, flats) sem reempacotar archives.
- **Reflection para modders** — ler e escrever campos e chamar métodos por nome
  contra objetos vivos do jogo através do RTTI da engine.
- **Ferramentas de archive** — ler e extrair containers `.archive`. Mods visuais
  `.archive` soltos (loose) já carregam in-game.
- **Duas formas de pôr mod** — o menu Mods do próprio jogo (mod.io) e a pasta `Mods/`
  na pasta do jogo (Nexus e outros sites). O selo do canto superior direito e a aba
  Mods do console mostram os mods dos dois, cada um com luz verde, amarela ou vermelha.

Loja: **Steam, versão 2.5 do jogo, testada e funcionando.** A versão 2.3.1 deixou de
ser suportada. O instalador recusa GOG e Epic: o bwms usa endereços da build da Steam,
e noutra build o jogo fecha no boot (`BWMS_IGNORE_BUILD_CHECK=1` força a instalação,
por sua conta). Mods do Windows: mods com `.reds` que chamam funções que o jogo não
tem, com `.xl` e com `.yaml`/`.tweak` funcionam dentro do que o bwms cobre — dois mods
de Windows com telas próprias compilam e carregam, e na 2.5 as telas deles abriram em
jogo com os itens e o controller do mod (build de desenvolvimento). Os `.xl` da pasta `Mods/` do jogo entram nas tabelas de runtime no boot;
fora da `Mods/`, seções do `.archive.xl` não são aplicadas.

É software beta. Espere arestas. Sempre faça backup dos seus saves antes de usar
cheats (veja o disclaimer no rodapé).

---

## O que mudou desde a 0.1.4

Lista completa: [`docs/CHANGELOG-0.1.5-PUBLICO.md`](docs/CHANGELOG-0.1.5-PUBLICO.md).

- **Cyberpunk 2.5.** O 0.1.5 é para a versão 2.5 do jogo (Steam); a 2.3.1 deixou de ser
  suportada, e o instalador confere 22 pontos do binário. A 2.5 trouxe nativos o
  `CallbackSystem` (só eventos `Session/*` e `Resource/*`), o `ScriptableService`, o
  `GameInstance.GetCallbackSystem()` e métodos de interface (`GetParentWidget`, foco,
  quebra de linha do texto, `SetPath`): o BWMS usa o do jogo onde serve e completa onde
  falta — os eventos `Entity/*`, `Input/*` e `Component/*` e os filtros de alvo continuam
  do BWMS. No Mac o jogo 2.5 não dispara os eventos de sessão (não cria o próprio
  `CallbackSystem`, medido em jogo): o BWMS entrega `Session/Start`, `Session/Ready` e
  `Session/End`, e `BeforeSave`, `AfterSave`, `Pause` e `Resume` não disparam. A 2.5 tirou a declaração de `Log`/`LogChannel`/
  `LogWarning`/`LogError`/`Trace`; o BWMS as declara de novo para mod do Windows
  compilar. Mod de Windows com `.reds` que chamam funções que o jogo não tem continua
  compilando como antes. Mod de Windows que escreve no TweakDB funciona: o BWMS aceita as
  escritas que ele faz (`SetFlat` de lista e de texto: 1898 de 1898; `CreateRecord` de
  slot pelo nome curto: 54 de 54).
- **Pasta `Mods/`.** Jogue a pasta de um mod do Nexus como vem no zip (`archive/pc/mod/`,
  `r6/scripts/`, `r6/tweaks/` ou arquivos soltos) em `Mods/`, na pasta do jogo (ao lado do
  `Cyberpunk2077.app`). No boot seguinte, sem passo manual, o BWMS usa o `.archive` (posto
  na pasta de mods do próprio jogo, `archive/Mac/mod/BWMS-Mods/`, por clone, sem gastar
  disco, e montado pelo próprio jogo com a prioridade mais alta: vence o jogo base e o
  Phantom Liberty), o `.xl` (`*.archive.xl` e
  `*.xl`: as tabelas de runtime são geradas no boot; `localization.onscreens` e
  `factories` vão para o suporte a `.xl` próprio da 2.5, que o jogo aplica), o `.yaml`/`.tweak`
  (aplicado ao carregar o save, pelo mesmo caminho dos tweaks de mod) e o `.reds`. A
  `Mods/` vence o jogo e, no `.archive`, o mod.io; entre duas pastas da `Mods/`, no
  archive e no `.xl` vence a que vem antes em ordem de caminho (letra a letra,
  maiúscula antes de minúscula), e no tweak, a que vem depois.
- **Scripts compilados na abertura do jogo.** Os `.reds` de `r6/scripts`, da `Mods/` e os
  scripts de plugin são compilados antes de o jogo ler os scripts, só quando algo mudou.
  Medido fora do jogo, com mudança: ~0,7 s sem mod quebrado, ~1,3 s com 1, ~2 s em
  cascata (mod quebrado por outro quebrado); 3–5 ms sem mudança. Num boot real, 877 ms
  com os dois mods de Windows com telas próprias, 48 `.reds` da `Mods/`. Sem mudança, medido
  também no jogo: 7 ms (boot de 2026-10-18).
- **Mod com script quebrado sai sozinho.** Se um `.reds` de um mod da `Mods/` não
  compila, só aquele mod fica de fora; os outros mods e o BWMS entram e o jogo abre. O
  mod fica vermelho no relatório com o arquivo:linha do erro e a ação ("corrija o
  arquivo, ou tire o mod da pasta"). Nada é apagado nem movido. Provado em jogo em
  2026-10-24 (build de desenvolvimento). Erro no próprio BWMS ou em mod posto direto em
  `r6/scripts` (estilo Windows) mantém o comportamento antigo: o jogo abre com os
  scripts da última compilação que deu certo, e o erro sai no relatório com
  arquivo:linha; não precisa rodar o instalador de novo. Esse caso foi testado fora do
  jogo, não em jogo.
- **Relatório.** `Mods/BWMS-RELATORIO.txt`, regravado a cada abertura, em PT-BR e inglês:
  por mod, o que entrou (archives, `.xl`, tweaks, `.reds`), o que ficou de fora e por quê,
  archives recusados com o motivo, conflitos de recurso entre archives (quem vence e como
  inverter: renomear a pasta ou o arquivo), seções de `.xl` que o BWMS ainda não aplica e
  a seção Scripts. Cada mod tem a linha `luz / light` (verde: entrou inteiro, amarelo:
  em parte, vermelho: não funciona); a seção `[mod.io]` lista os mods do mod.io; os
  caminhos são relativos, sem o caminho da máquina. O `.lua` sai como mod de Lua que o
  BWMS roda pela pasta Mods, menos o que depende de `.dll`; a `.dll`, como plugin de
  Windows que não carrega no Mac.
- **Spawn de widget no build do usuário.** O override de controller no spawn de widget
  (`Item:Controller`, usado pelos dois mods de Windows com telas próprias), com a injeção da
  dependência da biblioteca externa pelo RTTI, e o evento `InkWidget/Spawn` com o filtro
  `inkWidgetTarget` agora estão no build do usuário. O override e a injeção valem nos
  spawns síncronos e nos assíncronos; o evento `InkWidget/Spawn` sai só do síncrono.
- **Menos custo.** O que o BWMS soma a cada chamada de script do jogo caiu de ~33 ns
  (0.1.4) para ~8 ns, 4× menos — mesmo teste nas duas fontes, medido fora do jogo.
  O dylib público foi de 2,76 MB para 1,99 MB (−28%). O boot não teve ganho
  mensurável, e isto não é medida de FPS.
- **Zero código de terceiro no produto.** O dylib e o `bwms-helper` não linkam
  nenhum crate de terceiro. O dylib da 0.1.4 levava uma ponte de terceiro com o
  Metal, uma biblioteca de interface em C++ e um leitor de PNG; a ponte com o Metal
  agora é do BWMS, e a imagem do splash é lida pelo framework de imagem do macOS.
- **Arquivos de script com nome do BWMS.** Os `.reds` que levavam o nome de outro
  framework agora são `bwms-*` / `ponte-*`. Na atualização a partir da 0.1.4, o
  instalador apaga os nomes antigos por uma lista explícita; arquivos de outros mods
  na pasta ficam.
- **Duas formas de pôr mod; a biblioteca do BWMS saiu.** Mod entra pelo menu Mods do
  próprio jogo (mod.io; conta e internet para instalar) ou pela pasta `Mods/` (Nexus e
  outros sites). Saíram `BWMS/mods/<categoria>/`, `nexus/mods/`, o comando `bwms mods`
  que instalava e o liga/desliga da aba Mods. Quem atualiza da 0.1.2–0.1.4: o
  instalador leva cada mod LIGADO da biblioteca para `Mods/<nome>` por clone APFS (a
  origem fica). Nome que já existe na `Mods/` não é sobrescrito (fica na origem, e o
  relatório manda comparar); mod desligado fica em `BWMS/mods` e aparece no relatório;
  os arquivos que a biblioteca pôs no jogo (archives com prefixo do BWMS em
  `archive/Mac/content/`, tweaks, scripts) saem pela lista que ela gravou. Nada do
  usuário é apagado: o relatório sugere mandar a pasta `BWMS/` para a Lixeira.
- **Selo e aba Mods mostram os 2 caminhos.** O selo do canto superior direito e a aba
  Mods do console mostram cada mod da `Mods/` e do mod.io com luz verde (entrou
  inteiro), amarela (em parte) ou vermelha (não funciona), e a contagem. Conflitos
  entre `Mods/` e mod.io (e mod.io × mod.io) mostram quem vence: no `.archive` vence a
  `Mods/` (o jogo monta o mod.io depois, no mesmo grupo). O BWMS só lê o mod.io: nunca
  escreve na pasta dele nem no `EnabledMods.json`. Sem caixa de marcar. Antes, o selo
  mostrava "0" cinza com mods na `Mods/`.
- **`factories` aplicado pelo jogo.** O suporte a `.xl` próprio da 2.5 aplica a seção
  `factories` dos `.xl` da `Mods/`; o BWMS não tem mais gancho nem tabela para isso. Se o
  jogo não puder aplicar (ex.: o `.csv` não está em nenhum archive), o mod fica amarelo
  com o motivo.
- **Travamento antes do "pressione qualquer tecla" consertado.** O gancho do BWMS na
  abertura de vídeo não repassava um argumento (as flags) à biblioteca de vídeo do jogo; dependendo do boot,
  isso prendia a thread de leitura do jogo. Consertado em 2026-10-24. Outros 2 ganchos
  do BWMS consertados: um não repassava um argumento do registro de atualizações do
  motor; outro, num trecho de 8 bytes, sobrescrevia o começo da função vizinha.
- **"Zerar nível de procurado" consertado.** O cheat pede ao sistema de procurado do próprio
  jogo a mesma desescalada que o jogo usa. Antes, ele só escrevia os fatos do nível
  de procurado, e o procurado ficava.
- **Leitura do cache de recursos consertada.** A leitura dos recursos já carregados
  (`BwmsDepotIsResourceLoaded`, `BwmsDepotLoadedResourceClass` e o que depende dela)
  lia um endereço que só funcionava por coincidência de memória e podia falhar com
  archives na pasta `Mods/`. Agora lê o mapa pelo loader do motor. Conferido em jogo
  com 4 archives na `Mods/`.

Provado em jogo na 2.5 (build de desenvolvimento, 2026-10-23): boot até a gameplay com o
save carregado sozinho; tweaks pela `Mods/` com um mod de teste do BWMS (13 de 13 valores
no TweakDB); `.archive` da `Mods/` montados pelo jogo (4 de 4); a aba Cheats (God Mode,
Invisibilidade, Zerar procurado); as telas dos dois mods de Windows, com os
itens e o controller do mod (4 de 4). Em 2026-10-24: mod com script quebrado saindo
sozinho, o selo e a aba Mods com os mods da `Mods/` e do mod.io, o `factories` aplicado
pelo jogo e o roteiro completo depois do conserto do travamento. Ainda sem
prova: mod real do Nexus com `.yaml`/`.tweak` pela `Mods/`, tecla entregue a widget de mod e o
build público do zip instalado na 2.5. `.xl` pela via nativa do jogo: a 2.5 traz suporte
a `.xl` próprio, que lê `.json`, e os textos de interface (`localization.onscreens`)
dos `.xl` da `Mods/` passam a ir por ele (provado com os dois mods de Windows com telas próprias),
e o `factories` também. As outras seções continuam aplicadas pelo BWMS, e os textos
também quando mod do mod.io ou outra pasta de mods do jogo traz textos junto; o
`Mods/BWMS-RELATORIO.txt` diz quem aplica cada seção.

## Mods oficiais (mod.io) da 2.5 e o BWMS

O jogo 2.5 tem mods oficiais pelo mod.io, no menu Mods do jogo. Instalar precisa de conta
CD PROJEKT RED e de internet; depois eles rodam offline. Eles continuam no menu do jogo;
os do Nexus e os manuais vão na `Mods/`. São as duas formas de pôr mod. O BWMS só lê a
pasta do mod.io (nunca escreve nela nem no `EnabledMods.json`) e mostra esses mods no
selo, na aba Mods e no relatório. O BWMS não precisa de internet nem de conta. O
jogo 2.5 marca com um ícone o save feito com mod na pasta de mods do jogo; o BWMS põe lá
os archives da `Mods/`, então o save sai marcado. No Mac é só o ícone; bloqueio de
conquista no Mac: não medido.

---

## Requisitos

- macOS em **Apple Silicon** (M1 / M2 / M3 / M4). Testado no macOS 27; o macOS 26 não
  foi testado por nós (há um relato de crash com a 0.1.4 no macOS 26.6.2).
- **Rust** (stable) instalado via [rustup](https://rustup.rs), com o
  target `aarch64-apple-darwin`.
- `codesign` (vem com o macOS). Só isso: o passo de alinhamento do Mach-O que o
  build precisa é um binário Rust pequeno, compilado junto com o resto — então
  **não é mais preciso ter `python3`**. Antes era; deixou de ser.
- Uma cópia legítima e instalada de Cyberpunk 2077 (build macOS da Steam,
  versão 2.5).

Você **não** precisa do app Xcode completo ou do Homebrew para compilar o runtime.

Adicione o build target uma vez:

```sh
rustup target add aarch64-apple-darwin
```

---

## Compilar a partir do código-fonte

Estes são os comandos exatos e reproduzíveis. O runtime e todas as ferramentas compilam
só com os crates locais incluídos neste repositório, sem dependências do crates.io —
nada além disso é necessário.

### 1. Runtime (a dylib do produto)

```sh
cd cp77-console
./build-core.sh
```

`build-core.sh` compila com `cargo` (perfil `publico`), remapeia os build paths (por
privacidade), aplica o conserto de alinhamento da tabela de símbolos (`fix-symtab-align`,
compilado de `src/bin/`), define um install-name limpo, assina o resultado ad-hoc e
imprime a conferência do alinhamento (`stroff mod8=0` = carregável). Com o toolchain
nightly e o `rust-src` instalados, ele também recompila a `std` com `panic=immediate-abort`
e a dylib sai menor; no stable a única diferença é o tamanho. Se o binário do jogo estiver
no disco (`CP77_DIR`), ele antes refaz a tabela de endereços em `dados/`; sem ele, vale a
tabela que já está neste repositório.

**Saída:** `target/libcp77_console-publico.dylib`

O crate `cp77-console` não depende de nenhum pacote do crates.io: além da biblioteca
padrão, usa só o `bwms-primitives` deste repositório, então compila offline sem setup
extra.

### 2. Ferramentas de dados (opcional)

As ferramentas de dados são um crate Rust só, o `bwms`, com um subcomando por ferramenta.
Compile com:

```sh
cargo build --release
```

executado de dentro de `bwms/`:

| Diretório          | O que faz                             |
| ------------------ | ------------------------------------- |
| `bwms`             | Front-end de linha de comando unificado; `bwms mods checar` compila os scripts do jogo sem tocar no cache e aponta cada `native func` que ninguém registra, `bwms tweak` lê / edita `tweakdb.bin`, `bwms archive` lê / extrai containers `.archive`, `bwms input` mescla definições de keybind / input |

`bwms` usa o crate local `bwms-primitives`, que está incluído
neste repositório — nenhum fetch externo é necessário para ele.

### 3. Scripts redscript (in-game)

Os fontes redscript ficam em `r6/scripts/blackwall-mods/*.reds`. Eles são
compilados pelo compilador redscript `scc` embutido **no momento da instalação** pelo
instalador, e de novo na abertura do jogo se algum script mudou — não há etapa de
compilação manual separada para os usuários finais. O instalador copia o `scc` e a
`libscc_lib.dylib` para `red4ext/`; é essa cópia que compila na abertura do jogo.

---

## Instalar (usuários finais)

Para jogadores que só querem rodar os mods (sem necessidade de desenvolvimento):

1. Baixe o zip do release e descompacte-o.
2. Rode **`INSTALAR.command`** (ou `bwms-install.sh "<game dir>"` de um
   terminal).
3. Inicie o jogo pela **Steam (Play)** — não pelo Finder.

O instalador adiciona uma entrada `LC_LOAD_DYLIB` ao binário do jogo e reassina o
`.app` ad-hoc só com as exceções `com.apple.security.cs.*` de que o dylib precisa.
Os entitlements de identidade da CDPR saem: eles são presos ao certificado da CDPR,
e numa assinatura ad-hoc o macOS recusa abrir o jogo com eles. Ele usa apenas
ferramentas base do macOS (`codesign`, `xattr`): sem senha, sem mudanças no SIP ou
Gatekeeper.

Antes de desinstalar, desligue o God Mode e a invisibilidade no jogo e salve: o
próprio jogo grava o God Mode no save, e a invisibilidade é um efeito que o jogo
marca para salvar.

Para desinstalar, rode `extras/DESINSTALAR.command` (de um terminal:
`bwms-install.sh --restore "<game dir>"`). Ele põe o binário original de volta e
reassina o jogo ad-hoc; a assinatura da CDPR só volta com o "Verificar integridade
dos arquivos do jogo" da Steam. Se a Steam já pôs o binário dela (numa atualização
ou no "Verificar integridade"), ele não copia nada por cima e só tira os scripts do
BWMS. Limpeza completa opcional: apague `red4ext/`,
`Mods/BWMS-RELATORIO.txt` e, em `r6/cache/`, `final.redscripts.bwms-bak`,
`final.redscripts.bk` e `final.redscripts.ts`.

---

## Layout do repositório

```
cp77-console/            A dylib do runtime (console in-game, pontes nativas)
bwms/                    Ferramenta de linha de comando unificada (`bwms mods checar`: checar os scripts do jogo; `bwms tweak`: ler / editar tweakdb.bin; `bwms archive`: ler / extrair containers .archive; `bwms input`: mesclar definições de keybind / input) + `bwms-helper`, o ajudante sem privilégio usado pelo instalador (`src/bin/`)
bwms-primitives/         Fonte única do que o runtime e as ferramentas calculam igual (crate-folha): hashes, endereços do jogo, parser de `.yaml`/`.tweak`, tabelas de runtime da `Mods/`, migração da biblioteca, núcleos puros
r6/scripts/blackwall-mods/   fontes redscript (compilados no momento da instalação)
                         — mesmo conjunto que o pacote publicado leva
example-plugin/          Plugin nativo de exemplo (C ABI), pra autores de mod
                         (o pacote publicado leva esta mesma fonte em `engine/example-plugin/`)
docs/                    API pro modder, changelog público
INSTALAR.command         Instalador para usuário final (ponto de entrada)
bwms-install.sh          Script instalador (terminal / scriptável)
```

---

## Licença

Licenciado de forma dupla sob uma das opções:

- Licença MIT ([LICENSE-MIT](LICENSE-MIT))
- Licença Apache, Versão 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

à sua escolha.

---

## Marca registrada / não afiliado

"Cyberpunk 2077" é uma marca registrada da CD PROJEKT S.A.; este projeto não é
afiliado nem endossado pela CD PROJEKT.

Este projeto não distribui **nenhum asset ou dado do jogo de qualquer tipo**. Você precisa ter uma cópia
legal do jogo para usá-lo.

---

## Notas

- **Apenas single-player.** Não há suporte a anti-cheat e nenhum é pretendido.
- **Faça backup dos seus saves** antes de usar cheats.
- O BWMS é **gratuito**. Doações são bem-vindas, mas nunca obrigatórias.

Criado por **Blackwall**.

Home do projeto: `https://github.com/Blackwall-sys/black-wall-mod-system`
