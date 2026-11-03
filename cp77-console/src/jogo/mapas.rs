//! `HashMap<u64, V>` do motor, a camada comum de quem precisa achar, inserir ou crescer num mapa vivo do jogo (textos da localização; os
//! devices do A#51h usam a mesma forma com nó de 0x50).
//!
//! Cabeçalho: tabela de baldes `+0x00`, tamanho `+0x08`, capacidade `+0x0c`, nós `+0x10`, capacidade dos nós `+0x18`, passo do nó `+0x1c`,
//! próximo nó livre `+0x20`, nós usados `+0x24`, alocador inline `+0x28` (vtable: alocar `+0x08`, liberar `+0x20`). Nó: `next` u32 `+0x00`,
//! hash u32 `+0x04`, chave `+0x08`, valor `+0x10`. Hash = FNV-1a de 32 bits dos 8 bytes da chave; balde = hash % capacidade (o mesmo cálculo
//! que o `GetText` da localização faz inline em `0x101c32f4c`).

use core::ffi::c_void;

unsafe fn rd32(p: *const u8) -> u32 {
    (p as *const u32).read_unaligned()
}
unsafe fn rd64(p: *const u8) -> u64 {
    (p as *const u64).read_unaligned()
}

/// Cabeçalho com cara de mapa do motor: tabela de baldes alinhada a 4, nós e passo alinhados a 8. Um cabeçalho lido de objeto que já não é
/// o dono do mapa (o `GameInstance` do player do menu depois do load, crash SIGBUS de 2026-10-25, `proofs/20261025-crash-sigbus-executor/`)
/// passou no `is_readable` com nós em `0x300000001` e passo `0x79`; o alinhamento o recusa sem ida ao kernel.
fn alinhado(tabela: usize, nos: usize, passo: usize) -> bool {
    tabela % 4 == 0 && nos % 8 == 0 && passo % 8 == 0
}

/// `(tabela, capacidade, nós, passo, capacidade dos nós)`, conferidos legíveis; `None` com mapa vazio ou estranho.
pub(crate) unsafe fn partes(mapa: *const u8) -> Option<(*mut u32, usize, *mut u8, usize, usize)> {
    let tabela = rd64(mapa) as *mut u32;
    let cap = rd32(mapa.add(0x0c)) as usize;
    let nos = rd64(mapa.add(0x10)) as *mut u8;
    let cap_nos = rd32(mapa.add(0x18)) as usize;
    let passo = rd32(mapa.add(0x1c)) as usize;
    let ok = cap > 0 && cap <= 0x40_0000 && !tabela.is_null() && !nos.is_null() && (0x18..=0x100).contains(&passo) && cap_nos >= cap
        && alinhado(tabela as usize, nos as usize, passo)
        && crate::memoria::is_readable(tabela as *const c_void, cap * 4) && crate::memoria::is_readable(nos as *const c_void, cap_nos * passo);
    ok.then_some((tabela, cap, nos, passo, cap_nos))
}

/// Todos os nós do mapa, andando o índice e as cadeias como o motor — sem hash da chave, então serve pra qualquer tipo de chave (o
/// `defaultValues` do TweakDB é por `CName`). `None` com o mapa ilegível; vazio com capacidade 0.
pub(crate) unsafe fn nos(mapa: *const u8) -> Option<Vec<*mut u8>> {
    let (indice, cap) = (rd64(mapa) as *const u32, rd32(mapa.add(0x0c)) as usize);
    let (nos, passo) = (rd64(mapa.add(0x10)) as *mut u8, rd32(mapa.add(0x1c)) as usize);
    if cap == 0 {
        return Some(Vec::new());
    }
    if indice.is_null()
        || nos.is_null()
        || cap > 0x40_0000
        || !(0x18..=0x100).contains(&passo)
        || !alinhado(indice as usize, nos as usize, passo)
        || !crate::memoria::is_readable(indice as *const c_void, cap * 4)
    {
        return None;
    }
    let mut todos = Vec::new();
    for b in 0..cap {
        let mut i = indice.add(b).read_unaligned();
        let mut passos = 0;
        while i != u32::MAX && passos < 4096 {
            let no = nos.add(i as usize * passo);
            if !crate::memoria::is_readable(no as *const c_void, 0x18) {
                return None;
            }
            todos.push(no);
            i = rd32(no);
            passos += 1;
        }
    }
    Some(todos)
}

