//! Os widgets do terminal, desenhados pelo projeto — nenhuma biblioteca de terceiro.
//!
//! POR QUE EXISTE: o dylib que vai ao usuário leva só código do projeto, e o terminal é o que a pessoa
//! usa pra pedir uma arma. O desenho é o da REFERÊNCIA aprovada pelo dono: as imagens do terminal
//! feitas por um renderizador C++ de terceiro, fora do binário, guardadas em
//! `cp77-symbols/notes/proofs/20261003-terminal-referencia-…`. Toda métrica e toda cor abaixo foi
//! MEDIDA nela, não estimada; `overlay::prova_visual` mede a distância até ela e trava cada pixel
//! aprovado.
//!
//! COMO FUNCIONA: modo imediato. A cada frame o chamador monta uma `Tela`, chama os
//! widgets na ordem em que aparecem e no fim `terminar()` entrega a `Malha` ao mesmo pipeline Metal
//! que o overlay e o splash já usam. Estado entre frames é só o que está em `Persistente`, em campos
//! planos — estado de widget é dado, não objeto.

use crate::terminal::edicao::{AreaDeTransferencia, Edicao, Evento, Mods, Pedido};
use crate::terminal::malha_de_desenho::*;

// ---------------------------------------------------------------- tema

/// As quatro paletas do terminal, na ordem do seletor: 0 ENTROPISM, 1 KITSCH (padrão),
/// 2 NEOMILITARISM, 3 NEOKITSCH. São as nove cores de cada tema da referência, sem arredondar nem
/// "melhorar" nenhuma.
pub(crate) const PALETAS: [[[f32; 4]; 9]; 4] = [
    // ENTROPISM — utilitário, âmbar/laranja sobre marrom escuro
    [
        [0.98, 0.70, 0.33, 1.0],
        [0.10, 0.07, 0.035, 0.94],
        [0.45, 0.25, 0.07, 1.0],
        [0.62, 0.34, 0.09, 1.0],
        [0.30, 0.18, 0.06, 1.0],
        [0.55, 0.30, 0.09, 1.0],
        [0.95, 0.58, 0.18, 1.0],
        [0.22, 0.14, 0.05, 1.0],
        [0.55, 0.30, 0.10, 1.0],
    ],
    // KITSCH (padrão) — teal/verde vibrante com toque rosa
    [
        [0.20, 0.98, 0.55, 1.0],
        [0.0, 0.18, 0.10, 0.93],
        [0.0, 0.45, 0.15, 1.0],
        [0.0, 0.58, 0.20, 1.0],
        [0.0, 0.30, 0.12, 1.0],
        [0.95, 0.25, 0.70, 1.0],
        [0.20, 0.98, 0.55, 1.0],
        [0.0, 0.25, 0.10, 1.0],
        [0.0, 0.50, 0.20, 1.0],
    ],
    // NEOMILITARISM — austero, vermelho/cinza sobre quase-preto
    [
        [0.86, 0.84, 0.82, 1.0],
        [0.05, 0.05, 0.06, 0.96],
        [0.40, 0.08, 0.07, 1.0],
        [0.58, 0.12, 0.10, 1.0],
        [0.22, 0.06, 0.06, 1.0],
        [0.50, 0.12, 0.10, 1.0],
        [0.85, 0.20, 0.16, 1.0],
        [0.12, 0.10, 0.11, 1.0],
        [0.45, 0.12, 0.10, 1.0],
    ],
    // NEOKITSCH — luxo, magenta + ouro sobre roxo profundo
    [
        [0.96, 0.63, 0.91, 1.0],
        [0.10, 0.04, 0.12, 0.94],
        [0.42, 0.10, 0.36, 1.0],
        [0.60, 0.15, 0.52, 1.0],
        [0.32, 0.10, 0.30, 1.0],
        [0.60, 0.16, 0.52, 1.0],
        [0.95, 0.78, 0.38, 1.0],
        [0.18, 0.08, 0.18, 1.0],
        [0.60, 0.18, 0.52, 1.0],
    ],
];

pub(crate) const TEXTO: usize = 0;
pub(crate) const FUNDO_JANELA: usize = 1;
pub(crate) const TITULO: usize = 2;
pub(crate) const TITULO_ATIVO: usize = 3;
pub(crate) const BOTAO: usize = 4;
pub(crate) const BOTAO_SOBRE: usize = 5;
pub(crate) const BOTAO_ATIVO: usize = 6;
pub(crate) const FUNDO_CAMPO: usize = 7;
pub(crate) const BORDA: usize = 8;

/// Cores que o tema NÃO define, iguais nos quatro temas: as da referência fora das paletas.
/// Várias são AZUIS (a marca do checkbox, o punho do slider, o realce de campo, a alça de
/// redimensionar), azul num tema verde/âmbar/vermelho/roxo. Ficam porque o desenho aprovado é
/// este; trocar pela paleta é decisão do dono.
pub(crate) mod herdada {
    pub(crate) const TEXTO_APAGADO: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
    pub(crate) const FUNDO_DICA: [f32; 4] = [0.08, 0.08, 0.08, 0.94];
    pub(crate) const CAMPO_SOBRE: [f32; 4] = [0.26, 0.59, 0.98, 0.4];
    pub(crate) const CAMPO_ATIVO: [f32; 4] = [0.26, 0.59, 0.98, 0.67];
    pub(crate) const TEXTO_SELECIONADO: [f32; 4] = [0.26, 0.59, 0.98, 0.35];
    pub(crate) const TITULO_RECOLHIDO: [f32; 4] = [0.0, 0.0, 0.0, 0.51];
    pub(crate) const TRILHO_ROLAGEM: [f32; 4] = [0.02, 0.02, 0.02, 0.53];
    pub(crate) const PUNHO_ROLAGEM: [f32; 4] = [0.31, 0.31, 0.31, 1.0];
    pub(crate) const PUNHO_ROLAGEM_SOBRE: [f32; 4] = [0.41, 0.41, 0.41, 1.0];
    pub(crate) const PUNHO_ROLAGEM_ATIVO: [f32; 4] = [0.51, 0.51, 0.51, 1.0];
    pub(crate) const MARCA_DO_CHECK: [f32; 4] = [0.26, 0.59, 0.98, 1.0];
    pub(crate) const PUNHO_DESLIZANTE: [f32; 4] = [0.24, 0.52, 0.88, 1.0];
    pub(crate) const PUNHO_DESLIZANTE_ATIVO: [f32; 4] = [0.26, 0.59, 0.98, 1.0];
    pub(crate) const SEPARADOR: [f32; 4] = [0.43, 0.43, 0.5, 0.5];
    pub(crate) const ALCA: [f32; 4] = [0.26, 0.59, 0.98, 0.2];
    pub(crate) const ALCA_SOBRE: [f32; 4] = [0.26, 0.59, 0.98, 0.67];
    pub(crate) const ALCA_ATIVA: [f32; 4] = [0.26, 0.59, 0.98, 0.95];
}

/// Métricas, medidas na referência com o tema aplicado.
const RESPIRO_JANELA: [f32; 2] = [8.0, 8.0];
const RESPIRO_CAMPO: [f32; 2] = [4.0, 3.0];
const ESPACO_ENTRE_ITENS: [f32; 2] = [8.0, 4.0];
const ESPACO_INTERNO: [f32; 2] = [4.0, 4.0];
pub(crate) const ARREDONDAMENTO_JANELA: f32 = 3.0;
pub(crate) const ARREDONDAMENTO_CAMPO: f32 = 2.0;
const ARREDONDAMENTO_ABA: f32 = 4.0;
const LARGURA_ROLAGEM: f32 = 14.0;
const PUNHO_MINIMO: f32 = 12.0;
/// Posição e tamanho da primeira vez que o terminal abre; depois o usuário arrasta e redimensiona,
/// e isso fica em `Persistente`.
pub(crate) const JANELA_INICIAL: [f32; 4] = [60.0, 150.0, 660.0, 460.0];
/// A janela não encolhe abaixo do tamanho com que abre: nele as 7 abas e as opções de cada uma cabem inteiras, e o dono decidiu que
/// tudo aparece sempre.
pub(crate) const JANELA_MINIMA: [f32; 2] = [JANELA_INICIAL[2], JANELA_INICIAL[3]];
/// Quanto da janela fica sempre na tela, de cada lado (medido na referência).
const JANELA_VISIVEL: f32 = 19.0;
/// Janela de plugin nova: o canto e o tamanho mínimo da janela de plugin na referência.
const JANELA_DE_PLUGIN: [f32; 2] = [60.0, 60.0];
const JANELA_DE_PLUGIN_MINIMA: [f32; 2] = [32.0, 32.0];
/// O ajuste ao conteúdo para a esta distância da borda da tela (medido na referência).
const MARGEM_SEGURA: f32 = 3.0;

/// Em que ponto da vida a janela está. A de plugin nova passa um quadro só medindo o conteúdo, sem aparecer, e o tamanho segue o
/// conteúdo nos 2 primeiros quadros, como na referência. O terminal nasce visível, com o tamanho dele.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum FaseDaJanela {
    /// Ainda não abriu nenhuma vez.
    Nova,
    /// 1º quadro: mede o conteúdo, ajusta o tamanho e não aparece.
    Medindo,
    /// 1º quadro na tela: ainda ajusta o tamanho.
    Aparecendo,
    Visivel,
}

impl FaseDaJanela {
    /// A fase do quadro seguinte em que a janela abre.
    fn ao_abrir(self) -> Self {
        match self {
            FaseDaJanela::Nova => FaseDaJanela::Medindo,
            FaseDaJanela::Medindo => FaseDaJanela::Aparecendo,
            FaseDaJanela::Aparecendo | FaseDaJanela::Visivel => FaseDaJanela::Visivel,
        }
    }

    /// Desenha e pega o mouse.
    fn aparece(self) -> bool {
        matches!(self, FaseDaJanela::Aparecendo | FaseDaJanela::Visivel)
    }

    /// O tamanho ainda segue o conteúdo.
    fn ajusta(self) -> bool {
        matches!(self, FaseDaJanela::Medindo | FaseDaJanela::Aparecendo)
    }
}

/// Uma janela da tela, com o que sobrevive ao quadro.
#[derive(Clone, Debug)]
pub(crate) struct Janela {
    /// Título com que a janela nasceu. A janela de plugin é achada de um quadro para o outro pela identidade dele (`identidade_da_janela`).
    pub(crate) titulo: String,
    /// Canto e tamanho: [x, y, largura, altura].
    pub(crate) ret: [f32; 4],
    pub(crate) recolhida: bool,
    pub(crate) fase: FaseDaJanela,
    /// Último `Persistente::quadro` em que abriu.
    ultimo_quadro: u64,
}

impl Janela {
    fn do_terminal() -> Self {
        Janela { titulo: String::new(), ret: JANELA_INICIAL, recolhida: false, fase: FaseDaJanela::Visivel, ultimo_quadro: 0 }
    }

    /// Abriu no quadro anterior a `quadro`. A que não abriu sumiu da tela e do mouse; quando volta, ganha o foco e vai para a frente.
    fn abriu_antes_de(&self, quadro: u64) -> bool {
        self.ultimo_quadro + 1 == quadro
    }

    /// Retângulo na tela; recolhida, só a barra de título.
    fn na_tela(&self) -> [f32; 4] {
        let [x, y, l, a] = self.ret;
        [x, y, x + l, y + if self.recolhida { ALTURA_TITULO } else { a }]
    }
}

/// `m` está dentro de `r` ([x0, y0, x1, y1], fechado em cima e à esquerda).
fn ponto_em(r: [f32; 4], m: [f32; 2]) -> bool {
    m[0] >= r[0] && m[0] < r[2] && m[1] >= r[1] && m[1] < r[3]
}

/// A janela de cima sob o mouse, pelos retângulos do quadro anterior (como na referência); movendo ou redimensionando, é a do arraste. Só os itens dela pegam o mouse: o clique numa janela não vaza para a de baixo.
fn janela_sob_o_mouse(p: &Persistente, m: [f32; 2]) -> Option<usize> {
    if let Arraste::Janela { janela, .. } | Arraste::Redimensionando { janela, .. } = p.arraste {
        return Some(janela);
    }
    p.ordem.iter().rev().copied().find(|&i| {
        let j = &p.janelas[i];
        let estava_na_tela = j.fase.aparece() && j.abriu_antes_de(p.quadro);
        estava_na_tela && ponto_em(j.na_tela(), m)
    })
}

/// Canto da janela `[x, y, largura, altura]` depois que a tela muda de tamanho (escala ou resolução): ela volta inteira para dentro, se
/// couber; senão fica no canto de cima à esquerda. Os dois renderizadores do terminal usam esta regra.
pub(crate) fn janela_dentro_da_tela(janela: [f32; 4], tela: [f32; 2]) -> [f32; 2] {
    let [x, y, l, a] = janela;
    [x.min(tela[0] - l).max(0.0), y.min(tela[1] - a).max(0.0)]
}

/// Métrica do texto: a da ProggyClean que o terminal sempre teve (ver `texto_de_terminal`).
const AVANCO: f32 = LARGURA_GLIFO as f32;
const LINHA: f32 = ALTURA_GLIFO as f32;
const ALTURA_DE_CAMPO: f32 = LINHA + RESPIRO_CAMPO[1] * 2.0;
/// A ProggyClean assenta o glifo 1 px mais baixo na linha do que a fonte do projeto (que deixa a
/// linha 0 vazia). Medido lado a lado: todo texto saía 1 px acima, com os retângulos no lugar.
const DESCIDA_DO_TEXTO: f32 = 1.0;
const ALTURA_TITULO: f32 = ALTURA_DE_CAMPO;

pub(crate) fn para_bytes(c: [f32; 4]) -> [u8; 4] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        (c[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
    ]
}

/// O que aparece de um rótulo: o que vem depois de `##` é identificador, invisível.
/// `"Give##Items.X"` mostra `Give`.
pub(crate) fn rotulo_visivel(r: &str) -> &str {
    r.split("##").next().unwrap_or("")
}

/// O que identifica uma janela pelo título, como no `igBegin`: com `###`, só dali em diante (o texto antes muda e a janela é a mesma);
/// sem, o título inteiro (`"Nome##a"` e `"Nome##b"` são duas).
fn identidade_da_janela(titulo: &str) -> &str {
    titulo.find("###").map_or(titulo, |p| &titulo[p..])
}

/// Colunas da linha mais larga e número de linhas, como a referência mede texto: o `\n` quebra, o `\r` não ocupa lugar e o `\n` do fim
/// não abre uma linha a mais.
fn colunas_e_linhas(t: &str) -> (usize, usize) {
    let (mut maior, mut atual, mut quebras) = (0usize, 0usize, 0usize);
    for c in t.chars() {
        match c {
            '\n' => {
                maior = maior.max(atual);
                atual = 0;
                quebras += 1;
            }
            '\r' => {}
            _ => atual += 1,
        }
    }
    let linhas = if atual > 0 || quebras == 0 { quebras + 1 } else { quebras };
    (maior.max(atual), linhas)
}

// ---------------------------------------------------------------- entrada do frame

/// O que o mouse e o teclado fizeram NESTE frame. Preenchido por quem chama, antes de desenhar.
#[derive(Default, Clone)]
pub(crate) struct Entrada {
    pub(crate) mouse: [f32; 2],
    /// Botão esquerdo apertado agora (estado).
    pub(crate) segurando: bool,
    /// Apertou neste frame (borda de descida) — inicia arraste e dá foco.
    pub(crate) apertou: bool,
    /// Soltou neste frame (borda de subida) — fecha um clique.
    pub(crate) soltou: bool,
    pub(crate) roda: f32,
    /// Caracteres digitados neste frame, já resolvidos pelo sistema (acento morto incluído).
    pub(crate) digitados: Vec<char>,
    /// Teclas de edição/navegação apertadas neste frame, por código do macOS.
    pub(crate) teclas: Vec<u16>,
    /// Teclado do frame em ordem, com os modificadores: o campo de texto edita por eles. Vazio = o campo usa `digitados` e `teclas`.
    pub(crate) eventos: Vec<Evento>,
    /// O aperto deste frame é o segundo de um clique duplo (pega a palavra no campo).
    pub(crate) clique_duplo: bool,
    /// Segundos desde o frame anterior: o cursor do campo pisca por eles.
    pub(crate) dt: f32,
    /// Tamanho da tela em pixels: a dica vira de lado perto da borda. `[0, 0]` = desconhecido, a dica nunca vira.
    pub(crate) tamanho_da_tela: [f32; 2],
    /// Desenhar o ponteiro por cima de tudo (o overlay liga com o terminal aberto; o jogo esconde o do sistema).
    pub(crate) ponteiro: bool,
    /// Procura de dev neste quadro (`termonde`, `termclique`): os itens com o rótulo ou sob o ponto saem em `Frame::achados`.
    #[cfg(any(test, feature = "devtools"))]
    pub(crate) procura: Option<Alvo>,
}

