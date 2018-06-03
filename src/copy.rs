use arg;
use core::Oxy;
use std::{net::TcpStream, path::PathBuf};
use transportation::BufferedTransport;

pub fn run() {
    if !arg::homogeneous_sources() {
        eprintln!("Sorry! Copying from multiple different sources isn't supported yet. IT REALLY SHOULD BE. Expect a lot from your tools! Don't let it stay like this forever!");
        ::std::process::exit(1);
    }
    let src = &arg::source_peer_str(0) != "";
    let dest = &arg::dest_peer_str() != "";
    if src && dest {
        remote_to_different_remote();
        #[allow(unreachable_code)]
        {
            unreachable!();
        }
    }
    if src {
        unimplemented!();
    }
    if dest {
        unimplemented!();
    }
    warn!(
        "You appear to be asking me to copy local files to a local destination. \
         I mean, I'll do it for you, but it seems like a weird thing to ask of a remote access tool."
    );
    let dest = arg::matches().value_of("dest").unwrap();
    let metadata = ::std::fs::metadata(&dest);
    let dir = metadata.is_ok() && metadata.unwrap().is_dir();
    for source in arg::matches().values_of("source").unwrap() {
        let source: PathBuf = source.into();
        let source: PathBuf = source.canonicalize().unwrap();
        let dest2: PathBuf = dest.into();
        let mut dest2: PathBuf = dest2.canonicalize().unwrap();
        if dir {
            dest2.push(source.file_name().unwrap());
        }
        let result = ::std::fs::copy(&source, &dest2);
        if result.is_err() {
            warn!("{:?}", result);
        }
    }
}

fn remote_to_different_remote() -> ! {
    use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
    let (socka, sockb) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
    run_dest(socka.into());
    run_source(sockb.into());
    ::transportation::run();
}

fn run_source(peer: BufferedTransport) {
    let dest = arg::source_peer(0);
    let remote = TcpStream::connect(&dest[..]).unwrap();
    let oxy = Oxy::create(remote);
    oxy.fetch_files(peer);
    oxy.soft_launch();
}

fn run_dest(peer: BufferedTransport) {
    let dest = arg::dest_peer();
    let remote = TcpStream::connect(&dest[..]).unwrap();
    let oxy = Oxy::create(remote);
    oxy.recv_files(peer);
    oxy.soft_launch();
}
