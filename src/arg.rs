use clap::{crate_authors, crate_version, App, AppSettings, Arg, ArgMatches, SubCommand};
use env_logger;
use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::env;

lazy_static! {
    pub(crate) static ref MATCHES: ArgMatches<'static> = create_matches();
}

fn configure_subcommand() -> App<'static, 'static> {
    let config_client = Arg::with_name("config")
        .long("config")
        .help("Location of the configuration file to manage.")
        .default_value("~/.config/oxy/client.conf");
    let config_server = Arg::with_name("config")
        .long("config")
        .help("Location of the configuration file to manage.")
        .default_value("~/.config/oxy/server.conf");
    SubCommand::with_name("configure")
        .about("Manage configuration")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(
            SubCommand::with_name("encrypt-config")
                .about("Protect client keys with a passphrase")
                .arg(&config_client),
        )
        .subcommand(
            SubCommand::with_name("learn-client")
                .about("Register a new client that is allowed to connect to this server")
                .arg(Arg::with_name("name").help("Name for the client"))
                .arg(Arg::with_name("psk").help("PSK for the client"))
                .arg(Arg::with_name("pubkey").help("Public key for the client").required(true)),
        )
        .subcommand(
            SubCommand::with_name("learn-server")
                .about("Register a new server to connect to.")
                .arg(Arg::with_name("pubkey").help("Public key for the server").required(true))
                .arg(&config_client),
        )
        .subcommand(
            SubCommand::with_name("initialize-server")
                .about("Create an initial server configuration file. (Generates a knock key and long-term server key)")
                .arg(&config_server),
        )
}

