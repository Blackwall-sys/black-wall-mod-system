//! Recompilação dos `.reds` pro modder (RED4ext `#97`): com `red4ext/bwms-recarga-de-scripts` na pasta do jogo, o vigia recompila o bundle
//! pelo `scc` do pacote quando um `.reds` muda, e o `aplicar_tick` troca só o corpo das funções mudadas com o jogo aberto
//! (`tradutor_de_bytecode`).
//!
//! A recarga completa do motor (`0x104d7b26c`) não é usada: o bind refaz todas as funções de script, nada no varejo reaponta os caches
//! nativos e o jogo cai em menos de 1 s (`DATABASE.md`, "Carregador de scripts e recarga"; `HISTORICO.md`, "2026-09-19 — Boots 299–304").

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use bwms_primitives::native_registrada::{declaradas_em, registrada_em, registrada_no_jogo};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A lista do `-compilePathsFile`, em `red4ext/`. Fica depois do passe: diz o que entrou além de `r6/scripts` na última compilação.
const LISTA_DE_CAMINHOS: &str = "bwms-scripts-caminhos.txt";
/// A pasta dos `.reds` do BWMS em `r6/scripts`. As natives dela o instalador confere contra a dylib (`bwms-helper natives`) antes de
/// compilar, e a conferência daqui fica com os outros mods.
const PASTA_DO_BWMS: &str = "blackwall-mods";
/// A dylib do BWMS em `red4ext/`, o caminho que o instalador grava no `LC_LOAD_DYLIB` do jogo.
const DYLIB_DO_BWMS: &str = "libcp77_console.dylib";
/// O `scc` e a lib dele, que o passo 6 do `bwms-install.sh` copia para `red4ext/`: o `scc` carrega a lib da própria pasta.
const SCC_DO_JOGO: &str = "scc";
const LIB_DO_SCC: &str = "libscc_lib.dylib";

/// O `scc` do pacote, pela pasta da dylib (`red4ext/`).
pub(crate) fn scc() -> Option<PathBuf> {
    scc_em(Path::new(&crate::dylib_dir()?))
}

/// O `red4ext/scc` com a lib ao lado vem primeiro: o instalador o deixa na pasta do jogo, e o kit baixado pode ser apagado depois. Sem
/// ele, o caminho de `red4ext/bwms-scc.txt`, que o instalador grava com o `red4ext/scc` que ele copiou (`bwms-install.sh`, passo 6). Caminho relativo
/// no arquivo fica de fora: seria resolvido pela pasta corrente do jogo, não pela de quem o gravou.
fn scc_em(red4: &Path) -> Option<PathBuf> {
    let do_jogo = red4.join(SCC_DO_JOGO);
    if do_jogo.is_file() && red4.join(LIB_DO_SCC).is_file() {
        return Some(do_jogo);
    }
    let texto = std::fs::read_to_string(red4.join("bwms-scc.txt")).ok()?;
    let p = PathBuf::from(texto.trim());
    (p.is_absolute() && p.is_file()).then_some(p)
}

/// O texto do manifesto dos plugins (`scripts_add` da API); sem o arquivo, vazio.
pub(crate) fn texto_do_manifesto() -> String {
    crate::mods_do_usuario::api::scripts_add_manifest_path().and_then(|m| std::fs::read_to_string(m).ok()).unwrap_or_default()
}

/// Um mod de `.reds`: a pasta de 1º nível (ou o arquivo solto) da `Mods/` ou de `r6/scripts`, ou um item do manifesto dos plugins, com os
/// `.reds` dele na ordem do passe. A conferência das natives (`barrados`) tira da compilação o mod inteiro.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModDeScripts {
    pub(crate) raiz: PathBuf,
    pub(crate) reds: Vec<PathBuf>,
}

