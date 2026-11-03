//! Edição de texto de uma linha (comando, busca, número): cursor, seleção, palavras, área de transferência. Comportamento de Mac do
//! campo que o terminal de hoje usa: Option anda por palavra, Cmd vai ao começo/fim, Shift seleciona, Esc devolve o texto de antes da
//! edição. Puro: quem chama entrega os eventos do quadro em ordem e a área de transferência.

/// Códigos de tecla do macOS que a edição trata.
pub(crate) mod tecla {
    pub(crate) const ENTER: u16 = 36;
    pub(crate) const ENTER_NUMERICO: u16 = 76;
    pub(crate) const TAB: u16 = 48;
    pub(crate) const BACKSPACE: u16 = 51;
    pub(crate) const ESC: u16 = 53;
    pub(crate) const DELETE: u16 = 117;
    pub(crate) const HOME: u16 = 115;
    pub(crate) const END: u16 = 119;
    pub(crate) const ESQUERDA: u16 = 123;
    pub(crate) const DIREITA: u16 = 124;
    pub(crate) const A: u16 = 0;
    pub(crate) const C: u16 = 8;
    pub(crate) const V: u16 = 9;
    pub(crate) const X: u16 = 7;
}

/// Modificadores da tecla.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Mods {
    pub(crate) cmd: bool,
    pub(crate) shift: bool,
    pub(crate) option: bool,
}

/// Um evento do quadro, na ordem em que chegou.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Evento {
    /// Caractere já resolvido pelo sistema (acento morto incluído).
    Caractere(char),
    Tecla(u16, Mods),
}

/// De onde vem e para onde vai o texto de Cmd+C/X/V: no jogo, a do sistema; nos testes, uma falsa.
pub(crate) trait AreaDeTransferencia {
    fn ler(&mut self) -> Option<String>;
    fn gravar(&mut self, texto: &str);
}

/// O que a edição pede a quem a chamou.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Pedido {
    Nada,
    /// Enter: o texto foi confirmado e a edição acaba.
    Confirmou,
    /// Esc: o texto voltou ao de antes da edição e a edição acaba.
    Cancelou,
    /// Tab: a edição acaba e o foco vai ao próximo campo.
    Proximo,
}

/// Estado de uma edição em curso. `cursor` e `ancora` contam caracteres; seleção = entre os dois.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Edicao {
    pub(crate) cursor: usize,
    pub(crate) ancora: Option<usize>,
    /// O texto quando a edição começou, para o Esc devolver.
    pub(crate) original: String,
}

/// Caractere que entra no texto: nem controle nem as teclas de função que o AppKit manda na área privada (0xF700–0xF8FF).
pub(crate) fn aceita(c: char) -> bool {
    !c.is_control() && !('\u{E000}'..='\u{F8FF}').contains(&c)
}

fn indice(texto: &str, caractere: usize) -> usize {
    texto.char_indices().nth(caractere).map_or(texto.len(), |(i, _)| i)
}

/// Caractere de palavra = o que o campo da referência não trata como separador: só brancos e `, ; ( ) { } [ ] |` cortam
/// palavra, então `Items.Preset_Katana_Saburo` é uma palavra só no clique duplo e no Option+←/→/Backspace.
fn e_de_palavra(c: char) -> bool {
    !matches!(c, ' ' | '\t' | '\u{3000}' | ',' | ';' | '(' | ')' | '{' | '}' | '[' | ']' | '|' | '\n' | '\r')
}

/// Início da palavra à esquerda de `de` (pula separadores e depois a palavra).
fn palavra_a_esquerda(cs: &[char], de: usize) -> usize {
    let mut i = de;
    while i > 0 && !e_de_palavra(cs[i - 1]) {
        i -= 1;
    }
    while i > 0 && e_de_palavra(cs[i - 1]) {
        i -= 1;
    }
    i
}

/// Fim da palavra à direita de `de`.
fn palavra_a_direita(cs: &[char], de: usize) -> usize {
    let mut i = de;
    while i < cs.len() && !e_de_palavra(cs[i]) {
        i += 1;
    }
    while i < cs.len() && e_de_palavra(cs[i]) {
        i += 1;
    }
    i
}

