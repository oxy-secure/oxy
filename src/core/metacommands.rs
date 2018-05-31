use clap::{self, App, AppSettings, Arg, SubCommand};
use core::{BindNotificationProxy, Oxy, PortBind, SocksBind, SocksBindNotificationProxy};
use message::OxyMessage::*;
use num;
use std::{cell::RefCell, fs::File, rc::Rc};
use transportation::{
	self, mio::{net::TcpListener, PollOpt, Ready, Token},
};
#[cfg(unix)]
use tuntap::{TunTap, TunTapType};

fn create_app() -> App<'static, 'static> {
	let subcommands = vec![
		SubCommand::with_name("L").about("Create a local portforward."),
		SubCommand::with_name("R").about("Create a remote portforward."),
		SubCommand::with_name("download")
			.about("Download a file")
			.arg(Arg::with_name("remote path").help("Remote file path to download from.").index(1))
			.arg(Arg::with_name("local path").help("Local file path to download to.").index(2)),
		SubCommand::with_name("upload").about("Upload a file"),
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
		SubCommand::with_name("pty").about(
			"Open a remote PTY. \
			 Happens by default, usually not necessary",
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
			),
		SubCommand::with_name("exit").about("Exits the Oxy client."),
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
	pub(super) fn handle_metacommand(&self, mut parts: Vec<String>) {
		let matches = create_app().get_matches_from_safe(parts.clone());
		match matches {
			Err(clap::Error { message, .. }) => {
				println!("{}", message);
			}
			Ok(matches) => {
				match matches.subcommand_name().unwrap() {
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
					"exit" => {
						::std::process::exit(0);
					}
					_ => (),
				}
			}
		}
	}
}
