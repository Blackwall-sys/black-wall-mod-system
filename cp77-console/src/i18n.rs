//! i18n.rs — idioma da UI do console (overlay ImGui).
//!
//! POR QUE EXISTE: até aqui o overlay era texto fixo. O jogo em si é totalmente localizado —
//! um jogador com o jogo em turco abre o console e cai numa janela em inglês no meio da UI dele.
//!
//! COMO O IDIOMA É ESCOLHIDO (nesta ordem, primeiro que resolver vence):
//!   1. `~/.bwms-lang` com um código (`en`, `pt`, `tr`) — override manual, e é onde o seletor
//!      da UI grava. Existe pra quem joga em um idioma e quer a ferramenta em outro.
//!   2. O idioma DE TELA do próprio jogo, lido de `UserSettings.json` (`/language` → `OnScreen`,
//!      valor tipo `tr-tr`). É o default certo: a ferramenta acompanha o jogo sem configurar nada.
//!   3. Inglês.
//!
//! `OnScreen` (não `VoiceOver`) de propósito: é o que manda no texto na tela. Numa instalação real
//! aqui, VoiceOver=`en-us` e OnScreen=`tr-tr` — usar o primeiro daria o idioma errado.
//!
//! O idioma é resolvido UMA vez (`OnceLock`): `t()` roda no caminho de render, todo frame.
//!
//! Não-objetivo: traduzir as linhas de LOG. Elas são diagnóstico, vão pro K-LOG e pro arquivo, e
//! ficam em inglês/PT como estão — quem lê log quer casar com o que está escrito no código.

use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Pt,
    Tr,
}

impl Lang {
    /// Código curto usado no `~/.bwms-lang` e no seletor da UI.
    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Pt => "pt",
            Lang::Tr => "tr",
        }
    }
    /// Nome exibido no seletor — cada um NO PRÓPRIO idioma (padrão de seletor de idioma:
    /// quem procura "Türkçe" não procura "Turkish").
    pub fn label(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Pt => "Português",
            Lang::Tr => "Türkçe",
        }
    }
    pub const ALL: [Lang; 3] = [Lang::En, Lang::Pt, Lang::Tr];

    /// Aceita `tr`, `tr-tr`, `TR_tr`… — só o prefixo antes de `-`/`_` importa.
    fn from_tag(tag: &str) -> Option<Lang> {
        let base = tag
            .split(|c| c == '-' || c == '_')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        match base.as_str() {
            "en" => Some(Lang::En),
            "pt" => Some(Lang::Pt),
            "tr" => Some(Lang::Tr),
            _ => None,
        }
    }
}

fn override_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".bwms-lang"))
}

/// Idioma de tela do jogo, de `UserSettings.json`. Parser mínimo de propósito: o arquivo é grande
/// e o projeto não tem dependência de JSON — procuramos a opção `OnScreen` e pegamos o `value` que
/// vem depois dela. Se o formato mudar, isto devolve `None` e caímos no inglês (nunca quebra).
fn game_language() -> Option<Lang> {
    let home = std::env::var_os("HOME")?;
    let p = std::path::Path::new(&home)
        .join("Library/Application Support/CD Projekt Red/Cyberpunk 2077/UserSettings.json");
    let s = std::fs::read_to_string(p).ok()?;
    let at = s.find("\"OnScreen\"")?;
    let rest = &s[at..];
    let v = rest.find("\"value\"")?;
    let after = &rest[v + 7..];
    let q1 = after.find('"')?;
    let after = &after[q1 + 1..];
    let q2 = after.find('"')?;
    Lang::from_tag(&after[..q2])
}

pub fn lang() -> Lang {
    static L: OnceLock<Lang> = OnceLock::new();
    *L.get_or_init(|| {
        if let Some(p) = override_path() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                if let Some(l) = Lang::from_tag(s.trim()) {
                    crate::log(&format!("[i18n] idioma = {} (override ~/.bwms-lang)", l.code()));
                    return l;
                }
            }
        }
        if let Some(l) = game_language() {
            crate::log(&format!("[i18n] idioma = {} (OnScreen do jogo)", l.code()));
            return l;
        }
        crate::log("[i18n] idioma = en (sem override e sem OnScreen legível)");
        Lang::En
    })
}

/// Grava o override e devolve se conseguiu. O idioma JÁ resolvido não muda nesta sessão
/// (`OnceLock`) — o seletor avisa que vale no próximo boot, em vez de fingir que trocou.
pub fn set_override(l: Lang) -> bool {
    match override_path() {
        Some(p) => std::fs::write(p, l.code()).is_ok(),
        None => false,
    }
}

