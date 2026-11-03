//! `ComponentState` — combinação de overrides de MÚLTIPLAS FONTES (ArchiveXL `A#22`).
//!
//! **O que o item pede e por que é aqui.** Cada `ComponentState` guarda dois mapas
//! `hash → chunkMask` (um de *hiding*, outro de *showing*), onde **cada hash é uma FONTE/mod
//! distinta**. É isso que permite dois mods esconderem partes diferentes do mesmo componente sem
//! um apagar o outro. O cálculo do resultado final é lógica pura — e é a parte do item que dá pra
//! provar sem o jogo, separada da aplicação em objeto vivo (que é campo cru, já provada).
//!
//! **Ordem da aplicação real**, a de `App::EntityState::ApplyChunkMaskOverride`
//! (`enablers/ArchiveXL/src/App/Extensions/Garment/States.cpp:717-731`), que junta o estado do
//! componente e o do prefixo do nome:
//!
//! ```text
//! final = original
//! para cada showing: final |= showing
//! para cada hiding:  final &= hiding
//! ```
//!
//! Duas propriedades que essa ordem impõe, e que os testes fixam:
//! - **`hiding` vence `showing`**: todos os OR acontecem antes de qualquer AND, então uma fonte
//!   que esconde um chunk sobrepõe outra que o mostra. O `GetOverriddenChunkMask` da referência
//!   (`States.cpp:64-75`) faz o contrário, mas não tem chamador; o porte seguia ele até
//!   2026-09-13, e a máscara aplicada ao vivo sairia invertida.
//! - **A ordem DENTRO de cada grupo não importa**: AND e OR são associativos e comutativos, então
//!   o mapa desordenado do C++ e o nosso vetor ordenado dão o mesmo resultado. Isso é o que
//!   permite portar um `Core::Map` para qualquer container sem mudar semântica.

use std::collections::HashMap;

/// Estado de override de UM componente, acumulando fontes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ComponentState {
    /// fonte → máscara de esconder. Bit 0 = chunk escondido por essa fonte.
    hiding: HashMap<u64, u64>,
    /// fonte → máscara de mostrar. Bit 1 = chunk forçado a aparecer por essa fonte.
    showing: HashMap<u64, u64>,
    /// `m_chunkMaskChanged`: liga no 1º override e não desliga na remoção — voltar à original também é mudança a aplicar.
    mudou: bool,
    /// fonte → aparência (`CName`), em ordem de fonte.
    aparencias: std::collections::BTreeMap<u64, u64>,
    /// `m_appearanceChanged`, com a mesma regra da flag de máscara.
    mudou_aparencia: bool,
}

impl ComponentState {
    pub fn novo() -> Self {
        Self::default()
    }

    /// `AddHidingChunkMaskOverride`: AND no valor da fonte, partindo de `~0` — a 2ª máscara da mesma fonte soma à 1ª.
    pub fn esconder(&mut self, fonte: u64, mascara: u64) {
        *self.hiding.entry(fonte).or_insert(u64::MAX) &= mascara;
        self.mudou = true;
    }

    /// `AddShowingChunkMaskOverride`: OR no valor da fonte, partindo de 0.
    pub fn mostrar(&mut self, fonte: u64, mascara: u64) {
        *self.showing.entry(fonte).or_insert(0) |= mascara;
        self.mudou = true;
    }

    /// `RemoveChunkMaskOverride`: tira o esconder e o mostrar da fonte; a aparência sai por `remover_aparencia`. `true` se a fonte tinha
    /// máscara (a referência devolve sempre `true`).
    pub fn remover_fonte(&mut self, fonte: u64) -> bool {
        let h = self.hiding.remove(&fonte).is_some();
        let s = self.showing.remove(&fonte).is_some();
        h || s
    }

    /// Fontes com override de máscara (esconder ou mostrar); a aparência fica de fora, no mesmo escopo do `remover_fonte`.
    pub fn fontes(&self) -> usize {
        let mut set: std::collections::HashSet<u64> = self.hiding.keys().copied().collect();
        set.extend(self.showing.keys().copied());
        set.len()
    }

