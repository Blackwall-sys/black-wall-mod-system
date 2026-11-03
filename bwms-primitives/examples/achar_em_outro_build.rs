//! Onde um endereço de código do Steam 2.5.0 está noutro build, quando a assinatura da tabela não achou: a janela de palavras mascaradas
//! (`enderecos::mascarada`, o mesmo critério do resolvedor) que começa no alvo, procurada no `__text` do outro binário, com tamanhos
//! crescentes até ficar única.
//!
//!     cargo run --release --example achar_em_outro_build -- <binário steam> <binário outro> <vmaddr steam>...

use bwms_primitives::enderecos as en;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        return eprintln!("uso: achar_em_outro_build <steam> <outro> <vmaddr>...");
    }
    let (steam, outro) = (std::fs::read(&a[0]).expect("steam"), std::fs::read(&a[1]).expect("outro"));
    let (s, o) = (en::Imagem::do_arquivo(&steam).expect("steam Mach-O"), en::Imagem::do_arquivo(&outro).expect("outro Mach-O"));
    let n = o.texto.len() / 4;
    let mo: Vec<u32> = (0..n).map(|i| en::mascarada(o.texto, i)).collect();
    for alvo in &a[2..] {
        let vm = u64::from_str_radix(alvo.trim_start_matches("0x"), 16).expect("vmaddr hex");
        let i = ((vm - s.texto_vm) / 4) as usize;
        let mut achou = false;
        for tamanho in [8usize, 16, 32, 64, 128] {
            let janela: Vec<u32> = (i..i + tamanho).map(|j| en::mascarada(s.texto, j)).collect();
            let lugares: Vec<usize> = (0..n.saturating_sub(tamanho)).filter(|&p| mo[p] == janela[0] && mo[p..p + tamanho] == janela[..]).collect();
            println!("{alvo}: {tamanho} palavras → {} lugar(es){}", lugares.len(), if lugares.len() <= 4 {
                format!(" {:x?}", lugares.iter().map(|p| o.texto_vm + *p as u64 * 4).collect::<Vec<_>>())
            } else {
                String::new()
            });
            if lugares.len() <= 1 {
                achou = lugares.len() == 1;
                break;
            }
        }
        if !achou {
            println!("{alvo}: sem lugar único; pelos chamadores: {:x?}", pelos_chamadores(&s, &o, &mo, vm));
        }
    }
}

/// Corpo que se repete (instância de template): cada `bl` do Steam para o alvo tem a vizinhança (12 palavras de cada lado) procurada no outro
/// build; onde ela é única, o `bl` no mesmo ponto dá o endereço lá. Devolve os destinos achados, um por chamador.
fn pelos_chamadores(s: &en::Imagem, o: &en::Imagem, mo: &[u32], vm: u64) -> Vec<u64> {
    let destino = |img: &en::Imagem, i: usize| -> Option<u64> {
        let w = en::palavra(img.texto, i);
        if w & 0xFC00_0000 != 0x9400_0000 {
            return None;
        }
        let imm = ((w & 0x03FF_FFFF) << 6) as i32 >> 6;
        Some((img.texto_vm as i64 + i as i64 * 4 + imm as i64 * 4) as u64)
    };
    let n = s.texto.len() / 4;
    let mut achados = Vec::new();
    for site in (12..n.saturating_sub(12)).filter(|&i| destino(s, i) == Some(vm)) {
        let janela: Vec<u32> = (site - 12..site + 12).map(|j| en::mascarada(s.texto, j)).collect();
        let lugares: Vec<usize> = (0..mo.len().saturating_sub(24)).filter(|&p| mo[p] == janela[0] && mo[p..p + 24] == janela[..]).collect();
        if let [p] = lugares[..] {
            if let Some(d) = destino(o, p + 12) {
                achados.push(d);
            }
        }
    }
    achados
}
