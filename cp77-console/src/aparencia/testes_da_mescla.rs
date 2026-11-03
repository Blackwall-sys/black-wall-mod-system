//! Testes da mescla de aparência e patch (#17/#51) e das peças que ela usa (curvas, mapas do motor, objetos salvos, texto). Rodam no
//! `cargo test` público: são funções de produto.

use super::{arrays_com_o_plano, PassoDaMescla};
use crate::aparencia::plano_da_mescla;

#[test]
fn substitui_por_nome_e_id_e_anexa_o_novo() {
    let r = [None, Some((1, 10)), Some((2, 20))];
    let p = [Some((2, 20, false)), Some((3, 30, false)), None, Some((1, 11, false))];
    assert_eq!(plano_da_mescla(&r, &p, true), vec![PassoDaMescla::Substitui(2), PassoDaMescla::Anexa, PassoDaMescla::Pula, PassoDaMescla::Anexa]);
}

#[test]
fn externo_novo_fica_de_fora_so_em_mescla_de_parte() {
    let r = [Some((5, 50))];
    let p = [Some((7, 70, true)), Some((5, 50, true))];
    assert_eq!(plano_da_mescla(&r, &p, true), vec![PassoDaMescla::Pula, PassoDaMescla::Substitui(0)]);
    assert_eq!(plano_da_mescla(&r, &p, false), vec![PassoDaMescla::Anexa, PassoDaMescla::Substitui(0)]);
}

#[test]
fn anexado_entra_nas_buscas_seguintes() {
    let p = [Some((9, 90, false)), Some((9, 90, false))];
    assert_eq!(plano_da_mescla(&[], &p, true), vec![PassoDaMescla::Anexa, PassoDaMescla::Substitui(0)]);
}

#[test]
fn aplicado_troca_os_substituidos_e_move_os_anexados() {
    let r = [[1, 11], [2, 22]];
    let p = [[3, 33], [4, 44], [5, 55]];
    let (r1, p1) = arrays_com_o_plano(&r, &p, &[PassoDaMescla::Substitui(1), PassoDaMescla::Anexa, PassoDaMescla::Pula]);
    assert_eq!(r1, vec![[1, 11], [3, 33], [4, 44]]);
    assert_eq!(p1, vec![[2, 22], [0, 0], [5, 55]]);
}

#[test]
fn tabela_de_patch_le_a_mascara_e_pula_cabecalho_e_linha_torta() {
    let t = "# cabeçalho\nm | mod\\p.ent | base\\a.ent | -\nm | mod\\p2.ent | base\\b.ent | 0x4\nm | mod\\q.ent | base\\c.ent | 0x3\nlinha torta\nm |  | base\\d.ent | 0x1\n";
    let resumo: Vec<(&str, &str, bool)> =
        crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t).into_iter().map(|l| (l.patch, l.alvo, l.props & crate::arquivos_do_jogo::A17_PROP_COMPONENTS != 0)).collect();
    assert_eq!(resumo, vec![("mod\\p.ent", "base\\a.ent", true), ("mod\\p2.ent", "base\\b.ent", false), ("mod\\q.ent", "base\\c.ent", true)]);
}

#[test]
fn tabela_posta_a_mao_le_as_props_por_nome() {
    let t = "m | mod\\p.ent | base\\a.ent | entity components\nm | mod\\q.ent | base\\b.ent | visualTags nada\nm | mod\\r.ent | base\\c.ent | 0xzz\n";
    let props: Vec<u32> = crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t).into_iter().map(|l| l.props).collect();
    assert_eq!(props, vec![crate::arquivos_do_jogo::A17_PROP_COMPONENTS | crate::arquivos_do_jogo::A17_PROP_ENTITY, crate::arquivos_do_jogo::A17_PROP_VISUALTAGS, 0]);
}

