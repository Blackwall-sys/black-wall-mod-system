//! Os mods do mod.io (o menu Mods do próprio jogo): o jogo os baixa em `~/Library/Application Support/mod.io/common/5784/mods/<id>/` e os
//! carrega sozinho; o BWMS só LÊ essa pasta e o `EnabledMods.json` do jogo, para o selo do canto, a aba Mods e o relatório da `Mods/`
//! mostrarem os dois caminhos juntos. Nada aqui escreve na pasta do mod.io nem no `EnabledMods.json`.
//!
//! O que o jogo faz com um mod do mod.io (binário 2.5, `tmp/consolidacao-20261005b/z029/relatorios/MODIO.md`):
//! - Ligado = listado no `enabled_mods` do `EnabledMods.json` (`GameModService` `vt+0x178` = `0x101d191fc`); o `LoadModArchives`
//!   (`0x10151bc90`) monta os `.archive` dessas pastas no mesmo set de `archive/mac/mod` (scope 6), depois de tudo de `archive/Mac/mod`
//!   (logo depois da `BWMS-Mods`), pasta a pasta em ordem de `modID` com comparação assinada (`0x101d195a0`), e dentro da pasta por `strcmp`
//!   do caminho. No hash repetido serve o 1º (`RequestResource` `0x10151a748`): a `Mods/` vence o mod.io, e entre dois do mod.io vence o de
//!   id menor.
//! - Configs: só `*.json` (glob `0x101cbd700`); o binário não tem a string `.xl` nem `yaml` (z020), então o `.archive.xl` do mod.io fica sem
//!   efeito quando não vem com o `.json` correspondente.
//! - Scripts: o jogo liga blobs `*.redscripts` e não compila `.reds` (nenhuma string `.reds` no `__cstring`); o BWMS também não compila os
//!   `.reds` da pasta do mod.io.

use super::ModStatus;
use crate::mods_do_usuario::pasta_mods::{Caminho, Conflito};
use bwms_primitives::instalacao::apply::arquivos_da_pasta_mods;
use bwms_primitives::instalacao::xl_no_jogo::{mods_ligados_do_modio, ENABLED_MODS};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A pasta dos mods do mod.io do Cyberpunk 2077 (jogo 5784 no mod.io), relativa à home.
pub(crate) const PASTA_DOS_MODS: &str = "Library/Application Support/mod.io/common/5784/mods";

/// Os mods do mod.io que estão no disco ou no `EnabledMods.json`, coluna por coluna (índice `i` = um mod): primeiro os ligados na ordem em
/// que o jogo monta (`modID` crescente, assinado), depois os desligados por id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModsDoModIo {
    pub(crate) pastas: Vec<PathBuf>,
    pub(crate) ids: Vec<Option<i64>>,
    /// Rótulo pronto: `<nome> (mod.io <id>)`. O nome é o `modName` do `EnabledMods.json` (o que o menu do jogo mostra), senão o `ModName` do
    /// `about.json`, senão a pasta.
    pub(crate) rotulos: Vec<String>,
    /// `Some(true)` ligado no menu Mods do jogo; `Some(false)` desligado; `None` com o `EnabledMods.json` ilegível.
    pub(crate) ligados: Vec<Option<bool>>,
    /// Os `.archive` de cada mod que o jogo monta (válidos), em ordem de caminho: o que disputa recurso.
    pub(crate) archives: Vec<Vec<PathBuf>>,
    pub(crate) luzes: Vec<ModStatus>,
    /// Uma linha `"<pt> / <en>"` por motivo, separadas por `\n`; vazio no verde.
    pub(crate) motivos: Vec<String>,
}

