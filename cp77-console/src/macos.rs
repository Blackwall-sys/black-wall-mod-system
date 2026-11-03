//! Chamadas do macOS que mais de um módulo usa, declaradas uma vez: dyld, dl, runtime Objective-C, mach_vm e sysctl.
//! Uma declaração por símbolo: duas declarações do mesmo símbolo com tipos diferentes compilam e viram comportamento
//! indefinido. Símbolo que só um módulo usa fica declarado nele.

use std::os::raw::{c_char, c_int, c_void};

/// `id`, `SEL`, `Class`, `Method` e `IMP` do runtime Objective-C.
pub(crate) type Id = *mut c_void;
pub(crate) type Sel = *const c_void;
pub(crate) type Class = *mut c_void;
pub(crate) type Method = *mut c_void;
pub(crate) type Imp = *const c_void;

pub(crate) type KernReturn = i32;
pub(crate) type MachPort = u32;
pub(crate) const KERN_SUCCESS: KernReturn = 0;
pub(crate) const VM_PROT_READ: i32 = 1;
pub(crate) const VM_PROT_WRITE: i32 = 2;
pub(crate) const VM_PROT_EXECUTE: i32 = 4;
pub(crate) const VM_PROT_COPY: i32 = 0x10;

/// `RTLD_DEFAULT` do macOS: procura o símbolo em todas as imagens carregadas.
pub(crate) const RTLD_DEFAULT: *mut c_void = -2isize as *mut c_void;

/// `Dl_info` do `dladdr`.
#[repr(C)]
pub(crate) struct DlInfo {
    pub(crate) dli_fname: *const c_char,
    pub(crate) dli_fbase: *mut c_void,
    pub(crate) dli_sname: *const c_char,
    pub(crate) dli_saddr: *mut c_void,
}

extern "C" {
    pub(crate) fn _dyld_image_count() -> u32;
    pub(crate) fn _dyld_get_image_header(image_index: u32) -> *const c_void;
    pub(crate) fn _dyld_get_image_name(image_index: u32) -> *const c_char;

    pub(crate) fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    pub(crate) fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    pub(crate) fn dlerror() -> *const c_char;
    pub(crate) fn dladdr(addr: *const c_void, info: *mut DlInfo) -> c_int;

    pub(crate) fn objc_msgSend();
    pub(crate) fn objc_getClass(name: *const c_char) -> Class;
    #[cfg(feature = "capture")]
    pub(crate) fn objc_copyClassList(out_count: *mut u32) -> *mut Class;
    pub(crate) fn object_getClass(obj: Id) -> Class;
    pub(crate) fn class_getName(cls: Class) -> *const c_char;
    pub(crate) fn sel_registerName(name: *const c_char) -> Sel;
    pub(crate) fn class_getInstanceMethod(cls: Class, sel: Sel) -> Method;
    pub(crate) fn method_getImplementation(m: Method) -> Imp;
    pub(crate) fn method_setImplementation(m: Method, imp: Imp) -> Imp;

    pub(crate) static mach_task_self_: MachPort;
    pub(crate) fn mach_vm_read_overwrite(task: MachPort, address: u64, size: u64, data: u64, out_size: *mut u64) -> KernReturn;
    pub(crate) fn mach_vm_protect(task: MachPort, address: u64, size: u64, set_max: i32, new_prot: i32) -> KernReturn;
    pub(crate) fn mach_vm_allocate(target_task: MachPort, address: *mut u64, size: u64, flags: i32) -> KernReturn;

    pub(crate) fn sysctlbyname(name: *const c_char, oldp: *mut c_void, oldlenp: *mut usize, newp: *mut c_void, newlen: usize) -> c_int;
}

/// Seletor ObjC resolvido uma vez por ponto de chamada. Montar `CString` + `sel_registerName` a cada
/// uso custava 29–37 ns (bench `rustc -O`, 2 M iterações) e roda por frame no `desenhar_terminal` e por
/// evento de input no `my_sendevent`; guardado, é um load de 0,2 ns. Corrida no primeiro uso só
/// resolve duas vezes: o runtime devolve o mesmo ponteiro para o mesmo nome.
macro_rules! sel {
    ($nome:literal) => {{
        static SEL: ::std::sync::atomic::AtomicUsize = ::std::sync::atomic::AtomicUsize::new(0);
        let s = SEL.load(::std::sync::atomic::Ordering::Relaxed);
        if s != 0 {
            s as *const ::std::ffi::c_void
        } else {
            let r = $crate::macos::seletor_do_runtime(concat!($nome, "\0").as_ptr());
            SEL.store(r as usize, ::std::sync::atomic::Ordering::Relaxed);
            r
        }
    }};
}
pub(crate) use sel;

pub(crate) unsafe fn seletor_do_runtime(nome_terminado_em_nul: *const u8) -> Sel {
    sel_registerName(nome_terminado_em_nul as *const c_char)
}

/// Classe ObjC resolvida uma vez por ponto de chamada, pelo mesmo motivo do `sel!`: o render pede `MTLRenderPassDescriptor` todo
/// quadro. Classe ainda não carregada (nil) não fica guardada e é procurada de novo na próxima chamada.
macro_rules! classe {
    ($nome:literal) => {{
        static CLASSE: ::std::sync::atomic::AtomicUsize = ::std::sync::atomic::AtomicUsize::new(0);
        let c = CLASSE.load(::std::sync::atomic::Ordering::Relaxed);
        if c != 0 {
            c as $crate::macos::Id
        } else {
            let r = $crate::macos::objc_getClass(concat!($nome, "\0").as_ptr() as *const ::std::os::raw::c_char);
            CLASSE.store(r as usize, ::std::sync::atomic::Ordering::Relaxed);
            r
        }
    }};
}
pub(crate) use classe;

/// `objc_msgSend` com a assinatura do método escrita na chamada: `msg!(recv, "a:b:", T = x, U = y => R)`, sem `=> R` para método
/// `void`. É o `[recv a:x b:y]` do Objective-C: a ABI C do arm64 decide como cada tipo viaja (struct de mais de 16 bytes que não é
/// só de float, como `MTLRegion`, vai por ponteiro para uma cópia), e o compilador aplica isso pelo tipo declarado aqui. O tipo
/// tem que ser o do header do método: `NSUInteger` é `u64`, `BOOL` é `bool`, objeto é `Id`.
macro_rules! msg {
    ($recv:expr, $nome:literal $(, $tipo:ty = $arg:expr)* $(=> $ret:ty)?) => {{
        let f: extern "C" fn($crate::macos::Id, $crate::macos::Sel $(, $tipo)*) $(-> $ret)? =
            ::std::mem::transmute($crate::macos::objc_msgSend as *const ::std::ffi::c_void);
        f($recv, $crate::macos::sel!($nome) $(, $arg)*)
    }};
}
pub(crate) use msg;

#[inline]
pub(crate) unsafe fn msg0(recv: Id, s: Sel) -> Id {
    let f: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}
#[inline]
pub(crate) unsafe fn msg_usize(recv: Id, s: Sel) -> usize {
    let f: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const c_void);
    f(recv, s)
}

/// Objeto ObjC com uma referência que é nossa (+1, de `new`, `new…` ou `alloc`/`init`): o `release` sai no drop. Nil é aceito,
/// porque mensagem a nil não faz nada.
pub(crate) struct Retido(pub(crate) Id);

impl Drop for Retido {
    fn drop(&mut self) {
        unsafe { msg!(self.0, "release") }
    }
}
