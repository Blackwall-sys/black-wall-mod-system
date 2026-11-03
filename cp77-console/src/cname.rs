//! CName do CP2077 = FNV1a64 do nome flat (ASCII). Idêntico ao do
//! redscript — e ao `fnv1a64` do `bwms-primitives`, o mesmo do tweakdb-tool. É como se resolve um tipo
//! ou função RED por nome em runtime (IRTTISystem::GetClass(CName), etc.). Os tipos [`CName`] e
//! [`ResourcePathHash`] separam esses ids nas fronteiras com `u64` de espécies misturadas.

/// CName(nome) = FNV1a64(nome), pela fonte única `bwms-primitives`. `"None"`/vazio → 0 no engine (tratado pelo caller).
pub const fn cname(name: &str) -> u64 {
    bwms_primitives::fnv1a64(name.as_bytes())
}

/// `CName` nas assinaturas que recebem vários `u64` de espécies diferentes (entidade, hash de item, `NodeRef`, máscara, caminho): trocar a ordem
/// vira erro de compilação. Só nessas fronteiras; o resto do crate segue com o `u64` cru.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CName(u64);

impl CName {
    /// O hash de `nome` ([`cname`]).
    pub(crate) const fn cname(nome: &str) -> Self {
        Self(cname(nome))
    }

    /// O valor que veio pronto da memória do motor ou do frame de script, sem recalcular.
    pub(crate) const fn lido_do_motor(hash: u64) -> Self {
        Self(hash)
    }

    pub(crate) const fn valor(self) -> u64 {
        self.0
    }
}

/// Hash de caminho de recurso (`bwms_primitives::resource_path_hash`) nas mesmas fronteiras do [`CName`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResourcePathHash(u64);

impl ResourcePathHash {
    /// O valor que veio pronto da memória do motor ou do frame de script, sem recalcular.
    pub(crate) const fn lido_do_motor(hash: u64) -> Self {
        Self(hash)
    }

    pub(crate) const fn valor(self) -> u64 {
        self.0
    }
}

/// Pool de nomes (hash→string) pra `name_of`/`espelhado` reverterem — espelha o CNamePool do
/// engine. Populado por `intern`: o lado Lua (`CName.add`/`ToCName`/`StringToName`) e o Rust
/// que monta CName pro motor (`tipos_do_motor`, `aparencia`, `register`). (Não escreve
/// no pool nativo do jogo; isso é o `registrar_no_pool`.)
static POOL: std::sync::Mutex<Option<std::collections::HashMap<u64, String>>> =
    std::sync::Mutex::new(None);

/// Interna o nome (string→hash) no nosso espelho e devolve o hash. Idempotente.
pub fn intern(name: &str) -> u64 {
    let h = cname(name);
    if let Ok(mut g) = POOL.lock() {
        g.get_or_insert_with(std::collections::HashMap::new)
            .entry(h)
            .or_insert_with(|| name.to_string());
    }
    h
}

/// CNamePool::Get(hash) → char* (C-string null-terminated) no engine. Reverte qualquer
/// CName, não só os internados por nós. Endereço sem símbolo (inlined) → VERSIONADO,
/// validar com um hash conhecido no boot. ⚠️ muda por patch do jogo.
const VM_CNAMEPOOL_GET: u64 = 0x1_04e3_8560;

/// Resolve um CName (hash FNV1a64) → nome via pool NATIVO do engine. "" se desconhecido,
/// "None" se hash 0. Variante C-string (lê só x0 e varre até o NUL) p/ robustez de ABI.
/// Só seguro DEPOIS da RTTI viva (o hook do executor já garante isso quando isto roda).
pub fn resolve_cname(hash: u64) -> String {
    if hash == 0 {
        return "None".into();
    }
    unsafe {
        type Get = unsafe extern "C" fn(u64) -> *const i8;
        let get: Get = core::mem::transmute(crate::rebase(VM_CNAMEPOOL_GET));
        let p = get(hash);
        if p.is_null() || !crate::memoria::is_readable(p as *const std::ffi::c_void, 1) {
            return String::new();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

/// `CName(const char*)` do motor que registra o texto no pool (`0x104e389d0`): mede, calcula o FNV-1a64 e grava hash, texto e tamanho antes de devolver o
/// hash. Depois dele o `CNamePool::Get` resolve o nome. Texto vazio, `None` ou com NUL devolvem 0 sem registrar.
const VM_CNAME_REGISTRAR: u64 = 0x1_04e3_89d0;

/// Registra `nome` no pool NATIVO do motor e devolve o hash (o `CNamePool::Add` da referência). Só depois do motor no ar.
pub fn registrar_no_pool(nome: &str) -> u64 {
    let Ok(texto) = std::ffi::CString::new(nome) else { return 0 };
    unsafe {
        let registrar: unsafe extern "C" fn(*const i8) -> u64 = core::mem::transmute(crate::rebase(VM_CNAME_REGISTRAR));
        registrar(texto.as_ptr())
    }
}

/// Nome que o BWMS internou (`intern`), sem tocar o pool do motor.
pub fn espelhado(hash: u64) -> Option<String> {
    POOL.lock().ok()?.as_ref()?.get(&hash).cloned()
}

/// Reverte um hash pro nome: primeiro o espelho interno (intern), senão o pool NATIVO
/// (resolve_cname). None se nenhum resolver.
pub fn name_of(hash: u64) -> Option<String> {
    if let Some(s) = espelhado(hash) {
        return Some(s);
    }
    let s = resolve_cname(hash);
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vetor_padrao_fnv() {
        // Vetores canônicos do FNV-1a 64.
        assert_eq!(cname(""), bwms_primitives::FNV1A64_OFFSET);
        assert_eq!(cname("a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn cnames_dos_sistemas() {
        // Conferidos contra o cálculo Python no recon (mesmo algoritmo).
        assert_eq!(cname("gameStatsSystem"), 0x7617_74a5_71cc_8913);
        assert_eq!(cname("gameCameraSystem"), 0x73b1_5f77_7a57_3929);
        assert_eq!(cname("gameDamageSystem"), 0x3619_c142_d3ec_2e47);
        assert_eq!(cname("GetStatValue"), 0x2342_7ae3_52f8_9652);
    }
}
