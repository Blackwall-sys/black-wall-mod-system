//! Configuração do mod: ler e gravar opções do usuário.

#![allow(dead_code)]

pub(crate) mod argumentos;

pub(crate) mod do_mod;

use std::ffi::c_void;
use crate::rtti::{self};
use crate::register::*;

// ===== `redscript-mod-persistence`: `BwmsConfigGet(key)->String` / `BwmsConfigSet(key,value)->Bool`,
// arquivo flat `~/.bwms-modconfig.txt` (1 "key=value" por linha, key sem "="; NUNCA o save-file). =====
pub(crate) fn modconfig_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME").ok().map(|h| std::path::Path::new(&h).join(".bwms-modconfig.txt"))
}

/// Conteúdo do config junto com a identidade do arquivo na hora da leitura (caminho, `mtime`, tamanho).
/// Reler e decodificar o arquivo a cada `BwmsConfigGet` custava 8,6 µs (bench `rustc -O`); o `stat`
/// mais a busca na memória fica abaixo de 1 µs. A identidade é conferida em toda chamada, então a
/// edição feita por fora (os `bwms-prove-*.sh`, o usuário) vale já na chamada seguinte.
pub(crate) static MODCONFIG_LIDO: std::sync::Mutex<Option<(std::path::PathBuf, std::time::SystemTime, u64, String)>> =
    std::sync::Mutex::new(None);

pub(crate) fn modconfig_set(key: &str, value: &str) -> bool {
    let key = config_sanitize_key(key);
    if key.is_empty() {
        return false;
    }
    let value = config_sanitize_value(value);
    let Some(p) = modconfig_path() else { return false };
    let existing = std::fs::read_to_string(&p).unwrap_or_default();
    let out = config_upsert(&existing, &key, &value);
    // Escrita ATÔMICA: grava num temp e renomeia (rename é atômico no mesmo volume). Se o processo
    // morrer no meio, o config ANTIGO fica intacto — antes um `write` in-place truncado o corrompia.
    let tmp = p.with_extension("tmp");
    if std::fs::write(&tmp, &out).is_err() {
        return false;
    }
    std::fs::rename(&tmp, &p).is_ok()
}

pub(crate) unsafe extern "C" fn tramp_config_get(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let key = strings.first().cloned().flatten().unwrap_or_default();
    let v = modconfig_get(&key);
    let ok = write_cstring_inline_ret(out, &v);
    crate::log_dev!("[modconfig] BwmsConfigGet('{key}') = '{v}' (escrito={ok})");
}

pub(crate) unsafe extern "C" fn tramp_config_set(_c: *mut c_void, frame: *mut c_void, out: *mut c_void, _rt: i64) {
    let func = current_native_func();
    let (_args, strings) = rtti::read_params_consuming_with_strings(func, frame);
    let key = strings.first().cloned().flatten().unwrap_or_default();
    let value = strings.get(1).cloned().flatten().unwrap_or_default();
    let ok = modconfig_set(&key, &value);
    crate::log_dev!("[modconfig] BwmsConfigSet('{key}','{value}') -> {ok}");
    write_bool_ret(out, ok);
}


/// Chave saneada: sem `=` nem quebra de linha (quebrariam o formato "key=value"/1-linha).
pub(crate) fn config_sanitize_key(key: &str) -> String {
    key.chars().filter(|&c| c != '=' && c != '\n' && c != '\r').collect()
}

/// Valor saneado: sem quebra de linha (injetaria uma linha/chave falsa no arquivo).
pub(crate) fn config_sanitize_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

/// Lê o valor de `key` do conteúdo do config (1º match; formato "key=value" por linha). PURA.
pub(crate) fn config_get_from(content: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    content
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or("")
        .to_string()
}

/// Upsert PURO: devolve o conteúdo novo com `key=value`, COLAPSANDO duplicatas da chave numa linha
/// só (o writer antigo reescrevia CADA linha da chave, deixando duplicatas), preservando as outras
/// na ordem e dropando linhas vazias. Base testável do `modconfig_set`.
pub(crate) fn config_upsert(existing: &str, key: &str, value: &str) -> String {
    let prefix = format!("{key}=");
    let mut out = String::new();
    let mut written = false;
    for line in existing.lines() {
        if line.starts_with(&prefix) {
            if !written {
                out.push_str(&prefix);
                out.push_str(value);
                out.push('\n');
                written = true;
            }
            // duplicatas seguintes da MESMA chave: dropadas (colapso)
        } else if !line.is_empty() {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !written {
        out.push_str(&prefix);
        out.push_str(value);
        out.push('\n');
    }
    out
}

pub(crate) fn config_get_em(p: &std::path::Path, key: &str) -> String {
    let Ok(meta) = std::fs::metadata(p) else { return String::new() };
    let Ok(mtime) = meta.modified() else {
        return std::fs::read_to_string(p).map(|c| config_get_from(&c, key)).unwrap_or_default();
    };
    let mut lido = MODCONFIG_LIDO.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((caminho, m, tamanho, conteudo)) = lido.as_ref() {
        if caminho == p && *m == mtime && *tamanho == meta.len() {
            return config_get_from(conteudo, key);
        }
    }
    // O `stat` veio antes da leitura: se o arquivo mudar no meio, o conteúdo guardado é o mais novo e
    // o `mtime` guardado o mais velho, o que só custa uma releitura na próxima chamada.
    let Ok(conteudo) = std::fs::read_to_string(p) else { return String::new() };
    let v = config_get_from(&conteudo, key);
    *lido = Some((p.to_path_buf(), mtime, meta.len(), conteudo));
    v
}

pub(crate) fn modconfig_get(key: &str) -> String {
    let key = config_sanitize_key(key);
    let Some(p) = modconfig_path() else { return String::new() };
    config_get_em(&p, &key)
}

