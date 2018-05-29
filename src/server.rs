use arg;
use core::Oxy;
use std::net::{TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::io::IntoRawFd;

pub fn run() {
	#[cfg(unix)]
	{
		let listener = TcpListener::bind(arg::bind_address()).unwrap();
		info!("Listening on {:?}", arg::bind_address());
		loop {
			if let Ok((stream, _)) = listener.accept() {
				reexec_stream(stream);
			} else {
				warn!("Error receiving connection?");
			}
		}
	}
	#[cfg(windows)]
	unimplemented!();
}

fn reexec_stream(stream: TcpStream) {
	#[cfg(unix)]
	{
		use nix::unistd::{close, execve, fork, ForkResult::*};
		use std::ffi::CString;
		let fd = stream.into_raw_fd();

		// Build everything needed for the exec before forking, because memory
		// allocation isn't async-signal-safe
		let mut args: Vec<String> = Vec::new();
		args.push("oxy".to_string());
		args.push("reexec".to_string());
		args.push(format!("--fd={}", fd));
		let args: Vec<CString> = args.into_iter().map(|x| CString::new(x).unwrap()).collect();
		let mut env: Vec<String> = Vec::new();
		env.push(format!("OXY_SERVER_ARGS={}", "TODO"));
		let env: Vec<CString> = env.into_iter().map(|x| CString::new(x).unwrap()).collect();
		// SECURITY: The Rust std API docs say this is dangerous, citing https://securityvulns.com/Wdocument183.html
		// Let's not merge it to master with this.
		let path = ::std::env::current_exe().unwrap();
		let path = CString::new(path.to_string_lossy().into_owned()).unwrap();

		match fork() {
			Ok(Parent { .. }) => close(fd).unwrap(),
			Ok(Child) => {
				execve(&path, &args, &env).unwrap();
				unreachable!();
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
