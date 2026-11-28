//! Probe for lints added in Rust 1.98 and 1.99. Not product code.
pub fn assert_empty(v: &Vec<u8>) {
    assert!(v.is_empty());
}
pub fn zero_capacity() -> Vec<u8> {
    Vec::with_capacity(0)
}
pub fn chunked(xs: &[u8]) -> usize {
    xs.chunks_exact(4).count()
}
pub fn bit_width() -> u32 {
    (std::mem::size_of::<u32>() * 8) as u32
}
pub fn unbounded(mut n: u32) -> u32 {
    for i in 0.. {
        if i > n {
            break;
        }
        n += 1;
    }
    n
}
pub fn unchecked(o: Option<u8>) -> u8 {
    if o.is_some() {
        unsafe { o.unwrap_unchecked() }
    } else {
        0
    }
}
pub trait Job {
    fn run(&self) -> impl std::future::Future<Output = u8>;
}
pub struct J;
impl Job for J {
    async fn run(&self) -> u8 {
        1
    }
}
