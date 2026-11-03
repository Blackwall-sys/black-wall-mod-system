//! Os `.archive` da `Mods/` entregues à pasta que o próprio jogo monta, `<jogo>/archive/Mac/mod/`: sem gancho, sem conta e sem rede, o jogo
//! a carrega no scope 6, acima de `patch`/`ep1`/`content` (`relatorios/A.md`, RE do `InitializeArchives` `0x10151a118` → `0x10151bc90`).
//!
//! Cada archive vira um clone APFS (`clonefile(2)`, sem dado copiado) em `archive/Mac/mod/BWMS-Mods/<NNNN>/<nome>`, com `NNNN` a posição na
//! ordem de carga da `Mods/` (vence o 1º). A ordem sai do nome sem `modlist.txt`: o jogo lista a pasta com glob recursivo, ordena pelo
//! caminho inteiro com `strcmp` (`0x10151e5e4` → `0x1000303d8` → `_strcmp`) e, dentro do conjunto, o 1º archive que tem o recurso serve
//! (`RequestResource` `0x10151a748`). O `modlist.txt` da pasta é do usuário: só entradas dele vêm antes, e ele não é lido nas subpastas. O nome
//! do arquivo fica o mesmo da `Mods/` porque o `ArchiveExists` dos mods compara o nome.
//!
//! `BWMS-Mods/` é do BWMS (o nome é nosso) e, a cada abertura, espelha a `Mods/` atual: o que está lá dentro e não é o destino de um
//! `.archive` aceito da `Mods/` sai, por caminho exato, sem seguir link (o link sai, o alvo dele fica). Sem lista do que foi posto: o jogo
//! carrega todo `*.archive` da pasta, então o que importa é o que está nela, e um clone que uma lista perdeu ficaria montado sem o BWMS
//! saber. Fora de `BWMS-Mods/` nada é tocado. O que não chega à pasta (sem `archive/Mac/`, clone e cópia que falham) e o clone que o jogo não
//! montou ficam fora do jogo: o relatório da `Mods/` diz qual e por quê (`pasta_mods::conferir_montagem`).
//!
//! A mesma passada põe em `BWMS-Mods/xl/` os `.json` da parte dos `.xl` que o jogo aplica (`bwms_primitives::instalacao::xl_no_jogo`), no mesmo
//! espelho: `.json` que o plano não pede sai. É tudo ou nada: com um `.json` que não gravou, os outros saem; os textos o BWMS aplica (as
//! tabelas só os tiram com todos no disco, `xl_no_jogo::no_disco`), e o `factories` fica sem efeito, com o mod amarelo (`pasta_mods`).

use crate::arquivos_do_jogo::Recusa;
use bwms_primitives::instalacao::xl_no_jogo::ConfigDoJogo;
use std::collections::HashSet;
use std::ffi::{c_char, c_int, CString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// A pasta de archives de mod que o jogo monta sozinho (`0x10442f338` + `0x10151c340`: `<base>/archive/mac/mod`; o APFS não distingue caixa).
pub(crate) const PASTA_NATIVA: &str = "archive/Mac/mod";

/// A subpasta do BWMS dentro dela.
pub(crate) const PASTA_DO_BWMS: &str = "BWMS-Mods";

/// `CLONE_NOFOLLOW` de `<sys/clonefile.h>`.
const CLONE_NOFOLLOW: u32 = 0x0001;

extern "C" {
    fn clonefile(src: *const c_char, dst: *const c_char, flags: u32) -> c_int;
}

/// Como o archive chegou à pasta nativa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Via {
    /// Clone APFS: o arquivo divide os blocos com o da `Mods/`.
    Clone,
    /// Cópia normal: o volume não faz clone.
    Copia,
    /// Já estava lá, do mesmo tamanho e data que o da `Mods/`.
    Mantido,
}

/// Por que um archive aceito ficou fora do jogo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MotivoDeFora {
    /// O jogo não tem `archive/Mac/`.
    SemPastaDoJogo,
    /// Clone e cópia falharam.
    NaoCopiou(String),
    /// O jogo não montou o clone (`pasta_mods::conferir_montagem`).
    JogoNaoMontou,
}

