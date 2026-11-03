//! A conferência de `native func` declarada num `.reds` × registrada num binário. No jogo, uma native declarada que nenhum binário registra
//! derruba o bind dos scripts com SIGTRAP, e o jogo fecha sem mensagem. A regra é uma só para quem confere: o `bwms-helper natives` (o
//! `bwms-install.sh` e o `dist/pack-bwms.sh`), o `bwms mods checar` (com as natives do relatório do compilador) e a compilação dos scripts
//! na abertura do jogo (`cp77-console`, `scripts::compilacao_no_lancamento`).

/// `nome` aparece nos bytes de algum dos binários. Substring simples: os literais de texto do Rust ficam colados no binário, sem zero entre
/// eles, então um nome pode não ter delimitador (risco pequeno de falso positivo por coincidência, como um nome que é prefixo de outro).
/// O primeiro byte filtra a janela antes da comparação: são centenas de nomes contra um dylib de quase 2 MB.
pub fn registrada_em(nome: &str, binarios: &[Vec<u8>]) -> bool {
    let alvo = nome.as_bytes();
    let Some(&primeiro) = alvo.first() else {
        return false;
    };
    binarios.iter().any(|b| b.windows(alvo.len()).any(|w| w[0] == primeiro && w == alvo))
}

/// `nome` registrado pelo próprio jogo. A 2.5 tirou do bundle a declaração de natives que o executável ainda registra (`Log`,
/// `LogChannel`, `Trace`…), e a ponte do BWMS as declara de novo para mod de Windows compilar. O executável guarda o nome como texto C:
/// casa só com zero antes e depois (no 2.5, 1 ocorrência de cada uma das 8). Só para o nome que nenhum binário do BWMS registra, porque o
/// executável tem centenas de MB.
pub fn registrada_no_jogo(nome: &str, jogo: &[u8]) -> bool {
    let mut alvo = Vec::with_capacity(nome.len() + 2);
    alvo.push(0);
    alvo.extend_from_slice(nome.as_bytes());
    alvo.push(0);
    if alvo.len() < 3 {
        return false;
    }
    jogo.windows(alvo.len()).any(|w| w[1] == alvo[1] && w == alvo.as_slice())
}

/// Uma `native func` que o texto de um `.reds` declara.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeDeclarada<'a> {
    /// A linha da declaração; a primeira do arquivo é 1.
    pub linha: usize,
    /// Na declaração indentada, a classe declarada na coluna 0 que a contém; na da coluna 0, `None` (função global).
    pub dono: Option<&'a str>,
    pub nome: &'a str,
}

/// O resto da linha de `.reds` depois de `native func`, quando a linha declara uma native global: só espaço antes
/// (`^[[:space:]]*native func`) ou modificadores em minúscula desde a coluna 0 (`^([a-z]+ )+native func`, o `public static native func`).
/// Comentário de linha não casa (começa com `/`). Método `native` indentado com modificador (`    public native func GetTags`) fica de
/// fora: o nome curto de método casaria como substring em quase qualquer binário, e a checagem por bytes não diria nada dele.
fn depois_de_native_func(linha: &str) -> Option<&str> {
    const DECLARA: &str = "native func";
    if let Some(resto) = linha.trim_start_matches([' ', '\t', '\r', '\x0b', '\x0c']).strip_prefix(DECLARA) {
        return Some(resto);
    }
    let mut s = linha;
    loop {
        let palavra = s.len() - s.trim_start_matches(|c: char| c.is_ascii_lowercase()).len();
        if palavra == 0 {
            return None;
        }
        s = s[palavra..].strip_prefix(' ')?;
        if let Some(resto) = s.strip_prefix(DECLARA) {
            return Some(resto);
        }
    }
}

/// O identificador no começo de `texto` (letra, dígito ou `_`); vazio quando não há.
fn identificador(texto: &str) -> &str {
    let fim = texto.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).unwrap_or(texto.len());
    &texto[..fim]
}

/// O nome da classe que a linha declara na coluna 0: `class Nome` depois de modificadores em minúscula (`public native class Nome`).
fn classe_declarada(linha: &str) -> Option<&str> {
    let mut s = linha;
    loop {
        if let Some(resto) = s.strip_prefix("class ") {
            let nome = identificador(resto.trim_start_matches(' '));
            return (!nome.is_empty()).then_some(nome);
        }
        let palavra = s.len() - s.trim_start_matches(|c: char| c.is_ascii_lowercase()).len();
        if palavra == 0 {
            return None;
        }
        s = s[palavra..].strip_prefix(' ')?;
    }
}

