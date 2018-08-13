#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate clap;
#[macro_use]
extern crate serde_derive;

extern crate byteorder;
extern crate data_encoding;
extern crate dirs;
extern crate env_logger;
extern crate libc;
extern crate libflate;
extern crate linefeed;
extern crate nix;
extern crate num;
extern crate ring;
extern crate serde;
extern crate serde_cbor;
extern crate shlex;
extern crate snow;
extern crate termion;
extern crate toml;
extern crate transportation;

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
pub use core::Oxy;