/// Tipo do item que os widgets anotam para a procura de dev (`Tela::anotar`).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum TipoDeItem {
    Botao,
    Caixa,
    Aba,
    Deslizante,
    Texto,
}

#[cfg(any(test, feature = "devtools"))]
impl TipoDeItem {
    pub(crate) fn nome(self) -> &'static str {
        match self {
            TipoDeItem::Botao => "botão",
            TipoDeItem::Caixa => "caixa",
            TipoDeItem::Aba => "aba",
            TipoDeItem::Deslizante => "deslizante",
            TipoDeItem::Texto => "texto",
        }
    }
}

/// O que a procura de dev busca no quadro: um rótulo (o visível, ou o inteiro com `##`) ou um ponto da tela, em pixels lógicos.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Alvo {
    Rotulo(String),
    Ponto([f32; 2]),
}

/// Um item achado pela procura de dev, como este quadro o desenhou.
#[cfg(any(test, feature = "devtools"))]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ItemAchado {
    pub(crate) tipo: TipoDeItem,
    /// O rótulo visível (sem o `##` em diante); no texto, o texto inteiro.
    pub(crate) rotulo: String,
    /// Onde o item pega o mouse: [x0, y0, x1, y1] em pixels lógicos.
    pub(crate) ret: [f32; 4],
    /// O ponto conferido (o centro do item, ou o ponto procurado) está na área visível da janela de cima: um clique ali chega ao item.
    pub(crate) visivel: bool,
    /// O que o item desenhou: caixa marcada, aba ativa, botão marcado. `None` no texto e no deslizante.
    pub(crate) estado: Option<bool>,
}

/// Teto de itens por procura: o veredito sai numa linha só.
#[cfg(any(test, feature = "devtools"))]
const TETO_DE_ACHADOS: usize = 8;

/// Forma do ponteiro desenhado pelo terminal: seta, I sobre campo de texto, diagonal sobre a alça de redimensionar.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub(crate) enum Ponteiro {
    #[default]
    Seta,
    Texto,
    Diagonal,
}

/// Miolo branco do ponteiro em grade legível (`o`) e o ponto quente nela; a borda preta de 1 px sai em volta, calculada. Tamanho do
/// ponteiro da referência (escala 0,5), desenhado nítido.
fn forma_do_ponteiro(p: Ponteiro) -> (&'static [&'static str], [f32; 2]) {
    match p {
        Ponteiro::Seta => (&["o....", "oo...", "ooo..", "oooo.", "ooooo", "oo...", "o...."], [0.0, 0.0]),
        Ponteiro::Texto => (&["ooo", ".o.", ".o.", ".o.", ".o.", ".o.", "ooo"], [1.0, 3.0]),
        Ponteiro::Diagonal => (&["ooo....", "oo.....", "o.o....", "...o...", "....o.o", ".....oo", "....ooo"], [3.0, 3.0]),
    }
}

/// Canto da dica, como a referência a posiciona: evita o retângulo do ponteiro (16 px à esquerda, 8 acima, 12 à direita e 12 abaixo
/// do mouse, com o ponteiro em escala 0,5) e tenta à direita (na altura do mouse), embaixo, em cima e à esquerda, nessa ordem; a que
/// cabe na tela ganha, presa à borda na outra direção.
fn posicao_da_dica(mouse: [f32; 2], tamanho: [f32; 2], tela: [f32; 2]) -> [f32; 2] {
    let [mx, my] = mouse;
    let [l, h] = tamanho;
    let evita = [mx - 16.0, my - 8.0, mx + 12.0, my + 12.0];
    if tela[0] <= 0.0 || tela[1] <= 0.0 {
        return [evita[2], my];
    }
    let preso_x = mx.min(tela[0] - l).max(0.0);
    let preso_y = my.min(tela[1] - h).max(0.0);
    if tela[0] - evita[2] >= l {
        return [evita[2], preso_y];
    }
    if tela[1] - evita[3] >= h {
        return [preso_x, evita[3]];
    }
    if evita[1] >= h {
        return [preso_x, evita[1] - h];
    }
    [(evita[0] - l).max(0.0), preso_y]
}

pub(crate) const TECLA_BACKSPACE: u16 = 51;
pub(crate) const TECLA_ENTER: u16 = 36;
pub(crate) const TECLA_ESQUERDA: u16 = 123;
pub(crate) const TECLA_DIREITA: u16 = 124;
pub(crate) const TECLA_BAIXO: u16 = 125;
pub(crate) const TECLA_CIMA: u16 = 126;
pub(crate) const TECLA_DELETE: u16 = 117;

// ---------------------------------------------------------------- estado entre frames

/// O que o mouse está arrastando. Uma máquina de estados explícita: sem ela, "arrastando" vira três
/// booleanos soltos que discordam entre si no frame em que o botão é solto.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub(crate) enum Arraste {
    #[default]
    Nenhum,
    /// Movendo a janela `janela`; guarda onde o mouse pegou, relativo ao canto.
    Janela { janela: usize, pega: [f32; 2] },
    /// Redimensionando a janela `janela` pela alça; guarda a distância do mouse ao canto no aperto, que o tamanho mantém.
    Redimensionando { janela: usize, pega: [f32; 2] },
    /// Arrastando o punho de rolagem da área `indice`; guarda o ponto de pega dentro do punho.
    Rolagem { indice: usize, pega: f32 },
    /// Arrastando o punho do deslizante `indice` (ordem de aparição).
    Deslizante { indice: usize },
    /// Selecionando texto no campo `indice` com o botão seguro.
    Texto { indice: usize },
}

/// O pouco que precisa sobreviver ao frame. Campos planos, um por assunto — nada de mapa de widget
/// por id, que é o que faz UI imediata virar UI retida com outro nome.
pub(crate) struct Persistente {
    /// Janelas na tela: a 0 é a do terminal; a de cada plugin entra pelo título na primeira vez que abre.
    pub(crate) janelas: Vec<Janela>,
    /// Ordem de desenho, de trás para a frente (índices em `janelas`): clicar numa janela a traz para a frente.
    pub(crate) ordem: Vec<usize>,
    /// Janela com o foco, de barra de título acesa. `None` depois de um clique fora de todas.
    pub(crate) janela_em_foco: Option<usize>,
    /// Quadros montados: diz se uma janela abriu no quadro anterior.
    pub(crate) quadro: u64,
    pub(crate) arraste: Arraste,
    /// Rolagem de cada área rolável, por ordem de aparição no frame.
    pub(crate) rolagem: [f32; 8],
    /// A área rolável teve barra no quadro anterior (conteúdo maior que o visível): só então ela reserva a largura da barra, como na
    /// referência, que decide pelo conteúdo do quadro anterior.
    pub(crate) com_barra: [bool; 8],
    /// Campo de texto com o foco, por ordem de aparição (o ↑/↓ do histórico vale com ele).
    pub(crate) foco: Option<usize>,
    /// Edição em curso no campo em foco. `None` com foco = o campo segue focado mas as teclas voltaram ao jogo (depois do Enter/Esc).
    pub(crate) edicao: Option<Edicao>,
    /// O Tab do campo anterior pediu edição no campo `foco` assim que ele for desenhado.
    pub(crate) comecar_edicao: bool,
    /// Rolagem horizontal do campo em edição, em pixels.
    pub(crate) rolagem_do_campo: f32,
    /// Relógio do cursor piscando, em segundos; negativo = aceso logo depois de mexer.
    pub(crate) piscar: f32,
    /// Texto do campo numérico em edição (pode estar no meio, como "-" sozinho).
    pub(crate) texto_do_numero: String,
    /// Botão `−`/`+` seguro: (identidade, relógio). O relógio começa em −0,275 s e cada volta acima de zero repete a cada 0,05 s.
    pub(crate) repete: Option<(usize, f32)>,
    /// Item que recebeu o aperto do mouse e ainda está seguro (o item ativo), pela ordem de aparição.
    pub(crate) item_apertado: Option<usize>,
    /// Aba escolhida em cada barra de abas, por ordem de aparição.
    pub(crate) aba_ativa: [usize; 4],
    /// Tamanho da tela no quadro anterior: quando ele muda, a janela volta inteira para dentro (`janela_dentro_da_tela`).
    pub(crate) tela: [f32; 2],
}

impl Default for Persistente {
    fn default() -> Self {
        Persistente {
            janelas: vec![Janela::do_terminal()],
            ordem: vec![0],
            janela_em_foco: Some(0),
            quadro: 0,
            arraste: Arraste::Nenhum,
            rolagem: [0.0; 8],
            com_barra: [false; 8],
            foco: None,
            edicao: None,
            comecar_edicao: false,
            rolagem_do_campo: 0.0,
            piscar: 0.0,
            texto_do_numero: String::new(),
            repete: None,
            item_apertado: None,
            aba_ativa: [0; 4],
            tela: [0.0; 2],
        }
    }
}

// ---------------------------------------------------------------- a tela do frame

/// Uma área onde os widgets caem, empilhada ao entrar numa janela ou numa área rolável.
#[derive(Clone, Copy)]
struct Area {
    /// Canto superior esquerdo do conteúdo (já deslocado pela rolagem, se houver).
    origem: [f32; 2],
    /// Borda direita e inferior VISÍVEIS.
    limite: [f32; 2],
    cursor: [f32; 2],
    /// Topo da linha atual e a altura do maior item nela.
    linha_y: f32,
    altura_da_linha: f32,
    /// O próximo item fica na linha do anterior (`mesma_linha`).
    na_mesma_linha: bool,
    /// Linha de base do texto na linha atual: o maior respiro vertical dos itens com quadro nela. Texto que vem depois na mesma linha
    /// desce até ela, como na referência.
    base_do_texto: f32,
    /// Área rolável: o índice dela em `Persistente::rolagem` e o topo visível. `None` na janela.
    rolavel: Option<(usize, f32)>,
    /// Largura reservada à direita para a barra de rolagem (0 sem barra).
    barra: f32,
    /// Recorte do que é desenhado dentro da área (fechar uma área devolve o da área de fora).
    recorte: [f32; 4],
    /// Canto inferior direito mais longe dos itens colocados: a janela de plugin ajusta o tamanho por ele.
    alcance: [f32; 2],
}

pub(crate) struct Tela<'a> {
    pub(crate) malha: Malha,
    entrada: Entrada,
    persistente: &'a mut Persistente,
    paleta: usize,
    escala: f32,
    pilha: Vec<Area>,
    n_rolaveis: usize,
    n_abas: usize,
    n_campos: usize,
    n_deslizantes: usize,
    /// Itens clicáveis do quadro, na ordem: a identidade do item ativo.
    n_itens: usize,
    /// Retângulo do último item colocado — é dele que `mesma_linha` e `dica` falam.
    ultimo: [f32; 4],
    dica: Option<String>,
    /// Se algum widget reivindicou o foco no clique deste frame.
    foco_reivindicado: bool,
    /// O mouse está sobre a janela neste frame (o jogo não pode receber o clique).
    mouse_na_janela: bool,
    /// De onde vem e para onde vai o Cmd+C/X/V do campo. Sem ela, copiar e colar não fazem nada.
    area: Option<&'a mut dyn AreaDeTransferencia>,
    /// Um campo já tratou o teclado deste quadro: o Tab que passa a edição adiante não é tratado de novo pelo campo seguinte.
    teclado_tratado: bool,
    /// Forma do ponteiro neste quadro: o item sob o mouse escolhe (campo de texto, alça).
    ponteiro: Ponteiro,
    /// Janela aberta agora (índice em `Persistente::janelas`) e onde o desenho dela começa na malha (índices, vértices).
    atual: Option<(usize, usize, usize)>,
    /// Janela de cima sob o mouse neste quadro (`janela_sob_o_mouse`).
    janela_sob_o_mouse: Option<usize>,
    /// Desenho de cada janela na malha: (janela, início, fim) em `indices`. O fim do quadro os reordena pela `ordem`.
    faixas: Vec<(usize, usize, usize)>,
    /// As janelas como estavam no começo do quadro, só com procura: o `janela_sob_o_mouse` de um ponto qualquer sai delas, porque cada
    /// janela aberta neste quadro já trocou o `ultimo_quadro`.
    #[cfg(any(test, feature = "devtools"))]
    janelas_no_inicio: Option<Persistente>,
    /// O que a procura de dev achou neste quadro.
    #[cfg(any(test, feature = "devtools"))]
    achados: Vec<ItemAchado>,
}

/// Fim da linha que começa em `ini`, pela regra de quebra da referência com fonte monoespaçada (1 coluna por caractere): quebra em branco ou depois de `. , ; ! ? "`; palavra que não cabe numa linha inteira é cortada onde estoura.
fn fim_da_linha(cs: &[char], ini: usize, colunas: usize) -> usize {
    let largura = colunas as i64;
    let (mut linha, mut palavra, mut branco) = (0i64, 0i64, 0i64);
    let (mut fim_da_palavra, mut fim_da_anterior) = (ini, None::<usize>);
    let mut dentro = true;
    let mut s = ini;
    while s < cs.len() {
        let c = cs[s];
        if c == '\n' {
            return s;
        }
        if c == ' ' || c == '\t' || c == '\u{3000}' {
            if dentro {
                linha += branco;
                branco = 0;
                fim_da_palavra = s;
            }
            branco += 1;
            dentro = false;
        } else {
            palavra += 1;
            if dentro {
                fim_da_palavra = s + 1;
            } else {
                fim_da_anterior = Some(fim_da_palavra);
                linha += palavra + branco;
                palavra = 0;
                branco = 0;
            }
            dentro = !matches!(c, '.' | ',' | ';' | '!' | '?' | '"');
        }
        if linha + palavra > largura {
            if palavra < largura {
                s = fim_da_anterior.unwrap_or(fim_da_palavra);
            }
            break;
        }
        s += 1;
    }
    // Nada cabe: mostra 1 caractere, como a referência, para a linha andar.
    s.max(ini + 1).min(cs.len().max(ini))
}

/// O texto quebrado em linhas de até `colunas` caracteres (ver `fim_da_linha`). Os brancos do começo da linha seguinte são pulados e
/// `\n` sempre quebra.
pub(crate) fn linhas_quebradas(texto: &str, colunas: usize) -> Vec<String> {
    let cs: Vec<char> = texto.chars().collect();
    let mut linhas = Vec::new();
    let mut s = 0;
    loop {
        let fim = fim_da_linha(&cs, s, colunas.max(1));
        linhas.push(cs[s..fim].iter().collect::<String>().trim_end().to_string());
        s = fim;
        if s >= cs.len() {
            break;
        }
        if cs[s] == '\n' {
            s += 1;
        } else {
            while s < cs.len() && matches!(cs[s], ' ' | '\t' | '\u{3000}') {
                s += 1;
            }
            if s < cs.len() && cs[s] == '\n' {
                s += 1;
            }
        }
        if s >= cs.len() {
            break;
        }
    }
    linhas
}

/// Área de transferência ausente: copiar não guarda, colar não traz nada.
struct SemArea;

impl AreaDeTransferencia for SemArea {
    fn ler(&mut self) -> Option<String> {
        None
    }
    fn gravar(&mut self, _texto: &str) {}
}

/// Campo sem `eventos` no frame: os `digitados` e depois as `teclas`, sem modificador.
fn eventos_sem_ordem(e: &Entrada) -> Vec<Evento> {
    e.digitados
        .iter()
        .map(|c| Evento::Caractere(*c))
        .chain(e.teclas.iter().map(|k| Evento::Tecla(*k, Mods::default())))
        .collect()
}

