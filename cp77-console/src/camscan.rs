//! Scanner NATIVO da camera do render (mach_vm, Rust puro, in-process). Substitui o Memory.scan
//! LENTO do Frida (que travava). Le a memoria do processo em chunks via mach_vm_read_overwrite
//! (SEGURO: retorna erro em pagina invalida, nao falta), acha matrizes inverse-view (cam->world)
//! cujo col3 ~ a posicao-alvo da camera, e patcha col3 += right*eye_d (COW via mach_vm_protect).
//! Objetivo: teste DEFINITIVO se ALGUMA copia CPU-acessivel da camera move o render (o patch do
//! bound buffer via Frida dava zero — mas o Frida nunca completou o scan da memoria toda).

use std::sync::Mutex;

type KernReturn = i32;
type MachPort = u32;
const KERN_SUCCESS: KernReturn = 0;
const VM_PROT_READ: i32 = 1;
const VM_PROT_WRITE: i32 = 2;
const VM_PROT_COPY: i32 = 0x10;

extern "C" {
    static mach_task_self_: MachPort;
    fn mach_vm_read_overwrite(task: MachPort, address: u64, size: u64, data: u64, out_size: *mut u64) -> KernReturn;
    fn mach_vm_protect(task: MachPort, address: u64, size: u64, set_max: i32, new_prot: i32) -> KernReturn;
    fn mach_vm_region_recurse(
        task: MachPort,
        address: *mut u64,
        size: *mut u64,
        nesting_depth: *mut u32,
        info: *mut u32,
        count: *mut u32,
    ) -> KernReturn;
}

