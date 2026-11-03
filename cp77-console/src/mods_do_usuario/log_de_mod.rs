//! Log de mod no build público: o `Print`/`ModLog` dos scripts e o log dos plugins nativos vão pra `<jogo>/r6/logs/bwms-mods.log` —
//! só se esse arquivo existir (criar o arquivo liga; apagar desliga). Sem ele, nada é gravado: o BWMS não deixa arquivo na máquina de
//! quem só joga. O diagnóstico do próprio BWMS (`crate::log`) continua só no build de desenvolvimento.
//!
//! Dois limites, porque volume de I/O já derrubou o jogo (2026-08-20): o arquivo recomeça vazio quando passa de 8 MB na abertura, e cada
//! segundo aceita no máximo 200 linhas (o resto do segundo é descartado, com uma linha avisando quantas foram).

use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

const TETO_BYTES: u64 = 8 * 1024 * 1024;
const LINHAS_POR_SEGUNDO: u64 = 200;

static ARQUIVO: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
/// (segundo corrente, linhas nesse segundo, descartadas nesse segundo), empacotados: segundo << 32 | linhas << 16 | descartadas.
/// Lida e escrita só com a trava do `ARQUIVO`.
static JANELA: AtomicU64 = AtomicU64::new(0);

fn arquivo() -> Option<&'static Mutex<std::fs::File>> {
    ARQUIVO
        .get_or_init(|| {
            let dir = crate::arquivos_do_jogo::game_dir_from_dylib()?;
            let caminho = std::path::Path::new(&dir).join("r6/logs/bwms-mods.log");
            let tamanho = std::fs::metadata(&caminho).ok()?.len();
            let f = if tamanho > TETO_BYTES {
                std::fs::File::create(&caminho).ok()?
            } else {
                std::fs::OpenOptions::new().append(true).open(&caminho).ok()?
            };
            Some(Mutex::new(f))
        })
        .as_ref()
}

/// Teste ao vivo (`~/.bwms-hook-log-teste`, com o `r6/logs/bwms-mods.log` criado antes do boot): o `BwmsLogTestePasso` do script chama
/// `ModLog` e `Print` com um token, e as duas linhas têm que estar no arquivo. O arquivo é apagado no fim (é da máquina do usuário).
#[cfg(feature = "devtools")]
pub(crate) fn tick_teste() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: AtomicU64 = AtomicU64::new(0);
    static CHAMOU: OnceLock<std::time::Instant> = OnceLock::new();
    let fase = FASE.load(Ordering::Relaxed);
    if fase >= 2 || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("log-teste") {
        return;
    }
    if !crate::inicio::boot::gameplay_real() {
        return;
    }
    const TOKEN: &str = "bwms-log-teste-263";
    if fase == 0 {
        let player = crate::current_player();
        if player.is_null() {
            return;
        }
        let r = unsafe {
            crate::rtti::resolve_in_class(crate::rtti::class_of(player), "BwmsLogTestePasso")
                .and_then(|rf| crate::rtti::call_func(&rf, player, &[crate::rtti::Arg::I32(1)]))
                .is_some()
        };
        crate::veredito_dev!("[logm] script chamado {r}");
        let _ = CHAMOU.set(std::time::Instant::now());
        FASE.store(1, Ordering::Relaxed);
        return;
    }
    if CHAMOU.get().is_none_or(|c| c.elapsed().as_secs() < 3) {
        return;
    }
    FASE.store(2, Ordering::Relaxed);
    let caminho = crate::arquivos_do_jogo::game_dir_from_dylib().map(|d| std::path::Path::new(&d).join("r6/logs/bwms-mods.log"));
    let texto = caminho.as_ref().and_then(|c| std::fs::read_to_string(c).ok()).unwrap_or_default();
    let modlog = texto.contains(&format!("[BwmsTesteLog] linha do mod {TOKEN}"));
    let print = texto.contains(&format!("[DEBUG] linha do script {TOKEN}"));
    let ligado = arquivo().is_some();
    if let Some(c) = &caminho {
        let _ = std::fs::remove_file(c);
    }
    crate::veredito_dev!(
        "[logm] VEREDITO-LOGM arquivo ligado {ligado}, {} B lidos; ModLog no arquivo {modlog}; Print no arquivo {print}; arquivo apagado no fim; \
         log-ok={}",
        texto.len(),
        ligado && modlog && print
    );
}

/// Uma linha de log de mod: `origem` é o nome do mod (`ModLog`), `script` (`Print`) ou o plugin.
pub(crate) fn gravar(origem: &str, texto: &str) {
    crate::log_dev!("[{origem}] {texto}");
    let Some(f) = arquivo() else { return };
    let agora = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Espera a trava em vez de desistir: a linha não some por disputa entre threads, e a JANELA só muda com a trava na mão
    // (lida antes dela, uma thread sobrescreve a contagem da outra).
    let mut f = f.lock().unwrap_or_else(|e| e.into_inner());
    let j = JANELA.load(Ordering::Relaxed);
    let (seg, linhas, descartadas) = (j >> 32, (j >> 16) & 0xffff, j & 0xffff);
    if seg != agora & 0xffff_ffff {
        if descartadas > 0 {
            let _ = writeln!(f, "[bwms] {descartadas} linha(s) de log descartada(s) no segundo anterior (limite {LINHAS_POR_SEGUNDO}/s)");
        }
        JANELA.store(((agora & 0xffff_ffff) << 32) | (1 << 16), Ordering::Relaxed);
    } else if linhas >= LINHAS_POR_SEGUNDO {
        JANELA.store((seg << 32) | (linhas << 16) | (descartadas + 1).min(0xffff), Ordering::Relaxed);
        return;
    } else {
        JANELA.store((seg << 32) | ((linhas + 1) << 16) | descartadas, Ordering::Relaxed);
    }
    let _ = writeln!(f, "[{origem}] {texto}");
}
