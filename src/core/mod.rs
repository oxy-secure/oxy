mod kex;

use self::kex::{KexData, NakedState};
use arg::{self, perspective};
use byteorder::{self, ByteOrder};
use message::OxyMessage::{self, *};
use num;
#[cfg(unix)]
use pty::Pty;
use shlex;
use std::{
	cell::RefCell, collections::HashMap, fs::File, io::{Read, Write}, net::ToSocketAddrs, rc::Rc,
};
use transportation::{
	self, mio::{
		net::{TcpListener, TcpStream}, PollOpt, Ready, Token,
	}, BufferedTransport, EncryptedTransport,
	EncryptionPerspective::{Alice, Bob}, MessageTransport, Notifiable, Notifies, ProtocolTransport,
};
#[cfg(unix)]
use tuntap::{TunTap, TunTapType};
use ui::Ui;

#[derive(Clone)]
pub struct Oxy {
	naked_transport: Rc<RefCell<Option<MessageTransport>>>,
	underlying_transport: Rc<RefCell<Option<ProtocolTransport>>>,
	ui: Rc<RefCell<Option<Ui>>>,
	outgoing_ticker: Rc<RefCell<u64>>,
	incoming_ticker: Rc<RefCell<u64>>,
	transfers_out: Rc<RefCell<Vec<(u64, File)>>>,
	transfers_in: Rc<RefCell<HashMap<u64, File>>>,
	port_binds: Rc<RefCell<HashMap<u64, PortBind>>>,
	local_streams: Rc<RefCell<HashMap<u64, PortStream>>>,
	remote_streams: Rc<RefCell<HashMap<u64, PortStream>>>,
	remote_bind_destinations: Rc<RefCell<HashMap<u64, String>>>,
	naked_state: Rc<RefCell<NakedState>>,
	kex_data: Rc<RefCell<KexData>>,
	socks_binds: Rc<RefCell<HashMap<u64, SocksBind>>>,
	#[cfg(unix)]
	pty: Rc<RefCell<Option<Pty>>>,
	#[cfg(unix)]
	tuntaps: Rc<RefCell<HashMap<u64, TunTap>>>,
}

impl Oxy {
	fn alice_only(&self) {
		assert!(perspective() == Alice);
	}

	fn bob_only(&self) {
		assert!(perspective() == Bob);
	}

	fn create<T: Into<BufferedTransport>>(transport: T) -> Oxy {
		let bt: BufferedTransport = transport.into();
		let mt = <MessageTransport as From<BufferedTransport>>::from(bt);
		let x = Oxy {
			naked_transport: Rc::new(RefCell::new(Some(mt))),
			underlying_transport: Rc::new(RefCell::new(None)),
			ui: Rc::new(RefCell::new(None)),
			outgoing_ticker: Rc::new(RefCell::new(0)),
			incoming_ticker: Rc::new(RefCell::new(0)),
			transfers_out: Rc::new(RefCell::new(Vec::new())),
			transfers_in: Rc::new(RefCell::new(HashMap::new())),
			port_binds: Rc::new(RefCell::new(HashMap::new())),
			local_streams: Rc::new(RefCell::new(HashMap::new())),
			remote_streams: Rc::new(RefCell::new(HashMap::new())),
			remote_bind_destinations: Rc::new(RefCell::new(HashMap::new())),
			naked_state: Rc::new(RefCell::new(NakedState::Reject)),
			kex_data: Rc::new(RefCell::new(KexData::default())),
			socks_binds: Rc::new(RefCell::new(HashMap::new())),
			#[cfg(unix)]
			pty: Rc::new(RefCell::new(None)),
			#[cfg(unix)]
			tuntaps: Rc::new(RefCell::new(HashMap::new())),
		};
		let proxy = NakedNotificationProxy { oxy: x.clone() };
		x.naked_transport.borrow_mut().as_mut().unwrap().set_notify(Rc::new(proxy));
		x.create_ui();
		x
	}

	fn create_ui(&self) {
		if perspective() == Bob {
			return;
		}
		*self.ui.borrow_mut() = Some(Ui::create());
		let proxy = UiNotificationProxy { oxy: self.clone() };
		let proxy = Rc::new(proxy);
		self.ui.borrow_mut().as_ref().unwrap().set_notify(proxy);
	}

