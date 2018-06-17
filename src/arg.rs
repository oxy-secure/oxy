use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches, SubCommand};
use env_logger;
use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
use std::env;
use transportation::EncryptionPerspective;

lazy_static! {
    pub(crate) static ref MATCHES: ArgMatches<'static> = create_matches();
}

crate fn create_app() -> App<'static, 'static> {
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
        .help("Use [identity] as authentication information for connecting to the remote server.")
        .env("OXY_IDENTITY");
    let command = Arg::with_name("command").index(2).default_value("bash");
    let l_portfwd = Arg::with_name("Local Port Forward")
        .multiple(true)
        .short("L")
        .takes_value(true)
        .number_of_values(1)
        .help("Create a local portforward");
    let r_portfwd = Arg::with_name("Remote Port Forward")
        .multiple(true)
        .short("R")
        .number_of_values(1)
        .takes_value(true)
        .help("Create a remote portforward");
    let d_portfwd = Arg::with_name("SOCKS")
        .multiple(true)
        .short("D")
        .long("socks")
        .help("Bind a local port as a SOCKS5 proxy")
        .number_of_values(1)
        .takes_value(true);
    let port = Arg::with_name("port")
        .short("p")
        .long("port")
        .help("The port used for TCP")
        .takes_value(true)
        .default_value("2600");
    let xforward = Arg::with_name("X Forwarding").short("X").help("Enable X forwarding");
    let trusted_xforward = Arg::with_name("Trusted X Forwarding").short("Y").help("Enable trusted X forwarding");
    let client_args = vec![
        metacommand.clone(),
        identity.clone(),
        l_portfwd,
        r_portfwd,
        d_portfwd,
        port.clone(),
        xforward,
        trusted_xforward,
        command,
    ];
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
                .about("Listen for port knocks, accept TCP connections, then reexec for each one.")
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

crate fn batched_metacommands() -> Vec<String> {
    let values = MATCHES.subcommand_matches(mode()).unwrap().values_of("metacommand");
    if values.is_none() {
        return Vec::new();
    }
    values.unwrap().map(|x| x.to_string()).collect()
}

crate fn process() {
    ::lazy_static::initialize(&MATCHES);
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "oxy=info");
    }
    env_logger::try_init().ok();
}

crate fn mode() -> String {
    MATCHES.subcommand_name().unwrap().to_string()
}

crate fn matches() -> &'static ArgMatches<'static> {
    MATCHES.subcommand_matches(mode()).unwrap()
}

crate fn destination() -> String {
    MATCHES.subcommand_matches(mode()).unwrap().value_of("destination").unwrap().to_string()
}

crate fn bind_address() -> String {
    MATCHES
        .subcommand_matches(mode())
        .unwrap()
        .value_of("bind-address")
        .unwrap_or("0.0.0.0")
        .to_string()
}

crate fn perspective() -> EncryptionPerspective {
    use transportation::EncryptionPerspective::{Alice, Bob};
    match mode().as_str() {
        "reexec" => Bob,
        "server" => Bob,
        "serve-one" => Bob,
        "reverse-server" => Bob,
        _ => Alice,
    }
}