/// Tradução por chave. Fallback: se faltar tradução, devolve o INGLÊS — nunca string vazia e
/// nunca a chave crua, porque isso vaza nome interno pra tela do jogador.
pub fn t(key: &str) -> &'static str {
    let (en, pt, tr) = match key {
        // --- abas / cabeçalho ---
        "tab.console" => ("Console", "Console", "Konsol"),
        "tab.items" => ("Items", "Itens", "Eşyalar"),
        "tab.favorites" => ("Favorites", "Favoritos", "Favoriler"),
        "tab.mods" => ("Mods", "Mods", "Modlar"),
        "tab.klog" => ("K-LOG", "K-LOG", "K-LOG"),
        "tab.lut" => ("LUT", "LUT", "LUT"),
        "hdr.theme" => ("theme", "tema", "tema"),
        "hdr.title" => (
            "BWMS  //  CP2077 console  ( ` toggles )",
            "BWMS  //  CP2077 console  ( ` alterna )",
            "BWMS  //  CP2077 konsolu  ( ` açar/kapatır )",
        ),
        "hdr.language" => ("Language", "Idioma", "Dil"),
        "hdr.language_next_boot" => (
            "language applies on the next boot",
            "o idioma vale no próximo boot",
            "dil bir sonraki açılışta geçerli olur",
        ),
        // --- items ---
        "items.filter" => (
            "filter (e.g. erebus, katana, money)",
            "filtrar (ex: erebus, katana, money)",
            "filtrele (ör: erebus, katana, money)",
        ),
        "items.refine" => ("... refine the search", "... refine a busca", "... aramayı daraltın"),
        "items.give" => ("Give", "Give", "Ver"),
        "items.pin" => ("+pin", "+pin", "+sabitle"),
        "items.qty" => ("qty", "qtd", "adet"),
        // --- favorites ---
        "fav.header" => (
            "Favorites (pinned from the Items tab):",
            "Favoritos (pinados na aba Itens):",
            "Favoriler (Eşyalar sekmesinden sabitlenenler):",
        ),
        "fav.empty" => (
            "No favorites yet. Pin items from the Items tab (+ button) to see them here.",
            "Sem favoritos. Pinhe itens na aba Itens (botão +) pra aparecerem aqui.",
            "Henüz favori yok. Eşyalar sekmesinden (+ düğmesi) sabitleyin, burada görünsün.",
        ),
        // --- mods ---
        "mods.empty" => (
            "No mods in BWMS/mods/ (yet)",
            "Nenhum mod em BWMS/mods/ (ainda)",
            "BWMS/mods/ içinde mod yok (henüz)",
        ),
        "mods.refresh" => ("Refresh", "Atualizar", "Yenile"),
        "mods.cheats_note" => (
            "Cheats (Godmode/Money/Perks/Level...) live in Settings > Mods — not duplicated here.",
            "Cheats (Godmode/Money/Perks/Level...) ficam em Settings > Mods, sem duplicar.",
            "Hileler (Godmode/Para/Perk/Seviye...) Settings > Mods altında — burada tekrarlanmıyor.",
        ),
        // --- k-log ---
        "klog.capture" => (
            "capture keys (keyCode/char)",
            "capturar teclas (keyCode/char)",
            "tuşları yakala (keyCode/char)",
        ),
        "klog.capture_note" => (
            "keys captured while the overlay is CLOSED (input/hotkey debug)",
            "teclas capturadas com o overlay FECHADO (debug de input/atalho)",
            "overlay KAPALIYKEN yakalanan tuşlar (girdi/kısayol hata ayıklama)",
        ),
        "klog.clear" => ("clear", "limpar", "temizle"),
        "klog.scan" => ("Scan", "Escanear", "Tara"),
        "klog.badge" => (
            "Show badge (top-right corner)",
            "Exibir badge (canto superior direito)",
            "Rozeti göster (sağ üst köşe)",
        ),
        // --- lut ---
        "lut.effects" => (
            "Effects (toggle on top of the grade):",
            "Efeitos (liga/desliga sobre a grade):",
            "Efektler (renk katmanının üstünde aç/kapat):",
        ),
        "lut.note" => (
            "Grade (color) + effects LIVE on the frame; each one toggles on/off.",
            "Grade (cor) + efeitos AO VIVO no frame; cada um liga/desliga.",
            "Renk katmanı + efektler kare üzerinde CANLI; her biri ayrı açılıp kapanır.",
        ),
        "lut.soon" => (
            "Coming soon (needs the render-pass upgrade): Bloom, Chromatic aberration, Barrel.",
            "Em breve (precisa do upgrade do passe): Bloom, Aberracao cromatica, Barril.",
            "Yakında (render geçişi güncellemesi gerekiyor): Bloom, Kromatik sapma, Fıçı.",
        ),
        _ => return "",
    };
    match lang() {
        Lang::En => en,
        Lang::Pt if !pt.is_empty() => pt,
        Lang::Tr if !tr.is_empty() => tr,
        _ => en,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_parsing_accepts_region_and_case() {
        assert_eq!(Lang::from_tag("tr-tr"), Some(Lang::Tr));
        assert_eq!(Lang::from_tag("TR_TR"), Some(Lang::Tr));
        assert_eq!(Lang::from_tag("pt-br"), Some(Lang::Pt));
        assert_eq!(Lang::from_tag("en-us"), Some(Lang::En));
        // idioma que não traduzimos cai no None -> chamador usa inglês
        assert_eq!(Lang::from_tag("ja-jp"), None);
        assert_eq!(Lang::from_tag(""), None);
    }

    /// Toda chave usada pela UI tem que existir; `t()` devolvendo "" seria rótulo invisível.
    #[test]
    fn every_key_resolves_in_every_language() {
        const KEYS: [&str; 25] = [
            "tab.console", "tab.items", "tab.favorites", "tab.mods", "tab.klog", "tab.lut",
            "hdr.theme", "hdr.title", "hdr.language", "hdr.language_next_boot",
            "items.filter", "items.refine", "items.give", "items.pin", "items.qty",
            "fav.header", "fav.empty", "mods.empty", "mods.refresh", "mods.cheats_note",
            "klog.capture", "klog.capture_note", "klog.clear", "klog.scan", "klog.badge",
        ];
        for k in KEYS {
            assert!(!t(k).is_empty(), "chave sem tradução: {k}");
        }
    }

    #[test]
    fn unknown_key_is_empty_not_the_key_itself() {
        // devolver a chave crua vazaria nome interno pra tela; "" é tratado pelo chamador.
        assert_eq!(t("nao.existe"), "");
    }
}
