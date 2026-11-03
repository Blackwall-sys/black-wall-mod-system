//! Splash de boot desenhado direto no Metal, sem biblioteca de desenho de terceiro: três primitivas
//! (retângulo, linha e imagem), uma fonte de 7×13 desenhada no projeto e o mesmo shader e layout de
//! vértice do overlay, então o pipeline é um só.

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::Ordering;

use crate::macos::{classe, msg, msg0, msg_usize, sel, Id, Retido};
use crate::terminal::overlay::{
    novo_buffer, MTL_INDEX_TYPE_UINT16, MTL_LOAD_ACTION_LOAD, MTL_PRIMITIVE_TYPE_TRIANGLE, MTL_STORE_ACTION_STORE,
};

use crate::terminal::malha_de_desenho::*;

// ---------------------------------------------------------------- recursos Metal

/// Pipeline, atlas e a arte do usuário. Criados na primeira vez que o splash desenha e mantidos até
/// o fim do processo (o splash só existe no boot; recriar por frame seria compilar shader por frame).
struct Recursos {
    /// Amostragem NEAREST: a fonte é escalada por inteiro e o filtro linear borraria o glifo.
    pso_nitido: Retido,
    /// Amostragem LINEAR: a arte do usuário é reduzida pra caber na tela.
    pso_suave: Retido,
    atlas: Retido,
    arte: Option<(Retido, f32, f32)>,
    /// Índices de `QUADS_DO_SELO` quads no padrão da `Malha::quad`, montados uma vez: o selo desenha em todo quadro da gameplay.
    indices_do_selo: Retido,
    /// Último quadro composto do splash. As entradas do `compor` mudam no máximo 1×/s; quadro com a mesma chave reusa os buffers em vez
    /// de recompor e criar 2–4 MTLBuffer.
    quadro: Option<QuadroDoSplash>,
    /// Último selo composto, com a chave dele. Quadro com a mesma chave só repinta o pulso: o selo é desenhado em todo quadro da
    /// gameplay e compor aloca.
    selo: Option<(ChaveDoSelo, Malha)>,
}

/// Malha já gravada em buffers imutáveis da GPU.
struct LoteGravado {
    vertices: Retido,
    indices: Retido,
    n_indices: u64,
}

