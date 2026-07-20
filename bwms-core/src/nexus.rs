//! `nexus`: a BIBLIOTECA de mods do BWMS no macOS — onde o manager guarda CADA mod de terceiro
//! (baixado do Nexus ou solto), com o original intacto + um manifesto de metadata por mod. É a
//! camada 1 do modelo (ver `cp77-symbols/notes/MODELO-BIBLIOTECA-NEXUS.md`): a biblioteca GUARDA;
//! o `apply`/deploy é que COPIA pras pastas de load do jogo. O jogo NUNCA lê daqui.
//!
//! Premissa (verificada julho/2026): não há Vortex nem manager oficial do Nexus no macOS — o BWMS
//! é o próprio manager. Tomamos emprestada só a CONVENÇÃO: pasta-por-mod (SMAPI) + manifesto
//! distribuído legível (MO2 `meta.ini`), sem DB proprietário. A chave canônica é o par
//! `(mod_id, file_id)`, o MESMO do link `nxm://cyberpunk2077/mods/<mod_id>/files/<file_id>` — dá
//! update-check/re-download/endosso de graça quando o handler `nxm://` for ligado.

use std::path::{Path, PathBuf};

/// Raiz da biblioteca, relativa ao jogo: `<game>/BWMS/nexus/`. Fica ao lado do staging por-tema
/// (`BWMS/mods`, do `apply`), mas é OUTRA coisa — biblioteca (identidade Nexus) ≠ staging de load.
const LIBRARY: &str = "BWMS/nexus";
/// Nome do manifesto por-mod na raiz da pasta de cada mod.
pub const MANIFEST_FILE: &str = "bwms.toml";

/// `<game>/BWMS/nexus/`.
pub fn library_dir(game: &Path) -> PathBuf {
    game.join(LIBRARY)
}
/// `<game>/BWMS/nexus/mods/`.
pub fn mods_dir(game: &Path) -> PathBuf {
    library_dir(game).join("mods")
}
/// `<game>/BWMS/nexus/mods/<unique_id>/` — a pasta OWN de um mod (sem sufixo numérico).
pub fn mod_dir(game: &Path, unique_id: &str) -> PathBuf {
    mods_dir(game).join(unique_id)
}
/// `<game>/BWMS/nexus/mods/<unique_id>/bwms.toml`.
pub fn manifest_path(game: &Path, unique_id: &str) -> PathBuf {
    mod_dir(game, unique_id).join(MANIFEST_FILE)
}

/// Metadata de UM mod na biblioteca (o `bwms.toml`). Modelo MO2 (legível/versionável), com a
/// identidade Nexus mínima. Campos ausentes no arquivo caem no default (parser tolerante).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    pub game_domain: String,       // "cyberpunk2077"
    pub mod_id: Option<u64>,       // nº da página no Nexus (None = manual/sem página)
    pub file_id: Option<u64>,      // arquivo/versão específico (None = manual)
    pub version: String,           // semver ("1.3.0")
    pub repository: String,        // "Nexus" | "manual"
    pub unique_id: String,         // "Autor.NomeDoMod" (sem espaço) — a pasta
    pub name: String,              // nome legível
    pub author: String,
    pub installation_file: String, // nome do archive de origem
    pub installed_at: String,      // ISO-8601 (quem chama carimba; core não lê relógio)
    pub last_nexus_update: String, // p/ update-check
    pub enabled: bool,
    pub category: String,          // tema/pasta lógica (Roupas/Veículos/...)
    pub dependencies: Vec<String>, // unique_ids de deps (SMAPI-style)
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            schema: 1,
            game_domain: "cyberpunk2077".into(),
            mod_id: None,
            file_id: None,
            version: String::new(),
            repository: "manual".into(),
            unique_id: String::new(),
            name: String::new(),
            author: String::new(),
            installation_file: String::new(),
            installed_at: String::new(),
            last_nexus_update: String::new(),
            enabled: true,
            category: String::new(),
            dependencies: Vec::new(),
        }
    }
}

