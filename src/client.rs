use crate::{arg, core::Oxy, keys};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::{
    net::{TcpListener, TcpStream, UdpSocket},
    rc::Rc,
};
use transportation;

crate fn knock(peer: &str) {
    let destinations = crate::conf::locate_destination(peer);
    let port = crate::conf::knock_port_for_dest(peer);
    if destinations.is_empty() {
        error!("Failed to resolve {:?}", peer);
        ::std::process::exit(1);
    }
    let knock = UdpSocket::bind("0.0.0.0:0").unwrap();
    let knock6 = UdpSocket::bind("[::0]:0").ok();
    let knock_value = keys::make_knock(Some(peer));
    debug!("Knocking on port {}", port);
    for destination in &destinations {
        let mut destination = destination.clone();
        destination.set_port(port);
        if destination.is_ipv4() {
            knock.send_to(&knock_value, destination).unwrap();
        } else {
            knock6.as_ref().map(|x| x.send_to(&knock_value, destination).unwrap());
        }
    }
    ::std::thread::sleep(::std::time::Duration::from_millis(500));
}

crate fn run() {
    if let Some(hops) = crate::arg::matches().values_of("via") {
        let mut prev = None;
        for hop in hops.into_iter().rev() {
            if prev.is_none() {
                prev = Some(connect(hop));
                continue;
            }
            prev = Some(connect_via(prev.take().unwrap(), hop));
        }
        connect_via(prev.take().unwrap(), &crate::arg::destination());
        transportation::run();
    }
    connect(&arg::destination());
    info!("Connected");
    transportation::run();
}

crate fn connect_via(proxy_daemon: Oxy, dest: &str) -> Oxy {
    #[cfg(unix)]
    {
        use crate::message::OxyMessage::*;
        use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
        use transportation::BufferedTransport;
        let (socka, sockb) = socketpair(AddressFamily::Unix, SockType::Stream, None, SockFlag::empty()).unwrap();
        let bt = BufferedTransport::from(socka);
        proxy_daemon.set_daemon();
        let dest2 = dest;
        let dest = dest.to_string();
        proxy_daemon.clone().push_post_auth_hook(Rc::new(move || {
            let proxy_daemon = proxy_daemon.clone();
            let knock_port = crate::conf::knock_port_for_dest(&dest);
            let knock_host = crate::conf::host_for_dest(&dest);
            let knock_value = keys::make_knock(Some(&dest));
            let knock_dest = format!("{}:{}", knock_host, knock_port);
            proxy_daemon.send(KnockForward {
                destination: knock_dest,
                knock:       knock_value,
            });
            let stream_number = proxy_daemon.send(RemoteOpen {
                addr: crate::conf::canonicalize_destination(&dest),
            });
            let bt = bt.clone();
            let bt2 = bt.clone();
            let proxy_daemon2 = proxy_daemon.clone();
            let notify = Rc::new(move || {
                let proxy_daemon2 = proxy_daemon.clone();
                let bt = bt.clone();
                proxy_daemon.push_send_hook(Rc::new(move || {
                    if proxy_daemon2.has_write_space() {
                        let data = bt.take();
                        proxy_daemon2.send(RemoteStreamData {
                            data,
                            reference: stream_number,
                        });
                        return true;
                    }
                    return false;
                }));
            });
            let bt = bt2;
            ::transportation::Notifies::set_notify(&bt.clone(), notify.clone());
            notify();
            let proxy_daemon = proxy_daemon2;
            proxy_daemon.clone().watch(Rc::new(move |message, _| match message {
                LocalStreamData { data, reference } if *reference == stream_number => {
                    bt.put(&data[..]);
                    proxy_daemon.claim_message();
                    false
                }
                LocalStreamClosed { reference } if *reference == stream_number => {
                    bt.close();
                    proxy_daemon.claim_message();
                    true
                }
                _ => false,
            }));
        }));
        let result = Oxy::create(sockb);
        result.set_peer_name(&dest2);
        result
    }
    #[cfg(not(unix))]
    unimplemented!();
}

crate fn connect(destination: &str) -> Oxy {
    knock(destination);
    let destinations = crate::conf::locate_destination(destination);
    let stream = TcpStream::connect(&destinations[..]);
    if stream.is_err() {
        error!("Connection to {} failed: {:?}", destination, stream);
        ::std::process::exit(1);
    }
    let stream = stream.unwrap();
    let peer = Oxy::create(stream);
    peer.set_peer_name(destination);
    peer
}

crate fn reverse_client() {
    let acceptor = TcpListener::bind(&arg::bind_address()).unwrap();
    trace!("Bound");
    let (stream, _) = acceptor.accept().unwrap();
    trace!("Connected");
    let peer = Oxy::create(stream);
    peer.set_peer_name(crate::arg::matches().value_of("peer").unwrap());
    ::transportation::run();
}
