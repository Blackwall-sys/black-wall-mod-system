//! Argumentos de lançamento do jogo (`GetLaunchParameters`): o que veio na linha de comando do processo, pro script
//! (`BwmsGetLaunchArg`/`BwmsHasLaunchArg`) e pros plugins (`BwmsApi.launch_arg`). Lidos do próprio processo (`_NSGetArgv`).
//!
//! Formas aceitas, como o motor e os launchers escrevem: `-nome valor`, `--nome valor`, `-nome=valor`, `--nome=valor`; um argumento sem
//! valor (`-nome` seguido de outro `-…` ou do fim) existe com valor vazio. O nome não diferencia maiúscula de minúscula.

extern "C" {
    fn _NSGetArgc() -> *const i32;
    fn _NSGetArgv() -> *const *const *const i8;
}

/// Argumentos do processo, sem o executável.
pub(crate) fn do_processo() -> Vec<String> {
    unsafe {
        let (argc, argv) = (_NSGetArgc(), _NSGetArgv());
        if argc.is_null() || argv.is_null() || (*argv).is_null() {
            return Vec::new();
        }
        (1..*argc as usize)
            .filter_map(|i| {
                let p = *(*argv).add(i);
                (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
            })
            .collect()
    }
}

/// Valor de `nome` em `args` (sem o `-`): `Some("")` pra argumento sem valor, `None` se não veio.
pub(crate) fn valor(args: &[String], nome: &str) -> Option<String> {
    let nome = nome.trim_start_matches('-');
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        let Some(sem_traco) = a.strip_prefix("--").or_else(|| a.strip_prefix('-')) else { continue };
        let (chave, igual) = match sem_traco.split_once('=') {
            Some((c, v)) => (c, Some(v.to_string())),
            None => (sem_traco, None),
        };
        if !chave.eq_ignore_ascii_case(nome) {
            continue;
        }
        return Some(igual.unwrap_or_else(|| match it.peek() {
            Some(prox) if !prox.starts_with('-') => prox.to_string(),
            _ => String::new(),
        }));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn formas_de_argumento() {
        let args = a(&["-skipStartScreen", "--modo", "rapido", "-nivel=3", "--x=a=b", "-bwmsTeste", "263", "-final"]);
        assert_eq!(valor(&args, "skipStartScreen"), Some(String::new()));
        assert_eq!(valor(&args, "modo"), Some("rapido".into()));
        assert_eq!(valor(&args, "-nivel"), Some("3".into()));
        assert_eq!(valor(&args, "x"), Some("a=b".into()));
        assert_eq!(valor(&args, "BWMSTESTE"), Some("263".into()));
        assert_eq!(valor(&args, "final"), Some(String::new()));
        assert_eq!(valor(&args, "ausente"), None);
        assert_eq!(valor(&args, "rapido"), None);
    }
}
