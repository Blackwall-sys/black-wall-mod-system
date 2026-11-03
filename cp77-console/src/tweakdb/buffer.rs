//! Crescimento do buffer de valores do TweakDB (`+0x148`).

/// `red::memory::PoolStorageProxy<game::PoolGMPL_TDB>::AllocateAligned(u64 tamanho, u32 alinhamento) -> Block { ptr, tamanho }` (Steam 2.5.0).
const TDB_ALOCAR_ALINHADO_VM: u64 = 0x1_0375_fd74;
/// Cópia do ponteiro do buffer que o acessor de campo dos records lê.
const BUFFER_ESTATICO_OFF: usize = 0x00;
/// Mapa do valor padrão por tipo: `HashMap<CName, ponteiro absoluto do valor>` (índice +0x00, size +0x08, cap +0x0c, nós +0x10, passo +0x1c;
/// valor no nó +0x10).
pub(crate) const VALORES_PADRAO_OFF: usize = 0x108;
/// O `tdbOffset` do flat tem 24 bits.
pub(crate) const CAPACIDADE_MAXIMA: u32 = 0x00FF_FFFF;

/// O bloco que o `AllocateAligned` do pool devolve em `x0`/`x1`.
#[repr(C)]
struct Bloco {
    ponteiro: u64,
    tamanho: u64,
}

/// Cresce o buffer até a capacidade máxima. `Ok((antigo, novo, capacidade nova, padrões re-baseados))`.
pub(crate) unsafe fn crescer() -> Result<(u64, u64, u32, usize), String> {
    // A mesma vez da criação de valor: um valor criado no meio da cópia ficaria só no bloco antigo.
    let _vez = super::FLAT_POOL.lock().unwrap_or_else(|e| e.into_inner());
    crescer_com_a_vez_tomada()
}

/// [`crescer`] para quem já tem a vez de escrever no buffer (`super::FLAT_POOL`).
pub(super) unsafe fn crescer_com_a_vez_tomada() -> Result<(u64, u64, u32, usize), String> {
    let t = super::singleton().ok_or("sem TweakDB vivo")?;
    let ler = |off: usize| (t.add(off) as *const u64).read_unaligned();
    let antigo = ler(super::FLAT_DATA_BUFFER_OFF);
    let cap = (t.add(super::FLAT_DATA_CAP_OFF) as *const u32).read_unaligned();
    if antigo == 0 {
        return Err("buffer de valores nulo".into());
    }
    if cap >= CAPACIDADE_MAXIMA {
        return Err(format!("capacidade {cap:#x} já é a máxima"));
    }
    // O acessor de campo dos records lê o buffer pela cópia em +0x00: divergindo do +0x148, o layout não é o medido.
    let estatico = ler(BUFFER_ESTATICO_OFF);
    if estatico != antigo {
        return Err(format!("TweakDB+0x00 {estatico:#x} ≠ buffer {antigo:#x}"));
    }
    let mapa = t.add(VALORES_PADRAO_OFF);
    let padroes = crate::jogo::mapas::nos(mapa).ok_or("mapa de valores padrão ilegível")?;
    let dentro = |v: u64| v >= antigo && v < antigo + cap as u64;
    if let Some(fora) = padroes.iter().map(|n| (n.add(0x10) as *const u64).read_unaligned()).find(|v| !dentro(*v)) {
        return Err(format!("valor padrão {fora:#x} fora do buffer"));
    }
    let alocar_addr = crate::rebase(TDB_ALOCAR_ALINHADO_VM);
    if alocar_addr.is_null() || !crate::memoria::is_readable(alocar_addr, 4) {
        return Err("sem o AllocateAligned do pool do TweakDB".into());
    }
    let alocar: extern "C" fn(u64, u32) -> Bloco = std::mem::transmute(alocar_addr);
    let bloco = alocar(CAPACIDADE_MAXIMA as u64, 8);
    if bloco.ponteiro == 0 || bloco.tamanho < CAPACIDADE_MAXIMA as u64 {
        return Err(format!("o pool do TweakDB devolveu {:#x} com {:#x} B", bloco.ponteiro, bloco.tamanho));
    }
    let novo = bloco.ponteiro;
    let cap_nova = bloco.tamanho.min(CAPACIDADE_MAXIMA as u64) as u32;
    if !super::mutex00_lock(t) {
        // O bloco fica sem dono: o pool não tem soltura conhecida por nós, e o crescimento é raro.
        return Err("mutex00 ocupada; bloco novo abandonado".into());
    }
    // Sob a trava, de novo: um valor pode ter entrado entre a checagem e a trava.
    let (antigo_trava, fim) = (ler(super::FLAT_DATA_BUFFER_OFF), ler(super::FLAT_DATA_END_OFF));
    if antigo_trava != antigo || fim < antigo || fim - antigo > cap_nova as u64 {
        super::mutex00_unlock(t);
        return Err(format!("buffer mudou sob a trava ({antigo_trava:#x}, fim {fim:#x})"));
    }
    let usado = fim - antigo;
    std::ptr::copy_nonoverlapping(antigo as *const u8, novo as *mut u8, usado as usize);
    let mut rebaseados = 0usize;
    if let Some(nos) = crate::jogo::mapas::nos(mapa) {
        for no in nos {
            let valor = no.add(0x10) as *mut u64;
            let v = valor.read_unaligned();
            if dentro(v) {
                valor.write_unaligned(novo + (v - antigo));
                rebaseados += 1;
            }
        }
    }
    // Offsets dos flats valem nos dois blocos; o antigo nunca é solto (leitor no meio de um quadro pode estar com ponteiro dele).
    (t.add(super::FLAT_DATA_END_OFF) as *mut u64).write_unaligned(novo + usado);
    (t.add(super::FLAT_DATA_CAP_OFF) as *mut u32).write_unaligned(cap_nova);
    (t.add(super::FLAT_DATA_BUFFER_OFF) as *mut u64).write_unaligned(novo);
    (t.add(BUFFER_ESTATICO_OFF) as *mut u64).write_unaligned(novo);
    super::mutex00_unlock(t);
    crate::log_dev!(
        "[tweakdb] buffer de valores: {antigo:#x} ({cap:#x} B) -> {novo:#x} ({cap_nova:#x} B), {usado:#x} B copiados, {rebaseados} padrão(ões) re-baseado(s)"
    );
    Ok((antigo, novo, cap_nova, rebaseados))
}
