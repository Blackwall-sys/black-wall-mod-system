//! Depot de recursos: o `resource.copy` do `.archive.xl` servido no nível do depot (ArchiveXL `#26`). A cópia é um recurso próprio — o loader
//! cria token e instância pelo caminho da cópia — com os bytes do original: o `RequestResource` do depot (`0x10151a748`, entrada da vtable do
//! `ResourceDepot`; depot `x0`, caminho `x1`, `x2`, handle devolvido por sret `x8`) lê o original quando pedem a cópia, e o `CheckResource`
//! (`0x10151aa2c`, depot `x0`, caminho `x1`) diz que a cópia existe. O `resource.link` fica no loader (`selftest::reslink_*`), onde o pedido
//! vira o recurso do alvo; o `CheckResource` responde pelos dois, como a união de mapeamentos da referência.
//!
//! O mapa de recursos vivos também é lido aqui, só leitura: `depot_cache_buscar` devolve o recurso já carregado de um caminho, sem iniciar
//! carga. O mapa fica em `+0x00` do bank do `ResourceLoader` (`[loader+0x48]`, `loader` = `[CARREGADOR_VM]`), o caminho do `FindToken`
//! (`0x1010e8754` → `0x1010e3a34`). O `ResourceGameDepot` tem 0x80 bytes (`0x10442d35c`) e nenhuma função dele lê `+0xc8`: o
//! `[PATHB_DEPOT+0xc8]` lê 0x48 bytes além do fim do depot, na alocação que vier depois dele no `PoolEngine`, e só é o bank quando essa
//! alocação é o loader (outro bloco de 0x80 B do mesmo pool, `0x1010e9b50`).

use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use crate::arquivos_do_jogo::{TabelaDeRedirecionamento, PALAVRAS_DO_FILTRO};
use crate::register::{rd_u32, rd_u64};

const REQUEST_RESOURCE_VM: u64 = 0x1_0151_a748;

static COPIAS_FILTRO: [AtomicU64; PALAVRAS_DO_FILTRO] = [const { AtomicU64::new(0) }; PALAVRAS_DO_FILTRO];
/// Cópia → original (hashes de caminho), com a regra de toda entrada do `resource.copy` (`TabelaDeRedirecionamento`).
pub(crate) static COPIAS: TabelaDeRedirecionamento = TabelaDeRedirecionamento::nova("depot", &COPIAS_FILTRO, &COPIAS_N);
/// Quantas cópias há: o shim só sai do caminho direto com alguma.
static COPIAS_N: AtomicU64 = AtomicU64::new(0);
/// Trampolins dos originais, publicados antes de o salto ser escrito (com as outras threads paradas).
static ORIG_REQUEST: AtomicU64 = AtomicU64::new(0);
static ORIG_CHECK: AtomicU64 = AtomicU64::new(0);
/// Pedidos de cópia servidos pelo original, pra conferência.
pub(crate) static SERVIDAS: AtomicU64 = AtomicU64::new(0);

/// Registra a cópia `copia` com os bytes de `original` por cima das que já estão.
pub(crate) fn adicionar_copia(copia: u64, original: u64) -> bool {
    COPIAS.acrescentar(copia, original)
}

pub(crate) fn original_de(copia: u64) -> Option<u64> {
    COPIAS.alvo(copia)
}

/// Põe em `saida` os apelidos de `path` pelas cópias, nos dois sentidos: o original quando `path` é cópia, e cada cópia quando `path` é o original. A
/// cópia que também é chave de `resource.link` (`chaves_de_link`, um `RegisterLink` em runtime sobre ela) fica de fora: na referência o
/// `s_mappings[aLink] = aTarget` do `RegisterLink` tira a cópia da união (`ResourceLink/Extension.cpp:277`).
pub(crate) fn apelidos_de_copia(path: u64, saida: &mut Vec<u64>, chaves_de_link: &[u64]) {
    let Some(c) = COPIAS.ler() else { return };
    for &(copia, original) in c.iter() {
        let apelido = if copia == path {
            original
        } else if original == path {
            copia
        } else {
            continue;
        };
        if !chaves_de_link.contains(&copia) {
            saida.push(apelido);
        }
    }
}

