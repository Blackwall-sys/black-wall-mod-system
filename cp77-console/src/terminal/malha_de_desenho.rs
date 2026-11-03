//! Primitivas de desenho do projeto: a fonte, o atlas e a malha de vértices — sem nenhuma
//! biblioteca de terceiro.
//!
//! POR QUE EXISTE: o splash e o TERMINAL desenham com as mesmas primitivas, e o dylib que vai ao
//! usuário não leva biblioteca de desenho de terceiro. O splash tem que continuar idêntico ao pixel
//! (`splash::teste`) e o terminal igual ao desenho aprovado (`overlay::prova_visual`).
//!
//! O formato de vértice é o mesmo que o pipeline do overlay espera, então os três (splash,
//! terminal e overlay) compartilham um pipeline e uma textura só.

/// Glifo de 7 colunas × 13 linhas com traço de 1 px, desenhado no projeto: um byte por linha, bit 6 =
/// coluna da esquerda; índice = código ASCII − 32 (32..126). A métrica é essa por causa da MARCA: numa
/// caixa de 5×7 as diagonais do desenho isométrico ganham degrau duplo e as quatro letras se fundem
/// num bloco (visto na prévia rasterizada, antes de qualquer boot). Com 13 linhas e traço de 1 px o
/// desenho sai nítido — que é como ele sempre foi na tela.
pub(crate) const LARGURA_GLIFO: usize = 7;
pub(crate) const ALTURA_GLIFO: usize = 13;
pub(crate) const PRIMEIRO_CODIGO: usize = 32;
pub(crate) static FONTE: [[u8; ALTURA_GLIFO]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // (espaço)
    [0x00, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00], // !
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // "
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // #
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // $
    [0x00, 0x61, 0x62, 0x04, 0x08, 0x10, 0x26, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00], // %
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // &
    [0x00, 0x08, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '
    [0x04, 0x08, 0x10, 0x10, 0x10, 0x10, 0x10, 0x08, 0x04, 0x00, 0x00, 0x00, 0x00], // (
    [0x10, 0x08, 0x04, 0x04, 0x04, 0x04, 0x04, 0x08, 0x10, 0x00, 0x00, 0x00, 0x00], // )
    [0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // *
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // +
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x08, 0x20, 0x00, 0x00], // ,
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // -
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00], // .
    [0x01, 0x01, 0x02, 0x02, 0x04, 0x04, 0x08, 0x10, 0x10, 0x20, 0x20, 0x40, 0x40], // /
    [0x00, 0x1c, 0x22, 0x26, 0x2a, 0x2a, 0x32, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // 0
    [0x00, 0x08, 0x18, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, 0x00], // 1
    [0x00, 0x1c, 0x22, 0x02, 0x02, 0x04, 0x08, 0x10, 0x20, 0x3e, 0x00, 0x00, 0x00], // 2
    [0x00, 0x3e, 0x04, 0x08, 0x04, 0x02, 0x02, 0x02, 0x22, 0x1c, 0x00, 0x00, 0x00], // 3
    [0x00, 0x04, 0x0c, 0x14, 0x24, 0x24, 0x3e, 0x04, 0x04, 0x04, 0x00, 0x00, 0x00], // 4
    [0x00, 0x3e, 0x20, 0x20, 0x3c, 0x02, 0x02, 0x02, 0x22, 0x1c, 0x00, 0x00, 0x00], // 5
    [0x00, 0x0c, 0x10, 0x20, 0x20, 0x3c, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // 6
    [0x00, 0x3e, 0x02, 0x04, 0x04, 0x08, 0x08, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00], // 7
    [0x00, 0x1c, 0x22, 0x22, 0x22, 0x1c, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // 8
    [0x00, 0x1c, 0x22, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x04, 0x18, 0x00, 0x00, 0x00], // 9
    [0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00], // :
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ;
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // <
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // =
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // >
    [0x00, 0x3e, 0x41, 0x01, 0x06, 0x08, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00], // ?
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // @
    [0x00, 0x08, 0x14, 0x14, 0x22, 0x3e, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // A
    [0x00, 0x3c, 0x22, 0x22, 0x3c, 0x22, 0x22, 0x22, 0x22, 0x3c, 0x00, 0x00, 0x00], // B
    [0x00, 0x1c, 0x22, 0x20, 0x20, 0x20, 0x20, 0x20, 0x22, 0x1c, 0x00, 0x00, 0x00], // C
    [0x00, 0x3c, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x3c, 0x00, 0x00, 0x00], // D
    [0x00, 0x3e, 0x20, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x20, 0x3e, 0x00, 0x00, 0x00], // E
    [0x00, 0x3e, 0x20, 0x20, 0x20, 0x3c, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00], // F
    [0x00, 0x1c, 0x22, 0x20, 0x20, 0x26, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // G
    [0x00, 0x22, 0x22, 0x22, 0x22, 0x3e, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // H
    [0x00, 0x1c, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, 0x00], // I
    [0x00, 0x0e, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04, 0x24, 0x18, 0x00, 0x00, 0x00], // J
    [0x00, 0x22, 0x24, 0x28, 0x30, 0x20, 0x30, 0x28, 0x24, 0x22, 0x00, 0x00, 0x00], // K
    [0x00, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x20, 0x3e, 0x00, 0x00, 0x00], // L
    [0x00, 0x22, 0x36, 0x2a, 0x2a, 0x22, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // M
    [0x00, 0x22, 0x32, 0x2a, 0x2a, 0x26, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // N
    [0x00, 0x1c, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // O
    [0x00, 0x3c, 0x22, 0x22, 0x22, 0x3c, 0x20, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00], // P
    [0x00, 0x1c, 0x22, 0x22, 0x22, 0x22, 0x22, 0x2a, 0x24, 0x1a, 0x00, 0x00, 0x00], // Q
    [0x00, 0x3c, 0x22, 0x22, 0x22, 0x3c, 0x28, 0x24, 0x22, 0x22, 0x00, 0x00, 0x00], // R
    [0x00, 0x1e, 0x20, 0x20, 0x20, 0x1c, 0x02, 0x02, 0x02, 0x3c, 0x00, 0x00, 0x00], // S
    [0x00, 0x3e, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, 0x00], // T
    [0x00, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // U
    [0x00, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x14, 0x14, 0x08, 0x00, 0x00, 0x00], // V
    [0x00, 0x22, 0x22, 0x22, 0x22, 0x2a, 0x2a, 0x2a, 0x36, 0x22, 0x00, 0x00, 0x00], // W
    [0x00, 0x22, 0x22, 0x14, 0x14, 0x08, 0x14, 0x14, 0x22, 0x22, 0x00, 0x00, 0x00], // X
    [0x00, 0x22, 0x22, 0x14, 0x14, 0x08, 0x08, 0x08, 0x08, 0x08, 0x00, 0x00, 0x00], // Y
    [0x00, 0x3e, 0x02, 0x04, 0x04, 0x08, 0x10, 0x10, 0x20, 0x3e, 0x00, 0x00, 0x00], // Z
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // [
    [0x40, 0x40, 0x20, 0x20, 0x10, 0x10, 0x08, 0x04, 0x04, 0x02, 0x02, 0x01, 0x01], // barra invertida
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ]
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ^
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x7f, 0x00], // _
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // `
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x02, 0x1e, 0x22, 0x1e, 0x00, 0x00, 0x00], // a
    [0x00, 0x20, 0x20, 0x20, 0x20, 0x3c, 0x22, 0x22, 0x22, 0x3c, 0x00, 0x00, 0x00], // b
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x20, 0x20, 0x20, 0x1c, 0x00, 0x00, 0x00], // c
    [0x00, 0x02, 0x02, 0x02, 0x02, 0x1e, 0x22, 0x22, 0x22, 0x1e, 0x00, 0x00, 0x00], // d
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x22, 0x3e, 0x20, 0x1c, 0x00, 0x00, 0x00], // e
    [0x00, 0x0c, 0x10, 0x10, 0x10, 0x3c, 0x10, 0x10, 0x10, 0x10, 0x00, 0x00, 0x00], // f
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x1c, 0x00], // g
    [0x00, 0x20, 0x20, 0x20, 0x20, 0x3c, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // h
    [0x00, 0x00, 0x00, 0x08, 0x00, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, 0x00, 0x00], // i
    [0x00, 0x00, 0x00, 0x04, 0x00, 0x04, 0x04, 0x04, 0x24, 0x18, 0x00, 0x00, 0x00], // j
    [0x00, 0x20, 0x20, 0x20, 0x24, 0x28, 0x30, 0x28, 0x24, 0x22, 0x00, 0x00, 0x00], // k
    [0x00, 0x18, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x1c, 0x00, 0x00, 0x00], // l
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x34, 0x2a, 0x2a, 0x22, 0x22, 0x00, 0x00, 0x00], // m
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3c, 0x22, 0x22, 0x22, 0x22, 0x00, 0x00, 0x00], // n
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x22, 0x22, 0x22, 0x1c, 0x00, 0x00, 0x00], // o
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3c, 0x22, 0x22, 0x3c, 0x20, 0x20, 0x20, 0x00], // p
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x02, 0x00], // q
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x30, 0x20, 0x20, 0x20, 0x00, 0x00, 0x00], // r
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x1e, 0x20, 0x1c, 0x02, 0x3c, 0x00, 0x00, 0x00], // s
    [0x00, 0x10, 0x10, 0x10, 0x10, 0x3c, 0x10, 0x10, 0x10, 0x0c, 0x00, 0x00, 0x00], // t
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x22, 0x1e, 0x00, 0x00, 0x00], // u
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x14, 0x08, 0x00, 0x00, 0x00], // v
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x2a, 0x2a, 0x2a, 0x14, 0x00, 0x00, 0x00], // w
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x14, 0x08, 0x14, 0x22, 0x00, 0x00, 0x00], // x
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x22, 0x22, 0x22, 0x1e, 0x02, 0x02, 0x1c, 0x00], // y
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x3e, 0x04, 0x08, 0x10, 0x3e, 0x00, 0x00, 0x00], // z
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // {
    [0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08, 0x08], // |
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // }
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x31, 0x4a, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00], // ~
];