/// A tabela que o install grava para dois `.xl` com o alvo de um sendo patch do outro e o mesmo par 2× com props diferentes
/// (`bwms-primitives`, `alvo_que_e_patch_de_outro_xl_sai_e_par_repetido_fica_uma_vez_com_as_props_do_ultimo`) é lida linha a linha, sem nova
/// resolução: as linhas lidas são as gravadas pelo install, que `emit_resource_patch_table` já resolveu entre os mods.
#[test]
fn tabela_resolvida_no_install_e_lida_linha_a_linha() {
    let t = "# bwms-resource-patch (gerado de .xl). Formato: <mod> | <caminho do patch> | <caminho do alvo> | <props em máscara hex; - = todas>\nModA/a | mod\\p.ent | base\\x.ent | 0x4\nModB/b | modb\\p.ent | base\\z.ent | -\nModB/b | MOD/P.ENT | base\\y.ent | 0x4\n";
    let linha = |mod_id, patch, alvo, props| bwms_primitives::tabelas::ResourcePatch { mod_id, patch, alvo, props };
    let ap = crate::arquivos_do_jogo::A17_PROP_APPEARANCES;
    assert_eq!(
        crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t),
        vec![linha("ModA/a", "mod\\p.ent", "base\\x.ent", ap), linha("ModB/b", "modb\\p.ent", "base\\z.ent", u32::MAX), linha("ModB/b", "MOD/P.ENT", "base\\y.ent", ap)]
    );
}

#[test]
fn tabela_de_patch_marca_entity() {
    let t = "m | mod\\p.ent | base\\a.ent | -\nm | mod\\p2.ent | base\\b.ent | 0x1\nm | mod\\q.ent | base\\c.ent | 0x2\n";
    let flags: Vec<(bool, bool)> =
        crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t).into_iter().map(|l| (l.props & crate::arquivos_do_jogo::A17_PROP_COMPONENTS != 0, l.props & crate::arquivos_do_jogo::A17_PROP_ENTITY != 0)).collect();
    assert_eq!(flags, vec![(true, true), (true, false), (false, true)]);
}

#[test]
fn arquivo_de_objeto_volta_com_classe_e_pacote_e_recusa_corrompido() {
    let a = crate::salvamento::c252_montar_arquivo("entMeshComponent", &[1, 2, 3, 4, 5]);
    assert_eq!(crate::salvamento::c252_abrir_arquivo(&a), Some(("entMeshComponent", &[1u8, 2, 3, 4, 5][..])));
    assert_eq!(crate::salvamento::c252_abrir_arquivo(&a[..a.len() - 1]), None);
    let mut versao = a.clone();
    versao[4] = 2;
    assert_eq!(crate::salvamento::c252_abrir_arquivo(&versao), None);
    assert_eq!(crate::salvamento::c252_abrir_arquivo(&crate::salvamento::c252_montar_arquivo("", &[1])), None);
    assert_eq!(crate::salvamento::c252_abrir_arquivo(&crate::salvamento::c252_montar_arquivo("x", &[])), None);
    assert_eq!(crate::salvamento::c252_abrir_arquivo(b"BWOX\x01\x00\x00\x00"), None);
}

#[test]
fn caminho_de_objeto_so_relativo_sem_subir_de_pasta() {
    for ok in ["a.bwob", "teste/c252-rust.bwob", "mods/Meu_Mod/estado.1.bin"] {
        assert!(crate::salvamento::c252_caminho_valido(ok), "{ok}");
    }
    for ruim in ["", "/abs.bwob", "../fora.bwob", "a/../b", "a//b", "./a", "a/.", "com espaço", "c:\\x", "a/b\\c"] {
        assert!(!crate::salvamento::c252_caminho_valido(ruim), "{ruim}");
    }
    assert!(!crate::salvamento::c252_caminho_valido(&"a".repeat(201)));
}

#[test]
fn tabela_de_patch_le_props_do_template() {
    let t = "m | mod\\p.ent | base\\a.ent | 0xc\nm | mod\\q.ent | base\\b.ent | 0x10\n";
    let props: Vec<u32> = crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t).into_iter().map(|l| l.props).collect();
    assert_eq!(props, vec![crate::arquivos_do_jogo::A17_PROP_APPEARANCES | crate::arquivos_do_jogo::A17_PROP_DEPENDENCIES, crate::arquivos_do_jogo::A17_PROP_VISUALTAGS]);
}