    /// O resultado final, combinando TODAS as fontes sobre a máscara original: OR de todos os
    /// showing, depois AND de todos os hiding (a ordem do `ApplyChunkMaskOverride`).
    pub fn mascara_final(&self, original: u64) -> u64 {
        combinar_mascara(original, self.mascara_de_mostrar(), self.mascara_de_esconder())
    }

    /// `GetShowingChunkMask` — a união de todas as máscaras de mostrar, partindo de zero.
    pub fn mascara_de_mostrar(&self) -> u64 {
        self.showing.values().fold(0, |acc, m| acc | m)
    }

    /// `ChangesChunkMask`: algum override já entrou, mesmo que removido depois.
    pub fn muda_mascara(&self) -> bool {
        self.mudou
    }

    /// `GetHidingChunkMask` — a interseção de todas as máscaras de esconder, partindo de tudo-ligado.
    /// Sem nenhuma fonte devolve `!0`, que é "não esconde nada" e não "esconde tudo".
    pub fn mascara_de_esconder(&self) -> u64 {
        self.hiding.values().fold(u64::MAX, |acc, m| acc & m)
    }

    /// `AddAppearanceOverride`: a aparência da fonte (substitui a anterior da mesma fonte).
    pub fn sobrescrever_aparencia(&mut self, fonte: u64, aparencia: u64) {
        self.aparencias.insert(fonte, aparencia);
        self.mudou_aparencia = true;
    }

    /// `RemoveAppearanceOverride`. `true` se a fonte tinha aparência.
    pub fn remover_aparencia(&mut self, fonte: u64) -> bool {
        self.aparencias.remove(&fonte).is_some()
    }

    /// `GetAppearanceOverridde` com override presente (`HasAppearanceOverriddes`): vence o menor hash de fonte, o 1º do `BTreeMap` de `aparencias`,
    /// qualquer que seja a ordem de inserção. A referência devolve o `begin()` de um `Core::Map` = `tsl::hopscotch_map` (`Stl.hpp:40`), sem
    /// ordem, então não tem regra; esta é a escolha determinística, e vale para as natives de `BwmsAparencia` e para o es20, que usam esta mesma loja.
    pub fn aparencia_sobrescrita(&self) -> Option<u64> {
        self.aparencias.values().next().copied()
    }

    /// `ChangesAppearance`: algum override de aparência já entrou, mesmo que removido depois.
    pub fn muda_aparencia(&self) -> bool {
        self.mudou_aparencia
    }
}

/// A álgebra única da máscara de chunks (`ApplyChunkMaskOverride`, `States.cpp:717-731`): `(original | mostrar) & esconder`. Todo OR antes de
/// qualquer AND, então o esconder vence o mostrar. `mostrar` é a união das fontes (`0` sem fonte); `esconder`, a interseção (`!0` sem fonte).
pub fn combinar_mascara(original: u64, mostrar: u64, esconder: u64) -> u64 {
    (original | mostrar) & esconder
}

/// `ChunkMask::Set` (`Garment/ChunkMask.hpp:53-67`): lista de chunks → máscara, para a tabela de tags do runtime e o parser do `.xl`. OR de
/// `1 << chunk`, e esconder com lista inverte (`if (!set && mask)`, `:63-66`). Esconder sem lista fica `0`, que no AND de `combinar_mascara`
/// esconde tudo; mostrar sem lista fica `0` e não mostra nada. Shift em `u64`: na referência o `1` é `int` (C++20, `xmake.lua:7`), então o
/// chunk 31 sai com extensão de sinal (`0xFFFFFFFF80000000`) e de 32 em diante é UB. `None` com chunk > 63: tirar só o item mudaria o pedido
/// (`[64]` viraria a lista vazia, o esconder-tudo), então quem chama recusa a entrada inteira.
pub const fn mascara_de_chunks(mostrar: bool, chunks: &[u8]) -> Option<u64> {
    let mut mascara = 0u64;
    let mut i = 0;
    // `for` sobre iterador não roda em `const fn`
    while i < chunks.len() {
        if chunks[i] > 63 {
            return None;
        }
        mascara |= 1u64 << chunks[i];
        i += 1;
    }
    Some(mascara_de_bits(mostrar, mascara))
}

