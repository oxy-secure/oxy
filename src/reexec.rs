use core::Oxy;
use transportation::BufferedTransport;

pub fn run() {
	#[cfg(unix)]
	{
		use std::os::unix::io::RawFd;
		let bt = <BufferedTransport as From<RawFd>>::from(0);
		Oxy::run(bt);
	}
	#[cfg(windows)]
	unimplemented!();
}