/// Escapa `\` e `"` p/ um valor de string do mini-TOML (basta pros nomes/versões/datas reais).
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
fn unesc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('"') => out.push('"'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Manifest {
    /// Serializa pro formato `bwms.toml` (flat `key = value`, editável à mão). `mod_id`/`file_id`
    /// só saem se `Some`. `dependencies` vira lista separada por vírgula numa string (parse trivial).
    pub fn to_toml(&self) -> String {
        let mut s = String::new();
        s.push_str("# bwms.toml — manifesto do mod na biblioteca do BWMS (ver MODELO-BIBLIOTECA-NEXUS.md)\n");
        s.push_str(&format!("schema = {}\n", self.schema));
        s.push_str(&format!("unique_id = \"{}\"\n", esc(&self.unique_id)));
        s.push_str(&format!("name = \"{}\"\n", esc(&self.name)));
        s.push_str(&format!("author = \"{}\"\n", esc(&self.author)));
        s.push_str(&format!("version = \"{}\"\n", esc(&self.version)));
        s.push_str(&format!("game_domain = \"{}\"\n", esc(&self.game_domain)));
        s.push_str(&format!("repository = \"{}\"\n", esc(&self.repository)));
        if let Some(id) = self.mod_id {
            s.push_str(&format!("mod_id = {id}\n"));
        }
        if let Some(id) = self.file_id {
            s.push_str(&format!("file_id = {id}\n"));
        }
        s.push_str(&format!("installation_file = \"{}\"\n", esc(&self.installation_file)));
        s.push_str(&format!("installed_at = \"{}\"\n", esc(&self.installed_at)));
        s.push_str(&format!("last_nexus_update = \"{}\"\n", esc(&self.last_nexus_update)));
        s.push_str(&format!("enabled = {}\n", self.enabled));
        s.push_str(&format!("category = \"{}\"\n", esc(&self.category)));
        s.push_str(&format!("dependencies = \"{}\"\n", esc(&self.dependencies.join(", "))));
        s
    }

    /// Parser tolerante: linha `key = value`, `#`=comentário; strings entre aspas (unescape),
    /// ints/bools crus; chave desconhecida ignorada; ausente → default. Nunca falha.
    pub fn from_toml(text: &str) -> Manifest {
        let mut m = Manifest::default();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let key = k.trim();
            let mut val = v.trim().to_string();
            // desembrulha aspas
            let quoted = val.len() >= 2 && val.starts_with('"') && val.ends_with('"');
            if quoted {
                val = unesc(&val[1..val.len() - 1]);
            }
            match key {
                "schema" => m.schema = val.parse().unwrap_or(1),
                "unique_id" => m.unique_id = val,
                "name" => m.name = val,
                "author" => m.author = val,
                "version" => m.version = val,
                "game_domain" => m.game_domain = val,
                "repository" => m.repository = val,
                "mod_id" => m.mod_id = val.parse().ok(),
                "file_id" => m.file_id = val.parse().ok(),
                "installation_file" => m.installation_file = val,
                "installed_at" => m.installed_at = val,
                "last_nexus_update" => m.last_nexus_update = val,
                "enabled" => m.enabled = val != "false" && val != "0",
                "category" => m.category = val,
                "dependencies" => {
                    m.dependencies = val
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
                _ => {}
            }
        }
        m
    }

    /// Grava o manifesto na pasta do mod (`<lib>/mods/<unique_id>/bwms.toml`), criando os dirs.
    pub fn write(&self, game: &Path) -> std::io::Result<()> {
        let dir = mod_dir(game, &self.unique_id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(MANIFEST_FILE), self.to_toml())
    }

    /// Lê o manifesto de um mod da biblioteca (None se não existe/ilegível).
    pub fn read(game: &Path, unique_id: &str) -> Option<Manifest> {
        let text = std::fs::read_to_string(manifest_path(game, unique_id)).ok()?;
        Some(Manifest::from_toml(&text))
    }

    /// A chave canônica `(mod_id, file_id)` — só `Some` se AMBOS presentes (é o par do `nxm://`).
    pub fn nexus_key(&self) -> Option<(u64, u64)> {
        Some((self.mod_id?, self.file_id?))
    }
}

/// Parseia um link `nxm://cyberpunk2077/mods/<mod_id>/files/<file_id>[?...]` → `(game_domain,
/// mod_id, file_id)`. É o link que o botão "Mod Manager Download" do Nexus entrega — a fase 3 (o
/// `.app` reivindicando o esquema `nxm://`) chama isto p/ preencher o manifesto sozinho. Tolerante
/// à query-string (`?key=...&expires=...`) que o Nexus anexa. `None` se o formato não bate.
pub fn parse_nxm(url: &str) -> Option<(String, u64, u64)> {
    let rest = url.strip_prefix("nxm://")?;
    let rest = rest.split(['?', '#']).next().unwrap_or(rest); // tira query/fragment
    let parts: Vec<&str> = rest.split('/').filter(|s| !s.is_empty()).collect();
    // esperado: [game_domain, "mods", <mod_id>, "files", <file_id>]
    if parts.len() < 5 || parts[1] != "mods" || parts[3] != "files" {
        return None;
    }
    let mod_id = parts[2].parse().ok()?;
    let file_id = parts[4].parse().ok()?;
    Some((parts[0].to_string(), mod_id, file_id))
}

/// Sanitiza um pedaço de identificador SMAPI-style: mantém alfanumérico (preservando maiúsculas),
/// remove espaços/pontuação. "Immersive First Person" → "ImmersiveFirstPerson". Vazio → "Unknown".
pub fn sanitize_ident(s: &str) -> String {
    let out: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if out.is_empty() {
        "Unknown".into()
    } else {
        out
    }
}

/// Deriva o `unique_id` SMAPI-style `Autor.NomeDoMod` (sem espaço). Autor vazio → "Unknown".
pub fn make_unique_id(author: &str, name: &str) -> String {
    format!("{}.{}", sanitize_ident(author), sanitize_ident(name))
}

/// Dados p/ importar um mod pra biblioteca. O core NÃO lê relógio nem rede: quem chama (CLI/TUI)
/// preenche `installed_at` e, se veio de um `nxm://`, `mod_id`/`file_id`. `version` pode vir do
/// nome do arquivo (ver [`infer_name_version`]) ou da metadata do Nexus.
#[derive(Debug, Clone, Default)]
pub struct ImportInfo {
    pub name: String,
    pub author: String,
    pub version: String,
    pub mod_id: Option<u64>,
    pub file_id: Option<u64>,
    pub installation_file: String,
    pub installed_at: String,
    pub category: String,
}

/// Copia a árvore de `src` → `dst` (recursivo), preservando estrutura. Pula `.git`/`.DS_Store` e
/// um `bwms.toml` que porventura esteja no zip de origem (o nosso manifesto é gravado depois).
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if n == ".git" || n == ".DS_Store" || n == MANIFEST_FILE {
            continue;
        }
        let from = e.path();
        let to = dst.join(&name);
        if e.file_type()?.is_dir() {
            std::fs::create_dir_all(&to)?;
            copy_tree(&from, &to)?;
        } else if e.file_type()?.is_file() {
            if let Some(p) = to.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Importa um mod JÁ EXTRAÍDO em `src` pra biblioteca: deriva o `unique_id` (Autor.NomeDoMod),
/// copia a árvore pra `nexus/mods/<unique_id>/` (estrutura preservada, SEM sufixo numérico) e grava
/// o `bwms.toml`. Idempotente: re-importar SUBSTITUI só a pasta daquele mod. `repository` = "Nexus"
/// se veio `mod_id`, senão "manual". A extração de zip/rar/7z é do chamador (o binário).
pub fn import_from_dir(game: &Path, src: &Path, info: &ImportInfo) -> std::io::Result<Manifest> {
    let unique_id = make_unique_id(&info.author, &info.name);
    let dest = mod_dir(game, &unique_id);
    let _ = std::fs::remove_dir_all(&dest); // re-import limpo (só ESTE mod)
    std::fs::create_dir_all(&dest)?;
    copy_tree(src, &dest)?;
    let m = Manifest {
        unique_id,
        name: info.name.clone(),
        author: info.author.clone(),
        version: info.version.clone(),
        mod_id: info.mod_id,
        file_id: info.file_id,
        repository: if info.mod_id.is_some() { "Nexus".into() } else { "manual".into() },
        installation_file: info.installation_file.clone(),
        installed_at: info.installed_at.clone(),
        enabled: true,
        category: info.category.clone(),
        ..Default::default()
    };
    m.write(game)?;
    Ok(m)
}

/// Remove um mod da biblioteca (a pasta inteira). Não mexe no deploy — quem chama deve purgar o
/// deploy antes (senão os arquivos aplicados no jogo ficam órfãos). Devolve true se removeu.
pub fn remove_from_library(game: &Path, unique_id: &str) -> bool {
    std::fs::remove_dir_all(mod_dir(game, unique_id)).is_ok()
}

/// Heurística best-effort p/ separar nome e versão do nome de um arquivo do Nexus, tipo
/// `"Immersive First Person V133-669-V133-1752951286.zip"` → ("Immersive First Person", "133").
/// Reconhece um token `V<digitos>` (o padrão de versão do Nexus) OU um `<maj>.<min>[.<pat>]`. Se
/// nada casar, devolve (stem-limpo, ""). Não é infalível — só um ponto de partida pro import manual.
pub fn infer_name_version(filename: &str) -> (String, String) {
    let stem = filename
        .rsplit_once('.')
        .map(|(a, _)| a)
        .unwrap_or(filename);
    // procura o 1º token que começa com 'V'/'v' seguido de dígito, ou um dotted-number.
    let mut version = String::new();
    let mut cut = stem.len();
    for (i, tok) in stem.split(|c: char| c == ' ' || c == '-' || c == '_').enumerate() {
        let is_vtag = {
            let mut ch = tok.chars();
            matches!(ch.next(), Some('v' | 'V')) && ch.next().is_some_and(|c| c.is_ascii_digit())
        };
        let is_dotted = tok.contains('.')
            && tok.chars().all(|c| c.is_ascii_digit() || c == '.')
            && tok.chars().any(|c| c.is_ascii_digit());
        if (is_vtag || is_dotted) && i > 0 {
            version = tok.trim_start_matches(['v', 'V']).to_string();
            // posição do token no stem original p/ cortar o nome
            if let Some(pos) = stem.find(tok) {
                cut = pos;
            }
            break;
        }
    }
    let name = stem[..cut].trim_matches([' ', '-', '_']).to_string();
    (if name.is_empty() { stem.to_string() } else { name }, version)
}

/// Lista os `unique_id`s presentes na biblioteca (subpastas de `nexus/mods/` com um `bwms.toml`).
pub fn list_library(game: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(mods_dir(game)) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    if e.path().join(MANIFEST_FILE).exists() {
                        out.push(n.to_string());
                    }
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        Manifest {
            mod_id: Some(12345),
            file_id: Some(67890),
            version: "1.3.0".into(),
            repository: "Nexus".into(),
            unique_id: "Autor.ImmersiveFirstPerson".into(),
            name: "Immersive First Person".into(),
            author: "Autor".into(),
            installation_file: "Immersive First Person V133-669.zip".into(),
            installed_at: "2026-07-16T12:00:00Z".into(),
            last_nexus_update: "2026-07-16".into(),
            enabled: true,
            category: "gameplay".into(),
            dependencies: vec!["Autor.OutroMod".into(), "Autor.Terceiro".into()],
            ..Default::default()
        }
    }

    #[test]
    fn manifesto_round_trip() {
        let m = sample();
        let parsed = Manifest::from_toml(&m.to_toml());
        assert_eq!(parsed, m);
        assert_eq!(parsed.nexus_key(), Some((12345, 67890)));
    }

    #[test]
    fn manifesto_manual_sem_ids() {
        let mut m = Manifest::default();
        m.unique_id = "Autor.ModSolto".into();
        m.name = "Mod Solto".into();
        m.author = "Autor".into();
        // sem mod_id/file_id → não emite as linhas, e nexus_key = None
        let toml = m.to_toml();
        assert!(!toml.contains("mod_id"));
        assert!(!toml.contains("file_id"));
        let parsed = Manifest::from_toml(&toml);
        assert_eq!(parsed.nexus_key(), None);
        assert_eq!(parsed.repository, "manual");
        assert_eq!(parsed, m);
    }

    #[test]
    fn parser_tolerante_a_lixo_e_defaults() {
        let text = "# comentário\nunique_id = \"A.B\"\nlixo sem igual\nchave_desconhecida = 1\nenabled = false\nmod_id = 42\n";
        let m = Manifest::from_toml(text);
        assert_eq!(m.unique_id, "A.B");
        assert!(!m.enabled);
        assert_eq!(m.mod_id, Some(42));
        assert_eq!(m.file_id, None);
        assert_eq!(m.schema, 1); // default preservado
        assert_eq!(m.game_domain, "cyberpunk2077"); // default
    }

    #[test]
    fn escape_de_aspas_e_barra() {
        let mut m = Manifest::default();
        m.unique_id = "A.B".into();
        m.installation_file = "arquivo \"com aspas\" e \\barra.zip".into();
        let parsed = Manifest::from_toml(&m.to_toml());
        assert_eq!(parsed.installation_file, "arquivo \"com aspas\" e \\barra.zip");
    }

    #[test]
    fn unique_id_smapi_style() {
        assert_eq!(make_unique_id("Autor", "Immersive First Person"), "Autor.ImmersiveFirstPerson");
        assert_eq!(make_unique_id("", "Mod!!"), "Unknown.Mod");
        assert_eq!(sanitize_ident("  já  "), "j"); // remove não-ascii-alfanum (acento/espaço)
    }

    #[test]
    fn import_copia_arvore_e_grava_manifesto() {
        let g = std::env::temp_dir().join(format!("bwms-nexus-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        // "mod extraído" de origem, com estrutura de zip do Nexus
        let src = g.join("_src");
        std::fs::create_dir_all(src.join("archive/pc/mod")).unwrap();
        std::fs::create_dir_all(src.join("r6/scripts")).unwrap();
        std::fs::write(src.join("archive/pc/mod/ifp.archive"), b"FAKEARCH").unwrap();
        std::fs::write(src.join("r6/scripts/ifp.reds"), b"// reds").unwrap();
        std::fs::write(src.join(".DS_Store"), b"junk").unwrap(); // deve ser pulado

        let info = ImportInfo {
            name: "Immersive First Person".into(),
            author: "Autor".into(),
            version: "1.3.0".into(),
            mod_id: Some(12345),
            file_id: Some(67890),
            installation_file: "ifp.zip".into(),
            installed_at: "2026-07-16T00:00:00Z".into(),
            category: "gameplay".into(),
        };
        let m = import_from_dir(&g, &src, &info).unwrap();
        assert_eq!(m.unique_id, "Autor.ImmersiveFirstPerson");
        assert_eq!(m.repository, "Nexus");
        let md = mod_dir(&g, "Autor.ImmersiveFirstPerson");
        assert!(md.join("archive/pc/mod/ifp.archive").exists());
        assert!(md.join("r6/scripts/ifp.reds").exists());
        assert!(md.join(MANIFEST_FILE).exists());
        assert!(!md.join(".DS_Store").exists(), ".DS_Store não devia ser copiado");
        // re-import é idempotente (não duplica, não quebra)
        let m2 = import_from_dir(&g, &src, &info).unwrap();
        assert_eq!(m2, m);
        // remove da biblioteca
        assert!(remove_from_library(&g, "Autor.ImmersiveFirstPerson"));
        assert!(!md.exists());
        let _ = std::fs::remove_dir_all(&g);
    }

    #[test]
    fn infer_name_version_do_nome_do_arquivo() {
        assert_eq!(
            infer_name_version("Immersive First Person V133-669-V133-1752951286.zip"),
            ("Immersive First Person".into(), "133".into())
        );
        assert_eq!(
            infer_name_version("Better Vehicle Handling 2.1.0.rar"),
            ("Better Vehicle Handling".into(), "2.1.0".into())
        );
        // sem versão reconhecível → nome = stem, versão vazia
        assert_eq!(infer_name_version("MeuMod.zip"), ("MeuMod".into(), "".into()));
    }

    #[test]
    fn parse_nxm_link() {
        assert_eq!(
            parse_nxm("nxm://cyberpunk2077/mods/12345/files/67890"),
            Some(("cyberpunk2077".into(), 12345, 67890))
        );
        // com query-string do Nexus (key/expires/user)
        assert_eq!(
            parse_nxm("nxm://cyberpunk2077/mods/12345/files/67890?key=abc&expires=999&user_id=1"),
            Some(("cyberpunk2077".into(), 12345, 67890))
        );
        // formato errado
        assert_eq!(parse_nxm("https://nexusmods.com/cyberpunk2077/mods/1"), None);
        assert_eq!(parse_nxm("nxm://cyberpunk2077/mods/12345"), None);
    }

    #[test]
    fn write_read_e_lista_a_biblioteca() {
        let g = std::env::temp_dir().join(format!("bwms-nexus-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&g);
        let m = sample();
        m.write(&g).unwrap();
        // caminho canônico
        assert!(manifest_path(&g, "Autor.ImmersiveFirstPerson").exists());
        let back = Manifest::read(&g, "Autor.ImmersiveFirstPerson").unwrap();
        assert_eq!(back, m);
        assert_eq!(list_library(&g), vec!["Autor.ImmersiveFirstPerson".to_string()]);
        let _ = std::fs::remove_dir_all(&g);
    }
}
