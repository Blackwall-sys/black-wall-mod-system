//! Teste ao vivo da localização (`~/.bwms-hook-loc-teste`), na thread do jogo, com a trava de save:
//! 1. controle: o `onscreens` vanilla do idioma em uso, carregado pelo motor e lido pelo RTTI, tem que bater com o `GetText` do jogo — e é
//!    ele que diz qual mapa guarda o texto feminino;
//! 2. troca: 3 chaves que existem recebem o texto do `onscreens` de outro idioma (carregado pelo mesmo caminho dos mods), o `GetText` tem que
//!    devolver o texto novo, e depois o original volta;
//! 3. chave nova, pela primária e pela secundária (FNV-1a 32 e 64), nos dois mapas, lida de volta pelo `GetText`.

use super::{chaves, entradas_do_token, gravar_entrada, idioma_atual, pedir_recurso, repositorio, texto_do_jogo, cstring_texto, Entrada, Texto, MAPA_VARIANTE_0, MAPA_VARIANTE_1};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

type Pedido = Result<[u64; 2], String>;

/// Os dois onscreens pedidos (o do idioma em uso e o do outro), quando e em que idioma.
static PEDIDOS: std::sync::Mutex<Option<(String, Pedido, Pedido, std::time::Instant)>> = std::sync::Mutex::new(None);

pub(crate) fn tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("loc-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 40 {
        return;
    }
    let Ok(mut pedidos) = PEDIDOS.lock() else { return };
    unsafe {
        if pedidos.is_none() {
            let idioma = idioma_atual().unwrap_or_default();
            let outro = if idioma == "pt-br" { "en-us" } else { "pt-br" };
            let pedir = |i: &str| pedir_recurso(&format!("base\\localization\\{i}\\onscreens\\onscreens.json"));
            *pedidos = Some((idioma.clone(), pedir(&idioma), pedir(outro), std::time::Instant::now()));
            return;
        }
        let Some((idioma, vanilla, alheias, desde)) = pedidos.as_ref() else { return };
        let carregando = [vanilla, alheias].iter().any(|p| matches!(p, Ok(t) if crate::register::status_do_token(t[0]) == 0));
        if carregando && desde.elapsed().as_secs() < 30 {
            return;
        }
        FEITO.store(true, Ordering::Relaxed);
        let ler = |p: &Pedido| p.clone().and_then(|t| entradas_do_token(t)?.ok_or_else(|| format!("não terminou em {} s", desde.elapsed().as_secs())));
        let linha = rodar(idioma, ler(vanilla), ler(alheias));
        crate::log_veredito(&linha);
    }
}

fn estado(r: &Result<Vec<Entrada>, String>) -> String {
    match r {
        Ok(v) => format!("{} entradas", v.len()),
        Err(e) => format!("erro: {e}"),
    }
}

unsafe fn tamanhos(repo: *mut u8) -> [(u32, u32); 2] {
    let t = |off: usize| {
        let m = repo.add(off);
        ((m.add(0x08) as *const u32).read_unaligned(), (m.add(0x0c) as *const u32).read_unaligned())
    };
    [t(MAPA_VARIANTE_1), t(MAPA_VARIANTE_0)]
}

