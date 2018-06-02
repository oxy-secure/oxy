#[macro_use]
extern crate log;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;
extern crate byteorder;
extern crate data_encoding;
extern crate env_logger;
#[cfg(unix)]
extern crate libc;
#[cfg(unix)]
extern crate nix;
extern crate num;
extern crate shlex;
#[cfg(unix)]
extern crate termion;
extern crate textwrap;
extern crate transportation;

mod arg;
mod client;
mod core;
mod guide;
mod keys;
mod message;
#[cfg(unix)]
mod pty;
mod reexec;
mod server;
#[cfg(unix)]
mod tuntap;
mod ui;

pub fn run() {
    #[cfg(unix)]
    {
        if reexec::is_suid() {
            eprintln!("Running oxy suid is currently not supported!");
            std::process::exit(1);
        }
    }
    trace!("Oxy starting");
    arg::process();
    trace!("Args processed");
    keys::init();
    match arg::mode().as_str() {
        "client" => client::run(),
        "reexec" => reexec::run(),
        "server" => server::run(),
        "serve-one" => server::serve_one(),
        "reverse-server" => server::reverse_server(),
        "reverse-client" => client::reverse_client(),
        "guide" => guide::print_guide(),
        _ => unreachable!(),
    }
}
