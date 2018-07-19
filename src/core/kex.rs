use crate::core::Oxy;
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};

impl Oxy {
    pub(super) fn advertise_client_key(&self) {
        let peer_name = self.internal.peer_name.borrow().clone();
        let peer_name2 = peer_name.as_ref().map(|x| x.as_str());
        trace!("Peer name: {:?}", peer_name);
        let privkey = crate::keys::get_private_key(peer_name2);
        let psk = crate::keys::get_static_key(peer_name2);
        let peer_public_key = crate::conf::public_key(peer_name2);
        if peer_public_key.is_none() {
            error!("No peer public key found.");
            ::std::process::exit(1);
        }
        let peer_public_key = peer_public_key.unwrap();
        let mut session = ::snow::NoiseBuilder::new("Noise_IKpsk1_25519_AESGCM_SHA512".parse().unwrap())
            .local_private_key(&privkey[..])
            .psk(1, &psk[..])
            .remote_public_key(&peer_public_key[..])
            .build_initiator()
            .unwrap();
        let mut message = Vec::with_capacity(65535);
        message.resize(65535, 0);
        let size = session.write_message(b"", &mut message).unwrap();
        self.send_naked(&message[..size]);
        *self.internal.noise_session.borrow_mut() = Some(session);
        let proxy = self.clone();
        ::transportation::Notifies::set_notify(
            &*self.internal.naked_transport.borrow_mut().as_mut().unwrap(),
            ::std::rc::Rc::new(move || proxy.client_finish_handshake()),
        );
        self.client_finish_handshake();
    }

    fn send_naked(&self, message: &[u8]) {
        self.internal.naked_transport.borrow_mut().as_mut().unwrap().send_message(message);
    }

    fn recv_naked(&self) -> Option<Vec<u8>> {
        self.internal.naked_transport.borrow_mut().as_ref().unwrap().recv_message()
    }

    fn check_hangup(&self) {
        if self.internal.naked_transport.borrow_mut().as_mut().unwrap().is_closed() {
            warn!("The peer hung up on us. Auth failed?");
            ::std::process::exit(1);
        }
    }

    fn client_finish_handshake(&self) {
        trace!("client_finish_handshake called");
        self.check_hangup();

        if let Some(message) = self.recv_naked() {
            let mut buf = [0u8; 65535].to_vec();
            let result = self
                .internal
                .noise_session
                .borrow_mut()
                .as_mut()
                .unwrap()
                .read_message(&message, &mut buf);
            if result.is_err() {
                error!("Handshake failed: {:?}", result);
                ::std::process::exit(1);
            }
            let session = self.internal.noise_session.borrow_mut().take().unwrap().into_transport_mode().unwrap();
            debug!("Handshake successful!");
            *self.internal.noise_session.borrow_mut() = Some(session);
            self.upgrade_to_encrypted();
        }
    }

    pub(super) fn server_finish_handshake(&self) {
        trace!("server_finish_handshake called");
        self.check_hangup();

        if let Some(message) = self.recv_naked() {
            let privkey = crate::keys::get_private_key(None);
            trace!("Privkey: {:?}", privkey);
            let mut session = ::snow::NoiseBuilder::new("Noise_IKpsk1_25519_AESGCM_SHA512".parse().unwrap())
                .local_private_key(&privkey[..])
                .build_responder()
                .unwrap();
            let mut message_buffer = [0u8; 65535];
            session.read_message(&message, &mut message_buffer).ok();

            let peer_public_key = session.get_remote_static().map(|x| x.to_vec());
            if peer_public_key.is_none() {
                error!("Failed to extract client public key");
                ::std::process::exit(1);
            }
            let peer_public_key = peer_public_key.unwrap();

            let peer = crate::keys::get_peer_for_public_key(&peer_public_key[..]);
            if peer.is_none() {
                error!(
                    "Rejecting connection for unknown public key: {:?}",
                    ::data_encoding::BASE32_NOPAD.encode(&peer_public_key)
                );
                ::std::process::exit(1);
            }

            let psk = crate::conf::peer_static_key(peer.as_ref().unwrap().as_str());
            if psk.is_none() {
                error!("Failed to locate PSK for peer {:?}", peer);
                ::std::process::exit(1);
            }
            let psk = psk.unwrap();
            if psk.len() != 32 {
                error!("Invalid PSK");
                ::std::process::exit(1);
            }

            let mut session = ::snow::NoiseBuilder::new("Noise_IKpsk1_25519_AESGCM_SHA512".parse().unwrap())
                .local_private_key(&privkey)
                .psk(1, &psk)
                .build_responder()
                .unwrap();
            let result = session.read_message(&message, &mut message_buffer);
            if result.is_err() {
                error!("Handshake failed: {:?}", result);
                ::std::process::exit(1);
            }

            let result = session.write_message(b"", &mut message_buffer);
            if result.is_err() {
                error!("Failed to generate handshake response: {:?}", result);
                ::std::process::exit(1);
            }
            self.send_naked(&message_buffer[..result.unwrap()]);

            let session = session.into_transport_mode().unwrap();
            debug!("Handshake successful!");

            *self.internal.noise_session.borrow_mut() = Some(session);

            self.drop_privs();
            self.upgrade_to_encrypted();
        }
    }
}
