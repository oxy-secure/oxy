#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate clap;

mod arg;
mod client;
mod conf;
mod copy;
mod core;
mod exit;
mod keys;
mod message;
#[cfg(unix)]
mod pty;
mod server;
#[cfg(unix)]
mod tuntap;
mod ui;
mod util;

pub fn run() {
    #[cfg(not(unix))]
    {
        warn!("Running on non-unix platform is currently a 'toy' feature. Many things will be broken and it is not recommended unless you know exactly what you are doing.");
    }
    debug!("Oxy starting");
    arg::process();
    debug!("Args processed");
    match arg::mode().as_str() {
        "client" => client::run(),
        "server" => server::run(),
        "serve-one" => server::serve_one(),
        "reverse-server" => server::reverse_server(),
        "reverse-client" => client::reverse_client(),
        "copy" => copy::run(),
        "keygen" => keys::keygen(),
        "configure" => conf::configure(),
        _ => unreachable!(),
    }
}

// Reexports:
pub use crate::core::Oxy;