unsafe fn rodar(idioma: &str, vanilla: Result<Vec<Entrada>, String>, alheias: Result<Vec<Entrada>, String>) -> String {
    let Some(repo) = repositorio() else { return "[loc] VEREDITO-LOC sem repositório de textos; loc-ok=false".into() };
    let antes = tamanhos(repo);
    // 1. Controle contra o jogo e sentido dos mapas.
    let (mut batem, mut conferidas, mut mapa1_feminino) = (0, 0, None);
    if let Ok(v) = &vanilla {
        for e in v.iter().filter(|e| e.primaria != 0).take(200) {
            let (f, m) = (cstring_texto(e.feminino), cstring_texto(e.masculino));
            let (g1, g0) = (texto_do_jogo(repo, e.primaria, 1), texto_do_jogo(repo, e.primaria, 0));
            conferidas += 1;
            if !m.is_empty() && f != m && mapa1_feminino.is_none() {
                mapa1_feminino = Some(g1.as_deref() == Some(f.as_str()));
            }
            let m = if m.is_empty() { f.clone() } else { m };
            if (g1.as_deref() == Some(f.as_str()) && g0.as_deref() == Some(m.as_str())) || (g1.as_deref() == Some(m.as_str()) && g0.as_deref() == Some(f.as_str())) {
                batem += 1;
            }
        }
    }
    // 2. Troca por entradas de outro idioma, e volta: 2 chaves sem masculino (só o +0x38 muda, e a variante 0 cai nele) e 1 com masculino nos
    //    dois lados (os dois mapas mudam).
    let outro = if idioma == "pt-br" { "en-us" } else { "pt-br" };
    let (mut trocou, mut voltou, mut escolhidas, mut com_masculino) = (0, 0, 0, 0);
    if let Ok(a) = &alheias {
        for e in a.iter().filter(|e| e.primaria != 0) {
            if escolhidas == 3 {
                break;
            }
            let (novo_f, novo_m) = (cstring_texto(e.feminino), cstring_texto(e.masculino));
            let tem_masculino_no_jogo = crate::jogo::mapas::no(repo.add(MAPA_VARIANTE_0), e.primaria).is_some();
            let quer_masculino = escolhidas == 2;
            let serve = if quer_masculino { !novo_m.is_empty() && tem_masculino_no_jogo } else { novo_m.is_empty() && !tem_masculino_no_jogo };
            if novo_f.is_empty() || !serve {
                continue;
            }
            let (Some(o1), Some(o0)) = (texto_do_jogo(repo, e.primaria, 1), texto_do_jogo(repo, e.primaria, 0)) else { continue };
            if novo_f == o1 {
                continue;
            }
            escolhidas += 1;
            com_masculino += quer_masculino as usize;
            let masculino = quer_masculino.then_some(Texto::DoMotor(e.masculino));
            let r = gravar_entrada(repo, e.primaria, &Texto::DoMotor(e.feminino), masculino.as_ref(), true);
            let esperado_0 = if quer_masculino { novo_m.as_str() } else { novo_f.as_str() };
            if r.iter().all(|x| *x == Ok(false))
                && texto_do_jogo(repo, e.primaria, 1).as_deref() == Some(novo_f.as_str())
                && texto_do_jogo(repo, e.primaria, 0).as_deref() == Some(esperado_0)
            {
                trocou += 1;
            }
            let masculino_original = quer_masculino.then_some(Texto::Proprio(&o0));
            let _ = gravar_entrada(repo, e.primaria, &Texto::Proprio(&o1), masculino_original.as_ref(), true);
            if texto_do_jogo(repo, e.primaria, 1).as_deref() == Some(o1.as_str()) && texto_do_jogo(repo, e.primaria, 0).as_deref() == Some(o0.as_str()) {
                voltou += 1;
            }
        }
    }
    // 3. Chaves novas: pela primária (com masculino) e pela secundária (texto único; a variante 0 cai no +0x38).
    let primaria = bwms_primitives::fnv1a64(b"BwmsTesteLocPrimaria263");
    let secundarias = chaves(0, "BwmsTesteLocSecundaria263");
    let mut novas_ok = 0;
    let mut lidas = Vec::new();
    let mut erros = Vec::new();
    for (chave, fem, masc) in std::iter::once((primaria, "BWMS feminino 263", Some("BWMS masculino 263")))
        .chain(secundarias.iter().map(|k| (*k, "BWMS secundária 263", None)))
    {
        let masculino = masc.map(Texto::Proprio);
        let r = gravar_entrada(repo, chave, &Texto::Proprio(fem), masculino.as_ref(), true);
        erros.extend(r.iter().filter_map(|x| x.clone().err()));
        let (l1, l0) = (texto_do_jogo(repo, chave, 1), texto_do_jogo(repo, chave, 0));
        if r.iter().all(|x| *x == Ok(true)) && l1.as_deref() == Some(fem) && l0.as_deref() == Some(masc.unwrap_or(fem)) {
            novas_ok += 1;
        }
        lidas.push((l1, l0));
    }
    let depois = tamanhos(repo);
    let controle = conferidas > 0 && batem * 100 >= conferidas * 95;
    let ok = controle && mapa1_feminino == Some(true) && escolhidas == 3 && trocou == 3 && voltou == 3 && novas_ok == 3;
    format!(
        "[loc] VEREDITO-LOC idioma '{idioma}'; controle: onscreens vanilla pelo motor {}, {batem} de {conferidas} iguais ao GetText, \
         mapa +0x38 é o feminino {mapa1_feminino:?}; troca pelo onscreens '{outro}' ({}): {trocou} de {escolhidas} ({com_masculino} com masculino) lidas de volta, {voltou} voltaram ao \
         original; chaves novas (primária e as 2 da secundária) {novas_ok} de 3 {erros:?}, lidas {lidas:?}; mapas (tamanho, capacidade) antes {antes:?} depois \
         {depois:?}; loc-ok={ok}",
        estado(&vanilla),
        estado(&alheias),
    )
}

