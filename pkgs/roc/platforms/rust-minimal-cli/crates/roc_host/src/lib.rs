use core::ffi::c_void;
use roc_env::arg::ArgToAndFromHost;
use roc_std::{RocList, RocResult, RocStr};
use tokio::runtime::Runtime;
thread_local! {
   static TOKIO_RUNTIME: Runtime = tokio::runtime::Builder::new_current_thread()
       .enable_io()
       .enable_time()
       .build()
       .unwrap();
}
#[no_mangle]
pub unsafe extern "C" fn roc_alloc(size: usize, _alignment: u32) -> *mut c_void {
    libc::malloc(size)
}
#[no_mangle]
pub unsafe extern "C" fn roc_realloc(
    c_ptr: *mut c_void,
    new_size: usize,
    _old_size: usize,
    _alignment: u32,
) -> *mut c_void {
    libc::realloc(c_ptr, new_size)
}
#[no_mangle]
pub unsafe extern "C" fn roc_dealloc(c_ptr: *mut c_void, _alignment: u32) {
    libc::free(c_ptr)
}
#[no_mangle]
pub unsafe extern "C" fn roc_panic(msg: &RocStr, tag_id: u32) {
    _ = crossterm::terminal::disable_raw_mode();
    match tag_id {
        0 => {
            eprintln!("Roc crashed with:\n\n\t{}\n", msg.as_str());
            print_backtrace();
            std::process::exit(1);
        }
        1 => {
            eprintln!("The program crashed with:\n\n\t{}\n", msg.as_str());
            print_backtrace();
            std::process::exit(1);
        }
        _ => todo!(),
    }
}
#[no_mangle]
pub unsafe extern "C" fn roc_dbg(loc: &RocStr, msg: &RocStr, src: &RocStr) {
    eprintln!("[{}] {} = {}", loc, src, msg);
}
#[repr(C)]
pub struct Variable {
    pub name: RocStr,
    pub value: RocStr,
}
impl roc_std::RocRefcounted for Variable {
    fn inc(&mut self) {
        self.name.inc();
        self.value.inc();
    }
    fn dec(&mut self) {
        self.name.dec();
        self.value.dec();
    }
    fn is_refcounted() -> bool {
        true
    }
}
#[no_mangle]
pub unsafe extern "C" fn roc_expect_failed(
    loc: &RocStr,
    src: &RocStr,
    variables: &RocList<Variable>,
) {
    eprintln!("\nExpectation failed at {}:", loc.as_str());
    eprintln!("\nExpression:\n\t{}\n", src.as_str());
    if !variables.is_empty() {
        eprintln!("With values:");
        for var in variables.iter() {
            eprintln!("\t{} = {}", var.name.as_str(), var.value.as_str());
        }
        eprintln!();
    }
    std::process::exit(1);
}
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn roc_getppid() -> libc::pid_t {
    libc::getppid()
}
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn roc_mmap(
    addr: *mut libc::c_void,
    len: libc::size_t,
    prot: libc::c_int,
    flags: libc::c_int,
    fd: libc::c_int,
    offset: libc::off_t,
) -> *mut libc::c_void {
    libc::mmap(addr, len, prot, flags, fd, offset)
}
#[cfg(unix)]
#[no_mangle]
pub unsafe extern "C" fn roc_shm_open(
    name: *const libc::c_char,
    oflag: libc::c_int,
    mode: libc::mode_t,
) -> libc::c_int {
    libc::shm_open(name, oflag, mode as libc::c_uint)
}
fn print_backtrace() {
    eprintln!("Here is the call stack that led to the crash:\n");
    let mut entries = Vec::new();
    #[derive(Default)]
    struct Entry {
        pub fn_name: String,
        pub filename: Option<String>,
        pub line: Option<u32>,
        pub col: Option<u32>,
    }
    backtrace::trace(|frame| {
        backtrace::resolve_frame(frame, |symbol| {
            if let Some(fn_name) = symbol.name() {
                let fn_name = fn_name.to_string();
                if should_show_in_backtrace(&fn_name) {
                    let mut entry = Entry {
                        fn_name: format_fn_name(&fn_name),
                        ..Default::default()
                    };
                    if let Some(path) = symbol.filename() {
                        entry.filename = Some(path.to_string_lossy().into_owned());
                    };
                    entry.line = symbol.lineno();
                    entry.col = symbol.colno();
                    entries.push(entry);
                }
            } else {
                entries.push(Entry {
                    fn_name: "???".to_string(),
                    ..Default::default()
                });
            }
        });
        true
    });
    for entry in entries {
        eprintln!("\t{}", entry.fn_name);
        if let Some(filename) = entry.filename {
            eprintln!("\t\t{filename}");
        }
    }
    eprintln!("\nOptimizations can make this list inaccurate! If it looks wrong, try running without `--optimize` and with `--linker=legacy`\n");
}
fn should_show_in_backtrace(fn_name: &str) -> bool {
    let is_from_rust = fn_name.contains("::");
    let is_host_fn = fn_name.starts_with("roc_panic")
        || fn_name.starts_with("_roc__")
        || fn_name.starts_with("rust_main")
        || fn_name == "_main";
    !is_from_rust && !is_host_fn
}
fn format_fn_name(fn_name: &str) -> String {
    let mut pieces_iter = fn_name.split('_');
    if let (_, Some(module_name), Some(name)) =
        (pieces_iter.next(), pieces_iter.next(), pieces_iter.next())
    {
        display_roc_fn(module_name, name)
    } else {
        "???".to_string()
    }
}
fn display_roc_fn(module_name: &str, fn_name: &str) -> String {
    let module_name = if module_name == "#UserApp" {
        "app"
    } else {
        module_name
    };
    let fn_name = if fn_name.parse::<u64>().is_ok() {
        "(anonymous function)"
    } else {
        fn_name
    };
    format!("\u{001B}[36m{module_name}\u{001B}[39m.{fn_name}")
}
#[no_mangle]
pub unsafe extern "C" fn roc_memset(dst: *mut c_void, c: i32, n: usize) -> *mut c_void {
    libc::memset(dst, c, n)
}
pub fn init() {
    let funcs: &[*const extern "C" fn()] = &[
        roc_alloc as _,
        roc_realloc as _,
        roc_dealloc as _,
        roc_panic as _,
        roc_dbg as _,
        roc_memset as _,
        roc_fx_stdout_line as _,
        roc_fx_hello as _,
    ];
    #[allow(forgetting_references)]
    std::mem::forget(std::hint::black_box(funcs));
    if cfg!(unix) {
        let unix_funcs: &[*const extern "C" fn()] =
            &[roc_getppid as _, roc_mmap as _, roc_shm_open as _];
        #[allow(forgetting_references)]
        std::mem::forget(std::hint::black_box(unix_funcs));
    }
}
#[no_mangle]
pub extern "C" fn rust_main(args: RocList<ArgToAndFromHost>) -> i32 {
    init();
    extern "C" {
        #[link_name = "roc__main_for_host_1_exposed_generic"]
        pub fn roc_main_for_host_caller(
            exit_code: &mut i32,
            args: *const RocList<ArgToAndFromHost>,
        );
        #[link_name = "roc__main_for_host_1_exposed_size"]
        pub fn roc_main__for_host_size() -> usize;
    }
    let exit_code: i32 = unsafe {
        let mut exit_code: i32 = -1;
        let args = args;
        roc_main_for_host_caller(&mut exit_code, &args);
        debug_assert_eq!(std::mem::size_of_val(&exit_code), roc_main__for_host_size());
        std::mem::forget(args);
        exit_code
    };
    exit_code
}
#[no_mangle]
pub extern "C" fn roc_fx_stdout_line(line: &RocStr) -> RocResult<(), roc_io_error::IOErr> {
    roc_stdio::stdout_line(line)
}
#[no_mangle]
pub extern "C" fn roc_fx_hello(name: &RocStr) -> RocResult<RocStr, roc_io_error::IOErr> {
    roc_stdio::hello(name)
}
