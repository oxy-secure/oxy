use crate::arg;
use crate::core::Oxy;
use crate::keys;
use std::net::{TcpListener, TcpStream, UdpSocket};
use transportation;

pub fn knock(peer: &str) {
    let destinations = crate::conf::locate_destination(peer);
    let port = keys::knock_port(Some(peer));
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

pub fn run() {
    connect(&arg::destination());
    info!("Connected");
    transportation::run();
}

pub fn connect(destination: &str) -> Oxy {
    knock(destination);
    let destinations = crate::conf::locate_destination(destination);
    let stream = TcpStream::connect(&destinations[..]);
    if stream.is_err() {
        error!("Connection to {} failed: {:?}", destination, stream);
        ::std::process::exit(1);
    }
    let stream = stream.unwrap();
    let peer = Oxy::create(stream);
    peer
}

pub fn reverse_client() {
    let acceptor = TcpListener::bind(&arg::bind_address()).unwrap();
    trace!("Bound");
    let (stream, _) = acceptor.accept().unwrap();
    trace!("Connected");
    Oxy::run(stream);
}
