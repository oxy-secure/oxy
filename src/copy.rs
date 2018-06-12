use client;
use core::Oxy;
use message::OxyMessage::*;
use std::{
    cell::RefCell, collections::HashMap, fs::{metadata, read_dir, File}, io::{Read, Write}, path::PathBuf, rc::Rc,
};
use transportation;

const PEER_TO_PEER_BUFFER_AMT: u64 = 1024 * 1024;

pub fn run() -> ! {
    CopyManager::create();
    transportation::run();
}

#[derive(Default, Clone)]
struct CopyManager {
    i: Rc<CopyManagerInternal>,
}

#[derive(Default)]
struct CopyManagerInternal {
    connections:       RefCell<HashMap<String, Oxy>>,
    destination:       RefCell<String>,
    sources:           RefCell<Vec<String>>,
    synthetic_sources: RefCell<Vec<(Option<String>, String, String)>>,
    auth_ticker:       RefCell<u64>,
    progress:          RefCell<u64>,
}

impl CopyManager {
    fn create() -> CopyManager {
        let x = CopyManager::default();
        x.init();
        x
    }

    fn init(&self) {
        *self.i.progress.borrow_mut() = 1001;
        let mut locations: Vec<String> = ::arg::matches().values_of("location").unwrap().map(|x| x.to_string()).collect();
        if locations.len() < 2 {
            error!("Must provide at least two locations (a source and a destination)");
            ::std::process::exit(1);
        }
        let len = locations.len();
        let destination = locations.remove(len - 1);
        let sources = locations;
        for source in &sources {
            if let Some(dest) = get_peer(source) {
                self.create_connection(dest);
            }
        }
        if let Some(dest) = get_peer(&destination) {
            self.create_connection(dest);
        }
        *self.i.destination.borrow_mut() = destination;
        *self.i.sources.borrow_mut() = sources;
        if self.i.connections.borrow().is_empty() {
            self.tick_transfers();
        }
    }

    fn create_connection(&self, peer: &str) {
        if self.i.connections.borrow().contains_key(peer) {
            return;
        }
        let connection = client::connect(peer);
        connection.set_daemon();
        let proxy = self.clone();
        connection.set_post_auth_hook(Rc::new(move || {
            proxy.post_auth_hook();
        }));
        self.i.connections.borrow_mut().insert(peer.to_string(), connection);
    }

    fn print_progress(&self, progress: u64, filename: &str) {
        let prev = *self.i.progress.borrow();
        if progress < prev {
            print!("\n\n");
        }
        if *self.i.progress.borrow_mut() == progress {
            return;
        }
        *self.i.progress.borrow_mut() = progress;
        let percentage = progress / 10;
        let decimal = progress % 10;
        let line1 = format!("Transfering: {:?} Transfered: {}.{}%", filename, percentage, decimal);
        let width = ::termion::terminal_size().unwrap().0 as u64;
        let barwidth: u64 = (width * percentage) / 100;
        let mut line2 = "=".repeat(barwidth as usize);
        if line2.len() > 0 && percentage < 100 {
            let len = line2.len();
            line2.remove(len - 1);
            line2.push('>');
        }
        let mut data = Vec::new();
        data.extend(b"\x1b[2A"); // Move up two lines
        data.extend(b"\x1b[0K"); // Clear the line
        data.extend(line1.as_bytes());
        data.extend(b"\n\x1b[0K");
        data.extend(line2.as_bytes());
        data.extend(b"\n");
        let stdout = ::std::io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(&data[..]).unwrap();
        lock.flush().unwrap();
    }

