//! Lê e roda `.lua` de verdade. Os arquivos ficam fora do repositório: `tmp/bwms-lua-conformidade/` e `_referencia-de-terceiro/`.
//! Pasta ausente na máquina = teste passa sem rodar nada, e o `eprintln!` diz isso.

use bwms_lua::{Lua, Value};
use std::path::{Path, PathBuf};

fn raiz() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn lua_em(d: &Path, saida: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(d) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            lua_em(&p, saida);
        } else if p.extension().is_some_and(|x| x == "lua") {
            saida.push(p);
        }
    }
}

fn conformidade(arquivo: &str) {
    let p = raiz().join("tmp/bwms-lua-conformidade").join(arquivo);
    let Ok(codigo) = std::fs::read(&p) else {
        eprintln!("sem {}: nada rodou", p.display());
        return;
    };
    let lua = Lua::new();
    match lua.load(codigo).set_name(format!("@{arquivo}")).call::<String>(()) {
        Ok(r) => assert_eq!(r, "ok"),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn linguagem() {
    conformidade("linguagem.lua");
}

#[test]
fn biblioteca() {
    conformidade("biblioteca.lua");
}

#[test]
fn todo_lua_do_corpus_compila() {
    let mut todos = Vec::new();
    lua_em(&raiz().join("_referencia-de-terceiro"), &mut todos);
    eprintln!("{} arquivos .lua", todos.len());
    let lua = Lua::new();
    for p in &todos {
        let codigo = std::fs::read(p).unwrap_or_default();
        if let Err(e) = lua.load(codigo).set_name(format!("@{}", p.display())).into_function() {
            panic!("{}: {e}", p.display());
        }
    }
}

/// `print` capturado e a API de fronteira, sem nenhum `.lua`: `create_function`, `globals`, inteiro volta inteiro.
#[test]
fn fronteira_com_rust() {
    let lua = Lua::new();
    let f = lua.create_function(|_, (a, b): (i64, Option<String>)| Ok((a + 1, b.unwrap_or_default()))).expect("função");
    lua.globals().set("f", f.clone()).expect("global");
    let (n, s): (i64, String) = f.call((41, "x")).expect("chamada");
    assert_eq!((n, s.as_str()), (42, "x"));
    assert!(matches!(lua.globals().get::<Value>("f"), Ok(Value::Function(_))));
}