	fn is_encrypted(&self) -> bool {
		self.underlying_transport.borrow().is_some()
	}

	pub fn run<T: Into<BufferedTransport>>(transport: T) -> ! {
		let oxy = Oxy::create(transport);
		if perspective() == Alice {
			oxy.advertise_client_key();
		}
		if perspective() == Bob {
			*oxy.naked_state.borrow_mut() = NakedState::WaitingForClientKey;
		}
		transportation::run();
	}

	fn send(&self, message: OxyMessage) -> u64 {
		let message_number = self.tick_outgoing();
		debug!("Sending message {}", message_number);
		trace!("Sending message {}: {:?}", message_number, message);
		self.underlying_transport.borrow().as_ref().unwrap().send(message);
		message_number
	}

	fn handle_message(&self, message: OxyMessage, message_number: u64) {
		debug!("Recieved message {}", message_number);
		trace!("Received message {}: {:?}", message_number, message);
		match message {
			DummyMessage { .. } => (),
			BasicCommand { command } => {
				assert!(perspective() == Bob);
				#[cfg(unix)]
				let sh = "/bin/sh";
				#[cfg(unix)]
				let flag = "-c";
				#[cfg(windows)]
				let sh = "cmd.exe";
				#[cfg(windows)]
				let flag = "/c";
				let result = ::std::process::Command::new(sh).arg(flag).arg(command).output();
				if let Ok(result) = result {
					self.send(BasicCommandOutput {
						stdout: result.stdout,
						stderr: result.stderr,
					});
				}
			}
			#[cfg(unix)]
			PtyRequest { command } => {
				assert!(perspective() == Bob);
				let pty = Pty::forkpty(&command);
				pty.underlying.set_notify(Rc::new(PtyNotificationProxy { oxy: self.clone() }));
				*self.pty.borrow_mut() = Some(pty);
				self.send(PtyRequestResponse { granted: true });
				trace!("Successfully allocated PTY");
			}
			#[cfg(unix)]
			PtyRequestResponse { granted } => {
				assert!(perspective() == Alice);
				if !granted {
					warn!("PTY open failed");
					return;
				}
				let (w, h) = self.ui.borrow_mut().as_mut().unwrap().pty_size();
				self.send(PtySizeAdvertisement { w, h });
			}
			#[cfg(unix)]
			PtySizeAdvertisement { w, h } => {
				assert!(perspective() == Bob);
				self.pty.borrow_mut().as_mut().unwrap().set_size(w, h);
			}
			#[cfg(unix)]
			PtyInput { data } => {
				assert!(perspective() == Bob);
				if self.pty.borrow_mut().is_none() {
					self.send(Reject { message_number });
					return;
				}
				self.pty.borrow_mut().as_mut().unwrap().underlying.put(&data[..]);
			}
			#[cfg(unix)]
			PtyOutput { data } => {
				assert!(perspective() == Alice);
				self.ui.borrow_mut().as_mut().unwrap().pty_data(&data);
			}
			BasicCommandOutput { stdout, stderr } => {
				assert!(perspective() == Alice);
				debug!("BasicCommandOutput {:?}, {:?}", stdout, stderr);
				if let Ok(stdout) = String::from_utf8(stdout) {
					debug!("stdout:\n-----\n{}\n-----", stdout);
				}
			}
			DownloadRequest { path } => {
				assert!(perspective() == Bob);
				let file = File::open(path);
				if file.is_err() {
					self.send(Reject { message_number });
					return;
				}
				let file = file.unwrap();
				let metadata = file.metadata();
				if metadata.is_err() {
					self.send(Reject { message_number });
					return;
				}
				let metadata = metadata.unwrap();
				self.send(FileSize {
					reference: message_number,
					size:      metadata.len(),
				});
				self.transfers_out.borrow_mut().push((message_number, file));
			}
			UploadRequest { path } => {
				assert!(perspective() == Bob);
				let file = File::create(path).unwrap();
				self.transfers_in.borrow_mut().insert(message_number, file);
			}
			FileData { reference, data } => {
				if data.is_empty() {
					debug!("File transfer completed");
					self.transfers_in.borrow_mut().remove(&reference);
					return;
				}
				self.transfers_in.borrow_mut().get_mut(&reference).unwrap().write_all(&data[..]).unwrap();
			}
			BindConnectionAccepted { reference } => {
				assert!(perspective() == Alice);
				let addr = self.remote_bind_destinations.borrow_mut().get(&reference).unwrap().parse().unwrap();
				let stream = TcpStream::connect(&addr).unwrap();
				let bt = BufferedTransport::from(stream);
				let stream = PortStream {
					stream: bt,
					token:  message_number,
					oxy:    self.clone(),
					local:  false,
				};
				let stream2 = Rc::new(stream.clone());
				stream.stream.set_notify(stream2);
				self.remote_streams.borrow_mut().insert(message_number, stream);
			}
			RemoteOpen { addr } => {
				assert!(perspective() == Bob);
				let dest = addr.to_socket_addrs().unwrap().next().unwrap();
				debug!("Resolved RemoteOpen destination to {:?}", dest);
				let stream = TcpStream::connect(&dest).unwrap();
				let bt = BufferedTransport::from(stream);
				let stream = PortStream {
					stream: bt,
					token:  message_number,
					oxy:    self.clone(),
					local:  false,
				};
				let stream2 = Rc::new(stream.clone());
				stream.stream.set_notify(stream2);
				self.remote_streams.borrow_mut().insert(message_number, stream);
			}
			RemoteBind { addr } => {
				assert!(perspective() == Bob);
				let bind = TcpListener::bind(&addr.parse().unwrap()).unwrap();
				let proxy = BindNotificationProxy {
					oxy:   self.clone(),
					token: Rc::new(RefCell::new(message_number)),
				};
				let proxy = Rc::new(proxy);
				let token = transportation::insert_listener(proxy.clone());
				transportation::borrow_poll(|poll| {
					poll.register(&bind, Token(token), Ready::readable(), PollOpt::level()).unwrap();
				});
				let bind = PortBind {
					listener:    bind,
					local_spec:  addr,
					remote_spec: "".to_string(),
				};
				self.port_binds.borrow_mut().insert(message_number, bind);
			}
			RemoteStreamData { reference, data } => {
				self.remote_streams.borrow_mut().get_mut(&reference).unwrap().stream.put(&data[..]);
			}
			LocalStreamData { reference, data } => {
				self.local_streams.borrow_mut().get_mut(&reference).unwrap().stream.put(&data[..]);
			}
			#[cfg(unix)]
			TunnelRequest { tap, name } => {
				self.bob_only();
				let mode = if tap { TunTapType::Tap } else { TunTapType::Tun };
				let tuntap = TunTap::create(mode, &name, message_number, self.clone());
				self.tuntaps.borrow_mut().insert(message_number, tuntap);
			}
			#[cfg(unix)]
			TunnelData { reference, data } => {
				let borrow = self.tuntaps.borrow_mut();
				borrow.get(&reference).unwrap().send(&data);
			}
			_ => (),
		}
	}