/// O que o frame do terminal devolve: a malha e se o jogo deve ou não receber mouse e teclado.
pub(crate) struct Frame {
    pub(crate) malha: Malha,
    /// O mouse está sobre uma janela ou num arraste: o clique é do terminal, não do jogo.
    pub(crate) quer_mouse: bool,
    /// Equivale ao `want_capture_keyboard`: há um campo de texto em foco.
    pub(crate) quer_teclado: bool,
    /// O que a procura de dev da `Entrada` achou, na ordem em que foi desenhado.
    #[cfg(any(test, feature = "devtools"))]
    pub(crate) achados: Vec<ItemAchado>,
}

impl<'a> Tela<'a> {
    pub(crate) fn nova(entrada: Entrada, persistente: &'a mut Persistente, paleta: usize) -> Self {
        persistente.quadro += 1;
        // A tela mudou (escala ou resolução): cada janela volta inteira para dentro dela, se couber.
        let tela = entrada.tamanho_da_tela;
        if tela[0] > 0.0 && tela[1] > 0.0 && persistente.tela != tela {
            persistente.tela = tela;
            for j in &mut persistente.janelas {
                let [x, y] = janela_dentro_da_tela(j.ret, tela);
                j.ret[0] = x;
                j.ret[1] = y;
            }
        }
        let janela_sob_o_mouse = janela_sob_o_mouse(persistente, entrada.mouse);
        #[cfg(any(test, feature = "devtools"))]
        let janelas_no_inicio = entrada.procura.as_ref().map(|_| Persistente {
            janelas: persistente.janelas.clone(),
            ordem: persistente.ordem.clone(),
            quadro: persistente.quadro,
            arraste: persistente.arraste,
            ..Persistente::default()
        });
        Tela {
            malha: Malha::default(),
            entrada,
            persistente,
            paleta: paleta.min(PALETAS.len() - 1),
            escala: 1.0,
            pilha: Vec::new(),
            n_rolaveis: 0,
            n_abas: 0,
            n_campos: 0,
            n_deslizantes: 0,
            n_itens: 0,
            ultimo: [0.0; 4],
            dica: None,
            foco_reivindicado: false,
            mouse_na_janela: false,
            area: None,
            teclado_tratado: false,
            ponteiro: Ponteiro::Seta,
            atual: None,
            janela_sob_o_mouse,
            faixas: Vec::new(),
            #[cfg(any(test, feature = "devtools"))]
            janelas_no_inicio,
            #[cfg(any(test, feature = "devtools"))]
            achados: Vec::new(),
        }
    }

    /// Liga a área de transferência do Cmd+C/X/V.
    pub(crate) fn com_area_de_transferencia(&mut self, area: &'a mut dyn AreaDeTransferencia) {
        self.area = Some(area);
    }

    /// O texto do campo em edição mudou por fora (o ↑/↓ do histórico): o cursor vai para o fim, sem seleção.
    pub(crate) fn cursor_no_fim_do_campo(&mut self) {
        if let Some(ed) = self.persistente.edicao.as_mut() {
            ed.cursor = usize::MAX;
            ed.ancora = None;
        }
    }

    fn cor(&self, i: usize) -> [u8; 4] {
        para_bytes(PALETAS[self.paleta][i])
    }

    fn area(&mut self) -> &mut Area {
        self.pilha.last_mut().expect("widget fora de janela")
    }

    fn cursor(&self) -> [f32; 2] {
        self.pilha.last().map(|a| a.cursor).unwrap_or([0.0, 0.0])
    }

    /// A tecla `codigo` foi apertada neste frame.
    pub(crate) fn tecla(&self, codigo: u16) -> bool {
        self.entrada.teclas.contains(&codigo)
    }

    fn escrever(&mut self, x: f32, y: f32, cor: [u8; 4], t: &str) {
        let e = self.escala;
        self.malha.texto_de_terminal(x, y + DESCIDA_DO_TEXTO * e, e, cor, t);
    }

    fn dentro(&self, r: [f32; 4]) -> bool {
        ponto_em(r, self.entrada.mouse)
    }

    /// Sob o mouse, na janela de cima sob ele e dentro da área visível: um botão rolado pra fora da área ou coberto por outra janela não pode
    /// ser clicado.
    fn sob_o_mouse(&self, r: [f32; 4]) -> bool {
        if !self.dentro(r) {
            return false;
        }
        if self.janela_sob_o_mouse != self.atual.map(|(i, _, _)| i) {
            return false;
        }
        match self.malha.recorte {
            Some(c) => self.dentro(c),
            None => true,
        }
    }

    /// Item colocado agora, para a procura de dev da `Entrada` (`termonde`, `termclique`): casa pelo rótulo visível ou inteiro, ou pelo
    /// ponto dentro de `r`. No build público não faz nada.
    #[inline]
    fn anotar(&mut self, tipo: TipoDeItem, rotulo: &str, r: [f32; 4], estado: Option<bool>) {
        #[cfg(any(test, feature = "devtools"))]
        {
            let Some(alvo) = &self.entrada.procura else { return };
            let vis = rotulo_visivel(rotulo);
            let centro = [(r[0] + r[2]) * 0.5, (r[1] + r[3]) * 0.5];
            let ponto = match alvo {
                Alvo::Rotulo(a) if vis == a || rotulo == a => centro,
                Alvo::Ponto(p) if ponto_em(r, *p) => *p,
                _ => return,
            };
            if self.achados.len() >= TETO_DE_ACHADOS {
                return;
            }
            // A mesma regra do `sob_o_mouse`, com o ponto no lugar do mouse: a janela de cima nele (pelo começo do quadro) e o recorte.
            let janela = self.janelas_no_inicio.as_ref().and_then(|p| janela_sob_o_mouse(p, ponto));
            let visivel = janela == self.atual.map(|(i, _, _)| i) && self.malha.recorte.map_or(true, |c| ponto_em(c, ponto));
            let rotulo = if tipo == TipoDeItem::Texto { rotulo } else { vis };
            self.achados.push(ItemAchado { tipo, rotulo: rotulo.to_string(), ret: r, visivel, estado });
        }
        #[cfg(not(any(test, feature = "devtools")))]
        let _ = (tipo, rotulo, r, estado);
    }

    // ------------------------------------------------------------ formas

    /// Retângulo com cantos arredondados. `so_em_cima` arredonda só os dois de cima (a aba).
    fn retangulo_arredondado(&mut self, r: [f32; 4], raio: f32, cor: [u8; 4], so_em_cima: bool) {
        let [x0, y0, x1, y1] = r;
        let raio = raio.min((x1 - x0) * 0.5).min((y1 - y0) * 0.5).max(0.0);
        if raio <= 0.5 {
            self.malha.retangulo(x0, y0, x1, y1, cor);
            return;
        }
        let y_base = if so_em_cima { y1 } else { y1 - raio };
        self.malha.retangulo(x0 + raio, y0, x1 - raio, y1, cor);
        self.malha.retangulo(x0, y0 + raio, x0 + raio, y_base, cor);
        self.malha.retangulo(x1 - raio, y0 + raio, x1, y_base, cor);
        // Canto por fatias horizontais: em raio de 2–4 px o degrau é sub-pixel e sai mais limpo que
        // um leque de triângulos.
        let passos = (raio.ceil() as usize).max(1);
        let h = raio / passos as f32;
        for i in 0..passos {
            let meio = (i as f32 + 0.5) * h;
            let dx = raio - (raio * raio - (raio - meio) * (raio - meio)).max(0.0).sqrt();
            let yc = y0 + i as f32 * h;
            self.malha.retangulo(x0 + dx, yc, x0 + raio, yc + h, cor);
            self.malha.retangulo(x1 - raio, yc, x1 - dx, yc + h, cor);
            if !so_em_cima {
                let yb = y1 - (i as f32 + 1.0) * h;
                self.malha.retangulo(x0 + dx, yb, x0 + raio, yb + h, cor);
                self.malha.retangulo(x1 - raio, yb, x1 - dx, yb + h, cor);
            }
        }
    }

    fn contorno(&mut self, r: [f32; 4], cor: [u8; 4]) {
        let [x0, y0, x1, y1] = r;
        self.malha.retangulo(x0, y0, x1, y0 + 1.0, cor);
        self.malha.retangulo(x0, y1 - 1.0, x1, y1, cor);
        self.malha.retangulo(x0, y0, x0 + 1.0, y1, cor);
        self.malha.retangulo(x1 - 1.0, y0, x1, y1, cor);
    }

    /// Traço grosso entre dois pontos, como dois triângulos — é o ✓ do checkbox.
    fn traco(&mut self, p: [f32; 2], q: [f32; 2], espessura: f32, cor: [u8; 4]) {
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let comp = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (nx, ny) = (-dy / comp * espessura * 0.5, dx / comp * espessura * 0.5);
        let a = [p[0] + nx, p[1] + ny];
        let b = [p[0] - nx, p[1] - ny];
        let c = [q[0] - nx, q[1] - ny];
        let d = [q[0] + nx, q[1] + ny];
        self.malha.triangulo(a, b, c, cor);
        self.malha.triangulo(a, c, d, cor);
    }

    // ------------------------------------------------------------ layout

    /// Registra um item de `l` × `h` no cursor e desce pra próxima linha.
    fn colocou(&mut self, l: f32, h: f32) {
        self.colocou_com_base(l, h, 0.0);
    }

    /// `colocou` de item com quadro: `base` é o respiro vertical em que o texto dele fica, e vira a linha de base da linha.
    fn colocou_com_base(&mut self, l: f32, h: f32, base: f32) {
        let a = self.area();
        if !a.na_mesma_linha {
            a.linha_y = a.cursor[1];
            a.altura_da_linha = 0.0;
            a.base_do_texto = 0.0;
        }
        a.base_do_texto = a.base_do_texto.max(base);
        a.na_mesma_linha = false;
        let x = a.cursor[0];
        a.altura_da_linha = a.altura_da_linha.max(h);
        let (linha_y, altura) = (a.linha_y, a.altura_da_linha);
        a.cursor = [a.origem[0], linha_y + altura + ESPACO_ENTRE_ITENS[1]];
        a.alcance = [a.alcance[0].max(x + l), a.alcance[1].max(linha_y + h)];
        self.ultimo = [x, linha_y, x + l, linha_y + h];
    }

    /// O próximo item fica à direita do anterior, na mesma linha.
    pub(crate) fn mesma_linha(&mut self) {
        let ultimo = self.ultimo;
        let a = self.area();
        a.na_mesma_linha = true;
        a.cursor = [ultimo[2] + ESPACO_ENTRE_ITENS[0], a.linha_y];
    }

    pub(crate) fn espaco(&mut self) {
        let a = self.area();
        a.cursor[1] += ESPACO_ENTRE_ITENS[1];
    }

    /// Linha de lado a lado. Na referência ela não ocupa altura no layout (só o espaço entre itens abaixo)
    /// e é recortada a meio respiro da borda — com 1 px de altura aqui, tudo abaixo de cada separador
    /// desceria 1 px a mais que nela.
    pub(crate) fn separador(&mut self) {
        let (x0, x1, y) = {
            let a = self.area();
            (a.origem[0] - RESPIRO_JANELA[0] * 0.5, a.limite[0] + RESPIRO_JANELA[0] * 0.5, a.cursor[1])
        };
        let cor = para_bytes(herdada::SEPARADOR);
        self.malha.retangulo(x0, y, x1, y + 1.0, cor);
        self.colocou(0.0, 0.0);
    }

    /// Largura disponível até a borda da área, menos `margem` (o `set_next_item_width(-N)`).
    fn largura_ate_a_borda(&self, margem: f32) -> f32 {
        let a = self.pilha.last().expect("widget fora de janela");
        (a.limite[0] - a.cursor[0] - margem).max(1.0)
    }

    // ------------------------------------------------------------ texto

    pub(crate) fn texto(&mut self, t: &str) {
        let c = self.cor(TEXTO);
        self.texto_colorido(c, t);
    }

    pub(crate) fn texto_apagado(&mut self, t: &str) {
        let c = para_bytes(herdada::TEXTO_APAGADO);
        self.texto_colorido(c, t);
    }

    pub(crate) fn texto_colorido(&mut self, cor: [u8; 4], t: &str) {
        let [x, y] = self.cursor();
        let base = self.pilha.last().filter(|a| a.na_mesma_linha).map_or(0.0, |a| a.base_do_texto);
        self.escrever(x, y + base, cor, t);
        // Um item só, com as linhas coladas (13 px), como o `TextUnformatted`: o texto de plugin pode trazer `\n`.
        let (colunas, linhas) = colunas_e_linhas(t);
        self.colocou(colunas as f32 * AVANCO * self.escala, linhas as f32 * LINHA * self.escala + base);
        self.anotar(TipoDeItem::Texto, t, self.ultimo, None);
    }

    /// Texto que quebra na largura da área. Monoespaçada: a quebra é conta de colunas.
    pub(crate) fn texto_quebrado(&mut self, t: &str) {
        let c = self.cor(TEXTO);
        self.texto_quebrado_colorido(c, t);
    }

    pub(crate) fn texto_quebrado_colorido(&mut self, c: [u8; 4], t: &str) {
        let colunas = ((self.largura_ate_a_borda(0.0) / (AVANCO * self.escala)).floor() as usize).max(1);
        let linhas = linhas_quebradas(t, colunas);
        let [x, y] = self.cursor();
        let mut maior = 0usize;
        for (i, linha) in linhas.iter().enumerate() {
            self.escrever(x, y + i as f32 * LINHA * self.escala, c, linha);
            maior = maior.max(linha.chars().count());
        }
        // Um item só: as linhas da quebra ficam coladas (13 px), e o espaço entre itens vem depois do texto inteiro.
        self.colocou(maior as f32 * AVANCO * self.escala, linhas.len() as f32 * LINHA * self.escala);
        self.anotar(TipoDeItem::Texto, t, self.ultimo, None);
    }

    // ------------------------------------------------------------ widgets

    /// Botão. Devolve `true` no frame em que foi clicado.
    pub(crate) fn botao(&mut self, rotulo: &str) -> bool {
        self.botao_marcado(rotulo, false)
    }

    /// Botão que pode ficar marcado (o tema escolhido): marcado usa a cor da aba ativa.
    pub(crate) fn botao_marcado(&mut self, rotulo: &str, marcado: bool) -> bool {
        let vis = rotulo_visivel(rotulo);
        let l = vis.chars().count() as f32 * AVANCO * self.escala + RESPIRO_CAMPO[0] * 2.0;
        let h = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let [x0, y0] = self.cursor();
        let r = [x0, y0, x0 + l, y0 + h];
        let (sobre, seguro, disparou) = self.clique(r);
        self.anotar(TipoDeItem::Botao, rotulo, r, Some(marcado));
        let cor = self.cor(if seguro {
            BOTAO_ATIVO
        } else if marcado {
            TITULO_ATIVO
        } else if sobre {
            BOTAO_SOBRE
        } else {
            BOTAO
        });
        self.retangulo_arredondado(r, ARREDONDAMENTO_CAMPO, cor, false);
        let ct = self.cor(TEXTO);
        self.escrever(x0 + RESPIRO_CAMPO[0], y0 + RESPIRO_CAMPO[1], ct, vis);
        self.colocou_com_base(l, h, RESPIRO_CAMPO[1]);
        disparou
    }

    /// Clique da referência: o aperto em cima torna o item ativo, e ele só dispara ao soltar em cima se foi ele que
    /// recebeu o aperto — soltar um arraste em cima de um botão não o aciona. Com outro item ativo ou um arraste em curso, nenhum outro
    /// acende de "sobre". Devolve (sobre, seguro em cima, disparou).
    fn clique(&mut self, r: [f32; 4]) -> (bool, bool, bool) {
        let id = self.n_itens;
        self.n_itens += 1;
        let sob = self.sob_o_mouse(r);
        if sob && self.entrada.apertou {
            self.persistente.item_apertado = Some(id);
        }
        let ativo = self.persistente.item_apertado == Some(id);
        let livre = self.persistente.item_apertado.is_none() && self.persistente.arraste == Arraste::Nenhum;
        (sob && (ativo || livre), sob && ativo && self.entrada.segurando, sob && ativo && self.entrada.soltou)
    }

    /// Dica do item anterior, mostrada quando o mouse está sobre ele.
    pub(crate) fn dica(&mut self, t: &str) {
        if self.sob_o_mouse(self.ultimo) {
            self.dica = Some(t.to_string());
        }
    }

