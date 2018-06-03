use arg;
use core::Oxy;
use std::net::TcpStream;
use transportation::BufferedTransport;

pub fn run() {
    // If there's different remotes in sources, split the sources up into
    // one-list/source and rexec for each list
    remote_to_different_remote();
}

fn remote_to_different_remote() {
    use nix::{
        sys::socket::{socketpair, AddressFamily, SockFlag, SockType}, unistd::{close, fork, ForkResult::*},
    };
    let (socka, sockb) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
    match fork() {
        Ok(Parent { .. }) => {
            close(sockb).unwrap();
            run_dest(socka.into());
        }
        Ok(Child) => {
            close(socka).unwrap();
            run_source(sockb.into());
        }
        Err(_) => {
            panic!("Fork failed.");
        }
    }
}

fn run_source(peer: BufferedTransport) {
    let dest = arg::source_peer(0);
    let remote = TcpStream::connect(&dest[..]).unwrap();
    let oxy = Oxy::create(remote);
    oxy.fetch_files(peer);
    oxy.launch();
}

fn run_dest(peer: BufferedTransport) {
    let dest = arg::dest_peer();
    let remote = TcpStream::connect(&dest[..]).unwrap();
    let oxy = Oxy::create(remote);
    oxy.recv_files(peer);
    oxy.launch();
}