/// Cada `native func` que o texto de um `.reds` declara (ver [`depois_de_native_func`]), na ordem do arquivo. O dono de uma declaração
/// indentada é a última classe declarada na coluna 0 antes dela, até uma linha que começa com `}` na coluna 0.
pub fn declaradas_em(texto: &str) -> Vec<NativeDeclarada<'_>> {
    let mut dono = None;
    let mut achadas = Vec::new();
    for (i, linha) in texto.lines().enumerate() {
        if let Some(classe) = classe_declarada(linha) {
            dono = Some(classe);
        } else if linha.starts_with('}') {
            dono = None;
        }
        let Some(resto) = depois_de_native_func(linha) else { continue };
        let depois_do_espaco = resto.trim_start_matches([' ', '\t']);
        if depois_do_espaco.len() == resto.len() {
            continue;
        }
        let nome = identificador(depois_do_espaco);
        if nome.is_empty() {
            continue;
        }
        let indentada = linha.starts_with([' ', '\t']);
        achadas.push(NativeDeclarada { linha: i + 1, dono: if indentada { dono } else { None }, nome });
    }
    achadas
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_do_jogo_casa_so_como_texto_c_inteiro() {
        let exe = b"\x00ScriptLog\x00LogChannel\x00LogChannelError\x00";
        assert!(registrada_no_jogo("LogChannel", exe));
        assert!(registrada_no_jogo("LogChannelError", exe));
        assert!(!registrada_no_jogo("Log", exe), "prefixo de outro nome não conta");
        assert!(!registrada_no_jogo("Channel", exe), "pedaço do meio não conta");
        assert!(!registrada_no_jogo("", exe));
    }

    fn nomes(texto: &str) -> Vec<&str> {
        declaradas_em(texto).iter().map(|d| d.nome).collect()
    }

    #[test]
    fn nome_nos_bytes_mesmo_sem_delimitador() {
        let bin = vec![b"xxBwmsExisteBwmsOutraZ".to_vec()];
        assert!(registrada_em("BwmsExiste", &bin));
        assert!(registrada_em("BwmsOutra", &bin));
        assert!(!registrada_em("BwmsAusente", &bin));
        assert!(!registrada_em("", &bin));
    }

    #[test]
    fn qualquer_um_dos_binarios_e_nas_pontas() {
        let bins = vec![b"Bwms".to_vec(), b"plugin\0BwmsDoPlugin".to_vec()];
        assert!(registrada_em("BwmsDoPlugin", &bins), "nome no fim do segundo binário");
        assert!(registrada_em("Bwms", &bins), "nome do tamanho do binário inteiro");
        assert!(!registrada_em("BwmsMaiorQueOBinario", &bins[..1]));
        assert!(!registrada_em("x", &[]));
    }

    #[test]
    fn declaradas_pela_regra_de_linha() {
        let texto = "native func BwmsGlobal() -> Bool\n\
                     \x20 native func BwmsIndentado(x: Int32)\n\
                     \tnative func BwmsTab()\n\
                     public static native func BwmsEstatica() -> String;\n\
                     public native func BwmsPublica()\n\
                     \x20   public native func MetodoDeClasse() -> CName\n\
                     // native func BwmsComentada()\n\
                     //native func BwmsComentada2()\n\
                     @addMethod(X) public native func BwmsAnotada()\n\
                     Public native func BwmsMaiuscula()\n\
                     public  native func BwmsDoisEspacosAntes()\n\
                     func BwmsNaoNative()\n\
                     native funcBwmsColado()\n\
                     native func (\n\
                     native func BwmsCrlf()\r\n";
        assert_eq!(nomes(texto), ["BwmsGlobal", "BwmsIndentado", "BwmsTab", "BwmsEstatica", "BwmsPublica", "BwmsCrlf"]);
    }

    #[test]
    fn espaco_a_mais_antes_do_nome_ainda_e_o_nome() {
        // Espaço a mais ou tab depois de `func` é redscript válido: o nome é o identificador que vem depois, nunca o resto da linha.
        assert_eq!(nomes("native func  BwmsDois()\nnative func\tBwmsTab()"), ["BwmsDois", "BwmsTab"]);
    }

    #[test]
    fn linha_e_dono_de_cada_declaracao() {
        let texto = "module Plugin\n\
                     \n\
                     native func NaoRegistrada()\n\
                     public native class Cliente extends IScriptable {\n\
                     \x20 native func Conectar() -> Bool\n\
                     }\n\
                     \x20 native func DepoisDaClasse()\n\
                     class Uma { }\n\
                     \tnative func NaUma()\n\
                     public static native func Global()\n";
        assert_eq!(
            declaradas_em(texto),
            [
                NativeDeclarada { linha: 3, dono: None, nome: "NaoRegistrada" },
                NativeDeclarada { linha: 5, dono: Some("Cliente"), nome: "Conectar" },
                NativeDeclarada { linha: 7, dono: None, nome: "DepoisDaClasse" },
                NativeDeclarada { linha: 9, dono: Some("Uma"), nome: "NaUma" },
                NativeDeclarada { linha: 10, dono: None, nome: "Global" },
            ]
        );
    }
}
