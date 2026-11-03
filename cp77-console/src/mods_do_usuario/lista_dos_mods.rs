//! A lista dos mods dos 2 caminhos (a `Mods/`, que o BWMS faz funcionar, e o mod.io, que o jogo faz funcionar), pronta para quem a desenha a
//! cada quadro: o selo do canto lê só a contagem (`contagem`, contadores atômicos) e a aba Mods do terminal lê a lista com os textos já
//! montados (`com_a_lista`). Quem muda o que se sabe dos mods (`pasta_mods::publicar`: abertura, carga dos archives, conflitos, seção de
//! scripts ou de recursos) monta a lista nova fora da trava (`montar`) e só a troca dentro dela (`publicar`): o quadro não varre pasta, não
//! lê arquivo e não formata texto de mod.

use super::mod_io::ModsDoModIo;
use super::pasta_mods::{Caminho, Conflito, EstadosDosMods};
use super::ModStatus;
use std::path::Path;

/// Teto de mods desenhados por caminho na aba (o resto vira uma linha que aponta o relatório): com 10 mil mods a aba montaria 10 mil itens
/// por quadro. Os de luz pior vêm primeiro, então o teto só esconde verdes quando há mais de 200 problemas.
pub(crate) const TETO_POR_CAMINHO: usize = 200;

/// Um caminho na aba: o título com a contagem, o texto de vazio, os mods na ordem de exibição (cortados no teto) e a linha do que passou dele.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grupo {
    pub(crate) titulo: String,
    pub(crate) vazio: &'static str,
    pub(crate) indices: Vec<usize>,
    pub(crate) resto: String,
}

impl Grupo {
    const fn vazio() -> Self {
        Grupo { titulo: String::new(), vazio: "", indices: Vec::new(), resto: String::new() }
    }
}

/// A lista, coluna por coluna (índice `i` = um mod; primeiro os da `Mods/` na ordem de carga, depois os do mod.io na ordem do jogo).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListaDosMods {
    pub(crate) caminhos: Vec<Caminho>,
    pub(crate) luzes: Vec<ModStatus>,
    pub(crate) nomes: Vec<String>,
    /// Uma linha por motivo, separadas por `\n`; vazio no verde.
    pub(crate) motivos: Vec<String>,
    /// `[Mods/, mod.io]`.
    pub(crate) grupos: [Grupo; 2],
    /// Uma linha por conflito: quem vence quem, com o caminho de cada um.
    pub(crate) conflitos: Vec<String>,
    /// Por luz, `[ok, aviso, erro, inativo]`, só os 2 caminhos.
    pub(crate) contagem: [usize; 4],
    /// A contagem com os plugins `.dylib` para a qual `cabecalho` e `partes` foram montados: os plugins carregam depois da lista (no bind
    /// dos scripts e no 1º tick), e o topo da aba só se refaz quando ela muda.
    pub(crate) cabecalho_de: Option<[usize; 4]>,
    pub(crate) cabecalho: String,
    /// As partes coloridas da contagem, as mesmas do selo: `*n` por luz com algum mod, ou `0` cinza sem nenhum.
    pub(crate) partes: Vec<(ModStatus, String)>,
}

impl ListaDosMods {
    pub(crate) const fn vazia() -> Self {
        ListaDosMods {
            caminhos: Vec::new(),
            luzes: Vec::new(),
            nomes: Vec::new(),
            motivos: Vec::new(),
            grupos: [Grupo::vazio(), Grupo::vazio()],
            conflitos: Vec::new(),
            contagem: [0; 4],
            cabecalho_de: None,
            cabecalho: String::new(),
            partes: Vec::new(),
        }
    }

