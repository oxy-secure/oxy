use std::{
    fs::File, io::Read, net::{SocketAddr, ToSocketAddrs}, str::FromStr,
};
use toml::{
    self, Value::{Array, Table},
};

lazy_static! {
    static ref CONF: Conf = load_conf();
}

#[derive(Default)]
struct Conf {
    server: Option<toml::Value>,
    client: Option<toml::Value>,
}

pub fn init() {
    ::lazy_static::initialize(&CONF);
}

fn load_conf() -> Conf {
    let mut result = Conf::default();
    result.load_server_conf();
    result.load_client_conf();
    result
}

fn load_from_home(path: &str) -> Option<toml::Value> {
    let home = ::std::env::home_dir();
    if home.is_none() {
        return None;
    }
    let home = home.unwrap();
    let mut full_path = home.clone();
    full_path.push(path);
    let file = File::open(&full_path);
    if file.is_err() {
        debug!("No {} config to load.", path);
        return None;
    }
    let mut file = file.unwrap();
    let mut data = Vec::new();
    let read_result = file.read_to_end(&mut data);
    if read_result.is_err() {
        debug!("Error reading config. {}", path);
        return None;
    }
    let decode_result = String::from_utf8(data);
    if decode_result.is_err() {
        warn!("Error decoding config as UTF-8");
        return None;
    }
    let text = decode_result.unwrap();
    let value = toml::Value::from_str(&text);
    if value.is_err() {
        warn!("Error parsing TOML: {:?}", value);
    }
    debug!("Successfully loaded {:?}", path);
    value.ok()
}

impl Conf {
    fn load_client_conf(&mut self) {
        self.client = load_from_home(".config/oxy/client.conf");
    }

    fn load_server_conf(&mut self) {
        self.server = load_from_home(".config/oxy/server.conf");
    }
}

pub fn server_identity() -> Option<&'static str> {
    match &CONF.server {
        Some(Table(table)) => table.get("identity").map(|x| x.as_str().expect("Identity is not a string?")),
        _ => None,
    }
}

pub fn client_identity_for_peer(peer: &str) -> Option<&'static str> {
    debug!("Trying to load a client identity for {}", peer);
    match &CONF.client {
        Some(Table(table)) => {
            let default_identity = table.get("identity").map(|x| x.as_str().expect("Identity is not a string?"));
            debug!("Default identity is {:?}", default_identity);
            let servers = table.get("servers");
            debug!("Servers table is {:?}", servers);
            match servers {
                Some(Array(servers)) => {
                    for server in servers {
                        debug!("Examining server {:?}", server.as_table().unwrap().get("name"));
                        if server.as_table().unwrap().get("name").unwrap().as_str().unwrap() == peer {
                            debug!("Found matching server entry in client config");
                            let identity = server.as_table().unwrap().get("identity").map(|x| x.as_str().unwrap());
                            return if identity.is_none() { default_identity } else { identity };
                        }
                    }
                    default_identity
                }
                _ => default_identity,
            }
        }
        _ => None,
    }
}

pub fn client_identity() -> Option<&'static str> {
    debug!("Trying to load a client identity");
    client_identity_for_peer(&::arg::destination())
}

fn default_port(dest: &str) -> Vec<SocketAddr> {
    let a = dest.to_socket_addrs();
    if a.is_err() {
        let a = (dest, 2600).to_socket_addrs();
        if a.is_err() {
            return Vec::new();
        }
        return a.unwrap().collect();
    }
    a.unwrap().collect()
}

fn host_part<'a>(dest: &'a str) -> &'a str {
    if dest.starts_with('[') {
        return dest.splitn(2, '[').nth(1).unwrap().splitn(2, ']').next().unwrap();
    }
    dest.splitn(2, ':').next().unwrap()
}

fn port_part(dest: &str) -> Option<u16> {
    if !dest.contains(':') {
        return None;
    }
    dest.splitn(2, ':').nth(1).unwrap().parse().ok()
}

pub fn locate_destination(dest: &str) -> Vec<SocketAddr> {
    match &CONF.client {
        Some(Table(table)) => {
            let servers = table.get("servers");
            match servers {
                Some(Array(servers)) => {
                    for server in servers {
                        let server = server.as_table().unwrap();
                        let name = server.get("name").unwrap().as_str().unwrap();
                        if name == dest {
                            let host = server.get("host").map(|x| x.as_str().unwrap()).unwrap_or(host_part(dest));
                            let port = server
                                .get("port")
                                .map(|x| x.as_str().unwrap().parse().unwrap())
                                .unwrap_or(port_part(dest).unwrap_or(2600));
                            let result = (host, port).to_socket_addrs();
                            if result.is_err() {
                                return Vec::new();
                            }
                            return result.unwrap().collect();
                        }
                    }
                    default_port(dest)
                }
                _ => default_port(dest),
            }
        }
        _ => default_port(dest),
    }
}

pub fn identity() -> Option<&'static str> {
    match ::arg::mode().as_str() {
        "server" => server_identity(),
        "client" => client_identity(),
        _ => None,
    }
}