impl MotivoDeFora {
    /// `(PT-BR, inglês)`, cada um com a ação.
    pub(crate) fn texto(&self) -> (String, String) {
        match self {
            MotivoDeFora::SemPastaDoJogo => (
                "o jogo não tem a pasta archive/Mac; verifique a integridade dos arquivos do jogo na Steam e abra o jogo de novo".into(),
                "the game has no archive/Mac folder; verify the integrity of the game files in Steam and start the game again".into(),
            ),
            MotivoDeFora::NaoCopiou(e) => (
                format!(
                    "não deu para pôr o arquivo em archive/Mac/mod ({e}); confira o espaço livre e a permissão da pasta do jogo e abra o jogo de novo"
                ),
                format!(
                    "the file could not be placed in archive/Mac/mod ({e}); check the free space and permissions of the game folder and start the \
                     game again"
                ),
            ),
            MotivoDeFora::JogoNaoMontou => (
                "o jogo não montou o arquivo da pasta archive/Mac/mod; baixe o mod de novo e abra o jogo de novo".into(),
                "the game did not mount the file from archive/Mac/mod; download the mod again and start the game again".into(),
            ),
        }
    }
}

/// O resultado da sincronização de uma abertura.
#[derive(Debug, Default)]
pub(crate) struct Sincronia {
    /// `(archive da Mods/, destino na pasta nativa, via)` na ordem de carga: o que está na pasta agora.
    pub(crate) nativos: Vec<(PathBuf, PathBuf, Via)>,
    /// Os archives aceitos que não chegaram à pasta, na ordem de carga.
    pub(crate) fora: Vec<(PathBuf, MotivoDeFora)>,
    /// Os que não entram por caminho nenhum.
    pub(crate) recusados: Vec<(PathBuf, Recusa)>,
    /// Entradas que saíram de `BWMS-Mods/` (arquivo ou link; pasta não conta) porque não são destino de um `.archive` nem de um `.json` atual.
    pub(crate) removidos: usize,
    /// Os `.json` do plano que estão no disco agora, com o conteúdo do plano.
    pub(crate) jsons: Vec<PathBuf>,
    /// O erro do `.json` que não gravou; com ele, nenhum `.json` fica.
    pub(crate) json_falhou: Option<String>,
}

/// O destino de cada archive, relativo ao jogo: `archive/Mac/mod/BWMS-Mods/<NNNN>/<nome>`, `NNNN` a posição (de 1) com o mesmo número de
/// dígitos para todos (no mínimo 4): o `strcmp` do jogo põe `0010` depois de `0009`, e não `10` antes de `9`.
fn destinos(nomes: &[&std::ffi::OsStr]) -> Vec<PathBuf> {
    let digitos = nomes.len().to_string().len().max(4);
    nomes
        .iter()
        .enumerate()
        .map(|(i, n)| Path::new(PASTA_NATIVA).join(PASTA_DO_BWMS).join(format!("{:0digitos$}", i + 1)).join(n))
        .collect()
}

/// Mesmo tamanho e mesma data de modificação: o destino é cópia em dia da origem.
fn em_dia(origem: &Path, destino: &Path) -> bool {
    let (Ok(a), Ok(b)) = (std::fs::metadata(origem), std::fs::symlink_metadata(destino)) else { return false };
    b.is_file() && a.len() == b.len() && a.modified().ok().is_some_and(|m| b.modified().ok() == Some(m))
}

/// Clona `origem` em `destino` (que não existe); sem clone no volume, copia. A data de modificação fica a da origem nos dois casos, para o
/// `em_dia` da abertura seguinte.
fn clonar_ou_copiar(origem: &Path, destino: &Path) -> std::io::Result<Via> {
    let (Ok(o), Ok(d)) = (CString::new(origem.as_os_str().as_bytes()), CString::new(destino.as_os_str().as_bytes())) else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "caminho com byte nulo"));
    };
    // SAFETY: os dois caminhos são `CString` válidos até o fim da chamada.
    let via = if unsafe { clonefile(o.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) } == 0 {
        Via::Clone
    } else {
        std::fs::copy(origem, destino)?;
        Via::Copia
    };
    let data = std::fs::metadata(origem)?.modified()?;
    std::fs::File::options().write(true).open(destino)?.set_modified(data)?;
    Ok(via)
}

