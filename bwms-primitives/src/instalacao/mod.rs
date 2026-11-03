//! Motor de mods do BWMS na lib-folha que o runtime (`cp77-console`) e a CLI (`bwms`) usam: o linker só leva para o dylib o que o runtime
//! chama.
//!
//! - `xl`: o `.xl` do ArchiveXL no modelo tipado; o YAML é lido pelo `tweakxl::yaml`, o mesmo dos tweaks do TweakXL.
//! - `tabelas_de_runtime`: as tabelas `red4ext/bwms-*.txt` que o runtime lê, emitidas dos `.xl`.
//! - `localizacao`: os códigos de idioma que o `.xl` pode declarar.
//! - `xl_no_jogo`: a parte dos `.xl` da `Mods/` que o próprio jogo aplica, em `.json` na pasta que ele lê.
//! - `apply`: a leitura da pasta `Mods/` e a gravação das tabelas de runtime com os `.xl` dela, a cada abertura do jogo.
//! - `classify`: analisa um mod solto (arquivos, tipo, compat, dependências, `.xl`) para o `bwms classify`.
//! - `theme`: categoriza por tema (Roupas/Veículos/LUT/...) para o `bwms classify`/`bwms suggest`, e lê o estado da biblioteca antiga.
//! - `migracao`: leva para a `Mods/` os mods ligados da biblioteca da 0.1.2–0.1.4 e tira do jogo o que ela pôs lá (`bwms-helper migrar`).

pub mod apply;
pub mod classify;
pub mod localizacao;
pub mod migracao;
pub mod tabelas_de_runtime;
pub mod theme;
pub mod xl;
pub mod xl_no_jogo;
