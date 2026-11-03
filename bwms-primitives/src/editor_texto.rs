//! Modelo de edição de texto do BWMS — núcleo puro para o gap Codeware `#115` (`TextInput`).
//!
//! **Por que existe, e por que não é porte.** As 4 sub-partes que o BWMS já tinha para esse item
//! (`Caret`/`Selection`/`TextFlow`/`TextMeasurer`) foram para `quarentena-nao-original/`: eram
//! transliteração do Codeware. A capacidade é desejada, o corpo não pode viajar no produto. Este
//! módulo reprojeta a camada do zero, com decisões próprias e deliberadamente diferentes:
//!
//! - **Um só estado, não quatro objetos.** Cursor e seleção vivem no mesmo lugar, porque toda
//!   operação de edição mexe nos dois — separá-los só cria estados que podem discordar.
//! - **Seleção é âncora + cursor**, não um par ordenado. É o que o Shift+seta de fato produz;
//!   ordenar vira detalhe de leitura (`intervalo()`), não invariante a manter na escrita.
//! - **Índices em codepoint (`char`), nunca byte.** O projeto já quebrou fatiando byte no meio de um
//!   codepoint. Aqui isso é impossível por construção: o buffer é `Vec<char>`. Grafema de vários
//!   codepoints (letra + acento decomposto, bandeira, emoji com ZWJ) não é unidade: anda e apaga por partes.
//! - **Toda operação é total.** Nada panica, nada retorna erro: índice fora de faixa satura no
//!   limite. Widget de UI não tem a quem propagar erro — comportamento definido em 100% das
//!   entradas é a única postura segura.
//!
//! O layout visual (medição) fica de fora de propósito: depende de widget montado e não é lógica pura (`bwms-text-measurer.reds`).

/// Buffer de texto com cursor e seleção. Unidade é o codepoint (`char`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EditorTexto {
    chars: Vec<char>,
    cursor: usize,
    /// `Some(i)` = seleção ativa de `i` até `cursor`. `None` = sem seleção.
    ancora: Option<usize>,
}

/// Para onde uma navegação leva o cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Movimento {
    Esquerda,
    Direita,
    PalavraEsquerda,
    PalavraDireita,
    Inicio,
    Fim,
}

/// Bit do retorno dos mutadores: o conteúdo mudou. Quem desenha remede e avisa quem ouve.
pub const MUDOU_TEXTO: u8 = 1;
/// Bit do retorno dos mutadores: o cursor ou o intervalo selecionado mudou, então o caret e a
/// seleção se redesenham. Âncora sozinha não conta: âncora igual ao cursor é seleção vazia
/// (`intervalo()`). Dois bits e não um `bool`: o layout precisa de "caret mudou" e o aviso de
/// mudança precisa de "texto mudou", e digitar sobre uma seleção o mesmo texto muda só o caret.
pub const MUDOU_CARET: u8 = 2;

/// Um codepoint é separador de palavra? Espaço em branco e pontuação ASCII contam.
/// Deixado público porque a navegação por palavra é a única regra aqui que é escolha
/// de gosto, não consequência do modelo — quem quiser outra política enxerga esta.
pub fn eh_separador(c: char) -> bool {
    c.is_whitespace() || (c.is_ascii_punctuation() && c != '_')
}

impl EditorTexto {
    pub fn novo() -> Self {
        Self::default()
    }

    pub fn de_texto(t: &str) -> Self {
        let chars: Vec<char> = t.chars().collect();
        let cursor = chars.len();
        Self { chars, cursor, ancora: None }
    }

    pub fn texto(&self) -> String {
        self.chars.iter().collect()
    }