/// Chamado pelo shim com o caminho pedido; devolve o do original quando é uma cópia.
unsafe extern "C" fn caminho_servido(caminho: u64) -> u64 {
    match original_de(caminho) {
        Some(o) => {
            SERVIDAS.fetch_add(1, Ordering::Relaxed);
            o
        }
        None => caminho,
    }
}

#[unsafe(naked)]
unsafe extern "C" fn shim_request() {
    core::arch::naked_asm!(
        "adrp x9, {n}@PAGE",
        "add  x9, x9, {n}@PAGEOFF",
        "ldr  x9, [x9]",
        "cbz  x9, 2f",                 // sem cópias: direto pro original
        "stp  x0, x1, [sp, #-0x60]!",
        "stp  x2, x3, [sp, #0x10]",
        "stp  x4, x5, [sp, #0x20]",
        "stp  x6, x7, [sp, #0x30]",
        "stp  x8, x30, [sp, #0x40]",   // x8 = sret do handle devolvido
        "mov  x0, x1",
        "bl   {servido}",
        "mov  x1, x0",                 // caminho que o depot lê
        "ldp  x8, x30, [sp, #0x40]",
        "ldp  x6, x7, [sp, #0x30]",
        "ldp  x4, x5, [sp, #0x20]",
        "ldp  x2, x3, [sp, #0x10]",
        "ldr  x0, [sp]",
        "add  sp, sp, #0x60",
        "2:",
        "adrp x9, {o}@PAGE",
        "add  x9, x9, {o}@PAGEOFF",
        "ldr  x9, [x9]",
        "br   x9",
        n = sym COPIAS_N,
        o = sym ORIG_REQUEST,
        servido = sym caminho_servido,
    )
}

/// `CheckResource`: a cópia existe se o original existe; o caminho de um `link` existe se o alvo existe. O recurso que o depot não tem pega trava só
/// quando a tabela não está vazia e o bit do caminho está ligado (`TabelaDeRedirecionamento::alvo`).
unsafe extern "C" fn check(depot: *mut c_void, caminho: u64) -> u8 {
    let orig: unsafe extern "C" fn(*mut c_void, u64) -> u8 = std::mem::transmute(ORIG_CHECK.load(Ordering::Relaxed) as usize);
    let r = orig(depot, caminho);
    if r != 0 {
        return r;
    }
    match original_de(caminho).or_else(|| crate::arquivos_do_jogo::reslink_alvo(caminho)) {
        Some(alvo) => orig(depot, alvo),
        None => r,
    }
}

/// Instala os dois ganchos (uma vez). Chamado quando a tabela de `bwms-reslink.txt` é lida.
pub(crate) unsafe fn instalar() {
    if ORIG_REQUEST.load(Ordering::Relaxed) != 0 {
        return;
    }
    let (req, chk) = (crate::rebase(REQUEST_RESOURCE_VM), crate::rebase(crate::jogo::recursos::CHECAR_RECURSO_VM));
    if !crate::memoria::is_readable(req as *const c_void, 16) || !crate::memoria::is_readable(chk as *const c_void, 16) {
        return crate::log_dev!("[depot] RequestResource/CheckResource ilegíveis");
    }
    // O `CheckResource` antes: sem ele o `RequestResource` sozinho ainda serve a cópia; o contrário não teria efeito.
    if crate::ganchos::instalar_com_threads_paradas(chk as *mut c_void, check as *mut c_void, Some(&ORIG_CHECK)).is_none() {
        return crate::log_dev!("[depot] gancho do CheckResource recusado");
    }
    if crate::ganchos::instalar_com_threads_paradas(req as *mut c_void, shim_request as *mut c_void, Some(&ORIG_REQUEST)).is_none() {
        return crate::log_dev!("[depot] gancho do RequestResource recusado");
    }
    crate::veredito_dev!("[depot] cópias servidas no depot: RequestResource e CheckResource com gancho ({} cópia(s))", COPIAS_N.load(Ordering::Relaxed));
}

