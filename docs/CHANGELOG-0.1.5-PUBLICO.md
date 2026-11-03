# bwms 0.1.5 BETA — o que mudou

Versão anterior: 0.1.4 BETA.

## Mods de CET

- Mod de CET feito de `.lua` roda pela pasta `Mods/`: o BWMS lê e executa os `.lua` de Windows em Rust próprio, sem nenhum código de
  terceiro. Testado em jogo com o JB Third Person (terceira pessoa pelo atalho, janela do mod no terminal). A API segue o CET 1.38.1.
- Mod que precisa de `.dll` não roda. Função de janela sem equivalente no terminal do BWMS responde vazio e avisa uma vez. `GameOptions`
  responde vazio. Ícones do `IconGlyphs` saem em branco.
- Atalho de mod de CET: `tecla_<id> = "<tecla>"` em `red4ext/bwms.toml`.

## Atualização para o Cyberpunk 2.5

- O 0.1.5 é para o Cyberpunk 2077 **2.5** (Edgerunners 2 Update, Steam, Mac; versão 2.5.0, build 5814786). A 2.3.1 deixa de ser
  suportada. A GOG 2.5 é suportada. O instalador confere 22 pontos do binário e recusa outra build. Epic e Mac App Store: não suportadas.
- Depois de cada atualização do jogo pela Steam, o binário volta sem o BWMS: rode o instalador de novo.
- O que o BWMS passou a usar do jogo: a 2.5 trouxe nativos o `CallbackSystem` (só eventos `Session/*` e `Resource/*`), o
  `ScriptableService`, o `GameInstance.GetCallbackSystem()` e métodos de interface (`GetParentWidget`, foco, quebra de linha do texto,
  `SetPath`). O BWMS usa o do jogo onde serve.
- O que o BWMS completa: os eventos `Entity/*`, `Input/*` e `Component/*` e os filtros de alvo continuam do BWMS. No Mac o jogo 2.5 não
  dispara os eventos de sessão: ele não cria o próprio `CallbackSystem` (medido em jogo). O BWMS entrega `Session/Start`,
  `Session/Ready` e `Session/End`, também entre o menu e o save; `BeforeSave`, `AfterSave`, `Pause` e `Resume` não disparam no Mac.
  Mod escrito para o Codeware continua compilando como antes.
- A 2.5 tirou do jogo a declaração de `Log`, `LogChannel`, `LogWarning`, `LogError` e `Trace`. O BWMS as declara de novo para mod do
  Windows compilar. No jogo de varejo elas não escrevem em lugar nenhum, como antes.
- O compilador de scripts empacotado lê o formato de scripts da 2.5.
- `.archive` da `Mods/`: vão para a pasta de mods do próprio jogo (`archive/Mac/mod/BWMS-Mods/`, por clone, sem gastar disco), e é o
  jogo quem os monta, com a prioridade mais alta (acima do jogo base e do Phantom Liberty), sem gancho. A pasta espelha a `Mods/` a cada
  abertura: o que sai da `Mods/` sai de lá. O desinstalador a remove.
- `.xl` da `Mods/`: a 2.5 traz uma versão própria do ArchiveXL, que lê instruções em `.json` na pasta de mods do jogo. Duas seções vão
  por ela: `localization.onscreens` (textos de interface) e `factories`. O BWMS gera o `.json` em `archive/Mac/mod/BWMS-Mods/xl/`
  (espelha a `Mods/`, sai quando o `.xl` sai), o jogo aplica e o BWMS não aplica de novo. Para o `factories`, o BWMS não tem mais gancho
  nem tabela: se o jogo não puder aplicar (por exemplo, o `.csv` não está em nenhum archive), o mod fica amarelo no relatório com o
  motivo. As outras seções continuam aplicadas pelo BWMS: o jogo não tem (`animations`, `bodyTypes`, `attachments`, `quest.phases`) ou
  aplica com regra diferente (`journal` e as demais, sem semântica igual provada). Com textos de mod do mod.io ou de outra pasta de mods
  do jogo junto, os textos voltam todos ao BWMS, para a ordem entre mods não mudar. O `Mods/BWMS-RELATORIO.txt` diz, por `.xl`, quem
  aplica cada seção (jogo ou BWMS).
- Equipment-EX passou a funcionar: o BWMS aceita as escritas que ele faz no TweakDB (`SetFlat` de lista e de texto: 1898 de 1898;
  `CreateRecord` de slot pelo nome curto: 54 de 54; `parentSlot`/`dependencySlots` criados como no TweakXL: 21 de 21 e 115 de 115), e o
  `OnApply` dele roda antes de o save carregar.

### Mods oficiais (mod.io) da 2.5 e o BWMS

- O jogo 2.5 tem mods oficiais pelo mod.io, no menu Mods do jogo. Para instalar, precisa de conta CD PROJEKT RED e de internet; depois
  roda offline.
- O BWMS convive com eles: os mods do mod.io continuam no menu do jogo; os do Nexus e os manuais vão na `Mods/`. O BWMS não precisa de
  internet nem de conta.
- O BWMS só lê a pasta do mod.io: nunca escreve nela nem no `EnabledMods.json`. Os mods do mod.io aparecem no selo, na aba Mods do
  terminal e na seção `[mod.io]` do `Mods/BWMS-RELATORIO.txt`, com a luz de cada um. No `.archive`, a `Mods/` vence o mod.io: o jogo monta
  o mod.io depois, no mesmo grupo.
- Save com mod: o jogo 2.5 marca com um ícone o save feito com mod na pasta de mods do jogo. Com o BWMS pondo lá os archives da `Mods/`,
  o save sai marcado. No Mac é só o ícone; bloqueio de conquista no Mac: [não medido].

## Resumo

- A 0.1.4 levava **85** arquivos de script; a 0.1.5 leva **134**. Entraram 59 e saíram 10, que eram de prova, sonda ou laboratório,
  sem função para quem instala; o instalador os apaga na atualização.
- Todo arquivo de script tem nome do BWMS (`bwms-*`, `ponte-*` e o `blackwall-mods.reds`): nenhum leva mais o nome de outro framework.
  Na atualização, o instalador apaga também os nomes antigos da 0.1.4, só os do BWMS, pela lista.
- Funções nativas globais declaradas para os scripts: **233 → 371**.
- O que o BWMS soma a cada chamada de script do jogo caiu de **~33 ns** para **~8 ns** (4× menos), medido fora do jogo com o mesmo teste
  nas duas versões. Nas ~65 mil chamadas por segundo medidas em jogo, isso é ~2,1 ms → ~0,5 ms por segundo. Não é medida de FPS, e o
  boot não ficou mais rápido: o tempo até a gameplay não mudou de forma mensurável.
- O dylib do usuário caiu de **2,76 MB** para **1,99 MB** (1.997.200 B, −28%, build de 2026-10-24). O selftest, o log de
  desenvolvimento e os marcadores de teste saíram dele: no boot do build público instalado pelo zip final, com 5 mods na `Mods/`, o log
  de desenvolvimento em `/tmp` não cresceu (0 B; `proofs/20261018-zip-015-final-usuario-014-para-015-com-mods.log`). Os comandos de engenharia reversa que só escreviam no log de desenvolvimento saíram também; ficam os que mexem no
  jogo (TweakDB, reflexão, links de recurso, LUT), que o `ajuda re` conta sem listar.
- O terminal passa a ser desenhado pelo próprio BWMS: a biblioteca de interface em C++ de terceiro (ImGui) saiu do dylib. A aparência
  é a mesma.
- Zero código de terceiro dentro do mod: o dylib e o `bwms-helper` não levam nenhuma biblioteca de terceiro. O dylib da 0.1.4 levava a
  ponte de terceiro com o Metal, a interface em C++ e o leitor de PNG, cada um com a sua cadeia. A ponte com o Metal (splash, selo,
  terminal, filtro de cor) é do BWMS, e a imagem do splash é lida pelo leitor de imagem do macOS.
