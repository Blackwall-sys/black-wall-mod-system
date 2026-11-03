//! Repositório vivo de textos do jogo: os textos de mod (`localization.onscreens` do `.archive.xl`) entram nos mapas que o
//! `GetText` do motor lê. O recurso de textos do mod é carregado pelo próprio motor (que lê o `.archive` do mod, comprimido ou não), as
//! entradas saem pelo RTTI e cada texto vira um `CString` do motor no nó do mapa.
//!
//! RE (2026-09-19): o LocMgr é `*(0x109d021c0)` e o repositório fica em `+0x28`. `GetText` (`0x101c32f4c`: repositório, chave, variante,
//! `&achou`, sret `CString`) procura a chave no mapa `+0x38` (variante ≠ 0) ou `+0x68` (variante 0); o chamador (`0x101c331f4`) tenta de
//! novo por um mapa de apelidos em `+0x08`. Os dois mapas de texto são `HashMap<u64, CString>` com nó de 0x30 ([`crate::jogo::mapas`]).
//! Entrada de mod troca o texto de uma chave que já existe, menos a do idioma de reserva.

use crate::jogo::mapas;
use core::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(feature = "devtools")] // teste ao vivo, só no dev
pub(crate) mod teste;

const LOCMGR_VM: u64 = 0x1_09d0_21c0;
const REPOSITORIO_OFF: usize = 0x28;
/// Mapa da variante ≠ 0 e da variante 0 do `GetText`.
pub(crate) const MAPA_VARIANTE_1: usize = 0x38;
pub(crate) const MAPA_VARIANTE_0: usize = 0x68;
#[cfg(feature = "devtools")]
const GET_TEXT_VM: u64 = 0x1_01c3_2f4c;

/// Repositório de textos, com os dois mapas já carregados.
pub(crate) unsafe fn repositorio() -> Option<*mut u8> {
    let global = crate::rebase(LOCMGR_VM) as *const u64;
    if !crate::memoria::is_readable(global as *const c_void, 8) {
        return None;
    }
    let mgr = global.read_unaligned() as *mut u8;
    if mgr.is_null() || !crate::memoria::is_readable(mgr as *const c_void, 0xa0) {
        return None;
    }
    let repo = mgr.add(REPOSITORIO_OFF);
    (mapas::partes(repo.add(MAPA_VARIANTE_1)).is_some() && mapas::partes(repo.add(MAPA_VARIANTE_0)).is_some()).then_some(repo)
}

pub(crate) use crate::jogo::cstring::texto as cstring_texto;
use crate::jogo::cstring::{copia as cstring_copia, destroi as cstring_destroi, nova as cstring_nova};

/// O texto que o jogo devolve pra `chave` na variante (`GetText`), ou `None` se ele não acha.
#[cfg(feature = "devtools")]
pub(crate) unsafe fn texto_do_jogo(repo: *mut u8, chave: u64, variante: u32) -> Option<String> {
    let mut saida = [0u8; 0x20];
    let mut achou: u8 = 0;
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(GET_TEXT_VM),
        in("x0") repo,
        in("x1") chave,
        in("x2") variante as u64,
        in("x3") &mut achou as *mut u8,
        in("x8") saida.as_mut_ptr(),
        clobber_abi("C"),
    );
    let texto = cstring_texto(saida.as_ptr());
    cstring_destroi(saida.as_mut_ptr());
    (achou != 0).then_some(texto)
}

/// Texto novo pra `chave` num dos mapas, a partir de um `CString` do motor (`fonte`) ou de texto do BWMS. `Ok(true)` se a chave era nova,
/// `Ok(false)` se trocou o texto de uma que existia; com `troca = false`, uma chave que existe fica como está (`Ok(false)`).
pub(crate) enum Texto<'a> {
    DoMotor(*const u8),
    Proprio(&'a str),
}

