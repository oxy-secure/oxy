#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use shlex;
use std::{cell::RefCell, fs::File, io::Write, rc::Rc};
#[cfg(unix)]
use termion::{
    self,
    raw::{IntoRawMode, RawTerminal},
    terminal_size,
};
use transportation::{BufferedTransport, Notifiable, Notifies};

#[derive(Clone)]
crate struct Ui {
    notify_hook:   Rc<RefCell<Option<Rc<dyn Notifiable>>>>,
    underlying:    BufferedTransport,
    messages:      Rc<RefCell<Vec<UiMessage>>>,
    platform:      Rc<RefCell<UiPlatformData>>,
    prev_progress: Rc<RefCell<u64>>,
    escapestate:   Rc<RefCell<u64>>,
}

#[cfg(unix)]
struct UiPlatformData {
    raw: Option<RawTerminal<File>>,
}

#[cfg(windows)]
struct UiPlatformData {}

impl Ui {
    crate fn create() -> Ui {
        #[cfg(windows)]
        {
            unimplemented!();
        }
        #[cfg(unix)]
        {
            debug!("Creating a UI");
            let platform = UiPlatformData { raw: None };
            let ui = Ui {
                notify_hook:   Rc::new(RefCell::new(None)),
                underlying:    BufferedTransport::from(0),
                platform:      Rc::new(RefCell::new(platform)),
                messages:      Rc::new(RefCell::new(Vec::new())),
                prev_progress: Rc::new(RefCell::new(0)),
                escapestate:   Rc::new(RefCell::new(0)),
            };
            let ui2 = ui.clone();
            ui.underlying.set_notify(Rc::new(ui2));

            let old_panic_hook = ::std::panic::take_hook();
            ::std::panic::set_hook(Box::new(move |x| {
                ::std::process::Command::new("stty").arg("cooked").arg("echo").spawn().ok();
                cleanup();
                old_panic_hook(x)
            }));
            ui.raw();

            ui
        }
    }

    crate fn paint_progress_bar(&self, progress: u64) {
        #[cfg(unix)]
        {
            if progress == *self.prev_progress.borrow() {
                return;
            }
            *self.prev_progress.borrow_mut() = progress;
            self.cooked();
            let width = ::termion::terminal_size().unwrap().0 as u64;
            let percentage = progress / 10;
            let decimal = progress % 10;
            let line1 = format!("Transfered: {}.{}%", percentage, decimal);
            let barwidth: u64 = (width * percentage) / 100;
            let mut x = "=".repeat(barwidth as usize);
            if x.len() > 0 && percentage < 100 {
                let len = x.len();
                x.remove(len - 1);
                x.push('>');
            }
            {
                let stdout = ::std::io::stdout();
                let mut lock = stdout.lock();
                let mut data = Vec::new();
                data.extend(b"\x1b[s"); // Save cursor position
                data.extend(b"\x1b[100m"); // Grey background
                data.extend(b"\x1b[2;1H"); // Move to the second line
                data.extend(b"\x1b[0K"); // Clear the line
                data.extend(b"\x1b[1;1H"); // Move to the first line
                data.extend(b"\x1b[0K"); // Clear the line
                data.extend(line1.as_bytes());
                data.extend(b"\n");
                data.extend(x.as_bytes());
                data.extend(b"\n");
                data.extend(b"\x1b[0m"); // Reset background
                data.extend(b"\x1b[u"); // Restore cursor position
                lock.write_all(&data[..]).unwrap();
                lock.flush().unwrap();
            }
            self.raw();
        }
    }

    crate fn log_info(&self, message: &str) {
        #[cfg(unix)]
        self.cooked();
        info!("{}", message);
        #[cfg(unix)]
        self.raw();
    }

    crate fn log_debug(&self, message: &str) {
        #[cfg(unix)]
        self.cooked();
        debug!("{}", message);
        #[cfg(unix)]
        self.raw();
    }

    crate fn log_warn(&self, message: &str) {
        #[cfg(unix)]
        self.cooked();
        warn!("{}", message);
        #[cfg(unix)]
        self.raw();
    }

    crate fn pty_data(&self, data: &[u8]) {
        #[cfg(windows)]
        unimplemented!();
        #[cfg(unix)]
        {
            if self.is_raw() {
                self.platform.borrow_mut().raw.as_mut().unwrap().write_all(&data[..]).unwrap();
                self.platform.borrow_mut().raw.as_mut().unwrap().flush().unwrap();
            }
        }
    }