    /// Caixa de marcar. Devolve `true` se mudou neste frame, já tendo alterado `valor`.
    pub(crate) fn caixa_de_marcar(&mut self, rotulo: &str, valor: &mut bool) -> bool {
        let vis = rotulo_visivel(rotulo);
        let lado = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let [x0, y0] = self.cursor();
        let caixa = [x0, y0, x0 + lado, y0 + lado];
        let l_rotulo = vis.chars().count() as f32 * AVANCO * self.escala;
        let total = lado + if vis.is_empty() { 0.0 } else { ESPACO_INTERNO[0] + l_rotulo };
        let (sobre, seguro, disparou) = self.clique([x0, y0, x0 + total, y0 + lado]);
        self.anotar(TipoDeItem::Caixa, rotulo, [x0, y0, x0 + total, y0 + lado], Some(*valor));
        let fundo = if seguro {
            para_bytes(herdada::CAMPO_ATIVO)
        } else if sobre {
            para_bytes(herdada::CAMPO_SOBRE)
        } else {
            self.cor(FUNDO_CAMPO)
        };
        self.retangulo_arredondado(caixa, ARREDONDAMENTO_CAMPO, fundo, false);
        if *valor {
            // ✓ com o traço proporcional à caixa: folga de 1/6, espessura de 1/5 do miolo.
            let folga = (lado / 6.0).floor().max(1.0);
            let miolo = lado - folga * 2.0;
            let esp = (miolo / 5.0).max(1.0);
            let terco = miolo / 3.0;
            let (bx, by) = (x0 + folga + terco, y0 + folga + miolo - terco * 0.5);
            let marca = para_bytes(herdada::MARCA_DO_CHECK);
            self.traco([bx - terco, by - terco], [bx, by], esp, marca);
            self.traco([bx, by], [bx + terco * 2.0, by - terco * 2.0], esp, marca);
        }
        if !vis.is_empty() {
            let ct = self.cor(TEXTO);
            self.escrever(x0 + lado + ESPACO_INTERNO[0], y0 + RESPIRO_CAMPO[1], ct, vis);
        }
        self.colocou_com_base(total, lado, RESPIRO_CAMPO[1]);
        if disparou {
            *valor = !*valor;
        }
        disparou
    }

    /// Deslizante de inteiro, com o valor no meio e o rótulo à direita. `largura <= 0` usa a largura
    /// padrão da referência (65% da área). Devolve `true` se o valor mudou neste frame.
    pub(crate) fn deslizante(&mut self, rotulo: &str, valor: &mut i32, min: i32, max: i32, largura: f32) -> bool {
        let faixa = (max - min).max(1) as f32;
        let posicao = ((*valor - min) as f32 / faixa).clamp(0.0, 1.0);
        match self.deslizante_em(rotulo, Some(faixa), posicao, valor.to_string(), &|t| (min + (t * faixa).round() as i32).to_string(), largura) {
            Some(t) => {
                let novo = min + (t * faixa).round() as i32;
                let mudou = novo != *valor;
                *valor = novo;
                mudou
            }
            None => false,
        }
    }

    /// Deslizante de número real (o `SliderFloat` dos mods CET), com o valor em 3 casas no meio, como o `%.3f` padrão da referência.
    #[cfg(feature = "lua")]
    pub(crate) fn deslizante_real(&mut self, rotulo: &str, valor: &mut f32, min: f32, max: f32, largura: f32) -> bool {
        let faixa = (max - min).abs().max(f32::EPSILON);
        let posicao = ((*valor - min) / faixa).clamp(0.0, 1.0);
        match self.deslizante_em(rotulo, None, posicao, format!("{:.3}", *valor), &|t| format!("{:.3}", min + t * faixa), largura) {
            Some(t) => {
                let novo = min + t * faixa;
                let mudou = novo != *valor;
                *valor = novo;
                mudou
            }
            None => false,
        }
    }

    /// Corpo dos deslizantes: o punho em `posicao` (0..1) com o número `atual` no meio, ou sob o arraste na posição nova com o `texto` dela;
    /// devolve a posição nova. `faixa_inteira` dá ao punho a largura de 1 unidade e anda de unidade em unidade; sem ela (real), o mínimo do estilo e contínuo.
    fn deslizante_em(&mut self, rotulo: &str, faixa_inteira: Option<f32>, posicao: f32, atual: String, texto: &dyn Fn(f32) -> String, largura: f32) -> Option<f32> {
        let indice = self.n_deslizantes;
        self.n_deslizantes += 1;
        let vis = rotulo_visivel(rotulo);
        // Largura padrão da referência: 65% da janela inteira, não da área útil. Dentro de área rolável, 65% dela.
        let base = match self.pilha.last() {
            Some(a) if a.rolavel.is_some() => a.limite[0] - a.origem[0],
            _ => self.atual.map_or(JANELA_INICIAL[2], |(i, _, _)| self.persistente.janelas[i].ret[2]),
        };
        let largura = if largura > 0.0 { largura } else { (base * 0.65).floor() };
        let h = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let [x0, y0] = self.cursor();
        let r = [x0, y0, x0 + largura, y0 + h];
        let sobre = self.sob_o_mouse(r);
        self.anotar(TipoDeItem::Deslizante, rotulo, r, None);
        if sobre && self.entrada.apertou {
            self.persistente.arraste = Arraste::Deslizante { indice };
        }
        let arrastando = self.persistente.arraste == Arraste::Deslizante { indice };
        // Punho do `SliderBehaviorT`: 2 px de folga de cada lado e, em inteiro, a largura de 1 unidade (`trilho / (faixa + 1)`), nunca
        // menos que o mínimo do estilo.
        let trilho = largura - 4.0;
        let punho = faixa_inteira.map_or(PUNHO_MINIMO, |faixa| trilho / (faixa + 1.0)).max(PUNHO_MINIMO).min(trilho);
        let util = trilho - punho;
        let novo = (arrastando && self.entrada.segurando).then(|| {
            let t = ((self.entrada.mouse[0] - x0 - 2.0 - punho * 0.5) / util.max(1.0)).clamp(0.0, 1.0);
            faixa_inteira.map_or(t, |faixa| (t * faixa).round() / faixa)
        });
        let posicao = novo.unwrap_or(posicao);
        let fundo = if arrastando {
            para_bytes(herdada::CAMPO_ATIVO)
        } else if sobre {
            para_bytes(herdada::CAMPO_SOBRE)
        } else {
            self.cor(FUNDO_CAMPO)
        };
        self.retangulo_arredondado(r, ARREDONDAMENTO_CAMPO, fundo, false);
        let px = x0 + 2.0 + util * posicao;
        let cor_punho = para_bytes(if arrastando { herdada::PUNHO_DESLIZANTE_ATIVO } else { herdada::PUNHO_DESLIZANTE });
        self.retangulo_arredondado([px, y0 + 2.0, px + punho, y0 + h - 2.0], 0.0, cor_punho, false);
        let numero = novo.map_or(atual, texto);
        let ln = numero.chars().count() as f32 * AVANCO * self.escala;
        let ct = self.cor(TEXTO);
        self.escrever(x0 + (largura - ln) * 0.5, y0 + RESPIRO_CAMPO[1], ct, &numero);
        let mut total = largura;
        if !vis.is_empty() {
            self.escrever(x0 + largura + ESPACO_INTERNO[0], y0 + RESPIRO_CAMPO[1], ct, vis);
            total += ESPACO_INTERNO[0] + vis.chars().count() as f32 * AVANCO * self.escala;
        }
        self.colocou_com_base(total, h, RESPIRO_CAMPO[1]);
        novo
    }

    /// Campo de texto de uma linha, com dica apagada quando vazio. `largura <= 0` preenche até a borda
    /// menos `-largura`. Devolve (Enter apertado, campo em foco).
    pub(crate) fn campo_de_texto(&mut self, buf: &mut String, largura: f32, dica: &str) -> (bool, bool) {
        let id = self.n_campos;
        self.n_campos += 1;
        let largura = if largura > 0.0 { largura } else { self.largura_ate_a_borda(-largura) };
        let h = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let [x0, y0] = self.cursor();
        let confirmou = self.campo_editavel(id, buf, [x0, y0, x0 + largura, y0 + h], dica, |_| true);
        self.colocou_com_base(largura, h, RESPIRO_CAMPO[1]);
        (confirmou, self.persistente.foco == Some(id))
    }

    /// Miolo do campo editável `id` no retângulo `r`: foco e edição pelo mouse, teclado, rolagem horizontal e desenho. `aceita`
    /// diz o que pode ficar no texto (o numérico só aceita dígito e sinal). Devolve se o Enter confirmou.
    fn campo_editavel(&mut self, id: usize, buf: &mut String, r: [f32; 4], dica: &str, aceita: fn(char) -> bool) -> bool {
        let [x0, y0, x1, y1] = r;
        let (largura, h) = (x1 - x0, y1 - y0);
        let sobre = self.sob_o_mouse(r);
        let (tx, ty) = (x0 + RESPIRO_CAMPO[0], y0 + RESPIRO_CAMPO[1]);
        // I sobre o campo, como na referência; arrastando outra coisa, o ponteiro não muda.
        if sobre && matches!(self.persistente.arraste, Arraste::Nenhum | Arraste::Texto { .. }) {
            self.ponteiro = Ponteiro::Texto;
        }
        let passo = AVANCO * self.escala;
        let util = largura - RESPIRO_CAMPO[0] * 2.0;
        let deste = self.persistente.foco == Some(id);
        let rolagem = if deste && self.persistente.edicao.is_some() { self.persistente.rolagem_do_campo } else { 0.0 };
        let total = buf.chars().count();
        let caractere_sob = |mx: f32| (((mx - tx + rolagem) / passo).round().max(0.0) as usize).min(total);

        // Mouse: o aperto dá foco e edição e põe o cursor sob o mouse (clique duplo pega a palavra); segurar e arrastar seleciona.
        if self.entrada.apertou && sobre {
            let em = caractere_sob(self.entrada.mouse[0]);
            let mut ed = match self.persistente.edicao.take() {
                Some(e) if deste => e,
                _ => {
                    self.persistente.rolagem_do_campo = 0.0;
                    Edicao::comecar(buf)
                }
            };
            if self.entrada.clique_duplo {
                ed.selecionar_palavra(buf, em);
            } else {
                ed.cursor = em;
                ed.ancora = Some(em);
            }
            self.persistente.foco = Some(id);
            self.persistente.edicao = Some(ed);
            self.persistente.arraste = Arraste::Texto { indice: id };
            self.persistente.piscar = -0.3;
            self.foco_reivindicado = true;
        } else if self.entrada.segurando && self.persistente.arraste == (Arraste::Texto { indice: id }) {
            let em = caractere_sob(self.entrada.mouse[0]);
            if let Some(ed) = self.persistente.edicao.as_mut() {
                ed.cursor = em;
            }
        }
        if self.persistente.foco == Some(id) && self.persistente.comecar_edicao {
            self.persistente.edicao = Some(Edicao::comecar(buf));
            self.persistente.comecar_edicao = false;
            self.persistente.rolagem_do_campo = 0.0;
        }

        // Teclado: só com edição em curso neste campo.
        let mut confirmou = false;
        if self.persistente.foco == Some(id) && !self.teclado_tratado {
            if let Some(mut ed) = self.persistente.edicao.take() {
                self.teclado_tratado = true;
                let eventos = if self.entrada.eventos.is_empty() { eventos_sem_ordem(&self.entrada) } else { self.entrada.eventos.clone() };
                let mut sem_area = SemArea;
                let area: &mut dyn AreaDeTransferencia = match self.area.as_deref_mut() {
                    Some(a) => a,
                    None => &mut sem_area,
                };
                let mut continua = true;
                for ev in &eventos {
                    if let Evento::Caractere(c) = ev {
                        if !aceita(*c) {
                            continue;
                        }
                    }
                    let pedido = ed.aplicar(buf, ev, area);
                    // O colado também passa pelo filtro do campo.
                    if buf.chars().any(|c| !aceita(c)) {
                        buf.retain(aceita);
                        ed.cursor = ed.cursor.min(buf.chars().count());
                        ed.ancora = None;
                    }
                    match pedido {
                        Pedido::Nada => self.persistente.piscar = -0.3,
                        Pedido::Confirmou => {
                            confirmou = true;
                            continua = false;
                            break;
                        }
                        Pedido::Cancelou => {
                            continua = false;
                            break;
                        }
                        Pedido::Proximo => {
                            self.persistente.foco = Some(id + 1);
                            self.persistente.comecar_edicao = true;
                            continua = false;
                            break;
                        }
                    }
                }
                if continua {
                    // O cursor fica visível: a rolagem anda 1/4 da largura quando ele passa da borda (a regra do campo da referência).
                    let total = buf.chars().count();
                    ed.cursor = ed.cursor.min(total);
                    let cx = ed.cursor as f32 * passo;
                    let mut s = self.persistente.rolagem_do_campo;
                    if cx < s {
                        s = (cx - util * 0.25).max(0.0).floor();
                    } else if cx - util >= s {
                        s = (cx - util + util * 0.25).floor();
                    }
                    self.persistente.rolagem_do_campo = s;
                    self.persistente.piscar += self.entrada.dt;
                    self.persistente.edicao = Some(ed);
                }
            }
        }
        let editando = self.persistente.foco == Some(id) && self.persistente.edicao.is_some();

        // A referência desenha o quadro do campo sempre com o fundo de campo: sem cor de sobre nem de edição (só checkbox e deslizante
        // mudam).
        let fundo = self.cor(FUNDO_CAMPO);
        self.retangulo_arredondado(r, ARREDONDAMENTO_CAMPO, fundo, false);
        let recorte_antes = self.malha.recorte;
        self.malha.recorte = Some(interseccao(recorte_antes, [x0, y0, x0 + largura - RESPIRO_CAMPO[0], y0 + h]));
        let rolagem = if editando { self.persistente.rolagem_do_campo } else { 0.0 };
        let total = buf.chars().count();
        if editando {
            if let Some((a, b)) = self.persistente.edicao.as_ref().and_then(Edicao::selecao) {
                let (a, b) = (a.min(total), b.min(total));
                let c = para_bytes(herdada::TEXTO_SELECIONADO);
                let xa = tx - rolagem + a as f32 * passo;
                self.malha.retangulo(xa, ty, xa + (b - a) as f32 * passo, ty + LINHA * self.escala, c);
            }
        }
        if buf.is_empty() {
            let c = para_bytes(herdada::TEXTO_APAGADO);
            self.escrever(tx, ty, c, dica);
        } else {
            let c = self.cor(TEXTO);
            self.escrever(tx - rolagem, ty, c, buf);
        }
        // Cursor aceso 0,8 s a cada 1,2 s, e sempre aceso logo depois de mexer (o ciclo da referência).
        let piscar = self.persistente.piscar;
        if editando && (piscar <= 0.0 || piscar % 1.2 <= 0.8) {
            let cursor = self.persistente.edicao.as_ref().map_or(0, |e| e.cursor.min(total));
            let cx = tx - rolagem + cursor as f32 * passo;
            let c = self.cor(TEXTO);
            self.malha.retangulo(cx, ty, cx + 1.0, ty + LINHA * self.escala, c);
        }
        self.malha.recorte = recorte_antes;
        confirmou
    }

