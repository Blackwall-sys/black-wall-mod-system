//! O que o leitor do BWMS tira de arquivos `.xl` do ArchiveXL, sem o jogo: `cargo run --example xl_do_arquivo -- <arquivo.xl>...`.
use bwms_primitives::instalacao::xl::parse_xl;

fn main() {
    for caminho in std::env::args().skip(1) {
        let nome = std::path::Path::new(&caminho).file_name().map_or(caminho.clone(), |n| n.to_string_lossy().into_owned());
        let texto = match std::fs::read_to_string(&caminho) {
            Ok(t) => t,
            Err(e) => {
                println!("{nome}: ERRO de leitura: {e}");
                continue;
            }
        };
        match parse_xl(&texto) {
            Err(e) => println!("{nome}: ERRO: {e}"),
            Ok(xl) => println!(
                "{nome}: factories {} patch {} link {} scope {} copy {} fix {} localization {} garment {} quest {} journal {} anim {} \
                 customization {} streaming {} | não tipadas {:?} | issues {:?} | sem efeito no BWMS {:?}",
                xl.factories.len(),
                xl.patches.len(),
                xl.links.len(),
                xl.scopes.len(),
                xl.copies.len(),
                xl.fixes.len(),
                xl.localization.len(),
                xl.garment_overrides.len(),
                xl.quest_phases.len(),
                xl.journals.len(),
                xl.animations.len(),
                xl.customization.is_some(),
                xl.streaming.is_some(),
                xl.other_sections,
                xl.issues,
                xl.sem_efeito_no_bwms
            ),
        }
    }
}