    /// Comprimento em codepoints (`char`) — não em bytes.
    pub fn comprimento(&self) -> usize {
        self.chars.len()
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn tem_selecao(&self) -> bool {
        self.intervalo().is_some()
    }

    /// Intervalo selecionado como `(inicio, fim)` ordenado, meio-aberto. `None` se vazio.
    /// Âncora igual ao cursor é seleção VAZIA, não seleção de 1 codepoint.
    pub fn intervalo(&self) -> Option<(usize, usize)> {
        let a = self.ancora?;
        if a == self.cursor {
            return None;
        }
        Some((a.min(self.cursor), a.max(self.cursor)))
    }

    pub fn texto_selecionado(&self) -> String {
        match self.intervalo() {
            Some((i, f)) => self.chars[i..f].iter().collect(),
            None => String::new(),
        }
    }

    /// O que o desenho mostra do estado: cada mutador compara este par na entrada e na saída.
    fn caret(&self) -> (usize, Option<(usize, usize)>) {
        (self.cursor, self.intervalo())
    }

    fn bits(&self, texto_mudou: bool, caret_antes: (usize, Option<(usize, usize)>)) -> u8 {
        let mut b = 0;
        if texto_mudou {
            b |= MUDOU_TEXTO;
        }
        if self.caret() != caret_antes {
            b |= MUDOU_CARET;
        }
        b
    }

    /// Devolve `MUDOU_CARET` se havia seleção, 0 se não.
    pub fn limpar_selecao(&mut self) -> u8 {
        let antes = self.caret();
        self.ancora = None;
        self.bits(false, antes)
    }

    /// Devolve `MUDOU_CARET` se a seleção ou o cursor mudaram; 0 com tudo já selecionado ou buffer vazio.
    pub fn selecionar_tudo(&mut self) -> u8 {
        let antes = self.caret();
        if self.chars.is_empty() {
            // sem conteúdo não há o que selecionar — deixar âncora ligada aqui
            // criaria uma "seleção" que `intervalo()` nega logo em seguida
            self.ancora = None;
        } else {
            self.ancora = Some(0);
            self.cursor = self.chars.len();
        }
        self.bits(false, antes)
    }

    /// Move o cursor. `estender=true` mantém/abre a âncora (Shift+navegação);
    /// `false` colapsa a seleção. Devolve `MUDOU_CARET` ou 0 (Home já no início).
    pub fn mover(&mut self, m: Movimento, estender: bool) -> u8 {
        let antes = self.caret();
        self.mover_cursor(m, estender);
        self.bits(false, antes)
    }

    fn mover_cursor(&mut self, m: Movimento, estender: bool) {
        // Sem estender, uma seleção ativa COLAPSA na ponta para onde se anda —
        // é o que todo editor faz, e evita o cursor "pular por cima" do bloco.
        if !estender {
            if let Some((i, f)) = self.intervalo() {
                self.ancora = None;
                match m {
                    Movimento::Esquerda => {
                        self.cursor = i;
                        return;
                    }
                    Movimento::Direita => {
                        self.cursor = f;
                        return;
                    }
                    _ => {}
                }
            }
            self.ancora = None;
        } else if self.ancora.is_none() {
            self.ancora = Some(self.cursor);
        }
        self.cursor = self.destino(m);
    }

    // TODO(F1-53): Esquerda/Direita, o fim das varreduras de palavra, `apagar_antes` e `apagar_depois`
    // andam 1 codepoint, então partem "e"+U+0301, a bandeira (par de U+1F1E6..1F1FF) e o emoji com
    // ZWJ. A tecla só traz ASCII (`to_character_key`, cp77-console/src/entrada/mod.rs); texto
    // decomposto entra só pela colagem (`clipboard_get`, NSPasteboard sem normalização, terminal/
    // overlay.rs) e pelo `SetText`. Que chegue NFD por essas vias: não medido. Com essa prova entra
    // `fn fronteira(chars, i)` com contexto (GB9 Extend + ZWJ; GB11 ExtPict Extend* ZWJ × ExtPict;
    // GB12/13 paridade de RI), sem dependência, com os índices ainda em codepoint, e a native
    // `BwmsTextClusterLen` para o medidor medir o cluster inteiro. Um predicado por char não serve:
    // não resolve bandeira nem ZWJ.
    fn destino(&self, m: Movimento) -> usize {
        let n = self.chars.len();
        match m {
            Movimento::Inicio => 0,
            Movimento::Fim => n,
            Movimento::Esquerda => self.cursor.saturating_sub(1),
            Movimento::Direita => (self.cursor + 1).min(n),
            Movimento::PalavraEsquerda => {
                let mut i = self.cursor;
                // pula os separadores colados no cursor, depois a palavra
                while i > 0 && eh_separador(self.chars[i - 1]) {
                    i -= 1;
                }
                while i > 0 && !eh_separador(self.chars[i - 1]) {
                    i -= 1;
                }
                i
            }
            Movimento::PalavraDireita => {
                let mut i = self.cursor;
                while i < n && !eh_separador(self.chars[i]) {
                    i += 1;
                }
                while i < n && eh_separador(self.chars[i]) {
                    i += 1;
                }
                i
            }
        }
    }

    /// Remove a seleção, se houver, e devolve `true` se removeu algo.
    /// Toda escrita passa por aqui — é o que garante que digitar sobre uma
    /// seleção a substitui, sem cada operação repetir a regra.
    fn consumir_selecao(&mut self) -> bool {
        let Some((i, f)) = self.intervalo() else {
            self.ancora = None;
            return false;
        };
        self.chars.drain(i..f);
        self.cursor = i;
        self.ancora = None;
        true
    }

    /// Insere texto na posição do cursor, substituindo a seleção se houver. `MUDOU_TEXTO` só
    /// quando o trecho trocado difere do inserido: colar sobre a seleção o mesmo texto muda só o caret.
    pub fn inserir(&mut self, t: &str) -> u8 {
        let antes = self.caret();
        let novos: Vec<char> = t.chars().collect();
        let texto_mudou = match self.intervalo() {
            Some((i, f)) => self.chars[i..f] != novos[..],
            None => !novos.is_empty(),
        };
        self.consumir_selecao();
        let n = novos.len();
        let at = self.cursor.min(self.chars.len());
        self.chars.splice(at..at, novos);
        self.cursor = at + n;
        self.bits(texto_mudou, antes)
    }

    /// Backspace: apaga a seleção, ou o codepoint ANTES do cursor.
    pub fn apagar_antes(&mut self) -> u8 {
        let antes = self.caret();
        let apagou = if self.consumir_selecao() {
            true
        } else if self.cursor == 0 {
            false
        } else {
            self.cursor -= 1;
            self.chars.remove(self.cursor);
            true
        };
        self.bits(apagou, antes)
    }

    /// Delete: apaga a seleção, ou o codepoint SOB o cursor.
    pub fn apagar_depois(&mut self) -> u8 {
        let antes = self.caret();
        let apagou = if self.consumir_selecao() {
            true
        } else if self.cursor >= self.chars.len() {
            false
        } else {
            self.chars.remove(self.cursor);
            true
        };
        self.bits(apagou, antes)
    }

    /// Recorta: devolve o selecionado e o remove. Vazio se não havia seleção.
    pub fn recortar(&mut self) -> String {
        let s = self.texto_selecionado();
        self.consumir_selecao();
        s
    }

    /// Troca o conteúdo inteiro; cursor vai para o fim e a seleção some.
    pub fn definir_texto(&mut self, t: &str) -> u8 {
        let antes = self.caret();
        let texto_mudou = !self.chars.iter().copied().eq(t.chars());
        if texto_mudou {
            self.chars = t.chars().collect();
        }
        self.cursor = self.chars.len();
        self.ancora = None;
        self.bits(texto_mudou, antes)
    }
}

/// Deslocamento horizontal do conteúdo para manter o caret visível na janela.
///
/// Convenção: o deslocamento é NEGATIVO quando o conteúdo é empurrado para a esquerda
/// (o começo do texto sai pela borda). Devolve o deslocamento novo, já saturado — quem
/// chama não precisa validar nada.
///
/// Escrito como três casos que se excluem, em vez de ajustes encadeados: cabe tudo /
/// caret à esquerda da janela / caret à direita da janela. O caso "cabe tudo" vem
/// primeiro porque é o único que ignora o caret por completo.
pub fn deslocamento_para_caret(
    largura_conteudo: f32,
    largura_janela: f32,
    offset_caret: f32,
    largura_caret: f32,
    deslocamento_atual: f32,
) -> f32 {
    if !(largura_janela > 0.0) || largura_conteudo <= largura_janela {
        return 0.0;
    }
    // O quanto o conteúdo pode ser empurrado antes de sobrar buraco à direita.
    let limite = -(largura_conteudo - largura_janela + largura_caret).max(0.0);
    let esquerda = -deslocamento_atual;
    let direita = esquerda + largura_janela;

    // Cada ramo diz só ONDE quer o conteúdo; o clamp é único, no fim. A 1ª versão
    // clampava ramo a ramo e o ramo "caret à esquerda" ficou sem — com um
    // deslocamento de entrada absurdo ele empurrava além do fim do conteúdo,
    // deixando buraco à direita. Um só ponto de saturação torna isso impossível.
    let desejado = if offset_caret < esquerda {
        -offset_caret
    } else if offset_caret + largura_caret > direita {
        -(offset_caret + largura_caret - largura_janela)
    } else {
        deslocamento_atual
    };
    desejado.max(limite).min(0.0)
}

#[cfg(test)]
mod tests {
    use super::Movimento::*;
    use super::*;

