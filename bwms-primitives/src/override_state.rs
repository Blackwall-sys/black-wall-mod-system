//! `ComponentState` — overrides de máscara de chunks e de aparência de um componente, por fonte (mod).

/// Estado de override de UM componente, acumulando fontes. Cada tabela é SoA ordenada pela fonte: a igualdade não depende da ordem de
/// chegada (a guarda de teste compara estados) e a aparência de menor fonte é a 1ª da tabela.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComponentState {
    fontes_esconder: Vec<u64>,
    mascaras_esconder: Vec<u64>,
    fontes_mostrar: Vec<u64>,
    mascaras_mostrar: Vec<u64>,
    fontes_aparencia: Vec<u64>,
    aparencias: Vec<u64>,
    /// Fica ligado depois da 1ª máscara: sem fonte, a aplicação devolve a original ao componente.
    mudou_mascara: bool,
    /// Fica ligado depois da 1ª aparência: sem fonte, a aplicação restaura a original.
    mudou_aparencia: bool,
}

/// Grava `valor` da `fonte`; a fonte já presente combina o valor guardado com o novo por `juntar`.
fn gravar_por_fonte(fontes: &mut Vec<u64>, valores: &mut Vec<u64>, fonte: u64, valor: u64, juntar: fn(u64, u64) -> u64) {
    match fontes.binary_search(&fonte) {
        Ok(i) => valores[i] = juntar(valores[i], valor),
        Err(i) => {
            fontes.insert(i, fonte);
            valores.insert(i, valor);
        }
    }
}

fn tirar_fonte(fontes: &mut Vec<u64>, valores: &mut Vec<u64>, fonte: u64) -> bool {
    match fontes.binary_search(&fonte) {
        Ok(i) => {
            fontes.remove(i);
            valores.remove(i);
            true
        }
        Err(_) => false,
    }
}

impl ComponentState {
    pub fn novo() -> Self {
        Self::default()
    }

    /// A mesma fonte acumula por interseção: duas chamadas escondem os chunks das duas máscaras.
    pub fn esconder(&mut self, fonte: u64, mascara: u64) {
        gravar_por_fonte(&mut self.fontes_esconder, &mut self.mascaras_esconder, fonte, mascara, |a, b| a & b);
        self.mudou_mascara = true;
    }

    pub fn mostrar(&mut self, fonte: u64, mascara: u64) {
        gravar_por_fonte(&mut self.fontes_mostrar, &mut self.mascaras_mostrar, fonte, mascara, |a, b| a | b);
        self.mudou_mascara = true;
    }

    /// Tira o esconder e o mostrar da fonte; a aparência dela fica.
    pub fn remover_fonte(&mut self, fonte: u64) -> bool {
        let escondia = tirar_fonte(&mut self.fontes_esconder, &mut self.mascaras_esconder, fonte);
        let mostrava = tirar_fonte(&mut self.fontes_mostrar, &mut self.mascaras_mostrar, fonte);
        escondia || mostrava
    }

    /// Fontes distintas com máscara (esconder ou mostrar).
    pub fn fontes(&self) -> usize {
        let so_mostram = self.fontes_mostrar.iter().filter(|f| self.fontes_esconder.binary_search(f).is_err()).count();
        self.fontes_esconder.len() + so_mostram
    }

    pub fn mascara_final(&self, original: u64) -> u64 {
        combinar_mascara(original, self.mascara_de_mostrar(), self.mascara_de_esconder())
    }

    pub fn mascara_de_mostrar(&self) -> u64 {
        self.mascaras_mostrar.iter().fold(0, |a, m| a | m)
    }

    pub fn muda_mascara(&self) -> bool {
        self.mudou_mascara
    }

    /// `!0` sem fonte: nada escondido.
    pub fn mascara_de_esconder(&self) -> u64 {
        self.mascaras_esconder.iter().fold(u64::MAX, |a, m| a & m)
    }

    pub fn sobrescrever_aparencia(&mut self, fonte: u64, aparencia: u64) {
        gravar_por_fonte(&mut self.fontes_aparencia, &mut self.aparencias, fonte, aparencia, |_, nova| nova);
        self.mudou_aparencia = true;
    }

    pub fn remover_aparencia(&mut self, fonte: u64) -> bool {
        tirar_fonte(&mut self.fontes_aparencia, &mut self.aparencias, fonte)
    }

    /// A da fonte de menor hash: a escolha não depende da ordem em que os mods registram.
    pub fn aparencia_sobrescrita(&self) -> Option<u64> {
        self.aparencias.first().copied()
    }

