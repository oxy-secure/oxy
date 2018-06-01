use arg;
use core::Oxy;
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
		use nix::unistd::{fork, ForkResult::*};

		match fork() {
			Ok(Parent { .. }) => ::std::mem::drop(stream),
			Ok(Child) => {
				Oxy::run(stream);
				#[allow(unreachable_code)]
				{
					unreachable!();
				}
			}
			Err(_) => warn!("Fork error"),
		}
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
