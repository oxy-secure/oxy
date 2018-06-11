use libc::{ioctl, winsize, TIOCSWINSZ};
use nix::{
    pty::openpty, unistd::{
        close, dup2, execv, fork, setsid, ForkResult::{Child, Parent}, Pid,
    },
};
use std::{ffi::CString, os::unix::io::RawFd, path::PathBuf};
use transportation::BufferedTransport;

pub struct Pty {
    pub underlying: BufferedTransport,
    pub fd:         RawFd,
    pub child_pid:  Pid,
}

impl Pty {
    pub fn forkpty(command: &str) -> Pty {
        let result = openpty(None, None).expect("openpty failed");
        let parent_fd = result.master;
        let child_fd = result.slave;
        debug!("openpty results: {:?} {:?}", parent_fd, child_fd);

        // Do some allocs before the fork, because you're not supposed to after
        let sh = CString::new("/bin/sh").unwrap();
        let sh2 = CString::new("/bin/sh").unwrap();
        let minus_c = CString::new("-c").unwrap();
        let command = CString::new(command).unwrap();

        match fork() {
            Ok(Parent { child }) => {
                let bt = BufferedTransport::from(parent_fd);
                Pty {
                    underlying: bt,
                    fd:         parent_fd,
                    child_pid:  child,
                }
            }
            Ok(Child) => {
                setsid().unwrap();
                dup2(child_fd, 0).unwrap();
                dup2(child_fd, 1).unwrap();
                dup2(child_fd, 2).unwrap();
                close(child_fd).unwrap();
                execv(&sh, &[sh2, minus_c, command]).expect("execv failed");
                unreachable!();
            }
            Err(_) => panic!("Fork failed"),
        }
    }

    pub fn get_cwd(&self) -> String {
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

    pub fn set_size(&self, w: u16, h: u16) {
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