/// Célula do atlas: o glifo + 1 pixel de folga, pra que o sampler não puxe a coluna do vizinho.
pub(crate) const CELULA_L: usize = LARGURA_GLIFO + 1;
pub(crate) const CELULA_A: usize = ALTURA_GLIFO + 1;
/// O atlas tem três faixas de células, nesta ordem:
///   0..95    a `FONTE` acima, INTOCADA — é o que o splash desenha, e o splash não pode mudar um pixel;
///   95..190  o ASCII de TEXTO: a `FONTE` com as formas de texto de `/ \ | * ~` e os 16 sinais que a
///            marca nunca precisou (`" # $ & + ; < = > @ [ ] ^ \` { }`), que lá são células vazias;
///   190..286 o Latin-1 de texto (0xA0..=0xFF): acentos do português compostos e os sinais comuns.
/// Mais uma célula branca opaca no fim: dela saem retângulos e linhas, e assim texto e formas cabem
/// num único draw call, com uma textura só.
///
/// POR QUE DUAS FORMAS DO MESMO CARACTERE: a fonte foi desenhada pra MARCA isométrica do splash, e lá
/// `/` e `|` ocupam as 13 linhas da célula (pra arte emendar na vertical) e `*` é um ponto separador
/// (`CARREGANDO 42% * etapa`). Em texto corrido isso sai errado — `r6/scripts` com uma barra encostando
/// nas linhas vizinhas. Consertar na `FONTE` mudaria o splash; por isso o texto tem as suas.
pub(crate) const INICIO_TEXTO_ASCII: usize = 95;
pub(crate) const INICIO_TEXTO_LATIN1: usize = INICIO_TEXTO_ASCII + 95;
pub(crate) const CELULA_BRANCA: usize = INICIO_TEXTO_LATIN1 + 96;
pub(crate) const ATLAS_L: usize = CELULA_L * (CELULA_BRANCA + 1);
pub(crate) const ATLAS_A: usize = CELULA_A;

