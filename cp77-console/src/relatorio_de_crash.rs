//! `bwms-crash-report.md` — no boot, se apareceu um crash `.ips` NOVO e recente do Cyberpunk, o BWMS
//! ESCREVE ele mesmo um relatório em texto (`red4ext/bwms-crash-report.md`) que o usuário abre, copia
//! e cola numa issue do GitHub. Substitui o `bwms-report.command` (executável assusta usuário de mod
//! já marcado por antivírus — "arquivo que roda = malware").
//!
//! TRIGGER MODO-INDEPENDENTE: um `.ips` É um log de crash → "há um `.ips` novo" = "houve um crash
//! novo", em QUALQUER modo — inclusive modo 0 (sem skip) e build DESCONHECIDO/não-suportado (onde o
//! gate golden faz o jogo bootar vanilla e o dead-man's switch NUNCA arma). Esses dois casos são
//! justamente os que o usuário MAIS quer reportar ("o mod não funciona na minha versão do jogo").
//! Dedup por marcador persistente (`~/.bwms-last-crash-report`, guarda o basename do último `.ips`
//! reportado): reporta 1x por crash, não reescreve o mesmo relatório a cada boot. O dead-man's switch
//! (`inicio::boot::autocontinue_suppressed_stale_boot`) entra só como CONTEXTO no relatório, não é gate.
//!
//! Best-effort e NARROW: qualquer falha (parse/arquivo/permissão) degrada em silêncio ou vira
//! "(unavailable)", NUNCA crasha nem trava o boot. A .dylib é `panic=abort` → código 100% defensivo
//! (sem `unwrap`/indexação que panica). O WRITE do .md acontece no build PÚBLICO também (é o ponto:
//! o usuário precisa do relatório) — só os `crate::log` de diagnóstico são no-op no público.
//!
//! TRACELESS: só lê `Cyberpunk2077*.ips` (o crash do PRÓPRIO jogo, nada mais do DiagnosticReports) e
//! usa só o BASENAME das imagens (nunca vaza o `/Users/<nome>/...` do path). O `$HOME` em runtime é o
//! do usuário — ok. Sem path de dev, sem menção a IA, sem identidade embutida.

use std::ffi::c_void;
use std::time::{Duration, SystemTime};

use bwms_primitives::json::Valor;

/// Repo público de issues (identidade pública do projeto — traceless). Igual ao `bwms-report.command`.
const ISSUES_URL: &str = "https://github.com/Blackwall-sys/black-wall-mod-system/issues";
/// Só considera um `.ips` desta última ~1h (senão é de outra sessão antiga, não deste boot/crash).
const IPS_MAX_AGE_SECS: u64 = 3600;

/// Chamado do `on_load` logo após `check_stale_boot_attempt`. BARATO quando não houve crash novo:
/// varre o DiagnosticReports 1x, e se o `.ips` mais novo já foi reportado (ou não há nenhum recente),
/// retorna sem escrever. Ver o cabeçalho do módulo (trigger modo-independente + dedup).
/// `energia_anterior`: o registro `~/.bwms-boot-energia` que o boot anterior deixou por não chegar ao fim do load nem à saída limpa
/// (`inicio::energia`). Sem `.ips` novo, ele sozinho gera o relatório: travamento e morte sem `.ips` também contam.
pub(crate) fn write_crash_report_if_crashed(energia_anterior: Option<&str>) {
    // TRIGGER: existe um `.ips` RECENTE (<=1h) do Cyberpunk? Um `.ips` é um log de crash → sua mera
    // existência JÁ significa "houve um crash", em QUALQUER modo (0/1/2) e em QUALQUER build (inclusive
    // Unknown, onde o dead-man's switch nunca arma). O bound de 1h evita que um `.ips` velho de antes
    // do BWMS ser instalado gere um relatório espúrio no 1º launch.
    let ips = match newest_recent_ips() {
        Some(p) => p,
        None => {
            if let Some(registro) = energia_anterior {
                relatar_boot_sem_ips(registro);
            }
            return;
        }
    };
    let basename = ips
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    if basename.is_empty() {
        return;
    }
    // DEDUP: reporta 1x por crash. Se o `.ips` mais novo == o último já reportado (marcador persistente
    // `~/.bwms-last-crash-report`), não reescreve o mesmo relatório a cada boot. Cada crash gera um
    // `.ips` de nome único (timestamp) → basename novo = crash novo.
    if !is_unreported(&basename, read_last_reported().as_deref()) {
        return;
    }
    let text = build_report(Some(&ips), energia_anterior);
    if write_report_file(&text) {
        // Marca ESTE `.ips` como reportado — SÓ após escrever com sucesso, pra uma falha transitória
        // (ex.: red4ext não achado) poder tentar de novo no próximo boot em vez de perder o crash.
        write_last_reported(&basename);
        crate::log_dev!(
            "[crashreport] novo crash reportado ({basename}) — marcador atualizado"
        );
    }
}

