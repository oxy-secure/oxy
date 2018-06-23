mod handle_message;
mod kex;
mod metacommands;
mod restrict_message;

use self::kex::{KexData, NakedState};
use byteorder::{self, ByteOrder};
#[cfg(unix)]
use crate::pty::Pty;
#[cfg(unix)]
use crate::tuntap::TunTap;
use crate::{
    arg, keys,
    message::OxyMessage::{self, *},
    ui::Ui,
};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use shlex;
use std::{
    cell::RefCell,
    collections::HashMap,
    fs::File,
    io::Read,
    rc::Rc,
    time::{Duration, Instant},
};
use transportation::{
    self,
    mio::net::TcpListener,
    set_timeout, BufferedTransport, EncryptedTransport,
    EncryptionPerspective::{Alice, Bob},
    MessageTransport, Notifiable, Notifies, ProtocolTransport,
};

#[derive(Clone)]
pub struct Oxy {
    internal: Rc<OxyInternal>,
}

crate struct TransferOut {
    reference:        u64,
    file:             File,
    current_position: u64,
    cutoff_position:  u64,
}

crate struct PipeChild {
    child: ::std::process::Child,
    inp:   BufferedTransport,
    out:   BufferedTransport,
    err:   BufferedTransport,
}

#[derive(Default)]
crate struct OxyInternal {
    naked_transport: RefCell<Option<MessageTransport>>,
    underlying_transport: RefCell<Option<ProtocolTransport>>,
    peer_name: RefCell<Option<String>>,
    piped_children: RefCell<HashMap<u64, PipeChild>>,
    ui: RefCell<Option<Ui>>,
    outgoing_ticker: RefCell<u64>,
    incoming_ticker: RefCell<u64>,
    transfers_out: RefCell<Vec<TransferOut>>,
    port_binds: RefCell<HashMap<u64, PortBind>>,
    local_streams: RefCell<HashMap<u64, PortStream>>,
    remote_streams: RefCell<HashMap<u64, PortStream>>,
    remote_bind_destinations: RefCell<HashMap<u64, String>>,
    naked_state: RefCell<NakedState>,
    kex_data: RefCell<KexData>,
    socks_binds: RefCell<HashMap<u64, SocksBind>>,
    last_message_seen: RefCell<Option<Instant>>,
    launched: RefCell<bool>,
    response_watchers: RefCell<Vec<Rc<dyn Fn(&OxyMessage, u64) -> bool>>>,
    metacommand_queue: RefCell<Vec<Vec<String>>>,
    is_daemon: RefCell<bool>,
    post_auth_hooks: RefCell<Vec<Rc<dyn Fn() -> ()>>>,
    send_hooks: RefCell<Vec<Rc<dyn Fn() -> bool>>>,
    pipecmd_reference: RefCell<Option<u64>>,
    stdin_bt: RefCell<Option<BufferedTransport>>,
    remote_bind_cleaners: RefCell<HashMap<u64, Rc<dyn Fn() -> ()>>>,
    socks_bind_cleaners: RefCell<HashMap<String, Rc<dyn Fn() -> ()>>>,
    local_bind_cleaners: RefCell<HashMap<String, Rc<dyn Fn() -> ()>>>,
    kr_references: RefCell<HashMap<String, u64>>,
    peer_user: RefCell<Option<String>>,
    message_claim: RefCell<bool>,
    privs_dropped: RefCell<bool>,
    #[cfg(unix)]
    pty: RefCell<Option<Pty>>,
    #[cfg(unix)]
    tuntaps: RefCell<HashMap<u64, TunTap>>,
}

impl Oxy {
    fn alice_only(&self) {
        if !(self.perspective() == Alice) {
            error!("The peer sent a message that is only acceptable for a server to send to a client, but I am not a client");
            ::std::process::exit(1);
        }
    }

    fn bob_only(&self) {
        if !(self.perspective() == Bob) {
            error!("The peer sent a message that is only acceptable for a client to send to a server, but I am not a server");
            ::std::process::exit(1);
        }
    }

