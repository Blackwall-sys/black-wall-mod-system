//! A aba CPVR do terminal: o que o VR está fazendo, e o que dá pra mudar sem sair do jogo.
//!
//! Regra da aba, pedida pelo autor: **sem headset conectado ela não abre**. Fica como um rótulo
//! apagado ao lado das outras, porque uma aba cheia de controles que não fazem nada é pior do que
//! não ter aba — a pessoa mexe, não vê efeito, e conclui que o mod está quebrado. Assim que um
//! headset aparece, ela acende e passa a valer.
//!
//! Quem diz que há um headset é o transporte (`CPVR/navegador/transporte`): ele escreve o estado num
//! arquivo quando um navegador entra em sessão imersiva. O dylib não fala rede — lê o estado.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// O que o transporte reporta. Tudo em atômico: a aba é desenhada na thread de render e o estado é
/// atualizado no tick, sem lock no caminho de desenho.
static CONECTADO_EM: AtomicU64 = AtomicU64::new(0);
static QUADROS_ENVIADOS: AtomicU64 = AtomicU64::new(0);
static LARGURA: AtomicU32 = AtomicU32::new(0);
static ALTURA: AtomicU32 = AtomicU32::new(0);
/// Nome curto do headset, como o navegador o reporta (o WebXR diz o que é).
static APARELHO: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Ajustes que a aba escreve e o transporte lê. São poucos de propósito: o que muda o "feel" do VR
/// e o que troca qualidade por quadros.
pub(crate) static ESCALA_DE_RENDER: AtomicU32 = AtomicU32::new(100); // 50..150, em porcento
pub(crate) static DISTANCIA_DOS_OLHOS: AtomicU32 = AtomicU32::new(64); // 55..75, em milímetros
pub(crate) static QUALIDADE: AtomicU32 = AtomicU32::new(1); // 0 = fluido, 1 = equilibrado, 2 = nítido
pub(crate) static MAOS_LIGADAS: AtomicU32 = AtomicU32::new(1);

/// Há um headset em sessão agora? É isto que decide se a aba abre.
pub(crate) fn conectado() -> bool {
    let visto = CONECTADO_EM.load(Ordering::Relaxed);
    if visto == 0 {
        return false;
    }
    // O transporte carimba o instante a cada quadro entregue. Três segundos sem carimbo = a pessoa
    // tirou o headset ou fechou a aba do navegador, e a aba tem que apagar sozinha.
    !crate::tempo::ja_passou(visto, agora_em_segundos(), 3)
}

/// Acende ou apaga a aba CPVR na prova visual, como o transporte faz a cada quadro entregue.
#[cfg(test)]
pub(crate) fn headset_para_teste(ligado: bool) {
    CONECTADO_EM.store(if ligado { agora_em_segundos() } else { 0 }, Ordering::Relaxed);
}

/// Os testes que mexem nos estáticos do painel passam por esta trava: os daqui e a prova visual do terminal, que liga o headset e
/// desenha a aba CPVR. O Rust roda testes em paralelo no mesmo processo; sem ela, um teste põe escala 80, o outro põe 90, e o
/// primeiro a ler vê o valor do vizinho — falha intermitente que aparecia ~1 em 6 execuções.
#[cfg(test)]
pub(crate) static UM_POR_VEZ: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn agora_em_segundos() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Lê o estado que o transporte deixou no disco. Chamado do tick, a cada ~1 s: é um arquivo de
/// poucas dezenas de bytes, e o caminho de frame não encosta nele.
pub(crate) fn atualizar_do_transporte() {
    let Some(caminho) = arquivo_de_estado() else { return };
    let Ok(txt) = std::fs::read_to_string(&caminho) else { return };
    // Formato: uma chave por linha, `nome=valor`. Escrito por nós, lido por nós — não é TOML nem
    // JSON porque não precisa ser.
    for linha in txt.lines() {
        let Some((chave, valor)) = linha.split_once('=') else { continue };
        match chave.trim() {
            "visto" => CONECTADO_EM.store(valor.trim().parse().unwrap_or(0), Ordering::Relaxed),
            "quadros" => QUADROS_ENVIADOS.store(valor.trim().parse().unwrap_or(0), Ordering::Relaxed),
            "largura" => LARGURA.store(valor.trim().parse().unwrap_or(0), Ordering::Relaxed),
            "altura" => ALTURA.store(valor.trim().parse().unwrap_or(0), Ordering::Relaxed),
            "aparelho" => {
                if let Ok(mut a) = APARELHO.lock() {
                    *a = valor.trim().to_string();
                }
            }
            _ => {}
        }
    }
}