#[test]
fn plano_por_nome_substitui_ou_anexa_e_anexado_entra_na_busca() {
    let p = crate::arquivos_do_jogo::plano_por_nome(&[10, 20], &[20, 30, 30, 40]);
    assert_eq!(p, vec![PassoDaMescla::Substitui(1), PassoDaMescla::Anexa, PassoDaMescla::Substitui(2), PassoDaMescla::Anexa]);
    assert_eq!(crate::arquivos_do_jogo::plano_de_novos(&[10, 20], &[20, 30, 30, 5]), vec![false, true, false, true]);
}

#[test]
fn plano_de_morph_target_segue_o_modifies_da_referencia() {
    let props = crate::arquivos_do_jogo::linhas_da_tabela_de_patches("m | mod\\p.morphtarget | base\\a.morphtarget | 0x500\n")[0].props;
    assert_eq!(props, crate::arquivos_do_jogo::A17_PROP_BASEMESHAPP | crate::arquivos_do_jogo::A17_PROP_BOUNDINGBOX);
    // Prop escrita: muda mesmo com o valor do patch vazio (menos a caixa inválida); prop ausente não muda.
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph(props, 0, 7, 0.0, 1.0), (true, false, true));
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph(props, 5, 7, 1.0, 1.0), (true, false, false));
    // Props vazio (`-`): só muda o que o patch tem.
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph(u32::MAX, 0, 7, -1.0, 2.0), (false, true, true));
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph(u32::MAX, 5, 0, 2.0, 1.0), (true, false, false));
}

#[test]
fn plano_de_definicoes_modifica_por_nome_todas_com_none_e_anexa_a_nova() {
    use crate::arquivos_do_jogo::PassoDaDefinicao::{Modifica, Nova};
    let mut anexados = Vec::new();
    let p = crate::arquivos_do_jogo::plano_das_definicoes(&[10, 20], &[20, 30, 0, 30], &mut anexados);
    assert_eq!(p, vec![Modifica(vec![1]), Nova, Modifica(vec![0, 1, 2]), Modifica(vec![])]);
    assert_eq!(anexados, vec![30]);
    // 2º patch no mesmo recurso: o nome que o 1º anexou não é modificado de novo.
    assert_eq!(crate::arquivos_do_jogo::plano_das_definicoes(&[10, 20, 30], &[30, 10], &mut anexados), vec![Modifica(vec![]), Modifica(vec![0])]);
    let t = "m | mod\\p.app | base\\a.app | 0xe4\n";
    let props: Vec<u32> = crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t).into_iter().map(|l| l.props).collect();
    assert_eq!(
        props,
        vec![crate::arquivos_do_jogo::A17_PROP_APPEARANCES | crate::arquivos_do_jogo::A17_PROP_PARTSVALUES | crate::arquivos_do_jogo::A17_PROP_CENSORSHIP | crate::arquivos_do_jogo::A17_PROP_PARTSOVERRIDES]
    );
}

#[test]
fn plano_de_entidade_so_troca_raiz_valida_e_derivada() {
    assert_eq!(crate::arquivos_do_jogo::plano_de_mescla_da_entidade(0, 3, 0, 2, true), Some((0, 0)));
    assert_eq!(crate::arquivos_do_jogo::plano_de_mescla_da_entidade(0, 3, 0, 2, false), None);
    assert_eq!(crate::arquivos_do_jogo::plano_de_mescla_da_entidade(-1, 3, 0, 2, true), None);
    assert_eq!(crate::arquivos_do_jogo::plano_de_mescla_da_entidade(0, 3, 2, 2, true), None);
    assert_eq!(crate::arquivos_do_jogo::plano_de_mescla_da_entidade(1, 3, 1, 2, true), Some((1, 1)));
}

#[test]
fn bitset_do_extrator_liga_os_n_primeiros_objetos() {
    assert_eq!(crate::aparencia::bitset_do_extrator(0), Vec::<u32>::new());
    assert_eq!(crate::aparencia::bitset_do_extrator(1), vec![1]);
    assert_eq!(crate::aparencia::bitset_do_extrator(32), vec![u32::MAX]);
    assert_eq!(crate::aparencia::bitset_do_extrator(33), vec![u32::MAX, 1]);
    assert_eq!(crate::aparencia::bitset_do_extrator(70), vec![u32::MAX, u32::MAX, 0x3f]);
}

