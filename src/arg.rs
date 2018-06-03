use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use env_logger;
use std::{
    env, net::{SocketAddr, ToSocketAddrs},
};
use transportation::EncryptionPerspective;

lazy_static! {
    static ref CLEAN_ARGS: Vec<String> = clean_args();
    pub static ref MATCHES: ArgMatches<'static> = create_matches();
}

const REAL_SUBCOMMANDS: [&str; 9] = [
    "client",
    "reexec",
    "server",
    "help",
    "serve-one",
    "reverse-server",
    "reverse-client",
    "guide",
    "copy",
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
    let server_args = vec![identity.clone()];
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
                .arg(Arg::with_name("fd").long("fd").takes_value(true).required(true))
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
        .subcommand(
            SubCommand::with_name("copy")
                .about("Transfer files. Source, destination, or both may be remote paths.")
                .arg(identity)
                .arg(Arg::with_name("source").index(1).multiple(true).required(true))
                .arg(Arg::with_name("dest").index(2).required(true)),
        )
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

fn path_peer(arg: &str) -> Vec<SocketAddr> {
    if !arg.splitn(2, '/').next().unwrap().contains(':') {
        return Vec::new();
    }
    if arg.starts_with('[') {
        let peer = arg.splitn(2, '[').nth(1).unwrap().splitn(2, ']').next().unwrap().to_string();
        return peer.to_socket_addrs().unwrap().collect();
    }
    (arg.splitn(2, ':').next().unwrap(), 2600).to_socket_addrs().unwrap().collect()
}

fn path_peer_str(arg: &str) -> String {
    if !arg.splitn(2, '/').next().unwrap().contains(':') {
        return "".to_string();
    }
    if arg.starts_with('[') {
        return format!("{}]", arg.splitn(2, ']').next().unwrap());
    }
    arg.splitn(2, ':').next().unwrap().to_string()
}

pub fn homogeneous_sources() -> bool {
    let first = source_peer_str(0);
    for source in matches().values_of("source").unwrap().skip(1) {
        if path_peer_str(source) != first {
            return false;
        }
    }
    return true;
}

fn path_path(arg: &str) -> String {
    if !arg.splitn(2, '/').next().unwrap().contains(':') {
        return arg.to_string();
    }
    if arg.starts_with('[') {
        return arg.splitn(2, ']').nth(1).unwrap().splitn(2, ':').nth(1).unwrap().to_string();
    }
    arg.splitn(2, ':').nth(1).unwrap().to_string()
}

pub fn source_peer(n: u64) -> Vec<SocketAddr> {
    path_peer(matches().values_of("source").unwrap().nth(n as usize).unwrap())
}

pub fn source_peer_str(n: u64) -> String {
    path_peer_str(matches().values_of("source").unwrap().nth(n as usize).unwrap())
}

pub fn source_path(n: u64) -> String {
    path_path(matches().values_of("source").unwrap().nth(n as usize).unwrap())
}

pub fn dest_path() -> String {
    path_path(matches().value_of("dest").unwrap())
}

pub fn dest_peer() -> Vec<SocketAddr> {
    path_peer(matches().value_of("dest").unwrap())
}

pub fn dest_peer_str() -> String {
    path_peer_str(matches().value_of("dest").unwrap())
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