pub(crate) unsafe fn gravar(repo: *mut u8, mapa_off: usize, chave: u64, texto: &Texto, troca: bool) -> Result<bool, String> {
    let mapa = repo.add(mapa_off);
    let monta = |dst: *mut u8| match texto {
        Texto::DoMotor(fonte) => cstring_copia(dst, *fonte),
        Texto::Proprio(t) => cstring_nova(dst, t),
    };
    if let Some(no) = mapas::no(mapa, chave) {
        if troca {
            cstring_destroi(no.add(0x10));
            monta(no.add(0x10));
        }
        return Ok(false);
    }
    mapas::inserir(mapa, chave, monta).map(|_| true)
}

/// Uma entrada de texto de um recurso `onscreens` carregado: os `CString` apontam pra dentro do recurso (vivo enquanto o token durar).
pub(crate) struct Entrada {
    pub primaria: u64,
    pub secundaria: String,
    pub feminino: *const u8,
    pub masculino: *const u8,
}

/// Recursos carregados pelo motor e mantidos vivos (o token segura o recurso; os textos são copiados, mas o token fica até o fim).
static TOKENS: std::sync::Mutex<Vec<[u64; 2]>> = std::sync::Mutex::new(Vec::new());

/// Pede ao motor o recurso de textos em `caminho` e devolve o token, sem esperar: a carga termina em outra thread. O depot confere antes que o
/// caminho existe (o token de um caminho que não existe nunca termina com erro).
pub(crate) unsafe fn pedir_recurso(caminho: &str) -> Result<[u64; 2], String> {
    let hash = bwms_primitives::resource_path_hash(caminho);
    if crate::resource_exists_hash(hash) == Some(false) {
        return Err(format!("o depot não tem {hash:#018x}"));
    }
    let token = crate::jogo::recursos::token_do_path(hash).ok_or("sem loader")?;
    if token[0] == 0 {
        return Err("token nulo".into());
    }
    if let Ok(mut t) = TOKENS.lock() {
        t.push(token);
    }
    Ok(token)
}

/// Entradas do recurso do token (`JsonResource.root` → `entries`): `None` enquanto carrega. A fila com `Wait` no job não bloqueia neste binário
/// (boot 22), e no boot 268 o onscreens vanilla ainda carregava quando foi lido; por isso quem pede volta num tick seguinte.
pub(crate) unsafe fn entradas_do_token(token: [u64; 2]) -> Result<Option<Vec<Entrada>>, String> {
    match crate::register::status_do_token(token[0]) {
        0 => return Ok(None),
        1 => {}
        s => return Err(format!("a carga falhou (status {s}, erro {:#x})", ((token[0] + 0x5c) as *const u8).read_unaligned())),
    }
    let reg = crate::rtti::Registry::obtain().ok_or("sem registry")?;
    let recurso = ((token[0] + 0x28) as *const u64).read_unaligned() as *mut c_void;
    if !crate::rtti::sane(recurso) {
        return Err("token terminou sem recurso".into());
    }
    let campo = |obj: *mut c_void, nome: &str| -> Option<*mut u8> {
        let (voff, _, holder) = crate::rtti::resolve_prop_in_class(crate::rtti::class_of(obj), nome)?;
        let p = crate::rtti::field_ptr(obj, voff, holder) as *mut u8;
        (!p.is_null()).then_some(p)
    };
    let raiz = campo(recurso, "root").map(|p| (p as *const u64).read_unaligned() as *mut c_void).ok_or("sem root")?;
    if !crate::rtti::sane(raiz) {
        return Err("root vazio".into());
    }
    let lista = campo(raiz, "entries").ok_or("sem entries")?;
    let (dados, n) = ((lista as *const u64).read_unaligned() as *const u8, ((lista.add(0x0c)) as *const u32).read_unaligned() as usize);
    let cls = reg.class_by_name("localizationPersistenceOnScreenEntry");
    if cls.is_null() {
        return Err("sem a classe da entrada".into());
    }
    let off = |nome: &str| crate::rtti::resolve_prop_in_class(cls, nome).map(|(o, _, _)| o as usize);
    let (Some(o_p), Some(o_s), Some(o_f), Some(o_m)) = (off("primaryKey"), off("secondaryKey"), off("femaleVariant"), off("maleVariant")) else {
        return Err("campos da entrada não resolveram".into());
    };
    let passo = crate::rtti::type_size(cls) as usize;
    if passo == 0 || n > 0x10_0000 || (n > 0 && !crate::memoria::is_readable(dados as *const c_void, n * passo)) {
        return Err(format!("array de entradas ilegível ({n} x {passo:#x})"));
    }
    Ok(Some(
        (0..n)
            .map(|i| {
                let e = dados.add(i * passo);
                Entrada {
                    primaria: (e.add(o_p) as *const u64).read_unaligned(),
                    secundaria: cstring_texto(e.add(o_s)),
                    feminino: e.add(o_f),
                    masculino: e.add(o_m),
                }
            })
            .collect(),
    ))
}

