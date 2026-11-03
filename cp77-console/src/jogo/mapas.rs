//! `HashMap<u64, V>` do motor, a camada comum de quem precisa achar, inserir ou crescer num mapa vivo do jogo (textos da localização; os
//! devices usam a mesma forma com nó de 0x50).
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

/// Tira do mapa o 1º nó que `achou` aceita (baldes em ordem) e o devolve com chave e valor ainda lá: o nó sai da cadeia do balde e entra na
/// FRENTE da lista de livres (`+0x20`), como o motor faz ao apagar, e o tamanho cai 1. Quem chamou destrói o valor.
pub(crate) unsafe fn desligar(mapa: *mut u8, achou: impl Fn(*mut u8) -> bool) -> Option<*mut u8> {
    if !crate::memoria::is_readable(mapa as *const c_void, 0x28) || rd32(mapa.add(0x08)) == 0 {
        return None;
    }
    let (tabela, cap, nos, passo, cap_nos) = partes(mapa)?;
    for b in 0..cap {
        // `anterior` = onde mora o índice que aponta para o nó atual: o balde ou o `next` do nó de antes.
        let mut anterior = tabela.add(b);
        let mut idx = anterior.read_unaligned();
        let mut passos = 0;
        while idx != u32::MAX && passos < 4096 {
            if idx as usize >= cap_nos {
                return None;
            }
            let n = nos.add(idx as usize * passo);
            if achou(n) {
                anterior.write_unaligned(rd32(n));
                (n as *mut u32).write_unaligned(rd32(mapa.add(0x20)));
                (mapa.add(0x20) as *mut u32).write_unaligned(idx);
                (mapa.add(0x08) as *mut u32).write_unaligned(rd32(mapa.add(0x08)) - 1);
                return Some(n);
            }
            anterior = n as *mut u32;
            idx = rd32(n);
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

/// Capacidade nova para caber `novas` chaves a mais num mapa com `tam` chaves e capacidade `cap`: a mesma se cabe; senão parte de
/// `max(tam × 1,5, 4)` e cresce meia vez (mínimo 1) até caber, o passo de crescimento do próprio motor.
pub(crate) fn capacidade_para(tam: usize, cap: usize, novas: usize) -> usize {
    let precisa = tam + novas;
    if precisa <= cap {
        return cap;
    }
    let mut c = (tam + tam / 2).max(4);
    while c < precisa {
        c += (c / 2).max(1);
    }
    c
}

/// Retorno do alocar do alocador do mapa: `{memória, tamanho}` em `x0`/`x1`.
#[repr(C)]
struct Bloco {
    memoria: u64,
    _tamanho: u64,
}

/// Alocador inline do mapa (`+0x28`): o objeto é a própria palavra da vtable; alocar em `+0x08` (tamanho, alinhamento), liberar em `+0x20`
/// recebe `&{memória, tamanho}`.
type Alocar = unsafe extern "C" fn(*mut u8, u64, u64) -> Bloco;
type Liberar = unsafe extern "C" fn(*mut u8, *const [u64; 2]);

/// Cresce o mapa até `nova_cap` pelo alocador inline do próprio mapa (`+0x28`); com erro, não mexe nele. O bloco novo tem a forma que o
/// motor libera inteiro pelo início: `nova_cap` nós seguidos da tabela de `nova_cap` baldes. Os nós vivos vão para `0..k`, religados pelo
/// hash guardado no nó.
pub(crate) unsafe fn reservar(mapa: *mut u8, nova_cap: usize) -> Result<(), String> {
    if !crate::memoria::is_readable(mapa as *const c_void, 0x30) {
        return Err("mapa ilegível".into());
    }
    let tabela = rd64(mapa) as *mut u32;
    let tam = rd32(mapa.add(0x08)) as usize;
    let cap = rd32(mapa.add(0x0c)) as usize;
    let nos = rd64(mapa.add(0x10)) as *mut u8;
    let cap_nos = rd32(mapa.add(0x18)) as usize;
    let passo = rd32(mapa.add(0x1c)) as usize;
    if nova_cap <= cap || nova_cap > 0x40_0000 {
        return Err(format!("capacidade {nova_cap} recusada (atual {cap})"));
    }
    if !(0x18..=0x100).contains(&passo) || passo % 8 != 0 {
        return Err(format!("passo do nó {passo:#x} fora da faixa"));
    }
    let vt = rd64(mapa.add(0x28));
    if vt == 0 || !crate::memoria::is_readable(vt as *const c_void, 0x28) {
        return Err("alocador do mapa ilegível".into());
    }
    let tem_bloco = !nos.is_null();
    if tem_bloco && (tabela as usize != nos as usize + cap_nos * passo || cap == 0) {
        return Err("bloco antigo não é nós seguidos da tabela".into());
    }
    if tem_bloco && partes(mapa).is_none() {
        return Err("mapa ilegível".into());
    }
    let alocar: Alocar = core::mem::transmute(rd64((vt + 0x08) as *const u8));
    let liberar: Liberar = core::mem::transmute(rd64((vt + 0x20) as *const u8));
    let tamanho = nova_cap * (passo + 4);
    let bloco = alocar(mapa.add(0x28), tamanho as u64, 8).memoria as *mut u8;
    if bloco.is_null() {
        return Err(format!("alocador do mapa não deu {tamanho:#x} B"));
    }
    core::ptr::write_bytes(bloco, 0, tamanho);
    let nova_tabela = bloco.add(nova_cap * passo) as *mut u32;
    for b in 0..nova_cap {
        nova_tabela.add(b).write_unaligned(u32::MAX);
    }
    let mut k = 0usize;
    if tem_bloco {
        'baldes: for b in 0..cap {
            let mut idx = tabela.add(b).read_unaligned();
            let mut passos = 0;
            while idx != u32::MAX && passos < 4096 {
                if idx as usize >= cap_nos || k >= nova_cap {
                    break 'baldes;
                }
                let velho = nos.add(idx as usize * passo);
                let novo = bloco.add(k * passo);
                core::ptr::copy_nonoverlapping(velho, novo, passo);
                let balde = rd32(velho.add(4)) as usize % nova_cap;
                (novo as *mut u32).write_unaligned(nova_tabela.add(balde).read_unaligned());
                nova_tabela.add(balde).write_unaligned(k as u32);
                k += 1;
                idx = rd32(velho);
                passos += 1;
            }
        }
    }
    if k != tam {
        liberar(mapa.add(0x28), &[bloco as u64, tamanho as u64]);
        return Err(format!("{k} nó(s) vivos achados, tamanho diz {tam}"));
    }
    if tem_bloco {
        liberar(mapa.add(0x28), &[nos as u64, (cap_nos * (passo + 4)) as u64]);
    }
    (mapa as *mut u64).write_unaligned(nova_tabela as u64);
    (mapa.add(0x0c) as *mut u32).write_unaligned(nova_cap as u32);
    (mapa.add(0x10) as *mut u64).write_unaligned(bloco as u64);
    (mapa.add(0x18) as *mut u32).write_unaligned(nova_cap as u32);
    (mapa.add(0x20) as *mut u32).write_unaligned(k as u32);
    (mapa.add(0x24) as *mut u32).write_unaligned(k as u32);
    Ok(())
}

/// Próximo nó livre: `(índice, próximo livre, usados)` depois de tirar um nó, ou `None` sem nó livre. Com `proximo == usados` o nó é o
/// primeiro nunca usado e o seguinte é o de depois (nada a ler na lista); senão o nó veio da lista de livres e o seguinte é o `next` dele.
pub(crate) fn proximo_livre(proximo: u32, usados: u32, capacidade: u32, next_do_no: impl Fn(u32) -> u32) -> Option<(u32, u32, u32)> {
    if proximo == u32::MAX {
        return None;
    }
    if usados == proximo {
        return Some(if usados + 1 < capacidade { (proximo, proximo + 1, usados + 1) } else { (proximo, u32::MAX, usados) });
    }
    Some((proximo, next_do_no(proximo), usados))
}

/// Insere `chave` (que o mapa não tem); `inicia` monta o valor no nó (`+0x10`). Devolve o nó. Cresce antes, se não cabe.
pub(crate) unsafe fn inserir(mapa: *mut u8, chave: u64, inicia: impl FnOnce(*mut u8)) -> Result<*mut u8, String> {
    if !crate::memoria::is_readable(mapa as *const c_void, 0x30) {
        return Err("mapa ilegível".into());
    }
    let tam = rd32(mapa.add(0x08)) as usize;
    let cap = rd32(mapa.add(0x0c)) as usize;
    if tam + 1 > cap {
        reservar(mapa, capacidade_para(tam, cap, 1))?;
    }
    let (tabela, cap, nos, passo, cap_nos) = partes(mapa).ok_or("mapa ilegível")?;
    let (proximo, usados) = (rd32(mapa.add(0x20)), rd32(mapa.add(0x24)));
    if proximo != u32::MAX && proximo as usize >= cap_nos {
        return Err(format!("próximo livre {proximo} fora dos {cap_nos} nós"));
    }
    let (idx, seguinte, usados) =
        proximo_livre(proximo, usados, cap_nos as u32, |i| rd32(nos.add(i as usize * passo))).ok_or("sem nó livre")?;
    let n = nos.add(idx as usize * passo);
    core::ptr::write_bytes(n, 0, passo);
    let h = hash_da_chave(chave);
    (n.add(4) as *mut u32).write_unaligned(h);
    (n.add(8) as *mut u64).write_unaligned(chave);
    inicia(n.add(0x10));
    let balde = tabela.add(h as usize % cap);
    (n as *mut u32).write_unaligned(balde.read_unaligned());
    balde.write_unaligned(idx);
    (mapa.add(0x20) as *mut u32).write_unaligned(seguinte);
    (mapa.add(0x24) as *mut u32).write_unaligned(usados);
    (mapa.add(0x08) as *mut u32).write_unaligned(tam as u32 + 1);
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacidade_cresce_meia_vez_ate_caber() {
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

    /// Mapa vazio com o alocador inline falso (alocar `+0x08`, liberar `+0x20`, sobre o alocador do Rust): inserir cresce pelo bloco único
    /// (nós seguidos da tabela), a busca do motor acha cada chave, desligar põe o nó na frente da lista de livres e a inserção seguinte o reusa.
    #[test]
    fn inserir_cresce_busca_desliga_e_reusa_o_no_livre() {
        unsafe extern "C" fn alocar(_: *mut u8, tamanho: u64, alinhamento: u64) -> Bloco {
            let l = std::alloc::Layout::from_size_align(tamanho as usize, alinhamento as usize).expect("layout de teste");
            Bloco { memoria: unsafe { std::alloc::alloc(l) } as u64, _tamanho: tamanho }
        }
        unsafe extern "C" fn liberar(_: *mut u8, bloco: *const [u64; 2]) {
            let [m, t] = unsafe { *bloco };
            unsafe { std::alloc::dealloc(m as *mut u8, std::alloc::Layout::from_size_align(t as usize, 8).expect("layout de teste")) };
        }
        unsafe extern "C" fn nunca(_: *mut u8) {}
        let vtable: [u64; 5] = [nunca as usize as u64, alocar as usize as u64, nunca as usize as u64, nunca as usize as u64, liberar as usize as u64];
        let mut mapa = [0u8; 0x30];
        mapa[0x1c..0x20].copy_from_slice(&0x30u32.to_le_bytes());
        mapa[0x20..0x24].copy_from_slice(&u32::MAX.to_le_bytes());
        mapa[0x28..0x30].copy_from_slice(&(vtable.as_ptr() as u64).to_le_bytes());
        let m = mapa.as_mut_ptr();
        unsafe {
            for k in 1..=10u64 {
                let no = inserir(m, k * 0x1111, |v| (v as *mut u64).write_unaligned(k)).expect("inserir");
                assert_eq!(rd64(no.add(0x10)), k);
            }
            assert_eq!(rd32(m.add(0x08)), 10);
            assert!(rd32(m.add(0x0c)) >= 10);
            for k in 1..=10u64 {
                assert_eq!(no(m, k * 0x1111).map(|n| rd64(n.add(0x10))), Some(k), "chave {k}");
            }
            let tirado = desligar(m, |n| rd64(n.add(8)) == 3 * 0x1111).expect("desligar");
            assert_eq!(rd64(tirado.add(0x10)), 3);
            assert_eq!(rd32(m.add(0x08)), 9);
            assert!(no(m, 3 * 0x1111).is_none());
            let indice = (tirado as usize - rd64(m.add(0x10)) as usize) / 0x30;
            assert_eq!(rd32(m.add(0x20)) as usize, indice);
            let novo = inserir(m, 0xabcd, |v| (v as *mut u64).write_unaligned(99)).expect("inserir no nó livre");
            assert_eq!(novo, tirado);
            assert_eq!(no(m, 0xabcd).map(|n| rd64(n.add(0x10))), Some(99));
            assert_eq!(nos(m).map(|v| v.len()), Some(10));
            let cap = rd32(m.add(0x18)) as usize;
            liberar(m.add(0x28), &[rd64(m.add(0x10)), (cap * (0x30 + 4)) as u64]);
        }
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
