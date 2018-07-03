use byteorder::{self, ByteOrder};
use crate::core::Oxy;
use libc::{c_ulong, ioctl};
#[allow(unused_imports)]
use log::{debug, error, info, log, trace, warn};
use nix::{
    errno::{errno, Errno},
    fcntl::{open, OFlag},
    sys::stat::Mode,
    unistd::{read, write},
};
use std::{cell::RefCell, os::unix::io::RawFd, rc::Rc};
use transportation::{
    self,
    mio::{unix::EventedFd, PollOpt, Ready, Token},
    Notifiable, Notifies,
};

#[derive(Clone)]
pub(crate) struct TunTap {
    packets:          Rc<RefCell<Vec<Vec<u8>>>>,
    fd:               RawFd,
    reference_number: u64,
    oxy:              Oxy,
    notify_hook:      Rc<RefCell<Option<Rc<dyn Notifiable>>>>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum TunTapType {
    Tun,
    Tap,
}

const IFF_TUN: u16 = 1;
const IFF_TAP: u16 = 2;
const IFF_NO_PI: u16 = 4096;
const TUNSETIFF: c_ulong = 1074025674;

impl TunTap {
    crate fn create(mode: TunTapType, name: &str, reference_number: u64, oxy: Oxy) -> TunTap {
        let fd = open("/dev/net/tun", OFlag::O_RDWR, Mode::empty()).unwrap();
        let mut buf = name.as_bytes().to_vec();
        assert!(buf.len() <= 16);
        buf.resize(16, 0);
        let flags: u16 = match mode {
            TunTapType::Tun => IFF_TUN | IFF_NO_PI,
            TunTapType::Tap => IFF_TAP | IFF_NO_PI,
        };
        let mut flags2 = [0u8; 2];
        byteorder::NativeEndian::write_u16(&mut flags2, flags);
        buf.extend(&flags2);
        unsafe { Errno::clear() };
        unsafe { ioctl(fd, TUNSETIFF, &buf[..18]) };
        let error = errno();
        if error != 0 {
            let error2 = Errno::from_i32(error).desc();
            error!("Failed to open tunnel device: {} {}", error, error2);
        }
        let result = TunTap {
            packets: Rc::new(RefCell::new(Vec::new())),
            fd,
            reference_number,
            oxy,
            notify_hook: Rc::new(RefCell::new(None)),
        };
        // Turns out buffered transport is not appropriate here, because there's a
        // 1-to-1 read() to packet correspondence :(
        let token = transportation::insert_listener(Rc::new(result.clone()));
        transportation::borrow_poll(|x| x.register(&EventedFd(&fd), Token(token), Ready::readable(), PollOpt::level()).unwrap());
        result
    }

    crate fn get_packets(&self) -> Vec<Vec<u8>> {
        self.packets.borrow_mut().split_off(0)
    }

    crate fn send(&self, data: &[u8]) {
        let result = write(self.fd, data);
        if result != Ok(data.len()) {
            warn!("Failed to write tunnel data: {:?}", result);
        }
    }
}

impl Notifiable for TunTap {
    fn notify(&self) {
        let mut buf = [0u8; 2000];
        let size = read(self.fd, &mut buf).unwrap();
        let packet = buf[..size].to_vec();
        debug!("Tunnel packet: {:?}", packet);
        self.packets.borrow_mut().push(packet);
        self.oxy.notify_tuntap(self.reference_number);
    }
}

impl Notifies for TunTap {
    fn set_notify(&self, callback: Rc<dyn Notifiable>) {
        *self.notify_hook.borrow_mut() = Some(callback);
    }
}