/// Chaves do mapa de textos para uma entrada do recurso de textos do mod.
pub(crate) fn chaves(primaria: u64, secundaria: &str) -> Vec<u64> {
    // Secundária com `-`/`=` na frente é linha de comentário/seção do arquivo de textos, não texto.
    if secundaria.starts_with('-') || secundaria.starts_with('=') {
        return Vec::new();
    }
    if primaria != 0 {
        return vec![primaria];
    }
    if secundaria.is_empty() {
        return Vec::new();
    }
    // Sem chave primária, o texto vale pelas duas formas de hash da secundária: a de 32 bits e a de 64.
    vec![bwms_primitives::fnv1a32(secundaria.as_bytes()) as u64, bwms_primitives::fnv1a64(secundaria.as_bytes())]
}

/// Grava uma entrada de texto como o motor carrega o onscreens: o feminino (ou o único) no mapa `+0x38`; o masculino no `+0x68` só quando a
/// entrada tem um, ou quando a chave já tem masculino no jogo (senão ficaria o texto antigo). O `GetText` da variante 0 procura no `+0x68` e
/// cai no `+0x38` (no pt-br, 70.579 chaves no `+0x38` e 487 no `+0x68`, boot 269). Com `troca = false` (reserva), chave que já está no
/// `+0x38` deixa a entrada inteira como está, as duas variantes:
/// gravar só o masculino daria ao V masculino o texto da reserva e ao feminino o do idioma em uso. Devolve o resultado de cada mapa gravado.
pub(crate) unsafe fn gravar_entrada(repo: *mut u8, chave: u64, feminino: &Texto, masculino: Option<&Texto>, troca: bool) -> Vec<Result<bool, String>> {
    let mut r = vec![gravar(repo, MAPA_VARIANTE_1, chave, feminino, troca)];
    if !troca && matches!(r[0], Ok(false)) {
        return r;
    }
    let ja_tem_masculino = mapas::no(repo.add(MAPA_VARIANTE_0), chave).is_some();
    if masculino.is_some() || ja_tem_masculino {
        r.push(gravar(repo, MAPA_VARIANTE_0, chave, masculino.unwrap_or(feminino), troca));
    }
    r
}

/// `(novas, trocadas, falhas)` depois de gravar as entradas. `reserva` = idioma de reserva (não troca texto que existe).
pub(crate) unsafe fn gravar_entradas(repo: *mut u8, entradas: &[Entrada], reserva: bool) -> (usize, usize, usize) {
    let (mut novas, mut trocadas, mut falhas) = (0, 0, 0);
    for e in entradas {
        let masculino = (!cstring_texto(e.masculino).is_empty()).then_some(Texto::DoMotor(e.masculino));
        for chave in chaves(e.primaria, &e.secundaria) {
            for r in gravar_entrada(repo, chave, &Texto::DoMotor(e.feminino), masculino.as_ref(), !reserva) {
                match r {
                    Ok(true) => novas += 1,
                    Ok(false) => trocadas += 1,
                    Err(_) => falhas += 1,
                }
            }
        }
    }
    (novas, trocadas, falhas)
}

/// Uma linha da tabela gerada na instalação (`red4ext/bwms-localizacao.txt`): `<mod> | <tipo> | <idioma> | <caminho>`. Cópia própria da linha de
/// `bwms_primitives::tabelas::Localizacao`, porque o `tick` guarda a tabela lida uma vez para a sessão inteira.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Linha {
    pub modulo: String,
    pub tipo: String,
    pub idioma: String,
    pub caminho: String,
}

