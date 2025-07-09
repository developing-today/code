//! This crate provides common functionality common functionality for Roc to interface with `std::env`
pub mod arg;

use roc_std::{roc_refcounted_noop_impl, RocList, RocRefcounted, RocResult, RocStr};
use std::borrow::Borrow;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn env_dict() -> RocList<(RocStr, RocStr)> {
    // TODO: can we be more efficient about reusing the String's memory for RocStr?
    std::env::vars_os()
        .map(|(key, val)| {
            (
                RocStr::from(key.to_string_lossy().borrow()),
                RocStr::from(val.to_string_lossy().borrow()),
            )
        })
        .collect()
}

pub fn temp_dir() -> RocList<u8> {
    let path_os_string_bytes = std::env::temp_dir().into_os_string().into_encoded_bytes();

    RocList::from(path_os_string_bytes.as_slice())
}

pub fn env_var(roc_str: &RocStr) -> RocResult<RocStr, ()> {
    // TODO: can we be more efficient about reusing the String's memory for RocStr?
    match std::env::var_os(roc_str.as_str()) {
        Some(os_str) => RocResult::ok(RocStr::from(os_str.to_string_lossy().borrow())),
        None => RocResult::err(()),
    }
}

pub fn get_locale() -> RocResult<RocStr, ()> {
    sys_locale::get_locale().map_or_else(
        || RocResult::err(()),
        |locale| RocResult::ok(locale.to_string().as_str().into()),
    )
}

pub fn get_locales() -> RocList<RocStr> {
    const DEFAULT_MAX_LOCALES: usize = 10;
    let locales = sys_locale::get_locales();
    let mut roc_locales = RocList::with_capacity(DEFAULT_MAX_LOCALES);
    for l in locales {
        roc_locales.push(l.to_string().as_str().into());
    }
    roc_locales
}

#[derive(Debug)]
#[repr(C)]
pub struct ReturnArchOS {
    pub arch: RocStr,
    pub os: RocStr,
}

roc_refcounted_noop_impl!(ReturnArchOS);

pub fn current_arch_os() -> ReturnArchOS {
    ReturnArchOS {
        arch: std::env::consts::ARCH.into(),
        os: std::env::consts::OS.into(),
    }
}

pub fn posix_time() -> roc_std::U128 {
    // TODO in future may be able to avoid this panic by using C APIs
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards");

    roc_std::U128::from(since_epoch.as_nanos())
}

pub fn sleep_millis(milliseconds: u64) {
    let duration = Duration::from_millis(milliseconds);
    std::thread::sleep(duration);
}
