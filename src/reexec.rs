use crate::core::Oxy;

use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use transportation::BufferedTransport;

lazy_static! {
    static ref CURRENT_EXE: ::std::path::PathBuf = ::std::env::current_exe().unwrap();
}

crate fn run() {
    #[cfg(unix)]
    {
        use std::os::unix::io::RawFd;
        let fd = crate::arg::matches().value_of("fd").unwrap().parse().unwrap();
        debug!("Reexec using fd {}", fd);
        let bt = <BufferedTransport as From<RawFd>>::from(fd);
        Oxy::run(bt);
    }
    #[cfg(windows)]
    unimplemented!();
}

#[cfg(unix)]
crate fn is_suid() -> bool {
    let uid = ::nix::unistd::getuid();
    let euid = ::nix::unistd::geteuid();
    euid != uid
}

crate fn safety_check() {
    if crate::arg::mode() == "server" {
        safety_check_hard();
    }
}

crate fn safety_check_hard() {
    let path = CURRENT_EXE.clone();
    if crate::arg::matches().is_present("unsafe reexec") {
        warn!("Using --unsafe-reexec");
        return;
    }
    if !(path.starts_with(::std::env::home_dir().unwrap()) || path.starts_with("/usr")) {
        error!("Re-execution can lead to privilege escalation if another user can write to the executable path. Oxy detected that it is running from an uncommon location which makes it more likely this might apply. If you are certain no other user can hijack the path {:?}, run again with --unsafe-rexec", path);
        ::std::process::exit(1);
    }
}

crate fn reexec(args: &[&str]) {
    // SECURITYWATCH: We shouldn't reexec if another non-root user has write
    // permission on our binary or any parent folder. This is an out-and-out vuln
    // if somebody puts oxy in /tmp or something. It should be fine as long as
    // we're in /home/user/.bin/oxy or /usr/bin/local/oxy or whatever, but...
    // additional controls need to be added.
    safety_check_hard();
    #[cfg(unix)]
    {
        if is_suid() {
            panic!("Reexec when running suid is potentially unsafe - not implemented yet.");
        }
    }
    let path = CURRENT_EXE.clone();
    #[cfg(unix)]
    {
        use nix::unistd::{execv, fork, ForkResult::*};
        use std::ffi::CString;

        let mut args2: Vec<CString> = args.iter().map(|x| CString::new(x.as_bytes()).unwrap()).collect();
        let path = CString::new(path.as_os_str().to_str().unwrap().as_bytes()).unwrap();
        args2.insert(0, path.clone());
        match fork() {
            Ok(Parent { .. }) => {
                return;
            }
            Ok(Child) => {
                execv(&path, &args2).unwrap();
                unreachable!();
            }
            Err(_) => {
                panic!("Fork failed");
            }
        }
    }
}
