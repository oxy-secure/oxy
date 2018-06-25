use clap::{self, App, AppSettings, Arg, SubCommand};
#[cfg(unix)]
use crate::tuntap::{TunTap, TunTapType};
use crate::{
    core::{Oxy, PortBind, SocksBind, SocksBindNotificationProxy},
    message::OxyMessage::*,
};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use num;
use std::{
    cell::RefCell,
    fs::{metadata, read_dir, File},
    io::Write,
    path::PathBuf,
    rc::Rc,
};
#[cfg(unix)]
use transportation::mio::unix::EventedFd;
use transportation::{
    self,
    mio::{net::TcpListener, PollOpt, Ready, Token},
    BufferedTransport, Notifies,
};

fn create_app() -> App<'static, 'static> {
    let subcommands = vec![
        SubCommand::with_name("L")
            .about("Create a local portforward.")
            .arg(Arg::with_name("local spec").index(1))
            .arg(Arg::with_name("remote spec").index(2)),
        SubCommand::with_name("R")
            .about("Create a remote portforward.")
            .arg(Arg::with_name("remote spec").index(1))
            .arg(Arg::with_name("local spec").index(2)),
        SubCommand::with_name("download")
            .about("Download a file")
            .arg(Arg::with_name("remote path").help("Remote file path to download from.").index(1))
            .arg(Arg::with_name("local path").help("Local file path to download to.").index(2))
            .arg(Arg::with_name("offset start").long("start").takes_value(true))
            .arg(Arg::with_name("offset end").long("end").takes_value(true)),
        SubCommand::with_name("upload")
            .about("Upload a file")
            .arg(Arg::with_name("local path").index(1))
            .arg(Arg::with_name("remote path").index(2))
            .arg(Arg::with_name("offset start").long("start").takes_value(true)),
        SubCommand::with_name("tun")
            .about("Bridge two tun devices.")
            .long_about(
                "Bridge two tun devices. \
                 Creates the tap devices if both the local and remote are root, but \
                 it's better to create the devices, beforehand with 'ip tuntap create \
                 mode tun user youruser'",
            )
            .arg(Arg::with_name("local tun").index(1))
            .arg(Arg::with_name("remote tun").index(2)),
        SubCommand::with_name("tap")
            .about("Bridge two tap devices.")
            .arg(Arg::with_name("local tap").index(1))
            .arg(Arg::with_name("remote tap").index(2)),
        SubCommand::with_name("socks")
            .about("Bind a local port as a SOCKS5 proxy server")
            .arg(Arg::with_name("bind spec").index(1)),
        SubCommand::with_name("pty")
            .about(
                "Open a remote PTY. \
                 Happens by default, usually not necessary",
            )
            .arg(Arg::with_name("command").index(1).multiple(true)),
        SubCommand::with_name("sh")
            .about(
                "Run a remote basic-command. \
                 Useful for Windows servers.",
            )
            .arg(Arg::with_name("command").index(1).multiple(true)),
        SubCommand::with_name("exit").about("Exits the Oxy client."),
        SubCommand::with_name("f10").about("Send F10 to the remote"),
        SubCommand::with_name("f12").about("Send F12 to the remote"),
        SubCommand::with_name("hash")
            .arg(Arg::with_name("path").index(1).required(true))
            .arg(Arg::with_name("offset start").long("start").takes_value(true))
            .arg(Arg::with_name("offset end").long("end").takes_value(true))
            .about("Request the file hash of a file"),
        SubCommand::with_name("pipe")
            .about("Run a command without a pty.")
            .arg(Arg::with_name("command").index(1).multiple(true)),
        SubCommand::with_name("KL")
            .about("Terminate a local portforward")
            .arg(Arg::with_name("spec").index(1).required(true)),
        SubCommand::with_name("KR")
            .about("Terminate a remote portforward")
            .arg(Arg::with_name("spec").index(1).required(true)),
        SubCommand::with_name("KD")
            .about("Terminate a SOCKS portforward")
            .arg(Arg::with_name("spec").index(1).required(true)),
    ];
    let subcommands: Vec<App<'static, 'static>> = subcommands
        .into_iter()
        .map(|x| x.setting(AppSettings::DisableVersion).setting(AppSettings::DontCollapseArgsInUsage))
        .collect();
    let mut app = App::new("oxy>")
        .setting(AppSettings::NoBinaryName)
        .setting(AppSettings::DisableVersion)
        .setting(AppSettings::SubcommandRequiredElseHelp);
    for subcommand in subcommands {
        app = app.subcommand(subcommand);
    }
    app
}

