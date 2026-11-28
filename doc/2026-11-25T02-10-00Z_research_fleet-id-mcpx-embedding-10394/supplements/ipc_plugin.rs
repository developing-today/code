// cdylib for the dlopen leg of ipc_bench.rs: the same echo the other legs do.
#[unsafe(no_mangle)]
pub extern "C" fn plugin_echo(input: *const u8, len: usize, out: *mut u8) -> usize {
    unsafe { std::ptr::copy_nonoverlapping(input, out, len) };
    len
}
