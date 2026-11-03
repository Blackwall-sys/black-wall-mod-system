//! Atalho que abre e fecha o terminal. A pessoa grava na aba Mods (segura 1 ou mais teclas, com ou sem modificador, e solta) e ele fica no
//! `bwms.toml` como `atalho_terminal = "ctrl+shift+17"`: modificadores pelo nome, teclas pelo código do macOS. Sem a chave vale o de sempre
//! (crase/til ou F1), decidido no `sendEvent` do overlay.

use std::sync::Mutex;

use crate::configuracao::do_mod::{gravar_texto, ler_texto};

/// Chave do atalho no `bwms.toml`.
pub(crate) const CHAVE: &str = "atalho_terminal";
const COMENTARIO: &str = "Teclas que abrem e fecham o terminal (grave pela aba Mods; apague a linha para voltar à crase ou F1).";

/// Bits do `modifierFlags` do AppKit: `NSEventModifierFlagControl`, `…Option`, `…Shift` e `…Command`.
pub(crate) const CTRL: u64 = 0x4_0000;
pub(crate) const OPTION: u64 = 0x8_0000;
pub(crate) const SHIFT: u64 = 0x2_0000;
pub(crate) const CMD: u64 = 0x10_0000;

/// Bits do `modifierFlags` do AppKit que contam no atalho, na ordem em que aparecem no texto e no rótulo.
const MODIFICADORES: [(u64, &str, &str); 4] =
    [(CTRL, "ctrl", "Ctrl"), (OPTION, "option", "Option"), (SHIFT, "shift", "Shift"), (CMD, "cmd", "Cmd")];
const MASCARA: u64 = CTRL | OPTION | SHIFT | CMD;
const ESC: u16 = 53;

/// O `modifierFlags` do evento tem o modificador seguro.
pub(crate) fn modificador_seguro(mods: u64, modificador: u64) -> bool {
    mods & modificador != 0
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Atalho {
    pub(crate) modificadores: u64,
    /// Códigos das teclas que não são modificador, em ordem crescente.
    pub(crate) teclas: Vec<u16>,
}

impl Atalho {
    /// `"ctrl+shift+17"` → atalho; `None` se algum pedaço não é modificador conhecido nem código de tecla, ou se não há tecla.
    pub(crate) fn do_texto(texto: &str) -> Option<Atalho> {
        let mut a = Atalho::default();
        for pedaco in texto.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            if let Some((bit, _, _)) = MODIFICADORES.iter().find(|(_, nome, _)| *nome == pedaco) {
                a.modificadores |= bit;
            } else {
                let kc: u16 = pedaco.parse().ok().filter(|k| *k < 128)?;
                if !a.teclas.contains(&kc) {
                    a.teclas.push(kc);
                }
            }
        }
        a.teclas.sort_unstable();
        (!a.teclas.is_empty()).then_some(a)
    }

    pub(crate) fn texto(&self) -> String {
        let mods = MODIFICADORES.iter().filter(|(bit, _, _)| self.modificadores & bit != 0).map(|(_, nome, _)| nome.to_string());
        mods.chain(self.teclas.iter().map(|k| k.to_string())).collect::<Vec<_>>().join("+")
    }

    /// O que a aba Mods mostra: "Ctrl + Shift + T".
    pub(crate) fn rotulo(&self) -> String {
        let mods = MODIFICADORES.iter().filter(|(bit, _, _)| self.modificadores & bit != 0).map(|(_, _, r)| r.to_string());
        mods.chain(self.teclas.iter().map(|k| nome_da_tecla(*k))).collect::<Vec<_>>().join(" + ")
    }

    /// As teclas seguras agora, com estes modificadores, são o atalho.
    fn bate(&self, modificadores: u64, seguras: &[u16]) -> bool {
        let mut s = seguras.to_vec();
        s.sort_unstable();
        modificadores & MASCARA == self.modificadores && s == self.teclas
    }
}