/// Ponta a ponta do caminho de produção (`~/.bwms-hook-loc-mod-teste`): com a tabela da instalação (`red4ext/bwms-localizacao.txt`)
/// apontando um "mod" do idioma em uso pro onscreens de outro idioma que o depot tem, o tick de produção aplica a tabela; o teste espera
/// essa aplicação, carrega o mesmo recurso e confere que o `GetText` agora devolve os textos dele.
pub(crate) fn tick_mod() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static PEDIDO: std::sync::Mutex<Option<(Pedido, std::time::Instant)>> = std::sync::Mutex::new(None);
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("loc-mod-teste") {
        return;
    }
    if !crate::selftest::trava_de_save_confirmada() || !crate::inicio::boot::gameplay_real() {
        return;
    }
    let Some((idioma, lidos, novas, trocadas, falhas)) = super::aplicado() else { return };
    let Ok(mut pedido) = PEDIDO.lock() else { return };
    unsafe {
        let outro = if idioma == "pt-br" { "en-us" } else { "pt-br" };
        if pedido.is_none() {
            *pedido = Some((pedir_recurso(&format!("base\\localization\\{outro}\\onscreens\\onscreens.json")), std::time::Instant::now()));
            return;
        }
        let Some((p, desde)) = pedido.as_ref() else { return };
        if matches!(p, Ok(t) if crate::register::status_do_token(t[0]) == 0) && desde.elapsed().as_secs() < 30 {
            return;
        }
        FEITO.store(true, Ordering::Relaxed);
        let entradas = p.clone().and_then(|t| entradas_do_token(t)?.ok_or_else(|| "não terminou em 30 s".to_string()));
        let Some(repo) = repositorio() else { return };
        let (mut conferidas, mut iguais) = (0, 0);
        if let Ok(v) = &entradas {
            for e in v.iter().filter(|e| e.primaria != 0).take(200) {
                conferidas += 1;
                let f = cstring_texto(e.feminino);
                let m = cstring_texto(e.masculino);
                let m = if m.is_empty() { f.clone() } else { m };
                if texto_do_jogo(repo, e.primaria, 1).as_deref() == Some(f.as_str()) && texto_do_jogo(repo, e.primaria, 0).as_deref() == Some(m.as_str()) {
                    iguais += 1;
                }
            }
        }
        let ok = lidos == 1 && trocadas > 0 && falhas == 0 && conferidas > 0 && iguais * 100 >= conferidas * 95;
        crate::log_veredito(&format!(
            "[locm] VEREDITO-LOCM idioma '{idioma}': a tabela da instalação foi aplicada pelo tick de produção ({lidos} recurso(s), {novas} texto(s) \
             novo(s), {trocadas} trocado(s), {falhas} falha(s)); o onscreens '{outro}' da tabela ({}) dá {iguais} de {conferidas} textos iguais ao GetText; \
             locmod-ok={ok}",
            estado(&entradas)
        ));
    }
}

