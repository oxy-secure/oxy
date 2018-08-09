use libc::{ioctl, winsize, TIOCSCTTY, TIOCSWINSZ};
use nix::{
    pty::openpty,
    unistd::{
        close, dup2, execvp, fork, setsid,
        ForkResult::{Child, Parent},
        Pid,
    },
};
use std::{ffi::CString, os::unix::io::RawFd, path::PathBuf};
use transportation::BufferedTransport;

pub(crate) struct Pty {
    pub(crate) underlying: BufferedTransport,
    pub(crate) fd:         RawFd,
    pub(crate) child_pid:  Pid,
}

fn multiplexer_available(peer: Option<&str>) -> bool {
    let command = ::conf::multiplexer(peer);
    if command.is_none() {
        return false;
    }
    let command = command.unwrap();
    let command = ::shlex::split(&command);
    if command.is_none() {
        warn!("Failed to parse multiplexer command");
        return false;
    }
    let command = command.unwrap();
    if command.is_empty() {
        return false;
    }
    let status = ::std::fs::metadata(&command[0]);
    if status.is_err() {
        return false;
    }
    return true;
}

impl Pty {
    pub(crate) fn forkpty(command: Option<Vec<String>>, peer: Option<&str>) -> Result<Pty, ()> {
        let result = openpty(None, None).map_err(|_| ())?;
        let parent_fd = result.master;
        let child_fd = result.slave;
        debug!("openpty results: {:?} {:?}", parent_fd, child_fd);

        let exe;
        let argv: Vec<CString>;
        if command.is_some() {
            let command = command.unwrap();
            exe = CString::new(command[0].as_str()).map_err(|_| ())?;
            let argv2: Vec<Result<CString, _>> = command.into_iter().map(|x| CString::new(x)).collect();
            if argv2.iter().filter(|x| x.is_err()).next().is_some() {
                return Err(());
            }
            argv = argv2.into_iter().map(|x| x.unwrap()).collect();
        } else {
            if !::arg::matches().is_present("no tmux") && multiplexer_available(peer) {
                let command = ::shlex::split(&::conf::multiplexer(peer).unwrap()).unwrap();
                argv = command.into_iter().map(|x| CString::new(x).unwrap()).collect();
                exe = argv[0].clone();
            } else {
                let shell = ::util::current_user_pw();
                if shell.is_err() {
                    error!("Failed to get user shell.");
                    ::std::process::exit(1);
                }
                let shell = shell.unwrap().shell;
                let shell_fname = PathBuf::from(&shell).file_name().unwrap().to_str().unwrap().to_string();
                exe = CString::new(shell).unwrap();
                argv = vec![CString::new(format!("-{}", shell_fname)).unwrap()]; // A leading - makes it a login shell. Seems like a strange convention to
                                                                                 // me, but OK.
            }
        }

        let mut pids: Vec<i32> = Vec::new();
        if let Ok(dents) = ::std::fs::read_dir("/proc/self/fd/") {
            // TODO: This is probably too platform-specific
            for entry in dents {
                if entry.is_err() {
                    continue;
                }
                let entry = entry.unwrap();
                let entry = entry.file_name().into_string();
                if entry.is_err() {
                    continue;
                }
                let fd: Result<i32, _> = entry.unwrap().parse();
                debug!("Found fd: {:?}", fd);
                if fd.is_err() {
                    continue;
                }
                pids.push(fd.unwrap());
            }
        }
        let empty_signal_mask = ::nix::sys::signal::SigSet::empty();

        match fork() {
            Ok(Parent { child }) => {
                let bt = BufferedTransport::from(parent_fd);
                Ok(Pty {
                    underlying: bt,
                    fd:         parent_fd,
                    child_pid:  child,
                })
            }
            Ok(Child) => {
                empty_signal_mask.thread_set_mask().unwrap();
                setsid().unwrap();
                unsafe { ioctl(child_fd, TIOCSCTTY.into(), 0) };
                dup2(child_fd, 0).unwrap();
                dup2(child_fd, 1).unwrap();
                dup2(child_fd, 2).unwrap();
                close(child_fd).unwrap();
                for i in &pids {
                    if *i > 2 {
                        close(*i).ok();
                    }
                }
                execvp(&exe, &argv[..]).expect("execvp failed");
                unreachable!();
            }
            Err(_) => panic!("Fork failed"),
        }
    }

    pub(crate) fn get_cwd(&self) -> String {
        use nix::fcntl::readlink;
        let mut buf = [0u8; 8192];
        let cwd: PathBuf = ".".into();
        let cwd = cwd.canonicalize().unwrap();
        let cwd = cwd.to_str().unwrap().to_string();
        let proc_path: PathBuf = format!("/proc/{}/cwd", self.child_pid).into();
        readlink(&proc_path, &mut buf)
            .ok()
            .map(|x| x.to_str().unwrap().to_string())
            .unwrap_or(cwd)
    }

    pub(crate) fn set_size(&self, w: u16, h: u16) {
        let size = winsize {
            ws_row:    h,
            ws_col:    w,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // TODO: Is there really not a safe wrapper in nix??? I can't find it.
        unsafe {
            ioctl(self.fd, TIOCSWINSZ, &size as *const _);
        }
    }
}
