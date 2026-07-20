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
//! (`selfboot::autocontinue_suppressed_stale_boot`) entra só como CONTEXTO no relatório, não é gate.
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

use crate::cet_json::Value;

/// Repo público de issues (identidade pública do projeto — traceless). Igual ao `bwms-report.command`.
const ISSUES_URL: &str = "https://github.com/Blackwall-sys/black-wall-mod-system/issues";
/// Só considera um `.ips` desta última ~1h (senão é de outra sessão antiga, não deste boot/crash).
const IPS_MAX_AGE_SECS: u64 = 3600;

/// Chamado do `on_load` logo após `check_stale_boot_attempt`. BARATO quando não houve crash novo:
/// varre o DiagnosticReports 1x, e se o `.ips` mais novo já foi reportado (ou não há nenhum recente),
/// retorna sem escrever. Ver o cabeçalho do módulo (trigger modo-independente + dedup).
pub(crate) fn write_crash_report_if_crashed() {
    // TRIGGER: existe um `.ips` RECENTE (<=1h) do Cyberpunk? Um `.ips` é um log de crash → sua mera
    // existência JÁ significa "houve um crash", em QUALQUER modo (0/1/2) e em QUALQUER build (inclusive
    // Unknown, onde o dead-man's switch nunca arma). O bound de 1h evita que um `.ips` velho de antes
    // do BWMS ser instalado gere um relatório espúrio no 1º launch.
    let ips = match newest_recent_ips() {
        Some(p) => p,
        None => return, // nenhum crash recente do Cyberpunk — nada a fazer (barato/silencioso)
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
    let text = build_report(&ips);
    if write_report_file(&text) {
        // Marca ESTE `.ips` como reportado — SÓ após escrever com sucesso, pra uma falha transitória
        // (ex.: red4ext não achado) poder tentar de novo no próximo boot em vez de perder o crash.
        write_last_reported(&basename);
        crate::log(&format!(
            "[crashreport] novo crash reportado ({basename}) — marcador atualizado"
        ));
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
fn build_report(ips: &std::path::Path) -> String {
    let sig = crash_signature(ips);
    let build = build_line();
    let (os, model, chip, ram) = machine_info();
    let markers = skip_markers();
    // CONTEXTO (não é gate): o dead-man's switch sobreviveu = o boot anterior disparou o lever
    // (skip "até a gameplay") e NÃO chegou ao exit() limpo. "no" = ou saiu limpo, ou o lever nunca
    // armou (modo 0 / build não-suportado) — nesses o `.ips` presente já prova que crashou.
    let unclean = if crate::selfboot::autocontinue_suppressed_stale_boot() {
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
    block.push_str(&format!("Skip mode markers: {markers}\n"));
    block.push_str(&format!("Prior boot unclean exit (dead-man switch): {unclean}\n"));
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
    doc.push_str(
        "BWMS wrote this file because the last game session did not close cleanly (likely a crash) \
         and a recent macOS crash log for Cyberpunk was found. Nothing is sent automatically.\n\n",
    );
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
    let dir = match red4ext_dir() {
        Some(d) => d,
        None => {
            crate::log("[crashreport] não achei a pasta red4ext (dladdr falhou) — relatório não escrito");
            return false;
        }
    };
    let path = format!("{dir}/bwms-crash-report.md");
    match std::fs::write(&path, text) {
        Ok(_) => {
            crate::log(&format!("[crashreport] relatório de crash escrito em {path}"));
            true
        }
        Err(e) => {
            crate::log(&format!("[crashreport] falha ao escrever {path}: {e}"));
            false
        }
    }
}

/// Pasta `red4ext/` = onde a .dylib está carregada. `dylib_dir()` aponta direto pra ela; fallback =
/// o pai de `mods_dir()` (`<red4ext>/blackwall-mods`).
fn red4ext_dir() -> Option<String> {
    if let Some(d) = crate::dylib_dir() {
        return Some(d);
    }
    std::path::Path::new(&crate::mods_dir())
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Linha "Game build: <Steam|GOG|Unknown> — v2.31 supported: <yes|no>". Reusa a detecção de build
/// (prólogo do executor) e o gate golden de versão já existentes.
fn build_line() -> String {
    let build = match crate::game_build() {
        crate::GameBuild::Steam => "Steam",
        crate::GameBuild::Gog => "GOG",
        crate::GameBuild::Unknown => "Unknown",
    };
    let supported = if crate::build_supported() { "yes" } else { "no" };
    format!("{build} — v2.31 supported: {supported}")
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

/// Marcadores do "Pular boot" ligados (`~/.bwms-skipintro`/`-autocontinue`/`-fire-start`) — mostram
/// o modo de boot que estava ativo quando crashou. "none"/"unknown" se nenhum/sem HOME.
fn skip_markers() -> String {
    let home = match std::env::var("HOME") {
        Ok(h) => h,
        Err(_) => return "unknown".into(),
    };
    let p = std::path::Path::new(&home);
    let mut m = Vec::new();
    if p.join(".bwms-skipintro").exists() {
        m.push("skipintro=on");
    }
    if p.join(".bwms-autocontinue").exists() {
        m.push("autocontinue=on");
    }
    if p.join(".bwms-fire-start").exists() {
        m.push("fire-start=on");
    }
    if m.is_empty() {
        "none".into()
    } else {
        m.join(" ")
    }
}

// ===== ASSINATURA DO CRASH (parse do .ips via cet_json, zero-dep) =====

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
    let when_hdr = crate::cet_json::parse(header_line)
        .ok()
        .and_then(|h| get_str(&h, "timestamp"));

    let body = match crate::cet_json::parse(body_line) {
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
    let exc = get(&body, "exception");
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
    let fidx = get(&body, "faultingThread")
        .and_then(as_num)
        .map(|n| n as usize)
        .unwrap_or(0);
    let images = get(&body, "usedImages").and_then(as_arr);
    let mut tname = format!("thread {fidx}");
    let mut frames_out: Vec<String> = Vec::new();
    if let Some(threads) = get(&body, "threads").and_then(as_arr) {
        if let Some(t) = threads.get(fidx) {
            tname = get_str(t, "name")
                .or_else(|| get_str(t, "queue"))
                .unwrap_or_else(|| format!("thread {fidx}"));
            if let Some(frames) = get(t, "frames").and_then(as_arr) {
                for (i, fr) in frames.iter().take(8).enumerate() {
                    frames_out.push(format_frame(i, fr, images));
                }
            }
        }
    }

    let mut out = String::new();
    out.push_str(&format!("  when:      {when}\n"));
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

/// Formata 1 frame: `symbol` presente → "i  <sym> + <symbolLocation>" (com o hash Rust removido);
/// senão → "i  <basename(imageName)> + 0x<imageOffset>".
fn format_frame(i: usize, fr: &Value, images: Option<&Vec<Value>>) -> String {
    if let Some(sym) = get_str(fr, "symbol") {
        let sym = strip_rust_hash(&sym);
        let loc = get(fr, "symbolLocation").and_then(as_num).map(|n| n as i64).unwrap_or(0);
        return format!("    {i}  {sym} + {loc}");
    }
    let ii = get(fr, "imageIndex").and_then(as_num).map(|n| n as i64).unwrap_or(-1);
    let off = get(fr, "imageOffset").and_then(as_num).map(|n| n as u64).unwrap_or(0);
    let name = images
        .and_then(|imgs| if ii >= 0 { imgs.get(ii as usize) } else { None })
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

// ---- acessores mínimos sobre cet_json::Value (sem depender de mais nada) ----

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Obj(m) => m.get(key),
        _ => None,
    }
}
fn get_str(v: &Value, key: &str) -> Option<String> {
    match get(v, key) {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    }
}
fn as_num(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        _ => None,
    }
}
fn as_arr(v: &Value) -> Option<&Vec<Value>> {
    match v {
        Value::Arr(a) => Some(a),
        _ => None,
    }
}

// ===== sysctl FFI (mesmo padrão de selfboot::sysctl_memsize_bytes, estendido p/ string) =====

/// Lê um sysctl STRING por nome (nome tem que ser nul-terminado). None se a chamada falhar ou vazio.
fn sysctl_str(name: &[u8]) -> Option<String> {
    extern "C" {
        fn sysctlbyname(
            name: *const i8,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> i32;
    }
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
    extern "C" {
        fn sysctlbyname(
            name: *const i8,
            oldp: *mut c_void,
            oldlenp: *mut usize,
            newp: *mut c_void,
            newlen: usize,
        ) -> i32;
    }
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
    fn basename_pega_so_o_nome() {
        assert_eq!(basename("/Users/x/Cyberpunk2077.app/Contents/MacOS/Cyberpunk2077"), "Cyberpunk2077");
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
}
