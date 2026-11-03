//! Estado de energia do boot: modo de baixo consumo, tomada ou bateria e estado térmico, lidos no começo do boot e gravados em
//! `~/.bwms-boot-energia`. O arquivo sai no fim do load do save (junto do dead-man's switch) e na saída limpa; se o boot seguinte ainda o
//! acha, o anterior não terminou, e o relatório de crash diz em que condição ele rodou — mesmo sem `.ips`.
//!
//! Leitura só pelo `NSProcessInfo` (Foundation) e pelo `IOPSGetTimeRemainingEstimate` achado por `dlsym` (sem linkar framework novo:
//! símbolo ausente vira "desconhecido"). Nada vai ao save; o arquivo não tem identidade.

use std::ffi::c_void;
use std::os::raw::c_char;

use crate::macos::{dlsym, msg0, objc_getClass, objc_msgSend, sel, Id, Sel, RTLD_DEFAULT};

/// O que o boot leu do sistema. `None` = o sistema não respondeu.
pub(crate) struct EstadoDeEnergia {
    pub(crate) baixo_consumo: Option<bool>,
    pub(crate) tomada: Option<bool>,
    /// `NSProcessInfoThermalState`: 0 nominal, 1 justo, 2 sério, 3 crítico.
    pub(crate) termico: Option<i64>,
}

/// Lê o estado agora. Só mensagens ao `NSProcessInfo`, que é seguro fora da thread principal; nada ao AppKit.
pub(crate) unsafe fn ler() -> EstadoDeEnergia {
    let mut e = EstadoDeEnergia { baixo_consumo: None, tomada: None, termico: None };
    let classe = objc_getClass(b"NSProcessInfo\0".as_ptr() as *const c_char);
    if !classe.is_null() {
        let pi = msg0(classe as Id, sel!("processInfo"));
        if !pi.is_null() {
            let responde: extern "C" fn(Id, Sel, Sel) -> i8 = std::mem::transmute(objc_msgSend as *const c_void);
            if responde(pi, sel!("respondsToSelector:"), sel!("isLowPowerModeEnabled")) != 0 {
                let f: extern "C" fn(Id, Sel) -> i8 = std::mem::transmute(objc_msgSend as *const c_void);
                e.baixo_consumo = Some(f(pi, sel!("isLowPowerModeEnabled")) != 0);
            }
            if responde(pi, sel!("respondsToSelector:"), sel!("thermalState")) != 0 {
                let f: extern "C" fn(Id, Sel) -> i64 = std::mem::transmute(objc_msgSend as *const c_void);
                e.termico = Some(f(pi, sel!("thermalState")));
            }
        }
    }
    // `kIOPSTimeRemainingUnlimited` (-2) = na tomada; -1 = ainda calculando (na bateria); > 0 = minutos de bateria.
    let restante = dlsym(RTLD_DEFAULT, b"IOPSGetTimeRemainingEstimate\0".as_ptr() as *const c_char);
    if !restante.is_null() {
        let f: extern "C" fn() -> f64 = std::mem::transmute(restante);
        let t = f();
        e.tomada = Some(t == -2.0);
    }
    e
}

fn sim_nao(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "sim",
        Some(false) => "nao",
        None => "?",
    }
}

/// Linha `chave=valor` do registro, a mesma no arquivo e no veredito.
pub(crate) fn linha(e: &EstadoDeEnergia, modo: &str) -> String {
    format!(
        "baixo_consumo={} tomada={} termico={} modo={modo}",
        sim_nao(e.baixo_consumo),
        sim_nao(e.tomada),
        e.termico.map_or("?".to_string(), |t| t.to_string())
    )
}

fn caminho() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".bwms-boot-energia"))
}

/// O registro que o boot anterior deixou (ele não chegou ao fim do load nem à saída limpa), ou `None`. Chamar antes de `gravar`.
pub(crate) fn do_boot_anterior() -> Option<String> {
    let texto = std::fs::read_to_string(caminho()?).ok()?;
    let texto = texto.trim();
    (!texto.is_empty()).then(|| texto.to_string())
}

/// `inicio` e modo da gravação deste boot: o acompanhamento regrava o registro com eles.
static GRAVADO: std::sync::OnceLock<(u64, String)> = std::sync::OnceLock::new();

/// Grava o registro deste boot (`inicio=<segundos unix>` + a linha).
pub(crate) fn gravar(e: &EstadoDeEnergia, modo: &str) {
    let Some(p) = caminho() else { return };
    let inicio = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let _ = GRAVADO.set((inicio, modo.to_string()));
    let _ = std::fs::write(p, format!("inicio={inicio} {}\n", linha(e, modo)));
}

/// O que o acompanhamento juntou: pior estado térmico, trocas entre tomada e bateria, e a última linha gravada.
struct Acompanhamento {
    pior_termico: i64,
    tomada: Option<bool>,
    trocas_de_fonte: u32,
    ultima: String,
}
static ACOMPANHAMENTO: std::sync::Mutex<Option<Acompanhamento>> = std::sync::Mutex::new(None);

/// Durante o boot (thread do hb, a cada 2 s): relê energia, fase do boot e o `isWindowVisible` do motor e regrava o registro só quando a
/// linha muda. Para sozinho quando o registro some (fim do load ou saída limpa). Só `NSProcessInfo`, nada de AppKit fora da main.
pub(crate) unsafe fn acompanhar() {
    let (Some(p), Some((inicio, modo))) = (caminho(), GRAVADO.get()) else { return };
    if !p.exists() {
        return;
    }
    let e = ler();
    let fase = crate::inicio::boot::FASE_LIDA.load(std::sync::atomic::Ordering::Relaxed);
    let byte = crate::rebase(0x1_07d8_4c40) as *const u8;
    let janela = if crate::memoria::is_readable(byte as *const c_void, 1) { byte.read_volatile().to_string() } else { "?".into() };
    let Ok(mut guarda) = ACOMPANHAMENTO.lock() else { return };
    let a = guarda.get_or_insert(Acompanhamento { pior_termico: -1, tomada: e.tomada, trocas_de_fonte: 0, ultima: String::new() });
    a.pior_termico = a.pior_termico.max(e.termico.unwrap_or(-1));
    if e.tomada.is_some() && a.tomada.is_some() && e.tomada != a.tomada {
        a.trocas_de_fonte += 1;
    }
    if e.tomada.is_some() {
        a.tomada = e.tomada;
    }
    let texto = format!(
        "inicio={inicio} {} pior_termico={} trocas_de_fonte={} fase={fase} janela_visivel={janela}\n",
        linha(&e, modo),
        a.pior_termico,
        a.trocas_de_fonte
    );
    if texto != a.ultima {
        let _ = std::fs::write(&p, &texto);
        crate::log_dev!("[energia] {}", texto.trim_end());
        a.ultima = texto;
    }
}

/// O boot chegou ao fim do load ou saiu limpo: o registro cumpriu o papel.
pub(crate) fn apagar() {
    if let Some(p) = caminho() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod testes {
    use super::{linha, EstadoDeEnergia};

    #[test]
    fn linha_marca_o_que_o_sistema_nao_respondeu() {
        let e = EstadoDeEnergia { baixo_consumo: Some(true), tomada: None, termico: Some(2) };
        assert_eq!(linha(&e, "continue"), "baixo_consumo=sim tomada=? termico=2 modo=continue");
    }
}
