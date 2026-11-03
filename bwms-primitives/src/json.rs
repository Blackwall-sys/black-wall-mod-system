//! JSON (RFC 8259) dos `.json` que o BWMS lê e grava: o estado e o registro da biblioteca antiga (`.cp77-mods/bwms-mods.json` e
//! `BWMS/nexus/deploy.json`, lidos pela `instalacao::migracao`) e as configs que o jogo aplica (`instalacao::xl_no_jogo`). No runtime (`cp77-console`), o mesmo
//! leitor lê o `.ips` de crash do macOS (`relatorio_de_crash`) e o comando `readjson`. Um leitor e um
//! escritor só; mora nesta folha porque o runtime e as ferramentas dependem dela.
//!
//! O leitor é estrito: o que não é JSON vira `Err` com linha e coluna. Quem chama precisa saber que o
//! arquivo está corrompido para recusá-lo, em vez de reconstruir o estado por cima dele.

use std::fmt::Write as _;

/// Valor lido. O número fica como o texto validado pela gramática: cada campo converte para o tipo
/// que espera (`i32`, `u32`) e recusa o que não cabe, sem passar por `f64`.
#[derive(Debug, PartialEq)]
pub enum Valor {
    Nulo,
    Bool(bool),
    Numero(String),
    Texto(String),
    Lista(Vec<Valor>),
    Objeto(Vec<(String, Valor)>),
}

impl Valor {
    /// Valor da chave num objeto (`None` se não é objeto ou não tem a chave). Chave repetida: vale a primeira.
    pub fn campo(&self, chave: &str) -> Option<&Valor> {
        match self {
            Valor::Objeto(pares) => pares.iter().find(|(k, _)| k == chave).map(|(_, v)| v),
            _ => None,
        }
    }
}

// O leitor é recursivo: um arquivo corrompido com milhares de '[' não pode estourar a pilha e derrubar
// o processo. Os arquivos de estado têm 3 níveis; o `.ips` do macOS chega a 6 (222 `.ips` de
// `cp77-symbols/notes/crashes` e `proofs`).
const PROFUNDIDADE_MAXIMA: usize = 64;

/// Lê um documento JSON inteiro. Conteúdo depois do valor do topo (fora espaço) é erro.
pub fn ler(fonte: &str) -> Result<Valor, String> {
    let mut leitor = Leitor { fonte, b: fonte.as_bytes(), pos: 0 };
    leitor.espacos();
    let valor = leitor.valor(0)?;
    leitor.espacos();
    if leitor.pos < leitor.b.len() {
        return Err(leitor.erro("conteúdo depois do fim do JSON"));
    }
    Ok(valor)
}

/// `s` como texto JSON, com as aspas. Controle < 0x20 sai escapado: cru é JSON inválido (RFC 8259 §7)
/// e o `ler` acima o recusa.
pub fn texto(s: &str) -> String {
    let mut saida = String::with_capacity(s.len() + 2);
    saida.push('"');
    for c in s.chars() {
        match c {
            '"' => saida.push_str("\\\""),
            '\\' => saida.push_str("\\\\"),
            '\n' => saida.push_str("\\n"),
            '\r' => saida.push_str("\\r"),
            '\t' => saida.push_str("\\t"),
            '\u{8}' => saida.push_str("\\b"),
            '\u{c}' => saida.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(saida, "\\u{:04x}", c as u32);
            }
            c => saida.push(c),
        }
    }
    saida.push('"');
    saida
}

struct Leitor<'a> {
    fonte: &'a str,
    b: &'a [u8],
    pos: usize,
}

