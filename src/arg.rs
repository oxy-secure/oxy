use clap::{App, AppSettings, Arg, ArgMatches, SubCommand};
use env_logger;
use std::env;
use transportation::EncryptionPerspective;

lazy_static! {
    pub static ref MATCHES: ArgMatches<'static> = create_matches();
}

pub fn create_app() -> App<'static, 'static> {
    let metacommand = Arg::with_name("metacommand")
        .short("m")
        .long("metacommand")
        .takes_value(true)
        .multiple(true)
        .number_of_values(1)
        .help("A command to run after the connection is established. The same commands from the F10 prompt.");
    let identity = Arg::with_name("identity")
        .short("i")
        .long("identity")
        .takes_value(true)
        .env("OXY_IDENTITY");
    let command = Arg::with_name("command").index(2).default_value("bash");
    let l_portfwd = Arg::with_name("l_portfwd")
        .multiple(true)
        .short("L")
        .number_of_values(1)
        .takes_value(true);
    let r_portfwd = Arg::with_name("r_portfwd")
        .multiple(true)
        .short("R")
        .number_of_values(1)
        .takes_value(true);
    let port = Arg::with_name("port")
        .long("port")
        .help("The port used for TCP")
        .takes_value(true)
        .default_value("2600");
    let client_args = vec![metacommand.clone(), identity.clone(), l_portfwd, r_portfwd, port.clone(), command];
    let server_args = vec![identity.clone(), port.clone()];
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
                .arg(Arg::with_name("bind-address").index(1).default_value("::0")),
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
                .arg(Arg::with_name("bind-address").index(1).default_value("::0")),
        )
        .subcommand(
            SubCommand::with_name("copy")
                .about("Copy files from any number of sources to one destination.")
                .arg(Arg::with_name("location").index(1).multiple(true).number_of_values(1))
                .arg(identity.clone()),
        )
        .subcommand(SubCommand::with_name("guide").about("Print information to help a new user get the most out of Oxy."))
}

fn create_matches() -> ArgMatches<'static> {
    create_app().get_matches()
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
    MATCHES.subcommand_matches(mode()).unwrap().value_of("destination").unwrap().to_string()
}

pub fn bind_address() -> String {
    MATCHES
        .subcommand_matches(mode())
        .unwrap()
        .value_of("bind-address")
        .unwrap_or("0.0.0.0")
        .to_string()
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