/// Grava os ajustes pro transporte ler. Só quando algo muda — a aba chama isto no clique, não por
/// frame.
pub(crate) fn gravar_ajustes() {
    let Some(caminho) = arquivo_de_ajustes() else { return };
    let txt = format!(
        "escala={}\nolhos={}\nqualidade={}\nmaos={}\n",
        ESCALA_DE_RENDER.load(Ordering::Relaxed),
        DISTANCIA_DOS_OLHOS.load(Ordering::Relaxed),
        QUALIDADE.load(Ordering::Relaxed),
        MAOS_LIGADAS.load(Ordering::Relaxed),
    );
    let _ = std::fs::write(caminho, txt);
}

fn pasta_do_vr() -> Option<std::path::PathBuf> {
    // Ao lado do dylib, na pasta do jogo: o transporte roda na mesma máquina e sabe achar.
    Some(std::path::PathBuf::from(crate::dylib_dir()?).join("vr"))
}
fn arquivo_de_estado() -> Option<std::path::PathBuf> {
    Some(pasta_do_vr()?.join("estado.txt"))
}
fn arquivo_de_ajustes() -> Option<std::path::PathBuf> {
    let p = pasta_do_vr()?;
    let _ = std::fs::create_dir_all(&p);
    Some(p.join("ajustes.txt"))
}

/// Uma linha por informação, do jeito que a aba mostra.
pub(crate) fn resumo() -> Vec<String> {
    let mut v = Vec::new();
    let aparelho = APARELHO.lock().map(|a| a.clone()).unwrap_or_default();
    v.push(format!(
        "Headset: {}",
        if aparelho.is_empty() { "conectado".to_string() } else { aparelho }
    ));
    let (l, a) = (LARGURA.load(Ordering::Relaxed), ALTURA.load(Ordering::Relaxed));
    if l > 0 && a > 0 {
        let escala = ESCALA_DE_RENDER.load(Ordering::Relaxed);
        v.push(format!("Por olho: {}x{} ({escala}% da tela do jogo)", l * escala / 100, a * escala / 100));
    }
    v.push(format!("Quadros entregues: {}", QUADROS_ENVIADOS.load(Ordering::Relaxed)));
    v
}

/// O texto do rótulo quando não há headset. Aparece apagado, e explica o que fazer — um rótulo
/// apagado sem explicação faz a pessoa achar que o mod perdeu uma função.
pub(crate) const SEM_HEADSET: &str = "CPVR — sem headset. Abra o endereço do transporte no navegador do seu headset e entre em VR.";

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn sem_carimbo_a_aba_fica_apagada() {
        let _trava = UM_POR_VEZ.lock();
        CONECTADO_EM.store(0, Ordering::Relaxed);
        assert!(!conectado(), "sem nunca ter visto headset, a aba não pode abrir");
    }

    #[test]
    fn carimbo_velho_apaga_a_aba_sozinho() {
        let _trava = UM_POR_VEZ.lock();
        CONECTADO_EM.store(agora_em_segundos().saturating_sub(30), Ordering::Relaxed);
        assert!(!conectado(), "30 s sem quadro e a aba continuou acesa");
        CONECTADO_EM.store(agora_em_segundos(), Ordering::Relaxed);
        assert!(conectado(), "carimbo de agora tinha que acender a aba");
        CONECTADO_EM.store(0, Ordering::Relaxed);
    }

    #[test]
    fn o_resumo_mostra_a_resolucao_por_olho_na_escala_escolhida() {
        let _trava = UM_POR_VEZ.lock();
        LARGURA.store(2000, Ordering::Relaxed);
        ALTURA.store(1000, Ordering::Relaxed);
        ESCALA_DE_RENDER.store(80, Ordering::Relaxed);
        let linhas = resumo();
        assert!(linhas.iter().any(|l| l.contains("1600x800")), "escala não aplicada: {linhas:?}");
        ESCALA_DE_RENDER.store(100, Ordering::Relaxed);
        LARGURA.store(0, Ordering::Relaxed);
        ALTURA.store(0, Ordering::Relaxed);
    }

    #[test]
    fn os_ajustes_vao_e_voltam_pelo_arquivo() {
        let _trava = UM_POR_VEZ.lock();
        // sem pasta do jogo (fora do processo do jogo) a gravação não pode explodir
        ESCALA_DE_RENDER.store(90, Ordering::Relaxed);
        QUALIDADE.store(2, Ordering::Relaxed);
        gravar_ajustes(); // não deve entrar em pânico nem no ambiente de teste
        assert_eq!(ESCALA_DE_RENDER.load(Ordering::Relaxed), 90);
        ESCALA_DE_RENDER.store(100, Ordering::Relaxed);
        QUALIDADE.store(1, Ordering::Relaxed);
    }
}