/// Anda o mapa de recursos vivos (`HashMap` do RED no bank do loader, índice de `u32` e nó com próximo em `+0x00`, chave em `+0x08` e valor em
/// `+0x10`) chamando `f(chave, valor)` até ela devolver `false`. Só leitura; nó ilegível encerra a corrente dele.
pub(crate) unsafe fn depot_cache_para_cada(f: impl FnMut(u64, *mut c_void) -> bool) {
    depot_cache_faixa(0, usize::MAX, f);
}

/// `(índice de baldes, nº de baldes, nós, stride)` dos 0x20 bytes do cabeçalho de um `HashMap` do RED (índice `+0x00`, capacidade `+0x0c`,
/// nós `+0x10`, stride `+0x1c`), ou `None` com índice ou nós nulos ou stride fora de 1..=256.
fn cabecalho_do_mapa(b: &[u8; 0x20]) -> Option<(usize, usize, usize, usize)> {
    let u64_em = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes dentro de 0x20")) as usize;
    let u32_em = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes dentro de 0x20")) as usize;
    let (index_table, capacity, nodes, stride) = (u64_em(0x00), u32_em(0x0c), u64_em(0x10), u32_em(0x1c));
    if index_table == 0 || nodes == 0 || stride == 0 || stride > 256 {
        return None;
    }
    Some((index_table, capacity, nodes, stride))
}

/// Os 0x20 bytes do cabeçalho de mapa em `endereco`, numa leitura só; `None` se ilegíveis.
unsafe fn ler_cabecalho(endereco: u64) -> Option<[u8; 0x20]> {
    let mut b = [0u8; 0x20];
    crate::memoria::read_chunk(endereco as usize, &mut b).then_some(b)
}

/// O loader de onde sai o bank: o global `CARREGADOR_VM`, lido direto (fica no `__DATA` da imagem do jogo, mapeado sempre que o `rebase`
/// devolve não nulo). Sem as 2 syscalls de legibilidade do `resloader_this`: toda consulta relê o cabeçalho, e o custo dela é syscall —
/// ficam 2 por cabeçalho (o campo do bank e o cabeçalho). No executável de teste, que não tem o global do jogo, vale o loader falso de
/// `LOADER_DE_TESTE`.
#[cfg(not(test))]
unsafe fn loader_vivo() -> u64 {
    let slot = crate::rebase(crate::jogo::recursos::CARREGADOR_VM) as *mut u64;
    if slot.is_null() {
        return 0;
    }
    // O motor grava o global numa thread e as consultas leem em outras.
    AtomicU64::from_ptr(slot).load(Ordering::Relaxed)
}

#[cfg(test)]
static LOADER_DE_TESTE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
unsafe fn loader_vivo() -> u64 {
    LOADER_DE_TESTE.load(Ordering::Relaxed)
}

/// O bank do loader (`[loader+0x48]`, o `ldr x22, [x0, #0x48]` do `FindToken`); `None` sem loader ou com o campo nulo (o loader nasce com ele
/// zerado e o `0x1010e6f58` o preenche).
unsafe fn bank_do_loader(loader: u64) -> Option<u64> {
    if loader == 0 {
        return None;
    }
    crate::memoria::read_u64((loader + crate::arquivos_do_jogo::LOADER_BANK_OFF) as usize).filter(|&bank| bank != 0)
}

/// `(índice de baldes, nº de baldes, nós, stride)` do mapa de recursos vivos, ou `None` sem loader, sem bank ou com cabeçalho incoerente.
unsafe fn depot_cache_cabecalho() -> Option<(usize, usize, usize, usize)> {
    cabecalho_do_mapa(&ler_cabecalho(bank_do_loader(loader_vivo())?)?)
}