impl Leitor<'_> {
    fn erro(&self, o_que: &str) -> String {
        let antes = &self.b[..self.pos.min(self.b.len())];
        let linha = antes.iter().filter(|&&c| c == b'\n').count() + 1;
        let inicio_da_linha = antes.iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
        let coluna = String::from_utf8_lossy(&antes[inicio_da_linha..]).chars().count() + 1;
        format!("{o_que} (linha {linha}, coluna {coluna})")
    }

    fn espacos(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.b.get(self.pos) {
            self.pos += 1;
        }
    }

    fn consome(&mut self, c: u8) -> bool {
        if self.b.get(self.pos) == Some(&c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn valor(&mut self, profundidade: usize) -> Result<Valor, String> {
        if profundidade > PROFUNDIDADE_MAXIMA {
            return Err(self.erro("aninhamento fundo demais"));
        }
        match self.b.get(self.pos) {
            None => Err(self.erro("fim do arquivo no meio do JSON")),
            Some(b'{') => self.objeto(profundidade),
            Some(b'[') => self.lista(profundidade),
            Some(b'"') => self.texto().map(Valor::Texto),
            Some(b't') => self.literal("true", Valor::Bool(true)),
            Some(b'f') => self.literal("false", Valor::Bool(false)),
            Some(b'n') => self.literal("null", Valor::Nulo),
            Some(b'-' | b'0'..=b'9') => self.numero(),
            Some(_) => Err(self.erro("valor inesperado")),
        }
    }

    fn literal(&mut self, palavra: &str, valor: Valor) -> Result<Valor, String> {
        if self.b[self.pos..].starts_with(palavra.as_bytes()) {
            self.pos += palavra.len();
            Ok(valor)
        } else {
            Err(self.erro("valor inesperado"))
        }
    }

    /// Devolve se consumiu ao menos um dígito.
    fn digitos(&mut self) -> bool {
        let inicio = self.pos;
        while let Some(b'0'..=b'9') = self.b.get(self.pos) {
            self.pos += 1;
        }
        self.pos > inicio
    }

    fn numero(&mut self) -> Result<Valor, String> {
        let inicio = self.pos;
        self.consome(b'-');
        match self.b.get(self.pos) {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digitos();
            }
            _ => return Err(self.erro("número sem dígitos")),
        }
        if self.consome(b'.') && !self.digitos() {
            return Err(self.erro("número sem dígitos depois do ponto"));
        }
        if let Some(b'e' | b'E') = self.b.get(self.pos) {
            self.pos += 1;
            if let Some(b'+' | b'-') = self.b.get(self.pos) {
                self.pos += 1;
            }
            if !self.digitos() {
                return Err(self.erro("expoente sem dígitos"));
            }
        }
        Ok(Valor::Numero(self.fonte[inicio..self.pos].to_string()))
    }

    fn texto(&mut self) -> Result<String, String> {
        self.pos += 1;
        let mut saida = String::new();
        // Trechos sem escape são copiados inteiros; o corte só cai em byte ASCII ('"' ou '\\'), que é
        // sempre fronteira de caractere UTF-8.
        let mut trecho = self.pos;
        loop {
            match self.b.get(self.pos) {
                None => return Err(self.erro("texto sem aspas de fechamento")),
                Some(b'"') => {
                    saida.push_str(&self.fonte[trecho..self.pos]);
                    self.pos += 1;
                    return Ok(saida);
                }
                Some(b'\\') => {
                    saida.push_str(&self.fonte[trecho..self.pos]);
                    self.pos += 1;
                    saida.push(self.escape()?);
                    trecho = self.pos;
                }
                Some(&c) if c < 0x20 => return Err(self.erro("caractere de controle cru dentro de texto")),
                Some(_) => self.pos += 1,
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let c = match self.b.get(self.pos) {
            Some(b'"') => '"',
            Some(b'\\') => '\\',
            Some(b'/') => '/',
            Some(b'b') => '\u{8}',
            Some(b'f') => '\u{c}',
            Some(b'n') => '\n',
            Some(b'r') => '\r',
            Some(b't') => '\t',
            Some(b'u') => {
                self.pos += 1;
                return self.escape_unicode();
            }
            _ => return Err(self.erro("escape inválido")),
        };
        self.pos += 1;
        Ok(c)
    }

    /// `\uXXXX`. Fora do plano básico o JSON usa par substituto (`\ud83d\ude00`); metade solta é erro.
    fn escape_unicode(&mut self) -> Result<char, String> {
        let alto = self.hex4()?;
        let ponto = match alto {
            0xD800..=0xDBFF => {
                if !self.b[self.pos..].starts_with(b"\\u") {
                    return Err(self.erro("par substituto sem a metade baixa"));
                }
                self.pos += 2;
                let baixo = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&baixo) {
                    return Err(self.erro("par substituto sem a metade baixa"));
                }
                0x10000 + ((alto - 0xD800) << 10) + (baixo - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(self.erro("par substituto sem a metade alta")),
            _ => alto,
        };
        char::from_u32(ponto).ok_or_else(|| self.erro("escape \\u inválido"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let b = self.b;
        let Some(digitos) = b.get(self.pos..self.pos + 4) else {
            return Err(self.erro("escape \\u com menos de 4 dígitos"));
        };
        let mut v = 0u32;
        for &d in digitos {
            let Some(n) = (d as char).to_digit(16) else {
                return Err(self.erro("escape \\u com dígito que não é hexadecimal"));
            };
            v = v * 16 + n;
        }
        self.pos += 4;
        Ok(v)
    }

    fn lista(&mut self, profundidade: usize) -> Result<Valor, String> {
        self.pos += 1;
        let mut itens = Vec::new();
        self.espacos();
        if self.consome(b']') {
            return Ok(Valor::Lista(itens));
        }
        loop {
            self.espacos();
            itens.push(self.valor(profundidade + 1)?);
            self.espacos();
            if self.consome(b',') {
                continue;
            }
            if self.consome(b']') {
                return Ok(Valor::Lista(itens));
            }
            return Err(self.erro("esperava ',' ou ']'"));
        }
    }

    fn objeto(&mut self, profundidade: usize) -> Result<Valor, String> {
        self.pos += 1;
        let mut pares = Vec::new();
        self.espacos();
        if self.consome(b'}') {
            return Ok(Valor::Objeto(pares));
        }
        loop {
            self.espacos();
            if self.b.get(self.pos) != Some(&b'"') {
                return Err(self.erro("esperava o nome de uma chave entre aspas"));
            }
            let chave = self.texto()?;
            self.espacos();
            if !self.consome(b':') {
                return Err(self.erro("esperava ':' depois da chave"));
            }
            self.espacos();
            let valor = self.valor(profundidade + 1)?;
            pares.push((chave, valor));
            self.espacos();
            if self.consome(b',') {
                continue;
            }
            if self.consome(b'}') {
                return Ok(Valor::Objeto(pares));
            }
            return Err(self.erro("esperava ',' ou '}'"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(s: &str) -> Valor {
        Valor::Texto(s.into())
    }

    #[test]
    fn le_documento_com_espacos_e_todos_os_tipos() {
        let v = ler(" {\r\n\t\"a\" : [ 1 , -2.5e3 , true , false , null ] , \"b\" : \"x\" } ").unwrap();
        assert_eq!(
            v,
            Valor::Objeto(vec![
                (
                    "a".into(),
                    Valor::Lista(vec![
                        Valor::Numero("1".into()),
                        Valor::Numero("-2.5e3".into()),
                        Valor::Bool(true),
                        Valor::Bool(false),
                        Valor::Nulo,
                    ])
                ),
                ("b".into(), txt("x")),
            ])
        );
        assert_eq!(v.campo("b"), Some(&txt("x")));
        assert_eq!(v.campo("c"), None);
    }

    #[test]
    fn chave_repetida_vale_a_primeira() {
        let v = ler(r#"{"k":"1","k":"2"}"#).unwrap();
        assert_eq!(v.campo("k"), Some(&txt("1")));
    }

    #[test]
    fn decodifica_todos_os_escapes() {
        let v = ler(r#""\" \\ \/ \b \f \n \r \t \u00e9 \ud83d\ude00 }{""#).unwrap();
        assert_eq!(v, txt("\" \\ / \u{8} \u{c} \n \r \t é 😀 }{"));
    }

    #[test]
    fn recusa_o_que_nao_e_json() {
        for ruim in [
            "",
            "[",
            "[1,]",
            r#"{"a":1,}"#,
            r#"{"a" 1}"#,
            r#"{a:1}"#,
            "[1 2]",
            "01",
            "1.",
            "1e",
            "-",
            "trueX",
            "[] []",
            "\"sem fim",
            "\"a\tb\"",
            "\"\\x\"",
            "\"\\u12\"",
            "\"\\u12G4\"",
            "\"\\ud83d\"",
            "\"\\ud83d\\u0041\"",
            "\"\\ude00\"",
            "\u{feff}[]",
        ] {
            assert!(ler(ruim).is_err(), "devia recusar {ruim:?}");
        }
    }

    #[test]
    fn erro_diz_linha_e_coluna() {
        let e = ler("[\n  {\"a\":1,}\n]").unwrap_err();
        assert!(e.ends_with("(linha 2, coluna 10)"), "{e}");
    }

    #[test]
    fn aninhamento_fundo_e_erro_e_nao_estouro_de_pilha() {
        let fundo = "[".repeat(100_000);
        assert!(ler(&fundo).unwrap_err().starts_with("aninhamento fundo demais"));
        let raso = format!("{}{}", "[".repeat(PROFUNDIDADE_MAXIMA), "]".repeat(PROFUNDIDADE_MAXIMA));
        assert!(ler(&raso).is_ok());
    }

    #[test]
    fn texto_escapa_controle_e_volta_igual() {
        let original = "a\"b\\c\nd\re\tf\u{8}g\u{c}h\u{1}i\u{1f}j é 😀 /";
        let js = texto(original);
        assert!(!js.chars().any(|c| (c as u32) < 0x20), "{js:?}");
        assert!(js.contains("\\u0001") && js.contains("\\u001f"), "{js}");
        assert_eq!(ler(&js).unwrap(), txt(original));
    }
}