/// Relatório do boot que parou sem `.ips`: 1 por registro (dedup pelo `inicio=` do registro no mesmo marcador dos `.ips`).
fn relatar_boot_sem_ips(registro: &str) {
    let chave = format!("energia-{}", registro.split_whitespace().next().unwrap_or("?"));
    if !is_unreported(&chave, read_last_reported().as_deref()) {
        return;
    }
    if write_report_file(&build_report(None, Some(registro))) {
        write_last_reported(&chave);
        crate::log_dev!("[crashreport] boot anterior sem .ips reportado ({chave})");
    }
}

/// Chegou um `.ips` NÃO reportado? true se o mais novo difere do último reportado (ou não há último).
/// Pura (testável) — a decisão de dedup sem tocar disco.
fn is_unreported(newest: &str, last_reported: Option<&str>) -> bool {
    match last_reported {
        Some(l) => l != newest,
        None => true,
    }
}

/// Marcador persistente do último `.ips` já reportado. Fica em `$HOME` (sobrevive entre boots, junto
/// da família `~/.bwms-*`). Sem HOME não há dedup — mas `newest_recent_ips` também depende de HOME
/// (DiagnosticReports mora lá), então nesse caso já teríamos retornado antes.
fn last_reported_marker_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::Path::new(&h).join(".bwms-last-crash-report"))
}
fn read_last_reported() -> Option<String> {
    let p = last_reported_marker_path()?;
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}
fn write_last_reported(basename: &str) {
    if let Some(p) = last_reported_marker_path() {
        let _ = std::fs::write(p, basename);
    }
}

/// Diretórios de crash-report a varrer: `DiagnosticReports/` E o subdir `Retired/` (macOS novo move
/// relatórios pra lá). Ordem irrelevante — pegamos o mais novo de todos.
fn diagnostic_dirs() -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let base = std::path::Path::new(&home).join("Library/Logs/DiagnosticReports");
        v.push(base.join("Retired"));
        v.push(base);
    }
    v
}

