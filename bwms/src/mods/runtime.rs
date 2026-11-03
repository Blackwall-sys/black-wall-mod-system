//! runtime.rs — acha o compilador de scripts (`scc`) do pacote e checa os `.reds` do jogo com ele (`bwms mods checar`), sem tocar no cache
//! do jogo.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Acha o binário de uma peça do runtime: bundle ao lado do `bwms`, dirs de release do
/// projeto (mods-research), ou PATH. Retorna None se não achar.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let near = dir.join(name); // bundle ao lado
            if near.is_file() {
                return Some(near);
            }
        }
        // mods-research = .../bwms/target/release/<exe> → 4 pais acima
        if let Some(root) = exe.ancestors().nth(4) {
            for c in [
                root.join(format!("{name}/target/release/{name}")),
                root.join(format!("redscript/target/release/{name}")),
                // o mesmo scc dos roteiros da raiz (bwms-fastboot.sh, bwms-scc-rascunho.sh, bwms-only-redscript.sh)
                root.join(format!("redscript/dist/redscript-macos/engine/tools/{name}")),
            ] {
                if c.is_file() {
                    return Some(c);
                }
            }
        }
    }
    if Command::new(name).arg("--help").output().is_ok() {
        return Some(PathBuf::from(name));
    }
    None
}

/// Checa os scripts do jogo sem tocar no cache: compila `r6/scripts` inteiro numa saída temporária (`-outputCacheFile`) e devolve
/// `(passou, relatório)`, com os erros do compilador e as dicas do BWMS pra mod feito pro Windows (`r6/config/redsUserHints/bwms-dicas.toml`). É o
/// "checar meus .reds" antes de abrir o jogo: um erro aqui é o mod que faria a aba do BWMS sumir ou o jogo cair no carregamento.
/// `<temp>/r6` com o cache vanilla do jogo (o backup que o BWMS guarda, ou o do compilador) e `scripts`/`config` apontando pros do jogo. O
/// compilador, pela CLI com `-outputCacheFile` ou pela `libscc`, restaura o cache original em `<r6>/cache` antes de compilar — medido em
/// 2026-09-19: o `final.redscripts` compilado do jogo voltava ao vanilla, e o jogo abriria sem os scripts dos mods até a próxima compilação.
/// O vanilla da build instalada agora: o 1º blob sem a marca do `scc`, a começar pelo do jogo (depois de uma atualização ele é o vanilla
/// novo, e o `.bk`/`.bwms-bak` podem ser da build anterior).
fn vanilla_dos_scripts(cache: &Path) -> Option<PathBuf> {
    ["final.redscripts", "final.redscripts.bk", "final.redscripts.redscripts.bk", "final.redscripts.bwms-bak"]
        .iter()
        .map(|n| cache.join(n))
        .find(|p| std::fs::read(p).is_ok_and(|b| !b.is_empty() && !bwms_primitives::blob_compilado_pelo_scc(&b)))
}

fn r6_temporario(game: &Path, temp: &Path) -> Result<PathBuf, String> {
    let cache = game.join("r6/cache");
    let base = vanilla_dos_scripts(&cache)
        .ok_or("sem final.redscripts vanilla no jogo: verifique a integridade do jogo (Steam › Verificar arquivos)")?;
    let r6 = temp.join("r6");
    std::fs::create_dir_all(r6.join("cache")).map_err(|e| e.to_string())?;
    std::fs::copy(&base, r6.join("cache/final.redscripts")).map_err(|e| e.to_string())?;
    std::os::unix::fs::symlink(game.join("r6/scripts"), r6.join("scripts")).map_err(|e| e.to_string())?;
    if game.join("r6/config").is_dir() {
        std::os::unix::fs::symlink(game.join("r6/config"), r6.join("config")).map_err(|e| e.to_string())?;
    }
    Ok(r6)
}

pub fn checar_redscript(game: &Path, extra: Option<&Path>) -> (bool, String) {
    let Some(bin) = find_tool("scc") else {
        return (false, "scc não encontrado — bundle/instale o runtime".into());
    };
    let temp = std::env::temp_dir().join(format!("cp77-mods-checar-r6-{}", std::process::id()));
    let r6 = match r6_temporario(game, &temp) {
        Ok(r) => r,
        Err(e) => {
            let _ = std::fs::remove_dir_all(&temp);
            return (false, format!("não montou o r6 temporário: {e}"));
        }
    };
    let (passou, resumo) = checar_em(&bin, &r6, extra);
    let _ = std::fs::remove_dir_all(&temp);
    (passou, resumo)
}

/// Os `.reds` de uma pasta, em qualquer profundidade.
fn reds_em(pasta: &Path) -> Vec<PathBuf> {
    let (mut achados, mut pilha) = (Vec::new(), vec![pasta.to_path_buf()]);
    while let Some(d) = pilha.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                pilha.push(p);
            } else if p.extension().is_some_and(|x| x == "reds") {
                achados.push(p);
            }
        }
    }
    achados.sort();
    achados
}