#[test]
fn params_do_extrator_seguem_o_extractpartcomponents() {
    let cab: [u8; 0x70] = core::array::from_fn(|i| i as u8);
    let p = crate::aparencia::params_do_extrator(&cab, 0x1122_3344_5566_7788, [0, 1, 1, 0, 0], 0xdead_beef, 3, 70);
    assert_eq!(&p[0x00..0x18], &cab[0x18..0x30]);
    assert_eq!(&p[0x18..0x58], &cab[0x30..0x70]);
    assert_eq!(u64::from_le_bytes(p[0x58..0x60].try_into().unwrap()), 0x1122_3344_5566_7788);
    assert_eq!(&p[0x60..0x68], &[0u8, 1, 1, 0, 0, 0, 0, 0]);
    assert_eq!(u64::from_le_bytes(p[0x68..0x70].try_into().unwrap()), 0xdead_beef);
    assert_eq!(&p[0x70..0x7c], &[3u8, 0, 0, 0, 3, 0, 0, 0, 70, 0, 0, 0]);
    assert!(p[0x7c..].iter().all(|b| *b == 0));
}

#[test]
fn pares_da_amostra_saem_dos_substituidos() {
    let r = [[1, 0], [30, 0], [40, 0]];
    let p = [[20, 0], [0, 0], [5, 0]];
    assert_eq!(crate::arquivos_do_jogo::pares_substituidos(&[PassoDaMescla::Substitui(1), PassoDaMescla::Anexa, PassoDaMescla::Pula], &r, &p), vec![(30, 20)]);
    assert_eq!(crate::arquivos_do_jogo::pares_substituidos(&[PassoDaMescla::Substitui(9)], &r, &p), Vec::<(u64, u64)>::new());
}

#[test]
fn objetos_do_cabecalho_saem_dos_ponteiros_de_inicio_e_fim() {
    let mut c = [0u8; 0x70];
    assert_eq!(super::objetos_do_cabecalho(&c), 0);
    c[0x20..0x28].copy_from_slice(&0x1000u64.to_le_bytes());
    c[0x28..0x30].copy_from_slice(&0x1040u64.to_le_bytes());
    assert_eq!(super::objetos_do_cabecalho(&c), 8);
    c[0x28..0x30].copy_from_slice(&0x0ff8u64.to_le_bytes());
    assert_eq!(super::objetos_do_cabecalho(&c), 0);
}

#[test]
fn plano_de_curvas_copia_mesmo_nome_e_tipo_e_separa_tipo_diferente_e_nova() {
    use crate::curvas::A51gPasso::{Copia, Nova, TipoDiferente};
    let alvo = [(10, 1), (20, 1), (30, 2)];
    let patch = [(20, 1), (30, 1), (40, 1), (10, 1)];
    assert_eq!(crate::curvas::a51g_plano(&alvo, &patch), vec![Copia(1), TipoDiferente(2), Nova, Copia(0)]);
    assert_eq!(crate::curvas::a51g_plano(&[], &[(5, 1)]), vec![Nova]);
}

/// O slot 1 só é chamado na vtable conferida do `RawBufferAllocator`: objeto com outra vtable, ou sem vtable, não salta para lixo.
#[test]
fn liberar_curva_so_chama_o_alocador_conferido() {
    use std::sync::atomic::{AtomicU64, Ordering};
    static CHAMADAS: AtomicU64 = AtomicU64::new(0);
    unsafe extern "C" fn liberar_falso(_obj: *mut u8, _dados: u64) {
        CHAMADAS.fetch_add(1, Ordering::Relaxed);
    }
    let falso: unsafe extern "C" fn(*mut u8, u64) = liberar_falso;
    let vtable = [0u64, falso as usize as u64, 0];
    let mut obj = [vtable.as_ptr() as u64, 0];
    unsafe { crate::curvas::a51g_liberar(obj.as_mut_ptr() as *mut u8, 0x1234) };
    assert_eq!(CHAMADAS.load(Ordering::Relaxed), 0, "vtable que não é a do RawBufferAllocator foi chamada");
    obj[0] = 0;
    unsafe { crate::curvas::a51g_liberar(obj.as_mut_ptr() as *mut u8, 0x1234) };
    assert_eq!(CHAMADAS.load(Ordering::Relaxed), 0, "objeto sem vtable foi chamado");
}

