use client;
use core::Oxy;
use std::{cell::RefCell, collections::HashMap, rc::Rc};
use transportation;

pub fn run() -> ! {
    CopyManager::create();
    transportation::run();
}

#[derive(Default)]
struct CopyManager {
    i: Rc<CopyManagerInternal>,
}

#[derive(Default)]
struct CopyManagerInternal {
    connections: RefCell<HashMap<String, Oxy>>,
    destination: RefCell<String>,
    sources:     RefCell<Vec<String>>,
}

impl CopyManager {
    fn create() -> CopyManager {
        let x = CopyManager::default();
        x.init();
        x
    }

    fn init(&self) {
        let mut locations: Vec<String> = ::arg::matches().values_of("location").unwrap().map(|x| x.to_string()).collect();
        if locations.len() < 2 {
            error!("Must provide at least two locations (a source and a destination)");
            ::std::process::exit(1);
        }
        let len = locations.len();
        let destination = locations.remove(len - 1);
        let sources = locations;
        for source in &sources {
            if let Some(dest) = get_peer(source) {
                self.create_connection(dest);
            }
        }
        if let Some(dest) = get_peer(&destination) {
            self.create_connection(dest);
        }
        *self.i.destination.borrow_mut() = destination;
        *self.i.sources.borrow_mut() = sources;
    }

    fn create_connection(&self, peer: &str) {
        if self.i.connections.borrow().contains_key(peer) {
            return;
        }
        let connection = client::connect(peer);
        connection.set_daemon();
        self.i.connections.borrow_mut().insert(peer.to_string(), connection);
    }
}

fn get_peer<'a>(location: &'a str) -> Option<&'a str> {
    if !location.splitn(2, '/').next().unwrap().contains(':') {
        return None;
    }
    if location.starts_with('[') {
        return Some(location.splitn(2, ']').next().unwrap());
    }
    Some(location.splitn(2, ':').next().unwrap())
}
