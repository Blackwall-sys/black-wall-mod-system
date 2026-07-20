//! APPLY do ArchiveXL — camada PURA (offline, testável sem jogo): hash de ResourcePath +
//! construção do mapa de redirecionamento (resource.link / resource.copy) a partir de
//! [`crate::xl::XlFile`]. A camada ENGINE (hook no resolve do depot, in-game) é GATED e mora no
//! runtime (cp77-console), NÃO aqui. RE do loader em `cp77-symbols/notes/archivexl-apply-re.md`.

use crate::xl::XlFile;
use std::collections::HashMap;

/// Hash de ResourcePath do CP2077 = **FNV-1a 64-bit** do path normalizado (lowercase, separador `\`).
/// FONTE ÚNICA no crate `bwms-hashes` (compartilhada com o runtime cp77-console). VALIDADO offline:
/// 5/5 goldens do índice de um .archive real batem exato (`tests::hash_goldens`, abaixo).
pub use bwms_hashes::resource_path_hash;

/// Plano de apply: o que dá pra aplicar via REDIRECIONAMENTO de path (link/copy) — já codável —
/// e o que ainda NÃO tem caminho de runtime (patch/scope/fix/localization/factories), honesto.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XlApplyPlan {
    /// `hash(path pedido) → hash(path servido)`. De `resource.link` (direção depende da forma
    /// scalar/sequência — ver `build_apply_plan`, já com CADEIAS A→B→C colapsadas) e
    /// `resource.copy` (cada target→source, sem encadeamento — fiel ao C++).
    pub redirects: HashMap<u64, u64>,
    pub links: usize,
    pub copies: usize,
    /// Links que formavam um CICLO (A→B→...→A) — removidos do `redirects` (fiel ao C++:
    /// `Extension.cpp` loga erro e descarta, não aplica). Contagem, não os hashes (não há como
    /// reverter um hash pro path original).
    pub cyclic_links: usize,
    /// ops sem caminho de apply hoje (precisam de RE/hook ainda inexistente), com nome+contagem.
    pub unsupported: Vec<String>,
}

/// Colapsa cadeias `A→B→C→...` num mapa de links **em-lugar**, seguindo EXATAMENTE o algoritmo
/// de `ResourceLink/Extension.cpp::Configure()` (RE lida, não suposição): pra cada entrada,
/// persegue `links[target]` repetidamente (só dentro do PRÓPRIO mapa de links — `copies` nunca
/// participa de cadeia, fiel ao C++) até achar um alvo que não é mais chave de nenhum link
/// (resolução final) ou revisitar um hop já visto nesta cadeia (CICLO). Cadeias resolvidas viram
/// `link[X] = alvo_final`; ciclos são REMOVIDOS do mapa (o C++ loga erro e descarta — replicado
/// aqui só como remoção, sem log, já que é lib pura sem I/O). Devolve a contagem de ciclos.
fn resolve_link_chains(links: &mut HashMap<u64, u64>) -> usize {
    let keys: Vec<u64> = links.keys().copied().collect();
    let mut cyclic = Vec::new();
    for start in keys {
        let Some(&first_target) = links.get(&start) else { continue };
        let mut target = first_target;
        let mut hops: Vec<u64> = vec![start, target];
        let mut is_cyclic = false;
        while let Some(&next) = links.get(&target) {
            target = next;
            if hops.contains(&target) {
                is_cyclic = true;
                break;
            }
            hops.push(target);
        }
        if is_cyclic {
            cyclic.push(start);
        } else {
            links.insert(start, target);
        }
    }
    for k in &cyclic {
        links.remove(k);
    }
    cyclic.len()
}

