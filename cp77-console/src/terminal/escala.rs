//! Escala do terminal: múltiplo inteiro do desenho todo (fonte, respiros, janela). A fonte é de pixel e só fica nítida em múltiplo
//! inteiro; por isso a escala segue a resolução do jogo, não o tamanho da janela.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::configuracao::do_mod::{gravar_texto, ler_texto};
use crate::terminal::imediato::JANELA_MINIMA;

/// Chave da escala no `bwms.toml`: `"auto"` ou `"1"` a `"3"`.
pub(crate) const CHAVE: &str = "escala_do_terminal";
const COMENTARIO: &str = "Tamanho do terminal: \"auto\" (pela resolução do jogo) ou \"1\", \"2\", \"3\" (escolha pela aba Mods).";
/// Botões da aba Mods, na ordem da escolha: 0 = automática.
pub(crate) const ESCOLHAS: [&str; 4] = ["Auto", "1×", "2×", "3×"];
/// Altura do quadro que sai em 1×: a dos boots medidos (1280×800). A automática arredonda `altura / isto`.
const ALTURA_DE_REFERENCIA: f32 = 800.0;

/// Escolha do jogador; `u32::MAX` = o `bwms.toml` ainda não foi lido.
static ESCOLHA: AtomicU32 = AtomicU32::new(u32::MAX);
/// Escala do último quadro desenhado: a nota da aba Mods fala dela.
static ATUAL: AtomicU32 = AtomicU32::new(1);
static ERRO: Mutex<Option<String>> = Mutex::new(None);

/// A escala num quadro de `quadro` pixels. A automática arredonda pela altura: 1× em 1280×800 e 1920×1080, 2× em 1920×1200 e
/// 2560×1600, 3× em 3840×2160. A do jogador vale como veio. As duas param no maior múltiplo em que a janela mínima cabe na tela.
pub(crate) fn para(quadro: [f32; 2], escolha: u32) -> u32 {
    let cabe = (quadro[0] / JANELA_MINIMA[0]).min(quadro[1] / JANELA_MINIMA[1]).floor().max(1.0) as u32;
    let pedida = if escolha == 0 { (quadro[1] / ALTURA_DE_REFERENCIA).round() as u32 } else { escolha };
    pedida.clamp(1, cabe)
}

fn do_texto(t: &str) -> u32 {
    match t.trim() {
        "1" => 1,
        "2" => 2,
        "3" => 3,
        _ => 0,
    }
}

/// A escolha do jogador: 0 = automática, 1 a 3 = fixa. Lida do `bwms.toml` na primeira chamada.
pub(crate) fn escolha() -> u32 {
    let e = ESCOLHA.load(Ordering::Relaxed);
    if e != u32::MAX {
        return e;
    }
    let lida = ler_texto(CHAVE).map_or(0, |t| do_texto(&t));
    ESCOLHA.store(lida, Ordering::Relaxed);
    lida
}

/// Escala deste quadro. O renderizador chama uma vez por quadro, antes de montar o terminal.
pub(crate) fn deste_quadro(quadro: [f32; 2]) -> u32 {
    let n = para(quadro, escolha());
    ATUAL.store(n, Ordering::Relaxed);
    n
}

/// Troca a escolha e grava no `bwms.toml`. Vale na hora mesmo se a gravação falhar; aí o próximo boot volta à anterior.
pub(crate) fn escolher(n: u32) {
    let n = n.min(3);
    ESCOLHA.store(n, Ordering::Relaxed);
    let texto = if n == 0 { "auto".to_string() } else { n.to_string() };
    let erro = gravar_texto(CHAVE, &texto, COMENTARIO)
        .err()
        .map(|err| format!("Não consegui gravar a escala no bwms.toml ({err}). Confira se a pasta red4ext do jogo aceita escrita."));
    if let Ok(mut e) = ERRO.lock() {
        *e = erro;
    }
}

/// Erro da última gravação no `bwms.toml`, em PT-BR com a ação sugerida.
pub(crate) fn erro() -> Option<String> {
    ERRO.lock().ok().and_then(|e| e.clone())
}

/// O que a aba Mods mostra depois dos botões: a escala que a automática deu, ou que a escolhida não cabe nesta tela.
pub(crate) fn nota() -> Option<String> {
    let (e, n) = (escolha(), ATUAL.load(Ordering::Relaxed));
    if e == 0 {
        Some(format!("({n}× nesta tela)"))
    } else if n != e {
        Some(format!("({e}× não cabe: {n}×)"))
    } else {
        None
    }
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn automatica_arredonda_pela_altura() {
        assert_eq!(para([1280.0, 800.0], 0), 1);
        assert_eq!(para([1920.0, 1080.0], 0), 1);
        assert_eq!(para([1920.0, 1200.0], 0), 2);
        assert_eq!(para([2560.0, 1600.0], 0), 2);
        assert_eq!(para([3840.0, 2160.0], 0), 3);
    }

    #[test]
    fn escolhida_para_onde_a_janela_minima_nao_cabe() {
        assert_eq!(para([1280.0, 800.0], 2), 1);
        assert_eq!(para([1920.0, 1200.0], 3), 2);
        assert_eq!(para([3840.0, 2160.0], 3), 3);
        assert_eq!(para([3840.0, 2160.0], 1), 1);
    }

    #[test]
    fn tela_menor_que_a_janela_fica_em_1x() {
        assert_eq!(para([640.0, 400.0], 0), 1);
        assert_eq!(para([0.0, 0.0], 3), 1);
    }

    #[test]
    fn texto_do_toml() {
        assert_eq!(do_texto("auto"), 0);
        assert_eq!(do_texto(" 2 "), 2);
        assert_eq!(do_texto("9"), 0);
    }
}
