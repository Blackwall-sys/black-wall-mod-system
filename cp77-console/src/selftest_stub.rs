//! Stub do `selftest` para o build PÚBLICO.
//!
//! O `selftest.rs` tem ~25 mil linhas de testes dos gaps, sondas de RE e marcadores — 837 KB de
//! código e 143 strings de veredito medidos DENTRO do dylib público (2026-09-20). Nada disso serve a
//! quem instala o mod, e binário maior com nomes de teste é ruído pro analista de antivírus, que é a
//! razão de o projeto já gatear `devlog`/`autoproceed`.
//!
//! Este módulo entra no lugar dele quando a feature `devtools` está desligada: mesmas assinaturas,
//! corpos vazios, conferidos pelo compilador — se uma função do selftest mudar de forma, o build público quebra aqui,
//! que é o lugar certo pra ficar sabendo.
#![allow(unused_variables, dead_code, clippy::too_many_arguments)]

pub(crate) unsafe fn rawinput_despachar() {}
pub(crate) fn a48c_caminho_dinamico(hash: u64) -> Option<String> { Default::default() }

// O tick de produto (`cp77_tick`) pergunta pelo hook do `ProcessInputEvents` nos dois builds: o público usa o corpo real.
pub(crate) use crate::diagnostico::rawinput_hook_ativo;