/// Um glifo escrito como grade legível: 13 linhas de 7 colunas, `#` aceso. Hex à mão é onde erro de
/// pixel passa sem ninguém ver; em grade, dá pra revisar olhando.
const fn grade(l: [&str; 13]) -> [u8; ALTURA_GLIFO] {
    let mut out = [0u8; ALTURA_GLIFO];
    let mut r = 0;
    while r < ALTURA_GLIFO {
        let b = l[r].as_bytes();
        let mut c = 0;
        while c < LARGURA_GLIFO {
            if b[c] == b'#' {
                out[r] |= 1 << (LARGURA_GLIFO - 1 - c);
            }
            c += 1;
        }
        r += 1;
    }
    out
}

const V: &str = ".......";

/// Os caracteres ASCII que o TEXTO desenha diferente da `FONTE` (ou que a `FONTE` deixou vazios).
/// Métrica da fonte: linha 0 livre, maiúscula e sinal alto nas linhas 1–9 (base na 9), minúscula nas
/// 5–9, descendente até a 11, traço de 1 px nas colunas 1–5.
static GLIFOS_DE_TEXTO: [(char, [u8; ALTURA_GLIFO]); 21] = [
    ('"', grade([V, "..#.#..", "..#.#..", V, V, V, V, V, V, V, V, V, V])),
    ('#', grade([V, V, "..#.#..", "..#.#..", ".#####.", "..#.#..", ".#####.", "..#.#..", "..#.#..", V, V, V, V])),
    ('$', grade([V, "...#...", "..####.", ".#.#...", ".#.#...", "..###..", "...#.#.", "...#.#.", ".####..", "...#...", V, V, V])),
    ('&', grade([V, "..##...", ".#..#..", ".#..#..", ".#.#...", "..#....", ".#.#.#.", ".#..#..", ".#..#..", "..##.#.", V, V, V])),
    ('*', grade([V, V, "...#...", ".#.#.#.", "..###..", ".#.#.#.", "...#...", V, V, V, V, V, V])),
    ('+', grade([V, V, V, "...#...", "...#...", ".#####.", "...#...", "...#...", V, V, V, V, V])),
    ('/', grade([V, ".....#.", ".....#.", "....#..", "....#..", "...#...", "...#...", "..#....", "..#....", ".#.....", ".#.....", V, V])),
    (';', grade([V, V, V, "...#...", V, V, V, V, "...#...", "...#...", ".#.....", V, V])),
    ('<', grade([V, V, "....#..", "...#...", "..#....", ".#.....", "..#....", "...#...", "....#..", V, V, V, V])),
    ('=', grade([V, V, V, V, ".#####.", V, ".#####.", V, V, V, V, V, V])),
    ('>', grade([V, V, "..#....", "...#...", "....#..", ".....#.", "....#..", "...#...", "..#....", V, V, V, V])),
    ('@', grade([V, "..###..", ".#...#.", ".#.###.", ".#.#.#.", ".#.#.#.", ".#.###.", ".#.....", ".#...#.", "..###..", V, V, V])),
    ('[', grade([V, "..###..", "..#....", "..#....", "..#....", "..#....", "..#....", "..#....", "..#....", "..#....", "..###..", V, V])),
    ('\\', grade([V, ".#.....", ".#.....", "..#....", "..#....", "...#...", "...#...", "....#..", "....#..", ".....#.", ".....#.", V, V])),
    (']', grade([V, "..###..", "....#..", "....#..", "....#..", "....#..", "....#..", "....#..", "....#..", "....#..", "..###..", V, V])),
    ('^', grade([V, "...#...", "..#.#..", ".#...#.", V, V, V, V, V, V, V, V, V])),
    ('`', grade([V, "..#....", "...#...", V, V, V, V, V, V, V, V, V, V])),
    ('{', grade([V, "...##..", "..#....", "..#....", "..#....", ".#.....", "..#....", "..#....", "..#....", "..#....", "...##..", V, V])),
    ('|', grade([V, "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", V, V])),
    ('}', grade([V, ".##....", "...#...", "...#...", "...#...", "....#..", "...#...", "...#...", "...#...", "...#...", ".##....", V, V])),
    ('~', grade([V, V, V, V, V, "..#....", ".#.#.#.", "....#..", V, V, V, V, V])),
];

