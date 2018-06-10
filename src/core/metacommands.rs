use clap::{self, App, AppSettings, Arg, SubCommand};
use core::{Oxy, PortBind, SocksBind, SocksBindNotificationProxy};
use message::OxyMessage::*;
use num;
use std::{cell::RefCell, fs::File, path::PathBuf, rc::Rc};
use transportation::{
    self, mio::{net::TcpListener, PollOpt, Ready, Token},
};
#[cfg(unix)]
use tuntap::{TunTap, TunTapType};

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
            .arg(
                Arg::with_name("command")
                    .index(1)
                    .default_value(&::arg::matches().value_of("command").unwrap_or("bash")),
            ),
        SubCommand::with_name("sh")
            .about(
                "Run a remote basic-command. \
                 Useful for Windows servers.",
            )
            .long_about(
                "Run a remote basic-command. \
                 Useful for Windows servers. \
                 The command runs asyncronously and you don't get any output, \
                 but you can pipe output to a file and then download it later.",
            )
            .arg(Arg::with_name("command").index(1).required(true)),
        SubCommand::with_name("exit").about("Exits the Oxy client."),
        SubCommand::with_name("f10").about("Send F10 to the remote"),
        SubCommand::with_name("f12").about("Send F12 to the remote"),
        SubCommand::with_name("hash")
            .arg(Arg::with_name("path").index(1).required(true))
            .arg(Arg::with_name("offset start").long("start").takes_value(true))
            .arg(Arg::with_name("offset end").long("end").takes_value(true))
            .about("Request the file hash of a file"),
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

impl Oxy {
    pub(super) fn handle_metacommand(&self, parts: Vec<String>) {
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
                        self.send(BasicCommand {
                            command: matches.value_of("command").unwrap().to_string(),
                        });
                    }
                    "pty" => {
                        let command = matches.value_of("command").unwrap().to_string();
                        self.send(PtyRequest { command });
                    }
                    "download" => {
                        let file = File::create(matches.value_of("local path").unwrap().to_string());
                        if file.is_err() {
                            error!("Failed to open local file for writing: {}", matches.value_of("local path").unwrap());
                            return;
                        }
                        let file = file.unwrap();
                        let id = self.send(DownloadRequest {
                            path:         matches.value_of("remote path").unwrap().to_string(),
                            offset_start: matches.value_of("offset start").map(|x| x.parse().unwrap()),
                            offset_end:   matches.value_of("offset end").map(|x| x.parse().unwrap()),
                        });
                        debug!("Download started");
                        self.internal.transfers_in.borrow_mut().insert(id, file);
                    }
                    "upload" => {
                        let buf: PathBuf = matches.value_of("local path").unwrap().into();
                        let buf = buf.canonicalize().unwrap();
                        let file = File::open(buf.clone());
                        if file.is_err() {
                            error!("Failed to open local file for reading: {}", matches.value_of("local path").unwrap());
                            return;
                        }
                        let file = file.unwrap();
                        let id = self.send(UploadRequest {
                            path:         matches.value_of("remote path").unwrap().to_string(),
                            filepart:     buf.file_name().unwrap().to_string_lossy().into_owned(),
                            offset_start: matches.value_of("offset start").map(|x| x.parse().unwrap()),
                        });
                        let len = file.metadata().unwrap().len();
                        debug!("Upload started");
                        self.internal.transfers_out.borrow_mut().push(super::TransferOut {
                            reference: id,
                            file,
                            current_position: 0,
                            cutoff_position: len,
                        });
                    }
                    "L" => {
                        let remote_spec = matches.value_of("remote spec").unwrap().to_string();
                        let local_spec = matches.value_of("local spec").unwrap().to_string();
                        let bind = TcpListener::bind(&local_spec.parse().unwrap()).unwrap();
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
                            local_spec,
                            remote_spec,
                        };
                        self.internal.port_binds.borrow_mut().insert(token_sized, bind);
                    }
                    "R" => {
                        let bind_id = self.send(RemoteBind {
                            addr: matches.value_of("remote spec").unwrap().to_string(),
                        });
                        self.internal
                            .remote_bind_destinations
                            .borrow_mut()
                            .insert(bind_id, matches.value_of("local spec").unwrap().to_string());
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
                    "socks" => {
                        let local_spec = matches.value_of("bind spec").unwrap();
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
                        self.internal.socks_binds.borrow_mut().insert(token_sized, socks);
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
                    _ => (),
                }
            }
        }
    }
}
