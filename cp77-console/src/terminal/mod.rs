//! Terminal: a UI própria do BWMS por cima do quadro do jogo (overlay, widgets em modo imediato, malha de desenho, catálogo de comandos).

#![allow(dead_code)]

/// O que cada comando do terminal faz, como dado (a ajuda sai daqui).
pub(crate) mod catalogo;

/// Primitivas de desenho compartilhadas pelo splash, pelo terminal e pelo overlay: fonte, atlas e malha de vértices (ver o módulo).
pub(crate) mod malha_de_desenho;

/// Atalho que abre e fecha o terminal, gravado pela aba Mods no `bwms.toml`.
pub(crate) mod atalho;

/// Escala inteira do terminal pela resolução do jogo, com a escolha da aba Mods gravada no `bwms.toml`.
pub(crate) mod escala;

/// Edição de texto de uma linha do terminal (cursor, seleção, palavras, área de transferência).
pub(crate) mod edicao;

pub(crate) mod imediato;

pub(crate) mod overlay;

#[cfg(test)]
pub(crate) mod rasterizador_de_prova;