/// `depotcab`: o cabeçalho de mapa pelas duas leituras, para a prova no jogo — `[PATHB_DEPOT+0xc8]` (além do fim do depot) e o bank
/// `[loader+0x48]` que o `depot_cache_cabecalho` usa — com os 0x20 bytes de cada uma, o que a regra do cabeçalho decide sobre eles e a
/// distância loader − depot (0x80 = o loader no bloco seguinte ao depot, o único caso em que as duas leituras coincidem).
#[cfg(feature = "devtools")]
pub(crate) unsafe fn depotcab() -> String {
    let depot = crate::arquivos_do_jogo::PATHB_DEPOT.load(Ordering::Relaxed) as u64;
    let loader = loader_vivo();
    let pelo_depot = if depot == 0 { None } else { crate::memoria::read_u64(depot as usize + 0xc8) };
    let bank = bank_do_loader(loader);
    let descrever = |ponteiro: Option<u64>| -> String {
        let Some(p) = ponteiro else { return "ponteiro ilegível".to_string() };
        match ler_cabecalho(p) {
            None => format!("{p:#x} cabeçalho ilegível"),
            Some(b) => {
                let q: Vec<String> = b.chunks_exact(8).map(|c| format!("{:016x}", u64::from_le_bytes(c.try_into().expect("8 bytes")))).collect();
                format!("{p:#x} bytes=[{}] regra={:?}", q.join(" "), cabecalho_do_mapa(&b))
            }
        }
    };
    format!(
        "[depotcab] depot={depot:#x} loader={loader:#x} loader-depot={} | [depot+0xc8]: {} | [loader+0x48]: {} | iguais={} | \
depot_cache_cabecalho={:?}",
        match (depot, loader) {
            (0, _) | (_, 0) => "?".to_string(),
            _ if loader >= depot => format!("+{:#x}", loader - depot),
            _ => format!("-{:#x}", depot - loader),
        },
        descrever(pelo_depot),
        descrever(bank),
        pelo_depot.is_some() && pelo_depot == bank,
        depot_cache_cabecalho()
    )
}

/// Hasher do cache do depot, conferido em nós reais: 0 não conferido, 1 XOR-fold (o `HashMapHash<ResourcePath>` do SDK,
/// `(u32)k ^ (u32)(k >> 32)`), 2 os 32 bits baixos, 3 nenhum dos dois (a busca fica na varredura).
static HASHER_DO_DEPOT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn hash_do_depot(chave: u64, hasher: u8) -> u32 {
    if hasher == 1 {
        (chave as u32) ^ ((chave >> 32) as u32)
    } else {
        chave as u32
    }
}

/// Confere o hasher em até 32 nós reais: o hash guardado no nó (`+0x04`) tem que bater com o da chave (`+0x08`) e o nó tem que estar no
/// balde `hash % nº de baldes`. Um nó que não bate descarta o hasher. Com menos de 8 nós lidos devolve 0 (decide numa chamada seguinte).
/// Custo limitado: o índice é lido em blocos de 16 KB e só os primeiros 64 Ki baldes entram (cache ralo no começo do boot não vira
/// 1 syscall por balde em toda chamada).
unsafe fn conferir_hasher_do_depot() -> u8 {
    let Some((indice, baldes, nos, stride)) = depot_cache_cabecalho() else { return 0 };
    let (mut xor_ok, mut baixo_ok, mut vistos) = (true, true, 0usize);
    const BALDES_POR_BLOCO: usize = 4096;
    let limite = baldes.min(1 << 16);
    let mut bloco = [0u8; BALDES_POR_BLOCO * 4];
    'blocos: for inicio in (0..limite).step_by(BALDES_POR_BLOCO) {
        let n = BALDES_POR_BLOCO.min(limite - inicio);
        // Bloco ilegível sai inteiro: nada de cair pra 1 syscall por balde.
        if !crate::memoria::read_chunk(indice + inicio * 4, &mut bloco[..n * 4]) {
            continue;
        }
        for j in 0..n {
            let mut cur = u32::from_le_bytes([bloco[j * 4], bloco[j * 4 + 1], bloco[j * 4 + 2], bloco[j * 4 + 3]]);
            let mut guarda = 0;
            while cur != u32::MAX && guarda < 64 {
                if vistos >= 32 {
                    break 'blocos;
                }
                let node = nos + (cur as usize) * stride;
                if !crate::memoria::is_readable(node as *const c_void, 0x18) {
                    break;
                }
                let guardado = rd_u32(node as *const c_void, 0x04);
                let chave = rd_u64(node as *const c_void, 0x08);
                xor_ok &= guardado == hash_do_depot(chave, 1) && guardado as usize % baldes == inicio + j;
                baixo_ok &= guardado == hash_do_depot(chave, 2) && guardado as usize % baldes == inicio + j;
                vistos += 1;
                cur = rd_u32(node as *const c_void, 0x00);
                guarda += 1;
            }
        }
    }
    match (vistos >= 8, xor_ok, baixo_ok) {
        (false, _, _) => 0,
        (true, true, _) => 1,
        (true, false, true) => 2,
        _ => 3,
    }
}