	fn handle_metacommand(&self, mut parts: Vec<String>) {
		match parts[0].as_str() {
			"sh" => {
				self.send(BasicCommand { command: parts.remove(1) }); // TODO: ERROR_HANDLING
			}
			"pty" => {
				let command = if parts.len() > 1 { parts.remove(1) } else { "bash".to_string() };
				self.send(PtyRequest { command });
			}
			"download" => {
				debug!("File transfer started");
				let id = self.send(DownloadRequest { path: parts.remove(1) }); // TODO: ERROR_HANDLING
				let file = File::create(parts.remove(1)).unwrap();
				self.transfers_in.borrow_mut().insert(id, file);
			}
			"upload" => {
				let id = self.send(UploadRequest { path: parts.remove(2) });
				let file = File::open(parts.remove(1)).unwrap();
				self.transfers_out.borrow_mut().push((id, file));
			}
			"L" => {
				let remote_spec = parts.remove(2);
				let local_spec = parts.remove(1);
				let bind = TcpListener::bind(&local_spec.parse().unwrap()).unwrap();
				let proxy = BindNotificationProxy {
					oxy:   self.clone(),
					token: Rc::new(RefCell::new(0)),
				};
				let proxy = Rc::new(proxy);
				let token = transportation::insert_listener(proxy.clone());
				let token_sized = <u64 as num::NumCast>::from(token).unwrap();
				*proxy.token.borrow_mut() = token_sized;
				transportation::borrow_poll(|poll| {
					poll.register(&bind, Token(token), Ready::readable(), PollOpt::level()).unwrap();
				});
				let bind = PortBind {
					listener: bind,
					local_spec,
					remote_spec,
				};
				self.port_binds.borrow_mut().insert(token_sized, bind);
			}
			"R" => {
				let bind_id = self.send(RemoteBind { addr: parts.remove(1) });
				self.remote_bind_destinations.borrow_mut().insert(bind_id, parts.remove(1));
			}
			#[cfg(unix)]
			"tun" => {
				let reference_number = self.send(TunnelRequest {
					tap:  false,
					name: parts.remove(2),
				});
				let tuntap = TunTap::create(TunTapType::Tun, &parts[1], reference_number, self.clone());
				self.tuntaps.borrow_mut().insert(reference_number, tuntap);
			}
			#[cfg(unix)]
			"tap" => {
				let reference_number = self.send(TunnelRequest {
					tap:  true,
					name: parts.remove(2),
				});
				let tuntap = TunTap::create(TunTapType::Tap, &parts[1], reference_number, self.clone());
				self.tuntaps.borrow_mut().insert(reference_number, tuntap);
			}
			"socks" => {
				let local_spec = parts.remove(1);
				let bind = TcpListener::bind(&local_spec.parse().unwrap()).unwrap();
				let proxy = SocksBindNotificationProxy {
					oxy:   self.clone(),
					token: Rc::new(RefCell::new(0)),
				};
				let proxy = Rc::new(proxy);
				let token = transportation::insert_listener(proxy.clone());
				let token_sized = <u64 as num::NumCast>::from(token).unwrap();
				*proxy.token.borrow_mut() = token_sized;
				transportation::borrow_poll(|poll| {
					poll.register(&bind, Token(token), Ready::readable(), PollOpt::level()).unwrap();
				});
				let socks = SocksBind { listener: bind };
				self.socks_binds.borrow_mut().insert(token_sized, socks);
			}
			_ => (),
		}
	}

