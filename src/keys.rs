use arg::{self, perspective};
use byteorder::{self, ByteOrder};
use data_encoding;
use std::time::UNIX_EPOCH;
use transportation::{
    self, ring::{self, rand::SecureRandom, signature::Ed25519KeyPair}, untrusted, EncryptionPerspective::Alice,
};

use parking_lot::Mutex;

lazy_static! {
    static ref IDENTITY_BYTES: Vec<u8> = identity_bytes_initializer();
    static ref KNOCK_VALUES: Mutex<Vec<(u64, Vec<u8>)>> = Mutex::new(Vec::new());
}

const KNOCK_ROTATION_TIME: u64 = 60;

fn identity_bytes_initializer() -> Vec<u8> {
    if let Some(identity) = arg::matches().value_of("identity") {
        return data_encoding::BASE32_NOPAD.decode(identity.as_bytes()).unwrap();
    }
    if arg::mode() == "copy" {
        warn!("No identity provided.");
        return Vec::new();
    }
    if arg::mode() == "guide" {
        return Vec::new();
    }
    if perspective() == Alice {
        error!("No identity provided. If the server doesn't know who you are it won't talk to you, and how will it know who you are if you don't know who you are?");
        ::std::process::exit(1);
    }
    let mut bytes = [0u8; 36].to_vec();
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
    &IDENTITY_BYTES[12..24]
}

pub fn knock_data() -> &'static [u8] {
    &IDENTITY_BYTES[24..]
}

pub fn make_knock() -> Vec<u8> {
    make_knock_internal(0, 0)
}

pub fn verify_knock(knock: &[u8]) -> bool {
    let c = make_knock_internal(0, KNOCK_ROTATION_TIME);
    let a = make_knock_internal(0, 0);
    let b = make_knock_internal(KNOCK_ROTATION_TIME, 0);
    knock == &a[..] || knock == &b[..] || knock == &c[..] // TODO, SECURITY: This should be done in constant time
}

fn make_knock_internal(plus: u64, minus: u64) -> Vec<u8> {
    let mut timebytes = UNIX_EPOCH.elapsed().unwrap().as_secs();
    timebytes = timebytes - (timebytes % KNOCK_ROTATION_TIME);
    timebytes += plus;
    timebytes -= minus;
    if let Some(x) = KNOCK_VALUES.lock().iter().filter(|x| x.0 == timebytes).next() {
        return x.1.clone();
    }
    let mut result = Vec::with_capacity(100);
    result.resize(100, 0u8);
    debug!("Knock timestamp: {}", timebytes);
    let mut timebytes2 = [0u8; 8];
    byteorder::BE::write_u64(&mut timebytes2, timebytes);
    let mut input = knock_data().to_vec();
    input.extend(&timebytes2);
    ring::pbkdf2::derive(&ring::digest::SHA512, 1024, b"timeknock", &input[..], &mut result[..]);
    KNOCK_VALUES.lock().push((timebytes, result.clone()));
    if KNOCK_VALUES.lock().len() > 3 {
        KNOCK_VALUES.lock().remove(0);
    }
    result
}

pub fn knock_port() -> u16 {
    let mut data = knock_data().to_vec();
    let mut iter_count = 5;
    let result;
    loop {
        let val = byteorder::BE::read_u16(&data[..2]);
        if val > 1024 {
            result = val;
            break;
        }
        let old_data = data.clone();
        iter_count += 1;
        ring::pbkdf2::derive(&ring::digest::SHA512, iter_count, b"knock", &old_data[..], &mut data[..]);
    }
    result
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