/// Constrói o plano (link + copy → `redirects` hashados).
///
/// `link` — **direção depende da FORMA no YAML** (assimetria real do C++ upstream, RE
/// `ResourceLink/Config.cpp` + `Extension.cpp`, ver doc de `ResourceLink::is_sequence_form`):
/// - forma SCALAR (`target: source`): `redirects[target] = source` (o padrão "alvo fake → fonte real").
/// - forma SEQUÊNCIA (`target: [item1, item2]`): **INVERTIDO** — `redirects[item] = target` pra
///   CADA item da lista (padrão de migração: itens antigos redirecionam pro path novo/real).
///
/// `copy`: cada `target` é uma cópia de `source` → sempre `redirects[target] = source`
/// (Config.cpp não tem essa assimetria pra copy — as 2 formas armazenam igual).
pub fn build_apply_plan(xl: &XlFile) -> XlApplyPlan {
    let mut p = XlApplyPlan::default();
    let mut links: HashMap<u64, u64> = HashMap::new();
    for l in &xl.links {
        if l.is_sequence_form {
            for src in &l.sources {
                links.insert(resource_path_hash(src), resource_path_hash(&l.target));
                p.links += 1;
            }
        } else if let Some(src) = l.sources.first() {
            links.insert(resource_path_hash(&l.target), resource_path_hash(src));
            p.links += 1;
        }
    }
    p.cyclic_links = resolve_link_chains(&mut links);
    p.links -= p.cyclic_links;
    p.redirects.extend(links);
    for c in &xl.copies {
        for t in &c.targets {
            p.redirects.insert(resource_path_hash(t), resource_path_hash(&c.source));
            p.copies += 1;
        }
    }
    let mut note = |n: usize, what: &str| {
        if n > 0 {
            p.unsupported.push(format!("{what}×{n}"));
        }
    };
    note(xl.factories.len(), "factories (gated: hook LoadFactoryAsync)");
    note(xl.patches.len(), "patches (gated: patch de props em runtime)");
    note(xl.scopes.len(), "scopes");
    note(xl.fixes.len(), "fixes");
    note(xl.localization.len(), "localization (gated: loader onscreen/subtitle)");
    p
}

/// Emite o conteúdo de `red4ext/bwms-reslink.txt` a partir do `.xl`: uma linha
/// `<path_pedido> | <path_servido>` por `resource.link` (target|sources[0]) e por `resource.copy`
/// (cada target|source). O runtime (`selftest::reslink_file`) re-hasha os DOIS lados →
/// `reslink_add(hash(pedido), hash(servido))`, casando EXATO com `build_apply_plan().redirects`.
/// Só link/copy têm caminho de runtime hoje (o resto fica em `unsupported`).
pub fn emit_reslink(xl: &XlFile) -> String {
    let mut out =
        String::from("# bwms-reslink (gerado de .xl). Formato: <path_pedido> | <path_servido>\n");
    for l in &xl.links {
        if l.is_sequence_form {
            for src in &l.sources {
                out.push_str(&format!("{} | {}\n", src, l.target));
            }
        } else if let Some(src) = l.sources.first() {
            out.push_str(&format!("{} | {}\n", l.target, src));
        }
    }
    for c in &xl.copies {
        for t in &c.targets {
            out.push_str(&format!("{} | {}\n", t, c.source));
        }
    }
    out
}

