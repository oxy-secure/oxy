mod handle_message;
mod kex;
mod metacommands;

use self::kex::{KexData, NakedState};
use arg;
use byteorder::{self, ByteOrder};
use keys;
use message::OxyMessage::{self, *};
#[cfg(unix)]
use pty::Pty;
use shlex;
use std::{
    cell::RefCell, collections::HashMap, fs::File, io::Read, path::PathBuf, rc::Rc, time::{Duration, Instant},
};
use transportation::{
    self, mio::net::TcpListener, set_timeout, BufferedTransport, EncryptedTransport, EncryptionPerspective::{Alice, Bob}, MessageTransport,
    Notifiable, Notifies, ProtocolTransport,
};
#[cfg(unix)]
use tuntap::TunTap;
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
    copy_peer: Rc<RefCell<Option<BufferedTransport>>>,
    is_copy_source: Rc<RefCell<bool>>,
    fetch_file_ticker: Rc<RefCell<u64>>,
    last_message_seen: Rc<RefCell<Instant>>,
    file_transfer_reference: Rc<RefCell<Option<u64>>>,
    #[cfg(unix)]
    pty: Rc<RefCell<Option<Pty>>>,
    #[cfg(unix)]
    tuntaps: Rc<RefCell<HashMap<u64, TunTap>>>,
}

impl Oxy {
    fn alice_only(&self) {
        assert!(self.perspective() == Alice);
    }

    fn bob_only(&self) {
        assert!(self.perspective() == Bob);
    }

    fn perspective(&self) -> transportation::EncryptionPerspective {
        arg::perspective()
    }

