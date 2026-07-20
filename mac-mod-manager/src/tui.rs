//! tui.rs — TUI interativa zero-dep (raw mode via `stty`, render por ANSI). A cara Mac do
//! gerenciador: runtime + modlist + ações num lugar só. Sem crate de TUI.
//!
//! Teclas: ↑/↓ seleciona mod · [i] instalar (pasta/.zip) · [r] remover · [c] classificar · [q] sair.

use crate::{apply, runtime, source};
use bwms_core::classify;
use bwms_core::theme::{self, ModState};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CLEAR: &str = "\x1b[2J\x1b[H";
const HIDE: &str = "\x1b[?25l";
const SHOW: &str = "\x1b[?25h";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const INV: &str = "\x1b[7m";
const RST: &str = "\x1b[0m";

enum Key {
    Up,
    Down,
    Char(char),
    Other,
}

/// Salva/seta/restaura o modo do terminal via stty (zero-dep). Retorna o `stty -g` original.
fn stty(arg: &str) -> Option<String> {
    let o = Command::new("stty").arg(arg).stdin(Stdio::inherit()).output().ok()?;
    if o.status.success() {
        Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
    } else {
        None
    }
}

fn enter_raw() -> Option<String> {
    let orig = stty("-g");
    // cbreak: char-a-char, sem echo (mantém Ctrl-C funcionando)
    let _ = Command::new("stty").args(["-icanon", "-echo", "min", "1"]).stdin(Stdio::inherit()).status();
    print!("{HIDE}");
    let _ = io::stdout().flush();
    orig
}

fn leave_raw(orig: &Option<String>) {
    print!("{SHOW}{RST}");
    let _ = io::stdout().flush();
    if let Some(o) = orig {
        let _ = Command::new("stty").arg(o).stdin(Stdio::inherit()).status();
    } else {
        let _ = Command::new("stty").arg("sane").stdin(Stdio::inherit()).status();
    }
}

fn read_key() -> Key {
    let mut b = [0u8; 1];
    if io::stdin().read(&mut b).unwrap_or(0) == 0 {
        return Key::Other;
    }
    match b[0] {
        0x1b => {
            // sequência de seta: ESC [ A/B
            let mut seq = [0u8; 2];
            if io::stdin().read(&mut seq).unwrap_or(0) == 2 && seq[0] == b'[' {
                match seq[1] {
                    b'A' => Key::Up,
                    b'B' => Key::Down,
                    _ => Key::Other,
                }
            } else {
                Key::Char('q') // ESC sozinho = sair
            }
        }
        c => Key::Char(c as char),
    }
}

/// Lê uma linha em modo COZIDO (pra prompts de caminho), volta pro raw depois.
fn prompt_line(orig: &Option<String>, msg: &str) -> String {
    leave_raw(orig);
    print!("{msg}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    let _ = enter_raw();
    line.trim().to_string()
}

fn tool_status(name: &str) -> String {
    if runtime::find_tool(name).is_some() {
        format!("{GREEN}✓{RST}")
    } else {
        format!("{RED}✗{RST}")
    }
}

fn render(game: &Path, mods: &[ModState], sel: usize, msg: &str) {
    let mut s = String::new();
    s.push_str(CLEAR);
    s.push_str(&format!("{BOLD}{CYAN} Cyberpunk 2077 — Mac Mod Runtime (Blackwall){RST}\n"));
    s.push_str(&format!("{DIM} {}{RST}\n\n", game.display()));
    // runtime
    s.push_str(&format!("{BOLD} Runtime:{RST}  "));
    for (label, bin) in [
        ("TweakDB", "tweakdb-tool"),
        ("redscript", "scc"),
        ("input", "input-loader"),
    ] {
        s.push_str(&format!("{} {}   ", tool_status(bin), label));
    }
    let dylib = game.join("red4ext/libcp77_console.dylib").exists();
    s.push_str(&format!("{} CET(Blackwall.sys)\n\n", if dylib { format!("{GREEN}✓{RST}") } else { format!("{RED}✗{RST}") }));
    // modlist
    s.push_str(&format!("{BOLD} Mods no staging ({}):{RST}\n", mods.len()));
    if mods.is_empty() {
        s.push_str(&format!("{DIM}   (nenhum — aperte [i] pra instalar){RST}\n"));
    } else {
        for (i, m) in mods.iter().enumerate() {
            let line = format!("[{}] {} — {}", if m.active { "x" } else { " " }, m.name, theme::category_label(&m.category));
            if i == sel {
                s.push_str(&format!("{INV} ▸ {} {RST}\n", line));
            } else {
                s.push_str(&format!("   {}\n", line));
            }
        }
    }
    s.push_str(&format!("\n{DIM} ↑/↓ navega · [i] instalar · [r] remover · [c] classificar · [q] sair{RST}\n"));
    if !msg.is_empty() {
        s.push_str(&format!("\n{CYAN} {msg}{RST}\n"));
    }
    print!("{s}");
    let _ = io::stdout().flush();
}

/// Loop da TUI.
pub fn run(game: PathBuf) -> i32 {
    let orig = enter_raw();
    let mut sel = 0usize;
    let mut msg = String::new();
    loop {
        let mods = apply::reconcile(&game);
        if sel >= mods.len() {
            sel = mods.len().saturating_sub(1);
        }
        render(&game, &mods, sel, &msg);
        msg.clear();
        match read_key() {
            Key::Up => sel = sel.saturating_sub(1),
            Key::Down => {
                if sel + 1 < mods.len() {
                    sel += 1;
                }
            }
            Key::Char('q') | Key::Char('\x03') => break,
            Key::Char('i') => msg = install_flow(&orig, &game),
            Key::Char('r') => {
                if let Some(m) = mods.get(sel) {
                    msg = crate::remove_staged(&game, &m.name);
                }
            }
            Key::Char('c') => msg = classify_flow(&orig),
            _ => {}
        }
    }
    leave_raw(&orig);
    print!("{CLEAR}");
    let _ = io::stdout().flush();
    0
}

fn install_flow(orig: &Option<String>, game: &Path) -> String {
    let path = prompt_line(orig, "\n Caminho do mod (pasta ou .zip): ");
    if path.is_empty() {
        return "cancelado".into();
    }
    let dir = match source::open_source(Path::new(&path)) {
        Ok(d) => d,
        Err(e) => return format!("erro: {e}"),
    };
    let report = classify::classify(&dir);
    if !report.risks.is_empty() {
        return format!("BLOQUEADO: {} risco(s) — use `classify` no terminal pra ver", report.risks.len());
    }
    // pipeline UNIFICADO (o mesmo da CLI): stage → apply → activate
    match crate::install_staged(&dir, game, &report.name) {
        Ok(sumario) => format!("instalado {sumario}"),
        Err(e) => format!("falhou: {e}"),
    }
}

fn classify_flow(orig: &Option<String>) -> String {
    let path = prompt_line(orig, "\n Caminho do mod (pasta ou .zip): ");
    if path.is_empty() {
        return "cancelado".into();
    }
    let dir = match source::open_source(Path::new(&path)) {
        Ok(d) => d,
        Err(e) => return format!("erro: {e}"),
    };
    let r = classify::classify(&dir);
    format!(
        "{}: {:?}, {} arquivo(s), {} dep(s), {} risco(s)",
        r.name,
        r.class,
        r.files.len(),
        r.deps.len(),
        r.risks.len()
    )
}
