[English](README.md) · [Português](README.pt.md) · [简体中文](README.zh.md)

# BWMS — Black Wall Mod System

**Runtime nativo de modding de Cyberpunk 2077 para macOS / Apple Silicon.**

100% Rust + redscript. Sem máquina virtual, sem Wine, sem streaming do Windows — os
mods rodam diretamente dentro do build nativo macOS do jogo.

> **Status: ALPHA 0.1.2** — cedo, mas real. Este README é honesto sobre o que
> funciona hoje versus o que está no roadmap. Apenas single-player.

---

## O que faz

O BWMS é um runtime nativo e um conjunto de ferramentas de dados para o build Apple Silicon
do jogo. A partir da 0.1.2:

- **Console in-game + overlay ImGui** — um console de desenvolvedor renderizado sobre o
  jogo via um overlay ImGui baseado em Metal.
- **Cheats** — god mode, capacidade de carga, toggles de dano e recursos, eddies,
  atributos, perks, veículos e conveniências single-player similares, expostos
  como ações nativas em redscript.
- **Edição ao vivo do TweakDB** — ler e editar records no TweakDB em execução
  (dano, stats, flats) sem reempacotar archives.
- **Reflection para modders** — ler e escrever campos e chamar métodos por nome
  contra objetos vivos do jogo através do RTTI da engine.
- **Ferramentas de archive** — ler e extrair containers `.archive`.
- **Gerenciador de mods** — instalar, listar e remover mods de forma transacional.

É software alpha. Espere arestas. Sempre faça backup dos seus saves antes de usar
cheats (veja o disclaimer no rodapé).

---

## Requisitos

- macOS em **Apple Silicon** (M1 / M2 / M3 / M4).
- **Rust** (stable) instalado via [rustup](https://rustup.rs), com o
  target `aarch64-apple-darwin`.
- `python3` e `codesign` — ambos vêm com o sistema base do macOS.
- Uma cópia legítima e instalada de Cyberpunk 2077 (build macOS, via Steam).

Você **não** precisa de Xcode ou Homebrew para compilar o runtime.

Adicione o build target uma vez:

```sh
rustup target add aarch64-apple-darwin
```

---

## Compilar a partir do código-fonte

Estes são os comandos exatos e reproduzíveis. O runtime e todas as ferramentas compilam
a partir de dependências do crates.io mais os crates locais incluídos neste repositório —
nada além disso é necessário.

### 1. Runtime (a dylib do produto)

```sh
cd cp77-console
./build.sh
```

`build.sh` compila com `cargo` em modo release, remapeia os build paths (por
privacidade), faz strip do binário, define o install-name como `@rpath`, assina o resultado
ad-hoc e o valida carregando-o com `dlopen`.

**Saída:** `target/release/libcp77_console.dylib`

O crate `cp77-console` depende apenas de pacotes do crates.io (`metal`, `imgui`,
`foreign-types`, etc.), então compila sozinho sem setup extra.

### 2. Ferramentas de dados (opcional)

Cada ferramenta é um crate Rust padrão. Compile qualquer uma com:

```sh
cargo build --release
```

executado de dentro do diretório daquela ferramenta:

| Diretório          | O que faz                             |
| ------------------ | ------------------------------------- |
| `archive-tool`     | Ler / extrair containers `.archive`   |
| `tweakdb-tool`     | Ler / editar `tweakdb.bin`            |
| `input-loader`     | Mesclar definições de keybind / input |
| `mac-mod-manager`  | Instalar / listar / remover mods      |
| `bwms`             | Front-end de linha de comando unificado |

`bwms` e `mac-mod-manager` usam o crate local `bwms-core`, que está incluído
neste repositório — nenhum fetch externo é necessário para ele.

### 3. Scripts redscript (in-game)

Os fontes redscript ficam em `r6/scripts/blackwall-mods/*.reds`. Eles são
compilados pelo compilador redscript `scc` embutido **no momento da instalação** pelo
instalador — não há etapa de compilação manual separada para os usuários finais.

---

## Instalar (usuários finais)

Para jogadores que só querem rodar os mods (sem necessidade de desenvolvimento):

1. Baixe o zip do release e descompacte-o.
2. Rode **`INSTALAR.command`** (ou `bwms-install.sh "<game dir>"` de um
   terminal).
3. Inicie o jogo pela **Steam (Play)** — não pelo Finder.

O instalador adiciona uma entrada `LC_LOAD_DYLIB` ao binário do jogo e reassina o
`.app` ad-hoc **preservando os entitlements originais da CDPR**. Ele usa apenas
ferramentas base do macOS (`codesign`, `xattr`): sem senha, sem mudanças no SIP ou
Gatekeeper, e é totalmente reversível.

Para desinstalar:

```sh
INSTALAR.command --restore
```

ou rode `extras/DESINSTALAR.command`.

---

## Layout do repositório

```
cp77-console/            A dylib do runtime (console in-game + overlay ImGui)
bwms-core/               Biblioteca compartilhada (núcleo classify / theme / apply)
bwms/                    Ferramenta de linha de comando unificada
archive-tool/            Ler / extrair containers .archive
tweakdb-tool/            Ler / editar tweakdb.bin
input-loader/            Mesclar definições de keybind / input
mac-mod-manager/         Instalar / listar / remover mods
r6/scripts/blackwall-mods/   fontes redscript (compilados no momento da instalação)
example-rust-plugin/     Plugin nativo de exemplo
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
