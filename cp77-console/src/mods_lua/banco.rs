//! `db` dos mods Lua: o SQL que os mods usam (`CREATE TABLE`, `INSERT … VALUES`, `INSERT … SELECT … WHERE NOT EXISTS (…)`, `UPDATE … SET …
//! WHERE`, `SELECT … FROM … WHERE`, `DELETE FROM … WHERE`), um banco por mod gravado em `<mod>/db.bwms`. Os mods esperam SQLite; aqui é Rust.
//! Semântica do SQLite onde o mod depende dela: `exec` com vários comandos para no primeiro erro (o `CREATE TABLE` de uma tabela que já
//! existe falha e pula os `INSERT` do mesmo `exec`), `true`/`false` valem 1/0 e `"texto"` sem coluna desse nome é texto.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Valor {
    Nulo,
    Inteiro(i64),
    Real(f64),
    Texto(String),
}

impl Valor {
    /// Igualdade do `=` do SQLite: número com número pelo valor, texto com texto; `NULL` nunca é igual.
    fn igual(&self, outro: &Valor) -> bool {
        match (self, outro) {
            (Valor::Inteiro(a), Valor::Inteiro(b)) => a == b,
            (Valor::Inteiro(a), Valor::Real(b)) | (Valor::Real(b), Valor::Inteiro(a)) => *a as f64 == *b,
            (Valor::Real(a), Valor::Real(b)) => a == b,
            (Valor::Texto(a), Valor::Texto(b)) => a == b,
            _ => false,
        }
    }
}

struct Tabela {
    nome: String,
    colunas: Vec<String>,
    linhas: Vec<Vec<Valor>>,
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Palavra(String),
    Numero(Valor),
    Texto(String),
    Aspas(String),
    Simbolo(char),
}

fn tokens(sql: &str) -> Result<Vec<Token>, String> {
    let c: Vec<char> = sql.chars().collect();
    let mut t = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let ch = c[i];
        if ch.is_whitespace() {
            i += 1;
        } else if ch == '-' && c.get(i + 1) == Some(&'-') {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let ini = i;
            while i < c.len() && (c[i].is_ascii_alphanumeric() || c[i] == '_') {
                i += 1;
            }
            t.push(Token::Palavra(c[ini..i].iter().collect()));
        } else if ch.is_ascii_digit() || (ch == '.' && c.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let ini = i;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '.' || c[i] == 'e' || c[i] == 'E'
                || ((c[i] == '-' || c[i] == '+') && matches!(c[i - 1], 'e' | 'E')))
            {
                i += 1;
            }
            let s: String = c[ini..i].iter().collect();
            let v = match s.parse::<i64>() {
                Ok(n) => Valor::Inteiro(n),
                Err(_) => Valor::Real(s.parse::<f64>().map_err(|_| format!("número inválido: {s}"))?),
            };
            t.push(Token::Numero(v));
        } else if ch == '\'' || ch == '"' {
            let mut s = String::new();
            i += 1;
            loop {
                match c.get(i) {
                    None => return Err("texto sem fim".into()),
                    Some(&q) if q == ch && c.get(i + 1) == Some(&ch) => {
                        s.push(ch);
                        i += 2;
                    }
                    Some(&q) if q == ch => {
                        i += 1;
                        break;
                    }
                    Some(&o) => {
                        s.push(o);
                        i += 1;
                    }
                }
            }
            t.push(if ch == '\'' { Token::Texto(s) } else { Token::Aspas(s) });
        } else if "(),;=*-+<>!".contains(ch) {
            t.push(Token::Simbolo(ch));
            i += 1;
        } else {
            return Err(format!("caractere inesperado: {ch}"));
        }
    }
    Ok(t)
}

/// Condição do `WHERE`: igualdades e `[NOT] EXISTS (SELECT …)`, ligadas por `AND`.
enum Condicao {
    Igual(String, Valor),
    Diferente(String, Valor),
    Existe(bool, Box<Selecao>),
}