fn preprocess_parts(mut parts: Vec<String>) -> Vec<String> {
    // TODO: This whole thing is probably not the most robust way to accomplish
    // what it does
    if parts.is_empty() {
        return parts;
    }
    if parts[0].is_empty() {
        return parts;
    }
    {
        let cmd: &mut String = parts.get_mut(0).unwrap();
        if cmd.chars().next().unwrap() == '-' {
            cmd.remove(0);
        }
    }
    if parts[0].as_str() == "D" {
        parts[0] = "socks".to_string();
    }
    if parts[0].as_str() == "L" || parts[0].as_str() == "R" {
        if parts.len() > 2 {
            return parts;
        }
        let spec = parts[1].clone(); // TODO:  This should be the first positional argument, we might have --flags
        let parse_result: Option<u16> = spec.parse().ok();
        if let Some(port) = parse_result {
            return vec![parts[0].clone(), format!("localhost:{}", port), format!("localhost:{}", port)];
        }
        let colon_count = spec.matches(':').count();
        if colon_count == 0 {
            // Maybe spec is a unix socket path that should be used on both sides?
            return vec![parts[0].clone(), spec.clone(), spec.clone()];
        }
        if colon_count == 1 {
            return vec![
                parts[0].clone(),
                spec.split(':').next().unwrap().to_string(),
                spec.split(':').nth(1).unwrap().to_string(),
            ];
        }
        if colon_count == 2 {
            return vec![
                parts[0].clone(),
                spec.splitn(2, ':').next().unwrap().to_string(),
                spec.splitn(2, ':').nth(1).unwrap().to_string(),
            ];
        }
        if colon_count == 3 {
            let first_half = spec.split(':').take(2).collect::<Vec<&str>>().join(":");
            let second_half = spec.split(':').skip(2).collect::<Vec<&str>>().join(":");
            return vec![parts[0].clone(), first_half, second_half];
        }
    }
    parts
}

