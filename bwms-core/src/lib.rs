//! bwms-core — núcleo compartilhado do Black Wall Mod System.
//!
//! Quatro módulos, dependência em cadeia (cada um só usa o anterior + `std`):
//!   xl  →  classify  →  theme  →  apply
//!
//! - `xl`: parser do `.xl` do ArchiveXL (subconjunto YAML → modelo tipado). Usado pelo
//!   `classify` p/ entender o que um mod ArchiveXL faz (factories/patch/link/...).
//! - `classify`: analisa um mod solto (arquivos, tipo, compat, dependências, .xl).
//! - `theme`: categoriza por tema (Roupas/Veículos/LUT/...), sugere, e
//!   serializa o estado (`bwms-mods.json`) — ativar/desativar/favoritar.
//! - `apply`: reconcilia a pasta de staging e sincroniza os `.archive` ativos
//!   pro `archive/Mac/content` do jogo (prefixo `basegame_zzbwms_`), nunca
//!   tocando o jogo-base; remoção é segura (só apaga o que prefixamos).
//!
//! Isto é a lógica que ANTES vivia só no `mac-mod-manager`. Extraída pra cá pra
//! a CLI `bwms` e o runtime (dylib, aba LUT) usarem a MESMA implementação — em
//! vez de cada ferramenta reimplementar/shellar a outra.

pub mod apply;
pub mod apply_xl;
pub mod classify;
pub mod nexus;
pub mod theme;
pub mod xl;