crate fn create_app() -> App<'static, 'static> {
    let metacommand = Arg::with_name("metacommand")
        .short("m")
        .long("metacommand")
        .takes_value(true)
        .multiple(true)
        .number_of_values(1)
        .help("A command to run after the connection is established. The same commands from the F10 prompt.");
    let command = Arg::with_name("command").index(2).multiple(true);
    let l_portfwd = Arg::with_name("local port forward")
        .multiple(true)
        .short("L")
        .takes_value(true)
        .number_of_values(1)
        .help("Create a local portforward")
        .display_order(102);
    let r_portfwd = Arg::with_name("remote port forward")
        .multiple(true)
        .short("R")
        .number_of_values(1)
        .takes_value(true)
        .help("Create a remote portforward")
        .display_order(103);
    let d_portfwd = Arg::with_name("socks")
        .multiple(true)
        .short("D")
        .long("socks")
        .help("Bind a local port as a SOCKS5 proxy")
        .number_of_values(1)
        .takes_value(true)
        .display_order(104);
    let knock_port = Arg::with_name("knock port")
        .long("knock-port")
        .help("Override port used for UDP knock")
        .takes_value(true);
    let tcp_port = Arg::with_name("tcp port")
        .long("tcp-port")
        .help("Override port used for TCP")
        .takes_value(true);
    let via = Arg::with_name("via")
        .long("via")
        .takes_value(true)
        .multiple(true)
        .number_of_values(1)
        .help("Connect to a different oxy server first, then proxy traffic through the intermediary server.");
    let verbose = Arg::with_name("verbose")
        .long("verbose")
        .multiple(true)
        .short("v")
        .help("Increase debugging output");
    let xforward = Arg::with_name("X Forwarding").short("X").long("x-forwarding").help("Enable X forwarding");
    let trusted_xforward = Arg::with_name("Trusted X Forwarding")
        .short("Y")
        .long("trusted-x-forwarding")
        .help("Enable trusted X forwarding");
    let config = Arg::with_name("config")
        .long("config")
        .help("Path to configuration file (defaults to ~/.config/oxy/server.conf for servers and ~/.config/oxy/client.conf for clients)");
    let forced_command = Arg::with_name("forced command")
        .long("forced-command")
        .help("Restrict command execution to the specified command")
        .takes_value(true);
    let unsafe_reexec = Arg::with_name("unsafe reexec")
        .long("unsafe-reexec")
        .help("Bypass safety restrictions intended to avoid privilege elevation");
    let compression = Arg::with_name("compression")
        .short("C")
        .long("compress")
        .help("Enable ZLIB format compression of all transmitted data");
    let no_tmux = Arg::with_name("no tmux")
        .long("no-tmux")
        .help("Do not use a terminal multiplexer as the default pty command");
    let multiplexer = Arg::with_name("multiplexer")
        .long("multiplexer")
        .default_value("/usr/bin/tmux new-session -A -s oxy")
        .help(
            "The command to attach to a terminal multiplexer. Ignored if the first component is not an existent file, or if --no-tmux is supplied.",
        );
    let tun = Arg::with_name("tun").long("tun").help("Connect two tunnel devices together. This will work if either: both sides of the connection have root privileges (not recommended), or if the devices have been previously created with appropriate permissions (e.g. 'ip tuntap add tun0 mode tun user [youruser]')").takes_value(true).value_name("local[:remote]");
    let tap = Arg::with_name("tap").long("tap").help("Connect two tap devices together. This will work if either: both sides of the connection have root privileges (not recommended), or if the devices have been previously created with appropriate permissions (e.g. 'ip tuntap add tap0 mode tap user [youruser]')").takes_value(true).value_name("local[:remote]");
    let client_args = vec![
        metacommand.clone(),
        l_portfwd,
        r_portfwd,
        d_portfwd,
        tcp_port.clone(),
        knock_port.clone(),
        xforward,
        trusted_xforward,
        config.clone(),
        via,
        compression.clone(),
        verbose.clone(),
        tun,
        tap,
        command,
    ];
    let server_args = vec![
        config.clone(),
        forced_command,
        tcp_port.clone(),
        knock_port.clone(),
        verbose.clone(),
        no_tmux.clone(),
        multiplexer.clone(),
    ];

    let subcommands = vec![
        SubCommand::with_name("client")
            .about("Connect to an Oxy server.")
            .args(&client_args)
            .arg(Arg::with_name("destination").index(1).required(true)),
        SubCommand::with_name("reexec")
            .about("Service a single oxy connection. Not intended to be run directly, run by oxy server")
            .setting(AppSettings::Hidden)
            .arg(Arg::with_name("fd").long("fd").takes_value(true).required(true))
            .args(&server_args),
        SubCommand::with_name("server")
            .about("Listen for port knocks, accept TCP connections, then reexec for each one.")
            .args(&server_args)
            .arg(unsafe_reexec),
        SubCommand::with_name("serve-one")
            .about("Accept a single TCP connection, then service it in the same process.")
            .args(&server_args),
        SubCommand::with_name("reverse-server")
            .about("Connect out to a listening client. Then, be a server.")
            .args(&server_args)
            .arg(Arg::with_name("destination").index(1).required(true)),
        SubCommand::with_name("reverse-client")
            .about("Bind a port and wait for a server to connect. Then, be a client.")
            .args(&client_args)
            .arg(Arg::with_name("bind-address").index(1).default_value("::0")),
        SubCommand::with_name("copy")
            .about("Copy files from any number of sources to one destination.")
            .arg(config)
            .arg(compression)
            .arg(Arg::with_name("location").index(1).multiple(true).number_of_values(1))
            .arg(&verbose),
        SubCommand::with_name("guide").about("Print information to help a new user get the most out of Oxy."),
        SubCommand::with_name("keygen").about("Generate keys"),
        configure_subcommand(),
    ];
    let subcommands: Vec<_> = subcommands.into_iter().map(|x| x.setting(AppSettings::UnifiedHelpMessage)).collect();
    App::new("oxy")
        .version(crate_version!())
        .author(crate_authors!())
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .subcommands(subcommands)
}

fn create_matches() -> ArgMatches<'static> {
    trace!("Parsing arguments");
    if ::std::env::args().nth(1).as_ref().map(|x| x.as_str()) == Some("configure") {
        return create_app().get_matches();
    }
    let basic = create_app().get_matches_from_safe(::std::env::args());
    if let Ok(matches) = basic {
        return matches;
    }
    let error_kind = basic.as_ref().unwrap_err().kind;
    if error_kind == ::clap::ErrorKind::HelpDisplayed {
        return create_app().get_matches();
    }
    trace!("Trying implicit 'client'");
    let mut args2: Vec<String> = ::std::env::args().collect();
    args2.insert(1, "client".to_string());
    if let Ok(matches) = create_app().get_matches_from_safe(args2) {
        return matches;
    }
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
    let level = match matches().occurrences_of("verbose") {
        0 => "info",
        1 => "debug",
        2 => "trace",
        _ => "trace",
    };
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", format!("oxy={}", level));
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
