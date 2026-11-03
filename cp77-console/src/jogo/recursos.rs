//! Recursos do jogo: o carregador, a checagem de existência e a leitura de buffer.

use crate::arquivos_do_jogo::RESREF_LOADASYNC_VM;
use crate::diagnostico::RING_FINDTOKEN;
use std::ffi::c_void;
pub(crate) const LEITOR_BUFFER_VM: u64 = 0x1_0107_4b44;

pub(crate) const CHECAR_RECURSO_VM: u64 = 0x1_0151_aa2c;

/// `FindToken(path)` DE VERDADE: devolve `Some(sharedptr)` quando o recurso tem token vivo.
///
/// É a 1ª chamada real a um método do `ResourceLoader` neste projeto — o passo que faltava pros
/// itens `#406`/`#408` saírem de "endereço identificado" pra "API usável".
/// O `ResourceLoader` num GLOBAL — achado em 2026-09-07 lendo os 3 chamadores de
/// `LoadAsync(const ResourceRequest&)`. Os três fazem, idêntico:
///     adrp x8, #0x1099dc000 ; add x8, x8, #0x580 ; ldr x0, [x8]
/// Três call-sites independentes, em regiões diferentes do binário, apontando pro MESMO endereço.
/// Isso dispensa o ring do probe pra obter o `this` — a API fica chamável sem hook nenhum armado.
pub(crate) const CARREGADOR_VM: u64 = 0x1_099d_c690;

/// `ResourceSerializer::OnDependenciesReady`: roda quando as dependências de um recurso ficaram prontas e agenda os jobs de `PostLoad`.
/// Um gancho só, que serve o `resource.patch` e o `Resource/Load` (ver `recursos_de_mod::garantir_gancho_de_dependencias`).
pub(crate) const DEPENDENCIAS_PRONTAS_VM: u64 = 0x1_0120_90b0;

/// Primeira instrução esperada no alvo (`sub sp, sp, #0xa0`); outra coisa no lugar = build diferente, e o gancho não entra.
pub(crate) const DEPENDENCIAS_PRONTAS_PROLOGO: u32 = 0xd102_83ff;

/// Hash do caminho do recurso (`+0x30` do `CResource`, o campo que a 51e casa no mundo); 0 se ilegível.
pub(crate) unsafe fn hash_do_caminho_do_recurso(recurso: u64) -> u64 {
    if recurso != 0 && crate::memoria::is_readable((recurso + 0x30) as *const c_void, 8) { ((recurso + 0x30) as *const u64).read_unaligned() } else { 0 }
}

pub(crate) type PostLoadFn = unsafe extern "C" fn(*mut c_void, u64, u64);

/// O ponteiro do loader: primeiro o global (não precisa de probe), depois o ring como reserva.
pub(crate) unsafe fn resloader_this() -> u64 {
    let slot = crate::rebase(crate::jogo::recursos::CARREGADOR_VM);
    if !slot.is_null() && crate::memoria::is_readable(slot, 8) {
        let p = (slot as *const u64).read_unaligned();
        if p != 0 && crate::memoria::is_readable(p as *const c_void, 0x60) {
            return p;
        }
    }
    RING_FINDTOKEN.ultimo_this
}

/// `ResourceAsyncReference::LoadAsync` do motor (`0x1010f0d34`) pro hash de path: o `SharedPtr` do token por valor
/// (`[instância, bloco de contagem]`, dono dessa referência) ou `None` sem loader. Não espera o job — ver `esperar_token`.
/// Token (2026-09-11): `+0x28` Handle do recurso, `+0x48` entrada do path, `+0x50` job, `+0x58` terminou, `+0x5c` erro.
pub(crate) unsafe fn token_do_path(path_hash: u64) -> Option<[u64; 2]> {
    if resloader_this() == 0 {
        return None;
    }
    let referencia: u64 = path_hash;
    let mut token = [0u64; 2];
    core::arch::asm!(
        "blr {f}",
        f = in(reg) crate::rebase(RESREF_LOADASYNC_VM) as u64,
        in("x0") &referencia as *const u64 as u64,
        in("x1") 0u64,
        in("x8") token.as_mut_ptr() as u64,
        clobber_abi("C"),
    );
    Some(token)
}

