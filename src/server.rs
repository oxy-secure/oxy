use arg;
use core::Oxy;

pub fn run() {
	unimplemented!();
}

pub fn serve_one() {
	use std::net::TcpListener;
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
	use std::net::TcpStream;
	let stream = TcpStream::connect(&arg::destination()).unwrap();
	trace!("Connected");
	Oxy::run(stream);
}
