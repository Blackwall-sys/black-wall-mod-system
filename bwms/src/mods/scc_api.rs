//! Relatório estruturado do compilador de scripts pela API C do `libscc_lib.dylib` (a que fica ao lado do `scc` no pacote): cada definição
//! compilada, com tipo, se é `native`, dono, arquivo e linha (redscript `#15`/`#16`). O `checar` usa pra apontar a `native func` que nenhum
//! dylib registra — no jogo, uma native declarada sem registro derruba o bind dos scripts com SIGTRAP, sem dizer qual `.reds` foi.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

extern "C" {
    fn dlopen(caminho: *const c_char, modo: i32) -> *mut c_void;
    fn dlsym(h: *mut c_void, simbolo: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}
const RTLD_NOW: i32 = 2;

#[repr(C)]
struct StrWithLen {
    str: *const c_char,
    len: usize,
}

/// Tipo da definição no `SccSourceRefType`.
pub const FUNCAO: u8 = 3;

#[derive(Debug, Clone, PartialEq)]
pub struct Definicao {
    pub tipo: u8,
    pub native: bool,
    pub nome: String,
    pub dono: String,
    pub arquivo: String,
    pub linha: usize,
}

unsafe fn simbolo<T: Copy>(lib: *mut c_void, nome: &str) -> Result<T, String> {
    let c = CString::new(nome).map_err(|e| e.to_string())?;
    let p = dlsym(lib, c.as_ptr());
    if p.is_null() {
        return Err(format!("libscc sem {nome}"));
    }
    Ok(std::mem::transmute_copy(&p))
}

unsafe fn texto(s: StrWithLen) -> String {
    if s.str.is_null() {
        return String::new();
    }
    String::from_utf8_lossy(std::slice::from_raw_parts(s.str as *const u8, s.len)).into_owned()
}

/// Compila `r6/scripts` (e `extra`, se houver) com o cache vanilla de `r6` e saída em `saida` — o cache do jogo não é tocado — e devolve as
/// definições. `Err` com o texto do erro de compilação.
pub fn definicoes(lib: &Path, r6: &Path, extra: Option<&Path>, saida: &Path) -> Result<Vec<Definicao>, String> {
    unsafe {
        let caminho = CString::new(lib.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let h = dlopen(caminho.as_ptr(), RTLD_NOW);
        if h.is_null() {
            let e = dlerror();
            return Err(if e.is_null() { "dlopen do libscc falhou".into() } else { CStr::from_ptr(e).to_string_lossy().into_owned() });
        }
        type Novo = unsafe extern "C" fn(*const c_char) -> *mut c_void;
        type ComTexto = unsafe extern "C" fn(*mut c_void, *const c_char);
        type Com = unsafe extern "C" fn(*mut c_void);
        type Compilar = unsafe extern "C" fn(*mut c_void) -> *mut c_void;
        type CopiarErro = unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> usize;
        type Conta = unsafe extern "C" fn(*mut c_void) -> usize;
        type Ref = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
        type Tipo = unsafe extern "C" fn(*mut c_void, *mut c_void) -> u8;
        type Nativa = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
        type Nome = unsafe extern "C" fn(*mut c_void, *mut c_void) -> StrWithLen;
        type Linha = unsafe extern "C" fn(*mut c_void, *mut c_void) -> usize;

        let novo: Novo = simbolo(h, "scc_settings_new")?;
        let saida_em: ComTexto = simbolo(h, "scc_settings_set_output_cache_file")?;
        let mais_caminho: ComTexto = simbolo(h, "scc_settings_add_script_path")?;
        let sem_popup: Com = simbolo(h, "scc_settings_disable_error_popup")?;
        let compilar: Compilar = simbolo(h, "scc_compile")?;
        let soltar: Com = simbolo(h, "scc_free_result")?;
        let sucesso: Compilar = simbolo(h, "scc_get_success")?;
        let copiar_erro: CopiarErro = simbolo(h, "scc_copy_error")?;
        let quantas: Conta = simbolo(h, "scc_output_source_ref_count")?;
        let referencia: Ref = simbolo(h, "scc_output_get_source_ref")?;
        let tipo: Tipo = simbolo(h, "scc_source_ref_type")?;
        let nativa: Nativa = simbolo(h, "scc_source_ref_is_native")?;
        let nome: Nome = simbolo(h, "scc_source_ref_name")?;
        let dono: Nome = simbolo(h, "scc_source_ref_parent_name")?;
        let arquivo: Nome = simbolo(h, "scc_source_ref_path")?;
        let linha: Linha = simbolo(h, "scc_source_ref_line")?;

        let r6c = CString::new(r6.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let config = novo(r6c.as_ptr());
        let saida_c = CString::new(saida.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        saida_em(config, saida_c.as_ptr());
        if let Some(extra) = extra {
            let extra_c = CString::new(extra.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
            mais_caminho(config, extra_c.as_ptr());
        }
        sem_popup(config);
        let resultado = compilar(config);
        let saida_ok = sucesso(resultado);
        if saida_ok.is_null() {
            let mut buf = vec![0u8; 16 * 1024];
            let n = copiar_erro(resultado, buf.as_mut_ptr() as *mut c_char, buf.len());
            soltar(resultado);
            buf.truncate(n.min(buf.len()));
            return Err(String::from_utf8_lossy(&buf).trim_end_matches('\0').to_string());
        }
        let n = quantas(saida_ok);
        let mut defs = Vec::with_capacity(n);
        for i in 0..n {
            let r = referencia(saida_ok, i);
            if r.is_null() {
                continue;
            }
            defs.push(Definicao {
                tipo: tipo(saida_ok, r),
                native: nativa(saida_ok, r),
                nome: texto(nome(saida_ok, r)),
                dono: texto(dono(saida_ok, r)),
                arquivo: texto(arquivo(saida_ok, r)),
                linha: linha(saida_ok, r),
            });
        }
        soltar(resultado);
        Ok(defs)
    }
}

/// As `native func` dos `.reds` (fora do cache vanilla) que nenhum dos binários registra: `arquivo:linha dono.nome`. "Registra" é a regra
/// do `bwms-helper natives` (`bwms_primitives::native_registrada`), a mesma do instalador, do gate do pacote e da compilação na abertura do jogo.
pub fn natives_sem_registro(defs: &[Definicao], binarios: &[Vec<u8>]) -> Vec<String> {
    let mut vistas = std::collections::BTreeSet::new();
    defs.iter()
        .filter(|d| d.tipo == FUNCAO && d.native && !d.arquivo.is_empty())
        .filter(|d| !bwms_primitives::native_registrada::registrada_em(&d.nome, binarios))
        .filter(|d| vistas.insert((d.dono.clone(), d.nome.clone())))
        .map(|d| {
            let dono = if d.dono.is_empty() { String::new() } else { format!("{}.", d.dono) };
            format!("{}:{} {dono}{}", d.arquivo, d.linha, d.nome)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(nome: &str, native: bool, arquivo: &str) -> Definicao {
        Definicao { tipo: FUNCAO, native, nome: nome.into(), dono: String::new(), arquivo: arquivo.into(), linha: 3 }
    }

    #[test]
    fn so_native_de_arquivo_sem_registro() {
        let bin = vec![b"\0BwmsExiste\0".to_vec()];
        let defs = vec![
            def("BwmsExiste", true, "a.reds"),
            def("BwmsFalta", true, "b.reds"),
            def("BwmsFalta", true, "b.reds"),
            def("NaoNative", false, "c.reds"),
            def("DoCache", true, ""),
        ];
        assert_eq!(natives_sem_registro(&defs, &bin), vec!["b.reds:3 BwmsFalta".to_string()]);
    }
}