impl Oxy {
    crate fn handle_metacommand(&self, parts: Vec<String>) {
        let parts = preprocess_parts(parts);
        let matches = create_app().get_matches_from_safe(parts.clone());
        match matches {
            Err(clap::Error { message, .. }) => {
                println!("{}", message);
            }
            Ok(matches2) => {
                let name = matches2.subcommand_name().unwrap();
                let matches = matches2.subcommand_matches(name).unwrap();
                match name {
                    "sh" => {
                        let command = matches.values_of("command");
                        if command.is_none() {
                            self.log_warn("No command provided!");
                            return;
                        }
                        let command: Vec<String> = command.unwrap().map(|x| x.to_string()).collect();
                        self.send(BasicCommand { command: command });
                    }
                    "pty" => {
                        let command = matches.values_of("command");
                        let command = if command.is_none() {
                            None
                        } else {
                            Some(command.unwrap().map(|x| x.to_string()).collect())
                        };
                        let id = self.send(PtyRequest { command: command });
                        let proxy = self.clone();
                        #[cfg(unix)]
                        self.watch(Rc::new(move |message, _| match message {
                            Success { reference } => {
                                if *reference != id {
                                    return false;
                                }
                                if proxy.internal.ui.borrow().is_some() {
                                    let (w, h) = proxy.internal.ui.borrow_mut().as_mut().unwrap().pty_size();
                                    proxy.send(PtySizeAdvertisement { w, h });
                                    return true;
                                }
                                return true;
                            }
                            Reject { reference, .. } => {
                                if *reference != id {
                                    return false;
                                }
                                warn!("PTY open failed");
                                return true;
                            }
                            _ => false,
                        }));
                    }
                    "download" => {
                        let filepart: PathBuf = matches.value_of("remote path").unwrap().to_string().into();
                        let filepart = filepart.file_name().unwrap().to_str().unwrap().to_string();
                        let local_path = matches.value_of("local path").unwrap_or(&filepart).to_string();
                        let remote_path = matches.value_of("remote path").unwrap().to_string();
                        let offset_start = matches.value_of("offset start").map(|x| x.parse().unwrap());
                        let offset_end = matches.value_of("offset end").map(|x| x.parse().unwrap());

                        let id = self.send(StatRequest {
                            path:         remote_path.clone(),
                            follow_links: true,
                        });

                        let proxy = self.clone();
                        self.watch(Rc::new(move |message, _| match message {
                            Reject { reference, note } if *reference == id => {
                                proxy.log_warn(&format!("Download request rejected: {:?}", note));
                                proxy.pop_metacommand();
                                return true;
                            }
                            StatResult { reference, is_dir, len, .. } if *reference == id => {
                                let local_path = local_path.clone();
                                let remote_path = remote_path.clone();
                                let proxy = proxy.clone();
                                let len = *len;
                                if *is_dir {
                                    proxy.log_info("Trying to download a directory");
                                    let id = proxy.send(ReadDir { path: remote_path.clone() });
                                    proxy.clone().watch(Rc::new(move |message, _| match message {
                                        ReadDirResult {
                                            reference,
                                            complete,
                                            answers,
                                        } if *reference == id =>
                                        {
                                            for answer in answers {
                                                let mut new_remote_path: PathBuf = remote_path.clone().into();
                                                new_remote_path.push(answer);
                                                let mut new_local_path: PathBuf = local_path.clone().into();
                                                new_local_path.push(answer);
                                                let mut next = Vec::new();
                                                next.push("download".to_string());
                                                next.push(new_remote_path.to_str().unwrap().to_string());
                                                next.push(new_local_path.to_str().unwrap().to_string());
                                                proxy.queue_metacommand(next);
                                            }
                                            if *complete {
                                                proxy.pop_metacommand();
                                            }
                                            return *complete;
                                        }
                                        Reject { reference, note } if *reference == id => {
                                            proxy.log_warn(&format!("Failed to read remote directory: {:?}", note));
                                            proxy.pop_metacommand();
                                            return true;
                                        }
                                        _ => false,
                                    }));
                                    return true;
                                } else {
                                    let id = proxy.send(DownloadRequest {
                                        path:         remote_path.clone(),
                                        offset_start: offset_start,
                                        offset_end:   offset_end,
                                    });
                                    proxy.clone().watch(Rc::new(move |message, _| match message {
                                        Reject { reference, note } if *reference == id => {
                                            let proxy = proxy.clone();
                                            proxy.log_warn(&format!("Download request rejected: {:?}", note));
                                            proxy.pop_metacommand();
                                            return true;
                                        }
                                        Success { reference } if *reference == id => {
                                            let local_path: PathBuf = local_path.clone().into();
                                            let proxy = proxy.clone();
                                            let len: u64 = len.clone();
                                            if let Some(parent) = local_path.parent() {
                                                ::std::fs::create_dir_all(parent).ok();
                                            }
                                            let file = File::create(&local_path);
                                            if file.is_err() {
                                                proxy.log_warn(&format!("Failed to open local file for writing: {:?}", local_path));
                                                proxy.pop_metacommand();
                                                return true;
                                            }
                                            let file = Rc::new(RefCell::new(file.unwrap()));
                                            let downloaded_bytes = Rc::new(RefCell::new(0u64));
                                            proxy.log_info("Download started.");
                                            proxy.clone().watch(Rc::new(move |message, _| match message {
                                                FileData { reference, data } if *reference == id => {
                                                    let file = file.clone();
                                                    let proxy = proxy.clone();
                                                    let downloaded_bytes = downloaded_bytes.clone();
                                                    if data.is_empty() {
                                                        proxy.log_info("Download finished");
                                                        proxy.pop_metacommand();
                                                        return true;
                                                    } else {
                                                        let result = file.borrow_mut().write_all(&data[..]);
                                                        if result.is_err() {
                                                            proxy.log_warn("Failed writing download data to file");
                                                            proxy.pop_metacommand();
                                                            return true;
                                                        }
                                                        *downloaded_bytes.borrow_mut() += data.len() as u64;
                                                        let a = *downloaded_bytes.borrow();
                                                        let progress = (a * 1000) / len;
                                                        proxy.paint_progress_bar(progress, data.len() as u64);
                                                        return false;
                                                    }
                                                }
                                                _ => false,
                                            }));
                                            return true;
                                        }
                                        _ => false,
                                    }));
                                    return true;
                                }
                            }
                            _ => false,
                        }));
                    }
                    "upload" => {
                        let buf: PathBuf = matches.value_of("local path").unwrap().into();
                        let buf = buf.canonicalize().unwrap();
                        let remote_path = matches.value_of("remote path").unwrap_or("").to_string();

                        let metadata = metadata(&buf);
                        if metadata.is_err() {
                            self.log_warn("Failed to stat path for upload");
                            return;
                        }
                        let metadata = metadata.unwrap();
                        if metadata.is_dir() {
                            let dents = read_dir(&buf);
                            if dents.is_err() {
                                self.log_warn(&format!("Failed to read directory for upload. {:?}", buf));
                                return;
                            }
                            let dents = dents.unwrap();
                            for entry in dents {
                                let mut new_local = buf.clone();
                                new_local.push(entry.unwrap().file_name());
                                let mut new_remote: PathBuf = remote_path.clone().into();
                                new_remote.push(buf.file_name().unwrap());
                                let mut next = Vec::new();
                                next.push("upload".to_string());
                                next.push(new_local.to_str().unwrap().to_string());
                                next.push(new_remote.to_str().unwrap().to_string());
                                self.queue_metacommand(next);
                            }
                            self.pop_metacommand();
                            return;
                        }

                        let file = File::open(buf.clone());
                        if file.is_err() {
                            error!("Failed to open local file for reading: {}", matches.value_of("local path").unwrap());
                            return;
                        }
                        let file = file.unwrap();
                        let id = self.send(UploadRequest {
                            path:         remote_path,
                            filepart:     buf.file_name().unwrap().to_string_lossy().into_owned(),
                            offset_start: matches.value_of("offset start").map(|x| x.parse().unwrap()),
                        });
                        let file = Rc::new(RefCell::new(Some(file)));
                        let proxy = self.clone();
                        self.watch(Rc::new(move |message, _| match message {
                            Success { reference } if *reference == id => {
                                let len = file.borrow().as_ref().unwrap().metadata().unwrap().len();
                                proxy.log_info("Upload started");
                                proxy.internal.transfers_out.borrow_mut().push(super::TransferOut {
                                    reference:        id,
                                    file:             file.borrow_mut().take().unwrap(),
                                    current_position: 0,
                                    cutoff_position:  len,
                                });
                                return true;
                            }
                            Reject { reference, note } if *reference == id => {
                                proxy.log_warn(&format!("Upload rejected: {:?}", note));
                                return true;
                            }
                            _ => false,
                        }));
                    }
                    "KL" => {
                        let mut spec = matches.value_of("spec").unwrap().to_string();
                        if !spec.contains(":") && !spec.contains("/") {
                            spec = format!("localhost:{}", spec);
                        }
                        let cleaner = self.internal.local_bind_cleaners.borrow_mut().remove(&spec);
                        if cleaner.is_none() {
                            self.log_warn("Could not find local forward to close.");
                            return;
                        }
                        (cleaner.unwrap())();
                        self.log_info("Port forward closed.");
                    }
                    "L" => {
                        let remote_spec = matches.value_of("remote spec").unwrap().to_string();
                        let mut local_spec = matches.value_of("local spec").unwrap().to_string();
                        if !local_spec.contains(':') && !local_spec.contains('/') {
                            local_spec = format!("localhost:{}", local_spec);
                        }
                        if local_spec.contains('/') {
                            use nix::sys::socket::{accept, bind, listen, socket, AddressFamily, SockAddr, SockFlag, SockType};
                            let socket = socket(AddressFamily::Unix, SockType::Stream, SockFlag::empty(), None);
                            if socket.is_err() {
                                self.log_warn("Failed to create socket");
                                return;
                            }
                            let socket = socket.unwrap();
                            let path = PathBuf::from(local_spec.clone());
                            let sockaddr = SockAddr::new_unix(&path);
                            if sockaddr.is_err() {
                                self.log_warn("Failed to parse socket address");
                                return;
                            }
                            let sockaddr = sockaddr.unwrap();
                            let bind_result = bind(socket, &sockaddr);
                            if bind_result.is_err() {
                                self.log_warn(&format!("Failed to bind {}", local_spec));
                            }
                            let listen_result = listen(socket, 10);
                            if listen_result.is_err() {
                                self.log_warn(&format!("Failed to listen {}", local_spec));
                            }
                            let token = Rc::new(RefCell::new(0));
                            let token2 = token.clone();
                            let proxy = self.clone();
                            let local_spec2 = local_spec.clone();
                            let token3 = transportation::insert_listener(Rc::new(move || {
                                let token = token.clone();
                                let peer = accept(socket);
                                if peer.is_err() {
                                    proxy.log_warn(&format!("Failed to accept connection on {}", local_spec));
                                    transportation::remove_listener(*token.borrow());
                                    return;
                                }
                                let peer = peer.unwrap();
                                let stream_token = proxy.send(RemoteOpen {
                                    addr: remote_spec.to_string(),
                                });
                                let bt = BufferedTransport::from(peer);
                                let tracker = super::PortStream {
                                    stream: bt,
                                    token:  stream_token,
                                    oxy:    proxy.clone(),
                                    local:  true,
                                };
                                let tracker2 = Rc::new(tracker.clone());
                                tracker.stream.set_notify(tracker2);
                                proxy.internal.local_streams.borrow_mut().insert(stream_token, tracker);
                            }));
                            *token2.borrow_mut() = token3;
                            transportation::borrow_poll(|poll| {
                                poll.register(&EventedFd(&socket), Token(token3), Ready::readable(), PollOpt::level())
                                    .unwrap()
                            });
                            self.log_info("Forwarding port");
                            self.internal.local_bind_cleaners.borrow_mut().insert(
                                local_spec2,
                                Rc::new(move || {
                                    use nix::unistd::{close, unlink};
                                    transportation::remove_listener(token3);
                                    if close(socket).is_err() {
                                        warn!("Error closing socks socket");
                                    }
                                    if unlink(&path).is_err() {
                                        warn!("Error removing socks socket");
                                    }
                                }),
                            );
                            return;
                        }
                        let bind = ::std::net::TcpListener::bind(&local_spec).unwrap();
                        let bind = TcpListener::from_std(bind).unwrap();
                        let token_holder = Rc::new(RefCell::new(0));
                        let token_holder2 = token_holder.clone();
                        let proxy = self.clone();
                        let proxy = Rc::new(move || proxy.notify_bind(*token_holder2.borrow()));
                        let token = transportation::insert_listener(proxy.clone());
                        let token_sized = <u64 as num::NumCast>::from(token).unwrap();
                        *token_holder.borrow_mut() = token_sized;
                        transportation::borrow_poll(|poll| {
                            poll.register(&bind, Token(token), Ready::readable(), PollOpt::level()).unwrap();
                        });
                        let bind = PortBind {
                            listener: bind,
                            local_spec: local_spec.clone(),
                            remote_spec,
                        };
                        self.internal.port_binds.borrow_mut().insert(token_sized, bind);
                        self.log_info("Forwarding port");
                        let selfproxy = self.clone();
                        self.internal.local_bind_cleaners.borrow_mut().insert(
                            local_spec.clone(),
                            Rc::new(move || {
                                transportation::remove_listener(token);
                                let bind = selfproxy.internal.port_binds.borrow_mut().remove(&token_sized).unwrap();
                                transportation::borrow_poll(|poll| {
                                    poll.deregister(&bind.listener).ok();
                                });
                            }),
                        );
                    }
                    "KR" => {
                        let mut spec = matches.value_of("spec").unwrap().to_string();
                        if !spec.contains(":") && !spec.contains("/") {
                            spec = format!("localhost:{}", spec);
                        }
                        let reference = self.internal.kr_references.borrow_mut().remove(&spec);
                        if reference.is_none() {
                            self.log_warn("Could not find remote forward to close.");
                            return;
                        }
                        let reference = reference.unwrap();
                        self.send(CloseRemoteBind { reference });
                        self.log_info("Remote forward closed");
                    }
                    "R" => {
                        let remote_spec = matches.value_of("remote spec").unwrap().to_string();
                        let bind_id = self.send(RemoteBind { addr: remote_spec.clone() });
                        self.internal.kr_references.borrow_mut().insert(remote_spec, bind_id);
                        self.internal
                            .remote_bind_destinations
                            .borrow_mut()
                            .insert(bind_id, matches.value_of("local spec").unwrap().to_string());
                        self.log_info("Forwarding port");
                    }
                    #[cfg(unix)]
                    "tun" => {
                        let reference_number = self.send(TunnelRequest {
                            tap:  false,
                            name: matches.value_of("remote tun").unwrap().to_string(),
                        });
                        let tuntap = TunTap::create(TunTapType::Tun, matches.value_of("local tun").unwrap(), reference_number, self.clone());
                        self.internal.tuntaps.borrow_mut().insert(reference_number, tuntap);
                    }
                    #[cfg(unix)]
                    "tap" => {
                        let reference_number = self.send(TunnelRequest {
                            tap:  true,
                            name: matches.value_of("remote tap").unwrap().to_string(),
                        });
                        let tuntap = TunTap::create(TunTapType::Tap, matches.value_of("local tap").unwrap(), reference_number, self.clone());
                        self.internal.tuntaps.borrow_mut().insert(reference_number, tuntap);
                    }
                    "KD" => {
                        let mut spec = matches.value_of("spec").unwrap().to_string();
                        if !spec.contains(":") && !spec.contains("/") {
                            spec = format!("localhost:{}", spec);
                        }
                        let cleaner = self.internal.socks_bind_cleaners.borrow_mut().remove(&spec);
                        if cleaner.is_none() {
                            self.log_warn("Could not find SOCKS forward to close.");
                            return;
                        }
                        (cleaner.unwrap())();
                        self.log_info("SOCKS proxy closed.");
                    }
                    "socks" => {
                        let mut local_spec = matches.value_of("bind spec").unwrap().to_string();
                        if !local_spec.contains(':') && !local_spec.contains('/') {
                            local_spec = format!("localhost:{}", local_spec);
                        }
                        if local_spec.contains("/") {
                            use nix::sys::socket::{accept, bind, listen, socket, AddressFamily, SockAddr, SockFlag, SockType};
                            let socket = socket(AddressFamily::Unix, SockType::Stream, SockFlag::empty(), None);
                            if socket.is_err() {
                                self.log_warn("Failed to create socket");
                                return;
                            }
                            let socket = socket.unwrap();
                            let path = PathBuf::from(local_spec.clone());
                            let sockaddr = SockAddr::new_unix(&path);
                            if sockaddr.is_err() {
                                self.log_warn("Failed to parse socket address");
                                return;
                            }
                            let sockaddr = sockaddr.unwrap();
                            let bind_result = bind(socket, &sockaddr);
                            if bind_result.is_err() {
                                self.log_warn(&format!("Failed to bind {}", local_spec));
                            }
                            let listen_result = listen(socket, 10);
                            if listen_result.is_err() {
                                self.log_warn(&format!("Failed to listen {}", local_spec));
                            }
                            let token = Rc::new(RefCell::new(0));
                            let token2 = token.clone();
                            let proxy = self.clone();
                            let local_spec2 = local_spec.clone();
                            let token3 = transportation::insert_listener(Rc::new(move || {
                                let token = token.clone();
                                let peer = accept(socket);
                                if peer.is_err() {
                                    proxy.log_warn(&format!("Failed to accept connection on {}", local_spec));
                                    transportation::remove_listener(*token.borrow());
                                    return;
                                }
                                proxy.log_info("Accepted SOCKS connection");
                                let peer = peer.unwrap();
                                let bt = BufferedTransport::from(peer);
                                let sproxy = super::SocksConnectionNotificationProxy {
                                    oxy: proxy.clone(),
                                    bt,
                                    state: Rc::new(RefCell::new(super::SocksState::Initial)),
                                };
                                let sproxy = Rc::new(sproxy);
                                sproxy.bt.set_notify(sproxy.clone());
                            }));
                            *token2.borrow_mut() = token3;
                            transportation::borrow_poll(|poll| {
                                poll.register(&EventedFd(&socket), Token(token3), Ready::readable(), PollOpt::level())
                                    .unwrap()
                            });
                            self.internal.socks_bind_cleaners.borrow_mut().insert(
                                local_spec2,
                                Rc::new(move || {
                                    use nix::unistd::{close, unlink};
                                    transportation::remove_listener(token3);
                                    if close(socket).is_err() {
                                        warn!("Error closing socks socket");
                                    }
                                    if unlink(&path).is_err() {
                                        warn!("Error removing socks socket");
                                    }
                                }),
                            );
                            return;
                        }
                        let bind = ::std::net::TcpListener::bind(&local_spec).unwrap();
                        let bind = TcpListener::from_std(bind).unwrap();
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
                        self.internal.socks_binds.borrow_mut().insert(token_sized, socks);
                        self.log_info("SOCKS proxy established.");
                        let selfproxy = self.clone();
                        self.internal.socks_bind_cleaners.borrow_mut().insert(
                            local_spec.clone(),
                            Rc::new(move || {
                                transportation::remove_listener(token);
                                let bind = selfproxy.internal.socks_binds.borrow_mut().remove(&token_sized).unwrap();
                                transportation::borrow_poll(|poll| {
                                    poll.deregister(&bind.listener).ok();
                                });
                            }),
                        );
                    }
                    "exit" => {
                        ::std::process::exit(0);
                    }
                    "f10" => {
                        let f10 = [27, 91, 50, 49, 126];
                        self.send(PtyInput { data: f10.to_vec() });
                    }
                    "f12" => {
                        let f12 = [27, 91, 50, 52, 126];
                        self.send(PtyInput { data: f12.to_vec() });
                    }
                    "hash" => {
                        self.send(FileHashRequest {
                            path:           matches.value_of("path").unwrap().to_string(),
                            offset_start:   matches.value_of("offset start").map(|x| x.parse().unwrap()),
                            offset_end:     matches.value_of("offset end").map(|x| x.parse().unwrap()),
                            hash_algorithm: 3,
                        });
                    }
                    "pipe" => {
                        let command = matches.values_of("command");
                        if command.is_none() {
                            self.log_warn("No command provided!");
                            return;
                        }
                        let command: Vec<String> = command.unwrap().map(|x| x.to_string()).collect();
                        let reference = self.send(PipeCommand { command });
                        *self.internal.pipecmd_reference.borrow_mut() = Some(reference);
                    }
                    _ => (),
                }
            }
        }
    }
}