    fn perspective(&self) -> transportation::EncryptionPerspective {
        crate::arg::perspective()
    }

    pub fn create<T: Into<BufferedTransport>>(transport: T) -> Oxy {
        let bt: BufferedTransport = transport.into();
        let mt = <MessageTransport as From<BufferedTransport>>::from(bt);
        let internal = OxyInternal::default();
        *internal.naked_transport.borrow_mut() = Some(mt);
        *internal.last_message_seen.borrow_mut() = Some(Instant::now());
        let x = Oxy { internal: Rc::new(internal) };
        let proxy = x.clone();
        x.internal
            .naked_transport
            .borrow_mut()
            .as_mut()
            .unwrap()
            .set_notify(Rc::new(move || proxy.notify_naked()));
        let y = x.clone();
        set_timeout(Rc::new(move || y.notify_keepalive()), Duration::from_secs(60));
        let y = x.clone();
        transportation::set_timeout(Rc::new(move || y.launch()), Duration::from_secs(0));
        x
    }

    pub fn set_peer_name(&self, name: &str) {
        trace!("Setting peer name to {:?}", name);
        *self.internal.peer_name.borrow_mut() = Some(name.to_string());
    }

    pub fn set_daemon(&self) {
        *self.internal.is_daemon.borrow_mut() = true;
    }

    pub fn push_post_auth_hook(&self, callback: Rc<dyn Fn() -> ()>) {
        if self.is_encrypted() {
            (callback)();
        } else {
            self.internal.post_auth_hooks.borrow_mut().push(callback);
        }
    }

    pub fn push_send_hook(&self, callback: Rc<dyn Fn() -> bool>) {
        self.internal.send_hooks.borrow_mut().push(callback);
    }

    crate fn queue_metacommand(&self, command: Vec<String>) {
        self.internal.metacommand_queue.borrow_mut().push(command);
    }

    fn pop_metacommand(&self) {
        if !self.internal.metacommand_queue.borrow().is_empty() {
            self.handle_metacommand(self.internal.metacommand_queue.borrow_mut().remove(0));
        }
    }

    fn create_ui(&self) {
        if self.perspective() == Bob {
            return;
        }
        #[cfg(unix)]
        {
            if !self.interactive() {
                return;
            }
        }

        *self.internal.ui.borrow_mut() = Some(Ui::create());
        let proxy = self.clone();
        let proxy = Rc::new(move || proxy.notify_ui());
        self.internal.ui.borrow().as_ref().unwrap().set_notify(proxy);
    }

    fn is_encrypted(&self) -> bool {
        self.internal.underlying_transport.borrow().is_some()
    }

    fn launch(&self) {
        trace!("Launching");
        #[cfg(unix)]
        {
            let proxy = self.clone();
            crate::exit::push_hook(move || {
                if let Some(x) = proxy.internal.ui.borrow_mut().as_ref() {
                    x.cooked()
                };
                crate::ui::cleanup();
                if proxy.internal.pty.borrow().is_some() {
                    use nix::sys::signal::{kill, Signal::*};
                    kill(proxy.internal.pty.borrow().as_ref().unwrap().child_pid, SIGTERM).ok();
                }
                if proxy.perspective() == Alice && !*proxy.internal.is_daemon.borrow() {
                    eprint!("\r");
                    info!("Goodbye!");
                }
            });
        }
        if *self.internal.launched.borrow() {
            panic!("Attempted to launch an Oxy instance twice.");
        }
        *self.internal.launched.borrow_mut() = true;
        if self.perspective() == Alice {
            self.advertise_client_key();
        }
        if self.perspective() == Bob {
            *self.internal.naked_state.borrow_mut() = NakedState::WaitingForClientKey;
        }
    }

    pub fn run<T: Into<BufferedTransport>>(transport: T) -> ! {
        Oxy::create(transport);
        transportation::run();
    }

