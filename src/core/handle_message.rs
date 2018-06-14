use super::{PortBind, PortStream};
use crate::arg::perspective;
use crate::core::Oxy;
use crate::message::OxyMessage::{self, *};
#[cfg(unix)]
use crate::pty::Pty;
use std::{
    cell::RefCell, fs::{read_dir, symlink_metadata, File}, io::Write, net::ToSocketAddrs, path::PathBuf, rc::Rc, time::Instant,
};
use transportation::{
    self, mio::{
        net::{TcpListener, TcpStream}, PollOpt, Ready, Token,
    }, BufferedTransport, EncryptionPerspective::{Alice, Bob},
    Notifies,
};
#[cfg(unix)]
use crate::tuntap::{TunTap, TunTapType};

impl Oxy {
    fn dispatch_watchers(&self, message: &OxyMessage, message_number: u64) {
        let mut hot_watchers = (*self.internal.response_watchers.borrow()).clone();
        let start_len = hot_watchers.len();
        hot_watchers.retain(|x| !(x)(message, message_number));
        let mut borrow = self.internal.response_watchers.borrow_mut();
        borrow.splice(..start_len, hot_watchers.into_iter());
    }

    pub(crate) fn handle_message(&self, message: OxyMessage, message_number: u64) -> Result<(), String> {
        debug!("Recieved message {}", message_number);
        trace!("Received message {}: {:?}", message_number, message);
        self.dispatch_watchers(&message, message_number);
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
                let mut path: PathBuf = path.into();
                if !path.is_absolute() && self.internal.pty.borrow_mut().is_some() {
                    let mut base_path: PathBuf = self.internal.pty.borrow_mut().as_mut().unwrap().get_cwd().into();
                    base_path.push(path);
                    path = base_path;
                }
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
                        fs::OpenOptions, io::{Seek, SeekFrom},
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
            _ => {
                debug!("A not-statically supported message type came through.");
            }
        };
        Ok(())
    }
}