impl ModsDoModIo {
    pub(crate) const fn vazio() -> Self {
        ModsDoModIo {
            pastas: Vec::new(),
            ids: Vec::new(),
            rotulos: Vec::new(),
            ligados: Vec::new(),
            archives: Vec::new(),
            luzes: Vec::new(),
            motivos: Vec::new(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.pastas.len()
    }

    /// Põe o mod `i` com pelo menos a gravidade de `luz` e acrescenta `motivo`. Cinza (desligado ou sem estado) não sobe: o mod que o jogo
    /// não carrega não tem outro problema a mostrar.
    pub(crate) fn marcar(&mut self, i: usize, luz: ModStatus, motivo: &str) {
        if self.luzes[i] == ModStatus::Inactive {
            return;
        }
        if gravidade(luz) > gravidade(self.luzes[i]) {
            self.luzes[i] = luz;
        }
        if !motivo.is_empty() && !self.motivos[i].split('\n').any(|l| l == motivo) {
            if !self.motivos[i].is_empty() {
                self.motivos[i].push('\n');
            }
            self.motivos[i].push_str(motivo);
        }
    }

    /// Os ligados, na ordem em que o jogo os monta, com os archives: a entrada do mod.io na conta dos conflitos (`pasta_mods::conflitos`).
    pub(crate) fn na_ordem_de_carga(&self) -> Vec<(Caminho, String, Vec<PathBuf>)> {
        (0..self.len()).filter(|&i| self.ligados[i] == Some(true)).map(|i| (Caminho::ModIo, self.rotulos[i].clone(), self.archives[i].clone())).collect()
    }

    /// Uma cópia com o mod do mod.io que perde recurso marcado amarelo, com quem vence.
    pub(crate) fn com_perdas(&self, conflitos: &[Conflito]) -> Self {
        let mut m = self.clone();
        for c in conflitos.iter().filter(|c| c.perde_no == Caminho::ModIo) {
            if let Some(i) = m.rotulos.iter().position(|r| *r == c.perde) {
                m.marcar(i, ModStatus::Warning, &c.motivo_de_quem_perde());
            }
        }
        m
    }
}

/// Ordem de gravidade da luz para `marcar`: verde < amarelo < vermelho.
fn gravidade(l: ModStatus) -> u8 {
    match l {
        ModStatus::Ok => 0,
        ModStatus::Warning => 1,
        ModStatus::Error => 2,
        ModStatus::Inactive => 3,
    }
}

/// O `about.json` (`ModInfo.ModName`), o arquivo que o carregador de scripts do jogo exige numa pasta de mod (`0x1011e7af4`).
fn nome_do_about(pasta: &Path) -> Option<String> {
    let texto = std::fs::read_to_string(pasta.join("about.json")).ok()?;
    let doc = bwms_primitives::json::ler(texto.strip_prefix('\u{feff}').unwrap_or(&texto)).ok()?;
    match doc.campo("ModInfo")?.campo("ModName")? {
        bwms_primitives::json::Valor::Texto(n) if !n.trim().is_empty() => Some(n.clone()),
        _ => None,
    }
}

/// O que um mod do mod.io traz, por tipo, contado pela travessia da `Mods/` (`apply::arquivos_da_pasta_mods`: ordem de caminho, sem nome com
/// ponto, sem seguir link).
#[derive(Default)]
struct Pecas {
    archives: Vec<PathBuf>,
    xls: Vec<PathBuf>,
    /// Os `.json` sem a extensão, para casar com o `.xl` de mesmo nome (`X.archive.xl` ↔ `X.archive.json`).
    jsons: HashSet<PathBuf>,
    reds: usize,
    /// `.yaml`/`.yml`/`.tweak`: o carregador de tweaks do jogo só lê `*.json` (o binário não tem a string `yaml`, z020).
    tweaks_de_texto: usize,
    redscripts: usize,
    luas: usize,
    dlls: usize,
}

fn pecas(pasta: &Path) -> Pecas {
    let mut p = Pecas::default();
    for a in arquivos_da_pasta_mods(pasta) {
        let ext = a.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        match ext.as_str() {
            "archive" => p.archives.push(a),
            "xl" => p.xls.push(a),
            "json" if a.file_name().is_some_and(|n| n != "about.json") => {
                p.jsons.insert(a.with_extension(""));
            }
            "reds" => p.reds += 1,
            "yaml" | "yml" | "tweak" => p.tweaks_de_texto += 1,
            "redscripts" => p.redscripts += 1,
            "lua" => p.luas += 1,
            "dll" => p.dlls += 1,
            _ => {}
        }
    }
    p
}

/// Os mods do mod.io da `home`.
pub(crate) fn ler(home: &Path) -> ModsDoModIo {
    ler_em(&home.join(PASTA_DOS_MODS), &home.join(ENABLED_MODS))
}

/// Os mods em `pasta_dos_mods` e no `EnabledMods.json` em `enabled`, com a luz de cada um. Verde: ligado, e o jogo carrega tudo o que ele
/// traz. Cinza: desligado no menu, ou estado desconhecido (lista ilegível; o motivo diz). Vermelho/amarelo só com motivo medido no disco: a
/// pasta ligada não existe, `.archive` que não passa no `archive_valido` (o motor não o monta), `.xl` sem o `.json`, `.reds`, `.lua`, `.dll`.
pub(crate) fn ler_em(pasta_dos_mods: &Path, enabled: &Path) -> ModsDoModIo {
    let ligados = mods_ligados_do_modio(enabled);
    let mut no_disco: Vec<PathBuf> = std::fs::read_dir(pasta_dos_mods)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .collect();
    no_disco.sort();
    let id_da_pasta = |p: &Path| p.file_name().and_then(|n| n.to_str()).and_then(|n| n.parse::<i64>().ok());
    // (pasta, id, nome do EnabledMods, ligado)
    let mut entradas: Vec<(PathBuf, Option<i64>, Option<String>, Option<bool>)> = Vec::new();
    match &ligados {
        Ok(v) => {
            let mut v = v.clone();
            v.sort_by_key(|m| m.id.or_else(|| id_da_pasta(&m.pasta)).unwrap_or(i64::MAX));
            for m in v {
                let id = m.id.or_else(|| id_da_pasta(&m.pasta));
                entradas.push((m.pasta, id, m.nome, Some(true)));
            }
            for p in no_disco {
                if !entradas.iter().any(|(e, ..)| *e == p) {
                    let id = id_da_pasta(&p);
                    entradas.push((p, id, None, Some(false)));
                }
            }
        }
        Err(_) => entradas.extend(no_disco.into_iter().map(|p| (p.clone(), id_da_pasta(&p), None, None))),
    }
    let mut m = ModsDoModIo::vazio();
    for (pasta, id, nome, ligado) in entradas {
        let nome = nome.or_else(|| nome_do_about(&pasta)).unwrap_or_else(|| pasta.file_name().unwrap_or_default().to_string_lossy().into_owned());
        let rotulo = match id {
            Some(id) => format!("{nome} (mod.io {id})"),
            None => format!("{nome} (mod.io)"),
        };
        let i = m.len();
        m.pastas.push(pasta.clone());
        m.ids.push(id);
        m.rotulos.push(rotulo);
        m.ligados.push(ligado);
        m.archives.push(Vec::new());
        m.luzes.push(ModStatus::Ok);
        m.motivos.push(String::new());
        match ligado {
            Some(false) => {
                m.luzes[i] = ModStatus::Inactive;
                m.motivos[i] = "desligado no menu Mods do jogo / disabled in the game's Mods menu".into();
                continue;
            }
            None => {
                let erro = ligados.as_ref().err().cloned().unwrap_or_default();
                m.luzes[i] = ModStatus::Inactive;
                m.motivos[i] = format!(
                    "o BWMS não leu a lista de mods ligados do jogo (EnabledMods.json: {erro}); abra o menu Mods do jogo uma vez para ele a \
                     regravar / BWMS could not read the game's enabled mod list (EnabledMods.json: {erro}); open the game's Mods menu once so it \
                     rewrites it"
                );
                continue;
            }
            Some(true) => {}
        }
        if !pasta.is_dir() {
            m.marcar(
                i,
                ModStatus::Error,
                "a pasta do mod não existe; reinstale o mod pelo menu Mods do jogo / the mod folder does not exist; reinstall the mod from the \
                 game's Mods menu",
            );
            continue;
        }
        let p = pecas(&pasta);
        let rel = |a: &Path| a.strip_prefix(&pasta).unwrap_or(a).display().to_string();
        let validos: Vec<PathBuf> = p.archives.iter().filter(|a| crate::arquivos_do_jogo::archive_valido(&a.to_string_lossy()).is_some()).cloned().collect();
        let xls_sem_json: Vec<&PathBuf> = p.xls.iter().filter(|x| !p.jsons.contains(&x.with_extension(""))).collect();
        // O que o jogo carrega deste mod: archive válido, config `.json`, blob de script.
        let carrega = !validos.is_empty() || !p.jsons.is_empty() || p.redscripts > 0;
        let parte_ou_nada = if carrega { ModStatus::Warning } else { ModStatus::Error };
        for a in p.archives.iter().filter(|a| !validos.contains(a)) {
            m.marcar(
                i,
                parte_ou_nada,
                &format!(
                    "{}: não é um .archive válido e o jogo não o monta; reinstale o mod pelo menu Mods do jogo / not a valid .archive and the game \
                     does not mount it; reinstall the mod from the game's Mods menu",
                    rel(a)
                ),
            );
        }
        for x in xls_sem_json {
            m.marcar(
                i,
                parte_ou_nada,
                &format!(
                    "{}: o jogo não lê .xl (só o .json de mesmo nome), e o BWMS não aplica .xl do mod.io / the game does not read .xl (only the \
                     .json with the same name), and BWMS does not apply mod.io .xl files",
                    rel(x)
                ),
            );
        }
        if p.reds > 0 {
            m.marcar(
                i,
                parte_ou_nada,
                &format!(
                    "{} .reds: o jogo não compila .reds e o BWMS não compila os do mod.io; o mesmo mod na pasta Mods entra / the game does not \
                     compile .reds and BWMS does not compile mod.io ones; the same mod in the Mods folder loads",
                    p.reds
                ),
            );
        }
        if p.tweaks_de_texto > 0 {
            m.marcar(
                i,
                parte_ou_nada,
                &format!(
                    "{} .yaml/.tweak: o jogo só lê tweaks .json do mod.io e o BWMS não aplica os do mod.io; o mesmo mod na pasta Mods entra / the \
                     game only reads .json tweaks from mod.io and BWMS does not apply mod.io ones; the same mod in the Mods folder loads",
                    p.tweaks_de_texto
                ),
            );
        }
        if p.luas + p.dlls > 0 {
            m.marcar(
                i,
                parte_ou_nada,
                ".lua (mod Lua) só roda pela pasta Mods e .dll (plugin de Windows) não carrega no Mac / .lua (Lua mod) only runs from the Mods \
                 folder and .dll (Windows plugin) does not load on a Mac",
            );
        }
        m.archives[i] = validos;
    }
    m
}

/// A home do usuário (`$HOME`). Nos testes, nenhuma: a lista não pode depender dos mods do mod.io da máquina que roda o teste (a leitura tem
/// teste próprio sobre pastas falsas).
#[cfg(not(test))]
pub(crate) fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
pub(crate) fn home() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(nome: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("bwms-modio-{nome}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn escrever(p: &Path, conteudo: &[u8]) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, conteudo).unwrap();
    }

    /// `RDAR` v12 com o índice em 4096 e um recurso por hash de `hashes`: passa no `archive_valido` e no `hashes_do_archive`.
    fn archive(hashes: &[u64]) -> Vec<u8> {
        let mut indice = vec![0u8; 16];
        indice.extend_from_slice(&(hashes.len() as u32).to_le_bytes());
        indice.extend_from_slice(&[0u8; 8]);
        for h in hashes {
            let mut r = [0u8; bwms_primitives::rdar::TAMANHO_DO_RECURSO];
            r[..8].copy_from_slice(&h.to_le_bytes());
            indice.extend_from_slice(&r);
        }
        let mut b = vec![0u8; 4096];
        b[..4].copy_from_slice(b"RDAR");
        b[4..8].copy_from_slice(&12u32.to_le_bytes());
        b[8..16].copy_from_slice(&4096u64.to_le_bytes());
        b[16..20].copy_from_slice(&(indice.len() as u32).to_le_bytes());
        b.extend_from_slice(&indice);
        b
    }

    fn lista(pastas: &[(&Path, i64, &str)]) -> String {
        let itens: Vec<String> = pastas
            .iter()
            .map(|(p, id, nome)| {
                format!("{{\"modPath\": {}, \"modID\": {id}, \"modName\": {}}}", bwms_primitives::json::texto(&p.display().to_string()), bwms_primitives::json::texto(nome))
            })
            .collect();
        format!("{{\"version\": 6, \"enabled_mods\": [{}]}}", itens.join(", "))
    }

    /// Os 2 mods do dono no formato real (Mod Um: archive + `.archive.json` + `.archive.xl` + `r6/` vazio; Mod Dois: só o archive), um desligado, um ligado que só traz `.xl` e `.reds`, um com archive inválido e um ligado sem pasta: ordem do
    /// jogo (id), luz e motivo de cada um, e nada escrito.
    #[test]
    fn le_os_mods_com_a_luz_de_cada_um() {
        let b = base("luzes");
        let mods = b.join("mods");
        let (ir, lcdt, desl, so_xl, ruim, sumiu) =
            (mods.join("6383233"), mods.join("6428265"), mods.join("6000001"), mods.join("6500000"), mods.join("6600000"), mods.join("6700000"));
        escrever(&ir.join("about.json"), b"{\n\t\"ModInfo\": {\n\t\t\"ModName\": \"ModUm\",\n\t\t\"ModID\": -1\n\t}\n}");
        escrever(&ir.join("archive/ModUm.archive"), &archive(&[1, 2]));
        escrever(&ir.join("archive/ModUm.archive.json"), b"{\"journal\": []}");
        escrever(&ir.join("archive/ModUm.archive.xl"), b"journal: []");
        std::fs::create_dir_all(ir.join("r6/scripts")).unwrap();
        escrever(&lcdt.join("about.json"), b"{\"ModInfo\": {\"ModName\": \"ModDois\"}}");
        escrever(&lcdt.join("archive/ModDois.archive"), &archive(&[0xffa23560ec50cdc4]));
        escrever(&desl.join("about.json"), b"{\"ModInfo\": {\"ModName\": \"Desligado\"}}");
        escrever(&desl.join("archive/d.archive"), &archive(&[9]));
        escrever(&so_xl.join("archive/x.archive.xl"), b"localization: {}");
        escrever(&so_xl.join("r6/scripts/x/x.reds"), b"// x");
        escrever(&ruim.join("archive/r.archive"), b"nao e archive");
        escrever(&ruim.join("archive/r.archive.json"), b"{}");
        let enabled = b.join("EnabledMods.json");
        escrever(
            &enabled,
            lista(&[
                (&lcdt, 6428265, "Mod Dois"),
                (&ir, 6383233, "Mod Um"),
                (&so_xl, 6500000, "So Xl"),
                (&ruim, 6600000, "Ruim"),
                (&sumiu, 6700000, "Sumiu"),
            ])
            .as_bytes(),
        );
        let antes = std::fs::read(&enabled).unwrap();
        let m = ler_em(&mods, &enabled);
        assert_eq!(
            m.rotulos,
            [
                "Mod Um (mod.io 6383233)",
                "Mod Dois (mod.io 6428265)",
                "So Xl (mod.io 6500000)",
                "Ruim (mod.io 6600000)",
                "Sumiu (mod.io 6700000)",
                "Desligado (mod.io 6000001)",
            ]
        );
        assert_eq!(m.ligados, [Some(true), Some(true), Some(true), Some(true), Some(true), Some(false)]);
        assert_eq!(m.luzes, [ModStatus::Ok, ModStatus::Ok, ModStatus::Error, ModStatus::Warning, ModStatus::Error, ModStatus::Inactive]);
        assert!(m.motivos[0].is_empty() && m.motivos[1].is_empty());
        assert!(m.motivos[2].contains("archive/x.archive.xl: o jogo não lê .xl") && m.motivos[2].contains("1 .reds: o jogo não compila"), "{}", m.motivos[2]);
        assert!(m.motivos[3].starts_with("archive/r.archive: não é um .archive válido"), "{}", m.motivos[3]);
        assert!(m.motivos[4].starts_with("a pasta do mod não existe"));
        assert!(m.motivos[5].starts_with("desligado no menu Mods do jogo"));
        assert_eq!(m.archives[1], [lcdt.join("archive/ModDois.archive")]);
        assert!(m.archives[3].is_empty(), "o archive inválido não disputa recurso");
        let ordem: Vec<String> = m.na_ordem_de_carga().into_iter().map(|(_, r, _)| r).collect();
        assert_eq!(ordem.len(), 5, "o desligado fica fora da carga");
        assert_eq!(std::fs::read(&enabled).unwrap(), antes, "o EnabledMods.json não é tocado");
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Tweak do mod.io: o `tweaks/*.json` é do carregador do jogo (`TweakXLModLoader` `0x1037f93e0`) e o mod fica verde; `.yaml`/`.tweak` o
    /// jogo não lê — sozinho o mod fica vermelho, com `.json` amarelo, e o motivo diz que pela `Mods/` entra.
    #[test]
    fn tweaks_do_modio_json_do_jogo_e_yaml_avisado() {
        let b = base("tweaks");
        let mods = b.join("mods");
        let (json, yaml, misto) = (mods.join("7000001"), mods.join("7000002"), mods.join("7000003"));
        escrever(&json.join("tweaks/meu_mod/itens.json"), b"{}");
        escrever(&yaml.join("tweaks/meu_mod/itens.yaml"), b"Items.X:\n  mass: 1\n");
        escrever(&misto.join("tweaks/a.json"), b"{}");
        escrever(&misto.join("tweaks/b.tweak"), b"X { y = 1; }");
        let enabled = b.join("EnabledMods.json");
        escrever(&enabled, lista(&[(&json, 7000001, "Json"), (&yaml, 7000002, "Yaml"), (&misto, 7000003, "Misto")]).as_bytes());
        let m = ler_em(&mods, &enabled);
        assert_eq!(m.luzes, [ModStatus::Ok, ModStatus::Error, ModStatus::Warning]);
        assert!(m.motivos[0].is_empty());
        assert!(m.motivos[1].starts_with("1 .yaml/.tweak: o jogo só lê tweaks .json do mod.io") && m.motivos[1].contains("pasta Mods entra"), "{}", m.motivos[1]);
        assert!(m.motivos[2].starts_with("1 .yaml/.tweak"), "{}", m.motivos[2]);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Sem `EnabledMods.json` nenhum mod do mod.io está ligado (o jogo o regrava ao ligar o 1º); ilegível, os do disco ficam cinza com o erro
    /// e a ação, sem estado inventado; sem a pasta do mod.io, nada.
    #[test]
    fn lista_ausente_ou_ilegivel() {
        let b = base("ilegivel");
        let mods = b.join("mods");
        escrever(&mods.join("6428265/archive/a.archive"), &archive(&[1]));
        let ausente = ler_em(&mods, &b.join("nao-existe.json"));
        assert_eq!((ausente.ligados.clone(), ausente.luzes.clone()), (vec![Some(false)], vec![ModStatus::Inactive]));
        escrever(&b.join("EnabledMods.json"), b"{\"version\": 6}");
        let ilegivel = ler_em(&mods, &b.join("EnabledMods.json"));
        assert_eq!((ilegivel.ligados.clone(), ilegivel.luzes.clone()), (vec![None], vec![ModStatus::Inactive]));
        assert!(ilegivel.motivos[0].contains("EnabledMods.json: sem a lista enabled_mods") && ilegivel.motivos[0].contains("abra o menu Mods"));
        assert_eq!(ler_em(&b.join("sem-modio"), &b.join("nao-existe.json")), ModsDoModIo::vazio());
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Custo da leitura do mod.io real da máquina (só leitura) e do índice dos archives dele, o que o boot paga a mais. Rodar com
    /// `cargo test --lib medida_da_leitura_do_modio -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn medida_da_leitura_do_modio() {
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
        const VOLTAS: u32 = 50;
        let t = std::time::Instant::now();
        let mut m = ModsDoModIo::vazio();
        for _ in 0..VOLTAS {
            m = std::hint::black_box(ler(&home));
        }
        let leitura = t.elapsed().as_secs_f64() * 1000.0 / VOLTAS as f64;
        let t = std::time::Instant::now();
        let mut recursos = 0;
        for _ in 0..VOLTAS {
            recursos = m.archives.iter().flatten().filter_map(|a| crate::arquivos_do_jogo::hashes_do_archive(a)).map(|h| h.len()).sum::<usize>();
        }
        let indices = t.elapsed().as_secs_f64() * 1000.0 / VOLTAS as f64;
        eprintln!(
            "[medida] mod.io: {} mod(s) ({:?}); leitura {leitura:.3} ms; índices de {} archive(s), {recursos} recurso(s): {indices:.3} ms ({VOLTAS} voltas, cache quente)",
            m.len(),
            m.rotulos,
            m.archives.iter().flatten().count()
        );
    }
}
