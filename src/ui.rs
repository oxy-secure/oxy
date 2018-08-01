#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use shlex;
use std::{cell::RefCell, fs::File, io::Write, rc::Rc, time::Instant};
#[cfg(unix)]
use termion::{
    self,
    raw::{IntoRawMode, RawTerminal},
    terminal_size,
};
use transportation::{BufferedTransport, Notifiable, Notifies};

#[derive(Clone)]
crate struct Ui {
    internal: Rc<UiInternal>,
}

#[derive(Default)]
crate struct UiInternal {
    notify_hook:         RefCell<Option<Rc<dyn Notifiable>>>,
    underlying:          RefCell<Option<BufferedTransport>>,
    messages:            RefCell<Vec<UiMessage>>,
    platform:            RefCell<UiPlatformData>,
    prev_progress:       RefCell<u64>,
    escapestate:         RefCell<u64>,
    progress_start_time: RefCell<Option<Instant>>,
    progress_bytes:      RefCell<u64>,
}

#[cfg(unix)]
#[derive(Default)]
struct UiPlatformData {
    raw: Option<RawTerminal<File>>,
}

#[cfg(not(unix))]
#[derive(Default)]
struct UiPlatformData {}

impl Ui {
    crate fn create() -> Ui {
        #[cfg(not(unix))]
        {
            unimplemented!();
        }
        #[cfg(unix)]
        {
            debug!("Creating a UI");
            let ui = UiInternal::default();
            *ui.underlying.borrow_mut() = Some(BufferedTransport::from(::libc::STDIN_FILENO));
            let ui = Ui { internal: Rc::new(ui) };
            let ui2 = ui.clone();
            ui.internal.underlying.borrow().as_ref().unwrap().set_notify(Rc::new(ui2));
            ui.raw();
            ui
        }
    }