    /// Refaz o topo da aba se a contagem com os `plugins` mudou desde a última vez: quadro sem mudança não aloca.
    pub(crate) fn preparar_cabecalho(&mut self, plugins: [usize; 4]) {
        let total = [0, 1, 2, 3].map(|i| self.contagem[i] + plugins[i]);
        if self.cabecalho_de == Some(total) {
            return;
        }
        let (n_mods, n_modio) = (self.caminhos.iter().filter(|c| **c == Caminho::Mods).count(), self.caminhos.iter().filter(|c| **c == Caminho::ModIo).count());
        let n_plugins: usize = plugins.iter().sum();
        self.cabecalho = format!("Mods no jogo: {} (Mods/ {n_mods}, mod.io {n_modio}, plugins {n_plugins})", total.iter().sum::<usize>());
        self.partes.clear();
        if total.iter().sum::<usize>() == 0 {
            self.partes.push((ModStatus::Inactive, "0".into()));
        } else {
            for (luz, n) in [ModStatus::Ok, ModStatus::Warning, ModStatus::Error, ModStatus::Inactive].into_iter().zip(total) {
                if n > 0 {
                    self.partes.push((luz, format!("*{n}")));
                }
            }
        }
        self.cabecalho_de = Some(total);
    }
}

/// `s` com o que a fonte do terminal (Latin-1) não desenha trocado pelo equivalente: travessão e seta dos textos do relatório. O resto fora do
/// Latin-1 sai `?`, como em todo texto do terminal.
fn para_o_terminal(s: &str) -> String {
    s.replace('—', "-").replace('→', "->").replace('×', "x")
}

/// Ordem de exibição: o que pede ação primeiro (vermelho, amarelo), depois o que está fora do jogo (cinza), depois o verde; empate na ordem
/// de carga.
fn ordem_de_exibicao(l: ModStatus) -> u8 {
    match l {
        ModStatus::Error => 0,
        ModStatus::Warning => 1,
        ModStatus::Inactive => 2,
        ModStatus::Ok => 3,
    }
}

/// A lista com a luz de cada mod da `Mods/` (`luzes`, só as raízes debaixo de `raiz_mods`: o item do manifesto dos plugins conta no plugin),
/// os do mod.io (`modio`, com as perdas de conflito já marcadas) e os conflitos.
pub(crate) fn montar(raiz_mods: Option<&Path>, luzes: &EstadosDosMods, modio: &ModsDoModIo, conflitos: &[Conflito]) -> ListaDosMods {
    let mut l = ListaDosMods::vazia();
    let jogo = raiz_mods.and_then(Path::parent);
    let home = super::mod_io::home();
    let limpo = |s: &str| para_o_terminal(&super::pasta_mods::sem_caminho_da_maquina(s, jogo, home.as_deref()));
    if let Some(raiz) = raiz_mods {
        for i in 0..luzes.raizes.len() {
            let Ok(nome) = luzes.raizes[i].strip_prefix(raiz) else { continue };
            l.caminhos.push(Caminho::Mods);
            l.luzes.push(luzes.cores[i].into());
            l.nomes.push(para_o_terminal(&nome.display().to_string()));
            l.motivos.push(limpo(&luzes.motivos[i]));
        }
    }
    for i in 0..modio.len() {
        l.caminhos.push(Caminho::ModIo);
        l.luzes.push(modio.luzes[i]);
        l.nomes.push(para_o_terminal(&modio.rotulos[i]));
        l.motivos.push(limpo(&modio.motivos[i]));
    }
    for luz in &l.luzes {
        l.contagem[*luz as usize] += 1;
    }
    for (g, caminho, titulo, vazio) in [
        (0, Caminho::Mods, "Mods/ (pasta ao lado do jogo)", "Nenhum mod na pasta Mods/."),
        (1, Caminho::ModIo, "mod.io (menu Mods do jogo)", "Nenhum mod do mod.io instalado."),
    ] {
        let mut indices: Vec<usize> = (0..l.caminhos.len()).filter(|&i| l.caminhos[i] == caminho).collect();
        let n = indices.len();
        indices.sort_by_key(|&i| ordem_de_exibicao(l.luzes[i]));
        indices.truncate(TETO_POR_CAMINHO);
        let resto = if n > TETO_POR_CAMINHO {
            format!("... e mais {} mod(s): a lista inteira está em Mods/BWMS-RELATORIO.txt", n - TETO_POR_CAMINHO)
        } else {
            String::new()
        };
        l.grupos[g] = Grupo { titulo: format!("{titulo}: {n} mod(s)"), vazio, indices, resto };
    }
    l.conflitos = conflitos
        .iter()
        .map(|c| {
            para_o_terminal(&format!(
                "{} ({}) vence {} ({}): {} recurso(s) em comum",
                c.vence,
                c.vence_no.nome(),
                c.perde,
                c.perde_no.nome(),
                c.recursos
            ))
        })
        .collect();
    l
}

