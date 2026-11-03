//! Rasterizador por software, só de teste: desenha uma lista de triângulos (posição, uv, cor) num
//! quadro em memória e grava em BMP.
//!
//! POR QUE EXISTE: o terminal tem que sair com o desenho aprovado pelo dono, e a prova natural seria
//! uma foto do jogo, que depende de a máquina do dono ficar ociosa. A nossa `Malha` produz triângulos
//! com uv e cor por vértice ANTES de chegar na GPU, então ela é desenhada aqui, sem jogo, e comparada
//! com a referência em PNG e com a impressão aprovada (`overlay::prova_visual`). O mesmo rasterizador
//! prova que o splash continua idêntico ao pixel depois de mexer na fonte.
//!
//! Não é bonito nem rápido: é exato o bastante pra comparar. Amostra no centro do pixel, interpola
//! uv e cor por baricêntrica, compõe com alfa "over" — o que o pipeline do overlay faz na GPU.

pub(crate) struct Quadro {
    pub(crate) largura: usize,
    pub(crate) altura: usize,
    /// RGBA em 0..1, não premultiplicado.
    pub(crate) px: Vec<[f32; 4]>,
}

impl Quadro {
    pub(crate) fn novo(largura: usize, altura: usize, fundo: [f32; 4]) -> Self {
        Quadro { largura, altura, px: vec![fundo; largura * altura] }
    }
}

pub(crate) struct Textura<'a> {
    pub(crate) largura: usize,
    pub(crate) altura: usize,
    pub(crate) rgba: &'a [u8],
    /// `true` = vizinho mais próximo (fonte de bitmap); `false` = bilinear.
    pub(crate) nitida: bool,
}

impl Textura<'_> {
    fn amostra(&self, u: f32, v: f32) -> [f32; 4] {
        let texel = |x: i64, y: i64| -> [f32; 4] {
            let x = x.clamp(0, self.largura as i64 - 1) as usize;
            let y = y.clamp(0, self.altura as i64 - 1) as usize;
            let i = (y * self.largura + x) * 4;
            let b = &self.rgba[i..i + 4];
            [b[0] as f32 / 255.0, b[1] as f32 / 255.0, b[2] as f32 / 255.0, b[3] as f32 / 255.0]
        };
        let fx = u * self.largura as f32 - 0.5;
        let fy = v * self.altura as f32 - 0.5;
        if self.nitida {
            return texel((fx + 0.5).floor() as i64, (fy + 0.5).floor() as i64);
        }
        let (x0, y0) = (fx.floor() as i64, fy.floor() as i64);
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let (a, b, c, d) = (texel(x0, y0), texel(x0 + 1, y0), texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
        let mut s = [0.0; 4];
        for k in 0..4 {
            let cima = a[k] * (1.0 - tx) + b[k] * tx;
            let baixo = c[k] * (1.0 - tx) + d[k] * tx;
            s[k] = cima * (1.0 - ty) + baixo * ty;
        }
        s
    }
}

/// Um vértice já no formato comum: pixel, uv e cor RGBA 0..255.
#[derive(Clone, Copy)]
pub(crate) struct Ponto {
    pub(crate) pos: [f32; 2],
    pub(crate) uv: [f32; 2],
    pub(crate) cor: [u8; 4],
}

/// Desenha um triângulo, recortado por `recorte` = [x0, y0, x1, y1].
pub(crate) fn triangulo(q: &mut Quadro, t: [Ponto; 3], tex: &Textura, recorte: [f32; 4]) {
    let [a, b, c] = t;
    let area = (b.pos[0] - a.pos[0]) * (c.pos[1] - a.pos[1]) - (b.pos[1] - a.pos[1]) * (c.pos[0] - a.pos[0]);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = a.pos[0].min(b.pos[0]).min(c.pos[0]).max(recorte[0]).max(0.0).floor() as i64;
    let max_x = a.pos[0].max(b.pos[0]).max(c.pos[0]).min(recorte[2]).min(q.largura as f32).ceil() as i64;
    let min_y = a.pos[1].min(b.pos[1]).min(c.pos[1]).max(recorte[1]).max(0.0).floor() as i64;
    let max_y = a.pos[1].max(b.pos[1]).max(c.pos[1]).min(recorte[3]).min(q.altura as f32).ceil() as i64;
    for py in min_y..max_y {
        for px in min_x..max_x {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            if x < recorte[0] || x >= recorte[2] || y < recorte[1] || y >= recorte[3] {
                continue;
            }
            // Baricêntricas pelas funções de aresta; o sinal da área deixa a orientação livre.
            let w0 = ((b.pos[0] - x) * (c.pos[1] - y) - (b.pos[1] - y) * (c.pos[0] - x)) / area;
            let w1 = ((c.pos[0] - x) * (a.pos[1] - y) - (c.pos[1] - y) * (a.pos[0] - x)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                continue;
            }
            let u = a.uv[0] * w0 + b.uv[0] * w1 + c.uv[0] * w2;
            let v = a.uv[1] * w0 + b.uv[1] * w1 + c.uv[1] * w2;
            let mut cor = [0.0f32; 4];
            for k in 0..4 {
                cor[k] = (a.cor[k] as f32 * w0 + b.cor[k] as f32 * w1 + c.cor[k] as f32 * w2) / 255.0;
            }
            let s = tex.amostra(u, v);
            let src = [cor[0] * s[0], cor[1] * s[1], cor[2] * s[2], cor[3] * s[3]];
            let d = &mut q.px[py as usize * q.largura + px as usize];
            let alfa = src[3];
            for k in 0..3 {
                d[k] = src[k] * alfa + d[k] * (1.0 - alfa);
            }
            d[3] = alfa + d[3] * (1.0 - alfa);
        }
    }
}