/// `Cyberpunk2077*.ips` mais recente (por mtime) e novo o suficiente (<= ~1h). NARROW: só o prefixo
/// do jogo, nada mais. None = nenhum recente.
fn newest_recent_ips() -> Option<std::path::PathBuf> {
    let now = SystemTime::now();
    let mut best: Option<(SystemTime, std::path::PathBuf)> = None;
    for dir in diagnostic_dirs() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(_) => continue, // pasta pode não existir (Retired) — segue
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if !(name.starts_with("Cyberpunk2077") && name.ends_with(".ips")) {
                continue;
            }
            let mtime = match ent.metadata().and_then(|m| m.modified()) {
                Ok(t) => t,
                Err(_) => continue,
            };
            // idade <= 1h. `duration_since` dá Err se o mtime estiver no futuro (relógio) → trata fresco.
            let fresh = now
                .duration_since(mtime)
                .map(|d| d <= Duration::from_secs(IPS_MAX_AGE_SECS))
                .unwrap_or(true);
            if !fresh {
                continue;
            }
            let better = match &best {
                Some((bt, _)) => mtime > *bt,
                None => true,
            };
            if better {
                best = Some((mtime, path));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// Monta o documento .md inteiro (instrução curta + bloco cercado pra copiar + URL). Nunca falha —
/// se a assinatura do crash não puder ser lida, `crash_signature` degrada com uma nota e o resto vai.
/// `ips = None`: o boot anterior parou sem `.ips` (travou, foi fechado à força ou morreu sem log) e só o registro de energia sobrou.
fn build_report(ips: Option<&std::path::Path>, energia_anterior: Option<&str>) -> String {
    let sig = match ips {
        Some(p) => crash_signature(p),
        None => "  (no .ips: the previous boot stopped before the end of the save load and before a clean exit — hang, forced quit, or a\n   crash that macOS did not log)\n".to_string(),
    };
    let build = build_line();
    let (os, model, chip, ram) = machine_info();
    let modo = modo_de_boot();
    // CONTEXTO (não é gate): o dead-man's switch sobreviveu = o boot anterior disparou o lever
    // (skip "até a gameplay") e NÃO chegou ao exit() limpo. "no" = ou saiu limpo, ou o lever nunca
    // armou (modo 0 / build não-suportado) — nesses o `.ips` presente já prova que crashou.
    let unclean = if crate::inicio::boot::autocontinue_suppressed_stale_boot() {
        "yes"
    } else {
        "no (or skip-boot lever was not armed this path)"
    };

    // Bloco de diagnóstico (mesmo conteúdo do bwms-report.command). É o que o usuário copia.
    let mut block = String::new();
    block.push_str("### BWMS crash report\n");
    block.push_str(&format!("BWMS:         {}\n", crate::BWMS_VERSION));
    block.push_str(&format!("Game build:   {build}\n"));
    block.push_str(&format!(
        "macOS:        {os}      Mac: {model}   Chip: {chip}   RAM: {ram}\n"
    ));
    block.push_str(&format!("Boot mode (bwms.toml): {modo}\n"));
    block.push_str(&format!("Prior boot unclean exit (dead-man switch): {unclean}\n"));
    block.push_str(&format!("Prior boot power (BWMS record): {}\n", energia_anterior.unwrap_or("none (the boot finished or exited cleanly)")));
    block.push('\n');
    block.push_str("Crash signature (most recent Cyberpunk2077 .ips):\n");
    block.push_str(&sig);
    if !sig.ends_with('\n') {
        block.push('\n');
    }
    block.push('\n');
    block.push_str("What I did: <describe in one line what you were doing when it crashed>\n");

    // Documento .md: texto simples + UM bloco cercado (```), sem aninhar fences.
    let mut doc = String::new();
    doc.push_str("# BWMS crash report\n\n");
    doc.push_str(if ips.is_some() {
        "BWMS wrote this file because the last game session did not close cleanly (likely a crash) \
         and a recent macOS crash log for Cyberpunk was found. Nothing is sent automatically.\n\n"
    } else {
        "BWMS wrote this file because the last game start stopped before the save finished loading and did not \
         close cleanly. macOS left no crash log, so this report carries what BWMS recorded at that start. If you \
         closed the game yourself during loading, you can ignore it. Nothing is sent automatically.\n\n"
    });
    doc.push_str("How to report it (takes a minute):\n\n");
    doc.push_str("1. Copy everything inside the box below.\n");
    doc.push_str(&format!("2. Open a new issue: {ISSUES_URL}\n"));
    doc.push_str("3. Paste it there, and replace the \"What I did\" line with what you were doing.\n\n");
    doc.push_str(
        "This file only contains the crash signature and your Mac model/version — no personal \
         data, no file paths, no game files.\n\n",
    );
    doc.push_str("```\n");
    doc.push_str(&block);
    doc.push_str("```\n\n");
    doc.push_str(&format!("Issues: {ISSUES_URL}\n"));
    doc
}

/// Escreve o .md na pasta `red4ext/` (onde a nossa dylib mora — visível/achável pelo usuário). NÃO
/// escreve no Desktop (intrusivo). Loga o path (no-op no público, mas ajuda o dev). true = escreveu
/// (o caller só atualiza o marcador de dedup em caso de sucesso).
fn write_report_file(text: &str) -> bool {
    let dir = match crate::dylib_dir() {
        Some(d) => d,
        None => {
            crate::log_dev!("[crashreport] não achei a pasta red4ext (dladdr falhou) — relatório não escrito");
            return false;
        }
    };
    let path = format!("{dir}/bwms-crash-report.md");
    match std::fs::write(&path, text) {
        Ok(_) => {
            crate::log_dev!("[crashreport] relatório de crash escrito em {path}");
            true
        }
        Err(e) => {
            crate::log_dev!("[crashreport] falha ao escrever {path}: {e}");
            false
        }
    }
}

/// Valor da linha "Game build:" pela detecção do `game_build` (prólogo do executor), a mesma que decide o gate do `on_load`.
fn build_line() -> String {
    build_line_de(crate::game_build()).to_string()
}

fn build_line_de(b: crate::GameBuild) -> &'static str {
    match b {
        crate::GameBuild::Steam => "Steam 2.5.0 — supported: yes",
        crate::GameBuild::Gog => "GOG 2.5.0 — supported: yes",
        crate::GameBuild::Unknown => "not Steam or GOG 2.5.0 — supported: no (BWMS did not activate)",
    }
}

/// (macOS, Mac model, chip, RAM) — tudo via sysctl (sem spawnar processo). Cada campo degrada pra
/// "(unavailable)" sozinho.
fn machine_info() -> (String, String, String, String) {
    let os = macos_line();
    let model = sysctl_str(b"hw.model\0").unwrap_or_else(|| "(unavailable)".into());
    let chip = sysctl_str(b"machdep.cpu.brand_string\0").unwrap_or_else(|| "(unavailable)".into());
    let ram = match sysctl_u64(b"hw.memsize\0") {
        Some(b) if b > 0 => format!("{} GB", b / (1024 * 1024 * 1024)),
        _ => "(unavailable)".into(),
    };
    (os, model, chip, ram)
}

/// "<versão> (<codinome>) <build>" — o equivalente sysctl do `sw_vers` (kern.osproductversion +
/// kern.osversion), sem spawnar processo.
fn macos_line() -> String {
    let ver = match sysctl_str(b"kern.osproductversion\0") {
        Some(v) => v,
        None => return "(unavailable)".into(),
    };
    let mut s = ver.clone();
    let name = macos_codename(&ver);
    if !name.is_empty() {
        s = format!("{ver} ({name})");
    }
    if let Some(b) = sysctl_str(b"kern.osversion\0") {
        if !b.is_empty() {
            s = format!("{s} {b}");
        }
    }
    s
}

/// Codinome do macOS pelo major da versão (mesmo mapa do bwms-report.command). Vazio = major fora do
/// mapa (fica só o número, honesto).
fn macos_codename(ver: &str) -> &'static str {
    let major = ver
        .split('.')
        .next()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    match major {
        26 => "Tahoe",
        15 => "Sequoia",
        14 => "Sonoma",
        13 => "Ventura",
        12 => "Monterey",
        11 => "Big Sur",
        _ => "",
    }
}

