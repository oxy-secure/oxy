use crate::{core::Oxy, reexec::reexec};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use nix::{
    errno::Errno::ECHILD,
    sys::wait::{waitpid, WaitPidFlag, WaitStatus},
    Error::Sys,
};
use std::{
    cell::RefCell,
    net::IpAddr,
    rc::Rc,
    time::{Duration, Instant},
};
use transportation::{
    self,
    mio::{
        net::{TcpListener, TcpStream, UdpSocket},
        PollOpt, Ready, Token,
    },
};

crate fn run() -> ! {
    Server::create();
    transportation::run();
}

#[derive(Default, Clone)]
struct Server {
    i: Rc<ServerInternal>,
}

#[derive(Default)]
struct ServerInternal {
    knock4_listener:   RefCell<Option<UdpSocket>>,
    knock6_listener:   RefCell<Option<UdpSocket>>,
    knock4_token:      RefCell<usize>,
    knock6_token:      RefCell<usize>,
    tcp4_listener:     RefCell<Option<TcpListener>>,
    tcp6_listener:     RefCell<Option<TcpListener>>,
    tcp4_token:        RefCell<usize>,
    tcp6_token:        RefCell<usize>,
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
        crate::reexec::safety_check();
        let knock_port = crate::keys::knock_port(None);
        info!("Listening for knocks on port UDP {}", knock_port);

        {
            // Knock6
            let bind_addr = format!("[::]:{}", knock_port).parse().unwrap();
            if let Ok(knock6_listener) = UdpSocket::bind(&bind_addr) {
                let proxy = self.clone();
                let knock_token = transportation::insert_listener(Rc::new(move || proxy.notify_knock6()));
                let mut registered = false;
                transportation::borrow_poll(|poll| {
                    let result = poll.register(&knock6_listener, Token(knock_token), Ready::readable(), PollOpt::level());
                    if result.is_err() {
                        warn!("Failed to register knock6 socket");
                        transportation::remove_listener(knock_token);
                        return;
                    }
                    registered = true;
                });
                if registered {
                    *self.i.knock6_listener.borrow_mut() = Some(knock6_listener);
                    *self.i.knock6_token.borrow_mut() = knock_token;
                };
            }
        }

        {
            // Knock4
            let bind_addr = format!("0.0.0.0:{}", knock_port).parse().unwrap();
            if let Ok(knock4_listener) = UdpSocket::bind(&bind_addr) {
                let proxy = self.clone();
                let knock_token = transportation::insert_listener(Rc::new(move || proxy.notify_knock4()));
                let mut registered = false;
                transportation::borrow_poll(|poll| {
                    let result = poll.register(&knock4_listener, Token(knock_token), Ready::readable(), PollOpt::level());
                    if result.is_err() {
                        warn!("Failed to register knock4 socket");
                        transportation::remove_listener(knock_token);
                        return;
                    }
                    registered = true;
                });
                if registered {
                    *self.i.knock4_listener.borrow_mut() = Some(knock4_listener);
                    *self.i.knock4_token.borrow_mut() = knock_token;
                };
            }
        }

        if self.i.knock4_listener.borrow().is_none() && self.i.knock6_listener.borrow().is_none() {
            error!("Failed to bind knock listener");
            ::std::process::exit(1);
        }