/// As linhas de `bwms_primitives::tabelas::Localizacao` do texto; linha fora do formato fica de fora.
pub(crate) fn ler_tabela(texto: &str) -> Vec<Linha> {
    bwms_primitives::tabelas::ler(texto, bwms_primitives::tabelas::Localizacao::ler)
        .flatten()
        .map(|l| Linha { modulo: l.mod_id.into(), tipo: l.tipo.into(), idioma: l.idioma.into(), caminho: l.caminho.into() })
        .collect()
}

/// Recursos de texto a aplicar no idioma `idioma`: `(caminho, é_reserva)`.
pub(crate) fn recursos_do_idioma(tabela: &[Linha], idioma: &str) -> Vec<(String, bool)> {
    // O filtro de tipo vem antes da reserva: linha de tipo que o runtime não aplica não pode escolher o idioma de reserva do mod.
    let aplicadas: Vec<&Linha> = tabela.iter().filter(|l| bwms_primitives::TIPOS_DE_TEXTO_APLICADOS.contains(&l.tipo.as_str())).collect();
    let mut mods: Vec<&str> = Vec::new();
    for l in &aplicadas {
        if !mods.contains(&l.modulo.as_str()) {
            mods.push(&l.modulo);
        }
    }
    let mut saida = Vec::new();
    for m in mods {
        let do_mod: Vec<&&Linha> = aplicadas.iter().filter(|l| l.modulo == m).collect();
        let reserva = do_mod[0].idioma.as_str();
        saida.extend(do_mod.iter().filter(|l| l.idioma == idioma).map(|l| (l.caminho.clone(), false)));
        if reserva != idioma {
            saida.extend(do_mod.iter().filter(|l| l.idioma == reserva).map(|l| (l.caminho.clone(), true)));
        }
    }
    saida
}

/// Idioma dos textos em uso (`/language` `OnScreen` das configurações do jogo), pelo `BwmsIdiomaDosTextos` do script.
pub(crate) unsafe fn idioma_atual() -> Option<String> {
    let player = crate::current_player();
    if player.is_null() {
        return None;
    }
    let rf = crate::rtti::resolve_in_class(crate::rtti::class_of(player), "BwmsIdiomaDosTextos")?;
    let b = crate::rtti::call_func(&rf, player, &[])?;
    let nome = crate::cname::resolve_cname(u64::from_le_bytes(b[0..8].try_into().ok()?));
    (!nome.is_empty() && nome != "None").then_some(nome)
}

/// Última aplicação dos textos de mod: `(idioma, recursos lidos, novas, trocadas, falhas)`.
static APLICADO: std::sync::Mutex<Option<(String, usize, usize, usize, usize)>> = std::sync::Mutex::new(None);

#[cfg(feature = "devtools")]
pub(crate) fn aplicado() -> Option<(String, usize, usize, usize, usize)> {
    APLICADO.lock().ok().and_then(|a| a.clone())
}

/// Em que ponto está a aplicação dos textos de mod: recursos pedidos pro idioma e esperando a carga, ou aplicados (com a 1ª chave aplicada
/// pra saber se o jogo recarregou os textos).
enum Fase {
    Parado,
    Carregando { idioma: String, pedidos: Vec<(String, bool, Result<[u64; 2], String>)>, desde: std::time::Instant },
    Aplicado { idioma: String, sentinela: u64 },
}