/// Põe `origem` em `destino` por um temporário oculto ao lado (o glob `*.archive` do jogo não o vê) e `rename`, trocando o que estava lá.
fn por(origem: &Path, destino: &Path) -> std::io::Result<Via> {
    let pai = destino.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "destino sem pasta"))?;
    std::fs::create_dir_all(pai)?;
    let tmp = temporario(destino);
    let _ = std::fs::remove_file(&tmp);
    let via = clonar_ou_copiar(origem, &tmp);
    match via {
        Ok(v) => std::fs::rename(&tmp, destino).map(|_| v).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn temporario(destino: &Path) -> PathBuf {
    let nome = destino.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    destino.with_file_name(format!(".{nome}.bwms-tmp"))
}

/// Tira de `dir` tudo que não é um arquivo comum num caminho de `manter`, sem seguir link: o tipo vem da entrada (`lstat`), o link sai com
/// `unlink` e a pasta é descida e sai se ficou vazia. Devolve quantas entradas que não são pasta saíram.
fn limpar(dir: &Path, manter: &HashSet<PathBuf>) -> usize {
    // A lista inteira antes de apagar: apagar durante o `readdir` deixa a leitura seguinte indefinida (POSIX) e pode pular entrada.
    let Ok(entradas) = std::fs::read_dir(dir).map(|l| l.flatten().collect::<Vec<_>>()) else { return 0 };
    let mut tiradas = 0;
    for e in entradas {
        let p = e.path();
        let Ok(tipo) = e.file_type() else { continue };
        if tipo.is_dir() {
            tiradas += limpar(&p, manter);
            let _ = std::fs::remove_dir(&p);
        } else if !(tipo.is_file() && manter.contains(&p)) {
            match std::fs::remove_file(&p) {
                Ok(()) => tiradas += 1,
                Err(erro) => crate::log_dev!("[mods] {} não saiu de {PASTA_DO_BWMS}: {erro}", p.display()),
            }
        }
    }
    tiradas
}

/// Deixa em `raiz` (`BWMS-Mods/`) só os arquivos comuns de `manter`. `raiz` que é link ou arquivo sai inteira: o nome é do BWMS, e sair por
/// `unlink` não toca o alvo.
fn espelhar(raiz: &Path, manter: &HashSet<PathBuf>) -> usize {
    match std::fs::symlink_metadata(raiz) {
        Err(_) => 0,
        Ok(m) if m.is_dir() => limpar(raiz, manter),
        Ok(_) => usize::from(std::fs::remove_file(raiz).is_ok()),
    }
}

/// Sincroniza `archives` (os da `Mods/`, na ordem de carga) e os `.json` de `configs` com `<jogo>/archive/Mac/mod/BWMS-Mods/`. Roda no
/// `on_load`, antes do `InitializeArchives` do jogo e da leitura das configs (`0x101cbd304`).
pub(crate) fn sincronizar(jogo: &Path, archives: &[PathBuf], configs: &[ConfigDoJogo]) -> Sincronia {
    let mut s = Sincronia::default();
    // Arquivo inválido derruba o arquivo no assert do motor e abre o popup de erro do mod.io; nome repetido deixa o `ArchiveExists` ambíguo.
    let mut nomes_vistos: HashSet<&std::ffi::OsStr> = HashSet::new();
    let mut aceitos: Vec<&Path> = Vec::new();
    for a in archives {
        let nome = a.file_name().unwrap_or_default();
        if a.to_str().and_then(crate::arquivos_do_jogo::archive_valido).is_none() {
            s.recusados.push((a.clone(), Recusa::Invalido));
        } else if !nomes_vistos.insert(nome) {
            s.recusados.push((a.clone(), Recusa::NomeRepetido));
        } else {
            aceitos.push(a);
        }
    }
    let alvos: Vec<PathBuf> =
        destinos(&aceitos.iter().map(|a| a.file_name().unwrap_or_default()).collect::<Vec<_>>()).iter().map(|d| jogo.join(d)).collect();
    let raiz = jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS);
    let pasta_do_jogo = jogo.join(PASTA_NATIVA).parent().is_some_and(Path::is_dir);
    let jsons: Vec<(PathBuf, &str)> =
        if pasta_do_jogo { configs.iter().map(|c| (jogo.join(&c.destino), c.conteudo.as_str())).collect() } else { Vec::new() };
    s.removidos = espelhar(&raiz, &alvos.iter().cloned().chain(jsons.iter().map(|(p, _)| p.clone())).collect());
    match por_jsons(&jsons) {
        Ok(()) => s.jsons = jsons.into_iter().map(|(p, _)| p).collect(),
        Err(e) => {
            for (p, _) in &jsons {
                let _ = std::fs::remove_file(p);
            }
            s.json_falhou = Some(e.to_string());
        }
    }
    if !pasta_do_jogo {
        s.fora = aceitos.iter().map(|a| (a.to_path_buf(), MotivoDeFora::SemPastaDoJogo)).collect();
        return s;
    }
    for (origem, destino) in aceitos.iter().zip(alvos) {
        let resultado = if em_dia(origem, &destino) { Ok(Via::Mantido) } else { por(origem, &destino) };
        match resultado {
            Ok(via) => s.nativos.push((origem.to_path_buf(), destino, via)),
            Err(e) => {
                // O que estava no destino é de outra versão do archive: com ele lá o jogo montaria o conteúdo velho, e o relatório diria
                // que o archive ficou fora.
                let _ = std::fs::remove_file(&destino);
                if let Some(pai) = destino.parent() {
                    let _ = std::fs::remove_dir(pai);
                }
                s.fora.push((origem.to_path_buf(), MotivoDeFora::NaoCopiou(e.to_string())));
            }
        }
    }
    if s.nativos.is_empty() {
        let _ = std::fs::remove_dir(&raiz);
    }
    s
}

