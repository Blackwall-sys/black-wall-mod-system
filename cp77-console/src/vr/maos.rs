//! As mãos do headset, do lado do jogo.
//!
//! O transporte recebe a pose dos controles pelo navegador e a deixa em `<jogo>/red4ext/vr/maos.txt`.
//! Aqui elas viram estado consultável pelo redscript, que é quem pode mirar, atirar e pegar coisa
//! dentro do jogo — o dylib não decide nada sobre jogabilidade, só entrega o dado.
//!
//! Por que arquivo e não socket: o dylib roda dentro do processo do jogo e não fala rede (é o que o
//! mantém fora da mira de heurística de antivírus), e o transporte é outro processo. Um arquivo de
//! 78 bytes, escrito e renomeado, resolve sem acoplar os dois.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// Uma mão como o headset a vê. Os eixos são os do WebXR: x pra direita, y pra cima, z pra trás,
/// em metros, com a origem onde a pessoa calibrou o chão.
#[derive(Clone, Copy, Default)]
pub(crate) struct Mao {
    pub presente: bool,
    pub posicao: [f32; 3],
    pub orientacao: [f32; 4],
    pub gatilho: f32,
    pub agarrar: f32,
    pub botoes: u8,
}

static ESQUERDA: Mutex<Mao> = Mutex::new(Mao {
    presente: false,
    posicao: [0.0; 3],
    orientacao: [0.0, 0.0, 0.0, 1.0],
    gatilho: 0.0,
    agarrar: 0.0,
    botoes: 0,
});
static DIREITA: Mutex<Mao> = Mutex::new(Mao {
    presente: false,
    posicao: [0.0; 3],
    orientacao: [0.0, 0.0, 0.0, 1.0],
    gatilho: 0.0,
    agarrar: 0.0,
    botoes: 0,
});
/// Quantas leituras entraram. O redscript usa pra saber se o dado é novo sem comparar pose a pose.
static LEITURAS: AtomicU64 = AtomicU64::new(0);
/// O gatilho está apertado, DEPOIS da histerese. Bit 0 = esquerda, bit 1 = direita.
static GATILHO_SEGURADO: AtomicU32 = AtomicU32::new(0);
/// O gatilho ACABOU de cruzar o limiar nesta leitura. Vale pelo quadro e a próxima leitura apaga —
/// assim o script pode chamar a native quantas vezes quiser no mesmo quadro e ver o mesmo valor.
static GATILHO_BORDA: AtomicU32 = AtomicU32::new(0);

/// Histerese: aperta em 0,6 e solta em 0,3. Um limiar único faria o tiro tremer quando o dedo
/// descansa exatamente nele — e num gatilho analógico isso é o estado normal, não a exceção.
const APERTA: f32 = 0.6;
const SOLTA: f32 = 0.3;

/// Atualiza segurado/borda a partir do valor cru. Separado e puro porque é a conta que decide se um
/// tiro sai ou não: um sinal trocado aqui vira arma que atira sozinha.
fn passo_do_gatilho(valor: f32, estava_segurado: bool) -> (bool, bool) {
    let segurado = if estava_segurado { valor > SOLTA } else { valor >= APERTA };
    (segurado, segurado && !estava_segurado)
}
/// Quando o arquivo foi modificado da última vez — reler um arquivo que não mudou é I/O à toa.
static ULTIMA_MTIME: AtomicU64 = AtomicU64::new(0);
static FALHAS: AtomicU32 = AtomicU32::new(0);

fn arquivo() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(crate::dylib_dir()?).join("vr/maos.txt"))
}