/// Os `.reds` de `reds` (em ordem de caminho, todos debaixo de `base`) agrupados pelo mod: a pasta de 1º nível de `base`, ou o arquivo
/// solto nela. Na ordem de caminho os arquivos de um mod vêm juntos.
fn por_mod(base: &Path, reds: impl IntoIterator<Item = PathBuf>) -> Vec<ModDeScripts> {
    let mut mods: Vec<ModDeScripts> = Vec::new();
    for p in reds {
        let Some(primeiro) = p.strip_prefix(base).ok().and_then(|r| r.components().next()) else { continue };
        let raiz = base.join(primeiro);
        match mods.last_mut() {
            Some(m) if m.raiz == raiz => m.reds.push(p),
            _ => mods.push(ModDeScripts { raiz, reds: vec![p] }),
        }
    }
    mods
}

/// Os mods de `r6/scripts` de fora do bundle do BWMS; `reds` são os `.reds` da árvore em ordem de caminho.
pub(crate) fn mods_de_r6_scripts(scripts: &Path, reds: &[PathBuf]) -> Vec<ModDeScripts> {
    let bwms = scripts.join(PASTA_DO_BWMS);
    por_mod(scripts, reds.iter().filter(|p| !p.starts_with(&bwms)).cloned())
}

/// Os mods que o `scc` recebe além de `r6/scripts`, na ordem do passe: os da pasta `Mods/` (ordem de caminho) e os itens do manifesto dos
/// plugins (ordem do arquivo; uma pasta no manifesto entra com os `.reds` dela). Caminho repetido entra uma vez, e o que já está debaixo
/// de `r6/scripts` fica de fora: o `scc` compilaria o arquivo duas vezes e recusaria as definições repetidas. Mod sem `.reds` sai.
pub(crate) fn mods_extras(jogo: &Path, manifesto: &str) -> Vec<ModDeScripts> {
    use crate::mods_do_usuario::pasta_mods;
    let scripts = jogo.join("r6/scripts");
    let pasta = pasta_mods::pasta(jogo);
    let da_pasta = por_mod(&pasta, pasta_mods::reds(&pasta));
    let do_manifesto = manifesto.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| {
        let p = Path::new(l);
        let reds = if p.is_dir() {
            pasta_mods::reds(p)
        } else if p.is_file() {
            vec![p.to_path_buf()]
        } else {
            Vec::new()
        };
        ModDeScripts { raiz: p.to_path_buf(), reds }
    });
    let mut vistos = std::collections::HashSet::new();
    let mut mods = Vec::new();
    for mut m in da_pasta.into_iter().chain(do_manifesto) {
        m.reds.retain(|p| !p.starts_with(&scripts) && vistos.insert(p.clone()));
        if !m.reds.is_empty() {
            mods.push(m);
        }
    }
    mods
}

/// Os binários que registram natives: a dylib do BWMS (sempre a 1ª) e os plugins de `red4ext/plugins/` em ordem de caminho, a mesma lista
/// do `bwms mods checar`.
pub(crate) fn binarios(red4: &Path) -> Vec<PathBuf> {
    let mut plugins: Vec<PathBuf> = std::fs::read_dir(red4.join("plugins"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "dylib"))
        .collect();
    plugins.sort();
    std::iter::once(red4.join(DYLIB_DO_BWMS)).chain(plugins).collect()
}

/// O executável em que o dylib roda: no jogo, o binário do jogo, que registra natives próprias (`registrada_no_jogo`).
#[cfg(not(test))]
fn executavel_do_jogo() -> Vec<u8> {
    std::env::current_exe().and_then(std::fs::read).unwrap_or_default()
}

/// Nos testes o executável é o binário de teste, que leva como texto os nomes de native que os testes declaram sem registro.
#[cfg(test)]
fn executavel_do_jogo() -> Vec<u8> {
    Vec::new()
}

/// Um mod que declara `native func` que nenhum dos binários registra, com cada declaração como `arquivo:linha Dono.nome` e, na mesma
/// ordem, o nome dela (o que o blob guarda, sem o dono).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Barrado {
    pub(crate) raiz: PathBuf,
    pub(crate) natives: Vec<String>,
    pub(crate) nomes: Vec<String>,
}