/// Tira do mapa o 1º nó que `achou` aceita, como o `Remove` do motor: desliga da cadeia do balde, põe na frente da lista de nós livres (o
/// `GetNextAvailNode` pega de volta) e desconta do tamanho. Devolve o nó com o valor ainda lá, pra quem remove soltar o que ele segura.
pub(crate) unsafe fn desligar(mapa: *mut u8, achou: impl Fn(*mut u8) -> bool) -> Option<*mut u8> {
    let (indice, cap) = (rd64(mapa) as *mut u32, rd32(mapa.add(0x0c)) as usize);
    let (nos, passo) = (rd64(mapa.add(0x10)) as *mut u8, rd32(mapa.add(0x1c)) as usize);
    if cap == 0 || indice.is_null() || nos.is_null() || !(0x18..=0x100).contains(&passo) || !crate::memoria::is_readable(indice as *const c_void, cap * 4) {
        return None;
    }
    for b in 0..cap {
        let mut anterior = indice.add(b);
        let mut i = anterior.read_unaligned();
        let mut passos = 0;
        while i != u32::MAX && passos < 4096 {
            let no = nos.add(i as usize * passo);
            if !crate::memoria::is_readable(no as *const c_void, 0x18) {
                return None;
            }
            if achou(no) {
                anterior.write_unaligned(rd32(no));
                (no as *mut u32).write_unaligned(rd32(mapa.add(0x20)));
                (mapa.add(0x20) as *mut u32).write_unaligned(i);
                (mapa.add(0x08) as *mut u32).write_unaligned(rd32(mapa.add(0x08)).saturating_sub(1));
                return Some(no);
            }
            anterior = no as *mut u32;
            i = rd32(no);
            passos += 1;
        }
    }
    None
}

/// Hash do nó e do balde: FNV-1a 32 dos 8 bytes LE da chave (u64 ou ponteiro, como a chave `IType*` do `recordsByType` do TweakDB).
pub(crate) fn hash_da_chave(chave: u64) -> u32 {
    bwms_primitives::fnv1a32(&chave.to_le_bytes())
}

/// Nó da chave, ou `None`.
pub(crate) unsafe fn no(mapa: *const u8, chave: u64) -> Option<*mut u8> {
    let (tabela, cap, nos, passo, cap_nos) = partes(mapa)?;
    let h = hash_da_chave(chave);
    let mut idx = tabela.add(h as usize % cap).read_unaligned();
    for _ in 0..4096 {
        if idx == u32::MAX {
            return None;
        }
        // Índice fora do bloco de nós = mapa corrompido ou lido fora de hora: `partes` só confere os primeiros 512 B do bloco.
        if idx as usize >= cap_nos {
            return None;
        }
        let n = nos.add(idx as usize * passo);
        if rd32(n.add(4)) == h && rd64(n.add(8)) == chave {
            return Some(n);
        }
        idx = rd32(n);
    }
    None
}

/// Capacidade que cabe `novas` chaves a mais: a regra do `Emplace` (tamanho + metade, mínimo 4), crescendo até caber.
pub(crate) fn capacidade_para(tam: usize, cap: usize, novas: usize) -> usize {
    let precisa = tam + novas;
    if precisa <= cap {
        return cap;
    }
    let mut nova = (tam + tam / 2).max(4);
    while nova < precisa {
        nova += (nova / 2).max(1);
    }
    nova
}

#[repr(C)]
struct Bloco {
    memoria: u64,
    tamanho: u64,
}

