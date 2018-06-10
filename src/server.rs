use core::Oxy;
use reexec::reexec;
use std::{
    cell::RefCell, net::IpAddr, rc::Rc, time::{Duration, Instant},
};
use transportation::{
    self, mio::{
        net::{TcpListener, TcpStream, UdpSocket}, PollOpt, Ready, Token,
    },
};

pub fn run() -> ! {
    Server::create();
    transportation::run();
}

#[derive(Default, Clone)]
struct Server {
    i: Rc<ServerInternal>,
}

#[derive(Default)]
struct ServerInternal {
    knock_listener:    RefCell<Option<UdpSocket>>,
    knock_token:       RefCell<usize>,
    tcp_listener:      RefCell<Option<TcpListener>>,
    tcp_token:         RefCell<usize>,
    open_knocks:       RefCell<Vec<(Instant, IpAddr)>>,
    sweeper_scheduled: RefCell<bool>,
    serve_one:         RefCell<bool>,
}

impl Server {
    fn create() -> Server {
        let server = Server::default();
        server.init();
        server
    }

    fn set_serve_one(&self) {
        *self.i.serve_one.borrow_mut() = true;
    }

    fn init(&self) {
        let knock_port = ::keys::knock_port();
        info!("Listening for knocks on port {}", knock_port);
        let bind_addr = format!("[::]:{}", knock_port).parse().unwrap();
        let mut knock_listener = UdpSocket::bind(&bind_addr);
        if knock_listener.is_err() {
            let bind_addr = format!("0.0.0.0:{}", knock_port).parse().unwrap();
            knock_listener = UdpSocket::bind(&bind_addr);
            if knock_listener.is_err() {
                panic!("Failed to bind knock listener.");
            }
        }
        let knock_listener = knock_listener.unwrap();
        let proxy = self.clone();
        let knock_token = transportation::insert_listener(Rc::new(move || proxy.notify_knock()));
        transportation::borrow_poll(|poll| {
            poll.register(&knock_listener, Token(knock_token), Ready::readable(), PollOpt::level())
                .unwrap();
        });
        *self.i.knock_listener.borrow_mut() = Some(knock_listener);
        *self.i.knock_token.borrow_mut() = knock_token;
    }

    fn destroy(&self) {
        let knock_listener = self.i.knock_listener.borrow_mut().take().unwrap();
        let knock_token = *self.i.knock_token.borrow();
        transportation::borrow_poll(|poll| {
            poll.deregister(&knock_listener).unwrap();
        });
        transportation::remove_listener(knock_token);
        if self.i.tcp_listener.borrow().is_some() {
            let tcp_listener = self.i.tcp_listener.borrow_mut().take().unwrap();
            let tcp_token = *self.i.tcp_token.borrow();
            transportation::borrow_poll(|poll| {
                poll.deregister(&tcp_listener).ok();
            });
            transportation::remove_listener(tcp_token);
        }
    }

    fn notify_tcp(&self) {
        let result = self.i.tcp_listener.borrow_mut().as_mut().unwrap().accept();
        if let Ok((stream, remote_addr)) = result {
            if self.i.open_knocks.borrow().iter().filter(|x| x.1 == remote_addr.ip()).count() > 0 {
                info!("Accepting connection for {:?}", remote_addr);
                if !*self.i.serve_one.borrow() {
                    fork_and_handle(stream);
                } else {
                    self.destroy();
                    Oxy::run(stream);
                }
            } else {
                warn!("TCP connection from somebody who didn't knock: {:?}", remote_addr);
            }
        } else {
            warn!("Error accepting TCP connection");
        }
    }

    fn has_pending_knocks(&self) -> bool {
        self.i.open_knocks.borrow_mut().retain(|x| x.0.elapsed().as_secs() < 50);
        !self.i.open_knocks.borrow().is_empty()
    }

    fn refresh_tcp(&self) {
        if self.has_pending_knocks() {
            if self.i.tcp_listener.borrow().is_some() {
                return;
            }
            let bind_addr = "[::]:2600".parse().unwrap();
            let mut listener = TcpListener::bind(&bind_addr);
            if listener.is_err() {
                let bind_addr = "0.0.0.0:2600".parse().unwrap();
                listener = TcpListener::bind(&bind_addr);
                if listener.is_err() {
                    warn!("Failed to bind tcp listener: {:?}", listener);
                    return;
                }
            }
            let listener = listener.unwrap();
            let proxy = self.clone();
            let listen4_token = transportation::insert_listener(Rc::new(move || proxy.notify_tcp()));
            transportation::borrow_poll(|poll| {
                poll.register(&listener, Token(listen4_token), Ready::readable(), PollOpt::level())
                    .unwrap();
            });
            *self.i.tcp_listener.borrow_mut() = Some(listener);
            *self.i.tcp_token.borrow_mut() = listen4_token;
        } else {
            if self.i.tcp_listener.borrow().is_none() {
                return;
            }
            let token = *self.i.tcp_token.borrow_mut();
            transportation::remove_listener(token);
            let listener = self.i.tcp_listener.borrow_mut().take().unwrap();
            transportation::borrow_poll(|poll| poll.deregister(&listener).unwrap());
        }
    }

    fn sweep(&self) {
        *self.i.sweeper_scheduled.borrow_mut() = false;
        self.refresh_tcp();
        if self.i.tcp_listener.borrow().is_some() {
            self.schedule_sweeper();
        }
    }

    fn schedule_sweeper(&self) {
        if *self.i.sweeper_scheduled.borrow() {
            return;
        }
        *self.i.sweeper_scheduled.borrow_mut() = true;
        let proxy = self.clone();
        transportation::set_timeout(Rc::new(move || proxy.sweep()), Duration::from_secs(60));
    }

    fn consider_knock(&self, knock_data: &[u8], ip: IpAddr) {
        if ::keys::verify_knock(knock_data) {
            info!("Accepted knock from {:?}", ip);
            if self.i.open_knocks.borrow().len() < 1000 {
                self.i.open_knocks.borrow_mut().push((Instant::now(), ip));
                self.refresh_tcp();
                self.schedule_sweeper();
            }
        } else {
            warn!("Rejected knock from {:?}", ip);
        }
    }

    fn notify_knock(&self) {
        trace!("notify_knock");
        let mut buf = [0u8; 1500];
        let mut borrow = self.i.knock_listener.borrow_mut();
        let reader = borrow.as_mut().unwrap();
        let result = reader.recv_from(&mut buf);
        if result.is_err() {
            warn!("Error receiving knock packet {:?}", result);
            return;
        }
        let (size, addr) = result.unwrap();
        self.consider_knock(&buf[..size], addr.ip());
    }
}

pub fn serve_one() {
    let server = Server::create();
    server.set_serve_one();
    transportation::run();
}

pub fn reverse_server() {
    let stream = ::std::net::TcpStream::connect(&::arg::destination()).unwrap();
    trace!("Connected");
    Oxy::run(stream);
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