/// Os textos que o jogo aplica pelo `.json` (`~/.bwms-hook-xl-nativo-teste`, `mods_do_usuario::pasta_mods::textos_no_jogo`): para cada `.xl`
/// entregue ao jogo, carrega os recursos do idioma em uso (ou da reserva do mod, sem ele) pelo mesmo caminho dos mods e confere no `GetText`
/// que as chaves trazem o texto deles, e que a `bwms-localizacao.txt` não tem linha desse `.xl` (o BWMS não aplicou). Só lê: sem trava de save.
pub(crate) fn tick_xl_nativo() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FEITO: AtomicBool = AtomicBool::new(false);
    static INICIO: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    type Pedidos = Vec<(String, String, Vec<(String, Pedido)>)>;
    static PEDIDOS: std::sync::Mutex<Option<(Pedidos, std::time::Instant)>> = std::sync::Mutex::new(None);
    if FEITO.load(Ordering::Relaxed) || !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 30) || !crate::marcador_individual("xl-nativo-teste") {
        return;
    }
    if !crate::inicio::boot::gameplay_real() || INICIO.get_or_init(std::time::Instant::now).elapsed().as_secs() < 20 {
        return;
    }
    let Ok(mut pedidos) = PEDIDOS.lock() else { return };
    unsafe {
        if pedidos.is_none() {
            let idioma = idioma_atual().unwrap_or_default();
            let lista: Pedidos = crate::mods_do_usuario::pasta_mods::textos_no_jogo()
                .into_iter()
                .map(|(id, onscreens)| {
                    let (lido, caminhos) = onscreens
                        .iter()
                        .find(|(i, _)| *i == idioma)
                        .or_else(|| onscreens.first())
                        .map(|(i, v)| (i.clone(), v.clone()))
                        .unwrap_or_default();
                    (id, lido, caminhos.iter().map(|c| (c.clone(), pedir_recurso(c))).collect())
                })
                .collect();
            if lista.is_empty() {
                FEITO.store(true, Ordering::Relaxed);
                crate::log_veredito("[xlnativo] VEREDITO-XLNATIVO nenhum .xl entregue ao jogo neste boot; xlnativo-ok=false");
                return;
            }
            *pedidos = Some((lista, std::time::Instant::now()));
            return;
        }
        let Some((lista, desde)) = pedidos.as_ref() else { return };
        let carregando = lista.iter().flat_map(|(_, _, p)| p).any(|(_, p)| matches!(p, Ok(t) if crate::register::status_do_token(t[0]) == 0));
        if carregando && desde.elapsed().as_secs() < 30 {
            return;
        }
        FEITO.store(true, Ordering::Relaxed);
        let Some(repo) = repositorio() else {
            return crate::log_veredito("[xlnativo] VEREDITO-XLNATIVO sem repositório de textos; xlnativo-ok=false");
        };
        let idioma = idioma_atual().unwrap_or_default();
        let dir = crate::arquivos_do_jogo::game_dir_from_dylib().unwrap_or_default();
        let tabela =
            std::fs::read_to_string(std::path::Path::new(&dir).join(bwms_primitives::PASTA_DO_RUNTIME).join(bwms_primitives::tabelas::LOCALIZACAO.arquivo))
                .unwrap_or_default();
        let mut todos_ok = true;
        for (id, lido, recursos) in lista {
            let (mut conferidas, mut iguais, mut erros) = (0usize, 0usize, Vec::new());
            for (caminho, p) in recursos {
                match p.clone().and_then(|t| entradas_do_token(t)?.ok_or_else(|| format!("não terminou em {} s", desde.elapsed().as_secs()))) {
                    Ok(v) => {
                        for e in v.iter().take(200) {
                            let (f, m) = (cstring_texto(e.feminino), cstring_texto(e.masculino));
                            let m = if m.is_empty() { f.clone() } else { m };
                            for k in chaves(e.primaria, &e.secundaria) {
                                conferidas += 1;
                                let (g1, g0) = (texto_do_jogo(repo, k, 1), texto_do_jogo(repo, k, 0));
                                if (g1.as_deref() == Some(f.as_str()) && g0.as_deref() == Some(m.as_str()))
                                    || (g1.as_deref() == Some(m.as_str()) && g0.as_deref() == Some(f.as_str()))
                                {
                                    iguais += 1;
                                }
                            }
                        }
                    }
                    Err(e) => erros.push(format!("{caminho}: {e}")),
                }
            }
            let linhas = bwms_primitives::tabelas::ler(&tabela, bwms_primitives::tabelas::Localizacao::ler).flatten().filter(|l| l.mod_id == id).count();
            let ok = erros.is_empty() && conferidas > 0 && iguais * 100 >= conferidas * 95 && linhas == 0;
            todos_ok &= ok;
            crate::log_veredito(&format!(
                "[xlnativo] VEREDITO-XLNATIVO {id} idioma '{idioma}' (recursos de '{lido}'): {iguais} de {conferidas} chaves do mod com o texto dele no \
                 GetText (aplicadas pelo jogo); linhas deste .xl na bwms-localizacao.txt: {linhas}; erros {erros:?}; xlnativo-ok={ok}"
            ));
        }
        crate::log_veredito(&format!("[xlnativo] série encerrada: {} .xl; todos-ok={todos_ok}", lista.len()));
    }
}