    fn ed(t: &str) -> EditorTexto {
        EditorTexto::de_texto(t)
    }

    #[test]
    fn comprimento_e_em_caractere_nao_em_byte() {
        // "ção" tem 3 caracteres e 5 bytes — a classe de bug que já quebrou este projeto
        let e = ed("ção");
        assert_eq!(e.comprimento(), 3);
        assert_eq!(e.texto().len(), 5, "confirma que os bytes realmente divergem");
        assert_eq!(e.cursor(), 3);
    }

    #[test]
    fn inserir_no_meio_de_multibyte_nao_corrompe() {
        let mut e = ed("ção");
        e.mover(Inicio, false);
        e.mover(Direita, false);
        e.inserir("X");
        assert_eq!(e.texto(), "çXão");
    }

    #[test]
    fn backspace_e_delete() {
        let mut e = ed("abc");
        e.apagar_antes();
        assert_eq!((e.texto().as_str(), e.cursor()), ("ab", 2));
        e.mover(Inicio, false);
        e.apagar_depois();
        assert_eq!((e.texto().as_str(), e.cursor()), ("b", 0));
    }

    #[test]
    fn apagar_nos_limites_e_no_op_nao_panica() {
        let mut e = ed("");
        e.apagar_antes();
        e.apagar_depois();
        assert_eq!(e.texto(), "");
        let mut e = ed("a");
        e.mover(Inicio, false);
        e.apagar_antes(); // já no começo
        assert_eq!(e.texto(), "a");
        e.mover(Fim, false);
        e.apagar_depois(); // já no fim
        assert_eq!(e.texto(), "a");
    }