    pub fn send(&self, message: OxyMessage) -> u64 {
        let message_number = self.tick_outgoing();
        debug!("Sending message {}", message_number);
        trace!("Sending message {}: {:?}", message_number, message);
        if self.internal.underlying_transport.borrow().is_none() {
            error!("Attempted to send protocol message before key-exchange completed.");
            crate::exit::exit(1);
        }
        self.internal.underlying_transport.borrow().as_ref().unwrap().send(message);
        message_number
    }

    #[cfg(unix)]
    pub fn notify_tuntap(&self, reference_number: u64) {
        let borrow = self.internal.tuntaps.borrow_mut();
        let tuntap = borrow.get(&reference_number).unwrap();
        for packet in tuntap.get_packets() {
            self.send(TunnelData {
                reference: reference_number,
                data:      packet,
            });
        }
    }

    fn notify_bind(&self, token: u64) {
        let stream = self.internal.port_binds.borrow_mut().get_mut(&token).unwrap().listener.accept().unwrap();
        let remote_addr = self.internal.port_binds.borrow_mut().get_mut(&token).unwrap().remote_spec.clone();
        let local_addr = self.internal.port_binds.borrow_mut().get_mut(&token).unwrap().local_spec.clone();
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
        self.internal.local_streams.borrow_mut().insert(stream_token, stream);
    }