/// Lê as mãos se o arquivo mudou. Chamado do tick; sem VR conectado o arquivo não existe e isto é
/// um `stat` que falha — por isso o chamador só chama quando a aba diz que há headset.
pub(crate) fn atualizar() {
    let Some(caminho) = arquivo() else { return };
    let Ok(meta) = std::fs::metadata(&caminho) else { return };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    if mtime == ULTIMA_MTIME.load(Ordering::Relaxed) {
        return; // nada novo desde a última vez
    }
    let Ok(txt) = std::fs::read_to_string(&caminho) else { return };
    let mut leu = false;
    for linha in txt.lines() {
        let Some((nome, resto)) = linha.split_once('=') else { continue };
        let Some(m) = analisar(resto) else {
            FALHAS.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let alvo = match nome.trim() {
            "esq" => &ESQUERDA,
            "dir" => &DIREITA,
            _ => continue,
        };
        if let Ok(mut g) = alvo.lock() {
            *g = m;
            leu = true;
        }
    }
    if leu {
        ULTIMA_MTIME.store(mtime, Ordering::Relaxed);
        LEITURAS.fetch_add(1, Ordering::Relaxed);
        atualizar_gatilhos();
    }
}

/// Roda depois de cada leitura nova. A borda do quadro anterior é descartada aqui — é o que a torna
/// idempotente pro script dentro do mesmo quadro.
fn atualizar_gatilhos() {
    let antes = GATILHO_SEGURADO.load(Ordering::Relaxed);
    let (mut segurado, mut borda) = (0u32, 0u32);
    for (bit, direita) in [(0u32, false), (1u32, true)] {
        let m = mao(direita);
        let (s, b) = passo_do_gatilho(m.gatilho, antes & (1 << bit) != 0);
        // Mão fora de vista não atira: o gatilho da última pose vista ficaria preso apertado.
        let s = s && m.presente;
        segurado |= u32::from(s) << bit;
        borda |= u32::from(b && m.presente) << bit;
    }
    GATILHO_SEGURADO.store(segurado, Ordering::Relaxed);
    GATILHO_BORDA.store(borda, Ordering::Relaxed);
}

/// O gatilho dessa mão está apertado? (já com histerese)
pub(crate) fn gatilho_segurado(direita: bool) -> bool {
    let bit = u32::from(direita);
    GATILHO_SEGURADO.load(Ordering::Relaxed) & (1 << bit) != 0
}

/// O gatilho dessa mão ACABOU de ser apertado? Vale pelo quadro inteiro.
pub(crate) fn gatilho_borda(direita: bool) -> bool {
    let bit = u32::from(direita);
    GATILHO_BORDA.load(Ordering::Relaxed) & (1 << bit) != 0
}

/// `presente x y z qx qy qz qw gatilho agarrar botoes` — onze campos, separados por espaço.
fn analisar(linha: &str) -> Option<Mao> {
    let campos: Vec<&str> = linha.split_whitespace().collect();
    if campos.len() != 11 {
        return None;
    }
    let f = |i: usize| campos[i].parse::<f32>().ok();
    Some(Mao {
        presente: campos[0] == "1",
        posicao: [f(1)?, f(2)?, f(3)?],
        orientacao: [f(4)?, f(5)?, f(6)?, f(7)?],
        gatilho: f(8)?.clamp(0.0, 1.0),
        agarrar: f(9)?.clamp(0.0, 1.0),
        botoes: campos[10].parse().ok()?,
    })
}

pub(crate) fn mao(direita: bool) -> Mao {
    let alvo = if direita { &DIREITA } else { &ESQUERDA };
    alvo.lock().map(|g| *g).unwrap_or_default()
}

pub(crate) fn leituras() -> u64 {
    LEITURAS.load(Ordering::Relaxed)
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn a_linha_do_transporte_vira_mao() {
        let m = analisar("1 -0.3 1.1 -0.4 0 0.7 0 0.7 0.5 1 5").expect("linha válida");
        assert!(m.presente);
        assert_eq!(m.posicao, [-0.3, 1.1, -0.4]);
        assert_eq!(m.orientacao[3], 0.7);
        assert_eq!(m.gatilho, 0.5);
        assert_eq!(m.agarrar, 1.0);
        assert_eq!(m.botoes, 5);
    }

    #[test]
    fn linha_torta_nao_vira_mao_torta() {
        assert!(analisar("").is_none());
        assert!(analisar("1 2 3").is_none(), "aceitou linha com campos faltando");
        assert!(analisar("1 a b c 0 0 0 1 0 0 0").is_none(), "aceitou texto no lugar de número");
        assert!(analisar("1 0 0 0 0 0 0 1 0 0 0 99").is_none(), "aceitou campo sobrando");
    }

    /// A histerese é o que separa "atirou" de "arma tremendo": dedo parado no limiar é o normal.
    #[test]
    fn o_gatilho_aperta_em_um_valor_e_solta_em_outro() {
        // subindo: só aperta ao alcançar 0,6
        assert_eq!(passo_do_gatilho(0.5, false), (false, false));
        assert_eq!(passo_do_gatilho(0.6, false), (true, true), "não apertou no limiar");
        // já apertado: continua até cair de 0,3
        assert_eq!(passo_do_gatilho(0.4, true), (true, false), "soltou cedo demais");
        assert_eq!(passo_do_gatilho(0.29, true), (false, false));
        // e a borda só aparece na transição, nunca enquanto segura
        assert_eq!(passo_do_gatilho(1.0, true), (true, false), "repetiu a borda segurando");
    }

    /// Dedo descansando exatamente no meio não pode gerar uma borda por quadro.
    #[test]
    fn dedo_tremendo_no_meio_da_faixa_nao_atira_em_rajada() {
        let mut segurado = false;
        let mut bordas = 0;
        for v in [0.45, 0.44, 0.46, 0.45, 0.47, 0.43] {
            let (s, b) = passo_do_gatilho(v, segurado);
            segurado = s;
            bordas += i32::from(b);
        }
        assert_eq!(bordas, 0, "gerou {bordas} tiros com o dedo parado no meio");
    }

    /// Gatilho vindo fora da faixa viraria mira ou tiro errado dentro do jogo.
    #[test]
    fn gatilho_fora_da_faixa_e_preso() {
        let m = analisar("1 0 0 0 0 0 0 1 7.5 -2 0").unwrap();
        assert_eq!(m.gatilho, 1.0);
        assert_eq!(m.agarrar, 0.0);
    }

    #[test]
    fn sem_arquivo_nada_acontece() {
        let antes = leituras();
        atualizar(); // fora do jogo não há `dylib_dir`; não pode entrar em pânico
        assert_eq!(leituras(), antes);
    }
}