impl Edicao {
    /// Começa a editar `texto` com o cursor no fim, sem seleção.
    pub(crate) fn comecar(texto: &str) -> Self {
        Edicao { cursor: texto.chars().count(), ancora: None, original: texto.to_string() }
    }

    /// Faixa selecionada `(início, fim)`, se houver.
    pub(crate) fn selecao(&self) -> Option<(usize, usize)> {
        self.ancora.filter(|a| *a != self.cursor).map(|a| (a.min(self.cursor), a.max(self.cursor)))
    }

    fn apagar_selecao(&mut self, texto: &mut String) -> bool {
        let Some((a, b)) = self.selecao() else { return false };
        texto.replace_range(indice(texto, a)..indice(texto, b), "");
        self.cursor = a;
        self.ancora = None;
        true
    }

    fn inserir(&mut self, texto: &mut String, s: &str) {
        self.apagar_selecao(texto);
        let limpo: String = s.chars().filter(|c| aceita(*c)).collect();
        texto.insert_str(indice(texto, self.cursor), &limpo);
        self.cursor += limpo.chars().count();
        self.ancora = None;
    }

    /// Move o cursor para `para`; com Shift estende a seleção, sem ele a desfaz.
    fn mover(&mut self, para: usize, shift: bool) {
        if shift {
            self.ancora.get_or_insert(self.cursor);
        } else {
            self.ancora = None;
        }
        self.cursor = para;
    }

    /// Aplica um evento ao `texto`.
    pub(crate) fn aplicar(&mut self, texto: &mut String, ev: &Evento, area: &mut dyn AreaDeTransferencia) -> Pedido {
        let total = texto.chars().count();
        self.cursor = self.cursor.min(total);
        // Âncora no próprio cursor (o aperto do mouse a deixa pronta para o arraste) não é seleção.
        if self.ancora == Some(self.cursor) {
            self.ancora = None;
        }
        match ev {
            Evento::Caractere(c) => {
                if aceita(*c) {
                    self.inserir(texto, &c.to_string());
                }
                Pedido::Nada
            }
            Evento::Tecla(k, m) => {
                let cs: Vec<char> = texto.chars().collect();
                match *k {
                    tecla::ENTER | tecla::ENTER_NUMERICO => return Pedido::Confirmou,
                    tecla::ESC => {
                        *texto = self.original.clone();
                        self.cursor = texto.chars().count();
                        self.ancora = None;
                        return Pedido::Cancelou;
                    }
                    tecla::TAB => return Pedido::Proximo,
                    tecla::ESQUERDA => {
                        let para = if m.cmd {
                            0
                        } else if m.option {
                            palavra_a_esquerda(&cs, self.cursor)
                        } else if let (Some((a, _)), false) = (self.selecao(), m.shift) {
                            a
                        } else {
                            self.cursor.saturating_sub(1)
                        };
                        self.mover(para, m.shift);
                    }
                    tecla::DIREITA => {
                        let para = if m.cmd {
                            total
                        } else if m.option {
                            palavra_a_direita(&cs, self.cursor)
                        } else if let (Some((_, b)), false) = (self.selecao(), m.shift) {
                            b
                        } else {
                            (self.cursor + 1).min(total)
                        };
                        self.mover(para, m.shift);
                    }
                    tecla::HOME => self.mover(0, m.shift),
                    tecla::END => self.mover(total, m.shift),
                    tecla::BACKSPACE => {
                        if !self.apagar_selecao(texto) && self.cursor > 0 {
                            let de = if m.cmd {
                                0
                            } else if m.option {
                                palavra_a_esquerda(&cs, self.cursor)
                            } else {
                                self.cursor - 1
                            };
                            texto.replace_range(indice(texto, de)..indice(texto, self.cursor), "");
                            self.cursor = de;
                        }
                    }
                    tecla::DELETE => {
                        if !self.apagar_selecao(texto) && self.cursor < total {
                            let ate = if m.option { palavra_a_direita(&cs, self.cursor) } else { self.cursor + 1 };
                            texto.replace_range(indice(texto, self.cursor)..indice(texto, ate), "");
                        }
                    }
                    tecla::A if m.cmd => {
                        self.ancora = Some(0);
                        self.cursor = total;
                    }
                    // Sem seleção, Cmd+C e Cmd+X valem para o texto inteiro: o comando todo vai para a área de transferência num
                    // atalho só. Campo vazio não mexe nela.
                    tecla::C if m.cmd => {
                        let (a, b) = self.selecao().unwrap_or((0, total));
                        if a < b {
                            area.gravar(&cs[a..b].iter().collect::<String>());
                        }
                    }
                    tecla::X if m.cmd => {
                        let (a, b) = self.selecao().unwrap_or((0, total));
                        if a < b {
                            area.gravar(&cs[a..b].iter().collect::<String>());
                            self.ancora = Some(a);
                            self.cursor = b;
                            self.apagar_selecao(texto);
                        }
                    }
                    tecla::V if m.cmd => {
                        if let Some(colado) = area.ler() {
                            self.inserir(texto, &colado);
                        }
                    }
                    _ => {}
                }
                Pedido::Nada
            }
        }
    }