/// Desenha uma lista indexada inteira com um único recorte.
pub(crate) fn lista(q: &mut Quadro, pontos: &[Ponto], indices: &[u16], tex: &Textura, recorte: [f32; 4]) {
    for tri in indices.chunks_exact(3) {
        let t = [pontos[tri[0] as usize], pontos[tri[1] as usize], pontos[tri[2] as usize]];
        triangulo(q, t, tex, recorte);
    }
}

/// Grava em BMP de 24 bits (o formato mais simples que o Preview e o `sips` abrem).
pub(crate) fn gravar_bmp(caminho: &std::path::Path, q: &Quadro) -> std::io::Result<()> {
    let linha = (q.largura * 3 + 3) & !3;
    let tamanho_dados = linha * q.altura;
    let mut b = Vec::with_capacity(54 + tamanho_dados);
    b.extend_from_slice(b"BM");
    b.extend_from_slice(&((54 + tamanho_dados) as u32).to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&54u32.to_le_bytes());
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&(q.largura as i32).to_le_bytes());
    b.extend_from_slice(&(q.altura as i32).to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(&[0; 4]);
    b.extend_from_slice(&(tamanho_dados as u32).to_le_bytes());
    b.extend_from_slice(&[0; 16]);
    for y in (0..q.altura).rev() {
        let inicio = b.len();
        for x in 0..q.largura {
            let p = q.px[y * q.largura + x];
            let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            b.extend_from_slice(&[byte(p[2]), byte(p[1]), byte(p[0])]);
        }
        b.resize(inicio + linha, 0);
    }
    std::fs::write(caminho, b)
}

/// Resumo exato de um quadro: comparar dois quadros por isto é comparar pixel a pixel.
pub(crate) fn impressao(q: &Quadro) -> u64 {
    let mut h = bwms_primitives::FNV1A64_OFFSET;
    for p in &q.px {
        let bytes = p.map(|c| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
        h = bwms_primitives::fnv1a64_seeded(&bytes, h);
    }
    h
}

/// Junta dois quadros lado a lado, com uma faixa entre eles.
pub(crate) fn lado_a_lado(a: &Quadro, b: &Quadro, faixa: usize, cor_da_faixa: [f32; 4]) -> Quadro {
    let altura = a.altura.max(b.altura);
    let mut q = Quadro::novo(a.largura + faixa + b.largura, altura, cor_da_faixa);
    for y in 0..a.altura {
        for x in 0..a.largura {
            q.px[y * q.largura + x] = a.px[y * a.largura + x];
        }
    }
    for y in 0..b.altura {
        for x in 0..b.largura {
            q.px[y * q.largura + a.largura + faixa + x] = b.px[y * b.largura + x];
        }
    }
    q
}

/// Mapa de diferença entre dois quadros do mesmo tamanho: pinta de vermelho onde algum canal difere
/// mais que `limiar` (0..1) e devolve quantos pixels diferem. É a medida objetiva de "mesmo desenho".
pub(crate) fn diferenca(a: &Quadro, b: &Quadro, limiar: f32) -> (Quadro, usize) {
    let (l, h) = (a.largura.min(b.largura), a.altura.min(b.altura));
    let mut q = Quadro::novo(l, h, [0.0, 0.0, 0.0, 1.0]);
    let mut n = 0;
    for y in 0..h {
        for x in 0..l {
            let (pa, pb) = (a.px[y * a.largura + x], b.px[y * b.largura + x]);
            let d = (0..3).map(|k| (pa[k] - pb[k]).abs()).fold(0.0f32, f32::max);
            let cinza = (pa[0] + pa[1] + pa[2]) / 3.0 * 0.35;
            q.px[y * l + x] = if d > limiar {
                n += 1;
                [1.0, 0.1, 0.1, 1.0]
            } else {
                [cinza, cinza, cinza, 1.0]
            };
        }
    }
    (q, n)
}

#[cfg(test)]
mod teste {
    use super::*;

    #[test]
    fn triangulo_branco_pinta_so_dentro() {
        let branco = [255u8; 4];
        let tex = Textura { largura: 1, altura: 1, rgba: &branco, nitida: true };
        let mut q = Quadro::novo(10, 10, [0.0, 0.0, 0.0, 1.0]);
        let p = |x: f32, y: f32| Ponto { pos: [x, y], uv: [0.5, 0.5], cor: [255, 0, 0, 255] };
        triangulo(&mut q, [p(0.0, 0.0), p(10.0, 0.0), p(0.0, 10.0)], &tex, [0.0, 0.0, 10.0, 10.0]);
        assert_eq!(q.px[0][0], 1.0, "canto dentro tem que ficar vermelho");
        assert_eq!(q.px[99][0], 0.0, "canto oposto fica fora do triângulo");
    }

    #[test]
    fn recorte_impede_desenho_fora() {
        let branco = [255u8; 4];
        let tex = Textura { largura: 1, altura: 1, rgba: &branco, nitida: true };
        let mut q = Quadro::novo(10, 10, [0.0, 0.0, 0.0, 1.0]);
        let p = |x: f32, y: f32| Ponto { pos: [x, y], uv: [0.5, 0.5], cor: [255, 255, 255, 255] };
        triangulo(&mut q, [p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)], &tex, [0.0, 0.0, 5.0, 10.0]);
        assert_eq!(q.px[9][0], 0.0, "fora do recorte não pode pintar");
    }

    #[test]
    fn impressao_muda_com_um_pixel() {
        let mut a = Quadro::novo(4, 4, [0.0, 0.0, 0.0, 1.0]);
        let h0 = impressao(&a);
        a.px[5] = [1.0, 0.0, 0.0, 1.0];
        assert_ne!(h0, impressao(&a));
    }
}