/// Os mods de `mods` com `native func` sem registro em nenhum dos `binarios`, pela regra do `bwms-helper natives`. No jogo, uma native
/// declarada sem registro derruba o bind dos scripts com SIGTRAP, sem mensagem: é o mod de Windows que declara as natives do próprio plugin
/// (DLL), que não existe no Mac. Os binários só são lidos quando algum `.reds` declara native, e o `.reds` que não abre fica para o `scc`,
/// que aponta o erro. `Err` quando a dylib do BWMS não é lida: sem ela, toda native pareceria sem registro.
pub(crate) fn barrados(mods: &[ModDeScripts], binarios: &[PathBuf]) -> std::io::Result<Vec<Barrado>> {
    // (mod, nome da native, a declaração como `arquivo:linha Dono.nome`)
    let mut declaradas: Vec<(&ModDeScripts, String, String)> = Vec::new();
    for m in mods {
        for arquivo in &m.reds {
            let Ok(bytes) = std::fs::read(arquivo) else { continue };
            let texto = String::from_utf8_lossy(&bytes);
            for d in declaradas_em(&texto) {
                let dono = d.dono.map(|c| format!("{c}.")).unwrap_or_default();
                declaradas.push((m, d.nome.to_string(), format!("{}:{} {dono}{}", arquivo.display(), d.linha, d.nome)));
            }
        }
    }
    if declaradas.is_empty() {
        return Ok(Vec::new());
    }
    let mut lidos = Vec::with_capacity(binarios.len());
    for (i, b) in binarios.iter().enumerate() {
        match std::fs::read(b) {
            Ok(bytes) => lidos.push(bytes),
            Err(e) if i == 0 => return Err(e),
            // Plugin que não abre não registra nada: o mod que dependia dele fica fora.
            Err(_) => {}
        }
    }
    // Cada nome varre os binários inteiros (`registrada_em`): o nome repetido em outra declaração não varre de novo. O executável do jogo
    // só é lido para o nome que nenhum binário do BWMS registra (`registrada_no_jogo`).
    let mut conferidas = std::collections::HashMap::new();
    let mut jogo: Option<Vec<u8>> = None;
    let mut barrados: Vec<Barrado> = Vec::new();
    for (m, nome, declaracao) in declaradas {
        if *conferidas
            .entry(nome.clone())
            .or_insert_with_key(|n| registrada_em(n, &lidos) || registrada_no_jogo(n, jogo.get_or_insert_with(executavel_do_jogo)))
        {
            continue;
        }
        match barrados.last_mut() {
            Some(b) if b.raiz == m.raiz => {
                b.natives.push(declaracao);
                b.nomes.push(nome);
            }
            _ => barrados.push(Barrado { raiz: m.raiz.clone(), natives: vec![declaracao], nomes: vec![nome] }),
        }
    }
    Ok(barrados)
}

/// Os `.reds` de `mods` sem os dos mods de `barrados`, na ordem do passe.
pub(crate) fn permitidos(mods: &[ModDeScripts], barrados: &[Barrado]) -> Vec<PathBuf> {
    mods.iter().filter(|m| !barrados.iter().any(|b| b.raiz == m.raiz)).flat_map(|m| m.reds.iter().cloned()).collect()
}

/// Grava em `red4/` a lista do `-compilePathsFile`: um caminho por linha, que o `scc` toma como veio (arquivo ou pasta). Sem extras, apaga
/// a lista do passe anterior e devolve `None`. Caminho que não é UTF-8 fica fora: o `scc` lê a lista com `lines()`, e uma linha inválida
/// descarta a lista inteira.
pub(crate) fn gravar_lista(red4: &Path, extras: &[PathBuf]) -> std::io::Result<Option<PathBuf>> {
    let lista = red4.join(LISTA_DE_CAMINHOS);
    let linhas: Vec<&str> = extras.iter().filter_map(|p| p.to_str()).collect();
    if linhas.is_empty() {
        let _ = std::fs::remove_file(&lista);
        return Ok(None);
    }
    std::fs::write(&lista, linhas.join("\n"))?;
    Ok(Some(lista))
}