/// Grava cada `.json` cujo conteúdo no disco difere, por um temporário oculto ao lado (o glob `*.json` do jogo não o vê) e `rename`. Para no 1º
/// erro.
fn por_jsons(jsons: &[(PathBuf, &str)]) -> std::io::Result<()> {
    for (destino, conteudo) in jsons {
        if std::fs::read(destino).is_ok_and(|d| d == conteudo.as_bytes()) {
            continue;
        }
        let pai = destino.parent().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "destino sem pasta"))?;
        std::fs::create_dir_all(pai)?;
        let tmp = temporario(destino);
        std::fs::write(&tmp, conteudo).and_then(|()| std::fs::rename(&tmp, destino)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
    }
    Ok(())
}

/// O sufixo pelo qual o depot mostra o destino: `/BWMS-Mods/<NNNN>/<nome>` em minúsculas. O jogo monta a raiz como `archive/mac/mod`, com a
/// caixa dele, e a pasta do jogo que ele vê pode vir por outro caminho que o da dylib.
pub(crate) fn sufixo_no_depot(destino: &Path) -> String {
    let partes: Vec<String> = destino.components().rev().take(3).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    partes.iter().rev().fold(String::new(), |mut s, p| {
        s.push('/');
        s.push_str(p);
        s
    })
    .to_lowercase()
}

