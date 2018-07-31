#![feature(rust_2018_preview)]
#![feature(try_from)]

mod arg;
mod client;
mod conf;
mod copy;
mod core;
mod exit;
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
mod util;

#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

pub fn run() {
    #[cfg(not(unix))]
    {
        warn!("Running on non-unix platform is currently a 'toy' feature. Many things will be broken and it is not recommended unless you know exactly what you are doing.");
    }
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
    match arg::mode().as_str() {
        "client" => client::run(),
        "reexec" => reexec::run(),
        "server" => server::run(),
        "serve-one" => server::serve_one(),
        "reverse-server" => server::reverse_server(),
        "reverse-client" => client::reverse_client(),
        "guide" => guide::print_guide(),
        "copy" => copy::run(),
        "keygen" => keys::keygen(),
        "configure" => conf::configure(),
        _ => unreachable!(),
    }
}

// Reexports:
pub use crate::core::Oxy;
