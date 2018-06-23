use super::{PortBind, PortStream};
#[cfg(unix)]
use crate::pty::Pty;
#[cfg(unix)]
use crate::tuntap::{TunTap, TunTapType};
use crate::{
    arg::perspective,
    core::Oxy,
    message::OxyMessage::{self, *},
};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::{
    cell::RefCell,
    fs::{read_dir, symlink_metadata, File},
    io::Write,
    net::ToSocketAddrs,
    path::PathBuf,
    rc::Rc,
    time::Instant,
};
#[cfg(unix)]
use transportation::mio::unix::EventedFd;
use transportation::{
    self,
    mio::{
        net::{TcpListener, TcpStream},
        PollOpt, Ready, Token,
    },
    BufferedTransport,
    EncryptionPerspective::{Alice, Bob},
    Notifies,
};

impl Oxy {
    fn dispatch_watchers(&self, message: &OxyMessage, message_number: u64) {
        let mut hot_watchers = (*self.internal.response_watchers.borrow()).clone();
        let start_len = hot_watchers.len();
        hot_watchers.retain(|x| !(x)(message, message_number));
        let mut borrow = self.internal.response_watchers.borrow_mut();
        borrow.splice(..start_len, hot_watchers.into_iter());
    }

    crate fn claim_message(&self) {
        *self.internal.message_claim.borrow_mut() = true;
    }

    fn qualify_path(&self, path: String) -> PathBuf {
        let mut path: PathBuf = path.into();
        if !path.is_absolute() && self.internal.pty.borrow_mut().is_some() {
            let mut base_path: PathBuf = self.internal.pty.borrow_mut().as_mut().unwrap().get_cwd().into();
            base_path.push(path);
            path = base_path;
        }
        path
    }