    crate fn paint_progress_bar(&self, progress: u64, bytes: u64) {
        #[cfg(unix)]
        {
            if progress < *self.internal.prev_progress.borrow_mut() {
                *self.internal.progress_start_time.borrow_mut() = Some(Instant::now());
                *self.internal.progress_bytes.borrow_mut() = 0;
            }

            *self.internal.progress_bytes.borrow_mut() += bytes;

            if progress == *self.internal.prev_progress.borrow() {
                return;
            }
            *self.internal.prev_progress.borrow_mut() = progress;
            self.cooked();
            let width = ::termion::terminal_size().unwrap().0 as u64;
            let percentage = progress / 10;
            let decimal = progress % 10;
            let bytes = *self.internal.progress_bytes.borrow();
            let seconds = self
                .internal
                .progress_start_time
                .borrow()
                .as_ref()
                .map(|x| x.elapsed().as_secs())
                .unwrap_or(0);
            let throughput = crate::util::format_throughput(bytes, seconds);
            let bytes = crate::util::format_bytes(bytes);
            let line1 = format!("{}.{}%, {}, {}s, throughput: {}", percentage, decimal, bytes, seconds, throughput);
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
        #[cfg(not(unix))]
        unimplemented!();
        #[cfg(unix)]
        {
            if self.is_raw() {
                self.internal.platform.borrow_mut().raw.as_mut().unwrap().write_all(&data[..]).unwrap();
                self.internal.platform.borrow_mut().raw.as_mut().unwrap().flush().unwrap();
            }
        }
    }

    crate fn pty_size(&self) -> (u16, u16) {
        #[cfg(not(unix))]
        unimplemented!();
        // Maybe later we'll want to save space for other UI elements
        // (download progress indicators?)
        #[cfg(unix)]
        terminal_size().unwrap()
    }

    crate fn recv(&self) -> Option<UiMessage> {
        if self.internal.messages.borrow_mut().len() == 0 {
            return None;
        }
        Some(self.internal.messages.borrow_mut().remove(0))
    }

    #[cfg(unix)]
    crate fn cooked(&self) {
        self.internal.platform.borrow_mut().raw.take();
    }

    #[cfg(unix)]
    fn raw(&self) {
        if self.is_raw() {
            return;
        }
        let raw = termion::get_tty().unwrap().into_raw_mode().unwrap();
        self.internal.platform.borrow_mut().raw = Some(raw);
    }
    #[cfg(not(unix))]
    fn raw(&self) {
        unimplemented!();
    }

    #[cfg(unix)]
    fn is_raw(&self) -> bool {
        self.internal.platform.borrow_mut().raw.is_some()
    }

    #[cfg(unix)]
    fn write_tty(&self, output: &str) {
        if self.is_raw() {
            self.internal
                .platform
                .borrow_mut()
                .raw
                .as_mut()
                .unwrap()
                .write_all(output.as_bytes())
                .unwrap();
            self.internal.platform.borrow_mut().raw.as_mut().unwrap().flush().unwrap();
            return;
        }
        let mut tty = termion::get_tty().unwrap();
        tty.write_all(output.as_bytes()).unwrap();
        tty.flush().unwrap();
    }

    fn send(&self, msg: UiMessage) {
        self.internal.messages.borrow_mut().push(msg);
        if self.internal.notify_hook.borrow_mut().is_some() {
            let hook = self.internal.notify_hook.borrow_mut().as_ref().unwrap().clone();
            hook.notify();
        }
    }

    fn metacommand(&self, metacommand: String) {
        let parts = shlex::split(metacommand.trim());
        if parts.is_none() {
            warn!("Failed to split command input");
            self.raw();
            return;
        }
        let parts = parts.unwrap();
        let msg = UiMessage::MetaCommand { parts };
        self.send(msg)
    }

    #[cfg(not(unix))]
    fn spawn_readline_thread(&self) {
        unimplemented!();
    }

    #[cfg(unix)]
    fn spawn_readline_thread(&self) {
        if let Some(bt) = self.internal.underlying.borrow_mut().take() {
            bt.detach();
        }
        self.cooked();
        let (tx, rx) = ::std::sync::mpsc::sync_channel(0);
        let (registration, set_readiness) = ::transportation::mio::Registration::new2();
        let registration = Rc::new(registration);
        let registration2 = registration.clone();
        let proxy = self.clone();
        let token2 = Rc::new(RefCell::new(0usize));
        let token3 = token2.clone();
        let token = ::transportation::insert_listener(Rc::new(move || {
            let input: Result<String, _> = rx.recv();
            debug!("Readline thread sent {:?}", input);
            ::transportation::borrow_poll(|poll| {
                poll.deregister(&*registration2).unwrap();
            });
            ::transportation::remove_listener(*token2.borrow());
            let bt = BufferedTransport::from(::libc::STDIN_FILENO);
            bt.set_notify(Rc::new(proxy.clone()));
            *proxy.internal.underlying.borrow_mut() = Some(bt);
            proxy.metacommand(input.unwrap());
            proxy.raw();
        }));
        *token3.borrow_mut() = token;
        ::transportation::borrow_poll(|poll| {
            poll.register(
                &*registration,
                ::transportation::mio::Token(token),
                ::transportation::mio::Ready::readable(),
                ::transportation::mio::PollOpt::level(),
            ).unwrap();
        });
        ::std::thread::spawn(move || {
            let result: String = read_line();
            set_readiness.set_readiness(::transportation::mio::Ready::readable()).unwrap();
            let _ = tx.send(result);
        });
    }
}

fn read_line() -> String {
    let reader = ::linefeed::Interface::new("oxy");
    if let Err(error) = reader {
        eprintln!("");
        warn!("Failed to instantiate linefeed reader: {:?}", error);
        eprint!("oxy> ");
        let mut line = String::new();
        let _ = ::std::io::stdin().read_line(&mut line);
        return line;
    }
    let reader = reader.unwrap();
    reader.set_prompt("oxy> ").unwrap();
    let result = reader.read_line();
    match result {
        Ok(::linefeed::reader::ReadResult::Input(result)) => {
            return result;
        }
        _ => {
            return "".to_string();
        }
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

            let mut data = self.internal.underlying.borrow().as_ref().unwrap().take();
            if data[..] == f10[..] {
                self.spawn_readline_thread();
                return;
            }
            if data[..] == f12[..] {
                ::crate::exit::exit(0);
            }
            if data[..] == enter[..] {
                *self.internal.escapestate.borrow_mut() = 1;
            } else {
                let cur = *self.internal.escapestate.borrow();
                if cur == 1 && data[..] == tilde[..] {
                    *self.internal.escapestate.borrow_mut() = 2;
                    return;
                } else if cur == 2 && data[..] == key_c[..] {
                    *self.internal.escapestate.borrow_mut() = 1;
                    self.write_tty("\n\roxy> ");
                    self.cooked();
                    return;
                } else if cur == 2 && data[..] == dot[..] {
                    ::crate::exit::exit(0);
                } else if cur == 2 {
                    let mut data2 = tilde.to_vec();
                    data2.extend(data);
                    data = data2;
                    *self.internal.escapestate.borrow_mut() = 0;
                } else {
                    *self.internal.escapestate.borrow_mut() = 0;
                }
            }
            debug!("UI Data: {:?}", data);
            let msg = UiMessage::RawInput { input: data };
            self.send(msg);
        }
    }
}

impl Notifies for Ui {
    fn set_notify(&self, callback: Rc<dyn Notifiable>) {
        *self.internal.notify_hook.borrow_mut() = Some(callback);
    }
}

#[derive(Clone, Debug)]
crate enum UiMessage {
    MetaCommand { parts: Vec<String> },
    RawInput { input: Vec<u8> },
}
