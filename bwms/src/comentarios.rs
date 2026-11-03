//! `bwms comentarios <entrada> <saida>`: o zip público leva os `.reds` e os roteiros (`.sh`, `.command`) sem comentário; o fonte de dev
//! continua comentado. O corte lê como código: `//` dentro de texto entre aspas e `#` dentro de aspas ou de here-doc são conteúdo. Entrada e
//! saída são arquivo ou pasta, e podem ser a mesma (o `dist/pack-bwms.sh` corta no lugar, no staging). O que o leitor não consegue fechar
//! (aspas, comentário de bloco, here-doc) recusa o arquivo, e nesse caso nada é gravado: tudo é calculado antes da primeira gravação.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USO: &str = "uso: bwms comentarios <entrada> <saida>
  entrada e saida: arquivo ou pasta; a mesma = corte no lugar
  .reds: tira // e /* */ (o bloco aninha, como no scc); .sh e .command: tira só a linha inteira de comentário
  numa pasta, o resto dos arquivos é copiado igual
";

pub fn run(args: &[String]) -> ExitCode {
    let (Some(entrada), Some(saida), None) = (args.first(), args.get(1), args.get(2)) else {
        eprint!("{USO}");
        return ExitCode::FAILURE;
    };
    match cortar(Path::new(entrada), Path::new(saida)) {
        Ok(medida) => {
            medida.imprimir();
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("bwms comentarios: {e}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tipo {
    Reds,
    Roteiro,
}

fn tipo_do_arquivo(caminho: &Path) -> Option<Tipo> {
    match caminho.extension().and_then(|e| e.to_str()) {
        Some("reds") => Some(Tipo::Reds),
        Some("sh" | "command") => Some(Tipo::Roteiro),
        _ => None,
    }
}

#[derive(Default)]
struct Contagem {
    arquivos: usize,
    antes: usize,
    depois: usize,
}

#[derive(Default)]
struct Medida {
    reds: Contagem,
    roteiros: Contagem,
}

impl Medida {
    fn imprimir(&self) {
        for (rotulo, c) in [(".reds", &self.reds), ("roteiros", &self.roteiros)] {
            if c.arquivos == 0 {
                continue;
            }
            let queda = 100.0 * c.antes.saturating_sub(c.depois) as f64 / c.antes.max(1) as f64;
            println!("{rotulo:<9} {} arquivo(s): {} -> {} bytes (-{queda:.1}%)", c.arquivos, c.antes, c.depois);
        }
    }
}

struct Item {
    origem: PathBuf,
    destino: PathBuf,
    tipo: Option<Tipo>,
}

fn cortar(entrada: &Path, saida: &Path) -> Result<Medida, String> {
    let meta = fs::symlink_metadata(entrada).map_err(|e| format!("não li '{}': {e}", entrada.display()))?;
    let no_lugar = matches!((fs::canonicalize(entrada), fs::canonicalize(saida)), (Ok(a), Ok(b)) if a == b);
    let mut itens = Vec::new();
    if meta.is_dir() {
        listar(entrada, saida, &mut itens)?;
    } else if meta.is_file() {
        let tipo = tipo_do_arquivo(entrada).ok_or_else(|| format!("'{}' não é .reds, .sh nem .command", entrada.display()))?;
        itens.push(Item { origem: entrada.to_path_buf(), destino: saida.to_path_buf(), tipo: Some(tipo) });
    } else {
        return Err(format!("'{}' não é arquivo nem pasta", entrada.display()));
    }

    let mut medida = Medida::default();
    let mut gravacoes = Vec::with_capacity(itens.len());
    for item in itens {
        let bytes = fs::read(&item.origem).map_err(|e| format!("não li '{}': {e}; nada foi gravado", item.origem.display()))?;
        let permissoes = fs::metadata(&item.origem)
            .map_err(|e| format!("não li '{}': {e}; nada foi gravado", item.origem.display()))?
            .permissions();
        let Some(tipo) = item.tipo else {
            if !no_lugar {
                gravacoes.push((item.destino, bytes, permissoes));
            }
            continue;
        };
        let texto = String::from_utf8(bytes).map_err(|_| format!("'{}' não é UTF-8; nada foi gravado", item.origem.display()))?;
        let cortado = match tipo {
            Tipo::Reds => tirar_de_reds(&texto),
            Tipo::Roteiro => tirar_de_roteiro(&texto),
        }
        .map_err(|e| format!("'{}': {e}; nada foi gravado", item.origem.display()))?;
        let c = if tipo == Tipo::Reds { &mut medida.reds } else { &mut medida.roteiros };
        c.arquivos += 1;
        c.antes += texto.len();
        c.depois += cortado.len();
        gravacoes.push((item.destino, cortado.into_bytes(), permissoes));
    }

    for (destino, bytes, permissoes) in gravacoes {
        if let Some(pai) = destino.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(pai).map_err(|e| format!("não criei '{}': {e}", pai.display()))?;
        }
        fs::write(&destino, &bytes).map_err(|e| format!("não gravei '{}': {e}", destino.display()))?;
        // O roteiro do zip precisa continuar executável (o INSTALAR.command abre com duplo clique).
        fs::set_permissions(&destino, permissoes).map_err(|e| format!("não ajustei a permissão de '{}': {e}", destino.display()))?;
    }
    Ok(medida)
}

/// Ordem por nome: a mesma entrada dá a mesma ordem de gravação e de erro em qualquer disco.
fn listar(pasta: &Path, destino: &Path, itens: &mut Vec<Item>) -> Result<(), String> {
    let erro = |e: std::io::Error| format!("não li a pasta '{}': {e}; nada foi gravado", pasta.display());
    let mut caminhos = fs::read_dir(pasta).map_err(erro)?.map(|e| e.map(|e| e.path())).collect::<Result<Vec<_>, _>>().map_err(erro)?;
    caminhos.sort();
    for caminho in caminhos {
        let Some(nome) = caminho.file_name() else {
            continue;
        };
        let alvo = destino.join(nome);
        let meta = fs::symlink_metadata(&caminho).map_err(|e| format!("não li '{}': {e}; nada foi gravado", caminho.display()))?;
        if meta.is_dir() {
            listar(&caminho, &alvo, itens)?;
        } else if meta.is_file() {
            itens.push(Item { tipo: tipo_do_arquivo(&caminho), origem: caminho, destino: alvo });
        } else {
            return Err(format!("'{}' não é arquivo nem pasta (link simbólico?); nada foi gravado", caminho.display()));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// .reds — as regras léxicas do scc que o pacote leva (redscript 0.5, `compiler/src/parser.rs`): `//` vai até o `\n`; `/* */` ANINHA
// (`comment_content = comment() / …`); texto é `"…"` com escape por `\`, pode ter quebra de linha, e o prefixo `n`/`r`/`t` é só uma letra
// colada; `s"…"` interpola `\( expr )`, e a expressão é código de novo (texto e comentário dentro dela). Não há literal de caractere.
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct LinhaReds {
    texto: Vec<u8>,
    teve_comentario: bool,
    /// A linha começa ou termina dentro de texto entre aspas: o branco dela é conteúdo, a montagem não mexe.
    comeca_em_texto: bool,
    termina_em_texto: bool,
}

#[derive(Clone, Copy)]
enum QuadroReds {
    /// O arquivo, ou o `\( … )` de um `s"…"`, que fecha no primeiro `)` sem par.
    Codigo { parenteses: u32 },
    Texto { interpolado: bool },
}

fn identificador(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn aparar_fim(texto: &mut Vec<u8>) {
    while texto.last().is_some_and(|c| matches!(c, b' ' | b'\t' | b'\r')) {
        texto.pop();
    }
}

fn quebrar_linha(prontas: &mut Vec<LinhaReds>, atual: &mut LinhaReds, em_texto: bool) {
    atual.termina_em_texto = em_texto;
    prontas.push(std::mem::take(atual));
    atual.comeca_em_texto = em_texto;
}

pub fn tirar_de_reds(fonte: &str) -> Result<String, String> {
    let b = fonte.as_bytes();
    let mut prontas = Vec::new();
    let mut atual = LinhaReds::default();
    let mut pilha = vec![QuadroReds::Codigo { parenteses: 0 }];
    let mut linha = 1usize;
    let mut texto_aberto_na = 0usize;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        let proximo = b.get(i + 1).copied();
        let em_interpolacao = pilha.len() > 1;
        match pilha.last().copied() {
            Some(QuadroReds::Texto { interpolado }) => {
                match c {
                    b'\\' if interpolado && proximo == Some(b'(') => {
                        atual.texto.extend_from_slice(b"\\(");
                        pilha.push(QuadroReds::Codigo { parenteses: 0 });
                        i += 2;
                        continue;
                    }
                    b'\\' => {
                        atual.texto.push(c);
                        if let Some(p) = proximo.filter(|p| *p != b'\n') {
                            atual.texto.push(p);
                            i += 1;
                        }
                    }
                    b'"' => {
                        pilha.pop();
                        atual.texto.push(c);
                    }
                    b'\n' => {
                        quebrar_linha(&mut prontas, &mut atual, true);
                        linha += 1;
                    }
                    _ => atual.texto.push(c),
                }
                i += 1;
            }
            Some(QuadroReds::Codigo { .. }) => {
                if c == b'/' && proximo == Some(b'/') {
                    while i < b.len() && b[i] != b'\n' {
                        i += 1;
                    }
                    atual.teve_comentario = true;
                    aparar_fim(&mut atual.texto);
                    continue;
                }
                if c == b'/' && proximo == Some(b'*') {
                    let aberto_na = linha;
                    let mut nivel = 1u32;
                    let mut quebras = 0usize;
                    i += 2;
                    while nivel > 0 {
                        match (b.get(i), b.get(i + 1)) {
                            (None, _) => return Err(format!("comentário /* aberto na linha {aberto_na} não fecha")),
                            (Some(b'/'), Some(b'*')) => {
                                nivel += 1;
                                i += 2;
                            }
                            (Some(b'*'), Some(b'/')) => {
                                nivel -= 1;
                                i += 2;
                            }
                            (Some(b'\n'), _) => {
                                quebras += 1;
                                linha += 1;
                                i += 1;
                            }
                            _ => i += 1,
                        }
                    }
                    atual.teve_comentario = true;
                    if quebras > 0 {
                        // Uma quebra no lugar do bloco inteiro: separa o que estava antes e depois dele, como o branco que ele era.
                        aparar_fim(&mut atual.texto);
                        quebrar_linha(&mut prontas, &mut atual, em_interpolacao);
                        atual.teve_comentario = true;
                    }
                    if atual.texto.last().map_or(true, u8::is_ascii_whitespace) {
                        while matches!(b.get(i), Some(b' ' | b'\t')) {
                            i += 1;
                        }
                    } else if b.get(i).is_some_and(|x| !x.is_ascii_whitespace()) {
                        // `a/*x*/b` são dois tokens; sem o espaço virariam um.
                        atual.texto.push(b' ');
                    }
                    continue;
                }
                match c {
                    b'"' => {
                        let interpolado = i >= 1 && b[i - 1] == b's' && (i < 2 || !identificador(b[i - 2]));
                        if !em_interpolacao {
                            texto_aberto_na = linha;
                        }
                        pilha.push(QuadroReds::Texto { interpolado });
                    }
                    b'(' if em_interpolacao => {
                        if let Some(QuadroReds::Codigo { parenteses }) = pilha.last_mut() {
                            *parenteses += 1;
                        }
                    }
                    b')' if em_interpolacao => match pilha.last_mut() {
                        Some(QuadroReds::Codigo { parenteses }) if *parenteses > 0 => *parenteses -= 1,
                        _ => {
                            pilha.pop();
                        }
                    },
                    _ => {}
                }
                if c == b'\n' {
                    quebrar_linha(&mut prontas, &mut atual, em_interpolacao);
                    linha += 1;
                } else {
                    atual.texto.push(c);
                }
                i += 1;
            }
            None => return Err("leitor de .reds sem o quadro do arquivo".to_string()),
        }
    }
    if pilha.len() > 1 {
        return Err(format!("texto entre aspas aberto na linha {texto_aberto_na} não fecha"));
    }
    prontas.push(atual);
    montar_reds(prontas, fonte.ends_with('\n'))
}

/// Linha que só tinha comentário sai; branco original vira um só entre duas linhas de código, nunca no começo nem no fim do arquivo.
fn montar_reds(linhas: Vec<LinhaReds>, quebra_no_fim: bool) -> Result<String, String> {
    let mut saida = Vec::new();
    let mut branco_pendente = false;
    for mut l in linhas {
        if !l.comeca_em_texto && !l.termina_em_texto {
            if l.teve_comentario {
                aparar_fim(&mut l.texto);
            }
            if l.texto.iter().all(u8::is_ascii_whitespace) {
                branco_pendente |= !l.teve_comentario;
                continue;
            }
        }
        if branco_pendente && !saida.is_empty() {
            saida.push(b'\n');
        }
        branco_pendente = false;
        saida.extend_from_slice(&l.texto);
        saida.push(b'\n');
    }
    if !quebra_no_fim && saida.last() == Some(&b'\n') {
        saida.pop();
    }
    String::from_utf8(saida).map_err(|_| "a saída do .reds deixou de ser UTF-8".to_string())
}

// ---------------------------------------------------------------------------------------------------------------------------------------
// Roteiros — sai só a linha inteira de comentário no nível do roteiro: fora de aspas, de `${…}`, de `$(…)`, de crase, de aritmética e de
// corpo de here-doc, e nunca depois de uma linha terminada em `\` (ali o `#` é a continuação do comando). O comentário no fim de uma linha
// de código fica. O shebang fica. Os quadros acompanham o bash o bastante para saber onde cada linha começa; o que não fecha recusa o
// arquivo.
// ---------------------------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuadroSh {
    /// O roteiro (`substituicao: false`) ou um `$( … )`. Os contadores separam o `)` que fecha o `$(` do `)` de um subshell interno ou de
    /// um padrão de `case`.
    Codigo { parenteses: u32, cases: u32, substituicao: bool },
    Crase,
    AspasDuplas,
    AspasSimples,
    AnsiC,
    /// `${ … }`; dentro de aspas duplas a aspa simples é literal.
    Parametro { chaves: u32, em_aspas: bool },
    Aritmetica { parenteses: u32 },
}

impl QuadroSh {
    fn nome(self) -> &'static str {
        match self {
            QuadroSh::Codigo { .. } => "$(",
            QuadroSh::Crase => "`",
            QuadroSh::AspasDuplas => "\"",
            QuadroSh::AspasSimples => "'",
            QuadroSh::AnsiC => "$'",
            QuadroSh::Parametro { .. } => "${",
            QuadroSh::Aritmetica { .. } => "$((",
        }
    }
}

struct HereDoc {
    delimitador: String,
    tira_tabs: bool,
}

struct LeitorSh {
    /// O quadro e a linha onde abriu, para a mensagem de recusa.
    pilha: Vec<(QuadroSh, usize)>,
    /// A linha anterior terminou em `\` no código: esta continua o mesmo comando.
    continua: bool,
    palavra_antes_da_continuacao: bool,
}

impl LeitorSh {
    fn no_topo(&self) -> bool {
        self.pilha.len() == 1 && !self.continua
    }

    fn abrir(&mut self, q: QuadroSh, n: usize) {
        self.pilha.push((q, n));
    }

    /// Bytes consumidos e se abriu código novo (`$(`).
    fn abrir_cifrao(&mut self, b: &[u8], i: usize, em_aspas: bool, n: usize) -> (usize, bool) {
        match b.get(i + 1) {
            Some(b'(') if b.get(i + 2) == Some(&b'(') => {
                self.abrir(QuadroSh::Aritmetica { parenteses: 0 }, n);
                (3, false)
            }
            Some(b'(') => {
                self.abrir(QuadroSh::Codigo { parenteses: 0, cases: 0, substituicao: true }, n);
                (2, true)
            }
            Some(b'{') => {
                self.abrir(QuadroSh::Parametro { chaves: 0, em_aspas }, n);
                (2, false)
            }
            Some(b'\'') if !em_aspas => {
                self.abrir(QuadroSh::AnsiC, n);
                (2, false)
            }
            Some(b'"') if !em_aspas => {
                self.abrir(QuadroSh::AspasDuplas, n);
                (2, false)
            }
            _ => (1, false),
        }
    }

    /// `)` no código. Devolve se fechou um `$(`.
    fn fechar_parentese(&mut self) -> bool {
        if let Some((QuadroSh::Codigo { parenteses, cases, substituicao }, _)) = self.pilha.last_mut() {
            if *parenteses > 0 {
                *parenteses -= 1;
            } else if *cases == 0 && *substituicao {
                self.pilha.pop();
                return true;
            }
        }
        false
    }

    fn palavra(&mut self, p: &[u8]) {
        if let Some((QuadroSh::Codigo { cases, .. }, _)) = self.pilha.last_mut() {
            match p {
                b"case" => *cases += 1,
                b"esac" => *cases = cases.saturating_sub(1),
                _ => {}
            }
        }
    }

    fn ler_linha(&mut self, linha: &str, n: usize, docs: &mut VecDeque<HereDoc>) -> Result<(), String> {
        let b = linha.as_bytes();
        let mut inicio = if self.continua { self.palavra_antes_da_continuacao } else { true };
        self.continua = false;
        let mut i = 0;
        while i < b.len() {
            let c = b[i];
            let prox = b.get(i + 1).copied();
            let Some(&(topo, _)) = self.pilha.last() else {
                return Err("leitor de roteiro sem o quadro do arquivo".to_string());
            };
            match topo {
                QuadroSh::Codigo { .. } | QuadroSh::Crase => {
                    if c == b'#' && inicio {
                        return Ok(());
                    }
                    match c {
                        b'\\' => {
                            if prox.is_none() {
                                self.continua = true;
                                self.palavra_antes_da_continuacao = inicio;
                                return Ok(());
                            }
                            i += 2;
                            inicio = false;
                            continue;
                        }
                        b' ' | b'\t' | b';' | b'&' | b'|' | b'>' => {
                            inicio = true;
                        }
                        b'<' if prox == Some(b'<') => {
                            if b.get(i + 2) == Some(&b'<') {
                                i += 3;
                                inicio = true;
                            } else {
                                i = ler_heredoc(b, i + 2, n, docs)?;
                                inicio = false;
                            }
                            continue;
                        }
                        b'<' => inicio = true,
                        b'\'' => {
                            self.abrir(QuadroSh::AspasSimples, n);
                            inicio = false;
                        }
                        b'"' => {
                            self.abrir(QuadroSh::AspasDuplas, n);
                            inicio = false;
                        }
                        b'`' => {
                            if topo == QuadroSh::Crase {
                                self.pilha.pop();
                            } else {
                                self.abrir(QuadroSh::Crase, n);
                            }
                            inicio = false;
                        }
                        b'$' => {
                            let (usados, abriu_codigo) = self.abrir_cifrao(b, i, false, n);
                            i += usados;
                            inicio = abriu_codigo;
                            continue;
                        }
                        b'(' if prox == Some(b'(') && inicio => {
                            self.abrir(QuadroSh::Aritmetica { parenteses: 0 }, n);
                            i += 2;
                            continue;
                        }
                        b'(' => {
                            if let Some((QuadroSh::Codigo { parenteses, .. }, _)) = self.pilha.last_mut() {
                                *parenteses += 1;
                            }
                            inicio = true;
                        }
                        b')' => inicio = !self.fechar_parentese(),
                        _ if inicio && c.is_ascii_alphabetic() => {
                            let fim = b[i..].iter().position(|x| !identificador(*x)).map_or(b.len(), |p| i + p);
                            self.palavra(&b[i..fim]);
                            i = fim;
                            inicio = false;
                            continue;
                        }
                        _ => inicio = false,
                    }
                    i += 1;
                }
                QuadroSh::AspasDuplas => {
                    match c {
                        b'\\' => {
                            i += 2;
                            continue;
                        }
                        b'"' => {
                            self.pilha.pop();
                        }
                        b'`' => self.abrir(QuadroSh::Crase, n),
                        b'$' => {
                            let (usados, _) = self.abrir_cifrao(b, i, true, n);
                            i += usados;
                            continue;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                QuadroSh::AspasSimples => {
                    if c == b'\'' {
                        self.pilha.pop();
                    }
                    i += 1;
                }
                QuadroSh::AnsiC => {
                    match c {
                        b'\\' => {
                            i += 2;
                            continue;
                        }
                        b'\'' => {
                            self.pilha.pop();
                        }
                        _ => {}
                    }
                    i += 1;
                }
                QuadroSh::Parametro { em_aspas, .. } => {
                    match c {
                        b'\\' => {
                            i += 2;
                            continue;
                        }
                        b'"' => self.abrir(QuadroSh::AspasDuplas, n),
                        b'\'' if !em_aspas => self.abrir(QuadroSh::AspasSimples, n),
                        b'`' => self.abrir(QuadroSh::Crase, n),
                        b'$' => {
                            let (usados, _) = self.abrir_cifrao(b, i, em_aspas, n);
                            i += usados;
                            continue;
                        }
                        b'{' => {
                            if let Some((QuadroSh::Parametro { chaves, .. }, _)) = self.pilha.last_mut() {
                                *chaves += 1;
                            }
                        }
                        b'}' => match self.pilha.last_mut() {
                            Some((QuadroSh::Parametro { chaves, .. }, _)) if *chaves > 0 => *chaves -= 1,
                            _ => {
                                self.pilha.pop();
                            }
                        },
                        _ => {}
                    }
                    i += 1;
                }
                QuadroSh::Aritmetica { .. } => {
                    match c {
                        b'(' => {
                            if let Some((QuadroSh::Aritmetica { parenteses }, _)) = self.pilha.last_mut() {
                                *parenteses += 1;
                            }
                        }
                        b')' => match self.pilha.last_mut() {
                            Some((QuadroSh::Aritmetica { parenteses }, _)) if *parenteses > 0 => *parenteses -= 1,
                            _ => {
                                self.pilha.pop();
                                if prox == Some(b')') {
                                    i += 1;
                                }
                            }
                        },
                        b'"' => self.abrir(QuadroSh::AspasDuplas, n),
                        b'$' => {
                            let (usados, _) = self.abrir_cifrao(b, i, false, n);
                            i += usados;
                            continue;
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
        }
        Ok(())
    }
}

/// Lê o delimitador depois do `<<` (com `-`, aspas e `\` como o bash: tudo isso some do nome) e enfileira o here-doc. Devolve onde a
/// leitura da linha continua.
fn ler_heredoc(b: &[u8], mut i: usize, n: usize, docs: &mut VecDeque<HereDoc>) -> Result<usize, String> {
    let tira_tabs = b.get(i) == Some(&b'-');
    if tira_tabs {
        i += 1;
    }
    while matches!(b.get(i), Some(b' ' | b'\t')) {
        i += 1;
    }
    let mut delimitador = Vec::new();
    while let Some(&c) = b.get(i) {
        match c {
            b' ' | b'\t' | b';' | b'&' | b'|' | b'<' | b'>' | b'(' | b')' => break,
            b'\'' => {
                let fim = b[i + 1..]
                    .iter()
                    .position(|x| *x == b'\'')
                    .ok_or_else(|| format!("delimitador de here-doc com aspas sem fim na linha {n}"))?;
                delimitador.extend_from_slice(&b[i + 1..i + 1 + fim]);
                i += fim + 2;
            }
            b'"' => {
                i += 1;
                loop {
                    match b.get(i) {
                        None => return Err(format!("delimitador de here-doc com aspas sem fim na linha {n}")),
                        Some(b'"') => {
                            i += 1;
                            break;
                        }
                        Some(b'\\') if b.get(i + 1).is_some_and(|x| matches!(x, b'"' | b'\\' | b'$' | b'`')) => {
                            delimitador.push(b[i + 1]);
                            i += 2;
                        }
                        Some(&x) => {
                            delimitador.push(x);
                            i += 1;
                        }
                    }
                }
            }
            b'\\' => {
                if let Some(&x) = b.get(i + 1) {
                    delimitador.push(x);
                }
                i += 2;
            }
            _ => {
                delimitador.push(c);
                i += 1;
            }
        }
    }
    if delimitador.is_empty() {
        return Err(format!("here-doc sem delimitador na linha {n}"));
    }
    let delimitador = String::from_utf8(delimitador).map_err(|_| format!("delimitador de here-doc da linha {n} não é UTF-8"))?;
    docs.push_back(HereDoc { delimitador, tira_tabs });
    Ok(i)
}

pub fn tirar_de_roteiro(fonte: &str) -> Result<String, String> {
    let mut saida = String::with_capacity(fonte.len());
    let mut leitor = LeitorSh {
        pilha: vec![(QuadroSh::Codigo { parenteses: 0, cases: 0, substituicao: false }, 0)],
        continua: false,
        palavra_antes_da_continuacao: false,
    };
    // Os here-docs abertos numa linha têm o corpo nas linhas seguintes, um depois do outro, na ordem dos `<<`.
    let mut corpos: VecDeque<HereDoc> = VecDeque::new();
    let mut ultima_em_branco = false;
    for (indice, com_quebra) in fonte.split_inclusive('\n').enumerate() {
        let n = indice + 1;
        let linha = com_quebra.strip_suffix('\n').unwrap_or(com_quebra);
        if let Some(doc) = corpos.front() {
            let comparada = if doc.tira_tabs { linha.trim_start_matches('\t') } else { linha };
            if comparada == doc.delimitador {
                corpos.pop_front();
            }
            saida.push_str(com_quebra);
            ultima_em_branco = false;
            continue;
        }
        if leitor.no_topo() {
            let sem_recuo = linha.trim_start_matches([' ', '\t']);
            if sem_recuo.starts_with('#') && !(n == 1 && linha.starts_with("#!")) {
                continue;
            }
            if sem_recuo.is_empty() {
                if !ultima_em_branco {
                    saida.push('\n');
                    ultima_em_branco = true;
                }
                continue;
            }
        }
        leitor.ler_linha(linha, n, &mut corpos)?;
        saida.push_str(com_quebra);
        ultima_em_branco = false;
    }
    if let Some(doc) = corpos.front() {
        return Err(format!("here-doc '{}' sem a linha de fim", doc.delimitador));
    }
    if let Some(&(q, aberto_na)) = leitor.pilha.get(1..).and_then(|resto| resto.last()) {
        return Err(format!("o {} aberto na linha {aberto_na} não fecha", q.nome()));
    }
    Ok(saida)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reds(s: &str) -> String {
        tirar_de_reds(s).expect("o .reds do teste é válido")
    }

    fn sh(s: &str) -> String {
        tirar_de_roteiro(s).expect("o roteiro do teste é válido")
    }

    #[test]
    fn reds_linha_e_fim_de_linha() {
        let fonte = "// cabeçalho\n// mais\n\nfunc A() -> Int32 {\n    return 72; // 0x48\n}\n";
        assert_eq!(reds(fonte), "func A() -> Int32 {\n    return 72;\n}\n");
    }

    #[test]
    fn reds_texto_entre_aspas_e_conteudo() {
        let fonte = "let a = \"http://x\"; // fora\nlet b = n\"a//b\";\nlet c = \"aspas \\\" // ainda texto\";\nlet d = r\"/* nao */\";\n";
        assert_eq!(
            reds(fonte),
            "let a = \"http://x\";\nlet b = n\"a//b\";\nlet c = \"aspas \\\" // ainda texto\";\nlet d = r\"/* nao */\";\n"
        );
    }

    #[test]
    fn reds_bloco_aninha_como_no_scc() {
        assert_eq!(reds("/* a /* b */ c */ func F() {}\n"), "func F() {}\n");
        assert_eq!(reds("a/*x*/b;\n"), "a b;\n");
        assert_eq!(reds("a /*x*/ b;\n"), "a b;\n");
        assert_eq!(reds("    /* recuo */ F();\n"), "    F();\n");
        assert_eq!(reds("func G() {\n  /* uma\n     duas */\n  F();\n}\n"), "func G() {\n  F();\n}\n");
        assert_eq!(reds("let x = 1 /* começa\n fim */ + 2;\n"), "let x = 1\n+ 2;\n");
    }

    #[test]
    fn reds_abertura_de_bloco_dentro_de_comentario_de_linha() {
        let fonte = "// alvos em Callback/Targets/*.reds\nclass A {}\n//   RawVFunc</* addr = */ 0x1D0,\nclass B {}\n";
        assert_eq!(reds(fonte), "class A {}\nclass B {}\n");
    }

    #[test]
    fn reds_interpolacao_volta_a_ser_codigo() {
        let fonte = "let s = s\"v=\\(F(\"//x\", (1))) e \\(G()) // y\"; // z\n";
        assert_eq!(reds(fonte), "let s = s\"v=\\(F(\"//x\", (1))) e \\(G()) // y\";\n");
        assert_eq!(reds("let t = s\"\\( /* c */ x )\";\n"), "let t = s\"\\( x )\";\n");
    }

    #[test]
    fn reds_texto_com_quebra_de_linha_fica_inteiro() {
        let fonte = "let a = \"um\n\n\n  // dois   \n\";\n";
        assert_eq!(reds(fonte), fonte);
    }

    #[test]
    fn reds_brancos_viram_um_so_e_somem_das_pontas() {
        let fonte = "\n\n// x\n\nclass A {}\n\n\n// y\n\nclass B {}\n\n// fim\n";
        assert_eq!(reds(fonte), "class A {}\n\nclass B {}\n");
        assert_eq!(reds("class A {} // sem quebra no fim"), "class A {}");
    }

    #[test]
    fn reds_recusa_o_que_nao_fecha() {
        assert!(tirar_de_reds("let a = \"aberto;\n").unwrap_err().contains("linha 1"));
        assert!(tirar_de_reds("x\n/* a /* b */\n").unwrap_err().contains("linha 2"));
    }

    #[test]
    fn roteiro_linha_inteira_sai_e_o_resto_fica() {
        let fonte = "#!/usr/bin/env bash\n# cabeçalho\nset -u\n  # recuado\necho \"a # b\" # fica\nx=${#y}\n";
        assert_eq!(sh(fonte), "#!/usr/bin/env bash\nset -u\necho \"a # b\" # fica\nx=${#y}\n");
    }

    #[test]
    fn roteiro_heredoc_guarda_o_conteudo() {
        let fonte = "cat > \"$F\" <<'TOML'\n# BWMS — conteúdo do arquivo\nchave = true\n\n\n# outro\nTOML\n# some\necho ok\n";
        assert_eq!(sh(fonte), "cat > \"$F\" <<'TOML'\n# BWMS — conteúdo do arquivo\nchave = true\n\n\n# outro\nTOML\necho ok\n");
        let tabs = "if x; then\n\tcat <<-FIM\n\t# fica\n\tFIM\nfi\n";
        assert_eq!(sh(tabs), tabs);
        let dois = "cat <<A <<\"B\"\n# a\nA\n# b\nB\n# sai\n";
        assert_eq!(sh(dois), "cat <<A <<\"B\"\n# a\nA\n# b\nB\n");
    }

    #[test]
    fn roteiro_heredoc_dentro_de_substituicao() {
        let fonte = "B=\"$(cat <<EOF\n# fica\n\\`\\`\\`\nEOF\n)\"\n# sai\nS=\"$(osascript <<'JS'\n// js (x)\nJS\n)\"\n";
        assert_eq!(sh(fonte), "B=\"$(cat <<EOF\n# fica\n\\`\\`\\`\nEOF\n)\"\nS=\"$(osascript <<'JS'\n// js (x)\nJS\n)\"\n");
    }

    #[test]
    fn roteiro_aspas_e_substituicao_de_varias_linhas() {
        let fonte = "M=\"um\n# dentro das aspas\n\"\nX=$(\n  # dentro do \\$(: fica\n  case $a in\n    b) echo 1 ;;\n  esac\n)\n# sai\n";
        assert_eq!(
            sh(fonte),
            "M=\"um\n# dentro das aspas\n\"\nX=$(\n  # dentro do \\$(: fica\n  case $a in\n    b) echo 1 ;;\n  esac\n)\n"
        );
    }

    #[test]
    fn roteiro_continuacao_segura_o_comentario() {
        // Sem a linha do `#`, o `echo b` viraria argumento do primeiro echo.
        let fonte = "echo a \\\n# fim do comando\necho b\n";
        assert_eq!(sh(fonte), fonte);
    }

    #[test]
    fn roteiro_nao_confunde_deslocamento_nem_here_string() {
        let fonte = "x=$((1 << 2))\n((y = x << 1))\nread -r a <<< \"$x\"\n# sai\necho $x\n";
        assert_eq!(sh(fonte), "x=$((1 << 2))\n((y = x << 1))\nread -r a <<< \"$x\"\necho $x\n");
    }

    #[test]
    fn roteiro_parametro_com_aspas_dentro() {
        let fonte = "p=\"${p#\\'}\"\np=\"${p#\"${p%%[![:space:]]*}\"}\"   # trim\ncase \"$v\" in \\#*|\"\") continue ;; esac\n# sai\nz=1\n";
        assert_eq!(
            sh(fonte),
            "p=\"${p#\\'}\"\np=\"${p#\"${p%%[![:space:]]*}\"}\"   # trim\ncase \"$v\" in \\#*|\"\") continue ;; esac\nz=1\n"
        );
    }

    #[test]
    fn roteiro_brancos_no_topo_viram_um() {
        assert_eq!(sh("a\n\n# x\n\n\nb\n"), "a\n\nb\n");
    }

    #[test]
    fn roteiro_recusa_o_que_nao_fecha() {
        assert!(tirar_de_roteiro("cat <<EOF\nsem fim\n").unwrap_err().contains("EOF"));
        assert!(tirar_de_roteiro("echo \"aberto\n").unwrap_err().contains("linha 1"));
        assert!(tirar_de_roteiro("x=$(\n  echo\n").unwrap_err().contains("$("));
    }
}
