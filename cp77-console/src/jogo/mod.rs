//! As peças do jogo que o mod consulta, um nome por endereço. Quem precisa de uma peça pede aqui; nenhum endereço do motor
//! é escrito de novo em outro arquivo.

pub(crate) mod motor;

pub(crate) mod mapas;

pub(crate) mod cstring;

pub(crate) mod anexos;
pub(crate) mod aparencia;
pub(crate) mod buffers;
pub(crate) mod chamada;
pub(crate) mod fila;
pub(crate) mod interface;
pub(crate) mod itens;
pub(crate) mod listas;
pub(crate) mod mundo;
pub(crate) mod objetos;
pub(crate) mod recursos;
pub(crate) mod scripts;
pub(crate) mod sessao;
pub(crate) mod texto;
pub(crate) mod travas;

#[cfg(test)]
mod testes {
    /// Os endereços que o produto pede a `jogo/` aparecem em código só na linha que os define: uma cópia literal em outro arquivo (inclusive no
    /// `selftest_stub.rs`, que entra no build público) derruba o teste. O extrator é o que gera a tabela de assinaturas
    /// (`bwms_primitives::enderecos::enderecos_no_fonte`), então casa `0x1_02aa_9a7c`, `0x102AA9A7C` e qualquer mistura de `_` e maiúscula. Citar o endereço
    /// numa linha só de comentário (`//`) continua livre.
    #[test]
    fn endereco_de_produto_aparece_em_codigo_so_em_jogo() {
        let alvos = [
            (super::aparencia::ENTITY_TEMPLATE_FIND_APP_VM, "jogo/aparencia.rs"),
            (super::mundo::VEHICLESYSTEM_TOGGLEGARAGEVEHICLE_VM, "jogo/mundo.rs"),
            (super::interface::SPAWN_FROM_LOCAL_VM, "jogo/interface.rs"),
            (super::interface::SPAWN_FROM_EXTERNAL_VM, "jogo/interface.rs"),
            (super::interface::ASYNC_SPAWN_FROM_LOCAL_VM, "jogo/interface.rs"),
            (super::interface::ASYNC_SPAWN_FROM_EXTERNAL_VM, "jogo/interface.rs"),
            (super::interface::ENTREGA_DO_SPAWN_ASSINCRONO_VM, "jogo/interface.rs"),
        ];
        let raiz = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut achados: Vec<Vec<String>> = vec![Vec::new(); alvos.len()];
        let mut pilha = vec![raiz.clone()];
        while let Some(pasta) = pilha.pop() {
            for entrada in std::fs::read_dir(&pasta).expect("pasta de src legível") {
                let caminho = entrada.expect("entrada de src legível").path();
                if caminho.is_dir() {
                    pilha.push(caminho);
                    continue;
                }
                if caminho.extension().and_then(|x| x.to_str()) != Some("rs") {
                    continue;
                }
                let relativo = caminho.strip_prefix(&raiz).expect("arquivo dentro de src").to_string_lossy().into_owned();
                let fonte = std::fs::read_to_string(&caminho).unwrap_or_else(|e| panic!("{relativo}: {e}"));
                for (n, linha) in fonte.lines().enumerate() {
                    if linha.trim_start().starts_with("//") {
                        continue;
                    }
                    let citados = bwms_primitives::enderecos::enderecos_no_fonte(linha);
                    for (k, (endereco, _)) in alvos.iter().enumerate() {
                        if citados.contains(endereco) {
                            achados[k].push(format!("{relativo}:{}", n + 1));
                        }
                    }
                }
            }
        }
        for ((endereco, dono), onde) in alvos.iter().zip(&achados) {
            assert!(
                onde.len() == 1 && onde[0].starts_with(&format!("{dono}:")),
                "{endereco:#x} tem de aparecer em código só em {dono}; achado em {onde:?}"
            );
        }
    }
}