/// `ChunkMask(op, bits)`: `hide` inverte os bits dos chunks, menos a máscara vazia.
pub const fn mascara_de_bits(mostrar: bool, bits: u64) -> u64 {
    if !mostrar && bits != 0 {
        !bits
    } else {
        bits
    }
}

/// `EntityState::ApplyAppearanceOverride` sem a resolução de nome dinâmico: o override do componente, senão o do prefixo, senão a original quando algum
/// dos dois já mudou a aparência (restauração). `None` = a referência sai sem escrever. `original` só roda no ramo de restauração, como o
/// `GetOriginalAppearance` da referência (`States.cpp:684-686`): a 1ª leitura grava o cache de original, e componente sem mudança não pode gravá-lo.
pub fn aparencia_aplicada(original: impl FnOnce() -> u64, componente: Option<&ComponentState>, prefixo: Option<&ComponentState>) -> Option<u64> {
    let mudou = componente.is_some_and(ComponentState::muda_aparencia) || prefixo.is_some_and(ComponentState::muda_aparencia);
    // TODO(A#22 22E, original 0): original `0` (CName None) dá `None`, como a referência (`States.cpp:689-690`), e o override removido fica na
    // tela. Escrever 0 + `RefreshAppearance` só restaura se o motor tratar `meshAppearance` None no refresh, e isso não está provado: falta a
    // variante do [a22e] com componente de mesh do V de aparência 0 (gravar X, gravar 0, slot 81 `0x101991450`, ler de volta, conferir a tela,
    // prova em `proofs/`). A prova do boot 147 usou original "default" e não cobre este caso. Com a prova, os 2 pontos juntos (cp77-console):
    // 1. `es20_original` e `es20_original_da_aparencia` devolvem `Option<u64>`: lock envenenado, leitura que falha ou `Es20Malhas` sem resolver
    //    (a closure de `aplicar_aparencia_texto`) = `None`, sem escrita. Hoje dão 0 nesses casos, e sem o filtro esse 0 viraria CName None
    //    escrito no componente.
    // 2. Aqui: `original: impl FnOnce() -> Option<u64>` e `.filter(|a| *a != 0).or_else(|| mudou.then(original).flatten())`, com o filtro só nos
    //    overrides. `es20_aplicar_aparencia`, `aplicar_aparencia_composta` e `aplicar_aparencia_texto` passam por aqui e só trocam a closure.
    // No dylib público o ramo de restaurar é inalcançável: `BwmsAparencia` não tem `RemoveAppearanceOverride`.
    componente
        .and_then(ComponentState::aparencia_sobrescrita)
        .or_else(|| prefixo.and_then(ComponentState::aparencia_sobrescrita))
        .or_else(|| mudou.then(original))
        .filter(|a| *a != 0)
}

