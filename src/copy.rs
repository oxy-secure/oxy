use core::Oxy;
use std::{net::TcpStream, rc::Rc};
use transportation::{BufferedTransport, Notifiable, Notifies};

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
            run_source(socka.into());
        }
        Ok(Child) => {
            close(socka).unwrap();
            run_dest(sockb.into());
        }
        Err(_) => {
            panic!("Fork failed.");
        }
    }
}

fn run_source(peer: BufferedTransport) {
    let dest = ::arg::matches().value_of("source").unwrap();
    let dest = dest.split(':').next().unwrap().to_string();
    let dest = "127.0.0.1:2600"; // TODO
    let remote = TcpStream::connect(dest).unwrap();
    let oxy = Oxy::create(remote);
    oxy.fetch_files(peer);
    oxy.launch();
}

struct DiscardNotify {
    bt: BufferedTransport,
}

impl Notifiable for DiscardNotify {
    fn notify(&self) {
        self.bt.take();
    }
}

fn run_dest(peer: BufferedTransport) {
    let discard = DiscardNotify { bt: peer.clone() };
    peer.set_notify(Rc::new(discard));
}