    /// Campo numérico com `−` e `+` à direita e o rótulo depois. `largura` é
    /// o total do campo com os dois botões. Devolve `true` se o valor mudou.
    pub(crate) fn campo_numerico(&mut self, rotulo: &str, valor: &mut i32, largura: f32) -> bool {
        let id = self.n_campos;
        self.n_campos += 1;
        let vis = rotulo_visivel(rotulo);
        let h = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let lado = h;
        let l_campo = (largura - (lado + ESPACO_INTERNO[0]) * 2.0).max(1.0);
        let [x0, y0] = self.cursor();
        let campo = [x0, y0, x0 + l_campo, y0 + h];
        // O número se digita como no terminal de hoje: o texto em edição vale a cada mudança que vira número.
        let editando = self.persistente.foco == Some(id) && self.persistente.edicao.is_some();
        let mut texto = if editando { self.persistente.texto_do_numero.clone() } else { valor.to_string() };
        self.campo_editavel(id, &mut texto, campo, "", |c| c.is_ascii_digit() || c == '-' || c == '+');
        if self.persistente.foco == Some(id) && self.persistente.edicao.is_some() {
            self.persistente.texto_do_numero = texto.clone();
        }
        let mut mudou = false;
        if let Ok(n) = texto.trim().parse::<i32>() {
            if n != *valor {
                *valor = n;
                mudou = true;
            }
        }
        let ct = self.cor(TEXTO);
        for (i, sinal) in ["-", "+"].iter().enumerate() {
            let bx = x0 + l_campo + ESPACO_INTERNO[0] + i as f32 * (lado + ESPACO_INTERNO[0]);
            let b = [bx, y0, bx + lado, y0 + h];
            let (sobre, seguro, _) = self.clique(b);
            let cor = self.cor(if seguro {
                BOTAO_ATIVO
            } else if sobre {
                BOTAO_SOBRE
            } else {
                BOTAO
            });
            self.retangulo_arredondado(b, ARREDONDAMENTO_CAMPO, cor, false);
            self.escrever(bx + (lado - AVANCO * self.escala) * 0.5, y0 + RESPIRO_CAMPO[1], ct, sinal);
            // Dispara no aperto e repete enquanto seguro em cima: 0,275 s de espera e depois a cada 0,05 s (o botão de repetição da referência).
            let botao = id * 2 + i;
            let passo = if i == 0 { -1 } else { 1 };
            if sobre && self.entrada.apertou {
                *valor += passo;
                mudou = true;
                self.persistente.repete = Some((botao, -0.275));
            } else if let Some((b_seguro, relogio)) = self.persistente.repete {
                if b_seguro == botao && seguro {
                    let mut relogio = relogio + self.entrada.dt;
                    while relogio >= 0.0 {
                        *valor += passo;
                        mudou = true;
                        relogio -= 0.05;
                    }
                    self.persistente.repete = Some((botao, relogio));
                }
            }
        }
        if mudou && self.persistente.foco == Some(id) && self.persistente.edicao.is_some() {
            self.persistente.texto_do_numero = valor.to_string();
        }
        let mut total = largura;
        if !vis.is_empty() {
            self.escrever(x0 + largura + ESPACO_INTERNO[0], y0 + RESPIRO_CAMPO[1], ct, vis);
            total += ESPACO_INTERNO[0] + vis.chars().count() as f32 * AVANCO * self.escala;
        }
        self.colocou_com_base(total, h, RESPIRO_CAMPO[1]);
        mudou
    }

    /// Barra de abas. Devolve o índice escolhido. Rótulos com `##` mostram só a parte de antes.
    pub(crate) fn abas(&mut self, rotulos: &[&str]) -> usize {
        let indice = self.n_abas.min(self.persistente.aba_ativa.len() - 1);
        self.n_abas += 1;
        let h = LINHA * self.escala + RESPIRO_CAMPO[1] * 2.0;
        let [x_ini, y0] = self.cursor();
        // A linha da barra vai só até meio respiro da borda, como no terminal de hoje.
        let (x_esq, x_dir) = {
            let a = self.area();
            (a.origem[0] - RESPIRO_JANELA[0] * 0.5, a.limite[0] + RESPIRO_JANELA[0] * 0.5)
        };
        let ativa = self.persistente.aba_ativa[indice].min(rotulos.len().saturating_sub(1));
        let mut escolhida = ativa;
        let mut x = x_ini;
        for (i, r) in rotulos.iter().enumerate() {
            let vis = rotulo_visivel(r);
            // +1: a aba da referência mede texto + 2 × respiro + 1 (medido: "Console" = 58 px, não 57).
            let l = vis.chars().count() as f32 * AVANCO * self.escala + RESPIRO_CAMPO[0] * 2.0 + 1.0;
            let caixa = [x, y0, x + l, y0 + h];
            let (sobre, _, _) = self.clique(caixa);
            self.anotar(TipoDeItem::Aba, r, caixa, Some(i == ativa));
            let cor = self.cor(if i == ativa {
                TITULO_ATIVO
            } else if sobre {
                BOTAO_SOBRE
            } else {
                BOTAO
            });
            // O fundo da aba é recuado 1 px em cima e 1 px embaixo da caixa dela.
            self.retangulo_arredondado([x, y0 + 1.0, x + l, y0 + h - 1.0], ARREDONDAMENTO_ABA, cor, true);
            let ct = self.cor(TEXTO);
            self.escrever(x + RESPIRO_CAMPO[0], y0 + RESPIRO_CAMPO[1], ct, vis);
            // A aba troca no aperto, como na referência.
            if sobre && self.entrada.apertou {
                escolhida = i;
            }
            x += l + ESPACO_INTERNO[0];
        }
        let cl = self.cor(TITULO_ATIVO);
        self.malha.retangulo(x_esq, y0 + h - 1.0, x_dir, y0 + h, cl);
        // A rolagem é guardada pela ordem da área no quadro, não pela aba: sem zerar, a aba nova herda a da anterior (a lista da aba
        // Mods abria rolada até onde o log do Console estava, com o topo fora de vista; boot de 2026-10-24 20:38, lane M 3.4).
        if escolhida != ativa {
            self.persistente.rolagem = [0.0; 8];
        }
        self.persistente.aba_ativa[indice] = escolhida;
        self.colocou(x - x_ini, h);
        escolhida
    }

    // ------------------------------------------------------------ janela

    /// Abre a janela do terminal (a 0): fundo, borda, barra de título com o ▼ de recolher e a alça de redimensionar no canto. Arrastar pela
    /// barra move; arrastar a alça redimensiona. Devolve `false` se a janela está recolhida: aí o conteúdo não deve ser desenhado.
    pub(crate) fn abrir_janela(&mut self, titulo: &str) -> bool {
        self.abrir(0, titulo)
    }

    /// Abre a janela de um plugin, achada pela identidade do título (`identidade_da_janela`); a barra mostra só o que vem antes do `##`.
    /// A primeira vez ela nasce em (60, 60), na frente e com o foco, e o tamanho segue o conteúdo nos 2 primeiros quadros. Fecha a janela
    /// que estiver aberta. Devolve `false` se ela está recolhida, como o `igBegin`.
    pub(crate) fn abrir_janela_de_plugin(&mut self, titulo: &str) -> bool {
        let id = identidade_da_janela(titulo);
        let i = match self.persistente.janelas.iter().skip(1).position(|j| identidade_da_janela(&j.titulo) == id) {
            Some(k) => k + 1,
            None => {
                // Tamanho do 1º quadro, que não aparece: o ajuste de um conteúdo vazio (respiro e barra de título, no mínimo do estilo).
                let l = (RESPIRO_JANELA[0] * 2.0).max(JANELA_DE_PLUGIN_MINIMA[0]);
                let a = (RESPIRO_JANELA[1] * 2.0 + ALTURA_TITULO).max(JANELA_DE_PLUGIN_MINIMA[1]);
                self.persistente.janelas.push(Janela {
                    titulo: titulo.to_string(),
                    ret: [JANELA_DE_PLUGIN[0], JANELA_DE_PLUGIN[1], l, a],
                    recolhida: false,
                    fase: FaseDaJanela::Nova,
                    ultimo_quadro: 0,
                });
                let i = self.persistente.janelas.len() - 1;
                self.para_a_frente(i);
                i
            }
        };
        self.abrir(i, titulo)
    }

    /// Uma janela de plugin está aberta e não recolhida: o texto dela tem onde cair.
    pub(crate) fn janela_de_plugin_aberta(&self) -> bool {
        matches!(self.atual, Some((i, _, _)) if i != 0) && !self.pilha.is_empty()
    }

    /// Fecha a janela de plugin aberta; sem uma aberta, não faz nada (o plugin que fecha sem abrir não derruba o jogo).
    pub(crate) fn fechar_janela_de_plugin(&mut self) {
        if matches!(self.atual, Some((i, _, _)) if i != 0) {
            self.fechar_janela();
        }
    }

    /// A janela `i` passa a ser a da frente e a do foco.
    fn para_a_frente(&mut self, i: usize) {
        self.persistente.ordem.retain(|&j| j != i);
        self.persistente.ordem.push(i);
        self.persistente.janela_em_foco = Some(i);
    }

    /// O mouse está em `r` e a janela aberta é a de cima sob ele.
    fn dentro_desta(&self, r: [f32; 4]) -> bool {
        self.janela_sob_o_mouse == self.atual.map(|(i, _, _)| i) && self.dentro(r)
    }

    fn abrir(&mut self, i: usize, titulo: &str) -> bool {
        self.fechar_janela();
        let quadro = self.persistente.quadro;
        let j = &mut self.persistente.janelas[i];
        // Volta a abrir depois de um quadro sem abrir: ganha o foco e vai para a frente, como na referência. A nova já nasceu na frente.
        let voltou = j.fase != FaseDaJanela::Nova && !j.abriu_antes_de(quadro);
        j.ultimo_quadro = quadro;
        j.fase = j.fase.ao_abrir();
        let aparece = j.fase.aparece();
        if voltou {
            self.para_a_frente(i);
        }
        self.atual = Some((i, self.malha.indices.len(), self.malha.vertices.len()));
        self.tratar_arraste_da_janela(i);
        let recolhida = self.persistente.janelas[i].recolhida;
        // Presa à tela como na referência: pelo menos `JANELA_VISIVEL` dela fica dentro, arrastada para fora ou com a
        // tela encolhida pela escala. Recolhida, conta só a barra de título.
        let tela = self.entrada.tamanho_da_tela;
        if tela[0] > 0.0 && tela[1] > 0.0 {
            let j = &mut self.persistente.janelas[i];
            let [x, y, l, a] = j.ret;
            let a = if recolhida { ALTURA_TITULO } else { a };
            j.ret[0] = x.min(tela[0] - JANELA_VISIVEL).max(JANELA_VISIVEL - l).floor();
            j.ret[1] = y.min(tela[1] - JANELA_VISIVEL).max(JANELA_VISIVEL - a).floor();
        }
        let [x, y, l, a] = self.persistente.janelas[i].ret;
        let altura = if recolhida { ALTURA_TITULO } else { a };
        let (x1, y1) = (x + l, y + altura);
        if aparece {
            self.mouse_na_janela |= self.dentro([x, y, x1, y1]);
        }
        // O aperto na janela de cima a traz para a frente com o foco.
        if self.entrada.apertou && self.janela_sob_o_mouse == Some(i) {
            self.para_a_frente(i);
        }

        let titulo_r = [x, y, x1, y + ALTURA_TITULO];
        let seta = [x + RESPIRO_CAMPO[0], y, x + RESPIRO_CAMPO[0] + LINHA, y + ALTURA_TITULO];
        // A seta é um botão (dispara ao soltar se o aperto foi nela); clique duplo no resto da barra também recolhe, como na referência.
        let (_, _, seta_disparou) = self.clique(seta);
        let clique_duplo_no_titulo = self.entrada.apertou && self.entrada.clique_duplo && self.dentro_desta(titulo_r) && !self.dentro(seta);
        if seta_disparou || clique_duplo_no_titulo {
            let j = &mut self.persistente.janelas[i];
            j.recolhida = !j.recolhida;
        }

        if !recolhida {
            let fundo = self.cor(FUNDO_JANELA);
            self.retangulo_arredondado([x, y, x1, y1], ARREDONDAMENTO_JANELA, fundo, false);
        }
        let cor_titulo = if recolhida {
            para_bytes(herdada::TITULO_RECOLHIDO)
        } else if self.persistente.janela_em_foco != Some(i) {
            self.cor(TITULO)
        } else {
            self.cor(TITULO_ATIVO)
        };
        self.retangulo_arredondado(titulo_r, ARREDONDAMENTO_JANELA, cor_titulo, recolhida == false);
        let borda = self.cor(BORDA);
        self.contorno([x, y, x1, y1], borda);

        // ▼ aberta, ▶ recolhida. Raio de 40% da linha, centrado na caixa de uma linha deslocada pelo
        // respiro do campo; cantos em ±0,866r e 0,75r — o triângulo equilátero do terminal de hoje.
        let ct = self.cor(TEXTO);
        let r = LINHA * 0.40;
        let (cx, cy) = (x + RESPIRO_CAMPO[0] + LINHA * 0.5, y + RESPIRO_CAMPO[1] + LINHA * 0.5);
        if recolhida {
            self.malha.triangulo([cx + 0.75 * r, cy], [cx - 0.75 * r, cy + 0.866 * r], [cx - 0.75 * r, cy - 0.866 * r], ct);
        } else {
            self.malha.triangulo([cx, cy + 0.75 * r], [cx - 0.866 * r, cy - 0.75 * r], [cx + 0.866 * r, cy - 0.75 * r], ct);
        }
        let tx = x + RESPIRO_CAMPO[0] + LINHA + ESPACO_INTERNO[0];
        // O título acaba na borda direita da barra, como no `igBegin`: a janela de plugin mais estreita que o título não o deixa vazar.
        self.malha.recorte = Some(titulo_r);
        self.escrever(tx, y + (ALTURA_TITULO - LINHA) * 0.5, ct, rotulo_visivel(titulo));
        self.malha.recorte = None;

        if recolhida {
            return false;
        }

        // Alça de redimensionar: triângulo no canto inferior direito.
        let lado = (LINHA * 1.10).max(ARREDONDAMENTO_JANELA + 1.0 + LINHA * 0.2).floor();
        let alca = [x1 - lado, y1 - lado, x1, y1];
        let redimensionando = matches!(self.persistente.arraste, Arraste::Redimensionando { janela, .. } if janela == i);
        let sobre_a_alca = self.dentro_desta(alca);
        if redimensionando || (sobre_a_alca && self.persistente.arraste == Arraste::Nenhum) {
            self.ponteiro = Ponteiro::Diagonal;
        }
        let cor_alca = para_bytes(if redimensionando {
            herdada::ALCA_ATIVA
        } else if sobre_a_alca {
            herdada::ALCA_SOBRE
        } else {
            herdada::ALCA
        });
        self.malha.triangulo([x1 - 1.0, y1 - lado], [x1 - 1.0, y1 - 1.0], [x1 - lado, y1 - 1.0], cor_alca);

        let origem = [x + RESPIRO_JANELA[0], y + ALTURA_TITULO + RESPIRO_JANELA[1]];
        // O conteúdo é recortado a meio respiro das bordas laterais, abaixo da barra de título, como na referência.
        let recorte_da_janela = [x + RESPIRO_JANELA[0] * 0.5, y + ALTURA_TITULO, x1 - RESPIRO_JANELA[0] * 0.5, y1];
        self.pilha.push(Area {
            origem,
            limite: [x1 - RESPIRO_JANELA[0], y1 - RESPIRO_JANELA[1]],
            cursor: origem,
            linha_y: origem[1],
            altura_da_linha: 0.0,
            na_mesma_linha: false,
            base_do_texto: 0.0,
            rolavel: None,
            barra: 0.0,
            recorte: recorte_da_janela,
            alcance: origem,
        });
        self.malha.recorte = Some(recorte_da_janela);
        true
    }

    /// Fecha a janela aberta: fecha as áreas dela e guarda onde o desenho dela termina na malha. Nos quadros de ajuste, o tamanho passa a
    /// ser o que o conteúdo pediu; no 1º quadro de uma janela de plugin o desenho é descartado, porque ela só mediu.
    pub(crate) fn fechar_janela(&mut self) {
        let Some((i, inicio, inicio_dos_vertices)) = self.atual else { return };
        let base = self.pilha.first().copied();
        while !self.pilha.is_empty() {
            self.fechar_area();
        }
        self.atual = None;
        self.malha.recorte = None;
        let tela = self.entrada.tamanho_da_tela;
        let j = &mut self.persistente.janelas[i];
        if let Some(a) = base.filter(|_| j.fase.ajusta()) {
            // `CalcWindowAutoFitSize`: conteúdo + respiro dos dois lados + barra de título, entre o mínimo do estilo e a tela menos a margem.
            let conteudo = [(a.alcance[0] - a.origem[0]).max(0.0).floor(), (a.alcance[1] - a.origem[1]).max(0.0).floor()];
            let mut l = conteudo[0] + RESPIRO_JANELA[0] * 2.0;
            let mut h = conteudo[1] + RESPIRO_JANELA[1] * 2.0 + ALTURA_TITULO;
            if tela[0] > 0.0 && tela[1] > 0.0 {
                l = l.min(tela[0] - MARGEM_SEGURA * 2.0);
                h = h.min(tela[1] - MARGEM_SEGURA * 2.0);
            }
            j.ret[2] = l.max(JANELA_DE_PLUGIN_MINIMA[0]);
            j.ret[3] = h.max(JANELA_DE_PLUGIN_MINIMA[1]);
        }
        if j.fase == FaseDaJanela::Medindo {
            self.malha.indices.truncate(inicio);
            self.malha.vertices.truncate(inicio_dos_vertices);
            return;
        }
        self.faixas.push((i, inicio, self.malha.indices.len()));
    }