    pub(crate) fn handle_message(&self, message: OxyMessage, message_number: u64) -> Result<(), String> {
        debug!("Recieved message {}", message_number);
        trace!("Received message {}: {:?}", message_number, message);
        let message = self.restrict_message(message).map_err(|_| "Permission denied")?;
        *self.internal.message_claim.borrow_mut() = false;
        self.dispatch_watchers(&message, message_number);
        if *self.internal.message_claim.borrow() {
            return Ok(());
        }
        match message {
            DummyMessage { .. } => (),
            Reject { note, .. } => {
                let message = format!("Server rejected a request: {:?}", note);
                self.log_debug(&message);
            }
            Ping {} => {
                self.send(Pong {});
            }
            Pong {} => {
                *self.internal.last_message_seen.borrow_mut() = Some(Instant::now());
            }
            Exit {} => {
                crate::exit::exit(0);
            }
            UsernameAdvertisement { username } => {
                self.bob_only();
                *self.internal.peer_user.borrow_mut() = Some(username);
            }
            EnvironmentAdvertisement { key, value } => {
                self.bob_only();
                if key.as_str() != "TERM" {
                    Err("Unsupported")?;
                }
                ::std::env::set_var(key, value);
            }
            BasicCommand { command } => {
                self.bob_only();
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
            CompressionRequest { compression_type } => {
                self.bob_only();
                if compression_type != 0 {
                    Err("Unsupported compression algorithm")?;
                }
                let outbound_compression: bool = self
                    .internal
                    .underlying_transport
                    .borrow()
                    .as_ref()
                    .expect("Shouldn't happen")
                    .outbound_compression;
                if !outbound_compression {
                    debug!("Activating compression");
                    self.send(CompressionStart { compression_type: 0 });
                    self.internal
                        .underlying_transport
                        .borrow_mut()
                        .as_mut()
                        .expect("Shouldn't happen")
                        .outbound_compression = true;
                }
            }
            CompressionStart { compression_type } => {
                if compression_type != 0 {
                    panic!("Unknown compression algorithm");
                }
                self.internal
                    .underlying_transport
                    .borrow_mut()
                    .as_mut()
                    .expect("Shouldn't happen")
                    .inbound_compression = true;
                if !self
                    .internal
                    .underlying_transport
                    .borrow()
                    .as_ref()
                    .expect("Shouldn't happen")
                    .outbound_compression
                {
                    debug!("Activating compression.");
                    self.send(CompressionStart { compression_type: 0 });
                    self.internal
                        .underlying_transport
                        .borrow_mut()
                        .as_mut()
                        .expect("Shouldn't happen")
                        .outbound_compression = true;
                }
            }
            PipeCommand { command } => {
                self.bob_only();
                use std::process::Stdio;
                #[cfg(unix)]
                let sh = "/bin/sh";
                #[cfg(unix)]
                let flag = "-c";
                #[cfg(windows)]
                let sh = "cmd.exe";
                #[cfg(windows)]
                let flag = "/c";
                let mut result = ::std::process::Command::new(sh)
                    .arg(flag)
                    .arg(command)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .stdin(Stdio::piped())
                    .spawn()
                    .map_err(|x| format!("Spawn failed: {:?}", x))?;
                use std::os::unix::io::IntoRawFd;
                let inp = BufferedTransport::from(result.stdin.take().unwrap().into_raw_fd());
                let out = BufferedTransport::from(result.stdout.take().unwrap().into_raw_fd());
                let err = BufferedTransport::from(result.stderr.take().unwrap().into_raw_fd());
                let proxy = self.clone();
                let callback = Rc::new(move || {
                    proxy.notify_pipe_child(message_number);
                });
                out.set_notify(callback.clone());
                err.set_notify(callback.clone());
                let child = super::PipeChild {
                    child: result,
                    inp,
                    out,
                    err,
                };
                self.internal.piped_children.borrow_mut().insert(message_number, child);
            }
            PipeCommandInput { reference, input } => {
                self.bob_only();
                if input.is_empty() {
                    debug!("Recieved pipe EOF");
                    self.internal
                        .piped_children
                        .borrow_mut()
                        .get_mut(&reference)
                        .ok_or("Invalid reference")?
                        .inp
                        .close();
                    return Ok(());
                }
                self.internal
                    .piped_children
                    .borrow_mut()
                    .get_mut(&reference)
                    .ok_or("Invalid reference")?
                    .inp
                    .put(&input);
            }
            PipeCommandOutput {
                reference: _,
                stdout,
                stderr,
            } => {
                self.alice_only();
                if !stdout.is_empty() {
                    let a = ::std::io::stdout();
                    let mut lock = a.lock();
                    let status = lock.write_all(&stdout);
                    if status.is_err() {
                        self.log_warn(&format!("Error writing to stdout: {:?}", status));
                    }
                    lock.flush().ok();
                }
                if !stderr.is_empty() {
                    let a = ::std::io::stderr();
                    let mut lock = a.lock();
                    let status = lock.write_all(&stderr);
                    if status.is_err() {
                        self.log_warn(&format!("Error writing to stderr: {:?}", status));
                    }
                    lock.flush().ok();
                }
            }
            AdvertiseXAuth { cookie } => {
                self.bob_only();
                ::std::process::Command::new("xauth")
                    .arg("add")
                    .arg(":10")
                    .arg(".")
                    .arg(&cookie)
                    .output()
                    .map_err(|_| "Xauth failed")?;
                ::std::env::set_var("DISPLAY", ":10");
            }
            PipeCommandExited { reference: _ } => {
                self.alice_only();
                // This is crude and temporary
                // It'd be nice to like... check if we're actually waiting on a pipecommand/if
                // we're doing anything else also
                crate::exit::exit(0);
            }
            #[cfg(unix)]
            PtyRequest { command } => {
                self.bob_only();

                let command2 = command.as_ref().map(|x| x.as_str());

                let pty = Pty::forkpty(command2).map_err(|_| "forkpty failed")?;
                let proxy = self.clone();
                pty.underlying.set_notify(Rc::new(move || proxy.notify_pty()));
                *self.internal.pty.borrow_mut() = Some(pty);
                trace!("Successfully allocated PTY");
                self.send(Success { reference: message_number });
            }
            #[cfg(unix)]
            PtySizeAdvertisement { w, h } => {
                self.bob_only();
                self.internal.pty.borrow_mut().as_mut().ok_or("No PTY exists")?.set_size(w, h);
            }
            #[cfg(unix)]
            PtyInput { data } => {
                self.bob_only();
                self.internal.pty.borrow_mut().as_mut().ok_or("No PTY exists")?.underlying.put(&data[..]);
            }
            #[cfg(unix)]
            PtyOutput { data } => {
                self.alice_only();
                if self.internal.ui.borrow().is_some() {
                    self.internal.ui.borrow_mut().as_mut().unwrap().pty_data(&data);
                } else {
                    let stdout = ::std::io::stdout();
                    let mut lock = stdout.lock();
                    lock.write_all(&data).unwrap();
                }
            }
            BasicCommandOutput { stdout, stderr } => {
                self.alice_only();
                self.log_debug(&format!("BasicCommandOutput {:?}, {:?}", stdout, stderr));
                if let Ok(stdout) = String::from_utf8(stdout) {
                    self.log_debug(&format!("stdout:\n-----\n{}\n-----", stdout));
                }
            }
            DownloadRequest {
                path,
                offset_start,
                offset_end,
            } => {
                use std::io::{Seek, SeekFrom};
                self.bob_only();
                let path = self.qualify_path(path);
                let mut file = File::open(path).map_err(|_| "Failed to open file")?;
                if let Some(offset_start) = offset_start {
                    file.seek(SeekFrom::Start(offset_start)).map_err(|_| "Start-seek failed")?;
                }
                let metadata = file.metadata().unwrap();
                let offset_end = offset_end.unwrap_or(metadata.len());
                use super::TransferOut;
                self.internal.transfers_out.borrow_mut().push(TransferOut {
                    reference: message_number,
                    file,
                    current_position: offset_start.unwrap_or(0),
                    cutoff_position: offset_end,
                });
                self.send(Success { reference: message_number });
            }
            UploadRequest {
                path,
                filepart,
                offset_start,
            } => {
                self.bob_only();
                let path = if !path.is_empty() {
                    path
                } else {
                    if self.internal.pty.borrow_mut().is_some() {
                        self.internal.pty.borrow_mut().as_mut().unwrap().get_cwd()
                    } else {
                        ".".to_string()
                    }
                };
                let path: PathBuf = path.into();
                let mut path = if path.is_absolute() {
                    path
                } else {
                    let context = if self.internal.pty.borrow_mut().is_some() {
                        self.internal.pty.borrow_mut().as_mut().unwrap().get_cwd()
                    } else {
                        ".".to_string()
                    };
                    let mut context: PathBuf = context.into();
                    context.push(path);
                    context
                };
                ::std::fs::create_dir_all(&path).ok();
                path.push(filepart);
                info!("Trying to upload to {:?}", path);
                let file = if offset_start.is_none() {
                    File::create(path).map_err(|_| "Create file failed")?
                } else {
                    use std::{
                        fs::OpenOptions,
                        io::{Seek, SeekFrom},
                    };
                    let mut file = OpenOptions::new().write(true).truncate(false).open(path).map_err(|_| "Open file failed")?;
                    file.seek(SeekFrom::Start(offset_start.unwrap())).unwrap();
                    file
                };
                let file = Rc::new(RefCell::new(file));
                self.send(Success { reference: message_number });
                let proxy = self.clone();
                debug!("Watching for FileData with reference {:?}", message_number);
                self.watch(Rc::new(move |message, m2| match message {
                    FileData { reference, data } if *reference == message_number => {
                        debug!("Upload watcher sees FileData");
                        if data.is_empty() {
                            proxy.send(Success { reference: m2 });
                            info!("Upload complete");
                            return true;
                        }
                        let result = file.borrow_mut().write_all(&data[..]);
                        file.borrow_mut().flush().unwrap();
                        debug!("Wrote {:?} on upload", data.len());
                        if result.is_err() {
                            proxy.send(Reject {
                                reference: m2,
                                note:      "Failed to write upload data to file.".to_string(),
                            });
                            return true;
                        }
                        return false;
                    }
                    _ => false,
                }));
            }
            FileTruncateRequest { path, len } => {
                self.bob_only();
                #[cfg(unix)]
                {
                    use std::{fs::OpenOptions, os::unix::io::AsRawFd};

                    let file = OpenOptions::new()
                        .write(true)
                        .truncate(false)
                        .open(path)
                        .map_err(|_| "Failed to open file")?;
                    let result = ::nix::unistd::ftruncate(file.as_raw_fd(), len as i64);
                    if result.is_err() {
                        return Err("Truncate failed".to_string());
                    }
                }
                ();
            }
            BindConnectionAccepted { reference } => {
                assert!(perspective() == Alice);
                let addr = self
                    .internal
                    .remote_bind_destinations
                    .borrow_mut()
                    .get(&reference)
                    .ok_or("invalid_reference")?
                    .clone();
                if addr.contains('/') {
                    use nix::sys::socket::{connect, socket, AddressFamily, SockAddr, SockFlag, SockType};
                    let socket = socket(AddressFamily::Unix, SockType::Stream, SockFlag::empty(), None).map_err(|_| "Failed to create socket")?;
                    let sockaddr = SockAddr::new_unix(&PathBuf::from(addr.clone())).map_err(|_| "Failed to parse socket address")?;
                    connect(socket, &sockaddr).map_err(|_| "Failed to connect")?;
                    let bt = BufferedTransport::from(socket);
                    let stream = PortStream {
                        stream: bt,
                        token:  message_number,
                        oxy:    self.clone(),
                        local:  false,
                    };
                    let stream2 = Rc::new(stream.clone());
                    stream.stream.set_notify(stream2);
                    self.internal.remote_streams.borrow_mut().insert(message_number, stream);
                    return Ok(());
                }
                let mut addr = addr.to_socket_addrs().map_err(|_| "failed to resolve destination")?;
                let addr = addr.next().ok_or("Failed to resolve_destination")?;
                let stream = TcpStream::connect(&addr).map_err(|_| "Forward-connection failed")?;
                let bt = BufferedTransport::from(stream);
                let stream = PortStream {
                    stream: bt,
                    token:  message_number,
                    oxy:    self.clone(),
                    local:  false,
                };
                let stream2 = Rc::new(stream.clone());
                stream.stream.set_notify(stream2);
                self.internal.remote_streams.borrow_mut().insert(message_number, stream);
            }
            RemoteOpen { addr } => {
                assert!(perspective() == Bob);
                if addr.contains('/') {
                    use nix::sys::socket::{connect, socket, AddressFamily, SockAddr, SockFlag, SockType};
                    let socket = socket(AddressFamily::Unix, SockType::Stream, SockFlag::empty(), None).map_err(|_| "Failed to create socket")?;
                    let sockaddr = SockAddr::new_unix(&PathBuf::from(addr.clone())).map_err(|_| "Failed to parse socket address")?;
                    connect(socket, &sockaddr).map_err(|_| "Failed to connect")?;
                    let bt = BufferedTransport::from(socket);
                    let stream = PortStream {
                        stream: bt,
                        token:  message_number,
                        oxy:    self.clone(),
                        local:  false,
                    };
                    let stream2 = Rc::new(stream.clone());
                    stream.stream.set_notify(stream2);
                    self.internal.remote_streams.borrow_mut().insert(message_number, stream);
                    return Ok(());
                }
                let dest = addr
                    .to_socket_addrs()
                    .map_err(|_| "Resolving address failed.")?
                    .next()
                    .ok_or("Resolving address failed.")?;
                debug!("Resolved RemoteOpen destination to {:?}", dest);
                let stream = TcpStream::connect(&dest).map_err(|_| "Forward-connection failed")?;
                let bt = BufferedTransport::from(stream);
                let stream = PortStream {
                    stream: bt,
                    token:  message_number,
                    oxy:    self.clone(),
                    local:  false,
                };
                let stream2 = Rc::new(stream.clone());
                stream.stream.set_notify(stream2);
                self.internal.remote_streams.borrow_mut().insert(message_number, stream);
                self.send(Success { reference: message_number });
            }
            CloseRemoteBind { reference } => {
                let callback = self
                    .internal
                    .remote_bind_cleaners
                    .borrow_mut()
                    .remove(&reference)
                    .ok_or("Invalid reference")?;
                (callback)();
            }
            RemoteBind { addr } => {
                assert!(perspective() == Bob);
                if addr.contains("/") {
                    use nix::{
                        sys::socket::{accept, bind, listen, socket, AddressFamily, SockAddr, SockFlag, SockType},
                        unistd::{close, unlink},
                    };
                    let socket = socket(AddressFamily::Unix, SockType::Stream, SockFlag::empty(), None).map_err(|_| "Failed to create socket")?;
                    let sockaddr = SockAddr::new_unix(&PathBuf::from(addr.clone())).map_err(|_| "Failed to parse socket address")?;
                    bind(socket, &sockaddr).map_err(|_| "Failed to bind")?;
                    listen(socket, 10).map_err(|_| "Failed to listen")?;
                    debug!("Remote bind successful");
                    let token = Rc::new(RefCell::new(0));
                    let token2 = token.clone();
                    let proxy = self.clone();
                    let addr2 = addr.clone();
                    let token3 = transportation::insert_listener(Rc::new(move || {
                        let addr = addr.clone();
                        let token = token.clone();
                        let peer = accept(socket);
                        debug!("Accepted connection");
                        if peer.is_err() {
                            proxy.log_warn(&format!("Failed to accept connection on {}", addr));
                            transportation::remove_listener(*token.borrow());
                            return;
                        }
                        let peer = peer.unwrap();
                        let stream_token = proxy.send(BindConnectionAccepted { reference: message_number });
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

                    self.internal.remote_bind_cleaners.borrow_mut().insert(
                        message_number,
                        Rc::new(move || {
                            debug!("Closing remote bind.");
                            transportation::remove_listener(token3);
                            close(socket).map_err(|x| warn!("While closing remote bind: {:?}", x)).ok();
                            unlink(&PathBuf::from(addr2.clone()))
                                .map_err(|x| warn!("While unlinking remote bind: {:?}", x))
                                .ok();
                        }),
                    );

                    return Ok(());
                }
                let addr = if !addr.contains(':') { format!("localhost:{}", addr) } else { addr };
                let bind = ::std::net::TcpListener::bind(&addr).map_err(|_| "bind failed")?;
                let bind = TcpListener::from_std(bind).map_err(|_| "bind failed")?;
                let proxy = self.clone();
                let proxy = Rc::new(move || proxy.notify_bind(message_number));
                let token = transportation::insert_listener(proxy);
                transportation::borrow_poll(|poll| {
                    poll.register(&bind, Token(token), Ready::readable(), PollOpt::level()).unwrap();
                });
                let bind = PortBind {
                    listener:    bind,
                    local_spec:  addr,
                    remote_spec: "".to_string(),
                };
                self.internal.port_binds.borrow_mut().insert(message_number, bind);
                let proxy = self.clone();
                self.internal.remote_bind_cleaners.borrow_mut().insert(
                    message_number,
                    Rc::new(move || {
                        proxy.internal.port_binds.borrow_mut().remove(&message_number);
                    }),
                );
            }
            RemoteStreamData { reference, data } => {
                self.internal
                    .remote_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .ok_or("Invalid reference")?
                    .stream
                    .put(&data[..]);
            }
            RemoteStreamClosed { reference } => {
                self.internal
                    .remote_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .ok_or("Invalid reference")?
                    .stream
                    .close();
            }
            LocalStreamData { reference, data } => {
                self.internal
                    .local_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .ok_or("Invalid reference")?
                    .stream
                    .put(&data[..]);
            }
            LocalStreamClosed { reference } => {
                self.internal
                    .local_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .ok_or("Invalid reference")?
                    .stream
                    .close();
            }
            #[cfg(unix)]
            TunnelRequest { tap, name } => {
                self.bob_only();
                let mode = if tap { TunTapType::Tap } else { TunTapType::Tun };
                let tuntap = TunTap::create(mode, &name, message_number, self.clone());
                self.internal.tuntaps.borrow_mut().insert(message_number, tuntap);
            }
            #[cfg(unix)]
            TunnelData { reference, data } => {
                let borrow = self.internal.tuntaps.borrow_mut();
                borrow.get(&reference).unwrap().send(&data);
            }
            StatRequest { path } => {
                self.bob_only();
                let path = self.qualify_path(path);
                let info = symlink_metadata(path).map_err(|_| "Failed to stat")?;
                let message = StatResult {
                    reference:         message_number,
                    len:               info.len(),
                    is_dir:            info.is_dir(),
                    is_file:           info.is_file(),
                    atime:             None,
                    ctime:             None,
                    mtime:             None,
                    owner:             "".to_string(),
                    group:             "".to_string(),
                    octal_permissions: 0,
                };
                self.send(message);
            }
            ReadDir { path } => {
                self.bob_only();
                let path = self.qualify_path(path);
                let mut results = Vec::new();
                let dents = read_dir(path).map_err(|_| "read_dir failed")?;
                for entry in dents {
                    if let Ok(entry) = entry {
                        results.push(entry.file_name().to_string_lossy().into_owned());
                    }
                }
                self.send(ReadDirResult {
                    reference: message_number,
                    complete:  true, /* TODO: chunk out a limited number of entries at a time, spin with set_timeout to avoid clogging the
                                      * message queue. */
                    answers: results,
                });
            }
            PtyExited { status } => {
                self.alice_only();
                debug!("Remote PTY process exited with status {}", status);
                self.exit(0);
            }
            FileHashRequest {
                path,
                offset_start,
                offset_end,
                hash_algorithm,
            } => {
                use std::io::{Read, Seek, SeekFrom};
                use transportation::ring::digest::{Context, SHA1, SHA256, SHA512};
                let algorithm = match hash_algorithm {
                    0 => unimplemented!("MD5"),
                    1 => &SHA1,
                    2 => &SHA256,
                    3 => &SHA512,
                    _ => panic!(),
                };
                let mut file = File::open(path).map_err(|_| "Failed to open file")?;
                let mut context = Context::new(algorithm);
                file.seek(SeekFrom::Start(offset_start.unwrap_or(0))).unwrap();
                let mut ticker = offset_start.unwrap_or(0);
                let mut data = [0u8; 4096];
                let offset_end = offset_end.unwrap_or(file.metadata().unwrap().len());
                loop {
                    let result = file.read(&mut data[..]).map_err(|_| "Error reading file")? as u64;
                    if result == 0 {
                        Err("Reached end of file")?;
                    }
                    if ticker + result >= offset_end {
                        let to_take = (offset_end - ticker) as usize;
                        context.update(&data[..to_take]);
                        break;
                    }
                    context.update(&data[..(result as usize)]);
                    ticker += result;
                }
                let digest = context.finish();
                let digest: Vec<u8> = digest.as_ref().to_vec();
                info!("File digest: {}", ::data_encoding::HEXUPPER.encode(&digest));
                self.send(FileHashData {
                    reference: message_number,
                    digest,
                });
            }
            KnockForward { destination, knock } => {
                let mut sock = ::std::net::UdpSocket::bind("[::0]:0");
                if sock.is_err() {
                    sock = Ok(::std::net::UdpSocket::bind("0.0.0.0:0").map_err(|_| "Failed to create UDP socket")?);
                }
                let sock = sock.unwrap();
                let destination = ::std::net::ToSocketAddrs::to_socket_addrs(&destination).map_err(|_| "Failed to resolve destination")?;
                for destination in destination {
                    sock.send_to(&knock, &destination).ok();
                }
                ::std::thread::sleep(::std::time::Duration::from_millis(500)); // TODO: Now HERE's a hack-and-a-half.
            }
            _ => {
                debug!("A not-statically supported message type came through.");
            }
        };
        Ok(())
    }
}
