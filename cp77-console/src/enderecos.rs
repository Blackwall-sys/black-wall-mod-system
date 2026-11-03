//! Os endereços do jogo em qualquer build (RED4ext `#447`, sobreviver a patch do jogo). Os endereços do código são do Steam 2.5.0; a
//! tabela `dados/enderecos-steam-2.5.0*.bin` (gerada pelo example `gerar_enderecos` do bwms-primitives a partir do binário Steam e dos endereços citados no fonte)
//! guarda, pra cada um, o símbolo ou a assinatura das instruções em volta. Num build que não é o Steam (o GOG, ou um patch), o `rebase`
//! traduz pelo resultado de uma passada no `__text` da própria imagem na memória: símbolo pelo `LC_SYMTAB`, código pela assinatura, dado
//! por um lugar do código que o referencia. Endereço sem tradução volta `None` e quem pediu não instala (nunca o endereço Steam cru).
//! O gate do `on_load` (`build_supported`) só liga ganchos no Steam 2.5.0: a tradução é exercida pelo teste `enderecos-teste` do dev.

use std::collections::HashMap;
use std::sync::OnceLock;

// O dev embute a tabela de todo o `src/`; o público, a dos arquivos que o build sem feature compila (o gerador segue o `#[cfg]` de cada
// `mod`). Endereço que só código de dev cita não vai ao dylib do usuário.
#[cfg(feature = "devtools")]
static TABELA: &[u8] = include_bytes!("../dados/enderecos-steam-2.5.0.bin");
#[cfg(not(feature = "devtools"))]
static TABELA: &[u8] = include_bytes!("../dados/enderecos-steam-2.5.0-publico.bin");
/// O mapa resolvido. **Não é `OnceLock`**: a primeira leitura pode pegar a imagem do jogo ainda
/// incompleta. Medido em 2026-09-20, no mesmo binário e na mesma sessão: cedo no boot a passada dá
/// `1 de 806 (1 símbolo, 208 stubs)`; mais tarde dá `806 de 806 (67928 símbolos, 1826 stubs)`. Com
/// um `OnceLock` o primeiro resultado congelava e o mod ficava com UM endereço traduzido pelo resto
/// da sessão — e num build que não é o de origem isso é a diferença entre funcionar e não instalar
/// gancho nenhum. Agora uma passada ruim não é guardada: tenta de novo na próxima chamada.
static MAPA: std::sync::Mutex<Option<HashMap<u64, u64>>> = std::sync::Mutex::new(None);
/// Abaixo disto a passada é considerada incompleta (imagem ainda não mapeada), e não vira cache.
const MINIMO_ACEITAVEL: usize = 90;

/// A imagem do jogo na memória no formato do `bwms_primitives::enderecos`: cabeçalho, `__text` e símbolos lidos de onde o dyld os pôs.
unsafe fn imagem_na_memoria() -> Option<bwms_primitives::enderecos::Imagem<'static>> {
    let base = crate::game_base() as *const u8;
    if base.is_null() || !crate::memoria::is_readable(base as *const core::ffi::c_void, 0x20) {
        return None;
    }
    let cmds = (base.add(0x14) as *const u32).read_unaligned() as usize;
    if !crate::memoria::is_readable(base as *const core::ffi::c_void, 0x20 + cmds) {
        return None;
    }
    let cabecalho = std::slice::from_raw_parts(base, 0x20 + cmds);
    let na_memoria = |vm: u64, tamanho: usize| -> Option<&'static [u8]> {
        let p = base.add((vm - bwms_primitives::enderecos::LINK_BASE) as usize);
        crate::memoria::is_readable(p as *const core::ffi::c_void, tamanho).then(|| std::slice::from_raw_parts(p, tamanho))
    };
    let secoes = bwms_primitives::enderecos::secoes(cabecalho)?;
    let t = secoes.iter().find(|s| s.segmento == "__TEXT" && s.nome == "__text")?.clone();
    let texto = na_memoria(t.vm, t.tamanho as usize)?;
    // Símbolos e tabela indireta (dos stubs de import) estão no __LINKEDIT, cujos offsets no arquivo o LC_SYMTAB e o LC_DYSYMTAB dão:
    // endereço = vm do segmento + (offset - offset do segmento).
    let (simbolos, stubs) = (|| {
        let le = bwms_primitives::enderecos::segmentos(cabecalho)?.into_iter().find(|s| s.nome == "__LINKEDIT")?;
        let (so, n, st, sn) = bwms_primitives::enderecos::tabela_de_simbolos(cabecalho)?;
        let nlist = na_memoria(le.vm + (so - le.arquivo), n * 16)?;
        let strings = na_memoria(le.vm + (st - le.arquivo), sn)?;
        let stubs = bwms_primitives::enderecos::tabela_indireta(cabecalho)
            .and_then(|(io, ni)| na_memoria(le.vm + (io - le.arquivo), ni * 4))
            .map(|ind| bwms_primitives::enderecos::stubs_de(&secoes, ind, nlist, n, strings))
            .unwrap_or_default();
        Some((bwms_primitives::enderecos::simbolos_de(nlist, n, strings), stubs))
    })()
    .unwrap_or_default();
    Some(bwms_primitives::enderecos::Imagem { texto, texto_vm: t.vm, secoes, simbolos, stubs })
}

