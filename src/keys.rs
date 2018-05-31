use arg;
use base64;
use byteorder::{self, ByteOrder};
use std::{
	fs::File, io::{stdout, Read, Write}, path::Path,
};
use transportation::{
	self, ring::{self, rand::SecureRandom, signature::Ed25519KeyPair}, untrusted, EncryptionPerspective,
};
use wordlist::WORDS;

pub fn keygen_command() {
	if let Some(keyfile) = arg::keyfile() {
		let key = load_key(keyfile);
		let pubkey = key.public_key_bytes();
		let pubkey2 = base64::encode(pubkey);
		println!("{}", pubkey2);
		return;
	}
	let (s, _) = make_key();
	let stdout = stdout();
	stdout.lock().write_all(&s).unwrap();
}

pub fn load_key<P: AsRef<Path>>(path: P) -> Ed25519KeyPair {
	let path: ::std::path::PathBuf = path.as_ref().to_path_buf();
	let file = File::open(path.clone());
	if file.is_err() {
		error!(
			"Failed to load private key {}. Maybe do 'oxy keygen > {}'?",
			path.display(),
			path.display()
		);
		::std::process::exit(1);
	}
	let mut file = file.unwrap();
	let mut buf = Vec::new();
	file.read_to_end(&mut buf).unwrap();
	let input = untrusted::Input::from(&buf);
	Ed25519KeyPair::from_pkcs8(input).unwrap()
}

pub fn load_private_key() -> Ed25519KeyPair {
	if let Some(keypath) = arg::keyfile() {
		return load_key(&keypath);
	}
	match arg::perspective() {
		EncryptionPerspective::Alice => load_key("client_key"),
		EncryptionPerspective::Bob => load_key("server_key"),
	}
}

pub fn make_key() -> (Vec<u8>, Ed25519KeyPair) {
	let rng = ring::rand::SystemRandom::new();
	let private_key = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
	let serialized = private_key.to_vec();
	let input = untrusted::Input::from(&private_key);
	let key = Ed25519KeyPair::from_pkcs8(input).unwrap();
	(serialized, key)
}

fn random_psk_word() -> &'static str {
	loop {
		let mut buf = [0u8; 2];
		transportation::RNG.fill(&mut buf).unwrap();
		buf[0] &= 0b00011111;
		let idx = byteorder::BE::read_u16(&buf) as usize;
		if idx < 7776 {
			return WORDS[idx];
		}
	}
}

pub fn make_psk() -> String {
	(0..6).map(|_| random_psk_word()).collect::<Vec<&'static str>>().join(" ").to_string()
}