#[cfg(not(test))]
mod guardada {
    use super::ListaDosMods;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, PoisonError};

    static LISTA: Mutex<ListaDosMods> = Mutex::new(ListaDosMods::vazia());
    static CONTAGEM: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];

    pub(super) fn publicar(l: ListaDosMods) {
        for (n, c) in l.contagem.iter().zip(&CONTAGEM) {
            c.store(*n, Ordering::Relaxed);
        }
        *LISTA.lock().unwrap_or_else(PoisonError::into_inner) = l;
    }

    pub(super) fn contagem() -> [usize; 4] {
        CONTAGEM.each_ref().map(|c| c.load(Ordering::Relaxed))
    }

    pub(super) fn com_a_lista<R>(f: impl FnOnce(&mut ListaDosMods) -> R) -> R {
        f(&mut LISTA.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Nos testes a lista é da thread: a prova visual põe a lista de cada caso e desenha na mesma thread, e o teste do relatório que publica
/// não a troca no meio.
#[cfg(test)]
mod guardada {
    use super::ListaDosMods;
    use std::cell::RefCell;

    thread_local! {
        static LISTA: RefCell<ListaDosMods> = const { RefCell::new(ListaDosMods::vazia()) };
    }

    pub(super) fn publicar(l: ListaDosMods) {
        LISTA.with(|x| *x.borrow_mut() = l);
    }

    pub(super) fn contagem() -> [usize; 4] {
        LISTA.with(|x| x.borrow().contagem)
    }

    pub(super) fn com_a_lista<R>(f: impl FnOnce(&mut ListaDosMods) -> R) -> R {
        LISTA.with(|x| f(&mut x.borrow_mut()))
    }
}

/// Troca a lista em uso e a contagem do selo.
pub(crate) fn publicar(l: ListaDosMods) {
    guardada::publicar(l)
}

/// A contagem dos 2 caminhos por luz, `[ok, aviso, erro, inativo]`: o selo a lê a cada quadro.
pub(crate) fn contagem() -> [usize; 4] {
    guardada::contagem()
}

/// A aba Mods desenha com a lista em uso (a trava fica com ela durante o desenho dela, que não lê disco).
pub(crate) fn com_a_lista<R>(f: impl FnOnce(&mut ListaDosMods) -> R) -> R {
    guardada::com_a_lista(f)
}

/// O veredito de dev do comando `modslista` (canal do heartbeat): a contagem que o selo desenha, os mods de cada caminho com a luz e os
/// conflitos, numa linha. Só lê a lista pronta e os contadores.
#[cfg(any(test, feature = "devtools"))]
pub(crate) fn veredito() -> String {
    let selo = super::mod_counts();
    let plugins = super::contagem_dos_plugins();
    com_a_lista(|l| {
        let luz = |s: ModStatus| match s {
            ModStatus::Ok => "verde",
            ModStatus::Warning => "amarelo",
            ModStatus::Error => "vermelho",
            ModStatus::Inactive => "cinza",
        };
        let grupo = |c: Caminho| {
            (0..l.caminhos.len()).filter(|&i| l.caminhos[i] == c).map(|i| format!("{}={}", l.nomes[i], luz(l.luzes[i]))).collect::<Vec<_>>()
        };
        let (mods, modio) = (grupo(Caminho::Mods), grupo(Caminho::ModIo));
        format!(
            "[modslista] selo ok={} aviso={} erro={} inativo={} (plugins {:?}) | Mods/ {}: {} | mod.io {}: {} | conflitos {}: {} | MODSLISTA OK",
            selo.0,
            selo.1,
            selo.2,
            selo.3,
            plugins,
            mods.len(),
            mods.join("; "),
            modio.len(),
            modio.join("; "),
            l.conflitos.len(),
            l.conflitos.join("; ")
        )
    })
}

#[cfg(test)]
pub(crate) mod exemplos {
    //! Listas para a prova visual da aba e para os testes de layout, montadas pelo `montar` do produto a partir de estados sintéticos.
    use super::*;
    use crate::mods_do_usuario::pasta_mods::CorDoMod;
    use std::path::PathBuf;

    const RAIZ: &str = "/jogo/Mods";

    fn mods_da_pasta(itens: &[(&str, CorDoMod, &str)]) -> EstadosDosMods {
        let mut e = EstadosDosMods::default();
        for (nome, cor, motivo) in itens {
            e.marcar(&Path::new(RAIZ).join(nome), *cor, motivo);
        }
        e
    }

    fn modio(itens: &[(&str, ModStatus, &str)]) -> ModsDoModIo {
        let mut m = ModsDoModIo::vazio();
        for (rotulo, luz, motivo) in itens {
            m.pastas.push(PathBuf::from("/home/Library/Application Support/mod.io/common/5784/mods").join(rotulo));
            m.ids.push(None);
            m.rotulos.push(rotulo.to_string());
            m.ligados.push(Some(*luz != ModStatus::Inactive));
            m.archives.push(Vec::new());
            m.luzes.push(*luz);
            m.motivos.push(motivo.to_string());
        }
        m
    }

    pub(crate) fn vazia() -> ListaDosMods {
        montar(Some(Path::new(RAIZ)), &EstadosDosMods::default(), &ModsDoModIo::vazio(), &[])
    }

    pub(crate) fn um() -> ListaDosMods {
        montar(Some(Path::new(RAIZ)), &mods_da_pasta(&[("Virtual Atelier", CorDoMod::Verde, "")]), &ModsDoModIo::vazio(), &[])
    }

    /// 1 de cada cor na `Mods/`: verde, amarelo (archive recusado) e vermelho (só `.dll`).
    pub(crate) fn tres_cores() -> ListaDosMods {
        let e = mods_da_pasta(&[
            ("Equipment-EX", CorDoMod::Verde, ""),
            ("JB", CorDoMod::Amarelo, "recusado / refused: archive/pc/mod/jb.archive — não é um .archive válido; baixe o mod de novo / not a valid .archive; download the mod again"),
            ("input_loader", CorDoMod::Vermelho, "fora / left out: .lua (mod de CET) ou .dll (plugin de Windows) não carregam no Mac / .lua (CET mod) or .dll (Windows plugin) do not load on a Mac"),
        ]);
        montar(Some(Path::new(RAIZ)), &e, &ModsDoModIo::vazio(), &[])
    }

    /// A `Mods/` de teste do roteiro e os 2 do mod.io do dono, com a cópia do FairerCourierTimers na `Mods/` vencendo o do mod.io.
    pub(crate) fn dois_caminhos() -> ListaDosMods {
        let c = Conflito {
            vence: "BWMS Teste ModIO".into(),
            perde: "Longer Car Delivery Timers (mod.io 6428265)".into(),
            recursos: 1,
            vence_no: Caminho::Mods,
            perde_no: Caminho::ModIo,
        };
        let e = mods_da_pasta(&[
            ("BWMS Teste ModIO", CorDoMod::Verde, ""),
            ("Equipment-EX", CorDoMod::Verde, ""),
            ("JB", CorDoMod::Verde, ""),
            ("Quadra Turbo R", CorDoMod::Verde, ""),
            ("Virtual Atelier", CorDoMod::Verde, ""),
            ("input_loader", CorDoMod::Vermelho, "fora / left out: .lua (mod de CET) ou .dll (plugin de Windows) não carregam no Mac / .lua (CET mod) or .dll (Windows plugin) do not load on a Mac"),
        ]);
        let m = modio(&[
            ("Immersive Rippers (mod.io 6383233)", ModStatus::Ok, ""),
            ("Longer Car Delivery Timers (mod.io 6428265)", ModStatus::Ok, ""),
        ])
        .com_perdas(std::slice::from_ref(&c));
        montar(Some(Path::new(RAIZ)), &e, &m, &[c])
    }

    /// `n` mods na `Mods/` (1 a cada 7 amarelo, 1 a cada 11 vermelho) e 3 do mod.io (1 desligado).
    pub(crate) fn muitos(n: usize) -> ListaDosMods {
        let mut e = EstadosDosMods::default();
        for i in 0..n {
            let (cor, motivo) = if i % 11 == 0 {
                (CorDoMod::Vermelho, "fora / left out: .dll (plugin de Windows) não carrega no Mac / Windows plugin, does not load on a Mac")
            } else if i % 7 == 0 {
                (CorDoMod::Amarelo, "aviso / warning: parte do .xl ficou sem efeito / part of the .xl has no effect")
            } else {
                (CorDoMod::Verde, "")
            };
            e.marcar(&Path::new(RAIZ).join(format!("Mod {i:05}")), cor, motivo);
        }
        let m = modio(&[
            ("Immersive Rippers (mod.io 6383233)", ModStatus::Ok, ""),
            ("Longer Car Delivery Timers (mod.io 6428265)", ModStatus::Ok, ""),
            ("Desligado (mod.io 6000001)", ModStatus::Inactive, "desligado no menu Mods do jogo / disabled in the game's Mods menu"),
        ]);
        montar(Some(Path::new(RAIZ)), &e, &m, &[])
    }

    /// Nome e motivo longos, sem espaço e com espaço, e nome fora do Latin-1.
    pub(crate) fn nome_longo() -> ListaDosMods {
        let longo = "Um Mod Com Um Nome Muito Comprido Que Nao Cabe Na Largura Da Janela Do Terminal De Jeito Nenhum - Edicao Definitiva v2.0.1";
        let sem_espaco = "NomeSemEspacoNenhumQueAtravessaAJanelaInteiraSemPontoDeQuebraNenhumParaVerOndeOTextoCorta_0123456789_0123456789";
        let e = mods_da_pasta(&[
            (longo, CorDoMod::Amarelo, "aviso / warning: archive/pc/mod/um_arquivo_com_nome_comprido_tambem_para_a_quebra.archive — parte do mod ficou de fora porque o arquivo não é um .archive válido; baixe o mod de novo / part of the mod was left out"),
            (sem_espaco, CorDoMod::Verde, ""),
            ("日本語の名前", CorDoMod::Verde, ""),
        ]);
        montar(Some(Path::new(RAIZ)), &e, &ModsDoModIo::vazio(), &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods_do_usuario::pasta_mods::CorDoMod;

    /// A lista junta os 2 caminhos, conta por luz, põe o pior primeiro, corta no teto e escreve o conflito com o caminho de cada lado.
    #[test]
    fn monta_os_dois_caminhos_com_contagem_ordem_e_teto() {
        let l = exemplos::dois_caminhos();
        assert_eq!(l.contagem, [6, 1, 1, 0], "5 verdes da Mods + Immersive Rippers; o mod.io que perde fica amarelo; input_loader vermelho");
        assert_eq!(l.nomes[l.grupos[0].indices[0]], "input_loader", "vermelho primeiro");
        assert_eq!(l.grupos[0].titulo, "Mods/ (pasta ao lado do jogo): 6 mod(s)");
        assert_eq!(l.grupos[1].titulo, "mod.io (menu Mods do jogo): 2 mod(s)");
        let perdeu = l.grupos[1].indices[0];
        assert_eq!((l.nomes[perdeu].as_str(), l.luzes[perdeu]), ("Longer Car Delivery Timers (mod.io 6428265)", ModStatus::Warning));
        assert!(l.motivos[perdeu].starts_with("perde 1 recurso(s) para BWMS Teste ModIO (Mods) no .archive"), "{}", l.motivos[perdeu]);
        assert_eq!(l.conflitos, ["BWMS Teste ModIO (Mods) vence Longer Car Delivery Timers (mod.io 6428265) (mod.io): 1 recurso(s) em comum"]);
        assert!(l.motivos.iter().all(|m| !m.contains('—')), "o travessão sai trocado: a fonte do terminal é Latin-1");

        let muitos = exemplos::muitos(10_000);
        assert_eq!(muitos.contagem.iter().sum::<usize>(), 10_003);
        assert_eq!(muitos.grupos[0].indices.len(), TETO_POR_CAMINHO);
        assert!(muitos.grupos[0].indices.iter().all(|&i| muitos.luzes[i] == ModStatus::Error), "910 vermelhos enchem o teto antes dos verdes");
        assert_eq!(muitos.grupos[0].resto, "... e mais 9800 mod(s): a lista inteira está em Mods/BWMS-RELATORIO.txt");
        assert!(muitos.grupos[1].resto.is_empty());
        assert_eq!(exemplos::vazia().contagem, [0; 4]);
        assert_eq!(exemplos::vazia().grupos[0].titulo, "Mods/ (pasta ao lado do jogo): 0 mod(s)");
    }

    /// O topo da aba só se refaz quando a contagem com os plugins muda, e mostra o que o selo mostra.
    #[test]
    fn cabecalho_segue_o_selo_e_so_refaz_quando_muda() {
        let mut l = exemplos::tres_cores();
        l.preparar_cabecalho([0; 4]);
        assert_eq!(l.cabecalho, "Mods no jogo: 3 (Mods/ 3, mod.io 0, plugins 0)");
        assert_eq!(l.partes, [(ModStatus::Ok, "*1".to_string()), (ModStatus::Warning, "*1".into()), (ModStatus::Error, "*1".into())]);
        let antes = l.cabecalho.as_ptr();
        l.preparar_cabecalho([0; 4]);
        assert_eq!(l.cabecalho.as_ptr(), antes, "mesma contagem: nada refeito");
        l.preparar_cabecalho([2, 0, 0, 1]);
        assert_eq!(l.cabecalho, "Mods no jogo: 6 (Mods/ 3, mod.io 0, plugins 3)");
        assert_eq!(l.partes[0], (ModStatus::Ok, "*3".to_string()));
        assert_eq!(l.partes.last(), Some(&(ModStatus::Inactive, "*1".to_string())));
        let mut vazia = exemplos::vazia();
        vazia.preparar_cabecalho([0; 4]);
        assert_eq!(vazia.partes, [(ModStatus::Inactive, "0".to_string())], "sem mod nenhum, o 0 cinza do selo");
    }

    /// O caminho da máquina sai dos motivos: o do jogo fica relativo a ele.
    #[test]
    fn motivo_sem_caminho_da_maquina() {
        let mut e = EstadosDosMods::default();
        e.marcar(Path::new("/Volumes/X/Cyberpunk 2077/Mods/Q"), CorDoMod::Vermelho, "erro / error: /Volumes/X/Cyberpunk 2077/Mods/Q/r6/scripts/q.reds:5: [UNRESOLVED_FN] f");
        let l = montar(Some(Path::new("/Volumes/X/Cyberpunk 2077/Mods")), &e, &ModsDoModIo::vazio(), &[]);
        assert_eq!(l.motivos, ["erro / error: Mods/Q/r6/scripts/q.reds:5: [UNRESOLVED_FN] f"]);
        assert_eq!(contagem(), [0; 4], "montar não publica");
        publicar(l);
        assert_eq!(contagem(), [0, 0, 1, 0]);
        publicar(ListaDosMods::vazia());
    }

    /// O veredito do `modslista` traz a contagem do selo (lista + plugins), cada mod com a luz e o conflito, na linha que o roteiro confere.
    /// Os contadores dos plugins são lidos 2 vezes (no veredito e aqui): a trava impede a prova visual de zerá-los no meio.
    #[test]
    fn veredito_do_modslista() {
        let _trava = crate::vr::painel::UM_POR_VEZ.lock().unwrap_or_else(|e| e.into_inner());
        publicar(exemplos::dois_caminhos());
        let v = veredito();
        let p = crate::mods_do_usuario::contagem_dos_plugins();
        assert!(v.starts_with(&format!("[modslista] selo ok={} aviso={} erro={} inativo={}", 6 + p[0], 1 + p[1], 1 + p[2], p[3])), "{v}");
        assert!(v.contains("| Mods/ 6: BWMS Teste ModIO=verde; Equipment-EX=verde; JB=verde; Quadra Turbo R=verde; Virtual Atelier=verde; input_loader=vermelho |"), "{v}");
        assert!(v.contains("| mod.io 2: Immersive Rippers (mod.io 6383233)=verde; Longer Car Delivery Timers (mod.io 6428265)=amarelo |"), "{v}");
        assert!(v.ends_with("| conflitos 1: BWMS Teste ModIO (Mods) vence Longer Car Delivery Timers (mod.io 6428265) (mod.io): 1 recurso(s) em comum | MODSLISTA OK"), "{v}");
        publicar(ListaDosMods::vazia());
    }
}