    fn notify_ui(&self) {
        use crate::ui::UiMessage::*;
        while let Some(msg) = self.internal.ui.borrow().as_ref().unwrap().recv() {
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
        let data = self.internal.pty.borrow_mut().as_mut().unwrap().underlying.take();
        debug!("PTY Data: {:?}", data);
        if !data.is_empty() {
            self.send(PtyOutput { data });
        }
    }

    fn tick_outgoing(&self) -> u64 {
        let message_number = *self.internal.outgoing_ticker.borrow_mut();
        let next = message_number.checked_add(1).unwrap();
        *self.internal.outgoing_ticker.borrow_mut() = next;
        message_number
    }

    fn tick_incoming(&self) -> u64 {
        let message_number = *self.internal.incoming_ticker.borrow_mut();
        let next = message_number.checked_add(1).unwrap();
        *self.internal.incoming_ticker.borrow_mut() = next;
        message_number
    }

    pub fn has_write_space(&self) -> bool {
        self.internal.underlying_transport.borrow().as_ref().unwrap().has_write_space()
    }

    fn service_transfers(&self) {
        if !self.has_write_space() {
            debug!("Write buffer full! Holding off on servicing transfers.");
            return;
        }
        let mut to_remove = Vec::new();
        for TransferOut {
            reference,
            file,
            current_position,
            cutoff_position,
        } in self.internal.transfers_out.borrow_mut().iter_mut()
        {
            debug!("Servicing transfer {}", reference);
            let mut data = [0; 16384];
            let amt = file.read(&mut data[..]).unwrap();
            if *current_position + amt as u64 > *cutoff_position {
                let to_take = (*cutoff_position - *current_position) as usize;
                self.send(FileData {
                    reference: *reference,
                    data:      data[..to_take].to_vec(),
                });
                self.send(FileData {
                    reference: *reference,
                    data:      Vec::new(),
                });
                self.paint_progress_bar(1000, 0);
                self.log_info("File transfer completed");
                debug!("Transfer finished with cutoff: {}", reference);
                to_remove.push(*reference);
                continue;
            }
            if amt == 0 {
                self.paint_progress_bar(1000, 0);
                self.log_info("File transfer completed.");
                debug!("Transfer finished: {}", reference);
                to_remove.push(*reference);
            }
            self.send(FileData {
                reference: *reference,
                data:      data[..amt].to_vec(),
            });
            *current_position += amt as u64;
            if *cutoff_position != 0 {
                self.paint_progress_bar((*current_position * 1000) / *cutoff_position, amt as u64);
            } else {
                self.paint_progress_bar(1000, 0);
            }
        }
        self.internal.transfers_out.borrow_mut().retain(|x| !to_remove.contains(&x.reference));
        if !to_remove.is_empty() {
            self.pop_metacommand();
        }
    }

    fn paint_progress_bar(&self, progress: u64, bytes: u64) {
        self.internal.ui.borrow().as_ref().map(|x| x.paint_progress_bar(progress, bytes));
    }

    fn log_info(&self, message: &str) {
        if let Some(x) = self.internal.ui.borrow().as_ref() {
            x.log_info(message);
        } else {
            info!("{}", message);
        }
    }

    fn log_debug(&self, message: &str) {
        if let Some(x) = self.internal.ui.borrow().as_ref() {
            x.log_debug(message);
        } else {
            debug!("{}", message);
        }
    }

    fn log_warn(&self, message: &str) {
        if let Some(x) = self.internal.ui.borrow().as_ref() {
            x.log_warn(message);
        } else {
            warn!("{}", message);
        }
    }

    fn notify_pipe_child(&self, token: u64) {
        if let Some(child) = self.internal.piped_children.borrow_mut().get_mut(&token) {
            let out = child.out.take();
            let err = child.err.take();
            self.send(PipeCommandOutput {
                reference: token,
                stdout:    out,
                stderr:    err,
            });
        }
    }

    fn notify_local_stream(&self, token: u64) {
        debug!("Local stream notify for stream {}", token);
        let data = self.internal.local_streams.borrow_mut().get_mut(&token).unwrap().stream.take();
        self.send(RemoteStreamData { reference: token, data });
        if self.internal.local_streams.borrow_mut().get_mut(&token).unwrap().stream.is_closed() {
            self.internal.local_streams.borrow_mut().get_mut(&token).unwrap().stream.close();
            self.send(RemoteStreamClosed { reference: token });
            debug!("Stream closed");
        }
    }

    fn notify_remote_stream(&self, token: u64) {
        debug!("Remote stream notify for stream {}", token);
        let data = self.internal.remote_streams.borrow_mut().get_mut(&token).unwrap().stream.take();
        self.send(LocalStreamData { reference: token, data });
        if self.internal.remote_streams.borrow_mut().get_mut(&token).unwrap().stream.is_closed() {
            self.internal.remote_streams.borrow_mut().get_mut(&token).unwrap().stream.close();
            debug!("Stream closed.");
            self.send(LocalStreamClosed { reference: token });
        }
    }

    fn upgrade_to_encrypted(&self) {
        if self.is_encrypted() {
            return;
        }
        debug!("Activating encryption.");
        let transport = self.internal.naked_transport.borrow_mut().take().unwrap();
        let bt: BufferedTransport = match transport {
            MessageTransport::BufferedTransport(bt) => bt,
            _ => panic!(),
        };
        let mut key = self.internal.kex_data.borrow_mut().keymaterial.as_ref().unwrap().to_vec();
        let peer = self.internal.peer_name.borrow().clone();
        key.extend(keys::static_key(peer.as_ref().map(|x| &**x)));
        let et = EncryptedTransport::create(bt, self.perspective(), &key);
        let pt = ProtocolTransport::create(et);
        let proxy = self.clone();
        pt.set_notify(Rc::new(move || proxy.notify_main_transport()));
        *self.internal.underlying_transport.borrow_mut() = Some(pt);
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
                if self.perspective() == Alice && self.internal.ui.borrow().is_some() {
                    let (w, h) = self.internal.ui.borrow_mut().as_mut().unwrap().pty_size();
                    self.send(PtySizeAdvertisement { w, h });
                }
            }
            "SIGCHLD" => {
                info!("Received SIGCHLD");
                if self.internal.pty.borrow().is_some() {
                    let ptypid = self.internal.pty.borrow().as_ref().unwrap().child_pid;
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
                let mut to_remove = Vec::new();
                for (k, pipe_child) in self.internal.piped_children.borrow_mut().iter_mut() {
                    if let Ok(result) = pipe_child.child.try_wait() {
                        debug!("Pipe child exited. {:?}", result);
                        to_remove.push(*k);
                        self.send(PipeCommandExited { reference: *k });
                    }
                }
                for k in to_remove {
                    self.internal.piped_children.borrow_mut().remove(&k);
                }
            }
            _ => (),
        };
    }

    fn interactive(&self) -> bool {
        ::termion::is_tty(&::std::io::stdout()) && ::termion::is_tty(&::std::io::stdin())
    }