/// Resolve a tabela inteira contra a imagem na memória (uma vez).
fn mapa() -> HashMap<u64, u64> {
    let mut guarda = match MAPA.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if let Some(m) = guarda.as_ref() {
        return m.clone();
    }
    let resultado = {
        let inicio = std::time::Instant::now();
        let Some(tabela) = bwms_primitives::enderecos::ler(TABELA) else {
            crate::log_dev!("[enderecos] tabela embutida ilegível");
            return HashMap::new();
        };
        let Some(img) = (unsafe { imagem_na_memoria() }) else {
            crate::log_dev!("[enderecos] imagem do jogo ilegível na memória");
            return HashMap::new();
        };
        // DIAGNÓSTICO do "1 de 806" (2026-09-20): offline, contra o binário no disco, esta mesma
        // tabela resolve 806/806 — e 767/806 mesmo sem tabela de símbolos. Se na memória cai pra 1,
        // o que está diferente é a imagem que estamos lendo, não o algoritmo. Estes números dizem
        // qual: a base, onde achamos o `__text`, e se os primeiros bytes dele batem com o que a
        // tabela espera no mesmo endereço.
        #[cfg(feature = "devlog")]
        {
            let base = crate::game_base() as u64;
            let primeiras: Vec<String> = img.texto.iter().take(8).map(|b| format!("{b:02x}")).collect();
            let alvo = tabela.iter().find(|s| !s.palavras.is_empty());
            let esperado = alvo
                .map(|s| s.palavras.iter().take(2).map(|p| format!("{p:08x}")).collect::<Vec<_>>().join(" "))
                .unwrap_or_default();
            let lido = alvo
                .and_then(|s| {
                    let off = s.steam.checked_sub(img.texto_vm)? as usize;
                    img.texto.get(off..off + 8).map(|b| {
                        let w = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
                        format!("{:08x} {:08x}", w(0), w(4))
                    })
                })
                .unwrap_or_else(|| "fora do texto".to_string());
            crate::veredito_dev!(
                "[enderecos-diag] base={base:#x} texto_vm={:#x} texto_len={:#x} primeiros=[{}] alvo={:#x} esperado_na_tabela=[{esperado}] lido_na_memoria=[{lido}]",
                img.texto_vm,
                img.texto.len(),
                primeiras.join(" "),
                alvo.map(|s| s.steam).unwrap_or(0)
            );
        }
        let m: HashMap<u64, u64> = bwms_primitives::enderecos::resolver(&img, &tabela).into_iter().collect();
        crate::veredito_dev!(
            "[enderecos] {} de {} endereços da tabela resolvidos na imagem da memória ({} símbolos, {} stubs) em {} ms",
            m.len(),
            tabela.len(),
            img.simbolos.len(),
            img.stubs.len(),
            inicio.elapsed().as_millis()
        );
        (m, tabela.len())
    };
    let (m, total) = resultado;
    // Passada incompleta não vira cache: a imagem do jogo ainda estava sendo mapeada, e guardar isso
    // deixaria o mod sem tradução pelo resto da sessão.
    if total > 0 && m.len() * 100 >= total * MINIMO_ACEITAVEL {
        *guarda = Some(m.clone());
    } else {
        crate::veredito_dev!(
            "[enderecos] passada incompleta ({} de {}): NÃO virou cache, refaz na próxima chamada",
            m.len(),
            total
        );
    }
    m
}

/// Fim da imagem do jogo em vmaddr (o maior fim de segmento do cabeçalho na memória); sem cabeçalho legível, sem limite.
pub(crate) fn fim_da_imagem() -> u64 {
    static FIM: OnceLock<u64> = OnceLock::new();
    *FIM.get_or_init(|| unsafe {
        let base = crate::game_base() as *const u8;
        if base.is_null() || !crate::memoria::is_readable(base as *const core::ffi::c_void, 0x20) {
            return u64::MAX;
        }
        let cmds = (base.add(0x14) as *const u32).read_unaligned() as usize;
        if !crate::memoria::is_readable(base as *const core::ffi::c_void, 0x20 + cmds) {
            return u64::MAX;
        }
        bwms_primitives::enderecos::segmentos(std::slice::from_raw_parts(base, 0x20 + cmds))
            .and_then(|v| v.iter().filter(|s| s.nome != "__PAGEZERO").map(|s| s.vm + s.tamanho).max())
            .unwrap_or(u64::MAX)
    })
}