/// `(largura, altura, progresso, segundos)` em bits + a etapa: tudo o que o `compor` lê.
type ChaveDoQuadro = (u32, u32, u32, u64, &'static str);

struct QuadroDoSplash {
    chave: ChaveDoQuadro,
    malha: Option<LoteGravado>,
    imagem: Option<LoteGravado>,
}

/// Quadros que o splash compôs de verdade (a chave mudou). Sai no veredito do fim do splash: o esperado é perto dos segundos de splash.
#[cfg(feature = "devlog")]
pub(crate) static QUADROS_COMPOSTOS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Teto de quads do selo no caminho sem buffer por quadro (o texto cresce com os dígitos da contagem; hoje são até 18 quads).
const QUADS_DO_SELO: usize = 64;

/// `[b, b+1, b+2, b, b+2, b+3]` por quad, o mesmo que a `Malha::quad` grava.
const INDICES_DE_QUADS: [u16; QUADS_DO_SELO * 6] = {
    let mut a = [0u16; QUADS_DO_SELO * 6];
    let mut q = 0;
    while q < QUADS_DO_SELO {
        let b = (q * 4) as u16;
        a[q * 6] = b;
        a[q * 6 + 1] = b + 1;
        a[q * 6 + 2] = b + 2;
        a[q * 6 + 3] = b;
        a[q * 6 + 4] = b + 2;
        a[q * 6 + 5] = b + 3;
        q += 1;
    }
    a
};

struct RecursosEnviaveis(Recursos);
// O `present` chega em várias threads de dispatcher; o Mutex abaixo serializa o acesso, que é o mesmo
// contrato que o renderer do overlay já usa.
unsafe impl Send for RecursosEnviaveis {}
static RECURSOS: std::sync::Mutex<Option<RecursosEnviaveis>> = std::sync::Mutex::new(None);

unsafe fn montar_recursos(dev: Id, pixfmt: u64) -> Option<Recursos> {
    use crate::terminal::overlay::{nova_biblioteca, pipeline_da_malha, textura_do_atlas};
    let lib = nova_biblioteca(dev, crate::terminal::overlay::SHADER).map_err(|e| crate::log_dev!("[splash] shader: {e}")).ok()?;
    let r = Recursos {
        pso_nitido: pipeline_da_malha(dev, lib.0, pixfmt, "f_nearest")?,
        pso_suave: pipeline_da_malha(dev, lib.0, pixfmt, "f")?,
        atlas: textura_do_atlas(dev),
        arte: crate::terminal::overlay::carregar_arte_do_splash(dev),
        indices_do_selo: novo_buffer(dev, INDICES_DE_QUADS.as_ptr() as *const c_void, (INDICES_DE_QUADS.len() * 2) as u64),
        quadro: None,
        selo: None,
    };
    crate::log_dev!("[splash] recursos prontos (atlas {ATLAS_L}x{ATLAS_A}, arte={})", r.arte.is_some());
    Some(r)
}

// ---------------------------------------------------------------- a tela

/// Paleta NEOMILITARISM: o vermelho do projeto sobre quase-preto.
const ESCURO: [u8; 4] = [5, 5, 8, 255];
const VERMELHO: [u8; 4] = [219, 41, 38, 255];
const VERMELHO_FRACO: [u8; 4] = [219, 41, 38, 140];
const CINZA: [u8; 4] = [158, 158, 173, 204];
const TRILHO: [u8; 4] = [255, 255, 255, 20];

/// A marca do projeto, como sempre foi na tela.
const MARCA: &str = r#"      ___           ___           ___           ___
     /\  \         /\__\         /\__\         /\  \
    /::\  \       /:/ _/_       /::|  |       /::\  \
   /:/\:\  \     /:/ /\__\     /:|:|  |      /:/\ \  \
  /::\~\:\__\   /:/ /:/ _/_   /:/|:|__|__   _\:\~\ \  \
 /:/\:\ \:|__| /:/_/:/ /\__\ /:/ |::::\__\ /\ \:\ \ \__\
 \:\~\:\/:/  / \:\/:/ /:/  / \/__/~~/:/  / \:\ \:\ \/__/
  \:\ \::/  /   \::/_/:/  /        /:/  /   \:\ \:\__\
   \:\/:/  /     \:\/:/  /        /:/  /     \:\/:/  /
    \::/__/       \::/  /        /:/  /       \::/  /
     ~~            \/__/         \/__/         \/__/"#;

/// Monta a malha da tela inteira. Separada do Metal de propósito: é função pura de (largura, altura,
/// progresso, segundos, etapa, dimensão da arte) e por isso tem teste e prévia offline.
fn compor(
    largura: f32,
    altura: f32,
    progresso: f32,
    segundos: u64,
    etapa: &str,
    arte: Option<(f32, f32)>,
) -> (Malha, Malha) {
    let mut m = Malha::default();
    let mut imagem = Malha::default();
    m.retangulo(0.0, 0.0, largura, altura, ESCURO);
    // Unidade da tela: espessura e respiro em pixels fixos somem em 4K (a barra de 5 px virou um fio
    // na prévia de 3840×2160), então tudo que não é texto escala com a altura.
    let u = (altura / 720.0).max(1.0);

    // Centro: a arte do usuário, se ela existir; senão, a marca.
    match arte {
        Some((iw, ih)) if iw > 0.0 && ih > 0.0 => {
            let escala = (largura / iw).min(altura / ih);
            let (dw, dh) = (iw * escala, ih * escala);
            imagem.quad(
                (largura - dw) * 0.5,
                (altura - dh) * 0.5,
                (largura + dw) * 0.5,
                (altura + dh) * 0.5,
                [0.0, 0.0, 1.0, 1.0],
                [255, 255, 255, 255],
            );
        }
        _ => {
            let escala = escala_que_cabe(MARCA, largura * 0.52);
            let lm = largura_do_texto(MARCA, escala);
            let am = altura_do_texto(MARCA, escala);
            let x = (largura - lm) * 0.5;
            let y = (altura * 0.44 - am).max(altura * 0.08);
            m.texto(x, y, escala, VERMELHO, MARCA);
            let sub = "B L A C K   W A L L   M O D   S Y S T E M";
            let esub = escala_que_cabe(sub, largura * 0.46).min(escala).max(1.0);
            let ysub = y + am + 10.0 * u;
            m.texto((largura - largura_do_texto(sub, esub)) * 0.5, ysub, esub, CINZA, sub);
            let ylinha = ysub + altura_do_texto(sub, esub) + 6.0 * u;
            let meia = largura_do_texto(sub, esub) * 0.5;
            m.linha(largura * 0.5 - meia, ylinha, largura * 0.5 + meia, ylinha, 2.0 * u, VERMELHO_FRACO);
        }
    }

    // Promo NEONSYNC (o jogo do autor), no mesmo lugar e na mesma ordem de sempre.
    let epromo = escala_que_cabe("Enjoy being a netrunner?", largura * 0.26);
    let mut y = altura * 0.62;
    for (txt, esc, cor) in [
        ("Enjoy being a netrunner?", epromo, CINZA),
        ("NEONSYNC", escala_que_cabe("NEONSYNC", largura * 0.20), VERMELHO),
        ("Search on Steam", epromo, CINZA),
    ] {
        m.texto((largura - largura_do_texto(txt, esc)) * 0.5, y, esc, cor, txt);
        y += altura_do_texto(txt, esc) + 8.0 * u;
    }

    // Barra de progresso e as duas legendas.
    let barra_l = (largura * 0.42).min(560.0 * u);
    let barra_a = 5.0 * u;
    let bx = (largura - barra_l) * 0.5;
    let by = altura * 0.86;
    m.retangulo(bx, by, bx + barra_l, by + barra_a, TRILHO);
    m.retangulo(bx, by, bx + barra_l * progresso.clamp(0.0, 1.0), by + barra_a, VERMELHO);
    let pct = (progresso.clamp(0.0, 1.0) * 100.0) as u32;
    // `*` é o ponto-médio desta fonte (o atlas é indexado por ASCII, então `·` não entra).
    let legenda = format!("CARREGANDO   {pct}%   *   {etapa}   *   {segundos}s");
    let eleg = escala_que_cabe(&legenda, largura * 0.34);
    let aleg = altura_do_texto(&legenda, eleg);
    m.texto((largura - largura_do_texto(&legenda, eleg)) * 0.5, by - aleg - 8.0 * u, eleg, CINZA, &legenda);
    let versao = format!("v{}  BETA", crate::BWMS_VERSION);
    m.texto((largura - largura_do_texto(&versao, eleg)) * 0.5, by + barra_a + 8.0 * u, eleg, CINZA, &versao);
    (m, imagem)
}

/// O drawable do frame, resolvido uma vez: device, command buffer, textura, tamanho e formato. O
/// splash e o selo pedem exatamente isto.
unsafe fn alvo_do_frame(cb_raw: Id, drawable: Id) -> Option<(Id, Id, Id, f32, f32, u64)> {
    if drawable.is_null() {
        return None;
    }
    let tex_raw = msg0(drawable, sel!("texture"));
    if tex_raw.is_null() {
        return None;
    }
    let largura = msg_usize(tex_raw, sel!("width")) as f32;
    let altura = msg_usize(tex_raw, sel!("height")) as f32;
    if largura < 1.0 || altura < 1.0 {
        return None;
    }
    let dev_raw = msg0(cb_raw, sel!("device"));
    if dev_raw.is_null() {
        return None;
    }
    Some((dev_raw, cb_raw, tex_raw, largura, altura, msg_usize(tex_raw, sel!("pixelFormat")) as u64))
}

/// Passe que preserva o frame do jogo (`Load`) e escreve por cima dele, com a projeção ortográfica
/// de pixels (origem no canto superior esquerdo) já ligada. `None` se o Metal não deu o descritor da cor 0.
unsafe fn abrir_passe(cb: Id, tex: Id, largura: f32, altura: f32) -> Option<Id> {
    let rpd = msg0(classe!("MTLRenderPassDescriptor"), sel!("renderPassDescriptor"));
    let att = msg!(msg0(rpd, sel!("colorAttachments")), "objectAtIndexedSubscript:", u64 = 0 => Id);
    if att.is_null() {
        return None;
    }
    msg!(att, "setTexture:", Id = tex);
    msg!(att, "setLoadAction:", u64 = MTL_LOAD_ACTION_LOAD);
    msg!(att, "setStoreAction:", u64 = MTL_STORE_ACTION_STORE);
    let enc = msg!(cb, "renderCommandEncoderWithDescriptor:", Id = rpd => Id);
    let proj: [[f32; 4]; 4] = [
        [2.0 / largura, 0.0, 0.0, 0.0],
        [0.0, -2.0 / altura, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0, 1.0],
    ];
    msg!(enc, "setVertexBytes:length:atIndex:", *const c_void = proj.as_ptr() as *const c_void, u64 = 64, u64 = 1);
    Some(enc)
}

/// A malha em buffers da GPU; `None` sem índices (nada a desenhar).
unsafe fn gravar_lote(dev: Id, m: &Malha) -> Option<LoteGravado> {
    if m.indices.is_empty() {
        return None;
    }
    let vertices = novo_buffer(dev, m.vertices.as_ptr() as *const c_void, (m.vertices.len() * size_of::<Vertice>()) as u64);
    let indices = novo_buffer(dev, m.indices.as_ptr() as *const c_void, (m.indices.len() * 2) as u64);
    Some(LoteGravado { vertices, indices, n_indices: m.indices.len() as u64 })
}

/// Um lote = um pipeline + uma textura + a malha gravada.
unsafe fn desenhar_lote(enc: Id, l: &LoteGravado, pso: &Retido, textura: &Retido) {
    msg!(enc, "setRenderPipelineState:", Id = pso.0);
    msg!(enc, "setFragmentTexture:atIndex:", Id = textura.0, u64 = 0);
    msg!(enc, "setVertexBuffer:offset:atIndex:", Id = l.vertices.0, u64 = 0, u64 = 0);
    msg!(
        enc,
        "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:",
        u64 = MTL_PRIMITIVE_TYPE_TRIANGLE,
        u64 = l.n_indices,
        u64 = MTL_INDEX_TYPE_UINT16,
        Id = l.indices.0,
        u64 = 0
    );
}

/// Desenha o splash no drawable do frame. Chamado do `present`, na thread de render.
pub(crate) unsafe fn desenhar(cb_raw: Id, drawable: Id) {
    let Some((dev, cb, tex, largura, altura, pixfmt)) = alvo_do_frame(cb_raw, drawable) else { return };
    let mut guarda = match RECURSOS.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guarda.is_none() {
        *guarda = montar_recursos(dev, pixfmt).map(RecursosEnviaveis);
    }
    let Some(r) = guarda.as_mut().map(|x| &mut x.0) else { return };
    let chave: ChaveDoQuadro = (
        largura.to_bits(),
        altura.to_bits(),
        crate::inicio::boot::boot_progress().to_bits(),
        crate::inicio::boot::boot_elapsed_secs(),
        crate::inicio::boot::boot_stage_label(),
    );
    if r.quadro.as_ref().map_or(true, |q| q.chave != chave) {
        let (malha, imagem) = compor(largura, altura, f32::from_bits(chave.2), chave.3, chave.4, r.arte.as_ref().map(|(_, l, a)| (*l, *a)));
        r.quadro = Some(QuadroDoSplash { chave, malha: gravar_lote(dev, &malha), imagem: gravar_lote(dev, &imagem) });
        #[cfg(feature = "devlog")]
        QUADROS_COMPOSTOS.fetch_add(1, Ordering::Relaxed);
    }
    let Some(q) = r.quadro.as_ref() else { return };
    let Some(enc) = abrir_passe(cb, tex, largura, altura) else { return };
    // A arte primeiro: o texto e a barra ficam por cima dela.
    if let (Some((t, _, _)), Some(l)) = (r.arte.as_ref(), q.imagem.as_ref()) {
        desenhar_lote(enc, l, &r.pso_suave, t);
    }
    if let Some(l) = q.malha.as_ref() {
        desenhar_lote(enc, l, &r.pso_nitido, &r.atlas);
    }
    msg!(enc, "endEncoding");
    // Prova de pixel no dev (`terminal::overlay::sonda_de_pixels`): a malha deste quadro, recomposta só quando a sonda pede.
    #[cfg(any(feature = "devtools", test))]
    if crate::terminal::overlay::sonda_de_pixels::quer("splash") {
        let (m, _) = compor(largura, altura, f32::from_bits(chave.2), chave.3, chave.4, r.arte.as_ref().map(|(_, l, a)| (*l, *a)));
        crate::terminal::overlay::sonda_de_pixels::anotar_malha("splash", &m, 1.0);
    }
    PRIMEIRO_DESENHO.store(true, Ordering::Relaxed);
}

/// O splash já desenhou pelo menos um frame? Verificável do log de boot, sem screenshot.
pub(crate) static PRIMEIRO_DESENHO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// ---------------------------------------------------------------- selo de runtime vivo

const VERDE: [u8; 4] = [64, 255, 90, 255];
const VERDE_FRACO: [u8; 4] = [31, 140, 46, 191];
const AMARELO: [u8; 4] = [255, 217, 0, 255];
const VERMELHO_ERRO: [u8; 4] = [255, 51, 51, 255];
const CINZA_INATIVO: [u8; 4] = [128, 128, 128, 204];
const FUNDO_SELO: [u8; 4] = [0, 0, 0, 56];

/// A cor de cada luz de mod, na ordem de `ModStatus` (verde, amarelo, vermelho, cinza): as do selo, usadas também pela aba Mods do terminal.
pub(crate) const CORES_DAS_LUZES: [[u8; 4]; 4] = [VERDE, AMARELO, VERMELHO_ERRO, CINZA_INATIVO];

/// `(largura em bits, contagem de mods)`: tudo o que muda a geometria e o texto do selo. O pulso fica fora: ele só troca a cor de
/// `VERTICES_DO_PULSO`.
type ChaveDoSelo = (u32, (usize, usize, usize, usize));

/// O quad do pulso é o 2º da malha do selo, logo depois do fundo: sem recorte, a `Malha::quad` não descarta nenhum.
const VERTICES_DO_PULSO: std::ops::Range<usize> = 4..8;

#[cfg(test)]
thread_local! {
    /// Composições do selo nesta thread (cada teste roda na sua): prova que quadro com a mesma chave não recompõe.
    static SELOS_COMPOSTOS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// O selo do canto ("o runtime está vivo"): o pulso que alterna com os ticks e a contagem de mods por
/// estado. Dois retângulos e um texto curto, nas primitivas do projeto — o selo é do produto, não do
/// console de dev. Compõe em `m` reaproveitando a capacidade dela; a cor do pulso é a do `pintar_pulso`.
fn compor_selo(m: &mut Malha, largura: f32, contagem: (usize, usize, usize, usize)) {
    m.vertices.clear();
    m.indices.clear();
    let escala = ((largura / 1600.0).round()).max(1.0);
    let (ok, aviso, erro, inativo) = contagem;
    let mut partes: Vec<([u8; 4], String)> = Vec::new();
    if ok + aviso + erro + inativo == 0 {
        partes.push((CINZA_INATIVO, "0".to_string()));
    } else {
        for (cor, n) in [(VERDE, ok), (AMARELO, aviso), (VERMELHO_ERRO, erro), (CINZA_INATIVO, inativo)] {
            if n > 0 {
                partes.push((cor, format!("*{n}")));
            }
        }
    }
    let texto: String = partes.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>().join(" ");
    let lado = ALTURA_GLIFO as f32 * escala * 0.55;
    let pad = 6.0 * escala;
    let lt = largura_do_texto(&texto, escala);
    let at = altura_do_texto(&texto, escala);
    let caixa_l = lado + pad * 3.0 + lt;
    let caixa_a = at.max(lado) + pad;
    let x1 = largura - 14.0 * escala;
    let x0 = x1 - caixa_l;
    let y0 = 14.0 * escala;
    m.retangulo(x0, y0, x1, y0 + caixa_a, FUNDO_SELO);
    let py = y0 + (caixa_a - lado) * 0.5;
    m.retangulo(x0 + pad, py, x0 + pad + lado, py + lado, VERDE);
    let mut cx = x0 + pad * 2.0 + lado;
    let ty = y0 + (caixa_a - at) * 0.5;
    for (cor, t) in &partes {
        m.texto(cx, ty, escala, *cor, t);
        cx += largura_do_texto(t, escala) + CELULA_L as f32 * escala;
    }
}

/// O pulso alterna a cada 6 ticks.
fn pintar_pulso(m: &mut Malha, ticks: u64) {
    let cor = if (ticks / 6) % 2 == 0 { VERDE } else { VERDE_FRACO };
    if let Some(pulso) = m.vertices.get_mut(VERTICES_DO_PULSO) {
        for v in pulso {
            v.cor = cor;
        }
    }
}

/// Deixa em `selo` a malha deste quadro: recompõe só quando a chave muda e repinta o pulso.
fn preparar_selo(selo: &mut Option<(ChaveDoSelo, Malha)>, largura: f32, ticks: u64, contagem: (usize, usize, usize, usize)) {
    let chave: ChaveDoSelo = (largura.to_bits(), contagem);
    if selo.as_ref().map_or(true, |(c, _)| *c != chave) {
        let mut m = selo.take().map(|(_, m)| m).unwrap_or_default();
        compor_selo(&mut m, largura, contagem);
        *selo = Some((chave, m));
        #[cfg(test)]
        SELOS_COMPOSTOS.with(|n| n.set(n.get() + 1));
    }
    if let Some((_, m)) = selo {
        pintar_pulso(m, ticks);
    }
}

/// Desenha o selo do canto no drawable do frame (mesmo caminho do splash).
pub(crate) unsafe fn desenhar_selo(cb_raw: Id, drawable: Id) {
    let Some((dev, cb, tex, largura, altura, pixfmt)) = alvo_do_frame(cb_raw, drawable) else { return };
    let mut guarda = match RECURSOS.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if guarda.is_none() {
        *guarda = montar_recursos(dev, pixfmt).map(RecursosEnviaveis);
    }
    let Some(r) = guarda.as_mut().map(|x| &mut x.0) else { return };
    preparar_selo(&mut r.selo, largura, crate::ticks(), crate::mods_do_usuario::mod_counts());
    let Some((_, m)) = r.selo.as_ref() else { return };
    if m.indices.is_empty() {
        return;
    }
    let Some(enc) = abrir_passe(cb, tex, largura, altura) else { return };
    // Sem MTLBuffer por quadro: os vértices vão copiados no encoder (`setVertexBytes`, teto de 4 KB) e os índices são o buffer fixo
    // dos recursos. Malha maior que isso, ou com índice fora do padrão de quads, cai no lote comum (mesmos pixels).
    let bytes = m.vertices.len() * size_of::<Vertice>();
    let so_quads = m.indices.len() <= INDICES_DE_QUADS.len() && m.indices[..] == INDICES_DE_QUADS[..m.indices.len()];
    // Prova do caminho pelo próprio processo, sem captura de tela.
    static SELO_RELATADO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !SELO_RELATADO.swap(true, Ordering::Relaxed) {
        crate::veredito_dev!(
            "[selo] 1º quadro {largura}x{altura}: {} índices, {bytes} B de vértices, caminho {}",
            m.indices.len(),
            if bytes <= 4096 && so_quads { "setVertexBytes" } else { "lote" }
        );
    }
    if bytes <= 4096 && so_quads {
        msg!(enc, "setRenderPipelineState:", Id = r.pso_nitido.0);
        msg!(enc, "setFragmentTexture:atIndex:", Id = r.atlas.0, u64 = 0);
        msg!(enc, "setVertexBytes:length:atIndex:", *const c_void = m.vertices.as_ptr() as *const c_void, u64 = bytes as u64, u64 = 0);
        msg!(
            enc,
            "drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:",
            u64 = MTL_PRIMITIVE_TYPE_TRIANGLE,
            u64 = m.indices.len() as u64,
            u64 = MTL_INDEX_TYPE_UINT16,
            Id = r.indices_do_selo.0,
            u64 = 0
        );
    } else if let Some(l) = gravar_lote(dev, m) {
        desenhar_lote(enc, &l, &r.pso_nitido, &r.atlas);
    }
    msg!(enc, "endEncoding");
    #[cfg(any(feature = "devtools", test))]
    if crate::terminal::overlay::sonda_de_pixels::quer("selo") {
        crate::terminal::overlay::sonda_de_pixels::anotar_malha("selo", m, 1.0);
    }
}

#[cfg(test)]
mod teste {
    /// O selo vai pelo caminho sem buffer por quadro: só quads (os índices fixos servem) e vértices dentro dos 4 KB do `setVertexBytes`.
    #[test]
    fn selo_cabe_no_caminho_sem_buffer() {
        for contagem in [(0, 0, 0, 0), (3, 0, 0, 0), (12, 4, 1, 7), (999, 999, 999, 999)] {
            for largura in [1280.0, 3840.0] {
                let mut m = crate::terminal::malha_de_desenho::Malha::default();
                super::compor_selo(&mut m, largura, contagem);
                let bytes = m.vertices.len() * std::mem::size_of::<crate::terminal::malha_de_desenho::Vertice>();
                assert!(bytes <= 4096, "selo com {bytes} B em {contagem:?}");
                assert_eq!(m.indices[..], super::INDICES_DE_QUADS[..m.indices.len()], "índice fora do padrão de quads em {contagem:?}");
            }
        }
    }

    /// O selo num quadro de `largura`×200 sobre cinza: o fundo do selo é translúcido e some sobre preto.
    fn selo_rasterizado(m: &crate::terminal::malha_de_desenho::Malha, largura: usize) -> crate::terminal::rasterizador_de_prova::Quadro {
        use crate::terminal::rasterizador_de_prova::*;
        let altura = 200usize;
        let atlas = crate::terminal::malha_de_desenho::montar_atlas();
        let tex = Textura { largura: crate::terminal::malha_de_desenho::ATLAS_L, altura: crate::terminal::malha_de_desenho::ATLAS_A, rgba: &atlas, nitida: true };
        let pontos: Vec<Ponto> = m.vertices.iter().map(|v| Ponto { pos: v.pos, uv: v.uv, cor: v.cor }).collect();
        let mut q = Quadro::novo(largura, altura, [0.5, 0.5, 0.5, 1.0]);
        lista(&mut q, &pontos, &m.indices, &tex, [0.0, 0.0, largura as f32, altura as f32]);
        q
    }

    /// `(largura, contagem, impressão com o pulso aceso, impressão com o pulso fraco)` pelo `selo_rasterizado`: o contrato do desenho
    /// do selo. Qualquer pixel mudado muda o número.
    const IMPRESSOES_DO_SELO: [(usize, (usize, usize, usize, usize), u64, u64); 4] = [
        (3840, (999, 999, 999, 999), 0x300f38b8afdb767f, 0x5b3a43c25cadcd1f),
        (1280, (0, 0, 0, 0), 0xad6bd786dd91c585, 0xc3497f75598bf9b9),
        (2560, (12, 4, 1, 7), 0xcb2b2407fe05a1fc, 0x61c9d00714ca600c),
        (1920, (3, 0, 0, 0), 0x88ab30eba13be0ef, 0xef1b1213a0c1511f),
    ];

    /// O selo guardado, com o pulso repintado, sai com os pixels do contrato nas duas fases do pulso e na volta à primeira. Uma malha
    /// só passa por todos os casos, a maior primeiro: recompor em cima de malha maior não deixa sobra.
    #[test]
    fn selo_guardado_tem_os_pixels_do_contrato() {
        use crate::terminal::rasterizador_de_prova::impressao;
        let mut selo = None;
        for (largura, contagem, aceso, fraco) in IMPRESSOES_DO_SELO {
            for (ticks, esperada) in [(0, aceso), (5, aceso), (6, fraco), (11, fraco), (12, aceso)] {
                super::preparar_selo(&mut selo, largura as f32, ticks, contagem);
                let Some((_, m)) = selo.as_ref() else { panic!("preparar_selo deixou o selo vazio") };
                let agora = impressao(&selo_rasterizado(m, largura));
                assert_eq!(agora, esperada, "selo {largura} {contagem:?} no tick {ticks}: {agora:#018x}");
            }
        }
    }

    /// Quadro com a mesma chave não recompõe; mudar a contagem ou a largura recompõe uma vez.
    #[test]
    fn selo_recompoe_so_quando_a_chave_muda() {
        let compostos = || super::SELOS_COMPOSTOS.with(|n| n.get());
        let inicio = compostos();
        let mut selo = None;
        for ticks in 0..600 {
            super::preparar_selo(&mut selo, 1920.0, ticks, (3, 0, 0, 0));
        }
        assert_eq!(compostos() - inicio, 1, "600 quadros com a mesma chave");
        super::preparar_selo(&mut selo, 1920.0, 600, (3, 1, 0, 0));
        assert_eq!(compostos() - inicio, 2, "a contagem mudou");
        super::preparar_selo(&mut selo, 2560.0, 601, (3, 1, 0, 0));
        super::preparar_selo(&mut selo, 2560.0, 602, (3, 1, 0, 0));
        assert_eq!(compostos() - inicio, 3, "a largura mudou");
    }

    /// A composição é pura: dá pra conferir a tela inteira sem Metal e sem jogo.
    #[test]
    fn compoe_a_tela_inteira() {
        for (l, a) in [(1280.0, 720.0), (1920.0, 1080.0), (3840.0, 2160.0), (1024.0, 768.0)] {
            let (m, img) = super::compor(l, a, 0.42, 37, "assets", None);
            assert!(m.indices.len() % 6 == 0, "índices não fecham quads");
            assert!(m.vertices.len() < u16::MAX as usize, "estourou o índice de 16 bits");
            assert!(img.indices.is_empty(), "sem arte, o lote da imagem tem que ficar vazio");
            for v in &m.vertices {
                assert!(v.pos[0] >= -1.0 && v.pos[0] <= l + 1.0, "x fora da tela: {} em {l}x{a}", v.pos[0]);
                assert!(v.pos[1] >= -1.0 && v.pos[1] <= a + 1.0, "y fora da tela: {} em {l}x{a}", v.pos[1]);
            }
        }
    }

    #[test]
    fn com_arte_o_lote_da_imagem_preserva_o_aspecto() {
        let (_, img) = super::compor(1920.0, 1080.0, 1.0, 5, "menu", Some((800.0, 400.0)));
        assert_eq!(img.indices.len(), 6, "a arte é um quad só");
        let (x0, y0) = (img.vertices[0].pos[0], img.vertices[0].pos[1]);
        let (x1, y1) = (img.vertices[2].pos[0], img.vertices[2].pos[1]);
        let (l, a) = (x1 - x0, y1 - y0);
        assert!((l / a - 2.0).abs() < 0.01, "aspecto 2:1 virou {:.3}", l / a);
        assert!(l <= 1920.5 && a <= 1080.5, "a arte passou da tela");
        assert!((x0 + x1 - 1920.0).abs() < 0.5 && (y0 + y1 - 1080.0).abs() < 0.5, "arte fora do centro");
    }

    #[test]
    fn o_atlas_tem_glifo_aceso_e_a_celula_branca() {
        let px = super::montar_atlas();
        assert_eq!(px.len(), super::ATLAS_L * super::ATLAS_A * 4);
        let i = 65 - super::PRIMEIRO_CODIGO; // 'A'
        let acesos = (0..super::ALTURA_GLIFO)
            .flat_map(|r| (0..super::LARGURA_GLIFO).map(move |c| (r, c)))
            .filter(|(r, c)| px[((r * super::ATLAS_L) + i * super::CELULA_L + c) * 4 + 3] == 255)
            .count();
        assert!(acesos >= 12, "o 'A' do atlas não acendeu ({acesos} pixels)");
        let o = ((2 * super::ATLAS_L) + super::CELULA_BRANCA * super::CELULA_L + 2) * 4;
        assert_eq!(&px[o..o + 4], &[255, 255, 255, 255], "célula branca apagada");
    }

    /// A marca tem que caber e ficar em escala INTEIRA em toda resolução que o jogo abre.
    #[test]
    fn a_marca_cabe_em_toda_resolucao() {
        for (l, _a) in [(1280.0, 720.0), (1440.0, 900.0), (1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
            let e = super::escala_que_cabe(super::MARCA, l * 0.52);
            assert!(e >= 1.0 && e.fract() == 0.0, "escala não-inteira em {l}: {e}");
            assert!(super::largura_do_texto(super::MARCA, e) <= l * 0.55, "marca estourou em {l}");
        }
    }

    /// Rasteriza a tela num PPM pra conferir o desenho offline, sem jogo: os quads são alinhados aos
    /// eixos, então varrer pixel a pixel e amostrar o atlas (nearest) reproduz o que o Metal desenha.
    #[test]
    fn previa_em_arquivo() {
        let Ok(saida) = std::env::var("BWMS_PREVIA_SPLASH") else { return };
        let (l, a) = match std::env::var("BWMS_PREVIA_RES").ok().and_then(|r| {
            let (x, y) = r.split_once('x')?;
            Some((x.parse::<usize>().ok()?, y.parse::<usize>().ok()?))
        }) {
            Some(v) => v,
            None => (1280, 720),
        };
        let (m, _) = super::compor(l as f32, a as f32, 0.62, 41, "assets", None);
        let atlas = super::montar_atlas();
        let mut px = vec![0u8; l * a * 3];
        for q in m.indices.chunks(6) {
            let v = &m.vertices[q[0] as usize..q[0] as usize + 4];
            let (x0, y0) = (v[0].pos[0], v[0].pos[1]);
            let (x1, y1) = (v[2].pos[0], v[2].pos[1]);
            let (u0, tv0) = (v[0].uv[0], v[0].uv[1]);
            let (u1, tv1) = (v[2].uv[0], v[2].uv[1]);
            let cor = v[0].cor;
            for y in (y0.floor().max(0.0) as usize)..((y1.ceil() as usize).min(a)) {
                for x in (x0.floor().max(0.0) as usize)..((x1.ceil() as usize).min(l)) {
                    let fx = if x1 > x0 { (x as f32 + 0.5 - x0) / (x1 - x0) } else { 0.5 };
                    let fy = if y1 > y0 { (y as f32 + 0.5 - y0) / (y1 - y0) } else { 0.5 };
                    let ax = (((u0 + fx * (u1 - u0)) * super::ATLAS_L as f32) as usize).min(super::ATLAS_L - 1);
                    let ay = (((tv0 + fy * (tv1 - tv0)) * super::ATLAS_A as f32) as usize).min(super::ATLAS_A - 1);
                    let alfa = atlas[(ay * super::ATLAS_L + ax) * 4 + 3] as f32 / 255.0 * cor[3] as f32 / 255.0;
                    if alfa <= 0.0 {
                        continue;
                    }
                    let o = (y * l + x) * 3;
                    for c in 0..3 {
                        px[o + c] = (cor[c] as f32 * alfa + px[o + c] as f32 * (1.0 - alfa)) as u8;
                    }
                }
            }
        }
        let mut arq = format!("P6\n{l} {a}\n255\n").into_bytes();
        arq.extend_from_slice(&px);
        std::fs::write(&saida, arq).expect("gravar a prévia");
        println!("[previa] {saida} ({l}x{a}, {} quads)", m.indices.len() / 6);
    }

    /// O splash desenhado pelo rasterizador de prova, nas quatro etapas do boot. A impressão é o
    /// contrato: qualquer mudança na fonte ou no atlas que troque UM pixel do splash muda o número.
    fn splash_rasterizado(etapa: &str) -> crate::terminal::rasterizador_de_prova::Quadro {
        use crate::terminal::rasterizador_de_prova::*;
        let (l, a) = (1280usize, 720usize);
        let (m, _) = super::compor(l as f32, a as f32, 0.42, 37, etapa, None);
        let atlas = crate::terminal::malha_de_desenho::montar_atlas();
        let tex = Textura { largura: crate::terminal::malha_de_desenho::ATLAS_L, altura: crate::terminal::malha_de_desenho::ATLAS_A, rgba: &atlas, nitida: true };
        let pontos: Vec<Ponto> = m.vertices.iter().map(|v| Ponto { pos: v.pos, uv: v.uv, cor: v.cor }).collect();
        let mut q = Quadro::novo(l, a, [0.0, 0.0, 0.0, 1.0]);
        lista(&mut q, &pontos, &m.indices, &tex, [0.0, 0.0, l as f32, a as f32]);
        q
    }

    const ETAPAS: [&str; 4] = ["carregando dados do jogo", "iniciando a sessão", "carregando o save", "carregando o mundo"];

    /// Rodar: `BWMS_PROVA_VISUAL=<pasta> cargo test --features devtools,capture splash_impressao -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn splash_impressao() {
        let dir = std::env::var("BWMS_PROVA_VISUAL").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        for (i, e) in ETAPAS.iter().enumerate() {
            let q = splash_rasterizado(e);
            crate::terminal::rasterizador_de_prova::gravar_bmp(&dir.join(format!("splash-etapa{i}.bmp")), &q).expect("bmp");
            println!("[splash-impressao] etapa{i} {:016x}  ({e})", crate::terminal::rasterizador_de_prova::impressao(&q));
        }
    }
}