/// Os archives de `s` que ficaram fora do jogo, na ordem de carga de `ordem`: os que a sincronização não pôs e os nativos que o jogo não
/// montou (`montado` olha o depot).
pub(crate) fn fora_do_jogo(ordem: &[PathBuf], s: &Sincronia, mut montado: impl FnMut(&Path) -> bool) -> Vec<(PathBuf, MotivoDeFora)> {
    ordem
        .iter()
        .filter_map(|a| {
            if let Some((_, m)) = s.fora.iter().find(|(r, _)| r == a) {
                return Some((a.clone(), m.clone()));
            }
            let (_, destino, _) = s.nativos.iter().find(|(o, _, _)| o == a)?;
            (!montado(destino)).then(|| (a.clone(), MotivoDeFora::JogoNaoMontou))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jogo_de_teste(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-pasta-nativa-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join("archive/Mac/content")).unwrap();
        p
    }

    /// `RDAR` v12 com o índice em 4096 e `marca` no corpo, para dois archives válidos terem conteúdo diferente.
    fn archive(marca: u8) -> Vec<u8> {
        let mut indice = vec![0u8; 16];
        indice.extend_from_slice(&0u32.to_le_bytes());
        indice.extend_from_slice(&[0u8; 8]);
        let mut b = vec![marca; 4096];
        b[..4].copy_from_slice(b"RDAR");
        b[4..8].copy_from_slice(&12u32.to_le_bytes());
        b[8..16].copy_from_slice(&4096u64.to_le_bytes());
        b[16..20].copy_from_slice(&(indice.len() as u32).to_le_bytes());
        b.extend_from_slice(&indice);
        b
    }

    fn escrever(p: &Path, conteudo: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, conteudo).unwrap();
    }

    /// Os archives da pasta nativa como o jogo os ordena: caminho inteiro, `strcmp`.
    fn ordem_do_jogo(jogo: &Path) -> Vec<String> {
        fn andar(d: &Path, v: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    andar(&p, v);
                } else if p.extension().is_some_and(|x| x == "archive") {
                    v.push(p);
                }
            }
        }
        let mut v = Vec::new();
        andar(&jogo.join(PASTA_NATIVA), &mut v);
        v.sort_by(|a, b| a.as_os_str().as_bytes().cmp(b.as_os_str().as_bytes()));
        v.iter().map(|p| p.strip_prefix(jogo.join(PASTA_NATIVA)).unwrap().to_string_lossy().into_owned()).collect()
    }

    /// Tudo o que há em `BWMS-Mods/`, pasta e link inclusive, relativo a ela, sem seguir link.
    fn conteudo_do_bwms(jogo: &Path) -> Vec<String> {
        fn andar(raiz: &Path, d: &Path, v: &mut Vec<String>) {
            for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
                let p = e.path();
                let rel = p.strip_prefix(raiz).unwrap().to_string_lossy().into_owned();
                if e.file_type().unwrap().is_dir() {
                    v.push(format!("{rel}/"));
                    andar(raiz, &p, v);
                } else {
                    v.push(rel);
                }
            }
        }
        let raiz = jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS);
        let mut v = Vec::new();
        andar(&raiz, &raiz, &mut v);
        v.sort();
        v
    }

    /// O que está fora de `BWMS-Mods/` e o teste confere que fica: archive solto do usuário, `modlist.txt` e um archive do jogo.
    fn por_arquivos_do_usuario(jogo: &Path) {
        escrever(&jogo.join(PASTA_NATIVA).join("do-usuario.archive"), b"dele");
        escrever(&jogo.join(PASTA_NATIVA).join("modlist.txt"), b"do-usuario.archive\n");
        escrever(&jogo.join("archive/Mac/content/basegame_1.archive"), b"jogo");
    }

    fn arquivos_do_usuario_intactos(jogo: &Path) -> bool {
        std::fs::read(jogo.join(PASTA_NATIVA).join("do-usuario.archive")).is_ok_and(|c| c == b"dele")
            && std::fs::read(jogo.join(PASTA_NATIVA).join("modlist.txt")).is_ok_and(|c| c == b"do-usuario.archive\n")
            && std::fs::read(jogo.join("archive/Mac/content/basegame_1.archive")).is_ok_and(|c| c == b"jogo")
    }

    /// Ordem de carga preservada pelo nome, nome do arquivo intacto, conteúdo igual ao da `Mods/`; inválido e nome repetido ficam fora.
    #[test]
    fn poe_na_pasta_nativa_na_ordem_de_carga() {
        let jogo = jogo_de_teste("ordem");
        let mods = jogo.join("Mods");
        let (z, a, ruim, repetido) = (mods.join("Z mod/z.archive"), mods.join("A mod/archive/pc/mod/a.archive"), mods.join("B/ruim.archive"), mods.join("C/a.archive"));
        escrever(&z, &archive(1));
        escrever(&a, &archive(2));
        escrever(&ruim, b"nao e archive");
        escrever(&repetido, &archive(3));
        // A ordem de carga da `Mods/` (`pasta_mods::archives`) é a que chega aqui, não a alfabética do nome.
        let s = sincronizar(&jogo, &[z.clone(), a.clone(), ruim.clone(), repetido.clone()], &[]);
        assert_eq!(s.recusados, [(ruim, Recusa::Invalido), (repetido, Recusa::NomeRepetido)]);
        assert!(s.fora.is_empty(), "{:?}", s.fora);
        assert_eq!(ordem_do_jogo(&jogo), ["BWMS-Mods/0001/z.archive", "BWMS-Mods/0002/a.archive"]);
        // O `temp_dir` do macOS é APFS: o caminho do `clonefile` é o exercitado.
        assert!(s.nativos.iter().all(|(_, _, v)| *v == Via::Clone), "{:?}", s.nativos);
        assert_eq!(std::fs::read(jogo.join(PASTA_NATIVA).join("BWMS-Mods/0002/a.archive")).unwrap(), archive(2));
        // A 2ª abertura sem mudança só confere.
        let s = sincronizar(&jogo, &[z.clone(), a.clone()], &[]);
        assert!(s.nativos.iter().all(|(_, _, v)| *v == Via::Mantido), "{:?}", s.nativos);
        assert_eq!(s.removidos, 0);
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Sem lista nenhuma do que foi posto, os clones de uma abertura anterior que não batem com a `Mods/` atual saem (o defeito do boot de
    /// 2026-10-23: a lista sumiu e os clones ficaram montados pelo jogo); o que está fora de `BWMS-Mods/` fica.
    #[test]
    fn sem_lista_os_clones_orfaos_saem() {
        let jogo = jogo_de_teste("orfaos");
        por_arquivos_do_usuario(&jogo);
        let bwms = jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS);
        escrever(&bwms.join("0001/velho.archive"), &archive(7));
        escrever(&bwms.join("0002/a.archive"), &archive(8));
        escrever(&bwms.join("0003/c.archive"), &archive(9));
        let a = jogo.join("Mods/A/a.archive");
        escrever(&a, &archive(1));
        let s = sincronizar(&jogo, &[a.clone()], &[]);
        assert_eq!(s.removidos, 3);
        assert_eq!(conteudo_do_bwms(&jogo), ["0001/", "0001/a.archive"]);
        assert_eq!(std::fs::read(bwms.join("0001/a.archive")).unwrap(), archive(1));
        assert!(arquivos_do_usuario_intactos(&jogo));
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// `Mods/` esvaziada (ou ausente: `pasta_mods::preparar_no_boot` chama com a lista vazia): `BWMS-Mods/` fica vazia e some.
    #[test]
    fn mods_vazia_tira_a_pasta_do_bwms() {
        let jogo = jogo_de_teste("vazia");
        por_arquivos_do_usuario(&jogo);
        let (a, b) = (jogo.join("Mods/A/a.archive"), jogo.join("Mods/B/b.archive"));
        escrever(&a, &archive(1));
        escrever(&b, &archive(2));
        assert_eq!(sincronizar(&jogo, &[a, b], &[]).nativos.len(), 2);
        let s = sincronizar(&jogo, &[], &[]);
        assert_eq!((s.removidos, s.nativos.len(), s.fora.len()), (2, 0, 0));
        assert!(std::fs::symlink_metadata(jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS)).is_err());
        assert!(arquivos_do_usuario_intactos(&jogo));
        assert_eq!(ordem_do_jogo(&jogo), ["do-usuario.archive"]);
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Arquivo que não é destino de archive atual sai, qualquer que seja o nome: texto, `.DS_Store`, temporário largado, archive numa pasta
    /// que o BWMS não usa, pasta no lugar de um destino e arquivo diferente no destino (trocado pelo clone).
    #[test]
    fn arquivo_estranho_dentro_da_pasta_do_bwms_sai() {
        let jogo = jogo_de_teste("estranho");
        por_arquivos_do_usuario(&jogo);
        let bwms = jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS);
        escrever(&bwms.join("leia.txt"), b"x");
        escrever(&bwms.join("0001/.DS_Store"), b"x");
        escrever(&bwms.join("0001/.a.archive.bwms-tmp"), b"x");
        escrever(&bwms.join("outra/sub/a.archive"), &archive(5));
        escrever(&bwms.join("0001/a.archive/dentro.archive"), &archive(6));
        escrever(&bwms.join("0002/b.archive"), b"alheio");
        let (a, b) = (jogo.join("Mods/A/a.archive"), jogo.join("Mods/B/b.archive"));
        escrever(&a, &archive(1));
        escrever(&b, &archive(2));
        let s = sincronizar(&jogo, &[a, b], &[]);
        assert_eq!(s.removidos, 5);
        assert!(s.fora.is_empty(), "{:?}", s.fora);
        assert_eq!(conteudo_do_bwms(&jogo), ["0001/", "0001/a.archive", "0002/", "0002/b.archive"]);
        assert_eq!(std::fs::read(bwms.join("0001/a.archive")).unwrap(), archive(1));
        assert_eq!(std::fs::read(bwms.join("0002/b.archive")).unwrap(), archive(2));
        assert!(arquivos_do_usuario_intactos(&jogo));
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Link dentro de `BWMS-Mods/` sai sem ser seguido: o alvo (arquivo ou pasta, fora dela) fica inteiro, link no lugar de um destino vira o
    /// clone, link quebrado sai. `BWMS-Mods` que é ela mesma um link sai pelo link e vira pasta.
    #[test]
    fn link_simbolico_nao_e_seguido() {
        use std::os::unix::fs::symlink;
        let jogo = jogo_de_teste("link");
        let fora = jogo.join("fora");
        escrever(&fora.join("vitima.archive"), &archive(7));
        escrever(&fora.join("sub/outra.archive"), &archive(8));
        let bwms = jogo.join(PASTA_NATIVA).join(PASTA_DO_BWMS);
        std::fs::create_dir_all(bwms.join("0001")).unwrap();
        symlink(fora.join("vitima.archive"), bwms.join("0001/a.archive")).unwrap();
        symlink(&fora, bwms.join("pasta-link")).unwrap();
        symlink(jogo.join("nao-existe.archive"), bwms.join("quebrado.archive")).unwrap();
        let a = jogo.join("Mods/A/a.archive");
        escrever(&a, &archive(1));
        let s = sincronizar(&jogo, &[a.clone()], &[]);
        assert_eq!(s.removidos, 3);
        assert_eq!(conteudo_do_bwms(&jogo), ["0001/", "0001/a.archive"]);
        assert!(std::fs::symlink_metadata(bwms.join("0001/a.archive")).unwrap().file_type().is_file());
        assert_eq!(std::fs::read(bwms.join("0001/a.archive")).unwrap(), archive(1));
        assert_eq!(std::fs::read(fora.join("vitima.archive")).unwrap(), archive(7));
        assert_eq!(std::fs::read(fora.join("sub/outra.archive")).unwrap(), archive(8));

        // A própria `BWMS-Mods` como link para outra pasta.
        let outra = jogo.join("outra-pasta");
        escrever(&outra.join("0001/a.archive"), &archive(9));
        std::fs::remove_dir_all(&bwms).unwrap();
        symlink(&outra, &bwms).unwrap();
        let s = sincronizar(&jogo, &[a], &[]);
        assert_eq!(s.removidos, 1);
        assert!(std::fs::symlink_metadata(&bwms).unwrap().is_dir());
        assert_eq!(std::fs::read(bwms.join("0001/a.archive")).unwrap(), archive(1));
        assert_eq!(std::fs::read(outra.join("0001/a.archive")).unwrap(), archive(9));
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Archive trocado na `Mods/` (tamanho ou data) é posto de novo; a posição muda quando entra um mod antes dele.
    #[test]
    fn troca_o_que_mudou_e_renumera() {
        let jogo = jogo_de_teste("troca");
        let mods = jogo.join("Mods");
        let b = mods.join("B/b.archive");
        escrever(&b, &archive(1));
        sincronizar(&jogo, &[b.clone()], &[]);
        let mut novo = archive(9);
        novo.extend_from_slice(b"mais");
        escrever(&b, &novo);
        let a = mods.join("A/a.archive");
        escrever(&a, &archive(2));
        let s = sincronizar(&jogo, &[a.clone(), b.clone()], &[]);
        assert_eq!(s.removidos, 1);
        assert_eq!(ordem_do_jogo(&jogo), ["BWMS-Mods/0001/a.archive", "BWMS-Mods/0002/b.archive"]);
        assert_eq!(std::fs::read(jogo.join(PASTA_NATIVA).join("BWMS-Mods/0002/b.archive")).unwrap(), novo);
        let s = sincronizar(&jogo, &[a, b], &[]);
        assert!(s.nativos.iter().all(|(_, _, v)| *v == Via::Mantido));
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Sem `archive/Mac/` tudo fica fora, com o motivo, e nada é criado.
    #[test]
    fn sem_pasta_do_jogo_fica_tudo_fora() {
        let jogo = std::env::temp_dir().join(format!("bwms-pasta-nativa-sem-mac-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&jogo);
        let a = jogo.join("Mods/A/a.archive");
        escrever(&a, &archive(1));
        let s = sincronizar(&jogo, &[a.clone()], &[]);
        assert_eq!(s.fora, [(a, MotivoDeFora::SemPastaDoJogo)]);
        assert!(!jogo.join("archive").exists());
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Mais de 9999 archives: todos com 5 dígitos, e a ordem do `strcmp` segue a de carga.
    #[test]
    fn digitos_iguais_para_todos() {
        let nomes: Vec<std::ffi::OsString> = (0..10_000).map(|i| format!("{i}.archive").into()).collect();
        let refs: Vec<&std::ffi::OsStr> = nomes.iter().map(|n| n.as_os_str()).collect();
        let d = destinos(&refs);
        assert_eq!(d[0], Path::new("archive/Mac/mod/BWMS-Mods/00001/0.archive"));
        assert!(d.windows(2).all(|w| w[0].as_os_str().as_bytes() < w[1].as_os_str().as_bytes()));
        assert_eq!(destinos(&refs[..3])[2], Path::new("archive/Mac/mod/BWMS-Mods/0003/2.archive"));
    }

    /// Fora do jogo: os que a sincronização não pôs e os nativos que o jogo não montou, na ordem de carga; o sufixo casa o caminho do depot com
    /// a caixa do jogo.
    #[test]
    fn fora_do_jogo_na_ordem_de_carga() {
        let (a, b, c) = (PathBuf::from("/j/Mods/a.archive"), PathBuf::from("/j/Mods/b.archive"), PathBuf::from("/j/Mods/c.archive"));
        let destino = |n: &str| PathBuf::from(format!("/j/archive/Mac/mod/BWMS-Mods/{n}"));
        let nao_copiou = MotivoDeFora::NaoCopiou("sem espaço".into());
        let s = Sincronia {
            nativos: vec![(a.clone(), destino("0001/a.archive"), Via::Clone), (c.clone(), destino("0002/c.archive"), Via::Mantido)],
            fora: vec![(b.clone(), nao_copiou.clone())],
            ..Sincronia::default()
        };
        let no_depot = ["/Volumes/X/Cyberpunk 2077/archive/mac/mod/bwms-mods/0002/c.archive".to_string()];
        let montado = |d: &Path| no_depot.iter().any(|n| n.to_lowercase().ends_with(&sufixo_no_depot(d)));
        assert_eq!(sufixo_no_depot(&destino("0001/a.archive")), "/bwms-mods/0001/a.archive");
        assert_eq!(fora_do_jogo(&[a.clone(), b.clone(), c.clone()], &s, montado), [(a, MotivoDeFora::JogoNaoMontou), (b, nao_copiou)]);
    }
    fn config(destino: &str, conteudo: &str) -> ConfigDoJogo {
        ConfigDoJogo { xl: PathBuf::from("/j/Mods/x.xl"), destino: PathBuf::from(destino), conteudo: conteudo.into(), onscreens: Vec::new(), factories: Vec::new() }
    }

    /// Os `.json` do plano entram na `BWMS-Mods` junto dos clones, no mesmo espelho: o que o plano não pede mais sai (a pasta da corrente que
    /// esvaziou também), o temporário largado sai, e o mesmo conteúdo não é regravado (mtime igual).
    #[test]
    fn jsons_do_plano_no_mesmo_espelho() {
        let jogo = jogo_de_teste("jsons");
        por_arquivos_do_usuario(&jogo);
        let a = jogo.join("Mods/A/a.archive");
        escrever(&a, &archive(1));
        let raso = config("archive/Mac/mod/BWMS-Mods/xl/0002-b.archive.json", "{\"localization\": 2}\n");
        let fundo = config("archive/Mac/mod/BWMS-Mods/xl/xl/0001-a.archive.json", "{\"localization\": 1}\n");
        escrever(&jogo.join("archive/Mac/mod/BWMS-Mods/xl/.0002-b.archive.json.bwms-tmp"), b"largado");
        let s = sincronizar(&jogo, &[a.clone()], &[raso.clone(), fundo.clone()]);
        assert_eq!((s.jsons.len(), s.json_falhou.clone()), (2, None));
        assert_eq!(s.removidos, 1);
        assert_eq!(conteudo_do_bwms(&jogo), ["0001/", "0001/a.archive", "xl/", "xl/0002-b.archive.json", "xl/xl/", "xl/xl/0001-a.archive.json"]);
        assert_eq!(std::fs::read_to_string(jogo.join(&fundo.destino)).unwrap(), fundo.conteudo);
        let antes = std::fs::metadata(jogo.join(&raso.destino)).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        sincronizar(&jogo, &[a.clone()], &[raso.clone(), fundo.clone()]);
        assert_eq!(std::fs::metadata(jogo.join(&raso.destino)).unwrap().modified().unwrap(), antes);
        // O `.xl` saiu da `Mods/`: o `.json` dele sai, e a corrente encolhe.
        let s = sincronizar(&jogo, &[a], &[raso]);
        assert_eq!(s.removidos, 1);
        assert_eq!(conteudo_do_bwms(&jogo), ["0001/", "0001/a.archive", "xl/", "xl/0002-b.archive.json"]);
        assert!(arquivos_do_usuario_intactos(&jogo));
        let _ = std::fs::remove_dir_all(&jogo);
    }

    /// Um `.json` que não grava tira os outros: o BWMS aplica a seção de todos (`xl_no_jogo::no_disco`), e um `.json` sobrando seria a mesma
    /// seção aplicada duas vezes. Sem `archive/Mac/`, nenhum `.json` é posto.
    #[test]
    fn json_que_falha_tira_todos() {
        let jogo = jogo_de_teste("json-falha");
        let ok = config("archive/Mac/mod/BWMS-Mods/xl/0002-b.archive.json", "{}\n");
        // Byte nulo no nome: a gravação do 2º falha depois de o 1º estar no disco.
        let ruim = config("archive/Mac/mod/BWMS-Mods/xl/xl/0001-a\0.archive.json", "{}\n");
        let s = sincronizar(&jogo, &[], &[ok.clone(), ruim.clone()]);
        assert!(s.json_falhou.is_some(), "{s:?}");
        assert!(s.jsons.is_empty());
        assert!(!jogo.join(&ok.destino).exists() && !jogo.join(&ruim.destino).exists());
        let _ = std::fs::remove_dir_all(&jogo);
        let sem_mac = std::env::temp_dir().join(format!("bwms-pasta-nativa-json-sem-mac-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&sem_mac);
        let s = sincronizar(&sem_mac, &[], &[ok]);
        assert!(s.jsons.is_empty() && s.json_falhou.is_none());
        assert!(!sem_mac.join("archive").exists());
        let _ = std::fs::remove_dir_all(&sem_mac);
    }
}