// GOG 2.5.0: a mesma versão do jogo compilada à parte, com as funções em outra ordem (os mesmos layouts de struct). A tradução é fixa, medida
// offline pelo `build-core.sh` contra a cópia vanilla do binário GOG, em vez da passada no `__text` no boot (que cedo devolve imagem incompleta).
#[cfg(feature = "devtools")]
static GOG_25: &str = include_str!("../dados/enderecos-gog-2.5.0.txt");
#[cfg(not(feature = "devtools"))]
static GOG_25: &str = include_str!("../dados/enderecos-gog-2.5.0-publico.txt");

fn tabela_gog_25() -> &'static HashMap<u64, u64> {
    static MAPA_GOG: OnceLock<HashMap<u64, u64>> = OnceLock::new();
    MAPA_GOG.get_or_init(|| pares_de_texto(GOG_25))
}

/// `steam gog` por linha, em hexadecimal com `0x`; `#` abre comentário.
fn pares_de_texto(texto: &str) -> HashMap<u64, u64> {
    let hex = |s: &str| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok();
    texto
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let (a, b) = l.split_once(' ')?;
            Some((hex(a)?, hex(b.trim())?))
        })
        .collect()
}

/// O endereço Steam `vm` no GOG 2.5.0, pela tabela fixa.
pub(crate) fn traduzir_gog_25(vm: u64) -> Option<u64> {
    if vm == bwms_primitives::enderecos::LINK_BASE {
        return Some(vm);
    }
    tabela_gog_25().get(&vm).copied()
}

/// O endereço Steam `vm` neste build, pela tabela. A base da imagem (o cabeçalho Mach-O) é a base em qualquer build.
pub(crate) fn traduzir(vm: u64) -> Option<u64> {
    if vm == bwms_primitives::enderecos::LINK_BASE {
        return Some(vm);
    }
    // Caminho comum (mapa já resolvido): consulta sob a trava, sem clonar as ~800 entradas a cada `rebase` de um build não-Steam.
    {
        let guarda = match MAPA.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(m) = guarda.as_ref() {
            return m.get(&vm).copied();
        }
    }
    mapa().get(&vm).copied()
}

/// Teste da rota inteira no Steam (`~/.bwms-hook-enderecos-teste`): o `rebase` passa a traduzir tudo pela tabela, como num build
/// desconhecido. No Steam a tradução tem de dar o próprio endereço; os que não estão na tabela voltam nulo e aparecem no log.
#[cfg(feature = "devtools")]
pub(crate) fn forcar_resolvedor() -> bool {
    static FORCAR: OnceLock<bool> = OnceLock::new();
    *FORCAR.get_or_init(|| crate::marcador_individual("enderecos-teste"))
}

/// No teste: quantos endereços da tabela voltaram a si mesmos (identidade no Steam) e os primeiros que não.
#[cfg(feature = "devtools")]
pub(crate) fn conferir_identidade() {
    let m = mapa();
    let tabela = bwms_primitives::enderecos::ler(TABELA).unwrap_or_default();
    let fora: Vec<String> = m.iter().filter(|(s, r)| s != r).take(8).map(|(s, r)| format!("{s:#x}->{r:#x}")).collect();
    let iguais = m.iter().filter(|(s, r)| s == r).count();
    let sem = crate::REBASE_SEM_MAPA.load(std::sync::atomic::Ordering::Relaxed);
    let ok = iguais == m.len() && sem == 0 && m.len() * 100 >= tabela.len() * 95;
    crate::veredito_dev!(
        "[enderecos] VEREDITO-ENDERECOS com todo rebase pela tabela, no Steam {iguais} de {} resolvidos voltaram ao próprio endereço (tabela com {}), fora do lugar {fora:?}, pedidos sem tradução {sem}; enderecos-ok={ok}",
        m.len(),
        tabela.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pares_de_texto_pula_comentario_e_linha_torta() {
        let m = pares_de_texto("# cabeçalho\n0x10156e184 0x1016d67f4\nlixo\n0x1 \n");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get(&0x1_0156_e184), Some(&0x1_016d_67f4));
    }

    /// As duas tabelas embutidas têm o executor (a detecção do GOG lê o prólogo nele) e os pares manuais de template repetido.
    #[test]
    fn tabelas_gog_25_tem_executor_e_pares_manuais() {
        for texto in [include_str!("../dados/enderecos-gog-2.5.0.txt"), include_str!("../dados/enderecos-gog-2.5.0-publico.txt")] {
            let m = pares_de_texto(texto);
            assert!(m.len() > 600);
            assert!(m.contains_key(&crate::jogo::scripts::EXECUTOR_VM));
            assert_eq!(m.get(&0x1_0156_e184), Some(&0x1_016d_67f4));
            assert_eq!(m.get(&0x1_01c6_e2e4), Some(&0x1_016c_9df0));
        }
    }
}