#[test]
fn inclusao_de_partes_pede_parte_e_tag_na_definicao_ou_schema_forcando() {
    let tag = crate::aparencia::tag_appearance_parts();
    assert_ne!(tag, 0);
    assert!(!crate::aparencia::definicao_inclui_partes(0, &[tag], true, tag));
    assert!(!crate::aparencia::definicao_inclui_partes(2, &[1, 2], false, tag));
    assert!(crate::aparencia::definicao_inclui_partes(2, &[1, tag], false, tag));
    assert!(crate::aparencia::definicao_inclui_partes(1, &[], true, tag));
}

#[test]
fn anexados_da_parte_saem_do_patch_lido_antes_da_mescla() {
    let plano = [PassoDaMescla::Substitui(0), PassoDaMescla::Anexa, PassoDaMescla::Pula, PassoDaMescla::Anexa];
    let patch = [[10, 1], [20, 2], [30, 3], [40, 4]];
    assert_eq!(crate::aparencia::instancias_anexadas(&plano, &patch), vec![20, 40]);
    assert!(crate::aparencia::instancias_anexadas(&[PassoDaMescla::Anexa], &[]).is_empty());
}

#[test]
fn capacidade_do_mapa_cresce_como_o_emplace_do_sdk() {
    assert_eq!(crate::jogo::mapas::capacidade_para(38114, 38114, 1), 57171);
    assert_eq!(crate::jogo::mapas::capacidade_para(0, 0, 1), 4);
    assert_eq!(crate::jogo::mapas::capacidade_para(10, 20, 5), 20);
    assert_eq!(crate::jogo::mapas::capacidade_para(4, 4, 10), 19);
}

#[test]
fn proximo_no_livre_segue_o_get_next_avail_node() {
    let sem_lista = |_: u32| panic!("não devia ler a lista de livres");
    assert_eq!(crate::jogo::mapas::proximo_livre(0, 0, 4, sem_lista), Some((0, 1, 1)));
    assert_eq!(crate::jogo::mapas::proximo_livre(3, 3, 4, sem_lista), Some((3, u32::MAX, 3)));
    assert_eq!(crate::jogo::mapas::proximo_livre(u32::MAX, 4, 4, sem_lista), None);
    assert_eq!(crate::jogo::mapas::proximo_livre(1, 3, 4, |i| if i == 1 { 2 } else { 9 }), Some((1, 2, 3)));
}

#[test]
fn plano_da_51d_v2_segue_o_modifies_da_referencia() {
    use crate::arquivos_do_jogo::{A17_PROP_BASEMESH as M, A17_PROP_BASETEXTURE as T, A17_PROP_TARGETS as G};
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph_com_dono(u32::MAX, 0, 0), (false, false, true));
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph_com_dono(u32::MAX, 5, 7), (true, true, true));
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph_com_dono(M | T, 0, 0), (false, true, false));
    assert_eq!(crate::arquivos_do_jogo::plano_do_morph_com_dono(G, 5, 7), (false, false, true));
}

#[test]
fn parser_le_as_props_de_morph_e_mesh() {
    let t = "m | mod\\p.morphtarget | base\\a.morphtarget | 0x1f000\n";
    let l = crate::arquivos_do_jogo::linhas_da_tabela_de_patches(t);
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].props, crate::arquivos_do_jogo::A17_PROP_BASEMESH | crate::arquivos_do_jogo::A17_PROP_BASETEXTURE | crate::arquivos_do_jogo::A17_PROP_BLOB | crate::arquivos_do_jogo::A17_PROP_TARGETS | crate::arquivos_do_jogo::A17_PROP_RENDERBLOB);
}