/// Nome da tecla pelo código do macOS (posição do teclado americano; letras e números são as mesmas no ABNT2).
fn nome_da_tecla(kc: u16) -> String {
    const LETRAS: [(u16, &str); 36] = [
        (0, "A"), (11, "B"), (8, "C"), (2, "D"), (14, "E"), (3, "F"), (5, "G"), (4, "H"), (34, "I"), (38, "J"), (40, "K"), (37, "L"),
        (46, "M"), (45, "N"), (31, "O"), (35, "P"), (12, "Q"), (15, "R"), (1, "S"), (17, "T"), (32, "U"), (9, "V"), (13, "W"), (7, "X"),
        (16, "Y"), (6, "Z"), (29, "0"), (18, "1"), (19, "2"), (20, "3"), (21, "4"), (23, "5"), (22, "6"), (26, "7"), (28, "8"), (25, "9"),
    ];
    const ESPECIAIS: [(u16, &str); 25] = [
        (36, "Enter"), (48, "Tab"), (49, "Espaço"), (51, "Apagar"), (53, "Esc"), (50, "`"), (10, "§"), (122, "F1"), (120, "F2"),
        (99, "F3"), (118, "F4"), (96, "F5"), (97, "F6"), (98, "F7"), (100, "F8"), (101, "F9"), (109, "F10"), (103, "F11"), (111, "F12"),
        (123, "Esquerda"), (124, "Direita"), (125, "Baixo"), (126, "Cima"), (117, "Delete"), (76, "Enter do teclado numérico"),
    ];
    LETRAS
        .iter()
        .chain(ESPECIAIS.iter())
        .find(|(k, _)| *k == kc)
        .map_or_else(|| format!("Tecla {kc}"), |(_, n)| n.to_string())
}

/// O que o `sendEvent` faz com o evento.
#[derive(Debug, PartialEq)]
pub(crate) enum Efeito {
    /// Segue para o jogo ou o terminal como antes.
    Segue,
    /// Abre ou fecha o terminal; o evento não segue.
    Alterna,
    /// Gravando: o evento não segue.
    Engole,
    /// A gravação terminou com este atalho.
    Gravado(Atalho),
    /// Esc sozinho durante a gravação: nada muda.
    Cancelado,
}

/// Teclas seguras agora e a gravação em curso.
pub(crate) struct Teclado {
    seguras: Vec<u16>,
    gravando: Option<Atalho>,
}

impl Teclado {
    pub(crate) const fn novo() -> Self {
        Self { seguras: Vec::new(), gravando: None }
    }

    pub(crate) fn gravar(&mut self) {
        self.gravando = Some(Atalho::default());
    }

    /// Tecla desceu (`NSEventTypeKeyDown`). A repetição de uma tecla já segura não alterna de novo.
    pub(crate) fn desceu(&mut self, kc: u16, mods: u64, atalho: Option<&Atalho>) -> Efeito {
        let nova = !self.seguras.contains(&kc);
        if nova {
            self.seguras.push(kc);
        }
        let mods = mods & MASCARA;
        if let Some(g) = self.gravando.as_mut() {
            if kc == ESC && mods == 0 && self.seguras.len() == 1 && g.teclas.is_empty() {
                self.gravando = None;
                return Efeito::Cancelado;
            }
            if !g.teclas.contains(&kc) {
                g.teclas.push(kc);
                g.teclas.sort_unstable();
            }
            g.modificadores |= mods;
            return Efeito::Engole;
        }
        match atalho {
            Some(a) if nova && a.bate(mods, &self.seguras) => Efeito::Alterna,
            _ => Efeito::Segue,
        }
    }

    /// Tecla subiu (`NSEventTypeKeyUp`).
    pub(crate) fn subiu(&mut self, kc: u16, mods: u64) -> Efeito {
        self.seguras.retain(|k| *k != kc);
        self.talvez_terminar(mods)
    }

    /// Modificadores mudaram (`NSEventTypeFlagsChanged`).
    pub(crate) fn modificadores(&mut self, mods: u64) -> Efeito {
        if let Some(g) = self.gravando.as_mut() {
            g.modificadores |= mods & MASCARA;
        }
        self.talvez_terminar(mods)
    }

    /// A gravação termina quando tudo foi solto e havia pelo menos 1 tecla que não é modificador.
    fn talvez_terminar(&mut self, mods: u64) -> Efeito {
        let pronto = self.seguras.is_empty() && mods & MASCARA == 0;
        match self.gravando.take() {
            None => Efeito::Segue,
            Some(g) if pronto && !g.teclas.is_empty() => Efeito::Gravado(g),
            Some(g) => {
                self.gravando = Some(g);
                Efeito::Engole
            }
        }
    }
}

struct Estado {
    teclado: Teclado,
    atalho: Option<Atalho>,
    lido: bool,
    erro: Option<String>,
}

static ESTADO: Mutex<Estado> = Mutex::new(Estado { teclado: Teclado::novo(), atalho: None, lido: false, erro: None });

fn com_estado<T>(f: impl FnOnce(&mut Estado) -> T) -> Option<T> {
    let mut e = ESTADO.lock().ok()?;
    if !e.lido {
        e.atalho = ler_texto(CHAVE).and_then(|t| Atalho::do_texto(&t));
        e.lido = true;
    }
    Some(f(&mut e))
}