/// Valor do nó com `chave` no cache do depot (nulo se não está carregado). Com o hasher confirmado em nós reais, vai direto ao balde e
/// lê 1–3 nós; sem confirmação, varre o cache inteiro (~55 mil entradas, 1 syscall por nó), como as natives de recurso sempre fizeram.
pub(crate) unsafe fn depot_cache_buscar(chave: u64) -> *mut c_void {
    use std::sync::atomic::Ordering;
    // Sem decisão (cache ralo), a conferência volta só a cada 1024 buscas: as natives e os testes de recurso buscam em lote por tick.
    static BUSCAS_SEM_HASHER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let mut hasher = HASHER_DO_DEPOT.load(Ordering::Relaxed);
    if hasher == 0 && BUSCAS_SEM_HASHER.fetch_add(1, Ordering::Relaxed) % 1024 == 0 {
        hasher = conferir_hasher_do_depot();
        if hasher != 0 {
            HASHER_DO_DEPOT.store(hasher, Ordering::Relaxed);
            crate::veredito_dev!(
                "[depot] hasher do cache: {}",
                match hasher {
                    1 => "XOR-fold (o do SDK)",
                    2 => "32 bits baixos",
                    _ => "nenhum dos dois — busca pela varredura",
                }
            );
        }
    }
    if hasher == 1 || hasher == 2 {
        let Some((indice, baldes, nos, stride)) = depot_cache_cabecalho() else { return std::ptr::null_mut() };
        if baldes == 0 {
            return std::ptr::null_mut();
        }
        let slot = indice + (hash_do_depot(chave, hasher) as usize % baldes) * 4;
        if !crate::memoria::is_readable(slot as *const c_void, 4) {
            return std::ptr::null_mut();
        }
        let mut cur = (slot as *const u32).read_unaligned();
        let mut guarda = 0;
        while cur != u32::MAX && guarda < 4096 {
            let node = nos + (cur as usize) * stride;
            if !crate::memoria::is_readable(node as *const c_void, 0x18) {
                return std::ptr::null_mut();
            }
            if rd_u64(node as *const c_void, 0x08) == chave {
                return rd_u64(node as *const c_void, 0x10) as *mut c_void;
            }
            cur = rd_u32(node as *const c_void, 0x00);
            guarda += 1;
        }
        return std::ptr::null_mut();
    }
    let mut achado = std::ptr::null_mut();
    depot_cache_para_cada(|k, valor| {
        if k == chave {
            achado = valor;
        }
        k != chave
    });
    achado
}