    /// A máquina de estados do arraste da janela `i`: começa no aperto da barra ou da alça (na janela de cima sob o mouse), segue o mouse
    /// enquanto o botão está apertado, termina ao soltar.
    fn tratar_arraste_da_janela(&mut self, i: usize) {
        let [x, y, l, a] = self.persistente.janelas[i].ret;
        let m = self.entrada.mouse;
        let titulo = [x + RESPIRO_CAMPO[0] + LINHA, y, x + l, y + ALTURA_TITULO];
        let lado = (LINHA * 1.10).max(ARREDONDAMENTO_JANELA + 1.0 + LINHA * 0.2).floor();
        let alca = [x + l - lado, y + a - lado, x + l, y + a];
        if self.entrada.apertou && self.persistente.arraste == Arraste::Nenhum && self.janela_sob_o_mouse == Some(i) {
            if !self.persistente.janelas[i].recolhida && self.dentro(alca) {
                self.persistente.arraste = Arraste::Redimensionando { janela: i, pega: [x + l - m[0], y + a - m[1]] };
            } else if self.dentro(titulo) {
                self.persistente.arraste = Arraste::Janela { janela: i, pega: [m[0] - x, m[1] - y] };
            }
        }
        let minimo = if i == 0 { JANELA_MINIMA } else { JANELA_DE_PLUGIN_MINIMA };
        let arraste = self.persistente.arraste;
        let j = &mut self.persistente.janelas[i];
        match arraste {
            Arraste::Janela { janela, pega } if janela == i && self.entrada.segurando => {
                j.ret[0] = m[0] - pega[0];
                j.ret[1] = m[1] - pega[1];
            }
            Arraste::Redimensionando { janela, pega } if janela == i && self.entrada.segurando => {
                j.ret[2] = (m[0] + pega[0] - x).max(minimo[0]);
                j.ret[3] = (m[1] + pega[1] - y).max(minimo[1]);
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ áreas roláveis

    /// Abre uma área rolável. `altura <= 0` significa "até o fim da área
    /// de fora menos isso" — o painel de log pede `-30` pra deixar a linha do comando embaixo.
    pub(crate) fn abrir_rolavel(&mut self, largura: f32, altura: f32) {
        let indice = self.n_rolaveis.min(self.persistente.rolagem.len() - 1);
        self.n_rolaveis += 1;
        let (x0, y0, lim) = {
            let a = self.area();
            (a.cursor[0], a.cursor[1], a.limite)
        };
        let x1 = if largura <= 0.0 { lim[0] + largura } else { x0 + largura };
        let y1 = if altura <= 0.0 { lim[1] + altura } else { y0 + altura };
        let visivel = [x0, y0, x1, y1];
        if self.sob_o_mouse(visivel) && self.entrada.roda != 0.0 {
            let r = &mut self.persistente.rolagem[indice];
            *r = (*r - self.entrada.roda * LINHA * 5.0).max(0.0);
        }
        let desloc = self.persistente.rolagem[indice];
        let origem = [x0, y0 - desloc];
        let barra = if self.persistente.com_barra[indice] { LARGURA_ROLAGEM } else { 0.0 };
        let recorte = interseccao(self.malha.recorte, visivel);
        self.pilha.push(Area {
            origem,
            limite: [x1 - barra, y1],
            cursor: origem,
            linha_y: origem[1],
            altura_da_linha: 0.0,
            na_mesma_linha: false,
            base_do_texto: 0.0,
            rolavel: Some((indice, y0)),
            barra,
            recorte,
            alcance: origem,
        });
        self.malha.recorte = Some(recorte);
    }

    /// Fecha a área aberta: desenha a barra de rolagem se o conteúdo passou do visível, limita a
    /// rolagem ao conteúdo, e devolve o cursor da área de fora pra logo depois dela.
    pub(crate) fn fechar_area(&mut self) {
        let Some(a) = self.pilha.pop() else { return };
        self.malha.recorte = self.pilha.last().map(|p| p.recorte);
        let Some((indice, topo)) = a.rolavel else {
            return;
        };
        let conteudo = (a.cursor[1] - a.origem[1] - ESPACO_ENTRE_ITENS[1]).max(0.0);
        let visivel = a.limite[1] - topo;
        let max_rolagem = (conteudo - visivel).max(0.0);
        self.persistente.com_barra[indice] = conteudo > visivel;
        let r = &mut self.persistente.rolagem[indice];
        *r = r.min(max_rolagem);
        let rolagem = *r;
        // A barra deste quadro foi decidida na abertura, pelo conteúdo do quadro anterior.
        if a.barra > 0.0 {
            let trilho = [a.limite[0], topo, a.limite[0] + LARGURA_ROLAGEM, a.limite[1]];
            let bg = para_bytes(herdada::TRILHO_ROLAGEM);
            self.malha.retangulo(trilho[0], trilho[1], trilho[2], trilho[3], bg);
            let alt = (visivel * visivel / conteudo).max(PUNHO_MINIMO);
            let y = topo + (visivel - alt) * (rolagem / max_rolagem.max(1.0));
            let punho = [trilho[0] + 2.0, y + 2.0, trilho[2] - 2.0, y + alt - 2.0];
            let sobre = self.dentro(punho);
            if self.entrada.apertou && sobre {
                self.persistente.arraste = Arraste::Rolagem { indice, pega: self.entrada.mouse[1] - y };
            }
            let arrastando = matches!(self.persistente.arraste, Arraste::Rolagem { indice: i, .. } if i == indice);
            if let Arraste::Rolagem { indice: i, pega } = self.persistente.arraste {
                if i == indice && self.entrada.segurando {
                    let t = ((self.entrada.mouse[1] - pega - topo) / (visivel - alt).max(1.0)).clamp(0.0, 1.0);
                    self.persistente.rolagem[indice] = t * max_rolagem;
                }
            }
            let cor = para_bytes(if arrastando {
                herdada::PUNHO_ROLAGEM_ATIVO
            } else if sobre {
                herdada::PUNHO_ROLAGEM_SOBRE
            } else {
                herdada::PUNHO_ROLAGEM
            });
            let raio = (punho[2] - punho[0]) * 0.5;
            self.retangulo_arredondado(punho, raio, cor, false);
        }
        if let Some(pai) = self.pilha.last_mut() {
            pai.na_mesma_linha = false;
            pai.linha_y = a.limite[1];
            pai.altura_da_linha = 0.0;
            pai.cursor = [pai.origem[0], a.limite[1] + ESPACO_ENTRE_ITENS[1]];
        }
        self.ultimo = [a.origem[0], topo, a.limite[0] + a.barra, a.limite[1]];
    }

    /// Rola a área aberta até o fim (o `set_scroll_here_y(1.0)` do log). Chamar DEPOIS do conteúdo.
    pub(crate) fn rolar_para_o_fim(&mut self) {
        let Some(a) = self.pilha.last() else { return };
        let Some((indice, topo)) = a.rolavel else { return };
        let conteudo = a.cursor[1] - a.origem[1] - ESPACO_ENTRE_ITENS[1];
        let visivel = a.limite[1] - topo;
        self.persistente.rolagem[indice] = (conteudo - visivel).max(0.0);
    }

    /// Reordena o desenho das janelas pela `ordem`, de trás para a frente: a da frente cobre as outras. As janelas desenham uma depois da
    /// outra, então as faixas cobrem o começo da malha sem buraco; o que vem depois delas (dica, ponteiro) fica por cima de tudo.
    fn ordenar_faixas(&mut self) {
        if self.faixas.len() < 2 {
            return;
        }
        let fim = self.faixas.iter().map(|f| f.2).max().unwrap_or(0);
        let mut ordenados = Vec::with_capacity(self.malha.indices.len());
        for &i in &self.persistente.ordem {
            for &(j, inicio, fim_da_faixa) in &self.faixas {
                if j == i {
                    ordenados.extend_from_slice(&self.malha.indices[inicio..fim_da_faixa]);
                }
            }
        }
        ordenados.extend_from_slice(&self.malha.indices[fim..]);
        self.malha.indices = ordenados;
    }

    /// O ponteiro por cima de tudo, com o ponto quente no mouse: miolo branco e borda preta de 1 px em volta dele.
    fn desenhar_ponteiro(&mut self) {
        let (grade, quente) = forma_do_ponteiro(self.ponteiro);
        let x0 = (self.entrada.mouse[0] - quente[0]).floor();
        let y0 = (self.entrada.mouse[1] - quente[1]).floor();
        let branco = |c: isize, l: isize| {
            c >= 0 && l >= 0 && grade.get(l as usize).and_then(|s| s.as_bytes().get(c as usize)) == Some(&b'o')
        };
        let (colunas, linhas) = (grade[0].len() as isize, grade.len() as isize);
        for l in -1..=linhas {
            for c in -1..=colunas {
                let cor = if branco(c, l) {
                    [255, 255, 255, 255]
                } else if (-1..=1).any(|dl| (-1..=1).any(|dc| branco(c + dc, l + dl))) {
                    [0, 0, 0, 255]
                } else {
                    continue;
                };
                let (x, y) = (x0 + c as f32, y0 + l as f32);
                self.malha.retangulo(x, y, x + 1.0, y + 1.0, cor);
            }
        }
    }

    /// Termina o frame: fecha o que ficou aberto, desenha a dica por cima de tudo, resolve o foco e
    /// devolve a malha com o que o jogo NÃO deve receber.
    pub(crate) fn terminar(mut self) -> Frame {
        self.fechar_janela();
        while !self.pilha.is_empty() {
            self.fechar_area();
        }
        self.malha.recorte = None;
        self.ordenar_faixas();
        if let Some(t) = self.dica.take() {
            let m = self.entrada.mouse;
            let l = t.chars().count() as f32 * AVANCO * self.escala + RESPIRO_JANELA[0] * 2.0;
            let h = LINHA * self.escala + RESPIRO_JANELA[1] * 2.0;
            let [x, y] = posicao_da_dica(m, [l, h], self.entrada.tamanho_da_tela);
            let fundo = para_bytes(herdada::FUNDO_DICA);
            self.malha.retangulo(x, y, x + l, y + h, fundo);
            let borda = self.cor(BORDA);
            self.contorno([x, y, x + l, y + h], borda);
            let ct = self.cor(TEXTO);
            self.escrever(x + RESPIRO_JANELA[0], y + RESPIRO_JANELA[1], ct, &t);
        }
        if self.entrada.ponteiro {
            self.desenhar_ponteiro();
        }
        // Clique fora de qualquer campo tira o foco e a edição — senão o teclado ficava preso no terminal.
        if self.entrada.apertou && !self.foco_reivindicado {
            self.persistente.foco = None;
            self.persistente.edicao = None;
        }
        // Aperto em área vazia da janela (nenhum item pegou o clique) começa a mover a janela, como na referência; só aqui, no fim do quadro,
        // se sabe se algum widget pegou o aperto.
        if self.entrada.apertou
            && self.mouse_na_janela
            && self.persistente.arraste == Arraste::Nenhum
            && self.persistente.item_apertado.is_none()
            && !self.foco_reivindicado
        {
            if let Some(i) = self.janela_sob_o_mouse {
                let [x, y, _, _] = self.persistente.janelas[i].ret;
                let m = self.entrada.mouse;
                self.persistente.arraste = Arraste::Janela { janela: i, pega: [m[0] - x, m[1] - y] };
            }
        }
        // Clique fora de todas as janelas tira o foco (a barra de título escurece, como na referência).
        if self.entrada.apertou && !self.mouse_na_janela {
            self.persistente.janela_em_foco = None;
        }
        // Tab no último campo volta ao primeiro.
        if self.persistente.comecar_edicao && self.persistente.foco.is_some_and(|f| f >= self.n_campos) {
            self.persistente.foco = (self.n_campos > 0).then_some(0);
            self.persistente.comecar_edicao = self.n_campos > 0;
        }
        if self.entrada.soltou || !self.entrada.segurando {
            self.persistente.arraste = Arraste::Nenhum;
            self.persistente.repete = None;
            self.persistente.item_apertado = None;
        }
        let quer_mouse = self.mouse_na_janela || self.persistente.arraste != Arraste::Nenhum;
        // Só a edição prende o teclado: depois do Enter o campo segue focado (↑/↓ do histórico) e as teclas voltam ao jogo.
        let quer_teclado = self.persistente.edicao.is_some() || self.persistente.comecar_edicao;
        Frame {
            malha: self.malha,
            quer_mouse,
            quer_teclado,
            #[cfg(any(test, feature = "devtools"))]
            achados: self.achados,
        }
    }
}

fn interseccao(a: Option<[f32; 4]>, b: [f32; 4]) -> [f32; 4] {
    match a {
        None => b,
        Some(a) => [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])],
    }
}

/// Índice em BYTES do n-ésimo caractere — `String::insert`/`remove` pedem byte, e cortar no meio de
/// um caractere de vários bytes entra em pânico. O terminal aceita acento.
fn indice_do_caractere(s: &str, n: usize) -> usize {
    s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())
}

#[cfg(test)]
mod teste {
    use super::*;

    fn com_janela<R>(entrada: Entrada, p: &mut Persistente, f: impl FnOnce(&mut Tela) -> R) -> R {
        let mut t = Tela::nova(entrada, p, 1);
        assert!(t.abrir_janela("t"));
        f(&mut t)
    }

    /// Ponto dentro do conteúdo da janela na posição inicial.
    fn no_conteudo(dx: f32, dy: f32) -> [f32; 2] {
        [JANELA_INICIAL[0] + RESPIRO_JANELA[0] + dx, JANELA_INICIAL[1] + ALTURA_TITULO + RESPIRO_JANELA[1] + dy]
    }

    #[test]
    fn metricas_e_cores_batem_com_a_referencia() {
        assert_eq!(PALETAS[1][TEXTO], [0.20, 0.98, 0.55, 1.0]);
        assert_eq!(PALETAS[1][FUNDO_JANELA], [0.0, 0.18, 0.10, 0.93]);
        assert_eq!((ARREDONDAMENTO_JANELA, ARREDONDAMENTO_CAMPO, ARREDONDAMENTO_ABA), (3.0, 2.0, 4.0));
        assert_eq!((RESPIRO_CAMPO, ESPACO_ENTRE_ITENS, ESPACO_INTERNO), ([4.0, 3.0], [8.0, 4.0], [4.0, 4.0]));
        assert_eq!((LARGURA_ROLAGEM, PUNHO_MINIMO), (14.0, 12.0));
        assert_eq!(herdada::MARCA_DO_CHECK, [0.26, 0.59, 0.98, 1.0], "o ✓ é azul no terminal de hoje");
        assert_eq!(herdada::TEXTO_APAGADO, [0.5, 0.5, 0.5, 1.0]);
        assert_eq!((AVANCO, LINHA), (7.0, 13.0), "métrica da ProggyClean");
    }

    #[test]
    fn rotulo_esconde_o_identificador() {
        assert_eq!(rotulo_visivel("Give##Items.X"), "Give");
        assert_eq!(rotulo_visivel("CPVR##apagada"), "CPVR");
        assert_eq!(rotulo_visivel("##cmd"), "");
        assert_eq!(rotulo_visivel("simples"), "simples");
    }

    /// A régua do `igBegin`: `##` separa duas janelas de mesmo nome visível; `###` mantém a janela com o texto da frente mudando.
    #[test]
    fn identidade_da_janela_segue_o_igbegin() {
        assert_eq!(identidade_da_janela("Plugin"), "Plugin");
        assert_eq!(identidade_da_janela("Nome##a"), "Nome##a");
        assert_ne!(identidade_da_janela("Nome##a"), identidade_da_janela("Nome##b"));
        assert_eq!(identidade_da_janela("FPS: 60###fps"), identidade_da_janela("FPS: 59###fps"));
        assert_eq!(rotulo_visivel("FPS: 60###fps"), "FPS: 60");
    }