    crate fn pty_size(&self) -> (u16, u16) {
        #[cfg(windows)]
        unimplemented!();
        // Maybe later we'll want to save space for other UI elements
        // (download progress indicators?)
        #[cfg(unix)]
        terminal_size().unwrap()
    }

    crate fn recv(&self) -> Option<UiMessage> {
        if self.messages.borrow_mut().len() == 0 {
            return None;
        }
        Some(self.messages.borrow_mut().remove(0))
    }

    #[cfg(unix)]
    crate fn cooked(&self) {
        self.platform.borrow_mut().raw.take();
    }

    #[cfg(unix)]
    fn raw(&self) {
        if self.is_raw() {
            return;
        }
        let raw = termion::get_tty().unwrap().into_raw_mode().unwrap();
        self.platform.borrow_mut().raw = Some(raw);
    }

    #[cfg(unix)]
    fn is_raw(&self) -> bool {
        self.platform.borrow_mut().raw.is_some()
    }

    #[cfg(unix)]
    fn write_tty(&self, output: &str) {
        if self.is_raw() {
            self.platform.borrow_mut().raw.as_mut().unwrap().write_all(output.as_bytes()).unwrap();
            self.platform.borrow_mut().raw.as_mut().unwrap().flush().unwrap();
            return;
        }
        let mut tty = termion::get_tty().unwrap();
        tty.write_all(output.as_bytes()).unwrap();
        tty.flush().unwrap();
    }

    fn send(&self, msg: UiMessage) {
        self.messages.borrow_mut().push(msg);
        if self.notify_hook.borrow_mut().is_some() {
            let hook = self.notify_hook.borrow_mut().as_ref().unwrap().clone();
            hook.notify();
        }
    }
}

crate fn cleanup() {
    #[cfg(unix)]
    unsafe {
        let mut bits: i32 = 0;
        ::libc::fcntl(0, ::libc::F_GETFL, &mut bits);
        bits &= !::libc::O_NONBLOCK;
        ::libc::fcntl(0, ::libc::F_SETFL, bits);
    }
}

impl Notifiable for Ui {
    fn notify(&self) {
        #[cfg(unix)]
        {
            let f10 = [27, 91, 50, 49, 126];
            let f12 = [27, 91, 50, 52, 126];
            let enter = [13];
            let tilde = [126];
            let key_c = [67];
            let dot = [46];

            let mut data = self.underlying.take();
            if data[..] == f10[..] {
                self.write_tty("\n\roxy> ");
                self.cooked();
                return;
            }
            if data[..] == f12[..] {
                ::crate::exit::exit(0);
            }
            if data[..] == enter[..] {
                *self.escapestate.borrow_mut() = 1;
            } else {
                let cur = *self.escapestate.borrow();
                if cur == 1 && data[..] == tilde[..] {
                    *self.escapestate.borrow_mut() = 2;
                    return;
                } else if cur == 2 && data[..] == key_c[..] {
                    *self.escapestate.borrow_mut() = 1;
                    self.write_tty("\n\roxy> ");
                    self.cooked();
                    return;
                } else if cur == 2 && data[..] == dot[..] {
                    ::crate::exit::exit(0);
                } else if cur == 2 {
                    let mut data2 = tilde.to_vec();
                    data2.extend(data);
                    data = data2;
                    *self.escapestate.borrow_mut() = 0;
                } else {
                    *self.escapestate.borrow_mut() = 0;
                }
            }
            if !self.is_raw() {
                match String::from_utf8(data.to_vec()).unwrap().trim() {
                    "quit" => {
                        cleanup();
                        ::std::process::exit(0)
                    }
                    x => {
                        let parts = shlex::split(x);
                        if parts.is_none() {
                            warn!("Failed to split command input");
                            self.raw();
                            return;
                        }
                        let parts = parts.unwrap();
                        let msg = UiMessage::MetaCommand { parts };
                        self.send(msg)
                    }
                }
                self.raw();
                return;
            }
            debug!("UI Data: {:?}", data);
            let msg = UiMessage::RawInput { input: data };
            self.send(msg);
        }
    }
}

impl Notifies for Ui {
    fn set_notify(&self, callback: Rc<dyn Notifiable>) {
        *self.notify_hook.borrow_mut() = Some(callback);
    }
}

#[derive(Clone, Debug)]
crate enum UiMessage {
    MetaCommand { parts: Vec<String> },
    RawInput { input: Vec<u8> },
}