    pub fn muda_aparencia(&self) -> bool {
        self.mudou_aparencia
    }
}

/// Esconder vence mostrar no mesmo chunk.
pub fn combinar_mascara(original: u64, mostrar: u64, esconder: u64) -> u64 {
    (original | mostrar) & esconder
}

/// `None` se algum chunk passa de 63: tirar só o inválido mudaria o sentido da lista (esconder sem chunk = esconder tudo).
pub const fn mascara_de_chunks(mostrar: bool, chunks: &[u8]) -> Option<u64> {
    let mut bits = 0u64;
    let mut i = 0;
    while i < chunks.len() {
        if chunks[i] > 63 {
            return None;
        }
        bits |= 1u64 << chunks[i];
        i += 1;
    }
    Some(mascara_de_bits(mostrar, bits))
}

/// Esconder com `bits == 0` dá 0: a lista vazia de esconder é "esconder tudo" na interseção.
pub const fn mascara_de_bits(mostrar: bool, bits: u64) -> u64 {
    if mostrar || bits == 0 { bits } else { !bits }
}

/// Aparência a gravar: override do componente, senão do prefixo, senão a original se algum dos dois já mudou. `original` só é lida na
/// restauração: a 1ª leitura fica no cache de originais. Aparência 0 não é gravada.
pub fn aparencia_aplicada(original: impl FnOnce() -> u64, componente: Option<&ComponentState>, prefixo: Option<&ComponentState>) -> Option<u64> {
    let sobrescrita = componente.and_then(ComponentState::aparencia_sobrescrita).or_else(|| prefixo.and_then(ComponentState::aparencia_sobrescrita));
    let valor = match sobrescrita {
        Some(a) => a,
        None if componente.is_some_and(ComponentState::muda_aparencia) || prefixo.is_some_and(ComponentState::muda_aparencia) => original(),
        None => return None,
    };
    (valor != 0).then_some(valor)
}