/// `Reserve` pelo alocador inline do próprio mapa: bloco único `nós + tabela` (a forma do motor), nós rehasheados em ordem, bloco antigo
/// liberado pelo mesmo alocador. Só troca o mapa depois de mover todos os nós; com erro, não mexe nele.
pub(crate) unsafe fn reservar(mapa: *mut u8, nova_cap: usize) -> Result<(), String> {
    let vt = rd64(mapa.add(0x28));
    if !crate::memoria::is_readable(vt as *const c_void, 0x28) {
        return Err(format!("alocador ilegível ({vt:#x})"));
    }
    let (f_alocar, f_liberar) = (rd64((vt + 0x08) as *const u8), rd64((vt + 0x20) as *const u8));
    if !crate::rtti::sane(f_alocar as *mut c_void) || !crate::rtti::sane(f_liberar as *mut c_void) {
        return Err(format!("alocador {vt:#x} sem funções"));
    }
    let tam = rd32(mapa.add(0x08)) as usize;
    let cap = rd32(mapa.add(0x0c)) as usize;
    let passo = rd32(mapa.add(0x1c)) as usize;
    let (nos, tabela, cap_nos) = (rd64(mapa.add(0x10)), rd64(mapa), rd32(mapa.add(0x18)) as usize);
    if nova_cap <= cap || nova_cap > 0x40_0000 || !(0x18..=0x100).contains(&passo) {
        return Err(format!("capacidade {cap} -> {nova_cap} ou passo {passo:#x} fora da faixa"));
    }
    // O bloco antigo precisa ter a forma do `Reserve` (tabela logo depois dos nós): só assim liberar o início dos nós libera o bloco inteiro.
    if cap > 0 && (nos == 0 || tabela != nos + (cap_nos * passo) as u64 || partes(mapa).is_none()) {
        return Err("bloco antigo sem a forma do Reserve".into());
    }
    let bytes = nova_cap * (passo + 4);
    let alocar: unsafe extern "C" fn(*const u8, u32, u32) -> Bloco = std::mem::transmute(f_alocar);
    let liberar: unsafe extern "C" fn(*const u8, *const [u64; 2]) = std::mem::transmute(f_liberar);
    let bloco = alocar(mapa.add(0x28), bytes as u32, 8);
    if bloco.memoria == 0 || (bloco.tamanho as usize) < bytes {
        if bloco.memoria != 0 {
            liberar(mapa.add(0x28), &[bloco.memoria, 0]);
        }
        return Err("alocador não deu o bloco".into());
    }
    std::ptr::write_bytes(bloco.memoria as *mut u8, 0, bytes);
    let novos = bloco.memoria as *mut u8;
    let nova_tabela = novos.add(nova_cap * passo) as *mut u32;
    for i in 0..nova_cap {
        nova_tabela.add(i).write_unaligned(u32::MAX);
    }
    let mut k = 0usize;
    for b in 0..cap {
        let mut idx = (tabela as *const u32).add(b).read_unaligned();
        while idx != u32::MAX && k < nova_cap && (idx as usize) < cap_nos {
            let velho = (nos as *const u8).add(idx as usize * passo);
            let novo = novos.add(k * passo);
            std::ptr::copy_nonoverlapping(velho, novo, passo);
            let balde = nova_tabela.add(rd32(velho.add(4)) as usize % nova_cap);
            (novo as *mut u32).write_unaligned(balde.read_unaligned());
            balde.write_unaligned(k as u32);
            idx = rd32(velho);
            k += 1;
        }
    }
    if k != tam {
        liberar(mapa.add(0x28), &[bloco.memoria, 0]);
        return Err(format!("moveu {k} de {tam} nós"));
    }
    if cap > 0 {
        liberar(mapa.add(0x28), &[nos, 0]);
    }
    (mapa as *mut u64).write_unaligned(nova_tabela as u64);
    (mapa.add(0x0c) as *mut u32).write_unaligned(nova_cap as u32);
    (mapa.add(0x10) as *mut u64).write_unaligned(bloco.memoria);
    (mapa.add(0x18) as *mut u32).write_unaligned(nova_cap as u32);
    // `k` nós tirados em sequência de uma lista nova (`k < nova_cap`): próximo livre e usados = `k`, como o `GetNextAvailNode`.
    (mapa.add(0x20) as *mut u32).write_unaligned(k as u32);
    (mapa.add(0x24) as *mut u32).write_unaligned(k as u32);
    Ok(())
}

/// `GetNextAvailNode`: `(índice, próximo livre, usados)` depois de tirar um nó, ou `None` sem nó livre.
pub(crate) fn proximo_livre(proximo: u32, usados: u32, capacidade: u32, next_do_no: impl Fn(u32) -> u32) -> Option<(u32, u32, u32)> {
    if proximo == u32::MAX {
        return None;
    }
    if usados == proximo {
        Some(if usados + 1 < capacidade { (proximo, proximo + 1, usados + 1) } else { (proximo, u32::MAX, usados) })
    } else {
        Some((proximo, next_do_no(proximo), usados))
    }
}