- Só 2 formas de pôr mod: o menu Mods do próprio jogo (mod.io; conta e internet para instalar) e a pasta `<jogo>/Mods/` (Nexus e outros
  sites). A biblioteca do BWMS saiu: `BWMS/mods/<categoria>/`, `nexus/mods/`, o comando `bwms mods` que instalava e o liga/desliga da
  aba Mods. Na atualização a partir da 0.1.2–0.1.4, o instalador leva para a `Mods/` cada mod que estava ligado (ver "Duas formas de
  pôr mod").
- Mod da `Mods/` com script quebrado sai sozinho: só aquele mod fica de fora, os outros mods e o BWMS entram e o jogo abre.
- O selo do canto superior direito e a aba Mods do terminal mostram os mods da `Mods/` e do mod.io, com luz verde, amarela ou vermelha,
  e a contagem.
- Consertado o travamento intermitente antes da tela "pressione qualquer tecla" (ver "Motor e estabilidade").
- "Zerar nível de procurado" passa a pedir a desescalada ao próprio sistema de procurado do jogo. Antes ele só escrevia os fatos do nível, e o
  procurado não zerava.
- A pasta do mod vai para `<pasta do jogo>/Mods/` como vem no zip do Nexus, e na abertura seguinte o BWMS usa, sem passo manual e sem
  você copiar nada: `.archive` (montado pelo próprio jogo, acima do jogo base e do Phantom Liberty), `.xl`, tweaks (`.yaml`/`.tweak`)
  e `.reds`. Os `.reds` são compilados na abertura do jogo quando algo mudou, inclusive mod posto direto em `r6/scripts`, sem rodar o instalador de novo (esse caso foi
  testado fora do jogo, não em jogo). O que entrou e o que ficou de fora sai em `Mods/BWMS-RELATORIO.txt`.
- Mods do Windows que usam os nomes do Codeware cobertos pela ponte passam a compilar no Mac. Virtual Atelier e Equipment-EX compilam e
  carregam sem adaptação, e o jogo segue em gameplay com eles (build de desenvolvimento). O override de controller no spawn de widget,
  que os dois usam, e o evento `InkWidget/Spawn` passam ao build do usuário. Na 2.5, as telas do Virtual Atelier (loja e lista) e do
  Equipment-EX (guarda-roupa) abriram em jogo com os itens e o controller do mod (build de desenvolvimento).
- Mod que estende `ScriptableTweak` (como o Equipment-EX) não derruba mais o jogo.
- A configuração de boot fica num arquivo só: `red4ext/bwms.toml`.

## O que o jogador ganha

### Duas formas de pôr mod

- Mod entra no jogo de 2 formas: pelo menu Mods do próprio jogo (mod.io; conta e internet para instalar) ou pela pasta `<jogo>/Mods/`
  (Nexus e outros sites).
- A biblioteca do BWMS saiu: `BWMS/mods/<categoria>/`, `nexus/mods/`, o comando `bwms mods` que instalava e o liga/desliga da aba Mods.
- Quem atualiza da 0.1.2–0.1.4: o instalador leva cada mod LIGADO da biblioteca para `Mods/<nome>` por clone APFS. A origem fica.
  - Nome que já existe na `Mods/` não é sobrescrito: o mod fica na origem, e o relatório manda comparar as duas pastas.
  - Mod desligado fica em `BWMS/mods` e aparece no relatório.
  - Os arquivos que a biblioteca pôs no jogo (archives com prefixo do BWMS em `archive/Mac/content/`, tweaks e scripts) saem pela lista
    que ela gravou.
  - Nada do usuário é apagado. O relatório sugere mandar a pasta `BWMS/` para a Lixeira.

### Pasta `Mods/`

- A pasta do mod vai para `<pasta do jogo>/Mods/` (ao lado do `Cyberpunk2077.app`) como vem no zip do Nexus: com `archive/pc/mod/`,
  `r6/scripts/`, `r6/tweaks/` ou com os arquivos soltos. Na abertura seguinte o BWMS usa, sem passo manual e sem você copiar nada para
  `archive/` nem `r6/`:
  - `.archive`: em qualquer profundidade, com ou sem a árvore `archive/pc/mod/` do Windows. Vai para a pasta de mods do próprio jogo
    (`archive/Mac/mod/BWMS-Mods/`, por clone, sem gastar disco), e o jogo o monta com a prioridade mais alta: vence o jogo base e o
    Phantom Liberty. A pasta espelha a `Mods/` a cada abertura; o desinstalador a remove.
  - `.xl` (`*.archive.xl` e `*.xl`): as tabelas de runtime são geradas na abertura. `localization.onscreens` e `factories` vão para o
    ArchiveXL próprio da 2.5, que o jogo aplica.
  - `.yaml` e `.tweak`: aplicados ao carregar o save, pelo mesmo caminho dos tweaks de mod. Um mod de teste do BWMS com `.yaml` e
    `.tweak` foi aplicado em jogo pela `Mods/`: 13 de 13 valores no TweakDB, também na 2.5. Nenhum mod real de TweakXL do Nexus foi
    testado em jogo.
  - `.reds`: compilados na abertura do jogo, antes de o jogo ler os scripts, só quando algo mudou em `r6/scripts`, na `Mods/` ou nos
    scripts de plugin. Medido fora do jogo: com mudança, ~0,7 s sem mod quebrado, ~1,3 s com 1, ~2 s em cascata (mod quebrado por outro
    quebrado); sem mudança, 3–5 ms.
    No boot, 877 ms com Virtual Atelier e Equipment-EX, 48 `.reds` da `Mods/`. Sem mudança, medido também no jogo: 7 ms (boot de
    2026-10-18).
- Mod da `Mods/` com script quebrado sai sozinho: se um `.reds` dele não compila, só aquele mod fica de fora; os outros mods e o BWMS
  entram e o jogo abre. O mod fica vermelho no relatório com o arquivo:linha do erro e a ação ("corrija o arquivo, ou tire o mod da
  pasta"). Nada é apagado nem movido. Provado em jogo em 2026-10-24 (build de desenvolvimento).
- Erro no próprio BWMS ou em mod posto direto em `r6/scripts`, como no Windows: o jogo abre com os scripts da última compilação que deu
  certo, e o erro sai no relatório com arquivo:linha. Não precisa rodar o instalador de novo. Testado fora do jogo, não em jogo.
- A `Mods/` vence o jogo e, no `.archive`, o mod.io. Entre duas pastas da `Mods/`, no archive e no `.xl` vence a que vem antes em ordem
  de caminho (letra a letra, maiúscula antes de minúscula), e no tweak, a que vem depois.
- Ficam de fora: archive com cabeçalho inválido, archive com nome repetido, arquivo ou pasta cujo nome começa com `.`, e link
  simbólico.
- `Mods/BWMS-RELATORIO.txt`, regravado a cada abertura, em PT-BR e inglês: por mod, o que entrou (archives, `.xl`, tweaks, `.reds`) e o
  que ficou de fora e por quê; archive recusado com o motivo; conflito de recurso entre archives, com quem vence e como inverter
  (renomear a pasta ou o arquivo); seção de `.xl` que o BWMS ainda não aplica, sem mandar corrigir arquivo válido; e a seção Scripts.
  Cada mod tem a linha `luz / light`: verde (entrou inteiro), amarelo (em parte) ou vermelho (não funciona). A seção `[mod.io]` lista
  os mods do mod.io com a luz de cada um, e os conflitos entre a `Mods/` e o mod.io (e entre mods do mod.io) dizem quem vence. Os
  caminhos são relativos, sem o caminho da máquina.
- `.dll`: plugin de Windows, não carrega no Mac, e sai no relatório como fora.

### Mods do Windows feitos para o Codeware

- O BWMS passa a responder pelos nomes que esses mods chamam (`Codeware.UI`, `inkComponent`, `GenericMessageNotification.ShowInput`,
  `UTF8StrLower`, `GameFileExists` e outros; lista completa na seção do modder). A implementação é do BWMS. Nenhum código do Codeware
  entra no pacote.
- Virtual Atelier (46 erros), Equipment-EX (2) e Mod Settings (17) passam a compilar sem erro.
- No build de desenvolvimento, Virtual Atelier e Equipment-EX carregaram com os archives pela `Mods/`, e o jogo seguiu 60 s em gameplay
  com eles.
- No boot de 2026-10-18 (build de desenvolvimento), os dois entraram pela `Mods/` com archive, `.xl` e `.reds` (48, compilados na
  abertura), e o jogo chegou à gameplay.
- O override de controller no spawn de widget (`Item:Controller`), que o Virtual Atelier e o Equipment-EX usam, passa ao build do
  usuário, com a injeção da dependência da biblioteca externa pelo RTTI, nos spawns síncronos e nos assíncronos (os itens da grade da
  loja do Virtual Atelier nascem pelo assíncrono). Medido na entrega do item: 9 trocas, média 1.113 µs, maior 6.367 µs, uma vez por
  spawn, não por quadro.
- Na 2.5 (build de desenvolvimento, 2026-10-23), as telas do Virtual Atelier (loja e lista) e do Equipment-EX (guarda-roupa pelo
  inventário e pelo menu do jogo) abriram com os itens e o controller do mod: 4 de 4.
- `ArchiveXL.Require`, `TweakXL.Require` e `Codeware.Require` respondem pelas versões que o BWMS cobre (1.28.1, 1.12.0 e 1.21.1).
  `Require()` passa a responder `true` às versões que o Equipment-EX 1.2.9 pede (1.18.0, 1.26.0 e 1.11.3), então o aviso de requisito
  não deve mais abrir. A tela não foi conferida em jogo.
- Equipment-EX: o BWMS aceita as escritas que ele faz no TweakDB (`SetFlat` de lista e de texto: 1898 de 1898; `CreateRecord` de
  slot pelo nome curto: 54 de 54). Ver "Honestidade sobre o estado".

### Boot

- O modo de boot fica em `red4ext/bwms.toml`:
  - `pular_intro = true` pula os logos de abertura;
  - `splash = true` mostra a tela do BWMS sobre o loading (só vale com `pular_intro = true`);
  - `continuar_automatico = true` carrega o último save sozinho.
- A instalação nova já vem com os três ligados: o jogo abre e vai até a gameplay sozinho. Sem nenhum save, para no menu.
- O seletor "Pular boot" (Configurações > Mods > Blackwall Cheats) grava nesse arquivo: Desligado, Até o menu ou Até a gameplay.
  Quem já tem um `bwms.toml` mantém a escolha dele.
- O splash fica na tela até a tarefa acabar: o menu pronto, ou o save carregado com o mundo na tela.
  - Não some mais uns 20 s antes do jogo.
  - Com o splash na tela, a janela oculta (por exemplo, você em outro Space) não pausa mais o jogo. Depois que o splash sai, o jogo
    volta a pausar com a janela oculta, como sem o BWMS.
  - Com 0 saves, ou com o save mais recente inválido, sai com o menu pronto.
  - Em versão de jogo que não bate, sai na hora e libera o vídeo de abertura.
  - Não desliga mais sozinho em Mac com 8 GB de RAM ou menos.
  - O "ã" aparece certo.
- O carregamento automático do save só roda no boot. Voltar ao menu não recarrega nada.
- Com `pular_intro = false`, voltam os logos e a tela "APERTE ESPAÇO", e o splash não aparece (mesmo com `splash = true`). O resto do
  BWMS segue igual.

### Cheats

- Desligar o God Mode tira as fontes que o próprio jogo grava no save: a do BWMS, a do botão MODS e a do terminal. Antes, quem tinha
  save antigo continuava invulnerável depois do OFF. Medido pelo `godmode off` do terminal: save com 2 fontes, 0 depois de um OFF. O
  clique no "Invencível" do menu não foi medido.
- Desligado pelo botão MODS da pausa ou pelo `godmode off`, o God Mode fica desligado também nos saves carregados depois. Antes, o
  BWMS o religava no save seguinte. Ligado por esses dois caminhos, ele segue ligado nos saves carregados depois, como o da aba Cheats
  já fazia.
- "Zerar nível de procurado" pede ao sistema de procurado do jogo a mesma desescalada que o próprio jogo usa: as estrelas piscam e zeram, e a
  perseguição acaba. Antes, o cheat só escrevia os fatos do nível de procurado, com o procurado de pé, e ele não zerava. Em missão que
  fixa um nível mínimo de procurado, o clique não muda nada.
- "Invisível" mostra o estado real, lido do jogo: depois de carregar um save com a invisibilidade ligada, a aba mostra LIGADO e um
  clique desliga. Antes mostrava DESLIGADO com o V invisível. O clique mostra o novo estado na hora: o jogo só aplica e tira o efeito
  no passo seguinte, e sem isso a aba pintava o estado velho e o 2º clique aplicava o efeito de novo (medido em jogo).
- O próprio jogo grava o God Mode no save, e a invisibilidade é um efeito que o jogo marca para salvar. Vai desinstalar? Desligue os
  dois e salve antes.

### Terminal

- O terminal é desenhado pelo próprio BWMS, sem biblioteca de terceiro, com a mesma aparência. No campo de texto, Option+←/→/⌫ anda e
  apaga por palavra, Cmd+C e Cmd+X sem seleção copiam a linha inteira, e a repetição de tecla é a do macOS.
- A saída do `ajuda` fica na tela; antes sumia em menos de meio segundo.
- `spawnsub <id>` sem coordenadas põe o NPC ao lado do V (antes ia para a origem do mundo); coordenada inválida é recusada com mensagem.
- O atalho que abre e fecha o terminal é configurável na aba Mods ("Atalho do terminal: … [Trocar]": segure uma ou mais teclas e
  solte). Fica gravado em `atalho_terminal` no `bwms.toml`. Sem a chave, valem crase, til e F1.
- Escala inteira pela resolução, para a fonte de pixel ficar nítida: 1× em 1280×800 e 1920×1080, 2× em 1920×1200 e 2560×1600, 3× em 4K.
  A aba Mods tem "Escala do terminal" com Auto/1×/2×/3× (`escala_do_terminal` no `bwms.toml`).
- A janela não encolhe abaixo de 660×460, então as abas e as opções aparecem sempre inteiras. Quando a tela muda, a janela volta
  inteira para dentro dela.
- O tema escolhido fica aceso no próprio botão.
- `ajuda` lista os comandos de jogo (`give`, `godmode`, `heal`, `level`, `summon`…) e diz quantos comandos de engenharia reversa
  existem (`ajuda re`).
- `ajuda`, `godmode`, `heal`, `hasgod`, `attrs`, `perks`, `relic`, `level` e `summon` respondem na aba Console. No build do usuário
  eles só mostravam o eco.
- Dois comandos no mesmo instante não se perdem mais: o segundo apagava o primeiro.
- Comando desconhecido responde que não existe.

### Selo e aba Mods do terminal

- O selo do canto superior direito e a aba Mods do terminal mostram os mods dos 2 caminhos: cada mod da `Mods/` e do mod.io com luz
  verde (entrou inteiro), amarela (em parte) ou vermelha (não funciona), e a contagem. Antes, o selo mostrava "0" cinza com mods na
  `Mods/`.
- Conflitos entre a `Mods/` e o mod.io (e entre mods do mod.io) aparecem com quem vence: no `.archive` vence a `Mods/`, porque o jogo
  monta o mod.io depois, no mesmo grupo.
- O BWMS só lê o mod.io: nunca escreve na pasta dele nem no `EnabledMods.json`. A aba não tem caixa de marcar: mod entra e sai pela
  pasta `Mods/` ou pelo menu Mods do jogo.

### Relatório de crash

- Durante o boot, o BWMS grava `~/.bwms-boot-energia` (baixo consumo, tomada, estado térmico, modo de boot) e o apaga no fim do load ou
  na saída limpa.
- Se o registro sobrar, o boot seguinte gera `red4ext/bwms-crash-report.md` mesmo sem `.ips`. Todo relatório traz também a linha
  `power: lowPowerMode=`.
- Medido num MacBook Air M4, 1 boot de cada: com o baixo consumo ligado, o boot levou 117 s na tomada e 118 s na bateria, o mesmo tempo
  dos boots normais. O README em inglês e em português deixou de mandar desligá-lo.

## O que o modder ganha

Os nomes abaixo são os que o seu script chama. Quando o BWMS usa o mesmo nome do Codeware, do ArchiveXL, do TweakXL ou do RED4ext,
isso está dito. Quando o nome é próprio (`Bwms*`), o equivalente do Windows vem entre parênteses.

### Antes de tudo: seções do `.archive.xl` pedem uma tabela em `red4ext/`

O runtime das seções abaixo está no dylib do zip do Nexus e na fonte do GitHub. Mas o BWMS só as aplica com uma tabela em `red4ext/`
gerada a partir dos `.xl`. Com o `.xl` na pasta `Mods/`, o próprio BWMS gera essas tabelas na abertura do jogo. Fora da `Mods/`, nada
gera: o `bwms mods install`/`apply` saiu com a biblioteca. Tabela escrita à mão em `red4ext/` fica. O instalador do Nexus não lê `.xl`.
`localization.onscreens` e `factories` dos `.xl` da `Mods/` vão para o ArchiveXL próprio da 2.5, que o jogo aplica.

- `resource.patch`, `resource.fix`, `resource.scope` e `streaming.blocks`: patches de `.ent`, `.app`, `.mesh`, `.morphtarget`,
  `.curveset`, `.devices`, partes de roupa e `persistentStateData`; inclusão por tag `AppearanceParts`; troca de nomes de material; escopo
  valendo entre mods. Patch que não existe sai com erro no log; patch de classe errada é recusado. Diferença do ArchiveXL: o BWMS pede
  todos os patches no boot, não quando o alvo é pedido.
- `streaming.sectors`: `nodeDeletions`, `subNodeDeletions` e `nodeMutations` (posição, rotação e escala do nó e do sub-nó, `mesh`,
  `meshAppearance`). Cada grupo de mod e setor é conferido inteiro; uma linha errada cancela o mod naquele setor, como no ArchiveXL.
  Diferença: mutação só de posição, ou só de rotação, mantém a outra componente (o ArchiveXL zera).
- `journal`: entrada nova com a subárvore inteira, `*` no fim do id para editar entrada existente, textos soltos viram `LocKey#`, pins e
  POIs do mod no mapa. A mescla roda depois do fim do load, e não na inicialização do journal como no ArchiveXL.
- `quest.phases`: fases de mod entram no `.quest` na carga e no início de quest nova.
- `customizations`: grupo e opção novos, definição substituída, cor de cabelo, morph, switcher e `resource.fix` aplicados na montagem das
  opções de customização (a do criador de personagem e a do espelho), com volta exata.
- `localization`: só `onscreens` (chave nova e troca de texto existente, com feminino e masculino).
- `resource.copy`: a cópia vira recurso próprio, com os bytes do original.

### ArchiveXL

- `ArchiveXL.Version()`, `TweakXL.Version()` e `Codeware.Version()` devolvem a versão do BWMS (`0.1.5`). Para checar compatibilidade,
  use `Require(v)`, que responde por 1.28.1, 1.12.0 e 1.21.1.
- `ArchiveXL.RegisterArchive(path)` e `RegisterDir(path)` usam o mesmo carregador da pasta `Mods/`: o archive é conferido antes de
  entrar. Continuam no grupo `content`, que perde para o Phantom Liberty. Para vencer a expansão, use a pasta `Mods/`.
- Record com o flat `appearanceName` nasce com essa aparência quando spawnado, inclusive clone de personagem feito por TweakXL.
  Antes a aparência saía aleatória.
- World widgets: saiu o teto de 10. O 11º em diante deixa de ficar sem atualização.
- `BwmsAparencia.ResolveName` e `BwmsAparencia.ResolvePath` resolvem nomes com `*` pelo estado da entidade: `{gender}`, `{camera}`,
  `{body}`, `{arms}`, `{feet}`, `{sleeves}`, `{hair_type}`, `{skin_color}`, `{hair_color}`, `{eyes_color}`. Vale também para o
  `ApplyAppearanceOverride`.
- `PuppetStateSystem` (`GetArmsStateSuffix`, `GetFeetStateSuffix`) devolve o estado real de braços (MantisBlades, Monowire,
  ProjectileLauncher) e de pés (Flat, Lifted, HighHeels, FlatShoes). O handler do V é criado ao entrar no jogo. `BodyType` segue sempre
  `BaseBody`. Diferença: o estado é lido na hora, sem guardar o último item equipado.
- Override de visual tag por mod (`ApplyVisualTagOverrideForMod`, `RemoveVisualTagOverrideForMod` e as variantes `WithRefresh`):
  esconder vence mostrar, duas máscaras do mesmo mod acumulam, e tirar um mod desfaz só o efeito dele. `ApplyVisualTagOverride` (uma
  chamada, sem mod) grava sobre a máscara atual do componente pela mesma regra.

### TweakXL

- Toda gravação de mod no TweakDB (YAML, `.tweak`, `SetFlat` de script, plugin) passa por um escritor só. Mudar um record não altera
  mais outros records que dividiam o mesmo valor.
- YAML em runtime aceita Bool, record, CName, LocKey, recurso e array, além de número.
- `.tweak` passa a ser aplicado.
- `$base` e `$type` aceitam qualquer classe, inclusive pelo nome curto.
- `!append`, `!prepend` e `!remove` convertem o elemento pelo tipo do array. `!append` em escalar não muta nada.
- Clone (`$base`, `TweakDBManager.CloneRecord`) herda as mudanças feitas depois no record base, enquanto não sobrescreve o flat.
- Record criado por `$type` nasce com o valor padrão de cada flat do tipo.
- Recarga sem empilhar: `!append` aplica uma vez, linha apagada do arquivo volta ao valor do jogo, record existente re-herda do base novo.
  `TweakXL.Reload()` desfaz, reaplica e dispara o `OnApply` quando os arquivos terminam.
- O buffer de valores do TweakDB cresce até 16 MB em vez de faltar espaço.
- Stat novo: um YAML com `$type: gamedataStat_Record` e `enumName` novo cria um valor novo em `gamedataStatType`, e os getters do jogo
  leem esse stat.
- Records de mod ganham nome e genealogia: `BwmsTweakGetName`, `BwmsTweakRegisterName`, `BwmsTweakIsOriginalRecord`,
  `BwmsTweakGetOriginalParent`, `BwmsTweakGetDescendantCount`, `BwmsTweakGetDescendant` e `BwmsTweakRemoveRecord` (recusa record do
  jogo).
- `ScriptableTweak.OnApply` roda uma vez, no menu, depois de o TweakDB carregar e antes de o save começar a carregar, como no TweakXL;
  cada `OnApply` sobe a versão do TweakDB uma vez só, no fim. Medido na 2.5: o Equipment-EX aplica antes da gameplay, e o guarda-roupa
  abre com a grade cheia já na primeira vez.
- `SetFlat` de lista e de texto e `CreateRecord` de slot pelo nome curto passam a ser aceitos, e o `SetFlat` cria o flat que o record não
  tem (`TweakDBID`, `array:TweakDBID`, `array:CName`), como o TweakXL. Medido com o Equipment-EX na 2.5: 1898 de 1898, 54 de 54, e os
  `parentSlot`/`dependencySlots` dos slots de roupa 21 de 21 e 115 de 115.

### Codeware (camada equivalente do BWMS)

**Nomes do Codeware para mods do Windows**

- `import Codeware.UI.*`: `inkCustomController`, `SimpleButton`, `HubTextInput`, `InMenuPopup`, `InMenuPopupContent`,
  `InMenuPopupFooter`, `PopupButton`; e `inkComponent` global.
- `GenericMessageNotification.Show` com `inkTextParams`, e `ShowInput`: o mesmo popup com campo de texto, que devolve o texto em
  `CloseData.input`.
- `UTF8StrLower`, `GameFileExists` (`archive/pc/mod/X` é respondido pelo archive carregado), `GameInstance.GetResourceDepot`,
  `ResourceDepot.ArchiveExists`, `TweakDBInterface.GetRecords`.
- Campos do motor que o script vanilla não declara, por `native let`: scroll, nine-slice e tile do `inkImage`, wrapping do `inkText`,
  câmera da prévia do personagem, `enum textWrappingPolicy`.
- `BwmsScreenSize` ganhou `X`/`Y` em Float.
- `ModuleExists("Codeware")` continua `false` de propósito: mods com `@if` seguem no ramo que já funciona.

**Entidades**

- `DynamicEntitySystem` completo: `CreateEntity`, `DeleteEntity`, `EnableEntity`/`DisableEntity`, `GetEntity`, `GetTagged`,
  `GetTaggedIDs`, `GetTags`, `IsSpawned`, `IsSpawning`, `EnableTagged`/`DisableTagged`, `RegisterListener`, e o evento
  `DynamicEntityEvent` (Created, Spawned, Despawned, Dead, Deleted). `Dead` sai de uma checagem a cada ~1 s, porque o motor não emite.
- `DynamicEntitySpec.persistSpawn`/`persistState`: o spec fica em `r6/storage/bwms/entidades/<save>.toml`, um arquivo por save, e a
  entidade é recriada ao entrar na sessão. O BWMS não grava o spec no save.
- `StaticEntitySystem`: spawn por template, attach, detach, despawn e as versões por tag. Falha de template chega ao callback.
- `EntityBuilderEvent.GetEntityBuilder()` no `Entity/Extract`, com `EntityBuilderWrapper`, `EntityBuilderTemplateWrapper` e
  `EntityBuilderAppearanceWrapper` (`AddComponent`, `GetComponents`).
- `Entity.ApplyMorphTarget(alvo, região, valor)` e `Entity.SetWorldTransform(transform)`.
- `GameTagSystem.AssignEntityTag`/`UnassignEntityTag`: as consultas vanilla `GetAnyMatchingEntity`/`GetAllMatchingEntities` passam a
  achar a entidade. Só memória, não vai ao save.
- `BwmsPopulationSpawner` (`PopulationSpawnerWrapper`), só leitura: ativo, record, aparência, transform e os ids ativos, restaurados,
  spawnados e reservados.
- Workspot do NPC: `BwmsWorkspotHasInstance`, `BwmsWorkspotAnimName`, `BwmsWorkspotIdleAnim`, `BwmsWorkspotEntryId`,
  `BwmsWorkspotResourcePath`.

**CallbackSystem**

- Família `Entity/*` completa: `Initialize`, `Attach`, `AfterAttach`, `Detach`, `Uninitialize`, `Assemble`, `Reassemble` e `Extract`.
  Os nomes antigos (`Entity/Attached`, `Resource/Loaded`, `Resource/Ready`) são mapeados.
- `Session/Ready` dispara quando o player principal nasce.
- `Resource/Load` entrega o recurso por `ResourceEvent.GetResource()`.
- Pipeline: além de `FrameBegin`, disparam `Pipeline/PreBuckets` e `Pipeline/PreRenderUpdate` a cada quadro. `BwmsFrameDeltaTime()` e
  `BwmsFrameTimeDilation()` dão o tempo do quadro.
- Alvos novos: `StaticEntityTarget.Tags`, `EntityTarget.ID`, `EntityTarget` em `Component/Toggle` e `Vehicle/ToggleAuxLights` pelo
  dono.
- `InkWidget/Spawn`, com o filtro `inkWidgetTarget`, dispara no build do usuário. O `RegisterCallback(n"InkWidget/Spawn", ...)` já liga
  o despacho (`BwmsToggleWidgetSpawnEvent` nasce ligado). `GetItemInstance()` entrega a instância dentro de um `BwmsSerializable`. O
  evento sai só dos spawns síncronos, não dos `AsyncSpawnFrom*`. Fora da thread principal, ele não sai: o motor também chama o
  `SpawnFrom*` de outras threads, e ali o spawn fica como o do jogo.
- `ScriptableService.OnInitialize` roda na mesma instância do `OnLoad`, e `GetService` devolve essa instância.

**Recursos**

- `IComponent.LoadResource()`, `LoadResourceToken()` e `ResetMaterialCache()`. A aparência `default` resolve para a 1ª aparência do
  mesh, como no motor.
- `BwmsDepotLoadResourceToken(hash)`, `BwmsDepotRemoveResourceFromCache(hash)` e `BwmsLoadResourceReference(objeto, propriedade, hash)`
  (`ResourceDepot`, `ResourceHelper`). A carga vai pela rota do próprio motor.
- `ResourceToken.FromId`, `GetJobHandle`, `WaitForJob` (espera de verdade, teto de 2 s) e `RegisterCallback(alvo, função)` (chama uma
  vez, quando o recurso termina de carregar).
- `BwmsResourceRef` e `BwmsResourceAsyncRef` (`ResourceRef`/`ResourceAsyncRef`), com `*=`. Helpers: `ResRefGetPathHash`, `ResRefIsEmpty`,
  `ResRefIsLoaded`, `ResRefResourceClass`, `ResRefHashFromPath`.
- `BwmsSerializable` embrulha objeto do motor que não é `IScriptable` (recurso, nó de quest). Um `as` direto nesses objetos derruba o
  jogo no Mac; use `GetObjectClassName`/`IsObjectA`.
- `BwmsSoundBanks`: listar, trocar caminho e adicionar banco de som.
- `BwmsObjectSave(obj, caminho)` e `BwmsObjectLoad(caminho)` (`ObjectSerializer`): objeto em arquivo próprio em
  `r6/storage/bwms/objetos/`, fora do save.

**Interface**

- Campo de texto: `BwmsTextInput` e `BwmsHubTextInput` (`TextInput`/`HubTextInput`), com cursor, seleção, copiar/colar, Shift e
  Ctrl; `BwmsTextMeasurer`; `BwmsTextEdit`, que conta caractere e não byte ("ção" = 3). Quem monta o controller tem que guardar
  referência forte: o widget só guarda referência fraca.
- `BwmsInkKeyEvent` (tecla, press/release, caractere), entregue ao widget com foco pelo callback `OnBwmsInputKey`. `GetCharacter()`
  segue US-QWERTY com shift, não o layout do sistema.
- Widget criado por script e anexado a uma árvore na tela entra na camada: `inkWidget.BwmsAttachWidgetTree()` e
  `inkWidget.BwmsBindController(controller, secundário)` (`WidgetBuildingService`).
- Spawn de widget de biblioteca com `Item:Controller` (`SpawnFromLocal`/`SpawnFromExternal`): o controller pedido no nome troca o do
  item. No `SpawnFromExternal`, com ou sem controller no nome, a biblioteca externa entra na lista de dependências de quem chama. Vale
  no build do usuário, nos spawns síncronos e nos assíncronos (`AsyncSpawnFromLocal`/`AsyncSpawnFromExternal` e a entrega do item); o
  evento `InkWidget/Spawn` sai só do síncrono. Fora da thread principal o BWMS não troca controller, não injeta a dependência nem
  dispara o evento: o motor também chama o `SpawnFrom*` de outras threads, e ali o spawn fica como o do jogo.
- `BwmsButtonHints` (barra de dicas de botão do jogo), `BwmsWidgetRef` (`inkWidgetRef` para widget de mod), `ThemeColors.ResolveFrom`,
  `BwmsPreviewPuppets` (bonecos de prévia do inventário, do criador de personagem e do modo foto).
- `inkWidget.GetParentWidget()`, `CanSupportFocus()` e `SetSupportFocus(Bool)` voltaram. `inkWidget.parentWidget` existe como campo.
  `BwmsWidgetFind` ganhou `Parent`, `Root`, `Depth`, `IsMounted` e `FirstAncestorOfType`.
- `BwmsWidgetIsAttached`, `BwmsWidgetReparent`, `BwmsWidgetReparentAt` (`inkComponent.IsAttached`/`Reparent`). Aqui `IsAttached`
  pergunta se o widget está pendurado numa árvore.
- `BwmsPropOffset`, `BwmsPropU64` e `BwmsPropSetU64`: campo nativo pelo offset que o RTTI desta build informa (`@addField` do Codeware,
  como `inkText.letterCase`). A largura é obrigatória; valor que não cabe é recusado, sem truncar.
- `inkSystem`: `BwmsInkSystemGetLayerByName`, `BwmsInkSystemSetFocus`, `BwmsInkSystemResetFocus`, `BwmsInkSystemGetWorldWidgets`.
- `worlduiIGameController.GetWorldWidgetComponent()` (mesmo nome do Codeware) e `inkUserData.GetWorldWidgetComponent()`: o componente 3D
  por trás de uma tela de device ou placa. Nulo no HUD.

**Mundo, quests e consultas**

- `GameInstance.GetWorldStateSystem()`: ligar e desligar node e variante de prefab, ativar comunidade, fase de comunidade, reset de
  comunidade e de spawner. `CommunityWrapper` e `CommunityEntryWrapper` leem entradas, fases e ids.
- `WeatherSystem.SetWeatherState`, `ResetWeatherState`, `GetWeatherStateObject`, `GetEnvironmentDefinitionObject`.
- `BwmsWorldNodeSetup` (`WorldNodeSetupWrapper`, leitura e escrita), `BwmsWorldNodeInstance` e `BwmsInstancedMeshNode` (só leitura),
  `BwmsSectorCount`. Só vale para setor já carregado. Na 2.5 o índice do nó e as duas distâncias mudaram de lugar no setup: o BWMS lê e
  grava no lugar novo (antes, gravar `SecondaryRefPointDistance` estragava o índice do nó), e `SetNodeIndex` aceita índice até 524287.
- `inkCompoundWidget.GetChildIndex(child)` (nome do Codeware 1.21.1): a posição do filho, -1 se não for filho. `inkWidget.AttachController`
  já vem do jogo 2.5. `GarageComponent.LoadSummonSetup()`: relê a configuração de chamada de veículo do TweakDB.
- `streaming.sectors` do `.xl` em `worldInstancedMeshNode` na 2.5: `instanceDeletions`, mutação de instância, `mesh` e `meshAppearance` usam o
  formato cozido da 2.5 (antes eram recusados). `actorDeletions` em `worldCollisionNode` também lê o formato da 2.5.
- `resource.patch` de `.inkanim` (ArchiveXL 1.27.2): as sequências do patch trocam as do alvo com o mesmo nome, e as outras entram no fim.
- `actorDeletions: [[ator, shape]]` (ArchiveXL 1.27.0): só aquela shape do ator perde a colisão (preset `No Collision`); o resto do ator fica.
- TweakXL, ArchiveXL ou Codeware de terceiro no jogo (`red4ext/plugins/<nome>/` ou `r6/scripts/<nome>/`): o instalador recusa e a abertura
  do jogo não compila os scripts, com o que mover para a Lixeira no relatório da pasta `Mods/`. O BWMS já faz o que eles fazem.
- `{nails_color}` em aparência dinâmica (ArchiveXL 1.27): a cor da unha do V entra no estado dinâmico do jogo junto da pele, do cabelo e dos olhos.
- `{material.1}`..`{material.N}` em caminho dinâmico de material (ArchiveXL 1.27): partes do nome do material separadas por `+`.
- Nós cozidos da 2.5 no `streaming` (ArchiveXL 1.28): recurso, aparência e `nbNodesUnderProxyDiff` em `worldCookedStaticMeshNode`,
  `worldCookedGenericProxyMeshNode`, `worldCookedStaticDecalNode` e `worldCookedAcousticSectorNode`; deleção do `worldCookedStaticMeshNode`;
  deleção e posição/orientação de ator no `worldCookedCollisionNode`.
- Máscara de chunk com número no `hide`/`show` (`hide: 224`), além da lista (ArchiveXL 1.27.3).
- `widget.AttachController(controller)` num widget já na tela (HUD, menus): o controller do mod passa a ser registrado na camada da
  tela e a receber o `OnInitialize` (Codeware 1.21.1). `inkCompoundWidget.GetChildIndex(widget)` também entra.
- `entSkinnedMeshComponent.meshResource` e `audioDialogLineEventData.gender` (nomes do Codeware 1.21.1).
- Na 2.5 o `renderProxy` dos componentes de mesh com esqueleto mudou de lugar; as funções de aparência do BWMS que o usam leem o lugar novo.
- `BwmsQuestPhaseGraphAccessor` (`QuestPhaseGraphAccessor`): nós por classe, os 14 `Find*` e `GetAllGraphNodePaths`, só leitura.
- `QuestsSystem.BwmsExecuteNode(node, inputSocket)`: com a trava do sistema ocupada devolve -5 sem executar; objeto que não é nó, -3.
- `BwmsResetSecuritySystemNetwork` (`ResetSecuritySystemNetwork`).
- `BwmsTraceResultGetHitObject`, `BwmsTraceResultGetHitEntity` e `BwmsTraceResultGetHitObjectClass` (`TraceResult.GetHitObject`/
  `GetHitEntity`). O objeto vem como `ref<IScriptable>`. Chão e parede voltam nulo; a classe sai do 3º.
- Controller visual: `BwmsVisualControllerHasController`, `BwmsVisualControllerStatus`, `BwmsVisualControllerDependencyCount`,
  `BwmsVisualControllerLooseDependencyCount`.
- `BwmsRenderProxyIsVisible(componente)`.
- `BwmsLookAt` (o que está sob a mira, distância, NPC e facção do alvo, raio de um ponto a outro) e `BwmsNPC` (NPCs num raio, facção de
  cada um, gangue dominante, mais próximo de uma facção). Feitos só com a API do jogo.

**Reflexão e utilitários**

- `BwmsReflPropGetValue`/`SetValue` (em `Variant`), `BwmsReflPropGetValueString`/`SetValueString`, `BwmsReflPropToString`,
  `BwmsReflPropIsEqual`, `BwmsValueToString`, `BwmsValueIsEqual`: campo de qualquer tipo pelo próprio tipo do motor. Tipo errado é
  recusado.
- Reflexão de `Int32`/enum e `CName` (`BwmsReflPropGetValueInt32`, `SetValueInt32`, `GetValueCName`, `SetValueCName`) e
  `BwmsReflPropIsSet`. Na leitura de `Int32`, falha devolve -1.
- `IScriptable.BwmsClone()` (`Clone`): cópia funda; handle fraco para algo copiado passa a apontar para a cópia.
- `BwmsCurve` (`CurveData`): 5 tipos, tamanho, pontos, interpolação.
- `BwmsJobQueue` (`JobQueue`): `Handle` de um objeto de script como job no pool de threads do motor.
- `BwmsLocalization.GetOnScreenEntries(out entries)`, `BwmsLocProvider.GetKeys()` e a struct `localizationPersistenceOnScreenEntry`.

### RED4ext e plugins nativos

- `BwmsGetLaunchArg(nome)` e `BwmsHasLaunchArg(nome)`: argumentos de lançamento do jogo.
- `BwmsTakeScreenshot(caminho)`: PNG pelo próprio motor, no quadro seguinte.
- `ModLog(mod, texto)` e `Print(texto)` gravam em `<jogo>/r6/logs/bwms-mods.log` quando esse arquivo já existe ao abrir o jogo (criar
  liga e apagar desliga a partir da próxima abertura). Acima de 8 MB, o arquivo recomeça vazio na abertura seguinte. No máximo 200
  linhas/s. Plugins nativos usam o mesmo arquivo.
- Plugins carregam logo depois do registro do BWMS, no bind dos scripts: native registrada por plugin pode ser declarada num `.reds`.
- O `OnUpdate` de Running dos plugins chega em gameplay. Todo callback de estado recebe o `CGameApplication`.
- Janela de plugin nativo pela `BwmsApi` (`janela_abrir`, `janela_texto`, `janela_fechar`), desenhada pelo terminal do BWMS: plugin
  compilado desde a v10 continua desenhando sem recompilar.
- `BwmsApi` vai até `abi_version` 17. Campos novos: `launch_arg`; `prop_type_name`, `prop_to_string`, `prop_get_string`,
  `prop_set_string`, `prop_copy`, `prop_is_equal`; `tweak_is_original`, `tweak_original_parent`, `tweak_descendants`, `tweak_name`,
  `tweak_register_name`, `tweak_remove_record`; `call_global` (função global de script por nome, `h:0x…` para handle); `class_create` e
  `class_add_property` (classe e `native let` criados antes do bind); `take_screenshot`.
- `call_method`, `call_method_args` e `call_global`: com `ret16` nulo, o BWMS solta o `ref<>` devolvido. Com `ret16`, a referência fica
  com o plugin. A C-ABI ainda não tem função de soltar.

### Para toda native do BWMS

- Native que devolve String devolve o texto inteiro. Antes cortava em 19 bytes, ou voltava vazio.
- Native do BWMS usada como argumento de outra chamada (`F(BwmsX(a), b)`) não desloca mais os argumentos seguintes.
- Ainda assim, leia o retorno numa variável local antes de compará-lo ou de passá-lo a `ArraySize`: `let s = BwmsConfigGet(k); if
  Equals(s, "1")`. Com `ArraySize(F())` direto, o valor sai errado (115 no lugar de 1). A comparação inline não tem prova de que funciona.

## Correções

**ArchiveXL**

- `ArchiveXL.RegisterArchive`/`RegisterDir` passavam ao motor o nome do grupo no lugar da lista de recursos e soltavam a trava zerando o
  byte.
- `BwmsAparencia.ApplyAppearanceOverride` devolvia `false` no build do usuário mesmo aplicando.
- `BwmsAparencia.ApplyVisualTagOverrideForMod` combinava as máscaras na ordem invertida, e a 2ª máscara do mesmo mod substituía a 1ª.
- `BwmsAparencia.ApplyVisualTagOverride` gravava a máscara crua: um "show" de HighHeels escondia os outros chunks.
- `JournalManager.GetTrackedQuest()` voltava vazio com quest rastreada. Agora sobe do objetivo até a quest.
- O `PuppetStateHandler` do V nunca era criado no jogo do usuário.
- `ArchiveXL.Reload()` depois que o último mod com `resource.patch` saiu (a tabela gerada sai junto): os patches do mod
  que saiu continuavam valendo até fechar o jogo. Agora a tabela ausente vale como vazia, e as esperas de patch saem junto.
- Recarga sem a tabela de `customizations`: as opções do mod que saiu continuavam no criador e no espelho até fechar o jogo.
- Patch de `.ent` (componentes) numa montagem da 1ª leva da sessão era recusado sem aviso.

**TweakXL**

- Mod que estende `ScriptableTweak` derrubava o jogo logo depois do `OnApply`. Pela mesma causa passa a funcionar no build do usuário,
  onde estava desligado, o patch de fases de missão por mod (`quest.phases`).
- O `OnApply` de mod que estende `ScriptableTweak` podia derrubar o jogo: a classe que o BWMS cria para ele era menor que a do mod. Agora
  tem o tamanho certo, e o BWMS se recusa a chamar o `OnApply` se o tamanho não bater.
- O índice de flats do TweakDB gravava tamanho e capacidade trocados no primeiro crescimento de cada boot. Com isso, uns 250 flats do
  topo da ordem de hash voltavam ao valor padrão. Afetava mod com `$base`, `CloneRecord` ou `AddFlat` que cresce o índice.
- `SetFlat` gravava TweakDBID e CName com largura errada. `TweakDBBatch.AddFlat` grava 8 bytes.
- `!remove` do último elemento esvazia o array (antes recusava).
- `TweakDBInterface.GetRecords` sem alocador devolve array vazio.
- `TweakDBBatch.ReinheritFlat` passa pelo escritor.
- `$type` sem classe conhecida para no 1º record em vez de varrer o TweakDB inteiro.

**Codeware**

- A reflexão lia e gravava fora do campo em toda classe de script, inclusive pelos acessores por nome.
- Em componente da família skinned (o corpo do V, por exemplo), os métodos de `IComponent` liam e gravavam 8 bytes adiante:
  `GetResourcePath` devolvia o nome da aparência e `SetChunkMask` escrevia nos bytes de LOD e sombra.
- `MappinSystem.GetPoiMappinPhase` devolvia lixo e alterava o estado do mappin a cada leitura. `SetPoiMappinPhase` chamava uma função sem
  relação com fase de POI.
- `Entity.FindComponentByType` respeita herança.
- `WardrobeSystem.ForgetItemID` usa a trava no protocolo do Mac.
- `CallbackSystem` chamava método em ouvinte já destruído. Agora o registro é fraco e ouvinte morto é pulado. Vale também para o
  ouvinte desregistrado e destruído por outro ouvinte no meio do mesmo despacho.
- Filtros de alvo do `CallbackSystem`: `ResourceTarget.Path`/`Type` casavam todo recurso; `StaticEntityTarget` por tag nunca casava;
  `EntityTarget.RecordID` nunca casava com record real; a mesma função registrada 2 vezes virava um registro só.
- `ResourceTarget` com `regex:` casa o caminho inteiro, como o Codeware. `regex:(foo|bar)` não casa mais "xxbarxx". Padrão fora do
  suportado (`\d`, `\w`, classe POSIX, faixa invertida…) é recusado em vez de virar texto literal. Padrão patológico deixou de ser caro:
  `(a*)*b` caiu de 927 µs para 0,5 µs (medido fora do jogo).
- `BwmsPopulationSystemIsSpawning` chamava a função vizinha.
- Entidade morta de outra sessão não conta mais em `GetTaggedIDs`, `GetTags` e `IsTagged`.
- `BwmsToCharacterKey(key, shift)` devolvia sempre maiúscula.
- `ModLog` não descarta mais linha quando a trava está ocupada.
- A leitura dos recursos já carregados (`BwmsDepotIsResourceLoaded`, `BwmsDepotLoadedResourceClass` e o que depende dela: `ResRefIsLoaded`,
  `ResRefResourceClass`, `BwmsDepotClasseLonga`, `BwmsWorldNodeSetup`, `BwmsWorldNodeInstance.FromSectorSetup`, `BwmsInstancedMeshNode`,
  `BwmsSectorCount`) lia um endereço que só acertava pela ordem em que o motor aloca memória e podia falhar com archives na pasta
  `Mods/`: com 4 archives, falhou sempre (2026-10-18, prova do defeito:
  `cp77-symbols/notes/proofs/20261018-depot-cache-com-mods-antes-do-conserto.log`); com 2, em 2026-09-28, ainda acertava
  (`cp77-symbols/notes/proofs/smoke-20260928-111535.log`). Agora lê o mapa pelo loader do motor. Conferido em jogo (2026-10-18): com 4
  archives na `Mods/`, o mapa foi achado pelo loader, a varredura visitou 60.888 recursos e a busca achou o setor do mundo
  (`cp77-symbols/notes/proofs/20261018-depot-cache-com-mods-conserto.log`).

**Motor e estabilidade**

- Native de array ou handle usada direto numa expressão (`ArraySize(sys.GetTaggedIDs(tag))`) escrevia em retorno nulo e derrubava o
  jogo. Não derruba mais, mas na forma inline o valor ainda sai errado (115 no lugar de 1): leia o retorno num local antes de `ArraySize`.
- Array do motor que crescia por push do BWMS (`AddComponent`, controller secundário) derrubava o jogo ao ser solto.
- Leitura de texto do RTTI com tamanho lixo derrubava o jogo na carga dos scripts.
- Natives do BWMS chamadas em threads de job às vezes voltavam vazias, e uma native podia ler o descritor de outra.
- Chamadas que devolviam `ref<>` vazavam uma referência a cada chamada, em 30 pontos ao todo, entre eles os cheats `hasgod`, `godmode`,
  `heal` e `level` e as natives `BwmsReflFuncCall` e `BwmsCallPlayerMethod`.
- `KeyInputEvent.GetKey`/`GetAction` escreviam 8 bytes num retorno de 4.
- O overlay chamava o AppKit fora da thread principal. Num boot em que o jogo levou mais de 8 s para chegar ao AppKit, o jogo caiu com
  SIGTRAP.
- O resolvedor de endereços podia congelar uma passada incompleta pela sessão inteira (1 de 806 endereços). Pesa em build que não é a
  Steam, que hoje o instalador só aceita com `BWMS_IGNORE_BUILD_CHECK=1`.
- Endereço fora da imagem do jogo é recusado, em vez de virar salto para lixo.
- O gancho do pulo de vídeos gravava 16 bytes sobre uma função de 4 e estragava o começo da vizinha. Nenhum crash foi atribuído a isso.
- Travamento intermitente antes da tela "pressione qualquer tecla": o gancho do BWMS na abertura de vídeo não repassava um argumento (as
  flags) ao Bink, e, dependendo do boot, isso prendia a thread de leitura do jogo. Consertado em 2026-10-24
  (`cp77-symbols/notes/proofs/20261024-trava-fase0-amostra-2` tem a amostra do defeito; `20261024-integracao-final-roteiro-6de6`, o
  roteiro completo depois).
- Outros 2 ganchos do BWMS consertados: um não repassava um argumento do registro de atualizações do motor; outro, num trecho de 8 bytes,
  sobrescrevia o começo da função vizinha.
- Um crash depois de chegar ao gameplay fazia o boot seguinte pular o carregamento automático do save.
- Sem plugin instalado, o BWMS procurava a função de update de plugin a cada tick e vazava memória.
- A CString do caminho de cada archive que o BWMS montava vazava. Agora é solta pelo destrutor do próprio motor. Provado em 2026-10-18,
  na 2.3.1, com 4 archives da `Mods/` (4 soltas, nomes do depot certos 180 s depois). Na 2.5, os archives da `Mods/` são montados pelo
  próprio jogo.

**Terminal**

- `scenedump` saiu do terminal: derrubava o jogo.
- Cheat de mod de terceiro registrado por `BWMSDefH` com setas nunca chamava o `BWMSOnArrow` do handler.

**Desempenho (medido fora do jogo)**

- Executor de scripts: o que o BWMS soma a cada chamada de script do jogo foi de 32,85 ns na 0.1.4 para 8,10 ns (−75%), mesmo teste nas
  duas fontes, voltas alternadas. Quase toda chamada do jogo sai por um filtro antes da tabela de rotas, e a interceptação de prova
  saiu do build do usuário. Boot até a gameplay: sem ganho mensurável. FPS: não medido.
- `BwmsConfigGet`: de 9,1 µs para 1,0 µs por chamada. Só relê o arquivo quando ele muda.
- Busca de método e de função global com cache.
- `BwmsDepotIsResourceLoaded`/`BwmsDepotLoadedResourceClass` vão direto ao balde do cache, em vez de varrer ~55 mil entradas.
- `ResRef.GetHash` e `EngineTime.ToTicks` não resolvem mais CName a cada chamada.

## Instalador e extras

- **[0/6] build do jogo:** antes de tocar em qualquer coisa, o instalador lê 22 pontos do binário do jogo e compara com a build 2.5
  da Steam (2.5.0, build 5814786) e com a da GOG (mesma versão, outro binário). Build diferente é recusada com o jogo intacto, e isso
  inclui a 2.3.1. Epic: não suportada. `BWMS_IGNORE_BUILD_CHECK=1` força a instalação.
- **Instalação na 2.5, pelo kit (build de desenvolvimento):** portão 22 de 22, binário 2.5 guardado como original, teste de abertura e
  scripts compilados com o compilador da 2.5.
- **[6/6] natives:** recusa se alguma `native func` global dos `.reds` (com ou sem `public`/`static`) não aparecer no dylib, em vez de
  deixar o jogo fechar mudo no boot.
- **[6/6] compilador de scripts:** o instalador copia o `scc` e a `libscc_lib.dylib` para `red4ext/` e grava `red4ext/bwms-scc.txt`; é
  essa cópia que compila os `.reds` na abertura do jogo. O compilador empacotado lê o formato de scripts da 2.5. Se a cópia falha (disco sem espaço, por exemplo), o instalador avisa, e os
  `.reds` da `Mods/` não compilam.
- **Arquivos de versões anteriores:** o instalador apaga os `.reds` que o 0.1.3 e o 0.1.4 instalavam e o 0.1.5 não leva mais (só esses,
  pelo nome; arquivos de outros mods na pasta ficam). Com o `codeware-ui-customcontroller.reds` do 0.1.3 na pasta, nenhum script
  compilava. A lista cobre também os nomes que mudaram: os `.reds` do BWMS que levavam o nome de outro framework (`codeware-*`,
  `archivexl-*`, `tweakxl-*` e outros) agora se chamam `bwms-*` ou `ponte-*`, e o nome antigo sai na atualização.
- **Volta automática:** se a assinatura do app falha, o binário original e o cache do redscript voltam e a pasta `blackwall-mods` sai.
  Na recusa do [6/6], o instalador para e manda rodar `--restore`.
- **Backup e `DESINSTALAR.command`:** na instalação, um binário sem o load command do BWMS é o arquivo da loja para a build atual e
  passa a substituir o backup. Antes, o backup ficava o da primeira instalação, de outra build depois de uma atualização do jogo. No
  `DESINSTALAR.command`, se o binário já não tem o load command (a Steam já pôs o dela numa atualização ou no "Verificar integridade"),
  nada é copiado por cima: só saem os arquivos do BWMS (`r6/scripts/blackwall-mods` e as dicas de compilação). Antes, ele punha o
  binário velho por cima do novo. Testado fora do jogo com um binário de teste nos três casos: com o load command, sem ele e ilegível.
  O desinstalador remove também a pasta `archive/Mac/mod/BWMS-Mods/`.
- **GOG Galaxy:** aviso antes de instalar, se ele estiver aberto (ele repara o jogo e desfaz a instalação).
- O instalador põe em `red4ext/bwms-heranca.yaml` de quem cada record do jogo herda (o TweakDB compilado não guarda): a mudança de um mod
  num record do jogo chega aos records que herdam dele, como o `InheritanceMap` do TweakXL. Gerado das fontes `.tweak` da 2.5.
- O instalador grava `red4ext/bwms.toml` com `pular_intro = true`, `splash = true` e `continuar_automatico = true`, só se o arquivo não
  existir.
- Dicas de erro de compilação para mods do Windows vão para `r6/config/redsUserHints/bwms-dicas.toml`; a cópia com o nome antigo
  (`bwms.toml`) nessa pasta sai na instalação. `DESINSTALAR.command` apaga as duas.
- `bwms-report.command` não precisa mais de `python3` nem das Command Line Tools, e mostra o modo de boot lido do `bwms.toml`.

## Ferramentas do GitHub

- **Uma CLI só, `bwms`:** o `cp77-mods`, o `tweakdb-tool`, o `archive-tool` e o `input-loader` viraram `bwms mods`, `bwms tweak`,
  `bwms archive` e `bwms input`, com um build só, na pasta `bwms/`.
- **`bwms mods` não instala mais:** o install, o apply, o list, o remove, a biblioteca Nexus (`nexus/mods/`, import, `nxm://`) e a TUI
  saíram com a biblioteca, e também o `bwms list` e o `bwms apply`. Fica o `bwms mods checar`. As tabelas que o runtime lê dos `.xl`
  são geradas pelo próprio BWMS na abertura do jogo, a partir da `Mods/`: tabela gerada que fica sem linha sai; tabela escrita à mão
  fica.
- **`resource.link`/`resource.copy`:** mapas separados, como no ArchiveXL. Na mesma chave, `copy` vence `link`. Ciclo (A→B→A) não entra
  mais em laço e é avisado em PT-BR. A cadeia sai colapsada (A→B→C vira A→C) e o plano junta os `.xl` de todos os mods da `Mods/`.
- **Gravação atômica** nas tabelas geradas.
- **Leitor de `.xl`:** entrada conhecida mal escrita gera aviso em PT-BR com o lugar e o que ficou de fora, em vez de sumir.
- **`bwms mods checar`:** aponta cada `native func` dos `.reds` que nem o BWMS nem um plugin registra, com arquivo e linha. Compila num
  `r6` temporário, sem tocar no cache de scripts do jogo.
- **`bwms tweak registro <mod>`:** sem aplicar nada, lista os records que o mod afeta, os records e recursos que ele passa a
  referenciar e problemas visíveis no lote (`TweakChangelog` do TweakXL).
- **`bwms archive`:** `c2dadd` regrava a cópia da tabela que o `.csv` cozido guarda depois dos campos; antes, linha acrescentada nunca
  chegava ao jogo. `tira-campos` novo. `extract-one` aceita o hash do caminho (`0x…`).
- **Fonte:** o código de Lua e a dependência LuaJIT saíram. Os `.lua` de mod de CET rodam no crate `bwms-lua`, em Rust, sem dependência.
- **Crates como biblioteca:** `bwms-hashes` e `bwms-core` viraram um crate só, `bwms-primitives`, e mudaram de API. `Regex::compilar`
  devolve `Result`; `theme::load_states` devolve `StateLoad`.

## Mudanças que pedem ação

- Plugin nativo em Rust: os campos `imgui_begin`, `imgui_text` e `imgui_end` da `BwmsApi` passam a se chamar `janela_abrir`,
  `janela_texto` e `janela_fechar`. O binário já compilado continua funcionando; só quem recompila troca os nomes.
- O build do usuário não lê mais `~/.bwms-skipintro` nem `~/.bwms-autocontinue`. Quem usava esses marcadores no 0.1.4 precisa ajustar o
  `bwms.toml` ou o seletor "Pular boot".
- `Version()` do ArchiveXL, do TweakXL e do Codeware devolve `0.1.5`. Mod que compara `Version()` com 1.x lê errado: use `Require()`.
- Sob `ArchiveXL` ficam só os 8 métodos da fachada do ArchiveXL (`Require`, `Version`, `RegisterArchive`, `RegisterDir`, `Reload`,
  `EnableGarmentOffsets`, `DisableGarmentOffsets`, `GetBodyType`). As funções que o BWMS criou (`RegisterLink`, `GetAlias*`, aparência
  dinâmica, tags visuais, override de aparência e deslocamento de roupa) estão na classe `BwmsAparencia`: script que as chamava por
  `ArchiveXL.` troca o prefixo para `BwmsAparencia.`.
- Quem usava o 0.1.4 na GOG atualiza o jogo para a 2.5 antes: o [0/6] recusa a GOG 2.3.1.
- O 0.1.5 é para a 2.5: na 2.3.1, o [0/6] recusa a instalação.
- A biblioteca do BWMS (`BWMS/mods/`, `nexus/mods/`) e o `bwms mods` que instalava saíram. Mod novo vai na pasta `Mods/` ou pelo menu
  Mods do jogo. Na atualização, confira no relatório os mods que ficaram em `BWMS/mods` (desligados ou com nome repetido na `Mods/`).
  O relatório sugere mandar a pasta `BWMS/` para a Lixeira; o BWMS não apaga nada dela.

## Honestidade sobre o estado

Isto é **beta**. Quase toda prova em jogo foi feita com o build de desenvolvimento. Ele tem o instrumental de teste e um caminho que o
build do usuário não tem (o censo de spawners). O dylib do usuário do pacote de 2026-10-05 teve boots próprios até o fim do load do
save, sem queda, e aquele pacote, instalado pelo próprio `INSTALAR.command` num jogo sem mod (136 `.reds`), carregou o save em 126 s e
seguiu rodando, sem crash. Depois dele, cada rodada de mudanças fechou com o build de desenvolvimento em gameplay e o roteiro de testes
sem falha nova. Na de 2026-10-18, com 5 mods reais na `Mods/`, 26 testes passaram e 1 falhou, o `axl49scan`: a falha era o
defeito do leitor do cache de recursos com archive na `Mods/`, consertado (ver "Correções", Codeware). Conferido em jogo (2026-10-18):
com 4 archives na `Mods/`, o mapa foi achado pelo loader, a varredura visitou 60.888 recursos e a busca achou o setor do mundo
(`cp77-symbols/notes/proofs/20261018-depot-cache-com-mods-conserto.log`). Os roteiros finais de 2026-10-18 deram 27 testes passando, 0
falhas e 1 pulado com os 5 mods na `Mods/` (`cp77-symbols/notes/proofs/smoke-20261018-135441.log`) e os mesmos 27/0/1
(`cp77-symbols/notes/proofs/smoke-20261018-135542.log`) sem mods, com a prova do Metal OK
(`cp77-symbols/notes/proofs/20261018-prova-metal-sem-mods.log`).
As provas até 2026-10-19 foram feitas na 2.3.1. Na 2.5, o que foi provado está nos dois primeiros blocos abaixo. O instalado pelo zip com o
build público na 2.5: a provar.

**Provado em jogo na 2.5** (build de desenvolvimento, boots de 2026-10-23)

- Boot até a gameplay com o save carregado sozinho (120–175 s), sem crash; splash, pulo dos vídeos e saída limpa
  (`cp77-symbols/notes/proofs/20261023-boot-25-gameplay-dev.log`).
- Roteiro de provas, 5 de 5 (`cp77-symbols/notes/proofs/20261023-provas-25-cinco-blocos-pass.log`):
  - Tweaks pela `Mods/` (`.yaml` e `.tweak` de um mod de teste do BWMS): 13 de 13 valores no TweakDB.
  - `.archive` da `Mods/` pela pasta de mods do próprio jogo (`archive/Mac/mod/BWMS-Mods/`): 4 de 4 montados pelo jogo, sem gancho.
  - Aba Mods: liga e desliga mod, com disco e aviso "Reiniciar para aplicar" certos. O liga/desliga saiu depois, com a biblioteca.
  - Aba Cheats: God Mode, Invisibilidade, Zerar procurado.
  - Telas do Virtual Atelier (loja e lista) e do Equipment-EX (guarda-roupa pelo inventário e pelo menu do jogo): 4 de 4 com os itens
    e o controller do mod. Escritas do Equipment-EX no TweakDB: `SetFlat` de lista e de texto 1898 de 1898; `CreateRecord` de slot
    pelo nome curto 54 de 54.
- Textos de interface (`localization.onscreens`) dos `.xl` da `Mods/` pelo ArchiveXL do próprio jogo: com Equipment-EX e Virtual
  Atelier, `.json` gerado pelo BWMS, aplicado pelo jogo e texto conferido no jogo
  (`cp77-symbols/notes/proofs/20261023-provas-25-roteiro-com-xl-nativo.log`).
- Instalação pelo kit na 2.5: portão 22 de 22, binário 2.5 guardado como original, teste de abertura, scripts compilados com o
  compilador da 2.5.

**Provado em jogo na 2.5** (build de desenvolvimento, boots de 2026-10-24)

- Mod da `Mods/` com `.reds` quebrado: o jogo abriu até a gameplay, o mod saiu sozinho e ficou vermelho no relatório com o
  arquivo:linha do erro; Equipment-EX, Virtual Atelier e o BWMS compilaram e entraram
  (`cp77-symbols/notes/proofs/20261024-etapa2-mod-quebrado-sai-sozinho`).
- Selo e aba Mods com os mods da `Mods/` e do mod.io, luzes, contagem e conflitos; relatório com `luz / light` e `[mod.io]`
  (`cp77-symbols/notes/proofs/20261024-etapa3-selo-e-aba-dois-caminhos-PASS`).
- `factories` dos `.xl` da `Mods/` aplicado pelo ArchiveXL do próprio jogo
  (`cp77-symbols/notes/proofs/20261024-etapa4-lane-f-integrada-PASS`).
- Roteiro completo depois do conserto do travamento antes de "pressione qualquer tecla"
  (`cp77-symbols/notes/proofs/20261024-integracao-final-roteiro-6de6`).

**Provado em jogo na 2.3.1** (build de desenvolvimento, até 2026-10-19)

- Aba Mods do terminal (2026-10-18): a aba e a caixa do Quadra Turbo R achadas e clicadas pelo estado do mouse do terminal. Ligar:
  `active: true`, o archive no content igual à fonte, caixa marcada, "⚠ Reiniciar para aplicar" visível. Desligar: `active: false`, o
  archive fora do content, caixa desmarcada, o aviso some. Isso cobre o mesmo mod alternando mais de uma vez na sessão e o aviso
  comparando com o boot. O liga/desliga saiu depois, com a biblioteca.
- Aba Cheats (2026-10-18): Pausa, Configurações e a aba Cheats abertas por evento do próprio jogo; o clique do teste chama o
  `AcceptValue` do seletor vivo na tela, que é o que o clique no corpo do seletor chama (o mouse em si não entra). God Mode: desligar
  pinta 1→0 e o V perde a invulnerabilidade (`godmode=0` gravado); ligar volta. Invisibilidade: o clique pinta na hora, o efeito entra
  quando o menu fecha e sai no 2º clique. Zerar procurado: com 1 estrela, heat=0 e sem perseguição. Reabrir a aba pinta o estado real.
  22 vereditos, 0 falhas; saves iguais
  (`cp77-symbols/notes/proofs/20261018-provas-015-tweakxl-pathb-abamods-abacheats.log`).
- TweakXL pela `Mods/` com um mod de teste do BWMS (`.tweak` e `.yaml`): 13 de 13 valores no TweakDB depois do save carregar (mesmo
  boot).
- CString do caminho dos archives da `Mods/`: 4 soltas pelo destrutor do motor; nomes do depot certos 180 s depois (mesmo boot).
- Telas do Virtual Atelier (2026-10-18): loja aberta pelo pedido de vendedor do jogo, grade com 5 itens, todos com o controller do mod;
  lista de lojas aberta no inventário, com a página vazia explícita. Override de controller e injeção da dependência também no spawn
  assíncrono (`cp77-symbols/notes/proofs/20261018-telas-de-mod-async-va-2de2-eex-grade-vazia.log`).
- Terminal desenhado pelo BWMS: no jogo, o autoteste trocou de aba, focou o campo, digitou `help` e deu Enter (+9 linhas, teclas de
  volta ao jogo), e o plugin de exemplo antigo desenhou sem recompilar.
- Pasta `Mods/`: o archive vence o jogo base e a expansão; entre dois mods vence o primeiro. No boot de 2026-10-18 (build de
  desenvolvimento), com Equipment-EX, Virtual Atelier, Quadra Turbo R, JB Third Person e input_loader na `Mods/`: 4 archives entraram,
  os 2 `.xl` foram para as tabelas, 48 `.reds` compilados na abertura (877 ms), 0 conflitos, o `.lua` do JB e a `.dll` do input_loader
  listados como fora no relatório, e o jogo chegou à gameplay (prova: `cp77-symbols/notes/proofs/20261018-mods-reais-dev.log`).
- `Require()` dos três frameworks (5 de 6 chamadas ao vivo; a 6ª não chegou pelo canal de teste).
- Virtual Atelier e Equipment-EX compilam, carregam e seguem 60 s ou mais em gameplay (build de desenvolvimento, boot de 09-28, antes da
  reescrita do `Show`/`ShowInput` e do conserto do popup reaberto). As classes da ponte funcionam como controllers do motor, e o popup
  abre e fecha.
- `ScriptableTweak`: o crash antes e o jogo vivo depois.
- Índice de flats do TweakDB, comparado pelo próprio motor; escritor único, `.tweak`, YAML, recarga sem empilhar, buffer que cresce,
  genealogia e remoção de record, `$type` com padrões, stat novo por YAML; clone que herda pela C-ABI.
- `appearanceName` no spawn; teto de world widgets; `ResolveName`/`ResolvePath` dinâmicos; estado de braços e pés; handler do V criado ao
  entrar no jogo; `GetTrackedQuest`.
- Seções do `.xl` aplicadas pelo runtime a partir da tabela em `red4ext/`, com tabela montada para o teste: `resource.patch`/`fix`/
  `scope`, `streaming.blocks`, `streaming.sectors`, `journal`, `quest.phases`, `customizations`, `localization` (onscreens),
  `resource.copy`. Das tabelas geradas pela ferramenta, só a de `resource.patch` foi a jogo. `customizations` foi provado chamando o
  fluxo de edição direto, sem abrir o criador de personagem nem o espelho.
- Entidades: `DynamicEntitySystem`, `persistSpawn`/`persistState`, `StaticEntitySystem`, `EntityBuilder`, tags, workspot,
  `BwmsPopulationSpawner`, `ApplyMorphTarget` (peso conferido) e `SetWorldTransform` (posição local conferida).
- `CallbackSystem`: família `Entity/*`, `Session/Ready`, `Resource/Load`, estágios do quadro, alvos corrigidos e novos.
- Recursos: `IComponent` em componente skinned, carga por caminho, `ResourceToken`, `BwmsResourceRef`, sound banks, serializador.
- Interface: campo de texto, `BwmsInkKeyEvent`, widget criado por script, dicas de botão, `BwmsWidgetRef`, tema, bonecos de prévia,
  `GetParentWidget`, `parentWidget`, `BwmsWidgetIsAttached`/`Reparent`, `BwmsPropU64`, clipboard longo, `GetWorldWidgetComponent`.
- Mundo: `WorldStateSystem`, comunidades, clima, `BwmsWorldNodeSetup`, instâncias de nó, `BwmsQuestPhaseGraphAccessor`,
  `BwmsExecuteNode`, reset de rede de segurança, `TraceResult`, controller visual, `BwmsRenderProxyIsVisible`.
- Reflexão no campo certo e valor de qualquer tipo.
- RED4ext: argumentos de lançamento, screenshot, tempo do quadro, `ModLog` gravando em `bwms-mods.log` (caminho do build do usuário,
  provado no build de desenvolvimento), native de plugin no `.reds`, `OnUpdate` de plugin em gameplay, `call_global`,
  `class_create`/`class_add_property`.
- Motor: String inteira, native como argumento, natives em thread de job, vazamento de `ref<>` nos cheats.
- Boot: splash até o fim do load, `bwms.toml` lido pelo build do usuário, janela oculta com o splash na tela, registro de energia
  (inclusive no build do usuário), baixo consumo medido.
- SIGTRAP do AppKit: o crash ficou registrado uma vez (`.ips`), a causa foi lida nele, e o boot depois do conserto passou. O caso lento
  (o jogo chegando ao AppKit depois de 8 s) não se repetiu: [não medido].
- God Mode: 2 fontes do save → 0 com um `godmode off` do terminal.
- Terminal: comandos respondendo na aba Console, atalho configurável, crash do `scenedump`.
- Resolvedor de endereços: 806 de 806 na Steam.
- Ponte própria com o Metal: splash, selo, terminal e filtro de cor conferidos no jogo pela sonda de pixels do build de desenvolvimento.
- Scripts com o nome novo (`bwms-*`, `ponte-*`): compilam e o jogo chega à gameplay com eles (build de desenvolvimento).

**Implementado, sem prova ao vivo**

- `RegisterArchive`/`RegisterDir` com o carregador novo.
- Pasta `Mods/`: nenhum mod real de TweakXL do Nexus (`.yaml`/`.tweak`) foi testado em jogo; o provado é um mod de teste do BWMS.
  Erro no BWMS ou em mod direto em `r6/scripts` (o jogo abre com a última compilação que deu certo e o erro sai com arquivo:linha) e
  mod novo direto em `r6/scripts`: testados fora do jogo.
- Migração da biblioteca no instalador (0.1.2–0.1.4 → 0.1.5): testada fora do jogo, numa pasta de jogo falsa.
- `GetItemInstance()` do `inkWidgetSpawnEvent` não foi provado em jogo.
- Tecla entregue a widget de mod: religada no código, sem prova em jogo.
- O instalado pelo zip com o build público na 2.5: a provar.
- `Version()` dos três frameworks devolvendo `0.1.5`.
- O aviso de requisito do Equipment-EX deixando de abrir.
- `ApplyAppearanceOverride` devolvendo o resultado; `ApplyVisualTagOverride` sobre a máscara atual. A regra das máscaras por mod foi
  provada no componente vivo do V por uma função de teste; as natives de produção não foram chamadas ao vivo.
- Consertos menores do TweakDB (`AddFlat`, `GetRecords`, `!remove`, `ReinheritFlat`, versão do TweakDB, `TweakXL.Reload`, `$type`).
  Os boots de regressão chegaram ao gameplay sem queda.
- Consertos do Codeware: `FindComponentByType`, `ForgetItemID`, ouvinte do `CallbackSystem`, `BwmsPopulationSystemIsSpawning`,
  `ModLog`, tags de outra sessão.
- `ResourceTarget` com `regex:` casando o caminho inteiro (só testes).
- `GetPoiMappinPhase`: 48 de 48 leituras na faixa 0..5; sem POI com estado salvo para comparar com o jogo.
- `SetPoiMappinPhase`: o endereço foi medido; a semântica não tem prova própria.
- `BwmsReflPropGetValueInt32`/`CName`: a declaração foi confirmada em boot, sem teste funcional.
- `GenericMessageNotification.ShowInput` e o popup reaberto.
- Setas de cheat de terceiro (`BWMSOnArrow`): não há mod com seta para testar.
- `BwmsLookAt` (parte do alvo sob a mira) e `BwmsNPC`.
- Com `pular_intro = false`, o resto do BWMS seguindo igual.
- Splash: saída com 0 saves ou com o save mais recente inválido; saída em versão de jogo que não bate; splash em Mac de 8 GB ou menos.
- Carregamento automático só no boot (voltar ao menu não recarrega).
- Escala 2× e 3× do terminal e janela mínima: testados fora do jogo contra a referência. Os boots rodam em 1280×800 (1×).
- Filtro de cor (LUT) com o terminal fechado: o comando aplica; a imagem não foi conferida em jogo.
- `spawnsub`: a chamada vai ao motor; o NPC nascendo não foi conferido em jogo.
- `ArchiveXL.Reload()` sem a tabela, recarga de `customizations` e patch de `.ent` na 1ª leva: só testes.
- Aba Cheats: o mouse em si não entra no teste (ele chama o `AcceptValue` do seletor vivo). A invisibilidade no save é inferida do
  registro do jogo (efeito salvável), não medida.
- `bwms-report.command` sem Python (saída igual à antiga em 10 de 10 `.ips` reais).
- Instalador: checagem de build, checagem de natives e volta automática, testadas fora do jogo.
- Instalador apagando os nomes antigos dos `.reds` na atualização: rodada com o zip final em 2026-10-18 (0.1.4 instalada antes, pelo zip dela): os 85
  nomes da 0.1.4 saíram e ficaram os 137 do pacote; o build público chegou ao fim do load em 115 s com 5 mods reais na `Mods/`, sem
  relato de crash (`proofs/20261018-zip-015-final-usuario-014-para-015-com-mods.log`).
- Ferramentas do GitHub: tudo testado fora do jogo, menos a tabela de `resource.patch` gerada pela ferramenta, que foi a jogo.
- Ganhos de desempenho: medidos fora do jogo; o ganho em jogo é [não medido].

**Limites conhecidos**

- Plugin `.dll` é de Windows e não carrega no Mac.
- `RegisterArchive` injeta no grupo `content`, que perde para a expansão.
- Fora da `Mods/`, as seções do `.xl` não são aplicadas (só tabela escrita à mão em `red4ext/`).
- `.xl` pela via nativa do jogo (ArchiveXL próprio da 2.5, que lê `.json`): os textos de interface (`localization.onscreens`) e o
  `factories` dos `.xl` da `Mods/` vão por ela. As outras seções continuam com o BWMS, e os textos também voltam ao BWMS quando há junto
  textos de mod do mod.io ou de outra pasta de mods do jogo. `factories` que o jogo não aplica fica sem efeito, e o mod fica amarelo
  com o motivo.
- `localization` do `.xl` aplica só `onscreens`. `player.bodyTypes` não está implementado.
- Só a 2.5 da Steam e da GOG. Epic e Mac App Store: não suportadas. A 2.3.1: não suportada.
- `{nails_color}` em NPC sem componente de unha fica vazia (no V, sem o componente, vem da customização dos braços).
- `worldCookedCollisionNode` (colisão cozida da 2.5): sem deleção de shape (`[ator, shape]`), como no ArchiveXL.
- Records novos da 2.5 que as fontes `.tweak` públicas ainda não trazem ficam sem herança no mapa: mudança num record base desses não
  chega aos filhos.
- O evento `InkWidget/Spawn` sai só do spawn síncrono, não do `AsyncSpawnFrom*`.
- Fora da thread principal o BWMS não troca controller, não injeta a dependência nem dispara o evento: o motor também chama o
  `SpawnFrom*` de outras threads, e ali o spawn fica como o do jogo.
- Mod em `r6/scripts` que declara native de um plugin de Windows que o BWMS não tem impede a compilação dos scripts. Se os scripts
  compilados em uso (em `r6/cache`) já têm essa native, o jogo fecha ao ligar os scripts até o mod sair de `r6/scripts`; se não têm, o
  jogo abre com os da última compilação que deu certo. O relatório diz qual mod e quais natives. Mod assim na `Mods/` fica fora da
  compilação, com a linha "fora / left out: … precisa de um plugin de Windows" no relatório.
- `GetCharacter()` do `BwmsInkKeyEvent` segue US-QWERTY.
- No build do usuário não há censo de spawners: `BwmsSpawnerCensusCount()` devolve sempre 0, e o id do spawner tem que vir do mod.
- `BwmsPersistencySystemGetEntityStatus` devolve sempre `NotPersisted`: a função do motor não foi provada e não é chamada.
- Num boot de teste (build de desenvolvimento), o splash ainda saiu pelo watchdog (138 s), e com a janela oculta o jogo pausou.

Se algo quebrar, o `extras/bwms-report.command` gera um relatório pronto para colar numa issue do GitHub
(https://github.com/Blackwall-sys/black-wall-mod-system/issues). Os relatos são conferidos aos domingos.
