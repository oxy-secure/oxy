use libc;
use std::ffi::{CStr, CString};

pub(crate) fn format_throughput(bytes: u64, seconds: u64) -> String {
    let seconds = if seconds != 0 { seconds } else { 1 };
    let mut throughput = bytes / seconds;
    let mut throughput_decimal = (bytes * 10) / seconds;
    let mut unit = "B/s";

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "KiB/s";
    }

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "MiB/s";
    }

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "GiB/s";
    }
    format!("{}.{} {}", throughput, throughput_decimal % 10, unit)
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    let mut bytes_decimal = (bytes * 10) / 1024;
    let mut bytes = bytes / 1024;
    let mut unit = "KiB";

    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "MiB"
    }
    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "GiB"
    }
    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "TiB"
    }
    format!("{}.{} {}", bytes, bytes_decimal % 10, unit)
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Pwent {
    pub(crate) name:  String,
    pub(crate) uid:   u32,
    pub(crate) gid:   u32,
    pub(crate) home:  String,
    pub(crate) shell: String,
}

#[cfg(unix)]
fn extract_pwent(raw: *mut ::libc::passwd) -> Result<Pwent, ()> {
    if raw as usize == 0 {
        return Err(());
    }

    let mut result = Pwent::default();
    result.name = unsafe { CStr::from_ptr((*raw).pw_name).to_string_lossy().into_owned() };
    result.uid = unsafe { (*raw).pw_uid };
    result.gid = unsafe { (*raw).pw_gid };
    result.shell = unsafe { CStr::from_ptr((*raw).pw_shell).to_string_lossy().into_owned() };
    result.home = unsafe { CStr::from_ptr((*raw).pw_dir).to_string_lossy().into_owned() };

    Ok(result)
}

#[cfg(unix)]
pub(crate) fn getpwnam(name: &str) -> Result<Pwent, ()> {
    let name = CString::new(name).map_err(|_| ())?;
    let raw = unsafe { libc::getpwnam(name.as_ptr() as *const _) };
    extract_pwent(raw)
}

#[cfg(unix)]
pub(crate) fn getpwuid(uid: u32) -> Result<Pwent, ()> {
    let raw = unsafe { libc::getpwuid(uid) };
    extract_pwent(raw)
}

#[cfg(unix)]
pub(crate) fn current_user_pw() -> Result<Pwent, ()> {
    let uid = unsafe { libc::getuid() };
    getpwuid(uid)
}