    pub fn create<T: Into<BufferedTransport>>(transport: T) -> Oxy {
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
            copy_peer: Rc::new(RefCell::new(None)),
            is_copy_source: Rc::new(RefCell::new(false)),
            fetch_file_ticker: Rc::new(RefCell::new(0)),
            file_transfer_reference: Rc::new(RefCell::new(None)),
            last_message_seen: Rc::new(RefCell::new(Instant::now())),
            #[cfg(unix)]
            pty: Rc::new(RefCell::new(None)),
            #[cfg(unix)]
            tuntaps: Rc::new(RefCell::new(HashMap::new())),
        };
        let proxy = x.clone();
        x.naked_transport
            .borrow_mut()
            .as_mut()
            .unwrap()
            .set_notify(Rc::new(move || proxy.notify_naked()));
        x
    }

    fn create_ui(&self) {
        if self.perspective() == Bob {
            return;
        }
        #[cfg(unix)]
        {
            if !::termion::is_tty(&::std::io::stdout()) {
                return;
            }
        }

        *self.ui.borrow_mut() = Some(Ui::create());
        let proxy = self.clone();
        let proxy = Rc::new(move || proxy.notify_ui());
        self.ui.borrow_mut().as_ref().unwrap().set_notify(proxy);
    }

    fn is_encrypted(&self) -> bool {
        self.underlying_transport.borrow().is_some()
    }

    pub fn fetch_files(&self, peer: BufferedTransport) {
        *self.copy_peer.borrow_mut() = Some(peer);
        *self.is_copy_source.borrow_mut() = true;
    }

    pub fn recv_files(&self, peer: BufferedTransport) {
        *self.copy_peer.borrow_mut() = Some(peer);
        *self.is_copy_source.borrow_mut() = false;
    }

    pub fn soft_launch(&self) {
        if self.perspective() == Alice {
            self.advertise_client_key();
        }
        if self.perspective() == Bob {
            *self.naked_state.borrow_mut() = NakedState::WaitingForClientKey;
        }
    }

    pub fn launch(&self) -> ! {
        self.soft_launch();
        transportation::run();
    }

    pub fn run<T: Into<BufferedTransport>>(transport: T) -> ! {
        let oxy = Oxy::create(transport);
        oxy.launch();
    }

    fn send(&self, message: OxyMessage) -> u64 {
        let message_number = self.tick_outgoing();
        debug!("Sending message {}", message_number);
        trace!("Sending message {}: {:?}", message_number, message);
        self.underlying_transport.borrow().as_ref().unwrap().send(message);
        message_number
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
        let stream_token = match self.perspective() {
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
        key.extend(keys::static_key());
        let et = EncryptedTransport::create(bt, self.perspective(), &key);
        let pt = ProtocolTransport::create(et);
        let proxy = self.clone();
        pt.set_notify(Rc::new(move || proxy.notify_main_transport()));
        *self.underlying_transport.borrow_mut() = Some(pt);
        self.notify_main_transport();
        self.do_post_auth();
    }

    #[cfg(unix)]
    fn register_signal_handler(&self) {
        let proxy = self.clone();
        transportation::set_signal_handler(Rc::new(move || proxy.notify_signal()));
    }

    #[cfg(unix)]
    fn notify_signal(&self) {
        match transportation::get_signal_name().as_str() {
            "SIGWINCH" => {
                if self.perspective() == Alice && self.ui.borrow().is_some() {
                    let (w, h) = self.ui.borrow_mut().as_mut().unwrap().pty_size();
                    self.send(PtySizeAdvertisement { w, h });
                }
            }
            "SIGCHLD" => {
                info!("Received SIGCHLD");
                if self.pty.borrow().is_some() {
                    let ptypid = self.pty.borrow().as_ref().unwrap().child_pid;
                    let flags = ::nix::sys::wait::WaitPidFlag::WNOHANG;
                    let waitresult = ::nix::sys::wait::waitpid(ptypid, Some(flags));
                    use nix::sys::wait::WaitStatus::Exited;
                    match waitresult {
                        Ok(Exited(_pid, status)) => {
                            self.send(PtyExited { status });
                        }
                        _ => (),
                    };
                }
            }
            _ => (),
        };
    }

    fn do_post_auth(&self) {
        if self.copy_peer.borrow_mut().is_some() {
            if *self.is_copy_source.borrow_mut() {
                let filename = arg::matches().value_of("source").unwrap();
                let filename = filename.splitn(2, ':').nth(1).unwrap().to_string();
                let cmd = ["download", &filename, "/dev/null"].to_vec();
                let cmd = cmd.iter().map(|x| x.to_string()).collect();
                self.handle_metacommand(cmd);
            }
            let proxy = self.clone();
            let proxy = move || proxy.notify_transfer();
            self.copy_peer.borrow_mut().as_mut().unwrap().set_notify(Rc::new(proxy));
            self.notify_transfer();
            return;
        }
        if self.perspective() == Alice {
            self.run_batched_metacommands();
            #[cfg(unix)]
            {
                if ::termion::is_tty(&::std::io::stdout()) {
                    self.handle_metacommand(vec!["pty".to_string()]);
                }
            }
            self.create_ui();
        }
        #[cfg(unix)]
        self.register_signal_handler();
        let proxy = self.clone();
        set_timeout(Rc::new(move || proxy.notify_keepalive()), Duration::from_secs(60));
    }

    fn run_batched_metacommands(&self) {
        for command in arg::batched_metacommands() {
            let parts = shlex::split(&command).unwrap();
            self.handle_metacommand(parts);
        }
    }

    fn notify_transfer(&self) {
        if self.copy_peer.borrow_mut().is_none() {
            return;
        }
        trace!(
            "Transfer notified. Available: {}",
            self.copy_peer.borrow_mut().as_mut().unwrap().available()
        );
        if !self.has_write_space() {
            trace!("Outbound buffers are full, holding off");
            return;
        }
        let data = self.copy_peer.borrow_mut().as_mut().unwrap().recv_all_messages();
        for data in data {
            trace!("Transfer has a message {:?}", data);
            let filenumber = byteorder::BE::read_u64(&data[..8]);
            if filenumber == ::std::u64::MAX {
                *self.fetch_file_ticker.borrow_mut() = ::std::u64::MAX;
            }
            if self.file_transfer_reference.borrow().is_some() && filenumber == *self.fetch_file_ticker.borrow_mut() {
                let reference = self.file_transfer_reference.borrow_mut().unwrap();
                self.send(FileData {
                    data: data[8..].to_vec(),
                    reference,
                });
                #[cfg(unix)]
                ::copy::draw_progress_bar((data.len() - 8) as u64);
            } else {
                assert!(
                    (filenumber == 0 && self.file_transfer_reference.borrow().is_none()) || filenumber == *self.fetch_file_ticker.borrow_mut() + 1
                );
                *self.fetch_file_ticker.borrow_mut() = filenumber;
                let filepart: PathBuf = arg::source_path(filenumber).into();
                let filepart = filepart.file_name().unwrap().to_string_lossy().into_owned();
                let path = arg::dest_path();
                let reference = self.send(UploadRequest {
                    path,
                    filepart,
                    offset_start: None,
                    offset_end: None,
                });
                *self.file_transfer_reference.borrow_mut() = Some(reference);
                self.send(FileData {
                    data: data[8..].to_vec(),
                    reference,
                });
                #[cfg(unix)]
                ::copy::pop_file_size();
                #[cfg(unix)]
                ::copy::draw_progress_bar((data.len() - 8) as u64);
            }
        }
    }

    fn notify_keepalive(&self) {
        trace!("Keepalive!");
        if self.last_message_seen.borrow().elapsed() > Duration::from_secs(180) {
            trace!("Exiting due to lack of keepalives");
            self.exit(2);
        }
        self.send(Ping {});
        let proxy = self.clone();
        set_timeout(Rc::new(move || proxy.notify_keepalive()), Duration::from_secs(60));
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

    fn exit(&self, status: i32) -> ! {
        #[cfg(unix)]
        {
            if let Some(x) = self.ui.borrow_mut().as_ref() {
                x.cooked()
            };
            ::ui::cleanup();
        }
        ::std::process::exit(status);
    }

    fn notify_main_transport(&self) {
        trace!("Core notified");
        if self.underlying_transport.borrow().as_ref().unwrap().is_closed() {
            self.exit(0);
        }
        if self.copy_peer.borrow_mut().is_some() {
            if !*self.is_copy_source.borrow_mut() {
                let peer = self.copy_peer.borrow_mut();
                if *self.fetch_file_ticker.borrow_mut() == ::std::u64::MAX {
                    if peer.as_ref().unwrap().available() == 0 {
                        let underlying = self.underlying_transport.borrow();
                        let mt = &underlying.as_ref().unwrap().mt;
                        match mt {
                            transportation::MessageTransport::EncryptedTransport(et) => {
                                if et.is_drained_forward() {
                                    // Boy, that sure is a tall if stack, eh?
                                    ::std::process::exit(0);
                                }
                            }
                            _ => panic!(),
                        }
                    }
                }
            }
        }
        for message in self.underlying_transport.borrow().as_ref().unwrap().recv_all() {
            let message_number = self.tick_incoming();
            self.handle_message(message, message_number);
        }
        self.service_transfers();
        self.notify_transfer(); // Uhh... this is a function naming disaster. REFACTOR
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
