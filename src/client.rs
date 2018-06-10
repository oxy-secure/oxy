use arg;
use core::Oxy;
use keys;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};

pub fn knock<T: ToSocketAddrs>(destination: T, port: u16) {
    let knock = UdpSocket::bind("0.0.0.0:0").unwrap();
    let knock6 = UdpSocket::bind("[::0]:0").ok();
    let destinations: Vec<SocketAddr> = destination.to_socket_addrs().expect("Failed resolving destination").collect();
    let knock_value = keys::make_knock();
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
    let destinations: Vec<SocketAddr> = arg::destination().to_socket_addrs().expect("Failed resolving destination").collect();
    knock(&destinations[..], keys::knock_port());
    let stream = TcpStream::connect(&destinations[..]).unwrap();
    info!("Connected");
    Oxy::run(stream);
}

pub fn reverse_client() {
    let acceptor = TcpListener::bind(&arg::bind_address()).unwrap();
    trace!("Bound");
    let (stream, _) = acceptor.accept().unwrap();
    trace!("Connected");
    Oxy::run(stream);
}