/// Sinais do Latin-1 desenhados à parte (não são letra + acento). Os raros que faltam aqui saem `?`,
/// que é o que a referência do terminal mostra para tudo fora do Latin-1.
static SINAIS_LATIN1: [(u32, [u8; ALTURA_GLIFO]); 15] = [
    // ª e º: ordinais do português (`nº`, `1ª`) — `a`/`o` pequenos no alto, sublinhados.
    (0xAA, grade([V, "...##..", "..#.#..", "...###.", V, "..####.", V, V, V, V, V, V, V])),
    (0xBA, grade([V, "...#...", "..#.#..", "...#...", V, "..###..", V, V, V, V, V, V, V])),
    (0xA0, grade([V, V, V, V, V, V, V, V, V, V, V, V, V])),
    (0xA1, grade([V, V, V, "...#...", V, "...#...", "...#...", "...#...", "...#...", "...#...", "...#...", V, V])),
    (0xA8, grade([V, V, "..#.#..", V, V, V, V, V, V, V, V, V, V])),
    (0xAB, grade([V, V, V, V, "...#.#.", "..#.#..", ".#.#...", "..#.#..", "...#.#.", V, V, V, V])),
    (0xAD, grade([V, V, V, V, V, ".#####.", V, V, V, V, V, V, V])),
    (0xB0, grade([V, "...#...", "..#.#..", "...#...", V, V, V, V, V, V, V, V, V])),
    (0xB1, grade([V, V, "...#...", "...#...", ".#####.", "...#...", "...#...", V, ".#####.", V, V, V, V])),
    (0xB4, grade([V, "....#..", "...#...", V, V, V, V, V, V, V, V, V, V])),
    (0xB7, grade([V, V, V, V, V, V, "...#...", V, V, V, V, V, V])),
    (0xB8, grade([V, V, V, V, V, V, V, V, V, V, "...#...", "..##...", V])),
    (0xBB, grade([V, V, V, V, ".#.#...", "..#.#..", "...#.#.", "..#.#..", ".#.#...", V, V, V, V])),
    (0xD7, grade([V, V, V, V, ".#...#.", "..#.#..", "...#...", "..#.#..", ".#...#.", V, V, V, V])),
    (0xF7, grade([V, V, V, "...#...", V, ".#####.", V, "...#...", V, V, V, V, V])),
];

#[derive(Clone, Copy)]
enum Acento {
    Agudo,
    Grave,
    Circunflexo,
    Til,
    Trema,
    Anel,
    Cedilha,
}

/// Letra acentuada do Latin-1 → (letra base, acento). Fora da tabela, `None`.
fn decompor(c: u32) -> Option<(char, Acento)> {
    use Acento::*;
    let (base, acento) = match c {
        0xC0 => ('A', Grave), 0xC1 => ('A', Agudo), 0xC2 => ('A', Circunflexo), 0xC3 => ('A', Til), 0xC4 => ('A', Trema), 0xC5 => ('A', Anel),
        0xC7 => ('C', Cedilha),
        0xC8 => ('E', Grave), 0xC9 => ('E', Agudo), 0xCA => ('E', Circunflexo), 0xCB => ('E', Trema),
        0xCC => ('I', Grave), 0xCD => ('I', Agudo), 0xCE => ('I', Circunflexo), 0xCF => ('I', Trema),
        0xD1 => ('N', Til),
        0xD2 => ('O', Grave), 0xD3 => ('O', Agudo), 0xD4 => ('O', Circunflexo), 0xD5 => ('O', Til), 0xD6 => ('O', Trema),
        0xD9 => ('U', Grave), 0xDA => ('U', Agudo), 0xDB => ('U', Circunflexo), 0xDC => ('U', Trema),
        0xDD => ('Y', Agudo),
        0xE0 => ('a', Grave), 0xE1 => ('a', Agudo), 0xE2 => ('a', Circunflexo), 0xE3 => ('a', Til), 0xE4 => ('a', Trema), 0xE5 => ('a', Anel),
        0xE7 => ('c', Cedilha),
        0xE8 => ('e', Grave), 0xE9 => ('e', Agudo), 0xEA => ('e', Circunflexo), 0xEB => ('e', Trema),
        0xEC => ('i', Grave), 0xED => ('i', Agudo), 0xEE => ('i', Circunflexo), 0xEF => ('i', Trema),
        0xF1 => ('n', Til),
        0xF2 => ('o', Grave), 0xF3 => ('o', Agudo), 0xF4 => ('o', Circunflexo), 0xF5 => ('o', Til), 0xF6 => ('o', Trema),
        0xF9 => ('u', Grave), 0xFA => ('u', Agudo), 0xFB => ('u', Circunflexo), 0xFC => ('u', Trema),
        0xFD => ('y', Agudo), 0xFF => ('y', Trema),
        _ => return None,
    };
    Some((base, acento))
}