/// Emite o conteúdo de `red4ext/bwms-factories.txt` a partir do `.xl`: um path de `.csv` de factory
/// por linha (a secção `factories:` do `.xl`, FactoryIndex/Config.cpp). O runtime
/// (`selftest::factory_file`) lê 1 path por linha (`#`=comentário), re-hasha cada um via
/// `resource_path_hash` (`factory_add`) e re-injeta via HookAfter `LoadFactoryAsync` quando o
/// sentinel vanilla carrega — o mesmo padrão texto+re-hash já provado do reslink. Assim o
/// mod-manager fecha a METADE offline do uso #1 do ArchiveXL ("adicionar itens"), deixando só o
/// offset de `LoadFactoryAsync` como pendência in-game.
pub fn emit_factory_table(xl: &XlFile) -> String {
    let mut out =
        String::from("# bwms-factories (gerado de .xl). Formato: <path do .csv de factory>\n");
    for f in &xl.factories {
        out.push_str(f);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xl::{ResourceCopy, ResourceLink, XlFile};

    /// Goldens REAIS do índice de `basegame_zzbwms_quadra_e3.archive` (via archive-tool datamap).
    /// Provam que `resource_path_hash` É o hash do CP2077 (5/5 batem).
    #[test]
    fn hash_goldens() {
        let g = [
            ("base\\surfaces\\materials\\default\\white.xbm", 0x3959_590b_0d88_8df1u64),
            ("base\\vehicles\\common\\wheels\\wheel_quadra_turbo_clean_01.mlsetup", 0xc007_6ea3_a1d9_a9edu64),
            ("base\\surfaces\\microblends\\default.xbm", 0xf78f_7031_24ea_fb22u64),
            ("base\\surfaces\\materials\\metal\\metal_generic\\metal_generic_bare_01_300_d.xbm", 0x79c9_5673_c0d4_c73au64),
            ("base\\resource.cooked_mlsetup", 0x3a12_b4fd_1938_d5cau64),
        ];
        for (p, h) in g {
            assert_eq!(resource_path_hash(p), h, "hash de {p}");
        }
    }

    /// `/` vira `\` e case não importa → o path canônico e o "sujo" dão o MESMO hash.
    #[test]
    fn hash_normaliza_sep_e_case() {
        let canon = resource_path_hash("base\\surfaces\\materials\\default\\white.xbm");
        assert_eq!(resource_path_hash("BASE/Surfaces/Materials/Default/White.xbm"), canon);
    }

    #[test]
    fn plan_redirects_link_e_copy() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink {
            target: "base\\fake\\a.mesh".into(),
            sources: vec!["base\\real\\b.mesh".into()],
            is_sequence_form: false,
        });
        xl.copies.push(ResourceCopy {
            source: "base\\real\\c.mesh".into(),
            targets: vec!["base\\new\\d.mesh".into(), "base\\new\\e.mesh".into()],
        });
        let p = build_apply_plan(&xl);
        assert_eq!(p.links, 1);
        assert_eq!(p.copies, 2);
        assert_eq!(p.redirects.len(), 3);
        assert_eq!(p.redirects[&resource_path_hash("base\\fake\\a.mesh")], resource_path_hash("base\\real\\b.mesh"));
        assert_eq!(p.redirects[&resource_path_hash("base\\new\\d.mesh")], resource_path_hash("base\\real\\c.mesh"));
    }

    /// O emissor produz linhas que, RE-HASHADAS como o runtime faz (`selftest::reslink_file`),
    /// reproduzem EXATO o mapa de `build_apply_plan` → prova o round-trip emissor↔runtime sem jogo.
    #[test]
    fn emit_reslink_reproduz_o_plano() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink {
            target: "base\\fake\\a.mesh".into(),
            sources: vec!["base\\real\\b.mesh".into()],
            is_sequence_form: false,
        });
        xl.copies.push(ResourceCopy {
            source: "base\\real\\c.mesh".into(),
            targets: vec!["base\\new\\d.mesh".into(), "base\\new\\e.mesh".into()],
        });
        let plan = build_apply_plan(&xl);
        // parseia o emitido EXATAMENTE como selftest::reslink_file (split '|', re-hash dos 2 lados)
        let mut got: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
        for line in emit_reslink(&xl).lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((s, t)) = line.split_once('|') {
                got.insert(resource_path_hash(s.trim()), resource_path_hash(t.trim()));
            }
        }
        assert_eq!(got, plan.redirects);
        assert_eq!(got.len(), 3);
    }

    /// O emissor de factory produz linhas que, RE-HASHADAS como o runtime faz
    /// (`selftest::factory_file` → `factory_add` → `resource_path_hash`), reproduzem os hashes que o
    /// hook LoadFactoryAsync re-injeta. Prova o round-trip emissor↔runtime do factory sem jogo.
    #[test]
    fn emit_factory_table_reproduz_os_paths() {
        let mut xl = XlFile::default();
        xl.factories = vec![
            "mymod\\factories\\clothing.csv".into(),
            "mymod\\factories\\weapons.csv".into(),
        ];
        // parseia o emitido como o runtime (1 path/linha, `#`=comentário, re-hash de cada um).
        let mut got: Vec<u64> = Vec::new();
        for line in emit_factory_table(&xl).lines() {
            let l = line.trim();
            if !l.is_empty() && !l.starts_with('#') {
                got.push(resource_path_hash(l));
            }
        }
        let want: Vec<u64> = xl.factories.iter().map(|f| resource_path_hash(f)).collect();
        assert_eq!(got, want);
        assert_eq!(got.len(), 2);
        // xl vazio → só o header comentado, 0 paths.
        assert_eq!(emit_factory_table(&XlFile::default()).lines().filter(|l| !l.starts_with('#')).count(), 0);
    }

    /// **`axl-copy-link-direction-validate` + `axl-link-scalar-seq-asymmetry` (2026-07-13).**
    /// Corpus de validação da direção scalar vs sequência de `resource.link` CONTRA a fonte C++
    /// real (`enablers/ArchiveXL/src/App/Extensions/ResourceLink/{Config,Extension}.cpp`), traçada
    /// à mão instrução-a-instrução (não é suposição):
    ///
    /// - `Config.cpp` guarda os 2 casos com key/value TROCADOS: scalar faz
    ///   `links[sourcePath].insert(targetPath)` (map key=fonte real); sequência faz
    ///   `links[targetPath].insert(sourcePath)` (map key=alvo).
    /// - `Extension.cpp::Configure()` consome AMBOS com `finalLinks[member_do_set] = map_key` —
    ///   então o resultado FINAL diverge: scalar dá `finalLinks[alvo]=fonte`; sequência dá
    ///   `finalLinks[cada_item_da_lista]=alvo` (INVERTIDO em relação ao scalar).
    /// - Confirmado contra o USO REAL: `Migration.xl` (fixture em `xl.rs::link_real_migration`)
    ///   usa a forma sequência com `alvo=path NOVO`, `lista=paths ANTIGOS` — ou seja, a direção
    ///   invertida É o comportamento correto pretendido (migração: nomes antigos → path novo),
    ///   não um bug do C++ pra "consertar", mas uma FORMA DIFERENTE que nosso parser não
    ///   diferenciava (tratava as 2 formas identicamente, sempre como scalar).
    ///
    /// Este teste prova que, DEPOIS do fix (`is_sequence_form`), as 2 formas produzem as
    /// direções CORRETAS e DIFERENTES uma da outra — antes do fix, ambas dariam `target→item[0]`.
    #[test]
    fn link_direcao_scalar_vs_sequencia_bate_com_cpp() {
        let mut xl = XlFile::default();
        // forma SCALAR: "fake: real" -> fake resolve pra real (redirects[fake] = real).
        xl.links.push(ResourceLink {
            target: "base\\fake\\a.mesh".into(),
            sources: vec!["base\\real\\b.mesh".into()],
            is_sequence_form: false,
        });
        // forma SEQUÊNCIA (padrão Migration.xl): "novo: [antigo1, antigo2]" -> CADA antigo
        // resolve pro novo (redirects[antigoN] = novo), NÃO "novo -> antigo1".
        xl.links.push(ResourceLink {
            target: "base\\novo\\h1.mesh".into(),
            sources: vec!["base\\antigo\\base.mesh".into(), "base\\antigo\\legacy.mesh".into()],
            is_sequence_form: true,
        });
        let p = build_apply_plan(&xl);
        assert_eq!(p.links, 3); // 1 (scalar) + 2 (sequência, um por item)
        // scalar: fake -> real (igual de sempre)
        assert_eq!(p.redirects[&resource_path_hash("base\\fake\\a.mesh")], resource_path_hash("base\\real\\b.mesh"));
        // sequência: CADA item antigo -> o alvo novo (NÃO o contrário)
        assert_eq!(p.redirects[&resource_path_hash("base\\antigo\\base.mesh")], resource_path_hash("base\\novo\\h1.mesh"));
        assert_eq!(p.redirects[&resource_path_hash("base\\antigo\\legacy.mesh")], resource_path_hash("base\\novo\\h1.mesh"));
        // confirma que NÃO ficou com a direção do scalar (que seria o bug pré-fix)
        assert_ne!(
            p.redirects.get(&resource_path_hash("base\\novo\\h1.mesh")),
            Some(&resource_path_hash("base\\antigo\\base.mesh"))
        );
    }

    /// Mesmo corpus, mas passando pelo PARSER de verdade (YAML → `XlFile` → plano) — fecha o
    /// round-trip inteiro pro fixture REAL de migração (mesmo YAML de `xl.rs::link_real_migration`).
    #[test]
    fn link_migration_real_end_to_end_direcao_correta() {
        let src = "resource:\n  link:\n    archive_xl\\a\\h1.mesh:\n      - archive_xl\\a\\base.mesh\n";
        let xl = crate::xl::parse_xl(src).expect("parse");
        assert_eq!(xl.links.len(), 1);
        assert!(xl.links[0].is_sequence_form, "Migration.xl usa a forma sequência");
        let p = build_apply_plan(&xl);
        // "base.mesh" (nome antigo, na lista) deve resolver pro "h1.mesh" (o alvo/path novo).
        assert_eq!(
            p.redirects[&resource_path_hash("archive_xl\\a\\base.mesh")],
            resource_path_hash("archive_xl\\a\\h1.mesh")
        );
    }

    /// **`axl-link-nton-table` (2026-07-13).** Cadeia A→B→C colapsa pro alvo FINAL (C), igual
    /// `Extension.cpp::Configure()` (persegue `finalLinks[target]` até não achar mais chave,
    /// substitui `link.value()` pelo alvo final) — replicado em `resolve_link_chains`.
    #[test]
    fn link_cadeia_a_para_b_para_c_colapsa_no_final() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink {
            target: "a".into(),
            sources: vec!["b".into()],
            is_sequence_form: false,
        });
        xl.links.push(ResourceLink {
            target: "b".into(),
            sources: vec!["c".into()],
            is_sequence_form: false,
        });
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, 0);
        // a -> b -> c colapsa em a -> c (não em a -> b)
        assert_eq!(p.redirects[&resource_path_hash("a")], resource_path_hash("c"));
        // b -> c continua valendo por si só (é o alvo final da própria cadeia de b)
        assert_eq!(p.redirects[&resource_path_hash("b")], resource_path_hash("c"));
    }

    /// Ciclo A→B→A é detectado e REMOVIDO (não aplicado) — fiel ao C++ (`LogError` + `erase`).
    #[test]
    fn link_ciclo_e_detectado_e_removido() {
        let mut xl = XlFile::default();
        xl.links.push(ResourceLink {
            target: "a".into(),
            sources: vec!["b".into()],
            is_sequence_form: false,
        });
        xl.links.push(ResourceLink {
            target: "b".into(),
            sources: vec!["a".into()],
            is_sequence_form: false,
        });
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, 2);
        assert_eq!(p.links, 0);
        assert!(!p.redirects.contains_key(&resource_path_hash("a")));
        assert!(!p.redirects.contains_key(&resource_path_hash("b")));
    }

    /// Uma cadeia longa (A→B→C→D) resolve até o fim, sem confundir com ciclo.
    #[test]
    fn link_cadeia_longa_resolve_ate_o_fim_sem_falso_ciclo() {
        let mut xl = XlFile::default();
        for (t, s) in [("a", "b"), ("b", "c"), ("c", "d")] {
            xl.links.push(ResourceLink { target: t.into(), sources: vec![s.into()], is_sequence_form: false });
        }
        let p = build_apply_plan(&xl);
        assert_eq!(p.cyclic_links, 0);
        assert_eq!(p.redirects[&resource_path_hash("a")], resource_path_hash("d"));
    }
}