/// Insere `chave` (que o mapa não tem): cresce pelo `reservar` se não couber, tira um nó livre, zera, grava hash e chave, deixa
/// `inicia(valor)` montar o valor no nó (`+0x10`) e liga o nó no balde. Devolve o nó.
pub(crate) unsafe fn inserir(mapa: *mut u8, chave: u64, inicia: impl FnOnce(*mut u8)) -> Result<*mut u8, String> {
    let tam = rd32(mapa.add(0x08)) as usize;
    let cap = rd32(mapa.add(0x0c)) as usize;
    if tam + 1 > cap {
        reservar(mapa, capacidade_para(tam, cap, 1))?;
    }
    let (tabela, cap, nos, passo, cap_nos) = partes(mapa).ok_or("mapa ilegível")?;
    let (prox, usados) = (rd32(mapa.add(0x20)), rd32(mapa.add(0x24)));
    if prox != u32::MAX && prox as usize >= cap_nos {
        return Err(format!("próximo livre {prox} fora dos {cap_nos} nós"));
    }
    let (idx, novo_prox, novos_usados) =
        proximo_livre(prox, usados, cap_nos as u32, |i| rd32(nos.add(i as usize * passo))).ok_or("sem nó livre")?;
    let n = nos.add(idx as usize * passo);
    let h = hash_da_chave(chave);
    std::ptr::write_bytes(n, 0, passo);
    (n.add(4) as *mut u32).write_unaligned(h);
    (n.add(8) as *mut u64).write_unaligned(chave);
    inicia(n.add(0x10));
    let balde = tabela.add(h as usize % cap);
    (n as *mut u32).write_unaligned(balde.read_unaligned());
    balde.write_unaligned(idx);
    (mapa.add(0x20) as *mut u32).write_unaligned(novo_prox);
    (mapa.add(0x24) as *mut u32).write_unaligned(novos_usados);
    (mapa.add(0x08) as *mut u32).write_unaligned((tam + 1) as u32);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacidade_segue_o_emplace() {
        assert_eq!(capacidade_para(10, 20, 5), 20);
        assert_eq!(capacidade_para(10, 10, 1), 15);
        assert_eq!(capacidade_para(0, 0, 1), 4);
        assert_eq!(capacidade_para(38114, 38114, 1), 57171);
        assert_eq!(capacidade_para(4, 4, 10), 19);
    }

    #[test]
    fn hash_da_chave_e_fnv1a32_dos_bytes_le() {
        // Vetor de referência: FNV-1a 32 de 8 bytes zero.
        assert_eq!(hash_da_chave(0), 0x9be1_7165);
        // A chave 0 não distingue a ordem dos bytes; esta prende o LE: o motor faz o hash dos 8 bytes da chave como estão na memória (arm64).
        assert_eq!(hash_da_chave(0x0807_0605_0403_0201), bwms_primitives::fnv1a32(&[1, 2, 3, 4, 5, 6, 7, 8]));
        assert_ne!(hash_da_chave(0x0807_0605_0403_0201), bwms_primitives::fnv1a32(&[8, 7, 6, 5, 4, 3, 2, 1]));
    }

    #[test]
    fn proximo_livre_sequencial_e_por_lista() {
        assert_eq!(proximo_livre(3, 3, 10, |_| 0), Some((3, 4, 4)));
        assert_eq!(proximo_livre(9, 9, 10, |_| 0), Some((9, u32::MAX, 9)));
        assert_eq!(proximo_livre(2, 5, 10, |_| 7), Some((2, 7, 5)));
        assert_eq!(proximo_livre(u32::MAX, 5, 10, |_| 7), None);
        // Com nó livre na sequência ou no fim, a lista de livres não pode ser lida.
        let sem_lista = |_: u32| panic!("não devia ler a lista de livres");
        assert_eq!(proximo_livre(0, 0, 4, sem_lista), Some((0, 1, 1)));
        assert_eq!(proximo_livre(3, 3, 4, sem_lista), Some((3, u32::MAX, 3)));
        assert_eq!(proximo_livre(u32::MAX, 4, 4, sem_lista), None);
        assert_eq!(proximo_livre(1, 3, 4, |i| if i == 1 { 2 } else { 9 }), Some((1, 2, 3)));
    }
}