	#[cfg(unix)]
	pub fn notify_tuntap(&self, reference_number: u64) {
		let borrow = self.tuntaps.borrow_mut();
		let tuntap = borrow.get(&reference_number).unwrap();
		for packet in tuntap.get_packets() {
			self.send(TunnelData {
				reference: reference_number,
				data:      packet,
			});
		}
	}

	fn notify_bind(&self, token: u64) {
		let stream = self.port_binds.borrow_mut().get_mut(&token).unwrap().listener.accept().unwrap();
		let remote_addr = self.port_binds.borrow_mut().get_mut(&token).unwrap().remote_spec.clone();
		let local_addr = self.port_binds.borrow_mut().get_mut(&token).unwrap().local_spec.clone();
		debug!("Accepting a connection for local bind {}", local_addr);
		let stream_token = match perspective() {
			Alice => self.send(RemoteOpen { addr: remote_addr }),
			Bob => self.send(BindConnectionAccepted { reference: token }),
		};
		let bt = BufferedTransport::from(stream.0);
		let stream = PortStream {
			stream: bt,
			token:  stream_token,
			oxy:    self.clone(),
			local:  true,
		};
		let stream2 = Rc::new(stream.clone());
		stream.stream.set_notify(stream2);
		self.local_streams.borrow_mut().insert(stream_token, stream);
	}

	fn notify_ui(&self) {
		use ui::UiMessage::*;
		while let Some(msg) = self.ui.borrow_mut().as_mut().unwrap().recv() {
			match msg {
				MetaCommand { parts } => {
					if parts.is_empty() {
						continue;
					}
					self.handle_metacommand(parts);
				}
				RawInput { input } => {
					self.send(PtyInput { data: input });
				}
			}
		}
	}