/// Expressão de coluna do `SELECT`: `*`, uma coluna, ou um literal (`SELECT 1`, `INSERT … SELECT 5, 'x'`).
enum Item {
    Todas,
    Coluna(String),
    Literal(Valor),
}

struct Selecao {
    itens: Vec<Item>,
    tabela: Option<String>,
    filtro: Vec<Condicao>,
}

enum Comando {
    Criar { tabela: String, colunas: Vec<String>, se_nao_existe: bool },
    InserirValores { tabela: String, colunas: Option<Vec<String>>, linhas: Vec<Vec<Valor>> },
    InserirSelecao { tabela: String, colunas: Option<Vec<String>>, selecao: Selecao },
    Atualizar { tabela: String, campos: Vec<(String, Valor)>, filtro: Vec<Condicao> },
    Apagar { tabela: String, filtro: Vec<Condicao> },
    Selecionar(Selecao),
}

struct Leitor {
    t: Vec<Token>,
    i: usize,
}

impl Leitor {
    fn olhar(&self) -> Option<&Token> {
        self.t.get(self.i)
    }
    fn e_palavra(&self, p: &str) -> bool {
        matches!(self.olhar(), Some(Token::Palavra(w)) if w.eq_ignore_ascii_case(p))
    }
    fn palavra_opcional(&mut self, p: &str) -> bool {
        let sim = self.e_palavra(p);
        if sim {
            self.i += 1;
        }
        sim
    }
    fn palavra(&mut self, p: &str) -> Result<(), String> {
        if self.palavra_opcional(p) { Ok(()) } else { Err(format!("esperava {p}")) }
    }
    fn simbolo_opcional(&mut self, s: char) -> bool {
        let sim = self.olhar() == Some(&Token::Simbolo(s));
        if sim {
            self.i += 1;
        }
        sim
    }
    fn simbolo(&mut self, s: char) -> Result<(), String> {
        if self.simbolo_opcional(s) { Ok(()) } else { Err(format!("esperava '{s}'")) }
    }
    fn nome(&mut self) -> Result<String, String> {
        match self.t.get(self.i).cloned() {
            Some(Token::Palavra(w)) | Some(Token::Aspas(w)) => {
                self.i += 1;
                Ok(w)
            }
            _ => Err("esperava um nome".into()),
        }
    }
    fn literal(&mut self) -> Result<Valor, String> {
        let negativo = self.simbolo_opcional('-');
        if !negativo {
            self.simbolo_opcional('+');
        }
        let v = match self.t.get(self.i).cloned() {
            Some(Token::Numero(v)) => v,
            Some(Token::Texto(s)) | Some(Token::Aspas(s)) => Valor::Texto(s),
            Some(Token::Palavra(w)) if w.eq_ignore_ascii_case("true") => Valor::Inteiro(1),
            Some(Token::Palavra(w)) if w.eq_ignore_ascii_case("false") => Valor::Inteiro(0),
            Some(Token::Palavra(w)) if w.eq_ignore_ascii_case("null") => Valor::Nulo,
            _ => return Err("esperava um valor".into()),
        };
        self.i += 1;
        Ok(match (negativo, v) {
            (true, Valor::Inteiro(n)) => Valor::Inteiro(-n),
            (true, Valor::Real(r)) => Valor::Real(-r),
            (true, _) => return Err("'-' antes de texto".into()),
            (false, v) => v,
        })
    }
    fn lista_de_nomes(&mut self) -> Result<Vec<String>, String> {
        self.simbolo('(')?;
        let mut v = vec![self.nome()?];
        while self.simbolo_opcional(',') {
            v.push(self.nome()?);
        }
        self.simbolo(')')?;
        Ok(v)
    }
    fn filtro(&mut self) -> Result<Vec<Condicao>, String> {
        let mut v = Vec::new();
        if !self.palavra_opcional("WHERE") {
            return Ok(v);
        }
        loop {
            let nao = self.palavra_opcional("NOT");
            if self.palavra_opcional("EXISTS") {
                self.simbolo('(')?;
                let s = self.selecao()?;
                self.simbolo(')')?;
                v.push(Condicao::Existe(!nao, Box::new(s)));
            } else {
                if nao {
                    return Err("NOT só antes de EXISTS".into());
                }
                let col = self.nome()?;
                if self.simbolo_opcional('=') {
                    self.simbolo_opcional('=');
                    v.push(Condicao::Igual(col, self.literal()?));
                } else if (self.simbolo_opcional('!') || self.simbolo_opcional('<')) && (self.simbolo_opcional('=') || self.simbolo_opcional('>')) {
                    v.push(Condicao::Diferente(col, self.literal()?));
                } else {
                    return Err("só = e != no WHERE".into());
                }
            }
            if !self.palavra_opcional("AND") {
                return Ok(v);
            }
        }
    }
    fn selecao(&mut self) -> Result<Selecao, String> {
        self.palavra("SELECT")?;
        let mut itens = Vec::new();
        loop {
            itens.push(if self.simbolo_opcional('*') {
                Item::Todas
            } else if matches!(self.olhar(), Some(Token::Palavra(w)) if !["true", "false", "null"].iter().any(|l| w.eq_ignore_ascii_case(l))) {
                Item::Coluna(self.nome()?)
            } else {
                Item::Literal(self.literal()?)
            });
            if !self.simbolo_opcional(',') {
                break;
            }
        }
        let tabela = if self.palavra_opcional("FROM") { Some(self.nome()?) } else { None };
        let filtro = self.filtro()?;
        Ok(Selecao { itens, tabela, filtro })
    }
    fn comando(&mut self) -> Result<Comando, String> {
        if self.palavra_opcional("CREATE") {
            self.palavra("TABLE")?;
            let se_nao_existe = self.palavra_opcional("IF");
            if se_nao_existe {
                self.palavra("NOT")?;
                self.palavra("EXISTS")?;
            }
            let tabela = self.nome()?;
            self.simbolo('(')?;
            let mut colunas = Vec::new();
            let mut nivel = 0;
            let mut nova = true;
            while let Some(tok) = self.t.get(self.i).cloned() {
                self.i += 1;
                match tok {
                    Token::Simbolo('(') => nivel += 1,
                    Token::Simbolo(')') if nivel == 0 => break,
                    Token::Simbolo(')') => nivel -= 1,
                    Token::Simbolo(',') if nivel == 0 => nova = true,
                    Token::Palavra(w) | Token::Aspas(w) if nova => {
                        colunas.push(w);
                        nova = false;
                    }
                    _ => {}
                }
            }
            return Ok(Comando::Criar { tabela, colunas, se_nao_existe });
        }
        if self.palavra_opcional("INSERT") {
            self.palavra_opcional("OR");
            self.palavra_opcional("IGNORE");
            self.palavra("INTO")?;
            let tabela = self.nome()?;
            let colunas = if self.olhar() == Some(&Token::Simbolo('(')) { Some(self.lista_de_nomes()?) } else { None };
            if self.e_palavra("SELECT") {
                return Ok(Comando::InserirSelecao { tabela, colunas, selecao: self.selecao()? });
            }
            self.palavra("VALUES")?;
            let mut linhas = Vec::new();
            loop {
                self.simbolo('(')?;
                let mut l = vec![self.literal()?];
                while self.simbolo_opcional(',') {
                    l.push(self.literal()?);
                }
                self.simbolo(')')?;
                linhas.push(l);
                if !self.simbolo_opcional(',') {
                    break;
                }
            }
            return Ok(Comando::InserirValores { tabela, colunas, linhas });
        }
        if self.palavra_opcional("UPDATE") {
            let tabela = self.nome()?;
            self.palavra("SET")?;
            let mut campos = Vec::new();
            loop {
                let c = self.nome()?;
                self.simbolo('=')?;
                campos.push((c, self.literal()?));
                if !self.simbolo_opcional(',') {
                    break;
                }
            }
            let filtro = self.filtro()?;
            return Ok(Comando::Atualizar { tabela, campos, filtro });
        }
        if self.palavra_opcional("DELETE") {
            self.palavra("FROM")?;
            let tabela = self.nome()?;
            let filtro = self.filtro()?;
            return Ok(Comando::Apagar { tabela, filtro });
        }
        if self.e_palavra("SELECT") {
            return Ok(Comando::Selecionar(self.selecao()?));
        }
        Err("comando não suportado".into())
    }
}

