//! Scripts: compilar os .reds, ler o final.redscripts, traduzir bytecode e interceptar funções de script (antes, depois, no lugar).

#![allow(dead_code)]


pub(crate) mod sobrepor_funcao;

pub(crate) mod tradutor_de_bytecode;

pub(crate) mod blob;

pub(crate) mod carregador;

pub(crate) mod compilacao_no_lancamento;