/// Aplica os textos de mod do idioma em uso, na thread do jogo, quando os mapas estão carregados: pede os recursos num tick e grava quando
/// todos terminaram (teto de 60 s pra um que não termina); reaplica se o idioma muda ou se o jogo recarregou os textos.
pub(crate) fn tick() {
    static VOLTAS: AtomicU64 = AtomicU64::new(0);
    static FASE: std::sync::Mutex<Fase> = std::sync::Mutex::new(Fase::Parado);
    static TABELA: std::sync::OnceLock<Vec<Linha>> = std::sync::OnceLock::new();
    if !crate::tempo::a_cada(VOLTAS.fetch_add(1, Ordering::Relaxed), 120) || !crate::inicio::boot::gameplay_real() {
        return;
    }
    let tabela = TABELA.get_or_init(|| {
        let dir = crate::arquivos_do_jogo::game_dir_from_dylib().unwrap_or_default();
        let f = std::path::Path::new(&dir).join(bwms_primitives::PASTA_DO_RUNTIME).join(bwms_primitives::tabelas::LOCALIZACAO.arquivo);
        std::fs::read_to_string(f).map(|t| ler_tabela(&t)).unwrap_or_default()
    });
    if tabela.is_empty() {
        return;
    }
    unsafe {
        let (Some(repo), Some(atual)) = (repositorio(), idioma_atual()) else { return };
        let Ok(mut fase) = FASE.lock() else { return };
        match &*fase {
            Fase::Aplicado { idioma, sentinela } if *idioma == atual && mapas::no(repo.add(MAPA_VARIANTE_1), *sentinela).is_some() => return,
            Fase::Carregando { idioma, pedidos, desde } if *idioma == atual => {
                let carregando = pedidos.iter().any(|(_, _, t)| matches!(t, Ok(t) if crate::register::status_do_token(t[0]) == 0));
                if carregando && desde.elapsed().as_secs() < 60 {
                    return;
                }
                let (mut novas, mut trocadas, mut falhas, mut sentinela, mut lidos) = (0, 0, 0, 0u64, 0);
                for (caminho, reserva, token) in pedidos {
                    match token.clone().and_then(|t| entradas_do_token(t)?.ok_or_else(|| "não terminou em 60 s".to_string())) {
                        Ok(entradas) => {
                            if sentinela == 0 {
                                sentinela = entradas.iter().find_map(|e| chaves(e.primaria, &e.secundaria).first().copied()).unwrap_or(0);
                            }
                            let (n, t, f) = gravar_entradas(repo, &entradas, *reserva);
                            (novas, trocadas, falhas, lidos) = (novas + n, trocadas + t, falhas + f, lidos + 1);
                        }
                        Err(e) => crate::log_dev!("[localizacao] \x27{caminho}\x27: {e}"),
                    }
                }
                crate::log_dev!("[localizacao] idioma {atual}: {novas} texto(s) novo(s), {trocadas} trocado(s), {falhas} falha(s)");
                if let Ok(mut a) = APLICADO.lock() {
                    *a = Some((atual.clone(), lidos, novas, trocadas, falhas));
                }
                *fase = Fase::Aplicado { idioma: atual, sentinela };
            }
            _ => {
                let pedidos = recursos_do_idioma(tabela, &atual).into_iter().map(|(c, r)| {
                    let t = pedir_recurso(&c);
                    (c, r, t)
                });
                *fase = Fase::Carregando { idioma: atual, pedidos: pedidos.collect(), desde: std::time::Instant::now() };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaves_da_entrada() {
        assert_eq!(chaves(42, "qualquer"), vec![42]);
        assert!(chaves(0, "").is_empty());
        assert!(chaves(0, "-- comentário").is_empty());
        assert!(chaves(7, "== seção").is_empty());
        let k = chaves(0, "MeuMod-Item-Nome");
        assert_eq!(k.len(), 2);
        assert_eq!(k[1], bwms_primitives::fnv1a64(b"MeuMod-Item-Nome"));
        assert_eq!(k[0], bwms_primitives::fnv1a32(b"MeuMod-Item-Nome") as u64);
    }

    #[test]
    fn tabela_e_reserva_de_idioma() {
        let t = ler_tabela("# cabeçalho\na | onscreens | en-us | a\\en.json\na | onscreens | pt-br | a\\pt.json\nb | onscreens | en-us | b\\en.json\nb | subtitles | pt-br | b\\sub.json\n");
        assert_eq!(t.len(), 4);
        let r = recursos_do_idioma(&t, "pt-br");
        let par = |c: &str, reserva: bool| (c.to_string(), reserva);
        assert_eq!(r, vec![par("a\\pt.json", false), par("a\\en.json", true), par("b\\en.json", true)]);
        // o idioma da reserva não se repete como reserva
        assert_eq!(recursos_do_idioma(&t, "en-us"), vec![par("a\\en.json", false), par("b\\en.json", false)]);
    }

    #[test]
    fn linha_de_tipo_nao_aplicado_nao_muda_o_que_o_runtime_aplica() {
        // O aviso da instalação manda tirar o grupo do `.xl` e diz que o jogo fica igual. Com a linha de subtitles na frente, a reserva sairia
        // dela se o filtro viesse depois da escolha: o resultado tem que ser o mesmo com e sem a linha, em todo idioma.
        let com = ler_tabela("e | subtitles | pt-br | e\\sub.json\ne | onscreens | de-de | e\\de.json\ne | onscreens | pt-br | e\\pt.json\n");
        let sem = ler_tabela("e | onscreens | de-de | e\\de.json\ne | onscreens | pt-br | e\\pt.json\n");
        for idioma in ["pt-br", "de-de", "fr-fr"] {
            assert_eq!(recursos_do_idioma(&com, idioma), recursos_do_idioma(&sem, idioma), "{idioma}");
        }
    }

    #[test]
    fn reserva_e_o_primeiro_idioma_do_mod() {
        // mod sem en-us: num idioma que ele não declara, entra o primeiro declarado, não o en-us
        let t = ler_tabela("c | onscreens | de-de | c\\de.json\nc | onscreens | pt-br | c\\pt.json\nc | onscreens | pt-br | c\\pt2.json\n");
        let par = |c: &str, reserva: bool| (c.to_string(), reserva);
        assert_eq!(recursos_do_idioma(&t, "fr-fr"), vec![par("c\\de.json", true)]);
        assert_eq!(recursos_do_idioma(&t, "pt-br"), vec![par("c\\pt.json", false), par("c\\pt2.json", false), par("c\\de.json", true)]);
        assert_eq!(recursos_do_idioma(&t, "de-de"), vec![par("c\\de.json", false)]);
        // primeiro declarado pt-br, depois en-us: a reserva segue a ordem do `.xl`, não a preferência por en-us nem o menor código
        let t = ler_tabela("d | onscreens | pt-br | d\\pt.json\nd | onscreens | en-us | d\\en.json\n");
        assert_eq!(recursos_do_idioma(&t, "fr-fr"), vec![par("d\\pt.json", true)]);
        assert_eq!(recursos_do_idioma(&t, "en-us"), vec![par("d\\en.json", false), par("d\\pt.json", true)]);
    }

    #[test]
    fn reserva_deixa_a_entrada_inteira_de_chave_que_existe() {
        // `+0x38` com a chave (o idioma em uso só trouxe o feminino) e `+0x68` vazio; a reserva traz feminino e masculino da mesma chave
        let chave = 0x1234_5678_9abc_def0u64;
        let mut tabela = [0u32; 1];
        let mut no = [0u8; 0x30];
        no[0..4].copy_from_slice(&u32::MAX.to_le_bytes());
        no[4..8].copy_from_slice(&mapas::hash_da_chave(chave).to_le_bytes());
        no[8..16].copy_from_slice(&chave.to_le_bytes());
        let mut repo = [0u8; MAPA_VARIANTE_0 + 0x30];
        let m = MAPA_VARIANTE_1;
        repo[m..m + 8].copy_from_slice(&(tabela.as_mut_ptr() as u64).to_le_bytes());
        repo[m + 0x08..m + 0x0c].copy_from_slice(&1u32.to_le_bytes());
        repo[m + 0x0c..m + 0x10].copy_from_slice(&1u32.to_le_bytes());
        repo[m + 0x10..m + 0x18].copy_from_slice(&(no.as_mut_ptr() as u64).to_le_bytes());
        repo[m + 0x18..m + 0x1c].copy_from_slice(&1u32.to_le_bytes());
        repo[m + 0x1c..m + 0x20].copy_from_slice(&0x30u32.to_le_bytes());
        let r = unsafe { gravar_entrada(repo.as_mut_ptr(), chave, &Texto::Proprio("X-en"), Some(&Texto::Proprio("X-en-m")), false) };
        // só o `+0x38`, sem troca; o `+0x68` não é tocado e o `GetText` da variante 0 cai no texto do idioma em uso
        assert_eq!(r, vec![Ok(false)]);
        assert!(repo[MAPA_VARIANTE_0..].iter().all(|b| *b == 0));
    }
}