    /// A janela de plugin sem conteúdo fica no mínimo (32 px) e o título, mais largo, acaba na borda direita da barra, como no `igBegin`.
    #[test]
    fn titulo_de_plugin_nao_vaza_da_janela() {
        let mut p = Persistente::default();
        let mut vertices = Vec::new();
        for _ in 0..2 {
            let mut t = Tela::nova(Entrada::default(), &mut p, 1);
            t.abrir_janela("t");
            t.abrir_janela_de_plugin("Plugin de titulo comprido");
            t.fechar_janela_de_plugin();
            vertices = t.terminar().malha.vertices;
        }
        let [x, y, l, _] = p.janelas[1].ret;
        let na_barra: Vec<f32> = vertices.iter().filter(|v| v.pos[1] >= y && v.pos[1] <= y + ALTURA_TITULO && v.pos[0] >= x).map(|v| v.pos[0]).collect();
        assert!(!na_barra.is_empty(), "a barra do plugin foi desenhada");
        assert!(na_barra.iter().all(|&vx| vx <= x + l), "nada da barra passa da borda direita ({})", x + l);
    }

    /// Os casos da medida de texto da referência: altura = `\n` × linha, mais uma se a última linha tem texto ou não houve `\n`.
    #[test]
    fn colunas_e_linhas_contam_como_a_referencia() {
        assert_eq!(colunas_e_linhas(""), (0, 1));
        assert_eq!(colunas_e_linhas("oi"), (2, 1));
        assert_eq!(colunas_e_linhas("oi\n"), (2, 1));
        assert_eq!(colunas_e_linhas("\n"), (0, 1));
        assert_eq!(colunas_e_linhas("linha um\nlinha 2"), (8, 2));
        assert_eq!(colunas_e_linhas("linha um\r\nlinha 2\r\n"), (8, 2));
        assert_eq!(colunas_e_linhas("a\n\nbc"), (2, 3));
    }

    #[test]
    fn cor_converte_sem_perder_o_extremo() {
        assert_eq!(para_bytes([0.0, 1.0, 0.5, 1.0]), [0, 255, 128, 255]);
    }

    #[test]
    fn texto_desce_uma_linha_de_13_mais_o_espaco() {
        let mut p = Persistente::default();
        com_janela(Entrada::default(), &mut p, |t| {
            let y0 = t.cursor()[1];
            t.texto("abc");
            assert_eq!(t.cursor()[1] - y0, LINHA + ESPACO_ENTRE_ITENS[1]);
        });
    }

    #[test]
    fn mesma_linha_fica_na_altura_e_a_direita() {
        let mut p = Persistente::default();
        com_janela(Entrada::default(), &mut p, |t| {
            let y0 = t.cursor()[1];
            t.botao("um");
            t.mesma_linha();
            assert_eq!(t.cursor()[1], y0);
            assert_eq!(t.cursor()[0], t.ultimo[2] + ESPACO_ENTRE_ITENS[0]);
            t.botao("dois");
            assert!(t.cursor()[1] > y0, "depois do segundo, desce");
        });
    }

    #[test]
    fn botao_so_dispara_dentro_e_ao_soltar() {
        let aperta = |m: [f32; 2]| Entrada { apertou: true, segurando: true, mouse: m, ..Default::default() };
        let solta = |m: [f32; 2]| Entrada { soltou: true, mouse: m, ..Default::default() };
        let dentro = no_conteudo(3.0, 3.0);
        let mut p = Persistente::default();
        assert!(!com_janela(aperta(dentro), &mut p, |t| t.botao("ok")), "não dispara no aperto");
        assert!(com_janela(solta(dentro), &mut p, |t| t.botao("ok")), "dispara ao soltar em cima");
        let mut p = Persistente::default();
        com_janela(aperta(dentro), &mut p, |t| t.botao("ok"));
        assert!(!com_janela(solta([0.0, 0.0]), &mut p, |t| t.botao("ok")), "soltar fora não dispara");
    }

    #[test]
    fn dica_fica_a_direita_do_ponteiro_e_vira_perto_da_borda() {
        let tela = [760.0, 660.0];
        assert_eq!(posicao_da_dica([172.0, 186.0], [163.0, 29.0], tela), [184.0, 186.0], "à direita, na altura do mouse");
        assert_eq!(posicao_da_dica([700.0, 186.0], [163.0, 29.0], tela), [597.0, 198.0], "sem espaço à direita: embaixo, presa à borda");
        assert_eq!(posicao_da_dica([700.0, 640.0], [163.0, 29.0], tela), [597.0, 603.0], "sem espaço embaixo: em cima");
        assert_eq!(posicao_da_dica([172.0, 186.0], [163.0, 29.0], [0.0, 0.0]), [184.0, 186.0], "tela desconhecida: nunca vira");
    }

    #[test]
    fn soltar_um_arraste_em_cima_do_botao_nao_o_aciona() {
        let mut p = Persistente::default();
        let apertou_fora = Entrada { apertou: true, segurando: true, mouse: no_conteudo(300.0, 3.0), ..Default::default() };
        com_janela(apertou_fora, &mut p, |t| t.botao("ok"));
        let solta_em_cima = Entrada { soltou: true, mouse: no_conteudo(3.0, 3.0), ..Default::default() };
        assert!(!com_janela(solta_em_cima, &mut p, |t| t.botao("ok")));
    }

    #[test]
    fn botao_mostra_so_o_rotulo_antes_do_identificador() {
        let mut p = Persistente::default();
        com_janela(Entrada::default(), &mut p, |t| {
            t.botao("Give##Items.Preset_Katana_Saburo");
            assert_eq!(t.ultimo[2] - t.ultimo[0], 4.0 * AVANCO + RESPIRO_CAMPO[0] * 2.0);
        });
    }

    #[test]
    fn caixa_de_marcar_inverte_e_desenha_o_check() {
        let mut p = Persistente::default();
        let aperto = Entrada { apertou: true, segurando: true, mouse: no_conteudo(3.0, 3.0), ..Default::default() };
        com_janela(aperto, &mut p, |t| {
            let mut v = false;
            assert!(!t.caixa_de_marcar("liga", &mut v), "não inverte no aperto");
        });
        let e = Entrada { soltou: true, mouse: no_conteudo(3.0, 3.0), ..Default::default() };
        com_janela(e, &mut p, |t| {
            let mut v = false;
            let antes = t.malha.indices.len();
            assert!(t.caixa_de_marcar("liga", &mut v));
            assert!(v);
            assert!(t.malha.indices.len() > antes);
        });
    }

    #[test]
    fn campo_ganha_foco_no_clique_e_aceita_acento() {
        let mut p = Persistente::default();
        let clique = Entrada { apertou: true, segurando: true, mouse: no_conteudo(5.0, 5.0), ..Default::default() };
        let mut buf = String::new();
        let (_, foco) = com_janela(clique, &mut p, |t| t.campo_de_texto(&mut buf, 200.0, "dica"));
        assert!(foco);
        let digita = Entrada { digitados: vec!['ç', 'ã', 'o'], ..Default::default() };
        com_janela(digita, &mut p, |t| t.campo_de_texto(&mut buf, 200.0, "dica"));
        assert_eq!(buf, "ção");
    }

    #[test]
    fn clique_fora_tira_o_foco_e_devolve_o_teclado_ao_jogo() {
        let mut p = Persistente { foco: Some(0), ..Default::default() };
        let fora = Entrada { apertou: true, segurando: true, mouse: [5.0, 5.0], ..Default::default() };
        let mut buf = String::new();
        let mut t = Tela::nova(fora, &mut p, 1);
        t.abrir_janela("t");
        t.campo_de_texto(&mut buf, 200.0, "");
        let f = t.terminar();
        assert!(!f.quer_teclado);
    }

    #[test]
    fn backspace_apaga_caractere_de_varios_bytes_inteiro() {
        let mut p = Persistente { foco: Some(0), edicao: Some(Edicao::comecar("ção")), ..Default::default() };
        let e = Entrada { teclas: vec![TECLA_BACKSPACE], ..Default::default() };
        let mut buf = String::from("ção");
        com_janela(e, &mut p, |t| t.campo_de_texto(&mut buf, 200.0, ""));
        assert_eq!(buf, "çã");
    }

    #[test]
    fn enter_confirma_so_com_foco() {
        let mut buf = String::from("help");
        let mut p = Persistente { foco: Some(0), edicao: Some(Edicao::comecar("help")), ..Default::default() };
        let e = Entrada { teclas: vec![TECLA_ENTER], ..Default::default() };
        assert!(com_janela(e.clone(), &mut p, |t| t.campo_de_texto(&mut buf, 200.0, "")).0);
        let mut p = Persistente::default();
        assert!(!com_janela(e, &mut p, |t| t.campo_de_texto(&mut buf, 200.0, "")).0);
    }

    #[test]
    fn enter_sai_da_edicao_mas_o_foco_fica_e_o_teclado_volta_ao_jogo() {
        let mut buf = String::from("heal");
        let mut p = Persistente { foco: Some(0), edicao: Some(Edicao::comecar("heal")), ..Default::default() };
        let e = Entrada { eventos: vec![Evento::Tecla(crate::terminal::edicao::tecla::ENTER, Mods::default())], ..Default::default() };
        let mut t = Tela::nova(e, &mut p, 1);
        t.abrir_janela("t");
        let (confirmou, foco) = t.campo_de_texto(&mut buf, 200.0, "");
        let f = t.terminar();
        assert!(confirmou && foco, "o ↑/↓ do histórico segue valendo");
        assert!(!f.quer_teclado, "as teclas voltam ao jogo, como no terminal de hoje");
        assert!(p.edicao.is_none());
    }

    #[test]
    fn tab_passa_a_edicao_ao_proximo_campo_e_do_ultimo_volta_ao_primeiro() {
        let (mut a, mut b) = (String::from("um"), String::from("dois"));
        let mut p = Persistente { foco: Some(0), edicao: Some(Edicao::comecar("um")), ..Default::default() };
        let tab = Entrada { eventos: vec![Evento::Tecla(crate::terminal::edicao::tecla::TAB, Mods::default())], ..Default::default() };
        let quadro = |p: &mut Persistente, a: &mut String, b: &mut String| {
            let mut t = Tela::nova(tab.clone(), p, 1);
            t.abrir_janela("t");
            t.campo_de_texto(a, 200.0, "");
            t.campo_de_texto(b, 200.0, "");
            t.terminar();
        };
        quadro(&mut p, &mut a, &mut b);
        assert_eq!(p.foco, Some(1));
        assert_eq!(p.edicao.as_ref().map(|e| e.original.as_str()), Some("dois"));
        quadro(&mut p, &mut a, &mut b);
        assert_eq!(p.foco, Some(0), "do último volta ao primeiro");
        assert!(p.comecar_edicao, "a edição começa no primeiro campo no quadro seguinte");
    }

    #[test]
    fn clique_poe_o_cursor_sob_o_mouse_e_arrastar_seleciona() {
        let mut buf = String::from("give Items.X");
        let mut p = Persistente::default();
        let x = |c: f32| no_conteudo(RESPIRO_CAMPO[0] + c * AVANCO, 5.0);
        let aperta = Entrada { apertou: true, segurando: true, mouse: x(5.0), ..Default::default() };
        com_janela(aperta, &mut p, |t| t.campo_de_texto(&mut buf, 300.0, ""));
        assert_eq!(p.edicao.as_ref().map(|e| e.cursor), Some(5));
        let arrasta = Entrada { segurando: true, mouse: x(10.0), ..Default::default() };
        com_janela(arrasta, &mut p, |t| t.campo_de_texto(&mut buf, 300.0, ""));
        assert_eq!(p.edicao.as_ref().and_then(Edicao::selecao), Some((5, 10)));
        let duplo = Entrada { apertou: true, segurando: true, clique_duplo: true, mouse: x(2.0), ..Default::default() };
        com_janela(duplo, &mut p, |t| t.campo_de_texto(&mut buf, 300.0, ""));
        assert_eq!(p.edicao.as_ref().and_then(Edicao::selecao), Some((0, 4)), "clique duplo pega 'give'");
    }

    #[test]
    fn texto_maior_que_o_campo_rola_para_o_cursor_ficar_visivel() {
        let mut buf = "x".repeat(60);
        let mut p = Persistente { foco: Some(0), edicao: Some(Edicao::comecar(&buf)), ..Default::default() };
        com_janela(Entrada::default(), &mut p, |t| t.campo_de_texto(&mut buf, 100.0, ""));
        let util = 100.0 - RESPIRO_CAMPO[0] * 2.0;
        let cx = 60.0 * AVANCO;
        assert!(p.rolagem_do_campo > 0.0 && cx - p.rolagem_do_campo <= util, "o cursor no fim cabe no campo");
    }

    #[test]
    fn campo_numerico_soma_e_subtrai() {
        let mut v = 1;
        let mut p = Persistente::default();
        // botão `+`: fica depois do campo e do `-`
        let largura = 80.0;
        let lado = ALTURA_DE_CAMPO;
        let x_mais = largura - lado + 3.0;
        let e = Entrada { apertou: true, segurando: true, mouse: no_conteudo(x_mais, 5.0), ..Default::default() };
        assert!(com_janela(e, &mut p, |t| t.campo_numerico("qty", &mut v, largura)));
        assert_eq!(v, 2);
    }

    #[test]
    fn mais_seguro_repete_depois_da_espera() {
        let mut v = 1;
        let mut p = Persistente::default();
        let largura = 80.0;
        let x_mais = largura - ALTURA_DE_CAMPO + 3.0;
        let quadro = |e: Entrada, p: &mut Persistente, v: &mut i32| {
            let mut t = Tela::nova(e, p, 1);
            t.abrir_janela("t");
            t.campo_numerico("qty", v, largura);
            t.terminar();
        };
        quadro(Entrada { apertou: true, segurando: true, mouse: no_conteudo(x_mais, 5.0), ..Default::default() }, &mut p, &mut v);
        assert_eq!(v, 2, "dispara no aperto");
        quadro(Entrada { segurando: true, dt: 0.2, mouse: no_conteudo(x_mais, 5.0), ..Default::default() }, &mut p, &mut v);
        assert_eq!(v, 2, "ainda na espera de 0,275 s");
        quadro(Entrada { segurando: true, dt: 0.2, mouse: no_conteudo(x_mais, 5.0), ..Default::default() }, &mut p, &mut v);
        assert_eq!(v, 5, "passou da espera em 0,125 s: repete em 0, 0,05 e 0,10");
        quadro(Entrada { soltou: true, mouse: no_conteudo(x_mais, 5.0), ..Default::default() }, &mut p, &mut v);
        quadro(Entrada { segurando: true, dt: 1.0, mouse: no_conteudo(x_mais, 5.0), ..Default::default() }, &mut p, &mut v);
        assert_eq!(v, 5, "soltou: segurar de novo sem apertar não repete");
    }

    #[test]
    fn numero_se_digita_e_so_aceita_digito_e_sinal() {
        let mut v = 1;
        let mut p = Persistente::default();
        let clique = Entrada { apertou: true, segurando: true, mouse: no_conteudo(5.0, 5.0), ..Default::default() };
        com_janela(clique, &mut p, |t| t.campo_numerico("qty", &mut v, 80.0));
        let digita = Entrada { digitados: vec!['2', 'x', '5'], ..Default::default() };
        assert!(com_janela(digita, &mut p, |t| t.campo_numerico("qty", &mut v, 80.0)));
        assert_eq!(v, 251, "o clique no começo põe o cursor antes do 1; o x é recusado");
    }

    #[test]
    fn abas_trocam_ao_clicar_na_segunda() {
        let mut p = Persistente::default();
        let x = 7.0 * AVANCO + RESPIRO_CAMPO[0] * 2.0 + ESPACO_INTERNO[0] + 3.0;
        let e = Entrada { apertou: true, segurando: true, mouse: no_conteudo(x, 5.0), ..Default::default() };
        assert_eq!(com_janela(e, &mut p, |t| t.abas(&["Console", "Items"])), 1, "a aba troca no aperto");
        assert_eq!(p.aba_ativa[0], 1, "a escolha sobrevive ao frame");
    }

    #[test]
    fn arrastar_a_barra_move_a_janela() {
        let mut p = Persistente::default();
        let pega = [JANELA_INICIAL[0] + 100.0, JANELA_INICIAL[1] + 5.0];
        let e = Entrada { apertou: true, segurando: true, mouse: pega, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        let e = Entrada { segurando: true, mouse: [pega[0] + 40.0, pega[1] + 25.0], ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_INICIAL[0] + 40.0, JANELA_INICIAL[1] + 25.0]);
    }