    fn tick_remote_source(&self, peer: String, head: String, tail: String) {
        let borrow = self.i.connections.borrow();
        let connection = borrow.get(&peer).unwrap().clone();
        let mut path: PathBuf = head.clone().into();
        let tailbuf: PathBuf = tail.clone().into();
        path.push(tailbuf);
        let path = path.to_str().unwrap().to_string();
        let path = path.trim_right_matches('/').to_string();
        info!("Processing {:?}", path);
        let id = connection.send(StatRequest { path: path.clone() });
        let proxy = self.clone();
        connection.clone().watch(Rc::new(move |message, _| match message {
            StatResult { reference, is_dir, len, .. } if *reference == id => {
                let head = head.clone();
                let tail = tail.clone();
                let path = path.clone();
                let len = *len;
                debug!("Got stat result: {:?}", message);
                if *is_dir {
                    let id = connection.send(ReadDir { path: path.clone() });
                    let proxy = proxy.clone();
                    let peer = peer.clone();
                    connection.clone().watch(Rc::new(move |message, _| match message {
                        ReadDirResult {
                            reference,
                            complete,
                            answers,
                        } if *reference == id =>
                        {
                            let tail: PathBuf = tail.clone().into();
                            for answer in answers {
                                let mut tail = tail.clone();
                                tail.push(answer);
                                let tail = tail.to_str().unwrap().trim_right_matches('/').to_string();
                                proxy.i.synthetic_sources.borrow_mut().insert(0, (Some(peer.clone()), head.clone(), tail));
                            }
                            if *complete {
                                proxy.tick_transfers();
                            }
                            return *complete;
                        }
                        _ => false,
                    }));
                } else {
                    let dest_peer = get_peer(&proxy.i.destination.borrow().clone()).map(|x| x.to_string());
                    let dest_path = get_path(&proxy.i.destination.borrow().clone()).to_string();
                    if let Some(dest_peer) = dest_peer {
                        debug!("Doing peer-to-peer transfer");
                        let dest_connection = {
                            let borrow = proxy.i.connections.borrow();
                            borrow.get(&dest_peer).unwrap().clone()
                        };
                        let source_connection = connection.clone();
                        let mut upload_buf = PathBuf::from(&dest_path);
                        upload_buf.push(PathBuf::from(&tail));
                        let upload_path = upload_buf.parent().unwrap().to_str().unwrap().to_string();
                        let upload_filepart = upload_buf.file_name().unwrap().to_str().unwrap().to_string();
                        let proxy = proxy.clone();
                        let upload_id = dest_connection.send(UploadRequest {
                            path:         upload_path,
                            filepart:     upload_filepart.clone(),
                            offset_start: None,
                        });
                        dest_connection.clone().watch(Rc::new(move |message, _| match message {
                            Success { reference } if *reference == upload_id => {
                                debug!("Peer-to-peer upload accepted");
                                let done = Rc::new(RefCell::new(false));
                                let written = Rc::new(RefCell::new(0u64));
                                let download_id: Rc<RefCell<Option<u64>>> = Rc::new(RefCell::new(None));
                                let dest_connection = dest_connection.clone();
                                let source_connection = source_connection.clone();
                                let path = path.clone();
                                let upload_filepart = upload_filepart.clone();
                                let proxy = proxy.clone();
                                dest_connection.clone().push_send_hook(Rc::new(move || {
                                    let dest_connection = dest_connection.clone();
                                    let source_connection = source_connection.clone();
                                    let done = done.clone();
                                    let download_id = download_id.clone();
                                    let written = written.clone();
                                    let upload_filepart = upload_filepart.clone();
                                    let proxy = proxy.clone();
                                    if *done.borrow() {
                                        return true;
                                    }
                                    if download_id.borrow().is_none() && dest_connection.has_write_space() {
                                        let end = if len - *written.borrow() <= PEER_TO_PEER_BUFFER_AMT {
                                            None
                                        } else {
                                            Some(*written.borrow() + PEER_TO_PEER_BUFFER_AMT)
                                        };
                                        debug!("Initiating a peer-to-peer chunk-download {:?}", end);
                                        *download_id.borrow_mut() = Some(source_connection.send(DownloadRequest {
                                            path:         path.clone(),
                                            offset_start: Some(*written.borrow()),
                                            offset_end:   end.clone(),
                                        }));
                                        source_connection.clone().watch(Rc::new(move |message, _| match message {
                                            FileData { reference, data } if *reference == download_id.borrow().unwrap() => {
                                                if data.is_empty() && *written.borrow() < len {
                                                    debug!("Finished a chunk");
                                                    download_id.borrow_mut().take();
                                                    dest_connection.notify_main_transport();
                                                    return true;
                                                }
                                                if end.is_some() && *written.borrow() + data.len() as u64 > end.unwrap() {
                                                    panic!("Server sent more data than requested! Bad server!");
                                                }
                                                let send_id = dest_connection.send(FileData {
                                                    reference: upload_id,
                                                    data:      data.to_vec(),
                                                });
                                                *written.borrow_mut() += data.len() as u64;
                                                let progress = (*written.borrow() * 1000) / len;
                                                proxy.print_progress(progress, &upload_filepart);
                                                if data.is_empty() {
                                                    debug!("Final FileData sent. Waiting for confirmation.");
                                                    let done = done.clone();
                                                    let proxy = proxy.clone();
                                                    dest_connection.watch(Rc::new(move |message, _| match message {
                                                        Success { reference } if *reference == send_id => {
                                                            info!("Transfer complete.");
                                                            proxy.tick_transfers();
                                                            *done.borrow_mut() = true;
                                                            return true;
                                                        }
                                                        _ => false,
                                                    }));
                                                    return true;
                                                }
                                                return false;
                                            }
                                            Reject { reference, note } if *reference == download_id.borrow().unwrap() => {
                                                warn!("Error retrieving file: {:?}", note);
                                                proxy.tick_transfers();
                                                *done.borrow_mut() = true;
                                                return true;
                                            }
                                            _ => false,
                                        }));
                                    }
                                    return false;
                                }));
                                return true;
                            }
                            Reject { reference, note } if *reference == upload_id => {
                                warn!("Upload request failed: {:?}", note);
                                proxy.tick_transfers();
                                return true;
                            }
                            _ => false,
                        }));
                    } else {
                        let id = connection.send(DownloadRequest {
                            path:         path.clone(),
                            offset_start: None,
                            offset_end:   None,
                        });
                        let dest = proxy.i.destination.borrow().clone();
                        let mut dest: PathBuf = dest.into();
                        let dest_tail: PathBuf = tail.into();
                        dest.push(dest_tail);
                        let file_name = dest.file_name().unwrap().to_str().unwrap().to_string();
                        ::std::fs::create_dir_all(dest.parent().unwrap()).ok();
                        let file = File::create(&dest);
                        if file.is_err() {
                            warn!("Failed to create local file for writing: {:?}", dest);
                            proxy.tick_transfers();
                            return true;
                        }
                        let file = Rc::new(RefCell::new(file.unwrap()));
                        let proxy = proxy.clone();
                        let written = Rc::new(RefCell::new(0u64));
                        connection.clone().watch(Rc::new(move |message, _| match message {
                            FileData { reference, data } if *reference == id => {
                                if data.is_empty() {
                                    info!("Transfer finished.");
                                    proxy.tick_transfers();
                                    return true;
                                }
                                let result = file.borrow_mut().write_all(&data[..]);
                                if result.is_err() {
                                    warn!("Error writing data to local file");
                                    proxy.tick_transfers();
                                    return true;
                                }
                                *written.borrow_mut() += data.len() as u64;
                                let progress = (*written.borrow() * 1000) / len;
                                proxy.print_progress(progress, &file_name);
                                return false;
                            }
                            Reject { reference, note } if *reference == id => {
                                warn!("Error reading file: {:?}", note);
                                proxy.tick_transfers();
                                return true;
                            }
                            _ => false,
                        }));
                    }
                }
                return true;
            }
            Reject { reference, note } if *reference == id => {
                warn!("Failed to stat remote file {:?}, {:?}", path, note);
                proxy.tick_transfers();
                return true;
            }
            _ => false,
        }));
    }