/// O acento como linhas a acender. Minúscula tem as linhas 1–3 livres acima da altura-x (a 4 fica
/// de respiro); maiúscula começa na linha 1, então só sobra a linha 0 — por isso a forma achatada.
/// A cedilha vai embaixo, nas linhas 10–11, igual nos dois.
fn marca_do_acento(a: Acento, maiuscula: bool) -> [(usize, u8); 3] {
    use Acento::*;
    const NADA: (usize, u8) = (0, 0);
    if let Cedilha = a {
        return [(10, 0x08), (11, 0x18), NADA];
    }
    if maiuscula {
        let linha0 = match a {
            Agudo => 0x04,
            Grave => 0x10,
            Circunflexo => 0x14,
            Til => 0x2A,
            Trema => 0x22,
            Anel => 0x08,
            Cedilha => 0,
        };
        return [(0, linha0), NADA, NADA];
    }
    match a {
        Agudo => [(2, 0x04), (3, 0x08), NADA],
        Grave => [(2, 0x10), (3, 0x08), NADA],
        Circunflexo => [(2, 0x08), (3, 0x14), NADA],
        Til => [(2, 0x1A), (3, 0x24), NADA],
        Trema => [(3, 0x14), NADA, NADA],
        Anel => [(1, 0x08), (2, 0x14), (3, 0x08)],
        Cedilha => [NADA, NADA, NADA],
    }
}

/// O glifo ASCII como o TEXTO desenha: a forma de texto se houver uma, senão a da `FONTE`.
pub(crate) fn glifo_ascii_de_texto(c: char) -> [u8; ALTURA_GLIFO] {
    if let Some((_, g)) = GLIFOS_DE_TEXTO.iter().find(|(k, _)| *k == c) {
        return *g;
    }
    let i = (c as usize).wrapping_sub(PRIMEIRO_CODIGO);
    if i < FONTE.len() {
        FONTE[i]
    } else {
        FONTE['?' as usize - PRIMEIRO_CODIGO]
    }
}

/// O glifo Latin-1 (0xA0..=0xFF): sinal desenhado, letra composta com acento, ou `?`.
pub(crate) fn glifo_latin1(c: u32) -> [u8; ALTURA_GLIFO] {
    if let Some((_, g)) = SINAIS_LATIN1.iter().find(|(k, _)| *k == c) {
        return *g;
    }
    if c == 0xB5 {
        // µ: o `u` com a haste descendo pela esquerda.
        let mut g = glifo_ascii_de_texto('u');
        g[10] |= 0x20;
        g[11] |= 0x20;
        return g;
    }
    let Some((base, acento)) = decompor(c) else {
        return glifo_ascii_de_texto('?');
    };
    let mut g = glifo_ascii_de_texto(base);
    if base == 'i' {
        // Sem o pingo: o acento ocupa o lugar dele, e os dois juntos viram um borrão.
        for linha in g.iter_mut().take(5) {
            *linha = 0;
        }
    }
    for (linha, bits) in marca_do_acento(acento, base.is_ascii_uppercase()) {
        g[linha] |= bits;
    }
    g
}

/// Em que célula do atlas o TEXTO acha o caractere `c`. Fora do ASCII e do Latin-1, `?` — o mesmo que
/// a referência do terminal mostra.
pub(crate) fn celula_de_texto(c: char) -> usize {
    let u = c as u32;
    if (0x20..=0x7E).contains(&u) {
        INICIO_TEXTO_ASCII + (u as usize - PRIMEIRO_CODIGO)
    } else if (0xA0..=0xFF).contains(&u) {
        INICIO_TEXTO_LATIN1 + (u as usize - 0xA0)
    } else {
        INICIO_TEXTO_ASCII + ('?' as usize - PRIMEIRO_CODIGO)
    }
}

