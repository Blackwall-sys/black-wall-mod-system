//! Biblioteca do `tweakdb-tool` — expõe SÓ o parser puro do formato declarativo do TweakXL
//! (`.yaml`/`.toml` → `Vec<Op>`), pro runtime (`cp77-console`) reusar sem duplicar código.
//!
//! Deliberadamente NÃO expõe `writer`/`tweakdb`/`kraken`/`ui`/`names` — essas dependem da
//! decodificação Kraken nativa (`build.rs`+`ooz`) e do `Model` offline, que o dylib do jogo não
//! precisa (o runtime aplica direto no TweakDB VIVO, não num `Model` de arquivo). `tweakxl.rs`
//! (com `EditOp` movido pra dentro dele, 2026-07-15) é auto-contido: só depende de `yaml`+`hashes`.

pub mod hashes;
pub mod template;
pub mod tweakxl;
pub mod yaml;
