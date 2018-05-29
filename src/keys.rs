use arg;
use base64;
use std::{
	fs::File, io::{stdout, Read, Write}, path::Path,
};
use transportation::{
	ring::{self, signature::Ed25519KeyPair}, untrusted, EncryptionPerspective,
};

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
	let mut file = File::open(path).unwrap();
	let mut buf = Vec::new();
	file.read_to_end(&mut buf).unwrap();
	let input = untrusted::Input::from(&buf);
	let key = Ed25519KeyPair::from_pkcs8(input).unwrap();
	key
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
