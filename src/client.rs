use arg;
use core::Oxy;
use std::net::{TcpListener, TcpStream};

pub fn run() {
    let stream = TcpStream::connect(&arg::destination()).unwrap();
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
