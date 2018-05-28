use shlex;
use std::{cell::RefCell, fs::File, io::Write, rc::Rc};
#[cfg(unix)]
use termion::{
	self, raw::{IntoRawMode, RawTerminal}, terminal_size,
};
use transportation::{BufferedTransport, Notifiable, Notifies};

#[derive(Clone)]
pub struct Ui {
	notify_hook: Rc<RefCell<Option<Rc<Notifiable>>>>,
	underlying:  BufferedTransport,
	messages:    Rc<RefCell<Vec<UiMessage>>>,
	platform:    Rc<RefCell<UiPlatformData>>,
}

#[cfg(unix)]
struct UiPlatformData {
	raw: Option<RawTerminal<File>>,
}

#[cfg(windows)]
struct UiPlatformData {}

impl Ui {
	pub fn create() -> Ui {
		#[cfg(windows)]
		{
			unimplemented!();
		}
		#[cfg(unix)]
		{
			debug!("Creating a UI");
			let platform = UiPlatformData { raw: None };
			let ui = Ui {
				notify_hook: Rc::new(RefCell::new(None)),
				underlying:  BufferedTransport::from(0),
				platform:    Rc::new(RefCell::new(platform)),
				messages:    Rc::new(RefCell::new(Vec::new())),
			};
			let ui2 = ui.clone();
			ui.underlying.set_notify(Rc::new(ui2));

			ui.raw();

			ui
		}
	}

	pub fn pty_data(&self, data: Vec<u8>) {
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

	pub fn pty_size(&self) -> (u16, u16) {
		#[cfg(windows)]
		unimplemented!();
		// Maybe later we'll want to save space for other UI elements
		// (download progress indicators?)
		#[cfg(unix)]
		terminal_size().unwrap()
	}

	pub fn recv(&self) -> Option<UiMessage> {
		if self.messages.borrow_mut().len() == 0 {
			return None;
		}
		Some(self.messages.borrow_mut().remove(0))
	}

	#[cfg(unix)]
	pub fn cooked(&self) {
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
	fn write_tty(&self, output: String) {
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

impl Notifiable for Ui {
	fn notify(&self) {
		#[cfg(unix)]
		{
			let f10 = [27, 91, 50, 49, 126];
			let f12 = [27, 91, 50, 52, 126];

			let data = self.underlying.take();
			if &data[..] == &f10[..] {
				self.write_tty(format!("\n\roxy> "));
				self.cooked();
				return;
			}
			if &data[..] == &f12[..] {
				self.cooked();
				::std::process::exit(0);
			}
			if !self.is_raw() {
				match String::from_utf8(data.to_vec()).unwrap().trim() {
					"quit" => ::std::process::exit(0),
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
	fn set_notify(&self, callback: Rc<Notifiable>) {
		*self.notify_hook.borrow_mut() = Some(callback);
	}
}

#[derive(Clone, Debug)]
pub enum UiMessage {
	MetaCommand { parts: Vec<String> },
	RawInput { input: Vec<u8> },
}