	#[cfg(unix)]
	fn notify_pty(&self) {
		let data = self.pty.borrow_mut().as_mut().unwrap().underlying.take();
		debug!("PTY Data: {:?}", data);
		if !data.is_empty() {
			self.send(PtyOutput { data });
		}
	}

	fn tick_outgoing(&self) -> u64 {
		let message_number = *self.outgoing_ticker.borrow_mut();
		let next = message_number.checked_add(1).unwrap();
		*self.outgoing_ticker.borrow_mut() = next;
		message_number
	}

	fn tick_incoming(&self) -> u64 {
		let message_number = *self.incoming_ticker.borrow_mut();
		let next = message_number.checked_add(1).unwrap();
		*self.incoming_ticker.borrow_mut() = next;
		message_number
	}

	fn has_write_space(&self) -> bool {
		self.underlying_transport.borrow_mut().as_ref().unwrap().has_write_space()
	}

	fn service_transfers(&self) {
		if !self.has_write_space() {
			debug!("Write buffer full!  Holding off on servicing transfers.");
			return;
		}
		let mut to_remove = Vec::new();
		for (id, file) in self.transfers_out.borrow_mut().iter_mut() {
			debug!("Servicing transfer {}", id);
			let mut data = [0; 16384];
			let amt = file.read(&mut data[..]).unwrap();
			if amt == 0 {
				debug!("Transfer finished: {}", id);
				to_remove.push(*id);
			}
			self.send(FileData {
				reference: *id,
				data:      data[..amt].to_vec(),
			});
		}
		self.transfers_out.borrow_mut().retain(|x| !to_remove.contains(&x.0));
	}

	fn notify_local_stream(&self, token: u64) {
		debug!("Local stream notify for stream {}", token);
		let data = self.local_streams.borrow_mut().get_mut(&token).unwrap().stream.take();
		self.send(RemoteStreamData { reference: token, data });
	}

	fn notify_remote_stream(&self, token: u64) {
		debug!("Remote stream notify for stream {}", token);
		let data = self.remote_streams.borrow_mut().get_mut(&token).unwrap().stream.take();
		self.send(LocalStreamData { reference: token, data });
	}

	fn upgrade_to_encrypted(&self) {
		if self.is_encrypted() {
			return;
		}
		debug!("Activating encryption.");
		let transport = self.naked_transport.borrow_mut().take().unwrap();
		let bt: BufferedTransport = match transport {
			MessageTransport::BufferedTransport(bt) => bt,
			_ => panic!(),
		};
		let mut key = self.kex_data.borrow_mut().keymaterial.as_ref().unwrap().to_vec();
		key.extend(arg::key().as_bytes());
		let et = EncryptedTransport::create(bt, arg::perspective(), &key);
		let pt = ProtocolTransport::create(et);
		pt.set_notify(Rc::new(self.clone()));
		*self.underlying_transport.borrow_mut() = Some(pt);
		self.notify();
		if perspective() == Alice {
			self.run_batched_metacommands();
		}
	}

	fn run_batched_metacommands(&self) {
		for command in arg::batched_metacommands() {
			let parts = shlex::split(&command).unwrap();
			self.handle_metacommand(parts);
		}
		#[cfg(unix)]
		{
			if ::termion::is_tty(&::std::io::stdout()) {
				self.handle_metacommand(vec!["pty".to_string()]);
			}
		}
	}