#[test]
fn blob_do_morph_segue_o_modifies_com_o_alvo_tendo_blob() {
    assert!(crate::aparencia::morph_blob_muda(u32::MAX, true, false));
    assert!(!crate::aparencia::morph_blob_muda(u32::MAX, true, true));
    assert!(crate::aparencia::morph_blob_muda(crate::arquivos_do_jogo::A17_PROP_BLOB, true, true));
    assert!(!crate::aparencia::morph_blob_muda(crate::arquivos_do_jogo::A17_PROP_BLOB, false, false));
    assert!(!crate::aparencia::morph_blob_muda(crate::arquivos_do_jogo::A17_PROP_TARGETS, true, false));
}

#[test]
fn blob_do_mesh_segue_o_modifies_com_o_alvo_tendo_blob() {
    use crate::arquivos_do_jogo::A17_PROP_RENDERBLOB as B;
    assert!(!crate::aparencia::render_blob_muda(u32::MAX, false, false));
    assert!(crate::aparencia::render_blob_muda(u32::MAX, true, false));
    assert!(!crate::aparencia::render_blob_muda(u32::MAX, true, true));
    assert!(crate::aparencia::render_blob_muda(B, true, true));
    assert!(!crate::aparencia::render_blob_muda(crate::arquivos_do_jogo::A17_PROP_APPEARANCES, true, false));
}

#[test]
fn fix_de_nomes_le_so_names_e_o_ultimo_config_vence() {
    let t = "# cabeçalho\nbase\\m.mesh | names | @a | @b\nbase\\m.mesh | paths | x | y\nbase\\m.mesh | names | @a | @c\nbase\\n.mesh | names | @a | None\ntorta\n";
    let (nomes, caminhos) = crate::mods_do_usuario::pipeline::resfixes_linhas(t);
    let m = bwms_primitives::resource_path_hash("base\\m.mesh");
    let n = bwms_primitives::resource_path_hash("base\\n.mesh");
    assert_eq!(nomes.len(), 2);
    assert_eq!(nomes[&m], vec![(bwms_primitives::fnv1a64(b"@a"), bwms_primitives::fnv1a64(b"@c"))]);
    assert_eq!(nomes[&n], vec![(bwms_primitives::fnv1a64(b"@a"), 0)]);
    assert_eq!(caminhos[&m], vec![(bwms_primitives::resource_path_hash("x"), bwms_primitives::resource_path_hash("y"))]);
}

#[test]
fn definicao_do_patch_e_a_primeira_homonima_ou_sem_nome() {
    assert_eq!(crate::aparencia::definicao_do_patch(&[5, 0, 7], 7), Some(1));
    assert_eq!(crate::aparencia::definicao_do_patch(&[5, 7, 0], 7), Some(1));
    assert_eq!(crate::aparencia::definicao_do_patch(&[5, 6], 7), None);
    assert_eq!(crate::aparencia::definicao_do_patch(&[0], 9), Some(0));
}

#[test]
fn process_string_prefere_local_marca_faltando_e_opcional() {
    use std::collections::HashMap;
    let h = |s: &str| bwms_primitives::fnv1a64(s.as_bytes());
    let globais: HashMap<u64, String> = [(h("gender"), "w".to_string()), (h("camera"), "fpp".to_string())].into();
    let locais: HashMap<u64, String> = [(h("camera"), "tpp".to_string())].into();
    let r = crate::aparencia::processar_texto_dinamico(&globais, &locais, "*torso_{gender}_{camera}");
    assert_eq!((r.valid, r.missed, r.optional, r.value.as_str()), (true, false, false, "torso_w_tpp"));
    let r = crate::aparencia::processar_texto_dinamico(&globais, &HashMap::new(), "*x_{nada}?");
    assert_eq!((r.valid, r.missed, r.optional, r.value.as_str()), (true, true, true, "x_"));
    assert!(!crate::aparencia::processar_texto_dinamico(&globais, &locais, "sem_atributo").valid);
    assert!(!crate::aparencia::processar_texto_dinamico(&globais, &locais, "*aberto_{gender").valid);
    assert_eq!(super::resolver_nome_dinamico(&globais, &HashMap::new(), "*x_{nada}?"), "");
    assert_eq!(super::resolver_nome_dinamico(&globais, &HashMap::new(), "*x_{nada}"), "x_");
    assert_eq!(super::resolver_nome_dinamico(&globais, &HashMap::new(), "plano"), "plano");
}