/// Máscara a gravar, com o componente e o prefixo como uma fonte só; `None` (sem ler `original`) se nenhum dos dois mudou máscara.
pub fn mascara_aplicada(original: impl FnOnce() -> u64, componente: Option<&ComponentState>, prefixo: Option<&ComponentState>) -> Option<u64> {
    if !componente.is_some_and(ComponentState::muda_mascara) && !prefixo.is_some_and(ComponentState::muda_mascara) {
        return None;
    }
    let mostrar = componente.map_or(0, ComponentState::mascara_de_mostrar) | prefixo.map_or(0, ComponentState::mascara_de_mostrar);
    let esconder = componente.map_or(u64::MAX, ComponentState::mascara_de_esconder) & prefixo.map_or(u64::MAX, ComponentState::mascara_de_esconder);
    Some(combinar_mascara(original(), mostrar, esconder))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u64 = 0xA;
    const B: u64 = 0xB;
    const C: u64 = 0xC;

    #[test]
    fn sem_fonte_a_mascara_original_passa_intacta() {
        let s = ComponentState::novo();
        assert_eq!(s.mascara_final(0b1010_1010), 0b1010_1010);
        assert_eq!(s.mascara_final(0), 0);
        assert_eq!(s.mascara_final(u64::MAX), u64::MAX);
    }

    #[test]
    fn uma_fonte_escondendo_apaga_o_bit() {
        let mut s = ComponentState::novo();
        // máscara com bit 1 zerado = "esconde o chunk 1"
        s.esconder(A, !0b10);
        assert_eq!(s.mascara_final(0b1111), 0b1101);
    }

    #[test]
    fn duas_fontes_escondendo_chunks_DIFERENTES_acumulam() {
        // O ponto do item: dois mods escondendo partes diferentes, sem um apagar o outro.
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.esconder(B, !0b0100);
        assert_eq!(s.mascara_final(0b1111), 0b1001, "os dois chunks somem");
    }

    #[test]
    fn hiding_vence_showing() {
        // Todos os OR antes de qualquer AND: aplicar fonte a fonte na ordem de chegada deixaria o
        // mostrar de B religar o chunk 1 (0b1111).
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.mostrar(B, 0b0010);
        assert_eq!(s.mascara_final(0b1111), 0b1101, "quem esconde sobrepõe quem mostra");
    }

    #[test]
    fn combinar_mascara_esconder_vence_e_sem_fonte_e_identidade() {
        // O contraexemplo do show-wins: esconder o chunk 0 por uma fonte e mostrá-lo por outra deixa o chunk 0 escondido.
        assert_eq!(combinar_mascara(0b1111, 0b1, !0b1), 0b1110);
        assert_eq!(combinar_mascara(0b1010, 0, u64::MAX), 0b1010, "sem fonte a original passa intacta");
        assert_eq!(combinar_mascara(0, 0b100, u64::MAX), 0b100);
        assert_eq!(combinar_mascara(u64::MAX, 0b111, 0), 0, "esconder tudo vence qualquer mostrar");
    }

    #[test]
    fn mascara_de_chunks_esconde_os_listados_e_a_lista_vazia_esconde_tudo() {
        // Esconder os chunks 5, 6 e 7 do torso (panturrilhas, tornozelos, pés). Em `const`: a primitiva serve tabela `static`.
        const PERNAS_DO_TORSO: Option<u64> = mascara_de_chunks(false, &[5, 6, 7]);
        assert_eq!(PERNAS_DO_TORSO, Some(0xFFFF_FFFF_FFFF_FF1F));
        assert_eq!(mascara_de_chunks(true, &[0, 1, 2]), Some(0b111), "mostrar não inverte");
        assert_eq!(mascara_de_chunks(false, &[]), Some(0), "esconder sem lista = esconde tudo no AND");
        assert_eq!(mascara_de_chunks(true, &[]), Some(0), "mostrar sem lista não mostra nada");
        assert_eq!(combinar_mascara(u64::MAX, 0, mascara_de_chunks(false, &[]).unwrap()), 0);
    }

    #[test]
    fn mascara_de_chunks_vai_ate_o_63_e_recusa_a_lista_com_64() {
        assert_eq!(mascara_de_chunks(true, &[63]), Some(1u64 << 63));
        // Com `1` do tipo `int` o chunk 31 sairia `0xFFFFFFFF80000000`, e o 32 é UB.
        assert_eq!(mascara_de_chunks(true, &[31]), Some(0x8000_0000));
        assert_eq!(mascara_de_chunks(true, &[32]), Some(1u64 << 32));
        assert_eq!(mascara_de_chunks(false, &[63]), Some(!(1u64 << 63)));
        assert_eq!(mascara_de_chunks(false, &[64]), None, "tirar só o 64 daria o esconder-tudo");
        assert_eq!(mascara_de_chunks(true, &[64]), None);
        assert_eq!(mascara_de_chunks(false, &[5, 64]), None, "o válido não salva a lista");
        assert_eq!(mascara_de_chunks(false, &[255]), None);
    }

    #[test]
    fn showing_liga_bit_que_o_original_nao_tinha() {
        let mut s = ComponentState::novo();
        s.mostrar(A, 0b1_0000);
        assert_eq!(s.mascara_final(0b0001), 0b1_0001);
    }

    #[test]
    fn ordem_dentro_do_grupo_nao_importa() {
        // A ordem de chegada das fontes não entra no resultado: a tabela por fonte pode ser qualquer container.
        let mut x = ComponentState::novo();
        x.esconder(A, !0b0010);
        x.esconder(B, !0b0100);
        x.mostrar(C, 0b1000);
        let mut y = ComponentState::novo();
        y.mostrar(C, 0b1000);
        y.esconder(B, !0b0100);
        y.esconder(A, !0b0010);
        assert_eq!(x.mascara_final(0b1111), y.mascara_final(0b1111));
    }

    #[test]
    fn mesma_fonte_registrando_de_novo_ACUMULA() {
        // Esconder da mesma fonte faz AND no valor guardado (partindo de `!0`): a 2ª máscara da mesma fonte soma à 1ª.
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.esconder(A, !0b0100);
        assert_eq!(s.mascara_final(0b1111), 0b1001, "as duas valem");
        assert_eq!(s.fontes(), 1);
    }

    #[test]
    fn mesma_fonte_mostrando_de_novo_acumula_por_or() {
        // Mostrar da mesma fonte faz OR no valor guardado (partindo de 0): a 2ª máscara da mesma fonte soma à 1ª.
        let mut s = ComponentState::novo();
        s.mostrar(A, 0b0001);
        s.mostrar(A, 0b0100);
        assert_eq!(s.mascara_de_mostrar(), 0b0101);
        assert_eq!(s.fontes(), 1);
    }

    #[test]
    fn aparencia_vence_o_menor_hash_em_qualquer_ordem_e_a_mesma_fonte_substitui() {
        // A antes de B e B antes de A dão o mesmo vencedor: nem a 1ª nem a última inserção decide.
        let mut x = ComponentState::novo();
        x.sobrescrever_aparencia(A, 0x111);
        x.sobrescrever_aparencia(B, 0x222);
        let mut y = ComponentState::novo();
        y.sobrescrever_aparencia(B, 0x222);
        y.sobrescrever_aparencia(A, 0x111);
        assert_eq!(x.aparencia_sobrescrita(), Some(0x111));
        assert_eq!(y.aparencia_sobrescrita(), Some(0x111));
        x.sobrescrever_aparencia(A, 0x333);
        assert_eq!(x.aparencia_sobrescrita(), Some(0x333), "a mesma fonte substitui a própria aparência");
        assert!(x.remover_aparencia(A));
        assert_eq!(x.aparencia_sobrescrita(), Some(0x222), "sem A vence a próxima fonte");
    }

    #[test]
    fn aparencia_do_componente_vence_a_do_prefixo_e_a_original_volta() {
        let mut c = ComponentState::novo();
        let mut p = ComponentState::novo();
        c.sobrescrever_aparencia(A, 0x111);
        p.sobrescrever_aparencia(C, 0x222);
        assert_eq!(aparencia_aplicada(|| 0x999, Some(&c), Some(&p)), Some(0x111));
        assert!(c.remover_aparencia(A));
        assert_eq!(aparencia_aplicada(|| 0x999, Some(&c), Some(&p)), Some(0x222), "sem override no componente, vale o do prefixo");
        assert!(p.remover_aparencia(C));
        assert_eq!(aparencia_aplicada(|| 0x999, Some(&c), Some(&p)), Some(0x999), "sem override nenhum, a original volta");
    }

    #[test]
    fn aparencia_sem_mudanca_nao_escreve_e_vence_o_menor_hash() {
        assert_eq!(aparencia_aplicada(|| 0x999, Some(&ComponentState::novo()), None), None);
        let mut c = ComponentState::novo();
        c.sobrescrever_aparencia(B, 0x333);
        c.sobrescrever_aparencia(A, 0x444);
        assert_eq!(c.aparencia_sobrescrita(), Some(0x444));
    }

    #[test]
    fn remover_tudo_ainda_muda_e_aplica_a_original() {
        // `muda_mascara` não desliga: sem fonte nenhuma, a aplicação devolve a original e restaura o componente.
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        assert!(s.remover_fonte(A));
        assert!(s.muda_mascara());
        assert_eq!(mascara_aplicada(|| 0b1111, Some(&s), None), Some(0b1111));
    }

    #[test]
    fn remover_fonte_desfaz_o_override_dela_e_so_o_dela() {
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.esconder(B, !0b0100);
        assert!(s.remover_fonte(A));
        assert_eq!(s.mascara_final(0b1111), 0b1011, "sobra só o que B esconde");
        assert!(!s.remover_fonte(A), "remover de novo não mente que removeu");
    }

    #[test]
    fn remover_fonte_limpa_hiding_E_showing_dela() {
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.mostrar(A, 0b1000);
        assert!(s.remover_fonte(A));
        assert_eq!(s.fontes(), 0);
        assert_eq!(s.mascara_final(0b1111), 0b1111);
    }

    #[test]
    fn remover_fonte_e_fontes_ficam_so_na_mascara() {
        // `remover_fonte` só mexe na máscara; a aparência sai pelo `remover_aparencia`.
        let mut s = ComponentState::novo();
        s.sobrescrever_aparencia(A, 0x111);
        assert_eq!(s.fontes(), 0, "fonte só de aparência não conta");
        assert!(!s.remover_fonte(A), "sem máscara de A não há o que tirar");
        s.esconder(A, !0b10);
        assert_eq!(s.fontes(), 1);
        assert!(s.remover_fonte(A));
        assert_eq!(s.fontes(), 0);
        assert_eq!(s.mascara_final(0b1111), 0b1111);
        assert_eq!(s.aparencia_sobrescrita(), Some(0x111), "a aparência de A fica até o remover_aparencia");
        assert!(s.remover_aparencia(A));
        assert_eq!(s.aparencia_sobrescrita(), None);
    }

    #[test]
    fn restaurar_com_aparencia_original_zero_nao_escreve() {
        // Fixa o comportamento atual: original `0` = sem escrita, e o override removido fica na tela. Ver o TODO em `aparencia_aplicada`.
        let mut c = ComponentState::novo();
        c.sobrescrever_aparencia(A, 0x111);
        assert_eq!(aparencia_aplicada(|| 0, Some(&c), None), Some(0x111));
        assert!(c.remover_aparencia(A));
        assert_eq!(aparencia_aplicada(|| 0, Some(&c), None), None);
        assert_eq!(aparencia_aplicada(|| 0x999, Some(&c), None), Some(0x999), "com original não nula a restauração escreve");
    }

    #[test]
    fn mascara_de_esconder_sem_fonte_e_tudo_ligado() {
        // `!0` = "não esconde nada". Devolver 0 aqui inverteria o sentido e esconderia tudo.
        assert_eq!(ComponentState::novo().mascara_de_esconder(), u64::MAX);
    }

    #[test]
    fn mascara_de_esconder_e_a_intersecao() {
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.esconder(B, !0b0100);
        assert_eq!(s.mascara_de_esconder(), !0b0110);
    }

    #[test]
    fn fontes_conta_chave_distinta_nao_entrada() {
        let mut s = ComponentState::novo();
        s.esconder(A, !0b1);
        s.mostrar(A, 0b10);
        s.esconder(B, !0b100);
        assert_eq!(s.fontes(), 2, "A aparece nos 2 mapas mas é 1 fonte");
    }

    #[test]
    fn aplicada_combina_componente_e_prefixo() {
        let mut c = ComponentState::novo();
        c.esconder(A, !0b0010);
        c.mostrar(B, 0b1_0000);
        let mut p = ComponentState::novo();
        p.esconder(C, !0b0100);
        p.mostrar(C, 0b0010);
        // (0b0001 | 0b1_0000 | 0b0010) & !0b0010 & !0b0100: o mostrar do prefixo não vence o esconder do componente.
        assert_eq!(mascara_aplicada(|| 0b0001, Some(&c), Some(&p)), Some(0b1_0001));
        assert_eq!(mascara_aplicada(|| 0b0001, None, Some(&p)), Some(0b0011));
    }

    #[test]
    fn aplicada_sem_mudanca_nao_escreve() {
        let vazio = ComponentState::novo();
        assert_eq!(mascara_aplicada(|| 0b1111, Some(&vazio), None), None);
        assert_eq!(mascara_aplicada(|| 0b1111, None, None), None);
    }

    #[test]
    fn original_so_e_lida_quando_algum_estado_muda() {
        // A 1ª leitura da original grava o cache (`es20_original`): ler sem precisar congelaria o valor de um componente que nenhum override tocou.
        let leituras = std::cell::Cell::new(0u32);
        let original = |v: u64| {
            let leituras = &leituras;
            move || {
                leituras.set(leituras.get() + 1);
                v
            }
        };
        let vazio = ComponentState::novo();
        let mut com_aparencia = ComponentState::novo();
        com_aparencia.sobrescrever_aparencia(A, 0x111);
        let mut com_mascara = ComponentState::novo();
        com_mascara.esconder(A, !0b10);

        assert_eq!(aparencia_aplicada(original(0x999), Some(&vazio), Some(&vazio)), None);
        assert_eq!(aparencia_aplicada(original(0x999), Some(&com_aparencia), None), Some(0x111), "override do componente");
        assert_eq!(aparencia_aplicada(original(0x999), Some(&vazio), Some(&com_aparencia)), Some(0x111), "override do prefixo");
        assert_eq!(mascara_aplicada(original(0b1111), Some(&vazio), None), None);
        assert_eq!(mascara_aplicada(original(0b1111), None, None), None);
        assert_eq!(leituras.get(), 0, "sem restauração nem máscara mudada a original não é lida");

        assert!(com_aparencia.remover_aparencia(A));
        assert_eq!(aparencia_aplicada(original(0x999), Some(&com_aparencia), None), Some(0x999), "restauração");
        assert_eq!(leituras.get(), 1);
        assert_eq!(mascara_aplicada(original(0b1111), None, Some(&com_mascara)), Some(0b1101));
        assert_eq!(leituras.get(), 2, "uma leitura por aplicação que muda");
    }

    #[test]
    fn cenario_de_dois_mods_reais() {
        // Um mod de corpo esconde o torso; um mod de jaqueta força o ombro a aparecer.
        const TORSO: u64 = 1 << 3;
        const OMBRO: u64 = 1 << 7;
        const CORPO: u64 = 0x1111;
        const JAQUETA: u64 = 0x2222;
        let mut s = ComponentState::novo();
        s.esconder(CORPO, !TORSO);
        s.mostrar(JAQUETA, OMBRO);
        let final_ = s.mascara_final(u64::MAX);
        assert_eq!(final_ & TORSO, 0, "torso escondido pelo mod de corpo");
        assert_ne!(final_ & OMBRO, 0, "ombro forçado pela jaqueta");
        // Desligar o mod de corpo devolve o torso sem mexer na jaqueta.
        s.remover_fonte(CORPO);
        let depois = s.mascara_final(u64::MAX);
        assert_ne!(depois & TORSO, 0);
        assert_ne!(depois & OMBRO, 0);
    }

    /// A sequência de máscara e aparência dos vetores V01/V02 numa instância só.
    #[test]
    fn sequencia_de_mascara_e_aparencia_dos_vetores() {
        let mut s = ComponentState::novo();
        s.esconder(A, !0xF);
        s.esconder(B, !0xF0);
        s.esconder(A, !0x100);
        assert_eq!((s.fontes(), s.mascara_de_esconder()), (2, 0xffff_ffff_ffff_fe00));
        assert!(s.remover_fonte(A));
        assert_eq!(s.mascara_final(!0), 0xffff_ffff_ffff_ff0f);
        assert!(!s.remover_fonte(0xDEAD));
        assert!(s.remover_fonte(B));
        assert_eq!((s.fontes(), s.muda_mascara(), s.mascara_final(0x1234)), (0, true, 0x1234));
        s.mostrar(A, 1);
        s.mostrar(B, 2);
        s.mostrar(A, 4);
        assert_eq!((s.fontes(), s.mascara_de_mostrar()), (2, 7));
        s.esconder(C, !1);
        assert_eq!(s.mascara_final(0), 6);
        s.esconder(A, !2);
        assert_eq!((s.fontes(), s.mascara_final(0)), (3, 4));
        assert!(s.remover_fonte(A));
        assert_eq!(s.mascara_final(0), 2);
        let mut zero = ComponentState::novo();
        zero.esconder(A, 0);
        assert_eq!(zero.mascara_final(!0), 0);
        let mut nada = ComponentState::novo();
        nada.esconder(A, !0);
        assert!(nada.muda_mascara());
        assert_eq!(nada.mascara_final(0x55), 0x55);

        let mut ap = ComponentState::novo();
        assert_eq!((ap.aparencia_sobrescrita(), ap.muda_aparencia()), (None, false));
        ap.sobrescrever_aparencia(0xBB, 0x222);
        ap.sobrescrever_aparencia(0xAA, 0x111);
        assert_eq!(ap.aparencia_sobrescrita(), Some(0x111));
        ap.sobrescrever_aparencia(0xAA, 0x333);
        assert_eq!(ap.aparencia_sobrescrita(), Some(0x333));
        assert!(ap.remover_aparencia(0xAA));
        assert!(!ap.remover_aparencia(0xAA));
        assert!(ap.remover_aparencia(0xBB));
        assert_eq!((ap.aparencia_sobrescrita(), ap.muda_aparencia()), (None, true));
        ap.esconder(7, !1);
        ap.sobrescrever_aparencia(7, 0x700);
        assert!(ap.remover_fonte(7));
        assert_eq!(ap.aparencia_sobrescrita(), Some(0x700));
    }

    /// V03b e V04: as primitivas puras e a combinação componente + prefixo.
    #[test]
    fn mascaras_e_aplicacao_dos_vetores() {
        assert_eq!(mascara_de_chunks(true, &[2, 2, 2]), Some(4));
        assert_eq!(mascara_de_chunks(false, &[0, 1, 2, 3]), Some(0xffff_ffff_ffff_fff0));
        assert_eq!(mascara_de_bits(false, 5), 0xffff_ffff_ffff_fffa);
        assert_eq!(mascara_de_bits(false, !0), 0);
        assert_eq!(combinar_mascara(0xf0, 1, !1), 0xf0);
        let mut c = ComponentState::novo();
        c.esconder(A, !3);
        let mut p = ComponentState::novo();
        p.mostrar(B, 0xC);
        assert_eq!(mascara_aplicada(|| 0x11, Some(&c), Some(&p)), Some(0x1c));
        let mut p8 = ComponentState::novo();
        p8.esconder(B, !8);
        assert_eq!(mascara_aplicada(|| 0xFF, Some(&c), Some(&p8)), Some(0xf4));
        assert_eq!(mascara_aplicada(|| 0xFF, None, Some(&p8)), Some(0xf7));
    }
}
