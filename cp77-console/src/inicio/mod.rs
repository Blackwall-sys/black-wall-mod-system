//! Tudo que leva o jogo até a gameplay: o boot próprio (gancho do executor, pular intro, continuar, vigias de travamento, saída limpa) e a
//! tela de carregamento desenhada no Metal.

pub(crate) mod boot;
pub(crate) mod splash;
pub(crate) mod natives;
pub(crate) mod energia;
pub(crate) mod sessao_do_jogo;
