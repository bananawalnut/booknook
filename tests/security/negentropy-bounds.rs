//! Regression for the workspace-resolved dependency (RUSTSEC-2026-0365).
//! Compiled only by scripts/test-negentropy-bounds.py in an isolated temporary package.
//! The allocator refuses every allocation/reallocation above 1MiB before System sees it.
//! Exit86 means the allocation guard caught a regression before System allocated it.
//! The vulnerable-version negative control is retained in separate private evidence.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

const MAX_ALLOCATION: usize = 1024 * 1024;
static MAX_OBSERVED: AtomicUsize = AtomicUsize::new(0);
struct Guard;
extern "C" {
    fn write(fd: i32, buf: *const u8, len: usize) -> isize;
    fn _exit(code: i32) -> !;
}
fn inspect(size: usize) {
    MAX_OBSERVED.fetch_max(size, Ordering::Relaxed);
    if size > MAX_ALLOCATION {
        let prefix = b"GUARDED_ALLOCATION requested=";
        let mut digits = [0u8; 20];
        let mut n = size;
        let mut start = digits.len();
        while n != 0 {
            start -= 1;
            digits[start] = b'0' + (n % 10) as u8;
            n /= 10;
        }
        unsafe {
            write(2, prefix.as_ptr(), prefix.len());
            write(2, digits[start..].as_ptr(), digits.len() - start);
            write(2, b"\n".as_ptr(), 1);
            _exit(86)
        }
    }
}
unsafe impl GlobalAlloc for Guard {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        inspect(layout.size());
        System.alloc(layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        inspect(layout.size());
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        inspect(new_size);
        System.realloc(ptr, layout, new_size)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}
#[global_allocator]
static ALLOCATOR: Guard = Guard;
const HOSTILE: [u8; 9] = [0x61, 0, 0, 2, 0xc0, 0x80, 0x80, 0x80, 0];

macro_rules! exercise {
    ($dep:ident, $initiator:expr) => {{
        let mut storage = $dep::NegentropyStorageVector::new();
        storage
            .insert(0, $dep::Id::from_byte_array([0xaa; 32]))
            .unwrap();
        storage.seal().unwrap();
        let mut engine = $dep::Negentropy::borrowed(&storage, 4096).unwrap();
        if $initiator {
            let _ = engine.initiate().unwrap();
            let (mut have, mut need) = (Vec::new(), Vec::new());
            assert_eq!(
                engine.reconcile_with_ids(&HOSTILE, &mut have, &mut need),
                Err($dep::Error::ParseEndsPrematurely)
            );
            assert!(have.is_empty() && need.is_empty());
        } else {
            assert_eq!(
                engine.reconcile(&HOSTILE),
                Err($dep::Error::ParseEndsPrematurely)
            );
            let mut one_short = vec![0x61, 0, 0, 2, 2];
            one_short.extend_from_slice(&[0xbb; 32]);
            assert_eq!(
                engine.reconcile(&one_short),
                Err($dep::Error::ParseEndsPrematurely)
            );
        }
    }};
}
fn main() {
    assert!(
        usize::BITS >= 64,
        "This guarded witness targets the shipped 64-bit platforms"
    );
    let mode = std::env::args().nth(1).expect("responder|initiator");
    match mode.as_str() {
        "responder" => exercise!(negentropy, false),
        "initiator" => exercise!(negentropy, true),
        _ => panic!("Unknown case"),
    }
    println!(
        "RESULT mode={} result=ParseEndsPrematurely max_allocation={} limit={}",
        mode,
        MAX_OBSERVED.load(Ordering::Relaxed),
        MAX_ALLOCATION
    );
}