        let proxy = self.clone();
        transportation::set_signal_handler(Rc::new(move || proxy.harvest_children()));
    }

    fn harvest_children(&self) {
        loop {
            let result = waitpid(None, Some(WaitPidFlag::WNOHANG));
            match &result {
                Err(Sys(ECHILD)) => {
                    // No children left to harvest
                    return;
                }
                _ => (),
            };
            if result.is_err() {
                warn!("Error in waitpid: {:?}", result);
                return;
            }
            let result = result.unwrap();
            match result {
                WaitStatus::Exited(pid, status) => {
                    info!("Child process {} exited with status {}", pid, status);
                }
                WaitStatus::StillAlive => {
                    return;
                }
                _ => {
                    warn!("Surprising waitpid result: {:?}", result);
                    return;
                }
            }
        }
    }

    fn destroy(&self) {
        if let Some(knock_listener) = self.i.knock4_listener.borrow_mut().take() {
            let knock_token = *self.i.knock4_token.borrow();
            transportation::borrow_poll(|poll| {
                poll.deregister(&knock_listener).unwrap();
            });
            transportation::remove_listener(knock_token);
        }

        if let Some(knock_listener) = self.i.knock6_listener.borrow_mut().take() {
            let knock_token = *self.i.knock6_token.borrow();
            transportation::borrow_poll(|poll| {
                poll.deregister(&knock_listener).unwrap();
            });
            transportation::remove_listener(knock_token);
        }

        if let Some(tcp_listener) = self.i.tcp4_listener.borrow_mut().take() {
            let tcp_token = *self.i.tcp4_token.borrow();
            transportation::borrow_poll(|poll| {
                poll.deregister(&tcp_listener).ok();
            });
            transportation::remove_listener(tcp_token);
        }

        if let Some(tcp_listener) = self.i.tcp6_listener.borrow_mut().take() {
            let tcp_token = *self.i.tcp6_token.borrow();
            transportation::borrow_poll(|poll| {
                poll.deregister(&tcp_listener).ok();
            });
            transportation::remove_listener(tcp_token);
        }
    }

    fn notify_tcp4(&self) {
        let result = self.i.tcp4_listener.borrow_mut().as_mut().unwrap().accept();
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

    fn notify_tcp6(&self) {
        let result = self.i.tcp6_listener.borrow_mut().as_mut().unwrap().accept();
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

    fn bind_tcp4(&self) {
        let port = crate::arg::matches().value_of("port").unwrap();
        let bind_addr = format!("0.0.0.0:{}", port).parse().unwrap();
        if let Ok(listener) = TcpListener::bind(&bind_addr) {
            let proxy = self.clone();
            let tcp4_token = transportation::insert_listener(Rc::new(move || proxy.notify_tcp4()));
            let mut registered = false;
            transportation::borrow_poll(|poll| {
                let result = poll.register(&listener, Token(tcp4_token), Ready::readable(), PollOpt::level());
                if result.is_err() {
                    transportation::remove_listener(tcp4_token);
                    warn!("Failed to register TCP4 listener.");
                    return;
                }
                registered = true;
            });
            if registered {
                *self.i.tcp4_listener.borrow_mut() = Some(listener);
                *self.i.tcp4_token.borrow_mut() = tcp4_token;
            }
        }
    }

    fn bind_tcp6(&self) {
        let port = crate::arg::matches().value_of("port").unwrap();
        let bind_addr = format!("[::]:{}", port).parse().unwrap();
        match TcpListener::bind(&bind_addr) {
            Ok(listener) => {
                let proxy = self.clone();
                let tcp6_token = transportation::insert_listener(Rc::new(move || proxy.notify_tcp6()));
                let mut registered = false;
                transportation::borrow_poll(|poll| {
                    let result = poll.register(&listener, Token(tcp6_token), Ready::readable(), PollOpt::level());
                    if result.is_err() {
                        transportation::remove_listener(tcp6_token);
                        warn!("Failed to register TCP6 listener.");
                        return;
                    }
                    registered = true;
                });
                if registered {
                    *self.i.tcp6_listener.borrow_mut() = Some(listener);
                    *self.i.tcp6_token.borrow_mut() = tcp6_token;
                }
            }
            Err(err) => {
                debug!("Failed to bind TCP6: {:?}", err);
            }
        }
    }

    fn refresh_tcp(&self) {
        if self.has_pending_knocks() {
            if self.i.tcp4_listener.borrow().is_some() || self.i.tcp6_listener.borrow().is_some() {
                return;
            }

            {
                self.bind_tcp6();
                self.bind_tcp4();
            }
        } else {
            if let Some(listener) = self.i.tcp4_listener.borrow_mut().take() {
                transportation::remove_listener(*self.i.tcp4_token.borrow_mut());
                transportation::borrow_poll(|poll| poll.deregister(&listener).unwrap());
            }
            if let Some(listener) = self.i.tcp6_listener.borrow_mut().take() {
                transportation::remove_listener(*self.i.tcp6_token.borrow_mut());
                transportation::borrow_poll(|poll| poll.deregister(&listener).unwrap());
            }
        }
    }

    fn sweep(&self) {
        *self.i.sweeper_scheduled.borrow_mut() = false;
        self.refresh_tcp();
        if self.i.tcp4_listener.borrow().is_some() || self.i.tcp6_listener.borrow().is_some() {
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
        if crate::keys::verify_knock(None, knock_data) {
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

    fn notify_knock4(&self) {
        trace!("notify_knock");
        let mut buf = [0u8; 1500];
        let mut borrow = self.i.knock4_listener.borrow_mut();
        let reader = borrow.as_mut().unwrap();
        let result = reader.recv_from(&mut buf);
        if result.is_err() {
            warn!("Error receiving knock packet {:?}", result);
            return;
        }
        let (size, addr) = result.unwrap();
        self.consider_knock(&buf[..size], addr.ip());
    }

    fn notify_knock6(&self) {
        trace!("notify_knock");
        let mut buf = [0u8; 1500];
        let mut borrow = self.i.knock6_listener.borrow_mut();
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

crate fn serve_one() {
    let server = Server::create();
    server.set_serve_one();
    transportation::run();
}

crate fn reverse_server() {
    let stream = ::std::net::TcpStream::connect(&crate::arg::destination()).unwrap();
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

        let mut args = vec!["reexec".to_string(), format!("--fd={}", fd2)];
        if let Some(command) = crate::arg::matches().value_of("forced command") {
            args.push(format!("--forced-command={}", command));
        }
        if crate::arg::matches().occurrences_of("multiplexer") > 0 {
            if let Some(multiplexer) = crate::arg::matches().value_of("multiplexer") {
                args.push(format!("--multiplexer={}", multiplexer));
            }
        }
        if crate::arg::matches().is_present("no tmux") {
            args.push("--no-tmux".to_string());
        }
        reexec(&args.iter().map(|x| x.as_str()).collect::<Vec<&str>>()[..]);
        close(fd).unwrap();
        close(fd2).unwrap();
    }
    #[cfg(windows)]
    {
        // WSADuplicateSocket is an extremely awkward interface...
        unimplemented!();
    }
}