/// O `scc` com as fontes do passe: `r6/scripts` pelo `-compile` e a lista dos extras pelo `-compilePathsFile`, sem o ambiente do jogo.
/// A dylib que a trilha de dev ou o overlay da Steam injetam por variável de ambiente, carregada no compilador, rodaria o `on_load` (o
/// atraso do boot, o dead-man's switch, o relatório de crash) num processo que não é o jogo; e o `scc` não lê variável nenhuma (acha a
/// lib pelo próprio executável, `scc/cli/src/main.rs`). Os caminhos são absolutos: a pasta corrente do jogo não serve de base.
pub(crate) fn comando_do_scc(scc: &Path, scripts: &Path, lista: Option<&Path>) -> Command {
    let mut cmd = Command::new(scc);
    cmd.arg("-compile").arg(scripts).env_clear();
    if let Some(lista) = lista {
        cmd.arg("-compilePathsFile").arg(lista);
    }
    cmd
}

/// Compila os `.reds` do jogo pro caminho dado. Saída fora do `r6/cache` serve pro teste da troca
/// de corpo, que precisa do blob novo sem mexer no que o motor carregou.
pub(crate) fn compilar_em(saida: &Path) -> Result<(), String> {
    let scc = scc().ok_or("sem o compilador (red4ext/scc ou red4ext/bwms-scc.txt)")?;
    let red4 = PathBuf::from(crate::dylib_dir().ok_or("sem a pasta do jogo")?);
    let jogo = red4.parent().ok_or("sem a pasta do jogo")?.to_path_buf();
    // As mesmas fontes da compilação na abertura (`compilacao_no_lancamento`), sem os mods barrados e sem os que saíram dela por erro: sem os
    // `.reds` da `Mods/`, o blob quente perderia as funções deles e a troca de corpo recusaria por função removida; com o mod que saiu, o
    // blob quente não compilaria, ou ganharia funções que o carregado não tem.
    let extras = mods_extras(&jogo, &texto_do_manifesto());
    let barrados = barrados(&extras, &binarios(&red4)).map_err(|e| format!("{DYLIB_DO_BWMS} não lida: {e}"))?;
    let estado = super::compilacao_no_lancamento::estado_dos_scripts();
    let fora: Vec<&PathBuf> = (0..estado.raizes.len())
        .filter(|&i| estado.cores[i] == crate::mods_do_usuario::pasta_mods::CorDoMod::Vermelho)
        .map(|i| &estado.raizes[i])
        .collect();
    let permitidos: Vec<PathBuf> = permitidos(&extras, &barrados).into_iter().filter(|p| !fora.iter().any(|r| p.starts_with(r))).collect();
    let lista = gravar_lista(&red4, &permitidos).map_err(|e| format!("lista de caminhos em {}: {e}", red4.display()))?;
    let saida = comando_do_scc(&scc, &jogo.join("r6/scripts"), lista.as_deref()).arg(saida).output().map_err(|e| format!("scc: {e}"))?;
    let log = std::fs::read_to_string(jogo.join("r6/logs/redscript_rCURRENT.log")).unwrap_or_default();
    if !saida.status.success() || log.contains("[ERROR") {
        return Err(log.lines().filter(|l| l.contains("ERROR")).take(4).collect::<Vec<_>>().join(" | "));
    }
    Ok(())
}

/// Recompilação automática pro modder (ligada pelo arquivo `red4ext/bwms-recarga-de-scripts`): a cada 3 s, a data mais nova dos `.reds` de
/// `r6/scripts`; mudou → recompila numa thread pro blob quente. Erro do compilador vai pro log de mod; compilou, o `aplicar_tick` troca o corpo
/// das funções mudadas na thread do jogo.
pub(crate) fn vigia_tick() {
    static ULTIMA_OLHADA: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    static DATA: AtomicU64 = AtomicU64::new(0);
    static COMPILANDO: AtomicBool = AtomicBool::new(false);
    if COMPILANDO.load(Ordering::Acquire) {
        return;
    }
    {
        let Ok(mut u) = ULTIMA_OLHADA.try_lock() else { return };
        if u.is_some_and(|t| t.elapsed().as_secs() < 3) {
            return;
        }
        *u = Some(std::time::Instant::now());
    }
    let Some(jogo) = crate::dylib_dir().and_then(|d| std::path::PathBuf::from(d).parent().map(|p| p.to_path_buf())) else { return };
    if !jogo.join("red4ext/bwms-recarga-de-scripts").exists() {
        DATA.store(0, Ordering::Release);
        return;
    }
    let data = data_mais_nova(&jogo.join("r6/scripts"));
    let antes = DATA.swap(data, Ordering::AcqRel);
    if antes == 0 || data <= antes {
        return;
    }
    COMPILANDO.store(true, Ordering::Release);
    std::thread::spawn(move || {
        // Compila num blob separado: o `r6/cache` só é trocado depois que a aplicação der certo, pra
        // que o blob carregado siga sendo a referência da comparação.
        match crate::scripts::tradutor_de_bytecode::compilar_quente() {
            Ok(()) => APLICAR.store(true, Ordering::Release),
            Err(e) => crate::mods_do_usuario::log_de_mod::gravar("bwms", &format!("scripts não compilaram, nada aplicado: {e}")),
        }
        COMPILANDO.store(false, Ordering::Release);
    });
}

