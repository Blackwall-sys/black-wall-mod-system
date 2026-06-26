//! IA-no-jogo — Fase 0: loop de I/O game <-> processo externo (SEM LLM ainda).
//!
//! Arquitetura (roadmap cp77-ai-npc-roadmap): a native roda NA THREAD DO JOGO e NUNCA
//! pode bloquear (lição do deadlock do spinlock do TweakDB). Então `BwmsEmit` só
//! ENFILEIRA (escreve um arquivo no disco) e retorna na hora; um processo EXTERNO
//! (bwms-ai-agent) faz o trabalho lento (futuro: LLM) e escreve a resposta; o
//! `cp77_tick` faz POLL do arquivo de resposta (non-blocking) e loga/exibe.
//!
//! Fase 0 prova só o TRANSPORTE redondo (ping->pong), reusando a ponte
//! redscript->native (register.rs) já provada. Fase 1+ troca o agente por LLM real.

use std::sync::atomic::{AtomicU64, Ordering};

const DIR: &str = "/tmp/bwms-ai";

static EMIT_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_RESP_HASH: AtomicU64 = AtomicU64::new(0);
static TICK: AtomicU64 = AtomicU64::new(0);

fn fnv1a(s: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Chamado pela native `BwmsEmit` (do redscript) — ENFILEIRA um evento p/ o agente
/// externo e retorna IMEDIATO (non-blocking; nunca espera o LLM).
pub fn emit_event() {
    let n = EMIT_SEQ.fetch_add(1, Ordering::Relaxed) + 1;
    let _ = std::fs::create_dir_all(DIR);
    let payload = format!("ping {n}\n");
    let _ = std::fs::write(format!("{DIR}/event.txt"), &payload);
    crate::log(&format!("[ai] BwmsEmit -> {DIR}/event.txt (ping {n})"));
}

/// Chamado todo `cp77_tick`: POLL non-blocking do arquivo de resposta. Se mudou desde
/// a última vez, loga (Fase 0 = prova de transporte). Throttle p/ não ler todo frame.
pub fn poll_response() {
    let t = TICK.fetch_add(1, Ordering::Relaxed);
    if t % 30 != 0 {
        return; // ~2x/seg a 60fps
    }
    if let Ok(s) = std::fs::read_to_string(format!("{DIR}/response.txt")) {
        let h = fnv1a(&s);
        if h != LAST_RESP_HASH.swap(h, Ordering::Relaxed) && LAST_RESP_HASH.load(Ordering::Relaxed) != 0 {
            let resp = s.trim().to_string();
            crate::log(&format!("[ai] RESPOSTA do processo externo: \"{resp}\""));
            // publica p/ o overlay exibir (Fase 0.5)
            set_last_response(&resp);
        }
    }
}

// Última resposta p/ o overlay desenhar (Fase 0.5). Mutex simples.
static LAST_RESPONSE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

fn set_last_response(s: &str) {
    if let Ok(mut g) = LAST_RESPONSE.lock() {
        *g = s.to_string();
    }
}

/// Para o overlay (overlay.rs) exibir a última resposta da IA, se houver.
pub fn last_response() -> String {
    LAST_RESPONSE.lock().map(|g| g.clone()).unwrap_or_default()
}
