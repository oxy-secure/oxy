use byteorder::{self, ByteOrder};
use crate::{core::Oxy, keys};
use data_encoding::BASE32_NOPAD;
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use std::time::{SystemTime, UNIX_EPOCH};
use transportation::{
    ring::{
        agreement::{self, agree_ephemeral, EphemeralPrivateKey, X25519},
        signature,
    },
    untrusted::Input,
    RNG,
};

#[derive(Default)]
pub(super) struct KexData {
    crate connection_client_key: Option<Vec<u8>>,
    crate client_key_evidence:   Option<Vec<u8>>,
    crate my_ephemeral_key:      Option<EphemeralPrivateKey>,
    crate keymaterial:           Option<Vec<u8>>,
    crate server_key:            Option<Vec<u8>>,
    crate server_ephemeral:      Option<Vec<u8>>,
}

#[derive(Clone, PartialEq, Debug)]
pub(super) enum NakedState {
    Reject,
    WaitingForClientKey,
    WaitingForClientEphemeral,
    WaitingForClientSignature,
    WaitingForServerKey,
    WaitingForServerEphemeral,
    WaitingForServerSignature,
}

impl Oxy {
    pub(super) fn advertise_client_key(&self) {
        let peer_name = self.internal.peer_name.borrow().clone();
        trace!("x Peer name: {:?}", peer_name);
        let key = keys::asymmetric_key(peer_name.as_ref().map(|x| &**x));
        let mut pubkey: Vec<u8> = key.public_key_bytes().to_vec();
        pubkey.insert(0, 0);
        self.send_naked(&pubkey);
        let evidence = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut evidence_buf = [0u8; 8].to_vec();
        byteorder::BE::write_u64(&mut evidence_buf, evidence);
        let ephemeral_key = agreement::EphemeralPrivateKey::generate(&X25519, &*RNG).unwrap();
        evidence_buf.resize(8 + ephemeral_key.public_key_len(), 0);
        ephemeral_key.compute_public_key(&mut evidence_buf[8..]).unwrap();
        self.send_naked(&evidence_buf);
        let msg = key.sign(&evidence_buf);
        self.send_naked(msg.as_ref());
        self.internal.kex_data.borrow_mut().my_ephemeral_key = Some(ephemeral_key);
        *self.internal.naked_state.borrow_mut() = NakedState::WaitingForServerKey;
    }

    fn send_naked(&self, message: &[u8]) {
        self.internal.naked_transport.borrow_mut().as_mut().unwrap().send(message);
    }

    fn recv_naked(&self) -> Option<Vec<u8>> {
        self.internal.naked_transport.borrow_mut().as_ref().unwrap().recv()
    }