    /// Seleciona a palavra sob o caractere `em` (clique duplo).
    pub(crate) fn selecionar_palavra(&mut self, texto: &str, em: usize) {
        let cs: Vec<char> = texto.chars().collect();
        let em = em.min(cs.len());
        let (mut a, mut b) = (em, em);
        while a > 0 && e_de_palavra(cs[a - 1]) {
            a -= 1;
        }
        while b < cs.len() && e_de_palavra(cs[b]) {
            b += 1;
        }
        self.ancora = Some(a);
        self.cursor = b;
    }
}

#[cfg(test)]
mod teste {
    use super::*;

    #[derive(Default)]
    struct Falsa(Option<String>);
    impl AreaDeTransferencia for Falsa {
        fn ler(&mut self) -> Option<String> {
            self.0.clone()
        }
        fn gravar(&mut self, texto: &str) {
            self.0 = Some(texto.to_string());
        }
    }

    const NADA: Mods = Mods { cmd: false, shift: false, option: false };
    const SHIFT: Mods = Mods { cmd: false, shift: true, option: false };
    const CMD: Mods = Mods { cmd: true, shift: false, option: false };
    const OPT: Mods = Mods { cmd: false, shift: false, option: true };

    fn rodar(texto: &str, evs: &[Evento]) -> (String, Edicao, Vec<Pedido>, Falsa) {
        let mut t = texto.to_string();
        let mut e = Edicao::comecar(texto);
        let mut area = Falsa::default();
        let pedidos = evs.iter().map(|ev| e.aplicar(&mut t, ev, &mut area)).collect();
        (t, e, pedidos, area)
    }

    #[test]
    fn digita_no_cursor_e_filtra_controle_e_teclas_de_funcao() {
        let (t, e, _, _) = rodar(
            "give",
            &[Evento::Caractere(' '), Evento::Caractere('x'), Evento::Caractere('\n'), Evento::Caractere('\u{F704}')],
        );
        assert_eq!(t, "give x");
        assert_eq!(e.cursor, 6);
    }

    #[test]
    fn option_anda_por_palavra_e_cmd_vai_as_pontas() {
        let (_, e, _, _) = rodar("give Items.X 2", &[Evento::Tecla(tecla::ESQUERDA, OPT)]);
        assert_eq!(e.cursor, 13);
        let (_, e, _, _) = rodar("give Items.X 2", &[Evento::Tecla(tecla::ESQUERDA, OPT), Evento::Tecla(tecla::ESQUERDA, OPT)]);
        assert_eq!(e.cursor, 5);
        let (_, e, _, _) = rodar("give Items.X 2", &[Evento::Tecla(tecla::ESQUERDA, CMD)]);
        assert_eq!(e.cursor, 0);
        let (_, e, _, _) = rodar("give", &[Evento::Tecla(tecla::HOME, NADA), Evento::Tecla(tecla::END, NADA)]);
        assert_eq!(e.cursor, 4);
    }