    fn tick_local_source(&self, head: String, tail: String) {
        let mut fullpath = PathBuf::from(&head);
        fullpath.push(PathBuf::from(&tail));
        info!("Trying to upload {:?}", fullpath);
        let md = metadata(&fullpath);
        if md.is_err() {
            warn!("Failed to stat local file {:?}", fullpath);
            self.tick_transfers();
        }
        let md = md.unwrap();
        if md.is_dir() {
            for i in read_dir(&fullpath).unwrap() {
                let i = i.unwrap();
                let tail = format!("{}/{}", tail, i.file_name().into_string().unwrap());
                self.i.synthetic_sources.borrow_mut().push((None, head.clone(), tail));
            }
            self.tick_transfers();
            return;
        } else {
            let dest = self.i.destination.borrow().clone();
            let dest_peer = get_peer(&dest);
            let dest_path = get_path(&dest);
            if dest_peer.is_none() {
                let mut dest_path = PathBuf::from(dest_path);
                dest_path.push(PathBuf::from(&tail));
                ::std::fs::create_dir_all(dest_path.parent().unwrap()).ok();
                ::std::fs::copy(&fullpath, &dest_path).unwrap();
                info!("Uploaded {:?}", fullpath);
                self.tick_transfers();
                return;
            } else {
                let dest_connection = {
                    let borrow = self.i.connections.borrow();
                    borrow.get(dest_peer.unwrap()).unwrap().clone()
                };
                let proxy = self.clone();
                let mut dest_path = PathBuf::from(&dest_path);
                dest_path.push(PathBuf::from(&tail));

                let file_name = fullpath.file_name().unwrap().to_str().unwrap().to_string();
                let file = File::open(&fullpath);
                if file.is_err() {
                    warn!("Failed to open local file {:?}", file);
                }
                let file = file.unwrap();
                let len = file.metadata().unwrap().len();
                let file = Rc::new(RefCell::new(file));
                let written = Rc::new(RefCell::new(0));

                let id = dest_connection.send(UploadRequest {
                    path:         dest_path.parent().unwrap().to_str().unwrap().to_string(),
                    filepart:     dest_path.file_name().unwrap().to_str().unwrap().to_string(),
                    offset_start: None,
                });
                dest_connection.clone().watch(Rc::new(move |message, _| match message {
                    Success { reference } if *reference == id => {
                        info!("Upload request accepted");
                        let dest_connection = dest_connection.clone();
                        let file = file.clone();
                        let file_name = file_name.clone();
                        let written = written.clone();
                        let proxy = proxy.clone();
                        dest_connection.clone().push_send_hook(Rc::new(move || {
                            if dest_connection.has_write_space() {
                                let mut buf = [0u8; 8192];
                                let result = file.borrow_mut().read(&mut buf);
                                if result.is_err() {
                                    warn!("Failed to read file");
                                    proxy.tick_transfers();
                                    return true;
                                }
                                let result = result.unwrap();
                                let send_id = dest_connection.send(FileData {
                                    reference: id,
                                    data:      buf[..result].to_vec(),
                                });
                                *written.borrow_mut() += result as u64;
                                let progress = (*written.borrow() * 1000) / len;
                                proxy.print_progress(progress, &file_name);
                                if result == 0 {
                                    let proxy = proxy.clone();
                                    dest_connection.watch(Rc::new(move |message, _| match message {
                                        Success { reference } if *reference == send_id => {
                                            info!("Upload finished");
                                            proxy.tick_transfers();
                                            return true;
                                        }
                                        _ => false,
                                    }));
                                }
                                return false;
                            }
                            return false;
                        }));
                        return true;
                    }
                    Reject { reference, note } if *reference == id => {
                        warn!("Upload request failed: {:?}", note);
                        proxy.tick_transfers();
                        return true;
                    }
                    _ => false,
                }));
            }
        }
    }