    pub(super) fn notify_naked(&self) {
        let state = self.internal.naked_state.borrow().clone();
        match state {
            NakedState::Reject => panic!(),
            NakedState::WaitingForClientKey => {
                self.bob_only();
                if let Some(mut msg) = self.recv_naked() {
                    let version_indicator = msg.remove(0);
                    assert!(version_indicator == 0);
                    let peer = self.internal.peer_name.borrow().clone();
                    if !keys::validate_peer_public_key(&msg, peer.as_ref().map(String::as_ref)) {
                        panic!("Incorrect client key");
                    }
                    debug!("Accepted client key {:?}", BASE32_NOPAD.encode(&msg));
                    self.internal.kex_data.borrow_mut().connection_client_key = Some(msg.to_vec());
                    *self.internal.naked_state.borrow_mut() = NakedState::WaitingForClientEphemeral;
                    self.notify_naked();
                }
            }
            NakedState::WaitingForClientEphemeral => {
                self.bob_only();
                if let Some(msg) = self.recv_naked() {
                    assert_timestamp(&msg[..8]);
                    self.internal.kex_data.borrow_mut().client_key_evidence = Some(msg.to_vec());
                    *self.internal.naked_state.borrow_mut() = NakedState::WaitingForClientSignature;
                    self.notify_naked();
                }
            }
            NakedState::WaitingForClientSignature => {
                self.bob_only();
                if let Some(msg) = self.recv_naked() {
                    debug!("Evidence message: {:?}", msg);
                    let kex_data = self.internal.kex_data.borrow_mut();
                    signature::verify(
                        &signature::ED25519,
                        Input::from(kex_data.connection_client_key.as_ref().unwrap()),
                        Input::from(kex_data.client_key_evidence.as_ref().unwrap()),
                        Input::from(&msg),
                    ).unwrap();
                    ::std::mem::drop(kex_data);
                    let ephemeral = agreement::EphemeralPrivateKey::generate(&X25519, &*RNG).unwrap();
                    let peer_name = self.internal.peer_name.borrow().clone();
                    let server_key = keys::asymmetric_key(peer_name.as_ref().map(|x| &**x));
                    let mut public_key_message: Vec<u8> = server_key.public_key_bytes().to_vec();
                    public_key_message.insert(0, 0);
                    self.send_naked(&public_key_message);
                    let mut buf = Vec::new();
                    buf.resize(ephemeral.public_key_len() + 8, 0);
                    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
                    byteorder::BE::write_u64(&mut buf[..8], timestamp);
                    ephemeral.compute_public_key(&mut buf[8..]).unwrap();
                    self.send_naked(&buf);
                    self.send_naked(server_key.sign(&buf).as_ref());
                    let keymaterial = agree_ephemeral(
                        ephemeral,
                        &X25519,
                        Input::from(&self.internal.kex_data.borrow_mut().client_key_evidence.as_ref().unwrap()[8..]),
                        (),
                        |x| Ok(x.to_vec()),
                    ).unwrap();
                    debug!("Got keymaterial: {:?}", keymaterial);
                    self.internal.kex_data.borrow_mut().keymaterial = Some(keymaterial);
                    self.upgrade_to_encrypted();
                }
            }
            NakedState::WaitingForServerKey => {
                self.alice_only();
                if let Some(mut msg) = self.recv_naked() {
                    let version_indicator = msg.remove(0);
                    assert!(version_indicator == 0);
                    debug!("Host key: {}", BASE32_NOPAD.encode(&msg));
                    let peer = self.internal.peer_name.borrow().clone();
                    if !keys::validate_peer_public_key(&msg, peer.as_ref().map(String::as_ref)) {
                        panic!("Invalid host key!");
                    }
                    self.internal.kex_data.borrow_mut().server_key = Some(msg);
                    *self.internal.naked_state.borrow_mut() = NakedState::WaitingForServerEphemeral;
                    self.notify_naked();
                }
            }
            NakedState::WaitingForServerEphemeral => {
                self.alice_only();
                if let Some(msg) = self.recv_naked() {
                    self.internal.kex_data.borrow_mut().server_ephemeral = Some(msg);
                    *self.internal.naked_state.borrow_mut() = NakedState::WaitingForServerSignature;
                    self.notify_naked();
                }
            }
            NakedState::WaitingForServerSignature => {
                self.alice_only();
                if let Some(msg) = self.recv_naked() {
                    let mut kex_data = self.internal.kex_data.borrow_mut();
                    signature::verify(
                        &signature::ED25519,
                        Input::from(kex_data.server_key.as_ref().unwrap()),
                        Input::from(kex_data.server_ephemeral.as_ref().unwrap()),
                        Input::from(&msg),
                    ).unwrap();
                    assert_timestamp(&kex_data.server_ephemeral.as_ref().unwrap()[..8]);
                    let keymaterial = agree_ephemeral(
                        kex_data.my_ephemeral_key.take().unwrap(),
                        &X25519,
                        Input::from(&kex_data.server_ephemeral.as_ref().unwrap()[8..]),
                        (),
                        |x| Ok(x.to_vec()),
                    ).unwrap();
                    debug!("Got keymaterial: {:?}", keymaterial);
                    kex_data.keymaterial = Some(keymaterial);
                    ::std::mem::drop(kex_data);
                    self.upgrade_to_encrypted();
                }
            }
        }
    }
}

fn assert_timestamp(timestamp: &[u8]) {
    let time = byteorder::BE::read_u64(&timestamp);
    let expected_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    assert!(time > (expected_time - 60));
    assert!(time < (expected_time + 60));
}