// enderecos (inicio da matriz de 64 bytes) de TODAS as copias da camera achadas na memoria.
static CACHE: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// inverse-view rigida (cam->world) cujo col3 ~ (tx,ty,tz). Devolve col0 (right) se casar.
fn strict_invview(m: &[f32; 16], tx: f32, ty: f32, tz: f32) -> Option<[f32; 3]> {
    for v in m {
        if !v.is_finite() {
            return None;
        }
    }
    if m[3].abs() > 1e-3 || m[7].abs() > 1e-3 || m[11].abs() > 1e-3 {
        return None;
    }
    if (m[15] - 1.0).abs() > 1e-3 {
        return None;
    }
    let c0 = [m[0], m[1], m[2]];
    let c1 = [m[4], m[5], m[6]];
    let c2 = [m[8], m[9], m[10]];
    let c3 = [m[12], m[13], m[14]];
    // col3 perto do alvo (tolera bob ate 6m) — filtro forte, rejeita a maioria cedo
    if (c3[0] - tx).abs() > 6.0 || (c3[1] - ty).abs() > 6.0 || (c3[2] - tz).abs() > 6.0 {
        return None;
    }
    let len = |a: &[f32; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    if (len(&c0) - 1.0).abs() > 0.02 || (len(&c1) - 1.0).abs() > 0.02 || (len(&c2) - 1.0).abs() > 0.02 {
        return None;
    }
    let dot = |a: &[f32; 3], b: &[f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    if dot(&c0, &c1).abs() > 0.02 || dot(&c0, &c2).abs() > 0.02 || dot(&c1, &c2).abs() > 0.02 {
        return None;
    }
    let cx = c1[1] * c2[2] - c1[2] * c2[1];
    let cy = c1[2] * c2[0] - c1[0] * c2[2];
    let cz = c1[0] * c2[1] - c1[1] * c2[0];
    let det = c0[0] * cx + c0[1] * cy + c0[2] * cz;
    if (det.abs() - 1.0).abs() > 0.05 {
        return None;
    }
    Some(c0)
}

/// regioes rw mapeadas do processo (leaf, via recurse). Cap de tamanho pra evitar arenas gigantes.
unsafe fn rw_regions() -> Vec<(u64, u64)> {
    let mut out = Vec::new();
    let mut addr: u64 = 1;
    let mut guard = 0u32;
    loop {
        guard += 1;
        if guard > 400_000 {
            break;
        }
        let mut size: u64 = 0;
        let mut depth: u32 = 1000;
        let mut info = [0u32; 32];
        let mut count: u32 = 19; // VM_REGION_SUBMAP_INFO_COUNT_64
        let kr = mach_vm_region_recurse(mach_task_self_, &mut addr, &mut size, &mut depth, info.as_mut_ptr(), &mut count);
        if kr != KERN_SUCCESS {
            break;
        }
        let prot = info[0] as i32; // protection @ offset 0
        if prot & VM_PROT_READ != 0 && prot & VM_PROT_WRITE != 0 && size >= 4096 && size <= 768 * 1024 * 1024 {
            out.push((addr, size));
        }
        addr = addr.wrapping_add(size);
        if addr == 0 {
            break;
        }
    }
    out
}

/// patcha col3 += right*eye_d na matriz em `addr` (COW a pagina, escreve direto — UMA reflete na GPU).
unsafe fn patch(addr: u64, right: [f32; 3], pos: [f32; 3], eye_d: f32) -> bool {
    let page = addr & !0xFFF;
    let end = (addr + 64 + 0xFFF) & !0xFFF;
    let span = end - page;
    if mach_vm_protect(mach_task_self_, page, span, 0, VM_PROT_READ | VM_PROT_WRITE | VM_PROT_COPY) != KERN_SUCCESS
        && mach_vm_protect(mach_task_self_, page, span, 0, VM_PROT_READ | VM_PROT_WRITE) != KERN_SUCCESS
    {
        return false;
    }
    let p = addr as *mut f32;
    core::ptr::write_unaligned(p.add(12), pos[0] + right[0] * eye_d);
    core::ptr::write_unaligned(p.add(13), pos[1] + right[1] * eye_d);
    core::ptr::write_unaligned(p.add(14), pos[2] + right[2] * eye_d);
    true
}

/// scan-once (popula cache) + patcha TODAS as copias cacheadas. Retorna nº patchado neste frame.
/// A redscript chama por frame com a pos VIVA da camera (tolera bob: alvo acompanha, cache re-verifica).
pub unsafe fn cam_scan(tx: f32, ty: f32, tz: f32, eye_d: f32) -> i32 {
    // GUARDA: a camera real esta a MILHARES de unidades da origem (Night City). Alvo perto da origem =
    // pos local/bugada -> escanearia matrizes de MODELO perto da origem e patcharia a cena toda (crash).
    if tx.abs() < 50.0 && tz.abs() < 50.0 {
        return 0;
    }
    let mut cache = CACHE.lock().unwrap();
    if cache.is_empty() {
        let mut buf = vec![0u8; 4 * 1024 * 1024]; // chunk 4MB
        let regions = rw_regions();
        let mut scanned_mb = 0u64;
        'outer: for (base, size) in regions {
            let mut off: u64 = 0;
            while off < size {
                let n = ((size - off) as usize).min(buf.len());
                let mut outsz: u64 = 0;
                if mach_vm_read_overwrite(mach_task_self_, base + off, n as u64, buf.as_mut_ptr() as u64, &mut outsz)
                    != KERN_SUCCESS
                {
                    break;
                }
                let got = outsz as usize;
                scanned_mb += (got as u64) / (1024 * 1024);
                let nf = got / 4;
                let f = buf.as_ptr() as *const f32;
                let mut i = 0usize;
                while i + 16 <= nf {
                    let mut m = [0f32; 16];
                    for j in 0..16 {
                        m[j] = core::ptr::read_unaligned(f.add(i + j));
                    }
                    if strict_invview(&m, tx, ty, tz).is_some() {
                        cache.push(base + off + (i as u64) * 4);
                        if cache.len() >= 1024 {
                            break 'outer;
                        }
                    }
                    i += 4;
                }
                if got == 0 {
                    break;
                }
                off += got as u64;
            }
        }
        crate::log(&format!(
            "[camscan] DESCOBERTA: {} copias da camera na memoria (scan {}MB) alvo=[{:.0},{:.0},{:.0}]",
            cache.len(),
            scanned_mb,
            tx,
            ty,
            tz
        ));
    }
    // patcha cada copia cacheada: re-le, confirma que ainda e a camera (near alvo), patcha col3.
    let mut np = 0i32;
    for &addr in cache.iter() {
        let mut m = [0f32; 16];
        let mut outsz: u64 = 0;
        if mach_vm_read_overwrite(mach_task_self_, addr, 64, m.as_mut_ptr() as u64, &mut outsz) != KERN_SUCCESS {
            continue;
        }
        if let Some(right) = strict_invview(&m, tx, ty, tz) {
            let pos = [m[12], m[13], m[14]];
            if patch(addr, right, pos, eye_d) {
                np += 1;
            }
        }
    }
    np
}

/// limpa o cache (forca re-descoberta no proximo cam_scan) — chamado ao desarmar.
pub fn clear_cache() {
    if let Ok(mut c) = CACHE.lock() {
        c.clear();
    }
}