    fn do_post_auth(&self) {
        if self.perspective() == Alice {
            self.pop_metacommand();
            self.activate_compression();
            if !*self.internal.is_daemon.borrow() {
                self.run_batched_metacommands();
                #[cfg(unix)]
                {
                    if self.interactive() {
                        if let Ok(term) = ::std::env::var("TERM") {
                            self.send(EnvironmentAdvertisement {
                                key:   "TERM".to_string(),
                                value: term,
                            });
                        }
                        let mut cmd = vec!["pty".to_string()];
                        if let Some(command) = crate::arg::matches().value_of("command") {
                            cmd.push(command.to_string());
                        }
                        self.handle_metacommand(cmd);
                    } else {
                        if let Some(cmd) = crate::arg::matches().value_of("command") {
                            let stdin_bt = BufferedTransport::from(0);
                            let proxy = self.clone();
                            stdin_bt.set_notify(Rc::new(move || {
                                proxy.notify_pipe_stdin();
                            }));
                            *self.internal.stdin_bt.borrow_mut() = Some(stdin_bt);
                            self.handle_metacommand(vec!["pipe".to_string(), cmd.to_string()]);
                        }
                    }
                }
                self.create_ui();
            }
        }
        #[cfg(unix)]
        self.register_signal_handler();
        let mut hooks = Vec::new();
        ::std::mem::swap(&mut hooks, &mut *self.internal.post_auth_hooks.borrow_mut());
        for hook in hooks {
            (hook)();
        }
    }

    fn activate_compression(&self) {
        if crate::arg::matches().is_present("compression") {
            // This v is intended to block compression for via forwarders, because they'll
            // just be handling encrypted data, which isn't very compressible
            if !*self.internal.is_daemon.borrow() || crate::arg::mode() == "copy" {
                self.send(CompressionRequest { compression_type: 0 });
            }
        }
    }

    fn notify_pipe_stdin(&self) {
        if !self.has_write_space() {
            return;
        }
        if self.internal.stdin_bt.borrow().is_none() {
            return;
        }
        let closed = self.internal.stdin_bt.borrow_mut().as_mut().unwrap().is_closed();
        let available2 = self.internal.stdin_bt.borrow_mut().as_mut().unwrap().available();
        let available = if available2 > 8192 { 8192 } else { available2 };
        if available == 0 && !closed {
            return;
        }
        debug!("Processing stdin data {}", available);
        let input = self.internal.stdin_bt.borrow_mut().as_mut().unwrap().take_chunk(available).unwrap();
        if closed && available == 0 {
            self.internal.stdin_bt.borrow_mut().take();
        }
        let reference = self.internal.pipecmd_reference.borrow_mut().unwrap();
        self.send(PipeCommandInput { reference, input });
    }

    fn run_batched_metacommands(&self) {
        if let Some(user) = arg::matches().value_of("user") {
            self.send(UsernameAdvertisement { username: user.to_string() });
        }
        for command in arg::batched_metacommands() {
            let parts = shlex::split(&command).unwrap();
            self.handle_metacommand(parts);
        }
        let ls = arg::matches().values_of("local port forward");
        if ls.is_some() {
            for l in ls.unwrap() {
                self.handle_metacommand(vec!["L".to_string(), l.to_string()]);
            }
        }
        let rs = arg::matches().values_of("remote port forward");
        if rs.is_some() {
            for r in rs.unwrap() {
                self.handle_metacommand(vec!["R".to_string(), r.to_string()]);
            }
        }
        let ds = arg::matches().values_of("socks");
        if ds.is_some() {
            for d in ds.unwrap() {
                self.handle_metacommand(vec!["D".to_string(), d.to_string()]);
            }
        }
        if arg::matches().is_present("X Forwarding") {
            self.initiate_x_forwarding();
        }
    }

