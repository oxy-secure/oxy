use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use env_logger;
use std::env;
use transportation::EncryptionPerspective;

lazy_static! {
	static ref CLEAN_ARGS: Vec<String> = clean_args();
	pub static ref MATCHES: ArgMatches<'static> = create_matches();
}

const REAL_SUBCOMMANDS: [&str; 8] = [
	"client",
	"reexec",
	"server",
	"help",
	"serve-one",
	"reverse-server",
	"reverse-client",
	"guide",
];

fn get_first_positional_argument() -> Option<String> {
	let matches = App::new("fake")
		.setting(AppSettings::DisableVersion)
		.arg(Arg::with_name("fakearg").index(1))
		.get_matches_safe();
	if matches.is_err() {
		return None;
	}
	matches.unwrap().value_of("fakearg").map(|x| x.to_string())
}

fn clean_args() -> Vec<String> {
	let arg1 = get_first_positional_argument();
	let mut result = ::std::env::args().collect();
	if arg1.is_none() || REAL_SUBCOMMANDS.contains(&arg1.unwrap().as_str()) {
		return result;
	}
	result.insert(1, "client".to_string());
	result
}

fn create_matches() -> ArgMatches<'static> {
	let metacommand = Arg::with_name("metacommand")
		.short("m")
		.long("metacommand")
		.takes_value(true)
		.multiple(true)
		.help("A command to run after the connection is established. The same commands from the F10 prompt.");
	let identity = Arg::with_name("identity")
		.short("i")
		.long("identity")
		.takes_value(true)
		.env("OXY_IDENTITY");
	let client_args = vec![metacommand.clone(), identity.clone().required(true)];
	let server_args = vec![identity];
	App::new("oxy")
		.version(crate_version!())
		.author(crate_authors!())
		.setting(AppSettings::SubcommandRequiredElseHelp)
		.subcommand(
			SubCommand::with_name("client")
				.about("Connect to an Oxy server.")
				.args(&client_args)
				.arg(Arg::with_name("destination").index(1).required(true)),
		)
		.subcommand(
			SubCommand::with_name("reexec")
				.about("Service a single oxy connection. Communicates on stdio by default.")
				.args(&server_args),
		)
		.subcommand(
			SubCommand::with_name("server")
				.about("Accept TCP connections then reexec for each one.")
				.args(&server_args),
		)
		.subcommand(
			SubCommand::with_name("serve-one")
				.about("Accept a single TCP connection, then service it in the same process.")
				.args(&server_args)
				.arg(Arg::with_name("bind-address").index(1).default_value("0.0.0.0:2600")),
		)
		.subcommand(
			SubCommand::with_name("reverse-server")
				.about("Connect out to a listening client. Then, be a server.")
				.args(&server_args)
				.arg(Arg::with_name("destination").index(1).required(true)),
		)
		.subcommand(
			SubCommand::with_name("reverse-client")
				.about("Bind a port and wait for a server to connect. Then, be a client.")
				.args(&client_args)
				.arg(Arg::with_name("bind-address").index(1).default_value("0.0.0.0:2601")),
		)
		.subcommand(SubCommand::with_name("guide").about("Print information to help a new user get the most out of Oxy."))
		.get_matches_from(&*CLEAN_ARGS)
}

pub fn batched_metacommands() -> Vec<String> {
	let values = MATCHES.subcommand_matches(mode()).unwrap().values_of("metacommand");
	if values.is_none() {
		return Vec::new();
	}
	values.unwrap().map(|x| x.to_string()).collect()
}

pub fn process() {
	::lazy_static::initialize(&MATCHES);
	if env::var("RUST_LOG").is_err() {
		env::set_var("RUST_LOG", "oxy=info");
	}
	env_logger::try_init().ok();
}

pub fn mode() -> String {
	MATCHES.subcommand_name().unwrap().to_string()
}

pub fn matches() -> &'static ArgMatches<'static> {
	MATCHES.subcommand_matches(mode()).unwrap()
}

pub fn destination() -> String {
	let mut dest = MATCHES.subcommand_matches(mode()).unwrap().value_of("destination").unwrap().to_string();
	if !dest.contains(':') {
		dest = format!("{}:2600", dest);
	}
	dest
}

pub fn bind_address() -> String {
	let mut addr = MATCHES
		.subcommand_matches(mode())
		.unwrap()
		.value_of("bind-address")
		.unwrap_or("0.0.0.0:2600")
		.to_string();
	if !addr.contains(':') {
		addr = format!("{}:2600", addr);
	}
	addr
}

pub fn perspective() -> EncryptionPerspective {
	use transportation::EncryptionPerspective::{Alice, Bob};
	match mode().as_str() {
		"reexec" => Bob,
		"server" => Bob,
		"serve-one" => Bob,
		"reverse-server" => Bob,
		_ => Alice,
	}
}
