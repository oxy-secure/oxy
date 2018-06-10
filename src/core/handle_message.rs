use super::{PortBind, PortStream};
use arg::{self, perspective};
use byteorder::{self, ByteOrder};
use core::Oxy;
use message::OxyMessage::{self, *};
#[cfg(unix)]
use pty::Pty;
use std::{
    fs::{metadata, read_dir, symlink_metadata, File}, io::Write, net::ToSocketAddrs, path::PathBuf, rc::Rc, time::Instant,
};
use transportation::{
    self, mio::{
        net::{TcpListener, TcpStream}, PollOpt, Ready, Token,
    }, BufferedTransport, EncryptionPerspective::{Alice, Bob},
    Notifies,
};
#[cfg(unix)]
use tuntap::{TunTap, TunTapType};

impl Oxy {
    pub(crate) fn handle_message(&self, message: OxyMessage, message_number: u64) -> Result<(), String> {
        debug!("Recieved message {}", message_number);
        trace!("Received message {}: {:?}", message_number, message);
        match message {
            DummyMessage { .. } => (),
            Ping {} => {
                self.send(Pong {});
            }
            Pong {} => {
                *self.internal.last_message_seen.borrow_mut() = Instant::now();
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
            #[cfg(unix)]
            PtyRequest { command } => {
                self.bob_only();
                let pty = Pty::forkpty(&command);
                let proxy = self.clone();
                pty.underlying.set_notify(Rc::new(move || proxy.notify_pty()));
                *self.internal.pty.borrow_mut() = Some(pty);
                self.send(PtyRequestResponse { granted: true });
                trace!("Successfully allocated PTY");
            }
            #[cfg(unix)]
            PtyRequestResponse { granted } => {
                self.alice_only();
                if !granted {
                    warn!("PTY open failed");
                    return Ok(());
                }
                if self.internal.ui.borrow().is_some() {
                    let (w, h) = self.internal.ui.borrow_mut().as_mut().unwrap().pty_size();
                    self.send(PtySizeAdvertisement { w, h });
                }
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
                info!("BasicCommandOutput {:?}, {:?}", stdout, stderr);
                if let Ok(stdout) = String::from_utf8(stdout) {
                    info!("stdout:\n-----\n{}\n-----", stdout);
                }
            }
            DownloadRequest {
                path,
                offset_start,
                offset_end,
            } => {
                use std::io::{Seek, SeekFrom};
                self.bob_only();
                let mut file = File::open(path).map_err(|_| "Failed to open file")?;
                let metadata = file.metadata().map_err(|_| "Failed to stat file")?;
                self.send(FileSize {
                    reference: message_number,
                    size:      metadata.len(),
                });
                if let Some(offset_start) = offset_start {
                    file.seek(SeekFrom::Start(offset_start)).map_err(|_| "Start-seek failed")?;
                }
                let offset_end = offset_end.unwrap_or(metadata.len());
                use super::TransferOut;
                self.internal.transfers_out.borrow_mut().push(TransferOut {
                    reference: message_number,
                    file,
                    current_position: offset_start.unwrap_or(0),
                    cutoff_position: offset_end,
                });
            }
            UploadRequest {
                path,
                filepart,
                offset_start,
            } => {
                self.bob_only();
                let mut path: PathBuf = path.into();
                if let Ok(meta) = metadata(&path) {
                    if meta.is_dir() {
                        path.push(filepart);
                    }
                }
                let file = if offset_start.is_none() {
                    File::create(path).map_err(|_| "Create file failed")?
                } else {
                    use std::{
                        fs::OpenOptions, io::{Seek, SeekFrom},
                    };
                    let mut file = OpenOptions::new().write(true).truncate(false).open(path).map_err(|_| "Open file failed")?;
                    file.seek(SeekFrom::Start(offset_start.unwrap())).unwrap();
                    file
                };
                self.internal.transfers_in.borrow_mut().insert(message_number, file);
            }
            FileTruncateRequest { path, len } => {
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
            FileSize { reference: _, size } => {
                #[cfg(unix)]
                ::copy::push_file_size(size);
            }
            FileData { reference, data } => {
                if data.is_empty() {
                    self.internal.ui.borrow().as_ref().map(|x| x.log("File transfer completed"));
                    debug!("File transfer completed");
                    self.internal.transfers_in.borrow_mut().remove(&reference);
                    if self.internal.copy_peer.borrow_mut().is_some() {
                        *self.internal.fetch_file_ticker.borrow_mut() += 1;
                        if *self.internal.fetch_file_ticker.borrow_mut() < arg::matches().occurrences_of("source") {
                            let filename = arg::matches()
                                .values_of("source")
                                .unwrap()
                                .nth(*self.internal.fetch_file_ticker.borrow_mut() as usize)
                                .unwrap();
                            let filename = filename.splitn(2, ':').nth(1).unwrap().to_string();
                            let cmd = ["download", &filename, "unused"].to_vec();
                            let cmd = cmd.iter().map(|x| x.to_string()).collect();
                            self.handle_metacommand(cmd);
                        } else {
                            let mut message = [0u8; 8];
                            byteorder::BE::write_u64(&mut message, ::std::u64::MAX);
                            self.internal.copy_peer.borrow_mut().as_ref().unwrap().send_message(&message);
                            trace!("Source is done.");
                        }
                    }
                    return Ok(());
                }
                if self.internal.copy_peer.borrow_mut().is_none() {
                    self.internal
                        .transfers_in
                        .borrow_mut()
                        .get_mut(&reference)
                        .unwrap()
                        .write_all(&data[..])
                        .unwrap();
                } else {
                    let ticker = *self.internal.fetch_file_ticker.borrow_mut();
                    let mut message: Vec<u8> = Vec::new();
                    message.resize(8, 0);
                    byteorder::BE::write_u64(&mut message[..8], ticker);
                    message.extend(&data);
                    self.internal.copy_peer.borrow_mut().as_ref().unwrap().send_message(&message);
                }
            }
            BindConnectionAccepted { reference } => {
                assert!(perspective() == Alice);
                let addr = self
                    .internal
                    .remote_bind_destinations
                    .borrow_mut()
                    .get(&reference)
                    .unwrap()
                    .parse()
                    .unwrap();
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
            }
            RemoteBind { addr } => {
                assert!(perspective() == Bob);
                let bind = TcpListener::bind(&addr.parse().unwrap()).unwrap();
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
            }
            RemoteStreamData { reference, data } => {
                self.internal
                    .remote_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .unwrap()
                    .stream
                    .put(&data[..]);
            }
            LocalStreamData { reference, data } => {
                self.internal
                    .local_streams
                    .borrow_mut()
                    .get_mut(&reference)
                    .unwrap()
                    .stream
                    .put(&data[..]);
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
            _ => Err("Unsupported message type")?,
        };
        Ok(())
    }
}