    #[test]
    fn ancora_igual_ao_cursor_nao_e_selecao() {
        let mut e = ed("abc");
        e.mover(Esquerda, true);
        e.mover(Direita, true); // volta ao ponto de partida
        assert!(!e.tem_selecao(), "âncora coincidente com o cursor é seleção VAZIA");
        assert_eq!(e.texto_selecionado(), "");
    }

    #[test]
    fn selecionar_tudo_em_buffer_vazio_nao_cria_selecao_fantasma() {
        let mut e = ed("");
        e.selecionar_tudo();
        assert!(!e.tem_selecao());
        assert_eq!(e.intervalo(), None);
    }

    #[test]
    fn digitar_sobre_selecao_substitui() {
        let mut e = ed("abcdef");
        e.mover(Inicio, false);
        for _ in 0..3 {
            e.mover(Direita, true);
        }
        assert_eq!(e.texto_selecionado(), "abc");
        e.inserir("Z");
        assert_eq!((e.texto().as_str(), e.cursor()), ("Zdef", 1));
        assert!(!e.tem_selecao());
    }

    #[test]
    fn backspace_sobre_selecao_apaga_o_bloco_nao_um_caractere() {
        let mut e = ed("abcdef");
        e.selecionar_tudo();
        e.apagar_antes();
        assert_eq!(e.texto(), "");
    }

    #[test]
    fn recortar_devolve_e_remove() {
        let mut e = ed("abcdef");
        e.mover(Inicio, false);
        for _ in 0..3 {
            e.mover(Direita, true);
        }
        assert_eq!(e.recortar(), "abc");
        assert_eq!(e.texto(), "def");
        assert_eq!(e.recortar(), "", "sem seleção, recortar é vazio e não mexe no texto");
        assert_eq!(e.texto(), "def");
    }

    #[test]
    fn seta_sem_shift_colapsa_a_selecao_na_ponta_certa() {
        let mut e = ed("abcdef");
        e.mover(Inicio, false);
        for _ in 0..3 {
            e.mover(Direita, true);
        }
        let mut esq = e.clone();
        esq.mover(Esquerda, false);
        assert_eq!(esq.cursor(), 0, "esquerda colapsa no INÍCIO do bloco");
        let mut dir = e.clone();
        dir.mover(Direita, false);
        assert_eq!(dir.cursor(), 3, "direita colapsa no FIM do bloco");
    }

