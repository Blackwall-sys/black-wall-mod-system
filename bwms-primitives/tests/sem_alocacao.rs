//! `resource_path_hash` e `sha1` não podem alocar no heap: o primeiro roda em hook e carga, o segundo
//! recebe recursos de `.archive` inteiros (`bwms archive`). O contador do alocador é por thread porque o harness roda os
//! testes em paralelo.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

struct AlocadorContador;

thread_local! {
    static ALOCACOES: Cell<usize> = const { Cell::new(0) };
}

fn contar() {
    // try_with: a thread pode estar no fim, com o TLS já destruído, quando o runtime libera memória.
    let _ = ALOCACOES.try_with(|n| n.set(n.get() + 1));
}

unsafe impl GlobalAlloc for AlocadorContador {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        contar();
        System.alloc(layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        contar();
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        contar();
        System.realloc(ptr, layout, new_size)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALOCADOR: AlocadorContador = AlocadorContador;

fn alocacoes_em(f: impl FnOnce()) -> usize {
    let antes = ALOCACOES.with(Cell::get);
    f();
    ALOCACOES.with(Cell::get) - antes
}

#[test]
fn resource_path_hash_nao_aloca() {
    // Entradas montadas antes da medição: só o hash entra na conta.
    let longo = format!("{}'", "A/".repeat(150));
    let caminhos = ["Base//Characters/V.mesh", "\"base\\gameplay\\factories\\vehicles\\vehicles.csv\"", "", longo.as_str()];
    for c in caminhos {
        let n = alocacoes_em(|| {
            black_box(bwms_primitives::resource_path_hash(black_box(c)));
        });
        assert_eq!(n, 0, "resource_path_hash({c:?}) alocou {n} vez(es)");
    }
}

#[test]
fn sha1_nao_aloca() {
    let grande = vec![0x5Au8; 1 << 20];
    let entradas: [&[u8]; 6] = [b"", &[b'a'; 55], &[b'a'; 56], &[b'a'; 64], &[b'a'; 65], &grande];
    for e in entradas {
        let n = alocacoes_em(|| {
            black_box(bwms_primitives::sha1(black_box(e)));
        });
        assert_eq!(n, 0, "sha1 de {} bytes alocou {n} vez(es)", e.len());
    }
}