    fn tick_transfers(&self) {
        if self.i.sources.borrow().is_empty() && self.i.synthetic_sources.borrow().is_empty() {
            info!("Finished!");
            ::std::process::exit(0);
        }
        *self.i.progress.borrow_mut() = 1001;
        let peer;
        let head;
        let tail;
        if !self.i.synthetic_sources.borrow().is_empty() {
            let (a, b, c) = self.i.synthetic_sources.borrow_mut().remove(0);
            peer = a;
            head = b;
            tail = c;
        } else {
            let source = self.i.sources.borrow_mut().remove(0);
            let path = get_path(&source).to_string();
            if path.ends_with('/') {
                head = path.clone();
                tail = "".to_string();
            } else {
                head = PathBuf::from(path.clone())
                    .parent()
                    .map(|x| x.to_str().unwrap().to_string())
                    .unwrap_or("".to_string());
                tail = PathBuf::from(path.clone())
                    .file_name()
                    .map(|x| x.to_str().unwrap().to_string())
                    .unwrap_or("".to_string());
            }
            peer = get_peer(&source).map(|x| x.to_string());
        }
        if let Some(peer) = peer {
            self.tick_remote_source(peer, head, tail);
        } else {
            self.tick_local_source(head, tail);
        }
    }

    fn post_auth_hook(&self) {
        *self.i.auth_ticker.borrow_mut() += 1;
        if *self.i.auth_ticker.borrow() == self.i.connections.borrow().len() as u64 {
            self.tick_transfers();
        }
    }
}

fn get_peer<'a>(location: &'a str) -> Option<&'a str> {
    if !location.splitn(2, '/').next().unwrap().contains(':') {
        return None;
    }
    if location.starts_with('[') {
        return Some(location.splitn(2, ']').next().unwrap());
    }
    Some(location.splitn(2, ':').next().unwrap())
}

fn get_path<'a>(location: &'a str) -> &'a str {
    if !location.splitn(2, '/').next().unwrap().contains(':') {
        return location;
    }
    if location.starts_with('[') {
        return location.splitn(2, ']').nth(1).unwrap().splitn(2, ':').nth(1).unwrap();
    }
    location.splitn(2, ':').nth(1).unwrap()
}