	fn notify_socks_bind(&self, token: u64) {
		let mut borrow = self.socks_binds.borrow_mut();
		let bind = borrow.get_mut(&token).unwrap();
		let stream = bind.listener.accept().unwrap().0;
		let bt = BufferedTransport::from(stream);
		let proxy = SocksConnectionNotificationProxy {
			oxy: self.clone(),
			bt,
			state: Rc::new(RefCell::new(SocksState::Initial)),
		};
		let proxy = Rc::new(proxy);
		proxy.bt.set_notify(proxy.clone());
	}
	fn notify_socks_connection(&self, proxy: &SocksConnectionNotificationProxy) {
		let data = proxy.bt.take();
		if data.is_empty() {
			return;
		}
		debug!("Socks data: {:?}", data);
		let state = proxy.state.borrow().clone();
		match state {
			SocksState::Initial => {
				assert!(data[0] == 5);
				proxy.bt.put(b"\x05\x00");
				*proxy.state.borrow_mut() = SocksState::Authed;
			}
			SocksState::Authed => {
				assert!(data[0] == 5);
				assert!(data[1] == 1);
				assert!(data[2] == 0);
				let dest: String;
				match data[3] {
					1 => {
						dest = format!(
							"{}.{}.{}.{}:{}",
							data[4],
							data[5],
							data[6],
							data[7],
							byteorder::BE::read_u16(&data[8..10])
						)
					}
					3 => {
						let len = data[4] as usize;
						let host = String::from_utf8(data[5..5 + len].to_vec()).unwrap();
						let port = byteorder::BE::read_u16(&data[5 + len..5 + len + 2]);
						dest = format!("{}:{}", host, port);
					}
					_ => panic!(),
				}
				debug!("Socks dest: {}", dest);
				let reference = self.send(RemoteOpen { addr: dest });
				proxy.bt.put(b"\x05\x00\x00\x01\x00\x00\x00\x00\x00\x00"); // TODO: Maybe provide like... connection refused by destination host
				let stream = PortStream {
					stream: proxy.bt.clone(),
					token:  reference,
					oxy:    self.clone(),
					local:  true,
				};
				let stream2 = Rc::new(stream.clone());
				stream.stream.set_notify(stream2);
				self.local_streams.borrow_mut().insert(reference, stream);
			}
		}
	}
}

impl Notifiable for Oxy {
	fn notify(&self) {
		if self.underlying_transport.borrow().as_ref().unwrap().is_closed() {
			#[cfg(unix)]
			{
				if let Some(x) = self.ui.borrow_mut().as_ref() {
					x.cooked()
				};
			}
			::std::process::exit(0);
		}
		for message in self.underlying_transport.borrow().as_ref().unwrap().recv_all() {
			let message_number = self.tick_incoming();
			self.handle_message(message, message_number);
		}
		self.service_transfers();
	}
}

struct UiNotificationProxy {
	oxy: Oxy,
}

impl Notifiable for UiNotificationProxy {
	fn notify(&self) {
		self.oxy.notify_ui();
	}
}

struct PtyNotificationProxy {
	oxy: Oxy,
}

impl Notifiable for PtyNotificationProxy {
	fn notify(&self) {
		#[cfg(unix)]
		self.oxy.notify_pty();
	}
}

struct BindNotificationProxy {
	oxy:   Oxy,
	token: Rc<RefCell<u64>>,
}

impl Notifiable for BindNotificationProxy {
	fn notify(&self) {
		self.oxy.notify_bind(*self.token.borrow_mut());
	}
}

struct SocksBind {
	listener: TcpListener,
}

struct SocksBindNotificationProxy {
	oxy:   Oxy,
	token: Rc<RefCell<u64>>,
}

impl Notifiable for SocksBindNotificationProxy {
	fn notify(&self) {
		self.oxy.notify_socks_bind(*self.token.borrow_mut());
	}
}

struct PortBind {
	listener:    TcpListener,
	remote_spec: String,
	local_spec:  String,
}

#[derive(Clone)]
struct PortStream {
	stream: BufferedTransport,
	oxy:    Oxy,
	token:  u64,
	local:  bool,
}

impl Notifiable for PortStream {
	fn notify(&self) {
		if self.local {
			self.oxy.notify_local_stream(self.token);
		} else {
			self.oxy.notify_remote_stream(self.token);
		}
	}
}

struct NakedNotificationProxy {
	oxy: Oxy,
}

impl Notifiable for NakedNotificationProxy {
	fn notify(&self) {
		self.oxy.notify_naked();
	}
}

struct SocksConnectionNotificationProxy {
	oxy:   Oxy,
	bt:    BufferedTransport,
	state: Rc<RefCell<SocksState>>,
}

impl Notifiable for SocksConnectionNotificationProxy {
	fn notify(&self) {
		self.oxy.notify_socks_connection(self);
	}
}

#[derive(PartialEq, Clone, Debug)]
enum SocksState {
	Initial,
	Authed,
}