    #[test]
    fn navegacao_por_palavra() {
        let mut e = ed("foo bar  baz");
        e.mover(Inicio, false);
        e.mover(PalavraDireita, false);
        assert_eq!(e.cursor(), 4, "para no começo de 'bar'");
        e.mover(PalavraDireita, false);
        assert_eq!(e.cursor(), 9, "pula os 2 espaços e para em 'baz'");
        e.mover(PalavraDireita, false);
        assert_eq!(e.cursor(), 12, "no fim, satura");
        e.mover(PalavraEsquerda, false);
        assert_eq!(e.cursor(), 9);
        e.mover(PalavraEsquerda, false);
        assert_eq!(e.cursor(), 4);
    }

    #[test]
    fn underscore_nao_e_separador_de_palavra() {
        let mut e = ed("foo_bar baz");
        e.mover(Inicio, false);
        e.mover(PalavraDireita, false);
        assert_eq!(e.cursor(), 8, "'foo_bar' é UMA palavra");
    }

    #[test]
    fn shift_seta_estende_e_seta_normal_limpa() {
        let mut e = ed("abc");
        e.mover(Inicio, false);
        e.mover(Direita, true);
        assert_eq!(e.texto_selecionado(), "a");
        e.mover(Direita, true);
        assert_eq!(e.texto_selecionado(), "ab");
        e.mover(Esquerda, true);
        assert_eq!(e.texto_selecionado(), "a", "encolhe de volta");
        e.mover(Fim, false);
        assert!(!e.tem_selecao());
    }

    #[test]
    fn selecao_para_tras_tem_intervalo_ordenado() {
        let mut e = ed("abcdef");
        e.mover(Fim, false);
        for _ in 0..2 {
            e.mover(Esquerda, true);
        }
        assert_eq!(e.intervalo(), Some((4, 6)), "âncora > cursor, intervalo sai ordenado");
        assert_eq!(e.texto_selecionado(), "ef");
    }

    #[test]
    fn definir_texto_reseta_cursor_e_selecao() {
        let mut e = ed("abcdef");
        e.selecionar_tudo();
        e.definir_texto("oi");
        assert_eq!((e.texto().as_str(), e.cursor()), ("oi", 2));
        assert!(!e.tem_selecao());
    }

    // ── bits do que mudou ────────────────────────────────────────────────────

    const TUDO: u8 = MUDOU_TEXTO | MUDOU_CARET;

    #[test]
    fn operacao_sem_efeito_devolve_zero() {
        let mut e = ed("abc");
        e.mover(Inicio, false);
        assert_eq!(e.apagar_antes(), 0, "backspace no cursor 0");
        assert_eq!(e.mover(Inicio, false), 0, "Home já no início");
        assert_eq!(e.inserir(""), 0);
        assert_eq!(e.limpar_selecao(), 0);
        e.mover(Fim, false);
        assert_eq!(e.apagar_depois(), 0, "delete no fim");
        assert_eq!(e.definir_texto("abc"), 0, "mesmo texto, cursor já no fim");
        let mut vazio = ed("");
        assert_eq!(vazio.selecionar_tudo(), 0);
        let mut tudo = ed("ab");
        tudo.selecionar_tudo();
        assert_eq!(tudo.selecionar_tudo(), 0, "já estava tudo selecionado");
    }

    #[test]
    fn bits_por_operacao() {
        let mut e = EditorTexto::novo();
        assert_eq!(e.inserir("a"), TUDO);
        assert_eq!(e.mover(Esquerda, false), MUDOU_CARET);
        assert_eq!(e.apagar_depois(), MUDOU_TEXTO, "delete sob o cursor não move o cursor");
        assert_eq!(e.inserir("xy"), TUDO);
        assert_eq!(e.apagar_antes(), TUDO);
        assert_eq!(e.selecionar_tudo(), MUDOU_CARET);
        assert_eq!(e.limpar_selecao(), MUDOU_CARET);
        assert_eq!(e.definir_texto("novo"), TUDO);
        e.mover(Inicio, false);
        assert_eq!(e.definir_texto("novo"), MUDOU_CARET, "mesmo texto, cursor volta ao fim");
        e.selecionar_tudo();
        assert_eq!(e.apagar_antes(), TUDO, "backspace sobre a seleção");
    }