/// Anda os baldes `[inicio, inicio + quantos)` do cache do depot (cada leitura é uma syscall, então varredura grande vai em faixas); devolve o próximo
/// balde, ou `None` quando o cache acabou ou `f` pediu pra parar.
pub(crate) unsafe fn depot_cache_faixa(inicio: usize, quantos: usize, mut f: impl FnMut(u64, *mut c_void) -> bool) -> Option<usize> {
    let (index_table, capacity, nodes, stride) = depot_cache_cabecalho()?;
    let fim = capacity.min(1 << 20);
    let ate = inicio.saturating_add(quantos).min(fim);
    // O índice de baldes é um bloco contíguo do motor: lido 16 KB por syscall em vez de 4 B. Bloco que falha volta a 1 balde por vez.
    const BALDES_POR_BLOCO: usize = 4096;
    let mut bloco = [0u8; BALDES_POR_BLOCO * 4];
    let (mut bloco_inicio, mut bloco_len, mut bloco_ok) = (0usize, 0usize, false);
    for i in inicio..ate {
        if i >= bloco_inicio + bloco_len {
            bloco_inicio = i;
            bloco_len = BALDES_POR_BLOCO.min(ate - i);
            bloco_ok = crate::memoria::read_chunk(index_table + i * 4, &mut bloco[..bloco_len * 4]);
        }
        let mut cur = if bloco_ok {
            let k = (i - bloco_inicio) * 4;
            u32::from_le_bytes([bloco[k], bloco[k + 1], bloco[k + 2], bloco[k + 3]])
        } else {
            let slot = index_table + i * 4;
            if !crate::memoria::is_readable(slot as *const c_void, 4) {
                continue;
            }
            (slot as *const u32).read_unaligned()
        };
        let mut guarda = 0;
        while cur != u32::MAX && guarda < 4096 {
            let node = nodes + (cur as usize) * stride;
            if !crate::memoria::is_readable(node as *const c_void, 0x18) {
                break;
            }
            if !f(rd_u64(node as *const c_void, 0x08), rd_u64(node as *const c_void, 0x10) as *mut c_void) {
                return None;
            }
            cur = rd_u32(node as *const c_void, 0x00);
            guarda += 1;
        }
    }
    (ate < fim).then_some(ate)
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::mods_do_usuario::pipeline::testes::travar_tabelas_do_archivexl;

    #[test]
    fn copia_serve_o_original_e_o_resto_passa() {
        let _trava = travar_tabelas_do_archivexl();
        adicionar_copia(0x11, 0x22);
        adicionar_copia(0x11, 0x33);
        assert_eq!(original_de(0x11), Some(0x33));
        let antes = SERVIDAS.load(Ordering::Relaxed);
        assert_eq!(unsafe { caminho_servido(0x11) }, 0x33);
        assert_eq!(unsafe { caminho_servido(0x44) }, 0x44);
        assert_eq!(SERVIDAS.load(Ordering::Relaxed), antes + 1);
        // `COPIAS` é do processo de teste inteiro (os outros testes deixam cópias nela): a prova é a cópia única de 0x11 e o total que o shim lê
        // igual ao da tabela, lidos com a trava que `adicionar_copia` segura ao gravar os dois.
        let c = COPIAS.ler().expect("tabela de cópias");
        assert_eq!(c.iter().filter(|(k, _)| *k == 0x11).count(), 1);
        assert_eq!(COPIAS_N.load(Ordering::Relaxed), c.len() as u64);
    }

    /// O gancho do `CheckResource` responde pela união das tabelas (o `OnDepotResourceCheck` de `ResourceLink/Extension.cpp:237-249`): a cópia e o
    /// pedido de link existem quando o original ou o alvo existe, e o caminho do jogo passa direto. Por isso a recusa por existência da referência
    /// (`Extension.cpp:65`, `:99`, `:177`) só vale contra o original (`ORIG_CHECK`): por este gancho todo link carregado daria "já existe". O depot
    /// falso conhece só `bwms\f126\vanilla.mesh`, e os caminhos `bwms\f126\…` são só deste teste (as tabelas são do processo de teste inteiro).
    #[test]
    fn check_responde_pela_uniao_e_o_original_nao() {
        let _trava = travar_tabelas_do_archivexl();
        unsafe extern "C" fn depot_falso(_depot: *mut c_void, caminho: u64) -> u8 {
            u8::from(caminho == bwms_primitives::resource_path_hash("bwms\\f126\\vanilla.mesh"))
        }
        let h = |nome: &str| bwms_primitives::resource_path_hash(&format!("bwms\\f126\\{nome}.mesh"));
        let (vanilla, ausente, copia, pedido) = (h("vanilla"), h("ausente"), h("copia"), h("pedido"));
        let (copia_de_ausente, pedido_de_ausente) = (h("copia_de_ausente"), h("pedido_de_ausente"));
        let original: unsafe extern "C" fn(*mut c_void, u64) -> u8 = depot_falso;
        ORIG_CHECK.store(original as usize as u64, Ordering::Relaxed);
        assert!(adicionar_copia(copia, vanilla) && adicionar_copia(copia_de_ausente, ausente));
        assert!(crate::arquivos_do_jogo::reslink_add(pedido, vanilla) && crate::arquivos_do_jogo::reslink_add(pedido_de_ausente, ausente));

        let pelo_gancho = |c: u64| unsafe { check(core::ptr::null_mut(), c) };
        let pelo_original = |c: u64| unsafe { depot_falso(core::ptr::null_mut(), c) };
        assert_eq!([vanilla, copia, pedido].map(pelo_gancho), [1, 1, 1]);
        assert_eq!([vanilla, copia, pedido].map(pelo_original), [1, 0, 0]);
        assert_eq!([ausente, copia_de_ausente, pedido_de_ausente].map(pelo_gancho), [0, 0, 0]);
    }

    /// `apply-03`: a recarga troca a tabela de cópias inteira, com a regra de `adicionar_copia` (a cópia repetida fica com o último original, o caminho
    /// nulo não entra) e com a contagem que o shim lê igual à da tabela nova; a lista vazia esvazia, e o shim volta a servir o próprio caminho.
    #[test]
    fn substituir_troca_a_tabela_de_copias_e_a_contagem_do_shim() {
        let _trava = travar_tabelas_do_archivexl();
        assert!(adicionar_copia(0x12701, 0x12702));
        assert_eq!(COPIAS.substituir(&[(0x12711, 0x12712), (0x12711, 0x12713), (0, 0x12714), (0x12715, 0)]), 2);
        assert_eq!(*COPIAS.ler().expect("tabela de cópias"), vec![(0x12711, 0x12713)]);
        assert_eq!(COPIAS_N.load(Ordering::Relaxed), 1);
        assert_eq!(original_de(0x12701), None);
        assert_eq!(unsafe { caminho_servido(0x12711) }, 0x12713);

        assert_eq!(COPIAS.substituir(&[]), 0);
        assert!(COPIAS.ler().expect("tabela de cópias").is_empty());
        assert_eq!(COPIAS_N.load(Ordering::Relaxed), 0);
        assert_eq!(unsafe { caminho_servido(0x12711) }, 0x12711);
    }

    /// Os 0x20 bytes do cabeçalho de um `HashMap` do RED com `indice`, `baldes`, `nos` e `stride` nos offsets do motor.
    fn bytes_de_cabecalho(indice: u64, baldes: u32, nos: u64, stride: u32) -> [u8; 0x20] {
        let mut b = [0u8; 0x20];
        b[0x00..0x08].copy_from_slice(&indice.to_le_bytes());
        b[0x08..0x0c].copy_from_slice(&7u32.to_le_bytes());
        b[0x0c..0x10].copy_from_slice(&baldes.to_le_bytes());
        b[0x10..0x18].copy_from_slice(&nos.to_le_bytes());
        b[0x1c..0x20].copy_from_slice(&stride.to_le_bytes());
        b
    }

    /// A regra do cabeçalho: lê índice `+0x00`, capacidade `+0x0c`, nós `+0x10` e stride `+0x1c` (o `size` em `+0x08` não entra), e recusa índice ou
    /// nós nulos e stride 0 ou acima de 256. É a mesma regra que o `depotcab` aplica aos bytes de `[depot+0xc8]` e aos do bank.
    #[test]
    fn cabecalho_do_mapa_le_os_offsets_do_motor_e_recusa_o_incoerente() {
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 61447, 0x2000, 0x28)), Some((0x1000, 61447, 0x2000, 0x28)));
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 0, 0x2000, 1)), Some((0x1000, 0, 0x2000, 1)));
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 16, 0x2000, 256)), Some((0x1000, 16, 0x2000, 256)));
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0, 16, 0x2000, 0x28)), None);
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 16, 0, 0x28)), None);
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 16, 0x2000, 0)), None);
        assert_eq!(cabecalho_do_mapa(&bytes_de_cabecalho(0x1000, 16, 0x2000, 257)), None);
        assert_eq!(cabecalho_do_mapa(&[0u8; 0x20]), None);
    }

    /// O mapa de recursos vivos com o layout do bank do motor (índice de `u32`, nó `{próximo, hash, chave, WeakPtr{valor, bloco}}` de 0x28, o
    /// stride medido no boot 257), montado na memória do processo de teste e achado pelo loader falso em `[loader+0x48]`: a conferência acha o
    /// XOR-fold do SDK, a busca pelo balde e a varredura devolvem o valor do bank. Um depot falso em `PATHB_DEPOT` com outro mapa coerente em
    /// `+0xc8` (o que o bloco seguinte ao depot guarda quando é o loader) não muda nada: a busca não lê o depot. Sem loader, ou com o bank nulo,
    /// tudo devolve nulo. O loader e o depot falsos só ficam nos globais com a trava das tabelas (a carga do pipeline, que os outros testes rodam com
    /// ela, lê o depot); a volta a nulo vale também quando uma asserção falha.
    #[test]
    fn mapa_de_recursos_vem_do_bank_do_loader_e_nao_do_depot() {
        let _trava = travar_tabelas_do_archivexl();
        struct VoltaSemLoader;
        impl Drop for VoltaSemLoader {
            fn drop(&mut self) {
                LOADER_DE_TESTE.store(0, Ordering::Relaxed);
                crate::arquivos_do_jogo::PATHB_DEPOT.store(core::ptr::null_mut(), Ordering::Relaxed);
                HASHER_DO_DEPOT.store(0, Ordering::Relaxed);
            }
        }
        let _volta = VoltaSemLoader;
        const BALDES: usize = 16;
        let chave = |i: u64| 0x5000_0000_0000_0000 + i * 0x1_0000_0003;
        let montar = |valor: &dyn Fn(u64) -> u64| {
            let mut indice = vec![u32::MAX; BALDES];
            let mut nos = vec![[0u64; 5]; 12];
            for (i, no) in nos.iter_mut().enumerate() {
                let k = chave(i as u64);
                let hash = (k as u32) ^ ((k >> 32) as u32);
                let balde = hash as usize % BALDES;
                *no = [u64::from(indice[balde]) | u64::from(hash) << 32, k, valor(i as u64), 0x6000_0000 + i as u64, 0];
                indice[balde] = i as u32;
            }
            let cabecalho = [indice.as_ptr() as u64, (BALDES as u64) << 32, nos.as_ptr() as u64, 0x28u64 << 32];
            (indice, nos, cabecalho)
        };
        let valor = |i: u64| 0x7000_0000 + i * 0x10;
        let (_indice, nos, bank) = montar(&valor);
        let (_indice_isca, _nos_isca, mapa_isca) = montar(&|i| 0x9000_0000 + i * 0x10);
        let mut loader = [0u64; 0x80 / 8];
        loader[0x48 / 8] = bank.as_ptr() as u64;
        let mut depot = [0u64; 0xd0 / 8];
        depot[0xc8 / 8] = mapa_isca.as_ptr() as u64;
        crate::arquivos_do_jogo::PATHB_DEPOT.store(depot.as_mut_ptr().cast(), Ordering::Relaxed);
        LOADER_DE_TESTE.store(loader.as_ptr() as u64, Ordering::Relaxed);
        unsafe {
            assert_eq!(depot_cache_cabecalho(), Some((bank[0] as usize, BALDES, bank[2] as usize, 0x28)));
            assert_eq!(conferir_hasher_do_depot(), 1);
            for hasher in [1, 3] {
                HASHER_DO_DEPOT.store(hasher, Ordering::Relaxed);
                for i in 0..nos.len() as u64 {
                    assert_eq!(depot_cache_buscar(chave(i)) as u64, valor(i), "hasher {hasher}, nó {i}");
                }
                assert!(depot_cache_buscar(chave(99)).is_null(), "hasher {hasher}, chave ausente");
            }
            // Volátil: quem lê o campo é o `read_u64` do kernel, que o compilador não enxerga.
            std::ptr::write_volatile(&mut loader[0x48 / 8], 0);
            assert_eq!(depot_cache_cabecalho(), None, "bank nulo");
            LOADER_DE_TESTE.store(0, Ordering::Relaxed);
            for hasher in [0, 1, 3] {
                HASHER_DO_DEPOT.store(hasher, Ordering::Relaxed);
                assert!(depot_cache_buscar(chave(0)).is_null(), "hasher {hasher}, sem loader com o depot capturado");
            }
        }
    }
}
