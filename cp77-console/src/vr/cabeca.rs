//! A cabeça do headset, do lado do jogo.
//!
//! O transporte acumula o giro que a página manda e deixa em `<jogo>/red4ext/vr/cabeca.txt`. Aqui
//! isso vira estado consultável pelo redscript, que é quem pode girar a câmera do jogo — o dylib não
//! decide nada de jogabilidade, só entrega o número.
//!
//! **Absoluto, não delta.** O arquivo traz onde a cabeça está, e o script compara com o que já
//! aplicou. Se um arquivo se perder, o próximo já corrige; com delta, cada arquivo perdido viraria
//! um desvio permanente na mira.
//!
//! Mesmo caminho das mãos ([[maos_vr]]): arquivo em vez de socket, porque o dylib roda dentro do jogo
//! e não fala rede, e o transporte é outro processo.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;

/// Onde a cabeça está, em radianos, no sistema do WebXR.
#[derive(Clone, Copy, Default)]
pub(crate) struct Cabeca {
    pub yaw: f32,
    pub pitch: f32,
    pub sequencia: u32,
}

static ATUAL: Mutex<Cabeca> = Mutex::new(Cabeca { yaw: 0.0, pitch: 0.0, sequencia: 0 });
/// Quando o arquivo mudou pela última vez — reler arquivo parado é I/O à toa no tick.
static ULTIMA_MTIME: AtomicU64 = AtomicU64::new(0);
static FALHAS: AtomicU32 = AtomicU32::new(0);

fn arquivo() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(crate::dylib_dir()?).join("vr/cabeca.txt"))
}

/// Lê a cabeça se o arquivo mudou. Chamado do tick, só quando a aba diz que há headset em sessão —
/// sem VR o arquivo não existe e isto seria um `stat` falhando por quadro.
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
        return;
    }
    let Ok(txt) = std::fs::read_to_string(&caminho) else { return };
    let Some(c) = analisar(&txt) else {
        FALHAS.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if let Ok(mut g) = ATUAL.lock() {
        *g = c;
        ULTIMA_MTIME.store(mtime, Ordering::Relaxed);
    }
}

/// `yaw=<rad>`, `pitch=<rad>`, `seq=<n>`, uma por linha. Sem os três, não é leitura válida — meia
/// leitura viraria câmera pulando pro zero.
fn analisar(txt: &str) -> Option<Cabeca> {
    let (mut yaw, mut pitch, mut seq) = (None, None, None);
    for linha in txt.lines() {
        let Some((chave, valor)) = linha.split_once('=') else { continue };
        let valor = valor.trim();
        match chave.trim() {
            "yaw" => yaw = valor.parse::<f32>().ok(),
            "pitch" => pitch = valor.parse::<f32>().ok(),
            "seq" => seq = valor.parse::<u32>().ok(),
            _ => {}
        }
    }
    let (yaw, pitch) = (yaw?, pitch?);
    // Valor absurdo ou NaN giraria a câmera pra lugar nenhum. O pitch é preso no mesmo limite que o
    // transporte usa, e o yaw é livre porque dar voltas é legítimo.
    if !yaw.is_finite() || !pitch.is_finite() {
        return None;
    }
    Some(Cabeca { yaw, pitch: pitch.clamp(-1.55, 1.55), sequencia: seq? })
}

pub(crate) fn cabeca() -> Cabeca {
    ATUAL.lock().map(|g| *g).unwrap_or_default()
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn o_arquivo_do_transporte_vira_cabeca() {
        let c = analisar("yaw=0.75\npitch=-0.25\nseq=42\n").expect("arquivo válido");
        assert_eq!(c.yaw, 0.75);
        assert_eq!(c.pitch, -0.25);
        assert_eq!(c.sequencia, 42);
    }

    #[test]
    fn leitura_pela_metade_nao_vira_camera_no_zero() {
        assert!(analisar("").is_none());
        assert!(analisar("yaw=0.5\n").is_none(), "aceitou arquivo sem pitch");
        assert!(analisar("yaw=0.5\npitch=0.1\n").is_none(), "aceitou arquivo sem sequência");
        assert!(analisar("yaw=x\npitch=0.1\nseq=1\n").is_none(), "aceitou texto no lugar de número");
    }

    #[test]
    fn valor_impossivel_e_recusado_ou_preso() {
        assert!(analisar("yaw=nan\npitch=0\nseq=1\n").is_none(), "aceitou NaN");
        assert!(analisar("yaw=inf\npitch=0\nseq=1\n").is_none(), "aceitou infinito");
        let c = analisar("yaw=99.0\npitch=9.0\nseq=1\n").expect("valores grandes são lidos");
        assert_eq!(c.pitch, 1.55, "o pitch não foi preso no limite");
        assert_eq!(c.yaw, 99.0, "o yaw é livre: dar voltas é legítimo");
    }

    /// Ordem e espaço em branco vêm do transporte, não de uma especificação nossa: aceitar os dois.
    #[test]
    fn a_ordem_das_linhas_nao_importa() {
        let c = analisar("seq=7\n pitch = 0.5 \nyaw=1.0\n").expect("arquivo em outra ordem");
        assert_eq!((c.yaw, c.pitch, c.sequencia), (1.0, 0.5, 7));
    }
}