/// `EntityState::ApplyChunkMaskOverride` com o estado do prefixo: OR do mostrar do componente e do
/// prefixo, depois AND do esconder dos dois. `None` quando nenhum dos dois muda a máscara — a
/// referência sai sem escrever nada nesse caso. `original` só roda depois desse teste, como o
/// `GetOriginalChunkMask` da referência (`States.cpp:713-717`).
pub fn mascara_aplicada(original: impl FnOnce() -> u64, componente: Option<&ComponentState>, prefixo: Option<&ComponentState>) -> Option<u64> {
    if !componente.is_some_and(ComponentState::muda_mascara) && !prefixo.is_some_and(ComponentState::muda_mascara) {
        return None;
    }
    let estados = [componente, prefixo];
    let mostrar = estados.iter().flatten().fold(0, |acc, s| acc | s.mascara_de_mostrar());
    let esconder = estados.iter().flatten().fold(u64::MAX, |acc, s| acc & s.mascara_de_esconder());
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
        // Todos os OR antes de qualquer AND, como no `ApplyChunkMaskOverride`; a ordem do
        // `GetOverriddenChunkMask` (sem chamador) daria 0b1111.
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
    fn mascara_de_chunks_bate_com_o_full_body_e_a_lista_vazia() {
        // `chunkMask` de `t0_000_pwa_fpp__01_ca_pale` = `hide: [5, 6, 7]`, avaliada em `const` (a primitiva serve tabela `static`).
        const FULL_BODY: Option<u64> = mascara_de_chunks(false, &[5, 6, 7]);
        assert_eq!(FULL_BODY, Some(0xFFFF_FFFF_FFFF_FF1F));
        assert_eq!(mascara_de_chunks(true, &[0, 1, 2]), Some(0b111), "mostrar não inverte");
        assert_eq!(mascara_de_chunks(false, &[]), Some(0), "esconder sem lista = esconde tudo no AND");
        assert_eq!(mascara_de_chunks(true, &[]), Some(0), "mostrar sem lista não mostra nada");
        assert_eq!(combinar_mascara(u64::MAX, 0, mascara_de_chunks(false, &[]).unwrap()), 0);
    }

    #[test]
    fn mascara_de_chunks_vai_ate_o_63_e_recusa_a_lista_com_64() {
        assert_eq!(mascara_de_chunks(true, &[63]), Some(1u64 << 63));
        // Na referência o `1` é `int`: o chunk 31 sairia `0xFFFFFFFF80000000`, e o 32 é UB.
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
        // É o que permite portar `Core::Map` (desordenado) pra qualquer container.
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
        // `AddHidingChunkMaskOverride` faz AND no valor da chave (partindo de `~0`): a 2ª máscara da mesma fonte soma à 1ª.
        let mut s = ComponentState::novo();
        s.esconder(A, !0b0010);
        s.esconder(A, !0b0100);
        assert_eq!(s.mascara_final(0b1111), 0b1001, "as duas valem");
        assert_eq!(s.fontes(), 1);
    }

    #[test]
    fn mesma_fonte_mostrando_de_novo_acumula_por_or() {
        // `AddShowingChunkMaskOverride` faz OR no valor da chave (partindo de 0): a 2ª máscara da mesma fonte soma à 1ª.
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
        // `ChangesChunkMask` não desliga: sem fonte nenhuma, a aplicação devolve a original e restaura o componente.
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
        // 1:1 com `RemoveChunkMaskOverride` (`States.cpp:56-62`); a aparência é o par separado `RemoveAppearanceOverride` (`:119-122`).
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
        // Fixa o comportamento atual (o da referência): original `0` = sem escrita, e o override removido fica na tela. Ver o TODO em `aparencia_aplicada`.
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
        // full-body esconde o torso; um mod de jaqueta força o ombro a aparecer.
        const TORSO: u64 = 1 << 3;
        const OMBRO: u64 = 1 << 7;
        const FULLBODY: u64 = 0x1111;
        const JAQUETA: u64 = 0x2222;
        let mut s = ComponentState::novo();
        s.esconder(FULLBODY, !TORSO);
        s.mostrar(JAQUETA, OMBRO);
        let final_ = s.mascara_final(u64::MAX);
        assert_eq!(final_ & TORSO, 0, "torso escondido pelo full-body");
        assert_ne!(final_ & OMBRO, 0, "ombro forçado pela jaqueta");
        // e desligar o full-body devolve o torso sem mexer na jaqueta
        s.remover_fonte(FULLBODY);
        let depois = s.mascara_final(u64::MAX);
        assert_ne!(depois & TORSO, 0);
        assert_ne!(depois & OMBRO, 0);
    }
}
