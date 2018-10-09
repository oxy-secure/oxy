//! This module contains the main data structure for an Oxy connection.

/// The main data structure for an Oxy connection. This data structure is Send
/// + Sync and internally mutable.
#[derive(Default, Clone)]
pub struct Oxy {
    i: ::std::sync::Arc<OxyInternal>,
}

#[derive(Default)]
struct OxyInternal {
    config: ::parking_lot::Mutex<crate::config::Config>,
    socket: ::parking_lot::Mutex<Option<::mio::net::UdpSocket>>,
    socket_token: ::parking_lot::Mutex<Option<usize>>,
    immortality: ::parking_lot::Mutex<Option<Oxy>>,
    noise: ::parking_lot::Mutex<Option<::snow::Session>>,
}

impl Oxy {
    /// Make a weak reference to this Oxy instance.
    pub fn downgrade(&self) -> OxyWeak {
        OxyWeak {
            i: ::std::sync::Arc::downgrade(&self.i),
        }
    }

    /// Create a new Oxy instance with a provided config.
    pub fn new(config: &crate::config::Config) -> Oxy {
        let result: Oxy = Default::default();
        *result.i.config.lock() = config.clone();
        result.init();
        result
    }

    fn init(&self) {
        match self.mode() {
            crate::config::Mode::Server => self.init_server(),
            crate::config::Mode::Client => self.init_client(),
            _ => unimplemented!(),
        }
    }

    fn make_immortal(&self) {
        *self.i.immortality.lock() = Some(self.clone());
    }

    /// Allow this Oxy instance to be dropped if no external handle is held.
    pub fn make_mortal(&self) {
        self.i.immortality.lock().take();
    }

    fn init_client(&self) {
        self.make_immortal();
        let destination = self.i.config.lock().destination.as_ref().unwrap().clone();
        let destination: ::std::net::SocketAddr =
            ::std::net::ToSocketAddrs::to_socket_addrs(&destination)
                .expect("failed to resolve destination")
                .next()
                .expect("no address for destination");
        *self.i.noise.lock() = Some(
            ::snow::Builder::new("Noise_IK_25519_AESGCM_SHA512".parse().unwrap())
                .local_private_key(&self.local_private_key())
                .remote_public_key(&self.remote_public_key())
                .build_initiator()
                .unwrap(),
        );
    }

    fn local_private_key(&self) -> Vec<u8> {
        self.i
            .config
            .lock()
            .local_private_key
            .as_ref()
            .unwrap()
            .clone()
    }

    fn remote_public_key(&self) -> Vec<u8> {
        self.i
            .config
            .lock()
            .remote_public_key
            .as_ref()
            .unwrap()
            .clone()
    }

    fn init_server(&self) {
        self.make_immortal();
        let socket = ::mio::net::UdpSocket::bind(&"127.0.0.1:2600".parse().unwrap()).unwrap();
        self.info(|| "Successfully bound server socket");
        let weak = self.downgrade();
        let token_holder = ::std::rc::Rc::new(::std::cell::RefCell::new(0));
        let token_holder2 = token_holder.clone();
        let token = ::transportation::insert_listener(move |event| match weak.upgrade() {
            Some(oxy) => oxy.socket_event(event),
            None => {
                transportation::remove_listener(*token_holder.borrow());
            }
        });
        *token_holder2.borrow_mut() = token;
        ::transportation::borrow_poll(|poll| {
            poll.register(
                &socket,
                ::mio::Token(token),
                ::mio::Ready::readable(),
                ::mio::PollOpt::edge(),
            )
            .unwrap()
        });
        *self.i.socket.lock() = Some(socket);
        *self.i.socket_token.lock() = Some(token);
    }

    fn mode(&self) -> crate::config::Mode {
        self.i.config.lock().mode.expect("Oxy instance lacks mode")
    }

    fn socket_event(&self, event: ::mio::Event) {
        if event.readiness().is_readable() {
            self.read_socket();
        }
    }

    fn process_mid_packet(&self, mid: &[u8]) {
        match self.mode() {
            crate::config::Mode::Server => {
                let mut conversation_id = [0u8; 8];
                conversation_id[..].copy_from_slice(&mid[..8]);
                let conversation_id = u64::from_le_bytes(conversation_id);
                if conversation_id == 0 {
                    // make a new conversation
                } else if self.knows_conversation_id(conversation_id) {
                    // dispatch the message to the conversation process.
                } else {
                    self.warn(|| {
                        format!("Mid message for unknown conversation {}", conversation_id)
                    });
                }
            }
            crate::config::Mode::Client => {
                // Feed the packet into the noise session
            }
            _ => unimplemented!(),
        }
    }

    fn knows_conversation_id(&self, id: u64) -> bool {
        unimplemented!();
    }

    fn read_socket(&self) {
        loop {
            let mut buf = [0u8; crate::outer::OUTER_PACKET_SIZE];
            match self.i.socket.lock().as_ref().unwrap().recv_from(&mut buf) {
                Ok((amt, _src)) => {
                    if amt != crate::outer::OUTER_PACKET_SIZE {
                        self.warn(|| "Read less than one message worth in one call.");
                        continue;
                    }
                    let decrypt = self.decrypt_outer_packet(&buf, |mid| {
                        self.process_mid_packet(mid);
                    });
                    if decrypt.is_err() {
                        self.warn(|| "Rejecting packet with bad tag.");
                    };
                }
                Err(err) => {
                    if err.kind() == ::std::io::ErrorKind::WouldBlock {
                        break;
                    }
                    self.warn(|| format!("Error reading socket: {:?}", err));
                }
            }
        }
    }

    fn decrypt_outer_packet<T>(
        &self,
        packet: &[u8],
        callback: impl FnOnce(&mut [u8]) -> T,
    ) -> Result<T, ()> {
        let key = self.outer_key();
        crate::outer::decrypt_outer_packet(&key, packet, callback)
    }

    fn encrypt_outer_packet<T, R>(&self, interior: &[u8], callback: T) -> R
    where
        T: FnOnce(&mut [u8]) -> R,
    {
        let key = self.outer_key();
        crate::outer::encrypt_outer_packet(&key, interior, callback)
    }

    fn outer_key(&self) -> Vec<u8> {
        self.i.config.lock().outer_key.as_ref().unwrap().clone()
    }
}

/// A weak reference counted handle to an Oxy instance. Used to break reference
/// cycles. Only useful for upgrading to a real instance.
pub struct OxyWeak {
    i: ::std::sync::Weak<OxyInternal>,
}

impl OxyWeak {
    /// Upgrade to a real Oxy instance (if the corresponding real Oxy still
    /// exists)
    pub fn upgrade(&self) -> Option<Oxy> {
        match self.i.upgrade() {
            Some(i) => Some(Oxy { i }),
            None => None,
        }
    }
}
