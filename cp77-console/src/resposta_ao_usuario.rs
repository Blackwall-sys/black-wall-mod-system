//! O que o USUÁRIO lê quando digita um comando.
//!
//! **Por que isto existe:** a saída dos comandos saía toda por `log_dev!`, que é no-op no build
//! público — e a aba Console lia o arquivo `/tmp/cp77-console.log`, que o público não escreve. O
//! resultado era o pior possível: a pessoa digitava `give ...`, o comando **executava**, e a tela não
//! mostrava nada. Nem confirmação, nem erro. Ficava indistinguível de "o mod não funciona".
//!
//! São dois canais com públicos diferentes, e misturá-los foi o erro:
//!
//! - `log_dev!` — diagnóstico de RE e de boot. Vai pro arquivo, existe só no build de dev, e continua
//!   assim: é o instrumental de descoberta, não interessa a quem joga.
//! - `resposta!` (daqui) — o que a pessoa pediu e o que aconteceu. Vive **em memória**, nos dois
//!   builds, e é o que a aba Console mostra.
//!
//! Memória e não arquivo por duas razões: não escrever nada no disco mantém o build público sem
//! caminho de arquivo embutido (o gate de rastro do empacotador varre por isso), e o teto de linhas
//! garante que uma sessão longa não cresça sem parar.

use std::collections::VecDeque;
use std::sync::Mutex;

/// Quantas linhas a aba guarda. 400 cobre uma sessão de console inteira com folga e custa ~40 KB.
const TETO_DE_LINHAS: usize = 400;

static LINHAS: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

/// Guarda uma linha pro usuário ler. Silenciosa se o mutex estiver envenenado: perder uma linha de
/// console nunca justifica derrubar o jogo.
pub(crate) fn responder(linha: impl Into<String>) {
    let linha = linha.into();
    // Espelho no log de dev: é por ele que o boot de teste confere o que a aba mostraria, sem capturar a tela.
    crate::log_dev!("[resposta] {linha}");
    if let Ok(mut g) = LINHAS.lock() {
        if g.len() >= TETO_DE_LINHAS {
            g.pop_front();
        }
        g.push_back(linha);
    }
}

/// As últimas `n` linhas, da mais antiga pra mais nova (ordem de leitura).
pub(crate) fn ultimas(n: usize) -> Vec<String> {
    match LINHAS.lock() {
        Ok(g) => g.iter().skip(g.len().saturating_sub(n)).cloned().collect(),
        Err(_) => Vec::new(),
    }
}

/// Esvazia a vista — o botão de limpar da aba.
pub(crate) fn limpar() {
    if let Ok(mut g) = LINHAS.lock() {
        g.clear();
    }
}

/// Quantas linhas estão guardadas.
pub(crate) fn quantas() -> usize {
    LINHAS.lock().map(|g| g.len()).unwrap_or(0)
}

/// Responde ao usuário. Diferente de [`crate::log_dev`], existe nos DOIS builds: é o canal de quem
/// joga, não o de quem depura.
#[macro_export]
macro_rules! resposta {
    ($($arg:tt)*) => {{
        $crate::resposta_ao_usuario::responder(format!($($arg)*));
    }};
}

/// Os testes que escrevem ou leem o buffer estático o compartilham, e o Rust roda testes em paralelo no
/// mesmo processo — sem trava, um veria as linhas do outro. Vale para os testes do terminal que mandam
/// `help` também.
#[cfg(test)]
pub(crate) static UM_POR_VEZ: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn a_linha_volta_na_ordem_de_leitura() {
        let _trava = UM_POR_VEZ.lock();
        limpar();
        responder("primeira");
        responder("segunda");
        let v = ultimas(10);
        assert_eq!(v, vec!["primeira".to_string(), "segunda".to_string()]);
    }

    #[test]
    fn pedir_menos_linhas_traz_as_mais_novas() {
        let _trava = UM_POR_VEZ.lock();
        limpar();
        for i in 1..=5 {
            responder(format!("linha {i}"));
        }
        assert_eq!(ultimas(2), vec!["linha 4".to_string(), "linha 5".to_string()]);
    }

    /// Sessão longa não pode crescer sem parar: o buffer descarta as antigas.
    #[test]
    fn o_teto_descarta_as_antigas_e_mantem_as_recentes() {
        let _trava = UM_POR_VEZ.lock();
        limpar();
        for i in 0..(TETO_DE_LINHAS + 50) {
            responder(format!("{i}"));
        }
        assert_eq!(quantas(), TETO_DE_LINHAS, "o buffer passou do teto");
        let v = ultimas(1);
        assert_eq!(v[0], (TETO_DE_LINHAS + 49).to_string(), "perdeu a linha mais nova");
    }

    #[test]
    fn limpar_esvazia_a_vista() {
        let _trava = UM_POR_VEZ.lock();
        responder("alguma coisa");
        limpar();
        assert_eq!(quantas(), 0);
        assert!(ultimas(10).is_empty());
    }

    /// Pedir de um buffer vazio não pode entrar em pânico nem devolver lixo.
    #[test]
    fn buffer_vazio_devolve_nada() {
        let _trava = UM_POR_VEZ.lock();
        limpar();
        assert!(ultimas(0).is_empty());
        assert!(ultimas(100).is_empty());
    }
}
