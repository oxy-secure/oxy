use crate::core::Oxy;
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std;
use transportation::BufferedTransport;

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

crate fn reexec(args: &[&str]) {
    #[cfg(unix)]
    {
        if is_suid() {
            panic!("Reexec when running suid is potentially unsafe - not implemented yet.");
        }
    }
    let path = std::env::current_exe().unwrap();
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
