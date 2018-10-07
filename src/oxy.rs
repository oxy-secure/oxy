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
    immortality: ::parking_lot::Mutex<Option<Oxy>>,
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
                .unwrap()
                .next()
                .unwrap();
        let mut message = b"foo".to_vec();
        message.resize(272, 0);
        let mut send_buf = [0u8; 300];
        self.outer_encrypt_packet(&message, &mut send_buf);
        ::std::net::UdpSocket::bind("0.0.0.0:0")
            .unwrap()
            .send_to(&send_buf, &destination)
            .unwrap();
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
    }

    fn mode(&self) -> crate::config::Mode {
        self.i.config.lock().mode.expect("Oxy instance lacks mode")
    }

    fn socket_event(&self, event: ::mio::Event) {
        if event.readiness().is_readable() {
            self.read_socket();
        }
    }

    fn read_socket(&self) {
        loop {
            let mut buf = [0u8; 300];
            match self.i.socket.lock().as_ref().unwrap().recv_from(&mut buf) {
                Ok((amt, _src)) => {
                    if amt != 300 {
                        continue;
                    }
                    let decrypt = self.outer_decrypt_packet(&buf, |inner| {
                        self.info(|| format!("Decrypted: {:?}", &inner[..]));
                    });
                    if decrypt.is_err() {
                        self.warn(|| "Rejecting packet with bad tag.");
                    };
                }
                Err(err) => {
                    if err.kind() == ::std::io::ErrorKind::WouldBlock {
                        break;
                    }
                }
            }
        }
    }

    fn outer_decrypt_packet<T>(
        &self,
        packet: &[u8],
        callback: impl FnOnce(&mut [u8; 272]) -> T,
    ) -> Result<T, ()> {
        // SECURITY, TODO: this is a long-lived key where the IV values are purely
        // random and there's no mechanism to systematically prevent IV re-use.
        // That's not great! This layer protects conversation IDs and sequence
        // numbers, and conversation contents are separately encrypted inside
        // this layer using ephemeral keys and systematic IVs.
        //
        // It'd be sweet to use XChaCha with its sick 24 byte IVs for this? The only
        // reason I'm not doing that currently is because it's not out-of-the-box in
        // AEAD form in the libraries I looked at.
        //
        // Even sending lots of packets, 12 byte collisions should be _relatively_
        // rare. Nevertheless, there should be some trickery we can do later to make
        // this layer stronger.
        //
        // This is like this to support a single conversation roaming source IPs
        // without the client having to know when it has roamed or incurring any RTTs
        // when roaming happens. It also keeps the unauth surface area for exploits as
        // small as possible - even if there was an RCE in the ECDH implementation that
        // could be popped with a crafted public key, with this design it's nothing but
        // stream cipher from the word go. That's about as lean as conceivable, I think.
        assert!(packet.len() == 300);
        let key = self.outer_key();
        let iv = &packet[..12];
        let tag = &packet[12..28];
        let body = &packet[28..300];
        let mut out_buf = [0u8; 272];
        let result =
            ::chacha20_poly1305_aead::decrypt(&key, iv, b"", body, tag, &mut &mut out_buf[..]);
        if result.is_ok() {
            Ok(callback(&mut out_buf))
        } else {
            Err(())
        }
    }

    fn outer_encrypt_packet(&self, interior: &[u8], output: &mut [u8]) {
        // See security note in outer_decrypt_packet
        assert!(output.len() == 300);
        ::rand::Rng::fill(&mut ::rand::thread_rng(), &mut output[..12]);
        let key = self.outer_key();
        let (iv, tail) = output.split_at_mut(12);
        let (tag, mut body) = tail.split_at_mut(16);
        tag.copy_from_slice(
            &::chacha20_poly1305_aead::encrypt(&key, iv, b"", interior, &mut body).unwrap()[..],
        );
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
