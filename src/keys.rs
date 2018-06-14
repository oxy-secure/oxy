use byteorder::{self, ByteOrder};
use crate::arg::{self, perspective};
use data_encoding;
use lazy_static::{__lazy_static_create, __lazy_static_internal, lazy_static};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::time::UNIX_EPOCH;
use transportation::{
    self,
    ring::{self, rand::SecureRandom, signature::Ed25519KeyPair},
    untrusted,
    EncryptionPerspective::Alice,
};

use parking_lot::Mutex;

lazy_static! {
    static ref IDENTITY_BYTES: Vec<u8> = identity_bytes_initializer();
    static ref KNOCK_VALUES: Mutex<Vec<(u64, Option<String>, Vec<u8>)>> = Mutex::new(Vec::new());
}

const KNOCK_ROTATION_TIME: u64 = 60000;

fn identity_bytes_initializer() -> Vec<u8> {
    if let Some(identity) = arg::matches().value_of("identity") {
        return data_encoding::BASE32_NOPAD.decode(identity.as_bytes()).unwrap();
    }
    if let Some(identity) = crate::conf::identity() {
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

crate fn identity_string() -> String {
    data_encoding::BASE32_NOPAD.encode(&*IDENTITY_BYTES)
}

crate fn get_peer_id(peer: Option<&str>) -> Vec<u8> {
    trace!("get_peer_id for peer {:?}", peer);
    if arg::mode() == "copy" && peer.is_none() {
        panic!();
    }
    if peer.is_none() {
        return IDENTITY_BYTES.to_vec();
    }
    let id = crate::conf::client_identity_for_peer(peer.unwrap());
    if id.is_none() {
        return IDENTITY_BYTES.to_vec();
    }
    let id = id.unwrap();
    data_encoding::BASE32_NOPAD.decode(id.as_bytes()).unwrap().to_vec()
}

crate fn static_key(peer: Option<&str>) -> Vec<u8> {
    let id = get_peer_id(peer);
    id[12..24].to_vec()
}

crate fn knock_data(peer: Option<&str>) -> Vec<u8> {
    get_peer_id(peer)[24..].to_vec()
}

crate fn make_knock(peer: Option<&str>) -> Vec<u8> {
    make_knock_internal(peer, 0, 0)
}

crate fn verify_knock(peer: Option<&str>, knock: &[u8]) -> bool {
    let c = make_knock_internal(peer, 0, KNOCK_ROTATION_TIME);
    let a = make_knock_internal(peer, 0, 0);
    let b = make_knock_internal(peer, KNOCK_ROTATION_TIME, 0);
    knock == &a[..] || knock == &b[..] || knock == &c[..] // TODO, SECURITY: This should be done in constant time
}

fn make_knock_internal(peer: Option<&str>, plus: u64, minus: u64) -> Vec<u8> {
    let mut timebytes = UNIX_EPOCH.elapsed().unwrap().as_secs();
    timebytes = timebytes - (timebytes % KNOCK_ROTATION_TIME);
    timebytes += plus;
    timebytes -= minus;
    if let Some(x) = KNOCK_VALUES
        .lock()
        .iter()
        .filter(|x| x.0 == timebytes && x.1 == peer.map(|x| x.to_string()))
        .next()
    {
        return x.2.clone();
    }
    let mut result = Vec::with_capacity(100);
    result.resize(100, 0u8);
    debug!("Knock timestamp: {}", timebytes);
    let mut timebytes2 = [0u8; 8];
    byteorder::BE::write_u64(&mut timebytes2, timebytes);
    let mut input = knock_data(peer).to_vec();
    trace!("Using knock_data: {:?}", input);
    input.extend(&timebytes2);
    ring::pbkdf2::derive(&ring::digest::SHA512, 1024, b"timeknock", &input[..], &mut result[..]);
    KNOCK_VALUES.lock().push((timebytes, peer.map(|x| x.to_string()), result.clone()));
    if KNOCK_VALUES.lock().len() > 100 {
        KNOCK_VALUES.lock().remove(0);
    }
    result
}

crate fn knock_port(peer: Option<&str>) -> u16 {
    let mut data = knock_data(peer).to_vec();
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

crate fn validate_peer_public_key(key: &[u8], peer: Option<&str>) -> bool {
    let pubkey = asymmetric_key(peer);
    key == pubkey.public_key_bytes()
}

fn asymmetric_key_from_seed(seed: &[u8]) -> Ed25519KeyPair {
    let mut seed2 = [0u8; 32];
    ring::pbkdf2::derive(&ring::digest::SHA512, 10240, b"oxy", seed, &mut seed2);
    let bytes = untrusted::Input::from(&seed2);
    ring::signature::Ed25519KeyPair::from_seed_unchecked(bytes).unwrap()
}

crate fn asymmetric_key(peer: Option<&str>) -> Ed25519KeyPair {
    let id = get_peer_id(peer);
    debug!("Using identity data: {:?}", id);
    asymmetric_key_from_seed(&id[..12])
}

crate fn init() {
    ::lazy_static::initialize(&IDENTITY_BYTES);
}
