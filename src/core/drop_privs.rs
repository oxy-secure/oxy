use crate::core::Oxy;
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::ffi::CString;

impl Oxy {
    crate fn drop_privs(&self) {
        #[cfg(unix)]
        {
            for (k, _) in ::std::env::vars() {
                if ["LANG", "SHELL", "HOME", "TERM", "USER", "RUST_BACKTRACE", "RUST_LOG", "PATH"].contains(&k.as_str()) {
                    continue;
                }
                ::std::env::remove_var(&k);
            }
        }
        let peer = self.internal.peer_name.borrow().clone();
        if let Some(peer) = peer {
            let setuser = crate::conf::get_setuser(&peer);
            if let Some(setuser) = setuser {
                #[cfg(not(unix))]
                unimplemented!();
                #[cfg(unix)]
                {
                    info!("Setting user: {}", setuser);
                    let pwent = crate::util::getpwnam(&setuser);
                    if pwent.is_err() {
                        error!("Failed to gather user information for {}", setuser);
                        ::std::process::exit(1);
                    }
                    let pwent = pwent.unwrap();
                    ::std::env::set_var("HOME", &pwent.home);
                    ::std::env::set_var("SHELL", &pwent.shell);
                    ::std::env::set_var("USER", &pwent.name);
                    let gid = ::nix::unistd::Gid::from_raw(pwent.gid);
                    let cstr_setuser = CString::new(setuser).unwrap();
                    let grouplist = ::nix::unistd::getgrouplist(&cstr_setuser, gid);
                    if grouplist.is_err() {
                        error!("Failed to get supplementary group list");
                        ::std::process::exit(1);
                    }
                    let grouplist = grouplist.unwrap();
                    let result = ::nix::unistd::setgroups(&grouplist[..]);
                    if result.is_err() {
                        error!("Failed to set supplementary group list");
                        ::std::process::exit(1);
                    }

                    let result = ::nix::unistd::setgid(gid);
                    if result.is_err() {
                        error!("Failed to setgid");
                        ::std::process::exit(1);
                    }
                    let result = ::nix::unistd::setuid(::nix::unistd::Uid::from_raw(pwent.uid));
                    if result.is_err() {
                        error!("Failed to setuid");
                        ::std::process::exit(1);
                    }
                    let result = ::std::env::set_current_dir(pwent.home);
                    if result.is_err() {
                        let result = ::std::env::set_current_dir("/");
                        if result.is_err() {
                            error!("Failed to change directory");
                            ::std::process::exit(1);
                        }
                    }
                    *self.internal.privs_dropped.borrow_mut() = true;
                }
            } else {
                if let Some(home) = ::dirs::home_dir() {
                    ::std::env::set_current_dir(home).ok();
                }
            }
        }

        #[cfg(unix)]
        {
            if ::nix::unistd::getuid().is_root() && !*self.internal.privs_dropped.borrow() {
                error!("Running as root, but did not drop privileges. Exiting.");
                ::std::process::exit(1);
            }
        }
    }
}