/// O modo de boot do `bwms.toml` lido neste boot: é o do boot que caiu, a menos que o arquivo tenha mudado entre os dois.
fn modo_de_boot() -> String {
    let c = crate::configuracao::do_mod::config();
    format!("pular_intro={} splash={} continuar_automatico={}", c.pular_intro, c.splash, c.continuar_automatico)
}

// ===== ASSINATURA DO CRASH (parse do .ips via bwms_primitives::json) =====

/// Extrai a assinatura do crash do `.ips` (2 linhas: header JSON + body JSON). SEMPRE devolve um
/// bloco de texto — se algum passo falhar, degrada pra when/file + nota (nunca vazio, nunca panica).
fn crash_signature(ips: &std::path::Path) -> String {
    let fname = ips
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("?")
        .to_string();
    let content = match std::fs::read_to_string(ips) {
        Ok(c) => c,
        Err(_) => return format!("  (não consegui ler o .ips: {fname})\n"),
    };
    let mut it = content.splitn(2, '\n');
    let header_line = it.next().unwrap_or("");
    let body_line = it.next().unwrap_or("");

    // when: header.timestamp → (fallback) body.captureTime; primeiros 16 chars ("YYYY-MM-DD HH:MM").
    let when_hdr = bwms_primitives::json::ler(header_line)
        .ok()
        .and_then(|h| get_str(&h, "timestamp"));

    let body = match bwms_primitives::json::ler(body_line) {
        Ok(v) => v,
        Err(_) => {
            let when = when_hdr.map(|s| trim16(&s)).unwrap_or_else(|| "?".into());
            return format!(
                "  when:      {when}\n  file:      {fname}\n  (não deu pra ler a assinatura completa deste .ips)\n"
            );
        }
    };
    let when = when_hdr
        .or_else(|| get_str(&body, "captureTime"))
        .map(|s| trim16(&s))
        .unwrap_or_else(|| "?".into());

    // exception: type (signal) / subtype (campos opcionais degradam sozinhos).
    let exc = body.campo("exception");
    let etype = exc.and_then(|e| get_str(e, "type")).unwrap_or_else(|| "(unavailable)".into());
    let esig = exc.and_then(|e| get_str(e, "signal")).unwrap_or_default();
    let esub = exc.and_then(|e| get_str(e, "subtype")).unwrap_or_default();
    let mut exc_line = etype;
    if !esig.is_empty() {
        exc_line.push_str(&format!(" ({esig})"));
    }
    if !esub.is_empty() {
        exc_line.push_str(&format!(" / {esub}"));
    }

    // thread que falhou + top ~8 frames dela.
    let fidx = body.campo("faultingThread").and_then(numero::<usize>).unwrap_or(0);
    let images = body.campo("usedImages").and_then(lista);
    let mut tname = format!("thread {fidx}");
    let mut frames_out: Vec<String> = Vec::new();
    if let Some(threads) = body.campo("threads").and_then(lista) {
        if let Some(t) = threads.get(fidx) {
            tname = get_str(t, "name")
                .or_else(|| get_str(t, "queue"))
                .unwrap_or_else(|| format!("thread {fidx}"));
            if let Some(frames) = t.campo("frames").and_then(lista) {
                for (i, fr) in frames.iter().take(8).enumerate() {
                    frames_out.push(format_frame(i, fr, images));
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str(&format!("  when:      {when}\n"));
    out.push_str(&format!("  power:     {}\n", linha_de_energia_do_ips(&body)));
    out.push_str(&format!("  exception: {exc_line}\n"));
    out.push_str(&format!("  thread:    {tname} (faulting)\n"));
    out.push_str("  top frames:\n");
    if frames_out.is_empty() {
        out.push_str("    (sem frames legíveis)\n");
    } else {
        for f in &frames_out {
            out.push_str(f);
            out.push('\n');
        }
    }
    out
}

/// O `lowPowerMode` que o macOS grava no corpo do `.ips`, cru: o valor, ou `absent` quando a chave não veio (o `.ips` de 2026-07-12 com o
/// modo ligado trazia `"lowPowerMode":1`; sem o modo a chave não aparece nos `.ips` desta máquina). Sem inferência.
fn linha_de_energia_do_ips(body: &Valor) -> String {
    let v = match body.campo("lowPowerMode") {
        Some(Valor::Numero(n)) => n.clone(),
        Some(Valor::Texto(s)) if !s.is_empty() => s.clone(),
        Some(_) => "?".into(),
        None => "absent".into(),
    };
    format!("lowPowerMode={v}")
}

/// Formata 1 frame: `symbol` presente → "i  <sym> + <symbolLocation>" (com o hash Rust removido);
/// senão → "i  <basename(imageName)> + 0x<imageOffset>".
fn format_frame(i: usize, fr: &Valor, images: Option<&[Valor]>) -> String {
    if let Some(sym) = get_str(fr, "symbol") {
        let sym = strip_rust_hash(&sym);
        let loc = fr.campo("symbolLocation").and_then(numero::<i64>).unwrap_or(0);
        return format!("    {i}  {sym} + {loc}");
    }
    let off = fr.campo("imageOffset").and_then(numero::<u64>).unwrap_or(0);
    let name = fr
        .campo("imageIndex")
        .and_then(numero::<usize>)
        .and_then(|ii| images?.get(ii))
        .and_then(|img| get_str(img, "name"))
        .map(|n| basename(&n))
        .unwrap_or_else(|| "?".into());
    format!("    {i}  {name} + 0x{off:x}")
}

/// Remove o sufixo de hash de símbolo Rust (`::h<hex>`) pra legibilidade (ex.: `foo::bar::h1a2b3c` →
/// `foo::bar`). Só corta se o sufixo for `::h` + >=6 dígitos hex até o fim (o padrão do rustc).
fn strip_rust_hash(sym: &str) -> String {
    if let Some(pos) = sym.rfind("::h") {
        let tail = &sym[pos + 3..];
        if tail.len() >= 6 && tail.bytes().all(|b| b.is_ascii_hexdigit()) {
            return sym[..pos].to_string();
        }
    }
    sym.to_string()
}

/// Basename de um path (só o nome do arquivo) — evita vazar o diretório (`/Users/<nome>/...`) no .md.
fn basename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Primeiros 16 chars (o timestamp do .ips vem como "YYYY-MM-DD HH:MM:SS..."). char-safe.
fn trim16(s: &str) -> String {
    s.chars().take(16).collect()
}

// ---- acessores sobre bwms_primitives::json::Valor: chave ausente ou de outro tipo vira None ----

fn get_str(v: &Valor, key: &str) -> Option<String> {
    match v.campo(key) {
        Some(Valor::Texto(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}
/// Converte o texto do número direto no tipo do campo, sem `f64`: offset acima de 2^53 sai exato, e
/// negativo ou fracionário num campo sem sinal vira None.
fn numero<T: std::str::FromStr>(v: &Valor) -> Option<T> {
    match v {
        Valor::Numero(s) => s.parse().ok(),
        _ => None,
    }
}
fn lista(v: &Valor) -> Option<&[Valor]> {
    match v {
        Valor::Lista(itens) => Some(itens),
        _ => None,
    }
}

// ===== sysctl FFI (sysctlbyname, estendido p/ string) =====

/// Lê um sysctl STRING por nome (nome tem que ser nul-terminado). None se a chamada falhar ou vazio.
fn sysctl_str(name: &[u8]) -> Option<String> {
    use crate::macos::sysctlbyname;
    unsafe {
        // 1ª chamada: descobre o tamanho do buffer.
        let mut len: usize = 0;
        let r = sysctlbyname(
            name.as_ptr() as *const i8,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        );
        if r != 0 || len == 0 {
            return None;
        }
        // 2ª chamada: preenche.
        let mut buf = vec![0u8; len];
        let r = sysctlbyname(
            name.as_ptr() as *const i8,
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        );
        if r != 0 {
            return None;
        }
        // string C: corta no NUL terminador e valida UTF-8 (lossy) + trim.
        let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

/// Lê um sysctl u64 por nome (nome nul-terminado). None se falhar ou valor 0.
fn sysctl_u64(name: &[u8]) -> Option<u64> {
    use crate::macos::sysctlbyname;
    unsafe {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let r = sysctlbyname(
            name.as_ptr() as *const i8,
            &mut v as *mut u64 as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        );
        if r == 0 && v > 0 {
            Some(v)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_rust_hash_remove_sufixo() {
        assert_eq!(strip_rust_hash("core::ptr::drop_in_place::h1a2b3c4d"), "core::ptr::drop_in_place");
        // sufixo curto demais (<6 hex) NÃO é hash → preserva
        assert_eq!(strip_rust_hash("foo::hbeef"), "foo::hbeef");
        // sem sufixo de hash → intacto
        assert_eq!(strip_rust_hash("some::sym"), "some::sym");
        // conteúdo não-hex após ::h → preserva
        assert_eq!(strip_rust_hash("foo::hello_world"), "foo::hello_world");
    }

    #[test]
    fn energia_do_ips_le_o_low_power_mode_cru() {
        let corpo = bwms_primitives::json::ler(r#"{"lowPowerMode":1,"faultingThread":0}"#).unwrap();
        assert_eq!(linha_de_energia_do_ips(&corpo), "lowPowerMode=1");
    }

    #[test]
    fn energia_do_ips_sem_a_chave_e_absent() {
        let corpo = bwms_primitives::json::ler(r#"{"faultingThread":0}"#).unwrap();
        assert_eq!(linha_de_energia_do_ips(&corpo), "lowPowerMode=absent");
    }

    #[test]
    fn basename_pega_so_o_nome() {
        assert_eq!(basename("/opt/games/Cyberpunk2077.app/Contents/MacOS/Cyberpunk2077"), "Cyberpunk2077");
        assert_eq!(basename("libcp77_console.dylib"), "libcp77_console.dylib");
    }

    #[test]
    fn trim16_char_safe() {
        assert_eq!(trim16("2026-07-11 23:58:02.00 -0300"), "2026-07-11 23:58");
        assert_eq!(trim16("curto"), "curto");
    }

    #[test]
    fn crash_signature_do_ips_real() {
        // header + body mínimos no formato .ips (2 linhas). Frame sem símbolo (jogo stripado) usa
        // basename(image)+offset; a thread que falhou pega o `name`.
        let header = r#"{"app_name":"Cyberpunk2077","timestamp":"2026-07-11 23:58:02.00 -0300"}"#;
        let body = r#"{"captureTime":"2026-07-11 23:57:38.3735 -0300","exception":{"type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x10"},"faultingThread":0,"threads":[{"name":"redDispatcher7","queue":"com.apple.main-thread","frames":[{"imageOffset":64629344,"imageIndex":0},{"imageOffset":16,"symbol":"do_thing::h001122","symbolLocation":8,"imageIndex":1}]}],"usedImages":[{"name":"Cyberpunk2077","path":"/x/Cyberpunk2077"},{"name":"libcp77_console.dylib"}]}"#;
        let tmp = std::env::temp_dir().join("bwms-crashreport-test.ips");
        std::fs::write(&tmp, format!("{header}\n{body}")).unwrap();
        let sig = crash_signature(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(sig.contains("when:      2026-07-11 23:58"), "when: {sig}");
        assert!(sig.contains("exception: EXC_BAD_ACCESS (SIGSEGV) / KERN_INVALID_ADDRESS at 0x10"), "exc: {sig}");
        assert!(sig.contains("thread:    redDispatcher7 (faulting)"), "thread: {sig}");
        assert!(sig.contains("0  Cyberpunk2077 + 0x3da2a60"), "frame0: {sig}");
        // símbolo com hash Rust removido + symbolLocation
        assert!(sig.contains("1  do_thing + 8"), "frame1: {sig}");
    }

    #[test]
    fn crash_signature_decodifica_escape_u_e_le_offset_exato() {
        // 2^53 + 1 não cabe em f64: por f64 o offset sairia 0x20000000000000.
        let body = r#"{"faultingThread":1,"threads":[{"name":"x"},{"name":"r\u00e9d","frames":[{"imageOffset":9007199254740993,"imageIndex":0},{"imageOffset":16,"imageIndex":-1}]}],"usedImages":[{"name":"Cyberpunk2077"}]}"#;
        let tmp = std::env::temp_dir().join("bwms-crashreport-exato.ips");
        std::fs::write(&tmp, format!("{{}}\n{body}")).unwrap();
        let sig = crash_signature(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(sig.contains("thread:    réd (faulting)"), "thread: {sig}");
        assert!(sig.contains("0  Cyberpunk2077 + 0x20000000000001"), "frame0: {sig}");
        // imageIndex negativo não indexa imagem nenhuma
        assert!(sig.contains("1  ? + 0x10"), "frame1: {sig}");
    }

    #[test]
    fn dedup_is_unreported() {
        // 1º boot (marcador vazio) → reporta
        assert!(is_unreported("Cyberpunk2077-2026-07-19-120000.ips", None));
        // crash NOVO (basename diferente do último) → reporta
        assert!(is_unreported(
            "Cyberpunk2077-2026-07-19-130000.ips",
            Some("Cyberpunk2077-2026-07-19-120000.ips")
        ));
        // mesmo .ips já reportado → NÃO reescreve (dedup)
        assert!(!is_unreported(
            "Cyberpunk2077-2026-07-19-120000.ips",
            Some("Cyberpunk2077-2026-07-19-120000.ips")
        ));
    }

    #[test]
    fn crash_signature_body_invalido_degrada() {
        let tmp = std::env::temp_dir().join("bwms-crashreport-bad.ips");
        std::fs::write(&tmp, "{\"timestamp\":\"2026-01-02 03:04:05\"}\nlixo que nao e json").unwrap();
        let sig = crash_signature(&tmp);
        let _ = std::fs::remove_file(&tmp);
        assert!(sig.contains("when:      2026-01-02 03:04"), "sig: {sig}");
        assert!(sig.contains("assinatura completa"), "sig: {sig}");
    }

    #[test]
    fn build_line_diz_a_versao_suportada() {
        assert_eq!(build_line_de(crate::GameBuild::Steam), "Steam 2.5.0 — supported: yes");
        assert_eq!(build_line_de(crate::GameBuild::Gog), "GOG 2.5.0 — supported: yes");
        assert_eq!(build_line_de(crate::GameBuild::Unknown), "not Steam or GOG 2.5.0 — supported: no (BWMS did not activate)");
        assert!(!build_line_de(crate::GameBuild::Steam).contains("2.31"));
    }
}