#[test]
fn condicao_e_snake_case_do_estado() {
    let semente = bwms_primitives::fnv1a64_seeded(b"=", bwms_primitives::fnv1a64(b"gender"));
    assert_eq!(super::condicao_do_estado("gender", "w"), bwms_primitives::fnv1a64_seeded(b"w", semente));
    assert_eq!(crate::texto_e_numeros::para_snake_case("FPP"), "fpp");
    assert_eq!(crate::texto_e_numeros::para_snake_case("BaseBody"), "base_body");
    assert_eq!(crate::texto_e_numeros::para_snake_case("Part"), "part");
    // Os casos que um `to_snake_case` comum erra (`Str.hpp:25`).
    assert_eq!(crate::texto_e_numeros::para_snake_case("foo_Bar"), "foo_bar", "o `_` não abre divisão: nada de `__`");
    assert_eq!(crate::texto_e_numeros::para_snake_case("v2Model"), "v2_model", "dígito abre divisão como minúscula");
    assert_eq!(crate::texto_e_numeros::para_snake_case("çãoFoo"), "ção_foo", "não-ASCII passa intacto");
    assert_eq!(crate::texto_e_numeros::para_snake_case("ãB"), "ã_b", "não-ASCII abre divisão, como o `split = (c != '_')` da referência");
    assert_eq!(crate::texto_e_numeros::para_snake_case(""), "");
}

#[test]
fn espera_sem_ciclo_tira_patch_de_si_mesmo_e_ciclos_e_mantem_cadeia() {
    // Cadeia a ← b ← c e losango d ← (e, f) ← g ficam; patch de si mesmo e os ciclos x ↔ y e p → q → r → p saem inteiros.
    let pares = [(1, 2), (2, 3), (4, 5), (4, 6), (5, 7), (6, 7), (9, 9), (10, 11), (11, 10), (20, 21), (21, 22), (22, 20), (1, 2)];
    let v = crate::arquivos_do_jogo::esperas_sem_ciclo(&pares);
    assert_eq!(v, vec![(1, 2), (2, 3), (4, 5), (4, 6), (5, 7), (6, 7)]);
}

#[test]
fn controle_em_curso_confere_no_proprio_template_em_vez_de_recusar() {
    // Builder concorrente com patch, lendo o controle enquanto o 1º builder ainda extrai: recusar monta a entidade sem o patch até a próxima carga.
    use crate::entidades::ControleDaExtracao;
    assert!(super::mescla_no_builder_segue(ControleDaExtracao::EmCurso, || true));
    assert!(!super::mescla_no_builder_segue(ControleDaExtracao::EmCurso, || false));
}

#[test]
fn controle_decidido_nao_confere_de_novo_e_so_ok_mescla() {
    use crate::entidades::ControleDaExtracao;
    let sem_conferir = || -> bool { panic!("conferiu com o controle já decidido") };
    assert!(super::mescla_no_builder_segue(ControleDaExtracao::Ok, sem_conferir));
    assert!(!super::mescla_no_builder_segue(ControleDaExtracao::Falhou, sem_conferir));
    assert!(!super::mescla_no_builder_segue(ControleDaExtracao::Pendente, sem_conferir));
}

#[test]
fn controle_volta_do_byte_e_byte_fora_dos_estados_e_falhou() {
    use crate::entidades::ControleDaExtracao;
    for c in [ControleDaExtracao::Pendente, ControleDaExtracao::EmCurso, ControleDaExtracao::Ok, ControleDaExtracao::Falhou] {
        assert_eq!(ControleDaExtracao::do_byte(c as u8), c);
    }
    assert_eq!(ControleDaExtracao::do_byte(9), ControleDaExtracao::Falhou);
}
