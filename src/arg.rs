use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
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
	"keygen",
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
	let peer = Arg::with_name("peer")
		.short("p")
		.long("peer")
		.takes_value(true)
		.help("The base64 ed25519 public key of the peer.")
		.required(true);
	let key = Arg::with_name("static key")
		.short("k")
		.long("static-key")
		.takes_value(true)
		.help("A pre-shared static key. Must match the static key used by the host. Provides quantum resistance!")
		.required(true);
	let metacommand = Arg::with_name("metacommand")
		.short("m")
		.long("metacommand")
		.takes_value(true)
		.multiple(true)
		.help("A command to run after the connection is established. The same commands from the F10 prompt.");
	let client_args = vec![key.clone(), peer.clone(), metacommand.clone()];
	App::new("oxy")
		.version(crate_version!())
		.author(crate_authors!())
		.setting(AppSettings::SubcommandRequired)
		.subcommand(
			SubCommand::with_name("client")
				.about("Connect to an Oxy server.")
				.args(&client_args)
				.arg(Arg::with_name("destination").index(1)),
		)
		.subcommand(
			SubCommand::with_name("reexec")
				.about("Service a single oxy connection. Communicates on stdio by default.")
				.arg(peer.clone())
				.arg(key.clone()),
		)
		.subcommand(
			SubCommand::with_name("server")
				.about("Accept TCP connections then reexec for each one.")
				.arg(peer.clone())
				.arg(key.clone()),
		)
		.subcommand(
			SubCommand::with_name("serve-one")
				.about("Accept a single TCP connection, then service it in the same process.")
				.arg(peer.clone())
				.arg(key.clone())
				.arg(Arg::with_name("bind-address").index(1).default_value("0.0.0.0:2600")),
		)
		.subcommand(
			SubCommand::with_name("reverse-server")
				.about("Connect out to a listening client. Then, be a server.")
				.arg(peer.clone())
				.arg(key.clone())
				.arg(Arg::with_name("destination").index(1)),
		)
		.subcommand(
			SubCommand::with_name("reverse-client")
				.about("Bind a port and wait for a server to connect. Then, be a client.")
				.args(&client_args)
				.arg(Arg::with_name("bind-address").index(1).default_value("0.0.0.0:2601")),
		)
		.subcommand(
			SubCommand::with_name("keygen")
				.about("Generate a keypair")
				.arg(Arg::with_name("keyfile").index(1)),
		)
		.get_matches_from(&*CLEAN_ARGS)
}

pub fn keyfile() -> Option<String> {
	MATCHES.subcommand_matches(mode()).unwrap().value_of("keyfile").map(|x| x.to_string())
}

pub fn batched_metacommands() -> Vec<String> {
	let values = MATCHES.subcommand_matches(mode()).unwrap().values_of("metacommand");
	if values.is_none() {
		return Vec::new();
	}
	values.unwrap().map(|x| x.to_string()).collect()
}

pub fn process() {
	&*MATCHES;
}

pub fn mode() -> String {
	MATCHES.subcommand_name().unwrap().to_string()
}

pub fn peer() -> String {
	MATCHES
		.subcommand_matches(mode())
		.unwrap()
		.value_of("peer")
		.expect("You must provide the peer public key! (-p)")
		.to_string()
}

pub fn key() -> String {
	MATCHES
		.subcommand_matches(mode())
		.unwrap()
		.value_of("static key")
		.expect("You must provide a pre-shared static key! (-k)")
		.to_string()
}

pub fn destination() -> String {
	let mut dest = MATCHES.subcommand_matches(mode()).unwrap().value_of("destination").unwrap().to_string();
	if !dest.contains(":") {
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
	if !addr.contains(":") {
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