fn gravar_glifo(px: &mut [u8], celula: usize, glifo: &[u8; ALTURA_GLIFO]) {
    for (linha, bits) in glifo.iter().enumerate() {
        for coluna in 0..LARGURA_GLIFO {
            if bits & (1 << (LARGURA_GLIFO - 1 - coluna)) == 0 {
                continue;
            }
            let o = ((linha * ATLAS_L) + celula * CELULA_L + coluna) * 4;
            px[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
}

/// RGBA8 direto: o shader faz `cor_do_vértice * textura`, então o atlas é branco com o alfa do glifo.
pub(crate) fn montar_atlas() -> Vec<u8> {
    let mut px = vec![0u8; ATLAS_L * ATLAS_A * 4];
    for (i, glifo) in FONTE.iter().enumerate() {
        gravar_glifo(&mut px, i, glifo);
    }
    for i in 0..95 {
        let c = char::from_u32((PRIMEIRO_CODIGO + i) as u32).unwrap_or('?');
        gravar_glifo(&mut px, INICIO_TEXTO_ASCII + i, &glifo_ascii_de_texto(c));
    }
    for i in 0..96u32 {
        gravar_glifo(&mut px, INICIO_TEXTO_LATIN1 + i as usize, &glifo_latin1(0xA0 + i));
    }
    for linha in 0..ATLAS_A {
        for coluna in 0..CELULA_L {
            let o = ((linha * ATLAS_L) + CELULA_BRANCA * CELULA_L + coluna) * 4;
            px[o..o + 4].copy_from_slice(&[255, 255, 255, 255]);
        }
    }
    px
}

/// Mesmo formato de vértice que o pipeline do overlay já espera: posição em pixels, uv, cor RGBA8.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Vertice {
    pub(crate) pos: [f32; 2],
    pub(crate) uv: [f32; 2],
    pub(crate) cor: [u8; 4],
}

/// Polígono convexo recortado pelo retângulo `[x0, y0, x1, y1]` (Sutherland–Hodgman, um lado por vez). Vazio se ficou todo fora.
pub(crate) fn recortar_poligono(pontos: &[[f32; 2]], r: [f32; 4]) -> Vec<[f32; 2]> {
    let mut atual: Vec<[f32; 2]> = pontos.to_vec();
    // Cada lado: (eixo, limite, fica do lado de dentro se o valor for >= limite).
    for (eixo, limite, maior) in [(0, r[0], true), (0, r[2], false), (1, r[1], true), (1, r[3], false)] {
        let dentro = |p: [f32; 2]| if maior { p[eixo] >= limite } else { p[eixo] <= limite };
        let entrada = std::mem::take(&mut atual);
        for i in 0..entrada.len() {
            let (p, q) = (entrada[i], entrada[(i + 1) % entrada.len()]);
            let (dp, dq) = (dentro(p), dentro(q));
            if dp {
                atual.push(p);
            }
            if dp != dq {
                let t = (limite - p[eixo]) / (q[eixo] - p[eixo]);
                atual.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
            }
        }
        if atual.is_empty() {
            break;
        }
    }
    atual
}

/// Malha em construção. Dois lotes porque são duas texturas: o atlas (texto e formas, amostragem
/// nítida) e a arte do usuário (amostragem suave, porque é escalada pra caber na tela).
#[derive(Default)]
pub(crate) struct Malha {
    pub(crate) vertices: Vec<Vertice>,
    pub(crate) indices: Vec<u16>,
    /// Retângulo [x0, y0, x1, y1] fora do qual nada é desenhado. O splash nunca liga isto (`None` é o
    /// comportamento de sempre); o terminal liga dentro das áreas roláveis, pra o log não vazar por
    /// cima do resto.
    pub(crate) recorte: Option<[f32; 4]>,
}

impl Malha {
    pub(crate) fn quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, uv: [f32; 4], cor: [u8; 4]) {
        // Quad alinhado aos eixos: recortar é encolher o retângulo e puxar a uv na mesma proporção,
        // o que é exato com amostragem nítida.
        let (x0, y0, x1, y1, uv) = match self.recorte {
            None => (x0, y0, x1, y1, uv),
            Some([cx0, cy0, cx1, cy1]) => {
                let (nx0, ny0, nx1, ny1) = (x0.max(cx0), y0.max(cy0), x1.min(cx1), y1.min(cy1));
                if nx0 >= nx1 || ny0 >= ny1 {
                    return;
                }
                let fu = |x: f32| uv[0] + (x - x0) / (x1 - x0) * (uv[2] - uv[0]);
                let fv = |y: f32| uv[1] + (y - y0) / (y1 - y0) * (uv[3] - uv[1]);
                (nx0, ny0, nx1, ny1, [fu(nx0), fv(ny0), fu(nx1), fv(ny1)])
            }
        };
        let base = self.vertices.len() as u16;
        // 65532 vértices = 16383 quads; a tela inteira fica em ~4 mil. Estourar truncaria a malha
        // com índices errados, então o quad é descartado antes disso.
        if base > u16::MAX - 4 {
            return;
        }
        self.vertices.push(Vertice { pos: [x0, y0], uv: [uv[0], uv[1]], cor });
        self.vertices.push(Vertice { pos: [x1, y0], uv: [uv[2], uv[1]], cor });
        self.vertices.push(Vertice { pos: [x1, y1], uv: [uv[2], uv[3]], cor });
        self.vertices.push(Vertice { pos: [x0, y1], uv: [uv[0], uv[3]], cor });
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    /// Retângulo cheio, tirado da célula branca do atlas.
    pub(crate) fn retangulo(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, cor: [u8; 4]) {
        let u = (CELULA_BRANCA * CELULA_L) as f32 + 2.0;
        let uv = [u / ATLAS_L as f32, 2.0 / ATLAS_A as f32, (u + 1.0) / ATLAS_L as f32, 3.0 / ATLAS_A as f32];
        self.quad(x0, y0, x1, y1, uv, cor);
    }

    /// Triângulo cheio, da célula branca (o ▼ de recolher, a alça de redimensionar, o ✓ da caixa de marcar). Passa pelo recorte como
    /// os retângulos: o ✓ de uma linha rolada para fora da área não aparece por cima do resto.
    pub(crate) fn triangulo(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], cor: [u8; 4]) {
        let poligono = match self.recorte {
            Some(r) => recortar_poligono(&[a, b, c], r),
            None => vec![a, b, c],
        };
        if poligono.len() < 3 {
            return;
        }
        let base = self.vertices.len() as u16;
        if (base as usize) + poligono.len() > u16::MAX as usize {
            return;
        }
        let u = ((CELULA_BRANCA * CELULA_L) as f32 + 2.5) / ATLAS_L as f32;
        let v = 2.5 / ATLAS_A as f32;
        for p in &poligono {
            self.vertices.push(Vertice { pos: *p, uv: [u, v], cor });
        }
        for i in 1..poligono.len() as u16 - 1 {
            self.indices.extend_from_slice(&[base, base + i, base + i + 1]);
        }
    }

    /// Linha reta (só horizontal ou vertical — esta tela não tem diagonal fora da marca).
    pub(crate) fn linha(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, espessura: f32, cor: [u8; 4]) {
        let m = espessura * 0.5;
        if (y1 - y0).abs() < 0.5 {
            self.retangulo(x0.min(x1), y0 - m, x0.max(x1), y0 + m, cor);
        } else {
            self.retangulo(x0 - m, y0.min(y1), x0 + m, y0.max(y1), cor);
        }
    }

    /// Escreve `texto` com o canto superior esquerdo em (x, y). `escala` multiplica o glifo de 7×13.
    pub(crate) fn texto(&mut self, x: f32, y: f32, escala: f32, cor: [u8; 4], texto: &str) {
        let (mut cx, mut cy) = (x, y);
        let passo = CELULA_L as f32 * escala;
        for c in texto.chars() {
            if c == '\n' {
                cx = x;
                cy += CELULA_A as f32 * escala;
                continue;
            }
            let codigo = c as usize;
            // ASCII sai das células da `FONTE` (as da marca); letra do Latin-1 (`ã` em "iniciando a sessão") sai da faixa de texto
            // Latin-1, que é a mesma letra da `FONTE` com o acento composto por cima. Fora disso a célula fica em branco.
            let celula = if (PRIMEIRO_CODIGO..PRIMEIRO_CODIGO + 95).contains(&codigo) {
                let i = codigo - PRIMEIRO_CODIGO;
                FONTE[i].iter().any(|b| *b != 0).then_some(i)
            } else if (0xA1..=0xFF).contains(&codigo) {
                Some(celula_de_texto(c))
            } else {
                None
            };
            if let Some(i) = celula {
                let u0 = (i * CELULA_L) as f32 / ATLAS_L as f32;
                let u1 = (i * CELULA_L + LARGURA_GLIFO) as f32 / ATLAS_L as f32;
                let v1 = ALTURA_GLIFO as f32 / ATLAS_A as f32;
                self.quad(
                    cx,
                    cy,
                    cx + LARGURA_GLIFO as f32 * escala,
                    cy + ALTURA_GLIFO as f32 * escala,
                    [u0, 0.0, u1, v1],
                    cor,
                );
            }
            cx += passo;
        }
    }

    /// Texto corrido do terminal: usa as faixas de TEXTO do atlas (formas de texto e Latin-1), nunca
    /// as células da marca. Um caractere fora do Latin-1 vira `?` e ocupa uma célula, como na referência.
    ///
    /// AVANÇO 7 e LINHA 13, não os 8/14 do splash: é a métrica da ProggyClean da referência do
    /// terminal (medido nela: 39 caracteres de título em ~273 px). Com 8 o texto sairia 14%
    /// mais largo e o log escorregaria 1 px por linha. As letras já têm as colunas 0 e 6 vazias, então
    /// 7 é o avanço natural da fonte; o splash fica com 8 porque a marca precisa daquele respiro.
    pub(crate) fn texto_de_terminal(&mut self, x: f32, y: f32, escala: f32, cor: [u8; 4], texto: &str) {
        let (mut cx, mut cy) = (x, y);
        let passo = LARGURA_GLIFO as f32 * escala;
        for c in texto.chars() {
            if c == '\n' {
                cx = x;
                cy += ALTURA_GLIFO as f32 * escala;
                continue;
            }
            // O `\r` de um fim de linha CRLF não ocupa lugar nem vira `?`, como na referência: o texto dos plugins chega cru.
            if c == '\r' {
                continue;
            }
            if c != ' ' {
                let i = celula_de_texto(c);
                let u0 = (i * CELULA_L) as f32 / ATLAS_L as f32;
                let u1 = (i * CELULA_L + LARGURA_GLIFO) as f32 / ATLAS_L as f32;
                let v1 = ALTURA_GLIFO as f32 / ATLAS_A as f32;
                self.quad(
                    cx,
                    cy,
                    cx + LARGURA_GLIFO as f32 * escala,
                    cy + ALTURA_GLIFO as f32 * escala,
                    [u0, 0.0, u1, v1],
                    cor,
                );
            }
            cx += passo;
        }
    }
}

/// Largura que `texto` ocupa — a fonte é monoespaçada, então é conta, não medição.
pub(crate) fn largura_do_texto(texto: &str, escala: f32) -> f32 {
    let maior = texto.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    maior as f32 * CELULA_L as f32 * escala
}
pub(crate) fn altura_do_texto(texto: &str, escala: f32) -> f32 {
    texto.lines().count().max(1) as f32 * CELULA_A as f32 * escala
}

/// Maior escala INTEIRA em que `texto` ainda cabe em `largura_alvo`. Fonte de bitmap só fica nítida
/// em múltiplo inteiro do pixel (em 0,72× o `nearest` come linhas inteiras do glifo), e sem o teto
/// pela largura o layout depende da resolução — numa prévia de 1280 o subtítulo saiu com 1476 px.
pub(crate) fn escala_que_cabe(texto: &str, largura_alvo: f32) -> f32 {
    let colunas = texto.lines().map(|l| l.chars().count()).max().unwrap_or(1).max(1) as f32;
    (largura_alvo / (colunas * CELULA_L as f32)).floor().max(1.0)
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn triangulo_respeita_o_recorte() {
        let t = [[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]];
        assert_eq!(recortar_poligono(&t, [-5.0, -5.0, 20.0, 20.0]).len(), 3, "todo dentro fica igual");
        assert!(recortar_poligono(&t, [50.0, 50.0, 60.0, 60.0]).is_empty(), "todo fora some");
        let metade = recortar_poligono(&t, [0.0, 0.0, 5.0, 20.0]);
        assert!(metade.iter().all(|p| p[0] <= 5.0 + 1e-4), "nada passa da borda do recorte");
        let mut m = Malha { recorte: Some([50.0, 50.0, 60.0, 60.0]), ..Default::default() };
        m.triangulo(t[0], t[1], t[2], [255; 4]);
        assert!(m.indices.is_empty(), "fora do recorte não desenha");
    }

    #[test]
    fn celulas_da_marca_nao_mudaram_de_lugar() {
        // O splash desenha pelas células 0..95. Se elas andarem, a marca muda.
        assert_eq!(INICIO_TEXTO_ASCII, 95);
        assert!(CELULA_BRANCA > INICIO_TEXTO_LATIN1 + 95);
    }

    #[test]
    fn todo_caractere_de_texto_cai_numa_celula_do_atlas() {
        for c in ['a', 'Z', '~', '/', 'ã', 'Ç', 'ÿ', '—', '⚠', '\u{1F600}'] {
            assert!(celula_de_texto(c) < CELULA_BRANCA, "{c:?} saiu do atlas");
        }
    }

    #[test]
    fn fora_do_latin1_vira_interrogacao() {
        let q = celula_de_texto('?');
        assert_eq!(celula_de_texto('—'), q);
        assert_eq!(celula_de_texto('⚠'), q);
    }

    #[test]
    fn os_16_sinais_vazios_da_marca_tem_desenho_no_texto() {
        for c in "\"#$&+;<=>@[]^`{}".chars() {
            assert!(glifo_ascii_de_texto(c).iter().any(|b| *b != 0), "{c:?} continua vazio no texto");
        }
    }

    #[test]
    fn letra_acentuada_tem_mais_pixel_que_a_base() {
        for (acentuada, base) in [(0xE3u32, 'a'), (0xE9, 'e'), (0xF3, 'o'), (0xE7, 'c'), (0xC9, 'E')] {
            let conta = |g: [u8; ALTURA_GLIFO]| g.iter().map(|b| b.count_ones()).sum::<u32>();
            assert!(conta(glifo_latin1(acentuada)) > conta(glifo_ascii_de_texto(base)), "{acentuada:#x}");
        }
    }

    #[test]
    fn i_acentuado_perde_o_pingo() {
        let g = glifo_latin1(0xED);
        // o pingo do `i` fica na linha 3 da fonte; com acento, a linha 3 só pode ter o acento
        assert_eq!(g[3], 0x08, "a linha 3 tinha que ser só o traço do agudo");
    }

    /// Folha com todos os glifos de TEXTO, ampliada, pra revisar o desenho olhando.
    /// Rodar: `BWMS_PROVA_VISUAL=<pasta> cargo test --features devtools,capture folha_de_glifos -- --ignored`
    #[test]
    #[ignore]
    fn folha_de_glifos() {
        use crate::terminal::rasterizador_de_prova::*;
        let atlas = montar_atlas();
        let tex = Textura { largura: ATLAS_L, altura: ATLAS_A, rgba: &atlas, nitida: true };
        let linhas: [String; 5] = [
            (0x20u8..0x50).map(|b| b as char).collect(),
            (0x50u8..0x7F).map(|b| b as char).collect(),
            (0xA0u32..0xD0).filter_map(char::from_u32).collect(),
            (0xD0u32..=0xFF).filter_map(char::from_u32).collect(),
            "não é à ação sessão inventário você ÇÃO É Ã — ⚠".to_string(),
        ];
        let escala = 3.0;
        let (l, a) = (48 * CELULA_L * 3 + 20, 5 * CELULA_A * 3 + 30);
        let mut m = Malha::default();
        for (i, t) in linhas.iter().enumerate() {
            m.texto_de_terminal(10.0, 10.0 + i as f32 * (CELULA_A as f32 * escala + 4.0), escala, [230, 230, 230, 255], t);
        }
        let pontos: Vec<Ponto> = m.vertices.iter().map(|v| Ponto { pos: v.pos, uv: v.uv, cor: v.cor }).collect();
        let mut q = Quadro::novo(l, a, [0.06, 0.06, 0.07, 1.0]);
        lista(&mut q, &pontos, &m.indices, &tex, [0.0, 0.0, l as f32, a as f32]);
        let dir = std::env::var("BWMS_PROVA_VISUAL").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        gravar_bmp(&dir.join("folha-de-glifos.bmp"), &q).expect("bmp");
    }
}
