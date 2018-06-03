use arg;
use core::Oxy;
use reexec::reexec;
use std::net::{TcpListener, TcpStream};

pub fn run() {
    #[cfg(unix)]
    {
        let listener = TcpListener::bind(arg::bind_address()).unwrap();
        info!("Listening on {:?}", arg::bind_address());
        loop {
            if let Ok((stream, remote_addr)) = listener.accept() {
                info!("Incoming connection from {:?}", remote_addr);
                fork_and_handle(stream);
            } else {
                warn!("Error receiving connection?");
            }
        }
    }
    #[cfg(windows)]
    unimplemented!();
}

fn fork_and_handle(stream: TcpStream) {
    #[cfg(unix)]
    {
        use nix::unistd::{close, dup};
        use std::os::unix::io::IntoRawFd;
        let fd = stream.into_raw_fd();
        let fd2 = dup(fd).unwrap(); // We do this to clear O_CLOEXEC. It'd be nicer if F_SETFL could clear
                                    // O_CLOEXEC, but it can't~

        let identity = ::keys::identity_string();
        reexec(&["reexec", &format!("--fd={}", fd2), &format!("--identity={}", identity)]);
        close(fd).unwrap();
        close(fd2).unwrap();
    }
    #[cfg(windows)]
    {
        // WSADuplicateSocket is an extremely awkward interface...
        unimplemented!();
    }
}

pub fn serve_one() {
    let stream;
    {
        let listener = TcpListener::bind(arg::bind_address()).unwrap();
        info!("Listening on {:?}", arg::bind_address());
        stream = listener.accept().unwrap().0;
        info!("Got a connection");
    }
    Oxy::run(stream);
}

pub fn reverse_server() {
    let stream = TcpStream::connect(&arg::destination()).unwrap();
    trace!("Connected");
    Oxy::run(stream);
}