    #[test]
    fn ancora_vazia_nao_conta_como_mudanca() {
        // Shift+Direita no fim liga a âncora no próprio cursor: seleção vazia, nada visível muda.
        let mut e = ed("abc");
        assert_eq!(e.mover(Direita, true), 0);
        // o Delete seguinte zera essa âncora e não apaga nada
        assert_eq!(e.apagar_depois(), 0);
        assert_eq!(e.texto(), "abc");
    }

    #[test]
    fn colar_o_mesmo_texto_sobre_a_selecao_muda_so_o_caret() {
        let mut e = ed("abc");
        e.mover(Inicio, false);
        e.mover(Direita, true);
        assert_eq!(e.inserir("a"), MUDOU_CARET);
        assert_eq!((e.texto().as_str(), e.cursor()), ("abc", 1));
        assert!(!e.tem_selecao());
        e.mover(Direita, true);
        assert_eq!(e.inserir("Z"), TUDO, "mesmo tamanho, texto diferente");
        assert_eq!(e.texto(), "aZc");
    }

    // ── deslocamento_para_caret ──────────────────────────────────────────────

    #[test]
    fn conteudo_que_cabe_nunca_desloca() {
        assert_eq!(deslocamento_para_caret(50.0, 100.0, 40.0, 2.0, -30.0), 0.0);
        assert_eq!(deslocamento_para_caret(100.0, 100.0, 99.0, 2.0, -30.0), 0.0);
    }

    #[test]
    fn janela_degenerada_nao_desloca_nem_divide_por_zero() {
        assert_eq!(deslocamento_para_caret(200.0, 0.0, 50.0, 2.0, -10.0), 0.0);
        assert_eq!(deslocamento_para_caret(200.0, f32::NAN, 50.0, 2.0, -10.0), 0.0);
    }

    #[test]
    fn caret_a_esquerda_da_janela_traz_o_conteudo_de_volta() {
        // janela mostra [50,150]; caret em 20 está fora, à esquerda
        let d = deslocamento_para_caret(400.0, 100.0, 20.0, 2.0, -50.0);
        assert_eq!(d, -20.0, "alinha o caret na borda esquerda");
    }

    #[test]
    fn caret_a_direita_da_janela_empurra_o_conteudo() {
        // janela mostra [0,100]; caret em 180 está fora, à direita
        let d = deslocamento_para_caret(400.0, 100.0, 180.0, 2.0, 0.0);
        assert_eq!(d, -82.0, "caret+largura encostam na borda direita");
    }

    #[test]
    fn caret_dentro_da_janela_nao_mexe() {
        let d = deslocamento_para_caret(400.0, 100.0, 60.0, 2.0, -30.0);
        assert_eq!(d, -30.0);
    }

    #[test]
    fn nunca_empurra_alem_do_fim_do_conteudo() {
        // deslocamento atual absurdo: satura no limite, sem deixar buraco à direita
        let d = deslocamento_para_caret(400.0, 100.0, 350.0, 2.0, -9999.0);
        assert_eq!(d, -302.0, "-(400-100+2)");
        assert!(d >= -(400.0 - 100.0 + 2.0));
    }

    #[test]
    fn deslocamento_nunca_e_positivo() {
        for conteudo in [0.0f32, 50.0, 400.0] {
            for caret in [0.0f32, 10.0, 399.0] {
                for atual in [-500.0f32, -10.0, 0.0, 500.0] {
                    let d = deslocamento_para_caret(conteudo, 100.0, caret, 2.0, atual);
                    assert!(d <= 0.0, "conteudo={conteudo} caret={caret} atual={atual} -> {d}");
                }
            }
        }
    }

    #[test]
    fn sequencia_de_edicao_completa() {
        // cenário ponta-a-ponta: digitar, selecionar palavra, colar por cima, recortar
        let mut e = EditorTexto::novo();
        e.inserir("olá mundo");
        assert_eq!(e.texto(), "olá mundo");
        e.mover(Inicio, false);
        e.mover(PalavraDireita, true);
        assert_eq!(e.texto_selecionado(), "olá ");
        e.inserir("tchau ");
        assert_eq!(e.texto(), "tchau mundo");
        e.mover(Fim, false);
        e.mover(PalavraEsquerda, true);
        assert_eq!(e.recortar(), "mundo");
        assert_eq!(e.texto(), "tchau ");
    }
}