fn checar_em(bin: &Path, r6: &Path, extra: Option<&Path>) -> (bool, String) {
    let temp = r6.parent().unwrap_or(r6);
    let saida = temp.join("saida-cli.redscripts");
    let log = r6.join("logs/redscript_rCURRENT.log");
    let mut args: Vec<String> = vec!["-compile".into(), r6.join("scripts").to_string_lossy().into_owned()];
    // A pasta extra entra na mesma passada, como os `.reds` de plugin do `scripts_add`: lista de arquivos no `-compilePathsFile`.
    if let Some(extra) = extra {
        let lista = temp.join("extra-paths.txt");
        let arquivos: Vec<String> = reds_em(extra).iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if std::fs::write(&lista, arquivos.join("\n")).is_ok() && !arquivos.is_empty() {
            args.extend(["-compilePathsFile".into(), lista.to_string_lossy().into_owned()]);
        }
    }
    args.extend(["-outputCacheFile".into(), saida.to_string_lossy().into_owned()]);
    let rodou = Command::new(bin).args(&args).output();
    let texto = std::fs::read_to_string(&log).unwrap_or_default();
    // Só os erros, com a linha e o trecho que o compilador mostra, e o bloco das soluções conhecidas.
    let mut relatorio: Vec<&str> = Vec::new();
    let mut contexto = 0;
    for l in texto.lines() {
        if l.contains("[ERROR") || l.contains("known solution") {
            contexto = 4;
        }
        if contexto > 0 {
            relatorio.push(l);
            contexto -= 1;
        } else if l.starts_with("- ") && relatorio.last().is_some_and(|u| u.starts_with("- ") || u.contains("known solution")) {
            relatorio.push(l);
        }
    }
    let compilou = matches!(&rodou, Ok(o) if o.status.success()) && !texto.contains("[ERROR");
    let resumo = match &rodou {
        Err(e) => format!("erro ao rodar o scc: {e}"),
        Ok(_) if compilou => format!("scripts OK ({} aviso(s)); o cache do jogo não foi tocado", texto.matches("[WARN").count()),
        Ok(_) => relatorio.join("\n"),
    };
    if !compilou {
        return (false, resumo);
    }
    // A native declarada que ninguém registra compila, mas derruba o bind dos scripts no jogo (SIGTRAP): o relatório do compilador diz o
    // arquivo e a linha de cada `native func`, e o nome tem que estar no dylib do BWMS ou num plugin de `red4ext/plugins/`.
    let lib = bin.with_file_name("libscc_lib.dylib");
    let defs = super::scc_api::definicoes(&lib, r6, extra, &temp.join("saida-api.redscripts"));
    // Os binários que registram natives são os do jogo de verdade (o `r6` temporário é só pra compilação).
    let jogo = std::fs::read_link(r6.join("scripts")).ok().and_then(|s| s.parent().and_then(|p| p.parent()).map(Path::to_path_buf)).unwrap_or_default();
    let defs = match defs {
        Ok(d) => d,
        Err(e) => return (false, format!("{resumo}\nrelatório do compilador indisponível: {e}")),
    };
    let mut binarios: Vec<Vec<u8>> = std::fs::read(jogo.join(bwms_primitives::PASTA_DO_RUNTIME).join("libcp77_console.dylib")).into_iter().collect();
    if let Ok(plugins) = std::fs::read_dir(jogo.join(bwms_primitives::PASTA_DO_RUNTIME).join("plugins")) {
        binarios.extend(plugins.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "dylib")).filter_map(|p| std::fs::read(p).ok()));
    }
    let natives = defs.iter().filter(|d| d.tipo == super::scc_api::FUNCAO && d.native && !d.arquivo.is_empty()).count();
    let mut faltam = super::scc_api::natives_sem_registro(&defs, &binarios);
    // O que nenhum binário do BWMS registra pode ser native do próprio jogo que a ponte declara de novo (`registrada_no_jogo`): o
    // executável só é lido nesse caso.
    if !faltam.is_empty() {
        let exe = std::fs::read(jogo.join("Cyberpunk2077.app/Contents/MacOS/Cyberpunk2077")).unwrap_or_default();
        faltam.retain(|f| {
            let nome = f.rsplit([' ', '.']).next().unwrap_or_default();
            !bwms_primitives::native_registrada::registrada_no_jogo(nome, &exe)
        });
    }
    if faltam.is_empty() {
        return (true, format!("{resumo}; {natives} native func dos .reds, todas registradas pelo BWMS ou por um plugin"));
    }
    let lista: Vec<String> = faltam.iter().map(|f| format!("  {f}")).collect();
    (
        false,
        format!("{resumo}\n{} native func sem registro — o jogo cai ao ligar os scripts; tire a declaração ou instale quem registra:\n{}", faltam.len(), lista.join("\n")),
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    fn jogo(nome: &str) -> PathBuf {
        let g = std::env::temp_dir().join(format!("bwms-runtime-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        std::fs::create_dir_all(g.join("r6/cache")).unwrap();
        g
    }

    #[test]
    fn vanilla_dos_scripts_prefere_o_blob_do_jogo_sem_a_marca() {
        let g = jogo("vanilla");
        let c = g.join("r6/cache");
        std::fs::write(c.join("final.redscripts.bwms-bak"), "vanilla-velho").unwrap();
        std::fs::write(c.join("final.redscripts"), "vanilla-novo").unwrap();
        assert_eq!(vanilla_dos_scripts(&c), Some(c.join("final.redscripts")), "depois da atualização o blob do jogo é o vanilla novo");
        std::fs::write(c.join("final.redscripts"), "x$REDSCRIPT_SIGNATUREx").unwrap();
        std::fs::write(c.join("final.redscripts.bk"), "vanilla-do-scc").unwrap();
        assert_eq!(vanilla_dos_scripts(&c), Some(c.join("final.redscripts.bk")), "blob compilado fica de fora");
        std::fs::write(c.join("final.redscripts.bk"), "").unwrap();
        assert_eq!(vanilla_dos_scripts(&c), Some(c.join("final.redscripts.bwms-bak")), "vazio fica de fora");
        let _ = std::fs::remove_dir_all(&g);
    }
}
