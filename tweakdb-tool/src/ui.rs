//! ui.rs — servidor HTTP local (zero-dep, só `std::net`) que serve uma GUI web
//! pro tweakdb-tool, para modding por LEIGOS sem linha de comando. O browser é a
//! GUI; os handlers chamam os módulos do próprio crate (sem subprocess). O
//! tweakdb e os nomes são carregados UMA vez e compartilhados via `Arc` (não
//! recarrega 42MB + 3.5M nomes a cada clique).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

use crate::names::NameDb;
use crate::tweakdb::TweakDb;
use crate::writer::{Model, SetOutcome};

const INDEX_HTML: &str = include_str!("ui/index.html");

/// Estado compartilhado (read-only) entre as conexões.
struct State {
    db: TweakDb,
    names: NameDb,
}

pub fn serve(addr: &str) -> Result<(), String> {
    eprintln!("carregando tweakdb + nomes (uma vez)...");
    let db = crate::open(None)?;
    let names = crate::load_names(false)
        .ok_or("a UI precisa da lista de nomes (tweakdbstr.kark)")?;
    let state = Arc::new(State { db, names });

    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    println!("\n  UI do tweakdb em  http://{addr}");
    println!("  abra no navegador. Ctrl-C para parar.\n");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => {
                let st = Arc::clone(&state);
                std::thread::spawn(move || {
                    if let Err(e) = handle(s, &st) {
                        eprintln!("conexão: {e}");
                    }
                });
            }
            Err(e) => eprintln!("accept: {e}"),
        }
    }
    Ok(())
}

struct Req {
    method: String,
    path: String,
    query: String,
    body: String,
}

fn handle(mut stream: TcpStream, st: &State) -> std::io::Result<()> {
    let req = match read_request(&mut stream) {
        Ok(r) => r,
        Err(_) => return write_resp(&mut stream, 400, "text/plain; charset=utf-8", b"bad request"),
    };
    let (status, ctype, body) = route(&req, st);
    write_resp(&mut stream, status, ctype, &body)
}

fn read_request(stream: &mut TcpStream) -> Result<Req, String> {
    let mut r = BufReader::new(stream);
    let mut request_line = String::new();
    r.read_line(&mut request_line).map_err(|e| e.to_string())?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let raw_path = parts.next().unwrap_or("/").to_string();
    let (path, query) = match raw_path.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (raw_path, String::new()),
    };
    let mut content_length = 0usize;
    loop {
        let mut h = String::new();
        let n = r.read_line(&mut h).map_err(|e| e.to_string())?;
        if n == 0 || h.trim().is_empty() {
            break;
        }
        let low = h.to_ascii_lowercase();
        if let Some(v) = low.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        r.read_exact(&mut body).map_err(|e| e.to_string())?;
    }
    Ok(Req { method, path, query, body: String::from_utf8_lossy(&body).into_owned() })
}