/// Evento de teclado do `sendEvent` (tipos 10, 11 e 12 do AppKit). Grava o atalho novo no `bwms.toml` quando a gravação termina.
pub(crate) fn evento(tipo: usize, kc: u16, mods: u64) -> Efeito {
    com_estado(|e| {
        let efeito = match tipo {
            10 => e.teclado.desceu(kc, mods, e.atalho.as_ref()),
            11 => e.teclado.subiu(kc, mods),
            12 => e.teclado.modificadores(mods),
            _ => Efeito::Segue,
        };
        if let Efeito::Gravado(a) = &efeito {
            e.erro = gravar_texto(CHAVE, &a.texto(), COMENTARIO)
                .err()
                .map(|err| format!("Não consegui gravar o atalho no bwms.toml ({err}). Confira se a pasta red4ext do jogo aceita escrita."));
            e.atalho = Some(a.clone());
        }
        efeito
    })
    .unwrap_or(Efeito::Segue)
}

/// Há atalho gravado pela pessoa (senão o `sendEvent` usa crase/til ou F1).
pub(crate) fn tem_atalho() -> bool {
    com_estado(|e| e.atalho.is_some()).unwrap_or(false)
}

pub(crate) fn iniciar_gravacao() {
    com_estado(|e| {
        e.erro = None;
        e.teclado.gravar();
    });
}

pub(crate) fn gravando() -> bool {
    com_estado(|e| e.teclado.gravando.is_some()).unwrap_or(false)
}

/// Rótulo do atalho em uso, para a aba Mods.
pub(crate) fn rotulo() -> String {
    com_estado(|e| e.atalho.as_ref().map(Atalho::rotulo)).flatten().unwrap_or_else(|| "` ou F1".to_string())
}

/// Erro da última gravação no `bwms.toml`, em PT-BR com a ação sugerida.
pub(crate) fn erro() -> Option<String> {
    com_estado(|e| e.erro.clone()).flatten()
}

#[cfg(test)]
mod teste {
    use super::*;

    const CTRL: u64 = 0x4_0000;
    const SHIFT: u64 = 0x2_0000;

    #[test]
    fn texto_vai_e_volta() {
        let a = Atalho::do_texto("shift + CTRL + 17").expect("atalho");
        assert_eq!(a, Atalho { modificadores: CTRL | SHIFT, teclas: vec![17] });
        assert_eq!(a.texto(), "ctrl+shift+17");
        assert_eq!(a.rotulo(), "Ctrl + Shift + T");
        assert_eq!(Atalho::do_texto("ctrl"), None, "só modificador não é atalho");
        assert_eq!(Atalho::do_texto("ctrl+tecla"), None);
        assert_eq!(Atalho::do_texto("200"), None);
        assert_eq!(Atalho::do_texto("17+12").map(|a| a.teclas), Some(vec![12, 17]));
    }

    #[test]
    fn grava_varias_teclas_e_termina_ao_soltar_tudo() {
        let mut t = Teclado::novo();
        t.gravar();
        assert_eq!(t.modificadores(CTRL), Efeito::Engole);
        assert_eq!(t.desceu(17, CTRL, None), Efeito::Engole);
        assert_eq!(t.desceu(12, CTRL, None), Efeito::Engole);
        assert_eq!(t.subiu(17, CTRL), Efeito::Engole);
        assert_eq!(t.subiu(12, CTRL), Efeito::Engole, "o Ctrl ainda está seguro");
        assert_eq!(t.modificadores(0), Efeito::Gravado(Atalho { modificadores: CTRL, teclas: vec![12, 17] }));
        assert_eq!(t.desceu(1, 0, None), Efeito::Segue, "a gravação acabou");
    }

    #[test]
    fn esc_sozinho_cancela_e_esc_no_meio_entra() {
        let mut t = Teclado::novo();
        t.gravar();
        assert_eq!(t.desceu(ESC, 0, None), Efeito::Cancelado);
        t.subiu(ESC, 0);
        t.gravar();
        t.desceu(17, 0, None);
        assert_eq!(t.desceu(ESC, 0, None), Efeito::Engole);
    }

    #[test]
    fn alterna_so_na_descida_e_nao_na_repeticao() {
        let a = Atalho { modificadores: CTRL, teclas: vec![17] };
        let mut t = Teclado::novo();
        assert_eq!(t.desceu(17, 0, Some(&a)), Efeito::Segue, "sem o Ctrl não bate");
        t.subiu(17, 0);
        assert_eq!(t.desceu(17, CTRL, Some(&a)), Efeito::Alterna);
        assert_eq!(t.desceu(17, CTRL, Some(&a)), Efeito::Segue, "repetição da tecla segura");
        t.subiu(17, CTRL);
        assert_eq!(t.desceu(17, CTRL | 0x100, Some(&a)), Efeito::Alterna, "bits fora da máscara não contam");
    }
}
