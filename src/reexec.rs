use crate::core::Oxy;

use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use transportation::BufferedTransport;

crate fn reexec(args: &[&str]) {
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
                unimplemented!("In progress...");
            }
            Err(_) => {
                panic!("Fork failed");
            }
        }
    }
}
