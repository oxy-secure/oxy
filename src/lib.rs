#![feature(rust_2018_preview)]

mod arg;
mod client;
mod conf;
mod copy;
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

#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

pub fn run() {
    #[cfg(unix)]
    {
        if reexec::is_suid() {
            eprintln!("Running oxy suid is currently not supported!");
            std::process::exit(1);
        }
    }
    debug!("Oxy starting");
    arg::process();
    debug!("Args processed");
    conf::init();
    debug!("Conf processed");
    keys::init();
    match arg::mode().as_str() {
        "client" => client::run(),
        "reexec" => reexec::run(),
        "server" => server::run(),
        "serve-one" => server::serve_one(),
        "reverse-server" => server::reverse_server(),
        "reverse-client" => client::reverse_client(),
        "guide" => guide::print_guide(),
        "copy" => copy::run(),
        _ => unreachable!(),
    }
}

// Reexports:
pub use crate::core::Oxy;