/// Comandos de um texto SQL, separados por `;`.
fn comandos(sql: &str) -> Result<Vec<Comando>, String> {
    let mut l = Leitor { t: tokens(sql)?, i: 0 };
    let mut v = Vec::new();
    while l.i < l.t.len() {
        if l.simbolo_opcional(';') {
            continue;
        }
        v.push(l.comando()?);
        if l.i < l.t.len() && !l.simbolo_opcional(';') {
            return Err("esperava ';' entre comandos".into());
        }
    }
    Ok(v)
}

pub(crate) struct Banco {
    tabelas: Vec<Tabela>,
    arquivo: Option<PathBuf>,
}

/// Resultado de um `SELECT`: nomes das colunas e linhas.
pub(crate) struct Linhas {
    pub(crate) colunas: Vec<String>,
    pub(crate) linhas: Vec<Vec<Valor>>,
}

impl Banco {
    /// Banco do mod em `pasta/db.bwms` (vazio se o arquivo não existe ou não é legível).
    pub(crate) fn do_mod(pasta: &Path) -> Banco {
        let arquivo = pasta.join("db.bwms");
        let mut b = Banco { tabelas: Vec::new(), arquivo: None };
        if let Ok(txt) = std::fs::read_to_string(&arquivo) {
            b.ler(&txt);
        }
        b.arquivo = Some(arquivo);
        b
    }