/// Pedido de aplicação a quente esperando a thread do jogo.
static APLICAR: AtomicBool = AtomicBool::new(false);

/// Na thread do jogo: aplica o blob que o vigia compilou, trocando o corpo das funções mudadas
/// (RED4ext `#97`). Função criada ou removida não entra a quente — aí o relato pede reiniciar.
pub(crate) fn aplicar_tick() {
    if !APLICAR.swap(false, Ordering::AcqRel) || crate::inicio::boot::exec_nested() {
        return;
    }
    let t0 = std::time::Instant::now();
    match unsafe { crate::scripts::tradutor_de_bytecode::aplicar_a_quente() } {
        Ok(relato) => crate::mods_do_usuario::log_de_mod::gravar("bwms", &format!("{relato} em {} ms, sem reiniciar", t0.elapsed().as_millis())),
        Err(e) => crate::mods_do_usuario::log_de_mod::gravar("bwms", &format!("não aplicado a quente ({e}) — reinicie o jogo pra valer")),
    }
}

/// A data de modificação mais nova (segundos) dos `.reds` sob `dir`.
fn data_mais_nova(dir: &std::path::Path) -> u64 {
    let mut maior = 0;
    let mut pilha = vec![dir.to_path_buf()];
    while let Some(d) = pilha.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                pilha.push(p);
            } else if p.extension().is_some_and(|x| x == "reds") {
                let t = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
                maior = maior.max(t.map_or(0, |t| t.as_secs()));
            }
        }
    }
    maior
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arquivo(raiz: &Path, caminho: &str) -> PathBuf {
        escrito(raiz, caminho, "x")
    }

    fn escrito(raiz: &Path, caminho: &str, conteudo: &str) -> PathBuf {
        let p = raiz.join(caminho);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, conteudo).unwrap();
        p
    }

    fn pasta_de_teste(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-carregador-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn extras_vem_da_mods_e_do_manifesto_sem_repetir_nem_r6_scripts() {
        let raiz = pasta_de_teste("extras");
        let jogo = raiz.join("jogo");
        let mods_b = arquivo(&jogo, "Mods/B/r6/scripts/b.reds");
        let mods_a = arquivo(&jogo, "Mods/A/a.reds");
        let mods_a2 = arquivo(&jogo, "Mods/A/sub/a2.reds");
        let solto = arquivo(&jogo, "Mods/solto.reds");
        let em_scripts = arquivo(&jogo, "r6/scripts/ja-compila.reds");
        let avulso = arquivo(&raiz, "plugin/um.reds");
        let pasta_2 = arquivo(&raiz, "plugin2/sub/dois.reds");
        let manifesto = format!(
            "{}\n\n   \n{}\n{}\n{}\n{}\n{}\n",
            avulso.display(),
            raiz.join("plugin2").display(),
            raiz.join("nao-existe.reds").display(),
            em_scripts.display(),
            mods_a.display(),
            avulso.display()
        );
        let mods = mods_extras(&jogo, &manifesto);
        let esperado = [
            (jogo.join("Mods/A"), vec![mods_a.clone(), mods_a2.clone()]),
            (jogo.join("Mods/B"), vec![mods_b.clone()]),
            (solto.clone(), vec![solto.clone()]),
            (avulso.clone(), vec![avulso.clone()]),
            (raiz.join("plugin2"), vec![pasta_2.clone()]),
        ];
        assert_eq!(mods, esperado.map(|(raiz, reds)| ModDeScripts { raiz, reds }));
        assert_eq!(permitidos(&mods, &[]), [mods_a, mods_a2, mods_b, solto, avulso, pasta_2]);
        assert!(mods_extras(&raiz.join("sem-jogo"), "").is_empty());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn mods_de_r6_scripts_deixam_o_bundle_do_bwms_de_fora() {
        let scripts = PathBuf::from("/j/r6/scripts");
        let reds = ["Arquivo.reds", "blackwall-mods/a.reds", "blackwall-mods/b.reds", "ModX/a.reds", "ModX/sub/b.reds", "ModY/y.reds"]
            .map(|r| scripts.join(r));
        let mods = mods_de_r6_scripts(&scripts, &reds);
        let raizes: Vec<&Path> = mods.iter().map(|m| m.raiz.as_path()).collect();
        assert_eq!(raizes, [scripts.join("Arquivo.reds"), scripts.join("ModX"), scripts.join("ModY")]);
        assert_eq!(mods[1].reds, [scripts.join("ModX/a.reds"), scripts.join("ModX/sub/b.reds")]);
    }

    #[test]
    fn native_sem_registro_barra_o_mod_inteiro_e_so_ele() {
        let raiz = pasta_de_teste("barrados");
        let red4 = raiz.join("red4ext");
        escrito(&red4, DYLIB_DO_BWMS, "...BwmsRegistrada...");
        escrito(&red4, "plugins/um.dylib", "\0DoPlugin\0");
        escrito(&red4, "plugins/leia-me.txt", "NaoRegistrada");
        let windows = escrito(
            &raiz,
            "Mods/ModWindows/r6/scripts/plugin.reds",
            "module ModWindows\n\nnative func NaoRegistrada() -> Bool\n\
             public native class Cliente extends IScriptable {\n  native func TambemNao() -> Int32\n}\n",
        );
        let outro_do_mesmo = arquivo(&raiz, "Mods/ModWindows/r6/scripts/ui.reds");
        let usa_o_bwms = escrito(&raiz, "Mods/Bom/b.reds", "native func BwmsRegistrada()\npublic static native func DoPlugin()\n");
        let sem_native = arquivo(&raiz, "Mods/Zeta/z.reds");
        let pasta = raiz.join("Mods");
        let mods = por_mod(&pasta, [usa_o_bwms.clone(), windows.clone(), outro_do_mesmo, sem_native.clone()]);
        let b = barrados(&mods, &binarios(&red4)).unwrap();
        assert_eq!(
            b,
            [Barrado {
                raiz: pasta.join("ModWindows"),
                natives: vec![format!("{}:3 NaoRegistrada", windows.display()), format!("{}:5 Cliente.TambemNao", windows.display())],
                nomes: vec!["NaoRegistrada".into(), "TambemNao".into()],
            }]
        );
        assert_eq!(permitidos(&mods, &b), [usa_o_bwms, sem_native]);
        // Sem native declarada, nenhum binário é lido: a dylib ausente não importa.
        let sem_natives: Vec<ModDeScripts> = mods.into_iter().filter(|m| m.raiz.ends_with("Zeta")).collect();
        assert_eq!(barrados(&sem_natives, &[raiz.join("nao-existe.dylib")]).unwrap(), []);
        assert!(barrados(&[ModDeScripts { raiz: pasta.clone(), reds: vec![windows] }], &[raiz.join("nao-existe.dylib")]).is_err());
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn binarios_sao_a_dylib_e_os_plugins() {
        let raiz = pasta_de_teste("binarios");
        arquivo(&raiz, "plugins/b.dylib");
        arquivo(&raiz, "plugins/a.dylib");
        arquivo(&raiz, "plugins/c.txt");
        assert_eq!(binarios(&raiz), [raiz.join(DYLIB_DO_BWMS), raiz.join("plugins/a.dylib"), raiz.join("plugins/b.dylib")]);
        assert_eq!(binarios(&raiz.join("sem-plugins")), [raiz.join("sem-plugins").join(DYLIB_DO_BWMS)]);
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn lista_tem_um_caminho_por_linha_e_sai_sem_extras() {
        let red4 = pasta_de_teste("lista");
        assert!(matches!(gravar_lista(&red4, &[]), Ok(None)));
        assert!(!red4.join(LISTA_DE_CAMINHOS).exists());
        let extras = [PathBuf::from("/a b/um.reds"), PathBuf::from("/c/dois.reds")];
        let lista = gravar_lista(&red4, &extras).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&lista).unwrap(), "/a b/um.reds\n/c/dois.reds");
        assert!(matches!(gravar_lista(&red4, &[]), Ok(None)));
        assert!(!lista.exists(), "a lista do passe anterior não fica dizendo o que entrou");
        assert!(gravar_lista(&red4.join("nao-existe"), &extras).is_err());
        let _ = std::fs::remove_dir_all(&red4);
    }

    #[test]
    fn comando_do_scc_roda_sem_o_ambiente_do_jogo_e_leva_a_lista() {
        let cmd = comando_do_scc(Path::new("/k/scc"), Path::new("/j/r6/scripts"), Some(Path::new("/j/red4ext/lista.txt")));
        let args: Vec<&std::ffi::OsStr> = cmd.get_args().collect();
        assert_eq!(args, ["-compile", "/j/r6/scripts", "-compilePathsFile", "/j/red4ext/lista.txt"]);
        assert_eq!(comando_do_scc(Path::new("/k/scc"), Path::new("/s"), None).get_args().count(), 2);
        // O filho não herda nada: um `scc` de mentira grava o ambiente que recebeu.
        let raiz = pasta_de_teste("ambiente");
        let scc = escrito(&raiz, "scc", &format!("#!/bin/sh\nenv > \"{}/ambiente\"\n", raiz.display()));
        std::fs::set_permissions(&scc, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert!(comando_do_scc(&scc, &raiz, None).status().unwrap().success());
        let ambiente = std::fs::read_to_string(raiz.join("ambiente")).unwrap();
        assert!(std::env::var_os("HOME").is_some(), "o teste precisa de uma variável no pai para ver que ela não passa");
        assert!(!ambiente.lines().any(|l| l.starts_with("HOME=")), "ambiente do filho: {ambiente}");
        let _ = std::fs::remove_dir_all(&raiz);
    }

    #[test]
    fn scc_da_pasta_do_jogo_vem_antes_do_caminho_gravado() {
        let raiz = pasta_de_teste("scc");
        let red4 = raiz.join("red4ext");
        std::fs::create_dir_all(&red4).unwrap();
        assert_eq!(scc_em(&red4), None);
        let do_kit = arquivo(&raiz, "kit/tools/scc");
        std::fs::write(red4.join("bwms-scc.txt"), format!("{}\n", do_kit.display())).unwrap();
        assert_eq!(scc_em(&red4), Some(do_kit.clone()));
        // Relativo seria resolvido pela pasta corrente do jogo.
        std::fs::write(red4.join("bwms-scc.txt"), "kit/tools/scc").unwrap();
        assert_eq!(scc_em(&red4), None);
        // A cópia em `red4ext/` só vale com a lib ao lado; sem ela, o caminho gravado.
        std::fs::write(red4.join("bwms-scc.txt"), do_kit.to_str().unwrap()).unwrap();
        arquivo(&red4, SCC_DO_JOGO);
        assert_eq!(scc_em(&red4), Some(do_kit.clone()));
        arquivo(&red4, LIB_DO_SCC);
        assert_eq!(scc_em(&red4), Some(red4.join(SCC_DO_JOGO)));
        // O usuário apaga o kit baixado: a cópia do jogo continua servindo.
        std::fs::remove_dir_all(raiz.join("kit")).unwrap();
        assert_eq!(scc_em(&red4), Some(red4.join(SCC_DO_JOGO)));
        let _ = std::fs::remove_dir_all(&raiz);
    }
}
