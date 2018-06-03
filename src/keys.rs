use arg::{self, perspective};
use data_encoding;
use transportation::{
    self, ring::{self, rand::SecureRandom, signature::Ed25519KeyPair}, untrusted, EncryptionPerspective::Alice,
};

lazy_static! {
    static ref IDENTITY_BYTES: Vec<u8> = identity_bytes_initializer();
}

fn identity_bytes_initializer() -> Vec<u8> {
    if let Some(identity) = arg::matches().value_of("identity") {
        return data_encoding::BASE32_NOPAD.decode(identity.as_bytes()).unwrap();
    }
    if arg::mode() == "copy" {
        warn!("No identity provided.");
        return Vec::new();
    }
    if perspective() == Alice {
        error!("No identity provided. If the server doesn't know who you are it won't talk to you, and how will it know who you are if you don't know who you are?");
        ::std::process::exit(1);
    }
    let mut bytes = [0u8; 24].to_vec();
    transportation::RNG.fill(&mut bytes).unwrap();
    info!(
        "Using quickstart mode. Run the client with --identity={}",
        data_encoding::BASE32_NOPAD.encode(&bytes)
    );
    bytes
}

pub fn identity_string() -> String {
    data_encoding::BASE32_NOPAD.encode(&*IDENTITY_BYTES)
}

pub fn static_key() -> &'static [u8] {
    &IDENTITY_BYTES[12..]
}

pub fn validate_peer_public_key(key: &[u8]) -> bool {
    let pubkey = asymmetric_key();
    key == pubkey.public_key_bytes()
}

pub fn asymmetric_key() -> Ed25519KeyPair {
    let mut seed = [0u8; 32];
    ring::pbkdf2::derive(&ring::digest::SHA512, 10240, b"oxy", &IDENTITY_BYTES[..12], &mut seed);
    let bytes = untrusted::Input::from(&seed);
    ring::signature::Ed25519KeyPair::from_seed_unchecked(bytes).unwrap()
}

pub fn init() {
    ::lazy_static::initialize(&IDENTITY_BYTES);
}
