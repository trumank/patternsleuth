//! Runtime-generated native call stubs (x86-64).
//!
//! The engine dispatches a hooked or delegate-bound `UFunction` by calling its
//! `UFunction::Func` pointer with the fixed native signature
//! `fn(*mut UObject, *mut FFrame, *mut c_void)`. A single shared Rust trampoline
//! installed as that `Func` can't tell *which* binding it is serving: the call
//! frame doesn't reliably carry our function's identity (a BP-script function
//! called from bytecode arrives with the *caller's* frame, not ours). The old
//! design worked around this with a fixed array of `fn tramp<const SLOT>()`
//! monomorphisations — one distinct function pointer per slot — which is exactly
//! what capped the number of live bindings at a compile-time constant.
//!
//! Instead we emit one tiny stub per binding at runtime. Each stub loads a
//! per-binding *cookie* (a stable `*const SinkHeader`, opaque to this module)
//! into the 4th integer-argument register and tail-jumps to a single shared
//! dispatcher `fn(context, stack, result, cookie)`. The dispatcher recovers the
//! binding from the cookie. That removes the cap entirely and lets hooks and
//! delegates share one dispatch path.
//!
//! Only the game/interpreter thread allocates stubs; the emitted stubs
//! themselves run on whatever thread the engine calls them from. Stubs are
//! never freed or reused — see [`StubPool`].

use std::ffi::c_void;

use windows::Win32::System::Memory::{
    MEM_COMMIT, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc,
};

/// Bytes in one emitted stub:
/// ```text
///   49 B9 <imm64>   mov r9,  cookie    ; Win64 4th arg (REX 0x48 -> rcx on SysV)
///   48 B8 <imm64>   mov rax, dispatch
///   FF E0           jmp rax
/// ```
const STUB_LEN: usize = 22;
/// Padded slot size so stubs stay 16-aligned with a little headroom.
const STUB_STRIDE: usize = 32;
const PAGE: usize = 4096;

// The Win64 ABI passes the 4th integer argument in R9, System V in RCX. The two
// `mov reg, imm64` encodings differ only in the REX prefix. The DLL is only ever
// built for x86-64 Windows, but supporting both keeps host-target checks honest.
#[cfg(target_os = "windows")]
const MOV_ARG4_REX: u8 = 0x49; // REX.WB -> r9
#[cfg(not(target_os = "windows"))]
const MOV_ARG4_REX: u8 = 0x48; // REX.W  -> rcx

/// A bump allocator over executable pages. Stubs are **never freed or
/// reused**: restoring the target's `Func` makes a stub unreachable for *new*
/// calls, but a thread that loaded the old `Func` pointer just before the
/// restore can still enter the stub arbitrarily late — rewriting the slot for
/// a new binding would put torn code under a running thread. Like the leaked
/// `SinkHeader`s, a retired stub (32 bytes) is simply abandoned in place, and
/// its baked-in cookie keeps pointing at a header whose fallthrough stays
/// valid forever.
///
/// Not `Send`/`Sync` (it holds raw pointers and is only touched by the game
/// thread). The *emitted* stubs run on any thread.
pub struct StubPool {
    pages: Vec<*mut u8>,
    cursor: *mut u8,
    left: usize,
}

impl StubPool {
    pub const fn new() -> Self {
        Self {
            pages: Vec::new(),
            cursor: std::ptr::null_mut(),
            left: 0,
        }
    }

    fn grow(&mut self) {
        let p =
            unsafe { VirtualAlloc(None, PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) }
                as *mut u8;
        assert!(!p.is_null(), "VirtualAlloc failed for trampoline page");
        self.pages.push(p);
        self.cursor = p;
        self.left = PAGE / STUB_STRIDE;
    }

    /// Emit a stub that tail-jumps to `dispatch` carrying `cookie`, and return
    /// its address. Install the returned address as the target `UFunction::Func`.
    pub fn alloc(&mut self, cookie: usize, dispatch: usize) -> *mut u8 {
        if self.left == 0 {
            self.grow();
        }
        let slot = self.cursor;
        self.cursor = unsafe { self.cursor.add(STUB_STRIDE) };
        self.left -= 1;
        write_stub(slot, cookie, dispatch);
        slot
    }
}

fn write_stub(dst: *mut u8, cookie: usize, dispatch: usize) {
    let mut b = [0u8; STUB_LEN];
    b[0] = MOV_ARG4_REX;
    b[1] = 0xB9;
    b[2..10].copy_from_slice(&(cookie as u64).to_le_bytes());
    b[10] = 0x48; // REX.W
    b[11] = 0xB8; // mov rax, imm64
    b[12..20].copy_from_slice(&(dispatch as u64).to_le_bytes());
    b[20] = 0xFF; // jmp rax
    b[21] = 0xE0;
    unsafe {
        std::ptr::copy_nonoverlapping(b.as_ptr(), dst, STUB_LEN);
    }
    flush_icache(dst, STUB_LEN);
}

/// Serialize the freshly-written code against the instruction cache. x86-64
/// keeps I-cache coherent with stores on the same core, but a stub written on
/// the game thread may first execute on a different core; the flush guarantees
/// the new bytes are seen there.
fn flush_icache(addr: *mut u8, len: usize) {
    use windows::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let _ = FlushInstructionCache(GetCurrentProcess(), Some(addr as *const c_void), len);
    }
}
