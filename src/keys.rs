use byteorder::{self, ByteOrder};
use std::time::UNIX_EPOCH;

lazy_static! {
    static ref KNOCK_VALUES: ::std::sync::Mutex<Vec<(u64, Option<String>, Vec<u8>)>> = ::std::default::Default::default();
}

const KNOCK_ROTATION_TIME: u64 = 60;

pub(crate) fn get_static_key(peer: Option<&str>) -> Vec<u8> {
    if let Some(key) = ::conf::static_key(peer) {
        return key;
    }
    error!("No PSK found");
    ::std::process::exit(1);
}

pub(crate) fn knock_data(peer: Option<&str>) -> Vec<u8> {
    if let Some(peer) = peer {
        if let Some(data) = ::conf::peer_knock(peer) {
            return data;
        }
    }
    if let Some(data) = ::conf::default_knock() {
        return data;
    }
    error!("No knock key found");
    ::std::process::exit(1);
}

pub(crate) fn make_knock(peer: Option<&str>) -> Vec<u8> {
    trace!("Calculating knock value {:?}", peer);
    make_knock_internal(peer, 0, 0)
}

pub(crate) fn verify_knock(peer: Option<&str>, knock: &[u8]) -> bool {
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
        .unwrap()
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
    ::ring::pbkdf2::derive(&::ring::digest::SHA512, 1024, b"timeknock", &input[..], &mut result[..]);
    KNOCK_VALUES
        .lock()
        .unwrap()
        .push((timebytes, peer.map(|x| x.to_string()), result.clone()));
    if KNOCK_VALUES.lock().unwrap().len() > 100 {
        KNOCK_VALUES.lock().unwrap().remove(0);
    }
    result
}

pub(crate) fn get_peer_for_public_key(key: &[u8]) -> Option<String> {
    for name in ::conf::client_names() {
        let confkey = ::conf::pubkey_for_client(&name);
        if confkey.is_some() && &confkey.unwrap()[..] == key {
            debug!("Found peer name: {:?}", name);
            return Some(name);
        }
    }
    None
}

pub(crate) fn get_private_key(peer: Option<&str>) -> Vec<u8> {
    if let Some(key) = ::conf::asymmetric_key(peer) {
        debug!("Found key in config");
        return key;
    }
    error!("No private key found for {:?}.", peer);
    ::std::process::exit(1);
}

pub(crate) fn keygen() {
    let mut dh = ::snow::CryptoResolver::resolve_dh(&::snow::DefaultResolver, &::snow::params::DHChoice::Curve25519).unwrap();
    let mut rng = ::snow::CryptoResolver::resolve_rng(&::snow::DefaultResolver).unwrap();
    ::snow::types::Dh::generate(&mut *dh, &mut *rng);

    let privkey = ::snow::types::Dh::privkey(&*dh);
    let pubkey = ::snow::types::Dh::pubkey(&*dh);
    println!("privkey = {:?}", ::data_encoding::BASE32_NOPAD.encode(privkey));
    println!("pubkey = {:?}", ::data_encoding::BASE32_NOPAD.encode(pubkey));

    let mut knock = [0u8; 32];
    let rng = ::ring::rand::SystemRandom::new();
    ::ring::rand::SecureRandom::fill(&rng, &mut knock).unwrap();
    println!("knock = {:?}", ::data_encoding::BASE32_NOPAD.encode(&knock));
    let mut psk = [0u8; 32];
    ::ring::rand::SecureRandom::fill(&rng, &mut psk).unwrap();
    println!("psk = {:?}", ::data_encoding::BASE32_NOPAD.encode(&psk));
}