    fn initiate_x_forwarding(&self) {
        warn!(r"X Forwarding counts on xauth to set a good umask. If xauth doesn't set a umask, there's a brief window where someone could steal an xauthority cookie out of /tmp. It sets umask on my system! ¯\_(ツ)_/¯");
        let trust = if arg::matches().is_present("Trusted X Forwarding") {
            "trusted"
        } else {
            "untrusted"
        };
        let xauth = ::std::process::Command::new("xauth")
            .arg("-f")
            .arg("/tmp/xcookie")
            .arg("generate")
            .arg(":0")
            .arg(".")
            .arg(trust)
            .arg("timeout")
            .arg("3600")
            .output();
        if xauth.is_err() {
            warn!("Failed to generate an xauthority cookie");
            return;
        }
        let cookie = ::std::process::Command::new("xauth").arg("-f").arg("/tmp/xcookie").arg("list").output();
        if cookie.is_err() {
            warn!("Failed to retrieve the xauthority cookie");
            ::std::fs::remove_file("/tmp/xcookie").ok();
            return;
        }
        ::std::fs::remove_file("/tmp/xcookie").unwrap();
        let cookie = cookie.unwrap();
        let cookie = String::from_utf8(cookie.stdout.clone());
        if cookie.is_err() {
            warn!("Failed to decode xauth output");
        }
        let cookie = cookie.unwrap();
        let cookie = cookie.rsplit(" ").next().unwrap().to_string();
        debug!("xcookie: {:?}", cookie);
        self.send(AdvertiseXAuth { cookie });
        self.handle_metacommand(vec!["sh".to_string(), "mkdir /tmp/.X11-unix".to_string()]);
        self.handle_metacommand(vec!["R".to_string(), "/tmp/.X11-unix/X10".to_string(), "/tmp/.X11-unix/X0".to_string()]);
    }

    fn notify_keepalive(&self) {
        trace!("Keepalive!");
        if self.internal.last_message_seen.borrow().as_ref().unwrap().elapsed() > Duration::from_secs(180) {
            trace!("Exiting due to lack of keepalives");
            self.exit(2);
        }
        self.send(Ping {});
        let proxy = self.clone();
        set_timeout(Rc::new(move || proxy.notify_keepalive()), Duration::from_secs(60));
    }

    fn notify_socks_bind(&self, token: u64) {
        let mut borrow = self.internal.socks_binds.borrow_mut();
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
                self.internal.local_streams.borrow_mut().insert(reference, stream);
            }
        }
    }

    fn exit(&self, status: i32) -> ! {
        crate::exit::exit(status);
    }

    pub fn watch(&self, callback: Rc<dyn Fn(&OxyMessage, u64) -> bool>) {
        if self.internal.response_watchers.borrow().len() >= 10 {
            debug!("Potential response watcher accumulation detected.");
        }
        self.internal.response_watchers.borrow_mut().push(callback);
    }

    pub fn notify_main_transport(&self) {
        debug!("Core notified. Has write space: {}", self.has_write_space());
        if self.internal.underlying_transport.borrow().as_ref().unwrap().is_closed() {
            eprint!("\n\r");
            self.log_info("Connection loss detected.");
            crate::exit::exit(0);
        }
        loop {
            let message = self.internal.underlying_transport.borrow().as_ref().unwrap().recv_tolerant();
            if message.is_none() {
                break;
            }
            let message = message.unwrap();
            let message_number = self.tick_incoming();
            if message.is_none() {
                self.send(Reject {
                    reference: message_number,
                    note:      "Invalid message".to_string(),
                });
                continue;
            }
            let message = message.unwrap();
            let result = self.handle_message(message, message_number);
            if result.is_err() {
                self.send(Reject {
                    reference: message_number,
                    note:      result.unwrap_err(),
                });
            }
        }
        self.service_transfers();
        self.notify_pipe_stdin();
        let mut orig_send_hooks = self.internal.send_hooks.borrow().clone();
        let orig_send_hooks_len = orig_send_hooks.len();
        orig_send_hooks.retain(|x| !(x)());
        {
            let mut borrow = self.internal.send_hooks.borrow_mut();
            borrow.splice(..orig_send_hooks_len, orig_send_hooks.into_iter());
        }
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