    #[test]
    fn shift_seleciona_e_digitar_troca_a_selecao() {
        let (t, e, _, _) = rodar(
            "money 5000",
            &[Evento::Tecla(tecla::ESQUERDA, Mods { option: true, shift: true, cmd: false }), Evento::Caractere('9')],
        );
        assert_eq!(t, "money 9");
        assert_eq!(e.selecao(), None);
    }

    #[test]
    fn seta_sem_shift_recolhe_a_selecao_na_ponta() {
        let (_, e, _, _) = rodar("abcdef", &[Evento::Tecla(tecla::ESQUERDA, SHIFT), Evento::Tecla(tecla::ESQUERDA, SHIFT), Evento::Tecla(tecla::ESQUERDA, NADA)]);
        assert_eq!(e.cursor, 4);
        assert_eq!(e.selecao(), None);
    }

    #[test]
    fn apagar_por_palavra_e_ate_o_comeco() {
        let (t, _, _, _) = rodar("give Items.Katana", &[Evento::Tecla(tecla::BACKSPACE, OPT)]);
        assert_eq!(t, "give ");
        let (t, _, _, _) = rodar("give Items.Katana", &[Evento::Tecla(tecla::BACKSPACE, CMD)]);
        assert_eq!(t, "");
        let (t, _, _, _) = rodar("abc", &[Evento::Tecla(tecla::HOME, NADA), Evento::Tecla(tecla::DELETE, NADA)]);
        assert_eq!(t, "bc");
    }

    #[test]
    fn copiar_recortar_colar_e_selecionar_tudo() {
        let (t, _, _, area) = rodar("money 5000", &[Evento::Tecla(tecla::A, CMD), Evento::Tecla(tecla::X, CMD)]);
        assert_eq!(t, "");
        assert_eq!(area.0.as_deref(), Some("money 5000"));
        let mut texto = "give ".to_string();
        let mut e = Edicao::comecar(&texto);
        let mut area = Falsa(Some("Items.X\n2".to_string()));
        e.aplicar(&mut texto, &Evento::Tecla(tecla::V, CMD), &mut area);
        assert_eq!(texto, "give Items.X2", "o colado perde a quebra de linha");
    }

    #[test]
    fn sem_selecao_copiar_e_recortar_pegam_o_texto_inteiro() {
        let (t, _, _, area) = rodar("money 5000", &[Evento::Tecla(tecla::C, CMD)]);
        assert_eq!((t.as_str(), area.0.as_deref()), ("money 5000", Some("money 5000")));
        let (t, e, _, area) = rodar("money 5000", &[Evento::Tecla(tecla::X, CMD)]);
        assert_eq!((t.as_str(), area.0.as_deref(), e.cursor), ("", Some("money 5000"), 0));
        let (_, _, _, area) = rodar("", &[Evento::Tecla(tecla::C, CMD), Evento::Tecla(tecla::X, CMD)]);
        assert_eq!(area.0, None, "campo vazio não mexe na área de transferência");
    }

    #[test]
    fn enter_confirma_esc_devolve_o_texto_de_antes_e_tab_passa() {
        let (_, _, p, _) = rodar("heal", &[Evento::Tecla(tecla::ENTER_NUMERICO, NADA)]);
        assert_eq!(p, vec![Pedido::Confirmou]);
        let (t, _, p, _) = rodar("heal", &[Evento::Caractere('x'), Evento::Tecla(tecla::ESC, NADA)]);
        assert_eq!((t.as_str(), p.last().copied()), ("heal", Some(Pedido::Cancelou)));
        let (_, _, p, _) = rodar("heal", &[Evento::Tecla(tecla::TAB, NADA)]);
        assert_eq!(p, vec![Pedido::Proximo]);
    }

    #[test]
    fn clique_duplo_pega_a_palavra_e_acento_conta_como_um() {
        let mut e = Edicao::comecar("give Itens.Ação 2");
        e.selecionar_palavra("give Itens.Ação 2", 12);
        assert_eq!(e.selecao(), Some((5, 15)));
    }
}