    #[cfg(test)]
    pub(crate) fn em_memoria() -> Banco {
        Banco { tabelas: Vec::new(), arquivo: None }
    }

    fn tabela(&self, nome: &str) -> Result<&Tabela, String> {
        self.tabelas.iter().find(|t| t.nome.eq_ignore_ascii_case(nome)).ok_or_else(|| format!("tabela inexistente: {nome}"))
    }

    fn tabela_mut(&mut self, nome: &str) -> Result<&mut Tabela, String> {
        self.tabelas.iter_mut().find(|t| t.nome.eq_ignore_ascii_case(nome)).ok_or_else(|| format!("tabela inexistente: {nome}"))
    }

    fn coluna(t: &Tabela, nome: &str) -> Result<usize, String> {
        t.colunas.iter().position(|c| c.eq_ignore_ascii_case(nome)).ok_or_else(|| format!("coluna inexistente: {}.{nome}", t.nome))
    }

    fn casa(&self, t: &Tabela, linha: &[Valor], filtro: &[Condicao]) -> Result<bool, String> {
        for c in filtro {
            let ok = match c {
                Condicao::Igual(col, v) => linha[Self::coluna(t, col)?].igual(v),
                Condicao::Diferente(col, v) => !linha[Self::coluna(t, col)?].igual(v) && linha[Self::coluna(t, col)?] != Valor::Nulo,
                Condicao::Existe(sim, s) => !self.selecionar(s)?.linhas.is_empty() == *sim,
            };
            if !ok {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn selecionar(&self, s: &Selecao) -> Result<Linhas, String> {
        let Some(nome) = &s.tabela else {
            // `SELECT 5, 'x' WHERE …` sem FROM: uma linha de literais se o filtro passa.
            let vazia = Tabela { nome: String::new(), colunas: Vec::new(), linhas: Vec::new() };
            if !self.casa(&vazia, &[], &s.filtro)? {
                return Ok(Linhas { colunas: Vec::new(), linhas: Vec::new() });
            }
            let linha = s.itens.iter().map(|i| match i {
                Item::Literal(v) => Ok(v.clone()),
                _ => Err("coluna sem FROM".to_string()),
            });
            return Ok(Linhas { colunas: vec![String::new(); s.itens.len()], linhas: vec![linha.collect::<Result<_, _>>()?] });
        };
        let t = self.tabela(nome)?;
        let mut colunas = Vec::new();
        let mut fontes: Vec<Result<usize, Valor>> = Vec::new();
        for i in &s.itens {
            match i {
                Item::Todas => {
                    colunas.extend(t.colunas.iter().cloned());
                    fontes.extend((0..t.colunas.len()).map(Ok));
                }
                Item::Coluna(c) => {
                    let k = Self::coluna(t, c)?;
                    colunas.push(t.colunas[k].clone());
                    fontes.push(Ok(k));
                }
                Item::Literal(v) => {
                    colunas.push(String::new());
                    fontes.push(Err(v.clone()));
                }
            }
        }
        let mut linhas = Vec::new();
        for l in &t.linhas {
            if self.casa(t, l, &s.filtro)? {
                linhas.push(fontes.iter().map(|f| match f {
                    Ok(k) => l[*k].clone(),
                    Err(v) => v.clone(),
                }).collect());
            }
        }
        Ok(Linhas { colunas, linhas })
    }

    fn inserir(&mut self, tabela: &str, colunas: &Option<Vec<String>>, valores: Vec<Valor>) -> Result<(), String> {
        let t = self.tabela_mut(tabela)?;
        let linha = match colunas {
            None if valores.len() == t.colunas.len() => valores,
            None => return Err(format!("{} tem {} colunas e vieram {} valores", t.nome, t.colunas.len(), valores.len())),
            Some(cols) => {
                if cols.len() != valores.len() {
                    return Err("colunas e valores em número diferente".into());
                }
                let mut l = vec![Valor::Nulo; t.colunas.len()];
                for (c, v) in cols.iter().zip(valores) {
                    l[Self::coluna(t, c)?] = v;
                }
                l
            }
        };
        t.linhas.push(linha);
        Ok(())
    }

    /// Roda um comando; devolve as linhas se for `SELECT` e se mudou o banco.
    fn rodar(&mut self, c: Comando) -> Result<(Option<Linhas>, bool), String> {
        match c {
            Comando::Criar { tabela, colunas, se_nao_existe } => {
                if self.tabela(&tabela).is_ok() {
                    return if se_nao_existe { Ok((None, false)) } else { Err(format!("tabela {tabela} já existe")) };
                }
                self.tabelas.push(Tabela { nome: tabela, colunas, linhas: Vec::new() });
                Ok((None, true))
            }
            Comando::InserirValores { tabela, colunas, linhas } => {
                for l in linhas {
                    self.inserir(&tabela, &colunas, l)?;
                }
                Ok((None, true))
            }
            Comando::InserirSelecao { tabela, colunas, selecao } => {
                let novas = self.selecionar(&selecao)?.linhas;
                let mudou = !novas.is_empty();
                for l in novas {
                    self.inserir(&tabela, &colunas, l)?;
                }
                Ok((None, mudou))
            }
            Comando::Atualizar { tabela, campos, filtro } => {
                let t = self.tabela(&tabela)?;
                let alvo: Vec<(usize, Valor)> = campos.into_iter().map(|(c, v)| Self::coluna(t, &c).map(|k| (k, v))).collect::<Result<_, _>>()?;
                let marcadas: Vec<bool> = t.linhas.iter().map(|l| self.casa(t, l, &filtro)).collect::<Result<_, _>>()?;
                let t = self.tabela_mut(&tabela)?;
                for (l, sim) in t.linhas.iter_mut().zip(&marcadas) {
                    if *sim {
                        for (k, v) in &alvo {
                            l[*k] = v.clone();
                        }
                    }
                }
                Ok((None, marcadas.contains(&true)))
            }
            Comando::Apagar { tabela, filtro } => {
                let t = self.tabela(&tabela)?;
                let marcadas: Vec<bool> = t.linhas.iter().map(|l| self.casa(t, l, &filtro)).collect::<Result<_, _>>()?;
                let t = self.tabela_mut(&tabela)?;
                let mut i = 0;
                t.linhas.retain(|_| {
                    i += 1;
                    !marcadas[i - 1]
                });
                Ok((None, marcadas.contains(&true)))
            }
            Comando::Selecionar(s) => Ok((Some(self.selecionar(&s)?), false)),
        }
    }

    /// `db:exec(sql)`: roda os comandos em ordem e para no primeiro erro, como o SQLite; grava o arquivo se algo mudou.
    pub(crate) fn exec(&mut self, sql: &str) -> Result<(), String> {
        let mut mudou = false;
        let r = (|| {
            for c in comandos(sql)? {
                mudou |= self.rodar(c)?.1;
            }
            Ok(())
        })();
        if mudou {
            self.gravar();
        }
        r
    }

    /// `db:rows(sql)`: as linhas do último `SELECT` do texto.
    pub(crate) fn consultar(&mut self, sql: &str) -> Result<Linhas, String> {
        let mut ultimo = Linhas { colunas: Vec::new(), linhas: Vec::new() };
        let mut mudou = false;
        let r = (|| {
            for c in comandos(sql)? {
                let (l, m) = self.rodar(c)?;
                mudou |= m;
                if let Some(l) = l {
                    ultimo = l;
                }
            }
            Ok(())
        })();
        if mudou {
            self.gravar();
        }
        r.map(|()| ultimo)
    }

    /// Arquivo do banco: `tabela <nome>` e as colunas separadas por tab, depois uma linha por registro com cada valor marcado pelo tipo
    /// (`i:`, `r:`, `t:`, `n`); tab, quebra de linha e `\` do texto escapados.
    fn texto(&self) -> String {
        let mut s = String::new();
        for t in &self.tabelas {
            s.push_str("tabela\t");
            s.push_str(&escapar(&t.nome));
            for c in &t.colunas {
                s.push('\t');
                s.push_str(&escapar(c));
            }
            s.push('\n');
            for l in &t.linhas {
                s.push_str("linha");
                for v in l {
                    s.push('\t');
                    match v {
                        Valor::Nulo => s.push('n'),
                        Valor::Inteiro(n) => s.push_str(&format!("i:{n}")),
                        Valor::Real(r) => s.push_str(&format!("r:{r:?}")),
                        Valor::Texto(x) => {
                            s.push_str("t:");
                            s.push_str(&escapar(x));
                        }
                    }
                }
                s.push('\n');
            }
        }
        s
    }

    fn ler(&mut self, txt: &str) {
        for linha in txt.lines() {
            let mut partes = linha.split('\t');
            match partes.next() {
                Some("tabela") => {
                    let nome = partes.next().map(desescapar).unwrap_or_default();
                    self.tabelas.push(Tabela { nome, colunas: partes.map(desescapar).collect(), linhas: Vec::new() });
                }
                Some("linha") => {
                    let Some(t) = self.tabelas.last_mut() else { continue };
                    let l: Vec<Valor> = partes.map(|p| match p.split_at(p.len().min(2)) {
                        ("i:", n) => n.parse().map_or(Valor::Nulo, Valor::Inteiro),
                        ("r:", r) => r.parse().map_or(Valor::Nulo, Valor::Real),
                        ("t:", x) => Valor::Texto(desescapar(x)),
                        _ => Valor::Nulo,
                    }).collect();
                    if l.len() == t.colunas.len() {
                        t.linhas.push(l);
                    }
                }
                _ => {}
            }
        }
    }

    /// Grava num temporário e troca, para uma falha no meio não deixar o banco do mod pela metade.
    fn gravar(&self) {
        let Some(arq) = &self.arquivo else { return };
        let tmp = arq.with_extension("bwms.tmp");
        if std::fs::write(&tmp, self.texto()).is_ok() && std::fs::rename(&tmp, arq).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

fn escapar(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\t', "\\t").replace('\n', "\\n")
}

fn desescapar(s: &str) -> String {
    let mut o = String::new();
    let mut c = s.chars();
    while let Some(ch) = c.next() {
        if ch == '\\' {
            match c.next() {
                Some('t') => o.push('\t'),
                Some('n') => o.push('\n'),
                Some(x) => o.push(x),
                None => {}
            }
        } else {
            o.push(ch);
        }
    }
    o
}

#[cfg(test)]
mod testes {
    use super::*;

    /// O ciclo de um mod de câmera: criação com dados, `CREATE` repetido que aborta o resto do `exec`, `INSERT … WHERE NOT EXISTS`, `UPDATE`
    /// e `SELECT` por nome, e o arquivo lido de volta igual.
    #[test]
    fn ciclo_do_mod_de_camera() {
        let mut b = Banco::em_memoria();
        let criar = "CREATE TABLE settings(id, name, value);\n INSERT INTO settings VALUES(0, \"isTppEnabled\", false);\n INSERT INTO settings VALUES(1, \"weaponOverride\", true);";
        b.exec(criar).unwrap();
        assert!(b.exec(criar).is_err());
        assert_eq!(b.tabela("settings").unwrap().linhas.len(), 2);
        let novo = "INSERT INTO settings SELECT 13, 'fov', 80 WHERE NOT EXISTS(SELECT 1 FROM settings WHERE id = 13);";
        b.exec(novo).unwrap();
        b.exec(novo).unwrap();
        assert_eq!(b.tabela("settings").unwrap().linhas.len(), 3);
        b.exec("UPDATE settings SET value = 0.75 WHERE name = 'fov'").unwrap();
        let l = b.consultar("SELECT value FROM settings WHERE name = 'fov'").unwrap();
        assert_eq!(l.linhas, vec![vec![Valor::Real(0.75)]]);
        let l = b.consultar("SELECT value FROM settings WHERE name = 'weaponOverride'").unwrap();
        assert_eq!(l.linhas, vec![vec![Valor::Inteiro(1)]]);
        b.exec("CREATE TABLE cameras(id, x, y, z, w, rx, ry, rz, rw, camSwitch, freeForm); INSERT INTO cameras VALUES(0, 0, -2, 0, 0, 0, 0, 0, 1, false, false);").unwrap();
        b.exec("UPDATE cameras SET x = 0.5, y = -2.25, z=1  WHERE id = 0").unwrap();
        let l = b.consultar("SELECT * FROM cameras").unwrap();
        assert_eq!(l.colunas.len(), 11);
        assert_eq!(&l.linhas[0][..4], &[Valor::Inteiro(0), Valor::Real(0.5), Valor::Real(-2.25), Valor::Inteiro(1)]);

        let mut volta = Banco::em_memoria();
        volta.ler(&b.texto());
        assert_eq!(volta.texto(), b.texto());
    }

    #[test]
    fn texto_com_tab_e_aspas_volta_igual() {
        let mut b = Banco::em_memoria();
        b.exec("CREATE TABLE t(a); INSERT INTO t VALUES('it''s\tok\\n');").unwrap();
        let mut volta = Banco::em_memoria();
        volta.ler(&b.texto());
        assert_eq!(volta.consultar("SELECT a FROM t").unwrap().linhas, vec![vec![Valor::Texto("it's\tok\\n".into())]]);
    }

    #[test]
    fn erro_de_sql_nao_muda_o_banco() {
        let mut b = Banco::em_memoria();
        assert!(b.exec("DROP TABLE x").is_err());
        assert!(b.exec("SELECT * FROM nada").is_err());
        assert!(b.tabelas.is_empty());
    }
}