fn write_resp(stream: &mut TcpStream, status: u16, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        _ => "OK",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn route(req: &Req, st: &State) -> (u16, &'static str, Vec<u8>) {
    let q = parse_query(&req.query);
    let qget = |k: &str| q.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => (200, "text/html; charset=utf-8", INDEX_HTML.as_bytes().to_vec()),
        ("GET", "/api/status") => json_resp(api_status()),
        ("GET", "/api/areas") => json_resp(api_areas(st)),
        ("GET", "/api/find") => json_resp(api_find(st, qget("q").unwrap_or(""))),
        ("GET", "/api/record") => json_resp(api_record(st, qget("name").unwrap_or(""))),
        ("GET", "/api/records") => {
            json_resp(api_records(st, qget("class"), qget("like"), qget("key")))
        }
        ("POST", "/api/check") => json_resp(api_apply(st, &req.body, qget("yaml").is_some(), true)),
        ("POST", "/api/apply") => json_resp(api_apply(st, &req.body, qget("yaml").is_some(), false)),
        ("POST", "/api/install") => json_resp(api_install()),
        ("POST", "/api/uninstall") => json_resp(api_uninstall()),
        ("POST", "/api/backup") => json_resp(api_backup()),
        _ => (404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

fn json_resp(json: String) -> (u16, &'static str, Vec<u8>) {
    (200, "application/json; charset=utf-8", json.into_bytes())
}

// ---- handlers (devolvem JSON) -------------------------------------------------

fn api_status() -> String {
    let target = crate::install_target(false);
    let orig = crate::pristine_path(&target);
    let target_exists = target.is_file();
    let has_orig = orig.is_file();
    let state = if !target_exists {
        "sem-alvo"
    } else if !has_orig {
        "vanilla"
    } else {
        match (std::fs::read(&target), std::fs::read(&orig)) {
            (Ok(a), Ok(b)) if a == b => "vanilla",
            _ => "modificado",
        }
    };
    format!(
        "{{\"target\":{},\"target_exists\":{},\"pristine\":{},\"state\":{}}}",
        jstr(&target.display().to_string()),
        target_exists,
        has_orig,
        jstr(state)
    )
}

fn api_areas(st: &State) -> String {
    use std::collections::HashMap;
    let mut areas: HashMap<String, u64> = HashMap::new();
    for (i, ft) in st.db.flat_types.iter().enumerate() {
        if ft.resolved.is_none() {
            continue;
        }
        if let Ok(pairs) = st.db.read_values(i) {
            for (id, _) in pairs {
                if let Some(name) = st.names.resolve(id) {
                    let head = name.split('.').next().unwrap_or(name);
                    *areas.entry(head.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    let mut top: Vec<(String, u64)> = areas.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let items: Vec<String> = top
        .iter()
        .map(|(k, v)| format!("{{\"area\":{},\"count\":{v}}}", jstr(k)))
        .collect();
    format!("{{\"areas\":[{}]}}", items.join(","))
}

fn api_find(st: &State, q: &str) -> String {
    if q.trim().is_empty() {
        return "{\"hits\":[]}".into();
    }
    let mut hits: Vec<&str> = st.names.search(q).map(|(_, name)| name).collect();
    hits.sort_unstable();
    hits.dedup();
    let total = hits.len();
    let items: Vec<String> = hits.iter().take(300).map(|n| jstr(n)).collect();
    format!("{{\"total\":{total},\"hits\":[{}]}}", items.join(","))
}

fn api_record(st: &State, name: &str) -> String {
    if name.trim().is_empty() {
        return "{\"error\":\"sem nome\"}".into();
    }
    let prefix = format!("{name}.");
    let mut rows: Vec<(String, String, String)> = Vec::new();
    for (i, ft) in st.db.flat_types.iter().enumerate() {
        let Some(r) = ft.resolved else { continue };
        let label = r.label();
        if let Ok(pairs) = st.db.read_values(i) {
            for (id, v) in pairs {
                if let Some(n) = st.names.resolve(id) {
                    if let Some(prop) = n.strip_prefix(&prefix) {
                        rows.push((
                            prop.to_string(),
                            label.clone(),
                            crate::fmt_value_resolved(&v, &st.names),
                        ));
                    }
                }
            }
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let props: Vec<String> = rows
        .iter()
        .map(|(p, t, v)| {
            format!("{{\"prop\":{},\"type\":{},\"value\":{}}}", jstr(p), jstr(t), jstr(v))
        })
        .collect();
    format!("{{\"name\":{},\"count\":{},\"props\":[{}]}}", jstr(name), rows.len(), props.join(","))
}

fn api_records(st: &State, class: Option<&str>, like: Option<&str>, key: Option<&str>) -> String {
    // Filtro por type_key (via classe, record-amostra ou hex) ou histograma.
    let filter_key: Option<u32> = if let Some(c) = class {
        Some(crate::hashes::record_type_key(c))
    } else if let Some(l) = like {
        let id = crate::hashes::tweak_db_id(l);
        st.db.records.iter().find(|r| r.id == id).map(|r| r.type_key)
    } else if let Some(k) = key {
        u32::from_str_radix(k.trim_start_matches("0x"), 16).ok()
    } else {
        None
    };

    if let Some(k) = filter_key {
        let mut named: Vec<&str> = st
            .db
            .records
            .iter()
            .filter(|r| r.type_key == k)
            .filter_map(|r| st.names.resolve(r.id))
            .collect();
        named.sort_unstable();
        let total = named.len();
        let items: Vec<String> = named.iter().take(500).map(|n| jstr(n)).collect();
        return format!(
            "{{\"mode\":\"list\",\"type_key\":\"{k:08x}\",\"total\":{total},\"records\":[{}]}}",
            items.join(",")
        );
    }

    use std::collections::HashMap;
    let mut by_key: HashMap<u32, (usize, Option<&str>)> = HashMap::new();
    for r in &st.db.records {
        let e = by_key.entry(r.type_key).or_insert((0, None));
        e.0 += 1;
        if e.1.is_none() {
            e.1 = st.names.resolve(r.id);
        }
    }
    let mut top: Vec<(u32, usize, Option<&str>)> =
        by_key.into_iter().map(|(k, (c, n))| (k, c, n)).collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let items: Vec<String> = top
        .iter()
        .map(|(k, c, ex)| {
            format!(
                "{{\"type_key\":\"{k:08x}\",\"count\":{c},\"example\":{}}}",
                jstr(ex.unwrap_or(""))
            )
        })
        .collect();
    format!("{{\"mode\":\"histogram\",\"classes\":[{}]}}", items.join(","))
}

/// Aplica um changeset (ou YAML, se `is_yaml`) num Model novo a partir do db
/// cacheado. Com `check_only`, não grava — só valida. Senão grava o
/// `.patched.bin` padrão (que o `install` instala).
fn api_apply(st: &State, body: &str, is_yaml: bool, check_only: bool) -> String {
    let mut model = match Model::from_db(&st.db) {
        Ok(m) => m,
        Err(e) => return jerror(&format!("modelo: {e}")),
    };

    let mut results: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut fail = 0usize;

    if is_yaml {
        let root = match crate::yaml::parse(body) {
            Ok(r) => r,
            Err(e) => return jerror(&format!("YAML: {e}")),
        };
        let ops = match crate::tweakxl::interpret(&root) {
            Ok(o) => o,
            Err(e) => return jerror(&e),
        };
        for r in crate::writer::apply_ops(&mut model, &st.names, &ops) {
            if r.ok {
                ok += 1;
            } else {
                fail += 1;
            }
            results.push(format!(
                "{{\"flat\":{},\"ok\":{},\"detail\":{}}}",
                jstr(&r.desc),
                r.ok,
                jstr(&r.detail)
            ));
        }
    } else {
        let edits = match crate::parse_changeset(body) {
            Ok(e) => e,
            Err(e) => return jerror(&e),
        };
        for e in &edits {
            let (okk, detail) = match model.apply(&e.flat, &e.op) {
                SetOutcome::Applied(ty) => (true, ty),
                SetOutcome::NotFound => (false, "flat inexistente".into()),
                SetOutcome::NotEditable { ty, reason } => (false, format!("{ty}: {reason}")),
            };
            if okk {
                ok += 1;
            } else {
                fail += 1;
            }
            results.push(format!(
                "{{\"flat\":{},\"ok\":{},\"detail\":{}}}",
                jstr(&e.flat),
                okk,
                jstr(&detail)
            ));
        }
    }

    if check_only || fail > 0 {
        return format!(
            "{{\"ok\":{ok},\"fail\":{fail},\"wrote\":false,\"results\":[{}]}}",
            results.join(",")
        );
    }

    // Grava o .patched.bin padrão ao lado do alvo do jogo.
    let target = crate::install_target(false);
    let out = crate::default_patched_path(&target);
    let bytes = model.serialize();
    if let Err(e) = std::fs::write(&out, &bytes) {
        return jerror(&format!("gravando {}: {e}", out.display()));
    }
    format!(
        "{{\"ok\":{ok},\"fail\":{fail},\"wrote\":true,\"out\":{},\"bytes\":{},\"results\":[{}]}}",
        jstr(&out.display().to_string()),
        bytes.len(),
        results.join(",")
    )
}

fn api_install() -> String {
    let target = crate::install_target(false);
    let patched = crate::default_patched_path(&target);
    if !patched.is_file() {
        return jerror("nada para instalar — aplique um tweak primeiro");
    }
    if !target.is_file() {
        return jerror("alvo do jogo não encontrado (CP77_DIR?)");
    }
    if let Err(e) = TweakDb::open(&patched) {
        return jerror(&format!("arquivo a instalar inválido: {e}"));
    }
    let orig = crate::pristine_path(&target);
    if !orig.exists() {
        if let Err(e) = std::fs::copy(&target, &orig) {
            return jerror(&format!("salvando pristino: {e}"));
        }
    }
    match std::fs::copy(&patched, &target) {
        Ok(_) => jok("instalado — lance o jogo e valide; 'Desfazer' volta ao vanilla"),
        Err(e) => jerror(&format!("instalando: {e}")),
    }
}

fn api_uninstall() -> String {
    let target = crate::install_target(false);
    let orig = crate::pristine_path(&target);
    if !orig.is_file() {
        return jerror("sem pristino — nada foi instalado por esta ferramenta");
    }
    match std::fs::copy(&orig, &target) {
        Ok(_) => jok("vanilla restaurado"),
        Err(e) => jerror(&format!("restaurando: {e}")),
    }
}

fn api_backup() -> String {
    let target = crate::install_target(false);
    if !target.is_file() {
        return jerror("alvo não existe");
    }
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut s = target.as_os_str().to_os_string();
    s.push(format!(".{secs}.bak"));
    let bak = std::path::PathBuf::from(s);
    match std::fs::copy(&target, &bak) {
        Ok(_) => jok(&format!("backup: {}", bak.display())),
        Err(e) => jerror(&format!("backup: {e}")),
    }
}

// ---- helpers JSON -------------------------------------------------------------

fn jok(msg: &str) -> String {
    format!("{{\"ok\":true,\"msg\":{}}}", jstr(msg))
}
fn jerror(msg: &str) -> String {
    format!("{{\"ok\":false,\"error\":{}}}", jstr(msg))
}

fn jstr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Decodifica `a=b&c=d` com percent-decoding básico.
fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(b) = u8::from_str_radix(hex, 16) {
                    out.push(b);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
