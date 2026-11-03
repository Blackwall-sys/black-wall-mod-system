//! cp77-console — runtime de mods do Black Wall Mod System, 100% Rust (sem JS/Lua),
//! carregado pelo Cyberpunk 2077 do macOS via LC_LOAD_DYLIB.
//!
//! Os ganchos são Rust próprio (`ganchos`) e a leitura segura de memória está em `memoria`. Cobre o
//! console/RTTI, cheats, TweakDB, a página de configurações e o self-boot nativo.
#![allow(dead_code)] // esqueleto: vários itens só passam a ser usados com os hooks

mod cname;
mod enderecos;
#[macro_use]
mod resposta_ao_usuario;
mod register;
mod rtti;

mod inicio;
/// NPCs: comunidades, spawners e entidades dinâmicas.
mod npcs;
/// Roupas e armas: componentes de roupa, equipar, guarda-roupa.
mod roupas_e_armas;
/// Corpo e rosto: menu de personalização, morph targets e estado do puppet (braços, pernas, corpo inteiro).
mod corpo_e_rosto;
/// Mundo: clima, distrito, setores de streaming, nós da cena, traçado de raio e estado do mundo.
mod mundo;
/// Interface: widgets e camadas ink, texto editável, clipboard.
mod interface;
/// Aparência: malhas, componentes, render proxies, controladores visuais.
mod aparencia;
/// Entidades: entidade, objeto de jogo, tags, spawn por script e os sistemas de entidade.
mod entidades;
/// Missões e mapa: diário, fases de missão, pinos do mapa.
mod missoes_e_mapa;
/// Status do personagem: vida, stamina, nível, pontos e invencibilidade do V (os cheats do terminal).
mod status_do_personagem;
/// Memória do processo: ler sem crashar em ponteiro inválido (mach_vm_read_overwrite) e alocar fora do alocador do jogo.
mod memoria;
/// Ganchos: desviar código do jogo (hook inline arm64, trampolim, relocação do prólogo) e trocar entradas de tabela.
mod ganchos;
/// Mods do usuário: achar, carregar, versão e log dos mods instalados, API de plugin e compatibilidade de script.
mod mods_do_usuario;
/// Diagnóstico: provas, sondas, medição, dumps e ferramentas de RE. Não é produto.
mod diagnostico;
/// Tipos do motor: reflexão (classes, propriedades, funções, enums), chamar métodos, callbacks, overrides e sistemas por nome.
mod tipos_do_motor;
/// Arquivos do jogo: recursos (.ent, .mesh, .app), referências, tokens de carga, depot e existência de arquivo nos archives.
mod arquivos_do_jogo;
/// Texto e números: strings UTF-8, leitura de inteiros, bits e os hashes do jogo (FNV-1a, Murmur3, CRUID, EntityID, NodeRef).
mod texto_e_numeros;
/// Salvamento: gravar e recriar entidades com o save, serializar objetos, estado de persistência.
mod salvamento;
/// Configuração do mod: ler e gravar opções do usuário.
mod configuracao;
/// Entrada: eventos de teclado, eixo e caractere vindos do jogador.
mod entrada;
/// Tarefas (jobs) do motor: criar fila, despachar, esperar e finalizar.
mod tarefas;
/// Tempo do motor: duração do quadro, dilatação do tempo e conversão entre tempo e ticks.
mod tempo;
/// Curvas de dados do jogo: tipo, modo, pontos e valores.
mod curvas;
/// Som: bancos de som (soundbanks) carregados, caminho e residência.
mod som;
/// Realidade virtual (CPVR): modo estéreo, pose da cabeça e das mãos do headset, gatilho.
mod vr;
/// Câmera: terceira pessoa, olhar forçado, modo de visão (braindance), foto e captura de tela.
mod camera;
/// Veículos: estado de direção (aceleração, derrapagem, ré, no chão) e a garagem.
mod veiculos;
/// Scripts: compilar os .reds, ler o final.redscripts, traduzir bytecode e interceptar funções de script (antes, depois, no lugar).
mod scripts;
/// Terminal: a UI própria do BWMS por cima do quadro do jogo (overlay, widgets em modo imediato, malha de desenho, catálogo de comandos).
mod terminal;
/// Chamadas do macOS usadas por mais de um módulo (dyld, dl, Objective-C, mach_vm, sysctl), declaradas uma vez.
mod macos;
/// As peças do jogo que o mod consulta, um nome por endereço (ver `jogo/mod.rs`).
mod jogo;
pub(crate) mod relatorio_de_crash;
#[cfg(feature = "devtools")]
mod selftest;
// Build público: o `selftest` (837 KB de testes no binário) sai e o stub entra no lugar.
#[cfg(not(feature = "devtools"))]
#[path = "selftest_stub.rs"]
mod selftest;
mod tweakdb;
#[cfg(feature = "lua")]
mod mods_lua;
mod localizacao;

use crate::tempo::*;
use crate::inicio::boot::*;
#[cfg(feature = "cpvr")]
use crate::vr::*;
use crate::texto_e_numeros::*;
use crate::mods_do_usuario::*;
use crate::mods_do_usuario::api::EstadoDoJogo;
use crate::arquivos_do_jogo::*;
use crate::entrada::*;
use crate::diagnostico::{
    prove_attach_detach, prove_copy_tick, prove_derive, prove_relocreal, ATTACHDETACH_DONE, COPYTEST_ARMED, COPYTEST_DONE, COPY_PHASE,
    DERIVETEST_DONE, EQUIP_DEFERRED_DONE, RELOCREAL_DONE,
};
// Só os comandos de RE e a prova do executor usam, e os dois só existem no dev.
#[cfg(feature = "devtools")]
use crate::diagnostico::{
    create_archive_group_by_path, depotres_report, fnoverride_test_after, fnoverride_test_before, fnoverride_test_replace,
    resolve_archive_group_by_path, run_archivegroupdump,
};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, AtomicUsize, Ordering};

// Constructor do macOS: roda quando a .dylib é carregada no processo do jogo.
// Sem a crate `ctor` — usa a seção __mod_init_func direto (zero-dep).
#[link_section = "__DATA,__mod_init_func"]
#[used]
static CTOR: extern "C" fn() = on_load;

use crate::macos::{_dyld_get_image_header, _dyld_get_image_name, _dyld_image_count};

/// Endereço de carga do binário PRINCIPAL do jogo, achado por NOME (robusto —
/// `_dyld_get_image_vmaddr_slide(0)` devolveu o slide da NOSSA dylib, não o do
/// jogo, quando carregada via Module.load/dlopen).
pub(crate) fn game_base() -> usize {
    unsafe {
        let n = _dyld_image_count();
        for i in 0..n {
            let name = _dyld_get_image_name(i);
            if name.is_null() {
                continue;
            }
            if let Ok(s) = std::ffi::CStr::from_ptr(name).to_str() {
                if s.ends_with("/Cyberpunk2077") || s.ends_with("Contents/MacOS/Cyberpunk2077") {
                    return _dyld_get_image_header(i) as usize;
                }
            }
        }
        _dyld_get_image_header(0) as usize // fallback: imagem principal
    }
}

/// VM addr do binário (base 0x1_0000_0000) → endereço real em runtime.
///
/// Os offsets estáticos do projeto são da build STEAM 2.5.0: no Steam, identidade. Fora do Steam 2.5.0 o `on_load` não instala gancho
/// (`build_supported`); se algo chama o `rebase` mesmo assim, o endereço é traduzido pela tabela de assinaturas resolvida na imagem da memória
/// (`enderecos`) e, sem tradução, volta nulo, que todo call-site trata como "não instala". Nunca o endereço Steam cru num binário de
/// outro layout: em 2026-07-31 um vmaddr Steam no binário GOG caiu em bytes que pareciam um prólogo e o gancho corrompeu código real.
pub(crate) fn rebase(vmaddr: u64) -> *mut c_void {
    // Endereço fora da imagem (literal com dígito a mais, constante que não é endereço) vira nulo em vez de um salto pra lixo.
    if vmaddr < bwms_primitives::enderecos::LINK_BASE || vmaddr >= enderecos::fim_da_imagem() {
        #[cfg(feature = "devtools")]
        REBASE_SEM_MAPA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::log_dev!("[rebase] vmaddr {vmaddr:#x} fora da imagem do jogo -> SKIP (null)");
        return core::ptr::null_mut();
    }
    // O teste `~/.bwms-hook-enderecos-teste` manda o Steam também pela tabela.
    #[cfg(feature = "devtools")]
    let steam_direto = game_build() == GameBuild::Steam && !enderecos::forcar_resolvedor();
    #[cfg(not(feature = "devtools"))]
    let steam_direto = game_build() == GameBuild::Steam;
    let traduzido = if steam_direto {
        Some(vmaddr)
    } else if game_build() == GameBuild::Gog {
        enderecos::traduzir_gog_25(vmaddr)
    } else {
        enderecos::traduzir(vmaddr)
    };
    let Some(v) = traduzido else {
        #[cfg(feature = "devtools")]
        REBASE_SEM_MAPA.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        crate::log_dev!("[rebase] vmaddr {vmaddr:#x} sem tradução neste build -> SKIP (null)");
        return core::ptr::null_mut();
    };
    (game_base() + (v - bwms_primitives::enderecos::LINK_BASE) as usize) as *mut c_void
}

/// Endereço dentro da imagem do jogo convertido pra VM (o que o disassembly mostra); fora dela, devolve o valor cru.
pub(crate) fn vm_do_jogo(p: u64) -> u64 {
    let base = crate::rebase(0x1_0000_0000) as u64;
    if p >= base && p < base + 0x0b00_0000 { p - base + 0x1_0000_0000 } else { p }
}

/// Inverso de `rebase`: ponteiro runtime → VM addr estático do binário (p/ casar com
/// `nm`/símbolos no diagnóstico de vtable). 0 se o ponteiro estiver fora do módulo.
pub(crate) fn un_rebase(ptr: *const c_void) -> u64 {
    let p = ptr as usize;
    let b = game_base();
    if p < b {
        return 0;
    }
    (p - b) as u64 + bwms_primitives::enderecos::LINK_BASE
}

/// Qual build do jogo: `Steam` = o prólogo do executor está no vmaddr do Steam 2.5.0, o layout de todos os offsets do código; `Gog` = o
/// prólogo está no vmaddr GOG 2.5.0 do executor (a mesma versão compilada à parte: os mesmos layouts, as funções em outra ordem, traduzidas
/// pela tabela fixa `enderecos::traduzir_gog_25`). Qualquer outro binário (Epic, outra versão) desloca vmaddr e layouts, e é `Unknown`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum GameBuild {
    Steam,
    Gog,
    Unknown,
}

/// Build detectado (cacheado), lido 1x no prólogo do executor.
pub(crate) fn game_build() -> GameBuild {
    use std::sync::OnceLock;
    static BUILD: OnceLock<GameBuild> = OnceLock::new();
    *BUILD.get_or_init(|| unsafe {
        let base = game_base();
        let probe = |vm: u64| -> bool {
            let p = (base + (vm - bwms_primitives::enderecos::LINK_BASE) as usize) as *const c_void;
            crate::memoria::is_readable(p, 8) && (p as *const u64).read_unaligned() == crate::jogo::scripts::EXECUTOR_PROLOGO
        };
        let b = if probe(crate::jogo::scripts::EXECUTOR_VM) {
            GameBuild::Steam
        } else if enderecos::traduzir_gog_25(crate::jogo::scripts::EXECUTOR_VM).is_some_and(probe) {
            GameBuild::Gog
        } else {
            GameBuild::Unknown
        };
        crate::log_dev!("[build] detectado: {b:?} (game_base={base:#x})");
        b
    })
}

/// Flag: `build_supported()` deu falso no `on_load` (nenhum gancho de endereço instalado). Sem leitor no crate.
pub(crate) static BUILD_UNSUPPORTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// True no Steam 2.5.0 e no GOG 2.5.0: todos os layouts do código (frame 0xA8, POD 0x100, `ScriptClass` +0x28) são da 2.5.0, e um binário de
/// outra versão com o endereço traduzido ainda leria struct errada. Fora deles o gate do `on_load` não instala nenhum gancho e o jogo boota
/// vanilla.
pub(crate) fn build_supported() -> bool {
    matches!(game_build(), GameBuild::Steam | GameBuild::Gog)
}

/// Quantos endereços o `rebase` não achou neste build (o veredito do teste de endereços lê).
#[cfg(feature = "devtools")]
pub(crate) static REBASE_SEM_MAPA: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Uma anomalia é DIGNA DE LOG quando o objeto é POLIMÓRFICO e mesmo assim saiu do `Construct`
/// com ponteiro de vtable NULO.
///
/// A distinção importa e é a razão de isto ser função pura testável em vez de um `if` solto:
/// para quem deriva de `IScriptable` o offset 0 é o ponteiro de vtable (obrigatoriamente
/// não-nulo — é objeto C++ com métodos virtuais), mas para um struct PURO (`Vector4` etc.) o
/// offset 0 é DADO comum, e `x = 0.0` é um valor perfeitamente legítimo. Uma checagem cega em
/// `*obj == 0` acusaria todo `Vector4` zerado como defeito.
pub(crate) fn null_vtable_is_anomalous(derives_iscriptable: bool, vtable_word: usize) -> bool {
    derives_iscriptable && vtable_word == 0
}

/// Canal de VEREDITO — nunca descartado. Escreve em `/tmp/bwms-vereditos.log`, fora do orçamento
/// do `log()`, e também na trilha normal.
///
/// **Por que existe (medido 2026-09-04, depois de custar 3 boots).** O `log()` tem orçamento
/// verbatim de 20.000 linhas mesmo com `~/.bwms-log-full` ligado; passou disso, volta a descartar 7
/// de 8. E a 2ª firing de `OnGameAttached` — a que roda com o save REAL carregado, ou seja, a única
/// que interessa — cai por volta da linha 20.650 num boot típico. Resultado: o veredito do smoke
/// que mais importa é justamente o que tem mais chance de ser jogado fora, em silêncio, e "0
/// linhas" fica indistinguível de um achado negativo. Aconteceu 3 vezes nesta sessão.
///
/// Regra prática: contagem e diagnóstico vão por `log()`; **veredito vai por aqui**.
pub(crate) fn log_veredito(msg: &str) {
    #[cfg(not(feature = "devlog"))]
    {
        let _ = msg;
    }
    #[cfg(feature = "devlog")]
    {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/bwms-vereditos.log")
        {
            // Uma escrita só, com a quebra junto: o `writeln!` escrevia em duas e as linhas de threads de job saíam coladas (boot 107).
            let _ = f.write_all(format!("{msg}\n").as_bytes());
        }
        log(msg);
    }
}

/// Log de diagnóstico do dev. **Macro, não função**: sem a feature `devlog` o `format!` não roda e as strings de diagnóstico
/// não entram no binário público. Os argumentos ficam num closure nunca chamado: o compilador os vê usados (sem
/// aviso falso de variável morta) e o otimizador tira o closure.
#[macro_export]
macro_rules! log_dev {
    ($($arg:tt)*) => {{
        #[cfg(feature = "devlog")]
        $crate::log(&format!($($arg)*));
        #[cfg(not(feature = "devlog"))]
        let _ = || {
            let _ = format_args!($($arg)*);
        };
    }};
}

/// `~/<nome>` existe agora (sem cache). Só no dev: o público não lê marcador de teste (a chave do usuário vive no `bwms.toml`).
pub(crate) fn marcador_na_home(nome: &str) -> bool {
    if !cfg!(feature = "devtools") {
        return false;
    }
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(nome).exists()).unwrap_or(false)
}

/// Marcador `~/<nome>` lido UMA vez por processo (`OnceLock` próprio de cada uso): o caminho quente não paga syscall
/// por chamada. Macro, não função, para cada marcador ter o seu `static` e o código gerado ser o de uma função própria.
#[macro_export]
macro_rules! marcador_do_processo {
    ($nome:literal) => {{
        static LIDO: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *LIDO.get_or_init(|| $crate::marcador_na_home($nome))
    }};
}

/// Veredito de prova (`/tmp/bwms-vereditos.log`) com argumentos — mesma regra do [`log_dev`]: no
/// build público a chamada não existe. O canal de veredito é do boot de teste, não do usuário final.
#[macro_export]
macro_rules! veredito_dev {
    ($($arg:tt)*) => {{
        #[cfg(feature = "devlog")]
        $crate::log_veredito(&format!($($arg)*));
        #[cfg(not(feature = "devlog"))]
        let _ = || {
            let _ = format_args!($($arg)*);
        };
    }};
}

/// Teto de log: verdadeiro nas `n` primeiras chamadas contadas por `contador`. Conta também depois do teto: vários desses contadores
/// são o total que o veredito dos testes lê.
#[inline]
pub(crate) fn primeiras(contador: &AtomicU64, n: u64) -> bool {
    contador.fetch_add(1, Ordering::Relaxed) < n
}

/// Caminho de um marcador de dev (`~/.bwms-*`) — **`None` no build público**, então o bloco do
/// chamador nem toca em disco. Era `env::var("HOME")` + `join` + `stat` por frame, dentro do tick,
/// procurando arquivo que só existe na máquina de teste.
#[inline]
pub(crate) fn caminho_de_marcador(nome: &str) -> Option<std::path::PathBuf> {
    if !cfg!(feature = "devtools") {
        return None;
    }
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(nome))
}

/// Marcadores de dev presentes na HOME, por VARREDURA (`~/.bwms-*` e `~/.cpvr-*`), revista a cada
/// [`INTERVALO_DA_VARREDURA`]. O gate de cada teste fazia `env::var("HOME")` + `format!` + `stat` por
/// chamada; com ~130 gates por frame no build de dev isso media **58,1 µs e 130 syscalls por frame**
/// (bench na máquina, 600 frames) procurando arquivos que quase nunca existem. Um `read_dir` por meio
/// segundo custa 60 µs e responde a todos: **2,1 µs por frame amortizado**.
#[cfg(feature = "devtools")]
pub(crate) static MARCADORES_NA_HOME: std::sync::Mutex<Option<(std::time::Instant, Vec<Box<str>>)>> =
    std::sync::Mutex::new(None);

/// Meio segundo: o marcador é criado por mão/script antes ou durante o boot e lido por muitos frames
/// seguidos, então meia dúzia de frames de atraso na detecção não muda nenhum teste.
#[cfg(feature = "devtools")]
pub(crate) const INTERVALO_DA_VARREDURA: u128 = 500;

#[cfg(feature = "devtools")]
pub(crate) fn com_a_varredura<R>(f: impl FnOnce(&[Box<str>]) -> R) -> R
where
    R: Default,
{
    let Ok(mut idx) = MARCADORES_NA_HOME.lock() else {
        return R::default();
    };
    if idx
        .as_ref()
        .is_none_or(|(t, _)| t.elapsed().as_millis() >= INTERVALO_DA_VARREDURA)
    {
        let mut v: Vec<Box<str>> = Vec::new();
        if let Ok(home) = std::env::var("HOME") {
            if let Ok(rd) = std::fs::read_dir(&home) {
                for e in rd.flatten() {
                    if let Some(s) = e.file_name().to_str() {
                        if s.starts_with(".bwms") || s.starts_with(".cpvr") {
                            v.push(s.into());
                        }
                    }
                }
            }
        }
        *idx = Some((std::time::Instant::now(), v));
    }
    match idx.as_ref() {
        Some((_, v)) => f(v),
        None => R::default(),
    }
}

#[cfg(not(feature = "devtools"))]
#[inline(always)]
pub(crate) fn marcador_de_dev(_nome: &str) -> bool {
    false
}

#[cfg(not(feature = "devtools"))]
#[inline(always)]
pub(crate) fn marcador_individual(_nome: &str) -> bool {
    false
}

pub(crate) fn log(msg: &str) {
    // Build PÚBLICO (sem feature `devlog`): silencioso — não escreve /tmp/cp77-console.log nem
    // trace.log (o usuário final não precisa dos diagnósticos; um mod escrevendo em /tmp a cada
    // frame é comportamento desnecessário). Dev liga via `devtools`. Chamada direta `log(&format!(..))` ainda monta a string e
    // deixa o literal no binário público; diagnóstico vai por `log_dev!`, que some sem `devlog`.
    #[cfg(not(feature = "devlog"))]
    {
        let _ = msg;
        return;
    }
    #[cfg(feature = "devlog")]
    {
    use std::io::Write;
    // PERF (2026-08-17, achado do dia — mesma classe de bug já corrigida em `route_native`/
    // `capture()`): `log()` é chamado de ~324 call-sites, incl. o HOT-PATH de `exec_replacement`
    // e seus vizinhos (`AnimationSystem_FrameBeginReset`, ticks de GameState de plugin) — sob
    // carga pesada (streaming de boot, rajada de eventos) isso dispara MILHARES de vezes/segundo.
    // A versão antiga reabria o arquivo (`open+append+close`, syscalls de verdade) TODA CHAMADA —
    // custo que composto nessa escala pode facilmente virar minutos de I/O bloqueante, mesmo
    // padrão que já explicou o travamento do `route_native` O(n) e o `capture()` sem rate-limit.
    // Fix: handle cacheado (aberto 1x, `O_APPEND` garante posicionamento correto mesmo se algo
    // externo truncar o arquivo entre boots — cada processo novo abre um handle novo de qualquer
    // forma). `try_lock`: se contendido (log reentrante/concorrente), pula silenciosamente em vez
    // de bloquear — perder uma linha de log é aceitável, travar o hot-path não é.
    //
    // RATE-LIMIT GLOBAL (2026-08-20, madrugada, achado desta sessão): build de teste inteira desta
    // madrugada rodou com `devtools`(=`devlog` ligado) e reproduziu SIGSEGV+travamento de memória
    // consistentemente (≥8 tentativas); um boot idêntico com `devlog` OFF sobreviveu 570s+ sem
    // crash nem travamento severo, mesma memória crítica do sistema — forte indício de que o
    // VOLUME de I/O deste log (2 escritas por chamada em dev_mode: console.log+trace.log, cada
    // `writeln!` é 1 syscall real mesmo com handle cacheado) contribui pra fragilidade sob pressão,
    // mesmo sem ser a causa raiz da pressão em si. Fix cirúrgico: mantém verbatim as primeiras
    // ~800 chamadas do processo (cobre o boot inicial, onde o diagnóstico de sequência importa
    // mais), depois passa a escrever só 1 em cada 8 — corta ~87% do volume em regime de alta
    // atividade (exatamente a janela onde o crash/travamento historicamente acontece) sem perder
    // a capacidade de diagnóstico (amostragem ainda mostra a sequência, só mais esparsa).
    static LOG_CALL_COUNT: AtomicU64 = AtomicU64::new(0);
    let n = LOG_CALL_COUNT.fetch_add(1, Ordering::Relaxed);
    // Orçamento de linhas VERBATIM antes do 1-em-8 começar: 800 no boot normal, 20000 quando o
    // marcador de diagnóstico está armado. **É um TETO, não "sem limite"** — desligar o throttle
    // por completo devolve o firehose de I/O que correlacionou com fragilidade de boot em
    // 2026-08-20 (e um boot de 2026-08-21 com log irrestrito ficou visivelmente mais lento pra
    // chegar na engagement). 20000 cobre com folga a janela de um veredito de smoke (um boot
    // inteiro com throttle deu ~1375 linhas) sem virar firehose.
    // `~/.bwms-log-full` arma os 20000: com o 1-em-8, "0 linhas" de um smoke não distingue "não rodou" de "linha
    // descartada", então o boot de diagnóstico lê o log completo e o boot normal fica com o throttle.
    let budget: u64 = if crate::marcador_do_processo!(".bwms-log-full") { 20_000 } else { 800 };
    if n >= budget && !crate::tempo::a_cada(n, 8) {
        return;
    }
    static LOG_FILE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
    if let Ok(mut guard) = LOG_FILE.try_lock() {
        if guard.is_none() {
            *guard = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open("/tmp/cp77-console.log")
                .ok();
        }
        if let Some(f) = guard.as_mut() {
            let _ = writeln!(f, "{msg}");
        }
    }
    // Em dev, ESPELHA no trace.log — sink confiável p/ diagnóstico de 1 ciclo: o console.log é
    // zerado ao abrir o console in-game (perde prints), o trace.log só zera no boot. Junta num
    // lugar só: loads de mod, prints de Lua (cheats/Cron), erros — pra ler tudo de uma vez.
    // Mesmo fix de handle cacheado (era `open+append+close` por chamada).
    if dev_mode() {
        static TRACE_FILE: std::sync::Mutex<Option<std::fs::File>> = std::sync::Mutex::new(None);
        if let Ok(mut guard) = TRACE_FILE.try_lock() {
            if guard.is_none() {
                *guard = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open("/tmp/cp77-trace.log")
                    .ok();
            }
            if let Some(f) = guard.as_mut() {
                let _ = writeln!(f, "{msg}");
            }
        }
    }
    } // fim do bloco #[cfg(feature = "devlog")]
}

/// Comandos do console in-game a caminho do tick. Em memória: o canal por arquivo em /tmp sobrescrevia o comando anterior e
/// aceitava comando de qualquer usuário da máquina (/tmp é compartilhado). O tick roda 1 por vez, só depois do 1º phase 5.
static FILA_DE_COMANDOS: std::sync::Mutex<std::collections::VecDeque<String>> = std::sync::Mutex::new(std::collections::VecDeque::new());
/// Teto da fila: o que se digita no menu espera o load, e uma fila sem teto juntaria tudo pro 1º tick de gameplay.
const TETO_DA_FILA_DE_COMANDOS: usize = 16;

/// Enfileira um comando do console; `false` com a fila cheia.
pub(crate) fn enfileirar_comando(c: &str) -> bool {
    let Ok(mut f) = FILA_DE_COMANDOS.lock() else { return false };
    if f.len() >= TETO_DA_FILA_DE_COMANDOS {
        return false;
    }
    f.push_back(c.to_string());
    true
}

/// Modo-dev: liga diagnósticos verbosos (trace.log volumoso, logs de registro de hook, o
/// export `cp77-watch.txt`, de quando a leitura vinha de fora). Default OFF → jogo LIMPO, registro "debaixo dos
/// panos". Liga com env `BWMS_DEV` ou tocando `/tmp/bwms-dev` (sem mexer no launch da Steam).
/// Checado 1x e cacheado. Só no build de dev: no público é sempre `false` e não lê `/tmp` nem o ambiente.
pub(crate) fn dev_mode() -> bool {
    if !cfg!(feature = "devtools") {
        return false;
    }
    use std::sync::OnceLock;
    static DEV: OnceLock<bool> = OnceLock::new();
    *DEV.get_or_init(|| {
        std::env::var_os("BWMS_DEV").is_some() || std::path::Path::new("/tmp/bwms-dev").exists()
    })
}

/// Breadcrumb p/ crash nativo, só com `devlog`: sobrescreve /tmp/cp77-lasthook.txt com o ÚLTIMO ponto
/// (1 linha — sobrevive a segfault, é o que diagnostica crash nativo). O trace.log VOLUMOSO (sequência
/// inteira, ~100KB/sessão) só é gravado em [`dev_mode`]. O build público não escreve em /tmp (TRACELESS).
#[cfg(not(feature = "devlog"))]
pub(crate) fn trace(_msg: &str) {}

#[cfg(feature = "devlog")]
pub(crate) fn trace(msg: &str) {
    use std::io::Write;
    let _ = std::fs::write("/tmp/cp77-lasthook.txt", msg);
    if !dev_mode() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/cp77-trace.log")
    {
        let _ = writeln!(f, "{msg}");
    }
}

/// Marcador de dev consultado de dentro do tick. **No build público é `false` constante**: o `if` do
/// chamador desaparece na compilação e com ele a consulta inteira.
#[cfg(feature = "devtools")]
pub(crate) fn marcador_de_dev(nome: &str) -> bool {
    com_a_varredura(|v| v.iter().any(|m| &**m == nome))
}

/// Marcador individual `~/.bwms-hook-<nome>` (sem o global `~/.bwms-hook`): gate de mudança de comportamento, não de
/// diagnóstico. O prefixo é comparado em cima do nome já indexado, pra não alocar 130 `String` por frame só pra jogar fora.
#[cfg(feature = "devtools")]
pub(crate) fn marcador_individual(nome: &str) -> bool {
    com_a_varredura(|v| {
        v.iter()
            .any(|m| m.strip_prefix(".bwms-hook-").is_some_and(|r| r == nome))
    })
}

/// Atraso fixo no topo do `on_load()` — fix real (não workaround de dev) do crash determinístico
/// `brk #1`/`baseEngineInit.cpp:1094` ("Failed to initialize scripts data!") que acontecia em TODO
/// boot com o dylib presente, independente de conteúdo/registro/comportamento (isolado por
/// eliminação em 2026-08-19: mesmo crash com `BWMS_INERT=1`, com `register_all()` desligado, com
/// dylib antigo pré-sessão — é corrida de timing no boot, não bug de conteúdo). Achado: rodar sob
/// `lldb -- <exe>` (stop-at-entry) evita o crash por completo — a pausa do lldb entre dyld carregar
/// o dylib (roda este `on_load` como parte da carga) e o processo seguir pra `main()` desloca o
/// timing o suficiente pra sair da corrida. Um atraso artificial aqui reproduz o MESMO efeito sem
/// precisar de debugger: confirmado em boot normal (sem lldb), GAMEPLAY real alcançada, zero crash,
/// input funcional. `BWMS_BOOT_DELAY_MS` sobrescreve o valor (ms) pra tuning/teste.
const BOOT_RACE_DELAY_MS: u64 = 800;

/// `red4ext/bwms-ultimo-boot.txt`: o que o `on_load` decidiu neste boot, no build público também (segundos desde 1970 + o passo). Sem arquivo
/// novo, a dylib não carregou; só com "carregado", parou antes do gate de versão; com a linha do gate, a build detectada e se o BWMS ativou.
/// O 1º boot da GOG 2.5 recém-instalada pelo Galaxy não ativou e o público não deixava saber qual dos três (2026-10-26).
fn marcar_ultimo_boot(passo: &str, acrescentar: bool) {
    use std::io::Write;
    let Some(dir) = dylib_dir() else { return };
    let caminho = format!("{dir}/bwms-ultimo-boot.txt");
    let segundos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let linha = format!("{segundos} {passo}\n");
    let _ = if acrescentar {
        std::fs::OpenOptions::new().append(true).create(true).open(&caminho).and_then(|mut f| f.write_all(linha.as_bytes()))
    } else {
        std::fs::write(&caminho, linha)
    };
}

/// A linha do gate no `bwms-ultimo-boot.txt`; na build não reconhecida, a base da imagem e os 8 bytes nos endereços do executor do Steam e do
/// GOG 2.5.0 (o prólogo esperado é `EXECUTOR_PROLOGO`), o que separa "outra versão do jogo" de "imagem lida no lugar errado".
fn linha_do_gate() -> String {
    let b = game_build();
    if build_supported() {
        return format!("gate: build {b:?}, BWMS ativo");
    }
    let base = game_base();
    let ler = |vm: u64| -> String {
        let p = (base + (vm - bwms_primitives::enderecos::LINK_BASE) as usize) as *const c_void;
        if base != 0 && unsafe { crate::memoria::is_readable(p, 8) } { format!("{:#018x}", unsafe { (p as *const u64).read_unaligned() }) } else { "ilegível".into() }
    };
    let gog = enderecos::traduzir_gog_25(crate::jogo::scripts::EXECUTOR_VM).map_or_else(|| "sem tabela".into(), ler);
    format!(
        "gate: build {b:?}, BWMS não ativou (base {base:#x}; executor Steam {}, GOG {gog}; esperado {:#018x})",
        ler(crate::jogo::scripts::EXECUTOR_VM),
        crate::jogo::scripts::EXECUTOR_PROLOGO
    )
}

extern "C" fn on_load() {
    marcar_ultimo_boot(&format!("BWMS {BWMS_VERSION} carregado"), false);
    let delay_ms = std::env::var("BWMS_BOOT_DELAY_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(BOOT_RACE_DELAY_MS);
    if delay_ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms));
    }
    // Dead-man's switch do lever BwmsFireStart — TEM que rodar antes de qualquer coisa que possa
    // levar o lever a disparar nesta sessão (ver inicio::boot::check_stale_boot_attempt).
    inicio::boot::check_stale_boot_attempt();
    // Registro de energia que o boot anterior deixou por não chegar ao fim do load nem à saída limpa: vai pro relatório de crash (com ou sem
    // `.ips`). Lido antes de o boot atual gravar o dele (`selfboot`).
    let energia_anterior = inicio::energia::do_boot_anterior();
    if let Some(r) = &energia_anterior {
        crate::veredito_dev!("[energia] boot anterior sem fim do load nem saída limpa: {r}");
    }
    // Relatório de crash (best-effort): se há um `.ips` do Cyberpunk de até 1 h ainda não reportado,
    // escreve `red4ext/bwms-crash-report.md` pro usuário colar numa issue — substitui o
    // `bwms-report.command` (executável assusta usuário de mod já marcado por AV). Sem crash novo, só
    // varre as pastas de DiagnosticReports; o dead-man's switch acima entra como contexto, não como gate. NÃO gateado por
    // build/devlog: o WRITE do .md tem que valer no build PÚBLICO (é o ponto). Roda ANTES do gate
    // de versão de propósito — o relatório é útil justamente quando algo deu errado, inclusive num
    // build não-reconhecido. Toda falha degrada em silêncio (panic=abort → código defensivo). Ver
    // relatorio_de_crash.rs.
    relatorio_de_crash::write_crash_report_if_crashed(energia_anterior.as_deref());
    // GATE DE VERSÃO: fora do Steam e do GOG 2.5.0 (Epic, o jogo atualizado além da versão dos offsets) NÃO instala NENHUM gancho. Os offsets e os
    // layouts de struct do código são da 2.5.0; num binário de outro layout leem/escrevem memória errada → crash antes do menu. O executor
    // aborta por prólogo, mas os outros ganchos sempre ligados (bind/getfn/register) não têm essa guarda. No público o resultado é só "BWMS não
    // ativou, o jogo bootou vanilla".
    marcar_ultimo_boot("pré-gate ok", true);
    marcar_ultimo_boot(&linha_do_gate(), true);
    if !build_supported() {
        BUILD_UNSUPPORTED.store(true, Ordering::Relaxed);
        crate::log_dev!("[build] RECUSADO: o binário não é o Steam nem o GOG 2.5.0 (detectado: {:?}) — BWMS não instala nenhum gancho, o jogo boota sem o BWMS.", game_build());
        return;
    }
    #[cfg(feature = "devlog")]
    if dev_mode() {
        let _ = std::fs::write("/tmp/cp77-trace.log", ""); // trace fresh por sessão (só dev)
    }
    crate::log_dev!(
        "[cp77-console] carregada (Rust); game_base = {:#x}. Subindo thread do console.",
        game_base()
    );
    // A pasta `Mods/` lida uma vez: a compilação abaixo e o `preparar_no_boot` (tabelas, relatório, `.archive`) usam a mesma leitura.
    mods_do_usuario::pasta_mods::ler_no_boot();
    // Compila os `.reds` se algo mudou, antes de qualquer gancho: o motor só abre o `final.redscripts` no `baseEngineInit`, depois do
    // `main()`, e este construtor roda antes dele.
    scripts::compilacao_no_lancamento::compilar_se_mudou();
    // CPVR (dev): limpa o marcador .cpvr-ingame STALE de uma sessão anterior. Sem isso, o cpvr.js
    // O leitor externo antigo lia o marcador preso no startup → gameplayActive=true → captura no menu/boot → o boot
    // TRAVA na tela preta antes do menu. O cp77_tick recria o marcador só quando há player (gameplay).
    #[cfg(feature = "cpvr")]
    cpvr_clear_stale_ingame();
    // F-B: instala a ponte do bind orchestrator JÁ AQUI (topo do on_load), o mais cedo possível —
    // o bind do script (RedScriptsHost::Load → hook em `BIND_ORCH_VM`) roda muito cedo, antes
    // do overlay/selfboot. É só patch de código (sem RTTI). Sem marcador: só não instala se o prólogo não casar.
    unsafe { inicio::boot::install_bind_bridge() };
    // F4 (Facade/CallbackSystem/Reflection): hook de produção no validador de classe nativa (RE 2026-07-12,
    // ver inicio/boot.rs): forja a classe do BWMS que o validador visita, antes da original. Sem marcador.
    unsafe { inicio::boot::install_class_validate_probe() };
    // Espelho do `ScriptBundle` — lê o bundle no `ScriptValidator::Validate` e guarda contagens e arquivo/linha das funções.
    unsafe { inicio::boot::install_script_validate_hook() };
    // F2d: a recarga dos scripts dos mods do mod.io dá endereço novo às funções de script; o BWMS esquece as que guardou.
    unsafe { inicio::boot::install_recarga_de_scripts_dos_mods() };
    // Aparência do record no spawn por template (patch sempre ligado).
    unsafe { aparencia::do_record::instalar() };
    // Stats novos de mod (enum, `statRecords` e os getters de faixa e flags sem o teto do jogo).
    unsafe { tweakdb::stats::instalar() };
    // F2d: a recarga do TweakDB que o motor faz ao instalar mod do mod.io com o jogo aberto reaplica os tweaks da sessão.
    unsafe { tweakdb::instalar_recarga() };
    // Sem o teto de 10 world widgets atualizando.
    unsafe { interface::ink_mundo::instalar() };
    // Overlay (janela in-game) — swizzle do present do Metal numa thread própria.
    terminal::overlay::start();
    // Self-boot do runtime (hook do executor). É a única porta, e fica depois do gate de versão acima: com
    // build não suportado o `on_load` já saiu e nenhum hook de endereço entra. Idempotente (guard ACTIVE).
    unsafe { inicio::boot::selfboot_if_needed() };
    // Pasta `Mods/`: varre, regrava as tabelas de `red4ext/` com os `.xl` dela e grava o relatório. Antes do gancho do `InitializeArchives`
    // (que carrega os `.archive` desta varredura) e de quem lê as tabelas (o `PostLoad` de setor abaixo e a carga do 1º tick).
    mods_do_usuario::pasta_mods::preparar_no_boot();
    // Injeção de archive arbitrário (rota B) + `RegisterArchive`/`RegisterDir` da fachada de recursos: instala o hook no
    // InitializeArchives AQUI, no `on_load` SÍNCRONO (thread principal, na carga pelo dyld, mais cedo possível — ANTES do
    // InitializeArchives/LoadGlobs do boot). Instalar hook de CÓDIGO da nossa thread de heartbeat NÃO
    // efetivou a escrita (v2 não capturou nada); o copy-test que funciona instala do cp77_tick, dentro do
    // executor. **Sempre instalado** (antes só sob `~/.bwms-pathbtest`) — base da API Facade shipada,
    // não mais só probe de dev. Ver `install_pathb_capture`.
    unsafe { install_pathb_capture() };
    // `CallbackSystem.RegisterCallback(n"Pipeline/FrameBegin",...)` (✅ FECHADO
    // 2026-08-11): **Sempre instalado** (antes só sob `dev_mode()`+`~/.bwms-updateregistrar-probe`
    // dentro de `install_postload_hooks()`) — mesma promoção de "probe de dev" pra "base shipada"
    // já dada ao `install_pathb_capture()` acima. Passthrough observe-only puro, zero mudança de
    // comportamento do jogo. Ver `install_pipeline_framebegin_probe` (tipos_do_motor/mod.rs).
    unsafe { crate::tipos_do_motor::install_pipeline_framebegin_probe() };
    // `OpenWorldTracker`: hook no `PointOfInterestMappin::SetPhase`, com `openworldtracker = true` no `bwms.toml` (no dev, também
    // com `~/.bwms-hook-openworldtracker`).
    unsafe { crate::missoes_e_mapa::install_openworldtracker() };
    // `inkSystem::ProcessCharacterEvent` só observando, com `~/.bwms-hook-c176a-teste`.
    unsafe { crate::interface::tecla_no_widget::install_c176_pce_probe() };
    // Hooks do `Reparent`, do `AddChildWidget` e do `OnInitialize` com `bwms-widget-building.reds` no bundle ou `~/.bwms-hook-c175-teste`.
    unsafe { crate::interface::install_c175_widget_building() };
    // `inkWidget.AttachController` do jogo registra o controller na camada viva e chama o `OnInitialize` (sempre).
    unsafe { crate::interface::install_anexar_controller() };
    // `Item:Controller` no spawn de widget de biblioteca e o evento `InkWidget/Spawn`, hooks do
    // `SpawnFromLocal`/`SpawnFromExternal`, do `AsyncSpawnFromLocal`/`AsyncSpawnFromExternal` e da entrega do spawn assíncrono (`0x104368f34`)
    // sempre no ar.
    unsafe { crate::interface::spawn_de_widget::instalar() };
    // Sonda do `ReadSoundBanksJson` (quando roda e com quantos bancos), com `~/.bwms-hook-c122som-teste`.
    unsafe { crate::som::install_read_soundbanks_json_hook() };
    // Hook do job `CGameFramework/GameInstance`, que grava o framework de onde sai a `RuntimeScene`; o do job
    // `GameSession/Initialize_GameInstance` só no dev.
    unsafe { crate::inicio::boot::install_jobs_de_estado() };
    // Os `Session/*` que o `game::CallbackSystem` do jogo dispara, repassados aos ouvintes do BWMS (`inicio::sessao_do_jogo`).
    unsafe { crate::inicio::sessao_do_jogo::instalar() };
    // `ChangeAppearance`/`ChangeAppearances` com o `ApplyAppOverride` (sempre; o teste com `~/.bwms-hook-a11-teste`).
    unsafe { crate::aparencia::install_app_overrides() };
    // `{nails_color}`: hook do `CollectStateData` da aparência dinâmica nativa da 2.5 (sempre).
    unsafe { crate::aparencia::install_cor_da_unha() };
    // Nome composto de material (`{material.N}`): hook do construtor dos locais do `ExpandResourcePath` nativo da 2.5 (sempre).
    unsafe { crate::aparencia::install_material_composto() };
    // `ApplyAppOverride` nos 6 `Get*Appearances` do estado de customização (sempre); `Initialize*Option` observados com
    // `~/.bwms-hook-a47a-teste`.
    unsafe { crate::corpo_e_rosto::install_a47a() };
    // Hook do `OnPlayerMainObjectSpawned` (produto) e, com o marcador, o callback estático do teste.
    unsafe { crate::inicio::boot::install_player_spawned_hook() };
    if marcador_individual("a34-teste") {
        unsafe { crate::missoes_e_mapa::install_a34() };
    }
    if marcador_individual("a55-teste") || marcador_individual("a35-teste") {
        unsafe { crate::missoes_e_mapa::install_questsys_ongamerestored() };
    }
    // `worldStreamingSector::PostLoad` só observando, com `~/.bwms-hook-wssector-postload-c2`.
    unsafe { crate::mundo::install_wssector_postload_probe() };
    // Rota de carga de fase `0x1029e8c48` só observando, com `~/.bwms-hook-questphase-carga`.
    unsafe { crate::missoes_e_mapa::install_questphase_carga_probe(false) };
    // Hooks de teste e sondas do dev, cada um atrás do próprio marcador.
    #[cfg(feature = "devtools")] unsafe { crate::selftest::instalar() };
    // Offsets de roupa vazios com `DisableGarmentOffsets`, o padrão.
    unsafe { crate::roupas_e_armas::install_offsets_de_roupa() };
    // HEARTBEAT de boot (diagnóstico do early-stick intermitente): thread que loga a cada 2s ONDE o boot
    // está, até chegar na gameplay. Se travar, a ÚLTIMA linha [hb] crava o ponto exato (t + fase). Custo ~0,
    // só loga no build dev (devlog). Para no player (gameplay) ou no cap ~30min.
    std::thread::spawn(|| {
        let t0 = std::time::Instant::now();
        let mut seen_eng = false;
        let mut last = (false, false, -2i32);
        // (hook do append pra pathb agora é instalado no on_load síncrono, thread do jogo.)
        for _ in 0..900u32 {
            // (pathb: a injeção agora é automática no replace do InitializeArchives — thread do jogo,
            // depot real, com lock. Ver `init_archives_replacement`. Nada a fazer aqui.)
            // Registro de energia vivo até o fim do load: se o boot morrer, o próximo relatório diz em que condição ele estava por último.
            unsafe { inicio::energia::acompanhar() };
            let eng = terminal::overlay::engagement_active();
            if eng {
                seen_eng = true;
            }
            let player = !current_player().is_null();
            // GAMEPLAY REAL = o getter da phase-byte viu phase=5 (o load do save foi DISPARADO) há
            // `GAMEPLAY_REAL_APOS_PHASE5_S`, com player vivo. phase=5 sozinho não basta: sai quando o
            // load é só enfileirado (mesmo motivo de a splash esperar a 3ª leitura de phase 5, inicio/boot.rs). Em 2 provas
            // completas o load dispara em t=92s e o getter vê os objetos NOVOS da sessão em phase=5
            // em t=105s; 20s cobre isso com folga e ainda cabe no teto de 180s do harness.
            //
            // O critério anterior, `seen_eng && player`, disparava NA TELA DE ENGAGEMENT (medido em
            // 2026-09-11): o player-espúrio do menu existe desde t≈5s e a engagement vem ANTES do menu
            // e do load. Em 4 provas antigas e nos 2 processos daquela data a linha saía 1 linha depois
            // de `engagement ativa = true`, ~10s antes de `[autocontinue] disparou`. E o "fallback"
            // que setava `PHASE_REACHED_5` aqui "quando o getter não viu phase=5" rodava SEMPRE,
            // porque o load nem existia ainda: em 19 logs com esse set, nenhum tinha phase=5 antes, e
            // os 11 completos mostram o getter vendo phase=5 depois. Três efeitos do set precoce:
            // abria no menu os gates "só gameplay real" (CreateRecord, tweakdb, canal do hb); a
            // splash dizia "carregando o mundo" na engagement; e, como o `swap` do getter já achava
            // `true`, o `clear_boot_attempt_marker()` da gameplay NUNCA rodava — um crash depois de
            // chegar no jogo suprimia o auto-load do boot seguinte (esta thread roda também no build
            // público). Sem getter (modo 0, sem skip-intro) não há como afirmar gameplay daqui: o hb
            // só segue logando o estado.
            // Espera também o fim do load (`inicio::boot::gameplay_real`): "phase 5 há 20 s" sozinho caiu 14 s antes dele no R28.
            const GAMEPLAY_REAL_APOS_PHASE5_S: u64 = 20;
            if player && inicio::boot::gameplay_real() {
                // Canal de veredito: é a linha que o `bwms-fastboot.sh gameplay` espera, e `log()` amostra.
                crate::veredito_dev!(
                    "[hb] t={}s GAMEPLAY REAL (phase=5 +{GAMEPLAY_REAL_APOS_PHASE5_S}s, player) — boot ok",
                    t0.elapsed().as_secs()
                );
                // Boots 260, 261 e 265: o tick parou e a amostra não tinha o `exec_replacement` em nenhuma pilha. Separa "o executor
                // parou de passar pelo hook" (chamadas paradas) de "um tick não voltou" (chamadas andando com o tick ocupado).
                // Só com `devlog`: o corpo é um `log_dev!`, e no público a thread só dormiria 180 s.
                #[cfg(feature = "devlog")]
                std::thread::spawn(|| {
                    for _ in 0..18 {
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        crate::log_dev!(
                            "[hb-pos] chamadas do executor {} tick ocupado {}",
                            inicio::boot::exec_calls(),
                            TICK_BUSY.load(Ordering::Relaxed)
                        );
                    }
                });
                break;
            }
            // fase da sessão (GAME_SESSION_DESC+0x84, capturado pelo getter): 3=asset-load do boot, 1=engagement,
            // 2=initUser, 5=gameplay. Mostra ONDE travou — preso em phase=3 = thrash do asset-load (ambiente).
            let phase = unsafe {
                let sm = inicio::boot::GAME_SESSION_DESC.load(Ordering::Relaxed);
                if !sm.is_null() && crate::memoria::is_readable(sm as *const c_void, 0x85) {
                    (sm.add(0x84) as *const i8).read() as i32
                } else {
                    -1
                }
            };
            let t = t0.elapsed().as_secs();
            if t < 40 || (eng, seen_eng, phase) != last {
                let spur = if player { " (player do menu/load — ainda não é gameplay)" } else { "" };
                crate::log_dev!("[hb] t={t}s eng={eng} seen_eng={seen_eng} phase={phase}{spur} (esperando gameplay)");
            }
            last = (eng, seen_eng, phase);
            // Canal pré-gameplay (só `ping`, do ferramental de dev): prefixo `[hb-early]` pra nunca confundir com o canal pós-gameplay.
            #[cfg(feature = "devtools")]
            if let Ok(cmd) = std::fs::read_to_string("/tmp/cp77-cmd.txt") {
                let cmd = cmd.trim().to_string();
                let consumed = match cmd.as_str() {
                    "ping" => { crate::log_dev!("[hb-early] pong (t={t}s, pré-gameplay)"); true }
                    _ => false,
                };
                if consumed {
                    let _ = std::fs::write("/tmp/cp77-cmd.txt", "");
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        // Pós-boot: canal drain do ferramental de dev, independente do executor (que para quando o jogo fica idle após save-load).
        // No público a thread acaba aqui: o laço só existe pra esse canal.
        #[cfg(feature = "devtools")]
        loop {
            // Canal para comandos thread-safe (sem callf/nativo que exige a game thread).
            if let Ok(cmd) = std::fs::read_to_string("/tmp/cp77-cmd.txt") {
                let cmd = cmd.trim().to_string();
                if !cmd.is_empty() {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    let consumed = match parts.as_slice() {
                        ["ping"] => { crate::log_dev!("[hb-canal] pong"); true }
                        // 2026-08-16/17 (rodada 41, infra pendente desde a rodada 39/40):
                        // `archivegroupdump` é 100% read-only (zero mutação, só lê `PATHB_DEPOT`,
                        // já capturado pelo hook `InitializeArchives` bem mais cedo no boot,
                        // ~t=15s — MUITO antes de `PHASE_REACHED_5`/GAMEPLAY). Migrado pro canal
                        // do heartbeat (sempre vivo, independente do executor) pra que diagnósticos
                        // futuros do depot não precisem competir com a janela de risco do lock
                        // nativo do motor (`SharedSpinLock::Lock()`, confirmado nas rodadas
                        // 26/32/35/36, perto/depois de gameplay real). `archivegroupcreate` (muta
                        // memória real do engine) fica DE FORA de propósito — mesma cautela que já
                        // manteve `checkreshook`/`presskey` fora desta migração (categoria "escreve
                        // em memória/código vivo" precisa do executor, nunca da thread do
                        // heartbeat). Núcleo em `run_archivegroupdump()` (diagnostico/mod.rs, junto de
                        // `archive_scope_name`), reusado pelo fallback do canal do executor acima.
                        ["archivegroupdump"] => {
                            unsafe { run_archivegroupdump() };
                            crate::log_dev!("[hb-canal] archivegroupdump (via heartbeat, sem depender do executor)");
                            true
                        }
                        // A lista do selo e da aba Mods (os 2 caminhos, a luz de cada mod, os conflitos): só lê a lista pronta e contadores.
                        ["modslista"] => {
                            crate::veredito_dev!("{}", crate::mods_do_usuario::lista_dos_mods::veredito());
                            true
                        }
                        _ => false // callf/spawnsub/outros precisam da game thread — NÃO apaga
                    };
                    if consumed {
                        let _ = std::fs::write("/tmp/cp77-cmd.txt", "");
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
    // Nova tentativa automática do `InkSystem::Get` (2026-08-16, engenharia de robustez): 9
    // tentativas MANUAIS seguidas (ver HISTORICO.md) confirmaram que a janela em que o canal do
    // EXECUTOR fica vivo pós-gameplay é curta e IMPREVISÍVEL (às vezes fecha em 1-2s, às vezes
    // nem isso, dependendo de quando a 2ª rajada de `OnGameAttached` do autocontinue domina o
    // executor) — mandar `inkgethook` "na hora certa" por fora (agente/humano cronometrando)
    // nunca convergiu de forma confiável. Em vez de continuar apostando no timing manual, o
    // PRÓPRIO dylib agora tenta instalar o probe sozinho, repetidamente, numa THREAD DEDICADA
    // (sempre viva desde o `on_load`, independente do executor/foco — mesma categoria da thread
    // de heartbeat acima, só que com cadência mais apertada, 750ms, só pra esta tarefa).
    //
    // Por que é seguro chamar `install_inkget_probe()` em loop, de uma thread que não é a do
    // executor: (1) `install_inkget_probe()` JÁ é idempotente por design — o primeiro `swap`
    // em `INKGET_INSTALLED` funciona como uma trava: só 1 chamada por vez chega a de fato tentar
    // o patch; se falhar (alvo ilegível/prólogo mudou), a flag volta pra `false` e a PRÓXIMA
    // tentativa (deste mesmo loop) tenta de novo do zero — nunca instala 2x, nunca deixa o hook
    // pela metade. (2) O mecanismo de escrita (`ganchos::instalar_com_ponte`, ver `ganchos`)
    // suspende as outras threads da TASK inteira (`thread_suspend`), não só a que chama, e escreve
    // o patch com `mach_vm_protect`(COW)+`memcpy`+`sys_icache_invalidate` sem nenhum PC no trecho; e
    // `pthread_jit_write_protect_np` (usado só pro trampolim JIT, não pro patch em si) já é
    // corretamente per-thread e setado/resetado dentro da própria chamada, então funciona igual
    // não importa qual thread chama. (3) Já existe precedente direto no próprio projeto: o
    // `checkreshook` (mesmíssima categoria — instala hook de código) foi testado ao vivo
    // rodando desta MESMA thread de heartbeat e o hook instalou e
    // capturou dado real (`HISTORICO.md`, 2026-08-14). O único precedente NEGATIVO conhecido
    // (`install_pathb_capture`, comentário da chamada no `on_load`) foi pra um alvo que dispara 1 VEZ SÓ, bem
    // cedo no boot (`InitializeArchives`) — plausivelmente um problema de JANELA (o heartbeat só
    // chegou a instalar DEPOIS que a única chamada já tinha acontecido), não de escrita
    // cross-thread não-visível; `Red::InkSystem::Get()` é chamado continuamente (562+ sites, o
    // boot inteiro) — não tem essa janela de "1 chance só", então esse risco específico não se
    // aplica aqui.
    //
    // Gate `~/.bwms-hook-inkget-lr` (o mesmo marcador manual de sempre) checado 1x aqui, na
    // decisão de nascer a thread — ausente = zero overhead (thread nem chega a existir) pra
    // qualquer boot/usuário sem o marcador. `install_inkget_probe()` internamente RE-checa o
    // mesmo gate a cada chamada (barato, mesmo padrão de outros probes do projeto) — redundante
    // de propósito, não uma otimização perdida.
    #[cfg(feature = "devtools")]
    if let Some(hh) = std::env::var("HOME").ok() {
        if std::path::Path::new(&hh).join(".bwms-hook-inkget-lr").exists() {
            std::thread::spawn(|| {
                // ~7,5min a 750ms — generoso (feature dev-only, gate fechado por padrão pra
                // qualquer usuário final; não precisa ser econômico), mas não infinito: se o
                // alvo nunca ficar legível (módulo nunca mapeado, boot travado antes disso),
                // a thread desiste e para de gastar ciclos sozinha.
                const MAX_ATTEMPTS: u32 = 600;
                let mut n = 0u32;
                loop {
                    n += 1;
                    crate::log_dev!("[inkget-retry] tentativa {n}");
                    unsafe { crate::selftest::install_inkget_probe() };
                    if crate::selftest::inkget_is_installed() {
                        crate::log_dev!(
                            "[inkget-retry] SUCESSO na tentativa {n} — hook instalado, parando de tentar"
                        );
                        // Captura de baseline AUTOMÁTICA (sem precisar do comando manual
                        // `inkgetbaseline`): espera a instalação assentar (~10s de idle real)
                        // e drena o ring sozinho, sempre pela mesma thread — `inkget_baseline()`
                        // é 100% thread-safe (só drena `INKGET_RING`/`AtomicU64` + grava
                        // `INKGET_BASELINE`/`Mutex<BTreeSet>`, zero VM/RTTI/callf).
                        std::thread::sleep(std::time::Duration::from_secs(10));
                        crate::selftest::inkget_baseline();
                        crate::log_dev!("[inkget-retry] baseline automático capturado (10s pós-instalação) — presskey/inkgetpost seguem manuais via canal (precisam de ação externa)");
                        break;
                    }
                    if n >= MAX_ATTEMPTS {
                        crate::log_dev!(
                            "[inkget-retry] desistindo após {n} tentativas (~{}min) — alvo nunca ficou instalável nesta sessão",
                            (n as u64 * 750) / 60_000
                        );
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(750));
                }
            });
        }
    }
    // Execução de comandos: NÃO numa thread nossa (instanciar item da thread
    // errada crasha) — a sonda chama `cp77_tick` de dentro do hook do executor, num worker
    // `redDispatcherN` do motor, não na GameThread. (Esse mesmo mecanismo serve pro Observe/Override.)
}

/// Registry global, obtida 1x pelo `cp77_tick`, dentro do executor. Só-leitura após init → OnceLock
/// (sem Mutex, p/ o Lua poder ler sem deadlock dentro do cp77_tick).
struct SendReg(rtti::Registry);
unsafe impl Send for SendReg {}
unsafe impl Sync for SendReg {}
static REG: std::sync::OnceLock<SendReg> = std::sync::OnceLock::new();
/// player/tx atuais (grava `inicio::boot::capture`, a sonda do executor; o cp77_tick só lê, o Lua lê via Game.*). Voltam a nulo na troca de
/// mundo (`inicio::boot::trocar_de_mundo`) até o `capture` pegar os do mundo novo.
static CURRENT_PLAYER: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static CURRENT_TX: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// Algum player já foi capturado no processo. Para quem separa "boot" de "depois do 1º player" (o Bink): o `CURRENT_PLAYER` nulo da troca de
/// mundo não é boot.
static PLAYER_JA_VISTO: AtomicBool = AtomicBool::new(false);

pub(crate) fn registry() -> Option<&'static rtti::Registry> {
    REG.get().map(|s| &s.0)
}
pub(crate) fn current_player() -> *mut c_void {
    CURRENT_PLAYER.load(Ordering::Relaxed)
}
pub(crate) fn set_current_player(p: *mut c_void) {
    if !p.is_null() {
        PLAYER_JA_VISTO.store(true, Ordering::Relaxed);
    }
    CURRENT_PLAYER.store(p, Ordering::Relaxed);
}
pub(crate) fn player_ja_visto() -> bool {
    PLAYER_JA_VISTO.load(Ordering::Relaxed)
}
pub(crate) fn set_current_tx(t: *mut c_void) {
    CURRENT_TX.store(t, Ordering::Relaxed);
}
pub(crate) fn current_tx() -> *mut c_void {
    CURRENT_TX.load(Ordering::Relaxed)
}

/// Heartbeat do runtime: o cp77_tick incrementa todo tick (dentro do executor) só
/// quando há player/tx vivos. O badge do overlay lê isso → mostra "ativo" sem spam.
static TICKS: AtomicU64 = AtomicU64::new(0);

// ---- registry de status por mod (badge colorido) --------------------------------

/// Pasta onde a NOSSA dylib está carregada (via dladdr no próprio código). Base
/// pra resolver caminhos relativos (mods) de forma portável.
pub(crate) fn dylib_dir() -> Option<String> {
    use crate::macos::{dladdr, DlInfo};
    unsafe {
        let mut info: DlInfo = std::mem::zeroed();
        if dladdr(mods_dir as *const c_void, &mut info) == 0 || info.dli_fname.is_null() {
            return None;
        }
        let path = std::ffi::CStr::from_ptr(info.dli_fname).to_string_lossy().into_owned();
        path.rfind('/').map(|i| path[..i].to_string())
    }
}
pub(crate) fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Guard global do tick (ver cp77_tick): pula a reentrada e o tick simultâneo de outra thread. RAII reseta no fim de qualquer caminho.
static TICK_BUSY: AtomicBool = AtomicBool::new(false);
struct TickGuard;
impl Drop for TickGuard {
    fn drop(&mut self) {
        TICK_BUSY.store(false, Ordering::Relaxed);
    }
}

/// Versão do BWMS — escrita no splash de boot. Bumpar a cada release pro Nexus.
pub const BWMS_VERSION: &str = "0.1.5b";

/// Chamado de dentro do hook do executor (`inicio::boot::exec_replacement`) a cada `TICK_EVERY` chamadas, na thread que fez a chamada: na
/// prática um worker `redDispatcherN`, não a GameThread (`proofs/20260912-boot108-crash-a51f-tick-class-of.ips`). Chamar a VM vale por estar
/// dentro do executor (`inicio::boot::exec_nested`), não por uma thread fixa. Lê o player/tx da sonda, roda o comando pendente e o onUpdate Lua.
#[no_mangle]
pub extern "C" fn cp77_tick() {
    // RE-ENTRÂNCIA: o tick roda DENTRO do hook do executor. Se um callback (Override/Cron/
    // console via call_func) re-entrar o executor e o módulo de tick bater de novo, NÃO
    // re-executa — senão a recursão exec→tick→…→exec→tick estoura a pilha e crasha (foi o
    // crash do `ovcall`). RAII (`_tg`) reseta o flag ao sair, em qualquer return.
    if TICK_BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    let _tg = TickGuard;
    // As filas de tecla do AppKit esvaziam aqui, uma vez por tick, acima das saídas sem registry e sem
    // player. O overlay enfileira em menu, load e gameplay, com ou sem ouvinte; sem este ponto a fila retém as 64 teclas
    // mais velhas e um ouvinte de `Input/Key` registrado depois recebe todas num tick só. Toda saída antecipada descarta
    // o lote no drop; o despacho fica no fim do tick (`TeclasDoTick::despachar`).
    let teclas = TeclasDoTick::esvaziar_filas();
    // [execpace] rodada 31 (2026-08-17): mede o ritmo de exec_replacement, throttled a 1
    // log/segundo, investigando o achado da rodada 30 (cadeia de ~108 `@wrapMethod(PlayerPuppet)
    // OnGameAttached` empilhados pode nunca devolver controle dentro da janela testada). Roda
    // ANTES de qualquer gate (registry/RTTI) de propósito — queremos ver o ritmo mesmo se o
    // resto do tick ainda não puder agir. Puramente observacional. Ver inicio::boot::log_exec_pace.
    inicio::boot::log_exec_pace();
    // WATCHDOG anti-hang de boot: roda ANTES do gate de registry, todo tick, pra o caso do getter
    // parar de ser chamado mas o tick seguir vivo. Depois da 5ª leitura de phase 5 o getter não é mais
    // chamado, e a saída do splash no fim do load vem daqui. Idempotente/barato (ver
    // inicio::boot::boot_hang_watchdog — só age com skip ligado).
    inicio::boot::boot_hang_watchdog();
    // Proteção contra o watchdog PRÓPRIO do motor (CDPR) matando o processo sob macOS Low Power
    // Mode (CPU/GPU throttled -> acumulado do watchdog estoura o budget default de 120s). Roda
    // SEMPRE, todo tick, incondicional (não é diagnóstico) — ver inicio::boot::neutralize_engine_watchdog.
    inicio::boot::neutralize_engine_watchdog();
    if REG.get().is_none() {
        if let Some(r) = unsafe { rtti::Registry::obtain() } {
            let _ = REG.set(SendReg(r));
        }
    }
    let reg = match registry() {
        Some(r) => r,
        None => return,
    };
    // F-B: re-tenta registrar as nativas do BWMS se o selfboot pegou o RTTI cedo demais
    // (idempotente via REGISTERED). Garante BlackwallPing no RTTI p/ o bind do redscript.
    unsafe { crate::register::register_all() };
    // TweakDB runtime: dump observe-only do singleton (gated ~/.bwms-tdbdump) p/ confirmar
    // o records-map in-vivo antes de registrar record novo (PASSO 0 do clone-runtime).
    unsafe { crate::tweakdb::dump_once_if_marked() };
    // Clone com registro em runtime (PASSO final): registra Items.BwmsCloneTest no TweakDB vivo
    // via a nativa CreateRecord (gated ~/.bwms-tdbcreate). RecordExists antes/depois = prova.
    unsafe { crate::tweakdb::create_once_if_marked() };
    // IA Fase 0: poll non-blocking da resposta do processo externo (throttled). Loga/exibe. Só no dev: no
    // público o `BwmsEmit` não é registrado (`registro_de_prova!`), nada é enfileirado e não existe resposta pra ler.
    #[cfg(feature = "devtools")]
    crate::diagnostico::ai::poll_response();
    // Resolve o FromTDBID UMA vez e publica o endereço-alvo: o hook do executor então
    // captura fn/ctx/ret nativamente quando o jogo o chama (no lugar da leitura externa).
    if inicio::boot::active() && inicio::boot::FROMTD_TGT.load(Ordering::Relaxed).is_null() {
        let rf = unsafe { rtti::resolve_func(reg, "gameItemID", "FromTDBID") }
            .or_else(|| unsafe { rtti::resolve_func(reg, "ItemID", "FromTDBID") });
        if let Some(rf) = rf {
            inicio::boot::FROMTD_TGT.store(rf.func, Ordering::Relaxed);
        }
    }
    let (player, tx) = (current_player(), current_tx());
    unsafe { diagnostico::criar_no_fantasma(reg) };
    // Mods Lua (`mods_lua`): carga, Observe pedidos e teclas em todo tick; `onUpdate` só com o V e o tx no mundo.
    #[cfg(feature = "lua")]
    unsafe {
        crate::mods_lua::tick(reg, !player.is_null() && !tx.is_null())
    };
    // Pipeline unificado de carga de mods (6 serem 1): resource.link de recursos,
    // plugins. Idempotente — roda só na 1ª iteração.
    mods_do_usuario::pipeline::boot_phase();
    // Carga antecipada dos journals da tabela e plano pro `Initialize` da raiz, com `~/.bwms-hook-journal-menu`. Roda em todo tick: o
    // menu já tem o player do menu, e no boot 87, com o gate de player nulo, a carga foi pedida e o plano nunca ficou pronto antes do `Initialize`.
    #[cfg(feature = "devtools")] unsafe { mundo::journal_mods_menu_tick() };
    // 2026-08-11: cobertura completa de `EGameStateType` —
    // `BaseInitialization`(0)/`Initialization`(1) NUNCA disparavam antes (só `Running`(2, via
    // heartbeat phase-tracking) e `Shutdown`(3, via exit hook)). `boot_phase()` (acima) é o
    // 1º ponto idempotente ONDE plugins já tiveram chance de `add_game_state` (carregados
    // no bind dos scripts ou, o que faltou, dentro dele, via `pipeline::carregar_plugins` → `plugins::load_plugins`) — dispara enter(0)→exit(0)→enter(1) aqui,
    // 1x. `exit(1)` dispara mais abaixo, neste tick, na 1ª transição real pra
    // Running (mesmo sinal já usado pro `enter(2)`). Mapeamento é uma decisão pragmática (não
    // verificada contra o binário real) — residual: callback pode disparar num momento
    // ligeiramente diferente do original Windows, não um risco de crash.
    if !GS01_FIRED.swap(true, Ordering::Relaxed) {
        crate::mods_do_usuario::api::call_game_state_enter(EstadoDoJogo::BaseInitialization);
        crate::mods_do_usuario::api::call_game_state_exit(EstadoDoJogo::BaseInitialization);
        crate::mods_do_usuario::api::call_game_state_enter(EstadoDoJogo::Initialization);
        crate::log_dev!("[gs] enter(BaseInitialization)+exit(BaseInitialization)+enter(Initialization) — cobertura BaseInitialization/Initialization");
    }
    // Initialization.OnUpdate (1) a cada 2 s, entre o enter(1) acima e o exit(1) da transição pra Running: na mesma thread do enter/exit e
    // do OnUpdate(2), nunca antes do Enter nem depois do Exit.
    if GS01_FIRED.load(Ordering::Relaxed) && !GS1_EXITED.load(Ordering::Relaxed) {
        static PROXIMO_UPDATE_1: AtomicU64 = AtomicU64::new(0);
        let agora = crate::inicio::boot::boot_elapsed_secs();
        if agora >= PROXIMO_UPDATE_1.load(Ordering::Relaxed) {
            PROXIMO_UPDATE_1.store(agora + 2, Ordering::Relaxed);
            crate::mods_do_usuario::api::call_game_state_update(EstadoDoJogo::Initialization);
        }
    }
    // Relocador universal one-shot NO MENU: gated por `~/.bwms-relocreal`. Roda AQUI (antes do
    // gate de player) porque o alvo é um dtor de IA de NPC — dormente no menu (mundo não carregado),
    // colisão ~zero; em gameplay ele dispararia. Consome o marcador e roda 1×. Ver `prove_relocreal`.
    // Nos 3 one-shots abaixo o flag vem antes do marcador: depois de rodar, o tick não paga mais `env::var` + `stat`.
    if !RELOCREAL_DONE.load(Ordering::Relaxed) {
        if let Some(m) = crate::caminho_de_marcador(".bwms-relocreal") {
            if m.exists() && !RELOCREAL_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe { prove_relocreal() };
            }
        }
    }
    // Contrato de attach/detach: idem, one-shot no menu (2 hooks empilhados no MESMO alvo,
    // LIFO, Detach único). Ver `prove_attach_detach`.
    if !ATTACHDETACH_DONE.load(Ordering::Relaxed) {
        if let Some(m) = crate::caminho_de_marcador(".bwms-attachdetach") {
            if m.exists() && !ATTACHDETACH_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe { prove_attach_detach() };
            }
        }
    }
    // Registro de nome de tweak: derive nativo PURO (roda no MENU, não precisa player). Ver `prove_derive`.
    if !DERIVETEST_DONE.load(Ordering::Relaxed) {
        if let Some(m) = crate::caminho_de_marcador(".bwms-derivetest") {
            if m.exists() && !DERIVETEST_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe { prove_derive() };
            }
        }
    }
    // Item 461: hookar o `invoke` REAL do callback já registrado pelo `AnimationSystem_FrameBeginReset` vanilla, capturado pelo probe
    // observe-only de `UpdateRegistrar::RegisterUpdate`. Só com ouvinte de `Pipeline/*` (`PIPELINE_ARMADOS`): os 3 ganchos rodam a cada quadro
    // desde o boot, e sem ouvinte não há o que despachar — o jogo de quem não usa esses eventos fica sem eles (boot de 2026-10-25 00:09 travou
    // antes da engagement com os 3 no ar, causa não fechada). Rechecado todo tick até instalar — idempotente, barato.
    if !ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK.load(Ordering::Relaxed)
        && crate::tempo::PIPELINE_ARMADOS.load(Ordering::Relaxed) != 0
        && unsafe { crate::tempo::install_anim_framebegin_hook_if_ready() }
    {
        ANIM_FRAMEBEGIN_HOOK_INSTALLED_TICK.store(true, Ordering::Relaxed);
    }
    // Redirect de path inexistente (roda no MENU). Máquina de estados dirigida
    // pelo tick (captura o depot real → golden) — ver `prove_copy_tick`. Arma no 1º achado do marcador.
    if !COPYTEST_DONE.load(Ordering::Relaxed) {
        if let Some(m) = crate::caminho_de_marcador(".bwms-copytest") {
            if m.exists() {
                if !COPYTEST_ARMED.swap(true, Ordering::Relaxed) {
                    let _ = std::fs::remove_file(&m); // consome 1x; a partir daí roda por fase
                }
            }
        }
        if COPYTEST_ARMED.load(Ordering::Relaxed) {
            unsafe { prove_copy_tick() };
            if COPY_PHASE.load(Ordering::Relaxed) >= 3 {
                COPYTEST_DONE.store(true, Ordering::Relaxed); // fase terminal (3=ok, 9=abortado)
            }
        }
    }
    // CPVR (dev): sinaliza gameplay pro cpvr.js gatear a captura VR (só em jogo, nunca menu/boot).
    #[cfg(feature = "cpvr")]
    cpvr_ingame_marker(!player.is_null() && !tx.is_null());
    // CallbackSystem: controller STATE-DRIVEN de presença do player. "Player/Spawned" na 1ª presença (player+tx) do processo, que é a do
    // puppet do menu; a presença latcha (abaixo), então o ramo do "Player/Despawned" não roda: o fim de sessão sai do `OnWorldDetached`
    // (`inicio::sessao_do_jogo`) e do `Session/End` do hook do spawn (`inicio::natives::session_ready_tick`). Fica ANTES do gate de player. Mecanismo = fire_event
    // (provado + GUARDADO: o dispatch valida rtti::sane+class_of → alvo liberado é PULADO, sem o
    // crash de [[cp77-crash-callbacksystem-devreds]]; sem listener = no-op). SEM offset novo.
    {
        static PLAYER_PRESENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        // `redDispatcher-crash-handle-ctor-delay` (2026-07-18, hipótese do coordenador, análise
        // estática dos 2 primeiros .ips + corroborada por leitura de código): no tick EXATO da
        // transição `present` (só acontece de verdade num save-load REAL — nunca exercitada antes
        // desta sessão), 2 `Handle_ctor` (pool alloc) disparavam de volta-a-volta (Session/Start +
        // Entity/Attach), bem no instante em que o motor inunda com realocação de array/world-data
        // do save real. 4/4 crashes desta sessão (`redDispatcher4/7/9`, mesma assinatura EXATA:
        // `cp77_tick→call_func→exec_replacement`, mesmos offsets nativos 35074284/35074676)
        // aconteceram perto dessa janela. FIX: atrasa só os 2 eventos com `make_handle` por N ticks
        // após a transição (mesmo padrão `m_stable>=40`/`TICKS>120` já usado no projeto pra "esperar
        // o mundo assentar"). `Player/Spawned` (sem
        // handle) continua disparando na hora — não usa `Handle_ctor`, não é suspeito.
        static PENDING_HANDLE_EVENTS_AT: AtomicU64 = AtomicU64::new(0); // tick-alvo; 0 = nada pendente
        // ptr do player salvo antes da transição null — Entity/Detach precisa dele quando player já é null
        static LAST_ENTITY_PTR: AtomicUsize = AtomicUsize::new(0);
        const HANDLE_EVENTS_DELAY_TICKS: u64 = 180; // ~3-6s no ritmo já usado por TICKS neste arquivo
        // 2026-08-20 (madrugada, sessão 2, retomada pós-compactação): 4/4 crashes SIGSEGV reproduzidos
        // nesta sessão (mesma assinatura de sempre, `EXC_BAD_ACCESS`/zero frame nosso na pilha) sempre
        // IMEDIATAMENTE APÓS `mods_do_usuario::pipeline::session_phase` (scan RTTI de ~27800 tipos procurando
        // `ScriptableTweak`, `GetAllTypes`) terminar de rodar no MESMO tick dos 4
        // `fire_event_args` acima — nunca durante os 4 eventos em si, sempre logo depois do scan+
        // dispatch dos tweaks completar. Candidato de fix já listado em sessões anteriores, testado
        // agora pela 1ª vez: dar ao scan de tweaks um tick-alvo PRÓPRIO, mais tarde que os 4 eventos
        // (não junto no mesmo tick) — reduz a chance de o scan pesado colidir com a mesma janela de
        // fila-de-eventos do motor que os 4 `fire_event_args` já ocupam. Ver memória
        // `bwms-t82s-crash-vs-887pct-hang` (Atualização 10).
        static PENDING_SESSION_PHASE_AT: AtomicU64 = AtomicU64::new(0); // tick-alvo p/ o scan de tweaks; 0 = nada pendente
        const SESSION_PHASE_EXTRA_DELAY_TICKS: u64 = 120; // ~2-4s DEPOIS dos 4 eventos (soma a HANDLE_EVENTS_DELAY_TICKS)
        // Presença latcha: o player e o tx nulos da troca de mundo (`inicio::boot::trocar_de_mundo`) não são despawn. Disparar o ramo do
        // despawn ali poria `call_func` de ouvinte e `Entity/Detach` com o puppet do mundo que sai (`LAST_ENTITY_PTR`) dentro da carga.
        let present = PLAYER_PRESENT.load(Ordering::Relaxed) || (!player.is_null() && !tx.is_null());
        if present != PLAYER_PRESENT.swap(present, Ordering::Relaxed) {
            let ev = if present { "Player/Spawned" } else { "Player/Despawned" };
            let n = unsafe { tipos_do_motor::fire_event(ev) };
            crate::log_dev!("[cbs] {ev} (transição de presença do player) → {n} callback(s)");
            if present {
                // `1.4` (2026-07-19): sinal MODO-INDEPENDENTE de gameplay alcançada → arma a rede
                // anti-crash do redDispatcher também no modo 0 (onde o getter de skip — a única fonte
                // de PHASE_REACHED_5 — não instala). Latcha.
                inicio::boot::POST_SAVELOAD.store(true, Ordering::Relaxed);
                LAST_ENTITY_PTR.store(player as usize, Ordering::Relaxed);
                // 2026-08-11, fix desta sessão: `Initialization.
                // OnExit` (type=1) TEM que disparar ANTES de `Running.OnEnter` — mesma transição, mesmo
                // instante, mesmo mecanismo (presença do player, já provado confiável pro Running) em
                // vez do byte de fase lido pela thread de heartbeat (flaky, ver `GS1_EXITED` acima).
                if !GS1_EXITED.swap(true, Ordering::Relaxed) {
                    crate::mods_do_usuario::api::call_game_state_exit(EstadoDoJogo::Initialization);
                    crate::log_dev!("[gs] exit(Initialization) — saiu de Initialization, entrando em Running (via presença do player)");
                }
                // Sessão já fechada por um `Session/End` abre aqui; a aberta pelo spawn ou pelo `Session/BeforeStart` do jogo fica.
                inicio::sessao_do_jogo::abrir_sessao_na_presenca();
                #[cfg(feature = "devtools")]
                unsafe { inicio::sessao_do_jogo::sonda::na_gameplay() };
                // Running.OnEnter (type=2) — player spawnando = estado Running
                crate::mods_do_usuario::api::call_game_state_enter(EstadoDoJogo::Running);
                // NÃO dispara Session/Start/Entity/Attach aqui — agenda pra N ticks depois (bloco
                // fora do `if`, abaixo), fora da janela de flood do save-load real.
                let target = TICKS.load(Ordering::Relaxed) + HANDLE_EVENTS_DELAY_TICKS;
                PENDING_HANDLE_EVENTS_AT.store(target, Ordering::Relaxed);
                crate::log_dev!(
                    "[cbs] Session/Start+Entity/Attach AGENDADOS pra tick>={target} (delay anti-crash, atual={})",
                    TICKS.load(Ordering::Relaxed)
                );
                // session_phase (scan de tweaks) agendado num tick DEPOIS dos 4 eventos acima —
                // ver comentário de `SESSION_PHASE_EXTRA_DELAY_TICKS` acima.
                let sp_target = target + SESSION_PHASE_EXTRA_DELAY_TICKS;
                PENDING_SESSION_PHASE_AT.store(sp_target, Ordering::Relaxed);
                crate::log_dev!(
                    "[cbs] session_phase (scan de tweaks) AGENDADO pra tick>={sp_target} (separado dos 4 eventos, +{SESSION_PHASE_EXTRA_DELAY_TICKS} ticks)"
                );
            } else {
                // Despawn/End: sem o padrão de flood conhecido (mundo saindo, não entrando) — mantém
                // imediato, e também cancela qualquer disparo pendente de uma sessão anterior.
                PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
                PENDING_SESSION_PHASE_AT.store(0, Ordering::Relaxed);
                // Running.OnExit (type=2) — player despawnando = saindo do Running
                crate::mods_do_usuario::api::call_game_state_exit(EstadoDoJogo::Running);
                // Reserva do `Session/End`: desliga na 1ª entrega do `OnWorldDetached` do jogo (`inicio::sessao_do_jogo`); sai só se nenhuma
                // fonte entregou o `End` desta sessão.
                if inicio::sessao_do_jogo::reserva_ligada(inicio::sessao_do_jogo::FIM_DA_SESSAO) && inicio::sessao_do_jogo::marcar_fim() {
                    let n2 = unsafe { inicio::natives::disparar_evento_de_sessao("Session/End", false, true) };
                    crate::veredito_dev!("[sessao-reserva] Session/End (despawn do jogador) → {n2} callback(s)");
                }
                // O mundo saiu, as entidades dinâmicas, as tags e os ouvintes vão junto.
                unsafe { npcs::dynent_fim_de_sessao() };
                unsafe { entidades::statent_fim_de_sessao() };
                // Controller de entidade sub-evento: "Entity/Detach" —
                // usa LAST_ENTITY_PTR pq player já é null aqui na transição de despawn.
                let last = LAST_ENTITY_PTR.load(Ordering::Relaxed) as *mut c_void;
                // Sem ouvinte, nem lê o ponteiro nem constrói o evento.
                // Com o hook do `Detach` no ar, o `Entity/Detach` do player já sai dele.
                if let Some(ev) = entidades::evento_do_player(present, last, entidades::HOOKS_DO_CICLO_NO_AR.load(Ordering::Relaxed), tipos_do_motor::tem_ouvinte) {
                    let nd = unsafe { entidades::disparar_evento_de_entidade(ev, last) };
                    crate::log_dev!("[cbs] {ev} (EntityLifecycleEvent real) → {nd} callback(s)");
                }
            }
        }
        // Dispara os 2 eventos com `make_handle` (Session/Start + Entity/Attach) quando o tick-alvo
        // agendado acima é atingido — fora do `if` de transição (roda em ticks POSTERIORES ao evento
        // que armou o alvo). Guarda `PLAYER_PRESENT` pra não disparar se o player já saiu de novo.
        let pending = PENDING_HANDLE_EVENTS_AT.load(Ordering::Relaxed);
        // Com o player nulo da troca de mundo o disparo espera o puppet novo: o `Entity/Assemble` levaria a entidade nula.
        if pending != 0 && TICKS.load(Ordering::Relaxed) >= pending && !player.is_null() {
            PENDING_HANDLE_EVENTS_AT.store(0, Ordering::Relaxed);
            if PLAYER_PRESENT.load(Ordering::Relaxed) && present {
                // Controller de sessão: "Session/Start" (nome REAL,
                // `CallbackSystem::SessionStartEventName`). `restored=true` (auto-continue SEMPRE
                // carrega save) / `pregame=false` (player só presente pós-char-creation).
                // Sem ouvinte, nem constrói o `GameSessionEvent`.
                // Reserva do `Session/Start`: desliga na 1ª entrega do `OnGamePrepared` do jogo (`inicio::sessao_do_jogo`).
                if inicio::sessao_do_jogo::pela_reserva(inicio::sessao_do_jogo::INICIO_DA_SESSAO, "Session/Start") {
                let n2 = unsafe { inicio::natives::disparar_evento_de_sessao("Session/Start", false, true) };
                crate::veredito_dev!("[sessao-reserva] Session/Start (atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n2} callback(s)");
                }
                // Construtor de entidade (2026-07-24): "Entity/Assemble" (nome REAL)
                // — achado da investigação paralela: carrega o MESMO
                // `EntityLifecycleEvent` que "Entity/Attach" (não `EntityBuilderEvent`, esse é
                // específico de "Entity/Extract"), então dispara pelo MESMO mecanismo já provado,
                // zero RE nova. Ordem real do motor: Assemble roda ANTES de Attach — disparado
                // primeiro aqui pelo mesmo motivo (um `Entity.AddComponent` feito no handler de
                // Assemble deveria já estar no array quando o Attach nativo roda de verdade).
                // Ativação preguiçosa (2026-09-05): a construção do
                // `EntityLifecycleEvent` é um objeto RTTI de verdade e acontecia SEMPRE — mesmo
                // sem ninguém escutando, quando `fire_event_args` descobre isso lá dentro e volta.
                // A pergunta barata vem antes do trabalho caro.
                if tipos_do_motor::tem_ouvinte("Entity/Assemble") {
                    let n0 = unsafe { entidades::disparar_evento_de_entidade("Entity/Assemble", player) };
                    crate::log_dev!("[cbs] Entity/Assemble (EntityLifecycleEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n0} callback(s)");
                }
                // Controller de entidade: "Entity/Attach" (nome REAL) com
                // `EntityLifecycleEvent` real (`GetEntity()->ref<Entity>`, "raw"/sem dono).
                // Com o hook do `Attach` no ar, o `Entity/Attach` do player já sai dele; aqui repetiria.
                if let Some(ev) = entidades::evento_do_player(present, player, entidades::HOOKS_DO_CICLO_NO_AR.load(Ordering::Relaxed), tipos_do_motor::tem_ouvinte) {
                    let n3 = unsafe { entidades::disparar_evento_de_entidade(ev, player) };
                    crate::log_dev!("[cbs] {ev} (EntityLifecycleEvent real, atrasado {HANDLE_EVENTS_DELAY_TICKS} ticks) → {n3} callback(s)");
                }
                // `Component/Toggle` sai só do `IComponent::Toggle` de verdade (hooks instalados pelo `RegisterCallback`);
                // o disparo com o 1º componente do player que vivia aqui entregava o evento sem toggle nenhum.
            }
        }
        // session_phase (scan de tweaks/GetAllTypes) — tick-alvo PRÓPRIO, separado dos 4 eventos
        // acima (ver `SESSION_PHASE_EXTRA_DELAY_TICKS`).
        let sp_pending = PENDING_SESSION_PHASE_AT.load(Ordering::Relaxed);
        if sp_pending != 0 && TICKS.load(Ordering::Relaxed) >= sp_pending {
            PENDING_SESSION_PHASE_AT.store(0, Ordering::Relaxed);
            if PLAYER_PRESENT.load(Ordering::Relaxed) && present {
                // Pipeline de tweaks no Session/Start: ScriptableTweak dispatch + YAML auto-apply dos mods.
                if let Some(reg) = unsafe { rtti::Registry::obtain() } {
                    unsafe { mods_do_usuario::pipeline::session_phase(&reg) };
                }
            }
        }
    }
    // Eventos de ciclo de vida: onOverlayOpen/onOverlayClose — a borda é sinalizada na thread do render
    // (overlay.rs::desenhar_terminal, via presentDrawable) e consumida AQUI (cp77_tick, dentro do executor) pra
    // disparar o fire_event com segurança (chamar a VM fora do executor é arriscado). Roda ANTES
    // do gate de player (o overlay abre/fecha tanto no menu quanto em gameplay).
    if terminal::overlay::OVERLAY_OPEN_EDGE.swap(false, Ordering::Relaxed) {
        let n = unsafe { tipos_do_motor::fire_event("Overlay/Open") };
        crate::log_dev!("[cbs] Overlay/Open → {n} callback(s)");
    }
    if terminal::overlay::OVERLAY_CLOSE_EDGE.swap(false, Ordering::Relaxed) {
        let n = unsafe { tipos_do_motor::fire_event("Overlay/Close") };
        crate::log_dev!("[cbs] Overlay/Close → {n} callback(s)");
    }
    // Controller de recurso: "Resource/Load" — edge-triggered (MESMO padrão seguro de Player/
    // Spawned/Overlay acima, roda ANTES do gate de player pq streaming de recurso acontece desde
    // o boot). O hook `resource.link` (já instalado, zero-crash provado) marca `WATCH_RES_SEEN`
    // quando o path armado por `watchres <path>` é de fato construído pelo jogo.
    if crate::arquivos_do_jogo::WATCH_RES_SEEN.swap(false, Ordering::Relaxed) {
        let h = crate::arquivos_do_jogo::WATCH_RES_HASH.load(Ordering::Relaxed);
        // Sem ouvinte, nem constrói o `ResourceEvent`.
        if tipos_do_motor::tem_ouvinte("Resource/Load") {
        let n = unsafe { arquivos_do_jogo::disparar_resource_load_do_caminho(h) };
        crate::log_dev!("[cbs] Resource/Load (ResourceEvent real, path_hash={h:#018x}) → {n} callback(s)");
        }
    }
    // Customização (2026-08-14): drena os 3 edge flags marcados por
    // `BwmsCustomizationEdge` (chamado sincronamente pelos wraps em `characterCreationBodyMorphMenu`)
    // — MESMO padrão seguro de "Resource/Load" acima (edge setado por native chamado por bytecode
    // → dispatch de verdade só aqui, fora de qualquer nesting de call_func).
    unsafe { corpo_e_rosto::drain_customization_edges() };
    // `Session/Ready` dos spawns do jogador contados pelo hook, no mundo do menu também (antes do gate de player).
    unsafe { inicio::natives::session_ready_tick() };
    // Estado do VR pro painel: o transporte carimba um arquivo quando um headset está em sessão.
    // A cada ~32 ticks (~1 s) — é o que a aba precisa pra acender e apagar sozinha.
    {
        static VOLTAS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 32) {
            crate::vr::painel::atualizar_do_transporte();
        }
        // Mãos e cabeça só são lidas com headset em sessão: sem VR, isto é zero I/O por tick.
        #[cfg(feature = "cpvr")]
        if crate::vr::painel::conectado() {
            crate::vr::maos::atualizar();
            crate::vr::cabeca::atualizar();
        }
    }
    // Save novo na pasta → grava os specs das entidades persistentes (fora do save).
    crate::salvamento::entidades_persistentes::vigia_de_saves_tick();
    crate::salvamento::entidades_persistentes::fila_tick();
    crate::salvamento::entidades_persistentes::sessao_tick();
    #[cfg(feature = "devtools")] crate::salvamento::entidades_persistentes::teste_tick();
    // O vigia recompila os `.reds` do modder (ligado por red4ext/bwms-recarga-de-scripts) e a troca de corpo aplica a quente —
    // antes do gate de player, porque nenhum dos dois lê o jogador.
    crate::scripts::carregador::vigia_tick();
    crate::scripts::carregador::aplicar_tick();
    #[cfg(feature = "devtools")] crate::scripts::tradutor_de_bytecode::sonda_tick();
    #[cfg(feature = "devtools")] crate::scripts::tradutor_de_bytecode::teste_tick();
    #[cfg(feature = "devtools")] crate::scripts::tradutor_de_bytecode::teste_do_vigia_tick();
    // Bytes dos pedaços nativos do 2.5 (`Item:Controller`, anexar árvore, `OnInputKey`): veredito no menu e na gameplay.
    #[cfg(feature = "devtools")] crate::jogo::interface::bytes_nativos_tick(!player.is_null() && !tx.is_null());
    // A alavanca d4=2 adiada pelo `OnApply` do menu sai aqui (ele termina no bloco do `OnApply`, mais abaixo; ela sai no tick seguinte).
    // Antes do gate de player: o teto da espera não pode depender de haver jogador do menu.
    unsafe { inicio::natives::ordem_tick() };
    if player.is_null() || tx.is_null() {
        return;
    }
    // runtime vivo (player/tx ok): pulsa o heartbeat que o badge do overlay lê.
    TICKS.fetch_add(1, Ordering::Relaxed);
    // `ResourceToken.RegisterCallback` (2026-09-11): avisa os ouvintes dos tokens que terminaram. Sem vigia registrado
    // (registro só vem de script em gameplay) não chama nada — longe da janela do load.
    unsafe { crate::arquivos_do_jogo::vigiar_tokens() };
    // API para plugin de terceiro: chama a native registrada por um PLUGIN de 3o ("BwmsPluginOnUpdate",
    // se existir) A CADA TICK — prova lifecycle onUpdate contínuo pós-boot via BwmsApi (não só a
    // chamada 1x de bwms_plugin_main). MESMA via de resolução/chamada do comando `callg`
    // (register::get_function + rtti::call_func) — nenhum offset/ABI novo.
    // GATE (2026-07-16, 3 achados AO VIVO nesta sessão, nessa ordem):
    // 1) sem gate nenhum: um `rtti::call_func` disparado bem na janela da transição de
    //    LoadModdedSave crasha (mesmo padrão já corrigido no full-body).
    // 2) gate `GAME_SESSION_DESC+0x84==5` ANTES do gate de player: lê um objeto que ainda não
    //    estabilizou (phase oscilando 3→0→...) — nunca vira true.
    // 3) o MESMO gate, movido pra DEPOIS do gate de player (mesma posição do full-body): AINDA
    //    não disparou em 2 boots com gameplay REAL confirmada (phasedbg mostrava phase=5 em
    //    OUTROS objetos, mas GAME_SESSION_DESC especificamente não). Conclusão: esse sinal
    //    específico não é confiável fora do caso estreito do full-body (que só o usa como
    //    reforço, gateado TAMBÉM por `TICKS>1800`). Substituído pelo gate coarse já usado (e
    //    provado reliable) pelos comandos de canal (`hasgod`/`callg`) neste mesmo ponto: apenas
    //    "player/tx confirmados não-nulos NESTE tick" (o próprio fato de termos chegado até aqui,
    //    passando o gate acima) + um piso pequeno de ticks (evita a primeira rajada bem na
    //    transição, sem exigir os milhares de ticks que a via antiga precisava).
    // `redDispatcher-crash-any-callfunc-near-phase5` (2026-07-18, v3 — v2 usava
    // `inicio::boot::GAME_SESSION_DESC` (a MESMA leitura do bloco full-body) e NUNCA armou num boot que
    // crashou de novo (6ª ocorrência, mesma assinatura EXATA): achado real por que — o getter que
    // captura esse ponteiro TRAVA (`ENGAGEMENT_SM_LOCKED`) no objeto TRANSIENTE da tela de
    // engagement e NUNCA re-captura o objeto REAL pós-save-load (confirmado: `[phasedbg] getter#9`
    // e `getter#10`, ambos phase=5, têm `this=` DIFERENTE — um novo objeto substitui o antigo, mas
    // `GAME_SESSION_DESC` fica preso no velho). Ou seja: o próprio sinal que o bloco full-body usa
    // pode estar batendo em phase5 do objeto ERRADO nalguns casos — não invalida o full-body (que
    // parece ter funcionado antes por outra via), mas invalida meu uso aqui. FIX v3: usar
    // `inicio::boot::PHASE_REACHED_5`/`PHASE5_AT`, setados DIRETO dentro do PRÓPRIO hook do getter
    // (o mesmo que produz os logs `[phasedbg] phase=5` confiáveis, `this` qualquer que seja) —
    // sinal por EVENTO, não por re-derivação de ponteiro cacheado. `PHASE5_AT` é um `Instant` real,
    // então o cooldown é por TEMPO DE PAREDE (imune a incerteza de taxa de tick).
    //
    // Contexto (hipótese ampla, substituindo a estreita do coordenador que a v1 já refutou):
    // `exec_replacement` (frame do meio em TODOS os 6 crashes) tem assinatura de DISPATCHER
    // GENÉRICO de chamada nativa (func/ctx/frame/aOut/a4), não um hook de UMA função — plausível
    // que QUALQUER `rtti::call_func` disparado bem no instante do save-load real completar colida
    // com o motor. Gate: suprime as invocações call_func PERIÓDICAS (este bloco + Session/Ready/
    // Update/Tick abaixo) por uma janela de TEMPO após a 1ª vez que phase5 é observado — sem
    // impedir o uso ANTES (menu/player-espúrio, seguro há centenas de chamadas hoje) nem MUITO
    // DEPOIS (só a janela de risco real).
    // v3 (12s): crashou (7ª ocorrência) ~15-18s pós-`[autocontinue] disparou`, bem quando o
    // cooldown ACABAVA de expirar — o contador do plugin ficou PARADO até ali (gate ENGATOU de
    // verdade, sobreviveu TODA a janela suprimida). v4 (45s): MESMO padrão — sobreviveu os 45s
    // inteiros suprimido (1ª vez que qualquer boot passa da janela clássica de crash), MAS crashou
    // (8ª ocorrência) poucos segundos depois de RETOMAR as chamadas periódicas. **Achado que muda o
    // modelo de novo:** não parece ser "tempo insuficiente desde phase5" — em AMBOS os testes o
    // crash bateu logo após RETOMAR, não importa se retomou aos 12s ou aos 45s. Ou seja, o perigo
    // pode estar ligado ao ATO de retomar/1ª chamada pós-gap, não a uma janela de tempo fixa. v5:
    // suprime PERMANENTEMENTE (nunca retoma nesta sessão) uma vez que phase5 é visto — testa se
    // ficar OFF pra sempre pós-gameplay-real evita o crash de vez (aceitando a perda de feature:
    // plugin onUpdate + Session/Ready/Update/Tick não rodam mais DEPOIS do save-load real, só antes).
    // `1.4`: OR com POST_SAVELOAD pra a supressão valer no modo 0 (getter de skip não instala lá →
    // PHASE_REACHED_5 nunca setava → rede anti-crash inerte no modo 0). POST_SAVELOAD é modo-independente.
    let in_crash_window = inicio::boot::PHASE_REACHED_5.load(Ordering::Relaxed)
        || inicio::boot::POST_SAVELOAD.load(Ordering::Relaxed);
    {
        static WAS_IN_WINDOW: AtomicBool = AtomicBool::new(false);
        if in_crash_window != WAS_IN_WINDOW.swap(in_crash_window, Ordering::Relaxed) {
            crate::log_dev!(
                "[cbs] crash-window (call_func periódico suprimido) -> {in_crash_window} (phase5_at.elapsed={:?})",
                inicio::boot::PHASE5_AT.get().map(|t| t.elapsed())
            );
        }
    }
    {
        static PLUGIN_ONUPDATE_FN: AtomicU64 = AtomicU64::new(0);
        // Geração dos scripts em que a função foi achada: a recarga dos scripts dos mods do mod.io dá endereço novo a ela.
        static PLUGIN_ONUPDATE_GERACAO: AtomicU64 = AtomicU64::new(0);
        let past_transition_window = TICKS.load(Ordering::Relaxed) > 60 && !in_crash_window;
        // A busca só roda quando a chamada vai acontecer: sem plugin (pacote público) o `get_function` erra, varre as ~4.435
        // globais e o motor não solta o array delas — por tick.
        if past_transition_window {
            let geracao = rtti::geracao_de_scripts();
            if PLUGIN_ONUPDATE_GERACAO.load(Ordering::Relaxed) != geracao {
                PLUGIN_ONUPDATE_FN.store(0, Ordering::Relaxed);
                PLUGIN_ONUPDATE_GERACAO.store(geracao, Ordering::Relaxed);
            }
            if PLUGIN_ONUPDATE_FN.load(Ordering::Relaxed) == 0 {
                let f = unsafe { crate::register::get_function(reg, "BwmsPluginOnUpdate") };
                if rtti::sane(f) {
                    PLUGIN_ONUPDATE_FN.store(f as u64, Ordering::Relaxed);
                }
            }
            let f = PLUGIN_ONUPDATE_FN.load(Ordering::Relaxed);
            if f != 0 {
                let rf = rtti::ResolvedFn { func: f as *mut c_void, ret_type: std::ptr::null_mut(), is_static: true };
                unsafe { rtti::call_func(&rf, std::ptr::null_mut(), &[]) };
            }
        }
    }
    // Estado do puppet (2026-08-02, iteração 2): dispara a recuperação ATRASADA do
    // soft-lock do espelho, se `BwmsPuppetStateArmRecovery` armou uma (ver register.rs). Roda
    // todo tick, é um no-op de load quando nada está pendente (checa 1 AtomicU64).
    #[cfg(feature = "devtools")]
    unsafe { diagnostico::puppetstate_recovery_tick(reg) };
    // Garment (2026-08-02): teste da hipótese do agente de RE do crash de `equiponce`
    // (`EquipmentSystem::QueueRequest` crasha porque `CClassFunction::GetInvokable()` retorna null —
    // possível dependência de contexto/thread, não de cache-warmup). Em vez de disparar na hora que o
    // comando chega (thread arbitrária, pode ser reentrante numa chamada nativa qualquer do motor),
    // dispara UMA VEZ desta MESMA posição periódica onde `BwmsPluginOnUpdate` já fez centenas de
    // chamadas sem incidente — mesmo TICKS>N de "deixar a sessão assentar", SEM o gate de
    // `in_crash_window` (esse gate é sobre proximidade de save-load, não é a preocupação aqui).
    // Gated marcador ~/.bwms-equipdeferred + ~/.bwms-equip=1/2/3 (mesmo seletor de item de sempre).
    if !EQUIP_DEFERRED_DONE.load(Ordering::Relaxed) && TICKS.load(Ordering::Relaxed) > 120 {
        if let Some(m) = crate::caminho_de_marcador(".bwms-equipdeferred") {
            if m.exists() && !EQUIP_DEFERRED_DONE.swap(true, Ordering::Relaxed) {
                let _ = std::fs::remove_file(&m);
                unsafe {
                    if !player.is_null() {
                        let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
                        match gi {
                            Some(gi) => {
                                let f = crate::register::get_function(reg, "BwmsForceEquipOnce");
                                if rtti::sane(f) {
                                    let rf = rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                                    if rtti::call_func(&rf, std::ptr::null_mut(), &[rtti::Arg::Raw(gi)]).is_some() {
                                        crate::log_dev!("[equipdeferred] BwmsForceEquipOnce(game) chamado do tick periódico (rota alternativa, teste de contexto/thread)");
                                    } else {
                                        crate::log_dev!("[equipdeferred] call_func não completou");
                                    }
                                } else {
                                    crate::log_dev!("[equipdeferred] BwmsForceEquipOnce não resolveu");
                                }
                            }
                            None => crate::log_dev!("[equipdeferred] PlayerPuppet.GetGame falhou"),
                        }
                    } else {
                        crate::log_dev!("[equipdeferred] player null");
                    }
                }
            }
        }
    }
    // Recarga a quente dos tweaks: poll de mtime a cada ~10s; re-aplica yamls se algum mudou.
    if !in_crash_window {
        unsafe { mods_do_usuario::pipeline::hot_reload_tick() };
    }
    // `OpenWorldTracker`: aplica no journal os PoIs que chegaram na fase 0. Fica fora do `in_crash_window`, que
    // depois do 1º player fica ligado pra sempre; a função só age em gameplay real e com fila cheia (por evento, não periódica).
    unsafe { crate::missoes_e_mapa::aplicar_openworld_concluidos(reg, player) };
    // `ScriptableTweak.OnApply`: uma vez por processo, no 1º tick em que dá.
    // - No menu (o caminho normal): o load do save ainda não começou (nenhum phase 5), o TweakDB está carregado (o mesmo critério dos
    //   registros de sufixo, que gravam no menu e valem no gameplay) e os arquivos de tweak da sessão do menu já foram aplicados.
    //   Um guarda-roupa de mod aberto logo depois do load lê o `placementSlots` que o `OnApply` grava.
    // - Em gameplay (reserva, o load começou antes disso): 30 s depois da última leitura de phase 5, fora da janela do load.
    // A queda de 2026-09-11 (`+0x1afd4`, 5/11 com o `OnApply` no menu) seguiu com ele em gameplay (boot 42, 2/4 até o boot 51) e acabou com a
    // AL-2: era a subclasse reds estourando a forja de 0x40 B (HISTORICO 2026-09-12 e 2026-09-25); o despacho confere a AL-2 antes de rodar.
    // Depois dele, o `OnApply` que a recarga dos tweaks pede sai daqui, com as threads de arquivo terminadas.
    {
        static SCRIPTABLE_TWEAKS_DISPARADOS: AtomicBool = AtomicBool::new(false);
        // No dev, um boot de diagnóstico pode atrasar o dispatch pela chave `onapply_atraso_s` do `~/.bwms-modconfig.txt` (lida uma vez), pra
        // os smokes do boot terminarem antes dele. No público e sem a chave, 30 s.
        #[cfg(feature = "devtools")]
        let apos_ultimo_phase5_s = {
            static ONAPPLY_ATRASO_S: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
            *ONAPPLY_ATRASO_S.get_or_init(|| configuracao::modconfig_get("onapply_atraso_s").trim().parse().unwrap_or(30))
        };
        #[cfg(not(feature = "devtools"))]
        let apos_ultimo_phase5_s: u64 = 30;
        // v4 (2026-09-11, hipótese em teste): nunca com o executor aninhado. `call_func` síncrono no meio de uma execução em voo é o
        // mecanismo já provado do crash do `equiponce`, e o canal do tick usa o mesmo guard; o `OnApply` rodava sem ele (boot 43b caiu
        // 30–45 s depois). Aninhado, tenta de novo no próximo tick.
        let livre = !SCRIPTABLE_TWEAKS_DISPARADOS.load(Ordering::Relaxed)
            && !inicio::boot::exec_nested()
            && crate::tweakdb::arquivos_em_aplicacao() == 0;
        let no_menu = livre
            && inicio::boot::PHASE5_AT.get().is_none()
            && mods_do_usuario::pipeline::arquivos_da_sessao_aplicados()
            && unsafe { crate::tweakdb::carregado() }
            // Por último: só começa com a engagement em repouso e sem alavanca, e a alavanca espera ele (`natives::OrdemNoMenu`).
            && unsafe { inicio::natives::onapply_no_menu_comecar() };
        let em_gameplay = livre
            && inicio::boot::carga_do_save_terminou()
            && inicio::boot::PHASE5_ULTIMO_AT
                .lock()
                .ok()
                .and_then(|ultimo| *ultimo)
                .map_or(false, |t| t.elapsed().as_secs() >= apos_ultimo_phase5_s);
        if no_menu || em_gameplay {
            SCRIPTABLE_TWEAKS_DISPARADOS.store(true, Ordering::Relaxed);
            crate::veredito_dev!(
                "[tweaks] VEREDITO-ONAPPLY-MOMENTO {}: phase 5 visto {}, gameplay real {}, t={} s do boot, TweakDB {:?}; antes-do-load={} = {}",
                if no_menu { "no menu, antes do load do save" } else { "em gameplay (reserva: o load começou antes)" },
                inicio::boot::PHASE5_AT.get().is_some(),
                inicio::boot::gameplay_real(),
                inicio::boot::boot_elapsed_secs(),
                unsafe { crate::tweakdb::singleton() },
                no_menu,
                no_menu as u8
            );
            unsafe { mods_do_usuario::pipeline::dispatch_scriptable_tweaks(reg) };
            if no_menu {
                inicio::natives::onapply_no_menu_terminou();
            }
        } else if SCRIPTABLE_TWEAKS_DISPARADOS.load(Ordering::Relaxed) {
            unsafe { mods_do_usuario::pipeline::despachar_onapply_da_recarga(reg) };
        }
        // O que o `OnApply` do menu gravou só vale se o load do save não trocou o TweakDB: confere o objeto uma vez, já em gameplay.
        #[cfg(feature = "devlog")]
        {
            static TWEAKDB_CONFERIDO: AtomicBool = AtomicBool::new(false);
            if SCRIPTABLE_TWEAKS_DISPARADOS.load(Ordering::Relaxed) && inicio::boot::gameplay_real() && !TWEAKDB_CONFERIDO.swap(true, Ordering::Relaxed) {
                let do_onapply = mods_do_usuario::pipeline::TWEAKDB_DO_ONAPPLY.load(Ordering::Acquire);
                let agora = unsafe { crate::tweakdb::singleton() }.map_or(0, |t| t as usize);
                let mesmo = do_onapply != 0 && do_onapply == agora;
                crate::veredito_dev!(
                    "[tweaks] VEREDITO-ONAPPLY-TWEAKDB TweakDB do OnApply {do_onapply:#x}, do gameplay {agora:#x}; mesmo-tweakdb={mesmo} = {}",
                    mesmo as u8
                );
            }
        }
    }
    // Journals de mod da tabela `bwms-journal.txt` — carga e mescla na árvore viva com o load assentado (o mesmo teto do
    // `OnApply`) e nunca com o executor aninhado; sem tabela, a função sai na 1ª checagem.
    let journal_assentado = !inicio::boot::exec_nested()
        && inicio::boot::carga_do_save_terminou()
        && inicio::boot::PHASE5_ULTIMO_AT.lock().ok().and_then(|ultimo| *ultimo).map_or(false, |t| t.elapsed().as_secs() >= 30);
    if journal_assentado {
        unsafe { mundo::journal_mods_tick(reg, player) };
    }
    // Registros de sufixo do `PuppetStateSystem` (sempre, 1º tick com TweakDB carregado).
    crate::aparencia::registros_de_sufixo_tick();
    #[cfg(feature = "devtools")] crate::aparencia::do_record::teste_tick();
    #[cfg(feature = "devtools")] crate::jogo::motor::teste_tick();
    #[cfg(feature = "devtools")] crate::tweakdb::stats::teste_tick();
    #[cfg(feature = "devtools")] crate::interface::ink_mundo::teste_tick();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_arquivos();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_recarga();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_buffer();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_padroes();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_remover();
    #[cfg(feature = "devtools")] crate::tweakdb::teste::tick_updaterecord();
    #[cfg(feature = "devtools")] crate::mods_do_usuario::log_de_mod::tick_teste();
    crate::localizacao::tick();
    #[cfg(feature = "devtools")] crate::localizacao::teste::tick();
    #[cfg(feature = "devtools")] crate::localizacao::teste::tick_mod();
    #[cfg(feature = "devtools")] crate::localizacao::teste::tick_xl_nativo();
    // Veredito da sonda do `ReadSoundBanksJson` (`~/.bwms-hook-c122som-teste`).
    #[cfg(feature = "devtools")] crate::som::c122som_teste_tick();
    // Áudio: reenvio dos soundbanks pedido por um `RegisterCallback` de `Resource/Load` (produção).
    crate::som::bancos_de_som_tick();
    // Diagnóstico da entrega ligada no boot (`~/.bwms-hook-c176diag-teste`).
    #[cfg(feature = "devtools")] crate::interface::tecla_no_widget::c176diag_teste_tick();
    // Eventos do `DynamicEntitySystem` enfileirados (criação, remoção e os do broadcaster do motor) pros ouvintes de script.
    unsafe { crate::npcs::dynent_vigiar_mortes() };
    unsafe { crate::npcs::dynent_despachar() };
    // Entidades estáticas descartadas que chegaram ao status 7 saem do registry e têm o handle solto.
    unsafe { crate::entidades::statent_tick() };
    // Carga do `compiledData` das definições dos `.app` de patch.
    crate::aparencia::cargas_de_definicao_tick();
    // Telas de mods de terceiro (comando `telasmod`): nada sem pedido; daqui sai o agendamento de cada passo no DelaySystem
    // e a leitura do retorno, e o passo roda no `Call`, chamado pelo motor (a thread do `Call` é [UNCONFIRMED]).
    #[cfg(feature = "devtools")] crate::interface::teste_telas_de_mod::tick();
    // Testes in-game do dev, depois dos ticks de produto do frame; o 17b recebe a condição dos journals acima.
    #[cfg(feature = "devtools")] unsafe { crate::selftest::tick(journal_assentado) };
    // breadth Reflection: probe de CProperty num objeto VIVO (gated ~/.bwms-reflection-test), 1x.
    #[cfg(feature = "devtools")] unsafe { diagnostico::reflection_live_once(player) };
    // breadth de ganchos: valida trocar_slot_da_vtable/unhook na vtable real do player (gated ~/.bwms-vtable-test), 1x.
    #[cfg(feature = "devtools")] unsafe { ganchos::testar_troca_de_slot_uma_vez(player) };
    // Entrada no jogo (produto): uma vez por player de gameplay real, o disparador `BwmsAoEntrarNoJogo(game)` dos scripts, que cria o
    // `PuppetStateHandler` e devolve o God Mode da config externa. A GameInstance vai por valor, pela receita do
    // `PlayerPuppet.GetGame` (a do `GetGameInstance()` de dentro de um `call_func` cru volta morta). Sem a função no bundle, marca e não
    // procura de novo a cada tick.
    {
        static ENTROU_COM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        if inicio::boot::gameplay_real() && ENTROU_COM.load(Ordering::Relaxed) != player as usize {
            if let Some(gi) = unsafe { crate::jogo::sessao::game_instance(reg, player) } {
                ENTROU_COM.store(player as usize, Ordering::Relaxed);
                // Prova (dev): God Mode antes e depois e o handler do #29 no V, contra o `godmode` da config.
                let god_antes = if cfg!(feature = "devtools") { unsafe { status_do_personagem::hasgod(reg, player) } } else { None };
                let f = unsafe { crate::register::get_function(reg, "BwmsAoEntrarNoJogo") };
                let chamou = crate::rtti::sane(f) && {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                    unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) }.is_some()
                };
                if cfg!(feature = "devtools") {
                    let god_depois = unsafe { status_do_personagem::hasgod(reg, player) };
                    // `@addField` de script numa classe nativa mora no `valueHolder` (`inValueHolder`): ler `player + valueOffset` direto
                    // lê outro campo do V, e o resultado muda com o layout de cada build.
                    let (handler, no_holder) = unsafe {
                        let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), "m_puppetStateHandler");
                        if p.is_null() {
                            (false, false)
                        } else {
                            let no_holder = crate::rtti::prop_in_holder(p);
                            let campo = crate::rtti::field_ptr(player, crate::rtti::prop_value_offset(p), no_holder);
                            (!campo.is_null() && (campo as *const usize).read_unaligned() != 0, no_holder)
                        }
                    };
                    crate::veredito_dev!(
                        "[entrada] BwmsAoEntrarNoJogo(game) player={player:p} chamou={chamou} | cfg godmode='{}' god antes={god_antes:?} depois={god_depois:?} | handler #29={handler} (no valueHolder={no_holder})",
                        crate::configuracao::modconfig_get("godmode")
                    );
                }
            }
        }
    }
    // FULL-BODY auto-start (2026-07-15): chama o global redscript `BwmsBootFullbody` UMA vez, ~alguns
    // segundos após entrar em gameplay (t>120 ticks). Roda AQUI = game-thread (seguro): cria+agenda os
    // pollers do full-body SEM wrapar nenhuma classe de world-load (o @wrapMethod em PlayerPuppet/HUD
    // corrompia o SystemsUpdater no world-load — provado). Substitui o `callg BwmsBootFullbody` manual.
    //
    // ACHADO 2026-07-15 (mesmo dia, mais tarde): chamar com ctx=null e zero args fazia o
    // `GetGameInstance()` DENTRO do redscript devolver uma instância MORTA (GetDelaySystem/
    // GetPlayerSystem retornavam undefined mesmo em gameplay real confirmada por screenshot —
    // ver proofs/2026-07-15-callg-global-getgameinstance-dead-ACHADO.log). Fix: nunca deixar o
    // redscript conjurar a GameInstance sozinho a partir de uma chamada `call_func` crua — em vez
    // disso, o Rust obtém uma GameInstance de verdade AQUI (mesma receita AUTORITATIVA de
    // jogo::sessao::auth_player: `PlayerPuppet.GetGame` chamado com ctx=player, o ponteiro que
    // JÁ temos vivo neste ponto) e passa por VALOR (Arg::Raw, 16B) pro redscript receber como
    // parâmetro — BwmsBootFullbody(game: GameInstance) em vez de BwmsBootFullbody() + GetGameInstance().
    {
        // TESTE 2026-07-15 (mesmo dia, ainda mais tarde): os pollers agendados logo em TICKS>120
        // (~poucos segundos pós-player-não-nulo, ainda DENTRO da janela do autocontinue/
        // LoadModdedSave) param de vez depois de ~36-37 execuções (~11s), mesmo depois do fix de
        // reagendar `this`. Hipótese: o GameInstance/DelaySystem capturado tão cedo fica órfão
        // quando o mundo termina de carregar de verdade e o motor troca pra uma sessão final.
        // Teste: disparar bem mais tarde (TICKS>1800, ~30-60s pós-player-não-nulo, bem depois da
        // janela onde o poller morria) pra ver se um poller iniciado DEPOIS sobrevive indefinidamente.
        // GATE DE SEGURANÇA (2026-07-15, isolando um crash `SystemsUpdater::Node::LinkJob_NoFence`
        // que reproduziu num boot 100% passivo, SEM tppcam/forcelook ativos — só o auto-trigger +
        // pollers rodando). Até isolar a causa raiz, o auto-start fica OPT-IN (marcador ausente =
        // pollers NUNCA são criados = comportamento idêntico ao usuário que não mexe no full-body).
        // Pela varredura de marcadores (ver `marcador_de_dev`): eram 3 `stat` + 3 `format!` por frame
        // em gameplay, e no público o marcador nunca existe — lá as três leituras são `false` const.
        let fb_enabled = marcador_de_dev(".bwms-fullbody-enable");
        // OVERRIDE de trigger imediato (2026-07-16): com o jogo OCIOSO (sem input humano p/ andar),
        // TICKS acumula devagar demais (~1 por 2048 chamadas do executor) e o gate TICKS>1800 nunca
        // é atingido em tempo hábil pra testar. Com `~/.bwms-fullbody-now`, dispara assim que
        // phase5 vira true (o phase5 já garante gameplay real = seguro; o TICKS>1800 era só p/ evitar
        // a janela de world-load, que o phase5 também cobre). Lido a cada tick (barato) só quando fb ligado.
        let fb_now = fb_enabled && marcador_de_dev(".bwms-fullbody-now");
        // GATE DE GAMEPLAY REAL (2026-07-16): as funções full-body SÓ podem rodar em phase==5 (gameplay).
        // O menu/preview tem um "player-espúrio" — `player != null` NÃO basta (o preview satisfaz).
        // Rodar o full-body no preview deref um objeto STALE quando ele muda → crash na thread
        // redDispatcher (`cp77_tick → call_func → deref de ponteiro-lixo`, provado 2x nesta sessão,
        // inclusive um crash no MENU aos ~309s). phase (GAME_SESSION_DESC+0x84): 1/2/3=menu/boot, 5=gameplay.
        let phase5 = unsafe {
            let sm = inicio::boot::GAME_SESSION_DESC.load(Ordering::Relaxed);
            !sm.is_null()
                && crate::memoria::is_readable(sm as *const c_void, 0x85)
                && inicio::boot::FaseDaSessao::do_byte((sm.add(0x84) as *const i8).read() as i64) == Some(inicio::boot::FaseDaSessao::Jogo)
        };
        static FB_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if phase5 && fb_enabled && !FB_STARTED.load(Ordering::Relaxed) && (fb_now || TICKS.load(Ordering::Relaxed) > 1800) {
            let gi: Option<[u8; 16]> = unsafe {
                crate::jogo::sessao::game_instance(reg, player)
            };
            if let Some(gi) = gi {
                let f = unsafe { crate::register::get_function(reg, "BwmsBootFullbody") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn {
                        func: f,
                        ret_type: std::ptr::null_mut(),
                        is_static: true,
                    };
                    if unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) }.is_some() {
                        FB_STARTED.store(true, Ordering::Relaxed);
                        crate::log_dev!("[fullbody] BwmsBootFullbody(game) chamado com GameInstance real (PlayerPuppet.GetGame, não GetGameInstance())");
                    }
                }
            } else {
                crate::log_dev!("[fullbody] PlayerPuppet.GetGame falhou — adiando auto-start");
            }
        }
        // RE-DISPARO do full-body dirigido pelo TICK LOOP DO RUST (2026-07-15): o ActivateTPPRepresentation
        // é transiente em free-roam (~25s). Em vez de um DelayCallback de redscript que se reagenda (o
        // objeto script cai de escopo no contexto callg e o motor crasha ~1m57s tocando a ref pendente —
        // provado: crasha até com o Call() nunca disparando), o Rust re-chama uma função redscript
        // ONE-SHOT (`BwmsTppRefire`) a cada ~600 ticks (~10s). Cada chamada resolve o player fresco e
        // enfileira 1 evento — zero objeto de vida-longa, zero DelayCallback. Gate próprio (~/.bwms-tpp-refire)
        // pra não interferir no teste limpo do one-shot; só roda depois do FB_STARTED.
        let refire_on = marcador_de_dev(".bwms-tpp-refire");
        if phase5 && refire_on && FB_STARTED.load(Ordering::Relaxed) && crate::tempo::a_cada(TICKS.load(Ordering::Relaxed), 600) {
            let gi: Option<[u8; 16]> = unsafe {
                crate::jogo::sessao::game_instance(reg, player)
            };
            if let Some(gi) = gi {
                let f = unsafe { crate::register::get_function(reg, "BwmsTppRefire") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                    unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) };
                }
            }
        }
        // SEGURAR AS PERNAS EM PÉ (2026-07-16): as vars `fullbody`/`isTPP` do anim-graph precisam ser
        // reaplicadas RÁPIDO (~1.5s) pra segurar a pose entre os resets do ActivateTPP (a cada ~10s).
        // Cadência ~90 ticks. Mesmo gate (refire marker + FB_STARTED). Ver BwmsLegsHold no reds.
        if phase5 && refire_on && FB_STARTED.load(Ordering::Relaxed) && crate::tempo::a_cada(TICKS.load(Ordering::Relaxed), 60) {
            let gi: Option<[u8; 16]> = unsafe {
                crate::jogo::sessao::game_instance(reg, player)
            };
            if let Some(gi) = gi {
                let f = unsafe { crate::register::get_function(reg, "BwmsLegsHold") };
                if crate::rtti::sane(f) {
                    let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                    unsafe { crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) };
                }
            }
        }
    }
    // Running.OnUpdate (type=2) a cada 180 ticks de gameplay, com o app. Fora do bloco
    // `!in_crash_window` abaixo, que liga na primeira presença do player e não desliga: lá dentro os plugins nunca recebiam o OnUpdate em
    // gameplay. O callback é nativo, sem `call_func` periódico (o que a janela suprime).
    if TICKS.load(Ordering::Relaxed) % 180 == 90 && crate::inicio::boot::gameplay_real() {
        crate::mods_do_usuario::api::call_game_state_update(EstadoDoJogo::Running);
    }
    // CallbackSystem (lite): emite "Session/Ready" a cada ~120 ticks até despachar (espera o
    // OnGameAttached registrar). Aqui o "controller" = o tick de gameplay pronto; outros eventos
    // (input/entity) = hooks de função de jogo chamando fire_event. Ver tipos_do_motor::fire_event.
    if !in_crash_window {
        static CBS_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let t = TICKS.load(Ordering::Relaxed);
        // Reserva do `Session/Ready` antes da 1ª presença do jogador (depois dela o `in_crash_window` não desliga): desliga na 1ª entrega do
        // `OnGameRestored` do jogo; não sai na sessão em que o jogo ou o `session_ready_tick` (spawn do jogador, menu e carga de save) já
        // entregou (`inicio::sessao_do_jogo`).
        if !CBS_DONE.load(Ordering::Relaxed)
            && crate::tempo::a_cada(t, 120)
            && inicio::sessao_do_jogo::pela_reserva(inicio::sessao_do_jogo::SESSAO_PRONTA, "Session/Ready")
        {
            let n = unsafe { tipos_do_motor::fire_event("Session/Ready") };
            crate::veredito_dev!("[sessao-reserva] Session/Ready (sem objeto) → {n} callback(s)");
            if n > 0 {
                CBS_DONE.store(true, Ordering::Relaxed);
            }
        }
        // "Session/Update" = evento PERIÓDICO (onUpdate, callback contínuo de mod). 2º tipo de evento.
        if crate::tempo::a_cada(t, 180) {
            unsafe { tipos_do_motor::fire_event("Session/Update") };
        }
        // "Session/Tick" = evento periódico que PASSA DADO (o nº do tick) pro callback — prova que o
        // evento carrega args (= o que input/entity events precisam). callback = OnBwmsTick(n: Int32).
        if crate::tempo::a_cada(t, 240) {
            unsafe { tipos_do_motor::fire_event_args("Session/Tick", &[rtti::Arg::I32(t as u32)]) };
        }
    }
    // comando(s) pendente(s) (do overlay ou de fora, via /tmp) — suporta múltiplas linhas.
    // Gate: só processa em gameplay (phase==5). Durante boot/menu, comandos como `postloadprobe`
    // constroem objetos complexos antes do engine estar pronto → crash (2026-07-27).
    // O arquivo fica intacto até phase5 chegar — sem perda de comandos.
    // CORREÇÃO 2026-07-27: gate original usava GAME_SESSION_DESC+0x84 que trava no objeto TRANSIENTE
    // da engagement e NUNCA re-captura o objeto real pós-save-load (bug documentado em inicio/boot.rs:1039-1048).
    // Substituído por PHASE_REACHED_5 (event-based, set na 1ª vez que QUALQUER getter vê phase=5).
    // Player é garantido não-nulo aqui (gate player/tx em cima já retornou).
    // CORREÇÃO 2026-07-31 (RE do crash do `equiponce`): + `!exec_nested()` — não roda a fila se a
    // chamada ATUAL do executor está aninhada dentro de uma chamada do motor ainda em voo nesta
    // thread (mesmo mecanismo do crash: `call_func` síncrono disparado no meio de uma execução já
    // em andamento). Arquivo fica intacto (não é consumido) — tenta de novo no próximo tick não-aninhado.
    {
        let canal_phase5 = inicio::boot::PHASE_REACHED_5.load(Ordering::Relaxed);
        if canal_phase5 && !inicio::boot::exec_nested() {
            // Console in-game: 1 comando por tick, da fila em memória.
            let do_console = FILA_DE_COMANDOS.lock().ok().and_then(|mut f| f.pop_front());
            if let Some(cmd) = do_console {
                run_cmd(reg, player, tx, &cmd);
            }
            // Ferramental de dev (bwms-fastboot, smoke): o arquivo em /tmp, só com devtools.
            #[cfg(feature = "devtools")]
            {
                let content = std::fs::read_to_string("/tmp/cp77-cmd.txt").unwrap_or_default();
                let cmds: Vec<&str> = content.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                if !cmds.is_empty() {
                    let _ = std::fs::remove_file("/tmp/cp77-cmd.txt");
                    for cmd in cmds {
                        run_cmd(reg, player, tx, cmd);
                    }
                }
            }
        }
    }
    // CallbackSystem RawInput controller: emite "Input/Key" com um `ref<KeyInputEvent>` REAL
    // pra CADA tecla capturada no sendEvent durante gameplay (2026-07-18, input cru com nome real
    // — antes era o keycode cru como Int32; agora constrói+despacha o objeto real via
    // `Arg::Handle`, mesmo nome de evento REAL, `Input/Key`).
    // Tecla solta (2026-08-11): metade que faltava — emite "Input/KeyUp" com o MESMO
    // `ref<KeyInputEvent>` real (reuso puro do construtor já provado), pra CADA tecla solta
    // capturada no sendEvent durante gameplay. Nome de evento distinto (não reaproveita
    // "Input/Key") segue o padrão já estabelecido no projeto de sinalizar fases por NOME
    // (`Session/Start`/`Session/End`, `Overlay/Open`/`Overlay/Close`) em vez de um campo
    // extra no evento.
    // `teclas` já saiu das filas no topo do tick, com ou sem ouvinte; `despachar` só constrói um
    // `KeyInputEvent` por tecla para o evento que tem ouvinte, e o resto do lote cai no drop.
    // Com o hook do `ProcessInputEvents`, a tecla vem da fila do motor e o caminho do AppKit fica mudo (senão
    // cada tecla dispararia 2x); as teclas do AppKit deste tick caem no drop.
    let rawinput_motor = crate::selftest::rawinput_hook_ativo();
    if rawinput_motor {
        unsafe { crate::selftest::rawinput_despachar() };
    }
    teclas.despachar(rawinput_motor, tipos_do_motor::tem_ouvinte, |evento, acao, key, shift, control, alt| unsafe {
        entrada::disparar_evento_de_tecla(evento, acao, key, shift, control, alt);
    });
}

/// Faz o marshalling de UM arg do canal (`i:`/`f:`/`b:`/`n:`/`e:`/`s:` ou i32 cru) → `rtti::Arg`,
/// retornando `Err` explícito em vez de `I32(0)` silencioso. Isto é o coração de TODO
/// `call/callf/callg/callon` + do plugin C-ABI: um prefixo com typo (`ii:5`) ou um valor inválido
/// antes contaminava uma chamada de método do jogo VIVA com um zero mudo. Puro/testável offline.
fn parse_cmd_arg_checked(s: &str) -> Result<rtti::Arg, String> {
    if let Some((ty, val)) = s.split_once(':') {
        return match ty {
            "i" => parse_u32_flex(val)
                .map(rtti::Arg::I32)
                .ok_or_else(|| format!("i:{val} não é int (decimal ou 0xHEX)")),
            "f" => val
                .trim()
                .parse::<f32>()
                .map(rtti::Arg::F32)
                .map_err(|_| format!("f:{val} não é float")),
            "b" => parse_bool_flex(val)
                .map(rtti::Arg::Bool)
                .ok_or_else(|| format!("b:{val} não é bool (true/false/1/0/yes/no/on/off)")),
            "n" => Ok(rtti::Arg::CName(crate::cname::cname(val))),
            "e" => parse_u64_flex(val)
                .map(rtti::Arg::Enum)
                .ok_or_else(|| format!("e:{val} não é enum int (decimal ou 0xHEX)")),
            "s" => Ok(rtti::Arg::Str(val.to_string())),
            // Objeto vivo como handle, com a contagem sentinela que nunca solta (a mesma dos outros handles montados pelo BWMS).
            "h" => parse_u64_flex(val)
                .filter(|p| *p != 0)
                .map(|p| rtti::Arg::Handle(p as *mut core::ffi::c_void, unsafe { crate::rtti::refcnt() }))
                .ok_or_else(|| format!("h:{val} não é endereço (0xHEX)")),
            other => Err(format!(
                "prefixo de arg desconhecido '{other}:' em '{s}' — use i:/f:/b:/n:/e:/s:/h: (ou i32 cru)"
            )),
        };
    }
    // sem prefixo = i32 cru (decimal ou 0xHEX).
    parse_u32_flex(s)
        .map(rtti::Arg::I32)
        .ok_or_else(|| format!("arg '{s}' sem prefixo e não é int — use i:/f:/b:/n:/e:/s:"))
}

/// Wrapper que preserva a assinatura antiga (os 5+ callers de `call*` não mudam): em erro, LOGA o
/// motivo (não é mais silencioso) e cai pra `I32(0)`.
fn parse_cmd_arg(s: &str) -> rtti::Arg {
    match parse_cmd_arg_checked(s) {
        Ok(a) => a,
        Err(e) => {
            crate::log_dev!("[arg] {e} -> usando I32(0)");
            rtti::Arg::I32(0)
        }
    }
}

/// `ResourceDepot.ResourceExists(path)` (`PENDENCIAS-UNIFICADAS.md`),
/// a API redscript-facing que um mod chamaria pelo import do módulo de depósito. Composição de 2
/// peças JÁ PROVADAS, zero RE nova: `PATHB_DEPOT` (o depot real, capturado 1x em
/// `InitializeArchives`, sempre instalado desde o Facade — mesma fonte que `RegisterArchive`
/// já usa) + `ResourceDepot::CheckResource@0x10151aa2c` (achado 2026-07-16, PROVADO via
/// redirect de path inexistente; `bool(depot*, ResourcePath hash)`). Chamada DIRETA (sem hook/replace —
/// só invoca o endereço como função pura), zero mutação, zero side-effect.
pub unsafe fn resource_exists(path: &str) -> bool {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    if depot.is_null() {
        crate::log_dev!("[depot] ResourceExists: depot ainda não capturado (InitializeArchives não rodou) — abortado");
        return false;
    }
    let check_addr = crate::rebase(0x1_0151_aa2c);
    if !crate::memoria::is_readable(check_addr as *const c_void, 4) {
        crate::log_dev!("[depot] ResourceExists: CheckResource ilegível — abortado");
        return false;
    }
    let hash = bwms_primitives::resource_path_hash(path);
    let f: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(check_addr);
    let r = f(depot, hash) != 0;
    crate::log_dev!("[depot] ResourceExists('{path}', hash={hash:#018x}) = {r}");
    r
}

/// `ResourceDepot::ResourceExists` pelo hash do caminho, sem log (roda em thread de job no `OnResolveDefinition`). `None` com o
/// depot ainda não capturado ou o `CheckResource` ilegível.
pub(crate) unsafe fn resource_exists_hash(hash: u64) -> Option<bool> {
    let depot = PATHB_DEPOT.load(Ordering::Relaxed);
    let check_addr = crate::rebase(0x1_0151_aa2c);
    if depot.is_null() || !crate::memoria::is_readable(check_addr as *const c_void, 4) {
        return None;
    }
    let f: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(check_addr);
    Some(f(depot, hash) != 0)
}

/// Validação OFFLINE (zero jogo, zero boot) do filtro `inksystem_shape_matches` (`jogo/interface.rs`) —
/// possível porque `memoria::is_readable` lê via `mach_vm_read_overwrite(mach_task_self_, ...)`,
/// ou seja, SEMPRE a memória do PRÓPRIO processo chamador. Rodando dentro de `cargo test`, esse
/// "próprio processo" é o binário de teste — então dá pra montar um buffer LOCAL com o layout
/// EXATO de `Red::InkSystem` (endereços reais desta memória, não fabricados) e confirmar que o
/// filtro aceita/rejeita exatamente como projetado, sem precisar do jogo rodando. Isso responde
/// ao pedido da rodada (2026-08-14): "se identificar um jeito de validar o filtro OFFLINE, use
/// isso" — não havia dump de memória salvo em `cp77-symbols/` pra reusar, mas construir um
/// candidato sintético em memória real do processo de teste é uma alternativa igualmente válida
/// (e mais forte: cobre também os casos NEGATIVOS, um dump gravado só cobriria 1 estado).
#[cfg(test)]
mod inksystem_shape_tests {
    use crate::jogo::interface::inksystem_shape_matches;
    use std::ptr::write_unaligned;

    /// Monta um buffer de 0x400 bytes com um candidato
    /// BEM-FORMADO no layout exato que `inksystem_shape_matches` espera:
    /// - `inputWidget@0x2E8`/`requestsHandler@0x370`: `WeakHandle{instance,refCount}`, ambos
    ///   auto-referenciando o início do buffer (`base`) — sempre ponteiro mapeado válido nesta
    ///   memória de teste.
    /// - `requestsHandler.refCount` (`base`) aponta pro `storage[0]`, que guarda um `RefCnt`
    ///   pequeno e são de propósito (`strongRefs=1, weakRefs=1`) — sem isso, o valor cru de
    ///   `base` (um endereço de heap de verdade, quase sempre >10_000 nos bits baixos) seria
    ///   mal-interpretado como contador implausível e reprovaria por engano.
    /// - `layerManagers@0x380`: `entries` aponta pra `base+0x40` (região SEPARADA de
    ///   `storage[0]`, sem colisão), onde vive o elemento `[0]` `{instance=base,
    ///   refCount=base}` — não-nulo, mapeado, par consistente. `cap=4, size=3`.
    /// - `keyboardState@0x2F8` = `0x2A` (só bits baixos, nenhum bit reservado `b10`).
    fn build_valid_candidate() -> (Box<[u64; 128]>, u64) {
        let mut storage = Box::new([0u64; 128]); // 128*8 = 0x400 bytes, 8-alinhado
        let base = storage.as_ptr() as u64;
        assert!(base > 0x10000 && base % 8 == 0, "endereço de heap do teste fora do esperado: {base:#x}");
        unsafe {
            let p = storage.as_mut_ptr() as *mut u8;
            // storage[0] (byte 0x00): RefCnt{strongRefs=1,weakRefs=1} — alvo de requestsHandler.refCount.
            write_unaligned(p as *mut u32, 1u32);
            write_unaligned(p.add(4) as *mut u32, 1u32);
            // entries[0] (byte 0x40, região separada de storage[0]) — {instance,refCount} = base.
            write_unaligned(p.add(0x40) as *mut u64, base);
            write_unaligned(p.add(0x48) as *mut u64, base);
            // inputWidget@0x2E8
            write_unaligned(p.add(0x2E8) as *mut u64, base);
            write_unaligned(p.add(0x2E8 + 8) as *mut u64, base);
            // keyboardState@0x2F8 — só bits baixos (shiftLeft+controlLeft+altLeft, ex.), 0 nos reservados.
            write_unaligned(p.add(0x2F8) as *mut u16, 0x2Au16);
            // requestsHandler@0x370 — refCount aponta pro RefCnt são em storage[0].
            write_unaligned(p.add(0x370) as *mut u64, base);
            write_unaligned(p.add(0x370 + 8) as *mut u64, base);
            // layerManagers@0x380 — entries aponta pro elemento[0] em base+0x40, cap=4 size=3.
            write_unaligned(p.add(0x380) as *mut u64, base + 0x40);
            write_unaligned(p.add(0x388) as *mut u32, 4u32);
            write_unaligned(p.add(0x38C) as *mut u32, 3u32);
        }
        (storage, base)
    }

    #[test]
    fn candidato_bem_formado_passa() {
        let (_storage, base) = build_valid_candidate();
        let got = unsafe { inksystem_shape_matches(base) };
        let (kb, cap, size) = got.expect("candidato sintético bem-formado deveria bater");
        assert_eq!(cap, 4);
        assert_eq!(size, 3);
        assert_eq!(kb & 0xFC00, 0, "keyboardState do candidato não devia ter bit reservado setado");
    }

    /// O ACHADO-CHAVE desta rodada: este é o padrão que dominava os 987/1020 "candidatos" do
    /// filtro antigo — um bloco de heap zerado batia `cap==0<=>entries==0` (trivialmente
    /// satisfeito) e os 2 ponteiros null-ou-null passavam o teste frouxo de "plausível". O
    /// filtro novo EXIGE `layerManagers.cap>=1`, matando essa classe inteira de falso-positivo.
    #[test]
    fn zerado_por_completo_e_rejeitado_era_o_ruido_dominante() {
        let storage = Box::new([0u64; 128]);
        let base = storage.as_ptr() as u64;
        let got = unsafe { inksystem_shape_matches(base) };
        assert!(got.is_none(), "bloco 100% zerado não pode mais bater — cap=0 agora é rejeitado");
    }

    #[test]
    fn keyboardstate_com_bit_reservado_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x2F8) as *mut u16, 0xFFFFu16) };
        assert!(unsafe { inksystem_shape_matches(base) }.is_none(), "bit reservado (b10) setado tinha que reprovar");
    }

    #[test]
    fn nulidade_despareada_em_inputwidget_e_rejeitada() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            // instance != 0 mas refCount == 0 — nunca acontece num WeakHandle/SharedPtrBase real
            // (todo construtor seta os 2 juntos).
            write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x2E8 + 8) as *mut u64, 0);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "nulidade despareada (instance≠0, refCount=0) tinha que reprovar"
        );
    }

    #[test]
    fn layermanagers_vazio_e_rejeitado_era_aceito_pelo_filtro_antigo() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            let p = storage.as_mut_ptr() as *mut u8;
            write_unaligned(p.add(0x380) as *mut u64, 0);
            write_unaligned(p.add(0x388) as *mut u32, 0);
            write_unaligned(p.add(0x38C) as *mut u32, 0);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "layerManagers vazio (cap=0) tinha que reprovar — o filtro antigo aceitava isso"
        );
    }

    #[test]
    fn size_maior_que_cap_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x38C) as *mut u32, 999u32) };
        assert!(unsafe { inksystem_shape_matches(base) }.is_none(), "size>cap tinha que reprovar");
    }

    #[test]
    fn primeiro_elemento_do_array_nulo_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe { write_unaligned((storage.as_mut_ptr() as *mut u8).add(0x40) as *mut u64, 0) };
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "1º elemento do array nulo (size>=1 mas slot0 vazio) tinha que reprovar — sinal novo, o filtro antigo nunca olhava dentro do array"
        );
    }

    #[test]
    fn refcount_implausivel_e_rejeitado() {
        let (mut storage, base) = build_valid_candidate();
        unsafe {
            // requestsHandler.refCount continua apontando pro storage[0], mas o strongRefs
            // gravado lá vira um número absurdo (nenhum RefCnt real tem bilhões de referências).
            write_unaligned(storage.as_mut_ptr() as *mut u32, 0xFFFF_FFFFu32);
        }
        assert!(
            unsafe { inksystem_shape_matches(base) }.is_none(),
            "RefCnt com contador implausível (bilhões) tinha que reprovar"
        );
    }
}

/// Quantos comandos o terminal aceita neste build: nomes distintos que o `run_cmd` despacha, sem os apelidos
/// (`help`, `?`, `god`, `car`, `cls`) e sem os 5 de `autoproceed`. Alimenta a ajuda ("N de engenharia reversa"); o teste
/// `o_catalogo_nao_promete_comando_que_nao_existe` cuida do outro lado, que é a tabela não mentir.
/// O público tem os 34 que respondem na aba ou mexem no jogo; o comando cuja única saída é o log de dev fica só no dev
/// (decisão de 2026-10-13). `o_total_do_publico_e_o_do_fonte` reconta o público no fonte; o do dev é contado à mão,
/// porque os `rettest*` saem de um `match` próprio.
const TOTAL_DE_COMANDOS: usize = if cfg!(feature = "devtools") { 206 } else { 34 };

/// O terminal do mod. Vai no build PÚBLICO com todos os comandos que respondem ao usuário ou mexem no jogo: digitar
/// ali e receber o que se pediu é o produto, não ferramenta interna. O que sai do build público é o
/// instrumental de DESCOBERTA — o `selftest`, as sondas, os marcadores que usamos pra achar gap, os
/// comandos que só provam item já fechado (régua de 2026-10-03) e os que só falam pelo log de dev (decisão de
/// 2026-10-13: `#[cfg(feature = "devtools")]` no braço) —, nunca um comando que responde ao usuário.
fn run_cmd(reg: &rtti::Registry, player: *mut c_void, tx: *mut c_void, cmd: &str) {
    // Eco do que foi digitado, ANTES de qualquer rota. É o que transforma "a tela não mostra nada" em
    // "o mod recebeu o comando": vale pra todos, sem precisar tocar em cada um.
    crate::resposta!("> {cmd}");
    // `foto <caminho>`: screenshot do quadro do jogo pelo motor (prova visual de lane, sem captura de tela do sistema).
    #[cfg(all(feature = "devtools", feature = "lua"))]
    if let Some(caminho) = cmd.strip_prefix("foto ") {
        let ok = unsafe { crate::jogo::motor::screenshot(caminho.trim(), crate::jogo::motor::MODO_NORMAL) };
        crate::veredito_dev!("[lua] foto {}: pedido={ok}", caminho.trim());
        return;
    }
    // `luaobserva <classe> <método>`: `Observe` de prova num método que o jogo chama sozinho.
    #[cfg(all(feature = "devtools", feature = "lua"))]
    if let Some(args) = cmd.strip_prefix("luaobserva ") {
        let mut p = args.split_whitespace();
        let ok = match (p.next(), p.next()) {
            (Some(c), Some(m)) => unsafe { crate::mods_lua::observar_de_prova(c, m) },
            _ => false,
        };
        crate::veredito_dev!("[lua] luaobserva {args}: registrado={ok}");
        return;
    }
    // `luadraw`: quadros com o `onDraw` dos mods Lua rodado e pulado (trava do Lua ocupada).
    #[cfg(all(feature = "devtools", feature = "lua"))]
    if cmd == "luadraw" {
        let [r, p] = &crate::mods_lua::QUADROS_DO_ONDRAW;
        crate::veredito_dev!("[lua] onDraw: rodou {} quadro(s), pulou {}", r.load(Ordering::Relaxed), p.load(Ordering::Relaxed));
        return;
    }
    // `luarun <arquivo>`: roda um `.lua` de diagnóstico no estado dos mods Lua, com o V em cena. O arquivo fica fora do repositório.
    #[cfg(all(feature = "devtools", feature = "lua"))]
    if let Some(arquivo) = cmd.strip_prefix("luarun ") {
        match std::fs::read_to_string(arquivo.trim()) {
            Ok(codigo) => unsafe { crate::mods_lua::run_code(&codigo) },
            Err(e) => crate::veredito_dev!("[lua] luarun {}: {e}", arquivo.trim()),
        }
        return;
    }
    // `luatecla <id>`: o `registerHotkey` de um mod Lua pelo id, sem tecla (prova de lane).
    #[cfg(all(feature = "devtools", feature = "lua"))]
    if let Some(id) = cmd.strip_prefix("luatecla ") {
        let ok = unsafe { crate::mods_lua::disparar_hotkey(id.trim()) };
        crate::veredito_dev!("[lua] luatecla {}: disparou={ok}", id.trim());
        return;
    }
    // `clear` (ou `cls`): limpa o scrollback do console (trunca o log) — estilo terminal.
    if cmd == "clear" || cmd == "cls" {
        terminal::overlay::clear_console_view(); // view-only: limpa a tela, mantém o arquivo p/ debug
        return;
    }
    // `ovcall` — validação do Override-suppress: chama BwmsProbe a partir do RUST (aqui,
    // run_cmd, SEM segurar o lock do Lua) → o override registrado via Lua consegue rodar
    // seu callback (o teste que chamava de dentro de um callback Lua/Cron falhava por
    // re-entrância: o lock do Lua estava seguro → `call_hook_override` dava try_lock Err →
    // override pulado). Na vida real o JOGO (nativo/redscript) chama o método, igual a isto.
    #[cfg(feature = "devtools")]
    if cmd == "ovcall" {
        unsafe {
            let rp = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbe");
            let rc = rtti::resolve_func(reg, "PlayerPuppet", "BwmsProbeCalls");
            match (rp, rc) {
                (Some(rp), Some(rc)) => {
                    let i32_of = |o: Option<[u8; 0x20]>| {
                        o.map(|r| i32::from_le_bytes([r[0], r[1], r[2], r[3]])).unwrap_or(-1)
                    };
                    let before = i32_of(rtti::call_func(&rc, player, &[]));
                    // arma o override RUST-nativo de BwmsProbe→42 (sem lua) só p/ esta chamada
                    inicio::boot::RUST_OV_CNAME
                        .store(crate::cname::cname("BwmsProbe"), Ordering::Relaxed);
                    inicio::boot::RUST_OV_VAL.store(42, Ordering::Relaxed);
                    let ret = i32_of(rtti::call_func(&rp, player, &[])); // override RUST dispara AQUI
                    inicio::boot::RUST_OV_CNAME.store(0, Ordering::Relaxed); // desarma
                    let after = i32_of(rtti::call_func(&rc, player, &[]));
                    let verdict = if ret == 42 && after == before {
                        ">>> OVERRIDE-SUPPRESS OK (retorno reescrito + original suprimida) <<<"
                    } else if ret == 42 {
                        "retorno 42 mas a original rodou (rewrite, sem suppress)"
                    } else if after > before {
                        "original rodou (override nao pegou — CName/registro)"
                    } else {
                        "indefinido"
                    };
                    crate::log_dev!(
                        "[ovcall] retorno={ret} (esperado 42) | contador {before}->{after} | {verdict}"
                    );
                }
                _ => crate::log_dev!("[ovcall] BwmsProbe/BwmsProbeCalls nao resolveram (sonda compilada?)"),
            }
        }
        return;
    }
    // Hooks de produção — os 3 modos (Observe/Override/Suppress) num método REAL do jogo
    // (`HasGodMode`, read-only), caminho NÃO-Lua. Override+Suppress já provados via `ovcallreal`
    // (2026-07-13); aqui fecha a perna OBSERVE que faltava (`RUST_OBS_CNAME`, inicio/boot.rs): o
    // callback dispara (log + contador) e a original AINDA roda (sem suprimir).
    // `redscript-thirdparty-proof` via CANAL (roda no cp77_tick, dentro do executor, seguro pra
    // chamar a VM/redscript — ao contrário de uma thread nossa separada). Resolve GameInstance via
    // PlayerPuppet.GetGame(ctx=player) — a receita comprovada do BwmsBootFullbody — e chama
    // BwmsTestThirdPartyCheat3p(game) do mod de 3o. Evita o GetGameInstance() global (instância morta
    // quando chamado fora do contexto normal — achado 2026-07-15, cont.57).
    // Full-body: dispara BwmsBootFullbody(game) via CANAL (mesmo padrão do 3ptest) — bypassa o gate
    // automático do cp77_tick (TICKS/fb_now) que não disparou num boot com foco/timing atípico.
    // `redscript-cheat-effects-proof`: chama BwmsCheatEffectsTest(game) — exercita os cheats via a
    // MESMA instância/método (SettingsSelectorControllerBool.BWMSRun) que o clique real aciona.
    #[cfg(feature = "devtools")]
    if cmd == "cheatfxtest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[cheatfx] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsCheatEffectsTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            crate::log_dev!("[cheatfx] BwmsCheatEffectsTest(game) chamado via canal");
                        } else {
                            crate::log_dev!("[cheatfx] call_func não completou");
                        }
                    } else {
                        crate::log_dev!("[cheatfx] BwmsCheatEffectsTest não resolveu (mod de teste compilado?)");
                    }
                }
                None => crate::log_dev!("[cheatfx] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Clique na aba Cheats das Configurações: arma os passos do `bwms-aba-cheats-teste.reds` (`diagnostico::ABA_CHEATS`).
    #[cfg(feature = "devtools")]
    if cmd == "abacheats" {
        crate::diagnostico::aba_cheats_armar();
        return;
    }
    // Eventos de ciclo de vida: força o toggle do overlay (sem HID) — o cp77_tick do próximo tick
    // dispara onOverlayOpen/Close pela borda real (mesmo caminho que o backtick real usa).
    if cmd == "overlaytoggle" {
        let now = !terminal::overlay::is_shown();
        terminal::overlay::set_shown(now);
        crate::log_dev!("[overlaytoggle] SHOW agora={now}");
        return;
    }
    // Clique e procura de dev no terminal aberto, sem evento de sistema: o render atende um pedido por vez e o veredito sai em
    // `[terminal-clique]`/`[terminal-onde]` (`terminal::overlay::pedir_clique`).
    #[cfg(feature = "devtools")]
    if let Some(args) = cmd.strip_prefix("termclique ") {
        terminal::overlay::pedir_clique(args);
        return;
    }
    #[cfg(feature = "devtools")]
    if let Some(rotulo) = cmd.strip_prefix("termonde ") {
        terminal::overlay::pedir_onde(rotulo);
        return;
    }
    // Player, agendamento e veículo: chama a extensão REAL `DelaySystem.DelayEvent`.
    #[cfg(feature = "devtools")]
    if cmd == "delaytest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[cw-delaytest] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsDelaySystemTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        crate::log_dev!("[cw-delaytest] BwmsDelaySystemTest(game) chamado via canal");
                    } else {
                        crate::log_dev!("[cw-delaytest] BwmsDelaySystemTest não resolveu");
                    }
                }
                None => crate::log_dev!("[cw-delaytest] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "overlaylctest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[overlay-lifecycle] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsOverlayLifecycleTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        crate::log_dev!("[overlay-lifecycle] BwmsOverlayLifecycleTest(game) chamado via canal");
                    } else {
                        crate::log_dev!("[overlay-lifecycle] BwmsOverlayLifecycleTest não resolveu");
                    }
                }
                None => crate::log_dev!("[overlay-lifecycle] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Teste DEFINITIVO do problema real (pernas dobradas vs em pé) — torso+pernas+câmera numa chamada.
    #[cfg(feature = "devtools")]
    if cmd == "legstilt" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[legstilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsLegsTiltTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        crate::log_dev!("[legstilt] BwmsLegsTiltTest(game) chamado via canal");
                    } else {
                        crate::log_dev!("[legstilt] BwmsLegsTiltTest não resolveu");
                    }
                }
                None => crate::log_dev!("[legstilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "scenetier" {
        // `redDispatcher-crash-any-callfunc-near-phase5` (2026-07-18, achado do coordenador): o
        // screenshot do `legstilt` anterior pousou numa cena/animação roteirizada (V sentado,
        // jornal), não free-roam — confound real. `BwmsGetSceneTier(game)` (bwms-tppcam.reds) lê
        // `PlayerPuppet.GetSceneTier` (gamePSMHighLevel): 0=Default/free-roam real, 1-5=cena,
        // 6=nadando. Checar isto == 0 ANTES de disparar `legstilt` de novo.
        unsafe {
            if player.is_null() {
                crate::log_dev!("[scenetier] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsGetSceneTier");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if let Some(ret) = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            let tier = i32::from_le_bytes([ret[0], ret[1], ret[2], ret[3]]);
                            crate::log_dev!("[scenetier] BwmsGetSceneTier(game) -> tier={tier} (0=free-roam real, 1-5=cena, 6=nadando)");
                        } else {
                            crate::log_dev!("[scenetier] call_func não retornou");
                        }
                    } else {
                        crate::log_dev!("[scenetier] BwmsGetSceneTier não resolveu");
                    }
                }
                None => crate::log_dev!("[scenetier] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Torso + tilt NUMA CHAMADA SÓ (menos round-trips de canal = menos superfície pra crash).
    #[cfg(feature = "devtools")]
    if cmd == "fbtilt" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[fbtilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsFullbodyTiltTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        crate::log_dev!("[fbtilt] BwmsFullbodyTiltTest(game) chamado via canal");
                    } else {
                        crate::log_dev!("[fbtilt] BwmsFullbodyTiltTest não resolveu");
                    }
                }
                None => crate::log_dev!("[fbtilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Diagnóstico do force-look: chama BwmsCamTiltOnce(game) — isola se a escrita da câmera aplica de
    // vez ou é revertida no frame seguinte pela câmera nativa.
    #[cfg(feature = "devtools")]
    if cmd == "camtilt" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[camtilt] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsCamTiltOnce");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
                        crate::log_dev!("[camtilt] BwmsCamTiltOnce(game) chamado via canal");
                    } else {
                        crate::log_dev!("[camtilt] BwmsCamTiltOnce não resolveu");
                    }
                }
                None => crate::log_dev!("[camtilt] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "fbtest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[fbtest] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsBootFullbody");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            crate::log_dev!("[fbtest] BwmsBootFullbody(game) chamado via canal com GameInstance real");
                        } else {
                            crate::log_dev!("[fbtest] call_func não completou");
                        }
                    } else {
                        crate::log_dev!("[fbtest] BwmsBootFullbody não resolveu");
                    }
                }
                None => crate::log_dev!("[fbtest] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // equiprawv5 (2026-08-03): verificação DEFINITIVA — chama `BwmsCheckItemQty(game)` (redscript,
    // TransactionSystem.GetItemQuantity real) com GameInstance resolvido em Rust, mesmo padrão do
    // `fbtest` acima. Confirma se `QueueRequest` via frame real (equiprawv5) de fato deu o item.
    #[cfg(feature = "devtools")]
    if cmd == "itemqty" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[itemqty] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsCheckItemQty");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(r) => crate::log_dev!("[itemqty] BwmsCheckItemQty(game) -> qty={}", i32::from_le_bytes([r[0],r[1],r[2],r[3]])),
                            None => crate::log_dev!("[itemqty] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[itemqty] BwmsCheckItemQty não resolveu");
                    }
                }
                None => crate::log_dev!("[itemqty] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equipslotcheck" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equipslotcheck] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsCheckEquipSlot");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(r) => crate::log_dev!("[equipslotcheck] BwmsCheckEquipSlot(game) -> equipped={}", crate::rtti::bool_do_retorno(&r)),
                            None => crate::log_dev!("[equipslotcheck] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[equipslotcheck] BwmsCheckEquipSlot não resolveu");
                    }
                }
                None => crate::log_dev!("[equipslotcheck] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // enumcomp: corpo inteiro/torso (cont.41) — enumera DIRETO os componentes reais do player
    // (Entity+0xA0, offset ground-truth já usado no construtor de entidade) e conta quantos são
    // SkinnedMeshComponent, logando enabled/res_ptr de cada um. Observe-only, zero mutação.
    #[cfg(feature = "devtools")]
    if cmd == "enumcomp" {
        unsafe {
            log(&aparencia::run_enum_skinnedmesh(reg, player));
        }
        return;
    }
    // toggle <cname>: corpo inteiro/torso (cont.41) — ACHADO: 'torso'/'legs'/'shoes' existem como
    // entSkinnedMeshComponent reais do player, mesh carregado (res_ptr válido), mas enabled=0.
    // Este comando escreve enabled=1 no componente com o CName pedido (offset +0x8b, já usado
    // com segurança em IComponent::Toggle desde 2026-07-28). ESCRITA REAL — não gated por
    // padrão além de precisar de dev_mode (mesmo canal de todo comando de console).
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("toggle ") {
        let name = rest.trim();
        unsafe {
            log(&diagnostico::run_toggle_component_by_cname(reg, player, name));
        }
        return;
    }
    // togglerebuild <cname>: corpo inteiro/torso (cont.41) — `toggle` sozinho persiste mas não
    // muda o visual (confirmado por screenshot); esta variante TAMBÉM chama o método virtual
    // "rebuild render proxy" (vtable+0x288, já observado seguro) na mesma instância, testando
    // se a reconstrução do proxy é o passo que falta pro mesh aparecer.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("togglerebuild ") {
        let name = rest.trim();
        unsafe {
            log(&aparencia::run_toggle_and_rebuild(reg, player, name));
        }
        return;
    }
    // togglefull <cname>: corpo inteiro/torso (cont.42) — RE offline achou 2 leitores reais de
    // `enabled` (vtable+0x2d0/0x2d8, não o wrapper já testado); notificação de render exige
    // +0x24a==1 E +0x253 bit2, ambos falsos no torso por padrão. Seta os 3 campos + rebuild.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("togglefull ") {
        let name = rest.trim();
        unsafe {
            log(&diagnostico::run_toggle_full_and_rebuild(reg, player, name));
        }
        return;
    }
    // dissolve <cname>: corpo inteiro/torso (cont.42) — `+0x24a` é enum de estado (0/1/2), não
    // bool; simula a progressão 1→dispatch→2→dispatch que o sistema de dissolve/fade faria.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("dissolve ") {
        let name = rest.trim();
        unsafe {
            log(&aparencia::run_toggle_dissolve_sequence(reg, player, name));
        }
        return;
    }
    // forceopacity <cname>: corpo inteiro/torso (cont.42, 4ª rodada) — bypassa o gate de dissolve
    // inteiro: lê o proxy real (componente+0x1d8), força os 2 bits de +0x23 + SetOpacity flag
    // em +0x21, chama a função de commit (`0x104fe4710`) DIRETO.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("forceopacity ") {
        let name = rest.trim();
        unsafe {
            log(&diagnostico::run_force_proxy_opacity(reg, player, name));
        }
        return;
    }
    // dumpcomp <cname>: corpo inteiro/torso (cont.41) — dump hex bruto (0x300 bytes) do componente
    // achado por CName, pra comparação offline contra um componente sabidamente visível (ex.
    // pescoço FPP) e achar o offset real do gate de visibilidade (chunkMask ou equivalente).
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("dumpcomp ") {
        let name = rest.trim();
        unsafe {
            log(&diagnostico::run_dump_component_by_cname(reg, player, name));
        }
        return;
    }
    // dumpat <0xhex>: variante de dumpcomp que aceita ponteiro cru (já achado via enumcomp),
    // pra comparar um componente habilitado sem precisar adivinhar o CName exato.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("dumpat ") {
        let hexstr = rest.trim().trim_start_matches("0x");
        if let Ok(addr) = u64::from_str_radix(hexstr, 16) {
            unsafe {
                log(&diagnostico::run_dump_component_at(addr as *mut std::os::raw::c_void));
            }
        } else {
            crate::log_dev!("[dumpat] hex inválido: '{rest}'");
        }
        return;
    }
    // dumpres <0xptr_componente>: dump do RESOURCE apontado por res_ptr@+0x1f8 do componente
    // (não o componente em si). Hipótese: gate de visibilidade dentro do resource, não no comp.
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("dumpres ") {
        let hexstr = rest.trim().trim_start_matches("0x");
        if let Ok(addr) = u64::from_str_radix(hexstr, 16) {
            unsafe {
                log(&diagnostico::run_dump_resource_at(addr as *mut std::os::raw::c_void));
            }
        } else {
            crate::log_dev!("[dumpres] hex inválido: '{rest}'");
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv7" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv7] player null");
                return;
            }
            // Mesmo fix cirúrgico do equiprawv6 (idempotente — se já instalado nesta boot, só
            // reaponta o alvo). Precisa rodar ANTES da chamada em bytecode pra QueueRequest não
            // crashar em GetInvokable()==null.
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[equiprawv7] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[equiprawv7] install_getinvokable_fix falhou — abortando");
                return;
            }
            crate::log_dev!("[equiprawv7] fix de GetInvokable instalado/reapontado — chamando BwmsEquipViaQuery...");
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsEquipViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[equiprawv7] BwmsEquipViaQuery(game) chamado — ZERO CRASH, ver [equiprawv7] no log do reds"),
                            None => crate::log_dev!("[equiprawv7] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[equiprawv7] BwmsEquipViaQuery não resolveu");
                    }
                }
                None => crate::log_dev!("[equiprawv7] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Item 29, equipcyber (2026-08-14): mesmo padrão de equiprawv7, mas equipa `Items.MantisBlades`
    // (cyberware de braço REAL) em vez da T-shirt — teste DECISIVO do
    // `ComputePuppetArmsState`. Precisa de `give Items.MantisBlades` antes (posse real).
    #[cfg(feature = "devtools")]
    if cmd == "equipcyber" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equipcyber] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[equipcyber] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[equipcyber] install_getinvokable_fix falhou — abortando");
                return;
            }
            crate::log_dev!("[equipcyber] fix de GetInvokable instalado/reapontado — chamando BwmsEquipCyberwareViaQuery...");
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsEquipCyberwareViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[equipcyber] BwmsEquipCyberwareViaQuery(game) chamado — ZERO CRASH, ver [equipcyber] no log do reds"),
                            None => crate::log_dev!("[equipcyber] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[equipcyber] BwmsEquipCyberwareViaQuery não resolveu");
                    }
                }
                None => crate::log_dev!("[equipcyber] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Item 29, drawcyber (2026-08-18): teste DECISIVO #2 do estado dos braços — achado honesto de
    // 2026-08-14 (prova da detecção dos braços, inconclusiva)
    // confirmou que `equipcyber` move o item pra posse real (RightArm) mas não popula
    // `AttachmentSlots.WeaponRight` (a arma precisa estar DESEMBAINHADA/em uso). Mesmo padrão de
    // `equipcyber`, mas chama `BwmsDrawCyberwareAndCheckArms` (bwms-tppcam.reds) — despacha
    // `DrawItemRequest{itemID=CreateQuery(Items.MantisBlades)}` via QueueRequest (mesmo fix de
    // GetInvokable, já instalado se `equiprawv7`/`equipcyber` já rodaram nesta boot) e, na
    // sequência, chama `ComputePuppetArmsState` de novo e loga o ordinal (marcador 9613 +
    // EnumInt). Rodar depois de `give Items.MantisBlades` + `equipcyber`.
    #[cfg(feature = "devtools")]
    if cmd == "drawcyber" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[drawcyber] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[drawcyber] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[drawcyber] install_getinvokable_fix falhou — abortando");
                return;
            }
            crate::log_dev!("[drawcyber] fix de GetInvokable instalado/reapontado — chamando BwmsDrawCyberwareAndCheckArms...");
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsDrawCyberwareAndCheckArms");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[drawcyber] BwmsDrawCyberwareAndCheckArms(game) chamado — ZERO CRASH, ver [drawcyber]/[variant_log] no log do reds"),
                            None => crate::log_dev!("[drawcyber] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[drawcyber] BwmsDrawCyberwareAndCheckArms não resolveu");
                    }
                }
                None => crate::log_dev!("[drawcyber] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Transmog (2026-08-05): mesmo padrão de equiprawv7, com BwmsVisualEquipViaQuery
    // (EquipVisualsRequest em vez de EquipRequest) — testa se ChangeAppearanceToItem dispara sem
    // precisar da UI de Wardrobe/espelho. O fix de GetInvokable já é global pra QUALQUER request
    // que passe por QueueRequest, então não precisa reinstalar se equiprawv7 já rodou nesta boot,
    // mas reaponta de qualquer forma (idempotente, barato).
    #[cfg(feature = "devtools")]
    if cmd == "transmogviaquery" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[transmogviaquery] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[transmogviaquery] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[transmogviaquery] install_getinvokable_fix falhou — abortando");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsVisualEquipViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[transmogviaquery] BwmsVisualEquipViaQuery(game) chamado — ZERO CRASH, ver [transmogviaquery]/[axl-transmog] no log"),
                            None => crate::log_dev!("[transmogviaquery] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[transmogviaquery] BwmsVisualEquipViaQuery não resolveu");
                    }
                }
                None => crate::log_dev!("[transmogviaquery] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "playertest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[cw-player] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            // NOTA (2026-08-07): `camera::get_function` (CRTTISystem::GetFunction, vtbl+0x30)
            // NÃO é confiável pra achar função de SCRIPT pura (só acha o que a gente registrou
            // via Rust — já documentado no projeto como "PARQUEADO, não é o resolvedor real do
            // binder"). Pra chamar `PlayerSystem.GetPlayer()` (redscript puro, via @addMethod),
            // usa `resolve_func`/`call_func` — o mesmo mecanismo já provado dezenas de vezes
            // neste projeto (getf/setf/callf), que caminha o array de métodos real da classe.
            match gi {
                Some(gi) => {
                    let getps = crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetPlayerSystem");
                    let ps_handle = getps.as_ref().and_then(|f| crate::rtti::call_func(f, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]));
                    match ps_handle {
                        Some(h) => {
                            let ps_ptr = usize::from_le_bytes(h[..8].try_into().unwrap()) as *mut c_void;
                            crate::log_dev!("[cw-player] GameInstance.GetPlayerSystem(game) -> {ps_ptr:p}");
                            if !ps_ptr.is_null() {
                                // 2026-08-11, achado do agente de pesquisa: "PlayerSystem"
                                // é só um ALIAS script-time que o `scc` resolve em compile-time — o CNAME
                                // REAL na RTTI é `gamePlayerSystem` (mesma categoria de divergência já
                                // documentada #81-85, script-name vs native-name). Bug latente desde que
                                // esta linha foi escrita, nunca pego porque o bypass `callon`/`class_of`
                                // (2026-08-10) contorna resolução por nome.
                                match crate::rtti::resolve_func(reg, "gamePlayerSystem", "GetPlayer") {
                                    Some(gp) => match crate::rtti::call_func(&gp, ps_ptr, &[]) {
                                        Some(pb) => {
                                            let p_ptr = usize::from_le_bytes(pb[..8].try_into().unwrap()) as *mut c_void;
                                            crate::log_dev!(
                                                "[cw-player] PlayerSystem.GetPlayer() -> {p_ptr:p} (igual ao player capturado={})",
                                                p_ptr == player
                                            );
                                            crate::rtti::soltar_retorno(&gp, &pb);
                                        }
                                        None => crate::log_dev!("[cw-player] call_func(GetPlayer) não completou"),
                                    },
                                    None => crate::log_dev!("[cw-player] PlayerSystem.GetPlayer não resolveu (o @addMethod não bindou?)"),
                                }
                                match crate::rtti::resolve_func(reg, "gamePlayerSystem", "GetInventoryPuppet") {
                                    Some(gip) => match crate::rtti::call_func(&gip, ps_ptr, &[]) {
                                        Some(pb) => {
                                            let inv_ptr = usize::from_le_bytes(pb[..8].try_into().unwrap()) as *mut c_void;
                                            crate::log_dev!(
                                                "[cw-player] PlayerSystem.GetInventoryPuppet() -> {inv_ptr:p} (defined={})",
                                                !inv_ptr.is_null()
                                            );
                                            crate::rtti::soltar_retorno(&gip, &pb);
                                        }
                                        None => crate::log_dev!("[cw-player] call_func(GetInventoryPuppet) não completou"),
                                    },
                                    None => crate::log_dev!("[cw-player] PlayerSystem.GetInventoryPuppet não resolveu"),
                                }
                            }
                            if let Some(f) = &getps {
                                crate::rtti::soltar_retorno(f, &h);
                            }
                        }
                        None => crate::log_dev!("[cw-player] GameInstance.GetPlayerSystem falhou"),
                    }
                }
                None => crate::log_dev!("[cw-player] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // `updateregistrar` (2026-08-11, mesma sessão) — read-only: reporta o
    // ponteiro `UpdateRegistrar*` capturado pelo probe `update_registrar_group_probe` (se já
    // armado via `~/.bwms-updateregistrar-probe` e se o motor já chamou `RegisterUpdate` pra
    // algum sistema vanilla, o que acontece naturalmente durante o boot). Zero mutação.
    #[cfg(feature = "devtools")]
    if cmd == "updateregistrar" {
        let ptr = crate::diagnostico::UPDATE_REGISTRAR_CAPTURED_PTR.load(std::sync::atomic::Ordering::Relaxed);
        if ptr == 0 {
            crate::log_dev!("[updateregistrar] nenhum ponteiro capturado ainda (probe armado? RegisterUpdate já disparou?)");
        } else {
            crate::log_dev!("[updateregistrar] UpdateRegistrar* capturado = {:#018x}", ptr);
        }
        return;
    }
    // `registerupdate` (2026-08-11, mesma sessão) — a ÚLTIMA peça: constrói o
    // `Callback<>` real (ABI mapeada) e chama `RegisterUpdate` no `UpdateRegistrar*` já capturado,
    // registrando `bwms_update_tick_callback` (função MÍNIMA, só loga, nunca lê FrameInfo/JobQueue)
    // pra confirmar que a chamada de registro NÃO crasha e que o callback DISPARA de verdade nos
    // frames seguintes. Toca maquinaria de registro do motor ATIVAMENTE (não mais só observar) —
    // gate PRÓPRIO (`~/.bwms-registerupdate-confirm`) além do probe já precisar estar armado.
    #[cfg(feature = "devtools")]
    if cmd == "registerupdate" {
        let confirm = crate::marcador_de_dev(".bwms-registerupdate-confirm");
        if !confirm {
            crate::log_dev!("[registerupdate] BLOQUEADO: crie ~/.bwms-registerupdate-confirm p/ habilitar (toca registro ativo do motor)");
            return;
        }
        if player.is_null() {
            crate::log_dev!("[registerupdate] player null");
            return;
        }
        unsafe {
            crate::selftest::register_bwms_update_tick(player);
        }
        return;
    }
    // `sysbymap <classname>` (2026-08-11, sessão nova) — via SEGURA (zero
    // execução de código do motor): lê `systemMap` @ `GameInstance+0x08` DIRETO em vez de chamar
    // slot de vtable (mesmo campo-de-objeto, imune ao shift Itanium que já causou o crash da
    // tentativa anterior por vtable-call). Cross-valida contra `GetPlayerSystem` (native já
    // provado) quando classname=="gamePlayerSystem".
    #[cfg(feature = "devtools")]
    if let Some(rest) = cmd.strip_prefix("sysbymap ") {
        let class_name = rest.trim();
        unsafe {
            if player.is_null() {
                crate::log_dev!("[sysbymap] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let game_ptr = match gi {
                Some(g) => usize::from_le_bytes(g[..8].try_into().unwrap()) as *mut c_void,
                None => {
                    crate::log_dev!("[sysbymap] PlayerPuppet.GetGame falhou");
                    return;
                }
            };
            let itype = reg.class_by_name(class_name);
            if itype.is_null() {
                crate::log_dev!("[sysbymap] class_by_name('{class_name}') não resolveu");
                return;
            }
            let sys_ptr = crate::jogo::sessao::sistema_do_motor(game_ptr, itype);
            crate::log_dev!("[sysbymap] GetSystem('{class_name}') -> {sys_ptr:p} game={game_ptr:p} itype={itype:p}");
            if class_name == "gamePlayerSystem" {
                let known_ptr = crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetPlayerSystem")
                    .map_or(std::ptr::null_mut(), |f| crate::rtti::call_ptr(&f, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi.unwrap())]));
                if !known_ptr.is_null() {
                    crate::log_dev!(
                        "[sysbymap] cross-check: GameInstance.GetPlayerSystem(game) -> {known_ptr:p} (match={})",
                        known_ptr == sys_ptr
                    );
                }
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equipglasses" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equipglasses] player null");
                return;
            }
            // mesmo fix cirúrgico de GetInvokable (idempotente).
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[equipglasses] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[equipglasses] install_getinvokable_fix falhou — abortando");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsEquipGlassesViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[equipglasses] BwmsEquipGlassesViaQuery(game) chamado — ver [equipglasses] no log do reds"),
                            None => crate::log_dev!("[equipglasses] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[equipglasses] BwmsEquipGlassesViaQuery não resolveu");
                    }
                }
                None => crate::log_dev!("[equipglasses] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equipoutfit" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equipoutfit] player null");
                return;
            }
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[equipoutfit] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[equipoutfit] install_getinvokable_fix falhou — abortando");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsEquipOutfitViaQuery");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        match crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]) {
                            Some(_) => crate::log_dev!("[equipoutfit] BwmsEquipOutfitViaQuery(game) chamado — ver [equipoutfit] no log do reds"),
                            None => crate::log_dev!("[equipoutfit] call_func não completou"),
                        }
                    } else {
                        crate::log_dev!("[equipoutfit] BwmsEquipOutfitViaQuery não resolveu");
                    }
                }
                None => crate::log_dev!("[equipoutfit] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Garment/transmog (2026-08-03, /goal): achado ao vivo — `equipctxonce` E
    // `equiprawonce` (que chamam um native de 2 params `handle` de DENTRO do bytecode de
    // `BwmsForceEquipCtx`/`Raw`) crasham no MESMO 0x101099d98 de sempre, ANTES de qualquer log nosso
    // aparecer (nem o Print imediatamente antes, nem o log de entrada do trampolim Rust). Isso aponta
    // pro DESPACHO do native aninhado como o problema, não a lógica interna dele. Via nova: ZERO
    // native aninhado. `BwmsPrepEquipSys`/`BwmsPrepEquipReq` só CONSTROEM e RETORNAM objetos (sem
    // chamar nenhum native de 2-handle-params) — 2 chamadas TOP-LEVEL separadas (mesmo padrão
    // provado de `getcustsys`: ponteiro do objeto = primeiros 8 bytes do retorno). O `transmute` pro
    // endereço cru roda 100% aqui em `run_cmd`, sem NENHUM native aninhado no meio.
    // 2026-08-03 (/goal): diagnóstico MÍNIMO/incremental — isola qual PEÇA exata da cadeia
    // GetPlayerSystem/EquipmentSystem.GetInstance introduz o crash quando a função externa (chamada
    // via nosso `call_func`) retorna `ref<T>` em vez de `Void`. `rettest` (só Print+null) já passou.
    #[cfg(feature = "devtools")]
    let rettest_fn: Option<&str> = match cmd {
        "rettest2" => Some("BwmsTestRef2"),
        "rettest3" => Some("BwmsTestRef3"),
        "rettest4" => Some("BwmsTestRef4"),
        "rettest5" => Some("BwmsTestRef5"),
        "rettest6" => Some("BwmsPrepEquipSys"),
        "rettest7" => Some("BwmsPrepEquipReq"),
        _ => None,
    };
    #[cfg(feature = "devtools")]
    if cmd == "rettest" || rettest_fn.is_some() {
        let fname = rettest_fn.unwrap_or("BwmsTestReturnsRef");
        unsafe {
            if player.is_null() {
                crate::log_dev!("[{cmd}] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[{cmd}] PlayerPuppet.GetGame falhou"); return; }
            };
            let f = crate::register::get_function(reg, fname);
            if !crate::rtti::sane(f) {
                crate::log_dev!("[{cmd}] {fname} não resolveu (mod compilado?)");
                return;
            }
            crate::log_dev!("[{cmd}] resolvido f={f:p}, chamando {fname} agora...");
            let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
            let r = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[{cmd}] call_func retornou: {:?} — zero crash até aqui!", r.is_some());
            if let Some(r) = &r {
                crate::rtti::soltar_retorno(&rf, r);
            }
        }
        return;
    }
    // Garment/transmog (2026-08-03, /goal): 4ª via — 1 SÓ call_func top-level
    // (Void), 2 natives aninhadas de 1-handle-arg cada capturam es/req via atomic; Rust lê depois e
    // faz o transmute pro endereço cru. Evita os 2 bugs diagnosticados nesta madrugada (cont.17/19).
    // Garment/transmog (2026-08-03, /goal): instala a sonda naked observe-only
    // em 0x102445ee0 (dentro de QueueRequest, ANTES do dispatch por tabela runtime). Rodar ESTE
    // comando ANTES de `equiprawv4` — loga x0/x1/x2/x3/x8 + buf16 antes/depois, sem mudar
    // comportamento nenhum (a chamada real acontece igual).
    #[cfg(feature = "devtools")]
    if cmd == "qrprobeon" {
        unsafe { crate::selftest::install_qr_dispatch_probe() };
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv3" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv3] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[equiprawv3] PlayerPuppet.GetGame falhou"); return; }
            };
            diagnostico::CAPTURED_ES_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
            diagnostico::CAPTURED_REQ_PTR.store(0, std::sync::atomic::Ordering::Relaxed);
            let f = crate::register::get_function(reg, "BwmsPrepAndCapture");
            if !crate::rtti::sane(f) {
                crate::log_dev!("[equiprawv3] BwmsPrepAndCapture não resolveu (mod compilado?)");
                return;
            }
            crate::log_dev!("[equiprawv3] chamando BwmsPrepAndCapture (1 única call_func top-level, Void)...");
            let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
            let r = crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv3] call_func retornou: {:?} — zero crash até aqui", r.is_some());
            let es_ptr = diagnostico::CAPTURED_ES_PTR.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
            let req_ptr = diagnostico::CAPTURED_REQ_PTR.load(std::sync::atomic::Ordering::Relaxed) as *mut c_void;
            crate::log_dev!("[equiprawv3] es_ptr={es_ptr:p} req_ptr={req_ptr:p} (via atomics, sem 2ª call_func)");
            if es_ptr.is_null() || req_ptr.is_null() || !crate::rtti::sane(es_ptr) || !crate::rtti::sane(req_ptr) {
                crate::log_dev!("[equiprawv3] es_ptr/req_ptr inválido — abortando antes do transmute");
                return;
            }
            crate::log_dev!("[equiprawv3] chamando 0x102445e8c direto via transmute...");
            let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(0x1_0244_5e8cu64));
            tf(es_ptr, req_ptr);
            crate::log_dev!("[equiprawv3] transmute retornou — ZERO CRASH, via completa!");
        }
        return;
    }
    // Garment/transmog (2026-08-03, /goal): 5ª via — achado decisivo (cont.21):
    // o crash é especificamente `new T()` via `call_func` (bytecode) quando não é uma das 1ªs
    // alocações da sessão. `rtti::new_object` JÁ EXISTE (replica CClass::CreateInstance 100% em
    // Rust, usado pra outras coisas há sessões) — constrói `req` SEM bytecode nenhum, evitando o
    // mecanismo problemático por completo. `es` continua via `BwmsPrepEquipSys` (call_func, mas SEM
    // `new`, já provado seguro repetidas vezes hoje). Campos escritos via `resolve_prop_in_class`
    // (offset real, não chute) — `itemID`/`slotIndex`/`addToInventory` (EquipRequest) +
    // `owner` (PlayerScriptableSystemRequest, herdado).
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv4" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv4] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[equiprawv4] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = crate::register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                crate::log_dev!("[equiprawv4] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ptr = crate::rtti::call_ptr(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv4] es_ptr={es_ptr:p} (via BwmsPrepEquipSys, call_func mas sem `new`)");
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                crate::log_dev!("[equiprawv4] es_ptr inválido — abortando");
                return;
            }
            // req: construído 100% em Rust via rtti::new_object (zero bytecode `new`).
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            crate::log_dev!("[equiprawv4] req_ptr={req_ptr:p} (via rtti::new_object, ZERO bytecode new)");
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                crate::log_dev!("[equiprawv4] req_ptr inválido — abortando");
                return;
            }
            // FIX DECISIVO #3 v2 (2026-08-03, achado por 2ª RE offline dedicada, corrige a v1 de hoje):
            // `rtti::new_object` chama `Construct()` via vtable mas NUNCA constrói o `WeakHandle<ISerializable>`
            // auto-referente que mora DENTRO do próprio objeto, em `req+0x08..+0x17` (`ISerializable::ref`).
            // A v1 do fix (chamar `make_handle` numa saída SEPARADA/throwaway) não tinha efeito nenhum —
            // Handle_ctor escreve no `out` que você dá a ele, e uma saída throwaway nunca toca a memória
            // real do objeto. Confirmado por disasm do handler de dispatch real (índice 24, 0x1012012dc):
            // ele lê `req+0x10` (metade "refcount-block" do par) esperando um bloco real; com {0,0} o
            // valor colapsa pra um inteiro pequeno lido de outro lugar, que acaba virando o ponteiro cru
            // 0x2b deref'd no crash (slot 34 da vtable de EquipmentSystem, `ldr x8,[x0]`). Fix certo:
            // Handle_ctor com a saída APONTANDO DIRETO pra req_ptr+0x08 (auto-referência real).
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            crate::log_dev!("[equiprawv4] req+0x08 (ISerializable::ref) construído via make_handle DIRETO no objeto (fix v2)");
            let cls = reg.class_by_name("gameEquipRequest");
            // itemID: ItemID (16 bytes), via from_tdbid (já existe, mesma via de Skill 2/equiponce).
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            // owner: wref<GameObject> — FIX #2 (mesma RE): {player, NULL} cru não é um WeakHandle
            // válido pro motor (falta o refcount-block real); constrói via make_handle (Handle_ctor)
            // igual ao req_ptr acima, mesma técnica.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            // addToInventory: Bool (1 byte) = true.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            // slotIndex: Int32 = -1.
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            // FIX DECISIVO (2026-08-03, achado por RE dedicada offline): `ScriptableSystemRequest+0x40`
            // é um campo C++ interno NÃO-refletido (8 bytes em `+0x40`) — QueueRequest LÊ
            // esse campo (nunca escreve) esperando um ponteiro cru de volta pro `ScriptableSystem`/
            // `EquipmentSystem` dono (o mesmo `this`), usado internamente pra enfileirar no ring buffer
            // lock-free do próprio sistema (`CircularBuffer<THandle<ScriptableSystemRequest>>`). Nem
            // `new EquipRequest()` em redscript nem `Construct()` genérico via vtable populam isso —
            // fica null, e QueueRequest crasha lendo através dele (`+0x40..+0x5c` relativo). Fix: 1
            // escrita de 8 bytes, offset fixo (não resolvível via RTTI — não é propriedade refletida).
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            crate::log_dev!("[equiprawv4] req+0x40={es_ptr:p} (fix do campo interno não-refletido) — req montado, chamando 0x102445e8c direto...");
            let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(0x1_0244_5e8cu64));
            tf(es_ptr, req_ptr);
            crate::log_dev!("[equiprawv4] transmute retornou — ZERO CRASH, via completa!");
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv5" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv5] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[equiprawv5] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = crate::register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                crate::log_dev!("[equiprawv5] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ptr = crate::rtti::call_ptr(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv5] es_ptr={es_ptr:p}");
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                crate::log_dev!("[equiprawv5] es_ptr inválido — abortando");
                return;
            }
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            crate::log_dev!("[equiprawv5] req_ptr={req_ptr:p}");
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                crate::log_dev!("[equiprawv5] req_ptr inválido — abortando");
                return;
            }
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            let cls = reg.class_by_name("gameEquipRequest");
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            if let Some((voff, _, _)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            crate::log_dev!("[equiprawv5] req montado (mesmo setup do v4) — construindo CScriptStackFrame de verdade...");

            // FIX DECISIVO #4 (2026-08-03, achado por RE dedicada + sonda ao vivo `qrprobeon`):
            // `0x102445e8c` (QueueRequest) NÃO recebe `req` cru como 2º argumento — ele lê
            // `x1[0]` (frame->code) como CURSOR DE OPCODE da VM redscript, indexando a MESMA
            // OPCODE_TABLE (0x109a5c940) que `rtti::call_func` já usa pra ler argumentos. Os
            // crashes anteriores (0x6b/0x2b/0xEB, variando por boot) eram sempre bytes CRUS do
            // vtable/heap de `req` reinterpretados como opcode — nunca foi um campo faltando em
            // `req`, é o TIPO ERRADO de 2º argumento. `rtti::call_func` normal não serve aqui:
            // ela despacha via `crate::jogo::scripts::EXECUTOR_VM`, que SEMPRE resolve a função via `GetInvokable()`
            // antes de rodar — e `GetInvokable()` é um stub que retorna null incondicional pra
            // QueueRequest (RE-esgotado, ver HISTORICO cont.14/16 desta mesma madrugada). Fix:
            // construir o frame NÓS MESMOS (mesma receita exata de `call_func`: LocalVar+ParamEnd,
            // CProperty sintética via GetType("handle:IScriptable"), Handle no local) e chamar
            // `0x102445e8c` DIRETO com esse frame como x1 — bypassa crate::jogo::scripts::EXECUTOR_VM/GetInvokable por
            // completo, mas dá a QueueRequest o formato de argumento que ela realmente espera.
            let itype = crate::register::get_type(reg, "handle:IScriptable");
            if itype.is_null() {
                crate::log_dev!("[equiprawv5] GetType(handle:IScriptable) falhou — abortando");
                return;
            }
            let mut cprop = vec![0u8; 0x30];
            (cprop.as_mut_ptr() as *mut u64).write_unaligned(itype as u64); // CProperty+0 = IType*

            let mut locals = vec![0u8; 0x40 + 0x20]; // n=1 arg, mesma fórmula de call_func
            let dst = locals.as_mut_ptr().add(0x20);
            (dst as *mut *mut c_void).write_unaligned(req_ptr);
            (dst.add(8) as *mut *mut c_void).write_unaligned(crate::rtti::refcnt());

            let mut bc = vec![0u8; 16 + 1 * 9]; // mesma fórmula de call_func (n=1)
            bc[0] = 0x18; // LocalVar
            (bc.as_mut_ptr().add(1) as *mut *mut c_void).write_unaligned(cprop.as_mut_ptr() as *mut c_void);
            bc[9] = 0x26; // ParamEnd

            let mut fr = crate::rtti::frame_raiz(bc.as_mut_ptr(), locals.as_mut_ptr(), es_ptr);

            crate::log_dev!(
                "[equiprawv5] frame montado (fr={:p} bc={:p} locals={:p} cprop={:p} itype={:p}) — chamando 0x102445e8c(es_ptr, &frame)...",
                fr.as_ptr(), bc.as_ptr(), locals.as_ptr(), cprop.as_ptr(), itype
            );
            let tf: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(0x1_0244_5e8cu64));
            tf(es_ptr, fr.as_mut_ptr() as *mut c_void);
            crate::log_dev!("[equiprawv5] transmute retornou — ZERO CRASH, via completa (frame real)!");
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv6" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv6] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[equiprawv6] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = crate::register::get_function(reg, "BwmsPrepEquipSys");
            if !crate::rtti::sane(es_fn) {
                crate::log_dev!("[equiprawv6] BwmsPrepEquipSys não resolveu");
                return;
            }
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let es_ptr = crate::rtti::call_ptr(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv6] es_ptr={es_ptr:p}");
            if es_ptr.is_null() || !crate::rtti::sane(es_ptr) {
                crate::log_dev!("[equiprawv6] es_ptr inválido — abortando");
                return;
            }
            let req_ptr = crate::rtti::new_object(reg, "gameEquipRequest");
            crate::log_dev!("[equiprawv6] req_ptr={req_ptr:p}");
            if req_ptr.is_null() || !crate::rtti::sane(req_ptr) {
                crate::log_dev!("[equiprawv6] req_ptr inválido — abortando");
                return;
            }
            crate::rtti::make_handle((req_ptr as *mut u8).add(0x08) as *mut c_void, req_ptr);
            let cls = reg.class_by_name("gameEquipRequest");
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "itemID") {
                crate::log_dev!("[equiprawv6-diag] itemID voff={voff:#x} in_holder={in_holder}");
                if let Some(item16) = crate::rtti::from_tdbid(reg, "Items.Fixer_01_Set_TShirt") {
                    std::ptr::copy_nonoverlapping(item16.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
                }
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "owner") {
                crate::log_dev!("[equiprawv6-diag] owner voff={voff:#x} in_holder={in_holder}");
                let mut owner_pair = [0u8; 16];
                crate::rtti::make_handle(owner_pair.as_mut_ptr() as *mut c_void, player);
                std::ptr::copy_nonoverlapping(owner_pair.as_ptr(), (req_ptr as *mut u8).add(voff as usize), 16);
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "addToInventory") {
                crate::log_dev!("[equiprawv6-diag] addToInventory voff={voff:#x} in_holder={in_holder}");
                *(req_ptr as *mut u8).add(voff as usize) = 1u8;
            }
            if let Some((voff, _, in_holder)) = crate::rtti::resolve_prop_in_class(cls, "slotIndex") {
                crate::log_dev!("[equiprawv6-diag] slotIndex voff={voff:#x} in_holder={in_holder}");
                ((req_ptr as *mut u8).add(voff as usize) as *mut i32).write_unaligned(-1);
            }
            (req_ptr as *mut u8).add(0x40).cast::<*mut c_void>().write_unaligned(es_ptr);
            // DIAGNÓSTICO: dump dos 0x50 primeiros bytes de req_ptr logo antes da chamada, pra
            // conferir visualmente se os campos foram escritos no lugar certo (comparado ao layout
            // do header: vtable@0, ISerializable::ref@0x08, unk18@0x18, unk28@0x28(4B), nativeType
            // @0x30, valueHolder@0x38, unk40(es_ptr)@0x40).
            let dump: &[u8] = std::slice::from_raw_parts(req_ptr as *const u8, 0x80);
            crate::log_dev!("[equiprawv6-diag] req bytes[0..0x80]={:02x?}", dump);
            crate::log_dev!("[equiprawv6] req montado (mesmo setup do v4/v5) — fix cirúrgico de GetInvokable...");

            // FIX DECISIVO #5 (2026-08-03): equiprawv5 provou que chamar `0x102445e8c` DIRETO com
            // um frame à mão roda sem crash mas NÃO faz nada (nosso bytecode sintético só declara
            // 1 arg via LocalVar+ParamEnd — não há instrução real de corpo depois disso, então o
            // executor retorna sem invocar a lógica nativa de enfileiramento). A via CERTA é deixar
            // o `crate::jogo::scripts::EXECUTOR_VM` de sempre (o mesmo que todo `rtti::call_func` já usa com sucesso pra
            // dezenas de outras nativas) fazer TUDO — arg-parsing E dispatch pro código nativo real
            // — mas ele SÓ despacha depois de chamar `this->GetInvokable()`, que é um stub que
            // SEMPRE retorna null pra QueueRequest (RE já feita, `0x1008c5c9c`, cont.14 desta
            // madrugada — causa-raiz original do crash antes de qualquer workaround). Fix: hookar
            // `GetInvokable()` cirurgicamente (só pro descritor exato de QueueRequest, zero
            // regressão nos outros usos) pra devolver `0x102445e8c` em vez de null, e então usar
            // `rtti::call_func` NORMAL (a via já provada, não mais um transmute cru nosso).
            let rf_qr = match crate::rtti::resolve_func(reg, "EquipmentSystem", "QueueRequest") {
                Some(rf) => rf,
                None => { crate::log_dev!("[equiprawv6] resolve_func(EquipmentSystem, QueueRequest) falhou"); return; }
            };
            crate::log_dev!("[equiprawv6] rf_qr.func={:p} — instalando fix de GetInvokable...", rf_qr.func);
            if !crate::roupas_e_armas::install_getinvokable_fix(rf_qr.func, 0x1_0244_5e8cu64) {
                crate::log_dev!("[equiprawv6] install_getinvokable_fix falhou — abortando");
                return;
            }
            crate::log_dev!("[equiprawv6] fix instalado — chamando via rtti::call_func NORMAL (crate::jogo::scripts::EXECUTOR_VM)...");
            let ret = crate::rtti::call_func(&rf_qr, es_ptr, &[crate::rtti::Arg::Handle(req_ptr, crate::rtti::refcnt())]);
            crate::log_dev!("[equiprawv6] call_func retornou={:?} — ZERO CRASH, via crate::jogo::scripts::EXECUTOR_VM completa!", ret.is_some());
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "equiprawv2" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[equiprawv2] player null");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            let gi = match gi {
                Some(gi) => gi,
                None => { crate::log_dev!("[equiprawv2] PlayerPuppet.GetGame falhou"); return; }
            };
            let es_fn = crate::register::get_function(reg, "BwmsPrepEquipSys");
            let req_fn = crate::register::get_function(reg, "BwmsPrepEquipReq");
            if !crate::rtti::sane(es_fn) || !crate::rtti::sane(req_fn) {
                crate::log_dev!("[equiprawv2] BwmsPrepEquipSys/Req não resolveram (mod compilado?)");
                return;
            }
            // FIX 2026-08-03 (achado ao vivo, 3 crashes): ret_type NÃO pode ser hardcoded null pra
            // funções que retornam ref<T> (só é correto pra Void) — crate::jogo::scripts::EXECUTOR_VM usa esse valor
            // DIRETO, um ret_type errado corrompe o processamento da chamada inteira, não só o
            // retorno. Ver rtti::ret_type_of.
            let es_rf = crate::rtti::ResolvedFn { func: es_fn, ret_type: crate::rtti::ret_type_of(es_fn), is_static: true };
            let req_rf = crate::rtti::ResolvedFn { func: req_fn, ret_type: crate::rtti::ret_type_of(req_fn), is_static: true };
            crate::log_dev!("[equiprawv2] pré: chamando es_rf (1ª call_func desta invocação)...");
            let es_ptr = crate::rtti::call_ptr(&es_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv2] es_ptr={es_ptr:p} — chamando req_rf agora (2ª call_func)...");
            // O pedido é objeto novo e fica com o +1: sem saber se o `0x102445e8c` guarda referência dele, soltar poderia destruí-lo na fila.
            let req_ret = crate::rtti::call_func(&req_rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]);
            crate::log_dev!("[equiprawv2] req_ret={:?}", req_ret.is_some());
            let req_ptr = req_ret.map(|r| u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]) as *mut c_void).unwrap_or(std::ptr::null_mut());
            crate::log_dev!("[equiprawv2] es_ptr={es_ptr:p} req_ptr={req_ptr:p} — 2 chamadas top-level, zero native aninhado");
            if es_ptr.is_null() || req_ptr.is_null() || !crate::rtti::sane(es_ptr) || !crate::rtti::sane(req_ptr) {
                crate::log_dev!("[equiprawv2] es_ptr/req_ptr inválido — abortando antes do transmute");
                return;
            }
            crate::log_dev!("[equiprawv2] chamando 0x102445e8c direto via transmute, DIRETO de run_cmd (zero aninhamento)");
            let f: extern "C" fn(*mut c_void, *mut c_void) = std::mem::transmute(crate::rebase(0x1_0244_5e8cu64));
            f(es_ptr, req_ptr);
            crate::log_dev!("[equiprawv2] transmute retornou — zero crash até aqui!");
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "onshutdowntest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[onshutdown-test] player null — sem save carregado?");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsOnShutdownTest");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            crate::log_dev!("[onshutdown-test] BwmsOnShutdownTest(game) chamado — registrado em Session/End");
                        } else {
                            crate::log_dev!("[onshutdown-test] call_func de BwmsOnShutdownTest não completou");
                        }
                    } else {
                        crate::log_dev!("[onshutdown-test] BwmsOnShutdownTest não resolveu (mod compilado?)");
                    }
                }
                None => crate::log_dev!("[onshutdown-test] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "3ptest" {
        unsafe {
            if player.is_null() {
                crate::log_dev!("[bwms-3p-test] player null — sem save carregado?");
                return;
            }
            let gi: Option<[u8; 16]> = crate::jogo::sessao::game_instance(reg, player);
            match gi {
                Some(gi) => {
                    let f = crate::register::get_function(reg, "BwmsTestThirdPartyCheat3p");
                    if crate::rtti::sane(f) {
                        let rf = crate::rtti::ResolvedFn { func: f, ret_type: std::ptr::null_mut(), is_static: true };
                        if crate::rtti::call_func(&rf, std::ptr::null_mut(), &[crate::rtti::Arg::Raw(gi)]).is_some() {
                            crate::log_dev!("[bwms-3p-test] BwmsTestThirdPartyCheat3p(game) chamado com GameInstance real");
                        } else {
                            crate::log_dev!("[bwms-3p-test] call_func de BwmsTestThirdPartyCheat3p não completou");
                        }
                    } else {
                        crate::log_dev!("[bwms-3p-test] BwmsTestThirdPartyCheat3p não resolveu (mod externo compilado?)");
                    }
                }
                None => crate::log_dev!("[bwms-3p-test] PlayerPuppet.GetGame falhou"),
            }
        }
        return;
    }
    // Factories: dump read-only do FactoryIndex (aIndex), pra achar o offset REAL do
    // registry pós-async. Ver `selftest::dump_factory_index`.
    #[cfg(feature = "devtools")]
    if cmd == "factdump" {
        unsafe { crate::selftest::dump_factory_index() };
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "postload-probe" {
        unsafe { crate::selftest::postload_probe(&reg) };
        return;
    }
    #[cfg(feature = "devtools")]
    if cmd == "luaganchosprova" {
        unsafe {
            match status_do_personagem::hasgod(reg, player) {
                Some(real_before) => {
                    let hits_before = inicio::boot::RUST_OBS_HITS.load(Ordering::Relaxed);
                    // (a) OBSERVE: arma, chama 1x, desarma — original deve rodar (retorno == estado real).
                    inicio::boot::RUST_OBS_CNAME.store(crate::cname::cname("HasGodMode"), Ordering::Relaxed);
                    let observed = status_do_personagem::hasgod(reg, player);
                    inicio::boot::RUST_OBS_CNAME.store(0, Ordering::Relaxed);
                    let hits_after = inicio::boot::RUST_OBS_HITS.load(Ordering::Relaxed);
                    let observe_ok = hits_after > hits_before && observed == Some(real_before);

                    // (b) OVERRIDE: arma pro valor OPOSTO do estado real (prova válida seja
                    // qual for o baseline do ambiente — forçar pro MESMO valor que já é real não
                    // provaria rewrite nenhum) — chama 1x, retorno deve vir o oposto forçado.
                    let ov_target = !real_before;
                    inicio::boot::RUST_OV_CNAME.store(crate::cname::cname("HasGodMode"), Ordering::Relaxed);
                    inicio::boot::RUST_OV_VAL.store(ov_target as i64, Ordering::Relaxed);
                    let overridden = status_do_personagem::hasgod(reg, player);

                    // (c) SUPPRESS: desarma, chama de novo — volta ao estado real (nada ficou mutado).
                    inicio::boot::RUST_OV_CNAME.store(0, Ordering::Relaxed);
                    let real_after = status_do_personagem::hasgod(reg, player);

                    let override_ok = overridden == Some(ov_target);
                    let suppress_ok = real_after == Some(real_before);
                    let verdict = if observe_ok && override_ok && suppress_ok {
                        ">>> GANCHOS-NAO-LUA OK: Observe (cb disparou + original rodou) + Override (retorno reescrito) + Suppress (estado real preservado), tudo não-Lua em método REAL <<<"
                    } else {
                        "FALHA/verificar: alguma perna não bateu"
                    };
                    crate::log_dev!(
                        "[luaganchosprova] real_antes={real_before} | observe: hits {hits_before}->{hits_after} retorno={observed:?} ok={observe_ok} | override(alvo={ov_target}): retorno={overridden:?} ok={override_ok} | suppress: real_depois={real_after:?} ok={suppress_ok} | {verdict}"
                    );
                }
                None => crate::log_dev!("[luaganchosprova] hasgod() falhou antes de testar (player/sys indisponível)"),
            }
        }
        return;
    }
    let parts: Vec<&str> = cmd.split_whitespace().collect();
    // Pontos de desenvolvimento (PlayerDevelopmentData) — não retornam buffer.
    // Comandos de jogador respondem pela aba Console (`resposta!`): no build público é o único canal que aparece.
    let act = |label: &str, ok: bool| {
        if ok {
            crate::resposta!("'{cmd}': {label}");
        } else {
            crate::resposta!("'{cmd}' falhou: precisa do V em gameplay. Tente de novo com o jogo carregado, fora do menu.");
        }
    };
    let pts = |member: &str, n: &str| {
        let q = n.parse().unwrap_or(1);
        act("enviado", unsafe { status_do_personagem::add_points(reg, player, q, member) });
    };
    let ajuda = |assunto: &str| {
        for linha in crate::terminal::catalogo::ajuda(assunto, TOTAL_DE_COMANDOS).lines() {
            crate::resposta!("{linha}");
        }
    };
    match parts.as_slice() {
        // Primeiro comando do terminal: comando sem índice é o mesmo que nenhum.
        ["ajuda"] | ["help"] | ["?"] => return ajuda(""),
        ["ajuda", assunto] | ["help", assunto] => return ajuda(assunto),
        ["attrs", n] => return pts("Attribute", n),
        ["perks", n] => return pts("Primary", n),
        ["relic", n] => return pts("Espionage", n),
        ["level", n] => {
            return act("enviado", unsafe { status_do_personagem::level(reg, player, n.parse().unwrap_or(1)) })
        }
        ["godmode"] | ["god"] => return act("ON", unsafe { status_do_personagem::godmode(reg, player, true) }),
        ["godmode", "off"] | ["god", "off"] => {
            return act("OFF", unsafe { status_do_personagem::godmode(reg, player, false) })
        }
        // READ-ONLY: `redscript-cheat-effects-proof` — checa HasGodMode sem mutar (ver status_do_personagem::hasgod).
        ["hasgod"] => {
            return match unsafe { status_do_personagem::hasgod(reg, player) } {
                Some(true) => crate::resposta!("God Mode: ligado"),
                Some(false) => crate::resposta!("God Mode: desligado"),
                None => crate::resposta!("Não consegui ler o God Mode: precisa do V em gameplay. Tente de novo com o jogo carregado."),
            }
        }
        ["heal"] => return act("curado", unsafe { status_do_personagem::heal(reg, player) }),
        // O valor lido sai só no log de dev (dentro do staminacheck); no público não há o que mostrar.
        #[cfg(feature = "devtools")]
        ["staminacheck"] => {
            let _lido = unsafe { status_do_personagem::staminacheck(reg, player) };
            return crate::log_dev!("[console] '{cmd}' -> {}", if _lido { "lido" } else { "FALHOU" });
        }
        ["summon"] | ["car"] => return act("enviado", unsafe { veiculos::summon(reg, player) }),
        // Registro nativo (rodar no jogo p/ destravar a fundação):
        // cwprobe = despeja o layout de uma função nativa real (o handler não mora nela: ver o
        // doc do módulo `register`); cwreg = smoke-test (registra BlackwallPing global); cwfacade =
        // registra `Version`/`Require` da fachada de versão (precisa do .reds da fachada).
        #[cfg(feature = "devtools")]
        ["cwprobe"] => return log(&unsafe { diagnostico::probe(reg) }),
        #[cfg(feature = "devtools")]
        ["cwreg"] => return log(&unsafe { diagnostico::register_smoke(reg) }),
        #[cfg(feature = "devtools")]
        ["cwfacade"] => return log(&unsafe { tipos_do_motor::registrar_fachada_de_versao(reg) }),
        // RegisterType STEP-1: forja+registra uma CClass mínima (register-sem-instanciar). Confirmar
        // com `rttidump <nome>` depois. Ex.: `cwregtype BwmsTestClass` -> `rttidump BwmsTestClass`.
        #[cfg(feature = "devtools")]
        ["cwregtype", name] => {
            unsafe { crate::register::register_type_min(reg, name) };
            return;
        }
        // RegisterType STEP-2: forja uma classe INSTANCIÁVEL (alias fiel de <src>: size/vtable reais).
        // Ex.: `cwregalias BwmsAliasV PlayerPuppet` -> `newobj BwmsAliasV` (deve construir sem crash).
        #[cfg(feature = "devtools")]
        ["cwregalias", name, src] => {
            unsafe { diagnostico::register_type_alias(reg, name, src) };
            return;
        }
        // RegisterType STEP-3: forja classe com PROPRIEDADE Float custom. Confirmar com `propdump <novo>`.
        // Ex.: `cwregprop BwmsPropC gameuiInGameMenuGameController myFloat` -> `propdump BwmsPropC`.
        #[cfg(feature = "devtools")]
        ["cwregprop", name, src, prop] => {
            unsafe { diagnostico::register_type_with_prop(reg, name, src, prop) };
            return;
        }
        // Censo de hooks observe-only (2026-09-11, `censo.rs`, gate `~/.bwms-hook-censo`): contagem e
        // primeiras amostras de registradores de cada alvo achado offline.
        #[cfg(feature = "devtools")]
        ["censo"] => {
            crate::diagnostico::censo::relatorio();
            return;
        }
        // `roupas_e_armas/terceira_pessoa.rs` (gate `~/.bwms-tpp-slots`): tamanhos dos arrays do
        // TPPRepresentationComponent e ida-e-volta do RegisterAffectedItem num slot vanilla.
        #[cfg(feature = "devtools")]
        ["tppafetados"] => { unsafe { crate::roupas_e_armas::terceira_pessoa::relatorio() }; return; }
        #[cfg(feature = "devtools")]
        ["tppregteste", slot] => { unsafe { crate::roupas_e_armas::terceira_pessoa::registrar_teste(slot) }; return; }
        #[cfg(feature = "devtools")]
        ["tppdesregteste", slot] => { unsafe { crate::roupas_e_armas::terceira_pessoa::desregistrar_teste(slot) }; return; }
        #[cfg(feature = "devtools")]
        ["tppequipteste", slot] => { unsafe { crate::roupas_e_armas::terceira_pessoa::handler_teste(slot, true) }; return; }
        #[cfg(feature = "devtools")]
        ["tppunequipteste", slot] => { unsafe { crate::roupas_e_armas::terceira_pessoa::handler_teste(slot, false) }; return; }
        #[cfg(feature = "devtools")]
        ["tppanexarteste"] => { unsafe { crate::roupas_e_armas::terceira_pessoa::anexar_teste() }; return; }
        // `mkflatnovo <flat> <doador> <hex>` (2026-09-11): flat que o record não tem, qualquer largura.
        #[cfg(feature = "devtools")]
        ["mkflatnovo", flat, donor, hex] => { unsafe { crate::tweakdb::mkflatnovo_cmd(flat, donor, hex) }; return; }
        // Captura DINÂMICA de LR em `InkSystem::Get()` (2026-08-14).
        // `inkgethook` instala o probe observe-only (gate `~/.bwms-hook-inkget-lr`); rode
        // `inkgetbaseline` em idle (~10-15s após o hook), depois `presskey <kc>` 3-5x, depois
        // `inkgetpost` — loga os call-sites que só aparecem depois do keypress.
        //
        // TIMING (fix desta sessão, mesma lição corrigida do `checkreshook`/`#198`/`#199`
        // abaixo): mande `inkgethook` IMEDIATAMENTE ao confirmar GAMEPLAY, durante a própria
        // rajada de smoke-tests do `OnGameAttached` (enquanto o executor ainda dispara) — a 5ª
        // tentativa desta linha confirmou `inkgethook`/`inkgetbaseline` funcionando quando
        // enviados assim, mas `presskey`+`inkgetpost` (mandados ~20-27s depois) nunca foram
        // processados: o executor já tinha parado de disparar (mesmo mecanismo do
        // `checkresbaseline`/`checkrespost`, canal MORTO fora da rajada mesmo com
        // `PHASE_REACHED_5=true`).
        // `inkgethook` (este braço) continua existindo como comando MANUAL do canal do executor
        // (útil pra reinstalar sob demanda/depurar), mas desde a nova tentativa automática do inkGet (2026-08-16,
        // `on_load`, thread dedicada logo após a thread de heartbeat) NÃO é mais o único jeito de
        // instalar o probe: o dylib agora tenta sozinho, repetidamente (750ms, gate
        // `~/.bwms-hook-inkget-lr`), independente do executor estar vivo ou não — resolve
        // exatamente a fragilidade de timing descrita no parágrafo acima (janela do executor
        // curta e imprevisível). Este braço manual fica como fallback/atalho, nunca conflita com
        // o retry automático (mesma trava `INKGET_INSTALLED`, idempotente dos dois lados).
        //
        // `presskey`/`focusgame` (braços `["presskey", kc]`/`["focusgame"]` mais abaixo, neste
        // `run_cmd`) DELIBERADAMENTE NÃO migram pro canal do heartbeat: `presskey`
        // INJETA um evento de input REAL (CGEvent keyDown/keyUp) que tem efeito genuíno no jogo
        // vivo — categoria de risco diferente (mutação, não leitura). Rodar via heartbeat
        // tiraria a garantia do gate `PHASE_REACHED_5 && !exec_nested()` (o jogo poderia não
        // estar pronto pra receber input ainda) e chamaria `NSApplication.
        // activateIgnoringOtherApps:`/`CGEventPost` de uma thread Rust própria que nunca foi
        // confirmada como sendo a main-thread do processo (AppKit não garante thread-safety fora
        // dela) — risco novo, nunca testado, mesmo padrão de cautela do `checkreshook`.
        //
        // LIÇÃO DE TESTE (2026-08-14): o canal do
        // heartbeat (`[hb-canal]`, laço pós-boot da thread do heartbeat no `on_load`) lê o
        // arquivo INTEIRO e faz `split_whitespace()` sobre o CONTEÚDO TODO como se fosse UM comando só (`parts.as_slice()` == token array) — ao
        // contrário do canal do executor (`cp77_tick`), que faz `content.lines()` e despacha
        // CADA linha separada via `run_cmd`. Um arquivo com múltiplos comandos por linha (ex.
        // `focusgame\ninkgethook\ninkgetbaseline\n`) NUNCA bate em nenhum braço de match do
        // heartbeat (vira um array de 3 tokens, não `["focusgame"]`) — fica só esperando o
        // executor (que pode já estar morto). **Pra confiar no canal do heartbeat, sempre
        // escrever/enviar UM comando por vez** (uma escrita = um comando, sem `\n` interno) —
        // nunca bater múltiplos comandos na mesma escrita achando que "funciona igual ao canal
        // do executor". Um lote de 3 comandos (`focusgame`+`inkgethook`+`inkgetbaseline`)
        // mandado junto numa escrita se perdeu sem nenhum dos dois canais processá-lo.
        #[cfg(feature = "devtools")]
        ["inkgethook"] => {
            unsafe { crate::selftest::install_inkget_probe() };
            return;
        }
        #[cfg(feature = "devtools")]
        ["inkgetbaseline"] => {
            crate::selftest::inkget_baseline();
            return;
        }
        #[cfg(feature = "devtools")]
        ["inkgetpost"] => {
            crate::selftest::inkget_post();
            return;
        }
        // Candidato `0x104404720` (2026-08-17, achado por LR-tracing em `Get()` +
        // disassembly offline — ver `HISTORICO.md`).
        // `ink120hook` instala sob demanda (independente do gate de boot `~/.bwms-hook-
        // ink120cand`); `ink120dump` lê o ring (x0/x1/x2/x3 por chamada); `ink120reset` zera pra
        // uma janela nova. Mesmo idioma do trio `inkgethook`/`inkgetbaseline`/`inkgetpost` acima.
        #[cfg(feature = "devtools")]
        ["ink120hook"] => {
            unsafe { crate::selftest::install_ink120cand_probe() };
            return;
        }
        #[cfg(feature = "devtools")]
        ["ink120dump"] => {
            crate::selftest::ink120cand_dump();
            return;
        }
        #[cfg(feature = "devtools")]
        ["ink120reset"] => {
            crate::selftest::ink120cand_reset();
            return;
        }
        // `ResourceLoader::LoadAsync`/`LoadResource` — captura DINÂMICA
        // de LR em `ResourceDepot::CheckResource` (2026-08-14, ancoragem alternativa ao
        // `InkSystem::Get()` do `#120`). `checkreshook` instala o probe observe-only (gate
        // `~/.bwms-hook-checkres-lr`); `CheckResource` dispara sozinho durante loading real —
        // não precisa de `presskey`. NUNCA rodar junto de `copytest` (mesmo endereço, hooks
        // conflitam).
        //
        // TIMING (CORRIGIDO 2026-08-14, 2ª rodada — invalida a lição da 1ª rodada, que dizia
        // "espere uma janela de silêncio pós-gameplay antes de mandar"): esse conselho estava
        // ERRADO. Achado ao vivo: o canal deste bloco só drena dentro de `cp77_tick()`, chamado
        // só como efeito colateral de `exec_replacement` (o hook do executor de script) disparar.
        // O executor PARA de disparar assim que a rajada inicial de smoke-tests do
        // `OnGameAttached` termina e o jogo fica "quieto" — nesse ponto o canal fica MORTO pro
        // resto do boot (`checkreshook` ficou intocado em `/tmp/cp77-cmd.txt` por 35s+ até o
        // fim do boot, mesmo com `PHASE_REACHED_5=true`). A janela de "silêncio" NÃO é o momento
        // certo pra mandar — é exatamente quando o canal para de ser servido. **Mande
        // `checkreshook` IMEDIATAMENTE ao confirmar GAMEPLAY (durante a própria rajada, enquanto
        // o executor ainda está disparando), sem ficar esperando quietude.**
        #[cfg(feature = "devtools")]
        ["checkreshook"] => {
            unsafe { crate::selftest::install_checkres_probe() };
            return;
        }
        #[cfg(feature = "devtools")]
        ["checkresbaseline"] => {
            crate::selftest::checkres_baseline();
            return;
        }
        #[cfg(feature = "devtools")]
        ["checkrespost"] => {
            crate::selftest::checkres_post();
            return;
        }
        #[cfg(feature = "devtools")]
        ["reslinkdump"] => {
            crate::selftest::reslink_dump();
            return;
        }
        #[cfg(feature = "devtools")]
        ["reslink", src, tgt] => {
            let s = u64::from_str_radix(src.trim_start_matches("0x"), 16).unwrap_or(0);
            let t = u64::from_str_radix(tgt.trim_start_matches("0x"), 16).unwrap_or(0);
            crate::selftest::reslink_set(s, t);
            return;
        }
        ["reslinkadd", src, tgt] => {
            let s = u64::from_str_radix(src.trim_start_matches("0x"), 16).unwrap_or(0);
            let t = u64::from_str_radix(tgt.trim_start_matches("0x"), 16).unwrap_or(0);
            crate::arquivos_do_jogo::reslink_add(s, t);
            return;
        }
        ["reslinkpath", src, tgt] => {
            crate::arquivos_do_jogo::reslink_path(src, tgt);
            return;
        }
        // Controller de recurso: arma o watch de UM resource path — quando o jogo constrói esse
        // `ResourcePath` de verdade (via o hook resource.link já instalado), dispara "Resource/
        // Load" (ResourceEvent real) no próximo tick. Ex.: `watchres base\characters\...\x.mesh`.
        #[cfg(feature = "devtools")]
        ["watchres", rest @ ..] => {
            crate::selftest::reslink_watch(&rest.join(" "));
            return;
        }
        ["reslinkfile", rest @ ..] => {
            crate::arquivos_do_jogo::reslink_file(&rest.join(" "), false);
            return;
        }
        #[cfg(feature = "devtools")]
        ["reslinkstat"] => {
            crate::selftest::reslink_stat();
            return;
        }
        #[cfg(feature = "devtools")]
        ["inputlog", "on"] => {
            crate::terminal::overlay::input_log_set(true);
            return crate::log_dev!("[inputlog] ON -> /tmp/cp77-input.log (tecla/mouse/scroll/mod com ts)");
        }
        #[cfg(feature = "devtools")]
        ["inputlog", "off"] => {
            crate::terminal::overlay::input_log_set(false);
            return crate::log_dev!("[inputlog] OFF");
        }
        // Bateria: todas as sondas read-only num comando só (boot é moeda ao ar, ~50%).
        #[cfg(feature = "devtools")]
        ["bateria"] => { unsafe { crate::diagnostico::run_bateria() }; return; }
        // `refsis <SistemaDeScript>` / `refsis getter <GetXxx>` / `refsis metodo <Metodo>` — a rota do sistema devolve handle forte: mede se quem guarda só o ponteiro vaza a referência.
        #[cfg(feature = "devtools")]
        ["refsis", resto @ ..] => { unsafe { crate::diagnostico::run_refsis(reg, player, resto) }; return; }
        // `axl49 [path]` — o veredito do `#49` inteiro, em Rust, numa linha.
        //
        // POR QUE VIROU COMANDO DE CANAL. A prova vivia num smoke redscript disparado em
        // `OnGameAttached` + retentativa por `DelaySystem`. Nesse instante o mundo ainda não
        // streamou, e as 5 execuções desta sessão mediram MENU, não gameplay — "ausência de alvo"
        // logada com cara de negativo do item. Pior: as retentativas só rodam se o jogo sobreviver
        // aos +30s/+75s, e nesta máquina metade dos boots morre antes. Como comando, a medição
        // dispara QUANDO EU QUISER, depois da gameplay confirmada, e não depende do `DelaySystem`.
        //
        // Também não passa por String de RETORNO: o gate antigo era `BwmsSectorClass`, e
        // `worldStreamingSector` tem exatos 20 bytes — o limite do buffer inline da `CString`
        // (ver DATABASE.md) fazia a native devolver "" e o smoke ler isso como "não está no cache".
        #[cfg(feature = "devtools")]
        ["axl49", resto @ ..] => {
            let path: String = resto.first().map(|p| (*p).to_string()).unwrap_or_else(||
                "base\\worlds\\03_night_city\\_compiled\\default\\always_loaded_0.streamingsector".to_string());
            unsafe {
                let esperado = crate::rebase(0x1_01c0_5360) as u64;
                let hash = bwms_primitives::resource_path_hash(&path);
                let sec = crate::register::depot_loaded_resource(hash);
                let classe = crate::register::depot_loaded_resource_class_by_hash(hash);
                let mut slot = 0u64;
                let mut vt_static = 0u64;
                if !sec.is_null() && crate::memoria::is_readable(sec, 8) {
                    let vt = (sec as *const u8).cast::<u64>().read_unaligned() as *mut c_void;
                    if !vt.is_null() && crate::memoria::is_readable(vt, 0x38) {
                        slot = (vt as *const u8).add(0x30).cast::<u64>().read_unaligned();
                        vt_static = crate::un_rebase(vt) as u64;
                    }
                }
                // Controle negativo na MESMA chamada: path inventado tem que dar ponteiro nulo.
                let fake = crate::register::depot_loaded_resource(
                    bwms_primitives::resource_path_hash("base\\bwms\\nao_existe_zzz.streamingsector"));
                crate::log_dev!(
                    "[axl49] path_hash={hash:#x} sec={sec:p} classe='{classe}' vtable(static)={vt_static:#x} \
slot0x30={slot:#x} esperado={esperado:#x} bate={} controle(fake nulo)={}",
                    slot == esperado && slot != 0, fake.is_null());
            }
            return;
        }
        // `axl49scan` — varre o cache do depot em LEITURA PURA procurando entradas cuja vtable
        // tenha, no slot 0x30, o `worldStreamingSector::PostLoad` estático (0x101c05360).
        //
        // A 1ª versão deste comando filtrava por NOME de classe via `class_of` — chamada virtual —
        // e matou o jogo com SIGSEGV, porque o cache guarda entrada antes de a vtable existir. A
        // pilha do `.ips` nomeou `class_of <- depot_resources_by_class <- run_cmd`, então não há
        // dúvida sobre a causa. Aqui não se chama nada: compara-se o VALOR do slot.
        // `slotscan <vmaddr_estatico> [slot_hex]` — GENERALIZAÇÃO do `axl49scan`.
        //
        // O método que provou o alvo do `#49` não tem nada de específico daquele item: dado um
        // endereço estático de função e um slot, ele responde "existe objeto VIVO cuja vtable
        // despacha esse endereço nesse slot?". Cruzado com `re-xref vtable-of <addr>` (que diz em
        // QUAIS vtables do binário o endereço aparece), duas fontes independentes confirmam um alvo
        // de hook sem nunca executá-lo — que é o requisito, porque instalar hook em endereço errado
        // já corrompeu código executável neste projeto em 2026-07-31.
        //
        // Leitura pura, sempre: ver a regra `is_readable` vs CHAMADA no DATABASE.md.
        #[cfg(feature = "devtools")]
        ["slotscan", addr, resto @ ..] => {
            let est = u64::from_str_radix(addr.trim_start_matches("0x"), 16).unwrap_or(0);
            let slot = resto.first()
                .and_then(|x| usize::from_str_radix(x.trim_start_matches("0x"), 16).ok())
                .unwrap_or(0x30);
            if est == 0 || slot > 0x400 {
                crate::log_dev!("[slotscan] uso: slotscan <vmaddr_estatico_hex> [slot_hex, default 30]");
                return;
            }
            unsafe {
                let esperado = crate::rebase(est) as u64;
                let (visitados, com_vtable, batem, achados) =
                    crate::diagnostico::depot_entries_com_slot_em(esperado, slot, 6);
                let amostra: Vec<String> = achados.iter()
                    .map(|(k, vt)| format!("chave={k:#x} vt(static)={vt:#x}"))
                    .collect();
                crate::log_dev!(
                    "[slotscan] alvo(static)={est:#x} slot={slot:#x} visitados={visitados} \
com_vtable_sana={com_vtable} batem={batem} -> {amostra:?}");
            }
            return;
        }
        #[cfg(feature = "devtools")]
        ["axl49scan", ..] => {
            unsafe {
                let esperado = crate::rebase(0x1_01c0_5360) as u64;
                let (visitados, com_vtable, batem, achados) =
                    crate::diagnostico::depot_entries_com_slot_em(esperado, 0x30, 6);
                let amostra: Vec<String> = achados.iter()
                    .map(|(k, vt)| format!("chave={k:#x} vt(static)={vt:#x}"))
                    .collect();
                crate::log_dev!(
                    "[axl49scan] visitados={visitados} com_vtable_sana={com_vtable} \
slot0x30==worldStreamingSector::PostLoad: {batem} -> {amostra:?}");
            }
            return;
        }
        // `depotcab` — o cabeçalho do mapa de recursos vivos por `[PATHB_DEPOT+0xc8]` e por `[loader+0x48]`, e o que o
        // `depot_cache_cabecalho` decide: a prova no jogo de que o leitor acha o mapa com e sem archive na `Mods/`. Leitura pura.
        #[cfg(feature = "devtools")]
        ["depotcab"] => {
            crate::log_dev!("{}", unsafe { crate::arquivos_do_jogo::depot::depotcab() });
            return;
        }
        // `pathbsolta` — o veredito do destrutor da `CString` do caminho na injeção de archive (`montar_no_grupo`). Leitura pura.
        #[cfg(feature = "devtools")]
        ["pathbsolta"] => {
            crate::veredito_dev!("{}", unsafe { crate::arquivos_do_jogo::pathbsolta() });
            return;
        }
        // `resloader` — lê os 3 rings dos probes do `ResourceLoader` (`#406`/`#408`).
        // Gate: `~/.bwms-hook-resloader` (sem ele os shims nem instalam e os rings ficam zerados).
        #[cfg(feature = "devtools")]
        ["resloader"] => { unsafe { crate::selftest::resloader_dump() }; return; }
        // `resultimo` — estado do último objeto devolvido por `LoadAsync(req)` (ver diagnostico/terminal_re.rs).
        #[cfg(feature = "devtools")]
        ["resultimo"] => {
            unsafe { crate::selftest::resloader_ultimo_estado_cmd() };
            return;
        }
        // `resloadreq <path>` — o 3º método (`LoadAsync(const ResourceRequest&)`), construindo a
        // `ResourceRequest` à mão como o motor faz. Cruza o token de `FindToken` com o de `LoadAsync(req)`:
        // num recurso carregado têm que ser idênticos, e é isso (não "não crashou") que valida o layout da struct.
        #[cfg(feature = "devtools")]
        ["resloadreq", p, ..] => {
            unsafe { crate::selftest::resloader_loadreq_cmd(p) };
            return;
        }
        // `resloadfila` (2026-09-11): `LoadAsync(req)` + fila de jobs com `Wait` no handle do objeto (`#408`).
        #[cfg(feature = "devtools")]
        ["resloadfila", p, ..] => {
            unsafe { crate::selftest::resloader_loadreq_fila_cmd(p) };
            return;
        }
        // `resloadref` (2026-09-11): o `LoadAsync` da referência assíncrona do motor (`0x1010f0d34`) + a mesma fila.
        #[cfg(feature = "devtools")]
        ["resloadref", p, ..] => {
            unsafe { crate::selftest::resloader_loadref_fila_cmd(p) };
            return;
        }
        // `resloadreq2`/`resreqdump` (2026-09-11): request do BWMS na entrada que carregou (`0x1010e710c`) e dump da request do motor.
        #[cfg(feature = "devtools")]
        ["resloadreq2", p, ..] => {
            unsafe { crate::selftest::resloader_loadreq2_cmd(p) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["resreqdump", p, ..] => {
            unsafe { crate::selftest::resloader_reqdump_cmd(p) };
            return;
        }
        // `resforget <path>` (2026-09-11): `ResourceBank::ForgetResource` duas vezes (`RemoveResourceFromCache`).
        #[cfg(feature = "devtools")]
        ["resforget", p, ..] => {
            unsafe { crate::selftest::resloader_forget_cmd(p) };
            return;
        }
        // `resfindtoken <path>` — CHAMA `ResourceLoader::FindToken` de verdade (`#408`).
        // Critério discriminante: um asset comprovadamente carregado tem que devolver ponteiro
        // não-nulo, e um path inventado tem que devolver nulo. Se os dois derem igual, não prova.
        #[cfg(feature = "devtools")]
        ["resfindtoken", p, ..] => {
            unsafe { crate::selftest::resloader_findtoken_probe_cmd(p) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["depotres"] => { unsafe { depotres_report(None) }; return; }
        #[cfg(feature = "devtools")]
        ["depotres", alvo, ..] => { unsafe { depotres_report(Some(alvo)) }; return; }
        // Runtime system por índice, a partir da cena capturada.
        // `rtsys` sem argumento mostra os que interessam + o controle; com argumento, um índice do mapa.
        #[cfg(feature = "devtools")]
        ["rtsys"] => {
            unsafe { crate::diagnostico::dump_runtime_systems(); }
            return;
        }
        #[cfg(feature = "devtools")]
        ["rtsys", idx] => {
            unsafe {
                match idx.parse::<usize>() {
                    Ok(i) if !crate::register::runtime_system_no_mapa(i) => crate::log_dev!(
                        "[rtsys] índice {i} não está no mapa da RuntimeScene (cp77-symbols/notes/MAPA-RUNTIMESCENE.md): nada lido. Use um índice da tabela."
                    ),
                    Ok(i) => {
                        let (inst, nome) = crate::register::runtime_system_at(i);
                        crate::log_dev!("[rtsys] [{i}] -> {inst:#x} '{nome}'");
                    }
                    Err(_) => crate::log_dev!("[rtsys] uso: rtsys [<indice>]"),
                }
            }
            return;
        }
        // peekq <vmaddr-hex> [count] — lê N u64 num vmaddr ESTÁTICO (rebase p/ runtime), read-only.
        // Primitiva de RE: resolver ptr de handler global ([0x109cbd808]), func-field de descritor de
        // native, etc. NÃO chama nada (sem crash). count clampado 1..32.
        #[cfg(feature = "devtools")]
        ["peekq", addr, rest @ ..] => {
            let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).clamp(1, 32);
            let va = u64::from_str_radix(addr.trim_start_matches("0x"), 16).unwrap_or(0);
            if va == 0 {
                return crate::log_dev!("[peekq] uso: peekq <vmaddr-hex> [count]");
            }
            unsafe {
                let base = crate::rebase(va) as *const u8;
                if !crate::memoria::is_readable(base as *const c_void, n * 8) {
                    return crate::log_dev!("[peekq] {va:#x} (rt {base:p}) ILEGÍVEL");
                }
                let mut out = format!("[peekq] {va:#x} (rt {base:p}):");
                for i in 0..n {
                    let q = (base.add(i * 8) as *const u64).read();
                    out.push_str(&format!(" +{:#x}={q:#018x}", i * 8));
                }
                return log(&out);
            }
        }
        // vtdump <hex_ptr> [n] — irmão do `peekq`, MAS pra ponteiro RUNTIME já resolvido (ex.:
        // vtable de um objeto vivo, lido via `rd_u64(obj)`) — NÃO faz `rebase()` (que espera um
        // vmaddr ESTÁTICO de arquivo; aplicar rebase de novo num ponteiro já pós-ASLR dava
        // endereço errado). Read-only, nunca chama nada. count clampado 1..48 (cobre vtables
        // maiores, ex. `TweakDB::FlatValue` tem 31 slots).
        #[cfg(feature = "devtools")]
        ["vtdump", addr, rest @ ..] => {
            let n = rest.first().and_then(|s| s.parse::<usize>().ok()).unwrap_or(8).clamp(1, 48);
            let va = u64::from_str_radix(addr.trim_start_matches("0x"), 16).unwrap_or(0);
            if va == 0 {
                return crate::log_dev!("[vtdump] uso: vtdump <ptr-hex-runtime> [count]");
            }
            unsafe {
                let base = va as *const u8;
                if !crate::memoria::is_readable(base as *const c_void, n * 8) {
                    return crate::log_dev!("[vtdump] {va:#x} ILEGÍVEL");
                }
                let mut out = format!("[vtdump] {va:#x}:");
                let mut prev: Option<u64> = None;
                let mut run_start = 0usize;
                for i in 0..n {
                    let q = (base.add(i * 8) as *const u64).read_unaligned();
                    let same = prev == Some(q);
                    out.push_str(&format!(" +{:#x}={q:#018x}{}", i * 8, if same { "=" } else { "" }));
                    if !same {
                        run_start = i;
                    }
                    prev = Some(q);
                    let _ = run_start;
                }
                return log(&out);
            }
        }
        // `getrecords <classe> [limite]` (2026-08-10) — `GetRecords<R>`: lista
        // TODOS os records de um TIPO enumerando `recordsByID` (read-only).
        #[cfg(feature = "devtools")]
        ["getrecords", class] => {
            unsafe { crate::tweakdb::getrecords_cmd(reg, class, 500_000) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["getrecords", class, limit] => {
            let n: usize = limit.parse().unwrap_or(500_000);
            unsafe { crate::tweakdb::getrecords_cmd(reg, class, n) };
            return;
        }
        // `getrecbytype <classe> [limite]` (2026-08-10) — `GetRecordsByType`:
        // lookup DIRETO no índice `recordsByType`@+0x88 (O(1) amortizado), cross-validado contra
        // `getrecords` (scan completo filtrado) — mesma classe deve dar a MESMA contagem.
        #[cfg(feature = "devtools")]
        ["getrecbytype", class] => {
            unsafe { crate::tweakdb::getrecbytype_cmd(reg, class, 500_000) };
            return;
        }
        // `garmentoffsets` (2026-08-11) — `EnableGarmentOffsets`/
        // `DisableGarmentOffsets` da fachada de recursos: diagnóstico read-only do flag (a API não
        // tem getter — só existe pra provar o round-trip Enable/Disable ao vivo).
        #[cfg(feature = "devtools")]
        ["garmentoffsets"] => {
            crate::log_dev!("[garmentoffsets] enabled={}", crate::roupas_e_armas::garment_offsets_enabled());
            return;
        }
        // `innertype <typeName>` (2026-08-10) — `ReflectionType.GetInnerType()`:
        // resolve um IType composto (array/handle/weak-handle) por nome e mostra o tipo interno.
        #[cfg(feature = "devtools")]
        ["innertype", type_name] => {
            unsafe { crate::rtti::probe_inner_type(reg, type_name) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["getrecbytype", class, limit] => {
            let n: usize = limit.parse().unwrap_or(500_000);
            unsafe { crate::tweakdb::getrecbytype_cmd(reg, class, n) };
            return;
        }
        // `createalias <record_id_hex> <alias_id_hex>` (2026-08-10) —
        // `CreateRecordAlias`: cria uma entrada nova em `recordsByID` (via `CreateTDBRecord`,
        // growth-safe) apontando pro MESMO instance do record fonte.
        ["createalias", rid, aid] => {
            unsafe {
                let Some(t) = crate::tweakdb::singleton() else {
                    return crate::log_dev!("[createalias] singleton indisponível");
                };
                let record_id = u64::from_str_radix(rid.trim_start_matches("0x"), 16).unwrap_or(0);
                let alias_id = u64::from_str_radix(aid.trim_start_matches("0x"), 16).unwrap_or(0);
                let ok = crate::tweakdb::create_record_alias_by_id(t, reg, record_id, alias_id);
                crate::log_dev!("[createalias] record={record_id:#x} alias={alias_id:#x} -> {ok}");
            }
            return;
        }
        // `resexists <path>` (2026-08-10): testa `resource_exists`
        // contra um path CONHECIDO (deve existir) e um INVENTADO (não deve).
        #[cfg(feature = "devtools")]
        ["resexists", path] => {
            unsafe {
                let r = resource_exists(path);
                crate::log_dev!("[resexists] '{path}' -> {r}");
            }
            return;
        }
        // `archiveexists <nome>` (2026-08-10): testa `archive_exists_by_name`
        // contra um nome CONHECIDO (deve existir, ex. basegame_1_engine.archive) e um INVENTADO.
        #[cfg(feature = "devtools")]
        ["archiveexists", name] => {
            unsafe {
                let r = archive_exists_by_name(name);
                crate::log_dev!("[archiveexists] '{name}' -> {r}");
            }
            return;
        }
        #[cfg(feature = "devtools")]
        ["depotdump"] => {
            unsafe {
                let slot = crate::rebase(0x1_09d2_3188) as *const u8;
                let depot_pp = slot.add(0x1f8) as *const *const u8;
                if !crate::memoria::is_readable(depot_pp as *const c_void, 8) {
                    return crate::log_dev!("[depot] slot ilegível");
                }
                let depot = depot_pp.read();
                if depot.is_null() || !crate::memoria::is_readable(depot as *const c_void, 0x80) {
                    return crate::log_dev!("[depot] inst={depot:p} null/ilegível (boot incompleto?)");
                }
                let vt = (depot as *const *const u64).read();
                let mut s = format!("[depot] inst={depot:p} vtable static {:#x}\n", crate::un_rebase(vt as *const c_void));
                if crate::memoria::is_readable(vt as *const c_void, 8 * 0x20) {
                    for i in 0..0x20usize {
                        let f = vt.add(i).read();
                        s.push_str(&format!("  vt+{:#04x} = static {:#x}\n", i * 8, crate::un_rebase(f as *const c_void)));
                    }
                }
                for off in [0x10usize, 0x18, 0x1c, 0x20, 0x68, 0x74] {
                    let v = (depot.add(off) as *const u64).read();
                    s.push_str(&format!("  +{:#04x} = {:#018x}\n", off, v));
                }
                log(&s);
            }
            return;
        }
        // `archivegroupdump` (2026-08-17, auditoria de consistência,
        // `ResolveArchiveGroup`) — DIAGNÓSTICO read-only: lista TODOS os grupos do depot com
        // basePath (offset +0x10) e scope (offset +0x30), decodificados via o header oficial
        // vendorizado. Confirma (ou refuta) os 2 offsets novos antes de usar `resolve_archive_
        // _group_by_path` de verdade em qualquer lugar. Zero escrita, zero mutação.
        // 2026-08-16/17 (rodada 41): núcleo em `run_archivegroupdump()` (diagnostico/mod.rs, junto de
        // `archive_scope_name`) — este braço fica como fallback do canal gated-por-executor (útil
        // se o executor ainda estiver ativo quando o comando chegar); o braço PRINCIPAL agora é o
        // do `[hb-canal]` (loop pós-boot, thread do heartbeat, sempre viva) — ver logo abaixo.
        #[cfg(feature = "devtools")]
        ["archivegroupdump"] => {
            unsafe { run_archivegroupdump() };
            return;
        }
        // A lista do selo e da aba Mods também por aqui: os 2 canais leem o mesmo /tmp/cp77-cmd.txt e quem lê primeiro responde; em
        // gameplay este canal ganhou e respondia "não é um comando conhecido" (boot de 2026-10-24 19:33, lane M).
        #[cfg(feature = "devtools")]
        ["modslista"] => {
            crate::veredito_dev!("{}", crate::mods_do_usuario::lista_dos_mods::veredito());
            return;
        }
        // `archivegroupresolve <basePath>` (2026-08-17) — testa
        // `resolve_archive_group_by_path` contra um basePath real (achado via `archivegroupdump`
        // primeiro) e um inventado. Read-only, zero mutação.
        #[cfg(feature = "devtools")]
        ["archivegroupresolve", base_path] => {
            unsafe {
                let depot = PATHB_DEPOT.load(Ordering::Relaxed);
                if depot.is_null() {
                    return crate::log_dev!("[archivegroupresolve] PATHB_DEPOT ainda não capturado (boot incompleto?)");
                }
                match resolve_archive_group_by_path(depot as *mut u8, base_path) {
                    Some(g) => crate::log_dev!("[archivegroupresolve] '{base_path}' -> grupo @{g:p}"),
                    None => crate::log_dev!("[archivegroupresolve] '{base_path}' -> nenhum grupo bate (esperado se for path novo)"),
                }
            }
            return;
        }
        // `archivegroupcreate <basePath>` (2026-08-17, metade de ESCRITA) —
        // testa `create_archive_group_by_path` (grow+insert real do `depot->groups`, mutação de
        // memória viva do motor). Rode
        // `archivegroupdump` ANTES e DEPOIS pra confirmar visualmente: grupo novo no índice certo,
        // scope=Mod, count total +1, e os grupos vizinhos intactos (mesmos basePath/scope de antes).
        #[cfg(feature = "devtools")]
        ["archivegroupcreate", base_path] => {
            unsafe {
                let depot = PATHB_DEPOT.load(Ordering::Relaxed);
                if depot.is_null() {
                    return crate::log_dev!("[archivegroupcreate] PATHB_DEPOT ainda não capturado (boot incompleto?)");
                }
                match create_archive_group_by_path(depot as *mut u8, base_path) {
                    Some(g) => crate::log_dev!("[archivegroupcreate] '{base_path}' -> grupo @{g:p} (ok, ver archivegroupdump p/ confirmar)"),
                    None => crate::log_dev!("[archivegroupcreate] '{base_path}' -> falhou (ver log anterior pro motivo)"),
                }
            }
            return;
        }
        // Journal real (`gameJournalManager` pelo mapa de sistemas do jogo, a mesma rota da produção), só leitura: tree, contagens do
        // índice e as primeiras raízes com a classe de cada uma.
        #[cfg(feature = "devtools")]
        ["jmdump"] => {
            unsafe {
                let Some(jm) = crate::mundo::journal_manager_do_jogo(reg, player) else {
                    return crate::log_dev!("[jmdump] JournalManager não achado (precisa do player em jogo)");
                };
                let Some(tree) = crate::mundo::journal_tree_de(jm) else {
                    return crate::log_dev!("[jmdump] JM = {jm:p} sem tree legível");
                };
                let conta = |off: usize| ((jm as *const u8).add(off) as *const u32).read_unaligned();
                let raizes = crate::mundo::journal_tree_raizes(tree);
                crate::log_dev!(
                    "[jmdump] JM = {jm:p} tree = {tree:p} raízes = {} · índice: {} por hash, {} por entrada",
                    raizes.len(),
                    conta(crate::missoes_e_mapa::JOURNALTREE_INDICE_HASH_COUNT_OFF),
                    conta(crate::missoes_e_mapa::JOURNALTREE_INDICE_ENTRADA_COUNT_OFF)
                );
                for (i, raiz) in raizes.iter().take(8).enumerate() {
                    let classe = crate::cname::resolve_cname(crate::rtti::type_name_hash(crate::rtti::class_of(*raiz)));
                    crate::log_dev!("[jmdump] raiz [{i}] {raiz:p} {classe}");
                }
            }
            return;
        }
        // DIAGNÓSTICO de construção RED (rodar no MENU PRINCIPAL = save-safe):
        // rttidump = só LÊ a vtable + getters (seguro); newobj = tiro único de
        // CONSTRUÇÃO (arriscado — pode crashar; por isso no menu, sem save aberto).
        #[cfg(feature = "devtools")]
        ["rttidump", class] => {
            log(&unsafe { crate::rtti::dump_class(reg, class) });
            return;
        }
        // vtbl <cls-name> <n_slots> — lista N slots da vtable de uma classe RTTI
        // Usa o punteiro de instância da classe para achar a vtable e converte para vmaddr.
        // Ex: vtbl gameuiCharacterCustomizationSystem 30
        // Descoberta: constrói classe vinda do argumento e não solta o objeto (rtti.rs: Construct() em classe arbitrária travou o canal).
        #[cfg(feature = "devtools")]
        ["vtbl", class, n_str] => {
            let n: usize = n_str.parse().unwrap_or(20).min(64);
            unsafe {
                let obj = crate::rtti::new_object(&reg, class);
                if obj.is_null() {
                    crate::log_dev!("[vtbl] new_object falhou para '{class}'");
                    return;
                }
                if !crate::memoria::is_readable(obj, 8) {
                    crate::log_dev!("[vtbl] instância {obj:p} ilegível");
                    return;
                }
                let vtbl_ptr = (obj as *const u64).read_unaligned() as *const u64;
                if vtbl_ptr.is_null() || !crate::memoria::is_readable(vtbl_ptr as *const c_void, (n * 8) as usize) {
                    crate::log_dev!("[vtbl] vtable {vtbl_ptr:p} ilegível");
                    return;
                }
                crate::log_dev!("[vtbl] class={class} vtbl={vtbl_ptr:p}");
                let base = crate::game_base();
                for i in 0..n {
                    let slot_ptr = vtbl_ptr.add(i) as *const u64;
                    if !crate::memoria::is_readable(slot_ptr as *const c_void, 8) { break; }
                    let fn_rt = slot_ptr.read_unaligned() as usize;
                    let vmaddr = if fn_rt > base { fn_rt - base + 0x1_0000_0000 } else { fn_rt };
                    crate::log_dev!("[vtbl] [{i:02}] rt={fn_rt:#x} vmaddr={vmaddr:#010x}");
                }
            }
            return;
        }
        #[cfg(feature = "devtools")]
        ["propdump", class] => {
            log(&unsafe { crate::rtti::dump_props(reg, class) });
            return;
        }
        // Introspecção fina de flags (`PENDENCIAS-UNIFICADAS.md`, "CClass::Flags
        // 11 bits / CProperty.flags 13 bits / CBaseFunction.flags 13 bits — maioria nunca
        // lida"). 3 comandos read-only, decodificam os bits nomeados das 3 structs.
        #[cfg(feature = "devtools")]
        ["classflags", name] => {
            unsafe { crate::rtti::probe_class_flags(reg, name) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["propflags", class, prop] => {
            unsafe { crate::rtti::probe_property_flags(reg, class, prop) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["funcflags", class, method] => {
            unsafe { crate::rtti::probe_function_flags(reg, class, method) };
            return;
        }
        // `funcflagsglobal <nome>` (2026-08-10) — lado escrita: diagnóstico
        // pra confirmar/refutar o "palpite" documentado em `register.rs::FLAG_STATIC` (bit2,
        // nunca verificado contra o ground-truth de `decode_function_flags`, que diz bit1=
        // isStatic/bit2=isFinal do header vendorizado). Testa contra uma native GLOBAL nossa
        // (sempre estática) já registrada.
        #[cfg(feature = "devtools")]
        ["funcflagsglobal", name] => {
            unsafe {
                let f = crate::register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    return crate::log_dev!("[funcflagsglobal] '{name}' não resolveu");
                }
                match crate::rtti::function_flags(f) {
                    Some(raw) => crate::log_dev!(
                        "[funcflagsglobal] '{name}' flags={raw:#010x} -> {}",
                        crate::rtti::decode_function_flags(raw)
                    ),
                    None => crate::log_dev!("[funcflagsglobal] '{name}' flags ilegível"),
                }
            }
            return;
        }
        // `Reflection.ReflectionFunc` — "introspecção de assinatura" (nome, retorno,
        // params, is_native/is_static), junta `resolve_func`+`type_kind`+`function_flags`.
        #[cfg(feature = "devtools")]
        ["funcsig", class, method] => {
            unsafe { crate::rtti::probe_function_signature(reg, class, method) };
            return;
        }
        // `Reflection.ReflectionEnum`/`ReflectionBitfield.IsNative()`.
        #[cfg(feature = "devtools")]
        ["enumisnative", name] => {
            unsafe { crate::rtti::probe_type_is_native(reg, name) };
            return;
        }
        // `ReflectionClass.GetFunctions()`/`GetStaticFunctions()` (item #51).
        #[cfg(feature = "devtools")]
        ["funclistdump", class] => {
            unsafe { crate::rtti::probe_class_own_functions(reg, class, 15) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["funclistdump", class, n] => {
            let n: usize = n.parse().unwrap_or(15).min(200);
            unsafe { crate::rtti::probe_class_own_functions(reg, class, n) };
            return;
        }
        // `CClass.defaults` (Map<CName,Variant*>@+0x158, PENDENCIAS-UNIFICADAS.md
        // prosa "Restante"): valor-default de uma propriedade, nunca lido antes.
        #[cfg(feature = "devtools")]
        ["classdefault", class, prop] => {
            unsafe { crate::rtti::probe_class_default(reg, class, prop) };
            return;
        }
        // `newobjget <classe> <prop>` (2026-08-10) — `CClass.defaults`: a via
        // OFICIAL (ler o Map interno `CClass+0x158`) deu vazio nos 5 casos testados antes. Via
        // alternativa: construir uma instância NOVA (`rtti::new_object`, já provado) e ler a prop
        // (`find_property_in_class`+`prop_get_*`, já provado) — o valor que sai do `Construct()`
        // real do motor É o default de fato, na prática mais confiável que o Map (que parece
        // raramente populado). Read-only sobre um objeto DESCARTÁVEL (nunca registrado em lugar
        // nenhum, sem side-effect fora de si mesmo).
        #[cfg(feature = "devtools")]
        ["newobjget", class, prop] => {
            unsafe {
                let cls = reg.class_by_name(class);
                if cls.is_null() {
                    return crate::log_dev!("[newobjget] classe '{class}' não encontrada");
                }
                let obj = crate::rtti::new_object(reg, class);
                if obj.is_null() {
                    return crate::log_dev!("[newobjget] new_object('{class}') falhou");
                }
                let p = crate::rtti::find_property_in_class(cls, prop);
                if p.is_null() {
                    return crate::log_dev!("[newobjget] prop '{prop}' não achada em '{class}'");
                }
                let vo = crate::rtti::prop_value_offset(p);
                let u = crate::rtti::prop_get_u32(p, obj);
                let f = crate::rtti::prop_get_f32(p, obj);
                let b = crate::rtti::prop_get_bool(p, obj);
                crate::log_dev!(
                    "[newobjget] {class}.{prop} (vo={vo:#x}, instância NOVA descartável @{obj:p}) = u32 {u:#x} / i32 {} / f32 {f} / bool {b}",
                    u as i32
                );
            }
            return;
        }
        // "Família de tipos-wrapper" (itens #129-158): introspecção genérica de
        // categoria de tipo via `rtti::IType::GetType()` (macOS vtbl+0x28, shift de +0x08 vs
        // Windows — 2 dtors Itanium, achado já documentado no código, nunca exposto como comando).
        #[cfg(feature = "devtools")]
        ["proptypekind", class, prop] => {
            unsafe { crate::rtti::probe_property_type_kind(reg, class, prop) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["proptypekindall", class] => {
            unsafe { crate::rtti::probe_property_type_kind_all(reg, class, 20) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["proptypekindall", class, n] => {
            let n: usize = n.parse().unwrap_or(20).min(200);
            unsafe { crate::rtti::probe_property_type_kind_all(reg, class, n) };
            return;
        }
        // Customização: lista métodos RTTI (instância+estáticos) de uma classe.
        // Ex: rttifuncs gameuiCharacterCustomizationSystem → GetHeadOptions/GetBodyOptions etc.
        #[cfg(feature = "devtools")]
        ["rttifuncs", class] => {
            log(&unsafe { crate::rtti::dump_class_funcs(reg, class) });
            return;
        }
        // Controller de recurso: dado CClass+método, acha o CClassFunction* e despeja qwords
        // em múltiplos offsets — identifica qual offset tem o ponteiro nativo C++ (no TEXT).
        // Ex: nativefunc IComponent Toggle → despeja CFunc*+0x00..+0x60; candidato = vmaddr TEXT.
        // `resolvefuncrobust <nome>` (2026-08-10) — compara `register::get_function`
        // (vtbl+0x30, só acha o que foi RegisterFunction-ado pelo Rust) contra
        // `rtti::resolve_global_function_robust` (varre GetGlobalFunctions, casa por nome/fullName)
        // — prova se o fallback resolve uma global PURAMENTE ESCRIPTADA que a via rápida não acha.
        #[cfg(feature = "devtools")]
        ["resolvefuncrobust", name] => {
            unsafe {
                let fast = crate::register::get_function(reg, name);
                let robust = crate::rtti::resolve_global_function_robust(reg, name);
                crate::log_dev!(
                    "[resolvefuncrobust] '{name}': get_function(vtbl+0x30)={fast:p} | resolve_global_function_robust(varredura)={robust:p}"
                );
            }
            return;
        }
        #[cfg(feature = "devtools")]
        ["nativefunc", class, method] => {
            unsafe {
                let cls = reg.class_by_name(class);
                if cls.is_null() {
                    crate::log_dev!("[nativefunc] classe '{class}' não encontrada");
                    return;
                }
                match crate::rtti::resolve_in_class(cls, method) {
                    None => crate::log_dev!("[nativefunc] '{method}' não achado em '{class}'"),
                    Some(rf) => {
                        let f = rf.func as *const u8;
                        crate::log_dev!("[nativefunc] '{class}::{method}' func={f:p}");
                        let base = crate::game_base() as u64;
                        for off in (0usize..=0x80).step_by(8) {
                            if !crate::memoria::is_readable(f.add(off) as *const c_void, 8) { break; }
                            let v = (f.add(off) as *const u64).read_unaligned();
                            let is_text = v > base && v < base + 0x800_0000;
                            let vm = if is_text { format!("vmaddr={:#010x}", v - base + 0x1_0000_0000u64) } else { String::new() };
                            crate::log_dev!("[nativefunc] +{off:#04x}: {v:#018x} {vm}");
                        }
                    }
                }
            }
            return;
        }
        // `getcustsys`: obtém o ptr do gameuiCharacterCustomizationSystem via GameInstance
        // (player.GetGame() → GetCharacterCustomizationSystem()), atualiza CHAR_CUSTOM_SYS_PTR,
        // e loga o ptr para uso em callon/custsys.
        #[cfg(feature = "devtools")]
        ["getcustsys"] => {
            unsafe {
                // BUG CORRIGIDO 2026-07-28: `class_of(player)` derivava a classe da INSTÂNCIA
                // capturada — falha (game_cls=0x0) logo após save-load/transições de cena, mesmo
                // com `player` não-nulo (mesmo padrão documentado em jogo::sessao::auth_player,
                // "player capturado costuma ser puppet transiente"). Fix: resolver "GetGame"
                // DIRETO da classe estática "PlayerPuppet" (sempre registrada), sem depender do
                // estado da instância — mesma receita robusta já usada em auth_player.
                crate::log_dev!("[getcustsys] player={player:p}");
                match crate::jogo::sessao::game_instance(reg, player) {
                    None => crate::log_dev!("[getcustsys] GetGame falhou"),
                    Some(r) => {
                        let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                        crate::log_dev!("[getcustsys] GetGame()={game_ptr:#018x}");
                        if game_ptr == 0 { return; }
                        // CAUSA RAIZ ACHADA 2026-07-28 (via `nativefunc ScriptGameInstance
                        // GetQuestsSystem`, mesmo shape): a struct do CClassFunction tem
                        // p_count=1 (campo +0x30, lido por `call_func::p_count`) — este native
                        // espera 1 ARG explícito, não 0. `GetCharacterCustomizationSystem(self:
                        // GameInstance)` é o padrão redscript "static com self explícito"
                        // (chamado como `GameInstance.GetX(game)` no .script real, não
                        // `game.GetX()`) — o GameInstance tem que ir no ARRAY DE ARGS
                        // (`Arg::Raw`, o próprio comentário do enum já dizia "ex.: GameInstance")
                        // com `ctx` sendo QUALQUER objeto válido (não o GameInstance) — exatamente
                        // como `jogo::sessao::auth_player` já fazia pra `GetPlayerSystem` (mesmo
                        // padrão de assinatura). O crash de antes (EXC_BAD_ACCESS, endereço com
                        // cara de hash) era ler PARÂMETRO FALTANTE como se fosse dado — `call_func`
                        // só rejeita `args.len() > p_count`, nunca `args.len() < p_count`, então
                        // a chamada com 0 args "passava" sem erro só pra crashar dentro do native.
                        let mut game_arg = [0u8; 16];
                        game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                        match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetCharacterCustomizationSystem") {
                            None => crate::log_dev!("[getcustsys] GetCharacterCustomizationSystem não achado em GameInstance"),
                            Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                None => crate::log_dev!("[getcustsys] GetCharacterCustomizationSystem() não completou"),
                                Some(r2) => {
                                    let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                    // O +1 fica: é ele que mantém válido o ponteiro guardado em `CHAR_CUSTOM_SYS_PTR` pro `custsys` usar depois.
                                    crate::log_dev!("[getcustsys] gameuiCharacterCustomizationSystem={sys_ptr:#018x}");
                                    if sys_ptr != 0 {
                                        crate::selftest::CHAR_CUSTOM_SYS_PTR.store(sys_ptr, std::sync::atomic::Ordering::Relaxed);
                                        crate::log_dev!("[getcustsys] CHAR_CUSTOM_SYS_PTR actualizado — custsys probe vai funcionar");
                                    }
                                }
                            },
                        }
                    }
                }
            }
            return;
        }
        // Player, agendamento e veículo (WardrobeSystem::ForgetItemID): os offsets do Windows
        // — `HashMap<CName,ItemID> ItemStore`
        // em +0x48, `SharedSpinLock ItemStoreMutex` em +0xF4 — NÃO são endereços de função (que
        // mudam entre plataformas), são offsets de DADOS/struct, que este projeto já confirmou
        // repetidas vezes portarem 1:1 macOS↔Windows (`Core::OffsetPtr<0xNN>`). Em vez de tentar
        // achar uma função nativa que NÃO EXISTE (RE já confirmou isso — ver DATABASE.md), lê os
        // bytes crus nesses offsets de uma instância VIVA (mesma ABI de "static com self explícito"
        // já provada em getcustsys) — observe-only, zero mutação, zero hook — só pra
        // VALIDAR se os offsets batem (heurística: HashMap real tem cara de {ptr,count,cap} ou
        // similar; SharedSpinLock real é 0 quando destravado).
        #[cfg(feature = "devtools")]
        ["wardrobesys"] => {
            unsafe {
                match crate::jogo::sessao::game_instance(reg, player) {
                    None => crate::log_dev!("[wardrobesys] GetGame falhou"),
                    Some(r) => {
                        let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                        if game_ptr == 0 { crate::log_dev!("[wardrobesys] GetGame()=0"); return; }
                        let mut game_arg = [0u8; 16];
                        game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                        match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetWardrobeSystem") {
                            None => crate::log_dev!("[wardrobesys] GetWardrobeSystem não achado em GameInstance"),
                            Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                None => crate::log_dev!("[wardrobesys] GetWardrobeSystem() não completou"),
                                Some(r2) => {
                                    let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                    crate::rtti::soltar_retorno(&rf2, &r2);
                                    crate::log_dev!("[wardrobesys] WardrobeSystem={sys_ptr:#018x}");
                                    if sys_ptr == 0 { return; }
                                    let obj = sys_ptr as *const u8;
                                    if !crate::memoria::is_readable(obj as *const c_void, 0x100) {
                                        crate::log_dev!("[wardrobesys] instância ilegível (menos de 0x100 bytes)");
                                        return;
                                    }
                                    // dump bruto de +0x40..+0x60 (janela ao redor do candidato +0x48
                                    // ItemStore) + +0xF0..+0x100 (janela ao redor do +0xF4 mutex)
                                    for off in (0x40usize..0x60).step_by(8) {
                                        let v = (obj.add(off) as *const u64).read_unaligned();
                                        crate::log_dev!("[wardrobesys] +{off:#x} = {v:#018x}");
                                    }
                                    for off in (0xf0usize..0x100).step_by(8) {
                                        let v = (obj.add(off) as *const u64).read_unaligned();
                                        crate::log_dev!("[wardrobesys] +{off:#x} = {v:#018x}");
                                    }
                                    // 2026-07-31: enumeração COMPLETA, 100% observe-only (zero
                                    // escrita), do HashMap<CName,ItemID> real em WardrobeSystem+0x48.
                                    // Layout confirmado por agente de RE
                                    // (ground-truth, ver DATABASE.md): indexTable@+0x00(rel ao HashMap,
                                    // =WardrobeSystem+0x48), size@+0x08(u32), capacity@+0x0C(u32),
                                    // nodes@+0x10(Node*), Node{next:u32@0,hashedKey:u32@4,key:CName
                                    // u64@8,value:ItemID 16B@0x10}, stride=0x20. Caminha TODOS os
                                    // buckets + cadeias — nunca escreve nada, só lê e loga. Serve pra
                                    // validar o algoritmo de hash/bucket/cadeia ANTES de confiar numa
                                    // implementação de Remove (que precisaria escrever).
                                    let hm = obj.add(0x48);
                                    let index_table = (hm as *const u64).read_unaligned() as *const u32;
                                    let size_cap = (hm.add(0x08) as *const u64).read_unaligned();
                                    let map_size = (size_cap & 0xFFFF_FFFF) as u32;
                                    let capacity = (size_cap >> 32) as u32;
                                    let nodes = (hm.add(0x10) as *const u64).read_unaligned() as *const u8;
                                    crate::log_dev!("[wardrobesys] HashMap: indexTable={index_table:p} size={map_size} capacity={capacity} nodes={nodes:p}");
                                    if capacity == 0 || capacity > 4096 || index_table.is_null() || nodes.is_null() {
                                        crate::log_dev!("[wardrobesys] capacity/ponteiros fora de faixa plausível, abortando enumeração");
                                    } else if !crate::memoria::is_readable(index_table as *const c_void, (capacity as usize) * 4) {
                                        crate::log_dev!("[wardrobesys] indexTable ilegível");
                                    } else {
                                        const STRIDE: usize = 0x20;
                                        const INVALID: u32 = 0xFFFF_FFFF;
                                        let mut found: u32 = 0;
                                        for bucket in 0..capacity {
                                            let mut idx = index_table.add(bucket as usize).read_unaligned();
                                            let mut guard = 0;
                                            while idx != INVALID && guard < capacity + 1 {
                                                guard += 1;
                                                let node = nodes.add(idx as usize * STRIDE);
                                                if !crate::memoria::is_readable(node as *const c_void, STRIDE) {
                                                    crate::log_dev!("[wardrobesys] bucket={bucket} idx={idx} node ilegível, parando cadeia");
                                                    break;
                                                }
                                                let next = (node as *const u32).read_unaligned();
                                                let hashed_key = (node.add(4) as *const u32).read_unaligned();
                                                let key = (node.add(8) as *const u64).read_unaligned();
                                                let value_lo = (node.add(0x10) as *const u64).read_unaligned();
                                                let value_hi = (node.add(0x18) as *const u64).read_unaligned();
                                                found += 1;
                                                crate::log_dev!("[wardrobesys] item#{found} bucket={bucket} idx={idx} hashedKey={hashed_key:#010x} CName={key:#018x} ItemID=({value_lo:#018x},{value_hi:#018x})");
                                                idx = next;
                                            }
                                        }
                                        crate::log_dev!("[wardrobesys] enumeração completa: {found} entradas encontradas (map_size reportado={map_size})");
                                    }
                                }
                            },
                        }
                    }
                }
            }
            return;
        }
        // `WardrobeSystem.ForgetItemID` (item #46/#208, 2026-08-11, 3ª rodada de
        // candidatos baratos) — mesma cadeia GetGame/GetWardrobeSystem de `wardrobesys` acima,
        // mas remove por TDBID (scan-linear por `ItemID.tdbid`, ver
        // `selftest::wardrobe_forget_by_tdbid`) em vez de CName exato — não precisa saber a
        // CHAVE (appearanceName) de antemão, só o TweakDBID do item. Devolve a CONTAGEM removida
        // (pode ser >1 se o mesmo tdbid tiver mais de uma entrada, mesma semântica da fonte real).
        ["wardrobeforgettdbid", tdbid_hex] => {
            unsafe {
                let target_tdbid: u64 = match u64::from_str_radix(tdbid_hex.trim_start_matches("0x"), 16) {
                    Ok(v) => v,
                    Err(_) => { crate::log_dev!("[wardrobeforgettdbid] TweakDBID inválido (espera hex, ex: 2c00011165)"); return; }
                };
                match crate::jogo::sessao::game_instance(reg, player) {
                    None => crate::log_dev!("[wardrobeforgettdbid] GetGame falhou"),
                    Some(r) => {
                        let game_ptr = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                        if game_ptr == 0 { crate::log_dev!("[wardrobeforgettdbid] GetGame()=0"); return; }
                        let mut game_arg = [0u8; 16];
                        game_arg[..8].copy_from_slice(&game_ptr.to_le_bytes());
                        match crate::rtti::resolve_any(reg, &["ScriptGameInstance", "GameInstance", "gameScriptGameInstance"], "GetWardrobeSystem") {
                            None => crate::log_dev!("[wardrobeforgettdbid] GetWardrobeSystem não achado"),
                            Some(rf2) => match crate::rtti::call_func(&rf2, player, &[crate::rtti::Arg::Raw(game_arg)]) {
                                None => crate::log_dev!("[wardrobeforgettdbid] GetWardrobeSystem() não completou"),
                                Some(r2) => {
                                    let sys_ptr = u64::from_le_bytes([r2[0],r2[1],r2[2],r2[3],r2[4],r2[5],r2[6],r2[7]]);
                                    crate::rtti::soltar_retorno(&rf2, &r2);
                                    if sys_ptr == 0 { crate::log_dev!("[wardrobeforgettdbid] WardrobeSystem=0"); return; }
                                    let removed = crate::roupas_e_armas::wardrobe_forget_by_tdbid(sys_ptr as *mut u8, target_tdbid);
                                    crate::log_dev!("[wardrobeforgettdbid] TDBID={target_tdbid:#018x} removidos={removed}");
                                }
                            },
                        }
                    }
                }
            }
            return;
        }
        // Customização: usa o ptr salvo por getcustsys/CharCustomA probe para
        // inspecionar e chamar métodos do gameuiCharacterCustomizationSystem.
        // custsys probe → loga ptr + valida via class_of
        // custsys call <method> → callon <ptr_salvo> <method> via RTTI
        #[cfg(feature = "devtools")]
        ["custsys", action @ ..] => {
            unsafe {
                let ptr = crate::selftest::CHAR_CUSTOM_SYS_PTR.load(std::sync::atomic::Ordering::Relaxed);
                if ptr == 0 {
                    crate::log_dev!("[custsys] ptr não capturado — use getcustsys primeiro");
                    return;
                }
                crate::log_dev!("[custsys] ptr={ptr:#018x} action={}", action.join(" "));
                let obj = ptr as *mut std::ffi::c_void;
                match action {
                    ["probe"] | [] => {
                        let cls = crate::rtti::class_of(obj);
                        crate::log_dev!("[custsys] probe cls={cls:p} valid={}",
                            !cls.is_null());
                    }
                    ["call", method, raw_args @ ..] => {
                        let cls = crate::rtti::class_of(obj);
                        if cls.is_null() {
                            crate::log_dev!("[custsys] ptr={ptr:#018x} não é objeto RED válido");
                            return;
                        }
                        // Estado do puppet (2026-08-03, /goal): suporte a "player" como arg —
                        // passa o handle real do player capturado (Arg::Handle, mesmo padrão de
                        // `give`/roupas_e_armas::give), pra chamar métodos como `HasCharacterCustomizationComponent
                        // (entity: ref<Entity>)` que precisam de um handle de verdade, não um enum/int.
                        let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| {
                            if *a == "player" && !player.is_null() {
                                crate::rtti::Arg::Handle(player, crate::rtti::refcnt())
                            } else {
                                parse_cmd_arg(a)
                            }
                        }).collect();
                        match crate::rtti::resolve_in_class(cls, method) {
                            Some(rf) => match crate::rtti::call_func(&rf, obj, &args) {
                                Some(r) => {
                                    let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i+1], r[i+2], r[i+3]]));
                                    crate::log_dev!("[custsys] {method}({}) -> i32 {} / u32 {:#x} / f32 {}",
                                        raw_args.join(" "),
                                        i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                        u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                        f(0));
                                    // Itens 47/66 (2026-08-18): decodifica também como possível
                                    // DynArray<T> sret {entries(ptr)@0x00, cap(u32)@0x08, size(u32)@0x0C}
                                    // — layout já batalha-testado em todo o resto do projeto (rtti.rs).
                                    // GetHeadOptions()/etc. devolvem array<T>, não escalar; útil pra ler
                                    // ArraySize sem precisar de comando novo dedicado.
                                    let entries = u64::from_le_bytes([r[0],r[1],r[2],r[3],r[4],r[5],r[6],r[7]]);
                                    let cap = u32::from_le_bytes([r[8],r[9],r[10],r[11]]);
                                    let size = u32::from_le_bytes([r[12],r[13],r[14],r[15]]);
                                    crate::log_dev!("[custsys] {method}(...) como DynArray sret: entries={entries:#018x} cap={cap} size={size}");
                                    crate::rtti::soltar_retorno(&rf, &r);
                                }
                                None => crate::log_dev!("[custsys] {method}({}) → void/falha", raw_args.join(" ")),
                            },
                            None => crate::log_dev!("[custsys] método '{method}' não achado na classe"),
                        }
                    }
                    _ => crate::log_dev!("[custsys] uso: custsys probe | custsys call <method> [args]"),
                }
            }
            return;
        }
        // SetFlat em runtime: getflat <nome> (read-only, dumpa o FlatValue vivo);
        // setflat <nome> <0xhex|int|float> (escreve 4B em +0x08).
        #[cfg(feature = "devtools")]
        ["getflat", name] => {
            unsafe { crate::tweakdb::probe_flat(name) };
            return;
        }
        // `flatconf <flat> <esperado>` e `recconf <record> <classe>`: conferem o que um arquivo de tweak gravou (o roteiro da pasta `Mods/`)
        // e respondem no canal de veredito, fora da amostragem do log.
        #[cfg(feature = "devtools")]
        ["flatconf", flat, esperado] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_flat(flat, esperado) });
            return;
        }
        #[cfg(feature = "devtools")]
        ["recconf", record, classe] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_record(record, classe) });
            return;
        }
        // `refconf <flat> <classes>` e `refflat <flat> <índice> <prop> <esperado>`: records em linha (nome sintético) pelo flat que os aponta.
        #[cfg(feature = "devtools")]
        ["refconf", flat, classes] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_referencias(flat, classes) });
            return;
        }
        #[cfg(feature = "devtools")]
        ["meshctx", caminho, atributo, esperado, fonte] => {
            crate::veredito_dev!("{}", unsafe { crate::aparencia::conferir_contexto_do_mesh(caminho, atributo, esperado, fonte) });
            return;
        }
        #[cfg(feature = "devtools")]
        ["iconconf", record] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_icone(record) });
            return;
        }
        #[cfg(feature = "devtools")]
        ["arrconf", flat, tamanho, indice, record] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_array_por_tamanho(flat, tamanho, indice, record) });
            return;
        }
        #[cfg(feature = "devtools")]
        ["refflat", flat, indice, prop, esperado] => {
            crate::veredito_dev!("{}", unsafe { crate::tweakdb::teste::conferir_flat_referenciado(flat, indice, prop, esperado) });
            return;
        }
        // Leitura de records do TweakDB: getarr <nome> lê um flat ARRAY (ex.: attacks/statModifiers
        // de uma arma) de um record vivo, read-only. Ver tweakdb::probe_array_flat.
        #[cfg(feature = "devtools")]
        ["getarr", name] => {
            unsafe { crate::tweakdb::probe_array_flat(name) };
            return;
        }
        // Resource patch (2026-07-24): postloadprobe constrói uma instância REAL de
        // EntityTemplate/AppearanceResource/CMesh/MorphTargetMesh (rtti::new_object) e lê o slot
        // PostLoad (+0x30) da vtable própria do objeto — resolve os 4 endereços sem RE estática.
        // Read-only após construir (não chama PostLoad). Rodar no MENU (sem save aberto).
        #[cfg(feature = "devtools")]
        ["postloadprobe"] => {
            unsafe { crate::selftest::probe_postload_addresses() };
            return;
        }
        // `ISerializable.ProcessPostLoad`/`RefreshResource`: `postloadprobe`
        // acima só LÊ o endereço do slot PostLoad (+0x30), nunca chama. Este comando vai além:
        // constrói uma instância fresca/descartável (mesmo `new_object`, mesma classe já usada
        // com segurança por `postloadprobe` há sessões) e CHAMA `ProcessPostLoad` de verdade
        // (o mesmo código-path exposto ao redscript via `BwmsProcessPostLoad`), dumpando bytes
        // antes/depois pra confirmar mutação real, não só "não crashou". Ex.: `postloadcall CMesh`.
        #[cfg(feature = "devtools")]
        ["postloadcall", cls] => {
            unsafe { crate::selftest::postload_call_probe(cls) };
            return;
        }
        // `CBitfield` (12 métodos, PENDENCIAS-UNIFICADAS.md): lista os bits nomeados de um
        // bitfield já resolvido (vanilla ou forjado).
        #[cfg(feature = "devtools")]
        ["bitfielddump", name] => {
            if let Some(reg) = unsafe { crate::rtti::Registry::obtain() } {
                unsafe { crate::rtti::probe_bitfield_dump(&reg, name) };
            } else {
                crate::log_dev!("[bitfielddump] Registry indisponível");
            }
            return;
        }
        // Item #50 (Reflection.GetClasses/GetEnums/GetBitfields — universo filtrado por
        // categoria, composição de get_native_types+type_kind, já provados).
        #[cfg(feature = "devtools")]
        ["reflenum", kind] => {
            unsafe { crate::rtti::probe_count_and_list_by_kind(reg, kind, 10) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["reflenum", kind, n] => {
            let n: usize = n.parse().unwrap_or(10).min(100);
            unsafe { crate::rtti::probe_count_and_list_by_kind(reg, kind, n) };
            return;
        }

        // `reflfind <trecho> [n]` — o mapa de tipos por SUBSTRING. Complementa `reflenum` (que
        // filtra por categoria): serve pra responder "esse tipo existe neste build, e com que
        // nome?" sem depender de adivinhar o nome exato.
        #[cfg(feature = "devtools")]
        ["reflfind", trecho] => {
            unsafe { crate::rtti::probe_find_types(reg, trecho, 40) };
            return;
        }
        // `curvelayout <classe> <prop>` — MEDE o layout de um `curveData:*` instanciando a classe
        // portadora que o `reflholders` apontou. Regra do `#30`: medir, nunca ler do header.
        #[cfg(feature = "devtools")]
        ["curvelayout", classe, prop, ..] => {
            unsafe { crate::rtti::probe_curve_layout(reg, classe, prop) };
            return;
        }
        // `reflholders <trecho> [n]` — QUE CLASSES têm property daquele tipo. É a pergunta
        // seguinte à do `reflfind`: o tipo existe (já sabido), mas como chegar numa INSTÂNCIA?
        // Se alguma classe nativa tem um campo daquele tipo, qualquer instância dela é a rota.
        #[cfg(feature = "devtools")]
        ["reflholders", trecho, resto @ ..] => {
            let n = resto.first().and_then(|x| x.parse::<usize>().ok()).unwrap_or(30);
            unsafe { crate::rtti::probe_find_prop_holders(reg, trecho, n) };
            return;
        }
        #[cfg(feature = "devtools")]
        ["reflfind", trecho, n] => {
            let lim: usize = n.parse().unwrap_or(40).min(100);
            unsafe { crate::rtti::probe_find_types(reg, trecho, lim) };
            return;
        }

        // Item #50 (Reflection.GetDerivedClasses — via get_derived_classes já provado,
        // evita a anomalia de GetClasses/#63).
        #[cfg(feature = "devtools")]
        ["reflderived", base] => {
            unsafe { crate::rtti::probe_reflect_derived(reg, base) };
            return;
        }
        // Dump de vtables (prova de 2026-08-11 do dump de vtables): fatia BOUNDED do
        // universo RTTI, construindo instâncias reais e lendo o ponteiro de vtable. Gated
        // internamente atrás de dev_mode(). `count` sempre limitado (nunca o universo inteiro
        // numa chamada só) — ver doc-comment do módulo pro porquê.
        #[cfg(feature = "devtools")]
        ["vtabledump", start, count] => {
            let start: usize = start.parse().unwrap_or(0);
            let count: usize = count.parse().unwrap_or(50).min(500);
            unsafe { crate::diagnostico::vtabledump::probe_vtabledump(reg, start, count) };
            return;
        }
        // 2026-08-19 tarde/noite: constrói 1 instância NOMEADA (mesmo perfil
        // de risco de `newobj`, endurecido com filtro isAbstract) e dumpa os slots do vtable
        // dela como vmaddr estático, flagando os 2 candidatos já catalogados
        // (0x1043a060c/0x1043ac60c). Ver vtabledump.rs::probe_vtslots.
        #[cfg(feature = "devtools")]
        ["vtslots", class_name, count] => {
            let count: usize = count.parse().unwrap_or(48).min(200);
            unsafe { crate::diagnostico::vtabledump::probe_vtslots(reg, class_name, count) };
            return;
        }
        // `rttislot <hex_off>` — lê o ponteiro CRU do slot `off` da vtable do IRTTISystem (nunca
        // chama através dele) + computa o vmaddr ESTÁTICO. Usado pra alimentar `vtdump`
        // em qualquer offset novo sem precisar de um comando dedicado por item — ex. investigação
        // de disassembly do #63 (`GetClasses`@0x70)/#68 (`UnregisterType`@0x98).
        #[cfg(feature = "devtools")]
        ["rttislot", off_hex] => {
            let off = usize::from_str_radix(off_hex.trim_start_matches("0x"), 16).unwrap_or(usize::MAX);
            if off == usize::MAX {
                return crate::log_dev!("[rttislot] uso: rttislot <hex_off> (ex.: rttislot 98)");
            }
            unsafe {
                match crate::rtti::Registry::obtain() {
                    None => crate::log_dev!("[rttislot] Registry::obtain() falhou"),
                    Some(reg) => {
                        let fp = reg.vtbl_slot(off) as usize;
                        if fp == 0 {
                            return crate::log_dev!("[rttislot] +{off:#x} ilegível/null");
                        }
                        let base = crate::game_base();
                        let vmaddr = if fp > base { fp - base + 0x1_0000_0000 } else { fp };
                        crate::log_dev!("[rttislot] +{off:#x} rt={fp:#x} vmaddr={vmaddr:#010x}");
                    }
                }
            }
            return;
        }
        // Resource patch (2026-07-25): instala hooks observe-only em CMesh::PostLoad
        // (0x10519a58c) + MorphTargetMesh::PostLoad (0x1051ca1a4). Log primeiros 8 calls de cada
        // tipo com this + campos +0x08..+0x38. Objetivo: RE de layout do CResource (onde fica o
        // resource-path hash) + confirmar que os vmaddrs são realmente PostLoad. Após instalar,
        // esperar meshes carregarem em gameplay (rodar "reslinkdump" pra comparar hashes).
        #[cfg(feature = "devtools")]
        ["postload-hook"] => {
            unsafe { crate::selftest::install_postload_hooks() };
            return;
        }
        ["setflat", name, val] => {
            let v: u32 = val
                .strip_prefix("0x")
                .and_then(|x| u32::from_str_radix(x, 16).ok())
                .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                .unwrap_or(0);
            unsafe { crate::tweakdb::write_flat(name, v, 0x08) };
            return;
        }
        // SetFlat NÃO-escalar (array/string/etc.): mkflat <field> <donor> <hexbytes> — cria um
        // FlatValue novo (vtable do donor, mesmo tipo) e aponta o field pra ele.
        ["mkflat", field, donor, hex] => {
            unsafe { crate::tweakdb::mkflat_cmd(field, donor, hex) };
            return;
        }
        // SetFlat de ARRAY (armas: attacks/statModifiers): mkarr <field> <donor-array> <a,b,c>.
        // Elementos = nomes TweakDBID (ou 0xhex cru).
        ["mkarr", field, donor, list] => {
            unsafe { crate::tweakdb::mkarr_cmd(field, donor, list) };
            return;
        }
        // `!remove-all`: rmall <field> — limpa o array INTEIRO (não recebe valor,
        // distinto de `!remove`).
        ["rmall", field] => {
            let id = crate::tweakdb::tweak_db_id(field);
            let ok = match unsafe { crate::tweakdb::singleton() } {
                Some(t) => unsafe { crate::tweakdb::array_remove_all_by_id(t, id) },
                None => {
                    crate::log_dev!("[rmall] singleton indisponível");
                    return;
                }
            };
            crate::log_dev!("[rmall] '{field}' -> {ok}");
            return;
        }
        // Commit em lote de tweaks: batchset <f1>=<hex1> <f2>=<hex2> ... — aplica N sets escalares
        // TUDO-OU-NADA (se qualquer campo não resolver, aborta sem escrever nenhum).
        ["batchset", pairs @ ..] => {
            let owned: Vec<String> = pairs.iter().map(|s| s.to_string()).collect();
            unsafe { crate::tweakdb::batchset_cmd(&owned) };
            return;
        }
        // Utilitários de produção (fatia `json`): readjson <path> — lê+parseia um JSON de disco
        // (config de mod típica) sem Lua. Zero risco (I/O puro, sem tocar RTTI/memória do jogo).
        // O log diz só o tipo do topo: o `bwms_primitives::json` não serializa `Valor`, e o erro já traz linha e coluna.
        #[cfg(feature = "devtools")]
        ["readjson", path] => {
            match std::fs::read_to_string(path) {
                Ok(content) => match bwms_primitives::json::ler(&content) {
                    Ok(topo) => {
                        use bwms_primitives::json::Valor;
                        let tipo = match topo {
                            Valor::Objeto(_) => "objeto",
                            Valor::Lista(_) => "lista",
                            Valor::Texto(_) => "texto",
                            Valor::Numero(_) => "número",
                            Valor::Bool(_) => "booleano",
                            Valor::Nulo => "null",
                        };
                        crate::log_dev!("[readjson] '{path}' parseado OK: {tipo}");
                    }
                    Err(e) => crate::log_dev!("[readjson] '{path}': erro de parse: {e}"),
                },
                Err(e) => crate::log_dev!("[readjson] '{path}': erro de leitura: {e}"),
            }
            return;
        }
        // Utilitários de produção (fatia `dir`): listdir <path> — lista arquivos de um diretório
        // sem Lua (equivalente ao módulo `dir` da API Lua). Zero risco (I/O puro).
        #[cfg(feature = "devtools")]
        ["listdir", path] => {
            match std::fs::read_dir(path) {
                Ok(entries) => {
                    let names: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| e.file_name().to_string_lossy().to_string())
                        .collect();
                    crate::log_dev!("[listdir] '{path}' ({} entradas): {:?}", names.len(), names);
                }
                Err(e) => crate::log_dev!("[listdir] '{path}': erro: {e}"),
            }
            return;
        }
        // CLONE USÁVEL: cria o record novo e herda os flats do source (stats reais), em thread separada. A classe do comando fica por
        // compatibilidade; o clone a tira do base vivo. Ex:
        //   clone gamedataWeaponItem_Record Items.Preset_Lexington_Default Items.BwmsLexClone
        ["clone", _classe, source, newname] => {
            unsafe { crate::tweakdb::clone_cmd(source, newname) };
            return;
        }
        // Pipeline de tweaks em runtime: clona SEM dizer a classe (detectada automaticamente do
        // `source`) — a peça que faltava pro pipeline .yaml real dos mods, que nunca anota a
        // classe do `$base`. Ex.: xlautoclone Items.Preset_Lexington_Default Items.BwmsAutoClone
        ["xlautoclone", source, newname] => {
            unsafe { crate::tweakdb::xlautoclone_cmd(source, newname) };
            return;
        }
        // Pipeline de tweaks em runtime completo: lê+parseia um .yaml REAL de mod (o mesmo
        // parser `bwms_primitives::tweaks` do tweakdb-tool offline) e aplica no TweakDB vivo (clone/create com detecção
        // automática de classe + edits escalares). Ex.: applyxlfile /tmp/meumod.yaml
        ["applyxlfile", path] => {
            unsafe { crate::tweakdb::applyxlfile_cmd(path) };
            return;
        }
        // Reflection USÁVEL (GetValue/SetValue por nome no PLAYER vivo): getf <prop> lê;
        // setf <prop> <0xhex|int|float> escreve. class_of(player) + find_property + prop_get/set.
        #[cfg(feature = "devtools")]
        ["getf", prop] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    crate::log_dev!("[getf] prop '{prop}' não achada na classe do player");
                } else {
                    let vo = crate::rtti::prop_value_offset(p);
                    let u = crate::rtti::prop_get_u32(p, player);
                    let f = crate::rtti::prop_get_f32(p, player);
                    let b = crate::rtti::prop_get_bool(p, player);
                    crate::log_dev!(
                        "[getf] {prop} (vo={vo:#x}) = u32 {u:#x} / i32 {} / f32 {f} / bool {b}",
                        u as i32
                    );
                }
            }
            return;
        }
        ["setf", prop, val] => {
            unsafe {
                let p = crate::rtti::find_property_in_class(crate::rtti::class_of(player), prop);
                if p.is_null() {
                    crate::log_dev!("[setf] prop '{prop}' não achada");
                } else {
                    let v: u32 = val
                        .strip_prefix("0x")
                        .and_then(|x| u32::from_str_radix(x, 16).ok())
                        .or_else(|| val.parse::<i32>().ok().map(|i| i as u32))
                        .or_else(|| val.parse::<f32>().ok().map(f32::to_bits))
                        .unwrap_or(0);
                    let before = crate::rtti::prop_get_u32(p, player);
                    crate::rtti::prop_set_u32(p, player, v);
                    let after = crate::rtti::prop_get_u32(p, player);
                    crate::log_dev!("[setf] {prop}: {before:#x} -> {after:#x} (pediu {v:#x})");
                }
            }
            return;
        }
        // Reflection CALL com ARGS tipados: chama um método do player por nome, args =
        // `i:5 f:1.5 b:true n:Name s:txt e:3` (I32/F32/Bool/CName/Str/Enum). Sem args = getter.
        // Marshaling = o mesmo call_func dos cheats (provado). Completa get/set/call.
        ["callf", method, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                match crate::rtti::resolve_in_class(crate::rtti::class_of(player), method) {
                    Some(rf) => match crate::rtti::call_func(&rf, player, &args) {
                        // call_func devolve [u8;16] (sret). Interpreta como escalar (4B) E como
                        // Vector4/struct-de-16B (4×f32) — assim retornos como GetWorldPosition()→Vector4,
                        // GetWorldForward(), GetWorldOrientation()→Quaternion saem legíveis (Fase 1 do
                        // roadmap IA: posição/orientação do V por nome, sem RE nova).
                        Some(r) => {
                            let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                            crate::log_dev!(
                                "[callf] {method}({}) -> i32 {} / u32 {:#x} / f32 {} / vec4 [{:.3}, {:.3}, {:.3}, {:.3}] / bytes {:02x?}",
                                raw_args.join(" "),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                f(0),
                                f(0), f(4), f(8), f(12),
                                &r[..16]
                            );
                            crate::rtti::soltar_retorno(&rf, &r);
                        }
                        None => crate::log_dev!("[callf] {method}({}) não completou (void ou falha)", raw_args.join(" ")),
                    },
                    None => crate::log_dev!("[callf] método '{method}' não achado na classe do player"),
                }
            }
            return;
        }
        // callf num objeto ARBITRÁRIO (não só o player) — fecha o gap de registro de classe no RTTI:
        // registrar (cwregtype/cwregalias) -> instanciar (newobj, devolve o ponteiro) -> CHAMAR
        // MÉTODO nessa instância. `callon <0xptr> <method> [args]`; class_of(ptr) funciona em
        // QUALQUER objeto válido (lê vtable+8=GetType), inclusive o forjado (vtable clonada).
        #[cfg(feature = "devtools")]
        ["callon", ptr_hex, method, raw_args @ ..] => {
            unsafe {
                let addr = ptr_hex
                    .strip_prefix("0x")
                    .and_then(|x| u64::from_str_radix(x, 16).ok())
                    .unwrap_or(0);
                let obj = addr as *mut c_void;
                let cls = crate::rtti::class_of(obj);
                if cls.is_null() {
                    crate::log_dev!("[callon] ponteiro {ptr_hex} não é objeto válido (class_of falhou)");
                    return;
                }
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                match crate::rtti::resolve_in_class(cls, method) {
                    Some(rf) => match crate::rtti::call_func(&rf, obj, &args) {
                        Some(r) => {
                            let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                            crate::log_dev!(
                                "[callon] {ptr_hex}.{method}({}) -> i32 {} / u32 {:#x} / f32 {} / vec4 [{:.3}, {:.3}, {:.3}, {:.3}]",
                                raw_args.join(" "),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                f(0),
                                f(0), f(4), f(8), f(12)
                            );
                            crate::rtti::soltar_retorno(&rf, &r);
                        }
                        None => crate::log_dev!("[callon] {ptr_hex}.{method}({}) não completou (void ou falha)", raw_args.join(" ")),
                    },
                    None => crate::log_dev!("[callon] método '{method}' não achado na classe do objeto {ptr_hex}"),
                }
            }
            return;
        }
        // Reflection CALL de função GLOBAL por nome (com args tipados). Complementa callf (instância).
        // Ex.: `callg Cos f:0.0` -> 1.0 ; `callg SqrtF f:4.0` -> 2.0. get_function + call_func (provados).
        ["callg", name, raw_args @ ..] => {
            unsafe {
                let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                let f = crate::register::get_function(reg, name);
                if !crate::rtti::sane(f) {
                    crate::log_dev!("[callg] global '{name}' não resolveu");
                } else {
                    let rf = crate::rtti::ResolvedFn {
                        func: f,
                        ret_type: crate::rtti::ret_type_of(f),
                        is_static: true,
                    };
                    match crate::rtti::call_func(&rf, std::ptr::null_mut(), &args) {
                        Some(r) => {
                            crate::log_dev!(
                                "[callg] {name}({}) -> f32 {} / i32 {} / u32 {:#x} / bytes {:02x?}",
                                raw_args.join(" "),
                                f32::from_bits(u32::from_le_bytes([r[0], r[1], r[2], r[3]])),
                                i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                &r[..8]
                            );
                            crate::rtti::soltar_retorno(&rf, &r);
                        }
                        None => crate::log_dev!("[callg] {name}({}) não completou", raw_args.join(" ")),
                    }
                }
            }
            return;
        }
        // ponte de reflexão do jogo DISPATCH (in-game): cola uma linha Lua `Namespace.Method(args)`,
        // parseia via `parse_chamada_colada` (tokenizer+inferência offline provados), marshalha cada
        // ArgColado pro `rtti::Arg` tipado e DISPATCHA — resolve como MÉTODO na classe `namespace`
        // (callf-style, no player) OU como GLOBAL `method` (callg-style). Cobre o subconjunto de
        // linhas Lua que mapeiam direto pra função/método do RTTI (não os bindings especiais `Game.*`
        // do Lua, que roteiam pra N pontos distintos — esses seguem no `parse_linha_de_give` give/money).
        // Ex.: callm PlayerPuppet.IsDead()  |  callm Vector4.Length(...)
        ["callm", rest @ ..] => {
            let line = rest.join(" ");
            match parse_chamada_colada(&line) {
                None => crate::log_dev!("[callm] '{line}' não é Namespace.Method(args)"),
                Some(call) => unsafe {
                    let args: Vec<crate::rtti::Arg> = call
                        .args
                        .iter()
                        .map(|a| match a {
                            ArgColado::Str(s) => crate::rtti::Arg::Str(s.clone()),
                            ArgColado::Int(i) => crate::rtti::Arg::I32(*i as u32),
                            ArgColado::Float(f) => crate::rtti::Arg::F32(*f as f32),
                            ArgColado::Bool(b) => crate::rtti::Arg::Bool(*b),
                            ArgColado::Ident(s) => crate::rtti::Arg::CName(crate::cname::cname(s)),
                        })
                        .collect();
                    let fmt = |r: [u8; 32]| {
                        let w = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
                        format!("i32 {} / u32 {:#x} / f32 {}", w as i32, w, f32::from_bits(w))
                    };
                    // 1) tenta como método na classe `namespace` (chama no player)
                    if let Some(rf) = crate::rtti::resolve_func(reg, &call.namespace, &call.method) {
                        match crate::rtti::call_func(&rf, player, &args) {
                            Some(r) => {
                                crate::log_dev!("[callm] {}.{}(...) [método] -> {}", call.namespace, call.method, fmt(r));
                                crate::rtti::soltar_retorno(&rf, &r);
                            }
                            None => crate::log_dev!("[callm] {}.{} [método] não completou", call.namespace, call.method),
                        }
                    } else {
                        // 2) tenta como global `method`
                        let f = crate::register::get_function(reg, &call.method);
                        if crate::rtti::sane(f) {
                            let rf = crate::rtti::ResolvedFn { func: f, ret_type: crate::rtti::ret_type_of(f), is_static: true };
                            match crate::rtti::call_func(&rf, std::ptr::null_mut(), &args) {
                                Some(r) => {
                                    crate::log_dev!("[callm] {}(...) [global] -> {}", call.method, fmt(r));
                                    crate::rtti::soltar_retorno(&rf, &r);
                                }
                                None => crate::log_dev!("[callm] {} [global] não completou", call.method),
                            }
                        } else {
                            crate::log_dev!("[callm] {}.{} não resolveu (nem método da classe nem global)", call.namespace, call.method);
                        }
                    }
                }
            }
            return;
        }
        // Ferramenta de chamada: `sig <Class> <method>` imprime a assinatura (nº params + tipos +
        // tipo de retorno) sem chamar nada — útil pra descobrir a forma antes de arriscar `call`.
        // Reusa param_count/fn_param_type + o `ret_type` (IType) que o resolve_func já devolve + resolve_cname (hash->nome).
        #[cfg(feature = "devtools")]
        ["sig", class, method] => {
            unsafe {
                match crate::rtti::resolve_func(reg, class, method) {
                    Some(rf) => {
                        let n = crate::rtti::param_count(&rf) as usize;
                        let ret_ty = rf.ret_type;
                        let ret_name = if ret_ty.is_null() {
                            "Void".to_string()
                        } else {
                            crate::cname::resolve_cname(crate::rtti::type_name_getname(ret_ty))
                        };
                        let params: Vec<String> = (0..n)
                            .map(|i| crate::cname::resolve_cname(crate::rtti::fn_param_type(rf.func, i)))
                            .collect();
                        crate::log_dev!(
                            "[sig] {class}.{method}({}) -> {ret_name}  (static={}, params={n})",
                            params.join(", "),
                            rf.is_static
                        );
                    }
                    None => crate::log_dev!("[sig] {class}.{method} não resolveu (classe ou método não achado)"),
                }
            }
            return;
        }
        // Ferramenta de chamada/ponte de reflexão do jogo: `call <Class> <method> [args]` — chama
        // por NOME DE CLASSE (namespace), ctx=null (chamada estática). Cobre o padrão mais comum
        // do `Game.*` da API Lua (TDBID.*, ItemID.*, GameInstance.Get*System, Cast<>, etc — funções de
        // classe/utilitário, não métodos de INSTÂNCIA arbitrária — pra isso já existem `callf`
        // (objeto capturado) e `callon` (ponteiro explícito), ambos provados). Resolve via
        // resolve_func (reg.class_by_name + resolve_in_class, já provados) + auto-marshalling de
        // args (parse_cmd_arg, o mesmo de callf/callg).
        ["call", class, method, raw_args @ ..] => {
            unsafe {
                match crate::rtti::resolve_func(reg, class, method) {
                    Some(rf) => {
                        let args: Vec<crate::rtti::Arg> = raw_args.iter().map(|a| parse_cmd_arg(a)).collect();
                        // FIX 2026-07-16: método de INSTÂNCIA precisa do `this` (o player); passar
                        // null crashava (deref de null no corpo do método, ex. IsDead). Só static
                        // usa null. `sig` já expõe `static=`; aqui reusamos `rf.is_static`.
                        let ctx = if rf.is_static { std::ptr::null_mut() } else { player };
                        match crate::rtti::call_func(&rf, ctx, &args) {
                            Some(r) => {
                                let f = |i: usize| f32::from_bits(u32::from_le_bytes([r[i], r[i + 1], r[i + 2], r[i + 3]]));
                                crate::log_dev!(
                                    "[call] {class}.{method}({}) -> i32 {} / u32 {:#x} / f32 {} / bytes {:02x?}",
                                    raw_args.join(" "),
                                    i32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                    u32::from_le_bytes([r[0], r[1], r[2], r[3]]),
                                    f(0),
                                    &r[..8]
                                );
                                crate::rtti::soltar_retorno(&rf, &r);
                            }
                            None => crate::log_dev!("[call] {class}.{method}({}) não completou (void, ctx exigido, ou falha)", raw_args.join(" ")),
                        }
                    }
                    None => crate::log_dev!("[call] método '{class}.{method}' não achado"),
                }
            }
            return;
        }
        // 2026-09-11: estado do hook do `ProcessInputEvents` e injeção de tecla direto na fila do motor (sem o SO).
        #[cfg(feature = "devtools")]
        ["rawinputstat"] => {
            log_veredito(&crate::selftest::rawinput_estatistica());
            return;
        }
        // Crash de load, AL-1 (2026-09-11): allocator que o motor usa pra soltar cada classe entregue ao script, contra o `PoolDefault` do `new_object`.
        #[cfg(feature = "devtools")]
        ["alocadores"] => {
            log_veredito(&unsafe { crate::selftest::alocadores_medir() });
            return;
        }
        // Crash de load, AL-2a (2026-09-11): size/holder/flags/align declarados das classes forjadas, das subclasses reds e da doadora.
        #[cfg(feature = "devtools")]
        ["tamanhos"] => {
            log_veredito(&unsafe { crate::selftest::tamanhos_medir() });
            return;
        }
        // Crash de load, AL-2 (2026-09-12): `n` ciclos de criação + soltura pelo motor de uma classe (só com a AL-2c e o hook da destruição).
        #[cfg(feature = "devtools")]
        ["al2ciclos", classe, n] => {
            log_veredito(&unsafe { crate::selftest::al2_ciclos(classe, n.parse::<usize>().unwrap_or(0)) });
            return;
        }
        // `strtest` (2026-09-12): String longa pelo motor via `call_func` e vtable do tipo `String`.
        #[cfg(feature = "devtools")]
        ["strtest"] => {
            log_veredito(&unsafe { crate::diagnostico::strtest_mecanismo() });
            return;
        }
        // `rawinputsoltos` (2026-09-12): contadores da soltura do handle dos eventos da fila.
        #[cfg(feature = "devtools")]
        ["rawinputsoltos"] => {
            log_veredito(&unsafe { crate::selftest::rawinput_soltos_resumo() });
            return;
        }
        #[cfg(feature = "devtools")]
        ["rawinputaxis", kc, valor] => {
            let tecla = kc.parse::<u32>().unwrap_or(0);
            match (tecla, valor.parse::<f32>()) {
                (t, Ok(v)) if t != 0 => {
                    crate::selftest::rawinput_pedir_eixo(t, v);
                    crate::veredito_dev!("[rawinputinject] pedido eixo tecla={t} valor={v:.2} (entra na próxima chamada do hook)");
                }
                _ => crate::veredito_dev!("[rawinputinject] uso: rawinputaxis <EInputKey> <valor> (ex.: 150 0.8 = gatilho esquerdo)"),
            }
            return;
        }
        #[cfg(feature = "devtools")]
        ["rawinputinject", kc] => {
            let tecla = kc.parse::<u32>().unwrap_or(0);
            if tecla == 0 {
                crate::veredito_dev!("[rawinputinject] uso: rawinputinject <EInputKey> (ex.: 124 = IK_F13)");
            } else {
                crate::selftest::rawinput_pedir_injecao(tecla);
                crate::veredito_dev!("[rawinputinject] pedido tecla={tecla} (entra na próxima chamada do hook)");
            }
            return;
        }
        // Captura DINÂMICA de LR em InkSystem::Get(): traz a janela do jogo pra
        // FRENTE via `[NSApp activateIgnoringOtherApps:YES]` (mesma receita/permissão de
        // `presskey`/`mousedelta` — DENTRO do processo do jogo, não externo). Lição já documentada
        // (2026-08-11, item 45, `GetInventoryPuppet`): `presskey` sem foco de janela é NO-OP silencioso
        // (o `CGEventPost` GLOBAL só é postado quando `game_is_frontmost()==true`) — chamar ISTO
        // antes de qualquer `presskey` nesta investigação evita repetir esse bug.
        #[cfg(feature = "devtools")]
        ["focusgame"] => {
            unsafe { terminal::overlay::force_game_frontmost() };
            crate::log_dev!("[focusgame] force_game_frontmost() chamado");
            return;
        }
        // Prova de pixel da LUT: seta o preset de LUT por comando (sem depender de F2 nem de clique
        // na aba) — `lut 0` (off) / `lut 3` (P&B) / etc. Mesmo write atômico do clique na aba "LUT".
        ["lut", n] => {
            let idx = n.parse::<u32>().unwrap_or(0);
            terminal::overlay::set_lut_preset(idx);
            crate::log_dev!("[lut] preset setado via comando -> {idx}");
            return;
        }
        // input cru com nome real (2026-07-18) — dispara UM keyDown+keyUp sintético via CGEvent
        // (`cg_press`), pra testar o RawInput controller EM GAMEPLAY. Só existe com
        // `--features autoproceed` (dev); build público não importa
        // CGEventPost/CreateKeyboardEvent. `presskey 49` = SPACE.
        //
        // NÃO MIGRADO pro canal do heartbeat (2026-08-14, decisão desta rodada): `cg_press`
        // (mesmo sendo "thread-safe" no sentido de não tocar RTTI/VM do jogo, ver comentário em
        // `overlay.rs`) INJETA um evento de input REAL — mutação com efeito genuíno no jogo
        // vivo, categoria de risco diferente de um drain de ring-buffer. Rodar via heartbeat
        // (a) perderia o gate `PHASE_REACHED_5 && !exec_nested()` deste canal — o jogo poderia
        // não estar pronto pra receber input com segurança ainda; (b) chamaria
        // `CGEventSourceCreate`/`CGEventPost` (e `focusgame`/`force_game_frontmost` chamaria
        // `NSApplication.activateIgnoringOtherApps:`) de uma thread Rust própria cuja relação com
        // a main-thread AppKit do processo nunca foi verificada — AppKit não garante
        // thread-safety fora da main thread, risco novo nunca testado neste projeto. Mesma
        // categoria de cautela já aplicada ao `checkreshook` (patch de código executável também
        // ficou de fora da migração). `presskey`/`focusgame`/`mousedelta`/`mouseclick` continuam
        // só no canal do executor (game thread).
        #[cfg(feature = "autoproceed")]
        ["presskey", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { terminal::overlay::cg_press(keycode) };
            // Canal de veredito com o foco: o `cg_press` só posta no stream global com o jogo em primeiro plano (boots 43b e 44: a tecla não chegou à fila).
            let frente = unsafe { terminal::overlay::game_is_frontmost() };
            crate::veredito_dev!("[presskey] disparado keyDown+keyUp sintético, keycode={keycode} frontmost={frente}");
            return;
        }
        // `keydown`/`keyup` — metades ISOLADAS de `presskey` (ver `terminal::overlay::cg_key_event`),
        // pra permitir HOLD sustentado (ex. andar pra frente) controlado por um processo
        // EXTERNO (bridge de IA por visão) via 2 comandos + sleep do lado de FORA, sem bloquear
        // a thread do jogo. `automacao-mundo` (2026-08-17).
        #[cfg(feature = "autoproceed")]
        ["keydown", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { terminal::overlay::cg_key_event(keycode, true) };
            crate::log_dev!("[keydown] disparado, keycode={keycode}");
            return;
        }
        #[cfg(feature = "autoproceed")]
        ["keyup", kc] => {
            let keycode = kc.parse::<u16>().unwrap_or(49);
            unsafe { terminal::overlay::cg_key_event(keycode, false) };
            crate::log_dev!("[keyup] disparado, keycode={keycode}");
            return;
        }
        // Mod real ponta a ponta/prova visual do link (2026-07-19): navegação de menu por MOUSE via
        // CGEvent DELTA relativo, disparado de DENTRO do processo do jogo (mesma permissão/receita
        // de `presskey`/`cg_press` — evita o gate de Acessibilidade que bloqueia um processo
        // externo). `mousedelta <dx> <dy>` move o cursor RENDERIZADO PELO JOGO (não o do SO).
        #[cfg(feature = "autoproceed")]
        ["mousedelta", dx, dy] => {
            let dx = dx.parse::<i64>().unwrap_or(0);
            let dy = dy.parse::<i64>().unwrap_or(0);
            unsafe { terminal::overlay::cg_mouse_delta(dx, dy) };
            crate::log_dev!("[mousedelta] disparado dx={dx} dy={dy}");
            return;
        }
        // `mouseclick` — mouseDown+mouseUp do botão esquerdo na posição atual (pós-mousedelta).
        #[cfg(feature = "autoproceed")]
        ["mouseclick"] => {
            unsafe { terminal::overlay::cg_click() };
            crate::log_dev!("[mouseclick] disparado mouseDown+mouseUp");
            return;
        }
        // `CallbackLifetime.Session` (2026-08-11): dispara "Session/End" manualmente (mesmo
        // GameSessionEvent real que o despawn de player já usa em lib.rs) — testa o sweep sem
        // precisar navegar até o menu de verdade. Diagnóstico dev-only, seguro (mesma função já
        // provada 2x em produção: despawn + clean-exit).
        #[cfg(feature = "devtools")]
        ["firesessionend"] => {
            let n = unsafe { inicio::natives::disparar_evento_de_sessao("Session/End", false, true) };
            crate::log_dev!("[firesessionend] Session/End disparado manualmente -> {n} callback(s)");
            return;
        }
        // Sobreposição de função (não-Lua, `scripts::sobrepor_funcao`, 2026-08-11): registra Before+After
        // no método de teste `PlayerPuppet.OnFnOverrideTestTarget` (declarado no .reds de teste
        // temporário). Rodar `callon <ptr> OnFnOverrideTestTarget` antes e depois pra comparar.
        #[cfg(feature = "devtools")]
        ["fnoverridetest"] => {
            let ok = unsafe {
                crate::scripts::sobrepor_funcao::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::scripts::sobrepor_funcao::Phase::Before,
                    fnoverride_test_before,
                ) && crate::scripts::sobrepor_funcao::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::scripts::sobrepor_funcao::Phase::After,
                    fnoverride_test_after,
                )
            };
            crate::log_dev!("[fnoverridetest] registro Before+After -> {ok}");
            return;
        }
        // Adiciona um Replace por cima do Before/After já registrado — só o ÚLTIMO Replace
        // registrado roda (regra V1 documentada em `scripts::sobrepor_funcao`), escreve 999 no aOut.
        #[cfg(feature = "devtools")]
        ["fnoverridereplace"] => {
            let ok = unsafe {
                crate::scripts::sobrepor_funcao::register(
                    reg,
                    "PlayerPuppet",
                    "OnFnOverrideTestTarget",
                    crate::scripts::sobrepor_funcao::Phase::Replace,
                    fnoverride_test_replace,
                )
            };
            crate::log_dev!("[fnoverridereplace] registro Replace -> {ok}");
            return;
        }
        // smoke-test do Pilar 2 (CNamePool::Get): resolve um hash CName -> nome via pool
        // NATIVO. `cname 0x23427ae352f89652` deve dar "GetStatValue"; `cname 0` -> "None".
        #[cfg(feature = "devtools")]
        ["cname", h] => {
            let hash = h
                .strip_prefix("0x")
                .and_then(|x| u64::from_str_radix(x, 16).ok())
                .or_else(|| h.parse::<u64>().ok())
                .unwrap_or(0);
            let name = crate::cname::resolve_cname(hash);
            crate::log_dev!("[cname] {hash:#018x} -> '{name}'");
            return;
        }
        // `CompareSemVerPrerelease`: prova ao vivo do C-ABI v14 exposto em
        // BwmsApi (mesma via que um plugin de 3os chamaria) + do Require() real das 3 fachadas.
        // `semvercmp 1.2.3-rc1 1.2.3` deve dar cmp=-1 (release > pre-release, o bug original).
        #[cfg(feature = "devtools")]
        ["semvercmp", lhs, rhs] => {
            let cmp = unsafe { (crate::mods_do_usuario::api::BWMS_API.compare_semver_prerelease)(
                std::ffi::CString::new(*lhs).unwrap_or_default().as_ptr(),
                std::ffi::CString::new(*rhs).unwrap_or_default().as_ptr(),
            ) };
            let sat = unsafe { (crate::mods_do_usuario::api::BWMS_API.semver_satisfies)(
                std::ffi::CString::new(*rhs).unwrap_or_default().as_ptr(),
                std::ffi::CString::new(*lhs).unwrap_or_default().as_ptr(),
            ) };
            crate::log_dev!(
                "[semvercmp] compare_semver_prerelease('{lhs}','{rhs}')={cmp} | semver_satisfies(required='{rhs}',actual='{lhs}')={sat}"
            );
            return;
        }
        #[cfg(feature = "devtools")]
        ["newobj", class] => {
            crate::log_dev!("[newobj] tentando construir '{class}' ...");
            let p = unsafe { crate::rtti::new_object(reg, class) };
            crate::log_dev!(
                "[newobj] '{class}' -> {:p} (static {:#x}) {}",
                p,
                un_rebase(p),
                if p.is_null() { "NULL (resolve/size falhou, sem crash)" } else { "OK (nao crashou)" }
            );
            return;
        }
        // Prova da API de reflexão de classe: Reflection.GetClass + probe de props sem file gate
        #[cfg(feature = "devtools")]
        ["refltest"] => {
            unsafe {
                let cls = crate::rtti::class_of(player);
                log(&crate::rtti::reflection_probe_cls(cls, "refltest:player-class", player));
            }
            return;
        }
        // Prova das classes de evento-alvo: KeyInputEvent.GetKey/GetAction round-trip
        #[cfg(feature = "devtools")]
        ["keytest"] => {
            unsafe { diagnostico::run_keyinput_test(&reg); }
            return;
        }
        // Prova do construtor de entidade: BwmsEntityAddComponent via dynarray_push_handle16
        #[cfg(feature = "devtools")]
        ["entitytest"] => {
            unsafe { diagnostico::run_entity_builder_test(player); }
            return;
        }
        // Telas de mods de terceiro abertas pelo caminho do mod e conferidas por dentro (`interface/teste_telas_de_mod.rs`).
        #[cfg(feature = "devtools")]
        ["telasmod", resto @ ..] => return crate::interface::teste_telas_de_mod::pedir(resto),
        _ => {}
    }
    // Tradução de linha Lua colada (QoL): códigos colados da internet funcionam SEM Lua.
    // `Game.AddToInventory("Items.X", 5)` -> give | `Game.AddMoney(7777)` -> money.
    if let Some((name, n)) = parse_linha_de_give(cmd) {
        let r = unsafe { roupas_e_armas::give(reg, player, tx, &name, n) };
        match r {
            Some(res) if res[0] != 0 => crate::log_dev!("[console] Lua '{cmd}' -> give {name} x{n} OK"),
            _ => crate::log_dev!("[console] Lua '{cmd}' -> give {name} x{n} FALHOU"),
        }
        return;
    }
    let r = unsafe {
        match parts.as_slice() {
            ["money", n] => roupas_e_armas::give(reg, player, tx, "Items.money", n.parse().unwrap_or(1)),
            ["give", name] => roupas_e_armas::give(reg, player, tx, name, 1),
            ["give", name, n] => roupas_e_armas::give(reg, player, tx, name, n.parse().unwrap_or(1)),
            ["remove", name] => roupas_e_armas::remove(reg, player, tx, name, 1),
            ["remove", name, n] => roupas_e_armas::remove(reg, player, tx, name, n.parse().unwrap_or(1)),
            // `spawnsub <tdbid-or-path> [x y z]` via `CompanionSystem.SpawnSubcharacterOnPosition` (ver
            // `entidades::spawn_subcharacter`). Não testado em boot ainda. A função é `-> Void` (`orphans.script:28135`):
            // o buffer volta zerado mesmo quando o NPC nasce, então a resposta confirma só a chamada, como no `summon`.
            // Sem coordenada, ao lado do V: a origem do mundo fica longe de onde o V está.
            ["spawnsub", name] => {
                let Some(v) = entidades::posicao_do_jogador(player) else {
                    return crate::resposta!("Não consegui ler a posição do V: precisa do V em gameplay. Ou passe x y z.");
                };
                return act("enviado", entidades::spawn_subcharacter(reg, player, name, (v[0] + 2.0, v[1] + 2.0, v[2])).is_some());
            }
            ["spawnsub", name, x, y, z] => {
                let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>()) else {
                    return crate::resposta!("Coordenada inválida: use números, ex.: spawnsub {name} -1290.5 1500.2 22.0");
                };
                return act("enviado", entidades::spawn_subcharacter(reg, player, name, (x, y, z)).is_some());
            }
            _ => {
                // Errar uma letra cai aqui: dizer que o comando não existe é o que separa isso de um comando que rodou calado.
                crate::resposta!("\"{cmd}\" não é um comando conhecido. `ajuda` lista os comandos.");
                return;
            }
        }
    };
    match &r {
        Some(res) if res[0] != 0 => crate::resposta!("feito"),
        // O caminho de falha mais comum é nome de item que não existe. Dizer isso poupa a pessoa de
        // achar que o mod quebrou quando ela só errou o nome.
        _ => crate::resposta!(
            "não consegui. confira o nome do item (ex.: Items.Preset_Lexington_Default) e se você está em gameplay"
        ),
    }
    match r {
        Some(res) if res[0] != 0 => crate::log_dev!("[console] '{cmd}' -> OK (GiveItem ret={})", res[0]),
        Some(res) => crate::log_dev!(
            "[console] '{cmd}' -> NO-OP (GiveItem retornou {} — owner/tx errado?)",
            res[0]
        ),
        None => crate::log_dev!("[console] '{cmd}' -> FALHOU (resolve/from_tdbid/call)"),
    }
}

/// Um argumento tipado de uma linha Lua colada (ponte de reflexão do jogo): a inferência que decide se
/// `42` é int, `1.5` float, `"x"` string, `true` bool, ou um identificador cru (ex.: `TDBID.Create(...)`,
/// nome de enum) a ser resolvido em runtime.
#[derive(Debug, Clone, PartialEq)]
enum ArgColado {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
    Ident(String),
}

/// Uma chamada Lua tokenizada: `Namespace.Method(arg, ...)` (também aceita `:` como separador, ex.:
/// `TweakDB:GetFlat`). A DISPATCH pro RTTI do jogo é in-game; a tokenização+inferência é offline.
#[derive(Debug, Clone, PartialEq)]
struct ChamadaColada {
    namespace: String,
    method: String,
    args: Vec<ArgColado>,
}

/// Separa os args de nível-TOPO respeitando aspas (`"`/`'`, com escape `\`) e profundidade de
/// parênteses/colchetes — `Game.X(ItemID.FromTDBID(TDBID.Create("a")), 1)` vira 2 args, não 4.
fn split_top_level_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            cur.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => {
                out.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    let last = cur.trim();
    if !last.is_empty() {
        out.push(last.to_string());
    }
    out
}

/// Infere o tipo de UM token de argumento da linha colada.
fn inferir_arg(tok: &str) -> ArgColado {
    let t = tok.trim();
    let quoted = t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')));
    if quoted {
        return ArgColado::Str(t[1..t.len() - 1].to_string());
    }
    match t {
        "true" => return ArgColado::Bool(true),
        "false" => return ArgColado::Bool(false),
        _ => {}
    }
    if let Ok(i) = t.parse::<i64>() {
        return ArgColado::Int(i);
    }
    if let Ok(f) = t.parse::<f64>() {
        return ArgColado::Float(f);
    }
    ArgColado::Ident(t.to_string())
}

/// Tokeniza uma linha Lua genérica `Namespace.Method(arg, ...)` → [`ChamadaColada`]. `None` se não parece
/// uma chamada (sem `(`/`)` casados ou sem `Namespace.Method`). Base offline pra colar QUALQUER
/// linha Lua no build 0%-Lua (a dispatch pro jogo é o passo in-game).
fn parse_chamada_colada(line: &str) -> Option<ChamadaColada> {
    let s = line.trim();
    let open = s.find('(')?;
    let head = s[..open].trim();
    let inner = s[open + 1..].trim_end().strip_suffix(')')?;
    let sep = head.rfind(['.', ':'])?;
    let namespace = head[..sep].trim().to_string();
    let method = head[sep + 1..].trim().to_string();
    if namespace.is_empty() || method.is_empty() {
        return None;
    }
    let args = split_top_level_args(inner)
        .iter()
        .map(|a| inferir_arg(a))
        .collect();
    Some(ChamadaColada { namespace, method, args })
}

/// Linha Lua → (item, qtd), o par que o comando `give` do BWMS entende (build 0%-Lua). Aceita
/// `Game.AddToInventory("Items.X", 5)` (aspas '/"', qtd opcional=1) e `Game.AddMoney(7777)` →
/// ("Items.money", 7777). None = não é uma dessas duas. Construído sobre [`parse_chamada_colada`]
/// (o tokenizer genérico).
fn parse_linha_de_give(cmd: &str) -> Option<(String, u32)> {
    let call = parse_chamada_colada(cmd)?;
    if call.namespace != "Game" {
        return None;
    }
    match (call.method.as_str(), call.args.as_slice()) {
        ("AddMoney", [ArgColado::Int(n)]) => Some(("Items.money".to_string(), *n as u32)),
        ("AddToInventory", [ArgColado::Str(name)]) if !name.is_empty() => Some((name.clone(), 1)),
        ("AddToInventory", [ArgColado::Str(name), ArgColado::Int(q)]) if !name.is_empty() => {
            Some((name.clone(), *q as u32))
        }
        _ => None,
    }
}

#[cfg(test)]
mod arg_marshalling_tests {
    use super::{parse_bool_flex, parse_cmd_arg_checked, parse_u32_flex, parse_u64_flex};
    use crate::rtti::Arg;

    #[test]
    fn int_flex_aceita_hex_dec_e_sinal() {
        assert_eq!(parse_u32_flex("255"), Some(255));
        assert_eq!(parse_u32_flex("0xFF"), Some(255));
        assert_eq!(parse_u32_flex("0xff"), Some(255));
        assert_eq!(parse_u32_flex("-1"), Some(0xFFFF_FFFF)); // i32 -1 → u32
        assert_eq!(parse_u32_flex("4000000000"), Some(4_000_000_000)); // > i32::MAX
        assert_eq!(parse_u32_flex("0xFFFFFFFF"), Some(0xFFFF_FFFF));
        assert_eq!(parse_u32_flex("nope"), None);
        assert_eq!(parse_u64_flex("0x1_0000_0000".replace('_', "").as_str()), Some(0x1_0000_0000));
    }

    #[test]
    fn bool_flex_tolerante_case_insensitive() {
        for t in ["true", "TRUE", "True", "1", "yes", "Y", "on"] {
            assert_eq!(parse_bool_flex(t), Some(true), "{t}");
        }
        for f in ["false", "FALSE", "0", "no", "off"] {
            assert_eq!(parse_bool_flex(f), Some(false), "{f}");
        }
        assert_eq!(parse_bool_flex("maybe"), None);
    }

    #[test]
    fn checked_prefixo_desconhecido_erra_nao_zera() {
        // O CERNE do fix: um prefixo com typo NÃO pode virar I32(0) mudo dentro de uma chamada viva.
        assert!(parse_cmd_arg_checked("ii:5").is_err());
        assert!(parse_cmd_arg_checked("x:1").is_err());
        assert!(parse_cmd_arg_checked("i:notanint").is_err());
        assert!(parse_cmd_arg_checked("b:maybe").is_err());
        assert!(parse_cmd_arg_checked("semprefixo_nem_int").is_err());
    }

    #[test]
    fn checked_tipos_corretos() {
        match parse_cmd_arg_checked("i:0x10").unwrap() {
            Arg::I32(v) => assert_eq!(v, 16),
            _ => panic!("esperava I32"),
        }
        match parse_cmd_arg_checked("f:1.5").unwrap() {
            Arg::F32(v) => assert_eq!(v, 1.5),
            _ => panic!("esperava F32"),
        }
        match parse_cmd_arg_checked("b:TRUE").unwrap() {
            Arg::Bool(v) => assert!(v),
            _ => panic!("esperava Bool"),
        }
        match parse_cmd_arg_checked("e:0xFF").unwrap() {
            Arg::Enum(v) => assert_eq!(v, 255),
            _ => panic!("esperava Enum"),
        }
        match parse_cmd_arg_checked("s:hello:world").unwrap() {
            Arg::Str(v) => assert_eq!(v, "hello:world"), // '/' e ':' preservados no valor
            _ => panic!("esperava Str"),
        }
        match parse_cmd_arg_checked("42").unwrap() {
            Arg::I32(v) => assert_eq!(v, 42), // i32 cru sem prefixo
            _ => panic!("esperava I32"),
        }
    }
}

#[cfg(test)]
mod chamada_colada_tests {
    use super::{inferir_arg, parse_chamada_colada, parse_linha_de_give, ArgColado};
    #[test]
    fn traduz_linhas_cet() {
        assert_eq!(
            parse_linha_de_give(r#"Game.AddToInventory("Items.Preset_Yasha_Default", 1)"#),
            Some(("Items.Preset_Yasha_Default".into(), 1))
        );
        assert_eq!(
            parse_linha_de_give("Game.AddToInventory('Items.money', 5000)"),
            Some(("Items.money".into(), 5000))
        );
        assert_eq!(parse_linha_de_give(r#"Game.AddToInventory("Items.X")"#), Some(("Items.X".into(), 1)));
        assert_eq!(parse_linha_de_give("Game.AddMoney(7777)"), Some(("Items.money".into(), 7777)));
        assert_eq!(parse_linha_de_give("give Items.X"), None); // comando nosso ≠ linha Lua
        assert_eq!(parse_linha_de_give(r#"Game.AddToInventory("", 1)"#), None);
    }

    #[test]
    fn tokeniza_chamada_generica() {
        // Namespace.Method + args tipados (int/float/bool/string/ident).
        let c = parse_chamada_colada(r#"Game.SetLevel("StrengthSkill", 20, 3.5, true, gamedataProficiencyType.Combat)"#).unwrap();
        assert_eq!(c.namespace, "Game");
        assert_eq!(c.method, "SetLevel");
        assert_eq!(
            c.args,
            vec![
                ArgColado::Str("StrengthSkill".into()),
                ArgColado::Int(20),
                ArgColado::Float(3.5),
                ArgColado::Bool(true),
                ArgColado::Ident("gamedataProficiencyType.Combat".into()),
            ]
        );
    }

    #[test]
    fn args_aninhados_nao_quebram_no_virgula_interno() {
        // vírgula DENTRO de call aninhada + string com vírgula não devem separar no topo.
        let c = parse_chamada_colada(r#"Game.AddToInventory(ItemID.FromTDBID(TDBID.Create("Items.X")), 1)"#).unwrap();
        assert_eq!(c.args.len(), 2, "2 args no topo, não os internos");
        assert_eq!(c.args[1], ArgColado::Int(1));
        let c2 = parse_chamada_colada(r#"Foo.Bar("a,b,c", 2)"#).unwrap();
        assert_eq!(c2.args, vec![ArgColado::Str("a,b,c".into()), ArgColado::Int(2)]);
    }

    #[test]
    fn separador_dois_pontos_e_sem_args() {
        // `:` como separador (ex.: TweakDB:GetFlat) + chamada sem argumentos.
        let c = parse_chamada_colada("TweakDB:GetFlat()").unwrap();
        assert_eq!((c.namespace.as_str(), c.method.as_str()), ("TweakDB", "GetFlat"));
        assert!(c.args.is_empty());
        // namespace ponto-separado com método no fim.
        let c2 = parse_chamada_colada("Game.GetPlayer()").unwrap();
        assert_eq!((c2.namespace.as_str(), c2.method.as_str()), ("Game", "GetPlayer"));
    }

    #[test]
    fn nao_e_chamada() {
        assert_eq!(parse_chamada_colada("give Items.X"), None); // sem parênteses
        assert_eq!(parse_chamada_colada("Foo(1)"), None); // sem Namespace.Method
        assert_eq!(parse_chamada_colada("Game.Foo(1"), None); // parêntese não fechado
    }

    #[test]
    fn inferencia_de_tipo() {
        assert_eq!(inferir_arg("42"), ArgColado::Int(42));
        assert_eq!(inferir_arg("-7"), ArgColado::Int(-7));
        assert_eq!(inferir_arg("3.14"), ArgColado::Float(3.14));
        assert_eq!(inferir_arg("true"), ArgColado::Bool(true));
        assert_eq!(inferir_arg("false"), ArgColado::Bool(false));
        assert_eq!(inferir_arg("\"hi\""), ArgColado::Str("hi".into()));
        assert_eq!(inferir_arg("'hi'"), ArgColado::Str("hi".into()));
        assert_eq!(inferir_arg("EnumName.Value"), ArgColado::Ident("EnumName.Value".into()));
    }
}

#[cfg(test)]
mod null_vtable_guard_tests {
    use super::null_vtable_is_anomalous;

    // O caso que a assinatura de crash desta sessão descreve: objeto polimórfico com o ponteiro
    // de vtable zerado. Qualquer despacho virtual nele lê `[0 + slot]` e falta no offset do slot.
    #[test]
    fn objeto_polimorfico_com_vtable_zerada_e_anomalia() {
        assert!(null_vtable_is_anomalous(true, 0));
    }

    #[test]
    fn objeto_polimorfico_com_vtable_valida_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(true, 0x1_0472_d218));
    }

    // A razão de a checagem NÃO poder ser um `*obj == 0` cego: num struct puro o offset 0 é dado.
    // `Vector4 { x: 0.0, .. }` tem a primeira word zerada e é perfeitamente válido — acusá-lo
    // encheria o log de falso-positivo em cima do caminho mais comum de construção de struct.
    #[test]
    fn struct_puro_com_primeira_word_zerada_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(false, 0));
    }

    #[test]
    fn struct_puro_com_primeira_word_nao_zerada_tambem_nao_e_anomalia() {
        assert!(!null_vtable_is_anomalous(false, 0x3f80_0000));
    }
}

#[cfg(test)]
mod primeiras_tests {
    use super::primeiras;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn verdadeiro_ate_o_teto_e_conta_depois_dele() {
        let c = AtomicU64::new(0);
        assert_eq!([primeiras(&c, 2), primeiras(&c, 2), primeiras(&c, 2), primeiras(&c, 2)], [true, true, false, false]);
        assert_eq!(c.load(Ordering::Relaxed), 4);
    }
}