    #[test]
    fn arrastar_a_alca_redimensiona_e_respeita_o_minimo() {
        let mut p = Persistente::default();
        let canto = [JANELA_INICIAL[0] + JANELA_INICIAL[2] - 3.0, JANELA_INICIAL[1] + JANELA_INICIAL[3] - 3.0];
        let e = Entrada { apertou: true, segurando: true, mouse: canto, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        let e = Entrada { segurando: true, mouse: [canto[0] + 50.0, canto[1] + 30.0], ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[2], p.janelas[0].ret[3]], [JANELA_INICIAL[2] + 50.0, JANELA_INICIAL[3] + 30.0]);
        let e = Entrada { segurando: true, mouse: [JANELA_INICIAL[0] + 5.0, JANELA_INICIAL[1] + 5.0], ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[2], p.janelas[0].ret[3]], JANELA_MINIMA);
    }

    #[test]
    fn tela_que_encolhe_traz_a_janela_inteira_para_dentro() {
        let mut p = Persistente::default();
        let e = Entrada { tamanho_da_tela: [1920.0, 1200.0], ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_INICIAL[0], JANELA_INICIAL[1]]);
        // 2× em 1920×1200: a tela lógica tem 960×600 e a janela (y 150, altura 460) passaria da borda de baixo.
        let e = Entrada { tamanho_da_tela: [960.0, 600.0], ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_INICIAL[0], 600.0 - JANELA_INICIAL[3]]);
    }

    #[test]
    fn janela_de_plugin_mede_aparece_e_para_de_ajustar() {
        let mut fase = FaseDaJanela::Nova;
        let mut visto = Vec::new();
        for _ in 0..4 {
            fase = fase.ao_abrir();
            visto.push((fase, fase.aparece(), fase.ajusta()));
        }
        assert_eq!(
            visto,
            vec![
                (FaseDaJanela::Medindo, false, true),
                (FaseDaJanela::Aparecendo, true, true),
                (FaseDaJanela::Visivel, true, false),
                (FaseDaJanela::Visivel, true, false),
            ]
        );
        assert!(Janela::do_terminal().fase.aparece() && !Janela::do_terminal().fase.ajusta(), "o terminal nasce na tela, com o tamanho dele");
    }

    #[test]
    fn clique_vai_para_a_janela_de_cima_e_a_traz_para_a_frente() {
        let mut p = Persistente::default();
        let quadro = |p: &mut Persistente, e: Entrada| {
            let mut t = Tela::nova(e, p, 1);
            t.abrir_janela("terminal");
            t.abrir_janela_de_plugin("plugin");
            t.texto("oi");
            t.terminar()
        };
        quadro(&mut p, Entrada::default());
        quadro(&mut p, Entrada::default());
        // O plugin passa a cobrir um pedaço do corpo do terminal.
        p.janelas[1].ret = [100.0, 300.0, 200.0, 100.0];
        quadro(&mut p, Entrada::default());
        assert_eq!(p.ordem, vec![0, 1]);
        let coberto = [150.0, 350.0];
        let f = quadro(&mut p, Entrada { mouse: coberto, apertou: true, segurando: true, ..Default::default() });
        assert_eq!((p.ordem.clone(), p.janela_em_foco), (vec![0, 1], Some(1)), "o aperto na parte coberta é do plugin, que já está na frente");
        assert!(matches!(p.arraste, Arraste::Janela { janela: 1, .. }), "área vazia do plugin começa a mover o plugin, não o terminal");
        assert!(f.quer_mouse);
        quadro(&mut p, Entrada { mouse: coberto, soltou: true, ..Default::default() });
        let so_terminal = [600.0, 500.0];
        quadro(&mut p, Entrada { mouse: so_terminal, apertou: true, segurando: true, ..Default::default() });
        assert_eq!((p.ordem.clone(), p.janela_em_foco), (vec![1, 0], Some(0)), "o terminal vem para a frente com o foco");
        quadro(&mut p, Entrada { mouse: so_terminal, soltou: true, ..Default::default() });
        quadro(&mut p, Entrada { mouse: [5.0, 5.0], apertou: true, segurando: true, ..Default::default() });
        assert_eq!(p.janela_em_foco, None, "clique fora de todas tira o foco");
    }

    #[test]
    fn ponteiro_vira_i_no_campo_e_diagonal_na_alca() {
        let mut p = Persistente::default();
        let mut buf = String::new();
        let sobre_o_campo = [JANELA_INICIAL[0] + RESPIRO_JANELA[0] + 10.0, JANELA_INICIAL[1] + ALTURA_TITULO + RESPIRO_JANELA[1] + 5.0];
        let mut t = Tela::nova(Entrada { mouse: sobre_o_campo, ..Default::default() }, &mut p, 1);
        t.abrir_janela("t");
        assert_eq!(t.ponteiro, Ponteiro::Seta);
        t.campo_de_texto(&mut buf, -1.0, "");
        assert_eq!(t.ponteiro, Ponteiro::Texto);
        let canto = [JANELA_INICIAL[0] + JANELA_INICIAL[2] - 3.0, JANELA_INICIAL[1] + JANELA_INICIAL[3] - 3.0];
        let mut t = Tela::nova(Entrada { mouse: canto, ..Default::default() }, &mut p, 1);
        t.abrir_janela("t");
        assert_eq!(t.ponteiro, Ponteiro::Diagonal);
    }

    #[test]
    fn janela_arrastada_para_fora_fica_presa_a_tela() {
        let mut p = Persistente::default();
        let tela = [1280.0, 800.0];
        let pega = [JANELA_INICIAL[0] + 100.0, JANELA_INICIAL[1] + 5.0];
        let e = Entrada { apertou: true, segurando: true, mouse: pega, tamanho_da_tela: tela, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        let e = Entrada { segurando: true, mouse: [3000.0, 2000.0], tamanho_da_tela: tela, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [tela[0] - JANELA_VISIVEL, tela[1] - JANELA_VISIVEL]);
        let e = Entrada { segurando: true, mouse: [-3000.0, -2000.0], tamanho_da_tela: tela, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_VISIVEL - JANELA_INICIAL[2], JANELA_VISIVEL - JANELA_INICIAL[3]]);
    }

    #[test]
    fn clicar_na_seta_recolhe_e_esconde_o_conteudo() {
        let mut p = Persistente::default();
        let seta = [JANELA_INICIAL[0] + RESPIRO_CAMPO[0] + 4.0, JANELA_INICIAL[1] + 8.0];
        let aperto = Entrada { apertou: true, segurando: true, mouse: seta, ..Default::default() };
        let mut t = Tela::nova(aperto, &mut p, 1);
        t.abrir_janela("t");
        t.terminar();
        assert!(!p.janelas[0].recolhida, "a seta dispara ao soltar");
        let e = Entrada { soltou: true, mouse: seta, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert!(p.janelas[0].recolhida);
        assert!(!Tela::nova(Entrada::default(), &mut p, 1).abrir_janela("t"));
    }

    #[test]
    fn clique_duplo_na_barra_recolhe() {
        let mut p = Persistente::default();
        let barra = [JANELA_INICIAL[0] + 200.0, JANELA_INICIAL[1] + 8.0];
        let e = Entrada { apertou: true, segurando: true, clique_duplo: true, mouse: barra, ..Default::default() };
        Tela::nova(e, &mut p, 1).abrir_janela("t");
        assert!(p.janelas[0].recolhida);
    }

    #[test]
    fn arrastar_area_vazia_move_a_janela_e_arrastar_botao_nao() {
        let quadro = |p: &mut Persistente, e: Entrada| {
            let mut t = Tela::nova(e, p, 1);
            t.abrir_janela("t");
            t.botao("ok");
            t.terminar();
        };
        let vazio = no_conteudo(300.0, 200.0);
        let mut p = Persistente::default();
        quadro(&mut p, Entrada { apertou: true, segurando: true, mouse: vazio, ..Default::default() });
        quadro(&mut p, Entrada { segurando: true, mouse: [vazio[0] + 30.0, vazio[1] + 20.0], ..Default::default() });
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_INICIAL[0] + 30.0, JANELA_INICIAL[1] + 20.0]);
        let botao = no_conteudo(3.0, 3.0);
        let mut p = Persistente::default();
        quadro(&mut p, Entrada { apertou: true, segurando: true, mouse: botao, ..Default::default() });
        quadro(&mut p, Entrada { segurando: true, mouse: [botao[0] + 30.0, botao[1] + 20.0], ..Default::default() });
        assert_eq!([p.janelas[0].ret[0], p.janelas[0].ret[1]], [JANELA_INICIAL[0], JANELA_INICIAL[1]], "o aperto foi do botão");
    }

    #[test]
    fn rolavel_recorta_o_conteudo_e_limita_a_rolagem() {
        let mut p = Persistente::default();
        p.rolagem[0] = 1.0e6;
        com_janela(Entrada::default(), &mut p, |t| {
            t.abrir_rolavel(0.0, 100.0);
            assert!(t.malha.recorte.is_some());
            for i in 0..40 {
                t.texto(&format!("linha {i}"));
            }
            t.fechar_area();
        });
        let conteudo = 40.0 * (LINHA + ESPACO_ENTRE_ITENS[1]) - ESPACO_ENTRE_ITENS[1];
        assert_eq!(p.rolagem[0], conteudo - 100.0, "rolagem presa no fim do conteúdo");
    }

    #[test]
    fn mouse_na_janela_tira_o_clique_do_jogo() {
        let mut p = Persistente::default();
        let e = Entrada { mouse: no_conteudo(10.0, 10.0), ..Default::default() };
        let mut t = Tela::nova(e, &mut p, 1);
        t.abrir_janela("t");
        assert!(t.terminar().quer_mouse);
        let mut p = Persistente::default();
        let mut t = Tela::nova(Entrada { mouse: [1.0, 1.0], ..Default::default() }, &mut p, 1);
        t.abrir_janela("t");
        assert!(!t.terminar().quer_mouse);
    }

    #[test]
    fn quebra_como_a_referencia_em_branco_e_depois_de_pontuacao() {
        assert_eq!(linhas_quebradas("abc def ghi", 7), vec!["abc def", "ghi"]);
        assert_eq!(linhas_quebradas("aa.app,carregado,sem", 12), vec!["aa.app,", "carregado,", "sem"], "quebra depois da vírgula");
        assert_eq!(linhas_quebradas("x".repeat(10).as_str(), 4), vec!["xxxx", "xxxx", "xx"], "palavra maior que a linha é cortada");
        assert_eq!(linhas_quebradas("um    dois", 3), vec!["um", "doi", "s"], "brancos do começo da linha seguinte são pulados");
        assert_eq!(linhas_quebradas("a\nb", 10), vec!["a", "b"]);
        assert_eq!(linhas_quebradas("", 10), vec![""]);
    }

    #[test]
    fn texto_quebrado_corta_palavra_maior_que_a_linha() {
        let mut p = Persistente::default();
        com_janela(Entrada::default(), &mut p, |t| {
            let y0 = t.cursor()[1];
            t.texto_quebrado(&"x".repeat(400));
            assert!(t.cursor()[1] - y0 > 2.0 * LINHA, "400 caracteres não cabem numa linha");
        });
    }

    /// Um quadro com o que a procura de dev (`termonde`, `termclique`) acha: uma caixa com id no rótulo (`##`), um botão e um texto.
    fn quadro_com_caixa(p: &mut Persistente, e: Entrada, marcada: &mut bool) -> Frame {
        let mut t = Tela::nova(e, p, 1);
        t.abrir_janela("terminal");
        t.caixa_de_marcar("Quadra Turbo R##mod_Quadra Turbo R", marcada);
        t.botao("Atualizar");
        t.texto("⚠ Reiniciar para aplicar");
        t.terminar()
    }

    fn procurando(rotulo: &str) -> Entrada {
        Entrada { procura: Some(Alvo::Rotulo(rotulo.to_string())), ..Default::default() }
    }

    #[test]
    fn procura_pelo_rotulo_da_o_lugar_e_o_estado_da_caixa() {
        let mut p = Persistente::default();
        let mut marcada = false;
        let f = quadro_com_caixa(&mut p, procurando("Quadra Turbo R"), &mut marcada);
        let lado = LINHA + RESPIRO_CAMPO[1] * 2.0;
        let [x0, y0] = no_conteudo(0.0, 0.0);
        let largura = lado + ESPACO_INTERNO[0] + 14.0 * AVANCO;
        let caixa = ItemAchado { tipo: TipoDeItem::Caixa, rotulo: "Quadra Turbo R".into(), ret: [x0, y0, x0 + largura, y0 + lado], visivel: true, estado: Some(false) };
        assert_eq!(f.achados, vec![caixa.clone()]);
        assert_eq!(quadro_com_caixa(&mut p, procurando("Quadra Turbo R##mod_Quadra Turbo R"), &mut marcada).achados, vec![caixa], "o rótulo inteiro acha o mesmo");
        let aviso = quadro_com_caixa(&mut p, procurando("⚠ Reiniciar para aplicar"), &mut marcada).achados;
        assert_eq!(aviso.iter().map(|i| (i.tipo, i.visivel, i.estado)).collect::<Vec<_>>(), vec![(TipoDeItem::Texto, true, None)]);
        assert!(quadro_com_caixa(&mut p, procurando("Quadra"), &mut marcada).achados.is_empty(), "o rótulo casa inteiro, não um pedaço");
        assert!(quadro_com_caixa(&mut p, Entrada::default(), &mut marcada).achados.is_empty(), "sem procura, nada é anotado");
    }

    #[test]
    fn procura_pelo_ponto_acha_o_item_que_o_clique_ali_aciona() {
        let mut p = Persistente::default();
        let mut marcada = false;
        let ponto = no_conteudo(30.0, 9.0);
        let aperto = Entrada { mouse: ponto, apertou: true, segurando: true, procura: Some(Alvo::Ponto(ponto)), ..Default::default() };
        let f = quadro_com_caixa(&mut p, aperto, &mut marcada);
        assert_eq!(f.achados.iter().map(|i| (i.tipo, i.rotulo.as_str(), i.visivel)).collect::<Vec<_>>(), vec![(TipoDeItem::Caixa, "Quadra Turbo R", true)]);
        assert!(f.quer_mouse && !marcada, "o aperto só ativa a caixa");
        quadro_com_caixa(&mut p, Entrada { mouse: ponto, soltou: true, ..Default::default() }, &mut marcada);
        assert!(marcada, "a soltura no item achado liga a caixa");
    }

    #[test]
    fn item_rolado_para_fora_ou_coberto_por_outra_janela_nao_fica_visivel() {
        let mut p = Persistente::default();
        let mut t = Tela::nova(procurando("fundo"), &mut p, 1);
        t.abrir_janela("terminal");
        t.abrir_rolavel(0.0, 40.0);
        for i in 0..10 {
            t.texto(&format!("linha {i}"));
        }
        let mut v = false;
        t.caixa_de_marcar("fundo", &mut v);
        t.fechar_area();
        assert_eq!(t.terminar().achados.iter().map(|i| i.visivel).collect::<Vec<_>>(), vec![false], "abaixo da área rolável");

        let mut p = Persistente::default();
        let quadro = |p: &mut Persistente, e: Entrada| {
            let mut t = Tela::nova(e, p, 1);
            t.abrir_janela("terminal");
            let mut v = false;
            t.caixa_de_marcar("coberta", &mut v);
            t.abrir_janela_de_plugin("plugin");
            t.texto("oi");
            t.terminar()
        };
        quadro(&mut p, Entrada::default());
        quadro(&mut p, Entrada::default());
        let [x0, y0] = no_conteudo(0.0, 0.0);
        p.janelas[1].ret = [x0 - 10.0, y0 - 10.0, 200.0, 100.0];
        quadro(&mut p, Entrada::default());
        let coberta = quadro(&mut p, procurando("coberta")).achados;
        assert_eq!(coberta.iter().map(|i| i.visivel).collect::<Vec<_>>(), vec![false], "o plugin está por cima da caixa");
        let do_plugin = quadro(&mut p, procurando("oi")).achados;
        assert_eq!(do_plugin.iter().map(|i| i.visivel).collect::<Vec<_>>(), vec![true], "o texto do plugin é da janela de cima");
    }
}
